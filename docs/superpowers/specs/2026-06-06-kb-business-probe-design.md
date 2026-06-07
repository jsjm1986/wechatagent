# 知识库业务仿真探针（KB Business Probe）设计

> 状态：设计已批准（2026-06-06），待写实施计划。
> 定位：在现有 real-LLM CI 基建之上，新增一条**从真实业务起点（运营丢文档）到连续运行召回趋势**的端到端仿真探针。**全新独立测试，零改动现有任何资产。**

## 1. 背景与动机

### 1.1 要解决什么

现有真实 LLM 知识库测试是**断裂的两段**，中间从未接通：

- **K5/K6**（`tests/real_llm_knowledge.rs:670/737`）只验证文档抽取的**第一跳**：`import_operation_knowledge_preview` / `apply_image` 抽出 chunk、恒 `draft+needs_review`、AI 永不自动 verify。**断言到此为止**——抽出来就结束，不往后走召回。
- **召回基准 / 闭环轨迹**（`tests/real_llm_recall_benchmark.rs`）从**已 seed 好的 chunk**（`seed_verified`）或**对话补库**（`chat_create_and_verify`）起步。**没有一个从"运营丢文档给 agent"这个真实入口进来。**

业务真正的第一步——**文档 → 知识 agent 抽取 → 入库 → verify → 召回**——这条端到端链，当前**没有任何测试完整走通**。K5 抽完就断，召回基准凭空 seed。这是探针要填的断口。

更根本的局限：所有现有真模型测试都是**单局隔离**（建库→跑一遍→拆掉）。但"正式业务运行"的本质是**时间维度上的累积**——库越长越大、知识互相冲突/过期、query 分布漂移、召回质量可能随规模退化。这些长程问题，单局隔离永远压不出。

### 1.2 北极星链路

探针对齐产品核心价值闭环（verbatim）：

> 文档/对话 → 抽取入库 → 运营 agent 召回 → agent 自然语言维护知识库 → 缺知识反馈对话补全 → 同 query 再命中

闭环单测 `recall_benchmark_gap_closed_loop_trajectory` 已在 CI（run 27027291507）端到端跑通这条链的**共享域 + 生产路径**。探针在它之上加两件事：**（a）从文档入库这个真实起点起步；（b）连续运行 N 轮观测累积态趋势。**

## 2. 四个已锁定的设计决策

| # | 变量 | 决策 | 理由 |
|---|---|---|---|
| D1 | 语料/query 来源 | **LLM 动态生成**（非硬编码静态矩阵） | 模拟真实运营：库会生长、query 会到来；每次新随机种子 = 反过拟合 |
| D2 | 判据 | **双轨**：硬轨（gold-standard 命中/越界，确定性）+ 软轨（judge LLM 评 answer 质量） | 硬轨守红线，软轨给观测信号；两轨分离不混合 |
| D3 | 每"运行批"覆盖 | **完整六阶段业务弧**（文档入库→召回→改后召回→人机交互补全→闭环→累积） | 端到端，覆盖真实业务全环节 |
| D4 | 时间/累积模型 | **分层**：单局隔离做回归门（已存在，不动）；跨局累积做探针（本轮新建） | 各取所长，不互相污染 |
| D5 | 运行结果→优化的闭合方式 | **探针只产报告，人审后才改**（B 起步，留 C 自动提案接口） | 把反过拟合红线焊死在人这一关 |
| D6 | 落地范围 | **C 方案**：一期交付模块化探针主干（复用现有 CI）；持久化重基建推迟到报告价值验证后 | 先验证报告质量能否支撑人审决策，模块化保 B 期零返工 |
| D7 | 运行环境 | **本地 mongod + 本地 LLM key**，绕开 Docker/testcontainers | 本机有 mongod；探针全套含真模型端到端弧本地可跑，工作流不劈半 |

## 3. 架构总览

### 3.1 两层定位

```
┌─ 回归门（已存在，不动）──────────────────────────────┐
│  recall_benchmark_gap_closed_loop_trajectory 等        │
│  单局隔离 · 确定性 · 守北极星不退 · 红线破则 fail        │
└────────────────────────────────────────────────────┘
┌─ 探针（本轮新建）────────────────────────────────────┐
│  tests/real_llm_kb_probe.rs + tests/kb_probe/*         │
│  N 轮累积循环 · 动态生成 · 只产报告不改代码              │
│  ↑ 复用回归门已验证的闭环 helper（思路/接口）           │
└────────────────────────────────────────────────────┘
```

回归门和探针角色彻底分开：**回归门守不退（红线破即 fail）；探针只观测（永不因软轨低分 fail，只有硬红线才 fail）。** 探针的产出是**趋势报告 artifact**，不是绿叉守门。

### 3.2 进程内"伪累积"如何模拟连续运行

一期外壳是单个 `#[ignore]` test 起一个干净 workspace，跑一个 N 轮循环：

```
轮0: 动态生成器产 M 条铺底语料 → 走入口A 抽取入库（draft→verify）
轮1..N:
  ├ 生成器产 1 条新 query（落在已有知识 or 故意制造 gap）
  ├ 召回 → 双轨判据打分（硬轨 reach/adopt/越界 + 软轨 judge）
  ├ 若 miss → 对话补库（chat_create_and_verify）→ 库 +1 切片
  └ 记一行 ledger{轮次, 库规模, 命中, 越界, judge分, 是否补库}
跑完: N 行 ledger 聚合成趋势报告 → 写 artifact + 渲染人审摘要
```

库在一个进程里从 M 条长到 M+若干条，**召回打的是越来越大的库**——这是单局隔离压不出的"中程累积"信号（召回率随规模退化？越界随规模上升？）。

### 3.3 硬约束继承（全部不可破）

- **MCP 永远是桩**（绝不真发微信）。
- **D2 verify 红线不削弱**：探针走和回归门同一个 `chat_create_and_verify`，verify 仍需 sourceQuote+source_anchors 双非空；AI 永不自动 verify。
- **反过拟合**：探针只产报告不自动改，优化动作走人审闸（见 §6）。
- **召回集 ⊆ 库内 id** 是硬红线（出现库外 id 立即标 RED-LINE）。
- **现有资产零改动**：K 套件、召回基准、闭环轨迹一律不动；探针是并列的全新测试。
- **no-human-takeover lint** 全程守住（AI-autonomous 定位，无"人工接管/takeover/hand-off"）。

## 4. 六阶段业务弧（探针每运行批的骨架）

区分两种真实入口，探针都覆盖：

**入口 A — 文档批量入库（业务第一步，运营把资料丢给 agent）**
```
运营资料文本 → import_operation_knowledge_preview（agent 抽取，draft+needs_review）
            → 落库 → verify（显式人工确认，过 D2 闸）→ 进可召回库
```
这是 K5 抽完就断那一跳的**完整延伸**——抽取不再是终点，一路走到"可被召回"。

**入口 B — 对话式增量维护（运营在聊天里补/改知识）**
```
运营对话陈述 → chat_turn（agent 起草 proposal）→ chat_apply
            ├ 新增逻辑：create_chunk（溯源=运营陈述，回归门已验证）
            └ 修改逻辑：update/deprecate 已有切片
            → verify → 进可召回库
```

**每运行批 = 一条完整业务弧**：

| 阶段 | 动作 | 复用的已验证 helper（思路/接口） |
|---|---|---|
| ① 文档入库 | 生成器产运营资料 → 入口A 抽取入库+verify → 铺底库（M 条） | `import_operation_knowledge_preview`（K5 只验抽取，探针补到 verify+召回） |
| ② 召回基准 | 生成器产 query → 召回 → 双轨判据 | `answer_reach_adopt` / `reach_set` / `adopt_set` |
| ③ 改后召回 | 改写/废弃某切片 → 同 query 再召回（改对了召回跟着变；废弃的不再被召回） | `recall_benchmark_maintenance_stability` 的改库链路 |
| ④ 人机交互补全 | query 命中 gap → agent 主动提问 → 运营对话补充（修改/新增）→ verify | `chat_create_and_verify` |
| ⑤ 闭环验证 | 同 query 再召回 → 命中新/改的知识 → 记 ledger | 同 ② |
| ⑥ 累积循环 | 回 ② 打新 query，库已增大 → 观测召回随规模趋势 | — |

阶段③不必每批都改库（模拟真实运营节奏，作为可触发支路）；阶段①每批必走（这是探针填断口的核心）。

## 5. 模块边界划分（C 方案"B 期零返工"的命门）

核心原则：**"探针逻辑"与"运行外壳"彻底分离**。一期外壳是单进程 N 轮循环；二期外壳换成持久化跨批次。只要探针逻辑（生成、判据、ledger、弧）不知道自己跑在哪种外壳上，二期就零返工。

```
tests/
├ real_llm_kb_probe.rs        ← 一期外壳：N 轮循环 driver（薄，~200 行）
└ kb_probe/
   ├ mod.rs                    ← 模块导出 + 共享类型
   ├ generator.rs              ← 【模块1】动态生成器
   ├ judge.rs                  ← 【模块2】双轨判据
   ├ ledger.rs                 ← 【模块3】结构化报告
   └ arc.rs                    ← 【模块4】六阶段业务弧（单批执行单元）
```

> 注：Rust 集成测试支持 `tests/<name>/mod.rs` 共享模块；`tests/kb_probe/` 下文件不会被当作独立 test target，只作为 `real_llm_kb_probe.rs` 的子模块引入（`mod kb_probe;`）。

### 模块1 — generator.rs（动态生成器）

```
gen_corpus_doc(llm, industry, seed)       -> 运营资料文本    // 喂入口A
gen_query(llm, known_topics, force_gap)   -> Query           // force_gap 制造召回缺口
gen_maintenance_intent(llm, chunk)        -> 改写/废弃陈述    // 喂阶段③
gen_supplement_dialogue(llm, gap_query)   -> 运营补充陈述     // 喂阶段④新增/修改
```

- 只依赖 llm client + 随机种子，**不依赖外壳**。一期二期同一个生成器。
- 反过拟合：每次运行用新随机种子，产新语料/新 query，绝不固定样本。

### 模块2 — judge.rs（双轨判据）

```
struct DualVerdict {
  // 硬轨（确定性，无 LLM）
  reach_hit: bool, adopt_hit: bool, out_of_bounds: Vec<String>, verify_gate_held: bool,
  // 软轨（judge LLM）
  judge_score: f64, judge_reason: String,
}
fn hard_verdict(result, expected_id, all_lib_ids) -> 硬轨   // 纯函数，复用 reach_set/adopt_set 思路
async fn soft_verdict(judge_llm, query, answer)   -> 软轨   // judge 打分
fn combine(hard, soft) -> DualVerdict
```

- **硬轨纯函数**（无 IO），一期二期完全一致、可单测。
- 软轨 judge 用**异族模型**（`deepseek-3.2`，与被测主链 claude 家族不同源），防同源偏见——见 [[reference_methodology_authority]] 的"judge 换异族"杠杆。本地端点（kiro-api 网关，`127.0.0.1:5580`）同时提供 claude/deepseek/glm/qwen/minimax 五个家族，故 judge 可真异族，比"单 provider 同源"更硬。

### 模块3 — ledger.rs（结构化报告）

```
struct RoundRecord { round, lib_size, stage, reach_hit, adopt_hit, out_of_bounds, judge_score, did_maintain, did_supplement }
struct ProbeReport { batch_id, rounds: Vec<RoundRecord>, trends: Trends }
struct Trends { recall_rate_curve, oob_rate_curve, judge_score_curve }  // 随 lib_size 的退化曲线
fn write_report(report) -> target/kb_probe_ledger/<batch>.json   // 喂 artifact
fn render_summary(report) -> String                              // 人审用的可读短板报告
```

- **纯数据结构 + 序列化**，不知道数据来自单进程还是跨批次。**这是 B 期复用的核心**：二期只把多批 report 串成时间序列，schema 不变。

### 模块4 — arc.rs（六阶段业务弧 = 单批执行单元）

```
async fn run_one_arc(ctx: &ArcContext, gen, judge) -> Vec<RoundRecord>
// ctx 提供: llm, judge_llm, app_state, workspace, 已有库句柄
// 内部跑①→⑥，每阶段调 generator + 复用回归门 helper + judge 打分 + 攒 RoundRecord
```

- **关键抽象**：`run_one_arc` 接收 `ArcContext`（库句柄 + client），跑完一条弧。本身不关心库是内存还是持久化——**这就是零返工的支点**。
  - 一期外壳：循环 N 次调 `run_one_arc`，复用同一 workspace 库（库累积在本地 mongod）。
  - 二期外壳：每次 cron 调一次 `run_one_arc`，`ArcContext` 的库句柄指向持久化恢复的库。

### 依赖方向（单向，无环）

```
real_llm_kb_probe.rs (外壳) → arc.rs → generator.rs + judge.rs + ledger.rs
                                     ↘ 复用 recall_benchmark 的 helper 思路（reach_set 等）
```

### B 期零返工验证

二期要做的全部 = 写一个新外壳（restore 库→调 `run_one_arc`→snapshot 库）+ ledger 加一个"跨批聚合"函数。模块 1/2/4 **一行不改**，模块 3 只 append 一个聚合 fn。

## 6. 双轨判据 + 反过拟合边界

### 6.1 双轨判据

**硬轨（确定性，无 LLM，不可放水）**——每轮召回后算：

| 指标 | 定义 | 性质 |
|---|---|---|
| `reach_hit` | 期望切片 ∈ reach 集（opened∪cited） | 召回触达 |
| `adopt_hit` | 期望切片 ∈ adopt 集（cited） | 召回采纳 |
| `out_of_bounds` | 召回集里出现库外 id | **RED-LINE**，出现即标红 |
| `verify_gate_held` | 入库/补库切片是否真过了 D2（sourceQuote+anchors 双非空） | 红线守门 |

硬轨是纯函数 + 客观集合运算，judge 永远碰不到它。越界和 verify 闸是红线，任何一轮破了直接在报告里标 `RED-LINE`。

**软轨（judge LLM，只评质量不碰红线）**——judge 对每轮 answer 打分：
- `answer_quality`（准确/相关/无幻觉）
- `judge_reason`（为什么这个分，供人审定位）

软轨只产观测信号，**不参与任何 pass/fail 判定**。判据冲突时硬轨优先。

**关键纪律**：判据本身不决定"测试红绿"。探针是观测仪不是回归门——**永不因软轨低分 panic，只有硬红线（越界 / verify 闸破）才 fail。**

### 6.2 反过拟合三道焊缝（B 方案命门）

红线：只能沉淀可复现的抽象方法论，绝不对单条样本点对点修补 = 作弊。

**焊缝1 — 报告只给"失败模式分类 + 趋势"，不给"具体样本怎么改"**
```
[短板] adopt_hit 率随 lib_size 上升而下降（20条→62%，45条→41%）
       → 疑似：库膨胀后 judge agent 检索排序退化
       复现样本: batch_3f2a round 12,17,23（附 query+召回集，供人复核）
[红线] 0 次越界，0 次 verify 闸破（红线全程守住）
```
报告描述现象 + 指根因层 + 附复现样本，但**不写"把切片 X 的 anchor 改成 Y"这种点对点修法**。

**焊缝2 — 复现样本是"证据"不是"靶子"**
附的 round 样本作用是让人复核"真短板还是偶发噪声"，不是让我针对这几条修。判断标准写进报告：**短板必须在多个随机种子/多轮稳定复现才标"疑似真短板"**，单次偶发标"噪声待观察"。

**焊缝3 — 优化动作走人审闸**
报告产出 → 用户看 → 确认"抽象短板" → 授权 → 做**通用方法论修正**，且修正必须能在**新随机种子的探针运行**上复现改善，不是让历史样本变绿。

**反过拟合自检清单**（每次基于报告做优化前必过）：
1. 这个改动是通用原则还是针对报告里那几条样本？
2. judge truth/阈值有没有被动过去迎合输出？（不许）
3. 改完能否在全新随机种子的运行上复现改善？（必须能）

## 7. 运行环境与执行工作流

### 7.1 运行环境（D7：本地 mongod，绕开 Docker）

`TestApp::start()`（`tests/common/mod.rs:124`）用 testcontainers 起 Mongo 容器，依赖 Docker。但其装配里**只有 :131-137 依赖容器**（起 Mongo 拿 host/port）；从 `Database::connect(&uri, ...)`（:140）往后——迁移、索引、prompt pack、AppState 组装——全与容器无关。

因此 B 出路（无 Docker 底座）的全部基建 = **一个新构造函数**（纯增量，零碰生产代码/现有 TestApp/现有测试）：

```rust
// tests/common/mod.rs 新增
pub async fn start_on_local_mongod() -> Self {
    let uri = std::env::var("KB_PROBE_MONGO_URI")
        .unwrap_or_else(|_| "mongodb://127.0.0.1:27017".into());
    let db_name = format!("wechatagent_probe_{}", uuid::Uuid::new_v4().simple());
    let db = Database::connect(&uri, &db_name).await.expect("连接本地 mongod 失败");  // ← 唯一区别
    migrations::run(&db).await.expect(...);
    db.ensure_indexes().await.expect(...);
    // ……:148-191 整段原样（prompt pack / AppState 组装）
    // 无 _container 字段，靠唯一 db_name 隔离 + 跑完 drop_database 清理
}
```

**结论：本地 mongod + 本地 LLM key → 探针全套（含真模型六阶段弧）本地全跑得动，工作流不劈半。**

**待定参数（实施时提供）**：
- mongod URI：默认 `mongodb://127.0.0.1:27017`（探针建独立 `wechatagent_probe_<uuid>` db，跑完 drop 清理）。
- LLM 接入：本地端点 = kiro-api 网关 `http://127.0.0.1:5580`（OpenAI 兼容），统一 key。**三角色三模型**（职责分离 + 异族 judge）：

  | 角色 | 模型 | env 前缀 | rate |
  |---|---|---|---|
  | 被测主链（召回/抽取/对话补库，模拟生产 agent） | `claude-sonnet-4.6` | `KB_PROBE_LLM_*` | 1.3 |
  | judge 软轨（评 answer 质量，**异族**） | `deepseek-3.2` | `KB_PROBE_JUDGE_LLM_*` | 0.25 |
  | 动态生成器（产语料/query/补库陈述） | `claude-haiku-4.5` | `KB_PROBE_GEN_LLM_*` | 0.4 |

  每套 `*_BASE_URL` / `*_MODEL` / `*_API_KEY` 三项。base-url/key 一期同端点同 key，仅 model 不同；judge/gen 缺省可回落主链。
- 网关注入观察：该端点每请求注入约 4400+ token 的框架系统提示，会拖慢并可能干扰纯 JSON 输出。实测 fenced-JSON（```json 包裹）仍出，项目解析器本就处理 fence；探针 prompt 需显式要求"只输出 JSON"并走现有 fenced-JSON 解析。

### 7.2 执行工作流（subagent-driven-development，本地全程驱动）

依赖近乎串行链：(T1‖T2)→(T3‖T4)→T5→T6→T7，并行收益小且共享 `mod.rs` 易冲突。故采用 subagent-driven-development：每个任务派全新 subagent 实现 → spec 审 → 质量审 → 过了再下一个，控制器做总控。

```
我全程本地驱动：
T0  start_on_local_mongod()（连本地 mongod 的探针底座，纯增量）
T1  ledger 模块          → 本地纯函数单测绿（不烧真模型）
T2  硬轨判据             → 本地纯函数单测绿（不烧真模型）
T3  软轨 judge           → 本地真模型验（KB_PROBE_LLM_*）
T4  动态生成器           → 本地真模型验
T5  六阶段业务弧         → 本地真模型端到端跑通（连本地 mongod）
T6  N 轮外壳 + 报告      → 本地真模型跑 N 轮 → 产 ledger + summary
T7  CI job + cron        → 上 CI 常态化长跑观测（本地已先验证）
全程 baseline 门把关零回归；红线护栏全程在。
```

T7 的 CI 角色从"唯一验证手段"降为"常态化长跑观测"。

## 8. 实施任务拆分（八个 TDD 任务，全新增、零改现有资产）

| # | 任务 | 文件 | 验证 |
|---|---|---|---|
| T0 | 无 Docker 探针底座 `start_on_local_mongod` | `tests/common/mod.rs`（新增 fn，不碰 `start`） | 本地 cargo check + 连本地 mongod 起 state 成功 |
| T1 | ledger 数据结构 + 序列化 + 趋势聚合 | `tests/kb_probe/ledger.rs` | 纯函数单测：喂假 RoundRecord，断言 trends 曲线算对 |
| T2 | 硬轨判据（纯函数） | `tests/kb_probe/judge.rs` | 纯函数单测：构造召回集，断言 reach/adopt/越界判定 |
| T3 | 软轨 judge（LLM 打分）+ combine | `tests/kb_probe/judge.rs` | `#[ignore]` 真模型：judge 对好/坏 answer 打分有区分度 |
| T4 | 动态生成器四函数 | `tests/kb_probe/generator.rs` | `#[ignore]` 真模型：产出非空、结构合法、force_gap 真造缺口 |
| T5 | 六阶段业务弧 `run_one_arc` | `tests/kb_probe/arc.rs` | `#[ignore]` 真模型：单弧①→⑥走通，库真增长，红线守住 |
| T6 | N 轮外壳 + 报告落盘 | `tests/real_llm_kb_probe.rs` | `#[ignore]` 真模型：N 轮跑完产 ledger json + summary |
| T7 | CI job + cron 编排 | `.github/workflows/ci.yml` | push 后 job 出现、cron 配置正确、artifact 上传 |

**依赖**：T0 先行（底座）→ T1/T2 纯函数（本地全验证，不烧预算）→ T3/T4 真模型模块 → T5 串成弧 → T6 外壳 → T7 上 CI。

### 8.1 CI 落点（T7）

复用 real-llm-recall job 的成熟范式（已验证抗 429、抗墙腰斩）：

```yaml
real-llm-kb-probe:                    # 新增独立 job，与 recall/ops/quality 并列
  continue-on-error: true             # 探针红线外永不阻塞合并
  timeout-minutes: 45
  strategy: { fail-fast: false, max-parallel: 2 }   # 复用限流防 429
  # 触发：cron（不挂每次 push，省预算）+ workflow_dispatch（手动随时跑）
  env:
    # 本地端点缺位时回落 coderelay；三角色见 §7.1
    KB_PROBE_LLM_BASE_URL: https://coderelay.cn/v1
    KB_PROBE_LLM_MODEL: gpt-5.5
    KB_PROBE_LLM_API_KEY: ${{ secrets.REAL_LLM_CODERELAY_API_KEY }}
    KB_PROBE_JUDGE_LLM_MODEL: gpt-5.5    # CI 无异族端点时同源回落（本地用 deepseek-3.2）
    KB_PROBE_GEN_LLM_MODEL: gpt-5.5
    KB_PROBE_MONGO_URI: mongodb://localhost:27017     # CI 起的 mongo service
    KB_PROBE_ROUNDS: "12"             # 一期 N 轮数，抗墙先 12，验证后放开
  run: cargo test --test real_llm_kb_probe -- --ignored --nocapture
  # 跑完上传 target/kb_probe_ledger/ 作 artifact（保留 30 天，供人审下载）
```

要点：cron 触发（探针是长跑观测仪，不挂每次 push）+ `workflow_dispatch` 手动入口；`continue-on-error: true`（探针产出是报告 artifact 不是绿叉守门）；judge 异族（deepseek-3.2）；MCP 永远桩。

**环境变量统一约定（消歧）**：探针代码读三套变量（三角色），加 mongo/rounds：
- 主链 `KB_PROBE_LLM_BASE_URL/MODEL/API_KEY`、judge `KB_PROBE_JUDGE_LLM_*`、生成器 `KB_PROBE_GEN_LLM_*`、`KB_PROBE_MONGO_URI`、`KB_PROBE_ROUNDS`。judge/gen 的 base-url/key 缺省回落主链，仅 model 不同。
- **本地**：端点 = kiro-api 网关 `http://127.0.0.1:5580`，三 model 见 §7.1 表；`KB_PROBE_MONGO_URI` 缺省 `127.0.0.1:27017`。
- **CI**：若 CI 无该本地网关，回落到现有召回基准的 coderelay（`KB_PROBE_LLM_BASE_URL=https://coderelay.cn/v1`、`MODEL=gpt-5.5`、`API_KEY=${{ secrets.REAL_LLM_CODERELAY_API_KEY }}`），judge/gen 同源回落；`KB_PROBE_MONGO_URI` 指向 CI mongo service。探针代码不感知本地/CI 差异，只认 `KB_PROBE_*`。

## 9. 红线护栏（全程）

- MCP 桩（绝不真发微信）
- D2 verify 不削弱（sourceQuote+anchors 双非空；AI 永不自动 verify）
- 反过拟合三焊缝（§6.2）+ 自检清单
- 召回集 ⊆ 库内 id（越界即 RED-LINE）
- 现有资产零改动（K 套件 / 召回基准 / 闭环轨迹 / TestApp::start 一律不碰）
- 探针只产报告不自动改
- no-human-takeover lint
- baseline 门（lib ≥ 350/0 + 4 PBT ≥ 33/0）每任务后零回归

## 10. 一期交付物与二期接口

**一期交付物**：能本地跑 + 能 cron 跑的 KB 业务仿真探针 + 第一批结构化短板报告（artifact），供人审。

**二期接口（验证报告价值后再做）**：T1-T5 模块原样复用，只加持久化外壳（restore→`run_one_arc`→snapshot）+ ledger 跨批聚合 fn。模块 1/2/4 零改，模块 3 只 append。可选接 C（探针自动起草抽象方法论修正提案，永不自动 merge）。

## 11. 范围外（Out of scope）

- 不改任何现有测试 / 生产逻辑 / 禁区文件（models.rs 等）。
- 不削弱任何回归门断言或 judge 阈值。
- 二期持久化、异族 judge、自动提案（C）本轮不做。
- ops-agent / 客户对话仿真不在本设计范围（本设计专注知识库真实业务）。
