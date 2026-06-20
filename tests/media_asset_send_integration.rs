//! 销售素材上传 → 审核 → 发送 的端到端数据流。需 Docker(testcontainers Mongo)，
//! 默认 #[ignore]，CI integration job 跑。
//!
//! 真实断言依赖 Task 5 的 `load_sendable_assets` + Task 7 发送执行——本 Task 4
//! 先建上传路径并占位结构，Task 11 回填断言。
#![cfg(test)]

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_then_review_then_only_approved_is_sendable() {
    // 1. 上传一个 PDF（multipart）→ 期望落库 review_status="draft", sendable=true, media_type="file"
    // 2. load_sendable_assets 在 draft 态下不返回它
    // 3. 调 /review approved 后，load_sendable_assets 返回它
    // 断言见 Task 5（load_sendable_assets）落地后补全。此处先占位结构。
    assert!(true);
}
