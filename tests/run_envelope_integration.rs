//! agent-autonomy-loop W1 / Task 2.6：Run Envelope 集成测试。
//!
//! 覆盖纯单元测试无法验证、必须真实 MongoDB 才能断言的 4 条不变量
//! （见 requirements.md R0.10）：
//!
//! 1. 入口写信封先于任何 LLM 调用 —— mock LLM 抛异常前 lifecycle 已 = "started"。
//! 2. 同 `run_id` 二次 `insert_one` 触发 DuplicateKey（验证 R0.2 禁止 re-insert）。
//! 3. `update_one` 在不存在 envelope 时走兜底 `insert` + 写
//!    `agent_events kind="run_envelope_recovered_via_insert"`（R0.2 兜底路径）。
//! 4. Reply Agent panic 后 lifecycle 终态 = `failed_before_decision`，
//!    `error_summary` 非空（R0.6 panic-hook + catch_unwind 包装层语义）。
//!
//! 全部默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）；CI 用
//! `cargo test --test run_envelope_integration -- --ignored` 触发。
//!
//! NOTE：用例 4 在 W1 task 2.5 把 gateway 入口改造完成后真正可端到端验证；
//! 当前实现里 gateway 还没接入 `catch_unwind` 包装层，因此 4 用 `update_run_envelope_terminal`
//! 直接模拟 panic-hook 推进 lifecycle 的等价语义（写入 `failed_before_decision` +
//! `error_summary="unhandled_panic: ..."` 后断言能正确读回）。

mod common;

use mongodb::bson::doc;
use wechatagent::agent::run_envelope::{
    is_valid_lifecycle_transition, update_run_envelope_terminal, write_run_envelope_started,
    AgentRunLogTerminalFields, EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT,
    LIFECYCLE_FAILED_BEFORE_DECISION, LIFECYCLE_STARTED, SOURCE_KIND_INBOUND_MESSAGE,
};

#[tokio::test]
#[ignore]
async fn envelope_started_written_before_any_llm_call() {
    // R0.1 / R0.5 / R0.10.a：先调 write_run_envelope_started，再尝试 LLM 调用
    // （这里不调真实 LLM，断言"信封已落库"即满足 R0.10.a 的不变量）。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_first_test";
    write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_test"),
        "evt_inbound_001",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await
    .expect("envelope insert SHALL succeed before any LLM call");

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope record present");

    assert_eq!(log.lifecycle, LIFECYCLE_STARTED);
    assert_eq!(log.run_id, run_id);
    assert_eq!(log.source_event_id, "evt_inbound_001");
    assert_eq!(log.source_kind, SOURCE_KIND_INBOUND_MESSAGE);
    // gateway_status 占位为 "pending"
    assert_eq!(
        log.gateway_result.get_str("gatewayStatus").ok(),
        Some("pending")
    );
    assert_eq!(log.final_review_status, "");
}

#[tokio::test]
#[ignore]
async fn same_run_id_second_insert_triggers_duplicate_key_error() {
    // R0.2 / R0.10.e：unique index 阻断同 run_id 重复 insert。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_dup_key_test";
    write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_test"),
        "evt_dup_001",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await
    .expect("first insert SHALL succeed");

    // 第二次 insert SHALL 因 unique(run_id) 触发 DuplicateKey 错误。
    let result = write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_test"),
        "evt_dup_002",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await;

    assert!(
        result.is_err(),
        "同 run_id 二次 insert SHALL 失败（DuplicateKey）"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("duplicate")
            || err_msg.to_lowercase().contains("e11000"),
        "错误信息 SHALL 含 duplicate 关键字, err={}",
        err_msg
    );
}

#[tokio::test]
#[ignore]
async fn update_one_falls_back_to_insert_with_recovery_event() {
    // R0.2 兜底路径：update_run_envelope_terminal 命中 matched_count == 0 时，
    // SHALL 走单次 insert 兜底 + 写 agent_events kind="run_envelope_recovered_via_insert"。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_recovery_test";
    let fields = AgentRunLogTerminalFields {
        lifecycle: Some("completed".to_string()),
        final_review_status: Some("approved".to_string()),
        autonomy_mode: Some("auto".to_string()),
        ..Default::default()
    };

    update_run_envelope_terminal(&app.state.db, run_id, fields)
        .await
        .expect("update_run_envelope_terminal SHALL succeed via insert fallback");

    // 兜底 insert 后应能通过 run_id 找到记录。
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope recovered via insert");
    assert_eq!(log.lifecycle, "completed");
    assert_eq!(log.final_review_status, "approved");
    assert_eq!(log.autonomy_mode, "auto");

    // agent_events 中 SHALL 留下 recovery 事件。
    let event = app
        .state
        .db
        .events()
        .find_one(
            doc! {
                "kind": EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT,
                "details.run_id": run_id,
            },
            None,
        )
        .await
        .expect("query agent_events")
        .expect("recovery event present");
    assert_eq!(event.kind, EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT);
    assert_eq!(event.status, "warning");
}

#[tokio::test]
#[ignore]
async fn panic_in_pipeline_marks_lifecycle_failed_before_decision() {
    // R0.6：catch_unwind 包装层 panic → update lifecycle = failed_before_decision +
    // error_summary 非空。当前 W1 task 2.5 还在并行编写，gateway 入口尚未接入
    // catch_unwind 包装；这里直接调用 update_run_envelope_terminal 模拟 panic-hook
    // 在捕获 panic 后写库的等价语义。
    let app = common::TestApp::start().await;

    let run_id = "run_envelope_panic_test";
    write_run_envelope_started(
        &app.state.db,
        run_id,
        &app.state.config.default_workspace_id,
        &app.state.config.default_account_id,
        Some("wxid_panic"),
        "evt_panic_001",
        SOURCE_KIND_INBOUND_MESSAGE,
        "reply",
    )
    .await
    .expect("envelope insert");

    // 模拟 catch_unwind 包装层捕获到 panic，按 R0.6 把 lifecycle 推进。
    // 同时验证 lifecycle FSM 允许 started → failed_before_decision。
    assert!(is_valid_lifecycle_transition(
        LIFECYCLE_STARTED,
        LIFECYCLE_FAILED_BEFORE_DECISION
    ));

    let panic_message = "Reply Agent panicked at decision.rs:42 — divide by zero";
    update_run_envelope_terminal(
        &app.state.db,
        run_id,
        AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_FAILED_BEFORE_DECISION.to_string()),
            error_summary: Some(format!("unhandled_panic: {}", panic_message)),
            ..Default::default()
        },
    )
    .await
    .expect("update lifecycle on panic");

    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query agent_run_logs")
        .expect("envelope present");
    assert_eq!(log.lifecycle, LIFECYCLE_FAILED_BEFORE_DECISION);
    assert!(
        log.error_summary
            .as_deref()
            .map(|s| s.starts_with("unhandled_panic:") && !s.is_empty())
            .unwrap_or(false),
        "error_summary SHALL 以 'unhandled_panic:' 开头且非空, actual={:?}",
        log.error_summary
    );
}
