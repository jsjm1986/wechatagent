//! 销售素材选材：媒体类型→MCP 工具名映射、候选过滤（纯函数）。
//! 发送执行逻辑（ensure_media_uploaded / send_outbound_media）属于后续任务。
use crate::models::ContentAsset;

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
        let stages = a.target_stages.as_ref().map(|v| v.join(",")).unwrap_or_default();
        let pref = a.expression_pref.as_deref().unwrap_or("file_support");
        let hint = a.send_trigger_hint.as_deref().unwrap_or("");
        out.push_str(&format!(
            "- [id:{id}] {} | 阶段:{stages} | 表达:{pref}\n  触发提示:{hint}\n",
            a.title
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ContentAsset;
    use mongodb::bson::DateTime;

    fn asset(sendable: Option<bool>, review: Option<&str>, mt: Option<&str>, stages: Option<Vec<&str>>) -> ContentAsset {
        ContentAsset {
            id: None, workspace_id: "ws".into(), account_id: None,
            kind: "media".into(), title: "报价单".into(), body: None, tags: vec![],
            url: None, media_id: None, usage_scene: None,
            media_type: mt.map(|s| s.to_string()),
            file_path: Some("ws/ab/x.pdf".into()), file_name: Some("报价单.xlsx".into()),
            file_size: Some(1), mime_type: Some("application/pdf".into()),
            file_sha256: Some("ab".into()),
            sendable, send_trigger_hint: Some("问价时发".into()),
            target_stages: stages.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            expression_pref: Some("file_primary".into()),
            requires_principal_approval: Some(false),
            review_status: review.map(|s| s.to_string()), review_note: None,
            created_at: DateTime::now(), updated_at: DateTime::now(),
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
            asset(Some(true), Some("approved"), Some("file"), None),       // 保留
            asset(Some(true), Some("draft"), Some("file"), None),          // 排除：未审
            asset(Some(false), Some("approved"), Some("file"), None),      // 排除：不可发
            asset(None, None, None, None),                                 // 排除：老朋友圈行
            asset(Some(true), Some("approved"), None, None),               // 排除：无 media_type
        ];
        let kept = filter_sendable_candidates(&all, None);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn filter_matches_stage_or_empty_stages() {
        let all = vec![
            asset(Some(true), Some("approved"), Some("file"), Some(vec!["意向"])),    // 命中
            asset(Some(true), Some("approved"), Some("file"), Some(vec!["已成交"])),  // 不命中
            asset(Some(true), Some("approved"), Some("file"), None),                  // 空 stages 总命中
        ];
        let kept = filter_sendable_candidates(&all, Some("意向"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_lines_includes_id_stage_pref_hint() {
        let a = asset(Some(true), Some("approved"), Some("file"), Some(vec!["意向"]));
        let line = render_candidate_lines(&[&a]);
        assert!(line.contains("file_primary") || line.contains("文件为主"));
        assert!(line.contains("问价时发"));
    }
}
