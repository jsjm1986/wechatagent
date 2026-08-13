"""域④：卡片引荐（assist 开/关双路径）。

assist 关(默认)→即便高价值信号也不发卡(全自治红线兜底);
assist 开(contacts.domain_attributes.assist_mode_override="force_on")→高价值→名片入 outbox，
并经 loopback MCP stub 到达 sent。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain4.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "④卡片引荐"
WXID = "biztest_c4"


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 高意向客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo('db.referral_cards.deleteMany({display_name:/^biztest_/})')

    # 走正式人审生命周期：create 强制 draft+disabled → admin approve → enable。
    created = _lib.api(
        "POST", "/api/referral-cards",
        {"accountId": account_id, "displayName": "biztest_顾问王老师",
         "targetWxid": "biztest_advisor", "sendTriggerHint": "客户明确要签约或到店参观时引荐",
         "targetStages": [], "tags": ["签约"]},
        admin=True,
    )
    card_id = created.get("id") if isinstance(created, dict) else None
    _lib.expect(isinstance(card_id, str) and len(card_id) == 24, DOMAIN,
                "正式 API 创建 draft 名片", f"created={created}", "critical")
    if not isinstance(card_id, str) or len(card_id) != 24:
        return
    reviewed = _lib.api("POST", f"/api/referral-cards/{card_id}/review",
                        {"status": "approved", "note": "biz-test 人工审核"}, admin=True)
    enabled = _lib.api("POST", f"/api/referral-cards/{card_id}/toggle",
                       {"enabled": True}, admin=True)
    _lib.expect(reviewed.get("ok") is True and enabled.get("ok") is True, DOMAIN,
                "管理员审核并启用名片", f"review={reviewed} toggle={enabled}", "critical")
    if reviewed.get("ok") is not True or enabled.get("ok") is not True:
        return

    # 显式 force_off，而非假设账号级 assist_mode_enabled 默认关闭。
    _lib.mongo(
        f'db.contacts.updateOne({{wxid:"{WXID}",account_id:"{account_id}"}},'
        '{$set:{"domain_attributes.assist_mode_override":"force_off"}})'
    )

    # ── 路径1：assist 关（默认）即便高价值也不发卡 ──
    print(f"[{DOMAIN}] 路径1 assist关 高价值信号（真模型轮询）...")
    t0 = time.time()
    run1 = _lib.send_and_wait(app_id, WXID, "我想签约报名，怎么操作？", "m4a", max_wait=600)
    print(f"  耗时 {time.time()-t0:.1f}s run1={run1}")
    _lib.expect(run1 is not None, DOMAIN, "路径1 webhook 一轮完成", f"run1={run1}", "critical")
    if run1 is None:
        return
    run1_id = run1.get("run_id", "") if isinstance(run1, dict) else ""
    ob1 = _lib.outbox_for_run(WXID, run1_id)
    no_card = not any(o.get("referral_card_id") for o in ob1)
    _lib.expect(no_card, DOMAIN, "assist关(默认)即便高价值信号也不发卡(全自治兜底)",
                f"outbox={ob1}", "critical",
                "assist 默认关却发名片=全自治红线破(默认不该有真人引荐)")

    # ── 路径2：assist 开 → 高价值→名片入 outbox ──
    print(f"[{DOMAIN}] 路径2 assist开...")
    _lib.mongo(
        f'db.contacts.updateOne({{wxid:"{WXID}",account_id:"{account_id}"}},'
        '{$set:{"domain_attributes.assist_mode_override":"force_on"}})'
    )
    _lib.reset_contact_conversation(account_id, WXID)
    run2 = _lib.send_and_wait(app_id, WXID, "我想尽快签约，能安排顾问对接吗？", "m4b", max_wait=600)
    _lib.expect(run2 is not None, DOMAIN, "路径2 webhook 一轮完成", f"run2={run2}", "high")
    run2_id = run2.get("run_id", "") if isinstance(run2, dict) else ""
    ob2 = _lib.outbox_for_run(WXID, run2_id)
    tier2 = _lib.ptier_events_for_run(WXID, run2_id)
    loaded_full = _lib.ptier_loaded_full_context(tier2)
    _lib.expect(loaded_full, DOMAIN, "明确顾问请求在终决策前加载 Full 候选上下文",
                f"run_id={run2_id} events={tier2}", "high",
                "既未强升也未升档到 Full，会导致模型看不到带 cardId 的已审候选")
    has_card = any(o.get("referral_card_id") == card_id for o in ob2)
    _lib.expect(has_card, DOMAIN, "assist开+高价值→测试名片进入精确 run 的 outbox",
                f"run_id={run2_id} card_id={card_id} outbox={ob2}", "high",
                "Full 已加载已审候选但本轮未形成对应名片 Outbox")
    card_delivery = (
        _lib.wait_card_outbox_terminal(WXID, run2_id, card_id)
        if has_card else {}
    )
    _lib.expect(
        card_delivery.get("status") == "sent",
        DOMAIN,
        "同决策文本送达后测试名片仍保有授权并经 stub 送达",
        f"run_id={run2_id} card_id={card_id} card_delivery={card_delivery}",
        "critical",
        "名片被 stale_task_claim 取消表示文本结算过早清除了同决策授权绑定",
    )
    _lib.assert_llm_success_for_run(run2_id, "user.reply.fast.task", DOMAIN)

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
