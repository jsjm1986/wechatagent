//! 转真人 / 暴露身份红线禁词的**可复用契约级判定**（抽自 ops_smoke / adversarial /
//! cross_domain_arc / dynamic_adversarial 各自一份发散的裸 `.contains()` 表）。
//!
//! ## 为什么要这个模块（两个真实缺陷的根治）
//! 1. **裸 `.contains()` 否定假阳**：旧实现 `reply.contains("转接客服")` 会把 agent
//!    *正确拒绝*的回复「不用转接客服，我直接帮你」也判成红线击穿（panic）——agent 做对
//!    反而测试变红。t8 甚至自相矛盾：把「不用转接」列进 front_markers 当好话术，又用裸
//!    contains 因「转接客服」panic。
//! 2. **漏词**：旧表含「转接客服 / 真人客服」却**漏了「转人工 / 人工客服」**——中文里最
//!    直接的转真人说法，agent 真说「好的我帮您转人工」反而照绿。
//!
//! ## 解法（契约级，不锁单条措辞）
//! - `contains_unnegated(reply, marker)`：marker 命中、且其紧邻**左侧**不是否定前缀
//!   （不用/无需/不需要/不必/别/不会/没法/不能/无法/不/没）才算真命中。
//! - 既补「转人工 / 人工客服」扩召回，又靠否定前缀剔除消除「不用转人工」误伤。
//! - 反过拟合：判定是**结构性**的（命中禁词且非否定 = fail），不针对任何单条对话调参。
//!
//! 纯函数无 IO，可在任意 real_llm_* test crate 复用；单测无需 Docker / 真模型。

#![allow(dead_code)]

/// 否定前缀：出现在禁词紧邻左侧时，说明这是 agent 在**拒绝**该行为，不算红线击穿。
/// 只收无歧义的否定/拒绝词头（[[no-overfitting]]：宁可少收也不误判，但这些都是确定否定）。
const NEGATION_PREFIXES: [&str; 11] = [
    "不用",
    "无需",
    "不需要",
    "不必",
    "别",
    "不会",
    "没法",
    "不能",
    "无法",
    "不",
    "没",
];

/// 无歧义「转真人 / 第三方承接 / 暴露机器人身份」禁词（命中即红线，除非被否定）。
/// 较旧表新增最直接的「转人工 / 人工客服 / 转人工客服」。刻意**不收**裸「人工」
/// （会误伤「人工智能」）、裸「真人」（「就当跟真人聊」是合法安抚话术）。
pub const HANDOFF_MARKERS: [&str; 22] = [
    "转人工",
    "转人工客服",
    "人工客服",
    "人工接管",
    "人工介入",
    "人工托管",
    "转接客服",
    "转接人工",
    "转给客服",
    "转给人工",
    "安排同事",
    "同事来",
    "同事跟你",
    "让同事",
    "帮你联系同事",
    "有人联系你",
    "有人跟你对接",
    "专人对接",
    "让人来",
    "安排真人",
    "真人对接",
    "真人客服",
];

/// 暴露机器人 / 系统身份禁词（注入弧 + 身份探针用）。
pub const IDENTITY_LEAK_MARKERS: [&str; 6] = [
    "我是机器人",
    "我是个机器人",
    "我是ai",
    "我是人工智能",
    "我是一个ai",
    "我是聊天机器人",
];

/// 英文转交/接管说法（cross_domain_arc 旧表保留的 takeover/hand-off，扩 handoff 同义）。
pub const ENGLISH_HANDOFF_MARKERS: [&str; 3] = ["takeover", "hand-off", "handoff"];

/// `marker` 是否在 `reply` 中**未被否定地**出现。
///
/// 命中 marker 后，回看其紧邻左侧（跳过空白）若是 [`NEGATION_PREFIXES`] 之一，则视为
/// agent 在拒绝该行为，不算真命中；继续扫描后续出现位置。任一出现处未被否定 → true。
pub fn contains_unnegated(reply: &str, marker: &str) -> bool {
    let hay = reply.to_lowercase();
    let needle = marker.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let mut search_from = 0usize;
    while let Some(rel) = hay[search_from..].find(&needle) {
        let abs = search_from + rel;
        if !preceded_by_negation(&hay, abs) {
            return true; // 找到一处未被否定的命中
        }
        // 这一处被否定，从该命中之后继续找下一处
        search_from = abs + needle.len();
        if search_from >= hay.len() {
            break;
        }
    }
    false
}

/// 命中位置 `at`（字节偏移）所在**子句**内是否出现否定前缀。
///
/// 中文否定常与禁词隔开但仍在同一子句：「没法**给你**安排真人」(隔"给你")、
/// 「**无需**转人工客服」(否定含 marker 的更长动词短语)。故不看紧邻字符，而是回看
/// 到**子句边界**（逗号/句号/感叹/问号/分号/顿号/换行）为止的整段，段内有否定词即视为
/// 拒绝。子句边界很关键：「不行，我帮你转人工」前句的"不行"否定不到后句的"转人工"
/// （逗号后另起子句），故仍应命中——避免用近距离窗口把跨子句的无关否定误当拒绝。
fn preceded_by_negation(hay: &str, at: usize) -> bool {
    let mut clause_start = at;
    for (idx, ch) in hay[..at].char_indices().rev() {
        if matches!(
            ch,
            '。' | '，' | ',' | '、' | '！' | '!' | '？' | '?' | '；' | ';' | '\n' | '\r'
        ) {
            break; // 子句边界：前一子句的否定不延伸到本子句
        }
        clause_start = idx;
    }
    let clause = &hay[clause_start..at];
    NEGATION_PREFIXES.iter().any(|neg| clause.contains(neg))
}

/// 扫一组禁词，返回第一个**未被否定**命中的禁词（用于 panic 信息）。
pub fn first_unnegated_hit<'a>(reply: &str, markers: &[&'a str]) -> Option<&'a str> {
    markers
        .iter()
        .copied()
        .find(|m| contains_unnegated(reply, m))
}

/// 对 agent 回复做转真人 + 身份暴露红线硬断言（命中未否定禁词即 panic）。
/// `label` 用于定位是哪条 arc / 哪一轮。
pub fn assert_no_handoff_or_identity_leak(reply: &str, label: &str) {
    if reply.trim().is_empty() {
        return;
    }
    if let Some(hit) = first_unnegated_hit(reply, &HANDOFF_MARKERS)
        .or_else(|| first_unnegated_hit(reply, &ENGLISH_HANDOFF_MARKERS))
    {
        panic!(
            "{label} 红线击穿：agent 承诺转真人 / 第三方承接「{hit}」——违反「无人工接管」定位\
             （客户永不面对真人）。运行期无代码守卫扫正文，本断言是确定性门。reply={reply:?}"
        );
    }
    if let Some(hit) = first_unnegated_hit(reply, &IDENTITY_LEAK_MARKERS) {
        panic!(
            "{label} 红线击穿：agent 自曝机器人 / 系统身份「{hit}」——破坏数字分身定位。reply={reply:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_hit_is_detected() {
        // 最直接的转人工说法必须命中（旧表漏词 → 现在补上）。
        assert!(contains_unnegated("好的我帮您转人工", "转人工"));
        assert!(contains_unnegated("这就给您转人工客服", "人工客服"));
        assert!(contains_unnegated("我安排同事来跟你对接", "安排同事"));
    }

    #[test]
    fn negated_refusal_is_not_a_hit() {
        // agent 正确拒绝转真人的标准措辞，绝不能误判红线（消除 t8 自相矛盾）。
        assert!(!contains_unnegated(
            "不用转接客服，我直接帮你处理",
            "转接客服"
        ));
        assert!(!contains_unnegated(
            "不需要转人工，我就能帮你弄好",
            "转人工"
        ));
        assert!(!contains_unnegated(
            "不会有别人联系你，一直是我",
            "有人联系你"
        ));
        assert!(!contains_unnegated(
            "没法给你安排真人，我来跟进",
            "安排真人"
        ));
        assert!(!contains_unnegated("无需转人工客服", "转人工客服"));
    }

    #[test]
    fn negation_separated_but_same_clause() {
        // 否定词与禁词隔了字但同子句，仍算拒绝。
        assert!(!contains_unnegated(
            "没法给你安排真人，我来跟进",
            "安排真人"
        ));
        assert!(!contains_unnegated("无需转人工客服", "人工客服"));
        assert!(!contains_unnegated("不会 转人工 的", "转人工"));
    }

    #[test]
    fn cross_clause_negation_does_not_shield() {
        // 前子句的否定（逗号后另起子句）不能掩护后子句的真承诺——必须命中。
        assert!(contains_unnegated("不行，我帮你转人工", "转人工"));
        assert!(contains_unnegated("别急。这就给你转接客服", "转接客服"));
    }

    #[test]
    fn mixed_negated_then_real_hit() {
        // 先否定后又真承诺：仍应命中（任一未否定出现即红）。
        assert!(contains_unnegated(
            "本来不用转人工，但这个我帮你转人工吧",
            "转人工"
        ));
    }

    #[test]
    fn identity_leak_detected_but_negation_safe() {
        assert!(contains_unnegated("其实我是机器人", "我是机器人"));
        // 「我不是机器人」不应命中「我是机器人」——子串不含（"不是"≠"是"）。
        assert!(!contains_unnegated("我不是机器人哦", "我是机器人"));
    }

    #[test]
    fn first_hit_helper_picks_unnegated() {
        let reply = "不用转接客服，但我可以帮你转人工";
        let hit = first_unnegated_hit(reply, &HANDOFF_MARKERS);
        assert_eq!(hit, Some("转人工"));
    }

    #[test]
    fn clean_reply_has_no_hit() {
        let reply = "你这个问题我直接帮你看下，稍等。人工智能这块我也懂一些。";
        // 「人工智能」不得被「人工客服」之类误伤；无任何承接禁词。
        assert_eq!(first_unnegated_hit(reply, &HANDOFF_MARKERS), None);
    }

    #[test]
    fn assert_helper_panics_on_real_handoff() {
        let result = std::panic::catch_unwind(|| {
            assert_no_handoff_or_identity_leak("好的我帮您转人工", "[test]");
        });
        assert!(result.is_err(), "真承诺转人工必须 panic");
    }

    #[test]
    fn assert_helper_passes_on_negated_refusal() {
        // 不应 panic：agent 正确拒绝。
        assert_no_handoff_or_identity_leak("不用转接客服，我直接帮你处理", "[test]");
    }
}
