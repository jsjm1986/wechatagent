// 批1 监控只读组深度走查：operations / autonomy / quality / sendAnalytics。
// 只读走查——绝不点写按钮(立即复核/取消/开始自动校验/保存并发布/开始评测/outbox取消)。
// 采集：首屏耗时、每个 /api 响应时间、console/page 错误、4xx/5xx、tab 切换效果、空/数据态。
import { chromium } from "playwright-core";
import { writeFileSync, mkdirSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/fulltest/batch1-monitor";
const ADMIN_USER = process.env.ADMIN_USER || "admin";
const ADMIN_PASS = process.env.ADMIN_PASS || "admin";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch({
    executablePath: CHROME, headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"],
  });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();

  let current = "__login__";
  const consoleErrors = [], pageErrors = [], failedApi = [], apiTimings = [];
  page.on("console", (m) => { if (m.type() === "error") consoleErrors.push({ channel: current, text: m.text().slice(0, 400) }); });
  page.on("pageerror", (e) => pageErrors.push({ channel: current, text: String(e).slice(0, 400) }));
  page.on("requestfinished", async (req) => {
    const u = req.url();
    if (!u.includes("/api/")) return;
    try {
      const t = req.timing();
      const res = await req.response();
      apiTimings.push({ channel: current, url: u.replace(BASE, ""), method: req.method(),
        status: res ? res.status() : 0, ms: Math.round(t.responseEnd) });
    } catch { /* ignore */ }
  });
  page.on("response", (res) => {
    const u = res.url();
    if (res.status() >= 400) failedApi.push({ channel: current, status: res.status(), url: u.replace(BASE, ""), method: res.request().method() });
  });

  const report = { base: BASE, steps: [] };
  const log = (msg) => process.stdout.write(msg + "\n");

  // 登录
  const t0 = Date.now();
  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', ADMIN_USER);
  await page.fill('input[autocomplete="current-password"]', ADMIN_PASS);
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });
  report.loginMs = Date.now() - t0;
  log(`login ok in ${report.loginMs}ms`);

  // 通用：进频道并测首屏
  async function gotoChannel(id, label) {
    current = id;
    const before = { c: consoleErrors.length, p: pageErrors.length, a: failedApi.length, t: apiTimings.length };
    const start = Date.now();
    await page.locator('nav[aria-label="Product channels"] button', { hasText: label }).first().click({ timeout: 10000 });
    await sleep(1200);
    await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
    const firstScreenMs = Date.now() - start;
    await page.screenshot({ path: `${OUT}/${id}.png`, fullPage: true }).catch(() => {});
    const step = { id, label, firstScreenMs,
      consoleErr: consoleErrors.length - before.c, pageErr: pageErrors.length - before.p,
      failedApi: failedApi.length - before.a,
      apis: apiTimings.slice(before.t).map(x => ({ url: x.url, status: x.status, ms: x.ms })) };
    report.steps.push(step);
    log(`[${id}] firstScreen=${firstScreenMs}ms apis=${step.apis.length} err=${step.consoleErr}/${step.pageErr} api4xx=${step.failedApi}`);
    return step;
  }

  // 通用：点某个 tab 按钮(文本)，测切换后 API + 表格行数
  async function clickTab(id, tabText, screenshotName) {
    current = id;
    const before = apiTimings.length;
    try {
      await page.locator("button", { hasText: tabText }).first().click({ timeout: 6000 });
      await sleep(900);
      await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    } catch (e) {
      log(`  tab '${tabText}' click FAIL: ${String(e).slice(0,120)}`);
      return;
    }
    const rows = await page.locator("table tbody tr").count().catch(() => -1);
    const bodyText = (await page.locator("body").innerText().catch(() => "")).slice(0, 0);
    await page.screenshot({ path: `${OUT}/${screenshotName}.png`, fullPage: true }).catch(() => {});
    const newApis = apiTimings.slice(before).map(x => ({ url: x.url, status: x.status, ms: x.ms }));
    log(`  tab '${tabText}': rows=${rows} newApis=${JSON.stringify(newApis)}`);
    report.steps.push({ id: `${id}:tab:${tabText}`, rows, apis: newApis });
  }

  // ============ operations 任务日志 ============
  await gotoChannel("operations", "任务日志");
  for (const t of ["运营事件", "复核记录", "运行日志", "LLM 成本", "跟进任务"]) {
    await clickTab("operations", t, `operations_tab_${t}`);
  }
  // 尝试展开一条运行日志(只读)
  current = "operations";
  await page.locator("button", { hasText: "运行日志" }).first().click().catch(() => {});
  await sleep(600);
  const expandBtn = page.locator("button", { hasText: /^展开$/ }).first();
  if (await expandBtn.count()) { await expandBtn.click().catch(() => {}); await sleep(500);
    await page.screenshot({ path: `${OUT}/operations_run_expanded.png`, fullPage: true }).catch(() => {});
    log("  运行日志展开一条 ok"); }

  // ============ autonomy 自治回路监控 ============
  await gotoChannel("autonomy", "自治回路监控");
  // 切窗口 select 7d / 30d(只读拉数)
  current = "autonomy";
  const sel = page.locator("select").first();
  if (await sel.count()) {
    const before7 = apiTimings.length;
    await sel.selectOption("7d").catch(() => {}); await sleep(1000);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    log(`  切7d窗口 newApis=${JSON.stringify(apiTimings.slice(before7).map(x=>({u:x.url,s:x.status,ms:x.ms})))}`);
    const before30 = apiTimings.length;
    await sel.selectOption("30d").catch(() => {}); await sleep(1000);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    log(`  切30d窗口 newApis=${JSON.stringify(apiTimings.slice(before30).map(x=>({u:x.url,s:x.status,ms:x.ms})))}`);
    await page.screenshot({ path: `${OUT}/autonomy_30d.png`, fullPage: true }).catch(() => {});
  }

  // ============ quality 运营成效 ============
  await gotoChannel("quality", "运营成效");
  for (const t of ["知识自动校验", "公式遵守度", "产品声明标记词", "长期指标"]) {
    await clickTab("quality", t, `quality_tab_${t}`);
  }
  // 长期指标 7d/30d 切换(只读)
  current = "quality";
  await page.locator("button", { hasText: "长期指标" }).first().click().catch(() => {});
  await sleep(500);
  const qsel = page.locator("select").first();
  if (await qsel.count()) {
    const beforeQ = apiTimings.length;
    await qsel.selectOption("30d").catch(() => {}); await sleep(900);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    log(`  长期指标切30d newApis=${JSON.stringify(apiTimings.slice(beforeQ).map(x=>({u:x.url,s:x.status,ms:x.ms})))}`);
  }

  // ============ sendAnalytics 发送成效 ============
  await gotoChannel("sendAnalytics", "发送成效");
  for (const t of ["名片效果", "素材效果"]) {
    await clickTab("sendAnalytics", t, `sendAnalytics_tab_${t}`);
  }

  report.consoleErrors = consoleErrors;
  report.pageErrors = pageErrors;
  report.failedApi = failedApi;
  // 慢接口 top
  report.slowApis = [...apiTimings].sort((a,b)=>b.ms-a.ms).slice(0,15).map(x=>({url:x.url,status:x.status,ms:x.ms}));
  writeFileSync(`${OUT}/report.json`, JSON.stringify(report, null, 2));
  log(`\n=== SUMMARY === consoleErr=${consoleErrors.length} pageErr=${pageErrors.length} failedApi=${failedApi.length}`);
  log(`slowApis: ${JSON.stringify(report.slowApis.slice(0,8))}`);
  if (failedApi.length) log(`FAILED_API: ${JSON.stringify(failedApi)}`);
  if (consoleErrors.length) log(`CONSOLE_ERR: ${JSON.stringify(consoleErrors)}`);
  await browser.close();
}
main().catch((e) => { process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
