// 通用深度写流程：登录 → 进指定频道 → 按 placeholder/顺序填创建表单 → 提交 →
// 抓 POST 结果 + console 错误。DB 落库校验由外层 mongosh 单独做。
// 用法：CHANNEL=referral node deep_crud.mjs   (CHANNEL ∈ referral|content)
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/e2e-out";
const CH = process.env.CHANNEL || "referral";
const STAMP = process.env.STAMP || "s1";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 各频道：导航 label、创建 API 路径、填表逻辑（用 page 定位）、提交按钮文本。
const CONFIGS = {
  referral: {
    nav: "专属顾问",
    apiPath: "/api/referral-cards",
    submit: /保存（待审核）|保存/,
    fill: async (page) => {
      // displayName 是第一个 input；targetWxid 用占位符定位。
      const wxidBox = page.getByPlaceholder(/用于发送名片的微信 wxid/);
      const inputs = await page.locator("form input").all();
      if (inputs[0]) await inputs[0].fill(`biztest_e2e_顾问_${STAMP}`);
      if (await wxidBox.count()) await wxidBox.fill(`biztest_e2e_wxid_${STAMP}`);
    },
    // DB 校验字段（displayName）
    expectField: `biztest_e2e_顾问_${STAMP}`,
  },
  content: {
    nav: "内容资产",
    apiPath: "/api/content-assets",
    submit: /保存资产/,
    fill: async (page) => {
      // 类型 select 默认第一项即可；标题是必填。标题 input 无占位符，用 label 文案定位其兄弟 input。
      // 表单结构：<label><span>标题</span><input/></label> —— 取"标题"label 下的 input。
      const titleLabel = page.locator("label", { hasText: "标题" }).first();
      const titleInput = titleLabel.locator("input").first();
      if (await titleInput.count()) await titleInput.fill(`biztest_e2e_资产_${STAMP}`);
      else {
        const inputs = await page.locator("form input").all();
        if (inputs[1]) await inputs[1].fill(`biztest_e2e_资产_${STAMP}`);
      }
    },
    expectField: `biztest_e2e_资产_${STAMP}`,
  },
};

async function main() {
  const cfg = CONFIGS[CH];
  if (!cfg) { process.stdout.write(`unknown CHANNEL=${CH}\n`); process.exit(1); }
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"] });
  const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
  const consoleErrors = [], apiCalls = [];
  page.on("console", (m) => { if (m.type() === "error") consoleErrors.push(m.text().slice(0, 200)); });
  page.on("response", (res) => {
    if (res.url().includes(cfg.apiPath) && res.request().method() === "POST")
      apiCalls.push({ status: res.status(), url: res.url().replace(BASE, "") });
  });

  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', process.env.ADMIN_USER || "admin");
  await page.fill('input[autocomplete="current-password"]', process.env.ADMIN_PASS || "admin");
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });

  await page.locator('nav[aria-label="Product channels"] button', { hasText: cfg.nav }).first().click();
  await sleep(1500);
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});

  const result = { channel: CH, expectField: cfg.expectField, apiCalls: [], consoleErrors: [], filled: false };
  try {
    await cfg.fill(page);
    result.filled = true;
    await page.screenshot({ path: `${OUT}/deep_${CH}_filled.png` });
    await page.locator("button", { hasText: cfg.submit }).first().click({ timeout: 8000 });
    await sleep(2000);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  } catch (e) {
    result.fillError = String(e).slice(0, 300);
  }
  await page.screenshot({ path: `${OUT}/deep_${CH}_after.png` });
  result.apiCalls = apiCalls;
  result.consoleErrors = consoleErrors;
  writeFileSync(`${OUT}/deep_${CH}_report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`channel=${CH} filled=${result.filled} apiPOST=${JSON.stringify(apiCalls)} consoleErr=${consoleErrors.length} ${result.fillError ? "fillError=" + result.fillError : ""}\n`);
  await browser.close();
}
main().catch((e) => { process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
