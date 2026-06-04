# 召回率基准测试（recall@k）设计 v2——跨行业大语料矩阵 + 召回率②与优化后保持③

> 设计状态：v2 按用户 2026-06-05 明确要求升级（跨行业大语料矩阵做通用性测试；范围扩到「召回率②+优化后保持③」）。待用户复核后转 writing-plans。
> 实施时机：与质量循环（Q1-Q8）解耦——新增独立 `real-llm-recall` CI job，不进 quality matrix，不抢收敛判据。

## 背景与业务场景（用户反复强调，勿脱离）

用户业务闭环：**文档→抽取入库→运营 agent 召回知识回答→agent 自然语言维护知识库→缺知识反馈对话补全**。

三个功能级北极星：
1. **提取的知识召回率高且稳定**（可优化多种召回方案）。
2. agent 用**自然语言维护**知识库（**保持召回率/稳定性**）。
3. 运营 agent **找到知识库不足 → 自然语言完善**（缺知识闭环）。

现状（已读代码核实）：
- ③缺知识闭环：已落地，被 Q8 + 强化 K3 守（recall_miss gap 信号携带 query+追问）。
- ②自然语言维护：红线内做到「提议级」（draft+needs_review，AI 从不自动 verify）。
- ①**召回率本身没有任何量化指标在守**——最大测量缺口。Q1-Q8 测「答得好不好」(judge 打分)、K 套件测「cite⊆verified」，**都不是 recall@k**。

→ 本设计补①，并覆盖②的「**维护后召回率保持稳定**」这一可量化部分。

## 现状事实底座（已核实）

| 事实 | 出处 |
|---|---|
| 召回是**单一词法管线**：agent-first LLM 渐进披露（list_catalog→open_chunk→follow_relations→answer），排序靠 relevance_score()（bigram+token 覆盖，纯词法、CJK 友好）。无 embedding/向量/BM25 | knowledge_agent.rs relevance_score；knowledge_router.rs route_operation_knowledge |
| 召回链路**只召 `integrity_status="verified" + status="active"`** 的 chunk | knowledge_router.rs:70-71 |
| 触发：`AnswerRequest{workspace_id,account_id,query,filter,max_rounds}` → `answer(&state,req)` → `AnswerResult{cited_chunk_ids, tool_trace(每步含 opened 数组), rounds_used}` | knowledge_agent.rs |
| seed：`seed_chunk(app,ws,title,summary,body,integrity_status,status,dynamic_confidence,related)→hex id`；`seed_verified(...)`=verified+active 便捷封装 | real_llm_knowledge.rs:140/:185 |
| agent 自然语言维护入口 chat_turn 产出恒 draft+needs_review（不进召回，直到审定） | routes/knowledge.rs |

## 防过拟合 / 防作弊（用户红线）

**最易作弊点：ground-truth relevant 集**。若按「现有词法召回能召回的」标 relevant，基准变自证循环——永远虚高，毫无意义。

防御：
1. **预声明，不倒推**：每条 query 的 expected_chunk_ids 在 seed 时由语料构造意图钉死（本设计用 title→id 映射在 seed 后填入，但 title 关联是构造时人工声明的，非跑召回回填）。
2. **强制含词法对抗样本**：query 与目标 chunk 字面不重叠（同义/改写/概念相关）。如「买错了能不能换货」↔「退换货政策」、「服务挂了怎么赔偿」↔「SLA 服务保障」。纯词法盲区，如实暴露真实上限。
3. **诚实目标**：纯词法召回在字面不重叠改写上几乎不可能 100%。基准是**如实测上限+给改进标尺**，第一轮**不设硬 floor**（没 baseline 前不乱设阈值）。
4. **跨行业多类型**：覆盖 6 行业 × 多文档类型，不针对单一题材/历史样本。

## 范围（用户已拍板：跨行业大语料矩阵 + 召回率②+优化后保持③）

**本轮做：**
- **跨行业大语料矩阵**：6 行业（零售/SaaS/金融/教育/医疗/制造）× 多文档类型（规格/报价/合同条款/手册/FAQ/案例），约 18-30 条 verified chunk + 18-30 条 query，每行业含 lexical-easy 与 adversarial 两类 query。
- **召回率②**：recall@k（k=5/10），分 lexical-easy / adversarial 两组报告。
- **优化后保持③**：KB 内容变更后召回率不退化（详见下）。
- **稳定性**：每条 query 跑 N 次（≥3），报召回集抖动/方差。

**本轮不做（留后续）：** MRR/NDCG/precision@k；语义/向量召回**替代方案的实现与 A/B**（要新增生产召回代码）；提 floor / 断 TARGET。

## 设计

### 文件结构
- **新建** `tests/real_llm_recall_benchmark.rs`（不在禁区清单）。
- **不碰生产代码**——纯测量，复用 `answer()` + `seed_chunk/seed_verified` 模式。
- CI：新增 `real-llm-recall` job（仿 real-llm-knowledge，coderelay gpt-5.5 主链 + NVIDIA failover），独立于 quality matrix。

### 数据结构（整合 Plan agent 方案）
```rust
struct RecallCase { name: String, query: &'static str, expected_chunk_ids: Vec<String>, lexical_overlap: f64, adversarial: bool }
struct IndustryCorpus { industry: &'static str, doc_type: &'static str, chunks: Vec<ChunkSeed>, queries: Vec<QueryCase> }
struct ChunkSeed { title: &'static str, summary: &'static str, body: &'static str }
struct QueryCase { query: &'static str, expected_titles: Vec<&'static str> }   // adversarial 由客观词法重叠度量推导，不再人工标
// 两层召回分开：触达(reach)=agent 翻到过；采纳(adopt)=最终引用作答。
struct RecallResult { case_name, query, expected_count,
    reach_recall: f64, adopt_recall: f64,   // 分开报：reach=opened∪cited∩exp/exp；adopt=cited∩exp/exp
    reach_set, adopt_set, missing_reach, missing_adopt,
    rounds_used, stable_across_runs, run_variances }
```
- `build_industry_corpus_matrix() -> Vec<IndustryCorpus>`：6 行业语料（内容如 Plan agent 草案，每包 3 chunk + 3 query）。
- seed 后用 `title → hex_id` map 把 QueryCase.expected_titles 解析成 RecallCase.expected_chunk_ids（构造意图声明，非倒推）。
- **adversarial 客观推导（改进 5，去主观）**：`lexical_overlap` = query 与 expected chunk body 的 bigram 重叠率（确定性纯函数）；`adversarial = lexical_overlap < 阈值(如 0.15)`。easy/adversarial 分组由此客观划分，不靠人工拍 bool——避免「把其实词法能命中的标成 adversarial 刷虚假分」。

### 指标计算（确定性核心，触达/采纳两层分开——改进 1）
```rust
fn reach_set(result: &AnswerResult) -> Vec<String>   // tool_trace 各步 opened 并集 ∪ cited_chunk_ids —— agent「翻到过」
fn adopt_set(result: &AnswerResult) -> Vec<String>   // 仅 cited_chunk_ids —— agent「真正采纳作答」
fn recall_at_k(set: &[String], expected: &[String]) -> f64  // |set∩exp|/|exp|；exp 空→空记1.0、有噪声记0.0
fn bigram_overlap(query: &str, body: &str) -> f64    // 客观词法重叠度量，给 adversarial 分类用
```
**为何分两层**：reach（触达）测检索层——该召回的知识 agent 检索翻到了吗；adopt（采纳）测生成层——翻到了有没有用上。两者差值暴露「翻到却没采纳」这一独立失败模式。混成一个数会高估召回、且分不清失败在检索还是生成。**reach ≥ adopt 恒成立**（采纳必先触达），差值是诊断信号。
分组聚合：overall / lexical-easy / adversarial 各报 **reach_recall 与 adopt_recall 两个均值** + 方差。

### 「k」的操作定义（改进 2）
agent 渐进披露无固定 top-k 列表，故 k **映射到 agent 检索预算**：固定 `max_rounds`（如 4）+ 默认每轮 open 预算，recall@k 即「在该预算内触达/采纳了多少 expected」。报告标注实际 `rounds_used` 与触达 chunk 数，使 k 语义可解释（而非凭空写 5/10）。

### ⓪ 跨轮稳定性基线先行（改进 4，核心产出，③ 的前提）
**为何先做**：有两种稳定性，必须分清依赖——
- **跨轮稳定性**：同一 query、知识库不变，跑 N 次召回集是否一致（测 agent LLM 决策 / rerank 方差）。
- **跨变更稳定性**（③）：知识库变更后未涉及 query 的召回是否漂移。

若**跨轮本身就不稳**（N 次召回集都飘），③ 的 R1 vs R0 比较失去意义——分不清召回变了是因为改库还是 LLM 抽风。故**必须先测出跨轮稳定性基线**，③ 才可解释。

而且这恰是用户最该先知道的体检：**当前单一词法管线 + LLM agent 决策，跨轮到底稳不稳？** 若同一 query 跑 N 次召回集都不同，「召回稳定」这个北极星本身亮红灯。

操作：matrix 每条 query 跑 N 次（N 见下，统计功效要够），报每条的：
- reach_set / adopt_set 跨轮是否完全一致（bool）；
- reach_recall / adopt_recall 跨轮极差与方差；
- 全 matrix 的「跨轮完全稳定 query 占比」——**这是第一份核心产出数字**。

### ③ 优化后保持（KB 变更后召回不退化）——本轮新增，**测真实 agent chat 改库全链路**
**用户决策（2026-06-05）**：③ 测**真实 agent chat 改库全链路**（含 LLM 方差），不用确定性简化——因为「稳定为第一要务」，真实全链路才能暴露真实稳定性。忠于业务语义：agent 通过对话提议 draft → 运营审定 verified → 重测召回。**前提：⓪ 跨轮稳定性已量化**（否则 R1 vs R0 不可解释）。

覆盖三类维护操作（改进 3——不只新增，补最易破坏稳定的改写/废弃）：
1. **baseline**：跑全语料矩阵，记录每条 query 的 reach/adopt 召回集 R0[q]。
2. **变更 A·新增**：用 `chat_turn`（routes/knowledge.rs:4227）发「帮我新建一条关于 X 的知识切片，正文……」→ agent 真实走 LLM 产 draft（intent=create_chunk，恒 draft+needs_review）→ 取 id 调 `verify_operation_knowledge_chunk`（:569）审定 verified。针对性 query 应召回到新知识。
3. **变更 B·改写**（最易破坏召回稳定）：用 chat_turn 发「更新某条切片，补充同义表述 Y」→ agent 产 update draft → 审定。重测：该 chunk 对应 query 召回保持/提升。
4. **变更 C·废弃**（召回失稳高危）：把某条 verified chunk 置为 archived/needs_review（模拟运营废弃 agent 提议的过期知识）。重测：原召回它的 query 召回失败属预期；**关键看其余 query 不受牵连**。
5. **每次变更后重测全 matrix**，比对 R1[q] vs R0[q]：
   - 涉及变更的 query：recall 按预期升/降。
   - **未涉及 query：R1 应稳定等于 R0——稳定性第一要务**。跑 N 次量化抖动。
6. **稳定性断言**：未涉及变更的 query R1==R0。第一轮**量化记录 + soft-warn**（先如实测稳定性现状，没 baseline 前不硬断，避免反向过拟合到某抖动阈值）。
7. transient：chat_turn/answer/verify 上游瞬时不可达 → `unwrap_or_skip_transient!` skip 不 panic。

### 稳定性维度（改进 7：N 要够统计功效）
N 跑次数 = env RECALL_STABILITY_RUNS，**默认 5**（不是 3——若抖动率 20%，3 次有 51% 概率全同→误判稳定；5 次降到 33%，更可信但仍是粗观测，报告标注这是观测非严格统计）。记录每次 reach/adopt 召回集，报：是否每次一致（bool）+ recall 跨轮极差/方差。

### 报告
`eprintln!` ledger（仿 quality job），输出：
- **⓪ 跨轮稳定性**：全 matrix「跨轮完全稳定 query 占比」+ 每条 reach/adopt 召回集是否每轮一致（**第一份核心产出**）。
- **② 召回率**：每 case 的 reach_recall / adopt_recall，按 overall / lexical-easy / adversarial 分组；reach−adopt 差值（翻到却没采纳的诊断）。
- **③ 维护后保持**：每类变更（新增/改写/废弃）后，涉及 query 的 recall 升降 + 未涉及 query 的 R1 vs R0 漂移率。
第一轮**只观测 baseline 不 panic**，除唯一硬红线（见下）。

### 错误处理 / 红线
- 上游 429/瞬时不可用：复用 `unwrap_or_skip_transient!`，skip 不 panic。
- **召回集 ⊆ seed 硬断**：出现 seed 外 id = 真 bug，立即 fail（cite⊆verified 在召回层的等价红线）。
- expected 空的弃答对照：召回到噪声记 0，记录但本轮不 fail。
- ③ 的 R1≠R0（未涉及 query 召回漂移）：本轮 soft-warn 记录，不 fail（先量化）。
- 跨轮不稳（⓪ 召回集每轮飘）：soft-warn 记录，不 fail——这本身是要测的体检结论。

### 工程细节（改进 8）
- **title 全局唯一**：跨行业 chunk title 必须全局唯一，否则 title→id 映射冲突致 expected 错配——构造矩阵时校验去重。
- **②③ 隔离**：③ 改库会污染 ② 的 baseline，故 ② 与 ③ 用**独立 workspace_id**（或严格顺序：② 全跑完记 baseline 后才进 ③），互不串扰。
- **CI 墙时间**：matrix(~20 query)×N(5)×(⓪+②+③多变更) 真实 LLM 调用量大，45min 可能不够——首版可用 env 收窄 matrix 子集或调小 N 跑通，再逐步放开；job 内串行避免 429 风暴。

### CI
新增 `real-llm-recall` job：coderelay gpt-5.5 主链（REAL_LLM_CODERELAY_API_KEY），continue-on-error:true，45min。**按测试拆 matrix 并行**（smoke / cross_industry / maintenance 三测试各享独立 runner + 独立 45min 墙，fail-fast:false，max-parallel:2 限并发防瞬时 429），仿 real-llm-ops / real-llm-quality 形态——比单 job 抗墙腰斩、失败隔离。每行 ledger upload `real-llm-recall-ledger-<test>`。首版 `RECALL_STABILITY_RUNS=3`（spec 定稿 5，矩阵调用量大先 3 跑通再放开）。**不进 quality q: matrix**。
> 实施调整：首版**未接 NVIDIA failover**——召回是单端点串行、429 风险低，且 `real_llm_from_env()` 当前不消费 failover env，避免死配置；后续若实测撞限流再加（同 spec 的渐进思路）。

## 验证（实施后）
1. `CARGO_TARGET_DIR=target-check cargo check --tests` 过（新测试文件编译）。
2. `cargo test --lib` ≥350/0 + 4 PBT 累计 ≥33/0 不回归（不碰生产代码，预期零影响）。
3. push → 看 `real-llm-recall` job ledger：⓪ 跨轮稳定占比 + ② reach/adopt 分组 recall + ③ 三类变更 R0/R1 对比；确认召回集⊆seed 红线无违反。
4. **反过拟合自检**：expected 集 title 关联构造时声明（非倒推）；adversarial 由**客观 bigram 重叠度量**划分（非人工拍 bool）；6 行业多类型；无硬 floor；触达/采纳分两层不混。

## Rollback
纯增量：1 个新测试文件 + 1 个新 CI job。无生产改动。删文件+删 job 即回滚。

## Out of scope
- 语义/向量召回替代方案的实现与 A/B（要新增生产召回代码，留后续轮）。
- MRR/NDCG/precision@k；提 floor / 断 TARGET。
- 碰任何禁区生产文件 / 其它 agent 文件 / 前端。
