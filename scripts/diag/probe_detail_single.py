#!/usr/bin/env python3
"""429 冷却后单发:只测 contact_get_detail(用已知真实 wxid)看昵称/头像字段。
串行、单次、不轮询,避免再撞限流。117 本机跑,只读。"""
import json
import re
import sys
import time
import urllib.request
import urllib.error

env = {}
with open("/opt/wechatagent/.env", encoding="utf-8") as f:
    for line in f:
        m = re.match(r"^([A-Z_]+)=(.*)$", line.strip())
        if m:
            env[m.group(1)] = m.group(2)

URL = env["MCP_BASE_URL"].rstrip("/") + "/mcp"
KEY = env["MCP_API_KEY"]
HEADERS = {"Authorization": f"Bearer {KEY}", "Content-Type": "application/json",
           "Accept": "application/json, text/event-stream"}
# 之前 contacts_fetch_cache 满返回里抓到的真实好友 wxid。
SAMPLE = "wxid_2o93p4cc9n4x22"


def parse_body(raw):
    if "data:" in raw or "event:" in raw:
        data = "\n".join(ln[len("data:"):].lstrip() for ln in raw.splitlines() if ln.startswith("data:"))
        return json.loads(data)
    return json.loads(raw.strip())


def post(payload, session=None):
    body = json.dumps(payload).encode("utf-8")
    headers = dict(HEADERS)
    if session:
        headers["mcp-session-id"] = session
    req = urllib.request.Request(URL, data=body, headers=headers, method="POST")
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.headers.get("mcp-session-id"), resp.read().decode("utf-8", "replace")


def safe_call(sid, tool, args):
    args = dict(args)
    args.setdefault("account_alias", "t-1")
    try:
        _, raw = post({"jsonrpc": "2.0", "id": tool, "method": "tools/call",
                       "params": {"name": tool, "arguments": args}}, session=sid)
        return parse_body(raw).get("result", {})
    except urllib.error.HTTPError as e:
        return {"_httperror": e.code}
    except Exception as e:
        return {"_exc": repr(e)}


try:
    sid, _ = post({"jsonrpc": "2.0", "id": "i", "method": "initialize",
                  "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "p", "version": "0.1"}}})
except urllib.error.HTTPError as e:
    print(f"initialize HTTP {e.code}（仍在限流,稍后重试）")
    sys.exit(0)

print(f"[init] ok, 测 contact_get_detail(contact={SAMPLE})\n")
d = safe_call(sid, "contact_get_detail", {"contact": SAMPLE})
print("=== contact_get_detail 完整 result ===")
print(json.dumps(d, ensure_ascii=False, indent=2)[:2000])
