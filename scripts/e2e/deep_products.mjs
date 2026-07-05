// 深度写流程测试：products-deals 频道，真点击创建表单 → 断言 API 2xx + 无 console 错误。
// DB 落库校验由外层 mongosh 单独做（本脚本只驱动 UI + 抓网络结果）。
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/e2e-out";
const PID = process.env.TEST_PRODUCT_ID || ("biztest_e2e_" + (process.env.STAMP || "p1"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"] });
  const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
  const consoleErrors = [], apiCalls = [];
  page.on("console", (m) => { if (m.type() === "error") consoleErrors.push(m.text().slice(0, 300)); });
  page.on("response", (res) => {
    const u = res.url();
    if (u.includes("/api/products") && res.request().method() === "POST") apiCalls.push({ status: res.status(), url: u.replace(BASE, ""), method: "POST" });
  });

  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', process.env.ADMIN_USER || "admin");
  await page.fill('input[autocomplete="current-password"]', process.env.ADMIN_PASS || "admin");
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });

  // 进 产品与成交 频道
  await page.locator('nav[aria-label="Product channels"] button', { hasText: "产品与成交" }).first().click();
  await sleep(1500);
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});

  // catalog tab 默认。找到创建表单的输入框（按 placeholder / label 文本定位，先 dump 可见输入）。
  const inputs = await page.locator('form input, form textarea').all();
  const meta = [];
  for (const el of inputs) {
    meta.push({ ph: await el.getAttribute("placeholder"), name: await el.getAttribute("name"), type: await el.getAttribute("type") });
  }
  writeFileSync(`${OUT}/products_form_inputs.json`, JSON.stringify(meta, null, 2));
  await page.screenshot({ path: `${OUT}/products_before.png` });

  const result = { productId: PID, formInputs: meta, apiCalls: [], consoleErrors: [], filled: false };
  // 尝试按占位符填 productId + name（占位符文本从 index.tsx 读到）
  try {
    // productId 与 name：用第一个/第二个文本框兜底 + 尝试按占位符
    const pidBox = page.getByPlaceholder(/product|编号|ID|标识/i).first();
    const nameBox = page.getByPlaceholder(/名称|name/i).first();
    if (await pidBox.count()) await pidBox.fill(PID); else if (inputs[0]) await inputs[0].fill(PID);
    if (await nameBox.count()) await nameBox.fill("E2E 测试产品"); else if (inputs[1]) await inputs[1].fill("E2E 测试产品");
    result.filled = true;
    await page.screenshot({ path: `${OUT}/products_filled.png` });
    // 提交：点“保存产品”按钮
    await page.locator('button', { hasText: /保存产品|保存|创建/ }).first().click({ timeout: 8000 });
    await sleep(2000);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  } catch (e) {
    result.fillError = String(e).slice(0, 300);
  }
  await page.screenshot({ path: `${OUT}/products_after.png` });
  result.apiCalls = apiCalls;
  result.consoleErrors = consoleErrors;
  writeFileSync(`${OUT}/deep_products_report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`filled=${result.filled} apiPOST=${JSON.stringify(apiCalls)} consoleErr=${consoleErrors.length}\n`);
  await browser.close();
}
main().catch((e) => { process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
