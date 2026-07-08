#!/usr/bin/env python3
"""耐心轮询 contacts_fetch_cache 直到就绪，命中后立刻测 contact_get_detail +
contacts_search_remote，摸清能否拿昵称/头像。只读。117 本机跑。"""
import json
import re
import time
import urllib.request

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


sid, _ = post({"jsonrpc": "2.0", "id": "i", "method": "initialize",
              "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "p", "version": "0.1"}}})


def call(tool, args):
    args = dict(args)
    args.setdefault("account_alias", "t-1")
    _, raw = post({"jsonrpc": "2.0", "id": tool, "method": "tools/call",
                   "params": {"name": tool, "arguments": args}}, session=sid)
    return parse_body(raw).get("result", {})


friends = []
for i in range(15):
    sc = call("contacts_fetch_cache", {}).get("structuredContent", {})
    inner = sc.get("result", {}) if isinstance(sc, dict) else {}
    friends = inner.get("friends", []) if isinstance(inner, dict) else []
    print(f"poll {i}: friends={len(friends)}", flush=True)
    if friends:
        break
    time.sleep(4)

if not friends:
    print("cache 始终空，无法测详情")
    raise SystemExit(0)

sample = next((w for w in friends if str(w).startswith("wxid_")), friends[0])
print(f"\n样本 wxid={sample}")

print("\n=== contact_get_detail ===")
d = call("contact_get_detail", {"contact": sample}).get("structuredContent", {})
print(json.dumps(d, ensure_ascii=False)[:1500])

print("\n=== contacts_search_remote (contacts_info=样本) ===")
try:
    r = call("contacts_search_remote", {"contacts_info": sample}).get("structuredContent", {})
    print(json.dumps(r, ensure_ascii=False)[:1200])
except Exception as e:
    print("EXC", repr(e))
