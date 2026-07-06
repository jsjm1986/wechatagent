// 前端全频道冒烟：登录 → 逐频道点击导航 → 抓 console 错误 + 失败的 /api 响应(>=400) + 截图。
// 驱动服务器本机 google-chrome，打 localhost:3003（公网 502 绕不开）。
// 用法：node smoke.mjs [channelLabelSubstr]  —— 传参只测某频道，不传测全部。
import { chromium } from "playwright-core";
import { writeFileSync, mkdirSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/e2e-out";
const ADMIN_USER = process.env.ADMIN_USER || "admin";
const ADMIN_PASS = process.env.ADMIN_PASS || "admin";
const ONLY = process.argv[2] || "";

// channels.ts 单一事实来源（label 为导航按钮可见文字）。
const CHANNELS = [
  ["command", "AI 总控"],
  ["overview", "工作台"],
  ["userOps", "用户运营"],
  ["groupOps", "微信群运营"],
  ["momentOps", "朋友圈运营"],
  ["content", "内容资产"],
  ["referralCards", "专属顾问"],
  ["askHuman", "统一收件箱"],
  ["askHumanConfig", "请示通道配置"],
  ["campaign", "活动"],
  ["productsDeals", "产品与成交"],
  ["knowledgeWiki", "知识库 Wiki"],
  ["systemStrategy", "系统策略"],
  ["llmProviders", "AI 模型配置"],
  ["operations", "任务日志"],
  ["autonomy", "自治回路监控"],
  ["evolution", "演化中心"],
  ["quality", "运营成效"],
  ["sendAnalytics", "发送成效"],
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch({
    executablePath: CHROME,
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"],
  });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();

  // 全局收集器：按当前频道 tag 归类。
  let current = "__login__";
  const consoleErrors = [];
  const pageErrors = [];
  const failedApi = [];
  const failedAssets = [];
  page.on("console", (m) => {
    if (m.type() === "error") {
      const loc = m.location();
      consoleErrors.push({ channel: current, text: m.text().slice(0, 400), url: (loc && loc.url) ? loc.url.replace(BASE, "") : "" });
    }
  });
  page.on("pageerror", (e) => pageErrors.push({ channel: current, text: String(e).slice(0, 400) }));
  page.on("response", (res) => {
    const u = res.url();
    if (res.status() >= 400) {
      if (u.includes("/api/")) {
        failedApi.push({ channel: current, status: res.status(), url: u.replace(BASE, ""), method: res.request().method() });
      } else {
        failedAssets.push({ channel: current, status: res.status(), url: u.replace(BASE, ""), method: res.request().method() });
      }
    }
  });

  const result = { base: BASE, channels: [], login: null };

  // 登录
  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', ADMIN_USER);
  await page.fill('input[autocomplete="current-password"]', ADMIN_PASS);
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });
  result.login = "ok";
  await page.screenshot({ path: `${OUT}/00-after-login.png` });

  for (const [id, label] of CHANNELS) {
    if (ONLY && !label.includes(ONLY) && id !== ONLY) continue;
    current = id;
    const before = { c: consoleErrors.length, p: pageErrors.length, a: failedApi.length };
    let navOk = false, err = null;
    try {
      // 导航按钮：nav 内 button 含 span 文本 == label
      const btn = page.locator(`nav[aria-label="Product channels"] button`, { hasText: label });
      await btn.first().click({ timeout: 10000 });
      await sleep(1500); // 让 feature lazy-load + 首屏 fetch 发出
      await page.waitForLoadState("networkidle", { timeout: 20000 }).catch(() => {});
      navOk = true;
    } catch (e) {
      err = String(e).slice(0, 300);
    }
    await page.screenshot({ path: `${OUT}/${id}.png` }).catch(() => {});
    result.channels.push({
      id, label, navOk, err,
      newConsoleErrors: consoleErrors.length - before.c,
      newPageErrors: pageErrors.length - before.p,
      newFailedApi: failedApi.length - before.a,
    });
    process.stdout.write(`[${navOk ? "OK" : "FAIL"}] ${id} (${label}) consoleErr+${consoleErrors.length - before.c} pageErr+${pageErrors.length - before.p} api4xx/5xx+${failedApi.length - before.a}\n`);
  }

  result.consoleErrors = consoleErrors;
  result.pageErrors = pageErrors;
  result.failedApi = failedApi;
  writeFileSync(`${OUT}/report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`\n=== SUMMARY ===\nconsoleErrors=${consoleErrors.length} pageErrors=${pageErrors.length} failedApi=${failedApi.length}\nreport=${OUT}/report.json\n`);

  await browser.close();
}

main().catch((e) => {
  process.stdout.write(`FATAL ${String(e)}\n`);
  process.exit(1);
});
