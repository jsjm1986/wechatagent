#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
真实测试投递工具（避开 Windows GBK 把中文打坏）。

用法 (windows bash)：
    python scripts/rt_send.py r1-s1-1 "你好，最近在忙什么"
脚本：
- 把内容用 utf-8 写到 target/rt-payload.json
- curl POST /webhooks/wechat
- 打印响应

stdin 也支持： python scripts/rt_send.py r1-s2-1 - <<<"绝对承诺..."
"""
import json
import os
import subprocess
import sys
import time

APP_ID = "wx_wi_8NITtM8d0csT6tYDYX"
FROM_WXID = "fengrui86"
ENDPOINT = "http://localhost:8080/webhooks/wechat"

if len(sys.argv) < 3:
    print("usage: rt_send.py <slot> <content|->", file=sys.stderr)
    sys.exit(2)

slot = sys.argv[1]
arg = sys.argv[2]

if arg == "-":
    # stdin 已经是 utf-8 字节
    content = sys.stdin.buffer.read().decode("utf-8").rstrip("\r\n")
else:
    # argv 在 win-cmd 通常是 GBK，先按 sys.getfilesystemencoding() 解
    raw = arg
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8", errors="replace")
    # 这里 raw 已经是 str；如果它原本是 GBK 字节被错误当成 UTF-8 解，
    # python 在 Windows 通常会用 mbcs/GBK 自己解码，所以保留 raw 本身即可。
    content = raw

new_msg_id = f"{slot}-{int(time.time()*1000)}"
payload = {
    "appId": APP_ID,
    "fromWxid": FROM_WXID,
    "content": content,
    "newMsgId": new_msg_id,
}

target = os.path.join("target", "rt-payload.json")
with open(target, "w", encoding="utf-8") as f:
    json.dump(payload, f, ensure_ascii=False)

cmd = [
    "curl", "-sS", "-X", "POST", ENDPOINT,
    "-H", "content-type: application/json; charset=utf-8",
    "--data-binary", f"@{target}",
    "--max-time", "180",
]
print(f">>> {new_msg_id} :: {content}")
print(subprocess.check_output(cmd).decode("utf-8", errors="replace"))
