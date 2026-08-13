"""WechatAgent 全量业务测试公共库。所有域脚本 import 它。

支持远程 SSH 或服务器本机运行；API 端口与 Mongo 数据库由严格校验的 BIZTEST_APP_PORT/BIZTEST_DATABASE 绑定。
凭据从 env 读(DEPLOY_PASS)/server .env 读(MCP_API_KEY)，绝不写进文件。

观测纪律：业务行为断言查 mongo；journalctl/llm_call_logs 只抓 LLM 真调铁证。
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import re
import shlex
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
BLOCKED_LEDGER = REPO / "target" / "biztest_blocked.jsonl"

# server 连接 env。DEPLOY_PORT 必须 22——_remote_run.py 默认 3003 是 app 端口，不是 SSH。
os.environ.setdefault("DEPLOY_HOST", "117.72.54.28")
os.environ["DEPLOY_PORT"] = "22"
os.environ.setdefault("DEPLOY_USER", "root")
# DEPLOY_PASS 必须由调用者 export，不在这里设默认（绝不硬编码凭据）。


LOCAL_MODE = os.environ.get("BIZTEST_LOCAL") == "1"


def _validated_app_port(raw: str) -> int:
    try:
        port = int(raw)
    except ValueError as error:
        raise ValueError("BIZTEST_APP_PORT must be an integer") from error
    if not 1024 <= port <= 65535:
        raise ValueError("BIZTEST_APP_PORT must be between 1024 and 65535")
    return port


def _validated_database(raw: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9_-]+", raw):
        raise ValueError("BIZTEST_DATABASE contains unsafe characters")
    if raw in {"admin", "config", "local"}:
        raise ValueError("BIZTEST_DATABASE cannot be a Mongo system database")
    return raw


BIZTEST_APP_PORT = _validated_app_port(os.environ.get("BIZTEST_APP_PORT", "3003"))
BIZTEST_DATABASE = _validated_database(os.environ.get("BIZTEST_DATABASE", "wechatagent"))
APP_BASE_URL = f"http://127.0.0.1:{BIZTEST_APP_PORT}"


def remote_run(cmd: str) -> tuple[int, str]:
    """在 server 执行 ASCII cmd，返回 (exit_code, combined_output)。

    默认（远程模式）：cmd 经 stdin 传给 _remote_run.py（argv[1]=='-'），走 paramiko
    SSH，避开 Windows argv 32KB 上限（大 body base64 后可达数十 KB，走 argv 会 WinError 206）。

    BIZTEST_LOCAL=1（本机模式）：整套脚本已 setsid 托管在 server 上直接跑，cmd 直接用
    本机 bash 执行——免 SSH-to-self、免 fail2ban、免本机会话中断被杀。合并 stdout+stderr
    与远程 PTY 行为一致（解析方按"最后一个可解析 JSON 行"取值，stderr 噪声不影响）。
    """
    if LOCAL_MODE:
        p = subprocess.run(
            ["bash", "-c", cmd],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    p = subprocess.run(
        [sys.executable, str(REMOTE_RUN), "-"],
        input=cmd,
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
    """Run mongosh and fail closed when the remote command does not succeed."""
    b = base64.b64encode(js.encode("utf-8")).decode("ascii")
    script = (f"echo {b} | base64 -d > /tmp/biztest_mongo.js && "
              f"mongosh {shlex.quote(BIZTEST_DATABASE)} --quiet --file /tmp/biztest_mongo.js")
    return_code, out = remote_run(script)
    if return_code != 0:
        raise RuntimeError(f"mongosh failed with exit code {return_code}: {out[-500:]}")
    return out


def mongo_json(js: str) -> Any:
    """跑 print(JSON.stringify(<js>))，取输出里第一个能 json.loads 的行。

    mongosh --file 模式不回显表达式值（不像交互 REPL），必须显式 print。
    """
    out = mongo(f"print(JSON.stringify({js}))").strip()
    if not out:
        return None
    # mongosh --quiet 仍可能带连接/弃用提示行，逐行尝试解析，取最后一个成功的。
    missing = object()
    parsed: Any = missing
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            parsed = json.loads(line)
        except Exception:
            continue
    if parsed is missing:
        return {"_raw": out}
    return parsed


def api(method: str, path: str, body: dict | None = None, *, admin: bool = False, timeout: int = 180) -> dict:
    """Call the explicitly bound biz-test API. body dict→JSON(base64 中转)."""
    base = [
        "curl", "-s", "-X", method, f"{APP_BASE_URL}{path}",
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


def campaign_dispatch_body(spec: dict) -> dict:
    """Extract the complete frozen Campaign identity returned by create/preview."""
    spec_hash = spec.get("specHash") if isinstance(spec, dict) else None
    spec_version = spec.get("specVersion") if isinstance(spec, dict) else None
    if not isinstance(spec_hash, str) or not spec_hash.strip():
        raise ValueError("campaign response is missing specHash")
    if not isinstance(spec_version, int) or isinstance(spec_version, bool) or spec_version < 1:
        raise ValueError("campaign response is missing a positive specVersion")
    return {"specHash": spec_hash, "specVersion": spec_version}


def management_command_binding(command: dict, account_id: str) -> dict:
    """Bind confirm/reject to the selected account and server-issued frozen plan hash."""
    plan_hash = command.get("planHash") if isinstance(command, dict) else None
    if not account_id.strip():
        raise ValueError("management command account_id is empty")
    if not isinstance(plan_hash, str) or not plan_hash.strip():
        raise ValueError("management command response is missing planHash")
    return {"accountId": account_id, "planHash": plan_hash}


def guide_apply_binding(preview: dict, *, confirm_global_impact: bool | None = None) -> dict:
    """Bind Guide Apply to the exact server-issued preview identity and frozen candidate."""
    item = preview.get("item") if isinstance(preview, dict) else None
    item = item if isinstance(item, dict) else preview
    required = {
        "previewId": item.get("id") if isinstance(item, dict) else None,
        "expectedAccountId": item.get("accountId") if isinstance(item, dict) else None,
        "expectedContactId": item.get("contactId") if isinstance(item, dict) else None,
        "candidateHash": item.get("candidateHash") if isinstance(item, dict) else None,
    }
    if not all(isinstance(value, str) and value.strip() for value in required.values()):
        raise ValueError("guide preview response is missing frozen apply identity")
    requires = bool(item.get("requiresStrongConfirmation"))
    required["confirmGlobalImpact"] = requires if confirm_global_impact is None else bool(confirm_global_impact)
    return required


def domain_profile_identity(response: dict) -> tuple[str, int | None]:
    """Return the exact immutable profile row id/version from generate or publish responses."""
    object_id = response.get("id") if isinstance(response, dict) else None
    version = response.get("version") if isinstance(response, dict) else None
    if not isinstance(object_id, str) or not object_id.strip():
        raise ValueError("domain profile response is missing id")
    if version is not None and (not isinstance(version, int) or isinstance(version, bool)):
        raise ValueError("domain profile response has invalid version")
    return object_id, version


def bson_object_id(value: object) -> str:
    """Normalize mongosh Extended JSON ObjectId values without accepting empty identities."""
    if isinstance(value, dict):
        value = value.get("$oid")
    if not isinstance(value, str) or len(value) != 24:
        return ""
    return value


def sign_webhook_body(secret: str, timestamp_ms: str, raw_body: bytes) -> str:
    """Return the gewe-agent HMAC for ``<timestamp>.<raw_body>``.

    Production verifies the per-account ``webhook_secret`` rather than the MCP API key.
    Keeping this pure makes protocol drift visible in unit tests.
    """
    if not secret.strip():
        raise ValueError("webhook secret is empty")
    if not timestamp_ms.isdigit():
        raise ValueError("webhook timestamp must be milliseconds")
    signed = timestamp_ms.encode("ascii") + b"." + raw_body
    return hmac.new(secret.encode("utf-8"), signed, hashlib.sha256).hexdigest()


def _webhook_secret(app_id: str) -> str:
    """Read the account-scoped webhook secret without writing or printing it."""
    row = mongo_json(
        f'db.wechat_accounts.findOne({{app_id:{json.dumps(app_id)}}},'
        '{webhook_secret:1,_id:0})'
    )
    secret = row.get("webhook_secret") if isinstance(row, dict) else None
    if not isinstance(secret, str) or not secret.strip():
        raise RuntimeError("account webhook_secret is not configured")
    return secret


def api_bg(method: str, path: str, body: dict | None = None, *, admin: bool = False,
           max_wait: int = 720, poll: int = 10, tag: str = "bg") -> dict:
    """真调 LLM 的长请求：server 端 setsid 后台跑 curl + 轮询，脱离 SSH PTY 会话。

    主会话同步 curl 受 paramiko PTY 限制：单次 LLM 调用可达 100s+（claude-opus 长 JSON
    输出实测 113s，最坏 timeout×retries≈720s），同步等待会被 SSH/curl 超时杀掉造成假阳。
    本函数把请求写进 server 后台进程（setsid 免 SIGHUP），主会话只轮询结果文件。

    返回解析后的 JSON dict；失败/超时返回 {"_error": ...}。
    """
    out_f = f"/tmp/biztest_{tag}_out.json"
    err_f = f"/tmp/biztest_{tag}_err.txt"
    done_f = f"/tmp/biztest_{tag}_done"
    start_f = f"/tmp/biztest_{tag}_start"
    end_f = f"/tmp/biztest_{tag}_end"
    body_f = f"/tmp/biztest_{tag}_body.json"
    runner_f = f"/tmp/biztest_{tag}_runner.sh"

    curl = [
        "curl", "-s", "--max-time", str(max_wait), "-X", method,
        f"{APP_BASE_URL}{path}", "-H", "'Content-Type: application/json'",
    ]
    if admin:
        curl += ["-b", "/tmp/biztest_cookie"]
    data_line = ""
    if body is not None:
        bb = base64.b64encode(json.dumps(body, ensure_ascii=False).encode()).decode()
        curl += ["--data-binary", f"@{body_f}"]
    runner = (
        "#!/bin/bash\n"
        f"date +%s > {start_f}\n"
        f"rm -f {out_f} {err_f} {done_f} {end_f}\n"
        + " ".join(curl) + f" > {out_f} 2> {err_f}\n"
        f"date +%s > {end_f}\n"
        f"touch {done_f}\n"
    )
    rb = base64.b64encode(runner.encode()).decode()
    setup = ""
    if body is not None:
        setup += f"echo {bb} | base64 -d > {body_f} && "
    setup += (
        f"echo {rb} | base64 -d > {runner_f} && chmod +x {runner_f} && "
        f"setsid {runner_f} < /dev/null > /tmp/biztest_{tag}_nohup.log 2>&1 & disown; "
        f"sleep 1; echo launched"
    )
    remote_run_b64(setup)

    waited = 0
    while waited < max_wait + 30:
        chk = remote_run_b64(
            f"if [ -f {done_f} ]; then echo DONE; else echo RUN; fi; "
            f"for i in $(seq 1 {max(1, poll // 1)}); do [ -f {done_f} ] && break; sleep 1; done"
        )[1]
        if "DONE" in chk:
            break
        waited += poll
    res = remote_run_b64(
        f"if [ -f {done_f} ]; then cat {out_f}; else echo '__TIMEOUT__'; fi"
    )[1].strip()
    if "__TIMEOUT__" in res or not res:
        return {"_error": "timeout", "_waited": waited}
    for line in reversed(res.splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except Exception:
            continue
    return {"_raw": res[:500]}


def send_webhook(app_id: str, from_wxid: str, content: str, msg_id: str) -> dict:
    """Inject one strictly namespaced webhook using the production signature protocol."""
    if not from_wxid.startswith(BIZ_PREFIX) or not msg_id.startswith(BIZ_PREFIX):
        raise ValueError("biz-test webhooks require biztest_ sender and message ids")
    body = json.dumps(
        {"appId": app_id, "fromWxid": from_wxid, "content": content, "msgId": msg_id},
        ensure_ascii=False,
    )
    raw_body = body.encode("utf-8")
    timestamp_ms = str(int(time.time() * 1000))
    sig = sign_webhook_body(_webhook_secret(app_id), timestamp_ms, raw_body)
    b = base64.b64encode(raw_body).decode("ascii")
    script = (
        f"echo {b} | base64 -d > /tmp/biztest_wh.json && "
        f"curl -s -X POST {APP_BASE_URL}/webhooks/wechat "
        f"-H 'Content-Type: application/json' "
        f"-H 'X-Webhook-Timestamp: {timestamp_ms}' "
        f"-H 'X-Webhook-Signature: sha256={sig}' "
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


def run_log_count(wxid: str) -> int:
    """该 contact 当前 agent_run_logs 文档数。轮询基准（snake_case contact_wxid）。"""
    n = mongo_json(f'db.agent_run_logs.countDocuments({{contact_wxid:"{wxid}"}})')
    return n if isinstance(n, int) else 0


def inflight_inbound(wxid: str, window_s: int = 200) -> int:
    """该 contact 近 window_s 秒有几条 inbound 还没对应的终态 run log（疑似 runner 在跑）。

    barge-in 抢占（gateway.rs:2091）：runner 决策审查完成、写库前若来了更新 inbound，
    就放弃本轮交调度器重算，落 superseded_by_new_inbound。测试若在上一轮 runner 还在
    后台跑时就发下一条，必触发抢占→上一轮 run 被中止、断言假阴（域④路径2 实证：
    引荐话术 run 3a352fb2 被 superseded，名片没机会入 outbox）。

    粗略判据：近窗口 inbound 数 > 该 contact 终态 run log 数 → 有未消化的 inbound，
    runner 可能在跑。返回差值（>0 = 疑似 in-flight），供 send 前等空闲。
    """
    since = int(time.time() * 1000) - window_s * 1000
    n_in = mongo_json(
        f'db.conversation_messages.countDocuments({{contact_wxid:"{wxid}",'
        f'direction:"inbound",created_at:{{$gte:new Date({since})}}}})'
    )
    n_run = mongo_json(
        f'db.agent_run_logs.countDocuments({{contact_wxid:"{wxid}",'
        f'created_at:{{$gte:new Date({since})}}}})'
    )
    n_in = n_in if isinstance(n_in, int) else 0
    n_run = n_run if isinstance(n_run, int) else 0
    return max(0, n_in - n_run)


def wait_contact_idle(wxid: str, max_wait: int = 320, poll: int = 12) -> bool:
    """发下一条 webhook 前，等该 contact 没有 in-flight runner（避免 barge-in 抢占）。

    返回 True=已空闲可发；False=超时仍疑似在跑（调用方可照常发，但需知有抢占风险）。
    """
    deadline = time.time() + max_wait
    while time.time() < deadline:
        if inflight_inbound(wxid) == 0:
            return True
        time.sleep(poll)
    return False


def _default_min_reply_interval_seconds() -> int:
    """Read the deployment fallback without exposing unrelated environment values."""
    raw = os.environ.get("AGENT_MIN_REPLY_INTERVAL_SECONDS")
    if raw is None and not LOCAL_MODE:
        command = (
            'workdir=$(systemctl show wechatagent -p WorkingDirectory --value) && '
            'set -a && . "$workdir/.env" && set +a && '
            'printf "%s" "${AGENT_MIN_REPLY_INTERVAL_SECONDS:-20}"'
        )
        code, output = remote_run(command)
        if code != 0:
            raise RuntimeError("cannot read deployment min reply interval")
        raw = output.strip().splitlines()[-1] if output.strip() else ""
    if raw is None:
        raw = "20"
    try:
        value = int(raw)
    except (TypeError, ValueError) as error:
        raise RuntimeError("invalid AGENT_MIN_REPLY_INTERVAL_SECONDS") from error
    if value < 0 or value > 3600:
        raise RuntimeError("AGENT_MIN_REPLY_INTERVAL_SECONDS is outside safe bounds")
    return value


def reply_window_remaining_ms(wxid: str) -> int:
    """Return the exact production precheck window remaining for one test contact."""
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    fallback = _default_min_reply_interval_seconds()
    value = mongo_json(
        "(() => { const contact=db.contacts.findOne("
        f"{{wxid:{json.dumps(wxid)}}},{{last_outbound_at:1,_id:0}});"
        "const config=db.operation_domain_configs.findOne("
        "{workspace_id:'default',domain:'user_operations',current_version:true},"
        "{runtime_parameters:1,_id:0});"
        "const configured=config&&config.runtime_parameters"
        "?config.runtime_parameters.minReplyIntervalSeconds:null;"
        f"const interval=Number.isInteger(configured)?configured:{fallback};"
        "if(interval<0||interval>3600) throw new Error('invalid minReplyIntervalSeconds');"
        "const last=contact&&contact.last_outbound_at;"
        "return last instanceof Date"
        "?Math.max(0,interval*1000-(Date.now()-last.getTime())):0; })()"
    )
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise RuntimeError(f"unreadable reply window for {wxid}: {value}")
    return value


def wait_contact_reply_window(wxid: str, max_wait: int = 120) -> bool:
    """Wait out the configured production anti-spam interval; never clear its state."""
    deadline = time.time() + max_wait
    while True:
        remaining_ms = reply_window_remaining_ms(wxid)
        if remaining_ms <= 0:
            return True
        now = time.time()
        if now >= deadline or now + remaining_ms / 1000.0 + 1 > deadline:
            return False
        time.sleep(min(remaining_ms / 1000.0 + 1, 5.0))


def diagnose_no_run_log(wxid: str) -> dict:
    """run log 不落时的根因取证：一次性查清"inbound 落了但 run log 不来"属于哪种路径。

    后端实证（webhooks.rs / agent/gateway.rs）确认 run log **没有"必落"保证**——
    它只在 gateway _inner 各终态点显式 write_agent_run_log，任一上游 `.await?` 抛错
    （LLM 决策调用全失败最常见）会在到达写入点前 short-circuit，run log 永不产生，
    只在 agent_events 留一条 agent_error。故 run1=None 必须用本函数区分：
      - agent_status != managed → 根本不 spawn runner（webhooks.rs:578，永不落，设计如此）
      - inbound 未落 conversation_messages → webhook 没进站（HMAC/appId/限流/dedup）
      - agent_events 有 agent_error → gateway 抛错（多半 LLM 端点失败，标 BLOCKED 非 bug）
      - agent_events 有 webhook_handler_panic → 后台 runner panic（catch_unwind 兜住，不落 run log）
      - 以上都没有但 inbound 在 → 可能 quiet-hours defer（>600s 才醒）或仍在跑/端点超时
    返回一个 dict（各维度证据），同时打印便于直接看。
    """
    js = (
        'function j(k,x){print("[diag/"+k+"] "+JSON.stringify(x))} '
        f'j("agent_status",db.contacts.find({{wxid:"{wxid}"}},{{agent_status:1,account_id:1,_id:0}}).toArray());'
        f'j("inbound_count",db.conversation_messages.countDocuments({{contact_wxid:"{wxid}"}}));'
        f'j("run_log_count",db.agent_run_logs.countDocuments({{contact_wxid:"{wxid}"}}));'
        f'j("errors",db.agent_events.find({{contact_wxid:"{wxid}",kind:/error|panic|fail/i}},'
        '{kind:1,status:1,summary:1,_id:0}).sort({_id:-1}).limit(5).toArray());'
        f'j("recent_kinds",db.agent_events.find({{contact_wxid:"{wxid}"}},{{kind:1,_id:0}})'
        '.sort({_id:-1}).limit(8).toArray());'
        'j("llm_recent",db.llm_call_logs.find({},{prompt_key:1,status:1,_id:0})'
        '.sort({_id:-1}).limit(6).toArray());'
    )
    out = mongo(js)
    diag: dict[str, Any] = {"_raw": out}
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("[diag/") and "] " in line:
            key = line[len("[diag/"):line.index("]")]
            try:
                diag[key] = json.loads(line[line.index("] ") + 2:])
            except Exception:
                pass
    print(f"[diagnose_no_run_log/{wxid}]\n{out}", flush=True)
    return diag


def wait_run(wxid: str, prev_count: int = 0, *, max_wait: int = 600, poll: int = 12,
             diagnose_on_timeout: bool = True) -> dict | None:
    """轮询 agent_run_logs 直到该 contact 的 run log 数 > prev_count（一轮 webhook 处理完成）。

    webhook 后台 spawn 去抖 runner，一轮含 reaction+decision+review 多次真模型调用，
    rsxermu 单次 18-113s、并发≤2，最坏一轮 300s+，故固定 sleep 必假阴，必须轮询。
    run log 是 gateway 跑完一轮后一次性 insert 的终态记录（无 started 占位行，
    write_agent_run_log_with_finalize），所以"出现新 run log"=该轮已处理完成。
    返回最新一条（run_id/lifecycle/final_review_status/status/outbox_status），超时返回 None。

    用 time.time() 墙钟计时（不能用固定 poll 累加）：每轮 run_log_count 是一次
    SSH+mongosh 往返(~10s)，远大于 poll，固定累加会让 max_wait 失真（实测 600 跑成 957s）。

    超时返回 None 前默认跑 diagnose_no_run_log 取证（diagnose_on_timeout），把根因
    （非 managed / 没进站 / agent_error / panic / quiet-defer）直接 dump 出来，
    免得每次 run1=None 都要手工上 server 查。
    """
    deadline = time.time() + max_wait
    while time.time() < deadline:
        if run_log_count(wxid) > prev_count:
            rows = mongo_json(
                f'db.agent_run_logs.find({{contact_wxid:"{wxid}"}},'
                '{run_id:1,lifecycle:1,final_review_status:1,status:1,outbox_status:1,_id:0})'
                '.sort({created_at:-1}).limit(1).toArray()'
            )
            return rows[0] if isinstance(rows, list) and rows else {}
        time.sleep(poll)
    if diagnose_on_timeout:
        print(f"[wait_run] {wxid} 超时未落 run log（{max_wait}s），取证根因：", flush=True)
        diagnose_no_run_log(wxid)
    return None


def endpoint_glitch_recent(wxid: str) -> dict | None:
    """该 contact 最近是否有"LLM 端点偶发故障"事件（tool_use 劫持 / unavailable / 5xx）。

    rsxermu claude 偶发返回 tool_use 而非 JSON（llm_tool_use_instead_of_json，knowledge.agent
    这类带 tool-calling 的环节高发，~10%）。命中时 agent 抛 agent_error、run 落
    failed_before_decision、不落终态 run log。这是**端点/模型不遵从**（同输入重跑就成功），
    不是项目 bug、不是测试 bug——按 design spec「LLM 单次发挥不稳要多跑」，测试侧重试规避，
    绝不改业务 LLM 重试逻辑（那是有意设计：tool_use 治本靠 prompt 禁工具，不靠回喂重试）。

    返回 agent_error 事件 dict（命中）或 None（无端点故障，可能是真业务结果或别的根因）。
    """
    rows = mongo_json(
        f'db.agent_events.find({{contact_wxid:"{wxid}",kind:"agent_error"}},'
        '{summary:1,created_at:1,_id:0}).sort({_id:-1}).limit(1).toArray()'
    )
    if not (isinstance(rows, list) and rows):
        return None
    summary = str(rows[0].get("summary", ""))
    markers = ("llm_tool_use_instead_of_json", "llm unavailable", "external_error",
               "LLM HTTP 5", "body_decode_error", "timeout")
    return rows[0] if any(m in summary for m in markers) else None


def send_and_wait(app_id: str, wxid: str, content: str, tag: str, *,
                  max_wait: int = 600, poll: int = 12, endpoint_retries: int = 2,
                  retry_gap: int = 25) -> dict | None:
    """发一条 webhook → 轮询等这一轮 gateway 处理完成，返回该轮 run log（超时 None）。

    webhook handler 注册 inbound 后立即返回 200（runner 在后台 spawn），故必须轮询 mongo。

    端点偶发故障自动重试（endpoint_retries 次）：若本轮没落 run log 且 agent_events 显示
    是 LLM 端点偶发故障（tool_use 劫持 / unavailable / 5xx，见 endpoint_glitch_recent），
    则等 retry_gap 秒（>minReplyIntervalSeconds:20 避免撞限流）后重发同一条 webhook 重试。
    这是对端点不稳的测试侧容错（design spec 风险缓解：LLM 单次发挥不稳要多跑），
    **只对端点故障重试，业务终态（completed/precheck/rate_limited）一律不重试**——
    后者是真实结果，重试会掩盖真问题。
    """
    attempt = 0
    while True:
        # 发前等 contact 空闲：上一轮 runner 还在跑时发会触发 barge-in 抢占
        # （gateway.rs:2091 superseded_by_new_inbound），让上一轮 run 被中止断言假阴。
        if not wait_contact_idle(wxid):
            raise RuntimeError(f"contact {wxid} did not become idle before send")
        if not wait_contact_reply_window(wxid):
            raise RuntimeError(f"contact {wxid} remained inside min reply interval")
        prev = run_log_count(wxid)
        send_webhook(app_id, wxid, content, f"biztest_{tag}_{int(time.time()*1000)}")
        # 端点故障重试时不重复 dump 诊断（最后一次失败才 dump）
        run = wait_run(wxid, prev, max_wait=max_wait, poll=poll,
                       diagnose_on_timeout=(attempt >= endpoint_retries))
        if run is not None:
            return run
        # 没落 run log：区分端点偶发故障（可重试）vs 其它根因（不重试）
        glitch = endpoint_glitch_recent(wxid)
        if glitch and attempt < endpoint_retries:
            attempt += 1
            print(f"[send_and_wait] {wxid} 端点偶发故障(第{attempt}/{endpoint_retries}次重试)，"
                  f"等{retry_gap}s避限流后重发。glitch={str(glitch.get('summary',''))[:80]}",
                  flush=True)
            time.sleep(retry_gap)
            continue
        return None


def latest_decision_review(wxid: str) -> dict:
    """该 contact 最新一条 agent_decision_reviews。

    reaction 分析（outcome_status/reaction_analysis）与五维评分（scores 子文档,内部键 camelCase:
    factRisk/pressureRisk/humanLikeScore/emotionalValue/productAccuracy）、used_knowledge_ids
    (Vec<ObjectId>)、operation_state 都在这里——不在 agent_run_logs。按 run_id/contact_wxid 关联。
    """
    rows = mongo_json(
        f'db.agent_decision_reviews.find({{contact_wxid:"{wxid}"}},'
        '{run_id:1,used_knowledge_ids:1,scores:1,operation_state:1,approved:1,'
        'outcome_status:1,reaction_analysis:1,_id:0}).sort({_id:-1}).limit(1).toArray()'
    )
    return rows[0] if isinstance(rows, list) and rows else {}


def decision_review_for_run(wxid: str, run_id: str) -> dict:
    """Return the review bound to one exact run, never a later contact-level row."""
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    if not run_id.strip():
        return {}
    rows = mongo_json(
        "db.agent_decision_reviews.find("
        f"{{contact_wxid:{json.dumps(wxid)},run_id:{json.dumps(run_id)}}},"
        "{run_id:1,used_knowledge_ids:1,scores:1,status:1,outcome_status:1,reaction_analysis:1,"
        "approved:1,send_gateway_result:1,operation_state:1,post_decision_status:1,"
        "post_decision_error_kind:1,post_decision_profile_done:1,"
        "post_decision_memory_done:1,_id:1}).sort({_id:-1}).limit(1).toArray()"
    )
    return rows[0] if isinstance(rows, list) and rows else {}


def wait_review_status(wxid: str, run_id: str, statuses: set[str], *, max_wait: int = 180,
                       poll: int = 5) -> dict:
    """Wait until an exact review reaches a dispatcher-backed status such as ``sent``."""
    deadline = time.time() + max_wait
    while time.time() < deadline:
        review = decision_review_for_run(wxid, run_id)
        if review.get("status") in statuses:
            return review
        time.sleep(poll)
    return decision_review_for_run(wxid, run_id)


def memory_task_evidence(task_id: str) -> dict:
    """Read one exact manual consolidation task and its prepared/committed run identity."""
    if not task_id.strip():
        return {}
    rows = mongo_json(
        "db.agent_tasks.find("
        f"{{_id:ObjectId({json.dumps(task_id)})}},"
        "{status:1,gateway_status:1,claim_generation:1,prepared_commit:1,_id:1}).limit(1).toArray()"
    )
    return rows[0] if isinstance(rows, list) and len(rows) == 1 else {}


def memory_commit_events(task_id: str, claim_generation: int) -> list[dict]:
    """Return only durable events owned by one exact memory commit attempt."""
    if not task_id.strip() or not isinstance(claim_generation, int):
        return []
    prefix = f"memory_commit:{task_id}:{claim_generation}:"
    rows = mongo_json(
        "db.agent_events.find("
        f"{{dedupe_key:{{$regex:{json.dumps('^' + prefix)}}}}},"
        "{kind:1,status:1,details:1,dedupe_key:1,_id:0}).sort({dedupe_key:1}).toArray()"
    )
    return rows if isinstance(rows, list) else []


def wait_projection_terminal(wxid: str, run_id: str, *, max_wait: int = 300,
                             poll: int = 5) -> dict:
    """Wait for the exact decision review's asynchronous post-decision projection terminal."""
    terminal = {"completed", "failed_terminal", "discarded"}
    deadline = time.time() + max_wait
    while time.time() < deadline:
        review = decision_review_for_run(wxid, run_id)
        if review.get("post_decision_status") in terminal:
            return review
        time.sleep(poll)
    return decision_review_for_run(wxid, run_id)


def relationship_suggestions_for_run(contact_id: str, run_id: str) -> list[dict]:
    """Resolve suggestions through the strict projection-observation ledger identity."""
    if not contact_id or not run_id.strip():
        return []
    rows = mongo_json(
        "(() => { const obs=db.projection_observations.find("
        f"{{entity_type:'relationship_type_suggestion',run_id:{json.dumps(run_id)}}},"
        "{entity_id:1,_id:0}).toArray();"
        "const ids=obs.map(x=>{try{return ObjectId(x.entity_id)}catch(e){return null}})"
        ".filter(Boolean); return db.relationship_type_suggestions.find("
        f"{{_id:{{$in:ids}},contact_id:{json.dumps(contact_id)}}},"
        "{suggested_value:1,status:1,confidence:1,source_run_ids:1,_id:1}).toArray(); })()"
    )
    return rows if isinstance(rows, list) else []


def projection_llm_logs(run_id: str) -> list[dict]:
    """Return only the child LLM call used by one exact post-decision projection."""
    if not run_id.strip():
        return []
    rows = mongo_json(
        "db.llm_call_logs.find("
        f"{{run_id:{json.dumps(run_id + ':projection')},prompt_key:'user.projection.task'}},"
        "{run_id:1,prompt_key:1,status:1,final_status:1,_id:0}).toArray()"
    )
    return rows if isinstance(rows, list) else []


def seed_citable_knowledge_chunk(source_name: str, account_id: str, title: str, body: str) -> str:
    """Insert a reviewable, citable biz-test fixture using the current chunk BSON contract.

    The row deliberately starts as ``draft + needs_review``. Callers must use
    :func:`verify_knowledge_chunk` to enter the production catalog; this helper cannot create a
    verified row and therefore cannot bypass the human verification lifecycle.
    """
    if not source_name.startswith(BIZ_PREFIX):
        raise ValueError(f"source_name must start with {BIZ_PREFIX}")
    values = {
        "source_name": source_name,
        "workspace_id": "default",
        "account_id": account_id,
        "domain": "user_operations",
        "knowledge_type": "product_fact",
        "title": title,
        "summary": body,
        "body": body,
        "source_quote": body,
        "integrity_status": "needs_review",
        "status": "draft",
        "priority": 100,
        "wiki_type": "entity",
        "chunk_type": "product_fact",
    }
    literal = json.dumps(values, ensure_ascii=False)
    quote = json.dumps(body, ensure_ascii=False)
    mongo(f'db.operation_knowledge_chunks.deleteMany({{source_name:{json.dumps(source_name)}}})')
    row = mongo_json(
        "(() => { const value=" + literal + ";"
        "value.source_anchors=[{sourceQuote:" + quote + ",sourceName:value.source_name}];"
        "value.created_at=new Date();value.updated_at=value.created_at;"
        "return db.operation_knowledge_chunks.insertOne(value).insertedId; })()"
    )
    if isinstance(row, dict):
        return str(row.get("$oid", ""))
    return str(row or "")


def get_knowledge_chunk(chunk_id: str) -> dict:
    """Read one chunk through the authenticated production API contract."""
    response = api("GET", f"/api/operation-knowledge/chunks/{chunk_id}", admin=True)
    item = response.get("item") if isinstance(response, dict) else None
    return item if isinstance(item, dict) else {}


def verify_knowledge_chunk(chunk_id: str) -> dict:
    """Human-verify one chunk using the server-issued optimistic concurrency token."""
    item = get_knowledge_chunk(chunk_id)
    expected = item.get("updatedAt")
    if not isinstance(expected, str) or not expected.strip():
        return {"_error": "missing_updatedAt", "item": item}
    return api(
        "POST",
        f"/api/operation-knowledge/chunks/{chunk_id}/verify",
        {"expectedUpdatedAt": expected, "verifiedClaims": []},
        admin=True,
    )


def patch_knowledge_chunk(chunk_id: str, patch: dict) -> dict:
    """Apply an editable content patch through the revision/lifecycle endpoint."""
    return api(
        "POST",
        f"/api/operation-knowledge/chunks/{chunk_id}/patch",
        {"patch": patch, "reason": "biz-test lifecycle verification"},
        admin=True,
    )


def active_outbox_count(wxid: str) -> int:
    """Count queued deliveries that could still reach a biz-test contact."""
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    value = mongo_json(
        "db.agent_send_outbox.countDocuments("
        f"{{contact_wxid:{json.dumps(wxid)},status:{{$in:[\"pending\",\"in_flight\"]}}}})"
    )
    return int(value) if isinstance(value, (int, float)) else -1


def manual_send_requires_outbox_poll(response: object) -> bool:
    """Whether a management send result can still produce an async Outbox terminal.

    Only durable acceptance states justify polling. Policy/review failures are already terminal
    and intentionally create no Outbox; unknown shapes fail closed instead of wasting minutes or
    accidentally encouraging a retry.
    """
    if not isinstance(response, dict):
        return False
    status = response.get("gatewayStatus", response.get("gateway_status"))
    return status in {"outbox_enqueued", "skipped_duplicate"}


def latest_outbox(wxid: str, limit: int = 5) -> list[dict]:
    """该 contact 最近 N 条 agent_send_outbox（media_asset_id/referral_card_id/content/status）。

    status 闭集仅 5 个：pending/in_flight/sent/failed_terminal/canceled。
    held_by_ai_policy/blocked_by_safety_guard 是 agent_run_logs.final_review_status 的值,不在这里。
    """
    rows = mongo_json(
        f'db.agent_send_outbox.find({{contact_wxid:"{wxid}"}},'
        '{media_asset_id:1,referral_card_id:1,content:1,status:1,_id:0})'
        '.sort({_id:-1}).limit(' + str(int(limit)) + ').toArray()'
    )
    return rows if isinstance(rows, list) else []


def outbox_for_run(wxid: str, run_id: str) -> list[dict]:
    """Return Outbox rows for one exact biz-test run, never contact-level history."""
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    if not run_id.strip():
        return []
    rows = mongo_json(
        "db.agent_send_outbox.find("
        f"{{contact_wxid:{json.dumps(wxid)},run_id:{json.dumps(run_id)}}},"
        "{run_id:1,media_asset_id:1,referral_card_id:1,content:1,status:1,_id:0})"
        ".sort({_id:1}).toArray()"
    )
    return rows if isinstance(rows, list) else []


def ptier_events_for_run(wxid: str, run_id: str) -> list[dict]:
    """Return progressive-tier events whose durable details.run_id matches this run."""
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    if not run_id.strip():
        return []
    rows = mongo_json(
        "db.agent_events.find("
        f"{{contact_wxid:{json.dumps(wxid)},kind:/^ptier_/,"
        f"\"details.run_id\":{json.dumps(run_id)}}},"
        "{kind:1,details:1,_id:0}).sort({_id:1}).toArray()"
    )
    return rows if isinstance(rows, list) else []


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
    """Require durable evidence that the exact prompt path reached a successful model result."""
    logs = llm_logs_since(seconds, prompt_key)
    ok = any(l.get("status") in ("success", "cache_hit") for l in logs)
    return expect(
        ok,
        domain,
        f"{prompt_key} 有 success/cache_hit 的 llm_call_logs",
        f"logs={logs}",
        "high",
        "LLM 未真调或失败(json_error/failed)，该域没有可接受的真实模型证据",
    )


def assert_llm_success_for_run(run_id: str, prompt_key: str, domain: str) -> bool:
    """Require one successful LLM ledger row bound to the exact durable run identity."""
    if not run_id.strip():
        return expect(False, domain, f"{prompt_key} 有非空 run identity", "run_id is empty", "high")
    logs = mongo_json(
        "db.llm_call_logs.find("
        f"{{run_id:{json.dumps(run_id)},prompt_key:{json.dumps(prompt_key)}}},"
        "{run_id:1,prompt_key:1,status:1,final_status:1,model:1,_id:0})"
        ".sort({_id:-1}).toArray()"
    )
    logs = logs if isinstance(logs, list) else []
    ok = any(row.get("status") in ("success", "cache_hit") for row in logs)
    return expect(
        ok,
        domain,
        f"{prompt_key} exact run 有 success/cache_hit 的 llm_call_logs",
        f"run_id={run_id} logs={logs}",
        "high",
        "该业务 run 没有成功模型证据，不能用同 prompt 的并发历史调用代替",
    )


_INIT = False


def record_blocked(domain: str, capability: str, evidence: str, requirement: str) -> None:
    """Persist an environment/capability block separately from business assertion failures."""
    BLOCKED_LEDGER.parent.mkdir(parents=True, exist_ok=True)
    row = {
        "domain": domain,
        "capability": capability,
        "evidence": evidence[:500],
        "requirement": requirement[:500],
        "recordedAtMs": int(time.time() * 1000),
    }
    with BLOCKED_LEDGER.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(f"[BLOCKED] {domain}: {capability}", flush=True)


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


class BizTestAssertionError(AssertionError):
    """A non-observational acceptance assertion failed."""


def expect(cond: bool, domain: str, desc: str, evidence: str, severity: str = "high", root: str = "") -> bool:
    """Record every assertion and fail the process for every non-``low`` failure.

    ``low`` is the sole observational level. Critical/high/medium and explicit BLOCKED findings
    are authoritative suite failures; allowing those scripts to return zero made earlier matrices
    false-green even when their own evidence contradicted acceptance.
    """
    if cond:
        print(f"[PASS] {domain}: {desc}", flush=True)
        return True
    record(domain, f"断言失败: {desc}", evidence, severity, root or "断言不成立，见证据")
    if severity.strip().lower() != "low":
        raise BizTestAssertionError(f"[{severity}] {domain}: {desc}; evidence={evidence[:500]}")
    return False


def is_api_error(resp: Any) -> str | None:
    """识别一次 API 调用是否是「失败/端点故障」而非正常业务响应。返回错误字符串或 None。

    后端 AppError 响应体是 `{"error": "<kind>"}`（src/error.rs:63，key 是 `error`），
    端点上游故障典型 kind = upstream_error / external_error / llm_*。`api`/`api_bg` 失败
    另有内部哨兵 `_error`（超时等）、`_raw`（无法解析的原始输出）。这些都不是业务结果，
    断言前必须先判出来——否则 AppError dict 被当正常 command 处理 → 假绿（域⑩ 2026-06-29
    实证：upstream_error 被 `"_error" not in c1` 判过，三断言自洽全挂还误报成业务 finding）。

    端点故障应标 BLOCKED 不假绿（[[feedback_biztest_fix_loop_no_overfitting]] 纪律），
    不是项目 bug 也不是业务断言失败。
    """
    if not isinstance(resp, dict):
        return f"non_dict_response:{str(resp)[:120]}"
    if "_error" in resp:
        return f"_error:{resp['_error']}"
    if "error" in resp:
        return f"api_error:{resp['error']}"
    if "_raw" in resp:
        return f"unparsed_raw:{str(resp['_raw'])[:120]}"
    return None


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

    **updated_at 放 $set 不放 $setOnInsert**：models.rs 的 Contact.updated_at 是非 optional
    DateTime（无 serde(default)），webhook 链路读 contact 时缺它会反序列化失败 → 502 db_error
    → inbound 落库但 runner 永不 spawn → run log 恒 0（域②起 run1=None 的真因，2026-06-26）。
    放 $set 每次刷新，能顺带补上历史残缺 contact（如已造的 biztest_c2）。

    **强制关 quiet hours**：默认作息门控 22:00-08:00（东八区，config defaults::quiet_hours_*）
    启用。测试常在服务器夜间跑，命中静默时段 → webhook 走 deferred 分支（queued:False
    deferred:True）不 spawn runner → run log 不来、send_and_wait 假阴。这里给每个测试
    contact 设 operation_mode_override.quiet_hours.enabled_override=false（contact 级覆盖，
    见 models.rs:2109 QuietHoursMode + planner::resolve_operation_mode），无论几点跑都立即应答。
    BSON 全 snake_case（OperationMode/QuietHoursMode 无 serde rename）。不碰全局配置/不改 src。
    """
    js = (
        f'db.contacts.updateOne('
        f'{{wxid:"{wxid}",account_id:"{account_id}"}},'
        f'{{$set:{{agent_status:"managed",nickname:"{nickname}",updated_at:new Date(),'
        f'"operation_mode_override.quiet_hours.enabled_override":false}},'
        f'$setOnInsert:{{workspace_id:"default",created_at:new Date()}}}},'
        f'{{upsert:true}})'
    )
    mongo(js)


def reset_contact_conversation(account_id: str, wxid: str) -> None:
    """清掉某测试 contact 的历史对话/run/outbox/events/决策评审 + 重置滚动记忆，保证干净起点。

    含 agent_decision_reviews（召回/反应/五维证据所在层）与 contact.memory_summary（短期滚动
    上下文）：二者不清则跨轮残留污染——AI 从 memory_summary 加载"已承诺核对口径""重复发同一
    问题"等历史→后续轮回"先核对再答"/no_reply 而不再引用知识直接答（实证 c2 阶段4 召回恢复
    假阴根因）。不碰 operating_memories（长期记忆/memory_card，各域如⑨自行 deleteMany 管理）。
    """
    if not wxid.startswith(BIZ_PREFIX):
        raise ValueError(f"biz-test wxid must start with {BIZ_PREFIX}")
    for c, field in [
        ("conversation_messages", "contact_wxid"),
        ("agent_run_logs", "contact_wxid"),
        ("agent_send_outbox", "contact_wxid"),
        ("agent_events", "contact_wxid"),
        ("agent_decision_reviews", "contact_wxid"),
    ]:
        mongo(f'db.{c}.deleteMany({{{field}:"{wxid}",account_id:"{account_id}"}})')
    # 测试重跑还必须撤销上一轮由确定性 stop 写入的持久屏障，否则新 webhook 会在
    # precheck 阶段直接命中 cooldown，永远测不到当前入站的 stop 汇合路径。这里只允许
    # biztest_ contact，并只清 stop 自己拥有的字段；其它运营策略保持不动。
    mongo(
        'db.contacts.updateOne('
        f'{{wxid:{json.dumps(wxid)},account_id:{json.dumps(account_id)}}},'
        '{$set:{memory_summary:null,last_outbound_style:null},'
        '$unset:{cooldown_until:"",'
        '"operation_policy.explicitStopRequested":"",'
        '"operation_policy.explicitStopRequestedAt":""}})'
    )
