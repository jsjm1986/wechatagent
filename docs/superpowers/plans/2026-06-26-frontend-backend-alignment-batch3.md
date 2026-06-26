# 前后端业务对齐 批次3 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 76 缺口路线图中标 `[批次3]` 的全部 MEDIUM 条目 + 批次2 顺延的 C6/C9，共 18 条：补全编辑/操作入口、自治可观测深化、两条前后端闭环。（原列 19 条；**E11 management 高危确认流已在最新 main 完整实现——后端 confirm/reject 端点（management.rs:443/519）+ 乐观锁防 IDOR + 前端确认按钮（command-center/index.tsx:210 + commandStore.confirmCommand:109）均已落地，故从批次3 移除**。）

**Architecture:** 按后端 blast radius 分三组——组一纯前端 16 条（接线已有端点）、组二轻后端 1 条（E9 补 integrity-report 字段）、组三重后端 1 条（F23 suspected_deal 全链：gateway fail-soft 提取 + 专表 + 三端点 + approve 落 staff_confirmed outcome）。组一打底，组三排最后独立隔离。

**Tech Stack:** Rust 2021 / Axum / MongoDB（后端），React 19 + TypeScript + Vite + Zustand（前端），vitest（前端测）、cargo test（后端测）。

## Global Constraints

- 子 agent 一律 `model:"opus"`；回复中文。代码标识符/注释遵循既有约定。
- **无人工接管 CI lint**（`scripts/check-no-human-takeover.sh BASE HEAD`）：`src/agent|routes|evolution` + `frontend/src` 新增行（含注释/JSX 文案）禁含 `人工 / 接管 / takeover / hand-off / 人工接管 / 转人工 / 人工介入 / 人工托管`（测试目录除外）。E2 引荐态文案、F23 待核实文案用业务语义措辞（"AI 已退辅助答疑" / "疑似成交待核实"）。
- **测试基线不回退**：`cargo test --lib ≥350/0`；4 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33/0；`RUSTFLAGS=-Dwarnings cargo check --tests` 0 err 0 warn。本地只跑 `cargo test --lib` + 单 PBT，集成测（`#[ignore]`）留 CI（无本地 Docker）。
- **测试只增量叠加**：不删改旧维度/旧弧/旧金标；扩既有断言只能 ADD 不能弱化。
- **AI 永不自动验证知识红线**：E7 手工新建切片**前端提交必须显式带** `status:"draft"` + `integrityStatus:"needs_review"`（后端裸 POST chunks 默认落 active，不强制 draft）；F23 落正式成交一律人审 + `verification="staff_confirmed"`，AI 永不直写 outcome_events。
- **closed-set 枚举 DB 写点校验**：F23 signal status（pending/approved/rejected）。
- **后端改动 blast radius**：E9 补 integrity-report 字段须保 CockpitView 现有三卡不回归（加可选字段）；F23 gateway extract 必须 fail-soft（`let _ = ...` 吞错，不阻断主决策，受 RunBudget 约束）。
- **前端遵守设计系统**：tokens.css 变量、`.module.css` + import 绑定、4 级层级、蓝=主操作专属、紫=AI 身份专属。**优先复用现有组件**：D4 复用 `ActiveVersionsBar`、F23 复用 `SimpleApproveReject` 模式、E16 复用 products-deals 的 setError 模式。
- **git**：仅在用户要求时提交；只 `git add` 具名文件，**绝不 `git add -A`**（工作区有并行会话的 `scripts/biz-test/*` 未提交改动，绝不触碰）；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **serde rename 铁律**（批次1/2 踩 5 次）：所有后端请求/响应 struct 须核实 `#[serde(rename_all)]` 属性——snake 字段名 ≠ wire 键。嵌套 struct 可独立于顶层有自己的 rename_all。前端 PUT/POST body 键须对齐后端 wire 键，否则 typed 反序列化静默丢键。

## 基线与分支

- 基线：批次1（PR#44）+ 批次2（PR#46，merge `ae54a8f`）已合并 main。当前工作树 = 批次2 合并后状态，本 plan 行号均为最新 main 实证值（已 grep 核实）。
- 分支：批次3 实现应从最新 main 起**新分支**（如 `fix/frontend-backend-align-batch3`）。注意工作区有并行会话 `scripts/biz-test/*` 未提交改动 —— 起分支前需与用户确认如何处理（不可 stash/丢弃他人工作）。

## File Structure（按条目分组，标注创建/修改）

**组一 纯前端（16 条）**
- `frontend/src/stores/userOpsStore.ts` — A2（saveOperatingMemory action）/ A3（saveOperationProfile 扩字段）/ E2（hydrateSelected 读 referred）
- `frontend/src/features/user-ops/legacy.tsx` — A2（memoryDraft 编辑表单）/ A3（last_commitment/follow_up_policy 输入）/ E2（已引荐态展示）
- `frontend/src/types/index.ts` — A2（memoryDraft↔后端映射类型）/ E2（Contact 加 referred 字段）
- `frontend/src/features/ask-human/` 或 autonomy — B4（已裁决历史视图）
- `frontend/src/features/operations/` 或 autonomy — C6+C9（run envelope 视图 + tier 遥测，新建子视图/组件）+ `operationsStore.ts`（拉 /agent-runs）
- `frontend/src/stores/strategyStore.ts` — D5（saveDomainProfile 加 POST 分支）
- `frontend/src/features/system-strategy/index.tsx` — D4（DomainProfilePanel 挂 ActiveVersionsBar）/ D5（新建空白态编辑）
- `frontend/src/features/knowledge/shared.tsx` — E5（related_chunks 解除按钮）
- `frontend/src/features/knowledge/steward.tsx` — E6（文档编辑表单）/ E7（手工新建切片表单）
- `frontend/src/features/knowledge/ReviewChat.tsx` — E8（patch 预览）
- `frontend/src/components/review/ProposalReleaseCard.tsx` — E12（渲染 5 字段；**注意已从 features/evolution 迁到 components/review**）
- `frontend/src/app/Shell.tsx` — E15（workspace 切换器）
- `frontend/src/features/ask-human-config/DeciderChainEditor.tsx` — E16（catch 补 setError）

**组二 轻后端（1 条）**
- `src/routes/knowledge/catalog.rs` — E9（integrity-report 加 anchorsMissing 计数）
- `frontend/src/features/knowledge/cockpit/CockpitView.tsx` + `trustTypes.ts` — E9（三计数改口径 + 渲染 gaps）

**组三 重后端（1 条）**
- `src/routes/mod.rs` — F23（挂 suspected-deals 三路由）
- `src/models.rs` — F23（SuspectedDealSignal struct）
- `src/db/mod.rs` — F23（collection_suspected_deal_signals accessor）
- `src/db/indexes.rs` — F23（ensure_suspected_deal_signals_indexes）
- `src/agent/gateway.rs` — F23（extract_suspected_deal_signal + upsert 调用点）
- `src/routes/admin_suspected_deals.rs` — F23（新建：list/approve/reject 三 handler）
- `frontend/src/features/products-deals/` — F23（疑似成交待核实列表 + 审核）

**执行顺序**：组一（任意序，建议 A2→A3→E2 相邻、D4→D5 相邻、E5→E6→E7→E8 相邻、B4、C6+C9、E12、E15、E16）→ 组二（E9）→ 组三（F23）。

---

## 组一：纯前端

### Task 1: A2 — operating-memory 编辑表单

**Files:**
- Modify: `frontend/src/types/index.ts`（OperatingMemoryDraft 已存在 :389；新增后端映射辅助类型）
- Modify: `frontend/src/stores/userOpsStore.ts:287`（GET 处）+ 新增 `saveOperatingMemory` action（仿 saveRelationshipType :476-496）
- Modify: `frontend/src/features/user-ops/legacy.tsx:252-290`（cockpit 标签 memoryDraft 只读 → 可编辑）
- Test: `frontend/src/__tests__/stores/userOpsStore.test.ts`（新增或追加）+ 组件测

**Interfaces:**
- Consumes: 后端 `PUT /api/contacts/:id/operating-memory`，请求体 `OperatingMemoryRequest`（contacts.rs:47，`rename_all="camelCase"`）四字段均为 **Document（嵌套对象）**：`userUnderstanding` / `relationshipState` / `productFit` / `nextAction`。
- Produces: `saveOperatingMemory: () => Promise<void>` store action。

**关键约束**：前端 `OperatingMemoryDraft`（types/index.ts:389）是 15 个扁平 string 字段（identity/businessContext/...），后端是 4 个 Document。**不能直接透传**——须把扁平 draft 字段归组进四个 Document。映射方案（writing-plans 已定，避免歧义）：
- `userUnderstanding` ← { identity, businessContext, jobsToBeDone, painPoints, motivations, decisionStyle, communicationPreference }
- `relationshipState` ← { sensitivePoints, trustLevel, temperature, lastEmotion, relationshipGoal, doNotDo }
- `productFit` ← { interestedProducts, fitReason }
- `nextAction` ← { ...剩余字段 }（draft 里 next-action 相关字段）

- [ ] **Step 1: 写失败测试（store action 发 PUT + 字段归组）**

在 userOpsStore.test.ts 追加：
```ts
it("saveOperatingMemory PUTs grouped documents", async () => {
  const putSpy = vi.spyOn(api, "put").mockResolvedValue({});
  useUserOpsStore.setState({
    selected: { id: "c1" } as any,
    memoryDraft: { ...emptyMemoryDraft(), identity: "工程师", interestedProducts: "A产品" },
  });
  await useUserOpsStore.getState().saveOperatingMemory();
  expect(putSpy).toHaveBeenCalledWith(
    "/api/contacts/c1/operating-memory",
    expect.objectContaining({
      userUnderstanding: expect.objectContaining({ identity: "工程师" }),
      productFit: expect.objectContaining({ interestedProducts: "A产品" }),
    }),
  );
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts -t "saveOperatingMemory"`
Expected: FAIL（saveOperatingMemory is not a function）

- [ ] **Step 3: 实现 saveOperatingMemory action**

userOpsStore.ts 加（仿 saveRelationshipType :476-496 的 selected 守卫 + try 模式）：
```ts
saveOperatingMemory: async () => {
  const { selected, memoryDraft, currentAccountId } = get();
  if (!selected) return;
  await api.put(`/api/contacts/${selected.id}/operating-memory`, {
    userUnderstanding: {
      identity: memoryDraft.identity, businessContext: memoryDraft.businessContext,
      jobsToBeDone: memoryDraft.jobsToBeDone, painPoints: memoryDraft.painPoints,
      motivations: memoryDraft.motivations, decisionStyle: memoryDraft.decisionStyle,
      communicationPreference: memoryDraft.communicationPreference,
    },
    relationshipState: {
      sensitivePoints: memoryDraft.sensitivePoints, trustLevel: memoryDraft.trustLevel,
      temperature: memoryDraft.temperature, lastEmotion: memoryDraft.lastEmotion,
      relationshipGoal: memoryDraft.relationshipGoal, doNotDo: memoryDraft.doNotDo,
    },
    productFit: { interestedProducts: memoryDraft.interestedProducts, fitReason: memoryDraft.fitReason },
    nextAction: {
      objections: memoryDraft.objections, riskPoints: memoryDraft.riskPoints,
      unknowns: memoryDraft.unknowns, nextGoal: memoryDraft.nextGoal,
      recommendedMove: memoryDraft.recommendedMove, avoid: memoryDraft.avoid,
      timing: memoryDraft.timing, reason: memoryDraft.reason,
    },
  });
  if (currentAccountId) await get().refreshContacts(currentAccountId);
},
```
并在 store 类型接口处声明 `saveOperatingMemory: () => Promise<void>`。新增 `setMemoryDraft: (patch: Partial<OperatingMemoryDraft>) => void`（用于表单 onChange）：
```ts
setMemoryDraft: (patch) => set((s) => ({ memoryDraft: { ...s.memoryDraft, ...patch } })),
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts -t "saveOperatingMemory"`
Expected: PASS

- [ ] **Step 5: legacy.tsx cockpit 标签改可编辑表单**

把 :252-290 区只读 `<strong>/<p>` 改为 input/textarea（绑 memoryDraft 字段 + onChange 调 setMemoryDraft）+ 底部"保存运营记忆"按钮（onClick 调 saveOperatingMemory）。遵守设计系统：用现有 `.module.css` 的 input/textarea 样式类（参照 system-strategy 的 styles.input/textarea），按钮用主操作蓝。**注意 `memoryDraft.timing` 等字段在 OperatingMemoryDraft 类型里确实存在（types/index.ts:389 全 15+ 字段），渲染前核对字段名。**

- [ ] **Step 6: 组件测编辑→提交**

追加组件测：渲染 cockpit，改一个 input，点保存，断言 saveOperatingMemory 被调。

- [ ] **Step 7: 前端校验 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
Expected: 全绿，0 type error

```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/stores/userOpsStore.test.ts
git commit -m "feat(user-ops): operating-memory 可编辑表单 + saveOperatingMemory 归组四 Document(A2)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: A3 — operation-profile 补 last_commitment / follow_up_policy

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts:476-496`（saveRelationshipType → 扩为带两字段，或新增 saveOperationProfile）
- Modify: `frontend/src/features/user-ops/legacy.tsx`（加 last_commitment/follow_up_policy 输入）
- Test: `frontend/src/__tests__/stores/userOpsStore.test.ts`

**Interfaces:**
- Consumes: `PUT /api/contacts/:id/operation-profile`，`OperationProfileRequest`（contacts.rs:31-43，`rename_all="camelCase"`）。本 task 新增提交 `lastCommitment` / `followUpPolicy`（均 Option<String>）。**customer_stage/intent_level 维持只读不提交**（AI 派生）。
- Produces: 扩展后的 profile 保存路径携带两字段。

- [ ] **Step 1: 写失败测试**

```ts
it("saveOperationProfile sends lastCommitment + followUpPolicy", async () => {
  const putSpy = vi.spyOn(api, "put").mockResolvedValue({});
  useUserOpsStore.setState({
    selected: { id: "c1" } as any,
    relationshipType: "customer",
    profileEditDraft: { lastCommitment: "下周回复", followUpPolicy: "每周跟进" } as any,
  });
  await useUserOpsStore.getState().saveOperationProfile();
  expect(putSpy).toHaveBeenCalledWith(
    "/api/contacts/c1/operation-profile",
    expect.objectContaining({ lastCommitment: "下周回复", followUpPolicy: "每周跟进" }),
  );
  // customer_stage/intent_level 不应出现在 body
  const body = putSpy.mock.calls[0][1] as Record<string, unknown>;
  expect(body).not.toHaveProperty("customerStage");
  expect(body).not.toHaveProperty("intentLevel");
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts -t "saveOperationProfile"`
Expected: FAIL

- [ ] **Step 3: 实现**

把 saveRelationshipType（:476-496）扩名为 `saveOperationProfile`（保留 relationshipType，加两字段；或新增独立 action，二选一——选扩 saveRelationshipType 改名，避免两个 PUT 同端点）。新增 store 状态 `profileEditDraft: { lastCommitment?: string; followUpPolicy?: string }` + setter。body：
```ts
await api.put(`/api/contacts/${selected.id}/operation-profile`, {
  relationshipType: relationshipType || undefined,
  lastCommitment: profileEditDraft.lastCommitment || undefined,
  followUpPolicy: profileEditDraft.followUpPolicy || undefined,
});
```
（旧 saveRelationshipType 调用点同步改名，grep 确认无遗漏。）

- [ ] **Step 4: 跑测试确认通过**

Run: 同 Step 2
Expected: PASS

- [ ] **Step 5: legacy.tsx 加两字段输入**

在 relationshipType 编辑区附近加 last_commitment / follow_up_policy 的 input + onChange。customer_stage/intent_level 维持只读展示（明确注释"AI 派生，只读"）。

- [ ] **Step 6: 组件测 + 全量校验 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/stores/userOpsStore.test.ts
git commit -m "feat(user-ops): operation-profile 补 last_commitment/follow_up_policy 编辑(A3)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: E2 — referral 已引荐态可观测

**Files:**
- Modify: `frontend/src/types/index.ts`（Contact 加 referredSpecialistAt?/referredCardId? 可选字段）
- Modify: `frontend/src/stores/userOpsStore.ts:266-281`（hydrateSelected 读 referred 标记）
- Modify: `frontend/src/features/user-ops/legacy.tsx`（详情面板显式展示已引荐态）
- Test: 组件测

**Interfaces:**
- Consumes: contact 的 `domain_attributes["referred_specialist_at"]` / `["referred_card_id"]`（后端常量 models.rs:3246/3248 写入 domain_attributes，无独立顶层字段）。前端经 `domainAttributes` dotted-key 间接取。
- Produces: 详情面板"已引荐态 / AI 已退辅助答疑"指示。

**约束**：纯读展示。**E2 不需要 clear-referral 端点**（撤销是批次1 E1 的事，已落地；E2 只观测）。文案守无人工接管 lint（用"AI 已退辅助答疑"，不用"人工接管"）。

- [ ] **Step 1: 写失败测试（组件渲染已引荐态）**

```tsx
it("shows referred state when referred_specialist_at present", () => {
  const contact = { id: "c1", domainAttributes: { referred_specialist_at: "2026-06-26T00:00:00Z" } } as any;
  render(<PlannerOrDetailComponent contact={contact} />);
  expect(screen.getByText(/已引荐/)).toBeInTheDocument();
});
```
（实现期定具体组件名/渲染位置。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "referred state"`
Expected: FAIL

- [ ] **Step 3: hydrateSelected 读 referred + 详情面板展示**

hydrateSelected（:266-281）加读取：
```ts
const referredAt = (contact.domainAttributes as any)?.["referred_specialist_at"];
const referredCard = (contact.domainAttributes as any)?.["referred_card_id"];
set({ referredSpecialistAt: typeof referredAt === "string" ? referredAt : undefined,
      referredCardId: typeof referredCard === "string" ? referredCard : undefined });
```
（store 加这两状态字段 + 初值 undefined。）详情面板加条件渲染：referredSpecialistAt 存在时显"已引荐 · AI 已退辅助答疑"（含引荐时间）。

- [ ] **Step 4: 跑测试确认通过**

Run: 同 Step 2
Expected: PASS

- [ ] **Step 5: lint + 全量校验 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
Run: `bash scripts/check-no-human-takeover.sh HEAD HEAD`（确认新增文案无禁词——实际用 base commit）
```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/...
git commit -m "feat(user-ops): referral 已引荐态可观测(E2 与批次1 E1 同源)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: D4 — domain-profiles 版本回滚 UI

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx:2198`（DomainProfilePanel 挂 ActiveVersionsBar）
- Test: 组件测

**Interfaces:**
- Consumes: `ActiveVersionsBar`（system-strategy/index.tsx:118-132），props `{ meta, endpointPrefix, resourceLabel, busy, canPublish?, onAfterAction? }`，内部 runAction("publish"|"rollout"|"rollback") 按 endpointPrefix 拼端点。复用样板见 :582（prompt_templates）/ :859（playbooks）。
- Produces: DomainProfilePanel 内的版本回滚控件。

- [ ] **Step 1: 写失败测试**

```tsx
it("DomainProfilePanel renders ActiveVersionsBar with domain-profiles endpoint", () => {
  // 渲染 DomainProfilePanel，断言出现回滚控件 + endpointPrefix 指向 /api/admin/domain-profiles
  render(<DomainProfilePanel busy={false} />);
  expect(screen.getByText(/回滚|版本/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "DomainProfilePanel renders ActiveVersionsBar"`
Expected: FAIL

- [ ] **Step 3: 挂 ActiveVersionsBar**

DomainProfilePanel（:2198）内，在活跃 profile 展示处挂（仿 :582）：
```tsx
<ActiveVersionsBar
  meta={activeProfileMeta}
  endpointPrefix="/api/admin/domain-profiles"
  resourceLabel="行业画像"
  busy={busy}
  canPublish
  onAfterAction={() => { /* 刷新 profiles 列表 */ }}
/>
```
（activeProfileMeta 的来源实现期对齐 DomainProfilePanel 现有 profile 数据；meta 形态见 ActiveVersionMeta 类型。）

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/...
git commit -m "feat(strategy): domain-profiles 版本回滚 UI 复用 ActiveVersionsBar(D4)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: D5 — domain-profiles 手动新建空白配置

**Files:**
- Modify: `frontend/src/stores/strategyStore.ts:338-358`（newDomainProfileDraft 进可编辑空白态 + saveDomainProfile 加 POST 分支）
- Modify: `frontend/src/features/system-strategy/index.tsx`（editing===null 时渲染编辑器而非占位）
- Test: store 测

**Interfaces:**
- Consumes: `POST /api/admin/domain-profiles`（create，无 id）+ 既有 `PUT /api/admin/domain-profiles/:id`（update）。
- Produces: saveDomainProfile 支持 create（editingProfile===null）+ update（有 id）双路。

**现状**：saveDomainProfile（:347-358）写死 PUT + id 入参；editingProfile===null 时编辑区只渲染占位，onSave no-op，永不 POST。

- [ ] **Step 1: 写失败测试（新建走 POST）**

```ts
it("saveDomainProfile POSTs when creating (no editingProfile)", async () => {
  const postSpy = vi.spyOn(api, "post").mockResolvedValue({ id: "new1" });
  useStrategyStore.setState({ editingProfile: null, profileDraft: { profile_id: "p_new", display_name: "新域" } });
  await useStrategyStore.getState().saveDomainProfile();   // 注意签名可能要改
  expect(postSpy).toHaveBeenCalledWith("/api/admin/domain-profiles", expect.objectContaining({ profile_id: "p_new" }));
});
it("saveDomainProfile PUTs when editing existing", async () => {
  const putSpy = vi.spyOn(api, "put").mockResolvedValue({});
  useStrategyStore.setState({ editingProfile: { id: "x1" } as any, profileDraft: { display_name: "改" } });
  await useStrategyStore.getState().saveDomainProfile();
  expect(putSpy).toHaveBeenCalledWith("/api/admin/domain-profiles/x1", expect.anything());
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/strategyStore.test.ts -t "saveDomainProfile"`
Expected: FAIL（POST 分支不存在）

- [ ] **Step 3: 改 saveDomainProfile 双路**

把签名从 `(id: string) => Promise<void>` 改为 `() => Promise<void>`，内部按 editingProfile 判 create/update：
```ts
saveDomainProfile: async () => {
  const { editingProfile, profileDraft } = get();
  if (editingProfile?.id) {
    await api.put(`/api/admin/domain-profiles/${editingProfile.id}`, profileDraft);
  } else {
    await api.post(`/api/admin/domain-profiles`, profileDraft);   // create
  }
  // 刷新 profiles 列表 + 清 editing
},
```
newDomainProfileDraft（:338）置一个最小合法空白 draft（profile_id/display_name 空串而非 {}，保证编辑器能渲染各字段输入）。调用点（system-strategy）同步：editing===null 时也渲染 `<Editor>`（不再占位），onSave 调无参 saveDomainProfile。grep saveDomainProfile 旧调用点改无参。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/stores/strategyStore.ts frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/stores/strategyStore.test.ts
git commit -m "feat(strategy): domain-profiles 手动新建走 POST 修死链路(D5)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: B4 — 请示已裁决历史视图

**Files:**
- Modify: `frontend/src/features/ask-human/index.tsx`（加"已裁决历史"视图/筛选器）+ 对应 store/api
- Test: store/组件测

**Interfaces:**
- Consumes: `GET /api/admin/principal-escalations?status=resolved`（principal_escalations.rs:25-58 已支持，投影 decision / authorizationExpiresAt / resolvedVia）。
- Produces: ask-human 频道的已裁决历史列表。

- [ ] **Step 1: 写失败测试（拉 resolved + 渲染裁决结果）**

```ts
it("fetches resolved escalations and renders verdict", async () => {
  vi.spyOn(api, "get").mockResolvedValue({ items: [
    { shortCode: "E1", decision: { verdict: "approved" }, authorizationExpiresAt: "2026-07-01T00:00:00Z", resolvedVia: "principal_chat" },
  ]});
  // 渲染已裁决历史视图，断言出现 "approved" / 授权到期 / 裁决渠道
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "resolved escalations"`
Expected: FAIL

- [ ] **Step 3: 实现已裁决历史视图**

ask-human 频道加一个 tab/筛选器"已裁决历史"，调 `GET /api/admin/principal-escalations?status=resolved`（核实前端是否已有 principal-escalations 的 api wrapper，无则加），列表展示 shortCode / 裁决结果(decision.verdict) / 授权到期(authorizationExpiresAt) / 裁决渠道(resolvedVia)。**注意 principal_escalations 响应 struct 的 serde rename——核实 wire 键大小写（principal_escalations.rs 的 list handler 返回形态）**。措辞守无人工接管 lint。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/ask-human/index.tsx frontend/src/__tests__/...
git commit -m "feat(ask-human): 请示已裁决历史视图(B4)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: C6 + C9 — Agent 运行日志（run envelope）视图 + tier 遥测

**Files:**
- Modify: `frontend/src/stores/operationsStore.ts:29-34`（拉 /agent-runs）
- Create/Modify: `frontend/src/features/operations/` 或 autonomy 新增 run envelope 视图组件
- Test: store/组件测

**Interfaces:**
- Consumes: `GET /api/agent-runs`（mod.rs:375 → tasks.rs:85 list_agent_runs），返回 `{items}`，每项经 shared.rs:1091 agent_run_json **手工拼 camelCase**：`id / workspaceId / accountId / contactWxid / runId / triggerKind / status / planner / context / knowledgeRoute / decision / review / gatewayResult / error / createdAt`。各阶段是 Document（内部键由写入侧决定）。tier 遥测（tier_used / sufficiency / escalated / forced_full）在 `gatewayResult` 阶段内（C9，实现期 grep gateway_result 写入侧确认内部键形态——可能是 camelCase 或 snake，用通用 key-value 渲染兜底）。
- Produces: run envelope 列表 + 单运行展开（含 tier 遥测展示）。

**约束**：C6 是宿主，C9 是其内的字段展示，**同 task 实现**。顶层阶段键直接消费（camelCase 已就位）；各阶段 Document 内部字段按需展开，未知字段用通用 key-value 渲染兜底（不写死字段名，呼应通用化）。

- [ ] **Step 1: 写失败测试（store 拉 agent-runs）**

```ts
it("loadAgentRuns fetches /agent-runs", async () => {
  const getSpy = vi.spyOn(api, "get").mockResolvedValue({ items: [
    { id: "r1", runId: "run-1", status: "succeeded", triggerKind: "webhook",
      gatewayResult: { tier_used: "lean", sufficiency: 8, escalated: false } },
  ]});
  await useOperationsStore.getState().loadAgentRuns("acc1");
  expect(getSpy).toHaveBeenCalledWith(expect.stringContaining("/api/agent-runs"));
  expect(useOperationsStore.getState().agentRuns).toHaveLength(1);
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "loadAgentRuns"`
Expected: FAIL

- [ ] **Step 3: store 加 loadAgentRuns + 状态**

operationsStore 加 `agentRuns: AgentRunItem[]` 状态 + `loadAgentRuns(accountId)` action（GET /api/agent-runs，可带 accountId/limit query）。AgentRunItem 类型在 types/index.ts 加（顶层 camelCase 键 + 各阶段 `Record<string, unknown>`）。

- [ ] **Step 4: 跑测试确认通过**

Run: 同 Step 2
Expected: PASS

- [ ] **Step 5: run envelope 视图组件（含 tier 遥测）**

operations 或 autonomy 加运行日志视图：列表显 runId/triggerKind/status/createdAt，点开展示各阶段（planner/decision/review/gatewayResult/knowledgeRoute）。**C9**：gatewayResult 阶段内显式提取 tier 遥测字段（tier_used 用哪档 / sufficiency 自评分 / escalated 是否升档 / forced_full 是否强升），其余阶段用通用 key-value 渲染。遵守设计系统。

- [ ] **Step 6: 组件测（含 tier 遥测显示）**

```tsx
it("run envelope shows tier telemetry from gatewayResult", () => {
  // 给含 gatewayResult.tier_used 的 run，断言 tier 字段显示
});
```

- [ ] **Step 7: 全量校验 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/stores/operationsStore.ts frontend/src/types/index.ts frontend/src/features/operations/... frontend/src/__tests__/...
git commit -m "feat(operations): Agent 运行日志 run envelope 视图 + tier 遥测展示(C6+C9)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: E5 — 解除知识关联 unrelate 按钮

**Files:**
- Modify: `frontend/src/features/knowledge/shared.tsx:359-363`（related_chunks 列表项加解除按钮）
- Test: 组件测

**Interfaces:**
- Consumes: `DELETE /api/operation-knowledge/chunks/:id/relate/:target_id`（mod.rs:526，wiki_edit.rs:790 只删关联不删 chunk，返回 `{ok, removed}`）。
- Produces: related_chunks 项的"解除关联"按钮。

- [ ] **Step 1: 写失败测试**

```tsx
it("unrelate button DELETEs relate/:target_id", async () => {
  const delSpy = vi.spyOn(api, "delete").mockResolvedValue({ ok: true, removed: 1 });
  // 渲染含 relatedChunks 的 Inspector，点"解除关联"，断言 DELETE 调 /relate/<target>
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "unrelate"`
Expected: FAIL

- [ ] **Step 3: 实现解除按钮**

shared.tsx:359-363 related_chunks 列表项加"解除关联"按钮 → `api.delete(/api/operation-knowledge/chunks/${chunkId}/relate/${target.chunk_id})`；成功后刷新关联列表（重拉 chunk 或本地移除该项）。relatedChunks 类型见 shared.tsx:133 `{ chunk_id, kind, note? }`。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/knowledge/shared.tsx frontend/src/__tests__/...
git commit -m "feat(knowledge): 解除知识关联 unrelate 按钮(E5)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: E6 — 文档元数据编辑 PUT documents/:id

**Files:**
- Modify: `frontend/src/features/knowledge/steward.tsx`（文档项加编辑表单）
- Test: 组件测

**Interfaces:**
- Consumes: `PUT /api/operation-knowledge/documents/:id`（mod.rs:452，crud.rs:108 **replace_one 整文档替换**）。请求体 `OperationKnowledgeDocumentRequest`（mod.rs:75-103，camelCase）：`title`(必填)、`accountId/sourceName/summary/catalogSummary/rawContent/contentHash`(Option)、`domain/sourceType/status`(有 default)、`routingMap/riskNotes/productTags/businessTopics`(Vec)、`lineIndex/sectionIndex`(Vec<Document>)。
- Produces: 文档编辑表单 → PUT。

**关键约束（整替换陷阱）**：replace_one 非局部 patch。**前端必须回填完整文档字段**再提交——尤其 `rawContent`（不回填会被清空，连带 contentHash 重算丢失，影响 chunk 的 D2 锚点回填）、`summary`/`catalogSummary`/各 Vec 标签。`title` 漏填后端 400，`status` 漏填回落 active。表单初值用当前文档完整字段填充。

- [ ] **Step 1: 写失败测试（PUT body 含完整字段）**

```tsx
it("document edit PUTs full body including rawContent", async () => {
  const putSpy = vi.spyOn(api, "put").mockResolvedValue({});
  // 渲染文档编辑表单（初值含 title/rawContent/summary），改 title，提交
  // 断言 PUT body 含 title + 原 rawContent（未被清空）
  expect(putSpy.mock.calls[0][1]).toEqual(expect.objectContaining({
    title: expect.any(String), rawContent: expect.any(String),
  }));
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "document edit PUTs full body"`
Expected: FAIL

- [ ] **Step 3: 实现文档编辑表单**

steward.tsx 文档项加"编辑"按钮 → 表单（初值用当前文档全字段回填）→ PUT documents/:id。表单可只暴露少数可改字段（title/summary/catalogSummary/标签），但**提交 body 必须带上未编辑的 rawContent/contentHash/各字段原值**（从加载的文档对象取，避免清空）。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/knowledge/steward.tsx frontend/src/__tests__/...
git commit -m "feat(knowledge): 文档元数据编辑(E6 整替换回填全字段防清空)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: E7 — 手工单条新建切片 POST chunks

**Files:**
- Modify: `frontend/src/features/knowledge/steward.tsx`（加"手工新建切片"表单）
- Test: 组件测

**Interfaces:**
- Consumes: `POST /api/operation-knowledge/chunks`（mod.rs:464，crud.rs:192 create_operation_knowledge_chunk）。请求体 `OperationKnowledgeChunkRequest`（mod.rs:167-210，camelCase）：`title`(必填)、`documentId`(Option，可空=游离 chunk)、`body/summary/domain/knowledgeType` 等、`status`(默认 active!)、`integrityStatus`(Option)。
- Produces: 手工新建切片表单 → POST。

**关键红线约束**：裸 POST chunks **不强制** draft/needs_review（默认 status=active）。**E7 表单提交必须显式带 `status:"draft"` + `integrityStatus:"needs_review"`**，否则绕过 verify gate 直接落活跃池（违反"AI 不自动核验、人工新建也先进待审池"红线）。documentId 可空（不强制先选文档）。

- [ ] **Step 1: 写失败测试（提交带 draft/needs_review）**

```tsx
it("manual new chunk POSTs with status=draft + needs_review", async () => {
  const postSpy = vi.spyOn(api, "post").mockResolvedValue({ id: "ch1" });
  // 渲染新建切片表单，填 title + body，提交
  expect(postSpy).toHaveBeenCalledWith(
    "/api/operation-knowledge/chunks",
    expect.objectContaining({ title: expect.any(String), status: "draft", integrityStatus: "needs_review" }),
  );
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "manual new chunk"`
Expected: FAIL

- [ ] **Step 3: 实现新建切片表单**

steward.tsx 加"手工新建切片"按钮 → 表单（title 必填 + body/summary/可选 documentId 选择）→ POST，**body 写死 `status:"draft", integrityStatus:"needs_review"`**（注释说明红线：人工新建也先进待审池，AI 不自动核验）。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/knowledge/steward.tsx frontend/src/__tests__/...
git commit -m "feat(knowledge): 手工单条新建切片(E7 强制 draft+needs_review 守红线)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: E8 — ReviewChat 对话产 patch 实时预览

**Files:**
- Modify: `frontend/src/features/knowledge/ReviewChat.tsx:149`（turn.patch 渲染 diff 预览）
- Test: 组件测

**Interfaces:**
- Consumes: `turn.patch`（当前 :149 仅取为 boolean，patch 内容被弃用）。
- Produces: patch diff 预览 +（可选）左栏 chunk 实时刷新。

- [ ] **Step 1: 写失败测试**

```tsx
it("renders patch preview when turn.patch present", () => {
  // 给一个含 turn.patch 内容的 turn，断言 patch 预览区出现 patch 内容
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "patch preview"`
Expected: FAIL

- [ ] **Step 3: 实现 patch 预览**

ReviewChat.tsx:149 改为收到 turn.patch 后渲染 patch diff 预览（展示 patch 内容/字段变更）。左栏实时刷新可选（若复杂则留 patch 预览即可，避免过度设计）。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/knowledge/ReviewChat.tsx frontend/src/__tests__/...
git commit -m "feat(knowledge): ReviewChat patch 实时预览(E8)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: E12 — evolution proposal 详情 5 字段渲染

**Files:**
- Modify: `frontend/src/components/review/ProposalReleaseCard.tsx`（**注意路径：已从 features/evolution 迁到 components/review**）
- Test: 组件测

**Interfaces:**
- Consumes: `ProposalDetailResponse`（components/review/proposalTypes.ts）。**取值路径有别**：`diffSummary`(:95) / `riskNote`(:99) / `previousPromptVersion`(:100) / `evalMetrics: Record<string,unknown>`(:101) 在 `ProposalDetail`（data.proposal.*）；`cohortRunIds: string[]`(:117) 在 `ProposalDetailResponse` **顶层**（data.cohortRunIds）。
- Produces: ProposalReleaseCard 渲染这 5 字段。

- [ ] **Step 1: 写失败测试**

```tsx
it("ProposalReleaseCard renders riskNote/diffSummary/evalMetrics/cohortRunIds/previousPromptVersion", () => {
  // mock GET proposal 返回含 5 字段，断言 5 字段都渲染
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "ProposalReleaseCard renders"`
Expected: FAIL

- [ ] **Step 3: 渲染 5 字段**

ProposalReleaseCard 加渲染区：riskNote/diffSummary/previousPromptVersion 取 `data.proposal.*`（字符串直显），evalMetrics 取 `data.proposal.evalMetrics`（Record，key-value 渲染），cohortRunIds 取 `data.cohortRunIds`（**顶层**，数组渲染）。遵守设计系统。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/components/review/ProposalReleaseCard.tsx frontend/src/__tests__/...
git commit -m "feat(evolution): proposal 详情 5 字段渲染(E12)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 13: E15 — 多 workspace 切换器

**Files:**
- Modify: `frontend/src/app/Shell.tsx:187`（workspace 文本 → 切换器）
- Test: 组件测

**Interfaces:**
- Consumes: `onSwitchWorkspace(workspaceId)` handler（main.tsx:176-189 已接，POST /api/auth/workspace + reload），经 authStore（:13 引用，:15 setHandlers 注入）。`user.workspaces?: string[]`(authStore:6)、`user.currentWorkspace`(:7)。
- Produces: Shell 侧栏的 workspace 下拉切换器。

**约束**：纯接线（handler 已就绪）。仅在 `workspaces.length > 1` 时显切换器（沿用 Shell:142-144 的 showWorkspace 逻辑）。

- [ ] **Step 1: 写失败测试**

```tsx
it("workspace switcher calls onSwitchWorkspace on select", () => {
  const spy = vi.fn();
  // 渲染 Shell，workspaces=["ws1","ws2"]，选 ws2，断言 spy 被调("ws2")
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "workspace switcher"`
Expected: FAIL

- [ ] **Step 3: 实现切换器**

Shell.tsx:187 把 `<span>{workspace}</span>` 改为下拉/切换器（workspaces.length > 1 时），onChange 调 authStore 的 onSwitchWorkspace。单 workspace 时维持纯文本。遵守设计系统。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/app/Shell.tsx frontend/src/__tests__/...
git commit -m "feat(shell): 多 workspace 切换器接 onSwitchWorkspace(E15)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 14: E16 — DeciderChainEditor 静默吞错补 setError

**Files:**
- Modify: `frontend/src/features/ask-human-config/DeciderChainEditor.tsx:27-29`（裸 catch{} → setError）
- Test: 组件测

**Interfaces:**
- Consumes: products-deals/index.tsx:172/179-180 的 setError 模式（已做的那半参照）。
- Produces: DeciderChainEditor 加载失败显错误而非空列表。

**约束**：**只补 ask-human-config 半**（products-deals 半已做）。

- [ ] **Step 1: 写失败测试（加载失败显错误）**

```tsx
it("DeciderChainEditor shows error on load failure", async () => {
  vi.spyOn(api, "get").mockRejectedValue(new Error("boom"));
  // 渲染 DeciderChainEditor，断言显示错误信息而非空列表
  expect(await screen.findByText(/boom|加载失败/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run -t "DeciderChainEditor shows error"`
Expected: FAIL

- [ ] **Step 3: catch 补 setError**

DeciderChainEditor.tsx:27-29 裸 `catch {}` 改为：
```ts
} catch (e) {
  setError(e instanceof Error ? e.message : String(e));
  setContacts([]);
}
```
组件加 `const [error, setError] = useState<string | null>(null)` + 错误提示渲染（区分真无联系人 vs 加载失败）。

- [ ] **Step 4: 跑测试确认通过 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
```bash
git add frontend/src/features/ask-human-config/DeciderChainEditor.tsx frontend/src/__tests__/...
git commit -m "fix(ask-human-config): DeciderChainEditor 补 setError 区分错误态(E16)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 组二：轻后端

### Task 15: E9 — integrity-report 补 D2 降级字段 + 前端三计数改口径

**Files:**
- Modify: `src/routes/knowledge/catalog.rs:159-189`（integrity-report 加 anchorsMissing 计数）
- Modify: `frontend/src/features/knowledge/cockpit/CockpitView.tsx:78-95`（三计数改口径 + 渲染 gaps）
- Modify: `frontend/src/features/knowledge/trustTypes.ts:109`（解析 anchorsMissing）
- Test: 后端集成测（`#[ignore]`）+ 前端组件测

**Interfaces:**
- Consumes（后端）：`operation_knowledge_chunks` 的 `status` + `source_anchors`（OperationKnowledgeChunk）。D2 降级口径 = `status=="active" && source_anchors.is_empty()`（对齐 digest_inbox.rs:455）。
- Produces：integrity-report 返回体加 `anchorsMissing: usize`（camelCase）；前端 CockpitView 三卡 = 待审草稿(needsReview) / D2降级(anchorsMissing) / 知识缺口(knowledge_gap_signals pending 计数)。
- **知识缺口口径（已消歧义）**：= `knowledge_gap_signals` status="pending" 计数（依据 ask-human spec `2026-06-21-ask-human-unified-channel-phase1.md:1230`，后端 `list_knowledge_gap_signals` 端点已存在 sources_meta.rs:334）。**不是** gaps.length。

- [ ] **Step 1: 写失败的后端集成测（integrity-report 含 anchorsMissing）**

在 `tests/` grep 既有 integrity-report 集成测；无则新建 `tests/integrity_report_d2_e2e.rs`，append `#[ignore]`：
```rust
// 写 2 条 chunk：一条 active 无 source_anchors（应计入 anchorsMissing），一条 active 有 anchors（不计）
// GET integrity-report → 断言 anchorsMissing == 1
```
（testcontainers 设置照既有集成测：db connect → migrations::run → ensure_indexes 顺序。）

- [ ] **Step 2: 跑测试确认失败**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`（确认编译；本地无 Docker 不跑 #[ignore]）
Expected: 编译过，测试体引用 anchorsMissing 字段（后端未加则断言取不到）。

- [ ] **Step 3: 后端 catalog.rs 加 anchorsMissing 计数**

catalog.rs:164 的 while 循环内加（同 cursor 复用，零额外查询）：
```rust
if chunk.status == "active" && chunk.source_anchors.is_empty() {
    anchors_missing += 1;
}
```
返回体（:182 附近）加 `"anchorsMissing": anchors_missing`。**保 CockpitView 现有三卡字段（total/verified/needsReview/rejected）不变**（加可选字段不破现有消费）。

- [ ] **Step 4: 后端编译确认**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误

- [ ] **Step 5: 前端 trustTypes 解析 anchorsMissing + CockpitView 三计数改口径**

trustTypes.ts:109 附近解析层加 `anchorsMissing: typeof o.anchorsMissing === "number" ? o.anchorsMissing : 0`。CockpitView.tsx:78-95 三 MetricCard 改：
- 卡1「待审草稿」= integrity.needsReview（不变）
- 卡2「D2 降级」= integrity.anchorsMissing（**新字段**，替换原错配的 rejected）
- 卡3「知识缺口」= knowledge_gap_signals pending 计数（从 gap-signals 端点取，或复用已有 store 的 gapSignal pending 数）
并渲染此前丢弃的 gaps[]（integrity 报告内的缺口明细，作列表展示）。

- [ ] **Step 6: 前端组件测三计数口径**

```tsx
it("CockpitView shows anchorsMissing as D2 degraded count", () => {
  // 给 integrity={ needsReview:3, anchorsMissing:2, ... }，断言"D2 降级"卡显 2
});
```

- [ ] **Step 7: 全量校验 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
Run: `cargo test --lib 2>&1 | tail -3`（≥350/0）
```bash
git add src/routes/knowledge/catalog.rs frontend/src/features/knowledge/cockpit/CockpitView.tsx frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/... tests/integrity_report_d2_e2e.rs
git commit -m "feat(knowledge): integrity-report 补 D2 降级计数 + CockpitView 三计数改口径(E9)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 组三：重后端（blast 最大，排最后，独立隔离）

> **E11（management 高危确认流）已从批次3 移除** —— 写 plan 后复核最新 main 发现该条已完整实现：后端 `confirm_management_command`（management.rs:443）+ `reject_management_command`（:519）端点 + 乐观锁 `find_one_and_update`（filter `status=pending_confirmation` + workspace_id 防 IDOR）+ 抽公共 `execute_plan_tool_calls`（:93/:131）防 dual-path drift；路由已注册（mod.rs:788/792）；前端确认条 + "确认执行" 按钮（command-center/index.tsx:210-219）+ `commandStore.confirmCommand`（commandStore.ts:109 → POST /confirm）+ 测试（commandCenter.test.tsx:77）均已落地。故组三只剩 F23 一条。

### Task 16: F23 — 疑似成交待核实闭环（方案B 全链）

**Files:**
- Modify: `src/models.rs`（新增 `SuspectedDealSignal` struct，仿 RelationshipTypeSuggestion :2822）
- Modify: `src/db/mod.rs`（新增 `collection_suspected_deal_signals()` accessor，仿 :259）
- Modify: `src/db/indexes.rs`（新增 `ensure_suspected_deal_signals_indexes`，仿 :971，并在 :645 ensure_indexes 处调用）
- Modify: `src/agent/gateway.rs`（新增 `extract_suspected_deal_signal` + upsert 调用点，仿 extract_relationship_type_suggestion :4734 + :3960）
- Create: `src/routes/admin_suspected_deals.rs`（list/approve/reject 三 handler，仿 admin_relationship_suggestions.rs）
- Modify: `src/routes/mod.rs`（挂 /admin/suspected-deals 三路由 + import + mod 声明，仿 :808-819 + :23 + :143-145）
- Modify: `frontend/src/features/products-deals/`（疑似成交待核实列表 + 审核，复用 SimpleApproveReject 模式）
- Test: 后端集成测（`#[ignore]`）+ 前端 vitest

**Interfaces:**
- Consumes: `AgentSignal`（types.rs:56，`rename_all="camelCase"`：kind/value/evidence: Option<String>/confidence: i32）；`AgentDecision.agent_generated_signals: Vec<AgentSignal>`（types.rs:216）。suspected_deal 信号已由 entitlements.rs:302 + decision.rs:609 产生（kind="suspected_deal"，**无需改 prompt**）。`add_outcome_event_inner`（shared.rs:1329）+ `OutcomeEventInput`（shared.rs:1301）：source/marked_by/audit_summary/amount/currency/verification/event_kind/product_id/quantity/note/occurred_at_ms。verification 闭集（validate_deal_verification shared.rs:1286）：None/""/"staff_confirmed"→staff_confirmed，"payment_verified" 直通，**其它（含 conversation_inferred）→400**。
- Produces: `suspected_deal_signals` collection + GET/approve/reject 三端点 + gateway 提取链。

**关键决策（spec 拍板，方案B）**：
- **不需要 migration**（relationship_type_suggestions 也无 migration，索引建在 indexes.rs + ensure_indexes 调）。F23 同样只加 indexes.rs 函数。
- **approve 落正式成交**：调 `add_outcome_event_inner(verification=Some("staff_confirmed"), source="manual", marked_by=<admin>)`——不写 domain_attributes（这是 relationship_type 的做法，F23 不同）。红线：AI 永不直写 outcome，人审 staff_confirmed 才落。
- **gateway extract fail-soft**：`let _ = ...update_one(...)` 吞错，不阻断主决策（受 RunBudget 约束）。

- [ ] **Step 1: 新增 SuspectedDealSignal struct（写失败的序列化测试先行）**

models.rs 加（仿 RelationshipTypeSuggestion :2822，**无 rename_all** → snake BSON）：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspectedDealSignal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_id: String,
    pub value: String,            // "疑似成交·待核实"
    pub evidence: Option<String>,
    pub confidence: i32,
    pub status: String,           // pending | approved | rejected
    pub occurrences: i32,
    pub first_seen_at: DateTime,
    pub last_seen_at: DateTime,
    pub reviewed_at: Option<DateTime>,
    pub reviewed_by: Option<String>,
}
```
加一个 models 单测断言 round-trip（仿既有 model 测）。

- [ ] **Step 2: accessor + 索引**

db/mod.rs 加 `collection_suspected_deal_signals(&self) -> Collection<SuspectedDealSignal>` → "suspected_deal_signals"。indexes.rs 加 `ensure_suspected_deal_signals_indexes`（仿 :971：`(workspace_id, contact_id)` unique + `(workspace_id, status)`），在 ensure_indexes（:645 附近）处调用。

- [ ] **Step 3: 后端编译确认**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误

- [ ] **Step 4: gateway extract_suspected_deal_signal + upsert（fail-soft）**

gateway.rs 加（仿 extract_relationship_type_suggestion :4734）：
```rust
fn extract_suspected_deal_signal(
    signals: &[crate::agent::types::AgentSignal],
) -> Option<(String, Option<String>, i32)> {
    signals.iter().find(|s| s.kind == "suspected_deal")
        .map(|s| (s.value.clone(), s.evidence.clone(), s.confidence))
}
```
在 relationship_type upsert 调用点附近（:3960 区）加 suspected_deal 的 upsert（filter 锚 (workspace_id, contact_id, status="pending")，$setOnInsert 初始 + $inc occurrences + $set last_seen_at/evidence/confidence），**fail-soft**：`let _ = ...update_one(...)`。

- [ ] **Step 5: admin_suspected_deals.rs 三 handler**

新建文件，仿 admin_relationship_suggestions.rs：
- `list_suspected_deals`（GET，?status=pending 默认，workspace 隔离，sort last_seen_at）
- `approve_suspected_deal`（POST :id/approve）：find signal（校验 pending）→ find_contact_by_id（workspace 隔离）→ **`add_outcome_event_inner(state, &contact, OutcomeEventInput{ source:"manual".into(), marked_by:<admin.username>, audit_summary:"疑似成交人审确认".into(), verification:Some("staff_confirmed".into()), event_kind:Some("deal".into()), amount/currency/product_id: 从 body 取（可选）, ...})`** → mark signal approved + reviewed_at/by。
- `reject_suspected_deal`（POST :id/reject，body {reason}）：status=rejected + 记 reason。

- [ ] **Step 6: 挂路由 + import + mod**

mod.rs：`mod admin_suspected_deals;`（仿 :23）、import 三 handler（仿 :143-145）、挂三路由（仿 :808-819）：
```rust
.route("/admin/suspected-deals", get(list_suspected_deals))
.route("/admin/suspected-deals/:id/approve", post(approve_suspected_deal))
.route("/admin/suspected-deals/:id/reject", post(reject_suspected_deal))
```

- [ ] **Step 7: 写后端集成测（`#[ignore]`，全链）**

`tests/` 新建 `suspected_deal_e2e.rs`，append `#[ignore]`：
```rust
// 1. 插一条 SuspectedDealSignal（evidence="客户说要下单", confidence=75, status=pending）
// 2. GET /admin/suspected-deals?status=pending → 断言含该条 evidence/confidence
// 3. POST .../:id/approve（body 带 amount/currency）→ 断言 contact.outcome_events 多一条 verification=="staff_confirmed"，signal status==approved
// 4. POST .../:id/reject 另一条 → status==rejected
```

- [ ] **Step 8: 后端编译 + lib 测**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5` + `cargo test --lib 2>&1 | tail -3`
Expected: 0 警告；lib ≥350/0

- [ ] **Step 9: 前端待核实列表 + 审核**

products-deals 加"疑似成交待核实" Tab/区块：GET /api/admin/suspected-deals?status=pending，列表复用 SimpleApproveReject 模式富展示（依据 evidence / 置信度 confidence / 客户 contactId / 出现次数 occurrences）→ approve（可带成交金额/币种）/ reject。措辞守无人工接管 lint。inboxApi 或新 api wrapper 加对应调用。

- [ ] **Step 10: 前端 vitest + lint**

```tsx
it("suspected deal approve posts staff_confirmed deal", async () => {
  const postSpy = vi.spyOn(api, "post").mockResolvedValue({});
  // 渲染待核实列表，点 approve，断言 POST /admin/suspected-deals/:id/approve
});
```
Run: `cd frontend && npx vitest run && npx tsc --noEmit`
Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`

- [ ] **Step 11: 全量校验 + commit**

Run: `cargo test --lib 2>&1 | tail -3` + `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -3` + `cd frontend && npx vitest run`
```bash
git add src/models.rs src/db/mod.rs src/db/indexes.rs src/agent/gateway.rs src/routes/admin_suspected_deals.rs src/routes/mod.rs frontend/src/features/products-deals/... frontend/src/__tests__/... tests/suspected_deal_e2e.rs
git commit -m "feat(deals): 疑似成交待核实闭环(F23 方案B 全链:gateway提取+专表+三端点+approve落staff_confirmed)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 完工后

全 16 task 完成后：
1. whole-branch 终审（最强模型，range = `git merge-base main HEAD`..HEAD），重点查跨 task 一致性（A2/A3/E2 同改 userOpsStore+legacy；D4/D5 同改 strategyStore+system-strategy；E5/E6/E7/E8 同改 knowledge；F23 后端新端点鉴权/红线）+ serde rename 复核 + 累积 Minor triage。
2. 终审有 findings → 派 ONE fix subagent（完整 findings 列表）。
3. `superpowers:finishing-a-development-branch`：push + 创建 PR（**不自动合并**，除非用户确认），等 CI 双门绿。

## 验收清单（交付时人肉浏览器验收）

标 ✅需浏览器 的条目（spec 标注）：
- A2：选客户 → operating-memory 表单可编辑 → 保存回填。
- A3：last_commitment/follow_up_policy 可编辑提交（customer_stage/intent_level 只读）。
- B4：ask-human 已裁决历史显裁决结果/授权到期/渠道。
- C6+C9：运行日志视图显 run envelope 全链 + tier 遥测。
- D4：domain-profile 版本回滚可用。
- D5：手动新建空白配置可保存（走 POST）。
- E2：联系人详情显"已引荐 · AI 已退辅助答疑"。
- E5/E6/E7/E8：解除关联/文档编辑/手工建切片(draft)/patch 预览。
- E12：proposal 详情显 5 字段。
- E15：多 workspace 可切换。
- E16：决策链编辑器加载失败显错误态。
- F23：✅疑似成交待核实列表 → approve 落 staff_confirmed 成交（红线重点验收）。
