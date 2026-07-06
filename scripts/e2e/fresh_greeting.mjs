// 对照：fresh contact + 简单问候（不需知识、预期不升 Full）→ 单程能否成功过预算？
import { MongoClient } from "mongodb";
const BASE = "http://localhost:8080";
const DB = "wechatagent_local_e2e";
const APP_ID = "e2e_app_001", ACCT = "e2e_acct_1";
const WXID = "e2e_greet_" + Date.now();
const MSG = process.argv[2] || "你好，在吗？";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const c = new MongoClient("mongodb://localhost:27017"); await c.connect();
const db = c.db(DB); const now = new Date();
await db.collection("wechat_accounts").updateOne({ account_id: ACCT }, { $set: {
  workspace_id: "default", account_id: ACCT, app_id: APP_ID, wxid: "e2e_bot",
  online: true, updated_at: now }, $setOnInsert:{created_at:now} }, { upsert: true });
await db.collection("contacts").updateOne({ wxid: WXID, account_id: ACCT }, { $set: {
  workspace_id: "default", account_id: ACCT, wxid: WXID, nickname: "问候客户",
  agent_status: "managed", updated_at: now,
  "operation_mode_override.quiet_hours.enabled_override": false }, $setOnInsert:{created_at:now} }, { upsert: true });

console.log("wxid=", WXID, "msg=", MSG);
const r = await fetch(`${BASE}/webhooks/wechat`, { method:"POST", headers:{"Content-Type":"application/json"},
  body: JSON.stringify({ appId: APP_ID, fromWxid: WXID, content: MSG, msgId: "greet_"+Date.now() }) });
console.log("webhook HTTP", r.status, (await r.text()).slice(0,120));

let run=null; const dl=Date.now()+180000;
while(Date.now()<dl){ run=await db.collection("agent_run_logs").find({contact_wxid:WXID}).sort({_id:-1}).limit(1).next(); if(run)break; await sleep(5000); }
if(!run){ console.log("NO RUN"); await c.close(); process.exit(0); }
const RUN=run.run_id;
const calls=await db.collection("llm_call_logs").find({run_id:RUN}).project({prompt_key:1,total_tokens:1,_id:0}).sort({_id:1}).toArray();
console.log("RESULT:", JSON.stringify({ lifecycle:run.lifecycle, final_review_status:run.final_review_status,
  should_reply:run.decision?.shouldReply, tokens_used:run.tokens_used, token_budget:run.token_budget,
  reply_calls:calls.filter(x=>x.prompt_key==="user.reply.task").length,
  all_calls:calls.map(x=>x.prompt_key+":"+x.total_tokens) }));
const ptier=await db.collection("agent_events").find({contact_wxid:WXID,kind:/ptier|escalat/i}).project({kind:1,_id:0}).toArray();
console.log("PTIER_EVENTS:", JSON.stringify(ptier.map(e=>e.kind)));
const ob=await db.collection("agent_send_outbox").find({run_id:RUN}).project({status:1,content:1,source_event_id:1,_id:0}).toArray();
console.log("OUTBOX:", JSON.stringify(ob.map(o=>({s:o.status,c:(o.content||"").slice(0,40),src:o.source_event_id}))));
await c.close();
