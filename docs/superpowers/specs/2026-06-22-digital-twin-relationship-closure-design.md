# 数字分身：联系人关系类型闭环 设计文档

> 状态：设计已获用户口头批准（2026-06-22），待用户审阅本文件后转 writing-plans。

## 背景与动机

后端"通用化改造"已落地 main：DomainProfile 可为任意行业配置状态机、维度、人格(soul)、五闸策略、relationship_type(数字分身 customer/peer/friend)。但**前端 admin 仍是销售样子**——运营者无法在前端给联系人定义"客户类型"，也看不到激活 profile 的行业化信息。

更深一层的业务诉求（用户 2026-06-22 提出）：同一个微信账号下，不同联系人的关系是**混合、连续、模糊**的——"既是朋友又是业务合作伙伴""潜在供应链""潜在合作伙伴（要介绍我们做什么，但不像销售那样推）"。把人硬塞进固定枚举桶会组合爆炸，且违背本项目一贯的 **agent-first**（靠 LLM 理解语义，不靠关键词/固定枚举）立场。

经 5 路 opus 实证审查（不信 memory 文档，逐条追代码 file:line），原以为的"大工程"塌缩成几个精准小改。本设计据此收敛。

## 审查确证的关键事实（写码依据）

1. **激活 profile 是 workspace 级唯一**：`activate_domain_profile`(domain_profiles.rs:488-496) 把目标置 `is_active=true` 并把同 workspace 其他 profile 全部置 false。运行时 `DomainProfileCache`(domain_profile.rs:965) 是进程级 LazyLock 单例，按 workspace_id 分槽，**不按 account/contact 分**。运行时认定"激活"的充要条件 = `is_active=true AND current_version=true`(domain_profile.rs:1013 reload_from_db filter)。
2. **心智模型正交无冲突**：DomainProfile = workspace 级行业大框架；relationship_type = contact 级子类型，存 `contact.domain_attributes.relationship_type`，经 `resolve_operation_mode`(planner/mod.rs:938) 三级回落 `contact.operation_mode_override ?? profile.per_relationship_operation_mode[rt] ?? profile.operation_mode`。relationship_type **不选 profile，只在同一 profile 内挑一套 OperationMode**。
3. **relationship_type 当前只影响主动触达，不影响对话口吻**：`OperationMode`(models.rs:1806) 只有 funnel/silence/commitment/quiet_hours/calendar/renewal/reactivation 七个触达开关，无 voice/tone/soul。其消费点全在 planner 的主动 follow-up 扫描器；被动回复链路(webhook/gateway/decision.rs)不读 relationship_type。`domain_profile.rs:908-913` 注释明确："口吻分化是独立未启动专题"。
4. **DEFAULT 销售 profile 的 `per_relationship_operation_mode = None`**(domain_profile.rs:803，有逐字节等价护栏 H8)。所以即便运营标了 friend，`resolve_operation_mode` 整段回落 `profile.operation_mode`(销售三全开)——**触达零差异**。只有 companion 类 profile(domain_profile.rs:914-930) 才配了三套范式。
5. **口吻分化能力其实已存在于 `customAgentInstructions`**（这是最关键的发现，否定了"新建 per_relationship_soul"的必要）：
   - 逐联系人、自然语言、上限 1000 字，写入 `PUT /api/contacts/:id/custom-agent-instructions`(contacts.rs:603)，存 `contacts.custom_agent_instructions`(models.rs:147)。
   - 进 system prompt **最末位**(decision.rs:556-567 → assemble_system_prompt decision.rs:1039-1046)，措辞 = "# 运营关于本联系人的特别指令（最高优先级，覆盖 Soul + Policy）... 与 Soul / Policy 冲突时以本指令为准"(decision.rs:563)。
   - Soul 人格层**主动让位**：prompts.rs:800 与 prompts.rs:822 两处自我声明 "custom_agent_instructions 永远覆盖以上默认映射"。即 soul 写死"专业销售顾问"、末位写"用朋友口吻"，**LLM 听末位**。
   - 有真模型测试背书其"覆盖 Soul+Policy"真生效(real_llm_ops_smoke.rs:1868)。
   - 无闸门拦截、不被 review agent 干预。
6. **前端联系人编辑区交互范式现成**：profile tab(legacy.tsx:364-447) 扁平堆叠 `<label>` 块，无弹窗无折叠；下拉用原生 `<select>`(无统一 Select 组件)；保存三件套 saveAssistOverride/saveProfileNote/saveCustomAgentInstructions 结构一致(userOpsStore.ts:485-505)：`api.put → refreshContacts → catch setError(全局横幅 GlobalErrorBanner)`；当前值回显从 `contact.domainAttributes?.["xxx"]` 读(userOpsStore.ts:301-304)。

## 设计总览：3 个有序子项目

两条并行的轴，互不冲突，运营者可只用一条或都用：
- **口吻轴**（自然语言，agent-first）：customAgentInstructions → 影响被动回复口吻。
- **触达轴**（离散枚举，结构化）：relationship_type → per_relationship_operation_mode → 影响主动 follow-up 扫描器。

| 子项目 | 范围 | 规模 | 依赖 |
| --- | --- | --- | --- |
| 1. 口吻分化引导 | 纯前端文案 | 极小 | 无 |
| 3. 前端地基 + 类型录入入口 | 后端 1 端点 + 前端 3 点 | 中 | 无（与 1 独立） |
| 2. 触达分化范式补全 | 后端 profile seed/配置 | 小 | 依赖 relationship_type 已可录入（子项目 3） |

**执行顺序建议**：子项目 1（立即见效、零风险）→ 子项目 3（前端地基 + 录入入口）→ 子项目 2（触达范式，最依赖范式文案设计）。每个子项目独立可交付、独立可验证。

> 注：子项目编号按"价值/讨论顺序"，执行顺序为 1 → 3 → 2。writing-plans 时每个子项目可各自成计划，或合并为一个分阶段计划，由执行时定。

---

## 子项目 1：口吻分化引导（纯前端，极小）

### 结论
口吻分化能力**已经存在**且正是为此而生（见上文事实 5）。运营者用一段自然语言描述"这个人是谁 + 希望 AI 怎么对待"，写进 customAgentInstructions，AI 在被动回复时读懂语义自适应口吻——这天然覆盖混合/模糊关系（"大学同学，可能采购但别推销，先维护关系"逐字对应 Soul 的 casual_relationship 模式），无需归类、无组合爆炸、零新字段。

**唯一真实缺口**：运营者不知道这个框能这么用。它现在的 label「运营人员特别指令（最高优先级，可空）」偏业务感，placeholder 举的也是"已签约老客户不要推销"这种纯业务例子，没人会想到能写口吻/关系。

### 改动
仅 1 处前端文案（`frontend/src/features/user-ops/legacy.tsx:385-393`），不动任何逻辑、不动后端、不加字段：
- **label**：保留"最高优先级"的权威感，补"口吻/关系"用途提示。例：`运营人员特别指令（最高优先级，可空 — 也可描述关系与口吻）`。
- **placeholder**：在现有业务例子基础上，补一个混合关系 + 口吻的例子。例：`例：①这个客户已签约老客户，不要主动推销，只服务问题。②这是我大学同学，他公司可能采购我们产品，但别推销，先用轻松口吻维护关系。Agent 将在每轮对话最末尾读取这段指令，可覆盖默认人格口吻。`

### 可选增强（不在本子项目必做范围，列为备选）
- 在 Soul（prompts.rs:800 区域）或 operator_instruction 注入段（decision.rs:563）措辞里，把"口吻/关系描述"显式列为合法用途（当前是隐含）。**若做必须 bump PROMPT_PACK_VERSION**(prompts.rs)，且受 no-human-takeover lint 约束——故默认不做，留待真模型测试发现确有必要时再议。

### 验证
- 前端 `npm run build` 通过。
- 文案改动无逻辑，靠人工确认 placeholder/label 渲染正确即可。
- 不需要新测试（无新代码路径）。

### 反过拟合守则
不在前端做任何"口吻关键词检测/分类"。运营者写什么是自由文本，AI 语义理解，前端只负责引导和透传。

---

## 子项目 3：前端地基 + 类型录入入口（后端 1 端点 + 前端 3 点，中）

### 3.1 后端：`GET /api/admin/domain-profiles/active`（只读）

**文件**：`src/routes/domain_profiles.rs`（新增 handler）+ `src/routes/mod.rs`（挂载路由，约 :921-952 domain-profiles 区域）。

**契约**：
- 路径 `GET /api/admin/domain-profiles/active`，鉴权 `AuthenticatedAdmin`（与同组端点一致）。
- 查询条件**必须与运行时 DomainProfileCache 逐字一致**：`{workspace_id: <admin.current_workspace>, is_active: true, current_version: true}`。这是硬约束——否则前端显示的 profile ≠ AI 实际加载的，会误导运营者（事实 1）。
- 返回 `{ "item": <profile_view> | null }`：命中返序列化后的 profile，无激活 profile 时返 `{item: null}`（**不报 404、不报错**——无激活是合法状态，运行时此时回落 DEFAULT_PROFILE）。
- 复用现有 `profile_view`(domain_profiles.rs:94) 序列化（整体 snake_case + `_id`→hex 剥离）。

**惊喜/注意**：`profile_view` 会带 `generated_state_machine`(domain_profiles.rs:1683 区域)——AI 生成的状态机 draft 本体，体积大且前端显形用不到。本子项目地基阶段**先原样返回**（简单、与 get/list 口径一致），若后续前端嫌大再裁剪，不提前优化。

### 3.2 前端：profileStore（新建 `frontend/src/stores/profileStore.ts`）

仿 accountStore 范式，zustand `create`，走 `lib/api`：
- 状态：`activeProfile: DomainProfile | null`、`loading: boolean`、`error: string | null`。
- action：`loadActiveProfile()` → `api.get<{item: DomainProfile | null}>("/api/admin/domain-profiles/active")` → `set({activeProfile: item})`。失败兜底 `set({activeProfile: null, error})`（**降级：前端照常跑，只是没有行业化数据**，不阻断 UI）。
- 在 `App.tsx` 启动 useEffect bootstrap 处调一次（仿 :136-143 accounts 那段）。多租户/workspace 切换时需重取（active 是 per-workspace）。

`DomainProfile` 前端类型已是 snake_case（types/index.ts 已有，profile_view 整体 serde），直接复用。

### 3.3 前端：联系人 relationship_type 录入入口（`frontend/src/features/user-ops/legacy.tsx`）

**落点**：profile tab 内，辅助模式下拉块(legacy.tsx:402-419)之后、buttonRow(legacy.tsx:420)之前，新增一个与辅助模式结构完全并列的 `<label>` 块。

**交互（照搬辅助模式三件套，原生 select，简单易用）**：
```jsx
<label>
  <span>客户类型</span>
  <small>影响 AI 的主动触达策略（如：朋友不主动追单、销售对象继续跟进）。</small>
  <select value={relationshipType} onChange={(e) => onRelationshipType(e.target.value)}>
    <option value="">未分类</option>
    <option value="customer">客户（销售型）</option>
    <option value="peer">同行</option>
    <option value="friend">朋友</option>
  </select>
  <button className="secondary" onClick={onSaveRelationshipType} disabled={busy} type="button">
    <SquarePen size={16} />保存客户类型
  </button>
</label>
```

**数据流**：
- 选项**硬编码** customer/peer/friend（m024 seed 的三个 canonical 值 + 中文标签）。审查（事实 5、子项目交互审查 C4）确认：3 个固定枚举硬编码 option 最简；引入 `GET /api/admin/taxonomies?kind=relationship_type` 动态取数是首次引入、成本更高，YAGNI 不做。**注意**：option 的 value 必须用 canonical 英文 id（customer/peer/friend），后端 `update_operation_profile` 经字典校验只认 canonical（contacts.rs:715-727）。
- 回显：从 `contact.domainAttributes?.["relationship_type"]` 读（仿 userOpsStore.ts:301 读 assist_mode_override）。
- 保存：走**已有**端点 `PUT update_operation_profile`(contacts.rs:649，已支持 relationship_type 字段，:715-727 字典校验后写 domain_attributes.relationship_type)。

> **待 writing-plans 时核实**：`update_operation_profile` 对应的前端 HTTP 路径与请求体形状（OperationProfileRequest 含 relationship_type: Option<String>，contacts.rs:31-38）。userOpsStore 当前**无**调用 operation-profile 端点的代码（grep 零命中），故 saveRelationshipType 是首次接这个端点，需确认路由路径（routes/mod.rs domain/contacts 区域）。

**store 改动**（userOpsStore.ts，仿 assistOverride）：
- 新增状态 `relationshipType` + setter。
- `hydrateSelected`(userOpsStore.ts:301) 加一行读 `domainAttributes.relationship_type`。
- 新增 `saveRelationshipType`(仿 :485-505)：`api.put(update_operation_profile, {relationship_type}) → refreshContacts → catch setError`。
- legacy.tsx props 透传 `onRelationshipType / onSaveRelationshipType`（经 index.tsx）。

### 3.4 前端：ChannelDef visibleWhen 机制（`frontend/src/app/channels.ts` + `Shell.tsx`）

- `ChannelDef`(channels.ts:42-52) 加可选字段 `visibleWhen?: (profile: DomainProfile | null) => boolean`。
- `Shell.tsx` 频道渲染处(:156-171 GROUP_ORDER.map → CHANNELS.filter(group))加一道 filter：`visibleWhen` 未定义 → 显示（默认显示，白名单退出）；定义了 → 按返回值。读 profileStore.activeProfile 传入。
- **本子项目只建机制，不给任何频道接规则**（全显示）。用户明确：频道是账号级、客户类型是联系人级，频道门控当前价值不大且易藏错；机制留作后续扩展点。

### 验证
- 后端：`RUSTFLAGS="-D warnings" cargo check --tests` 通过；新端点加单测（命中 active / 无 active 返 null / 查询条件含三件）。`cargo test --lib` ≥350/0 + 四 PBT ≥33/0 不回归。
- 前端：`npm run build` 通过；手动验证联系人编辑区下拉回显 + 保存 + refetch 回显；侧栏频道全显示无消失。
- 端到端：选一个联系人设 relationship_type=friend，保存后回显正确，DB `domain_attributes.relationship_type` 写入 canonical。

### 反过拟合守则
- relationship_type 是 AdminDirect + Taxonomy 通道(dimension_registry.rs:67)，AI 不能自改，运营标注 → 安全。
- 不在前端做类型推断/关键词判断；下拉就是运营者显式选。

---

## 子项目 2：触达分化范式补全（后端，小）

### 目标
让"朋友不被主动追单、销售对象继续跟进"真生效。这是 customAgentInstructions（口吻轴）管不到的——口吻轴只影响**被动回复**的措辞，主动 follow-up 扫描器(planner/mod.rs)是独立的**触达轴**，靠结构化的 OperationMode 开关，不能靠自然语言。

### 现状（审查事实 4）
- 框架已实现：`resolve_operation_mode`(planner/mod.rs:938) 三级回落已接通，6 个扫描器调用点已传 profile。
- companion 类 profile(domain_profile.rs:914-930) 已配三套 per_relationship_operation_mode 范式（customer 日历开 / peer 关漏斗 / friend 关漏斗+承诺）。
- **缺口**：当前 workspace 实际激活的 profile（典型是 DEFAULT 销售或其衍生）`per_relationship_operation_mode = None`(domain_profile.rs:803)，故运营标了 friend 也整段回落、触达零差异。

### 改动方向（writing-plans 时细化，此处定方向）
给"运营者实际会激活的主要 profile"补 `per_relationship_operation_mode` 三套范式。两种落地方式，writing-plans 时择一：
- **(a) seed 迁移**：新增 migration 给某个非 DEFAULT 的"通用销售+关系"profile seed 三套范式。**DEFAULT 销售 profile 必须保持 None**（逐字节护栏 H8，domain_profile.rs:1262，不可破）。
- **(b) 运营前端配置**：让运营者在 system-strategy 的 DomainProfilePanel 里编辑 per_relationship_operation_mode（前端已能编辑 profile_dimensions，扩展到 per_relationship）。这条更通用但前端改造面更大。

**推荐 (a) 起步**：成本小、立即让"类型→触达"闭环可演示；(b) 留作后续 profile 编辑器增强。

### 关键约束
- **DEFAULT 销售 profile 的 per_relationship_operation_mode 永远 None**——逐字节等价护栏(domain_profile.rs:1262 H8)锁死，破坏即测试红。补范式只能加在非 DEFAULT profile。
- OperationMode 范式设计（哪套关漏斗、哪套关承诺催促）是真正的工作量，需贴合"朋友/同行/客户"的真实触达差异，不针对单条对话（反过拟合）。
- 触达分化与口吻分化**完全解耦**（不同消费点、不同数据结构、零文件重叠），可独立交付。

### 验证
- `cargo test --lib` ≥350/0 + 四 PBT ≥33/0；DEFAULT profile 逐字节护栏测试仍绿。
- 补单测：配了范式的 profile 下，relationship_type=friend 的 contact 经 resolve_operation_mode 拿到 friend 范式（关漏斗），customer 拿到 customer 范式。
- 已有 4 个三级回落单测(planner/mod.rs:938 区域)不回归。

---

## 非目标（明确不做，防范围蔓延）

- **口吻分化不新建 per_relationship_soul 字段**——customAgentInstructions 已胜任（事实 5），新建会与之职责重叠（用户明确想避免）。
- **不给 OperationMode 加 voice/tone 字段**——OperationMode 是触达层结构，混入口吻会污染 planner 语义；口吻走 customAgentInstructions 自然语言通道。
- **不做频道门控的具体规则**——只建 visibleWhen 机制（子项目 3.4），频道当前全显示。
- **不引入 taxonomy 动态取数填类型下拉**——3 个固定枚举硬编码最简（YAGNI）。
- **不做维度取值标签的全面行业化翻译**（labelFor 机制 + 替换 user-ops 写死的"客户阶段/意向/异议"等）——这是更大的独立专题，本轮地基(profileStore)为它铺路，但不在本设计范围。
- **不碰 reviewer 行业化**（memory 标"确认不做，重启需先问用户"）。

## 风险与回滚

- **子项目 1**：纯文案，零逻辑风险，`git revert` 即恢复。
- **子项目 3**：新端点只读、新 store 降级兜底 null、新下拉复用成熟范式、visibleWhen 默认显示——单点失败不影响现有 17 频道与现有编辑区。回滚单文件粒度。
  - 最大风险点：`update_operation_profile` 前端路由路径需 writing-plans 时核实（当前前端无调用范例）。核错会导致保存 404，但不影响其他功能。
  - 安全提示：`/active` 端点是只读 admin 端点，无写风险；返回完整 profile 给已鉴权 admin，无越权（与现有 list/get 同权限面）。
- **子项目 2**：补范式只加在非 DEFAULT profile，DEFAULT 逐字节护栏锁死兜底。范式配错只影响该 profile 的主动触达节奏（不影响被动回复、不发错消息），可调。

## 测试基线（不可回归）

- `cargo test --lib` ≥350 passed / 0 failed。
- 四 PBT(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter) 累计 ≥33 / 0。
- `scripts/check-no-human-takeover.sh` clean（本设计全用 AI 内部口径：客户类型/关系类型/relationship_type/主动触达，无禁词）。
- `scripts/check-no-model-hint.sh` clean。
- 子项目 3 后端改动用 `RUSTFLAGS="-D warnings" cargo check --tests` 验证（CI 等价，`--lib` 不够）。
- 新增测试只增量叠加，不删改旧维度/旧断言。

## 关联

- [[project-universalization-residuals]]（本设计是其中 #2 前端 + relationship_type 闭环的收敛）
- [[referral-card-push-design]]（assist_mode 下拉是本设计类型下拉的交互范本）
- [[project_agent_first_no_keyword_filters]]（口吻走自然语言而非枚举的立场依据）
- [[feedback_no_overfitting]]（范式/文案不针对单条对话）
