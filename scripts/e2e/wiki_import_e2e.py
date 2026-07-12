"""Task#98:Playwright 驱动 117 生产前端,完整走 29KB 星零感 MD 导入端到端:
step1 粘贴 → runPreview(分块抽取,最长300s)→ 断言 step2 多条 chunks(非0)→ runApply → 断言拿到 chunkIds。
apply 落库为 draft/needs_review(红线),不进检索,安全。DB 校验由配套 mongosh 脚本做。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_import_e2e.py"""
import json
import sys
from playwright.sync_api import sync_playwright

BASE = "http://117.72.54.28:3003"
OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"
MD = "E:/yw/agiatme/工作项目/知识库/星零感微孔去眼袋_AI知识库.md"
RUN_TAG = sys.argv[1] if len(sys.argv) > 1 else "e2e1"


def main():
    with open(MD, "r", encoding="utf-8") as f:
        md_content = f.read()
    print(f"MD loaded: {len(md_content)} chars, {md_content.count(chr(10))+1} lines, tag={RUN_TAG}")

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1560, "height": 960})
        page.set_default_timeout(20000)

        captured = {"preview": None, "preview_status": None, "apply": None, "apply_status": None}

        def on_response(resp):
            if "import-preview" in resp.url:
                captured["preview_status"] = resp.status
                try:
                    captured["preview"] = resp.json()
                except Exception as e:
                    captured["preview"] = {"_parse_error": str(e), "_text": resp.text()[:2000]}
            if "import-apply" in resp.url and "import-apply-" not in resp.url:
                captured["apply_status"] = resp.status
                try:
                    captured["apply"] = resp.json()
                except Exception as e:
                    captured["apply"] = {"_parse_error": str(e), "_text": resp.text()[:2000]}
        page.on("response", on_response)
        page.on("requestfailed", lambda req: print(f"[REQFAIL] {req.url} :: {req.failure}"))
        page.on("console", lambda msg: print(f"[CONSOLE.{msg.type}] {msg.text[:200]}") if msg.type == "error" else None)
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

        # 填来源名称(带唯一tag破缓存) + 粘贴完整 MD
        src_name = f"星零感E2E-{RUN_TAG}"
        page.locator("input[placeholder*='来源名称']").first.fill(src_name)
        ta = page.locator("textarea[placeholder*='粘贴']").first
        ta.fill(md_content)
        print(f"filled textarea len={len(ta.input_value())} sourceName={src_name!r}")

        # 点预览,等 import-preview(分块并发,给 300s)
        btn = page.locator("button.wikiBtn:has-text('预览'), button:has-text('下一步')").first
        print("clicking 预览 ... (分块抽取,最长 720s)")
        t_preview = __import__("time").time()
        try:
            with page.expect_response(lambda r: "import-preview" in r.url, timeout=720000):
                btn.click()
            print(f"import-preview responded: HTTP {captured['preview_status']}")
        except Exception as e:
            print(f"!! preview expect_response failed: {str(e)[:150]}")
            page.screenshot(path=f"{OUT}/e2e_preview_fail.png", full_page=True)
        page.wait_for_timeout(2500)
        page.wait_for_load_state("networkidle")
        page.screenshot(path=f"{OUT}/e2e_02_preview.png", full_page=True)

        pv = captured["preview"] or {}
        chunks = pv.get("chunks") or [] if isinstance(pv, dict) else []
        items = pv.get("items") or [] if isinstance(pv, dict) else []
        rep = pv.get("importReport") or {} if isinstance(pv, dict) else {}
        print(f"=== PREVIEW status={captured['preview_status']} chunks={len(chunks)} items={len(items)} importReport={json.dumps(rep, ensure_ascii=False)}")
        if not chunks:
            print("[FAIL] preview 返回 0 chunks,分块修复未生效")
            browser.close()
            sys.exit(2)

        # step2 → 应用:定位应用按钮(steward.tsx runApply)
        page.wait_for_timeout(800)
        apply_btn = page.locator("button:has-text('应用'), button:has-text('导入'), button.wikiBtn:has-text('确认')").first
        apply_cnt = page.locator("button:has-text('应用')").count()
        print(f"apply btn count(应用)={apply_cnt}")
        try:
            with page.expect_response(lambda r: ("import-apply" in r.url and "import-apply-" not in r.url), timeout=120000):
                apply_btn.click()
            print(f"import-apply responded: HTTP {captured['apply_status']}")
        except Exception as e:
            print(f"!! apply expect_response failed: {str(e)[:150]}")
            page.screenshot(path=f"{OUT}/e2e_apply_fail.png", full_page=True)
        page.wait_for_timeout(2500)
        page.screenshot(path=f"{OUT}/e2e_03_applied.png", full_page=True)

        ap = captured["apply"] or {}
        chunk_ids = ap.get("chunkIds") or [] if isinstance(ap, dict) else []
        print(f"=== APPLY status={captured['apply_status']} chunkIds={len(chunk_ids)}")
        if chunk_ids[:8]:
            print("chunkIds[:8]=", chunk_ids[:8])

        # 落盘完整结果供 DB 校验
        with open(f"{OUT}/e2e_result_{RUN_TAG}.json", "w", encoding="utf-8") as f:
            json.dump({"sourceName": src_name, "preview": {"chunks": len(chunks), "items": len(items), "importReport": rep},
                       "apply": {"status": captured["apply_status"], "chunkIds": chunk_ids}}, f, ensure_ascii=False, indent=2)

        browser.close()
        ok = len(chunks) > 0 and (rep.get("totalSegments") or 1) > 1 and captured["apply_status"] == 200 and len(chunk_ids) > 0
        print("[PASS] 前端端到端:多段分块+多chunks+应用落库成功" if ok else "[FAIL] 端到端未达预期")
        sys.exit(0 if ok else 2)


if __name__ == "__main__":
    main()
