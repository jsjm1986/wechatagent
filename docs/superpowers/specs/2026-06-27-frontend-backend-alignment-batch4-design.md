# 前后端业务对齐修复 批次4（P3 收尾：通用化维度补全 + 可观测细节 + SSE 韧性 + D9）设计

> 本 spec 把批次4 的 9 条从父 spec `2026-06-26-frontend-backend-alignment-fixes-design.md` 的全量 76→67 路线图细化为可直接 writing-plans 的详设。父 spec 是全量路线图，本文件只覆盖批次4。

- 状态：设计稿，待用户审。
- 基线：批次1（PR#44）+ 批次2（PR#46）+ 批次3（PR#47，merge `e8f353a`）均已合并 main，CI 双门全绿。批次4 实现须基于此最新 main。
- 条目集（用户拍板）：**9 条** = A 组真缺口 4（D10 / D11 / F11 / F14）+ B 组体验打磨 4（F13 / F15 / F16 / F17）+ D9（有意缺口转做）。
- **行号说明**：本 spec 的 file:line 是核实当下（批次3 合并后）的实证值，已逐条用 Explore 子 agent + 直接 Read 核对。writing-plans 阶段须基于最新 main 复核行号（批次1-3 改过的文件行号可能再变）。

---

## 一、批次4 的性质（收尾批）

批次1（功能性中断 + 核心能力）、批次2（通用化前端断裂单一反模式）、批次3（跨域杂项 MEDIUM）依次交付后，剩余即本批：**P3 增强 + 体验打磨 + 1 条有意缺口转做**。四条主线：

1. **通用化维度编辑补全**（D10/D11）：后端字段早就绪，前端编辑器缺输入入口，"改字典/配维度即通用"在 UI 走不通的最后两块。
2. **可信度 / 可观测细节**（F11 confirmedBy 来源 / F13 gatewayStatus 中文化 / F15 加载态）。
3. **explore / SSE 韧性**（F14 死控件移除 / F16 SSE 退避重连 / F17 stale closure 修复）。
4. **D9**（domain-schemas 写表单）：有意缺口转做，后端 CRUD 齐全，补前端表单 + 改文案承诺。

**与批次1-3 的关键区别**：本批**几乎纯前端**。除 D9 复用已存在的后端 domain_schemas CRUD 外，**无任何后端逻辑改动、无新端点、无 migration、无新集成测**。基线门只需 `cargo test --lib` 不回退（前端改动不影响）+ 前端 vitest/tsc/lint。

---

## 二、已实证厘清的关键边界（writing-plans 须沿用）

- **C9 三态遥测不在本批**：tier_used/missingTier/forced_full 真值的数据源是 run envelope 的 decision 文档（`/api/agent-runs` → `run.decision.sufficiency/missingTier`），批次3 的 C6+C9 task 已通过 `operations/index.tsx:82-112` 的 `tierTelemetry()` 正常读出并在运行日志 tab 渲染（:432-437）。**C9 已闭环，不依赖 events.detail**。原以为的"C9 范围缺口"经核实不成立。
- **C10 不在本批**：events.detail 投影是独立的"事件 feed 也能看结构化 detail"增强，价值中低，非 C9 根因，留后续按需。
- **D9 不碰 AI 知识验证红线**：`domain_schemas` 是行业字段表定义（把行业差异下沉到 `chunks.domain_attributes` 的约束层），纯管理员配置项。create 落 `is_active=false`，靠 `activate` 路由切换 active（同 workspace 至多一条 active）。这与"AI 生成知识 status=draft+needs_review"红线是两回事——后者管知识 chunk 的可信度，前者管字段表 schema 的定义。`validate_schema_payload`（domain_schemas.rs:427）已有完整校验（字段名黑名单/enum allowed_values/alias 指向合法）。
- **剔除项（已做掉/证伪/有意废弃，不在本批）**：F2（批次3 E6 已消费 GET documents/:id）/ E13（批次2 已做 formatScores 动态遍历）/ F7（operation-state-policies 列表已自足，GET :id 非必需）/ F8（decision-reviews/:id 详情与列表同投影，无诊断字段增量）/ E4（pack 修复后端 propose 已显式下线 `repair.rs:543`，items 集合已删）/ F6（gap-digest usage 聚合 logs/analyze + per-chunk usageStats 已覆盖）/ F1（operating-memory 已透出主体，仅 initial 态种子回落边角）/ F12-distortionRisks（ReviewChat 已渲染，PARTIAL-REFUTED 成立，仅 provenance 真冗余但低价值不做）。

---

## 三、条目分组（= 后续 plan 的任务簇）

| 组 | 条目 | 后端改动 | 风险 |
| --- | --- | --- | --- |
| 组一 纯前端·后端就绪 | D10 / D11 / F11 / F14 | 零 | 低 |
| 组二 体验打磨 | F13 / F15 / F16 / F17 | 零 | 低 |
| 组三 有意缺口转做 | D9 | 零（CRUD 已存在）+ 改 UI 文案承诺 | 中 |

**执行顺序**：组一（低风险打底，任意序）→ 组二（体验）→ 组三（D9 最重，独立 task）。

---

## 组一：纯前端·后端就绪（4 条）

### D10. ProfileDimension.participates_in_decision 无 checkbox `[前端]` — 通用化
- **现状**：维度编辑器 `system-strategy/index.tsx:1388-1431` 每行只有 kind / display_name / description 三个 input，无 participates_in_decision 复选框；`:1440` "+添加维度" 硬编码 `participates_in_decision: true`。现状只能建"进决策"维度，无法建"只观测不进决策"维度，已有维度的该标志 UI 不可见不可改。
- **修复**：维度编辑行加 `participates_in_decision` 复选框（读现有值 + onChange 写回）；"+添加维度" 新建项默认 `participates_in_decision: true`（保持现有语义不变），用户可在复选框改为 false 建"只观测"维度。后端 ProfileDimension 已有该字段，纯前端。
- **测试**：vitest 组件测——建"只观测"维度（participates_in_decision=false）后提交 body 含该字段为 false；切换复选断言状态。
- **验收**：需浏览器。

### D11. CoverageDimension.initial_signal / anchor_hint 无编辑 `[前端]` — 通用化
- **现状**：后端 `CoverageDimension`(models.rs:2230) 含 `anchor_hint`(:2240) + `initial_signal`(:2251)；前端 type `CoverageDimension`(types/index.ts:547-552) 只有 key/display_name/required/anchor_hint，**缺 initial_signal**（后端发出会被前端 type 丢弃）；编辑器(system-strategy/index.tsx:1651-1684) coverage 维度行只有 key/display_name/required 复选，无 anchor_hint、无 initial_signal 输入。前端新建 completeness 维度的 degraded 审计因 initial_signal 缺失恒 missing。
- **修复**：① 先补 types `CoverageDimension.initial_signal?: string`；② coverage 维度编辑器加 anchor_hint + initial_signal 文本输入（读 + 写回 + 提交）。
- **测试**：vitest 组件测——编辑 anchor_hint/initial_signal 后提交 body 含两字段；type 补全后 round-trip 不丢 initial_signal。
- **验收**：需浏览器。

### F11. AI 确信层 confirmedBy 来源未展示 `[前端]`
- **现状**：后端 `ConfirmedTag` 已 emit `confirmed_by`→wire `confirmedBy`，语义闭集 `strong_evidence`（强证据快通道）| `consolidation`（压缩重判）（models.rs:256-257 注释）。前端 type `ConfirmedTag`(types/index.ts:51) 已有 `confirmedBy: string` 字段，但 `TagTrustPanel.tsx:96-98` 的 aiChip 只渲染 `tag.value` + evidenceCount，**未消费 confirmedBy**，运营无法区分某 AI 标签是"强证据快确信"还是"压缩重判确信"。
- **修复**（用户拍板：徽标 + 说明 tooltip）：aiChip 内加来源徽标——`strong_evidence`→「强证据」、`consolidation`→「压缩重判」、其它/缺省→不显或显原值；徽标带 tooltip（title 或既有 tooltip 机制）说明两种来源的可信度差别（强证据=直接证据快通道确信；压缩重判=记忆压缩时整体重新判定确信）。
- **设计系统**：TagTrustPanel 是 AI 身份区（紫色系，文件头注释明示），徽标用紫系/中性，不误用蓝（主操作专属）。
- **测试**：vitest 组件测——给 confirmedBy=strong_evidence 断言显「强证据」、consolidation 断言显「压缩重判」；缺省值不崩。既有 tagTrustPanel.test.tsx 用例不删改。
- **验收**：需浏览器。

### F14. explore 任意租户 id 输入框（误导性死控件）`[前端]`
- **现状**：前端 `knowledge/explore.tsx:46-61` 有 workspaceId state + localStorage 持久化 + 输入框(:217-226)，并在 ask(:97) / stream(:116-117) 请求里携带。**后端无条件忽略**：`ask_knowledge`(sources_meta.rs:534) 与 stream(:634) 都执行 `let workspace_id = admin.current_workspace.clone();`，请求结构体字段标注「已废弃：服务端忽略此字段，一律用 session 的 current_workspace（防跨租户读取）」(sources_meta.rs:496-498) + `#[allow(dead_code)]`。用户以为能切租户实际无效——误导性死控件，存在认知安全隐患（运营误判在查别的租户）。
- **修复**：移除 workspaceId 输入框 + 相关 state + localStorage 读写 + 请求携带。切租户的正确路径是 `POST /api/auth/workspace`（已有，批次3 E15 的 workspace 切换器）。**纯删除**，请求不再带废弃字段（后端 #[allow(dead_code)] 字段保留，无需动后端）。
- **测试**：vitest 组件测——explore 渲染后无租户输入框（testid/placeholder 不存在）；提交请求不含 workspaceId（或后端忽略不影响）。
- **验收**：需浏览器。

---

## 组二：体验打磨（4 条）

### F13. command-center gatewayStatus 裸英文展示 `[前端]`
- **现状**：`command-center/index.tsx:75` `gatewayStatus` 走 `String(gatewayStatus)` 裸展示，无中文 label map。后端闭集 `GATEWAY_STATUS_VALUES`(run_envelope.rs:86-135) 共 **32 个值**。同文件已有 `callStatusLabel`(:30-45) 中文映射样板（switch + default 回落原值）。
- **修复**：加 `gatewayStatusLabel(status: string): string` 中文 label map（参照 callStatusLabel 模式，覆盖后端 32 值闭集，default 回落原值——保证未来新值不崩）。:75 用它替换裸 String()。
- **边界**：32 值需逐一核对 run_envelope.rs:86-135 的闭集，中文用业务语义（守无人工接管 lint：避开禁词）。
- **测试**：vitest 组件测——给若干 gatewayStatus 值断言中文标签；未知值回落原值。
- **验收**：需浏览器。

### F15. Operations 加载态缺失 `[前端]`
- **现状**：`operationsStore.ts` state 无 loading 字段（:6-16）；`loadOperationsData` 失败静默置空(:54-60)；operations/index.tsx 挂载即 loadOperationsData(:162-164)，各 tab 数据空时直接 EmptyState(:204/251/279/309/362)，首帧/拉取中与"真的没数据"视觉无法区分。
- **修复**：operationsStore 加 `loading: boolean` 字段（loadOperationsData 开始置 true、finally 置 false）；index.tsx 各 tab 在 loading 时显加载态（而非 EmptyState），加载完成后才区分空态 vs 数据。
- **边界**：与批次3 C1 错误态（loadFailed）正交——loading（拉取中）/ error（失败）/ empty（成功无数据）三态清晰。
- **测试**：vitest store 测 loading 生命周期（开始 true→结束 false）；组件测 loading 时显加载态非 EmptyState。
- **验收**：需浏览器。

### F16. SSE 断连无重连 `[前端]`
- **现状**：`today.tsx:138/826` + `explore.tsx:155/160` 的 SSE error handler 主动 `es.close()`，关闭浏览器原生 EventSource 自动重连，且无任何退避补偿。全仓无现成退避重连可参照（chunk WebSocket 也无退避实现，grep 空）。断连后需用户手动重试/重新提交。
- **修复（用户拍板：完整指数退避自动重连）**：抽一个 SSE 重连工具（如 `lib/sseReconnect.ts` 或 hook），实现指数退避（base × 2^attempt，带上限如 30s）+ 重试次数封顶（如 5-8 次）+ 成功后重置 attempt。today.tsx / explore.tsx 的 SSE error handler 接入：断连后自动退避重连，达上限才放弃并提示。**抽共享工具避免两处重复**（DRY）。
- **边界**：重连要正确清理旧 EventSource（避免句柄泄漏）；组件卸载/用户主动取消时停止重连（不与卸载竞争）；重连期间 UI 给"重连中"提示。注意 explore 的 stream 与 today 的两处 stream（聊天历史 + attachStream）形态可能不同，工具要够通用或分别接入。
- **测试**：vitest 测重连工具——模拟 error 触发退避（用 fake timers 断言 setTimeout 间隔指数增长）；达上限停止；成功重置。组件层至少冒烟测接入不崩。
- **验收**：需浏览器（断网/服务重启场景）。

### F17. explore stale closure 误抑制错误横幅 `[前端]`
- **现状**：`explore.tsx:150-158` submitStream 的 error handler 闭包捕获创建时刻的 `result`；上一轮有结果（result 非 null）、用户再次提交时 `resetForSubmit()`(:75) 的 `setResult(null)` 是异步 state，闭包捕获的仍是旧非空 result → 新查询失败时 `!result` 为 false → 错误横幅被误抑制。罕见时序，影响仅"该报错没报"。
- **修复**：用 ref 跟踪 result（resultRef），error handler 读 `resultRef.current` 最新值而非闭包捕获值；或在 error 帧内重新判断。**与 F16 同改 explore.tsx 的 SSE 处理，建议同 task 或相邻**（都在 explore SSE error handler 区）。
- **测试**：vitest 组件测——连续两次提交（第一次成功、第二次失败），断言第二次失败时错误横幅出现（不被旧 result 抑制）。
- **验收**：需浏览器。

---

## 组三：有意缺口转做（1 条，blast 最大）

### D9. domain-schemas CRUD 写操作无入口 `[前端 + 改文案承诺]`
- **现状**：后端 CRUD **全齐**——`POST /admin/domain-schemas`(create, domain_schemas.rs:197)、`PUT /admin/domain-schemas/:id`(update, :238)、`DELETE /admin/domain-schemas/:id`(delete, :294，不允许删 active)、`POST .../:id/activate`(:331)。前端 `atlas.tsx` DomainSchemaTab 只有 load(:497) + activate(:512)，字段区(:590-616)纯只读。UI 文案 `atlas.tsx:544` 明示「字段表由系统管理员维护…不能直接改内容」、空态 `:552`「…后台创建后…供切换」——**有意缺口的文案承诺**。
- **修复（前端 + 文案）**：
  1. DomainSchemaTab 加 create / edit / delete 表单 → 对接后端 CRUD。
  2. **同步改 :544 / :552 文案承诺**（去掉"不能直接改内容/后台创建"，改成可在此创建/编辑/删除），否则自相矛盾。
- **serde 命门（UpsertRequest，domain_schemas.rs:93-105，`rename_all="camelCase"`）**：前端提交 body wire 键须 camelCase：`schemaId`（必填非空）/ `name`（必填非空）/ `fields`（数组，每项 `DomainFieldPayload`：`name` / `label` / `kind`(string|enum|number|date|reference) / `required`(bool) / `allowedValues`(enum 时必填非空数组) / `aliasOf`）/ `aliasDict`（JSON object {中文别名: canonical字段名}）/ `guardDsl`（可选）/ `workspaceId`（可选，缺省用 session current_workspace）。
- **复用后端校验**：`validate_schema_payload`(domain_schemas.rs:427) 已做 fields≤64 / 字段名黑名单 / 名唯一 / kind 合法 / enum allowed_values 非空 / alias 指向合法。前端做基本必填校验即可，越界交后端 400 提示。
- **边界**：create 落 is_active=false（不自动激活，靠现有 activate 切换，保持"同 workspace 至多一条 active"不变量）；delete 不允许删 active（后端已拦，前端给提示）。**不碰 AI 知识验证红线**（见第二节，domain_schemas 是字段表定义非知识 chunk）。
- **测试**：vitest 组件测——create 表单提交断言 POST body camelCase 键（schemaId/fields[{allowedValues}]/aliasDict）；edit 提交 PUT；delete active 时显提示。文案改动断言新文案渲染。
- **验收**：✅需浏览器（有意缺口转做，重点验收 + 确认文案承诺一致）。

---

## 四、依赖与顺序

- **组一 4 条**互相独立，任意序。D10/D11 同改 system-strategy/index.tsx（建议相邻）。F11 改 user-ops/TagTrustPanel.tsx。F14 改 knowledge/explore.tsx。
- **组二 4 条**：F13 改 command-center。F15 改 operationsStore + operations/index.tsx。**F16 + F17 同改 explore.tsx 的 SSE 区，建议相邻或同 task**（F17 是 F16 接入时顺带修的 stale closure）。
- **组三 D9** 独立，排最后（blast 最大：表单 + 文案承诺 + serde camelCase）。
- **文件重叠提示**：批次1-3 改过的文件（system-strategy/index.tsx、operations/index.tsx、operationsStore.ts、explore.tsx）行号已变，**plan 任务须基于最新 main 重新 grep 实证行号**。

## 五、不在批次4 范围

- C10（events.detail 投影，独立增强，按需后续）。
- F22 + F12-provenance（chunk 修复落库闭环依赖不存在的 applyAiRepairPatch 前端闭环，价值低，不做）。
- 已剔除项（见第二节）。
- 群运营 / 朋友圈（Phase1 范围外）。

## 六、全局约束（writing-plans 须逐条带入 plan 的 Global Constraints）

- 子 agent 一律 `model:"opus"`；回复中文。
- **无人工接管 CI lint**：`src/agent|routes|evolution` + `frontend/src` 新增行（含注释/JSX 文案）禁 `人工/接管/takeover/hand-off/人工接管/转人工/人工介入/人工托管`（测试目录除外）。F13 gatewayStatus 中文标签、F11 来源徽标文案、D9 文案改动用业务语义措辞避开禁词。
- 测试基线不回退：`cargo test --lib ≥350/0`、4 PBT ≥33/0、`RUSTFLAGS=-Dwarnings cargo check --tests` 0/0。**本批纯前端无后端逻辑改动**，cargo 侧只需不回退（不新增集成测）。前端 `npx vitest run` 全绿 + `npx tsc --noEmit` 0 + lint 0。本地只跑 `cargo test --lib` + 前端测。
- 测试只增量叠加，不删改旧维度/旧弧/旧金标。
- **AI 永不自动验证知识红线**：本批 D9 不碰此红线（domain_schemas 是字段表定义非知识 chunk），但若任何改动触及知识 chunk 写入须守 status=draft + needs_review。
- 前端遵守现有设计系统：tokens.css 变量、`.module.css`、4 级层级、蓝=主操作专属、紫=AI 身份专属（见 `docs/frontend-design-system.md`）。**F11 在 TagTrustPanel 紫色 AI 身份区，徽标不误用蓝；F16 复用退避工具避免重复**。
- git：仅在用户要求时提交；只 `git add` 具名文件，绝不 `git add -A`（工作区有并行会话未提交的 scripts/biz-test/* + scripts/_remote_run.py，绝不 stash/checkout/覆盖）；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`；破坏性 gitops 须显式授权。

## 七、不变量（修复全程守住）

- D9 create 落 is_active=false，靠 activate 切换，保持"同 workspace 至多一条 active"不变量；delete 不删 active。
- D9 前端 wire 键 camelCase 对齐 UpsertRequest rename_all。
- D11 先补 types initial_signal 字段，否则后端发出被前端丢弃。
- F16 重连正确清理旧 EventSource + 组件卸载停止重连（不泄漏、不竞争）。
- F14 移除死控件后，切租户走 /api/auth/workspace（批次3 E15 已有）。
- 所有新增前端文案不含禁词（CI 门）。
- 前端新组件/改动遵守设计系统，优先复用现有组件与模式。

## 八、测试策略

- **前端**：vitest store 测（loading 生命周期、SSE 重连退避用 fake timers）+ 组件测（D10/D11 维度编辑、F11 徽标、F13 标签、F14 无死控件、D9 表单 camelCase 提交）。新增 feature 加 `__tests__/`。
- **后端**：本批无后端逻辑改动，无新增集成测。守 baseline lib ≥350/0 不回退。
- **用户验收**：标 ✅需浏览器 的条目（D9 重点）用户起 dev server 人肉验收。每批交付列清单。
- **回归**：跑 `scripts/check-baseline`（lib 不回退）+ 前端 vitest + tsc + 禁词 lint。
