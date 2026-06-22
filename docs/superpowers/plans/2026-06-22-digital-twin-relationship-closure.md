# 数字分身：联系人关系类型闭环 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让运营者在前端给联系人定义关系类型(customer/peer/friend)并能编辑特别指令分化口吻，立起 profileStore 前端地基，给非 DEFAULT profile 补触达范式让类型真生效。

**Architecture:** 三个独立可交付子项目。口吻轴复用现有 customAgentInstructions（只加前端引导文案，零后端）；前端地基新增只读 `/active` 端点 + profileStore + 联系人 relationship_type 下拉（照搬 assistOverride 三件套）+ ChannelDef visibleWhen 机制（默认显示）；触达轴给非 DEFAULT profile seed `per_relationship_operation_mode` 三套范式。

**Tech Stack:** Rust 2021 / Axum / MongoDB(BSON) / serde；React 19 + Vite + TypeScript + zustand。

## Global Constraints

- 回复用户用中文；子代理 ALWAYS `model: "opus"`。
- `cargo test --lib` ≥350 passed / 0 failed；四 PBT(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter) 累计 ≥33 / 0。
- `scripts/check-no-human-takeover.sh` clean（用 AI 内部口径：客户类型/关系类型/relationship_type/主动触达，禁 `人工/接管/takeover/hand-off`）。
- `scripts/check-no-model-hint.sh` clean。
- 后端改动提交前用 `RUSTFLAGS="-D warnings" cargo check --tests` 验证（CI 等价，`cargo check --lib` 不够）。
- **DEFAULT 销售 profile 逐字节等价护栏不可破**：`default_domain_profile` 的 `per_relationship_operation_mode` 永远 `None`（domain_profile.rs:803 区域，护栏 H8 在 domain_profile.rs:1262 附近）。补范式只能加在非 DEFAULT profile。
- 前端遵守现有设计系统：原生 `<select>`（无统一 Select 组件）、保存失败走全局 `GlobalErrorBanner`（useUiStore.setError）、不新建弹窗/不二次确认。
- serde 向后兼容：不破坏现有 DomainProfile / Contact 反序列化。
- 反过拟合：不在前端做关系类型推断/关键词判断；relationship_type 是运营显式选 + 后端字典校验。
- 提交需用户显式批准；commit 精确 `git add` 指定文件，排除并行会话产物（tests/common/*、tests/real_llm_*、tests/redline*、.kiro/specs/universal-test-coverage/*、AGENTS.md、agent_t*.txt、t15_single.txt、.github/workflows/ci.yml）。
- 磁盘纪律：编译前若磁盘紧 `rm -rf target/debug/incremental`。

## 关键实证接口（写码依据，已亲核 file:line）

- **后端激活条件**（运行时口径，`/active` 端点必须逐字一致）：`DomainProfileCache::reload_from_db`(domain_profile.rs:1013) filter = `doc! { "is_active": true, "current_version": true }`，按 `workspace_id` 分槽。
- **profile_view**(domain_profiles.rs:94-103)：`serde_json::to_value(p)` 整体 snake_case + `id`→hex + 删 `_id`。
- **路由挂载**：domain-profiles 区域 mod.rs:922-947；`use domain_profiles::{...}` import 在 mod.rs:157-161。
- **OperationProfileRequest**(contacts.rs:29-40)：`#[serde(rename_all = "camelCase")]`，故请求体 key 是 **`relationshipType`**（不是 snake_case）；`relationship_type: Option<String>`，`None`→不改现值；写入端点 `PUT /api/contacts/:id/operation-profile`(mod.rs:326)。字典校验后写 `domain_attributes.relationship_type`(contacts.rs:715-727)。
- **m024 字典**：relationship_type kind seed 三值 customer/peer/friend，global scope（m024_seed_relationship_type.rs:40）。
- **前端 DomainProfile 类型**：types/index.ts:540（snake_case，已有 profile_dimensions 等）。
- **accountStore 范式**：stores/accountStore.ts（zustand create，最简只读 store 参照）。
- **App.tsx bootstrap**：App.tsx:136-143（accounts useEffect + ref guard，profileStore init 照搬）。
- **assistOverride 三件套**（relationship 下拉照搬）：
  - 状态声明 userOpsStore.ts:46/254；setter :282；hydrate :301-304（读 `domainAttributes["assist_mode_override"]`）；save :485-505（`api.put` → `refreshContacts` → catch `setError`）。
  - index.tsx 接线 :66 destructure、:90 setter、:111 save、:260/274/279 传 props。
  - legacy.tsx props :153/158/177/191/195 + JSX 落点辅助模式块 :402-419 之后、buttonRow :420 之前。
- **customAgentInstructions textarea**：legacy.tsx:385-393（label :386 / placeholder :392）。

---

## 执行顺序

子项目 1（口吻引导，纯前端文案）→ 子项目 3（前端地基 + 类型录入入口）→ 子项目 2（触达范式）。每个子项目独立可交付、独立可验证、独立提交。

---

# 子项目 1：口吻分化引导（纯前端文案）

## Task 1：customAgentInstructions 文案加口吻/关系引导

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx:386`（label）+ `:392`（placeholder）

**Interfaces:**
- Consumes: 无（纯文案改动，不动任何逻辑/state/props）
- Produces: 无新接口

**说明**：审查确证 customAgentInstructions 已是口吻分化的现成载体（system prompt 最末位、最高优先级、Soul 主动让位、有真模型测证），唯一缺口是运营者不知道能这么用。本任务只改两处展示文案，让运营者知道这个框能写"关系 + 口吻"。不动后端、不加字段、不碰 onChange/save 逻辑。

- [ ] **Step 1: 改 label（legacy.tsx:386）**

把：
```tsx
            <span>运营人员特别指令（最高优先级，可空）</span>
```
改为：
```tsx
            <span>运营人员特别指令（最高优先级，可空 — 也可描述关系与口吻）</span>
```

- [ ] **Step 2: 改 placeholder（legacy.tsx:392）**

把：
```tsx
              placeholder="例：这个客户已签约老客户，不要主动推销，只服务问题。Agent 将在每轮对话最末尾读取这段指令。"
```
改为：
```tsx
              placeholder="例①：这个客户已签约老客户，不要主动推销，只服务问题。例②：这是我大学同学，他公司可能采购我们产品，但别推销，先用轻松口吻维护关系。Agent 将在每轮对话最末尾读取这段指令，可覆盖默认人格口吻。"
```

- [ ] **Step 3: 前端构建验证**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错（纯字符串改动不影响类型）。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(user-ops): 特别指令文案加关系/口吻引导(口吻分化复用现有最高优先级指令通道)"
```

---

# 子项目 3：前端地基 + 类型录入入口

## Task 2：后端 `GET /api/admin/domain-profiles/active` 只读端点

**Files:**
- Modify: `src/routes/domain_profiles.rs`（新增 handler `active_domain_profile`，紧跟 `get_domain_profile` 之后，约 :121 后）
- Modify: `src/routes/mod.rs:157-161`（import 加 `active_domain_profile`）+ `:922`（新增 route，**注意放在 `/:id` 路由之前**避免 `active` 被当成 `:id` 匹配）
- Test: `src/routes/domain_profiles.rs`（同文件 `#[cfg(test)]` 内不便起 Mongo；改为 lib 级纯逻辑无法覆盖 DB 查询——本端点测试走集成测试，见 Step 5 说明）

**Interfaces:**
- Consumes: `profile_view(&DomainProfile) -> Value`(domain_profiles.rs:94)、`AuthenticatedAdmin`(已 import)
- Produces: `GET /api/admin/domain-profiles/active` → `{ "item": <profile_view> | null }`

- [ ] **Step 1: 写 handler（domain_profiles.rs，紧跟 get_domain_profile :121 之后插入）**

```rust
/// 取当前 workspace 运行时生效的 active profile（只读）。
///
/// 查询条件与 [`crate::agent::domain_profile::DomainProfileCache::reload_from_db`]
/// 逐字一致（`is_active=true AND current_version=true` + 同 workspace），确保前端
/// 显示的 profile 与 AI 实际加载的是同一行。无 active 时返 `{item: null}`（合法状态：
/// 运行时此时回落 DEFAULT_PROFILE），不报 404。
pub(super) async fn active_domain_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let profile = state
        .db
        .domain_profiles()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "is_active": true,
                "current_version": true,
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "item": profile.map(|p| profile_view(&p)) })))
}
```

- [ ] **Step 2: import handler（mod.rs:157-161）**

把：
```rust
use domain_profiles::{
    activate_domain_profile, create_domain_profile, delete_domain_profile, get_domain_profile,
    list_domain_profiles, publish_domain_profile, rollback_domain_profile, rollout_domain_profile,
    update_domain_profile,
};
```
改为（加 `active_domain_profile`，保持字母序）：
```rust
use domain_profiles::{
    activate_domain_profile, active_domain_profile, create_domain_profile, delete_domain_profile,
    get_domain_profile, list_domain_profiles, publish_domain_profile, rollback_domain_profile,
    rollout_domain_profile, update_domain_profile,
};
```

- [ ] **Step 3: 挂路由（mod.rs:922，在 `/admin/domain-profiles` route 之后、`/admin/domain-profiles/:id` route 之前插入）**

在 `.route("/admin/domain-profiles", get(list_domain_profiles).post(create_domain_profile))` 之后、`.route("/admin/domain-profiles/:id", ...)` 之前插入：
```rust
        .route(
            "/admin/domain-profiles/active",
            get(active_domain_profile),
        )
```
> 必须在 `/:id` 之前——axum 路由静态段优先于动态段，但顺序明确写在前更稳妥，避免 `active` 被 `:id` 捕获。

- [ ] **Step 4: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --tests`
Expected: 编译通过，0 warning。

- [ ] **Step 5: lib 测试 + 基线不回归**

Run: `cargo test --lib`
Expected: ≥350 passed / 0 failed。
说明：本端点是 DB 查询，纯逻辑无可单测点；正确性靠 Step 4 编译 + 查询条件与 reload_from_db 逐字一致（人工核对 domain_profile.rs:1013）保证。集成测试（testcontainers）留 CI；本地不强制。

- [ ] **Step 6: Commit**

```bash
git add src/routes/domain_profiles.rs src/routes/mod.rs
git commit -m "feat(domain-profiles): 补 GET /admin/domain-profiles/active 只读端点(查询条件与运行时缓存逐字一致)"
```

## Task 3：前端 profileStore + App 启动加载

**Files:**
- Create: `frontend/src/stores/profileStore.ts`
- Modify: `frontend/src/App.tsx:130-143`（bootstrap useEffect 加载 active profile）

**Interfaces:**
- Consumes: `GET /api/admin/domain-profiles/active`(Task 2) → `{item: DomainProfile | null}`；`api`(lib/api)；`DomainProfile`(types/index.ts:540)
- Produces: `useProfileStore` zustand store，暴露 `activeProfile: DomainProfile | null` / `loading` / `error` / `loadActiveProfile()`

- [ ] **Step 1: 写 profileStore（新建 frontend/src/stores/profileStore.ts）**

```ts
import { create } from "zustand";
import { api } from "../lib/api";
import type { DomainProfile } from "../types";

interface ProfileState {
  activeProfile: DomainProfile | null;
  loading: boolean;
  error: string | null;
  loadActiveProfile: () => Promise<void>;
}

export const useProfileStore = create<ProfileState>((set) => ({
  activeProfile: null,
  loading: false,
  error: null,
  loadActiveProfile: async () => {
    set({ loading: true, error: null });
    try {
      const data = await api.get<{ item: DomainProfile | null }>(
        "/api/admin/domain-profiles/active"
      );
      set({ activeProfile: data.item, loading: false });
    } catch (err) {
      // 降级：拿不到 active profile 时前端照常跑，只是没有行业化数据。
      set({
        activeProfile: null,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  },
}));
```

- [ ] **Step 2: App 启动加载（App.tsx:136-143 bootstrap useEffect 内追加）**

把：
```tsx
  const accountsBootstrapRef = useRef(false);
  useEffect(() => {
    if (accountsBootstrapRef.current) return;
    accountsBootstrapRef.current = true;
    void api
      .get<{ items: Account[] }>("/api/accounts")
      .then((data) => useAccountStore.getState().setAccounts(data.items))
      .catch((err) => useUiStore.getState().setError(err instanceof Error ? err.message : String(err)));
  }, []);
```
改为（在同一 effect 末尾追加 loadActiveProfile；profileStore 自带降级兜底，不污染全局错误横幅）：
```tsx
  const accountsBootstrapRef = useRef(false);
  useEffect(() => {
    if (accountsBootstrapRef.current) return;
    accountsBootstrapRef.current = true;
    void api
      .get<{ items: Account[] }>("/api/accounts")
      .then((data) => useAccountStore.getState().setAccounts(data.items))
      .catch((err) => useUiStore.getState().setError(err instanceof Error ? err.message : String(err)));
    void useProfileStore.getState().loadActiveProfile();
  }, []);
```

- [ ] **Step 3: 加 import（App.tsx 顶部 import 区，与其他 store import 并列）**

加一行：
```tsx
import { useProfileStore } from "./stores/profileStore";
```
（核对 App.tsx 现有 store import 风格，放同组。）

- [ ] **Step 4: 前端构建 + 类型检查**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/stores/profileStore.ts frontend/src/App.tsx
git commit -m "feat(frontend): profileStore 启动加载 active profile(降级兜底,数字分身前端地基)"
```

## Task 4：联系人 relationship_type 录入下拉

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`（state 声明 :45-46 区 + 初值 :253-254 区 + setter :281-282 区 + hydrateSelected :297-308 + 新增 saveRelationshipType，仿 saveAssistOverride :485-505）
- Modify: `frontend/src/features/user-ops/index.tsx`（:66 destructure / :90 setter / :111 save / :260+274+279 传 props，仿 assistOverride）
- Modify: `frontend/src/features/user-ops/legacy.tsx`（props 类型 :177+191+195 区 + 解构 :153/158 区 + JSX 落点 :419 之后）

**Interfaces:**
- Consumes: `PUT /api/contacts/:id/operation-profile` body `{relationshipType}`（camelCase，contacts.rs:31）；`contact.domainAttributes["relationship_type"]` 回显；`refreshContacts` / `useUiStore`（userOpsStore 现有）
- Produces: store `relationshipType: string` + `setRelationshipType` + `saveRelationshipType`；legacy props `onRelationshipType` / `onSaveRelationshipType`

- [ ] **Step 1: store 加 state 声明（userOpsStore.ts，在 :46 `assistOverride: string;` 后）**

把：
```ts
  customAgentInstructions: string;
  assistOverride: string; // "default" | "force_on" | "force_off"
```
改为：
```ts
  customAgentInstructions: string;
  assistOverride: string; // "default" | "force_on" | "force_off"
  relationshipType: string; // "" | "customer" | "peer" | "friend"
```

- [ ] **Step 2: store 加 action 声明（同 interface 内，setter/save 声明区，仿 assistOverride 对应行）**

在 interface 里 `setAssistOverride` 声明附近加：
```ts
  setRelationshipType: (value: string) => void;
  saveRelationshipType: () => Promise<void>;
```
（在 `saveAssistOverride` 声明附近同样加 `saveRelationshipType: () => Promise<void>;`，与现有声明风格一致。）

- [ ] **Step 3: store 加初值（userOpsStore.ts:254 `assistOverride: "default",` 后）**

把：
```ts
  customAgentInstructions: "",
  assistOverride: "default",
```
改为：
```ts
  customAgentInstructions: "",
  assistOverride: "default",
  relationshipType: "",
```

- [ ] **Step 4: store 加 setter（userOpsStore.ts:282 `setAssistOverride` 后）**

把：
```ts
  setCustomAgentInstructions: (instructions) => set({ customAgentInstructions: instructions }),
  setAssistOverride: (mode) => set({ assistOverride: mode }),
```
改为：
```ts
  setCustomAgentInstructions: (instructions) => set({ customAgentInstructions: instructions }),
  setAssistOverride: (mode) => set({ assistOverride: mode }),
  setRelationshipType: (value) => set({ relationshipType: value }),
```

- [ ] **Step 5: hydrateSelected 加回显（userOpsStore.ts:301-304 区，assistOverride 读取后）**

把：
```ts
      assistOverride:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "assist_mode_override"
        ] as string) || "default",
      selectedPlaybookId: contact.playbookId || "",
```
改为：
```ts
      assistOverride:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "assist_mode_override"
        ] as string) || "default",
      relationshipType:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "relationship_type"
        ] as string) || "",
      selectedPlaybookId: contact.playbookId || "",
```

- [ ] **Step 6: 加 saveRelationshipType（userOpsStore.ts，saveAssistOverride :505 后）**

在 `saveAssistOverride` 整个 action 之后插入：
```ts
  saveRelationshipType: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { relationshipType } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/operation-profile`, {
        relationshipType: relationshipType || undefined,
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
```
> 注：body key 是 `relationshipType`（camelCase，OperationProfileRequest 是 rename_all=camelCase）；空串传 `undefined` → 后端 `None` → 不改现值（避免误清空）。

- [ ] **Step 7: index.tsx 接线（user-ops/index.tsx，仿 assistOverride 的 :66/:90/:111/:260/:274/:279）**

在对应位置加：
- destructure store（:66 `assistOverride,` 附近）：`relationshipType,`
- destructure setter（:90 `setAssistOverride,` 附近）：`setRelationshipType,`
- destructure save（:111 `saveAssistOverride,` 附近）：`saveRelationshipType,`
- 传 props（:260 `assistOverride={assistOverride}` 附近）：`relationshipType={relationshipType}`
- 传 props（:274 `onAssistOverride={setAssistOverride}` 附近）：`onRelationshipType={setRelationshipType}`
- 传 props（:279 `onSaveAssistOverride={saveAssistOverride}` 附近）：`onSaveRelationshipType={saveRelationshipType}`

- [ ] **Step 8: legacy.tsx props 类型与解构（仿 assistOverride）**

- 解构（:153 `onAssistOverride,` 附近）加：`onRelationshipType,`
- 解构（:158 `onSaveAssistOverride,` 附近）加：`onSaveRelationshipType,`
- 值 prop 解构（:177 `assistOverride,` 附近，组件参数对象里）加：`relationshipType,`
- 类型（:177 `assistOverride: string;` 附近）加：`relationshipType: string;`
- 类型（:191 `onAssistOverride: (mode: string) => void;` 附近）加：`onRelationshipType: (value: string) => void;`
- 类型（:195 `onSaveAssistOverride: () => void;` 附近）加：`onSaveRelationshipType: () => void;`

- [ ] **Step 9: legacy.tsx JSX 下拉（辅助模式块 :419 `</label>` 之后、buttonRow :420 之前插入）**

```tsx
          <label>
            <span>客户类型</span>
            <small>影响 AI 的主动触达策略（如：朋友不主动追单、销售对象继续跟进）。</small>
            <select
              value={relationshipType}
              onChange={(event) => onRelationshipType(event.target.value)}
            >
              <option value="">未分类</option>
              <option value="customer">客户（销售型）</option>
              <option value="peer">同行</option>
              <option value="friend">朋友</option>
            </select>
            <button className="secondary" onClick={onSaveRelationshipType} disabled={busy} type="button">
              <SquarePen size={16} />
              保存客户类型
            </button>
          </label>
```
> 落点紧跟辅助模式 `</label>`(:419) 之后。注意：辅助模式块外层有 `{selected.agentStatus === "managed" && (...)}` 条件——客户类型下拉**放在该条件块之外**（普通联系人也可标类型），即在 :419 的 `)}` 之后、:420 `<div className="buttonRow">` 之前。

- [ ] **Step 10: 前端构建 + 类型检查**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错（props 透传链完整）。

- [ ] **Step 11: Commit**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/index.tsx frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(user-ops): 联系人客户类型(relationship_type)录入下拉(照搬辅助模式三件套,接已有operation-profile端点)"
```

## Task 5：ChannelDef visibleWhen 机制（默认显示）

**Files:**
- Modify: `frontend/src/app/channels.ts:42-52`（ChannelDef 加可选 visibleWhen 字段）
- Modify: `frontend/src/app/Shell.tsx:159`（频道 filter 加 visibleWhen 判断）

**Interfaces:**
- Consumes: `DomainProfile`(types/index.ts:540)；`useProfileStore`(Task 3)
- Produces: `ChannelDef.visibleWhen?: (profile: DomainProfile | null) => boolean`（默认显示语义：未定义→显示）

**说明**：本任务只建机制，不给任何频道接规则（全显示）。用户明确频道是账号级、客户类型是联系人级，频道门控当前价值不大；机制留作后续扩展点。

- [ ] **Step 1: ChannelDef 加字段（channels.ts:42-52）**

把：
```ts
export interface ChannelDef {
  id: Channel;
  group: "运营" | "知识" | "系统";
  label: string;
  caption: string;
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  subtitle: string;
  Component: LazyExoticComponent<ComponentType>;
}
```
改为（末尾加可选字段）：
```ts
export interface ChannelDef {
  id: Channel;
  group: "运营" | "知识" | "系统";
  label: string;
  caption: string;
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  subtitle: string;
  Component: LazyExoticComponent<ComponentType>;
  /** 频道可见性谓词：未定义→默认显示（白名单退出）；定义了→按返回值。
   *  读 active profile 决定该频道是否对当前行业显示。本期无频道使用，留作扩展点。 */
  visibleWhen?: (profile: DomainProfile | null) => boolean;
}
```
（确保 channels.ts 顶部 import 了 `DomainProfile`：若无，加 `import type { DomainProfile } from "../types";`。）

- [ ] **Step 2: Shell.tsx filter 加 visibleWhen（Shell.tsx:159）**

把：
```tsx
              {CHANNELS.filter((c) => c.group === group).map((c) => {
```
改为：
```tsx
              {CHANNELS.filter((c) => c.group === group)
                .filter((c) => (c.visibleWhen ? c.visibleWhen(activeProfile) : true))
                .map((c) => {
```

- [ ] **Step 3: Shell.tsx 读 profileStore（Shell.tsx:133 区，其他 store 选择器附近加）**

在 `const activeChannel = useNavigationStore((s) => s.activeChannel);` 附近加：
```tsx
  const activeProfile = useProfileStore((s) => s.activeProfile);
```
并在 Shell.tsx 顶部 import 区加：
```tsx
import { useProfileStore } from "../stores/profileStore";
```

- [ ] **Step 4: 前端构建验证**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。所有 17 频道仍全显示（无频道定义 visibleWhen）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/app/channels.ts frontend/src/app/Shell.tsx
git commit -m "feat(frontend): ChannelDef visibleWhen 机制(默认显示,profile驱动频道门控扩展点)"
```

---

# 子项目 2：触达分化范式补全

## Task 6：给"通用销售+关系"示例 profile 配 per_relationship_operation_mode

**Files:**
- Modify: `src/agent/domain_profile.rs`（新增 `example_sales_with_relationships_profile(workspace_id)` 构造函数，紧跟 `example_emotional_companion_profile` :932 之后）
- Modify: `src/db/migrations/`（新增 migration seed 该 profile 为 draft/inactive，仿 m020 seed_example_profile :156-181；migration 编号取当前最大 + 1，注册进 migrations 列表）
- Test: `src/agent/domain_profile.rs`（`#[cfg(test)]` 内加构造函数断言，仿 :1291 区）

**Interfaces:**
- Consumes: `default_domain_profile(workspace_id)`(domain_profile.rs:736)、`OperationMode::default()`(models.rs)、`resolve_operation_mode`(planner/mod.rs:938) 三级回落（已实现，本任务只喂数据）
- Produces: `example_sales_with_relationships_profile(workspace_id: &str) -> DomainProfile`（销售框架 + 三套 per_relationship 范式）；一个 seed 该 profile 的 migration（draft/inactive，运营手动 activate 才生效）

**说明**：审查事实 4——`resolve_operation_mode` 三级回落框架已实现，companion profile 已配三套范式，但 companion 是"纯陪伴"框架。销售场景下运营者实际激活的 profile（DEFAULT 或其衍生）`per_relationship_operation_mode=None`，故标了 friend 也整段回落、触达零差异。本任务提供一个"销售框架 + 关系分化"的示例 profile：保留销售域人格/状态机，但给三种关系类型配差异化主动触达范式。DEFAULT profile 保持 None 不动（护栏 H8）。seed 为 draft/inactive，运营审阅后手动 activate（不擅自改变零配置启动行为）。

- [ ] **Step 1: 写构造函数（domain_profile.rs，example_emotional_companion_profile :932 之后插入）**

```rust
/// 数字分身样例（销售框架 + 关系分化）：保留默认销售域人格/状态机/五闸，但按
/// relationship_type 配三套主动触达范式。运营给 contact 标 relationship_type 后，
/// planner 经 resolve_operation_mode 路由到对应范式。
///
/// 与 [`example_emotional_companion_profile`] 的区别：那是"纯陪伴"框架（关漏斗、
/// 开日历关怀）；本 profile 是"销售为主、对非客户关系收敛触达"框架。
///
/// **注**：per_relationship 仅切 OperationMode 各驱动（漏斗/承诺/日历开关），不含
/// 口吻分化——口吻走 customAgentInstructions 自然语言通道（见
/// 2026-06-22-digital-twin-relationship-closure-design 设计文档子项目 1）。
pub fn example_sales_with_relationships_profile(workspace_id: &str) -> DomainProfile {
    let mut profile = default_domain_profile(workspace_id);
    profile.profile_id = "sales_with_relationships".to_string();
    profile.display_name = "销售 + 关系分化".to_string();
    profile.description =
        "销售为主框架；按客户类型分化主动触达：客户追单、同行低频维护、朋友只留情感关怀。"
            .to_string();
    let mut per_relationship = std::collections::BTreeMap::new();
    // 客户（销售型）：沿用销售默认全开 + 日历（怕丢单，主动跟进）。
    let mut customer_mode = crate::models::OperationMode::default();
    customer_mode.calendar.enabled = true;
    per_relationship.insert("customer".to_string(), customer_mode);
    // 同行：关漏斗（不推单），保留承诺与日历（行业节点维护）。
    let mut peer_mode = crate::models::OperationMode::default();
    peer_mode.funnel.enabled = false;
    peer_mode.calendar.enabled = true;
    per_relationship.insert("peer".to_string(), peer_mode);
    // 朋友：关漏斗 + 关承诺催促，只留日历个人关怀（不主动追单/不催进度）。
    let mut friend_mode = crate::models::OperationMode::default();
    friend_mode.funnel.enabled = false;
    friend_mode.commitment.enabled = false;
    friend_mode.calendar.enabled = true;
    per_relationship.insert("friend".to_string(), friend_mode);
    profile.per_relationship_operation_mode = Some(per_relationship);
    profile
}
```

- [ ] **Step 2: 导出构造函数（domain_profile.rs 若有 pub 导出区 / mod.rs:179 re-export）**

确认 `example_sales_with_relationships_profile` 是 `pub fn`（已是）；若 `src/agent/mod.rs:179` re-export 了 `example_emotional_companion_profile`，同样加上本函数：
```rust
    default_domain_profile, example_emotional_companion_profile,
    example_sales_with_relationships_profile,
```

- [ ] **Step 3: 写构造函数单测（domain_profile.rs #[cfg(test)] 内，仿 :1291）**

```rust
    #[test]
    fn sales_with_relationships_routes_three_modes() {
        let p = example_sales_with_relationships_profile("ws-s");
        let per = p
            .per_relationship_operation_mode
            .as_ref()
            .expect("应配三套 per_relationship 范式");
        // 客户：漏斗保持开（销售追单）。
        assert!(per.get("customer").unwrap().funnel.enabled);
        // 同行：漏斗关。
        assert!(!per.get("peer").unwrap().funnel.enabled);
        // 朋友：漏斗关 + 承诺关。
        assert!(!per.get("friend").unwrap().funnel.enabled);
        assert!(!per.get("friend").unwrap().commitment.enabled);
    }

    #[test]
    fn default_profile_keeps_per_relationship_none() {
        // 护栏 H8：DEFAULT 永远 None，不被本任务影响。
        let d = default_domain_profile("ws-d");
        assert!(d.per_relationship_operation_mode.is_none());
    }
```

- [ ] **Step 4: 运行构造函数单测 + 基线**

Run: `cargo test --lib domain_profile`
Expected: 新两个测试 PASS；DEFAULT 逐字节护栏测试仍 PASS。
Run: `cargo test --lib`
Expected: ≥350 passed / 0 failed。

- [ ] **Step 5: 写 seed migration（src/db/migrations/，新文件，编号取当前最大+1）**

先确认当前最大 migration 编号：`ls src/db/migrations/` 找最大 `mNNN_*.rs`。新建 `mNNN_seed_sales_with_relationships.rs`，仿 m020 seed_example_profile（draft/inactive，`$setOnInsert` upsert，不覆盖运营编辑）：

```rust
//! seed「销售 + 关系分化」示例 profile（draft, inactive）。
//!
//! 提供一个 per_relationship_operation_mode 已配三套范式的可激活 profile，让运营给
//! contact 标 relationship_type 后主动触达真分化。draft/inactive——运营审阅后手动
//! activate 才生效，不改零配置启动（DEFAULT 仍 active）。幂等 $setOnInsert。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::agent::domain_profile::example_sales_with_relationships_profile;
use crate::db::Database;
use crate::error::AppResult;

const PROFILE_ID: &str = "sales_with_relationships";

pub(super) async fn run(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    let workspace_id = "default";
    let collection = db.domain_profiles();
    let filter = doc! { "workspace_id": workspace_id, "profile_id": PROFILE_ID };
    let mut profile = example_sales_with_relationships_profile(workspace_id);
    profile.created_at = now;
    profile.updated_at = now;
    // draft/inactive：运营手动 activate 才生效。
    profile.current_version = false;
    profile.is_active = false;
    profile.seeded_by = Some("system".to_string());
    let mut doc_to_set = mongodb::bson::to_document(&profile)?;
    doc_to_set.remove("_id");
    let result = collection
        .update_one(
            filter,
            doc! { "$setOnInsert": doc_to_set },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    tracing::info!(
        migration_id = "seed_sales_with_relationships",
        upserted = result.upserted_id.is_some(),
        "seeded sales+relationships domain profile (draft, inactive)"
    );
    Ok(())
}
```
> 核对 m020 的真实 import 路径与 `Database` / `AppResult` 引用风格，对齐本文件 use 段。`current_version=false` 让它不与 DEFAULT 的 current 冲突；运营 activate 时走 publish→activate 正常流程。

- [ ] **Step 6: 注册 migration（src/db/migrations/mod.rs，仿现有 mNNN 注册）**

在 migrations mod.rs 里 `mod mNNN_...;` 声明 + run 列表（仿 m020/m021/m023/m024 的注册形态）加上新 migration。核对现有注册顺序，追加到末尾。

- [ ] **Step 7: 编译 + 基线**

Run: `RUSTFLAGS="-D warnings" cargo check --tests`
Expected: 编译通过，0 warning。
Run: `cargo test --lib`
Expected: ≥350 passed / 0 failed。

- [ ] **Step 8: Commit**

```bash
git add src/agent/domain_profile.rs src/agent/mod.rs src/db/migrations/
git commit -m "feat(digital-twin): seed 销售+关系分化示例profile(三套per_relationship范式,draft待运营激活;DEFAULT保持None)"
```

---

## 自审记录（writing-plans Self-Review）

**1. Spec 覆盖**：
- 子项目 1（口吻引导）→ Task 1 ✅
- 子项目 3.1（/active 端点）→ Task 2 ✅；3.2（profileStore）→ Task 3 ✅；3.3（relationship_type 下拉）→ Task 4 ✅；3.4（visibleWhen 机制）→ Task 5 ✅
- 子项目 2（触达范式）→ Task 6 ✅
- 非目标（不新建 per_relationship_soul / 不加 voice 字段 / 不接频道规则 / 不动 DEFAULT）→ 各任务约束已写明 ✅

**2. 占位符扫描**：无 TBD/TODO。两处需执行时确认的（Task 6 migration 编号、Task 6 mod.rs 注册形态）已明确指示"先 ls 确认/核对现有形态"，非占位（属正常的"按现有约定对齐"）。

**3. 类型一致性**：
- 后端请求体 key 全用 `relationshipType`（camelCase，OperationProfileRequest rename_all=camelCase）✅
- 前端 store 字段 `relationshipType` / setter `setRelationshipType` / save `saveRelationshipType` / props `onRelationshipType`+`onSaveRelationshipType` 全任务一致 ✅
- `/active` 返回 `{item: DomainProfile | null}`，profileStore 消费同形 ✅
- `visibleWhen?: (profile: DomainProfile | null) => boolean` 在 ChannelDef 定义、Shell 消费一致 ✅

