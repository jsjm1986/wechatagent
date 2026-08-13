//! 提示词自然语言编辑的三层分级 + 双闸校验（spec §4.4）。
//! 红线靠机制不靠 LLM 自觉：任何经自然语言写回 prompt_templates 的内容，
//! 落库前强制过两道闸（禁用词 + 锚完整性），命中即拒、fail-closed。
//!
//! 本模块从 routes/management_prompt_edit.rs 下沉到中立顶层模块，供人工编辑路径
//! 与 evolution release 路径共用（两条写 prompt 的路径同享三道红线闸）。
//!
//! 注意：本文件位于 src/ 顶层，CI lint 会扫新增行的禁用词字面量
//! （含测试 mod，因为只排除 */tests/* 路径而非同文件 #[cfg(test)]）。所以
//! 非测试代码绝不内联任何禁用词 / 红线正文（只 import 锚常量，锚常量定义在
//! prompts.rs 不在扫描区）；测试构造禁用词时用字符拼接绕过字面量。

use crate::evolution::lint::passes_forbidden_words;
use crate::prompts::{
    normalize_prompt_content, DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_FAST_TASK_REDLINE_ANCHORS,
    DEFAULT_REPLY_REDLINE_ANCHORS, DEFAULT_REPLY_SYSTEM_REDLINE_ANCHORS,
    DEFAULT_REVIEWER_FEWSHOT, PROMPT_EVOLUTION_FORBIDDEN_KEYS,
};
use crate::routes::AppState;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEditTier {
    FreelyEditable,
    ConstrainedEditable,
    Forbidden,
}

/// 强约束层 key → 写回后必须逐字保留的全部锚段（业务锚 + 红线锚）。
/// user.reply.policy 既要保留业务锚 DEFAULT_MODE_GATE_POLICY，**也要保留
/// 反真人转介红线锚 DEFAULT_REPLY_REDLINE_ANCHORS**（核心修正——旧设计只查
/// 业务锚，红线被删却能放行）。
pub fn required_anchors(template_key: &str) -> Vec<&'static str> {
    match template_key {
        "user.reply.policy" => {
            // 业务锚 + 红线锚（红线在正文 :1123/:1146，旧锚闸漏查）
            let mut v = vec![DEFAULT_MODE_GATE_POLICY];
            v.extend_from_slice(DEFAULT_REPLY_REDLINE_ANCHORS);
            v
        }
        "user.reply.system" => DEFAULT_REPLY_SYSTEM_REDLINE_ANCHORS.to_vec(),
        // 注：退役的完整版 `user.reply.task` 已随种子包移除退出治理面（生产零消费；
        // 遗留 DB 行按普通话术 key 走禁用词闸）。
        "user.reply.fast.task" => DEFAULT_REPLY_FAST_TASK_REDLINE_ANCHORS.to_vec(),
        "user.review.system" => vec![DEFAULT_REVIEWER_FEWSHOT],
        _ => Vec::new(),
    }
}

pub fn prompt_edit_tier(template_key: &str) -> PromptEditTier {
    // 禁止改：evolution critic（与 PROMPT_EVOLUTION_FORBIDDEN_KEYS 同源）。
    // 注：reset-system-pack 是 route handler 不是 template_key，靠不接入工具来禁。
    if PROMPT_EVOLUTION_FORBIDDEN_KEYS.contains(&template_key)
        || template_key == "management.prompt_redline_review.system"
    {
        return PromptEditTier::Forbidden;
    }
    // 可改但需强约束：含红线 / 锚的业务模板（真实 key，已核实存在）
    if !required_anchors(template_key).is_empty()
        || matches!(
            template_key,
            "user.reply.policy"
                | "user.reply.system"
                | "user.review.system"
                | "user.reply.fast.task"
        )
    {
        return PromptEditTier::ConstrainedEditable;
    }
    // 其余业务话术 key，可自由改（仍过禁用词闸）
    PromptEditTier::FreelyEditable
}

/// 双闸校验（fail-closed）：命中任一闸即 Err，不写入。
pub fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String> {
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

// ── Task 6.6 第三闸：LLM 红线语义审查（三态降级人确认）──
// 字面双闸（validate_prompt_edit）只挡禁词字面量与锚段被删；挡不住「保留锚段、
// 无字面禁词、却插入变相真人转介/承诺转交/削弱 grounding」的语义绕过。第三闸用
// LLM 对 diff 增量做语义判定，靠语义不靠词表（守 agent-first）。
// 三态：Pass 放行 / Reject(理由) 拒绝 / NeedsHumanConfirm 降级人确认
// （LLM 重试退避后仍不可用——不 fail-closed 死路、不 fail-open 放水）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEditVerdict {
    Pass,
    Reject(String),
    NeedsHumanConfirm { diff: String, reason: String },
}

/// Return complete normalized before/after snapshots for every semantic mutation.
/// A set-based line diff can miss reorder-only edits and deletion of one duplicate
/// line, so only CRLF-equivalent content is allowed to bypass semantic review.
pub fn extract_diff(old: &str, new: &str) -> String {
    let old = normalize_prompt_content(old);
    let new = normalize_prompt_content(new);
    if old == new {
        return String::new();
    }
    format!("=== BEFORE ===\n{old}\n=== AFTER ===\n{new}")
}

/// 末尾追加合成：原 prompt 正文逐字保留在开头（红线锚点据此天然通过锚闸），
/// critic 片段追加到末尾,空行分隔。critic 只能「加约束」不能改写原红线段。
pub fn compose_appended_content(current: &str, snippet: &str) -> String {
    let snippet = snippet.trim();
    if snippet.is_empty() {
        return current.to_string();
    }
    format!("{}\n\n{}", current.trim_end(), snippet)
}

/// 第三闸：LLM 语义审查 diff 增量。先过字面双闸（调用方保证），本函数只做语义层。
/// 复用 generate_agent_json（项目唯一 LLM JSON 入口，自带重试/退避/RunBudget）。
pub async fn review_prompt_edit(
    state: &AppState,
    workspace_id: &str,
    template_key: &str,
    old: &str,
    new: &str,
) -> PromptEditVerdict {
    let diff = extract_diff(old, new);
    if diff.trim().is_empty() {
        // CRLF-equivalent content is the only mutation-free case.
        return PromptEditVerdict::Pass;
    }
    // judge 的 system 指令从 prompt pack 加载（key=management.prompt_redline_review.system）；
    // 加载失败也降级人确认（不静默放行）。
    let system_prompt = match crate::prompts::load_prompt(
        &state.db,
        workspace_id,
        "management.prompt_redline_review.system",
    )
    .await
    {
        Ok(s) => s,
        Err(_) => {
            return PromptEditVerdict::NeedsHumanConfirm {
                diff,
                reason: "红线语义审查指令加载失败，请逐字核对本次改动有无变相引入真人转介再确认"
                    .to_string(),
            };
        }
    };
    let user = format!(
        "待审提示词 key：{template_key}\n\n本次变更的完整前后快照（必须同时审查删除与新增）：\n{diff}"
    );
    let judge = crate::agent::generate_agent_json(
        state,
        workspace_id,
        None,
        None,
        None,
        "management.prompt_redline_review.system",
        &system_prompt,
        &user,
    )
    .await;
    match judge {
        Ok(v) => classify_review_verdict(&v, &diff),
        // 重试退避后仍失败（503/空/不可解析）→ 降级人确认
        Err(_) => PromptEditVerdict::NeedsHumanConfirm {
            diff,
            reason: "红线语义审查服务暂不可用，请逐字核对本次改动有无变相引入真人转介再确认"
                .to_string(),
        },
    }
}

/// 把 judge 返回的 JSON 解析为三态。纯函数,便于单测。
/// judge 契约（prompts.rs management.prompt_redline_review.system）：{"violation": bool, "reason": str}。
/// - violation==true → Reject(reason)
/// - violation==false → Pass（LLM 明确判合规）
/// - 其余（字段缺失/非布尔/空对象/拼写异常等模糊响应）→ NeedsHumanConfirm（不 fail-open 放行）
///   收紧自原 `Ok(_) => Pass`：模糊响应里无法确认 LLM 到底判没判违规,不能当审查通过。
fn classify_review_verdict(v: &Value, diff: &str) -> PromptEditVerdict {
    match v.get("violation").and_then(Value::as_bool) {
        Some(true) => {
            let reason = v
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("LLM 判定本次改动变相引入真人转介 / 削弱红线")
                .to_string();
            PromptEditVerdict::Reject(reason)
        }
        Some(false) => PromptEditVerdict::Pass,
        None => PromptEditVerdict::NeedsHumanConfirm {
            diff: diff.to_string(),
            reason: "红线语义审查返回结果无法解析（缺 violation 字段或格式异常），请逐字核对本次改动有无变相引入真人转介再确认"
                .to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::{
        DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_REDLINE_ANCHORS, DEFAULT_REVIEWER_FEWSHOT,
    };

    /// 用字符拼接构造禁用词，绕过源码字面量（本文件在 lint 扫描区，
    /// 连续的禁用词字面量会被禁词 CI lint 扫到导致误判）。
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
            prompt_edit_tier("user.reply.fast.task"),
            PromptEditTier::ConstrainedEditable
        );
        // 退役收缩：完整版 user.reply.task 已移出种子包与治理面，遗留 DB 行按
        // 普通话术 key 处理（仍过禁用词闸，不再要求红线锚）。
        assert_eq!(
            prompt_edit_tier("user.reply.task"),
            PromptEditTier::FreelyEditable
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
        let bad = format!(
            "{DEFAULT_MODE_GATE_POLICY}\n遇到难题就{}",
            forbidden_phrase()
        );
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

    // ── Task 6.6 第三闸（LLM 红线语义审查）纯函数部分 ──
    // LLM 真实判定行为留真模型 nightly 套件；此处只锁 diff 提取 + 三态形状。

    #[test]
    fn extract_diff_includes_complete_snapshots_for_additions() {
        let old = "第一行\n第二行";
        let new = "第一行\n第二行\n遇到难题转给后台老师跟进";
        let d = extract_diff(old, new);
        assert!(d.contains("=== BEFORE ==="));
        assert!(d.contains("=== AFTER ==="));
        assert!(d.contains("转给后台老师跟进"));
        assert!(d.contains("第一行"));
    }

    #[test]
    fn extract_diff_exposes_pure_deletions() {
        let diff = extract_diff("保留行\n关键安全约束", "保留行");
        assert!(diff.contains("=== BEFORE ===\n保留行\n关键安全约束"));
        assert!(diff.contains("=== AFTER ===\n保留行"));
    }

    #[test]
    fn extract_diff_exposes_reorder_and_duplicate_deletion() {
        assert!(!extract_diff("alpha\nbeta", "beta\nalpha").is_empty());
        assert!(!extract_diff("guard\nguard", "guard").is_empty());
    }

    #[test]
    fn extract_diff_ignores_only_crlf_equivalence() {
        assert!(extract_diff("alpha\r\nbeta", "alpha\nbeta").is_empty());
    }

    #[test]
    fn semantic_reviewer_prompt_is_forbidden_from_self_edit() {
        assert_eq!(
            prompt_edit_tier("management.prompt_redline_review.system"),
            PromptEditTier::Forbidden
        );
        assert!(
            validate_prompt_edit("management.prompt_redline_review.system", "replacement").is_err()
        );
    }

    #[test]
    fn reply_system_and_task_require_runtime_contract_anchors() {
        assert!(validate_prompt_edit("user.reply.system", "普通内容").is_err());
        assert!(validate_prompt_edit("user.reply.fast.task", "普通内容").is_err());

        let fast = crate::prompts::prompt_specs_for_test()
            .into_iter()
            .find(|(key, _)| key == "user.reply.fast.task")
            .expect("fast reply prompt exists")
            .1;
        assert!(validate_prompt_edit("user.reply.fast.task", &fast).is_ok());
    }

    #[test]
    fn verdict_variants_shape() {
        // 三态可构造（编译期锁形状；LLM 行为留真模型 nightly 套件）
        let _p = PromptEditVerdict::Pass;
        let _r = PromptEditVerdict::Reject("命中变相真人转介".into());
        let _h = PromptEditVerdict::NeedsHumanConfirm {
            diff: "x".into(),
            reason: "LLM 审查不可用".into(),
        };
    }

    #[test]
    fn compose_appends_snippet_preserving_original() {
        let current = "原始 prompt 正文\n红线锚段";
        let snippet = "补充：本行业语气更稳重";
        let composed = compose_appended_content(current, snippet);
        // 原文逐字保留在开头（锚点闸据此天然通过）
        assert!(composed.starts_with(current));
        // 片段出现在末尾
        assert!(composed.ends_with(snippet));
        // 中间有空行分隔
        assert!(composed.contains("红线锚段\n\n补充"));
    }

    #[test]
    fn compose_empty_snippet_is_byte_for_byte_noop() {
        let current = "原始 prompt 正文  \n红线锚段\n";
        assert_eq!(compose_appended_content(current, ""), current);
        assert_eq!(compose_appended_content(current, " \n\t "), current);
    }

    #[test]
    fn compose_trims_snippet_edge_whitespace_but_keeps_body() {
        let composed = compose_appended_content("正文", "  \n追加片段\n  ");
        assert!(composed.starts_with("正文\n\n"));
        assert!(composed.contains("追加片段"));
        // 不产生多余尾部空白行
        assert_eq!(
            composed.trim_end(),
            composed.trim_end_matches('\n').trim_end()
        );
    }

    // ── classify_review_verdict：第三闸 JSON→三态判定（收紧 fail-open 回归门）──
    // 原 `Ok(_) => Pass` 把所有非 violation==true 的合法 JSON 都放行；收紧后仅
    // violation==false 才 Pass，模糊响应（字段缺失/非布尔/空对象）降级 NeedsHumanConfirm。

    #[test]
    fn classify_verdict_violation_true_rejects() {
        let v = serde_json::json!({"violation": true, "reason": "变相真人转介"});
        assert!(matches!(
            classify_review_verdict(&v, "diff"),
            PromptEditVerdict::Reject(_)
        ));
    }

    #[test]
    fn classify_verdict_violation_false_passes() {
        let v = serde_json::json!({"violation": false});
        assert!(matches!(
            classify_review_verdict(&v, "diff"),
            PromptEditVerdict::Pass
        ));
    }

    #[test]
    fn classify_verdict_missing_field_needs_confirm() {
        // 字段缺失 → 不再 fail-open Pass，降级人确认
        let v = serde_json::json!({"reason": "忘了填 violation"});
        assert!(matches!(
            classify_review_verdict(&v, "diff"),
            PromptEditVerdict::NeedsHumanConfirm { .. }
        ));
    }

    #[test]
    fn classify_verdict_non_bool_violation_needs_confirm() {
        // violation 是字符串而非布尔 → 模糊响应，降级人确认
        let v = serde_json::json!({"violation": "true"});
        assert!(matches!(
            classify_review_verdict(&v, "diff"),
            PromptEditVerdict::NeedsHumanConfirm { .. }
        ));
    }

    #[test]
    fn classify_verdict_empty_object_needs_confirm() {
        let v = serde_json::json!({});
        assert!(matches!(
            classify_review_verdict(&v, "diff"),
            PromptEditVerdict::NeedsHumanConfirm { .. }
        ));
    }
}
