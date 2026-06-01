//! `real_llm_knowledge_quality` —— 知识库**内容质量**多轮「测-优」迭代套件（Q 系列）。
//!
//! 与 `real_llm_knowledge.rs`（K1–K11，**红线/形状**套件）互补：K 系列证明真模型
//! 在生产闸门下「不破红线、shape 稳定」；本套件在其之上叠加**内容质量度量**——
//! 用同一真模型作 **LLM-as-judge**（0–10 打分）量化每个 LLM 能力的专业度，定位
//! 最低分短板，驱动「测 → 读分 → 修生产代码 → 再测」的多轮收敛闭环。
//!
//! ## 两层判据（每个 Q 测试都同时跑）
//! 1. **硬命中红线**（确定性 `assert!`）：与 K 系列同源——cite⊆seed、抽取/vision 恒
//!    draft+needs_review、对话只产 proposal（计数不变 + verified=0）、审计只读、修复
//!    不落库、标签双数组、关键事实 token 命中。**破则 fail**。
//! 2. **LLM-judge 打分**（`judge_quality`）：真模型按维度 0–10 评 grounding/accuracy/
//!    completeness 等，`overall < MIN_QUALITY_FLOOR(6.0)` 即 fail（= 未达专业基线，
//!    驱动修生产 prompt/检索/抽取逻辑，**绝不放水断言**）。`TARGET_QUALITY(7.0)` 仅
//!    记录、驱动下一轮选短板。
//!
//! ## 三维交叉覆盖
//! - 类型轴：`quality_corpus()` 一次 seed 跨齐 **9 个 wiki_type**（thesis/synthesis/
//!   methodology/finding/comparison/concept/entity/source/query）× **4 个 chunk_type**
//!   （product_fact/style_template/peer_case/negative_example）的真实业务知识库。
//! - 场景轴：用例覆盖 price/trust 等 objection_type、create_chunk 等 intent。
//! - 能力轴：Q1–Q7 扫 7 个 LLM 驱动能力（检索/文章抽取/vision/对话/审计/修复/打标）。
//!
//! ## 红线（与 K 系列同口径，全程不破）
//! - MCP 永远空 wiremock 桩（绝不真发微信）；密钥零泄漏（只 env 读、judge prompt/
//!   日志不打 key）；抽取/vision 落库恒 draft+needs_review；env-gated/瞬时不可达 skip
//!   不 panic；修生产代码不迁就测试；闸门只严不松。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=mimo-v2.5-pro \
//!   cargo test --test real_llm_knowledge_quality -- --ignored --nocapture
//! ```
//! CI 日志可 `grep '\[QUALITY\]'` 拿到每能力/场景的 judge 分，驱动定位短板。

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppResult;
use wechatagent::llm::{LlmClient, LlmProvider};
use wechatagent::models::{LlmProviderConfig, OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::ext_knowledge::{
    build_operation_knowledge_completeness, chat_turn, extract_operation_knowledge_tags,
    import_operation_knowledge_apply_image, import_operation_knowledge_preview,
    propose_chunk_repair, ChatTurnRequest, ExtractKnowledgeTagsRequest, ImportApplyImageRequest,
    OperationKnowledgeImportRequest,
};

use crate::common::TestApp;
use wiremock::MockServer;

// ── env-gated 真实 provider 构造（与 real_llm_knowledge.rs 同形；测试 crate 各自
//    独立编译，fixture 不跨文件共享，故本文件自带一份）──────────────────────────

fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let client =
        LlmClient::new(base_url, api_key, model, 180, 3, 1500).expect("构造真实 LlmClient");
    Some(Arc::new(client))
}

macro_rules! require_real_llm {
    () => {{
        match real_llm_from_env() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过真实大模型知识库质量套件");
                return;
            }
        }
    }};
}

/// 解包 `AppResult<T>`；遇真模型上游瞬时不可达（`LlmUnavailable`）打印 skip 并
/// `return`（不 panic、不算质量失败——模型没产出任何输出，无内容可评质量）。
macro_rules! unwrap_or_skip_transient {
    ($result:expr, $what:expr) => {{
        match $result {
            Ok(value) => value,
            Err(wechatagent::error::AppError::LlmUnavailable {
                kind,
                retry_count,
                ..
            }) => {
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），\
                     按计划「真模型抖动有限重试+跳过」处理，不算质量失败",
                    $what
                );
                return;
            }
            Err(other) => panic!("{}：{other}", $what),
        }
    }};
}

async fn dummy_mcp_server() -> MockServer {
    MockServer::start().await
}

// ── 质量门常量 ────────────────────────────────────────────────────────────────

/// 专业基线：judge overall 低于此即硬 fail（= 未达生产可用，驱动修生产代码）。
const MIN_QUALITY_FLOOR: f64 = 6.0;
/// 收敛目标：达此即专业生产级；仅记录、驱动下一轮选最低分短板，不作硬断言。
const TARGET_QUALITY: f64 = 7.0;
/// 抽取召回基线：每个 split 的平均「参考事实原子单元」召回率低于此即硬 fail。
/// 确定性度量（零模型方差、可复现），是抗过拟合的主武器。
const MIN_RECALL_FLOOR: f64 = 0.6;
/// 泛化差距门（核心抗作弊）：train 与 holdout 平均召回差 > 此即判过拟合硬 fail。
/// prompt 若被特调去适配 train 文档，train 召回高、holdout 召回塌，gap 拉大即暴露。
const MAX_GENERALIZATION_GAP: f64 = 0.18;
/// judge 多次运行取中位的次数（控真模型方差）；单次瞬时抖动跳过，全失败则 skip。
const JUDGE_RUNS: usize = 3;
/// 校准锚最小分差：judge 对「通用 good vs bad 样本」须拉开的最小 overall 差，
/// 拉不开视为裁判漂移/失灵 → 调用方 skip（不把裁判问题算到被测对象头上）。
const CALIB_MIN_GAP: f64 = 2.0;

// ── LLM-as-judge ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct JudgeScore {
    overall: f64,
    dims: BTreeMap<String, f64>,
    reasons: String,
}

/// 通用锚定评分量表（behavior-anchored rating scale）。内嵌进 judge system prompt，
/// 给每个分数挡位一个**与具体文档无关**的行为锚，把「凭感觉打分」收敛成「对照锚位判级」，
/// 显著压低同模型多次打分的方差，也避免裁判对不同题材给分尺度漂移。
/// **刻意不含任何业务/文档特定内容**——锚的是「输出与参考事实的关系」这一通用维度。
const JUDGE_RUBRIC: &str = "通用锚定量表（每维按下列行为锚定级，只看『输出相对参考事实』的质量，与题材无关）：\n\
- 10：完全 grounded 于参考事实，关键信息零遗漏、零捏造，表述精准可直接对客；\n\
- 8：grounded，覆盖绝大多数关键信息，仅个别非关键细节欠缺，无事实错误；\n\
- 6（及格基线）：主体 grounded，覆盖主要信息但有可察觉的遗漏或粗糙，无硬伤性捏造；\n\
- 4：部分 grounded，存在明显遗漏或表述偏差，或夹带少量未经支撑的内容；\n\
- 2：大面积偏离参考事实，关键信息缺失，或出现凭空捏造；\n\
- 0：与参考事实无关、整体捏造或答非所问。\n\
打分一律对照上述锚位定级，不要凭整体印象浮动；任一短板维度都要把 overall 拉到该维水平附近。";

/// 解析 judge 返回的 JSON 为 JudgeScore（容错：overall 缺失时取各维均值）。
fn parse_judge_value(value: &serde_json::Value) -> JudgeScore {
    let mut parsed_dims = BTreeMap::new();
    if let Some(obj) = value.get("dims").and_then(|d| d.as_object()) {
        for (k, v) in obj {
            if let Some(f) = v.as_f64() {
                parsed_dims.insert(k.clone(), f.clamp(0.0, 10.0));
            }
        }
    }
    let overall = value
        .get("overall")
        .and_then(|v| v.as_f64())
        .map(|f| f.clamp(0.0, 10.0))
        .unwrap_or_else(|| {
            if parsed_dims.is_empty() {
                0.0
            } else {
                parsed_dims.values().sum::<f64>() / parsed_dims.len() as f64
            }
        });
    let reasons = value
        .get("reasons")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    JudgeScore {
        overall,
        dims: parsed_dims,
        reasons,
    }
}

/// 中位数（偶数取中间两值均值）。空切片返回 0。
fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// 极差（max-min），作离散度指标记进日志，离散过大说明裁判不稳、该结论谨慎看待。
fn spread(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let max = xs.iter().cloned().fold(f64::MIN, f64::max);
    let min = xs.iter().cloned().fold(f64::MAX, f64::min);
    max - min
}

/// 用同一真模型作内容质检员，对 `model_output` 按 `dims` 逐维 0–10 打分。
/// 对照 `ground_truth` 判断 grounding/准确性，发现幻觉/偏离/遗漏扣分。只输出 JSON。
/// system prompt 内嵌通用锚定量表（[`JUDGE_RUBRIC`]），降低方差、避免题材漂移。
async fn judge_quality(
    llm: &dyn LlmProvider,
    task: &str,
    model_output: &str,
    ground_truth: &str,
    dims: &[&str],
) -> AppResult<JudgeScore> {
    let system = format!(
        "你是严格的私域运营知识内容质检员。针对给定任务，对【模型输出】按【评分维度】\
逐维打分，每维 0-10 分。必须对照【参考事实】判断 grounding 与准确性：发现凭空捏造/\
偏离事实/关键信息遗漏要显著扣分。\n{JUDGE_RUBRIC}\n\
只输出 JSON，禁止任何额外文字、禁止 markdown 代码围栏，形如：\
{{\"dims\":{{\"维度名\":分数,...}},\"overall\":综合分,\"reasons\":\"一句话评分依据\"}}。"
    );
    let user = format!(
        "## 任务\n{task}\n\n## 评分维度\n{dims_joined}\n\n## 参考事实(ground truth)\n{ground_truth}\n\n## 模型输出\n{model_output}\n",
        dims_joined = dims.join("、"),
    );
    let value = llm.generate_json(&system, &user).await?;
    Ok(parse_judge_value(&value))
}

/// 跑 [`JUDGE_RUNS`] 次 `judge_quality`，对 overall 与各维**分别取中位**，控真模型方差。
/// - 单次瞬时不可达（`LlmUnavailable`）不计入；全部失败才向上抛（调用方 skip）。
/// - 把 overall 极差（离散度）记进日志：离散大 = 裁判不稳，结论应谨慎。
async fn judge_quality_median(
    llm: &dyn LlmProvider,
    task: &str,
    model_output: &str,
    ground_truth: &str,
    dims: &[&str],
) -> AppResult<(JudgeScore, f64)> {
    let mut runs: Vec<JudgeScore> = Vec::new();
    let mut last_err = None;
    for _ in 0..JUDGE_RUNS {
        match judge_quality(llm, task, model_output, ground_truth, dims).await {
            Ok(s) => runs.push(s),
            Err(wechatagent::error::AppError::LlmUnavailable { .. }) => continue,
            Err(other) => last_err = Some(other),
        }
    }
    if runs.is_empty() {
        return Err(last_err.unwrap_or(wechatagent::error::AppError::LlmUnavailable {
            kind: "judge_all_runs_failed".to_string(),
            retry_count: JUDGE_RUNS as u32,
            detail: "judge 全部运行均瞬时不可达".to_string(),
            hint: "真模型上游抖动，按计划 skip 不算质量失败".to_string(),
        }));
    }
    let overalls: Vec<f64> = runs.iter().map(|s| s.overall).collect();
    let overall_median = median(&overalls);
    let overall_spread = spread(&overalls);
    // 各维分别取中位。
    let mut dim_medians = BTreeMap::new();
    let dim_keys: std::collections::BTreeSet<String> =
        runs.iter().flat_map(|s| s.dims.keys().cloned()).collect();
    for k in dim_keys {
        let vals: Vec<f64> = runs.iter().filter_map(|s| s.dims.get(&k).copied()).collect();
        dim_medians.insert(k, median(&vals));
    }
    let reasons = runs
        .last()
        .map(|s| s.reasons.clone())
        .unwrap_or_default();
    Ok((
        JudgeScore {
            overall: overall_median,
            dims: dim_medians,
            reasons,
        },
        overall_spread,
    ))
}

/// 校准锚：用一对**通用的** good/bad 样本（与被测文档无关）探裁判此刻是否还能分辨
/// 「贴合参考事实」与「偏离/捏造」。good 应显著高于 bad；拉不开（gap < [`CALIB_MIN_GAP`]）
/// 说明裁判此刻漂移/失灵 → 返回 false，调用方据此 skip（裁判问题不算被测对象的质量失败）。
async fn judge_calibrated(llm: &dyn LlmProvider) -> AppResult<bool> {
    let truth = "某产品的退款政策：开通后 7 天内无理由全额退款，超过 7 天不退。";
    let good = "退款政策：开通后 7 天内可无理由全额退款，超过 7 天则不予退款。";
    let bad = "退款政策：随时可退，30 天内退 80%，并赠送下一年免费使用。";
    let dims = ["grounding", "accuracy"];
    let task = "评估下面这段退款政策表述相对参考事实的 grounding 与准确性。";
    let (gs, _) = judge_quality_median(llm, task, good, truth, &dims).await?;
    let (bs, _) = judge_quality_median(llm, task, bad, truth, &dims).await?;
    let gap = gs.overall - bs.overall;
    eprintln!(
        "[CALIB] good_overall={:.1} bad_overall={:.1} gap={:.1} (min={CALIB_MIN_GAP}) ok={}",
        gs.overall,
        bs.overall,
        gap,
        gap >= CALIB_MIN_GAP,
    );
    Ok(gap >= CALIB_MIN_GAP)
}

/// 记一行可被 CI `grep '\[QUALITY\]'` 抓到的质量分；`spread` 是 K 次 judge 的 overall 极差
/// （离散度），离散大说明裁判此刻不稳、该结论应谨慎看待。
fn report_quality(qid: &str, scene: &str, score: &JudgeScore, spread: f64) {
    eprintln!(
        "[QUALITY] {qid}/{scene} overall={:.1} spread={:.1} floor_met={} target_met={} dims={:?} reasons={}",
        score.overall,
        spread,
        score.overall >= MIN_QUALITY_FLOOR,
        score.overall >= TARGET_QUALITY,
        score.dims,
        score.reasons,
    );
}

/// judge 打分 + 记日志 + 专业基线硬断言（破则 fail，驱动修生产代码）。
fn assert_quality_floor(qid: &str, score: &JudgeScore) {
    assert!(
        score.overall >= MIN_QUALITY_FLOOR,
        "[{qid}] judge overall={:.1} < 专业基线 {MIN_QUALITY_FLOOR}（dims={:?} reasons={}）——\
         内容质量未达生产可用，按迭代闭环修生产 prompt/检索/抽取逻辑，绝不放水断言",
        score.overall,
        score.dims,
        score.reasons,
    );
}

// ── 抽取语料矩阵 + 参考事实召回（抗过拟合地基）─────────────────────────────────
//
// 用户红线：「提示词等要通用科学的方法论，不能针对某一文章或者图片等进行特殊优化，
// 造成通用性不足，这个是作弊行为」。对策 = 多文档类型 + train/holdout 分割 + 确定性
// 的「参考事实原子单元召回」+ 泛化差距门：
//   - prompt 只能编码通用认知原则（原子单元召回 / 认知状态分类），一旦被特调去
//     适配某类文档，train 召回会高、holdout 召回会塌，泛化差距 gap 拉大即暴露作弊。
//   - 召回是**确定性 token 命中**（零模型方差、完全可复现），是抗作弊主武器；
//     judge 中位分是辅证（量化"读起来专不专业"）。
//
// 混合策略（用户选）：先合成跨 ≥10 类文档跑通框架；用户后续逐步把真实（脱敏）
// 文档按同结构追加进 corpus_matrix()，replace 掉合成条目即可，框架/判据不动。

/// 文档类型（覆盖差异极大的题材，确保 prompt 不能靠"猜业务"取巧）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocType {
    ContractClause,   // 合同/服务条款
    Spec,             // 产品规格参数
    QuoteTable,       // 报价表
    Faq,              // 常见问答
    CaseStudy,        // 客户案例
    TechManual,       // 技术操作手册
    Regulation,       // 规章/合规要求
    MeetingNotes,     // 会议纪要
    Methodology,      // 运营方法论
    Comparison,       // 方案对比
}

/// train=调优集（看分调 prompt）；holdout=留出集（绝不据其调 prompt，专测泛化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    Train,
    Holdout,
}

/// 一篇带 ground-truth 的抽取语料。
struct DocSpec {
    doc_type: DocType,
    split: Split,
    source_name: &'static str,
    content: &'static str,
    /// 手写的**参考事实原子单元**：每个内层切片是「判定该单元被召回所需全部命中的 token」。
    /// 一个 unit 被算召回 ⟺ 其全部 token 都出现在抽取文本里（确定性、可复现）。
    /// token 取**原文照搬的关键名词/数字**，不取主观描述，避免把召回判定本身写偏。
    reference_units: &'static [&'static [&'static str]],
}

/// 确定性召回率：抽取文本里命中了多少比例的参考事实原子单元。零模型方差、可复现。
fn reference_recall(extracted: &str, units: &[&[&str]]) -> f64 {
    if units.is_empty() {
        return 1.0;
    }
    let hit = units
        .iter()
        .filter(|unit| unit.iter().all(|tok| extracted.contains(*tok)))
        .count();
    hit as f64 / units.len() as f64
}

/// 抽取语料矩阵：10 类文档 × train/holdout 各半。每条手写参考事实原子单元。
/// **刻意覆盖差异极大的题材**：prompt 若只编码通用「原子单元召回」原则，应在所有题材
/// 上稳定召回；若被特调适配某类，holdout 题材召回会塌、泛化差距暴露。
fn corpus_matrix() -> Vec<DocSpec> {
    vec![
        // ── Train 集（5 类）──────────────────────────────────────────────
        DocSpec {
            doc_type: DocType::ContractClause,
            split: Split::Train,
            source_name: "企业服务合同-违约与赔付条款",
            content: "第八条 违约责任。\n8.1 乙方未按约定交付的，每延迟一日按合同总额 0.5% 支付违约金，累计不超过合同总额 10%。\n8.2 因乙方原因导致服务中断超过 24 小时的，甲方有权解除合同并要求退还剩余服务期款项。\n8.3 任一方泄露对方商业秘密的，应赔偿由此造成的全部损失。",
            reference_units: &[
                &["延迟", "0.5%", "违约金"],
                &["10%"],
                &["中断", "24 小时", "解除合同"],
                &["退还", "剩余服务期"],
                &["商业秘密", "赔偿"],
            ],
        },
        DocSpec {
            doc_type: DocType::Spec,
            split: Split::Train,
            source_name: "网关设备-规格参数表",
            content: "型号 GW-200。\n并发连接数：5 万。\n吞吐量：2.5 Gbps。\n工作温度：-10℃ 至 55℃。\n电源：DC 12V / 3A。\n接口：4 个千兆以太网口 + 2 个 SFP 光口。\n防护等级：IP54。",
            reference_units: &[
                &["GW-200"],
                &["并发", "5 万"],
                &["吞吐量", "2.5 Gbps"],
                &["-10℃", "55℃"],
                &["DC 12V", "3A"],
                &["千兆以太网", "SFP"],
                &["IP54"],
            ],
        },
        DocSpec {
            doc_type: DocType::QuoteTable,
            split: Split::Train,
            source_name: "SaaS 订阅报价表",
            content: "基础版：99 元/坐席/月，含 3 个坐席起。\n专业版：199 元/坐席/月，含工单系统与 API。\n企业版：399 元/坐席/月，含私有化部署与专属客户成功经理。\n年付优惠：一次性年付享 85 折。\n增值包：短信通道 0.05 元/条。",
            reference_units: &[
                &["基础版", "99 元"],
                &["专业版", "199 元"],
                &["企业版", "399 元"],
                &["私有化部署"],
                &["年付", "85 折"],
                &["短信", "0.05 元"],
            ],
        },
        DocSpec {
            doc_type: DocType::Faq,
            split: Split::Train,
            source_name: "账号与计费 FAQ",
            content: "问：试用期多久？答：新注册赠送 14 天全功能试用，无需绑卡。\n问：支持哪些支付方式？答：支持对公转账、微信、支付宝。\n问：可以中途升级套餐吗？答：可随时升级，按剩余天数折算差价。\n问：发票怎么开？答：支持增值税普通发票与专用发票，T+3 个工作日开具。",
            reference_units: &[
                &["试用", "14 天"],
                &["无需绑卡"],
                &["对公转账", "微信", "支付宝"],
                &["升级", "折算差价"],
                &["增值税", "专用发票"],
                &["T+3"],
            ],
        },
        DocSpec {
            doc_type: DocType::CaseStudy,
            split: Split::Train,
            source_name: "连锁餐饮客户案例",
            content: "客户为一家拥有 120 家门店的连锁餐饮品牌。\n接入前：私域消息靠人工回复，平均首响 2 小时，夜间无人应答。\n接入后：AI 自动应答覆盖 90% 高频咨询，平均首响降至 45 秒，夜间订单转化提升 15%。\n实施周期 4 周，投入 2 名运营配置知识库。",
            reference_units: &[
                &["120 家门店"],
                &["首响", "2 小时"],
                &["90%", "高频咨询"],
                &["45 秒"],
                &["夜间", "转化", "15%"],
                &["4 周"],
            ],
        },
        // ── Holdout 集（5 类，绝不据其调 prompt）──────────────────────────
        DocSpec {
            doc_type: DocType::TechManual,
            split: Split::Holdout,
            source_name: "数据备份操作手册",
            content: "执行全量备份：1. 进入「系统设置-数据」页。2. 点击「立即备份」，选择全量模式。3. 备份文件默认保留 30 天，可在策略里改为最长 180 天。\n恢复数据：在备份列表选择目标快照，点击「恢复」，恢复期间服务只读约 10 分钟。\n注意：跨大版本恢复需先升级到对应版本。",
            reference_units: &[
                &["全量备份"],
                &["立即备份"],
                &["保留 30 天"],
                &["180 天"],
                &["恢复", "只读", "10 分钟"],
                &["跨大版本", "升级"],
            ],
        },
        DocSpec {
            doc_type: DocType::Regulation,
            split: Split::Holdout,
            source_name: "个人信息处理合规要求",
            content: "一、收集个人信息须事先取得用户单独同意，并明示收集目的、方式、范围。\n二、敏感个人信息（身份证号、生物特征）须单独授权并加密存储。\n三、用户有权随时撤回同意、查询、更正、删除其个人信息，企业须在 15 个工作日内响应。\n四、向境外提供个人信息须通过安全评估。",
            reference_units: &[
                &["单独同意"],
                &["收集目的", "方式", "范围"],
                &["敏感", "加密存储"],
                &["撤回同意", "删除"],
                &["15 个工作日"],
                &["境外", "安全评估"],
            ],
        },
        DocSpec {
            doc_type: DocType::MeetingNotes,
            split: Split::Holdout,
            source_name: "Q2 产品迭代评审会纪要",
            content: "时间：2026 年 4 月 10 日。参会：产品、研发、运营负责人。\n决议一：知识库协作编辑功能列为 Q2 P0，研发 5 月底前提测。\n决议二：移动端推送暂缓到 Q3，本季度资源优先保障稳定性。\n决议三：客户成功团队每周同步 Top10 客户反馈给产品。\n待办：李雷整理竞品调研，4 月 20 日前出。",
            reference_units: &[
                &["2026 年 4 月 10 日"],
                &["协作编辑", "P0"],
                &["5 月底", "提测"],
                &["移动端推送", "Q3"],
                &["Top10", "客户反馈"],
                &["李雷", "竞品调研", "4 月 20 日"],
            ],
        },
        DocSpec {
            doc_type: DocType::Methodology,
            split: Split::Holdout,
            source_name: "沉睡客户唤醒方法论",
            content: "唤醒沉睡客户分三步：第一步分层，按最近活跃时间分为 30/90/180 天三档。\n第二步触点，30 天档用新功能/案例软触达，180 天档用专属回归权益。\n第三步节奏，单客户唤醒触达每周不超过 1 次，连续 3 次无响应转入低频池，避免打扰造成拉黑。",
            reference_units: &[
                &["分层", "30", "90", "180 天"],
                &["新功能", "案例"],
                &["专属", "回归权益"],
                &["每周不超过 1 次"],
                &["3 次无响应", "低频池"],
            ],
        },
        DocSpec {
            doc_type: DocType::Comparison,
            split: Split::Holdout,
            source_name: "自建团队 vs AI 托管对比",
            content: "自建运营团队：人力成本高（人均月薪 8000+），招聘培训周期长，夜间与节假日覆盖难，质量随人员流动波动。\nAI 托管：按坐席订阅，成本可预测，7×24 小时覆盖，话术质量统一可审计，但需前期投入配置知识库。\n结论：高咨询量、强时效场景更适合 AI 托管。",
            reference_units: &[
                &["人均月薪 8000"],
                &["招聘培训", "周期长"],
                &["夜间", "节假日"],
                &["7×24"],
                &["质量统一", "可审计"],
                &["配置知识库"],
            ],
        },
    ]
}

/// 图片抽取语料清单项：便于用户后续把真实（脱敏）图片按同结构投喂。
/// `base64` 为图片内容、`reference_units` 为手写参考事实原子单元（判定召回）。
struct ImageSpec {
    split: Split,
    source_name: &'static str,
    hint: &'static str,
    base64: &'static str,
    reference_units: &'static [&'static [&'static str]],
}

/// 图片抽取语料矩阵。当前先用 K6 既有 fixture 占一个 train 槽位跑通框架；
/// 用户后续把真实（脱敏）多题材图片（表格/表单/图表/幻灯片/信息图等）按同结构
/// 追加，并补 holdout 槽位，即可让 vision 侧也进入 train/holdout 泛化判据。
fn image_matrix() -> Vec<ImageSpec> {
    vec![ImageSpec {
        split: Split::Train,
        source_name: "k6_terms_image",
        hint: "企业服务条款图片",
        base64: include_str!("fixtures/k6_article_image.b64"),
        // K6 图为合成条款图；这里只放低敏感、稳定可命中的原子单元占位。
        // 真实图替换后，按图中确切文字改写本列表。
        reference_units: &[],
    }]
}

// ── seed helper：可控 wiki_type / chunk_type 的 verified chunk ─────────────────

#[allow(clippy::too_many_arguments)]
async fn seed_typed(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
    wiki_type: &str,
    chunk_type: &str,
    dynamic_confidence: f64,
    related: Vec<RelatedRef>,
) -> String {
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: title.to_string(),
        summary: Some(summary.to_string()),
        body: Some(body.to_string()),
        source_quote: Some(body.to_string()),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(90),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some(wiki_type.to_string()),
        dynamic_confidence: Some(dynamic_confidence),
        chunk_type: chunk_type.to_string(),
        related_chunks: if related.is_empty() { None } else { Some(related) },
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert typed chunk");
    id.to_hex()
}

// ── 质量语料（跨 9 wiki_type × 4 chunk_type 的真实私域运营知识库）─────────────

const CORPUS_PRICE_METHOD: &str =
    "处理价格异议的标准方法：第一步共情，认可客户对预算的顾虑；第二步用 ROI 价值锚点\
（节省的人力成本与转化提升）重构性价比，而非比单价；第三步给出按月付费或 14 天试用，\
降低首次决策门槛。绝不直接降价，降价会损害长期价值认知。";

const CORPUS_COMPARISON: &str =
    "与传统群发工具对比：群发是一对多广播、易触发风控被封、缺乏上下文；WechatAgent 是\
逐人逐场景的对话 Agent，带合规审查与渐进式知识检索，按真人节奏发送，不触发风控。";

const CORPUS_PEER_CASE: &str =
    "某连锁零售客户接入 6 周后，私域消息平均首响从 4 小时降到 3 分钟，沉睡客户激活率\
提升 18%，整体转化率提升 22%。关键动作是把高频咨询交给 AI 自动应答 + 人工聚焦高意向客户。";

/// 一次 seed 跨齐 9 个 wiki_type × 4 个 chunk_type 的知识库；返回全部 chunk id。
async fn quality_corpus(app: &TestApp, ws: &str) -> Vec<String> {
    let mut ids = Vec::new();
    // thesis / product_fact
    ids.push(
        seed_typed(
            app, ws,
            "产品核心主张",
            "WechatAgent 用 AI 全自动接管私域逐人对话决策。",
            "WechatAgent 的核心主张：私域运营里重复的对话决策应由 AI 全自动完成，运营聚焦\
策略而非逐条回复。它逐人逐场景做决策、合规审查与跟进，不是群发工具。",
            "thesis", "product_fact", 0.95, Vec::new(),
        )
        .await,
    );
    // synthesis / product_fact
    ids.push(
        seed_typed(
            app, ws,
            "整体解决方案",
            "决策 + 审查 + 渐进式知识检索三件套。",
            "WechatAgent 的整体方案由三部分组成：Reply Agent 做对话决策、独立 Review Agent\
做合规与事实审查、知识库 Agent 做渐进式检索为回答提供已验证依据，三者串成一条自动链路。",
            "synthesis", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // methodology / style_template（价格异议处理方法论）
    ids.push(
        seed_typed(
            app, ws,
            "价格异议处理方法论",
            "共情 → ROI 价值锚点 → 试用/分期，绝不直接降价。",
            CORPUS_PRICE_METHOD,
            "methodology", "style_template", 0.92, Vec::new(),
        )
        .await,
    );
    // finding / peer_case（客户案例）
    ids.push(
        seed_typed(
            app, ws,
            "零售客户实施成效",
            "某零售客户 6 周首响 4 小时→3 分钟、转化 +22%。",
            CORPUS_PEER_CASE,
            "finding", "peer_case", 0.88, Vec::new(),
        )
        .await,
    );
    // comparison / product_fact（与群发工具对比）
    ids.push(
        seed_typed(
            app, ws,
            "与传统群发工具对比",
            "群发广播易被封；本产品逐人对话不触发风控。",
            CORPUS_COMPARISON,
            "comparison", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // concept / product_fact（渐进式检索概念）
    ids.push(
        seed_typed(
            app, ws,
            "渐进式知识检索概念",
            "先看目录摘要，再按需展开正文与关联条目。",
            "渐进式知识检索指 Agent 先读 catalog 目录摘要，再按需 open 正文、follow 关联条目，\
而非一次性把全部知识塞进 prompt，既省 token 又避免上下文淹没。",
            "concept", "product_fact", 0.85, Vec::new(),
        )
        .await,
    );
    // entity / product_fact（定价实体）
    ids.push(
        seed_typed(
            app, ws,
            "企业版定价",
            "企业版 299 元/坐席/月，含私有化部署选项。",
            "企业版定价为 299 元/坐席/月，含私有化部署选项与每年 3 次远程培训；标准版 99 元/坐席/月。",
            "entity", "product_fact", 0.9, Vec::new(),
        )
        .await,
    );
    // source / product_fact（SLA 来源条款）
    ids.push(
        seed_typed(
            app, ws,
            "SLA 来源条款",
            "企业版月度可用性 99.95%，低于 99.9% 赔 30%。",
            "企业版 SLA 原始条款：承诺月度可用性 99.95%；当月低于 99.9% 按服务费 30% 以等额\
服务时长赔付，低于 99.5% 赔 50%，不退现金。",
            "source", "product_fact", 0.8, Vec::new(),
        )
        .await,
    );
    // query / negative_example（错误问法负面示例）
    ids.push(
        seed_typed(
            app, ws,
            "错误问法负面示例",
            "直接问客户预算会激发防御，应探询业务目标。",
            "运营常犯的错误问法：开场就问『你预算多少』会激发客户防御与压力感。应改为先探询\
业务目标与现状痛点，再自然引出方案与投入，属于负面示例，不要照搬。",
            "query", "negative_example", 0.7, Vec::new(),
        )
        .await,
    );
    ids
}

// ── Q1 · 检索/answer 内容质量（grounding/accuracy/relevance）────────────────────
//
// 跨 methodology(style_template) + comparison(product_fact) 语料，问一个 price
// objection 场景。硬命中红线：answer 非空 ∧ cite⊆seed ∧ 命中价格方法论关键事实
// token。judge：grounding/accuracy/relevance ≥ floor。

#[tokio::test]
#[ignore]
async fn q1_retrieval_price_objection_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();
    let seed = quality_corpus(&app, &ws).await;

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        account_id: None,
        query: "客户说『你们太贵了』，作为运营我该怎么回应才专业又不掉价？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };
    let result =
        unwrap_or_skip_transient!(answer(&state, req).await, "Q1 真实知识 agent answer");

    let hits_method = ["共情", "ROI", "价值", "试用", "按月", "锚点", "降价"]
        .iter()
        .any(|t| result.answer.contains(t));
    eprintln!(
        "[q1] rounds={} cited={:?} hits_method={} answer={:?}",
        result.rounds_used,
        result.cited_chunk_ids,
        hits_method,
        result.answer.chars().take(160).collect::<String>(),
    );

    // 硬命中红线。
    assert!(!result.answer.trim().is_empty(), "Q1 answer 不应为空");
    for c in &result.cited_chunk_ids {
        assert!(seed.contains(c), "Q1 cite 了不存在的 chunk id={c}（不在 seed）");
    }
    assert!(
        hits_method,
        "Q1 answer 未命中价格异议方法论任一关键事实 token——检索未把对的方法论喂给生成。answer={:?}",
        result.answer
    );

    // LLM-judge 内容质量（K 次取中位，控真模型方差）。
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "运营问『客户嫌贵怎么回应』，评估 AI 给出的话术建议质量。",
            &result.answer,
            CORPUS_PRICE_METHOD,
            &["grounding", "accuracy", "relevance"],
        )
        .await,
        "Q1 judge"
    );
    report_quality("Q1", "price_objection", &score, spread);
    assert_quality_floor("Q1", &score);
}

// ── Q2 · 文章抽取内容质量【旗舰：全语料矩阵 + 召回 + 中位 judge + 泛化差距门】──────
//
// 这是演示完整科学方法论的旗舰用例，直接落实用户红线「不能针对某一文章特殊优化，
// 这个是作弊行为」：
//   1. 跑 corpus_matrix() 全 10 类文档（差异极大的题材），每条:
//      - 硬命中红线：抽出 ≥1 条 ∧ 每条恒 draft + needs_review；
//      - 确定性「参考事实原子单元召回」reference_recall（零模型方差、可复现）；
//      - 中位 judge（K 次取中位 + 离散度）量化专业度。
//   2. 对 train / holdout 两个 split **分别**断言平均召回 ≥ MIN_RECALL_FLOOR。
//   3. **泛化差距门**：|mean(train_recall) - mean(holdout_recall)| ≤ MAX_GENERALIZATION_GAP。
//      prompt 若被特调去适配 train 文档，train 召回会虚高、holdout 召回塌，gap 爆 = 硬 fail。
// judge 失灵（校准锚拉不开）时跳过 judge 断言，但**确定性召回断言照常生效**（不受裁判影响）。

#[tokio::test]
#[ignore]
async fn q2_article_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());

    // 裁判此刻是否还能分辨贴合/偏离参考事实；分不开则本轮跳过 judge 断言（确定性召回仍跑）。
    let judge_ok = match judge_calibrated(llm.as_ref()).await {
        Ok(ok) => ok,
        Err(wechatagent::error::AppError::LlmUnavailable { .. }) => {
            eprintln!("skip judge: 校准锚瞬时不可达，本轮只跑确定性召回断言");
            false
        }
        Err(e) => panic!("Q2 校准锚异常：{e}"),
    };

    let mut train_recalls: Vec<f64> = Vec::new();
    let mut holdout_recalls: Vec<f64> = Vec::new();

    for spec in corpus_matrix() {
        let req: OperationKnowledgeImportRequest = serde_json::from_value(json!({
            "accountId": null,
            "sourceName": spec.source_name,
            "content": spec.content,
        }))
        .expect("构造 OperationKnowledgeImportRequest");

        let resp = unwrap_or_skip_transient!(
            import_operation_knowledge_preview(State(state.clone()), Json(req)).await,
            "Q2 真实文章抽取"
        );
        let body = resp.0;
        let chunks = body["chunks"].as_array().cloned().unwrap_or_default();
        let items = body["items"].as_array().cloned().unwrap_or_default();

        // 硬命中红线：抽出至少 1 条 ∧ 每条恒 draft + 绝不自动 verified（对所有题材一致）。
        // 「AI 永不自动 verify」的结构性保证是 integrityStatus ∈ {needs_review, rejected}：
        // preview 路径恒 0 verified（integrity_report_for_preview 只产这两种），其中
        // rejected 是更严方向——chunk 带 safeClaims/evidenceItems 却无可锚定原文引用时硬挡，
        // 比 needs_review 更不可放行。断言只锁「绝不 verified」这条真红线，
        // 不把更严的 rejected 误判为回归。
        assert!(
            !chunks.is_empty() || !items.is_empty(),
            "Q2[{:?}/{}] 应至少抽出 1 条 chunk/item",
            spec.doc_type, spec.source_name
        );
        for (i, chunk) in chunks.iter().enumerate() {
            let integrity = chunk["integrityStatus"].as_str();
            assert!(
                matches!(integrity, Some("needs_review") | Some("rejected")),
                "Q2[{}] preview chunk[{i}] integrityStatus 必须 ∈ {{needs_review, rejected}}（AI 永不自动 verify），实际 {integrity:?}",
                spec.source_name
            );
            assert_eq!(
                chunk["status"].as_str(),
                Some("draft"),
                "Q2[{}] preview chunk[{i}] 必须 draft",
                spec.source_name
            );
        }

        // 抽取文本 = 所有 chunk(title/summary/body) + items(并入兜底) 拼接。
        let mut extracted = String::new();
        for c in &chunks {
            for k in ["title", "summary", "body", "sourceQuote", "routingCard"] {
                if let Some(s) = c.get(k).and_then(|v| v.as_str()) {
                    extracted.push_str(s);
                    extracted.push('\n');
                }
            }
        }
        for it in &items {
            extracted.push_str(&serde_json::to_string(it).unwrap_or_default());
            extracted.push('\n');
        }

        let recall = reference_recall(&extracted, spec.reference_units);
        eprintln!(
            "[Q2-RECALL] {:?}/{} split={:?} recall={:.2} chunks={} items={}",
            spec.doc_type, spec.source_name, spec.split, recall, chunks.len(), items.len()
        );
        match spec.split {
            Split::Train => train_recalls.push(recall),
            Split::Holdout => holdout_recalls.push(recall),
        }

        // 中位 judge（裁判可用时）：量化专业度，记日志驱动选短板。
        if judge_ok && !chunks.is_empty() {
            let model_output = serde_json::to_string_pretty(&body["chunks"]).unwrap_or_default();
            match judge_quality_median(
                llm.as_ref(),
                "评估 AI 从一篇资料中抽取出的知识切片：是否穷尽覆盖原文每个信息单元、\
标题摘要是否准确、正文是否保真不丢关键数字与表述、有无编造原文没有的内容。",
                &model_output,
                spec.content,
                &["extraction_completeness", "fidelity", "structure"],
            )
            .await
            {
                Ok((score, spread)) => {
                    report_quality("Q2", spec.source_name, &score, spread);
                    assert_quality_floor("Q2", &score);
                }
                Err(wechatagent::error::AppError::LlmUnavailable { .. }) => {
                    eprintln!("[Q2-JUDGE] {} 瞬时不可达，跳过该条 judge", spec.source_name);
                }
                Err(e) => panic!("Q2[{}] judge 异常：{e}", spec.source_name),
            }
        }
    }

    // ── 确定性召回断言（不受裁判影响，抗过拟合主门）──────────────────────────
    let mean = |xs: &[f64]| if xs.is_empty() { 0.0 } else { xs.iter().sum::<f64>() / xs.len() as f64 };
    let train_mean = mean(&train_recalls);
    let holdout_mean = mean(&holdout_recalls);
    let gap = (train_mean - holdout_mean).abs();
    eprintln!(
        "[Q2-GENERALIZE] train_recall={:.2}(n={}) holdout_recall={:.2}(n={}) gap={:.2} (max={MAX_GENERALIZATION_GAP})",
        train_mean, train_recalls.len(), holdout_mean, holdout_recalls.len(), gap
    );

    assert!(
        !train_recalls.is_empty() && !holdout_recalls.is_empty(),
        "Q2 训练/留出集都必须有样本（实际 train={} holdout={}）",
        train_recalls.len(), holdout_recalls.len()
    );
    assert!(
        train_mean >= MIN_RECALL_FLOOR,
        "Q2 训练集平均召回 {train_mean:.2} < 基线 {MIN_RECALL_FLOOR}——抽取漏掉过多参考事实，\
         修通用抽取 prompt（原子单元召回），绝不放水"
    );
    assert!(
        holdout_mean >= MIN_RECALL_FLOOR,
        "Q2 留出集平均召回 {holdout_mean:.2} < 基线 {MIN_RECALL_FLOOR}——在没见过的题材上抽取召回不足，\
         说明 prompt 通用性不够，修通用认知原则而非堆题材枚举"
    );
    assert!(
        gap <= MAX_GENERALIZATION_GAP,
        "Q2 泛化差距 {gap:.2} > 上限 {MAX_GENERALIZATION_GAP}（train={train_mean:.2} holdout={holdout_mean:.2}）\
         ——train 召回远高于 holdout = prompt 被特调适配训练文档（过拟合/作弊）。\
         必须把 prompt 收敛回与题材无关的通用原则，绝不靠枚举特定文档结构取巧"
    );
}

// ── Q3 · vision 抽取内容质量（fidelity/completeness）────────────────────────────
//
// 硬命中红线：任何落库 chunk 恒 draft + needs_review。judge：视觉抽取保真度/完整度。
// 复用 K6 的中文条款图 fixture。

const Q3_ARTICLE_IMAGE_BASE64: &str = include_str!("fixtures/k6_article_image.b64");

#[tokio::test]
#[ignore]
async fn q3_vision_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let api_key = std::env::var("REAL_LLM_API_KEY").expect("require_real_llm 已保证存在");
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let vision_model = std::env::var("REAL_LLM_VISION_MODEL")
        .or_else(|_| std::env::var("REAL_LLM_MODEL"))
        .unwrap_or_else(|_| "mimo-v2.5".to_string());
    let vision_cfg = LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: ws.clone(),
        provider_id: "real_vision_q3".to_string(),
        name: "real_vision_q3".to_string(),
        format: "openai".to_string(),
        base_url,
        api_key,
        model: vision_model,
        is_active: false,
        timeout_seconds: Some(180),
        max_retries: Some(3),
        retry_base_ms: Some(1500),
        supports_vision: true,
        is_vision_active: true,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    app.state
        .db
        .llm_provider_configs()
        .insert_one(&vision_cfg, None)
        .await
        .expect("insert vision provider");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q3_admin".into(),
        username: "q3_admin".into(),
        current_workspace: ws.clone(),
    });
    let req = ImportApplyImageRequest {
        image_base64: Q3_ARTICLE_IMAGE_BASE64.trim().to_string(),
        mime: Some("image/png".to_string()),
        source_name: Some("q3_article_image".to_string()),
        account_id: None,
        hint: Some("企业版服务条款图片".to_string()),
    };

    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_apply_image(State(app.state.clone()), admin, Json(req)).await,
        "Q3 真实 vision 抽取"
    );
    let body = resp.0;
    let chunk_ids = body["chunkIds"].as_array().cloned().unwrap_or_default();
    eprintln!("[q3] vision chunkIds={}", chunk_ids.len());

    // 硬命中红线：落库 chunk 恒 draft + needs_review；同时收集正文喂 judge。
    let mut extracted_bodies = Vec::new();
    for id in &chunk_ids {
        let id_hex = id.as_str().expect("chunkId str");
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! { "_id": ObjectId::parse_str(id_hex).expect("parse oid"), "workspace_id": &ws },
                None,
            )
            .await
            .expect("query chunk")
            .expect("chunk exists");
        assert_eq!(chunk.status, "draft", "Q3 vision chunk 必须 draft");
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "Q3 vision chunk 必须 needs_review"
        );
        extracted_bodies.push(format!(
            "{}：{}",
            chunk.title,
            chunk.body.unwrap_or_default()
        ));
    }

    if extracted_bodies.is_empty() {
        eprintln!("[q3] vision 未抽出任何 chunk（真模型软能力，红线真空成立），跳过 judge");
        return;
    }
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "评估 AI 从一张中文条款图片里抽取的知识：文字识别是否保真、图中每个信息单元\
是否被穷尽覆盖、有无编造图中没有的内容。",
            &extracted_bodies.join("\n"),
            "图片为一张含企业服务条款的中文文章图；理想抽取应逐条覆盖图中出现的每个条款/字段，\
保留原文关键表述与数字，且不编造图里没有的内容。",
            &["fidelity", "completeness"],
        )
        .await,
        "Q3 judge"
    );
    report_quality("Q3", "vision_terms", &score, spread);
    assert_quality_floor("Q3", &score);
}

// ── Q4 · 对话工作台内容质量（intent_correctness/reply_naturalness）──────────────
//
// 硬命中红线：intent∈7闭集 ∧ chunk 计数不变 ∧ verified=0。judge：意图判对 + 回复自然度。

#[tokio::test]
#[ignore]
async fn q4_chat_workstation_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    let chunks_before = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks before");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q4_admin".into(),
        username: "q4_admin".into(),
        current_workspace: ws.clone(),
    });
    let req: ChatTurnRequest = serde_json::from_value(json!({
        "sessionId": null,
        "accountId": null,
        "operatorId": "q4_operator",
        "content": "帮我新建一条知识切片：企业版支持私有化部署，数据不出客户内网，\
                    知识类型是产品能力，请起草标题、摘要和正文。",
        "attachments": [],
    }))
    .expect("构造 ChatTurnRequest");

    let resp = unwrap_or_skip_transient!(
        chat_turn(State(state.clone()), admin, Json(req)).await,
        "Q4 真实对话工作台 chat_turn"
    );
    let body = resp.0;
    let intent = body.get("intent").and_then(|v| v.as_str()).unwrap_or("");
    let natural_reply = body
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    eprintln!(
        "[q4] intent={intent} canApply={:?} naturalReply.len={}",
        body.get("canApply"),
        natural_reply.chars().count(),
    );

    // 硬命中红线。
    const INTENTS: &[&str] = &[
        "create_chunk",
        "update_chunk",
        "clarify_chunk",
        "update_pack",
        "digest_action",
        "update_operator_memory",
        "freeform",
    ];
    assert!(
        INTENTS.contains(&intent),
        "Q4 intent 必须 ∈ 闭集 {INTENTS:?}，实际 {intent:?}"
    );
    let chunks_after = state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks after");
    assert_eq!(
        chunks_before, chunks_after,
        "Q4 对话起草自动落库了 chunk——「只产 proposal、永不落库」红线被击穿"
    );
    let verified_after = state
        .db
        .operation_knowledge_chunks()
        .count_documents(
            doc! { "workspace_id": &ws, "integrity_status": "verified", "status": "active" },
            None,
        )
        .await
        .expect("count verified after");
    assert_eq!(verified_after, 0, "Q4 对话起草落库了 verified chunk——红线被击穿");

    // judge：意图判对 + 回复自然度（明确的新建意图，正确 intent 应为 create_chunk）。
    let model_output = format!("intent={intent}\nnaturalReply={natural_reply}");
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "运营在对话框说『帮我新建一条关于私有化部署的产品能力切片，起草标题/摘要/正文』。\
评估 AI 的意图分类是否正确（应为新建切片 create_chunk）、回复是否自然且有效引导补全。",
            &model_output,
            "正确意图是 create_chunk（新建切片）；理想回复应自然地确认意图并起草/引导补全标题、\
摘要、正文等字段，而不是答非所问或生硬。",
            &["intent_correctness", "reply_naturalness"],
        )
        .await,
        "Q4 judge"
    );
    report_quality("Q4", "create_chunk_intent", &score, spread);
    assert_quality_floor("Q4", &score);
}

// ── Q5 · 完整度审计内容质量（gap_analysis_quality/coverage_accuracy）────────────
//
// 硬命中红线：answeringMode∈3闭集 ∧ needs_review chunk 审计后仍 needs_review（只读）。
// judge：gap 分析质量、覆盖判断准确性。

#[tokio::test]
#[ignore]
async fn q5_completeness_audit_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();

    // 语料态：一批 verified 业务知识 + 1 条 needs_review 报价草稿（审计绝不能转它）。
    let _seed = quality_corpus(&app, &ws).await;
    let needs_review_id = seed_typed(
        &app,
        &ws,
        "未审定的报价草稿",
        "一条尚未审定的报价说明。",
        "旗舰版 999 元/月（待核实）。",
        "entity",
        "product_fact",
        0.9,
        Vec::new(),
    )
    .await;
    // 把它压回 needs_review（seed_typed 默认 verified）。
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": ObjectId::parse_str(&needs_review_id).expect("oid") },
            doc! { "$set": { "integrity_status": "needs_review" } },
            None,
        )
        .await
        .expect("set needs_review");

    let audit = unwrap_or_skip_transient!(
        build_operation_knowledge_completeness(&state, &ws, &account_id).await,
        "Q5 真实知识完整度审计"
    );
    let mode = audit
        .get("answeringMode")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    eprintln!(
        "[q5] answeringMode={mode} totalChunks={:?} verifiedChunks={:?} gaps={:?}",
        audit.get("totalChunks"),
        audit.get("verifiedChunks"),
        audit.get("gaps"),
    );

    // 硬命中红线。
    const MODES: &[&str] = &["relationship_only", "product_safe", "fully_supported"];
    assert!(
        MODES.contains(&mode),
        "Q5 answeringMode 必须 ∈ 闭集 {MODES:?}，实际 {mode:?}"
    );
    let after = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": ObjectId::parse_str(&needs_review_id).expect("parse oid") },
            None,
        )
        .await
        .expect("query needs_review chunk")
        .expect("chunk exists");
    assert_eq!(
        after.integrity_status.as_deref(),
        Some("needs_review"),
        "Q5 完整度审计把 needs_review 改成 {:?}——审计只读红线被击穿",
        after.integrity_status
    );

    let model_output = serde_json::to_string_pretty(&audit).unwrap_or_default();
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "评估 AI 对知识库的完整度自审：给出的 answeringMode 与 gaps 是否合理反映了知识库\
现状（哪些维度有已验证客观事实、哪些只有方法论/话术或未审定草稿），gap 分析是否有指导价值。",
            &model_output,
            "知识库已有产品主张/整体方案/对比/案例/SLA 等 verified 内容，并存在一条未审定的报价草稿。\
理想审计应按认知状态分类区分『已验证客观事实 vs 仅方法论/未审定草稿』，识别可支撑范围并指出\
需补强/核实的缺口，且不得因未审定草稿存在就判 fully_supported。",
            &["gap_analysis_quality", "coverage_accuracy"],
        )
        .await,
        "Q5 judge"
    );
    report_quality("Q5", "audit_mixed_corpus", &score, spread);
    assert_quality_floor("Q5", &score);
}

// ── Q6 · AI 修复内容质量（patch_reasonableness/field_targeting）─────────────────
//
// 硬命中红线：propose 后 DB 里 body/status/integrity_status 完全不变（patch 永不落库）。
// judge：修复 patch 是否合理、是否精准命中缺失字段。

#[tokio::test]
#[ignore]
async fn q6_repair_patch_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());
    let ws = state.config.default_workspace_id.clone();

    // seed 一条信息残缺的 needs_review chunk（缺正文细节 + 无 source_quote）。
    let id = ObjectId::new();
    let now = DateTime::now();
    let original_body = "退款政策。".to_string();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: ws.clone(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: "退款政策".to_string(),
        summary: Some("退款相关说明。".to_string()),
        body: Some(original_body.clone()),
        source_quote: None,
        integrity_status: Some("needs_review".to_string()),
        confidence_score: Some(30),
        status: "draft".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(0.5),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert repair-target chunk");

    let admin = Extension(AuthenticatedAdmin {
        user_id: "q6_admin".into(),
        username: "q6_admin".into(),
        current_workspace: ws.clone(),
    });
    let resp = unwrap_or_skip_transient!(
        propose_chunk_repair(State(state.clone()), admin, Path(id.to_hex())).await,
        "Q6 真实 AI 修复 propose"
    );
    let body = resp.0;
    eprintln!(
        "[q6] hasPatch={} missingFields={:?}",
        body.get("patch").map(|p| !p.is_null()).unwrap_or(false),
        body.get("missingFields"),
    );

    // 硬命中红线：DB 完全不变。
    let after = state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("query chunk")
        .expect("chunk exists");
    assert_eq!(
        after.body.as_deref(),
        Some(original_body.as_str()),
        "Q6 AI 修复不得改 body（patch 只返回不落库）"
    );
    assert_eq!(after.status, "draft", "Q6 AI 修复不得改 status");
    assert_eq!(
        after.integrity_status.as_deref(),
        Some("needs_review"),
        "Q6 AI 修复不得改 integrity_status"
    );

    let model_output = serde_json::to_string_pretty(&body).unwrap_or_default();
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "一条退款政策切片正文只有『退款政策。』、缺 source_quote。评估 AI 给出的修复方案：\
是否精准指出缺失字段、提出的补全建议是否合理（不得凭空编造具体数字当成事实）。",
            &model_output,
            "原 chunk 正文残缺、无原文引用。理想修复应识别正文过简、缺 source_quote 等缺口，\
建议补全退款条件/期限等结构，但不应把未经核实的具体数字写成既定事实。",
            &["patch_reasonableness", "field_targeting"],
        )
        .await,
        "Q6 judge"
    );
    report_quality("Q6", "incomplete_chunk", &score, spread);
    assert_quality_floor("Q6", &score);
}

// ── Q7 · 打标内容质量（tag_accuracy/taxonomy_mapping）───────────────────────────
//
// 硬命中红线：productTags / businessTopics 双数组。judge：标签准确性 + 是否贴合内容。

#[tokio::test]
#[ignore]
async fn q7_tag_extraction_quality() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm.clone(), mcp.uri());

    let title = "价格异议处理方法论";
    let req: ExtractKnowledgeTagsRequest = serde_json::from_value(json!({
        "accountId": null,
        "title": title,
        "body": CORPUS_PRICE_METHOD,
    }))
    .expect("构造 ExtractKnowledgeTagsRequest");

    let resp = unwrap_or_skip_transient!(
        extract_operation_knowledge_tags(State(state.clone()), Json(req)).await,
        "Q7 真实标签抽取"
    );
    let body = resp.0;
    eprintln!(
        "[q7] productTags={:?} businessTopics={:?}",
        body.get("productTags"),
        body.get("businessTopics"),
    );

    // 硬命中红线：两字段都是数组。
    assert!(body["productTags"].is_array(), "Q7 productTags 必须是数组");
    assert!(
        body["businessTopics"].is_array(),
        "Q7 businessTopics 必须是数组"
    );

    let model_output = serde_json::to_string_pretty(&body).unwrap_or_default();
    let (score, spread) = unwrap_or_skip_transient!(
        judge_quality_median(
            llm.as_ref(),
            "评估 AI 为『价格异议处理方法论』切片抽取的标签（productTags / businessTopics）：\
是否贴合内容主题（价格异议/销售方法/客户沟通等），有无明显跑题或空泛标签。",
            &model_output,
            CORPUS_PRICE_METHOD,
            &["tag_accuracy", "taxonomy_mapping"],
        )
        .await,
        "Q7 judge"
    );
    report_quality("Q7", "price_method_tags", &score, spread);
    assert_quality_floor("Q7", &score);
}

// ── 确定性回归锁（非 #[ignore]：CI 编译并执行，零 key / 零 Docker / 零模型方差）──────
//
// 锁死抗过拟合方法论的纯函数语义，防止后续迭代误改 reference_recall / median /
// spread / corpus_matrix 的形状导致泛化差距门失效。这些是质量套件判据的地基。

#[test]
fn reference_recall_is_deterministic_and_bounded() {
    let units: &[&[&str]] = &[&["A", "B"], &["C"], &["D", "E"]];
    // 全命中。
    assert_eq!(reference_recall("A B C D E", units), 1.0);
    // 一个 unit 缺一个 token（A B 的 B 缺）→ 该 unit 不算召回。
    assert!((reference_recall("A C D E", units) - 2.0 / 3.0).abs() < 1e-9);
    // 全不命中。
    assert_eq!(reference_recall("zzz", units), 0.0);
    // 空 unit 列表视为满召回（不拖低均值，由用例自行决定是否有 ground truth）。
    assert_eq!(reference_recall("anything", &[]), 1.0);
}

#[test]
fn median_and_spread_match_definition() {
    assert_eq!(median(&[5.0]), 5.0);
    assert_eq!(median(&[1.0, 9.0]), 5.0); // 偶数取中间两值均值
    assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0); // 内部排序
    assert_eq!(median(&[]), 0.0);
    assert_eq!(spread(&[2.0, 7.0, 4.0]), 5.0);
    assert_eq!(spread(&[]), 0.0);
}

#[test]
fn corpus_matrix_has_both_splits_and_diverse_types() {
    let m = corpus_matrix();
    let train = m.iter().filter(|d| d.split == Split::Train).count();
    let holdout = m.iter().filter(|d| d.split == Split::Holdout).count();
    assert!(train >= 5, "训练集至少 5 类，实际 {train}");
    assert!(holdout >= 5, "留出集至少 5 类，实际 {holdout}");
    // 题材必须足够多样（去重后 ≥10），否则 prompt 能靠猜业务取巧。
    let mut types: Vec<DocType> = m.iter().map(|d| d.doc_type).collect();
    types.dedup();
    assert!(
        m.len() >= 10 && types.len() == m.len(),
        "语料矩阵应 ≥10 类且类型不重复，实际 {} 条 / {} 类",
        m.len(),
        types.len()
    );
    // 每条都必须带至少 1 个参考事实原子单元（否则召回判定形同虚设）。
    for d in &m {
        assert!(
            !d.reference_units.is_empty(),
            "语料 {} 缺参考事实原子单元，召回门会被架空",
            d.source_name
        );
    }
}
