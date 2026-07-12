# KC-05 圈人粗筛口径分裂修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 带 product_ids 的 campaign 圈人不再静默漏掉缺 verification/event_kind 字段的老成交客户（恢复"粗筛 ⊇ 精筛"）。

**Architecture:** 两条独立防线。防线 A：粗筛 `$elemMatch` 查询把 verification/eventKind 精确匹配改成"缺字段=默认值"显式表达（即时生效、不依赖生产迁移）。防线 B：新增 m030 迁移用 `$map`+`$mergeObjects` 回填库中 outcome_events/deal_events 数组元素缺失的两字段默认值（治本清历史，非破坏幂等，不加 APP_ENV 守卫）。

**Tech Stack:** Rust 2021 / Axum / mongodb bson / testcontainers（集成测）。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-12-kc-family2-segment-coverage-design.md`（已获批 commit a60084f）。所有行号亲验于 origin/main 3042829。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为"，绝不为过测试改业务逻辑/阈值。
- 不动精筛 `contact_matches_segment`/`project_entitlements`（口径正确，是被对齐的基准）。不删 outcome_events 顶层老字段/不改 serde 默认定义。不含 KC-06/KC-07。
- m030 **不加** `APP_ENV=production` 守卫（语义保持型回填，同 m018/m022/m025；写的就是 serde 读时默认值，误加守卫会致 117 生产静默 SKIP）。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不回退。集成测 `#[ignore]`（CI Docker 跑），本地只 `cargo test --lib`。
- 存储键（亲验）：Contact **无** `#[serde(rename_all)]`（models.rs:148）→ 顶层字段存 snake_case `outcome_events`（见 db/indexes.rs:38-40 索引键 + 防线A campaigns.rs 亦用 snake_case），alias `deal_events`（models.rs:248）；内层 OutcomeEvent **带** `rename_all="camelCase"` → `event_kind`→`eventKind`，`verification` 不变。粗筛现有查询用的正是 `outcome_events`（顶层 snake）+ `verification`/`eventKind`/`productRef.productId`（内层 camel）。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。绝不动任何 sibling worktree 的 target/。

---

## File Structure

- `src/routes/campaigns.rs`：**Modify** `build_segment_coarse_filter`（:31-58）`$elemMatch` 构造 + 既有单测 `coarse_filter_with_products_uses_elemmatch_real_keys`（:813-831）。防线 A 全在此文件。
- `src/db/migrations/m030_backfill_outcome_event_defaults.rs`：**Create** 防线 B 迁移（`backfill_array` 纯函数 + `backfill_filter` + `pub async fn run_step` + 纯函数单测）。
- `src/db/migrations/mod.rs`：**Modify** 注册 `mod m030_*` + `MIGRATIONS` 追加。
- `tests/campaign_segment_coverage.rs`：**Create** 集成测（m030 回填语义 + 端到端粗筛纳入）。

---

## Task 1: 防线 A —— 粗筛查询侧对齐（campaigns.rs）

**Files:**
- Modify: `src/routes/campaigns.rs:46-56`（`build_segment_coarse_filter` 的 `$elemMatch` 段）
- Modify: `src/routes/campaigns.rs:813-831`（既有单测 `coarse_filter_with_products_uses_elemmatch_real_keys`）

**Interfaces:**
- Consumes: 无（改既有函数内部构造，签名 `build_segment_coarse_filter(workspace_id: &str, account_id: &str, filter: &SegmentFilter) -> Document` 不变）。
- Produces: 修改后的 `$elemMatch` 文档形态——后续 Task 3 集成测端到端依赖它对缺字段老成交命中。

- [ ] **Step 1: 更新既有单测到新构造（先改断言，验证会失败）**

把 `src/routes/campaigns.rs:813-831` 的 `coarse_filter_with_products_uses_elemmatch_real_keys` 整体替换为：

```rust
    #[test]
    fn coarse_filter_with_products_uses_elemmatch_real_keys() {
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        let d = build_segment_coarse_filter("ws", "acc", &f);
        let em = d.get_document("outcome_events").unwrap();
        let elem = em.get_document("$elemMatch").unwrap();
        // productRef.productId（camelCase 内嵌）仍在
        assert!(elem.contains_key("productRef.productId"));
        // KC-05：verification / eventKind 从精确匹配改成"缺字段=默认值"显式表达，
        // 用 $elemMatch 内的 $and 数组承载（字段级"或缺失"不能用顶层 $or）。
        let and = elem.get_array("$and").unwrap();
        assert_eq!(and.len(), 2, "$and 恰两个子条件：verification 或缺失 + eventKind != reversal");
        // 子条件 1：verification $in 白名单 OR $exists:false
        let ver = and[0].as_document().unwrap();
        let ver_or = ver.get_array("$or").unwrap();
        assert_eq!(ver_or.len(), 2, "verification: $in 白名单 或 $exists:false");
        // 子条件 2：eventKind $ne reversal（缺字段天然命中，与精筛口径对齐）
        let kind = and[1].as_document().unwrap();
        let kind_ne = kind.get_document("eventKind").unwrap();
        assert_eq!(kind_ne.get_str("$ne").unwrap(), "reversal");
        // 始终带租户隔离
        assert_eq!(d.get_str("workspace_id").unwrap(), "ws");
        assert_eq!(d.get_str("account_id").unwrap(), "acc");
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }
```

- [ ] **Step 2: 运行单测确认失败（旧构造不产生 $and）**

Run: `cargo test --lib coarse_filter_with_products_uses_elemmatch_real_keys 2>&1 | tail -20`
Expected: FAIL —— `elem.get_array("$and").unwrap()` panic（旧构造无 `$and`，只有 `verification`/`eventKind` 平铺键）。

- [ ] **Step 3: 改 build_segment_coarse_filter 的 $elemMatch 构造**

把 `src/routes/campaigns.rs:46-56` 的 product 反查段：

```rust
    // 产品反查：$elemMatch 同一成交事件内匹配「指定产品 + 高可信 + 正向」。
    if !filter.product_ids.is_empty() {
        d.insert(
            "outcome_events",
            doc! { "$elemMatch": {
                "productRef.productId": { "$in": &filter.product_ids },
                "verification": { "$in": ["staff_confirmed", "payment_verified"] },
                "eventKind": "deal",
            }},
        );
    }
```

替换为：

```rust
    // 产品反查：$elemMatch 同一成交事件内匹配「指定产品 + 高可信 + 非退款」。
    // KC-05：verification/eventKind 的 serde 默认(staff_confirmed/deal)只在反序列化补、
    // Mongo 查询不补，缺这两字段的老成交(§4.5 上线前登记)会被精确匹配漏掉→product 定向
    // 静默漏老客户。故把"缺字段=默认值"显式写进查询：
    // - verification：白名单命中 或 字段缺失(老文档=staff_confirmed)。$elemMatch 内多键是
    //   隐式 AND，字段级"或缺失"须用 $and 包裹(顶层 $or 不能做字段级)。
    // - eventKind：$ne:"reversal" 一箭双雕——缺字段(missing ≠ reversal)与显式"deal"都命中，
    //   只排退款；同时与精筛口径对齐(精筛不按 kind 排除、只对 reversal 抵消件数)。
    if !filter.product_ids.is_empty() {
        d.insert(
            "outcome_events",
            doc! { "$elemMatch": {
                "productRef.productId": { "$in": &filter.product_ids },
                "$and": [
                    { "$or": [
                        { "verification": { "$in": ["staff_confirmed", "payment_verified"] } },
                        { "verification": { "$exists": false } },
                    ]},
                    { "eventKind": { "$ne": "reversal" } },
                ],
            }},
        );
    }
```

- [ ] **Step 4: 运行单测确认通过**

Run: `cargo test --lib coarse_filter 2>&1 | tail -20`
Expected: PASS —— `coarse_filter_with_products_uses_elemmatch_real_keys` 和 `coarse_filter_empty_products_skips_outcome_condition` 都绿（后者不受影响：空 product_ids 仍不加 outcome_events 条件）。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/routes/campaigns.rs
git commit -m "fix(campaigns): 粗筛 verification/eventKind 缺字段=默认值口径对齐 (KC-05 防线A)"
```

---

## Task 2: 防线 B —— m030 回填迁移（新增文件 + 注册）

**Files:**
- Create: `src/db/migrations/m030_backfill_outcome_event_defaults.rs`
- Modify: `src/db/migrations/mod.rs:70`（在 `pub mod m029_*` 后加 `mod m030_*`）
- Modify: `src/db/migrations/mod.rs:195-198`（`MIGRATIONS` 数组末尾追加条目）

**Interfaces:**
- Consumes: `Database`（`crate::db::Database`），`AppResult`（`crate::error::AppResult`）。
- Produces: `pub async fn run_step(db: &Database) -> AppResult<()>`（`pub` 供 Task 3 集成测直接调用，同 m018/m029 先例）；`pub(super) fn backfill_array(field: &str) -> Document`；`pub(super) fn backfill_filter() -> Document`。

- [ ] **Step 1: 新建 m030 文件（含纯函数单测——先写会失败的单测形态）**

Create `src/db/migrations/m030_backfill_outcome_event_defaults.rs`：

```rust
//! 2026_07_030：回填 outcome_events 数组元素缺失的 verification/event_kind 默认值。
//!
//! 背景(KC-05)：`OutcomeEvent.verification`(models.rs:451 default→staff_confirmed) 与
//! `event_kind`(models.rs:464 default→deal) 的 `#[serde(default)]` 只作用于反序列化、
//! Mongo 查询不补。§4.5(2026-06-15)字段上线前登记的老成交事件 BSON 里没这两键，
//! campaign 圈人粗筛 `$elemMatch` 精确匹配(campaigns.rs)对缺字段落空 → product 定向
//! 活动静默漏老客户。防线 A(查询侧 $exists/$ne 对齐)已即时止血；本迁移治本清历史，
//! 彻底消除 serde 默认与 Mongo 查询的长期口径分裂。
//!
//! **不加 APP_ENV=production 守卫**：本回填写的就是 serde 读时本已假设的默认值
//! (staff_confirmed/deal)，语义保持、非破坏、幂等——与 m018/m022/m025 同类(它们均无
//! 守卫、生产照跑)。带守卫的 m011/m012/m014 是破坏性 drop、m016 是多租户前置回填(有
//! "过早回填致误黑"特定危害)，均与本迁移性质不同。误加守卫会致 117 生产静默 SKIP、
//! 防线 B 名存实亡。
//!
//! **兼容 legacy alias**：Contact.outcome_events serde alias="deal_events"(models.rs:248)，
//! 故极老文档数组键可能是 `deal_events`；两个键各回填一次。
//!
//! 合并策略：`$map` 遍历数组，每元素 `$mergeObjects([默认值底, $$ev])` —— 默认值在底、
//! 元素已有键在上覆盖，**只补缺失键**，已有 conversation_inferred/reversal 原值胜出不改。
//!
//! 幂等：二次执行元素已有两键、mergeObjects 结果不变、modified_count → 0。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 对单个数组字段名构造回填 pipeline 段(纯函数,便于单测)。
/// `$map` 遍历 `$field`,每元素合并"默认值底 + 元素本身",元素已有键覆盖默认值。
pub(super) fn backfill_array(field: &str) -> Document {
    doc! { "$set": { field: {
        "$map": {
            "input": { "$ifNull": [format!("${field}"), []] },
            "as": "ev",
            "in": { "$mergeObjects": [
                { "verification": "staff_confirmed", "eventKind": "deal" },
                "$$ev",
            ]},
        }
    }}}
}

/// 命中过滤器:两个数组字段任一存在即需回填(纯函数,便于单测)。
pub(super) fn backfill_filter() -> Document {
    doc! {
        "$or": [
            { "outcome_events": { "$exists": true } },
            { "deal_events": { "$exists": true } },
        ]
    }
}

/// 迁移主体。`pub` 暴露给 `tests/` 集成测试(同 m018/m029 先例)。
/// 对 outcome_events 与 legacy deal_events 各跑一次 update_many pipeline。
pub async fn run_step(db: &Database) -> AppResult<()> {
    for field in ["outcome_events", "deal_events"] {
        let result = db
            .contacts()
            .update_many(backfill_filter(), vec![backfill_array(field)], None)
            .await?;
        tracing::info!(
            migration_id = "2026_07_030_backfill_outcome_event_defaults",
            field = field,
            modified = result.modified_count,
            matched = result.matched_count,
            "backfilled missing verification/eventKind defaults into outcome_events elements"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_array_maps_with_defaults_as_base_and_element_on_top() {
        let stage = backfill_array("outcome_events");
        let set = stage.get_document("$set").unwrap();
        let field = set.get_document("outcome_events").unwrap();
        let map = field.get_document("$map").unwrap();
        // $map 遍历该字段(用 $ifNull 兜空)
        assert!(map.contains_key("input"));
        assert_eq!(map.get_str("as").unwrap(), "ev");
        // in 是 $mergeObjects([默认值底, $$ev])——默认在前(底)、元素在后(覆盖)
        let merge = map.get_document("in").unwrap().get_array("$mergeObjects").unwrap();
        assert_eq!(merge.len(), 2);
        let base = merge[0].as_document().unwrap();
        assert_eq!(base.get_str("verification").unwrap(), "staff_confirmed");
        assert_eq!(base.get_str("eventKind").unwrap(), "deal");
        assert_eq!(merge[1].as_str().unwrap(), "$$ev", "元素本身须在末位覆盖默认值(只补缺失键)");
    }

    #[test]
    fn backfill_filter_matches_either_array_key() {
        let filter = backfill_filter();
        let or = filter.get_array("$or").unwrap();
        assert_eq!(or.len(), 2);
        let keys: Vec<String> = or
            .iter()
            .filter_map(|b| b.as_document())
            .flat_map(|d| d.keys().cloned().collect::<Vec<_>>())
            .collect();
        assert!(keys.contains(&"outcome_events".to_string()), "须命中 camelCase outcome_events");
        assert!(keys.contains(&"deal_events".to_string()), "须命中 legacy alias deal_events");
    }
}
```

- [ ] **Step 2: 注册 mod 声明**

在 `src/db/migrations/mod.rs:70`（`pub mod m029_cleanup_contact_identity;` 之后）加一行：

```rust
mod m030_backfill_outcome_event_defaults;
```

- [ ] **Step 3: 注册进 MIGRATIONS 数组**

在 `src/db/migrations/mod.rs:198`（m029 条目 `}` 之后、数组闭合 `];` 之前）追加：

```rust
    Migration {
        id: "2026_07_030_backfill_outcome_event_defaults",
        run: |db| Box::pin(m030_backfill_outcome_event_defaults::run_step(db)),
    },
```

- [ ] **Step 4: 运行纯函数单测 + 迁移 id 顺序单测**

Run: `cargo test --lib m030 2>&1 | tail -15 && cargo test --lib migration_ids 2>&1 | tail -8`
Expected: `backfill_array_*` / `backfill_filter_*` PASS；`migration_ids_are_unique` 与 `migration_ids_are_chronologically_ordered` PASS（`2026_07_029...` < `2026_07_030...` 字符串序成立）。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/db/migrations/m030_backfill_outcome_event_defaults.rs src/db/migrations/mod.rs
git commit -m "feat(migration): m030 回填 outcome_events 缺失 verification/eventKind 默认值 (KC-05 防线B)"
```

---

## Task 3: 集成测 —— 端到端复现 KC-05 修复

**Files:**
- Create: `tests/campaign_segment_coverage.rs`

**Interfaces:**
- Consumes: `m030_backfill_outcome_event_defaults::run_step`（Task 2 的 `pub` 入口）；`build_segment_coarse_filter`（Task 1，`pub(super)`——集成测在 crate 外，须走公开路径，见 Step 1 说明）；`common::TestApp`。
- Produces: 无（终端测试）。

**说明（实现者先读）**：`build_segment_coarse_filter` 是 `pub(super)`，集成测（独立 crate）**不可**直接调用。故集成测用两段验证：(a) 直接调 `m030::run_step` 验回填落库语义（这是 `pub`）；(b) 手工构造与防线 A 等价的粗筛查询文档、跑 `contacts().find()` 验缺字段老成交被纳入。若发现 `build_segment_coarse_filter` 确需跨 crate 暴露，**不要**擅自改可见性——先记为 finding 报主控裁决（改可见性属超范围）。

- [ ] **Step 1: 写集成测文件**

Create `tests/campaign_segment_coverage.rs`：

```rust
//! KC-05 端到端：缺 verification/eventKind 的老成交客户，回填后 + 粗筛口径对齐后被纳入。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m030_backfill_outcome_event_defaults;

use crate::common::TestApp;

/// 直接插一条 outcome_events 缺 verification/eventKind 的"老成交"contact（raw Document
/// 绕过 serde 默认，模拟 §4.5 上线前的 BSON 形态），跑 m030 后两键补齐为默认值。
#[tokio::test]
#[ignore]
async fn m030_backfills_missing_verification_and_event_kind() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    // raw insert：outcome_events 元素只有 productRef，无 verification/eventKind。
    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "old_buyer",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed legacy contact");

    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030");

    let after = raw
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": "old_buyer", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("contact exists");
    let ev = after.get_array("outcome_events").unwrap()[0].as_document().unwrap();
    assert_eq!(ev.get_str("verification").unwrap(), "staff_confirmed", "缺 verification 补默认");
    assert_eq!(ev.get_str("eventKind").unwrap(), "deal", "缺 eventKind 补默认");
    // productRef 原值不被破坏
    assert_eq!(
        ev.get_document("productRef").unwrap().get_str("productId").unwrap(),
        "vip"
    );
}

/// m030 幂等：已有 conversation_inferred/reversal 的元素原值不被默认值覆盖。
#[tokio::test]
#[ignore]
async fn m030_does_not_overwrite_existing_values() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "explicit_buyer",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "verification": "conversation_inferred",
                    "eventKind": "reversal",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed");

    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030 once");
    // 再跑一次验幂等
    m030_backfill_outcome_event_defaults::run_step(&app.state.db)
        .await
        .expect("run m030 twice");

    let after = raw
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": "explicit_buyer", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    let ev = after.get_array("outcome_events").unwrap()[0].as_document().unwrap();
    assert_eq!(ev.get_str("verification").unwrap(), "conversation_inferred", "已有值不被覆盖");
    assert_eq!(ev.get_str("eventKind").unwrap(), "reversal", "已有 reversal 不被改成 deal");
}

/// 端到端：缺字段老成交客户，用防线 A 等价的粗筛查询能命中(回填前靠 $exists/$ne 就纳入)。
/// 手工构造与 build_segment_coarse_filter 等价的 $elemMatch(集成测在 crate 外不可直调
/// pub(super) 函数)，验证缺字段老成交被粗筛纳入。
#[tokio::test]
#[ignore]
async fn coarse_query_includes_legacy_event_missing_fields() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();

    raw.collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "wxid": "legacy_vip",
                "agent_status": "managed",
                "outcome_events": [ {
                    "markedAt": mongodb::bson::DateTime::from_millis(0),
                    "source": "manual",
                    "productRef": { "productId": "vip", "name": "P", "quantity": 1 },
                } ],
            },
            None,
        )
        .await
        .expect("seed legacy");

    // 与 build_segment_coarse_filter 防线 A 等价的粗筛(product_ids=["vip"])。
    let coarse = doc! {
        "workspace_id": &ws,
        "account_id": "acc",
        "agent_status": "managed",
        "outcome_events": { "$elemMatch": {
            "productRef.productId": { "$in": ["vip"] },
            "$and": [
                { "$or": [
                    { "verification": { "$in": ["staff_confirmed", "payment_verified"] } },
                    { "verification": { "$exists": false } },
                ]},
                { "eventKind": { "$ne": "reversal" } },
            ],
        }},
    };
    let count = raw
        .collection::<Document>("contacts")
        .count_documents(coarse, None)
        .await
        .expect("count");
    assert_eq!(count, 1, "缺 verification/eventKind 的老成交老客户须被粗筛纳入(KC-05 修复)");
}
```

- [ ] **Step 2: 编译集成测（不跑，本地无 Docker）**

Run: `cargo test --test campaign_segment_coverage --no-run 2>&1 | tail -8`
Expected: `Finished` / `Executable ...campaign_segment_coverage-*.exe`（0 编译错误）。

- [ ] **Step 3: 确认 db::migrations 模块路径可达**

验证 `wechatagent::db::migrations::m030_backfill_outcome_event_defaults` 可跨 crate 引用。若 `db` 或 `migrations` 非 `pub`、或 m030 的 `mod` 声明非 `pub`（Task 2 Step 2 用的是 `mod`，不是 `pub mod`）导致 E0603：把 Task 2 Step 2 的声明改为 `pub mod m030_backfill_outcome_event_defaults;`（同 m018/m029 先例：它们为集成测暴露而用 `pub mod`）。

Run: `cargo test --test campaign_segment_coverage --no-run 2>&1 | grep -E "E0603|error" | head`
Expected: 空（无可见性错误）。若有 E0603，按上句改 `pub mod` 后重编。

- [ ] **Step 4: Commit**

```bash
git add tests/campaign_segment_coverage.rs src/db/migrations/mod.rs
git commit -m "test: KC-05 端到端(m030回填语义+幂等+缺字段老成交粗筛纳入)"
```

---

## Self-Review

**1. Spec coverage：**
- 防线 A（查询 $or/$exists + $ne:reversal）→ Task 1 ✓
- 防线 B（m030 $map+$mergeObjects 回填、不加 APP_ENV 守卫、兼容 deal_events alias）→ Task 2 ✓
- 既有单测旧断言更新（:826 eventKind:"deal"）→ Task 1 Step 1 ✓
- 迁移注册 + id 顺序 → Task 2 Step 2/3/4 ✓
- 集成测端到端复现 → Task 3 ✓
- 存储键 camelCase/alias → 全任务已用 `outcome_events`/`eventKind`/`deal_events` ✓

**2. Placeholder scan：** 无 TBD/TODO；每个 code step 都有完整可编译代码。

**3. Type consistency：**
- `run_step(db: &Database) -> AppResult<()>` — Task 2 定义、Task 3 消费，签名一致 ✓
- `backfill_array(field: &str) -> Document` / `backfill_filter() -> Document` — Task 2 定义并单测 ✓
- `run_step` 用 `vec![backfill_array(field)]` 传 pipeline（update_many 第二参收 pipeline，同 m018 传 `build_backfill_pipeline()` 返回的 `Vec<Document>`）✓
- 可见性风险已在 Task 3 Step 3 显式处理（E0603 → pub mod）✓
