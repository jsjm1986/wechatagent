// 批3 userOps LLM/写操作直调(严格串行,一次一个)。对象=某 managed 走查联系人。
// 覆盖: analyze-profile / operating-memory GET+PUT / guide preview+apply / simulation(影子,不真发)。
// 用法: BASE/ADMIN_USER/ADMIN_PASS/CID/ACCOUNT 环境变量(CID=走查联系人 _id hex)。
const BASE = process.env.BASE || "http://localhost:3003";
const CID = process.env.CID;
const ACCOUNT = process.env.ACCOUNT || "102";
if (!CID) { console.error("ERR: 需设置 CID(走查联系人 _id hex)"); process.exit(1); }
const U = process.env.ADMIN_USER, P = process.env.ADMIN_PASS;

function cookieFrom(res) {
  const raw = res.headers.get("set-cookie") || "";
  const m = raw.match(/wa_session=[^;]+/);
  if (!m) throw new Error("no cookie: " + raw);
  return m[0];
}
async function login() {
  const r = await fetch(`${BASE}/api/auth/login`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ username: U, password: P }) });
  if (!r.ok) throw new Error("login " + r.status);
  return cookieFrom(r);
}
const out = [];
const rec = (k, v) => { out.push({ k, v }); console.log(`[${k}]`, typeof v === "string" ? v : JSON.stringify(v)); };

async function main() {
  const cookie = await login();
  const H = { Cookie: cookie, "Content-Type": "application/json" };

  // === 1. operating-memory GET (读现值) ===
  let t = Date.now();
  let r = await fetch(`${BASE}/api/contacts/${CID}/operating-memory`, { headers: H });
  let mem = r.ok ? await r.json() : null;
  rec("mem.GET", `HTTP ${r.status} ${Date.now() - t}ms hasItem=${!!(mem && mem.item)}`);

  // === 2. operating-memory PUT (写测试值, 记录原值供恢复) ===
  const testMemo = { relationshipState: { temperature: "warm-b3test", trustLevel: "medium" } };
  t = Date.now();
  r = await fetch(`${BASE}/api/contacts/${CID}/operating-memory`, { method: "PUT", headers: H, body: JSON.stringify(testMemo) });
  rec("mem.PUT", `HTTP ${r.status} ${Date.now() - t}ms`);
  // 回读确认落库
  r = await fetch(`${BASE}/api/contacts/${CID}/operating-memory`, { headers: H });
  const memAfter = r.ok ? await r.json() : null;
  const temp = memAfter?.item?.relationshipState?.temperature;
  rec("mem.PUT.verify", `temperature=${temp}`);

  // === 3. analyze-profile (LLM 画像, 串行) ===
  t = Date.now();
  r = await fetch(`${BASE}/api/contacts/${CID}/analyze-profile`, { method: "POST", headers: H, body: "{}" });
  const txt3 = await r.text();
  rec("analyzeProfile", `HTTP ${r.status} ${Date.now() - t}ms`);
  if (r.ok) {
    const j = JSON.parse(txt3); const it = j.item || {};
    rec("analyzeProfile.result", `agentProfile=${!!it.agentProfile} customerStage=${it.customerStage} operationState=${it.operationState} intentLevel=${it.intentLevel}`);
    // 画像内容抽样(合理性人工评估)
    if (it.agentProfile) rec("analyzeProfile.profileSample", JSON.stringify(it.agentProfile).slice(0, 400));
  } else rec("analyzeProfile.err", txt3.slice(0, 300));

  // === 4. guide preview (LLM 指令预览, 串行) ===
  t = Date.now();
  r = await fetch(`${BASE}/api/user-operations/guide/preview`, { method: "POST", headers: H, body: JSON.stringify({ accountId: ACCOUNT, contactId: CID, instruction: "这个客户比较关注价格，沟通时多强调性价比和长期价值，语气亲和一些。" }) });
  const txt4 = await r.text();
  rec("guide.preview", `HTTP ${r.status} ${Date.now() - t}ms`);
  let previewId = null;
  if (r.ok) {
    const it = JSON.parse(txt4).item || {};
    previewId = it.id;
    rec("guide.preview.result", `id=${previewId} hasHealth=${!!it.health} summary=${(it.summary || it.explanation || "").slice(0, 200)}`);
  } else rec("guide.preview.err", txt4.slice(0, 300));

  // === 5. guide apply (应用预览, 串行) ===
  if (previewId) {
    t = Date.now();
    r = await fetch(`${BASE}/api/user-operations/guide/apply`, { method: "POST", headers: H, body: JSON.stringify({ previewId }) });
    const txt5 = await r.text();
    rec("guide.apply", `HTTP ${r.status} ${Date.now() - t}ms`);
    if (r.ok) {
      const it = JSON.parse(txt5).item || {};
      rec("guide.apply.result", `appliedFields=${JSON.stringify(it.appliedFields || [])} skipped=${JSON.stringify(it.skippedFields || [])}`);
    } else rec("guide.apply.err", txt5.slice(0, 300));
  }

  // === 6. simulation dialogue (影子模式, 不真发, LLM 串行) ===
  t = Date.now();
  r = await fetch(`${BASE}/api/user-operations/simulations/dialogue`, { method: "POST", headers: H, body: JSON.stringify({ accountId: ACCOUNT, contactId: CID, messages: ["你们这个AI运营系统多少钱？", "能不能先试用看看效果？"] }) });
  const txt6 = await r.text();
  rec("simulation", `HTTP ${r.status} ${Date.now() - t}ms`);
  if (r.ok) {
    const j = JSON.parse(txt6);
    rec("simulation.result", `runMode=${j.runMode} applied=${j.applied} turns=${(j.items || []).length}`);
    // 话术质量抽样
    for (const turn of (j.items || []).slice(0, 2)) {
      rec("simulation.turn", JSON.stringify({ inbound: (turn.inbound || turn.userMessage || "").slice(0, 40), reply: (turn.reply || turn.replyText || turn.outbound || "").slice(0, 160), status: turn.status || turn.finalStatus }).slice(0, 300));
    }
  } else rec("simulation.err", txt6.slice(0, 300));

  console.log("\n=== DONE ===");
}
main().catch((e) => { console.error("FATAL", e.message); process.exit(1); });
