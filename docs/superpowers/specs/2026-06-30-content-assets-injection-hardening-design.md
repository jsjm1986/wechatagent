# content_assets 注入加固 设计 spec

> 日期：2026-06-30　基线：origin/main（含 PR#63/#66/#69 三个 content_assets 改造）

## 背景

对本对话交付的 content_assets 三个 PR（#63 分档注入 / #66 可用性补全 / #69 禁语独立段）做了多维度饱和式验证（6 维 agent 并行 + 交叉验证），逐行独立核实后确认 4 个真问题 + 1 个测试纵深缺口。本 spec 是这一包的加固修复。

全部已亲自读 `src/agent/decision.rs` / `src/webhooks.rs` / `frontend/src/features/content-assets/index.tsx` 核实为真，无误报。

## 已核实的问题

| # | 问题 | 严重度 | 来源 | 核实证据 |
| --- | --- | --- | --- | --- |
| A | 禁语恒注入被共享 `limit(16)` 击穿 | Important | 本三PR引入 | `decision.rs:1525-1546` 单 `find()` 把禁语支（无 tier_cond）和可引用 4 类（带 tier_cond）合并进 `$or`，`limit(16)`+`sort(updated_at:-1)` 作用于合并后全集。账号话术/FAQ 多时，较旧的禁语被截断挤出 → `forbidden` 组空 → 模板「禁止使用」段空 → 安全红线静默失效，运营无信号。 |
| B | 读写 workspace 锚定不一致 | Important | 既有 | `load_context_assets:1530` / `load_sendable_assets:1383` 写死 `state.config.default_workspace_id`，且签名只收 `account_id` 不收 workspace；`load_referral_cards:1431` 经 `build_referral_cards_filter` 也喂 default。但 `contact.workspace_id` 来自 `account.workspace_id`（`webhooks.rs:950`）可非 default。多租户启用即三段注入（含安全红线禁语）静默失效。姊妹 `load_recent_reaction_hint:225` 用 `contact.workspace_id`，证明该层本应传真实 workspace。 |
| C | filter 内联无 query-shape 单测 | Minor（测试纵深） | 本三PR | `load_context_assets:1506-1554` / `load_sendable_assets:1378-1395` 的 Mongo filter 完全内联；同文件 `build_reaction_hint_filter:256` / `build_referral_cards_filter:1406` 已确立「filter 抽纯函数 + query-shape 单测」模式，本查询独缺。 |
| D | 前端禁语仍显示 tier 选择器/标签 | Minor | 本三PR | `index.tsx:432-433` 对所有文本类（含 forbidden_expression）显示 `tierLabel`；新增表单 `:263-275` + 编辑态 TextAssetRow 的「最低注入档」选择器对禁语一视同仁。但禁语恒注入无视 tier，运营给禁语设「完整档」会误以为仅深入业务时注入。 |
| E | 陈旧注释 | Minor | 本三PR | `decision.rs:309-310` 仍把「可引用内容资产」列在「业务组——仅 Full 注入」，与 `:334` 无条件分档调用（不受 `include_business` 门控）矛盾。 |

## 设计决策（已与用户对齐）

| # | 决策 | 取值 |
| --- | --- | --- |
| K1 | 禁语 limit 击穿（A）修法 | **拆两次独立查询**：可引用 4 类一次（带 tier 下推 + limit16）、禁语一次（无 tier + 无 limit，恒全量）。物理隔离，禁语永不与可引用争名额。 |
| K2 | workspace 修复（B）范围 | **三加载器一起修**：`load_context_assets` / `load_sendable_assets` / `load_referral_cards` 签名都加 `workspace_id`，调用点传 `contact.workspace_id`。同根一次性根治。 |
| K3 | filter 抽纯函数（C）范围 | **可引用 + 禁语 + sendable 都抽** `build_*_filter` 纯函数 + query-shape 单测；`build_referral_cards_filter` 已是纯函数，只改调用点 workspace 实参。 |
| K4 | `split_context_assets` 去留 | **拆成两个渲染纯函数** `render_referable_assets` / `render_forbidden_assets`：查询已天然分流，渲染只管格式。原 split 的渲染逻辑与测试改造成这两个。 |
| K5 | 前端禁语 tier 控件（D） | **隐藏选择器 + 显示「恒注入」徽标**：`kind==="forbidden_expression"` 时表单/编辑态不渲染「最低注入档」选择器，行内 chip 改显「恒注入」。 |

## 改动单元

### 后端 `src/agent/decision.rs`

**1. 拆查询 + filter 纯函数（A + C + K1 + K3）**

新增三个 filter 纯函数（镜像 `build_reaction_hint_filter` 风格）：

```rust
/// 可引用内容资产（text/faq/script/brand_voice）query 形状：本 workspace+account
/// （或 account 无关 null）、kind 在可引用白名单、且按当前 tier 过滤 min_inject_tier。
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
/// （安全红线恒注入）。调用方对此查询不设 limit（禁语数量天然少，恒全量入 prompt）。
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

**2. 拆渲染纯函数（K4）** —— 替换 `split_context_assets`：

```rust
/// 渲染可引用内容资产为 prompt 行：`- [kind] 标题: 正文`，`\n` 连接；空 → 空串。
pub(crate) fn render_referable_assets(assets: Vec<crate::models::ContentAsset>) -> String {
    assets.into_iter()
        .map(|a| format!("- [{}] {}: {}", a.kind, a.title, a.body.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染禁语为 prompt 行：`- 标题: 正文`（不带 kind 标签，段落标题已框定禁止语义）；
/// `\n` 连接；空 → 空串。
pub(crate) fn render_forbidden_assets(assets: Vec<crate::models::ContentAsset>) -> String {
    assets.into_iter()
        .map(|a| format!("- {}: {}", a.title, a.body.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}
```

**3. `load_context_assets` 改两次查询 + 收 workspace（A + B + K1 + K2）**

```rust
pub(crate) async fn load_context_assets(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    tier: crate::agent::sufficiency::PromptTier,
) -> AppResult<(String, String)> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    // 可引用 4 类：tier 下推 + limit(16)
    let mut ref_cursor = state.db.content_assets()
        .find(
            build_referable_assets_filter(workspace_id, account_id, tier),
            FindOptions::builder().sort(mongodb::bson::doc! { "updated_at": -1 }).limit(16).build(),
        ).await?;
    let mut ref_assets = Vec::new();
    while let Some(a) = ref_cursor.try_next().await? { ref_assets.push(a); }
    // 禁语：无 tier、无 limit（恒全量注入，安全红线）
    let mut forb_cursor = state.db.content_assets()
        .find(
            build_forbidden_assets_filter(workspace_id, account_id),
            FindOptions::builder().sort(mongodb::bson::doc! { "updated_at": -1 }).build(),
        ).await?;
    let mut forb_assets = Vec::new();
    while let Some(a) = forb_cursor.try_next().await? { forb_assets.push(a); }
    Ok((render_referable_assets(ref_assets), render_forbidden_assets(forb_assets)))
}
```

**4. `load_sendable_assets` / `load_referral_cards` 收 workspace（B + K2 + K3）**
- `load_sendable_assets(state, workspace_id, account_id)`：filter 改用 `build_sendable_assets_filter(workspace_id, account_id)`。
- `load_referral_cards(state, workspace_id, account_id)`：`build_referral_cards_filter(workspace_id, account_id)`（纯函数已存在，只把实参从 `state.config.default_workspace_id` 改成传入 `workspace_id`）。

**5. 三调用点传 contact.workspace_id（B + K2）**
- `:334` `load_context_assets(state, &contact.workspace_id, &contact.account_id, tier)`
- `:344` `load_sendable_assets(state, &contact.workspace_id, &contact.account_id)`
- `:398` `load_referral_cards(state, &contact.workspace_id, &contact.account_id)`

**6. 陈旧注释（E）** —— `:309-310` 把「可引用内容资产」从「业务组（仅 Full）」移出，改述「可引用内容资产按每条 min_inject_tier 分档注入；禁语恒注入无视 tier」。

### 前端 `frontend/src/features/content-assets/index.tsx`（D + K5）

- 新增表单（`:263-275`）：`assetDraft.kind === "forbidden_expression"` 时不渲染「最低注入档」`<label>`（整块条件渲染）。
- 编辑态 TextAssetRow：同样 `asset.kind === "forbidden_expression"` 时不渲染 tier 选择器。
- 行内标签（`:432-433`）：禁语时 tierLabel 那个 chip 改显「恒注入」（可加一个常量 `FORBIDDEN_TIER_BADGE = "恒注入"`），非禁语仍显 `tierLabel(asset.minInjectTier)`。

## 数据流（修复后）

```
contact(workspace_id 真实, account_id)
  ├─ load_context_assets(workspace, account, tier)
  │    ├─ find1: build_referable_assets_filter (tier下推, limit16) → render_referable → referable
  │    └─ find2: build_forbidden_assets_filter (无tier, 无limit)   → render_forbidden → forbidden（恒全量）
  ├─ load_sendable_assets(workspace, account) → build_sendable_assets_filter
  └─ load_referral_cards(workspace, account) → build_referral_cards_filter
       referable → 「可引用内容资产」段；forbidden → 「禁止使用」段（永不被 limit 挤空）
```

## 错误处理

不变，best-effort。两次查询任一 `find` 失败 → `load_context_assets` 返 `Err` → 调用点 `.unwrap_or_default()` → `(String::new(), String::new())`，对应段空、不阻塞决策。禁语查询失败 → forbidden 段空（与改造前同等降级，不比现状更糟）。

## 测试

- **3 个新 filter 纯函数 query-shape 单测**（镜像 `build_reaction_hint_filter` 现有测试）：
  - `build_referable_assets_filter`：workspace/account `$or`/kind `$in` 4 类/tier_cond（Lean 非 Full 只 `$in`；Full 含 `$exists:false` 兜底）。
  - `build_forbidden_assets_filter`：workspace/account/`kind="forbidden_expression"`、**断言不含 min_inject_tier 任何键**（恒注入证据）。
  - `build_sendable_assets_filter`：workspace/account/sendable/review_status。
- **2 个渲染纯函数单测**（改造原 `split_context_assets_tests`）：
  - `render_referable_assets`：带 `[kind]` 前缀、多条 `\n`、空 → 空串、body None 不 panic。
  - `render_forbidden_assets`：不带 kind 标签、空 → 空串、body None。
- 现有 `asset_visible_at_tier` / `visible_min_tiers_for` 测试不动（仍服务 build_referable_assets_filter 的 tier 逻辑）。
- **前端** contentAssets.test.tsx 补：禁语行不显示 tier 选择器/显示「恒注入」徽标；非禁语仍显 tierLabel。
- 回归门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0；前端 vitest（`--pool=forks`）+ tsc + build；no-human-takeover lint 0 + 新增行禁词扫描 0。

## 不做（YAGNI）

- 不加禁语出站硬拦截门（延续 PR#69 边界，禁语仍依赖 LLM 遵从）。
- 不动 brand_voice/text/faq/script 注入语义、playbook「禁用规则:」段、知识库 wiki、文件发送链。
- 不改 min_inject_tier 字段语义、`normalize_min_inject_tier` 归一化。
- kind 不可原地改（验证发现的 Minor）本次不修——有删除重建 workaround，且 kind 闭集校验是另一专题。

## 命名红线

新增行（filter 注释、渲染函数、前端「恒注入」徽标、测试文案）不得含 CI 禁词 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。本设计用词均合规。
