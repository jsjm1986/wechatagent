// Authenticated end-to-end test of LLM-dependent user-ops endpoints.
// Drives analyze-profile then verifies DB write + llm_call_logs.
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";

function extractCookie(res) {
  const raw = res.headers.get("set-cookie") || "";
  const m = raw.match(/wa_session=[^;]+/);
  if (!m) throw new Error("no wa_session cookie in login response: " + raw);
  return m[0];
}

async function login() {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: "admin", password: "admin" }),
  });
  if (!res.ok) throw new Error(`login failed ${res.status}: ${await res.text()}`);
  const cookie = extractCookie(res);
  const body = await res.json();
  console.log("LOGIN ok, user:", body.username, "workspace:", body.currentWorkspace);
  return cookie;
}

async function main() {
  const cookie = await login();

  const mc = new MongoClient("mongodb://127.0.0.1:27017");
  await mc.connect();
  const db = mc.db("wechatagent_local_e2e");
  const ct = await db.collection("contacts").findOne({ wxid: WXID });
  const id = ct._id.toString();
  console.log("CONTACT before:", { id, agent_profile: !!ct.agent_profile, operation_state: ct.operation_state });

  const before = await db.collection("llm_call_logs").countDocuments({});

  console.log("--> POST analyze-profile ...");
  const t0 = Date.now();
  const res = await fetch(`${BASE}/api/contacts/${id}/analyze-profile`, {
    method: "POST",
    headers: { Cookie: cookie, "Content-Type": "application/json" },
  });
  const elapsed = Date.now() - t0;
  const text = await res.text();
  console.log("analyze-profile HTTP", res.status, `(${elapsed}ms)`);
  if (!res.ok) {
    console.log("BODY:", text.slice(0, 800));
  } else {
    const j = JSON.parse(text);
    const item = j.item || {};
    console.log("RESP item.agentProfile present:", !!item.agentProfile,
      "customerStage:", item.customerStage, "operationState:", item.operationState);
  }

  const after = await db.collection("llm_call_logs")
    .find({}).sort({ _id: -1 }).limit(5).toArray();
  console.log("LLM logs delta:", (await db.collection("llm_call_logs").countDocuments({})) - before);
  for (const l of after) {
    console.log("  llm_log:", l.prompt_key || l.promptKey, "status:", l.status, "model:", l.model);
  }

  const updated = await db.collection("contacts").findOne({ _id: ct._id });
  console.log("CONTACT after:", {
    agent_profile: !!updated.agent_profile,
    profile_attributes: Array.isArray(updated.profile_attributes) ? updated.profile_attributes.length : updated.profile_attributes,
    operation_state: updated.operation_state,
    customer_stage: updated.customer_stage,
    profile_updated_at: !!updated.profile_updated_at,
  });

  await mc.close();
}

main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
