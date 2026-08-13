"""域⑧：确定性停止屏障 + 对前序 sent 回复的购买反应分析。

停止意图是当前入站的确定性安全信号，不依赖前序 Review 或 LLM。普通 Reaction 则是
当前入站对**前一条已 sent AI 回复**的反馈，结果写回被 claim 的前序 Review。
结果落 agent_decision_reviews.outcome_status + reaction_analysis 子文档。
叫停终值 = user_replied_stop_requested。prompt_key = user.reaction.task。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain8.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑧反应分析"


def main() -> None:
    account_id, app_id = _lib.biztest_account()

    # ── 停止意图（红线）──
    # 明确停止必须不依赖前序 sent Review，并终止当前 run、持久化停止屏障、清空在途发送。
    wxid = "biztest_c8stop"
    _lib.ensure_managed_contact(account_id, wxid, "biztest 叫停客户")
    _lib.reset_contact_conversation(account_id, wxid)
    print(f"[{DOMAIN}] 明确停止（无需前序回复或 Reaction LLM）...")
    stop_run = _lib.send_and_wait(
        app_id, wxid, "别再发了，我不想聊了，到此为止吧", "m8stop", max_wait=600
    )
    contact_stop = _lib.mongo_json(
        f'db.contacts.findOne({{wxid:"{wxid}",account_id:"{account_id}"}},'
        '{cooldown_until:1,operation_policy:1,_id:0})'
    )
    stopped = (isinstance(stop_run, dict)
               and stop_run.get("status") == "user_reaction_stop_requested"
               and isinstance(contact_stop, dict)
               and contact_stop.get("operation_policy", {}).get("explicitStopRequested") is True
               and _lib.active_outbox_count(wxid) == 0)
    _lib.expect(stopped, DOMAIN, "停止意图终止当前 run、持久化屏障且无在途发送",
                f"run={stop_run} contact={contact_stop}", "critical",
                "漏判停止意图=autonomy 红线(对明确拒绝仍推进)")

    # ── 购买信号 ──
    wxid2 = "biztest_c8buy"
    _lib.ensure_managed_contact(account_id, wxid2, "biztest 购买意向客户")
    _lib.reset_contact_conversation(account_id, wxid2)
    price_chunk = _lib.seed_citable_knowledge_chunk(
        "biztest_reaction_price",
        account_id,
        "biztest 课程价格与报名",
        "课程价格为 9800 元，客户确认后可以立即报名付款。",
    )
    price_verified = _lib.verify_knowledge_chunk(price_chunk) if price_chunk else {}
    _lib.expect(price_verified.get("ok") is True, DOMAIN, "购买场景价格知识已人工审定",
                f"chunk={price_chunk} verify={price_verified}", "critical")
    if price_verified.get("ok") is not True:
        return
    print(f"[{DOMAIN}] 购买信号 第一段...")
    b1 = _lib.send_and_wait(app_id, wxid2, "你们课程多少钱？", "m8b1", max_wait=600)
    _lib.expect(b1 is not None, DOMAIN, "购买-第一段 AI 回复完成", f"b1={b1}", "high")
    b1_run_id = b1.get("run_id", "") if isinstance(b1, dict) else ""
    predecessor = _lib.wait_review_status(wxid2, b1_run_id, {"sent"}, max_wait=240)
    eligible = predecessor.get("status") == "sent"
    _lib.expect(eligible, DOMAIN, "购买反应存在可 claim 的前序 sent Review",
                f"review={predecessor}", "BLOCKED",
                "前序回复未实际送达，不能评价后续 Reaction 分类")
    if not eligible:
        return
    print(f"[{DOMAIN}] 购买信号 第二段...")
    b2 = _lib.send_and_wait(app_id, wxid2, "可以现在就报名付款吗？我要买", "m8b2", max_wait=600)
    _lib.expect(b2 is not None, DOMAIN, "购买-第二段 webhook 完成", f"b2={b2}", "high")
    reacted = _lib.decision_review_for_run(wxid2, b1_run_id)
    buy = reacted.get("outcome_status") == "user_replied_buying_signal"
    _lib.expect(buy, DOMAIN, "付款意愿被判 buying_signal 类 outcome",
                f"predecessor_review={reacted}", "high",
                "明确付款意愿未被识别为购买信号→reaction 判定精度问题")

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
