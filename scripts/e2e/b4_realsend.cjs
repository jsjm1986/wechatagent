// 批4 真发闭环：webhook 灌某走查联系人入站消息(带方案B HMAC签名) → 触发生产 agent 完整链路。
// 在部署机本机跑，secret 从 DB 取不落盘不回显。发完轮询 agent_run_logs + outbox 验落库。
// 用法：APP_ID(账号 app_id)/TARGET_WXID(走查联系人 wxid)/MSG_CONTENT 环境变量。
const { MongoClient } = require("mongodb");
const crypto = require("crypto");
const http = require("http");

const BASE_HOST = process.env.BASE_HOST || "127.0.0.1";
const BASE_PORT = Number(process.env.BASE_PORT || 3003);
const APP_ID = process.env.APP_ID;
const FROM_WXID = process.env.TARGET_WXID;
const CONTENT = process.env.MSG_CONTENT || "在吗";
if (!APP_ID || !FROM_WXID) { console.error("ERR: 需设置 APP_ID 与 TARGET_WXID"); process.exit(1); }

function postWebhook(bodyStr, tsMs, sig) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: BASE_HOST, port: BASE_PORT, path: "/webhooks/wechat", method: "POST",
        headers: {
          "content-type": "application/json; charset=utf-8",
          "content-length": Buffer.byteLength(bodyStr),
          "x-webhook-timestamp": String(tsMs),
          "x-webhook-signature": "sha256=" + sig,
        },
      },
      (res) => { let d = ""; res.on("data", (c) => (d += c)); res.on("end", () => resolve({ status: res.statusCode, body: d })); }
    );
    req.on("error", reject);
    req.write(bodyStr); req.end();
  });
}

(async () => {
  const c = await MongoClient.connect(process.env.MONGODB_URI);
  const db = c.db(process.env.MONGODB_DATABASE);
  const acct = await db.collection("wechat_accounts").findOne({ account_id: "102" });
  const secret = (acct && acct.webhook_secret || "").trim();
  if (!secret) { console.log("ERR: 账号102无webhook_secret"); process.exit(1); }

  const msgId = "b4test-" + Date.now();
  const body = JSON.stringify({ appId: APP_ID, fromWxid: FROM_WXID, content: CONTENT, newMsgId: msgId });
  const tsMs = Date.now();
  const mac = crypto.createHmac("sha256", secret);
  mac.update(String(tsMs)); mac.update("."); mac.update(Buffer.from(body, "utf8"));
  const sig = mac.digest("hex");

  const prevRuns = await db.collection("agent_run_logs").countDocuments({ contact_wxid: FROM_WXID });
  console.log("发送前 agent_run_logs(" + FROM_WXID + ")=" + prevRuns + " | content=\"" + CONTENT + "\" msgId=" + msgId);

  const resp = await postWebhook(body, tsMs, sig);
  console.log("webhook HTTP=" + resp.status + " body=" + resp.body.slice(0, 200));
  if (resp.status !== 200) { console.log("!! webhook 非200，链路未触发"); await c.close(); process.exit(1); }

  // 轮询 agent_run_logs 新增 + outbox
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    const runs = await db.collection("agent_run_logs").countDocuments({ contact_wxid: FROM_WXID });
    if (runs > prevRuns) {
      const run = await db.collection("agent_run_logs").find({ contact_wxid: FROM_WXID }).sort({ _id: -1 }).limit(1).next();
      console.log("[" + (i * 3) + "s] 新 run: status=" + (run.gateway_status || run.status) + " finalReview=" + (run.final_review_status || "?"));
      const ob = await db.collection("agent_send_outbox").find({ contact_wxid: FROM_WXID }).sort({ _id: -1 }).limit(1).next();
      if (ob) console.log("  outbox: status=" + ob.status + " content=\"" + String(ob.content || "").slice(0, 120) + "\" error=" + (ob.last_error || "null"));
      const decision = run.decision || {};
      console.log("  决策: should_reply=" + decision.should_reply + " replyText=\"" + String(decision.reply_text || decision.replyText || "").slice(0, 150) + "\"");
      await c.close(); return;
    }
    console.log("[" + (i * 3) + "s] 等待 run 落库...");
  }
  console.log("!! 30 轮未见新 run，链路可能未完成");
  await c.close();
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
