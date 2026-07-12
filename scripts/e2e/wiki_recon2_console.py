"""侦察 wiki「控制台」tab 里的文档导入向导 DOM。
运行:PYTHONUTF8=1 python scripts/e2e/wiki_recon2_console.py"""
from playwright.sync_api import sync_playwright

BASE = "http://117.72.54.28:3003"
OUT = "E:/yw/agiatme/工作项目/wechatagent/scripts/e2e"


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1560, "height": 960})
        page.set_default_timeout(20000)

        page.goto(BASE)
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(600)
        # 登录
        page.locator("input[type=text], input[placeholder*=用户], input[placeholder*=账]").first.fill("admin")
        page.locator("input[type=password]").first.fill("admin")
        page.locator("button[type=submit], button:has-text('登录')").first.click()
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1200)

        # 进知识库 Wiki
        page.get_by_text("知识库 Wiki", exact=True).first.click()
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1500)

        # 点顶部"控制台"tab
        try:
            page.get_by_text("控制台", exact=False).first.click(timeout=6000)
            page.wait_for_timeout(1500)
            print("clicked 控制台 tab")
        except Exception as e:
            print("控制台 click issue:", str(e)[:100])
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1000)
        page.screenshot(path=f"{OUT}/wiki_recon2_00_console.png", full_page=True)

        # dump 控制台里的子导航/按钮/textarea
        print(f"=== textarea count: {page.locator('textarea').count()} ===")
        print("=== buttons ===")
        for b in page.locator("button").all_inner_texts():
            b = b.strip()
            if b and b not in ("AI 总控","账号管理","工作台","用户运营","微信群运营","朋友圈运营","统一收件箱","请示通道配置","活动","产品与成交","内容资产","专属顾问","知识库 Wiki","系统策略","AI 模型配置","任务日志","自治回路监控","演化中心","运营成效","发送成效","同步微信号","登出"):
                print(repr(b))

        # 点「导入向导」子入口
        try:
            page.get_by_text("导入向导", exact=False).first.click(timeout=6000)
            page.wait_for_timeout(1800)
            print("clicked 导入向导")
        except Exception as e:
            print("导入向导 skip:", str(e)[:100])
        page.wait_for_load_state("networkidle")
        page.wait_for_timeout(1000)
        page.screenshot(path=f"{OUT}/wiki_recon2_01_import.png", full_page=True)
        print(f"=== after import click, textarea count: {page.locator('textarea').count()} ===")
        # dump textarea placeholders
        for i in range(page.locator("textarea").count()):
            ph = page.locator("textarea").nth(i).get_attribute("placeholder")
            print(f"textarea[{i}] placeholder={ph!r}")
        # dump inputs
        print("=== inputs ===")
        for i in range(page.locator("input").count()):
            el = page.locator("input").nth(i)
            print(f"input[{i}] type={el.get_attribute('type')!r} placeholder={el.get_attribute('placeholder')!r}")
        # dump buttons on import view
        print("=== buttons on import view ===")
        for b in page.locator("button").all_inner_texts():
            b = b.strip()
            if b and b not in ("工作台\n今日待办与起草","知识库\n问答、浏览与治理","控制台\n录入、Schema 与系统","概览","文档目录","导入向导","外部源","行业 Schema","系统配置","高级"):
                print(repr(b))

        browser.close()
        print("RECON2 DONE")


if __name__ == "__main__":
    main()
