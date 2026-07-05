// Seed pending memory_candidates, then drive memory-consolidation/run (LLM),
// verify operating_memory updated + candidates flipped off pending + llm_call_logs.
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";

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

async function main() {
  const cookie = await login();
  const mc = new MongoClient("mongodb://127.0.0.1:27017");
  await mc.connect();
  const db = mc.db("wechatagent_local_e2e");
  const ct = await db.collection("contacts").findOne({ wxid: WXID });
  const id = ct._id.toString();

  // Seed a fresh pending candidate bundle matching validated_memory_candidate shape.
  const now = new Date();
  const cand = {
    workspace_id: ct.workspace_id,
    account_id: ct.account_id,
    contact_wxid: WXID,
    run_id: "e2e-seed-" + now.getTime(),
    source: "user.reply.task",
    candidates: [
      { type: "preference", content: "客户希望孩子先试听少儿编程课再决定是否报名", evidence: "她提到预算有限、希望先试听", importance: 8, confidence: 8 },
      { type: "fact", content: "客户有两个孩子，一个小学三年级、一个幼儿园大班", evidence: "客户备注：两个孩子的妈妈", importance: 7, confidence: 9 },
    ],
    memory_write_score: 8,
    status: "pending",
    reason: "试听意向与家庭结构信息",
    created_at: now,
    updated_at: now,
  };
  await db.collection("memory_candidates").insertOne(cand);
  const pendingBefore = await db.collection("memory_candidates").countDocuments({ contact_wxid: WXID, status: "pending" });
  const omBefore = await db.collection("operating_memories").findOne({ contact_wxid: WXID });
  const coreBefore = omBefore?.memory_card?.core_facts?.length ?? omBefore?.memory_card?.coreFacts?.length ?? 0;
  console.log("BEFORE pending:", pendingBefore, "core_facts:", coreBefore);

  console.log("--> POST memory-consolidation/run ...");
  const t0 = Date.now();
  const res = await fetch(`${BASE}/api/contacts/${id}/memory-consolidation/run`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
  });
  const elapsed = Date.now() - t0;
  const text = await res.text();
  console.log("consolidation HTTP", res.status, `(${elapsed}ms)`);
  if (!res.ok) { console.log("BODY:", text.slice(0, 700)); }
  else {
    const j = JSON.parse(text);
    const card = j.item?.memoryCard || j.item?.memory_card || {};
    console.log("RESP ok:", j.ok, "summary:", (j.item?.summary||"").slice(0,60));
    console.log("  card coreFacts:", (card.coreFacts||card.core_facts||[]).length,
      "recentFacts:", (card.recentFacts||card.recent_facts||[]).length);
  }

  const pendingAfter = await db.collection("memory_candidates").countDocuments({ contact_wxid: WXID, status: "pending" });
  const omAfter = await db.collection("operating_memories").findOne({ contact_wxid: WXID });
  const cardAfter = omAfter?.memory_card || {};
  console.log("AFTER pending:", pendingAfter,
    "core_facts:", (cardAfter.core_facts||cardAfter.coreFacts||[]).length,
    "recent_facts:", (cardAfter.recent_facts||cardAfter.recentFacts||[]).length);
  const cf = cardAfter.core_facts || cardAfter.coreFacts || [];
  for (const f of cf.slice(0, 6)) console.log("   fact:", typeof f === "string" ? f : (f.text || JSON.stringify(f)).slice(0, 80));

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(3).toArray();
  for (const l of logs) console.log("  llm_log:", l.prompt_key, "status:", l.status);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
