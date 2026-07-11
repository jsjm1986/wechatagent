# 深度审查批 A（自动回复命脉链）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **注意：这是审查工程，不是写功能。** 每个 task 的"deliverable"是**台账里带 file:line 证据的 findings**（+ 能真跑的 117 复现结果），不是新代码。审查阶段**只入账、不改任何 src**。没有 TDD 红绿循环——用"读码建图→逐条对照 spec 审→亲验→（可复现的）117 真跑→写入台账 section"替代。

**Goal:** 把自动回复命脉链（webhook→去抖→gateway 闸→决策→review→outbox→MCP 发送→回写）逐环节读透、对照 spec 审业务逻辑正确性，产出带 file:line 证据、经主控亲验的 findings 台账；能在 117 安全复现的实证为 CONFIRMED。

**Architecture:** 8 个审查 task 对应链路 8 环节，每 task 输出台账一个 section。审查全程只读；发现只入账不修。可并行派 opus subagent 分段只读审，主控逐条亲验再入账。真跑复现集中在 Task 9（gateway 闸 + 阈值闸 + outbox 幂等）。Task 10 汇总台账 + 优先级。

**Tech Stack:** Rust/Axum 后端；MongoDB；MCP JSON-RPC；OpenAI 兼容 LLM。审查工具 = Read/Grep + 117 paramiko 真跑。

## Global Constraints（逐字来自设计文档，每个 task 隐含遵守）

- **只入账不改 src**：审查阶段禁止修改任何 `src/` 代码。发现 ≠ 修复。
- **引用必亲验**：每条 finding 的 file:line、每个"某函数/字段/闸这样"的断言，当场 Read/Grep 亲验，不靠记忆/memory 旧描述。
- **subagent 结论必亲验**：派 subagent 只读审，指令硬要求先读懂再断言、产出带 file:line 证据；主控逐条亲验后才入账，凭猜产出打回。subagent 一律 opus（harness 拒 model:"opus" 时省略参数继承主会话）。
- **117 真跑硬约束**：真发只对吴界 `wxid_ydzaomn4scsb12` + AI应用开发 `wxid_czpvyjvhzizj22`（账号 102）；绝不与套件并发（端点 2 线程，串行）；webhook 带方案 B 签名 HMAC-SHA256（`<ts_ms>.`+raw_body，头 `x-webhook-signature: sha256=<hex>`+`x-webhook-timestamp`）；一律 paramiko `scripts/_remote_run.py`（DEPLOY_HOST=117.72.54.28/PORT=22/USER=root/PASS/PYTHONUTF8=1/MSYS_NO_PATHCONV=1），绝不系统 ssh；造的数据必清、联系人状态不乱改；凭据不回显值。
- **防假绿**：端点/MCP 失败标 BLOCKED 不算过；真跑拿到真实输出才算数；无法在生产安全构造的标 PLAUSIBLE + 说明原因。
- **权威依据**：`.kiro/specs/agent-autonomy-loop/requirements.md`、`docs/agent-policy.md`、CLAUDE.md「Hard rules baked into the code」。

## 已亲验的链路锚点（实现者信赖，均已 Read/Grep 确认）

- ① `src/webhooks.rs:287` `wechat_webhook`（入口）；`:333` `webhook_verify_signature` 块；`:590` `managed` 判定；`:130` `run_debounce_pipeline`；`:205` `handle_managed_message_aggregated`；`:270` `reload_managed_contact`。
- ③ `src/agent/gateway.rs:616` `run_user_operation_gateway`；`:999` `run_user_operation_gateway_inner`（巨型闸函数）。
- ⑤ `src/agent/review/gates.rs:20` `review_passed`；`:115` `classify_dual_gate`。阈值是 **runtime 可配值**（`runtime.fact_risk_block_at` / `human_like_rewrite_below` / `pressure_risk_block_at` / `product_accuracy_block_below`），非硬编码——审查须核默认值来源与临界比较符（`>=` vs `<`）。
- ⑥ `src/agent/outbox.rs` + `src/agent/outbox_dispatcher.rs`。
- ⑦ `src/mcp.rs:160` `call_tool_with_key`；`:196` `isError` 检查。

## 台账文件（Task 1 创建，后续每 task 追加 section）

`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`

每条 finding 固定字段：
`[A-NNN] 一句话标题 | 入口频道 | 链路环节 | 类型(逻辑正确性|竞态|幂等|错误处理|红线|越权|一致性) | 严重度(Critical/High/Med/Low) | 复现步骤 | 现象 | 根因(file:line 亲验) | 验证状态(CONFIRMED 真跑|PLAUSIBLE 读码) | 修复建议 | 状态(Open)`

---

## Task 1: 建台账骨架 + 环节①webhook 入口/签名/managed 门审查

**Files:**
- Create: `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`
- 审查(只读): `src/webhooks.rs`（`wechat_webhook`:287 起、`verify_webhook_signature`、`:590` managed 门、`:441` 领导分流、inbound 落库）

**Deliverable:** 台账文件建好（头部 + finding 字段说明 + "## 环节① webhook 入口" section），环节①的 findings 入账（含 0 finding 也要写"本环节亲验无问题"）。

- [ ] **Step 1: 建台账骨架**

创建 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`，写入标题、single-source 声明、finding 字段格式说明（见上）、配套 spec/plan 链接、批 A 8 环节的空 section 占位（`## 环节① … ⑧`）。

- [ ] **Step 2: 读透 webhook 入口全路径**

Read `src/webhooks.rs` 完整 `wechat_webhook`（:287 到函数结束）。逐条问：
  - 签名校验（:333）：`WEBHOOK_VERIFY_SIGNATURE=false` 时是否完全跳过？时间戳 skew 边界（300s）比较符正确？重放窗口？签名失败返回码（应 400 非 500）？
  - 账号解析（`resolve_account_context`）：appId→账号找不到时行为？回落 default 账号是否可能张冠李戴（对照 memory 账号错配家族）？
  - 领导分流（:441）：领导同时是某 contact 时的分流顺序，是否可能漏落库或双触发？
  - managed 门（:590）：`agent_status != Managed` 只落库不回复——判定读的是刚 reload 的最新态还是旧态？
  - inbound 落库与触发 agent 的顺序：落库失败是否吞掉？重复 msgId 幂等？

- [ ] **Step 3: 对照 spec 核 webhook 契约**

Grep `.kiro/specs/agent-autonomy-loop/requirements.md` 中 webhook / 签名 / managed 相关条款；对照实现找偏差。亲验每个断言的 file:line。

- [ ] **Step 4: 环节①findings 入账**

把 Step 2-3 发现写入台账"环节①"section，每条带 file:line + 验证状态（读码=PLAUSIBLE，真跑候选标注"可 117 复现"留给 Task 9）。无问题也明确写"亲验通过：<核了哪些点>"。

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md
git commit -m "audit(batch-a): 台账骨架 + 环节①webhook入口/签名/managed门"
```

---

## Task 2: 环节②去抖聚合 pipeline 审查

**Files:**
- 审查(只读): `src/webhooks.rs:130` `run_debounce_pipeline`、`register_inbound`、`reload_managed_contact`:270、generation/deadline/barge_in 相关

**Deliverable:** 台账"环节②"section 的 findings。

- [ ] **Step 1: 读透去抖机制**

Read `run_debounce_pipeline`(:130) 全体 + `register_inbound`。逐条问：
  - deadline 刷新：连发多条时 deadline 是否正确顺延？窗口内静默才决策？
  - generation 抢占（bump generation 不重复 spawn）：新消息进来时旧 pipeline 如何被淘汰？有无两个 pipeline 同时活的竞态？
  - `reload_managed_contact`(:270)：窗口期内 contact 转 unmanaged/被删的早退是否所有分支都覆盖？
  - barge_in：回复进行到一半用户又说话，抢占是否干净（不会发出半句 + 新句叠加）？
  - 聚合读取：`load_recent_messages` 读窗口内全部消息一次性回复——边界（消息刚好在窗口边缘）？

- [ ] **Step 2: 对照 F-021 台账既有结论**

Read 上轮台账 F-021 段（`2026-07-10-full-system-test-findings.md`）——它已亲验去抖机制"健全"。本 task 是**更深一层**：不看"机制在不在"，看"竞态/边界是否真的无缺口"。若发现上轮"健全"结论下的边界缺口，明确标注"补 F-021 深层"。

- [ ] **Step 3: 环节②findings 入账 + Commit**

写入台账；`git commit -m "audit(batch-a): 环节②去抖聚合pipeline"`

---

## Task 3: 环节③gateway 巨型闸函数审查（验证重心）

**Files:**
- 审查(只读): `src/agent/gateway.rs:616` `run_user_operation_gateway`、`:999` `run_user_operation_gateway_inner`（~1294 行无单测）

**Deliverable:** 台账"环节③"section——这是批 A 最重环节，findings 预期最多。

- [ ] **Step 1: 测绘 inner 函数的闸序**

Read `run_user_operation_gateway_inner`(:999) 全函数。画出闸的**执行顺序与短路点**：managed / cooldown / min-interval / 日上限 / 过期。逐个闸问：
  - 每个闸的判定读的是哪个字段、比较符方向对不对、临界值（如 min-interval 恰好相等时放行还是拦截）？
  - 闸之间的顺序：先查 cooldown 还是先查日上限？顺序错会不会导致该拦的没拦 / 该放的没放？
  - reload context：inner 里 reload 的 contact 与 pipeline 层 reload 是否可能不一致（TOCTOU）？

- [ ] **Step 2: 审 operation_state 派生一致性**

Grep gateway 里 `operation_state` / `customer_stage` 写入点。对照 CLAUDE.md「operation_state 从 customer_stage 派生（C2/m006 同 id 空间）」+ fail-soft（非法转移跳过写、发审计事件、不阻断已发送回复）。核：派生逻辑是否真的在同一写点、fail-soft 是否真的不阻断。

- [ ] **Step 3: 审预算 RunBudget 降级路径**

Grep gateway 里 `RunBudget` / `is_exceeded` / `BudgetExceeded` / `local_decision_review`。对照 CLAUDE.md「超预算返 BudgetExceeded、gateway 回落（local_decision_review、skip rewrite）、不 5xx 给 webhook」。核降级分支是否真的兜住、是否有分支漏兜导致 no_reply。

- [ ] **Step 4: 派 subagent 复审 + 主控亲验**

派 1 个 opus subagent 只读复审 inner 函数（指令：先读懂闸序再断言、每条带 file:line、不许凭猜）。主控对 subagent 每条结论亲验 file:line 后才入账。

- [ ] **Step 5: 环节③findings 入账 + Commit**

写入台账（标注哪些是"可 117 复现"候选留 Task 9）；`git commit -m "audit(batch-a): 环节③gateway巨型闸函数(验证重心)"`

---

## Task 4: 环节④决策 + 知识路由审查

**Files:**
- 审查(只读): `src/agent/decision.rs`、`src/agent/knowledge_router.rs`

**Deliverable:** 台账"环节④"section。

- [ ] **Step 1: 读透决策主流程**

Read `decision.rs` 的 Reply Agent 主决策 + 初始画像生成。逐条问：
  - 决策 JSON 契约：结构化字段（customer_stage/intent_level/objection_type）漏填时降级路径？对照 memory「LLM 内层键 camelCase 漂移」是否已兜（snake 未命中回退 camel）？
  - tool_calling 形态：对照 memory「主链路 tool_calling 静默 no_reply 已根治(PR#107)」——核 decisionPhase 是否恒 final、三站点（首发/rewrite/revision）是否都覆盖。
  - 双层标签：customer_stage 等必来自 system_taxonomies；自由词进 candidates 不阻断 run——核实现。

- [ ] **Step 2: 读透知识路由**

Read `knowledge_router.rs` catalog→search→open_slice tool-calling planner。问：
  - 产品声明必须 verified knowledge 背书否则 blocked_unverified_product_claim——路由拿不到知识时是否正确导向拦截而非幻觉？
  - 渐进式升档（Lean→Full）：对照 memory B-1（升档撑爆预算已修 PR#143）——核 escalated budget 是否真的授予、非升档不授予。

- [ ] **Step 3: 环节④findings 入账 + Commit**

`git commit -m "audit(batch-a): 环节④决策+知识路由"`

---

## Task 5: 环节⑤review 阈值闸 + revision 审查（验证重心）

**Files:**
- 审查(只读): `src/agent/review/gates.rs:20` `review_passed`、`:115` `classify_dual_gate`、revision 流程

**Deliverable:** 台账"环节⑤"section。

- [ ] **Step 1: 逐闸核阈值与比较符**

Read `review_passed`(:20) + `classify_dual_gate`(:115) 全体。对每个闸核**临界比较符**（这是最易错处）：
  - FactRisk：`hallucination_score >= fact_risk_block_at` block（:120）——`>=` 对不对？临界相等该拦吗？
  - HumanLike：`human_like < human_like_rewrite_below` rewrite（:148）——`<` 对不对？
  - ProductAccuracy：`knowledge_grounding_score < product_accuracy_block_below` block（:135）？
  - PressureRisk：`pressure_risk < pressure_risk_block_at`（:38）+ `==0 豁免`——0 豁免会不会被利用绕过？
  - 阈值默认值来源：Grep `fact_risk_block_at` 等在 runtime/config 的默认值，核是否 = CLAUDE.md 声明的 6/7/6/7。

- [ ] **Step 2: 审 revision 单次约束**

Grep revision 触发与计数。核「最多一次 revision」是否真的有硬计数、二次触发是否被挡、revision 后是否重新过闸。

- [ ] **Step 3: 审 gateway/finalReview 状态枚举闭集**

对照 CLAUDE.md「状态枚举闭集、写未知状态须 DB 写点拒绝不静默 coerce」。Grep 状态写入点，核未知状态处理。

- [ ] **Step 4: 环节⑤findings 入账 + Commit**

标注"可 117 复现"候选；`git commit -m "audit(batch-a): 环节⑤review阈值闸+revision(验证重心)"`

---

## Task 6: 环节⑥outbox 幂等/claim/second-pass 审查（验证重心）

**Files:**
- 审查(只读): `src/agent/outbox.rs`、`src/agent/outbox_dispatcher.rs`

**Deliverable:** 台账"环节⑥"section。

- [ ] **Step 1: 读透幂等键与入队**

Read `outbox.rs`。问：
  - 幂等键派生：同一 decision 重复入队是否被 unique index 挡？键构成是否含足够维度（不会两条不同 send 撞键）？
  - approved decision 必须先入 outbox 再 MCP——核顺序，有无"先发后记"窗口。
  - 用户 rejection/cooldown 取消 pending outbox——核取消是否彻底。

- [ ] **Step 2: 读透 dispatcher claim 与 second-pass**

Read `outbox_dispatcher.rs`。对照 memory 三处已修（PR#136/#164）：
  - `atomic_claim_pending` 是否有 `.sort({created_at:1,_id:1})`（memory 说已修 86d127f）——亲验现码。
  - dispatcher timeout 与 mcp_logs 写序（memory 说 client-timeout<send-timeout 修了 59d84b5）——亲验现码。
  - second-pass safety gate 每次 MCP 前重查——亲验。
  - **更深**：claim 并发（两个 dispatcher 实例同抢一条）？retry backoff 有界？Ok-on-DB-write-failure fail-soft 是否可能漏发/重发？

- [ ] **Step 3: 环节⑥findings 入账 + Commit**

标注"可 117 复现"候选（幂等竞态是真跑重心）；`git commit -m "audit(batch-a): 环节⑥outbox幂等/claim/second-pass(验证重心)"`

---

## Task 7: 环节⑦MCP 发送审查

**Files:**
- 审查(只读): `src/mcp.rs:160` `call_tool_with_key`、`:196` isError、message_send_text 调用点、超时配置

**Deliverable:** 台账"环节⑦"section。

- [ ] **Step 1: 读透 send + 失败识别**

Read `call_tool_with_key`(:160) + isError 分支(:196)。问：
  - HTTP 状态 / JSON-RPC error / `result.isError` 三层失败是否都识别（memory 说 isError 已补 5779c33）——亲验。
  - 超时：`MCP_CLIENT_TIMEOUT_SECONDS`(60) < `MCP_SEND_TIMEOUT_SECONDS`(150) 关系（memory）——亲验现码。
  - API key 只作 Bearer 不入日志——亲验。
  - 5xx 转 upstream_error 不 panic——亲验。

- [ ] **Step 2: 环节⑦findings 入账 + Commit**

`git commit -m "audit(batch-a): 环节⑦MCP发送"`

---

## Task 8: 环节⑧回写审查（events/metrics/run log/operation_state）

**Files:**
- 审查(只读): gateway 回写段 + `src/agent/run_envelope.rs` + outcome metrics / decision review 写入点

**Deliverable:** 台账"环节⑧"section。

- [ ] **Step 1: 读透回写路径**

Grep gateway 回写 events / outcome metrics / decision review / run log 的写入点。问：
  - 送达后 DB 写失败：是否降级不返 Err（防重发，memory 说这是刻意 fail-soft）——亲验方向对不对。
  - run log 的 promptVersions / token usage 记录完整性。
  - operation_state 派生写点（与 Task 3 Step 2 呼应，此处看回写侧）。
  - 计数/聚合口径（outcome aggregation）是否与查询侧一致。

- [ ] **Step 2: 环节⑧findings 入账 + Commit**

`git commit -m "audit(batch-a): 环节⑧回写events/metrics/run log"`

---

## Task 9: 117 真跑复现（验证重心的 CONFIRMED 实证）

**Files:**
- 只读复现脚本（若造脚本放 117 `/opt/wechatagent/scripts/e2e/`，收尾清理）
- Modify(台账): 把可复现 findings 从 PLAUSIBLE 升级 CONFIRMED 或证伪

**Deliverable:** 台账中标记"可 117 复现"的 findings 全部有真跑结论（CONFIRMED/证伪/BLOCKED）。

- [ ] **Step 1: 盘点可复现清单**

从台账筛出所有标"可 117 复现"的 findings（预期集中在环节③gateway 闸、⑤阈值闸临界、⑥outbox 幂等）。列出每条的复现路径（需要什么 webhook 输入 / 什么前置状态）。

- [ ] **Step 2: 确认 117 环境与 2 联系人状态**

paramiko 只读查：吴界/AI应用开发当前 agent_status、账号 102 webhook_secret（用于签名）、服务 active、端点空闲（不与套件并发）。

- [ ] **Step 3: 逐条串行真跑复现**

对每条 finding，用带方案 B 签名的真实 webhook 灌入 + 直连 Mongo 核对落库结果。**串行**（端点 2 线程）。真发只碰 2 测试联系人。能复现→CONFIRMED（记实测证据）；不能→证伪并从台账移除或降级说明；端点/MCP 挂→BLOCKED 标注。

- [ ] **Step 4: 环境恢复 + 台账更新**

清造的测试数据、联系人状态还原、删 117 临时脚本、核零残留。台账对应 finding 验证状态更新。

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md
git commit -m "audit(batch-a): 117真跑复现结论(CONFIRMED/证伪)"
```

---

## Task 10: 批 A 台账汇总 + 优先级 + PR

**Files:**
- Modify(台账): 批 A 总评 section

**Deliverable:** 批 A findings 汇总（按严重度分类 + 修复优先级建议），PR 开好。

- [ ] **Step 1: 汇总与分级**

台账加"批 A 总评"：finding 计数按 Critical/High/Med/Low；标出跨环节根因家族（若有）；给修复优先级建议（供用户定修复顺序，同上轮台账体例）。

- [ ] **Step 2: 自检防假绿**

复核每条 finding：file:line 是否亲验、CONFIRMED 是否真有 117 证据、有无把闸门观测值误当 bug、有无过拟合式结论。

- [ ] **Step 3: Commit + PR**

```bash
git add docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md
git commit -m "audit(batch-a): 批A总评+修复优先级"
git push -u origin audit/deep-logic-batch-a
gh pr create --title "audit: 核心自动回复链路深度审查 批A findings（只审不修）" --body "..."
```

（PR body 说明：这是纯审查产出台账，无 src 改动；findings 待用户定优先级后进入修复批次。）

---

## Self-Review 结论

- **Spec coverage**：设计文档批 A 的 8 环节 ↔ Task 1-8 一一对应；117 真跑重心（gateway 闸/阈值闸临界/outbox 幂等）↔ Task 3/5/6 标注 + Task 9 集中复现；只入账不修 ↔ 全 task 无 src 修改、Global Constraints 明列；台账结构 ↔ Task 1 Step 1 建骨架 + 固定字段。
- **Placeholder scan**：无 TBD；每个 Step 给了具体审查问题清单与亲验锚点（file:line）。审查计划的 Step 内容是"审什么问题"而非"写什么代码"，符合审查工程性质。
- **一致性**：链路锚点（webhooks:287/gateway:999/gates:20&115/mcp:160&196）全部前置亲验；台账文件名、finding 字段跨 task 一致；memory 既有结论（F-021/PR#107/#136/#143/#164）在对应 task 作为"更深一层"基线引用而非重复。
- **审查工程适配**：无 TDD 红绿（审查无产物代码），用"读码→对照 spec→亲验→117 真跑→入账"替代，每 task 仍有独立可提交 deliverable（台账 section）。
