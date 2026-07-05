# 全项目深度冒烟 findings（2026-07-05，部署 59d84b5 = main #118 + dispatcher 修复）

## 环境/基线（GREEN）
- 部署成功：server 117 checkout `59d84b5`，重启后新进程起，boot 日志确认 dispatcher 修复生效（`lease_seconds=180`，原 60）。localhost/127/公网自 curl 均 200。
- 前端 vitest：106 文件 / 448 测试全绿（本地，含 #113-#118 新增 label 测试）。
- 后端 lib：`cargo test --lib` 1814 passed / 0 failed；dispatcher 不变量测试 `send_timeout_covers_worst_case_mcp_calls_and_stays_below_lease` 通过。

## 只读 API 冒烟（GREEN，不依赖 LLM，2026-07-05 对 live 部署 59d84b5）
26 个非 LLM GET 端点全部 HTTP 200 + 合法 serde JSON（server 端 localhost 直打，绕过公网 502）：
- 鉴权链路：`/api/auth/login` 200、`/api/auth/me` 返回 admin/workspaces。
- 核心资源读：accounts(584B)/events(17KB)/llm-usage(46KB)/decision-reviews(63KB)/operation-domains(10KB)/operation-knowledge/usage(8.8KB)/evolution/experiments(8.2KB) 全返真实 Mongo 数据。
- ask-human 收件箱：inbox(47KB) + summary（principalEscalation:0 / knowledgeReview:17 / taxonomyCandidate:106）。
- observability：phase-rollup / worker-health / outcomes/autonomy 全 200。
- 空集合正确返 `{"items":[]}`（contacts/tasks/agent-runs/products/suspected-deals/outbox…，无 500、无 serde 崩）。
**意义**：新 build（59d84b5，含 dispatcher 修复）+ 真实 Mongo + auth + serde 反序列化端到端联通，这是 vitest（纯前端）和 lib 测试（进程内）都覆盖不到的一层。

## BLOCKED（外部依赖，非项目 bug）—— 后端业务逻辑冒烟当前无法进行

**三家 LLM provider 全部不可用（2026-07-05 ~01:00 CST，逐一 provider-test 端点串行验证 retries=1）：**
| provider | baseUrl | 结果 | 判定 |
|---|---|---|---|
| rsxermu-claude-opus-48（active 主模型） | rsxermu666.cn | HTTP 503 "Service temporarily unavailable"（retryCount:0，连测两次均 503） | 平台侧全面 outage，非 2 线程并发争用 |
| nvidia-deepseek-v4-flash | integrate.api.nvidia.com | HTTP 503 "ResourceExhausted: Worker local total request limit reached (62/48)" | NVIDIA 配额/worker 限流打满 |
| default（aliyun qwen3.7-max） | dashscope.aliyuncs.com | HTTP 400 "Arrearage / Access denied...good standing"（overdue payment） | 阿里云账号欠费 |

**铁证**：近 90min `llm_call_logs` = 0 success / 0 cache / 10 failed；0 run log 落库。domain2 跑 31min 仍失败于"run log 未落"= 端点 503 级联，**非项目 bug**（domain2 断言 critical 是端点噪声）。

**结论**：后端业务逻辑深度冒烟（webhook→决策→审查→outbox，每轮必调 LLM）在任一 provider 恢复前无法进行。已停 server 端 setsid 套件（避免烧时间产假 critical 噪声）。需用户：切一个可用 provider，或等主端点恢复后复跑。

**其它 BLOCKED（正交，同样卡端到端）：**
| 项 | 现象 | 判定 |
|---|---|---|
| 外部 MCP server | `47.108.57.147:3001` TCP 超时（独立于 117） | 管理 Agent（批C management）/账号工具域会 upstream_error → BLOCKED，等 MCP 恢复复跑 |
| 公网 HTTP 到 117:3003 | 外网访问返 502，但 localhost/127/服务器自 curl 均 200，服务器无 nginx/caddy 反代 | 云边缘/安全组层问题，非应用；阻断本机 Playwright 前端 E2E |

## Findings（真实缺陷）
| 域 | 现象 | severity | 根因 | 证据 |
|---|---|---|---|---|
| 可观测性 | 失败的 llm_call_logs 记录的 `model` 是 `.env` 里的陈旧标签 `deepseek-ai/deepseek-v4-flash`，而非实际 active provider（claude-opus-4.8）。调试端点故障时会误判"是 deepseek 在失败" | low | 成功路径写 `result.model`（真实模型，agent/mod.rs:287），但失败路径写 `state.config.openai_model`（陈旧 .env 值，agent/mod.rs:443）。`.env` 仍 `OPENAI_MODEL=deepseek-ai/deepseek-v4-flash`（DB active provider 与 .env 解耦，热切后 .env 未更新） | 失败日志 6 条全 deepseek 标签 + 全 http_5xx；同期成功日志全 claude-opus-4.8 |
| 用户运营·影子验证 | **前端「影子验证」功能从 UI 完全不可用**：点「开始验证」恒返 HTTP 400 `"messages are required"`。 | **HIGH（已修 6822ffb）** | 前后端字段契约漂移。store `runDialogueSimulation`（userOpsStore.ts:787-796）发 `{inboundText, runMode, dryRun}`；后端 `UserDialogueSimulationRequest`（simulations.rs:19-26，camelCase）只读 `{messages:Vec<String>, applyMemory}`，二者均 `#[serde(default)]` → `messages` 恒空 → handler 早退 400。`inboundText/runMode/dryRun` 后端完全不读（runMode="shadow"/applied=false 硬编码返回）。 | 本地 e2e `deep_simulation.mjs`：(A) 原前端 payload → 400；(B) `{messages:[...]}` → 200 + 1 turn。修复=store 按换行 split 成 `messages[]`（UI placeholder 已写"每行一条用户消息"），tsc --noEmit 绿。 |
| dispatcher | `MAX_SEQUENTIAL_MCP_CALLS_PER_SEND` 常量非 test 构建 dead_code 警告 | trivial（已修 0149abd） | 该常量只在 `send_timeout_covers_worst_case` 不变量测试引用，非 test 构建 cargo check 报 dead_code | 加 `#[cfg(test)]` 门；运行时行为不变，不变量测试仍 1 passed |

## LLM 依赖用户运营流程本地深测（GREEN，2026-07-05 本地栈 haiku-4.5 真调）
逐一 UI→handler→LLM→Mongo 三方核对，全部端到端跑通：
| 流程 | 端点 | 结果 |
|---|---|---|
| 生成初始画像 | `POST /contacts/:id/analyze-profile` | 稀疏 note→LLM 拒绝臆造→503 fail-closed（非 bug）；富 note 经 `profile-note` → agent_profile+profile_attributes 落库，operation_state=new_contact |
| 长期记忆固化 | `POST /contacts/:id/memory-consolidation/run` | 1 候选→6 core+2 recent facts 落库，pending 归零，consolidator success |
| 方法论优化 | `POST /operation-playbooks/:id/optimize` | version 1→2，created_by system_v3→agent_optimized，全字段重生成 |
| 运营引导预览→应用 | `POST /user-operations/guide/preview` + `/apply` | preview pending→applied，11 字段应用，event 落库，改动经 dimension-registry 校验（fail-soft 放行未配置字典）|
| 行业画像生成 | `POST /admin/domain-profiles/generate` | 候选 profile 落 draft（is_active=false/seeded_by=generated_by_ai）；haiku 少产结构化数组属模型能力非 bug，人审门拦截 |
| 核心 webhook 管线 | `POST /webhooks/wechat` | 空知识库定价问询→blocked_unverified_product_claim（红线正确）+ 兜底安抚占位（intended）|

## 产品红线双向验证（capstone，GREEN）
知识 `chat → apply → verify` 三步造出一条**已核实**定价切片后重跑 webhook 定价问询，闸门判定从
`blocked_unverified_product_claim`（kcov=missing / knowledgeGroundingScore=4 / 0 chunk）
→ `blocked_by_required_field`（kcov=enough / knowledgeGroundingScore=**10** / hallucination=0 / 选中 1 chunk）。
**证明已核实知识确实解锁产品说法红线**（路由检索到并 grounding 到该切片）。新命中的
`blocked_by_required_field` 是 R3.5/R3.6 结构性协议闸（`has_protocol_violation`，gates.rs:559-582）——
haiku 少产必填决策字段被硬拦，属"弱模型漏产结构化输出"模式，fail-closed 正确，非 bug。
过程中两次撞 abc-tunnel HTTP 530（Argo Tunnel origin unregistered，瞬时抖动，直连 200 复测确认端点本身健康）。

## 非 LLM 写路径契约审计（GREEN）
subagent 交叉核对 4 store（strategy/content/referralCard/command）+ 手核 4 feature 组件（products/suspected-deals/outbox/quality）共 ~30 个 POST/PUT 站点：**除已修的 simulation 外零字段契约漂移**（全部 `#[serde(rename_all="camelCase")]` 覆盖，Option/default 正确）。content-asset CRUD + taxonomy 候选 approve/reject 已实测落库正确。
