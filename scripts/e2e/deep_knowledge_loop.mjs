// Capstone: prove product-claim red line from the ALLOW side.
// chat(draft pricing knowledge) -> apply(draft chunk) -> verify(integrity=verified)
// -> re-run webhook pricing question -> confirm NO LONGER blocked_unverified_product_claim.
import { MongoClient, ObjectId } from "mongodb";

const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";
const ACCOUNT = "e2e_acct_1";
const APPID = "e2e_app_001";

function extractCookie(res) {
  const raw = res.headers.get("set-cookie") || "";
  const m = raw.match(/wa_session=[^;]+/);
  if (!m) throw new Error("no wa_session cookie: " + raw);
  return m[0];
}
async function login() {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: "admin", password: "admin" }),
  });
  if (!res.ok) throw new Error(`login ${res.status}: ${await res.text()}`);
  return extractCookie(res);
}
const sid = () => "e2e-know-" + Math.random().toString(36).slice(2, 10);

async function main() {
  const cookie = await login();
  const mc = new MongoClient("mongodb://127.0.0.1:27017");
  await mc.connect();
  const db = mc.db("wechatagent_local_e2e");
  const H = { Cookie: cookie, "Content-Type": "application/json" };

  // ---- STEP 1: chat to draft pricing knowledge ----
  const session = sid();
  console.log("STEP 1 chat draft, session:", session);
  let res = await fetch(`${BASE}/api/operation-knowledge/chat`, {
    method: "POST", headers: H,
    body: JSON.stringify({
      sessionId: session,
      accountId: ACCOUNT,
      content: "帮我把这条课程价格信息整理成一条知识：我们的少儿编程启蒙课（Python方向）标准价是每期3980元，一期共16课时，购买两期享9折优惠。这是市场部2026年最新核定价格。",
    }),
  });
  let text = await res.text();
  console.log("  chat HTTP", res.status);
  if (!res.ok) { console.log("  BODY:", text.slice(0, 400)); await mc.close(); return; }
  let j = JSON.parse(text);
  console.log("  intent:", j.turn?.intent || j.intent, "hasPatch:", !!(j.turn?.patch || j.patch));

  // ---- STEP 2: apply the draft ----
  console.log("STEP 2 apply");
  res = await fetch(`${BASE}/api/operation-knowledge/chat/${session}/apply`, {
    method: "POST", headers: H, body: JSON.stringify({ accountId: ACCOUNT }),
  });
  text = await res.text();
  console.log("  apply HTTP", res.status, "->", text.slice(0, 200));
  if (!res.ok) { await mc.close(); return; }
  j = JSON.parse(text);
  const chunkId = j.result?.chunkId || j.result?.chunk_id || j.result?.id;
  console.log("  created chunkId:", chunkId);

  // inspect the created chunk
  const chunk = await db.collection("operation_knowledge_chunks").findOne(
    chunkId ? { _id: new ObjectId(chunkId) } : { workspace_id: "default" },
    { sort: { _id: -1 } }
  );
  console.log("  chunk integrity_status:", chunk?.integrity_status, "has sourceQuote:", !!chunk?.source_quote,
    "anchors:", (chunk?.source_anchors||[]).length, "account_id:", chunk?.account_id);

  // ---- STEP 3: verify the chunk ----
  const cid = chunk._id.toString();
  console.log("STEP 3 verify chunk", cid);
  res = await fetch(`${BASE}/api/operation-knowledge/chunks/${cid}/verify`, {
    method: "POST", headers: H, body: JSON.stringify({ verifiedClaims: ["少儿编程启蒙课标准价每期3980元", "购买两期9折"] }),
  });
  text = await res.text();
  console.log("  verify HTTP", res.status, "->", text.slice(0, 200));
  const afterChunk = await db.collection("operation_knowledge_chunks").findOne({ _id: chunk._id });
  console.log("  chunk after: integrity_status:", afterChunk?.integrity_status, "status:", afterChunk?.status, "confidence:", afterChunk?.confidence_score);

  // ---- STEP 4: re-run webhook pricing question ----
  console.log("STEP 4 webhook pricing question (verified KB now exists)");
  const msgId = "e2e-know-msg-" + Date.now();
  res = await fetch(`${BASE}/webhooks/wechat`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      appId: APPID, fromWxid: WXID, toWxid: ACCOUNT,
      msgId, msgType: "text",
      content: "你们少儿编程启蒙课怎么收费？多少钱一期？",
    }),
  });
  text = await res.text();
  console.log("  webhook HTTP", res.status, "->", text.slice(0, 150));

  // poll for run log
  let runLog = null;
  for (let i = 0; i < 40; i++) {
    await new Promise(r => setTimeout(r, 3000));
    runLog = await db.collection("agent_run_logs").find({ contact_wxid: WXID }).sort({ _id: -1 }).limit(1).toArray();
    if (runLog[0] && runLog[0].inbound_message_id === msgId) break;
    runLog = null;
  }
  if (!runLog || !runLog[0]) { console.log("  (no run log matched msgId; latest may be older)"); }
  const rl = runLog?.[0];
  if (rl) {
    console.log("  RUN LOG lifecycle:", rl.lifecycle, "final_review_status:", rl.final_review_status);
    console.log("  used_knowledge:", (rl.used_knowledge_ids||[]).length, "outbox_status:", rl.outbox_status);
    const rev = await db.collection("decision_reviews").find({ contact_wxid: WXID }).sort({_id:-1}).limit(1).toArray();
    if (rev[0]) console.log("  REVIEW approved:", rev[0].approved, "scores:", JSON.stringify(rev[0].scores));
  }

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(4).toArray();
  for (const l of logs) console.log("  llm_log:", l.prompt_key, l.status);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
