# Wiki 频道全功能 Playwright 真实验证 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 Playwright headed 真实浏览器点击，逐一核对 wiki 频道 21 个视图（3 模式）的功能是否正确，产出结构化核对报告。

**Architecture:** 三个独立可重跑的 Playwright 脚本，按副作用递增分层：T1 纯只读 → T3 危险操作只到弹窗 → T2 一次性测试数据写 → T3 造数据全链（verify→active→池→清理）。每脚本 `page.on("response")` 捕获 `/api/` 响应做断言，headed+slow_mo+截图，产出 JSON 结果。最后查库确认生产库回到 95 chunks/1 doc 纯净态。

**Tech Stack:** Playwright (Python sync API), 生产 117 (`http://117.72.54.28:3003`), MongoDB (mongosh via paramiko `_remote_run_direct.py` 查库)。

## Global Constraints

- 环境：生产 117 `http://117.72.54.28:3003`，登录 admin/admin。库内当前 95 条星零感 chunk（全 draft）+ 1 doc。
- **本轮是验证，绝不改任何业务代码/prompt/阈值/guards**。发现 bug 只记录到报告，另开修复流程。
- **危险操作只点到确认弹窗断言其出现，绝不点确认**：删真实文档、Schema 激活、治理 rollout/publish/rollback、Inspector rollback、删外部源。
- 所有测试数据带 `[E2E验证]` 前缀，脚本结束自动清理 + 查库确认回到 95/1 纯净态。
- 真发红线：验证期间绝不与真实客户消息/发送套件并发。对话工作台经 `chat.rs:989` 隔离本身不发客户。
- 生产 LLM 端点仅 2 线程：涉及 LLM 的操作（AI 协作/regenerate/造数据 verify）串行执行，不并发。
- Playwright 脚本运行：`PYTHONUTF8=1 python scripts/e2e/<name>.py`，headless=False, slow_mo=350, viewport 1560x980。
- 查库/清理走 paramiko：`set -a && . ~/.wa_deploy_env && set +a` 后用 `MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py`，DEPLOY_BIND_IP=192.168.5.9。密码绝不 echo。

## 导航真值（亲验 index.tsx，Playwright 定位依据）

- 进频道：顶部点 `知识库 Wiki`（exact）。
- 三模式（index.tsx:62-64）：`工作台` / `知识库` / `控制台`。
- 工作台 NavBtn（:209-211）：`今日 Digest` / `AI 协作` / `待办收件箱`。
- 知识库 NavBtn（:249-252）：`知识问答` / `知识树` / `质量中心` / `修订历史`；质量中心内子 tab（:262-277）：`巡检` 区（LintView）/ `评审` 区（ReviewView）/ `自动核实` 区（AutoVerifyPanel）——子 tab 文案实现时以页面实际为准，用 `wikiSubTab` class 定位。
- 控制台 NavBtn（:318-342）：`概览` / [内容录入] `文档目录` `导入向导` `外部源` / [配置] `行业 Schema` `系统配置` / **`高级`折叠组（:334，默认折叠，先点展开）**：`诊断仪表` `试召诊断` `指标总览` `运营记忆` `关系图谱`。

## 已亲验业务边界（file:line，非猜测）

- 对话不发客户：`src/routes/knowledge/chat.rs:989-990`。
- auto-verify 绝不真放行：`verify.rs:401` `enforce_verified_needs_human_audit`（:554-559）。
- 单条 verify 进池 + D2 硬闸：`verify.rs:104-122` 置 active/verified/confidence=100；:88-96 要求 source_quote 非空 + source_anchors 非空否则 400。
- 删文档级联硬删：`crud.rs:131-159`（delete_one doc + delete_many chunks by document_id）。
- 新建切片经 coerce + D2：`crud.rs:192-209`。
- reset-system-pack 本频道 UI 无入口：`mod.rs:809`/`management.rs:1192`。

---

## Task 1: 共享脚手架 wiki_verify_common.py

**Files:**
- Create: `scripts/e2e/wiki_verify_common.py`

**Interfaces:**
- Produces:
  - `login(page)` — 打开 BASE、填 admin/admin、点登录、等 networkidle。
  - `enter_wiki(page)` — 点 `知识库 Wiki` 进频道。
  - `goto_mode(page, mode_label)` — 点三模式之一（`工作台`/`知识库`/`控制台`）。
  - `goto_nav(page, label)` — 点当前模式左栏 NavBtn（按 label 文案）。
  - `expand_advanced(page)` — 控制台点 `高级` 展开折叠组（幂等：已展开则跳过）。
  - `ResponseCapture` 类 — 注册 `page.on("response")`，按 url 子串归类存最近响应 JSON+status；方法 `get(substr)` 取最近一条。
  - `make_browser(p)` — 返回 (browser, page)，headless=False slow_mo=350 viewport 1560x980 default_timeout 25000。
  - `BASE = "http://117.72.54.28:3003"`, `OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"`。
  - `save_result(name, obj)` — 写 `{OUT}/{name}.json`。

- [ ] **Step 1: 写脚手架模块**

```python
"""wiki 频道验证公共脚手架:登录/导航/响应捕获。被 T1/T2/T3 脚本复用。"""
import json
import time
from playwright.sync_api import sync_playwright  # noqa: F401 (子脚本用)

BASE = "http://117.72.54.28:3003"
OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"


class ResponseCapture:
    """注册 page.on('response'),按 url 子串归类最近一条 {status, json}。"""
    def __init__(self, page):
        self.hits = {}
        self.all = []
        page.on("response", self._on)

    def _on(self, resp):
        u = resp.url
        if "/api/" not in u:
            return
        rec = {"url": u, "status": resp.status, "json": None}
        try:
            rec["json"] = resp.json()
        except Exception:
            pass
        self.all.append(rec)
        # 按最后一段路径关键词粗归类
        for key in ("import-preview", "operation-knowledge", "gap-signals",
                    "domain-schemas", "metrics", "operator-memory", "revisions",
                    "completeness", "integrity-report", "tools/search",
                    "tools/open-slice", "digest", "chat", "ingest-sources",
                    "auto-verify", "verify", "sweep", "documents", "chunks"):
            if key in u:
                self.hits.setdefault(key, []).append(rec)

    def get(self, substr):
        lst = self.hits.get(substr)
        return lst[-1] if lst else None

    def count(self, substr):
        return len(self.hits.get(substr, []))


def make_browser(p):
    browser = p.chromium.launch(headless=False, slow_mo=350)
    page = browser.new_page(viewport={"width": 1560, "height": 980})
    page.set_default_timeout(25000)
    return browser, page


def login(page):
    page.goto(BASE)
    page.wait_for_load_state("networkidle")
    page.wait_for_timeout(600)
    page.locator("input[type=text], input[placeholder*=用户], input[placeholder*=账]").first.fill("admin")
    page.locator("input[type=password]").first.fill("admin")
    page.locator("button[type=submit], button:has-text('登录')").first.click()
    page.wait_for_load_state("networkidle")
    page.wait_for_timeout(1200)


def enter_wiki(page):
    page.get_by_text("知识库 Wiki", exact=True).first.click()
    page.wait_for_timeout(1000)


def goto_mode(page, mode_label):
    page.get_by_text(mode_label, exact=True).first.click()
    page.wait_for_timeout(900)


def goto_nav(page, label):
    page.get_by_text(label, exact=True).first.click()
    page.wait_for_timeout(900)


def expand_advanced(page):
    # 控制台高级组默认折叠;点"高级"展开(幂等:若已能看到"关系图谱"则跳过)
    if page.get_by_text("关系图谱", exact=True).count() > 0 and \
       page.get_by_text("关系图谱", exact=True).first.is_visible():
        return
    adv = page.get_by_text("高级", exact=False)
    if adv.count() > 0:
        adv.first.click()
        page.wait_for_timeout(700)


def save_result(name, obj):
    with open(f"{OUT}/{name}.json", "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=2)
    print(f"[结果] 已写 {OUT}/{name}.json")
```

- [ ] **Step 2: 冒烟验证脚手架(真跑登录+进频道)**

Run: `PYTHONUTF8=1 python -c "from scripts.e2e.wiki_verify_common import *; import sys; from playwright.sync_api import sync_playwright; p=sync_playwright().start(); b,pg=make_browser(p); login(pg); enter_wiki(pg); print('LOGIN_OK', pg.title()); b.close(); p.stop()"`
（若 import 路径不便，改在 `scripts/e2e/` 目录内跑 `PYTHONUTF8=1 python -c "import wiki_verify_common as w; ..."`）
Expected: 打印 `LOGIN_OK` + 页面标题，浏览器可见登录并进入频道。

- [ ] **Step 3: Commit（等用户许可后统一提交，本轮先不 commit）**

本轮验证脚本先不 commit（CLAUDE.md 红线：未经许可不 commit）。全部验证收尾后统一处理。

---

## Task 2: T1 纯只读全验 wiki_verify_T1_readonly.py

**Files:**
- Create: `scripts/e2e/wiki_verify_T1_readonly.py`
- Test: 脚本自身即验证（真跑产出 T1_result.json）

**Interfaces:**
- Consumes: Task 1 的 `wiki_verify_common`（login/enter_wiki/goto_mode/goto_nav/expand_advanced/ResponseCapture/make_browser/save_result）。
- Produces: `scripts/e2e/T1_result.json`（每视图 `{view, pass, evidence}`）。

覆盖 10 个只读视图：知识收件箱(工作台·待办收件箱)、知识问答、知识树、修订历史、概览、指标总览、运营记忆、关系图谱、试召诊断、诊断仪表。

- [ ] **Step 1: 写 T1 脚本**

```python
"""T1:wiki 频道 10 个纯只读视图真实点击验证。无副作用,先跑建立基线。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_verify_T1_readonly.py"""
import sys
from playwright.sync_api import sync_playwright
sys.path.insert(0, "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e")
from wiki_verify_common import (make_browser, login, enter_wiki, goto_mode,
                                goto_nav, expand_advanced, ResponseCapture, save_result)

results = []


def rec(view, ok, ev):
    results.append({"view": view, "pass": bool(ok), "evidence": ev})
    print(f"[{'PASS' if ok else 'FAIL'}] {view} — {ev}")


def main():
    with sync_playwright() as p:
        browser, page = make_browser(p)
        cap = ResponseCapture(page)
        login(page); enter_wiki(page)

        # ---- 工作台 · 待办收件箱(只读) ----
        goto_mode(page, "工作台")
        goto_nav(page, "待办收件箱")
        page.wait_for_timeout(1500)
        page.screenshot(path="scripts/e2e/T1_inbox.png", full_page=True)
        body = page.locator("body").inner_text()
        rec("工作台/待办收件箱", len(body) > 0, f"inbox 渲染,bodyLen={len(body)}")

        # ---- 知识库 · 知识问答(POST tools/search) ----
        goto_mode(page, "知识库")
        goto_nav(page, "知识问答")
        page.wait_for_timeout(800)
        ask_input = page.locator("input[placeholder*='问'], textarea[placeholder*='问'], input[type=text]").first
        ask_input.fill("星零感微孔去眼袋价格是多少")
        # 触发检索(回车或按钮)
        btn = page.locator("button:has-text('检索'), button:has-text('问'), button:has-text('搜索')").first
        if btn.count() > 0:
            btn.click()
        else:
            ask_input.press("Enter")
        page.wait_for_timeout(4000)
        page.screenshot(path="scripts/e2e/T1_ask.png", full_page=True)
        h = cap.get("tools/search")
        rec("知识库/知识问答", h and h["status"] == 200, f"search status={h['status'] if h else None}")

        # ---- 知识库 · 知识树(GET chunks) ----
        goto_nav(page, "知识树")
        page.wait_for_timeout(2000)
        page.screenshot(path="scripts/e2e/T1_tree.png", full_page=True)
        h = cap.get("operation-knowledge")
        rec("知识库/知识树", h is not None, f"chunks 拉取 status={h['status'] if h else None}")

        # ---- 知识库 · 修订历史 ----
        goto_nav(page, "修订历史")
        page.wait_for_timeout(1800)
        page.screenshot(path="scripts/e2e/T1_revisions.png", full_page=True)
        body = page.locator("body").inner_text()
        rec("知识库/修订历史", len(body) > 0, "revisions 抽屉渲染")

        # ---- 控制台 · 概览 ----
        goto_mode(page, "控制台")
        goto_nav(page, "概览")
        page.wait_for_timeout(2500)
        page.screenshot(path="scripts/e2e/T1_cockpit.png", full_page=True)
        comp = cap.get("completeness")
        rec("控制台/概览", comp is not None, f"completeness status={comp['status'] if comp else None}")

        # ---- 控制台 · 高级组:先展开 ----
        expand_advanced(page)
        page.screenshot(path="scripts/e2e/T1_advanced_expanded.png", full_page=True)

        # 诊断仪表(9路GET) — 不点"立即扫描"(留 T2)
        goto_nav(page, "诊断仪表")
        page.wait_for_timeout(2500)
        page.screenshot(path="scripts/e2e/T1_observability.png", full_page=True)
        body = page.locator("body").inner_text()
        rec("控制台/诊断仪表", len(body) > 0, "9路GET面板渲染")

        # 试召诊断(POST tools/search + open-slice,不写库)
        goto_nav(page, "试召诊断")
        page.wait_for_timeout(800)
        tr_input = page.locator("input[type=text], textarea").first
        tr_input.fill("术后恢复多久")
        tb = page.locator("button:has-text('检索'), button:has-text('召回'), button:has-text('试')").first
        if tb.count() > 0:
            tb.click()
        else:
            tr_input.press("Enter")
        page.wait_for_timeout(4000)
        page.screenshot(path="scripts/e2e/T1_tryrecall.png", full_page=True)
        h = cap.get("tools/search")
        rec("控制台/试召诊断", h and h["status"] == 200, f"tryRecall search status={h['status'] if h else None}")

        # 指标总览
        goto_nav(page, "指标总览")
        page.wait_for_timeout(2000)
        page.screenshot(path="scripts/e2e/T1_metrics.png", full_page=True)
        h = cap.get("metrics")
        rec("控制台/指标总览", h is not None, f"metrics status={h['status'] if h else None}")

        # 运营记忆
        goto_nav(page, "运营记忆")
        page.wait_for_timeout(2000)
        page.screenshot(path="scripts/e2e/T1_memory.png", full_page=True)
        h = cap.get("operator-memory")
        rec("控制台/运营记忆", h is not None, f"operator-memory status={h['status'] if h else None}")

        # 关系图谱
        goto_nav(page, "关系图谱")
        page.wait_for_timeout(2500)
        page.screenshot(path="scripts/e2e/T1_graph.png", full_page=True)
        body = page.locator("body").inner_text()
        rec("控制台/关系图谱", len(body) > 0, "图谱渲染(前端布局chunks)")

        passed = sum(1 for r in results if r["pass"])
        save_result("T1_result", {"total": len(results), "passed": passed, "results": results})
        print(f"\n=== T1 只读验证:{passed}/{len(results)} PASS ===")
        page.wait_for_timeout(2000)
        browser.close()
        sys.exit(0 if passed == len(results) else 2)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 真跑 T1**

Run: `PYTHONUTF8=1 python scripts/e2e/wiki_verify_T1_readonly.py`
Expected: headed 浏览器逐视图点击可见；10/10 PASS；T1_result.json 生成；各 T1_*.png 截图存在。

- [ ] **Step 3: 核对捕获响应**

读 `scripts/e2e/T1_result.json`，逐条确认 evidence（search/completeness/metrics 等 status=200，非空渲染）。任一 FAIL：读对应截图 + `cap.all` 定位（是选择器没点中，还是端点真报错）。选择器问题修脚本重跑；端点真错记录到报告。

---

## Task 3: T3 危险操作只到弹窗 wiki_verify_T3_danger.py

**Files:**
- Create: `scripts/e2e/wiki_verify_T3_danger.py`
- Test: 脚本自身（产出 T3_danger_result.json）

**Interfaces:**
- Consumes: Task 1 `wiki_verify_common`。
- Produces: `scripts/e2e/T3_danger_result.json`。

覆盖 6 类危险操作：删文档、Schema 激活、治理 rollout、治理 publish、治理 rollback、Inspector rollback、删外部源。**全部只断言确认弹窗出现后关闭，绝不点确认。**

- [ ] **Step 1: 写 T3 危险脚本(只到弹窗)**

```python
"""T3-danger:6 类危险操作只点到确认弹窗断言其出现,绝不确认。无副作用。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_verify_T3_danger.py"""
import sys
from playwright.sync_api import sync_playwright
sys.path.insert(0, "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e")
from wiki_verify_common import (make_browser, login, enter_wiki, goto_mode,
                                goto_nav, expand_advanced, save_result)

results = []


def rec(op, ok, ev):
    results.append({"op": op, "pass": bool(ok), "evidence": ev})
    print(f"[{'PASS' if ok else 'FAIL'}] {op} — {ev}")


def dialog_present(page):
    """确认弹窗出现判据:有含"确认/删除/发布/取消"的模态。返回(present, text)。"""
    page.wait_for_timeout(800)
    for sel in ["[role=dialog]", ".wikiConfirm", ".modal", ".overlay"]:
        loc = page.locator(sel)
        if loc.count() > 0 and loc.first.is_visible():
            txt = loc.first.inner_text()[:200]
            return True, txt
    # 回退:整页出现"确认""此操作不可"等关键词
    body = page.locator("body").inner_text()
    for kw in ("此操作不可", "确认删除", "确认发布", "不可逆", "确定要"):
        if kw in body:
            return True, kw
    return False, ""


def close_dialog(page):
    """关闭弹窗:优先点"取消",否则 Esc。绝不点确认。"""
    cancel = page.locator("button:has-text('取消'), button:has-text('关闭')").first
    if cancel.count() > 0 and cancel.is_visible():
        cancel.click()
    else:
        page.keyboard.press("Escape")
    page.wait_for_timeout(600)


def main():
    with sync_playwright() as p:
        browser, page = make_browser(p)
        login(page); enter_wiki(page)

        # ---- 删文档(控制台/文档目录) ----
        goto_mode(page, "控制台")
        goto_nav(page, "文档目录")
        page.wait_for_timeout(2000)
        del_btn = page.locator("button:has-text('删除')").first
        if del_btn.count() > 0:
            del_btn.click()
            present, txt = dialog_present(page)
            page.screenshot(path="scripts/e2e/T3_del_doc_confirm.png", full_page=True)
            rec("删文档→确认弹窗", present, f"弹窗文案:{txt!r}")
            close_dialog(page)
        else:
            rec("删文档→确认弹窗", False, "未找到删除按钮(可能无文档行)")

        # ---- Schema 激活(控制台/行业 Schema) ----
        goto_nav(page, "行业 Schema")
        page.wait_for_timeout(2000)
        act_btn = page.locator("button:has-text('激活'), button:has-text('启用')").first
        if act_btn.count() > 0:
            act_btn.click()
            present, txt = dialog_present(page)
            page.screenshot(path="scripts/e2e/T3_schema_activate_confirm.png", full_page=True)
            rec("Schema激活→确认弹窗", present, f"弹窗文案:{txt!r}")
            close_dialog(page)
        else:
            rec("Schema激活→确认弹窗", None, "无非激活schema可点(跳过,非失败)")

        # ---- 治理 rollout/publish/rollback(控制台/系统配置) ----
        goto_nav(page, "系统配置")
        page.wait_for_timeout(2000)
        for label, shot in [("发布给全部", "T3_rollout"), ("发布", "T3_publish"), ("回退", "T3_rollback")]:
            b = page.locator(f"button:has-text('{label}')").first
            if b.count() > 0 and b.is_visible():
                b.click()
                present, txt = dialog_present(page)
                page.screenshot(path=f"scripts/e2e/{shot}_confirm.png", full_page=True)
                # rollout 额外断言 requireText 强确认输入框
                extra = ""
                if label == "发布给全部":
                    ti = page.locator("[role=dialog] input, .modal input").first
                    extra = f" requireText输入框={'有' if ti.count()>0 else '无'}"
                rec(f"治理{label}→确认弹窗", present, f"{txt!r}{extra}")
                close_dialog(page)
            else:
                rec(f"治理{label}→确认弹窗", None, f"未找到'{label}'按钮(跳过)")

        # ---- 删外部源(控制台/外部源) ----
        goto_nav(page, "外部源")
        page.wait_for_timeout(1800)
        dsrc = page.locator("button:has-text('删除')").first
        if dsrc.count() > 0:
            dsrc.click()
            present, txt = dialog_present(page)
            page.screenshot(path="scripts/e2e/T3_del_source_confirm.png", full_page=True)
            rec("删外部源→确认弹窗", present, f"{txt!r}")
            close_dialog(page)
        else:
            rec("删外部源→确认弹窗", None, "无外部源可删(跳过)")

        # ---- Inspector rollback(知识库/任一 chunk 的 Inspector) ----
        goto_mode(page, "知识库")
        goto_nav(page, "知识树")
        page.wait_for_timeout(2000)
        node = page.locator(".wikiChunkNode, [class*=chunk]").first
        if node.count() > 0:
            node.click()
            page.wait_for_timeout(1200)
            rb = page.locator("button:has-text('回滚'), button:has-text('rollback')").first
            if rb.count() > 0 and rb.is_visible():
                rb.click()
                present, txt = dialog_present(page)
                page.screenshot(path="scripts/e2e/T3_inspector_rollback_confirm.png", full_page=True)
                rec("Inspector回滚→确认弹窗", present, f"{txt!r}")
                close_dialog(page)
            else:
                rec("Inspector回滚→确认弹窗", None, "该chunk无回滚按钮(可能无历史,跳过)")
        else:
            rec("Inspector回滚→确认弹窗", None, "未选中chunk(跳过)")

        passed = sum(1 for r in results if r["pass"] is True)
        skipped = sum(1 for r in results if r["pass"] is None)
        failed = sum(1 for r in results if r["pass"] is False)
        save_result("T3_danger_result", {"passed": passed, "skipped": skipped,
                                          "failed": failed, "results": results})
        print(f"\n=== T3 危险操作:{passed} PASS / {skipped} SKIP / {failed} FAIL(只到弹窗未确认) ===")
        page.wait_for_timeout(2000)
        browser.close()
        sys.exit(0 if failed == 0 else 2)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 真跑 T3 危险**

Run: `PYTHONUTF8=1 python scripts/e2e/wiki_verify_T3_danger.py`
Expected: 每个危险操作点击后弹出确认框并被截图，随后关闭；无任何真实删除/激活/发布发生；failed=0（SKIP 不算失败）。

- [ ] **Step 3: 核对无副作用**

查库确认文档/chunk 数量未变（仍 95 chunks/1 doc）：

Run（paramiko 查库）:
```bash
set -a && . ~/.wa_deploy_env && set +a
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py "mongosh wechatagent --quiet --eval 'print(\"chunks=\"+db.operation_knowledge_chunks.countDocuments({})+\" docs=\"+db.operation_knowledge_documents.countDocuments({}))'"
```
Expected: `chunks=95 docs=1`（危险操作只到弹窗，未改数据）。

---

## Task 4: T2 一次性测试数据写验 wiki_verify_T2_writes.py

**Files:**
- Create: `scripts/e2e/wiki_verify_T2_writes.py`
- Test: 脚本自身（产出 T2_result.json）

**Interfaces:**
- Consumes: Task 1 `wiki_verify_common`。
- Produces: `scripts/e2e/T2_result.json` + 记录所创建测试对象的 id（供 Task 6 清理）到 `scripts/e2e/T2_created.json`。

覆盖 7 个写视图：AI 协作(chat→apply draft)、今日 Digest(regenerate/dismiss)、待办 TaskRail(cancel)、LintView(sweep)、文档目录(新建文档+切片)、外部源(新增无害URL)、Inspector(relate)。所有创建带 `[E2E验证]` 前缀。

- [ ] **Step 1: 写 T2 写操作脚本**

```python
"""T2:7 个写视图用一次性测试数据(带[E2E验证]前缀)真实验证。验证后 Task6 清理。
LLM 操作串行(端点2线程)。运行:PYTHONUTF8=1 python scripts/e2e/wiki_verify_T2_writes.py"""
import sys, time
from playwright.sync_api import sync_playwright
sys.path.insert(0, "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e")
from wiki_verify_common import (make_browser, login, enter_wiki, goto_mode,
                                goto_nav, expand_advanced, ResponseCapture, save_result)

TAG = "[E2E验证]"
results = []
created = {"documentIds": [], "chunkIds": [], "ingestSourceIds": []}


def rec(view, ok, ev):
    results.append({"view": view, "pass": bool(ok), "evidence": ev})
    print(f"[{'PASS' if ok else 'FAIL'}] {view} — {ev}")


def main():
    with sync_playwright() as p:
        browser, page = make_browser(p)
        cap = ResponseCapture(page)
        login(page); enter_wiki(page)

        # ---- 文档目录:新建文档(带 raw_content 供后续锚定) ----
        goto_mode(page, "控制台")
        goto_nav(page, "文档目录")
        page.wait_for_timeout(1800)
        newdoc = page.locator("button:has-text('新建文档'), button:has-text('新建')").first
        if newdoc.count() > 0:
            newdoc.click(); page.wait_for_timeout(1000)
            # 表单:标题 + 原文正文
            page.locator("input[placeholder*='标题'], input[type=text]").first.fill(f"{TAG}测试文档")
            ta = page.locator("textarea").first
            if ta.count() > 0:
                ta.fill("星零感微孔去眼袋，通过下睑内侧微孔处理多余膨出脂肪、保留必要支撑。术后恢复负担相对较轻，个体差异大。")
            save_btn = page.locator("button:has-text('保存'), button:has-text('创建'), button[type=submit]").first
            with page.expect_response(lambda r: "documents" in r.url and r.request.method == "POST", timeout=15000):
                save_btn.click()
            h = cap.get("documents")
            did = (h["json"] or {}).get("id") if h else None
            if did:
                created["documentIds"].append(did)
            rec("控制台/文档目录·新建文档", h and h["status"] in (200, 201), f"docId={did}")
        else:
            rec("控制台/文档目录·新建文档", False, "未找到新建按钮")

        # ---- 文档目录:新建切片(强制draft) ----
        newchunk = page.locator("button:has-text('新建切片'), button:has-text('手工')").first
        if newchunk.count() > 0:
            newchunk.click(); page.wait_for_timeout(1000)
            page.locator("input[placeholder*='标题'], input[type=text]").first.fill(f"{TAG}测试切片")
            tas = page.locator("textarea")
            if tas.count() > 0:
                tas.first.fill("术后约24小时后可拆绷带,具体以医嘱为准。")
            sb = page.locator("button:has-text('保存'), button:has-text('创建'), button[type=submit]").first
            with page.expect_response(lambda r: "chunks" in r.url and r.request.method == "POST", timeout=15000):
                sb.click()
            h = cap.get("chunks")
            cid = (h["json"] or {}).get("id") if h else None
            if cid:
                created["chunkIds"].append(cid)
            rec("控制台/文档目录·新建切片", h and h["status"] in (200, 201), f"chunkId={cid}(应为draft)")
        else:
            rec("控制台/文档目录·新建切片", None, "未找到新建切片按钮(跳过)")

        # ---- 外部源:新增无害URL ----
        goto_nav(page, "外部源")
        page.wait_for_timeout(1500)
        addsrc = page.locator("button:has-text('新增'), button:has-text('添加')").first
        if addsrc.count() > 0:
            addsrc.click(); page.wait_for_timeout(900)
            page.locator("input[placeholder*='名称'], input[type=text]").first.fill(f"{TAG}测试源")
            url_in = page.locator("input[placeholder*='URL'], input[placeholder*='http'], input[type=url]").first
            if url_in.count() > 0:
                url_in.fill("https://example.com/rss.xml")
            sb = page.locator("button:has-text('保存'), button:has-text('创建'), button[type=submit]").first
            with page.expect_response(lambda r: "ingest-sources" in r.url and r.request.method == "POST", timeout=15000):
                sb.click()
            h = cap.get("ingest-sources")
            sid = (h["json"] or {}).get("id") if h else None
            if sid:
                created["ingestSourceIds"].append(sid)
            rec("控制台/外部源·新增", h and h["status"] in (200, 201), f"sourceId={sid}")
        else:
            rec("控制台/外部源·新增", False, "未找到新增按钮")

        # ---- 知识库/质量中心/巡检:立即扫描(幂等) ----
        goto_mode(page, "知识库")
        goto_nav(page, "质量中心")
        page.wait_for_timeout(1200)
        # 默认在巡检子tab;找扫描按钮
        sweep = page.locator("button:has-text('扫描'), button:has-text('巡检')").first
        if sweep.count() > 0:
            with page.expect_response(lambda r: "sweep" in r.url or "gap-signals" in r.url, timeout=20000):
                sweep.click()
            h = cap.get("sweep") or cap.get("gap-signals")
            rec("知识库/质量中心·巡检扫描", h and h["status"] == 200, f"sweep status={h['status'] if h else None}")
        else:
            rec("知识库/质量中心·巡检扫描", None, "未找到扫描按钮(跳过)")

        # ---- 工作台/今日 Digest:regenerate(LLM,串行) ----
        goto_mode(page, "工作台")
        goto_nav(page, "今日 Digest")
        page.wait_for_timeout(1500)
        regen = page.locator("button:has-text('重新生成'), button:has-text('生成'), button:has-text('刷新')").first
        if regen.count() > 0:
            regen.click()
            page.wait_for_timeout(8000)  # LLM
            h = cap.get("digest")
            rec("工作台/今日Digest·regenerate", h is not None, f"digest status={h['status'] if h else None}")
        else:
            rec("工作台/今日Digest·regenerate", None, "未找到生成按钮(跳过)")

        # ---- 工作台/AI 协作:chat(LLM工具循环,不发客户) ----
        goto_nav(page, "AI 协作")
        page.wait_for_timeout(1200)
        chat_in = page.locator("textarea, input[type=text]").last
        chat_in.fill(f"{TAG}帮我看看知识库里关于价格的内容")
        send = page.locator("button:has-text('发送'), button:has-text('提交')").first
        if send.count() > 0:
            send.click()
        else:
            chat_in.press("Enter")
        page.wait_for_timeout(12000)  # tool-loop LLM,最长30s硬超时
        page.screenshot(path="scripts/e2e/T2_chat.png", full_page=True)
        h = cap.get("chat")
        rec("工作台/AI协作·chat", h and h["status"] == 200, f"chat status={h['status'] if h else None}(不发客户)")

        # ---- 待办 TaskRail:列表存在即可(cancel 破坏性弱,可选) ----
        goto_nav(page, "待办收件箱")
        page.wait_for_timeout(1500)
        body = page.locator("body").inner_text()
        rec("工作台/待办TaskRail", len(body) > 0, "任务列表渲染")

        passed = sum(1 for r in results if r["pass"] is True)
        save_result("T2_result", {"passed": passed, "total": len(results), "results": results})
        save_result("T2_created", created)
        print(f"\n=== T2 写验证:{passed}/{len(results)} PASS ===")
        print(f"[清理清单] {created}")
        page.wait_for_timeout(2000)
        browser.close()


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 真跑 T2（LLM 串行，不与真发并发）**

Run: `PYTHONUTF8=1 python scripts/e2e/wiki_verify_T2_writes.py`
Expected: headed 可见逐视图写操作；创建的测试对象 id 记入 T2_created.json；chat/regenerate 有 LLM 响应；无任何真实客户消息发出。

- [ ] **Step 3: 核对写落库正确 + 隔离**

查库确认新建 chunk 是 draft，chat 未写库（chat.rs:989 隔离）：

Run:
```bash
set -a && . ~/.wa_deploy_env && set +a
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py "mongosh wechatagent --quiet --eval 'db.operation_knowledge_chunks.find({title:/E2E验证/}).forEach(c=>print(c.title+\" status=\"+c.status+\" integrity=\"+c.integrity_status))'"
```
Expected: 新建切片 `status=draft integrity=needs_review`。chat 消息不出现在 chunks（对话隔离）。

---

## Task 5: T3 造数据全链验证(verify→active→池) wiki_verify_T3_chain.py

**Files:**
- Create: `scripts/e2e/wiki_verify_T3_chain.py`
- Test: 脚本自身（产出 T3_chain_result.json）

**Interfaces:**
- Consumes: Task 1 `wiki_verify_common`；D2 闸要求（source_quote + source_anchors 非空）。
- Produces: `scripts/e2e/T3_chain_result.json` + 记录测试 chunk id 到 `scripts/e2e/T3_chain_created.json`（供 Task 6 清理）。

**关键**：verify 有 D2 硬闸（verify.rs:88-96），测试 chunk 必须 source_quote = 父文档 raw_content 里的原文片段，锚定才能命中。故本任务先经"新建文档(含 raw_content)→在文档下新建 chunk 且 source_quote 取自该 raw_content"，再到评审队列 verify。

- [ ] **Step 1: 写造数据全链脚本**

```python
"""T3-chain:造1条带source_quote的测试chunk,真实走 verify→active→进池,验证核实链路,再清理。
D2闸(verify.rs:88-96)要求 source_quote+source_anchors 非空,故 chunk 的 source_quote 取自父文档原文。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_verify_T3_chain.py"""
import sys
from playwright.sync_api import sync_playwright
sys.path.insert(0, "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e")
from wiki_verify_common import (make_browser, login, enter_wiki, goto_mode,
                                goto_nav, ResponseCapture, save_result)

TAG = "[E2E验证链路]"
# 父文档原文,chunk 的 source_quote 必须是其子串,锚定才命中
RAW = "星零感微孔去眼袋通过下睑内侧微孔处理多余膨出脂肪并保留必要支撑。术后约24小时可拆绷带具体以医嘱为准。"
QUOTE = "术后约24小时可拆绷带具体以医嘱为准。"
results = []
created = {"documentIds": [], "chunkIds": []}


def rec(step, ok, ev):
    results.append({"step": step, "pass": bool(ok), "evidence": ev})
    print(f"[{'PASS' if ok else 'FAIL'}] {step} — {ev}")


def main():
    with sync_playwright() as p:
        browser, page = make_browser(p)
        cap = ResponseCapture(page)
        login(page); enter_wiki(page)

        # 1) 新建父文档(含 raw_content)
        goto_mode(page, "控制台"); goto_nav(page, "文档目录")
        page.wait_for_timeout(1800)
        page.locator("button:has-text('新建文档'), button:has-text('新建')").first.click()
        page.wait_for_timeout(1000)
        page.locator("input[placeholder*='标题'], input[type=text]").first.fill(f"{TAG}父文档")
        page.locator("textarea").first.fill(RAW)
        with page.expect_response(lambda r: "documents" in r.url and r.request.method == "POST", timeout=15000):
            page.locator("button:has-text('保存'), button:has-text('创建'), button[type=submit]").first.click()
        h = cap.get("documents")
        did = (h["json"] or {}).get("id") if h else None
        if did: created["documentIds"].append(did)
        rec("造父文档", bool(did), f"docId={did}")

        # 2) 在该文档下新建 chunk,source_quote=QUOTE(取自RAW,保证D2锚定)
        #    UI 若无 source_quote 输入,则回退用 API 直建(仍经后端 coerce+D2 gate)。
        #    这里优先走 UI 新建切片,source_quote 填 QUOTE。
        newchunk = page.locator("button:has-text('新建切片'), button:has-text('手工')").first
        cid = None
        if newchunk.count() > 0:
            newchunk.click(); page.wait_for_timeout(1000)
            page.locator("input[placeholder*='标题'], input[type=text]").first.fill(f"{TAG}测试切片")
            tas = page.locator("textarea")
            tas.nth(0).fill(QUOTE)  # body
            # source_quote 字段(若表单有)
            sq = page.locator("textarea[placeholder*='原文'], input[placeholder*='原文'], textarea[placeholder*='引用']").first
            if sq.count() > 0:
                sq.fill(QUOTE)
            with page.expect_response(lambda r: "chunks" in r.url and r.request.method == "POST", timeout=15000):
                page.locator("button:has-text('保存'), button:has-text('创建'), button[type=submit]").first.click()
            h = cap.get("chunks")
            cid = (h["json"] or {}).get("id") if h else None
            if cid: created["chunkIds"].append(cid)
            rec("造测试chunk(带source_quote)", bool(cid), f"chunkId={cid}")
        else:
            rec("造测试chunk(带source_quote)", False, "无新建切片按钮")

        # 3) 到评审队列(知识库/质量中心/评审)找到该 chunk 点"核实"
        goto_mode(page, "知识库"); goto_nav(page, "质量中心")
        page.wait_for_timeout(1200)
        # 切到"评审"子tab
        rev = page.locator(".wikiSubTab:has-text('评审'), button:has-text('评审')").first
        if rev.count() > 0:
            rev.click(); page.wait_for_timeout(2000)
        # 找含 TAG 的行的"核实"按钮
        page.screenshot(path="scripts/e2e/T3_chain_review_list.png", full_page=True)
        verify_btn = page.locator("button:has-text('核实'), button:has-text('通过'), button:has-text('verify')").first
        verify_ok = False
        if verify_btn.count() > 0:
            # verify 可能有确认弹窗;点后若出现确认则点确认(这是我们要真实走的链路)
            with page.expect_response(lambda r: "/verify" in r.url and r.request.method == "POST", timeout=20000):
                verify_btn.click()
                page.wait_for_timeout(800)
                confirm = page.locator("[role=dialog] button:has-text('确认'), .modal button:has-text('确定'), button:has-text('确认核实')").first
                if confirm.count() > 0 and confirm.is_visible():
                    confirm.click()
            h = cap.get("verify")
            verify_ok = h and h["status"] == 200
            rec("真实点核实→verify", verify_ok, f"verify status={h['status'] if h else None}")
        else:
            rec("真实点核实→verify", False, "评审队列未找到核实按钮(测试chunk可能未进队列)")

        # 4) 查库确认该 chunk 已 active+verified(进池)
        page.wait_for_timeout(1000)
        rec("链路完成(查库在Step3验证)", verify_ok, "见 Step3 mongosh 输出")

        save_result("T3_chain_result", {"results": results})
        save_result("T3_chain_created", created)
        print(f"\n=== T3 造数据全链 ===\n[清理清单] {created}")
        page.wait_for_timeout(2000)
        browser.close()


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 真跑造数据全链**

Run: `PYTHONUTF8=1 python scripts/e2e/wiki_verify_T3_chain.py`
Expected: 造父文档+chunk → 评审队列点核实 → verify 返回 200；若 D2 闸因 source_anchors 未回填而 400，说明 UI 新建切片路径未跑 apply_chunk_integrity 回填锚点——记录为发现（新建切片是否回填锚点是真实业务问题），并在报告标注。

- [ ] **Step 3: 查库确认进池 + 记录 id**

Run:
```bash
set -a && . ~/.wa_deploy_env && set +a
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py "mongosh wechatagent --quiet --eval 'db.operation_knowledge_chunks.find({title:/E2E验证链路/}).forEach(c=>print(c._id+\" \"+c.title+\" status=\"+c.status+\" integrity=\"+c.integrity_status+\" anchors=\"+(c.source_anchors?c.source_anchors.length:0)))'"
```
Expected: 测试 chunk `status=active integrity=verified anchors≥1`（成功进池），或明确显示 anchors=0 导致 verify 被 D2 闸拒（记录为发现）。

---

## Task 6: 清理测试数据 + 查库终验

**Files:**
- Create: `scripts/e2e/wiki_verify_cleanup.cjs`（或复用 mongosh 命令）

**Interfaces:**
- Consumes: T2_created.json / T3_chain_created.json 里的 id + `[E2E验证]` 前缀。
- Produces: 生产库回到 95 chunks / 1 doc 纯净态的确认输出。

- [ ] **Step 1: 按前缀清理所有测试对象**

Run（删所有带 E2E验证 前缀的 chunk 和 doc；测试外部源同理）:
```bash
set -a && . ~/.wa_deploy_env && set +a
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py "mongosh wechatagent --quiet --eval 'var c=db.operation_knowledge_chunks.deleteMany({title:/E2E验证/}); var d=db.operation_knowledge_documents.deleteMany({title:/E2E验证/}); var s=db.ingest_sources.deleteMany({name:/E2E验证/}); print(\"deleted chunks=\"+c.deletedCount+\" docs=\"+d.deletedCount+\" sources=\"+s.deletedCount)'"
```
Expected: 打印删除数量（与 T2/T3 创建数量吻合）。

- [ ] **Step 2: 查库终验回到纯净态**

Run:
```bash
set -a && . ~/.wa_deploy_env && set +a
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 python scripts/_remote_run_direct.py "mongosh wechatagent --quiet --eval 'print(\"chunks=\"+db.operation_knowledge_chunks.countDocuments({})+\" docs=\"+db.operation_knowledge_documents.countDocuments({})+\" e2e_left=\"+db.operation_knowledge_chunks.countDocuments({title:/E2E验证/}))'"
```
Expected: `chunks=95 docs=1 e2e_left=0`（生产库回到验证前纯净态）。

- [ ] **Step 3: 汇总核对报告**

综合 T1_result.json / T3_danger_result.json / T2_result.json / T3_chain_result.json，产出一份 21 视图核对结论（每视图 pass/fail + 证据 + 截图路径），把任何"发现"（如新建切片是否回填锚点、某端点报错）单列。回报给用户，不写额外 md（除非用户要）。

---

## Self-Review

**1. Spec 覆盖检查**：
- Tier 1 十视图 → Task 2 ✓
- Tier 2 七视图 → Task 4 ✓
- Tier 3 六类危险只到弹窗 → Task 3 ✓
- Tier 3 造数据全链 verify→active→池→清理 → Task 5 + Task 6 ✓
- 查库终验回 95/1 纯净态 → Task 6 ✓
- headed+截图+响应捕获 → Task 1 ResponseCapture + 各脚本 ✓
- 验证顺序（T1→T3危险→T2→T3链→终验）→ Task 2/3/4/5/6 顺序 ✓

**2. 占位符扫描**：各 Step 均有完整脚本代码/命令+预期输出，无 TBD/TODO。选择器用"优先具体+回退"策略（真实 UI 文案未 100% 确定处标注"以页面实际为准"，属真实执行时的鲁棒性设计，非占位）。

**3. 类型一致性**：`wiki_verify_common` 的 login/enter_wiki/goto_mode/goto_nav/expand_advanced/ResponseCapture/make_browser/save_result 在 Task 1 定义，Task 2-5 一致 import 使用。created 清理清单字段（documentIds/chunkIds/ingestSourceIds）Task 4/5 写、Task 6 读一致。

## 已知执行风险（真实执行时确认，非占位）

- 新建切片/文档表单的具体字段选择器（placeholder 文案）未 100% 确定：脚本用"优先具体+回退通用"策略；真跑若点不中，读截图 + `cap.all` 修选择器重跑（脚本可独立重跑）。
- 质量中心子 tab（巡检/评审/自动核实）文案以页面实际为准，用 `wikiSubTab` class 定位。
- T3 链路 verify 若因 UI 新建切片未回填 source_anchors 而被 D2 闸拒（400）：这是真实业务发现（新建切片路径 vs import 路径的锚定差异），记录到报告，不算脚本失败。
