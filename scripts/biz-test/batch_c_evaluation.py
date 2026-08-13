"""Evaluation acceptance against the production-derived shadow terminal contract."""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "评测体系(prod同源终态)"
WXID = "biztest_eval"
OLD_DEAD_RULES = ["knowledge_grounding 评分不足", "hallucination 评分过高"]
# simulation.rs maps an allowed + approved reply to would_send, while shadow_finalize preserves
# all other production-derived terminals. This is intentionally broader than "pass" statuses.
VALID_TERMINALS = {
    "would_send",
    "no_reply",
    "revision_required",
    "gateway_blocked",
    "review_blocked",  # legacy compatible simulation artifact
    "blocked_by_required_field",
    "blocked_by_budget",
    "blocked_unverified_product_claim",
    "blocked_by_safety_guard",
    "held_by_ai_policy",
    "ai_waiting_for_more_context",
}
PASS_TERMINALS = {"would_send", "no_reply"}


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise SystemExit(f"{description}: {evidence}")


def main() -> None:
    account_id, _ = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "评测客户")
    _lib.reset_contact_conversation(account_id, WXID)
    contact = _lib.mongo_json(
        "db.contacts.findOne("
        f"{{wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},{{_id:1}})"
    )
    contact_id = _lib.bson_object_id(contact.get("_id") if isinstance(contact, dict) else None)
    _require(bool(contact_id), "评测联系人存在", contact)

    started = time.time()
    result = _lib.api_bg(
        "POST", "/api/user-operations/evaluations/run",
        {"accountId": account_id, "contactId": contact_id},
        admin=True, max_wait=1200, tag="evaluation_prod_terminals",
    )
    error = _lib.is_api_error(result)
    if error:
        raise SystemExit(f"evaluation endpoint BLOCKED: {error} response={result}")
    summary = result.get("summary") if isinstance(result, dict) else None
    items = result.get("items") if isinstance(result, dict) else None
    _require(isinstance(summary, dict) and isinstance(items, list) and bool(items),
             "评测返回非空场景与汇总", result)
    elapsed = max(1, int(time.time() - started) + 5)
    _lib.assert_llm_success(elapsed, "user.reply.fast.task", DOMAIN)

    statuses: list[str] = []
    issues: list[str] = []
    for item in items:
        evaluation = item.get("evaluation") if isinstance(item, dict) else None
        _require(isinstance(evaluation, dict), "每个场景含 evaluation", item)
        status = evaluation.get("finalReviewStatus")
        statuses.append(status if isinstance(status, str) else "")
        item_issues = evaluation.get("issues") or []
        if isinstance(item_issues, list):
            issues.extend(str(issue) for issue in item_issues)
        passed = bool(item.get("passed"))
        _require(passed == (status in PASS_TERMINALS),
                 "passed 与生产同源允许终态一致",
                 {"scenario": item.get("scenario"), "status": status, "passed": passed})

    _require(all(status in VALID_TERMINALS for status in statuses),
             "所有 finalReviewStatus 均属于真实模拟终态闭集",
             {"statuses": statuses, "valid": sorted(VALID_TERMINALS)})
    dead = [issue for issue in issues if any(marker in issue for marker in OLD_DEAD_RULES)]
    _require(not dead, "评测不再使用 0-100 与 0-10 错配的旧死规则", dead)
    passed = int(summary.get("passed", 0))
    failed = int(summary.get("failed", 0))
    total = int(summary.get("total", 0))
    _require(total == len(items) and passed + failed == total,
             "评测汇总与场景明细一致", {"summary": summary, "items": len(items)})
    _require(passed > 0, "至少一个内置场景获得生产发送授权或安全静默", summary)
    print(f"[{DOMAIN}] 完成：{passed}/{total} passed，耗时 {time.time()-started:.1f}s，终态={statuses}")


if __name__ == "__main__":
    main()
