# content_assets 注入加固 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 content_assets 注入的 4 个已核实问题：禁语恒注入被共享 limit 击穿、读写 workspace 锚定不一致、前端禁语 tier 误导、陈旧注释；并补 filter query-shape 单测纵深。

**Architecture:** 后端 `src/agent/decision.rs`：拆 `load_context_assets` 为两次独立 Mongo 查询（可引用 4 类带 tier 下推+limit16 / 禁语无 tier 无 limit 恒全量），三个加载器签名收 `workspace_id` 并由调用点传 `contact.workspace_id`，filter 抽三个纯函数 + `split_context_assets` 拆成两个渲染纯函数，全部配 query-shape / 渲染单测。前端 `index.tsx`：禁语 kind 隐藏 tier 选择器、行内改显「恒注入」徽标。

**Tech Stack:** Rust 2021 / Axum / MongoDB（`mongodb` bson `doc!`）/ `cargo test --lib`；React 19 + Vite + TS + vitest。

## Global Constraints

- 禁语（forbidden_expression）查询**无 tier 过滤、无 limit**，恒全量注入（安全红线）；可引用 4 类（text/faq/script/brand_voice）保留 tier 下推（`$in` + Full 档 `$exists:false` 老数据兜底）+ `limit(16)`。
- 三个 Agent 加载器（load_context_assets / load_sendable_assets / load_referral_cards）签名加 `workspace_id: &str`，调用点传 `&contact.workspace_id`（不再用 `state.config.default_workspace_id`）。
- filter 抽纯函数镜像现有 `build_reaction_hint_filter` / `build_referral_cards_filter` 模式，各配 query-shape 单测。
- best-effort 不变：任一 `find` 失败 → 调用点 `.unwrap_or_default()` → 该段空串，不阻塞决策。
- 不动：brand_voice/text/faq/script 注入语义、playbook「禁用规则:」段、知识库 wiki、文件发送链、min_inject_tier 字段语义、normalize_min_inject_tier。
- CI 基线门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0（新纯函数必须有生产调用方）。
- 命名红线（CI 硬门）：新增行不得含 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。
- 本 worktree 路径含非 ASCII（工作项目）：vitest 用 `--pool=forks`；`cargo test --lib` 前 `touch src/lib.rs` 强制 relink 避共享 target stale 二进制。

---

### Task 1: 三个 filter 纯函数 + query-shape 单测

**Files:**
- Modify: `src/agent/decision.rs`（在 `load_context_assets` 之前加 3 个纯函数；在文件末尾测试区加 query-shape 单测）

**Interfaces:**
- Consumes: `visible_min_tiers_for(tier) -> Vec<&str>`（已存在）、`crate::agent::sufficiency::PromptTier`。
- Produces:
  - `pub(crate) fn build_referable_assets_filter(workspace_id: &str, account_id: &str, tier: PromptTier) -> Document`
  - `pub(crate) fn build_forbidden_assets_filter(workspace_id: &str, account_id: &str) -> Document`
  - `pub(crate) fn build_sendable_assets_filter(workspace_id: &str, account_id: &str) -> Document`
  - Task 3/4 的 load_* 函数调用它们。

- [ ] **Step 1: 写失败测试**

在 `src/agent/decision.rs` 末尾（与 `referral_loader_tests` 同级）新增：

```rust
#[cfg(test)]
mod context_assets_filter_tests {
    //! content_assets 注入加固：三个 query 形状契约。镜像 reaction_hint / referral
    //! 同构——避免 filter 被静默改坏（漏 workspace pin → 跨租户泄漏；禁语误带 tier
    //! → 恒注入被破；可引用漏 tier_cond → 分档失效）。
    use super::*;
    use crate::agent::sufficiency::PromptTier;

    #[test]
    fn referable_filter_lean_pins_workspace_kind_tier_in_only() {
        let f = build_referable_assets_filter("ws", "acct", PromptTier::Lean);
        assert_eq!(f.get_str("workspace_id").ok(), Some("ws"));
        assert!(f.contains_key("$or")); // account_id null / acct
        let kind = f.get_document("kind").unwrap();
        let arr = kind.get_array("$in").unwrap();
        assert_eq!(arr.len(), 4); // text/faq/script/brand_voice
        // 非 Full 档：tier_cond 只有 $in，无 $exists 兜底
        let and = f.get_array("$and").unwrap();
        let tier_cond = and[0].as_document().unwrap();
        let mit = tier_cond.get_document("min_inject_tier").unwrap();
        assert!(mit.contains_key("$in"));
        assert!(!mit.contains_key("$exists"));
    }

    #[test]
    fn referable_filter_full_adds_exists_false_fallback() {
        let f = build_referable_assets_filter("ws", "acct", PromptTier::Full);
        let and = f.get_array("$and").unwrap();
        let tier_cond = and[0].as_document().unwrap();
        // Full 档：tier_cond 是 $or [ {$in}, {$exists:false} ]
        let or = tier_cond.get_array("$or").unwrap();
        assert_eq!(or.len(), 2);
        let has_exists = or.iter().any(|b| {
            b.as_document()
                .and_then(|d| d.get_document("min_inject_tier").ok())
                .map(|m| m.contains_key("$exists"))
                .unwrap_or(false)
        });
        assert!(has_exists);
    }

    #[test]
    fn forbidden_filter_has_no_tier_no_limit_key() {
        let f = build_forbidden_assets_filter("ws", "acct");
        assert_eq!(f.get_str("workspace_id").ok(), Some("ws"));
        assert_eq!(f.get_str("kind").ok(), Some("forbidden_expression"));
        assert!(f.contains_key("$or")); // account
        // 恒注入证据：filter 完全不含 min_inject_tier / tier 相关键
        assert!(!f.contains_key("min_inject_tier"));
        assert!(!f.contains_key("$and"));
    }

    #[test]
    fn sendable_filter_pins_workspace_account_sendable_approved() {
        let f = build_sendable_assets_filter("ws", "acct");
        assert_eq!(f.get_str("workspace_id").ok(), Some("ws"));
        assert_eq!(f.get_bool("sendable").ok(), Some(true));
        assert_eq!(f.get_str("review_status").ok(), Some("approved"));
        assert!(f.contains_key("$or"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib context_assets_filter_tests 2>&1 | tail -15`
Expected: 编译失败 `cannot find function build_referable_assets_filter`（等三个未定义）。

- [ ] **Step 3: 写最小实现**

在 `src/agent/decision.rs` 中 `load_context_assets` 定义**之前**插入：

```rust
/// 可引用内容资产（text/faq/script/brand_voice）query 形状：本 workspace+account
/// （或 account 无关 null）、kind 在可引用白名单、按当前 tier 过滤 min_inject_tier。
/// Full 档纳入「字段缺失」老数据（按 full 处理）；非 Full 档只取显式可见值。
pub(crate) fn build_referable_assets_filter(
    workspace_id: &str,
    account_id: &str,
    tier: crate::agent::sufficiency::PromptTier,
) -> mongodb::bson::Document {
    use mongodb::bson::doc;
    let visible: Vec<&str> = visible_min_tiers_for(tier);
    let tier_cond = if matches!(tier, crate::agent::sufficiency::PromptTier::Full) {
        doc! { "$or": [
            { "min_inject_tier": { "$in": &visible } },
            { "min_inject_tier": { "$exists": false } },
        ] }
    } else {
        doc! { "min_inject_tier": { "$in": &visible } }
    };
    doc! {
        "workspace_id": workspace_id,
        "$or": [ { "account_id": null }, { "account_id": account_id } ],
        "kind": { "$in": ["text", "faq", "script", "brand_voice"] },
        "$and": [ tier_cond ],
    }
}

/// 禁语（forbidden_expression）query 形状：本 workspace+account，**无 tier 过滤**
/// （安全红线恒注入）。调用方对此查询**不设 limit**（禁语数量天然少，恒全量入 prompt）。
pub(crate) fn build_forbidden_assets_filter(
    workspace_id: &str,
    account_id: &str,
) -> mongodb::bson::Document {
    use mongodb::bson::doc;
    doc! {
        "workspace_id": workspace_id,
        "$or": [ { "account_id": null }, { "account_id": account_id } ],
        "kind": "forbidden_expression",
    }
}

/// 可发送素材 query 形状（原 load_sendable_assets 内联 filter 抽出）。
pub(crate) fn build_sendable_assets_filter(
    workspace_id: &str,
    account_id: &str,
) -> mongodb::bson::Document {
    use mongodb::bson::doc;
    doc! {
        "workspace_id": workspace_id,
        "$or": [ { "account_id": null }, { "account_id": account_id } ],
        "sendable": true,
        "review_status": "approved",
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib context_assets_filter_tests 2>&1 | tail -8`
Expected: `test result: ok. 4 passed; 0 failed`。

> 注：这三个纯函数此刻只被测试调用（生产调用在 Task 3/4 接上）。本任务**不要**跑 `RUSTFLAGS="-D warnings" cargo check --tests`（会因暂时无生产调用方报 dead_code，属预期，Task 3/4 接上后消失）。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "feat(ca-hardening): 抽 referable/forbidden/sendable filter 纯函数+query-shape单测

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 两个渲染纯函数（替换 split_context_assets）

**Files:**
- Modify: `src/agent/decision.rs`（删 `split_context_assets`，加 `render_referable_assets` / `render_forbidden_assets`；改造 `split_context_assets_tests`）

**Interfaces:**
- Consumes: `crate::models::ContentAsset`。
- Produces:
  - `pub(crate) fn render_referable_assets(assets: Vec<ContentAsset>) -> String`
  - `pub(crate) fn render_forbidden_assets(assets: Vec<ContentAsset>) -> String`
  - Task 3 的 `load_context_assets` 调用它们。

当前 `split_context_assets`（decision.rs，约 :1490-1504）和它的测试 mod `split_context_assets_tests`（约 :2077）将被替换。

- [ ] **Step 1: 改造测试（先改测试，红）**

把 `split_context_assets_tests` mod 整体替换为（保留 `asset()` helper 不变，改导入和断言）：

```rust
#[cfg(test)]
mod render_assets_tests {
    use super::{render_referable_assets, render_forbidden_assets};
    use crate::models::ContentAsset;
    use mongodb::bson::DateTime;

    fn asset(kind: &str, title: &str, body: Option<&str>) -> ContentAsset {
        ContentAsset {
            id: None,
            workspace_id: "w".into(),
            account_id: None,
            kind: kind.into(),
            title: title.into(),
            body: body.map(|b| b.into()),
            tags: vec![],
            url: None,
            media_id: None,
            usage_scene: None,
            media_type: None,
            file_path: None,
            file_name: None,
            file_size: None,
            mime_type: None,
            file_sha256: None,
            sendable: None,
            send_trigger_hint: None,
            target_stages: None,
            expression_pref: None,
            requires_principal_approval: None,
            review_status: None,
            review_note: None,
            min_inject_tier: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn referable_renders_with_kind_prefix() {
        let out = render_referable_assets(vec![
            asset("faq", "退款政策", Some("7天无理由")),
            asset("script", "开场白", Some("你好")),
        ]);
        assert_eq!(out, "- [faq] 退款政策: 7天无理由\n- [script] 开场白: 你好");
    }

    #[test]
    fn forbidden_renders_without_kind_label() {
        let out = render_forbidden_assets(vec![
            asset("forbidden_expression", "保本承诺", Some("不得说保本保收益")),
        ]);
        assert_eq!(out, "- 保本承诺: 不得说保本保收益");
    }

    #[test]
    fn empty_returns_empty_string() {
        assert_eq!(render_referable_assets(vec![]), "");
        assert_eq!(render_forbidden_assets(vec![]), "");
    }

    #[test]
    fn none_body_renders_empty_without_panic() {
        assert_eq!(render_referable_assets(vec![asset("text", "无正文", None)]), "- [text] 无正文: ");
        assert_eq!(render_forbidden_assets(vec![asset("forbidden_expression", "禁语无正文", None)]), "- 禁语无正文: ");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib render_assets_tests 2>&1 | tail -12`
Expected: 编译失败 `cannot find function render_referable_assets`（split_context_assets 还在但新函数没定义）。

- [ ] **Step 3: 替换实现**

把 `split_context_assets`（约 :1486-1504，含其 `///` 注释）整体替换为：

```rust
/// 渲染可引用内容资产为 prompt 行：`- [kind] 标题: 正文`，`\n` 连接；空 → 空串。
pub(crate) fn render_referable_assets(assets: Vec<crate::models::ContentAsset>) -> String {
    assets
        .into_iter()
        .map(|a| format!("- [{}] {}: {}", a.kind, a.title, a.body.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染禁语为 prompt 行：`- 标题: 正文`（不带 kind 标签，段落标题已框定禁止语义）；
/// `\n` 连接；空 → 空串。
pub(crate) fn render_forbidden_assets(assets: Vec<crate::models::ContentAsset>) -> String {
    assets
        .into_iter()
        .map(|a| format!("- {}: {}", a.title, a.body.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib render_assets_tests 2>&1 | tail -8`
Expected: `test result: ok. 4 passed; 0 failed`。

> 注：此刻 `load_context_assets` 还在调用已删除的 `split_context_assets` → `cargo check --lib` 会报错。这是预期，Task 3 修复。本任务只验 `render_assets_tests` 这组（用 `cargo test --lib render_assets_tests` 单跑，编译该测试需要的符号已就绪）。若单跑也因 load_context_assets 编译失败而带不起来，跳到 Task 3 一起验证（两任务连续提交）。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "feat(ca-hardening): split_context_assets 拆成 render_referable/forbidden 两渲染纯函数

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: load_context_assets 拆两次查询 + 收 workspace

**Files:**
- Modify: `src/agent/decision.rs`（`load_context_assets` 函数体 + 签名；调用点 :334）

**Interfaces:**
- Consumes: `build_referable_assets_filter` / `build_forbidden_assets_filter`（Task 1）、`render_referable_assets` / `render_forbidden_assets`（Task 2）。
- Produces: `load_context_assets(state, workspace_id, account_id, tier) -> AppResult<(String, String)>`（签名加 workspace_id）。

- [ ] **Step 1: 替换 load_context_assets 函数体**

把当前 `load_context_assets`（约 :1506-1554）整体替换为：

```rust
pub(crate) async fn load_context_assets(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    tier: crate::agent::sufficiency::PromptTier,
) -> AppResult<(String, String)> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    // 可引用 4 类：tier 下推 + limit(16)（与改造前一致）。
    let mut ref_cursor = state
        .db
        .content_assets()
        .find(
            build_referable_assets_filter(workspace_id, account_id, tier),
            FindOptions::builder()
                .sort(mongodb::bson::doc! { "updated_at": -1 })
                .limit(16)
                .build(),
        )
        .await?;
    let mut ref_assets = Vec::new();
    while let Some(a) = ref_cursor.try_next().await? {
        ref_assets.push(a);
    }
    // 禁语：无 tier、无 limit（安全红线恒全量注入，绝不与可引用争 limit 名额）。
    let mut forb_cursor = state
        .db
        .content_assets()
        .find(
            build_forbidden_assets_filter(workspace_id, account_id),
            FindOptions::builder()
                .sort(mongodb::bson::doc! { "updated_at": -1 })
                .build(),
        )
        .await?;
    let mut forb_assets = Vec::new();
    while let Some(a) = forb_cursor.try_next().await? {
        forb_assets.push(a);
    }
    Ok((
        render_referable_assets(ref_assets),
        render_forbidden_assets(forb_assets),
    ))
}
```

- [ ] **Step 2: 改调用点（:334）**

把 `:334` 的调用：

```rust
    let (referable_assets, forbidden_assets) = load_context_assets(state, &contact.account_id, tier)
        .await
        .unwrap_or_default();
```

改为（插入 `&contact.workspace_id` 作为第二参）：

```rust
    let (referable_assets, forbidden_assets) = load_context_assets(state, &contact.workspace_id, &contact.account_id, tier)
        .await
        .unwrap_or_default();
```

- [ ] **Step 3: 编译确认**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | tail -6`
Expected: `Finished`，0 error（split_context_assets 已被两渲染函数取代、调用点签名对齐）。若仍报 split_context_assets 未定义 → 说明 Task 2 的删除没生效，回查。

- [ ] **Step 4: 跑相关测试**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib "context_assets_filter_tests" && cargo test --lib "render_assets_tests" && cargo test --lib "tier_injection_tests" 2>&1 | tail -10`
Expected: 三组全 `ok`。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "fix(ca-hardening): load_context_assets 拆两次查询(禁语无limit恒注入)+收 workspace

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: load_sendable_assets / load_referral_cards 收 workspace + 调用点

**Files:**
- Modify: `src/agent/decision.rs`（两函数签名+体；调用点 :344 / :398）

**Interfaces:**
- Consumes: `build_sendable_assets_filter`（Task 1）、`build_referral_cards_filter`（已存在）。
- Produces: `load_sendable_assets(state, workspace_id, account_id)` / `load_referral_cards(state, workspace_id, account_id)`。

- [ ] **Step 1: 改 load_sendable_assets**

签名加 `workspace_id: &str`（在 `account_id` 前），函数体内联 filter 改用 `build_sendable_assets_filter(workspace_id, account_id)`。当前（约 :1371-1396）：

```rust
pub(crate) async fn load_sendable_assets(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<crate::models::ContentAsset>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let mut cursor = state
        .db
        .content_assets()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id }
                ],
                "sendable": true,
                "review_status": "approved",
            },
            FindOptions::builder().sort(doc! { "updated_at": -1 }).limit(30).build(),
        )
        .await?;
    // ...
```

改为：

```rust
pub(crate) async fn load_sendable_assets(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<crate::models::ContentAsset>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let mut cursor = state
        .db
        .content_assets()
        .find(
            build_sendable_assets_filter(workspace_id, account_id),
            FindOptions::builder().sort(doc! { "updated_at": -1 }).limit(30).build(),
        )
        .await?;
    // ...（while let 收集体不变）
```

（`use mongodb::bson::doc;` 仍需保留，sort 用到。）

- [ ] **Step 2: 改 load_referral_cards**

签名加 `workspace_id: &str`，filter 实参从 `&state.config.default_workspace_id` 改为 `workspace_id`。当前（约 :1425-1431）：

```rust
pub(crate) async fn load_referral_cards(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<crate::models::ReferralCard>> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let filter = build_referral_cards_filter(&state.config.default_workspace_id, account_id);
```

改为：

```rust
pub(crate) async fn load_referral_cards(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<crate::models::ReferralCard>> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let filter = build_referral_cards_filter(workspace_id, account_id);
```

- [ ] **Step 3: 改调用点 :344 / :398**

`:344`：

```rust
    let sendable_assets = load_sendable_assets(state, &contact.workspace_id, &contact.account_id)
```

`:398`：

```rust
        let cards = load_referral_cards(state, &contact.workspace_id, &contact.account_id)
```

（两处都只是在 `state,` 后插入 `&contact.workspace_id,`，其余 `.await` / `.unwrap_or_default()` 链不变。）

- [ ] **Step 4: 编译 + 全量 lib 测试 + dead-code 门**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | tail -3 && touch src/lib.rs && cargo test --lib 2>&1 | tail -5 && RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -3
```
Expected: `cargo check --lib` Finished；`cargo test --lib` ≥350 passed / 0 failed；`cargo check --tests` Finished EXIT=0（三 filter 纯函数现已全有生产调用方，无 dead-code）。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "fix(ca-hardening): load_sendable_assets/load_referral_cards 收 contact.workspace_id

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 陈旧注释修正（E）

**Files:**
- Modify: `src/agent/decision.rs:309-310`

- [ ] **Step 1: 改注释**

当前（:309-310）：

```rust
    // 业务组（知识 / 知识路由 / 产品目录 / 持有投影 / 疑似成交 / 可发素材 / 已发素材 /
    //   名片引荐 / 运营方法 / 运营域策略 / 状态机 / 硬运行参数 / 可引用内容资产）——仅 Full 注入。
```

改为（把「可引用内容资产」从业务组移出，单列说明）：

```rust
    // 业务组（知识 / 知识路由 / 产品目录 / 持有投影 / 疑似成交 / 可发素材 / 已发素材 /
    //   名片引荐 / 运营方法 / 运营域策略 / 状态机 / 硬运行参数）——仅 Full 注入。
    // 内容资产例外：可引用内容资产按每条 min_inject_tier 分档注入（不绑死 Full）；
    //   禁语（forbidden_expression）恒注入无视 tier（安全红线）——见 load_context_assets。
```

- [ ] **Step 2: 编译确认（纯注释，应无影响）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | tail -3`
Expected: `Finished`，0 error。

- [ ] **Step 3: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "docs(ca-hardening): 修正:309陈旧注释——可引用资产分档/禁语恒注入非仅Full

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 前端禁语隐藏 tier 选择器 + 恒注入徽标（D）

**Files:**
- Modify: `frontend/src/features/content-assets/index.tsx`
- Test: `frontend/src/__tests__/features/content-assets/contentAssets.test.tsx`

**Interfaces:**
- Consumes: 现有 `kindLabel` / `tierLabel` / `KIND_OPTIONS` / `assetDraft` / `TextAssetRow`。

- [ ] **Step 1: 加恒注入徽标常量 + 行内条件渲染**

在 `tierLabel` 函数附近加常量：

```tsx
const FORBIDDEN_TIER_BADGE = "恒注入";
```

把 TextAssetRow 行内（约 :432-433）：

```tsx
          <span className={styles.kind}>{kindLabel(asset.kind)}</span>
          <span className={styles.kind}>{tierLabel(asset.minInjectTier)}</span>
```

改为：

```tsx
          <span className={styles.kind}>{kindLabel(asset.kind)}</span>
          <span className={styles.kind}>
            {asset.kind === "forbidden_expression" ? FORBIDDEN_TIER_BADGE : tierLabel(asset.minInjectTier)}
          </span>
```

- [ ] **Step 2: 新增表单 tier 选择器条件渲染（:263-275）**

把新增表单的「最低注入档」`<label>`（:263-275 整块）包成条件渲染：`assetDraft.kind !== "forbidden_expression"` 时才渲染。即在 `<label className={styles.field}>`（:263）外层加 `{assetDraft.kind !== "forbidden_expression" && (` ... `)}`。禁语 kind 时整块不渲染（assetDraft.minInjectTier 仍保留默认值 "full"，createAsset 照常带上，后端禁语查询本就无视它，无副作用）。

- [ ] **Step 3: 编辑态 TextAssetRow tier 选择器条件渲染**

TextAssetRow 编辑态里的「最低注入档」select（PR#66 加的，约 :484-495 区域）同样包条件渲染：`asset.kind !== "forbidden_expression"` 才渲染。禁语编辑时不显示档位选择器。

- [ ] **Step 4: 改测试**

在 `contentAssets.test.tsx` 补两条断言（在现有 describe 内）：

```tsx
  // 禁语资产行显示「恒注入」徽标,不显示档位选择器误导
  it("forbidden asset row shows 恒注入 badge not tier label", () => {
    useContentStore.setState({
      assets: [
        { id: "f1", kind: "forbidden_expression", title: "保本承诺", body: "不得说保本", minInjectTier: "full" } as ContentAsset
      ],
    } as Partial<ReturnType<typeof useContentStore.getState>> as never);
    render(<ContentAssetsFeature />);
    expect(screen.getByText("恒注入")).toBeInTheDocument();
    // 不应把禁语渲染成「完整档」误导
    expect(screen.queryByText("完整档")).toBeNull();
  });
```

（若上面 setState 写法与现有测试的 store mock 风格不一致，以现有测试 `beforeEach` 里 `useContentStore.setState({...})` 的写法为准，把 assets 换成单条 forbidden_expression 资产即可。实现时对齐现有风格。）

- [ ] **Step 5: 前端验证**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npx vitest run src/__tests__/features/content-assets/ --pool=forks 2>&1 | tail -8 && npx tsc --noEmit 2>&1 | grep "error TS" | head; echo "TSC_DONE"
```
Expected: vitest 全 passed；`TSC_DONE` 前无 `error TS`。

- [ ] **Step 6: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add frontend/src/features/content-assets/index.tsx frontend/src/__tests__/features/content-assets/contentAssets.test.tsx
git commit -m "fix(ca-hardening-fe): 禁语资产隐藏tier选择器+显恒注入徽标(不再误导档位)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: 收口回归 + 双 lint

**Files:** 无代码改动，仅验证。

- [ ] **Step 1: 全量 lib 测试**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && touch src/lib.rs && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥350 passed / 0 failed。

- [ ] **Step 2: dead-code 门**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -3`
Expected: `Finished`，EXIT=0。

- [ ] **Step 3: 前端 build**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npm run build 2>&1 | tail -5`
Expected: `built in` 成功。

- [ ] **Step 4: 双 lint**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && bash scripts/check-no-human-takeover.sh 2>&1 | tail -3 && echo "HITS=$(git diff origin/main...HEAD -- 'src/**' 'frontend/**' | grep -E '^\+' | grep -cE '人工接管|人工介入|人工托管|takeover|hand[ -]?off|接管|人工')"
```
Expected: `0 violations`；`HITS=0`。

- [ ] **Step 5: 无代码改动，本 Task 不提交**（验证-only）

---

## Self-Review

**1. Spec coverage:**
- A 禁语 limit 击穿 → Task 1（forbidden filter 无 tier）+ Task 3（禁语查询无 limit）✅
- B workspace 锚定 → Task 3（context）+ Task 4（sendable/referral）签名收 workspace + 调用点传 contact.workspace_id ✅
- C filter query-shape 单测 → Task 1 四个测试 ✅
- D 前端禁语 tier → Task 6 行内徽标 + 表单/编辑态条件渲染 ✅
- E 陈旧注释 → Task 5 ✅
- K4 split 拆两渲染函数 → Task 2 ✅

**2. Placeholder scan:** 无 TBD/TODO；每个 code step 有完整代码；测试有真实断言。Task 6 Step 4 测试 setState 写法标注「以现有风格为准」——这是对实现者的合理指引，非占位（现有测试文件已有 beforeEach setState 样板可循）。✅

**3. Type consistency:**
- `build_referable_assets_filter(workspace,account,tier)` / `build_forbidden_assets_filter(workspace,account)` / `build_sendable_assets_filter(workspace,account)` 在 Task 1 定义、Task 3/4 调用一致 ✅
- `render_referable_assets` / `render_forbidden_assets` Task 2 定义、Task 3 调用一致 ✅
- `load_context_assets(state, workspace_id, account_id, tier)` Task 3 改签名、Task 3 Step 2 调用点对齐；`load_sendable_assets(state, workspace_id, account_id)` / `load_referral_cards(state, workspace_id, account_id)` Task 4 改签名+调用点对齐 ✅

**4. 任务顺序依赖：** Task 1（filter）→ Task 2（render）→ Task 3（load_context_assets 用前两者，恢复编译）→ Task 4（其余加载器）→ Task 5（注释）→ Task 6（前端独立）→ Task 7（收口）。Task 1/2 各自暂时无生产调用方（dead-code），故这两个任务不跑 `check --tests`，dead-code 门留到 Task 4 全部接上后验证——已在对应 Step 注明。
