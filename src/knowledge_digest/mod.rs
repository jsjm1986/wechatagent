//! 知识库日报工作站 worker 入口（knowledge-digest-workstation）。
//!
//! 设计见 `.kiro/specs/knowledge-digest-workstation/{requirements,design,tasks}.md`
//! 与 `docs/agent-policy.md` 知识库日报工作站章节。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、
//! `crate::mcp::*`、`agent_send_outbox` 写入路径或 `run_user_operation_gateway`
//! 等生产链路入口。日报合成是离线分析任务，与对话发送链路彻底隔离。
//!
//! Phase 1（本波）：仅 worker 骨架 + early-return on disabled flag + Phase 2
//! 的 `generate_today_digest` 占位（`todo!()`），路由 `GET /api/knowledge/digest/today`
//! 在未命中时直接 404，不触发同步合成。

use std::time::Duration;

use chrono::{Local, NaiveTime, TimeZone};
use tokio::time::sleep;

use crate::routes::AppState;

/// 主循环：`KNOWLEDGE_DIGEST_ENABLED=false` 时立即 return，等价于功能未启用。
///
/// 启用时按 `KNOWLEDGE_DIGEST_RUN_HOUR`（运营时区，默认 9）计算到下一次本地
/// 时间该小时整点的 sleep 时长，醒来跑一次 [`generate_today_digest`]，再 sleep
/// 到次日同一时刻。日内手动重算走 `POST /api/knowledge/digest/regenerate`，
/// 不依赖此 loop。
pub async fn worker_loop(state: AppState) {
    if !state.config.knowledge_digest_enabled {
        tracing::info!(
            "knowledge digest worker disabled (KNOWLEDGE_DIGEST_ENABLED=false); skip spawn"
        );
        return;
    }
    let run_hour = state.config.knowledge_digest_run_hour.min(23);
    tracing::info!(
        run_hour,
        "knowledge digest worker starting (Phase 1 skeleton — generate_today_digest is todo!())"
    );
    loop {
        let wait = duration_until_next_run(run_hour);
        tracing::debug!(?wait, "knowledge digest worker sleeping until next run");
        sleep(wait).await;
        if let Err(err) = generate_today_digest(&state).await {
            tracing::warn!(?err, "knowledge digest tick failed; continuing");
        }
    }
}

/// 计算从现在到下一次 `run_hour:00` 的本地时间间隔。今天 `run_hour` 还没到则等到今天，
/// 否则等到次日。
fn duration_until_next_run(run_hour: u32) -> Duration {
    let now = Local::now();
    let target_today = Local
        .from_local_datetime(
            &now.date_naive()
                .and_time(NaiveTime::from_hms_opt(run_hour, 0, 0).unwrap_or_default()),
        )
        .single();
    let target = match target_today {
        Some(t) if t > now => t,
        _ => {
            // 今天已过 → 次日
            let next_day = now.date_naive().succ_opt().unwrap_or(now.date_naive());
            Local
                .from_local_datetime(
                    &next_day.and_time(NaiveTime::from_hms_opt(run_hour, 0, 0).unwrap_or_default()),
                )
                .single()
                .unwrap_or(now + chrono::Duration::hours(24))
        }
    };
    let delta = (target - now).to_std().unwrap_or(Duration::from_secs(60));
    // 至少 sleep 60s，避免边界条件死循环（now == target 时 delta=0）。
    if delta < Duration::from_secs(60) {
        Duration::from_secs(60)
    } else {
        delta
    }
}

/// 生成当日 `knowledge_daily_reports` 记录。
///
/// Phase 2 落地：扫描 4 数据源（chunks 完整度 / hit-rate / blocked runs / evolution
/// proposals）→ `knowledge.digest.compose` LLM → 卡片数组 → upsert by
/// `(workspace_id, account_id, report_date)`。
///
/// Phase 1 仅占位 `todo!()`，配合 worker disabled 默认值（`KNOWLEDGE_DIGEST_ENABLED=false`）
/// 整体不会被调用到。
async fn generate_today_digest(_state: &AppState) -> anyhow::Result<()> {
    // Phase 2 实装
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_until_next_run_is_positive() {
        let d = duration_until_next_run(9);
        assert!(d.as_secs() >= 60, "duration must be at least 60s, got {:?}", d);
        assert!(
            d.as_secs() <= 24 * 3600,
            "duration must be ≤ 24h, got {:?}",
            d
        );
    }

    #[test]
    fn duration_until_next_run_clamps_invalid_hour() {
        // 超过 23 的 hour 在 worker_loop 里会先 .min(23)，但本函数本身收 u32，
        // 给一个 23 的边界值确保不 panic。
        let d = duration_until_next_run(23);
        assert!(d.as_secs() >= 60);
    }
}
