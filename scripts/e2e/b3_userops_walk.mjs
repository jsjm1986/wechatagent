// 批3 userOps 三模式深度走查(L1/UX/P)：smart/roster/traditional。
// 抓 console error + /api 失败(>=400) + 每步耗时 + API 响应时间 + roster 大列表渲染 + 截图。
// 零真实发送(本脚本只点导航/切模式/切tab/翻页，不触发 LLM/发送动作)。
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/fulltest/batch3-userops";
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
  let t0 = Date.now();
  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', ADMIN_USER);
  await page.fill('input[autocomplete="current-password"]', ADMIN_PASS);
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });
  log("login", `ok ${Date.now() - t0}ms`);

  const nav = (label) => page.locator('nav[aria-label="Product channels"] button', { hasText: label }).first();

  // ============ 进入 用户运营 ============
  current = "userOps";
  t0 = Date.now();
  await nav("用户运营").click();
  await sleep(1500); await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
  log("userOps.firstScreen", `${Date.now() - t0}ms`);
  await page.screenshot({ path: `${OUT}/00-userops-landing.png`, fullPage: true });

  // ---- 切账号 102（AccountSwitcher）----
  // 顶部账号切换器：找含 "102" / "Demi" 的 option 或 button
  const acctInfo = await page.evaluate(() => {
    const sels = Array.from(document.querySelectorAll("select"));
    return sels.map((s) => ({ opts: Array.from(s.options).map((o) => ({ v: o.value, t: o.textContent })) }));
  });
  log("userOps.selectsFound", JSON.stringify(acctInfo).slice(0, 500));

  // 全局账号切换器：自定义 dropdown（trigger button + role=option）。默认账号1"客服A"，
  // 需展开后点含 t-1/测试1 的 option。account_id=102 alias=t-1 display=测试1。
  // 点 trigger 展开
  const acctTrigger = page.locator('button', { hasText: /在线/ }).first();
  let switched = { ok: false };
  if (await acctTrigger.count()) {
    await acctTrigger.click().catch(() => {});
    await sleep(500);
    const opt = page.locator('[role="option"]', { hasText: /t-1|测试1|102/ }).first();
    if (await opt.count()) {
      const txt = await opt.innerText().catch(() => "?");
      await opt.click().catch(() => {});
      switched = { ok: true, picked: txt };
    } else {
      // 列出所有 option 文本供诊断
      const opts = await page.locator('[role="option"]').allInnerTexts().catch(() => []);
      switched = { ok: false, optionsSeen: opts };
    }
  }
  log("userOps.acctSwitch", JSON.stringify(switched));
  await sleep(2000); await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  await page.screenshot({ path: `${OUT}/01-after-acct-switch.png`, fullPage: true });

  // ============ 模式1：smart 智能模式 ============
  current = "smart";
  // 读取 pool tabs 计数（已互动/Agent/待启用）
  const poolTabs = await page.locator('.segmented button').allInnerTexts().catch(() => []);
  log("smart.poolTabs", poolTabs.map((s) => s.replace(/\s+/g, " ")));

  // 联系人列表条数
  const contactCount = await page.locator('.contactList .contact').count().catch(() => -1);
  log("smart.contactListRendered", contactCount);

  // 点第一个联系人进驾驶舱
  if (contactCount > 0) {
    t0 = Date.now();
    await page.locator('.contactList .contact').first().click();
    await sleep(2000); await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
    log("smart.openContact", `${Date.now() - t0}ms`);
    await page.screenshot({ path: `${OUT}/02-cockpit-observe.png`, fullPage: true });

    // 切到"配置"段
    const cfgBtn = page.locator('[role="tab"]', { hasText: "配置" }).first();
    if (await cfgBtn.count()) {
      await cfgBtn.click(); await sleep(1200);
      await page.screenshot({ path: `${OUT}/03-cockpit-configure.png`, fullPage: true });
      log("smart.configureView", "ok");
    }
    // 回观测
    const obsBtn = page.locator('[role="tab"]', { hasText: "观测" }).first();
    if (await obsBtn.count()) { await obsBtn.click(); await sleep(600); }

    // 测下钻：会话/发送历史/记忆/趋势（点观测视图里的下钻入口）
    const drillTexts = ["会话", "发送历史", "记忆", "趋势"];
    for (const dt of drillTexts) {
      const btn = page.locator("button", { hasText: dt }).first();
      if (await btn.count()) {
        await btn.click().catch(() => {});
        await sleep(900);
        await page.screenshot({ path: `${OUT}/04-drill-${dt}.png` }).catch(() => {});
        // 返回
        const back = page.locator("button", { hasText: /返回|返 回|←/ }).first();
        if (await back.count()) await back.click().catch(() => {});
        await sleep(400);
      }
    }
  }

  // pool tab 切换测试
  for (const tabLabel of ["Agent", "待启用", "已互动"]) {
    const b = page.locator('.segmented button', { hasText: tabLabel }).first();
    if (await b.count()) {
      await b.click().catch(() => {}); await sleep(700);
      const n = await page.locator('.contactList .contact').count().catch(() => -1);
      log(`smart.tab.${tabLabel}`, `rendered=${n}`);
    }
  }
  // 搜索过滤
  const searchInput = page.locator('.toolbar input, input[placeholder*="过滤"]').first();
  if (await searchInput.count()) {
    await searchInput.fill("吴"); await searchInput.blur(); await sleep(1500);
    const n = await page.locator('.contactList .contact').count().catch(() => -1);
    log("smart.searchFilter", `q=吴 rendered=${n}`);
    await searchInput.fill(""); await searchInput.blur(); await sleep(1000);
  }

  // ============ 模式2：roster 花名册 ============
  current = "roster";
  // 切模式：UserOpsModeHeader 里的模式按钮
  const modeBtns = await page.locator('header button, .segmented button, [role="tab"]').allInnerTexts().catch(() => []);
  log("roster.modeButtonsSeen", modeBtns.map((s) => s.replace(/\s+/g, " ")).slice(0, 30));

  // 点包含"花名册"/"通讯录"/"名册" 的模式切换
  let rosterClicked = false;
  for (const label of ["通讯录"]) {
    const b = page.locator("button", { hasText: label }).first();
    if (await b.count()) { await b.click().catch(() => {}); rosterClicked = true; log("roster.modeSwitch", label); break; }
  }
  if (!rosterClicked) log("roster.modeSwitch", "NOT FOUND - 未找到花名册模式按钮");

  t0 = Date.now();
  await sleep(2500); await page.waitForLoadState("networkidle", { timeout: 30000 }).catch(() => {});
  log("roster.firstScreen", `${Date.now() - t0}ms`);
  await page.screenshot({ path: `${OUT}/05-roster-landing.png`, fullPage: true });

  // roster 卡片渲染数 + 分页
  const rosterCards = await page.evaluate(() => {
    // RosterView.module.css grid 下的卡片
    const grids = document.querySelectorAll('[class*="grid"]');
    let max = 0;
    grids.forEach((g) => { const n = g.querySelectorAll('button[class*="card"]').length; if (n > max) max = n; });
    return max;
  });
  log("roster.cardsPerPage", rosterCards);

  // 分页器信息
  const pagerText = await page.locator('[class*="pager"]').allInnerTexts().catch(() => []);
  log("roster.pager", pagerText.map((s) => s.replace(/\s+/g, " ")));

  // === 大列表性能：翻页渲染耗时（重点）===
  const nextBtn = page.locator('button', { hasText: "下一页" }).first();
  if (await nextBtn.count()) {
    const renderMs = [];
    for (let i = 0; i < 5; i++) {
      const isDisabled = await nextBtn.isDisabled().catch(() => true);
      if (isDisabled) break;
      const tp = Date.now();
      await nextBtn.click();
      // 等卡片重渲染（首个卡片文本变化不易测，用 rAF 两帧 + 短 sleep 近似）
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
      await sleep(120);
      renderMs.push(Date.now() - tp);
    }
    log("roster.pageFlipMs", renderMs);
  } else {
    log("roster.pageFlipMs", "no-pager(可能好友数<60或列表空)");
  }

  // roster 过滤输入
  const rosterFilter = page.locator('[class*="filter"] input, input[placeholder*="过滤"]').first();
  if (await rosterFilter.count()) {
    const tf = Date.now();
    await rosterFilter.fill("a"); await sleep(600);
    log("roster.filterMs", `${Date.now() - tf}ms`);
    await rosterFilter.fill(""); await sleep(400);
  }

  // 滚动性能：连续滚动到底测卡顿
  const scrollMs = await page.evaluate(async () => {
    const t = performance.now();
    for (let y = 0; y < 3000; y += 300) { window.scrollTo(0, y); await new Promise((r) => requestAnimationFrame(r)); }
    return Math.round(performance.now() - t);
  });
  log("roster.scrollMs", `${scrollMs}ms`);
  await page.screenshot({ path: `${OUT}/06-roster-scrolled.png` }).catch(() => {});

  // ============ 模式3：traditional 传统模式 ============
  current = "traditional";
  let tradClicked = false;
  for (const label of ["传统", "traditional", "专业"]) {
    const b = page.locator("button", { hasText: label }).first();
    if (await b.count()) { await b.click().catch(() => {}); tradClicked = true; log("traditional.modeSwitch", label); break; }
  }
  if (!tradClicked) log("traditional.modeSwitch", "NOT FOUND");
  t0 = Date.now();
  await sleep(2000); await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
  log("traditional.firstScreen", `${Date.now() - t0}ms`);
  await page.screenshot({ path: `${OUT}/07-traditional-playbooks.png`, fullPage: true });

  // 4 子 tab 遍历：playbooks / prompts / settings / audit
  const subTabs = ["运营方法", "Agent 提示词", "运行策略", "审计复盘"];
  for (const st of subTabs) {
    const b = page.locator("button", { hasText: st }).first();
    if (await b.count()) {
      const before = { c: consoleErrors.length, a: failedApi.length };
      await b.click().catch(() => {}); await sleep(1500);
      await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
      await page.screenshot({ path: `${OUT}/08-trad-${st}.png`, fullPage: true }).catch(() => {});
      log(`traditional.subtab.${st}`, `consoleErr+${consoleErrors.length - before.c} api4xx+${failedApi.length - before.a}`);
    }
  }

  // ---- 汇总 ----
  result.consoleErrors = consoleErrors;
  result.pageErrors = pageErrors;
  result.failedApi = failedApi;
  // API 计时统计
  const slow = apiTimings.filter((a) => a.ms && a.ms > 1000).sort((a, b) => b.ms - a.ms);
  result.slowApis = slow.slice(0, 20);
  result.apiCount = apiTimings.length;
  writeFileSync(`${OUT}/report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`\n=== SUMMARY ===\nconsoleErrors=${consoleErrors.length} pageErrors=${pageErrors.length} failedApi=${failedApi.length} apiCount=${apiTimings.length}\n`);
  process.stdout.write(`slowApis(>1s)=${slow.length}: ${JSON.stringify(slow.slice(0, 10))}\n`);
  process.stdout.write(`failedApi: ${JSON.stringify(failedApi.slice(0, 15))}\n`);

  await browser.close();
}
main().catch((e) => { process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
