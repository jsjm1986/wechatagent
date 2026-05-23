#!/usr/bin/env bash
# 用法：./rt-send.sh <slot> <content...>
# slot=唯一槽位，例如 r1-s1-1。content 支持中文。
set -euo pipefail
slot="${1:?slot required}"
shift
content="$*"
ts=$(date +%s%N)
payload=$(python -c "
import json,sys
print(json.dumps({
  'appId':'wx_wi_8NITtM8d0csT6tYDYX',
  'fromWxid':'fengrui86',
  'content':sys.argv[1],
  'newMsgId':sys.argv[2]
}, ensure_ascii=False))
" "$content" "$slot-$ts")
echo "$payload" > target/rt-payload.json
curl -sS -X POST http://localhost:8080/webhooks/wechat \
  -H 'content-type: application/json; charset=utf-8' \
  --data-binary @target/rt-payload.json \
  --max-time 90
echo
