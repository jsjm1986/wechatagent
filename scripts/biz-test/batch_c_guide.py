"""Guide v3 acceptance: read-only Preview, frozen Apply, and idempotent receipt replay."""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "guide引导层v3"
WXID = "biztest_guide"


def _contact(account_id: str) -> dict:
    row = _lib.mongo_json(
        "db.contacts.findOne("
        f"{{wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},"
        "{human_profile_note:1,follow_up_policy:1,operation_state:1,domain_attributes:1,"
        "updated_at:1,_id:1})"
    )
    return row if isinstance(row, dict) else {}


def _memory_count(account_id: str) -> int:
    value = _lib.mongo_json(
        "db.operating_memories.countDocuments("
        f"{{contact_wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}})"
    )
    return int(value) if isinstance(value, (int, float)) else -1


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise SystemExit(f"{description}: {evidence}")


def main() -> None:
    account_id, _ = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "引导测试客户")
    _lib.reset_contact_conversation(account_id, WXID)
    # Exact test-root cleanup only. Starting without memory exercises Preview's read-only projection.
    _lib.mongo(
        f'db.user_operation_guide_previews.deleteMany({{contact_wxid:{json.dumps(WXID)},'
        f'account_id:{json.dumps(account_id)}}});'
        f'db.operating_memories.deleteMany({{contact_wxid:{json.dumps(WXID)},'
        f'account_id:{json.dumps(account_id)}}})'
    )
    before = _contact(account_id)
    contact_id = _lib.bson_object_id(before.get("_id"))
    _require(bool(contact_id), "测试联系人存在", before)
    _require(_memory_count(account_id) == 0, "Preview 前无记忆行", _memory_count(account_id))

    response = _lib.api_bg(
        "POST", "/api/user-operations/guide/preview",
        {
            "accountId": account_id,
            "contactId": contact_id,
            "instruction": "仅针对当前客户：标记为重点跟进，并备注他明确关注价格。",
            "mode": "smart",
        },
        admin=True, max_wait=720, tag="guide_preview_v3",
    )
    error = _lib.is_api_error(response)
    if error:
        raise SystemExit(f"Guide Preview BLOCKED: {error}")
    _lib.assert_llm_success(720, "user.guide.preview", DOMAIN)
    item = response.get("item") if isinstance(response, dict) else None
    _require(isinstance(item, dict), "Preview 返回冻结候选", response)
    binding = _lib.guide_apply_binding(response)
    _require(binding["expectedAccountId"] == account_id, "Apply 绑定测试账号", binding)
    _require(binding["expectedContactId"] == contact_id, "Apply 绑定测试联系人", binding)

    stored = _lib.mongo_json(
        "db.user_operation_guide_previews.findOne("
        f"{{_id:ObjectId({json.dumps(binding['previewId'])})}},"
        "{status:1,candidate_hash:1,apply_protocol_version:1,_id:0})"
    )
    _require(
        isinstance(stored, dict) and stored.get("status") == "pending"
        and stored.get("candidate_hash") == binding["candidateHash"],
        "Preview 以 pending + candidateHash 精确落库", stored,
    )
    _require(_contact(account_id) == before, "Preview 不修改 contact", {"before": before, "after": _contact(account_id)})
    _require(_memory_count(account_id) == 0, "Preview 不创建 operating_memory", _memory_count(account_id))

    tampered = dict(binding)
    tampered["candidateHash"] = "0" * 64
    rejected = _lib.api("POST", "/api/user-operations/guide/apply", tampered, admin=True)
    _require(_lib.is_api_error(rejected) is not None, "篡改 candidateHash 被拒", rejected)
    _require(_contact(account_id) == before and _memory_count(account_id) == 0,
             "错误哈希产生零业务副作用", {"contact": _contact(account_id), "memory": _memory_count(account_id)})

    applied = _lib.api("POST", "/api/user-operations/guide/apply", binding, admin=True, timeout=180)
    error = _lib.is_api_error(applied)
    if error:
        raise SystemExit(f"Guide Apply failed: {error} response={applied}")
    receipt = applied.get("item") if isinstance(applied, dict) else None
    _require(
        isinstance(receipt, dict) and receipt.get("committed") is True
        and receipt.get("previewId") == binding["previewId"]
        and receipt.get("candidateHash") == binding["candidateHash"],
        "Apply 返回与冻结候选绑定的提交回执", receipt,
    )
    _require(bool(receipt.get("appliedFields")), "明确指令至少应用一个合法字段", receipt)
    _require(_memory_count(account_id) == 1, "确认后恰好创建冻结记忆基线", _memory_count(account_id))
    status = _lib.mongo_json(
        f'db.user_operation_guide_previews.findOne({{_id:ObjectId({json.dumps(binding["previewId"])})}}).status'
    )
    _require(status == "applied", "提交后 Preview 进入 applied", status)

    replay = _lib.api("POST", "/api/user-operations/guide/apply", binding, admin=True)
    replay_receipt = replay.get("item") if isinstance(replay, dict) else None
    _require(replay_receipt == receipt, "重复 Apply 幂等返回同一持久化回执", {"first": receipt, "replay": replay_receipt})
    events = _lib.mongo_json(
        "db.agent_events.countDocuments("
        f"{{kind:'user_operation_guide_applied','details.previewId':{json.dumps(binding['previewId'])}}})"
    )
    _require(events == 1, "幂等重放不重复写审计事件", events)
    print(f"[{DOMAIN}] 完成：Preview零业务写入✓ 冻结绑定✓ Apply提交✓ 幂等回执✓")


if __name__ == "__main__":
    main()
