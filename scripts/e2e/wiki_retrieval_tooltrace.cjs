// #89 铁证:天气无关问题的完整 route.toolTrace，确认走 fallback_rank 兜底而非误匹配
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
    const r = http.request({ host: "127.0.0.1", port: 3003, path, method, headers, timeout: 300000 }, (res) => {
      let d = ""; res.on("data", (c) => (d += c)); res.on("end", () => resolve({ status: res.statusCode, body: d, headers: res.headers }));
    });
    r.on("error", reject);
    r.on("timeout", () => { r.destroy(); reject(new Error("超时")); });
    if (data) r.write(data);
    r.end();
  });
}
(async () => {
  const login = await req("POST", "/api/auth/login", { body: { username: readEnv("BOOTSTRAP_ADMIN_USERNAME"), password: readEnv("BOOTSTRAP_ADMIN_PASSWORD") } });
  const cookie = (login.headers["set-cookie"] || []).map((c) => c.split(";")[0]).join("; ");
  const res = await req("POST", "/api/operation-knowledge/test-match", { cookie, body: { message: "今天天气怎么样？", accountId: "102" } });
  const item = JSON.parse(res.body).item || {};
  const route = item.route || {};
  console.log("knowledgeCoverage=" + route.knowledgeCoverage + " riskLevel=" + route.riskLevel + " requiresEvidence=" + route.requiresEvidence);
  console.log("evidenceExcerpts.len=" + (route.evidenceExcerpts || []).length);
  console.log("toolTrace 完整:");
  console.log(JSON.stringify(route.toolTrace || [], null, 2));
  await req("POST", "/api/auth/logout", { cookie });
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
