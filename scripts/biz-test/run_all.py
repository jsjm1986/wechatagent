"""全量编排：cleanup→step0→批A各域→批B(切 active profile,最后跑)→收尾 cleanup。

单域失败不中断其它域（先全量出清单再修）。findings 累积进
docs/superpowers/specs/2026-06-26-full-business-logic-test-findings.md。

跑法：export DEPLOY_PASS=... ADMIN_USER=... ADMIN_PASS=...; python scripts/biz-test/run_all.py
"""
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).parent

# 批 A：销售域基线（DEFAULT profile，不切全局 active）
BATCH_A = [
    "batch_a_domain1",      # ①文章进库
    "batch_a_domain2",      # ②改库召回含恢复
    "batch_a_domain3",      # ③报价单素材+二次门+Review五闸
    "batch_a_domain4",      # ④卡片引荐 assist 开关双路径
    "batch_a_domain5",      # ⑤三段式提示词档位
    "batch_a_domain6",      # ⑥请示通道四阶段+误报反向
    "batch_a_domain8",      # ⑧用户反应分析
    "batch_a_domain9",      # ⑨长期记忆固化
    "batch_a_domain1011",   # ⑩管理agent编排+⑪提示词编辑红线
    "batch_a_domain13",     # ⑬知识库自治LLM群
]


def run(mod: str) -> int:
    print(f"\n{'='*72}\n>>> {mod}\n{'='*72}", flush=True)
    t0 = time.time()
    r = subprocess.run([sys.executable, str(HERE / f"{mod}.py")])
    print(f"<<< {mod} 退出码 {r.returncode}，耗时 {time.time()-t0:.0f}s", flush=True)
    return r.returncode


def main() -> None:
    run("cleanup")
    if run("step0_preflight") != 0:
        print("step0 失败（端点/凭据/account 问题），中止——不假绿。")
        return
    for m in BATCH_A:
        run(m)                 # 单域失败不挡其它域
    run("batch_b_industry")    # 批B：切换全局 active profile，最后跑，finally 恢复
    run("cleanup")             # 收尾清测试数据
    print("\n全量完成。问题清单见 "
          "docs/superpowers/specs/2026-06-26-full-business-logic-test-findings.md")
    print("逐条复核证据、按 severity 排序、标注红线预期 vs 真 bug 后再下结论。")


if __name__ == "__main__":
    main()
