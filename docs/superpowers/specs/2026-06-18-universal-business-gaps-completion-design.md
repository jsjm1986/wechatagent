# 通用化业务逻辑缺口全面补全 设计

> **日期**：2026-06-18
> **基线**：HEAD = `be1c319`（维度 registry + 校验工作已落 + 推送）
> **来源**：用户问"通用基底业务逻辑是否满足要求"，三路 opus 实证核查（贴 file:line）确认主链路已真通用，但挖出两个业务缺口 + 一个工程整洁化项。
> **流程**：brainstorming（本文）→ writing-plans → 实施。
> **范围裁决**：实证推翻路线图旧评估——五闸可配=过度设计（砍）、driver 框架=高风险低收益（缓）。本轮 = 两业务缺口 + C3 轻量。

> **实现状态（2026-06-19 已落码）**：实施计划 `docs/superpowers/plans/2026-06-18-universal-business-gaps-completion.md`，子代理逐任务 + 逐任务交叉验证 + 全分支终审（Ready to merge: Yes，无 Critical/Important）。
> - 模块 A：`DomainProfile.mode_gate_policy_override`（feb8c60/d2b41e9，接 reply.policy 链）+ `ReviewerOrientation.reviewer_fewshot_override`（d6359ac，接 review.system 链）。仿 apply_reviewer_review_focus 子串替换 + 锚漂移护栏 + boundary 红线不纳入。
> - 模块 C：`apply_reply_policy_prompt_overrides` / `apply_review_system_prompt_overrides` per-chain 收敛 + 字节等价守恒测试 + 模块头 4 步约定文档（16a7689，两链刻意不合并）。
> - 模块 B：`relationship_type_suggestions` collection（f1aa0ad）→ 决策后 extract+MachineWrite 校验+upsert 写建议（ce9fd57，fail-soft）→ decision prompt 引导（c4dd92f，仿 render_suspected_deal_guidance）→ REST 审核路由 approve/reject（76709e7，AdminWrite 校验写 contact + workspace 隔离）。**保守红线在结构层闭合**：relationship_type 是 AdminDirect 通道，LLM 经 MachineWrite 越界恒 Reject、合法值也只 upsert 进建议表，全代码无一处在决策侧写 contact.relationship_type——approve 是唯一 contact 写点。
> - **字节等价红线的精确边界**：约束的是 reply.policy + review.system 两条 prompt 链（模块 A/C，DEFAULT None 回落逐字节一致）。模块 B 的 relationship_type 引导是常驻追加在 **task prompt**，DEFAULT 销售域 task prompt 多一段（设计内：「有新证据才产出、无证据零扰动」，行为不变但 token/指令多一段）。
> - 验证：lib 1336/0、四 PBT 36/0、no-takeover lint 0 violations。
> - 终审 triage 的 Minor（全部留后续、无一阻断合并）：写建议失败日志分级（E11000 降 debug/其它 warn）、approve mark/contact update 补 workspace+matched_count 一致性、ValueSource::FreeText 旧债（上轮遗留）。

---

## Context（为什么做 + 实证依据）

三路核查结论：
- **主链路（决策/五闸/状态机/发送/记忆）真通用** —— 全部追到 profile 派生 / None 回落分支，DEFAULT 销售域字节等价，非销售行业能跑通不被阻断。**这一层不动**。
- **缺口①（prompt 话术半通用，中）**：两处销售散文写死、无 profile override 字段。非销售域（情感陪伴）激活后这两段仍讲销售话术。
  - `prompts.rs:958-964`「## 模式与 5 闸的关系」整段：casual/consultative 模式说明是销售散文，新模式（intimate_companion）无任何闸说明。
  - `prompts.rs:1300-1314` reviewer 软闸打分锚点 few-shot：PressureRisk 高压锚 = "今天最后一天…现在就定吧（逼单）"，销售场景写死，非销售域打分尺度被带偏。
- **缺口②（数字分身赋值段命门，高）**：`relationship_type`（客户/同行/朋友）只能 admin 手标，LLM 不自动识别。`dimension_registry.rs:64` 钉死 `AdminDirect` 通道，`src/agent/` 无任何 relationship_type 产出。消费段（per_relationship_operation_mode 多套范式 + resolve_operation_mode 三级回落，planner/mod.rs:938-953）已做扎实——缺的是"谁来确定关系类型"。
- **C3（profile 字段中央接线，低-中）**：`apply_active_profile`（runtime.rs:261）只归并 3 个 runtime 标量，~25 个行为字段散读。本轮新增 3 个字段，借机确立轻量接线约定，不全量重构。

**意图结果**：让非销售行业话术不再串销售味（缺口①）；让数字分身能 LLM 自动识别关系类型、运营一键确认（缺口②，保守闭环）；新字段走统一接线约定（C3）。

---

## 关键实证接口（核查亲核，写码依据，不臆造）

### 软通道现状（纠正了原假设）
- `upsert_candidate(db, scope_account_id, kind, raw_value, evidence: Option<&str>, confidence: i32)`（taxonomy.rs:291）—— 按 `(scope,kind,raw_value)` 幂等 upsert 到 `taxonomy_candidates`，pending 累加 occurrences。
- **但 `taxonomy_candidates` 采纳后写的是 system_taxonomies 字典**（admin_taxonomy_candidates.rs 的 approve 路由），语义="建议入字典"，**不是"给某 contact 打标"**。
- **`agent_generated_signals` 不是软通道**：仅是 `AgentDecision` 的 `Vec<AgentSignal>` 字段（types.rs:202），随 run log 落库，无 collection、无审核路由。
- **结论**：缺口②不能直接搭现有软通道车，需新建 contact 级建议-审核链（候选写入可借 upsert_candidate 的幂等模式，但独立 collection）。

### LLM 决策产出结构
- `AgentDecision.agent_generated_signals: Vec<AgentSignal>`（types.rs:202），`AgentSignal { kind, value, evidence, confidence }`。LLM 可经此产出非标准信号。
- `RawAgentDecision`（types.rs:357）是 LLM JSON 反序列化目标。relationship_type 建议挂这里最小改动。

### reviewer override 切法（缺口①参照）
- `reviewer_orientation: Option<ReviewerOrientation>`（models.rs:1499，含 review_focus/balance_principle 两 `Option<String>`）。
- `apply_reviewer_review_focus`（domain_profile.rs:495/512）：取 `DEFAULT_*` 常量为锚 → `system.replace(old, new)`，锚找不到原样返回（幂等、空覆盖 no-op）。
- 消费链 review/mod.rs:282-304（system prompt 渲染）+ :434（user prompt 侧）。
- **失败模式**：few-shot 是多行段，锚常量必须逐字复刻 prompts.rs 现文，否则 replace 静默失配 → 必须加护栏测试锁锚一致性。

---

## 设计（三模块）

### 模块 A — 缺口① prompt 话术 override

给 DomainProfile 加两个 prompt 类 override：

1. **`mode_gate_policy_override: Option<String>`**（DomainProfile 直接字段）
   - 替换 `prompts.rs:958-964`「## 模式与 5 闸的关系」整段。
   - 切法：提一个 `DEFAULT_MODE_GATE_POLICY` 常量（逐字复刻现 :958-963 的模式-闸说明部分），在 decision prompt 组装处仿 conversation_mode_policy 的剥离/替换逻辑，None 回落 DEFAULT。
   - **boundary_protection 红线段（:964）不纳入可替换范围**——它是跨域恒定的安全红线（所有行业都要守边界保护），override 只替换前面的模式-闸说明（:958-963），:964 始终保留。

2. **`reviewer_fewshot_override: Option<...>`**（进 `ReviewerOrientation` 结构加字段）
   - 替换 `prompts.rs:1300-1314` reviewer 软闸打分 few-shot 锚点。
   - 切法：提 `DEFAULT_REVIEWER_FEWSHOT` 常量（逐字复刻），仿 `apply_reviewer_review_focus` 写 `apply_reviewer_fewshot`，在 review/mod.rs:298 渲染链插一道。
   - **护栏测试**：断言 `DEFAULT_REVIEWER_FEWSHOT` 与 prompts.rs 实际段逐字一致（锚漂移即测试红，防静默失配）。

**红线**：两字段 None → 回落销售 seed；DEFAULT 销售 profile 经全链字节等价。`#[serde(default, skip_serializing_if = "Option::is_none")]`。

### 模块 B — 缺口② relationship_type LLM 识别建议-审核链（完整后端闭环）

**数据流**：LLM 决策时若发现关系性质有新证据 → 产出建议信号 → 落独立建议 collection（不直接生效）→ 运营经 REST 审核 → approve 写 `contact.domain_attributes.relationship_type`（复用已有 AdminDirect validate）。

1. **LLM 产出**：decision prompt 增加引导——"仅当对话出现关系性质的明确新证据时"，在 `agent_generated_signals` 产出 `{kind:"relationship_type", value:<canonical>, evidence:<对话依据>, confidence}`。不是每轮强判（关系类型是稳定属性）。
   - 落地步骤：在决策落库后，从 `agent_generated_signals` 提取 kind=relationship_type 的信号，写入建议 collection（不进 domain_signals 容器、不碰 retain_declared_dimensions 白名单）。

2. **建议 collection**：新建 `relationship_type_suggestions`（typed accessor + 索引以 workspace_id 打头）。字段：`workspace_id / account_id / contact_id / suggested_value / evidence / confidence / status(pending/approved/rejected) / first_seen_at / last_seen_at / occurrences`。按 `(workspace_id, contact_id)` 幂等 upsert（同一 contact 反复建议累加 occurrences/刷证据，不堆积重复行）。

3. **REST 审核路由**（仅后端，前端后一步）：
   - `GET /api/admin/relationship-type-suggestions?status=pending` —— 列待审。
   - `POST .../:id/approve` —— 复用 `validate_dimension_value(AdminWrite)` 校验 suggested_value → 写 `contact.domain_attributes.relationship_type` → mark approved。
   - `POST .../:id/reject` —— mark rejected。
   - 全部强制 workspace 隔离（参照 find_contact_by_id 安全契约）。

4. **建议值校验**：suggested_value 经 relationship_type 字典校验（taxonomy），LLM 臆造的非字典值在写入建议时就 drop（不污染审核队列）。

**红线**：
- 不直接生效——LLM 误判不会切错运营范式（符合"画像更新须保守"）。
- no-human-takeover：审核是"AI 建议 + 运营在事前配置层确认"，客户从不面对真人，措辞用 AI 内部状态名（建议/待审/已采纳），不用"人工接管"。
- DEFAULT 零扰动：无建议时 collection 空、行为与现状一致。

### 模块 C — C3 轻量接线约定

不全量重构 25 字段。借模块 A 新增的 prompt override 字段：
- 在 `apply_active_profile`（runtime.rs:261）附近或 decision prompt 组装处，建立一个清晰的"prompt 类 override 注入点"——把 mode_gate_policy_override / reviewer_fewshot_override 的注入收敛到一处有文档的约定（而非散落）。
- 写一段模块注释/文档说明"新增 prompt 类 profile 字段走这里"。
- 旧 25 字段散读不动（能工作）。

---

## 测试策略

- **模块 A**：① 锚常量逐字一致护栏测试（DEFAULT_* == prompts.rs 实际段）；② None 回落 + DEFAULT 字节等价；③ Some override 真替换（情感域样例 profile 验证模式-闸说明/few-shot 被换）。
- **模块 B**：① 建议 collection 幂等 upsert（同 contact 反复建议累加不堆重复）；② approve 复用 validate 校验越界值被拒；③ approve 写 contact + workspace 隔离；④ LLM 臆造非字典值在写建议时 drop；⑤ 纯函数测"从 agent_generated_signals 提取 relationship_type 信号"逻辑。
- **模块 C**：注入点约定的回归（新字段经注入点正确生效）。
- 全程守基线：lib ≥350/0、四 PBT ≥33/0、no-takeover lint clean。

## 护栏（本项目铁律）

- 反过拟合：prompt override 是"给已存在的话术段加可替换性"，不是针对单条对话调话术；relationship_type 识别引导是通用方法论（"关系性质新证据"），不写死行业。
- DEFAULT 销售域字节等价（模块 A 必守）。
- 画像保守：缺口②不直接生效，运营确认才写（模块 B 核心红线）。
- serde 向后兼容：新字段 `#[serde(default, skip_serializing_if)]`；新 collection 不破老库。
- no-human-takeover：建议-审核措辞用 AI 内部状态名。
- 提交需用户显式批准；commit 精确 add 排除并行会话产物。

## 范围边界（YAGNI）

**本 spec 做**：缺口①两个 prompt override 字段 + 缺口② relationship_type LLM 识别建议-审核后端闭环 + C3 轻量接线约定。

**不做**：五闸数量可配（实证=过度设计，5 维是审查方法论固定本体）；driver 框架抽象（实证=高风险大重构低收益，各 scanner 结构不雷同）；C3 全量 25 字段收拢（中风险，新字段示范即可）；缺口②前端审核面板（前端统一后一步）；规模/多账号批次。

## 验证（端到端）

- 模块 A 提交前：锚护栏测试 + DEFAULT 字节等价 + 情感域 override 替换验证。
- 模块 B 提交前：建议链幂等/校验/approve 写入/workspace 隔离单测 + 基线 + lint。
- 模块 C 提交前：注入点回归。
- 各模块用 subagent 交叉验证（A 验字节等价+锚一致、B 验保守闭环+不直接生效+红线、C 验注入点）后再提交。
