# 决策复盘补齐 AI 自治协议 9 字段（后端投影 + 前端展示）

> 日期：2026-07-02
> 类型：后端为主（Rust/Axum，纯读投影）+ 前端展示（React + TS）
> 前置：基于最新 origin/main（commit d4558bb，含 PR #93 驾驶舱重设计）。所有引用已亲验（带 file:line）。

## 0. 背景与问题

驾驶舱重设计（PR #93）的决策复盘下钻 `ConversationReviewView` 只展示了 `DecisionReview` 上真实携带的结构化字段（scores/risks/nextBestAction/finalReviewStatus/holdCategory）。spec §3 曾提「决策复盘（自治协议 9 字段）」，但当时核实发现：**这 9 个「AI 内心独白」字段（whyShouldReply/selfCritique/userUnderstanding 等）不在 `AgentDecisionReview` 模型上**，故未展示。用户已确认要补齐后端能力，surface 全部 9 个字段（分组展示）。

## 1. 已亲验的关键事实（设计地基，全部带 file:line）

### 1.1 9 个自治协议字段在 `AgentDecision` 上（`src/agent/types.rs:180-196`）
`AgentDecision` 结构体（`types.rs:82`，`#[serde(rename_all = "camelCase")]` 于 `:81`）含 R1.1 自治协议 9 字段，全为 `String`：
- `user_understanding` / `relationship_read` / `operation_goal` / `knowledge_need_reason` / `memory_update_reason` / `self_critique` / `why_should_reply` / `why_skip_reply` / `risk_self_check`
- 因 camelCase 序列化，落库键名为 `userUnderstanding` / `relationshipRead` / `operationGoal` / `knowledgeNeedReason` / `memoryUpdateReason` / `selfCritique` / `whyShouldReply` / `whySkipReply` / `riskSelfCheck`。

### 1.2 这 9 字段已持久化在 `agent_run_logs.decision`（`src/agent/gateway.rs:2059`）
run log 写入侧 `to_document(&final_decision).unwrap_or_default()`（`gateway.rs:2059` / `:2192`）把整个 `AgentDecision` 序列化进 `agent_run_logs.decision`（`AgentRunLog.decision: Document`，`models.rs:2663`）。→ **9 字段已在库里，无需迁移、无需改写入侧。** types.rs:177 注释亦明确「9 个字段全部以 String 落入 `agent_run_logs.decision`，便于审计端原文读取」。

### 1.3 `/api/decision-reviews` 端点已 join agent_run_logs（`src/routes/reviews.rs:29,92`）
- `list_decision_reviews`（`reviews.rs:29`）读 `agent_decision_reviews`，对每条调 `fetch_run_status(&state, review.run_id)`（`reviews.rs:62`）。
- `fetch_run_status`（`reviews.rs:92`）已按 `run_id` 对 `agent_run_logs` 做 `find_one`（`:102`），当前只取 `log.final_review_status` 与 `log.review.holdCategory`，返回 `(Option<String>, Option<String>)`。
- **关键**：这个 `find_one` 已经把整条 `AgentRunLog`（含 `decision` Document）读进内存了 —— 补 9 字段是从**已在手的** `log.decision` 里多抽几个键，**零新增查询**。
- `get_decision_review`（`reviews.rs:68`，单条端点）同样调 `fetch_run_status`（`:86`）→ 一处改动两个端点都受益。

### 1.4 输出投影 `decision_review_json`（`src/routes/shared.rs:1175-1211`）
`decision_review_json(review, final_review_status, hold_category)` 把 `AgentDecisionReview` 投影成 JSON（现 29 键，含 `finalReviewStatus`/`holdCategory` 两个来自 join 的参数）。这是补 `autonomyProtocol` 输出的落点。

### 1.5 契约快照测试（`src/routes/shared.rs:1980-2025` + `contract_snapshot.rs`）
`decision_review_json_matches_contract_fixture`（`shared.rs:1981`）构造一条 `AgentDecisionReview` 调 `decision_review_json`，`assert_contract_fixture("decision_review", …)` 断言匹配 fixture。fixture 路径是 **`frontend/src/contracts/decision_review.fixture.json`**（`contract_snapshot.rs:51-53`：`CARGO_MANIFEST_DIR/frontend/src/contracts/<name>.fixture.json`，前后端共享唯一真相源），**不是** `src/contracts/`。re-bless 机制：`UPDATE_SNAPSHOTS=1 cargo test --lib decision_review_json_matches_contract_fixture` 写文件。改投影必须：①更新单测（传新参 + 期望）②`UPDATE_SNAPSHOTS=1` re-bless fixture ③同步前端 vitest 契约测试 `frontend/src/contracts/decisionReview.contract.ts` 的 CANONICAL_KEYS（加 `autonomyProtocol`）。**本机 `cargo test --lib` 可跑此单测验证**。

### 1.6 前端消费点
- `DecisionReview` TS 类型（`frontend/src/types/index.ts:288-304`），当前无自治协议字段。
- `ConversationReviewView`（`frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx`）的 `ReviewItem` 展开区（现渲染 finalReviewStatus/holdCategory/scores/risks/nextBestAction）是新增「AI 内心独白」分组的落点。
- store `userOpsStore.ts` 拉 `/api/decision-reviews` 得 `decisionReviews`（无需改 loader，字段随投影自动带上）。

## 2. 目标与方案

**核心洞察**：9 字段后端已落库、端点已 join，故这是**纯读投影 + 前端展示**改动 —— 零迁移、零新查询、不动 `AgentDecisionReview` 模型、不动写入侧、不动 autonomy 聚合接口。

### 2.1 后端：run-log join 多带 9 字段 → 投影加嵌套对象
- `fetch_run_status` 的返回从 `(Option<String>, Option<String>)` 改为一个结构体 `RunStatusView { final_review_status: Option<String>, hold_category: Option<String>, autonomy_protocol: Option<Value> }`，其中 `autonomy_protocol` 从 `log.decision`（Document）抽 9 个 camelCase 字符串键构造；**任一字段缺失按空串处理，9 个全空或无 run_id/无 log → `None`**（优雅降级）。
- `decision_review_json` 增第 4 个参数 `autonomy_protocol: Option<Value>`，输出加嵌套键 `autonomyProtocol`（`None` → JSON `null`）。选**嵌套对象**而非 9 个顶层键：契约清爽、分组语义清晰、前端一次判空即可整体降级。

`autonomyProtocol` 形状（9 键，全 string）：
```json
"autonomyProtocol": {
  "userUnderstanding": "...", "relationshipRead": "...", "operationGoal": "...",
  "knowledgeNeedReason": "...", "memoryUpdateReason": "...", "riskSelfCheck": "...",
  "selfCritique": "...", "whyShouldReply": "...", "whySkipReply": "..."
}
```

### 2.2 前端：三分组展示 + 优雅降级
`DecisionReview` TS 类型加可选 `autonomyProtocol?: AutonomyProtocol`（9 个可选 string）。`ConversationReviewView` 的展开区新增「AI 内心独白」区，**三分组**：
- **回复决策**：`whyShouldReply`（should_reply=true 时非空）或 `whySkipReply`（should_reply=false 时非空）+ `selfCritique`
- **理解**：`userUnderstanding` / `relationshipRead` / `operationGoal`
- **运营依据**：`knowledgeNeedReason` / `memoryUpdateReason` / `riskSelfCheck`

`autonomyProtocol` 为 `null` 或全字段空 → 整个「AI 内心独白」区不渲染（优雅降级，不崩、不留空壳）。每个字段项：字段值为空则该项不渲染（只显有内容的）。措辞遵守禁词约束（见 §4）。

## 3. 文件结构

| 文件 | 改动 |
| --- | --- |
| `src/routes/reviews.rs` | `fetch_run_status` 返回类型改结构体，从 `log.decision` 抽 9 字段；两个调用点（`:62`/`:86`）适配 |
| `src/routes/shared.rs` | `decision_review_json` 加第 4 参数 `autonomy_protocol`，输出加 `autonomyProtocol` 键；更新契约单测（`:1981`）传新参 |
| `src/contracts/decision_review.fixture.json` | 加 `autonomyProtocol` 期望值（re-bless） |
| `frontend/src/types/index.ts` | `DecisionReview` 加 `autonomyProtocol?`，新增 `AutonomyProtocol` 类型 |
| `frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx` | 展开区加「AI 内心独白」三分组渲染 + 优雅降级 |
| `frontend/src/features/user-ops/cockpit/cockpit.module.css` | 独白分组样式（tokens.css var，无硬编码 hex） |
| `frontend/src/__tests__/features/user-ops/*.test.tsx` | ConversationReviewView 独白渲染 + 降级单测 |

## 4. 全局约束（每 task 隐含遵守）

- **禁用词 lint**（`scripts/check-no-human-takeover.sh` 扫 `src/`+`frontend/src/` 新增行）：不得含 `人工`/`接管`/`takeover`/`hand-off`（单字「人工」也被字面级拦截）。独白分组标题/字段标签用 AI 内部语义文案（如「AI 内心独白 / 回复决策 / 自我批判」）。
- **后端只读聚合红线**：本次后端改动纯读投影，**不碰 agent 决策/gateway 写入侧/发送/`AgentDecision` 结构本身**，不改 quiet_hours 等纯函数。不加迁移。
- **契约快照不回归**：`decision_review_json_matches_contract_fixture` 改投影后同步更新断言 + fixture，标注为「新增 autonomyProtocol 字段的必要连带」。
- **测试基线不回归**：`cargo test --lib` ≥350/0；前端 tsc 0 / vitest 全绿 / build 过。
- **CSS Modules**：前端新样式 `className={styles.x}` + tokens.css var，无硬编码 hex（避免 tree-shake 坑）。
- **优雅降级**：旧 run log / 无 run_id / 字段缺失时 `autonomyProtocol=null`，前端不渲染该区、不报错。这是硬要求（历史数据大量缺这些字段）。

## 5. 不做（YAGNI 边界，明确排除）

- **不改 `AgentDecisionReview` 模型**：不往 `agent_decision_reviews` collection 加列、不改写入侧（9 字段已在 `agent_run_logs.decision`，join 即可，双写是冗余）。
- **不动 autonomy 聚合接口**（`outcomes_autonomy.rs`）：那是 horizon 指标聚合，与单条复盘正交。
- **不加数据迁移**：纯读投影，历史数据靠优雅降级兼容。
- **不在复盘里做编辑**：只读展示，不提供改独白字段的入口。
- **不碰 JudgmentBar/ObserveView 等其它驾驶舱组件**：只动决策复盘下钻。

## 6. 测试

- **后端单测（本机 `cargo test --lib`）**：`decision_review_json` 加 `autonomyProtocol` 后契约快照单测（`shared.rs:1981`）；补一个 `fetch_run_status` 从 Document 抽 9 字段的纯逻辑单测（若抽出可测的纯函数）；`autonomyProtocol=None` 时投影出 `null` 的断言。
- **后端集成（留 CI）**：若已有 decision-review 端点集成测试，加「run log 有 decision 9 字段 → 端点返回 autonomyProtocol」断言；本机无 Docker，`#[ignore]` 留 CI 跑。
- **前端 vitest**：ConversationReviewView —— mock 带完整 autonomyProtocol 的 review → 断言三分组字段渲染；`autonomyProtocol=null` → 断言不渲染该区且不崩；should_reply 分支（whyShouldReply vs whySkipReply）各显对应字段。
- **回归**：`cargo test --lib` ≥350/0；前端现有 user-ops 测试不回归；契约快照更新如实标注「重构连带」。
