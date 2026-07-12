"""侦察 117 生产前端:登录→知识库 Wiki→文档导入向导,截图看 DOM 选择器。
只读侦察,不提交导入。运行:PYTHONUTF8=1 python scripts/e2e/wiki_recon.py"""
import sys
from playwright.sync_api import sync_playwright

BASE = "http://117.72.54.28:3003"
OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1560, "height": 960})
        page.set_default_timeout(20000)

        # 1) 登录
        page.goto(BASE)
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(600)
        page.screenshot(path=f"{OUT}/wiki_recon_00_landing.png", full_page=True)

        # 找用户名/密码输入框(多候选)
        try:
            user = page.locator("input[type=text], input[name=username], input[placeholder*=用户], input[placeholder*=账]").first
            pwd = page.locator("input[type=password]").first
            user.fill("admin")
            pwd.fill("admin")
            # 提交按钮
            btn = page.locator("button[type=submit], button:has-text('登录'), button:has-text('登陆')").first
            btn.click()
            page.wait_for_load_state("networkidle")
            page.wait_for_timeout(1200)
            print("login submitted")
        except Exception as e:
            print("login step issue:", e)
        page.screenshot(path=f"{OUT}/wiki_recon_01_after_login.png", full_page=True)

        # 2) 列出所有可见的频道/导航文字(诊断)
        try:
            texts = page.locator("nav a, nav button, [class*=channel] a, [class*=channel] button, aside a, aside button").all_inner_texts()
            print("=== nav candidates ===")
            for t in texts:
                t = t.strip()
                if t:
                    print(repr(t))
        except Exception as e:
            print("nav list issue:", e)

        # 3) 尝试点"知识库 Wiki"
        for label in ["知识库 Wiki", "知识库", "Wiki"]:
            try:
                page.get_by_text(label, exact=True).first.click(timeout=4000)
                page.wait_for_timeout(1000)
                print(f"clicked channel: {label}")
                break
            except Exception as e:
                print(f"channel '{label}' click skip:", str(e)[:80])
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1000)
        page.screenshot(path=f"{OUT}/wiki_recon_02_wiki_channel.png", full_page=True)

        # 4) 找"文档导入 / 导入向导"入口
        for label in ["文档导入", "导入向导", "导入", "steward"]:
            try:
                page.get_by_text(label, exact=False).first.click(timeout=4000)
                page.wait_for_timeout(1000)
                print(f"clicked import entry: {label}")
                break
            except Exception as e:
                print(f"import entry '{label}' skip:", str(e)[:80])
        page.wait_for_timeout(800)
        page.screenshot(path=f"{OUT}/wiki_recon_03_import_wizard.png", full_page=True)

        # 5) dump 导入向导区域的 textarea/button 选择器
        try:
            tas = page.locator("textarea").count()
            print(f"textarea count: {tas}")
            btns = page.locator("button").all_inner_texts()
            print("=== buttons on import view ===")
            for b in btns:
                b = b.strip()
                if b:
                    print(repr(b))
        except Exception as e:
            print("dump issue:", e)

        browser.close()
        print("RECON DONE")


if __name__ == "__main__":
    main()
