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

sys.path.insert(0, str(Path(__file__).parent))
import _lib

# 测试 account：用户冒烟时定的 account_id=2（客服b）。
TEST_ACCOUNT_ID = os.environ.get("BIZTEST_ACCOUNTID", "2")


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
    print(_lib.remote_run("cd /opt/wechatagent && git rev-parse HEAD && git log --oneline -2")[1])
    print("app:", _lib.remote_run("curl -s -o /dev/null -w '%{http_code}' http://localhost:3003/")[1])

    banner("[2/5] 管理员登录 → /tmp/biztest_cookie")
    admin_user = os.environ.get("ADMIN_USER", "admin")
    admin_pass = os.environ.get("ADMIN_PASS", "admin")
    body = json.dumps({"username": admin_user, "password": admin_pass}, ensure_ascii=False)
    b = base64.b64encode(body.encode("utf-8")).decode("ascii")
    login = (
        f"echo {b} | base64 -d > /tmp/biztest_login.json && "
        f"curl -s -c /tmp/biztest_cookie -w ' HTTP:%{{http_code}}' -X POST "
        f"http://localhost:3003/api/auth/login -H 'Content-Type: application/json' "
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
    acc = _lib.mongo_json(
        f'db.wechat_accounts.find({{account_id:"{TEST_ACCOUNT_ID}"}},'
        f'{{account_id:1,app_id:1,display_name:1,online:1,webhook_secret:1,_id:0}}).toArray()'
    )
    if not acc or not isinstance(acc, list) or not acc[0].get("app_id"):
        _lib.record("step0", f"测试 account_id={TEST_ACCOUNT_ID} 不存在", str(acc)[:200], "critical", "无可用 account")
        raise SystemExit("测试 account 不存在")
    account = acc[0]
    app_id = account["app_id"]
    print(f"account_id={TEST_ACCOUNT_ID} app_id={app_id} name={account.get('display_name')}")
    if account.get("online") is not True:
        raise SystemExit("测试账号不在线；拒绝修改生产账号状态，请先使用隔离测试账号")
    secret = account.get("webhook_secret")
    if not isinstance(secret, str) or not secret.strip():
        raise SystemExit("测试账号未配置 webhook_secret，无法按正式协议验签")

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
    _lib.remote_run(f"echo '{TEST_ACCOUNT_ID}|{app_id}' > /tmp/biztest_account")

    banner("[5/5] preflight 完成")
    print(f"真模型记录: ACTIVE={active.get('model')} VISION={vision.get('model') if vision else 'NONE'}")
    print("后续域脚本可读 /tmp/biztest_account + /tmp/biztest_cookie")


if __name__ == "__main__":
    main()
