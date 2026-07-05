// 核心链路测试：seed account+managed contact → 发 webhook → 轮询 agent_run_logs →
// 验 decision/review/outbox 三段落库。WEBHOOK_VERIFY_SIGNATURE=false 故无需 HMAC。
import { MongoClient } from "mongodb";

const BASE = process.env.E2E_BASE || "http://localhost:8080";
const MONGO = "mongodb://localhost:27017";
const DB = process.env.E2E_DB || "wechatagent_local_e2e";
const APP_ID = "e2e_app_001";
const ACCT = "e2e_acct_1";
const WXID = "e2e_customer_001";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const c = new MongoClient(MONGO); await c.connect();
  const db = c.db(DB);
  const now = new Date();

  // 1) seed account（online=true 避免 outbox 离线 defer；app_id 供 webhook resolve）
  await db.collection("wechat_accounts").updateOne(
    { account_id: ACCT },
    { $set: {
      workspace_id: "default", account_id: ACCT, alias: "e2e客服", display_name: "E2E 客服",
      app_id: APP_ID, wxid: "e2e_bot", nick_name: "E2E Bot", online: true,
      last_sync_at: now, capacity: 0, off_hours: [], created_at: now, updated_at: now,
    } },
    { upsert: true },
  );

  // 2) seed managed contact（quiet_hours 覆盖关，防夜间 defer；updated_at 必填）
  await db.collection("contacts").updateOne(
    { wxid: WXID, account_id: ACCT },
    { $set: {
      workspace_id: "default", account_id: ACCT, wxid: WXID, nickname: "E2E 客户",
      agent_status: "managed", updated_at: now,
      "operation_mode_override.quiet_hours.enabled_override": false,
    }, $setOnInsert: { created_at: now } },
    { upsert: true },
  );

  const prevRuns = await db.collection("agent_run_logs").countDocuments({ contact_wxid: WXID });
  console.log("seeded. prevRuns=", prevRuns);

  // 3) send webhook
  const body = JSON.stringify({ appId: APP_ID, fromWxid: WXID, content: "你们的课程怎么收费？", msgId: "e2e_msg_" + Date.now() });
  const r = await fetch(`${BASE}/webhooks/wechat`, { method: "POST", headers: { "Content-Type": "application/json" }, body });
  console.log("webhook HTTP", r.status, (await r.text()).slice(0, 200));

  // 4) poll run log（真调 LLM 一轮 reaction+decision+review，最长 180s）
  let run = null;
  const deadline = Date.now() + 180000;
  while (Date.now() < deadline) {
    const n = await db.collection("agent_run_logs").countDocuments({ contact_wxid: WXID });
    if (n > prevRuns) {
      run = await db.collection("agent_run_logs").find({ contact_wxid: WXID }).sort({ _id: -1 }).limit(1).next();
      break;
    }
    await sleep(5000);
  }

  if (!run) {
    // 诊断：inbound 落了吗？agent_error？
    const inbound = await db.collection("conversation_messages").countDocuments({ contact_wxid: WXID });
    const errs = await db.collection("agent_events").find({ contact_wxid: WXID, kind: /error|panic|fail/i }, { projection: { kind: 1, summary: 1, _id: 0 } }).sort({ _id: -1 }).limit(5).toArray();
    console.log("NO RUN LOG. inbound=", inbound, "errs=", JSON.stringify(errs));
    await c.close(); return;
  }

  console.log("RUN LOG:", JSON.stringify({ run_id: run.run_id, lifecycle: run.lifecycle, final_review_status: run.final_review_status, outbox_status: run.outbox_status }));
  const review = await db.collection("agent_decision_reviews").find({ contact_wxid: WXID }, { projection: { scores: 1, operation_state: 1, approved: 1, outcome_status: 1, _id: 0 } }).sort({ _id: -1 }).limit(1).next();
  console.log("REVIEW:", JSON.stringify(review));
  const outbox = await db.collection("agent_send_outbox").find({ contact_wxid: WXID }, { projection: { status: 1, content: 1, _id: 0 } }).sort({ _id: -1 }).limit(3).toArray();
  console.log("OUTBOX:", JSON.stringify(outbox));
  const llm = await db.collection("llm_call_logs").find({}, { projection: { prompt_key: 1, status: 1, _id: 0 } }).sort({ _id: -1 }).limit(6).toArray();
  console.log("LLM:", JSON.stringify(llm));
  await c.close();
}
main().catch((e) => { console.error("FATAL", e); process.exit(1); });
