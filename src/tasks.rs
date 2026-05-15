use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::FindOptions,
};
use tokio::time::sleep;

use crate::{
    agent, mcp,
    models::{ConversationMessage, MessageDirection},
    routes::AppState,
};

pub async fn run_task_worker(state: AppState) {
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "task worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.task_worker_interval_seconds,
        ))
        .await;
    }
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "status": "pending",
                "run_at": { "$lte": DateTime::now() }
            },
            FindOptions::builder()
                .limit(20)
                .sort(doc! { "run_at": 1 })
                .build(),
        )
        .await?;

    while let Some(task) = cursor.try_next().await? {
        let Some(task_id) = task.id else {
            continue;
        };
        state
            .db
            .tasks()
            .update_one(
                doc! { "_id": task_id },
                doc! { "$set": { "status": "running", "updated_at": DateTime::now() } },
                None,
            )
            .await?;
        let result = mcp::logged_call(
            state,
            "message_send_text",
            serde_json::json!({
                "recipient": task.contact_wxid,
                "content": task.content
            }),
        )
        .await;

        match result {
            Ok(response) => {
                state
                    .db
                    .messages()
                    .insert_one(
                        ConversationMessage {
                            id: None,
                            workspace_id: task.workspace_id.clone(),
                            account_id: task.account_id.clone(),
                            contact_wxid: task.contact_wxid.clone(),
                            message_id: response
                                .get("newMsgId")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                            direction: MessageDirection::Outbound,
                            content: task.content.clone(),
                            raw: mongodb::bson::to_document(&response).ok(),
                            created_at: DateTime::now(),
                        },
                        None,
                    )
                    .await?;
                state
                    .db
                    .tasks()
                    .update_one(
                        doc! { "_id": task_id },
                        doc! { "$set": { "status": "sent", "updated_at": DateTime::now() } },
                        None,
                    )
                    .await?;
                agent::write_event(
                    state,
                    Some(&task.contact_wxid),
                    "follow_up_sent",
                    "success",
                    "跟进任务已发送",
                    None,
                )
                .await?;
            }
            Err(error) => {
                state
                    .db
                    .tasks()
                    .update_one(
                        doc! { "_id": task_id },
                        doc! {
                            "$set": {
                                "status": "failed",
                                "error": error.to_string(),
                                "updated_at": DateTime::now()
                            }
                        },
                        None,
                    )
                    .await?;
                agent::write_event(
                    state,
                    Some(&task.contact_wxid),
                    "follow_up_failed",
                    "failed",
                    &error.to_string(),
                    None,
                )
                .await?;
            }
        }
    }
    Ok(())
}
