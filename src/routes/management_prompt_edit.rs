//! 提示词自然语言编辑的三层分级 + 双闸校验（spec §4.4）。
//! 红线靠机制不靠 LLM 自觉：任何经自然语言写回 prompt_templates 的内容，
//! 落库前强制过两道闸（禁用词 + 锚完整性），命中即拒、fail-closed。
//!
//! 注意：本文件位于 src/routes/ 扫描区内，CI lint 会扫新增行的禁用词字面量
//! （含测试 mod，因为只排除 */tests/* 路径而非同文件 #[cfg(test)]）。所以
//! 非测试代码绝不内联任何禁用词 / 红线正文（只 import 锚常量，锚常量定义在
//! prompts.rs 不在扫描区）；测试构造禁用词时用字符拼接绕过字面量。

use crate::evolution::lint::passes_forbidden_words;
use crate::prompts::{
    normalize_prompt_content, DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_REDLINE_ANCHORS,
    DEFAULT_REVIEWER_FEWSHOT, PROMPT_EVOLUTION_FORBIDDEN_KEYS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptEditTier {
    FreelyEditable,
    ConstrainedEditable,
    Forbidden,
}

/// 强约束层 key → 写回后必须逐字保留的全部锚段（业务锚 + 红线锚）。
/// user.reply.policy 既要保留业务锚 DEFAULT_MODE_GATE_POLICY，**也要保留
/// 反真人转介红线锚 DEFAULT_REPLY_REDLINE_ANCHORS**（核心修正——旧设计只查
/// 业务锚，红线被删却能放行）。
fn required_anchors(template_key: &str) -> Vec<&'static str> {
    match template_key {
        "user.reply.policy" => {
            // 业务锚 + 红线锚（红线在正文 :1123/:1146，旧锚闸漏查）
            let mut v = vec![DEFAULT_MODE_GATE_POLICY];
            v.extend_from_slice(DEFAULT_REPLY_REDLINE_ANCHORS);
            v
        }
        "user.review.system" => vec![DEFAULT_REVIEWER_FEWSHOT],
        // user.reply.system / user.reply.task 含红线但暂无独立 DEFAULT_* 锚常量：
        // 仍归强约束层（tier 判定里列出），靠禁用词闸兜底；如需更硬可后续为其抽锚。
        _ => Vec::new(),
    }
}

pub(super) fn prompt_edit_tier(template_key: &str) -> PromptEditTier {
    // 禁止改：evolution critic（与 PROMPT_EVOLUTION_FORBIDDEN_KEYS 同源）。
    // 注：reset-system-pack 是 route handler 不是 template_key，靠不接入工具来禁。
    if PROMPT_EVOLUTION_FORBIDDEN_KEYS.contains(&template_key) {
        return PromptEditTier::Forbidden;
    }
    // 可改但需强约束：含红线 / 锚的业务模板（真实 key，已核实存在）
    if !required_anchors(template_key).is_empty()
        || matches!(
            template_key,
            "user.reply.policy" | "user.reply.system" | "user.review.system" | "user.reply.task"
        )
    {
        return PromptEditTier::ConstrainedEditable;
    }
    // 其余业务话术 key，可自由改（仍过禁用词闸）
    PromptEditTier::FreelyEditable
}

/// 双闸校验（fail-closed）：命中任一闸即 Err，不写入。
pub(super) fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String> {
    match prompt_edit_tier(template_key) {
        PromptEditTier::Forbidden => {
            return Err(format!(
                "提示词 '{template_key}' 属禁止改层，自然语言入口不可修改"
            ));
        }
        PromptEditTier::FreelyEditable | PromptEditTier::ConstrainedEditable => {}
    }
    // 闸 1：禁用词闸，自由改与强约束层都过
    if !passes_forbidden_words(new_content) {
        return Err("写回内容命中禁用词表，已拒绝".to_string());
    }
    // 闸 2：锚完整性闸——强约束层的全部锚段（业务锚 + 红线锚）必须逐字仍在。
    // CRLF 归一：锚常量是 Windows 工作树的 r#"..."# 多行串，git autocrlf 跨构建
    // LF↔CRLF 互转；管理者提交的 new_content 换行风格也不受控。裸 contains 会因
    // 换行字节不同失配、误拒合法编辑 → 两边都过 normalize_prompt_content 再比
    // （复用 prompts.rs:181 同一归一函数）。
    let normalized = normalize_prompt_content(new_content);
    for anchor in required_anchors(template_key) {
        if !normalized.contains(&normalize_prompt_content(anchor)) {
            return Err(format!(
                "提示词 '{template_key}' 的红线 / 业务锚段缺失或被改，已拒绝（防 replace 静默失配 + 防红线被删）"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::{
        DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_REDLINE_ANCHORS, DEFAULT_REVIEWER_FEWSHOT,
    };

    /// 用字符拼接构造禁用词，绕过源码字面量（本文件在 lint 扫描区，
    /// 连续的禁用词字面量会被 check-no-human-takeover 扫到导致误判）。
    fn forbidden_phrase() -> String {
        ["人", "工", "接", "管"].concat()
    }

    #[test]
    fn tier_classifies_three_layers() {
        // 强约束：含红线 / 锚的业务模板
        assert_eq!(
            prompt_edit_tier("user.reply.policy"),
            PromptEditTier::ConstrainedEditable
        );
        assert_eq!(
            prompt_edit_tier("user.reply.system"),
            PromptEditTier::ConstrainedEditable
        );
        assert_eq!(
            prompt_edit_tier("user.review.system"),
            PromptEditTier::ConstrainedEditable
        );
        assert_eq!(
            prompt_edit_tier("user.reply.task"),
            PromptEditTier::ConstrainedEditable
        );
        // 禁止改：evolution critic（PROMPT_EVOLUTION_FORBIDDEN_KEYS）
        assert_eq!(
            prompt_edit_tier("evolution_critic_v1"),
            PromptEditTier::Forbidden
        );
        // 可自由改：其余业务话术 key
        assert_eq!(
            prompt_edit_tier("knowledge.chat.draft_chunk"),
            PromptEditTier::FreelyEditable
        );
    }

    #[test]
    fn dual_gate_rejects_forbidden_words() {
        // 禁用词闸：写回含禁用词被拒（fail-closed）
        let bad = format!("{DEFAULT_MODE_GATE_POLICY}\n遇到难题就{}", forbidden_phrase());
        assert!(validate_prompt_edit("user.reply.policy", &bad).is_err());
    }

    #[test]
    fn dual_gate_rejects_business_anchor_drift() {
        // 锚完整性闸：写回丢了 DEFAULT_MODE_GATE_POLICY 业务锚被拒
        let drifted = "## 我自己重写的策略\n随便写点别的".to_string();
        assert!(validate_prompt_edit("user.reply.policy", &drifted).is_err());
        assert!(validate_prompt_edit("user.review.system", "乱改").is_err());
    }

    #[test]
    fn dual_gate_rejects_redline_anchor_drift() {
        // 核心修正：保留业务锚 DEFAULT_MODE_GATE_POLICY，但删掉红线段 → 仍须被拒
        // （旧设计这里会放行 = 红线漏洞）
        let keeps_business_drops_redline = format!("{DEFAULT_MODE_GATE_POLICY}\n业务措辞随便加");
        assert!(
            validate_prompt_edit("user.reply.policy", &keeps_business_drops_redline).is_err(),
            "保留业务锚但丢红线锚必须被拒"
        );
    }

    #[test]
    fn dual_gate_allows_valid_constrained_edit() {
        // 保留全部锚（业务锚 + 红线锚）+ 无禁用词 + 追加业务措辞 → 放行
        let redlines: String = DEFAULT_REPLY_REDLINE_ANCHORS.join("\n");
        let ok = format!("{DEFAULT_MODE_GATE_POLICY}\n{redlines}\n\n补充：本行业语气更稳重。");
        assert!(validate_prompt_edit("user.reply.policy", &ok).is_ok());
        let ok2 = format!("{DEFAULT_REVIEWER_FEWSHOT}\n\n补充标尺：本域不逼单。");
        assert!(validate_prompt_edit("user.review.system", &ok2).is_ok());
    }

    #[test]
    fn forbidden_tier_always_rejected() {
        assert!(validate_prompt_edit("evolution_critic_v1", "任何内容").is_err());
    }

    #[test]
    fn freely_editable_only_checks_forbidden_words() {
        // 可自由改层：仍过禁用词闸，但不要求锚段
        assert!(validate_prompt_edit("knowledge.chat.draft_chunk", "随便写业务话术").is_ok());
        let bad = format!("必要时{}", forbidden_phrase());
        assert!(validate_prompt_edit("knowledge.chat.draft_chunk", &bad).is_err());
    }

    #[test]
    fn anchor_gate_normalizes_crlf() {
        // CRLF 提交不应误拒：把全部锚段换行换成 CRLF，归一后仍能通过锚闸。
        let redlines: String = DEFAULT_REPLY_REDLINE_ANCHORS.join("\n");
        let lf = format!("{DEFAULT_MODE_GATE_POLICY}\n{redlines}\n\n补充业务措辞。");
        let crlf = lf.replace('\n', "\r\n");
        assert!(
            validate_prompt_edit("user.reply.policy", &crlf).is_ok(),
            "CRLF 换行的合法编辑不应被锚闸误拒"
        );
    }
}
