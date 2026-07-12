"""Task#86:Playwright 驱动 117 生产前端,导入星零感 MD 走 import-preview,看 LLM 真实拆块。
只做到预览(step2),不点应用。捕获 import-preview 响应 JSON 落盘。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_import_preview.py"""
import json
from playwright.sync_api import sync_playwright

BASE = "http://117.72.54.28:3003"
OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"
MD = "E:/yw/agiatme/工作项目/知识库/星零感微孔去眼袋_AI知识库.md"


def main():
    with open(MD, "r", encoding="utf-8") as f:
        md_content = f.read()
    print(f"MD loaded: {len(md_content)} chars, {md_content.count(chr(10))+1} lines")

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1560, "height": 960})
        page.set_default_timeout(20000)

        # 捕获 import-preview 响应 + 请求 + console + 页面错误
        captured = {"preview": None, "status": None}

        def on_response(resp):
            if "import-preview" in resp.url:
                captured["status"] = resp.status
                try:
                    captured["preview"] = resp.json()
                except Exception as e:
                    captured["preview"] = {"_parse_error": str(e), "_text": resp.text()[:2000]}
        page.on("response", on_response)
        page.on("request", lambda req: print(f"[REQ] {req.method} {req.url}") if ("operation-knowledge" in req.url or "import" in req.url) else None)
        page.on("requestfailed", lambda req: print(f"[REQFAIL] {req.url} :: {req.failure}"))
        page.on("console", lambda msg: print(f"[CONSOLE.{msg.type}] {msg.text[:200]}") if msg.type in ("error", "warning") else None)
        page.on("pageerror", lambda exc: print(f"[PAGEERROR] {str(exc)[:200]}"))

        # 登录
        page.goto(BASE)
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(600)
        page.locator("input[type=text], input[placeholder*=用户], input[placeholder*=账]").first.fill("admin")
        page.locator("input[type=password]").first.fill("admin")
        page.locator("button[type=submit], button:has-text('登录')").first.click()
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1200)
        print("logged in")

        # 进知识库 Wiki → 控制台 → 导入向导
        page.get_by_text("知识库 Wiki", exact=True).first.click()
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1200)
        page.get_by_text("控制台", exact=False).first.click()
        page.wait_for_timeout(1200)
        page.get_by_text("导入向导", exact=False).first.click()
        page.wait_for_timeout(1500)
        print("at import wizard")

        # 填来源名称 + 粘贴 MD
        page.locator("input[placeholder*='来源名称']").first.fill("星零感微孔去眼袋_AI知识库 v1.1.0")
        ta = page.locator("textarea[placeholder*='粘贴']").first
        ta.fill(md_content)
        print(f"filled textarea, current len={len(ta.input_value())}")
        page.screenshot(path=f"{OUT}/import_01_pasted.png", full_page=True)

        # 精确定位预览按钮(steward.tsx:768 <button className=wikiBtn onClick=runPreview>)
        btn = page.locator("button.wikiBtn:has-text('预览'), button:has-text('下一步')").first
        btn_cnt = page.locator("button:has-text('下一步')").count()
        print(f"preview btn count={btn_cnt} disabled={btn.is_disabled()}")
        # 点击并等 import-preview 响应(LLM 拆块,给 180s);若请求根本没发,90s后自己会超时暴露
        print("clicking 下一步：预览 ... (LLM 拆块,最长 180s)")
        try:
            with page.expect_response(lambda r: "import-preview" in r.url, timeout=180000) as resp_info:
                btn.click()
            resp = resp_info.value
            print(f"import-preview responded: HTTP {resp.status}")
        except Exception as e:
            print(f"!! expect_response failed: {str(e)[:150]}")
            page.screenshot(path=f"{OUT}/import_02b_after_click.png", full_page=True)
        # 等前端渲染 step2
        page.wait_for_timeout(3000)
        page.wait_for_load_state("networkidle")
        page.screenshot(path=f"{OUT}/import_02_preview.png", full_page=True)

        # dump 捕获的 preview JSON
        if captured["preview"] is not None:
            with open(f"{OUT}/import_preview_result.json", "w", encoding="utf-8") as f:
                json.dump(captured["preview"], f, ensure_ascii=False, indent=2)
            pv = captured["preview"]
            if isinstance(pv, dict):
                chunks = pv.get("chunks") or []
                doc = pv.get("document") or {}
                items = pv.get("items") or []
                print(f"=== PREVIEW: status={captured['status']} ===")
                print(f"document.title={doc.get('title')!r}")
                print(f"document.summary={(doc.get('summary') or '')[:120]!r}")
                print(f"items count={len(items)}")
                print(f"chunks count={len(chunks)}")
                for i, c in enumerate(chunks[:40]):
                    if isinstance(c, dict):
                        print(f"  chunk[{i}] wikiType={c.get('wikiType')!r} title={(c.get('title') or '')[:50]!r} bodyLen={len(c.get('body') or '')}")
        else:
            print(f"!! no import-preview captured, status={captured['status']}")

        # dump 页面上可见的候选计数文字
        try:
            hdr = page.get_by_text("候选知识", exact=False).first.inner_text(timeout=3000)
            print(f"visible header: {hdr!r}")
        except Exception as e:
            print("header read skip:", str(e)[:80])

        browser.close()
        print("IMPORT PREVIEW DONE")


if __name__ == "__main__":
    main()
