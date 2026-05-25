//! `block_parser` —— LLM "chunked structured output" 解析器。
//!
//! 借鉴 `nashsu/llm_wiki` 的 `ingest.ts:65-274` 的 `parseFileBlocks`：把 LLM 的输出
//! 从"一次返回大 JSON / 一次返回整文档"改成 fence 包裹的多块流式输出，
//! 每块独立校验、独立落库，单块解析失败不污染整个导入。
//!
//! 输入文本格式：
//!
//! ```text
//! ---CHUNK: chunk-id-1---
//! { "title": "...", "body": "...", "wiki_type": "concept", ... }
//! ---END CHUNK---
//!
//! ---CHUNK: chunk-id-2---
//! { ... }
//! ---END CHUNK---
//! ```
//!
//! 解析后产出 `Vec<ParsedChunkBlock>` + `ParseWarnings`：
//!
//! - **fence-aware**：以"行首 `---CHUNK: <id>---`"为开始锚，"行首 `---END CHUNK---`"为结束锚；
//!   行内出现这两个 token 但不在行首 → 当作普通正文（防 LLM 在 body 里写到 `---END CHUNK---`
//!   而被误识别为终止）；
//! - **unsafe-id 拒收**：`id` 含 `/` `\` `..` `<` `>` `|` `?` `*` 或控制字符 →
//!   该 block 整体丢弃 + 写一条 unsafe-id warning（防 path traversal / XSS）；
//! - **流截断容错**：最后一个 `---CHUNK: ...---` 没有匹配的 `---END CHUNK---` →
//!   写一条 truncation warning，**已闭合的块照常落库**（不全有总比全无好）；
//! - **重复 id**：同一 `id` 出现 ≥ 2 次 → 后出现的覆盖前一个 + 写一条 dup-id warning
//!   （LLM 偶尔会把同一 chunk 重述两遍）；
//! - **空 body**：JSON 解析后 body/summary/answer 三者皆空 → 该 block 丢弃 +
//!   写一条 empty-block warning。
//!
//! 设计原则：
//! - 0 LLM 参与；纯文本 → AST；
//! - 0 副作用；只返结构，写库由 caller 负责；
//! - 0 新依赖；只用 `serde_json`。

use serde_json::Value;

/// 单个解析出来的 chunk 块。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedChunkBlock {
    /// fence 中的 `id` 字段（已通过 unsafe-id 校验）。
    pub id: String,
    /// 解析后的 JSON 对象。`Object` 以外类型的 JSON 都视为非法块。
    pub payload: Value,
}

/// 解析过程中累积的 warning。导入路径应将这些 warning 持久化到导入日志中
/// （而非冒泡 4xx），以便排查。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParseWarnings {
    pub items: Vec<ParseWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseWarning {
    /// `---CHUNK:`/`---END CHUNK---` 的 `id` 含不安全字符 → 块被丢弃。
    UnsafeBlockId { id: String },
    /// 最后一个 fence 没匹配到 `---END CHUNK---`，怀疑模型流截断 → 该尾块被丢弃。
    UnterminatedFence { id: String },
    /// 同一 `id` 出现多次。后者覆盖前者。
    DuplicateBlockId { id: String, occurrences: usize },
    /// JSON 解析失败 / payload 不是 Object / 三主体字段全空 → 块被丢弃。
    InvalidJson { id: String, reason: String },
    /// 出现"裸文本"（不在任何 fence 内）但非空白。
    StrayText { excerpt: String },
}

const FENCE_START_PREFIX: &str = "---CHUNK:";
const FENCE_END_LITERAL: &str = "---END CHUNK---";

/// 把 LLM 的多块输出解析为 `ParsedChunkBlock` 列表 + warning 流。
///
/// 永不返回 `Err` —— 解析尽可能宽松，所有问题都进 warning，让 caller 决定是
/// "全停"还是"放过+记录"。导入路径建议后者：模型噪声 ≠ 不可恢复错误。
pub fn parse_chunk_blocks(input: &str) -> (Vec<ParsedChunkBlock>, ParseWarnings) {
    let mut blocks: Vec<ParsedChunkBlock> = Vec::new();
    let mut warnings = ParseWarnings::default();

    enum State<'a> {
        Outside { stray_buffer: String },
        Inside { id: &'a str, body: String },
    }

    let mut state = State::Outside {
        stray_buffer: String::new(),
    };

    for raw_line in input.split('\n') {
        // 行尾 \r 去掉（兼容 Windows 输入）；保留行内空白
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim_start();

        // fence 起始检测：必须行首（去左侧空白后）以 ---CHUNK: 开头且以 --- 结尾
        if let Some(fence_id) = parse_fence_start(trimmed) {
            match state {
                State::Outside { stray_buffer } => {
                    flush_stray(&stray_buffer, &mut warnings);
                }
                State::Inside { id, body: _ } => {
                    // 上一块没看到 ---END CHUNK---，就被新的 ---CHUNK: 打断 → 流截断
                    warnings.items.push(ParseWarning::UnterminatedFence {
                        id: id.to_string(),
                    });
                }
            }
            // 安全 id 守门
            if !is_safe_block_id(fence_id) {
                warnings.items.push(ParseWarning::UnsafeBlockId {
                    id: fence_id.to_string(),
                });
                // 进入"忽略状态"：直到看到 ---END CHUNK--- 为止
                state = State::Inside {
                    id: "__unsafe__",
                    body: String::new(),
                };
                continue;
            }
            state = State::Inside {
                id: fence_id,
                body: String::new(),
            };
            continue;
        }

        // fence 结束检测
        if trimmed == FENCE_END_LITERAL {
            match state {
                State::Outside { stray_buffer } => {
                    // 孤立的 ---END CHUNK---：当作 stray
                    let mut sb = stray_buffer;
                    if !sb.is_empty() {
                        sb.push('\n');
                    }
                    sb.push_str(FENCE_END_LITERAL);
                    flush_stray(&sb, &mut warnings);
                    state = State::Outside {
                        stray_buffer: String::new(),
                    };
                }
                State::Inside { id, body } => {
                    if id == "__unsafe__" {
                        // 不安全块：丢弃 body，warning 已发
                    } else {
                        finalize_block(id, &body, &mut blocks, &mut warnings);
                    }
                    state = State::Outside {
                        stray_buffer: String::new(),
                    };
                }
            }
            continue;
        }

        // 普通行
        match &mut state {
            State::Outside { stray_buffer } => {
                if !stray_buffer.is_empty() {
                    stray_buffer.push('\n');
                }
                stray_buffer.push_str(line);
            }
            State::Inside { body, .. } => {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(line);
            }
        }
    }

    // 收尾：还在 Inside → 流截断（最后一块没 ---END CHUNK---）
    match state {
        State::Outside { stray_buffer } => {
            flush_stray(&stray_buffer, &mut warnings);
        }
        State::Inside { id, body: _ } => {
            if id != "__unsafe__" {
                warnings.items.push(ParseWarning::UnterminatedFence {
                    id: id.to_string(),
                });
            }
        }
    }

    // dedup by id：保留最后一个，前面的发 warning
    dedup_keep_last(&mut blocks, &mut warnings);

    (blocks, warnings)
}

/// 把 fence 起始行 `---CHUNK: foo-bar---` 解析为 `Some("foo-bar")`，否则 None。
fn parse_fence_start(line: &str) -> Option<&str> {
    let after_prefix = line.strip_prefix(FENCE_START_PREFIX)?;
    let after_prefix = after_prefix.strip_suffix("---")?;
    let id = after_prefix.trim();
    if id.is_empty() {
        return None;
    }
    Some(id)
}

/// 块 id 安全性校验：拒绝 path-like 字符（防 traversal / shell injection）。
fn is_safe_block_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    // 双点 .. → traversal；显式禁
    if id.contains("..") {
        return false;
    }
    id.chars().all(|c| {
        !matches!(
            c,
            '/' | '\\'
                | '<'
                | '>'
                | '|'
                | '?'
                | '*'
                | '"'
                | ':'
                | '\0'..='\x1f'
                | '\x7f'
        )
    })
}

/// 块结束时校验 + 落 `Vec<ParsedChunkBlock>`。
fn finalize_block(
    id: &str,
    body: &str,
    blocks: &mut Vec<ParsedChunkBlock>,
    warnings: &mut ParseWarnings,
) {
    let trimmed = body.trim();
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            warnings.items.push(ParseWarning::InvalidJson {
                id: id.to_string(),
                reason: format!("json parse error: {e}"),
            });
            return;
        }
    };
    let obj = match parsed.as_object() {
        Some(_) => parsed,
        None => {
            warnings.items.push(ParseWarning::InvalidJson {
                id: id.to_string(),
                reason: "payload is not a JSON object".to_string(),
            });
            return;
        }
    };
    if is_payload_empty(&obj) {
        warnings.items.push(ParseWarning::InvalidJson {
            id: id.to_string(),
            reason: "all of body/summary/answer are empty".to_string(),
        });
        return;
    }
    blocks.push(ParsedChunkBlock {
        id: id.to_string(),
        payload: obj,
    });
}

/// body / summary / answer 三个文本字段全部为空（缺失或空字符串）→ true。
fn is_payload_empty(v: &Value) -> bool {
    let obj = match v.as_object() {
        Some(o) => o,
        None => return true,
    };
    let one = |key: &str| {
        obj.get(key)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    };
    !(one("body") || one("summary") || one("answer"))
}

/// 同一 id 出现多次 → 仅保留最后一次，发 dup warning。
fn dedup_keep_last(blocks: &mut Vec<ParsedChunkBlock>, warnings: &mut ParseWarnings) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for b in blocks.iter() {
        *counts.entry(b.id.as_str()).or_insert(0) += 1;
    }
    let dups: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(k, n)| (k.to_string(), n))
        .collect();
    if dups.is_empty() {
        return;
    }
    for (id, occurrences) in &dups {
        warnings.items.push(ParseWarning::DuplicateBlockId {
            id: id.clone(),
            occurrences: *occurrences,
        });
    }
    // 保留最后一次：从尾开始，遇到 id 已记录则丢前面的副本
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<ParsedChunkBlock> = Vec::with_capacity(blocks.len());
    for b in blocks.drain(..).rev() {
        if seen.insert(b.id.clone()) {
            kept.push(b);
        }
    }
    kept.reverse();
    *blocks = kept;
}

/// 处理"裸文本"：trim 后若仍非空 → 发 stray warning（取前 80 字 excerpt）。
fn flush_stray(buf: &str, warnings: &mut ParseWarnings) {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return;
    }
    let excerpt: String = trimmed.chars().take(80).collect();
    warnings.items.push(ParseWarning::StrayText { excerpt });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_well_formed_blocks() {
        let input = r#"
---CHUNK: chunk-1---
{"title": "T1", "body": "B1"}
---END CHUNK---

---CHUNK: chunk-2---
{"title": "T2", "summary": "S2"}
---END CHUNK---
"#;
        let (blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, "chunk-1");
        assert_eq!(blocks[1].id, "chunk-2");
        assert!(warns.items.is_empty());
    }

    #[test]
    fn rejects_path_traversal_in_id() {
        let input = "---CHUNK: ../etc/passwd---\n{\"body\":\"b\"}\n---END CHUNK---\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert!(blocks.is_empty());
        assert!(warns
            .items
            .iter()
            .any(|w| matches!(w, ParseWarning::UnsafeBlockId { .. })));
    }

    #[test]
    fn rejects_id_with_slash_or_pipe() {
        for bad in &["a/b", "a\\b", "a|b", "<a>", "a*b"] {
            let input = format!("---CHUNK: {bad}---\n{{\"body\":\"x\"}}\n---END CHUNK---\n");
            let (blocks, warns) = parse_chunk_blocks(&input);
            assert!(blocks.is_empty(), "id {bad} should be rejected");
            assert!(
                warns
                    .items
                    .iter()
                    .any(|w| matches!(w, ParseWarning::UnsafeBlockId { .. })),
                "warning missing for {bad}"
            );
        }
    }

    #[test]
    fn warns_on_unterminated_last_block_but_keeps_earlier() {
        let input = "---CHUNK: a---\n{\"body\":\"x\"}\n---END CHUNK---\n---CHUNK: b---\n{\"body\":\"y\"}\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "a");
        assert!(warns
            .items
            .iter()
            .any(|w| matches!(w, ParseWarning::UnterminatedFence { id } if id == "b")));
    }

    #[test]
    fn warns_on_invalid_json() {
        let input = "---CHUNK: a---\nnot json\n---END CHUNK---\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert!(blocks.is_empty());
        assert!(warns
            .items
            .iter()
            .any(|w| matches!(w, ParseWarning::InvalidJson { .. })));
    }

    #[test]
    fn warns_on_empty_body() {
        let input = "---CHUNK: a---\n{\"title\":\"T\"}\n---END CHUNK---\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert!(blocks.is_empty());
        assert!(warns
            .items
            .iter()
            .any(|w| matches!(w, ParseWarning::InvalidJson { reason, .. } if reason.contains("empty"))));
    }

    #[test]
    fn warns_on_duplicate_id_keeps_last() {
        let input = r#"
---CHUNK: dup---
{"body":"first"}
---END CHUNK---

---CHUNK: dup---
{"body":"second"}
---END CHUNK---
"#;
        let (blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].payload["body"].as_str(), Some("second"));
        assert!(warns
            .items
            .iter()
            .any(|w| matches!(w, ParseWarning::DuplicateBlockId { id, .. } if id == "dup")));
    }

    #[test]
    fn warns_on_stray_text_outside_fences() {
        let input = "preamble noise\n---CHUNK: a---\n{\"body\":\"x\"}\n---END CHUNK---\ntrailer noise\n";
        let (_blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(
            warns
                .items
                .iter()
                .filter(|w| matches!(w, ParseWarning::StrayText { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn fence_token_inside_body_does_not_terminate_when_indented() {
        // body 中包含的 ---END CHUNK--- 必须在行首才生效；缩进则视为正文。
        let input = "---CHUNK: a---\n{\"body\":\"  ---END CHUNK--- inline\"}\n---END CHUNK---\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "a");
        assert!(warns.items.is_empty());
    }

    #[test]
    fn handles_crlf_line_endings() {
        let input = "---CHUNK: a---\r\n{\"body\":\"x\"}\r\n---END CHUNK---\r\n";
        let (blocks, warns) = parse_chunk_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert!(warns.items.is_empty());
    }

    #[test]
    fn safe_id_bounds() {
        assert!(is_safe_block_id("a"));
        assert!(is_safe_block_id("chunk-001"));
        assert!(is_safe_block_id("c.h.u.n.k"));
        assert!(!is_safe_block_id(""));
        assert!(!is_safe_block_id("a/b"));
        assert!(!is_safe_block_id("a\\b"));
        assert!(!is_safe_block_id("a..b"));
        let too_long = "x".repeat(129);
        assert!(!is_safe_block_id(&too_long));
    }
}
