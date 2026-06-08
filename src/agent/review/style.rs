//! Phase D / D2：出站风格指纹与风格漂移判定。
//!
//! 纯字符串运算、确定性、不占 RunBudget；从 review 主流程拆出，便于独立演进
//! 与单测。被 gateway / reviewer 调用以决定是否触发 single-shot revision。

/// Phase D / D2：从一段出站文本提取风格指纹。
///
/// 设计取舍：选**结构特征**（长度桶 + 标点密度 + emoji 出现 + 句末符号），
/// 而非 LLM 嵌入向量。理由：
/// - 廉价、确定性、纯字符串运算，不占 RunBudget；
/// - 风格漂移最容易在结构上暴露（一会儿一句话、一会儿三段；一会儿带表情、
///   一会儿正经；一会儿陈述句、一会儿问句堆叠）；
/// - 语义级风格（如"专业 vs 亲切"）已经在 reviewer prompt 里通过 reply_style
///   playbook 字段控制，不重复造轮子。
///
/// 输出形如 `"len:s|emoji:0|qmark:1|excl:0|tail:.|nl:0"` 的紧凑串。
pub(crate) fn extract_outbound_style_fingerprint(content: &str) -> String {
    let trimmed = content.trim();
    let chars = trimmed.chars().count();
    let len_bucket = if chars <= 30 {
        "xs"
    } else if chars <= 80 {
        "s"
    } else if chars <= 200 {
        "m"
    } else {
        "l"
    };

    let has_emoji = trimmed
        .chars()
        .any(|c| matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF));
    let has_qmark = trimmed.contains('?') || trimmed.contains('？');
    let has_excl = trimmed.contains('!') || trimmed.contains('！');
    let nl_count = trimmed.matches('\n').count().min(9);

    // 句末符号：跳过尾部 emoji / 空白，归一化中英文标点。emoji 常作"装饰"挂在
    // 真句末符号之后（"方便聊一下吗？😊"），把它纳入 tail 会误把所有带 emoji 句
    // 都标成 tail:x，掩盖真实的问句 / 陈述句结构差异。
    let tail = trimmed
        .chars()
        .rev()
        .find(|c| {
            !c.is_whitespace()
                && !matches!(*c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF)
        })
        .unwrap_or('.');
    let tail_class = match tail {
        '?' | '？' => 'q',
        '!' | '！' => 'e',
        '。' | '.' => '.',
        '~' | '～' => '~',
        _ => 'x',
    };

    format!(
        "len:{}|emoji:{}|qmark:{}|excl:{}|tail:{}|nl:{}",
        len_bucket,
        if has_emoji { 1 } else { 0 },
        if has_qmark { 1 } else { 0 },
        if has_excl { 1 } else { 0 },
        tail_class,
        nl_count,
    )
}

/// Phase D / D2：判断两条风格指纹是否"分歧足够大"。
///
/// 风格指纹是 5 段 `key:value` 拼接的串。分歧度 = 不同 key 数量 ≥ 3 视为发散。
/// 本函数只做语义判定，不读 contact / state；上层 reviewer 拿 bool 决定要不要
/// 触发 single-shot revision。
pub(crate) fn style_diverged(prev: &str, current: &str) -> bool {
    if prev.is_empty() || current.is_empty() {
        return false;
    }
    let prev_parts: Vec<&str> = prev.split('|').collect();
    let cur_parts: Vec<&str> = current.split('|').collect();
    let len = prev_parts.len().min(cur_parts.len());
    let diff = (0..len)
        .filter(|i| prev_parts[*i] != cur_parts[*i])
        .count();
    diff >= 3
}

#[cfg(test)]
mod style_fingerprint_tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic() {
        let s = extract_outbound_style_fingerprint("您好，请问需要更多信息吗？");
        let s2 = extract_outbound_style_fingerprint("您好，请问需要更多信息吗？");
        assert_eq!(s, s2);
    }

    #[test]
    fn fingerprint_captures_length_bucket() {
        let xs = extract_outbound_style_fingerprint("好的");
        let m = extract_outbound_style_fingerprint(&"中".repeat(120));
        assert!(xs.contains("len:xs"));
        assert!(m.contains("len:m"));
    }

    #[test]
    fn fingerprint_captures_emoji_and_question() {
        let s = extract_outbound_style_fingerprint("方便聊一下吗？😊");
        assert!(s.contains("emoji:1"));
        assert!(s.contains("qmark:1"));
        assert!(s.contains("tail:q"), "trailing emoji 之前是问号: {}", s);
    }

    #[test]
    fn fingerprint_captures_newlines() {
        let s = extract_outbound_style_fingerprint("第一段\n\n第二段\n第三段");
        assert!(s.contains("nl:3"));
    }

    /// 完全相同的两条 → 不分歧。
    #[test]
    fn style_diverged_same_returns_false() {
        let a = extract_outbound_style_fingerprint("好的，请稍等。");
        assert!(!style_diverged(&a, &a));
    }

    /// 长度桶 + 句末符号 + 问号同时变 → 分歧 ≥ 3 → true。
    #[test]
    fn style_diverged_three_axes_changed() {
        let prev = extract_outbound_style_fingerprint("收到。");
        let cur = extract_outbound_style_fingerprint(&format!(
            "{}\n请问您还需要补充哪些信息呢？",
            "嗯".repeat(120)
        ));
        assert!(style_diverged(&prev, &cur), "prev={} cur={}", prev, cur);
    }

    /// 仅长度桶变（其它一致）→ 1 处不同 → false（容忍小幅波动）。
    #[test]
    fn style_diverged_minor_change_returns_false() {
        let prev = extract_outbound_style_fingerprint("好的。");
        let cur = extract_outbound_style_fingerprint("好的，已收到。");
        assert!(!style_diverged(&prev, &cur), "prev={} cur={}", prev, cur);
    }

    /// 空指纹（首轮回复）→ 永远不分歧，避免误触发首次 revision。
    #[test]
    fn style_diverged_empty_returns_false() {
        let cur = extract_outbound_style_fingerprint("好的。");
        assert!(!style_diverged("", &cur));
        assert!(!style_diverged(&cur, ""));
    }
}
