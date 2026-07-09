# roster MCP 限流归入「同步中」应急修复 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** MCP server 返回 HTTP 429/503 限流时，roster 端点不再弹 `internal_error` 红条，而是柔化为 `syncing:true`，前端显示「正在从微信同步好友…」并自动重拉，退避后自愈。

**Architecture:** 新增 `AppError::UpstreamBusy` 变体承载 429/503；`post_rpc` 用一个纯分类函数把非 2xx 状态码分成 UpstreamBusy(429/503) 与 External(其余)；`fetch_roster_for_account` 捕获 UpstreamBusy 转成空 cache 走既有 syncing 重试路径，真实错误仍上抛。前端零改动（复用 #155 已上线的 syncing 态 + 8s 自动重拉）。

**Tech Stack:** Rust 2021 (Axum, thiserror, reqwest, serde_json)；`cargo test --lib`。

设计出处：`docs/superpowers/specs/2026-07-09-roster-mcp-ratelimit-syncing-design.md`。

## Global Constraints

- 基线门不回退：`cargo test --lib` ≥ 350 passed / 0 failed；4 个 PBT 文件（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥ 33 / 0。只增测试不降阈值。
- no-human-takeover lint：`src/` 新增行不得含 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。本改动用 `upstream_busy` / 既有「正在从微信同步好友」文案，均合规。
- 本地磁盘紧 + Windows Defender 对默认 `target/` 有 exec 锁：跑 cargo 用 `CARGO_TARGET_DIR=E:/yw/cargo-target-roster` 覆盖；只跑 `cargo test --lib`（含子串过滤），**不**跑全量 `cargo test`（编 100+ 集成二进制会爆盘）。本机跑不动时改 `cargo check --lib` 证明编译过，报告注明单测留 CI，**不假绿**。
- 项目根含非 ASCII（`工作项目`），用绝对正斜杠路径，避免 `cd`。
- 只改 wechatagent 一端，不碰 MCP server。前端零改动。
- 红线:动手前先 Read 相关文件确认现状与本计划引用一致(post_rpc/fetch_roster_for_account/error.rs 的确切行);引用必亲验。

---

### Task 1: 新增 `AppError::UpstreamBusy` 变体 + IntoResponse 映射

**Files:**
- Modify: `src/error.rs`（变体定义在 enum AppError 内；映射在 `impl IntoResponse`）

**Interfaces:**
- Produces: `AppError::UpstreamBusy(String)` — 承载「上游 MCP 返回 429/503」。`IntoResponse` 映射 503 + `{"error":"upstream_busy"}`。Task 2/3 消费此变体。

- [ ] **Step 1: 加变体定义**

`src/error.rs` 在 `enum AppError` 里，`External(String)` 变体之后（当前 :30-31 `#[error("{0}")] External(String),` 下一行）新增：

```rust
    /// 上游 MCP server 返回 429/503(SSE 连接数满 / 瞬时不可用)。语义是「稍后重试」，
    /// 调用方(如 roster)可捕获并柔化为「同步中」而非硬错误。
    #[error("upstream busy: {0}")]
    UpstreamBusy(String),
```

- [ ] **Step 2: 加 IntoResponse 映射**

`src/error.rs` `impl IntoResponse for AppError` 的 `match self` 里，在 `AppError::LlmUnavailable{..} => (...)` 分支之后、`AppError::Db(_) | AppError::Http(_) | ...` 那组之前，新增独立分支：

```rust
            AppError::UpstreamBusy(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "upstream_busy" })),
            )
                .into_response(),
```

（注意：`UpstreamBusy` 不要加进下面 `AppError::Db(_) | AppError::Http(_) | AppError::Json(_) | AppError::BsonSer(_) | AppError::External(_)` 那组 `internal_error/502` 映射——它有自己的分支。）

- [ ] **Step 3: 加单元测试（新建测试模块）**

`src/error.rs` 无现成测试模块。在文件末尾（`impl IntoResponse` 的收尾 `}` 之后）新增：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn upstream_busy_maps_to_503_upstream_busy() {
        let resp = AppError::UpstreamBusy("MCP HTTP 429: xxx".into()).into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, json!({ "error": "upstream_busy" }));
    }

    #[tokio::test]
    async fn external_still_maps_to_internal_error_502() {
        // 回归守卫:非限流的 External 仍走 internal_error/502,不被新分支误伤。
        let resp = AppError::External("boom".into()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, json!({ "error": "internal_error" }));
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib error::tests`
Expected: `upstream_busy_maps_to_503_upstream_busy` + `external_still_maps_to_internal_error_502` PASS。
若 `to_bytes` 签名因 axum 版本不符编译失败：先 `grep -rn "to_bytes" src/` 看项目现有用法对齐，或改用 `axum::body::to_bytes(body, usize::MAX)` 的项目既有等价写法；仍不行则本任务测试退为「构造 `AppError::UpstreamBusy` + 匹配 `into_response().status()==503`」的最小断言（不读 body），并在报告注明 body 断言留 CI。**不假绿**。

- [ ] **Step 5: Commit**

```bash
git add src/error.rs
git commit -m "feat(error): 新增 AppError::UpstreamBusy 变体(上游 429/503→503 upstream_busy)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `post_rpc` 用纯函数分类非 2xx 状态码

**Files:**
- Modify: `src/mcp.rs`（新增 `fn classify_mcp_http_error`；`post_rpc` 当前 :144-149 调用它）

**Interfaces:**
- Consumes: `AppError::UpstreamBusy`（Task 1）。
- Produces: `fn classify_mcp_http_error(code: u16, detail: String) -> AppError` — 429/503 → `UpstreamBusy(detail)`，其余 → `External(detail)`。`post_rpc` 非 2xx 分支改用它。

- [ ] **Step 1: 写失败测试（纯函数分类）**

`src/mcp.rs` 文件末尾（现有最后一个 `#[cfg(test)] mod` 之后）新增：

```rust
#[cfg(test)]
mod mcp_http_classify_tests {
    use super::classify_mcp_http_error;
    use crate::error::AppError;

    #[test]
    fn http_429_is_upstream_busy() {
        assert!(matches!(
            classify_mcp_http_error(429, "MCP HTTP 429: too many".into()),
            AppError::UpstreamBusy(_)
        ));
    }

    #[test]
    fn http_503_is_upstream_busy() {
        assert!(matches!(
            classify_mcp_http_error(503, "MCP HTTP 503".into()),
            AppError::UpstreamBusy(_)
        ));
    }

    #[test]
    fn http_500_is_external() {
        // 非限流错误仍是 External(→internal_error),不被柔化,不掩盖真问题。
        assert!(matches!(
            classify_mcp_http_error(500, "MCP HTTP 500".into()),
            AppError::External(_)
        ));
    }

    #[test]
    fn http_401_is_external() {
        assert!(matches!(
            classify_mcp_http_error(401, "MCP HTTP 401".into()),
            AppError::External(_)
        ));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib mcp_http_classify_tests`
Expected: 编译失败 —— `classify_mcp_http_error` 未定义。

- [ ] **Step 3: 实现纯函数**

`src/mcp.rs` 在 `fn truncate_for_error`（当前 :236 附近）旁边新增（模块级自由函数，非 impl 内）：

```rust
/// 分类 MCP server 的非 2xx HTTP 响应:429/503(SSE 连接数满/瞬时不可用)→UpstreamBusy
/// (调用方可柔化为「同步中」);其余(401/500 等)→External(→internal_error,不掩盖真错误)。
fn classify_mcp_http_error(code: u16, detail: String) -> AppError {
    if code == 429 || code == 503 {
        AppError::UpstreamBusy(detail)
    } else {
        AppError::External(detail)
    }
}
```

- [ ] **Step 4: post_rpc 改用纯函数**

`src/mcp.rs` `post_rpc`（当前 :144-149）当前：

```rust
            if !status.is_success() {
                return Err(AppError::External(format!(
                    "MCP HTTP {status}: {}",
                    truncate_for_error(&body)
                )));
            }
```
改为：
```rust
            if !status.is_success() {
                return Err(classify_mcp_http_error(
                    status.as_u16(),
                    format!("MCP HTTP {status}: {}", truncate_for_error(&body)),
                ));
            }
```

注意:上方 404 会话失效重握手分支(当前 :138 `status.as_u16() == 404 && !reinitialized`)在此判断**之前**,不受影响——404 首次仍走重握手,重握手后仍失败才落到这里被归为 External。

- [ ] **Step 5: 运行测试确认通过**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib mcp_http_classify_tests`
Expected: 4 个测试全 PASS。
（若本机 target 锁/爆盘跑不动，退 `cargo check --lib` 证明编译过，报告注明单测留 CI，不假绿。）

- [ ] **Step 6: Commit**

```bash
git add src/mcp.rs
git commit -m "feat(mcp): post_rpc 用 classify_mcp_http_error 区分限流(429/503)与真错误

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `fetch_roster_for_account` 捕获 UpstreamBusy 归 syncing

**Files:**
- Modify: `src/mcp.rs`（`fetch_roster_for_account` 循环体，当前 :563-581）

**Interfaces:**
- Consumes: `AppError::UpstreamBusy`（Task 1）；`classify_mcp_http_error`（Task 2，间接经 post_rpc）；既有 `roster_outcome_from_result`（#155）、`roster_result_is_empty_cache`、`RosterFetchOutcome{friends,syncing}`。
- Produces: 行为——MCP 限流时 `fetch_roster_for_account` 返回 `Ok(RosterFetchOutcome{syncing:true})` 而非上抛错误；真实错误仍 `Err` 上抛。

- [ ] **Step 1: 改循环体的错误处理**

`src/mcp.rs` `fetch_roster_for_account` 循环体（当前 :564-570）当前：

```rust
        last_result = logged_call_for_account(
            state,
            account_id,
            "contacts_fetch_cache",
            serde_json::json!({}),
        )
        .await?;
```
改为显式 match（去掉 `?`）：
```rust
        match logged_call_for_account(
            state,
            account_id,
            "contacts_fetch_cache",
            serde_json::json!({}),
        )
        .await
        {
            Ok(v) => {
                last_result = v;
            }
            // 上游限流(429/503):柔化为「同步中」而非硬错误。当作本次空 cache——
            // 还有重试机会则退避重试(退避后 MCP SSE 名额常已释放),用尽仍限流则
            // 返回 syncing:true 让前端提示「同步中」并自动重拉,退避后自愈。
            Err(AppError::UpstreamBusy(_)) => {
                last_result = serde_json::Value::Null;
            }
            // 真实错误(401/500/配置错等)照常上抛 → 前端红条,不掩盖真问题。
            Err(other) => return Err(other),
        }
```

其余不变:后续 `let outcome = roster_outcome_from_result(&last_result);`(Null → `roster_result_is_empty_cache` 为 true → outcome.syncing=true)、`if !outcome.syncing { return Ok(outcome); }`、退避、循环用尽 `Ok(roster_outcome_from_result(&last_result))`。

- [ ] **Step 2: 编译确认**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo check --lib`
Expected: 编译通过（`AppError` 已在 mcp.rs 顶部 `use crate::error::{AppError, AppResult}` 导入，无需改 import）。

- [ ] **Step 3: 加行为回归测试（纯函数层）**

`fetch_roster_for_account` 依赖 `AppState`(真实 MCP/DB)难以纯单测。其「UpstreamBusy → last_result=Null → syncing」这条路径的**判定内核**是「Null 结果 → syncing:true」——此已由 #155 的 `roster_outcome_tests::empty_object_is_syncing`(空 `{}`)覆盖，但 Null 分支值得显式锁一条。在 `src/mcp.rs` 现有 `roster_outcome_tests` 模块(#155 加的)里追加：

```rust
    #[test]
    fn null_result_is_syncing() {
        // fetch_roster_for_account 遇 UpstreamBusy(限流)会把 last_result 置 Null,
        // 应判为空 cache→syncing:true(而非当作真 0 好友或报错)。
        let out = roster_outcome_from_result(&serde_json::Value::Null);
        assert!(out.friends.is_empty());
        assert!(out.syncing, "Null(限流柔化)必须 syncing=true");
    }
```

（说明:此测试锁「Null→syncing」这一判定;「UpstreamBusy→Null」的 match 分支本身是 3 行直白映射,由 code review + 集成层保证,不为它硬造 AppState mock——见设计第四节。）

- [ ] **Step 4: 运行测试确认通过**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib roster_outcome_tests`
Expected: 含新 `null_result_is_syncing` 在内的 `roster_outcome_tests` 全 PASS。

- [ ] **Step 5: 跑 lib 基线不回退**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。
（本机跑不动则 `cargo check --lib` + 报告注明基线留 CI 验，不假绿。）

- [ ] **Step 6: Commit**

```bash
git add src/mcp.rs
git commit -m "fix(roster): MCP 限流(UpstreamBusy)柔化为 syncing 而非 internal_error 红条

fetch_roster_for_account 捕获 AppError::UpstreamBusy→当空 cache 走既有 syncing
重试路径(复用 #155 前端同步中态+8s自动重拉),退避后自愈;真实错误仍上抛。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 部署（实现全绿后，单独动作）

wechatagent 单端部署到 117（前端零改动）：
1. `git`：把本分支合并到 main（走 PR + CI 双门，同 #155 流程）。
2. 117：`cd /opt/wechatagent && git fetch origin main && git merge --ff-only origin/main`。
3. 构建：`source ~/.cargo/env && cargo build --release`（前端无改动，跳过 npm build）。
4. 重启：`systemctl restart wechatagent.service`，`systemctl is-active` + `curl -s -o /dev/null -w "%{http_code}" http://localhost:3003/` 应 200。
5. 浏览器验证:通讯录不再弹红条 `internal_error`;MCP 限流/cache 未就绪时显示「正在从微信同步好友…」并自动重拉,退避后好友列表刷出。

SSH 纪律(踩过的坑):端口 22(脚本 _remote_run.py 默认 3003 错,必传 DEPLOY_PORT=22);高频连触 fail2ban(报 Error reading SSH protocol banner,须长退避~10min);密码走 DEPLOY_PASS 环境变量不进 argv;别高频打 MCP(占 SSE 名额撞 429 影响线上发送)。

## 后续（不在本计划）

MCP server 端由用户新增同步「拉全量+补昵称头像」具名工具后，wechatagent 端另立 spec 做数据源切换（届时 syncing 重试逻辑可能简化为单次同步拉）。本应急修复独立可上线。
