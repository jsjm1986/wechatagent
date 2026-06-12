//! Strategic Planner — 自主发起扫描器集合。
//!
//! 周期性扫描 `agent_status="managed"` 联系人，对满足条件的对象 emit
//! `kind="follow_up"` 任务。所有发送动作仍由 task worker →
//! [`agent::run_user_operation_gateway`] 完成；本模块**绝不**直接调 MCP 发消息，
//! 也不绕过 review/revision/outbox。
//!
//! 三个串行段（每段独立 try，单段失败不阻断其它段）：
//! 1. **silent**（M1）：联系人 inbound 老于阈值；
//! 2. **commitment**（M2）：`Contact.commitments` 中存在 `due_at` 已过期 / 临近的条目；
//! 3. **stage_stagnation**（M2）：`customer_stage_updated_at` 老于阈值且非终状态。
//!
//! daily-cap 跨三段共享：`agent_events` 上反查所有 `strategic_planner_emit /
//! _commitment_overdue / _commitment_imminent / _stage_stagnation` kind 的当日条数。

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use tokio::time::sleep;

use crate::models::{AgentTask, CommitmentRepr, Contact};
use crate::routes::AppState;

/// 旧 `customer_stage` 字段已迁入 `Contact.domain_attributes`。这两个 helper 把
/// 读端集中起来，sales 旧库清理后所有 planner 排序/过滤都从 domain_attributes 读。
fn contact_customer_stage(contact: &Contact) -> Option<String> {
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()))
}

fn contact_intent_level(contact: &Contact) -> Option<String> {
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("intent_level").ok().map(|s| s.to_string()))
}

fn contact_customer_stage_updated_at(contact: &Contact) -> Option<DateTime> {
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_datetime("customer_stage_updated_at").ok().copied())
}

/// universal-domain-adaptation 1C：按配置的停滞维度读 `<dim>_updated_at` 计时戳。
/// DEFAULT dim="customer_stage" 时等价 [`contact_customer_stage_updated_at`]。
fn contact_stagnation_updated_at(contact: &Contact, dim: &str) -> Option<DateTime> {
    let key = format!("{dim}_updated_at");
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_datetime(key.as_str()).ok().copied())
}

/// 扫描器结束时输出的统计信息（写入 `*_tick` 事件 detail）。
#[derive(Debug, Default, Clone, Copy)]
struct ScanCounters {
    scanned: i64,
    emitted: i64,
}

/// 由 `main.rs` 在 `strategic_planner_enabled=true` 时通过 `tokio::spawn` 启动的
/// 长驻循环。失败一次只 `tracing::error!` 后等待下一个周期，避免 tick 内瞬时
/// MongoDB 抖动把整个 loop 撕掉。
pub async fn run_strategic_planner(state: AppState) {
    tracing::info!(
        interval_seconds = state.config.strategic_planner_interval_seconds,
        silent_threshold_hours = state.config.strategic_planner_silent_threshold_hours,
        daily_emit_cap = state.config.strategic_planner_daily_emit_cap,
        commitment_imminent_window_hours =
            state.config.strategic_planner_commitment_imminent_window_hours,
        stage_stagnation_threshold_days =
            state.config.strategic_planner_stage_stagnation_threshold_days,
        "strategic planner loop started"
    );
    loop {
        if let Err(error) = scan_silent(&state).await {
            tracing::error!(error = %error, "strategic planner silent scan failed");
        }
        if let Err(error) = scan_commitments(&state).await {
            tracing::error!(error = %error, "strategic planner commitment scan failed");
        }
        if let Err(error) = scan_stage_stagnation(&state).await {
            tracing::error!(error = %error, "strategic planner stage_stagnation scan failed");
        }
        sleep(Duration::from_secs(
            state.config.strategic_planner_interval_seconds,
        ))
        .await;
    }
}

/// 单 tick 入口（测试用 + 兼容旧调用）：跑完三个扫描器，任何一段失败短路返回。
///
/// 生产 loop（[`run_strategic_planner`]）逐段独立 try 以避免相互拖累；这里允许
/// 短路是因为测试需要"任何一段失败立即可见"的语义。
pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    scan_silent(state).await?;
    scan_commitments(state).await?;
    scan_stage_stagnation(state).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 共享：daily cap、emit helper、event 写入
// ---------------------------------------------------------------------------

/// 三个扫描器共用的 emit kind 集合，daily cap 反查时跨段汇总。
const EMIT_EVENT_KINDS: &[&str] = &[
    "strategic_planner_emit",
    "strategic_planner_commitment_overdue",
    "strategic_planner_commitment_imminent",
    "strategic_planner_stage_stagnation",
];

/// 当日已 emit 计数（跨三段汇总）。daily cap 在 tick 开始时一次性算余额。
async fn count_today_emit_events(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    now: DateTime,
) -> anyhow::Result<i64> {
    let day_start = day_start_before(now);
    let count = state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "kind": { "$in": EMIT_EVENT_KINDS },
                "created_at": { "$gte": day_start },
            },
            None,
        )
        .await?;
    Ok(count as i64)
}

/// 当日起点（以 epoch 整数小时近似 UTC 0 点；与 `tasks.rs::today_date_string`
/// 同款粗近似，足够 daily-cap 这种"按日计数"的语义）。
fn day_start_before(now: DateTime) -> DateTime {
    let now_ms = now.timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let day_index = now_ms / day_ms;
    DateTime::from_millis(day_index * day_ms)
}

/// 共享 emit helper：把"调用方拼好的 content"写成一条 follow_up 任务。
///
/// 三个扫描器都调它；M1 silent / M2 commitment / M2 stage_stagnation 各自负责
/// 拼出语义清晰的 `Planner: <reason> ...` content 前缀，Reply Agent 在执行 task
/// 时按 content 推断"为什么发起"。
async fn emit_planner_follow_up(
    state: &AppState,
    contact: &Contact,
    content: String,
    now: DateTime,
) -> anyhow::Result<()> {
    // 默认给 48 小时 expiry，与 `RuntimeParametersTyped::default().follow_up_expires_hours`
    // 对齐；Planner 不读 OperationDomainConfig，避免重建 runtime。
    let expires_hours: i64 = 48;
    let expires_at = DateTime::from_millis(now.timestamp_millis() + expires_hours * 60 * 60 * 1000);
    let task = AgentTask {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        kind: "follow_up".to_string(),
        run_at: now,
        expires_at: Some(expires_at),
        content,
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
    };
    state.db.tasks().insert_one(task, None).await?;
    Ok(())
}

async fn write_event(
    state: &AppState,
    account_id: &str,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> anyhow::Result<()> {
    crate::agent::write_event_for_account(
        state,
        account_id,
        contact_wxid,
        kind,
        status,
        summary,
        details,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

async fn has_pending_follow_up(state: &AppState, contact: &Contact) -> anyhow::Result<bool> {
    let count = state
        .db
        .tasks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": "follow_up",
                "status": { "$in": ["pending", "retry", "running"] },
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

/// 写一条 `*_capped` 事件并 break——剩余余额耗尽时短路。
async fn write_capped_event(
    state: &AppState,
    account_id: &str,
    counters: ScanCounters,
    daily_cap: i64,
    already_emitted_today: i64,
    segment: &str,
) -> anyhow::Result<()> {
    write_event(
        state,
        account_id,
        None,
        "strategic_planner_capped",
        "capped",
        &format!("today's emit cap reached during {segment} scan"),
        Some(doc! {
            "segment": segment,
            "scanned": counters.scanned,
            "emittedThisTick": counters.emitted,
            "dailyEmitCap": daily_cap,
            "alreadyEmittedToday": already_emitted_today,
        }),
    )
    .await
}

// ---------------------------------------------------------------------------
// 段 1：silent（M1）
// ---------------------------------------------------------------------------

async fn scan_silent(state: &AppState) -> anyhow::Result<()> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();
    let global_threshold_hours = state.config.strategic_planner_silent_threshold_hours;
    let threshold_ms = global_threshold_hours.saturating_mul(60 * 60 * 1000);
    let silent_before = DateTime::from_millis(now_ms - threshold_ms);
    // universal-domain-adaptation H8：每 tick 加载一次 active profile，取行业默认范式。
    // DB 仍按全局阈值粗筛（粗筛 = 全局阈值即所有可能候选的上界，按 contact override
    // 再收紧只会让候选更少，绝不会漏）；逐 contact 的 enabled 短路 + 阈值收紧在内存做。
    let profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await;
    let profile_mode = profile.operation_mode.clone();

    let filter = silent_candidate_filter(&workspace_id, &account_id, silent_before);
    let mut cursor = state.db.contacts().find(filter, None).await?;

    let daily_cap = state.config.strategic_planner_daily_emit_cap;
    let already_emitted_today =
        count_today_emit_events(state, &workspace_id, &account_id, now).await?;
    let mut remaining = daily_cap.saturating_sub(already_emitted_today);
    let mut counters = ScanCounters::default();

    while let Some(contact) = cursor.try_next().await? {
        counters.scanned += 1;
        if !silent_candidate_passes_in_memory(&contact) {
            continue;
        }
        // H8：解析有效范式。silence 关 → 该 contact 不走静默唤醒（陪伴/维护型也可能关）。
        let mode = resolve_operation_mode(&contact, &profile_mode);
        if !mode.silence.enabled {
            continue;
        }
        // H8：有效静默阈值 = override ?? profile ?? 全局 config。DEFAULT(None)→全局，
        // 与 DB 粗筛一致 → 此 in-memory 检查为恒真（金标零变化）；override 更长则收紧。
        let effective_threshold_hours = mode
            .silence
            .threshold_hours
            .unwrap_or(global_threshold_hours);
        if silent_hours_for(&contact, now_ms) < effective_threshold_hours {
            continue;
        }
        if has_pending_follow_up(state, &contact).await? {
            continue;
        }
        if remaining <= 0 {
            write_capped_event(
                state,
                &account_id,
                counters,
                daily_cap,
                already_emitted_today,
                "silent",
            )
            .await?;
            tracing::info!(
                scanned = counters.scanned,
                emitted = counters.emitted,
                already_emitted_today,
                "strategic planner hit daily cap (silent)"
            );
            break;
        }
        // M3 反馈环：block-rate 过高时跳过该 contact，写 backoff 事件，不消耗 daily cap。
        if let Some(payload) = should_skip_for_block_rate(state, &contact, now).await? {
            write_backoff_event(state, "silent", &contact, payload).await?;
            continue;
        }
        let last_inbound_repr = contact
            .last_inbound_at
            .map(|d| d.timestamp_millis().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        emit_planner_follow_up(
            state,
            &contact,
            format!("Planner: silent_follow_up since {last_inbound_repr}"),
            now,
        )
        .await?;
        let silent_hours = silent_hours_for(&contact, now_ms);
        write_event(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "strategic_planner_emit",
            "emitted",
            &format!("Planner: silent_follow_up emitted (silent {silent_hours}h)"),
            Some(doc! {
                "source": "strategic_planner",
                "silentHours": silent_hours,
                "lastInboundAt": contact
                    .last_inbound_at
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(0),
            }),
        )
        .await?;
        counters.emitted += 1;
        remaining -= 1;
    }

    write_event(
        state,
        &account_id,
        None,
        "strategic_planner_tick",
        "ok",
        &format!(
            "strategic planner tick: scanned {}, emitted {}",
            counters.scanned, counters.emitted
        ),
        Some(doc! {
            "scanned": counters.scanned,
            "emitted": counters.emitted,
            "dailyEmitCap": daily_cap,
            "alreadyEmittedToday": already_emitted_today,
            "silentThresholdHours": state.config.strategic_planner_silent_threshold_hours,
        }),
    )
    .await?;
    Ok(())
}

/// MongoDB 端的粗筛：workspace + account + managed + 静默 + 不在冷却中。
/// 进一步约束（last_outbound_at vs last_inbound_at、pending follow_up 是否
/// 已存在）放在 Rust 侧逐条判，便于测试覆盖且避免在 mongo 层堆复杂表达式。
pub(crate) fn silent_candidate_filter(
    workspace_id: &str,
    account_id: &str,
    silent_before: DateTime,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
        "last_inbound_at": { "$lt": silent_before },
        "$or": [
            { "cooldown_until": { "$exists": false } },
            { "cooldown_until": null },
            { "cooldown_until": { "$lt": DateTime::now() } },
        ],
    }
}

/// Rust 侧的语义校验：Agent 自己刚发出去但用户还没回的情况不算"静默"，
/// 否则会变成 Planner 在用户没说话时帮 Agent 自言自语堆消息。
pub(crate) fn silent_candidate_passes_in_memory(contact: &Contact) -> bool {
    if !matches!(contact.agent_status, crate::models::AgentStatus::Managed) {
        return false;
    }
    let Some(last_inbound) = contact.last_inbound_at else {
        return false;
    };
    if let Some(last_outbound) = contact.last_outbound_at {
        if last_outbound.timestamp_millis() >= last_inbound.timestamp_millis() {
            return false;
        }
    }
    if let Some(cooldown) = contact.cooldown_until {
        if cooldown.timestamp_millis() > DateTime::now().timestamp_millis() {
            return false;
        }
    }
    true
}

fn silent_hours_for(contact: &Contact, now_ms: i64) -> i64 {
    let Some(last_inbound) = contact.last_inbound_at else {
        return 0;
    };
    // R14：i64::saturating_sub 在负数时不会 clamp 到 0（saturate at i64::MIN），
    // 时钟回退 / 测试夹具 last_inbound > now_ms 会导致 silent_hours 为负，
    // 让下游比较语义出错。这里显式 max(0) 防御。
    let diff_ms = now_ms.saturating_sub(last_inbound.timestamp_millis()).max(0);
    diff_ms / (60 * 60 * 1000)
}

// ---------------------------------------------------------------------------
// 段 2：commitment（M2）
// ---------------------------------------------------------------------------

/// 单个 contact 上"应该 emit 哪条承诺"的判定结果（最早到期那一条）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommitmentEmitTarget {
    /// commitment.id（用于 dedup 反查）。
    pub(crate) id: String,
    pub(crate) text: String,
    /// reason kind（`"commitment_overdue"` / `"commitment_imminent"`）。
    pub(crate) reason: CommitmentReason,
    pub(crate) due_at: DateTime,
    /// 该 due_at 是否由 created_at + fallback 窗口合成（承诺本身无显式 due_at）。
    /// 仅供 emit 事件审计；true 表示这是兜底跟进而非 LLM 给出的真实到期时间。
    pub(crate) is_fallback_due: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CommitmentReason {
    Overdue,
    Imminent,
}

impl CommitmentReason {
    fn event_kind(&self) -> &'static str {
        match self {
            CommitmentReason::Overdue => "strategic_planner_commitment_overdue",
            CommitmentReason::Imminent => "strategic_planner_commitment_imminent",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            CommitmentReason::Overdue => "commitment_overdue",
            CommitmentReason::Imminent => "commitment_imminent",
        }
    }
}

/// MongoDB 端筛 managed + 非冷却 + commitments 非空；具体 due_at 判定全部在 Rust 侧。
pub(crate) fn commitment_candidate_filter(workspace_id: &str, account_id: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
        "commitments": { "$exists": true, "$not": { "$size": 0 } },
        "$or": [
            { "cooldown_until": { "$exists": false } },
            { "cooldown_until": null },
            { "cooldown_until": { "$lt": DateTime::now() } },
        ],
    }
}

/// 在 Rust 侧从 contact.commitments 选"最早到期"那条作为代表 emit 目标。
///
/// - `Plain(_)` 元素无 due_at 也无 created_at，跳过；
/// - `Structured` 元素若有 due_at：due_at < now → overdue；now <= due_at <= now+window → imminent；
/// - `Structured` 元素若**无 due_at**：当 `fallback_due_hours > 0` 时用
///   `created_at + fallback_due_hours` 合成兜底 due_at（标记 `is_fallback_due`），
///   再按同样的 overdue/imminent 判定——这样 LLM 当前产出的无 due_at 承诺（全部
///   走 from_plain_text，due_at=None）也能被兜底跟进，而不是永远被跳过；
///   `fallback_due_hours == 0` 时保留旧行为（无 due_at 即跳过）。
/// - 选 due_at 最早的那条；overdue 与 imminent 都存在时优先 overdue（更紧迫）。
pub(crate) fn pick_commitment_emit_target(
    contact: &Contact,
    now: DateTime,
    imminent_window_hours: i64,
    fallback_due_hours: i64,
) -> Option<CommitmentEmitTarget> {
    let now_ms = now.timestamp_millis();
    let imminent_horizon_ms = now_ms.saturating_add(imminent_window_hours * 60 * 60 * 1000);
    let mut best: Option<CommitmentEmitTarget> = None;
    for repr in &contact.commitments {
        let CommitmentRepr::Structured(entry) = repr else {
            continue;
        };
        if entry.id.is_empty() {
            continue;
        }
        // due_at 缺失时用 created_at + fallback 窗口合成（仅当 fallback 启用）。
        let (due_at, is_fallback_due) = match entry.due_at {
            Some(d) => (d, false),
            None if fallback_due_hours > 0 => {
                let synthetic = DateTime::from_millis(
                    entry
                        .created_at
                        .timestamp_millis()
                        .saturating_add(fallback_due_hours * 60 * 60 * 1000),
                );
                (synthetic, true)
            }
            None => continue,
        };
        let due_ms = due_at.timestamp_millis();
        let reason = if due_ms < now_ms {
            CommitmentReason::Overdue
        } else if due_ms <= imminent_horizon_ms {
            CommitmentReason::Imminent
        } else {
            continue;
        };
        let candidate = CommitmentEmitTarget {
            id: entry.id.clone(),
            text: entry.text.clone(),
            reason,
            due_at,
            is_fallback_due,
        };
        best = match best {
            None => Some(candidate),
            Some(prev) => {
                // overdue 优先于 imminent；同 reason 内取更早的 due_at。
                let prev_score = (matches!(prev.reason, CommitmentReason::Overdue), -prev.due_at.timestamp_millis());
                let cand_score = (matches!(candidate.reason, CommitmentReason::Overdue), -candidate.due_at.timestamp_millis());
                if cand_score > prev_score {
                    Some(candidate)
                } else {
                    Some(prev)
                }
            }
        };
    }
    best
}

/// 反查 `agent_events`：某个 commitment_id 在最近 dedup_hours 内是否已被 emit 过。
async fn commitment_recently_emitted(
    state: &AppState,
    contact: &Contact,
    commitment_id: &str,
    now: DateTime,
    dedup_hours: i64,
) -> anyhow::Result<bool> {
    let dedup_ms = dedup_hours.saturating_mul(60 * 60 * 1000);
    let since = DateTime::from_millis(now.timestamp_millis() - dedup_ms);
    let count = state
        .db
        .events()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": {
                    "$in": [
                        "strategic_planner_commitment_overdue",
                        "strategic_planner_commitment_imminent",
                    ],
                },
                "details.commitmentId": commitment_id,
                "created_at": { "$gte": since },
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

/// content 前缀里 text 字段截断长度，避免 task content 越界 prompt。
fn snippet_for_content(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

async fn scan_commitments(state: &AppState) -> anyhow::Result<()> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let now = DateTime::now();
    let global_imminent_window = state.config.strategic_planner_commitment_imminent_window_hours;
    let fallback_due_hours = state.config.strategic_planner_commitment_fallback_due_hours;
    let dedup_hours = state.config.strategic_planner_commitment_emit_dedup_hours;
    let priority_enabled = state.config.strategic_planner_priority_enabled;
    // universal-domain-adaptation H8/1C：每 tick 加载一次 active profile，复用它构造漏斗
    // 排序配置 + 取行业默认范式（commitment enabled/窗口）。
    let profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await;
    let profile_mode = profile.operation_mode.clone();
    let stage_config = build_planner_stage_config(state, &account_id, &profile).await;

    let filter = commitment_candidate_filter(&workspace_id, &account_id);
    let mut cursor = state.db.contacts().find(filter, None).await?;

    let daily_cap = state.config.strategic_planner_daily_emit_cap;
    let already_emitted_today =
        count_today_emit_events(state, &workspace_id, &account_id, now).await?;
    let mut remaining = daily_cap.saturating_sub(already_emitted_today);
    let mut counters = ScanCounters::default();

    // 第一阶段：扫描 + 内存过滤（managed/cooldown/dedup/pending），收集候选。
    let mut candidates: Vec<(Contact, CommitmentEmitTarget)> = Vec::new();
    while let Some(contact) = cursor.try_next().await? {
        counters.scanned += 1;
        if !managed_and_not_in_cooldown(&contact) {
            continue;
        }
        // H8：解析有效范式。commitment 关 → 该 contact 不走承诺到期催进。
        let mode = resolve_operation_mode(&contact, &profile_mode);
        if !mode.commitment.enabled {
            continue;
        }
        // H8：有效临近窗口 = override ?? profile ?? 全局 config。DEFAULT(None)→全局
        // → 与改造前逐字等价；情感/维护型可调长窗口。
        let imminent_window = mode
            .commitment
            .imminent_window_hours
            .unwrap_or(global_imminent_window);
        let Some(target) = pick_commitment_emit_target(&contact, now, imminent_window, fallback_due_hours) else {
            continue;
        };
        if commitment_recently_emitted(state, &contact, &target.id, now, dedup_hours).await? {
            continue;
        }
        if has_pending_follow_up(state, &contact).await? {
            continue;
        }
        candidates.push((contact, target));
    }

    // 第二阶段：跨 contact 优先级稳定排序。priority_enabled=false 时退回 cursor 自然顺序。
    if priority_enabled {
        candidates.sort_by(|a, b| commitment_priority_key(&a.0, &a.1, &stage_config).cmp(&commitment_priority_key(&b.0, &b.1, &stage_config)));
    }

    // 第三阶段：按优先级序消费 daily cap，对每个候选检查 block-rate 反馈环。
    for (contact, target) in candidates {
        if remaining <= 0 {
            write_capped_event(
                state,
                &account_id,
                counters,
                daily_cap,
                already_emitted_today,
                "commitment",
            )
            .await?;
            tracing::info!(
                scanned = counters.scanned,
                emitted = counters.emitted,
                already_emitted_today,
                "strategic planner hit daily cap (commitment)"
            );
            break;
        }
        // M3 反馈环：block-rate 过高时跳过该 contact，写 backoff 事件，不消耗 daily cap。
        if let Some(payload) = should_skip_for_block_rate(state, &contact, now).await? {
            write_backoff_event(state, "commitment", &contact, payload).await?;
            continue;
        }
        let due_at_ms = target.due_at.timestamp_millis();
        let snippet = snippet_for_content(&target.text, 80);
        let content = format!(
            "Planner: {label} (id={id}, due_at={due_ms}, text=\"{snippet}\")",
            label = target.reason.label(),
            id = target.id,
            due_ms = due_at_ms,
        );
        emit_planner_follow_up(state, &contact, content, now).await?;
        write_event(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            target.reason.event_kind(),
            "emitted",
            &format!(
                "Planner: {} emitted (id={}, due_at={})",
                target.reason.label(),
                target.id,
                due_at_ms
            ),
            Some(doc! {
                "source": "strategic_planner",
                "reason": target.reason.label(),
                "commitmentId": &target.id,
                "dueAt": due_at_ms,
                "isFallbackDue": target.is_fallback_due,
                "textSnippet": snippet,
            }),
        )
        .await?;
        counters.emitted += 1;
        remaining -= 1;
    }

    write_event(
        state,
        &account_id,
        None,
        "strategic_planner_commitment_tick",
        "ok",
        &format!(
            "strategic planner commitment tick: scanned {}, emitted {}",
            counters.scanned, counters.emitted
        ),
        Some(doc! {
            "scanned": counters.scanned,
            "emitted": counters.emitted,
            "dailyEmitCap": daily_cap,
            "alreadyEmittedToday": already_emitted_today,
            "imminentWindowHours": global_imminent_window,
            "dedupHours": dedup_hours,
            "priorityEnabled": priority_enabled,
        }),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 段 3：stage_stagnation（M2）
// ---------------------------------------------------------------------------

/// 终态/不推进态列表：这些 stage 不再 emit stage_stagnation。
/// 取值对齐 m006 种子的真实 customer_stage id——`customer_success`(成交后维护)、
/// `cooldown`(风险冷却,另有 cooldown_until 时间门控)、`dormant_reactivation`
/// (沉默唤醒,本就低频触达,不应被停滞催)。后续 milestone 可考虑从
/// `system_taxonomies` 按 metadata 标记读出。
const TERMINAL_STAGES: &[&str] = &["customer_success", "cooldown", "dormant_reactivation"];

/// universal-domain-adaptation 1C：planner 漏斗排序的运行时配置，每个 tick 由
/// [`build_planner_stage_config`] 从 active DomainProfile + taxonomy 缓存构造一次
/// （避免 N+1）。
///
/// **DEFAULT 等价**：销售域里 m006 seed 的 priority_weight / is_terminal 与写死的
/// `stage_priority_weight` / `intent_level_weight` / `TERMINAL_STAGES` 逐字相等
/// （`seeded_weights_match_planner_hardcoded_verbatim` 已锁），故"读字典"与"读写死"
/// 对 DEFAULT 产出完全一致。字典缺该取值时回落写死函数（空缓存 / 未配置 / 新加 stage
/// 尚无权重都安全）。
///
/// `stagnation_dimension`：planner stage_stagnation 段的计时维度 dotted-key
/// （DEFAULT="customer_stage"）。当前仅承载该值供内存判定；MongoDB 端 filter 的
/// dotted-key 动态化随后续 milestone（需要动态字段名拼接）跟进，DEFAULT 不变。
#[derive(Debug, Clone)]
pub(crate) struct PlannerStageConfig {
    /// customer_stage 取值 → priority_weight（仅含 Some 权重的取值）。
    stage_weights: std::collections::HashMap<String, i32>,
    /// intent_level 取值 → priority_weight。
    intent_weights: std::collections::HashMap<String, i32>,
    /// 终态 stage canonical id 集合（is_terminal=true）。
    terminal_stages: std::collections::HashSet<String>,
    /// stage_stagnation 计时维度（DEFAULT="customer_stage"）。
    stagnation_dimension: String,
}

impl Default for PlannerStageConfig {
    /// 空配置：所有查询回落写死 DEFAULT 函数 + customer_stage 维度。
    fn default() -> Self {
        Self {
            stage_weights: std::collections::HashMap::new(),
            intent_weights: std::collections::HashMap::new(),
            terminal_stages: std::collections::HashSet::new(),
            stagnation_dimension: "customer_stage".to_string(),
        }
    }
}

impl PlannerStageConfig {
    /// stage 权重：字典命中则用字典值，否则回落写死 [`stage_priority_weight`]。
    fn stage_weight(&self, stage: Option<&str>) -> i32 {
        match stage.and_then(|s| self.stage_weights.get(s)) {
            Some(w) => *w,
            None => stage_priority_weight(stage),
        }
    }

    /// intent 权重：字典命中则用字典值，否则回落写死 [`intent_level_weight`]。
    fn intent_weight(&self, level: Option<&str>) -> i32 {
        match level.and_then(|l| self.intent_weights.get(l)) {
            Some(w) => *w,
            None => intent_level_weight(level),
        }
    }

    /// 是否终态：字典非空时以字典 is_terminal 为准，否则回落写死 [`TERMINAL_STAGES`]。
    fn is_terminal_stage(&self, stage: &str) -> bool {
        if self.terminal_stages.is_empty() {
            TERMINAL_STAGES.iter().any(|t| *t == stage)
        } else {
            self.terminal_stages.contains(stage)
        }
    }
}

/// universal-domain-adaptation H8：从**已加载的** profile + taxonomy 缓存构造
/// [`PlannerStageConfig`]，让扫描器每 tick 只加载一次 profile（既取 stage 排序配置，
/// 又取 `profile.operation_mode`），避免对同一 profile 双重加载。缓存已由 agent
/// 路径 warm_up；这里再 `find_or_load` 一次保证 TTL 自愈。
pub(crate) async fn build_planner_stage_config(
    state: &AppState,
    account_id: &str,
    profile: &crate::models::DomainProfile,
) -> PlannerStageConfig {
    use crate::agent::taxonomy::{dimension_value_weights, global_taxonomy_cache};

    let stagnation_dimension = profile
        .stagnation_dimension
        .clone()
        .unwrap_or_else(|| "customer_stage".to_string());

    let cache = global_taxonomy_cache();
    cache.find_or_load(&state.db).await;

    let mut config = PlannerStageConfig {
        stage_weights: std::collections::HashMap::new(),
        intent_weights: std::collections::HashMap::new(),
        terminal_stages: std::collections::HashSet::new(),
        stagnation_dimension,
    };
    for (id, weight, is_terminal) in
        dimension_value_weights("customer_stage", account_id, &cache)
    {
        if let Some(w) = weight {
            config.stage_weights.insert(id.clone(), w);
        }
        if is_terminal {
            config.terminal_stages.insert(id);
        }
    }
    for (id, weight, _is_terminal) in
        dimension_value_weights("intent_level", account_id, &cache)
    {
        if let Some(w) = weight {
            config.intent_weights.insert(id, w);
        }
    }
    config
}

/// universal-domain-adaptation H8：解析单个 contact 的**有效运营范式**。
/// 三级回落：`contact.operation_mode_override ?? profile.operation_mode`
/// （profile 缺省即 `OperationMode::default()` = 三全开 + 阈值 None）。
/// 覆盖是**整组**替换（不做逐驱动力 merge），与设计「单客户整套范式覆盖」一致。
///
/// DEFAULT_PROFILE + 无 override → 返回 `OperationMode::default()`，三驱动力 enabled
/// 且阈值 None，planner 行为与改造前逐字等价。
pub(crate) fn resolve_operation_mode(
    contact: &Contact,
    profile_mode: &crate::models::OperationMode,
) -> crate::models::OperationMode {
    contact
        .operation_mode_override
        .clone()
        .unwrap_or_else(|| profile_mode.clone())
}

/// MongoDB 端筛 managed + 非冷却 + 非终状态 + customer_stage_updated_at 老于阈值
/// + last_inbound_at 不太近（avoid 与 silent 段重叠）。
pub(crate) fn stage_stagnation_candidate_filter(
    workspace_id: &str,
    account_id: &str,
    stage_updated_before: DateTime,
    inbound_before: DateTime,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
        "domain_attributes.customer_stage": { "$exists": true, "$ne": null, "$nin": TERMINAL_STAGES },
        "domain_attributes.customer_stage_updated_at": { "$lt": stage_updated_before },
        "last_inbound_at": { "$lt": inbound_before },
        "$or": [
            { "cooldown_until": { "$exists": false } },
            { "cooldown_until": null },
            { "cooldown_until": { "$lt": DateTime::now() } },
        ],
    }
}

/// Rust 侧的语义校验：与 silent 同款 managed/cooldown 检查 + last_outbound>=last_inbound 跳过。
pub(crate) fn stage_stagnation_passes_in_memory(
    contact: &Contact,
    now: DateTime,
    config: &PlannerStageConfig,
) -> bool {
    if !managed_and_not_in_cooldown(contact) {
        return false;
    }
    let Some(stage) = contact_customer_stage(contact) else {
        return false;
    };
    let stage = stage.as_str();
    if config.is_terminal_stage(stage) {
        return false;
    }
    // universal-domain-adaptation 1C：停滞计时维度从 profile 读（DEFAULT=customer_stage）。
    if contact_stagnation_updated_at(contact, &config.stagnation_dimension).is_none() {
        return false;
    }
    // 用户最近刚说过话——交给 silent / 自然回路推进，stage 段不抢 emit。
    if let Some(last_inbound) = contact.last_inbound_at {
        if let Some(last_outbound) = contact.last_outbound_at {
            if last_outbound.timestamp_millis() >= last_inbound.timestamp_millis() {
                // Agent 已 ping 过用户但用户没回——避免 stage 段叠加催。
                let _ = now;
                return false;
            }
        }
    }
    true
}

fn managed_and_not_in_cooldown(contact: &Contact) -> bool {
    if !matches!(contact.agent_status, crate::models::AgentStatus::Managed) {
        return false;
    }
    if let Some(cooldown) = contact.cooldown_until {
        if cooldown.timestamp_millis() > DateTime::now().timestamp_millis() {
            return false;
        }
    }
    true
}

fn idle_days_since(stage_updated_at: Option<DateTime>, now_ms: i64) -> i64 {
    let Some(updated) = stage_updated_at else {
        return 0;
    };
    let diff_ms = now_ms.saturating_sub(updated.timestamp_millis());
    diff_ms / (24 * 60 * 60 * 1000)
}

// ---------------------------------------------------------------------------
// M3 优先级：commitment / stage_stagnation 段在内存里按权重稳定排序，
// 决定 daily cap 撞顶时谁先被消费。silent 段不参与排序（单一信号）。
// ---------------------------------------------------------------------------

/// 客户阶段价值权重：值越大越优先 emit。映射常量化，**不**新增 collection。
///
/// 取值来自 `system_taxonomies` 的 customer_stage 种子（m006，9 个 canonical id），
/// 权重按**销售漏斗推进度**单调排序——越接近成交、越需要及时跟进的阶段权重越高，
/// 这样 daily cap 撞顶时高价值客户优先被 emit。这是可复现的抽象规则（漏斗序），
/// 不针对任何单条对话。`customer_success` / `cooldown` / `dormant_reactivation` 是
/// 成交后 / 冷却 / 沉默的终态或低优先态，已在 TERMINAL_STAGES 内存过滤里排除出
/// stage_stagnation 段，这里给低位权重供 commitment 段排序复用。
pub(crate) fn stage_priority_weight(stage: Option<&str>) -> i32 {
    match stage {
        Some("commitment_followup") => 100,
        Some("objection_handling") | Some("solution_fit") => 80,
        Some("need_discovery") => 60,
        Some("relationship_building") => 40,
        Some("new_contact") => 20,
        Some("customer_success") | Some("cooldown") | Some("dormant_reactivation") => 10,
        _ => 20,
    }
}

/// `intent_level` 权重：值越大越优先 emit。取值来自 m006 种子（high / medium / low）。
pub(crate) fn intent_level_weight(level: Option<&str>) -> i32 {
    match level {
        Some("high") => 80,
        Some("medium") => 50,
        Some("low") => 20,
        _ => 10,
    }
}

/// commitment 段排序键。返回的元组按 **升序** 排序时，**值越小越优先 emit**。
///
/// 序：
/// 1. reason 紧迫度：`Overdue=0` < `Imminent=1`（overdue 先于 imminent）；
/// 2. 客户阶段权重：使用 `-stage_priority_weight`（数值越大→越靠前）；
/// 3. `intent_level` 权重：`-intent_level_weight`（hot/warm 优先于 cold）；
/// 4. due_at 早先：`due_at` 毫秒时间戳直接升序（更早 due 越优先）。
pub(crate) fn commitment_priority_key(
    contact: &Contact,
    target: &CommitmentEmitTarget,
    config: &PlannerStageConfig,
) -> (i32, i32, i32, i64) {
    let reason_ord = match target.reason {
        CommitmentReason::Overdue => 0,
        CommitmentReason::Imminent => 1,
    };
    let stage_w = -config.stage_weight(contact_customer_stage(contact).as_deref());
    let intent_w = -config.intent_weight(contact_intent_level(contact).as_deref());
    (reason_ord, stage_w, intent_w, target.due_at.timestamp_millis())
}

/// stage_stagnation 段排序键。返回的元组按 **升序** 排序时，**值越小越优先 emit**。
///
/// 序：
/// 1. 客户阶段权重：`-stage_priority_weight`（高价值阶段优先）；
/// 2. 停滞时长：`-(now_ms - stage_updated_at)`（停滞越久越优先）。
pub(crate) fn stage_stagnation_priority_key(
    contact: &Contact,
    now_ms: i64,
    config: &PlannerStageConfig,
) -> (i32, i64) {
    let stage_w = -config.stage_weight(contact_customer_stage(contact).as_deref());
    let stagnation_ms = match contact_stagnation_updated_at(contact, &config.stagnation_dimension) {
        Some(ts) => now_ms.saturating_sub(ts.timestamp_millis()),
        None => 0,
    };
    (stage_w, -stagnation_ms)
}

// ---------------------------------------------------------------------------
// M3 反馈环（block-rate backoff）：emit 前查"过去 N 小时该 contact 的 final_review_status
// 命中率"，若 block-rate ≥ 阈值则当次跳过 emit、写一条 `*_backoff` 事件。
// 不消耗 daily cap，下一个 tick 重新评估。
// ---------------------------------------------------------------------------

/// `final_review_status` 桶映射：返回 `(blocked_like, ok_like)` 计数贡献。
fn classify_review_status(status: &str) -> (i64, i64) {
    match status {
        // blocked-like：5 闸或预算判 fail，AI 没成功对外说话。
        "blocked_unverified_product_claim"
        | "held_by_ai_policy"
        | "blocked_by_safety_guard"
        | "ai_waiting_for_more_context"
        | "blocked_by_budget"
        | "blocked_by_required_field"
        | "revision_failed" => (1, 0),
        // ok-like：真实送出或本地审完。
        "approved" | "revision_applied_approved" | "local_decision_review" => (0, 1),
        // 其它（含 legacy_mode_unchecked / 历史脏值 / 空字符串）→ 不参与判定。
        _ => (0, 0),
    }
}

/// 计算 contact 在过去 `block_rate_window_hours` 内的 block-rate；如果 ≥ 阈值
/// 且窗口内 run 总数（blocked + ok）≥ `min_runs`，返回 `Some(detail_doc)` 表示
/// 应当跳过；否则返回 `None`。
async fn should_skip_for_block_rate(
    state: &AppState,
    contact: &Contact,
    now: DateTime,
) -> anyhow::Result<Option<Document>> {
    let window_hours = state.config.strategic_planner_block_rate_window_hours;
    let min_runs = state.config.strategic_planner_block_rate_min_runs;
    // M4 W4 Task 5.1：planner_block_rate_threshold 通过 resolve_thresholds 取值，
    // 让 threshold_overrides 的 release 在下一个 tick 立即生效。
    let threshold = crate::agent::runtime::resolve_thresholds(state, contact)
        .await
        .map_err(|e| anyhow::anyhow!("resolve_thresholds failed: {e}"))?
        .planner_block_rate_threshold;
    if window_hours <= 0 || min_runs <= 0 || threshold <= 0.0 {
        return Ok(None);
    }
    let since = DateTime::from_millis(
        now.timestamp_millis() - window_hours.saturating_mul(60 * 60 * 1000),
    );
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "created_at": { "$gte": since },
            },
            None,
        )
        .await?;
    let mut blocked: i64 = 0;
    let mut ok: i64 = 0;
    while let Some(log) = cursor.try_next().await? {
        let (b, o) = classify_review_status(&log.final_review_status);
        blocked += b;
        ok += o;
    }
    let total = blocked + ok;
    if total < min_runs {
        return Ok(None);
    }
    let rate = (blocked as f64) / (total as f64);
    if rate < threshold {
        return Ok(None);
    }
    Ok(Some(doc! {
        "reason": "block_rate_above_threshold",
        "windowHours": window_hours,
        "blockedCount": blocked,
        "okCount": ok,
        "blockRate": rate,
        "threshold": threshold,
        "contactWxid": &contact.wxid,
    }))
}

/// 写一条 `strategic_planner_<segment>_backoff` 事件。
///
/// 与 [`write_capped_event`] 同款政策——backoff kind **不**进 [`EMIT_EVENT_KINDS`]，
/// 因此不消耗 daily cap。下一个 tick 重新评估。
async fn write_backoff_event(
    state: &AppState,
    segment: &str,
    contact: &Contact,
    payload: Document,
) -> anyhow::Result<()> {
    let kind = match segment {
        "silent" => "strategic_planner_silent_backoff",
        "commitment" => "strategic_planner_commitment_backoff",
        "stage_stagnation" => "strategic_planner_stage_stagnation_backoff",
        _ => "strategic_planner_backoff",
    };
    let mut details = payload;
    details.insert("segment", segment);
    write_event(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        kind,
        "skipped",
        &format!(
            "Planner: {segment} skipped (AI 自主回退：block-rate above threshold)"
        ),
        Some(details),
    )
    .await
}

async fn scan_stage_stagnation(state: &AppState) -> anyhow::Result<()> {
    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let now = DateTime::now();
    let now_ms = now.timestamp_millis();
    let global_threshold_days = state
        .config
        .strategic_planner_stage_stagnation_threshold_days;
    let recent_inbound_hours = state
        .config
        .strategic_planner_stage_stagnation_recent_inbound_hours;
    let priority_enabled = state.config.strategic_planner_priority_enabled;
    // universal-domain-adaptation H8/1C：每 tick 加载一次 active profile，复用它构造漏斗
    // 排序配置 + 取行业默认范式（funnel enabled/停滞阈值）。
    let profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await;
    let profile_mode = profile.operation_mode.clone();
    let stage_config = build_planner_stage_config(state, &account_id, &profile).await;
    let stage_updated_before =
        DateTime::from_millis(now_ms - global_threshold_days.saturating_mul(24 * 60 * 60 * 1000));
    let inbound_before =
        DateTime::from_millis(now_ms - recent_inbound_hours.saturating_mul(60 * 60 * 1000));

    let filter = stage_stagnation_candidate_filter(
        &workspace_id,
        &account_id,
        stage_updated_before,
        inbound_before,
    );
    let mut cursor = state.db.contacts().find(filter, None).await?;

    let daily_cap = state.config.strategic_planner_daily_emit_cap;
    let already_emitted_today =
        count_today_emit_events(state, &workspace_id, &account_id, now).await?;
    let mut remaining = daily_cap.saturating_sub(already_emitted_today);
    let mut counters = ScanCounters::default();

    // 第一阶段：扫描 + 内存过滤 + pending 检查，收集候选。
    let mut candidates: Vec<Contact> = Vec::new();
    while let Some(contact) = cursor.try_next().await? {
        counters.scanned += 1;
        if !stage_stagnation_passes_in_memory(&contact, now, &stage_config) {
            continue;
        }
        // H8：解析有效范式。funnel 关 = 漏斗推进短路（陪伴/维护型对该 contact 不催阶段）。
        // 这是设计里的「关 funnel = scan_stage_stagnation 对该 contact return/continue」纯减法。
        let mode = resolve_operation_mode(&contact, &profile_mode);
        if !mode.funnel.enabled {
            continue;
        }
        // H8：有效停滞阈值（天）= override ?? profile ?? 全局 config。DEFAULT(None)→全局
        // → 与 DB 粗筛一致、恒真（金标零变化）；override 更长则收紧。
        let effective_threshold_days = mode
            .funnel
            .stagnation_threshold_days
            .unwrap_or(global_threshold_days);
        let stage_updated = contact_stagnation_updated_at(&contact, &stage_config.stagnation_dimension);
        if idle_days_since(stage_updated, now_ms) < effective_threshold_days {
            continue;
        }
        if has_pending_follow_up(state, &contact).await? {
            continue;
        }
        candidates.push(contact);
    }

    // 第二阶段：跨 contact 优先级稳定排序。priority_enabled=false 时退回 cursor 自然顺序。
    if priority_enabled {
        candidates.sort_by(|a, b| stage_stagnation_priority_key(a, now_ms, &stage_config).cmp(&stage_stagnation_priority_key(b, now_ms, &stage_config)));
    }

    // 第三阶段：按优先级序消费 daily cap，对每个候选检查 block-rate 反馈环。
    for contact in candidates {
        if remaining <= 0 {
            write_capped_event(
                state,
                &account_id,
                counters,
                daily_cap,
                already_emitted_today,
                "stage_stagnation",
            )
            .await?;
            tracing::info!(
                scanned = counters.scanned,
                emitted = counters.emitted,
                already_emitted_today,
                "strategic planner hit daily cap (stage_stagnation)"
            );
            break;
        }
        if let Some(payload) = should_skip_for_block_rate(state, &contact, now).await? {
            write_backoff_event(state, "stage_stagnation", &contact, payload).await?;
            continue;
        }
        let stage = contact_customer_stage(&contact)
            .unwrap_or_else(|| "unknown".to_string());
        let stage_updated = contact_customer_stage_updated_at(&contact);
        let idle_days = idle_days_since(stage_updated, now_ms);
        let content = format!("Planner: stage_stagnation (stage={stage}, idle={idle_days}d)");
        emit_planner_follow_up(state, &contact, content, now).await?;
        write_event(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "strategic_planner_stage_stagnation",
            "emitted",
            &format!("Planner: stage_stagnation emitted (stage={stage}, idle={idle_days}d)"),
            Some(doc! {
                "source": "strategic_planner",
                "stage": &stage,
                "idleDays": idle_days,
                "stageUpdatedAt": stage_updated
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(0),
            }),
        )
        .await?;
        counters.emitted += 1;
        remaining -= 1;
    }

    write_event(
        state,
        &account_id,
        None,
        "strategic_planner_stage_stagnation_tick",
        "ok",
        &format!(
            "strategic planner stage_stagnation tick: scanned {}, emitted {}",
            counters.scanned, counters.emitted
        ),
        Some(doc! {
            "scanned": counters.scanned,
            "emitted": counters.emitted,
            "dailyEmitCap": daily_cap,
            "alreadyEmittedToday": already_emitted_today,
            "stageStagnationThresholdDays": global_threshold_days,
            "recentInboundHours": recent_inbound_hours,
            "priorityEnabled": priority_enabled,
        }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentStatus, CommitmentEntry, CommitmentRepr};
    use mongodb::bson::Document;

    /// 1C：测试用空 PlannerStageConfig——所有查询回落写死 DEFAULT 函数 +
    /// customer_stage 维度。等价护栏：DEFAULT 行为与改造前逐字一致。
    fn cfg() -> PlannerStageConfig {
        PlannerStageConfig::default()
    }

    fn template() -> Contact {
        Contact {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            wxid: "user_test".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            tags: Vec::new(),
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    fn entry(id: &str, text: &str, due_at: Option<DateTime>) -> CommitmentRepr {
        CommitmentRepr::Structured(CommitmentEntry {
            id: id.to_string(),
            text: text.to_string(),
            due_at,
            created_at: DateTime::from_millis(0),
            extra: Document::new(),
        })
    }

    fn dt(ms: i64) -> DateTime {
        DateTime::from_millis(ms)
    }

    fn attrs_with_stage(stage: &str) -> Document {
        let mut d = Document::new();
        d.insert("customer_stage", stage);
        d.insert("customer_stage_updated_at", DateTime::from_millis(0));
        d
    }

    fn attrs_with_stage_updated(stage: &str, updated_at_ms: i64) -> Document {
        let mut d = Document::new();
        d.insert("customer_stage", stage);
        d.insert("customer_stage_updated_at", DateTime::from_millis(updated_at_ms));
        d
    }

    fn attrs_with_stage_intent(stage: &str, intent: &str) -> Document {
        let mut d = Document::new();
        d.insert("customer_stage", stage);
        d.insert("customer_stage_updated_at", DateTime::from_millis(0));
        d.insert("intent_level", intent);
        d
    }

    // -----------------------------------------------------------------
    // silent 段（M1）
    // -----------------------------------------------------------------

    #[test]
    fn passes_when_inbound_long_ago_and_no_outbound() {
        let contact = Contact {
            last_inbound_at: Some(dt(1_000)),
            ..template()
        };
        assert!(silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn excludes_unmanaged_contact() {
        let contact = Contact {
            agent_status: AgentStatus::Normal,
            last_inbound_at: Some(dt(1_000)),
            ..template()
        };
        assert!(!silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn excludes_when_inbound_missing() {
        let contact = template();
        assert!(!silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn excludes_when_last_outbound_after_last_inbound() {
        let contact = Contact {
            last_inbound_at: Some(dt(1_000)),
            last_outbound_at: Some(dt(2_000)),
            ..template()
        };
        assert!(!silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn passes_when_last_outbound_before_last_inbound() {
        let contact = Contact {
            last_inbound_at: Some(dt(3_000)),
            last_outbound_at: Some(dt(2_000)),
            ..template()
        };
        assert!(silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn excludes_when_cooldown_until_in_future() {
        let future = dt(DateTime::now().timestamp_millis() + 60 * 60 * 1000);
        let contact = Contact {
            last_inbound_at: Some(dt(1_000)),
            cooldown_until: Some(future),
            ..template()
        };
        assert!(!silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn passes_when_cooldown_until_in_past() {
        let past = dt(DateTime::now().timestamp_millis() - 60 * 60 * 1000);
        let contact = Contact {
            last_inbound_at: Some(dt(1_000)),
            cooldown_until: Some(past),
            ..template()
        };
        assert!(silent_candidate_passes_in_memory(&contact));
    }

    #[test]
    fn silent_hours_diff_basic() {
        let contact = Contact {
            last_inbound_at: Some(dt(0)),
            ..template()
        };
        let now_ms: i64 = 5 * 60 * 60 * 1000;
        assert_eq!(silent_hours_for(&contact, now_ms), 5);
    }

    #[test]
    fn silent_hours_for_returns_zero_on_clock_skew() {
        // R14：now_ms < last_inbound（时钟回退或测试夹具偏移） SHALL 返回 0
        // 而非负数，依赖 saturating_sub 防御。
        let contact = Contact {
            last_inbound_at: Some(dt(10_000_000)),
            ..template()
        };
        let now_ms: i64 = 5_000_000;
        assert_eq!(silent_hours_for(&contact, now_ms), 0);
    }

    #[test]
    fn silent_hours_for_exact_one_hour_boundary() {
        // R14：精确 1 小时（3,600,000 ms）SHALL 返回 1
        let contact = Contact {
            last_inbound_at: Some(dt(0)),
            ..template()
        };
        let now_ms: i64 = 60 * 60 * 1000;
        assert_eq!(silent_hours_for(&contact, now_ms), 1);
    }

    #[test]
    fn silent_hours_for_just_below_one_hour_returns_zero() {
        // R14：59min 59s 999ms SHALL 返回 0（向下取整）
        let contact = Contact {
            last_inbound_at: Some(dt(0)),
            ..template()
        };
        let now_ms: i64 = 60 * 60 * 1000 - 1;
        assert_eq!(silent_hours_for(&contact, now_ms), 0);
    }

    #[test]
    fn silent_hours_for_handles_missing_inbound() {
        // R14：last_inbound_at=None SHALL 返回 0（不静默）
        let contact = template();
        let now_ms: i64 = 999_999_999_999;
        assert_eq!(silent_hours_for(&contact, now_ms), 0);
    }

    #[test]
    fn silent_filter_includes_expected_keys() {
        let filter = silent_candidate_filter("ws", "acc", dt(1_000));
        assert!(filter.contains_key("workspace_id"));
        assert!(filter.contains_key("account_id"));
        assert!(filter.contains_key("agent_status"));
        assert!(filter.contains_key("last_inbound_at"));
        assert!(filter.contains_key("$or"));
    }

    // -----------------------------------------------------------------
    // commitment 段（M2）
    // -----------------------------------------------------------------

    /// 多条 commitment 中应选最早 due 的那条；overdue 优先于 imminent。
    #[test]
    fn commitment_overdue_picks_earliest() {
        let now = dt(10_000_000);
        let later_overdue = entry(
            "id-late",
            "晚一些过期",
            Some(dt(now.timestamp_millis() - 60 * 60 * 1000)),
        );
        let earlier_overdue = entry(
            "id-early",
            "早一点过期",
            Some(dt(now.timestamp_millis() - 5 * 60 * 60 * 1000)),
        );
        let imminent = entry(
            "id-imminent",
            "快到期",
            Some(dt(now.timestamp_millis() + 60 * 60 * 1000)),
        );
        let contact = Contact {
            commitments: vec![later_overdue, earlier_overdue, imminent],
            ..template()
        };
        let target = pick_commitment_emit_target(&contact, now, 8, 0).expect("target should exist");
        assert_eq!(target.id, "id-early");
        assert_eq!(target.reason, CommitmentReason::Overdue);
    }

    /// imminent 窗口内（now <= due_at <= now + window）。
    #[test]
    fn commitment_imminent_within_window() {
        let now = dt(10_000_000);
        let due = dt(now.timestamp_millis() + 4 * 60 * 60 * 1000);
        let contact = Contact {
            commitments: vec![entry("id-im", "4 小时后到期", Some(due))],
            ..template()
        };
        let target = pick_commitment_emit_target(&contact, now, 8, 0).expect("imminent");
        assert_eq!(target.reason, CommitmentReason::Imminent);
        assert_eq!(target.id, "id-im");
    }

    /// imminent 窗口外（due_at > now + window）→ 不命中。
    #[test]
    fn commitment_imminent_outside_window() {
        let now = dt(10_000_000);
        let due = dt(now.timestamp_millis() + 12 * 60 * 60 * 1000);
        let contact = Contact {
            commitments: vec![entry("id-far", "12 小时后到期", Some(due))],
            ..template()
        };
        assert!(pick_commitment_emit_target(&contact, now, 8, 0).is_none());
    }

    /// `Plain` 元素无 due_at → 跳过；只剩 Plain 时不 emit。
    #[test]
    fn commitment_skips_plain_repr() {
        let now = dt(10_000_000);
        let contact = Contact {
            commitments: vec![CommitmentRepr::Plain("旧字符串承诺".to_string())],
            ..template()
        };
        assert!(pick_commitment_emit_target(&contact, now, 8, 0).is_none());
    }

    /// 构造一条无 due_at、可控 created_at 的 Structured 承诺(模拟 from_plain_text 落库形态)。
    fn entry_no_due(id: &str, created_ms: i64) -> CommitmentRepr {
        CommitmentRepr::Structured(CommitmentEntry {
            id: id.to_string(),
            text: "无 due 的承诺".to_string(),
            due_at: None,
            created_at: DateTime::from_millis(created_ms),
            extra: Document::new(),
        })
    }

    /// #70 兜底:无 due_at 承诺在 fallback 启用时用 created_at + fallback 合成 due。
    /// created_at 很早 → 合成 due 已过 now → overdue,被 emit(原行为是永远跳过)。
    #[test]
    fn commitment_no_due_with_fallback_emits_overdue() {
        let now = dt(100 * 60 * 60 * 1000); // now = 100h
        let contact = Contact {
            commitments: vec![entry_no_due("id-nodue", 0)], // created=0,fallback=72h → due=72h < now
            ..template()
        };
        let target =
            pick_commitment_emit_target(&contact, now, 8, 72).expect("fallback should emit");
        assert_eq!(target.id, "id-nodue");
        assert_eq!(target.reason, CommitmentReason::Overdue);
        assert!(target.is_fallback_due, "应标记为兜底合成的 due");
    }

    /// #70 兜底:fallback=0(禁用)时,无 due_at 承诺仍按旧行为跳过。
    #[test]
    fn commitment_no_due_without_fallback_skips() {
        let now = dt(100 * 60 * 60 * 1000);
        let contact = Contact {
            commitments: vec![entry_no_due("id-nodue", 0)],
            ..template()
        };
        assert!(pick_commitment_emit_target(&contact, now, 8, 0).is_none());
    }

    /// #70 兜底:created_at 较新(合成 due 还在未来窗口内)→ imminent 而非 overdue。
    #[test]
    fn commitment_no_due_fallback_imminent_when_recently_created() {
        // created=98h,fallback=4h → 合成 due=102h;now=100h,imminent 窗口 8h → 102h 在 [100,108] 内。
        let now = dt(100 * 60 * 60 * 1000);
        let contact = Contact {
            commitments: vec![entry_no_due("id-nodue", 98 * 60 * 60 * 1000)],
            ..template()
        };
        let target =
            pick_commitment_emit_target(&contact, now, 8, 4).expect("fallback imminent");
        assert_eq!(target.reason, CommitmentReason::Imminent);
        assert!(target.is_fallback_due);
    }

    /// 没有 id 的 Structured（理论上迁移后不该存在）也跳过。
    #[test]
    fn commitment_skips_structured_without_id() {
        let now = dt(10_000_000);
        let contact = Contact {
            commitments: vec![entry(
                "",
                "无 id",
                Some(dt(now.timestamp_millis() - 1_000)),
            )],
            ..template()
        };
        assert!(pick_commitment_emit_target(&contact, now, 8, 0).is_none());
    }

    /// commitment filter 只筛 commitments 非空。
    #[test]
    fn commitment_filter_includes_expected_keys() {
        let filter = commitment_candidate_filter("ws", "acc");
        assert!(filter.contains_key("workspace_id"));
        assert!(filter.contains_key("account_id"));
        assert!(filter.contains_key("agent_status"));
        assert!(filter.contains_key("commitments"));
        assert!(filter.contains_key("$or"));
    }

    // -----------------------------------------------------------------
    // stage_stagnation 段（M2）
    // -----------------------------------------------------------------

    #[test]
    fn stage_stagnation_excludes_terminal_stage() {
        let now = DateTime::now();
        // 真实终态 id（customer_success 成交后维护）应被 TERMINAL_STAGES 内存过滤排除，
        // 即便 last_inbound 已远超停滞阈值也不 emit。
        let contact = Contact {
            last_inbound_at: Some(dt(now.timestamp_millis() - 30 * 24 * 60 * 60 * 1000)),
            domain_attributes: Some(attrs_with_stage("customer_success")),
            ..template()
        };
        assert!(!stage_stagnation_passes_in_memory(&contact, now, &cfg()));
    }

    #[test]
    fn stage_stagnation_excludes_recent_inbound() {
        let now = DateTime::now();
        let contact = Contact {
            last_inbound_at: Some(dt(now.timestamp_millis() - 60 * 60 * 1000)),
            // 通过 last_outbound>=last_inbound 路径排除（与 silent 段同款 in-memory 检查）。
            last_outbound_at: Some(dt(now.timestamp_millis())),
            ..template()
        };
        assert!(!stage_stagnation_passes_in_memory(&contact, now, &cfg()));
    }

    #[test]
    fn stage_stagnation_triggers_on_threshold() {
        let now = DateTime::now();
        let contact = Contact {
            last_inbound_at: Some(dt(now.timestamp_millis() - 30 * 24 * 60 * 60 * 1000)),
            domain_attributes: Some(attrs_with_stage("need_discovery")),
            ..template()
        };
        assert!(stage_stagnation_passes_in_memory(&contact, now, &cfg()));
    }

    #[test]
    fn stage_stagnation_excludes_when_no_stage() {
        let now = DateTime::now();
        let contact = Contact {
            last_inbound_at: Some(dt(now.timestamp_millis() - 30 * 24 * 60 * 60 * 1000)),
            ..template()
        };
        assert!(!stage_stagnation_passes_in_memory(&contact, now, &cfg()));
    }

    #[test]
    fn stage_stagnation_excludes_when_no_updated_at() {
        let now = DateTime::now();
        let contact = Contact {
            last_inbound_at: Some(dt(now.timestamp_millis() - 30 * 24 * 60 * 60 * 1000)),
            ..template()
        };
        assert!(!stage_stagnation_passes_in_memory(&contact, now, &cfg()));
    }

    #[test]
    fn stage_stagnation_filter_includes_expected_keys() {
        let filter = stage_stagnation_candidate_filter("ws", "acc", dt(1_000), dt(2_000));
        assert!(filter.contains_key("workspace_id"));
        assert!(filter.contains_key("agent_status"));
        // customer_stage / customer_stage_updated_at 存在于 domain_attributes 容器，
        // 不在文档顶层。过滤器必须用 dotted-key 查 domain_attributes.*，否则真实库
        // 筛空、stage_stagnation 段整段空转。
        assert!(filter.contains_key("domain_attributes.customer_stage"));
        assert!(filter.contains_key("domain_attributes.customer_stage_updated_at"));
        assert!(!filter.contains_key("customer_stage"));
        assert!(!filter.contains_key("customer_stage_updated_at"));
        assert!(filter.contains_key("last_inbound_at"));
    }

    #[test]
    fn idle_days_since_basic() {
        let now_ms: i64 = 5 * 24 * 60 * 60 * 1000;
        assert_eq!(idle_days_since(Some(dt(0)), now_ms), 5);
        assert_eq!(idle_days_since(None, now_ms), 0);
    }

    #[test]
    fn snippet_truncates_long_text() {
        let long = "一".repeat(100);
        let snip = snippet_for_content(&long, 80);
        assert!(snip.chars().count() <= 81); // 80 + 省略号
        assert!(snip.ends_with('…'));
    }

    // -----------------------------------------------------------------
    // M3 优先级与反馈环（block-rate）pure-function 单测
    // -----------------------------------------------------------------

    fn make_target(id: &str, due_ms: i64, reason: CommitmentReason) -> CommitmentEmitTarget {
        CommitmentEmitTarget {
            id: id.to_string(),
            text: "test".to_string(),
            reason,
            due_at: dt(due_ms),
            is_fallback_due: false,
        }
    }

    /// commitment_priority_key：Overdue 比 Imminent 优先（升序）。
    #[test]
    fn commitment_priority_overdue_before_imminent() {
        let now_ms: i64 = 10_000_000;
        let contact = template();
        let overdue = make_target("a", now_ms - 1_000, CommitmentReason::Overdue);
        let imminent = make_target("b", now_ms + 1_000, CommitmentReason::Imminent);
        let key_a = commitment_priority_key(&contact, &overdue, &cfg());
        let key_b = commitment_priority_key(&contact, &imminent, &cfg());
        assert!(key_a < key_b, "overdue {:?} should sort before imminent {:?}", key_a, key_b);
    }

    /// commitment_priority_key：同 reason 同 due 时，stage 价值高（commitment_followup）优先于 new_contact。
    #[test]
    fn commitment_priority_value_weight_breaks_tie() {
        let now_ms: i64 = 10_000_000;
        let due = now_ms - 1_000;
        let target = make_target("x", due, CommitmentReason::Overdue);
        let high = Contact {
            domain_attributes: Some(attrs_with_stage("commitment_followup")),
            ..template()
        };
        let low = Contact {
            domain_attributes: Some(attrs_with_stage("new_contact")),
            ..template()
        };
        assert!(
            commitment_priority_key(&high, &target, &cfg()) < commitment_priority_key(&low, &target, &cfg()),
            "commitment_followup should sort before new_contact"
        );
    }

    /// commitment_priority_key：同 reason / 同 stage 时，high intent 优先于 low。
    #[test]
    fn commitment_priority_intent_weight_breaks_tie() {
        let now_ms: i64 = 10_000_000;
        let due = now_ms - 1_000;
        let target = make_target("x", due, CommitmentReason::Overdue);
        let hot = Contact {
            domain_attributes: Some(attrs_with_stage_intent("solution_fit", "high")),
            ..template()
        };
        let cold = Contact {
            domain_attributes: Some(attrs_with_stage_intent("solution_fit", "low")),
            ..template()
        };
        assert!(commitment_priority_key(&hot, &target, &cfg()) < commitment_priority_key(&cold, &target, &cfg()));
    }

    /// commitment_priority_key：同 reason / stage / intent 时，更早 due_at 的更优先。
    #[test]
    fn commitment_priority_earlier_due_first() {
        let now_ms: i64 = 10_000_000;
        let contact = Contact {
            ..template()
        };
        let early = make_target("e", now_ms - 5_000, CommitmentReason::Overdue);
        let late = make_target("l", now_ms - 1_000, CommitmentReason::Overdue);
        assert!(commitment_priority_key(&contact, &early, &cfg()) < commitment_priority_key(&contact, &late, &cfg()));
    }

    /// stage_stagnation_priority_key：同停滞天数下，commitment_followup 阶段优先于 relationship_building。
    #[test]
    fn stage_stagnation_priority_higher_value_first() {
        let now_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        let updated = dt(0);
        let _ = updated;
        let high = Contact {
            domain_attributes: Some(attrs_with_stage_updated("commitment_followup", 0)),
            ..template()
        };
        let low = Contact {
            domain_attributes: Some(attrs_with_stage_updated("relationship_building", 0)),
            ..template()
        };
        assert!(
            stage_stagnation_priority_key(&high, now_ms, &cfg())
                < stage_stagnation_priority_key(&low, now_ms, &cfg()),
            "commitment_followup should sort before relationship_building"
        );
    }

    /// stage_stagnation_priority_key：同 stage 下，停滞更久（30d）优先于停滞较短（15d）。
    #[test]
    fn stage_stagnation_priority_more_stagnant_first() {
        let now_ms: i64 = 30 * 24 * 60 * 60 * 1000;
        let stale_30d = Contact {
            domain_attributes: Some(attrs_with_stage_updated("solution_fit", 0)),
            ..template()
        };
        let stale_15d = Contact {
            domain_attributes: Some(attrs_with_stage_updated(
                "solution_fit",
                15 * 24 * 60 * 60 * 1000,
            )),
            ..template()
        };
        assert!(
            stage_stagnation_priority_key(&stale_30d, now_ms, &cfg())
                < stage_stagnation_priority_key(&stale_15d, now_ms, &cfg()),
            "30d stale should sort before 15d stale"
        );
    }

    /// stage_priority_weight：覆盖默认分支（None / 未识别 stage）走 fallback=20，
    /// 以及真实种子 id 的梯度（漏斗推进度）。
    #[test]
    fn stage_priority_weight_fallback() {
        assert_eq!(stage_priority_weight(None), 20);
        assert_eq!(stage_priority_weight(Some("nonsense")), 20);
        assert_eq!(stage_priority_weight(Some("commitment_followup")), 100);
        assert_eq!(stage_priority_weight(Some("need_discovery")), 60);
    }

    /// intent_level_weight：覆盖默认分支 + 真实种子档位（high/medium/low）。
    #[test]
    fn intent_level_weight_fallback() {
        assert_eq!(intent_level_weight(None), 10);
        assert_eq!(intent_level_weight(Some("high")), 80);
        assert_eq!(intent_level_weight(Some("medium")), 50);
        assert_eq!(intent_level_weight(Some("low")), 20);
        assert_eq!(intent_level_weight(Some("nonsense")), 10);
    }

    /// 1C 等价护栏：空 config（PlannerStageConfig::default）的 stage_weight /
    /// intent_weight / is_terminal_stage 必须与写死函数 / TERMINAL_STAGES 逐字一致。
    #[test]
    fn empty_config_falls_back_to_hardcoded_verbatim() {
        let c = PlannerStageConfig::default();
        for stage in [
            Some("commitment_followup"),
            Some("objection_handling"),
            Some("solution_fit"),
            Some("need_discovery"),
            Some("relationship_building"),
            Some("new_contact"),
            Some("customer_success"),
            Some("nonsense"),
            None,
        ] {
            assert_eq!(c.stage_weight(stage), stage_priority_weight(stage), "stage={stage:?}");
        }
        for level in [Some("high"), Some("medium"), Some("low"), Some("x"), None] {
            assert_eq!(c.intent_weight(level), intent_level_weight(level), "level={level:?}");
        }
        for stage in ["customer_success", "cooldown", "dormant_reactivation"] {
            assert!(c.is_terminal_stage(stage), "{stage} 应为终态");
        }
        for stage in ["new_contact", "need_discovery", "commitment_followup"] {
            assert!(!c.is_terminal_stage(stage), "{stage} 不应为终态");
        }
        assert_eq!(c.stagnation_dimension, "customer_stage");
    }

    /// 1C：非空 config（来自字典）覆盖写死值——权重表 / 终态集 / 停滞维度都可换。
    /// 这是通用化的核心：换行业 = 换一份 profile/字典，planner 逻辑不变。
    #[test]
    fn populated_config_overrides_hardcoded() {
        let mut stage_weights = std::collections::HashMap::new();
        // 故意给一个与写死值不同的权重，证明读的是字典。
        stage_weights.insert("relationship_building".to_string(), 999);
        let mut terminal = std::collections::HashSet::new();
        // 故意把一个非默认 stage 标终态，证明终态集来自字典。
        terminal.insert("custom_done".to_string());
        let c = PlannerStageConfig {
            stage_weights,
            intent_weights: std::collections::HashMap::new(),
            terminal_stages: terminal,
            stagnation_dimension: "relationship_closeness".to_string(),
        };
        // 字典命中 → 用字典值。
        assert_eq!(c.stage_weight(Some("relationship_building")), 999);
        // 字典未命中该 stage → 回落写死。
        assert_eq!(c.stage_weight(Some("need_discovery")), 60);
        // 终态集非空时以字典为准：默认销售终态不再算终态，自定义 stage 算终态。
        assert!(c.is_terminal_stage("custom_done"));
        assert!(!c.is_terminal_stage("customer_success"));
        assert_eq!(c.stagnation_dimension, "relationship_closeness");
    }

    /// classify_review_status：覆盖 5 闸 + 预算 + ok-like + 未知。
    #[test]
    fn classify_review_status_buckets() {
        // blocked-like
        assert_eq!(classify_review_status("held_by_ai_policy"), (1, 0));
        assert_eq!(classify_review_status("blocked_by_safety_guard"), (1, 0));
        assert_eq!(classify_review_status("ai_waiting_for_more_context"), (1, 0));
        assert_eq!(classify_review_status("blocked_unverified_product_claim"), (1, 0));
        assert_eq!(classify_review_status("blocked_by_budget"), (1, 0));
        assert_eq!(classify_review_status("revision_failed"), (1, 0));
        // ok-like
        assert_eq!(classify_review_status("approved"), (0, 1));
        assert_eq!(classify_review_status("revision_applied_approved"), (0, 1));
        assert_eq!(classify_review_status("local_decision_review"), (0, 1));
        // legacy / unknown / empty
        assert_eq!(classify_review_status("legacy_mode_unchecked"), (0, 0));
        assert_eq!(classify_review_status(""), (0, 0));
        assert_eq!(classify_review_status("nonsense"), (0, 0));
    }

    /// EMIT_EVENT_KINDS 不应包含 `*_backoff` —— backoff 不消耗 daily cap。
    #[test]
    fn backoff_event_kinds_not_in_emit_event_kinds() {
        assert!(!EMIT_EVENT_KINDS.contains(&"strategic_planner_silent_backoff"));
        assert!(!EMIT_EVENT_KINDS.contains(&"strategic_planner_commitment_backoff"));
        assert!(!EMIT_EVENT_KINDS.contains(&"strategic_planner_stage_stagnation_backoff"));
        assert!(!EMIT_EVENT_KINDS.contains(&"strategic_planner_capped"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // universal-domain-adaptation H8：运营范式 OperationMode + resolve_operation_mode。
    // 锁死：① DEFAULT(无 profile/无 override) = 三驱动力全开 + 阈值 None（金标零变化护栏）；
    // ② contact override 整组替换 profile；③ profile 范式在无 override 时生效。
    // ─────────────────────────────────────────────────────────────────────

    /// H8：OperationMode::default() = 三驱动力 enabled=true + 所有阈值 None。
    /// 这是 planner 金标零变化的根护栏——DEFAULT 下三扫描器的 enabled 短路都不触发、
    /// 阈值全部回落全局 config。
    #[test]
    fn h8_default_operation_mode_all_enabled_thresholds_none() {
        let m = crate::models::OperationMode::default();
        assert!(m.funnel.enabled, "funnel 默认开");
        assert!(m.silence.enabled, "silence 默认开");
        assert!(m.commitment.enabled, "commitment 默认开");
        assert_eq!(m.funnel.stagnation_threshold_days, None, "阈值默认 None 回落 config");
        assert_eq!(m.silence.threshold_hours, None);
        assert_eq!(m.commitment.imminent_window_hours, None);
    }

    /// H8：无 override 时 resolve_operation_mode 返回 profile 范式（逐字）。
    #[test]
    fn h8_resolve_falls_back_to_profile_when_no_override() {
        let contact = template();
        assert!(contact.operation_mode_override.is_none(), "template 默认无 override");
        let profile_mode = crate::models::OperationMode {
            funnel: crate::models::FunnelMode { enabled: false, stagnation_threshold_days: Some(30) },
            ..crate::models::OperationMode::default()
        };
        let resolved = resolve_operation_mode(&contact, &profile_mode);
        assert_eq!(resolved, profile_mode, "无 override → 用 profile 范式");
        assert!(!resolved.funnel.enabled, "profile 关 funnel 生效");
    }

    /// H8：contact override 整组替换 profile（不逐驱动力 merge）。
    #[test]
    fn h8_resolve_contact_override_replaces_profile() {
        let mut contact = template();
        // 该客户「只维护不推进」：单独关 funnel。
        contact.operation_mode_override = Some(crate::models::OperationMode {
            funnel: crate::models::FunnelMode { enabled: false, stagnation_threshold_days: None },
            silence: crate::models::SilenceMode { enabled: true, threshold_hours: Some(240) },
            ..crate::models::OperationMode::default()
        });
        // profile 范式三全开（销售型）——但 override 优先。
        let profile_mode = crate::models::OperationMode::default();
        let resolved = resolve_operation_mode(&contact, &profile_mode);
        assert!(!resolved.funnel.enabled, "override 关 funnel 覆盖 profile 的开");
        assert_eq!(resolved.silence.threshold_hours, Some(240), "override 自定义静默阈值生效");
    }

    /// H8 DEFAULT 等价：默认范式下，三驱动力的有效阈值 == 传入的全局 config 值
    /// （None → unwrap_or(global)），证明无配置时 planner 用的还是全局 config。
    #[test]
    fn h8_default_mode_effective_thresholds_equal_global() {
        let m = crate::models::OperationMode::default();
        let global_silent_hours = 48_i64;
        let global_stagnation_days = 7_i64;
        let global_imminent_hours = 12_i64;
        assert_eq!(m.silence.threshold_hours.unwrap_or(global_silent_hours), global_silent_hours);
        assert_eq!(
            m.funnel.stagnation_threshold_days.unwrap_or(global_stagnation_days),
            global_stagnation_days
        );
        assert_eq!(
            m.commitment.imminent_window_hours.unwrap_or(global_imminent_hours),
            global_imminent_hours
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // universal-domain-adaptation H19：作息门控纳入 operation_mode override 链。
    // 锁死：① DEFAULT(无 override) → 沿用全局 enabled（金标零变化）；② Some(false)
    // 关闭静默（情感陪伴夜间黄金时段）；③ Some(true) 强制开启。
    // ─────────────────────────────────────────────────────────────────────

    /// H19：无 override → effective == 全局 enabled（两种全局取值都验证）。
    #[test]
    fn h19_no_override_follows_global() {
        let contact = template();
        assert!(contact.operation_mode_override.is_none());
        assert!(crate::agent::quiet_hours::effective_quiet_hours_enabled(&contact, true));
        assert!(!crate::agent::quiet_hours::effective_quiet_hours_enabled(&contact, false));
    }

    /// H19：override Some(false) → 关闭静默（即便全局开），夜间不被压制。
    #[test]
    fn h19_override_false_disables_quiet_hours() {
        let mut contact = template();
        contact.operation_mode_override = Some(crate::models::OperationMode {
            quiet_hours: crate::models::QuietHoursMode { enabled_override: Some(false) },
            ..crate::models::OperationMode::default()
        });
        // 全局开，但 contact 范式关 → 有效为关。
        assert!(!crate::agent::quiet_hours::effective_quiet_hours_enabled(&contact, true));
    }

    /// H19：override Some(true) → 强制开启（即便全局关）。
    #[test]
    fn h19_override_true_forces_quiet_hours() {
        let mut contact = template();
        contact.operation_mode_override = Some(crate::models::OperationMode {
            quiet_hours: crate::models::QuietHoursMode { enabled_override: Some(true) },
            ..crate::models::OperationMode::default()
        });
        assert!(crate::agent::quiet_hours::effective_quiet_hours_enabled(&contact, false));
    }

    /// H19：QuietHoursMode 默认 enabled_override = None（DEFAULT 等价根护栏）。
    #[test]
    fn h19_default_quiet_hours_mode_is_none() {
        assert_eq!(
            crate::models::QuietHoursMode::default().enabled_override,
            None
        );
        // OperationMode::default() 内含的 quiet_hours 也是 None。
        assert_eq!(
            crate::models::OperationMode::default().quiet_hours.enabled_override,
            None
        );
    }
}
