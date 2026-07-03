"""域⑧：用户反应分析（两段对话，多种 outcome）。

reaction 无独立端点，是对**前一条 AI approved 回复**做 claim 分析，故必须两段：
先发让 AI 回复的消息 → 再发反应消息(停止/购买)才触发。
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


def _reaction_blob(wxid: str) -> str:
    """最近若干条 decision_review 的 outcome_status + reaction_analysis 拼成串，供模糊判定。

    取最近 6 条(非 2 条):reaction 分析滞后一轮触发(reaction.rs 分析的是"对上一条已发
    回复的反应",当前轮刚触发的 review 其 reaction_analysis 尚空,要等下一轮才填)。若只看
    最新 2 条,刚发的当前轮(reaction 空)会把已判对的历史轮挤出窗口→假阴。扩大窗口覆盖
    已落 reaction 的历史轮(实证:buyingSignal/stopRequested 真判对的 review 在更早位置)。
    """
    rows = _lib.mongo_json(
        f'db.agent_decision_reviews.find({{contact_wxid:"{wxid}"}},'
        '{outcome_status:1,reaction_analysis:1,_id:0}).sort({_id:-1}).limit(6).toArray()'
    )
    return str(rows if isinstance(rows, list) else [])


def main() -> None:
    account_id, app_id = _lib.biztest_account()

    # ── 停止意图（红线）──
    # reaction 分析的是"客户对 AI **上一条已发出(status:sent)回复**的反应"(reaction.rs:87-96
    # claim_filter status:sent),且滞后一轮触发。故须三段:①让 AI 真回复(避开 fast_chat
    # no_reply——"介绍课程"实测会 no_reply 致无 sent 回复可分析)→②客户停止意图→③再发一条
    # 触发对②的 reaction 分析。第一段用明确问句逼出实质回复。
    wxid = "biztest_c8stop"
    _lib.ensure_managed_contact(account_id, wxid, "biztest 叫停客户")
    _lib.reset_contact_conversation(account_id, wxid)
    print(f"[{DOMAIN}] 停止意图 第一段（让 AI 回复，真模型轮询）...")
    r1 = _lib.send_and_wait(app_id, wxid, "你们课程怎么收费？我想了解一下", "m8s1", max_wait=600)
    _lib.expect(r1 is not None, DOMAIN, "停止-第一段 AI 回复完成(reaction 需前一轮 approved 回复)",
                f"r1={r1}", "high", "第一段没跑完→第二段 reaction 无分析对象")
    print(f"[{DOMAIN}] 停止意图 第二段（表达停止）...")
    r2 = _lib.send_and_wait(app_id, wxid, "别再发了，我不想聊了，到此为止吧", "m8s2", max_wait=600)
    _lib.expect(r2 is not None, DOMAIN, "停止-第二段 webhook 完成", f"r2={r2}", "high")
    print(f"[{DOMAIN}] 停止意图 第三段（触发对停止消息的 reaction 分析，滞后一轮）...")
    r3 = _lib.send_and_wait(app_id, wxid, "嗯", "m8s3", max_wait=600)
    _lib.expect(r3 is not None, DOMAIN, "停止-第三段 webhook 完成(触发对②的反应分析)", f"r3={r3}", "high")
    _lib.assert_llm_success(600, "user.reaction.task", DOMAIN)

    blob = _reaction_blob(wxid)
    stop = "stop" in blob.lower() or "user_replied_stop" in blob
    _lib.expect(stop, DOMAIN, "停止意图被判 stop_requested(红线:漏判→继续骚扰已拒绝客户)",
                f"reaction={blob[:400]}", "critical",
                "漏判停止意图=autonomy 红线(对明确拒绝仍推进)")

    # ── 购买信号 ──
    wxid2 = "biztest_c8buy"
    _lib.ensure_managed_contact(account_id, wxid2, "biztest 购买意向客户")
    _lib.reset_contact_conversation(account_id, wxid2)
    print(f"[{DOMAIN}] 购买信号 第一段...")
    b1 = _lib.send_and_wait(app_id, wxid2, "你们课程多少钱？", "m8b1", max_wait=600)
    _lib.expect(b1 is not None, DOMAIN, "购买-第一段 AI 回复完成", f"b1={b1}", "high")
    print(f"[{DOMAIN}] 购买信号 第二段...")
    b2 = _lib.send_and_wait(app_id, wxid2, "可以现在就报名付款吗？我要买", "m8b2", max_wait=600)
    _lib.expect(b2 is not None, DOMAIN, "购买-第二段 webhook 完成", f"b2={b2}", "high")
    print(f"[{DOMAIN}] 购买信号 第三段（触发对购买消息的 reaction 分析，滞后一轮）...")
    b3 = _lib.send_and_wait(app_id, wxid2, "嗯", "m8b3", max_wait=600)
    _lib.expect(b3 is not None, DOMAIN, "购买-第三段 webhook 完成(触发对②的反应分析)", f"b3={b3}", "high")

    blob2 = _reaction_blob(wxid2)
    buy = any(k in blob2.lower() for k in ("buy", "purchas", "signal", "购买", "成交", "付款"))
    _lib.expect(buy, DOMAIN, "付款意愿被判 buying_signal 类 outcome",
                f"reaction={blob2[:400]}", "high",
                "明确付款意愿未被识别为购买信号→reaction 判定精度问题")

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
