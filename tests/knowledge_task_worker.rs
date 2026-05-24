//! knowledge-digest-workstation Phase 4：worker / SSE 不变量测试（无 Docker 依赖）。
//!
//! 配合 `src/knowledge_task/mod.rs` 内的两个 tokio::test，以及 Phase 1 的
//! `tests/knowledge_digest_skeleton.rs`，把 Phase 4 的核心契约 fence 起来：
//!
//! 1. `KnowledgeChatTask.planned_steps` 6 个合法 action 闭集（fix_chunk / add_chunk
//!    / retag / review_evolution / analyze_logs / dismiss）+ status 闭集
//!    （pending / running / finished / failed / cancelled）。
//! 2. `KnowledgeChatTurn { kind = "task_progress" | "task_summary" }` 能完整
//!    BSON round-trip，attachments 里 phase / taskId / stepIndex / total 都保留。
//! 3. `ChatProgressBus` 在并发场景下：`subscribe` 后 `bump` 必然让订阅者观察到
//!    `changed()`；同 sessionId 锁始终是同一 Arc。
//! 4. summary turn details 中 needsReviewChunkIds / failedStepIds / completedSteps
//!    三个字段类型契约稳定（Vec<String> / Vec<String> / Vec<Document>）。

use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, to_bson, to_document, DateTime, Document};
use wechatagent::knowledge_task::ChatProgressBus;
use wechatagent::models::{KnowledgeChatTask, KnowledgeChatTurn};

#[test]
fn planned_step_action_enum_is_closed_set() {
    // 与 src/knowledge_task/mod.rs:execute_step 的 match arms 对齐；任何 worker 端
    // 新增 action 必须同步加入这里，否则 worker 会走 unsupported 分支 fail-soft。
    let allowed_actions = [
        "fix_chunk",
        "add_chunk",
        "retag",
        "review_evolution",
        "analyze_logs",
        "dismiss",
    ];
    let example_step = doc! {
        "stepId": "step_1",
        "cardId": "card_a",
        "action": "fix_chunk",
        "summary": "修复缺 sourceQuote 的切片",
        "estimatedLlmCalls": 2_i32,
    };
    let action = example_step.get_str("action").expect("action present");
    assert!(
        allowed_actions.contains(&action),
        "action {action} 必须在闭集 {allowed_actions:?} 中"
    );
}

#[test]
fn knowledge_chat_task_status_closed_set_round_trip() {
    // pending → running → (finished | failed | cancelled) 是 worker 唯一合法迁移。
    let allowed_statuses = ["pending", "running", "finished", "failed", "cancelled"];
    for status in allowed_statuses {
        let task = KnowledgeChatTask {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            session_id: "sess_x".to_string(),
            operator_id: Some("op_1".to_string()),
            cards: vec![],
            planned_steps: vec![doc! {
                "stepId": "step_1",
                "cardId": "card_a",
                "action": "fix_chunk",
            }],
            completed_steps: vec![],
            status: status.to_string(),
            error_kind: None,
            created_at: DateTime::now(),
            started_at: None,
            finished_at: None,
        };
        let bson = to_bson(&task).expect("serialize task");
        let doc: Document = bson.as_document().expect("doc").clone();
        let back: KnowledgeChatTask =
            mongodb::bson::from_document(doc).expect("round-trip task");
        assert_eq!(back.status, status);
        assert_eq!(back.planned_steps.len(), 1);
    }
}

#[test]
fn task_progress_turn_preserves_phase_payload() {
    // worker write_progress_turn 写出来的 attachments 形态：
    // { taskId: ObjectId, phase: "started"|"step", stepIndex?: i32, total: i32 }
    let task_id = ObjectId::new();
    let attachments = vec![
        doc! { "taskId": task_id, "phase": "started", "total": 3_i32 },
        doc! { "taskId": task_id, "phase": "step", "stepIndex": 1_i32, "total": 3_i32 },
    ];
    for att in &attachments {
        let turn = KnowledgeChatTurn {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            session_id: "sess_y".to_string(),
            turn_index: 7,
            role: "system".to_string(),
            intent: Some("digest_action".to_string()),
            content: "AI 已开始处理 3 条派工任务".to_string(),
            attachments: vec![att.clone()],
            patch: None,
            missing_fields: vec![],
            followup_questions: vec![],
            status: "pending".to_string(),
            tokens_used: 0,
            prompt_key: None,
            kind: Some("task_progress".to_string()),
            tool_calls: vec![],
            created_at: DateTime::now(),
        };
        let doc = to_document(&turn).expect("serialize progress turn");
        let back: KnowledgeChatTurn =
            mongodb::bson::from_document(doc).expect("round-trip progress turn");
        assert_eq!(back.kind.as_deref(), Some("task_progress"));
        assert_eq!(back.role, "system");
        assert_eq!(back.attachments.len(), 1);
        let phase = back
            .attachments
            .first()
            .and_then(|d| d.get_str("phase").ok())
            .unwrap_or("");
        assert!(["started", "step"].contains(&phase));
    }
}

#[test]
fn task_summary_turn_carries_review_and_failed_lists() {
    // summary turn 里这三个 vector 是前端 / 测试断言的关键载荷。
    let task_id = ObjectId::new();
    let needs_review = vec!["chunk_1".to_string(), "chunk_2".to_string()];
    let failed_steps: Vec<String> = vec!["step_3".to_string()];
    let completed_steps: Vec<Document> = vec![doc! {
        "stepId": "step_1",
        "cardId": "card_a",
        "action": "fix_chunk",
        "status": "ok",
        "chunkId": "chunk_1",
    }];
    let summary_attach = doc! {
        "taskId": task_id,
        "phase": "summary",
        "status": "finished",
        "needsReviewChunkIds": needs_review.clone(),
        "failedStepIds": failed_steps.clone(),
        "completedSteps": completed_steps.clone(),
    };
    let turn = KnowledgeChatTurn {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        session_id: "sess_z".to_string(),
        turn_index: 12,
        role: "system".to_string(),
        intent: Some("digest_action".to_string()),
        content: "AI 派工任务已完成 · 共 3 步 · 成功 2 · 失败 1 · 待运营审核 chunk 2"
            .to_string(),
        attachments: vec![summary_attach],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        tokens_used: 0,
        prompt_key: None,
        kind: Some("task_summary".to_string()),
        tool_calls: vec![],
        created_at: DateTime::now(),
    };
    let doc = to_document(&turn).expect("serialize summary turn");
    let back: KnowledgeChatTurn =
        mongodb::bson::from_document(doc).expect("round-trip summary turn");
    assert_eq!(back.kind.as_deref(), Some("task_summary"));
    let attach = back.attachments.first().expect("summary attachment");
    let status = attach.get_str("status").expect("status");
    assert!(["finished", "failed", "cancelled"].contains(&status));
    let review = attach
        .get_array("needsReviewChunkIds")
        .expect("needsReviewChunkIds is array");
    assert_eq!(review.len(), 2);
    let failed = attach
        .get_array("failedStepIds")
        .expect("failedStepIds is array");
    assert_eq!(failed.len(), 1);
    let completed = attach
        .get_array("completedSteps")
        .expect("completedSteps is array");
    assert_eq!(completed.len(), 1);
}

#[tokio::test]
async fn chat_progress_bus_bump_is_visible_to_subscriber() {
    // worker 每写一条 turn 后 bump；订阅端 changed().await 必须能立即回返。
    let bus = Arc::new(ChatProgressBus::new());
    let mut rx = bus.subscribe("sess_bus").await;

    let bus_clone = bus.clone();
    let bumper = tokio::spawn(async move {
        // 给订阅端起线程的时间，再 bump。
        tokio::time::sleep(Duration::from_millis(20)).await;
        bus_clone.bump("sess_bus").await;
    });

    tokio::time::timeout(Duration::from_millis(500), rx.changed())
        .await
        .expect("subscriber should observe bump within 500ms")
        .expect("watch channel still alive");
    assert!(*rx.borrow_and_update() >= 1);
    bumper.await.expect("bumper task joined");
}

#[tokio::test]
async fn chat_progress_bus_late_subscriber_still_sees_subsequent_bump() {
    // 订阅在 bump 之后才发生 → 后续 bump 仍要让 changed() 触发；
    // 守住 worker_loop 与前端 SSE 重连之间的最差时序窗口。
    let bus = Arc::new(ChatProgressBus::new());
    bus.bump("sess_late").await;
    let mut rx = bus.subscribe("sess_late").await;
    // 已存在的版本号通过 borrow 获取一次；下一次 bump 才视为 changed。
    let _ = *rx.borrow_and_update();

    let bus_clone = bus.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        bus_clone.bump("sess_late").await;
    });

    tokio::time::timeout(Duration::from_millis(500), rx.changed())
        .await
        .expect("late subscriber should observe next bump")
        .expect("watch channel still alive");
}
