//! Phase C / C5 历史 threshold auto-release 实现（HC-017 当前政策休眠）。
//!
//! 当前产品边界是“全部 proposal 由管理员显式发布”。[`CURRENT_AUTO_RELEASE_POLICY_ENABLED`]
//! 固定为 false，因此配置总闸和 workspace 子闸即使同时误开，本模块也在任何查询或
//! 写入前返回零。下方决策实现仅为将来经产品确认后加入“类型+方向白名单”保留，
//! 不能被现有配置单独激活。
//!
//! 触发链路：
//! 1. 演化器 tick 末尾调 [`auto_release_eligible_thresholds`]；
//! 2. 扫描 `proposals.proposal_kind="threshold" AND status="eligible_for_release"`；
//! 3. 对每条候选回看 `evolution_auto_release_window_hours` 小时窗口的
//!    `agent_run_logs`，统计该 gate 的命中率；
//! 4. 若命中率仍在 [`super::threshold::THRESHOLD_REASONABLE_BANDS`] 之外（方向
//!    与候选方向一致，意味着信号没有自然回正） → 调
//!    [`super::release::release_threshold`]，admin id=`"evolution_auto_release"`；
//! 5. release_threshold 内部会自动 schedule +24h post_release_review，
//!    "经 post_release 评估通过才生效"由现有 review 路径承担（不自动回滚 ——
//!    Requirements 9.7：post-release 仅观测，回滚必须经 admin）。
//!
//! 历史实现仅覆盖 threshold；prompt 从未进入该路径。rollback 始终只能由 admin
//! 发起。若未来开放白名单，仍必须先修改代码政策常量并补齐方向化回归证据。
//!
//! **隔离红线**：本文件继承 `evolution/` 模块的红线——严禁引用
//! `crate::agent::gateway / outbox`、`crate::mcp::*` 等生产链路入口。

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};

use crate::routes::AppState;

use super::error::EvolutionError;
use super::threshold::THRESHOLD_REASONABLE_BANDS;

/// Product policy (HC-017): every Evolution proposal requires a human release
/// during the current pre-production phase. Configuration flags remain in the
/// schema for a future typed/directional allowlist, but cannot enable automatic
/// release until this policy constant is deliberately changed with new tests.
pub const CURRENT_AUTO_RELEASE_POLICY_ENABLED: bool = false;

/// 代码政策硬闸 AND 历史 env 总闸 AND workspace 子闸。当前第一项恒 false。
fn auto_release_gate_open(env_enabled: bool, flag_threshold_enabled: Option<bool>) -> bool {
    CURRENT_AUTO_RELEASE_POLICY_ENABLED && env_enabled && flag_threshold_enabled.unwrap_or(false)
}

/// 单 tick 自动 release 主入口。返回本 tick 实际触发自动 release 的条数。
///
/// 当前代码政策硬闸为 false → 立即 return Ok(0)。任何下游错误均
/// 被吞掉转 warn，避免一条候选的失败拖累整个 tick；调用方（`run_one_tick`）也
/// 已用 `unwrap_or_else` 兜底。
pub async fn auto_release_eligible_thresholds(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> Result<usize, EvolutionError> {
    if !CURRENT_AUTO_RELEASE_POLICY_ENABLED || !state.config.evolution_auto_release_enabled {
        return Ok(0);
    }
    // EVO-3:总闸(env)开后,再读该 workspace 的子闸 threshold_auto_release_enabled。
    // 子闸关/文档缺失/读失败 → 不自动 release(保守,镜像 is_evolution_enabled_for 顺序)。
    // load_runtime_flag 返回 AppResult,这里 .ok().flatten() 把读失败也视作"子闸未开"——
    // auto_release 整体 best-effort,调用方 run_one_tick 已 unwrap_or_else 兜底,不让读失败透传。
    let flag_threshold = crate::evolution::runtime_flag::load_runtime_flag(state, workspace_id)
        .await
        .ok()
        .flatten()
        .map(|f| f.threshold_auto_release_enabled);
    if !auto_release_gate_open(state.config.evolution_auto_release_enabled, flag_threshold) {
        return Ok(0);
    }
    let cap = state.config.evolution_auto_release_per_tick_cap.max(1);
    let window_hours = state.config.evolution_auto_release_window_hours.max(1) as i64;

    // 1. 拉所有 eligible_for_release threshold proposal。
    let proposals: Vec<crate::models::Proposal> = state
        .db
        .proposals()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "proposal_kind": "threshold",
                "status": "eligible_for_release",
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .try_collect()
        .await
        .map_err(EvolutionError::from)?;

    if proposals.is_empty() {
        return Ok(0);
    }

    // 2. 算窗口内每 gate 的命中率（一次扫描复用给所有候选）。
    let now = DateTime::now();
    let window_start = DateTime::from_millis(
        now.timestamp_millis()
            .saturating_sub(window_hours * 3600 * 1000),
    );
    let hit_rates =
        compute_window_gate_hit_rates(state, workspace_id, account_id, window_start).await?;

    // 2.5-main-4：负反应强制门。仅当开关开启时算一次当前窗口的**绝对**负反应率
    // （per-tick 复用给所有候选，与 hit_rates 同窗口 [window_start, now) 同口径、
    // 同极性源——复用 post_release 的 compute_negative_reaction_rate）。门关时跳过
    // 计算，零额外开销、字节等价。
    let neg_gate_enabled = state
        .config
        .evolution_auto_release_negative_reaction_gate_enabled;
    let negative_reaction_rate = if neg_gate_enabled {
        super::post_release::compute_negative_reaction_rate(
            state,
            workspace_id,
            account_id,
            window_start,
            now,
        )
        .await?
    } else {
        None
    };

    // 3. 顺序处理候选；命中 cap 后跳过余下。
    let mut released = 0_usize;
    for proposal in proposals {
        if released >= cap {
            break;
        }
        let proposal_id = match proposal.id {
            Some(id) => id,
            None => continue,
        };
        let gate_key = match proposal.gate_key.as_deref() {
            Some(g) => g,
            None => {
                tracing::warn!(
                    ?proposal_id,
                    "auto_release: eligible threshold proposal missing gate_key; skip"
                );
                continue;
            }
        };
        let band = THRESHOLD_REASONABLE_BANDS
            .iter()
            .find(|(k, _, _)| *k == gate_key);
        let (lower, upper) = match band {
            Some((_, l, u)) => (*l, *u),
            None => {
                tracing::warn!(
                    proposal_id = ?proposal_id,
                    gate_key,
                    "auto_release: gate_key not in THRESHOLD_REASONABLE_BANDS; skip"
                );
                continue;
            }
        };
        let observed = hit_rates.get(gate_key).copied();
        let decision = decide_auto_release(
            observed,
            lower,
            upper,
            proposal.current_value,
            proposal.proposed_value,
        );

        // 2.5-main-4：放行判定为 true 时，再过负反应强制门——命中则强制改判 SKIP，
        // 拒绝自动放行、退回 admin 显式判断（非回滚，不触碰 Req 9.7）。门关时 forced_skip
        // 恒为 false，final_decision == decision，字节等价。
        let forced_skip = decision
            && decide_negative_reaction_block(
                neg_gate_enabled,
                negative_reaction_rate,
                state
                    .config
                    .evolution_auto_release_max_negative_reaction_rate,
            );
        let final_decision = decision && !forced_skip;

        // 决策事件先写——无论 release 成功失败都留审计。
        let _ = write_auto_release_decision_event(
            state,
            workspace_id,
            account_id,
            proposal_id,
            gate_key,
            observed,
            lower,
            upper,
            final_decision,
            forced_skip,
            negative_reaction_rate,
        )
        .await;

        if !final_decision {
            continue;
        }

        match super::release::release_threshold(
            state,
            proposal_id,
            workspace_id,
            account_id,
            "evolution_auto_release",
        )
        .await
        {
            Ok(()) => {
                released += 1;
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    ?proposal_id,
                    gate_key,
                    "auto_release: release_threshold failed; will retry next tick"
                );
            }
        }
    }
    Ok(released)
}

/// 纯函数版本：命中率仍在 band 外**且偏离方向与候选修正方向一致**时释放（true）。
///
/// KE-01：旧实现只判 `rate<lower || rate>upper`（band 外任意一侧），不看候选方向，
/// 与模块 doc「方向与候选方向一致才放行」相悖。命中率跨 band 翻转到相反外侧时会
/// 反向放量（升阈候选在命中率已过低时仍放行、继续把命中率推更低）。
///
/// 方向由 `proposed_value - current_value` 符号表达：
/// - **升阈候选**（proposed>current，阈值调高→命中率将下降）：仅 `rate>upper`（仍过高）放行；
/// - **降阈候选**（proposed<current，阈值调低→命中率将上升）：仅 `rate<lower`（仍过低）放行；
/// - proposed==current（无方向）/ current 或 proposed 缺失 / `observed=None`：保守 SKIP。
///
/// 这是旧逻辑的**安全收窄**：只减少误放行、绝不新增放行。
pub fn decide_auto_release(
    observed: Option<f64>,
    target_lower: f64,
    target_upper: f64,
    current_value: Option<f64>,
    proposed_value: Option<f64>,
) -> bool {
    let Some(rate) = observed else {
        return false; // 无信号不盲动
    };
    let (Some(cur), Some(prop)) = (current_value, proposed_value) else {
        return false; // 缺方向不盲动
    };
    if prop > cur {
        rate > target_upper // 升阈候选：仅命中率仍过高才放行
    } else if prop < cur {
        rate < target_lower // 降阈候选：仅命中率仍过低才放行
    } else {
        false // 无方向变化
    }
}

/// universal-domain-adaptation 2.5-main-4：客户负反应强制门的纯决策核心。
///
/// 在 [`decide_auto_release`] 已判定放行（true）之后追加这道闸：返回 `true` 表示
/// **强制 SKIP**（拒绝自动放行、退回 admin），`false` 表示放行不受本门干预。
///
/// - `enabled=false` → 永远 `false`（字节等价：门关时 auto_release 行为与 main-4 前一致）。
/// - `observed=None`（窗口内无已分类客户反应）→ `false`：无信号不强制 skip（保守，与
///   [`decide_auto_release`] 的「无信号不盲动」一致）。
/// - `observed > max_rate` → `true`：当前绝对负反应率过高，拒绝自动放行阈值放松。
///
/// 注意阈值是**绝对值**（当前窗口负反应率），不是 pre-3 的前/后窗口升幅 delta ——
/// auto_release 在 release 前决策，没有「后窗口」可比。
pub fn decide_negative_reaction_block(enabled: bool, observed: Option<f64>, max_rate: f64) -> bool {
    if !enabled {
        return false;
    }
    match observed {
        None => false,
        Some(rate) => rate > max_rate,
    }
}

/// 在 `[window_start, now)` 区间扫一次 `agent_run_logs`，按 [`THRESHOLD_REASONABLE_BANDS`]
/// 6 个 gate 的命中分类聚合命中率（命中 / 总数）。`total=0` 时返回空 map（与 None 等价）。
///
/// 终态归因与 [`super::threshold::classify_gate_hit`] 同口径：block 类只有
/// fact（held_by_ai_policy）与 product（blocked_unverified_product_claim）两个
/// 有真实终态源；`blocked_by_safety_guard` 来自证据门/fail-closed 基础设施路径，
/// **不归任何 gate**（缺陷 #16）。pressure_risk_block 是软闸无 block 终态——
/// 窗口内恒 0 命中率没有意义，故与 planner 一样**不落 map**（decide_auto_release
/// 收到 None 保守拒释放）。rewrite 类用 `revision_applied=true` 给 human_like /
/// emotional_value 各 +1 命中（反映"draft 不达标曾被 rewrite"的频次）。
async fn compute_window_gate_hit_rates(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    window_start: DateTime,
) -> Result<HashMap<String, f64>, EvolutionError> {
    let runs = state.db.agent_run_logs();
    let base = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "created_at": { "$gte": window_start },
    };
    let total = runs
        .count_documents(base.clone(), None)
        .await
        .map_err(EvolutionError::from)? as f64;
    let mut out: HashMap<String, f64> = HashMap::new();
    if total <= 0.0 {
        return Ok(out);
    }

    // 只为有真实统计源的 gate 建计数（缺陷 #16：pressure 软闸与 planner 一样
    // 无源——不建条目 → map 缺失 → decide_auto_release 收 None 保守拒）。
    let mut counts: HashMap<&'static str, i64> = HashMap::new();
    for (gate, _l, _u) in THRESHOLD_REASONABLE_BANDS {
        if matches!(
            *gate,
            "pressure_risk_block" | "planner_block_rate_threshold"
        ) {
            continue;
        }
        counts.insert(*gate, 0);
    }

    let mut cursor = runs
        .find(base.clone(), None)
        .await
        .map_err(EvolutionError::from)?;
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        match run.final_review_status.as_str() {
            "blocked_unverified_product_claim" => {
                *counts.entry("product_accuracy_score_block").or_default() += 1;
            }
            "held_by_ai_policy" => {
                *counts.entry("fact_risk_block").or_default() += 1;
            }
            // blocked_by_safety_guard：证据门/fail-closed 基础设施终态，不归任何
            // gate（缺陷 #16，与 threshold::classify_gate_hit 同口径）。
            _ => {}
        }
        if run.revision_applied {
            *counts.entry("human_like_score_rewrite").or_default() += 1;
            *counts.entry("emotional_value_rewrite").or_default() += 1;
        }
    }
    for (gate, hit) in counts {
        out.insert(gate.to_string(), hit as f64 / total);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn write_auto_release_decision_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    proposal_id: mongodb::bson::oid::ObjectId,
    gate_key: &str,
    observed: Option<f64>,
    target_lower: f64,
    target_upper: f64,
    decision: bool,
    forced_skip: bool,
    negative_reaction_rate: Option<f64>,
) -> Result<(), EvolutionError> {
    let mut details = doc! {
        "proposal_id": proposal_id,
        "gate_key": gate_key,
        "target_lower": target_lower,
        "target_upper": target_upper,
        "decision_release": decision,
    };
    if let Some(rate) = observed {
        details.insert("hit_rate_observed", rate);
    }
    // 2.5-main-4：负反应强制门命中时落审计标记 + 观测值，供 admin 区分「命中率回正的
    // 自然 skip」与「负反应过高的强制 skip」。门关或未命中时不写这两个字段（天然空缺）。
    if forced_skip {
        details.insert("negative_reaction_forced_skip", true);
    }
    if let Some(rate) = negative_reaction_rate {
        details.insert("negative_reaction_rate_observed", rate);
    }
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_auto_release_decision".to_string(),
        status: if decision { "release" } else { "skip" }.to_string(),
        summary: format!(
            "auto_release decision for {gate_key}: {} (observed={:?}, band=[{:.3},{:.3}]){}",
            if decision { "RELEASE" } else { "SKIP" },
            observed,
            target_lower,
            target_upper,
            if forced_skip {
                format!(
                    " [negative_reaction_forced_skip rate={:?}]",
                    negative_reaction_rate
                )
            } else {
                String::new()
            }
        ),
        details: Some(details),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

/// 给 [`super::run_one_tick`] 的事件 summary 用——把上一段事件的"已自动 release
/// 多少条"压成 [`Document`]，方便 tick_completed 事件附带。
pub fn auto_release_event_details(released: usize) -> Document {
    doc! {
        "auto_released_count": released as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_auto_release_inside_band_skips() {
        // 命中率回到正常区间 → 留给 admin，不自动 release（无论方向）。
        // 升阈候选(6→7)：band 内一律 SKIP。
        assert!(!decide_auto_release(
            Some(0.10),
            0.05,
            0.15,
            Some(6.0),
            Some(7.0)
        ));
        assert!(!decide_auto_release(
            Some(0.05),
            0.05,
            0.15,
            Some(6.0),
            Some(7.0)
        ));
        assert!(!decide_auto_release(
            Some(0.15),
            0.05,
            0.15,
            Some(6.0),
            Some(7.0)
        ));
    }

    #[test]
    fn decide_auto_release_no_signal_skips() {
        // 窗口内无样本：保守拒释放（方向齐备也不放行）。
        assert!(!decide_auto_release(None, 0.05, 0.15, Some(6.0), Some(7.0)));
    }

    // ── KE-01 方向门：升阈候选(proposed>current)仅命中率仍过高(>upper)才放行 ──

    #[test]
    fn decide_auto_release_raise_threshold_releases_only_when_still_above_upper() {
        // 升阈候选(6→7)：命中率仍 > upper（仍过高、需继续降）→ RELEASE。
        assert!(decide_auto_release(
            Some(0.50),
            0.05,
            0.15,
            Some(6.0),
            Some(7.0)
        ));
    }

    #[test]
    fn decide_auto_release_raise_threshold_skips_when_flipped_below_lower() {
        // KE-01 核心修复：升阈候选(6→7)，但命中率已翻转到 < lower（已过低）→ SKIP。
        // 旧逻辑 rate<lower 也放行 = 反向放量把命中率推更低；本测锁死修复（回退即红）。
        assert!(!decide_auto_release(
            Some(0.02),
            0.05,
            0.15,
            Some(6.0),
            Some(7.0)
        ));
    }

    // ── KE-01 方向门：降阈候选(proposed<current)仅命中率仍过低(<lower)才放行 ──

    #[test]
    fn decide_auto_release_lower_threshold_releases_only_when_still_below_lower() {
        // 降阈候选(6→5)：命中率仍 < lower（仍过低、需继续升）→ RELEASE。
        assert!(decide_auto_release(
            Some(0.02),
            0.05,
            0.15,
            Some(6.0),
            Some(5.0)
        ));
    }

    #[test]
    fn decide_auto_release_lower_threshold_skips_when_flipped_above_upper() {
        // 降阈候选(6→5)，但命中率已翻转到 > upper（已过高）→ SKIP（反向放量防护）。
        assert!(!decide_auto_release(
            Some(0.50),
            0.05,
            0.15,
            Some(6.0),
            Some(5.0)
        ));
    }

    #[test]
    fn decide_auto_release_no_direction_skips() {
        // proposed==current（无方向变化）→ SKIP。
        assert!(!decide_auto_release(
            Some(0.50),
            0.05,
            0.15,
            Some(6.0),
            Some(6.0)
        ));
    }

    #[test]
    fn decide_auto_release_missing_value_skips() {
        // current/proposed 任一缺失（无法定方向）→ 保守 SKIP。
        assert!(!decide_auto_release(
            Some(0.50),
            0.05,
            0.15,
            None,
            Some(7.0)
        ));
        assert!(!decide_auto_release(
            Some(0.50),
            0.05,
            0.15,
            Some(6.0),
            None
        ));
    }

    #[test]
    fn auto_release_event_details_serializes_count() {
        let d = auto_release_event_details(3);
        assert_eq!(d.get_i32("auto_released_count").unwrap(), 3);
    }

    // ── 2.5-main-4：负反应强制门纯函数测 ──

    #[test]
    fn negative_reaction_block_disabled_never_blocks() {
        // 门关：任何负反应率（哪怕 1.0）都不强制 skip——字节等价于 main-4 前。
        assert!(!decide_negative_reaction_block(false, Some(1.0), 0.30));
        assert!(!decide_negative_reaction_block(false, Some(0.0), 0.30));
        assert!(!decide_negative_reaction_block(false, None, 0.30));
    }

    #[test]
    fn negative_reaction_block_above_threshold_blocks() {
        // 门开 + 当前绝对负反应率高于阈值 → 强制 skip。
        assert!(decide_negative_reaction_block(true, Some(0.50), 0.30));
        assert!(decide_negative_reaction_block(true, Some(0.31), 0.30));
    }

    #[test]
    fn negative_reaction_block_at_or_below_threshold_allows() {
        // 门开 + 负反应率正常（≤ 阈值）→ 不干预放行。边界相等不算超阈。
        assert!(!decide_negative_reaction_block(true, Some(0.30), 0.30));
        assert!(!decide_negative_reaction_block(true, Some(0.10), 0.30));
        assert!(!decide_negative_reaction_block(true, Some(0.0), 0.30));
    }

    #[test]
    fn negative_reaction_block_no_signal_allows() {
        // 门开但窗口内无已分类客户反应（None）→ 不强制 skip：无信号不盲动，
        // 与 decide_auto_release 的保守口径一致。
        assert!(!decide_negative_reaction_block(true, None, 0.30));
    }

    // ── EVO-3：auto_release 双闸（env 总闸 AND per-workspace 子闸）纯函数测 ──

    #[test]
    fn auto_release_dual_gate() {
        // 总闸关 → 不论子闸都 false
        assert!(!super::auto_release_gate_open(false, Some(true)));
        assert!(!super::auto_release_gate_open(false, None));
        // 总闸开 + 子闸关/缺失 → false(默认保守)
        assert!(!super::auto_release_gate_open(true, Some(false)));
        assert!(!super::auto_release_gate_open(true, None));
        // Current product policy requires human release even when both legacy
        // configuration gates are accidentally enabled.
        assert!(!super::auto_release_gate_open(true, Some(true)));
        assert!(!super::CURRENT_AUTO_RELEASE_POLICY_ENABLED);
    }
}
