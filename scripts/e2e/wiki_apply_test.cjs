// Task#98 apply诊断:直连117 localhost,完整走 preview(唯一sourceName破缓存) → apply,
// 精确计时 apply 耗时+打印真实响应,判定 apply 是否真慢还是 Playwright 断连假象。
// 运行(在117上,nohup脱离SSH):node /opt/wechatagent/scripts/e2e/wiki_apply_test.cjs <tag>
const http = require("http");
const fs = require("fs");

function readEnv(k) {
  const txt = fs.readFileSync("/opt/wechatagent/.env", "utf8");
  const m = txt.match(new RegExp("^" + k + "=(.*)$", "m"));
  return m ? m[1].trim().replace(/^["']|["']$/g, "") : "";
}

function req(method, path, { body, cookie } = {}) {
  return new Promise((resolve, reject) => {
    const data = body ? JSON.stringify(body) : null;
    const headers = { "content-type": "application/json" };
    if (data) headers["content-length"] = Buffer.byteLength(data);
    if (cookie) headers["cookie"] = cookie;
    const r = http.request(
      { host: "127.0.0.1", port: 3003, path, method, headers, timeout: 900000 },
      (res) => {
        let d = "";
        res.on("data", (c) => (d += c));
        res.on("end", () => resolve({ status: res.statusCode, body: d, headers: res.headers }));
      }
    );
    r.on("error", reject);
    r.on("timeout", () => { r.destroy(); reject(new Error("HTTP 超时(900s)")); });
    if (data) r.write(data);
    r.end();
  });
}

(async () => {
  // argv[2] = 完整 sourceName(复用已缓存的可秒回 preview);缺省则新造(冷路径)
  const md = fs.readFileSync("/opt/wechatagent/scripts/e2e/xingling.md", "utf8");
  const srcName = process.argv[2] || ("星零感apply诊断-" + Math.floor(Date.now() / 1000));
  console.log(`MD=${md.length}chars srcName=${srcName}`);

  const login = await req("POST", "/api/auth/login", {
    body: { username: readEnv("BOOTSTRAP_ADMIN_USERNAME"), password: readEnv("BOOTSTRAP_ADMIN_PASSWORD") },
  });
  const cookie = (login.headers["set-cookie"] || []).map((c) => c.split(";")[0]).join("; ");

  const t0 = Date.now();
  const pv = await req("POST", "/api/operation-knowledge/import-preview", {
    cookie, body: { content: md, sourceName: srcName },
  });
  console.log(`preview HTTP ${pv.status} 耗时 ${((Date.now() - t0) / 1000).toFixed(1)}s`);
  if (pv.status !== 200) { console.error("preview非200", pv.body.slice(0, 300)); process.exit(1); }
  const preview = JSON.parse(pv.body);
  const chunks = preview.chunks || [];
  console.log(`preview: chunks=${chunks.length} items=${(preview.items || []).length} importReport=${JSON.stringify(preview.importReport)}`);

  // 完全模拟前端 runApply 的 payload:document + items + 全部 chunks + sourceName
  const t1 = Date.now();
  const ap = await req("POST", "/api/operation-knowledge/import-apply", {
    cookie,
    body: {
      document: preview.document,
      items: preview.items,
      chunks: chunks,
      sourceName: srcName,
    },
  });
  const applyDt = ((Date.now() - t1) / 1000).toFixed(1);
  console.log(`apply HTTP ${ap.status} 耗时 ${applyDt}s`);
  if (ap.status !== 200) { console.error("apply非200", ap.body.slice(0, 500)); process.exit(1); }
  const applied = JSON.parse(ap.body);
  const ids = applied.chunkIds || [];
  console.log(`apply: documentId=${applied.documentId} chunkIds=${ids.length} itemIds=${(applied.itemIds || []).length}`);

  await req("POST", "/api/auth/logout", { cookie });

  const ok = ap.status === 200 && ids.length > 0;
  console.log(ok ? `\n[PASS] apply落库 ${ids.length} chunks 耗时 ${applyDt}s` : "\n[FAIL] apply未落库");
  process.exit(ok ? 0 : 2);
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
