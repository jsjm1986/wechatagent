// 批3 运营组只读+轻交互走查：overview / accountManagement / productsDeals / askHuman
// 抓 console error + /api 失败(>=400) + 每频道首屏耗时 + API 响应时间 + 截图。零真实发送。
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/fulltest/batch3-readonly";
const ADMIN_USER = process.env.ADMIN_USER || "admin";
const ADMIN_PASS = process.env.ADMIN_PASS || "admin";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();

  let current = "__login__";
  const consoleErrors = [], pageErrors = [], failedApi = [], apiTimings = [];
  page.on("console", (m) => { if (m.type() === "error") consoleErrors.push({ channel: current, text: m.text().slice(0, 400) }); });
  page.on("pageerror", (e) => pageErrors.push({ channel: current, text: String(e).slice(0, 400) }));
  page.on("response", (res) => {
    const u = res.url();
    if (u.includes("/api/")) {
      const t = res.request().timing();
      apiTimings.push({ channel: current, url: u.replace(BASE, ""), status: res.status(), method: res.request().method(), ms: t ? Math.round(t.responseEnd) : null });
      if (res.status() >= 400) failedApi.push({ channel: current, status: res.status(), url: u.replace(BASE, ""), method: res.request().method() });
    }
  });

  const result = { steps: [] };
  const log = (k, v) => { result.steps.push({ k, v }); process.stdout.write(`[${k}] ${typeof v === "string" ? v : JSON.stringify(v)}\n`); };

  // ---- 登录 ----
  const t0 = Date.now();
  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', ADMIN_USER);
  await page.fill('input[autocomplete="current-password"]', ADMIN_PASS);
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });
  log("login", `ok ${Date.now() - t0}ms`);

  const nav = (label) => page.locator('nav[aria-label="Product channels"] button', { hasText: label }).first();

  // ============ OVERVIEW ============
  current = "overview";
  let t = Date.now();
  await nav("工作台").click();
  await sleep(1200); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  log("overview.firstScreen", `${Date.now() - t}ms`);
  await page.screenshot({ path: `${OUT}/overview.png`, fullPage: true });
  // 读统计卡数字
  const ovStats = await page.locator('button').filter({ hasText: /托管联系人|托管覆盖率|在线账号/ }).allInnerTexts().catch(() => []);
  log("overview.stats", ovStats.map(s => s.replace(/\s+/g, " ").slice(0, 60)));
  // 点第一张统计卡看是否跳 userOps
  await page.locator('button').filter({ hasText: /托管联系人/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(800);
  const afterClick = await page.locator('nav[aria-label="Product channels"] button[aria-current], nav[aria-label="Product channels"] button.active').first().innerText().catch(() => "?");
  const bodyHasUserOps = await page.locator('body').innerText().then(t => t.includes("用户运营") || t.includes("联系人")).catch(() => false);
  log("overview.statCardJump", `activeNav=${afterClick} bodyHint=${bodyHasUserOps}`);
  await page.screenshot({ path: `${OUT}/overview-after-statclick.png` });

  // ============ ACCOUNT MANAGEMENT ============
  current = "accountManagement";
  t = Date.now();
  await nav("账号管理").click().catch(async () => { await nav("账号").click(); });
  await sleep(1200); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  log("acct.firstScreen", `${Date.now() - t}ms`);
  await page.screenshot({ path: `${OUT}/acct.png`, fullPage: true });
  const acctStats = await page.locator('body').innerText().then(t => t.split("\n").filter(l => /在线账号|总账号数|离线账号/.test(l)).slice(0, 6)).catch(() => []);
  log("acct.stats", acctStats);
  const acctCards = await page.locator('[class*="accountCard"]').count().catch(() => 0);
  log("acct.cardCount", acctCards);
  // 打开登录向导（不提交）
  await page.locator('button', { hasText: /登录微信账号/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(800);
  await page.screenshot({ path: `${OUT}/acct-login-wizard.png`, fullPage: true });
  const wizardVisible = await page.locator('body').innerText().then(t => t.includes("微信账号登录") || t.includes("开始登录") || t.includes("扫码")).catch(() => false);
  log("acct.wizardOpen", wizardVisible);
  // 返回
  await page.locator('button', { hasText: /返回账号列表/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(600);
  const backOk = await page.locator('body').innerText().then(t => t.includes("账号管理")).catch(() => false);
  log("acct.backOk", backOk);
  // 同步账号（只读性质，从 MCP 拉，不改客户）
  t = Date.now();
  await page.locator('button', { hasText: /同步账号/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(2500); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  log("acct.syncClicked", `${Date.now() - t}ms`);
  await page.screenshot({ path: `${OUT}/acct-after-sync.png`, fullPage: true });

  // ============ PRODUCTS & DEALS ============
  current = "productsDeals";
  t = Date.now();
  await nav("产品与成交").click();
  await sleep(1200); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  log("products.firstScreen", `${Date.now() - t}ms`);
  await page.screenshot({ path: `${OUT}/products-catalog.png`, fullPage: true });

  // --- catalog: 创建产品 b3test ---
  const STAMP = process.env.STAMP || String(Date.now()).slice(-6);
  const PID = `b3test_${STAMP}`;
  const inputs = await page.locator('form input, form textarea').all();
  log("products.formInputCount", inputs.length);
  // 按 index: [0]productId [1]name [2]price [3]currency [4]sku [5]summary
  if (inputs[0]) await inputs[0].fill(PID);
  if (inputs[1]) await inputs[1].fill("b3test 测试产品");
  if (inputs[2]) await inputs[2].fill("199.99");
  if (inputs[4]) await inputs[4].fill("SKU-B3");
  if (inputs[5]) await inputs[5].fill("批3走查测试产品，可删");
  await page.screenshot({ path: `${OUT}/products-filled.png` });
  await page.locator('button', { hasText: /^保存产品$/ }).first().click({ timeout: 8000 }).catch((e) => log("products.saveErr", String(e).slice(0, 150)));
  await sleep(2000); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  const listHasNew = await page.locator('body').innerText().then(t => t.includes(PID) || t.includes("b3test 测试产品")).catch(() => false);
  log("products.createdVisible", `pid=${PID} visible=${listHasNew}`);
  await page.screenshot({ path: `${OUT}/products-after-create.png`, fullPage: true });

  // --- 归档该产品 ---
  const row = page.locator('[class*="row"]').filter({ hasText: PID }).first();
  await row.locator('button', { hasText: /归档/ }).first().click({ timeout: 5000 }).catch((e) => log("products.archiveErr", String(e).slice(0, 150)));
  await sleep(1500); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  const archived = await page.locator('[class*="row"]').filter({ hasText: PID }).first().innerText().then(t => t.includes("已归档")).catch(() => false);
  log("products.archived", archived);
  // 恢复
  await page.locator('[class*="row"]').filter({ hasText: PID }).first().locator('button', { hasText: /恢复/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(1500); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  log("products.restored", await page.locator('[class*="row"]').filter({ hasText: PID }).first().innerText().then(t => t.includes("在售")).catch(() => false));

  // --- deals tab ---
  await page.locator('button', { hasText: /^成交记录$/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(1000); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  await page.screenshot({ path: `${OUT}/products-deals.png`, fullPage: true });
  const contactBtns = await page.locator('[class*="pickerItem"]').count().catch(() => 0);
  log("deals.contactCount", contactBtns);
  let dealDone = false;
  if (contactBtns > 0) {
    await page.locator('[class*="pickerItem"]').first().click({ timeout: 5000 }).catch(() => {});
    await sleep(1200);
    await page.screenshot({ path: `${OUT}/products-deals-selected.png`, fullPage: true });
    // 选产品
    const sel = page.locator('select').first();
    const opts = await sel.locator('option').count().catch(() => 0);
    log("deals.productOptions", opts);
    if (opts > 1) {
      await sel.selectOption({ index: 1 }).catch(() => {});
      await sleep(400);
    }
    // 金额
    const amt = page.locator('input[type="number"][step="0.01"]').first();
    await amt.fill("199.99").catch(() => {});
    // 登记成交
    await page.locator('button', { hasText: /登记成交|提交中/ }).first().click({ timeout: 8000 }).catch((e) => log("deals.submitErr", String(e).slice(0, 150)));
    await sleep(2000); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
    dealDone = await page.locator('body').innerText().then(t => t.includes("已登记成交") || t.includes("已核实")).catch(() => false);
    log("deals.registered", dealDone);
    await page.screenshot({ path: `${OUT}/products-deals-after.png`, fullPage: true });
  }

  // --- holdings tab ---
  await page.locator('button', { hasText: /^客户持有$/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(1000); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  if (contactBtns > 0) { await page.locator('[class*="pickerItem"]').first().click({ timeout: 5000 }).catch(() => {}); await sleep(1200); }
  await page.screenshot({ path: `${OUT}/products-holdings.png`, fullPage: true });
  log("holdings.body", await page.locator('body').innerText().then(t => t.split("\n").filter(l => /持有|暂无|件|有效期/.test(l)).slice(0, 5)).catch(() => []));

  // --- review tab (疑似成交) ---
  await page.locator('button', { hasText: /疑似成交待核实/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(1000); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  await page.screenshot({ path: `${OUT}/products-review.png`, fullPage: true });
  const suspectedCount = await page.locator('[class*="row"]').filter({ hasText: /待核实|置信度/ }).count().catch(() => 0);
  log("review.suspectedCount", suspectedCount);

  // ============ ASK HUMAN ============
  current = "askHuman";
  t = Date.now();
  await nav("统一收件箱").click();
  await sleep(1500); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  log("askHuman.firstScreen", `${Date.now() - t}ms`);
  await page.screenshot({ path: `${OUT}/askhuman.png`, fullPage: true });
  const chips = await page.locator('[class*="SummaryChip"]').allInnerTexts().catch(() => []);
  log("askHuman.chips", chips);
  // 点每个 chip 过滤
  const chipEls = await page.locator('[class*="SummaryChip"]').all().catch(() => []);
  for (let i = 0; i < chipEls.length && i < 8; i++) {
    await chipEls[i].click({ timeout: 3000 }).catch(() => {});
    await sleep(600);
  }
  await page.screenshot({ path: `${OUT}/askhuman-filtered.png`, fullPage: true });
  // 切到已裁决历史
  await page.locator('button', { hasText: /已裁决历史/ }).first().click({ timeout: 5000 }).catch(() => {});
  await sleep(1200); await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(() => {});
  await page.screenshot({ path: `${OUT}/askhuman-resolved.png`, fullPage: true });
  log("askHuman.resolvedBody", await page.locator('body').innerText().then(t => t.split("\n").filter(Boolean).slice(0, 8)).catch(() => []));

  // ---- 汇总 ----
  result.consoleErrors = consoleErrors;
  result.pageErrors = pageErrors;
  result.failedApi = failedApi;
  // 慢 API (>1s)
  result.slowApi = apiTimings.filter(a => a.ms != null && a.ms > 1000).sort((a, b) => b.ms - a.ms);
  result.apiTimingsSummary = {};
  for (const a of apiTimings) {
    const key = `${a.method} ${a.url.split("?")[0]}`;
    if (!result.apiTimingsSummary[key]) result.apiTimingsSummary[key] = [];
    result.apiTimingsSummary[key].push(a.ms);
  }
  writeFileSync(`${OUT}/b3_walk_report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`\n=== SUMMARY ===\nconsoleErrors=${consoleErrors.length} pageErrors=${pageErrors.length} failedApi=${failedApi.length} slowApi=${result.slowApi.length}\n`);
  process.stdout.write(`failedApi=${JSON.stringify(failedApi)}\nslowApi=${JSON.stringify(result.slowApi.slice(0, 10))}\n`);
  process.stdout.write(`consoleErrors=${JSON.stringify(consoleErrors)}\npageErrors=${JSON.stringify(pageErrors)}\n`);
  await browser.close();
}
main().catch((e) => { process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
