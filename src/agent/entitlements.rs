//! G4 · 当前持有状态投影（客观购买事实增强 spec §5）。
//!
//! G4 持有状态（entitlement）的真相源就是已核实的 `outcome_events`，**派生而非存储**
//! （独立落库会与 outcome_events drift，重蹈 C2 覆辙）。本模块是运行时纯投影函数：
//!
//! ```text
//! project_entitlements(outcome_events, active_products, cap_n) =
//!   fold over outcome_events
//!     .filter(verification ∈ {staff_confirmed, payment_verified})  // §2.1 红线
//!     .filter(has product_ref)
//!   → 按 product_id 聚合（正向 deal 累加件数 + reversal 退款抵消，净件数 ≤ 0 退出投影）
//!   → owned_since / 快照名只跟随正向成交（reversal 不刷新购买时刻）
//!   → 用 active product.attributes.entitlement_days 算"是否售后期内 / 何时到期"
//!   → 按 owned_since 倒序 take(N) 防撑爆 RunBudget
//! ```
//!
//! **零扰动**：空 outcome_events / 无 product_ref / 情感域产品表空 → 投影空 → 注入空串，
//! 与改造前字节等价（同 intent_trajectory 老文档向前兼容路径）。
//!
//! **退款非单调（§4.5）**：逆转**不删原 deal**（审计完整性），而是 append 一条
//! `event_kind="reversal"` 的反向事件。投影按 `product_id` 抵消净件数：全额退款
//! （净 ≤ 0）→ 退出持有；部分退款（净 > 0）→ 保留剩余件数。`event_kind` 缺省 `deal`，
//! 旧文档零破坏。

use mongodb::bson::DateTime;

use crate::models::{OutcomeEvent, Product};

/// G4 持有投影注入决策 prompt 的软上限（spec §5.1 防撑爆 RunBudget）。
/// 量级同 intent_trajectory `take(5)` / deprecated_facts `take(5)`；超量按
/// owned_since 倒序截断、段尾标注省略数。read 端点（§9 #6）不受此限。
pub(crate) const ENTITLEMENTS_PROMPT_CAP: usize = 8;

/// G4 投影输出的单条持有记录（派生视图，不落库）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entitlement {
    pub product_id: String,
    /// 产品名：优先取 active 产品表活名，回落成交快照名（产品已下架/改名时）。
    pub name: String,
    /// 最早持有时间（同一产品多次购买取最早一笔的 occurred_at ?? marked_at）。
    pub owned_since: DateTime,
    /// 累计件数（同一产品多次购买求和）。
    pub quantity: u32,
    /// 是否在售后/有效期内。无 `entitlement_days` 规则时为 `None`（不带时效，只表"已购买"）。
    pub in_aftercare: Option<bool>,
    /// 有效期截止时间（owned_since + entitlement_days）。无规则时 `None`。
    pub expires_at: Option<DateTime>,
}

/// 只有这两档可信度进入 G4 投影（§2.1：conversation_inferred 绝不进）。
fn verification_drives_entitlement(verification: &str) -> bool {
    matches!(verification, "staff_confirmed" | "payment_verified")
}

/// 对单个 contact 的 `outcome_events` 跑 G4 投影。
///
/// - `active_products`：当前 workspace 的 active 产品（解引用活名/算 entitlement_days）。
///   传入侧负责 `find({workspace_id, status:"active"})`（IDOR：只取本 workspace）。
/// - `now`：判定 in_aftercare 的当前时间（注入而非内部取，便于单测）。
/// - `cap_n`：投影上限，超量按 owned_since 倒序 take(N)（防撑爆 RunBudget，spec §5.1）。
///
/// 返回 `(entitlements, total)`：`entitlements` 是 cap 后的列表，`total` 是 cap 前的去重产品总数
/// （供 hint 段标注"等共 M 项"让 agent 知道有省略）。
pub(crate) fn project_entitlements(
    outcome_events: &[OutcomeEvent],
    active_products: &[Product],
    now: DateTime,
    cap_n: usize,
) -> (Vec<Entitlement>, usize) {
    // 按 product_id 聚合：最早 owned_since + 净件数（正向成交累加、reversal 抵消）+ 末次快照名兜底。
    // 用 Vec 保插入序的可重复读，规模小（单 contact 成交数），线性查足够。
    // owned_since / snapshot_name 只由正向 deal 事件决定（reversal 不是"购买时刻"，§4.5）。
    let mut agg: Vec<(String, DateTime, i64, String)> = Vec::new();
    for ev in outcome_events {
        if !verification_drives_entitlement(&ev.verification) {
            continue;
        }
        let Some(pref) = ev.product_ref.as_ref() else {
            continue;
        };
        let occurred = ev.occurred_at.unwrap_or(ev.marked_at);
        let is_reversal = ev.event_kind == "reversal";
        let signed_qty = if is_reversal {
            -(i64::from(pref.quantity.max(1)))
        } else {
            i64::from(pref.quantity.max(1))
        };
        match agg.iter_mut().find(|(pid, ..)| pid == &pref.product_id) {
            Some((_, owned_since, qty, snapshot_name)) => {
                *qty += signed_qty;
                // owned_since / 快照名只跟随正向成交（reversal 不刷新购买时刻）。
                if !is_reversal && occurred < *owned_since {
                    *owned_since = occurred;
                    *snapshot_name = pref.name.clone();
                }
            }
            None => {
                // 首见该 product：reversal 先于 deal 到达时 owned_since 暂记其时间，
                // 后续 deal 会按更早时间覆盖；净件数 ≤ 0 的最终会被过滤掉。
                agg.push((
                    pref.product_id.clone(),
                    occurred,
                    signed_qty,
                    pref.name.clone(),
                ));
            }
        }
    }

    // §4.5 非单调：净件数 ≤ 0（全额退款/撤单）→ 不再持有，退出投影。
    agg.retain(|(_, _, qty, _)| *qty > 0);

    let total = agg.len();

    // owned_since 倒序（最近购买优先注入），再 take(N)。
    agg.sort_by(|a, b| b.1.cmp(&a.1));
    agg.truncate(cap_n);

    let entitlements = agg
        .into_iter()
        .map(|(product_id, owned_since, quantity, snapshot_name)| {
            // 解引用活产品：取活名 + entitlement_days 规则。下架/改名 → 回落快照名、无时效。
            let active = active_products.iter().find(|p| p.product_id == product_id);
            let name = active
                .map(|p| p.name.clone())
                .unwrap_or(snapshot_name);
            let entitlement_days = active.and_then(entitlement_days_of);
            let (in_aftercare, expires_at) = match entitlement_days {
                Some(days) if days > 0 => {
                    let expires =
                        DateTime::from_millis(owned_since.timestamp_millis() + days * 86_400_000);
                    let within = now.timestamp_millis() <= expires.timestamp_millis();
                    (Some(within), Some(expires))
                }
                _ => (None, None),
            };
            Entitlement {
                product_id,
                name,
                owned_since,
                quantity: quantity.max(0) as u32,
                in_aftercare,
                expires_at,
            }
        })
        .collect();

    (entitlements, total)
}

/// 从 `Product.attributes.entitlement_days` 读售后/有效期天数（容忍 i32/i64/f64 数值键）。
fn entitlement_days_of(product: &Product) -> Option<i64> {
    let v = product.attributes.get("entitlement_days")?;
    v.as_i64()
        .or_else(|| v.as_i32().map(|n| n as i64))
        .or_else(|| v.as_f64().map(|n| n as i64))
        .filter(|n| *n > 0)
}

/// 把投影渲染成决策 prompt 的「客户已持有」段。空列表 → 空串（零扰动）。
///
/// 与 `format_intent_trajectory_hint` 同形态：纯展示，不含任何指令；时效信息只在
/// 有 entitlement_days 规则时出现。`total > entitlements.len()` 时段尾标注省略数。
pub(crate) fn format_entitlements_hint(entitlements: &[Entitlement], total: usize) -> String {
    if entitlements.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::with_capacity(entitlements.len() + 1);
    for e in entitlements {
        let mut parts = vec![format!("已购买「{}」", e.name)];
        if e.quantity > 1 {
            parts.push(format!("共 {} 件", e.quantity));
        }
        match (e.in_aftercare, e.expires_at) {
            (Some(true), Some(exp)) => parts.push(format!(
                "售后/有效期内（至 {}）",
                fmt_date(exp)
            )),
            (Some(false), Some(exp)) => {
                parts.push(format!("有效期已过（{} 截止）", fmt_date(exp)))
            }
            _ => {}
        }
        lines.push(format!("- {}", parts.join("，")));
    }
    if total > entitlements.len() {
        lines.push(format!("- …等共 {} 项持有记录（已省略较早条目）", total));
    }
    lines.join("\n")
}

fn fmt_date(dt: DateTime) -> String {
    // 仅到日期粒度，避免泄露精确时间戳噪声进 prompt。
    dt.try_to_rfc3339_string()
        .ok()
        .and_then(|s| s.get(0..10).map(|d| d.to_string()))
        .unwrap_or_else(|| dt.timestamp_millis().to_string())
}

/// 加载本 workspace 的 active 产品（供报价 + G4 投影解引用）。
///
/// IDOR（spec §3.5）：filter 必含 `workspace_id`，绝不全局加载。best-effort：
/// 任何 DB / 游标错误 → 空 Vec（决策层零注入、不阻塞回复，同 operator_memory 形态）。
pub(crate) async fn load_active_products(
    db: &crate::db::Database,
    workspace_id: &str,
) -> Vec<Product> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;

    let cursor = db
        .products()
        .find(
            doc! { "workspace_id": workspace_id, "status": "active" },
            None,
        )
        .await;
    match cursor {
        Ok(c) => c.try_collect().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// 把 active 产品渲染成决策 prompt 的「产品目录」段。空表 → 空串（零扰动）。
///
/// 只输出报价必需的结构化字段（名/价/币种/SKU/简述），不堆砌 attributes 噪声。
/// 这是 agent 报准确价的依据，区别于知识库非结构化 chunk 的模糊描述。
pub(crate) fn format_product_catalog_for_prompt(products: &[Product]) -> String {
    if products.is_empty() {
        return String::new();
    }
    products
        .iter()
        .map(|p| {
            let mut parts = vec![format!("「{}」(id={})", p.name, p.product_id)];
            match (p.price, p.currency.as_deref()) {
                (Some(price), Some(cur)) => parts.push(format!("{price} {cur}")),
                (Some(price), None) => parts.push(format!("{price}")),
                _ => parts.push("价格未设".to_string()),
            }
            if let Some(sku) = p.sku.as_deref().filter(|s| !s.is_empty()) {
                parts.push(format!("SKU={sku}"));
            }
            if let Some(s) = p.summary.as_deref().filter(|s| !s.is_empty()) {
                parts.push(s.to_string());
            }
            format!("- {}", parts.join("，"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// G3 §5.5：疑似成交线索的 agent 侧落点指引（运行时追加进 task prompt 末尾）。
///
/// **红线（§2.1）**：AI 永不自断成交、永不直写 `outcome_events`。当 agent 从对话里
/// 嗅到"客户像是已下单/已付款"的迹象时，正确动作有二：
///   1. 发一条**弱信号**（`agentGeneratedSignals` 里 kind=`suspected_deal`），它只进
///      `agent_run_logs.decision` 供后台「成交记录」Tab 高亮待核实，**不进 G4 投影、
///      不进正例池、不阻断本轮回复**；运营点确认后才由后台落成 `staff_confirmed` 真成交。
///   2. 用**主动求证话术**自然地跟客户确认（"方便确认下您这边是已经入手了吗？"），
///      这句写进本轮 reply，不写库。
///
/// **零扰动**：只在本 workspace 有 active 产品时追加本段（与目录/持有段同一隐式开关）；
/// 情感陪伴等无产品域 → 空串 → task prompt 字节等价（同 H17 形态）。
pub(crate) fn render_suspected_deal_guidance(active_products: &[Product]) -> String {
    if active_products.is_empty() {
        return String::new();
    }
    "\n\n# 疑似成交线索（仅在你从对话里察觉客户可能已下单/已付款时）\n\
     红线：你永远不自行认定成交、不直接登记成交记录。若察觉疑似成交，做两件事：\n\
     1) 在 agentGeneratedSignals 里追加一条 {\"kind\":\"suspected_deal\",\"value\":\"疑似成交·待核实\",\"evidence\":\"客户哪句话/哪个行为让你这么判断\",\"confidence\":0-100}，\
     这条只是供后台高亮待核实的线索，不代表成交已发生；\n\
     2) 在本轮 reply 里用自然口吻主动求证（例如\"方便确认下您这边是已经入手了吗？\"），不要替客户认定、不要催款施压。\n\
     未察觉疑似成交时，不要输出该信号。"
        .to_string()
}

/// R5.4 `priced_from_catalog` 判定（spec §5.4）：决策引用的 product_id 是否有任一
/// 命中本 workspace 的 active 产品。命中 → G2 结构化报价视为已背书，与 verified_chunks
/// 取或，避免准确报价被 `blocked_unverified_product_claim` 错杀。
///
/// 只认 active（§5.4.4：新报价读活表，archived 不进可报价集合 → 引用 archived 报新价仍触发红线）。
/// 空 quoted_ids / 空产品表 → 恒假 → 零扰动。
pub(crate) fn priced_from_active_catalog(
    quoted_product_ids: &[String],
    active_products: &[Product],
) -> bool {
    if quoted_product_ids.is_empty() || active_products.is_empty() {
        return false;
    }
    quoted_product_ids.iter().any(|qid| {
        let q = qid.trim();
        !q.is_empty() && active_products.iter().any(|p| p.product_id == q)
    })
}

/// 客观购买事实增强 spec §6（G4 当 G1 的客观锚）：G1「购买生命周期」维度的
/// **canonical 取值集合**。与 m020 seed 进 `system_taxonomies`（kind=`purchase_lifecycle`）
/// 的 value.id 逐字一致——纠偏逻辑产出的覆盖值必须落在该集合内，否则下游 taxonomy
/// 校验会把它当 CandidateNew。
pub(crate) const G1_DIMENSION_KIND: &str = "purchase_lifecycle";
pub(crate) const G1_NOT_PURCHASED: &str = "not_purchased";
pub(crate) const G1_PURCHASED: &str = "purchased";
pub(crate) const G1_AFTERCARE: &str = "aftercare";
pub(crate) const G1_REPURCHASE: &str = "repurchase";

/// spec §6：复用 C2「客观事实约束主观标签」模式，把 G4 持有投影当 G1 的客观锚。
///
/// G1 是 LLM 从聊天推断的「购买生命周期」维度（profile dimension）；G4 是从已核实
/// `outcome_events` 派生的**客观硬事实**。两者是同一件事的主客观两面。本纯函数判定
/// LLM 推断的 G1 标签是否与 G4 客观态冲突，冲突时**以 G4 为准**（客观锚优先）：
///
/// - G4 投影非空（有 staff_confirmed/payment_verified 持有）但 LLM 把 G1 推断成
///   `not_purchased`（未购买）→ 冲突 → 纠偏：
///     - 任一持有 `in_aftercare == Some(true)` → 覆盖为 `aftercare`（售后期内）；
///     - 否则 → 覆盖为 `purchased`（已购买）。
/// - G4 投影为空（无任何已核实持有）→ **不**纠偏：返回 `None`。spec §2.1 红线——
///   `conversation_inferred` 疑似线索绝不进 G4 投影，故"投影空"不代表"一定没买"，
///   只代表"无客观证据"，此时尊重 LLM 推断（可能是 not_purchased，也可能 repurchase
///   等纯对话信号），不臆测覆盖。
/// - `repurchase`（复购期）是含购买语义的标签，与"已购买"客观态不冲突 → 不纠偏。
/// - LLM 未给 G1（`llm_g1` 为空）但 G4 有持有 → 补一个客观锚值（同上 aftercare/purchased
///   二选一），让维度不因 LLM 漏报而空缺。
///
/// 返回 `Some((corrected_value, llm_original))`：需要覆盖时给出 canonical 纠偏值与
/// LLM 原值（供 gateway emit fail-soft 审计事件，类比 operation_state_transition_rejected）；
/// 无需纠偏时 `None`。
///
/// **零扰动**：销售域 DEFAULT profile 不含 purchase_lifecycle 维度 → 调用方不会对其
/// 调用本函数（仅当 profile 声明了 G1 维度且 G4 有持有时才进），情感域产品表空 →
/// 投影恒空 → 恒返回 `None`。
pub(crate) fn reconcile_g1_with_entitlements(
    llm_g1: Option<&str>,
    entitlements: &[Entitlement],
) -> Option<(String, String)> {
    // G4 投影空 → 无客观证据 → 尊重 LLM 推断，不纠偏。
    if entitlements.is_empty() {
        return None;
    }
    let objective = if entitlements.iter().any(|e| e.in_aftercare == Some(true)) {
        G1_AFTERCARE
    } else {
        G1_PURCHASED
    };
    let llm = llm_g1.map(str::trim).filter(|s| !s.is_empty());
    match llm {
        // LLM 漏报 → 补客观锚。
        None => Some((objective.to_string(), String::new())),
        // 含购买语义的标签（purchased/aftercare/repurchase）与"已购买"客观态不冲突。
        Some(G1_PURCHASED) | Some(G1_AFTERCARE) | Some(G1_REPURCHASE) => None,
        // 明确冲突：LLM 说未购买，但客观有已核实持有 → 以 G4 为准。
        Some(G1_NOT_PURCHASED) => Some((objective.to_string(), G1_NOT_PURCHASED.to_string())),
        // 其它未知 G1 取值：尊重 LLM（交给 taxonomy 候选通道），不强行覆盖。
        Some(_) => None,
    }
}

/// H11-linkage（spec §9 #9）：从 contact 的 `outcome_events` 抽取**已核实正向成交**
/// 的发生时刻，供自学习正向循环（回路① 召回置信度）的「成交追认」归因使用。
///
/// **红线守点（§2.1 AI 永不自断成交）**：复用 [`verification_drives_entitlement`]
/// 闭集——只有 `staff_confirmed` / `payment_verified` 进，`conversation_inferred`
/// （AI 推断的疑似成交）在此被物理排除，绝不借成交追认通道混进正向循环。
///
/// 只取**正向** deal（`event_kind != "reversal"`）：退款/撤单不产生正向追认（也不
/// 反向扣分——负向训练是回路② 的职责，不在本通道）。时刻取 `occurred_at ?? marked_at`
/// （与 G4 投影 [`project_entitlements`] 同口径）。返回升序时刻列表。
///
/// **零扰动**：空 outcome_events / 无已核实成交（情感陪伴域无产品 → 无带 product_ref
/// 的成交）→ 空 Vec → 上游成交追认天然不触发。
pub(crate) fn confirmed_deal_timestamps(outcome_events: &[OutcomeEvent]) -> Vec<DateTime> {
    let mut times: Vec<DateTime> = outcome_events
        .iter()
        .filter(|ev| verification_drives_entitlement(&ev.verification))
        .filter(|ev| ev.event_kind != "reversal")
        .map(|ev| ev.occurred_at.unwrap_or(ev.marked_at))
        .collect();
    times.sort_by_key(|t| t.timestamp_millis());
    times
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, DateTime, Document};

    use crate::models::OutcomeProductRef;

    fn ev(verification: &str, product: Option<(&str, &str, u32)>, occurred_ms: i64) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(occurred_ms),
            occurred_at: Some(DateTime::from_millis(occurred_ms)),
            amount: Some(199.0),
            currency: Some("CNY".to_string()),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: product.map(|(pid, name, qty)| OutcomeProductRef {
                product_id: pid.to_string(),
                name: name.to_string(),
                unit_price: Some(199.0),
                sku: None,
                quantity: qty,
            }),
            event_kind: "deal".to_string(),
        }
    }

    /// 构造一条退款/逆转事件（§4.5）：同 `ev` 但 `event_kind="reversal"`，按 product_id 抵消件数。
    fn reversal(product: (&str, &str, u32), occurred_ms: i64) -> OutcomeEvent {
        let mut e = ev("staff_confirmed", Some(product), occurred_ms);
        e.event_kind = "reversal".to_string();
        e
    }

    fn product(pid: &str, name: &str, attributes: Document) -> Product {
        Product {
            id: None,
            workspace_id: "default".to_string(),
            product_id: pid.to_string(),
            name: name.to_string(),
            price: Some(199.0),
            currency: Some("CNY".to_string()),
            sku: None,
            status: "active".to_string(),
            summary: None,
            attributes,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn conversation_inferred_never_enters_projection() {
        let now = DateTime::from_millis(1_000_000);
        let events = vec![ev("conversation_inferred", Some(("p1", "疑似买的", 1)), 100)];
        let (ents, total) = project_entitlements(&events, &[], now, 10);
        assert!(ents.is_empty(), "conversation_inferred 绝不进 G4 投影");
        assert_eq!(total, 0);
    }

    #[test]
    fn event_without_product_ref_skipped() {
        let now = DateTime::from_millis(1_000_000);
        let events = vec![ev("staff_confirmed", None, 100)];
        let (ents, _) = project_entitlements(&events, &[], now, 10);
        assert!(ents.is_empty(), "无 product_ref 的成交不进持有投影");
    }

    #[test]
    fn empty_events_zero_perturbation() {
        let now = DateTime::from_millis(1_000_000);
        let (ents, total) = project_entitlements(&[], &[], now, 10);
        assert!(ents.is_empty());
        assert_eq!(total, 0);
        assert_eq!(format_entitlements_hint(&ents, total), "", "空投影注入空串");
    }

    #[test]
    fn staff_confirmed_aggregates_by_product_and_sums_quantity() {
        let now = DateTime::from_millis(1_000_000);
        let events = vec![
            ev("staff_confirmed", Some(("vip", "年度会员", 1)), 200),
            ev("payment_verified", Some(("vip", "年度会员", 2)), 100),
        ];
        let (ents, total) = project_entitlements(&events, &[], now, 10);
        assert_eq!(total, 1, "同 product_id 聚合为一条");
        assert_eq!(ents.len(), 1);
        assert_eq!(ents[0].product_id, "vip");
        assert_eq!(ents[0].quantity, 3, "件数累计");
        assert_eq!(ents[0].owned_since.timestamp_millis(), 100, "取最早一笔");
        assert_eq!(ents[0].in_aftercare, None, "无 entitlement_days 规则 → 无时效");
    }

    #[test]
    fn entitlement_days_drives_in_aftercare_window() {
        let day = 86_400_000i64;
        let bought_at = DateTime::from_millis(0);
        // entitlement_days=30：第 10 天仍在期内，第 40 天已过期。
        let prod = product("course", "训练营", doc! { "entitlement_days": 30i64 });
        let events = vec![ev("staff_confirmed", Some(("course", "训练营", 1)), 0)];

        let (within, _) =
            project_entitlements(&events, &[prod.clone()], DateTime::from_millis(10 * day), 10);
        assert_eq!(within[0].in_aftercare, Some(true), "第10天在售后期内");
        assert!(within[0].expires_at.is_some());

        let (expired, _) =
            project_entitlements(&events, &[prod], DateTime::from_millis(40 * day), 10);
        assert_eq!(expired[0].in_aftercare, Some(false), "第40天已过期");
        let _ = bought_at;
    }

    #[test]
    fn active_product_name_overrides_snapshot_but_falls_back_when_archived() {
        let now = DateTime::from_millis(1_000_000);
        // 成交快照名是旧名「基础版」；活产品表已改名「标准版」。
        let events = vec![ev("staff_confirmed", Some(("p", "基础版", 1)), 100)];
        let renamed = product("p", "标准版", Document::new());
        let (ents, _) = project_entitlements(&events, &[renamed], now, 10);
        assert_eq!(ents[0].name, "标准版", "有活产品时取活名");

        // 产品已下架（不在 active_products）→ 回落快照名，不污染。
        let (ents2, _) = project_entitlements(&events, &[], now, 10);
        assert_eq!(ents2[0].name, "基础版", "下架产品回落成交快照名");
    }

    #[test]
    fn cap_n_truncates_and_total_reports_full_count() {
        let now = DateTime::from_millis(1_000_000);
        // 5 个不同产品，occurred_ms = 100..500（p4 最近）。
        let events: Vec<OutcomeEvent> = (0..5)
            .map(|i| {
                let name = format!("产品{i}");
                let mut e = ev("staff_confirmed", None, (i as i64 + 1) * 100);
                e.product_ref = Some(OutcomeProductRef {
                    product_id: format!("p{i}"),
                    name,
                    unit_price: Some(199.0),
                    sku: None,
                    quantity: 1,
                });
                e
            })
            .collect();
        let (ents, total) = project_entitlements(&events, &[], now, 2);
        assert_eq!(total, 5, "total 报 cap 前去重总数");
        assert_eq!(ents.len(), 2, "cap_n=2 截断");
        // owned_since 倒序：最近的 p4、p3 在前。
        assert_eq!(ents[0].product_id, "p4");
        assert_eq!(ents[1].product_id, "p3");

        let hint = format_entitlements_hint(&ents, total);
        assert!(hint.contains("等共 5 项"), "段尾标注省略数：{hint}");
    }

    #[test]
    fn priced_from_active_catalog_hits_active_id_only() {        let p1 = product("vip", "年度会员", Document::new());
        let p2 = product("course", "训练营", Document::new());
        let active = vec![p1, p2];

        // 命中 active id → true
        assert!(priced_from_active_catalog(
            &["vip".to_string()],
            &active
        ));
        // 多个引用，任一命中即 true
        assert!(priced_from_active_catalog(
            &["unknown".to_string(), "course".to_string()],
            &active
        ));
        // 全不命中 → false
        assert!(!priced_from_active_catalog(
            &["ghost".to_string()],
            &active
        ));
        // 空引用 → false（零扰动）
        assert!(!priced_from_active_catalog(&[], &active));
        // 空产品表 → false（零扰动）
        assert!(!priced_from_active_catalog(&["vip".to_string()], &[]));
        // 空白字符串引用不算命中
        assert!(!priced_from_active_catalog(
            &["   ".to_string()],
            &active
        ));
    }

    #[test]
    fn suspected_deal_guidance_zero_perturbation_when_no_products() {
        // 无产品域（情感陪伴）→ 空串，task prompt 字节等价。
        assert_eq!(render_suspected_deal_guidance(&[]), "");
    }

    #[test]
    fn suspected_deal_guidance_present_with_active_products() {
        let p = product("vip", "年度会员", Document::new());
        let g = render_suspected_deal_guidance(&[p]);
        assert!(!g.is_empty(), "有产品时追加疑似成交指引");
        // 走弱信号通道而非直写成交。
        assert!(g.contains("agentGeneratedSignals"));
        assert!(g.contains("suspected_deal"));
        // §2.1 红线在 prompt 文本里显式重申。
        assert!(g.contains("不直接登记成交"));
        // 命名红线复核：不得出现 check-no-human-takeover 禁词。
        for forbidden in ["人工", "接管", "takeover", "hand-off", "handoff"] {
            assert!(
                !g.contains(forbidden),
                "疑似成交指引不得含禁词 {forbidden}"
            );
        }
    }

    #[test]
    fn full_reversal_removes_entitlement() {
        // 买 1 件后全额退款 → 净件数 0 → 退出持有投影（§4.5 非单调）。
        let now = DateTime::from_millis(1_000_000);
        let events = vec![
            ev("staff_confirmed", Some(("vip", "年度会员", 1)), 100),
            reversal(("vip", "年度会员", 1), 200),
        ];
        let (ents, total) = project_entitlements(&events, &[], now, 10);
        assert!(ents.is_empty(), "全额退款后不再持有");
        assert_eq!(total, 0);
    }

    #[test]
    fn partial_reversal_keeps_remaining_quantity() {
        // 买 3 件、退 1 件 → 净 2 件仍持有。
        let now = DateTime::from_millis(1_000_000);
        let events = vec![
            ev("staff_confirmed", Some(("course", "训练营", 3)), 100),
            reversal(("course", "训练营", 1), 200),
        ];
        let (ents, total) = project_entitlements(&events, &[], now, 10);
        assert_eq!(total, 1);
        assert_eq!(ents[0].quantity, 2, "净件数 = 3 - 1");
        assert_eq!(ents[0].owned_since.timestamp_millis(), 100, "reversal 不刷新购买时刻");
    }

    #[test]
    fn over_reversal_clamps_and_removes() {
        // 退款件数超过持有（数据异常）→ 净 ≤ 0 → 退出投影，不出现负件数。
        let now = DateTime::from_millis(1_000_000);
        let events = vec![
            ev("staff_confirmed", Some(("x", "甲", 1)), 100),
            reversal(("x", "甲", 5), 200),
        ];
        let (ents, total) = project_entitlements(&events, &[], now, 10);
        assert!(ents.is_empty(), "净件数 ≤ 0 退出投影");
        assert_eq!(total, 0);
    }

    #[test]
    fn legacy_event_without_event_kind_defaults_to_deal() {
        // 旧 JSON 无 event_kind 字段 → 反序列化缺省 "deal"，正向成交语义不变。
        let json = r#"{
            "markedAt": {"$date": {"$numberLong": "100"}},
            "verification": "staff_confirmed",
            "productRef": {"productId": "p", "name": "旧货", "quantity": 1}
        }"#;
        let ev: OutcomeEvent = serde_json::from_str(json).expect("旧 OutcomeEvent 须可反序列化");
        assert_eq!(ev.event_kind, "deal", "缺 event_kind 缺省 deal");
        let (ents, _) =
            project_entitlements(&[ev], &[], DateTime::from_millis(1_000_000), 10);
        assert_eq!(ents.len(), 1, "旧正向成交照常进投影");
    }

    // ── spec §6：G4→G1 客观锚纠偏 ──

    fn entitlement(in_aftercare: Option<bool>) -> Entitlement {
        Entitlement {
            product_id: "p1".to_string(),
            name: "课程".to_string(),
            owned_since: DateTime::from_millis(100),
            quantity: 1,
            in_aftercare,
            expires_at: None,
        }
    }

    #[test]
    fn reconcile_empty_projection_never_corrects() {
        // G4 投影空 → 无客观证据 → 尊重 LLM 推断（含 not_purchased），不纠偏。
        assert_eq!(reconcile_g1_with_entitlements(Some(G1_NOT_PURCHASED), &[]), None);
        assert_eq!(reconcile_g1_with_entitlements(None, &[]), None);
        assert_eq!(reconcile_g1_with_entitlements(Some(G1_REPURCHASE), &[]), None);
    }

    #[test]
    fn reconcile_conflict_not_purchased_overridden_by_objective() {
        // LLM 说未购买，但客观有已核实持有（无售后时效）→ 覆盖为 purchased。
        let ents = vec![entitlement(None)];
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_NOT_PURCHASED), &ents),
            Some((G1_PURCHASED.to_string(), G1_NOT_PURCHASED.to_string()))
        );
    }

    #[test]
    fn reconcile_conflict_prefers_aftercare_when_in_window() {
        // 任一持有在售后期内 → 覆盖为 aftercare（更具体的客观态优先于 purchased）。
        let ents = vec![entitlement(Some(true))];
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_NOT_PURCHASED), &ents),
            Some((G1_AFTERCARE.to_string(), G1_NOT_PURCHASED.to_string()))
        );
    }

    #[test]
    fn reconcile_expired_aftercare_falls_back_to_purchased() {
        // 持有但售后期已过（in_aftercare=Some(false)）→ 仍是"已购买"，不是 aftercare。
        let ents = vec![entitlement(Some(false))];
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_NOT_PURCHASED), &ents),
            Some((G1_PURCHASED.to_string(), G1_NOT_PURCHASED.to_string()))
        );
    }

    #[test]
    fn reconcile_purchase_semantic_labels_not_corrected() {
        // 含购买语义的标签与"已购买"客观态不冲突 → 不纠偏。
        let ents = vec![entitlement(Some(true))];
        assert_eq!(reconcile_g1_with_entitlements(Some(G1_PURCHASED), &ents), None);
        assert_eq!(reconcile_g1_with_entitlements(Some(G1_AFTERCARE), &ents), None);
        assert_eq!(reconcile_g1_with_entitlements(Some(G1_REPURCHASE), &ents), None);
    }

    #[test]
    fn reconcile_missing_g1_backfills_objective_anchor() {
        // LLM 漏报 G1 但客观有持有 → 补客观锚，让维度不空缺。llm_original 为空串。
        let ents = vec![entitlement(Some(true))];
        assert_eq!(
            reconcile_g1_with_entitlements(None, &ents),
            Some((G1_AFTERCARE.to_string(), String::new()))
        );
        assert_eq!(
            reconcile_g1_with_entitlements(Some("  "), &ents),
            Some((G1_AFTERCARE.to_string(), String::new())),
            "空白 G1 视同漏报"
        );
    }

    #[test]
    fn reconcile_unknown_g1_value_respected() {
        // 未知 G1 取值（非 canonical 集合）→ 尊重 LLM，交 taxonomy 候选通道，不覆盖。
        let ents = vec![entitlement(Some(true))];
        assert_eq!(
            reconcile_g1_with_entitlements(Some("某种自造阶段"), &ents),
            None
        );
    }

    // ── H11-linkage：confirmed_deal_timestamps ──

    #[test]
    fn confirmed_deals_only_verified_positive() {
        // staff_confirmed / payment_verified 进；conversation_inferred 排除（红线）；
        // reversal 排除（退款不产生正向追认）。
        let events = vec![
            ev("staff_confirmed", Some(("p1", "课程", 1)), 100),
            ev("payment_verified", Some(("p2", "会员", 1)), 200),
            ev("conversation_inferred", Some(("p3", "疑似", 1)), 300),
            {
                let mut r = ev("staff_confirmed", Some(("p1", "课程", 1)), 400);
                r.event_kind = "reversal".to_string();
                r
            },
        ];
        let times = confirmed_deal_timestamps(&events);
        let ms: Vec<i64> = times.iter().map(|t| t.timestamp_millis()).collect();
        assert_eq!(ms, vec![100, 200], "只取 staff_confirmed/payment_verified 的正向成交");
    }

    #[test]
    fn confirmed_deals_uses_occurred_at_then_marked_at() {
        // occurred_at 优先；缺省回落 marked_at。
        let mut e = ev("staff_confirmed", Some(("p", "x", 1)), 500);
        e.occurred_at = None; // marked_at = 500（ev 里 marked_at=occurred_ms）
        let times = confirmed_deal_timestamps(&[e]);
        assert_eq!(times[0].timestamp_millis(), 500);
    }

    #[test]
    fn confirmed_deals_empty_zero_perturbation() {
        assert!(confirmed_deal_timestamps(&[]).is_empty());
        // 全是 conversation_inferred → 空（情感域/疑似线索不产追认）。
        let only_inferred = vec![ev("conversation_inferred", Some(("p", "x", 1)), 1)];
        assert!(confirmed_deal_timestamps(&only_inferred).is_empty());
    }

    #[test]
    fn confirmed_deals_sorted_ascending() {
        let events = vec![
            ev("staff_confirmed", Some(("p1", "a", 1)), 300),
            ev("payment_verified", Some(("p2", "b", 1)), 100),
            ev("staff_confirmed", Some(("p3", "c", 1)), 200),
        ];
        let ms: Vec<i64> = confirmed_deal_timestamps(&events)
            .iter()
            .map(|t| t.timestamp_millis())
            .collect();
        assert_eq!(ms, vec![100, 200, 300], "时刻升序");
    }
}

