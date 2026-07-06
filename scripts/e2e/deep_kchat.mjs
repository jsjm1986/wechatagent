// LLM 流程深测：知识库 Wiki → AI 协作(ChatWorkbench) → 输入一句起草请求 → 提交 →
// 等真实 LLM 返回一轮 draft。抓 POST /api/operation-knowledge/chat 结果 + 回合是否渲染。
// DB/日志三方校验由外层 mongosh 做。
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/e2e-out";
const MSG = process.env.KCHAT_MSG || "帮我起草一条知识：本机构少儿编程课程的退费规则是开课前7天可全额退款。";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox","--disable-dev-shm-usage","--disable-gpu"] });
  const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
  const consoleErrors = [], apiCalls = [];
  page.on("console", (m)=>{ if(m.type()==="error") consoleErrors.push(m.text().slice(0,200)); });
  page.on("response", (res)=>{
    const u=res.url();
    if (u.includes("/api/operation-knowledge/chat") && res.request().method()==="POST")
      apiCalls.push({ status: res.status(), url: u.replace(BASE,"") });
  });

  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', process.env.ADMIN_USER||"admin");
  await page.fill('input[autocomplete="current-password"]', process.env.ADMIN_PASS||"admin");
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });

  // 进 知识库 Wiki
  await page.locator('nav[aria-label="Product channels"] button', { hasText: "知识库 Wiki" }).first().click();
  await sleep(1500);
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(()=>{});

  const result = { apiCalls: [], consoleErrors: [], enteredChat: false, submitted: false, turnRendered: false };

  // 知识库 Wiki 默认在「工作台」mode（index.tsx:75）。点左侧 nav 的「AI 协作」按钮
  // （index.tsx:210 NavBtn label="AI 协作"）切到 ChatWorkbench pane。
  try {
    const chatNav = page.locator('button', { hasText: /^AI 协作$/ }).first();
    await chatNav.waitFor({ timeout: 8000 });
    await chatNav.click({ timeout: 5000 });
    await sleep(1200);
    result.enteredChat = true;
  } catch (e) { result.navError = String(e).slice(0,200); }

  // 定位起草输入框（today.tsx:368 placeholder 前缀"向 AI 描述要起草"）
  try {
    const box = page.getByPlaceholder(/向 AI 描述要起草/).first();
    await box.waitFor({ timeout: 8000 });
    await box.fill(MSG);
    await page.screenshot({ path: `${OUT}/kchat_filled.png` });
    // 提交按钮：ChatWorkbench 里的发送按钮。尝试 Enter + 点"发送/提交"。
    const sendBtn = page.locator('button', { hasText: /发送|提交|生成/ }).first();
    if (await sendBtn.count()) await sendBtn.click({ timeout: 5000 });
    else await box.press("Enter");
    result.submitted = true;
  } catch (e) {
    result.submitError = String(e).slice(0,300);
  }

  // 等真实 LLM 一轮（最长 120s）：轮询 POST 出现且状态回来
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    if (apiCalls.length > 0 && apiCalls.some(c=>c.status)) break;
    await sleep(3000);
  }
  await sleep(3000);
  await page.screenshot({ path: `${OUT}/kchat_after.png` });
  // 回合是否渲染：找 assistant/naturalReply 类文本区（宽松判 turn 卡片存在）
  try {
    const turnEls = await page.locator('.wikiChatWorkbench, [class*="chatTurn"], [class*="Turn"]').count();
    result.turnRendered = turnEls > 0;
  } catch {}

  result.apiCalls = apiCalls;
  result.consoleErrors = consoleErrors.filter(t=>!/auth\/me|favicon/.test(t));
  writeFileSync(`${OUT}/deep_kchat_report.json`, JSON.stringify(result, null, 2));
  process.stdout.write(`enteredChat=${result.enteredChat} submitted=${result.submitted} apiPOST=${JSON.stringify(apiCalls)} realConsoleErr=${result.consoleErrors.length} ${result.submitError?"submitErr="+result.submitError:""}\n`);
  await browser.close();
}
main().catch((e)=>{ process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
