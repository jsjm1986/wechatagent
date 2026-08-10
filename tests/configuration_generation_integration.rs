//! Cross-replica runtime configuration generation redline.
//!
//! Two independent Database wrappers model two application replicas connected to the same
//! Mongo database. A write through replica A must be visible on replica B's very next runtime
//! read, without process-local invalidation or a 30-second TTL wait.

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::agent::{
    default_domain_profile, ensure_workspace_taxonomies, inspect_taxonomy_value,
    load_active_domain_profile,
};
use wechatagent::db::{
    config_generation::{
        bump_generation, read_generation, DOMAIN_PROFILE_NAMESPACE, LLM_PROVIDER_NAMESPACE,
        TAXONOMY_NAMESPACE,
    },
    Database,
};
use wechatagent::llm::{
    ensure_default_llm_provider, LlmClient, LlmFormat, LlmProviderMeta, LlmRegistry,
};
use wechatagent::models::{LlmProviderConfig, TaxonomyEntry, TaxonomyValue};

fn registry(provider_id: &str, model: &str) -> Arc<LlmRegistry> {
    let client = LlmClient::with_format(
        "http://127.0.0.1:1/v1".to_string(),
        "test-key".to_string(),
        model.to_string(),
        LlmFormat::Openai,
        1,
        1,
        100,
    )
    .expect("build registry client");
    Arc::new(LlmRegistry::new(
        "default",
        client,
        LlmProviderMeta {
            provider_id: provider_id.to_string(),
            format: LlmFormat::Openai,
            model: model.to_string(),
            base_url: "http://127.0.0.1:1/v1".to_string(),
            revision_ms: 0,
            runtime_fingerprint: format!("fixture:{provider_id}:{model}"),
        },
    ))
}

fn provider(provider_id: &str, model: &str, updated_at: DateTime) -> LlmProviderConfig {
    LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        provider_id: provider_id.to_string(),
        name: provider_id.to_string(),
        format: "openai".to_string(),
        base_url: "http://127.0.0.1:1/v1".to_string(),
        api_key: "test-key".to_string(),
        model: model.to_string(),
        is_active: true,
        timeout_seconds: Some(1),
        max_retries: Some(1),
        retry_base_ms: Some(100),
        supports_vision: false,
        is_vision_active: false,
        created_at: updated_at,
        updated_at,
    }
}

#[tokio::test]
#[ignore = "requires MongoDB"]
async fn cross_replica_configuration_revisions_refresh_on_next_read() {
    let app = common::TestApp::start().await;
    let replica_b = Database::connect(
        &app.state.config.mongodb_uri,
        &app.state.config.mongodb_database,
    )
    .await
    .expect("connect independent replica B database wrapper");

    // Domain profile: replica B caches v1. Replica A then performs the source mutation and
    // advances the shared generation, matching the production activation protocol. Replica B
    // must observe v2 on its very next read without waiting for the recovery TTL.
    let mut profile = default_domain_profile("default");
    let profile_id = ObjectId::new();
    profile.id = Some(profile_id);
    profile.profile_id = "cross_replica_profile".to_string();
    profile.prompt_fragment = Some("profile-v1".to_string());
    profile.updated_at = DateTime::from_millis(1_700_000_000_000);
    app.state
        .db
        .domain_profiles()
        .insert_one(&profile, None)
        .await
        .expect("insert active profile");
    assert_eq!(
        load_active_domain_profile(&replica_b, "default")
            .await
            .expect("replica B loads profile v1")
            .prompt_fragment
            .as_deref(),
        Some("profile-v1")
    );
    app.state
        .db
        .domain_profiles()
        .update_one(
            doc! { "_id": profile_id, "is_active": true },
            doc! { "$set": { "prompt_fragment": "profile-v2" } },
            None,
        )
        .await
        .expect("replica A updates active profile");
    bump_generation(&app.state.db, DOMAIN_PROFILE_NAMESPACE, "default")
        .await
        .expect("publish domain profile generation");
    assert_eq!(
        load_active_domain_profile(&replica_b, "default")
            .await
            .expect("replica B immediately refreshes profile")
            .prompt_fragment
            .as_deref(),
        Some("profile-v2")
    );

    // Taxonomy: cache a custom current row, then add an alias and advance the workspace
    // generation. The next read reloads only this workspace rather than scanning every tenant.
    let taxonomy_id = ObjectId::new();
    app.state
        .db
        .collection_system_taxonomies()
        .insert_one(
            TaxonomyEntry {
                id: Some(taxonomy_id),
                workspace_id: "default".to_string(),
                scope: "global".to_string(),
                kind: "cross_replica_kind".to_string(),
                value: TaxonomyValue {
                    id: "canonical-v1".to_string(),
                    display_name: "Canonical V1".to_string(),
                    description: String::new(),
                    aliases: Vec::new(),
                    status: "active".to_string(),
                    priority_weight: Some(1),
                    is_terminal: false,
                    is_reactivation_target: false,
                },
                updated_at: DateTime::from_millis(1_700_000_000_000),
                version: 1,
                current_version: true,
                previous_version: None,
                seeded_by: Some("test".to_string()),
            },
            None,
        )
        .await
        .expect("insert current taxonomy row");
    assert_eq!(
        inspect_taxonomy_value(
            &replica_b,
            "default",
            "account-a",
            "cross_replica_kind",
            "new-alias",
        )
        .await
        .expect("replica B warms taxonomy cache"),
        "candidate_new"
    );
    app.state
        .db
        .collection_system_taxonomies()
        .update_one(
            doc! { "_id": taxonomy_id, "current_version": true },
            doc! { "$set": { "value.aliases": ["new-alias"] } },
            None,
        )
        .await
        .expect("replica A updates taxonomy alias");
    bump_generation(&app.state.db, TAXONOMY_NAMESPACE, "default")
        .await
        .expect("publish taxonomy generation");
    assert_eq!(
        inspect_taxonomy_value(
            &replica_b,
            "default",
            "account-a",
            "cross_replica_kind",
            "new-alias",
        )
        .await
        .expect("replica B immediately refreshes taxonomy"),
        "alias:canonical-v1"
    );

    // Provider registry: two process-local registries pin the same DB active row. A runtime-field
    // edit plus the committed provider generation must rebuild replica B on its next read.
    let provider_v1 = provider(
        "provider-cross-replica",
        "model-v1",
        DateTime::from_millis(1_700_000_000_000),
    );
    app.state
        .db
        .llm_provider_configs()
        .insert_one(&provider_v1, None)
        .await
        .expect("insert active provider");
    let registry_a = registry("stale-a", "stale-a");
    let registry_b = registry("stale-b", "stale-b");
    let a = registry_a
        .snapshot_synced(&app.state.db, &app.state.config, "default")
        .await
        .expect("replica A syncs provider");
    let b = registry_b
        .snapshot_synced(&replica_b, &app.state.config, "default")
        .await
        .expect("replica B syncs provider");
    assert_eq!(a.meta.model, "model-v1");
    assert_eq!(b.meta.model, "model-v1");

    app.state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": "default", "providerId": "provider-cross-replica", "isActive": true },
            doc! { "$set": { "model": "model-v2" } },
            None,
        )
        .await
        .expect("replica A updates active provider revision");
    bump_generation(&app.state.db, LLM_PROVIDER_NAMESPACE, "default")
        .await
        .expect("publish provider generation");
    let refreshed = registry_b
        .snapshot_synced(&replica_b, &app.state.config, "default")
        .await
        .expect("replica B immediately refreshes provider");
    assert_eq!(refreshed.meta.provider_id, "provider-cross-replica");
    assert_eq!(refreshed.meta.model, "model-v2");
    assert_eq!(refreshed.meta.revision_ms, 1_700_000_000_000);
    assert_eq!(refreshed.generation, b.generation + 1);

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB replica set"]
async fn initialization_writes_publish_generation_once_and_are_idempotent() {
    let app = common::TestApp::start_repl_set().await;

    let taxonomy_workspace = "configuration-init-taxonomy";
    assert_eq!(
        read_generation(&app.state.db, TAXONOMY_NAMESPACE, taxonomy_workspace)
            .await
            .expect("read taxonomy generation before initialization"),
        0
    );
    assert!(
        ensure_workspace_taxonomies(&app.state.db, taxonomy_workspace)
            .await
            .expect("initialize workspace taxonomy")
    );
    assert!(app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("migrations")
        .find_one(
            doc! { "_id": format!("workspace_taxonomy_template_v1:{taxonomy_workspace}") },
            None,
        )
        .await
        .expect("read taxonomy marker")
        .is_some());
    assert!(
        app.state
            .db
            .collection_system_taxonomies()
            .count_documents(doc! { "workspace_id": taxonomy_workspace }, None)
            .await
            .expect("count initialized taxonomies")
            > 0
    );
    let taxonomy_generation =
        read_generation(&app.state.db, TAXONOMY_NAMESPACE, taxonomy_workspace)
            .await
            .expect("read taxonomy generation after initialization");
    assert_eq!(taxonomy_generation, 1);
    assert!(
        !ensure_workspace_taxonomies(&app.state.db, taxonomy_workspace)
            .await
            .expect("repeat workspace taxonomy initialization")
    );
    assert_eq!(
        read_generation(&app.state.db, TAXONOMY_NAMESPACE, taxonomy_workspace)
            .await
            .expect("read taxonomy generation after idempotent repeat"),
        taxonomy_generation,
        "durable marker must prevent both reseeding and an extra generation bump",
    );

    let provider_workspace = "configuration-init-provider";
    let mut provider_config = app.state.config.clone();
    provider_config.default_workspace_id = provider_workspace.to_string();
    assert_eq!(
        read_generation(&app.state.db, LLM_PROVIDER_NAMESPACE, provider_workspace,)
            .await
            .expect("read provider generation before initialization"),
        0
    );
    let first = ensure_default_llm_provider(&app.state.db, &provider_config)
        .await
        .expect("initialize default provider");
    assert!(first.is_active);
    assert_eq!(first.workspace_id, provider_workspace);
    let provider_generation =
        read_generation(&app.state.db, LLM_PROVIDER_NAMESPACE, provider_workspace)
            .await
            .expect("read provider generation after initialization");
    assert_eq!(provider_generation, 1);
    let second = ensure_default_llm_provider(&app.state.db, &provider_config)
        .await
        .expect("repeat default provider initialization");
    assert_eq!(second.id, first.id);
    assert_eq!(
        read_generation(&app.state.db, LLM_PROVIDER_NAMESPACE, provider_workspace,)
            .await
            .expect("read provider generation after idempotent repeat"),
        provider_generation,
        "an existing active provider must not advance the generation",
    );
    assert_eq!(
        app.state
            .db
            .llm_provider_configs()
            .count_documents(
                doc! { "workspaceId": provider_workspace, "isActive": true },
                None,
            )
            .await
            .expect("count active initialized providers"),
        1
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB replica set"]
async fn concurrent_initialization_across_replicas_commits_once() {
    let app = common::TestApp::start_repl_set().await;
    let replica_b = Database::connect(
        &app.state.config.mongodb_uri,
        &app.state.config.mongodb_database,
    )
    .await
    .expect("connect independent replica B database wrapper");

    let taxonomy_workspace = "configuration-init-taxonomy-concurrent";
    let (taxonomy_a, taxonomy_b) = tokio::join!(
        ensure_workspace_taxonomies(&app.state.db, taxonomy_workspace),
        ensure_workspace_taxonomies(&replica_b, taxonomy_workspace),
    );
    let taxonomy_a = taxonomy_a.expect("replica A taxonomy initialization");
    let taxonomy_b = taxonomy_b.expect("replica B taxonomy initialization");
    assert_ne!(
        taxonomy_a, taxonomy_b,
        "exactly one replica must perform the durable taxonomy seed",
    );
    assert_eq!(
        read_generation(&app.state.db, TAXONOMY_NAMESPACE, taxonomy_workspace)
            .await
            .expect("read concurrent taxonomy generation"),
        1,
        "concurrent taxonomy initialization must publish exactly one generation",
    );
    assert_eq!(
        app.state
            .db
            .raw()
            .collection::<mongodb::bson::Document>("migrations")
            .count_documents(
                doc! { "_id": format!("workspace_taxonomy_template_v1:{taxonomy_workspace}") },
                None,
            )
            .await
            .expect("count concurrent taxonomy markers"),
        1,
    );

    let provider_workspace = "configuration-init-provider-concurrent";
    let mut provider_config = app.state.config.clone();
    provider_config.default_workspace_id = provider_workspace.to_string();
    let (provider_a, provider_b) = tokio::join!(
        ensure_default_llm_provider(&app.state.db, &provider_config),
        ensure_default_llm_provider(&replica_b, &provider_config),
    );
    let provider_a = provider_a.expect("replica A provider initialization");
    let provider_b = provider_b.expect("replica B provider initialization");
    assert_eq!(provider_a.id, provider_b.id);
    assert_eq!(provider_a.provider_id, provider_b.provider_id);
    assert_eq!(
        read_generation(&app.state.db, LLM_PROVIDER_NAMESPACE, provider_workspace)
            .await
            .expect("read concurrent provider generation"),
        1,
        "concurrent provider initialization must publish exactly one generation",
    );
    assert_eq!(
        app.state
            .db
            .llm_provider_configs()
            .count_documents(
                doc! { "workspaceId": provider_workspace, "isActive": true },
                None,
            )
            .await
            .expect("count concurrent active providers"),
        1,
    );

    app.cleanup().await;
}
