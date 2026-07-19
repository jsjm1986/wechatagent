//! 销售素材选材：媒体类型→MCP 工具名映射、候选过滤（纯函数）。
//! 发送执行逻辑（ensure_media_uploaded / send_outbound_media）+ 崩溃恢复防重发核对。
use crate::error::{AppError, AppResult};
use crate::media_storage;
use crate::models::{ContentAsset, ConversationMessage, MessageDirection};
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime};
use serde_json::{json, Value};

/// 媒体类型 → MCP 私聊发送工具名。不写死在调用处，集中此表。
/// 链接卡片/小程序：MCP 私聊侧确认工具名后在此加一行即可，不改调用方。
pub(crate) fn mcp_tool_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image" => Some("message_send_image"),
        "file" => Some("message_send_file"),
        "video" => Some("message_send_video"),
        _ => None,
    }
}

/// 发送前准入二次校验：可发 + 已审核 + 媒体类型能映射到工具名。
/// 与 filter 共用的硬门——老朋友圈行（media_type 为 None）与草稿绝不放行。
pub(crate) fn validate_asset_sendable(asset: &ContentAsset) -> bool {
    asset.sendable == Some(true)
        && asset.review_status.as_deref() == Some("approved")
        && asset
            .media_type
            .as_deref()
            .and_then(mcp_tool_for_media_type)
            .is_some()
}

/// 候选过滤（纯函数）：先过准入硬门，再按 target_stages 命中 customer_stage。
/// target_stages 为 None 或空 = 总命中；非空则需包含当前 stage。
pub(crate) fn filter_sendable_candidates<'a>(
    assets: &'a [ContentAsset],
    customer_stage: Option<&str>,
) -> Vec<&'a ContentAsset> {
    assets
        .iter()
        .filter(|a| validate_asset_sendable(a))
        .filter(|a| match (&a.target_stages, customer_stage) {
            (None, _) => true,
            (Some(stages), _) if stages.is_empty() => true,
            (Some(stages), Some(cs)) => stages.iter().any(|s| s == cs),
            (Some(_), None) => false,
        })
        .collect()
}

/// 把候选素材渲染成注入 prompt 的清单文本（供决策 Agent 选材，没有合适的就不发）。
pub(crate) fn render_candidate_lines(candidates: &[&ContentAsset]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut out = String::from("可发送素材（按需选择，没有合适的就不发）：\n");
    for a in candidates {
        let id = a.id.map(|i| i.to_hex()).unwrap_or_default();
        let stages = a
            .target_stages
            .as_ref()
            .map(|v| v.join(","))
            .unwrap_or_default();
        let pref = a.expression_pref.as_deref().unwrap_or("file_support");
        let hint = a.send_trigger_hint.as_deref().unwrap_or("");
        // tags 非空才渲染标签段（旧素材 tags 空则跳过，向后兼容）。
        let tags_seg = if a.tags.is_empty() {
            String::new()
        } else {
            format!(" | 标签:{}", a.tags.join(","))
        };
        out.push_str(&format!(
            "- [id:{id}] {} | 阶段:{stages} | 表达:{pref}{tags_seg}\n  触发提示:{hint}\n",
            a.title
        ));
    }
    out
}

/// 轻量概览（纯函数）：渐进式三档 Lean/Relational 档恒注入用。只列「标题 + 发送时机
/// （send_trigger_hint）」，**不含** assetId / 文件元数据 / 阶段 / 表达偏好——目的是让
/// Reply Agent 在第一程小档就知道「库里有哪些可发素材 + 运营标注的何时发」，据此自评
/// 本轮客户消息是否契合某条发送时机；契合则它会自评 need_more_context + missingTier=full
/// 升档，到 Full 档再用完整的 [`render_candidate_lines`]（带 assetId）选材输出 assetsToSend。
///
/// 设计取舍：概览段**不带**「契合就升档」的显式指令，只客观罗列素材线索，把「是否升档」
/// 完全交给 LLM 语义判断（agent-first，不硬塞指令、不引入关键词）。空候选返回空串。
pub(crate) fn render_candidate_overview(candidates: &[&ContentAsset]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("可发送素材线索（仅概览；完整清单与发送方式在你确认需要时会提供）：\n");
    for a in candidates {
        let hint = a.send_trigger_hint.as_deref().unwrap_or("").trim();
        if hint.is_empty() {
            out.push_str(&format!("- {}\n", a.title));
        } else {
            out.push_str(&format!("- {} | 发送时机：{hint}\n", a.title));
        }
    }
    // ③升档盲区修复（2026-06-27）：A/B 已证「passive 客观罗列」不足以驱动升档——
    // LLM 在 Lean 档看到素材线索后会自评 enough 停档，导致承诺发素材却拿不到 assetId。
    // 故此处补一句**显式升档引导**（与 ④ assist_hint 同构）：只描述"契合即升档"的语义
    // 条件，不列任何触发关键词（守 agent-first，是否契合由 LLM 判）。
    out.push_str(
        "以上仅为线索概览。若你判断客户当前消息契合其中某条素材的发送时机，应判 \
         sufficiency=need_more_context、missingTier=full，以便加载完整清单（含可发送的素材标识）\
         后再决定是否发送；不契合就正常回复，不为发而发。\n",
    );
    out
}

/// media_id 缓存有效性（纯函数）：`updated_at` 距 `now` 不超过 `ttl_hours`，
/// 且非未来时间（时钟回拨 / 脏数据 → 视为无效，强制重传）。
/// 不依赖"media_id 永久有效"——超 TTL 即过期重传。
pub(crate) fn media_id_cache_valid(updated_at: DateTime, ttl_hours: i64, now: DateTime) -> bool {
    let age_ms = now.timestamp_millis() - updated_at.timestamp_millis();
    age_ms >= 0 && age_ms < ttl_hours * 3600 * 1000
}

/// 媒体发送工具集——崩溃恢复核对时用来圈定"这条记录是一次素材发送"。
const MEDIA_SEND_TOOLS: [&str; 3] = [
    "message_send_image",
    "message_send_file",
    "message_send_video",
];

/// 确保 asset 在 MCP 侧有有效 media_id：缓存命中（media_id 存在且未过 TTL）直接用，
/// 否则读盘 base64 → `media_upload_base64` → 回写 media_id + updated_at。
/// **不依赖"media_id 永久有效"假设**——TTL 过期即重传。
async fn ensure_media_uploaded(state: &AppState, asset: &ContentAsset) -> AppResult<String> {
    let now = DateTime::now();
    if let Some(mid) = asset.media_id.as_ref() {
        if media_id_cache_valid(asset.updated_at, state.config.media_id_cache_ttl_hours, now) {
            return Ok(mid.clone());
        }
    }
    let rel = asset
        .file_path
        .as_ref()
        .ok_or_else(|| AppError::External("asset has no file_path".into()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    let bytes = media_storage::read_bytes(root, rel)
        .await
        .map_err(|e| AppError::External(format!("read media failed: {e}")))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    // MCP 入参字段名以 server tools/list 为准；这里用占位形态，集成时对齐。
    let account_id = asset
        .account_id
        .as_deref()
        .unwrap_or(&state.config.default_account_id);
    let resp = crate::mcp::logged_call_for_account(
        state,
        account_id,
        "media_upload_base64",
        json!({
            "fileName": asset.file_name,
            "mediaType": asset.media_type,
            "base64": b64,
        }),
    )
    .await?;
    let media_id = resp
        .get("mediaId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::External("media_upload_base64 no mediaId".into()))?
        .to_string();

    // 回写缓存：**失败即传播错误、绝不继续发送**。崩溃恢复 (`media_already_succeeded`)
    // 依赖不变式「asset.media_id == None ⇒ 从未发出过 ⇒ 可放行重发」。若此处 best-effort
    // 吞掉回写失败，则会出现「MCP send 成功(文件已送达) 但 media_id 没存上」的状态：
    // 重试时 recovery 读到 media_id 仍为 None → 误判没发过 → 重发，客户收到重复文件。
    // 故宁可这次不发（下次重试会重新上传+回写），也不让「已发未存」状态出现。
    if let Some(oid) = asset.id {
        state
            .db
            .content_assets()
            .update_one(
                doc! { "_id": oid },
                doc! { "$set": { "media_id": &media_id, "updated_at": now } },
                None,
            )
            .await
            .map_err(|e| {
                AppError::External(format!(
                    "persist media_id failed (abort send to avoid dup): {e}"
                ))
            })?;
    }
    Ok(media_id)
}

/// 发送一个素材文件给客户。调用方（dispatcher）已确保经 outbox 幂等。
/// 流程：查 asset → 准入二次校验（防 AI 幻觉出未审素材）→ 媒体类型映射工具名
/// → ensure_media_uploaded → MCP 发送 → 落出站 `ConversationMessage`（带 media 标注）。
pub(crate) async fn send_outbound_media(
    state: &AppState,
    contact: &crate::models::Contact,
    asset_id: &str,
) -> Result<Value, super::types::OutboundSendError> {
    let oid =
        ObjectId::parse_str(asset_id).map_err(|_| AppError::External("bad asset_id".into()))?;
    let asset = state
        .db
        .content_assets()
        // 纵深防御：按 _id + workspace_id 双条件查，杜绝跨租户 IDOR（asset_id 来自 outbox）。
        .find_one(
            doc! { "_id": oid, "workspace_id": &contact.workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    // 发送前准入二次校验（防 AI 幻觉出未审/不可发素材一路漏到发送）。
    if !validate_asset_sendable(&asset) {
        return Err(
            AppError::External("asset not sendable (draft/disabled/bad type)".into()).into(),
        );
    }
    let tool = mcp_tool_for_media_type(asset.media_type.as_deref().unwrap_or(""))
        .ok_or_else(|| AppError::External("unsupported media_type".into()))?;
    let media_id = ensure_media_uploaded(state, &asset).await?;

    let resp = crate::mcp::logged_send_call_for_account(
        state,
        &contact.account_id,
        tool,
        json!({ "recipient": contact.wxid, "mediaId": media_id }),
    )
    .await
    .map_err(super::types::OutboundSendError::from)?;

    if !super::gateway::send_receipt_is_ok(&resp) {
        return Err(super::types::OutboundSendError::SafeToRetry(
            "media send returned a negative or unverifiable delivery receipt".into(),
        ));
    }

    // MCP 已成功 = 文件已送达客户，既成事实。此后落库失败**绝不**返 Err——
    // 否则 dispatcher 会 retry 重发，客户收到重复文件（与 send_outbound_message 对称）。
    let message_id = resp
        .get("newMsgId")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let mut raw = to_document(&resp).unwrap_or_default();
    raw.insert("mediaAssetId", asset_id);
    let now = DateTime::now();
    if let Err(err) = state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id,
                dedupe_key: None,
                direction: MessageDirection::Outbound,
                content: asset.title.clone(),
                msg_type: Some("media".to_string()),
                media_ref: Some(asset_id.to_string()),
                raw: Some(raw),
                is_synthetic_relay: false,
                created_at: now,
            },
            None,
        )
        .await
    {
        tracing::error!(
            account_id = %contact.account_id,
            contact_wxid = %contact.wxid,
            error = %err,
            "MCP media send succeeded but persisting outbound conversation_messages failed; file delivered but record missing",
        );
    }
    Ok(resp)
}

/// 媒体条目崩溃恢复核对（硬伤④）：按 **media_id** 定位该 asset 的成功发送记录。
///
/// 可靠性依据：素材发送前必走 `ensure_media_uploaded`，它在 send 之前把 media_id
/// 回写到 `content_assets`，且 MCP send 请求体携带 `request.mediaId`。故：
/// - asset.media_id 为 None ⇒ 从未完成上传回写 ⇒ 客户投递不可能发生；
/// - asset.media_id 为 Some 且命中成功日志 ⇒ 已送达；
/// - asset.media_id 为 Some 但本地日志未命中 ⇒ 本地日志不是权威远端查询，只能判
///   `Inconclusive`，禁止把“缺证据”误作“确认未送达”而重发。
async fn mcp_media_delivery_verification(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    asset_id: &str,
    entry_created_at: DateTime,
) -> AppResult<super::types::DeliveryVerification> {
    use super::types::DeliveryVerification;
    let oid = match ObjectId::parse_str(asset_id) {
        Ok(o) => o,
        Err(_) => return Ok(DeliveryVerification::NotDelivered),
    };
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid }, None)
        .await?;
    // media_id 缺失 ⇒ send 的客户投递步骤尚不可能发生 ⇒ 安全放行。
    let media_id = match asset.and_then(|a| a.media_id) {
        Some(m) => m,
        None => return Ok(DeliveryVerification::NotDelivered),
    };
    let lower_bound_millis = entry_created_at
        .timestamp_millis()
        .saturating_sub(5 * 60 * 1000);
    let lower_bound = DateTime::from_millis(lower_bound_millis);
    let count = state
        .db
        .mcp_logs()
        .count_documents(
            doc! {
                "account_id": account_id,
                "tool_name": { "$in": MEDIA_SEND_TOOLS.to_vec() },
                "request.recipient": contact_wxid,
                "request.mediaId": &media_id,
                "error": null,
                "$or": [
                    { "response.ok": true },
                    {
                        "response.ok": { "$exists": false },
                        "response.newMsgId": { "$type": "string", "$ne": "" },
                    },
                ],
                "created_at": { "$gte": lower_bound },
            },
            None,
        )
        .await?;
    Ok(if count > 0 {
        DeliveryVerification::Delivered
    } else {
        DeliveryVerification::Inconclusive
    })
}

/// dispatcher 在媒体条目崩溃恢复 / timeout / 歧义错误分支调用。
pub(crate) async fn media_delivery_verification(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    asset_id: &str,
    entry_created_at: DateTime,
) -> AppResult<super::types::DeliveryVerification> {
    mcp_media_delivery_verification(state, account_id, contact_wxid, asset_id, entry_created_at)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ContentAsset;
    use mongodb::bson::DateTime;

    #[test]
    fn media_id_cache_respects_ttl() {
        let now_ms = 1_000_000_000_000i64;
        let now = DateTime::from_millis(now_ms);
        let fresh = DateTime::from_millis(now_ms - 1000 * 60 * 60); // 1h 前
        let stale = DateTime::from_millis(now_ms - 1000 * 60 * 60 * 48); // 48h 前
        assert!(media_id_cache_valid(fresh, 24, now)); // 24h TTL 内
        assert!(!media_id_cache_valid(stale, 24, now)); // 超 TTL
    }

    #[test]
    fn media_id_cache_future_timestamp_is_invalid() {
        // 时钟回拨 / 脏数据：updated_at 在 now 之后 → 视为无效，强制重传。
        let now = DateTime::from_millis(1_000_000_000_000);
        let future = DateTime::from_millis(1_000_000_000_000 + 5000);
        assert!(!media_id_cache_valid(future, 24, now));
    }

    fn asset(
        sendable: Option<bool>,
        review: Option<&str>,
        mt: Option<&str>,
        stages: Option<Vec<&str>>,
    ) -> ContentAsset {
        ContentAsset {
            id: None,
            workspace_id: "ws".into(),
            account_id: None,
            kind: "media".into(),
            title: "报价单".into(),
            body: None,
            tags: vec![],
            url: None,
            media_id: None,
            usage_scene: None,
            media_type: mt.map(|s| s.to_string()),
            file_path: Some("ws/ab/x.pdf".into()),
            file_name: Some("报价单.xlsx".into()),
            file_size: Some(1),
            mime_type: Some("application/pdf".into()),
            file_sha256: Some("ab".into()),
            sendable,
            send_trigger_hint: Some("问价时发".into()),
            target_stages: stages.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            expression_pref: Some("file_primary".into()),
            requires_principal_approval: Some(false),
            review_status: review.map(|s| s.to_string()),
            review_note: None,
            min_inject_tier: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn tool_mapping_covers_three_types_and_rejects_unknown() {
        assert_eq!(mcp_tool_for_media_type("image"), Some("message_send_image"));
        assert_eq!(mcp_tool_for_media_type("file"), Some("message_send_file"));
        assert_eq!(mcp_tool_for_media_type("video"), Some("message_send_video"));
        assert_eq!(mcp_tool_for_media_type("link_card"), None);
        assert_eq!(mcp_tool_for_media_type(""), None);
    }

    #[test]
    fn filter_excludes_draft_and_non_sendable_and_no_media_type() {
        let all = vec![
            asset(Some(true), Some("approved"), Some("file"), None), // 保留
            asset(Some(true), Some("draft"), Some("file"), None),    // 排除：未审
            asset(Some(false), Some("approved"), Some("file"), None), // 排除：不可发
            asset(None, None, None, None),                           // 排除：老朋友圈行
            asset(Some(true), Some("approved"), None, None),         // 排除：无 media_type
        ];
        let kept = filter_sendable_candidates(&all, None);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn filter_matches_stage_or_empty_stages() {
        let all = vec![
            asset(
                Some(true),
                Some("approved"),
                Some("file"),
                Some(vec!["意向"]),
            ), // 命中
            asset(
                Some(true),
                Some("approved"),
                Some("file"),
                Some(vec!["已成交"]),
            ), // 不命中
            asset(Some(true), Some("approved"), Some("file"), None), // 空 stages 总命中
        ];
        let kept = filter_sendable_candidates(&all, Some("意向"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_lines_includes_id_stage_pref_hint() {
        let a = asset(
            Some(true),
            Some("approved"),
            Some("file"),
            Some(vec!["意向"]),
        );
        let line = render_candidate_lines(&[&a]);
        assert!(line.contains("file_primary") || line.contains("文件为主"));
        assert!(line.contains("问价时发"));
    }

    #[test]
    fn render_candidate_includes_tags() {
        let mut a = asset(Some(true), Some("approved"), Some("file"), None);
        a.title = "报价单".to_string();
        a.tags = vec!["报价类".to_string(), "价格".to_string()];
        let out = render_candidate_lines(&[&a]);
        assert!(out.contains("报价类"), "候选清单应渲染 tags");
    }

    #[test]
    fn render_candidate_skips_empty_tags() {
        let mut a = asset(Some(true), Some("approved"), Some("file"), None);
        a.title = "无标签素材".to_string();
        // tags 留空
        a.tags = vec![];
        let out = render_candidate_lines(&[&a]);
        assert!(!out.contains("标签:"), "空 tags 不渲染标签段");
    }

    #[test]
    fn overview_lists_title_and_hint_but_not_id_or_metadata() {
        // 轻量概览只露标题 + 发送时机，绝不露 assetId / 阶段 / 表达偏好（那是 Full 档完整清单的事）。
        let mut a = asset(
            Some(true),
            Some("approved"),
            Some("file"),
            Some(vec!["意向"]),
        );
        a.title = "报价单".to_string();
        let out = render_candidate_overview(&[&a]);
        assert!(out.contains("报价单"), "概览应含标题");
        assert!(out.contains("问价时发"), "概览应含 send_trigger_hint");
        assert!(!out.contains("id:"), "概览绝不露 assetId");
        assert!(!out.contains("阶段:"), "概览不露阶段");
        assert!(!out.contains("表达:"), "概览不露表达偏好");
    }

    #[test]
    fn overview_handles_missing_hint() {
        let mut a = asset(Some(true), Some("approved"), Some("file"), None);
        a.title = "无提示素材".to_string();
        a.send_trigger_hint = None;
        let out = render_candidate_overview(&[&a]);
        assert!(out.contains("无提示素材"), "无 hint 也应列出标题");
        assert!(!out.contains("发送时机："), "无 hint 不渲染发送时机段");
    }

    #[test]
    fn overview_empty_candidates_is_empty() {
        assert_eq!(render_candidate_overview(&[]), "");
    }

    #[test]
    fn overview_appends_escalation_guidance_when_nonempty() {
        // 非空候选：概览末尾必须带显式升档引导（A/B 已证 passive 列举不足以升档）。
        let mut a = asset(Some(true), Some("approved"), Some("file"), None);
        a.title = "报价单".to_string();
        let out = render_candidate_overview(&[&a]);
        // 升档引导：语义描述"契合就判 need_more_context + missingTier=full"，不列触发词。
        assert!(
            out.contains("need_more_context"),
            "概览应含升档引导关键判定"
        );
        assert!(
            out.contains("missingTier") || out.contains("full"),
            "升档引导应指明升 full 档"
        );
    }
}
