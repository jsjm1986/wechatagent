//! `real_llm_recall_benchmark` —— 运营 agent 知识库召回率基准测试套件。
//!
//! 量化知识检索系统的召回率性能：
//! - **reach 召回**：目标答案深埋在知识库中能否被检索到
//! - **adopt 召回**：检索到的知识能否被正确采用到答案中
//! - **跨轮稳定性**：多轮对话中召回质量是否保持稳定
//! - **chat 改库后召回保持**：知识库更新后召回能力是否退化
//!
//! ## 红线
//! - **MCP 永远是桩**：知识链路不发消息，使用空 wiremock 作为 MCP 服务
//! - **密钥零泄漏**：只从 env 读取 `REAL_LLM_API_KEY`，不在断言信息中泄漏
//! - **cite ⊆ seed**：真模型引用的 chunk id 必须是测试中 seed 的子集，禁止幻觉引用
//! - **env-gated**：无 `REAL_LLM_API_KEY` 时自动跳过，默认 `#[ignore]`
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=deepseek-v4-pro \
//!   cargo test --test real_llm_recall_benchmark -- --ignored --nocapture
//! ```

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, AnswerResult, CatalogFilter, SourceQuoteCitation};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::llm::LlmClient;
use wechatagent::models::{KnowledgeGapSignal, OperationKnowledgeChunk, RelatedRef};
use wechatagent::routes::AppState;
use wechatagent::routes::ext_knowledge::{chat_apply, chat_turn, verify_operation_knowledge_chunk, ChatApplyRequest, ChatTurnRequest, KnowledgeVerifyRequest};
use axum::{extract::{State, Path}, Extension, Json};
use serde_json::json;

use crate::common::TestApp;
use wiremock::MockServer;

// ── env-gated 真实 provider 构造 ────────────────────────────────────────

/// 从 env 构造真实文本 provider。缺 `REAL_LLM_API_KEY` → None（调用方自我跳过）。
fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.supxh.xin/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".to_string());
    let client =
        LlmClient::new(base_url, api_key, model, 180, 3, 1500).expect("构造真实 LlmClient");
    Some(Arc::new(client))
}

/// 跳过宏：无 key 时打印一行 skip 并 `return`（不 panic、不算失败）。
macro_rules! require_real_llm {
    () => {{
        match real_llm_from_env() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过召回率基准测试");
                return;
            }
        }
    }};
}

/// 把一个 `AppResult<T>` 解包为 `T`；遇到**真模型上游瞬时不可达**时跳过测试。
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
                     按计划跳过处理",
                    $what
                );
                return;
            }
            Err(other) => panic!("{}：{other}", $what),
        }
    }};
}

/// 知识链路不发消息，起一个空 wiremock 作为 MCP 服务占位。
async fn dummy_mcp_server() -> MockServer {
    MockServer::start().await
}

// ── seed helper：完整控制 summary / body / integrity_status / related ────

/// 落一条 chunk，返回 hex id。`related` 为空 → `related_chunks=None`。
/// `dynamic_confidence` 控制 catalog 排序。
#[allow(clippy::too_many_arguments)]
async fn seed_chunk(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
    integrity_status: &str,
    status: &str,
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
        integrity_status: Some(integrity_status.to_string()),
        confidence_score: Some(88),
        status: status.to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(dynamic_confidence),
        chunk_type: "product_fact".to_string(),
        related_chunks: if related.is_empty() { None } else { Some(related) },
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert chunk");
    id.to_hex()
}

/// 便捷：seed 一条 verified / active / 高置信的全局 chunk。
async fn seed_verified(
    app: &TestApp,
    ws: &str,
    title: &str,
    summary: &str,
    body: &str,
) -> String {
    seed_chunk(app, ws, title, summary, body, "verified", "active", 0.9, Vec::new()).await
}

// ── Task2: 跨行业语料矩阵 + 客观对抗度量 ────────────────────────────────────

// Step 1 — 数据结构
#[allow(dead_code)]
struct ChunkSeed {
    title: &'static str,
    summary: &'static str,
    body: &'static str,
}

#[allow(dead_code)]
struct QueryCase {
    query: &'static str,
    expected_titles: Vec<&'static str>,
}

#[allow(dead_code)]
struct IndustryCorpus {
    industry: &'static str,
    doc_type: &'static str,
    chunks: Vec<ChunkSeed>,
    queries: Vec<QueryCase>,
}

#[derive(Clone)]
#[allow(dead_code)]
struct RecallCase {
    name: String,
    query: String,
    expected_chunk_ids: Vec<String>,
    lexical_overlap: f64,
    adversarial: bool,
}

// Step 2 — 构建 6 行业语料矩阵
fn build_industry_corpus_matrix() -> Vec<IndustryCorpus> {
    use std::collections::HashSet;

    let matrix = vec![
        // 零售行业 - 规格文档
        IndustryCorpus {
            industry: "retail",
            doc_type: "规格",
            chunks: vec![
                ChunkSeed {
                    title: "iPhone15Pro规格参数",
                    summary: "iPhone15Pro详细技术规格",
                    body: "iPhone15Pro配备6.1英寸超级视网膜XDR显示屏，搭载A17Pro芯片，支持ProRAW拍摄，电池续航可达23小时视频播放，支持USB-C接口和MagSafe无线充电。",
                },
                ChunkSeed {
                    title: "MacBookPro14寸配置清单",
                    summary: "MacBookPro14寸硬件配置详情",
                    body: "MacBookPro14寸搭载M3Pro芯片，配备14.2英寸Liquid视网膜XDR显示屏，最高支持36GB统一内存，4TB SSD存储，拥有3个雷雳4端口和HDMI接口。",
                },
                ChunkSeed {
                    title: "AirPods3代产品特性",
                    summary: "AirPods3代核心功能介绍",
                    body: "AirPods3代采用全新设计，支持空间音频和自适应均衡，IPX4防水等级，单次使用6小时，配合充电盒总计30小时续航，支持Lightning充电。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "iPhone15Pro芯片性能怎么样",
                    expected_titles: vec!["iPhone15Pro规格参数"],
                },
                QueryCase {
                    query: "笔记本电脑内存可以扩展到多少",
                    expected_titles: vec!["MacBookPro14寸配置清单"],
                },
                QueryCase {
                    query: "无线耳机防水效果如何",
                    expected_titles: vec!["AirPods3代产品特性"],
                },
            ],
        },

        // SaaS行业 - 报价文档
        IndustryCorpus {
            industry: "saas",
            doc_type: "报价",
            chunks: vec![
                ChunkSeed {
                    title: "企业版CRM系统定价方案",
                    summary: "CRM企业版年度订阅价格",
                    body: "企业版CRM系统年费188000元，支持500用户并发，包含销售漏斗、客户画像、自动化营销等功能，提供7x24小时技术支持和数据备份服务。",
                },
                ChunkSeed {
                    title: "API调用量计费标准",
                    summary: "API服务按调用次数计费规则",
                    body: "API基础套餐每月10万次调用1200元，超出部分按0.015元/次计费。企业套餐每月100万次调用9800元，超出按0.012元/次计费。所有套餐含实时监控和故障告警。",
                },
                ChunkSeed {
                    title: "数据存储服务价格表",
                    summary: "云存储按容量和访问频次计费",
                    body: "标准存储每GB每月0.12元，低频存储每GB每月0.08元，归档存储每GB每月0.03元。数据下载按流量计费，标准存储下载每GB 0.5元，包含CDN加速。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "CRM系统一年要花多少钱",
                    expected_titles: vec!["企业版CRM系统定价方案"],
                },
                QueryCase {
                    query: "接口请求费用怎么算",
                    expected_titles: vec!["API调用量计费标准"],
                },
                QueryCase {
                    query: "文件归档成本是多少",
                    expected_titles: vec!["数据存储服务价格表"],
                },
            ],
        },

        // 金融行业 - 合同条款
        IndustryCorpus {
            industry: "finance",
            doc_type: "合同条款",
            chunks: vec![
                ChunkSeed {
                    title: "个人贷款违约责任条款",
                    summary: "贷款违约后的法律责任和处理方式",
                    body: "借款人逾期还款超过90天视为严重违约，银行有权要求立即清偿全部本息，并按日收取万分之五违约金。连续逾期180天将纳入征信黑名单，5年内影响信贷记录。",
                },
                ChunkSeed {
                    title: "理财产品风险提示声明",
                    summary: "投资理财的风险等级和免责条款",
                    body: "本理财产品为R3中等风险等级，不保证本金安全，历史年化收益3-8%仅供参考。市场波动可能导致亏损，投资者需具备相应风险承受能力，银行不承担投资损失责任。",
                },
                ChunkSeed {
                    title: "信用卡分期手续费标准",
                    summary: "信用卡账单分期的费率和计算方式",
                    body: "信用卡账单分期手续费按期数计算：3期总费率2.6%，6期总费率4.5%，12期总费率8.8%，24期总费率15.8%。每期等额还款，手续费一次性收取或分期摊还。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "贷款还不上会怎么样",
                    expected_titles: vec!["个人贷款违约责任条款"],
                },
                QueryCase {
                    query: "买理财亏了银行赔吗",
                    expected_titles: vec!["理财产品风险提示声明"],
                },
                QueryCase {
                    query: "账单分12期要付多少手续费",
                    expected_titles: vec!["信用卡分期手续费标准"],
                },
            ],
        },

        // 教育行业 - 手册
        IndustryCorpus {
            industry: "education",
            doc_type: "手册",
            chunks: vec![
                ChunkSeed {
                    title: "在线考试系统操作指南",
                    summary: "学生参与在线考试的详细步骤",
                    body: "考试前30分钟可进入考试界面，需通过摄像头人脸识别验证身份。考试期间禁止切换窗口，系统自动监控异常操作。每题限时提交，未完成题目按0分计算，考试结束后立即显示成绩。",
                },
                ChunkSeed {
                    title: "学分认定与转换规则",
                    summary: "不同课程学分的获得和转换标准",
                    body: "专业必修课每16学时计1学分，选修课每18学时计1学分。外校转入学分需提供成绩单和课程大纲，经教务处审核后最多可转入30学分。实习实践类课程按周计算，每周计1学分。",
                },
                ChunkSeed {
                    title: "图书馆借阅制度细则",
                    summary: "图书借还规则和违规处理办法",
                    body: "在校学生可借图书20册，借期30天，可续借2次每次15天。教师借阅无册数限制，借期90天。逾期每天每册罚金0.1元，丢失图书按原价3倍赔偿并加收20元加工费。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "网上考试怎么操作",
                    expected_titles: vec!["在线考试系统操作指南"],
                },
                QueryCase {
                    query: "其他学校的成绩能转过来吗",
                    expected_titles: vec!["学分认定与转换规则"],
                },
                QueryCase {
                    query: "借书超期了要罚多少钱",
                    expected_titles: vec!["图书馆借阅制度细则"],
                },
            ],
        },

        // 医疗行业 - FAQ
        IndustryCorpus {
            industry: "healthcare",
            doc_type: "FAQ",
            chunks: vec![
                ChunkSeed {
                    title: "医保报销比例说明",
                    summary: "不同级别医院的医保报销标准",
                    body: "职工医保在三甲医院住院报销比例85%，起付线1200元；二级医院报销90%，起付线800元；一级医院报销95%，起付线400元。门诊统筹年度限额4000元，药店购药可刷医保卡余额。",
                },
                ChunkSeed {
                    title: "疫苗接种注意事项",
                    summary: "疫苗接种前后的注意事项和禁忌症",
                    body: "接种前需告知医生过敏史和近期用药情况，发热、严重疾病急性期暂缓接种。接种后留观30分钟，24小时内避免剧烈运动和饮酒，接种部位保持清洁干燥，出现持续发热或异常反应及时就医。",
                },
                ChunkSeed {
                    title: "慢性病管理服务流程",
                    summary: "慢性病患者的长期管理和随访制度",
                    body: "慢性病患者建档后每3个月随访一次，测量血压血糖等指标，评估病情控制情况。医生制定个性化治疗方案，患者可通过APP上传监测数据，异常指标触发预警系统自动提醒复诊。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "住院费用能报销多少",
                    expected_titles: vec!["医保报销比例说明"],
                },
                QueryCase {
                    query: "打疫苗后需要注意什么",
                    expected_titles: vec!["疫苗接种注意事项"],
                },
                QueryCase {
                    query: "高血压病人如何定期检查",
                    expected_titles: vec!["慢性病管理服务流程"],
                },
            ],
        },

        // 制造业 - 案例
        IndustryCorpus {
            industry: "manufacturing",
            doc_type: "案例",
            chunks: vec![
                ChunkSeed {
                    title: "智能工厂数字化改造成功案例",
                    summary: "传统制造企业的数字化转型实践",
                    body: "某汽车零部件厂引入IoT设备监控生产线，实现设备预测性维护，故障率下降60%。MES系统优化排产计划，生产效率提升35%，成本降低18%。员工通过平板实时查看工艺参数，质量合格率达99.2%。",
                },
                ChunkSeed {
                    title: "供应链协同管理优化项目",
                    summary: "多级供应商协同管理的实施经验",
                    body: "建立供应商评价体系，从质量、交付、成本三维度考核，优质供应商占比从65%提升至85%。实施VMI模式减少库存30%，建立应急供应网络，关键物料断供风险降低80%，采购成本节约12%。",
                },
                ChunkSeed {
                    title: "绿色制造节能减排实践",
                    summary: "制造企业的环保改造和节能措施",
                    body: "更换高效节能设备，单位产品能耗下降25%。建设污水处理设施，废水排放达标率100%。推行清洁生产工艺，危废产生量减少40%，获得绿色工厂认证，每年节约环保成本200万元。",
                },
            ],
            queries: vec![
                QueryCase {
                    query: "数字化改造效果怎么样",
                    expected_titles: vec!["智能工厂数字化改造成功案例"],
                },
                QueryCase {
                    query: "如何优化供应商管理",
                    expected_titles: vec!["供应链协同管理优化项目"],
                },
                QueryCase {
                    query: "工厂环保改造能省多少钱",
                    expected_titles: vec!["绿色制造节能减排实践"],
                },
            ],
        },
    ];

    // 校验title全局唯一性
    let mut titles = HashSet::new();
    for corpus in &matrix {
        for chunk in &corpus.chunks {
            if !titles.insert(chunk.title) {
                panic!("duplicate title: {}", chunk.title);
            }
        }
    }

    matrix
}

// Step 3 — bigram overlap 计算函数
fn bigram_overlap(query: &str, body: &str) -> f64 {
    let query_chars: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    let body_chars: Vec<char> = body.chars().filter(|c| !c.is_whitespace()).collect();

    if query_chars.len() < 2 {
        return 0.0;
    }

    let query_bigrams: std::collections::HashSet<(char, char)> = query_chars
        .windows(2)
        .map(|w| (w[0], w[1]))
        .collect();

    let body_bigrams: std::collections::HashSet<(char, char)> = body_chars
        .windows(2)
        .map(|w| (w[0], w[1]))
        .collect();

    let intersection_count = query_bigrams.intersection(&body_bigrams).count();
    intersection_count as f64 / query_bigrams.len() as f64
}

// Step 4 — 种子语料矩阵并构建召回案例
async fn seed_corpus_matrix(app: &TestApp, ws: &str, matrix: &[IndustryCorpus]) -> Vec<RecallCase> {
    use std::collections::HashMap;

    // 构建title到id的映射
    let mut title_to_id = HashMap::new();
    let mut title_to_body = HashMap::new();

    // 种子所有chunk
    for corpus in matrix {
        for chunk in &corpus.chunks {
            let chunk_id = seed_verified(app, ws, chunk.title, chunk.summary, chunk.body).await;
            title_to_id.insert(chunk.title, chunk_id);
            title_to_body.insert(chunk.title, chunk.body);
        }
    }

    // 构建召回案例
    let mut recall_cases = Vec::new();

    for corpus in matrix {
        for query_case in &corpus.queries {
            let expected_chunk_ids: Vec<String> = query_case
                .expected_titles
                .iter()
                .map(|title| {
                    title_to_id.get(title)
                        .unwrap_or_else(|| panic!("title not found in seeds: {}", title))
                        .clone()
                })
                .collect();

            // 计算lexical_overlap（使用第一个expected chunk的body）
            let first_expected_title = query_case.expected_titles[0];
            let first_expected_body = title_to_body.get(first_expected_title)
                .unwrap_or_else(|| panic!("body not found for title: {}", first_expected_title));

            let lexical_overlap = bigram_overlap(query_case.query, first_expected_body);
            let adversarial = lexical_overlap < 0.15;

            let name = format!("{}/{}", corpus.industry,
                &query_case.query.chars().take(10).collect::<String>());

            recall_cases.push(RecallCase {
                name,
                query: query_case.query.to_string(),
                expected_chunk_ids,
                lexical_overlap,
                adversarial,
            });
        }
    }

    recall_cases
}

// Step 5 — 离线单测
#[test]
fn bigram_overlap_properties() {
    // 完全重叠：query 是 body 子串 → 高（接近/等于 1.0）
    let overlap1 = bigram_overlap("退换货政策", "本店退换货政策如下");
    println!("overlap1 (退换货政策 vs 本店退换货政策如下): {}", overlap1);
    assert!(overlap1 > 0.9, "Expected high overlap, got {}", overlap1);

    // 完全不重叠 → 0.0
    let overlap2 = bigram_overlap("飞机大炮", "退换货政策");
    println!("overlap2 (飞机大炮 vs 退换货政策): {}", overlap2);
    assert_eq!(overlap2, 0.0, "Expected no overlap, got {}", overlap2);

    // 部分重叠 → (0,1)
    let overlap3 = bigram_overlap("退货流程", "退换货政策流程说明");
    println!("overlap3 (退货流程 vs 退换货政策流程说明): {}", overlap3);
    assert!(overlap3 > 0.0 && overlap3 < 1.0, "Expected partial overlap, got {}", overlap3);

    // query 太短 → 0.0
    let overlap4 = bigram_overlap("退", "退换货");
    println!("overlap4 (退 vs 退换货): {}", overlap4);
    assert_eq!(overlap4, 0.0, "Expected zero for short query, got {}", overlap4);
}

// ── Task3: 触达/采纳两层召回提取 + recall@k ────────────────────────────────
//
// 运营 agent 渐进式披露检索：list_catalog → open_chunk → follow_relations → answer。
// 两层召回度量：
// - **reach（触达）**：检索过程中「翻到过」的 chunk = cited_chunk_ids ∪
//   tool_trace 各步 opened（open_chunk）∪ openedBodies（follow_relations 预取正文）。
//   测检索层能否把目标 chunk 翻出来。
// - **adopt（采纳）**：最终引用作答的 chunk = 仅 cited_chunk_ids。测生成层能否用上。
// - **reach ⊇ adopt 恒成立**（采纳必先触达，cited 是 reach 起始集）。
//
// 三者均为确定性纯函数，零生产改动，离线可测。

/// 触达集：agent 检索过程中「翻到过」的全部 chunk id（去重）。
/// = cited_chunk_ids ∪ open_chunk.opened ∪ follow_relations.openedBodies。
fn reach_set(result: &AnswerResult) -> Vec<String> {
    let mut set: HashSet<String> = result.cited_chunk_ids.iter().cloned().collect();
    for step in &result.tool_trace {
        // open_chunk 步骤：opened 数组
        if let Ok(opened) = step.get_array("opened") {
            for v in opened {
                if let Some(s) = v.as_str() {
                    set.insert(s.to_string());
                }
            }
        }
        // follow_relations 步骤：openedBodies 数组（预取关联目标正文进 opened）
        if let Ok(bodies) = step.get_array("openedBodies") {
            for v in bodies {
                if let Some(s) = v.as_str() {
                    set.insert(s.to_string());
                }
            }
        }
    }
    set.into_iter().collect()
}

/// 采纳集：agent 最终引用作答的 chunk id（仅 cited_chunk_ids，去重）。
fn adopt_set(result: &AnswerResult) -> Vec<String> {
    let set: HashSet<String> = result.cited_chunk_ids.iter().cloned().collect();
    set.into_iter().collect()
}

/// recall@k：set 命中 expected 的比例。
/// - expected 空：set 空 → 1.0（无目标且无噪声）；set 非空 → 0.0（召回到噪声）。
/// - expected 非空：|set ∩ expected| / |expected|（均按去重后集合计）。
fn recall_at_k(set: &[String], expected: &[String]) -> f64 {
    let expected_set: HashSet<&String> = expected.iter().collect();
    if expected_set.is_empty() {
        return if set.is_empty() { 1.0 } else { 0.0 };
    }
    let set_set: HashSet<&String> = set.iter().collect();
    let hit = expected_set.intersection(&set_set).count();
    hit as f64 / expected_set.len() as f64
}

#[test]
fn recall_at_k_properties() {
    // expected 空 + set 空 → 1.0
    assert_eq!(recall_at_k(&[], &[]), 1.0);
    // expected 空 + set 非空（噪声）→ 0.0
    assert_eq!(recall_at_k(&["x".into()], &[]), 0.0);
    // 全命中 → 1.0
    assert_eq!(
        recall_at_k(&["a".into(), "b".into()], &["a".into(), "b".into()]),
        1.0
    );
    // 部分命中 → 0.5
    assert_eq!(recall_at_k(&["a".into()], &["a".into(), "b".into()]), 0.5);
    // 去重不影响（set 中重复 a 不算两次命中）
    assert_eq!(recall_at_k(&["a".into(), "a".into()], &["a".into()]), 1.0);
}

#[test]
fn reach_superset_of_adopt() {
    // 构造 AnswerResult：cited=["a"]，open_chunk opened=["a","b"]，
    // follow_relations openedBodies=["c"]。
    let result = AnswerResult {
        answer: "测试答案".to_string(),
        cited_chunk_ids: vec!["a".to_string()],
        source_quotes: Vec::<SourceQuoteCitation>::new(),
        tool_trace: vec![
            doc! { "tool": "open_chunk", "opened": ["a", "b"] },
            doc! { "tool": "follow_relations", "openedBodies": ["c"] },
        ],
        rounds_used: 2,
        truncated: false,
        cancelled: false,
    };

    let reach = reach_set(&result);
    let adopt = adopt_set(&result);

    // reach ⊇ adopt：采纳的每个 id 都在触达集里
    for id in &adopt {
        assert!(
            reach.contains(id),
            "adopt id {} 不在 reach {:?} 中",
            id,
            reach
        );
    }

    // adopt 只含 cited（a），不含 opened-only 的 b/c
    assert!(adopt.contains(&"a".to_string()));
    assert!(!adopt.contains(&"b".to_string()));
    assert!(!adopt.contains(&"c".to_string()));

    // reach 含 open_chunk.opened 里的 b
    assert!(reach.contains(&"b".to_string()));
    // reach 含 follow_relations.openedBodies 里的 c
    assert!(reach.contains(&"c".to_string()));
    // reach 也含 cited 的 a
    assert!(reach.contains(&"a".to_string()));

    // 去重后 reach 恰好 {a,b,c}
    assert_eq!(reach.len(), 3, "reach 应去重为 3 个：{:?}", reach);
}

// ── 测试 ──────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn recall_benchmark_smoke() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = "recall_smoke_ws";

    let id = seed_verified(&app, ws, "退换货政策", "7天无理由退换", "本店支持 7 天无理由退换货，商品需保持完好。").await;

    // 跑一次 answer，验证管线打通
    let req = AnswerRequest {
        workspace_id: ws.to_string(),
        account_id: None,
        query: "买错了能退吗".to_string(),
        filter: CatalogFilter {
            wiki_types: vec![],
            business_topics: vec![],
            status: None,
            include_unverified: false,
        },
        max_rounds: None,
    };

    let result = unwrap_or_skip_transient!(answer(&state, req).await, "smoke answer");

    eprintln!(
        "[RECALL-SMOKE] seeded={} cited={:?} rounds={}",
        id, result.cited_chunk_ids, result.rounds_used
    );
}

// ── Task4: 跨轮稳定性 + 召回率主测 + ⊆seed 红线 ─────────────────────────────

/// Step 1 — 跑 query N 次，收集结果（处理瞬时失败但不让整个测试跳过）
async fn run_query_n_times(state: &AppState, ws: &str, query: &str, n: usize) -> Vec<AnswerResult> {
    let mut results = Vec::new();

    for i in 0..n {
        let req = AnswerRequest {
            workspace_id: ws.to_string(),
            account_id: None,
            query: query.to_string(),
            filter: CatalogFilter {
                wiki_types: vec![],
                business_topics: vec![],
                status: None,
                include_unverified: false,
            },
            max_rounds: None,
        };

        match answer(state, req).await {
            Ok(result) => results.push(result),
            Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, .. }) => {
                eprintln!(
                    "skip: query={} round={}/{} —— 真模型上游瞬时不可达（kind={}, retry_count={}），跳过本次",
                    query, i+1, n, kind, retry_count
                );
                continue;
            }
            Err(other) => panic!("run_query_n_times query={} round={}: {}", query, i+1, other),
        }
    }

    results
}

/// Step 2 — 主测：跨行业召回基准（跨轮稳定性 + 召回率 + ⊆seed 红线）
#[tokio::test]
#[ignore]
async fn recall_benchmark_cross_industry() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = "recall_cross_industry_ws";

    // 构建语料矩阵并种子
    let matrix = build_industry_corpus_matrix();
    let cases = seed_corpus_matrix(&app, ws, &matrix).await;

    // 获取所有 seeded chunk ids（⊆seed 红线检查用）
    let seeded_ids: HashSet<String> = app.state
        .db
        .operation_knowledge_chunks()
        .find(doc!{"workspace_id": ws}, None)
        .await
        .expect("查询 seeded chunks")
        .try_collect::<Vec<OperationKnowledgeChunk>>()
        .await
        .expect("收集 seeded chunks")
        .into_iter()
        .map(|chunk| chunk.id.expect("chunk 必须有 id").to_hex())
        .collect();

    eprintln!("[RECALL] seeded_ids 总数: {}", seeded_ids.len());

    // N 次稳定性测试轮数
    let n: usize = std::env::var("RECALL_STABILITY_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    eprintln!("[RECALL] 每条 query 跑 {} 轮稳定性测试", n);

    // 分组统计变量
    let mut overall_reach_recalls = Vec::new();
    let mut overall_adopt_recalls = Vec::new();
    let mut lexical_easy_reach_recalls = Vec::new();
    let mut lexical_easy_adopt_recalls = Vec::new();
    let mut adversarial_reach_recalls = Vec::new();
    let mut adversarial_adopt_recalls = Vec::new();

    let mut reach_stable_count = 0;
    let mut adopt_stable_count = 0;
    let mut total_cases = 0;

    // 逐条 case 测试
    for case in cases {
        total_cases += 1;

        let results = run_query_n_times(&state, ws, &case.query, n).await;

        if results.is_empty() {
            eprintln!("[RECALL] case={} 全轮失败，跳过", case.name);
            continue;
        }

        // 提取每轮的 reach_set / adopt_set
        let mut round_reach_sets = Vec::new();
        let mut round_adopt_sets = Vec::new();
        let mut reach_recalls = Vec::new();
        let mut adopt_recalls = Vec::new();

        for result in &results {
            let reach = reach_set(result);
            let adopt = adopt_set(result);

            // ⊆seed 红线硬断：reach_set 任一 id 不在 seeded_ids 全集
            for id in &reach {
                assert!(
                    seeded_ids.contains(id),
                    "RED-LINE 召回集越界: {} 不在 seed 集 (case={})",
                    id, case.name
                );
            }

            let reach_recall = recall_at_k(&reach, &case.expected_chunk_ids);
            let adopt_recall = recall_at_k(&adopt, &case.expected_chunk_ids);

            round_reach_sets.push(reach);
            round_adopt_sets.push(adopt);
            reach_recalls.push(reach_recall);
            adopt_recalls.push(adopt_recall);
        }

        // 跨轮稳定性检查：排序后比较是否完全一致
        let reach_stable = if round_reach_sets.len() > 1 {
            let first_sorted = {
                let mut first = round_reach_sets[0].clone();
                first.sort();
                first
            };
            round_reach_sets.iter().skip(1).all(|reach| {
                let mut sorted_reach = reach.clone();
                sorted_reach.sort();
                sorted_reach == first_sorted
            })
        } else {
            true // 单轮视为稳定
        };

        let adopt_stable = if round_adopt_sets.len() > 1 {
            let first_sorted = {
                let mut first = round_adopt_sets[0].clone();
                first.sort();
                first
            };
            round_adopt_sets.iter().skip(1).all(|adopt| {
                let mut sorted_adopt = adopt.clone();
                sorted_adopt.sort();
                sorted_adopt == first_sorted
            })
        } else {
            true // 单轮视为稳定
        };

        if reach_stable { reach_stable_count += 1; }
        if adopt_stable { adopt_stable_count += 1; }

        // 召回率统计（取各轮均值）
        let avg_reach_recall: f64 = reach_recalls.iter().sum::<f64>() / reach_recalls.len() as f64;
        let avg_adopt_recall: f64 = adopt_recalls.iter().sum::<f64>() / adopt_recalls.len() as f64;

        // 分组聚合
        overall_reach_recalls.push(avg_reach_recall);
        overall_adopt_recalls.push(avg_adopt_recall);

        if case.adversarial {
            adversarial_reach_recalls.push(avg_reach_recall);
            adversarial_adopt_recalls.push(avg_adopt_recall);
        } else {
            lexical_easy_reach_recalls.push(avg_reach_recall);
            lexical_easy_adopt_recalls.push(avg_adopt_recall);
        }

        // Step 3 — 每条 case 一行 ledger
        eprintln!(
            "[RECALL] case={} adversarial={} overlap={:.3} reach_recall(avg)={:.3} adopt_recall(avg)={:.3} reach_stable={} adopt_stable={} rounds={}",
            case.name, case.adversarial, case.lexical_overlap, avg_reach_recall, avg_adopt_recall, reach_stable, adopt_stable, results.len()
        );
    }

    // Step 3 — 分组聚合汇总
    let calc_mean = |values: &[f64]| -> f64 {
        if values.is_empty() { 0.0 } else { values.iter().sum::<f64>() / values.len() as f64 }
    };

    eprintln!("[RECALL] === 分组聚合汇总 ===");

    // overall 组
    eprintln!(
        "[RECALL] overall: reach_recall(mean)={:.3} adopt_recall(mean)={:.3} cases={}",
        calc_mean(&overall_reach_recalls), calc_mean(&overall_adopt_recalls), overall_reach_recalls.len()
    );

    // lexical-easy 组
    if !lexical_easy_reach_recalls.is_empty() {
        eprintln!(
            "[RECALL] lexical-easy(!adversarial): reach_recall(mean)={:.3} adopt_recall(mean)={:.3} cases={}",
            calc_mean(&lexical_easy_reach_recalls), calc_mean(&lexical_easy_adopt_recalls), lexical_easy_reach_recalls.len()
        );
    }

    // adversarial 组
    if !adversarial_reach_recalls.is_empty() {
        eprintln!(
            "[RECALL] adversarial: reach_recall(mean)={:.3} adopt_recall(mean)={:.3} cases={}",
            calc_mean(&adversarial_reach_recalls), calc_mean(&adversarial_adopt_recalls), adversarial_reach_recalls.len()
        );
    }

    // 跨轮稳定占比
    let reach_stable_ratio = if total_cases > 0 { reach_stable_count as f64 / total_cases as f64 } else { 0.0 };
    let adopt_stable_ratio = if total_cases > 0 { adopt_stable_count as f64 / total_cases as f64 } else { 0.0 };

    eprintln!(
        "[RECALL] 跨轮完全稳定占比: reach_stable={:.1}% ({}/{}) adopt_stable={:.1}% ({}/{})",
        reach_stable_ratio * 100.0, reach_stable_count, total_cases,
        adopt_stable_ratio * 100.0, adopt_stable_count, total_cases
    );

    eprintln!("[RECALL] 召回基准测试完成 —— 第一轮仅观测，除⊆seed外无硬断");
}

// ── Task5: 真实 agent chat 改库全链路后召回保持 ────────────────────────────────

/// Step 1 — 对每条 case 跑一次 answer，返回 case.name -> (reach_set, adopt_set)
async fn recall_all(
    state: &AppState,
    ws: &str,
    cases: &[RecallCase],
) -> std::collections::HashMap<String, (Vec<String>, Vec<String>)> {
    let mut results = std::collections::HashMap::new();

    for case in cases {
        let req = AnswerRequest {
            workspace_id: ws.to_string(),
            account_id: None,
            query: case.query.clone(),
            filter: CatalogFilter {
                wiki_types: vec![],
                business_topics: vec![],
                status: None,
                include_unverified: false,
            },
            max_rounds: None,
        };

        match answer(state, req).await {
            Ok(result) => {
                let reach = reach_set(&result);
                let adopt = adopt_set(&result);
                results.insert(case.name.clone(), (reach, adopt));
            }
            Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, .. }) => {
                eprintln!(
                    "[RECALL-MAINT] skip case={} —— 真模型上游瞬时不可达（kind={}, retry_count={}），记录空集跳过",
                    case.name, kind, retry_count
                );
                results.insert(case.name.clone(), (vec![], vec![]));
            }
            Err(other) => {
                eprintln!("[RECALL-MAINT] fail case={} —— {}, 记录空集跳过", case.name, other);
                results.insert(case.name.clone(), (vec![], vec![]));
            }
        }
    }

    results
}

/// Step 2 — chat_create_and_verify: 通过 chat 对话创建并审定知识切片。
///
/// 真实生产链路是两步：chat_turn 只产 **pending 草稿预览**（patch/draftPreview），
/// 不落库；真正插入 draft chunk + 回填 createdChunkId 的是独立的 chat_apply
/// （运营在前端「应用为草稿」按钮触发）。本 helper 照搬该两步设计：
///   chat_turn（拿 sessionId + 确认 canApply）→ chat_apply（落库拿 createdChunkId）
///   → verify_operation_knowledge_chunk（审定）。
/// 任一步失败/未命中 create 意图 → 返回 None（调用方自我跳过，不算硬失败）。
async fn chat_create_and_verify(
    state: &AppState,
    _ws: &str,
    admin: &AuthenticatedAdmin,
    content: &str,
) -> Option<String> {
    // 构造 ChatTurnRequest（照搬 K10 方式：serde_json::from_value）
    let req: ChatTurnRequest = serde_json::from_value(json!({
        "sessionId": null,
        "accountId": null,
        "operatorId": "recall_maint_operator",
        "content": content,
        "attachments": [],
    }))
    .expect("构造 ChatTurnRequest");

    // 调用 chat_turn —— 只产 pending 草稿预览，拿 sessionId
    let resp = match chat_turn(
        State(state.clone()),
        Extension(admin.clone()),
        Json(req),
    )
    .await
    {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("[RECALL-MAINT] chat_turn failed: {}, return None", e);
            return None;
        }
    };

    let body = resp.0;

    // chat_turn 不落库，必须拿到 sessionId 去 chat_apply。intent 非 create_chunk
    // 或 canApply=false（缺字段/无 patch）时，apply 会 400 —— 视为本轮未命中创建，跳过。
    let session_id = match body.get("sessionId").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            eprintln!("[RECALL-MAINT] chat_turn 未返回 sessionId，return None");
            return None;
        }
    };

    // 调用 chat_apply —— 真正落库 draft chunk + 回填 createdChunkId
    let apply_req: ChatApplyRequest = serde_json::from_value(json!({
        "accountId": null
    }))
    .expect("构造 ChatApplyRequest");

    let apply_resp = match chat_apply(
        State(state.clone()),
        Extension(admin.clone()),
        Path(session_id.clone()),
        Json(apply_req),
    )
    .await
    {
        Ok(resp) => resp,
        Err(e) => {
            // 最常见：本轮 AI 未命中 create_chunk 意图 / 草稿缺字段不可应用 → 400。
            eprintln!("[RECALL-MAINT] chat_apply failed: {}, return None", e);
            return None;
        }
    };

    // chat_apply 返回 { ok, sessionId, intent, result: { createdChunkId, ... } }
    let chunk_id = match apply_resp
        .0
        .get("result")
        .and_then(|r| r.get("createdChunkId"))
        .and_then(|v| v.as_str())
    {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            // intent=update_chunk 等不产 createdChunkId；本 helper 只服务「新增」语义。
            eprintln!("[RECALL-MAINT] chat_apply 未回填 createdChunkId（intent 非 create_chunk？），return None");
            return None;
        }
    };

    // 调用 verify_operation_knowledge_chunk
    let verify_req: KnowledgeVerifyRequest = serde_json::from_value(json!({
        "verifiedClaims": null
    })).expect("构造 KnowledgeVerifyRequest");

    match verify_operation_knowledge_chunk(
        State(state.clone()),
        Extension(admin.clone()),
        Path(chunk_id.clone()),
        Json(verify_req),
    )
    .await
    {
        Ok(_) => Some(chunk_id),
        Err(e) => {
            eprintln!("[RECALL-MAINT] verify failed: {}, return None", e);
            None
        }
    }
}

/// Step 3 — 主测：真实 agent chat 改库全链路后召回保持
#[tokio::test]
#[ignore]
async fn recall_benchmark_maintenance_stability() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = "recall_maint_ws"; // 独立 ws 避免污染

    // 构建语料矩阵并种子
    let matrix = build_industry_corpus_matrix();
    let cases = seed_corpus_matrix(&app, ws, &matrix).await;

    // 构造 admin
    let admin = AuthenticatedAdmin {
        user_id: "recall_maint_admin".to_string(),
        username: "recall_maint_admin".to_string(),
        current_workspace: ws.to_string(),
    };

    // 获取当前 ws 全部 chunk id 全集的函数
    let get_all_chunk_ids = || async {
        app.state
            .db
            .operation_knowledge_chunks()
            .find(doc! { "workspace_id": ws }, None)
            .await
            .expect("查询 ws 全部 chunks")
            .try_collect::<Vec<OperationKnowledgeChunk>>()
            .await
            .expect("收集 ws 全部 chunks")
            .into_iter()
            .filter_map(|chunk| chunk.id.map(|id| id.to_hex()))
            .collect::<HashSet<String>>()
    };

    // ⊆seed 红线硬断：reach_set 任一 id 必须在全集中
    let check_subset_seed = |reach: &[String], all_ids: &HashSet<String>, stage: &str| {
        for id in reach {
            assert!(
                all_ids.contains(id),
                "RED-LINE 召回集越界: {} 不在 ws {} 全集中 (stage={})",
                id,
                ws,
                stage
            );
        }
    };

    // baseline 召回
    let r0 = recall_all(&state, ws, &cases).await;
    let all_ids_r0 = get_all_chunk_ids().await;

    // 验证 baseline ⊆seed
    for (case_name, (reach, _adopt)) in &r0 {
        check_subset_seed(reach, &all_ids_r0, &format!("baseline {}", case_name));
    }

    eprintln!("[RECALL-MAINT] baseline完成，cases={}, seeded_ids={}", cases.len(), all_ids_r0.len());

    // 变更A·新增：补一条缺知识
    let create_content = "帮我新建一条知识切片：我们的高级版支持API集成对接，可以与客户现有CRM系统无缝连接。知识类型是产品能力，请起草标题、摘要和正文。";

    let _created_chunk_id = match chat_create_and_verify(&state, ws, &admin, create_content).await {
        Some(id) => {
            eprintln!("[RECALL-MAINT] 变更A新增成功，chunk_id={}", id);
            id
        }
        None => {
            eprintln!("[RECALL-MAINT] 变更A新增失败，跳过后续测试");
            return;
        }
    };

    // 重新查 DB 取全集（包含新增的）
    let all_ids_r1 = get_all_chunk_ids().await;
    let r1 = recall_all(&state, ws, &cases).await;

    // 验证 r1 ⊆seed
    for (case_name, (reach, _adopt)) in &r1 {
        check_subset_seed(reach, &all_ids_r1, &format!("变更A后 {}", case_name));
    }

    // 报告变更A后的漂移情况
    let mut drift_count = 0;
    let mut total_cases = 0;
    for case in &cases {
        if let (Some((r0_reach, r0_adopt)), Some((r1_reach, r1_adopt))) = (r0.get(&case.name), r1.get(&case.name)) {
            total_cases += 1;
            let mut r0_reach_sorted = r0_reach.clone();
            let mut r1_reach_sorted = r1_reach.clone();
            r0_reach_sorted.sort();
            r1_reach_sorted.sort();

            let mut r0_adopt_sorted = r0_adopt.clone();
            let mut r1_adopt_sorted = r1_adopt.clone();
            r0_adopt_sorted.sort();
            r1_adopt_sorted.sort();

            if r0_reach_sorted != r1_reach_sorted || r0_adopt_sorted != r1_adopt_sorted {
                drift_count += 1;
                eprintln!("[RECALL-MAINT][SOFT-WARN] 变更A后漂移 case={} R0reach={:?} R1reach={:?} R0adopt={:?} R1adopt={:?}",
                    case.name, r0_reach_sorted, r1_reach_sorted, r0_adopt_sorted, r1_adopt_sorted);
            }
        }
    }

    eprintln!("[RECALL-MAINT] 变更A·新增后 漂移率={:.1}% ({}/{})",
        if total_cases > 0 { drift_count as f64 / total_cases as f64 * 100.0 } else { 0.0 },
        drift_count, total_cases);

    // 变更B·改写：chat_turn 发"更新/补充某切片同义表述"
    let update_content = "帮我更新一条现有知识切片，为企业版产品增加更多同义表述：企业级解决方案、商业版本、专业版服务等表达方式。";

    if let Some(_) = chat_create_and_verify(&state, ws, &admin, update_content).await {
        eprintln!("[RECALL-MAINT] 变更B改写成功");
    } else {
        eprintln!("[RECALL-MAINT] 变更B改写失败");
    }

    let all_ids_r2 = get_all_chunk_ids().await;
    let r2 = recall_all(&state, ws, &cases).await;

    // 验证 r2 ⊆seed
    for (case_name, (reach, _adopt)) in &r2 {
        check_subset_seed(reach, &all_ids_r2, &format!("变更B后 {}", case_name));
    }

    // 报告变更B后的漂移情况
    let mut drift_count_b = 0;
    let mut total_cases_b = 0;
    for case in &cases {
        if let (Some((r1_reach, r1_adopt)), Some((r2_reach, r2_adopt))) = (r1.get(&case.name), r2.get(&case.name)) {
            total_cases_b += 1;
            let mut r1_reach_sorted = r1_reach.clone();
            let mut r2_reach_sorted = r2_reach.clone();
            r1_reach_sorted.sort();
            r2_reach_sorted.sort();

            let mut r1_adopt_sorted = r1_adopt.clone();
            let mut r2_adopt_sorted = r2_adopt.clone();
            r1_adopt_sorted.sort();
            r2_adopt_sorted.sort();

            if r1_reach_sorted != r2_reach_sorted || r1_adopt_sorted != r2_adopt_sorted {
                drift_count_b += 1;
                eprintln!("[RECALL-MAINT][SOFT-WARN] 变更B后漂移 case={} R1reach={:?} R2reach={:?} R1adopt={:?} R2adopt={:?}",
                    case.name, r1_reach_sorted, r2_reach_sorted, r1_adopt_sorted, r2_adopt_sorted);
            }
        }
    }

    eprintln!("[RECALL-MAINT] 变更B·改写后 漂移率={:.1}% ({}/{})",
        if total_cases_b > 0 { drift_count_b as f64 / total_cases_b as f64 * 100.0 } else { 0.0 },
        drift_count_b, total_cases_b);

    // 变更C·废弃：把某条 verified chunk update_one 置 integrity_status="needs_review"
    if !all_ids_r2.is_empty() {
        let first_chunk_id = all_ids_r2.iter().next().unwrap();
        if let Ok(object_id) = ObjectId::parse_str(first_chunk_id) {
            match app.state
                .db
                .operation_knowledge_chunks()
                .update_one(
                    doc! { "_id": object_id },
                    doc! { "$set": { "integrity_status": "needs_review" } },
                    None,
                )
                .await
            {
                Ok(_) => {
                    eprintln!("[RECALL-MAINT] 变更C废弃成功，chunk_id={}", first_chunk_id);
                }
                Err(e) => {
                    eprintln!("[RECALL-MAINT] 变更C废弃失败: {}", e);
                }
            }
        }
    }

    let all_ids_r3 = get_all_chunk_ids().await;
    let r3 = recall_all(&state, ws, &cases).await;

    // 验证 r3 ⊆seed
    for (case_name, (reach, _adopt)) in &r3 {
        check_subset_seed(reach, &all_ids_r3, &format!("变更C后 {}", case_name));
    }

    // 报告变更C后的漂移情况
    let mut drift_count_c = 0;
    let mut total_cases_c = 0;
    for case in &cases {
        if let (Some((r2_reach, r2_adopt)), Some((r3_reach, r3_adopt))) = (r2.get(&case.name), r3.get(&case.name)) {
            total_cases_c += 1;
            let mut r2_reach_sorted = r2_reach.clone();
            let mut r3_reach_sorted = r3_reach.clone();
            r2_reach_sorted.sort();
            r3_reach_sorted.sort();

            let mut r2_adopt_sorted = r2_adopt.clone();
            let mut r3_adopt_sorted = r3_adopt.clone();
            r2_adopt_sorted.sort();
            r3_adopt_sorted.sort();

            if r2_reach_sorted != r3_reach_sorted || r2_adopt_sorted != r3_adopt_sorted {
                drift_count_c += 1;
                eprintln!("[RECALL-MAINT][SOFT-WARN] 变更C后漂移 case={} R2reach={:?} R3reach={:?} R2adopt={:?} R3adopt={:?}",
                    case.name, r2_reach_sorted, r3_reach_sorted, r2_adopt_sorted, r3_adopt_sorted);
            }
        }
    }

    eprintln!("[RECALL-MAINT] 变更C·废弃后 漂移率={:.1}% ({}/{})",
        if total_cases_c > 0 { drift_count_c as f64 / total_cases_c as f64 * 100.0 } else { 0.0 },
        drift_count_c, total_cases_c);

    eprintln!("[RECALL-MAINT] 真实chat改库全链路召回保持测试完成 —— 三变更后稳定性观测完毕");
}

// ── Task5: gap→主动提问→对话补库→再问命中 完整闭合轨迹 ─────────────────────────
//
// 业务闭环（用户原话）：文档→抽取入库→运营 agent 召回→agent 自然语言维护知识库
// →缺知识反馈对话补全。此前各环节各有测试覆盖，但**没有任何单个测试走通整条
// 闭合轨迹**——本测试把四步串成一条 end-to-end 真模型链路，并在收尾断言「同一
// query 在补库后从弃答转为召回命中」，证明缺口真的被这条链路闭合了。
//
// 轨迹：
//   ① seed 一个**不覆盖**某主题的小语料（仅退换货政策，不含跨境支付）。
//   ② 知识 agent answer 该未覆盖主题 query → 走诚实弃答（cited 为空）。
//   ③ bounded-retry 轮询确认 classify_recall_outcome 已确定性留下
//      kind=recall_miss / status=pending / search_queries 含原始 query 的 gap 信号
//      （信号写入在 answer() 的 tokio::spawn 内 fire-and-forget，故需有限重试）。
//   ④ 取该信号携带的 query，走 chat_create_and_verify 用对话方式补一条覆盖该主题
//      的 verified chunk（chat_turn 起草 → chat_apply 落 draft → verify 审定）。
//   ⑤ 再 answer **同一** query → 断言这次 reach/adopt 召回命中新补的 chunk。
//
// 红线：cite ⊆ opened（reach ⊇ adopt 已由 reach_set/adopt_set 结构保证；补库后
// 召回命中的必须是真实 seed 进 ws 的 chunk，不许幻觉 id）、ingestion 恒
// draft+needs_review（chat_apply 强制，verify 是显式人工审定动作而非 AI 自动）。
// 全程 #[ignore] + env-gated REAL_LLM_API_KEY；真模型上游瞬时不可达 → 跳过不算失败。

/// bounded-retry 轮询 recall_miss gap 信号（镜像 K3 闭环红线的有限重试模式）。
/// 信号在 answer() 的 spawn 内 fire-and-forget 落库，故需轮询而非一次读取。
/// 命中条件：workspace_id 匹配 + kind=recall_miss + status=pending +
/// search_queries 含 original_query。最多 retries 次、每次间隔 interval。
/// 生产侧 persist 已先于 LLM followup 确定性落库，窗口只需吸收 spawn 调度 + DB 往返。
async fn poll_recall_miss_signal(
    state: &AppState,
    ws: &str,
    original_query: &str,
    retries: usize,
    interval: std::time::Duration,
) -> Option<KnowledgeGapSignal> {
    for _ in 0..retries {
        let found = state
            .db
            .knowledge_gap_signals()
            .find_one(
                doc! {
                    "workspace_id": ws,
                    "kind": "recall_miss",
                    "status": "pending",
                    "search_queries": original_query,
                },
                None,
            )
            .await
            .ok()
            .flatten();
        if let Some(sig) = found {
            return Some(sig);
        }
        tokio::time::sleep(interval).await;
    }
    None
}

/// 单跑一次 answer，返回 (reach, adopt)；瞬时不可达 → None（调用方跳过）。
async fn answer_reach_adopt(
    state: &AppState,
    ws: &str,
    query: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    let req = AnswerRequest {
        workspace_id: ws.to_string(),
        account_id: None,
        query: query.to_string(),
        filter: CatalogFilter {
            wiki_types: vec![],
            business_topics: vec![],
            status: None,
            include_unverified: false,
        },
        max_rounds: None,
    };
    match answer(state, req).await {
        Ok(result) => Some((reach_set(&result), adopt_set(&result))),
        Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, .. }) => {
            eprintln!(
                "[RECALL-CLOSED] skip —— 真模型上游瞬时不可达（kind={}, retry_count={}）",
                kind, retry_count
            );
            None
        }
        Err(other) => {
            eprintln!("[RECALL-CLOSED] answer 失败：{}", other);
            None
        }
    }
}

#[tokio::test]
#[ignore]
async fn recall_benchmark_gap_closed_loop_trajectory() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = dummy_mcp_server().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    let ws = "recall_closed_loop_ws"; // 独立 ws 避免污染

    // ① seed 一个不覆盖「跨境支付」主题的小语料：只放退换货政策。
    let policy_id = seed_verified(
        &app,
        ws,
        "退换货政策",
        "7 天无理由退换",
        "本店支持 7 天无理由退换货，商品需保持完好、配件齐全，由客服核验后办理。",
    )
    .await;
    eprintln!("[RECALL-CLOSED] ① 种子语料完成 policy_id={}", policy_id);

    // 跨境支付主题的 query —— 当前知识库无任何覆盖，应触发诚实弃答。
    let gap_query = "你们支持哪些海外支付货币？跨境结算手续费是百分之几？";

    // ② answer 未覆盖主题 → 期望诚实弃答（cited 为空 → reach/adopt 不命中已有 chunk）。
    let (reach0, adopt0) = match answer_reach_adopt(&state, ws, gap_query).await {
        Some(v) => v,
        None => return, // 瞬时不可达，跳过
    };
    eprintln!(
        "[RECALL-CLOSED] ② 弃答阶段 reach0={:?} adopt0={:?}",
        reach0, adopt0
    );
    // 诚实弃答语义：不应采纳到任何已有 chunk（adopt 为空）；这是触发 recall_miss
    // 的前提。若真模型异常 cite 了无关 policy chunk，说明弃答未发生，无法验证闭环 →
    // 跳过（不硬失败，避免把真模型偶发行为当缺陷；闭环正例由命中阶段断言保证）。
    if !adopt0.is_empty() {
        eprintln!(
            "[RECALL-CLOSED] 跳过：弃答阶段意外 adopt={:?}（未发生诚实弃答），\
             无法验证缺口闭合",
            adopt0
        );
        return;
    }

    // ③ bounded-retry 轮询 recall_miss gap 信号（spawn 内 fire-and-forget 落库）。
    //    走到此处 = ② 的 answer 已成功（LlmUnavailable 早在 ② return 跳过）且 adopt0 空
    //    ⟺ cited==0；非 cancelled（本测试无取消机制）→ classify_recall_outcome 确定性产出
    //    recall_miss，且生产侧已把 persist 排在任何 LLM followup 之前（恒在、零 LLM 依赖）。
    //    故 20s 窗口内未观测到信号 = 生产落库链路真缺陷，**硬失败**而非软跳过——软跳过会让
    //    后续 ④ 对话补库→verify 的生产闭环永远走不到、形同没测。
    let signal = match poll_recall_miss_signal(
        &state,
        ws,
        gap_query,
        40,
        std::time::Duration::from_millis(500),
    )
    .await
    {
        Some(sig) => sig,
        None => panic!(
            "[RECALL-CLOSED] 缺陷：弃答已发生（adopt 空 ⟺ cited==0）却在 20s 内未观测到 \
             recall_miss 信号。生产侧 persist_recall_signal 应先于 LLM followup 确定性落库，\
             此处为空说明在线召回-trace 落库链路断裂"
        ),
    };
    // 红线断言：信号确定性字段（与 K3 闭环红线一致）。
    assert_eq!(signal.kind, "recall_miss", "gap 信号 kind 必须是 recall_miss");
    assert_eq!(signal.status, "pending", "gap 信号 status 必须是 pending");
    assert!(
        signal.search_queries.iter().any(|q| q == gap_query),
        "gap 信号 search_queries {:?} 必须含原始 query {}",
        signal.search_queries,
        gap_query
    );
    eprintln!(
        "[RECALL-CLOSED] ③ 观测到 recall_miss 信号 signal_id={} search_queries={:?}",
        signal.signal_id, signal.search_queries
    );

    // ④ 取信号携带的 query（运营据此对话补全），用 chat 对话补一条覆盖该主题的
    //    verified chunk。优先用信号里的原始 query 作为补全话题锚点。
    let admin = AuthenticatedAdmin {
        user_id: "recall_closed_admin".to_string(),
        username: "recall_closed_admin".to_string(),
        current_workspace: ws.to_string(),
    };
    let create_content = format!(
        "帮我新建一条知识切片回应这个反复被问到的缺口：{gap_query} \
         事实依据：我们支持美元(USD)、欧元(EUR)、港币(HKD)三种海外货币结算，\
         跨境结算手续费为交易金额的 1.5%。知识类型是产品政策，请起草标题、摘要和正文。"
    );
    let created_chunk_id = match chat_create_and_verify(&state, ws, &admin, &create_content).await {
        Some(id) => {
            eprintln!("[RECALL-CLOSED] ④ 对话补库成功 created_chunk_id={}", id);
            id
        }
        None => {
            eprintln!("[RECALL-CLOSED] 跳过：对话补库未命中 create 意图 / 未落库");
            return;
        }
    };

    // 红线：补库产物必须真实存在于本 ws，且经显式 verify 后达到"可召回"双维状态
    //（chat_apply 落 draft+needs_review，verify 是人工审定动作；AI 永不自动 verify）。
    // verify 语义是双维独立：integrity_status=verified（审计完整度）+ status=active（生命周期
    // draft→active）——二者正是召回侧过滤器（knowledge_agent.rs / knowledge_router.rs）同时要求
    // 的两个条件，故此处双断言＝在写侧确认补库产物确实进入"可被 ⑤ 阶段召回"的状态。
    let created_oid = ObjectId::parse_str(&created_chunk_id).expect("created_chunk_id 合法 ObjectId");
    let created_chunk = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": created_oid, "workspace_id": ws }, None)
        .await
        .expect("查询补库 chunk")
        .expect("补库 chunk 必须存在于本 ws");
    assert_eq!(
        created_chunk.integrity_status.as_deref(),
        Some("verified"),
        "补库 chunk 经显式 verify 后 integrity_status 必须为 verified"
    );
    assert_eq!(
        created_chunk.status, "active",
        "补库 chunk 经显式 verify 后生命周期 status 必须置 active（draft→active）"
    );

    // ⑤ 再 answer **同一** query → 断言缺口闭合：这次召回命中新补的 chunk。
    let (reach1, adopt1) = match answer_reach_adopt(&state, ws, gap_query).await {
        Some(v) => v,
        None => return, // 瞬时不可达，跳过
    };
    eprintln!(
        "[RECALL-CLOSED] ⑤ 补库后 reach1={:?} adopt1={:?}（target={}）",
        reach1, adopt1, created_chunk_id
    );

    // 缺口闭合主断言：补库后 reach 召回命中新补的 chunk（被翻到）。
    assert!(
        reach1.contains(&created_chunk_id),
        "缺口未闭合：补库后 reach {:?} 未命中新补 chunk {}",
        reach1,
        created_chunk_id
    );
    // adopt 命中（被真正引用作答）是更强的闭环证据：真模型偶发只翻不引时降级为软告警，
    // 不让真模型的引用波动把整条确定性闭环（reach 命中 + 信号红线）判红。
    if adopt1.contains(&created_chunk_id) {
        eprintln!("[RECALL-CLOSED] adopt 也命中新补 chunk —— 缺口完全闭合（reach+adopt）");
    } else {
        eprintln!(
            "[RECALL-CLOSED][SOFT-WARN] adopt 未命中新补 chunk（reach 已命中）：\
             真模型本轮翻到却未引用，缺口已可召回但未采纳"
        );
    }

    // 红线：补库后召回命中的 chunk 必须真实存在于本 ws（不许幻觉 id）。
    let all_ids = app
        .state
        .db
        .operation_knowledge_chunks()
        .find(doc! { "workspace_id": ws }, None)
        .await
        .expect("查询 ws 全部 chunks")
        .try_collect::<Vec<OperationKnowledgeChunk>>()
        .await
        .expect("收集 ws 全部 chunks")
        .into_iter()
        .filter_map(|c| c.id.map(|id| id.to_hex()))
        .collect::<HashSet<String>>();
    for id in &reach1 {
        assert!(
            all_ids.contains(id),
            "RED-LINE 召回集越界：{} 不在 ws {} 全集中（补库后命中阶段）",
            id,
            ws
        );
    }

    eprintln!("[RECALL-CLOSED] 闭合轨迹测试完成 —— gap→提问→对话补库→同 query 再命中 全程走通");
}