// Task#98 前置快速验证:直连 117 localhost:3003,把 29KB 完整星零感 MD 走 import-preview,
// 断言分块修复产出多条 chunks(非 0)、importReport.totalSegments>1。比 Playwright 快,直接证明分块生效。
// 运行(在 117 上):node /opt/wechatagent/scripts/e2e/wiki_import_chunked_smoke.cjs
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
      { host: "127.0.0.1", port: 3003, path, method, headers, timeout: 600000 },
      (res) => {
        let d = "";
        res.on("data", (c) => (d += c));
        res.on("end", () => resolve({ status: res.statusCode, body: d, headers: res.headers }));
      }
    );
    r.on("error", reject);
    r.on("timeout", () => {
      r.destroy();
      reject(new Error("HTTP 超时(600s)"));
    });
    if (data) r.write(data);
    r.end();
  });
}

(async () => {
  const md = fs.readFileSync("/opt/wechatagent/scripts/e2e/xingling.md", "utf8");
  console.log(`MD loaded: ${md.length} chars`);

  const login = await req("POST", "/api/auth/login", {
    body: {
      username: readEnv("BOOTSTRAP_ADMIN_USERNAME"),
      password: readEnv("BOOTSTRAP_ADMIN_PASSWORD"),
    },
  });
  if (login.status !== 200) {
    console.error("登录失败", login.status, login.body.slice(0, 300));
    process.exit(1);
  }
  const cookie = (login.headers["set-cookie"] || []).map((c) => c.split(";")[0]).join("; ");

  const t0 = Date.now();
  const res = await req("POST", "/api/operation-knowledge/import-preview", {
    cookie,
    body: { content: md, sourceName: "星零感冷路径验证-" + (process.argv[2] || "n1") },
  });
  const dt = ((Date.now() - t0) / 1000).toFixed(1);
  console.log(`import-preview HTTP ${res.status} 耗时 ${dt}s`);
  if (res.status !== 200) {
    console.error("非200:", res.body.slice(0, 500));
    process.exit(1);
  }
  const pv = JSON.parse(res.body);
  const chunks = pv.chunks || [];
  const items = pv.items || [];
  const doc = pv.document || {};
  const rep = pv.importReport || {};
  console.log("=== PREVIEW 结果 ===");
  console.log("document.title=", JSON.stringify(doc.title));
  console.log("items=", items.length, " chunks=", chunks.length);
  console.log("importReport=", JSON.stringify(rep));
  console.log("integrityReport.anchoredCount=", (pv.integrityReport || {}).anchoredCount);
  chunks.slice(0, 40).forEach((c, i) => {
    if (c && typeof c === "object")
      console.log(`  chunk[${i}] type=${JSON.stringify(c.wikiType)} title=${JSON.stringify((c.title || "").slice(0, 40))} bodyLen=${(c.body || "").length}`);
  });

  await req("POST", "/api/auth/logout", { cookie });

  // 断言:分块生效 = totalSegments>1 且 chunks 非 0
  const ok = chunks.length > 0 && (rep.totalSegments || 1) > 1;
  console.log(ok ? "\n[PASS] 分块修复生效:多段抽取+chunks非0" : "\n[FAIL] 未达预期");
  process.exit(ok ? 0 : 2);
})().catch((e) => {
  console.error("ERR", e.message);
  process.exit(1);
});
