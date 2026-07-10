//! m029：运营池真人化——清理存量 contacts 的身份污染。
//!
//! 修 3 个存量问题（webhook 建档 bug 遗留，2026-07-10 117 亲验）：
//! 1. 删非真人 normal 记录（gh_ 公众号 / @chatroom 群，本不该进运营池）。
//! 2. 剩余 contacts 按 roster 快照回填正确 nickname/avatar_url。
//! 3. nickname == "Demi"（账号自己昵称，find_string 递归误取）且 roster 未命中 → 置 None。
//!
//! 安全红线：只碰 nickname/avatar_url/删非真人 normal 行；绝不动 agent_status/
//! operation_state/画像/记忆；managed 一律保留（只清昵称不删）；无 APP_ENV 守卫
//! （无条件对所有环境存量生效）；幂等；不删 conversation_messages。

use futures::stream::TryStreamExt;
use mongodb::bson::{doc, Document};
use std::collections::HashMap;

use crate::db::Database;
use crate::error::AppResult;
use crate::webhooks::is_operatable_person;

/// `pub`（非 `pub(super)`）：集成测试 `tests/m029_cleanup_contact_identity.rs` 在独立
/// crate 里直接调用本函数验证清理语义，与 m018 先例一致。
pub async fn run_step(db: &Database) -> AppResult<()> {
    // (1) 删非真人 normal 记录。managed 一律保留（哪怕 gh_/群，只会在 step 3 被回填昵称，绝不删）。
    let mut deleted = 0u64;
    let mut cursor = db
        .contacts()
        .find(doc! { "agent_status": "normal" }, None)
        .await?;
    let mut normal_wxids: Vec<String> = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        normal_wxids.push(c.wxid);
    }
    for wxid in &normal_wxids {
        if !is_operatable_person(wxid) {
            let r = db
                .contacts()
                .delete_many(doc! { "wxid": wxid, "agent_status": "normal" }, None)
                .await?;
            deleted += r.deleted_count;
        }
    }

    // (2) 建 wxid -> (nickname, avatar_url) 映射（遍历所有 roster 快照）。
    //     migration 不限定单一 account，故遍历所有 (workspace, account) 快照建全局映射。
    let mut identity: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut snap_cursor = db.roster_snapshots().find(doc! {}, None).await?;
    while let Some(snap) = snap_cursor.try_next().await? {
        for f in snap.friends {
            identity.entry(f.wxid).or_insert((f.nickname, f.avatar_url));
        }
    }

    // (3) 遍历剩余 contacts：roster 命中→回填 nickname/avatar_url；nickname=="Demi" 且未命中→置 None。
    let mut enriched = 0u64;
    let mut demi_cleared = 0u64;
    let mut all_cursor = db.contacts().find(doc! {}, None).await?;
    while let Some(c) = all_cursor.try_next().await? {
        let wxid = c.wxid.clone();
        let mut set = Document::new();
        let mut unset = Document::new();
        match identity.get(&wxid) {
            Some((nick, avatar)) => {
                if let Some(n) = nick {
                    set.insert("nickname", n);
                }
                if let Some(a) = avatar {
                    set.insert("avatar_url", a);
                }
            }
            None => {
                // roster 未命中 + nickname 是账号自己昵称 "Demi" → 清掉（回落 wxid 显示）。
                if c.nickname.as_deref() == Some("Demi") {
                    unset.insert("nickname", "");
                    demi_cleared += 1;
                }
            }
        }
        if set.is_empty() && unset.is_empty() {
            continue;
        }
        let mut update = Document::new();
        if !set.is_empty() {
            update.insert("$set", set);
            enriched += 1;
        }
        if !unset.is_empty() {
            update.insert("$unset", unset);
        }
        db.contacts()
            .update_one(
                doc! { "wxid": &wxid, "account_id": &c.account_id, "workspace_id": &c.workspace_id },
                update,
                None,
            )
            .await?;
    }

    tracing::info!(
        migration_id = "2026_07_029_cleanup_contact_identity",
        deleted_non_person = deleted,
        enriched_from_roster = enriched,
        demi_cleared = demi_cleared,
        "cleaned up contact identity pollution"
    );
    Ok(())
}
