// 深度交互扫描：登录后逐频道进入，点击频道内每一个「非提交类」可点元素
// （sub-tab / 过滤器 / 只读切换），捕获点击后新增的 API 4xx/5xx + console 错误。
// 刻意跳过含"删除/保存/推送/发送/确认/归档/停用"字样的写/危险按钮（避免误改数据），
// 只点导航/tab/展开这类只读交互——目的是发现"某个 sub-view 一进去就报错/断链"。
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE || "http://localhost:3003";
const CHROME = process.env.E2E_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.E2E_OUT || "/tmp/e2e-out";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const CHANNELS = [
  "AI 总控","工作台","用户运营","微信群运营","朋友圈运营","内容资产","专属顾问",
  "统一收件箱","请示通道配置","活动","产品与成交","知识库 Wiki","系统策略",
  "AI 模型配置","任务日志","自治回路监控","演化中心","运营成效","发送成效",
];

// 不点这些（写/危险/触发 LLM/MCP）——本扫描只做只读交互，写流程另有专测。
const SKIP = /删除|保存|推送|发送|确认|归档|停用|启用|移除|清空|重置|生成|优化|应用|执行|同步|上传|导入|预览|开始验证|整理候选|重新分析|引荐|撤销|驳回|拒绝|通过|放行|退回|拆分|合并|关联|登出|新建活动|登记/;

async function main() {
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--no-sandbox","--disable-dev-shm-usage","--disable-gpu"] });
  const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
  let current = "__login__";
  const consoleErrors = [], failedApi = [];
  page.on("console", (m) => { if (m.type()==="error") { const l=m.location(); consoleErrors.push({ channel: current, text: m.text().slice(0,200), url: l?.url?.replace(BASE,"")||"" }); } });
  page.on("response", (res) => { if (res.status()>=400 && res.url().includes("/api/")) failedApi.push({ channel: current, status: res.status(), url: res.url().replace(BASE,""), method: res.request().method() }); });

  await page.goto(BASE, { waitUntil: "networkidle", timeout: 30000 });
  await page.fill('input[autocomplete="username"]', process.env.ADMIN_USER||"admin");
  await page.fill('input[autocomplete="current-password"]', process.env.ADMIN_PASS||"admin");
  await page.click('button[type="submit"]');
  await page.waitForSelector('nav[aria-label="Product channels"]', { timeout: 30000 });

  const report = { channels: [] };
  for (const label of CHANNELS) {
    current = label;
    await page.locator('nav[aria-label="Product channels"] button', { hasText: label }).first().click();
    await sleep(1200);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(()=>{});
    const before = { c: consoleErrors.length, a: failedApi.length };

    // 收集频道主区内所有 button（排除左侧导航），点击安全的（不匹配 SKIP、可见、可用）。
    const btns = await page.locator('main button, main [role="tab"]').all();
    let clicked = 0, safeTexts = [];
    for (const b of btns) {
      let txt = "";
      try { txt = (await b.innerText({ timeout: 500 })).trim().replace(/\s+/g," "); } catch { continue; }
      if (!txt || txt.length > 20 || SKIP.test(txt)) continue;
      try {
        if (!(await b.isVisible()) || !(await b.isEnabled())) continue;
        await b.click({ timeout: 3000 });
        clicked++; safeTexts.push(txt);
        await sleep(500);
      } catch { /* 点不动跳过 */ }
      if (clicked >= 15) break; // 每频道封顶,防止无限
    }
    await page.waitForLoadState("networkidle", { timeout: 10000 }).catch(()=>{});
    await page.screenshot({ path: `${OUT}/sweep_${label.replace(/[^\w一-龥]/g,"_")}.png` }).catch(()=>{});
    report.channels.push({ label, clickedCount: clicked, clicked: safeTexts, newConsoleErr: consoleErrors.length-before.c, newFailedApi: failedApi.length-before.a });
    process.stdout.write(`[${label}] clicked=${clicked} consoleErr+${consoleErrors.length-before.c} api4xx/5xx+${failedApi.length-before.a}\n`);
  }
  report.consoleErrors = consoleErrors;
  report.failedApi = failedApi;
  writeFileSync(`${OUT}/sweep_report.json`, JSON.stringify(report, null, 2));
  process.stdout.write(`\n=== SWEEP DONE consoleErrors=${consoleErrors.length} failedApi=${failedApi.length} ===\n`);
  await browser.close();
}
main().catch((e)=>{ process.stdout.write(`FATAL ${String(e)}\n`); process.exit(1); });
