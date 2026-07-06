// Reproduce dialogue-simulation field mismatch: frontend sends inboundText/runMode/dryRun,
// backend expects messages[]. Then confirm correct payload works.
const BASE = "http://localhost:8080";
const WXID = "e2e_customer_001";
const ACCOUNT = "e2e_acct_1";

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
  const { MongoClient } = await import("mongodb");
  const mc = new MongoClient("mongodb://127.0.0.1:27017");
  await mc.connect();
  const db = mc.db("wechatagent_local_e2e");
  const ct = await db.collection("contacts").findOne({ wxid: WXID });
  const id = ct._id.toString();
  await mc.close();

  // ---- (A) EXACT FRONTEND PAYLOAD (userOpsStore.ts:787-796) ----
  console.log("--> (A) frontend payload {inboundText, runMode, dryRun}");
  let res = await fetch(`${BASE}/api/user-operations/simulations/dialogue`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ accountId: ACCOUNT, contactId: id, inboundText: "你们的课程怎么收费？", runMode: "once", dryRun: true }),
  });
  console.log("   HTTP", res.status, "->", (await res.text()).slice(0, 200));

  // ---- (B) BACKEND-EXPECTED PAYLOAD {messages[]} ----
  console.log("--> (B) backend payload {messages: [...]}");
  const t0 = Date.now();
  res = await fetch(`${BASE}/api/user-operations/simulations/dialogue`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ accountId: ACCOUNT, contactId: id, messages: ["你们的课程怎么收费？"] }),
  });
  const text = await res.text();
  console.log("   HTTP", res.status, `(${Date.now()-t0}ms)`);
  if (res.ok) {
    const j = JSON.parse(text);
    console.log("   runMode:", j.runMode, "applied:", j.applied, "turns:", (j.items||[]).length);
  } else {
    console.log("   BODY:", text.slice(0, 300));
  }
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
