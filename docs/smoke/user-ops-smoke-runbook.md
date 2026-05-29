# user-ops 真实流量烟雾 runbook

> Phase 0→E 收口验证用：从 webhook 入口到 outbox 末端跑完一条真实链路，
> 抽样验证 `reaction_hint` / `operator_memory` / `negative_example` 三条新链路，
> 以及 outbox 5 状态闭环。仅 user-ops 私聊路径，不涉及 group/moments。
>
> 与 `knowledge-smoke-doc.md`（知识库导入冒烟）正交：那份验证 catalog/chunks
> 写入；本份验证 webhook → decision → review → outbox → MCP 全链路。

## 0. 前置

```text
- MongoDB（本地：mongodb://localhost:27017，库名 wechatagent）
- DeepSeek / OpenAI 协议 LLM key（OPENAI_API_KEY）
- MCP 网关 key（MCP_API_KEY）
- bash（项目根 cd 用，路径含中文，绝对路径优先）
- python（curl payload 编码用，参考 scripts/rt-send.sh）
```

`.env` 关键字段（默认值见 `.env.example`）：

```ini
APP_HOST=0.0.0.0
APP_PORT=8080
MONGODB_URI=mongodb://localhost:27017
MONGODB_DATABASE=wechatagent

MCP_BASE_URL=http://47.108.57.147:3001
MCP_API_KEY=<填真>
OPENAI_BASE_URL=https://api.deepseek.com
OPENAI_API_KEY=<填真>
OPENAI_MODEL=<填 LlmProviderConfigs 中 active 条目的 model 字段>

# P0 鉴权 / Webhook 签名
SESSION_TTL_HOURS=8
SESSION_COOKIE_SECURE=false       # 本地 http 调试；上线 true
BOOTSTRAP_ADMIN_USERNAME=admin
BOOTSTRAP_ADMIN_PASSWORD=<≥12 位>  # 启动一次后即可清空
WEBHOOK_VERIFY_SIGNATURE=true     # 联调期可临时 false 做 payload 调试，生产必须 true
```

## 1. 启动顺序

```bash
cd "E:/yw/agiatme/工作项目/wechatagent"
cargo run
```

启动期日志关注顺序（任何一步失败都不要继续往下走）：

1. `migrations::run` 完成
2. `ensure_indexes` 完成（其中包括
   `decision_reviews(workspace_id, account_id, contact_wxid, created_at:-1)`，
   是 `load_recent_reaction_hint` 的依赖）
3. `bootstrap_admin_if_needed`：若 `admin_users` 为空且 BOOTSTRAP_* 都已配，
   会种入第一个 admin。已有 admin 会幂等跳过。
4. `ensure_default_llm_provider` / `ensure_prompt_pack_v2`
5. `run_active_domain_state_machine_sanity_check`：active domain 必须有非空
   state machine，否则 fail-closed 启动失败（S1.2 hard rule）。
6. `tasks` worker spawn（`TASK_WORKER_INTERVAL_SECONDS=30`）
7. `Listening on 0.0.0.0:8080`

确认 admin 可登录：

```bash
curl -sS -X POST http://localhost:8080/api/auth/login \
  -H 'content-type: application/json' \
  -c /tmp/wa_cookie.txt \
  -d '{"username":"admin","password":"<刚配的>"}'
# 期望：{"username":"admin","expiresAt":"<rfc3339>"}
```

把目标联系人改成 managed（`/api/contacts/{id}/agent_status`，admin UI
"联系人 → 启用 AI 接管"按钮等价）。**未 managed 的 contact 入站只落
`conversation_messages`，不进 gateway，本 runbook 验证不到链路。**

## 2. 五条 webhook 真实流量

webhook 路径：`POST /webhooks/wechat`。

字段别名：webhook 同时支持小写驼峰（自测/手工脚本）与 GeWe 大写驼峰
（MCP 真实推送），见 `src/webhooks.rs:111-190`。本 runbook 用小写驼峰
出题，便于直接 `curl --data-binary` 不必造 envelope。

### 2.1 HMAC 签名

`WEBHOOK_VERIFY_SIGNATURE=true` 时必须带 header
`X-MCP-Signature: <hex(HMAC-SHA256(body, MCP_API_KEY))>`，body 与 raw
post body **逐字节一致**（含空白与字段顺序）。一次错就 400 invalid
signature。脚本（bash + python）：

```bash
sign() {
  local body="$1"
  python -c "
import hmac, hashlib, os, sys
key = os.environ['MCP_API_KEY'].encode()
print(hmac.new(key, sys.argv[1].encode('utf-8'), hashlib.sha256).hexdigest())
" "$body"
}

post() {
  local body="$1"
  local sig
  sig=$(sign "$body")
  curl -sS -X POST http://localhost:8080/webhooks/wechat \
    -H 'content-type: application/json; charset=utf-8' \
    -H "X-MCP-Signature: $sig" \
    --data-binary "$body" \
    --max-time 90
  echo
}

export MCP_API_KEY=<和 .env 里一致>
```

### 2.2 五条用例

把 `<APP>` 换成 `wechat_accounts.app_id`，`<WX>` 换成已经 managed 的
contact `wxid`。`newMsgId` 必须每条不同（webhook 用它做 dedupe key），
推荐 `r1-s1-1-<unix_nanos>` 风格。

| 编号 | 目的 | content | 期望 |
| --- | --- | --- | --- |
| W1 | 首问，激活决策 | 你好 想咨询下你们这个 | inbound 落库 + decision/review/outbox 全走 |
| W2 | 第二轮，验 reaction_hint 注入 | 价格能再优惠点吗 | run_log 含 reaction_hint 段（W1 的 reaction_analysis） |
| W3 | 第三轮，product 询问，验 grounding 闸 | 你们这个能解决我那个 XX 痛点吗 | 没 verified chunk → blocked_unverified_product_claim |
| W4 | testMsg 控制事件 | -（用 `{"testMsg":"ping"}`） | 直接 200 + `ignored:"callback_test"`，不进限流不进决策 |
| W5 | Offline 控制事件 | -（用 `{"TypeName":"Offline"}`） | 直接 200 + `ignored:"offline_event"` |

W1：

```bash
body=$(python -c '
import json
print(json.dumps({
  "appId":"<APP>", "fromWxid":"<WX>",
  "content":"你好 想咨询下你们这个",
  "newMsgId":"r1-s1-1-100"
}, ensure_ascii=False))')
post "$body"
# 期望：{"ok":true,"managed":true,"queued":true}
```

W2（间隔 ≥ AGENT_MIN_REPLY_INTERVAL_SECONDS=20s 后发，否则 gateway
拒发——这是设计内安全门，不是 bug）：

```bash
body=$(python -c '
import json
print(json.dumps({
  "appId":"<APP>", "fromWxid":"<WX>",
  "content":"价格能再优惠点吗",
  "newMsgId":"r1-s1-2-200"
}, ensure_ascii=False))')
post "$body"
```

W3（与 W2 间隔 ≥20s）：

```bash
body=$(python -c '
import json
print(json.dumps({
  "appId":"<APP>", "fromWxid":"<WX>",
  "content":"你们这个能解决我那个甲方临时改需求的痛点吗",
  "newMsgId":"r1-s1-3-300"
}, ensure_ascii=False))')
post "$body"
```

W4 / W5（控制事件 short-circuit，不进决策路径，无需 ≥20s 间隔）：

```bash
post '{"testMsg":"ping"}'
# {"ok":true,"ignored":"callback_test","echo":"ping"}

post '{"TypeName":"Offline"}'
# {"ok":true,"ignored":"offline_event","type":"Offline"}
```

## 3. 验收抽样（mongo shell）

`mongosh wechatagent`：

### 3.1 inbound 落库（W1-W3）

```js
db.conversation_messages.find(
  { contact_wxid: "<WX>", direction: "Inbound" },
  { dedupe_key:1, content:1, created_at:1 }
).sort({ created_at: -1 }).limit(5)
// 期望：dedupe_key 形如 "message:r1-s1-3-300" 三条，无重复
```

### 3.2 reaction_hint 链路（Phase A / A1）

W1 的 reaction_analysis 是 W2 决策时的 reaction_hint 输入。

```js
// 决策时回看的 reaction_analysis 必须有写入
db.decision_reviews.find(
  { contact_wxid: "<WX>", reaction_analysis: { $exists: true, $ne: null } },
  { reaction_analysis:1, reviewer_misjudge_signal:1, created_at:1 }
).sort({ created_at: -1 }).limit(3)

// W2 / W3 对应的 agent_run_logs 应包含 reaction_hint 注入痕迹（prompt
// 段或 promptBlocks，看具体 run_log 结构）
db.agent_run_logs.find(
  { contact_wxid: "<WX>" },
  { promptBlocks:1, prompt_segments:1, created_at:1 }
).sort({ created_at: -1 }).limit(3)
// 期望：W2 / W3 至少一条命中 "近期反馈" / "reaction_hint" 段
```

如果 W2 run_log 里看不到 reaction_hint 段，依次排查：

1. W1 的 `decision_reviews.reaction_analysis` 是否真写入？没写入说明
   `agent::record_user_reaction` 没跑（contact 不是 managed？或 spawn 任务报错见
   `agent_events.kind="agent_error"`）。
2. `decision_reviews` 索引存在性：`db.decision_reviews.getIndexes()` 应
   含 `(workspace_id, account_id, contact_wxid, created_at:-1)`。
3. `load_recent_reaction_hint` 是 best-effort，DB 错只 warn，不会上调用方。
   排查 `cargo run` 日志的 `load_recent_reaction_hint find failed`。

### 3.3 operator_memory 链路（Phase A / A2）

operator_memory 在 `decide_reply_with_promote` 装 prompt 时调
`load_operator_memory(contact_id, domain_id)`。要先有内容才能验注入：

```js
// 先用 admin 路径 / 直接 mongo 写一条 operator_memory
db.operator_memories.insertOne({
  workspace_id: "default",
  account_id: "default",
  contact_wxid: "<WX>",
  domain_id: "user_operations",
  body: "客户偏好简短回复，避免长段落；上次提过预算敏感",
  status: "active",
  created_at: new Date(),
  updated_at: new Date()
})
```

再发一轮（间隔 ≥20s，content 任意非控制事件），看 W2/W3 之后的
run_log prompt 段是否含 `format_operator_memory_for_reply_prompt`
渲染出的 "运营长期记忆" 段。

### 3.4 negative_example 链路（Phase C / C2）

触发条件 = reviewer 通过（`approved=true`）但 reaction_analysis
`outcomeStatus` 落到负反应桶（如 `user_blocked` / `user_complained` /
`user_explicit_stop`）。**自然流量很难当场凑出**——通常需要专门设计一条
让 reviewer 放过、用户立刻表态停的对话。本 runbook 用直接 mongo 注入
触发：

```js
// 1) 找一条最近 approved=true 的 decision_review
const dr = db.decision_reviews.findOne(
  { contact_wxid: "<WX>", "approved": true },
  { sort: { created_at: -1 } }
)
// 2) 直接补 reviewer_misjudge_signal=approved_but_user_negative
//    + outcome_status 显式负反应。注：生产路径上这两个字段是 reaction
//    收到时一并写的，本步只是手工模拟"reviewer 误判"信号。
db.decision_reviews.updateOne(
  { _id: dr._id },
  { $set: {
      reviewer_misjudge_signal: "approved_but_user_negative",
      outcome_status: "user_explicit_stop"
  }}
)
```

正确链路（推荐）：让真实 W2/W3 reply 让用户回 "别再发了 / 不要骚扰 /
拉黑你"，`reaction.rs::analyze_user_reaction` 会自动判定为
`user_explicit_stop` 类负反应，`compute_reviewer_misjudge_signal` 自然
落 `approved_but_user_negative`，然后 `enqueue_negative_example_chunk`
把 reply_text 写到 chunk review queue。

落库验证：

```js
db.operation_knowledge_chunks.find(
  {
    chunk_type: "negative_example",
    business_context: "reviewer_misjudge_feedback",
    integrity_status: "needs_review",
    "domain_attributes.source": "reviewer_misjudge"
  },
  { title:1, summary:1, body:1, "domain_attributes.source_review_id":1, status:1 }
).sort({ created_at: -1 }).limit(3)
// 期望：status="draft" 且 source_review_id 与上面 dr._id 一致；admin 在
// chunk review queue UI 里能看到，需人工 approve 后才会进 verified 池。
```

幂等：同一 `source_review_id` 重复触发不会写第二条。

### 3.5 outbox 5 状态闭环（W4 / R13）

```js
// 五状态枚举：pending / in_flight / sent / failed_terminal / canceled
// 严禁出现 "failed" / "queued"（dispatcher 会拒绝读，写入侧也已断言）
db.agent_send_outbox.find(
  { contact_wxid: "<WX>" },
  { status:1, attempt:1, idempotency_key:1, last_error:1, created_at:1, sent_at:1 }
).sort({ created_at: -1 }).limit(5)
```

期望轨迹：

- W1 / W2：`pending → in_flight → sent`
- W3：grounding 闸触发 `blocked_unverified_product_claim`，**不写 outbox**
  （没到 enqueue 阶段），`agent_run_logs.gateway_status` 落
  `held_by_ai_policy` 类（AI-internal status；商业语义不是托管移交，
  而是 AI 自己在等更多上下文 / 更多依据）。
- 若想触发 `canceled`：W2 之后立刻发一条 `不要再回我` 类入站，gateway
  侧的 `cancel_for_contact_on_user_reaction` 会把同 contact 还在
  `pending / in_flight` 的 entry 一并 cancel。

idempotency key 形如 `sha256(source_event_id:contact_wxid:content_hash)`
（`src/agent/outbox.rs:1900` 附近），同 webhook newMsgId 重发自然
`IdempotentSkip`，对照表 `db.agent_send_outbox.find(...)` 不会多一行。

## 4. 故障排查清单

| 现象 | 排查 |
| --- | --- |
| `401 Unauthorized` 调 `/api/*` | cookie 没带；admin 没种入；session TTL 过期。看 `admin_sessions.expires_at`，重发 `/api/auth/login`。`/health` 与 `/api/auth/login` 是白名单。 |
| `400 invalid signature` | body 多余空格 / charset 没声明 utf-8 / `MCP_API_KEY` 与 .env 不一致 / 路由经过反代被改写 body。临时 `WEBHOOK_VERIFY_SIGNATURE=false` 仅用于联调，回归生产前必须改回 true。 |
| `400 webhook appId ... not registered` | inbound `appId` 在 `wechat_accounts` 没记录。原先版本会静默回退 default account（导致 inbound 落错 account / managed 失效），P1 改成显式 400 + 写一条 `webhook_unknown_app_id` admin 事件。补 `wechat_accounts` 记录。 |
| `429 rate_limited` | per-account 滑窗超限（默认 60s/30 条）。`webhook_rate_limited` 事件按当日去重写一次，不会刷屏。调 `WEBHOOK_RATE_LIMIT_*` 或排查上游重发风暴。 |
| inbound 200 但 AI 不回 | contact `agent_status != "managed"`（最常见）；或 `min_reply_interval` 拦截；或 `RunBudget` 已耗尽；或 grounding 闸触发 `blocked_unverified_product_claim`。看 `agent_run_logs.gateway_status` + `agent_events`。 |
| `webhook_managed_contact_account_mismatch` 事件 | 同一 wxid 在另一个 account 下被标 managed，本次 inbound 落到非 managed 的影子 account。运营侧统一 wxid 归属 account。 |
| outbox 出现 `failed` / `queued` 字面量 | 立即视为 bug 上报（R13.5 / R13.10 hard rule，写入侧已 fail-closed，落到这两个值说明绕过了 enqueue 入口）。 |

## 5. 端到端验证 checklist

- [ ] `cargo run` 启动日志通过（migrations + ensure_indexes + admin bootstrap + state-machine sanity + worker spawn）
- [ ] `/api/auth/login` 200 并回 Set-Cookie wa_session（HttpOnly + SameSite=Strict）
- [ ] W1-W3 三条 webhook 200，managed=true / queued=true
- [ ] W4 testMsg / W5 Offline 控制事件 ack ignored，**不占限流**也**不写 conversation_messages**
- [ ] `decision_reviews.reaction_analysis` ≥ 2 条
- [ ] W2/W3 任一 `agent_run_logs` prompt 段含 reaction_hint
- [ ] 注入 operator_memory 后下一轮 prompt 段含 operator_memory 段
- [ ] 触发 reviewer_misjudge 后 `operation_knowledge_chunks` 出现
      `chunk_type=negative_example, integrity_status=needs_review` 一条，admin 路径可复核
- [ ] `agent_send_outbox` 三轮 pending→sent；status 不出现 "failed" / "queued"
- [ ] `scripts/check-baseline.sh` 通过（cargo test --lib ≥ 350 + 4 PBT 累计 ≥ 33 不回归，PBT 集合见脚本 `LIB_BASELINE` / `PBT_BASELINE`）
- [ ] `scripts/check-no-human-takeover.sh` 通过（新增字面量不命中禁词）
