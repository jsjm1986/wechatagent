# 全量系统测试 P0+P1 修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复全量系统深度测试台账（`docs/superpowers/specs/2026-07-10-full-system-test-findings.md`）中的 P0+P1 findings：账号错配家族（F-005+F-020）、知识库概览卡顿（F-013）、任务日志内部作业泄漏（F-003）。

**Architecture:** 后端 Rust(Axum) + 前端 React19/TS/Zustand。四个修复彼此正交、边界清晰：①前端三处联系人选择器传当前选中账号；②campaign 创建走前端传账号 + 后端加字段（治本，照搬 list_contacts 的"组合过滤即隔离"模式，不做多余的账号归属校验）；③completeness 端点加进程内 TTL 缓存 + 前端概览页局部骨架不阻塞整屏；④list_tasks 加 kind 白名单过滤 + 前端补 3 个 status label。

**Tech Stack:** Rust + Axum + MongoDB(mongodb driver) + DashMap(进程内缓存，已在 AppState.chunk_locks 用) + React19 + Zustand + Vite。

## Global Constraints

- **改代码前 100% 读懂相关代码，绝不猜测；引用必亲验 file:line**（CLAUDE.md 最高红线）。本计划的关键事实已由主控亲验（见"关键事实"节），实现者据此不必重新发现，但落地前仍须 Read 确认 old_string 精确匹配。
- **账号隔离模式**：后端一律"组合过滤即隔离"——filter 同时插 `workspace_id`(来自 `admin.current_workspace`) + `account_id`(query/body 缺省回落 `state.config.default_account_id`)。`list_contacts`(contacts.rs:108-112) 与 `list_tasks`(tasks.rs:45-55) 均此模式，**不显式校验"账号是否属于该 workspace"**（不属于则组合查询自然返回空，无需额外校验——加校验是过度设计，偏离现有模式）。
- **前端当前账号唯一来源**：`useAccountStore().currentAccountId()`(accountStore.ts:24-29)——选中且在 accounts 列表内则用选中，否则回落 `accounts[0]?.accountId ?? ""`。禁止另造账号选择逻辑。
- **无人工接管红线**：新增/改动的字符串（label/注释/status 名）在 `src/agent/`、`src/routes/`、`src/evolution/`、`frontend/src/` 下**禁止**出现 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`（`scripts/check-no-human-takeover.{sh,ps1}` CI lint 扫新增行）。
- **测试基线不回退**：`cargo test --lib` ≥350 passed/0 failed；四个 PBT 文件累计 ≥33/0。新工作加测试不降基线。
- **不过拟合**：只修真 bug，绝不为让页面"看起来对"改 prompt/阈值/guards。
- **status/kind 是自由字符串**：`AgentTask.kind`(models.rs:835) 与 status 都是 `String`。status 有闭集守卫 `ALLOWED_AGENT_TASK_STATUS`(models.rs:868-877)：`pending/running/retry/failed/cancelled/sent/completed/outbox_enqueued`。kind 当前实际取值 6 个：`follow_up`/`deferred_inbound_reply`/`principal_decision_relay`/`memory_consolidation`/`outcome_aggregation`/`initial_profile`。
- 台账：`docs/superpowers/specs/2026-07-10-full-system-test-findings.md`。设计依据见该台账 F-005/F-020/F-013/F-003 条目。

## 关键事实（主控已亲验，实现者据此不必重新发现，但落地前 Read 确认精确匹配）

- **F-005 前端三处**均为 `const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");`，无 accountId、无 useAccountStore import：
  - `frontend/src/features/ask-human-config/DeciderChainEditor.tsx:26`（在 `useEffect([picking])` 内）
  - `frontend/src/features/products-deals/index.tsx:363`（`ContactPicker` 的 `useEffect([])` 内）
- **F-020 后端** `CreateCampaignRequest`(campaigns.rs:112-119) 无 account_id 字段；`create_campaign`(campaigns.rs:195-226) 的 `account_id: state.config.default_account_id.clone()`(:210) 硬编码。前端 `CampaignCreate.tsx:43` POST body 为 `{title, intentText, segmentFilter}` 无 accountId。dispatch/preview/report 读存库的 `campaign.account_id`（自洽，改创建站点即可）。
- **F-013** `get_operation_knowledge_completeness`(catalog.rs:105-117) 与 `refresh_operation_knowledge_completeness`(catalog.rs:119-131) 走同一 `build_operation_knowledge_completeness`(catalog.rs:474-767)，内含 5×count + 2×find + 1 次阻塞 `state.llm.generate_json`(catalog.rs:728-732)，**零缓存**。前端 `CockpitView`(frontend/src/features/knowledge/cockpit/CockpitView.tsx) `if (!completeness) return <loading>`(:67-69) 整屏阻塞。build 参数是 `(state, workspace_id, account_id)`。
- **F-003** `list_tasks`(tasks.rs:40-83) filter 是内联 `doc!{ "workspace_id":..., "account_id":... }`(:52-55)，无 kind 过滤。消费方：`operationsStore.ts:38`（全量平铺）+ `commandStore.ts:55,97`（filter status==pending 计数）。前端 `TASK_STATUS_LABELS`(operations/index.tsx:98-107) 缺 `retry`/`sent`/`outbox_enqueued` 三个 key，`labelOf`(reviewLabels.ts:334-340) 未命中回落裸英文。
- **AppState**(routes/mod.rs:289-331) 已有进程内并发容器范式：`chunk_locks: chunk_locks::ChunkLockMap`(DashMap)、`prompt_pack_version: Arc<AtomicU64>`。completeness 缓存照此加一个 `Arc<DashMap<...>>` 字段。AppState 构造点需全部补齐新字段（含 tests helper）。

---

### Task 1: F-005 联系人选择器传当前账号（前端两处）

**Files:**
- Modify: `frontend/src/features/ask-human-config/DeciderChainEditor.tsx:1-34`
- Modify: `frontend/src/features/products-deals/index.tsx:350-369`

**Interfaces:**
- Consumes: `useAccountStore().currentAccountId()`(accountStore.ts:24-29) → `string`；`api.get<{items:Contact[]}>(url)`(lib/api.ts)。
- Produces: 无（纯前端行为修复，无新导出）。

**背景（一句话）**：这两处联系人选择器写死 `/api/contacts?limit=100` 不带账号，后端回落 default_account_id="default" → 在账号 102 下查无联系人，功能对运营不可用。修复=带上当前选中账号。

- [ ] **Step 1: DeciderChainEditor 引入 useAccountStore**

Read `DeciderChainEditor.tsx:1-4` 确认 import 区。在 `import { api }` 行后加一行 import。

old_string:
```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { Contact, DeciderRef } from "../../types";
```
new_string:
```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import type { Contact, DeciderRef } from "../../types";
```

- [ ] **Step 2: DeciderChainEditor 取当前账号并传参**

Read `DeciderChainEditor.tsx:16-34` 确认 hook 区与 useEffect。在组件顶部 hooks 区（`const [picking, ...]` 前）加取账号；useEffect 内 URL 带 accountId；把 `currentAccountId` 加进依赖数组。

old_string:
```tsx
  const [picking, setPicking] = useState(false);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!picking) return;
    void (async () => {
      try {
        const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");
        setContacts(res.items);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setContacts([]);
      }
    })();
  }, [picking]);
```
new_string:
```tsx
  const [picking, setPicking] = useState(false);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");
  const [error, setError] = useState<string | null>(null);
  const currentAccountId = useAccountStore((s) => s.currentAccountId);

  useEffect(() => {
    if (!picking) return;
    void (async () => {
      try {
        const accountId = currentAccountId();
        const url = accountId
          ? `/api/contacts?limit=100&accountId=${encodeURIComponent(accountId)}`
          : "/api/contacts?limit=100";
        const res = await api.get<{ items: Contact[] }>(url);
        setContacts(res.items);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setContacts([]);
      }
    })();
  }, [picking, currentAccountId]);
```

- [ ] **Step 3: products-deals ContactPicker 引入 useAccountStore**

Read `products-deals/index.tsx:1-20` 确认 import 区（找到 `import { api }` 或 `useAccountStore` 是否已 import——若已 import 则跳过本 step）。若未 import，在现有 import 区加：
```tsx
import { useAccountStore } from "../../stores/accountStore";
```
（实现者：先 grep 本文件 `useAccountStore`，已存在则复用不重复 import。）

- [ ] **Step 4: products-deals ContactPicker 取账号并传参**

Read `products-deals/index.tsx:356-369` 确认 ContactPicker hook 区。

old_string:
```tsx
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");

  useEffect(() => {
    void (async () => {
      try {
        const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, []);
```
new_string:
```tsx
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");
  const currentAccountId = useAccountStore((s) => s.currentAccountId);

  useEffect(() => {
    void (async () => {
      try {
        const accountId = currentAccountId();
        const url = accountId
          ? `/api/contacts?limit=100&accountId=${encodeURIComponent(accountId)}`
          : "/api/contacts?limit=100";
        const res = await api.get<{ items: Contact[] }>(url);
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, [currentAccountId]);
```

- [ ] **Step 5: 前端类型检查 + 构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。（`currentAccountId` 是稳定的 store selector 引用，加进依赖数组不会引发无限重渲染——Zustand selector 返回的函数引用在 store 未变时稳定。）

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/ask-human-config/DeciderChainEditor.tsx frontend/src/features/products-deals/index.tsx
git commit -m "fix(frontend): 联系人选择器传当前选中账号(F-005 决策链/成交登记)"
```

---

### Task 2: F-020 campaign 创建走前端账号 + 后端加字段（治本）

**Files:**
- Modify: `src/routes/campaigns.rs:112-119`（CreateCampaignRequest 加字段）
- Modify: `src/routes/campaigns.rs:195-220`（create_campaign 读 body.account_id）
- Modify: `frontend/src/features/campaign/CampaignCreate.tsx:37-56`（POST body 带 accountId）

**Interfaces:**
- Consumes: `state.config.default_account_id: String`；`admin.current_workspace: String`；前端 `useAccountStore().currentAccountId()`。
- Produces: `CreateCampaignRequest` 新增 `account_id: Option<String>`（serde camelCase `accountId`, `#[serde(default)]`）。

**背景（一句话）**：campaign 创建把 `account_id` 硬编码成 default_account_id，导致在账号 102 下建的活动圈人恒 0（查的是 "default" 账号的联系人）。治本=前端传当前账号 + 后端从 body 读，照搬 list_contacts 的组合过滤模式，不做多余校验。

- [ ] **Step 1: CreateCampaignRequest 加 account_id 字段**

Read `campaigns.rs:112-119` 确认结构体。

old_string:
```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaignRequest {
    pub title: String,
    pub intent_text: String,
    #[serde(default)]
    pub segment_filter: SegmentFilter,
}
```
new_string:
```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaignRequest {
    pub title: String,
    pub intent_text: String,
    #[serde(default)]
    pub segment_filter: SegmentFilter,
    /// 目标账号；缺省回落 default_account_id（与 list_contacts / list_tasks 同模式，
    /// workspace_id + account_id 组合过滤即隔离，不额外校验账号归属）。
    #[serde(default)]
    pub account_id: Option<String>,
}
```

- [ ] **Step 2: create_campaign 从 body 读 account_id**

Read `campaigns.rs:205-213` 确认。在 `let now` 后、构造 Campaign 前解析 account_id；把 `account_id:` 字段改为读解析值。

old_string:
```rust
    let now = DateTime::now();
    assert_campaign_status_valid("draft");
    let campaign = Campaign {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: state.config.default_account_id.clone(),
        title: body.title.trim().to_string(),
```
new_string:
```rust
    let now = DateTime::now();
    assert_campaign_status_valid("draft");
    let account_id = body
        .account_id
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let campaign = Campaign {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        title: body.title.trim().to_string(),
```

- [ ] **Step 3: 后端编译检查**

Run: `cargo check`
Expected: 编译通过，无 warning/error。

- [ ] **Step 4: 前端 CampaignCreate 引入 useAccountStore 并传 accountId**

Read `CampaignCreate.tsx:1-7` 确认 import 区，加 useAccountStore import。

old_string:
```tsx
import { useState } from "react";
import { api } from "../../lib/api";
import { useCampaignStore } from "../../stores/campaignStore";
import { useUiStore } from "../../stores/uiStore";
```
new_string:
```tsx
import { useState } from "react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import { useCampaignStore } from "../../stores/campaignStore";
import { useUiStore } from "../../stores/uiStore";
```

- [ ] **Step 5: CampaignCreate 取账号并放进 POST body**

Read `CampaignCreate.tsx:11-56` 确认 hooks 与 handlePreview。在 hooks 区（`const setError` 后）加取账号；POST body 带 accountId。

old_string:
```tsx
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);
  const setError = useUiStore((s) => s.setError);
```
new_string:
```tsx
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);
  const setError = useUiStore((s) => s.setError);
  const currentAccountId = useAccountStore((s) => s.currentAccountId);
```

Read `CampaignCreate.tsx:42-48` 确认 POST 调用。

old_string:
```tsx
        const created = await api.post<{ id: string }>("/api/campaigns", {
          title: title.trim(), intentText: intentText.trim(), segmentFilter: segmentFilter(),
        });
```
new_string:
```tsx
        const created = await api.post<{ id: string }>("/api/campaigns", {
          title: title.trim(), intentText: intentText.trim(), segmentFilter: segmentFilter(),
          accountId: currentAccountId() || undefined,
        });
```

- [ ] **Step 6: 前端构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。

- [ ] **Step 7: 提交**

```bash
git add src/routes/campaigns.rs frontend/src/features/campaign/CampaignCreate.tsx
git commit -m "fix(campaign): 创建活动走前端选中账号+后端加accountId字段(F-020治本)"
```

---

### Task 3: F-013 completeness 进程内 TTL 缓存 + 前端概览局部骨架

**Files:**
- Modify: `src/routes/mod.rs:289-331`（AppState 加缓存字段）
- Modify: `src/routes/mod.rs`（api_router 或 AppState 构造点补字段——实现者 grep 全部 `AppState {` 构造点）
- Modify: `src/main.rs`（生产 AppState 构造点补字段）
- Modify: `src/routes/knowledge/catalog.rs:105-131`（GET 读缓存/refresh 强制重算）
- Modify: `frontend/src/features/knowledge/cockpit/CockpitView.tsx:56-69`（completeness 区块局部骨架）

**Interfaces:**
- Consumes: `state.completeness_cache`；`build_operation_knowledge_completeness(state, workspace_id, account_id) -> AppResult<Value>`(catalog.rs:474)。
- Produces: `AppState.completeness_cache: CompletenessCache`（类型别名，见 Step 1）。

**背景（一句话）**：completeness 端点每次 GET 都同步跑一次阻塞 LLM 审计（15-21s），前端概览在结果为 null 时整屏卡 loading。修复=后端加进程内 TTL 缓存（GET 命中秒回、miss 才算、refresh 强制重算），前端把 completeness 三块改局部骨架不阻塞整页（其余 integrity/gap 卡已能独立渲染）。

**决策依据**：用户选"缓存+骨架"。缓存用进程内 DashMap（与 AppState.chunk_locks 同范式，重启清空、多副本各自算，对读多写少的概览完全够；不新建 Mongo 集合，避免 accessor/index/migration 的重结构改动）。TTL 取 `completeness_cache_ttl_seconds`，默认 300s。

- [ ] **Step 1: AppState 加缓存字段 + 类型别名**

Read `src/routes/mod.rs:1-40` 找到 use 区与 chunk_locks 模块引用位置。在 AppState 定义前（或文件顶部合适处）加类型别名，并在 struct 内加字段。

先加类型别名（放在 `pub struct AppState` 定义前）：
```rust
/// F-013：operation-knowledge completeness 的进程内 TTL 缓存。
/// key = (workspace_id, account_id)；value = (计算完成的 Unix 毫秒, 结果 JSON)。
/// 进程内 DashMap（与 [`chunk_locks::ChunkLockMap`] 同范式），重启清空、
/// 多副本各自算；对"读多写少"的概览页足够。TTL 见
/// `AppConfig::completeness_cache_ttl_seconds`。
pub type CompletenessCache =
    std::sync::Arc<dashmap::DashMap<(String, String), (i64, serde_json::Value)>>;
```

Read `src/routes/mod.rs:328-331` 确认 struct 末尾字段与右花括号。

old_string:
```rust
    /// Phase G P1-7：RS256 JWT keypair。`jwt_enabled=false` → None；
    /// `true` 时 main.rs 启动期 `JwtKeys::from_config` 解码 PEM 失败直接 panic。
    pub jwt_keys: Option<Arc<crate::auth::jwt::JwtKeys>>,
}
```
new_string:
```rust
    /// Phase G P1-7：RS256 JWT keypair。`jwt_enabled=false` → None；
    /// `true` 时 main.rs 启动期 `JwtKeys::from_config` 解码 PEM 失败直接 panic。
    pub jwt_keys: Option<Arc<crate::auth::jwt::JwtKeys>>,
    /// F-013：operation-knowledge completeness 的进程内 TTL 缓存。
    pub completeness_cache: CompletenessCache,
}
```

（实现者：确认 `dashmap` 已是依赖——chunk_locks 已用 DashMap，grep `Cargo.toml` 的 `dashmap` 与 `chunk_locks.rs` 的 `use dashmap`。若 chunk_locks 用的是自定义 map 类型，则改用与其一致的写法。）

- [ ] **Step 2: 补齐所有 AppState 构造点**

Run: `grep -rn "AppState {" src/ tests/`
对每个构造点加 `completeness_cache: std::sync::Arc::new(dashmap::DashMap::new()),`（放在 `jwt_keys` 附近，字段顺序不影响）。生产构造点在 `src/main.rs`；测试 helper 可能在 `src/lib.rs` 或 `tests/`。

Run: `cargo check --tests`
Expected: 无 E0063（missing field）。所有构造点补齐后编译通过。

- [ ] **Step 3: config 加 TTL 字段**

Read `src/config.rs` 找到 AppConfig 结构体与 `from_env`/默认值区（grep `task_worker_interval_seconds` 作为参照锚点，completeness TTL 加在附近）。加字段：
```rust
    /// F-013：completeness 缓存 TTL（秒）。默认 300。
    pub completeness_cache_ttl_seconds: i64,
```
在 `from_env`（或等价构造）里加：
```rust
            completeness_cache_ttl_seconds: std::env::var("COMPLETENESS_CACHE_TTL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
```
（实现者：先 Read config.rs 确认现有字段的赋值风格——是 `from_env` 手写还是宏；照现有风格加。补齐所有 `AppConfig {` 字面量构造点，含 tests。`cargo check --tests` 验 E0063。）

- [ ] **Step 4: GET 端点读缓存，miss 才算并写回**

Read `catalog.rs:105-131` 确认两个 handler 完整体。把 GET handler 改为先查缓存（未过期直接返回），miss 时算一次并写回缓存；refresh(POST) 保持每次强制重算但也刷新缓存。

Read `catalog.rs:105-117` 确认 GET handler 现状（它调 `build_operation_knowledge_completeness` 并解析 account_id 的确切代码），据此改写。GET 改为：
```rust
pub(super) async fn get_operation_knowledge_completeness(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let workspace_id = admin.current_workspace.clone();
    let key = (workspace_id.clone(), account_id.clone());
    let now_ms = crate::models::DateTime::now().timestamp_millis();
    let ttl_ms = state.config.completeness_cache_ttl_seconds * 1000;
    if let Some(entry) = state.completeness_cache.get(&key) {
        let (computed_at, ref value) = *entry;
        if now_ms - computed_at < ttl_ms {
            return Ok(Json(value.clone()));
        }
    }
    let value =
        build_operation_knowledge_completeness(&state, &workspace_id, &account_id).await?;
    state
        .completeness_cache
        .insert(key, (now_ms, value.clone()));
    Ok(Json(value))
}
```
（实现者：`AccountScopedQuery`(routes/shared.rs:34-38) 与 `list_tasks` 同用；确认 GET handler 现有签名与参数解析，若现状不同则以现状为准适配。`entry` 解构注意 DashMap Ref 的借用——若 `*entry` 解构报借用错误，改为 `let computed_at = entry.0; let value = entry.1.clone(); drop(entry);` 再判 TTL。`DateTime` 时间戳方法名亲验 models.rs。）

- [ ] **Step 5: refresh(POST) 强制重算并刷新缓存**

Read `catalog.rs:119-131` 确认 refresh handler。改为算完写回缓存（强制重算，忽略 TTL）：
```rust
pub(super) async fn refresh_operation_knowledge_completeness(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<AccountScopedQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let workspace_id = admin.current_workspace.clone();
    let value =
        build_operation_knowledge_completeness(&state, &workspace_id, &account_id).await?;
    let now_ms = crate::models::DateTime::now().timestamp_millis();
    state
        .completeness_cache
        .insert((workspace_id, account_id), (now_ms, value.clone()));
    Ok(Json(value))
}
```

- [ ] **Step 6: 后端编译 + 相关单测**

Run: `cargo check --tests && cargo test --lib`
Expected: 编译通过；`cargo test --lib` ≥350 passed/0 failed（不回退基线）。

- [ ] **Step 7: 前端 CockpitView completeness 区块局部骨架**

Read `frontend/src/features/knowledge/cockpit/CockpitView.tsx:20-130` 完整确认 load()、loadFailed、`if (!completeness) return`、以及 completeness 驱动的三块（AnsweringModeGauge :74-79 / CoverageVerdict :84 / 缺口明细 :115-127）与独立块（integrity :92-101 / gapPendingCount :102-107）。

改法：删掉 `if (!completeness) return <loading>`(:67-69) 这个整屏阻塞，改为——页面正常渲染，仅 completeness 依赖的三块在 `completeness===null` 时各自显骨架/占位。保留 `if (loadFailed)`(:56-65) 的整屏错误态（那是 comp 请求真失败，非 loading）。

具体：
1. 删除 `if (!completeness) return <div ...>正在加载知识库状态…</div>`（:67-69 整段）。
2. 对 AnsweringModeGauge / CoverageVerdict / 缺口明细三处，包一层 `completeness ? <真内容> : <骨架占位>`。骨架用一个轻量占位（如 `<div className={styles.skeleton}>概览计算中…</div>` 或复用现有 loading 文案组件——实现者按 CockpitView 现有样式风格选，Read 该文件的 CSS import 确认 `styles` 里有无 skeleton 类，无则用纯文本占位 `正在计算知识完整度…`）。

（实现者：先 Read 整个 CockpitView.tsx 100% 理解 completeness 被哪些子组件消费、各子组件是否已容忍 null——`AnsweringModeGauge`/`CoverageVerdict` 若假设非空需给它们传前才判 null。绝不猜哪些块依赖 completeness，Read 确认每一处 `completeness.` 或 `completeness={...}` 引用。）

- [ ] **Step 8: 前端构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。

- [ ] **Step 9: 提交**

```bash
git add src/routes/mod.rs src/main.rs src/config.rs src/routes/knowledge/catalog.rs frontend/src/features/knowledge/cockpit/CockpitView.tsx
git commit -m "perf(knowledge): completeness加进程内TTL缓存+概览局部骨架不阻塞整屏(F-013)"
```

**验收判据**：GET completeness 第二次调用命中缓存秒回（TTL 内）；refresh(POST) 强制重算；前端概览页不再整屏卡 loading，completeness 三块单独显骨架；`cargo test --lib` 不回退。

---

### Task 4: F-003 list_tasks kind 白名单过滤 + status label 补齐

**Files:**
- Modify: `src/routes/tasks.rs:48-61`（list_tasks filter 加 kind $in）
- Modify: `frontend/src/features/operations/index.tsx:98-107`（TASK_STATUS_LABELS 补 3 个 key）

**Interfaces:**
- Consumes: 无新依赖。
- Produces: 无新导出（行为修复）。

**背景（一句话）**：任务日志页把 `outcome_aggregation`（统计作业，contact_wxid 是占位串 `_outcome_aggregation`）等内部后台作业也列给运营看了；且 `sent`/`retry`/`outbox_enqueued` 三个 status 无中文 label 显示成裸英文。

**决策依据**：用户选"客户触达类可见 + 后端过滤"。白名单 = `follow_up`/`deferred_inbound_reply`/`principal_decision_relay`（均有真实客户语义、属客户触达链）；隐藏 = `outcome_aggregation`/`memory_consolidation`/`initial_profile`（纯内部作业）。后端过滤=运营页与指挥中心 pendingTasks 计数一起变干净（指挥中心也不该把内部作业算"待执行"）。

- [ ] **Step 1: list_tasks filter 加 kind 白名单**

Read `tasks.rs:48-61` 确认 filter 内联 doc。

old_string:
```rust
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id
            },
            FindOptions::builder()
                .sort(doc! { "run_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
```
new_string:
```rust
    // F-003：只展示客户触达类任务（运营视角）；隐藏纯内部后台作业
    // （outcome_aggregation 统计 / memory_consolidation 记忆整理 / initial_profile 画像生成）。
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "kind": { "$in": ["follow_up", "deferred_inbound_reply", "principal_decision_relay"] }
            },
            FindOptions::builder()
                .sort(doc! { "run_at": -1 })
                .limit(100)
                .build(),
        )
        .await?;
```

- [ ] **Step 2: 后端编译 + 单测**

Run: `cargo check && cargo test --lib`
Expected: 编译通过；`cargo test --lib` ≥350 passed/0 failed。

- [ ] **Step 3: 前端 TASK_STATUS_LABELS 补齐缺失 status**

Read `operations/index.tsx:97-107` 确认字典。补 `retry`/`sent`/`outbox_enqueued` 三个后端闭集里有、字典里缺的 key。

old_string:
```tsx
// 跟进任务状态(agent_tasks.status;未知值回落原值)。
const TASK_STATUS_LABELS: Record<string, string> = {
  pending: "待执行",
  scheduled: "已排程",
  running: "执行中",
  done: "已完成",
  completed: "已完成",
  failed: "已失败",
  cancelled: "已取消",
  canceled: "已取消",
};
```
new_string:
```tsx
// 跟进任务状态(agent_tasks.status 闭集见 models.rs:868-877;未知值回落原值)。
const TASK_STATUS_LABELS: Record<string, string> = {
  pending: "待执行",
  scheduled: "已排程",
  running: "执行中",
  retry: "待重试",
  outbox_enqueued: "已入发件箱",
  sent: "已发送",
  done: "已完成",
  completed: "已完成",
  failed: "已失败",
  cancelled: "已取消",
  canceled: "已取消",
};
```

- [ ] **Step 4: 前端构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS 报错。

- [ ] **Step 5: 提交**

```bash
git add src/routes/tasks.rs frontend/src/features/operations/index.tsx
git commit -m "fix(tasks): 任务日志只列客户触达类任务+补齐status中文label(F-003)"
```

**验收判据**：`GET /api/tasks` 只返回 follow_up/deferred_inbound_reply/principal_decision_relay；outcome_aggregation 等内部作业不再出现；前端任务状态 sent/retry/outbox_enqueued 显中文。

---

## Self-Review

**Spec coverage（对照台账 P0+P1 findings）：**
- F-005 账号错配（前端两处联系人选择器）→ Task 1 ✓
- F-020 campaign 账号错配（前端传参 + 后端加字段治本）→ Task 2 ✓
- F-013 completeness 卡顿（缓存 + 骨架）→ Task 3 ✓
- F-003 任务泄漏（kind 白名单 + status label）→ Task 4 ✓

**Placeholder scan：** 每个改代码步骤都给了完整 old_string/new_string 或完整函数体。少数标注"实现者先 Read 确认现状"处（catalog.rs GET handler 现有参数解析、config.rs 赋值风格、CockpitView 子组件 null 容忍、products-deals 是否已 import useAccountStore、dashmap 依赖确认）是**亲验红线的要求**（落地前必须 Read 确认精确匹配），非计划占位——每处都给了锚点 grep 与 fallback 写法。

**Type consistency：**
- `currentAccountId` 三处前端一致用 `useAccountStore((s) => s.currentAccountId)` 取函数再调用 `currentAccountId()`（accountStore.ts:11 签名 `currentAccountId: () => string`）。
- `CompletenessCache` 类型别名（mod.rs 定义）→ AppState 字段 → 构造点 → catalog.rs 使用，全程 `Arc<DashMap<(String,String),(i64,Value)>>` 一致。
- `AccountScopedQuery` 复用（tasks.rs 与 catalog.rs GET 同用，routes/shared.rs:34-38）。
- account_id 缺省回落 `state.config.default_account_id.clone()` 三处后端（contacts/tasks/campaign）一致模式。

**执行者留意：**
- Task 间无严格依赖，但建议按 1→2→3→4 顺序（1/2 前端为主快、3 结构改动最大放中间、4 收尾）。
- 每个 Task 结束独立可测、独立提交，是一个 reviewer gate。
- Task 3 是本计划结构改动最大项（AppState 加字段牵动所有构造点 + config 加字段牵动所有 AppConfig 字面量）——implementer 必须 `cargo check --tests` 复刻 CI 的 E0063 检查，不能只 `cargo test --lib`（后者不编译集成测试会漏构造点）。
- 前端无独立单测覆盖这些改动（纯行为/label），验收靠 `npm run build` 类型门 + 终审阶段的实机走查。
