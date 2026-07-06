// Drive domain-profile generate (LLM, industry adaptation keystone),
// verify candidate DomainProfile persisted as draft (is_active=false, seeded_by=generated_by_ai).
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const DESC = "我们是一家少儿编程教育机构，主要面向 6-12 岁孩子的家长，通过微信私域运营招生。" +
  "核心产品是图形化编程和 Python 启蒙课，客户关心师资、课程体系、试听体验和价格。" +
  "经营目标是把咨询的家长转化为试听、再转化为报名，长期维护家长关系促进续费和转介绍。" +
  "对话风格要亲和、专业、不販压，像一位懂教育的顾问朋友。";
const PROFILE_ID = "kids-coding-edu-e2e";

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

  console.log("--> POST /api/admin/domain-profiles/generate (LLM) ...");
  const t0 = Date.now();
  const res = await fetch(`${BASE}/api/admin/domain-profiles/generate`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ businessDescription: DESC, profileId: PROFILE_ID, displayName: "少儿编程教育运营画像" }),
  });
  const text = await res.text();
  console.log("   HTTP", res.status, `(${Date.now()-t0}ms)`);
  if (!res.ok) { console.log("   BODY:", text.slice(0, 700)); await mc.close(); return; }
  const j = JSON.parse(text);
  console.log("   resp id:", j.id, "profileId:", j.profileId);

  const prof = await db.collection("domain_profiles").findOne({ profile_id: PROFILE_ID });
  console.log("   persisted:", {
    is_active: prof.is_active,
    current_version: prof.current_version,
    version: prof.version,
    seeded_by: prof.seeded_by,
    display_name: prof.display_name,
    dimensions: Array.isArray(prof.dimensions) ? prof.dimensions.length : typeof prof.dimensions,
    has_state_machine: !!prof.generated_state_machine,
  });

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(2).toArray();
  for (const l of logs) console.log("   llm_log:", l.prompt_key, "status:", l.status);

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
