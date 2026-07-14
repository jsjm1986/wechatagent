# agent 旁挂能力深度逻辑审查（第一批）—— 设计

> 接续 2026-07-11 深度逻辑审查工程（53 findings，P0-P3 + 收官后补漏 F-01 全闭环）之后的**新一轮审查**。用户裁定第一批范围 = agent 旁挂能力子系统。方法论沿用上轮，只审不修，产出新 findings 台账后再按 P0-P3 分批修。

## 背景：为何是"新范围"

上轮 53 findings 覆盖的是**主链路**——"一条客户消息进来 → AI 回复出去"这条主干线的 8 个环节：①webhook 入口 ②去抖 pipeline ③gateway 闸 ④决策+知识路由 ⑤review 阈值闸 ⑥outbox 幂等 ⑦MCP 发送 ⑧回写（审查文件集中在 webhooks.rs / gateway.rs / decision.rs / knowledge_router.rs / review/gates.rs / outbox*.rs / mcp.rs）。

主链路**旁挂**的 agent 能力子系统上轮几乎没深审。本批圈定这些模块（全部 file:line 亲验存在，共 ~10.6k 行）。后续批次再依次推进 auth+routes 安全隔离面 / 后台 worker 群 / knowledge_wiki 子系统 / evolution 自优化。

## 范围与分簇

按语义关联分 4 个审查簇，每簇一个审查 subagent：

### 簇A 记忆固化（~3368 行）
- `src/agent/memory.rs`(3291) + `src/agent/consolidation_window.rs`(77)
- **重心**：memoryCard 长期固化（MP-8）的无界 append / 覆盖语义 / 并发写窗口 / 置信门缺失。已知线索（memory `feedback_cautious_profiling`）："memory_summary 无界 append 写侧严谨待修" + "画像/记忆更新须保守，不因一句话盲目画像"——正好深审这两条是否仍成立。

### 簇B 标签体系（~1766 行）
- `src/agent/taxonomy.rs`(1036) + `src/agent/decision_taxonomy.rs`(427) + `src/agent/tag_evidence.rs`(101) + `src/agent/bayesian_slots.rs`(202)
- **重心**：双层标签铁律——"AI 标签不可信 → 三层物理隔离(manual 权威 / confirmed AI / tag_observation)" + "证据 fail-closed" + "bayesian/personality 只写不进决策"。查这些声称的不变量实现层有无旁路。已知线索（memory `project_tag_trust_reform`）：8 条铁律 HOLDS——本批复核是否真 HOLDS。

### 簇C 通用化底座（~3466 行）
- `src/agent/domain_profile.rs`(2454) + `src/agent/domain.rs`(107) + `src/agent/domain_signals.rs`(456) + `src/agent/dimension_registry.rs`(449)
- **重心**：行业无关引擎的扩展点是否有硬编码销售假设、profile 加载/派生/应用（apply_active_profile）的新旧字段不对称、dimension 注册表的默认值口径。已知线索（memory `project_universalization_residuals`）：引擎/契约/知识三层已闭环，残留命门在前端 labelFor——本批查后端引擎层残留。

### 簇D 节流与准入（~1984 行）
- `src/agent/simulation.rs`(265) + `src/agent/pacing.rs`(51) + `src/agent/quiet_hours.rs`(357) + `src/agent/entitlements.rs`(1311)
- **重心**：影子模拟（simulate_user_dialogue）与真实发送路径的隔离性（不误触真实 MCP/outbox）、pacing 账号级间隔边界、quiet_hours 时区/边界、entitlements 权限门的 fail-open/fail-closed 方向。

## 审查方法（沿用上轮验证有效的流程）

- **4 个审查 subagent 并行**，每簇一个，全部继承主会话 Opus（省略 model 参数——memory 亲验 `model:"opus"` 报 400 INVALID_MODEL_ID，省略即继承 opus 满足子 agent 红线）。
- **subagent 硬约束**（写进每个 dispatch 指令）：先 100% 读懂相关代码再下结论；每个 finding 必附亲验的 `file:line` 证据（贴实际代码行）；只读审查不改任何代码；凭猜测/印象的产出打回。
- **两态标注**：`PLAUSIBLE`（纯读码推断因果链）/ `CONFIRMED`（能构造推荐配置下的真实触发）。
- **主控逐条亲验**：subagent 交回的每个 finding，主控用 Read/Grep 亲自复核 file:line 属实性 + 因果链成立性，**驳回夸大/误报**（上轮铁律：subagent 首轮常夸大，主控亲验是唯一防线）。
- **元家族聚焦**：重点找"设计声称的不变量/闭环/口径，实现层有旁路 / 缺口 / 非原子窗口 / 新旧字段不对称"——上轮 53 findings 的根因主线，本批大概率同型。

## 严重度校准（防夸大，沿用上轮）

- **High**：推荐配置下**确定性发生**的核心交互失效 / 红线破坏（如标签不可信铁律被旁路进决策、记忆固化真实污染画像驱动错误决策）。
- **Medium**：需多条件叠加、或依赖 DB/LLM 瞬时故障注入才触发的正确性/一致性缺陷。
- **Low**：观测项 / 边缘场景 / 就绪债 / 死代码 / 文档-代码漂移。
- 每条严重度带**主控裁定理由**（subagent 初判 + 主控校准）。

## 台账格式与产出

- 新建 `docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md`。
- 每条 finding 字段：入口频道 / 所属簇 / 类型 / 严重度（带裁定理由）/ 现象风险 / 根因（亲验 file:line）/ 复现设想 / 验证状态（PLAUSIBLE|CONFIRMED）/ 修复建议 / 状态（Open|Fixed|WontFix）。
- 台账头部：审查范围、分簇、方法论、严重度校准口径、元家族说明。
- **本批只审不修**：先产出完整台账，合并台账 docs PR（像上轮 PR#178）。修复是后续批次的事。

## 后续修复路径

台账产出后按严重度定 P0-P3，每 finding（或同族 findings）独立走 `brainstorming → writing-plans → subagent-driven-development → PR`，与上轮完全一致。

## 约束

- 纯代码/设计层审查，**绝不为"发现问题"去改业务逻辑 / prompt / 阈值 / 词表**（上轮反过拟合红线）。
- 不碰主仓在途工作（主仓被并行会话占在 `feat/principal-auth-exemption` 分支）。
- 审查分支 `docs/agent-capabilities-audit` 基于含 #206 的最新 origin/main（69698eb）。
- 无代码改动、无依赖变更、无 DB 迁移——本批产出纯 docs（台账文件）。

## 非目标

- 不审主链路（上轮已深审）。
- 不审本批外的子系统（auth/routes/worker群/wiki/evolution 留后续批次）。
- 不在本批做任何修复（只出台账）。
