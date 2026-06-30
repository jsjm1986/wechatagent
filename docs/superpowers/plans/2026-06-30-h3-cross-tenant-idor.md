# H3 跨租户 IDOR 修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 堵住 `llm_providers.rs` / `domain_schemas.rs` / `domain_profiles.rs` 14 个 handler 的认证后水平越权——调用方自带的 `workspaceId` 解析后须二次校验 ∈ admin ACL，否则拒绝。

**Architecture:** 加一个纯函数 `is_workspace_authorized`（auth/mod.rs，无 IO，判定 resolved workspace 是否在 ACL 内）+ 一个 DB 包装 `resolve_authorized_workspace`（routes/shared.rs，加载 AdminUser 拿 ACL 后调纯函数，失败返 `BadRequest("workspace_not_in_user_acl")`）。14 个站点把 `X.unwrap_or_else(|| admin.current_workspace.clone())` 统一替换成 `resolve_authorized_workspace(&state, &admin, X).await?`。`switch_workspace` 内联判定改调同一纯函数（DRY）。

**Tech Stack:** Rust 2021 / Axum / MongoDB（mongodb crate）/ testcontainers（集成测试）。

## Global Constraints

- 设计依据：`docs/superpowers/specs/2026-06-30-h3-cross-tenant-idor-fix-design.md`（已审核通过）。
- 分支：`fix/h3-cross-tenant-idor`（已 off origin/main 建好）。
- 过拟合红线：绝不为过测试改业务逻辑/prompt/guards/阈值；只修真 bug，改根因不迎合断言。
- `cargo test --lib` 基线 ≥ 350 passed / 0 failed；新增测试只增不减。
- cargo 命令前先 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`；磁盘紧时 `CARGO_INCREMENTAL=0`。本地只跑 `cargo test --lib`，**绝不**跑全量 `cargo test`（100+ 集成 binary 撑爆磁盘）。集成测试 `#[ignore]` 留 CI Docker 跑。
- 禁词 lint（`scripts/check-no-human-takeover.*`）：新增 src 行不得含 `人工/接管/takeover/hand-off` 等；本改动全是 workspace/ACL 词汇，天然不触雷。
- commit 仅在授权时；`git add` 具名文件，**绝不** `-A`/`.`；commit message 结尾加 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- 错误码复用既有：越权 → `AppError::BadRequest("workspace_not_in_user_acl")`(400)；admin 查不到 → `AppError::Unauthorized("admin_user_not_found")`(401)。AppError 无 Forbidden 变体。
- 中文回复用户；代码/标识符/commit message 遵循既有约定。

## 关键事实（已逐行读码确认，实现者照用，勿臆测）

- `AuthenticatedAdmin`（`src/auth/mod.rs:58`）只有 `user_id` / `username` / `current_workspace: String`，**不带** ACL。
- `AdminUser`（`src/auth/mod.rs:29`）DB 记录有 `workspaces: Vec<String>` + `default_workspace: Option<String>`。
- `get_admin_user(db, user_id)`（`src/auth/session.rs:174`）签名 `-> Result<Option<AdminUser>, AuthError>`（**不是** `AppResult`）。`AuthError` 是 `pub`（session.rs:21），变体含 `Mongo(#[from] mongodb::error::Error)`。函数体只一次 `find_one`，唯一可能错误是 `AuthError::Mongo(_)`。
- `AppError`（`src/error.rs`）：`Db(#[from] mongodb::error::Error)`(:23)、`External(String)`(:31)、`BadRequest(String)`(:11)、`Unauthorized(String)`(:21) 都存在。
- `switch_workspace`（`src/routes/auth.rs:130-163`）：用 `.map_err(map_auth_error)?` 处理 `get_admin_user`，内联 ACL 判定在 :144-148。`map_auth_error` 是 auth.rs 私有 fn。
- `mod shared` 在 `routes/mod.rs:88` 是**私有** mod（非 pub）；`resolve_authorized_workspace` 落这里，对**外部 test crate 不可见**，故只能由同 crate 的 handler 调用。
- 14 个站点的 workspaceId 来源（已确认）：
  - **Query<ListQuery>**（`params.workspace_id`）：llm_providers `list_providers`(105) / `delete_provider`(287) / `activate_provider`(322)；domain_schemas `list_domain_schemas`(175) / `delete`(303) / `activate`(340)；domain_profiles `list`(71)。
  - **Json body**（`body.workspace_id.clone()`）：llm_providers `create_provider`(155, UpsertRequest) / `update_provider`(214, UpsertRequest) / `set_vision_active`(389, VisionActivateRequest) / `test_provider`(461, TestRequest)；domain_schemas `create`(205) / `update`(247)（UpsertRequest）；domain_profiles `create`(174, UpsertRequest)。
  - 行号锚定 origin/main commit `5e1f63b`；漂移时以 grep `unwrap_or_else(|| admin.current_workspace.clone())` 命中为准。
- 集成测试可达性（已确认）：`llm_providers` 模块 + 其 handler + `ListQuery`(:94) 都是 `pub`，test crate 可直调（范式见 `tests/llm_provider_activate_integration.rs`）。`TestRequest`/`UpsertRequest`/`VisionActivateRequest` 及 domain_schemas/profiles 的 `ListQuery`/`UpsertRequest` 是 `pub(super)`，test crate **不可命名** → 集成测试只挑 `activate_provider` + `list_providers`（均收 `pub ListQuery`）作端到端越权验证。
- 集成测试 seed admin 范式：`bootstrap_admin_if_needed(db, Some(user), Some(pw), Some(&ws))` + `authenticate(db, user, pw)` 拿真实 `user_id`（见 `tests/auth_middleware_integration.rs:139` `switch_workspace_rejects_outside_acl`）。`test_config` 的 `default_workspace_id="default"`。

---

### Task 1: 纯函数 `is_workspace_authorized` + lib 单元测试

判定 resolved workspace 是否在 admin ACL 内。空 ACL = 单租户回落语义（只允许 default workspace），与 `switch_workspace` 同源。纯函数无 IO，直接 lib 单测。

**Files:**
- Modify: `src/auth/mod.rs`（68 行，当前只有 struct 定义；在文件末尾追加纯函数 + `#[cfg(test)]` 测试模块）

**Interfaces:**
- Produces: `pub fn is_workspace_authorized(resolved: &str, user_workspaces: &[String], default_workspace_id: &str) -> bool`
  - **可见性必须是 `pub`**（不是 spec 初稿写的 `pub(crate)`）：Task 4 的集成测试在外部 test crate，需要能间接验证它走的判定；且 `pub` 不损害封装（无副作用纯谓词）。同 crate 调用（shared.rs / auth.rs）照常。

- [ ] **Step 1: 写失败测试**

在 `src/auth/mod.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_acl_allows_only_default_workspace() {
        let acl: Vec<String> = vec![];
        assert!(is_workspace_authorized("default", &acl, "default"));
        assert!(!is_workspace_authorized("other", &acl, "default"));
    }

    #[test]
    fn non_empty_acl_allows_only_contained() {
        let acl = vec!["ws_a".to_string(), "ws_b".to_string()];
        assert!(is_workspace_authorized("ws_a", &acl, "default"));
        assert!(is_workspace_authorized("ws_b", &acl, "default"));
        // 非空 ACL 下 default 不在列表 → 拒绝（不因 default 特殊放行）
        assert!(!is_workspace_authorized("default", &acl, "default"));
    }

    #[test]
    fn non_empty_acl_rejects_outsider() {
        let acl = vec!["ws_a".to_string()];
        assert!(!is_workspace_authorized("ws_evil", &acl, "default"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败（函数未定义）**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib is_workspace_authorized 2>&1 | tail -20
```
Expected: 编译失败，`cannot find function is_workspace_authorized`。

- [ ] **Step 3: 写最小实现**

在 `src/auth/mod.rs` 末尾（`#[cfg(test)] mod tests` **之前**）追加：

```rust
/// 判定 `resolved` workspace 是否在 admin 的允许列表内。
/// 空列表 = 单租户回落语义：只允许默认 workspace（与 `switch_workspace` 同源）。
pub fn is_workspace_authorized(
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

- [ ] **Step 4: 跑测试确认通过**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib is_workspace_authorized 2>&1 | tail -20
```
Expected: 3 个测试 PASS。

- [ ] **Step 5: commit**

```bash
git add src/auth/mod.rs
git commit -m "$(cat <<'EOF'
feat(security): is_workspace_authorized 纯函数判定 workspace ∈ ACL(H3 地基)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: DB 包装 `resolve_authorized_workspace`

解析请求目标 workspace（override trim 非空优先，否则回落 `current_workspace`）并校验 ∈ ACL。落在 `routes/shared.rs`（私有 mod，与既有 fail-closed workspace helper 同家）。本函数需 DB，独立单测不便；编译通过即交付，端到端验证在 Task 4。

**Files:**
- Modify: `src/routes/shared.rs`（顶部补 import；文件末尾或合适位置加函数）

**Interfaces:**
- Consumes: `is_workspace_authorized`（Task 1）、`get_admin_user`（session.rs）、`AuthenticatedAdmin`、`AppState`、`AppError`/`AppResult`。
- Produces: `pub(super) async fn resolve_authorized_workspace(state: &AppState, admin: &AuthenticatedAdmin, override_ws: Option<String>) -> AppResult<String>`
  - 返回校验通过的 workspace_id（`String`）；越权 → `Err(BadRequest("workspace_not_in_user_acl"))`；admin 查不到 → `Err(Unauthorized("admin_user_not_found"))`。
  - **可见性 `pub(super)`**：14 个站点都在 `routes::*` 子模块内，`pub(super)` 足够（`shared` 是 `routes` 的私有子 mod，`pub(super)` 暴露给 `routes` 及其兄弟模块）。

- [ ] **Step 1: 确认 shared.rs 顶部 import 现状**

Read `src/routes/shared.rs:1-20`。已有 `use crate::{agent, error::{AppError, AppResult}, ...}` 和 `use super::AppState`。需要补 `AuthenticatedAdmin`、`get_admin_user`、`is_workspace_authorized` 的引用。

- [ ] **Step 2: 补 import**

在 `src/routes/shared.rs` 顶部 `use super::AppState;`（:20）下方追加：

```rust
use crate::auth::{is_workspace_authorized, session::get_admin_user, AuthenticatedAdmin};
```

> 注：`crate::auth` 是否已 `pub use` 这些符号需顺带确认——`AuthenticatedAdmin` 在 `auth/mod.rs` 是 `pub struct`，`get_admin_user` 在 `auth::session` 是 `pub async fn`，`is_workspace_authorized` 经 Task 1 是 `pub fn`，三者全路径可达。若 `session` 子模块未 `pub`（auth/mod.rs:16 是 `pub mod session;` → 已 pub），则 `crate::auth::session::get_admin_user` 可达。

- [ ] **Step 3: 写函数实现**

在 `src/routes/shared.rs` 末尾追加：

```rust
/// #H3：解析请求目标 workspace 并校验 ∈ admin ACL，堵认证后水平越权。
///
/// 解析顺序：`override_ws`（trim 后非空）优先，否则回落 `admin.current_workspace`。
/// 校验对**每个请求**都做（含回落值），单一路径无遗漏。失败语义与
/// `switch_workspace` 同源（同错误码字符串）。
pub(super) async fn resolve_authorized_workspace(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    override_ws: Option<String>,
) -> AppResult<String> {
    let resolved = override_ws
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| admin.current_workspace.clone());

    // get_admin_user 返回 Result<_, AuthError>（非 AppResult，无 From<AppError>），
    // 故不能裸 `?`。函数体仅一次 find_one，唯一可能变体是 AuthError::Mongo，
    // 映射成 AppError::Db 与既有错误语义一致（兜底 External 防变体新增时漏接）。
    let user = get_admin_user(&state.db, &admin.user_id)
        .await
        .map_err(|e| match e {
            crate::auth::session::AuthError::Mongo(err) => AppError::Db(err),
            other => AppError::External(format!("admin lookup: {other}")),
        })?
        .ok_or_else(|| AppError::Unauthorized("admin_user_not_found".into()))?;

    if !is_workspace_authorized(&resolved, &user.workspaces, &state.config.default_workspace_id) {
        return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
    }
    Ok(resolved)
}
```

> **实现阶段必做核对：** 读 `src/auth/session.rs` 确认 `AuthError` 的全部变体集合（计划锚定的是 `Mongo(#[from] mongodb::error::Error)` 存在）。若 `AuthError` 不可从 shared.rs 命名（应可——`pub enum` 且 `pub mod session`），改用 `.map_err(|e| AppError::External(format!("admin lookup: {e}")))?` 兜底（牺牲 Db 精度但能编译）。`{other}` / `{e}` 要求 `AuthError: Display`——它派生了 `thiserror::Error`（session.rs:22 起有 `#[error(...)]`），满足 `Display`。

- [ ] **Step 4: cargo check 确认编译通过**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib 2>&1 | tail -20
```
Expected: `Finished`，无 error。可能有 `unused function resolve_authorized_workspace` 的 dead_code warning——Task 3 接入后消失，本步骤可接受该 warning。

- [ ] **Step 5: commit**

```bash
git add src/routes/shared.rs
git commit -m "$(cat <<'EOF'
feat(security): resolve_authorized_workspace 解析+校验 workspace ∈ ACL(H3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 接入 7 个 llm_providers.rs 站点

把 7 处 `X.unwrap_or_else(|| admin.current_workspace.clone())` 替换为 `resolve_authorized_workspace(&state, &admin, X).await?`。handler 其余逻辑不动。每处的 `X` 与来源已确认。

**Files:**
- Modify: `src/routes/llm_providers.rs`（7 处：105 / 155 / 214 / 287 / 322 / 389 / 461）

**Interfaces:**
- Consumes: `resolve_authorized_workspace`（Task 2）。
- 需在 `llm_providers.rs` 顶部 import 处补 `use super::shared::resolve_authorized_workspace;`（确认现有 `use super::AppState;` 旁加）。

- [ ] **Step 1: 补 import**

在 `src/routes/llm_providers.rs` 的 `use super::AppState;`（:37）下方加：

```rust
use super::shared::resolve_authorized_workspace;
```

- [ ] **Step 2: 替换 7 处**

每处模式相同。`list_providers`(105) / `delete_provider`(287) / `activate_provider`(322) 用 `params.workspace_id`（注意 `delete`/`activate` 原本有 `.clone()`）；`create`(155) / `update`(214) / `set_vision_active`(389) / `test_provider`(461) 用 `body.workspace_id.clone()`。

`list_providers`（无 .clone()）：
```rust
// 改前：
    let workspace_id = params
        .workspace_id
        .unwrap_or_else(|| admin.current_workspace.clone());
// 改后：
    let workspace_id = resolve_authorized_workspace(&state, &admin, params.workspace_id).await?;
```

`delete_provider` / `activate_provider`（params + .clone()）：
```rust
// 改前：
    let workspace_id = params
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
// 改后：
    let workspace_id = resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
```

`create_provider` / `update_provider` / `set_vision_active` / `test_provider`（body + .clone()）：
```rust
// 改前：
    let workspace_id = body
        .workspace_id
        .clone()
        .unwrap_or_else(|| admin.current_workspace.clone());
// 改后：
    let workspace_id = resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
```

> 注：`.clone()` 可保留（`Option<String>` clone 廉价，且 body 后续未再用 workspace_id 字段，但保留最稳妥不改其它行）。7 处全部替换后，文件内应已无 `unwrap_or_else(|| admin.current_workspace.clone())`。

- [ ] **Step 3: cargo check 确认编译通过**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib 2>&1 | tail -20
```
Expected: `Finished`，无 error。

- [ ] **Step 4: 确认无残留**

```bash
grep -n "unwrap_or_else(|| admin.current_workspace.clone())" src/routes/llm_providers.rs
```
Expected: 无输出（7 处全替换）。

- [ ] **Step 5: commit**

```bash
git add src/routes/llm_providers.rs
git commit -m "$(cat <<'EOF'
fix(security): llm_providers 7 handler 改用 resolve_authorized_workspace(H3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 接入 5 个 domain_schemas.rs 站点

**Files:**
- Modify: `src/routes/domain_schemas.rs`（5 处：175 / 205 / 247 / 303 / 340）

**Interfaces:**
- Consumes: `resolve_authorized_workspace`（Task 2）。需补 `use super::shared::resolve_authorized_workspace;`（确认是否已 import shared 的其它符号——若已 `use super::shared::parse_object_id;` 之类则合并）。

- [ ] **Step 1: 补 import**

Read `src/routes/domain_schemas.rs` 顶部 use 块，在 `use super::AppState;` 附近加（若已有 `use super::shared::...` 则合并进去）：

```rust
use super::shared::resolve_authorized_workspace;
```

- [ ] **Step 2: 替换 5 处**

`list_domain_schemas`(175) / `delete`(303) / `activate`(340) 用 `params.workspace_id`（原本有 `.clone()`）；`create`(205) / `update`(247) 用 `body.workspace_id.clone()`。模式与 Task 3 完全一致：

```rust
// params 来源（list / delete / activate）：
    let workspace_id = resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
// body 来源（create / update）：
    let workspace_id = resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
```

- [ ] **Step 3: cargo check**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib 2>&1 | tail -20
```
Expected: `Finished`，无 error。

- [ ] **Step 4: 确认无残留**

```bash
grep -n "unwrap_or_else(|| admin.current_workspace.clone())" src/routes/domain_schemas.rs
```
Expected: 无输出。

- [ ] **Step 5: commit**

```bash
git add src/routes/domain_schemas.rs
git commit -m "$(cat <<'EOF'
fix(security): domain_schemas 5 handler 改用 resolve_authorized_workspace(H3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 接入 2 个 domain_profiles.rs 站点

**Files:**
- Modify: `src/routes/domain_profiles.rs`（2 处：71 `list_domain_profiles` / 174 `create_domain_profile`）

**Interfaces:**
- Consumes: `resolve_authorized_workspace`。`domain_profiles.rs:51` 已有 `use super::shared::parse_object_id;` → 合并成 `use super::shared::{parse_object_id, resolve_authorized_workspace};`。

- [ ] **Step 1: 补 import（合并既有 shared import）**

`src/routes/domain_profiles.rs:51` 当前：
```rust
use super::shared::parse_object_id;
```
改为：
```rust
use super::shared::{parse_object_id, resolve_authorized_workspace};
```

- [ ] **Step 2: 替换 2 处**

`list_domain_profiles`(71, params) 与 `create_domain_profile`(174, body)，两处都有 `.clone()`：

```rust
// list_domain_profiles：
    let workspace_id = resolve_authorized_workspace(&state, &admin, params.workspace_id.clone()).await?;
// create_domain_profile：
    let workspace_id = resolve_authorized_workspace(&state, &admin, body.workspace_id.clone()).await?;
```

- [ ] **Step 3: cargo check**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib 2>&1 | tail -20
```
Expected: `Finished`，无 error（此时 `resolve_authorized_workspace` 的 dead_code warning 应消失——14 站点全接入）。

- [ ] **Step 4: 确认 3 文件全无残留**

```bash
grep -rn "unwrap_or_else(|| admin.current_workspace.clone())" src/routes/llm_providers.rs src/routes/domain_schemas.rs src/routes/domain_profiles.rs
```
Expected: 无输出（14 处全替换）。

- [ ] **Step 5: commit**

```bash
git add src/routes/domain_profiles.rs
git commit -m "$(cat <<'EOF'
fix(security): domain_profiles 2 handler 改用 resolve_authorized_workspace(H3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: switch_workspace DRY 重构

`switch_workspace`（auth.rs:144-148）内联了与纯函数同一套 ACL 判定。改调 `is_workspace_authorized` 消除重复，保证两处判定永远一致。`switch_workspace` 已自行加载 `user`（拿 `get_admin_user`），**不**改用 `resolve_authorized_workspace`（其入参 `target` 是必填、非 override 回落，语义不同），只复用最底层纯函数。

**Files:**
- Modify: `src/routes/auth.rs:144-150`

**Interfaces:**
- Consumes: `is_workspace_authorized`（Task 1）。需确认 auth.rs 顶部能命名它——加 `use crate::auth::is_workspace_authorized;`（auth.rs 在 `routes` 下，`crate::auth` 是另一模块，需全路径或 import）。

- [ ] **Step 1: 补 import**

Read `src/routes/auth.rs` 顶部 use 块，加：

```rust
use crate::auth::is_workspace_authorized;
```

- [ ] **Step 2: 替换内联判定**

`src/routes/auth.rs:144-151` 当前：
```rust
    let allowed = if user.workspaces.is_empty() {
        target == state.config.default_workspace_id
    } else {
        user.workspaces.iter().any(|w| w == target)
    };
    if !allowed {
        return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
    }
```
改为：
```rust
    if !is_workspace_authorized(target, &user.workspaces, &state.config.default_workspace_id) {
        return Err(AppError::BadRequest("workspace_not_in_user_acl".into()));
    }
```

> `target` 是 `&str`（`req.workspace_id.trim()` 的结果，:136），纯函数首参收 `&str`，类型匹配。

- [ ] **Step 3: cargo check**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib 2>&1 | tail -20
```
Expected: `Finished`，无 error，无 `allowed` unused warning。

- [ ] **Step 4: 跑既有 lib 测试确认不回归**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib 2>&1 | tail -15
```
Expected: ≥ 350 passed / 0 failed（含 Task 1 新增 3 个）。

- [ ] **Step 5: commit**

```bash
git add src/routes/auth.rs
git commit -m "$(cat <<'EOF'
refactor(security): switch_workspace 复用 is_workspace_authorized 纯函数(H3 DRY)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: 跨租户越权隔离集成测试（CI/Docker）

新建集成测试文件，直调真实 handler 验证端到端越权拒绝。挑两个**完全可达**且最高危的 handler：`activate_provider`（进程级热切换面）+ `list_providers`（读泄漏面），都收 `pub ListQuery`。seed 真实 AdminUser（ACL=`["ws_a"]`）使 `get_admin_user` 命中；用 `current_workspace="ws_a"` 的 admin 尝试 override 到 `ws_b`，断言被拒且 `ws_b` 数据无副作用。

> 为什么只挑这两个：`TestRequest`/`UpsertRequest`/`VisionActivateRequest` 及 schema/profile 的 `ListQuery`/`UpsertRequest` 是 `pub(super)`，外部 test crate 不可命名，无法构造调用。两个可达 handler 已覆盖核心修复路径（`resolve_authorized_workspace` 是 14 站点共用的单一闸，验证它在真实 handler 里生效即证明闸有效）。纯函数三路径已由 Task 1 lib 单测覆盖。

**Files:**
- Create: `tests/h3_cross_tenant_idor.rs`

**Interfaces:**
- Consumes: `TestApp`（`tests/common/mod.rs`）、`bootstrap_admin_if_needed` / `authenticate`（`wechatagent::auth::session`）、`activate_provider` / `list_providers` / `ListQuery`（`wechatagent::routes::llm_providers`）、`AuthenticatedAdmin`。

- [ ] **Step 1: 写测试文件（失败态：handler 当前不校验，越权会成功而非被拒）**

创建 `tests/h3_cross_tenant_idor.rs`：

```rust
//! H3 跨租户 IDOR 回归：handler 解析的 workspaceId 必须 ∈ admin ACL，否则拒绝。
//! 直调真实 handler（activate_provider / list_providers），seed 真实 AdminUser
//! 提供 ACL。全部 #[ignore]，需 Docker testcontainers。
//! CI: `cargo test --test h3_cross_tenant_idor -- --ignored`。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, Query, State};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::LlmProviderConfig;
use wechatagent::routes::llm_providers::{activate_provider, list_providers, ListQuery};

use crate::common::TestApp;

fn admin_ctx(user_id: &str, current_ws: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: "h3_admin".to_string(),
        current_workspace: current_ws.to_string(),
    }
}

fn make_provider(ws: &str, provider_id: &str, active: bool) -> LlmProviderConfig {
    let now = DateTime::now();
    LlmProviderConfig {
        id: None,
        workspace_id: ws.to_string(),
        provider_id: provider_id.to_string(),
        name: provider_id.to_string(),
        format: "openai".to_string(),
        base_url: "http://llm.example/v1".to_string(),
        api_key: "sk-secret-b".to_string(),
        model: "demo-model".to_string(),
        is_active: active,
        timeout_seconds: None,
        max_retries: None,
        retry_base_ms: None,
        supports_vision: false,
        is_vision_active: false,
        created_at: now,
        updated_at: now,
    }
}

/// seed 一个 ACL=[ws_a] 的 admin，返回其真实 user_id。
async fn seed_admin_with_acl(app: &TestApp, ws_a: &str) -> String {
    bootstrap_admin_if_needed(&app.state.db, Some("h3_admin"), Some("pw-h3-123456"), Some(ws_a))
        .await
        .expect("bootstrap admin");
    let user = authenticate(&app.state.db, "h3_admin", "pw-h3-123456")
        .await
        .expect("authenticate");
    user.user_id
}

/// 红线：admin(ACL=[ws_a]) 用 override=ws_b 调 activate_provider 必须被拒，
/// 且 ws_b 的 provider 不被激活（无进程级热切换副作用）。
#[tokio::test]
#[ignore]
async fn activate_provider_blocks_cross_tenant_override() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let ws_b = "ws_b";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    // ws_b 有一条未激活 provider，攻击者想跨租户激活它。
    let coll = app.state.db.llm_provider_configs();
    coll.insert_one(make_provider(ws_b, "victim_provider", false), None)
        .await
        .expect("seed ws_b provider");

    // override workspaceId=ws_b（ACL 外）→ 必须 Err。
    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({ "workspaceId": ws_b })).expect("query"));
    let result = activate_provider(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        Path("victim_provider".to_string()),
        query,
    )
    .await;
    assert!(
        result.is_err(),
        "ACL=[ws_a] 的 admin 用 override=ws_b 激活必须被拒(workspace_not_in_user_acl)"
    );
    assert!(
        format!("{:?}", result.err().unwrap()).contains("workspace_not_in_user_acl"),
        "拒绝错误码必须是 workspace_not_in_user_acl"
    );

    // 副作用断言：ws_b 的 victim_provider 仍未激活。
    let still = coll
        .find_one(doc! { "workspaceId": ws_b, "providerId": "victim_provider" }, None)
        .await
        .expect("find victim")
        .expect("victim exists");
    assert!(
        !still.is_active,
        "越权被拒后 ws_b 的 provider 不应被激活（无热切换副作用）"
    );
}

/// 正向：admin(ACL=[ws_a]) 不带 override（回落 current_workspace=ws_a）调
/// activate_provider 激活自己租户的 provider 应成功。
#[tokio::test]
#[ignore]
async fn activate_provider_allows_own_workspace() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    let coll = app.state.db.llm_provider_configs();
    coll.insert_one(make_provider(ws_a, "mine", false), None)
        .await
        .expect("seed ws_a provider");

    // 不传 workspaceId → 回落 current_workspace=ws_a（∈ ACL）→ 成功。
    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({})).expect("query"));
    let result = activate_provider(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        Path("mine".to_string()),
        query,
    )
    .await;
    assert!(result.is_ok(), "本租户 provider 激活应成功，实际 {result:?}");

    let mine = coll
        .find_one(doc! { "workspaceId": ws_a, "providerId": "mine" }, None)
        .await
        .expect("find mine")
        .expect("mine exists");
    assert!(mine.is_active, "本租户 provider 应被激活");
}

/// 红线：list_providers 用 override=ws_b（ACL 外）必须被拒，不泄漏 ws_b 列表。
#[tokio::test]
#[ignore]
async fn list_providers_blocks_cross_tenant_override() {
    let app = TestApp::start().await;
    let ws_a = "ws_a";
    let ws_b = "ws_b";
    let user_id = seed_admin_with_acl(&app, ws_a).await;

    app.state
        .db
        .llm_provider_configs()
        .insert_one(make_provider(ws_b, "secret_b", true), None)
        .await
        .expect("seed ws_b provider");

    let query: Query<ListQuery> =
        Query(serde_json::from_value(serde_json::json!({ "workspaceId": ws_b })).expect("query"));
    let result = list_providers(
        State(app.state.clone()),
        Extension(admin_ctx(&user_id, ws_a)),
        query,
    )
    .await;
    assert!(
        result.is_err(),
        "list_providers 用 override=ws_b 必须被拒，不能泄漏他租户 provider 列表"
    );
    assert!(
        format!("{:?}", result.err().unwrap()).contains("workspace_not_in_user_acl"),
        "拒绝错误码必须是 workspace_not_in_user_acl"
    );
}
```

- [ ] **Step 2: 本地确认编译（不跑，留 CI 跑 ignored）**

> 本地磁盘紧，编译单个集成 binary 可能撑爆磁盘。若磁盘允许：
```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
export CARGO_INCREMENTAL=0
cargo test --test h3_cross_tenant_idor --no-run 2>&1 | tail -20
```
Expected: 编译通过（`Finished`）。若磁盘报 `os error 112` / `no space left`，先 `rm -rf target/debug/incremental`，仍不行则跳过本地编译、依赖 CI——在 commit message / PR 注明「集成测试本地未编译，留 CI 验证」。

- [ ] **Step 3: commit**

```bash
git add tests/h3_cross_tenant_idor.rs
git commit -m "$(cat <<'EOF'
test(security): H3 跨租户越权隔离集成测试(activate/list provider 真调 handler)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## 收尾：基线门 + 推送 + PR

- [ ] **Step 1: 跑 lib 基线确认不回归**

```bash
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib 2>&1 | tail -15
```
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 2: 全文件确认 14 站点零残留**

```bash
grep -rn "unwrap_or_else(|| admin.current_workspace.clone())" src/routes/
```
Expected: 无输出（若其它文件还有同模式命中，属本次范围外——记录但不在本 PR 处理，见 spec §8）。

- [ ] **Step 3: 推送分支 + 建 PR（需用户授权后执行）**

```bash
git push -u origin fix/h3-cross-tenant-idor
gh pr create --title "fix(security): H3 跨租户 IDOR——workspaceId 解析后校验 ∈ admin ACL" --body "$(cat <<'EOF'
## Summary
- 14 个 admin handler（llm_providers ×7 / domain_schemas ×5 / domain_profiles ×2）接受调用方自带 workspaceId 却无 ACL 校验，认证后可水平越权读写他租户资源。
- 新增纯函数 `is_workspace_authorized` + DB 包装 `resolve_authorized_workspace`，14 站点统一改用；每请求都校验解析出的 workspace ∈ admin ACL。
- `switch_workspace` 内联判定改调同一纯函数（DRY）。
- 错误码复用既有 `workspace_not_in_user_acl`(400) / `admin_user_not_found`(401)。

## Test plan
- [ ] `cargo test --lib`（含 3 个 is_workspace_authorized 单测）≥ 350 / 0
- [ ] CI Baseline gate（R11.6）绿
- [ ] CI Integration（Docker）绿——含 `h3_cross_tenant_idor` 越权隔离测试
- [ ] 禁词 lint 绿

设计：docs/superpowers/specs/2026-06-30-h3-cross-tenant-idor-fix-design.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

> 监控 CI：用 `gh run view <run-id> --json jobs`（权威态）看 Baseline gate + Integration tests 两个必过门。两门均 success 才 squash 合并。

