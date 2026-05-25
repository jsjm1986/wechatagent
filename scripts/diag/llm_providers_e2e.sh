#!/usr/bin/env bash
# LLM provider admin endpoints E2E：list / create / update / activate / test / delete
# 用法：BASE=http://localhost:8080 bash scripts/diag/llm_providers_e2e.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
PID="e2e-test-$(date +%s)"
PASS=0
FAIL=0
log() { echo -e "[$1] $2"; }
hit() {
  local name="$1" expect="$2" got="$3"
  if [[ "$got" == "$expect" ]]; then
    PASS=$((PASS+1)); log OK "$name (HTTP $got)"
  else
    FAIL=$((FAIL+1)); log FAIL "$name expect=$expect got=$got"
  fi
}

echo "=== BASE=$BASE PID=$PID ==="

# 1) list
code=$(curl -sS -o /tmp/lp.list.json -w "%{http_code}" "$BASE/api/admin/llm-providers")
hit "list providers" 200 "$code"

# 2) create
code=$(curl -sS -o /tmp/lp.create.json -w "%{http_code}" -X POST "$BASE/api/admin/llm-providers" \
  -H 'content-type: application/json' \
  -d "{\"providerId\":\"$PID\",\"name\":\"E2E Test\",\"format\":\"openai\",\"baseUrl\":\"https://api.example.invalid/v1\",\"apiKey\":\"sk-real-secret-abc1234567\",\"model\":\"gpt-4o-mini\"}")
hit "create" 200 "$code"
masked=$(grep -oE '"apiKeyMasked":"[^"]*"' /tmp/lp.create.json || true)
if [[ "$masked" == *'****'* ]]; then PASS=$((PASS+1)); log OK "create masks api_key ($masked)"; else FAIL=$((FAIL+1)); log FAIL "create did not mask api_key: $masked"; fi

# 3) update with masked apiKey → must keep old
code=$(curl -sS -o /tmp/lp.update.json -w "%{http_code}" -X PUT "$BASE/api/admin/llm-providers/$PID" \
  -H 'content-type: application/json' \
  -d "{\"providerId\":\"$PID\",\"name\":\"E2E Renamed\",\"format\":\"openai\",\"baseUrl\":\"https://api.example.invalid/v1\",\"apiKey\":\"sk-****1234\",\"model\":\"gpt-4o-mini\"}")
hit "update with masked key" 200 "$code"

# 4) update with real new key
code=$(curl -sS -o /tmp/lp.update2.json -w "%{http_code}" -X PUT "$BASE/api/admin/llm-providers/$PID" \
  -H 'content-type: application/json' \
  -d "{\"providerId\":\"$PID\",\"name\":\"E2E Renamed\",\"format\":\"anthropic\",\"baseUrl\":\"https://api.anthropic.invalid\",\"apiKey\":\"sk-ant-new-key-xyz9876\",\"model\":\"claude-haiku-4-5\"}")
hit "update with real key + format switch" 200 "$code"

# 5) activate
code=$(curl -sS -o /tmp/lp.act.json -w "%{http_code}" -X POST "$BASE/api/admin/llm-providers/$PID/activate")
hit "activate" 200 "$code"

# 6) activate idempotent (再激活同一条)
code=$(curl -sS -o /tmp/lp.act2.json -w "%{http_code}" -X POST "$BASE/api/admin/llm-providers/$PID/activate")
hit "activate idempotent" 200 "$code"

# 7) delete active → expect 400
code=$(curl -sS -o /tmp/lp.del_active.json -w "%{http_code}" -X DELETE "$BASE/api/admin/llm-providers/$PID")
hit "delete active blocked" 400 "$code"

# 8) test connection by providerId（即便 provider 不可达，也应返回 200 + ok=false）
code=$(curl -sS -o /tmp/lp.test.json -w "%{http_code}" -X POST "$BASE/api/admin/llm-providers/test" \
  -H 'content-type: application/json' \
  -d "{\"providerId\":\"$PID\"}")
hit "test by providerId" 200 "$code"

# 9) test inline OpenAI format（缺 api_key 应 400）
code=$(curl -sS -o /tmp/lp.test_bad.json -w "%{http_code}" -X POST "$BASE/api/admin/llm-providers/test" \
  -H 'content-type: application/json' \
  -d '{"format":"openai","baseUrl":"https://x.invalid","model":"foo"}')
hit "test inline missing apiKey → 400" 400 "$code"

# 10) cleanup：先激活别的（如果有），再删；否则跳过 delete
# 找一个非当前的 provider 切过去，避免 delete-active 限制
other=$(grep -oE '"providerId":"[^"]+"' /tmp/lp.list.json | grep -v "$PID" | head -1 | sed -E 's/.*"providerId":"([^"]+)".*/\1/')
if [[ -n "$other" ]]; then
  curl -sS -o /tmp/lp.swap.json -X POST "$BASE/api/admin/llm-providers/$other/activate" >/dev/null
  code=$(curl -sS -o /tmp/lp.del.json -w "%{http_code}" -X DELETE "$BASE/api/admin/llm-providers/$PID")
  hit "delete after swap" 200 "$code"
else
  log SKIP "no other provider to swap → leaving $PID active+exists"
fi

echo "=== summary: PASS=$PASS FAIL=$FAIL ==="
[[ $FAIL -eq 0 ]] || exit 1
