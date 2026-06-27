# Prompt 自优化「AI 提议 + 人工把关发布」设计

> 日期：2026-06-27　状态：设计（brainstorming 产出，待 writing-plans）
> 关联核实记忆：`project_evolution_self_optimization_audit`（全模块 100% 代码核实结论）

## 0. 一句话目标

把 prompt（提示词）自优化从当前「看得见建议、发不出去、且发布路径绕过红线防护」的半成品，做成「AI 提议 + 真模型证据 + 人工把关发布」的可用闭环。**阈值（threshold）通道已是生产级闭环，本设计不动它。**

## 1. 为什么是「人工把关」而非「全自动闭环」

经全模块代码核实后的方法论判断：

- **阈值能自动闭环**，因为改的是连续数值、shadow 能用纯函数精确重判（`replay.rs::evaluate_threshold` 读 `review.scores` 按新阈值重算，决定性、无 LLM）、放松安全闸有 #152 零容忍门（`significance.rs`）。
- **prompt 不能自动闭环**：① prompt 改写后 LLM 的输出无法被影子精确预测（LLM 非确定性 + 生产上下文不可复现）；② 要改的恰恰是核心红线 prompt（`user.reply.policy` 锁着「无人工接管 / grounding / 五闸」红线锚点）；③ 让 LLM 改写约束 LLM 自己的规则，人工把关是**终态设计而非过渡**。

因此 prompt 通道定位为：**critic 自动发现 + 真模型跑对照产出证据 + 管理员看证据点发布**。

## 2. 当前缺口（全部经代码核实，含行号）

| # | 缺口 | 核实证据 |
|---|---|---|
| G1 | prompt 候选的 shadow replay 未实装 → 候选结构性永远到不了可发布 | `replay.rs:222` prompt 候选直接 `ReplayOutcome::failed("prompt_replay_not_implemented_w3")` → 所有 shadow_replays 全 failed → `significance.rs` `grade_prompt` 走 `early_reject`（completed=0 < min_replays 30）→ 候选永远 `rejected_below_threshold` |
| G2 | `release_prompt` 自动发布路径**完全不过**人工编辑的三道红线闸 | `release.rs:195-371` grep 无 `validate`/`anchor`/`redline`；对比人工路径 `prompt_templates.rs:150`（validate_prompt_edit）+`:169`（review_prompt_edit）走完整三闸 |
| G3 | snippet 语义错配：critic schema 说「片段」，release 当「整篇」覆盖 | critic schema `prompts.rs:2340`「建议追加/替换的 prompt 片段」；`release.rs:225` `proposal.diff_snippet.clone()` 整段当新 content 写库，且 `release.rs:227` 错误信息原文「W4 release path requires a complete content body」——现有 W4 代码**预期 diff_snippet 是完整正文**，与 critic「片段」语义正面冲突 → 若 critic 真提片段，发布后正文只剩片段、红线全删 |
| G4 | `grade_prompt` 原始侧基线是假的 | `significance.rs:424` `original_self_critique_for_metric()` 恒 None、`original_5gate_hit_or_default` 恒 false |

**当前不可触发**：`release_prompt` 唯一调用者是 `routes/evolution.rs:169`（admin 点确认），无自动路径；且 prompt 候选因 G1 永远到不了 `eligible_for_release`。是潜在隐患非现行漏洞——但一旦补 G1 让 prompt 闭环、或有人手动 release 一个候选，G2/G3 立即变成真红线漏洞。

## 3. 三条不可破的红线（贯穿设计）

1. **发送隔离**：shadow replay 绝不碰 `scripts/check-evolution-isolation.sh` 禁的 8 个发送符号（`crate::agent::gateway`/`crate::agent::outbox`/`crate::mcp::`/`agent_send_outbox.insert`/`mcp_client.send`/`run_user_operation_gateway`/`handle_managed_message`/`handle_follow_up_task`）。**注意**：该脚本只禁发送符号，不禁调 LLM / decision / review（critic 自己就调 `state.llm`）。
2. **无人工接管红线**：critic 改 prompt 只能「末尾追加」，原 prompt 正文逐字保留；发布时锚点闸校验 `DEFAULT_REPLY_REDLINE_ANCHORS`（`prompts.rs:64-69` 两条逐字子串）仍在。
3. **红线靠机制不靠 LLM 自觉**：发布前置三道闸（禁词+锚点+LLM 语义）是硬卡点（fail-closed），不依赖 critic 自我约束（`management_prompt_edit.rs:2-3` 既有原则）。

## 4. 架构：四个改动点

### 改动 A — 三道闸下沉到中立模块（阶段一）

**现状**（核实）：`validate_prompt_edit`/`review_prompt_edit`/`prompt_edit_tier`/`required_anchors`/`extract_diff`/`PromptEditTier`/`PromptEditVerdict` 在 `src/routes/management_prompt_edit.rs`，模块声明为 `mod management_prompt_edit`（私有，`routes/mod.rs:59`），函数是 `pub(super)` → evolution 跨模块调不到。

**改动**：新建 `src/prompt_guard.rs`（顶层模块，`lib.rs` 加 `pub mod prompt_guard;`），把上述符号迁入，可见性升 `pub`。`routes/management_prompt_edit.rs` 改为 re-export 或直接调用 `crate::prompt_guard::*`（保持 `prompt_templates.rs:150/169` 调用点不破）。

**迁移依赖**（核实 `management_prompt_edit.rs` 的 import）：
- `crate::evolution::lint::passes_forbidden_words`（禁词闸，已跨模块可达）
- `crate::prompts::{normalize_prompt_content, DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_REDLINE_ANCHORS, DEFAULT_REVIEWER_FEWSHOT, PROMPT_EVOLUTION_FORBIDDEN_KEYS}`
- `crate::prompts::load_prompt` + `crate::agent::generate_agent_json`（review_prompt_edit 的 LLM 语义闸）
- `crate::routes::AppState`（改为 `crate::routes::AppState` 仍可，AppState 是 pub）

**合规性**：`validate_prompt_edit` 是纯函数（只读锚点常量）；`review_prompt_edit` 调 `generate_agent_json`——都不在 8 个发送符号内，下沉后 evolution/release 引用合规。

**测试迁移**：`management_prompt_edit.rs` 现有 10 个单测随函数迁到 `prompt_guard.rs`（逐字保留，含 `forbidden_phrase()` 字符拼接绕禁词扫描的 helper）。10 个：tier_classifies_three_layers / dual_gate_rejects_forbidden_words / dual_gate_rejects_business_anchor_drift / dual_gate_rejects_redline_anchor_drift / dual_gate_allows_valid_constrained_edit / forbidden_tier_always_rejected / freely_editable_only_checks_forbidden_words / anchor_gate_normalizes_crlf / extract_diff_isolates_added_lines / verdict_variants_shape。

### 改动 B — release_prompt 接三闸 + snippet 改末尾追加（阶段一）

**现状**（核实 `release.rs:220-312`）：取 `proposal.proposed_template_key` → `proposal.diff_snippet` 作 `new_content` → 加载 `current`（current_version=true 那条）→ 事务里把旧 current 置 false + insert 新版本 content=`new_content`。

**改动**：在 `release_prompt` 加载 `current`（`release.rs:234`，字段 `current.content` 核实存在）后、写库前：
1. 合成 `new_content = format!("{}\n\n{}", current.content, proposal.diff_snippet)`（末尾追加，原文逐字保留）
2. 调 `crate::prompt_guard::validate_prompt_edit(&prompt_key, &new_content)` —— 禁词闸 + 锚点闸（锚点因原文保留天然通过；若不过说明原 prompt 已缺锚点，fail-closed 拒绝正确）
3. 调 `crate::prompt_guard::review_prompt_edit(state, &workspace_id, &prompt_key, &current.content, &new_content)` —— LLM 语义闸（审增量 = 追加的片段）；返回 `Reject` → release 中止；`NeedsHumanConfirm` → 见错误处理 §6
4. 两闸过才进事务写库

**EvolutionError 扩展**：`error.rs` 加 `RedlineGateRejected(String)` 变体；`routes/evolution.rs::evolution_error_to_app_error` 映射为 `AppError::BadRequest`。

### 改动 C — 补 prompt shadow replay = 跑真模型对照（阶段二）

**目标**：对 cohort.prompt 每条历史失败 run，用「原 prompt + 追加候选片段」跑真模型 Reply+Review，记新旧 `review.scores` 对照。

**放置位置**（核实约束）：**不能放 `src/evolution/`**——shadow 要调 `decide_reply`/`review_decision`，而它们的入参重建依赖 `simulation.rs:21-23` import 的 `super::gateway::{load_context_messages, load_pending_tasks, precheck_send_gateway}`，evolution 引用 `crate::agent::gateway` 会被隔离脚本拦。

**方案**：在 `src/agent/` 下新建 `prompt_shadow.rs`（与 simulation 同层，可合法引用 gateway 的非发送 helper），暴露一个 `pub(crate) async fn shadow_replay_prompt_candidate(state, proposal, source_run_id, prompt_override) -> AppResult<PromptShadowOutcome>`。`evolution::replay.rs` 的 prompt 分支改为调 `crate::agent::prompt_shadow::shadow_replay_prompt_candidate(...)`（调 agent 模块入口，不引用 8 个发送符号 → 合规）。

**核心技术决策（prompt 注入）**：`decide_reply`/`review_decision` 的 prompt 不是入参，在 `decide_reply_with_promote` 内部经 `load_prompt_for_contact`（`decision.rs:597-626`）从 DB 加载。选定方案：**给 `decide_reply_with_promote` + `review_decision` 加可选入参 `prompt_override: Option<&PromptOverride>`**，注入到 `assemble_system_prompt`（`decision.rs:704`）之前 —— `prompt_override` 含 `{ reply_policy_append: Option<String>, ... }`，对命中的 prompt_key 在加载后追加片段。现有所有调用点传 `None`（字节等价，护栏：DEFAULT 行为不变）。
- **不选「临时写 DB version」**：会污染 prompt_templates、并发不安全、清理易遗漏。

**上下文重建**：复用 simulation 的加载链（`load_operation_playbook_for_contact`/`load_or_create_operating_memory`/`load_operation_knowledge`/`load_context_messages`/`route_operation_knowledge`/`select_operation_knowledge_chunks`），源 run 的 inbound 经 `AgentRunLog.context` 的 `inboundMessageId`（`replay.rs:196` 既有读法）反查 `messages()`。inbound 已不在（retention 清理）→ 该条记 failed `source_message_unavailable`（沿用 threshold 既有处理）。

**预算**：shadow 跑真模型烧 token，计入 `EvolutionBudget`（`budget.record_call`，`prompt_critic.rs:137` 同款）；耗尽则停止后续 replay，候选留 `pending_eval` 下 tick 续跑。

### 改动 D — 显著性改存证据 + 前端展示（阶段二 + 阶段三）

**阶段二**：`grade_prompt`（`significance.rs:219`）改为：不再作自动放行 gate，而是把新旧对照（per-gate hit delta、send_success delta、self_critique delta、completed/failed 计数、逐样本新旧 review scores 摘要）写进 `proposal.eval_metrics`。`aggregate_and_grade`（`significance.rs:445`）对 prompt 候选：shadow 完成（completed ≥ 1）即置 `eligible_for_release`（**语义重定为「证据就绪、等人工看」**，复用现有态不加新态 —— 核实 `proposal.status` 是 String 无闭集校验，但散落 `release.rs` 多处判断 + 前端 `proposalTypes.ts:7-9` 显式联合类型；复用 `eligible_for_release` 避免改这些）。同时修 G4：让 `original_*` 基线从源 run 真实数据取（源 run 的 `final_review_status`/`self_critique` 已在 `AgentRunLog`）。

**阶段三**：前端 `features/evolution/EvolutionCenterTab.tsx` + `components/review/ProposalReleaseCard.tsx` 候选详情页：展示「原 prompt vs 新 prompt 在 N 条历史样本上的五闸/自评对照表」+ critic reasoning + 追加的 diff 片段；管理员看完点 release（确认串 `RELEASE`，UI 已存在 `routes/evolution.rs:149`）。`proposalTypes.ts` 若 `eligible_for_release` 语义变化加注释，不改类型。

## 5. 数据流（端到端）

```
[worker tick] critic 生成候选(prompt_critic.rs,已有)
  → proposal{kind:prompt, diff_snippet:"追加片段", status:pending_eval}
   ↓
[shadow 改动C] evolution::replay prompt 分支 → crate::agent::prompt_shadow::shadow_replay_prompt_candidate
  对 cohort.prompt 每条源 run:重建上下文 → 合成"原文+追加片段"(prompt_override) → decide_reply+review_decision 跑真模型
  → 新旧 review.scores 对照 → shadow_replays 行
   ↓
[显著性 改动D] grade_prompt 算对照写 eval_metrics(不再决定放行) → shadow 完成则 status=eligible_for_release(语义=证据就绪等人工)
   ↓
[前端 改动D] 候选详情:新旧五闸/自评对照表 + critic reasoning + diff
   ↓
[人工] admin 看证据 → 点 release + 确认串"RELEASE"(routes/evolution.rs:149,已有)
   ↓
[release_prompt 改动B] 合成"原文+追加" → validate_prompt_edit(禁词+锚点) + review_prompt_edit(LLM语义) → 任一拒则中止
  → 事务化写 prompt_templates 新版本 + prompt_pack_version.fetch_add(release.rs:338,已有)
   ↓
[post_release 已有] +24h 复查(post_release.rs)
```

## 6. 错误处理与边界

- **shadow 真模型失败**（端点 503 / tool_use 劫持 / 超时）：该条 replay 记 failed，不阻塞其他；对照证据标「样本不足」，但**不阻止人工 release**（人可基于 critic reasoning + 部分样本判断）。区别于 threshold——threshold 自动放行所以 shadow 失败必拒；prompt 人工放行所以 shadow 只是参考信号。
- **EvolutionBudget 耗尽**：停止后续 shadow，候选留 `pending_eval` 下 tick 续跑（不报错）。
- **release 时三闸拒**：proposal 不进 released，`release_prompt` 返回 `EvolutionError::RedlineGateRejected`，前端显示「该候选触碰红线，已拒绝发布」+ failure_reason。
- **review_prompt_edit 第三闸 LLM 不可用**（重试退避后仍 503）：返回 `NeedsHumanConfirm`（`management_prompt_edit.rs:171` 既有三态）。release 路径处理：**视为拒绝本次自动 release，要求管理员逐字核对后再确认**（不 fail-open 放水，不 fail-closed 死路）。具体 UI 交互留 writing-plans。
- **锚点闸天然保证**：末尾追加、原文逐字保留 → 锚点必过；若不过说明原 prompt 本身已缺锚点（更严重问题），fail-closed 拒绝正确。

## 7. 测试策略

- **纯函数单测**：prompt 合成（原文+追加片段）；三闸下沉后行为不变（迁移 `management_prompt_edit.rs` 现有 11 测试到 `prompt_guard.rs`，逐字保留）。
- **shadow 重放**：mock `decide_reply` 返回，验证新旧 scores 对照计算 + `prompt_override` 注入正确；真模型对照留 nightly / 真实 LLM 套件。
- **release 三闸集成**：构造 critic 候选其 diff_snippet 含禁词 / 试图破坏锚点 → 验证 release 被拒（`RedlineGateRejected`）。
- **基线不回归**：`cargo test --lib` ≥ 350 / 0；4 PBT 文件（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥ 33 / 0（CLAUDE.md 硬门）。
- **隔离脚本仍绿**：`scripts/check-evolution-isolation.sh` —— 确认 `prompt_shadow.rs` 放 `src/agent/`（非 evolution/），evolution::replay 只调其入口、不引入 8 个发送符号。
- **`prompt_override=None` 字节等价**：现有所有 `decide_reply`/`review_decision` 调用点传 None，行为与改造前逐字相同（反过拟合护栏）。

## 8. 分阶段交付（三阶段一起规划，独立可验证可合并）

1. **阶段一（安全底线，最高优先级，独立有价值）**：改动 A（三闸下沉 `prompt_guard.rs`）+ 改动 B（release_prompt 接三闸 + snippet 末尾追加）。**这一阶段单独就堵住了 G2/G3 红线缺口**，即使后两阶段不做也让系统更安全。不依赖 G1 是否修复。
2. **阶段二（证据闭环）**：改动 C（`agent/prompt_shadow.rs` 真模型对照 + `prompt_override` 入参）+ 改动 D 后端部分（grade_prompt 改存证据 + 修 G4 原始侧基线 + eligible_for_release 语义重定）。
3. **阶段三（前端）**：候选详情页新旧对照证据表 + 人工 release 决策界面（复用现有确认串 UI）。

## 9. 不做（YAGNI / 守红线）

- **不做 prompt 全自动发布**（核心决策）。
- **不动 threshold 通道**（已生产级闭环）。
- **不给 critic 输入标「红线只读」**（末尾追加 + 锚点闸已兜底）。
- **不加 prompt section 标记 / 查找-替换合成**（末尾追加最安全，原红线正文一字不动）。
- **不加 proposal status 新态**（复用 eligible_for_release 改语义，避免改 release.rs 散落判断 + 前端联合类型）。
