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
    AnsweringModeProfile, BusinessFormula, ChunkRole, CommitmentMarkers, CoverageDimension,
    DomainProfile, MemoryDimension, OutcomePolarity, ProfileDimension,
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

/// universal-domain-adaptation H17：DEFAULT_PROFILE 的销售域记忆维度 seed。逐字复刻
/// `memory.rs::compact_memory_card_with_previous` 写死的八个 `extra` 业务槽位 cap
/// （行 424-434 的 `limit_extra_array` 调用）+ Reply Agent memoryCandidates[].type
/// 当前枚举（preference/doNotDo/commitment/objection/openLoop）。作为 DEFAULT 等价的
/// 单一真相源：cap 接线后各消费方在空集时回落这同一组维度，故声明值与回落值字节相等，
/// 不因手抄漂移。等价性测试 `default_profile_memory_dimensions_match_hardcoded_verbatim`
/// 锁死。注：coreFacts(6)/recentFacts(10)/deprecatedFacts(20) 是 typed 骨架固定 cap，
/// 不在此表；confirmedFacts/conflicts 是 extra 槽但非 candidate 类型（candidate_type=false）。
pub fn default_memory_dimensions() -> Vec<MemoryDimension> {
    // (key, display_name, cap, candidate_type)：cap 逐字对齐 memory.rs limit_extra_array；
    // candidate_type 对齐 prompts.rs Reply Agent memoryCandidates[].type 枚举。
    let specs: &[(&str, &str, usize, bool)] = &[
        ("preferences", "偏好", 8, true),
        ("doNotDo", "禁忌/不要做", 10, true),
        ("commitments", "承诺", 8, true),
        ("objections", "异议", 8, true),
        ("openLoops", "未闭合事项", 8, true),
        ("openQuestions", "待解答问题", 8, false),
        ("confirmedFacts", "已确认事实", 12, false),
        ("conflicts", "冲突", 6, false),
    ];
    specs
        .iter()
        .map(|(key, name, cap, cand)| MemoryDimension {
            key: key.to_string(),
            display_name: name.to_string(),
            cap: *cap,
            is_core: false,
            prompt_hint: None,
            candidate_type: *cand,
            // §3.7：DEFAULT 销售八槽均非日期维度 → scan_calendar 对销售域 no-op。
            date_dimension: false,
        })
        .collect()
}

/// H17：把 active profile 的记忆维度渲染成一段 Reply Agent 任务指引，告知本行业
/// `memoryCandidates[].type` 的合法值（candidate_type=true 的维度 key + 固定的
/// fact/conflict）。
///
/// 与 consolidator 指引同款门控——**只在维度偏离 DEFAULT 销售八维时才追加**：
/// `user.reply.task` 静态 prompt 的 memoryCandidates schema 已写死销售 type 枚举
/// （fact|preference|doNotDo|commitment|objection|openLoop|conflict），DEFAULT profile 下
/// 它准确，追加冗余且扰动调好的销售行为 → DEFAULT 返回空串、Reply Agent prompt 逐字
/// 不变。换非销售行业（情感域声明情绪史/纪念日为 candidate_type）时，这段告知 LLM
/// 本行业真实可用的候选类型，让情感记忆能作为 candidate 写出（否则 LLM 只认骨架的
/// 销售 type）。`fact` / `conflict` 是系统固定派生（不依赖 candidate_type 字段）。
pub fn render_memory_candidate_types_guidance(
    dimensions: &[crate::models::MemoryDimension],
) -> String {
    if dimensions.is_empty() || dimensions == default_memory_dimensions().as_slice() {
        return String::new();
    }
    let mut types: Vec<String> = vec!["fact".to_string()];
    for dim in dimensions {
        if dim.candidate_type {
            types.push(dim.key.clone());
        }
    }
    types.push("conflict".to_string());
    format!(
        "\n\n# 本行业 memoryCandidates 合法 type（覆盖上面 schema 示例里的销售默认枚举）\n本行业可用的候选记忆 type 为：{}。请只用这些 type，按语义归类，不要沿用与本行业无关的销售字段。",
        types.join(" | ")
    )
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

/// universal-domain-adaptation H15：DEFAULT_PROFILE 的销售域四公式 seed，作为 DEFAULT
/// 等价的单一真相源。各字段对应散落副本的不同投影（**非逐字全等**，按字段对齐）：
/// - `expression`（英文式，Unicode 减号 `−`）= `prompts.rs` policy「关系经营公式
///   （自检）」段逐字。该段已由 decision.rs 运行时剥离+注入本函数渲染值，护栏
///   `strip_then_inject_default_roundtrips_to_original_section` 锁死。
/// - `key` + `expression` = `agent/review/mod.rs` reviewer `formulaBreakdown` 模板逐字
///   （`render_business_formulas_json_example`，护栏 `render_json_example_default_shape`）。
/// - `key` + `eval_score_key` = `routes/evaluations.rs` `formulas` 数组 + `score_key_for`
///   fallback 映射，护栏 `score_key_for_matches_default_formula_eval_keys`（第 77 点补盲）。
/// - `display_name`（中文名）= `prompts.rs` default_playbook method_prompt「核心公式」段
///   的中文公式名。**注意**：playbook 段是 H12 methodology 层的方法论叙述（用中文运算符 +
///   多一条「学习深度」非 5 闸公式），与本 seed **不逐字对齐、不强制同数量**——它走
///   methodology_override 路径自定，本 seed 只保证 4 个经营公式的中文名与之一致。
///
/// 空集时各消费方回落本函数同源常量（DEFAULT_PROFILE 即显式 seed 这四条，seed 与回落同源）。
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

/// 把 camelCase formula key 转成 policy 自检段用的 PascalCase 名（首字母大写）。
/// `trust`→`Trust`、`conversionReadiness`→`ConversionReadiness`，与原 policy 散文逐字对齐。
fn formula_key_pascal(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// universal-domain-adaptation H15（3A-1c）：把经营公式渲染成 policy「关系经营公式
/// （自检）」散文段的 markdown bullet 列表。**单一真相源**——policy prompt 不再内联
/// 写死公式，改由 decision.rs 运行时注入本函数输出。空集回落 [`default_business_formulas`]，
/// 故 DEFAULT_PROFILE 渲染出的 4 行与改造前 policy 散文逐字相同（PascalCase 名 +
/// expression 逐字）。
pub fn render_business_formulas_self_check(formulas: &[BusinessFormula]) -> String {
    let seed;
    let effective = if formulas.is_empty() {
        seed = default_business_formulas();
        &seed[..]
    } else {
        formulas
    };
    effective
        .iter()
        .map(|f| format!("- {} = {}", formula_key_pascal(&f.key), f.expression))
        .collect::<Vec<_>>()
        .join("\n")
}

/// universal-domain-adaptation H15（3A-1c-3）：policy prompt 里「关系经营公式（自检）」
/// 段的固定标题。运行时归一/注入都以它为锚。
pub const POLICY_FORMULA_SECTION_HEADING: &str = "## 关系经营公式（自检）";

/// 运行时自愈归一：从已加载的 policy 文本里剥离任何遗留的「关系经营公式（自检）」段
/// （`## 关系经营公式（自检）` 标题起，到下一个 `## ` 二级标题前为止）。对旧库
/// （seed 时内联写死公式段）→ 剥离；对新库（公式段已不在 seed 里）→ 原样返回。
/// 幂等：剥离后再调一次无变化。返回 (剥离后的文本, 是否剥离过)。
///
/// 这是「单一真相源 + 不 bump PROMPT_PACK_VERSION、不清运营编辑」方案的核心——
/// 公式块改由 [`render_business_formulas_self_check`] 运行时注入，本函数确保旧库不会
/// 出现「内联公式 + 注入公式」双份。
pub fn strip_legacy_formula_self_check_section(policy: &str) -> (String, bool) {
    let Some(start) = policy.find(POLICY_FORMULA_SECTION_HEADING) else {
        return (policy.to_string(), false);
    };
    // 从标题之后找下一个二级标题 `\n## `。
    let after = &policy[start..];
    let rest_offset = after
        .match_indices("\n## ")
        .find(|(i, _)| *i > 0)
        .map(|(i, _)| start + i + 1) // +1 跳过该换行，保留下一段的 `## `
        .unwrap_or(policy.len());
    let mut out = String::with_capacity(policy.len());
    out.push_str(policy[..start].trim_end_matches('\n'));
    if rest_offset < policy.len() {
        out.push_str("\n\n");
        out.push_str(&policy[rest_offset..]);
    }
    (out, true)
}

/// universal-domain-adaptation H15（3A-1c-3）：构造运行时注入 policy 的「关系经营
/// 公式（自检）」整段（标题 + 渲染的公式 bullet 列表）。空集回落 DEFAULT 四公式，
/// 故 DEFAULT_PROFILE 注入出的整段与旧库内联段逐字相同。
pub fn build_policy_formula_section(formulas: &[BusinessFormula]) -> String {
    format!(
        "{POLICY_FORMULA_SECTION_HEADING}\n\n{}",
        render_business_formulas_self_check(formulas)
    )
}

/// universal-domain-adaptation H9（第 20 点）：policy 里「## 对话模式判定」段的固定标题。
/// 运行时注入 `conversation_mode_policy` 覆盖时以它为锚剥离原销售判定段。
pub const POLICY_CONVERSATION_MODE_SECTION_HEADING: &str = "## 对话模式判定";

/// universal-domain-adaptation H9（第 20 点）：剥离 policy 里「## 对话模式判定」整段
/// （标题起，到下一个 `## ` 二级标题前为止）。仅当 active profile 声明了
/// `conversation_mode_policy` 覆盖时调用——把写死销售世界观的判定规则段剥掉，由
/// [`apply_conversation_mode_policy`] 注入本行业判定规则。下一段「## 模式与 5 闸的关系」
/// （含 boundary_protection 不放宽边界保护红线）不在剥离范围、继续写死守护。
///
/// 剥离逻辑与 [`strip_legacy_formula_self_check_section`] 同构（不同的只是 heading），
/// 幂等：剥离后再调一次无变化。返回 (剥离后的文本, 是否剥离过)。
pub fn strip_conversation_mode_section(policy: &str) -> (String, bool) {
    let Some(start) = policy.find(POLICY_CONVERSATION_MODE_SECTION_HEADING) else {
        return (policy.to_string(), false);
    };
    let after = &policy[start..];
    let rest_offset = after
        .match_indices("\n## ")
        .find(|(i, _)| *i > 0)
        .map(|(i, _)| start + i + 1) // +1 跳过该换行，保留下一段的 `## `
        .unwrap_or(policy.len());
    let mut out = String::with_capacity(policy.len());
    out.push_str(policy[..start].trim_end_matches('\n'));
    if rest_offset < policy.len() {
        out.push_str("\n\n");
        out.push_str(&policy[rest_offset..]);
    }
    (out, true)
}

/// universal-domain-adaptation H9（第 20 点）：把 active profile 的对话模式判定规则
/// 应用到 policy 文本。`None`（DEFAULT_PROFILE / 老库）→ 原样返回，销售判定段逐字保留、
/// 销售域零变化。`Some` → 剥离写死的「## 对话模式判定」段并在原位注入本行业规则
/// （注入文本应自带 `## 对话模式判定` 标题；若运营漏写标题，这里补一个锚，保证下游
/// 「## 模式与 5 闸的关系」段衔接）。
pub fn apply_conversation_mode_policy(policy: &str, override_text: Option<&str>) -> String {
    let Some(raw) = override_text.map(str::trim).filter(|s| !s.is_empty()) else {
        return policy.to_string();
    };
    let (stripped, _) = strip_conversation_mode_section(policy);
    let injected = if raw.starts_with(POLICY_CONVERSATION_MODE_SECTION_HEADING) {
        raw.to_string()
    } else {
        format!("{POLICY_CONVERSATION_MODE_SECTION_HEADING}\n\n{raw}")
    };
    // 注入到剥离后文本的最前（原判定段就在 policy 开头），与下文以空行分隔。
    format!("{injected}\n\n{}", stripped.trim_start_matches('\n'))
}

/// 把模式集合渲染成 policy `## 决策协议字段` 里的 JSON 数组枚举形：
/// `["casual_relationship", "value_exchange", ...]`（与写死文本逐字同形）。
fn render_modes_json_array(modes: &[String]) -> String {
    let inner = modes
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// 把模式集合渲染成 task final 形态契约里的竖线枚举形：
/// `casual_relationship | value_exchange | ...`（与写死文本逐字同形）。
fn render_modes_pipe(modes: &[String]) -> String {
    modes.join(" | ")
}

/// universal-domain-adaptation H9 修复（问题 A）：把 prompt 里**写死的 conversationMode
/// 四模式枚举列表**替换为 active profile 声明的模式集合。
///
/// 背景：`user.reply.policy` 的「## 决策协议字段」段（JSON 数组形）与 `user.reply.task`
/// 的 final 形态契约（竖线形）都写死了销售四模式 `[casual_relationship / value_exchange /
/// consultative / boundary_protection]`，**无条件注入**、不随 profile 变。而运行时
/// `validate_and_promote`（types.rs）用的是 `runtime.allowed_conversation_modes`
/// （= active profile.conversation_modes 覆盖）做严格枚举校验。二者一旦不一致：非销售
/// 行业（声明 `intimate_companion` 等）的 LLM 被 prompt 带偏输出销售模式、或输出本行业
/// 模式但 prompt 压它选销售模式造成漂移 → `invalid_enum_value:conversation_mode:*` →
/// **硬协议违规**（gates.rs `is_protocol_violation_tag`）→ reply 被硬 block、不给改写。
///
/// 本函数以「单一真相源」`runtime::default_conversation_modes()`（四模式）构造**旧串**，
/// 以 active profile 的 `modes`（空集回落同一默认）构造**新串**，在 `text` 里做精确子串
/// 替换。**DEFAULT_PROFILE / 老库**（modes 为空或恰为默认四模式）→ 旧串==新串 → 不替换
/// → prompt 字节等价、销售域零变化。换行业声明非默认模式集 → policy/task 的枚举列表与
/// runtime 校验集合对齐，消除矛盾指令。
///
/// 只替换精确的枚举列表子串（数组形 + 竖线形），不触碰「## 模式与 5 闸的关系」段里
/// 各模式的散文描述（boundary_protection 边界保护红线段继续写死守护）。
pub fn apply_conversation_mode_enum_list(text: &str, modes: &[String]) -> String {
    let default_modes = crate::agent::runtime::default_conversation_modes();
    let effective: Vec<String> = if modes.is_empty() {
        default_modes.clone()
    } else {
        modes.to_vec()
    };
    let mut out = text.to_string();
    let old_array = render_modes_json_array(&default_modes);
    let new_array = render_modes_json_array(&effective);
    if new_array != old_array {
        out = out.replace(&old_array, &new_array);
    }
    let old_pipe = render_modes_pipe(&default_modes);
    let new_pipe = render_modes_pipe(&effective);
    if new_pipe != old_pipe {
        out = out.replace(&old_pipe, &new_pipe);
    }
    out
}

/// universal-domain-adaptation H15（3A-1c）：把经营公式渲染成 reviewer prompt
/// `formulaBreakdown` JSON 示例的内层行（`"key": "expression",` 逐行，最后一行无逗号）。
/// 同一单一真相源；空集回落 [`default_business_formulas`]。
pub fn render_business_formulas_json_example(formulas: &[BusinessFormula]) -> String {
    let seed;
    let effective = if formulas.is_empty() {
        seed = default_business_formulas();
        &seed[..]
    } else {
        formulas
    };
    let n = effective.len();
    effective
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let comma = if i + 1 < n { "," } else { "" };
            format!("    \"{}\": \"{}\"{}", f.key, f.expression, comma)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// universal-domain-adaptation 第 19 点：reviewer `scores` 示例块里**销售专属软维度**
/// （`relationshipProgress` / `conversionReadiness`）条件化。
///
/// reviewer 实际消费（[`crate::agent::types::ReviewScores`]）的只有 5 个硬闸维度
/// （humanLike / emotionalValue / productAccuracy / pressureRisk / factRisk）——它们
/// 写死在 scores 示例上下半段、域无关、始终保留。`relationshipProgress` /
/// `conversionReadiness` 这类**软观测维度**反序列化时被丢弃（不在 ReviewScores 里），
/// 纯属示例装饰；销售域才有"成交准备度/关系推进"语义，情感陪伴/同行/朋友域不该被强塞。
///
/// 本函数从 active profile 的 `business_formulas.eval_score_key` 派生这些额外软维度
/// （排除 5 个硬闸键），渲染成 scores 块中段的若干行（每行 `"key": 6,`，带行尾换行）。
/// DEFAULT_PROFILE 的四公式 eval_score_key = [humanLike, conversionReadiness,
/// emotionalValue, relationshipProgress]，排除硬闸后 → conversionReadiness +
/// relationshipProgress 两行。非销售 profile 未声明这些 eval_score_key → 返回空串，
/// scores 块只剩 5 个硬闸维度。
///
/// **字节等价豁免点（2026-06-16 审查 D2-1 复核批准，非疏漏）**：改造前 prompt 这两行
/// 手写顺序是 `relationshipProgress` 在前、`conversionReadiness` 在后；本函数按
/// `default_business_formulas` 声明序产出，得 `conversionReadiness` 在前、
/// `relationshipProgress` 在后——**两行换序**，故 DEFAULT 销售域 reviewer prompt
/// 并非逐字节等于改造前。判定**可接受**：①这两键不在 [`crate::agent::types::ReviewScores`]、
/// 反序列化即被 serde 丢弃（已由 types.rs `legacy_review_json` 等测试旁证无消费方）；
/// ②两行值同为 6；③不修是因为「改 default_business_formulas 声明序」会破坏更大的字节锁
/// （`render_self_check_default_matches_policy_prose_verbatim` 锁的 policy 自检段 +
/// `render_json_example_default_shape` 锁的 formulaBreakdown，二者依赖现声明序对齐原
/// policy 英文式），「render 内对销售键特排」又会往通用化函数塞销售特例。两害取轻，
/// 登记为「render 后语义等价、非逐字节」豁免（沿用 H15 既有标准）。
pub fn render_reviewer_extra_score_lines(formulas: &[BusinessFormula]) -> String {
    const HARD_GATES: [&str; 5] = [
        "humanLike",
        "emotionalValue",
        "productAccuracy",
        "pressureRisk",
        "factRisk",
    ];
    let seed;
    let effective = if formulas.is_empty() {
        seed = default_business_formulas();
        &seed[..]
    } else {
        formulas
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for f in effective {
        if let Some(key) = f.eval_score_key.as_deref() {
            if HARD_GATES.contains(&key) || !seen.insert(key) {
                continue;
            }
            lines.push(format!("    \"{key}\": 6,"));
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// universal-domain-adaptation D：reviewer system prompt 里写死的「评审重点」取向描述
/// （冒号后那串）。`apply_reviewer_review_focus` 以它为锚做精确整行替换。
pub const REVIEWER_REVIEW_FOCUS_LABEL: &str = "评审重点：";
pub const DEFAULT_REVIEWER_REVIEW_FOCUS: &str =
    "事实准确、像真人微信、情绪价值、低压推进、产品知识一致性、没有操控营销。";

/// universal-domain-adaptation D：reviewer user prompt 评审原则里写死的「转化平衡」整条
/// bullet（`- ` 之后的整句）。`apply_reviewer_balance_principle` 以它为锚做精确替换。
pub const DEFAULT_REVIEWER_BALANCE_PRINCIPLE: &str =
    "转化平衡：既允许适度推进，也不能伤害信任。";

/// universal-domain-adaptation D：把 active profile 的 `reviewer_orientation.review_focus`
/// 应用到 reviewer **system** prompt。`None`（DEFAULT/老库）→ 原样返回（销售取向逐字保留、
/// 销售域字节等价）。`Some` → 把「评审重点：<销售取向>。」整行替换成「评审重点：<本域取向>」
/// （标签「评审重点」域中性故保留，只换冒号后的取向描述）。
///
/// 锚 = `评审重点：` + [`DEFAULT_REVIEWER_REVIEW_FOCUS`]。找不到锚（prompt 被运营改写过）→
/// 原样返回不强插，避免污染。与 [`apply_conversation_mode_enum_list`] 同构（精确子串替换、
/// 幂等、空覆盖即 no-op）。
pub fn apply_reviewer_review_focus(system: &str, override_text: Option<&str>) -> String {
    let Some(focus) = override_text.map(str::trim).filter(|s| !s.is_empty()) else {
        return system.to_string();
    };
    let old_line = format!("{REVIEWER_REVIEW_FOCUS_LABEL}{DEFAULT_REVIEWER_REVIEW_FOCUS}");
    let new_line = format!("{REVIEWER_REVIEW_FOCUS_LABEL}{focus}");
    if new_line == old_line {
        return system.to_string();
    }
    system.replace(&old_line, &new_line)
}

/// universal-domain-adaptation D：把 active profile 的
/// `reviewer_orientation.balance_principle` 应用到 reviewer **user** prompt 评审原则。
/// `None`（DEFAULT/老库）→ 原样返回（销售域字节等价）。`Some` → 把「转化平衡：既允许适度
/// 推进，也不能伤害信任。」整条替换成本域取向（标签「转化平衡」含销售「转化」语义，故整条
/// 含标签一并替换）。锚找不到 → 原样返回。空覆盖即 no-op、幂等。
pub fn apply_reviewer_balance_principle(user: &str, override_text: Option<&str>) -> String {
    let Some(principle) = override_text.map(str::trim).filter(|s| !s.is_empty()) else {
        return user.to_string();
    };
    if principle == DEFAULT_REVIEWER_BALANCE_PRINCIPLE {
        return user.to_string();
    }
    user.replace(DEFAULT_REVIEWER_BALANCE_PRINCIPLE, principle)
}

/// universal-domain-adaptation T3：把 active profile 的
/// `reviewer_orientation.reviewer_fewshot_override` 应用到 reviewer **system** prompt 的
/// 「软闸打分锚点（few-shot）」三档示例段。`None`（DEFAULT/老库）/ 空白 → 原样返回
/// （销售 few-shot 逐字保留、销售域字节等价）。`Some` 且 != 销售锚 → 把整段
/// [`crate::prompts::DEFAULT_REVIEWER_FEWSHOT`] 替换成本域 few-shot（非销售域的打分尺度，
/// 替换掉销售逼单高压锚）。锚找不到（prompt 被运营改写过）→ 原样返回不强插，避免污染。
///
/// 与 [`apply_reviewer_review_focus`] / [`apply_mode_gate_policy`] 同构
/// （精确子串替换、幂等、空覆盖即 no-op）。
pub fn apply_reviewer_fewshot(system: &str, override_text: Option<&str>) -> String {
    let Some(fewshot) = override_text.map(str::trim).filter(|s| !s.is_empty()) else {
        return system.to_string();
    };
    if fewshot == crate::prompts::DEFAULT_REVIEWER_FEWSHOT {
        return system.to_string();
    }
    system.replace(crate::prompts::DEFAULT_REVIEWER_FEWSHOT, fewshot)
}

/// universal-domain-adaptation A/T1：把 active profile 的 `mode_gate_policy_override`
/// 应用到 decision/policy prompt 的「## 模式与 5 闸的关系」模式-闸说明段。
/// `None`（DEFAULT/老库）/ 空白 → 原样返回（销售取向逐字保留、销售域字节等价）。
/// `Some` 且 != 销售锚 → 把整段 [`crate::prompts::DEFAULT_MODE_GATE_POLICY`] 替换成本域
/// 模式-闸说明。锚找不到（prompt 被运营改写过）→ 原样返回不强插，避免污染。
///
/// 与 [`apply_reviewer_review_focus`] / [`apply_reviewer_balance_principle`] 同构
/// （精确子串替换、幂等、空覆盖即 no-op）。注意销售锚**不含** boundary_protection
/// 红线续行——那是跨域恒定红线，不随 profile 替换。本函数只替换模式-闸说明散文。
pub fn apply_mode_gate_policy(system: &str, override_text: Option<&str>) -> String {
    let Some(policy) = override_text.map(str::trim).filter(|s| !s.is_empty()) else {
        return system.to_string();
    };
    if policy == crate::prompts::DEFAULT_MODE_GATE_POLICY {
        return system.to_string();
    }
    system.replace(crate::prompts::DEFAULT_MODE_GATE_POLICY, policy)
}

/// universal-domain-adaptation I：completeness `answeringMode` 三档的写死销售释义
/// （喂 LLM 的「判断规则」段，逐字复刻 catalog.rs 原文）。三档 key 是域无关认知阶梯，
/// 恒定；释义可被 `AnsweringModeProfile` 按行业覆盖。
pub const DEFAULT_ANSWERING_RULE_RELATIONSHIP_ONLY: &str =
    "没有足够 verified 知识支撑产品/服务事实，只能关系维护、澄清需求、收集信息。";
pub const DEFAULT_ANSWERING_RULE_PRODUCT_SAFE: &str =
    "可回答部分产品/服务能力，但报价、案例、效果或交付边界仍不足。";
pub const DEFAULT_ANSWERING_RULE_FULLY_SUPPORTED: &str =
    "能力、边界、证据类内容足够支撑常见产品事实问题。";

/// I：三档前端中文标签的写死销售文案（逐字复刻 `AnsweringModeGauge.tsx` MODE_MAP）。
pub const DEFAULT_ANSWERING_LABEL_RELATIONSHIP_ONLY: &str = "仅关系维护";
pub const DEFAULT_ANSWERING_LABEL_PRODUCT_SAFE: &str = "可安全讲产品";
pub const DEFAULT_ANSWERING_LABEL_FULLY_SUPPORTED: &str = "完全支撑";

/// I：渲染 completeness 审计 prompt「判断规则」段开头的三档释义 bullet。
///
/// 每档释义按 `AnsweringModeProfile.{档}.rule` 覆盖，`None`（DEFAULT/老库 / 该档未声明）
/// 回落写死销售释义。三档 key 恒定（`relationship_only` / `product_safe` /
/// `fully_supported`）。DEFAULT_PROFILE（answering_mode_profile=None）→ 三行与改造前
/// prompt 字面量逐字一致 → completeness prompt 字节等价。
pub fn render_answering_mode_rules(profile: Option<&AnsweringModeProfile>) -> String {
    let pick = |descriptor: Option<&crate::models::AnsweringModeDescriptor>, default: &str| {
        descriptor
            .and_then(|d| d.rule.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default)
            .to_string()
    };
    let r = pick(
        profile.and_then(|p| p.relationship_only.as_ref()),
        DEFAULT_ANSWERING_RULE_RELATIONSHIP_ONLY,
    );
    let p_safe = pick(
        profile.and_then(|p| p.product_safe.as_ref()),
        DEFAULT_ANSWERING_RULE_PRODUCT_SAFE,
    );
    let f = pick(
        profile.and_then(|p| p.fully_supported.as_ref()),
        DEFAULT_ANSWERING_RULE_FULLY_SUPPORTED,
    );
    format!(
        "- relationship_only: {r}\n- product_safe: {p_safe}\n- fully_supported: {f}"
    )
}

/// I：三档前端标签（label）解析，按 `AnsweringModeProfile.{档}.label` 覆盖、`None` 回落
/// 写死销售标签。回传 `(relationship_only, product_safe, fully_supported)` 标签三元组，
/// 由 completeness 响应带给前端 `AnsweringModeGauge`（前端不再硬编码销售标签）。
pub fn answering_mode_labels(profile: Option<&AnsweringModeProfile>) -> (String, String, String) {
    let pick = |descriptor: Option<&crate::models::AnsweringModeDescriptor>, default: &str| {
        descriptor
            .and_then(|d| d.label.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default)
            .to_string()
    };
    (
        pick(
            profile.and_then(|p| p.relationship_only.as_ref()),
            DEFAULT_ANSWERING_LABEL_RELATIONSHIP_ONLY,
        ),
        pick(
            profile.and_then(|p| p.product_safe.as_ref()),
            DEFAULT_ANSWERING_LABEL_PRODUCT_SAFE,
        ),
        pick(
            profile.and_then(|p| p.fully_supported.as_ref()),
            DEFAULT_ANSWERING_LABEL_FULLY_SUPPORTED,
        ),
    )
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
        // H9（第 20 点）：DEFAULT 对话模式判定规则 = None → 保留 policy 写死的销售判定段
        // （逐字等价、销售域零变化）。换行业声明本字段即整段替换判定规则。
        conversation_mode_policy: None,
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
            CoverageDimension { key: "capability".to_string(), display_name: "能力".to_string(), required: false, anchor_hint: Some("有 verified 切片陈述产品/服务\"能做什么\"的具体能力或功能事实。".to_string()), initial_signal: Some("verified".to_string()) },
            CoverageDimension { key: "pricing".to_string(), display_name: "报价".to_string(), required: false, anchor_hint: Some("有 verified 切片含具体报价/计费/套餐金额（注意：仅 needs_review 草稿里的报价不计入 verifiedFact，而应置 pendingDraft=true 并入 gap）。".to_string()), initial_signal: None },
            CoverageDimension { key: "caseEvidence".to_string(), display_name: "案例证据".to_string(), required: false, anchor_hint: Some("有 verified 切片描述**具体客户案例/实施成效**（含可核验的主体、场景或落地结果），即判 true。".to_string()), initial_signal: Some("evidence".to_string()) },
            CoverageDimension { key: "effectClaims".to_string(), display_name: "效果声明".to_string(), required: false, anchor_hint: Some("有 verified 切片含**可核验的效果数据/量化成果**（如转化率提升、响应时长变化等具体数字），即判 true。".to_string()), initial_signal: Some("evidence".to_string()) },
            CoverageDimension { key: "deliveryBoundary".to_string(), display_name: "交付边界".to_string(), required: false, anchor_hint: Some("有 verified 切片陈述交付方式/SLA/可用性/部署边界等具体条款。".to_string()), initial_signal: Some("verified".to_string()) },
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
        // §3.7：DEFAULT 销售 profile 不声明按关系类型的多套范式 → resolve 回落 operation_mode。
        per_relationship_operation_mode: None,
        // H14：DEFAULT 销售域 = false → grounding 软分数硬闸无条件生效（字节等价）。
        grounding_gate_bypass_without_claim: false,
        // reviewer 优化：DEFAULT 销售域 = false → 沿用既有 should_run_review 判定
        // （字节等价）；高敏域（情感陪伴）seed 时显式置 true 强制走 LLM review。
        distrust_self_reported_low_risk: false,
        // G4 #5：DEFAULT 是销售域（交易型）= true → 注入产品目录 + 持有投影段（逐字等价
        // 历史行为，反过拟合护栏）。非交易域 profile 显式置 false 关闭交易事实注入。
        transaction_facts_enabled: true,
        // H16：DEFAULT 销售域 = 逐字复刻 knowledge_router 写死的四态角色（字节等价）。
        chunk_roles: default_chunk_roles(),
        // H11：DEFAULT 销售极性 = 显式填回回路① fallback 常量（正极 buying_signal +
        // 负极 5 词）。空集 default 会让消费方回落同一对常量，故 seed 与回落同源、字节等价。
        outcome_polarity: default_outcome_polarity(),
        // H15：DEFAULT 销售域 = 显式填回四公式（Trust/ConversionReadiness/EmotionalValue/
        // NextBestActionScore）。空集时各消费方回落内置销售公式常量，故 seed 与回落同源、
        // 字节等价。
        business_formulas: default_business_formulas(),
        // H17：DEFAULT 销售域 = 显式填回八个记忆维度（preferences/doNotDo/commitments/
        // objections/openLoops/openQuestions/confirmedFacts/conflicts）+ 原 cap。空集时
        // 消费方回落同一组维度，故 seed 与回落同源、cap/prompt 字节等价。
        memory_dimensions: default_memory_dimensions(),
        // C3：DEFAULT 不声明行业专属生成器引导语 → 回落领域中性 PLAYBOOK_METHODOLOGY_SYSTEM
        // （已去销售偏见）。换行业可在引导层声明自己的生成偏好。
        methodology_generator_preamble: None,
        // M2：DEFAULT 不覆盖五闸阈值 → gateway 沿用 domain_config 解析的销售域阈值
        // （字节等价）。换行业可声明自己的阈值（如情感域放宽 pressure_risk）。
        threshold_overrides: None,
        // D：DEFAULT 不覆盖评审取向 → reviewer prompt 的「评审重点 / 转化平衡」两句保留
        // 写死销售取向（字节等价）。换行业可声明中性 / 本域取向。
        reviewer_orientation: None,
        // A/T1：DEFAULT 不覆盖模式-闸说明 → decision/policy prompt「## 模式与 5 闸的关系」
        // 保留写死销售说明（prompt 字节等价）。非销售域可声明本域模式-闸尺度。
        mode_gate_policy_override: None,
        // I：DEFAULT 不覆盖 answeringMode 三档释义/标签 → completeness prompt 三档释义
        // 与前端档位标签保留写死销售文案（prompt 字节等价、UI 标签不变）。
        answering_mode_profile: None,
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("default".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

/// universal-domain-adaptation 第 78 点：情感陪伴行业的最小示例 DomainProfile
/// 构造器——**非销售价值契约的单一真相源**。以 [`default_domain_profile`] 为基底，
/// 仅覆盖体现"长期陪伴、情绪承接、尊重边界、不做成交推进"价值观的关键字段。
///
/// 既供 lib 单测做纯内存端到端价值断言（profile → runtime → 下游 gate 行为），也供
/// `tests/common/roleplay_fixtures.rs` 的 seed helper 复用（单一真相源，避免两份漂移）。
///
/// 价值契约（与设计 §5.2 对齐）：
/// - `conversation_modes` 含 `intimate_companion`（H9：不只销售四模式）；
/// - `grounding_gate_bypass_without_claim = true`（H14：纯情感回复不被 grounding 误拦）；
/// - `distrust_self_reported_low_risk = true`（高敏域强制走 LLM review）；
/// - `transaction_facts_enabled = false`（G4 #5：非交易域不注入产品/持有事实）；
/// - `operation_mode.funnel.enabled = false`（H8：陪伴不催进成交）。
pub fn example_emotional_companion_profile(workspace_id: &str) -> DomainProfile {
    let mut profile = default_domain_profile(workspace_id);
    profile.profile_id = "emotional_companion_minimal".to_string();
    profile.display_name = "情感陪伴".to_string();
    profile.description = "长期陪伴、情绪承接、尊重边界，不做成交推进".to_string();
    profile.conversation_modes = vec![
        "intimate_companion".to_string(),
        "casual_relationship".to_string(),
        "value_exchange".to_string(),
        "boundary_protection".to_string(),
    ];
    profile.grounding_gate_bypass_without_claim = true;
    profile.distrust_self_reported_low_risk = true;
    // G4 #5：情感陪伴是非交易域 → 显式关交易事实注入（派生自 default_domain_profile 的
    // true，不显式覆盖会继承注入）。即便 admin 误配产品表也不让"已购买X"裸入情感对话。
    profile.transaction_facts_enabled = false;
    profile.operation_mode.funnel.enabled = false;
    // §3.7：开启主动情绪关怀驱动力（销售域默认关）。纪念日/生日当天主动触达。
    profile.operation_mode.calendar.enabled = true;
    // §3.7：声明一个带日期语义的记忆维度 anniversaries，consolidator 据 date_dimension
    // 引导 LLM 输出结构化日期对象（AnniversaryEntry），scan_calendar 据此做今日匹配。
    profile.memory_dimensions.push(crate::models::MemoryDimension {
        key: "anniversaries".to_string(),
        display_name: "纪念日".to_string(),
        cap: 8,
        is_core: true,
        prompt_hint: Some("生日 / 相识纪念 / 重要日子（含日期）".to_string()),
        candidate_type: false,
        date_dimension: true,
    });
    profile.prompt_fragment = Some(
        "本行业目标是长期陪伴、情绪承接、尊重对方节奏与边界，不是成交推进。\
         主动关心、轻量追问本身是正当行为，不等于施压。"
            .to_string(),
    );
    // §3.7 数字分身样例：按关系类型配三套范式（运营接入时给 contact 标 relationship_type
    // 即按此路由）。customer=漏斗全开追单；peer=漏斗关、低频维护；friend=漏斗关、只留日历
    // 关怀、口吻最像本人。这是"微信号 AI 化身"托管客户/同行/朋友三层社交的具体兑现。
    let mut per_relationship = std::collections::BTreeMap::new();
    // 客户：漏斗 + 沉默 + 承诺 + 日历全开（怕丢单）。
    let mut customer_mode = crate::models::OperationMode::default();
    customer_mode.calendar.enabled = true;
    per_relationship.insert("customer".to_string(), customer_mode);
    // 同行：漏斗关、低频维护，留承诺与日历（行业节点祝福）。
    let mut peer_mode = crate::models::OperationMode::default();
    peer_mode.funnel.enabled = false;
    peer_mode.calendar.enabled = true;
    per_relationship.insert("peer".to_string(), peer_mode);
    // 朋友：漏斗关、承诺关，只留日历个人情感关怀，口吻最像本人。
    let mut friend_mode = crate::models::OperationMode::default();
    friend_mode.funnel.enabled = false;
    friend_mode.commitment.enabled = false;
    friend_mode.calendar.enabled = true;
    per_relationship.insert("friend".to_string(), friend_mode);
    profile.per_relationship_operation_mode = Some(per_relationship);
    profile
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
///
/// `pub`（非 `pub(crate)`）：集成测试 seed active DomainProfile 后必须能强制失效
/// 进程级缓存，否则 30s TTL 窗口内 `load_active_domain_profile` 仍返回 seed 前的
/// 旧值（roleplay-fuzz P0 fixture 落地依赖此入口）。生产语义不变——引导层 publish
/// profile 后本就应调用它。
pub fn invalidate_global_domain_profile_cache() {
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

/// universal-domain-adaptation G1：销售域两维 kind 集合（`customer_stage` /
/// `intent_level`）——它们由 LLM 以 typed JSON 键（`customerStage`/`intentLevel`）
/// 输出，prompt schema 已写死，**不**走 `domainSignals` 容器。其余「参与决策」维度
/// （购买生命周期 / 关系亲密度 / 情绪状态等）才需要本模块的 prompt 指引告知 LLM 走
/// `domainSignals` 容器输出。
///
/// 派生自 `dimension_registry::typed_dimension_kinds()` 单一真相源（收敛历史硬编码
/// 列表，零行为变化）。

/// G1：把 active profile 里**非销售 typed**的「参与决策」维度渲染成一段决策任务
/// 指引，告知 Reply Agent 这些维度要写进 `domainSignals` 容器（而非 schema 里写死的
/// 销售 typed 键）。
///
/// 与 H17 [`render_memory_candidate_types_guidance`] 同款门控——**只在存在非销售
/// typed 维度时才追加**：
/// - DEFAULT 销售域只有 `customer_stage` / `intent_level` 两维（均为 typed），过滤后
///   为空 → 返回空串 → Reply Agent / 初始画像 prompt **字节不变、销售零扰动**；
/// - 换非销售行业（陪伴域声明 `relationship_closeness`、本专题的 `purchase_lifecycle`
///   等）时，本段告知 LLM 这些维度的合法语义与输出位置，让维度值能真正从 LLM 流到
///   `AgentDecision.domain_signals`（否则 prompt 从不提 `domainSignals`，LLM 不会输出，
///   维度永远空）。
///
/// `dimensions` 传 `profile.profile_dimensions`；只取 `participates_in_decision=true`
/// 且 kind 不在销售 typed 集合里的维度。`description` 非空时一并注入语义提示。
pub fn render_decision_dimensions_guidance(dimensions: &[ProfileDimension]) -> String {
    let extra: Vec<&ProfileDimension> = dimensions
        .iter()
        .filter(|d| {
            d.participates_in_decision
                && !crate::agent::dimension_registry::typed_dimension_kinds()
                    .contains(&d.kind.as_str())
        })
        .collect();
    if extra.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::with_capacity(extra.len());
    for d in &extra {
        if d.description.trim().is_empty() {
            lines.push(format!("- {}（{}）", d.kind, d.display_name));
        } else {
            lines.push(format!(
                "- {}（{}）：{}",
                d.kind,
                d.display_name,
                d.description.trim()
            ));
        }
    }
    format!(
        "\n\n# 本行业参与决策的画像维度（写进 domainSignals 容器）\n\
         除上面 schema 里的字段外，本行业还要在 JSON 顶层输出一个 \"domainSignals\" 对象，\
         为下列每个维度给出当前取值（取值用简短词，能从对话或画像中解释）：\n{}\n\
         示例：\"domainSignals\": {{ {} }}。维度取值无法判断时该键留空或省略，不要臆测。",
        lines.join("\n"),
        extra
            .iter()
            .map(|d| format!("\"{}\": \"...\"", d.kind))
            .collect::<Vec<_>>()
            .join(", ")
    )
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
        // 跨模块等价护栏（修复 G）：DEFAULT seed 必须逐字等于 guards 的 fallback const
        // 单一真相源——直接引用 guards::{PRODUCT_EFFECT_MARKERS, TONE_ONLY_MARKERS}，
        // 而非各自抄一份字面量。此前本测试只断言 seed==内联字面量、从不引用 guards const，
        // 故 guards const 若被改，seed 与 fallback 漂移也照样绿（与 outcome_polarity 的
        // 引用式同源相比缺一层保护）。现升级为真交叉引用，锁死两侧任一漂移。
        let p = default_domain_profile("ws-1");
        assert_eq!(
            p.commitment_markers.product_effect,
            crate::agent::guards::PRODUCT_EFFECT_MARKERS.to_vec(),
            "DEFAULT seed product_effect 必须与 guards::PRODUCT_EFFECT_MARKERS 逐字一致"
        );
        assert_eq!(
            p.commitment_markers.tone_only,
            crate::agent::guards::TONE_ONLY_MARKERS.to_vec(),
            "DEFAULT seed tone_only 必须与 guards::TONE_ONLY_MARKERS 逐字一致"
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
        // §3.7 护栏：calendar 默认**关**（主动情绪触达销售域绝不默认开）→ scan_calendar
        // 对销售域 no-op，所有 planner 金标零变化。
        assert!(!p.operation_mode.calendar.enabled);
    }

    #[test]
    fn default_profile_memory_dimensions_have_no_date_dimension() {
        // §3.7 护栏：DEFAULT 销售八槽均非 date_dimension → scan_calendar 在销售域没有
        // 数据源、整段 no-op（无任何带日期语义维度时直接短路），字节等价。
        let p = default_domain_profile("ws-1");
        assert!(
            p.memory_dimensions.iter().all(|d| !d.date_dimension),
            "DEFAULT 八槽不应有 date_dimension=true"
        );
    }

    #[test]
    fn emotional_companion_profile_enables_calendar_with_date_dimension() {
        // §3.7：情感陪伴 profile 开 calendar + 声明带日期语义的 anniversaries 槽，
        // scan_calendar 据此对该域生效（与 DEFAULT 销售域形成对照）。
        let p = example_emotional_companion_profile("ws-e");
        assert!(p.operation_mode.calendar.enabled, "情感域应开 calendar");
        let anni = p
            .memory_dimensions
            .iter()
            .find(|d| d.key == "anniversaries")
            .expect("应声明 anniversaries 维度");
        assert!(anni.date_dimension, "anniversaries 应标 date_dimension");
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
        // reviewer 优化逐字等价护栏：DEFAULT_PROFILE = false → 沿用既有
        // should_run_review 判定（销售域字节等价）；高敏域 seed 时才置 true。
        assert!(!p.distrust_self_reported_low_risk);
        // G4 #5 逐字等价护栏：DEFAULT 是销售域（交易型）→ transaction_facts_enabled=true，
        // 决策注入产品目录 + 持有投影段，与改造前注入行为字节等价。注意此开关默认 false
        // 不代表销售等价（销售行为是注入），故 default_domain_profile 必须显式置 true。
        assert!(
            p.transaction_facts_enabled,
            "DEFAULT 销售域必须开交易事实注入（反过拟合护栏）"
        );
    }

    #[test]
    fn emotional_companion_disables_transaction_facts() {
        // G4 #5：情感陪伴是非交易域 → 显式关交易事实注入，即便 admin 误配产品表也不让
        // "已购买X"裸入情感对话。派生自 default（true），必须被显式覆盖为 false。
        let p = example_emotional_companion_profile("ws-1");
        assert!(
            !p.transaction_facts_enabled,
            "情感陪伴域必须关交易事实注入"
        );
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
        // H17：memory_dimensions 经 BSON 往返不丢（camelCase key/displayName/cap/isCore/promptHint/candidateType）。
        assert_eq!(parsed.memory_dimensions, p.memory_dimensions);
        // M2：DEFAULT threshold_overrides=None 经 BSON 往返仍 None（不覆盖五闸、零扰动）。
        assert!(parsed.threshold_overrides.is_none());
        // §3.7：DEFAULT per_relationship_operation_mode=None 经 BSON 往返仍 None
        //（销售域不配多套范式，resolve 回落 operation_mode、零扰动）。
        assert!(parsed.per_relationship_operation_mode.is_none());
    }

    /// §3.7：per_relationship_operation_mode = Some(多关系 map) 经 BSON 往返不丢，
    /// 且 BTreeMap 键序稳定。仿 profile_thresholds_partial_override_bson_round_trip。
    #[test]
    fn per_relationship_operation_mode_bson_round_trip() {
        use std::collections::BTreeMap;
        let mut profile = default_domain_profile("ws-1");
        let mut map = BTreeMap::new();
        // customer：默认三全开；friend：关 funnel。
        map.insert(
            "customer".to_string(),
            crate::models::OperationMode::default(),
        );
        let mut friend_mode = crate::models::OperationMode::default();
        friend_mode.funnel.enabled = false;
        map.insert("friend".to_string(), friend_mode.clone());
        profile.per_relationship_operation_mode = Some(map);

        let doc = mongodb::bson::to_document(&profile).expect("serialize");
        let parsed: DomainProfile = mongodb::bson::from_document(doc).expect("deserialize");
        let parsed_map = parsed
            .per_relationship_operation_mode
            .expect("per_relationship 往返不丢");
        assert_eq!(parsed_map.len(), 2);
        assert!(
            parsed_map.get("customer").expect("customer 那套").funnel.enabled,
            "customer 三全开往返保持"
        );
        assert_eq!(
            parsed_map.get("friend").expect("friend 那套").funnel.enabled,
            false,
            "friend 关 funnel 往返保持"
        );
        // BTreeMap 键序稳定：customer 在 friend 前（字典序）。
        let keys: Vec<&String> = parsed_map.keys().collect();
        assert_eq!(keys, vec!["customer", "friend"]);
    }

    #[test]
    fn default_profile_threshold_overrides_is_none() {
        // M2 零扰动护栏：DEFAULT_PROFILE 不声明阈值覆盖 → gateway 沿用 domain_config
        // 阈值，销售域行为字节等价。换行业才声明 threshold_overrides。
        let p = default_domain_profile("ws-1");
        assert!(p.threshold_overrides.is_none());
    }

    #[test]
    fn default_profile_reviewer_orientation_is_none() {
        // D 零扰动护栏：DEFAULT_PROFILE 不声明评审取向覆盖 → reviewer prompt 的
        // 「评审重点 / 转化平衡」两句保留写死销售取向、字节等价。
        let p = default_domain_profile("ws-1");
        assert!(p.reviewer_orientation.is_none());
    }

    #[test]
    fn default_profile_coverage_initial_signal_reproduces_legacy_rule() {
        // H 等价护栏：completeness degraded fallback 初值规则原写死按维度名分派
        // （capability/deliveryBoundary→verified、caseEvidence/effectClaims→evidence、
        // pricing→恒缺）。规则下放 DomainProfile.coverage_dimensions.initial_signal 后，
        // DEFAULT 五维必须 seed 出与原规则逐项一致的 signal，否则销售域 fallback 行为漂移。
        let p = default_domain_profile("ws-1");
        let sig = |key: &str| -> Option<String> {
            p.coverage_dimensions
                .iter()
                .find(|d| d.key == key)
                .and_then(|d| d.initial_signal.clone())
        };
        assert_eq!(sig("capability").as_deref(), Some("verified"));
        assert_eq!(sig("deliveryBoundary").as_deref(), Some("verified"));
        assert_eq!(sig("caseEvidence").as_deref(), Some("evidence"));
        assert_eq!(sig("effectClaims").as_deref(), Some("evidence"));
        // pricing 原规则恒缺（落 else 分支）→ 不声明 signal。
        assert_eq!(sig("pricing"), None);
    }

    #[test]
    fn profile_thresholds_partial_override_bson_round_trip() {
        // 非销售域只覆盖部分闸（如情感域放宽 pressure、提高 emotional_value 改写线），
        // 其余字段 None 经 BSON 往返保持 None（逐字段独立回落 config）。
        let th = crate::models::ProfileThresholds {
            pressure_risk_block_at: Some(9),
            emotional_value_rewrite_below: Some(8),
            ..Default::default()
        };
        let doc = mongodb::bson::to_document(&th).expect("serialize");
        let parsed: crate::models::ProfileThresholds =
            mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed.pressure_risk_block_at, Some(9));
        assert_eq!(parsed.emotional_value_rewrite_below, Some(8));
        assert_eq!(parsed.fact_risk_block_at, None);
        assert_eq!(parsed.human_like_rewrite_below, None);
        assert_eq!(parsed.product_accuracy_block_below, None);
    }

    // ── H17：记忆维度 seed 等价 ──

    #[test]
    fn default_memory_dimensions_default_is_empty_not_sales() {
        // DomainProfile.memory_dimensions 的 serde 默认是空 Vec（非销售八槽）——
        // 销售记忆维度由 seed 显式填回。这是消费方"空集→回落内置维度"契约的前提：
        // default 不能预埋销售槽，否则换行业 profile 漏配会静默继承销售记忆维度。
        let dims: Vec<crate::models::MemoryDimension> = Vec::default();
        assert!(dims.is_empty());
    }

    #[test]
    fn default_profile_memory_dimensions_match_hardcoded_verbatim() {
        // 锁死 DEFAULT 八个记忆维度的 key + cap + candidate_type，逐字对齐
        // memory.rs::compact_memory_card_with_previous 写死的 limit_extra_array cap 表
        // 与 prompts.rs Reply Agent memoryCandidates[].type 枚举。cap 接线（H17-b）/
        // candidate type 派生（H17-d）后，此测试是"seed 与消费方回落同源"的护栏：
        // 任一处漂移→本测试红。
        let dims = default_memory_dimensions();
        let got: Vec<(&str, usize, bool)> = dims
            .iter()
            .map(|d| (d.key.as_str(), d.cap, d.candidate_type))
            .collect();
        let expected: Vec<(&str, usize, bool)> = vec![
            ("preferences", 8, true),
            ("doNotDo", 10, true),
            ("commitments", 8, true),
            ("objections", 8, true),
            ("openLoops", 8, true),
            ("openQuestions", 8, false),
            ("confirmedFacts", 12, false),
            ("conflicts", 6, false),
        ];
        assert_eq!(got, expected, "DEFAULT 记忆维度 key/cap/candidate_type 必须逐字锁死");
        // coreFacts/recentFacts/deprecatedFacts 是 typed 骨架固定 cap，不得混进 memory_dimensions。
        assert!(
            !dims.iter().any(|d| matches!(
                d.key.as_str(),
                "coreFacts" | "recentFacts" | "deprecatedFacts"
            )),
            "typed 骨架数组不应出现在 memory_dimensions"
        );
    }

    #[test]
    fn memory_candidate_types_guidance_empty_for_default() {
        // DEFAULT 销售八维 + 空维度 → 不追加（Reply Agent prompt 字节等价、销售零扰动）。
        assert_eq!(
            render_memory_candidate_types_guidance(&default_memory_dimensions()),
            ""
        );
        assert_eq!(render_memory_candidate_types_guidance(&[]), "");
    }

    #[test]
    fn memory_candidate_types_guidance_lists_emotional_types() {
        // 情感 profile：candidate_type=true 的维度进合法 type，fact/conflict 固定派生；
        // candidate_type=false 的维度（如纪念日）不作为 candidate type。
        let dims = vec![
            crate::models::MemoryDimension {
                key: "emotionHistory".to_string(),
                display_name: "情绪史".to_string(),
                cap: 12,
                is_core: true,
                prompt_hint: None,
                candidate_type: true,
                date_dimension: false,
            },
            crate::models::MemoryDimension {
                key: "anniversaries".to_string(),
                display_name: "纪念日".to_string(),
                cap: 6,
                is_core: false,
                prompt_hint: None,
                candidate_type: false,
                date_dimension: true,
            },
        ];
        let out = render_memory_candidate_types_guidance(&dims);
        assert!(out.contains("fact"), "fact 固定派生");
        assert!(out.contains("emotionHistory"), "candidate_type=true 进合法集");
        assert!(out.contains("conflict"), "conflict 固定派生");
        assert!(
            !out.contains("anniversaries"),
            "candidate_type=false 不作为候选 type"
        );
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

    #[test]
    fn render_self_check_default_matches_policy_prose_verbatim() {
        // 3A-1c 单一真相源护栏:DEFAULT 渲染的自检段 == 原 policy「关系经营公式（自检）」
        // 4 行逐字(PascalCase 名 + expression + Unicode 减号 −)。policy 剥离公式后由
        // decision.rs 运行时注入本输出,此快照锁住等价。
        let rendered = render_business_formulas_self_check(&default_business_formulas());
        let expected = "- Trust = Credibility + Reliability + Intimacy − SelfOrientation\n\
            - ConversionReadiness = Motivation × ProductFit × Timing × Trust ÷ Friction\n\
            - EmotionalValue = Empathy + Validation + Specificity + AutonomySupport − Pressure\n\
            - NextBestActionScore = RelationshipGain + ConversionProgress + EmotionalValue + ProductFit − PressureRisk − FactRisk";
        assert_eq!(rendered, expected);
        // 空集回落同源:空 slice 渲染 == seed 渲染。
        assert_eq!(render_business_formulas_self_check(&[]), rendered);
    }

    #[test]
    fn render_json_example_default_shape() {
        // reviewer formulaBreakdown 示例:每行 `    "key": "expression"`,最后一行无逗号。
        let rendered = render_business_formulas_json_example(&default_business_formulas());
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "    \"trust\": \"Credibility + Reliability + Intimacy − SelfOrientation\","
        );
        // 末行无逗号。
        assert!(lines[3].ends_with('"'));
        assert!(lines[3].starts_with("    \"nextBestActionScore\":"));
        // 空集回落同源。
        assert_eq!(render_business_formulas_json_example(&[]), rendered);
    }

    #[test]
    fn reviewer_extra_score_lines_default_yields_two_sales_dims() {
        // 第 19 点：DEFAULT 四公式 eval_score_key=[humanLike, conversionReadiness,
        // emotionalValue, relationshipProgress]，排除 5 硬闸后 → 仅 conversionReadiness
        // + relationshipProgress 两行（值同为 6）。
        // 注意：此顺序（conversionReadiness 在前）与改造前 prompt 手写顺序
        // （relationshipProgress 在前）相反——这是 D2-1 审查批准的字节等价豁免点
        // （详见 render_reviewer_extra_score_lines 文档注释），**不是回归**。本断言锁死
        // 当前顺序，防止豁免点进一步漂移；若未来要恢复原序，须连带评估 policy 自检段/
        // formulaBreakdown 的字节锁。
        let rendered = render_reviewer_extra_score_lines(&default_business_formulas());
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "    \"conversionReadiness\": 6,");
        assert_eq!(lines[1], "    \"relationshipProgress\": 6,");
        // 末尾带换行，便于直接拼进 scores 块。
        assert!(rendered.ends_with('\n'));
        // 空集回落 DEFAULT 同源。
        assert_eq!(render_reviewer_extra_score_lines(&[]), rendered);
    }

    #[test]
    fn reviewer_extra_score_lines_empty_for_non_sales_profile() {
        // 非销售 profile：要么不声明 eval_score_key，要么只映射到硬闸维度 → 无额外软维度，
        // scores 块只剩 5 个硬闸维度，不再被强塞销售「成交准备度/关系推进」。
        let formulas = vec![
            BusinessFormula {
                key: "warmth".to_string(),
                expression: "Empathy + Presence".to_string(),
                display_name: "陪伴温度".to_string(),
                eval_score_key: None,
            },
            BusinessFormula {
                key: "comfort".to_string(),
                expression: "Validation + Safety".to_string(),
                display_name: "情绪安抚".to_string(),
                // 映射到硬闸 emotionalValue → 不产生额外软维度行。
                eval_score_key: Some("emotionalValue".to_string()),
            },
        ];
        assert_eq!(render_reviewer_extra_score_lines(&formulas), "");
    }

    #[test]
    fn reviewer_extra_score_lines_dedupes_and_renders_custom_dims() {
        // 自定义非硬闸 eval_score_key 渲染为额外软维度行；重复键去重。
        let formulas = vec![
            BusinessFormula {
                key: "a".to_string(),
                expression: "x".to_string(),
                display_name: "甲".to_string(),
                eval_score_key: Some("companionDepth".to_string()),
            },
            BusinessFormula {
                key: "b".to_string(),
                expression: "y".to_string(),
                display_name: "乙".to_string(),
                eval_score_key: Some("companionDepth".to_string()),
            },
        ];
        let rendered = render_reviewer_extra_score_lines(&formulas);
        assert_eq!(rendered, "    \"companionDepth\": 6,\n");
    }

    #[test]
    fn formula_key_pascal_capitalizes_first() {
        assert_eq!(formula_key_pascal("trust"), "Trust");
        assert_eq!(formula_key_pascal("conversionReadiness"), "ConversionReadiness");
        assert_eq!(formula_key_pascal("nextBestActionScore"), "NextBestActionScore");
        assert_eq!(formula_key_pascal(""), "");
    }

    #[test]
    fn strip_legacy_formula_section_removes_inline_block() {
        // 模拟旧库 policy:公式段夹在两段之间。
        let policy = "## conversationMode\n\n- 必须严格选自 [...].\n\n## 关系经营公式（自检）\n\n\
            - Trust = Credibility + Reliability + Intimacy − SelfOrientation\n\
            - ConversionReadiness = Motivation × ProductFit × Timing × Trust ÷ Friction\n\n\
            ## 表达红线\n\n- 每轮开口前对照最近对话。";
        let (stripped, did) = strip_legacy_formula_self_check_section(policy);
        assert!(did);
        assert!(!stripped.contains("关系经营公式"));
        assert!(!stripped.contains("Trust = Credibility"));
        // 前后两段保留。
        assert!(stripped.contains("## conversationMode"));
        assert!(stripped.contains("## 表达红线"));
        assert!(stripped.contains("每轮开口前"));
        // 幂等:再剥一次无变化、did=false。
        let (again, did2) = strip_legacy_formula_self_check_section(&stripped);
        assert!(!did2);
        assert_eq!(again, stripped);
    }

    #[test]
    fn strip_legacy_formula_section_noop_when_absent() {
        let policy = "## conversationMode\n\n- 选自 [...].\n\n## 表达红线\n\n- 每轮开口。";
        let (out, did) = strip_legacy_formula_self_check_section(policy);
        assert!(!did);
        assert_eq!(out, policy);
    }

    #[test]
    fn strip_then_inject_default_roundtrips_to_original_section() {
        // 单一真相源往返:旧库内联段 == 剥离后 + DEFAULT 注入段。锁「不 bump 版本、
        // 运行时自愈」方案的等价护栏。
        let injected = build_policy_formula_section(&default_business_formulas());
        let expected_section = "## 关系经营公式（自检）\n\n\
            - Trust = Credibility + Reliability + Intimacy − SelfOrientation\n\
            - ConversionReadiness = Motivation × ProductFit × Timing × Trust ÷ Friction\n\
            - EmotionalValue = Empathy + Validation + Specificity + AutonomySupport − Pressure\n\
            - NextBestActionScore = RelationshipGain + ConversionProgress + EmotionalValue + ProductFit − PressureRisk − FactRisk";
        assert_eq!(injected, expected_section);
    }

    /// 第 77 点护栏补盲区：default_playbook「核心公式」段（H12 methodology 中文叙述）
    /// 的 4 个经营公式中文名应与 single-source 的 display_name 一致。playbook 用中文
    /// 运算符 + 多一条「学习深度」非 5 闸公式，不逐字对齐，但 4 个经营公式名必须同步——
    /// 改了 single-source display_name 却忘改 playbook（或反之）时本测试即红。
    #[test]
    fn playbook_core_formula_names_match_single_source_display_names() {
        let playbook = crate::prompts::default_playbook("ws-test", "acct-test");
        let method = playbook.method_prompt;
        for f in default_business_formulas() {
            assert!(
                method.contains(&f.display_name),
                "playbook 核心公式段缺少 single-source 公式中文名「{}」（display_name 漂移）",
                f.display_name
            );
        }
    }

    // ── H9 第 20 点：对话模式判定规则覆盖 ──

    /// 模拟真实 policy 结构：「## 对话模式判定」段后接「## 模式与 5 闸的关系」（红线段）。
    const SAMPLE_POLICY: &str = "## 对话模式判定（必须输出 conversationMode 字段）\n\n\
        2. customer_stage ∈ {方案匹配, 异议处理} → consultative。\n\
        3. 用户问产品/价格 → consultative。\n\n\
        ## 模式与 5 闸的关系\n\n\
        - boundary_protection：严禁承诺真人/上级/转交。\n\n\
        ## 表达红线\n\n- 每轮开口前对照。";

    #[test]
    fn strip_conversation_mode_section_removes_only_decision_block() {
        let (stripped, did) = strip_conversation_mode_section(SAMPLE_POLICY);
        assert!(did);
        // 销售判定条款被剥离。
        assert!(!stripped.contains("customer_stage ∈"));
        assert!(!stripped.contains("用户问产品/价格"));
        // 红线段 + 后续段保留。
        assert!(stripped.contains("## 模式与 5 闸的关系"));
        assert!(stripped.contains("boundary_protection：严禁承诺真人"));
        assert!(stripped.contains("## 表达红线"));
        // 幂等。
        let (again, did2) = strip_conversation_mode_section(&stripped);
        assert!(!did2);
        assert_eq!(again, stripped);
    }

    #[test]
    fn strip_conversation_mode_section_noop_when_absent() {
        let policy = "## 模式与 5 闸的关系\n\n- boundary_protection。";
        let (out, did) = strip_conversation_mode_section(policy);
        assert!(!did);
        assert_eq!(out, policy);
    }

    #[test]
    fn apply_conversation_mode_policy_none_is_byte_identical() {
        // DEFAULT_PROFILE / 老库 = None → 原样返回，销售判定段逐字保留、零变化。
        assert_eq!(apply_conversation_mode_policy(SAMPLE_POLICY, None), SAMPLE_POLICY);
        // 空串 / 纯空白同样视为未覆盖。
        assert_eq!(apply_conversation_mode_policy(SAMPLE_POLICY, Some("   ")), SAMPLE_POLICY);
    }

    #[test]
    fn apply_conversation_mode_policy_replaces_decision_keeps_redline() {
        let override_text = "## 对话模式判定\n\n用户表达情绪 → empathetic_support。";
        let out = apply_conversation_mode_policy(SAMPLE_POLICY, Some(override_text));
        // 本行业规则注入。
        assert!(out.contains("empathetic_support"));
        // 销售判定条款已被替换掉。
        assert!(!out.contains("customer_stage ∈"));
        assert!(!out.contains("用户问产品/价格"));
        // 红线段继续守护（不可配）。
        assert!(out.contains("## 模式与 5 闸的关系"));
        assert!(out.contains("boundary_protection：严禁承诺真人"));
    }

    #[test]
    fn apply_conversation_mode_policy_adds_heading_when_missing() {
        // 运营漏写标题时补锚，保证下游段衔接。
        let out = apply_conversation_mode_policy(SAMPLE_POLICY, Some("用户表达情绪 → empathetic_support。"));
        assert!(out.starts_with(POLICY_CONVERSATION_MODE_SECTION_HEADING));
        assert!(out.contains("empathetic_support"));
        assert!(out.contains("## 模式与 5 闸的关系"));
    }

    // ── H9 修复（问题 A）：conversationMode 枚举列表随 profile 渲染 ──

    /// DEFAULT / 空集 / 恰为默认四模式 → 旧串==新串 → 不替换 → 字节等价（数组形 + 竖线形）。
    #[test]
    fn apply_conversation_mode_enum_list_default_is_byte_identical() {
        // policy 数组形 + task 竖线形各一段，模拟两处写死枚举列表。
        let text = "- conversationMode 必须严格选自 [\"casual_relationship\", \"value_exchange\", \"consultative\", \"boundary_protection\"]。\n\
                    \"conversationMode\": \"casual_relationship | value_exchange | consultative | boundary_protection\",";
        // 空集 → 回落默认四模式 → 字节等价。
        assert_eq!(apply_conversation_mode_enum_list(text, &[]), text);
        // 显式默认四模式 → 同样字节等价。
        let default_modes = crate::agent::runtime::default_conversation_modes();
        assert_eq!(apply_conversation_mode_enum_list(text, &default_modes), text);
    }

    /// 非销售模式集 → 数组形与竖线形两处枚举列表都被替换为本行业模式。
    #[test]
    fn apply_conversation_mode_enum_list_replaces_both_forms_for_custom_modes() {
        let text = "- conversationMode 必须严格选自 [\"casual_relationship\", \"value_exchange\", \"consultative\", \"boundary_protection\"]。\n\
                    \"conversationMode\": \"casual_relationship | value_exchange | consultative | boundary_protection\",";
        let modes = vec![
            "intimate_companion".to_string(),
            "casual_relationship".to_string(),
            "boundary_protection".to_string(),
        ];
        let out = apply_conversation_mode_enum_list(text, &modes);
        // 数组形替换为本行业三模式。
        assert!(
            out.contains("[\"intimate_companion\", \"casual_relationship\", \"boundary_protection\"]"),
            "数组形未按 profile 替换：{out}"
        );
        // 竖线形替换为本行业三模式。
        assert!(
            out.contains("intimate_companion | casual_relationship | boundary_protection"),
            "竖线形未按 profile 替换：{out}"
        );
        // 写死的销售四模式列表不再残留（消除矛盾指令）。
        assert!(!out.contains("\"value_exchange\", \"consultative\""), "销售数组形残留：{out}");
        assert!(!out.contains("value_exchange | consultative"), "销售竖线形残留：{out}");
    }

    /// 只替换精确的枚举列表子串，不触碰各模式的散文描述（boundary_protection 红线段保留）。
    #[test]
    fn apply_conversation_mode_enum_list_keeps_mode_prose_descriptions() {
        // 模拟「## 模式与 5 闸的关系」段里逐模式散文 + 红线（不含枚举列表子串形态）。
        let prose = "- **boundary_protection**：禁止任何主动话术；严禁承诺真人。\n\
                     - **consultative**：所有产品声明必须由 verified_chunks 支撑。";
        let modes = vec!["intimate_companion".to_string(), "boundary_protection".to_string()];
        let out = apply_conversation_mode_enum_list(prose, &modes);
        // 散文逐字保留（无数组/竖线枚举列表子串可匹配 → 不动）。
        assert_eq!(out, prose, "散文描述段不应被触碰");
    }

    // ── D：reviewer 评审取向随 profile 渲染（去销售取向）──

    const SAMPLE_REVIEW_SYSTEM: &str = "评分范围 0-10，risk 越高越危险。\n\
        评审重点：事实准确、像真人微信、情绪价值、低压推进、产品知识一致性、没有操控营销。\n\
        判 HumanLikeScore 时……";
    const SAMPLE_REVIEW_USER: &str = "评审原则：\n\
        - 转化平衡：既允许适度推进，也不能伤害信任。\n\
        - 禁止虚假稀缺、恐惧营销、编造案例、编造价格、编造承诺。";

    /// DEFAULT / None / 空白覆盖 → reviewer system「评审重点」行逐字保留、字节等价。
    #[test]
    fn apply_reviewer_review_focus_none_is_byte_identical() {
        assert_eq!(apply_reviewer_review_focus(SAMPLE_REVIEW_SYSTEM, None), SAMPLE_REVIEW_SYSTEM);
        assert_eq!(apply_reviewer_review_focus(SAMPLE_REVIEW_SYSTEM, Some("   ")), SAMPLE_REVIEW_SYSTEM);
        // 覆盖文本恰等于写死销售取向 → 也不替换（幂等、无扰动）。
        assert_eq!(
            apply_reviewer_review_focus(SAMPLE_REVIEW_SYSTEM, Some(DEFAULT_REVIEWER_REVIEW_FOCUS)),
            SAMPLE_REVIEW_SYSTEM
        );
    }

    /// Some → 替换「评审重点：」冒号后的销售取向描述为本域取向，保留中性标签前缀。
    #[test]
    fn apply_reviewer_review_focus_replaces_sales_orientation() {
        let out = apply_reviewer_review_focus(
            SAMPLE_REVIEW_SYSTEM,
            Some("真诚陪伴、像真人微信、情绪价值、尊重边界、不越界承诺。"),
        );
        assert!(out.contains("评审重点：真诚陪伴、像真人微信、情绪价值、尊重边界、不越界承诺。"));
        // 销售漏斗措辞已不再残留。
        assert!(!out.contains("低压推进"), "销售取向残留：{out}");
        assert!(!out.contains("产品知识一致性"), "销售取向残留：{out}");
        // 其余写死内容（前后行）逐字保留。
        assert!(out.contains("评分范围 0-10，risk 越高越危险。"));
        assert!(out.contains("判 HumanLikeScore 时……"));
    }

    /// 锚找不到（运营改写过 prompt）→ 原样返回，不强插污染。
    #[test]
    fn apply_reviewer_review_focus_no_anchor_is_noop() {
        let custom = "评分范围 0-10。\n判 HumanLikeScore 时……";
        assert_eq!(apply_reviewer_review_focus(custom, Some("本域取向。")), custom);
    }

    /// DEFAULT / None / 空白覆盖 → reviewer user「转化平衡」条逐字保留、字节等价。
    #[test]
    fn apply_reviewer_balance_principle_none_is_byte_identical() {
        assert_eq!(apply_reviewer_balance_principle(SAMPLE_REVIEW_USER, None), SAMPLE_REVIEW_USER);
        assert_eq!(apply_reviewer_balance_principle(SAMPLE_REVIEW_USER, Some("  ")), SAMPLE_REVIEW_USER);
        assert_eq!(
            apply_reviewer_balance_principle(SAMPLE_REVIEW_USER, Some(DEFAULT_REVIEWER_BALANCE_PRINCIPLE)),
            SAMPLE_REVIEW_USER
        );
    }

    /// Some → 整条「转化平衡：…」替换为本域取向（含标签，去掉销售「转化」语义）。
    #[test]
    fn apply_reviewer_balance_principle_replaces_conversion_framing() {
        let out = apply_reviewer_balance_principle(
            SAMPLE_REVIEW_USER,
            Some("关系平衡：既允许真诚靠近，也不能制造依赖或越界。"),
        );
        assert!(out.contains("- 关系平衡：既允许真诚靠近，也不能制造依赖或越界。"));
        assert!(!out.contains("转化平衡"), "销售「转化」标签残留：{out}");
        assert!(!out.contains("适度推进"), "销售取向残留：{out}");
        // 同段其余写死原则逐字保留。
        assert!(out.contains("禁止虚假稀缺、恐惧营销、编造案例、编造价格、编造承诺。"));
    }

    // ── A/T1：mode_gate_policy 模式-闸说明段随 profile 替换 ──

    /// None / 空白覆盖 → 原样返回（字节等价）。
    #[test]
    fn apply_mode_gate_policy_none_returns_unchanged() {
        let s = format!("前\n{}\n后", crate::prompts::DEFAULT_MODE_GATE_POLICY);
        assert_eq!(apply_mode_gate_policy(&s, None), s);
        assert_eq!(apply_mode_gate_policy(&s, Some("   ")), s);
        // 覆盖文本恰等于写死销售说明 → 幂等不替换。
        assert_eq!(
            apply_mode_gate_policy(&s, Some(crate::prompts::DEFAULT_MODE_GATE_POLICY)),
            s
        );
    }

    /// Some → 把模式-闸说明锚替换为本域说明，销售锚不再残留。
    #[test]
    fn apply_mode_gate_policy_some_replaces_anchor() {
        let s = format!("前\n{}\n后", crate::prompts::DEFAULT_MODE_GATE_POLICY);
        let out = apply_mode_gate_policy(&s, Some("情感域模式说明"));
        assert!(out.contains("情感域模式说明"));
        assert!(!out.contains(crate::prompts::DEFAULT_MODE_GATE_POLICY));
        // 锚以外的前后文逐字保留。
        assert!(out.contains("前\n"));
        assert!(out.contains("\n后"));
    }

    /// 锚找不到（运营改写过 prompt）→ 原样返回，不强插污染。
    #[test]
    fn apply_mode_gate_policy_no_anchor_is_noop() {
        let custom = "前文\n## 别的标题\nXXX\n后文";
        assert_eq!(apply_mode_gate_policy(custom, Some("本域说明")), custom);
    }

    // ── T3：reviewer 软闸打分锚点 few-shot 段随 profile 替换 ──

    /// None / 空白覆盖 → 原样返回（字节等价）。
    #[test]
    fn apply_reviewer_fewshot_none_unchanged() {
        let s = format!("前\n{}\n后", crate::prompts::DEFAULT_REVIEWER_FEWSHOT);
        assert_eq!(apply_reviewer_fewshot(&s, None), s);
        assert_eq!(apply_reviewer_fewshot(&s, Some("   ")), s);
        // 覆盖文本恰等于写死销售 few-shot → 幂等不替换。
        assert_eq!(
            apply_reviewer_fewshot(&s, Some(crate::prompts::DEFAULT_REVIEWER_FEWSHOT)),
            s
        );
    }

    /// Some → 把 few-shot 锚替换为本域打分锚点，销售逼单锚不再残留。
    #[test]
    fn apply_reviewer_fewshot_some_replaces() {
        let s = format!("前\n{}\n后", crate::prompts::DEFAULT_REVIEWER_FEWSHOT);
        let out = apply_reviewer_fewshot(&s, Some("情感域打分锚点"));
        assert!(out.contains("情感域打分锚点"));
        assert!(!out.contains(crate::prompts::DEFAULT_REVIEWER_FEWSHOT));
        // 锚以外的前后文逐字保留。
        assert!(out.contains("前\n"));
        assert!(out.contains("\n后"));
    }

    /// 锚找不到（运营改写过 prompt）→ 原样返回，不强插污染。
    #[test]
    fn apply_reviewer_fewshot_no_anchor_is_noop() {
        let custom = "前文\n别的打分说明\n后文";
        assert_eq!(apply_reviewer_fewshot(custom, Some("本域锚点")), custom);
    }

    /// 两字段独立回落：只覆盖其一时，另一处保持写死。
    #[test]
    fn apply_reviewer_orientation_fields_fall_back_independently() {
        // 只覆盖 balance_principle，review_focus=None → system 仍为销售取向。
        assert_eq!(apply_reviewer_review_focus(SAMPLE_REVIEW_SYSTEM, None), SAMPLE_REVIEW_SYSTEM);
        let user_out = apply_reviewer_balance_principle(SAMPLE_REVIEW_USER, Some("关系平衡：真诚靠近、不制造依赖。"));
        assert!(user_out.contains("关系平衡："));
    }

    // ── I：answeringMode 三档释义/标签随 profile 渲染 ──

    /// DEFAULT（None）→ 三档释义 bullet 与改造前写死 prompt 字面量逐字一致（字节等价）。
    #[test]
    fn render_answering_mode_rules_none_is_byte_equivalent() {
        let got = render_answering_mode_rules(None);
        let expected = "- relationship_only: 没有足够 verified 知识支撑产品/服务事实，只能关系维护、澄清需求、收集信息。\n\
            - product_safe: 可回答部分产品/服务能力，但报价、案例、效果或交付边界仍不足。\n\
            - fully_supported: 能力、边界、证据类内容足够支撑常见产品事实问题。";
        assert_eq!(got, expected);
    }

    /// 换行业声明本域释义 → 三档 key 恒定、释义替换；缺的档逐档回落写死销售释义。
    #[test]
    fn render_answering_mode_rules_overrides_per_state() {
        use crate::models::{AnsweringModeDescriptor, AnsweringModeProfile};
        let profile = AnsweringModeProfile {
            relationship_only: Some(AnsweringModeDescriptor {
                rule: Some("只能纯倾听陪伴，不触及任何专业判断。".to_string()),
                label: None,
            }),
            product_safe: None, // 逐档回落
            fully_supported: Some(AnsweringModeDescriptor {
                rule: Some("可在已验证边界内深入交流。".to_string()),
                label: None,
            }),
        };
        let got = render_answering_mode_rules(Some(&profile));
        assert!(got.contains("- relationship_only: 只能纯倾听陪伴，不触及任何专业判断。"));
        // product_safe 未声明 → 回落写死销售释义。
        assert!(got.contains("- product_safe: 可回答部分产品/服务能力，但报价、案例、效果或交付边界仍不足。"));
        assert!(got.contains("- fully_supported: 可在已验证边界内深入交流。"));
        // key 恒定（认知阶梯），三档齐全。
        assert_eq!(got.matches("- relationship_only:").count(), 1);
        assert_eq!(got.matches("- product_safe:").count(), 1);
        assert_eq!(got.matches("- fully_supported:").count(), 1);
    }

    /// 标签解析：DEFAULT/None → 内置销售标签；Some 逐档覆盖、缺的档回落。
    #[test]
    fn answering_mode_labels_fall_back_per_state() {
        use crate::models::{AnsweringModeDescriptor, AnsweringModeProfile};
        // None → 三档写死销售标签。
        assert_eq!(
            answering_mode_labels(None),
            ("仅关系维护".to_string(), "可安全讲产品".to_string(), "完全支撑".to_string())
        );
        let profile = AnsweringModeProfile {
            relationship_only: Some(AnsweringModeDescriptor { rule: None, label: Some("纯陪伴倾听".to_string()) }),
            product_safe: None,
            fully_supported: None,
        };
        let (r, p, f) = answering_mode_labels(Some(&profile));
        assert_eq!(r, "纯陪伴倾听");
        assert_eq!(p, "可安全讲产品"); // 逐档回落
        assert_eq!(f, "完全支撑");
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
        let cache = DomainProfileCache::new();
        let mut seeded = default_domain_profile("ws-A");
        seeded.display_name = "行业A".to_string();
        seeded.profile_id = "profile-a".to_string();
        cache.seed_for_test(seeded);

        let fallback = cache.lookup_or_default("ws-B");
        assert_eq!(fallback.profile_id, DEFAULT_PROFILE_ID);
        assert_eq!(fallback.workspace_id, "ws-B");
        let hit = cache.lookup_or_default("ws-A");
        assert_eq!(hit.profile_id, "profile-a");
        assert_eq!(hit.display_name, "行业A");
    }

    // ── G1：render_decision_dimensions_guidance 维度指引 ──

    #[test]
    fn decision_dimensions_guidance_empty_for_default_sales() {
        // DEFAULT 销售域只有 customer_stage/intent_level（均为 typed）→ 过滤后空 →
        // 空串、Reply Agent / 初始画像 prompt 字节不变、销售零扰动。
        let p = default_domain_profile("ws-1");
        assert_eq!(render_decision_dimensions_guidance(&p.profile_dimensions), "");
        assert_eq!(render_decision_dimensions_guidance(&[]), "");
    }

    #[test]
    fn decision_dimensions_guidance_lists_non_sales_dimensions() {
        // 非销售「参与决策」维度 → 进指引 + 提示走 domainSignals 容器；
        // participates_in_decision=false 的维度不进；销售 typed 两维即便在场也不进。
        let dims = vec![
            ProfileDimension {
                kind: "customer_stage".to_string(),
                display_name: "客户阶段".to_string(),
                participates_in_decision: true,
                description: String::new(),
            },
            ProfileDimension {
                kind: "purchase_lifecycle".to_string(),
                display_name: "购买生命周期".to_string(),
                participates_in_decision: true,
                description: "未购买/已购买/售后期/复购期。".to_string(),
            },
            ProfileDimension {
                kind: "anniversaries".to_string(),
                display_name: "纪念日".to_string(),
                participates_in_decision: false,
                description: String::new(),
            },
        ];
        let out = render_decision_dimensions_guidance(&dims);
        assert!(out.contains("domainSignals"), "应告知 LLM 走 domainSignals 容器");
        assert!(out.contains("purchase_lifecycle"), "非销售参与决策维度进指引");
        assert!(out.contains("购买生命周期"), "带 display_name");
        assert!(out.contains("未购买/已购买"), "带 description 语义");
        assert!(
            !out.contains("customer_stage"),
            "销售 typed 维度不进 domainSignals 指引（仍走 typed schema 键）"
        );
        assert!(
            !out.contains("anniversaries"),
            "participates_in_decision=false 的维度不进决策指引"
        );
    }
}
