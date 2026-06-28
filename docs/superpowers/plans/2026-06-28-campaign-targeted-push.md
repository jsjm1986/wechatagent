# 活动定向推送 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按购买的产品（及售后/价值/阶段维度）圈定客户人群，经人工确认后向该人群批量创建 follow_up 任务，复用现有发送链路定向推送活动信息。

**Architecture:** 新建 `campaigns` 引擎（数据模型 + segment 两阶段查询 + 生命周期 create/preview/confirm_dispatch + REST 路由），总控 AI（management agent）两个工具委托引擎 handler。扇出只是批量自建 `kind="follow_up"` AgentTask，发送链路（task worker → gateway → 独立 review → 产品红线 → outbox → MCP）一字不改。

**Tech Stack:** Rust 2021 + Axum + MongoDB（mongodb crate）；前端本期不做。

**上游 spec:** `docs/superpowers/specs/2026-06-28-campaign-targeted-push-design.md`

## Global Constraints

- **真实 BSON key 混合大小写**：`Contact` 无 `rename_all`（snake_case），但 `OutcomeEvent`/`OutcomeProductRef` 带 `#[serde(rename_all="camelCase")]`。成交事件内嵌字段真实路径 = `outcome_events.productRef.productId`、`verification`、`eventKind`（**不是** `product_ref.product_id`）。索引与 `$elemMatch` 查询必须用此真实路径，否则建在空字段上、查询恒空。
- **只认高可信成交**：segment 圈人只认 `verification ∈ {staff_confirmed, payment_verified}` + `eventKind="deal"`（G4 红线，`conversation_inferred` 绝不进）。
- **dispatch 确认门必须用 `tool_always_requires_confirmation`**：`post_management_message` 硬编码 `dangerous_confirm_enabled=false`，仅定 `Dangerous` 档**不会**触发确认门。dispatch_campaign 必须加进 `tool_always_requires_confirmation`（management.rs:1263）才恒确认。
- **引擎自建 AgentTask，不调 planner**：`emit_planner_follow_up`（planner/mod.rs:198）是私有 `async fn` 跨模块不可调；照 management `create_follow_up_task`（management.rs:1461）自建 AgentTask。
- **AgentTask 闭集不动**：不新增 AgentTask 字段；活动关联放 `campaign_sends.task_id`。
- **status 闭集校验**：所有 `campaigns.status` 写入前过 `assert_campaign_status_valid`（仿 `assert_agent_task_status_valid` models.rs:766）。
- **多租户隔离（IDOR 红线）**：所有 Mongo filter 必含 `workspace_id`；写入 `workspace_id` 由 admin 会话注入，绝不信前端请求体。management 委托用 `management_admin(workspace_id)`（management.rs:2323）注入可信 workspace。
- **命名红线（check-no-human-takeover lint）**：新增行不得含 `人工|接管|takeover|hand-off|人工介入` 等禁词；status/文案用 AI 中性词。lint 扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 新增行。
- **金额整数化**：金额字段一律 `Option<i64>`（分），不用 f64。
- **测试基线门（不可回归）**：`cargo test --lib` ≥ 350 passed/0 failed；4 个 PBT 累计 ≥ 33/0。新工作只增量加测试。
- **本地编译纪律**：导出 `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`；本地只跑 `cargo test --lib` 和单个 PBT，全量集成测试留 CI。
- **YAGNI（本期不做）**：前端频道/Tab、定时活动、任意布尔表达式引擎、效果统计仪表盘、支付闭环。

---

### Task 1: 数据模型（Campaign / CampaignSend / SegmentFilter）+ status 闭集

**Files:**
- Modify: `src/models.rs`（在 `Product`(:499) 之后、约 :547 现有自由函数前插入新结构体；status 闭集仿 `ALLOWED_AGENT_TASK_STATUS` :752 模式追加）

**Interfaces:**
- Produces:
  - `pub struct Campaign { id, workspace_id, account_id, title, intent_text, segment_filter: SegmentFilter, status, target_count: Option<i64>, dispatched_count: i64, created_by, created_at, updated_at }`
  - `pub struct CampaignSend { id, workspace_id, account_id, campaign_id: ObjectId, contact_wxid, task_id: Option<ObjectId>, status, created_at }`
  - `pub struct SegmentFilter { product_ids: Vec<String>, aftercare: Option<String>, value_tier: Option<String>, customer_stage: Option<String> }`
  - `pub const ALLOWED_CAMPAIGN_STATUS: &[&str]`
  - `pub fn assert_campaign_status_valid(status: &str)`

- [ ] **Step 1: 写失败测试**（追加到 `src/models.rs` 末尾 `#[cfg(test)]` 区，新建一个 `mod campaign_model_tests`）

```rust
#[cfg(test)]
mod campaign_model_tests {
    use super::*;

    #[test]
    fn campaign_status_closed_set_covers_lifecycle() {
        for s in ["draft", "previewed", "confirmed", "dispatching", "completed", "canceled"] {
            assert!(ALLOWED_CAMPAIGN_STATUS.contains(&s), "缺少状态 {s}");
        }
        assert_eq!(ALLOWED_CAMPAIGN_STATUS.len(), 6);
    }

    #[test]
    fn assert_campaign_status_accepts_valid_rejects_unknown() {
        // 合法值不 panic
        for s in ["draft", "previewed", "confirmed", "dispatching", "completed", "canceled"] {
            assert_campaign_status_valid(s);
        }
        // 闭集外值在 debug 下 panic（用 catch_unwind 验证）
        let r = std::panic::catch_unwind(|| assert_campaign_status_valid("bogus"));
        assert!(r.is_err(), "闭集外值应触发 debug_assert panic");
    }

    #[test]
    fn segment_filter_serializes_camelcase_and_defaults_empty() {
        // SegmentFilter 默认全空 = 不约束任何维度
        let f = SegmentFilter::default();
        assert!(f.product_ids.is_empty());
        assert!(f.aftercare.is_none());
        assert!(f.value_tier.is_none());
        assert!(f.customer_stage.is_none());
        // camelCase 序列化（前端/JSON 契约）
        let v = serde_json::to_value(&f).unwrap();
        assert!(v.get("productIds").is_some(), "应序列化为 productIds");
    }

    #[test]
    fn campaign_send_roundtrips_bson() {
        // CampaignSend 能 BSON 往返（落库可行）
        let cs = CampaignSend {
            id: None,
            workspace_id: "ws".into(),
            account_id: "acc".into(),
            campaign_id: ObjectId::new(),
            contact_wxid: "wx1".into(),
            task_id: None,
            status: "enqueued".into(),
            created_at: DateTime::now(),
        };
        let doc = mongodb::bson::to_document(&cs).unwrap();
        let back: CampaignSend = mongodb::bson::from_document(doc).unwrap();
        assert_eq!(back.contact_wxid, "wx1");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib campaign_model_tests
```
Expected: 编译失败（`Campaign`/`SegmentFilter`/`ALLOWED_CAMPAIGN_STATUS` 未定义）。

- [ ] **Step 3: 实现结构体 + 闭集**（插入 `src/models.rs` `Product`(:528 结构体结束) 之后、`fn default_product_status`(:530) 之前，或紧邻其后；与现有结构体同区即可）

```rust
/// 活动定向推送：活动实体（workspace+account 级）。圈出买过指定产品的客户，
/// 经人工确认后批量建 follow_up 任务定向推送活动信息。
/// 字段 camelCase 落库（与 OutcomeEvent 同约定，前端 JSON 契约一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub title: String,
    /// 活动意图要点（注入 follow_up content，喂 Reply Agent 生成个性化话术）。
    pub intent_text: String,
    #[serde(default)]
    pub segment_filter: SegmentFilter,
    /// draft / previewed / confirmed / dispatching / completed / canceled（闭集，
    /// 见 [`ALLOWED_CAMPAIGN_STATUS`]）。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_count: Option<i64>,
    #[serde(default)]
    pub dispatched_count: i64,
    pub created_by: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// 活动人群圈选条件。各维 AND；空 = 不约束该维。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentFilter {
    /// 买过其中任一产品（$in 取并集）；空 = 不限产品。
    #[serde(default)]
    pub product_ids: Vec<String>,
    /// 售后状态过滤：`in_aftercare` / `expired` / `any`；None = 不限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aftercare: Option<String>,
    /// 价值分层：`high` / `mid` / `low`；None = 不限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_tier: Option<String>,
    /// 客户阶段（走 system_taxonomies 字典）；None = 不限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_stage: Option<String>,
}

/// 活动每人推送台账。唯一索引 (campaign_id, contact_wxid) = 活动级去重闸：
/// 同一活动对同一人只推一次（仿 outbox idempotency_key 幂等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignSend {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub campaign_id: ObjectId,
    pub contact_wxid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<ObjectId>,
    /// `enqueued`（已建 follow_up 任务）/ `skipped_duplicate`（去重命中）。
    pub status: String,
    pub created_at: DateTime,
}

/// `campaigns.status` 封闭枚举。所有写入路径在 `$set: { status: ... }` 前
/// 必须经 [`assert_campaign_status_valid`] 校验。
pub const ALLOWED_CAMPAIGN_STATUS: &[&str] = &[
    "draft",
    "previewed",
    "confirmed",
    "dispatching",
    "completed",
    "canceled",
];

/// 任意 `campaigns.status` 写入站点的闭集断言。命中闭集外值 debug panic /
/// release 下 `tracing::error!` + 拒绝。仿 [`assert_agent_task_status_valid`]。
#[track_caller]
pub fn assert_campaign_status_valid(status: &str) {
    if !ALLOWED_CAMPAIGN_STATUS.contains(&status) {
        let msg = format!(
            "campaigns.status='{status}' 不在 ALLOWED_CAMPAIGN_STATUS 闭集 {ALLOWED_CAMPAIGN_STATUS:?}"
        );
        debug_assert!(false, "{msg}");
        tracing::error!(target: "agent_protocol_violation", "{msg}");
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

```
cargo test --lib campaign_model_tests
```
Expected: 4 个测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs
git commit -m "feat(campaign): Campaign/CampaignSend/SegmentFilter 数据模型 + status 闭集"
```

---

### Task 2: db 访问器 + 索引

**Files:**
- Modify: `src/db/mod.rs:383`（在 `products()` 访问器后、`impl` 块结束 `}`(:384) 前加两个访问器）
- Modify: `src/db/indexes.rs`（新增 `ensure_campaigns_indexes` + contacts 加 outcome_events 索引；在 `ensure_products_indexes`(:696) 同区加新函数，并在 `ensure_indexes` 主函数里 :653 附近调用）

**Interfaces:**
- Consumes: `Campaign` / `CampaignSend`（Task 1）
- Produces: `db.campaigns() -> Collection<Campaign>`、`db.campaign_sends() -> Collection<CampaignSend>`

- [ ] **Step 1: 加 db 访问器**（`src/db/mod.rs`，`products()`(:381-383) 之后、`}`(:384) 之前）

```rust
    /// 活动定向推送：活动实体集合。
    pub fn campaigns(&self) -> Collection<crate::models::Campaign> {
        self.db.collection("campaigns")
    }

    /// 活动每人推送台账（去重 + 送达追踪）。
    pub fn campaign_sends(&self) -> Collection<crate::models::CampaignSend> {
        self.db.collection("campaign_sends")
    }
```

（确认 `src/db/mod.rs` 顶部已 `use crate::models::...` 或用全路径 `crate::models::Campaign`；现有 `products()` 用的是裸 `Product`，说明顶部已 import models 类型——照现有 import 风格加 `Campaign, CampaignSend` 到 use 列表，或保持全路径。检查文件顶部 use 段决定。）

- [ ] **Step 2: 加索引函数**（`src/db/indexes.rs`，`ensure_products_indexes`(:715 结束) 之后）

```rust
/// 活动定向推送索引。
/// - campaigns `(workspace_id, account_id, status)`：按状态列活动。
/// - campaign_sends `(campaign_id, contact_wxid)` unique：活动级去重闸
///   （同一活动对同一人只推一次，仿 outbox idempotency_key）。
async fn ensure_campaigns_indexes(db: &Database) -> anyhow::Result<()> {
    db.campaigns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    db.campaign_sends()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "campaign_id": 1, "contact_wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    Ok(())
}
```

- [ ] **Step 3: contacts 加 outcome_events 反查索引**（在 `ensure_indexes` 主函数里 contacts 索引段 :29-37 之后追加一条）

```rust
    // 活动定向推送：按购买产品反查客户。真实 BSON 路径是混合大小写——
    // outcome_events(snake_case，Contact 无 rename_all) + productRef.productId
    // (camelCase，OutcomeEvent/OutcomeProductRef 带 rename_all=camelCase)。
    // outcome_events 是数组 → multikey 索引；$elemMatch 按产品反查命中此索引前缀。
    db.contacts()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "outcome_events.productRef.productId": 1
                })
                .build(),
            None,
        )
        .await?;
```

- [ ] **Step 4: 主函数注册调用**（`src/db/indexes.rs` `ensure_indexes` 里，`ensure_products_indexes(db).await?;`(:653) 后加一行）

```rust
    ensure_campaigns_indexes(db).await?;
```

- [ ] **Step 5: 编译确认**

```
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo check --lib
```
Expected: 编译通过（无 E0599 方法未找到 / 无类型错误）。

- [ ] **Step 6: 提交**

```bash
git add src/db/mod.rs src/db/indexes.rs
git commit -m "feat(campaign): campaigns/campaign_sends 访问器 + 索引(含 outcome_events 反查)"
```

---

### Task 3: segment 两阶段查询（核心引擎，纯可测）

**Files:**
- Create: `src/routes/campaigns.rs`（本任务先建文件 + segment 查询 + 单元测试；Task 4 在同文件加生命周期 handler）
- Modify: `src/routes/mod.rs`（声明 `mod campaigns;`——在 :20-83 的 mod 声明区按字母序加 `mod campaigns;`）

**Interfaces:**
- Consumes: `SegmentFilter`（Task 1）；`agent::entitlements::{project_entitlements, compute_customer_value_cents, classify_value_tier, load_active_products}`（entitlements.rs:64/493/511/229）；`Contact`、`OutcomeEvent`
- Produces:
  - `pub(super) fn build_segment_coarse_filter(workspace_id, account_id, filter: &SegmentFilter) -> Document`（阶段1 Mongo 粗筛 filter）
  - `pub(super) fn contact_matches_segment(contact, active_products, filter, now, mid_threshold, high_threshold) -> bool`（阶段2 内存精筛，纯函数）

- [ ] **Step 1: 写失败测试**（`src/routes/campaigns.rs` 新建，先只放 `#[cfg(test)]` + 两个待测函数签名的 `todo!()` 占位）

```rust
//! 活动定向推送引擎：segment 圈人（两阶段）+ 活动生命周期。
use mongodb::bson::{doc, DateTime, Document};
use crate::models::{Contact, Product, SegmentFilter};
use crate::agent::entitlements;

/// 阶段1：Mongo 粗筛 filter。命中 outcome_events.productRef.productId 索引。
/// product_ids 非空时用 $elemMatch 同元素匹配「买过指定产品 + 高可信 + 正向成交」。
pub(super) fn build_segment_coarse_filter(
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> Document {
    todo!()
}

/// 阶段2：内存精筛。复用 G4 纯函数判净持有/售后/价值分层。
pub(super) fn contact_matches_segment(
    contact: &Contact,
    active_products: &[Product],
    filter: &SegmentFilter,
    now: DateTime,
    mid_threshold: i64,
    high_threshold: i64,
) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OutcomeEvent, OutcomeProductRef};

    fn ev(verification: &str, pid: &str, qty: u32, kind: &str, amount: i64) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(0),
            occurred_at: Some(DateTime::from_millis(0)),
            amount: Some(amount),
            currency: Some("CNY".to_string()),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: Some(OutcomeProductRef {
                product_id: pid.to_string(),
                name: "P".to_string(),
                unit_price: Some(amount),
                sku: None,
                quantity: qty,
                entitlement_days: None,
            }),
            event_kind: kind.to_string(),
        }
    }

    // 用真实 Contact::default 形态构造（字段多——用一个 helper 取最小集；
    // 若 Contact 无 Default，实现者照 models.rs Contact 定义补全必填字段，
    // outcome_events 设为入参，account_id/workspace_id 设固定值）。
    fn contact_with(events: Vec<OutcomeEvent>) -> Contact {
        let mut c = crate::routes::campaigns::tests::base_contact();
        c.outcome_events = events;
        c
    }

    // 实现者：照 models.rs 的 Contact 真实字段构造一个最小 base（managed 状态）。
    pub(super) fn base_contact() -> Contact { todo!("照 Contact 定义补全必填字段") }

    #[test]
    fn coarse_filter_with_products_uses_elemmatch_real_keys() {
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 真实混合大小写路径
        let em = d.get_document("outcome_events").unwrap();
        let elem = em.get_document("$elemMatch").unwrap();
        // productRef.productId（camelCase 内嵌）
        assert!(elem.get_document("productRef").is_ok()
            || elem.contains_key("productRef.productId"));
        // verification 高可信 $in
        assert!(elem.contains_key("verification"));
        // eventKind 正向
        assert_eq!(elem.get_str("eventKind").ok(), Some("deal"));
        // 始终带租户隔离
        assert_eq!(d.get_str("workspace_id").unwrap(), "ws");
        assert_eq!(d.get_str("account_id").unwrap(), "acc");
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn coarse_filter_empty_products_skips_outcome_condition() {
        let f = SegmentFilter::default();
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 空 product_ids：不加 outcome_events 条件，退化为按其他维度圈纳管客户
        assert!(d.get("outcome_events").is_none());
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn precise_filter_net_holding_excludes_fully_refunded() {
        // 买1件后全额退款 → 净持有0 → 不命中「买过 vip」
        let events = vec![
            ev("staff_confirmed", "vip", 1, "deal", 19900),
            ev("staff_confirmed", "vip", 1, "reversal", 19900),
        ];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_conversation_inferred_never_matches() {
        // conversation_inferred 不进 G4 投影 → 不算持有
        let events = vec![ev("conversation_inferred", "vip", 1, "deal", 19900)];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_value_tier_high_only() {
        // 累计 35 万分 = high 档（high_threshold=30万）；要求 high → 命中
        let events = vec![ev("staff_confirmed", "vip", 1, "deal", 350000)];
        let f = SegmentFilter {
            product_ids: vec!["vip".into()],
            value_tier: Some("high".into()),
            ..Default::default()
        };
        assert!(contact_matches_segment(
            &contact_with(events.clone()), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
        // 要求 high 但只值 1.99 元(19900分=low) → 不命中
        let cheap = vec![ev("staff_confirmed", "vip", 1, "deal", 19900)];
        assert!(!contact_matches_segment(
            &contact_with(cheap), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }
}
```

- [ ] **Step 2: 声明模块并跑测试确认失败**

`src/routes/mod.rs` mod 声明区（:20-83）按字母序加：
```rust
mod campaigns;
```
然后：
```
cargo test --lib routes::campaigns
```
Expected: 失败（`todo!()` panic / `base_contact` todo）。

- [ ] **Step 3: 实现两个函数 + base_contact helper**

```rust
pub(super) fn build_segment_coarse_filter(
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> Document {
    let mut d = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
    };
    // 客户阶段裸字段在粗筛层 filter（domain_attributes.customer_stage 真实路径，
    // 实现者按 Contact 里 customer_stage 实际存储位置确认——见 contacts.rs:786
    // `domain_attributes.get_str("customer_stage")`，故路径为 domain_attributes.customer_stage）。
    if let Some(stage) = &filter.customer_stage {
        d.insert("domain_attributes.customer_stage", stage);
    }
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
    d
}

pub(super) fn contact_matches_segment(
    contact: &Contact,
    active_products: &[Product],
    filter: &SegmentFilter,
    now: DateTime,
    mid_threshold: i64,
    high_threshold: i64,
) -> bool {
    // 复用 G4 投影：净持有（退款抵消、净件数>0）。
    let (entitlements, _) = entitlements::project_entitlements(
        &contact.outcome_events, active_products, now, usize::MAX,
    );
    // 产品维度：要求净持有指定产品之一。
    if !filter.product_ids.is_empty() {
        let holds = entitlements.iter().any(|e| filter.product_ids.contains(&e.product_id));
        if !holds {
            return false;
        }
    }
    // 售后维度。
    if let Some(aftercare) = filter.aftercare.as_deref() {
        match aftercare {
            "in_aftercare" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(true)) {
                    return false;
                }
            }
            "expired" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(false)) {
                    return false;
                }
            }
            _ => {} // "any" 或未知：不约束
        }
    }
    // 价值分层维度。
    if let Some(tier) = filter.value_tier.as_deref() {
        let value = entitlements::compute_customer_value_cents(&contact.outcome_events);
        let actual = entitlements::classify_value_tier(value, mid_threshold, high_threshold);
        if actual != tier {
            return false;
        }
    }
    true
}
```

`base_contact` helper（实现者照 `src/models.rs` Contact 真实定义补全所有必填字段，关键字段：`workspace_id="ws"`、`account_id="acc"`、`wxid="wx1"`、`agent_status` 设为 managed 对应枚举值、`outcome_events=vec![]`、其余 Option 设 None / 集合设空）。

- [ ] **Step 4: 跑测试确认通过**

```
cargo test --lib routes::campaigns
```
Expected: 5 个 tests PASS。

- [ ] **Step 5: 提交**

```bash
git add src/routes/campaigns.rs src/routes/mod.rs
git commit -m "feat(campaign): segment 两阶段圈人查询(粗筛 filter + 内存精筛复用 G4)"
```

---

### Task 4: 活动生命周期 handler（create / preview / confirm_dispatch）+ REST 路由

**Files:**
- Modify: `src/routes/campaigns.rs`（加 3 个 handler + 请求体类型 + 圈人执行 + 自建 follow_up task）
- Modify: `src/routes/mod.rs`（`use campaigns::{...}` + 注册 4 条路由 + 导出 handler 供 Task 5 management 委托）

**Interfaces:**
- Consumes: `build_segment_coarse_filter` / `contact_matches_segment`（Task 3）；`AuthenticatedAdmin`；`AppState`；`AgentTask`（自建，仿 management.rs:1461）；`assert_campaign_status_valid`（Task 1）
- Produces（供 Task 5 management 委托，必须 `pub`/`pub(crate)`）：
  - `pub async fn create_campaign(State<AppState>, Extension<AuthenticatedAdmin>, Json<CreateCampaignRequest>) -> AppResult<Json<Value>>`
  - `pub async fn preview_campaign(State<AppState>, Extension<AuthenticatedAdmin>, Path<String>) -> AppResult<Json<Value>>`
  - `pub async fn dispatch_campaign(State<AppState>, Extension<AuthenticatedAdmin>, Path<String>) -> AppResult<Json<Value>>`
  - 请求体 `CreateCampaignRequest { title, intent_text, segment_filter }`（派生 Deserialize）

- [ ] **Step 1: 写圈人执行 + 自建 task 的单元测试**（追加 `src/routes/campaigns.rs` tests）

```rust
    #[test]
    fn build_follow_up_task_carries_intent_and_review() {
        // 自建 follow_up task（不调 planner 私有函数）：content=活动意图，
        // review_required=true，kind=follow_up，48h expiry，status=pending。
        let now = DateTime::from_millis(1_000_000);
        let task = build_campaign_follow_up_task("ws", "acc", "wx1", "双11老客7折", now);
        assert_eq!(task.kind, "follow_up");
        assert_eq!(task.content, "双11老客7折");
        assert!(task.review_required);
        assert_eq!(task.status, "pending");
        assert_eq!(task.contact_wxid, "wx1");
        // 48h expiry
        assert_eq!(
            task.expires_at.unwrap().timestamp_millis(),
            now.timestamp_millis() + 48 * 60 * 60 * 1000
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

```
cargo test --lib routes::campaigns::tests::build_follow_up_task
```
Expected: 失败（`build_campaign_follow_up_task` 未定义）。

- [ ] **Step 3: 实现请求体 + 自建 task helper + 3 个 handler**

```rust
use axum::extract::{Path, State};
use axum::{Extension, Json};
use mongodb::bson::oid::ObjectId;
use serde::Deserialize;
use serde_json::{json, Value};
use futures::TryStreamExt;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::{AgentTask, Campaign, CampaignSend, assert_campaign_status_valid};
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaignRequest {
    pub title: String,
    pub intent_text: String,
    #[serde(default)]
    pub segment_filter: crate::models::SegmentFilter,
}

/// 自建活动 follow_up 任务（不调 planner 私有 emit_planner_follow_up；
/// 形态对齐 management.rs:1461 create_follow_up_task）。
pub(super) fn build_campaign_follow_up_task(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    intent_text: &str,
    now: DateTime,
) -> AgentTask {
    let expires_at = DateTime::from_millis(now.timestamp_millis() + 48 * 60 * 60 * 1000);
    AgentTask {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.to_string(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: Some(expires_at),
        content: intent_text.to_string(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    }
}

/// 跑两阶段圈人，返回命中的 contacts。粗筛 Mongo + 内存精筛复用 G4。
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &crate::models::SegmentFilter,
) -> AppResult<Vec<crate::models::Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
    let mut cursor = state.db.contacts().find(coarse, None).await?;
    let active_products = entitlements::load_active_products(&state.db, workspace_id).await;
    let now = DateTime::now();
    // value_tier 阈值取 config（实现者确认 config 字段名；与 G6 classify_value_tier
    // 调用方一致。grep `value_tier` / `mid_threshold` 在 gateway/config 找现有阈值来源，
    // 复用同一对阈值，不新造配置）。
    let (mid, high) = crate::routes::campaigns::value_tier_thresholds(&state.config);
    let mut hits = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        if contact_matches_segment(&c, &active_products, filter, now, mid, high) {
            hits.push(c);
        }
    }
    Ok(hits)
}

pub async fn create_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<CreateCampaignRequest>,
) -> AppResult<Json<Value>> {
    if body.title.trim().is_empty() || body.intent_text.trim().is_empty() {
        return Err(AppError::BadRequest("title 与 intentText 不能为空".to_string()));
    }
    let now = DateTime::now();
    assert_campaign_status_valid("draft");
    let campaign = Campaign {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: state.config.default_account_id.clone(),
        title: body.title.trim().to_string(),
        intent_text: body.intent_text.trim().to_string(),
        segment_filter: body.segment_filter,
        status: "draft".to_string(),
        target_count: None,
        dispatched_count: 0,
        created_by: admin.username.clone(),
        created_at: now,
        updated_at: now,
    };
    let res = state.db.campaigns().insert_one(&campaign, None).await?;
    Ok(Json(json!({
        "id": res.inserted_id.as_object_id().map(|i| i.to_hex()),
        "status": "draft"
    })))
}

pub async fn preview_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    let campaign = state.db.campaigns()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;
    let hits = resolve_segment_contacts(
        &state, &campaign.workspace_id, &campaign.account_id, &campaign.segment_filter,
    ).await?;
    let target = hits.len() as i64;
    // 抽样 3-5 个示例（名/wxid）。
    let samples: Vec<Value> = hits.iter().take(5).map(|c| json!({
        "wxid": c.wxid,
        "name": c.remark.clone().or(c.nickname.clone()).unwrap_or_default(),
    })).collect();
    assert_campaign_status_valid("previewed");
    state.db.campaigns().update_one(
        doc! { "_id": oid, "workspace_id": &admin.current_workspace },
        doc! { "$set": { "status": "previewed", "targetCount": target, "updatedAt": DateTime::now() } },
        None,
    ).await?;
    Ok(Json(json!({
        "campaignId": id,
        "intentText": campaign.intent_text,
        "targetCount": target,
        "samples": samples,
    })))
}

pub async fn dispatch_campaign(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    let campaign = state.db.campaigns()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;
    // 重新跑圈人（防预览后数据漂移）。
    let hits = resolve_segment_contacts(
        &state, &campaign.workspace_id, &campaign.account_id, &campaign.segment_filter,
    ).await?;
    if hits.is_empty() {
        return Err(AppError::BadRequest("命中 0 人，无可推送对象".to_string()));
    }
    assert_campaign_status_valid("dispatching");
    state.db.campaigns().update_one(
        doc! { "_id": oid },
        doc! { "$set": { "status": "dispatching", "updatedAt": DateTime::now() } },
        None,
    ).await?;
    let now = DateTime::now();
    let mut dispatched = 0i64;
    for c in &hits {
        // 活动级去重：先尝试插 campaign_sends（唯一索引 (campaign_id, contact_wxid)）。
        // DuplicateKey → 已推过，跳过。
        let send = CampaignSend {
            id: None,
            workspace_id: campaign.workspace_id.clone(),
            account_id: campaign.account_id.clone(),
            campaign_id: oid,
            contact_wxid: c.wxid.clone(),
            task_id: None,
            status: "enqueued".to_string(),
            created_at: now,
        };
        match state.db.campaign_sends().insert_one(&send, None).await {
            Ok(send_res) => {
                let task = build_campaign_follow_up_task(
                    &campaign.workspace_id, &campaign.account_id, &c.wxid,
                    &campaign.intent_text, now,
                );
                crate::models::assert_agent_task_status_valid(&task.status);
                let task_res = state.db.tasks().insert_one(&task, None).await?;
                // 回填 task_id。
                if let (Some(send_id), Some(task_id)) = (
                    send_res.inserted_id.as_object_id(),
                    task_res.inserted_id.as_object_id(),
                ) {
                    state.db.campaign_sends().update_one(
                        doc! { "_id": send_id },
                        doc! { "$set": { "taskId": task_id } },
                        None,
                    ).await?;
                }
                dispatched += 1;
            }
            Err(e) if is_duplicate_key(&e) => { /* 去重命中，跳过 */ }
            Err(e) => return Err(e.into()),
        }
    }
    assert_campaign_status_valid("completed");
    state.db.campaigns().update_one(
        doc! { "_id": oid },
        doc! { "$set": { "status": "completed", "dispatchedCount": dispatched, "updatedAt": DateTime::now() } },
        None,
    ).await?;
    Ok(Json(json!({ "campaignId": id, "dispatchedCount": dispatched, "status": "completed" })))
}

/// DuplicateKey 判定（仿 products.rs:329 is_duplicate_key）。
fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(
        *err.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref e)) if e.code == 11000
    )
}

/// value_tier 阈值来源。实现者 grep 现有 classify_value_tier 调用方（gateway/config）
/// 复用同一对阈值，不新造配置；若现有阈值在 config，则读 config 对应字段。
pub(super) fn value_tier_thresholds(config: &crate::config::AppConfig) -> (i64, i64) {
    // 占位：实现者替换为真实 config 字段（grep `value_tier` 找现有 mid/high 来源）。
    // 若现有代码硬编码或在 DomainProfile，照同一来源取。
    (config.value_tier_mid_threshold_cents, config.value_tier_high_threshold_cents)
}
```

> **实现者注意**：`value_tier_thresholds` 的真实来源必须先 grep 确认（`classify_value_tier` 的现有调用方传的是什么阈值——可能在 `config.rs`、可能在 `DomainProfile`）。**复用现有来源**，不新造配置字段。若现有调用方从 `DomainProfile` 取，则 `resolve_segment_contacts` 也应从 active profile 取，签名相应调整。这是本任务唯一需要现场核实的依赖点。

- [ ] **Step 4: 注册路由**（`src/routes/mod.rs`）

`use` 段加：
```rust
use campaigns::{create_campaign, dispatch_campaign, preview_campaign};
```
`api_router` 里（products 路由 :775-779 同区）加：
```rust
        .route("/campaigns", post(create_campaign))
        .route("/campaigns/:id/preview", post(preview_campaign))
        .route("/campaigns/:id/dispatch", post(dispatch_campaign))
```

- [ ] **Step 5: 跑测试 + 编译确认**

```
cargo test --lib routes::campaigns
cargo check --lib
```
Expected: 全 PASS + 编译通过。

- [ ] **Step 6: 提交**

```bash
git add src/routes/campaigns.rs src/routes/mod.rs
git commit -m "feat(campaign): 活动生命周期 handler(create/preview/confirm_dispatch)+路由+去重扇出"
```

---

### Task 5: 总控 AI 工具接入（preview 只读 / dispatch 恒确认门）

**Files:**
- Modify: `src/routes/management.rs`（3+1 处：merge_product_tools 工具目录 :617-878；tool_effect 风险分级 :1096-1188；tool_always_requires_confirmation :1263-1268；execute_management_tool match 分支 :1321-2317）

**Interfaces:**
- Consumes: `crate::routes::campaigns::{create_campaign, preview_campaign, dispatch_campaign}`（Task 4，已 `pub`）；`management_admin(workspace_id)`（management.rs:2323）

- [ ] **Step 1: 写工具分级 + 确认门测试**（追加 management.rs tests，仿 :2488 `tool_effect_classifies_risk`）

```rust
    #[test]
    fn campaign_tools_risk_and_confirmation() {
        // preview 只读（dry-run 下也执行返回圈人结果）
        assert_eq!(tool_effect("wechatagent.preview_campaign").risk, ToolRisk::Readonly);
        assert!(tool_effect("wechatagent.preview_campaign").read_only);
        // dispatch 恒确认门——关键：dangerous 开关默认 false 下仍须确认
        assert!(tool_always_requires_confirmation("wechatagent.dispatch_campaign"));
        assert!(plan_requires_confirmation(&["wechatagent.dispatch_campaign"], false),
            "dispatch 必须无视第一期 dangerous 开关恒走确认门");
    }

    #[test]
    fn campaign_tools_in_catalog() {
        let merged = merge_product_tools(json!({ "tools": [] }));
        let names = advertised_tool_names(&merged);
        assert!(names.contains("wechatagent.preview_campaign"));
        assert!(names.contains("wechatagent.dispatch_campaign"));
    }
```

- [ ] **Step 2: 跑测试确认失败**

```
cargo test --lib routes::management::tests::campaign_tools
```
Expected: 失败（工具未注册 / 未分级）。

- [ ] **Step 3a: 工具目录声明**（`merge_product_tools` 的 `product_tools` 数组追加，management.rs:877 末项后）

```rust
        json!({
            "name": "wechatagent.preview_campaign",
            "description": "创建活动并预览圈中多少客户（只读，不发送）。先建活动再按条件圈人，返回命中人数+抽样示例。参数：title(活动名)，intentText(活动意图要点，将作为给客户的推送语境)，segmentFilter(圈人条件对象：productIds 买过的产品id数组、aftercare 售后状态 in_aftercare|expired|any、valueTier 价值分层 high|mid|low、customerStage 客户阶段，各项可选留空即不限)。返回 campaignId 供后续 dispatch。"
        }),
        json!({
            "name": "wechatagent.dispatch_campaign",
            "description": "确认扇出活动推送：给圈中的每个客户创建主动跟进任务，由 AI 结合各自画像生成个性化话术并经发送网关逐条发出。高风险动作，执行前必须确认。参数：campaignId（先用 preview_campaign 拿到并核对命中人数）。"
        }),
```

- [ ] **Step 3b: 风险分级**（`tool_effect` management.rs:1100-1109 Readonly 段加 preview；dispatch 不需进 Dangerous 段——靠 always_requires_confirmation）

Readonly 段（:1100-1109，与 query_* 同列）加：
```rust
        | "wechatagent.preview_campaign"
```

- [ ] **Step 3c: 恒确认门**（`tool_always_requires_confirmation` management.rs:1263-1268，与 verify 类同列加 dispatch）

```rust
pub(super) fn tool_always_requires_confirmation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "wechatagent.verify_knowledge_chunk"
            | "wechatagent.batch_verify_chunks"
            | "wechatagent.dispatch_campaign"
    )
}
```

- [ ] **Step 3d: 执行分支**（`execute_management_tool` match management.rs:1321，仿 query_health :1639 / generate_playbook :2037 委托范式加两 arm）

```rust
        "wechatagent.preview_campaign" => {
            // 先创建活动，再预览。create + preview 两步，返回 preview 结果。
            let body = serde_json::from_value(planned.arguments.clone())
                .map_err(|e| AppError::BadRequest(format!("参数解析失败: {e}")))?;
            let created = crate::routes::campaigns::create_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Json(body),
            ).await?;
            let campaign_id = created.0.get("id").and_then(Value::as_str)
                .ok_or_else(|| AppError::External("campaign id missing".to_string()))?
                .to_string();
            let resp = crate::routes::campaigns::preview_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(campaign_id),
            ).await?;
            Ok(resp.0)
        }
        "wechatagent.dispatch_campaign" => {
            let campaign_id = string_arg(&planned.arguments, "campaignId")?;
            let resp = crate::routes::campaigns::dispatch_campaign(
                State(state.clone()),
                Extension(management_admin(workspace_id)),
                Path(campaign_id),
            ).await?;
            Ok(resp.0)
        }
```

> **实现者注意**：`create_campaign` 返回的 JSON id 字段是 `Option<&str>`（可能 null），上面 `as_str` 失败要给清晰错误。preview 工具把 create+preview 合成一步是有意的——让总控 AI 一次调用就拿到"建好并圈完人"的预览结果，dispatch 再单独确认。

- [ ] **Step 4: 跑测试确认通过**

```
cargo test --lib routes::management::tests::campaign_tools
cargo check --lib
```
Expected: 2 个新测试 PASS + 编译通过。

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "feat(campaign): 总控AI接入 preview_campaign(只读)/dispatch_campaign(恒确认门)"
```

---

### Task 6: 基线门 + lint 收口

**Files:** 无新增（验证性任务）

- [ ] **Step 1: lib 基线（注意共享 target 污染，以 CI 为准）**

```
export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"
cargo test --lib campaign 2>&1 | tail -20
cargo test --lib routes::campaigns routes::management 2>&1 | tail -20
```
Expected: 新增测试全绿。（lib 全量 ≥350/0 以 CI 单分支 checkout 为准，本地共享 target 可能受并行会话污染。）

- [ ] **Step 2: no-human-takeover lint（新增行禁词扫描）**

```
bash scripts/check-no-human-takeover.sh 2>&1 | tail -10
```
Expected: 无禁词命中（campaign/segment 文案已用 AI 中性词；"主动跟进任务""个性化话术"等不含禁词）。
若命中：检查工具 description / 注释，把任何 `人工|接管|takeover|hand-off|人工介入` 替换为中性表述。

- [ ] **Step 3: 全量 lib 编译 + 文档同步检查**

```
cargo check --lib --tests
```
Expected: 编译通过（含 tests 目标，复刻 CI baseline step2）。

- [ ] **Step 4: 提交（若 lint/编译触发任何修补）**

```bash
git add -A
git commit -m "chore(campaign): 基线门 + no-human-takeover lint 收口"
```

> 若本步无改动则跳过提交。

---

## 自审记录

**1. spec 覆盖**：
- §4.1 Campaign/§4.2 CampaignSend/§4.3 不碰 AgentTask → Task 1 ✓
- §4.4 contacts 索引（真实 key）→ Task 2 ✓
- §5 两阶段查询（复用 G4）→ Task 3 ✓
- §6 生命周期 + 确认门 + 自建 task + 去重 → Task 4 ✓
- §7 总控 AI 两工具 + §7.2 确认门修正 → Task 5 ✓
- §8 错误处理（空人群/漂移重圈/去重幂等）→ Task 4 ✓
- Global Constraints lint/基线 → Task 6 ✓
- §9 前端不做 → 计划无前端任务 ✓

**2. 占位符扫描**：唯一现场核实点 = `value_tier_thresholds` 阈值来源（Task 4 Step 3 明确标注实现者须 grep `classify_value_tier` 调用方复用现有阈值，不新造配置）——这是真实依赖核实，非占位空话。其余代码块完整。

**3. 类型一致性**：`build_segment_coarse_filter`/`contact_matches_segment`（Task 3）→ Task 4 `resolve_segment_contacts` 调用签名一致；`SegmentFilter` 字段（product_ids/aftercare/value_tier/customer_stage）跨 Task 1/3/4/5 一致；`build_campaign_follow_up_task` 返回 `AgentTask`，字段对齐 models.rs:713 真实定义；status 字面量（draft/previewed/dispatching/completed）全过 `assert_campaign_status_valid` 且属闭集。

**4. 关键修正落实**：① 索引真实 key `outcome_events.productRef.productId`（Task 2 Step 3 + Task 3 filter）；② dispatch 用 `tool_always_requires_confirmation`（Task 5 Step 3c，非 Dangerous 档）；③ 自建 task 不调 planner（Task 4 `build_campaign_follow_up_task`）；④ gateway 频控/content 语义已在 spec §8 记录，发送链路不改（计划无 gateway 改动任务）。
