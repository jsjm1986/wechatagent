// Drive ask-human taxonomy-candidate approve + reject, verify DB state + system_taxonomies insert.
import { MongoClient, ObjectId } from "mongodb";

const BASE = "http://localhost:8080";

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
  const pending = await db.collection("taxonomy_candidates").find({ status: "pending" }).sort({ _id: 1 }).toArray();
  if (pending.length < 2) { console.log("need >=2 pending, have", pending.length); await mc.close(); return; }
  const approveC = pending[0]; // customer_stage raw=关注
  const rejectC = pending[1];  // intent_level raw=初期咨询

  // ---- APPROVE ----
  console.log("--> APPROVE", approveC._id.toString(), "kind:", approveC.kind, "raw:", approveC.raw_value);
  let res = await fetch(`${BASE}/api/admin/taxonomy-candidates/${approveC._id}/approve`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ canonicalValue: { id: "attention", label: "关注期", aliases: ["关注"] }, reviewedBy: "e2e-admin" }),
  });
  console.log("   HTTP", res.status, "->", (await res.text()).slice(0, 160));
  const ac = await db.collection("taxonomy_candidates").findOne({ _id: approveC._id });
  const tax = await db.collection("system_taxonomies").findOne({ kind: approveC.kind, "value.id": "attention", scope: approveC.scope });
  console.log("   candidate.status:", ac.status, "reviewed_by:", ac.reviewed_by);
  console.log("   system_taxonomies inserted:", !!tax, "aliases:", tax?.value?.aliases, "seeded_by:", tax?.seeded_by);

  // ---- REJECT ----
  console.log("--> REJECT", rejectC._id.toString(), "kind:", rejectC.kind, "raw:", rejectC.raw_value);
  res = await fetch(`${BASE}/api/admin/taxonomy-candidates/${rejectC._id}/reject`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ reason: "过于泛化，不作为规范值", reviewedBy: "e2e-admin" }),
  });
  console.log("   HTTP", res.status, "->", (await res.text()).slice(0, 160));
  const rc = await db.collection("taxonomy_candidates").findOne({ _id: rejectC._id });
  console.log("   candidate.status:", rc.status, "rejection_reason:", rc.rejection_reason);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
