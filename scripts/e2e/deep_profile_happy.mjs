// Happy-path test: set a substantive profile note (drives build_initial_operation_profile LLM),
// then verify DB write + llm_call_logs, then re-run analyze-profile on the now-rich contact.
import { MongoClient } from "mongodb";

const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";
const NOTE = "这位客户叫李婷，35岁，两个孩子的妈妈（一个上小学三年级、一个幼儿园大班）。" +
  "上周通过朋友圈广告加的微信，主动问过我们少儿编程课的价格和上课时间，说孩子对搭积木和游戏很感兴趣。" +
  "她比较看重师资和课程体系，也提到预算有限、希望先试听。目前还在对比其他几家机构，没有决定报名。";

function extractCookie(res) {
  const raw = res.headers.get("set-cookie") || "";
  const m = raw.match(/wa_session=[^;]+/);
  if (!m) throw new Error("no wa_session cookie: " + raw);
  return m[0];
}

async function login() {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
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

  console.log("--> PUT profile-note (substantive) ...");
  const t0 = Date.now();
  const res = await fetch(`${BASE}/api/contacts/${id}/profile-note`, {
    method: "PUT",
    headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ humanProfileNote: NOTE }),
  });
  const elapsed = Date.now() - t0;
  const text = await res.text();
  console.log("profile-note HTTP", res.status, `(${elapsed}ms)`);
  if (!res.ok) { console.log("BODY:", text.slice(0, 600)); }
  else {
    const item = JSON.parse(text).item || {};
    console.log("RESP agentProfile:", !!item.agentProfile, "customerStage:", item.customerStage,
      "intentLevel:", item.intentLevel, "operationState:", item.operationState);
    if (item.agentProfile) {
      console.log("  profile keys:", Object.keys(item.agentProfile));
    }
  }

  const updated = await db.collection("contacts").findOne({ _id: ct._id });
  console.log("CONTACT after:", {
    agent_profile: !!updated.agent_profile,
    profile_attributes_len: Array.isArray(updated.profile_attributes) ? updated.profile_attributes.length : updated.profile_attributes,
    operation_state: updated.operation_state,
    customer_stage: updated.customer_stage,
    intent_level: updated.intent_level,
  });

  const logs = await db.collection("llm_call_logs").find({}).sort({ _id: -1 }).limit(3).toArray();
  for (const l of logs) console.log("  llm_log:", l.prompt_key, "status:", l.status);

  await mc.close();
}

main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
