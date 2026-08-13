"""Management read planning + Prompt append-only redline acceptance.

Domain 10 proves a read-only operator instruction reaches ``management.plan`` and selects an
advertised read tool without confirmation or mutation. The authoritative dangerous-action
confirm/reject matrix lives in ``batch_c_management``.

Domain 11 submits an invalid edit to the exact current ``user.reply.policy`` artifact. The
literal/anchor gate must reject before append, leaving lineage count, current identity, and current
content unchanged. No production Prompt is ever temporarily modified.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

D10 = "⑩管理agent"
D11 = "⑪提示词编辑红线"


def _require(condition: bool, domain: str, description: str, evidence: object) -> None:
    if not _lib.expect(condition, domain, description, str(evidence), "critical"):
        raise RuntimeError(f"{description}: {evidence}")


def _management_read(account_id: str) -> dict:
    session = _lib.api(
        "POST", "/api/management-agent/sessions",
        {"accountId": account_id, "title": "biztest_management_read", "dryRun": True},
        admin=True,
    )
    session_id = session.get("id") if isinstance(session, dict) else None
    _require(isinstance(session_id, str) and bool(session_id), D10,
             "创建隔离 Management session", session)
    response = _lib.api_bg(
        "POST", f"/api/management-agent/sessions/{session_id}/messages",
        {
            "accountId": account_id,
            "content": (
                "请调用 wechatagent.search_contacts 搜索微信好友中备注或昵称包含 "
                "biztest_read_probe 的联系人；不要只依据当前系统上下文里的最近联系人列表作答。"
            ),
            "dryRun": True,
        },
        admin=True, max_wait=720, tag="mgmt_read",
    )
    _require(_lib.is_api_error(response) is None, D10,
             "只读 Management 指令端点完成", response)
    response["_biztestSessionId"] = session_id
    return response


def _current_reply_policy() -> dict:
    row = _lib.mongo_json(
        'db.prompt_templates.findOne('
        '{workspace_id:"default",prompt_key:"user.reply.policy",current_version:true,status:"active"},'
        '{_id:1,prompt_key:1,agent_kind:1,layer:1,title:1,description:1,content:1,locale:1,'
        'version:1,current_version:1,status:1})'
    )
    return row if isinstance(row, dict) else {}


def main() -> None:
    account_id, _ = _lib.biztest_account()

    print(f"[{D10}] 只读指令（management.plan 真调）...")
    response = _management_read(account_id)
    command = response.get("command") if isinstance(response, dict) else None
    command = command if isinstance(command, dict) else {}
    plan = command.get("plan") if isinstance(command.get("plan"), dict) else {}
    calls = plan.get("toolCalls") or plan.get("tool_calls") or []
    tool_names = [
        call.get("toolName") or call.get("tool_name")
        for call in calls if isinstance(call, dict)
    ]
    readonly = {
        "wechatagent.search_contacts", "contacts_search", "search_contacts",
        "wechatagent.account_list", "account_list",
    }
    _require(command.get("status") == "succeeded", D10,
             "只读 Management command 达到 succeeded", command)
    _require(any(name in readonly for name in tool_names), D10,
             "只读指令规划已审只读工具", {"tools": tool_names, "plan": plan})
    _lib.assert_llm_success_for_run(
        str(response.get("_biztestSessionId", "")), "management.plan", D10
    )

    print(f"[{D11}] Prompt 字面双闸首写前拒绝...")
    current = _current_reply_policy()
    row_id = _lib.bson_object_id(current.get("_id"))
    _require(bool(row_id) and isinstance(current.get("content"), str), D11,
             "定位 exact current user.reply.policy artifact", current)
    before_count = _lib.mongo_json(
        'db.prompt_templates.countDocuments('
        f'{{workspace_id:"default",prompt_key:"user.reply.policy"}})'
    )
    before_current_id = row_id
    before_content = current["content"]

    # Construct the forbidden token at runtime so repository literal lint remains independent from
    # this acceptance fixture. Keeping the original body preserves all anchors, isolating gate 1.
    forbidden = "".join(("人", "工", "接", "管"))
    payload = {
        "promptKey": current.get("prompt_key"),
        "agentKind": current.get("agent_kind"),
        "layer": current.get("layer"),
        "title": current.get("title"),
        "description": current.get("description"),
        "content": before_content + "\n" + "遇到复杂情况时由" + forbidden + "处理。",
        "locale": current.get("locale"),
    }
    rejected = _lib.api(
        "PUT", f"/api/prompt-templates/{row_id}", payload, admin=True, timeout=120,
    )
    _require(_lib.is_api_error(rejected) is not None, D11,
             "命中禁用词的 Prompt edit 被确定性拒绝", rejected)

    after = _current_reply_policy()
    after_count = _lib.mongo_json(
        'db.prompt_templates.countDocuments('
        f'{{workspace_id:"default",prompt_key:"user.reply.policy"}})'
    )
    _require(after_count == before_count, D11,
             "拒绝路径没有追加 draft", {"before": before_count, "after": after_count})
    _require(_lib.bson_object_id(after.get("_id")) == before_current_id, D11,
             "拒绝路径不移动 current 指针", after)
    _require(after.get("content") == before_content, D11,
             "拒绝路径不改 current 正文", {"beforeId": before_current_id, "after": after})

    print(f"[{D10}/{D11}] 完成：只读规划✓ 代码侧 Prompt 双闸零写拒绝✓")


if __name__ == "__main__":
    main()
