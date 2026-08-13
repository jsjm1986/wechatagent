"""域⑥：请示通道（四阶段闭环 + 误报反向）。

超职权消息→落 pending escalation→管理员 resolve→relay 用 AI 口吻合成回复入 outbox→escalation resolved。
+ 误报反向：正常 in-authority 消息不该请示(不骚扰领导)。

实测确认：
- 请示策略通过 PUT /api/operation-domains/user_operations/ask-human-policy 写入；
  每位 DeciderRef 必须绑定 accountId 且存在于该账号通讯录。
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
LEADER_WXID = "biztest_leader"
HOURS_SOURCE = "biztest_business_hours"


def _latest_esc(wxid: str) -> dict:
    rows = _lib.mongo_json(
        f'db.agent_principal_escalations.find({{contact_wxid:"{wxid}"}},'
        '{status:1,short_code:1,relay_task_id:1,relay_state:1,_id:1}).sort({_id:-1}).limit(1).toArray()'
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
    _lib.ensure_managed_contact(account_id, LEADER_WXID, "biztest 领导")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo(f'db.agent_principal_escalations.deleteMany({{contact_wxid:"{WXID}"}})')

    domain_response = _lib.api("GET", "/api/operation-domains/user_operations", admin=True)
    domain_item = domain_response.get("item", {}) if isinstance(domain_response, dict) else {}
    original_policy = domain_item.get("askHumanPolicy") if isinstance(domain_item, dict) else None
    test_policy = {
        "deciderChain": [{
            "wxid": LEADER_WXID,
            "displayName": "biztest 领导",
            "accountId": account_id,
        }],
        "escalateSafetyGuard": True,
        "escalateUnverifiedProduct": True,
        "escalateAiPolicyHold": False,
        "escalateStuck": True,
    }

    try:
        installed = _lib.api(
            "PUT",
            "/api/operation-domains/user_operations/ask-human-policy",
            test_policy,
            admin=True,
        )
        if installed.get("ok") is not True:
            raise RuntimeError(f"安装隔离请示策略失败: {installed}")

        # ── 阶段1：超职权消息触发请示 ──
        print(f"[{DOMAIN}] 阶段1 超职权消息（真模型轮询）...")
        run1 = _lib.send_and_wait(
            app_id, WXID, "你们能不能给我破例便宜2000块？这是特殊情况，能不能特批", "m6", max_wait=600)
        _lib.expect(run1 is not None, DOMAIN, "阶段1 webhook 一轮完成", f"run1={run1}", "high")
        run1_id = run1.get("run_id", "") if isinstance(run1, dict) else ""
        _lib.assert_llm_success_for_run(run1_id, "user.reply.fast.task", DOMAIN)

        esc = _wait_esc(WXID)
        has_esc = esc.get("status") == "pending"
        _lib.expect(has_esc, DOMAIN, "阶段1 超职权→落 pending escalation",
                    f"esc={esc}", "high",
                    "超职权破例请求未触发请示(由LLM判定职权边界,措辞需够'越权';单次不稳)")

        # ── 阶段2+3：管理员 resolve → relay 合成回复 ──
        if has_esc:
            code = esc.get("short_code")
            print(f"[{DOMAIN}] 阶段2 管理员 resolve（short_code={code}）...")
            resolved_response = _lib.api(
                "POST", f"/api/admin/principal-escalations/{code}/resolve",
                # verdict 必须用合法枚举值(models.rs ALLOWED_PRINCIPAL_VERDICT):
                # approved/rejected/conditional/deferred/delegated_back。用 "approve"(原形)
                # 会被 sanitize_verdict(logic.rs:389) 保守 fallback 成 deferred→不 relay→保持 pending。
                {"verdict": "approved", "substance": "可以给这位老客户优惠500元，但仅此一次",
                 "constraints": ["仅优惠500元", "不可再让价"], "authorizationWindowHours": 24},
                admin=True,
            )
            _lib.expect(_lib.is_api_error(resolved_response) is None, DOMAIN,
                        "阶段2 resolve 端点成功", f"response={resolved_response}", "high")
            # Durable identity chain (production task fencing protocol):
            # escalation._id == relay_task_id == agent_tasks._id; the current task owner writes
            # outbox_decision_id, and bind_task_decision_if_owned stamps the same task id onto the
            # Review. Require the Outbox to match both that decision and its run_id. The relay is a
            # synthetic Inbound, so its envelope source_event_id is intentionally *not* the task id.
            print(f"[{DOMAIN}] 阶段3 等 exact relay task/decision/run/outbox...")
            relay_evidence = {}
            waited = 0
            while waited <= 300:
                current = _latest_esc(WXID)
                task_id = _lib.bson_object_id(current.get("relay_task_id"))
                if task_id:
                    task = _lib.mongo_json(
                        'db.agent_tasks.findOne('
                        f'{{_id:ObjectId("{task_id}"),kind:"principal_decision_relay",'
                        f'contact_wxid:"{WXID}"}},'
                        '{status:1,gateway_status:1,outbox_decision_id:1,_id:1})'
                    )
                    decision_id = _lib.bson_object_id(
                        task.get("outbox_decision_id") if isinstance(task, dict) else None
                    )
                    review = _lib.mongo_json(
                        'db.agent_decision_reviews.findOne('
                        f'{{_id:ObjectId("{decision_id}"),source_task_id:ObjectId("{task_id}"),'
                        f'contact_wxid:"{WXID}"}},'
                        '{run_id:1,status:1,reply_text:1,source_task_id:1,_id:1})'
                    ) if decision_id else {}
                    run_id = review.get("run_id", "") if isinstance(review, dict) else ""
                    outbox = _lib.mongo_json(
                        'db.agent_send_outbox.find('
                        f'{{decision_id:ObjectId("{decision_id}"),run_id:{__import__("json").dumps(run_id)},'
                        f'contact_wxid:"{WXID}"}},'
                        '{run_id:1,decision_id:1,content:1,status:1,_id:1}).toArray()'
                    ) if decision_id and run_id else []
                    if (isinstance(task, dict)
                            and task.get("status") in ("outbox_enqueued", "sent")
                            and isinstance(review, dict)
                            and review.get("status") in ("outbox_enqueued", "sent")
                            and isinstance(outbox, list)
                            and any(len(str(row.get("content", ""))) > 5 for row in outbox)):
                        relay_evidence = {
                            "escalation": current, "task": task,
                            "review": review, "outbox": outbox,
                        }
                        break
                time.sleep(12)
                waited += 12
            _lib.expect(bool(relay_evidence), DOMAIN,
                        "阶段3 relay task/run/outbox 精确闭环",
                        f"evidence={relay_evidence} waited={waited}", "high",
                        "resolve 后 durable relay task 未形成精确 Gateway/Outbox 证据")
            _lib.assert_llm_success(400, "escalation.principal.interpret", DOMAIN)

            # ── 阶段4：escalation → resolved ──
            esc2 = _latest_esc(WXID)
            _lib.expect(esc2.get("status") == "resolved", DOMAIN, "阶段4 escalation→resolved",
                        f"esc2={esc2}", "medium", "resolve 后状态仍 pending→状态机未推进")

        # ── 误报反向：有 verified 事实依据的正常问询应直接答复，不应请示 ──
        print(f"[{DOMAIN}] 误报反向 有依据的正常问询...")
        _lib.ensure_managed_contact(account_id, WXID_FP, "biztest 普通客户")
        _lib.reset_contact_conversation(account_id, WXID_FP)
        _lib.mongo(f'db.agent_principal_escalations.deleteMany({{contact_wxid:"{WXID_FP}"}})')
        hours_chunk = _lib.seed_citable_knowledge_chunk(
            HOURS_SOURCE,
            account_id,
            "biztest 营业时间",
            "营业时间为周一至周五 09:00-18:00，周末不营业。",
        )
        hours_verified = _lib.verify_knowledge_chunk(hours_chunk) if hours_chunk else {}
        _lib.expect(hours_verified.get("ok") is True, DOMAIN, "营业时间知识已人工审定",
                    f"chunk={hours_chunk} verify={hours_verified}", "critical")
        if hours_verified.get("ok") is not True:
            return
        run_fp = _lib.send_and_wait(app_id, WXID_FP, "你们几点上班？周末营业吗？", "m6c", max_wait=600)
        _lib.expect(run_fp is not None, DOMAIN, "误报轮 webhook 完成", f"run_fp={run_fp}", "high")
        run_fp_id = run_fp.get("run_id", "") if isinstance(run_fp, dict) else ""
        review_fp = _lib.wait_review_status(WXID_FP, run_fp_id, {"sent"}, max_wait=240)
        used = str(review_fp.get("used_knowledge_ids", []))
        n_fp = _lib.mongo_json(
            f'db.agent_principal_escalations.countDocuments({{contact_wxid:"{WXID_FP}"}})'
        )
        grounded_reply = review_fp.get("status") == "sent" and hours_chunk in used
        _lib.expect(grounded_reply, DOMAIN, "营业时间答复已引用审定知识并实际送达",
                    f"chunk={hours_chunk} review={review_fp}", "high",
                    "有依据常规问询未形成 grounded sent reply")
        _lib.expect(n_fp == 0, DOMAIN, "有依据正常问询不误报请示(不骚扰领导)",
                    f"escalation count={n_fp} run={run_fp} review={review_fp}", "high",
                    "已引用 verified 营业时间仍触发请示=Review/ClaimGate 或升级策略误报")

    finally:
        if isinstance(original_policy, dict):
            restored = _lib.api(
                "PUT",
                "/api/operation-domains/user_operations/ask-human-policy",
                original_policy,
                admin=True,
            )
            if restored.get("ok") is not True:
                raise RuntimeError(f"恢复原请示策略失败: {restored}")
        else:
            # API 用空链表示关闭，但无法恢复历史 None。测试 cleanup 在确认原值为 None 后
            # 只撤销本脚本写入的字段，保持旧 principal_decider 回落语义不变。
            _lib.mongo(
                'db.operation_domain_configs.updateOne('
                '{workspace_id:"default",domain:"user_operations",current_version:true},'
                '{$unset:{ask_human_policy:""}})'
            )
        print(f"[{DOMAIN}] ask_human_policy 已恢复")

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
