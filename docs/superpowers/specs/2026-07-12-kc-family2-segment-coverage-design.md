# KC-05 修复设计：campaign 圈人粗筛口径分裂（serde 默认 vs Mongo 查询）

> 批 C 家族②（P2 独立单条 Medium）。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KC-05（:724-738）。

## 背景与根因（全部主控当场 Read/Grep 亲验，行号基于 origin/main 3042829）

campaign 圈人分两阶段：**粗筛**（Mongo 查询，`build_segment_coarse_filter` campaigns.rs:31-58）→ **精筛**（内存纯函数，`contact_matches_segment` campaigns.rs:61-110，复用 G4 `project_entitlements`）。既定设计是"粗筛 superset ⊇ 精筛"——粗筛宽、精筛严，精筛能捞回粗筛纳入的人，但粗筛漏掉的人精筛永远没机会捞。

带 `product_ids` 的活动，粗筛用 `$elemMatch`（campaigns.rs:50-54）在同一成交事件内匹配：

```rust
"outcome_events": { "$elemMatch": {
    "productRef.productId": { "$in": &filter.product_ids },
    "verification": { "$in": ["staff_confirmed", "payment_verified"] },
    "eventKind": "deal",
}}
```

**口径分裂根因（亲验坐实）**：

1. `verification`（models.rs:451 `#[serde(default = "default_outcome_verification")]` → `staff_confirmed`，:472-474）与 `event_kind`（models.rs:464 `#[serde(default = "default_outcome_kind")]` → `deal`，:468-470）的 serde 默认**只在 Rust 反序列化时补，Mongo 查询时不补**。models.rs:449-450 注释明说"`#[serde(default)]` 只作用于反序列化"。
2. 库中 §4.5（2026-06-15）字段上线前登记的老成交事件，BSON 里根本没有这两个字段——**无迁移回填**（亲验：全部 5 个带 `APP_ENV=production` 守卫的迁移是 m011/m012/m014 破坏性 drop + m016 多租户前置回填；campaigns.rs 无任何 `$exists`/`$or`/`$ne`）。
3. 精筛 `project_entitlements`（agent/entitlements.rs:64）读的是**已反序列化的 Contact**（缺字段已补 `staff_confirmed`/`deal`）→ 精筛认为该客户持有；但粗筛 `$in`/`"deal"` 精确匹配对缺字段落空 → 老客户被粗筛挡在精筛之外。

**后果**：既定"粗筛 superset ⊇ 精筛"被**反转**——粗筛比精筛更严。带 `product_ids` 的定向活动静默漏掉缺字段的老成交客户（本该收到活动推送却收不到），无任何报错。

**次要口径问题（读精筛时发现）**：精筛（entitlements.rs:85）对 `event_kind` **不做排除**——正向 deal 累加件数、reversal 抵消件数，两种都进 fold。粗筛的 `eventKind:"deal"` 精确匹配因此在**第二个维度**上也比精筛严：它要求元素显式等于 `"deal"`，缺字段的老 deal（默认值 deal）落空。修复用 `$ne:"reversal"` 同时解决"缺字段"与"口径对齐精筛"。

## 目标

带 `product_ids` 的活动圈人，缺 `verification`/`event_kind` 字段的老成交客户不再被粗筛静默漏掉，恢复"粗筛 ⊇ 精筛"。两条独立防线，各治一半：查询侧对齐（治标、即时生效、不依赖生产迁移）+ 迁移回填（治本、清历史、消除长期口径分裂）。

## 架构：两条独立防线

### 防线 A —— 粗筛查询侧对齐（治标 + 即时）

`build_segment_coarse_filter`（campaigns.rs:31-58）的 `$elemMatch` 把两个精确匹配改成"缺字段 = 默认值"的显式表达：

```rust
"outcome_events": { "$elemMatch": {
    "productRef.productId": { "$in": &filter.product_ids },
    "$and": [
        { "$or": [
            { "verification": { "$in": ["staff_confirmed", "payment_verified"] } },
            { "verification": { "$exists": false } },
        ]},
        { "eventKind": { "$ne": "reversal" } },
    ],
}}
```

- **`verification`**：白名单命中 **或** 字段缺失（老文档）都算高可信——与 serde 默认 `staff_confirmed` 对齐。`$elemMatch` 内多个键是隐式 AND，字段级"或缺失"不能用顶层 `$or`，故用 `$and` 数组包裹两个子条件。
- **`eventKind`**：`$ne:"reversal"` 一箭双雕——缺字段（Mongo 中 missing ≠ "reversal" 判真）与显式 `"deal"` 都命中，只排除真正的退款事件。同时修正原 `eventKind:"deal"` 比精筛更严的次要口径问题（精筛不按 kind 排除、只对 reversal 抵消件数）。当前模型 `event_kind` 只有 `deal`/`reversal` 两值（models.rs:459）。
- **`productRef.productId`** 保持不变。缺 productRef 的成交本就不该进 product 定向（无产品语义），语义正确。

即时生效，不依赖迁移是否在生产跑。

### 防线 B —— 迁移回填（治本 + 清历史）

新增 `m030_backfill_outcome_event_defaults`，把库中所有 `outcomeEvents`（含 legacy alias `deal_events`，models.rs:248）数组元素中缺 `verification`/`eventKind` 的补上 serde 默认值。彻底消除长期口径分裂——以后任何新查询点都不用记得带 `$exists` 兜底。

**关键设计决策：不加 `APP_ENV=production` 守卫。** 亲验：只有破坏性 drop（m011/m012/m014）+ m016（多租户前置回填，有特定"过早回填致误黑"危害）带该守卫；三个语义保持型回填 m018/m022/m025 **零守卫、生产照跑**。本回填写的就是 serde 读时本已假设的默认值（`staff_confirmed`/`deal`），非破坏、幂等，属 m018 那一类。若误加守卫，会在 117 生产（`APP_ENV` 疑似 production）静默 SKIP，使防线 B 名存实亡——这正是要避开的坑。

迁移管道（纯函数化，对齐 m018 `build_backfill_pipeline`/`backfill_filter` 可单测范式）：

```rust
// 对单个数组字段名构造回填 pipeline 段
fn backfill_array(field: &str) -> Document {
    doc! { "$set": { field: {
        "$map": {
            "input": format!("${field}"),
            "as": "ev",
            "in": { "$mergeObjects": [
                { "verification": "staff_confirmed", "eventKind": "deal" }, // 底：默认值
                "$$ev",  // 覆盖：元素已有键胜出
            ]},
        }
    }}}
}
```

`$mergeObjects` 默认值在底、`$$ev` 在上——**元素已有键覆盖默认值，只补缺失键**。已有 `verification:"conversation_inferred"` / `eventKind:"reversal"` 的元素原值胜出不被改。命中 filter：

```rust
doc! { "$or": [
    { "outcomeEvents": { "$exists": true } },
    { "deal_events":   { "$exists": true } },
]}
```

两个数组字段各跑一次 `update_many` pipeline。幂等：二次跑元素已有两键、`$mergeObjects` 结果不变、`modified_count` → 0。

## 存储键约定（亲验）

- `Contact` struct `#[serde(rename_all = "camelCase")]`（models.rs:148）→ `outcome_events` 存储键 = `outcomeEvents`；`#[serde(alias = "deal_events")]`（:248）→ 极老文档可能用 `deal_events`。
- `OutcomeEvent` 也是 camelCase → `event_kind` → `eventKind`；`verification` 无变化。粗筛现有查询（campaigns.rs:51-53）用的正是 `verification`/`eventKind`/`productRef.productId`，与 camelCase 存储一致。

## 回归风险

1. **防线 A 会不会误纳人？** `$ne:"reversal"` 比旧 `"deal"` 宽——但只多纳"缺 eventKind 字段"（= 老 deal，本就该纳）。不会误纳退款事件。精筛 `contact_matches_segment`（净持有 > 0）是第二道闸兜底，粗筛放宽被精筛收窄——恢复"粗筛 ⊇ 精筛"。
2. **迁移会不会改坏已有值？** `$mergeObjects` 默认在底、`$$ev` 在上，已有值胜出，只补缺失键。幂等。
3. **legacy `deal_events` 别名**：两个数组字段独立回填，互不干扰（alias 二选一，同时存在几无可能）。
4. **既有单测断言**：campaigns.rs:815+ 的 `segment_coarse_filter_*` 单测断言了旧 `eventKind:"deal"` 精确值——**这是被本修复废除的旧断言**（同批 B/批 C① 教训：查询口径修复必扫既有测试是否断言了被废除的旧行为）。需同步更新到新构造。

## 改动面

- **Modify** `src/routes/campaigns.rs:build_segment_coarse_filter`（:31-58）：只改 `$elemMatch` 内部构造，函数签名与其它维度（workspace/account/managed/customer_stage）全不动。
- **Modify** `src/routes/campaigns.rs` 既有单测（:815+ `segment_coarse_filter_*`）：更新到新 `$and`/`$or`/`$ne` 构造。
- **Create** `src/db/migrations/m030_backfill_outcome_event_defaults.rs`：`backfill_array` 纯函数 + filter + `pub async fn run_step`。
- **Modify** `src/db/migrations/mod.rs`：`mod m030_*` 声明 + `MIGRATIONS` 追加（id `2026_07_030_backfill_outcome_event_defaults`，排 m029 之后，满足 chronological 单测）。
- **Create** 集成测 `tests/campaign_segment_coverage.rs`：seed 缺字段老成交 → 跑 m030 → 断言补齐 + 粗筛端到端纳入。

## 测试计划

- **单测（lib，本地可跑）**：
  - `build_segment_coarse_filter` 新构造断言：`$elemMatch` 含 `$and`→`[$or(verification $in / $exists:false), eventKind $ne reversal]` 结构；空 `product_ids` 仍不加 outcome_events 条件（既有断言保留）。
  - 迁移 `backfill_array` 纯函数：缺键补 staff_confirmed/deal；已有 `conversation_inferred`/`reversal` 不被覆盖；幂等（二次结果等价）。
  - 迁移 filter：`$or` 命中 outcomeEvents / deal_events 任一存在。
- **集成测（`#[ignore]` CI Docker）**：
  - seed 一条 `outcomeEvents` 缺 `verification`/`eventKind` 的老成交 contact（managed、买过 product X）→ 直接调 `m030::run_step` → 断言两键补齐为 staff_confirmed/deal。
  - 再走 `resolve_segment_contacts`（或 `build_segment_coarse_filter` + 查询）断言该老客户被纳入粗筛（端到端复现 KC-05 假阴修复：回退防线 A 或防线 B 任一即变红）。

## 非目标（YAGNI）

- 不动精筛 `contact_matches_segment` / `project_entitlements`（口径正确，是被对齐的基准）。
- 不做 KC-06（targetCount 三义命名）/ KC-07（受众规模无上限）——独立 Low，本 PR 不含。
- 不删 `outcome_events` 顶层老字段 / 不改 serde 默认定义。
- 不碰 customer_stage 前端传值口径核（台账未确认项，独立）。
