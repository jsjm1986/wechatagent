// 决定性对照：全新零历史 wxid（真实新用户）发一条 webhook，按 run_id 精确核实
// budget/review/outbox/should_reply，隔离「基础 prompt 逼近预算」vs「历史累积」。
import { MongoClient } from "mongodb";
const BASE = "http://localhost:8080";
const MONGO = "mongodb://localhost:27017";
const DB = "wechatagent_local_e2e";
const APP_ID = "e2e_app_001";
const ACCT = "e2e_acct_1";
const WXID = "e2e_fresh_" + Date.now();  // 全新，绝无历史
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const c = new MongoClient(MONGO); await c.connect();
const db = c.db(DB);
const now = new Date();

await db.collection("wechat_accounts").updateOne({ account_id: ACCT }, { $set: {
  workspace_id: "default", account_id: ACCT, alias: "e2e客服", display_name: "E2E 客服",
  app_id: APP_ID, wxid: "e2e_bot", nick_name: "E2E Bot", online: true,
  last_sync_at: now, capacity: 0, off_hours: [], created_at: now, updated_at: now,
} }, { upsert: true });

await db.collection("contacts").updateOne({ wxid: WXID, account_id: ACCT }, { $set: {
  workspace_id: "default", account_id: ACCT, wxid: WXID, nickname: "全新客户",
  agent_status: "managed", updated_at: now,
  "operation_mode_override.quiet_hours.enabled_override": false,
}, $setOnInsert: { created_at: now } }, { upsert: true });

console.log("fresh wxid=", WXID, " (零历史)");
const body = JSON.stringify({ appId: APP_ID, fromWxid: WXID, content: "你好，你们的课程怎么收费？", msgId: "fresh_" + Date.now() });
const r = await fetch(`${BASE}/webhooks/wechat`, { method: "POST", headers: { "Content-Type": "application/json" }, body });
console.log("webhook HTTP", r.status, (await r.text()).slice(0, 150));

let run = null;
const deadline = Date.now() + 180000;
while (Date.now() < deadline) {
  run = await db.collection("agent_run_logs").find({ contact_wxid: WXID }).sort({ _id: -1 }).limit(1).next();
  if (run) break;
  await sleep(5000);
}
if (!run) {
  const inbound = await db.collection("conversation_messages").countDocuments({ contact_wxid: WXID });
  const errs = await db.collection("agent_events").find({ contact_wxid: WXID, kind: /error|panic|fail/i }, { projection: { kind:1,summary:1,_id:0 } }).sort({_id:-1}).limit(5).toArray();
  console.log("NO RUN LOG. inbound=", inbound, "errs=", JSON.stringify(errs));
  await c.close(); process.exit(0);
}
const RUN = run.run_id;
console.log("RUN:", JSON.stringify({ run_id:RUN, lifecycle:run.lifecycle, final_review_status:run.final_review_status, status:run.status, token_budget:run.token_budget, tokens_used:run.tokens_used, llm_calls_used:run.llm_calls_used, degraded_reasons:run.degraded_reasons, autonomy_mode:run.autonomy_mode }));

// 按 run_id 精确取本 run 的 LLM 调用（不跨 run 污染）
const calls = await db.collection("llm_call_logs").find({ run_id: RUN }).project({prompt_key:1,status:1,prompt_tokens:1,completion_tokens:1,total_tokens:1,retry_count:1,_id:0}).sort({_id:1}).toArray();
console.log("CALLS(byRun):", JSON.stringify(calls));

// 按 run_id / source 精确取本 run 产生的 outbox（隔离历史 run）
const inbId = await db.collection("conversation_messages").find({ contact_wxid: WXID, direction:"inbound" }).sort({_id:-1}).limit(1).next();
const outbox = await db.collection("agent_send_outbox").find({ contact_wxid: WXID }).project({status:1,content:1,source_run_id:1,decision_run_id:1,run_id:1,_id:0}).sort({_id:-1}).limit(5).toArray();
console.log("OUTBOX(wxid, 全新故应仅本 run):", JSON.stringify(outbox));
await c.close();
