#!/usr/bin/env python3
"""只读探测 gewe-multi-tenant MCP server：拉 tools/list 权威工具清单 + schema。
在 117 本机跑，直连 localhost:3001，用 .env 里的 workspace key。不改任何东西。"""
import json
import re
import sys
import urllib.request

# 从 /opt/wechatagent/.env 读 MCP_BASE_URL / MCP_API_KEY。
env = {}
with open("/opt/wechatagent/.env", encoding="utf-8") as f:
    for line in f:
        m = re.match(r"^([A-Z_]+)=(.*)$", line.strip())
        if m:
            env[m.group(1)] = m.group(2)

BASE = env["MCP_BASE_URL"].rstrip("/")
KEY = env["MCP_API_KEY"]
URL = BASE + "/mcp"
HEADERS = {
    "Authorization": f"Bearer {KEY}",
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream",
}


def parse_body(raw):
    """MCP Streamable-HTTP：SSE(data: {json}) 或纯 JSON。"""
    if "data:" in raw or "event:" in raw:
        data = "\n".join(
            ln[len("data:"):].lstrip()
            for ln in raw.splitlines()
            if ln.startswith("data:")
        )
        return json.loads(data)
    return json.loads(raw.strip())


def post(payload, session=None):
    body = json.dumps(payload).encode("utf-8")
    headers = dict(HEADERS)
    if session:
        headers["mcp-session-id"] = session
    req = urllib.request.Request(URL, data=body, headers=headers, method="POST")
    with urllib.request.urlopen(req, timeout=30) as resp:
        sid = resp.headers.get("mcp-session-id")
        raw = resp.read().decode("utf-8", errors="replace")
        return resp.status, sid, raw


# 1) initialize 握手拿 session。
init = {
    "jsonrpc": "2.0", "id": "init", "method": "initialize",
    "params": {"protocolVersion": "2024-11-05", "capabilities": {},
               "clientInfo": {"name": "probe", "version": "0.1"}},
}
status, sid, raw = post(init)
print(f"[initialize] HTTP {status} session={sid}")
print("  body:", raw[:200])

# 2) tools/list。
tl = {"jsonrpc": "2.0", "id": "tl", "method": "tools/list", "params": {}}
status, _, raw = post(tl, session=sid)
print(f"\n[tools/list] HTTP {status}")
msg = parse_body(raw)
tools = msg.get("result", {}).get("tools", [])
print(f"  共 {len(tools)} 个工具：\n")
for t in tools:
    name = t.get("name", "?")
    desc = (t.get("description") or "").replace("\n", " ")
    print(f"  ● {name}")
    print(f"      desc: {desc[:200]}")
    schema = t.get("inputSchema", {})
    props = schema.get("properties", {})
    required = schema.get("required", [])
    if props:
        print(f"      params: {list(props.keys())}  required={required}")
    print()
