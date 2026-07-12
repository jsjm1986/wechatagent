// 验证 fix 分支 API 侧可客观测量的两处修复(在117本机跑):
// F-013: 连打两次 /api/operation-knowledge/completeness 计时 —— 首次 miss 慢、二次命中应显著更快(缓存生效)
// F-003: /api/tasks 看 kind 分布 —— 应只剩客户触达类(follow_up/deferred_inbound_reply/principal_decision_relay),无 outcome_aggregation 等内部作业
// 凭据从 .env 读,不回显明文。
const http = require("http");
const fs = require("fs");

const BASE = "127.0.0.1", PORT = 3003;

// 从 /opt/wechatagent/.env 读 admin 凭据
function readEnv() {
  const txt = fs.readFileSync("/opt/wechatagent/.env", "utf8");
  const get = (k) => {
    const m = txt.match(new RegExp("^" + k + "=(.*)$", "m"));
    return m ? m[1].trim().replace(/^["']|["']$/g, "") : "";
  };
  return { user: get("BOOTSTRAP_ADMIN_USERNAME"), pass: get("BOOTSTRAP_ADMIN_PASSWORD") };
}

function req(method, path, { body, cookie } = {}) {
  return new Promise((resolve, reject) => {
    const data = body ? JSON.stringify(body) : null;
    const headers = { "content-type": "application/json" };
    if (data) headers["content-length"] = Buffer.byteLength(data);
    if (cookie) headers["cookie"] = cookie;
    const r = http.request({ host: BASE, port: PORT, path, method, headers }, (res) => {
      let d = "";
      res.on("data", (c) => (d += c));
      res.on("end", () => resolve({ status: res.statusCode, body: d, headers: res.headers }));
    });
    r.on("error", reject);
    if (data) r.write(data);
    r.end();
  });
}

(async () => {
  const { user, pass } = readEnv();
  if (!user || !pass) { console.log("ERR: 无 admin 凭据"); process.exit(1); }

  // 登录拿 cookie
  const login = await req("POST", "/api/auth/login", { body: { username: user, password: pass } });
  if (login.status !== 200) { console.log("ERR: 登录失败 " + login.status + " " + login.body.slice(0, 120)); process.exit(1); }
  const cookie = (login.headers["set-cookie"] || []).map((c) => c.split(";")[0]).join("; ");
  console.log("登录 OK");

  // 账号 102 参数
  const ACCT = "?accountId=102";

  // === F-013: completeness 连打两次计时 ===
  console.log("\n=== F-013 completeness 缓存验证 ===");
  const t1 = Date.now();
  const c1 = await req("GET", "/api/operation-knowledge/completeness" + ACCT, { cookie });
  const d1 = Date.now() - t1;
  console.log("第1次(预期 miss,慢): " + c1.status + " 耗时 " + d1 + "ms");
  const t2 = Date.now();
  const c2 = await req("GET", "/api/operation-knowledge/completeness" + ACCT, { cookie });
  const d2 = Date.now() - t2;
  console.log("第2次(预期命中缓存,快): " + c2.status + " 耗时 " + d2 + "ms");
  console.log("→ 缓存判定: " + (d2 < d1 / 3 || d2 < 200 ? "命中生效(二次显著更快) ✓" : "疑似未命中(二次未明显加速) ⚠️"));
  // refresh 强制重算
  const t3 = Date.now();
  const c3 = await req("POST", "/api/operation-knowledge/completeness" + ACCT, { cookie, body: {} });
  const d3 = Date.now() - t3;
  console.log("refresh(POST 强制重算): " + c3.status + " 耗时 " + d3 + "ms (预期与首次同量级=重算)");

  // === F-003: /api/tasks kind 分布 ===
  console.log("\n=== F-003 任务日志 kind 过滤验证 ===");
  const tasks = await req("GET", "/api/tasks" + ACCT, { cookie });
  if (tasks.status !== 200) { console.log("ERR: /api/tasks " + tasks.status); }
  else {
    let items = [];
    try { items = JSON.parse(tasks.body).items || []; } catch (e) { console.log("解析失败:" + tasks.body.slice(0, 120)); }
    const kinds = {};
    for (const it of items) kinds[it.kind] = (kinds[it.kind] || 0) + 1;
    console.log("任务总数: " + items.length);
    console.log("kind 分布: " + JSON.stringify(kinds));
    const WHITELIST = ["follow_up", "deferred_inbound_reply", "principal_decision_relay"];
    const leaked = Object.keys(kinds).filter((k) => !WHITELIST.includes(k));
    console.log("→ 过滤判定: " + (leaked.length === 0 ? "无内部作业泄漏 ✓" : "仍泄漏: " + leaked.join(",") + " ⚠️"));
  }

  // 登出
  await req("POST", "/api/auth/logout", { cookie });
  console.log("\n验证完成");
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
