"""域⑥：请示通道（四阶段闭环 + 误报反向）。

超职权消息→落 pending escalation→管理员 resolve→relay 用 AI 口吻合成回复入 outbox→escalation resolved。
+ 误报反向：正常 in-authority 消息不该请示(不骚扰领导)。

实测确认：
- decider_chain 存 operation_domain_configs {workspace_id:"default",domain:"user_operations"}
  的 ask_human_policy.deciderChain(camelCase 内嵌,DeciderRef={wxid,displayName})。
- escalation 集合 agent_principal_escalations:status(pending/resolved)/short_code/contact_wxid。
- resolve 端点 POST /api/admin/principal-escalations/:short_code/resolve,
  body camelCase {verdict,substance,constraints[],authorizationWindowHours}。
- relay prompt_key=escalation.principal.interpret。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain6.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑥请示通道"
WXID = "biztest_c6"
WXID_FP = "biztest_c6b"


def _latest_esc(wxid: str) -> dict:
    rows = _lib.mongo_json(
        f'db.agent_principal_escalations.find({{contact_wxid:"{wxid}"}},'
        '{status:1,short_code:1,_id:0}).sort({_id:-1}).limit(1).toArray()'
    )
    return rows[0] if isinstance(rows, list) and rows else {}


def _wait_esc(wxid: str, max_wait: int = 120, poll: int = 10) -> dict:
    """轮询等 escalation 落库（webhook 后台 runner 跑完 decision 才落）。"""
    waited = 0
    while waited <= max_wait:
        e = _latest_esc(wxid)
        if e.get("short_code"):
            return e
        time.sleep(poll)
        waited += poll
    return {}


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 超职权咨询客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo(f'db.agent_principal_escalations.deleteMany({{contact_wxid:"{WXID}"}})')

    # 配 decider_chain（workspace=default 的 user_operations 域），跑完恢复（finally）。
    cfg_q = '{workspace_id:"default",domain:"user_operations"}'
    orig = _lib.mongo_json(
        f'db.operation_domain_configs.findOne({cfg_q},{{ask_human_policy:1,_id:0}})'
    )
    had_policy = isinstance(orig, dict) and orig.get("ask_human_policy") is not None

    try:
        _lib.mongo(
            f'db.operation_domain_configs.updateOne({cfg_q},'
            '{$set:{"ask_human_policy.deciderChain":[{wxid:"biztest_leader",displayName:"biztest 领导"}],'
            '"ask_human_policy.escalateStuck":true}})'
        )

        # ── 阶段1：超职权消息触发请示 ──
        print(f"[{DOMAIN}] 阶段1 超职权消息（真模型轮询）...")
        run1 = _lib.send_and_wait(
            app_id, WXID, "你们能不能给我破例便宜2000块？这是特殊情况，能不能特批", "m6", max_wait=600)
        _lib.expect(run1 is not None, DOMAIN, "阶段1 webhook 一轮完成", f"run1={run1}", "high")
        _lib.assert_llm_success(600, "user.reply.task", DOMAIN)

        esc = _wait_esc(WXID)
        has_esc = esc.get("status") == "pending"
        _lib.expect(has_esc, DOMAIN, "阶段1 超职权→落 pending escalation",
                    f"esc={esc}", "high",
                    "超职权破例请求未触发请示(由LLM判定职权边界,措辞需够'越权';单次不稳)")

        # ── 阶段2+3：管理员 resolve → relay 合成回复 ──
        if has_esc:
            code = esc.get("short_code")
            print(f"[{DOMAIN}] 阶段2 管理员 resolve（short_code={code}）...")
            prev_ob = len(_lib.latest_outbox(WXID, limit=20))
            _lib.api(
                "POST", f"/api/admin/principal-escalations/{code}/resolve",
                # verdict 必须用合法枚举值(models.rs ALLOWED_PRINCIPAL_VERDICT):
                # approved/rejected/conditional/deferred/delegated_back。用 "approve"(原形)
                # 会被 sanitize_verdict(logic.rs:389) 保守 fallback 成 deferred→不 relay→保持 pending。
                {"verdict": "approved", "substance": "可以给这位老客户优惠500元，但仅此一次",
                 "constraints": ["仅优惠500元", "不可再让价"], "authorizationWindowHours": 24},
                admin=True,
            )
            # relay 走后台 task，轮询等 outbox 新增（AI 口吻合成回复）
            print(f"[{DOMAIN}] 阶段3 等 relay 合成回复入 outbox...")
            relayed = False
            waited = 0
            while waited <= 300:
                ob = _lib.latest_outbox(WXID, limit=20)
                if len(ob) > prev_ob and any(len(str(o.get("content", ""))) > 5 for o in ob):
                    relayed = True
                    break
                time.sleep(12)
                waited += 12
            _lib.expect(relayed, DOMAIN, "阶段3 relay 用 AI 口吻合成回复入 outbox",
                        f"outbox 新增={relayed} prev={prev_ob}", "high",
                        "resolve 后无 relay 合成回复→请示闭环断(领导裁决没回传客户)")
            _lib.assert_llm_success(400, "escalation.principal.interpret", DOMAIN)

            # ── 阶段4：escalation → resolved ──
            esc2 = _latest_esc(WXID)
            _lib.expect(esc2.get("status") == "resolved", DOMAIN, "阶段4 escalation→resolved",
                        f"esc2={esc2}", "medium", "resolve 后状态仍 pending→状态机未推进")

        # ── 误报反向：正常消息不该请示 ──
        print(f"[{DOMAIN}] 误报反向 正常问询...")
        _lib.ensure_managed_contact(account_id, WXID_FP, "biztest 普通客户")
        _lib.reset_contact_conversation(account_id, WXID_FP)
        _lib.mongo(f'db.agent_principal_escalations.deleteMany({{contact_wxid:"{WXID_FP}"}})')
        run_fp = _lib.send_and_wait(app_id, WXID_FP, "你们几点上班？周末营业吗？", "m6c", max_wait=600)
        _lib.expect(run_fp is not None, DOMAIN, "误报轮 webhook 完成", f"run_fp={run_fp}", "high")
        time.sleep(5)
        n_fp = _lib.mongo_json(
            f'db.agent_principal_escalations.countDocuments({{contact_wxid:"{WXID_FP}"}})'
        )
        no_fp = (n_fp == 0)
        _lib.expect(no_fp, DOMAIN, "正常问询不误报请示(不骚扰领导)",
                    f"escalation count={n_fp}", "high",
                    "营业时间这类常规问询触发请示=误报,请示判定精度问题")

    finally:
        # 恢复 decider_chain（不污染：无原 policy 则 unset 整个 ask_human_policy）
        if had_policy:
            print(f"[{DOMAIN}] 恢复原 ask_human_policy")
            import json as _json
            _lib.mongo(
                f'db.operation_domain_configs.updateOne({cfg_q},'
                f'{{$set:{{ask_human_policy:{_json.dumps(orig["ask_human_policy"])}}}}})'
            )
        else:
            _lib.mongo(
                f'db.operation_domain_configs.updateOne({cfg_q},'
                '{$unset:{ask_human_policy:""}})'
            )
        print(f"[{DOMAIN}] decider_chain 已恢复")

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
