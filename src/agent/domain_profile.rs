//! universal-domain-adaptation Phase 0：行业「总装配单」的内置默认值 + 加载器。
//!
//! 设计见 `docs/superpowers/specs/2026-06-11-universal-domain-adaptation-design.md`。
//!
//! **本模块在 Phase 0 仅提供存储读取 + 内置 DEFAULT_PROFILE 兜底；运行时各消费点
//! （decision_taxonomy / prompts / guards / catalog completeness）尚未接线**——这是
//! 刻意的：Phase 0 必须零行为变化，仅把「加载 active profile」的管道铺好，消费解耦
//! 留 Phase 1。
//!
//! `#![allow(dead_code)]`：Phase 0 落地存储 + 加载器但运行时尚未消费，公开项暂时
//! 无调用方。Phase 1 接线后**移除本 allow**，由编译器确保每个导出项都被真实消费。
#![allow(dead_code)]
//!
//! ## DEFAULT_PROFILE 的角色（关键安全网）
//!
//! 系统出厂对行业零假设，但**旧库 / 全新部署 / 未配置**时 `domain_profiles` 为空。
//! 此时 [`load_active_domain_profile`] 返回 [`default_domain_profile`]，其内容**逐字
//! 等价于当前写死在源码里的销售域行为**：
//!
//! - 画像维度 = `customer_stage` / `intent_level`（对齐 `decision_taxonomy::TAGGED_FIELDS`）；
//! - 承诺词表 = `guards::commitment_claim_class` 的 5 + 3 词（逐字复刻）；
//! - completeness 维度 = `catalog.rs` 的五维 coverage（逐字复刻）。
//!
//! 这保证 Phase 1 把消费点切到 profile 后，DEFAULT_PROFILE 下的所有现有 PBT /
//! real-LLM 套件**逐条等价**——这是反过拟合的硬护栏：换行业只是「另一份 profile」，
//! 不改任何通用逻辑。

use mongodb::bson::doc;

use crate::db::Database;
use crate::models::{
    CommitmentMarkers, CoverageDimension, DomainProfile, ProfileDimension,
};

/// 内置默认 profile 的 `profile_id`。运行时无 active profile 时使用。
pub const DEFAULT_PROFILE_ID: &str = "__default__";

/// 构造内置 DEFAULT_PROFILE。内容逐字等价当前源码写死的销售域行为。
///
/// 注意：这里复刻的常量与以下源码点**必须保持同步**，Phase 1 切换消费点后由
/// 等价性测试锁死：
/// - `src/agent/decision_taxonomy.rs::TAGGED_FIELDS`（customer_stage / intent_level）
/// - `src/agent/guards.rs::commitment_claim_class`（product_effect / tone_only 词表）
/// - `src/routes/knowledge/catalog.rs`（五维 coverage）
pub fn default_domain_profile(workspace_id: &str) -> DomainProfile {
    let now = mongodb::bson::DateTime::now();
    DomainProfile {
        id: None,
        profile_id: DEFAULT_PROFILE_ID.to_string(),
        workspace_id: workspace_id.to_string(),
        display_name: "默认运营画像（通用兜底）".to_string(),
        description: "系统内置兜底配置：未配置行业 profile 时使用，行为等价历史默认。\
                      通过「行业配置向导」与 AI 对话生成专属 profile 后，此兜底不再生效。"
            .to_string(),
        profile_dimensions: vec![
            ProfileDimension {
                kind: "customer_stage".to_string(),
                display_name: "客户阶段".to_string(),
                participates_in_decision: true,
                description: "客户在运营关系中所处阶段。".to_string(),
            },
            ProfileDimension {
                kind: "intent_level".to_string(),
                display_name: "意向程度".to_string(),
                participates_in_decision: true,
                description: "客户当前的意向高低。".to_string(),
            },
        ],
        domain_schema_id: None,
        prompt_fragment: None,
        commitment_markers: CommitmentMarkers {
            // 逐字复刻 guards.rs::commitment_claim_class
            product_effect: vec![
                "成功率".to_string(),
                "见效".to_string(),
                "回款".to_string(),
                "百分之".to_string(),
                "百分百".to_string(),
            ],
            tone_only: vec![
                "保证".to_string(),
                "一定能".to_string(),
                "绝对".to_string(),
            ],
        },
        coverage_dimensions: vec![
            // 逐字复刻 catalog.rs 五维
            CoverageDimension { key: "capability".to_string(), display_name: "能力".to_string(), required: false },
            CoverageDimension { key: "pricing".to_string(), display_name: "报价".to_string(), required: false },
            CoverageDimension { key: "caseEvidence".to_string(), display_name: "案例证据".to_string(), required: false },
            CoverageDimension { key: "effectClaims".to_string(), display_name: "效果声明".to_string(), required: false },
            CoverageDimension { key: "deliveryBoundary".to_string(), display_name: "交付边界".to_string(), required: false },
        ],
        // 逐字复刻 planner 写死的停滞计时维度（customer_stage）。
        stagnation_dimension: Some("customer_stage".to_string()),
        // 逐字复刻 agent::types::CONVERSATION_MODE_VALUES 的四模式（H9 DEFAULT 等价）。
        conversation_modes: vec![
            "casual_relationship".to_string(),
            "value_exchange".to_string(),
            "consultative".to_string(),
            "boundary_protection".to_string(),
        ],
        // H8：DEFAULT 范式 = 三驱动力全开 + 阈值 None 回落全局 config（planner 金标零变化）。
        operation_mode: crate::models::OperationMode::default(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("default".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

/// 加载某 workspace 当前生效的 DomainProfile。
///
/// 查 `is_active=true` 一条；无则 fallback 到 [`default_domain_profile`]。
/// DB 错误也 fallback（不阻塞运行时；与 taxonomy cache warm_up 失败静默同精神）。
///
/// Phase 0：本函数已可调用，但运行时各消费点尚未接线——仅供 Phase 1 切换时使用 +
/// 引导层落库后的读取验证。
pub async fn load_active_domain_profile(db: &Database, workspace_id: &str) -> DomainProfile {
    match db
        .domain_profiles()
        .find_one(
            doc! { "workspace_id": workspace_id, "is_active": true, "current_version": true },
            None,
        )
        .await
    {
        Ok(Some(profile)) => profile,
        Ok(None) => default_domain_profile(workspace_id),
        Err(error) => {
            tracing::warn!(
                ?error,
                workspace_id,
                "load_active_domain_profile failed; falling back to DEFAULT_PROFILE"
            );
            default_domain_profile(workspace_id)
        }
    }
}

/// 取「参与决策」的维度 kind 列表（对应旧 `TAGGED_FIELDS` 成员集合）。
/// Phase 1 由 `decision_taxonomy` 消费以替换 const 表。
pub fn decision_dimension_kinds(profile: &DomainProfile) -> Vec<String> {
    profile
        .profile_dimensions
        .iter()
        .filter(|d| d.participates_in_decision)
        .map(|d| d.kind.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_has_sales_domain_dimensions() {
        let p = default_domain_profile("ws-1");
        let kinds = decision_dimension_kinds(&p);
        assert_eq!(kinds, vec!["customer_stage", "intent_level"]);
        assert!(p.is_active && p.current_version);
        assert_eq!(p.profile_id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn default_profile_commitment_markers_match_guards_verbatim() {
        // 逐字等价护栏：与 guards.rs::commitment_claim_class 的两组词表一致。
        let p = default_domain_profile("ws-1");
        assert_eq!(
            p.commitment_markers.product_effect,
            vec!["成功率", "见效", "回款", "百分之", "百分百"]
        );
        assert_eq!(
            p.commitment_markers.tone_only,
            vec!["保证", "一定能", "绝对"]
        );
    }

    #[test]
    fn default_profile_coverage_matches_catalog_five_dims() {
        let p = default_domain_profile("ws-1");
        let keys: Vec<&str> = p.coverage_dimensions.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["capability", "pricing", "caseEvidence", "effectClaims", "deliveryBoundary"]
        );
    }

    #[test]
    fn default_profile_conversation_modes_match_const_verbatim() {
        // H9 逐字等价护栏：DEFAULT_PROFILE 声明的四模式与 types::CONVERSATION_MODE_VALUES
        // 一致，保证 1E 把校验切到 profile 后销售域行为不变。
        let p = default_domain_profile("ws-1");
        assert_eq!(
            p.conversation_modes,
            vec![
                "casual_relationship",
                "value_exchange",
                "consultative",
                "boundary_protection"
            ]
        );
    }

    #[test]
    fn default_profile_operation_mode_is_all_enabled_default() {
        // H8 逐字等价护栏：DEFAULT_PROFILE 的范式 = OperationMode::default()
        // （三驱动力全开 + 阈值 None 回落全局 config），保证 1F 切 planner 后金标零变化。
        let p = default_domain_profile("ws-1");
        assert_eq!(p.operation_mode, crate::models::OperationMode::default());
        assert!(p.operation_mode.funnel.enabled);
        assert!(p.operation_mode.silence.enabled);
        assert!(p.operation_mode.commitment.enabled);
    }

    #[test]
    fn default_profile_bson_round_trip() {
        let p = default_domain_profile("ws-1");
        let doc = mongodb::bson::to_document(&p).expect("serialize");
        let parsed: DomainProfile = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed.profile_id, p.profile_id);
        assert_eq!(parsed.profile_dimensions.len(), 2);
        assert_eq!(parsed.commitment_markers.product_effect.len(), 5);
    }
}
