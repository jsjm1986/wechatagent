//! 决策请示通道集成测试。多数需 MongoDB（testcontainers），标 #[ignore]，CI 跑。
//! 纯函数测试（不标 ignore）随 `cargo test --test principal_decision_channel` 即跑。

// 注：assert_target_is_principal 是 pub(crate)，集成测试在 crate 外无法直接调；
// 其纯函数测试放在 src/agent/escalation.rs 的 #[cfg(test)] mod 内。
// 本文件聚焦需 DB 的端到端流程（Task 24 填充）。

#[test]
fn placeholder_compiles() {
    assert!(true);
}
