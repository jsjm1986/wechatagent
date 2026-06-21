//! 多模态入站地基（⑤完整 feature 的第 2 步）。
//!
//! Phase 1 范围只做三件**地基**，真实接通与语音 ASR 留待 ⑤ 独立立项：
//! 1. [`fetch_inbound_media`]：拉取入站媒体二进制内容的接口——当前 MCP server
//!    尚无"下载入站媒体"tool（仓内零调用、零书面依据），按 referral-card 纪律
//!    "仓内零书面依据不能凭空实现"**打桩返回 `Ok(None)`（未接通）**，绝不 panic。
//! 2. [`describe_inbound_image`]：图片理解封装——**复用**知识库导入已存在的 vision
//!    能力（`select_vision_provider` + `vision_generate_json` →
//!    `LlmClient::generate_json_with_image`），不另写一套。拿到 base64 即可调；
//!    media 下载未接通时它暂不会被实际触发（依赖 [`fetch_inbound_media`]），是地基。
//! 3. [`non_text_transition_reply`]：非文本消息过渡话术——当消息为非文本且媒体
//!    理解链路未接通时，AI 用自治口吻请客户文字补充，**绝不硬答空串/原始 XML、
//!    绝不崩**。话术全程 AI 自治口吻（客户始终只跟 AI 对话）。

use crate::error::{AppError, AppResult};
use crate::routes::AppState;

/// 入站媒体的二进制内容（图片/语音/文件）。当前仅 [`describe_inbound_image`]
/// 的图片路径会消费 `base64` + `mime`；`bytes` 保留给后续文件/语音落盘路径。
///
/// `dead_code`：地基阶段 [`fetch_inbound_media`] 打桩恒 `None`，本类型字段/方法暂
/// 不会被实际读到——⑤完整 feature 接通下载后即消费。保留是有意的地基。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MediaContent {
    /// 原始二进制字节。
    pub bytes: Vec<u8>,
    /// MIME 类型（如 `image/jpeg`、`audio/amr`）。
    pub mime: String,
}

#[allow(dead_code)]
impl MediaContent {
    /// 把二进制内容编码成 vision 端点要求的 base64 字符串。
    pub fn to_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.bytes)
    }
}

/// 拉取入站媒体（图片/语音/文件）的二进制内容。
///
/// **打桩**：当前 MCP server 没有确认的"下载入站媒体"tool（仓内零调用、零书面
/// 依据），按 referral-card `message_send_namecard` 同款纪律——仓内零书面依据不
/// 能凭空实现。故本函数恒返回 `Ok(None)`（未接通），下游据此走过渡话术，绝不 panic。
///
/// TODO(⑤完整 feature)：打 `server tools/list` 确认媒体下载 tool 能力后再接通，
/// 届时返回 `Ok(Some(MediaContent { .. }))`。语音 ASR 同样待独立立项。
pub async fn fetch_inbound_media(
    _state: &AppState,
    _media_ref: &str,
) -> AppResult<Option<MediaContent>> {
    // 打桩：未接通。fail-soft —— 返回 None 而非 Err，下游走过渡话术兜底。
    Ok(None)
}

/// 理解客户发来的图片，返回一段文字描述供决策链路使用。
///
/// **真复用**知识库导入的 vision 能力：经 `select_vision_provider` 选 supports_vision
/// 的模型（active 文字主模型支持图片则直接复用运行时 provider，否则用 workspace
/// 指派的视觉副模型候选链），再经 `vision_generate_json` →
/// `LlmClient::generate_json_with_image` 发真正的多模态 image_url content block。
///
/// 约束 LLM 输出 `{"description": "..."}`，取出描述文本返回。地基阶段此封装暂不会
/// 被实际触发（依赖 [`fetch_inbound_media`] 接通），先把"拿到 base64 即可调"备好。
///
/// `dead_code`：同上——下载接通前不会被调用，保留是有意的地基。
#[allow(dead_code)]
pub async fn describe_inbound_image(
    state: &AppState,
    workspace_id: &str,
    image_base64: &str,
    mime: &str,
) -> AppResult<String> {
    let provider =
        crate::routes::knowledge::select_vision_provider(state, workspace_id).await?;
    let system_prompt = "你是客户消息的图片理解助手。任务：用中文如实描述这张图片里的可见内容，\
便于后续对话理解客户想表达什么。只描述真实可见的内容，绝不编造、补全或推断图中没有的东西；\
看不清就如实说看不清。返回严格 JSON：{\"description\": <一段自然语言描述字符串>}。";
    let user_prompt = "请描述这张客户发来的图片。";
    let value = crate::routes::knowledge::vision_generate_json(
        &provider,
        state,
        system_prompt,
        user_prompt,
        image_base64,
        mime,
    )
    .await?;
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if description.is_empty() {
        return Err(AppError::External(
            "vision 返回空描述：图片可能无可读内容".to_string(),
        ));
    }
    Ok(description)
}

/// 非文本消息且理解链路未接通时的**过渡话术**：AI 自治口吻请客户文字补充关键信息，
/// 既不硬答空串/原始 XML、也不崩。
///
/// 话术对所有已知非文本类型（image/voice/link/miniprogram/file 及 unknown 兜底）
/// 都返回非空、自然、AI 自治口吻的文案——客户始终只跟 AI 对话。**绝不**出现暗示
/// 人工接管的措辞（由 `check-no-human-takeover` lint 在 diff 层兜底）。
pub fn non_text_transition_reply(msg_type: &str) -> String {
    match msg_type {
        "image" => "我看到您发的图片啦～方便简单文字描述下您想了解什么吗？这样我能更准确帮您看～".to_string(),
        "voice" => "收到您的语音啦～方便文字打一下吗？我好第一时间帮您看～".to_string(),
        "video" => "收到您发的视频啦～方便文字简单说下重点吗？我好帮您处理～".to_string(),
        "link" | "appmsg" => "收到您分享的链接啦～方便文字说下您想了解哪方面吗？我帮您看～".to_string(),
        "miniprogram" => "收到您发的小程序啦～方便文字说下您的需求吗？我好帮您～".to_string(),
        "file" => "收到您发的文件啦～方便文字简单说下里面的关键信息吗？我好帮您看～".to_string(),
        // unknown 及其它一切非文本类型的兜底：保证永远有一条自然回复。
        _ => "收到～方便文字简单说下您的需求吗？我好帮您处理～".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 反过拟合：覆盖多类型（含 unknown 兜底），每类都返回非空话术。
    /// 禁词（人工/接管等）由 check-no-human-takeover lint 在 diff 层兜底。
    #[test]
    fn non_text_transition_reply_covers_types_non_empty() {
        for t in [
            "image",
            "voice",
            "video",
            "link",
            "appmsg",
            "miniprogram",
            "file",
            "unknown",
            "",
            "some_future_type",
        ] {
            let r = non_text_transition_reply(t);
            assert!(!r.trim().is_empty(), "msg_type={t:?} 过渡话术不能为空");
        }
    }

    /// unknown / 未知类型走兜底分支，与显式 image 分支不同（确认 match 兜底生效）。
    #[test]
    fn non_text_transition_reply_unknown_uses_fallback() {
        let fallback = non_text_transition_reply("unknown");
        assert_eq!(fallback, non_text_transition_reply("some_future_type"));
        assert_ne!(fallback, non_text_transition_reply("image"));
    }
}
