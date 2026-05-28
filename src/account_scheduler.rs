//! Phase D / D4：多账号调度器。
//!
//! webhook 入站 + planner / cold worker emit 时，先经本调度器决定"哪个账号
//! 来承接这位 contact"。原则：
//!
//! - **粘性优先**：contact 在 `contacts` 表里已经绑定了 `account_id` → 直接复用
//!   （除非该账号离线 / 全天 off_hours）。这避免一个 contact 被来回甩。
//! - **persona 内轮询**：如果是新 contact / 旧绑定不可用，按 contact_wxid 稳定
//!   散列在同 persona 池里挑账号，保证同 wxid 多次进入同一账号（除非池变化）。
//! - **off_hours 跳过**：命中任一区间的账号不被选；全部命中时 fallback 回任一
//!   `online=true` 账号（保送达 > 严格遵守 off_hours）。
//! - **capacity 上限**：达到 capacity 的账号被跳过（capacity=0 表示"不参与多
//!   账号策略"，永远当作未满）。
//!
//! 关键不变量：
//! - **零侵入现有 webhook**：调度器只下游于 `resolve_account_context`，已绑定
//!   contact 仍走原账号；本模块只在新 contact / 重激活无绑定时介入。
//! - **不绕开 outbox**：调度器只决定 `account_id` + 写入 contact 绑定，发送仍
//!   走 gateway / outbox / MCP。
//! - **审计**：每次调度写一条 `account_scheduler_assignment` 事件，便于事后核
//!   对"哪个 contact 在哪个时间被路由到哪个账号"。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{Datelike, Timelike};
use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::models::{HourRange, WechatAccount};
use crate::routes::AppState;

/// 给定 (workspace_id, contact_wxid, persona_tag) → 决策一个 account_id。
///
/// `persona_tag = None` 时回退到"默认 persona"（取 workspace 默认 account），
/// 这是 D4 关停态语义；任何已存在的单账号部署都不受影响。
pub async fn assign_account(
    state: &AppState,
    workspace_id: &str,
    contact_wxid: &str,
    persona_tag: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let accounts = load_persona_pool(state, workspace_id, persona_tag).await?;
    if accounts.is_empty() {
        return Ok(None);
    }
    let now = chrono::Utc::now();
    let day_start = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|nd| nd.and_utc())
        .unwrap_or(now);

    // 当日已分配数（仅本调度器写入的事件计数；现存绑定不计入，避免抑制热账号）。
    let today_count = count_today_assignments(state, workspace_id, day_start.timestamp_millis())
        .await
        .unwrap_or_default();

    let cur_hour = now.hour();
    let mut eligible: Vec<&WechatAccount> = accounts
        .iter()
        .filter(|a| a.online)
        .filter(|a| !is_in_off_hours(&a.off_hours, cur_hour))
        .filter(|a| {
            if a.capacity == 0 {
                return true;
            }
            let used = today_count
                .iter()
                .find(|(id, _)| id == &a.account_id)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            (used as u32) < a.capacity
        })
        .collect();

    // 全部命中 off_hours / 满 capacity → 退化为"任意 online 账号"，保送达。
    if eligible.is_empty() {
        eligible = accounts.iter().filter(|a| a.online).collect();
    }
    if eligible.is_empty() {
        return Ok(None);
    }

    let pick = stable_pick(&eligible, contact_wxid);
    let assigned = pick.account_id.clone();

    // 审计：写一条 account_scheduler_assignment。
    let _ = crate::agent::write_event_for_account(
        state,
        &assigned,
        Some(contact_wxid),
        "account_scheduler_assignment",
        "ok",
        &format!(
            "scheduler assigned wxid={} to account={} persona={}",
            contact_wxid,
            &assigned,
            persona_tag.unwrap_or("default")
        ),
        Some(doc! {
            "workspaceId": workspace_id,
            "contactWxid": contact_wxid,
            "personaTag": persona_tag.unwrap_or(""),
            "poolSize": eligible.len() as i64,
        }),
    )
    .await;
    Ok(Some(assigned))
}

async fn load_persona_pool(
    state: &AppState,
    workspace_id: &str,
    persona_tag: Option<&str>,
) -> anyhow::Result<Vec<WechatAccount>> {
    let mut filter = doc! {
        "workspace_id": workspace_id,
    };
    if let Some(tag) = persona_tag {
        filter.insert("persona_tag", tag);
    }
    let cursor = state.db.accounts().find(filter, None).await?;
    let pool: Vec<WechatAccount> = cursor.try_collect().await?;
    Ok(pool)
}

async fn count_today_assignments(
    state: &AppState,
    workspace_id: &str,
    day_start_ms: i64,
) -> anyhow::Result<Vec<(String, i64)>> {
    use mongodb::bson::Document;
    let pipeline = vec![
        doc! { "$match": {
            "workspace_id": workspace_id,
            "kind": "account_scheduler_assignment",
            "created_at": { "$gte": mongodb::bson::DateTime::from_millis(day_start_ms) },
        }},
        doc! { "$group": {
            "_id": "$account_id",
            "count": { "$sum": 1 },
        }},
    ];
    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .aggregate(pipeline, None)
        .await?;
    let mut out = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        let id = doc.get_str("_id").unwrap_or("").to_string();
        let n = doc.get_i64("count").unwrap_or(0);
        out.push((id, n));
    }
    Ok(out)
}

pub(crate) fn is_in_off_hours(ranges: &[HourRange], cur_hour: u32) -> bool {
    ranges.iter().any(|r| hour_in_range(r, cur_hour))
}

fn hour_in_range(range: &HourRange, cur: u32) -> bool {
    let start = range.start_hour;
    let end = range.end_hour;
    if start == end {
        return false;
    }
    if start < end {
        // 同日区间 [start, end)
        cur >= start && cur < end
    } else {
        // 跨午夜 [start, 24) U [0, end)
        cur >= start || cur < end
    }
}

fn stable_pick<'a>(pool: &'a [&'a WechatAccount], contact_wxid: &str) -> &'a WechatAccount {
    debug_assert!(!pool.is_empty());
    let mut hasher = DefaultHasher::new();
    contact_wxid.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % pool.len();
    pool[idx]
}

/// Phase D / D4：纯决策函数版本的"在已知 pool + 当日已用量 + 当前小时下挑账号"。
///
/// 与 [`assign_account`] 的核心决策逻辑同源（off_hours / capacity / online 过滤
/// + stable_pick 散列），但不读 / 不写 mongo，便于单元 + PBT 直接断言：
/// - **不变量 1（capacity full → fallback only when forced）**：所有账号 capacity
///   均满 + 至少一个 online 时，返回任一 online 账号 ID，**绝不返回 None**（保送达
///   优先）。
/// - **不变量 2（同 wxid 同 pool 决策稳定）**：同 (pool, used, cur_hour, wxid)
///   两次调用必返回同一 account_id。
/// - **不变量 3（容量为 0 视为不限）**：capacity=0 始终参与候选。
/// - **不变量 4（off_hours 命中跳过）**：命中 off_hours 的账号在严格池里不参与，
///   只有"所有候选都被 off_hours / capacity 排除时"才退化到 online-only。
pub fn decide_assigned_account<'a>(
    accounts: &'a [WechatAccount],
    used_today: &[(String, i64)],
    cur_hour: u32,
    contact_wxid: &str,
) -> Option<&'a WechatAccount> {
    if accounts.is_empty() {
        return None;
    }
    let strict_idxs: Vec<usize> = accounts
        .iter()
        .enumerate()
        .filter(|(_, a)| a.online)
        .filter(|(_, a)| !is_in_off_hours(&a.off_hours, cur_hour))
        .filter(|(_, a)| {
            if a.capacity == 0 {
                return true;
            }
            let used = used_today
                .iter()
                .find(|(id, _)| id == &a.account_id)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            (used as u32) < a.capacity
        })
        .map(|(i, _)| i)
        .collect();

    let pool_idxs: Vec<usize> = if !strict_idxs.is_empty() {
        strict_idxs
    } else {
        accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| a.online)
            .map(|(i, _)| i)
            .collect()
    };
    if pool_idxs.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    contact_wxid.hash(&mut hasher);
    let pick = pool_idxs[(hasher.finish() as usize) % pool_idxs.len()];
    Some(&accounts[pick])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::DateTime as BsonDate;

    fn account(id: &str, online: bool, capacity: u32, off: Vec<HourRange>) -> WechatAccount {
        WechatAccount {
            id: None,
            workspace_id: "default".to_string(),
            account_id: id.to_string(),
            alias: id.to_string(),
            display_name: id.to_string(),
            app_id: None,
            wxid: None,
            nick_name: None,
            mcp_base_url: None,
            mcp_api_key: None,
            online,
            last_sync_at: BsonDate::now(),
            capacity,
            persona_tag: Some("sales_assistant".to_string()),
            off_hours: off,
            created_at: BsonDate::now(),
            updated_at: BsonDate::now(),
        }
    }

    #[test]
    fn off_hours_same_day_window() {
        let r = HourRange {
            start_hour: 22,
            end_hour: 24,
        };
        assert!(hour_in_range(&r, 22));
        assert!(hour_in_range(&r, 23));
        assert!(!hour_in_range(&r, 21));
        assert!(!hour_in_range(&r, 0));
    }

    #[test]
    fn off_hours_cross_midnight() {
        let r = HourRange {
            start_hour: 22,
            end_hour: 6,
        };
        assert!(hour_in_range(&r, 23));
        assert!(hour_in_range(&r, 0));
        assert!(hour_in_range(&r, 5));
        assert!(!hour_in_range(&r, 6));
        assert!(!hour_in_range(&r, 12));
    }

    #[test]
    fn off_hours_zero_length_never_matches() {
        let r = HourRange {
            start_hour: 9,
            end_hour: 9,
        };
        for h in 0u32..24 {
            assert!(!hour_in_range(&r, h));
        }
    }

    #[test]
    fn is_in_off_hours_any_match() {
        let ranges = vec![
            HourRange {
                start_hour: 0,
                end_hour: 6,
            },
            HourRange {
                start_hour: 22,
                end_hour: 24,
            },
        ];
        assert!(is_in_off_hours(&ranges, 3));
        assert!(is_in_off_hours(&ranges, 23));
        assert!(!is_in_off_hours(&ranges, 12));
    }

    #[test]
    fn stable_pick_is_deterministic_per_wxid() {
        let a = account("acc_1", true, 100, vec![]);
        let b = account("acc_2", true, 100, vec![]);
        let c = account("acc_3", true, 100, vec![]);
        let pool: Vec<&WechatAccount> = vec![&a, &b, &c];
        let p1 = stable_pick(&pool, "user_xyz").account_id.clone();
        let p2 = stable_pick(&pool, "user_xyz").account_id.clone();
        assert_eq!(p1, p2, "same wxid must pick the same account");
    }

    #[test]
    fn stable_pick_distributes_across_pool() {
        let a = account("acc_1", true, 100, vec![]);
        let b = account("acc_2", true, 100, vec![]);
        let pool: Vec<&WechatAccount> = vec![&a, &b];
        let mut acc1 = 0;
        let mut acc2 = 0;
        for i in 0..200 {
            let pick = stable_pick(&pool, &format!("wxid_{}", i)).account_id.clone();
            if pick == "acc_1" {
                acc1 += 1;
            } else {
                acc2 += 1;
            }
        }
        // 不要求严格 1:1，但两侧都至少有 1/4，否则散列烂得离谱。
        assert!(acc1 > 50 && acc2 > 50, "{} vs {}", acc1, acc2);
    }
}
