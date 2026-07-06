// Drive playbook optimize (LLM), verify version bump + created_by + fields persist.
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const INSTR = "请强化：与家长客户沟通时更强调试听体验和孩子的兴趣引导，弱化促单话术，语气更亲和。";

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
  // default-workspace playbook (admin.current_workspace = default)
  const pb = await db.collection("operation_playbooks").findOne({ workspace_id: "default", account_id: "default" });
  const id = pb._id.toString();
  console.log("BEFORE:", { id, name: pb.name, version: pb.version, created_by: pb.created_by });

  console.log("--> POST optimize ...");
  const t0 = Date.now();
  const res = await fetch(`${BASE}/api/operation-playbooks/${id}/optimize`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ instruction: INSTR }),
  });
  const elapsed = Date.now() - t0;
  const text = await res.text();
  console.log("optimize HTTP", res.status, `(${elapsed}ms)`);
  if (!res.ok) { console.log("BODY:", text.slice(0, 700)); }
  else {
    const item = JSON.parse(text).item || {};
    console.log("RESP name:", item.name, "version:", item.version);
    console.log("  replyStyle:", (item.replyStyle||"").slice(0,80));
  }

  const after = await db.collection("operation_playbooks").findOne({ _id: pb._id });
  console.log("AFTER DB:", { version: after.version, created_by: after.created_by,
    method_prompt_len: (after.method_prompt||"").length });

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(2).toArray();
  for (const l of logs) console.log("  llm_log:", l.prompt_key, "status:", l.status);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
