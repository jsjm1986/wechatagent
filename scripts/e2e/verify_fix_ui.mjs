// 验证 F-005(联系人选择器漏传accountId)+F-020(campaign圈人)前端修复(117本机跑真Chrome)。
// 关键:先把当前账号设为102(localStorage),否则 currentAccountId() 回落 accounts[0]。
// 判据来自 explore 逐行读证的交互结构。
import { chromium } from "playwright-core";
import { readFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/verify-fix-ui";

function readEnv() {
  const txt = readFileSync("/opt/wechatagent/.env", "utf8");
  const get = (k) => { const m = txt.match(new RegExp("^" + k + "=(.*)$", "m")); return m ? m[1].trim().replace(/^["']|["']$/g, "") : ""; };
  return { user: get("BOOTSTRAP_ADMIN_USERNAME"), pass: get("BOOTSTRAP_ADMIN_PASSWORD") };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const { user, pass } = readEnv();
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const results = {};

  // 登录
  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', user);
  await page.fill('input[autocomplete="current-password"]', pass);
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });
  console.log("登录 OK");

  // 关键:设当前账号=102(先确认102在账号列表)
  const accts = await page.evaluate(async () => {
    const r = await fetch("/api/accounts", { credentials: "include" });
    return (await r.json());
  });
  const acctList = Array.isArray(accts) ? accts : (accts.items || accts.accounts || []);
  const has102 = acctList.some((a) => String(a.accountId ?? a.account_id) === "102");
  console.log("账号列表含102: " + has102 + " (共" + acctList.length + "个账号)");
  await page.evaluate(() => localStorage.setItem("wechatagent.accountId", "102"));
  await page.reload({ waitUntil: "networkidle", timeout: 30000 });
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });

  const nav = page.locator('nav[aria-label="Product channels"]');

  // === F-005a 决策链联系人选择器 ===
  try {
    await nav.getByRole("button", { name: "请示通道配置" }).click({ timeout: 10000 });
    await sleep(1500);
    await page.getByRole("button", { name: "+ 从联系人添加" }).click({ timeout: 10000 });
    await sleep(2000); // 等 /api/contacts 回
    const emptyVisible = await page.getByText("无可选联系人").isVisible().catch(() => false);
    const errVisible = await page.getByRole("alert").isVisible().catch(() => false);
    // pickerItem 计数:搜索框出现即 picker 打开,统计其下联系人按钮
    const searchVisible = await page.getByPlaceholder("搜索联系人（昵称/备注/wxid）").isVisible().catch(() => false);
    // 联系人候选按钮:picker 面板内除"取消"外的 button 数(粗略),用 wxid span 更准
    const itemCount = await page.locator('button', { has: page.locator('span') }).count().catch(() => -1);
    await page.screenshot({ path: `${OUT}/f005a-decider.png` }).catch(() => {});
    results.f005a = { searchVisible, emptyVisible, errVisible, note: "emptyVisible=true 表示仍空(修复失败);false+搜索框出现表示有联系人" };
    console.log("F-005a 决策链: 搜索框=" + searchVisible + " 空态'无可选联系人'=" + emptyVisible + " 错误alert=" + errVisible);
  } catch (e) { results.f005a = { error: String(e).slice(0, 200) }; console.log("F-005a 异常: " + String(e).slice(0, 150)); }

  // === F-005b 成交登记联系人选择器 ===
  try {
    await nav.getByRole("button", { name: "产品与成交" }).click({ timeout: 10000 });
    await sleep(1200);
    await page.getByRole("button", { name: "成交记录" }).click({ timeout: 10000 });
    await sleep(2000); // 等 ContactPicker 的 /api/contacts
    const friendSearchVisible = await page.getByPlaceholder("搜索好友（昵称/备注/wxid）").isVisible().catch(() => false);
    // ContactPicker 无空态文案:数 .pickerList 下 button。用 placeholder 附近容器粗略统计所有含文本的小按钮
    // 更稳:统计页面里"选择好友查看成交"空态是否消失+联系人按钮出现
    const pickHint = await page.getByText("请选择好友").isVisible().catch(() => false);
    await page.screenshot({ path: `${OUT}/f005b-deals.png` }).catch(() => {});
    results.f005b = { friendSearchVisible, pickHint, note: "friendSearchVisible=true 表示ContactPicker渲染;需截图看列表是否有联系人" };
    console.log("F-005b 成交登记: 好友搜索框=" + friendSearchVisible + " 右侧'请选择好友'空态=" + pickHint);
  } catch (e) { results.f005b = { error: String(e).slice(0, 200) }; console.log("F-005b 异常: " + String(e).slice(0, 150)); }

  // === F-020 campaign 圈人 ===
  try {
    await nav.getByRole("button", { name: "活动" }).click({ timeout: 10000 });
    await sleep(1500);
    await page.getByRole("button", { name: "新建活动" }).click({ timeout: 10000 });
    await sleep(1000);
    await page.getByPlaceholder("如：双11老客续费7折").fill("验证活动-fix-" + Date.now());
    await page.getByPlaceholder(/活动要点/).fill("验证圈人是否命中当前账号联系人");
    await sleep(500);
    await page.getByRole("button", { name: "圈人预览" }).click({ timeout: 10000 });
    await sleep(4000); // 等圈人
    const hitText = await page.getByText(/命中\s*\d+\s*人/).first().textContent().catch(() => "");
    const hitNum = (hitText.match(/命中\s*(\d+)\s*人/) || [])[1];
    await page.screenshot({ path: `${OUT}/f020-campaign.png` }).catch(() => {});
    results.f020 = { hitText: (hitText || "").trim(), hitNum: hitNum ?? null, pass: hitNum != null && Number(hitNum) > 0 };
    console.log("F-020 campaign: 圈人结果文案=\"" + (hitText || "").trim() + "\" 命中数=" + hitNum + " → " + (hitNum != null && Number(hitNum) > 0 ? "命中>0 ✓" : "命中0或未读到 ⚠️"));
  } catch (e) { results.f020 = { error: String(e).slice(0, 200) }; console.log("F-020 异常: " + String(e).slice(0, 150)); }

  console.log("\n=== 结果 JSON ===");
  console.log(JSON.stringify(results, null, 2));
  await browser.close();
}
main().catch((e) => { console.log("FATAL " + String(e)); process.exit(1); });
