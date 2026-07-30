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
    // owned_since / snapshot_name 取最早正向 deal（reversal 不是"购买时刻"，§4.5）——资历展示语义。
    // 售后到期则**不**锁死最早笔：每笔正向成交各贡献一个到期锚 (occurred, 该笔快照 days)，
    // map 阶段取 max(occurred+days)（A 修复：续费/复购各续一段窗、取最晚到期，而非按首购固定）。
    // agg 元组：(product_id, owned_since, 净件数, 最早笔名兜底, 售后到期锚 Vec<(occurred_ms, days)>)。
    let mut agg: Vec<(String, DateTime, i64, String, Vec<(i64, Option<i64>)>)> = Vec::new();
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
            Some((_, owned_since, qty, snapshot_name, expiry_anchors)) => {
                *qty += signed_qty;
                if !is_reversal {
                    // 资历：owned_since / 快照名跟随最早一笔正向成交。
                    if occurred < *owned_since {
                        *owned_since = occurred;
                        *snapshot_name = pref.name.clone();
                    }
                    // 售后窗：每笔正向成交都追加到期锚（续费/复购各续一段）。
                    expiry_anchors.push((occurred.timestamp_millis(), pref.entitlement_days));
                }
            }
            None => {
                // 首见该 product：reversal 先于 deal 到达时 owned_since 暂记其时间，
                // 后续 deal 会按更早时间覆盖；净件数 ≤ 0 的最终会被过滤掉。
                // reversal 不延长售后 → 到期锚仅正向成交贡献（首见即 reversal 时为空）。
                let anchors = if is_reversal {
                    Vec::new()
                } else {
                    vec![(occurred.timestamp_millis(), pref.entitlement_days)]
                };
                agg.push((
                    pref.product_id.clone(),
                    occurred,
                    signed_qty,
                    pref.name.clone(),
                    anchors,
                ));
            }
        }
    }

    // §4.5 非单调：净件数 ≤ 0（全额退款/撤单）→ 不再持有，退出投影。
    agg.retain(|(_, _, qty, _, _)| *qty > 0);

    let total = agg.len();

    // owned_since 倒序（最近购买优先注入），再 take(N)。
    agg.sort_by(|a, b| b.1.cmp(&a.1));
    agg.truncate(cap_n);

    let entitlements = agg
        .into_iter()
        .map(
            |(product_id, owned_since, quantity, snapshot_name, expiry_anchors)| {
                // 解引用活产品：取活名。下架/改名 → 回落快照名。
                let active = active_products.iter().find(|p| p.product_id == product_id);
                let name = active.map(|p| p.name.clone()).unwrap_or(snapshot_name);
                // G4 #4 + A：售后到期 = 各正向成交锚 max(occurred + 该笔 days)。
                // 每笔 days 优先用成交时冻结的快照，缺失（老成交未登记）才回落活产品表。
                // archived 产品活引用解不到，但快照仍在 → in_aftercare 不丢（#4）；
                // 续费/复购各笔独立续窗、取最晚到期 → 刚续费客户不被按首购判过期（A）。
                let active_days = active.and_then(entitlement_days_of);
                let expires_ms = expiry_anchors
                    .iter()
                    .filter_map(|(occurred_ms, snap_days)| {
                        snap_days
                            .or(active_days)
                            .filter(|d| *d > 0)
                            .map(|d| occurred_ms + d * 86_400_000)
                    })
                    .max();
                let (in_aftercare, expires_at) = match expires_ms {
                    Some(ms) => {
                        let within = now.timestamp_millis() <= ms;
                        (Some(within), Some(DateTime::from_millis(ms)))
                    }
                    None => (None, None),
                };
                Entitlement {
                    product_id,
                    name,
                    owned_since,
                    quantity: quantity.max(0) as u32,
                    in_aftercare,
                    expires_at,
                }
            },
        )
        .collect();

    (entitlements, total)
}

/// 从 `Product.attributes.entitlement_days` 读售后/有效期天数（容忍 i32/i64/f64 数值键）。
pub(crate) fn entitlement_days_of(product: &Product) -> Option<i64> {
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
            (Some(true), Some(exp)) => parts.push(format!("售后/有效期内（至 {}）", fmt_date(exp))),
            (Some(false), Some(exp)) => parts.push(format!("有效期已过（{} 截止）", fmt_date(exp))),
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

/// 金额整数化命门：把最小币种单位整数（分）渲染成「元」展示字符串（如 19900→"199.00"）。
/// AI 决策语义是元，若把分值 19900 直接喂给 agent 会报成 100 倍错价。金额已校验非负
/// （add_deal_event / validate_product_money），故只需处理非负；防御性对负值取绝对值避免
/// 出现 "-1.-5" 形态。两位小数固定（÷100），与全局"不按币种驱动小数位"约定一致。
fn fmt_minor_as_major(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
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
                (Some(price), Some(cur)) => {
                    parts.push(format!("{} {cur}", fmt_minor_as_major(price)))
                }
                (Some(price), None) => parts.push(fmt_minor_as_major(price)),
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

/// 数字分身 T7：关系性质（relationship_type）建议的 agent 侧落点指引（运行时追加进
/// task prompt 末尾）。引导 LLM 在嗅到「对方关系性质的明确新证据」时，往
/// `agentGeneratedSignals` 追加一条 `kind=relationship_type` 弱信号——经 T6 提取 + 字典
/// 校验后 upsert 进建议 collection，**须运营审核才回写 contact**（不直接生效）。
///
/// **与 T6 提取契约对齐**：kind 逐字 `relationship_type`，字段 `value`/`evidence`/
/// `confidence`（见 `gateway::extract_relationship_type_suggestion`）。`value` 取本账号
/// `relationship_type` 字典的 canonical id（当前 seed：customer/peer/friend，m024）。
///
/// **反过拟合**：引导基于「关系性质新证据」这个通用方法论，不写死任何行业判断规则；
/// 取值口径指向「本账号字典」，保持行业中性（运营可扩展 supplier 等）。
///
/// **稳定属性，不每轮臆测**：relationship_type 是稳定属性，只在本轮出现明确新证据
/// （对方明确表达身份/关系定位）时才产出，没有新证据就不输出、更不反复改判。
///
/// **零扰动**：本段常驻（对所有 profile 一致追加），但它只是「有新证据才产出」的可选
/// 指引——不强制 LLM 每轮输出，故 DEFAULT 销售域追加本段不改变既有行为（无新证据 →
/// 不产信号 → 决策与改造前等价）。无参纯函数，供 decision.rs 调用 + lib 单测共用。
pub(crate) fn render_relationship_type_suggestion_guidance() -> String {
    "\n\n# 关系性质识别（数字分身，仅在本轮出现关系性质的明确新证据时）\n\
     关系性质指对方相对本账号机主的关系定位（按本账号 relationship_type 字典的取值，\
     当前如 customer 客户 / peer 同行 / friend 朋友，运营可扩展）。这是稳定属性，\
     **不要每轮臆测或反复改判**。\n\
     仅当本轮对话出现**明确的新证据**（对方明确表达自身身份/关系定位，如自称同行、\
     以朋友口吻相处、确立或言明客户关系等），才在 agentGeneratedSignals 追加一条 \
     {\"kind\":\"relationship_type\",\"value\":\"<本账号字典里的取值 id>\",\
     \"evidence\":\"对方哪句话/哪个行为构成这条新证据\",\"confidence\":0-100}。\n\
     这条只是供后台高亮待核实的建议，不直接改变对方的关系标签（须经审核才回写）。\n\
     没有明确新证据时，不要输出该信号。"
        .to_string()
}

/// G4 #5：决策 prompt 三段交易事实注入（产品目录 / 当前持有投影 / 疑似成交指引）的统一
/// 渲染 + 交易域闸。返回 `(product_catalog_text, entitlements_text, suspected_deal_text)`。
///
/// `enabled=false`（非交易域：情感陪伴/朋友）→ 三段一律空串，**即便传入非空 products**
/// （双重保险：调用方虽已按 enabled 决定是否 load，闸门仍内聚于此，防未来调用点漏判）。
/// `enabled=true`（交易域）→ 组合既有三个 format 函数；products 为空时各 format 自然产出
/// 空串（零扰动，与无产品域一致）。纯函数、无 IO，供 decision.rs 调用 + lib 单测共用。
pub(crate) fn render_transaction_facts_sections(
    enabled: bool,
    active_products: &[Product],
    outcome_events: &[OutcomeEvent],
    now: DateTime,
) -> (String, String, String) {
    if !enabled {
        return (String::new(), String::new(), String::new());
    }
    let product_catalog_text = format_product_catalog_for_prompt(active_products);
    let (entitlements, total) = project_entitlements(
        outcome_events,
        active_products,
        now,
        ENTITLEMENTS_PROMPT_CAP,
    );
    let entitlements_text = format_entitlements_hint(&entitlements, total);
    let suspected_deal_text = render_suspected_deal_guidance(active_products);
    (product_catalog_text, entitlements_text, suspected_deal_text)
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

/// G6 价值分层维度 kind + 三档 canonical 取值（与 m023 seed 的 value.id 逐字一致）。
/// value_tier 是**客观计算派生值**（累计成交额规则算），不经 LLM domain_signals 通道——
/// gateway 写侧直接 set domain_attributes.value_tier，与 G1（LLM 推断 + 客观锚纠偏）不同。
pub(crate) const VALUE_TIER_KIND: &str = "value_tier";
pub(crate) const VALUE_TIER_HIGH: &str = "high";
pub(crate) const VALUE_TIER_MID: &str = "mid";
pub(crate) const VALUE_TIER_LOW: &str = "low";

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

/// G6 客户累计价值（LTV）：已核实正向成交 `amount` 之和 - reversal 之和（单币种 CNY，分）。
///
/// **红线守点（§2.1）**：复用 [`verification_drives_entitlement`] 闭集——只有
/// `staff_confirmed` / `payment_verified` 计入，`conversation_inferred`（AI 疑似成交）排除。
///
/// 单币种假设（用户拍板 RMB）：只累加 `currency==CNY`（或未设 currency 视为默认 CNY）的
/// 金额，非 CNY 成交跳过（多币种归一留后续增强）。`amount` 缺失的成交跳过（无金额事件不计）。
/// reversal（退款/撤单）按反向扣减；超额退款用 `.max(0)` clamp，不出现负价值。
///
/// **零扰动**：空 outcome_events / 无已核实成交（情感陪伴域无产品）→ 0。
pub(crate) fn compute_customer_value_cents(outcome_events: &[OutcomeEvent]) -> i64 {
    outcome_events
        .iter()
        .filter(|ev| verification_drives_entitlement(&ev.verification))
        .filter(|ev| ev.currency.as_deref().map_or(true, |c| c == "CNY"))
        .filter_map(|ev| {
            ev.amount
                .map(|a| if ev.event_kind == "reversal" { -a } else { a })
        })
        .sum::<i64>()
        .max(0)
}

/// G6 累计成交额分层：`>= high_threshold` → `high`；`>= mid_threshold` → `mid`；否则 `low`。
/// 零成交（value==0）→ `low`（与未购买一致，不单列）。canonical 取值与 m023 seed 一致。
///
/// 自我保护：若运维误配 `mid > high`，用 min/max 归一（high 门槛恒 ≥ mid 门槛），避免
/// `>=high` 先判吞掉整个 mid 档（误配时 mid 档静默失效）。正常配置 min/max 是恒等变换。
pub(crate) fn classify_value_tier(
    value_cents: i64,
    mid_threshold: i64,
    high_threshold: i64,
) -> &'static str {
    let lo = mid_threshold.min(high_threshold);
    let hi = mid_threshold.max(high_threshold);
    if value_cents >= hi {
        VALUE_TIER_HIGH
    } else if value_cents >= lo {
        VALUE_TIER_MID
    } else {
        VALUE_TIER_LOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{doc, DateTime, Document};

    use crate::models::OutcomeProductRef;

    fn ev(
        verification: &str,
        product: Option<(&str, &str, u32)>,
        occurred_ms: i64,
    ) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(occurred_ms),
            occurred_at: Some(DateTime::from_millis(occurred_ms)),
            amount: Some(19900),
            currency: Some("CNY".to_string()),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: product.map(|(pid, name, qty)| OutcomeProductRef {
                product_id: pid.to_string(),
                name: name.to_string(),
                unit_price: Some(19900),
                sku: None,
                quantity: qty,
                entitlement_days: None,
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
            price: Some(19900),
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
        let events = vec![ev(
            "conversation_inferred",
            Some(("p1", "疑似买的", 1)),
            100,
        )];
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
        assert_eq!(
            ents[0].in_aftercare, None,
            "无 entitlement_days 规则 → 无时效"
        );
    }

    #[test]
    fn entitlement_days_drives_in_aftercare_window() {
        let day = 86_400_000i64;
        let bought_at = DateTime::from_millis(0);
        // entitlement_days=30：第 10 天仍在期内，第 40 天已过期。
        let prod = product("course", "训练营", doc! { "entitlement_days": 30i64 });
        let events = vec![ev("staff_confirmed", Some(("course", "训练营", 1)), 0)];

        let (within, _) = project_entitlements(
            &events,
            &[prod.clone()],
            DateTime::from_millis(10 * day),
            10,
        );
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

    /// 构造一条带 `entitlement_days` 快照的正向成交事件（G4 #4）。
    fn ev_with_days(pid: &str, name: &str, occurred_ms: i64, days: Option<i64>) -> OutcomeEvent {
        let mut e = ev("staff_confirmed", Some((pid, name, 1)), occurred_ms);
        e.product_ref.as_mut().unwrap().entitlement_days = days;
        e
    }

    #[test]
    fn snapshot_entitlement_days_survives_product_archived() {
        let day = 86_400_000i64;
        // 成交时冻结售后期 30 天的快照；active_products 传空模拟产品已 archived/删除。
        let events = vec![ev_with_days("course", "训练营", 0, Some(30))];

        let (within, _) = project_entitlements(&events, &[], DateTime::from_millis(10 * day), 10);
        assert_eq!(
            within[0].in_aftercare,
            Some(true),
            "产品 archived 后，快照售后期仍让第10天判在期内（#4 核心）"
        );
        assert_eq!(
            within[0].expires_at.map(|d| d.timestamp_millis()),
            Some(30 * day),
            "expires_at 按快照 30 天算出"
        );

        let (expired, _) = project_entitlements(&events, &[], DateTime::from_millis(40 * day), 10);
        assert_eq!(expired[0].in_aftercare, Some(false), "第40天超快照售后期");
    }

    #[test]
    fn snapshot_days_takes_priority_over_active_product() {
        let day = 86_400_000i64;
        // 快照冻结 30 天；活产品表后来改成 7 天。投影必须用快照 30 天（成交当时口径），
        // 否则改产品配置会回溯篡改历史客户的售后期判定。
        let events = vec![ev_with_days("course", "训练营", 0, Some(30))];
        let shrunk = product("course", "训练营", doc! { "entitlement_days": 7i64 });
        let (ents, _) =
            project_entitlements(&events, &[shrunk], DateTime::from_millis(10 * day), 10);
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(30 * day),
            "快照优先于活产品表（成交时冻结，不被后续改配置回溯）"
        );
    }

    #[test]
    fn missing_snapshot_days_falls_back_to_active_product() {
        let day = 86_400_000i64;
        // 快照无 days（未登记的老成交）→ 回落活产品表的 entitlement_days。
        let events = vec![ev_with_days("course", "训练营", 0, None)];
        let prod = product("course", "训练营", doc! { "entitlement_days": 30i64 });
        let (ents, _) = project_entitlements(&events, &[prod], DateTime::from_millis(10 * day), 10);
        assert_eq!(
            ents[0].in_aftercare,
            Some(true),
            "快照缺失时回落活产品表（回落路径不回归）"
        );
    }

    #[test]
    fn renewal_extends_aftercare_window_from_latest_deal() {
        // A 修复核心：同一产品续费 → 售后窗各笔独立续、取最晚到期，而非锁死首购。
        let day = 86_400_000i64;
        // 首购 Day0（30天）+ 续费 Day25（30天）。合理售后应到 Day55（25+30），非 Day30。
        let events = vec![
            ev_with_days("course", "训练营", 0, Some(30)),
            ev_with_days("course", "训练营", 25 * day, Some(30)),
        ];
        // owned_since 仍取最早（资历语义不变）。
        let (ents, _) = project_entitlements(&events, &[], DateTime::from_millis(35 * day), 10);
        assert_eq!(
            ents[0].owned_since.timestamp_millis(),
            0,
            "资历仍记首购 Day0"
        );
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(55 * day),
            "售后到期取最晚锚 = 续费Day25 + 30天"
        );
        assert_eq!(
            ents[0].in_aftercare,
            Some(true),
            "Day35 续费客户仍在售后期（修复前会误判 false：按首购Day0+30=Day30 已过）"
        );
    }

    #[test]
    fn renewal_with_different_days_takes_max_expiry() {
        // 各笔 days 不同（先短后长 / 先长后短）→ 一律取 max(occurred+days)。
        let day = 86_400_000i64;
        // 首购 Day0（90天，到Day90）+ 续费 Day10（30天，到Day40）→ max=Day90。
        let events = vec![
            ev_with_days("course", "训练营", 0, Some(90)),
            ev_with_days("course", "训练营", 10 * day, Some(30)),
        ];
        let (ents, _) = project_entitlements(&events, &[], DateTime::from_millis(50 * day), 10);
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(90 * day),
            "取最晚到期锚（首购90天 Day90 > 续费30天 Day40）"
        );
    }

    #[test]
    fn reversal_does_not_extend_aftercare_window() {
        // reversal 不是购买时刻 → 不贡献售后到期锚（只抵消净件数）。
        let day = 86_400_000i64;
        // 买2件 Day0（30天，到Day30）+ 退1件 Day40（净1件仍持有）。
        // reversal 不延长售后 → 到期仍 Day30，不会因 reversal occurred=Day40 续到 Day70。
        let deal = {
            let mut e = ev("staff_confirmed", Some(("course", "训练营", 2)), 0);
            e.product_ref.as_mut().unwrap().entitlement_days = Some(30);
            e
        };
        let refund = {
            let mut e = ev("staff_confirmed", Some(("course", "训练营", 1)), 40 * day);
            e.product_ref.as_mut().unwrap().entitlement_days = Some(30);
            e.event_kind = "reversal".to_string();
            e
        };
        let (ents, _) =
            project_entitlements(&[deal, refund], &[], DateTime::from_millis(35 * day), 10);
        assert_eq!(ents[0].quantity, 1, "净件数 = 2 - 1");
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(30 * day),
            "reversal 的 occurred 不延长售后窗（到期仍 Day30，非 Day70）"
        );
    }

    #[test]
    fn renewal_anchor_without_days_and_archived_product_drops_that_window() {
        // 边界（P2 已知降级语义契约）：续费笔快照缺 days 且产品已 archived（active 表查不到）
        // → 该锚 days 无从确定，被保守丢弃，只剩有快照的首购锚。
        // "无法确定 days 时不延窗"是安全选择（宁可漏判售后，不可凭空延长）。全新项目新成交
        // 都带快照，此组合仅理论存在；此测试把该降级行为钉成显式契约，防未来误改成"延窗"。
        let day = 86_400_000i64;
        let events = vec![
            ev_with_days("course", "训练营", 0, Some(30)), // 首购带快照 30 天 → 到 Day30
            ev_with_days("course", "训练营", 25 * day, None), // 续费缺快照
        ];
        // active_products 空 = 产品已 archived → 续费锚回落失败、被丢。
        let (ents, _) = project_entitlements(&events, &[], DateTime::from_millis(20 * day), 10);
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(30 * day),
            "缺 days 且 archived 的续费锚被丢，到期仅由首购快照锚决定"
        );
    }

    #[test]
    fn reversal_first_then_later_deal_expiry_anchors_only_positive() {
        // 边界：reversal 早于正向 deal 到达（异常时序，现实中退款不应早于购买）。
        // owned_since 是旧有行为（可能停在 reversal 时刻），但售后到期只认正向成交锚，
        // 不受 reversal occurred 影响 → expires 仍由正向 deal 决定。
        let day = 86_400_000i64;
        let refund_first = {
            let mut e = ev("staff_confirmed", Some(("course", "训练营", 1)), 5 * day);
            e.product_ref.as_mut().unwrap().entitlement_days = Some(30);
            e.event_kind = "reversal".to_string();
            e
        };
        // 两笔正向 deal（净件数 2-1=1>0 存活），occurred=Day10（到 Day40）。
        let deal_a = ev_with_days("course", "训练营", 10 * day, Some(30));
        let deal_b = ev_with_days("course", "训练营", 10 * day, Some(30));
        let (ents, _) = project_entitlements(
            &[refund_first, deal_a, deal_b],
            &[],
            DateTime::from_millis(20 * day),
            10,
        );
        assert_eq!(ents[0].quantity, 1, "净件数 = -1 + 1 + 1");
        assert_eq!(
            ents[0].expires_at.map(|d| d.timestamp_millis()),
            Some(40 * day),
            "售后到期只认正向成交锚（Day10+30），不受 reversal occurred=Day5 影响"
        );
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
                    unit_price: Some(19900),
                    sku: None,
                    quantity: 1,
                    entitlement_days: None,
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
        // 命名红线复核：不得出现禁词（此处用编译期拼接构造禁词值，避免源码
        // 字面量被禁词 lint 脚本反向命中——本断言意在验证 render 结果不含
        // 禁词，而非引入禁词）。
        let forbidden_words = [
            concat!("人", "工"),
            concat!("接", "管"),
            concat!("take", "over"),
            concat!("hand-", "off"),
            concat!("hand", "off"),
        ];
        for forbidden in forbidden_words {
            assert!(!g.contains(forbidden), "疑似成交指引不得含禁词 {forbidden}");
        }
    }

    #[test]
    fn relationship_type_suggestion_guidance_present_and_aligns_with_t6_contract() {
        let g = render_relationship_type_suggestion_guidance();
        assert!(!g.is_empty(), "引导段常驻，不应为空");
        // 与 T6 提取契约对齐：kind == relationship_type，字段 value/evidence/confidence。
        assert!(g.contains("relationship_type"), "kind 名须与 T6 提取一致");
        assert!(g.contains("agentGeneratedSignals"), "走弱信号通道");
        assert!(g.contains("\"value\""), "字段 value");
        assert!(g.contains("\"evidence\""), "字段 evidence");
        assert!(g.contains("\"confidence\""), "字段 confidence");
        // 通用方法论锚点：基于"关系性质"+"新证据"，且显式约束不要每轮臆测。
        assert!(g.contains("关系性质"), "引导基于关系性质新证据");
        assert!(g.contains("新证据"), "仅在明确新证据时产出");
        assert!(g.contains("不要每轮"), "关系类型是稳定属性，不每轮改判");
        // 反过拟合护栏：行业中性，不写死某行业判断规则。
        assert!(
            g.contains("字典"),
            "取值按本账号 relationship_type 字典，保持行业中性"
        );
    }

    #[test]
    fn relationship_type_suggestion_guidance_forbidden_words_clean() {
        let g = render_relationship_type_suggestion_guidance();
        // 命名红线：render 结果不得含禁词（用编译期拼接构造禁词，避免源码字面量被 lint 反向命中）。
        let forbidden_words = [
            concat!("人", "工"),
            concat!("接", "管"),
            concat!("转", "真人"),
            concat!("take", "over"),
            concat!("hand-", "off"),
            concat!("hand", "off"),
        ];
        for forbidden in forbidden_words {
            assert!(
                !g.contains(forbidden),
                "关系类型建议引导不得含禁词 {forbidden}"
            );
        }
    }

    #[test]
    fn fmt_minor_as_major_renders_cents_as_yuan() {
        // 金额整数化命门：分→元两位小数。
        assert_eq!(fmt_minor_as_major(19900), "199.00");
        assert_eq!(fmt_minor_as_major(5), "0.05");
        assert_eq!(fmt_minor_as_major(0), "0.00");
        assert_eq!(fmt_minor_as_major(100), "1.00");
        assert_eq!(fmt_minor_as_major(199), "1.99");
        assert_eq!(fmt_minor_as_major(1000000), "10000.00");
    }

    #[test]
    fn product_catalog_prompt_renders_price_as_yuan_not_cents() {
        // 命门回归：catalog 注入 AI 的价格必须是「元」(199.00)，绝不能把分值 19900
        // 直接喂给 agent（否则报成 100 倍错价）。product() helper price=19900。
        let p = product("vip", "年度会员", Document::new());
        let catalog = format_product_catalog_for_prompt(&[p]);
        assert!(catalog.contains("199.00 CNY"), "价格须渲染成元：{catalog}");
        assert!(
            !catalog.contains("19900"),
            "绝不能把分值原样喂给 AI：{catalog}"
        );
    }

    #[test]
    fn transaction_facts_gate_off_yields_all_empty_even_with_products() {
        // G4 #5 闸门核心：非交易域（enabled=false）即便产品表非空 + 有真成交，三段一律空串。
        let now = DateTime::from_millis(1_000_000);
        let products = vec![product(
            "vip",
            "年度会员",
            doc! { "entitlement_days": 30i64 },
        )];
        let events = vec![ev("staff_confirmed", Some(("vip", "年度会员", 1)), 100)];
        let (catalog, ents, suspected) =
            render_transaction_facts_sections(false, &products, &events, now);
        assert_eq!(catalog, "", "非交易域不注入产品目录");
        assert_eq!(ents, "", "非交易域不注入持有投影");
        assert_eq!(suspected, "", "非交易域不注入疑似成交指引");
    }

    #[test]
    fn transaction_facts_gate_on_renders_three_sections() {
        // 交易域（enabled=true）+ 产品非空 + 有成交 → 三段都非空。
        let now = DateTime::from_millis(1_000_000);
        let products = vec![product(
            "vip",
            "年度会员",
            doc! { "entitlement_days": 30i64 },
        )];
        let events = vec![ev("staff_confirmed", Some(("vip", "年度会员", 1)), 100)];
        let (catalog, ents, suspected) =
            render_transaction_facts_sections(true, &products, &events, now);
        assert!(!catalog.is_empty(), "交易域注入产品目录");
        assert!(!ents.is_empty(), "交易域注入持有投影（有成交）");
        assert!(!suspected.is_empty(), "交易域注入疑似成交指引（有产品）");
    }

    #[test]
    fn transaction_facts_gate_on_empty_products_zero_perturbation() {
        // 交易域但产品表空（未配置）→ 三段仍空串，与无产品域零扰动一致。
        let now = DateTime::from_millis(1_000_000);
        let (catalog, ents, suspected) = render_transaction_facts_sections(true, &[], &[], now);
        assert_eq!(catalog, "");
        assert_eq!(ents, "");
        assert_eq!(suspected, "");
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
        assert_eq!(
            ents[0].owned_since.timestamp_millis(),
            100,
            "reversal 不刷新购买时刻"
        );
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
        let (ents, _) = project_entitlements(&[ev], &[], DateTime::from_millis(1_000_000), 10);
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
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_NOT_PURCHASED), &[]),
            None
        );
        assert_eq!(reconcile_g1_with_entitlements(None, &[]), None);
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_REPURCHASE), &[]),
            None
        );
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
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_PURCHASED), &ents),
            None
        );
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_AFTERCARE), &ents),
            None
        );
        assert_eq!(
            reconcile_g1_with_entitlements(Some(G1_REPURCHASE), &ents),
            None
        );
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
        assert_eq!(
            ms,
            vec![100, 200],
            "只取 staff_confirmed/payment_verified 的正向成交"
        );
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

    // ── G6 价值分层：compute_customer_value_cents + classify_value_tier ──

    /// 构造带指定金额/币种/事件类型的成交事件（compute_customer_value 测试用）。
    fn ev_money(
        verification: &str,
        amount: Option<i64>,
        currency: Option<&str>,
        event_kind: &str,
    ) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(0),
            occurred_at: Some(DateTime::from_millis(0)),
            amount,
            currency: currency.map(ToString::to_string),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: None,
            event_kind: event_kind.to_string(),
        }
    }

    #[test]
    fn customer_value_sums_confirmed_deals_minus_reversals() {
        let events = vec![
            ev_money("staff_confirmed", Some(30000), Some("CNY"), "deal"),
            ev_money("payment_verified", Some(20000), Some("CNY"), "deal"),
            ev_money("staff_confirmed", Some(10000), Some("CNY"), "reversal"),
        ];
        // 30000 + 20000 - 10000 = 40000
        assert_eq!(compute_customer_value_cents(&events), 40000);
    }

    #[test]
    fn customer_value_excludes_conversation_inferred() {
        let events = vec![
            ev_money("staff_confirmed", Some(30000), Some("CNY"), "deal"),
            // conversation_inferred 是 AI 疑似成交，§2.1 红线排除，不计入 LTV。
            ev_money("conversation_inferred", Some(99999), Some("CNY"), "deal"),
        ];
        assert_eq!(compute_customer_value_cents(&events), 30000);
    }

    #[test]
    fn customer_value_skips_non_cny_and_missing_amount() {
        let events = vec![
            ev_money("staff_confirmed", Some(30000), Some("CNY"), "deal"),
            // 非 CNY 单币种假设跳过。
            ev_money("staff_confirmed", Some(50000), Some("USD"), "deal"),
            // amount 缺失跳过。
            ev_money("staff_confirmed", None, Some("CNY"), "deal"),
            // 未设 currency 视为默认 CNY，计入。
            ev_money("staff_confirmed", Some(5000), None, "deal"),
        ];
        assert_eq!(compute_customer_value_cents(&events), 35000);
    }

    #[test]
    fn customer_value_clamps_overrefund_to_zero() {
        let events = vec![
            ev_money("staff_confirmed", Some(10000), Some("CNY"), "deal"),
            ev_money("staff_confirmed", Some(30000), Some("CNY"), "reversal"),
        ];
        // 10000 - 30000 = -20000 → clamp 0（不出现负价值）。
        assert_eq!(compute_customer_value_cents(&events), 0);
    }

    #[test]
    fn customer_value_empty_is_zero() {
        assert_eq!(compute_customer_value_cents(&[]), 0);
    }

    #[test]
    fn classify_value_tier_boundaries() {
        let (mid, high) = (50000_i64, 300000_i64);
        // 边界：< mid → low；== mid → mid；mid<v<high → mid；== high → high；> high → high。
        assert_eq!(classify_value_tier(0, mid, high), VALUE_TIER_LOW);
        assert_eq!(classify_value_tier(49999, mid, high), VALUE_TIER_LOW);
        assert_eq!(classify_value_tier(50000, mid, high), VALUE_TIER_MID);
        assert_eq!(classify_value_tier(299999, mid, high), VALUE_TIER_MID);
        assert_eq!(classify_value_tier(300000, mid, high), VALUE_TIER_HIGH);
        assert_eq!(classify_value_tier(500000, mid, high), VALUE_TIER_HIGH);
    }

    /// 误配防护：mid > high（运维配反）时用 min/max 归一，mid 档仍可达、不被静默吞掉。
    #[test]
    fn classify_value_tier_handles_misconfigured_thresholds() {
        // 配反：mid=400000 > high=300000。归一后 lo=300000, hi=400000。
        assert_eq!(classify_value_tier(250000, 400000, 300000), VALUE_TIER_LOW);
        assert_eq!(
            classify_value_tier(350000, 400000, 300000),
            VALUE_TIER_MID,
            "mid 档仍可达，未被吞"
        );
        assert_eq!(classify_value_tier(450000, 400000, 300000), VALUE_TIER_HIGH);
    }
}
