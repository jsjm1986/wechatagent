# H3 跨租户 IDOR 修复设计

> 日期：2026-06-30
> 分支：`fix/h3-cross-tenant-idor`（off origin/main）
> 来源：终极审判审计 H3 项

## 1. 漏洞描述

`llm_providers.rs`、`domain_schemas.rs`、`domain_profiles.rs` 三个路由文件里共 14 个 handler 接受调用方 body/params 自带的 `workspaceId`，缺省才回落 `admin.current_workspace`，解析出的 `workspace_id` 直接进 Mongo 查询/写入，**没有任何 `user.workspaces` 二次校验**。

这是认证后的**水平越权（horizontal privilege escalation）**：一个已登录的管理员，只要在请求体/查询串里塞入别的租户的 `workspaceId`，就能读写、甚至操作不属于自己的工作区资源。

全仓**唯一**做 ACL 校验的点是 `switch_workspace`（`src/routes/auth.rs:130`），它检查 `user.workspaces.contains(target)`。但这 14 条 handler 链路完全不经过它。

### 受影响 handler（14 个站点，已逐行读码确认）

| 文件 | 行号 | handler | 危害 |
| --- | --- | --- | --- |
| `llm_providers.rs` | 105 | `list_providers` | 读他租户 provider 列表（含配置元数据） |
| `llm_providers.rs` | 155 | `create_provider` | 写他租户 provider 配置 |
| `llm_providers.rs` | 214 | `update_provider` | 改/覆盖他租户 provider 配置 |
| `llm_providers.rs` | 287 | `delete_provider` | 删他租户 provider |
| `llm_providers.rs` | 322 | **`activate_provider`** | **进程级 LlmRegistry 热切换，影响全局运行时** |
| `llm_providers.rs` | 389 | `set_vision_active` | 切他租户的 vision-active provider |
| `llm_providers.rs` | 461 | **`test_provider`** | **用目标租户真实 api_key 发起出站测试调用** |
| `domain_schemas.rs` | 175 | `list_domain_schemas` | 读他租户领域 schema |
| `domain_schemas.rs` | 205 | `create_domain_schema` | 写他租户 schema |
| `domain_schemas.rs` | 247 | `update_domain_schema` | 改他租户 schema |
| `domain_schemas.rs` | 303 | `delete_domain_schema` | 删他租户 schema |
| `domain_schemas.rs` | 340 | `activate_domain_schema` | 激活他租户 schema 版本 |
| `domain_profiles.rs` | 71 | `list_domain_profiles` | 读他租户画像配置 |
| `domain_profiles.rs` | 174 | `create_domain_profile` | 写他租户画像配置 |

> **范围边界（已确认）：** `domain_profiles.rs` 的其余 mutator（`update`/`delete`/`publish`/`rollout`/`rollback`/`activate`，行 215/273/307/396/436/496）以及 `get_domain_profile`/`active_domain_profile`，都用 `doc! { "_id": object_id, "workspace_id": &admin.current_workspace }` **直接锚定 `admin.current_workspace`、不接受 override**——无越权面，**不在本次修复范围**。`llm_providers.rs` 与 `domain_schemas.rs` 的全部 handler 都走 override 模式，故各自全员在列（7 + 5）。

### 漏洞模式（统一）

```rust
// 当前（脆弱）：
let workspace_id = params.workspace_id      // 或 body.workspace_id
    .unwrap_or_else(|| admin.current_workspace.clone());
// → 直接进 Mongo 查询/写入（filter 键名各文件不一：profiles 用 snake_case
//   `workspace_id`，providers/schemas 用 camelCase `workspaceId`），全程无校验
```

### 为什么 ACL 不在 handler 手边

- `AuthenticatedAdmin`（`src/auth/mod.rs:58`，middleware 注入）**只有** `user_id` / `username` / `current_workspace`，**不带** `workspaces` 允许列表。
- `JwtClaims`（`src/auth/jwt.rs`）**刻意**只放 `user_id` / `username` / `current_workspace`，设计注释（jwt.rs:10-12）明确「不复制 ACL；workspace 切换走重新签发」。
- 允许列表 `workspaces: Vec<String>` 只在 `AdminUser`（DB 记录）上，只能通过 `get_admin_user(user_id)` 从库里取。

因此修复**必须**在 handler 里加载 `AdminUser` 才能拿到 ACL —— 这是核心设计约束。

## 2. 方案选型

### 方案 A（选定）：保留 override 能力，但每请求二次校验

保留 body/params 自带 `workspaceId` 的覆盖能力，但解析出最终 workspace 后，**每个请求都**对照 `user.workspaces` 校验一次。不在 `AuthenticatedAdmin` / JWT claims 里塞 ACL（与既有设计一致），而是在校验点用 `get_admin_user` 加载 DB 记录拿 `workspaces`。

**为什么选 A：**
- 与全仓既有 canonical ACL 模式（`switch_workspace`）同源、语义一致。
- 不改 `AuthenticatedAdmin` / JWT claims 形状，不碰中间件、不碰签发链，blast radius 最小。
- `get_admin_user` 是一次 `_id` 主键查（轻量），handler 本就要打 Mongo，多一次可接受。

**否决方案 B（ACL 进 claims/AuthenticatedAdmin）：** 需改 JWT 签发 + 中间件 + session，且与 jwt.rs:10-12 的刻意设计（claims 不放 ACL、切换走重签）直接冲突。改动面大、违背既有约定，否决。

**否决方案 C（仅当显式传 override 时才校验）：** 留下「忘记校验」的站点风险——每个 handler 要自己判断「是否传了 override」，分支多、易漏。且回落 `current_workspace` 本身也应是合法的（见 §6），统一校验更安全。

### 校验时机（选定）：每请求都校验解析出的 workspace

不是「只在显式传 override 时校验」，而是**对每个请求解析出的最终 workspace 都校验**——单一代码路径，没有任何站点能忘记。即使回落到 `current_workspace`，也走同一道校验（`current_workspace` 在正常部署里必然 ∈ ACL，见 §6 边界分析）。

## 3. 核心单元一：纯函数 `is_workspace_authorized`

落点：`src/auth/mod.rs`（与 `AdminUser` 同文件，ACL 语义的自然归属）。

```rust
/// 判定 resolved workspace 是否在 admin 的允许列表内。
/// 空列表 = 单租户回落语义：只允许默认 workspace（与 switch_workspace 同源）。
pub(crate) fn is_workspace_authorized(
    resolved: &str,
    user_workspaces: &[String],
    default_workspace_id: &str,
) -> bool {
    if user_workspaces.is_empty() {
        resolved == default_workspace_id
    } else {
        user_workspaces.iter().any(|w| w == resolved)
    }
}
```

这是从 `switch_workspace`（auth.rs:144-148）抽出来的同一套判定逻辑，提成纯函数后两处共用（见 §6 DRY 重构）。纯函数无 IO，直接 lib 单测覆盖三条路径。

## 4. 核心单元二：DB 包装 `resolve_authorized_workspace`

落点：`src/routes/shared.rs`（已是 fail-closed workspace 辅助函数之家：`validate_account:126`、`find_contact_by_id:155`）。shared.rs 顶部已 import `AppError` / `AppResult` / `AppState`；实现时需补 `use crate::auth::session::get_admin_user;` 与 `use crate::auth::is_workspace_authorized;`（或全路径引用）。

```rust
/// 解析请求的目标 workspace 并校验 ∈ admin ACL。
/// 解析顺序：override（trim 非空）优先，否则回落 admin.current_workspace。
/// 校验失败 → BadRequest("workspace_not_in_user_acl")。
pub(super) async fn resolve_authorized_workspace(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    override_ws: Option<String>,
) -> AppResult<String> {
    let resolved = override_ws
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| admin.current_workspace.clone());

    // get_admin_user 返回 Result<Option<AdminUser>, AuthError>（不是 AppResult）。
    // AuthError 无 From<AppError>，故不能裸 `?`；这里只会发生 AuthError::Mongo
    // （函数体仅一次 find_one），映射成 AppError::Db 与既有错误语义一致。
    let user = get_admin_user(&state.db, &admin.user_id)
        .await
        .map_err(|e| match e {
            crate::auth::session::AuthError::Mongo(err) => AppError::Db(err),
            other => AppError::External(format!("admin lookup: {other}")),
        })?
        .ok_or_else(|| AppError::Unauthorized("admin_user_not_found".into()))?;

    if !is_workspace_authorized(
        &resolved,
        &user.workspaces,
        &state.config.default_workspace_id,
    ) {
        return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
    }
    Ok(resolved)
}
```

> **错误映射注意（实现阶段核对）：** `get_admin_user` 的错误类型是 `auth::session::AuthError`，**不是** `AppError`，所以**不能**裸 `?`（`switch_workspace` 因此用了私有 `map_auth_error`）。`map_auth_error` 是 `routes/auth.rs` 的私有 fn，shared.rs 取不到。两条路可选，实现时择一：
> - **(a) 内联 map_err**（上方代码所示）：`get_admin_user` 体内只有一次 `find_one`，唯一可能的 `AuthError` 变体是 `Mongo(_)`，映射到 `AppError::Db` 即可；
> - **(b) 把 `map_auth_error` 提为 `pub(crate)`** 并移到可共享位置（如 `auth/mod.rs`），shared.rs 与 auth.rs 共用——更 DRY，但改动面略大。
> 推荐 (a)，blast radius 更小；最终以实现阶段读 `AuthError` 定义确认变体集合为准。注意 `AuthError` 当前是否 `pub` 也需核对（若非 pub 需配合 (b)）。

**错误模型复用既有码（AppError 无 Forbidden 变体）：**
- 越权 → `BadRequest("workspace_not_in_user_acl")`（HTTP 400），与 `switch_workspace` 拒绝时同字符串。
- admin 记录查不到 → `Unauthorized("admin_user_not_found")`（HTTP 401），与 `switch_workspace` 同。

## 5. 14 个调用站点改写

每个站点的统一改写（保持 handler 其余逻辑不动）：

```rust
// 改前：
let workspace_id = X.unwrap_or_else(|| admin.current_workspace.clone());

// 改后：
let workspace_id = resolve_authorized_workspace(&state, &admin, X).await?;
```

其中 `X` 是各 handler 原本的 `params.workspace_id` 或 `body.workspace_id`（`Option<String>`）。

**逐站点核对要求（实现阶段）：** 每个 handler 改写前，先读该 handler 完整签名确认：
1. `X` 的来源（params 还是 body 字段）与字段名；
2. handler 是否已持有 `state: AppState` 与 `admin: AuthenticatedAdmin`（14 个都应有，但逐一确认）；
3. 改写后 `workspace_id` 的所有下游用法不变（只是值的来源从「无校验回落」变成「校验后解析」）。

14 个站点：
- `llm_providers.rs`: 105, 155, 214, 287, 322, 389, 461
- `domain_schemas.rs`: 175, 205, 247, 303, 340
- `domain_profiles.rs`: 71, 174

> 行号锚定 origin/main（commit 5e1f63b）。实现时若已 rebase / 行号漂移，以 grep `unwrap_or_else(|| admin.current_workspace.clone())` 的命中为准——这是定位站点的稳定锚。

## 6. switch_workspace DRY 重构

`switch_workspace`（auth.rs:144-148）当前内联了同一套判定：

```rust
let allowed = if user.workspaces.is_empty() {
    target == state.config.default_workspace_id
} else {
    user.workspaces.iter().any(|w| w == target)
};
if !allowed { return Err(AppError::BadRequest("workspace_not_in_user_acl".into())); }
```

改为调用新纯函数（消除重复，保证两处判定永远一致）：

```rust
if !crate::auth::is_workspace_authorized(target, &user.workspaces, &state.config.default_workspace_id) {
    return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
}
```

`switch_workspace` 已自行加载 `AdminUser`（拿 `user`），所以它**不**改用 `resolve_authorized_workspace`（那会重复查库且语义是「解析+校验 override」，而 switch 的入参 `target` 是必填、非 override 回落）——只复用最底层的纯函数。

## 7. 测试策略

### 7.1 lib 单元测试（纯函数，本地可跑）

`is_workspace_authorized` 三条路径：
- 空 ACL ⟹ 只有 `resolved == default_workspace_id` 为 true，其它 false；
- 非空 ACL ⟹ `contains(resolved)` 为 true，不含则 false；
- 拒绝路径：非空 ACL 且 resolved 不在列表 ⟹ false。

这些计入 `cargo test --lib` 基线（新增测试，只增不减）。

### 7.2 集成测试（CI/Docker，testcontainers MongoDB）

参照 #153 IDOR 隔离测试范式（`get_admin_user` 造两个租户、两个 admin，A 持 workspace-A 的 ACL，尝试用 workspace-B 的 id 访问）。覆盖代表性高危站点：
- provider：list / create / **test** / **activate**（后两者是最高危）；
- schema：list / create；
- profile：list。

断言：
- 合法（resolved ∈ ACL）→ 2xx，数据落在正确 workspace；
- 越权（override = 他租户 id，∉ ACL）→ 400 `workspace_not_in_user_acl`，且**目标租户数据无任何读写副作用**（尤其 activate 不得触发 LlmRegistry 热切换、test 不得用他租户 api_key 出站）。

集成测试 `#[ignore]`，本地不跑（磁盘纪律），留 CI Docker 跑。

### 7.3 基线门

- `cargo test --lib` ≥ 350 / 0（本地验证）。
- 4 个 PBT 文件累计 ≥ 33 / 0（不受本改动影响，确认不回归）。
- `scripts/check-no-human-takeover.*` 禁词 lint：新增 src 行不得含禁词（本改动全是 workspace/ACL 词汇，天然不触雷）。

## 8. 已知边界与权衡

**每请求校验也会校验回落的 `current_workspace`。** 这是有意的（单一路径、零遗漏）。影响分析：
- 正常部署：admin 的 `current_workspace` 是登录时由 `default_workspace_id` 或一次合法 `switch_workspace` 设定的，必然 ∈ ACL（或空 ACL 时 == default），校验恒过，**无行为变化**。
- 唯一理论回归：某 admin 被错误配置成 `default_workspace ∉ workspaces`（非空 ACL 且不含其 current）。但这种 admin 本来就**切不进**那个 workspace（switch 会拒），属于配置错误而非本改动引入——fail-closed 拒绝是**正确**行为，不是回归。

**性能：** 每个受影响请求多一次 `_id` 主键查（`get_admin_user`）。这些都是低频管理后台 handler，可接受。

**不在本次范围：** 其它路由文件若有同模式 override（本设计只锚定审计点名的 3 文件 14 站点）。实现阶段若 grep 发现 `unwrap_or_else(|| admin.current_workspace.clone())` 在 3 文件外还有命中，记录但不在本 PR 扩范围——单独评估。

