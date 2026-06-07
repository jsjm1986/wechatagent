"""T13 静态走查:后端未起,用 page.route mock 过 AuthGate 登录墙 +
喂 completeness/integrity-report 真实 schema,验证治理驾驶舱 4 屏静态渲染、
精确大白话文案、无崩溃。其余 API 自然失败,正好验证 CockpitView 的 .catch 降级。

驱动系统 Chrome(channel=chrome),无需下载 Playwright 自带 chromium。
"""
import json
import os
import sys
import io

# Windows 终端 GBK 与中文输出冲突 → 强制 UTF-8
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
from playwright.sync_api import sync_playwright

SHOTS = "/tmp/kb-cockpit-shots"
os.makedirs(SHOTS, exist_ok=True)

# 后端 completeness 真实响应形状(src/routes/knowledge.rs:3367-3378):
# answeringMode + 5 维 coverage(每维 verifiedFact/methodologyOnly/pendingDraft/state)
COMPLETENESS = {
    "totalChunks": 42, "verifiedChunks": 18, "anchoredChunks": 15,
    "evidenceChunks": 6, "needsReviewChunks": 4,
    "answeringMode": "product_safe", "summary": "可安全讲产品",
    "coverage": {
        "capability":       {"verifiedFact": True,  "methodologyOnly": False, "pendingDraft": False, "state": "verified"},
        "pricing":          {"verifiedFact": False, "methodologyOnly": False, "pendingDraft": True,  "state": "draft"},
        "caseEvidence":     {"verifiedFact": True,  "methodologyOnly": False, "pendingDraft": False, "state": "verified"},
        "effectClaims":     {"verifiedFact": False, "methodologyOnly": False, "pendingDraft": False, "state": "missing"},
        "deliveryBoundary": {"verifiedFact": False, "methodologyOnly": True,  "pendingDraft": False, "state": "methodology"},
    },
    "gaps": ["效果数据维度无任何已验证知识"],
}
# integrity-report 真实形状:{ item: { total, verified, needsReview, rejected } }
INTEGRITY = {"item": {"total": 42, "verified": 18, "needsReview": 4, "rejected": 2}}

failures = []
console_errors = []


def check(cond, msg):
    mark = "PASS" if cond else "FAIL"
    if not cond:
        failures.append(msg)
    print(f"  [{mark}] {msg}")


def fulfill_json(route, payload):
    route.fulfill(status=200, content_type="application/json", body=json.dumps(payload))


with sync_playwright() as p:
    browser = p.chromium.launch(headless=True, channel="chrome")
    page = browser.new_page(viewport={"width": 1440, "height": 900})
    page.on("console", lambda m: console_errors.append(f"{m.type}: {m.text}") if m.type == "error" else None)
    page.on("pageerror", lambda e: console_errors.append(f"pageerror: {e}"))

    # --- mock 后端:过登录墙 + 喂 cockpit 两端点;其余 /api 返空让前端走降级 ---
    def handle_api(route):
        url = route.request.url
        if "/api/auth/me" in url:
            fulfill_json(route, {"username": "walkthrough", "userId": "u-test", "workspaces": ["default"], "currentWorkspace": "default"})
        elif "/api/accounts" in url:
            fulfill_json(route, {"items": []})
        elif "/api/operation-knowledge/completeness" in url:
            fulfill_json(route, COMPLETENESS)
        elif "/api/operation-knowledge/integrity-report" in url:
            fulfill_json(route, INTEGRITY)
        elif "/api/knowledge/digest/today" in url:
            # Today 模式 DigestCanvas 要求 cards/dismissedCardIds 数组(mount 瞬间会渲染)
            fulfill_json(route, {"reportDate": "2026-06-07", "status": "ready", "generatedAt": "2026-06-07T06:00:00Z", "cards": [], "dismissedCardIds": []})
        else:
            # 其余端点返回安全空响应(数组/对象都给,前端各自挑)
            fulfill_json(route, {"items": [], "item": None, "cards": [], "dismissedCardIds": [], "signals": [], "candidates": []})
    page.route("**/api/**", handle_api)

    # ========== 1. 进首页(应直接过登录墙到 Shell) ==========
    page.goto("http://localhost:5173", wait_until="networkidle", timeout=30000)
    page.wait_for_timeout(1500)
    page.screenshot(path=f"{SHOTS}/01-landing.png", full_page=True)
    print("=== 1. landing title:", page.title())
    body = page.inner_text("body")
    check("登录" not in body or "WeAgent" in body, "已过登录墙(未卡 LoginScreen)")
    check("Wiki 管理" in body, "侧栏含「Wiki 管理」频道")

    # ========== 2. 进 Wiki 管理频道 → 切「治理」模式 ==========
    page.locator("aside button", has_text="Wiki 管理").first.click(timeout=5000)
    page.wait_for_timeout(1500)
    page.screenshot(path=f"{SHOTS}/02-knowledge.png", full_page=True)
    # 知识库工作站默认 today 模式,需切到「治理」模式(StewardMode)才有 cockpit
    check("知识库工作站" in page.inner_text("body"), "进入知识库工作站")
    page.locator(".wikiModeBar button", has_text="治理").first.click(timeout=5000)
    page.wait_for_timeout(1500)
    page.screenshot(path=f"{SHOTS}/02b-steward.png", full_page=True)
    kbody = page.inner_text("body")
    check("治理总览" in kbody, "治理模式含「治理总览」导航(cockpit 默认 pane)")
    check("批量校验" in kbody, "治理模式含「批量校验」导航")
    check("待评审" in kbody, "治理模式含「待评审」导航")

    # ========== 3. 治理总览(cockpit 主屏) ==========
    page.locator(".wikiStewardNavBtn", has_text="治理总览").first.click(timeout=5000)
    page.wait_for_timeout(1500)
    page.screenshot(path=f"{SHOTS}/03-cockpit.png", full_page=True)
    cbody = page.inner_text("body")
    print("=== 3. cockpit 文案核验 ===")
    check("正在加载知识库状态" not in cbody, "completeness 已加载(非永久 loading)")
    # AnsweringModeGauge
    check("可安全讲产品" in cbody, "仪表显示 answeringMode=product_safe 大白话「可安全讲产品」")
    check("档" in cbody, "仪表显示档位(level/3 档)")
    # CoverageVerdict 5 维大白话标签
    for dim in ["能力", "定价", "案例", "效果数据", "交付边界"]:
        check(dim in cbody, f"5 维裁决含维度「{dim}」")
    # badge 大白话(至少命中我们 mock 出来的几种态)
    check("可放心讲" in cbody, "verified 维度 badge「可放心讲」")
    check("待你审" in cbody, "draft 维度 badge「待你审」")
    check("空白·高风险" in cbody, "missing 维度 badge「空白·高风险」")
    check("只能讲思路" in cbody, "methodology 维度 badge「只能讲思路」")
    # effectClaims=missing 的安全闸大白话
    check("会被安全闸当场拦下" in cbody, "效果数据空白时的安全闸大白话警示")
    # 治理待办 MetricCard
    for kw in ["待审草稿", "需复核", "知识总数", "批量自动校验"]:
        check(kw in cbody, f"治理待办含「{kw}」")
    # 不该泄漏的英文术语(大白话红线)
    check("source_quote" not in cbody, "未泄漏后端术语 source_quote")

    # ========== 4. 批量校验(auto-verify 屏) ==========
    page.locator(".wikiStewardNavBtn", has_text="批量校验").first.click(timeout=5000)
    page.wait_for_timeout(1200)
    page.screenshot(path=f"{SHOTS}/04-autoverify.png", full_page=True)
    abody = page.inner_text("body")
    print("=== 4. 批量校验 文案核验 ===")
    check("让 AI 帮你筛一遍待处理的知识" in abody, "auto-verify 点题「让 AI 帮你筛一遍待处理的知识」")
    check("AI 不会替你放行没把握的" in abody, "auto-verify 红线「AI 不会替你放行没把握的」")
    for seg in ["宽松", "适中", "严格"]:
        check(seg in abody, f"把关松紧档「{seg}」")
    check("留一批我复查" in abody, "含「留一批我复查」开关")
    check("开始筛" in abody, "含「开始筛」按钮")

    # ========== 5. 待评审(ReviewView + ReviewChat) ==========
    page.locator(".wikiStewardNavBtn", has_text="待评审").first.click(timeout=5000)
    page.wait_for_timeout(1200)
    page.screenshot(path=f"{SHOTS}/05-review.png", full_page=True)
    print("=== 5. 待评审 已截图(空数据态)")

    # ========== 视觉合规:左缘竖色杠探测 ==========
    # 设计红线:禁用左缘竖色杠。探测 cockpit 5 维行是否有 border-left / box-shadow 竖杠。
    page.locator(".wikiStewardNavBtn", has_text="治理总览").first.click(timeout=5000)
    page.wait_for_timeout(1000)
    bar_count = page.evaluate("""() => {
      const els = Array.from(document.querySelectorAll('*'));
      let n = 0;
      for (const el of els) {
        const s = getComputedStyle(el);
        const blw = parseFloat(s.borderLeftWidth) || 0;
        const otherw = Math.max(parseFloat(s.borderTopWidth)||0, parseFloat(s.borderRightWidth)||0, parseFloat(s.borderBottomWidth)||0);
        // 左边框 >=3px 且明显粗于其它边 => 疑似竖色杠
        if (blw >= 3 && blw > otherw + 1.5) n++;
      }
      return n;
    }""")
    print("=== 视觉合规:疑似左缘竖色杠元素数 =", bar_count)
    check(bar_count == 0, "无左缘竖色杠(设计红线)")

    print("\n=== console/page 错误数:", len(console_errors))
    for e in console_errors[:30]:
        print("  ", e)
    check(len(console_errors) == 0, "无 console/page 运行时错误")

    browser.close()

print("\n========== 走查结论 ==========")
if failures:
    print(f"FAIL × {len(failures)}:")
    for f in failures:
        print("  -", f)
    print("截图在", SHOTS)
    sys.exit(1)
else:
    print("ALL PASS · 截图在", SHOTS)
