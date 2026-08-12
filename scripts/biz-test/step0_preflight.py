"""Step0 前置体检：核对 server HEAD / active provider / vision provider，准备管理员 cookie + 测试 account。

跑法：export DEPLOY_PASS=... ADMIN_USER=admin ADMIN_PASS=admin; python scripts/biz-test/step0_preflight.py

产出（供后续所有域脚本依赖）：
- server 上 /tmp/biztest_cookie（管理员 session wa_session）
- server 上 /tmp/biztest_account（account_id|app_id）
- 打印 active provider model + vision 状态（结论可复现）
"""
import base64
import json
import os
import sys
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).parent))
import _lib

# 可通过环境显式绑定；未指定时仅接受唯一一个完整、在线且 active 的账号。
# 禁止历史默认值静默指向另一套环境中的账号。
TEST_ACCOUNT_ID = os.environ.get("BIZTEST_ACCOUNTID", "").strip() or None


def select_test_account(rows: object, requested_id: Optional[str] = None) -> dict:
    """Select one fully usable account, failing closed on absence or ambiguity."""
    if not isinstance(rows, list):
        raise ValueError("wechat account inventory is unreadable")

    def usable(row: object) -> bool:
        return (
            isinstance(row, dict)
            and isinstance(row.get("account_id"), str)
            and bool(row["account_id"].strip())
            and isinstance(row.get("app_id"), str)
            and bool(row["app_id"].strip())
            and isinstance(row.get("webhook_secret"), str)
            and bool(row["webhook_secret"].strip())
            and row.get("online") is True
            and row.get("status") == "active"
        )

    candidates = [row for row in rows if usable(row)]
    if requested_id:
        matches = [row for row in candidates if row.get("account_id") == requested_id]
        if len(matches) != 1:
            raise ValueError(
                f"explicit BIZTEST_ACCOUNTID={requested_id} does not identify exactly one usable account"
            )
        return matches[0]
    if len(candidates) != 1:
        ids = sorted(str(row.get("account_id", "<missing>")) for row in candidates)
        raise ValueError(f"expected exactly one usable account, found {len(candidates)}: {ids}")
    return candidates[0]


def unsafe_principal_targets(rows: object) -> list[str]:
    """Return configured principal wxids that are outside the test namespace."""
    if not isinstance(rows, list):
        return ["<unreadable-principal-policy>"]
    targets: set[str] = set()
    for row in rows:
        if not isinstance(row, dict):
            continue
        policy = row.get("ask_human_policy") or row.get("askHumanPolicy") or {}
        if not isinstance(policy, dict):
            continue
        chain = policy.get("deciderChain") or policy.get("decider_chain") or []
        if isinstance(chain, list):
            for item in chain:
                if isinstance(item, dict) and isinstance(item.get("wxid"), str):
                    targets.add(item["wxid"].strip())
        legacy = policy.get("principal_decider") or policy.get("principalDecider")
        if isinstance(legacy, str):
            targets.add(legacy.strip())
    return sorted(target for target in targets if target and not target.startswith(_lib.BIZ_PREFIX))


def banner(t: str) -> None:
    print(f"\n{'='*70}\n{t}\n{'='*70}", flush=True)


def main() -> None:
    banner("[1/5] server HEAD + app health")
    print(f"release={Path(__file__).resolve().parents[2]}")
    print("app:", _lib.remote_run(
        f"curl -s -o /dev/null -w '%{{http_code}}' {_lib.APP_BASE_URL}/"
    )[1])

    banner("[2/5] 管理员登录 → /tmp/biztest_cookie")
    admin_user = os.environ.get("ADMIN_USER", "admin")
    admin_pass = os.environ.get("ADMIN_PASS", "admin")
    body = json.dumps({"username": admin_user, "password": admin_pass}, ensure_ascii=False)
    b = base64.b64encode(body.encode("utf-8")).decode("ascii")
    login = (
        f"echo {b} | base64 -d > /tmp/biztest_login.json && "
        f"curl -s -c /tmp/biztest_cookie -w ' HTTP:%{{http_code}}' -X POST "
        f"{_lib.APP_BASE_URL}/api/auth/login -H 'Content-Type: application/json' "
        f"--data-binary @/tmp/biztest_login.json"
    )
    _, out = _lib.remote_run_b64(login)
    print(out[-300:])
    if "HTTP:200" not in out:
        _lib.record("step0", "管理员登录失败", out[-200:], "critical", "无 admin cookie 全部域无法测,检查 admin 账号/密码")
        raise SystemExit("登录失败，中止")

    banner("[3/5] active provider + vision（admin 鉴权验证）")
    provs = _lib.api("GET", "/api/admin/llm-providers", admin=True)
    items = provs.get("items", provs) if isinstance(provs, dict) else provs
    if not isinstance(items, list):
        _lib.record("step0", "无法读 providers（cookie 可能无效）", str(provs)[:200], "critical", "admin 鉴权未生效")
        raise SystemExit("providers 读取失败")
    active = next((p for p in items if p.get("isActive")), None)
    vision = next((p for p in items if p.get("isVisionActive")), None)
    print(f"ACTIVE PROVIDER = {active.get('model') if active else 'NONE'}")
    print(f"VISION PROVIDER = {vision.get('model') if vision else 'NONE → 域⑬ vision 子项将标 BLOCKED'}")
    if not active:
        _lib.record("step0", "无 active LLM provider", str(items)[:200], "critical", "运行时无真模型,全部域无法测")
        raise SystemExit("无 active provider")

    banner("[4/5] 测试 account")
    inventory = _lib.mongo_json(
        'db.wechat_accounts.find({},'
        '{account_id:1,app_id:1,display_name:1,online:1,status:1,webhook_secret:1,_id:0})'
        '.toArray()'
    )
    try:
        account = select_test_account(inventory, TEST_ACCOUNT_ID)
    except ValueError as error:
        safe_inventory = [
            {
                "account_id": row.get("account_id"),
                "online": row.get("online"),
                "status": row.get("status"),
                "has_app_id": bool(row.get("app_id")),
                "has_webhook_secret": bool(row.get("webhook_secret")),
            }
            for row in inventory
            if isinstance(row, dict)
        ] if isinstance(inventory, list) else "<unreadable>"
        _lib.record("step0", "无法唯一选择可用测试 account",
                    f"error={error} inventory={safe_inventory}", "critical",
                    "需唯一 online+active 且具备 app_id/webhook_secret 的账号，或显式设置 BIZTEST_ACCOUNTID")
        raise SystemExit("测试 account 不可用或不唯一") from error
    account_id = account["account_id"]
    app_id = account["app_id"]
    print(f"account_id={account_id} app_id={app_id} name={account.get('display_name')}")

    policies = _lib.mongo_json(
        'db.operation_domain_configs.find({workspace_id:"default",current_version:true},'
        '{ask_human_policy:1,_id:0}).toArray()'
    )
    unsafe = unsafe_principal_targets(policies)
    if unsafe:
        raise SystemExit(
            "生产决策人目标未隔离，拒绝注入式业务测试；请使用随机数据库候选环境。"
            f" unsafePrincipalCount={len(unsafe)}"
        )
    _lib.remote_run(f"echo '{account_id}|{app_id}' > /tmp/biztest_account")

    banner("[5/5] preflight 完成")
    print(f"真模型记录: ACTIVE={active.get('model')} VISION={vision.get('model') if vision else 'NONE'}")
    print("后续域脚本可读 /tmp/biztest_account + /tmp/biztest_cookie")


if __name__ == "__main__":
    main()
