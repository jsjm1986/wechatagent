"""域⑤：三段式渐进式提示词（Lean 停档 / Full 升档 / 恒注入铁律）。

观测点在 mongo agent_events：kind=ptier_run_tier(details.tier_used=lean/escalated/full)、
ptier_escalated、ptier_forced_full、ptier_clarify。details 字段名是 details(非 payload)。
寒暄期望停 lean；复杂多诉求咨询期望升档。升档由 LLM 自评驱动,单次不稳,建议多跑。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain5.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑤三段式"
WXID = "biztest_c5"


def _ptier_events() -> list[dict]:
    rows = _lib.mongo_json(
        f'db.agent_events.find({{contact_wxid:"{WXID}",kind:/ptier/}},'
        '{kind:1,details:1,_id:0}).sort({_id:-1}).limit(20).toArray()'
    )
    return rows if isinstance(rows, list) else []


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 三段式测试客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo(f'db.agent_events.deleteMany({{contact_wxid:"{WXID}"}})')

    # 寒暄 → 期望停 lean
    print(f"[{DOMAIN}] 寒暄（期望停 lean，真模型轮询）...")
    run_a = _lib.send_and_wait(app_id, WXID, "在吗？", "m5a", max_wait=600)
    _lib.expect(run_a is not None, DOMAIN, "寒暄轮 webhook 完成", f"run_a={run_a}", "high")

    # 复杂多诉求咨询 → 期望升档
    print(f"[{DOMAIN}] 复杂咨询（期望升档）...")
    run_b = _lib.send_and_wait(
        app_id, WXID,
        "我想详细了解你们课程的具体内容、师资水平、收费标准和退费政策，能一次说清楚吗？",
        "m5b", max_wait=600,
    )
    _lib.expect(run_b is not None, DOMAIN, "复杂咨询轮 webhook 完成", f"run_b={run_b}", "high")
    _lib.assert_llm_success(600, "user.reply.task", DOMAIN)

    evts = _ptier_events()
    kinds = [e.get("kind") for e in evts]
    _lib.expect("ptier_run_tier" in str(kinds), DOMAIN,
                "ptier_run_tier 事件落 mongo(三段式生效)",
                f"kinds={kinds}", "high", "无 ptier_run_tier→三段式未生效或 PROGRESSIVE_TIER_ENABLED=false")

    # 至少一轮升档（复杂咨询应触发 escalated/forced_full，或 run_tier 的 tier_used 非 lean）
    tiers = [str(e.get("details", {}).get("tier_used", "")) for e in evts
             if e.get("kind") == "ptier_run_tier"]
    escalated = (
        any(k in ("ptier_escalated", "ptier_forced_full") for k in kinds)
        or any(t in ("escalated", "full") for t in tiers)
    )
    _lib.expect(escalated, DOMAIN, "复杂咨询触发升档(escalated/full 或 ptier_escalated 事件)",
                f"kinds={kinds} tiers={tiers}", "medium",
                "升档由 LLM 自评驱动,单次不稳,建议多跑几轮看分布;持续不升才是真问题")

    print(f"[{DOMAIN}] 完成（升档 LLM 驱动，建议跑 3 次看稳定性）")


if __name__ == "__main__":
    main()
