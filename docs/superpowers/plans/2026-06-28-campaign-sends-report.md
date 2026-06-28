# 活动推送结果查询（漏推可观测性）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增只读端点 `GET /api/campaigns/:id/sends`，把散落在 `campaign_sends` + `agent_run_logs` 的推送真相聚合成 7 桶分布（sent/pending/blocked/canceled/escalated/skipped/unknown）+ 每人明细，让运营能看到活动真实触达率。

**Architecture:** 纯聚合查询，零写链路改动。3 次固定 Mongo 查询（campaign_sends → agent_run_logs `$in` → contacts `$in`），通过关联键 `agent_run_logs.source_event_id == taskId.hex AND source_kind == follow_up_task` 拿到每人最新一次 run 的 `status`（是否进发送/被拦原因）+ `outbox_status`（是否真送达），交给两个纯函数分类 + 聚合。

**Tech Stack:** Rust 2021 / Axum / MongoDB (mongodb crate, BSON `doc!`)。

## Global Constraints

- **混合大小写 BSON key（核实强制）**：查 `campaign_sends` 用 camelCase（`campaignId`/`workspaceId`）；查 `agent_run_logs` 用 snake_case（`source_event_id`/`source_kind`）；查 `contacts` 用 snake_case（`workspace_id`/`account_id`/`wxid`）。
- **关联键常量**：`source_kind` 匹配值用常量 `crate::agent::run_envelope::SOURCE_KIND_FOLLOW_UP_TASK`（= `"follow_up_task"`，run_envelope.rs:43），**不硬编码字符串**。
- **IDOR 红线**：所有 `campaigns` / `campaign_sends` 的 filter 必含 `workspaceId = admin.current_workspace`（沿用 campaigns.rs:10 红线）。
- **命名红线（CI 硬门 check-no-human-takeover，扫 src/routes/ 新增行）**：禁 `人工|接管|takeover|hand-off|人工介入|人工托管`。本端点用技术词 sent/blocked/canceled，天然安全；注释也不得引入禁词。
- **零写链路改动**：不碰 `campaign_sends`/`campaigns` 的任何 insert/update，不碰 gateway/worker/model。本计划只新增读路径。
- **桶全集闭合**：`sent / pending / blocked / canceled / escalated / skipped / unknown` 七桶，贯穿分类函数、聚合函数、测试三处一致。
- **基线**：分支 `feat/campaign-sends-report` ← `origin/main` d615bdc。基线门 `cargo test --lib` ≥350/0；本地共享 target 可能被并行会话污染，全量基线以 CI 单分支 checkout 为准。

---

## File Structure

- **Modify `src/routes/campaigns.rs`**：
  - import 增补 `AgentRunLog`（现 import 块 :14-17 无此项）+ 常量 `SOURCE_KIND_FOLLOW_UP_TASK`。
  - 新增纯函数 `classify_send_outcome(send_status, run_log: Option<&Document>) -> (&'static str, Option<String>)`。
  - 新增纯函数 `build_sends_summary(items: &[Value]) -> Value`。
  - 新增 handler `campaign_sends_report(State, Extension<AuthenticatedAdmin>, Path<String>) -> AppResult<Json<Value>>`。
  - `mod tests` 内追加纯函数单测。
- **Modify `src/routes/mod.rs`**：`use campaigns::{...}` 增 `campaign_sends_report`；route 表加 `GET /campaigns/:id/sends`。

任务划分：Task 1 = 两个纯函数 + 全部纯函数单测（可独立测、是质量主战场）；Task 2 = handler + 路由接线（依赖 Task 1 的函数签名）。

---

### Task 1: 分类与聚合纯函数 + 单测

**Files:**
- Modify: `src/routes/campaigns.rs`（import 块 :14-17；新增两函数；`mod tests` :379 内追加测试）

**Interfaces:**
- Produces:
  - `fn classify_send_outcome(send_status: &str, run_log: Option<&Document>) -> (&'static str, Option<String>)` — 返回 `(bucket, reason)`，bucket ∈ {sent,pending,blocked,canceled,escalated,skipped,unknown}，reason 为原始底层值（仅 blocked/canceled/escalated/unknown/部分 pending 带）。
  - `fn build_sends_summary(items: &[Value]) -> Value` — items 每条形如 `{"contactWxid","name","status","reason"?}`，返回 `{"targetCount", "sent", "pending", "skipped", "unknown", "blocked": {reason:count}, "canceled": {reason:count}, "escalated": {reason:count}}`。
- Consumes: `mongodb::bson::{doc, Document}`（已 import :22）；`serde_json::{json, Value}`（已 import :24）。

- [ ] **Step 1: 增补 import**

把 `src/routes/campaigns.rs:14-17` 的 import 块改为（增 `AgentRunLog`）：

```rust
use crate::models::{
    assert_agent_task_status_valid, assert_campaign_status_valid, AgentRunLog, AgentTask, Campaign,
    CampaignSend, Contact, Product, SegmentFilter,
};
```

在 `use super::AppState;`（:26）下方新增一行常量引入：

```rust
use crate::agent::run_envelope::SOURCE_KIND_FOLLOW_UP_TASK;
```

> 说明：`AgentRunLog` 供 Task 2 的 db accessor `agent_run_logs()` 返回类型用；常量供查询 filter 用。Task 1 本身只用 `Document`，但一次性补齐 import 避免 Task 2 再动 import 块。

- [ ] **Step 2: 写分类纯函数（在文件末尾 `#[cfg(test)]` 之前插入）**

```rust
/// 把一条 campaign_send 的真实推送结果归桶。输入 = 台账 status + 关联到的最新
/// agent_run_log（None 表示 task 还没被 worker 跑到 / 无关联 run log）。
/// 桶：sent/pending/blocked/canceled/escalated/skipped/unknown。优先级自上而下命中即停：
/// outbox_status=sent（真送达）先于 status 判定。run_log.status 取值是
/// GATEWAY_STATUS_VALUES 闭集（run_envelope.rs:86-135），逐值明确归桶。
/// escalated = 走请示通道交幕后领导裁决、待补料后 AI 会继续触达（非失败漏推），
/// 与 blocked（纯频控/硬约束、无后续）区分。详见设计 spec §5.3/§10。
pub(super) fn classify_send_outcome(
    send_status: &str,
    run_log: Option<&Document>,
) -> (&'static str, Option<String>) {
    // ① 去重命中：dispatch 当初就没建 task。
    if send_status == "skipped_duplicate" {
        return ("skipped", None);
    }
    // ② 有 taskId 但查不到 run log：task 还没被 worker 跑到。
    let Some(log) = run_log else {
        return ("pending", Some("not_yet_run".to_string()));
    };
    let outbox_status = log.get_str("outbox_status").ok();
    let run_status = log.get_str("status").ok();
    // ③ 真送达（最高优先级，先于一切 status 判定）。
    if outbox_status == Some("sent") {
        return ("sent", None);
    }
    // ④ outbox 终态失败/取消。
    if matches!(outbox_status, Some("failed_terminal") | Some("canceled")) {
        return ("canceled", outbox_status.map(str::to_string));
    }
    // ⑤ 进了发送队列、还没发出/发送中。
    if matches!(outbox_status, Some("pending") | Some("in_flight")) {
        return ("pending", None);
    }
    // ⑥ 按 run_log.status 归桶（GATEWAY_STATUS_VALUES 闭集逐值明确）。
    match run_status {
        // a. 放行/已入队/作息重排：会继续，视作在途。
        Some("allowed" | "outbox_enqueued" | "quiet_hours_deferred") => ("pending", None),
        // b. 频控/硬约束/改写失败——没发出且无后续。
        Some(s @ ("daily_limit" | "cooldown" | "rate_limited"
            | "policy_cooldown" | "policy_wait_user_reply" | "policy_consecutive_limit"
            | "blocked_by_required_field" | "blocked_by_budget"
            | "review_blocked" | "revision_failed" | "revision_skipped_invalid_direction"
            | "revision_skipped_budget_exceeded" | "revision_llm_failure"
            | "tool_loop_timeout")) => ("blocked", Some(s.to_string())),
        // c. 已转交幕后领导请示，待裁决后 AI 会继续触达（非失败漏推）。
        Some(s @ ("blocked_unverified_product_claim" | "blocked_by_safety_guard"
            | "held_by_ai_policy" | "ai_waiting_for_more_context")) => {
            ("escalated", Some(s.to_string()))
        }
        // d. 取消（无后续）。
        Some(s @ ("context_changed" | "expired" | "not_managed"
            | "no_reply" | "admin_cancelled" | "superseded_by_new_inbound")) => {
            ("canceled", Some(s.to_string()))
        }
        // e. 灰度/口径态 / 不认识的值：诚实标 unknown，绝不强划进 sent。
        Some(other) => ("unknown", Some(other.to_string())),
        None => ("unknown", None),
    }
}
```

- [ ] **Step 3: 写聚合纯函数（紧接其后）**

```rust
/// 把每人明细 items 聚合成 summary。sent/pending/skipped/unknown 标量计数，
/// blocked/canceled/escalated 按 reason 二级 map 计数。targetCount = items 总数。
pub(super) fn build_sends_summary(items: &[Value]) -> Value {
    use serde_json::Map;
    let (mut sent, mut pending, mut skipped, mut unknown) = (0i64, 0i64, 0i64, 0i64);
    let mut blocked: Map<String, Value> = Map::new();
    let mut canceled: Map<String, Value> = Map::new();
    let mut escalated: Map<String, Value> = Map::new();
    for it in items {
        let status = it.get("status").and_then(Value::as_str).unwrap_or("unknown");
        let reason = it.get("reason").and_then(Value::as_str);
        match status {
            "sent" => sent += 1,
            "pending" => pending += 1,
            "skipped" => skipped += 1,
            "blocked" => bump(&mut blocked, reason.unwrap_or("unknown")),
            "canceled" => bump(&mut canceled, reason.unwrap_or("unknown")),
            "escalated" => bump(&mut escalated, reason.unwrap_or("unknown")),
            _ => unknown += 1,
        }
    }
    json!({
        "targetCount": items.len() as i64,
        "sent": sent,
        "pending": pending,
        "skipped": skipped,
        "unknown": unknown,
        "blocked": Value::Object(blocked),
        "canceled": Value::Object(canceled),
        "escalated": Value::Object(escalated),
    })
}

/// reason 二级计数自增。
fn bump(map: &mut serde_json::Map<String, Value>, reason: &str) {
    let n = map.get(reason).and_then(Value::as_i64).unwrap_or(0);
    map.insert(reason.to_string(), json!(n + 1));
}
```

- [ ] **Step 4: 写失败测试（`mod tests` 内追加，:379 `mod tests` 块末尾、最后一个 `}` 之前）**

```rust
    fn run_log(status: &str, outbox_status: Option<&str>) -> Document {
        let mut d = doc! { "status": status };
        if let Some(o) = outbox_status {
            d.insert("outbox_status", o);
        }
        d
    }

    #[test]
    fn classify_covers_all_buckets_and_priority() {
        // ① 去重
        assert_eq!(classify_send_outcome("skipped_duplicate", None), ("skipped", None));
        // ② 有 task 无 run log
        assert_eq!(
            classify_send_outcome("enqueued", None),
            ("pending", Some("not_yet_run".to_string()))
        );
        // ③ 真送达
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("sent")))),
            ("sent", None)
        );
        // ③优先级：outbox=sent 时即便 status 非 allowed 也归 sent（命中即停）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", Some("sent")))),
            ("sent", None)
        );
        // ③优先级关键：outbox=sent 压过 escalated 类 status（已送达优先于请示）
        assert_eq!(
            classify_send_outcome(
                "enqueued",
                Some(&run_log("blocked_unverified_product_claim", Some("sent")))
            ),
            ("sent", None)
        );
        // ④ outbox 终态失败
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("failed_terminal")))),
            ("canceled", Some("failed_terminal".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("canceled")))),
            ("canceled", Some("canceled".to_string()))
        );
        // ⑤ 在途（outbox pending / in_flight）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", Some("pending")))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", Some("in_flight")))),
            ("pending", None)
        );
        // ⑥a 放行/已入队/作息重排 → pending（会继续）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("allowed", None))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("outbox_enqueued", None))),
            ("pending", None)
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("quiet_hours_deferred", None))),
            ("pending", None)
        );
        // ⑥b 频控/硬约束/改写失败 → blocked，原因保留
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("daily_limit", None))),
            ("blocked", Some("daily_limit".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("policy_wait_user_reply", None))),
            ("blocked", Some("policy_wait_user_reply".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_by_required_field", None))),
            ("blocked", Some("blocked_by_required_field".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("revision_failed", None))),
            ("blocked", Some("revision_failed".to_string()))
        );
        // ⑥c 请示通道（escalated）：产品红线/安全门/AI策略/等上下文，原因保留
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_unverified_product_claim", None))),
            ("escalated", Some("blocked_unverified_product_claim".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("blocked_by_safety_guard", None))),
            ("escalated", Some("blocked_by_safety_guard".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("held_by_ai_policy", None))),
            ("escalated", Some("held_by_ai_policy".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("ai_waiting_for_more_context", None))),
            ("escalated", Some("ai_waiting_for_more_context".to_string()))
        );
        // ⑥d 取消（run status）
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("context_changed", None))),
            ("canceled", Some("context_changed".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("no_reply", None))),
            ("canceled", Some("no_reply".to_string()))
        );
        // ⑥e 灰度/口径态 / 不认识的 status → unknown
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("weird_new_status", None))),
            ("unknown", Some("weird_new_status".to_string()))
        );
        assert_eq!(
            classify_send_outcome("enqueued", Some(&run_log("precheck_blocked", None))),
            ("unknown", Some("precheck_blocked".to_string()))
        );
        // ⑦ run log 有但 status 字段缺失
        assert_eq!(
            classify_send_outcome("enqueued", Some(&Document::new())),
            ("unknown", None)
        );
    }

    #[test]
    fn summary_counts_scalars_and_reason_submaps() {
        let items = vec![
            json!({ "contactWxid": "a", "name": "甲", "status": "sent" }),
            json!({ "contactWxid": "b", "name": "乙", "status": "sent" }),
            json!({ "contactWxid": "c", "name": "丙", "status": "pending" }),
            json!({ "contactWxid": "d", "name": "丁", "status": "skipped" }),
            json!({ "contactWxid": "e", "name": "戊", "status": "blocked", "reason": "daily_limit" }),
            json!({ "contactWxid": "f", "name": "己", "status": "blocked", "reason": "daily_limit" }),
            json!({ "contactWxid": "g", "name": "庚", "status": "blocked", "reason": "cooldown" }),
            json!({ "contactWxid": "h", "name": "辛", "status": "canceled", "reason": "context_changed" }),
            json!({ "contactWxid": "j", "name": "癸", "status": "escalated", "reason": "blocked_unverified_product_claim" }),
            json!({ "contactWxid": "k", "name": "子", "status": "escalated", "reason": "blocked_unverified_product_claim" }),
            json!({ "contactWxid": "l", "name": "丑", "status": "escalated", "reason": "held_by_ai_policy" }),
            json!({ "contactWxid": "i", "name": "壬", "status": "unknown" }),
        ];
        let s = build_sends_summary(&items);
        assert_eq!(s["targetCount"], json!(12));
        assert_eq!(s["sent"], json!(2));
        assert_eq!(s["pending"], json!(1));
        assert_eq!(s["skipped"], json!(1));
        assert_eq!(s["unknown"], json!(1));
        assert_eq!(s["blocked"]["daily_limit"], json!(2));
        assert_eq!(s["blocked"]["cooldown"], json!(1));
        assert_eq!(s["canceled"]["context_changed"], json!(1));
        assert_eq!(s["escalated"]["blocked_unverified_product_claim"], json!(2));
        assert_eq!(s["escalated"]["held_by_ai_policy"], json!(1));
    }

    #[test]
    fn summary_empty_items_all_zero() {
        let s = build_sends_summary(&[]);
        assert_eq!(s["targetCount"], json!(0));
        assert_eq!(s["sent"], json!(0));
        assert_eq!(s["pending"], json!(0));
        assert_eq!(s["skipped"], json!(0));
        assert_eq!(s["unknown"], json!(0));
        assert_eq!(s["blocked"], json!({}));
        assert_eq!(s["canceled"], json!({}));
        assert_eq!(s["escalated"], json!({}));
    }
```

- [ ] **Step 5: 运行测试确认通过 + 编译**

```
cargo test --lib routes::campaigns::tests::classify_covers_all_buckets_and_priority routes::campaigns::tests::summary_counts_scalars_and_reason_submaps routes::campaigns::tests::summary_empty_items_all_zero 2>&1 | tail -15
cargo check --lib 2>&1 | tail -5
```
Expected: 3 测试 PASS + 编译通过。

> 本地共享 target 若被并行会话污染导致 `0 tests filtered out`，先 `touch src/lib.rs` 强制 relink 后重跑（参见活动定向推送交付经验）。

- [ ] **Step 6: 提交**

```bash
git add src/routes/campaigns.rs
git commit -m "feat(campaign): 推送结果分类/聚合纯函数(classify_send_outcome/build_sends_summary)+单测"
```

---

### Task 2: 聚合查询 handler + 路由接线

**Files:**
- Modify: `src/routes/campaigns.rs`（新增 `campaign_sends_report` handler，放在 `dispatch_campaign` 之后、`#[cfg(test)]` 之前）
- Modify: `src/routes/mod.rs`（:257 `use campaigns::{...}` 增项；:784 route 表加一行）

**Interfaces:**
- Consumes: `classify_send_outcome` / `build_sends_summary`（Task 1）；db accessors `state.db.campaigns()` / `campaign_sends()` / `agent_run_logs()` / `contacts()`（均已存在 db/mod.rs:391/391/179/74）；`AuthenticatedAdmin.current_workspace`（campaigns.rs 既有用法）。
- Produces: `pub async fn campaign_sends_report(State<AppState>, Extension<AuthenticatedAdmin>, Path<String>) -> AppResult<Json<Value>>`，供 mod.rs 路由注册。

- [ ] **Step 1: 写 handler（在 `dispatch_campaign` 结束的 `}` 之后、`#[cfg(test)]` 之前插入）**

```rust
/// GET /campaigns/:id/sends —— 活动推送结果聚合（只读）。
/// 把 campaign_sends 台账与 agent_run_logs（关联键 source_event_id=taskId.hex）
/// 聚合成 7 桶分布 + 每人明细。零写入。IDOR：filter 含 workspaceId。
pub async fn campaign_sends_report(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("非法 campaign id".to_string()))?;
    // IDOR：先核实活动归属本 workspace。
    let campaign = state
        .db
        .campaigns()
        .find_one(doc! { "_id": oid, "workspaceId": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("campaign not found".to_string()))?;

    // 1) 台账（已有唯一索引 (campaignId, contactWxid)）。
    let sends: Vec<CampaignSend> = state
        .db
        .campaign_sends()
        .find(
            doc! { "campaignId": oid, "workspaceId": &admin.current_workspace },
            None,
        )
        .await?
        .try_collect()
        .await?;

    // 2) 批量拉 run log：taskId.hex 集合 → 一次 $in，内存按 source_event_id 取最新（max _id）。
    let task_hexes: Vec<String> = sends
        .iter()
        .filter_map(|s| s.task_id.map(|t| t.to_hex()))
        .collect();
    let mut latest_run: std::collections::HashMap<String, AgentRunLog> = std::collections::HashMap::new();
    if !task_hexes.is_empty() {
        let logs: Vec<AgentRunLog> = state
            .db
            .agent_run_logs()
            .find(
                doc! {
                    "source_event_id": { "$in": &task_hexes },
                    "source_kind": SOURCE_KIND_FOLLOW_UP_TASK,
                },
                None,
            )
            .await?
            .try_collect()
            .await?;
        for log in logs {
            let key = log.source_event_id.clone();
            // 同一 task 多条（retry）取 _id 最大那条 = 最新一次 run。
            match latest_run.get(&key) {
                Some(prev) if prev.id >= log.id => {}
                _ => {
                    latest_run.insert(key, log);
                }
            }
        }
    }

    // 3) 批量补客户名。
    let wxids: Vec<&String> = sends.iter().map(|s| &s.contact_wxid).collect();
    let mut name_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !wxids.is_empty() {
        let contacts: Vec<Contact> = state
            .db
            .contacts()
            .find(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &campaign.account_id,
                    "wxid": { "$in": &wxids },
                },
                None,
            )
            .await?
            .try_collect()
            .await?;
        for c in contacts {
            let name = c.remark.clone().or(c.nickname.clone()).unwrap_or_default();
            name_of.insert(c.wxid, name);
        }
    }

    // 4) 逐人分类 → items。
    let mut items: Vec<Value> = Vec::with_capacity(sends.len());
    for s in &sends {
        let run_doc = s
            .task_id
            .map(|t| t.to_hex())
            .and_then(|hex| latest_run.get(&hex))
            .and_then(|log| mongodb::bson::to_document(log).ok());
        let (bucket, reason) = classify_send_outcome(&s.status, run_doc.as_ref());
        let name = name_of.get(&s.contact_wxid).cloned().unwrap_or_default();
        let mut item = json!({
            "contactWxid": s.contact_wxid,
            "name": name,
            "status": bucket,
        });
        if let Some(r) = reason {
            item["reason"] = json!(r);
        }
        items.push(item);
    }

    let summary = build_sends_summary(&items);
    Ok(Json(json!({
        "campaignId": id,
        "title": campaign.title,
        "status": campaign.status,
        "summary": summary,
        "items": items,
    })))
}
```

> 实现者注意：`classify_send_outcome` 收 `Option<&Document>`，所以把 `AgentRunLog` 用 `mongodb::bson::to_document` 转回 doc 再传——这样分类函数只依赖 BSON 字段名（`status`/`outbox_status`），不耦合 struct（`outbox_status` 是 dispatcher 动态 `$set` 的字段，AgentRunLog struct 里通过 `#[serde(default)]` 兜底，to_document 后该 key 可能缺失，classify 已用 `get_str().ok()` 容错）。

- [ ] **Step 2: 路由接线（`src/routes/mod.rs`）**

把 :257 的 use 改为（增 `campaign_sends_report`）：

```rust
use campaigns::{campaign_sends_report, create_campaign, dispatch_campaign, preview_campaign};
```

在 :784 `.route("/campaigns/:id/dispatch", post(dispatch_campaign))` 之后加一行：

```rust
        .route("/campaigns/:id/sends", get(campaign_sends_report))
```

> 确认 `get` 已在 mod.rs 顶部从 `axum::routing` import（现有 GET 路由必已引入；若编译报 `get` 未找到，在 `use axum::routing::{...post...}` 里补 `get`）。

- [ ] **Step 3: 编译验证**

```
cargo check --lib 2>&1 | tail -8
```
Expected: 编译通过（0 error）。重点排查：`AgentRunLog` 是否实现 Serialize（to_document 需要）—— 它派生 `Serialize`（models.rs），OK；`get` 是否已 import。

- [ ] **Step 4: no-human-takeover lint**

```
bash scripts/check-no-human-takeover.sh 2>&1 | tail -5
```
Expected: 0 violations。

- [ ] **Step 5: 提交**

```bash
git add src/routes/campaigns.rs src/routes/mod.rs
git commit -m "feat(campaign): GET /campaigns/:id/sends 推送结果聚合端点+路由"
```

---

### Task 3: 基线门 + 收口

**Files:** 无新增（验证性任务）

- [ ] **Step 1: 全量 lib 编译 + 测试目标编译（CI baseline step2 复刻）**

```
cargo check --lib --tests 2>&1 | tail -5
```
Expected: 编译通过。

- [ ] **Step 2: 跑本功能相关测试**

```
touch src/lib.rs && cargo test --lib campaign 2>&1 | grep -E "test result|classify|summary|campaign_tools|build_follow_up" | tail -20
```
Expected: 本任务 3 个新测试 + 既有 campaign 测试全绿（`touch` 规避共享 target 污染）。

- [ ] **Step 3: lint 复核**

```
bash scripts/check-no-human-takeover.sh 2>&1 | tail -5
```
Expected: 0 violations。

> 全量 lib ≥350/0 基线以 CI 单分支 checkout 为准（本地共享 target 受并行会话污染，spec §Global Constraints 已注明）。本任务若无改动则不提交。

---

## 自审记录

**1. spec 覆盖**：
- §4 端点契约（GET、IDOR、400/404、response shape）→ Task 2 handler ✓
- §5.1 取数 3 查询 → Task 2 Step 1 ✓
- §5.2 混合大小写 key → Task 2 filter（campaignId/workspaceId camelCase；source_event_id/source_kind/workspace_id/wxid snake_case）✓ + Global Constraints ✓
- §5.3 7 桶分类 → Task 1 classify_send_outcome ✓
- §5.4 聚合 → Task 1 build_sends_summary ✓
- §6 边界（空台账/taskId None/retry 取最新/老 doc 缺字段/IDOR/命名红线）→ Task 1 测试（空 doc/不认识值）+ Task 2（空台账 items[]、retry max _id、IDOR filter）✓
- §8 测试 → Task 1 Step 4 三测试 ✓
- §7 范围（不做前端/不写台账/不分页）→ 计划无对应任务 ✓

**2. 占位符扫描**：无 TBD/TODO；每个 code step 给完整代码；测试有真实断言。

**3. 类型一致性**：`classify_send_outcome(&str, Option<&Document>) -> (&'static str, Option<String>)` Task 1 定义、Task 2 调用一致；`build_sends_summary(&[Value]) -> Value` 一致；handler 返回 `AppResult<Json<Value>>` 与 mod.rs `post`/`get` 注册兼容；`s.task_id: Option<ObjectId>`（CampaignSend models.rs:605）→ `.map(|t| t.to_hex())` 类型对；`log.source_event_id: String`（AgentRunLog models.rs:2686）→ HashMap key 对；`AgentRunLog.id: Option<ObjectId>` → `prev.id >= log.id`（Option<ObjectId> 实现 Ord，None 最小）取最新对。

**关键修正落实**：① source_kind 用常量 `SOURCE_KIND_FOLLOW_UP_TASK`（非裸串）；② outbox_status=sent 优先级先于 status（classify ③ 在 ⑤⑥ 之前，含 daily_limit+sent 归 sent 的命中即停测试）；③ classify 收 Document 而非 struct，对 dispatcher 动态字段 outbox_status 用 get_str().ok() 容错。
