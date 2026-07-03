"""域④：卡片引荐（assist 开/关双路径）。

assist 关(默认)→即便高价值信号也不发卡(全自治红线兜底);
assist 开(contacts.domain_attributes.assist_mode_override="force_on")→高价值→名片入 outbox(referral_card_id,不真发)。

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

    # 种 approved+enabled 名片。字段按 ReferralCard 真实 BSON。
    # workspace_id/updated_at 是 struct 非 optional 字段，漏写后端反序列化失败(同 502 bug)。
    _lib.mongo(
        'db.referral_cards.insertOne({'
        'display_name:"biztest_顾问王老师",target_wxid:"biztest_advisor",'
        'review_status:"approved",enabled:true,target_stages:[],workspace_id:"default",'
        'send_trigger_hint:"客户明确要签约或到店参观时引荐",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '})'
    )
    # 确保 contact assist 关（默认）——清掉可能的 override
    _lib.mongo(
        f'db.contacts.updateOne({{wxid:"{WXID}",account_id:"{account_id}"}},'
        '{$unset:{"domain_attributes.assist_mode_override":""}})'
    )

    # ── 路径1：assist 关（默认）即便高价值也不发卡 ──
    print(f"[{DOMAIN}] 路径1 assist关 高价值信号（真模型轮询）...")
    t0 = time.time()
    run1 = _lib.send_and_wait(app_id, WXID, "我想签约报名，怎么操作？", "m4a", max_wait=600)
    print(f"  耗时 {time.time()-t0:.1f}s run1={run1}")
    _lib.expect(run1 is not None, DOMAIN, "路径1 webhook 一轮完成", f"run1={run1}", "critical")
    if run1 is None:
        return
    ob1 = _lib.latest_outbox(WXID, limit=8)
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
    run2 = _lib.send_and_wait(app_id, WXID, "我想尽快签约，能安排顾问对接吗？", "m4b", max_wait=600)
    _lib.expect(run2 is not None, DOMAIN, "路径2 webhook 一轮完成", f"run2={run2}", "high")
    ob2 = _lib.latest_outbox(WXID, limit=8)
    has_card = any(o.get("referral_card_id") for o in ob2)
    _lib.expect(has_card, DOMAIN, "assist开+高价值→名片入 outbox(referral_card_id,不真发)",
                f"outbox={ob2}", "high",
                "assist 开+明确签约意向却不引荐=辅助模式引荐链路未走通(注:由LLM判定,单次可能不出,可多跑)")
    _lib.assert_llm_success(600, "user.reply.task", DOMAIN)

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
