//! Agent-first 渐进式披露知识库 agent（替代 BM25 / 向量召回路径）。
//!
//! 设计目标：让 LLM 自己驱动 wiki chunk 探索，按 skills 那样的"按需披露"模式：
//! - 工具集：`list_catalog`（按 query 重排后的 chunk 摘要） / `open_document`
//!   （按文档下钻到它的原子摘要） / `open_chunk`（按需展开正文+引用） /
//!   `follow_relations`（沿 related_chunks 跳）。注意：#619 设想的 round 1
//!   **文档级目录注入**（[`DocEntry`] catalogSummary / routingMap 导航卡片）
//!   从未接线——`DocEntry` 零构造点，catalog 只有原子级 [`CatalogEntry`]（不含
//!   documentId），`open_document` 因 agent 无从得知 documentId 而近乎不可用。
//! - 多轮预算：≤ 4 轮 LLM 决策（文档目录+catalog → open_document → open_chunk /
//!   follow → answer）；超出强制 answer 当前已 opened chunks。
//! - 预算复用：所有 LLM 调用走 [`super::generate_agent_json`]，自动累计到当前
//!   run 的 [`super::RunBudget`]；上游 gateway 已用掉大半 budget 时，本 agent
//!   会被自然挤掉早 answer。
//! - 不写 chunk：本 agent 是**只读**面向 agent 的检索面，不触发 patch / verify。
//! - 隔离：本模块**不**引用 `gateway` / `outbox` / `mcp` / `agent_send_outbox`，
//!   保持 agent 子模块对运营网关零耦合，可独立给 `/api/knowledge/ask` 用。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_bson, Bson, DateTime, Document};
use mongodb::options::FindOptions;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::error::AppResult;
use crate::knowledge_wiki::gap_signals::{persist_recall_signal, GapSignalCandidate};
use crate::knowledge_wiki::structural_proposals::{propose_structural_change, StructuralKind};
use crate::models::OperationKnowledgeChunk;
use crate::routes::AppState;

use super::budget::{current_run_budget, current_run_mode};
use super::generate_agent_json;
pub use super::types::{
    KnowledgeAnswerability, KnowledgeNextStep, KnowledgeRequiredAuthority, KnowledgeResolution,
};

mod cache;
pub use cache::{cache_stats, AnswerCacheStats};

/// 单轮探索的硬上限：4 轮 LLM 决策。按 #619 分层召回设想，最坏路径
/// `round1 文档目录 + catalog → open_document 下钻到原子 → open_chunk 展开正文
/// → answer` 正好 4 轮（该设想中的文档目录注入未接线，见模块头注）；现行实际
/// 链路 `list_catalog → open_chunk → follow_relations → answer` 同样 4 轮。
/// 第 5 轮直接强制 answer；与 [`super::RunBudget::max_llm_calls`]
/// 互不替代——budget 用尽更早跳出循环。
pub(crate) const MAX_ROUNDS: i32 = 4;

/// `list_catalog` 一次返回 chunk 摘要的硬上限，控制 prompt size。
const CATALOG_PAGE_SIZE: usize = 30;

/// 一次 `open_chunk` 最多展开几条 chunk，避免 prompt 爆炸。
const OPEN_CHUNK_BATCH: usize = 8;

/// `follow_relations` 单跳/双跳最大展开数量。
/// 取 [`CATALOG_PAGE_SIZE`] / 2：单次跳跃顶多占 catalog 半席，避免 follow_relations
/// 把 catalog 挤爆，让后续 list_catalog 还能补进新 chunk。
const FOLLOW_RELATIONS_LIMIT: usize = 16;

/// `follow_relations` 单次最多把几条 depth-1 关联目标的【完整正文】直接载入
/// `opened`，让 agent 当轮即可 cite，免去再花一轮 `open_chunk`（关联链
/// open A → follow → answer 正好 3 轮，不撞 [`MAX_ROUNDS`]）。取 3：覆盖最相关
/// 的直接关联又不撑爆 prompt；其余关联仍只进 catalog 摘要（需要时再 open_chunk）。
/// 复用 [`follow_relations`] 中已 `find_one` 出的完整文档——零额外 DB / 零额外 LLM 轮次。
const FOLLOW_PREFETCH_BODIES: usize = 3;

/// `summary` 在 catalog 中的截断长度（按 char 数算，CJK 友好）。
const CATALOG_SUMMARY_CHARS: usize = 120;

/// `list_catalog` 在 query 非空时从 DB 拉取的候选上限。先按静态置信度排序取这么多，
/// 再在进程内按 **query 相关度** 重排、截断到 [`CATALOG_PAGE_SIZE`]。
///
/// **为什么需要它（#619 召回硬伤）**：旧实现按 `dynamic_confidence` 取 120 条后直接
/// `truncate(30)`，完全无视 query —— 一条「与查询高度相关但置信度排 31+」的 chunk
/// 会被静态截断悄悄丢掉，知识库 agent 与运营 agent 都召回残缺。把窗口放到 400 并在
/// 进程内按 query 相关度重排后，私域运营知识库的现实规模（单 workspace verified chunk
/// 通常数十到数百条）下「全量重排」，相关 chunk 不再被静态置信度埋没。
///
/// **为什么不上向量库 / `$text`**：MongoDB `$text` 不对 CJK 分词（中文查询近乎失效），
/// 向量库是新依赖 + 部署拓扑变更。这里用语言无关的 bigram + token 覆盖度在进程内重排，
/// 零新依赖、对中英混排都鲁棒。corpus 超过 400 条 verified 的极端规模才会有尾部漏召，
/// 届时再引入 DB 侧检索（明确的后续工作，不在本修复内）。
const CATALOG_CANDIDATE_CAP: usize = 400;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AnswerRequest {
    pub workspace_id: String,
    /// account_id 为 None 时只看 workspace 共享 chunk。
    pub account_id: Option<String>,
    pub query: String,
    /// 初始过滤；为空时返回整个 workspace 的 catalog 头部。
    #[serde(default)]
    pub filter: CatalogFilter,
    /// 客户端可选提示：希望 ≤ 多少轮 answer。clamp 到 `[1, MAX_ROUNDS]`。
    #[serde(default)]
    pub max_rounds: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFilter {
    #[serde(default)]
    pub wiki_types: Vec<String>,
    #[serde(default)]
    pub business_topics: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// 默认 false：catalog / open_chunk 仅暴露 `integrity_status="verified"` chunk，
    /// 与 [`super::knowledge_router::load_operation_knowledge`] 的 verified-only
    /// 加载对齐。设为 true 时上层（如内部审阅工具）可越权拉取 needs_review / draft
    /// chunk，但 router 路径永远走 false。
    #[serde(default)]
    pub include_unverified: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub chunk_id: String,
    pub wiki_type: String,
    pub chunk_type: String,
    pub title: String,
    pub summary: String,
    pub business_topics: Vec<String>,
    pub verified: bool,
    pub has_source_quote: bool,
    pub dynamic_confidence: f64,
    pub related_count: i32,
}

/// 文档级导航卡片（#619 分层召回）——设想给 agent 在 **round 1** 当「目录/索引」读的
/// 上层信号，密度远高于 30 条等价原子摘要。`catalogSummary` / `routingMap` 是抽取
/// 时**专门写给 agent**的导航文本（"这份文档解决什么问题、何时该打开"）。
///
/// **现状：预留结构，全库零构造点**——round 1 注入从未接线，agent 目前只看到
/// 原子级 [`CatalogEntry`]。接线时应在 round 1 组装本结构并随 catalog 下发。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocEntry {
    pub document_id: String,
    pub title: String,
    /// 给 agent 看的目录摘要（抽取时写入），无则回退到 `summary`。
    pub catalog_summary: String,
    /// 自然语言目录项（章节级路标），帮助 agent 判断该不该 open_document 下钻。
    pub routing_map: Vec<String>,
    pub business_topics: Vec<String>,
    /// 该文档下 verified chunk 数量（=open_document 能下钻出多少原子）。
    pub verified_chunk_count: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkFull {
    pub chunk_id: String,
    pub wiki_type: String,
    pub chunk_type: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub source_quote: Option<String>,
    pub source_anchors: Vec<Document>,
    pub related_chunks: Vec<RelatedRefView>,
    pub verified: bool,
    pub business_topics: Vec<String>,
    /// D3(a)：经哪种关系角色被拉入 opened。`None`=agent 直接 open / 正向关系
    /// 跟随；`Some("contradiction")`=经 `contradicts` 关系拉入，prompt 渲染时附
    /// 「仅供辨别、勿作支撑引用」话术（见 `build_prompt`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_role: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedRefView {
    pub chunk_id: String,
    pub kind: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceQuoteCitation {
    pub chunk_id: String,
    pub quote: String,
    /// 在 chunk.source_anchors 中对应的下标；越界视为 None。
    pub source_anchor_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerResult {
    pub answer: String,
    pub resolution: KnowledgeResolution,
    pub cited_chunk_ids: Vec<String>,
    pub source_quotes: Vec<SourceQuoteCitation>,
    pub tool_trace: Vec<Document>,
    pub rounds_used: i32,
    /// true 表示 3 轮内未给出 answer，由兜底逻辑强制返回。
    pub truncated: bool,
    /// true 表示客户端断开 / 显式取消，agent 提前退出循环。
    /// 与 `truncated` 不互斥（cancel 触发的兜底也算 truncated）。
    pub cancelled: bool,
}

/// SSE 流式接口的事件载荷。每个 `Step` 与 [`AnswerResult::tool_trace`] 中的一条
/// Document 一一对应；`Final` 在 inner 跑完后发一次，携带最终聚合结果。
///
/// **为什么不直接传 `bson::Document`**：BSON Document 序列化会输出 ExtJson
/// （`{"$numberInt":"3"}`），前端需要再二次桥接。这里直接走 `serde_json::Value`，
/// 在 emit 点一次性 `into_relaxed_extjson()`，SSE 主循环只做 `serde_json::to_string`。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TraceEvent {
    /// 一步工具调用：与 `tool_trace` 中的 Document 内容相同（已转 relaxed extjson）。
    Step { payload: serde_json::Value },
    /// 终态前的渐进 token：携带本次 delta（上游真流式解码出的 `answer` 正文片段）。
    /// answer 轮的 LLM 调用走 [`super::generate_agent_json_streaming`]，原始 JSON
    /// 文本片段经 [`AnswerStreamer`] 增量抽取顶层 `answer` 字段后，把解码正文逐段
    /// 下发；前端按 token append 即可获得真实流式视觉。工具轮无 answer 字段不产生
    /// token。
    Token { delta: String },
    /// 整条知识问答未能产出 Final 的失败终态。只携带稳定错误码和面向运营的
    /// 通用文案；详细上游错误保留在服务端日志，避免经 SSE 暴露内部信息。
    Failed { code: String, message: String },
    /// 终态：携带最终 `AnswerResult`（不再有 step）。
    Final { answer: AnswerResult },
}

/// LLM 在每一轮回包必须遵守的 action 协议。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentAction {
    ListCatalog {
        #[serde(default)]
        filter: CatalogFilter,
    },
    OpenChunk {
        #[serde(default)]
        ids: Vec<String>,
    },
    /// #619 分层召回下钻：按 documentId 把该文档下所有 verified chunk 的【摘要】
    /// 合并进 catalog（不直接载正文，避免 prompt 爆炸；agent 再 open_chunk 展开
    /// 选中的原子）。这是「先文档目录、再下钻原子」的中间一跳。
    OpenDocument {
        #[serde(default, alias = "documentId")]
        document_id: String,
    },
    FollowRelations {
        #[serde(default, alias = "chunkId")]
        chunk_id: String,
        #[serde(default)]
        depth: Option<u32>,
    },
    Answer {
        #[serde(default, alias = "citedChunkIds")]
        cited_chunk_ids: Vec<String>,
        #[serde(default, alias = "sourceQuotes")]
        source_quotes: Vec<RawSourceQuote>,
        #[serde(default)]
        answer: String,
        #[serde(default)]
        resolution: KnowledgeResolution,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSourceQuote {
    #[serde(default, alias = "chunkId")]
    pub chunk_id: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default, alias = "sourceAnchorIndex")]
    pub source_anchor_index: Option<i32>,
}

const SYSTEM_PROMPT: &str = "你是运营知识库的 wiki 研究员。\n\
系统会直接提供按本次查询排序的原子知识 catalog。你必须按渐进式披露模式工作：先判断 catalog 中哪些原子相关，相关时优先 open_chunk 展开完整正文；只有已经掌握 documentId、确实需要查看同一文档下其它原子摘要时才使用 open_document；最后给出带引用的 answer。\n\
你不能凭空回答；任何回答都必须来自被你 open 过的 chunk。\n\
你只输出严格 JSON。每轮只能输出 5 个 action 之一：list_catalog / open_document / open_chunk / follow_relations / answer。\n\
最多 4 轮工具调用。最后一轮必须 answer。\n\
你的 answer 是给系统内部回复 Agent 的**知识研判**，不是发给客户的话术，也不是可执行的对客行动脚本。本系统定位是 AI 全程自治：客户始终只与 AI 对话。若被检索的知识内容描述了机构内部的流程分工（例如某类事项需按正式政策核对、由内部相应岗位确认），你可以如实转述该知识所界定的**事实边界**，但绝不把内部角色或流程写成对客户的话术。凡超出当前已 open 知识能确定的事项，必须在 resolution 中按语义标明可回答性、缺失信息、所需权威和下一步；禁止按客户文本中的单词、短语或词表判断。";

/// Stable user-message prefix for every round of one Knowledge loop.
///
/// Keep all invariant schema and policy before query/catalog/round state. Providers that cache a
/// common prompt prefix can then reuse this block across requests and also reuse the much longer
/// `protocol + query + catalog` prefix between round 1 and round 2 of the same research loop.
const ACTION_PROTOCOL: &str = r#"知识库研究协议（稳定规则）：
下一步只输出以下 5 种 action 之一的严格 JSON：
{"action":"list_catalog","filter":{"wikiTypes":["..."],"businessTopics":["..."]}}
{"action":"open_document","documentId":"..."}
{"action":"open_chunk","ids":["chunk_id_1","chunk_id_2"]}
{"action":"follow_relations","chunkId":"...","depth":1}
{"action":"answer","citedChunkIds":["..."],"sourceQuotes":[{"chunkId":"...","quote":"...","sourceAnchorIndex":0}],"answer":"...","resolution":{"answerability":"supported | partially_supported | unsupported | not_required","requiredAuthority":"none | customer | authorized_operator | licensed_professional | external_system","recommendedNextStep":"respond | clarify_customer | ask_principal | defer","missingInformation":["..."],"authorityQuestion":"...","unresolvedProposition":"..."}}

- 召回漏斗：catalog 已按与本次 query 的相关度排过序，越靠前越相关。看到与 query 相关的候选，默认动作就是 open_chunk 展开它的正文再核对作答。
- catalog 只给摘要且会被截断，回答任何细节问题（具体数字、比例、条件、期限等）前必须先 open_chunk 读正文，绝不能仅凭摘要臆测或直接说没有。
- open_document 仅在你已掌握某条目的 documentId、想一次性查看同文档下其它原子摘要时才用；没有 documentId 时无需 open_document，直接 open_chunk 即可。
- 只有当 catalog 里确实没有任何与 query 相关的候选时，才回答“知识库无相关内容”并让 cited 留空；在尚未 open 任何相关候选正文之前，禁止下此结论。
- 诚实弃答原则：判定一条候选能否支撑作答，标准是它是否直接覆盖本 query 所问的对象与口径，而非主题词面相近。若 open 后只有主题相邻、口径不同、或仅部分相关，必须说明知识库仅有近似或部分知识，无法据此确切作答，并让 cited 留空或仅 cite 真正能支撑的部分。绝不把近似知识当确切事实硬答，绝不编造或外推正文未明确给出的数字、比例、条件、期限或范围。
- 弃答须可推进：凡诚实弃答时，answer 末尾必须补一句具体、可执行的追问，点明缺少哪类信息、需要补充什么口径或细节，或提示运营补全该知识。禁止只说“不知道”或“暂无”。
- 每个 answer 都必须输出完整 resolution。resolution 是内部语义结论，不是客户话术；不得把内部角色、控制字段或推理过程写进 answer。
- `unresolvedProposition` 是一个简洁、完整、可判断真假的现实命题，专门描述当前证据仍未关闭且会实质影响本轮答复的那一部分。它必须由完整 query、已 open 证据和 resolution 共同语义判断生成，明确当前对象与所问范围，不能从关键词、词表、金额、行业类别或固定句式推导；任何业务主题都使用同一判断方法。若 answerability=partially_supported/unsupported 且仍有一个待核准事实，必须填写；若 answerability=supported 或 not_required，通常留空。它是给 Reply/Reviewer/ClaimGate 的边界，不是客户可见文案。
- answerability=supported 仅限已 open 且实际引用的 verified 证据直接覆盖本 query 全部所问；只覆盖一部分用 partially_supported；没有直接证据用 unsupported。无需业务知识即可处理的社交或关系性消息用 not_required + none + respond，且 cited 留空。
- requiredAuthority / recommendedNextStep 必须按缺口实质选择，禁止按客户文本中的词或短语匹配：缺客户自身信息用 customer + clarify_customer；缺机构当前安排、当期政策、实时可用性或其它须由有权运营人员确认的内部事实，用 authorized_operator + ask_principal；须执业判断用 licensed_professional + defer；须查询实时外部记录用 external_system + defer。已有充分证据或无需知识则用 none + respond。
- 健康、医疗或其它受专业资质约束的语境中，必须区分“一般教育信息”和“对当前个体状态的判断”。知识条目说明某种现象通常如何，不等于能据此认定这位客户当前情况属于该现象、风险低或无需处理。若仍缺个体的程度、变化、伴随情况等实质信息，先用 customer + clarify_customer 只补最关键的一项；若结论本身需要专业评估，则用 licensed_professional + defer。按完整语义判断，不建立症状词表。
- recommendedNextStep=ask_principal 时 authorityQuestion 必须是一句具体、可直接交给有权人员回答的问题；其它 nextStep 可留空。missingInformation 只列当前证据真正缺少的事实，不写泛化套话。`unresolvedProposition` 与 authorityQuestion 指向同一个尚未关闭的命题，但前者要写成完整可判定的现实命题，不能只写“当前状态”。
- citedChunkIds 必须是“已 open 的 chunks”中的 chunkId 子集，不能凭空创造。
- follow_relations 会把最相关的关联条目正文直接载入“已 open 的 chunks”，可当轮直接 cite，无需再 open_chunk。
- 每个 cited 必须配 sourceQuote；如某 chunk 没有可引用原文，可省略 sourceQuote 但仍可 cite。
- 候选 catalog 中所有 chunk 都已 integrity_status=verified；遇到 verified=false 是异常，不要 cite。
- 关系角色（relationRole）：已 open 的 chunk 若带 relationRole="contradiction"，表示它是经 contradicts 关系拉入的、与来源 chunk 相矛盾的说法，仅供对比辨别用，绝不可作为支撑材料 cite。遇到矛盾，应据其余正向 chunk 作答，必要时说明该点存在不同说法、需要核实，但不要把矛盾说法当确切事实引用。
- 不要复述 catalog 中的整段 summary；用自然语言总结答复。"#;

/// 知识库 agent 主循环。
///
/// 流程：
/// 1. 先取一份 catalog（30 条摘要）；
/// 2. 进入 ≤ 3 轮 LLM 循环；每轮把"已 open 的 chunk + 当前 catalog + 已收集的 follow"
///    一起喂给 LLM，让它输出下一步 action；
/// 3. 收到 `answer` action 立即返回；
/// 4. 超过 3 轮或 budget 用尽 → 用当前 opened 强制 answer，标 `truncated=true`。
pub async fn answer(state: &AppState, req: AnswerRequest) -> AppResult<AnswerResult> {
    // workspace_id 在 answer_inner 消费 req 前先捕获，供失败签名落库时尊重隔离。
    let workspace_id = req.workspace_id.clone();
    // 原始 query 在 answer_inner 消费 req 前先捕获：诚实弃答时把它确定性写进 gap
    // 信号，供运营用对话形式补全知识库（零 LLM 依赖，恒在）。
    let query = req.query.clone();
    let result = answer_inner(state, req, None, None).await?;
    // 在线召回-trace 闭环（方法论点 5）：把本次召回的失败签名 fire-and-forget 入
    // 离线整改队列。镜像 record_chunk_hit 的非阻塞模式——绝不 .await 在召回热路径
    // 内，落库失败只丢日志、不影响已返回给调用方的 result。
    if let Some(mut candidate) = classify_recall_outcome(&result) {
        let db = state.db.clone();
        let ws = workspace_id.clone();
        // 点 5→点 6 接驳：recall_low_yield = 多 open 少 cite，是典型的「粒度过粗」
        // 信号 → 顺带 emit 一条 split StructuralProposal intent（只入队、绝不应用、
        // 绝不物理删除），把召回失败导向结构化写质检队列。recall_miss 是「查无内容」
        // 偽阴性（缺知识/放置问题），不映射到拆分意图，仅留 gap_signal。
        let split_targets = if candidate.kind == "recall_low_yield" {
            Some(candidate.affected_chunk_ids.clone())
        } else {
            None
        };
        // recall_miss = 诚实弃答/查无内容：把原始 query 确定性写入待补全线索（恒在，
        // 不依赖 LLM）；随后在 spawn 内 fire-and-forget 追加一句 LLM 生成的追问，
        // 两者结合给人类更完整的对话补全入口。low_yield 是结构拆分信号，不带 query。
        let followup_query = if candidate.kind == "recall_miss" {
            candidate.search_queries = vec![query.clone()];
            Some(query.clone())
        } else {
            None
        };
        let state_clone = state.clone();
        tokio::spawn(async move {
            // 先确定性落库（恒在、零 LLM 依赖）：recall_miss 的 search_queries 已在
            // spawn 前同步置为原始 query，low_yield 携 affected_chunk_ids。绝不把这一步
            // 排在任何 LLM 调用之后——否则 followup 的真模型往返（常 >数秒）会阻塞信号
            // 写入，运营/在线闭环要等数秒才见到 gap，等同丢失「恒在」语义。
            if let Err(e) = persist_recall_signal(&db, &ws, candidate.clone()).await {
                tracing::warn!(error = %e, "persist_recall_signal failed (non-fatal)");
            }
            // recall_miss 增强：落库后再 fire-and-forget 生成一句 LLM 追问，成功则以二次
            // merge-update 并入同一信号的 search_queries（dedup_key 同 → 命中并集分支）。
            // 失败/超时只丢日志，首次确定性落库已不可逆地完成。
            if let Some(q) = followup_query {
                if let Some(followup) = generate_gap_followup_question(&state_clone, &ws, &q).await
                {
                    if !followup.is_empty() && followup != q {
                        let mut enrich = candidate.clone();
                        enrich.search_queries = vec![followup];
                        if let Err(e) = persist_recall_signal(&db, &ws, enrich).await {
                            tracing::warn!(error = %e, "persist_recall_signal (followup enrich) failed (non-fatal)");
                        }
                    }
                }
            }
            if let Some(targets) = split_targets {
                if !targets.is_empty() {
                    if let Err(e) = propose_structural_change(
                        &db,
                        &ws,
                        StructuralKind::Split,
                        targets,
                        "recall_low_yield：多 open 少 cite，疑似原子粒度过粗，建议拆细视图",
                        "recall_trace",
                        None,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "propose_structural_change failed (non-fatal)");
                    }
                }
            }
        });
    }
    Ok(result)
}

/// Run the same knowledge-agent reasoning in read-only mode. This deliberately
/// bypasses the production wrapper that emits recall gap signals and
/// structural proposals. LLM cost logs remain enabled and inherit the current
/// `run_mode=shadow` task-local marker.
pub(crate) async fn answer_read_only(
    state: &AppState,
    req: AnswerRequest,
) -> AppResult<AnswerResult> {
    answer_inner(state, req, None, None).await
}

/// 为「知识库查无可引用知识」生成一句面向人类运营的追问，供其用对话形式补全知识库。
/// fire-and-forget：任何错误/超时返回 None，调用方回退到仅用原始 query。system/user
/// 以字面量传入 `generate_agent_json`，不触 prompts.rs；prompt_key 仅作日志/缓存标签。
async fn generate_gap_followup_question(
    state: &AppState,
    workspace_id: &str,
    query: &str,
) -> Option<String> {
    let system = "你是知识库补全助手。知识库对用户问题查无可引用知识。请只输出 JSON \
                  {\"question\":\"<一句精炼、面向人类运营的追问，引导其补充缺失知识>\"}，\
                  不要编造任何事实，只生成引导补全的问题。";
    let user = format!("用户原始问题：{query}\n请生成一句追问。");
    let v = super::generate_agent_json(
        state,
        workspace_id,
        None,
        None,
        None,
        "knowledge.gap.followup",
        system,
        &user,
    )
    .await
    .ok()?;
    v.get("question")
        .and_then(|q| q.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 流式版本：每个 `tool_trace.push` 同步通过 `tx` 发出 [`TraceEvent::Step`]，跑完
/// 再发一次 [`TraceEvent::Final`] 携带最终 [`AnswerResult`]。`tx` 在前端断开时
/// `send` 静默失败，不影响主线 LLM 调用与 mongo I/O 完成度（写库、写日志照常）。
///
/// `cancel`：取消标志位。客户端断开时 SSE handler 把它 `store(true)`，agent 在
/// 下次轮询前检测到 → push `cancelled` step + 走兜底路径返回（`cancelled=true`）。
/// 取消是软取消：正在跑的 LLM call 不强 abort（避免连接池脏），但下一轮不会启动。
///
/// 与 [`answer`] 共用 [`answer_inner`] 主体，行为完全等价；本函数只是把 `tx` /
/// `cancel` 包成 `Some` 注入。
pub async fn answer_streaming(
    state: &AppState,
    req: AnswerRequest,
    tx: UnboundedSender<TraceEvent>,
    cancel: Option<Arc<AtomicBool>>,
) -> AppResult<AnswerResult> {
    // answer_inner 在 answer 轮内部已通过 [`AnswerStreamer`] 把上游真 token 解码后
    // 逐段发出 [`TraceEvent::Token`]；这里只需在跑完后补一帧 [`TraceEvent::Final`]，
    // 让前端拿到完整 `AnswerResult`（含 cited / quotes / trace）。cache-hit /
    // truncated / cancelled 等不产生 token 的路径也靠这一帧兜底渲染最终答案。
    let result = answer_inner(state, req, Some(&tx), cancel.as_ref()).await?;
    let _ = tx.send(TraceEvent::Final {
        answer: result.clone(),
    });
    Ok(result)
}

/// 增量抽取流式原始 JSON 中顶层 `answer` 字段的字符串值，把新增正文片段解码下发。
///
/// 上游 token 流过来的是模型**原始 JSON 文本**（如 `{"action":"answer","answer":"你好`
/// → `世界"}`），不是裸正文。本结构体逐 char 喂入，维护一个轻量解析状态机，仅在
/// 指针落在 `answer` 字符串值内部时把解码后的字符累积返回。设计取舍：
/// - 定位靠 [`locate_answer_value_start`] 的**朴素子串匹配**（找 `"answer"` 键 +
///   冒号 + 起始引号，取首个命中），**不做大括号层级计数**——若 `answer` 值之前
///   出现嵌套对象里的同名键，会提前把嵌套值当正文下发；LLM 正常输出的扁平
///   `{"action":"answer","answer":"..."}` 形态无此问题。
/// - 处理 JSON 字符串转义（`\"` / `\\` / `\n` / `\uXXXX` 等），保证下发的是人类
///   可读正文而非转义序列。
/// - 工具轮（无 answer 字段）整段喂完后累积恒空，自然不产生 token。
#[derive(Default)]
struct AnswerStreamer {
    /// 已扫描但尚未匹配到结构的原始字符缓冲（处理跨 chunk 的键名 / 转义切割）。
    buf: String,
    /// 是否已定位到顶层 `answer` 键、且正处于其字符串值内部。
    in_answer_value: bool,
    /// answer 值是否已完整结束（遇到未转义的闭合引号）——之后忽略一切输入。
    done: bool,
}

impl AnswerStreamer {
    /// 喂入一段新的原始文本片段，返回本次新解码出的 `answer` 正文（可能为空）。
    fn push(&mut self, frag: &str) -> String {
        if self.done {
            return String::new();
        }
        self.buf.push_str(frag);

        if !self.in_answer_value {
            // 还没进入 answer 值：在缓冲里找 `"answer"` 键后的起始引号。
            // 用最朴素的子串定位即可——key 名固定，且只取顶层第一次出现。
            if let Some(found) = locate_answer_value_start(&self.buf) {
                self.in_answer_value = true;
                // 丢弃起始引号及之前的所有内容，只留值正文待解析。
                self.buf = self.buf[found..].to_string();
            } else {
                // 防止 buf 无界增长：保留尾部足够覆盖 `"answer"` + 空白 + `:` + `"`
                // 的窗口（远小于 64），其余前缀确定不含起始锚点可丢弃。
                const KEEP_TAIL: usize = 64;
                if self.buf.len() > KEEP_TAIL {
                    let cut = self.buf.len() - KEEP_TAIL;
                    // 对齐到 char 边界，避免切碎多字节 UTF-8。
                    let cut = (0..=cut)
                        .rev()
                        .find(|&i| self.buf.is_char_boundary(i))
                        .unwrap_or(0);
                    self.buf = self.buf[cut..].to_string();
                }
                return String::new();
            }
        }

        // 已在 answer 值内部：逐 char 解码，直到遇到未转义闭合引号。
        let (decoded, consumed, finished) = decode_json_string_body(&self.buf);
        self.buf = self.buf[consumed..].to_string();
        if finished {
            self.done = true;
        }
        decoded
    }
}

/// 在原始 JSON 文本里定位顶层 `answer` 键对应字符串值起始引号的**下一个**字节位置。
/// 返回 `Some(idx)` 时，`text[idx..]` 即 answer 值正文（不含起始引号）。
///
/// 朴素实现：找到子串 `"answer"`，跳过其后空白与冒号，再要求下一个非空白字符是
/// `"`。够覆盖 LLM 正常输出的 `{"action":"answer","answer":"..."}` 形态。
fn locate_answer_value_start(text: &str) -> Option<usize> {
    let key = "\"answer\"";
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(key) {
        let after_key = search_from + rel + key.len();
        let rest = &text[after_key..];
        let mut bytes = rest.char_indices().peekable();
        // 跳过空白。
        let mut cursor = after_key;
        let mut saw_colon = false;
        let mut started = false;
        for (off, ch) in &mut bytes {
            if ch.is_whitespace() {
                continue;
            }
            if ch == ':' && !saw_colon {
                saw_colon = true;
                continue;
            }
            if ch == '"' && saw_colon {
                cursor = after_key + off + ch.len_utf8();
                started = true;
            }
            break;
        }
        if started {
            return Some(cursor);
        }
        // 不是值起点（可能是嵌套键名片段），继续向后找。
        search_from = after_key;
    }
    None
}

/// 从 JSON 字符串值正文起点开始解码，直到遇到未转义闭合引号或输入耗尽。
///
/// 返回 `(decoded, consumed, finished)`：
/// - `decoded`：本次能确定解码出的正文字符（不含尚未闭合的转义序列）。
/// - `consumed`：已消费的字节数（调用方据此裁剪缓冲；未消费的尾部留待下次拼接）。
/// - `finished`：是否遇到了未转义的闭合引号（answer 值结束）。
///
/// 对不完整结尾（半个转义 `\` 或半个 `\uXXXX`）保守处理：不消费、留待下次。
fn decode_json_string_body(s: &str) -> (String, usize, bool) {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                // 未转义闭合引号 → answer 值结束。消费含该引号。
                return (out, i + 1, true);
            }
            b'\\' => {
                // 转义序列：至少要再有一个字符才能判定。
                if i + 1 >= bytes.len() {
                    // 半个转义，留待下次。
                    return (out, i, false);
                }
                let esc = bytes[i + 1];
                match esc {
                    b'"' => {
                        out.push('"');
                        i += 2;
                    }
                    b'\\' => {
                        out.push('\\');
                        i += 2;
                    }
                    b'/' => {
                        out.push('/');
                        i += 2;
                    }
                    b'n' => {
                        out.push('\n');
                        i += 2;
                    }
                    b't' => {
                        out.push('\t');
                        i += 2;
                    }
                    b'r' => {
                        out.push('\r');
                        i += 2;
                    }
                    b'b' => {
                        out.push('\u{0008}');
                        i += 2;
                    }
                    b'f' => {
                        out.push('\u{000C}');
                        i += 2;
                    }
                    b'u' => {
                        // \uXXXX：需要 4 个十六进制位。
                        if i + 6 > bytes.len() {
                            return (out, i, false); // 不完整，留待下次
                        }
                        let hex = &s[i + 2..i + 6];
                        match u32::from_str_radix(hex, 16) {
                            Ok(cp) => {
                                // 不处理代理对拼接（罕见于中文正文）；落单代理对
                                // 用替换字符兜底，避免 panic。
                                if let Some(ch) = char::from_u32(cp) {
                                    out.push(ch);
                                } else {
                                    out.push('\u{FFFD}');
                                }
                                i += 6;
                            }
                            Err(_) => {
                                // 非法转义，原样保留反斜杠跳过。
                                out.push('\\');
                                i += 1;
                            }
                        }
                    }
                    _ => {
                        // 未知转义，原样保留。
                        out.push('\\');
                        i += 1;
                    }
                }
            }
            _ => {
                // 普通字符：找到该 UTF-8 char 的完整字节再 push，处理多字节边界。
                let ch_len = utf8_char_len(b);
                if i + ch_len > bytes.len() {
                    // 半个多字节 char，留待下次。
                    return (out, i, false);
                }
                let ch = s[i..i + ch_len].chars().next().unwrap();
                out.push(ch);
                i += ch_len;
            }
        }
    }
    (out, i, false)
}

/// 由 UTF-8 首字节推断该字符的字节长度（1–4）。
fn utf8_char_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        1 // 非法首字节，按 1 推进避免卡死
    }
}

/// 把一条 trace doc 同时写进 `tool_trace` 与可选 `tx`。`tx` 走 relaxed extjson
/// 桥接，前端拿到的就是纯 JSON（`3` 而不是 `{"$numberInt":"3"}`）。
fn push_trace(
    tool_trace: &mut Vec<Document>,
    tx: Option<&UnboundedSender<TraceEvent>>,
    entry: Document,
) {
    if let Some(tx) = tx {
        let payload = Bson::Document(entry.clone()).into_relaxed_extjson();
        let _ = tx.send(TraceEvent::Step { payload });
    }
    tool_trace.push(entry);
}

async fn answer_inner(
    state: &AppState,
    req: AnswerRequest,
    tx: Option<&UnboundedSender<TraceEvent>>,
    cancel: Option<&Arc<AtomicBool>>,
) -> AppResult<AnswerResult> {
    let max_rounds = req.max_rounds.unwrap_or(MAX_ROUNDS).clamp(1, MAX_ROUNDS);

    let mut tool_trace: Vec<Document> = Vec::new();
    let mut opened: Vec<ChunkFull> = Vec::new();
    let mut opened_seen: HashSet<String> = HashSet::new();
    let mut catalog = list_catalog(
        state,
        &req.workspace_id,
        req.account_id.as_deref(),
        &req.filter,
        Some(&req.query),
    )
    .await?;
    push_trace(
        &mut tool_trace,
        tx,
        doc! {
            "tool": "list_catalog",
            "filter": filter_to_doc(&req.filter),
            "returned": catalog.len() as i32,
        },
    );

    if catalog.is_empty() {
        return Ok(AnswerResult {
            answer: "知识库无相关内容。".to_string(),
            resolution: KnowledgeResolution::default(),
            cited_chunk_ids: Vec::new(),
            source_quotes: Vec::new(),
            tool_trace,
            rounds_used: 0,
            truncated: false,
            cancelled: false,
        });
    }

    // E4: cache identity covers every visible active/verified chunk and the
    // exact provider registry generation. The full-corpus signature prevents
    // edits outside the displayed top-30 (including relation targets) from
    // leaving a stale answer cache entry valid.
    // 取消路径不查 cache（用户显式想重跑）。
    let cache_key = if current_run_mode() != "shadow" && !is_cancelled(cancel) {
        let corpus_sig =
            visible_corpus_signature(state, &req.workspace_id, req.account_id.as_deref()).await?;
        let provider = current_provider_cache_identity(state, &req.workspace_id).await?;
        let key = cache::CacheKey {
            workspace_id: req.workspace_id.clone(),
            account_id: req.account_id.clone(),
            provider_id: provider.provider_id,
            provider_model: provider.model,
            provider_generation: provider.generation,
            prompt_pack_version: state
                .prompt_pack_version
                .load(std::sync::atomic::Ordering::SeqCst),
            filter_norm: normalized_filter_key(&req.filter),
            query_norm: cache::normalize_query(&req.query),
            corpus_sig,
            max_rounds,
        };
        if let Some(cached) = cache::get(&key) {
            push_trace(
                &mut tool_trace,
                tx,
                doc! {
                    "tool": "cache_hit",
                    "rounds": cached.rounds_used,
                    "citedCount": cached.cited_chunk_ids.len() as i32,
                },
            );
            let mut out = cached;
            out.tool_trace = tool_trace;
            return Ok(out);
        }
        push_trace(
            &mut tool_trace,
            tx,
            doc! {
                "tool": "cache_miss",
                "maxRounds": max_rounds,
            },
        );
        Some(key)
    } else {
        None
    };

    // last_completed_round：跟踪实际跑完了多少轮（含 budget_exceeded 提前 break /
    // invalid_action continue 的情况）。供兜底 / budget 提前退出时上报真实
    // rounds_used，避免前端误以为"用了 max_rounds 才放弃"。
    let mut last_completed_round: i32 = 0;
    let mut cancelled = false;
    let scoped_run_id = current_run_budget().map(|budget| budget.run_id.clone());
    for round in 1..=max_rounds {
        if is_cancelled(cancel) {
            push_trace(
                &mut tool_trace,
                tx,
                doc! {
                    "tool": "cancelled",
                    "round": round,
                    "phase": "loop_top",
                },
            );
            cancelled = true;
            break;
        }
        if let Some(budget) = current_run_budget() {
            if budget.should_stop_optional_llm_calls() {
                let stop_reason = if budget.is_exceeded() {
                    "budget_exceeded"
                } else {
                    budget.mark_degraded("knowledge_agent_stopped_usage_unknown");
                    "usage_unknown"
                };
                push_trace(
                    &mut tool_trace,
                    tx,
                    doc! {
                        "tool": stop_reason,
                        "round": round,
                    },
                );
                break;
            }
        }
        last_completed_round = round;

        let user_prompt = build_prompt(&req.query, &opened, &catalog, round, max_rounds);
        // 流式分支（tx=Some）：每轮都走真上游 SSE，把模型**原始 JSON 文本片段**喂给
        // 一个增量 `answer` 字段抽取器（[`AnswerStreamer`]）；只有正文落在顶层
        // `answer` 字段里的字符才被解码成 [`TraceEvent::Token`] 下发，工具轮（没有
        // answer 字段）自然不产生 token。非流式分支（tx=None，例如 `answer()` /
        // 单测）保持原 [`generate_agent_json`] 调用，零额外开销。
        let value = if let Some(ev_tx) = tx {
            let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let ev_tx_cloned = ev_tx.clone();
            let forwarder = tokio::spawn(async move {
                let mut streamer = AnswerStreamer::default();
                while let Some(frag) = raw_rx.recv().await {
                    let delta = streamer.push(&frag);
                    if !delta.is_empty() {
                        let _ = ev_tx_cloned.send(TraceEvent::Token { delta });
                    }
                }
            });
            let value = super::generate_agent_json_streaming(
                state,
                &req.workspace_id,
                req.account_id.as_deref(),
                None,
                scoped_run_id.as_deref(),
                "knowledge.agent",
                SYSTEM_PROMPT,
                &user_prompt,
                raw_tx,
            )
            .await?;
            // raw_tx 已 move 进 streaming 调用；调用返回即被 drop → forwarder 收到
            // 通道关闭后退出，await 确保末尾 token 已发完再进入 action 解析。
            let _ = forwarder.await;
            value
        } else {
            generate_agent_json(
                state,
                &req.workspace_id,
                req.account_id.as_deref(),
                None,
                scoped_run_id.as_deref(),
                "knowledge.agent",
                SYSTEM_PROMPT,
                &user_prompt,
            )
            .await?
        };

        // LLM 跑完后再查一次 cancel：客户端在这次 LLM call 期间断开的话，下一轮
        // 不该再启动；当前轮的 mongo 副作用（写日志/usage）已在 generate_agent_json
        // 内完成，不需要回滚。
        if is_cancelled(cancel) {
            push_trace(
                &mut tool_trace,
                tx,
                doc! {
                    "tool": "cancelled",
                    "round": round,
                    "phase": "post_llm",
                },
            );
            cancelled = true;
            break;
        }

        let action = match serde_json::from_value::<AgentAction>(value.clone()) {
            Ok(action) => action,
            Err(err) => {
                push_trace(
                    &mut tool_trace,
                    tx,
                    doc! {
                        "tool": "error",
                        "round": round,
                        "reason": format!("invalid_action:{err}"),
                        "raw": value.to_string(),
                    },
                );
                continue;
            }
        };

        match action {
            AgentAction::ListCatalog { filter } => {
                let effective_filter = intersect_catalog_filters(&req.filter, &filter);
                catalog = match effective_filter.as_ref() {
                    Some(filter) => {
                        list_catalog(
                            state,
                            &req.workspace_id,
                            req.account_id.as_deref(),
                            filter,
                            Some(&req.query),
                        )
                        .await?
                    }
                    None => Vec::new(),
                };
                push_trace(
                    &mut tool_trace,
                    tx,
                    doc! {
                        "tool": "list_catalog",
                        "round": round,
                        "filter": effective_filter
                            .as_ref()
                            .map(filter_to_doc)
                            .unwrap_or_else(|| doc! { "impossible": true }),
                        "returned": catalog.len() as i32,
                    },
                );
            }
            AgentAction::OpenDocument { document_id } => {
                let entries = open_document(
                    state,
                    &req.workspace_id,
                    req.account_id.as_deref(),
                    &document_id,
                    Some(&req.query),
                    &req.filter,
                )
                .await?;
                let appended = entries.len() as i32;
                merge_catalog(&mut catalog, entries);
                push_trace(
                    &mut tool_trace,
                    tx,
                    doc! {
                        "tool": "open_document",
                        "round": round,
                        "documentId": document_id,
                        "appended": appended,
                    },
                );
            }
            AgentAction::OpenChunk { ids } => {
                let mut opened_now: Vec<String> = Vec::new();
                let mut not_found: Vec<String> = Vec::new();
                for id in ids.into_iter().take(OPEN_CHUNK_BATCH) {
                    if opened_seen.contains(&id) {
                        continue;
                    }
                    match open_chunk(state, &req.workspace_id, req.account_id.as_deref(), &id)
                        .await?
                    {
                        Some(full) => {
                            // D3(b)：open_chunk 可能把已被取代的旧版 redirect 到现行版本，
                            // 故 opened_seen / opened_now 记 **full.chunk_id**（现行版 id）而非
                            // 请求的旧 id——agent prompt 里看到、并据以 cite 的是现行版 id，
                            // cite⊆opened 不变量靠这条对齐（filter_answer_against_opened 用
                            // opened_seen 过滤）。redirect 命中已 open 过的现行版时跳过重复 push。
                            if opened_seen.insert(full.chunk_id.clone()) {
                                opened_now.push(full.chunk_id.clone());
                                opened.push(full);
                            }
                        }
                        None => {
                            not_found.push(id);
                        }
                    }
                }
                let mut entry = doc! {
                    "tool": "open_chunk",
                    "round": round,
                    "opened": opened_now.clone(),
                };
                if !not_found.is_empty() {
                    entry.insert("notFound", not_found);
                }
                push_trace(&mut tool_trace, tx, entry);
            }
            AgentAction::FollowRelations { chunk_id, depth } => {
                let depth = depth.unwrap_or(1).clamp(1, 2);
                let (entries, prefetched) = follow_relations(
                    state,
                    &req.workspace_id,
                    req.account_id.as_deref(),
                    &chunk_id,
                    depth,
                    &opened_seen,
                )
                .await?;
                let appended = entries.len() as i32;
                merge_catalog(&mut catalog, entries);
                // 关联目标的完整正文直接载入 opened（镜像 OpenChunk 分支），
                // 让 agent 当轮即可 cite，无需再花一轮 open_chunk。
                let mut opened_bodies: Vec<String> = Vec::new();
                for full in prefetched {
                    if opened_seen.insert(full.chunk_id.clone()) {
                        opened_bodies.push(full.chunk_id.clone());
                        opened.push(full);
                    }
                }
                let mut entry = doc! {
                    "tool": "follow_relations",
                    "round": round,
                    "chunkId": chunk_id,
                    "depth": depth as i32,
                    "appended": appended,
                };
                if !opened_bodies.is_empty() {
                    entry.insert("openedBodies", opened_bodies);
                }
                push_trace(&mut tool_trace, tx, entry);
            }
            AgentAction::Answer {
                cited_chunk_ids,
                source_quotes,
                answer,
                resolution,
            } => {
                // 服务端不信任「声称回答却无正文」的终态：LLM 偶尔 emit 一个结构合法
                // 但 answer 为空白的 Answer action 提前收尾，会让调用方拿到空答案。
                // 空白正文不当终态——push 纠正 trace 并 continue：还有剩余轮次则让
                // agent 重新作答，已是末轮则循环自然结束落到下方兜底产出非空摘要。
                if answer.trim().is_empty() {
                    push_trace(
                        &mut tool_trace,
                        tx,
                        doc! {
                            "tool": "error",
                            "round": round,
                            "reason": "empty_answer:Answer action 正文为空白，已忽略并要求继续作答",
                        },
                    );
                    continue;
                }
                // B5：留下「模型尝试过引用」这个事实，供 `classify_recall_outcome`
                // 区分 `citation_format_rejected`（试了但 anchor/quote 校验没过）与
                // `knowledge_missing`（压根没试）。filter 会 move 掉这两个 Vec，
                // 故在调用前先取长度。
                let attempted_cited = cited_chunk_ids.len() as i32;
                let attempted_quotes = source_quotes.len() as i32;
                let (cited, quotes) =
                    filter_answer_against_opened_chunks(&opened, cited_chunk_ids, source_quotes);
                push_trace(
                    &mut tool_trace,
                    tx,
                    doc! {
                        "tool": "answer",
                        "round": round,
                        "citedCount": cited.len() as i32,
                        "quoteCount": quotes.len() as i32,
                        // 被 filter 静默丢弃的数量 = attempted - kept。>0 且 citedCount==0
                        // 时，真因是引用格式不合规，不是知识缺失。
                        "attemptedCitedCount": attempted_cited,
                        "attemptedQuoteCount": attempted_quotes,
                    },
                );
                let result = AnswerResult {
                    answer,
                    resolution,
                    cited_chunk_ids: cited,
                    source_quotes: quotes,
                    tool_trace,
                    rounds_used: round,
                    truncated: false,
                    cancelled: false,
                };
                if let Some(k) = cache_key.clone() {
                    // A hot swap may happen after lookup and before the final
                    // LLM round. Never store a new generation's answer under
                    // the generation observed at lookup.
                    let provider =
                        current_provider_cache_identity(state, &req.workspace_id).await?;
                    if k.provider_id == provider.provider_id
                        && k.provider_model == provider.model
                        && k.provider_generation == provider.generation
                    {
                        cache::put(k, result.clone());
                    }
                }
                return Ok(result);
            }
        }
    }

    // 兜底：未在循环内 answer。可能原因：跑完 max_rounds、budget 提前 break、
    // 多次 invalid_action 把轮数耗光、客户端取消。rounds_used 上报真实跑过的轮数
    //（最低 0），而不是 max_rounds，避免前端误读。
    // The loop did not produce an AI-selected answer/evidence pair. Opened chunks may inform the
    // generic fallback text, but opening a chunk alone is not a citation. Do not manufacture
    // evidence IDs here; a later consumer must never mistake exploration for grounded support.
    let cited_chunk_ids: Vec<String> = Vec::new();
    push_trace(
        &mut tool_trace,
        tx,
        doc! {
            "tool": "answer",
            "rounds": last_completed_round,
            "truncated": true,
            "cancelled": cancelled,
            "citedCount": cited_chunk_ids.len() as i32,
        },
    );
    let answer_text = if cancelled {
        "取消：agent 已停止探索；返回当前已打开的 chunk 摘要。".to_string()
    } else {
        "知识库未在限定轮数内得出结论；已返回当前打开的 chunk 摘要供运营人员判断。".to_string()
    };
    Ok(AnswerResult {
        answer: answer_text,
        resolution: KnowledgeResolution::default(),
        cited_chunk_ids,
        source_quotes: Vec::new(),
        tool_trace,
        rounds_used: last_completed_round,
        truncated: true,
        cancelled,
    })
}

/// `cancel.is_some_and(|c| c.load(Relaxed))` 的简短形式。`Relaxed` 足够：
/// 取消是单向 false→true，跨任务延迟一两轮可接受（软取消语义）。
fn is_cancelled(cancel: Option<&Arc<AtomicBool>>) -> bool {
    cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false)
}

/// 列出 chunk 摘要（不含 body）。`query` 非空时按 [`rank_key`]（relevance ×
/// trust/recency + 静态平手序）在 `CATALOG_CANDIDATE_CAP` 候选窗内**全量重排**
/// 再截 `CATALOG_PAGE_SIZE`，修 #619「与查询高相关但静态置信度排在窗外」的漏召；
/// query 为空时退化为 live 优先 + 静态序。account_id=None 时只看 workspace 共享
/// chunk；带 account_id 时合并查（共享 + 私有）。
pub async fn list_catalog(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    filter: &CatalogFilter,
    query: Option<&str>,
) -> AppResult<Vec<CatalogEntry>> {
    let mut q = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "status": filter.status.clone().unwrap_or_else(|| "active".to_string()),
    };
    // 默认仅暴露 verified chunk（与 router corpus 对齐）。include_unverified=true
    // 由上层显式开启（例如知识库后台审阅 UI 想看 needs_review）。
    if !filter.include_unverified {
        q.insert("integrity_status", "verified");
    }
    let account_or = match account_id {
        Some(id) => vec![doc! { "account_id": null }, doc! { "account_id": id }],
        None => vec![doc! { "account_id": null }],
    };
    q.insert("$or", account_or);
    if !filter.wiki_types.is_empty() {
        q.insert("wiki_type", doc! { "$in": filter.wiki_types.clone() });
    }
    if !filter.business_topics.is_empty() {
        q.insert(
            "business_topics",
            doc! { "$in": filter.business_topics.clone() },
        );
    }

    // DB 侧仍按静态序拉一个宽候选窗（CATALOG_CANDIDATE_CAP），把 query 相关度重排
    // 放进程内做——MongoDB `$text` 不分 CJK 词，靠 [`rank_key`] 的 bigram 覆盖率。
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            q,
            FindOptions::builder()
                .sort(doc! {
                    "dynamic_confidence": -1,
                    "priority": -1,
                    "updated_at": -1,
                })
                .limit(CATALOG_CANDIDATE_CAP as i64)
                .build(),
        )
        .await?;
    let mut chunks: Vec<OperationKnowledgeChunk> = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        chunks.push(item);
    }

    let now = DateTime::now();
    let query = query.unwrap_or("");
    chunks.sort_by(|a, b| {
        let ka = rank_key(query, a, now);
        let kb = rank_key(query, b, now);
        kb.cmp(&ka)
    });
    chunks.truncate(CATALOG_PAGE_SIZE);

    Ok(chunks.into_iter().map(chunk_to_catalog_entry).collect())
}

/// D3(b)：superseded_by 版本链跟随的最大跳数（防超长链 / 兜底）。
const MAX_REDIRECT_HOPS: usize = 8;

/// D3(b)：沿 `superseded_by` 链把一个 chunk_id 解析到**现行版本**的 ObjectId。
///
/// 语义：一条 chunk 若 `superseded_by` 非空（指向取代它的新版 chunk_id hex），且新版
/// 存在、同 workspace、`integrity_status="verified"`，就 redirect 到新版；新版自己又被
/// superseded 时继续跟随，直到链尾 / 命中环 / 达 [`MAX_REDIRECT_HOPS`]。任一跳的新版
/// 不满足条件（不存在 / 跨 workspace / 未 verified / id 非法）即**停在当前**有效版本，
/// 绝不跳到一个取不到或未审定的目标。返回最终现行版本的 ObjectId。
///
/// 纯 redirect 解析（只读 `_id` + `superseded_by`），不强制 verified 门——起点可能是
/// 任意 chunk（调用方各自把 verified 门）。防环：visited 集合记录已访问 hex。
async fn resolve_superseded(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    start: ObjectId,
) -> AppResult<ObjectId> {
    let mut current = start;
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(current.to_hex());
    for _ in 0..MAX_REDIRECT_HOPS {
        let chunk = state
            .db
            .operation_knowledge_chunks()
            .find_one(
                chunk_scope_filter(workspace_id, account_id, doc! { "_id": current }),
                None,
            )
            .await?;
        let Some(chunk) = chunk else {
            return Ok(current);
        };
        // superseded_by 空白 / 缺失 → current 即现行版本。
        let next_hex = match chunk.superseded_by.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(current),
        };
        // 防环：已访问过的 id 不再跟随，停在 current。
        if !visited.insert(next_hex.clone()) {
            return Ok(current);
        }
        let next_oid = match ObjectId::parse_str(&next_hex) {
            Ok(oid) => oid,
            Err(_) => return Ok(current),
        };
        // 新版必须存在、同 workspace、verified 才 redirect；否则停在 current。
        let next_ok = state
            .db
            .operation_knowledge_chunks()
            .find_one(
                visible_chunk_filter(
                    workspace_id,
                    account_id,
                    doc! { "_id": next_oid, "integrity_status": "verified" },
                ),
                None,
            )
            .await?
            .is_some();
        if !next_ok {
            return Ok(current);
        }
        current = next_oid;
    }
    Ok(current)
}

/// 打开单条 chunk 的完整正文 + 引用 + relations。
///
/// 默认只返回 `integrity_status="verified"` 的 chunk；非 verified（draft /
/// needs_review）静默返回 `None`，避免 agent cite 到未 verify 的内容。
///
/// D3(b)：若请求的 chunk 已被 `superseded_by` 取代，先 [`resolve_superseded`] 到
/// 现行版本再返回——返回的 `ChunkFull.chunk_id` 即新版 id，agent 看到并 cite 的就是
/// 现行版本，cite⊆opened 不变量天然保持（调用方据此 id 推进 `opened_seen`）。
pub async fn open_chunk(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    chunk_id: &str,
) -> AppResult<Option<ChunkFull>> {
    let oid = match ObjectId::parse_str(chunk_id) {
        Ok(oid) => oid,
        Err(_) => return Ok(None),
    };
    // D3(b)：先把已被取代的旧版 redirect 到现行版本（新版必须 verified）。
    let resolved = resolve_superseded(state, workspace_id, account_id, oid).await?;
    let result = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            visible_chunk_filter(
                workspace_id,
                account_id,
                doc! { "_id": resolved, "integrity_status": "verified" },
            ),
            None,
        )
        .await?;
    Ok(result.map(chunk_to_full))
}

/// #619 分层召回下钻：按 `document_id` 把该文档下所有 verified chunk 的【摘要】
/// 取出，按 [`rank_key`]（query 相关度 + trust/recency）排序后返回 `CatalogEntry`，
/// 供调用方 merge 进 catalog。不直接载正文（避免 prompt 爆炸；agent 再 open_chunk
/// 展开选中原子）。account 可见域与 [`list_catalog`] 一致（共享 + 该 account 私有）。
/// `document_id` 非法 / 不属于本 workspace → 返回空集。
pub async fn open_document(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    document_id: &str,
    query: Option<&str>,
    filter: &CatalogFilter,
) -> AppResult<Vec<CatalogEntry>> {
    let doc_oid = match ObjectId::parse_str(document_id) {
        Ok(oid) => oid,
        Err(_) => return Ok(Vec::new()),
    };
    let mut q = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "document_id": doc_oid,
        "status": filter.status.clone().unwrap_or_else(|| "active".to_string()),
        "$or": account_or(account_id),
    };
    if !filter.include_unverified {
        q.insert("integrity_status", "verified");
    }
    if !filter.wiki_types.is_empty() {
        q.insert("wiki_type", doc! { "$in": filter.wiki_types.clone() });
    }
    if !filter.business_topics.is_empty() {
        q.insert(
            "business_topics",
            doc! { "$in": filter.business_topics.clone() },
        );
    }
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            q,
            FindOptions::builder()
                .sort(doc! { "dynamic_confidence": -1, "priority": -1, "updated_at": -1 })
                .limit(CATALOG_CANDIDATE_CAP as i64)
                .build(),
        )
        .await?;
    let mut chunks: Vec<OperationKnowledgeChunk> = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        chunks.push(item);
    }
    let now = DateTime::now();
    let query = query.unwrap_or("");
    chunks.sort_by(|a, b| rank_key(query, b, now).cmp(&rank_key(query, a, now)));
    chunks.truncate(CATALOG_PAGE_SIZE);
    Ok(chunks.into_iter().map(chunk_to_catalog_entry).collect())
}
/// 最先发现的前 [`FOLLOW_PREFETCH_BODIES`] 个 verified 关联目标的【完整正文】
/// 直接以 `ChunkFull` 返回（供调用方推进 `opened`，当轮即可 cite），其余只回
/// 摘要 `CatalogEntry`。已 opened 的目标在遍历中被跳过，避免重复给 agent。
pub async fn follow_relations(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    chunk_id: &str,
    depth: u32,
    opened_seen: &HashSet<String>,
) -> AppResult<(Vec<CatalogEntry>, Vec<ChunkFull>)> {
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(chunk_id.to_string());
    let mut frontier: Vec<String> = vec![chunk_id.to_string()];
    // 按发现顺序收集完整文档：depth-1 在前、depth-2 在后；split_prefetch 取最前
    // FOLLOW_PREFETCH_BODIES 个载正文（即最相关的直接关联），其余转摘要。
    // D3(a)：每条目标带上它经哪种关系角色拉入（Support / Contradiction），
    // Contradiction 在转 ChunkFull 时标 relation_role，prompt 警示「勿作支撑引用」。
    let mut collected: Vec<(OperationKnowledgeChunk, RelationRole)> = Vec::new();

    'outer: for _ in 0..depth {
        let mut next: Vec<String> = Vec::new();
        for src in frontier.drain(..) {
            let oid = match ObjectId::parse_str(&src) {
                Ok(oid) => oid,
                Err(_) => continue,
            };
            let chunk = state
                .db
                .operation_knowledge_chunks()
                .find_one(
                    visible_chunk_filter(workspace_id, account_id, doc! { "_id": oid }),
                    None,
                )
                .await?;
            let Some(chunk) = chunk else { continue };
            let related = chunk.related_chunks.unwrap_or_default();
            for rel in related {
                // D3(a)：按 relation_kind 的语义角色分流。
                let role = classify_relation_role(&rel.kind);
                // Version（superseded_by）是版本指针，不当普通关系扩散——版本 redirect
                // 由 open_chunk/resolve_superseded 处理，这里跳过避免把旧→新版指针当材料。
                if role == RelationRole::Version {
                    continue;
                }
                // 仅**检查**原始关系目标是否已访问，不在此处占位插入——去重的唯一
                // insert 留给下方 resolved_hex（:1315）。否则未被取代的目标
                // （resolve_superseded 原样返回，resolved_hex == rel.chunk_id）会在这里
                // 先占位、到 :1315 二次 insert 必返 false → 被误 continue 丢弃（D3 关系
                // 图谱预取对所有非 superseded 目标整体失效）。superseded 目标的原始 id
                // 此处不占位，由 resolved_hex 去重；防环（指回 source / 已收集目标）也
                // 由 visited 里已有的现行 id 兜住。
                if visited.contains(&rel.chunk_id) {
                    continue;
                }
                let target_oid = match ObjectId::parse_str(&rel.chunk_id) {
                    Ok(oid) => oid,
                    Err(_) => continue,
                };
                // D3(b)：关系目标若已被取代，redirect 到现行版本再收集，避免旧版正文进 opened。
                let resolved_oid =
                    resolve_superseded(state, workspace_id, account_id, target_oid).await?;
                let resolved_hex = resolved_oid.to_hex();
                // redirect 后的现行版可能已被 open / 已收集过 → 去重。
                if opened_seen.contains(&resolved_hex) || !visited.insert(resolved_hex.clone()) {
                    continue;
                }
                if let Some(target) = state
                    .db
                    .operation_knowledge_chunks()
                    .find_one(
                        visible_chunk_filter(
                            workspace_id,
                            account_id,
                            doc! { "_id": resolved_oid, "integrity_status": "verified" },
                        ),
                        None,
                    )
                    .await?
                {
                    collected.push((target, role));
                    next.push(resolved_hex);
                    if collected.len() >= FOLLOW_RELATIONS_LIMIT {
                        break 'outer;
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }

    let (prefetch_chunks, rest_chunks) = split_prefetch(collected, FOLLOW_PREFETCH_BODIES);
    let prefetched: Vec<ChunkFull> = prefetch_chunks
        .into_iter()
        .map(|(chunk, role)| {
            let mut full = chunk_to_full(chunk);
            if role == RelationRole::Contradiction {
                full.relation_role = Some("contradiction".to_string());
            }
            full
        })
        .collect();
    let catalog: Vec<CatalogEntry> = rest_chunks
        .into_iter()
        .map(|(chunk, _role)| chunk_to_catalog_entry(chunk))
        .collect();
    Ok((catalog, prefetched))
}

fn chunk_to_catalog_entry(chunk: OperationKnowledgeChunk) -> CatalogEntry {
    let chunk_id = chunk.id.map(|oid| oid.to_hex()).unwrap_or_default();
    let summary = chunk
        .summary
        .clone()
        .or_else(|| chunk.body.clone())
        .map(|s| truncate_chars(&s, CATALOG_SUMMARY_CHARS))
        .unwrap_or_default();
    let related_count = chunk.related_chunks.as_ref().map(|v| v.len()).unwrap_or(0) as i32;
    let verified = chunk
        .integrity_status
        .as_deref()
        .map(|s| s == "verified")
        .unwrap_or(false);
    CatalogEntry {
        chunk_id,
        wiki_type: chunk.wiki_type.clone().unwrap_or_default(),
        chunk_type: chunk.chunk_type.clone(),
        title: chunk.title.clone(),
        summary,
        business_topics: chunk.business_topics.clone(),
        verified,
        has_source_quote: chunk
            .source_quote
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
        dynamic_confidence: chunk.dynamic_confidence.unwrap_or(0.0),
        related_count,
    }
}

fn chunk_to_full(chunk: OperationKnowledgeChunk) -> ChunkFull {
    let chunk_id = chunk.id.map(|oid| oid.to_hex()).unwrap_or_default();
    let related_chunks = chunk
        .related_chunks
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|rel| RelatedRefView {
            chunk_id: rel.chunk_id,
            kind: rel.kind,
            note: rel.note,
        })
        .collect();
    let verified = chunk
        .integrity_status
        .as_deref()
        .map(|s| s == "verified")
        .unwrap_or(false);
    ChunkFull {
        chunk_id,
        wiki_type: chunk.wiki_type.unwrap_or_default(),
        chunk_type: chunk.chunk_type,
        title: chunk.title,
        summary: chunk.summary.unwrap_or_default(),
        body: chunk.body.unwrap_or_default(),
        source_quote: chunk.source_quote,
        source_anchors: chunk.source_anchors,
        related_chunks,
        verified,
        business_topics: chunk.business_topics,
        relation_role: None,
    }
}

/// D3(a)：`related_chunks` 的 `relation_kind` 在 BFS 遍历时的语义角色。
///
/// 6 种 kind（`ALLOWED_RELATION_KINDS`，routes/knowledge/wiki_edit.rs）按检索语义
/// 分三类——决定 `follow_relations` 该如何对待跳转目标：
/// - **Support**：references/requires/clarifies/refines——正向支撑材料，照旧跟随、
///   作为 opened 正文喂给 agent。
/// - **Contradiction**：contradicts——「与来源 chunk 矛盾的说法」，是警示标记而非
///   支撑。仍跟随（用户要 agent 能看到做对比辨别），但在 prompt 里标注「仅供辨别、
///   勿作支撑引用」（见 `ChunkFull.relation_role` + `build_prompt`）。
/// - **Version**：superseded_by——版本指针，不作为普通关系扩散（旧版指向新版，应由
///   `resolve_superseded` 在 open/follow 时做版本 redirect，而非当关系材料拉入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationRole {
    Support,
    Contradiction,
    Version,
}

/// 把 `relation_kind` 字符串映射到 [`RelationRole`]。未知 kind 归 Support（保守：
/// 宁可当普通支撑跟随，也不静默丢弃——白名单校验在写入侧 `relate` 端点已把关）。
pub fn classify_relation_role(kind: &str) -> RelationRole {
    match kind {
        "contradicts" => RelationRole::Contradiction,
        "superseded_by" => RelationRole::Version,
        // references / requires / clarifies / refines + 任何未知 kind
        _ => RelationRole::Support,
    }
}

fn merge_catalog(target: &mut Vec<CatalogEntry>, incoming: Vec<CatalogEntry>) {
    let mut seen: HashSet<String> = target.iter().map(|e| e.chunk_id.clone()).collect();
    for entry in incoming {
        if seen.insert(entry.chunk_id.clone()) {
            target.push(entry);
        }
    }
    if target.len() > CATALOG_PAGE_SIZE * 2 {
        target.truncate(CATALOG_PAGE_SIZE * 2);
    }
}

/// PBT 入口：merge_catalog 的纯版本，便于在 tests/knowledge_agent_pbt.rs
/// 验证幂等性 / 去重 / 顺序保留。
pub fn merge_catalog_pure(target: &mut Vec<CatalogEntry>, incoming: Vec<CatalogEntry>) {
    merge_catalog(target, incoming)
}

/// 把按发现顺序收集的关联目标切成「前 `cap` 个载正文 / 其余转摘要」两段。
/// 纯函数，供 [`follow_relations`] 调用，并由 cfg(test) 单测 + PBT 锁死不变量：
/// 1. `prefetch.len() <= cap`；
/// 2. `prefetch` ⧺ `rest` 顺序拼接 == 原输入（不丢不乱序）；
/// 3. 两段无交叠（按位置切分，天然无重复）。
pub fn split_prefetch<T>(mut items: Vec<T>, cap: usize) -> (Vec<T>, Vec<T>) {
    let take = cap.min(items.len());
    let rest = items.split_off(take);
    (items, rest)
}

/// 从 LLM 输出的 cited_chunk_ids / source_quotes 过滤出仅包含已 open 过的
/// chunk 的子集。这是 plan v3 P0 的"answer subset of opened"硬不变量：
/// LLM 不许凭空创造没有打开过的 chunk_id。
///
/// **公开供 PBT 使用**——`tests/knowledge_agent_pbt.rs` 把这个函数作为
/// 纯逻辑测试入口，验证：
/// 1. cited 永远是 opened_seen 的子集；
/// 2. quote.chunk_id 必须命中 opened_seen 才保留；
/// 3. 空 quote chunk_id / 不在 opened_seen 中的全部丢掉。
pub fn filter_answer_against_opened(
    opened_seen: &HashSet<String>,
    cited_chunk_ids: Vec<String>,
    raw_quotes: Vec<RawSourceQuote>,
) -> (Vec<String>, Vec<SourceQuoteCitation>) {
    let cited: Vec<String> = cited_chunk_ids
        .into_iter()
        .filter(|id| opened_seen.contains(id))
        .collect();
    let quotes: Vec<SourceQuoteCitation> = raw_quotes
        .into_iter()
        .filter(|q| !q.chunk_id.is_empty() && opened_seen.contains(&q.chunk_id))
        .map(|q| SourceQuoteCitation {
            chunk_id: q.chunk_id,
            quote: q.quote,
            source_anchor_index: q.source_anchor_index,
        })
        .collect();
    (cited, quotes)
}

/// Production evidence-integrity filter for an AI-selected answer.
///
/// The AI still decides which knowledge is relevant and what to say. This function only proves
/// that every accepted citation refers to a verified, opened, non-contradiction chunk and that
/// every accepted quote is literal evidence from that cited chunk. The anchor index is required,
/// must exist, and its own sourceQuote must identify the same evidence.
pub fn filter_answer_against_opened_chunks(
    opened: &[ChunkFull],
    cited_chunk_ids: Vec<String>,
    raw_quotes: Vec<RawSourceQuote>,
) -> (Vec<String>, Vec<SourceQuoteCitation>) {
    let eligible: std::collections::HashMap<&str, &ChunkFull> = eligible_opened_chunks(opened)
        .map(|chunk| (chunk.chunk_id.as_str(), chunk))
        .collect();

    let mut requested_seen = HashSet::new();
    let requested: Vec<String> = cited_chunk_ids
        .into_iter()
        .filter(|id| eligible.contains_key(id.as_str()) && requested_seen.insert(id.clone()))
        .collect();
    let requested_set: HashSet<&str> = requested.iter().map(String::as_str).collect();

    let quotes: Vec<SourceQuoteCitation> = raw_quotes
        .into_iter()
        .filter_map(|raw| {
            let chunk = eligible.get(raw.chunk_id.as_str())?;
            if !requested_set.contains(raw.chunk_id.as_str())
                || !quote_is_chunk_evidence(chunk, &raw)
            {
                return None;
            }
            Some(SourceQuoteCitation {
                chunk_id: raw.chunk_id,
                quote: raw.quote.trim().to_string(),
                source_anchor_index: raw.source_anchor_index,
            })
        })
        .collect();
    let evidenced: HashSet<&str> = quotes.iter().map(|quote| quote.chunk_id.as_str()).collect();
    let cited = requested
        .into_iter()
        .filter(|id| evidenced.contains(id.as_str()))
        .collect();
    (cited, quotes)
}

fn eligible_opened_chunks(opened: &[ChunkFull]) -> impl Iterator<Item = &ChunkFull> {
    opened.iter().filter(|chunk| {
        chunk.verified
            && !chunk.chunk_id.trim().is_empty()
            && chunk.relation_role.as_deref() != Some("contradiction")
    })
}

fn quote_is_chunk_evidence(chunk: &ChunkFull, raw: &RawSourceQuote) -> bool {
    let quote = normalize_evidence_text(&raw.quote);
    if quote.is_empty() {
        return false;
    }
    let appears_in_chunk = chunk
        .source_quote
        .as_deref()
        .into_iter()
        .chain(std::iter::once(chunk.body.as_str()))
        .map(normalize_evidence_text)
        .any(|text| text.contains(&quote));
    if !appears_in_chunk {
        return false;
    }

    // A quote without an anchor remains useful context for the AI but cannot become an accepted
    // evidence citation. This keeps the read path aligned with the knowledge verification gate.
    let Some(index) = raw.source_anchor_index else {
        return false;
    };
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    let Some(anchor) = chunk.source_anchors.get(index) else {
        return false;
    };
    let Ok(anchor_quote) = anchor.get_str("sourceQuote") else {
        return false;
    };
    let anchor_quote = normalize_evidence_text(anchor_quote);
    !anchor_quote.is_empty() && (anchor_quote.contains(&quote) || quote.contains(&anchor_quote))
}

fn normalize_evidence_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_prompt(
    query: &str,
    opened: &[ChunkFull],
    catalog: &[CatalogEntry],
    round: i32,
    max_rounds: i32,
) -> String {
    let opened_json = serde_json::to_string_pretty(
        &opened
            .iter()
            .map(|c| {
                json!({
                    "chunkId": c.chunk_id,
                    "title": c.title,
                    "wikiType": c.wiki_type,
                    "summary": c.summary,
                    "body": c.body,
                    "sourceQuote": c.source_quote,
                    "sourceAnchors": c.source_anchors.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                    "relatedChunks": c.related_chunks,
                    "verified": c.verified,
                    "businessTopics": c.business_topics,
                    "relationRole": c.relation_role,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let catalog_json =
        serde_json::to_string_pretty(&catalog.iter().map(|c| json!(c)).collect::<Vec<_>>())
            .unwrap_or_default();
    let last_round = round >= max_rounds;
    let force_answer_hint = if last_round {
        "\n这是最后一轮，必须输出 action=answer。"
    } else {
        ""
    };
    format!(
        r#"{ACTION_PROTOCOL}

用户查询：
{query}

候选 catalog（仅摘要，不含正文）：
{catalog_json}

当前轮次：{round}/{max_rounds}{force_answer_hint}

已 open 的 chunks（含正文）：
{opened_json}

现在只输出本轮 action 的严格 JSON。"#,
    )
}

fn filter_to_doc(filter: &CatalogFilter) -> Document {
    to_bson(filter)
        .ok()
        .and_then(|b| b.as_document().cloned())
        .unwrap_or_default()
}

/// 9 类 wiki_type 的排序权重（数值越大越优先）。与
/// `knowledge_router::format_operation_knowledge_for_prompt` 的输出顺序一致。
pub fn wiki_type_priority(wiki_type: Option<&str>) -> i32 {
    match wiki_type.unwrap_or("entity") {
        "thesis" => 90,
        "synthesis" => 80,
        "methodology" => 70,
        "finding" => 60,
        "comparison" => 50,
        "concept" => 40,
        "entity" => 30,
        "source" => 20,
        "query" => 10,
        _ => 0,
    }
}

/// 召回排序键（方法论点 3「排序检索」+ 点 4「trust/recency 层」）。**全序**
/// （`Ord` 派生，字段从主到次按下序词典比较），`list_catalog` 用 `b.cmp(&a)`
/// 降序排（key 大者排前）：
/// 1. `effective_relevance_micros`：query↔chunk 覆盖率 [`relevance_score`] ×
///    trust 因子，量化成微整数。**主键**——relevance 驱动漏斗。
/// 2. `live`：未被 superseded 且未过期 → true。同 relevance 时 live 排在前，
///    兑现「superseded 绝不与现行版同列竞争」。
/// 3. `wiki_priority` → 4. `confidence_micros` → 5. `priority`：静态平手序。
///
/// **trust/recency 降格不剔除**：superseded 把有效相关度乘 0.1（重罚排底，需 10×
/// 原始相关度才追平 live 同行）、过期再乘 0.5。降格的 chunk 仍留在候选里，仍受
/// [`open_chunk`] 的 verified-only 硬门约束——只重排不剔除，cite⊆opened /
/// verified-only 不破。query 为空（无检索意图）时有效相关度恒 0，整体退化为
/// 「live 优先 + 静态序」。
///
/// 纯函数（无 IO），cfg(test) 单测 + `tests/knowledge_agent_pbt.rs` 锁：全序 ∧
/// now 单调（now 增大只可能令 chunk 过期 → 排名只降不升）∧ 「除 superseded 惩罚
/// 外全等的 live 同行」恒排在 superseded 之前。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RankKey {
    pub effective_relevance_micros: i64,
    pub live: bool,
    pub wiki_priority: i32,
    pub confidence_micros: i64,
    pub priority: i32,
}

/// 见 [`RankKey`]。`now` 显式传入（不在函数内取 `DateTime::now()`）以便 PBT 锁
/// now 单调性。
pub fn rank_key(query: &str, chunk: &OperationKnowledgeChunk, now: DateTime) -> RankKey {
    let base = relevance_score(query, &chunk_haystack(chunk));
    let superseded = chunk
        .superseded_by
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let expired = chunk
        .valid_to
        .map(|t| t.timestamp_millis() < now.timestamp_millis())
        .unwrap_or(false);
    let trust_factor = if superseded { 0.1 } else { 1.0 } * if expired { 0.5 } else { 1.0 };
    let effective = base * trust_factor;
    RankKey {
        effective_relevance_micros: (effective * 1_000_000.0) as i64,
        live: !superseded && !expired,
        wiki_priority: wiki_type_priority(chunk.wiki_type.as_deref()),
        confidence_micros: (chunk.dynamic_confidence.unwrap_or(0.0) * 1_000_000.0) as i64,
        priority: chunk.priority,
    }
}

/// 在线召回-trace 闭环（方法论点 5）的失败签名分类器。
///
/// 输入一次 [`AnswerResult`]，输出**一条**待入队的离线整改候选信号
/// （[`GapSignalCandidate`]），或 `None`（本次召回健康，不打扰运营队列）。生产者
/// 在 [`answer`] wrapper 返回后 fire-and-forget 落库（见 `persist_recall_signal`），
/// **绝不在召回热路径内 await**。
///
/// 两类签名（按严重度从高到低短路）：
/// 1. **recall_miss**：`cited==0` 且（`truncated` 或一个正文都没 open）。即「查了
///    等于没查」——空 catalog 偽阴性、4 轮没收敛、或开了一堆摘要却没能 open 到任何
///    正文。这是粒度/放置问题最强的信号。
/// 2. **recall_low_yield**：open 了 ≥ [`LOW_YIELD_OPENED_MIN`] 个正文却只 cite
///    ≤ [`LOW_YIELD_CITED_MAX`] 个——「多 open 少 cite」，典型「找到一堆但都不够
///    精确支撑回答」，提示该主题的原子要么太粗（该 split）要么放错文档（该
///    reclassify）。affected 记 opened-但-未-cite 的那批（诊断价值最高）。
///
/// 设计红线：
/// - `cancelled`（用户主动取消）→ `None`：那不是召回质量问题。
/// - 纯函数、无 IO、永不 panic（解析 trace 全用容错读法，缺字段当空集）。
/// - `affected_chunk_ids ⊆ 本次 opened ∪ cited`（PBT 锁），尊重 workspace 隔离由
///   调用方 `persist_recall_signal(db, workspace_id, _)` 负责。
const LOW_YIELD_OPENED_MIN: usize = 3;
const LOW_YIELD_CITED_MAX: usize = 1;

/// B5：从 answer trace 读回「模型本轮**尝试**了多少条引用」。
///
/// `AnswerResult.cited_chunk_ids` 是 `filter_answer_against_opened_chunks` **过滤后**
/// 的结果，看不出模型原本给没给引用。生产路径在 filter 之前把原始条数写进 answer
/// trace 的 `attemptedCitedCount` / `attemptedQuoteCount`，本函数取两者较大值作为
/// 「尝试过引用」的证据。
///
/// 缺字段（历史 trace / 兜底路径的 answer trace 不带这两个键）时返回 0 —— 退化成旧
/// 行为（判为 `recall_miss`），不会误报成格式问题。
fn attempted_citation_count(tool_trace: &[Document]) -> i64 {
    tool_trace
        .iter()
        .filter(|entry| entry.get_str("tool").unwrap_or("") == "answer")
        .map(|entry| {
            let cited = entry.get_i32("attemptedCitedCount").unwrap_or(0) as i64;
            let quotes = entry.get_i32("attemptedQuoteCount").unwrap_or(0) as i64;
            cited.max(quotes)
        })
        .max()
        .unwrap_or(0)
}

pub fn classify_recall_outcome(result: &AnswerResult) -> Option<GapSignalCandidate> {
    // 用户主动取消不算召回缺陷：提前退出循环是预期行为。
    if result.cancelled {
        return None;
    }

    // 统计本次「载入正文、可被 cite」的 chunk：open_chunk.opened ∪
    // follow_relations.openedBodies。摘要级（open_document/list_catalog appended）
    // 不算——它们没正文，本就不能直接 cite。
    let mut opened: HashSet<String> = HashSet::new();
    for entry in &result.tool_trace {
        let tool = entry.get_str("tool").unwrap_or("");
        let field = match tool {
            "open_chunk" => "opened",
            "follow_relations" => "openedBodies",
            _ => continue,
        };
        if let Ok(arr) = entry.get_array(field) {
            for v in arr {
                if let Bson::String(s) = v {
                    opened.insert(s.clone());
                }
            }
        }
    }
    let cited_count = result.cited_chunk_ids.len();

    // B5：`cited==0` 至少有三种真因，旧实现把它们压成同一条 kind + 同一句
    // description（「疑似目标知识缺失…待补录」），于是引用格式不合规的场景会把运营
    // 指向「补录知识」这个错误方向——补录再多也不会让 cited 变非零。这里按 answer
    // trace 里的 `attemptedCitedCount` / `attemptedQuoteCount` 把真因分开。
    let attempted = attempted_citation_count(&result.tool_trace);

    // 签名 1a：citation_format_rejected —— 模型**试过**引用，但全部被
    // `filter_answer_against_opened_chunks` 拒掉（anchor 缺 sourceQuote / quote 与
    // anchor 不匹配 / 引用了未 open 的 id）。修复方向是重锚定，不是补录。
    if cited_count == 0 && attempted > 0 {
        let affected: Vec<String> = opened.into_iter().collect();
        return Some(GapSignalCandidate {
            kind: "citation_format_rejected".into(),
            title: "引用被拒：模型给出引用但未通过锚点校验".into(),
            severity: "high".into(),
            affected_chunk_ids: affected,
            search_queries: Vec::new(),
            description: "模型本次**给出了引用**，但全部未通过 sourceQuote/锚点校验，\
                 因此没有可用证据。真因通常是这些切片的 source_anchors 缺 sourceQuote、\
                 或 source_quote 与正文不再逐字一致——请重锚定（可触发 AI 自主修复），\
                 而不是补录新知识。"
                .to_string(),
        });
    }

    // 签名 1b：recall_miss —— 模型压根没给引用。区分「跑完轮数没收敛」与「查无内容」：
    // 前者是探索预算问题，后者才是真的知识缺失。两者共用 kind（保持既有收件箱分类与
    // 前端闭集稳定），但 title / description 不同 → dedup_key 不同 → 分列两行，运营
    // 不会再把两类混在一条 pending 里。
    if cited_count == 0 {
        let affected: Vec<String> = opened.into_iter().collect();
        let (title, description) = if result.truncated {
            (
                "召回未收敛：限定轮数内未得出可引用结论",
                "模型在限定轮数内没能收敛到可引用的知识（未给出任何引用）。可能是\
                 议题过宽、需要拆分提问，或知识分布过散导致探索预算不足；请先核对该\
                 议题是否确有对应知识，再决定补录或拆分。",
            )
        } else {
            (
                "召回偽阴性：查询未命中可引用知识",
                "本次没检索到可引用的知识，且模型未尝试任何引用。疑似目标知识缺失、\
                 粒度过粗或放置错位，待运营质检定位后补录、拆分或重新归类。",
            )
        };
        return Some(GapSignalCandidate {
            kind: "recall_miss".into(),
            title: title.into(),
            severity: "high".into(),
            affected_chunk_ids: affected,
            search_queries: Vec::new(),
            description: description.to_string(),
        });
    }

    // 签名 2：recall_low_yield。
    if opened.len() >= LOW_YIELD_OPENED_MIN && cited_count <= LOW_YIELD_CITED_MAX {
        let cited_set: HashSet<&str> = result.cited_chunk_ids.iter().map(|s| s.as_str()).collect();
        // affected = open 了正文却没被 cite 的那批（诊断价值最高）；恒 ⊆ opened。
        let affected: Vec<String> = opened
            .iter()
            .filter(|id| !cited_set.contains(id.as_str()))
            .cloned()
            .collect();
        return Some(GapSignalCandidate {
            kind: "recall_low_yield".into(),
            title: "召回低产出：多 open 少 cite".into(),
            severity: "medium".into(),
            affected_chunk_ids: affected,
            search_queries: Vec::new(),
            description: "本次翻阅了多条知识正文，可真正引用的却很少。知识条目可能\
                 粒度过粗或放错文档，待运营质检判断是否拆分或重新归类。"
                .to_string(),
        });
    }

    None
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// `account_id=None` 只看 workspace 共享（account_id=null）；带 id 时合并查
///（共享 + 该 account 私有）。三处召回查询（chunk catalog / document 目录 /
/// open_document）共用，保证可见域一致。
fn account_or(account_id: Option<&str>) -> Vec<Document> {
    match account_id {
        Some(id) => vec![doc! { "account_id": null }, doc! { "account_id": id }],
        None => vec![doc! { "account_id": null }],
    }
}

/// Account/domain capability only. This intentionally does not require an
/// active/verified row: an archived predecessor must remain readable just
/// long enough to resolve its `superseded_by` pointer. Callers that expose
/// content must apply [`visible_chunk_filter`] to the final target.
fn chunk_scope_filter(workspace_id: &str, account_id: Option<&str>, extra: Document) -> Document {
    let mut filter = doc! {
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "$or": account_or(account_id),
    };
    filter.extend(extra);
    filter
}

fn visible_chunk_filter(workspace_id: &str, account_id: Option<&str>, extra: Document) -> Document {
    let mut filter = chunk_scope_filter(
        workspace_id,
        account_id,
        doc! {
            "status": "active",
            "integrity_status": "verified",
        },
    );
    filter.extend(extra);
    filter
}

async fn visible_corpus_signature(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
) -> AppResult<u64> {
    let options = FindOptions::builder()
        .projection(doc! { "_id": 1_i32, "updated_at": 1_i32 })
        .sort(doc! { "_id": 1_i32 })
        .build();
    let mut cursor = state
        .db
        .raw()
        .collection::<Document>("operation_knowledge_chunks")
        .find(
            chunk_scope_filter(workspace_id, account_id, Document::new()),
            options,
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        let id = item
            .get_object_id("_id")
            .map(|value| value.to_hex())
            .unwrap_or_else(|_| "missing-object-id".to_string());
        let updated_at = item
            .get_datetime("updated_at")
            .map(|value| value.timestamp_millis())
            .unwrap_or(i64::MIN);
        items.push((id, updated_at));
    }
    Ok(cache::corpus_signature(&items))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderCacheIdentity {
    provider_id: String,
    model: String,
    generation: u64,
}

async fn current_provider_cache_identity(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<ProviderCacheIdentity> {
    match super::resolve_llm_registry_snapshot(state, workspace_id).await? {
        Some(snapshot) => Ok(ProviderCacheIdentity {
            provider_id: snapshot.meta.provider_id,
            model: snapshot.meta.model,
            generation: snapshot.generation,
        }),
        None => Ok(ProviderCacheIdentity {
            provider_id: "injected".to_string(),
            model: state.config.openai_model.clone(),
            generation: 0,
        }),
    }
}

fn normalized_filter_key(filter: &CatalogFilter) -> String {
    fn normalized_values(values: &[String]) -> String {
        let mut values: Vec<String> = values
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();
        values.sort();
        values.dedup();
        values.join("\u{1f}")
    }

    format!(
        "wiki={};topics={};status={};include_unverified={}",
        normalized_values(&filter.wiki_types),
        normalized_values(&filter.business_topics),
        filter.status.as_deref().map(str::trim).unwrap_or("active"),
        filter.include_unverified,
    )
}

fn intersect_catalog_filters(
    base: &CatalogFilter,
    requested: &CatalogFilter,
) -> Option<CatalogFilter> {
    fn intersect_values(base: &[String], requested: &[String]) -> Option<Vec<String>> {
        let normalize = |values: &[String]| {
            values
                .iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<HashSet<_>>()
        };
        let base = normalize(base);
        let requested = normalize(requested);
        let both_scoped = !base.is_empty() && !requested.is_empty();
        let mut values: Vec<String> = if base.is_empty() {
            requested.into_iter().collect()
        } else if requested.is_empty() {
            base.into_iter().collect()
        } else {
            base.intersection(&requested).cloned().collect()
        };
        if both_scoped && values.is_empty() {
            return None;
        }
        values.sort();
        Some(values)
    }

    let explicit_status = |filter: &CatalogFilter| -> Option<String> {
        filter
            .status
            .as_deref()
            .map(str::trim)
            .filter(|status| !status.is_empty())
            .map(ToString::to_string)
    };
    let base_status = explicit_status(base).unwrap_or_else(|| "active".to_string());
    if explicit_status(requested)
        .as_deref()
        .is_some_and(|requested| requested != base_status.as_str())
    {
        return None;
    }
    let status = Some(base_status);

    Some(CatalogFilter {
        wiki_types: intersect_values(&base.wiki_types, &requested.wiki_types)?,
        business_topics: intersect_values(&base.business_topics, &requested.business_topics)?,
        status,
        include_unverified: base.include_unverified && requested.include_unverified,
    })
}

/// 语言无关的 query↔文本相关度，落在 `[0,1]`：query 的检索信号在候选文本里被
/// 覆盖的比例。CJK 用相邻字符 bigram（单字回退 unigram）、ASCII 用连续 alnum
/// token。零依赖、对中英混排鲁棒，**不是**针对任何样本调过的特征——纯覆盖率。
///
/// 用于 [`list_catalog`] 在 query 非空时对候选 chunk 重排：把「与查询高相关但静态
/// 置信度排在 30 名外」的 chunk 拉进 catalog 头部，修 #619 静态截断漏召。query
/// 为空（无检索意图）返回 0，调用方回退到静态 wiki_type/confidence 排序。
///
/// 纯函数，cfg(test) 单测 + `tests/knowledge_agent_pbt.rs` 锁不变量：
/// 结果恒 `[0,1]`；query/haystack 任一为空信号集 → 0；haystack ⊇ query 信号 → 1。
pub fn relevance_score(query: &str, haystack: &str) -> f64 {
    let q = text_signals(query);
    if q.is_empty() {
        return 0.0;
    }
    let h = text_signals(haystack);
    if h.is_empty() {
        return 0.0;
    }
    let hit = q.iter().filter(|t| h.contains(*t)).count();
    hit as f64 / q.len() as f64
}

/// 把文本拆成「检索信号」集合：ASCII 连续 alnum 串（小写整体当一个 token）+
/// 相邻 CJK 字符 bigram（单字 run 回退 unigram，覆盖单字查询）。其它字符当分隔符。
fn text_signals(s: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let mut ascii = String::new();
    let mut cjk_run: Vec<char> = Vec::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            push_cjk(&cjk_run, &mut out);
            cjk_run.clear();
            ascii.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            if !ascii.is_empty() {
                out.insert(std::mem::take(&mut ascii));
            }
            cjk_run.push(ch);
        } else {
            if !ascii.is_empty() {
                out.insert(std::mem::take(&mut ascii));
            }
            push_cjk(&cjk_run, &mut out);
            cjk_run.clear();
        }
    }
    if !ascii.is_empty() {
        out.insert(ascii);
    }
    push_cjk(&cjk_run, &mut out);
    out
}

/// 把一段连续 CJK run 拆成 bigram 写进 `out`；单字 run 回退 unigram（覆盖单字
/// 查询，否则单字永远 0 命中）。空 run 无操作。
fn push_cjk(run: &[char], out: &mut HashSet<String>) {
    match run.len() {
        0 => {}
        1 => {
            out.insert(run[0].to_string());
        }
        _ => {
            for w in run.windows(2) {
                out.insert(w.iter().collect());
            }
        }
    }
}

/// 常见 CJK / 假名码段。够覆盖中文与中日混排私域语料；非穷举（不含韩文/扩展 B+），
/// 命中之外的字符当分隔符处理，不影响 ASCII / 中文主路径。
fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF      // CJK 统一表意文字
        | 0x3400..=0x4DBF    // 扩展 A
        | 0x3040..=0x30FF    // 平假名 + 片假名
        | 0xF900..=0xFAFF    // 兼容表意文字
    )
}

/// 把一条 chunk 的可检索文本拼成一个 haystack，喂给 [`relevance_score`]。
/// 覆盖 title / summary / body / business_topics / wiki_type —— 召回信号尽量全，
/// 不漏掉只在正文出现的关键词。
fn chunk_haystack(c: &OperationKnowledgeChunk) -> String {
    let mut s = String::with_capacity(256);
    s.push_str(&c.title);
    if let Some(x) = &c.summary {
        s.push(' ');
        s.push_str(x);
    }
    if let Some(x) = &c.body {
        s.push(' ');
        s.push_str(x);
    }
    for t in &c.business_topics {
        s.push(' ');
        s.push_str(t);
    }
    for t in &c.product_tags {
        s.push(' ');
        s.push_str(t);
    }
    if let Some(x) = &c.wiki_type {
        s.push(' ');
        s.push_str(x);
    }
    s
}

/// PBT 入口：knowledge_agent 多轮预算上限。
pub const PBT_MAX_ROUNDS: i32 = MAX_ROUNDS;
/// PBT 入口：catalog 摘要截断长度（CJK char 数）。
pub const PBT_CATALOG_SUMMARY_CHARS: usize = CATALOG_SUMMARY_CHARS;
/// PBT 入口：catalog 单页上限。
pub const PBT_CATALOG_PAGE_SIZE: usize = CATALOG_PAGE_SIZE;

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_helpers {
    use super::*;

    /// 给 PBT 用：把任意字符串当作 chunk_id 在内存里造一条 ChunkFull。
    pub fn synthetic_chunk(chunk_id: &str, title: &str, body: &str) -> ChunkFull {
        ChunkFull {
            chunk_id: chunk_id.to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: title.to_string(),
            summary: title.to_string(),
            body: body.to_string(),
            source_quote: Some(body.to_string()),
            source_anchors: Vec::new(),
            related_chunks: Vec::new(),
            verified: true,
            business_topics: Vec::new(),
            relation_role: None,
        }
    }

    pub fn synthetic_catalog_entry(chunk_id: &str, title: &str) -> CatalogEntry {
        CatalogEntry {
            chunk_id: chunk_id.to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: title.to_string(),
            summary: title.to_string(),
            business_topics: Vec::new(),
            verified: true,
            has_source_quote: true,
            dynamic_confidence: 0.9,
            related_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence_chunk(chunk_id: &str, relation_role: Option<&str>) -> ChunkFull {
        ChunkFull {
            chunk_id: chunk_id.to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: "Evidence".to_string(),
            summary: "Evidence summary".to_string(),
            body: "The supported fact is 42 units.".to_string(),
            source_quote: Some("The supported fact is 42 units.".to_string()),
            source_anchors: vec![doc! {
                "sourceQuote": "The supported fact is 42 units.",
                "startOffset": 0_i32,
                "endOffset": 31_i32,
            }],
            related_chunks: Vec::new(),
            verified: true,
            business_topics: Vec::new(),
            relation_role: relation_role.map(ToString::to_string),
        }
    }

    fn raw_quote(chunk_id: &str, quote: &str, anchor: Option<i32>) -> RawSourceQuote {
        RawSourceQuote {
            chunk_id: chunk_id.to_string(),
            quote: quote.to_string(),
            source_anchor_index: anchor,
        }
    }

    #[test]
    fn strict_evidence_accepts_ai_selected_anchored_quote() {
        let opened = vec![evidence_chunk("support", None)];
        let (cited, quotes) = filter_answer_against_opened_chunks(
            &opened,
            vec!["support".to_string()],
            vec![raw_quote("support", "supported fact is 42 units", Some(0))],
        );

        assert_eq!(cited, vec!["support"]);
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].source_anchor_index, Some(0));
    }

    #[test]
    fn strict_evidence_rejects_contradiction_even_when_quote_is_real() {
        let opened = vec![evidence_chunk("contra", Some("contradiction"))];
        let (cited, quotes) = filter_answer_against_opened_chunks(
            &opened,
            vec!["contra".to_string()],
            vec![raw_quote("contra", "supported fact is 42 units", Some(0))],
        );

        assert!(cited.is_empty());
        assert!(quotes.is_empty());
    }

    #[test]
    fn catalog_filter_intersection_never_widens_initial_scope() {
        let base = CatalogFilter {
            wiki_types: vec!["methodology".to_string(), "finding".to_string()],
            business_topics: vec!["sales".to_string()],
            status: None,
            include_unverified: false,
        };
        let requested = CatalogFilter {
            wiki_types: vec!["finding".to_string(), "source".to_string()],
            business_topics: Vec::new(),
            status: None,
            include_unverified: true,
        };

        let effective = intersect_catalog_filters(&base, &requested)
            .expect("overlapping filters must remain satisfiable");
        assert_eq!(effective.wiki_types, vec!["finding"]);
        assert_eq!(effective.business_topics, vec!["sales"]);
        assert_eq!(effective.status.as_deref(), Some("active"));
        assert!(!effective.include_unverified);
    }

    #[test]
    fn catalog_filter_intersection_rejects_conflicting_scope() {
        let base = CatalogFilter {
            wiki_types: vec!["methodology".to_string()],
            status: None,
            ..CatalogFilter::default()
        };
        let disjoint_type = CatalogFilter {
            wiki_types: vec!["source".to_string()],
            ..CatalogFilter::default()
        };
        assert!(intersect_catalog_filters(&base, &disjoint_type).is_none());

        let archived = CatalogFilter {
            status: Some("archived".to_string()),
            ..CatalogFilter::default()
        };
        assert!(intersect_catalog_filters(&base, &archived).is_none());
    }

    #[test]
    fn catalog_filter_intersection_inherits_non_default_status_when_model_omits_it() {
        let base = CatalogFilter {
            status: Some("draft".to_string()),
            include_unverified: true,
            ..CatalogFilter::default()
        };

        let effective = intersect_catalog_filters(&base, &CatalogFilter::default())
            .expect("omitted model status must inherit the initial capability");
        assert_eq!(effective.status.as_deref(), Some("draft"));
        assert!(!effective.include_unverified);
    }

    #[test]
    fn strict_evidence_rejects_quote_for_id_not_selected_by_ai() {
        let opened = vec![evidence_chunk("support", None)];
        let (cited, quotes) = filter_answer_against_opened_chunks(
            &opened,
            Vec::new(),
            vec![raw_quote("support", "supported fact is 42 units", Some(0))],
        );

        assert!(cited.is_empty());
        assert!(quotes.is_empty());
    }

    #[test]
    fn strict_evidence_rejects_fabricated_quote_and_its_bare_citation() {
        let opened = vec![evidence_chunk("support", None)];
        let (cited, quotes) = filter_answer_against_opened_chunks(
            &opened,
            vec!["support".to_string()],
            vec![raw_quote("support", "fabricated evidence", Some(0))],
        );

        assert!(cited.is_empty());
        assert!(quotes.is_empty());
    }

    #[test]
    fn strict_evidence_rejects_missing_out_of_range_or_mismatched_anchor() {
        let mut opened = vec![evidence_chunk("support", None)];
        for anchor in [None, Some(-1), Some(1)] {
            let (cited, quotes) = filter_answer_against_opened_chunks(
                &opened,
                vec!["support".to_string()],
                vec![raw_quote("support", "supported fact is 42 units", anchor)],
            );
            assert!(cited.is_empty(), "anchor={anchor:?}");
            assert!(quotes.is_empty(), "anchor={anchor:?}");
        }

        opened[0].source_anchors[0].insert("sourceQuote", "different evidence");
        let (cited, quotes) = filter_answer_against_opened_chunks(
            &opened,
            vec!["support".to_string()],
            vec![raw_quote("support", "supported fact is 42 units", Some(0))],
        );
        assert!(cited.is_empty());
        assert!(quotes.is_empty());
    }

    #[test]
    fn classify_relation_role_maps_all_six_kinds() {
        // D3(a)：六种合法 relation_kind 的语义分类锁定。
        assert_eq!(classify_relation_role("references"), RelationRole::Support);
        assert_eq!(classify_relation_role("requires"), RelationRole::Support);
        assert_eq!(classify_relation_role("clarifies"), RelationRole::Support);
        assert_eq!(classify_relation_role("refines"), RelationRole::Support);
        assert_eq!(
            classify_relation_role("contradicts"),
            RelationRole::Contradiction
        );
        assert_eq!(
            classify_relation_role("superseded_by"),
            RelationRole::Version
        );
    }

    #[test]
    fn classify_relation_role_unknown_falls_back_to_support() {
        // 未知 kind 保守归 Support（白名单在 relate 写入侧把关；这里不静默丢弃）。
        assert_eq!(classify_relation_role(""), RelationRole::Support);
        assert_eq!(classify_relation_role("supersedes"), RelationRole::Support);
        assert_eq!(classify_relation_role("random_kind"), RelationRole::Support);
    }

    /// D3(a) 命门：relation_role="contradiction" 的 opened chunk 必须在 build_prompt
    /// 输出里既带 `relationRole` 字段、又有规则区"勿作支撑引用"警示——否则标记送不到
    /// agent = 等于没修。锁住字段名拼写 + 警示话术不被误删。
    #[test]
    fn build_prompt_surfaces_contradiction_marker_and_warning() {
        let contra = ChunkFull {
            chunk_id: "c_contra".to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: "矛盾说法".to_string(),
            summary: "s".to_string(),
            body: "与来源矛盾的正文".to_string(),
            source_quote: None,
            source_anchors: Vec::new(),
            related_chunks: Vec::new(),
            verified: true,
            business_topics: Vec::new(),
            relation_role: Some("contradiction".to_string()),
        };
        let prompt = build_prompt("任意查询", &[contra], &[], 1, 4);
        // 1) opened JSON 必须把 relation_role 渲染成 relationRole 字段（字段名拼写锁）。
        assert!(
            prompt.contains("\"relationRole\": \"contradiction\""),
            "build_prompt 必须把 contradiction 标记渲染进 opened JSON: {prompt}"
        );
        // 2) 规则区必须有"勿引用矛盾说法"的警示话术（话术不被误删）。
        assert!(
            prompt.contains("关系角色") && prompt.contains("绝不可作为支撑材料 cite"),
            "build_prompt 规则区必须警示 contradiction 仅供辨别勿作支撑引用: {prompt}"
        );
    }

    #[test]
    fn build_prompt_omits_relation_role_for_normal_opened() {
        // 无 relation_role 的普通 opened chunk：relationRole 渲染为 null（serde 不 skip，
        // 但值为 null），不应出现 "contradiction" 字样——DEFAULT 路径无警示污染。
        let normal = ChunkFull {
            chunk_id: "c_normal".to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: "普通".to_string(),
            summary: "s".to_string(),
            body: "正文".to_string(),
            source_quote: None,
            source_anchors: Vec::new(),
            related_chunks: Vec::new(),
            verified: true,
            business_topics: Vec::new(),
            relation_role: None,
        };
        let prompt = build_prompt("查询", &[normal], &[], 1, 4);
        assert!(
            !prompt.contains("\"relationRole\": \"contradiction\""),
            "普通 chunk 不应带 contradiction 标记"
        );
    }

    #[test]
    fn build_prompt_keeps_protocol_query_and_catalog_before_round_state() {
        let catalog = vec![CatalogEntry {
            chunk_id: "catalog-1".to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: "目录条目".to_string(),
            summary: "摘要".to_string(),
            business_topics: vec!["topic".to_string()],
            verified: true,
            has_source_quote: true,
            dynamic_confidence: 0.9,
            related_count: 0,
        }];
        let opened = ChunkFull {
            chunk_id: "catalog-1".to_string(),
            wiki_type: "methodology".to_string(),
            chunk_type: "product_fact".to_string(),
            title: "目录条目".to_string(),
            summary: "摘要".to_string(),
            body: "完整正文".to_string(),
            source_quote: None,
            source_anchors: Vec::new(),
            related_chunks: Vec::new(),
            verified: true,
            business_topics: vec!["topic".to_string()],
            relation_role: None,
        };

        let first = build_prompt("同一查询", &[], &catalog, 1, 4);
        let second = build_prompt("同一查询", &[opened], &catalog, 2, 4);
        assert!(first.starts_with(ACTION_PROTOCOL));

        let first_round = first.find("当前轮次：").expect("round marker");
        let second_round = second.find("当前轮次：").expect("round marker");
        assert_eq!(
            &first[..first_round],
            &second[..second_round],
            "round-specific state must come after the reusable protocol + query + catalog prefix"
        );
        let query_header = first.find("\n用户查询：").unwrap();
        let catalog_header = first.find("\n候选 catalog（仅摘要，不含正文）：").unwrap();
        assert!(query_header < catalog_header);
        assert!(catalog_header < first_round);
        assert!(first_round < first.find("\n已 open 的 chunks（含正文）：").unwrap());
    }

    #[test]
    fn system_prompt_matches_the_atomic_catalog_runtime() {
        assert!(SYSTEM_PROMPT.contains("原子知识 catalog"));
        assert!(SYSTEM_PROMPT.contains("优先 open_chunk"));
        assert!(!SYSTEM_PROMPT.contains("先读文档目录"));
    }

    #[test]
    fn split_prefetch_caps_and_preserves_order() {
        let (head, tail) = split_prefetch(vec![1, 2, 3, 4, 5], 3);
        assert_eq!(head, vec![1, 2, 3]);
        assert_eq!(tail, vec![4, 5]);
        // 不变量：拼回 == 原输入
        let mut roundtrip = head.clone();
        roundtrip.extend(tail.clone());
        assert_eq!(roundtrip, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn split_prefetch_cap_exceeds_len_takes_all() {
        let (head, tail) = split_prefetch(vec![1, 2], 5);
        assert_eq!(head, vec![1, 2]);
        assert!(tail.is_empty());
    }

    #[test]
    fn split_prefetch_zero_cap_keeps_all_in_rest() {
        let (head, tail) = split_prefetch(vec![1, 2, 3], 0);
        assert!(head.is_empty());
        assert_eq!(tail, vec![1, 2, 3]);
    }

    #[test]
    fn truncate_chars_handles_cjk_boundary() {
        let input = "三步价格异议处理方法论：先共情、后说价值、最后给方案。";
        let out = truncate_chars(input, 5);
        assert_eq!(out.chars().count(), 6); // 5 chars + 省略号
        assert!(out.ends_with('…'));
    }

    #[test]
    fn wiki_type_priority_orders_thesis_above_source() {
        assert!(wiki_type_priority(Some("thesis")) > wiki_type_priority(Some("source")));
        assert!(wiki_type_priority(Some("methodology")) > wiki_type_priority(Some("entity")));
        assert_eq!(wiki_type_priority(None), wiki_type_priority(Some("entity")));
    }

    /// rank_key 单测用：构造一条最小可用 chunk（只填 rank_key 读到的字段 + 必填）。
    fn rk_chunk(
        title: &str,
        body: &str,
        wiki_type: &str,
        confidence: f64,
        priority: i32,
    ) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: None,
            workspace_id: "w".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_operations".to_string(),
            knowledge_type: None,
            business_context: None,
            title: title.to_string(),
            summary: None,
            body: Some(body.to_string()),
            applicable_scenes: Vec::new(),
            not_applicable_scenes: Vec::new(),
            product_tags: Vec::new(),
            business_topics: Vec::new(),
            source_quote: None,
            source_anchors: Vec::new(),
            integrity_status: Some("verified".to_string()),
            confidence_score: None,
            status: "active".to_string(),
            priority,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
            wiki_type: Some(wiki_type.to_string()),
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: None,
            usage_stats: None,
            dynamic_confidence: Some(confidence),
            integrity_score: None,
            locked_fields: None,
            chunk_type: "product_fact".to_string(),
        }
    }

    #[test]
    fn rank_key_relevance_beats_static_confidence() {
        // 高相关但低静态置信度的 chunk，应排在低相关高置信度 chunk 之前。
        let now = DateTime::now();
        let relevant = rk_chunk("价格异议处理方法论", "先共情后说价值", "entity", 0.1, 0);
        let irrelevant = rk_chunk("产品定价表", "标准版每月99元", "thesis", 0.99, 100);
        let kr = rank_key("价格异议", &relevant, now);
        let ki = rank_key("价格异议", &irrelevant, now);
        assert!(
            kr > ki,
            "high-relevance chunk must outrank high-confidence irrelevant one"
        );
    }

    #[test]
    fn rank_key_superseded_demoted_below_live_peer() {
        // 除 superseded 惩罚外全等的 live 同行，必排在 superseded 之前。
        let now = DateTime::now();
        let live = rk_chunk("价格异议处理", "先共情", "methodology", 0.8, 10);
        let mut superseded = rk_chunk("价格异议处理", "先共情", "methodology", 0.8, 10);
        superseded.superseded_by = Some("newer-chunk-id".to_string());
        let kl = rank_key("价格异议", &live, now);
        let ks = rank_key("价格异议", &superseded, now);
        assert!(kl > ks, "live peer must outrank superseded");
        assert!(kl.live && !ks.live);
    }

    #[test]
    fn rank_key_expired_demoted_below_live_peer() {
        // valid_to < now 的过期 chunk，排在除时效外全等的 live 同行之后。
        let now = DateTime::now();
        let live = rk_chunk("价格异议处理", "先共情", "methodology", 0.8, 10);
        let mut expired = rk_chunk("价格异议处理", "先共情", "methodology", 0.8, 10);
        expired.valid_to = Some(DateTime::from_millis(now.timestamp_millis() - 86_400_000));
        let kl = rank_key("价格异议", &live, now);
        let ke = rank_key("价格异议", &expired, now);
        assert!(kl > ke, "live peer must outrank expired");
        assert!(kl.live && !ke.live);
    }

    #[test]
    fn rank_key_empty_query_falls_back_to_static_order() {
        // query 空 → 有效相关度恒 0，退化为 live 优先 + 静态序（thesis > entity）。
        let now = DateTime::now();
        let thesis = rk_chunk("A", "x", "thesis", 0.5, 0);
        let entity = rk_chunk("B", "y", "entity", 0.5, 0);
        let kt = rank_key("", &thesis, now);
        let ke = rank_key("", &entity, now);
        assert_eq!(kt.effective_relevance_micros, 0);
        assert_eq!(ke.effective_relevance_micros, 0);
        assert!(
            kt > ke,
            "with empty query, thesis outranks entity by static priority"
        );
    }

    #[test]
    fn rank_key_blank_superseded_by_is_still_live() {
        // superseded_by 为空白串不算 superseded（防脏数据误降格）。
        let now = DateTime::now();
        let mut c = rk_chunk("A", "x", "entity", 0.5, 0);
        c.superseded_by = Some("   ".to_string());
        assert!(rank_key("", &c, now).live);
    }

    #[test]
    fn chunk_haystack_includes_product_tags() {
        // product_tags 里的词（如品牌别名）即使不在 title/body，也应进 haystack 并参与打分。
        // 场景：body 不含 "星零感"，只有 product_tags 里有——修复前召不回，修复后能命中。
        let now = DateTime::now();
        let mut with_tag = rk_chunk("去眼袋方案", "微孔技术介绍", "product_fact", 0.5, 0);
        with_tag.product_tags = vec!["星零感".to_string()];
        let without_tag = rk_chunk("去眼袋方案", "微孔技术介绍", "product_fact", 0.5, 0);

        assert!(
            chunk_haystack(&with_tag).contains("星零感"),
            "product_tags 的词必须进 haystack"
        );
        let k_with = rank_key("星零感", &with_tag, now);
        let k_without = rank_key("星零感", &without_tag, now);
        assert!(
            k_with.effective_relevance_micros > k_without.effective_relevance_micros,
            "命中 product_tags 词的 chunk 相关度应高于同结构但无该 tag 的 chunk"
        );
    }

    /// classify_recall_outcome 单测用：构造一条 AnswerResult，trace 里塞
    /// open_chunk / follow_relations 步以驱动 opened 统计。
    fn ar(
        cited: Vec<&str>,
        truncated: bool,
        cancelled: bool,
        opened_chunks: Vec<&str>,
        opened_bodies: Vec<&str>,
    ) -> AnswerResult {
        let mut trace: Vec<Document> = Vec::new();
        if !opened_chunks.is_empty() {
            trace.push(doc! {
                "tool": "open_chunk",
                "opened": opened_chunks.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            });
        }
        if !opened_bodies.is_empty() {
            trace.push(doc! {
                "tool": "follow_relations",
                "openedBodies": opened_bodies.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            });
        }
        AnswerResult {
            answer: String::new(),
            resolution: KnowledgeResolution::default(),
            cited_chunk_ids: cited.iter().map(|s| s.to_string()).collect(),
            source_quotes: Vec::new(),
            tool_trace: trace,
            rounds_used: 1,
            truncated,
            cancelled,
        }
    }

    #[test]
    fn classify_healthy_recall_is_none() {
        // open 了正文且 cite 命中 → 健康，不打扰队列。
        let r = ar(vec!["a", "b"], false, false, vec!["a", "b", "c"], vec![]);
        assert!(classify_recall_outcome(&r).is_none());
    }

    #[test]
    fn classify_empty_catalog_false_negative_is_recall_miss() {
        // 空 catalog 偽阴性：没 open 任何正文、cited=0、未 truncated。
        let r = ar(vec![], false, false, vec![], vec![]);
        let c = classify_recall_outcome(&r).expect("should flag recall_miss");
        assert_eq!(c.kind, "recall_miss");
        assert!(c.affected_chunk_ids.is_empty());
    }

    #[test]
    fn classify_truncated_zero_cite_is_recall_miss() {
        // 4 轮没收敛、强制兜底、cited=0 → recall_miss；affected ⊆ opened。
        let r = ar(vec![], true, false, vec!["a", "b"], vec![]);
        let c = classify_recall_outcome(&r).expect("should flag recall_miss");
        assert_eq!(c.kind, "recall_miss");
        assert_eq!(c.affected_chunk_ids.len(), 2);
        for id in &c.affected_chunk_ids {
            assert!(["a", "b"].contains(&id.as_str()));
        }
    }

    /// B5 用：在 `ar` 的基础上补一条带 `attempted*` 计数的 answer trace，
    /// 模拟「模型给出了引用，但被 filter 全部拒掉」。
    fn ar_with_attempted(
        cited: Vec<&str>,
        truncated: bool,
        opened_chunks: Vec<&str>,
        attempted_cited: i32,
        attempted_quotes: i32,
    ) -> AnswerResult {
        let mut result = ar(cited, truncated, false, opened_chunks, vec![]);
        result.tool_trace.push(doc! {
            "tool": "answer",
            "round": 1i32,
            "citedCount": result.cited_chunk_ids.len() as i32,
            "attemptedCitedCount": attempted_cited,
            "attemptedQuoteCount": attempted_quotes,
        });
        result
    }

    /// B5 红线：模型**试过**引用但全被锚点校验拒掉时，必须报
    /// `citation_format_rejected` 而不是 `recall_miss`——后者会把运营指向「补录知识」，
    /// 而真因是锚点不合规，补录再多也不会让 cited 变非零。
    #[test]
    fn classify_attempted_but_rejected_citations_is_format_rejected() {
        let r = ar_with_attempted(vec![], false, vec!["a", "b"], 2, 2);
        let c = classify_recall_outcome(&r).expect("should flag a signal");
        assert_eq!(
            c.kind, "citation_format_rejected",
            "试过引用却全被拒 → 必须是格式问题，不能报成知识缺失"
        );
        assert_ne!(c.kind, "recall_miss");
        assert!(
            c.description.contains("重锚定"),
            "修复指引应指向重锚定: {}",
            c.description
        );
        // affected 给出待重锚定的候选，运营据此定位。
        assert_eq!(c.affected_chunk_ids.len(), 2);
    }

    /// 对偶：只给了 quote 没给 citedIds（或反之）也算「试过」。
    #[test]
    fn classify_attempted_quotes_only_still_counts_as_attempt() {
        let r = ar_with_attempted(vec![], false, vec!["a"], 0, 3);
        let c = classify_recall_outcome(&r).expect("should flag a signal");
        assert_eq!(c.kind, "citation_format_rejected");
    }

    /// 未尝试引用时仍是 `recall_miss`——本次修复不得把「真的没知识」也改判成格式问题。
    #[test]
    fn classify_zero_attempt_remains_recall_miss() {
        let r = ar_with_attempted(vec![], false, vec!["a"], 0, 0);
        let c = classify_recall_outcome(&r).expect("should flag a signal");
        assert_eq!(c.kind, "recall_miss");
    }

    /// `truncated` 与「查无内容」共用 kind 但 title/description 不同 → dedup_key 不同
    /// → 收件箱分列两行，运营不会再把「探索没收敛」和「真缺知识」混成一条 pending。
    #[test]
    fn truncated_and_empty_recall_miss_have_distinct_dedup_keys() {
        let truncated = classify_recall_outcome(&ar(vec![], true, false, vec!["a"], vec![]))
            .expect("truncated signal");
        let empty = classify_recall_outcome(&ar(vec![], false, false, vec![], vec![]))
            .expect("empty signal");
        assert_eq!(truncated.kind, empty.kind, "共用 kind 保持前端闭集稳定");
        assert_ne!(
            truncated.title, empty.title,
            "两种真因的 title 必须不同，否则 dedup_key 相同会合并成一条"
        );
        assert_ne!(truncated.dedup_key(), empty.dedup_key());
    }

    #[test]
    fn classify_many_open_few_cite_is_low_yield() {
        // open 4 个正文只 cite 1 个 → recall_low_yield；affected = open 但未 cite 的。
        let r = ar(vec!["a"], false, false, vec!["a", "b", "c", "d"], vec![]);
        let c = classify_recall_outcome(&r).expect("should flag low_yield");
        assert_eq!(c.kind, "recall_low_yield");
        // affected 恰是 b/c/d（open 但未 cite），不含已 cite 的 a。
        let mut ids = c.affected_chunk_ids.clone();
        ids.sort();
        assert_eq!(ids, vec!["b".to_string(), "c".to_string(), "d".to_string()]);
    }

    #[test]
    fn classify_low_yield_counts_follow_relations_bodies() {
        // openedBodies（follow_relations 预取正文）也计入 opened 统计。
        let r = ar(vec!["a"], false, false, vec!["a"], vec!["b", "c"]);
        let c = classify_recall_outcome(&r).expect("should flag low_yield");
        assert_eq!(c.kind, "recall_low_yield");
    }

    #[test]
    fn classify_cancelled_is_none() {
        // 用户主动取消不算召回缺陷，即使 cited=0。
        let r = ar(vec![], true, true, vec![], vec![]);
        assert!(classify_recall_outcome(&r).is_none());
    }

    #[test]
    fn merge_catalog_dedups_by_chunk_id() {
        let mut base = vec![CatalogEntry {
            chunk_id: "a".to_string(),
            wiki_type: "entity".to_string(),
            chunk_type: "product_fact".to_string(),
            title: String::new(),
            summary: String::new(),
            business_topics: Vec::new(),
            verified: false,
            has_source_quote: false,
            dynamic_confidence: 0.0,
            related_count: 0,
        }];
        let incoming = vec![
            CatalogEntry {
                chunk_id: "a".to_string(),
                wiki_type: "entity".to_string(),
                chunk_type: "product_fact".to_string(),
                title: String::new(),
                summary: String::new(),
                business_topics: Vec::new(),
                verified: false,
                has_source_quote: false,
                dynamic_confidence: 0.0,
                related_count: 0,
            },
            CatalogEntry {
                chunk_id: "b".to_string(),
                wiki_type: "entity".to_string(),
                chunk_type: "product_fact".to_string(),
                title: String::new(),
                summary: String::new(),
                business_topics: Vec::new(),
                verified: false,
                has_source_quote: false,
                dynamic_confidence: 0.0,
                related_count: 0,
            },
        ];
        merge_catalog(&mut base, incoming);
        assert_eq!(base.len(), 2);
        assert_eq!(base[0].chunk_id, "a");
        assert_eq!(base[1].chunk_id, "b");
    }

    /// `Answer` action 的 cited_chunk_ids 子集断言由 PBT 覆盖；这里只验证
    /// 最朴素的"action 反序列化"路径。
    #[test]
    fn parse_answer_action_with_camel_alias() {
        let raw: serde_json::Value = serde_json::from_str(
            r#"{"action":"answer","citedChunkIds":["c1"],"sourceQuotes":[{"chunkId":"c1","quote":"q","sourceAnchorIndex":0}],"answer":"hello","resolution":{"answerability":"unsupported","requiredAuthority":"authorized_operator","recommendedNextStep":"ask_principal","missingInformation":["current state"],"authorityQuestion":"What is the current state?","unresolvedProposition":"Whether the requested action is currently authorized"}}"#,
        )
        .unwrap();
        let action: AgentAction = serde_json::from_value(raw).unwrap();
        match action {
            AgentAction::Answer {
                cited_chunk_ids,
                source_quotes,
                answer,
                resolution,
            } => {
                assert_eq!(cited_chunk_ids, vec!["c1".to_string()]);
                assert_eq!(source_quotes.len(), 1);
                assert_eq!(answer, "hello");
                assert_eq!(
                    resolution.recommended_next_step,
                    crate::agent::types::KnowledgeNextStep::AskPrincipal
                );
                assert_eq!(
                    resolution.required_authority,
                    crate::agent::types::KnowledgeRequiredAuthority::AuthorizedOperator
                );
                assert_eq!(
                    resolution.unresolved_proposition,
                    "Whether the requested action is currently authorized"
                );
            }
            _ => panic!("expected answer"),
        }
    }

    /// 把一组原始片段顺序喂入 [`AnswerStreamer`]，返回拼接出的全部解码正文。
    fn drive_streamer(frags: &[&str]) -> String {
        let mut s = AnswerStreamer::default();
        let mut out = String::new();
        for f in frags {
            out.push_str(&s.push(f));
        }
        out
    }

    #[test]
    fn streamer_extracts_answer_from_single_chunk() {
        let raw = r#"{"action":"answer","citedChunkIds":["c1"],"answer":"你好世界"}"#;
        assert_eq!(drive_streamer(&[raw]), "你好世界");
    }

    #[test]
    fn streamer_handles_cjk_split_across_fragments() {
        // 多字节 char 被切在两段之间：不能丢字、不能 panic。
        let frags = ["{\"action\":\"answer\",\"answer\":\"你", "好", "世界\"}"];
        assert_eq!(drive_streamer(&frags), "你好世界");
    }

    #[test]
    fn streamer_decodes_escapes() {
        // \" 转义引号、\n 换行、\\ 反斜杠，都应被解码成人类可读正文。
        let raw = r#"{"answer":"行1\n说\"价值\"\\结束"}"#;
        assert_eq!(drive_streamer(&[raw]), "行1\n说\"价值\"\\结束");
    }

    #[test]
    fn streamer_stops_at_closing_quote() {
        // answer 值闭合后，后续 JSON 字段不应被当成正文。
        let raw = r#"{"answer":"前半","truncated":false}"#;
        assert_eq!(drive_streamer(&[raw]), "前半");
    }

    #[test]
    fn streamer_emits_nothing_for_tool_round() {
        // 工具轮没有顶层 answer 字段 → 不产生任何 token。
        let raw = r#"{"action":"open_chunk","ids":["c1","c2"]}"#;
        assert_eq!(drive_streamer(&[raw]), "");
    }

    #[test]
    fn streamer_handles_escape_split_across_fragments() {
        // 转义序列 `\n` 被切在两段之间：半个 `\` 应留待下次，不能误输出。
        let frags = [r#"{"answer":"行1\"#, r#"n行2"}"#];
        assert_eq!(drive_streamer(&frags), "行1\n行2");
    }

    #[test]
    fn streamer_handles_unicode_escape() {
        // JSON 你好 应解码成「你好」。用普通串嵌入字面反斜杠。
        let raw = "{\"answer\":\"\\u4f60\\u597d!\"}";
        assert_eq!(drive_streamer(&[raw]), "你好!");
    }
}
