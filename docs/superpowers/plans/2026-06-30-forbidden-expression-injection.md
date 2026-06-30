# 禁用表达独立段注入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `forbidden_expression`（禁语素材）从决策提示词的「可引用内容资产」段剖出，注入独立的「禁止使用」段，修复语义反转。

**Architecture:** 全部改动在 `src/agent/decision.rs`。新增纯函数 `split_context_assets` 把查回的内容资产按 kind 分流渲染成 `(referable, forbidden)` 两段文本；`load_context_assets` 的 Mongo 查询让 `forbidden_expression` 豁免 tier 过滤（恒注入）、返回值从 `String` 改为 `(String, String)`；提示词模板新增一个「禁止使用」段并在参数列表精确对位插入 `forbidden` 参数。

**Tech Stack:** Rust 2021 / Axum / MongoDB（`mongodb` crate bson `doc!`）/ `cargo test --lib`。

## Global Constraints

- 禁语（forbidden_expression）**恒注入，无视 min_inject_tier**；可引用 4 类（text/faq/script/brand_voice）仍按各自 tier 过滤（保留 PR#63 的 `$in` tier 下推）。
- 本次**只剖 forbidden_expression**；不动 brand_voice/text/faq/script 注入；不加出站硬拦截门；不动 playbook「禁用规则:」段；不动前端；不碰知识库/名片/文件发送。
- best-effort 不变：DB 故障 → `(String::new(), String::new())`，不阻塞决策。
- 占位符对位铁律：模板里新增的 `{}` 在第 N 个，参数列表里 `forbidden_assets` 必须插在第 N 个对应位置，否则后续所有占位符整体移位。
- CI 基线门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0（新函数必须有生产调用方，非仅 #[cfg(test)]，否则 dead-code 失败）。
- 命名红线（CI 硬门）：新增行不得含 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。
- 本 worktree 路径含非 ASCII（工作项目）：vitest 用 `--pool=forks`；`cargo test --lib` 前 `touch src/lib.rs` 强制 relink 避共享 target stale 二进制。

---

### Task 1: 新增纯函数 `split_context_assets`（分流渲染）

**Files:**
- Modify: `src/agent/decision.rs`（在 `load_context_assets` 函数定义之前插入新函数 + 其 `#[cfg(test)]` 测试）
- Test: 同文件 `#[cfg(test)] mod` 内新增单测

**Interfaces:**
- Consumes: `crate::models::ContentAsset`（字段：`kind: String`、`title: String`、`body: Option<String>`）。
- Produces: `pub(crate) fn split_context_assets(assets: Vec<crate::models::ContentAsset>) -> (String, String)` —— 返回 `(referable, forbidden)`，Task 2 的 `load_context_assets` 调用它。

- [ ] **Step 1: 写失败测试**

在 `src/agent/decision.rs` 末尾的测试区（与 `asset_visible_at_tier` 测试同一 `mod`，或紧邻新增一个 `#[cfg(test)] mod split_context_assets_tests`）加：

```rust
#[cfg(test)]
mod split_context_assets_tests {
    use super::split_context_assets;
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
    fn forbidden_goes_to_forbidden_group_others_to_referable() {
        let input = vec![
            asset("faq", "退款政策", Some("7天无理由")),
            asset("forbidden_expression", "保本承诺", Some("不得说保本保收益")),
            asset("script", "开场白", Some("你好")),
        ];
        let (referable, forbidden) = split_context_assets(input);
        // 可引用组带 [kind] 前缀
        assert_eq!(referable, "- [faq] 退款政策: 7天无理由\n- [script] 开场白: 你好");
        // 禁语组不带 kind 标签
        assert_eq!(forbidden, "- 保本承诺: 不得说保本保收益");
    }

    #[test]
    fn empty_groups_return_empty_string() {
        let (referable, forbidden) = split_context_assets(vec![]);
        assert_eq!(referable, "");
        assert_eq!(forbidden, "");
        // 只有可引用 → forbidden 空
        let (r, f) = split_context_assets(vec![asset("text", "A", Some("a"))]);
        assert_eq!(r, "- [text] A: a");
        assert_eq!(f, "");
        // 只有禁语 → referable 空
        let (r2, f2) = split_context_assets(vec![asset("forbidden_expression", "X", Some("x"))]);
        assert_eq!(r2, "");
        assert_eq!(f2, "- X: x");
    }

    #[test]
    fn none_body_renders_empty_without_panic() {
        let (referable, forbidden) = split_context_assets(vec![
            asset("text", "无正文", None),
            asset("forbidden_expression", "禁语无正文", None),
        ]);
        assert_eq!(referable, "- [text] 无正文: ");
        assert_eq!(forbidden, "- 禁语无正文: ");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib split_context_assets_tests 2>&1 | tail -15`
Expected: 编译失败 `cannot find function split_context_assets`（函数尚未定义）。

- [ ] **Step 3: 写最小实现**

在 `src/agent/decision.rs` 中 `load_context_assets`（约 :1483 `pub(crate) async fn load_context_assets`）定义**之前**插入：

```rust
/// 把查回的内容资产按 kind 分流渲染成两段提示词文本：
/// - 可引用组（text/faq/script/brand_voice）：`- [kind] 标题: 正文`，语义＝可引用。
/// - 禁语组（forbidden_expression）：`- 标题: 正文`（不带 kind 标签，段落标题已框定语义）。
/// 返回 (referable, forbidden)，各自 `\n` 连接；某组空 → 空串。
pub(crate) fn split_context_assets(
    assets: Vec<crate::models::ContentAsset>,
) -> (String, String) {
    let mut referable = Vec::new();
    let mut forbidden = Vec::new();
    for asset in assets {
        let body = asset.body.unwrap_or_default();
        if asset.kind == "forbidden_expression" {
            forbidden.push(format!("- {}: {}", asset.title, body));
        } else {
            referable.push(format!("- [{}] {}: {}", asset.kind, asset.title, body));
        }
    }
    (referable.join("\n"), forbidden.join("\n"))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib split_context_assets_tests 2>&1 | tail -8`
Expected: `test result: ok. 3 passed; 0 failed`。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "feat(forbidden-expr): 加 split_context_assets 纯函数(禁语/可引用分流渲染)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `load_context_assets` 查询禁语豁免 tier + 返回 `(String, String)`

**Files:**
- Modify: `src/agent/decision.rs:1483-1530`（`load_context_assets` 函数体）

**Interfaces:**
- Consumes: `split_context_assets(Vec<ContentAsset>) -> (String, String)`（Task 1）。
- Produces: `load_context_assets(...) -> AppResult<(String, String)>`（签名变更，返回 `(referable, forbidden)`）。Task 3 的调用点 `:334` 依赖此签名。

当前函数全文（供精确替换参考）：

```rust
pub(crate) async fn load_context_assets(
    state: &AppState,
    account_id: &str,
    tier: crate::agent::sufficiency::PromptTier,
) -> AppResult<String> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let visible: Vec<&str> = visible_min_tiers_for(tier);
    // Full 档额外纳入「字段缺失」= 老数据按 full 处理；非 Full 档只取显式可见值。
    let tier_cond = if matches!(tier, crate::agent::sufficiency::PromptTier::Full) {
        doc! { "$or": [
            { "min_inject_tier": { "$in": &visible } },
            { "min_inject_tier": { "$exists": false } },
        ] }
    } else {
        doc! { "min_inject_tier": { "$in": &visible } }
    };
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
                "kind": { "$in": ["text", "faq", "script", "brand_voice", "forbidden_expression"] },
                "$and": [ tier_cond ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(12)
                .build(),
        )
        .await?;
    let mut lines = Vec::new();
    while let Some(asset) = cursor.try_next().await? {
        lines.push(format!(
            "- [{}] {}: {}",
            asset.kind,
            asset.title,
            asset.body.unwrap_or_default()
        ));
    }
    Ok(lines.join("\n"))
}
```

- [ ] **Step 1: 替换函数体**

把上面整个 `load_context_assets` 替换为：

```rust
pub(crate) async fn load_context_assets(
    state: &AppState,
    account_id: &str,
    tier: crate::agent::sufficiency::PromptTier,
) -> AppResult<(String, String)> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let visible: Vec<&str> = visible_min_tiers_for(tier);
    // Full 档额外纳入「字段缺失」= 老数据按 full 处理；非 Full 档只取显式可见值。
    let tier_cond = if matches!(tier, crate::agent::sufficiency::PromptTier::Full) {
        doc! { "$or": [
            { "min_inject_tier": { "$in": &visible } },
            { "min_inject_tier": { "$exists": false } },
        ] }
    } else {
        doc! { "min_inject_tier": { "$in": &visible } }
    };
    // 禁语恒注入（安全红线，无视 tier）；可引用 4 类仍受 tier_cond 约束（保留 $in 下推）。
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
                "$and": [ doc! { "$or": [
                    {
                        "kind": { "$in": ["text", "faq", "script", "brand_voice"] },
                        "$and": [ tier_cond ]
                    },
                    { "kind": "forbidden_expression" }
                ] } ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(16)
                .build(),
        )
        .await?;
    let mut collected = Vec::new();
    while let Some(asset) = cursor.try_next().await? {
        collected.push(asset);
    }
    Ok(split_context_assets(collected))
}
```

注意：原顶层 `kind` + `$and:[tier_cond]` 两个键合并进一个 `$and:[ {$or:[...]} ]`，避免顶层重复 `$and` 键碰撞。`account_id` 的 `$or` 是另一个独立顶层键，保持不变。

- [ ] **Step 2: 编译确认（此时调用点 :334 会因签名变更报错，预期）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | grep -E "error|mismatched" | head -10`
Expected: 出现类型不匹配错误，定位在调用点（`assets` 现在是 tuple）—— 这是预期的，Task 3 修复。若**没有**任何 error 反而要警惕（说明调用点没接住签名变化）。

- [ ] **Step 3: 提交（与 Task 3 连续，但先提交查询层）**

> 因签名变更会暂时打断编译，本 Task 不单独跑测试；提交后立即进 Task 3 修调用点恢复编译。

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "feat(forbidden-expr): load_context_assets 禁语豁免tier+返回(referable,forbidden)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 调用点 + 提示词模板加「禁止使用」段（占位符对位）

**Files:**
- Modify: `src/agent/decision.rs:334`（调用点解构）
- Modify: `src/agent/decision.rs:901-914`（模板字符串）
- Modify: `src/agent/decision.rs:950`（参数列表）

**Interfaces:**
- Consumes: `load_context_assets(...) -> AppResult<(String, String)>`（Task 2）。

当前调用点（:334）：

```rust
    let assets = load_context_assets(state, &contact.account_id, tier)
        .await
        .unwrap_or_default();
```

当前模板片段（:900-906）：

```
自由画像字段: {}
可引用内容资产:
{}
{}
{}
可引荐的专属顾问:
{}
```

当前参数列表相关片段（:949-961）：

```rust
        profile_attributes_text,
        assets,
        // ③升档盲区修复：复用同一占位 ...
        format!("{sendable_candidates_text}{sendable_overview_text}"),
        recent_media_text,
        // ④升档盲区 + 红线让位修复：复用「可引荐的专属顾问」占位 ...
        format!("{referral_block}{assist_escalation_hint}{assist_redline_yield}"),
        task_text,
```

- [ ] **Step 1: 改调用点解构（:334）**

把 `:334` 的三行替换为：

```rust
    let (referable_assets, forbidden_assets) = load_context_assets(state, &contact.account_id, tier)
        .await
        .unwrap_or_default();
```

`unwrap_or_default()` 对 `(String, String)` 返回 `(String::new(), String::new())`，best-effort 语义不变。

- [ ] **Step 2: 改模板（:901-905 区域）**

把模板里的：

```
可引用内容资产:
{}
{}
{}
可引荐的专属顾问:
```

替换为（在三个 `{}` 之后、`可引荐的专属顾问:` 之前插入新段，新增 1 个 `{}`）：

```
可引用内容资产:
{}
{}
{}
以下表达禁止使用（运营标注的禁语，不得直接说，也不得改写后变相说）:
{}
可引荐的专属顾问:
```

- [ ] **Step 3: 改参数列表（对位插入 `forbidden_assets`）**

模板顺序：三个可引用 `{}`（referable_assets 槽 / sendable 槽 / recent_media 槽）→ **新「禁止使用」`{}`（forbidden_assets 槽）** → 「可引荐的专属顾问:」`{}`（referral format! 槽）。所以参数列表对应顺序：`referable_assets` → sendable format! → `recent_media_text` → **`forbidden_assets`** → referral format!。

两处改动：①把 `assets,`（:950）改为 `referable_assets,`；②在 `recent_media_text,`（:955）**之后**、`format!("{referral_block}...")`（:960）**之前**插入 `forbidden_assets,`。原有注释行跟随其原占位不动。改完如下：

```rust
        profile_attributes_text,
        referable_assets,
        // ③升档盲区修复：复用同一占位 ...
        format!("{sendable_candidates_text}{sendable_overview_text}"),
        recent_media_text,
        forbidden_assets,
        // ④升档盲区 + 红线让位修复：复用「可引荐的专属顾问」占位 ...
        format!("{referral_block}{assist_escalation_hint}{assist_redline_yield}"),
        task_text,
```

- [ ] **Step 4: 编译确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | tail -5`
Expected: `Finished`，0 error。若报 `argument never used` 或占位符数量不匹配 → 说明 `{}` 与参数数量没对齐，回 Step 2/3 数对。

- [ ] **Step 5: 全量 lib 测试 + dead-code 门**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && touch src/lib.rs && cargo test --lib 2>&1 | tail -5 && RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -3
```
Expected: `cargo test --lib` ≥350 passed / 0 failed；`cargo check --tests` `Finished`（EXIT=0，无 dead-code）。

- [ ] **Step 6: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/agent/decision.rs
git commit -m "feat(forbidden-expr): 提示词加独立禁止使用段+调用点对位 forbidden_assets

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 收口回归 + 双 lint

**Files:** 无代码改动，仅验证。

- [ ] **Step 1: 全量 lib 测试（touch 强制 relink）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && touch src/lib.rs && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥350 passed / 0 failed。

- [ ] **Step 2: dead-code 门（复刻 CI baseline step2）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -3`
Expected: `Finished`，EXIT=0。

- [ ] **Step 3: no-human-takeover lint**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && bash scripts/check-no-human-takeover.sh 2>&1 | tail -3`
Expected: `0 violations`。

- [ ] **Step 4: 新增行禁词扫描**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && git diff origin/main...HEAD | grep -E "^\+" | grep -cE "人工接管|人工介入|人工托管|takeover|hand[ -]?off|接管|人工"`
Expected: `0`。

- [ ] **Step 5: 无新代码改动，本 Task 不提交**（验证-only；若前序有遗漏修复则单独提交）

---

## Self-Review

**1. Spec coverage:**
- D1 禁语恒注入无视 tier → Task 2 查询 `$or` 里 `{kind:"forbidden_expression"}` 不带 tier_cond ✅
- D2 独立段贴可引用后 → Task 3 Step 2 模板位置 ✅
- D3 只剖 forbidden_expression → Task 1 分流只判 `kind=="forbidden_expression"`，其余全进 referable ✅
- D4 一次查询+Rust 纯函数分流 → Task 1（纯函数）+ Task 2（一次 find）✅
- D5 不加出站硬拦截门 → 计划无 guard/gateway 改动 ✅
- 测试（split_context_assets 纯函数）→ Task 1 Step 1 三个测试 ✅
- 错误处理 best-effort → Task 3 Step 1 `unwrap_or_default()` 对 tuple ✅
- limit 12→16 → Task 2 Step 1 ✅

**2. Placeholder scan:** 无 TBD/TODO；每个 code step 都有完整代码块；测试有真实断言。✅

**3. Type consistency:** `split_context_assets(Vec<ContentAsset>) -> (String, String)` 在 Task 1 定义、Task 2 调用一致；`load_context_assets -> AppResult<(String, String)>` 在 Task 2 定义、Task 3 解构一致；`referable_assets`/`forbidden_assets` 命名跨 Task 3 各 step 一致。✅

**4. 已知风险点（实现者注意）：** Task 3 占位符对位是唯一高危点——`forbidden_assets` 必须插在 `recent_media_text` 之后、referral `format!` 之前（Step 3 已给完整正确顺序）。实现者务必以 Step 4 的 `cargo check --lib` 编译结果为准：占位符与参数数量不匹配时 Rust 会直接编译报错，是硬保护。
