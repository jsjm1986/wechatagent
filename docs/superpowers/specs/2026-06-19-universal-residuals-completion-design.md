# 通用化后端三残留收口 设计

> **日期**：2026-06-19
> **基线**：HEAD = main `b543bbf`（通用化大工程 PR #27 已合并）
> **来源**：用户问"后端业务逻辑通用化是否全部结束"，实证核查（3 opus 子代理 + 主会话亲核）确认主链路 + 引导层已闭环，剩三个残留小尾巴；用户决定"三个都彻底做"。
> **流程**：brainstorming（本文）→ writing-plans → subagent-driven 实施。
> **授权**：用户授权实现时自主选最佳路径，不必逐细节确认。

---

## Context（为什么做 + 实证依据）

PR #27 合并后实证核查 main：运行时主链路（decision/五闸/状态机引擎/记忆/答复）+ 引导层（运营对话→AI 生成 DomainProfile→审核→激活，含前端）已全部从 active DomainProfile 派生闭环。残留三处：

- **H13（最大，真缺口）**：状态机**语义本体**（9 态 goal/advanceSignals/riskRules，`prompts.rs:615-733 default_user_operation_state_machine`）写死销售域，活在 DB `operation_domain_configs`。引擎（`guards.rs:144 check_state_transition`）读 `initial`/`allowFromAny`/`allowedFrom` 标志已泛化、不认状态名，但 **DomainProfile 与状态机本体是两条独立配置线**：profile 切换不带状态机（`apply_active_profile` runtime.rs:261 只覆盖 3 个 runtime 标量、不碰状态机；`activate_domain_profile` domain_profiles.rs:462 不联动 operation_domain_configs），引导层生成 profile 时不生成状态机本体。换行业只能靠 migration 那份销售 seed 或 admin 手动 PUT。
- **H17（数据层已维度化，剩本体）**：`IntentTrajectoryEntry.objection_type`（models.rs:3631）扁平销售字段。值已过 `validate_dimension_value(MachineWrite)` + taxonomy 字典（reaction.rs:647，随 profile 可配），纯 ledger 不进闸（reaction.rs:640 注释）。残留是字段名/结构是销售本体。
- **H18（时区已修完，剩 debounce）**：off_hours 时区错位已由 commit 80f74b7 修完（全 src 无 `.hour()` UTC 残留 + 回归测试 `off_hours_uses_operator_tz_not_utc_hour`）。残留仅 debounce 窗口（webhooks.rs:584 `state.config.message_debounce_window_ms`）已 env 可配但非 per-profile。

**意图结果**：三处全部随 profile 可配，DEFAULT 销售域字节等价，让后端业务逻辑通用化彻底收口。

---

## 设计

### H13 — 状态机本体随 profile（路径 B：引导层联动 publish）

**核心决策**（避免双真相源）：状态机本体**继续单一存储在 `operation_domain_configs`**，不在 DomainProfile 内嵌（不重蹈 `domain_schema_id` 死字段覆辙）。关联靠"同 workspace + activate 动作联动 + 版本机制 + seeded_by 溯源"，**不给 config 加 profile_id 静态外键**。

**数据流**：
1. **引导层联动生成**：`build_profile_generation_prompt`（guide_profile.rs）扩展，AI 生成 profile 时同时产出该行业状态机本体（states + 每态 goal/advanceSignals/riskRules + `initial`/`allowedFrom`/`forbidsProactive` 标志）。
2. **校验 + draft（守"AI 永不自动 verify"红线）**：生成的状态机过 `validate_state_machine`（domains.rs:239 现成结构校验）→ 落 draft（随 profile 一起待审，不自动生效）。
3. **activate 联动 publish**：运营 activate profile 时，通过现成 `publish_operation_domain_version`（admin_ops_versions.rs:45）事务性 publish 一版新 `OperationDomainConfig`（`state_machine` Document 装 AI 生成本体），复用现有 `(version/current_version/previous_version)` 多版本灰度机制。
4. **消费方零改动**：状态机仍存 `operation_domain_configs`，读路径（`check_state_transition` / `format_operation_state_machine_for_prompt` decision.rs:310）按 `(workspace_id, domain, current_version=true)` 读，不动。
5. **回落兜底**：profile 没带状态机本体（DEFAULT / 生成失败 / 校验不过）→ 保留现有 `default_user_operation_state_machine` 销售 9 态。DEFAULT 域字节等价。

**关键接口（已亲核）**：`OperationDomainConfig.state_machine: Document`（models.rs:764，含完整版本字段）、`publish_operation_domain_version`（admin_ops_versions.rs:45）、`validate_state_machine`（domains.rs:239）、`default_user_operation_state_machine`（prompts.rs:615）。

**红线**：AI 生成状态机走 draft+人审+结构校验+回落，绝不自动生效；DEFAULT 销售 9 态字节等价（无 profile 状态机时逐字回落）。

### H17 — intent_trajectory 轨迹维度容器化

仿现成 `MemoryDimension` / `DomainProfile.memory_dimensions`（domain_profile.rs:86 `default_memory_dimensions`）范式：

1. **结构**：`IntentTrajectoryEntry`（models.rs:3623）加通用维度容器 `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")] dimensions: BTreeMap<String, String>`（key=profile 声明的轨迹维度名，value=canonical 取值）。`objection_type: Option<String>` 字段**保留**（`#[serde(default, skip_serializing_if = "Option::is_none")]` 不变，老数据读得回 —— 向后兼容铁律）。
2. **写侧**（`reaction::push_intent_trajectory_entry` reaction.rs:615）：按 active profile 声明的轨迹维度产出，每维过 `validate_dimension_value(MachineWrite)` + 字典校验（复用现链）。**DEFAULT 销售域只写 objection_type 旧字段**（dimensions 容器留空、序列化省略 → 与现状字节等价）；**非销售 profile 写 dimensions 容器**（其声明的轨迹维度）。不双写，避免冗余与歧义。
3. **读侧**（`format_intent_trajectory_hint` reaction.rs:729）：DEFAULT 读 objection_type 旧字段渲染（文案逐字不变）；非销售 profile 读 dimensions 容器，display label 随 profile 维度声明（陪伴"顾虑类型"等）。
4. **reaction prompt**：维度名随 active profile，DEFAULT 销售域 prompt 逐字不变。

**红线**：向后兼容（老 objection_type 数据可读）；DEFAULT 销售域 trajectory 写入/hint 渲染字节等价；reaction prompt 改动守反过拟合（改的是"维度名随 profile"抽象机制，非针对单条对话调话术）。

### H18 — debounce 窗口随 profile

1. **DomainProfile 加字段**：`#[serde(default, skip_serializing_if = "Option::is_none")] debounce_window_ms_override: Option<u64>`。
2. **webhook 去抖路径**（webhooks.rs:584）：先 load active profile，取 `debounce_window_ms_override`，None 回落现有 `state.config.message_debounce_window_ms`（env 默认 4000）。
3. DEFAULT 不设 override = env 默认，字节等价。

**注意**：webhook 去抖是热路径，load active profile 走现成进程级 `DomainProfileCache`（30s TTL，已有），不引入 N+1。

**红线**：DEFAULT（None）行为与现状逐字等价；缓存复用不新增 DB 查询。

---

## 测试策略

- **H13**：① `validate_state_machine` 对 AI 生成本体的结构校验（缺 initial/非法 allowedFrom 引用 reject）；② activate 联动 publish 后 `operation_domain_configs` 新版本 current_version 切换正确（集成测试，testcontainers）；③ profile 无状态机本体 → 回落 DEFAULT 销售 9 态字节等价（纯函数/单测）；④ 引导层生成 prompt 含状态机 schema（e2e，复用 domain_profile_e2e 模式）；⑤ AI 生成状态机落 draft 不自动生效（红线测试）。
- **H17**：① 老 objection_type 数据 round-trip 兼容（serde）；② DEFAULT 销售域 trajectory 写入字节等价；③ profile 声明陪伴维度时按新维度产出（纯函数）；④越界值 drop（复用 validate 链）。
- **H18**：① None 回落 env 默认逐字等价；② Some override 真生效；③ 缓存命中不新增 DB 查询。
- 全程守基线：lib ≥350/0、四 PBT ≥33/0、`check-no-human-takeover` / `check-no-sales-domain` clean。

## 护栏（本项目铁律）

- **DEFAULT 销售域字节等价**（三项必守，helper 守恒 + 锚护栏测试）。
- **AI 永不自动 verify**：H13 状态机本体走 draft+人审+结构校验+回落（最关键红线）。
- **serde 向后兼容**：新字段 `#[serde(default, skip_serializing_if)]`；H17 保留 objection_type 旧字段。
- **反过拟合**：H17 reaction prompt 改的是"维度随 profile"抽象机制，非点对点调话术。
- **不造双真相源**：H13 状态机本体单一存 operation_domain_configs，不内嵌 profile。
- 提交需用户显式批准；commit 精确 add 排除并行会话产物；子代理 model:opus；回复中文。

## 范围边界（YAGNI）

**本 spec 做**：H13 状态机本体随 profile（路径 B 完整闭环）+ H17 轨迹维度容器化 + H18 debounce 随 profile。

**不做**：H13 状态机的运营逐态编辑 UI（生成+审核+activate 即可，逐态编辑复用现有 admin PUT）；前端展示新轨迹维度（前端统一后一步）；debounce 之外的节奏参数 per-profile（按需）。

## 验证（端到端）

- 各模块提交前：对应单测 + DEFAULT 字节等价 + 基线 + lint。
- H13 用 subagent 交叉验证（红线：draft 不自动生效 / 回落 DEFAULT / 不造双真相源 / validate 拦非法状态机）。
- 本地 lib + 四 PBT；集成测试（H13 publish 联动 / H17 round-trip）靠 CI testcontainers。
