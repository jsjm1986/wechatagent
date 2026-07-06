// Drive content-asset create (non-LLM CRUD), verify DB persist + list round-trip.
import { MongoClient } from "mongodb";

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
  const before = await db.collection("content_assets").countDocuments({});

  console.log("--> POST /api/content-assets (frontend payload shape)");
  let res = await fetch(`${BASE}/api/content-assets`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({
      accountId: "e2e_acct_1",
      kind: "article",
      title: "少儿编程试听课介绍",
      body: "面向小学阶段孩子的图形化编程试听课，45 分钟体验搭建小游戏。",
      usageScene: "家长咨询课程内容时",
      minInjectTier: "lean",
    }),
  });
  const text = await res.text();
  console.log("   HTTP", res.status, "->", text.slice(0, 120));
  let newId = null;
  if (res.ok) newId = JSON.parse(text).id;

  const after = await db.collection("content_assets").countDocuments({});
  console.log("   count", before, "->", after);
  if (newId) {
    const doc = await db.collection("content_assets").findOne({ _id: new (await import("mongodb")).ObjectId(newId) });
    console.log("   persisted:", { kind: doc.kind, title: doc.title, min_inject_tier: doc.min_inject_tier, account_id: doc.account_id, usage_scene: doc.usage_scene });
  }

  // list round-trip via API
  res = await fetch(`${BASE}/api/content-assets?accountId=e2e_acct_1`, { headers: { Cookie: cookie } });
  const list = JSON.parse(await res.text());
  console.log("   list HTTP", res.status, "items:", (list.items||[]).length, "contains new:", (list.items||[]).some(x=>x.id===newId));

  // negative: missing title -> 400
  res = await fetch(`${BASE}/api/content-assets`, {
    method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "article", title: "  " }),
  });
  console.log("   missing-title HTTP", res.status, "->", (await res.text()).slice(0, 80));

  await mc.close();
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
