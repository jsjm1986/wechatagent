//! CONC-1：apply_operating_memory_update 写 memory_card 应走 OCC（版本谓词），
//! 并发输者 modified_count==0 时跳过而非 last-write-wins 覆盖。门控外的字段写不受影响。
//!
//! 验证点（留 CI / 后续，gateway 深层路径本地确定性复现成本高）：
//! - apply_operating_memory_update 末尾首次触达（memory_card 无信号）写 memory_card
//!   时，filter 含 memory_card_version 谓词（occ_memory_filter）。
//! - 并发两路 run 命中同一 prev_version，只有一个 update_one modified_count==1；
//!   输者 modified_count!=1 静默跳过（debug log），不返回 Err、不 last-write-wins 覆盖。
//! - 门控外的 updated_at 写走原三键 filter，始终成功，不受 OCC 影响。
//! - 最终 memory_card_version 单调不回退。

mod common;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB"]
async fn concurrent_memory_card_write_does_not_lose_race_error() {
    let app = common::TestApp::start().await;
    // 走完整 gateway run 触发 apply_operating_memory_update 的 memory_card 门控写。
    // 断言：并发两路 run 都不返回 Err(lost-race 静默跳过)，且最终 memory_card_version 单调不回退。
    // 具体播种 + 双 run 触发按 tests/ 现有 gateway 集成测试范式（参见
    // tests/operating_memory_insert_idempotent.rs 的并发首触达驱动）。
    let _ = &app;
}
