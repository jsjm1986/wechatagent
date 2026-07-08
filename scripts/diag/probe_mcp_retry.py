#!/usr/bin/env python3
"""多次重试 contacts_fetch_cache 抓稳定形态 + 满数据时的完整结构。只读。117 本机跑。"""
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

BASE = env["MCP_BASE_URL"].rstrip("/")
KEY = env["MCP_API_KEY"]
URL = BASE + "/mcp"
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
        return resp.status, resp.headers.get("mcp-session-id"), resp.read().decode("utf-8", "replace")


status, sid, _ = post({"jsonrpc": "2.0", "id": "init", "method": "initialize",
                       "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "p", "version": "0.1"}}})
print(f"[init] {status}")

full = None
for i in range(6):
    payload = {"jsonrpc": "2.0", "id": f"c{i}", "method": "tools/call",
               "params": {"name": "contacts_fetch_cache", "arguments": {"account_alias": "t-1"}}}
    try:
        _, _, raw = post(payload, session=sid)
        sc = parse_body(raw).get("result", {}).get("structuredContent", {})
        keys = list(sc.keys())
        inner = sc.get("result", {}) if isinstance(sc, dict) else {}
        n = len(inner.get("friends", [])) if isinstance(inner, dict) else 0
        print(f"  try {i}: topKeys={keys} friends={n}")
        if n > 0 and full is None:
            full = sc
    except Exception as e:
        print(f"  try {i}: EXC {e!r}")
    time.sleep(2)

if full:
    inner = full["result"]
    print("\n=== 满数据完整结构 ===")
    print("result 下所有 keys:", list(inner.keys()))
    for k, v in inner.items():
        if isinstance(v, list):
            print(f"  {k}: list len={len(v)} 首类型={type(v[0]).__name__ if v else '?'} 样本={json.dumps(v[:2], ensure_ascii=False)[:200]}")
        else:
            print(f"  {k}: {type(v).__name__}={json.dumps(v, ensure_ascii=False)[:150]}")
else:
    print("\n6 次均未拿到满数据 —— cache 当前为空态")
