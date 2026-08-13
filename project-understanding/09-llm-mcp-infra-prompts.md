# LLM/MCP/配置/prompts/基础设施 深读记录（核证日期 2026-08-13）

> 覆盖范围：`src/llm.rs`、`src/llm_concurrency.rs`、`src/mcp.rs`、`src/config.rs`、`src/prompts.rs`、`src/prompt_guard.rs`、`src/prompt_template_versions.rs`、`src/soul_versions.rs`、`src/supervisor.rs`、`src/secret.rs`、`src/error.rs`、`src/media_storage.rs`、`src/outbound_fetch.rs`，共 12,898 行，全部逐行读完（含测试 mod）。所有断言附 `file:line`，跨文件引用另行亲验（`src/agent/mod.rs`、`src/db/config_generation.rs`、`src/agent/outbox_dispatcher.rs` 相关片段）。

---

## 1. 模块地图

```
                        ┌────────────────────────────────────────────┐
                        │ config.rs: AppConfig（唯一 env 入口，~120 项）│
                        └────────────┬───────────────────────────────┘
                                     │ 启动注入
     ┌───────────────────────────────┼──────────────────────────────────┐
     ▼                               ▼                                  ▼
┌──────────────┐   ┌─────────────────────────────┐        ┌─────────────────────┐
│ llm.rs       │   │ mcp.rs                      │        │ prompts.rs          │
│ LlmClient    │   │ McpClient（微信通道 JSON-RPC）│        │ 全部 prompt/soul 种子│
│ (openai/     │   │ session 缓存/roster/送达核对 │        │ + 状态机 + playbook  │
│  anthropic)  │   └───────────┬─────────────────┘        └─────┬───────────────┘
│ LlmRegistry  │               │ logged_call*→mcp_logs           │ ensure/align/reset
│ (热切换+同步) │               ▼                                 ▼
└──────┬───────┘   agent/outbox_dispatcher（150s 外层 timeout）  ┌──────────────────────────┐
       │ 由 generate_agent_json 调用                            │ prompt_template_versions │
       ▼                                                       │ soul_versions（append-only│
┌──────────────────┐                                           │ + 事务发布指针）           │
│ llm_concurrency  │◄─ acquire(priority_for_prompt(key))       └──────────┬───────────────┘
│ 双信号量准入      │                                                      │ 写回校验
└──────────────────┘                                           ┌──────────▼───────────────┐
                                                               │ prompt_guard.rs           │
支撑件：                                                        │ 三层分级+双闸+LLM 语义审查 │
  supervisor.rs   —— 16 个长驻 worker 的 panic 熔断状态机        └──────────────────────────┘
  error.rs        —— AppError 全集 + HTTP 映射（泄漏防护）
  secret.rs       —— mask_secret 脱敏
  media_storage.rs—— 素材内容寻址存储 + pending 两段发布 + reconciler
  outbound_fetch.rs— ingest 出站抓取 SSRF 防护（DNS pin + 公网白名单）
```

依赖方向：`agent/*`（业务脑）→ `generate_agent_json`（`src/agent/mod.rs:254`）→ `llm_concurrency` + `LlmRegistry`（llm.rs）；`agent/gateway`/`outbox_dispatcher` → `mcp.rs`；启动期 `main.rs` → `prompts::ensure_prompt_pack_v2` → 版本模块；管理编辑/evolution 写 prompt → `prompt_guard`。

---

## 2. 逐文件深读

### 2.1 `src/llm.rs`（3388 行）——双协议 LLM 客户端 + JSON 容错栈 + Registry

#### 协议形态（两种）
- `LlmFormat` 枚举：`Openai` / `Anthropic`（`src/llm.rs:25-29`）。`parse` 接受中性别名：`"chat"|"openai"|""`→Openai，`"messages"|"anthropic"|"claude"`→Anthropic，其它报 BadRequest（`src/llm.rs:39-50`）。对外中性协议名 `as_protocol()` 返回 `"chat"/"messages"`，`as_str()` 保留历史品牌值 `"openai"/"anthropic"` 以兼容存量库（`src/llm.rs:32-59`）。
- **OpenAI 路径**：`POST {base_url}/chat/completions`，`bearer_auth`，body=`{model, temperature, messages:[{system},{user}]}`，可选 `max_tokens`（`src/llm.rs:564-590`）。解析 `choices[0].message.content` + `usage`。**兼容强制 SSE 网关**：即使请求未带 stream，部分中转网关也回 `data: {chunk}` 帧——`is_openai_sse_body`（首个非空行以 `data:` 开头，`src/llm.rs:1409-1414`）检测后走 `aggregate_openai_sse` 聚合 delta.content/usage/finish_reason（`src/llm.rs:1418-1454`）。
- **Anthropic 路径**：`POST {base_url}/v1/messages`，header `x-api-key` + `anthropic-version: 2023-06-01`，body=`{model, max_tokens, temperature, system, messages:[{user}]}`（`src/llm.rs:741-778`）。max_tokens = `max_output_tokens.unwrap_or(8192).clamp(1,8192)`（`src/llm.rs:760`）。解析 `content[]` 中首个 `type=="text"` block（`src/llm.rs:814-824`）。
- **ANTHROPIC_JSON_GUARD**（`src/llm.rs:758`）：Anthropic 请求会在 system 末尾强制追加一段"[OUTPUT FORMAT — STRICT] 当前是对话生成模式…禁止调用任何工具（不要 WebFetch…）…第一个字符必须是 `{`"——针对 claude 实测的"入戏写共情散文"与"tool_use 劫持"两种失效。
- **tool_use 劫持检测**：`detect_tool_use_hijack`（`src/llm.rs:1487-1507`）——`stop_reason=="tool_use"` 或 content 含 tool_use block 即返回 `llm_tool_use_instead_of_json: ...` 诊断（带 text 开场白前 120 字符）；该错误**列入可重试白名单**（`src/llm.rs:1349`，注释称 ~25% 长任务高发、同输入重跑通常成功）。
- **截断防护**：OpenAI `finish_reason=="length"` → `reject_truncated_output` 抛 `llm_output_truncated`（`src/llm.rs:1456-1469`）；Anthropic `stop_reason=="max_tokens"` 同样抛（`src/llm.rs:808-813`）。截断错误在 repair 循环中直接上抛不再重复修复（`src/llm.rs:421-423`）。
- **`<think>` 前缀剥离**：`strip_reasoning_prefix`（`src/llm.rs:1473-1481`）仅当 `<think>...</think>` 闭合时剥掉前缀；未闭合原样返回。
- **DeepSeek fast-json 控制**：`apply_fast_json_controls`（`src/llm.rs:371-383`）——仅当（有 output limit）且 format==Openai 且 model 以 `deepseek` 开头（大小写不敏感）时注入 `response_format={type:json_object}` + `thinking={type:disabled}`；普通/流式调用不注入（测试 `src/llm.rs:3140-3206` 锁定 provider-scoped + bounded-only）。

#### reqwest 客户端构造（抗中转网关工程化，`src/llm.rs:326-354`）
- `timeout(timeout_seconds)`；`user_agent = "Mozilla/5.0 wechatagent-llm/1.0"`（`src/llm.rs:15`，部分兼容网关拒默认 reqwest UA）；
- `tcp_keepalive(15s)`——防 NAT/防火墙 idle 杀 chunked 流（`src/llm.rs:338`）；
- `pool_max_idle_per_host(0)`——关连接池，每请求新拨号（实测 DeepSeek 复用 TCP 偶发 60s 截断，`src/llm.rs:343`）；
- `http1_only()`——实测 HTTP/2+rustls 过 DeepSeek 偶发 stream stall（`src/llm.rs:349`）；
- `no_proxy()`——防 Windows WinHTTP 自动代理把请求引到 VPN 内部地址 404（`src/llm.rs:353`）。
- `max_retries.max(1)`、`retry_base_ms.max(100)`（`src/llm.rs:355-356`）；温度默认 0.2，`with_temperature` 链式覆盖（roleplayer 测试用 ~0.8），修复路径固定 0.0（`src/llm.rs:71-74,361-366`）。

#### JSON 解析：三层确定性 + 第三层回喂修复（宣称"三层"实为 3+1 结构）
`parse_json_content`（`src/llm.rs:1524-1556`）依次：
1. **围栏剥离 + 快路径**：剥 ```` ```json ```` 围栏后 `serde_json::from_str` 直接解析（`src/llm.rs:1526-1538`）；
2. **`repair_loose_json`**（`src/llm.rs:1633-1729`）：只做保守修复——① 删 trailing comma（`,]`/`,}`，peek 下一个非空白字符判定）；② 按嵌套深度补末尾未闭合 `]`/`}`；③ 字符串内**裸控制字符**转义（`\n`→`\\n`、`\r`、`\t`、`\b`、`\f`、其它 <0x20 用 `\uXXXX`，`src/llm.rs:1656-1668`）。**明确不做**单引号→双引号、未引号 key、注释剥离（会吞掉真错误，`src/llm.rs:1627-1632`）。无修改返回 None；
3. **`extract_embedded_json`**（`src/llm.rs:1563-1590`）：针对 claude "推理散文+JSON" 混合输出。遍历每个 `{`/`[` 起点，`balanced_block` 括号配平截块（跳过字符串字面量与转义，`src/llm.rs:1594-1625`），逐块严格解析→失败再 repair。**对象优先**：命中对象立即返回；数组暂存为 fallback；`[{...}]` 单对象数组拆出内部对象（对齐 `normalize_singleton_array`，`src/llm.rs:1513-1522`——generate_json 契约是对象，claude 偶发包成单元素数组）。
4. 全部失败：抛严格 serde 错误，绝不吞噪声（`src/llm.rs:1552-1555`）。

`parse_or_repair_bounded`（`src/llm.rs:399-435`）在上述之上加**第三层回喂修复**：
- 前置短路：`account_unavailable_reason`（`src/llm.rs:1285-1303`）检测 HTTP-200-但-正文是"余额不足/欠费/密钥已过期/insufficient balance/payment required/arrearage"类中转网关账户错误 → 直接抛 `llm_account_unavailable: {reason}`，不回喂（回喂只会重复计费，`src/llm.rs:408-415`）；
- 回喂 `repair_via_llm`（`src/llm.rs:439-449`）：固定中文"JSON 修复器"system（保持字段取值不变只修格式），温度 0.0 走 `fetch_raw_text`（`src/llm.rs:453-541`，独立 HTTP 链路避免递归），响应只过 `parse_json_content`（断递归）；最多 `REPAIR_MAX_ATTEMPTS=2` 次（`src/llm.rs:290`）；
- 命中 `llm_output_truncated` 立即上抛（截断确定性失败，不再修，`src/llm.rs:421-423`）；
- 2 次全失败：抛 `json_decode after 2 llm-repair attempts failed: ...; raw_head=前200字符; repair#N=...` 全诊断错误（`src/llm.rs:427-434`）。
- 测试锚：可解析输入不发网络（用 127.0.0.1:1 必拒连客户端验证，`src/llm.rs:2848-2886`）；噪声最终 Err 且带 `json_decode`+`raw_head`。

#### 重试判定全表
`is_retryable_llm_error`（`src/llm.rs:1327-1353`）——**可重试**：
| 错误 | 依据 |
|---|---|
| `AppError::Http` 且 `is_timeout()` 或 `is_connect()` | `src/llm.rs:1329` |
| `LLM HTTP 429` | `src/llm.rs:1332` |
| `LLM HTTP 500/502/503/504` | `src/llm.rs:1333-1336` |
| `LLM HTTP 520/522/524`（Cloudflare 源站层） | `src/llm.rs:1341-1343`（注释：曾漏列 524 致一条 CF 524 不重试直接冒泡） |
| `LLM HTTP body_decode_error`（chunked 流中断包装） | `src/llm.rs:1344` |
| `llm_tool_use_instead_of_json`（tool_use 劫持） | `src/llm.rs:1349` |
| 瞬时模型路由 404：`is_transient_model_routing_error`——须同时含 `llm http 404` + `model_not_found` + `not supported by any configured account in this group` | `src/llm.rs:1316-1321`；普通 404 或证据不全 fail-closed 不重试（测试 `src/llm.rs:2586-2613`） |

**不可重试**：`AppError::Json`（模型确定性吐非 JSON，重试只烧 token，`src/llm.rs:1323-1326`）、HTTP 400/401/402/403、其它。

#### 退避与 Retry-After
- `compute_backoff(attempt, base_ms, retry_after_secs)`（`src/llm.rs:1356-1371`）：`base*2^(min(attempt-1,10)) + jitter(0..base)`，**单次封顶 60s**（`MAX_BACKOFF_MS=60_000`）；Retry-After 更长则取它（不受 60s 顶约束，`src/llm.rs:1366-1369`）。测试态 jitter=0（`src/llm.rs:1382-1386`）。测试锚：base=2500 attempt=6 → 封顶 60s；attempt=5 → 40s；Retry-After 120s 胜出（`src/llm.rs:2674-2686`）。
- Retry-After 来源：响应头 `parse_retry_after`（`src/llm.rs:1388-1393`）→ 失败时编码进错误串 `[retry_after_secs=N]`（`src/llm.rs:610-616`）→ 重试循环 `parse_retry_after_from_error` 解出（`src/llm.rs:1395-1405`）。
- 主重试循环 `generate_json_with_usage_bounded`（`src/llm.rs:842-878`）：`for attempt in 1..=max_retries`；成功时 `retry_count` 回填 + `latency_ms` = 含失败尝试与退避的全程耗时（`src/llm.rs:856-861`）；不可重试或耗尽 → `classify_llm_error_for_user`。
- vision 同款重试循环（`src/llm.rs:1059-1087`，此前单次调用致 CI 假绿）。

#### 错误分类（面向前端）
`classify_llm_error_for_user`（`src/llm.rs:1106-1192`）把终态错误折成 `AppError::LlmUnavailable{kind, retry_count, detail, hint}`，kind 全集：`timeout` / `connect_failed` / `body_decode_error` / `network_error` / `account_unavailable`（余额/凭据，含 http_account_unavailable_reason 对 401/402/欠费文案的识别，`src/llm.rs:1252-1283`）/ `model_routing_unavailable` / `rate_limited` / `http_5xx` / `endpoint_not_found`（404，hint 教用户填 OpenAI 兼容 base_url 原文，附阿里云/DeepSeek 示例）/ `http_4xx` / `empty_response` / `json_decode_error` / `external_error` / `unknown`。每个 kind 带中文 hint。`is_transient_llm_unavailable_kind`（`src/llm.rs:1196-1207`）白名单 7 种瞬时 kind（rate_limited/model_routing_unavailable/http_5xx/timeout/connect_failed/network_error/body_decode_error），真实模型测试仅允许跳过这些，未知默认 false（fail closed）。`llm_account_unavailable_reason`（`src/llm.rs:1218-1250`）返回稳定 reason（`insufficient_balance`/`invalid_credential`/`account_unavailable`）供持久化/通知；5xx 即使 body 带凭据字样也不算账户错误（测试 `src/llm.rs:2957-2963`）。

#### usage 语义
`ChatUsage`（`src/llm.rs:167-202`）：`usage_known` 区分"上游明确报 0"与"上游没报"——中转网关常整体省略 usage，若当 0 会污染成本/预算遥测。`reported()` 置位；`is_known()` = usage_known 或任一字段非 0。Anthropic 的 `input_tokens/output_tokens` 转换时恒 `usage_known=true`（`src/llm.rs:155-165`）。

#### 流式实现
- trait `LlmProvider`（`src/llm.rs:222-286`）5 个方法：`generate_json`、`generate_json_with_usage`、`generate_json_with_usage_limit`（默认忽略 limit）、`generate_json_streaming`（默认**非流式兜底**：先整段生成再把 JSON 文本一次性推入 token_tx——与真流式口径一致都喂"JSON 文本"）、`generate_json_with_image`（默认抛 `vision_not_supported`，绝不把 base64 当文本塞 prompt）。`#[cfg_attr(test, mockall::automock)]`。
- 真流式 `generate_json_streaming_openai`（`src/llm.rs:892-996`）：body 带 `stream:true` + `stream_options:{include_usage:true}`；消费 `bytes_stream()`，CRLF 归一后按 `\n\n` 事件边界切分，残缺片段留 buffer；`data:` 行解析 `StreamChunkResponse`，delta.content 逐段 push 累积并 `token_tx.send`（前端断开 send 失败静默）；`[DONE]` 跳出；单帧解析失败跳过（keepalive 注释容忍）。**单次尝试不走重试循环**（流式已推 token 重试会致前端重复，`src/llm.rs:889-891`）。结束后 `parse_or_repair(accumulated)`——即流式也有第三层 LLM-repair 兜底（M10，不 re-stream，`src/llm.rs:986-988`）。
- Anthropic 流式未实现：退回 trait 默认整段一次性推（`src/llm.rs:1036-1044`）。
- Anthropic vision 未实现：抛 `vision_not_supported`（`src/llm.rs:1090-1092`）；OpenAI vision 用 content 数组 `[{type:text},{type:image_url,image_url:{url:"data:<mime>;base64,..."}}]`（`src/llm.rs:666-736`）。

#### Registry 与 DB 同步协议
- `LlmProviderMeta`（`src/llm.rs:1732-1743`）：`revision_ms`（updatedAt 毫秒，仅诊断）+ `runtime_fingerprint`——**缓存权威**是指纹不是时间戳：`llm_provider_runtime_fingerprint`（`src/llm.rs:1745-1760`）对 provider_id/format/base_url/api_key/model/timeout/max_retries/retry_base_ms/is_active 全字段 BSON 序列化取 SHA-256，可检测同毫秒写或手工库改。
- `ensure_default_llm_provider`（`src/llm.rs:1771-1958`）：启动确保默认 workspace 恰有一条 active provider。外层最多 5 attempt（`DEFAULT_PROVIDER_INIT_MAX_ATTEMPTS=5`，`src/llm.rs:1762`）：① 事务外先查 active 有则直接返回；② 开启事务（max_commit_time=5s）内再查；③ 无 active 但有行：按 `createdAt,providerId` 最早行做**选举提升**，update 条件带 `isActive:{$ne:true}` + `updatedAt` 旧值 CAS，modified≠1 抛 `default_provider_election_conflict`；updatedAt 取 `max(now, existing+1ms)` 保证单调（`src/llm.rs:1824-1827`）；④ 全空：插入 seed（provider_id="default"，name="默认 LLM"，format="openai"，取 config 的 openai_* 与 llm_* 参数，supports_vision=false）；⑤ 同事务内 `bump_generation_with_session(LLM_PROVIDER_NAMESPACE)`（`src/llm.rs:1878-1884`）；⑥ commit 循环：`UnknownTransactionCommitResult` 重提交至多 5 次，最后一次用**指纹核对**读回权威行判定是否实际已提交（`src/llm.rs:1908-1953`）；TransientTransactionError → abort + 20ms*attempt 退避重来。全部耗尽抛 `default_provider_initialization_exhausted`。
- `LlmRegistry`（`src/llm.rs:1969-2281`）：`RwLock<HashMap<workspace_id, {Arc<LlmClient>, meta, generation}>>` + `refresh_lock: Mutex`（refresh 合并）+ `database_state: RwLock<HashMap<ws,(gen, Instant)>>`（已观测的 DB generation + 校验时间）。
- **`snapshot_synced` 每请求同步协议**（`src/llm.rs:2126-2220`）：每次请求先读一行小 `configuration_generations`（namespace=`llm_provider`）；`seen != generation || fetched.elapsed() >= 30s`（`LLM_PROVIDER_CACHE_TTL`，`src/llm.rs:1979`）才进 refresh；拿 `refresh_lock` 后**重读** generation 防重复刷新；仍 stale 才查完整 active provider 行 → 算指纹 → 与当前 meta 比对 → 不同则 build 新 `LlmClient` 并 `swap`；最后 `mark_database_generation`。生产写方在同事务 bump generation → 立即可见；**手工改库不 bump → 由 30s TTL 兜底收敛**（`src/llm.rs:1973-1976`）。无 active 行抛 `LlmUnavailable{kind:"workspace_provider_missing"}`。
- `LlmRegistrySnapshot`（`src/llm.rs:1994-2046`）：请求获得的**不可变代**视图——持有 Arc client + meta + generation，途中热切换不影响在途请求（缓存标识与上游调用同一线性化点）。`swap` 写锁替换并 generation+1（overflow expect panic，`src/llm.rs:2249-2273`）；新 workspace 从 0 开始。`current()` 仅默认 workspace，缺失 panic（`src/llm.rs:2275-2281`）。

#### 测试段要点（`src/llm.rs:2331-3388`）
SSE 检测/聚合/usage 区分、think 前缀、finish_reason=length 保留、429/5xx/CF 5xx/body_decode 可重试、400/404 不可重试、tool_use 劫持可重试、退避表、trailing comma/未闭合/控制字符/自然语言包裹/伪块跳过/单对象数组拆包、repair 不吞噪声、账户错误短路、UA 头、bounded limit 透传 repair、截断 fail-closed 单请求、DeepSeek 控制字段作用域、Anthropic limit、重试 latency 含退避（wiremock）。

### 2.2 `src/llm_concurrency.rs`（197 行）——进程本地 LLM 准入
- 双信号量：`total`（总闸）+ `background`（后台闸）。`new(total_limit, foreground_reserved)`：total≥1，reserved clamp 到 [1,total]，`background_limit = max(total-reserved, total==1 时 1)`——退化部署 total=1 时保留 1 条后台道（`src/llm_concurrency.rs:66-82`）。
- `acquire(priority)`（`src/llm_concurrency.rs:88-113`）：Background 先拿 background permit 再拿 total permit；Foreground 只拿 total。效果：后台并发 ≤ background_limit，前台可吃满 total；预留量 foreground_reserved 后台永远碰不到。返回 `LlmAdmission`（RAII permits + queue_wait 计时）。
- `priority_for_prompt`（`src/llm_concurrency.rs:33-47`）后台白名单（刻意窄，未知一律前台）：`user.projection.task`、`user.memory_consolidator.*`、`user.initial_profile.*`、`evolution.*`、`knowledge.digest.*`、`knowledge.import.*`、`knowledge.auto_verify*`、`knowledge.tags.*`。测试锁定 reply/reaction/knowledge.agent/未知 → Foreground（`src/llm_concurrency.rs:138-164`）。

### 2.3 `src/mcp.rs`（1924 行）——微信 MCP 通道客户端

#### 超时铁律
`MCP_CLIENT_TIMEOUT_SECONDS = 60`（`src/mcp.rs:25`）：每次 MCP HTTP 的 reqwest 硬超时。**必须严格小于** dispatcher 外层 timeout——亲验 `src/agent/outbox_dispatcher.rs:154` `MCP_SEND_TIMEOUT_SECONDS=150`（60×2 次顺序调用=120 下界，取 150），lease `DEFAULT_LEASE_SECONDS=180 > 150`（`src/agent/outbox_dispatcher.rs:3227-3231`），不变量测试 `src/agent/outbox_dispatcher.rs:3647-3663`。理由（finding ①）：已送达但回包慢时须由 reqwest 自己超时返回 Err、`logged_call_for_account` 照常写 mcp_logs；若被外层 timeout 取消 future 会丢 mcp_logs → 崩溃恢复守卫查不到成功记录 → 误重试 → 客户收重复消息（`src/mcp.rs:12-24`）。

#### 握手与 session
- `sessions: Arc<DashMap<String, Option<String>>>`，键 = `base_url|api_key`（trim 尾斜杠，`src/mcp.rs:76-78`）——同进程对多 server/多 key 调用按 pair 隔离。值 `Some(id)`=有状态 server 下发的 `mcp-session-id`（如 gewe-multi-tenant）；`None`=无状态 server（MCP 规范里该头可选，两类都兼容，`src/mcp.rs:42-48`）。
- `initialize_session`（`src/mcp.rs:93-134`）：JSON-RPC `initialize`，`protocolVersion: "2024-11-05"`，clientInfo `{name:"wechatagent", version:"0.1"}`；header `Authorization: Bearer` + `Accept: application/json, text/event-stream`。**session id 在响应头，必须先取头再消费 body**（body 可能是 SSE，`src/mcp.rs:114-119`）。
- `post_rpc`（`src/mcp.rs:138-170`）：带 session 头发 JSON-RPC；HTTP 404（`Unknown MCP session`，server 重启/驱逐）→ 丢缓存重握手**一次**（`reinitialized` 标志），仍失败如实报错；非 2xx → `classify_mcp_http_error`：429/503→`AppError::UpstreamBusy`（可柔化为"同步中"），其它→External（`src/mcp.rs:337-343`）。

#### 响应解析（SSE 兼容）
`parse_mcp_response_body`（`src/mcp.rs:352-379`）：任一行以 `data:`/`event:` 开头判为 SSE → 收集全部 `data:` 行按 `\n` 拼接（SSE 规范多行 data）解析；否则整体按纯 JSON 解析；两者皆失败报错带截断 body（`truncate_for_error` 300 字符，`src/mcp.rs:346-348`）。garbage（HTML Bad Gateway）如实报错不吞（测试 `src/mcp.rs:1183-1186`）。

#### 工具调用两条路
- 通用 `call_tool_with_key`（`src/mcp.rs:172-224`）：`tools/call`；自动注入 `account_alias`（仅当提供且 arguments 是对象且未含该键——Workspace Key 下账号类工具必传，Account Key 可省，`src/mcp.rs:180-187`）。错误三层：JSON-RPC 顶层 `error`；**`result.isError==true`（finding ③：MCP 标准用 HTTP200+isError 表示"工具执行了但失败"如联系人拒收，只查 HTTP+顶层 error 会误判送达）**；成功取 `result.structuredContent`（无则 Null）。
- 发送专用 `call_send_tool_with_key`（`src/mcp.rs:228-306`）：区分 `McpSendError::SafeToRetry`（可证明未进入投递：ensure_session 失败、JSON-RPC error、isError）vs `DeliveryUncertain`（**从 `send()` 开始**的网络错、非 2xx、body 读取/解析失败——都不能证明未投递，必须停自动重放，`src/mcp.rs:263-282`）。供 outbox 决定能否自动重试。
- `list_tools_with_key`：`tools/list`（`src/mcp.rs:308-332`）。

#### 日志与脱敏
- 三个 logged 入口都写 `mcp_logs`（`McpCallLog{workspace_id, account_id, tool_name, request(脱敏), response, error, created_at}`）：`logged_call`（默认 workspace 部署级凭据，`src/mcp.rs:395-426`）、`logged_call_for_account`（per-account 凭据，`src/mcp.rs:428-470`）、`logged_send_call_for_account`（保留 McpSendError 分类，`src/mcp.rs:474-520`）。insert 失败静默（`let _ =`）。
- `redact_request_for_log`（`src/mcp.rs:387-393`，M16）：**只**把 `base64` 字段替换为 `<redacted base64: N chars>`（media_upload_base64 的 base64 可达 ~67MB，原样落库超 16MB BSON 上限）；**其它字段一字不动——尤其 `content`/`recipient`**，否则崩溃恢复 `mcp_already_succeeded` 的精确匹配失败 → 重复发送（红线测试 `src/mcp.rs:1230-1245`）。

#### per-account 凭据
`credentials_for_account`（`src/mcp.rs:572-628`）：查 `accounts` 集合（filter=`{workspace_id, account_id}`）；`mcp_base_url`/`mcp_api_key` 有值（trim 后非空）用账号级；缺失时**仅默认 workspace**允许回落到部署级 `config.mcp_base_url/mcp_api_key`（`deployment_mcp_fallback_allowed`：`workspace_id == default_workspace_id`，`src/mcp.rs:557-559`），非默认 workspace 缺配置直接 BadRequest（多租户不共享部署凭据）。`account_alias` 恒取 `account.alias`。

#### roster（好友列表）子系统
- `parse_roster_items`（`src/mcp.rs:765-870`）：多候选路径 `/items`（现工具 contacts_fetch_full 富化对象数组）、`/result/friends`（旧工具 contacts_fetch_cache 裸 wxid 字符串数组，2026-07-08 线上亲验）、`/result/contacts`、`/contacts`……+ `/structuredContent/*` 防御 + `/content/0/text` 内嵌 JSON 防御 + `contact_like_array` 按内容识别兜底（元素带 wxid/userName 键或纯字符串，`src/mcp.rs:689-707`）。**候选选取策略：优先"元素是对象"的数组**（同时含旧裸字符串与新富化形态时必须选富化，否则全表昵称/头像/性别丢光——"定时炸弹回归守卫"测试 `src/mcp.rs:1439-1464`）；非数组的高优先键不得短路后续候选（`src/mcp.rs:1392-1406`）。字段提取：wxid/userName/UserName/username；昵称 nickName/nickname/NickName；头像 bigHeadImgUrl→smallHeadImgUrl→…7 个候选；sex 兼容裸 int 与 int64 对象 `{high,low}` 取 `.low`（`src/mcp.rs:862-865`）。
- 非真人判定：`WECHAT_SYSTEM_ACCOUNTS` 13 个微信保留系统号白名单（fmessage/qqmail/weixin/mphelper/medianote/qmessage/floatbottle/tmessage/qqsync/newsapp/filehelper/weibo/brandsessionholder，`src/mcp.rs:711-725`）；`is_system_account` 与 webhooks `is_operatable_person` 共用（单一数据源防漂移）；`non_human_exclusion_filter()` DB 侧 `$nor` 等价过滤（`^gh_` 公众号 / `@chatroom` 群 / `@openim` 企微 / `$in` 白名单，`src/mcp.rs:744-758`）；公众号 gh_ 前缀单独拦，媒体号（wxid_* 如福州晚报）无法识别只能人工移除（`src/mcp.rs:727-733`）。
- 就绪判定不变式：`roster_outcome_from_result`（`src/mcp.rs:878-896`）——**解析出任何好友一定 syncing=false**（否则前端无限 8s 重拉且清空运营勾选）；空列表时看 `/status`：`"ready"`→就绪（真 0 好友），其它字符串→同步中，无 status→回落 `roster_result_is_empty_cache`（{}/Null/无命名数组候选=空 cache；空数组=真 0 好友已就绪，`src/mcp.rs:650-672`）。
- `fetch_roster_for_account`（`src/mcp.rs:913-959`）：调 `contacts_fetch_full`（无参）；同请求内短重试 3 次、间隔 2s；`UpstreamBusy`（429/503 限流）柔化为本次空 cache 继续重试/最终 syncing:true（前端提示"同步中"），**其它错误照常上抛**（不掩盖 401/500/配置错）。
- 快照：`roster_snapshots` 每账号恒一条（replace_one upsert，`src/mcp.rs:979-1006`）；过期阈值 24h（`ROSTER_SNAPSHOT_STALE_HOURS`，`src/mcp.rs:899`）。
- 后台自刷 `spawn_roster_refresh`（`src/mcp.rs:1104-1148`）：fire-and-forget spawn；**single-flight 去重**——`roster_refreshing: DashMap` 以 `ws|account` 为键 `insert` 抢锁（返回 Some=已占用放弃，原子无 TOCTOU）；`RosterRefreshGuard` RAII Drop 释放（正常结束/提前 return/**panic** 都释放，`src/mcp.rs:1011-1020`，panic 测试 `src/mcp.rs:1833-1848`）。最多 `ROSTER_REFRESH_MAX_RETRIES=5` 次（`src/mcp.rs:901`），退避 `3*2^attempt` 秒（3/6/12/24/48，`src/mcp.rs:909-911`）；**任何错误（含 Http 解码失败）都退避重试**（区别于同步路径）；拿到就绪即覆盖写快照。
- 送达核对 `chat_search_outbound`（`src/mcp.rs:1059-1096`）：调 `chat_search` 工具（direction=outbound, peer, content_contains, since, limit=100）核对某条 outbound 是否已真提交微信（timeout 兜底防重发）；命中判据 `chat_search_hit`（`src/mcp.rs:1025-1044`）：items 中存在 **content 精确等于**（非子串，防历史相似内容误命中）且 `createdAt >= since`（ISO-8601 解析失败保守不命中）；since 前移 5 分钟时钟偏差容忍（`CHAT_SEARCH_CLOCK_SKEW_TOLERANCE_MILLIS`，`src/mcp.rs:1046-1055`）。items 取顶层，防御回落 `/structuredContent/items`。

### 2.4 `src/config.rs`（924 行）——全部配置项
结构 `AppConfig`（`src/config.rs:10-400`）+ `from_env`（`src/config.rs:465-814`）。手写 Debug 对 mcp_api_key/openai_api_key/reviewer_second_provider_api_key/bootstrap_admin_password/jwt 双 PEM 过 `mask_secret`（`src/config.rs:405-462`），`finish_non_exhaustive`。辅助：`env_or`、`require_env`（缺失即启动失败）、`parse_bool`（1/true/yes/on 大小写不敏感）、`parse_csv_identities`（trim+sort+dedup）、`parse_bounded_i64/i32`（越界 bail）。全部配置项 env→默认→clamp 见 §4.1 事实卡（此处不重复）。要点：
- 必填仅 `MCP_API_KEY`、`OPENAI_API_KEY`（`src/config.rs:473,475`）。
- 有显式 clamp 的：`message_debounce_window_ms`[1000,10000]、`inbound_reply_worker_concurrency`[1,32]、`llm_max_concurrency`[1,64]、`llm_foreground_reserved`[1,64]、`post_decision_worker_concurrency`[1,32]、`post_decision_max_attempts`[1,100]、`post_decision_snapshot_max_bytes`[256KiB,8MiB]、`post_decision_prompt_max_chars`[8k,500k]、`post_decision_token_budget`[1k,500k]、`post_decision_failed_snapshot_retention_days`[1,365]、`agent_reply_max_segment_chars`≥1、`agent_reply_max_segments`≥1；bounded 校验（越界报错而非 clamp）：`knowledge_digest_run_token_budget`[1,1e6]、`knowledge_digest_run_max_llm_calls`[1,100]。
- 语义雷区：`campaign_max_audience` 是硬上限且 **0=全拒**不是"不限"（与 `ACCOUNT_SEND_*_INTERVAL_MS` 的 0=关闭约定相反，`src/config.rs:186-188`）；`progressive_tier_enabled`/`strategic_planner_priority_enabled`/`dynamic_confidence_real_outcome_enabled`/`reaction_gateway_parallel_enabled` 默认 **true**（与多数 *_ENABLED 默认 false 相反）；`evolution_auto_release_enabled` 即使 true 也被 HC-017 代码政策闸否决（`src/config.rs:282-285`）；`EVOLUTION_ENABLED_DEFAULT="false"` 是 env 硬上限，UI/Mongo runtime flag 只能在其下开启（`src/config.rs:5-7`）。

### 2.5 `src/prompts.rs`（3096 行）——prompt 包全文与治理

#### 版本与锚常量
- `PROMPT_PACK_VERSION = "wechatagent_prompt_pack_v16_2026_06_28_memory_structured_fact_and_dimension_required"`（`src/prompts.rs:16-17`）。
- `DEFAULT_MODE_GATE_POLICY`（`src/prompts.rs:31-36`）：「## 模式与 5 闸的关系」段（4 模式的五闸尺度），供非销售域 profile 整段替换的锚——**刻意不含** boundary_protection 反接管红线续行（那是跨域恒定红线，测试 `src/prompts.rs:2967-2979`）。
- `DEFAULT_REVIEWER_FEWSHOT`（`src/prompts.rs:49-52`）：reviewer 软闸打分 few-shot 三档锚（HumanLike 8/3/3 分例、EmotionalValue 8/3 分例、PressureRisk 8/1 分例——高压锚是销售逼单"今天最后一天…现在就定吧"）。
- `DEFAULT_REPLY_REDLINE_ANCHORS`（`src/prompts.rs:66-71`）2 条反接管红线逐字子串：`用户要求"真人 / 不想跟机器人聊"时，用 AI 自治语义承接`、`严禁承诺"安排真人 / 让同事来联系 / 稍后有人对接你 / 转接客服"`。
- `DEFAULT_REPLY_SYSTEM_REDLINE_ANCHORS`（`src/prompts.rs:75-78`）：不暴露 AI/不编造两句。`DEFAULT_REPLY_TASK_REDLINE_ANCHORS`（`src/prompts.rs:80-86`）：5 个 schema 关键行。`DEFAULT_REPLY_FAST_TASK_REDLINE_ANCHORS`（`src/prompts.rs:91-98`）：6 条（含"产品事实只能使用已注入的 verified 知识或产品目录"、"不要输出 profileUpdate、tags…"）。
- `DEFAULT_LOCALE="zh-CN"`（`src/prompts.rs:41`），缺失/空白回落（`src/prompts.rs:100-114`）。

#### 启动对齐协议（spec 为真相，不靠版本号）
- `ensure_prompt_pack_v2`（`src/prompts.rs:152-184`）：按"库里有无任何 prompt_templates 行"分流——空库→`bootstrap_prompt_pack_v2`（首次种四集合 souls/playbook/domain_configs/templates）；非空→`ensure_builtin_souls` + `delete_redundant_prompt_data`（只删 archived playbook；**prompt 行 append-only 永不启动 GC**，`src/prompts.rs:532-542`）+ `align_prompt_specs`。探针失败绝不当空库（`classify_prompt_pack_probe`，`src/prompts.rs:142-150`——防瞬时 DB 错误授权 bootstrap 覆写）。
- `align_prompt_specs`（`src/prompts.rs:262-346`）逐 key：draft spec 走 `align_planning_prompt_spec`（只保证存在一条内容匹配的 system draft，绝不发布；发现 current/active 行报错，`src/prompts.rs:348-405`）；active spec 用 `load_current_for_publish` 拿 current——`seeded_by=="evolution_release"` 跳过并写 `prompt_pack_align_skipped_evolution` 事件（演化发布的版本启动不覆盖）；`is_refreshable_prompt_seeded_by` 只认 `Some("system")`（manual/evolution_release/system_evolution_v1/None 一律保留，`src/prompts.rs:239-241`）；内容比对经 `normalize_prompt_content`（仅统一 CRLF→LF，不 trim 行尾——防 git autocrlf 造成每次重启版本膨胀，`src/prompts.rs:249-251`）；漂移则 `append_version` + `publish_version` 原子发布。
- `reset_prompt_pack_v2[_as_actor]`（`src/prompts.rs:407-426`）：显式重置——soul 走 `reset_builtin_souls`（追加 `seeded_by="system_reset"` 新版本并发布，**保留不可变历史**，`src/prompts.rs:923-976`）；`reseed_prompt_pack_components`（`src/prompts.rs:442-530`）物理删 prompt_templates/playbooks/domain_configs 后重种，并给所有 managed contacts 绑默认 playbook；末尾补种 `ensure_evolution_prompt_pack_v1`（M12：reset 删了 critic pack，不补则演化循环持续报错直到重启）+ `reconcile_prompt_pack_state_policies`（状态机 policy 对齐，不完整则报 `prompt_pack_state_policy_reconcile_failed`，`src/prompts.rs:191-230`）。
- 运行时加载：`load_prompt`/`load_prompt_for_contact`（`src/prompts.rs:544-572`）——`load_unique_current` 优先，无版本行回落 `default_prompt_content`（编译内置 spec）；有行但 current 指针坏 → 报错 fail-closed。`prompt_versions`（`src/prompts.rs:585-609`）汇总 run log 用的版本指纹（promptPackVersion + 各 key version + soul.version + playbook version）。`ab_bucket_for_contact`：DefaultHasher 稳定分桶（保留给 config/policy 路由，PromptTemplate 已不用，`src/prompts.rs:576-583`）。

#### soul_specs 全部（`src/prompts.rs:991-1108`）
| kind | name | status | 内容要点（逐字读后概括） |
|---|---|---|---|
| `user` | 默认用户运营 Soul v3 | published | 第一原则长期关系优先；开口前看 4 件事（customer_stage/tags/**custom_agent_instructions 最高优先级覆盖 Soul+Policy**/最近对话）；四模式锁定（casual_relationship/value_exchange/consultative/boundary_protection）；shouldReply=false 仅三种（明示不打扰/刚回复未表态/非真人探测），**寒暄全部必须接住**；按画像调口吻（理性客户"专业≠书面官腔"大段辨析、焦虑客户先共情、决策/高LTV绝不催促）；打分尺子四条（像微信真人/情绪价值/不施压/有独立个性——允许适度幽默但分场合）；多轮连续性 5 条（不重复寒暄/不自相矛盾/不重复追问/模式平滑过渡/多轮好差例）；情绪价值分轮次两把尺子 + **对抗压力轮两种隐蔽退行**（①把带火气的施压误判成边界而撤退——"那不打扰你了"是放弃这个人；②镜像攻击性或居高临下说教——"你先冷静"是甩责任）；硬约束红线：不暴露 AI、不编造、**无 verified 知识时绝不描述产品能力、不发占位符/半成品话术**（"我去确认"远胜假装有方案）、微信化表达抑制顾问报告腔、**反接管红线全文**（"我一直都在"承接；绝不承诺安排真人/同事；逼问负责人联系方式时①不得确认暗示存在可升级真人后台②绝不编造人名职务微信号工号；**首轮即生效**，附 ❌/✅ 对照话术与判定自检"回复里有没有'我'之外的人作为兜底角色"；与动词无关——传达/转交/上报/带话都违规）；两个跑偏警示（答非所问塞挡箭牌、复述被嘲讽为模板的原话）。结尾"统一话术就是失败的关系经营"。（`src/prompts.rs:993-1074`） |
| `management` | 默认后台管理 Soul v2 | published | 服务内部操作员；先判断意图/对象/账号/风险/缺失信息再生成结构化计划；只能通过系统工具执行、不编造结果；账号隔离；风险分级（查询/导入/画像低风险可自动，发送/纳管/配置中风险须目标明确，删好友/退群/登出/改资料默认不自动）。（`src/prompts.rs:1075-1086`） |
| `group` | 默认微信群运营 Soul v2 | draft | 只分析/识别线索/总结/建议；默认不群内自动发言；未来发言须白名单+频控+审计。（`src/prompts.rs:1087-1096`） |
| `moment` | 默认朋友圈运营 Soul v2 | draft | 产出可信草稿；优先内容资产/真实素材；不编造案例收入评价；默认只草稿+计划，自动发布须策略显式允许。（`src/prompts.rs:1097-1106`） |

#### prompt_specs 全部 35 key（`src/prompts.rs:1118-2428`）+ evolution pack 1 key
> layer/用途/正文要点/红线句；status 除注明 draft 外均 active。

**用户运营主链：**
1. `user.initial_profile.system`（system_contract）：画像分析 Agent 契约——严格 JSON；区分已确认/强线索/待确认/未知，未知不猜；服务长期关系不服务一次性强销售（`src/prompts.rs:1120-1131`）。
2. `user.initial_profile.task`（task_template）：初始画像 JSON schema（agentProfile{summary/interests/communicationStyle/operationGoal}/tags/customerStage/intentLevel/lastCommitment/commitment{text,dueAt RFC3339}/followUpPolicy/profileAttributes 7 字段）；红线：不把猜测写成事实、下一步策略低压像真人（`src/prompts.rs:1132-1172`）。
3. `user.reply.system`（system_contract）：运行时输出契约——只输出严格 JSON；长期关系经营者不是客服/强销售；**"不要暴露 AI、系统、模型、工具、提示词、内部评分或数据库字段。"**、**"不要编造价格、承诺、成交、案例、身份、产品能力或已经发生的事实。"**（此两句即 DEFAULT_REPLY_SYSTEM_REDLINE_ANCHORS）；微信纯文本不渲染 markdown、内容多拆短消息（`src/prompts.rs:1174-1186`）。
4. `user.reply.policy`（policy，v3）：①对话模式判定优先级树 6 级自上而下命中即停（特别指令>评估决策阶段>产品向问题>明确边界>可分享价值>casual 兜底），理由写 conversationModeReason；②「## 模式与 5 闸的关系」（=DEFAULT_MODE_GATE_POLICY 锚）+ boundary_protection 反接管续行（红线锚①："用 AI 自治语义承接…严禁承诺'安排真人/让同事来联系/稍后有人跟你对接'"+逼问负责人不松动+**首轮即生效**+"回复里除了'我'不得再出现任何可兜底/可接收诉求的人类角色"）；③shouldReply 高门槛 false（寒暄一律 true）；④决策协议字段枚举约束（consultative 无 verified 知识必须 knowledgeNeed=required/insufficient 不先编造）；⑤表达红线 10 条（多轮一致性、每次最多问 1 个关键问题、直接给清单不说"我发你"却不给、不暗示无来源案例经验、避免绝对化与数字承诺、不制造焦虑稀缺权威、**要真人时任何模式任何轮次正面接住诉求**（红线锚②）、不暴露 AI、**【隐私/内部画像】内部判断绝不向客户复述、不暴露幕后决策源存在**）；⑥标签与画像（持久属性 vs 本轮临时情景严格区分——施压/质疑/翻供/威胁投诉/试探是不是 AI 绝不写成 tags；"我是不是在被测试"不是用户画像；只增谨慎累积不整组重写）（`src/prompts.rs:1188-1248`）。
5. `user.reply.fast.task`（task_template）：紧凑发送决策 schema（decisionPhase=final/riskLevel/knowledgeNeed/runMode/autonomyMode/needsReview/conversationMode(+Reason)/shouldReply/replyText/operationState(+Reason/Confidence)/riskSelfCheck/why*/sufficiency/missingTier/clarificationIntent/usedKnowledgeIds/matchedKnowledgeIds/safeClaimsUsed/lastCommitment/commitment/followUp/assetsToSend/namecardToSend/escalationRequest）；硬规则：needsReview 只选复盘深度不决定是否审核（所有正文仍过独立 Reviewer+ClaimGate）；**产品事实只能使用已注入 verified 知识或产品目录**；时间承诺必须同步填 commitment；**不要输出 profileUpdate、tags、customerStage、intentLevel、domainSignals…（由发送后独立投影任务处理）**——权责分离锚（`src/prompts.rs:1250-1299`）。
6. `user.reply.task`（task_template，全量版）：**单发**决策（decisionPhase 恒 "final"，无 tool_calling 形态——护栏测试 `src/prompts.rs:2835-2850` 锁死，防 LLM 误选相位致静默 no_reply）；R1.3 七思考链字段（userUnderstanding/relationshipRead/operationGoal/knowledgeNeedReason/memoryUpdateReason/selfCritique/riskSelfCheck，低风险轮可 'unchanged' 短形式、关键变化轮 ≥20 unicode 字符禁 unchanged）；R1.4 whyShouldReply/whySkipReply 互斥必填（≥10 字符 ≥6 汉字；关键轮 ≥30/12）；nextBestAction 8 维评分；profileUpdate/tags/customerStage/intentLevel/dimensionDisplayNames（自造新值配 4-8 字中文名）/tagEvidenceTurns/stageEvidenceTurns/stageExplicitIntent（证据链，窗口序号 0-based）/bayesianObservations（≤6 开放维度带 confidence+evidenceTurns）/sufficiency 三档（enough/need_more_context/need_clarification——**need_clarification 时 replyText 只能是澄清问句不得硬答**）/missingTier（relational/full）/operatingMemoryUpdate 四段/memoryCandidates（≤6 型 fact|preference|doNotDo|commitment|objection|openLoop|conflict 带 evidence/importance/confidence）/memoryWriteScore（≥6 才异步整理）/followUp/assetsToSend（file_primary 只做一句引导 vs file_support 正常回答+佐证；禁编造 id）/namecardToSend（辅助模式引荐——铺垫话术不得出现"负责人/上级/能拍板的人/转接"，定位"增配专属顾问"不是"交给我之上的人"；见"已引荐"信号退为辅助答疑）/escalationRequest（**按事项实质判定，触及决策墙则必填 needed=true**；三 category：out_of_scope_decision/high_risk_gated/stuck_or_undelivered；**【自洽自检·必做】replyText 说了"向上反馈/领导确认"却省略 escalationRequest = 自相矛盾**；reply 仍正常写不冷场）/**【转述模式】`__PRINCIPAL_RELAY__` 载荷**（verdict=approved/rejected/conditional/deferred/delegated_back+substance+constraints；绝不把内部字段发给客户；按 verdict 定基调；rejected 保关系优先给替代方案）；死字段已删（intentAnalysis/productFitScore/forbiddenClaimRisk/recommendedResourceIds，护栏 `src/prompts.rs:2913-2940`）（`src/prompts.rs:1301-1503`）。
7. `user.projection.system`（post_decision_projection）：发送后投影 Agent——不回复客户、不能修改已授权的回复/审核/素材/名片/请示/承诺/跟进；只从冻结快照提取有证据增量；没有新信息返回空字段不为填满 schema 猜测（`src/prompts.rs:1505-1513`）。
8. `user.projection.task`（post_decision_projection）：稀疏增量 schema（profileUpdate/tags/evidence/bayesian/customerStage/intentLevel/domainSignals/dimensionDisplayNames/followUpPolicy/profileAttributes/nextBestAction/objectionsDetected/operatingMemoryUpdate/memoryCandidates/memoryWriteScore/consolidationNeeded/memoryUpdate/agentGeneratedSignals）；红线：临时情绪/施压/投诉/要求真人/对抗行为不得固化为标签；弱推断 stageExplicitIntent=false；suspected_deal 只作待核实弱信号绝不直接认定成交；relationship_type 是待审核信号；**不得输出 replyText/shouldReply/review/assetsToSend/namecardToSend/escalationRequest/lastCommitment/commitment/followUp**（与 fast.task 权威不相交，护栏测试 `src/prompts.rs:2856-2911`）（`src/prompts.rs:1515-1556`）。
9. `user.memory_consolidator.system`（memory_consolidator）：整理 Agent 原则——最新明确表达优先/猜测不写成事实/重复合并/过期进 deprecatedFacts 或 conflicts/寒暄丢弃（`src/prompts.rs:1558-1568`）。
10. `user.memory_consolidator.task`（memory_consolidator）：memoryCard schema（coreProfile/relationshipState/**coreFacts 结构化 {id,text,dimension,importance}**/recentFacts/preferences/doNotDo/commitments/objections/openLoops/recentEpisodeSummary/deprecatedFacts/conflicts）+ summary/discarded/reconfirmedTags（每个保留标签必须指认 evidenceTurns）/discardedTags/personality。**事实冲突/客户改口机制**：必须结构化显式弃用（deprecatedFacts{id,reason,supersededBy,deprecatedAt} 或 discarded 原文），summary 里写"已失效"不触发任何动作；系统自动保留未显式弃用的上一版 coreFacts（防早期事实意外丢失），不显式弃用会新旧矛盾并存污染决策；**dimension 改口必填**——同维度新旧 fact 同名，系统自动让旧值退出生效层。限制：coreFacts≤6 按 importance 倒序、recentFacts≤10 按 recency、fact 原子化（一条只讲一个事实）、preferences≤8、doNotDo≤10、commitments/objections/openLoops 各≤8。**大五 OCEAN 人格搭车输出**：五维固定封闭量表禁自创；每维必须挂 evidenceTurns，无证据 confidence=0（"宁可承认不知道，绝不脑补人格"）；行为锚定不套刻板印象（`src/prompts.rs:1570-1657`）。
11. `user.reaction.system`（reaction_analysis）：反应分析 Agent——结合长上下文/原话/语气/反讽，不按关键词机械分类（`src/prompts.rs:1659-1669`）。
12. `user.reaction.task`（reaction_analysis）：outcomeStatus 枚举 7 值（user_replied_positive/neutral/negative/objection/stop_requested/buying_signal/continue_exploring）+sentiment/intent/stopRequested/buyingSignal/objection/continueExploring/reason/confidence；few-shot 判例（"不用担心，可以继续聊"≠停止；"好像不太需要"≠正向；"谢谢，先不用了"通常是停止/降频；"可以，发我看看"是继续探索/购买信号；信息不足用 neutral 或 unclassified 不强判）（`src/prompts.rs:1671-1696`）。
13. `user.review.system`（review）：独立评审 Agent。**拦截阈值（Phase B 双闸+三 hard 闸）写死在 prompt**：FactRisk≥6 禁发（hallucination 闸）、PressureRisk≥7 禁发（软闸，0 仅"完全无法判断"兜底）、HumanLikeScore<6 改写、EmotionalValue<6 改写、ProductAccuracyScore<7 禁发产品承诺（grounding 闸）。requiresProductKnowledge 判定：含可验证产品断言（效果数据/价格/案例/能力承诺）无论语气软硬都置 true；纯情感承接/轻量澄清才 false。HumanLike 压分三形态（反射性编号列表顾问报告腔/微信不渲染的 markdown/超长段落）。few-shot 锚段（=DEFAULT_REVIEWER_FEWSHOT）。EmotionalValue 两把尺子（情绪轮：泛泛"我理解"压 5 以下；中性轮：不因"没共情"压分、硬挤共情/编造情绪是冒犯，**理性客户盲区**："专业但零温度的标准答案"压 5 以下，6+ 必须有只属于 ta 的接应；helpfulness≠emotionalValue）。**对抗压力轮两种退行检查**（误判边界而撤退/镜像攻击或说教——命中压 EmotionalValue 到 4 以下并 needs_revision）。多轮一致性红线三条（重复寒暄/自相矛盾/重复已答已问）。**反接管红线**：承诺"安排真人/让同事联系/运营同事整理后回你/转接客服"命中即改写；两种隐蔽变体也命中（①确认暗示存在可升级真人后台即便随即拒绝转接；②编造人名/职务/微信号/手机号/工号——最严重失约必拦截）；判定标准"是不是引入了'我'之外的人来接手"（`src/prompts.rs:1698-1741`）。
14. `user.review.light.system`（review）：轻量评审——不放弃底线（不编造/不暴露 AI/不高压/不违反 doNotDo）；涉产品能力价格案例效果承诺、用户拒绝或明显负面情绪必须提高风险给改写或拦截意见（`src/prompts.rs:1743-1755`）。
15. `user.review.product_claim_markers`（review_guard）：**JSON 格式的 prompt**——Rust 字符串兜底 guard 的可编辑标记词表：markers 10 条（literal：保证/一定能/绝对/百分之/案例/成功率/见效/回款；regex 类：numeric_percent_or_discount、price_amount）+ whitelistPhrases（准时/按时/尊重/保护/你的）+ whitelistWindowChars=8（`src/prompts.rs:1757-1779`）。

**知识/评测/管理：**
16. `knowledge.auto_verify`（knowledge_integrity）：切片自动校验——**只有 sourceQuote 非空且 sourceAnchors 能定位来源才允许 integrityStatus="verified"**；输出 confidenceScore/integrityStatus(verified|needs_review|rejected)/verifiedClaims/distortionRisks（`src/prompts.rs:1781-1798`）。
17. `eval.user_operation_judge.system`（evaluation）：shadow simulation 回归 Judge——具体价值/doNotDo/编造/状态迁移/有效记忆/像真人微信；知识不足允许保守说明但不允许编造（`src/prompts.rs:1800-1811`）。
18. `management.plan.system`（system_contract）：把操作员自然语言转成工具计划 JSON（intent/riskLevel=read|draft|configure|act|dangerous/requiresConfirmation/missingInformation/summary/toolCalls）；不编造工具名/执行结果（`src/prompts.rs:1813-1833`）。
19. `management.plan.policy`（policy）：风险分级明细（查询=read；草稿画像=draft；纳管/标签/内部任务=configure；发消息/建群/邀请/发布任务=act；**删好友/退群解散/登出/改资料/原始危险 MCP=dangerous 必须 requiresConfirmation=true 且 toolCalls 留空**）；发文本只用产品工具 `wechatagent.send_contact_message`（contactId/content）**禁止规划 message_send_text**；content 只含最终微信正文不得混入操作说明；"内容必须完全等于"须逐字；搜好友用 contacts_search/wechatagent.search_contacts，明确要导入才用 wechatagent.import_contacts（`src/prompts.rs:1835-1854`）。
20. `management.prompt_redline_review.system`（system_contract）：第三闸语义审查 judge——给 BEFORE/AFTER 完整快照（必须同时检查删除/新增/行内改写/重排/重复段删减）；违规信号 4 类（变相承认真人后台会直接对话/承诺转交传达上报给第三方真人/削弱知识 grounding/绕过"AI 永不自动认定知识已核实"）；合规：纯业务话术语气调整；**注意区分既定业务能力**（幕后决策源请示后 AI 转述、辅助模式引荐名片——不应仅因提及真人判违规，关键看是否让客户脱离与 AI 的对话）；输出 `{violation: bool, reason}`（`src/prompts.rs:1856-1881`）。
21. `playbook.generator.system`（methodology_generator）：content = `PLAYBOOK_METHODOLOGY_SYSTEM` 常量（`src/prompts.rs:2431-2440`）——方法论设计专家 7 条（严格 JSON/自然中文/可执行含观察信号判断规则下一步禁用动作复盘标准/科学克制不操控恐吓虚假承诺伪造稀缺/微信像真人/越聊越懂用户/**不预设行业——行业语义来自运营输入不写死行业词**）。
22. `group.policy`（policy，**draft**）：群运营第一阶段只分析/总结/线索/草稿建议，不自动发言邀请移除改公告解散退出（`src/prompts.rs:1893-1900`）。
23. `moment.policy`（policy，**draft**）：朋友圈只生成计划草稿；无来源素材不发布；不编造案例评价；自动发布须策略显式允许并记录来源（`src/prompts.rs:1901-1909`）。

**知识修复/对话/日报（全部强调"文案严守 AI 自治定位，只写'运营确认'"）：**
24. `knowledge.chunk.repair.propose`（knowledge_repair）：切片修复首轮——领域无关（先读懂切片讲什么/领域/读者/何时用）；**以原文为唯一事实源**（patch 内具体陈述必须能在父文档找到，找不到进 missingFields）；schema 是建议不是教条（字段在当前领域不适用就不硬填）；routingCard ≤60 字"何时打开"；evidenceItems 是溯源短语禁重写概括；领域专属字段进 patch.extras；追问 ≤3 条各 ≤60 字关联具体 missingField；confidenceHint 0-100 诚实自评；输出 interpretation{domain/audience/purpose/openConditions}+patch+missingFields+followupQuestions+confidenceHint（`src/prompts.rs:1911-1973`）。
25. `knowledge.chunk.repair.followup`（knowledge_repair）：追问后合并——运营回答只抽字段相关事实不整段塞；仍只在原文/运营回答两个事实源取材（编造证据是严重错误）；到最大轮数（3）followupQuestions 必须空数组；stillMissing 报告（`src/prompts.rs:1975-2033`）。
26. `knowledge.pack.repair.propose`（knowledge_repair）：知识包元数据一轮修复——跨切片归纳（输入含 ≤5 条 verified 切片摘要）；**销售色彩字段按领域重解读**（commonQuestions 在工程文档=工程师常见问题；customerStages 在医院制度=患者就诊阶段）；不适用不硬填；本轮不需要 followupQuestions（`src/prompts.rs:2035-2093`）。
27. `knowledge.chat.intent`（knowledge_chat）：对话意图识别 6 分类（create_chunk/update_chunk/clarify_chunk/digest_action/update_operator_memory/freeform）；引用 chunkId→大概率 update/clarify；无法判断不硬猜走 freeform；confidence≤0.6 也建议 freeform；memoryKind 闭集（preference/rejection/context）；memoryContent 提炼 ≤80 字非照抄否则降 freeform（`src/prompts.rs:2095-2141`）。
28. `knowledge.chat.draft_chunk`（knowledge_chat）：起草新切片 patch+追问 ≤3；**sourceQuote 必须真实原文不允许 AI 编造**（没给原文→missingFields+至少 1 条追问问出处）；patch 禁含 status/integrityStatus/sourceAnchors 系统字段；naturalReply 2-3 句对话风格（`src/prompts.rs:2143-2195`）。
29. `knowledge.chat.update_chunk`（knowledge_chat）：只改运营明确提到的字段其它省略键让后端用旧值；改 sourceQuote 必须确认新 quote 存在于父文档；applicable/notApplicableScenes 按加/删语义合并不全量覆盖（`src/prompts.rs:2197-2246`）。
30. `knowledge.chat.clarify`（knowledge_chat）：纯澄清不输出 patch；naturalReply 2-5 句；可 askMoreField/askMoreQuestion/nextSuggestion；不输出 JSON schema/代码块/markdown 列表（运营是普通对话视角）（`src/prompts.rs:2248-2274`）。
31. `knowledge.digest.compose`（knowledge_digest）：4 路只读信号（chunkHealth/usageDigest/blockedRuns/evolutionDigest）合成 ≤50 张行动卡片；kind 7 值/suggestedAction 6 值/severity 3 值；同信号源同目标只 1 张多信号合并 metric 求和；targetRefs.id 不在输入中整卡丢弃不硬造；**文案硬约束：AI 自治口径（AI 建议补完/复核/运营确认），禁止"人工接管/人工介入/人工托管/takeover/hand-off"字面量**（`src/prompts.rs:2277-2311`）。
32. `knowledge.digest.dispatch`（knowledge_digest）：勾选卡片→plannedSteps（stepId/cardId/action/summary/estimatedLlmCalls 1-3）；步数 ≤8、总 LLM 调用 ≤12（超则低优先级合并成 freeform）；naturalReply 不写"人工接管/接管"（`src/prompts.rs:2313-2345`）。
33. `knowledge.digest.summarize_logs`（knowledge_digest）：同 chunkId 多条 blocked run 摘 1 句 ≤50 字+topBlockReason+sampleRunIds≤3；**不泄露用户对话原文细节只说类别频次**；不用"人工/接管/hand-off"字面量（`src/prompts.rs:2347-2370`）。

**决策请示通道：**
34. `escalation.principal.interpret`（escalation）：把领导自然语言回复解读成结构化裁决——verdict 5 值（approved/rejected/conditional/deferred/delegated_back）+substance（转述唯一事实源）+constraints+authorization_window_hours（**领导说了算：明确给时限才填数字，没提填 null 不自己默认**；不控制长期豁免）+exemption_type（none 默认/customer_only 仅当前客户长期/knowledge 沉淀通用口径；判断不出 none）（`src/prompts.rs:2372-2407`）。
35. `escalation.sediment.title`（escalation）：把领导裁决实质提炼成 ≤20 字知识标题；不写"领导同意/授权"过程描述；只依据给定实质不臆造（`src/prompts.rs:2409-2427`）。

**evolution pack（独立版本号）：**
36. `evolution_critic_v1`（agent_kind=evolution，layer=critic，`EVOLUTION_PROMPT_PACK_VERSION="wechatagent_evolution_pack_v1_2026_05"`，`src/prompts.rs:2512,2514-2556`）：审视 Reply Agent prompt 的 Critic——输出 diffs[]（templateKey/section/summary≤200 字/snippet≤4000 字/expectedImprovementOn/riskNote）；policy 违反任一整批 drop：**禁词表**（human takeover/hand off/handoff/takeover/人工接管/人工介入/人工托管/接管/人工——遇风险用 AI 内部状态名 held_by_ai_policy/blocked_by_safety_guard/ai_waiting_for_more_context）、不得建议绕 5 闸拦截阈值（可改进触发前表达不可放宽 review 判定）、不得建议直接引用未验证产品事实、**不得自指**（templateKey≠evolution_critic_v1，`PROMPT_EVOLUTION_FORBIDDEN_KEYS`，`src/prompts.rs:2508`——防"prompt 互斥反馈环"design.md §9.3）；单 tick ≤4 条 diff，没有可信建议输出空 diffs 不凑数；目标是"prompt 表达层根因"非"模型能力问题"。种入 `ensure_evolution_prompt_pack_v1`（幂等：有 current 即跳过，`src/prompts.rs:2458-2503`），seeded_by="system_evolution_v1"。

#### 状态机种子（`src/prompts.rs:760-878`）
`default_user_operation_state_machine`：9 态（new_contact/relationship_building/need_discovery/solution_fit/objection_handling/commitment_followup/customer_success/cooldown/dormant_reactivation），每态 key/name/goal/allowedActions/allowedFrom/advanceSignals/cooldownSignals/riskRules/successCriteria。标志位（H13 行业无关引擎依据）：`new_contact` 唯一 `initial:true`（空 from 唯一合法迁入目标，`src/prompts.rs:772`）；`cooldown` `allowFromAny:true` + **`forbidsProactive:true`**（禁 planner 主动触达+m013 policy 禁 reply，`src/prompts.rs:850-855`）；`dormant_reactivation` `allowFromAny:true`（任意态可流失休眠，G5 阶段 2，`src/prompts.rs:866-870`）。

#### playbook 与 domain_configs 种子
- `default_playbook`（`src/prompts.rs:611-649`）："默认长期关系运营方法 v3"——method_prompt 核心公式 5 条（信任=专业可信+稳定可靠+亲近感-自我推销感；成交准备度=动机×产品匹配×时机×信任÷阻力；情绪价值/下一步动作评分/学习深度）；profile/tag/stage/intent/follow_up/reply_style/forbidden_rules/success_criteria 8 段方法。tag_method 强调临时情景不写标签；intent_method 强调**低意向≠不回复**；follow_up_method 强调用户主动消息必须回应、同一关键问题最多连续追问 2 次。
- `default_domain_configs`（`src/prompts.rs:651-758`）：`user_operations`（active，runtime_parameters：recentMessageLimit=12/minReplyIntervalSeconds=20/maxDailyTouches=3/maxPendingFollowUps=3/followUpExpiresHours=48/cooldownAfterNoReplyHours=24/**factRiskBlockAt=6/pressureRiskBlockAt=7/humanLikeRewriteBelow=6/emotionalValueRewriteBelow=6/productAccuracyBlockBelow=7**/operationStateConfidenceFullReviewBelow=4/runTokenBudget=30000/runMaxLlmCalls=6/simulationTokenBudget=60000）；`group_operations` 与 `moment_operations` 均 **draft**（Phase 2 规划域无状态机，active 会被启动 sanity check bail，`src/prompts.rs:712-715`）。

### 2.6 `src/prompt_guard.rs`（484 行）——三层分级 + 双闸 + LLM 语义第三闸
- 模块位置纪律：位于 src/ 顶层在 CI 禁词 lint 扫描区，非测试代码绝不内联禁用词字面量（只 import 锚常量）；测试用字符拼接 `["人","工","接","管"].concat()` 绕字面量（`src/prompt_guard.rs:8-11,231-233`）。
- 三层 `PromptEditTier`（`src/prompt_guard.rs:22-27,49-72`）：**Forbidden**=`evolution_critic_v1`（与 PROMPT_EVOLUTION_FORBIDDEN_KEYS 同源）+ `management.prompt_redline_review.system`（语义审查 judge 禁自改）；**ConstrainedEditable**=有 required_anchors 的 5 个 key；**FreelyEditable**=其余（仍过禁用词闸）。
- `required_anchors`（`src/prompt_guard.rs:33-47`）：`user.reply.policy`→业务锚 DEFAULT_MODE_GATE_POLICY + 红线锚 DEFAULT_REPLY_REDLINE_ANCHORS（**核心修正：旧设计只查业务锚，红线被删却能放行**）；`user.reply.system`/`user.reply.task`/`user.reply.fast.task`→各自 schema/红线锚；`user.review.system`→DEFAULT_REVIEWER_FEWSHOT。
- 双闸 `validate_prompt_edit`（`src/prompt_guard.rs:75-102`，fail-closed）：闸 1 禁用词（`evolution::lint::passes_forbidden_words`，自由/强约束层都过）；闸 2 锚完整性（全部锚段 normalize CRLF 后逐字 contains，缺失即拒）。
- 第三闸 `review_prompt_edit`（`src/prompt_guard.rs:142-195`）：字面双闸挡不住"保留锚段、无禁词、却语义绕过"（变相真人转介/承诺转交/削弱 grounding）→ LLM 对 `extract_diff`（**完整 BEFORE/AFTER 快照**，非增量 diff——集合式行 diff 会漏重排与重复行删除，仅 CRLF 等价可跳过审查，`src/prompt_guard.rs:118-128`）做语义判定。三态 `PromptEditVerdict`：Pass / Reject(reason) / **NeedsHumanConfirm{diff,reason}**（LLM 加载失败/调用失败/**模糊响应**都降级人确认——`classify_review_verdict` 只有 `violation==false` 才 Pass，字段缺失/非布尔/空对象不 fail-open，`src/prompt_guard.rs:203-220`）。judge 走 `generate_agent_json`（复用重试/退避/RunBudget）。
- `compose_appended_content`（`src/prompt_guard.rs:132-138`）：evolution critic 片段**末尾追加**合成（原文逐字保留在开头→锚闸天然通过；critic 只能加约束不能改写红线段）；空 snippet 字节级 no-op。

### 2.7 `src/prompt_template_versions.rs`（441 行）——append-only 版本流 + 发布事务
- `NewPromptTemplateVersion` 校验：prompt_key/agent_kind/layer/prompt_pack_version/actor/seeded_by 须 canonical（非空且无首尾空白），title/content 非空（`src/prompt_template_versions.rs:34-50`）。
- `append_version`（`src/prompt_template_versions.rs:81-111`）：读最新 version+1 → insert（status="draft", current_version=false）；**唯一索引仲裁并发**——duplicate key（11000/11001，`is_duplicate_key_error` 兼容 BulkWrite，`src/prompt_template_versions.rs:375-387`）重读重试最多 `VERSION_INSERT_RETRIES=8` 次，耗尽抛 `prompt_version_allocation_conflict`。`append_edited_draft`：prompt_key 不可变（`prompt_key_is_immutable`）。
- 读取：`load_current_for_publish`（`src/prompt_template_versions.rs:161-187`）limit 2 查 current_version=true——0 行 Ok(None)；1 行且 active Ok；1 行非 active 报 `current_prompt_not_active`；≥2 报 `multiple_current_prompts`。`load_unique_current`（`src/prompt_template_versions.rs:135-157`）在其上加：无 current 但存在任何版本行 → `current_prompt_missing`（运行时 fail-closed，不静默回落内置 spec）。
- `publish_version`（`src/prompt_template_versions.rs:208-373`）Mongo 事务：① 读 target（status 须 draft|archived|active）；② 读全部 current（limit 2）+ 查 non-current active 行；③ `validate_publish_pointer_state`（>1 current / current 非 active / 存在 non-current active 均 Conflict，`src/prompt_template_versions.rs:189-204`）；④ target 已是 current 幂等返回；⑤ 归档旧 current（update 条件含 `_id`+prompt_key+version+current_version:true 精确 CAS，modified≠1 抛 `prompt_publish_pointer_changed`）→ 提升 target（条件含 status 旧值+current_version:false CAS，失败抛 `prompt_publish_target_changed`）；⑥ commit 循环：`UnknownTransactionCommitResult` 无限重试 continue，其它错误 abort 并折成 `prompt_publish_conflict`；事务内 Db 错误也折成 Conflict（把存储错误归一为可重试冲突）。内容行只改生命周期元数据，正文永不改（append-only）。

### 2.8 `src/soul_versions.rs`（478 行）——Soul 版本流（与 2.7 同构，差异如下）
- 状态集是 `draft|archived|published`（prompt 是 active，soul 是 **published**）。
- 多了两个幂等初始种子入口：`ensure_initial_published`（`src/soul_versions.rs:110-149`）——仅当 `(workspace, kind)` 流完全不存在才种 v1 published；已存在则 `load_unique_published` 读回（**draft-only 存量流是不变量错误，不静默修复**）；并发 duplicate key 输掉后重读。`ensure_initial_draft`（`src/soul_versions.rs:155-195`）——同理种 v1 draft 占位，已有运营流原样保留。两者都拒绝 previous_version=Some（初始版本不能有前驱）。
- `publish_version`（`src/soul_versions.rs:238-400`）：事务内额外做**重复 published 检测**（同 kind 存在 `_id≠current` 的第二条 published → `multiple_published_souls`）；归档/提升 CAS 同 prompt；commit 循环同款。`published_at/published_by` 审计字段。
- `load_unique_published`（`src/soul_versions.rs:403-437`）：恰 1 条 published 才 Ok；0 条 `published_soul_missing`、≥2 `multiple_published_souls`（运行时 fail-closed）。
- `append_edited_draft` 拒绝改 kind（`soul_agent_kind_is_immutable`——防拼接两条独立版本流）。

### 2.9 `src/supervisor.rs`（538 行）——worker panic 熔断状态机
- 背景：main.rs `tokio::spawn` 的长驻 worker panic 后 JoinHandle 被 drop 静默死亡；`spawn_supervised` 包 `loop{ catch_unwind }`（`src/supervisor.rs:307-408`）。
- 常量：`INITIAL_BACKOFF_SECS=1`、`MAX_BACKOFF_SECS=30`（指数 1→2→4→8→16→30 封顶）、`FAST_PANIC_WINDOW_SECS=60`、`CIRCUIT_OPEN_AFTER_FAST_PANICS=5`、`CIRCUIT_POLL_SECONDS=30`（`src/supervisor.rs:28-32`）。
- `SUPERVISED_WORKERS` 16 个（`src/supervisor.rs:34-51`）：task_worker/inbound_reply_worker/import_worker/outbox_dispatcher/post_decision_worker/media_storage_reconciler/strategic_planner/cold_contact_worker/silence_signal_worker/evolutionary_worker/knowledge_digest_worker/knowledge_task_worker/catalog_rebuild_worker/knowledge_feedback_worker/ingest_worker/management_command_sweeper。（注：文件头注释写"8 个"已过时，实际 16 个——见 §5 疑点。）
- **熔断状态机**（持久化在 `background_worker_controls`，`_id="worker::{name}"`）：
  - `closed`/无行 → 允许启动；worker 正常返回 () 视为主动退出**不重启**；panic → 计 `next_fast_panic_count`（距上次启动 ≥60s 重置为 1，否则 +1，`src/supervisor.rs:57-63`）→ 写 `agent_events`（kind=`background_worker_panic` 或 `background_worker_circuit_open`）→ 未达阈值 sleep 退避重启。
  - 60s 内连续 5 次 panic → `open_worker_circuit`（`src/supervisor.rs:148-207`）置 `status="open"`（正常路径 filter 限定 closed/null/不存在，可 upsert 建行；**probe 失败路径以 probe_token 精确 fence，绝不 upsert**——防 stale probe 覆写新 owner）+ `$inc circuit_generation` + `$unset` 全部 probe 字段。open 后停止重启。
  - 管理员 `resume_worker_circuit`（`src/supervisor.rs:252-288`）：open→`half_open`（校验 worker 名在白名单）+ 审计字段。
  - `wait_until_circuit_allows_start`（`src/supervisor.rs:71-147`）每 30s 轮询：closed/无行放行；open 等待；half_open 或 probing-已过期（`probe_locked_until <= now` 或缺失）→ CAS `find_one_and_update` 抢 `status="probing"` + 新 uuid `probe_token` + `probe_locked_until = now+120s`（60s×2），**读回 token 相等才算抢到**（跨副本只有一个 probe）；DB 错误 fail-closed 继续等。
  - probe 运行 60s 无 panic → `mark_worker_recovered`（并行 spawn，token fence CAS probing→closed 清零，成功置 `stabilized` 原子标志，`src/supervisor.rs:209-246`）；probe 期 panic（stabilized 未置位）→ `should_open_circuit(is_probe && !stabilized)=true` **立即重开**（`src/supervisor.rs:248-250`）；已稳定后的 panic 按普通失败计数。worker 结束时 abort recovery_marker 任务。
- `panic_payload_to_string`：&str/String/其它 `<non-string panic payload>`（`src/supervisor.rs:410-418`）。

### 2.10 `src/secret.rs`（65 行）
`mask_secret`：空串原样返回（区分"未配置"）；字符数 ≤8 整体 `"****"`（防短 key 反推）；否则 `前3 + "****" + 后4`（按 char 数防多字节 panic，`src/secret.rs:17-34`）。使用约定：任何 api_key/password/secret/token 在 tracing/format/API 响应出现都过此函数；含此类字段的结构体必须手写 Debug（`src/secret.rs:6-12`）。

### 2.11 `src/error.rs`（203 行）
`AppError` 14 变体 → HTTP 映射：BadRequest→400、NotFound→404、Conflict→409、Unauthorized→401、Forbidden→403、`BudgetExceeded{run_id,reason}`→503（error=budget_exceeded；MP-5 调用方应捕获走降级不外泄 5xx 给 webhook）、`RateLimited{retry_after,account_id}`→429+Retry-After 头、`AuthRateLimited`→429+Retry-After（响应刻意不暴露 username/指纹）、`LlmUnavailable{kind,retry_count,detail,hint}`→503 `{error:"llm_unavailable", kind, retryCount, detail, hint}`（前端按 kind 渲染中文文案+「AI 重试」按钮）、`UpstreamBusy`→503 `{error:"upstream_busy"}`、**Db/Http/Json/BsonSer/External→502 只回稳定分类码**（db_error/upstream_error/serialization_error/internal_error），原始错误（可能含连接串/上游 URL）只进 tracing 绝不写 HTTP body（`src/error.rs:139-156`）。

### 2.12 `src/media_storage.rs`（684 行）——素材内容寻址存储
- **路径安全**：`is_safe_segment` 仅 [A-Za-z0-9_-]；`safe_relative_path(ws, sha, ext)` → `ws/{sha 前 2 位}/{sha}.{ext}` 分片布局（`src/media_storage.rs:69-82`）；`is_safe_relative_path` 拒绝绝对路径/任何非 Normal component（`..` 穿越，`src/media_storage.rs:60-67`）——所有读/写/删入口都先 `ensure_safe_relative`（防老库脏值把清理变成穿越）。扩展名白名单 14 种（pdf/png/jpg/jpeg/gif/webp/doc/docx/xls/xlsx/ppt/pptx/mp4/mov，`sanitize_ext` 须 ext+mime 匹配或 mime 为空，`src/media_storage.rs:84-116`）。
- **路径锁**：进程级 `PATH_LOCKS: LazyLock<Mutex<HashMap<abs_path, Weak<AsyncMutex>>>>`（Weak 自动清理）；`lock_paths` 排序去重后逐个锁（稳定顺序防死锁）——所有引用创建/释放与物理发布/删除都持锁，关掉单进程 count-then-delete 竞态（`src/media_storage.rs:143-187`）。
- **两段发布协议**：`stage_bytes`（`src/media_storage.rs:200-223`）写 `{rel}.wa-pending` 并 flush+sync_all **在任何 Mongo 写之前**；已有 Valid 终态对象返回 false（内容寻址去重）、Corrupt 先删。`publish_staged`（`src/media_storage.rs:227-251`）：读 pending 校验 sha256 与路径中的 sha 一致（不符删 pending 抛 InvalidData）→ 同目录 `rename` 原子发布。`discard_staged` 删 pending。
- **崩溃窗口处理场景**：① Mongo 写失败仍持锁 → `settle_staged_after_db_failure`（`src/media_storage.rs:261-278`）：查 content_assets 引用数，0→discard，>0→publish（引用赢）；查询失败留给 reconciler。② DB 已提交但 rename 未发生 → `recover_pending_file`（`src/media_storage.rs:282-294`）持锁：final 存在 true；pending 存在则 publish；都无 false。③ 读时恢复 `read_bytes_recovering`（`src/media_storage.rs:298-326`）持锁：Valid 直读；**Corrupt（sha 不符）删除后尝试 pending**，无 pending 按 was_corrupt 抛 InvalidData/NotFound（内容寻址对象损坏绝不返回）。
- **reconciler**（`reconcile_once`，`src/media_storage.rs:454-557`，`RECONCILE_INTERVAL_SECS=3600`）六场景：a) DB file_path 非法（穿越/非管理布局）→ `fail_close_assets`（sendable=false + review_status=draft + review_note=storage_path_invalid + $unset media_id，计 disabled_invalid_assets）；b) 引用行持锁重查引用数=0 → 删残留 pending（removed_pending）；c) 引用且 Valid → 清多余 pending；d) 引用且 Corrupt → 删损坏终态（removed_corrupt）再试 pending；e) 引用且 Missing → pending 存在则 publish（recovered_pending；InvalidData 则删计 removed_pending），仍无对象 → fail_close（storage_object_missing，disabled_missing_assets）；f) 盘上文件（`collect_managed_files` 递归walk，只认 `ws/sh/sha.ext` 管理布局 `is_managed_layout`：3 段、sha 64 hex、shard=sha[..2]，`src/media_storage.rs:343-385`）不在引用集 → 持锁重查仍无引用 → 删（pending 计 removed_pending、终态计 removed_orphans）。`reconciler_loop` 每小时跑一次（有变化才 info 日志）。
- `should_delete_physical_file(remaining_refs)==0`（`src/media_storage.rs:575-577`）：upload 不去重、多条记录可共享同一 file_path，删记录前查兄弟引用。

### 2.13 `src/outbound_fetch.rs`（476 行）——ingest 出站 SSRF 防护
- 常量：`MAX_INGEST_RESPONSE_BYTES=4MiB`、`MAX_REDIRECTS=5`、DNS 5s / connect 10s / request 30s（`src/outbound_fetch.rs:17-21`）。
- **URL 规则**（`parse_public_http_url`，`src/outbound_fetch.rs:186-208`）：仅 http/https 绝对 URL；**禁 URL 内嵌凭据**（user:pass@）；禁端口 0；域名 trim 尾点+转小写，**禁 localhost 与 *.localhost**；清 fragment。
- **DNS 解析与公网校验**（`resolve_public_target`，`src/outbound_fetch.rs:210-239`）：IP 字面量直接检查；域名 `lookup_host`（5s timeout）→ 排序去重 → **任一解析地址非公网即整体拒绝**（`NonPublicAddress`）。
- **DNS pinning + 防 rebinding**：`fetch_ingest_url`（`src/outbound_fetch.rs:76-184`）每一跳新建 client——`redirect(Policy::none())`（手动跳转）+ `no_proxy()`（环境/系统代理都不能绕检查）+ `resolve_to_addrs(domain, checked_addresses)`（连接钉死在已校验地址）；响应后再验 `remote_addr()`：**必须公网且在解析集内**（`src/outbound_fetch.rs:110-120`，MissingRemoteAddress 也拒）。存库前校验 `validate_public_http_url` 与请求前解析是同一套（保存时决策不可信赖——DNS 后改也拦，`src/outbound_fetch.rs:65-72`）。
- **重定向**：仅 301/302/303/307/308（`is_follow_redirect`；300/304/305/306 不跟）；Location join 后**重新过完整 URL 检查**（file:///凭据/localhost 都拒，`resolve_redirect_url`）；≤5 跳；**条件请求 If-None-Match 只发给首跳**（不把 etag 泄给重定向目标；内容 hash 去重兜底，`src/outbound_fetch.rs:99-105`）。
- **响应约束**：304 直接返回空 body；非 2xx 返回 status+etag 不读体；Content-Type 按 source_kind 白名单（rss：application/rss+xml|atom+xml|xml、text/xml|plain；html：text/html、application/xhtml+xml、text/plain；缺失或其它 kind 一律拒，`src/outbound_fetch.rs:259-282`）；Content-Length>4MiB 先拒；流式读体 `append_limited` 增量限 4MiB（`src/outbound_fetch.rs:284-294`）。
- **公网 IP 判定**：IPv4 排除 15 个保留段（0/8、10/8、100.64/10 CGNAT、127/8、169.254/16 链路本地含云 metadata 169.254.169.254、172.16/12、192.0.0/24、192.0.2/24、192.88.99/24、192.168/16、198.18/15、198.51.100/24、203.0.113/24、224/4 组播、240/4，`src/outbound_fetch.rs:303-326`）；IPv6：**v4-mapped 先转 v4 判**（::ffff:127.0.0.1 拦截）；fail-closed 到 2000::/3 全球单播白名单再排除 2001::/23、2001:db8::/32、**2002::/16（6to4 可编码私网 v4）**、3ffe::/16（`src/outbound_fetch.rs:337-355`）。测试覆盖十进制/十六进制/八进制编码 IP（2130706433、0x7f000001、0177.0.0.1）都被解析成 IP 后拦截（`src/outbound_fetch.rs:371-397`）。

---

## 3. 跨文件机制

### 3.1 一次 LLM 调用的完整链路（从 `generate_agent_json` 到 HTTP）
1. **入口**：`src/agent/mod.rs:254` `generate_agent_json(state, workspace_id, account_id, contact_wxid, run_id, prompt_key, system, user)`——项目唯一 LLM JSON 入口（CLAUDE.md 约定；`prompt_guard::review_prompt_edit` 等也走它）。
2. **Registry 同步**：`state.llm_registry.snapshot_synced(db, config, workspace_id)`（`src/agent/mod.rs:269-276`）→ 读 `configuration_generations` 一行（namespace=`llm_provider`，`src/db/config_generation.rs:18,36-53`；生产写方在同事务 `bump_generation_with_session` `$inc generation`，`src/db/config_generation.rs:74-90`）→ generation 变化或 30s TTL 到期才重新拉 active provider 行、按 runtime_fingerprint 决定是否 rebuild client 并 `swap`（`src/llm.rs:2126-2220`）。返回不可变 `LlmRegistrySnapshot`（client+meta+generation 同一线性化点）。
3. **精确缓存查询**：cache key 仅 4 个白名单 prompt key（`knowledge.import.preview`/`playbook.generator`/`playbook.optimizer`/`user.guide.preview`，`src/agent/mod.rs:828-836`）；key = FNV hash(workspace)+hash(provider)+hash(model)+generation+hash(prompt_key)+pack_version+hash(system)+hash(user)（`src/agent/mod.rs:818-848`）——provider 热切换（generation）与 prompt 发布（`state.prompt_pack_version` 原子计数）都会令旧 entry 自动失效；shadow run 不读不写缓存（`src/agent/mod.rs:294-297`）。命中 → 写 `llm_call_logs` status=`cache_hit` 直接返回。
4. **预算与准入**：`reserve_current_run_llm_attempt()`（RunBudget task-local，超额抛 `AppError::BudgetExceeded`）→ `state.llm_concurrency.acquire(priority_for_prompt(prompt_key))`（`src/agent/mod.rs:349-353`；后台 prompt 先过 background 信号量再过 total，见 §2.2）→ 记录 queue_wait。
5. **上游调用**：按 `critical_path_output_token_limit(prompt_key)`（fast reply 8192 / light reviewer 3072 / reviewer 8192 / claim gate 3072，`src/agent/mod.rs:233-246`）选择 `generate_json_with_usage_limit` 或 `generate_json_with_usage`（`src/agent/mod.rs:356-370`）→ 进入 `LlmClient::generate_json_with_usage_bounded` 重试循环（`src/llm.rs:842-878`）→ 单次 `generate_json_once_{openai,anthropic}` → HTTP → SSE 检测/聚合 → 截断检查 → `strip_reasoning_prefix` → `parse_or_repair_bounded`（三层确定性 + 2 次回喂修复）。
6. **收尾**：成功 → `record_current_run_reserved_llm_usage(usage)` 计入 RunBudget、写缓存、`log_llm_call_success`（llm_call_logs status=success，含 queue_wait_ms/provider_latency_ms/retry_count/usage_known）、`system_incident::observe_llm_recovery`；失败 → 计一次未计量调用（token 完整性显式标记 unknown）、`log_llm_call_failure`（failed/json_error）、若 `llm_account_unavailable_reason` 命中还会 `observe_llm_account_unavailable` 记系统事件（`src/agent/mod.rs:373-455`）。

### 3.2 prompt 从种子到运行时到版本记录的旅程
1. **编译期**：`prompt_specs()`/`soul_specs()` 硬编码在 `src/prompts.rs`（35+1 个 prompt、4 个 soul）——spec 即真相。
2. **启动期**：`main.rs` 调 `ensure_prompt_pack_v2`（空库 bootstrap 四集合 / 非空库 align 逐 key 内容比对）+ `ensure_evolution_prompt_pack_v1`（独立 pack）。align 只刷新 `seeded_by=="system"` 的 current；manual/evolution_release 的 current 永不被启动覆盖（`src/prompts.rs:307-315`）。所有写入走 `prompt_template_versions::append_version`（draft）→ `publish_version`（事务原子换 current 指针，旧行归档不删）。
3. **编辑期**：管理者自然语言编辑 → `prompt_guard::validate_prompt_edit`（禁词闸+锚闸）→ `review_prompt_edit`（LLM 语义第三闸，三态）→ 通过才 `append_version`+`publish_version`（seeded_by=manual）。evolution release 走 `compose_appended_content` 末尾追加 + 同三闸（seeded_by=evolution_release）。
4. **运行期**：`load_prompt`/`load_prompt_for_contact` → `load_unique_current`（fail-closed：有版本行但指针坏即报错）→ 无任何行回落编译内置 spec。发布后 `state.prompt_pack_version` fetch_add 令 LLM 精确缓存旧 entry 失效（`src/agent/mod.rs:287-292`）。
5. **审计**：每次 run 写 `prompt_versions` 文档（promptPackVersion + 每 key version + soul.kind version + playbook version/name，`src/prompts.rs:585-609`）进 run log。

### 3.3 MCP 发送链路的时序契约（亲验）
`mcp.rs` reqwest 60s（`src/mcp.rs:25`）< dispatcher 单条 send 外层 timeout 150s（`src/agent/outbox_dispatcher.rs:154`，覆盖媒体上传+发送 2 次顺序调用 60×2=120 下界）< lease 180s（`src/agent/outbox_dispatcher.rs:3227-3231`）。三层不等式由测试 `src/agent/outbox_dispatcher.rs:3647-3663` 锁死。语义：内层先超时保住 mcp_logs 写入（崩溃恢复凭据）；外层不把在发条目让给 reclaim 并发重发。发送用 `call_send_tool_with_key` 的 SafeToRetry/DeliveryUncertain 二分 + `chat_search_outbound` 事后核对，构成防重发三重保障。

---

## 4. 事实卡速查

### 4.1 全部配置项（env → 默认 → clamp/校验 → 用途；均出自 `src/config.rs:465-814`）

**基础/存储/身份**
| env | 默认 | 约束 | 用途 |
|---|---|---|---|
| APP_HOST | 0.0.0.0 | — | 监听地址 |
| APP_PORT | 8080 | parse u16 | 端口 |
| APP_BASE_URL | http://localhost:8080 | — | 对外 base URL |
| MONGODB_URI | mongodb://localhost:27017 | — | Mongo 连接 |
| MONGODB_DATABASE | wechatagent | — | 库名 |
| MCP_BASE_URL | http://47.108.57.147:3001 | — | 部署级 MCP server |
| MCP_API_KEY | **必填** | 缺失启动失败 | 部署级 MCP key |
| OPENAI_BASE_URL | https://api.openai.com/v1 | — | 默认 LLM base |
| OPENAI_API_KEY | **必填** | 缺失启动失败 | 默认 LLM key |
| OPENAI_MODEL | gpt-4.1-mini | — | 默认模型 |
| DEFAULT_WORKSPACE_ID / DEFAULT_ACCOUNT_ID | default / default | — | 默认租户/账号 |

**Agent 回复节奏**
| env | 默认 | 约束 | 用途 |
|---|---|---|---|
| AGENT_RECENT_MESSAGE_LIMIT | 12 | i64 | 上下文窗口条数 |
| AGENT_MIN_REPLY_INTERVAL_SECONDS | 20 | i64 | 最小回复间隔 |
| ACCOUNT_SEND_MIN/MAX_INTERVAL_MS | 1000 / 4000 | i64（0=关闭） | 账号级发送随机间隔 |
| AGENT_REPLY_MAX_SEGMENT_CHARS | 120 | ≥1 | #68 单条出站软上限，按句末标点切分 |
| AGENT_REPLY_MAX_SEGMENTS | 4 | ≥1 | 单次最多几条短消息 |
| MESSAGE_DEBOUNCE_WINDOW_MS | 2000 | clamp[1000,10000] | 连发多条去抖窗口 |
| COMPLETENESS_CACHE_TTL_SECONDS | 300 | i64 | F-013 completeness 缓存 |

**LLM/并发/worker**
| env | 默认 | 约束 | 用途 |
|---|---|---|---|
| LLM_TIMEOUT_SECONDS | 45 | u64 | reqwest 超时 |
| LLM_MAX_RETRIES | 5 | u32（client 内 max(1)） | 重试次数 |
| LLM_RETRY_BASE_MS | 1500 | u64（client 内 max(100)） | 退避基数 |
| LLM_MAX_CONCURRENCY | 4 | clamp[1,64] | 进程级 LLM 总并发 |
| LLM_FOREGROUND_RESERVED | 2 | clamp[1,64] | 前台预留（后台=total-reserved） |
| TASK_WORKER_INTERVAL_SECONDS | 30 | u64 | follow-up worker tick |
| INBOUND_REPLY_WORKER_CONCURRENCY | 4 | clamp[1,32] | 入站积压恢复并发 |
| TASK_CLAIM_TIMEOUT_SECONDS | 300 | u64 | HP-1 stale running 回收 |
| IMPORT_WORKER_INTERVAL_SECONDS | 2 | u64 | 导入 worker tick |
| IMPORT_JOB_CLAIM_TIMEOUT_SECONDS | 600 | u64 | 导入孤儿回收 |
| REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS | 60 | u64 | HP-3 reaction claim 超时 |
| POST_DECISION_WORKER_CONCURRENCY | 4 | clamp[1,32] | 发送后投影并发 |
| POST_DECISION_MAX_ATTEMPTS | 8 | clamp[1,100] | 投影认领次数上限 |
| POST_DECISION_SNAPSHOT_MAX_BYTES | 2097152 | clamp[262144,8MiB] | 冻结快照上限 |
| POST_DECISION_PROMPT_MAX_CHARS | 80000 | clamp[8000,500000] | 投影 prompt 裁剪 |
| POST_DECISION_TOKEN_BUDGET | 32000 | clamp[1000,500000] | 投影独立预算 |
| POST_DECISION_FAILED_SNAPSHOT_RETENTION_DAYS | 14 | clamp[1,365] | 失败快照保留 |
| WEBHOOK_RATE_LIMIT_WINDOW_SECONDS / _CAPACITY | 60 / 30 | u32 | webhook 限流 |

**Strategic Planner 全系**
| env | 默认 | 用途 |
|---|---|---|
| STRATEGIC_PLANNER_ENABLED | false | 总开关（回滚开关） |
| STRATEGIC_PLANNER_INTERVAL_SECONDS | 600 | 扫描周期 |
| STRATEGIC_PLANNER_SILENT_THRESHOLD_HOURS | 72 | 静默判定 |
| STRATEGIC_PLANNER_DAILY_EMIT_CAP | 20 | 每账号日 emit 上限 |
| STRATEGIC_PLANNER_COMMITMENT_IMMINENT_WINDOW_HOURS | 8 | 承诺临期提醒窗 |
| STRATEGIC_PLANNER_COMMITMENT_FALLBACK_DUE_HOURS | 72 | 无 due_at 合成兜底（0=禁用） |
| STRATEGIC_PLANNER_COMMITMENT_EMIT_DEDUP_HOURS | 24 | 同 commitment 去重 |
| STRATEGIC_PLANNER_STAGE_STAGNATION_THRESHOLD_DAYS | 14 | 阶段停滞 |
| STRATEGIC_PLANNER_STAGE_STAGNATION_RECENT_INBOUND_HOURS | 24 | 近期入站跳过 |
| STRATEGIC_PLANNER_CALENDAR_LOOKAHEAD_DAYS / _DAILY_CAP / _TZ_OFFSET_HOURS | 1 / 3 / +8 | 纪念日关怀（i32 时区固定偏移） |
| STRATEGIC_PLANNER_RENEWAL_LOOKAHEAD_DAYS / _GRACE_DAYS / _DAILY_CAP | 14 / 7 / 3 | 续费推进 |
| STRATEGIC_PLANNER_REACTIVATION_DORMANT_DAYS / _CADENCE_DAYS / _DAILY_CAP | 30 / 30 / 3 | 休眠唤醒 |
| STRATEGIC_PLANNER_BLOCK_RATE_WINDOW_HOURS / _MIN_RUNS / _THRESHOLD | 24 / 3 / 0.6 | M3 反馈环 backoff |
| STRATEGIC_PLANNER_PRIORITY_ENABLED | **true** | 跨联系人优先级排序 |
| COLD_CONTACT_WORKER_ENABLED / _THRESHOLD_HOURS / _DAILY_EMIT_CAP | false / 168 / 5 | D3 冷激活 |
| VALUE_TIER_MID/HIGH_THRESHOLD_CENTS | 50000 / 300000 | G6 价值分层（分） |

**发送治理/请示/campaign**
| env | 默认 | 用途 |
|---|---|---|
| HOLDING_REPLY_MIN_INTERVAL_HOURS | 6.0 (f64) | 链尾失联安抚限频 |
| HOLDING_REPLY_TOKEN_BUDGET | 3000 | 安抚文案独立预算 |
| ACCOUNT_DAILY_SEND_SOFT_CAP | 500 | ④ 仅告警不拦截 |
| CAMPAIGN_MAX_AUDIENCE | 500 | **硬上限；0=全拒不是不限** |
| WAKE_JITTER_MAX_SECONDS | 900 | 唤醒 jitter（0=恒整点） |

**自学习/换血/渐进档**
| env | 默认 | 用途 |
|---|---|---|
| SILENCE_SIGNAL_WORKER_ENABLED / SILENCE_THRESHOLD_SECONDS / SILENCE_SIGNAL_INTERVAL_SECONDS / SILENCE_SIGNAL_DAILY_CAP | false / 86400 / 600 / 500 | 沉默信号（恒 censored） |
| DYNAMIC_CONFIDENCE_MIN_SAMPLES | 5 | S7 止血最小样本 |
| DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED | **true** | P1 换血（真实用户反应替代 reviewer 自评；false 秒回滚） |
| BEHAVIOR_SIGNAL_METRICS_ENABLED | false | P3 采集健康度 |
| KNOWLEDGE_EXPLORATION_ENABLED / _TEMPERATURE | false / 1.0 | P4 受控探索（仅 verified 池） |
| PROGRESSIVE_TIER_ENABLED | **true** | 渐进三档 kill switch |
| REACTION_GATEWAY_PARALLEL_ENABLED | **true** | reaction 与 gateway 并行 |

**演化器 M4/C5**
| env | 默认 | 用途 |
|---|---|---|
| EVOLUTION_ENABLED | false（env 硬上限） | 演化中心 |
| EVOLUTION_TICK_SECONDS | 21600 | 6h 主循环 |
| EVOLUTION_RUN_TOKEN_BUDGET / _MAX_LLM_CALLS | 60000 / 30 | 单 tick 预算 |
| EVOLUTION_EVAL_WINDOW_HOURS / MIN_REPLAYS | 72 / 30 | cohort 窗口/门槛 |
| EVOLUTION_MIN_SEND_SUCCESS_DELTA | 0.05 | 释放门槛 |
| EVOLUTION_MAX_5GATE_HIT_INCREASE | 0.10 | 5 闸上升上限 |
| EVOLUTION_MAX_SAFETY_REGRESSION_RATE | 0.0 | #152 安全闸放松零容忍 |
| EVOLUTION_REPLAY_CONCURRENCY / _MAX_FAIL_RATE | 4 / 0.30 | shadow replay |
| EVOLUTION_THRESHOLD_RELEASE_COOLDOWN_HOURS | 24 | 同 gate cooldown |
| EVOLUTION_COHORT_PER_CONTACT_CAP / _SAMPLE_PER_FAILURE_BUCKET | 3 / 10 | cohort 去重/采样 |
| EVOLUTION_MAX_NEGATIVE_REACTION_INCREASE | 0.05 | 2.5-pre-3 仅观测 |
| EVOLUTION_AUTO_RELEASE_ENABLED | false（HC-017 代码闸恒否决） | 历史自动发布 |
| EVOLUTION_AUTO_RELEASE_WINDOW_HOURS / _PER_TICK_CAP | 336 / 1 | 预留 |
| EVOLUTION_AUTO_RELEASE_NEGATIVE_REACTION_GATE_ENABLED / _MAX_NEGATIVE_REACTION_RATE | false / 0.30 | 预留强制门 |

**知识子系统**
| env | 默认 | 约束 | 用途 |
|---|---|---|---|
| KNOWLEDGE_DIGEST_ENABLED / _RUN_HOUR | false / 9 | u32 | 日报 worker |
| KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET | 24000 | bounded[1,1e6] | 日报预算 |
| KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS | 8 | bounded[1,100] | 日报调用上限 |
| KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS | 30 | 0=停 | 知识任务 worker |
| CATALOG_REBUILD_WORKER_INTERVAL_SECONDS | 3 | 0=停 | catalog 重建 |
| KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS | 600 | 0=停 | Phase F 反馈 |
| INGEST_WORKER_ENABLED / _INTERVAL_SECONDS | false / 3600 | — | P1-6 自动摄取 |

**双 reviewer / 鉴权 / JWT / webhook / 媒体**
| env | 默认 | 用途 |
|---|---|---|
| REVIEWER_DUAL_ENABLED | false | E2 双脑（true 但缺 base_url 启动拒绝） |
| REVIEWER_SECOND_PROVIDER_BASE_URL / _API_KEY / _MODEL | 无（Option） | 第二 provider |
| REVIEWER_SECOND_PROVIDER_FORMAT | openai | 与 LlmFormat 同集合 |
| SESSION_TTL_HOURS | 168 | cookie 7 天 |
| SESSION_COOKIE_SECURE | false | 生产须 true |
| BOOTSTRAP_ADMIN_USERNAME / _PASSWORD | 无（空白过滤） | 首个 admin |
| SYSTEM_OPERATOR_USERNAMES | ""（空=全拒） | 全局 worker 熔断控制面白名单（CSV trim+dedup） |
| AUTH_RATE_LIMIT_WINDOW_SECONDS / _CLIENT_CAPACITY / _TARGET_CAPACITY / _GLOBAL_CAPACITY | 300 / 20 / 10 / 100 | 登录限流三层 |
| WEBHOOK_VERIFY_SIGNATURE | **true** | HMAC-SHA256(body, MCP_API_KEY) 校验 X-MCP-Signature |
| WEBHOOK_TIMESTAMP_SKEW_SECONDS | 300 | ±5 分钟防重放 |
| JWT_ENABLED | false | true 须配齐双 PEM 否则启动 panic |
| JWT_TTL_MINUTES | 60 | token 过期 |
| JWT_PRIVATE/PUBLIC_KEY_PEM | 无 | RS256 |
| MEDIA_STORAGE_DIR | ./media | 素材根目录 |
| MEDIA_MAX_FILE_SIZE_MB | 50 | 上传上限 |
| MEDIA_ID_CACHE_TTL_HOURS | 24 | MCP media_id 缓存 |

### 4.2 prompt key 全表（key → layer → 用途）
| key | layer | status | 用途 |
|---|---|---|---|
| user.initial_profile.system / .task | system_contract / task_template | active | 初始画像契约/schema |
| user.reply.system | system_contract | active | 回复运行时契约（红线锚×2） |
| user.reply.policy | policy | active | 模式判定树+5 闸+表达红线+标签纪律（业务锚+红线锚×2） |
| user.reply.fast.task | task_template | active | 紧凑发送决策（投影字段禁输出；锚×6） |
| user.reply.task | task_template | active | 全量单发决策（七思考链/证据链/请示/转述；锚×5） |
| user.projection.system / .task | post_decision_projection | active | 发送后投影（无发送控制权） |
| user.memory_consolidator.system / .task | memory_consolidator | active | 长期记忆整理（结构化 fact+dimension+OCEAN） |
| user.reaction.system / .task | reaction_analysis | active | 用户反应分析（7 值 outcomeStatus） |
| user.review.system | review | active | 独立评审（5 闸阈值+few-shot 锚+反接管红线） |
| user.review.light.system | review | active | 轻量评审 |
| user.review.product_claim_markers | review_guard | active | 字符串兜底 guard 标记词表（JSON） |
| knowledge.auto_verify | knowledge_integrity | active | 切片校验（sourceQuote 才可 verified） |
| eval.user_operation_judge.system | evaluation | active | shadow 回归 Judge |
| management.plan.system / .policy | system_contract / policy | active | 后台管理工具计划/风险分级 |
| management.prompt_redline_review.system | system_contract | active | 提示词编辑第三闸 judge（禁自改） |
| playbook.generator.system | methodology_generator | active | 方法论生成（行业无关） |
| group.policy / moment.policy | policy | **draft** | Phase 2 域占位 |
| knowledge.chunk.repair.propose / .followup | knowledge_repair | active | 切片修复首轮/追问合并 |
| knowledge.pack.repair.propose | knowledge_repair | active | 知识包元数据修复 |
| knowledge.chat.intent / .draft_chunk / .update_chunk / .clarify | knowledge_chat | active | 知识对话 4 分路 |
| knowledge.digest.compose / .dispatch / .summarize_logs | knowledge_digest | active | 日报合成/派工/日志摘要 |
| escalation.principal.interpret / escalation.sediment.title | escalation | active | 领导裁决解读/沉淀标题 |
| evolution_critic_v1 | critic | active | 演化 Critic（独立 pack；禁自指禁编辑） |

### 4.3 重试/退避/超时全部数值
| 参数 | 值 | 位置 |
|---|---|---|
| LLM 重试次数 / 退避基数 | env 默认 5 次 / 1500ms | `src/config.rs:500-501` |
| LLM 单次退避封顶 | 60s（Retry-After 更长则尊重） | `src/llm.rs:1361` |
| LLM 指数移位上限 | attempt-1 min 10（2^10） | `src/llm.rs:1362` |
| LLM jitter | 0..base_ms（测试=0） | `src/llm.rs:1373-1386` |
| LLM 回喂修复次数 | 2 | `src/llm.rs:290` |
| LLM reqwest 超时 | env 默认 45s | `src/config.rs:499` |
| LLM Registry TTL / provider init | 30s / 5 attempts + 5s max_commit + 20ms*attempt | `src/llm.rs:1979,1762-1763,1900` |
| MCP reqwest 超时 | 60s | `src/mcp.rs:25` |
| dispatcher send 外层 timeout / lease | 150s / 180s | `src/agent/outbox_dispatcher.rs:154,3227` |
| roster 同步内短重试 | 3 次 × 2s | `src/mcp.rs:922-923` |
| roster 后台重试 | 5 次，退避 3·2^n（3/6/12/24/48s） | `src/mcp.rs:901,909-911` |
| roster 快照过期 | 24h | `src/mcp.rs:899` |
| chat_search 时钟容忍 / limit | 5min / 100 | `src/mcp.rs:1046-1047` |
| supervisor 退避 | 1s 起 ×2 封顶 30s | `src/supervisor.rs:28-29` |
| supervisor 熔断 | 60s 窗口内 5 次 panic → open；轮询 30s；probe 锁 120s；probe 稳定期 60s | `src/supervisor.rs:30-32,98-99,215` |
| 版本分配重试（prompt/soul） | 各 8 次 | `src/prompt_template_versions.rs:16`、`src/soul_versions.rs:20` |
| media reconciler 周期 | 3600s | `src/media_storage.rs:14` |
| outbound_fetch | DNS 5s / connect 10s / request 30s / 重定向 ≤5 / body ≤4MiB | `src/outbound_fetch.rs:17-21` |
| 关键路径输出限额 | fast_reply 8192 / light_reviewer 3072 / reviewer 8192 / claim_gate 3072 | `src/agent/mod.rs:233-236` |

### 4.4 MCP 工具名全集（本批文件内出现）
| 工具 | 调用点 | 用途 |
|---|---|---|
| `initialize`（JSON-RPC 方法） | `src/mcp.rs:97` | 握手拿 mcp-session-id |
| `tools/call`（JSON-RPC 方法） | `src/mcp.rs:191,245` | 工具调用信封 |
| `tools/list`（JSON-RPC 方法） | `src/mcp.rs:322` | 工具目录 |
| `contacts_fetch_full` | `src/mcp.rs:930` | 全量好友（富化字段，status=ready 判就绪） |
| `chat_search` | `src/mcp.rs:1073` | 出站送达核对（direction/peer/content_contains/since/limit） |
| `contacts_fetch_cache` | 注释引用（`src/mcp.rs:770-774`） | 旧好友工具（result.friends 裸 wxid），代码保留解析兼容 |
| `media_upload_base64` | 注释引用（`src/mcp.rs:383-386`） | 媒体上传（base64 落日志前脱敏） |
| `message_send_text` | prompt 禁令（`src/prompts.rs:1850`）与 CLAUDE.md | 原始发文本工具，管理 Agent 禁止直接规划，走 `wechatagent.send_contact_message` 产品工具 |
| `contacts_search` / `wechatagent.search_contacts` / `wechatagent.import_contacts` | prompt（`src/prompts.rs:1853`） | 搜好友/导入 |
| `auth_whoami` | 注释引用（`src/mcp.rs:543`） | account_alias 对应关系 |

### 4.5 supervisor 熔断参数
见 §4.3 表；worker 名单 16 个见 §2.9；控制集合 `background_worker_controls`，行 id `worker::{name}`；状态集 `closed/open/half_open/probing`；事件 kind `background_worker_panic` / `background_worker_circuit_open`；恢复 API `resume_worker_circuit`（open→half_open，操作者白名单由 `SYSTEM_OPERATOR_USERNAMES` 控制——该关联在 config 注释 `src/config.rs:357-359`）。

---

## 5. 偏差与疑点

1. **supervisor 文件头注释过时**：`src/supervisor.rs:3-5` 说 "main.rs 用 tokio::spawn 拉起 8 个长驻 worker" 并列举 9 个名字，但 `SUPERVISED_WORKERS` 实际 16 个（`src/supervisor.rs:34-51`）。注释滞后于名单扩张，不影响行为。
2. **`user.review.claim_gate` 有输出限额但无 spec**：`critical_path_output_token_limit` 给 `"user.review.claim_gate"` 配了 3072 限额（`src/agent/mod.rs:243`），但 `prompt_specs()` 中没有该 key。推测 ClaimGate 的 prompt 在 `agent/review` 内部构造、只以此 key 记 llm_call_logs/限额（本批文件范围外，未核）。~~疑点：ClaimGate prompt 正文在哪定义、是否受版本治理~~ → **【主会话已核证 2026-08-13】ClaimGate prompt 是代码内嵌英文常量 `SYSTEM`（`src/agent/review/mod.rs:340-354`），`run_independent_claim_gate` 以 `"user.review.claim_gate"` 作为 prompt_key 调 `generate_agent_json`（`review/mod.rs:391-401`）——该 key 仅用于记账/优先级/限额，不经 `prompt_templates` 加载。设计含义：ClaimGate 有意置于 prompt 治理三闸与演化体系之外（`EVOLVABLE_PROMPT_KEYS` 亦无此 key），是不可被运营编辑或演化篡改的独立硬编码审查器。**
3. **LLM 精确缓存白名单 4 个 key 均不在 prompt_specs 内**：`knowledge.import.preview`/`playbook.generator`/`playbook.optimizer`/`user.guide.preview`（`src/agent/mod.rs:828-836`）与 spec key（如 `playbook.generator.system`）不同名——缓存 key 是调用方自定义的 prompt_key 记账名而非模板 key。命名两套体系并存，易混淆但行为正确（非模板 key 落到 default_prompt_content 会 NotFound，说明这些调用不走 load_prompt）。
4. **`parse_or_repair` 的"层数"口径不一**：注释多处称"三层"（快路径+repair+extract 为前两层？`src/llm.rs:288-289` 称"前两层（快路径 + repair_loose_json + extract_embedded_json）"），流式路径 `src/llm.rs:986` 又称"三层确定性解析…第四层 LLM-repair"。实际结构 = 3 层确定性 + 1 层回喂；文档口径混乱但代码一致。
5. **repair_loose_json 的"字符串内裸控制字符转义"会使无损判定失真**：输入含裸 `\n` 时函数必返回 Some（out≠input），即使 JSON 其它部分完全合法——行为正确（严格模式本就拒收），仅指出"None=无修改"的语义包含此类必然改写。
6. **mcp_logs 写失败静默**（`let _ =` 三处：`src/mcp.rs:408,452,502`）：崩溃恢复依赖 mcp_logs 的精确匹配，插入失败（如 BSON 超限之外的网络抖动）会让 `mcp_already_succeeded` 查不到成功记录。M16 已解决 base64 超限主因，剩余窗口靠 chat_search 权威核对兜底（`src/mcp.rs:1057-1096`），但该兜底仅覆盖文本（content 精确等于），媒体/名片发送若 mcp_logs 丢失仍有理论重发窗口。**疑点：媒体发送的 timeout 兜底核对路径是否有 chat_search 等价物（在 outbox_dispatcher 内，本批未深读）**。
7. **`ensure_default_llm_provider` 的选举提升不会自动跟随 config**：非空库已有 provider 行时，启动只提升既有最早行为 active，env 里 OPENAI_* 的变更不会同步进 DB（仅首次 seed 用 config）。这是"DB 为真相"的刻意设计，但意味着改 .env 的模型对已初始化库无效——需通过管理 UI 改 provider。属行为确认非缺陷。
8. **Anthropic max_tokens 硬顶 8192**（`src/llm.rs:505,760`）：新模型支持更大输出也被 clamp；长生成任务（如 domain_profile）在 Anthropic 形态下截断风险高于 OpenAI 形态（后者不传 max_tokens 时用 provider 默认）。
9. **`fetch_raw_text`（repair 路径）不带重试循环**：修复请求单次失败即计一次 attempt（`src/llm.rs:418-426` 循环体只重复调用 2 次，但每次内部无 HTTP 重试）——与主调用的 max_retries 语义不同。修复失败成本低（还有第 2 次+最终报错），可接受但与"复用 HTTP 链路"的直觉不同。
10. **soul reset 的 draft 特例**：`reset_builtin_souls` 对 status=draft 的 spec（group/moment）在已有流上仍 `append_version`（seeded_by=system_reset）但不 publish（`src/soul_versions.rs` 无此逻辑，是 `src/prompts.rs:956-973` 的 `if spec.status == "published"` 条件）——draft soul 的 reset 会累积未发布版本行。轻微版本膨胀，无行为影响。
11. **outbound_fetch 的 IPv6 白名单未排除 64:ff9b::/96（NAT64）**：v4-mapped（::ffff:x）已处理，但 NAT64 前缀 64:ff9b:: 不在 2000::/3 内故天然被拒（fail-closed 白名单已覆盖）——确认安全，非疑点，记录推理过程。
12. **config.rs 两个重复测试**：`system_operator_csv_is_trimmed_deduplicated_and_empty_by_default`（`src/config.rs:894-901`）与 `system_operator_identities_are_trimmed_deduplicated_and_fail_closed`（`src/config.rs:916-923`）内容完全相同。无害冗余。

---

## 6. 覆盖自证

| 文件 | 总行数 | 读取方式 | 覆盖 |
|---|---|---|---|
| src/llm.rs | 3388 | 分 4 段（1-850 / 851-1730 / 1730-2609 / 2609-3388） | 100% |
| src/llm_concurrency.rs | 197 | 一次整读 | 100% |
| src/mcp.rs | 1924 | 分 2 段（1-1000 / 1000-1924） | 100% |
| src/config.rs | 924 | 一次整读 | 100% |
| src/prompts.rs | 3096 | 分 4 段（1-800 / 800-1600 / 1600-2400 / 2400-3096），全部 prompt 正文逐字读 | 100% |
| src/prompt_guard.rs | 484 | 一次整读 | 100% |
| src/prompt_template_versions.rs | 441 | 一次整读 | 100% |
| src/soul_versions.rs | 478 | 一次整读 | 100% |
| src/supervisor.rs | 538 | 一次整读 | 100% |
| src/secret.rs | 65 | 一次整读 | 100% |
| src/error.rs | 203 | 一次整读 | 100% |
| src/media_storage.rs | 684 | 一次整读 | 100% |
| src/outbound_fetch.rs | 476 | 一次整读 | 100% |
| **合计** | **12,898** | | **100%** |

跨文件亲验（非本批但为核证引用而读的片段）：`src/agent/mod.rs:230-456`（generate_agent_json 全文）、`src/agent/mod.rs:804-857`（llm_exact_cache_key）、`src/db/config_generation.rs:1-108`（全文）、`src/agent/outbox_dispatcher.rs` 中 `MCP_SEND_TIMEOUT_SECONDS`/`DEFAULT_LEASE_SECONDS`/时序不变量测试片段（grep 定位后读取）。
