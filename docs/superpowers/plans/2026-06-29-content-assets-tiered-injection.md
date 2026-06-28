# 内容资产分档注入 + 清理过期录入项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让文本型内容资产可按条配置「最低注入档」（lean/relational/full），在对应档位起注入决策 prompt（修复绑死 Full、降档失效的缺陷），成为知识库的轻量正交补充；同时清理「新增资产」表单的过期录入项（URL / MCP Media ID 输入框 + moment_media 选项）。

**Architecture:** 后端在 `ContentAsset` 加 `min_inject_tier` 字段 + 纯函数 `asset_visible_at_tier` 判定档位可见性；`load_context_assets` 去掉绑死 Full 的门、改按当前轮 tier 用 `$in` 集合下推查询。前端文本表单去掉 url/mediaId 录入 + moment_media 选项，加注入档下拉。知识库 wiki / 顾问名片 / 文件素材发送链全部隔离不碰。

**Tech Stack:** Rust (Axum) + MongoDB (bson doc!) 后端；React 19 + Vite + TS + Zustand 前端。

## Global Constraints

- 后端 `cargo test --lib` ≥ 350 passed, 0 failed（基线门，不可退）。
- 前端全套测试只增不减；`tsc --noEmit` 0 error；`npm run build` 成功；CSS module 类名在 dist 存活（避 tree-shake）。
- no-human-takeover lint：`src/`、`frontend/src/` 新增行禁 `人工|接管|takeover|hand[ -]?off|人工介入|人工托管|人工托管`。前端文案用 AI 中性词。
- 共享 worktree + 并行会话：提交只 `git add` 具名文件，**禁 `git add -A`/`.`**。
- 非 ASCII 路径（工作项目）：跑 vitest 加 `--pool=threads` 避 worker 超时。
- **安全红线（不可违反）**：①`ContentAsset` 的 `media_id` / `url` 字段**保留不删**（media_id 是文件发送链命脉：ensure_media_uploaded 缓存/防重发不变式/换文件清缓存；url 保留 Option）。②不碰文件素材发送链（send_outbound_media / ensure_media_uploaded / outbox）。③不碰知识库（knowledge_chunks / format_operation_knowledge_* / knowledge_router）。④不碰顾问名片（referral_cards / send_outbound_namecard / referral_block 注入行）。
- `ContentAsset` 加字段后**所有 4 个构造点必须同步补全**否则 E0063：`src/models.rs:6219`、`src/routes/assets.rs:123`、`src/routes/media_assets.rs:158`、`src/agent/media_send.rs:353`。
- 档位序约定：`Lean < Relational < Full`。`min_inject_tier` 闭集 `{lean, relational, full}`，None/非法值按 `full` 处理（等价改造前「仅 Full 注入」）。
- 本地 `cargo test --lib` 受共享 target 并行污染可能 0 测试，全量基线以 CI 单分支为准；本地认 `cargo test --lib content_asset` / 具名子集 + `cargo check --lib --tests`。

---

## Task 1: 数据模型加字段 `min_inject_tier` + 全构造点补全

**Files:**
- Modify: `src/models.rs:978`（ContentAsset struct，review_note 后加字段）
- Modify: `src/models.rs:6219`（测试构造点补全）
- Modify: `src/routes/media_assets.rs:158`（upload 构造点补全）
- Modify: `src/agent/media_send.rs:353`（测试 helper 构造点补全）
- Modify: `src/routes/assets.rs:123`（create 构造点补全 — Task 4 会进一步改这里，本任务先补字段防编译错）

**Interfaces:**
- Produces: `ContentAsset.min_inject_tier: Option<String>` 字段，供 Task 2/3/4 消费。

- [ ] **Step 1: 在 models.rs ContentAsset 加字段**

`src/models.rs` 第 978 行 `pub review_note: Option<String>,` 之后、`created_at` 之前插入：

```rust
    // ===== 文本资产分档注入（2026-06-29）=====
    /// 文本型资产的最低注入档：控制本条资产从哪个档位起注入决策 prompt。
    /// "lean"=任何档恒注入（最常生效）/ "relational"=关系档起 / "full"=仅完整档。
    /// None（缺失/老数据）按 "full" 处理 —— 与改造前「只 Full 注入」逐字等价。
    /// 仅对文本型 kind 有意义；文件型素材（走 sendable 发送链）不读此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_inject_tier: Option<String>,
```

- [ ] **Step 2: 补全 src/models.rs:6219 测试构造点**

定位 `src/models.rs` 里 `let asset = ContentAsset {`（约 :6219），在其字段列表里 `review_note` 同款位置（created_at 之前）加一行：

```rust
            min_inject_tier: None,
```

- [ ] **Step 3: 补全 src/routes/media_assets.rs:158 构造点**

`src/routes/media_assets.rs` 的 `let asset = ContentAsset {`（约 :158）字段列表（created_at 之前）加：

```rust
        min_inject_tier: None,
```

- [ ] **Step 4: 补全 src/agent/media_send.rs:353 测试 helper 构造点**

`src/agent/media_send.rs` 的 `ContentAsset {`（约 :353）字段列表（created_at 之前）加：

```rust
            min_inject_tier: None,
```

- [ ] **Step 5: 补全 src/routes/assets.rs:123 构造点（临时 None，Task 4 改）**

`src/routes/assets.rs` 的 `let asset = ContentAsset {`（约 :123）字段列表（created_at 之前）加：

```rust
        min_inject_tier: None,
```

- [ ] **Step 6: 编译验证**

Run: `cargo check --lib --tests 2>&1 | grep -E "error" | head` （在 worktree 目录，已 export CARGO_TARGET_DIR 共享 target）
Expected: 无 error 输出（4 构造点补全 → 无 E0063）。

- [ ] **Step 7: Commit**

```bash
git add src/models.rs src/routes/media_assets.rs src/agent/media_send.rs src/routes/assets.rs
git commit -m "feat(content-assets): ContentAsset 加 min_inject_tier 字段 + 全构造点补全"
```

---

## Task 2: 纯函数 `asset_visible_at_tier` + `visible_min_tiers_for` + 单测

**Files:**
- Modify: `src/agent/decision.rs`（在 load_context_assets 附近，约 :1404 之前加两个纯函数 + 一个 mod tests）

**Interfaces:**
- Consumes: `crate::agent::sufficiency::PromptTier`（已存在：Lean/Relational/Full），`ContentAsset.min_inject_tier`（Task 1）。
- Produces:
  - `pub(crate) fn asset_visible_at_tier(min_tier: Option<&str>, current: PromptTier) -> bool`
  - `pub(crate) fn visible_min_tiers_for(current: PromptTier) -> Vec<&'static str>` — 返回当前档可见的 min_tier 取值集合（供 Task 3 查询下推）。

- [ ] **Step 1: 写失败测试**

在 `src/agent/decision.rs` 文件末尾（最后一个 `}` 之前不行——加在文件尾部新 mod）加：

```rust
#[cfg(test)]
mod tier_injection_tests {
    use super::{asset_visible_at_tier, visible_min_tiers_for};
    use crate::agent::sufficiency::PromptTier;

    #[test]
    fn lean_asset_visible_in_all_tiers() {
        assert!(asset_visible_at_tier(Some("lean"), PromptTier::Lean));
        assert!(asset_visible_at_tier(Some("lean"), PromptTier::Relational));
        assert!(asset_visible_at_tier(Some("lean"), PromptTier::Full));
    }

    #[test]
    fn relational_asset_hidden_in_lean_visible_from_relational() {
        assert!(!asset_visible_at_tier(Some("relational"), PromptTier::Lean));
        assert!(asset_visible_at_tier(Some("relational"), PromptTier::Relational));
        assert!(asset_visible_at_tier(Some("relational"), PromptTier::Full));
    }

    #[test]
    fn full_asset_visible_only_in_full() {
        assert!(!asset_visible_at_tier(Some("full"), PromptTier::Lean));
        assert!(!asset_visible_at_tier(Some("full"), PromptTier::Relational));
        assert!(asset_visible_at_tier(Some("full"), PromptTier::Full));
    }

    #[test]
    fn none_and_invalid_default_to_full() {
        // None/非法值按 full 处理 → 仅 Full 可见（与改造前逐字等价）
        assert!(!asset_visible_at_tier(None, PromptTier::Lean));
        assert!(!asset_visible_at_tier(None, PromptTier::Relational));
        assert!(asset_visible_at_tier(None, PromptTier::Full));
        assert!(!asset_visible_at_tier(Some("garbage"), PromptTier::Relational));
        assert!(asset_visible_at_tier(Some("garbage"), PromptTier::Full));
    }

    #[test]
    fn visible_set_widens_with_tier() {
        assert_eq!(visible_min_tiers_for(PromptTier::Lean), vec!["lean"]);
        assert_eq!(
            visible_min_tiers_for(PromptTier::Relational),
            vec!["lean", "relational"]
        );
        assert_eq!(
            visible_min_tiers_for(PromptTier::Full),
            vec!["lean", "relational", "full"]
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib tier_injection_tests 2>&1 | tail -15`
Expected: FAIL（asset_visible_at_tier / visible_min_tiers_for 未定义）。

- [ ] **Step 3: 实现两个纯函数**

在 `src/agent/decision.rs` 的 `pub(crate) async fn load_context_assets` 定义（约 :1405）**之前**插入：

```rust
/// 档位序数：Lean=0 < Relational=1 < Full=2。用于分档注入可见性判定。
fn tier_rank(t: crate::agent::sufficiency::PromptTier) -> u8 {
    match t {
        crate::agent::sufficiency::PromptTier::Lean => 0,
        crate::agent::sufficiency::PromptTier::Relational => 1,
        crate::agent::sufficiency::PromptTier::Full => 2,
    }
}

/// 文本资产是否在当前轮档位注入：当前档序 >= 资产最低档序时可见。
/// min_tier=None/非法值按 "full"（序 2）处理 —— 仅 Full 可见，等价改造前。
pub(crate) fn asset_visible_at_tier(
    min_tier: Option<&str>,
    current: crate::agent::sufficiency::PromptTier,
) -> bool {
    let min_rank = match min_tier {
        Some("lean") => 0,
        Some("relational") => 1,
        _ => 2, // "full" / None / 非法值
    };
    tier_rank(current) >= min_rank
}

/// 当前档可见的 min_inject_tier 取值集合（供 Mongo 查询下推 $in）。
/// Lean→{lean}；Relational→{lean,relational}；Full→{lean,relational,full}。
pub(crate) fn visible_min_tiers_for(
    current: crate::agent::sufficiency::PromptTier,
) -> Vec<&'static str> {
    match current {
        crate::agent::sufficiency::PromptTier::Lean => vec!["lean"],
        crate::agent::sufficiency::PromptTier::Relational => vec!["lean", "relational"],
        crate::agent::sufficiency::PromptTier::Full => vec!["lean", "relational", "full"],
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib tier_injection_tests 2>&1 | tail -15`
Expected: PASS（5 tests）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/decision.rs
git commit -m "feat(content-assets): 分档注入纯函数 asset_visible_at_tier + visible_min_tiers_for + 单测"
```

---

## Task 3: `load_context_assets` 按 tier 查询下推 + 去掉绑死 Full 的门

**Files:**
- Modify: `src/agent/decision.rs:1405`（load_context_assets 签名加 tier 参数 + 查询加档位条件）
- Modify: `src/agent/decision.rs:332`（调用点去掉 `if include_business` 门）

**Interfaces:**
- Consumes: `visible_min_tiers_for`（Task 2），`PromptTier`，调用点已有的 `tier` 变量（decision.rs:302 入参）。
- Produces: `load_context_assets(state, account_id, tier)` 新签名。

**说明（关键查询语义）：** Full 档要捞「min_inject_tier ∈ {lean,relational,full}」**以及「字段缺失」**（老数据=full）。Lean/Relational 档只捞集合内显式值（缺失=full 不在这两档可见）。所以 Full 档查询用 `$or: [{min_inject_tier: {$in: [...]}}, {min_inject_tier: {$exists: false}}]`，非 Full 档只用 `{min_inject_tier: {$in: [...]}}`。

- [ ] **Step 1: 改 load_context_assets 签名 + 查询**

`src/agent/decision.rs` 的 `load_context_assets`（:1405-1437）整体替换为：

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

> 注：filter 顶层已有一个 `$or`（account_id），tier_cond 自身在 Full 档也是 `$or`，故用 `$and: [tier_cond]` 包裹避免顶层 `$or` 键冲突。

- [ ] **Step 2: 改调用点去掉绑死 Full 的门**

`src/agent/decision.rs:332-336` 现为：

```rust
    let assets = if include_business {
        load_context_assets(state, &contact.account_id).await?
    } else {
        String::new()
    };
```

替换为（任何档都加载，传入 tier；best-effort 失败空串不阻塞，与现有 sendable 路径一致）：

```rust
    // 文本资产分档注入（2026-06-29）：不再绑死 Full，按当前轮 tier 过滤每条 min_inject_tier。
    // best-effort：DB 故障 → 空串（不阻塞决策，同 reaction_hint / sendable 路径）。
    let assets = load_context_assets(state, &contact.account_id, tier)
        .await
        .unwrap_or_default();
```

- [ ] **Step 3: 编译验证**

Run: `cargo check --lib 2>&1 | grep -E "error" | head`
Expected: 无 error（include_business 仍被其他段使用，不会变 unused）。

- [ ] **Step 4: 跑相关测试 + 全 lib 编译**

Run: `cargo test --lib tier_injection_tests 2>&1 | tail -8` （确认纯函数仍绿）
Run: `cargo check --lib --tests 2>&1 | grep -E "error" | head` （Expected: 无 error）

- [ ] **Step 5: Commit**

```bash
git add src/agent/decision.rs
git commit -m "feat(content-assets): load_context_assets 按 tier 查询下推+去掉绑死Full门(任何档分档注入)"
```

---

## Task 4: 后端 create 端点去 url/media_id 入参 + 加 min_inject_tier

**Files:**
- Modify: `src/routes/assets.rs:31-43`（ContentAssetRequest 去 url/media_id，加 min_inject_tier）
- Modify: `src/routes/assets.rs:113-154`（create_content_asset：构造时 url/media_id=None，min_inject_tier 校验+赋值）
- Modify: `src/routes/assets.rs:82-110`（list 输出加 minInjectTier，url/mediaId 保留）
- Test: `src/routes/assets.rs`（mod tests 新增 min_inject_tier 归一化纯函数测试）

**Interfaces:**
- Consumes: `ContentAsset.min_inject_tier`（Task 1）。
- Produces: `fn normalize_min_inject_tier(raw: Option<&str>) -> String`（归一化：闭集内原值，否则 "full"）。

- [ ] **Step 1: 写归一化纯函数失败测试**

在 `src/routes/assets.rs` 末尾加（若无 mod tests 则新建）：

```rust
#[cfg(test)]
mod tests {
    use super::normalize_min_inject_tier;

    #[test]
    fn normalize_keeps_valid_lowercases_defaults_full() {
        assert_eq!(normalize_min_inject_tier(Some("lean")), "lean");
        assert_eq!(normalize_min_inject_tier(Some("relational")), "relational");
        assert_eq!(normalize_min_inject_tier(Some("full")), "full");
        assert_eq!(normalize_min_inject_tier(None), "full");
        assert_eq!(normalize_min_inject_tier(Some("garbage")), "full");
        assert_eq!(normalize_min_inject_tier(Some("")), "full");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib routes::assets::tests 2>&1 | tail -10`
Expected: FAIL（normalize_min_inject_tier 未定义）。

- [ ] **Step 3: 加归一化纯函数**

`src/routes/assets.rs` 顶部 use 之后加：

```rust
/// 归一化前端传入的 min_inject_tier：闭集 {lean,relational,full} 内保留原值，
/// 否则（None/空/非法）落 "full"（保守，等价改造前仅 Full 注入）。
fn normalize_min_inject_tier(raw: Option<&str>) -> String {
    match raw.map(str::trim) {
        Some("lean") => "lean".to_string(),
        Some("relational") => "relational".to_string(),
        _ => "full".to_string(),
    }
}
```

- [ ] **Step 4: 改 ContentAssetRequest 入参**

`src/routes/assets.rs:31-43` 的 `ContentAssetRequest` 结构，**删除** `url` 和 `media_id` 两行，**新增** `min_inject_tier`：

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContentAssetRequest {
    account_id: Option<String>,
    kind: String,
    title: String,
    body: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    usage_scene: Option<String>,
    min_inject_tier: Option<String>,
}
```

- [ ] **Step 5: 改 create_content_asset 构造**

`src/routes/assets.rs:123-149` 的 ContentAsset 构造块，把 `url`/`media_id`/`min_inject_tier` 三处改为：

```rust
        url: None,
        media_id: None,
```
（其余文件字段保持 None 不变）并把 `min_inject_tier: None,`（Task 1 临时加的）改为：

```rust
        min_inject_tier: Some(normalize_min_inject_tier(payload.min_inject_tier.as_deref())),
```

> 注意：payload 不再有 url/media_id 字段，构造里这两个直接写 None。

- [ ] **Step 6: list 输出加 minInjectTier**

`src/routes/assets.rs` 的 `list_content_assets` 里 json! 块（约 :84-108），在 `"reviewNote": asset.review_note,` 之后加一行（url/mediaId 行保留不动）：

```rust
            "minInjectTier": asset.min_inject_tier,
```

- [ ] **Step 7: 跑测试 + 编译**

Run: `cargo test --lib routes::assets::tests 2>&1 | tail -8`
Expected: PASS。
Run: `cargo check --lib --tests 2>&1 | grep -E "error" | head`
Expected: 无 error。

- [ ] **Step 8: Commit**

```bash
git add src/routes/assets.rs
git commit -m "feat(content-assets): create端点去url/mediaId入参+加min_inject_tier归一化(字段保留)"
```

---

## Task 5: 前端文本表单去 url/mediaId/moment_media + 加注入档下拉

**Files:**
- Modify: `frontend/src/features/content-assets/index.tsx`（KIND_OPTIONS 去 moment_media；表单去 url/mediaId 输入框，加注入档下拉）
- Modify: `frontend/src/stores/contentStore.ts`（assetDraft 去 url/mediaId，加 minInjectTier；createAsset body 同步）
- Modify: `frontend/src/types/index.ts`（ContentAsset 类型加 minInjectTier?，url/mediaId 保留）
- Test: `frontend/src/__tests__/features/content-assets/contentAssets.test.tsx`（补/改断言）

**Interfaces:**
- Consumes: 后端 create 端点新契约（Task 4：不收 url/mediaId，收 minInjectTier）。

- [ ] **Step 1: 改 types/index.ts**

`frontend/src/types/index.ts` 的 ContentAsset interface（约 :211-232），加一行（url/mediaId 保留）：

```typescript
  minInjectTier?: string;
```

- [ ] **Step 2: 改 contentStore.ts 的 assetDraft 与 createAsset**

`frontend/src/stores/contentStore.ts`：
1. `ContentState.assetDraft` 类型（:8-15）：删 `url: string;` `mediaId: string;`，加 `minInjectTier: string;`
2. `setAssetDraft` 入参类型（:19-26）同样改
3. 初始 assetDraft（:44-51）：删 url/mediaId，加 `minInjectTier: "full"`
4. `createAsset` 的 POST body（:78-86）：删 `url`/`mediaId` 两行，加 `minInjectTier: assetDraft.minInjectTier,`
5. createAsset 重置 draft（:89-97）：删 url/mediaId，加 `minInjectTier: assetDraft.minInjectTier`（保留上次选择）

改后 assetDraft 类型：
```typescript
  assetDraft: {
    kind: string;
    title: string;
    body: string;
    usageScene: string;
    minInjectTier: string;
  };
```
改后 POST body：
```typescript
      await api.post("/api/content-assets", {
        accountId: accountId || undefined,
        kind: assetDraft.kind,
        title: assetDraft.title,
        body: assetDraft.body || undefined,
        usageScene: assetDraft.usageScene || undefined,
        minInjectTier: assetDraft.minInjectTier
      });
```

- [ ] **Step 3: 改 index.tsx — 去 moment_media 选项**

`frontend/src/features/content-assets/index.tsx:17` 删除整行：
```typescript
  { value: "moment_media", label: "朋友圈素材" }
```
（注意删掉它前一行末尾可能的逗号，保持数组合法。）

- [ ] **Step 4: 改 index.tsx — 表单去 url/mediaId 输入框，加注入档下拉**

删除「素材 URL」label 块（约 :236-243）和「MCP Media ID」label 块（约 :244-251）。在「使用场景」label 块（:252-259）之后、保存按钮（:260）之前，加注入档下拉：

```tsx
              <label className={styles.field}>
                <span className={styles.fieldLabel}>最低注入档</span>
                <select
                  className={styles.select}
                  value={assetDraft.minInjectTier}
                  onChange={(event) => setAssetDraft({ ...assetDraft, minInjectTier: event.target.value })}
                >
                  <option value="lean">精简档（任何对话都注入，最常生效）</option>
                  <option value="relational">关系档（进入关系经营时注入）</option>
                  <option value="full">完整档（仅深入业务时注入）</option>
                </select>
                <span className={styles.hint}>核心禁语/口吻选精简档时刻生效；重型话术/长 FAQ 选完整档。</span>
              </label>
```

> 已核实：ContentAssets.module.css 有 `.hint`（:104）和 `.select`（:68）、`.fieldLabel`（:64），**无 `.fieldHint`**——故用 `styles.hint`。

- [ ] **Step 5: 改测试 mock 数据 + 断言**

读 `frontend/src/__tests__/features/content-assets/contentAssets.test.tsx`：
1. **必改**：beforeEach 里的 `assetDraft` mock（约 :33-40）含 `url: ""` / `mediaId: ""` 两行 → 删除这两行，加 `minInjectTier: "full"`。否则 TS 类型与新 assetDraft 不符报错。改后：
```typescript
      assetDraft: {
        kind: "text",
        title: "",
        body: "",
        usageScene: "",
        minInjectTier: "full"
      },
```
2. 若有断言「素材 URL」/「MCP Media ID」/「朋友圈素材」文本存在的，改为断言它们**不存在**（`queryByText` 为 null）。现有断言（标题/header/列表展示）不涉及这些字段名，不受影响。
3. 补一条：选注入档下拉 → createAsset POST body 带对应 minInjectTier、不含 url/mediaId。

- [ ] **Step 6: 跑前端测试 + tsc**

Run: `cd frontend && npx vitest run src/__tests__/features/content-assets/ --pool=threads 2>&1 | tail -15`
Expected: 全 PASS。
Run: `cd frontend && npx tsc --noEmit 2>&1 | grep "error TS" | head`
Expected: 空（0 error）。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/content-assets/index.tsx frontend/src/stores/contentStore.ts frontend/src/types/index.ts frontend/src/__tests__/features/content-assets/contentAssets.test.tsx
git commit -m "feat(content-assets-fe): 文本表单去url/mediaId/朋友圈选项+加最低注入档下拉"
```

---

## Task 6: 全套回归 + 双 lint 收口

**Files:** 无新增（验证性任务）

- [ ] **Step 1: 后端 lib 回归**

Run: `cargo test --lib content_asset 2>&1 | tail -10`（content_assets 相关）
Run: `cargo test --lib tier_injection_tests 2>&1 | tail -5`
Run: `cargo test --lib routes::assets 2>&1 | tail -8`
Expected: 全 PASS。
Run: `cargo check --lib --tests 2>&1 | grep -E "error|warning: unused" | head`
Expected: 无 error（unused 警告若来自本改动则修，非本改动忽略）。

- [ ] **Step 2: 前端全套回归**

Run: `cd frontend && npx vitest run --pool=threads 2>&1 | tail -8`
Expected: 全 PASS，0 failed（只增不减）。

- [ ] **Step 3: 前端构建 + CSS 存活**

Run: `cd frontend && npm run build 2>&1 | tail -6`
Expected: built 成功。
Run: `cd frontend && grep -rl "fieldLabel\|select" dist/assets/*.css | head`
Expected: 至少一个 dist CSS 命中（content-assets module 类名存活）。

- [ ] **Step 4: no-human-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 0 violations。

- [ ] **Step 5: 命名红线自查**

Run: `git diff origin/main...HEAD -- src/ frontend/src/ | grep -E "^\+" | grep -iE "人工|接管|takeover|hand[ -]?off|人工介入|人工托管" | head`
Expected: 无输出。

- [ ] **Step 6: 若有收口修补则提交，否则跳过**

```bash
# 仅当本任务触发修补时
git add <具名文件>
git commit -m "chore(content-assets): 回归收口"
```

---

## 完成标准

- 文本资产可按条配置最低注入档（lean/relational/full），在对应档位起注入决策 prompt；修复「绑死 Full、降档失效」缺陷，成为知识库的轻量正交补充。
- 「新增资产」表单去掉 URL / MCP Media ID 录入框 + moment_media 选项，加最低注入档下拉。
- create 端点不再接受 url/media_id 入参；models 的 url/media_id 字段及文件发送链、知识库、顾问名片**完整保留零影响**。
- 全套测试绿 + lib 基线 ≥350/0 + 前端只增不减 + tsc 0 + build 成功 + CSS 存活 + 双 lint 0 命中。

## Self-Review 记录

- **Spec 覆盖**：§3.1 模型→Task1；§3.2 后端注入→Task2(纯函数)+Task3(查询);§3.3 前端→Task5;§3.4 create端点→Task4;§5 测试→各任务TDD+Task6回归;§2 安全边界(media_id/url字段保留)→Task1保留字段+Task4构造None;§4 不做(知识库/名片/发送链/moment后端)→Global Constraints红线。全覆盖。
- **占位符扫描**：无 TBD/TODO；每步有完整代码。Step 4.4 的 fieldHint 给了 fallback 说明（非占位，是真实条件分支）。
- **类型一致**：`min_inject_tier`(Rust snake)/`minInjectTier`(TS/JSON camel) 跨任务一致；`asset_visible_at_tier`/`visible_min_tiers_for`/`normalize_min_inject_tier` 签名 Task2/3/4 一致；档位字面量 lean/relational/full 全任务统一。
