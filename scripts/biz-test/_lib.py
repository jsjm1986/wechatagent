"""WechatAgent 全量业务测试公共库。所有域脚本 import 它。

连 server 117 经 scripts/_remote_run.py(paramiko)，在 server 上 curl localhost:3003 + mongosh wechatagent。
凭据从 env 读(DEPLOY_PASS)/server .env 读(MCP_API_KEY)，绝不写进文件。

观测纪律：业务行为断言查 mongo；journalctl/llm_call_logs 只抓 LLM 真调铁证。
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

REPO = Path(__file__).resolve().parents[2]
REMOTE_RUN = REPO / "scripts" / "_remote_run.py"
BIZ_PREFIX = "biztest_"
FINDINGS = REPO / "docs" / "superpowers" / "specs" / "2026-06-26-full-business-logic-test-findings.md"

# server 连接 env。DEPLOY_PORT 必须 22——_remote_run.py 默认 3003 是 app 端口，不是 SSH。
os.environ.setdefault("DEPLOY_HOST", "117.72.54.28")
os.environ["DEPLOY_PORT"] = "22"
os.environ.setdefault("DEPLOY_USER", "root")
# DEPLOY_PASS 必须由调用者 export，不在这里设默认（绝不硬编码凭据）。


def remote_run(cmd: str) -> tuple[int, str]:
    """在 server 执行 ASCII cmd，返回 (exit_code, combined_output)。"""
    p = subprocess.run(
        [sys.executable, str(REMOTE_RUN), cmd],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return p.returncode, p.stdout


def remote_run_b64(script_text: str) -> tuple[int, str]:
    """server 端跑可能含非 ASCII 的 bash：本地 base64 编码，server 解码执行。"""
    b = base64.b64encode(script_text.encode("utf-8")).decode("ascii")
    return remote_run(f"echo {b} | base64 -d | bash")


def mongo(js: str) -> str:
    """server 上 mongosh wechatagent --quiet --eval。js 经 base64 中转避免引号/中文炸 shell。"""
    b = base64.b64encode(js.encode("utf-8")).decode("ascii")
    script = f"echo {b} | base64 -d > /tmp/biztest_mongo.js && mongosh wechatagent --quiet --file /tmp/biztest_mongo.js"
    _, out = remote_run(script)
    return out


def mongo_json(js: str) -> Any:
    """跑 print(JSON.stringify(<js>))，取输出里第一个能 json.loads 的行。

    mongosh --file 模式不回显表达式值（不像交互 REPL），必须显式 print。
    """
    out = mongo(f"print(JSON.stringify({js}))").strip()
    if not out:
        return None
    # mongosh --quiet 仍可能带连接/弃用提示行，逐行尝试解析，取最后一个成功的。
    parsed: Any = None
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            parsed = json.loads(line)
        except Exception:
            continue
    if parsed is None:
        return {"_raw": out}
    return parsed


def api(method: str, path: str, body: dict | None = None, *, admin: bool = False, timeout: int = 180) -> dict:
    """server 上 curl localhost:3003 打 API。body dict→JSON(base64 中转)。admin 带 cookie。"""
    base = [
        "curl", "-s", "-X", method, f"http://localhost:3003{path}",
        "-H", "'Content-Type: application/json'", "--max-time", str(timeout),
    ]
    if admin:
        base += ["-b", "/tmp/biztest_cookie"]
    if body is not None:
        b = base64.b64encode(json.dumps(body, ensure_ascii=False).encode("utf-8")).decode("ascii")
        script = (
            f"echo {b} | base64 -d > /tmp/biztest_body.json && "
            + " ".join(base) + " --data-binary @/tmp/biztest_body.json"
        )
    else:
        script = " ".join(base)
    _, out = remote_run_b64(script)
    txt = out.strip()
    # 取最后一个能解析的 JSON 行（curl 前可能有 base64 写文件的无输出）。
    for line in reversed(txt.splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except Exception:
            continue
    return {"_raw": out}


def _mcp_key() -> str:
    """从 server .env 读 MCP_API_KEY（不落本地文件）。"""
    _, out = remote_run("grep -E '^MCP_API_KEY=' /opt/wechatagent/.env | head -1 | cut -d= -f2-")
    return out.strip()


def send_webhook(app_id: str, from_wxid: str, content: str, msg_id: str) -> dict:
    """算 HMAC-SHA256(MCP_API_KEY, raw_body) hex → X-MCP-Signature，POST /webhooks/wechat。"""
    body = json.dumps(
        {"appId": app_id, "fromWxid": from_wxid, "content": content, "msgId": msg_id},
        ensure_ascii=False,
    )
    sig = hmac.new(_mcp_key().encode("utf-8"), body.encode("utf-8"), hashlib.sha256).hexdigest()
    b = base64.b64encode(body.encode("utf-8")).decode("ascii")
    script = (
        f"echo {b} | base64 -d > /tmp/biztest_wh.json && "
        f"curl -s -X POST http://localhost:3003/webhooks/wechat "
        f"-H 'Content-Type: application/json' -H 'X-MCP-Signature: {sig}' "
        f"--data-binary @/tmp/biztest_wh.json"
    )
    _, out = remote_run_b64(script)
    for line in reversed(out.strip().splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except Exception:
            continue
    return {"_raw": out}


def llm_logs_since(seconds: int, prompt_key: str | None = None) -> list[dict]:
    """查 llm_call_logs 最近 N 秒记录（mongo），用于真调铁证断言。

    注意：mongo BSON 字段是 snake_case（prompt_key/created_at），非 API 的 camelCase。
    """
    since = int(time.time() * 1000) - seconds * 1000
    if prompt_key:
        q = f'{{created_at:{{$gte:new Date({since})}},prompt_key:"{prompt_key}"}}'
    else:
        q = f'{{created_at:{{$gte:new Date({since})}}}}'
    js = (
        f"db.llm_call_logs.find({q},{{prompt_key:1,status:1,model:1,_id:0}})"
        f".sort({{_id:-1}}).limit(20).toArray()"
    )
    res = mongo_json(js)
    return res if isinstance(res, list) else []


def assert_llm_success(seconds: int, prompt_key: str, domain: str) -> bool:
    """真调铁证：最近 N 秒该 prompt_key 有 success 记录，否则 record 一条 high（可能假绿）。"""
    logs = llm_logs_since(seconds, prompt_key)
    ok = any(l.get("status") == "success" for l in logs)
    if not ok:
        record(
            domain,
            f"{prompt_key} 无 success 的 llm_call_logs",
            f"logs={logs}",
            "high",
            "LLM 未真调或失败(json_error/failed)→该域断言可能假绿，排查端点",
        )
    return ok


_INIT = False


def record(domain: str, phenomenon: str, evidence: str, severity: str, root_cause: str) -> None:
    """把一条 finding append 到问题清单 md。"""
    global _INIT
    if not _INIT:
        FINDINGS.parent.mkdir(parents=True, exist_ok=True)
        if not FINDINGS.exists():
            FINDINGS.write_text(
                "# 全量业务逻辑测试 问题清单\n\n"
                "| 域 | 现象 | severity | 根因初判 | 证据 |\n"
                "|---|---|---|---|---|\n",
                encoding="utf-8",
            )
        _INIT = True
    ev = evidence.replace("\n", " ").replace("|", "/")[:500]
    with FINDINGS.open("a", encoding="utf-8") as f:
        f.write(f"| {domain} | {phenomenon} | {severity} | {root_cause} | `{ev}` |\n")
    print(f"[FINDING/{severity}] {domain}: {phenomenon}", flush=True)


def expect(cond: bool, domain: str, desc: str, evidence: str, severity: str = "high", root: str = "") -> bool:
    """断言：真→打 PASS；假→record finding。返回 cond 供链式判断。"""
    if cond:
        print(f"[PASS] {domain}: {desc}", flush=True)
    else:
        record(domain, f"断言失败: {desc}", evidence, severity, root or "断言不成立，见证据")
    return cond


def biztest_account() -> tuple[str, str]:
    """读 step0 写的 /tmp/biztest_account（account_id|app_id）。"""
    _, out = remote_run("cat /tmp/biztest_account")
    parts = out.strip().split("|")
    if len(parts) != 2:
        raise SystemExit(f"未找到 /tmp/biztest_account，请先跑 step0_preflight.py。got={out!r}")
    return parts[0], parts[1]


def ensure_managed_contact(account_id: str, wxid: str, nickname: str = "biztest 客户") -> None:
    """把测试 contact 设成 managed（webhook 触发 agent 决策链的前提）。

    contacts 集合主键字段是 wxid + account_id（注意：不是 contact_wxid，那是别的集合的字段）。
    upsert：不存在则建，存在则确保 agent_status=managed。
    """
    js = (
        f'db.contacts.updateOne('
        f'{{wxid:"{wxid}",account_id:"{account_id}"}},'
        f'{{$set:{{agent_status:"managed",nickname:"{nickname}"}},'
        f'$setOnInsert:{{workspace_id:"default",created_at:new Date()}}}},'
        f'{{upsert:true}})'
    )
    mongo(js)


def reset_contact_conversation(account_id: str, wxid: str) -> None:
    """清掉某测试 contact 的历史对话/run/outbox/events，保证每次触发是干净起点。"""
    for c, field in [
        ("conversation_messages", "contact_wxid"),
        ("agent_run_logs", "contact_wxid"),
        ("agent_send_outbox", "contact_wxid"),
        ("agent_events", "contact_wxid"),
    ]:
        mongo(f'db.{c}.deleteMany({{{field}:"{wxid}",account_id:"{account_id}"}})')
