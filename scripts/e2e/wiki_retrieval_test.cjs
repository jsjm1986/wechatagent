// #89 wiki 检索回答能力真实测试(在117本机跑,直连本地API)
// 目标:验证 #88 放行的唯一 verified 切片"星零感微孔去眼袋推荐一句话定位"能被
//   POST /api/operation-knowledge/test-match 的 catalog→open_slice→answer LLM 工具链检索到。
// 多问法覆盖:直接命中 / 语义近义 / 无关(应 miss) 三类,验证召回既不漏也不乱。
const http = require("http");
const fs = require("fs");

const BASE = "127.0.0.1", PORT = 3003;
const TARGET_CHUNK_ID = "6a5293cb0403db3d063d77d3"; // 唯一 verified 切片

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
    const r = http.request({ host: BASE, port: PORT, path, method, headers, timeout: 300000 }, (res) => {
      let d = "";
      res.on("data", (c) => (d += c));
      res.on("end", () => resolve({ status: res.statusCode, body: d, headers: res.headers }));
    });
    r.on("error", reject);
    r.on("timeout", () => { r.destroy(); reject(new Error("请求超时(300s)")); });
    if (data) r.write(data);
    r.end();
  });
}

const QUERIES = [
  { tag: "直接命中", q: "星零感微孔去眼袋是什么？", expectHit: true },
  { tag: "语义近义", q: "我眼袋很重，你们有什么去眼袋的项目推荐吗？", expectHit: true },
  { tag: "无关对照", q: "今天天气怎么样？", expectHit: false },
];

(async () => {
  const user = readEnv("BOOTSTRAP_ADMIN_USERNAME");
  const pass = readEnv("BOOTSTRAP_ADMIN_PASSWORD");
  if (!user || !pass) { console.log("ERR: 无 admin 凭据"); process.exit(1); }

  const login = await req("POST", "/api/auth/login", { body: { username: user, password: pass } });
  if (login.status !== 200) { console.log("ERR: 登录失败 " + login.status + " " + login.body.slice(0, 120)); process.exit(1); }
  const cookie = (login.headers["set-cookie"] || []).map((c) => c.split(";")[0]).join("; ");
  console.log("登录 OK\n");

  for (const { tag, q, expectHit } of QUERIES) {
    console.log("=== [" + tag + "] " + q + " ===");
    const t0 = Date.now();
    let res;
    try {
      res = await req("POST", "/api/operation-knowledge/test-match", {
        cookie,
        body: { message: q, accountId: "102" },
      });
    } catch (e) {
      console.log("  请求异常: " + e.message + " (耗时 " + (Date.now() - t0) + "ms)\n");
      continue;
    }
    const dt = Date.now() - t0;
    if (res.status !== 200) {
      console.log("  HTTP " + res.status + " 耗时 " + dt + "ms body=" + res.body.slice(0, 200) + "\n");
      continue;
    }
    let item;
    try { item = JSON.parse(res.body).item || {}; } catch (e) { console.log("  解析失败: " + res.body.slice(0, 200) + "\n"); continue; }
    const route = item.route || {};
    const selected = item.selectedChunks || [];
    const selectedIds = selected.map((c) => c.id);
    const hitTarget = selectedIds.includes(TARGET_CHUNK_ID);
    console.log("  HTTP 200 耗时 " + dt + "ms");
    console.log("  knowledgeCoverage=" + route.knowledgeCoverage + " riskLevel=" + route.riskLevel);
    console.log("  selectedChunkIds=" + JSON.stringify(selectedIds));
    console.log("  命中目标切片(" + TARGET_CHUNK_ID.slice(0, 8) + ")=" + hitTarget + " (期望命中=" + expectHit + ")");
    console.log("  toolTrace steps=" + (route.toolTrace || []).length);
    console.log("  answer(reason前240字)=" + String(route.reason || "").slice(0, 240));
    const verdict = hitTarget === expectHit ? "✓ 符合预期" : "⚠️ 不符预期";
    console.log("  → " + verdict + "\n");
  }

  await req("POST", "/api/auth/logout", { cookie });
  console.log("检索测试完成");
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
