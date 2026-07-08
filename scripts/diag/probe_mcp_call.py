#!/usr/bin/env python3
"""只读实调 gewe MCP 联系人类工具，打印完整未剥壳 JSON-RPC 返回。
在 117 本机跑。带 account_alias=t-1（workspace key 需要）。不改任何东西。"""
import json
import re
import urllib.request

env = {}
with open("/opt/wechatagent/.env", encoding="utf-8") as f:
    for line in f:
        m = re.match(r"^([A-Z_]+)=(.*)$", line.strip())
        if m:
            env[m.group(1)] = m.group(2)

BASE = env["MCP_BASE_URL"].rstrip("/")
KEY = env["MCP_API_KEY"]
URL = BASE + "/mcp"
ALIAS = "t-1"
HEADERS = {
    "Authorization": f"Bearer {KEY}",
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream",
}


def parse_body(raw):
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
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.status, resp.headers.get("mcp-session-id"), resp.read().decode("utf-8", "replace")


# initialize
status, sid, _ = post({
    "jsonrpc": "2.0", "id": "init", "method": "initialize",
    "params": {"protocolVersion": "2024-11-05", "capabilities": {},
               "clientInfo": {"name": "probe", "version": "0.1"}},
})
print(f"[init] HTTP {status} session={sid}\n")


def call(tool, args):
    args = dict(args)
    args.setdefault("account_alias", ALIAS)
    payload = {"jsonrpc": "2.0", "id": tool, "method": "tools/call",
               "params": {"name": tool, "arguments": args}}
    print(f"========== tools/call {tool} args={json.dumps(args, ensure_ascii=False)} ==========")
    try:
        status, _, raw = post(payload, session=sid)
        msg = parse_body(raw)
        result = msg.get("result", msg)
        # 完整打印 result 的 keys 与各部分。
        if isinstance(result, dict):
            print("  result keys:", list(result.keys()))
            sc = result.get("structuredContent")
            print("  structuredContent:", json.dumps(sc, ensure_ascii=False)[:1500])
            content = result.get("content")
            if content:
                for i, c in enumerate(content):
                    txt = c.get("text") if isinstance(c, dict) else str(c)
                    print(f"  content[{i}].text:", (txt or "")[:1500])
            print("  isError:", result.get("isError"))
        else:
            print("  result:", json.dumps(result, ensure_ascii=False)[:1500])
        if "error" in msg:
            print("  JSON-RPC error:", json.dumps(msg["error"], ensure_ascii=False)[:800])
    except Exception as e:
        print("  EXCEPTION:", repr(e))
    print()


call("contacts_fetch_cache", {})
call("contacts_search", {"query": "Demi"})
call("account_get_status", {})
