"""Campaign frozen-spec dispatch acceptance test.

Uses the preflight-selected account and test-only data.  Evidence is bound to the exact campaign,
campaign_send, deterministic task, run, and Outbox rows; contact-level "latest" history is never
accepted as proof.
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "campaign派发闭环"
WXID = "biztest_campaign"
STAGE = "biztest_campaign_only"


def _cleanup() -> None:
    _lib.mongo(
        'const ids=db.campaigns.find({title:/^biztest_/},{_id:1}).toArray().map(x=>x._id);'
        'db.campaign_sends.deleteMany({$or:[{campaignId:{$in:ids}},{contactWxid:/^biztest_/}]});'
        'db.campaigns.deleteMany({_id:{$in:ids}});'
        f'db.agent_tasks.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.agent_send_outbox.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.agent_run_logs.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.agent_decision_reviews.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.agent_events.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.conversation_messages.deleteMany({{contact_wxid:{json.dumps(WXID)}}});'
        f'db.contacts.deleteMany({{wxid:{json.dumps(WXID)}}});'
    )


def _create(account_id: str, title: str, stage: str) -> dict:
    response = _lib.api(
        "POST", "/api/campaigns",
        {"accountId": account_id, "title": title,
         "intentText": "向测试客户介绍适合的方案并询问是否方便继续沟通。",
         "segmentFilter": {"customerStage": stage}},
        admin=True,
    )
    _lib.expect(
        isinstance(response.get("id"), str)
        and isinstance(response.get("specHash"), str)
        and response.get("specVersion") == 1,
        DOMAIN, "create 返回完整冻结规格身份", f"response={response}", "critical",
    )
    return response


def _campaign_send(campaign_id: str) -> dict:
    rows = _lib.mongo_json(
        f'db.campaign_sends.find({{campaignId:ObjectId({json.dumps(campaign_id)})}},'
        '{campaignId:1,contactWxid:1,taskId:1,specHash:1,status:1,_id:0}).toArray()'
    )
    return rows[0] if isinstance(rows, list) and len(rows) == 1 else {}


def main() -> None:
    account_id, _ = _lib.biztest_account()
    _cleanup()
    try:
        empty = _create(account_id, "biztest_empty", "biztest_no_such_stage")
        empty_id = empty["id"]

        missing = _lib.api(
            "POST", f"/api/campaigns/{empty_id}/dispatch", {}, admin=True,
        )
        _lib.expect(
            _lib.is_api_error(missing) is not None, DOMAIN,
            "缺少 specHash/specVersion 的派发被拒", f"response={missing}", "critical",
            "派发必须绑定管理员看到的冻结规格",
        )

        tampered = _lib.campaign_dispatch_body(empty)
        tampered["specHash"] = "0" * 64
        mismatch = _lib.api(
            "POST", f"/api/campaigns/{empty_id}/dispatch", tampered, admin=True,
        )
        _lib.expect(
            _lib.is_api_error(mismatch) is not None, DOMAIN,
            "篡改冻结规格哈希的派发被拒", f"response={mismatch}", "critical",
        )
        writes = _lib.mongo_json(
            f'db.campaign_sends.countDocuments({{campaignId:ObjectId({json.dumps(empty_id)})}})'
        )
        _lib.expect(writes == 0, DOMAIN, "规格不匹配产生零派发副作用", f"count={writes}",
                    "critical")

        no_hits = _lib.api(
            "POST", f"/api/campaigns/{empty_id}/dispatch",
            _lib.campaign_dispatch_body(empty), admin=True,
        )
        _lib.expect(
            _lib.is_api_error(no_hits) is not None, DOMAIN, "冻结规格命中 0 人时拒绝派发",
            f"response={no_hits}", "high",
        )

        _lib.ensure_managed_contact(account_id, WXID, "biztest campaign customer")
        _lib.reset_contact_conversation(account_id, WXID)
        _lib.mongo(
            f'db.contacts.updateOne({{wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},'
            f'{{$set:{{"domain_attributes.customer_stage":{json.dumps(STAGE)}}}}})'
        )
        created = _create(account_id, "biztest_hit", STAGE)
        campaign_id = created["id"]
        preview = _lib.api(
            "POST", f"/api/campaigns/{campaign_id}/preview", {}, admin=True,
        )
        _lib.expect(preview.get("targetCount") == 1, DOMAIN, "preview 仅命中隔离测试客户",
                    f"preview={preview}", "high")
        _lib.expect(
            _lib.campaign_dispatch_body(preview) == _lib.campaign_dispatch_body(created),
            DOMAIN, "create 与 preview 指向同一冻结规格", f"create={created} preview={preview}",
            "critical",
        )

        dispatched = _lib.api(
            "POST", f"/api/campaigns/{campaign_id}/dispatch",
            _lib.campaign_dispatch_body(preview), admin=True, timeout=90,
        )
        _lib.expect(dispatched.get("dispatchedCount") == 1, DOMAIN, "首次派发物化一个任务",
                    f"response={dispatched}", "critical")
        send = _campaign_send(campaign_id)
        task_id = send.get("taskId", {}).get("$oid") if isinstance(send.get("taskId"), dict) else send.get("taskId")
        _lib.expect(
            send.get("contactWxid") == WXID and send.get("status") == "enqueued"
            and send.get("specHash") == preview.get("specHash") and isinstance(task_id, str),
            DOMAIN, "campaign_send 精确绑定规格、客户与确定性任务", f"send={send}", "critical",
        )

        repeated = _lib.api(
            "POST", f"/api/campaigns/{campaign_id}/dispatch",
            _lib.campaign_dispatch_body(preview), admin=True,
        )
        _lib.expect(
            _lib.is_api_error(repeated) is not None, DOMAIN,
            "completed 活动的重复派发被状态门拒绝", f"response={repeated}", "critical",
        )
        sends_after = _lib.mongo_json(
            f'db.campaign_sends.countDocuments({{campaignId:ObjectId({json.dumps(campaign_id)})}})'
        )
        _lib.expect(sends_after == 1, DOMAIN, "重复派发未新增台账", f"count={sends_after}",
                    "critical")

        deadline = time.time() + 360
        run = {}
        while task_id and time.time() < deadline:
            rows = _lib.mongo_json(
                f'db.agent_run_logs.find({{source_event_id:{json.dumps(task_id)},'
                'source_kind:"follow_up_task"},{run_id:1,status:1,lifecycle:1,'
                'final_review_status:1,outbox_status:1,_id:0})'
                '.sort({_id:-1}).limit(1).toArray()'
            )
            if isinstance(rows, list) and rows:
                run = rows[0]
                lifecycle = str(run.get("lifecycle", ""))
                if (lifecycle == "completed" or lifecycle.startswith("failed_")
                        or lifecycle.startswith("aborted_")):
                    break
            time.sleep(5)
        lifecycle = str(run.get("lifecycle", ""))
        terminal_run = bool(run.get("run_id")) and (
            lifecycle == "completed" or lifecycle.startswith("failed_")
            or lifecycle.startswith("aborted_")
        )
        _lib.expect(terminal_run, DOMAIN, "确定性 campaign task 到达 Gateway 吸收终态",
                    f"taskId={task_id} run={run}", "high")
        if terminal_run:
            run_id = run["run_id"]
            terminal = run.get("final_review_status")
            outbox = _lib.outbox_for_run(WXID, run_id)
            if lifecycle == "completed" and terminal in (
                "approved", "revision_applied_approved"
            ):
                outbox_deadline = time.time() + 180
                while not outbox and time.time() < outbox_deadline:
                    time.sleep(5)
                    outbox = _lib.outbox_for_run(WXID, run_id)
            safe_terminal = (
                lifecycle.startswith("failed_") or lifecycle.startswith("aborted_")
                or terminal not in (None, "", "approved", "revision_applied_approved")
            )
            _lib.expect(
                bool(outbox) or safe_terminal,
                DOMAIN, "本轮派发有精确 Outbox 或明确安全终态",
                f"run={run} outbox={outbox}", "high",
            )
    finally:
        _cleanup()


if __name__ == "__main__":
    main()
