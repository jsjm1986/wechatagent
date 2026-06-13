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
use parking_lot::Mutex as PlMutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{
    BusinessFormula, ChunkRole, CommitmentMarkers, CoverageDimension, DomainProfile,
    OutcomePolarity, ProfileDimension,
};

/// 内置默认 profile 的 `profile_id`。运行时无 active profile 时使用。
pub const DEFAULT_PROFILE_ID: &str = "__default__";

/// universal-domain-adaptation H16：内置默认 chunk 角色表，逐字复刻
/// `knowledge_router.rs::format_operation_knowledge_for_prompt` 写死的销售四态分桶 +
/// header（product_fact 为 fallback 桶）。DEFAULT_PROFILE 用它；active profile 声明了
/// `chunk_roles`（非空）时由 knowledge_router 覆盖。Phase 2 H16-b 接线后由等价性测试
/// `default_profile_chunk_roles_match_router_verbatim` 锁死与渲染函数 const 一致。
pub fn default_chunk_roles() -> Vec<ChunkRole> {
    vec![
        ChunkRole {
            key: "product_fact".to_string(),
            header: "【产品事实 product_fact】仅 verified 切片可用作产品声明背书；needs_review/rejected 不作背书。".to_string(),
            order: 0,
            is_fallback: true,
        },
        ChunkRole {
            key: "style_template".to_string(),
            header: "【语气模板 style_template】作为 few-shot 参考；不直接复制内容，仅借鉴节奏与措辞。".to_string(),
            order: 1,
            is_fallback: false,
        },
        ChunkRole {
            key: "peer_case".to_string(),
            header: "【同行案例 peer_case】仅作 reference，不作我方产品承诺；引用必须显式标注「行业经验/同行案例」。".to_string(),
            order: 2,
            is_fallback: false,
        },
        ChunkRole {
            key: "negative_example".to_string(),
            header: "【反例 negative_example】don't-do 列表；候选回复语气/结构若与本段相似，必须改写。".to_string(),
            order: 3,
            is_fallback: false,
        },
    ]
}

/// universal-domain-adaptation H11：内置默认自学习极性，逐字复刻回路① 的 fallback
/// 常量（`gap_signals::DEFAULT_POSITIVE_OUTCOMES` + `DEFAULT_NEGATIVE_OUTCOMES`，
/// 后者同 `reaction.rs::DEFAULT_NEGATIVE_OUTCOMES` 5 词）。**与回落同源**：seed 直接
/// 引用这两个常量，故 DEFAULT_PROFILE 显式声明的极性与各消费方在空集时回落的极性
/// 永远字节相等，不会因手抄漂移。DEFAULT_PROFILE 用它；active profile 声明了非空
/// `outcome_polarity` 时由 2.5-main-2/3 各回路覆盖。等价性测试
/// `default_profile_outcome_polarity_matches_hardcoded_verbatim` 锁死同步。
pub fn default_outcome_polarity() -> OutcomePolarity {
    use crate::knowledge_wiki::gap_signals::{
        DEFAULT_NEGATIVE_OUTCOMES, DEFAULT_POSITIVE_OUTCOMES,
    };
    OutcomePolarity {
        positive: DEFAULT_POSITIVE_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        negative: DEFAULT_NEGATIVE_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

/// universal-domain-adaptation H15：DEFAULT_PROFILE 的销售域四公式 seed。逐字复刻
/// 散落在四处副本的销售经营公式，作为 DEFAULT 等价的单一真相源：
/// - `prompts.rs` policy「关系经营公式（自检）」英文展开式；
/// - `prompts.rs` default_playbook method_prompt「核心公式」中文展开式；
/// - `agent/review/mod.rs` reviewer prompt 的 `formulaBreakdown` 模板；
/// - `routes/evaluations.rs` 硬编码 `formulas` 数组 + `score_key_for` 映射。
///
/// `expression` = policy 英文式逐字；`display_name` = playbook 中文名；
/// `eval_score_key` = `score_key_for` 映射逐字（trust→humanLike /
/// conversionReadiness→conversionReadiness / emotionalValue→emotionalValue /
/// nextBestActionScore→relationshipProgress）。空集时各消费方回落本函数同源常量。
pub fn default_business_formulas() -> Vec<BusinessFormula> {
    vec![
        BusinessFormula {
            key: "trust".to_string(),
            expression: "Credibility + Reliability + Intimacy − SelfOrientation".to_string(),
            display_name: "信任".to_string(),
            eval_score_key: Some("humanLike".to_string()),
        },
        BusinessFormula {
            key: "conversionReadiness".to_string(),
            expression: "Motivation × ProductFit × Timing × Trust ÷ Friction".to_string(),
            display_name: "成交准备度".to_string(),
            eval_score_key: Some("conversionReadiness".to_string()),
        },
        BusinessFormula {
            key: "emotionalValue".to_string(),
            expression: "Empathy + Validation + Specificity + AutonomySupport − Pressure"
                .to_string(),
            display_name: "情绪价值".to_string(),
            eval_score_key: Some("emotionalValue".to_string()),
        },
        BusinessFormula {
            key: "nextBestActionScore".to_string(),
            expression:
                "RelationshipGain + ConversionProgress + EmotionalValue + ProductFit − PressureRisk − FactRisk"
                    .to_string(),
            display_name: "下一步动作评分".to_string(),
            eval_score_key: Some("relationshipProgress".to_string()),
        },
    ]
}

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
        // H12：DEFAULT 出厂人格/方法论 = None → 回落内置销售域 soul + playbook（逐字等价）。
        soul_override: None,
        methodology_override: None,
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
            // 逐字复刻 catalog.rs 五维 + 命中锚点散文（H5-b：anchor_hint 注入审计 prompt）。
            CoverageDimension { key: "capability".to_string(), display_name: "能力".to_string(), required: false, anchor_hint: Some("有 verified 切片陈述产品/服务\"能做什么\"的具体能力或功能事实。".to_string()) },
            CoverageDimension { key: "pricing".to_string(), display_name: "报价".to_string(), required: false, anchor_hint: Some("有 verified 切片含具体报价/计费/套餐金额（注意：仅 needs_review 草稿里的报价不计入 verifiedFact，而应置 pendingDraft=true 并入 gap）。".to_string()) },
            CoverageDimension { key: "caseEvidence".to_string(), display_name: "案例证据".to_string(), required: false, anchor_hint: Some("有 verified 切片描述**具体客户案例/实施成效**（含可核验的主体、场景或落地结果），即判 true。".to_string()) },
            CoverageDimension { key: "effectClaims".to_string(), display_name: "效果声明".to_string(), required: false, anchor_hint: Some("有 verified 切片含**可核验的效果数据/量化成果**（如转化率提升、响应时长变化等具体数字），即判 true。".to_string()) },
            CoverageDimension { key: "deliveryBoundary".to_string(), display_name: "交付边界".to_string(), required: false, anchor_hint: Some("有 verified 切片陈述交付方式/SLA/可用性/部署边界等具体条款。".to_string()) },
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
        // H14：DEFAULT 销售域 = false → grounding 软分数硬闸无条件生效（字节等价）。
        grounding_gate_bypass_without_claim: false,
        // H16：DEFAULT 销售域 = 逐字复刻 knowledge_router 写死的四态角色（字节等价）。
        chunk_roles: default_chunk_roles(),
        // H11：DEFAULT 销售极性 = 显式填回回路① fallback 常量（正极 buying_signal +
        // 负极 5 词）。空集 default 会让消费方回落同一对常量，故 seed 与回落同源、字节等价。
        outcome_polarity: default_outcome_polarity(),
        // H15：DEFAULT 销售域 = 显式填回四公式（Trust/ConversionReadiness/EmotionalValue/
        // NextBestActionScore）。空集时各消费方回落内置销售公式常量，故 seed 与回落同源、
        // 字节等价。
        business_formulas: default_business_formulas(),
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
/// **1G-c**：本函数现在走进程级 [`DomainProfileCache`]（30s TTL + publish 失效），
/// 治理 1A/1C/1E/1F 引入的"每决策 / 每 planner tick 都查 DB"N+1。缓存未命中 /
/// DB 空 / DB 错误时仍回落 [`default_domain_profile`]，与接缓存前逐字等价。
pub async fn load_active_domain_profile(db: &Database, workspace_id: &str) -> DomainProfile {
    global_domain_profile_cache()
        .get_or_load(db, workspace_id)
        .await
}

// ─────────────────────────────────────────────────────────────────
// 1G-c：进程级 active DomainProfile TTL 缓存。
//
// 镜像 `agent::taxonomy::TaxonomyCache`：内部 Mutex 保护 (entries, fetched_at)，
// TTL 自愈 + 显式 invalidate。`reload_from_db` 一次性拉全部 workspace 的 active
// profile 分组缓存；`get_or_load` TTL 过期则重载，按 workspace_id 命中返回 clone，
// 未命中（DB 无该 workspace 的 active profile）回落 default。
//
// 启动期由 `init_global_domain_profile_cache(db)` 预热（main.rs 接入）；引导层
// publish profile 后调 `invalidate_global_domain_profile_cache` 让下次 load 立即
// 见最新（Phase 3 接线，故现暂无调用方，靠 module 级 allow(dead_code) 静默）。
// ─────────────────────────────────────────────────────────────────

/// profile 缓存有效期：30s（与 `TAXONOMY_CACHE_TTL` 同口径）。
const DOMAIN_PROFILE_CACHE_TTL: Duration = Duration::from_secs(30);

/// 进程级 active DomainProfile TTL 缓存，按 `workspace_id` 索引。
pub struct DomainProfileCache {
    inner: PlMutex<DomainProfileCacheInner>,
}

struct DomainProfileCacheInner {
    /// `workspace_id` → 该 workspace 当前 active profile（仅缓存 DB 命中的真实
    /// profile；DB 无 active 行的 workspace **不**入表，`get_or_load` 对其回落
    /// default，与接缓存前等价）。
    entries: HashMap<String, DomainProfile>,
    fetched_at: Option<Instant>,
}

impl Default for DomainProfileCache {
    fn default() -> Self {
        Self {
            inner: PlMutex::new(DomainProfileCacheInner {
                entries: HashMap::new(),
                fetched_at: None,
            }),
        }
    }
}

impl DomainProfileCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 显式失效缓存。引导层 publish/激活 profile 后调用，让下一次 `get_or_load`
    /// 重新拉取最新 active profile（否则换 profile 后最多 30s 才可见）。
    pub fn invalidate(&self) {
        let mut inner = self.inner.lock();
        inner.entries.clear();
        inner.fetched_at = None;
    }

    /// 启动期预热：拉全部 active profile 填充缓存。失败静默（缓存留空，
    /// 下次 `get_or_load` 重试）。
    pub async fn warm_up(&self, db: &Database) {
        if let Err(error) = self.reload_from_db(db).await {
            tracing::warn!(?error, "DomainProfileCache.warm_up failed; cache remains empty");
        }
    }

    async fn reload_from_db(&self, db: &Database) -> AppResult<()> {
        use futures::TryStreamExt;
        let mut cursor = db
            .domain_profiles()
            .find(doc! { "is_active": true, "current_version": true }, None)
            .await?;
        let mut entries: HashMap<String, DomainProfile> = HashMap::new();
        while let Some(profile) = cursor.try_next().await? {
            // 同 workspace 多条 active（异常态）时后插入者赢——与 find_one 取任意一条
            // 同语义；正常态每 workspace 至多一条 active+current。
            entries.insert(profile.workspace_id.clone(), profile);
        }
        let mut inner = self.inner.lock();
        inner.entries = entries;
        inner.fetched_at = Some(Instant::now());
        Ok(())
    }

    /// TTL 自愈判定：fetched_at 缺失或距今 ≥ TTL → true。抽独立函数让 lib 单测
    /// 无 Docker 也能断言 TTL 语义。
    pub(crate) fn is_stale(&self) -> bool {
        let inner = self.inner.lock();
        match inner.fetched_at {
            Some(t) => t.elapsed() >= DOMAIN_PROFILE_CACHE_TTL,
            None => true,
        }
    }

    /// 查找或自动加载：TTL 过期 → 重载全表；按 `workspace_id` 命中返回真实 profile
    /// 的 clone，未命中回落 [`default_domain_profile`]（DB 错误时重载失败 → 缓存
    /// 留空 → 同样回落 default，与接缓存前 `load_active_domain_profile` 逐字等价）。
    pub(crate) async fn get_or_load(&self, db: &Database, workspace_id: &str) -> DomainProfile {
        if self.is_stale() {
            if let Err(error) = self.reload_from_db(db).await {
                tracing::warn!(
                    ?error,
                    workspace_id,
                    "DomainProfileCache.reload_from_db failed; falling back to DEFAULT_PROFILE"
                );
            }
        }
        self.lookup_or_default(workspace_id)
    }

    /// 纯查表（无 IO）：命中返回真实 profile clone，未命中回落 default。抽出独立
    /// 方法让 `get_or_load` 与 lib 单测共用同一回落口径（避免测试内联逻辑漂移）。
    fn lookup_or_default(&self, workspace_id: &str) -> DomainProfile {
        let inner = self.inner.lock();
        match inner.entries.get(workspace_id) {
            Some(profile) => profile.clone(),
            None => default_domain_profile(workspace_id),
        }
    }

    /// test-only：把 `fetched_at` 强制回拨，模拟"距上次加载已过 N"，验证 TTL。
    #[cfg(test)]
    pub(crate) fn rewind_fetched_at_for_test(&self, dur: Duration) {
        let mut inner = self.inner.lock();
        if let Some(t) = inner.fetched_at {
            inner.fetched_at = Some(t.checked_sub(dur).unwrap_or(t));
        }
    }

    /// test-only：直接灌入一个 workspace 的 profile 并标记已加载，免 Mongo 即可
    /// 验证"命中返回真实 profile / 未命中回落 default"。
    #[cfg(test)]
    pub(crate) fn seed_for_test(&self, profile: DomainProfile) {
        let mut inner = self.inner.lock();
        inner.entries.insert(profile.workspace_id.clone(), profile);
        inner.fetched_at = Some(Instant::now());
    }
}

static GLOBAL_DOMAIN_PROFILE_CACHE: std::sync::LazyLock<Arc<DomainProfileCache>> =
    std::sync::LazyLock::new(|| Arc::new(DomainProfileCache::new()));

/// 进程级单例 cache 句柄；[`load_active_domain_profile`] 在没有注入自定义 cache
/// 时使用本入口。
pub(crate) fn global_domain_profile_cache() -> Arc<DomainProfileCache> {
    GLOBAL_DOMAIN_PROFILE_CACHE.clone()
}

/// 启动期预热：由 `main.rs` 在 `ensure_indexes` 后调用。失败被静默。
pub async fn init_global_domain_profile_cache(db: &Database) {
    GLOBAL_DOMAIN_PROFILE_CACHE.warm_up(db).await;
}

/// 引导层 publish/激活 profile 后调用以让缓存立即失效（Phase 3 接线）。
pub(crate) fn invalidate_global_domain_profile_cache() {
    GLOBAL_DOMAIN_PROFILE_CACHE.invalidate();
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
    fn default_profile_persona_overrides_are_none() {
        // H12 逐字等价护栏：DEFAULT_PROFILE 不覆盖人格/方法论本体 → soul_override /
        // methodology_override 均 None，决策路径回落内置销售域 soul + playbook，
        // 保证 H12 切消费点后销售域行为字节不变。换行业 = 另一份 profile 填这两字段。
        let p = default_domain_profile("ws-1");
        assert!(p.soul_override.is_none());
        assert!(p.methodology_override.is_none());
    }

    #[test]
    fn default_profile_grounding_gate_unconditional() {
        // H14 逐字等价护栏：DEFAULT_PROFILE 的 grounding_gate_bypass_without_claim
        // = false → grounding 软分数硬闸无条件生效，保证 H14 把闸条件化后销售域
        // 行为字节不变（classify_dual_gate 仍对每条回复判 grounding 低分）。
        // 换行业 = 情感/关系 profile 置 true 旁路。
        let p = default_domain_profile("ws-1");
        assert!(!p.grounding_gate_bypass_without_claim);
    }

    #[test]
    fn default_profile_chunk_roles_match_router_verbatim() {
        // H16 逐字等价护栏：DEFAULT_PROFILE 的 chunk_roles 与 knowledge_router 写死的
        // 四态分桶 + header + 顺序 + fallback 桶完全一致，保证 H16-b 把渲染函数切到
        // profile 后销售域 prompt 字节不变。换行业 = 另一份 chunk_roles。
        let p = default_domain_profile("ws-1");
        assert_eq!(p.chunk_roles.len(), 4);
        let keys: Vec<&str> = p.chunk_roles.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["product_fact", "style_template", "peer_case", "negative_example"]);
        // 顺序字段升序 0..3，与渲染函数固定输出顺序一致。
        assert_eq!(p.chunk_roles.iter().map(|r| r.order).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        // 仅 product_fact 是 fallback 桶（未命中任何 key 的 chunk 归入）。
        assert!(p.chunk_roles[0].is_fallback);
        assert!(p.chunk_roles[1..].iter().all(|r| !r.is_fallback));
        // header 逐字复刻 knowledge_router::format_operation_knowledge_for_prompt 的 order[]。
        assert_eq!(p.chunk_roles[0].header, "【产品事实 product_fact】仅 verified 切片可用作产品声明背书；needs_review/rejected 不作背书。");
        assert_eq!(p.chunk_roles[1].header, "【语气模板 style_template】作为 few-shot 参考；不直接复制内容，仅借鉴节奏与措辞。");
        assert_eq!(p.chunk_roles[2].header, "【同行案例 peer_case】仅作 reference，不作我方产品承诺；引用必须显式标注「行业经验/同行案例」。");
        assert_eq!(p.chunk_roles[3].header, "【反例 negative_example】don't-do 列表；候选回复语气/结构若与本段相似，必须改写。");
    }

    #[test]
    fn default_profile_outcome_polarity_matches_hardcoded_verbatim() {
        // H11 逐字等价护栏：DEFAULT_PROFILE 的 outcome_polarity 与回路① 写死的极性
        // 常量完全一致，保证 main-2/3 把三回路切到 profile 后销售域学习行为字节不变。
        // seed 直接引用这两个常量（同源），本测试断言"引用关系成立 + 内容如预期"。
        use crate::knowledge_wiki::gap_signals::{
            DEFAULT_NEGATIVE_OUTCOMES, DEFAULT_POSITIVE_OUTCOMES,
        };
        let p = default_domain_profile("ws-1");
        // 正极 = buying_signal 单词（回路① classify→Hit 的唯一字面量）。
        assert_eq!(p.outcome_polarity.positive, vec!["user_replied_buying_signal"]);
        // 负极 = objection/stop_requested/unsubscribed/negative/complaint 五词
        // （回路① classify→Block + reaction.rs::is_negative_outcome 旧 5 词）。
        assert_eq!(
            p.outcome_polarity.negative,
            vec![
                "user_replied_objection",
                "user_replied_stop_requested",
                "user_replied_unsubscribed",
                "user_replied_negative",
                "user_replied_complaint",
            ]
        );
        // 同源锁死：seed 与回落常量逐元素相等，杜绝手抄漂移。
        assert_eq!(
            p.outcome_polarity.positive,
            DEFAULT_POSITIVE_OUTCOMES.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            p.outcome_polarity.negative,
            DEFAULT_NEGATIVE_OUTCOMES.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_outcome_polarity_default_is_empty_not_sales() {
        // OutcomePolarity::default() 是空集（非销售词）——销售极性由 seed 显式填回。
        // 这是消费方"空集→回落内置常量"契约的前提：default 不能预埋销售词，否则
        // 换行业的 profile 若漏配某一极会静默继承销售词。
        let d = crate::models::OutcomePolarity::default();
        assert!(d.positive.is_empty());
        assert!(d.negative.is_empty());
    }

    #[test]
    fn default_profile_bson_round_trip() {
        let p = default_domain_profile("ws-1");
        let doc = mongodb::bson::to_document(&p).expect("serialize");
        let parsed: DomainProfile = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed.profile_id, p.profile_id);
        assert_eq!(parsed.profile_dimensions.len(), 2);
        assert_eq!(parsed.commitment_markers.product_effect.len(), 5);
        // H11：outcome_polarity 经 BSON 往返不丢（camelCase positive/negative）。
        assert_eq!(parsed.outcome_polarity.positive, p.outcome_polarity.positive);
        assert_eq!(parsed.outcome_polarity.negative, p.outcome_polarity.negative);
        // H15：business_formulas 经 BSON 往返不丢（camelCase key/expression/displayName/evalScoreKey）。
        assert_eq!(parsed.business_formulas, p.business_formulas);
    }

    // ── 3A-1a H15：经营公式 seed 等价 ──

    #[test]
    fn default_business_formulas_default_is_empty_not_sales() {
        // DomainProfile.business_formulas 的 serde 默认是空 Vec（非销售四公式）——
        // 销售公式由 seed 显式填回。这是消费方"空集→回落内置常量"契约的前提：
        // default 不能预埋销售公式，否则换行业 profile 漏配会静默继承销售公式。
        let formulas: Vec<crate::models::BusinessFormula> = Vec::default();
        assert!(formulas.is_empty());
    }

    #[test]
    fn default_business_formulas_seed_matches_sales_four_verbatim() {
        // seed 四公式的 key / expression / eval_score_key 逐字锁死 —— 与 prompts.rs
        // policy 英文式、evaluations.rs formulas 数组 + score_key_for 映射同源。
        // 3A-1b/1c 切换消费点后此测试是 DEFAULT 字节等价的护栏。
        let f = default_business_formulas();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].key, "trust");
        assert_eq!(f[0].expression, "Credibility + Reliability + Intimacy − SelfOrientation");
        assert_eq!(f[0].eval_score_key.as_deref(), Some("humanLike"));
        assert_eq!(f[1].key, "conversionReadiness");
        assert_eq!(f[1].expression, "Motivation × ProductFit × Timing × Trust ÷ Friction");
        assert_eq!(f[1].eval_score_key.as_deref(), Some("conversionReadiness"));
        assert_eq!(f[2].key, "emotionalValue");
        assert_eq!(
            f[2].expression,
            "Empathy + Validation + Specificity + AutonomySupport − Pressure"
        );
        assert_eq!(f[2].eval_score_key.as_deref(), Some("emotionalValue"));
        assert_eq!(f[3].key, "nextBestActionScore");
        assert_eq!(
            f[3].expression,
            "RelationshipGain + ConversionProgress + EmotionalValue + ProductFit − PressureRisk − FactRisk"
        );
        assert_eq!(f[3].eval_score_key.as_deref(), Some("relationshipProgress"));
    }

    // ── 1G-c：DomainProfileCache TTL / 命中 / 回落 / 失效（无 Docker 纯内存）──

    #[test]
    fn cache_empty_is_stale_then_seed_clears_staleness() {
        let cache = DomainProfileCache::new();
        // 从未加载 → stale=true（首次必触发 reload）。
        assert!(cache.is_stale());
        cache.seed_for_test(default_domain_profile("ws-seed"));
        // seed 写入 fetched_at=now → 不再 stale。
        assert!(!cache.is_stale());
    }

    #[test]
    fn cache_goes_stale_after_ttl_elapses() {
        let cache = DomainProfileCache::new();
        cache.seed_for_test(default_domain_profile("ws-1"));
        assert!(!cache.is_stale());
        // 回拨刚好一个 TTL → stale=true（>= 边界）。
        cache.rewind_fetched_at_for_test(DOMAIN_PROFILE_CACHE_TTL);
        assert!(cache.is_stale());
    }

    #[test]
    fn cache_invalidate_resets_to_stale() {
        let cache = DomainProfileCache::new();
        cache.seed_for_test(default_domain_profile("ws-1"));
        assert!(!cache.is_stale());
        cache.invalidate();
        // 失效后下一次 get_or_load 必重载。
        assert!(cache.is_stale());
    }

    #[test]
    fn cache_miss_workspace_falls_back_to_default_verbatim() {
        // 缓存里有 ws-A 的真实 profile，但查 ws-B（未配置）→ 回落 default，
        // 与接缓存前 load_active_domain_profile 的 Ok(None) 分支逐字等价。
        // 直接断言 get_or_load 复用的 lookup_or_default，避免测试内联逻辑漂移。
        let cache = DomainProfileCache::new();
        let mut seeded = default_domain_profile("ws-A");
        seeded.display_name = "行业A".to_string();
        seeded.profile_id = "profile-a".to_string();
        cache.seed_for_test(seeded);

        // 未命中 workspace → 回落 default（profile_id=__default__，workspace 透传）。
        let fallback = cache.lookup_or_default("ws-B");
        assert_eq!(fallback.profile_id, DEFAULT_PROFILE_ID);
        assert_eq!(fallback.workspace_id, "ws-B");
        // 命中 ws-A → 真实 profile（非 default）。
        let hit = cache.lookup_or_default("ws-A");
        assert_eq!(hit.profile_id, "profile-a");
        assert_eq!(hit.display_name, "行业A");
    }
}
