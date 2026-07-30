mod common;

use common::TestApp;
use mongodb::bson::{doc, DateTime, Document};
use wechatagent::{
    behavior_signals,
    models::Contact,
    proactive_outreach::{
        commit_follow_up, commit_signal_with_daily_quota, CommitOutcome, DailyQuota, FollowUpIntent,
    },
};

fn contact(workspace_id: &str, account_id: &str, wxid: &str, now: DateTime) -> Contact {
    mongodb::bson::from_document(doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "wxid": wxid,
        "agent_status": "managed",
        "operation_policy": {},
        "profile_attributes": {},
        "created_at": now,
        "updated_at": now,
    })
    .expect("minimal contact")
}

fn quota(namespace: &'static str, cap: i64) -> DailyQuota {
    DailyQuota {
        namespace,
        account_scope: Some("account-a".to_string()),
        total_cap: cap,
        segment_cap: None,
        initial_total: 0,
        initial_segment: 0,
    }
}

fn follow_up(
    contact: &Contact,
    subject: impl Into<String>,
    now: DateTime,
    cap: i64,
) -> FollowUpIntent {
    FollowUpIntent {
        contact: contact.clone(),
        segment: "silent",
        subject: subject.into(),
        content: "Planner: sr135 deterministic candidate".to_string(),
        event_kind: "sr135_emit",
        event_summary: "sr135 emitted".to_string(),
        event_details: doc! { "source": "sr135_test" },
        now,
        quota: quota("sr135_follow_up", cap),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires replica-set MongoDB"]
async fn concurrent_same_intent_commits_one_task_event_and_reservation() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let contact = contact("workspace-a", "account-a", "wxid-same", now);
    // cap=1 is deliberate: after the winner fills the last slot, every
    // same-intent loser must still converge to Duplicate, never Capped.
    let intent = follow_up(&contact, "last-inbound:100", now, 1);

    let mut joins = Vec::new();
    for _ in 0..32 {
        let state = app.state.clone();
        let intent = intent.clone();
        joins.push(tokio::spawn(async move {
            commit_follow_up(&state, intent).await
        }));
    }

    let mut emitted = 0;
    let mut duplicate = 0;
    for join in joins {
        match join.await.expect("join").expect("commit") {
            CommitOutcome::Emitted => emitted += 1,
            CommitOutcome::Duplicate => duplicate += 1,
            CommitOutcome::Capped => panic!("same intent must not consume the quota repeatedly"),
        }
    }
    assert_eq!(emitted, 1);
    assert_eq!(duplicate, 31);
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": "workspace-a" }, None)
            .await
            .expect("count tasks"),
        1
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! { "workspace_id": "workspace-a", "kind": "sr135_emit" },
                None
            )
            .await
            .expect("count events"),
        1
    );
    let bucket = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find_one(doc! { "namespace": "sr135_follow_up" }, None)
        .await
        .expect("read quota")
        .expect("quota exists");
    assert_eq!(bucket.get_i64("total").expect("quota total"), 1);
    app.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires replica-set MongoDB"]
async fn concurrent_distinct_intents_never_exceed_daily_cap() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let contact = contact("workspace-cap", "account-a", "wxid-cap", now);

    let mut joins = Vec::new();
    for index in 0..24 {
        let state = app.state.clone();
        let intent = follow_up(&contact, format!("candidate:{index}"), now, 3);
        joins.push(tokio::spawn(async move {
            commit_follow_up(&state, intent).await
        }));
    }

    let mut emitted = 0;
    let mut capped = 0;
    for join in joins {
        match join.await.expect("join").expect("commit") {
            CommitOutcome::Emitted => emitted += 1,
            CommitOutcome::Capped => capped += 1,
            CommitOutcome::Duplicate => panic!("all business intents are distinct"),
        }
    }
    assert_eq!(emitted, 3);
    assert_eq!(capped, 21);
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": "workspace-cap" }, None)
            .await
            .expect("count tasks"),
        3
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! { "workspace_id": "workspace-cap", "kind": "sr135_emit" },
                None
            )
            .await
            .expect("count events"),
        3
    );
    let bucket = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find_one(doc! { "namespace": "sr135_follow_up" }, None)
        .await
        .expect("read quota")
        .expect("quota exists");
    assert_eq!(bucket.get_i64("total").expect("quota total"), 3);
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn full_utc_day_bucket_does_not_block_the_next_day() {
    let app = TestApp::start_repl_set().await;
    let day_ms = 86_400_000_i64;
    let current_day_start = DateTime::now().timestamp_millis().div_euclid(day_ms) * day_ms;
    let first_day = DateTime::from_millis(current_day_start - 1);
    let next_day = DateTime::from_millis(current_day_start);
    let contact = contact(
        "workspace-midnight",
        "account-a",
        "wxid-midnight",
        first_day,
    );

    assert_eq!(
        commit_follow_up(
            &app.state,
            follow_up(&contact, "candidate:first", first_day, 1),
        )
        .await
        .expect("fill first day"),
        CommitOutcome::Emitted
    );
    let second = follow_up(&contact, "candidate:second", first_day, 1);
    assert_eq!(
        commit_follow_up(&app.state, second.clone())
            .await
            .expect("first-day cap"),
        CommitOutcome::Capped
    );

    let mut next_day_second = second;
    next_day_second.now = next_day;
    assert_eq!(
        commit_follow_up(&app.state, next_day_second)
            .await
            .expect("next-day reservation"),
        CommitOutcome::Emitted
    );
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": "workspace-midnight" }, None)
            .await
            .expect("count midnight tasks"),
        2
    );
    let buckets = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find(doc! { "namespace": "sr135_follow_up" }, None)
        .await
        .expect("find day buckets");
    use futures::TryStreamExt;
    let rows: Vec<Document> = buckets.try_collect().await.expect("collect day buckets");
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|bucket| bucket.get_i64("total").ok() == Some(1)));
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn segment_cap_and_shared_total_cap_are_both_persistent() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let contact = contact("workspace-segment", "account-a", "wxid-segment", now);
    let quota = DailyQuota {
        namespace: "sr135_segment_caps",
        account_scope: Some("account-a".to_string()),
        total_cap: 3,
        segment_cap: Some(2),
        initial_total: 0,
        initial_segment: 0,
    };
    let make = |segment: &'static str, subject: &str| FollowUpIntent {
        contact: contact.clone(),
        segment,
        subject: subject.to_string(),
        content: format!("Planner: {segment}"),
        event_kind: "sr135_emit",
        event_summary: "sr135 emitted".to_string(),
        event_details: doc! { "source": "sr135_test" },
        now,
        quota: quota.clone(),
    };

    assert_eq!(
        commit_follow_up(&app.state, make("calendar", "calendar:1"))
            .await
            .expect("calendar 1"),
        CommitOutcome::Emitted
    );
    assert_eq!(
        commit_follow_up(&app.state, make("calendar", "calendar:2"))
            .await
            .expect("calendar 2"),
        CommitOutcome::Emitted
    );
    assert_eq!(
        commit_follow_up(&app.state, make("calendar", "calendar:3"))
            .await
            .expect("calendar capped"),
        CommitOutcome::Capped
    );
    assert_eq!(
        commit_follow_up(&app.state, make("renewal", "renewal:1"))
            .await
            .expect("renewal uses remaining total"),
        CommitOutcome::Emitted
    );
    assert_eq!(
        commit_follow_up(&app.state, make("renewal", "renewal:2"))
            .await
            .expect("shared total capped"),
        CommitOutcome::Capped
    );

    let bucket = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find_one(doc! { "namespace": "sr135_segment_caps" }, None)
        .await
        .expect("read quota")
        .expect("quota exists");
    assert_eq!(bucket.get_i64("total").expect("quota total"), 3);
    let segments = bucket.get_document("segments").expect("segments");
    assert_eq!(segments.get_i64("calendar").expect("calendar count"), 2);
    assert_eq!(segments.get_i64("renewal").expect("renewal count"), 1);
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn existing_bucket_catches_up_with_late_legacy_event_baseline() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let contact = contact("workspace-rolling", "account-a", "wxid-rolling", now);

    let mut first = follow_up(&contact, "rolling:new-protocol-first", now, 4);
    first.quota.namespace = "sr135_rolling_bridge";
    assert_eq!(
        commit_follow_up(&app.state, first)
            .await
            .expect("create quota bucket"),
        CommitOutcome::Emitted
    );

    // Simulate two emits from an older binary after the new bucket already
    // exists. Production callers observe these through their legacy event
    // count and pass total/segment baselines of three (the first protocol emit
    // plus these two late legacy emits).
    for index in 0..2 {
        app.state
            .db
            .events()
            .clone_with_type::<Document>()
            .insert_one(
                doc! {
                    "workspace_id": "workspace-rolling",
                    "account_id": "account-a",
                    "contact_wxid": "legacy",
                    "kind": "sr135_emit",
                    "status": "emitted",
                    "summary": format!("legacy {index}"),
                    "created_at": now,
                },
                None,
            )
            .await
            .expect("insert late legacy event");
    }

    let mut last_slot = follow_up(&contact, "rolling:last-slot", now, 4);
    last_slot.quota.namespace = "sr135_rolling_bridge";
    last_slot.quota.initial_total = 3;
    last_slot.quota.initial_segment = 3;
    assert_eq!(
        commit_follow_up(&app.state, last_slot)
            .await
            .expect("reserve after legacy catch-up"),
        CommitOutcome::Emitted
    );

    let mut beyond_cap = follow_up(&contact, "rolling:beyond-cap", now, 4);
    beyond_cap.quota.namespace = "sr135_rolling_bridge";
    beyond_cap.quota.initial_total = 4;
    beyond_cap.quota.initial_segment = 4;
    assert_eq!(
        commit_follow_up(&app.state, beyond_cap)
            .await
            .expect("enforce reconciled cap"),
        CommitOutcome::Capped
    );

    let bucket = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find_one(doc! { "namespace": "sr135_rolling_bridge" }, None)
        .await
        .expect("read rolling bucket")
        .expect("rolling bucket exists");
    assert_eq!(bucket.get_i64("total").expect("quota total"), 4);
    assert_eq!(
        bucket
            .get_document("segments")
            .expect("segments")
            .get_i64("silent")
            .expect("silent total"),
        4
    );
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": "workspace-rolling" }, None)
            .await
            .expect("count rolling tasks"),
        2
    );
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn event_insert_failure_rolls_back_task_and_quota() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let contact = contact("workspace-rollback", "account-a", "wxid-rollback", now);

    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_events",
                "validator": { "kind": { "$ne": "sr135_emit" } },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install event validator");

    let result = commit_follow_up(
        &app.state,
        follow_up(&contact, "rollback-generation", now, 10),
    )
    .await;
    assert!(result.is_err(), "validator must reject the event insert");
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": "workspace-rollback" }, None)
            .await
            .expect("count tasks"),
        0
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "workspace_id": "workspace-rollback" }, None)
            .await
            .expect("count events"),
        0
    );
    assert_eq!(
        app.state
            .db
            .raw()
            .collection::<Document>("proactive_daily_quotas")
            .count_documents(doc! { "namespace": "sr135_follow_up" }, None)
            .await
            .expect("count quotas"),
        0
    );

    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_events",
                "validator": {},
                "validationLevel": "off",
            },
            None,
        )
        .await
        .expect("remove event validator");
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB"]
async fn silence_duplicate_does_not_consume_persistent_daily_quota() {
    let app = TestApp::start_repl_set().await;
    let now = DateTime::now();
    let first = behavior_signals::build_silence(
        "workspace-silence",
        "account-a",
        "wxid-first",
        DateTime::from_millis(now.timestamp_millis() - 100_000),
        now,
    );
    let second = behavior_signals::build_silence(
        "workspace-silence",
        "account-a",
        "wxid-second",
        DateTime::from_millis(now.timestamp_millis() - 200_000),
        now,
    );
    let signal_quota = DailyQuota {
        namespace: "sr135_silence",
        account_scope: None,
        total_cap: 1,
        segment_cap: None,
        initial_total: 0,
        initial_segment: 0,
    };

    assert_eq!(
        commit_signal_with_daily_quota(&app.state, first.clone(), "silence", signal_quota.clone(),)
            .await
            .expect("first signal"),
        CommitOutcome::Emitted
    );
    assert_eq!(
        commit_signal_with_daily_quota(&app.state, first, "silence", signal_quota.clone(),)
            .await
            .expect("duplicate signal"),
        CommitOutcome::Duplicate
    );
    assert_eq!(
        commit_signal_with_daily_quota(&app.state, second, "silence", signal_quota)
            .await
            .expect("capped signal"),
        CommitOutcome::Capped
    );
    assert_eq!(
        app.state
            .db
            .behavior_signals()
            .count_documents(doc! { "workspace_id": "workspace-silence" }, None)
            .await
            .expect("count signals"),
        1
    );
    let bucket = app
        .state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas")
        .find_one(doc! { "namespace": "sr135_silence" }, None)
        .await
        .expect("read quota")
        .expect("quota exists");
    assert_eq!(bucket.get_i64("total").expect("quota total"), 1);
    app.cleanup().await;
}
