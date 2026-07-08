#!/usr/bin/env python3
"""只读探测：确认 contacts_fetch_cache 的完整结构 + contact_get_detail 返回形态。
决定 roster 能否带昵称/头像。在 117 本机跑。不改任何东西。"""
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


status, sid, _ = post({
    "jsonrpc": "2.0", "id": "init", "method": "initialize",
    "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "p", "version": "0.1"}},
})
print(f"[init] {status} {sid}")


def call(tool, args):
    args = dict(args)
    args.setdefault("account_alias", ALIAS)
    payload = {"jsonrpc": "2.0", "id": tool, "method": "tools/call", "params": {"name": tool, "arguments": args}}
    status, _, raw = post(payload, session=sid)
    return parse_body(raw).get("result", {})


# 1) contacts_fetch_cache 完整结构分析。
r = call("contacts_fetch_cache", {})
sc = r.get("structuredContent", {})
print("\n=== contacts_fetch_cache structuredContent 顶层 keys ===")
print(list(sc.keys()))
inner = sc.get("result", {})
if isinstance(inner, dict):
    print("result 下 keys:", list(inner.keys()))
    for k, v in inner.items():
        if isinstance(v, list):
            print(f"  {k}: 数组 len={len(v)}, 首元素类型={type(v[0]).__name__ if v else 'empty'}, 样本={json.dumps(v[:3], ensure_ascii=False)}")
        else:
            print(f"  {k}: {type(v).__name__} = {json.dumps(v, ensure_ascii=False)[:120]}")

# 拿一个真实好友 wxid（跳过系统号）测详情。
friends = inner.get("friends", []) if isinstance(inner, dict) else []
sample_wxid = next((w for w in friends if str(w).startswith("wxid_")), friends[0] if friends else None)
print(f"\n=== 用样本 wxid={sample_wxid} 测 contact_get_detail ===")
if sample_wxid:
    d = call("contact_get_detail", {"contact": sample_wxid})
    dsc = d.get("structuredContent", {})
    print("detail structuredContent:", json.dumps(dsc, ensure_ascii=False)[:1200])
