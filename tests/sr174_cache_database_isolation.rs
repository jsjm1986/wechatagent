//! SR-174: process-local runtime caches must not cross isolated Mongo databases.
#![cfg(test)]

mod common;

use mongodb::bson::DateTime;
use wechatagent::agent::{
    default_domain_profile, init_global_domain_profile_cache, init_global_taxonomy_cache,
    load_active_domain_profile, normalize_target_stages,
};
use wechatagent::models::{TaxonomyEntry, TaxonomyValue};

use crate::common::TestApp;

#[derive(Debug)]
struct IsolationEvidence {
    a_profiles: Vec<String>,
    b_profiles: Vec<String>,
    a_stages: Vec<Vec<String>>,
    b_stages: Vec<Vec<String>>,
    a_rejects_b: bool,
    b_rejects_a: bool,
}

async fn seed_database(app: &TestApp, profile_id: &str, stage_id: &str) -> anyhow::Result<()> {
    let workspace_id = &app.state.config.default_workspace_id;
    let mut profile = default_domain_profile(workspace_id);
    profile.profile_id = profile_id.to_string();
    profile.display_name = profile_id.to_string();
    profile.is_active = true;
    profile.current_version = true;
    profile.updated_at = DateTime::now();
    app.state
        .db
        .domain_profiles()
        .insert_one(profile, None)
        .await?;

    app.state
        .db
        .collection_system_taxonomies()
        .insert_one(
            TaxonomyEntry {
                id: None,
                workspace_id: workspace_id.clone(),
                scope: "global".to_string(),
                kind: "customer_stage".to_string(),
                value: TaxonomyValue {
                    id: stage_id.to_string(),
                    display_name: stage_id.to_string(),
                    description: "SR-174 database-isolation marker".to_string(),
                    aliases: vec![],
                    status: "active".to_string(),
                    priority_weight: None,
                    is_terminal: false,
                    is_reactivation_target: false,
                },
                updated_at: DateTime::now(),
                version: 1,
                current_version: true,
                previous_version: None,
                seeded_by: Some("sr174_test".to_string()),
            },
            None,
        )
        .await?;
    Ok(())
}

async fn read_profile_and_stage(
    app: &TestApp,
    stage_id: &str,
) -> anyhow::Result<(String, Vec<String>)> {
    let workspace_id = &app.state.config.default_workspace_id;
    let profile = load_active_domain_profile(&app.state.db, workspace_id).await;
    let stages = normalize_target_stages(
        &app.state.db,
        workspace_id,
        &app.state.config.default_account_id,
        &[stage_id.to_string()],
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok((profile.profile_id, stages))
}

async fn exercise_interleaved_databases(
    app_a: &TestApp,
    app_b: &TestApp,
) -> anyhow::Result<IsolationEvidence> {
    const PROFILE_A: &str = "sr174-profile-a";
    const PROFILE_B: &str = "sr174-profile-b";
    const STAGE_A: &str = "sr174_stage_a";
    const STAGE_B: &str = "sr174_stage_b";

    seed_database(app_a, PROFILE_A, STAGE_A).await?;
    seed_database(app_b, PROFILE_B, STAGE_B).await?;

    let mut evidence = IsolationEvidence {
        a_profiles: vec![],
        b_profiles: vec![],
        a_stages: vec![],
        b_stages: vec![],
        a_rejects_b: false,
        b_rejects_a: false,
    };

    // Warm both databases before reading the first one. With the old shared
    // cache, B's warm-up overwrote A and the following A read returned B's
    // profile/taxonomy. Repeat in reverse order to cover both directions.
    init_global_domain_profile_cache(&app_a.state.db).await;
    init_global_taxonomy_cache(&app_a.state.db).await;
    init_global_domain_profile_cache(&app_b.state.db).await;
    init_global_taxonomy_cache(&app_b.state.db).await;

    let (profile, stages) = read_profile_and_stage(app_a, STAGE_A).await?;
    evidence.a_profiles.push(profile);
    evidence.a_stages.push(stages);

    let (profile, stages) = read_profile_and_stage(app_b, STAGE_B).await?;
    evidence.b_profiles.push(profile);
    evidence.b_stages.push(stages);

    init_global_domain_profile_cache(&app_b.state.db).await;
    init_global_taxonomy_cache(&app_b.state.db).await;
    init_global_domain_profile_cache(&app_a.state.db).await;
    init_global_taxonomy_cache(&app_a.state.db).await;

    let (profile, stages) = read_profile_and_stage(app_b, STAGE_B).await?;
    evidence.b_profiles.push(profile);
    evidence.b_stages.push(stages);

    let (profile, stages) = read_profile_and_stage(app_a, STAGE_A).await?;
    evidence.a_profiles.push(profile);
    evidence.a_stages.push(stages);

    let workspace_a = &app_a.state.config.default_workspace_id;
    let workspace_b = &app_b.state.config.default_workspace_id;
    evidence.a_rejects_b = normalize_target_stages(
        &app_a.state.db,
        workspace_a,
        &app_a.state.config.default_account_id,
        &[STAGE_B.to_string()],
    )
    .await
    .is_err();
    evidence.b_rejects_a = normalize_target_stages(
        &app_b.state.db,
        workspace_b,
        &app_b.state.config.default_account_id,
        &[STAGE_A.to_string()],
    )
    .await
    .is_err();

    Ok(evidence)
}

#[tokio::test]
async fn sr174_same_workspace_caches_remain_scoped_to_their_database() {
    let app_a = TestApp::start().await;
    let app_b = TestApp::start().await;

    let result = exercise_interleaved_databases(&app_a, &app_b).await;

    // Collect evidence before asserting so both random databases are removed
    // even when the isolation contract fails.
    app_a.cleanup().await;
    app_b.cleanup().await;

    let evidence = result.expect("exercise SR-174 interleaved databases");
    assert_eq!(
        evidence.a_profiles,
        vec!["sr174-profile-a", "sr174-profile-a"]
    );
    assert_eq!(
        evidence.b_profiles,
        vec!["sr174-profile-b", "sr174-profile-b"]
    );
    assert_eq!(
        evidence.a_stages,
        vec![vec!["sr174_stage_a"], vec!["sr174_stage_a"]]
    );
    assert_eq!(
        evidence.b_stages,
        vec![vec!["sr174_stage_b"], vec!["sr174_stage_b"]]
    );
    assert!(
        evidence.a_rejects_b,
        "database A must reject B-only taxonomy"
    );
    assert!(
        evidence.b_rejects_a,
        "database B must reject A-only taxonomy"
    );
}
