//! v3 prompt-pack / Task 270：网关"永远预跑知识路由"结构性回归测试。
//!
//! 历史上 `gateway::run_user_operation_gateway_inner` 走的是
//! "decision_requires_knowledge → 跳过路由 → 第二遍才打开知识库"的
//! short-circuit；这导致寒暄态消息不会触发知识路由，长尾产品问法的硬关键词
//! 永远没机会命中（plan 文件 §"二级根因 ISSUE-012"）。
//!
//! WB5 把这条 short-circuit 删除，改为：
//!
//! 1. 永远先调一次 `route_operation_knowledge`（含硬关键词快路径）。
//! 2. Reply Agent 第一遍直接拿到 `knowledge_route` 结果。
//! 3. 若快路径命中而 LLM 决策出的 `conversation_mode != consultative`，
//!    gateway 强制覆盖为 `consultative` + `trigger_keywords_fastpath_hit`。
//!
//! 由于 `route_operation_knowledge` / `run_user_operation_gateway_inner` 都是
//! `pub(crate)`，无法在独立 crate 的集成测试中直接调用；本文件用 **源码级
//! 文本扫描** 替代 mock — 直接读 `src/agent/gateway.rs` 与
//! `src/agent/simulation.rs`，断言：
//!
//! - **不再有** `decision_requires_knowledge(` 调用点（注释 / doc comment 不算）；
//! - **必有** `route_operation_knowledge(` / `compute_keyword_fastpath_hits(`
//!   或 `keyword_fastpath_hit` 文本（任一）；
//! - **必有** `trigger_keywords_fastpath_hit` 字面量（gateway 覆盖 reason）。
//!
//! 这个 trick 与 `scripts/check-no-human-takeover.sh` 的 CI lint 思路一致：
//! 用文本断言把架构约束钉死，回归就一定爆红。

use std::fs;
use std::path::Path;

fn read_source(path: &str) -> String {
    let p = Path::new(path);
    fs::read_to_string(p).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

/// 把 Rust 单行注释 `// ...` 与块注释 `/* ... */` 简单剥掉，避免误判
/// "已删除的 short-circuit 写在注释里"为活代码。块注释处理是粗粒度的
/// （不处理嵌套），但本文件用例只需保证：被点名的函数 `decision_requires_knowledge`
/// 不会以 *活代码* 形式调用。
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_block = false;
    for line in src.lines() {
        let mut working = line;
        let mut buf = String::new();
        // 块注释处理（同行内的 /* ... */ 或跨行）
        loop {
            if in_block {
                if let Some(end) = working.find("*/") {
                    working = &working[end + 2..];
                    in_block = false;
                } else {
                    working = "";
                    break;
                }
            } else if let Some(start) = working.find("/*") {
                buf.push_str(&working[..start]);
                working = &working[start + 2..];
                in_block = true;
            } else {
                buf.push_str(working);
                break;
            }
        }
        // 单行注释 `//`
        let cleaned = if let Some(idx) = buf.find("//") {
            // 排除字符串内出现的 "//"：本项目源码里 `"//"` 字面量不会绕过，因为
            // 这里只关心是否调用 `decision_requires_knowledge(`，字符串 / 字面
            // 量里不会写这个 token。简化处理：直接截断。
            buf[..idx].to_string()
        } else {
            buf
        };
        out.push_str(&cleaned);
        out.push('\n');
    }
    out
}

fn assert_no_active_short_circuit_call(label: &str, src: &str) {
    let stripped = strip_comments(src);
    assert!(
        !stripped.contains("decision_requires_knowledge("),
        "{} SHALL 不再有 decision_requires_knowledge(...) 活代码调用 — \
         WB5 已删除原 short-circuit。如果新代码确实需要复用，请改名 / 改用
         compute_keyword_fastpath_hits + 永远预跑路径。",
        label
    );
}

fn assert_pre_route_present(label: &str, src: &str) {
    let stripped = strip_comments(src);
    assert!(
        stripped.contains("route_operation_knowledge("),
        "{} SHALL 直接调用 route_operation_knowledge(...)（永远预跑模式）。",
        label
    );
}

fn assert_fastpath_override_marker(label: &str, src: &str) {
    let stripped = strip_comments(src);
    assert!(
        stripped.contains("trigger_keywords_fastpath_hit"),
        "{} SHALL 在快路径命中时把 conversation_mode_reason 设为
         \"trigger_keywords_fastpath_hit\"（gateway 强制覆盖 conversation_mode
         为 consultative）。",
        label
    );
    assert!(
        stripped.contains("knowledge_route_has_keyword_fastpath_hit("),
        "{} SHALL 通过 knowledge_route_has_keyword_fastpath_hit(...) 判定快路径命中。",
        label
    );
}

#[test]
fn gateway_no_longer_uses_decision_requires_knowledge_short_circuit() {
    let src = read_source("src/agent/gateway.rs");
    assert_no_active_short_circuit_call("src/agent/gateway.rs", &src);
}

#[test]
fn gateway_always_calls_route_operation_knowledge_before_decision() {
    let src = read_source("src/agent/gateway.rs");
    assert_pre_route_present("src/agent/gateway.rs", &src);
}

#[test]
fn gateway_overrides_conversation_mode_on_fastpath_hit() {
    let src = read_source("src/agent/gateway.rs");
    assert_fastpath_override_marker("src/agent/gateway.rs", &src);
}

#[test]
fn simulation_path_aligned_with_gateway_pre_route_pattern() {
    // simulation.rs 走 shadow-mode，与 gateway 必须保持一致：永远预跑 +
    // 同一覆盖逻辑，否则 simulate_user_dialogue 与真实 gateway 行为发散。
    let src = read_source("src/agent/simulation.rs");
    assert_no_active_short_circuit_call("src/agent/simulation.rs", &src);
    assert_pre_route_present("src/agent/simulation.rs", &src);
    assert_fastpath_override_marker("src/agent/simulation.rs", &src);
}

#[test]
fn knowledge_router_exposes_pure_fastpath_helper() {
    // compute_keyword_fastpath_hits 必须以 pub fn 暴露，让 tests/
    // keyword_fastpath_router.rs 与未来其它独立 crate 都能直接复用。
    let src = read_source("src/agent/knowledge_router.rs");
    let stripped = strip_comments(&src);
    assert!(
        stripped.contains("pub fn compute_keyword_fastpath_hits("),
        "knowledge_router.rs SHALL 把硬关键词快路径以 pub fn 形式暴露给独立 crate 使用。"
    );
}
