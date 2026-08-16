//! Immutable per-turn authority compilation and append-only evidence ledger.
//!
//! Natural-language meaning remains an AI decision. This module enforces only provenance,
//! ownership, lifecycle, freshness metadata, bounded context, and immutable source identity.

use std::collections::HashSet;
use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, DateTime, Document};
use mongodb::options::{FindOneOptions, FindOptions};
use mongodb::ClientSession;
use parking_lot::RwLock;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentTurnSnapshot, Appointment, AuthorityObservation, CommitmentRepr, Contact, ContentAsset,
    ConversationMessage, MemoryFactRepr, MessageDirection, OperatingMemory,
    OperationKnowledgeChunk, PersonaWorldState, Product, ReferralCard,
};
use crate::routes::AppState;

use super::sufficiency::PromptTier;

pub(crate) const AUTHORITY_VERSION: i32 = 1;
const MAX_BASE_SOURCES: usize = 64;
const MAX_LEDGER_SOURCES: usize = 32;
const MAX_SOURCE_TEXT_CHARS: usize = 4_000;
const MAX_PROMPT_CHARS: usize = 24_000;
const SNAPSHOT_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// Persist a trusted, structured observation that may be compiled into a later turn's authority
/// bundle. This is intentionally a provenance API rather than a semantic classifier: callers must
/// provide the exact source identity, subject, content, and boundary they already verified.
#[allow(dead_code)]
pub(crate) async fn record_authority_observation(
    state: &AppState,
    observation: AuthorityObservation,
) -> AppResult<()> {
    let (identity, document) = prepare_observation(observation)?;
    let collection = state
        .db
        .authority_observations()
        .clone_with_type::<Document>();
    let result = collection
        .update_one(
            identity.clone(),
            observation_insert_update(document.clone()),
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await?;
    let existing = if result.upserted_id.is_none() {
        collection.find_one(identity, None).await?
    } else {
        None
    };
    ensure_observation_write(&result, existing.as_ref(), &document)
}

/// Persist an observation while the caller's Mongo transaction is still open. This is used by
/// authority-bearing state transitions (human decisions and appointment confirmations) so the
/// state change cannot commit without its provenance record.
pub(crate) async fn record_authority_observation_with_session(
    db: &Database,
    session: &mut ClientSession,
    observation: AuthorityObservation,
) -> AppResult<()> {
    let (identity, document) = prepare_observation(observation)?;
    let collection = db.authority_observations().clone_with_type::<Document>();
    let result = collection
        .update_one_with_session(
            identity.clone(),
            observation_insert_update(document.clone()),
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
            session,
        )
        .await?;
    let existing = if result.upserted_id.is_none() {
        collection
            .find_one_with_session(identity, None, session)
            .await?
    } else {
        None
    };
    ensure_observation_write(&result, existing.as_ref(), &document)
}

/// Authority observations are immutable facts. Retries for the same source identity must be
/// idempotent, while a changed fact must arrive under a new source id (or a new versioned
/// identity) instead of silently rewriting the historical record.
fn observation_insert_update(document: Document) -> Document {
    doc! { "$setOnInsert": document }
}

fn prepare_observation(mut observation: AuthorityObservation) -> AppResult<(Document, Document)> {
    validate_observation(&observation)?;
    let now = DateTime::now();
    if observation.id.is_none() {
        observation.id = Some(mongodb::bson::oid::ObjectId::new());
    }
    if observation.created_at.timestamp_millis() == 0 {
        observation.created_at = now;
    }
    observation.updated_at = now;
    let mut document = mongodb::bson::to_document(&observation)?;
    document.remove("_id");
    let identity = observation_identity(&observation);
    Ok((identity, document))
}

const IMMUTABLE_OBSERVATION_FIELDS: &[&str] = &[
    "workspace_id",
    "account_id",
    "contact_wxid",
    "source_type",
    "source_id",
    "subject",
    "content",
    "authority_boundary",
    "valid_from",
    "valid_until",
    "status",
    "superseded_by",
    "source_run_id",
];

fn ensure_observation_write(
    result: &mongodb::results::UpdateResult,
    existing: Option<&Document>,
    expected: &Document,
) -> AppResult<()> {
    if result.matched_count == 0 && result.upserted_id.is_none() {
        return Err(AppError::Conflict(
            "authority observation was not persisted".to_string(),
        ));
    }
    if result.upserted_id.is_none()
        && existing.is_some_and(|current| !observation_immutable_fields_match(current, expected))
    {
        return Err(AppError::Conflict(
            "authority observation source identity was reused with different immutable content"
                .to_string(),
        ));
    }
    Ok(())
}

fn observation_immutable_fields_match(existing: &Document, expected: &Document) -> bool {
    IMMUTABLE_OBSERVATION_FIELDS.iter().all(|field| {
        let current = existing.get(*field);
        let wanted = expected.get(*field);
        match (current, wanted) {
            (None, None) => true,
            (Some(Bson::Null), None) | (None, Some(Bson::Null)) => true,
            (Some(current), Some(wanted)) => current == wanted,
            _ => false,
        }
    })
}

fn observation_identity(observation: &AuthorityObservation) -> Document {
    doc! {
        "workspace_id": &observation.workspace_id,
        "account_id": &observation.account_id,
        "contact_wxid": &observation.contact_wxid,
        "source_type": &observation.source_type,
        "source_id": &observation.source_id,
        "subject": &observation.subject,
    }
}

fn validate_observation(observation: &AuthorityObservation) -> AppResult<()> {
    let fields = [
        ("workspace_id", observation.workspace_id.as_str()),
        ("account_id", observation.account_id.as_str()),
        ("contact_wxid", observation.contact_wxid.as_str()),
        ("source_type", observation.source_type.as_str()),
        ("source_id", observation.source_id.as_str()),
        ("subject", observation.subject.as_str()),
        ("content", observation.content.as_str()),
        (
            "authority_boundary",
            observation.authority_boundary.as_str(),
        ),
    ];
    if let Some((field, _)) = fields.iter().find(|(_, value)| value.trim().is_empty()) {
        return Err(AppError::BadRequest(format!(
            "authority observation {field} must not be empty"
        )));
    }
    if observation.status != "active" {
        return Err(AppError::BadRequest(
            "only active authority observations may be recorded".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct SourceBucket {
    reserve: usize,
    sources: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityInvocation {
    Conversation,
    ManualOutreach,
}

#[derive(Clone)]
pub(crate) struct AuthorityCompileInput<'a> {
    pub state: &'a AppState,
    pub run_id: &'a str,
    pub turn_id: &'a str,
    pub contact: &'a Contact,
    pub inbound: &'a ConversationMessage,
    pub recent_messages: &'a [ConversationMessage],
    pub memory: &'a OperatingMemory,
    pub active_products: &'a [Product],
    pub referral_cards: &'a [ReferralCard],
    pub effective_soul: &'a str,
    /// Read-only state created inside a Simulation run. Durable appointments are still loaded
    /// from Mongo and merged by stable identity; production callers pass an empty slice.
    pub projected_appointments: &'a [Appointment],
    /// Optional preselected account-wide persona state. Simulation freezes one value for the
    /// complete run; production may pass the state just generated before compilation.
    pub projected_world_state: Option<&'a PersonaWorldState>,
    pub invocation: AuthorityInvocation,
    pub evaluated_at: DateTime,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthoritySnapshot {
    run_id: String,
    turn_id: String,
    workspace_id: String,
    account_id: String,
    contact_wxid: String,
    compiled_at: DateTime,
    bundle_hash: String,
    base_sources: Arc<Vec<Value>>,
    ledger: TurnEvidenceLedger,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnEvidenceLedger {
    entries: Arc<RwLock<Vec<Value>>>,
}

impl TurnEvidenceLedger {
    pub(crate) fn append(&self, source: Value) -> AppResult<bool> {
        let id = source
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::External("authority source missing id".to_string()))?;
        let mut entries = self.entries.write();
        if let Some(existing) = entries
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        {
            if existing == &source {
                return Ok(false);
            }
            return Err(AppError::Conflict(format!(
                "authority source identity changed within turn: {id}"
            )));
        }
        if entries.len() >= MAX_LEDGER_SOURCES {
            return Err(AppError::External(
                "turn evidence ledger source limit exceeded".to_string(),
            ));
        }
        entries.push(source);
        Ok(true)
    }

    pub(crate) fn entries(&self) -> Vec<Value> {
        self.entries.read().clone()
    }

    pub(crate) fn hash(&self) -> String {
        stable_source_hash(&self.entries())
    }
}

impl AuthoritySnapshot {
    pub(crate) fn bundle_hash(&self) -> &str {
        &self.bundle_hash
    }

    pub(crate) fn ledger(&self) -> &TurnEvidenceLedger {
        &self.ledger
    }

    /// Return the immutable authority bundle projected for one model visibility tier. The tier is
    /// an explicit argument rather than mutable snapshot state: Lean and Full generations may be
    /// interleaved by async orchestration, and a shared "current tier" would leak Full-only assets
    /// into a Lean prompt or hide evidence from a later authorization pass.
    pub(crate) fn evidence_catalog_for_tier(&self, tier: PromptTier) -> Vec<Value> {
        let mut sources = self.base_sources.as_ref().clone();
        sources.extend(self.ledger.entries());
        sources.retain(|source| source_visible_at_tier(source, tier));
        sources
    }

    /// Claim authorization uses the complete immutable bundle by default. Prompt rendering must
    /// call [`Self::render_for_prompt`] with the pass-specific tier instead.
    pub(crate) fn evidence_catalog(&self) -> Vec<Value> {
        self.evidence_catalog_for_tier(PromptTier::Full)
    }

    pub(crate) fn render_for_prompt(&self, tier: PromptTier) -> String {
        render_prompt_payload(
            &self.bundle_hash,
            &self.ledger.hash(),
            self.evidence_catalog_for_tier(tier),
        )
    }

    pub(crate) fn append_verified_knowledge(
        &self,
        chunks: &[OperationKnowledgeChunk],
        used_ids: &[String],
        evaluated_at: DateTime,
    ) -> AppResult<usize> {
        let used = used_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let mut appended = 0;
        for chunk in chunks.iter().filter(|chunk| {
            chunk
                .id
                .map(|id| used.contains(id.to_hex().as_str()))
                .unwrap_or(false)
                && crate::agent::guards::is_verified(chunk, evaluated_at)
        }) {
            let Some(id) = chunk.id else { continue };
            let text = chunk
                .source_quote
                .as_deref()
                .or(chunk.body.as_deref())
                .or(chunk.summary.as_deref())
                .unwrap_or_default();
            let source = claim_source(
                format!("verified_knowledge:{}", id.to_hex()),
                "verified_knowledge",
                "business",
                "May support only claims directly entailed by this currently verified text.",
                true,
                &["business", "customer", "third_party", "general"],
                true,
                json!({
                    "title": chunk.title,
                    "text": bounded_text(text),
                    "validFromMillis": chunk.valid_from.map(|value| value.timestamp_millis()),
                    "validUntilMillis": chunk.valid_to.map(|value| value.timestamp_millis()),
                }),
            );
            appended += usize::from(self.ledger.append(source)?);
        }
        Ok(appended)
    }

    pub(crate) fn append_selected_referral(
        &self,
        card: &ReferralCard,
        card_id: &str,
    ) -> AppResult<bool> {
        self.ledger.append(claim_source(
            format!("approved_referral_card:{card_id}"),
            "approved_referral_card",
            "third_party",
            "Authorizes only sending this exact reviewed advisor card and describing that controlled referral action; it establishes no price, schedule, outcome, or unrelated service fact.",
            true,
            &["business", "third_party"],
            false,
            json!({
                "displayName": card.display_name,
                "sendTriggerHint": card.send_trigger_hint,
                "targetStages": card.target_stages,
                "tags": card.tags,
            }),
        ))
    }

    pub(crate) async fn persist_initial(&self, db: &Database) -> AppResult<()> {
        let now_ms = self.compiled_at.timestamp_millis();
        let snapshot = AgentTurnSnapshot {
            id: None,
            run_id: self.run_id.clone(),
            turn_id: self.turn_id.clone(),
            workspace_id: self.workspace_id.clone(),
            account_id: self.account_id.clone(),
            contact_wxid: self.contact_wxid.clone(),
            authority_version: AUTHORITY_VERSION,
            bundle_hash: self.bundle_hash.clone(),
            sources: values_to_documents(self.base_sources.as_ref()),
            evidence_ledger: Vec::new(),
            loop_trace: Vec::new(),
            authorization_manifest: None,
            commit_receipt: None,
            created_at: self.compiled_at,
            expires_at: DateTime::from_millis(now_ms.saturating_add(SNAPSHOT_TTL_MS)),
        };
        let mut insert = mongodb::bson::to_document(&snapshot)?;
        insert.remove("_id");
        db.agent_turn_snapshots()
            .clone_with_type::<Document>()
            .update_one(
                doc! { "run_id": &self.run_id, "turn_id": &self.turn_id },
                doc! { "$setOnInsert": insert },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        let persisted = db
            .agent_turn_snapshots()
            .find_one(
                doc! { "run_id": &self.run_id, "turn_id": &self.turn_id },
                None,
            )
            .await?
            .ok_or_else(|| {
                AppError::Conflict("authority snapshot disappeared after persistence".to_string())
            })?;
        validate_persisted_snapshot(self, &persisted)?;
        Ok(())
    }

    pub(crate) async fn persist_runtime_state(
        &self,
        db: &Database,
        loop_trace: &[Document],
        authorization_manifest: Option<Document>,
        commit_receipt: Option<Document>,
    ) -> AppResult<()> {
        let result = db
            .agent_turn_snapshots()
            .clone_with_type::<Document>()
            .update_one(
                doc! {
                    "run_id": &self.run_id,
                    "turn_id": &self.turn_id,
                    "bundle_hash": &self.bundle_hash,
                },
                doc! {
                    "$set": {
                        "evidence_ledger": values_to_documents(&self.ledger.entries()),
                        "loop_trace": loop_trace,
                        "authorization_manifest": authorization_manifest,
                        "commit_receipt": commit_receipt,
                    }
                },
                None,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict(
                "authority snapshot changed before runtime state persistence".to_string(),
            ));
        }
        Ok(())
    }

    /// Persist the evidence ledger, final authorization manifest, and commit receipt inside the
    /// same transaction as production side effects. The post-commit runtime writer remains useful
    /// for loop trace enrichment, but this method is the durable authority boundary.
    pub(crate) async fn persist_commit_state_with_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        authorization_manifest: Document,
        commit_receipt: Document,
    ) -> AppResult<()> {
        let result = db
            .agent_turn_snapshots()
            .clone_with_type::<Document>()
            .update_one_with_session(
                doc! {
                    "run_id": &self.run_id,
                    "turn_id": &self.turn_id,
                    "bundle_hash": &self.bundle_hash,
                },
                doc! {
                    "$set": {
                        "evidence_ledger": values_to_documents(&self.ledger.entries()),
                        "authorization_manifest": authorization_manifest,
                        "commit_receipt": commit_receipt,
                    }
                },
                None,
                session,
            )
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict(
                "authority snapshot changed before atomic commit state persistence".to_string(),
            ));
        }
        Ok(())
    }
}

pub(crate) async fn compile(input: AuthorityCompileInput<'_>) -> AppResult<AuthoritySnapshot> {
    let ownership = doc! {
        "workspace_id": &input.contact.workspace_id,
        "account_id": &input.contact.account_id,
        "contact_wxid": &input.contact.wxid,
    };
    let now = input.evaluated_at;
    let appointments_future = async {
        input
            .state
            .db
            .appointments()
            .find(
                {
                    let mut filter = ownership.clone();
                    filter.insert(
                        "status",
                        doc! { "$in": ["requested", "pending_confirmation", "confirmed", "reschedule_requested"] },
                    );
                    filter
                },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .limit(12)
                    .build(),
            )
            .await?
            .try_collect::<Vec<Appointment>>()
            .await
            .map_err(AppError::from)
    };
    let observations_future = async {
        let mut filter = ownership.clone();
        filter.insert("status", "active");
        filter.insert(
            "$and",
            vec![
                doc! { "$or": [{ "valid_from": null }, { "valid_from": { "$lte": now } }] },
                doc! { "$or": [{ "valid_until": null }, { "valid_until": { "$gt": now } }] },
            ],
        );
        input
            .state
            .db
            .authority_observations()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(24)
                    .build(),
            )
            .await?
            .try_collect::<Vec<AuthorityObservation>>()
            .await
            .map_err(AppError::from)
    };
    let world_state_future = async {
        input
            .state
            .db
            .persona_world_states()
            .find_one(
                doc! {
                    "workspace_id": &input.contact.workspace_id,
                    "account_id": &input.contact.account_id,
                    "current": true,
                    "effective_from": { "$lte": now },
                    "effective_until": { "$gt": now },
                },
                FindOneOptions::builder()
                    .sort(doc! { "version": -1 })
                    .build(),
            )
            .await
            .map_err(AppError::from)
    };
    let text_assets_future = load_governed_text_assets(
        input.state,
        &input.contact.workspace_id,
        &input.contact.account_id,
    );
    let (mut appointments, observations, durable_world_state, text_assets) = tokio::try_join!(
        appointments_future,
        observations_future,
        world_state_future,
        text_assets_future,
    )?;
    appointments.extend(input.projected_appointments.iter().cloned());
    appointments.sort_by(|left, right| {
        right
            .updated_at
            .timestamp_millis()
            .cmp(&left.updated_at.timestamp_millis())
            .then_with(|| appointment_identity(left).cmp(&appointment_identity(right)))
    });
    let mut seen_appointments = HashSet::new();
    appointments.retain(|appointment| seen_appointments.insert(appointment_identity(appointment)));
    appointments.truncate(12);
    let world_state = input.projected_world_state.cloned().or(durable_world_state);

    let mut current_turn = Vec::new();
    let mut history = Vec::new();
    let mut persona = Vec::new();
    let mut lifecycle = Vec::new();
    let mut contact_facts = Vec::new();
    let mut memory_sources = Vec::new();
    let mut content_assets = Vec::new();
    let mut catalog = Vec::new();
    let mut observation_sources = Vec::new();
    let mut referral = Vec::new();
    append_conversation_sources(&mut current_turn, &mut history, &input);
    append_contact_salutation(&mut persona, input.contact);
    append_soul_source(&mut persona, input.effective_soul);
    append_world_state(&mut persona, world_state.as_ref());
    append_appointments(&mut lifecycle, &appointments);
    append_commitments(&mut lifecycle, input.contact);
    append_contact_facts(&mut contact_facts, input.contact);
    append_memory_facts(&mut memory_sources, input.memory, input.evaluated_at);
    append_content_assets(&mut content_assets, &text_assets);
    append_observations(&mut observation_sources, &observations);
    append_products(&mut catalog, input.active_products);
    append_referral_overview(&mut referral, input.referral_cards);
    let sources = budget_sources(vec![
        SourceBucket {
            reserve: 2,
            sources: current_turn,
        },
        SourceBucket {
            reserve: 3,
            sources: persona,
        },
        SourceBucket {
            reserve: 12,
            sources: lifecycle,
        },
        SourceBucket {
            // Operator-entered contact facts are a small, high-value authority class. Reserve
            // room for them independently so a large history or asset set cannot evict them.
            reserve: 6,
            sources: contact_facts,
        },
        SourceBucket {
            reserve: 10,
            sources: memory_sources,
        },
        SourceBucket {
            reserve: 10,
            sources: history,
        },
        SourceBucket {
            reserve: 12,
            sources: content_assets,
        },
        SourceBucket {
            reserve: 8,
            sources: observation_sources,
        },
        SourceBucket {
            reserve: 6,
            sources: catalog,
        },
        SourceBucket {
            reserve: 1,
            sources: referral,
        },
    ]);

    let bundle_hash = stable_source_hash(&sources);
    Ok(AuthoritySnapshot {
        run_id: input.run_id.to_string(),
        turn_id: input.turn_id.to_string(),
        workspace_id: input.contact.workspace_id.clone(),
        account_id: input.contact.account_id.clone(),
        contact_wxid: input.contact.wxid.clone(),
        compiled_at: input.evaluated_at,
        bundle_hash,
        base_sources: Arc::new(sources),
        ledger: TurnEvidenceLedger::default(),
    })
}

fn appointment_identity(appointment: &Appointment) -> String {
    appointment.id.map_or_else(
        || format!("idempotency:{}", appointment.idempotency_key),
        |id| format!("id:{}", id.to_hex()),
    )
}

async fn load_governed_text_assets(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Vec<ContentAsset>> {
    // Compile a bounded immutable superset here.  Visibility is projected per prompt tier in
    // `evidence_catalog_for_tier`; keeping the query tier-agnostic prevents a shared snapshot
    // from becoming a mutable "last requested tier" cache and keeps authorization evidence
    // complete even when the same turn first renders Lean and later renders Full.
    state
        .db
        .content_assets()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "$or": [{ "account_id": null }, { "account_id": account_id }],
                "enabled": true,
                "review_status": "approved",
                "body": { "$type": "string" },
                "kind": { "$nin": ["media", "forbidden_expression"] },
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(32)
                .build(),
        )
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

fn append_conversation_sources(
    current_sources: &mut Vec<Value>,
    historical_sources: &mut Vec<Value>,
    input: &AuthorityCompileInput<'_>,
) {
    if input.invocation == AuthorityInvocation::Conversation {
        if input.inbound.is_synthetic_relay {
            current_sources.push(claim_source(
                "principal_decision".to_string(),
                "principal_decision",
                "business",
                "An unforgeable current principal decision may support only claims directly entailed by its verdict, substance, and constraints; it is not reusable general knowledge.",
                true,
                &["business", "customer"],
                true,
                json!({
                    "text": bounded_text(&crate::agent::prompt_isolation::inbound_prompt_content(&input.inbound.content, true)),
                    "createdAtMillis": input.inbound.created_at.timestamp_millis(),
                    "temporalFresh": crate::agent::prompt_isolation::temporal_chat_evidence_is_fresh(input.inbound.created_at, input.evaluated_at),
                }),
            ));
        } else {
            current_sources.push(customer_message_source(
                "current_user_message".to_string(),
                "current_user_statement",
                input.inbound,
                input.evaluated_at,
                true,
            ));
        }
    }

    let mut historical = input
        .recent_messages
        .iter()
        .filter(|message| matches!(message.direction, MessageDirection::Inbound))
        .filter(|message| {
            input.invocation == AuthorityInvocation::ManualOutreach
                || !crate::agent::prompt_isolation::message_matches_inbound(message, input.inbound)
        })
        .collect::<Vec<_>>();
    historical.sort_by(|left, right| {
        right
            .created_at
            .timestamp_millis()
            .cmp(&left.created_at.timestamp_millis())
            .then_with(|| right.id.cmp(&left.id))
    });
    for (index, message) in historical.into_iter().take(12).enumerate() {
        historical_sources.push(customer_message_source(
            format!("recent_user_message:{index}"),
            "historical_user_statement",
            message,
            input.evaluated_at,
            false,
        ));
    }
}

fn customer_message_source(
    id: String,
    source_type: &str,
    message: &ConversationMessage,
    evaluated_at: DateTime,
    current: bool,
) -> Value {
    let fresh = crate::agent::prompt_isolation::temporal_chat_evidence_is_fresh(
        message.created_at,
        evaluated_at,
    );
    let rendered = if current {
        crate::agent::prompt_isolation::inbound_prompt_content(&message.content, false)
    } else {
        crate::agent::prompt_isolation::history_prompt_content(&message.content)
    };
    claim_source(
        id,
        source_type,
        "customer",
        "May support only what this customer said or a customer-side fact directly entailed by the statement while temporally fresh; it never establishes our policy, capability, appointment record, price, or outcome.",
        fresh,
        &["customer"],
        false,
        json!({
            "text": bounded_text(&rendered),
            "createdAtMillis": message.created_at.timestamp_millis(),
            "ageMillis": evaluated_at.timestamp_millis().saturating_sub(message.created_at.timestamp_millis()).max(0),
            "temporalFresh": fresh,
        }),
    )
}

fn append_contact_salutation(sources: &mut Vec<Value>, contact: &Contact) {
    let mut values = Vec::new();
    for value in [
        contact.nickname.as_deref(),
        contact.remark.as_deref(),
        contact.alias.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let value = value.trim();
        if !value.is_empty() && !values.iter().any(|existing| existing == value) {
            values.push(value.to_string());
        }
    }
    if !values.is_empty() {
        sources.push(claim_source(
            "contact_salutation".to_string(),
            "contact_salutation",
            "customer",
            "May support only addressing this contact with one listed conversational label; it establishes no legal identity, consent, history, or business fact.",
            true,
            &["customer"],
            false,
            json!({ "values": values }),
        ));
    }
}

fn append_soul_source(sources: &mut Vec<Value>, soul: &str) {
    if soul.trim().is_empty() {
        return;
    }
    sources.push(claim_source(
        "published_soul".to_string(),
        "published_soul",
        "business",
        "Authorizes the account persona, stable first-person identity, voice, and explicitly declared personal background only. It does not silently grant business capabilities, prices, appointments, or outcomes.",
        true,
        &["business", "general", "third_party"],
        false,
        json!({ "text": bounded_text(soul) }),
    ));
}

fn append_content_assets(sources: &mut Vec<Value>, assets: &[ContentAsset]) {
    for asset in assets {
        let Some(id) = asset.id else { continue };
        let Some(allowed_levels) = governed_insertion_levels(asset) else {
            continue;
        };
        sources.push(claim_source(
            format!("approved_content_asset:{}", id.to_hex()),
            "approved_content_asset",
            "business",
            "May support only statements directly contained in this enabled, operator-approved asset and only according to its usage guidance; nearby implications are not authorized.",
            true,
            &["business", "general", "third_party"],
            true,
            json!({
                "title": asset.title,
                "kind": asset.kind,
                "text": bounded_text(asset.body.as_deref().unwrap_or_default()),
                "usageGuidance": asset.usage_guidance,
                "minInjectTier": asset.min_inject_tier,
                "allowedInsertionLevels": allowed_levels,
            }),
        ));
    }
}

fn governed_insertion_levels(asset: &ContentAsset) -> Option<Vec<String>> {
    let levels = match asset.allowed_insertion_levels.clone() {
        Some(levels) if !levels.is_empty() => levels,
        _ => vec![
            "subtle".to_string(),
            "contextual".to_string(),
            "direct".to_string(),
        ],
    };
    let mut valid = levels
        .into_iter()
        .filter(|level| matches!(level.as_str(), "subtle" | "contextual" | "direct"))
        .collect::<Vec<_>>();
    valid.sort();
    valid.dedup();
    (!valid.is_empty()).then_some(valid)
}

fn source_visible_at_tier(source: &Value, current: PromptTier) -> bool {
    if source.get("sourceType").and_then(Value::as_str) != Some("approved_content_asset") {
        return true;
    }
    let min_rank = match source.get("minInjectTier").and_then(Value::as_str) {
        Some("lean") => 0,
        Some("relational") => 1,
        _ => 2,
    };
    let current_rank = match current {
        PromptTier::Lean => 0,
        PromptTier::Relational => 1,
        PromptTier::Full => 2,
    };
    if current_rank < min_rank {
        return false;
    }
    // Asset rows are normalized at the API boundary. Keep a defensive schema check here so a
    // malformed legacy row cannot silently become an unconstrained insertion instruction.
    match source.get("allowedInsertionLevels") {
        None => true,
        Some(Value::Array(levels)) => levels
            .iter()
            .any(|level| matches!(level.as_str(), Some("subtle" | "contextual" | "direct"))),
        Some(_) => false,
    }
}

fn append_products(sources: &mut Vec<Value>, products: &[Product]) {
    for product in products {
        sources.push(claim_source(
            format!("catalog:{}", product.product_id),
            "active_product_catalog",
            "business",
            "May support only the listed product identity, exact price, currency, and SKU; never capability, outcome, delivery, discount, or guarantee.",
            true,
            &["business"],
            true,
            json!({
                "productId": product.product_id,
                "name": product.name,
                "amountMinor": product.price,
                "currency": product.currency,
                "sku": product.sku,
            }),
        ));
    }
}

fn append_appointments(sources: &mut Vec<Value>, appointments: &[Appointment]) {
    for appointment in appointments {
        let Some(id) = appointment.id else { continue };
        let confirmed = appointment.status == "confirmed"
            && appointment.confirmation_source_type.is_some()
            && appointment.confirmation_source_id.is_some();
        let boundary = if confirmed {
            "Authorizes only this exact confirmed appointment status, confirmed time range, and recorded location."
        } else {
            "Authorizes only that a request or pending/reschedule state exists. It never authorizes saying the appointment, time, availability, or location is confirmed."
        };
        sources.push(claim_source(
            format!("appointment:{}", id.to_hex()),
            "appointment",
            "customer",
            boundary,
            true,
            &["business", "customer"],
            false,
            json!({
                "status": appointment.status,
                "requestText": bounded_text(&appointment.request_text),
                "requestedStartMillis": appointment.requested_start.map(|value| value.timestamp_millis()),
                "requestedEndMillis": appointment.requested_end.map(|value| value.timestamp_millis()),
                "confirmedStartMillis": appointment.confirmed_start.map(|value| value.timestamp_millis()),
                "confirmedEndMillis": appointment.confirmed_end.map(|value| value.timestamp_millis()),
                "location": appointment.location,
                "confirmationSourceType": appointment.confirmation_source_type,
            }),
        ));
    }
}

fn append_commitments(sources: &mut Vec<Value>, contact: &Contact) {
    for commitment in &contact.commitments {
        let CommitmentRepr::Structured(entry) = commitment else {
            continue;
        };
        if entry.status != "active" || entry.id.trim().is_empty() {
            continue;
        }
        sources.push(claim_source(
            format!("active_commitment:{}", entry.id),
            "active_commitment",
            "business",
            "Authorizes only acknowledging and honoring this recorded active commitment. It does not prove an unrelated capability or that the commitment has already been fulfilled.",
            true,
            &["business", "customer"],
            false,
            json!({
                "text": bounded_text(&entry.text),
                "dueAtMillis": entry.due_at.map(|value| value.timestamp_millis()),
                "createdAtMillis": entry.created_at.timestamp_millis(),
                "relatedEntityId": entry.related_entity_id.map(|id| id.to_hex()),
            }),
        ));
    }
}

/// Include operator-authored customer facts even when an older memory card already has signal
/// and therefore is not reseeded from the contact. These facts are deliberately scoped to the
/// customer subject and never authorize business capability, price, schedule, or outcome claims.
fn append_contact_facts(sources: &mut Vec<Value>, contact: &Contact) {
    if let Some(note) = contact
        .human_profile_note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_contact_fact(
            sources,
            "operator_contact_note",
            "operator_manual",
            note,
            contact.manual_tags_updated_at,
        );
    }
    for tag in &contact.manual_tags {
        append_contact_fact(
            sources,
            "operator_manual_tag",
            "operator_manual",
            tag,
            contact.manual_tags_updated_at,
        );
    }
    for tag in &contact.confirmed_tags {
        append_contact_fact(
            sources,
            "confirmed_customer_tag",
            "confirmed_tag",
            &tag.value,
            Some(tag.confirmed_at),
        );
    }
}

fn append_contact_fact(
    sources: &mut Vec<Value>,
    source_prefix: &str,
    source_type: &str,
    value: &str,
    observed_at: Option<DateTime>,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let identity_seed = format!("{source_prefix}:{}", value);
    let identity_hash = hex::encode(Sha256::digest(identity_seed.as_bytes()));
    sources.push(claim_source(
        format!("contact_fact:{identity_hash}"),
        "operator_contact_fact",
        "customer",
        "Authorizes only this exact operator-recorded customer-side fact. It does not establish a business capability, price, schedule, appointment confirmation, outcome, or professional conclusion.",
        true,
        &["customer"],
        false,
        json!({
            "text": bounded_text(value),
            "originSourceType": source_type,
            "observedAtMillis": observed_at.map(|value| value.timestamp_millis()),
        }),
    ));
}

fn append_memory_facts(sources: &mut Vec<Value>, memory: &OperatingMemory, evaluated_at: DateTime) {
    for fact in memory
        .memory_card
        .core_facts
        .iter()
        .chain(memory.memory_card.recent_facts.iter())
    {
        let MemoryFactRepr::Structured(fact) = fact else {
            continue;
        };
        let active = fact.status.as_deref() == Some("active")
            && fact
                .authority
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty() && value != "legacy_unverified")
            && fact.source_type.as_deref() != Some("legacy")
            && fact.valid_from.is_none_or(|value| value <= evaluated_at)
            && fact.valid_until.is_none_or(|value| value > evaluated_at);
        if !active || fact.id.trim().is_empty() {
            continue;
        }
        let subject = fact.subject.as_deref().unwrap_or("customer");
        sources.push(claim_source(
            format!("verified_memory_fact:{}", fact.id),
            "verified_memory_fact",
            subject,
            fact.authority.as_deref().unwrap_or(
                "May support only the exact fact recorded with its original provenance.",
            ),
            true,
            &[subject],
            false,
            json!({
                "text": bounded_text(&fact.text),
                "evidence": fact.evidence.as_deref().map(bounded_text),
                "originSourceType": fact.source_type,
                "sourceMessageIds": fact.source_message_ids.iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
                "validFromMillis": fact.valid_from.map(|value| value.timestamp_millis()),
                "validUntilMillis": fact.valid_until.map(|value| value.timestamp_millis()),
            }),
        ));
    }
}

fn append_observations(sources: &mut Vec<Value>, observations: &[AuthorityObservation]) {
    for observation in observations {
        let Some(id) = observation.id else { continue };
        sources.push(claim_source(
            format!("authority_observation:{}", id.to_hex()),
            "authority_observation",
            &observation.subject,
            &observation.authority_boundary,
            true,
            &[&observation.subject],
            observation.subject == "business",
            json!({
                "text": bounded_text(&observation.content),
                "originSourceType": observation.source_type,
                "originSourceId": observation.source_id,
                "validFromMillis": observation.valid_from.map(|value| value.timestamp_millis()),
                "validUntilMillis": observation.valid_until.map(|value| value.timestamp_millis()),
            }),
        ));
    }
}

fn append_world_state(sources: &mut Vec<Value>, state: Option<&PersonaWorldState>) {
    let Some(state) = state else { return };
    sources.push(claim_source(
        format!("persona_world_state:{}", state.version),
        "persona_world_state",
        "business",
        "Authorizes only consistent casual first-person context explicitly recorded in this current account-wide world state. It establishes no customer fact or business capability.",
        true,
        &["business", "general"],
        false,
        json!({
            "text": bounded_text(&state.state_text),
            "availability": state.availability,
            "mood": state.mood,
            "effectiveFromMillis": state.effective_from.timestamp_millis(),
            "effectiveUntilMillis": state.effective_until.timestamp_millis(),
            "version": state.version,
        }),
    ));
}

fn append_referral_overview(sources: &mut Vec<Value>, cards: &[ReferralCard]) {
    if cards.is_empty() {
        return;
    }
    sources.push(claim_source(
        "referral_catalog_overview".to_string(),
        "referral_catalog_overview",
        "third_party",
        "Generation guidance only. It does not authorize a claim or referral until the exact selected card is appended to the evidence ledger after server-side validation.",
        false,
        &[],
        false,
        json!({
            "cards": cards.iter().filter_map(|card| card.id.map(|id| json!({
                "cardId": id.to_hex(),
                "displayName": card.display_name,
                "sendTriggerHint": card.send_trigger_hint,
            }))).collect::<Vec<_>>()
        }),
    ));
}

fn claim_source(
    id: String,
    source_type: &str,
    subject: &str,
    authority_boundary: &str,
    authorizes_claims: bool,
    allowed_subjects: &[&str],
    allows_product_claims: bool,
    payload: Value,
) -> Value {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert("id".to_string(), Value::String(id));
    object.insert(
        "sourceType".to_string(),
        Value::String(source_type.to_string()),
    );
    object.insert("subject".to_string(), Value::String(subject.to_string()));
    object.insert(
        "authorityBoundary".to_string(),
        Value::String(authority_boundary.to_string()),
    );
    object.insert(
        "authorizesClaims".to_string(),
        Value::Bool(authorizes_claims),
    );
    object.insert(
        "allowedSubjects".to_string(),
        Value::Array(
            allowed_subjects
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
    object.insert(
        "allowsProductClaims".to_string(),
        Value::Bool(allows_product_claims),
    );
    Value::Object(object)
}

fn source_id(value: &Value) -> &str {
    value.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn stable_source_hash(sources: &[Value]) -> String {
    let mut ordered = sources.to_vec();
    ordered.sort_by(|left, right| source_id(left).cmp(source_id(right)));
    let bytes = serde_json::to_vec(&ordered).unwrap_or_default();
    hex::encode(Sha256::digest(bytes))
}

fn budget_sources(buckets: Vec<SourceBucket>) -> Vec<Value> {
    let mut selected = Vec::with_capacity(MAX_BASE_SOURCES);
    let mut seen = HashSet::new();
    for bucket in &buckets {
        for source in bucket.sources.iter().take(bucket.reserve) {
            push_unique_source(&mut selected, &mut seen, source);
            if selected.len() == MAX_BASE_SOURCES {
                return selected;
            }
        }
    }
    for bucket in &buckets {
        for source in &bucket.sources {
            push_unique_source(&mut selected, &mut seen, source);
            if selected.len() == MAX_BASE_SOURCES {
                return selected;
            }
        }
    }
    selected
}

fn push_unique_source(selected: &mut Vec<Value>, seen: &mut HashSet<String>, source: &Value) {
    let id = source_id(source);
    if !id.is_empty() && seen.insert(id.to_string()) {
        selected.push(source.clone());
    }
}

fn validate_persisted_snapshot(
    expected: &AuthoritySnapshot,
    persisted: &AgentTurnSnapshot,
) -> AppResult<()> {
    let identity_matches = persisted.bundle_hash == expected.bundle_hash
        && persisted.authority_version == AUTHORITY_VERSION
        && persisted.workspace_id == expected.workspace_id
        && persisted.account_id == expected.account_id
        && persisted.contact_wxid == expected.contact_wxid;
    if identity_matches {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "run/turn authority snapshot already exists with different authority".to_string(),
        ))
    }
}

fn render_prompt_payload(bundle_hash: &str, ledger_hash: &str, mut sources: Vec<Value>) -> String {
    sources.sort_by(|left, right| {
        prompt_source_priority(left)
            .cmp(&prompt_source_priority(right))
            .then_with(|| source_id(left).cmp(source_id(right)))
    });
    let required_ids = required_prompt_source_ids(&sources);
    let original_count = sources.len();
    for text_limit in [MAX_SOURCE_TEXT_CHARS, 2_000, 1_000, 500, 250, 120, 48] {
        let compacted = sources
            .iter()
            .map(|source| compact_prompt_source(source, text_limit))
            .collect::<Vec<_>>();
        let rendered = serialize_prompt_payload(
            bundle_hash,
            ledger_hash,
            compacted,
            original_count.saturating_sub(sources.len()),
            text_limit,
        );
        if rendered.chars().count() <= MAX_PROMPT_CHARS {
            return rendered;
        }
    }

    while let Some(index) = (0..sources.len())
        .rev()
        .find(|index| !required_ids.contains(source_id(&sources[*index])))
    {
        sources.remove(index);
        let compacted = sources
            .iter()
            .map(|source| compact_prompt_source(source, 48))
            .collect::<Vec<_>>();
        let rendered = serialize_prompt_payload(
            bundle_hash,
            ledger_hash,
            compacted,
            original_count.saturating_sub(sources.len()),
            48,
        );
        if rendered.chars().count() <= MAX_PROMPT_CHARS {
            return rendered;
        }
    }

    let rendered = serialize_prompt_payload(
        bundle_hash,
        ledger_hash,
        sources
            .iter()
            .map(|source| compact_prompt_source(source, 16))
            .collect(),
        original_count.saturating_sub(sources.len()),
        16,
    );
    if rendered.chars().count() <= MAX_PROMPT_CHARS {
        return rendered;
    }
    serialize_prompt_payload(
        bundle_hash,
        ledger_hash,
        sources.iter().map(minimal_prompt_source).collect(),
        original_count.saturating_sub(sources.len()),
        0,
    )
}

fn serialize_prompt_payload(
    bundle_hash: &str,
    ledger_hash: &str,
    sources: Vec<Value>,
    omitted_source_count: usize,
    source_text_limit_chars: usize,
) -> String {
    serde_json::to_string(&json!({
        "authorityVersion": AUTHORITY_VERSION,
        "bundleHash": bundle_hash,
        "evidenceLedgerHash": ledger_hash,
        "rules": [
            "Decide meaning autonomously from the complete conversation; never classify by a keyword list.",
            "Only a source with authorizesClaims=true may support a real-world claim, and only inside its authorityBoundary.",
            "Customer statements establish only customer-side facts. Historical assistant output is absent and is never evidence.",
            "Appointment requests are not confirmations. Only a confirmed appointment source may support a confirmed schedule.",
            "When support is missing, remove or narrow only the unsupported claim, ask one necessary clarification, or state that you will verify."
        ],
        "omittedSourceCount": omitted_source_count,
        "sourceTextLimitChars": source_text_limit_chars,
        "sources": sources,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn prompt_source_priority(source: &Value) -> u8 {
    match source.get("sourceType").and_then(Value::as_str) {
        Some("current_user_statement" | "principal_decision") => 0,
        Some("verified_knowledge" | "approved_referral_card") => 1,
        Some("published_soul" | "persona_world_state") => 2,
        Some(
            "contact_salutation" | "appointment" | "active_commitment" | "verified_memory_fact",
        ) => 3,
        Some("operator_contact_fact") => 3,
        Some("historical_user_statement") => 4,
        _ => 5,
    }
}

fn required_prompt_source_ids(sources: &[Value]) -> HashSet<String> {
    let mut required = HashSet::new();
    let mut protected_types = HashSet::new();
    for source in sources {
        let source_type = source
            .get("sourceType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let protect_every = matches!(source_type, "verified_knowledge" | "approved_referral_card");
        let protect_first = matches!(
            source_type,
            "current_user_statement"
                | "principal_decision"
                | "published_soul"
                | "persona_world_state"
                | "verified_memory_fact"
                | "operator_contact_fact"
        );
        if protect_every || (protect_first && protected_types.insert(source_type.to_string())) {
            let id = source_id(source);
            if !id.is_empty() {
                required.insert(id.to_string());
            }
        }
    }
    required
}

fn compact_prompt_source(source: &Value, text_limit: usize) -> Value {
    let mut compacted = source.clone();
    let Some(object) = compacted.as_object_mut() else {
        return compacted;
    };
    for key in [
        "text",
        "authorityBoundary",
        "usageGuidance",
        "requestText",
        "sendTriggerHint",
        "title",
        "displayName",
        "location",
        "availability",
        "mood",
    ] {
        if let Some(Value::String(value)) = object.get_mut(key) {
            *value = truncate_chars(value, text_limit);
        }
    }
    compacted
}

fn minimal_prompt_source(source: &Value) -> Value {
    json!({
        "id": source.get("id").cloned().unwrap_or(Value::Null),
        "sourceType": source.get("sourceType").cloned().unwrap_or(Value::Null),
        "subject": source.get("subject").cloned().unwrap_or(Value::Null),
        "authorizesClaims": source.get("authorizesClaims").cloned().unwrap_or(Value::Bool(false)),
        "allowedSubjects": source.get("allowedSubjects").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "allowsProductClaims": source.get("allowsProductClaims").cloned().unwrap_or(Value::Bool(false)),
        "authorityBoundary": source
            .get("authorityBoundary")
            .and_then(Value::as_str)
            .map(|value| truncate_chars(value, 16))
            .unwrap_or_default(),
        "text": source
            .get("text")
            .and_then(Value::as_str)
            .map(|value| truncate_chars(value, 16))
            .unwrap_or_default(),
    })
}

fn bounded_text(value: &str) -> String {
    truncate_chars(value, MAX_SOURCE_TEXT_CHARS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn values_to_documents(values: &[Value]) -> Vec<Document> {
    values
        .iter()
        .filter_map(|value| mongodb::bson::to_document(value).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_source(id: impl Into<String>, source_type: &str, text: &str) -> Value {
        claim_source(
            id.into(),
            source_type,
            "business",
            "bounded test authority",
            true,
            &["business"],
            false,
            json!({ "text": text }),
        )
    }

    #[test]
    fn authority_observation_writes_are_insert_only() {
        let update = observation_insert_update(doc! { "content": "原始事实" });
        assert!(update.get_document("$setOnInsert").is_ok());
        assert!(!update.contains_key("$set"));
    }

    #[test]
    fn observation_identity_reuse_detects_immutable_content_changes() {
        let expected = doc! {
            "workspace_id": "ws",
            "account_id": "account",
            "contact_wxid": "wxid",
            "source_type": "operator",
            "source_id": "source-1",
            "subject": "customer",
            "content": "原始事实",
            "authority_boundary": "exact fact",
            "status": "active",
        };
        let mut same = expected.clone();
        same.insert("updated_at", DateTime::now());
        assert!(observation_immutable_fields_match(&same, &expected));

        let mut changed = same;
        changed.insert("content", "被替换的事实");
        assert!(!observation_immutable_fields_match(&changed, &expected));
    }

    fn bucket(source_type: &str, prefix: &str, count: usize, reserve: usize) -> SourceBucket {
        SourceBucket {
            reserve,
            sources: (0..count)
                .map(|index| test_source(format!("{prefix}:{index}"), source_type, "value"))
                .collect(),
        }
    }

    #[test]
    fn evidence_ledger_is_append_only_by_source_identity() {
        let ledger = TurnEvidenceLedger::default();
        let first = claim_source(
            "source-1".into(),
            "test",
            "business",
            "exact only",
            true,
            &["business"],
            false,
            json!({ "text": "a" }),
        );
        assert!(ledger.append(first.clone()).unwrap());
        assert!(!ledger.append(first).unwrap());
        let changed = claim_source(
            "source-1".into(),
            "test",
            "business",
            "exact only",
            true,
            &["business"],
            false,
            json!({ "text": "b" }),
        );
        assert!(ledger.append(changed).is_err());
    }

    #[test]
    fn source_hash_does_not_depend_on_insertion_order() {
        let a = claim_source(
            "a".into(),
            "test",
            "general",
            "a",
            false,
            &[],
            false,
            json!({}),
        );
        let b = claim_source(
            "b".into(),
            "test",
            "general",
            "b",
            false,
            &[],
            false,
            json!({}),
        );
        assert_eq!(
            stable_source_hash(&[a.clone(), b.clone()]),
            stable_source_hash(&[b, a])
        );
    }

    #[test]
    fn soul_has_identity_authority_but_not_product_authority() {
        let mut sources = Vec::new();
        append_soul_source(&mut sources, "我是小星，今天在院里值班。 ");
        let soul = &sources[0];
        assert_eq!(soul["authorizesClaims"], true);
        assert_eq!(soul["allowsProductClaims"], false);
        assert!(soul["authorityBoundary"]
            .as_str()
            .unwrap()
            .contains("persona"));
    }

    #[test]
    fn operator_contact_fact_is_customer_scoped() {
        let mut sources = Vec::new();
        append_contact_fact(
            &mut sources,
            "operator_manual_tag",
            "operator_manual",
            "复诊客户",
            Some(DateTime::from_millis(7)),
        );
        let source = &sources[0];
        assert_eq!(source["sourceType"], "operator_contact_fact");
        assert_eq!(source["subject"], "customer");
        assert_eq!(source["authorizesClaims"], true);
        assert_eq!(source["allowsProductClaims"], false);
        assert_eq!(source["allowedSubjects"], json!(["customer"]));
        assert!(source["authorityBoundary"]
            .as_str()
            .is_some_and(|value| value.contains("does not establish a business capability")));
    }

    #[test]
    fn source_budget_preserves_every_reserved_provenance_class_under_saturation() {
        let selected = budget_sources(vec![
            bucket("current_user_statement", "current", 20, 2),
            SourceBucket {
                reserve: 3,
                sources: vec![
                    test_source("contact_salutation", "contact_salutation", "称呼"),
                    test_source("published_soul", "published_soul", "人格"),
                    test_source("persona_world_state:1", "persona_world_state", "日常"),
                ],
            },
            bucket("appointment", "lifecycle", 20, 12),
            bucket("verified_memory_fact", "memory", 20, 10),
            bucket("historical_user_statement", "history", 20, 10),
            bucket("approved_content_asset", "asset", 20, 12),
            bucket("authority_observation", "observation", 20, 8),
            bucket("active_product_catalog", "catalog", 20, 6),
            bucket("referral_catalog_overview", "referral", 20, 1),
        ]);

        assert_eq!(selected.len(), MAX_BASE_SOURCES);
        for required in [
            "current:0",
            "published_soul",
            "persona_world_state:1",
            "memory:0",
            "asset:0",
            "observation:0",
            "catalog:0",
            "referral:0",
        ] {
            assert!(selected.iter().any(|source| source_id(source) == required));
        }
    }

    #[test]
    fn saturated_prompt_render_is_bounded_valid_json_and_keeps_core_sources() {
        let large = "资料".repeat(MAX_SOURCE_TEXT_CHARS);
        let base_sources = budget_sources(vec![
            bucket("current_user_statement", "current", 20, 2),
            SourceBucket {
                reserve: 3,
                sources: vec![
                    test_source("published_soul", "published_soul", &large),
                    test_source("persona_world_state:1", "persona_world_state", &large),
                ],
            },
            bucket("verified_memory_fact", "memory", 20, 10),
            bucket("approved_content_asset", "asset", 80, 52),
        ]);
        let snapshot = AuthoritySnapshot {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: "wxid".to_string(),
            compiled_at: DateTime::from_millis(1),
            bundle_hash: stable_source_hash(&base_sources),
            base_sources: Arc::new(base_sources),
            ledger: TurnEvidenceLedger::default(),
        };
        for index in 0..MAX_LEDGER_SOURCES {
            snapshot
                .ledger()
                .append(test_source(
                    format!("verified_knowledge:{index}"),
                    "verified_knowledge",
                    &large,
                ))
                .unwrap();
        }

        let rendered = snapshot.render_for_prompt(PromptTier::Full);
        assert!(rendered.chars().count() <= MAX_PROMPT_CHARS);
        let parsed: Value = serde_json::from_str(&rendered).expect("authority JSON must parse");
        let sources = parsed["sources"].as_array().expect("sources array");
        for required in [
            "current:0",
            "published_soul",
            "persona_world_state:1",
            "memory:0",
            "verified_knowledge:0",
        ] {
            assert!(sources.iter().any(|source| source_id(source) == required));
        }
    }

    #[test]
    fn saturated_prompt_render_keeps_the_first_operator_contact_fact() {
        let large = "资料".repeat(MAX_SOURCE_TEXT_CHARS);
        let sources = vec![
            claim_source(
                "current:0".to_string(),
                "current_user_statement",
                "customer",
                "customer statement",
                true,
                &["customer"],
                false,
                json!({ "text": large }),
            ),
            claim_source(
                "operator:0".to_string(),
                "operator_contact_fact",
                "customer",
                "exact operator-recorded customer fact",
                true,
                &["customer"],
                false,
                json!({ "text": large }),
            ),
        ]
        .into_iter()
        .chain((0..MAX_LEDGER_SOURCES).map(|index| {
            claim_source(
                format!("verified_knowledge:{index}"),
                "verified_knowledge",
                "business",
                "verified knowledge",
                true,
                &["business"],
                true,
                json!({ "text": large }),
            )
        }))
        .collect::<Vec<_>>();

        let rendered = render_prompt_payload("bundle", "ledger", sources);
        let parsed: Value = serde_json::from_str(&rendered).expect("authority JSON must parse");
        let rendered_sources = parsed["sources"].as_array().expect("sources array");
        assert!(rendered_sources
            .iter()
            .any(|source| source_id(source) == "operator:0"));
    }

    #[test]
    fn persisted_snapshot_with_different_bundle_is_a_conflict() {
        let expected = AuthoritySnapshot {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: "wxid".to_string(),
            compiled_at: DateTime::from_millis(1),
            bundle_hash: "expected".to_string(),
            base_sources: Arc::new(Vec::new()),
            ledger: TurnEvidenceLedger::default(),
        };
        let persisted = AgentTurnSnapshot {
            id: None,
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: "wxid".to_string(),
            authority_version: AUTHORITY_VERSION,
            bundle_hash: "different".to_string(),
            sources: Vec::new(),
            evidence_ledger: Vec::new(),
            loop_trace: Vec::new(),
            authorization_manifest: None,
            commit_receipt: None,
            created_at: DateTime::from_millis(1),
            expires_at: DateTime::from_millis(2),
        };

        assert!(validate_persisted_snapshot(&expected, &persisted).is_err());
    }

    #[test]
    fn authority_catalog_honors_asset_tier_without_hiding_other_evidence() {
        let sources = vec![
            claim_source(
                "asset:lean".to_string(),
                "approved_content_asset",
                "business",
                "asset boundary",
                true,
                &["business"],
                true,
                json!({
                    "minInjectTier": "lean",
                    "allowedInsertionLevels": ["subtle"],
                    "text": "lean asset"
                }),
            ),
            claim_source(
                "asset:full".to_string(),
                "approved_content_asset",
                "business",
                "asset boundary",
                true,
                &["business"],
                true,
                json!({
                    "minInjectTier": "full",
                    "allowedInsertionLevels": ["direct"],
                    "text": "full asset"
                }),
            ),
            claim_source(
                "current".to_string(),
                "current_user_statement",
                "customer",
                "customer boundary",
                true,
                &["customer"],
                false,
                json!({ "text": "hello" }),
            ),
        ];
        let snapshot = AuthoritySnapshot {
            run_id: "run".to_string(),
            turn_id: "turn".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: "wxid".to_string(),
            compiled_at: DateTime::from_millis(1),
            bundle_hash: stable_source_hash(&sources),
            base_sources: Arc::new(sources),
            ledger: TurnEvidenceLedger::default(),
        };
        let lean_sources = snapshot.evidence_catalog_for_tier(PromptTier::Lean);
        let lean_ids = lean_sources.iter().map(source_id).collect::<Vec<_>>();
        assert!(lean_ids.contains(&"asset:lean"));
        assert!(!lean_ids.contains(&"asset:full"));
        assert!(lean_ids.contains(&"current"));

        let full_sources = snapshot.evidence_catalog_for_tier(PromptTier::Full);
        let full_ids = full_sources.iter().map(source_id).collect::<Vec<_>>();
        assert!(full_ids.contains(&"asset:lean"));
        assert!(full_ids.contains(&"asset:full"));

        // The projection is per call, not shared mutable state. Rendering Full must not change
        // the result of a subsequent Lean projection.
        let lean_again = snapshot.evidence_catalog_for_tier(PromptTier::Lean);
        assert!(!lean_again
            .iter()
            .any(|source| source_id(source) == "asset:full"));
    }
}
