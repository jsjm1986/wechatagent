"""Digital Twin acceptance using only real post-decision projection output."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "digital-twin关系建议"
WXID = "biztest_peer"
CANONICAL = {"customer", "peer", "friend"}


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise SystemExit(f"{description}: {evidence}")


def _relationship(account_id: str) -> object:
    row = _lib.mongo_json(
        "db.contacts.findOne("
        f"{{wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},"
        "{domain_attributes:1,_id:0})"
    )
    if not isinstance(row, dict) or not isinstance(row.get("domain_attributes"), dict):
        return None
    return row["domain_attributes"].get("relationship_type")


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "同行交流")
    _lib.reset_contact_conversation(account_id, WXID)
    contact = _lib.mongo_json(
        "db.contacts.findOne("
        f"{{wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},{{_id:1}})"
    )
    contact_id = _lib.bson_object_id(contact.get("_id") if isinstance(contact, dict) else None)
    _require(bool(contact_id), "测试联系人存在", contact)
    _lib.mongo(
        f'db.relationship_type_suggestions.deleteMany({{contact_id:{json.dumps(contact_id)}}});'
        f'db.projection_observations.deleteMany({{entity_type:"relationship_type_suggestion",'
        f'run_id:/^biztest_/}})'
    )
    before = _relationship(account_id)

    messages = [
        "我也是做私域运营的，我们是同行。我不采购产品，只想以同行身份交流获客方法。",
        "再明确一下：我自己带客户运营团队，不是潜在客户，请把我理解为 peer 同行关系。",
    ]
    observed: list[tuple[str, dict]] = []
    terminals: list[dict] = []
    for index, message in enumerate(messages, 1):
        run = _lib.send_and_wait(app_id, WXID, message, f"twin_{index}", max_wait=600)
        _require(isinstance(run, dict) and bool(run.get("run_id")), f"第{index}轮产生精确 run", run)
        run_id = run["run_id"]
        terminal = _lib.wait_projection_terminal(WXID, run_id, max_wait=420)
        terminals.append(terminal)
        _require(terminal.get("post_decision_status") == "completed",
                 f"第{index}轮 post-decision Projection 完成", terminal)
        llm = _lib.projection_llm_logs(run_id)
        _require(any(row.get("status") == "success" for row in llm),
                 f"第{index}轮精确 user.projection.task 真调成功", llm)
        for suggestion in _lib.relationship_suggestions_for_run(contact_id, run_id):
            observed.append((run_id, suggestion))
        if observed:
            break

    _require(bool(observed), "强同行信号经真实 Projection 自主产生关系建议",
             {"terminals": terminals, "runs": [r.get("run_id") for r in terminals]})
    run_id, suggestion = observed[0]
    suggestion_id = _lib.bson_object_id(suggestion.get("_id"))
    value = suggestion.get("suggested_value")
    _require(suggestion.get("status") == "pending" and value in CANONICAL and bool(suggestion_id),
             "建议为 pending 且值属于 canonical 字典", suggestion)
    _require(_relationship(account_id) == before,
             "AI 建议在审核前不直接修改联系人关系", {"before": before, "after": _relationship(account_id)})

    approved = _lib.api(
        "POST", f"/api/admin/relationship-type-suggestions/{suggestion_id}/approve",
        {}, admin=True, timeout=90,
    )
    error = _lib.is_api_error(approved)
    if error:
        raise SystemExit(f"relationship approval failed: {error} response={approved}")
    item = approved.get("item") if isinstance(approved, dict) else None
    _require(isinstance(item, dict) and item.get("status") == "approved"
             and item.get("suggestedValue") == value,
             "审核端点批准精确建议", approved)
    _require(_relationship(account_id) == value,
             "批准后 canonical 关系值事务写回 contact", {"expected": value, "actual": _relationship(account_id)})
    ledger = _lib.mongo_json(
        "db.projection_observations.countDocuments("
        f"{{entity_type:'relationship_type_suggestion',entity_id:{json.dumps(suggestion_id)},"
        f"run_id:{json.dumps(run_id)}}})"
    )
    _require(ledger == 1, "批准建议仍可追溯到精确 Projection run", ledger)
    print(f"[{DOMAIN}] 完成：run={run_id} suggestion={suggestion_id} value={value}")


if __name__ == "__main__":
    main()
