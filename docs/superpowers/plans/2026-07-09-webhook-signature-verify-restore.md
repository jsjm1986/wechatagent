# Webhook 签名校验恢复 实现计划（方案 B）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 wechatagent 校验 gewe-agent 的新签名方案（每账号密钥 + 时间戳时效 + fail-closed 全路径验签），从而安全地把 `WEBHOOK_VERIFY_SIGNATURE` 从联调期的 `false` 回退到 `true`，封死 `:3003` 公网无鉴权入口。

**Architecture:** 在 `WechatAccount` 加每账号明文 `webhook_secret`，在 `AppConfig` 加时间戳偏差窗口。新增纯函数 `verify_webhook_signature` 承载全部校验与单测覆盖。webhook handler 重排为「parse json → testMsg 纯 ack 短路 → parse appId → 查账号（连带取 secret）→ 验签门 → Offline/Online 副作用短路 → 限流 → 持久化 / 喂 Agent」，把验签门下沉到「查到账号密钥之后、任何副作用之前」。旧 `x-mcp-signature` + 全局 `MCP_API_KEY` 方案整体退役。

**Tech Stack:** Rust 2021 / Axum、`hmac` + `sha2` + `hex`（webhooks.rs 已依赖）、bson `DateTime`、MongoDB。

**设计依据：** `docs/superpowers/specs/2026-07-09-webhook-signature-verify-restore-design.md`（已定稿并提交 `e23a692`）。本计划范围 = spec §1–§5 验签核心；账号密钥用 mongosh 写入（部署步骤 3），spec §6 的管理端 API + 前端表单不在本计划内。

## Global Constraints

- 基线不可回退：`cargo test --lib` ≥ 350 passed / 0 failed；四个 PBT 文件累计 ≥ 33 / 0（`state_transition_pbt` / `memory_card_invariants` / `wiki_chunk_revision_pbt` / `llm_retry_jitter`）。新工作只加测试，不降数字。
- 红线：绝不为过测试改业务逻辑 / prompt / guards / 阈值；落地任一改动前先 Read/Grep 亲验相关行号（代码可能已变动），不得凭本文件旧引用直接改。
- no-human-takeover lint 扫 `src/agent/` `src/routes/` `src/evolution/` `frontend/src/` 新增行禁用词——本计划核心改动在 `src/webhooks.rs` / `src/models.rs` / `src/config.rs`（均不在扫描面），但仍禁止在任何新增文案里出现 `人工接管 / takeover / hand-off / 人工` 等词。
- 本地磁盘紧张：只跑 `cargo check` / `cargo check --tests` / `cargo test --lib`；完整集成套件留给 CI。
- 共享 worktree：`git add` 只点名具体文件，绝不 `git add -A` / `git add .`；提交遵循项目「用户显式授权才提交」规则。
- 工作分支 `fix/webhook-signature-verify`（worktree `.claude/worktrees/roster-debug`），所有路径用该 worktree 的绝对路径。

---

### Task 1: `WechatAccount` 加 `webhook_secret` 字段 + Debug 掩码 + 修全部 6 处结构体字面量

**Files:**
- Modify: `src/models.rs`（结构体定义 `:57-94`；手写 `Debug` impl `:98-124`）
- Modify: `src/routes/accounts.rs:87`（`WechatAccount { ... }` 字面量）
- Modify: `src/account_scheduler.rs:258`（`fn account` helper 内字面量）
- Modify: `tests/account_offline_defer_integration.rs:88`
- Modify: `tests/account_round_robin_pbt.rs:25`
- Modify: `tests/account_security_integration.rs:34`
- Modify: `tests/contacts_batch_enable.rs:33`

**Interfaces:**
- Produces: `WechatAccount.webhook_secret: Option<String>`（每账号 gewe-agent slot 明文回调密钥；`None` = 未配置）。Task 3/4 消费该字段。

- [ ] **Step 1: 亲验当前结构体与所有字面量位置**

Run:
```bash
grep -rn "WechatAccount {" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/src" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/tests"
```
Expected: 6 处字面量（accounts.rs、account_scheduler.rs + 4 个 tests 文件），与上方 Files 列表一致。若行号漂移，以实际为准。

- [ ] **Step 2: 结构体定义加字段**

在 `src/models.rs` `WechatAccount` 里 `mcp_api_key` 之后加：
```rust
    pub mcp_api_key: Option<String>,
    /// 方案 B：本账号对应 gewe-agent slot 的明文回调签名密钥
    /// （`messageWebhookSecret`）。用于校验 gewe-agent 转发的
    /// `x-webhook-signature`。`None` = 未配置；验签开关打开时视为拒绝（fail-closed）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
```

- [ ] **Step 3: 手写 `Debug` impl 加掩码字段**

在 `src/models.rs` `impl std::fmt::Debug for WechatAccount` 里，紧跟 `mcp_api_key` 的 `.field(...)` 之后加（复用与 `mcp_api_key` 相同的 `mask_secret`）：
```rust
            .field(
                "webhook_secret",
                &self.webhook_secret.as_deref().map(crate::secret::mask_secret),
            )
```

- [ ] **Step 4: 6 处字面量各加一行**

在下列每个 `WechatAccount { ... }` 字面量里加 `webhook_secret: None,`（放在 `mcp_api_key` 附近即可）：
`src/routes/accounts.rs:87`、`src/account_scheduler.rs:258`、`tests/account_offline_defer_integration.rs:88`、`tests/account_round_robin_pbt.rs:25`、`tests/account_security_integration.rs:34`、`tests/contacts_batch_enable.rs:33`。

- [ ] **Step 5: 加 Debug 掩码单测（失败先行）**

在 `src/models.rs` 文件末尾加（若已有 `#[cfg(test)] mod`，并入即可）：
```rust
#[cfg(test)]
mod wechat_account_debug_tests {
    use super::*;
    use bson::DateTime;

    fn sample() -> WechatAccount {
        WechatAccount {
            id: None,
            workspace_id: "ws".into(),
            account_id: "acc".into(),
            alias: "a".into(),
            display_name: "d".into(),
            app_id: None,
            wxid: None,
            nick_name: None,
            avatar_url: None,
            mcp_base_url: None,
            mcp_api_key: None,
            webhook_secret: Some("super-secret-value".into()),
            online: false,
            status: None,
            last_sync_at: DateTime::now(),
            capacity: 0,
            persona_tag: None,
            off_hours: Vec::new(),
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn debug_masks_webhook_secret() {
        let dbg = format!("{:?}", sample());
        assert!(!dbg.contains("super-secret-value"), "raw webhook_secret leaked into Debug: {dbg}");
    }
}
```

- [ ] **Step 6: 运行单测确认通过 + 全量编译（含 tests）通过**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo test --lib debug_masks_webhook_secret -- --nocapture && cargo check --tests
```
Expected: 该测试 PASS；`cargo check --tests` 无 E0063（漏字段）等错误。

- [ ] **Step 7: 提交（须用户授权后执行）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && git add src/models.rs src/routes/accounts.rs src/account_scheduler.rs tests/account_offline_defer_integration.rs tests/account_round_robin_pbt.rs tests/account_security_integration.rs tests/contacts_batch_enable.rs && git commit -m "feat(models): WechatAccount 加每账号 webhook_secret(明文,Debug 掩码)

方案 B 每账号签名密钥载体。补齐 6 处结构体字面量避免 E0063。

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: `AppConfig` 加 `webhook_timestamp_skew_seconds` + from_env 解析 + 修全部字面量 + `.env.example`

**Files:**
- Modify: `src/config.rs`（结构体字段声明 `:328` 一带；`from_env` 构造 `:694` 一带；`env_or` helper `:711` 已存在）
- Modify: `src/routes/evolution.rs:864`（`test_app_config` helper 字面量）
- Modify: `src/evolution/budget.rs:61`（`cfg` helper 字面量）
- Modify: `tests/common/mod.rs:256`（`test_config` helper 字面量）
- Modify: `tests/jwt_auth.rs:30`（`base_cfg` helper 字面量）
- Modify: `.env.example`

**Interfaces:**
- Produces: `AppConfig.webhook_timestamp_skew_seconds: i64`（默认 300）。Task 4 消费。

- [ ] **Step 1: 亲验所有 `AppConfig {` 字面量位置**

Run:
```bash
grep -rn "AppConfig {" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/src" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/tests"
```
Expected: 5 处——`src/config.rs`（真构造）、`src/routes/evolution.rs`、`src/evolution/budget.rs`、`tests/common/mod.rs`、`tests/jwt_auth.rs`。行号漂移以实际为准。

- [ ] **Step 2: 结构体加字段**

在 `src/config.rs` `webhook_verify_signature: bool,`（`:328`）之后加：
```rust
    pub webhook_verify_signature: bool,
    /// 方案 B：`x-webhook-timestamp`（毫秒）与当前时间允许的最大偏差（秒），
    /// 超窗拒绝以防重放。默认 300（±5 分钟），与 gewe-agent 入站侧 skew 校验对称。
    pub webhook_timestamp_skew_seconds: i64,
```

- [ ] **Step 3: `from_env` 加解析**

在 `src/config.rs` `from_env` 里 `webhook_verify_signature: parse_bool(...)`（`:694`）之后加：
```rust
            webhook_verify_signature: parse_bool(&env_or("WEBHOOK_VERIFY_SIGNATURE", "true")),
            webhook_timestamp_skew_seconds: env_or("WEBHOOK_TIMESTAMP_SKEW_SECONDS", "300").parse()?,
```

- [ ] **Step 4: 4 处 helper 字面量各加一行**

在下列每个 `AppConfig { ... }` 字面量里、`webhook_verify_signature: false,` 之后加 `webhook_timestamp_skew_seconds: 300,`：
`src/routes/evolution.rs:864`、`src/evolution/budget.rs:61`、`tests/common/mod.rs:256`、`tests/jwt_auth.rs:30`。

- [ ] **Step 5: 更新 `.env.example`**

在 `.env.example` 里 `WEBHOOK_VERIFY_SIGNATURE` 一行附近，改注释并加新变量（若无 `WEBHOOK_VERIFY_SIGNATURE` 行则一并补上）：
```dotenv
# webhook 签名校验总开关。生产必须 true（校验 gewe-agent 每账号 x-webhook-signature）；
# 仅联调应急可临时 false（会使 /webhooks/wechat 成为无鉴权入口，勿在生产长期保留）。
WEBHOOK_VERIFY_SIGNATURE=true
# x-webhook-timestamp 允许的最大时间偏差（秒），超窗拒绝以防重放。默认 300。
WEBHOOK_TIMESTAMP_SKEW_SECONDS=300
```

- [ ] **Step 6: 编译（含 tests）通过**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo check --tests
```
Expected: 无 E0063 / 类型错误。

- [ ] **Step 7: 提交（须用户授权后执行）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && git add src/config.rs src/routes/evolution.rs src/evolution/budget.rs tests/common/mod.rs tests/jwt_auth.rs .env.example && git commit -m "feat(config): 加 WEBHOOK_TIMESTAMP_SKEW_SECONDS(默认 300) 防重放窗口

补齐 4 处 AppConfig helper 字面量。

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: 新增 `verify_webhook_signature` 纯函数 + 单测（golden + 全部拒绝路径）

**Files:**
- Modify: `src/webhooks.rs`（新增 `WebhookSigError` enum + `verify_webhook_signature` fn；新增 `#[cfg(test)] mod webhook_sig_tests`）

**Interfaces:**
- Produces:
  ```rust
  enum WebhookSigError { SecretNotConfigured, MissingSignature, MissingTimestamp, BadTimestamp, TimestampOutOfWindow, BadSignatureFormat, Mismatch }
  fn verify_webhook_signature(
      secret: Option<&str>,
      timestamp_header: Option<&str>,
      signature_header: Option<&str>,
      body: &[u8],
      now_ms: i64,
      skew_seconds: i64,
  ) -> Result<(), WebhookSigError>
  ```
  Task 4 在 handler 里调用它。

- [ ] **Step 1: 亲验 webhooks.rs 已有 hmac/sha2/hex 依赖**

Run:
```bash
grep -n "use hmac\|use sha2\|Hmac<Sha256>\|hex::decode" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/src/webhooks.rs"
```
Expected: 现有 `verify_hmac_sha256`（`:1165` 一带）已用 `Hmac<Sha256>` / `hex::decode`，说明 `use` 已就位，新函数无需加依赖。

- [ ] **Step 2: 写失败单测（含 golden 字节对齐样本）**

在 `src/webhooks.rs` 文件末尾加：
```rust
#[cfg(test)]
mod webhook_sig_tests {
    use super::*;

    // 与 gewe-agent webhook-signing.ts 逐字节对齐的金标：
    // HMAC-SHA256(secret="test-secret", "<ts>." + body) hex。
    const SECRET: &str = "test-secret";
    const TS: &str = "1720500000000";
    const BODY: &[u8] = b"{\"foo\":\"bar\"}";
    // python: hmac.new(b"test-secret", b"1720500000000." + BODY, sha256).hexdigest()
    const GOLDEN_HEX: &str = "1936755de0397e2cc912ab1652aaeccb278cae4bb489f16f0dbe3173a8057cbe";
    const NOW_MS: i64 = 1_720_500_000_000; // 与 TS 相等 → 偏差 0
    const SKEW: i64 = 300;

    fn header() -> String {
        format!("sha256={GOLDEN_HEX}")
    }

    #[test]
    fn accepts_correct_signature_within_window() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_signature_without_sha256_prefix() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(GOLDEN_HEX), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn accepts_uppercase_hex() {
        let h = format!("sha256={}", GOLDEN_HEX.to_uppercase());
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&h), BODY, NOW_MS, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_body() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), b"{\"foo\":\"BAR\"}", NOW_MS, SKEW),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_wrong_secret() {
        assert_eq!(
            verify_webhook_signature(Some("other-secret"), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::Mismatch)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_future() {
        // now 比 ts 早 301s（ts 在未来 301s）→ 超窗
        let now = NOW_MS - 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn rejects_timestamp_out_of_window_past() {
        // now 比 ts 晚 301s → 超窗
        let now = NOW_MS + 301_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Err(WebhookSigError::TimestampOutOfWindow)
        );
    }

    #[test]
    fn accepts_timestamp_at_window_edge() {
        // 恰好 300s → 不超窗（用 <= 边界语义）
        let now = NOW_MS + 300_000;
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some(&header()), BODY, now, SKEW),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_signature() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), None, BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some("  "), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingSignature)
        );
    }

    #[test]
    fn rejects_missing_timestamp() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), None, Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::MissingTimestamp)
        );
    }

    #[test]
    fn rejects_bad_timestamp() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some("not-a-number"), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::BadTimestamp)
        );
    }

    #[test]
    fn rejects_bad_signature_format() {
        assert_eq!(
            verify_webhook_signature(Some(SECRET), Some(TS), Some("sha256=not-hex!!"), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::BadSignatureFormat)
        );
    }

    #[test]
    fn rejects_secret_not_configured() {
        assert_eq!(
            verify_webhook_signature(None, Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
        assert_eq!(
            verify_webhook_signature(Some("  "), Some(TS), Some(&header()), BODY, NOW_MS, SKEW),
            Err(WebhookSigError::SecretNotConfigured)
        );
    }
}
```

- [ ] **Step 3: 运行确认测试失败（函数未定义）**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo test --lib webhook_sig_tests 2>&1 | head -30
```
Expected: 编译失败，`cannot find function verify_webhook_signature` / `cannot find type WebhookSigError`。

- [ ] **Step 4: 写实现**

在 `src/webhooks.rs` `verify_hmac_sha256`（`:1165` 一带）之前或之后加：
```rust
/// 方案 B：校验 gewe-agent 每账号签名 + 时间戳时效（纯函数，便于单测）。
///
/// gewe-agent 侧签名内容 = `"<timestamp_header.trim()>." + raw_body`，
/// HMAC-SHA256(每 slot 明文 messageWebhookSecret)，hex 写到
/// `x-webhook-signature: sha256=<hex>`，配套 `x-webhook-timestamp`（毫秒）。
/// 全部通过返回 Ok；否则返回具体拒绝原因（handler 统一转 400 + 脱敏 warn 日志）。
/// `secret=None`/空 → SecretNotConfigured（验签开关打开时的 fail-closed 语义）。
#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)] // Task 4 接入 handler 后即为 live
enum WebhookSigError {
    SecretNotConfigured,
    MissingSignature,
    MissingTimestamp,
    BadTimestamp,
    TimestampOutOfWindow,
    BadSignatureFormat,
    Mismatch,
}

#[allow(dead_code)] // Task 4 接入 handler 后即为 live
fn verify_webhook_signature(
    secret: Option<&str>,
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    now_ms: i64,
    skew_seconds: i64,
) -> Result<(), WebhookSigError> {
    let secret = secret
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::SecretNotConfigured)?;
    let sig = signature_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingSignature)?;
    let ts_str = timestamp_header
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(WebhookSigError::MissingTimestamp)?;
    let ts_ms: i64 = ts_str.parse().map_err(|_| WebhookSigError::BadTimestamp)?;
    if (now_ms - ts_ms).abs() > skew_seconds.saturating_mul(1000) {
        return Err(WebhookSigError::TimestampOutOfWindow);
    }
    let hex_part = sig.strip_prefix("sha256=").unwrap_or(sig);
    let expected = hex::decode(hex_part).map_err(|_| WebhookSigError::BadSignatureFormat)?;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| WebhookSigError::SecretNotConfigured)?;
    // 与 gewe-agent 一致：先喂 "<ts>." 再喂 raw body。
    mac.update(ts_str.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&expected).map_err(|_| WebhookSigError::Mismatch)
}
```

- [ ] **Step 5: 运行确认全部通过**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo test --lib webhook_sig_tests
```
Expected: 13 个测试全 PASS。特别是 `accepts_correct_signature_within_window` 通过 = 与 gewe-agent 字节对齐成立（方案 B 成败关键）。

- [ ] **Step 6: 提交（须用户授权后执行）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && git add src/webhooks.rs && git commit -m "feat(webhook): 加 verify_webhook_signature 纯函数(方案 B)+ 单测

每账号密钥 + x-webhook-timestamp 时效 + sha256= 前缀。含与 gewe-agent
逐字节对齐的 golden HMAC 测试。dead_code allow 待 Task 4 接入后移除。

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: `resolve_account_context` 回带 `webhook_secret` + handler 接入方案 B + 退役旧方案

**Files:**
- Modify: `src/webhooks.rs`（`resolve_account_context` `:972-996`；handler `wechat_webhook` `:287` 起，重点 `:295-398`；删除旧 `verify_hmac_sha256` `:1160-1180` 及其测试模块 `:1372-1417`；移除 Task 3 的两处 `#[allow(dead_code)]`）

**Interfaces:**
- Consumes: `WechatAccount.webhook_secret`（Task 1）、`AppConfig.webhook_timestamp_skew_seconds`（Task 2）、`verify_webhook_signature` / `WebhookSigError`（Task 3）。
- Produces: `resolve_account_context(...) -> AppResult<(String, String, Option<String>)>`（第三元 = 该账号 `webhook_secret`；无 appId 回退分支返回 `None`）。

- [ ] **Step 1: 亲验 handler 当前结构与旧签名门/短路块行号**

Run:
```bash
grep -n "webhook_verify_signature\|x-mcp-signature\|verify_hmac_sha256\|testMsg\|TypeName\|resolve_account_context\|let app_id = find_string" "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug/src/webhooks.rs"
```
Expected: 旧签名门（`webhook_verify_signature` + `x-mcp-signature` + `verify_hmac_sha256`）、testMsg 短路、TypeName(Offline/Online) 短路、appId 解析、`resolve_account_context` 定义与调用位置，与本任务 Files 描述一致。行号漂移以实际为准。

- [ ] **Step 2: 改 `resolve_account_context` 返回类型回带 secret**

把 `src/webhooks.rs:972-996` 改为（返回三元组；命中账号带出 `webhook_secret`，无 appId 回退返回 `None`）：
```rust
async fn resolve_account_context(
    state: &AppState,
    app_id: Option<&str>,
) -> AppResult<(String, String, Option<String>)> {
    if let Some(app_id) = app_id {
        if let Some(account) = state
            .db
            .accounts()
            .find_one(doc! { "app_id": app_id }, None)
            .await?
        {
            return Ok((account.workspace_id, account.account_id, account.webhook_secret));
        }
        return Err(AppError::BadRequest(format!(
            "webhook appId {app_id} not registered in wechat_accounts"
        )));
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
}
```

- [ ] **Step 3: 重排 handler —— 删旧签名门、下沉验签门、Offline/Online 移到门后**

在 `src/webhooks.rs::wechat_webhook` 里做以下四处改动：

**(a) 删除旧签名门** `:295-308` 整块（`if state.config.webhook_verify_signature { ... x-mcp-signature ... verify_hmac_sha256 ... }`）。

**(b) 保留 json 解析与 testMsg 短路不动**（testMsg 无副作用，可留在门前）。删除原 Offline/Online 短路块 `:333-362`（下面 (d) 会在门后重建）。

**(c) 在 appId 解析 + `resolve_account_context` 之后、限流之前**，插入验签门。把原：
```rust
    let (workspace_id, account_id) =
        match resolve_account_context(&state, app_id.as_deref()).await {
            Ok(pair) => pair,
            Err(AppError::BadRequest(msg)) => {
                let _ = emit_unknown_app_id_event(&state, app_id.as_deref()).await;
                return Err(AppError::BadRequest(msg));
            }
            Err(other) => return Err(other),
        };
```
改为：
```rust
    let (workspace_id, account_id, webhook_secret) =
        match resolve_account_context(&state, app_id.as_deref()).await {
            Ok(triple) => triple,
            Err(AppError::BadRequest(msg)) => {
                let _ = emit_unknown_app_id_event(&state, app_id.as_deref()).await;
                return Err(AppError::BadRequest(msg));
            }
            Err(other) => return Err(other),
        };

    // 方案 B 验签门（fail-closed）：签名开关打开时，任何副作用之前必须验签通过。
    // 校验 gewe-agent 每账号 x-webhook-signature + x-webhook-timestamp 时效。
    if state.config.webhook_verify_signature {
        let now_ms = DateTime::now().timestamp_millis();
        if let Err(reason) = verify_webhook_signature(
            webhook_secret.as_deref(),
            headers
                .get("x-webhook-timestamp")
                .and_then(|v| v.to_str().ok()),
            headers
                .get("x-webhook-signature")
                .and_then(|v| v.to_str().ok()),
            &body,
            now_ms,
            state.config.webhook_timestamp_skew_seconds,
        ) {
            tracing::warn!(
                ?reason,
                account_id = %account_id,
                body_len = body.len(),
                "webhook rejected: signature verification failed"
            );
            return Err(AppError::BadRequest("invalid signature".into()));
        }
    }

    // Offline/Online 控制事件（写 online 有副作用，必须在验签门之后）：
    if let Some(type_name) = find_string(&payload, &["TypeName", "typeName"]) {
        let lower = type_name.to_ascii_lowercase();
        if lower == "offline" || lower == "online" {
            let online = lower == "online";
            if let Some(app_id) = app_id.as_deref() {
                // fail-soft：状态落库失败不应让 MCP 侧收不到 ack（会触发重推）。
                let res = state
                    .db
                    .accounts()
                    .update_one(
                        doc! { "app_id": app_id },
                        doc! { "$set": { "online": online, "last_sync_at": DateTime::now() } },
                        None,
                    )
                    .await;
                if let Err(err) = res {
                    tracing::warn!(?err, app_id, online, "persist account online state failed");
                }
            }
            return Ok(Json(serde_json::json!({
                "ok": true,
                "ignored": if online { "online_event" } else { "offline_event" },
                "type": type_name
            })));
        }
    }
```
注意：`workspace_id` 若在后续代码里未直接使用而报 unused 警告，保持与改前一致的用法（改前它已被后续流程消费；若确实未用则改 `let (_workspace_id, ...)`，以实际编译提示为准，勿臆改后续逻辑）。

**(d) 确认 `app_id` 变量在验签门之前已解析**（原 `let app_id = find_string(...)` 在 `:369`，本就在 `resolve_account_context` 之前，位置正确，无需移动）。

- [ ] **Step 4: 删除退役的旧 `verify_hmac_sha256` 及其测试模块**

删除 `src/webhooks.rs` 里 `fn verify_hmac_sha256`（`:1160-1180` 一带，含其 doc 注释）以及其对应的 `#[cfg(test)] mod`（`:1372-1417`，含 `sign` helper 与 7 个 `verify_*` 测试）。同时移除 Task 3 里给 `WebhookSigError` 与 `verify_webhook_signature` 加的两处 `#[allow(dead_code)]`（现已被 handler 调用，成为 live 代码）。

- [ ] **Step 5: 编译（含 tests）通过 + lib 基线不回退**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo check --tests && cargo test --lib 2>&1 | tail -15
```
Expected: `cargo check --tests` 干净（无未用变量/未用函数报错）；`cargo test --lib` 结果 ≥ 350 passed / 0 failed（删了 7 个旧测试、加了 13+1 个新测试，净增，不回退）。

- [ ] **Step 6: 运行四个 PBT 确认基线第二门不回退**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && cargo test --test state_transition_pbt && cargo test --test memory_card_invariants && cargo test --test wiki_chunk_revision_pbt && cargo test --test llm_retry_jitter
```
Expected: 四个文件累计 ≥ 33 passed / 0 failed。

- [ ] **Step 7: 提交（须用户授权后执行）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/roster-debug" && git add src/webhooks.rs && git commit -m "feat(webhook): handler 接入方案 B 验签门 + 退役旧 x-mcp-signature 方案

resolve_account_context 回带每账号 webhook_secret;验签门下沉到查账号后、
副作用前(fail-closed);Offline/Online 写库移到门后,testMsg 纯 ack 保留门前;
删除旧 verify_hmac_sha256 及其 7 个测试。

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## 部署与回退（实现完成、CI 双门绿后执行，详见 spec §7）

严格按序，错序会中断生产消息流：

1. 合并 `fix/webhook-signature-verify` → main（过 CI baseline 双门 + no-human-takeover lint + integration job）。
2. 部署 117，`.env` **仍保持** `WEBHOOK_VERIFY_SIGNATURE=false`（git bundle + paramiko 上传，`setsid` 后台构建，构建完 `systemctl restart wechatagent`）。此时新代码上线但验签仍关，消息流不断。
3. 给账号 102 写 `webhook_secret`（值 = gewe-agent slot 102 明文回调密钥），mongosh 写 `wechat_accounts`。
4. `.env` 改 `WEBHOOK_VERIFY_SIGNATURE=true` + `WEBHOOK_TIMESTAMP_SKEW_SECONDS=300` → `systemctl restart wechatagent`。
5. 验证矩阵（缺一不可）：吴界真实微信消息 → AI 正常回复；伪造无签名/错签名 POST → 400 无副作用；时间戳超窗 POST → 400。

## Self-Review 记录

- **Spec 覆盖**：§3.1 每账号密钥 → Task 1；§3.2 时间戳时效 → Task 2（窗口）+ Task 3（校验逻辑）；§3.3 方案 B fail-closed 全路径验签 → Task 4；§4 错误处理（各 400、fail-closed、逃生阀）→ Task 3 拒绝路径 + Task 4 handler `webhook_verify_signature` 分支保留逃生阀；§5 测试（纯函数 + golden 字节对齐 + 退役旧 fn）→ Task 3 + Task 4 Step 4；§6 变更面 → Task 1-4（管理端 API/前端明确不在本计划范围，密钥走 mongosh）。
- **占位符扫描**：无 TBD/TODO；每个 code step 均给完整代码。
- **类型一致性**：`verify_webhook_signature` / `WebhookSigError` 签名在 Task 3 Interfaces 定义、Task 3 实现、Task 4 调用三处一致；`resolve_account_context` 三元组返回类型在 Task 4 Interfaces 与实现一致；`webhook_secret: Option<String>` / `webhook_timestamp_skew_seconds: i64` 跨 Task 一致。
