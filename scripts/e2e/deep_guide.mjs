// Drive guide preview (LLM) -> apply, verify preview persisted pending->applied,
// suggested_changes applied to contact, event written, llm_call_logs.
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";
const ACCOUNT = "e2e_acct_1";
const INSTR = "这个客户比较看重孩子的兴趣培养，请把她的意向等级往上调一档，并在运营备注里强调先安排试听。";

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

  // ---- PREVIEW ----
  console.log("--> POST guide/preview ...");
  let t0 = Date.now();
  let res = await fetch(`${BASE}/api/user-operations/guide/preview`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ accountId: ACCOUNT, contactId: id, instruction: INSTR }),
  });
  let text = await res.text();
  console.log("preview HTTP", res.status, `(${Date.now()-t0}ms)`);
  if (!res.ok) { console.log("BODY:", text.slice(0, 700)); await mc.close(); return; }
  const preview = JSON.parse(text).item;
  console.log("  preview id:", preview.id, "summary:", (preview.summary||"").slice(0,60));
  console.log("  impactScope:", preview.impactScope, "readableChanges:", (preview.readableChanges||[]).length,
    "suggestedChanges keys:", Object.keys(preview.suggestedChanges||{}));

  const stored = await db.collection("user_operation_guide_previews").findOne({ _id: new (await import("mongodb")).ObjectId(preview.id) });
  console.log("  DB preview status:", stored?.status);

  // ---- APPLY ----
  console.log("--> POST guide/apply ...");
  t0 = Date.now();
  res = await fetch(`${BASE}/api/user-operations/guide/apply`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ previewId: preview.id }),
  });
  text = await res.text();
  console.log("apply HTTP", res.status, `(${Date.now()-t0}ms)`);
  if (!res.ok) { console.log("BODY:", text.slice(0, 700)); }
  else {
    const item = JSON.parse(text).item;
    console.log("  appliedFields:", item.appliedFields, "skippedFields:", (item.skippedFields||[]).map(s=>s.field));
  }

  const afterPreview = await db.collection("user_operation_guide_previews").findOne({ _id: stored._id });
  console.log("AFTER preview status:", afterPreview?.status);
  const ev = await db.collection("agent_events").find({ kind: "user_operation_guide_applied", contact_wxid: WXID }).sort({ _id: -1 }).limit(1).toArray();
  console.log("EVENT written:", ev.length > 0, ev[0]?.status);
  const updated = await db.collection("contacts").findOne({ _id: ct._id });
  console.log("CONTACT after: intent_level:", updated.intent_level, "domain_attributes:", updated.domain_attributes, "profile_note:", (updated.human_profile_note||"").slice(0,50));

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(2).toArray();
  for (const l of logs) console.log("  llm_log:", l.prompt_key, "status:", l.status);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
