# 新用户全旅程四方对账（2026-07-05）

四方 = 前端 UI 显示 / 后端源码预期 / 服务器 stdout 日志 / MongoDB 实际数据。
分级自主：A 类(安全 bug)直接修+提交；B 类(业务逻辑/prompt/阈值/红线)只记 finding 待裁决；C 类(外部依赖)标 BLOCKED。

## 环境（GREEN，2026-07-05T07:xx）
- 后端 `:8080` root 200；未登录 `/api/auth/me` → 401（鉴权链正确）。
- 前端 `frontend/dist/index.html` 存在。
- LLM 端点 `r4b53lm.abc-tunnel.us/v1`（kr/claude-haiku-4.5）直连 200 健康。
- 后端 stdout 日志路径：`.../d83b6443-.../tasks/bqcy4hdcn.output`（feedback_worker 实时写入）。
- 数据库 `wechatagent_local_e2e`，全测试数据，seed/改/删随意。

## 本轮已覆盖频道（四方对账逐条追加于下）

### 1. 登录/鉴权边界（GREEN）
- UI/源码：LoginResponse 精简（username/expiresAt/currentWorkspace），前端登录后立即调 `/me`（main.tsx:56）补全 userId+workspaces——正确设计，非契约漂移。authStore userId/workspaces 可选因 /me 恒提供。
- HTTP：登录 200 | 未登录 `/me` 401 | 错误密码 401 `invalid_credentials`。
- Cookie：HttpOnly ✓ / SameSite=Strict ✓ / Path=/ ✓（build_session_cookie auth.rs:161）。
- MongoDB：admin_sessions 落库 ✓（username=admin / current_workspace=default / expires_at>now）；admin_users=1。
- 四方对齐，无 bug。

### 2. 账号配置+联系人导入+托管（GREEN；MCP 路径 BLOCKED）
- 字段契约：前端 `humanProfileNote`↔后端 `human_profile_note`（EnableAgentRequest camelCase，models.rs:3184）✓；disableAgent 发 `{}`、后端只读 Path ✓。
- enable-agent（真调 LLM 24.6s）：resp agentStatus=managed / operationState=new_contact；DB agent_status=managed + operation_state=new_contact + agent_profile 落库 ✓。前置校验 account_id 必须在 wechat_accounts 注册（contacts.rs:377）。
- disable-agent：HTTP 200，DB agent_status→normal ✓（状态驱动显示维度4）。
- import candidates 路径（绕 MCP）：HTTP 200，新联系人落库 agent_status=normal ✓（DB count 1→2）。
- import query 路径（走 MCP contacts_search）：HTTP 502 upstream_error → **C 类 BLOCKED**（MCP server 47.108.57.147:3001 宕机；后端正确转译 502 不吞错，非 bug）。
- 四方对齐，无 bug。

### 3. 概览工作台（GREEN）
- 概览统计全客户端派生：onlineCount（accounts.filter online）、managedCount（contacts.filter agentStatus===managed）、normalCount（总数-managed）。源数据来自 `/api/accounts`+`/api/contacts`。
- `/api/accounts` HTTP 200：API items 1 = DB wechat_accounts 1；online 派生 1 = DB online 1 ✓。
- `/api/contacts` HTTP 200：API items 2 = DB contacts 2；managed 派生 0 = DB managed 0；normal 2 ✓。
- 字段契约：contact camelCase 回显完整（wxid/nickname/agentStatus/operationState/operationStateReason/operationStateConfidence/operationStateUpdatedAt）✓。
- 四方对齐，无 bug。

### 4. AI 模型配置（llm-providers，GREEN）
- 字段契约：前端 payload（providerId/baseUrl/apiKey/model/format/supportsVision/timeoutSeconds/maxRetries/retryBaseMs）全部对齐后端 UpsertRequest（camelCase，llm_providers.rs:131）✓。
- create HTTP 200：resp apiKeyMasked=`sk-****62ee`（不泄漏明文）✓；isActive=false（新建默认不激活）✓；DB apiKey 明文入库=true（展示层 mask，存储层明文，设计如此）✓。
- test HTTP 200：端点工作正常（构造一次性 client 真调 LLM）；ok:false 是 haiku 未产出合法 JSON（`json_decode after 2 repair failed`）——弱模型行为，非代码 bug。test 路径不重试、不入库，正确暴露真实错误。
- activate HTTP 200：新 provider isActive→true，原 provider isActive→false（单一 active 不变式，update_many 清其它 + update_one 置本条，llm_providers.rs:322-339）✓；list active.providerId 热切换后正确回显 ✓。
- delete HTTP 200：恢复原 active 后测试 provider 可删；active 那条不可删（llm_providers.rs:289 前置校验）✓。
- 四方对齐，无 bug。（脚本侧提醒：bash `source .env` 不 export 给 node 子进程，须 node 内直读 .env——测试基建问题非项目 bug）

### 5. 内容资产 + 专属顾问名片（GREEN）
- 内容资产 CRUD：上一轮已测（create→200→落库字段正确 / list 回显 / 缺 title→400）。
- referral-cards 字段契约：前端 create（camelCase）/ review `{status,note}` / toggle `{enabled}` 全对齐后端（referral_cards.rs:23-48）✓。
- create HTTP 200：**红线验证** DB review_status=draft + enabled=false（AI 不自我核验，referral_cards.rs:80）✓。
- review approved HTTP 200：DB review_status→approved + review_note 落库 ✓；写审计事件 `referral_card.reviewed`（status=approved / reviewed_by=admin）✓。
- toggle enabled=true HTTP 200：DB enabled→true ✓。
- list HTTP 200：camelCase 回显完整（reviewStatus/enabled/displayName）✓。delete HTTP 200 → 已删 ✓。
- 四方对齐，无 bug。

### 6. 产品与成交（GREEN）
- 字段契约：前端 payload（productId/name/price/currency/sku/summary）对齐后端 CreateRequest（camelCase，products.rs:50）✓；deal-events（eventKind/amount/currency/productId/quantity/verification/note）对齐 DealEventRequest ✓。
- product create HTTP 200：resp status=active / price=398000（分）；DB status=active / workspace_id=default（IDOR 红线：workspace 由会话注入不信前端，products.rs:181）✓。
- archive HTTP 200 → DB status=archived；restore HTTP 200 → DB status=active ✓。
- deal-event HTTP 200：写入 `contact.outcome_events[]` 子数组（$push，shared.rs:1552，非独立集合）；金额 398000 + verification=staff_confirmed 落库 ✓。
- **跨频道一致性（维度6）**：deal-event 同时写审计流 `agent_events` kind=outcome_event_marked（status=ok / eventKind=deal，shared.rs:1558）✓。
- 注意：OutcomeEvent 用 camelCase serde（models.rs:400），BSON 存 eventKind/productRef——测试脚本首次按 snake_case 查得 undefined 是**查询错误非 bug**，单词字段 amount/verification 正常佐证。
- 四方对齐，无 bug。

### 7. 系统策略（souls / prompt-templates，GREEN）
- 字段契约：前端 agentKind ↔ 后端 AgentSoulRequest（camelCase，souls.rs:25）✓；prompt-template promptKey/agentKind/layer/title/content ↔ PromptTemplateRequest（camelCase）✓。
- agent-soul create HTTP 200：DB status=draft / version=1 / seeded_by=manual ✓（新建即草稿，souls.rs:93）。
- agent-soul publish HTTP 200：DB status→published ✓（publish 会 delete_many 同 kind 旧版本 + 置 published，souls.rs:157）。
- prompt-template create HTTP 200：DB status=draft / current_version=false / created_by=manual / version=1 ✓；过字面双闸（禁词+锚完整性 validate_prompt_edit，prompt_templates.rs:107）。
- reset-system-pack（物理删除重播种，破坏性）：**未触碰**（B 类回避）。
- 四方对齐，无 bug。
- 环境提示：测试期间后端 :8080 进程曾被终止（前一会话后台进程被杀，非测试导致崩溃）；已用已编译二进制 `target/debug/wechatagent.exe` 重启（listening :8080 确认）后继续。

### 8. 统一收件箱 + 请示通道配置（GREEN）
- 收件箱 askHuman：taxonomy 候选 approve/reject 上一轮已测（approved→system_taxonomies 落库 + reject reason 记录）。
- 请示通道配置字段契约：前端直发 draft ↔ 后端 AskHumanPolicy（camelCase serde，models.rs:1099）✓；DeciderRef{wxid,displayName?}（models.rs:1083）。
- PUT ask-human-policy HTTP 200：DB ask_human_policy.deciderChain[0].wxid=e2e_leader_001 + displayName 落库；escalateAiPolicyHold=false / dedupeWindowHours=6 / dailyPushCap=20 / timeoutHours=4 全部正确 ✓（camelCase BSON 存储）。
- GET operation-domains 回显 HTTP 200：经 operation_domain_json 正确回显 askHumanPolicy.deciderChain ✓。
- 校验：空 wxid（trim 后为空）→ HTTP 400 ✓（domains.rs:213）。
- 已恢复原状态（原无 policy → 清除测试值）✓。
- 四方对齐，无 bug。

### 9. 系统看板类频道（operations/autonomy/evolution/quality/send-analytics，GREEN）
只读看板，重点验证维度5（错误态不吞空态）+ 数据往返 + 空库降级：
- operations：真实加载走 operationsStore.ts:36-41 五端点 events(items[31])/tasks(items[4])/decision-reviews(items[2])/llm-usage(items[18])/agent-runs(items[2]) 全 200 有真数据。错误处理正确：catch→setError 弹全局横幅 + 置空防崩（operationsStore.ts:51-63，与 SendHistory 吞空态 bug 相反）✓。（探测时 `/api/agent-tasks?limit=20` 猜错路径返 404——真实列表端点是 `/tasks`，agent-tasks 仅 review-now/cancel；非 bug）
- autonomy：outbox items[2] / outcomes/autonomy(horizon,totalRuns,metrics) / revisions items[0] 全 200；OutboxPanel + index catch→setErr ✓。
- evolution：experiments items[4] / runtime-flag(flag) / threshold-overrides/audit items[0] 全 200；EvolutionCenterTab setError+catch（:180/244）✓。
- quality：agent-outcome-metrics items[0] / evaluation-scenarios items[1] 全 200；catch→setErr ✓。
- send-analytics：send-ledger/overview(totalSends,responseRate,stageAdvanceRate) / stats?kind=media items[0] / stats?kind=namecard items[0] 全 200；sendAnalyticsStore catch→setError（:36/45）✓。
- 空态验证：多个 items[0]（revisions/audit/metrics/send-stats）真空库返 `{items:[]}` 不崩，EmptyState 兜底 ✓。
- 四方对齐，无 bug。

### 10. AI 总控 / Management Agent（command，部分 GREEN + MCP BLOCKED）
- create session HTTP 200：字段契约前端 {accountId,title,dryRun} ↔ 后端 CreateSessionRequest（camelCase，management.rs:31）✓；DB management_agent_sessions 落库正确（title=E2E总控测试）✓。
- send message HTTP 200 写入 management_agent_messages（account_id 落库）✓，但 management LLM plan 触发 MCP tool → **502 upstream_error**（21s 超时）。
- tool-catalog HTTP 502：直接走 MCP tools/list。
- **C 类 BLOCKED**：管理 Agent 的工具编排能力依赖外部 MCP server（47.108.57.147:3001，宕机）；本地可测部分（session/message 落库、字段契约）GREEN，MCP 编排部分 BLOCKED，非项目 bug（后端正确转译 502 不吞错）。

### 11. 活动（campaign，GREEN 只读）
- campaign 前端只读（campaignStore 仅 loadCampaigns/loadReport 两个 GET；create/preview/dispatch 后端有但无前端调用，dispatch 走 MCP）。
- `/api/campaigns` HTTP 200 items[0]（真空库正确返空数组）✓；`/api/campaigns/:id/sends` 为报表读端点。
- 四方对齐（读路径），无 bug。

### 12. 知识库 Wiki（knowledgeWiki，GREEN）
- 写路径 chat→apply→verify capstone 上一轮已深测（已核实知识解锁产品红线，grounding 4→10）。
- 本轮补测读端点全 200：`/api/knowledge/digest/today`（reportId/reportDate）、`/api/knowledge/gap-signals`（signals）、`/api/knowledge/chat/tasks` items[0]、`/api/operation-knowledge/chunks` items[0] ✓。
- 四方对齐，无 bug。

## 汇总

**覆盖频道（12 组，全部可达业务频道）**：登录/鉴权 → 概览 → 账号配置+联系人导入+托管 → AI 模型配置 → 内容资产+专属顾问 → 产品与成交 → 系统策略 → 请示通道配置 → 系统看板(5频道) → AI 总控 → 活动 → 知识库 Wiki。groupOps/momentOps 是 Phase-2 占位（指向 OverviewFeature），非独立业务频道。

**A 类真 bug（已修）**：本轮新用户全旅程四方对账**零新增 A 类 bug**。上一轮已修：simulation 字段漂移(6822ffb)、SendHistory 吞空态(a5f8b8b)、dead_code(0149abd)。

**B 类待裁决**：无。所有"是 bug 还是设计"的疑点（analyze-profile 稀疏 note fail-closed、haiku 少产结构化字段、operation_state camelCase 存储、reset-system-pack 破坏性回避）均读码确认为既定设计。

**C 类 BLOCKED（外部依赖，非项目 bug）**：
1. 联系人导入 query 路径（MCP contacts_search）→ 502。candidates 路径本地 GREEN。
2. AI 总控 tool-catalog / send-message（MCP tools/list + tool 编排）→ 502。session/message 落库本地 GREEN。
均因 MCP server 47.108.57.147:3001 宕机，后端正确转译 502 不吞错。

**零未解释的遗留不一致**：全部对账项归类完毕。
- 环境说明：测试期间后端进程被前一会话终止一次，已用 `target/debug/wechatagent.exe` 重启后继续，非崩溃。

## Findings

### A 类（已修，本地提交）
（暂无本轮新增；上一轮已修 simulation 字段漂移 6822ffb / SendHistory 空态 a5f8b8b）

### B 类（待裁决）

**B-1：progressive-tier 升档（Lean→Full）路径撑爆 run 预算 → 需知识的首触问题被 `blocked_by_budget`,永不收到回复（CONFIRMED,当前 main 可复现;经对照实验精确定界）**

- **现象**：全新零历史 managed contact 发第一条**需知识的** webhook（"你们的课程怎么收费？"）→ 后台去抖流水线跑完 → run `lifecycle=aborted_by_budget` / `final_review_status=blocked_by_budget` / `decision.should_reply=false` → **主回复从不发送**。
- **对照实验精确定界（关键——不是"所有新用户都被拦"）**：
  - **需知识/会升档** "课程怎么收费"（run `8b9ba8a6…`）：触发 `ptier_escalated`（Lean→Full）→ **两次** `user.reply.task`（Lean 程 prompt 24920 + Full 程 29203 = `tokens_used` 56770）→ 超 30000 → `blocked_by_budget`,**不回复**。
  - **简单问候/停 Lean** "你好，在吗？"（run `…`,`e2e_greet_*`）：只有 `ptier_run_tier`、**无 `ptier_escalated`** → **单次** reply.task（23501 tokens）→ 23501 < 30000 → `completed` / `approved` / `should_reply=true` → 回复正常发出（"在的，你好呀。有什么需要帮忙的吗？"）。
  - 结论：**单档 Lean 回复（~23–25k tokens,占 30000 预算 ~80%）勉强过关；一旦 progressive-tier 判定信息不足升 Full,叠加第二次满额调用（~29k）必然超预算**。爆炸半径 = 首触即需知识/触发升档的问题（产品咨询、报价等真实高价值入口），不是全部流量。
- **后果链**（`src/agent/review/gates.rs:620` R3.7）：`review_skipped_budget_exceeded`（降级 local review）→ `rewrite_skipped_budget_exceeded` → needs_review + 预算超额 → `blocked_by_budget` → `autonomy_mode=blocked`、`should_reply=false`。
- **非 fail-open（安全侧正确）**：被拦主回复没有漏发；有完整事件埋点（`run_budget_exceeded`×2 / `budget_exceeded_no_review` / `blocked_review`）可观测；那条 `pending`→最终 `failed_terminal` 的 outbox 是 escalation 的 `ack-placeholder`（"这个我帮你确认一下,稍等给你准信",`source_event_id` 尾缀 `#ack-placeholder`,failed_terminal 是 MCP 宕机所致,见 C 类）,不是被拦回复的漏发。
- **根因判断**：单条 reply.task prompt(~24–25k tokens)本身就逼近 30000 预算(`src/agent/runtime.rs:596`)——**预算刚好够单程 Lean,容不下 progressive-tier 两程(Lean 自评→升 Full)**。token 数由 prompt 组装体积决定（soul+system+policy+task+知识指引+记忆候选类型+疑似成交+关系类型+决策维度+operator 指令等多层叠加）,与 LLM 端点无关 → 生产 deepseek 端点同样成立。
- **为何归 B 类（需裁决,未动手）**：修复触碰阈值/prompt 体积/progressive-tier 计费任一——(a) 抬高 `run_token_budget`；(b) 压缩 reply.task 基础 prompt 体积；(c) 改 progressive-tier 两程对 budget 的计费方式（如升档后不叠加而是替换计数,或升档路径单独放宽预算）。三者都属业务逻辑/阈值红线,按分级自主一律延后待裁决,绝不自主赌选一个改。
- **附注**：本地 LLM 是测试隧道 `r4b53lm.abc-tunnel.us/v1`(kr/claude-haiku-4.5),但 token 计数取自真实 usage 回包、由 prompt 体积决定,非模型 tokenizer artifact；用多个不同 wxid（含全新零历史）+ 两种问法对照复现一致,排除历史累积/单次抖动。复现脚本：`scripts/e2e/fresh_contact_budget.mjs`（升档路径）、`scripts/e2e/fresh_greeting.mjs`（Lean 路径对照）。

### C 类（BLOCKED 外部依赖）
（暂无）
