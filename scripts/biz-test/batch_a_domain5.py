"""域⑤：三段式渐进式提示词（Lean 停档 / Full 升档 / 恒注入铁律）。

观测点在 mongo agent_events：kind=ptier_run_tier(details.tier_used=lean/escalated/full)、
ptier_escalated、ptier_forced_full、ptier_clarify。details 字段名是 details(非 payload)。
寒暄必须精确停 Lean；复杂多事实咨询因已命中审定知识，必须加载 Full 业务上下文。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain5.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑤三段式"
WXID = "biztest_c5"



def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 三段式测试客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo(f'db.agent_events.deleteMany({{contact_wxid:"{WXID}"}})')
    chunk_id = _lib.seed_citable_knowledge_chunk(
        "biztest_tier_complex_course", account_id, "biztest 课程完整说明",
        "课程包含基础编程与项目实践；师资为认证讲师；收费标准为 9800 元；开课前 7 天可申请全额退费。",
    )
    verified = _lib.verify_knowledge_chunk(chunk_id) if chunk_id else {}
    _lib.expect(verified.get("ok") is True, DOMAIN, "复杂咨询知识已人工审定",
                f"chunk={chunk_id} verify={verified}", "critical")
    if verified.get("ok") is not True:
        return

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
    run_b_id = run_b.get("run_id", "") if isinstance(run_b, dict) else ""
    _lib.assert_llm_success_for_run(run_b_id, "user.reply.fast.task", DOMAIN)

    run_a_id = run_a.get("run_id", "") if isinstance(run_a, dict) else ""
    events_a = _lib.ptier_events_for_run(WXID, run_a_id)
    events_b = _lib.ptier_events_for_run(WXID, run_b_id)
    if _lib.ptier_requested_clarification(events_b):
        # Clarify 是生产契约允许的安全分支。明确消除模型所报歧义后再验证 Full，
        # 避免把合法澄清随机性误判成三档机制故障。
        print(f"[{DOMAIN}] 首轮安全澄清；明确四项均需说明后验证 Full...")
        run_b = _lib.send_and_wait(
            app_id, WXID,
            "四项都同等重要，不需要再澄清或追问。请直接依据已审定的课程说明，"
            "一次完整说明课程内容、师资、9800元收费标准和开课前7天退费政策。",
            "m5b_full", max_wait=600,
        )
        _lib.expect(run_b is not None, DOMAIN, "消除歧义后的复杂咨询轮 webhook 完成",
                    f"run_b={run_b}", "high")
        run_b_id = run_b.get("run_id", "") if isinstance(run_b, dict) else ""
        _lib.assert_llm_success_for_run(run_b_id, "user.reply.fast.task", DOMAIN)
        events_b = _lib.ptier_events_for_run(WXID, run_b_id)

    tiers_a = [e.get("details", {}).get("tier_used") for e in events_a
               if e.get("kind") == "ptier_run_tier"]
    _lib.expect(tiers_a == ["lean"], DOMAIN, "寒暄轮精确停在 Lean",
                f"run_id={run_a_id} events={events_a}", "high",
                "寒暄不应加载昂贵业务上下文")
    loaded_full = _lib.ptier_loaded_full_context(events_b)
    _lib.expect(loaded_full, DOMAIN, "无歧义复杂多事实咨询精确加载 Full 业务上下文",
                f"run_id={run_b_id} events={events_b}", "high",
                "复杂咨询停 Lean 会在未读取审定知识时作答")

    print(f"[{DOMAIN}] 完成（寒暄 Lean、复杂咨询 Full 均按 run_id 取证）")


if __name__ == "__main__":
    main()
