# 通用化底座可扩展性审查报告

> **审查日期**：2026-06-18
> **基线**：HEAD = `cff6e88`（feat(digital-twin): relationship_type 关系类型轴…B 阶段·后端先行）
> **范围**：后端 only（前端显形层另作专题）。聚焦 4 条扩展轴：新行业接入 / 新维度扩展 / DomainProfile 字段契约 / 数字分身关系类型轴。
> **方法**：所有发现贴 `file:line` 亲核（本项目审查历史常臆造误报，故逐条 Read/Grep 证实或证伪，附录记录证伪项）。零代码改动。

---

## 执行摘要

底座的**运行时引擎层是真通用的**（状态机、五闸、极性、记忆、人格/方法论 override、运营范式三级回落都追到了真消费分支，且严守 None 回落 / DEFAULT 销售域字节等价的反过拟合护栏）。可扩展性的真实摩擦**不在引擎，而在"配置如何诞生"和"扩展点是否收拢"**：

- **轴 A（新行业接入）**：profile（大脑）能 AI 生成→审→激活，闭环完整且红线（draft 不自动生效）守得住。**真实约束**=整条私聊链路硬绑单一 `domain="user_operations"` 锚点；新行业不是"创建新 domain"，而是复用这个 domain、换 profile + 改它的状态机。能自助（有 PUT 状态机路由），但 profile 与状态机分属两套 admin 入口、无关联校验。
- **轴 B（新维度扩展）**：字典 seed 模板高度一致，但**扩展点散落 ≥4 处、无单一 registry**；`objection_type` 是"声明为字典、实现是裸 string"的脱节点。
- **轴 C（profile 字段契约）**：22 个字段几乎全部真驱动（仅 `domain_schema_id` 是死字段），回落契约健壮。**摩擦**=没有中央接线点，加新字段要在 5+ 文件按类型各走各的渲染点。初判的几个"残留硬编码销售域"经亲核**大多是有意权衡**（闸结构 / 迁移兼容回落），被证伪。
- **轴 D（数字分身轴）**：`cff6e88` 落地扎实——三级回落正确、写入经字典归一、DEFAULT 等价。**真缺口**=relationship_type 只有 admin 手动直写、未接 LLM 自动识别通道，"谁是客户/同行/朋友"仍靠人工标。

一句话：**大脑通用化是真的；扩展成本集中在"扩展动作分散、无统一注册/接线点"，以及"单 domain 锚定 + 关系类型靠人工"两个结构约束**。

---

## 逐轴发现

### 轴 A — 新行业从 0 接入

| 编号 | 发现 | 实证 | 定级 | vs 初判 |
| --- | --- | --- | --- | --- |
| A1 | 状态机引擎已泛化，但**新 domain 无创建路径**：`operation-domains` 路由只有 list/get/update/state-machine/reset/publish/rollout/rollback，无 create；`update_operation_domain` 用 `update_one` 不带 upsert，对新 domain key 匹配 0 行静默无效；域来源是写死的 3 个固定枚举 | 路由 `routes/mod.rs:620-631`；handler `routes/domains.rs:87`（无 upsert）、`:170`（reset 仅 find 既有）；种子 `prompts.rs:482 default_domain_configs`（user_operations/group_operations/moment_operations）；引擎 `guards.rs:144 check_state_transition`（读 initial/allowFromAny/allowedFrom，已泛化） | **medium**（自初判 critical 下修） | 部分证实 |
| A1' | **比 A1 更深的结构事实**：整条私聊链路硬绑字面量 `domain="user_operations"`。所以"换行业"≠"建新 domain"，而是复用此 domain、换 active profile + 改该 domain 的状态机。这缓解了 A1（新行业不需 create API），但意味着 group/moment 是平行运营域、非"给新行业用" | `decision.rs:825,884`（`"domain": "user_operations"` 写死）；调用方 `gateway.rs:145,445`、`webhooks.rs:557` | medium | 审查中新发现 |
| A2 | 引导层闭环真能跑且红线守得住：AI 生成的 profile 在 struct 层强制 `is_active=false`/`current_version=false`/`seeded_by="generated_by_ai"`，必须人审 publish+activate；运行时无 active 回落 DEFAULT | `routes/guide_profile.rs:217,275-290`；激活 `domain_profiles.rs`；加载 `domain_profile.rs:load_active_domain_profile`（30s TTL+回落 DEFAULT） | **正确（证实）** | 证实 |

**轴 A 结论**：新行业接入是"换大脑 + 改单 domain 的状态机"模式，**后端可自助、无需改码**（profile 走 AI 生成+审批，状态机走 PUT）。两个真实约束：①profile 与状态机是两套独立 admin 入口，激活新 profile 时无机制校验配套状态机是否匹配新行业的 stage 命名（C2 状态派生靠 customer_stage 同 id 空间，若 profile 维度值与状态机 state key 不对齐会触发 transition_rejected 审计但不阻断）；②单 domain 锚定，未来真要并行多套行业运营（非串行换配置）需打破 `user_operations` 字面量。

---

### 轴 B — 新维度/标签扩展统一性

| 编号 | 发现 | 实证 | 定级 | vs 初判 |
| --- | --- | --- | --- | --- |
| B1 | 加一个新维度字典**扩展点散落 ≥4 处、无单一 registry**：① 新建 migration ② mod.rs 两处注册（mod 声明 + MIGRATIONS 表项）③（仅 LLM 通道）profile `profile_dimensions` 声明 ④（仅 admin 手填）路由 typed 字段。seed 模板本身高度一致（`$setOnInsert` 幂等 upsert + `{scope,kind,value.id}` filter） | 模板 `m020/m021/m023/m024_seed_*.rs`；注册 `migrations/mod.rs:53,159`；声明 `domain_profile.rs:decision_dimension_kinds`；路由 `routes/contacts.rs:31` | **high**（扩展成本核心） | 证实 |
| B2 | `objection_type` **声明为字典、实现是裸 string**：产生侧 `build_intent_trajectory_entry` 直接读 LLM `objectionType` 写入 `IntentTrajectoryEntry.objection_type`，**不经 `check_value`/`normalize`、不进 candidate 通道**；而 taxonomy 模块对它有字典支持（seed+测试），注释也称其"严格字典字段"——声明与实现脱节 | 产生 `reaction.rs:638-648`；模型 `models.rs:3586`（裸 `Option<String>`）；脱节注释 `models.rs:2331,1038`、`types.rs:54`；字典支持 `taxonomy.rs:706,771` | **medium**（自初判 high 下修：它写进 intent_trajectory 轨迹观测、不进五闸/状态机，drift 影响面是"轨迹数据不规整"，非污染决策） | 证实但降级 |
| B3 | typed↔容器双轨：销售两维（customer_stage/intent_level）永远走 typed JSON 键，新维度走 `domainSignals` 容器，靠镜像抹平 | `domain_signals.rs:retain_declared_dimensions`；typed 常量 `agent/types.rs`、`domain_profile.rs SALES_TYPED_DIMENSION_KINDS` | medium | 证实 |
| B4 | 两条写入路径白名单语义不统一：LLM 维度**必须** profile 声明否则被 `retain_declared_dimensions` 丢弃；admin 直写/派生维度（value_tier/relationship_type）**刻意绕开**声明、未知值保留原文 | LLM 路径 `domain_signals.rs:162`；admin 路径 `agent/taxonomy.rs:554 normalize_dimension_value`（不过白名单） | medium | 证实 |

**轴 B 结论**：维度扩展的"原子操作"是清晰的（migration 模板可抄），但**没有一个地方枚举"系统认识哪些维度、各走哪条通道"**——判断"新维度要不要进 profile_dimensions / 要不要加 typed 字段"散落在各 migration 注释里。这是未来加维度时最容易漏步、最易 drift 的地方。`objection_type` 就是这种 drift 的现成样本。

---

### 轴 C — DomainProfile 字段契约与回落安全

DomainProfile（`models.rs:1331+`）22 个字段，亲核每个的运行时消费点：

| 类别 | 字段 | 结论 |
| --- | --- | --- |
| 真驱动（prompt 类） | prompt_fragment / soul_override / methodology_override / conversation_mode_policy / conversation_modes | 真驱动，`decision.rs` 各 render/apply 点，None 回落 |
| 真驱动（runtime 类） | grounding_gate_bypass_without_claim / distrust_self_reported_low_risk / threshold_overrides | 真驱动，**经 `runtime.rs:261 apply_active_profile` 统一注入** |
| 真驱动（planner 类） | operation_mode / per_relationship_operation_mode / stagnation_dimension | 真驱动，`planner/mod.rs resolve_operation_mode` 三级回落 + 停滞计时 |
| 真驱动（知识/审计类） | coverage_dimensions / chunk_roles / answering_mode_profile / reviewer_orientation / commitment_markers / outcome_polarity / business_formulas / memory_dimensions / transaction_facts_enabled / profile_dimensions | 真驱动，分散在 catalog/review/memory/gateway |
| 引导层（不进 runtime） | methodology_generator_preamble | 真驱动（仅引导层生成器） |
| 治理元数据 | version/current_version/previous_version/seeded_by/is_active | 真驱动（多版本灰度） |
| **死字段** | **domain_schema_id** | **零运行时读点**，仅定义+DEFAULT None+被列为"普通字段"注释 |

| 编号 | 发现 | 实证 | 定级 | vs 初判 |
| --- | --- | --- | --- | --- |
| C2 | `domain_schema_id` 是死字段，零运行时消费 | `models.rs:1344`；`domain_profile.rs:628`（=None）；`domain_profiles.rs:559`（注释列为普通字段） | **low** | 证实 |
| C3 | **无中央接线点**：`apply_active_profile` 只覆盖 3 个 runtime 标量字段，注释明确 conversation_modes 派生"不并入本函数"；prompt 类/planner 类/catalog 类字段各走各的渲染点，散在 5+ 文件。加新字段无统一模式 | `runtime.rs:261`（仅 grounding/distrust/thresholds）；其余散在 decision.rs/review/catalog/planner | **high**（扩展摩擦核心） | 证实 |
| C4-a | planner DB 预筛回落键：**比初判好**——主路径已用动态 `<dim>_updated_at`（按 stagnation_dimension 拼），仅在 `$exists:false`（换维度后存量 contact 无新字段）时回落 `customer_stage_updated_at`，是迁移兼容兜底 | `planner/mod.rs:980-990` + 详注 | **low**（自初判 high：合理迁移回落，非漏洞） | **证伪初判** |
| C4-b | reviewer 5 硬闸维度写死 = `ReviewScores` 闸结构本身，已登记 H15"render 后语义等价"豁免，**合理不可配**；answering_mode 三档 key 恒定=域无关认知阶梯，释义/标签可配、档数不可配 | `domain_profile.rs:445`（HARD_GATES）、`:537`（三档 key+可配释义） | **low**（设计约束，已在案） | **大部分证伪** |
| C5 | 回落安全：未发现破坏 None 回落 / DEFAULT 字节等价的字段，护栏处处强调"销售域逐字等价" | `apply_active_profile`/`resolve_operation_mode`/各 render 点 | **正确（证实）** | 证实 |
| C1 | 模块头注释过时（声称"运行时各消费点尚未接线"），实际已接线，易误导后续扩展者 | `domain_profile.rs:1-12` | low（文档债） | 证实 |

**轴 C 结论**：字段契约层非常健康——几乎无死字段、回落安全严守。唯一的真实扩展摩擦是 **C3（无中央 dispatch）**：`apply_active_profile` 本可作为统一注入点，但只长到 runtime 标量这一半。初判的"残留硬编码销售域"经亲核大多被证伪（是闸结构或迁移兼容），这印证了"逐条亲核、不盲信扫描嫌疑"的必要。

---

### 轴 D — 数字分身（客户/同行/朋友）关系类型轴

**基线修正**：审查中发现该轴已提交为 `cff6e88`（非计划假设的"零代码"）。本轴改为审实现质量。

| 编号 | 发现 | 实证 | 定级 |
| --- | --- | --- | --- |
| D1 | 三级回落实现正确：override early-return → relationship_type 查 per_relationship map → unwrap_or_else 回落 operation_mode。DEFAULT（per_relationship=None）逐字等价 | `planner/mod.rs:943-953`（实际代码体，非仅注释） | **正确** |
| D2 | 写入经字典归一：relationship_type 走 `normalize_dimension_value` alias→canonical（比 objection_type 规范）；m024 seed customer/peer/friend 幂等 upsert、取值中性词（兼容 no-human-takeover 红线） | `routes/contacts.rs:651`；`m024_seed_relationship_type.rs` | **正确** |
| D3 | **真缺口**：relationship_type 只有 admin 手动直写路径，**未在 profile_dimensions 声明**→ LLM 无法在对话中自动识别"这是客户还是朋友"并产出该维度。分身的"每种关系一套范式"做好了，但"谁是哪种关系"仍靠运营逐个手标 | `domain_profile.rs:763`（仅注释提及，无 ProfileDimension 条目）；写入仅 `contacts.rs` admin 路径 | **medium** |

**轴 D 结论**：cff6e88 是一次**质量扎实的后端落地**（回落、归一、等价、红线兼容都对）。但要兑现"AI 化身自动托管客户/同行/朋友"的产品愿景，还差一步：把 relationship_type 接入 LLM 自动识别通道（进 profile_dimensions + decision 通道），否则关系类型分配是人工瓶颈。

---

## 扩展成本基线（三张标准动作表）

### 加一个新画像维度

| 步骤 | 文件 | 必须改码? |
| --- | --- | --- |
| 1. 新建 seed migration | `src/db/migrations/mXXX_seed_<dim>.rs`（抄 m023/m024） | ✅ |
| 2. 注册 migration | `migrations/mod.rs` 两处（mod 声明 + MIGRATIONS 表项） | ✅ |
| 3.（仅 LLM 产出维度）profile 声明 | profile 的 `profile_dimensions` 加 `ProfileDimension{participates_in_decision:true}` | ✅（或运营经 UI 编 profile） |
| 4.（仅 admin 手填）路由 typed 字段 | `routes/contacts.rs OperationProfileRequest` + update_operation_profile 调 normalize | ✅ |
| 5. 前端显形（本轮范围外） | frontend | （另专题） |

### 加一个新 DomainProfile 字段

| 步骤 | 文件 | 说明 |
| --- | --- | --- |
| 1. 结构体加字段 | `models.rs DomainProfile`（带 `#[serde(default)]`） | |
| 2. DEFAULT 显式 seed 销售域等价值 | `domain_profile.rs default_domain_profile` | 反过拟合护栏要求字节等价，不靠 Default |
| 3. 选接线路径（**无统一入口**，见 C3） | runtime 标量→`runtime.rs apply_active_profile`；prompt→decision.rs 写 apply_/render_;planner→resolve_operation_mode | 散在 5+ 文件 |
| 4. diff 比对 | `domain_profiles.rs` hot-reload 比对 | |
| 5. 等价性测试 | DEFAULT 字节等价 + 样例 profile 对照 | |

### 接一个新行业

| 步骤 | 入口 | 必须改码? |
| --- | --- | --- |
| 1. AI 生成行业 profile 草稿 | UI →`POST /admin/domain-profiles`（guide_profile 生成 draft） | ❌ 自助 |
| 2. 逐项编辑 profile | UI ProfileEditor | ❌ 自助 |
| 3. publish + activate | UI →`domain_profiles` publish/activate | ❌ 自助 |
| 4. 配 `user_operations` 域的状态机 | UI →`PUT /operation-domains/user_operations/state-machine` | ❌ 自助（但与 profile 是两套入口、无关联校验） |
| 5.（若需新画像维度字典） | 见上表"加一个新维度" | ✅ 改码 |
| 6. 前端显形（本轮范围外） | frontend 仍硬编码销售语境 | （另专题） |

---

## 修复方向建议（仅方向，按价值/风险排序）

1. **【high 价值·低风险】维度 registry 收拢（B1/B4）**：建一个集中声明"系统认识哪些维度、各维度走 LLM 通道还是 admin 直写、是否 typed"的单一源，替代散落在 migration 注释的判断。降低未来加维度的漏步风险。
2. **【high 价值·中风险】profile 字段中央接线点（C3）**：把 `apply_active_profile` 扩成真正的统一注入分发，或建一个 trait/表驱动的字段→消费点映射，让加新字段有单一模式。
3. **【中价值·低风险】objection_type 走字典（B2）**：`build_intent_trajectory_entry` 产出前过 `normalize_dimension_value` + candidate 通道，消除"声明字典实现裸 string"的脱节。**零风险快赢**：它不进决策，改它不动五闸。
4. **【中价值·中风险】relationship_type 接 LLM 识别通道（D3）**：进 profile_dimensions，让 agent 能自动判断关系类型，解除数字分身的人工标注瓶颈。需配反过拟合验证（多 seed）。
5. **【低价值·零风险快赢】清理死字段与过时注释（C1/C2）**：删 `domain_schema_id` 或接线；修 `domain_profile.rs` 模块头过时注释。
6. **【需专项·高价值】打破单 domain 锚定（A1'）**：若产品要并行运营多套行业（而非串行换配置），需把 `user_operations` 字面量参数化。当前串行换配置场景下不紧迫。

---

## 附录：被证伪的初判（诚实记录，防臆造）

本项目审查 agent 历史常臆造误报。本轮逐条亲核后，以下初始扫描嫌疑被**证伪或降级**：

- **C4-a「planner 预筛键写死 customer_stage_updated_at」初判 high → 实为 low**：主路径已动态化，写死键只是存量数据迁移兜底，有详注、DEFAULT 等价。
- **C4-b「reviewer 5 闸 + answering 三档硬编码」初判 high → 实为 low**：5 闸是 ReviewScores 闸结构本身（已登记 H15 豁免）；answering 三档释义/标签可配、仅档数恒定（设计判断，已在案）。
- **B2「objection_type 假字典」初判 high → 实为 medium**：确实脱节，但它写进轨迹观测、不进决策五闸/状态机，影响面是数据卫生而非污染决策。
- **A1「状态机新域无 create = critical 断点」→ 实为 medium**：被 A1'（单 domain 锚定，新行业复用 user_operations、不需建新 domain）缓解；状态机可经 PUT 自助改。
- **基线假设「数字分身轴零代码」被推翻**：审查中实测该轴已提交为 cff6e88，质量扎实。

**未发现**任何破坏 None 回落 / DEFAULT 销售域字节等价的字段——回落契约层是干净的。
