//! R5 动态发现线底座 —— 三族异族硬门（R5.0.1）+ 轨迹级裁判（R5.2）。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R5.0.1 / R5.2。
//!
//! ## 定位（用户拍板「建底座不进门」）
//! 动态发现线（roleplayer 演客户 × 真实博弈 × 轨迹评判）用于**发现**固定脚本测不出的
//! 对话质量/抗刁难短板，进 ledger 观测 + 软门，**不进 PR 合并门**（成本+flaky）。本模块
//! 提供两块底座：
//! - **R5.0.1 三族异族硬门**：roleplayer/agent/judge 必须三个不同 provider 家族（不同
//!   端点 + 不同模型基座），同源 → panic。防「三角色其实同一个模型自说自话」的伪多样性。
//! - **R5.2 轨迹裁判**：评**整段对话**而非单条（信任是否累积/关系是否前进/全程红线是否守住），
//!   维度接 R1.1 `build_judge_rubric` 派生（**不在轨迹层重新硬编码销售世界观**）。
//!   **校准未达标 → 只进 ledger 观测，绝不进任何软门**（spec R5.0 铁律③：trajectory judge
//!   方差大、最易「看着很懂其实在编」，投用前必须有人工金标 trajectory 校准——金标留后续，
//!   现阶段 trajectory 分只观测）。

#![allow(dead_code)]

use std::sync::Arc;

use wechatagent::llm::{LlmClient, LlmProvider};

use crate::common::judge::JudgeRubric;
use crate::common::roleplayer::{DialogueTurn, Speaker};

// ════════════════════════════════════════════════════════════════════════════
// R5.0.1 三族异族硬门
// ════════════════════════════════════════════════════════════════════════════

/// 一个角色的 provider 指纹（从测试构造 client 的 env 派生——不读 LlmClient 私有字段，
/// 不碰生产代码）。`family` 取「端点 host + 模型基座前缀」，用于判同源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFingerprint {
    pub role: String,
    pub base_url: String,
    pub model: String,
}

impl ProviderFingerprint {
    /// 模型家族标识：端点 host + 模型名的「家族前缀」（去掉版本号尾巴）。
    /// 例：`rsxermu666.cn|claude` / `integrate.api.nvidia.com|meta` / `rsxermu666.cn|gpt`。
    /// 同 host + 同基座 = 同族。
    pub fn family(&self) -> String {
        let host = self
            .base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        // 模型基座：取第一个 `/` 前的厂商段，或 `-`/`.` 前的系列名。
        let base = self.model.to_lowercase();
        let vendor = if let Some((v, _)) = base.split_once('/') {
            v.to_string() // meta/llama-3.3 → meta
        } else {
            // claude-opus-4-8 → claude；gpt-5.4 → gpt；mimo-v2.5 → mimo
            base.split(['-', '.']).next().unwrap_or("").to_string()
        };
        format!("{host}|{vendor}")
    }
}

/// 从 env 读三角色指纹（与各自 client 构造器同 env 约定）。
pub fn read_role_fingerprints() -> Vec<ProviderFingerprint> {
    let agent = ProviderFingerprint {
        role: "agent".to_string(),
        base_url: std::env::var("REAL_LLM_BASE_URL")
            .unwrap_or_else(|_| "https://rsxermu666.cn".to_string()),
        model: std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "claude-opus-4-8".to_string()),
    };
    let roleplayer = ProviderFingerprint {
        role: "roleplayer".to_string(),
        base_url: std::env::var("ROLEPLAYER_BASE_URL")
            .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string()),
        model: std::env::var("ROLEPLAYER_MODEL")
            .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string()),
    };
    let judge = ProviderFingerprint {
        role: "judge".to_string(),
        base_url: std::env::var("REAL_LLM_JUDGE_BASE_URL")
            .unwrap_or_else(|_| "https://rsxermu666.cn/v1".to_string()),
        model: std::env::var("REAL_LLM_JUDGE_MODEL").unwrap_or_else(|_| "gpt-5.4".to_string()),
    };
    vec![agent, roleplayer, judge]
}

/// R5.0.1 硬门：断言 agent / roleplayer / judge 三方 provider 家族**两两不同**。
/// 同源（任意两角色同族）→ panic（job 红）。返回三方指纹供写 report。
///
/// **为什么硬门**：动态博弈若三角色实为同一模型，等于模型自问自答自评——发现不了真盲区，
/// 是伪多样性。spec R5.0.1 要求「同源 → job 红」（非仅观测）。
pub fn assert_three_families_distinct() -> Vec<ProviderFingerprint> {
    let fps = read_role_fingerprints();
    for i in 0..fps.len() {
        for j in (i + 1)..fps.len() {
            assert_ne!(
                fps[i].family(),
                fps[j].family(),
                "R5.0.1 异族硬门违反：{} 与 {} 同 provider 家族（{}）——三角色须三个不同模型家族，\
                 否则动态博弈是模型自说自话的伪多样性。配置三方不同端点/基座。",
                fps[i].role,
                fps[j].role,
                fps[i].family()
            );
        }
    }
    eprintln!(
        "[R5.0.1] 三族异族 ✓ agent={} roleplayer={} judge={}",
        fps[0].family(),
        fps[1].family(),
        fps[2].family()
    );
    fps
}

// ════════════════════════════════════════════════════════════════════════════
// R5.2 轨迹级裁判（只进 ledger，不进软门——校准未达标前）
// ════════════════════════════════════════════════════════════════════════════

/// 轨迹裁判的一次评分（整段对话）。
#[derive(Debug, Clone)]
pub struct TrajectoryVerdict {
    /// 各 arc 维度分（overall_progress/pressure_arc/consistency_arc/emotional_attunement_arc/redlineHeld/trustTrajectory/overall）。
    pub scores: std::collections::HashMap<String, i64>,
    /// 一句总评。
    pub verdict: String,
    /// 是否解析成功（失败时 scores 空、verdict 带错误）。
    pub ok: bool,
}

/// 把对话历史渲染成轨迹裁判的输入（完整多轮，标注客户/助理）。
fn render_full_dialogue(history: &[DialogueTurn]) -> String {
    let mut out = String::from("完整对话（按时间顺序，客户与助理交替）：\n\n");
    for (i, turn) in history.iter().enumerate() {
        let who = match turn.speaker {
            Speaker::Customer => "客户",
            Speaker::Agent => "助理",
        };
        out.push_str(&format!("第{}轮 {who}：{}\n", i + 1, turn.text));
    }
    out
}

/// R5.2 轨迹裁判：评整段对话。**只返回分数供写 ledger 观测,调用方绝不可拿它做硬断言/软门**
/// （校准未达标——无人工金标 trajectory,spec R5.0 铁律③；阶段4 才补校准）。
///
/// 阶段3 重构：薄委托对话级内核——用 `conversation_rubric_from_base` 出 arc rubric,
/// `run_graded_samples`(K=1, ObserveOnly) 打分。维度从旧 6 维迁到 arc 7 维（并集:含
/// redlineHeld/trustTrajectory 轨迹特有维 + spec 四 arc 维 + overall）。
/// judge 调用失败/解析失败 → ok=false（不 panic——动态线不因 judge 抖动染红）。
pub async fn judge_trajectory(
    judge: &dyn LlmProvider,
    rubric: &JudgeRubric,
    history: &[DialogueTurn],
) -> TrajectoryVerdict {
    use crate::common::judge::{
        build_judge_user_with_context, conversation_rubric_from_base, run_graded_samples,
        JudgeContext, JudgeGate, CONVERSATION_DIMS,
    };
    let conv = conversation_rubric_from_base(rubric);
    let transcript = render_full_dialogue(history);
    let ctx = JudgeContext { transcript: Some(transcript), ..Default::default() };
    let user = build_judge_user_with_context(
        "轨迹裁判", "（对话级总评,无单条 inbound）", "（见上方完整对话）", &ctx,
    );
    let dims: Vec<String> = CONVERSATION_DIMS.iter().map(|s| s.to_string()).collect();
    // ObserveOnly：动态线轨迹裁判只观测,judge 掉线/全失败 → None（不 panic）。
    match run_graded_samples(judge, &conv.system, &user, &dims, "轨迹裁判", 1, JudgeGate::ObserveOnly).await {
        Some(outcome) => {
            let verdict = format!("arc 总评 medians={:?}", outcome.medians);
            let ok = !outcome.medians.is_empty();
            TrajectoryVerdict { scores: outcome.medians, verdict, ok }
        }
        None => TrajectoryVerdict {
            scores: std::collections::HashMap::new(),
            verdict: "轨迹裁判未出分(env 未设/裁判全掉线,只观测)".to_string(),
            ok: false,
        },
    }
}

/// 便捷构造轨迹裁判 client（异族——优先 REAL_LLM_JUDGE_*，缺 key → None）。
pub fn trajectory_judge_client() -> Option<Arc<LlmClient>> {
    use wechatagent::llm::LlmFormat;
    let api_key = std::env::var("REAL_LLM_JUDGE_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_JUDGE_BASE_URL")
        .unwrap_or_else(|_| "https://rsxermu666.cn/v1".to_string());
    let model = std::env::var("REAL_LLM_JUDGE_MODEL").unwrap_or_else(|_| "gpt-5.4".to_string());
    let fmt = match std::env::var("REAL_LLM_JUDGE_FORMAT").ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    };
    LlmClient::with_format(base_url, api_key, model, fmt, 180, 6, 2500)
        .ok()
        .map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认配置（agent=rsxermu/claude, roleplayer=nvidia/meta, judge=rsxermu/gpt）三族不同。
    #[test]
    fn default_three_families_are_distinct() {
        // 不设任何 env → 用默认值。agent=rsxermu|claude, roleplayer=nvidia|meta, judge=rsxermu|gpt。
        // 注意：agent 与 judge 同 host(rsxermu) 但不同 vendor(claude vs gpt) → family 不同。
        let fps = read_role_fingerprints();
        let fam: Vec<String> = fps.iter().map(|f| f.family()).collect();
        assert_eq!(fam.len(), 3);
        // 两两不同。
        assert_ne!(fam[0], fam[1], "agent vs roleplayer 应异族");
        assert_ne!(fam[0], fam[2], "agent vs judge 应异族（同 host 不同 vendor）");
        assert_ne!(fam[1], fam[2], "roleplayer vs judge 应异族");
    }

    #[test]
    fn family_distinguishes_vendor_on_same_host() {
        let claude = ProviderFingerprint {
            role: "a".into(),
            base_url: "https://rsxermu666.cn".into(),
            model: "claude-opus-4-8".into(),
        };
        let gpt = ProviderFingerprint {
            role: "j".into(),
            base_url: "https://rsxermu666.cn/v1".into(),
            model: "gpt-5.4".into(),
        };
        // 同 host 不同 vendor → 不同 family。
        assert_ne!(claude.family(), gpt.family());
        assert_eq!(claude.family(), "rsxermu666.cn|claude");
        assert_eq!(gpt.family(), "rsxermu666.cn|gpt");
    }

    #[test]
    fn family_detects_same_source() {
        // 两个角色完全同 provider → 同 family（应被硬门 panic 捕获）。
        let a = ProviderFingerprint {
            role: "a".into(),
            base_url: "https://rsxermu666.cn".into(),
            model: "claude-opus-4-8".into(),
        };
        let b = ProviderFingerprint {
            role: "b".into(),
            base_url: "https://rsxermu666.cn".into(),
            model: "claude-opus-4-8".into(),
        };
        assert_eq!(a.family(), b.family(), "同 provider 应判同族");
    }

    #[test]
    fn nvidia_meta_family() {
        let rp = ProviderFingerprint {
            role: "rp".into(),
            base_url: "https://integrate.api.nvidia.com/v1".into(),
            model: "meta/llama-3.3-70b-instruct".into(),
        };
        assert_eq!(rp.family(), "integrate.api.nvidia.com|meta");
    }

    #[tokio::test]
    async fn judge_trajectory_uses_arc_dims_and_skips_without_env() {
        use crate::common::judge::build_judge_rubric;
        use crate::common::roleplayer::{DialogueTurn, Speaker};
        // 未设 REAL_LLM_JUDGE → 底层 run_graded_samples 返 None → ok=false（不 panic,只观测）。
        let prev = std::env::var("REAL_LLM_JUDGE").ok();
        std::env::remove_var("REAL_LLM_JUDGE");
        struct Boom;
        #[async_trait::async_trait]
        impl wechatagent::llm::LlmProvider for Boom {
            async fn generate_json(&self, _: &str, _: &str) -> wechatagent::error::AppResult<serde_json::Value> { panic!("env 未设不应调用") }
            async fn generate_json_with_usage(&self, _: &str, _: &str) -> wechatagent::error::AppResult<wechatagent::llm::LlmJsonResult> { panic!("env 未设不应调用") }
        }
        let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
        let history = vec![
            DialogueTurn { speaker: Speaker::Customer, text: "你好".into() },
            DialogueTurn { speaker: Speaker::Agent, text: "在的,您说".into() },
        ];
        let v = judge_trajectory(&Boom, &rubric, &history).await;
        assert!(!v.ok, "env 未设时轨迹裁判应 ok=false(只观测,不 panic)");
        assert!(v.scores.is_empty(), "未出分时 scores 应空");
        if let Some(p) = prev { std::env::set_var("REAL_LLM_JUDGE", p); }
    }
}
