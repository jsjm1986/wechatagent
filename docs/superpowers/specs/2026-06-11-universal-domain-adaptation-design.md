# 通用化知识库：LLM 对话驱动的行业/产品自适应 — 设计文档

**日期**：2026-06-11
**状态**：实施中 — **Phase 0 已提交（commit e93cd63）；Phase 1 子步 1B 已提交（557cb38）、1D 已提交（a36164d）、1A 已提交（a40c6ab）、1C 已提交（20288c3）、1E 已提交（1b95d5a）、1F-a 已提交（61eaeaa）、1F-b 已提交（13dfe9b）、1G-a 已提交（fae02a4，C1 pressure_risk typed）、1G-b 已提交（b84de31，H10 deal_events→outcome_events）；1G-c（profile 进程缓存 + 版本切换兼容护栏）已落 working tree 待提交**。**Phase 1（H1–H10 + C1 + 性能/兼容护栏）全部完成，lib 1000/0**；下一步进 Phase 1.5（H12/H13 出厂人格 + 状态机本体 + C2）
**作者**：运营 agent 侧
**目标读者**：产品负责人 + 知识库后端维护者

---

## 0. 一句话目标

系统出厂时对行业**零假设**。任意行业、任意产品的运营，通过**与 LLM 对话 + 上传行业文档**，由 AI 生成本行业的「画像维度 + 知识字段表 + prompt 片段」候选，经人工审核后落成**稳定、可版本化、可回滚**的配置；运行时消费这份配置，而非消费写死的销售域字段。

非目标（明确排除）：
- ❌ 让运行时决策 agent 每轮自由发明维度（会摧毁画像稳定性，见 §2.1）。
- ❌ 预设若干行业模板（医疗/教培/SaaS）。我们要的是「对话适配任意行业」，不是「内置 N 个行业」。
- ❌ 召回算法升级（向量/BM25/语义检索）——是另一条正交的天花板，本设计不含（见 §7 风险）。

---

## 1. 现状：意图 vs 现实（证据级）

经三个 Opus 子代理 + 主线亲自核实，知识库子系统按「通用运营知识库」设计，**知识容器层已通用，画像决策层仍锁在销售域**。

### 1.1 已经通用的部分 ✅

| 层 | 现状 | 证据 |
| --- | --- | --- |
| chunk 主表 | 30+ 跨行业稳定字段 + 单一 `domain_attributes` 可变子文档 | `models.rs:854-944` |
| 9 类 wiki_type | 认知类型（非销售类型），跨行业举例 | `knowledge-wiki.md §3` |
| 去销售域 CI 闸 | `check-no-sales-domain.sh` 主动禁止销售域命名爬回 src/ | 脚本存在 |
| 写入三层保护 | 锁定字段/数组union/70%阈值，PBT 覆盖 | `chunk_revisions.rs` + `page_merge.rs` |
| AI 永不自动 verify | 6 重冗余兜死，无漏网 | `chunk_revisions.rs:207-210` 等 |
| grounding 主硬闸 | verified-ID 精确交集，行业无关、不可被幻觉绕过 | `review/gates.rs:563-606` |
| 召回排序 | 字符 n-gram 覆盖率，语言无关 | `knowledge_agent.rs:1670-1681` |
| taxonomy check_value | 按 `kind` 字符串泛化 + scope 分层(account/global) | `taxonomy.rs:196-239` |
| 多租户 workspace_id | 贯穿所有集合 + ACL 白名单防 IDOR | `auth/middleware.rs`、`routes/auth.rs:144-151` |
| 配置版本灰度 | system_taxonomies 等三表 publish/rollout/rollback | `agent-policy.md` E5-T1 |

### 1.2 仍锁在销售域的部分 ❌（本设计的攻坚对象）

> **2026-06-11 修订**：原 H1–H5 漏掉了最硬的一块——`planner`。经穷尽排查（见
> §1.4 消费点全图），补 H6（planner 漏斗权重/终态/停滞）与 H7（两份独立维度
> 校验列表）。本设计目标修正为「**二进制里不留任何行业语义**」：哪个维度 / 什么
> 取值 / 谁优先 / 什么算终态 / 怎么解释，全部下沉到配置（引导层 LLM + 人审生成），
> 代码只保留「读配置、按配置运转」的通用机制。

| # | 硬编码点 | 位置 | 影响 |
| --- | --- | --- | --- |
| H1 | `AgentDecision.customer_stage / intent_level` 是 typed 字段 | `agent/types.rs:99-100` | 换行业维度必须改 Rust 源码 |
| H2 | `TAGGED_FIELDS` const 表硬绑 getter/setter | `agent/decision_taxonomy.rs:38-49` | 维度集合写死，不可配 |
| H3 | prompt 点名销售维度语义 | `prompts.rs:749,771,886-887,936,1595` | "客户阶段=陌生/关注/评估/决策/成交" 写死 |
| H4 | grounding 兜底探针词表中文销售词 | `guards.rs:331-345` | ✅Phase 2 已解：commitment_claim_class 走 profile.commitment_markers，DEFAULT 词表逐字等价 |
| H5 | completeness 审计五维 coverage | `catalog.rs:553-605` | ✅Phase 2 已解：走 profile.coverage_dimensions(骨架+锚点散文)，DEFAULT 五维 prompt byte-equivalent |
| **H6** | **planner 漏斗权重/终态/停滞** | `planner/mod.rs:800-829`(stage/intent 权重)、`737-738`(stagnation dotted-key 过滤)、`TERMINAL_STAGES` 常量 | 9 个销售阶段→优先级权重、high/medium/low→权重、哪些算终态全写死；换行业客户跟进排序失灵 |
| **H7** | **两份独立维度校验列表** | `decision_taxonomy.rs:38`(decision 阶段) + `gateway.rs:3262`(finalize soft gate) | 同一维度集合写死两遍，各自漂移的正确性风险 |
| **H8** | **planner 运营范式焊死成单向漏斗** | `planner/mod.rs:70-78`(三扫描器无条件每 tick 跑)、`727-774`(stage_stagnation 过滤硬绑 customer_stage) | 整套主动触达假设「人人都走陌生→成交→维护」；情绪陪伴/关系维护型无法关掉漏斗推进，也无法因用户而异调范式（见 §3.6） |
| **H9** | **conversationMode 判定 + 模式×5闸阈值写死销售域价值观** | `prompts.rs:935-940`(判定六条 if-else)、`944-950`(模式→闸映射，casual=PressureRisk≥5) | 只有「销售推进/克制寒暄」两种人格，缺「主动经营情感」；casual 被设计成 passive，情感陪伴场景的主动推进被压制（见 §3.8） |



### 1.4 消费点全图（穷尽排查结论，Phase 1 实施依据）

`customer_stage / intent_level` 的写入 + 读取点（改造必须全覆盖，否则双写不一致）：

- **写入 contact.domain_attributes**（3 套实现，须同步）：`gateway.rs:2457-2476`（AI 决策，手写 dotted-key）、`routes/shared.rs:76-91`（helper，被运营改画像 `shared.rs:548-556` + planner `update_profile` 工具 `management.rs:886-915` 共用）。
- **`customer_stage_updated_at` 时间戳**（planner stagnation 计时器命门）：3 处条件写（仅 stage 变化时刷）`gateway.rs:2468`、`shared.rs:88`、`m018:47`；planner 读它做停滞判定（`planner/mod.rs:738/858`）。`stage_changed` 判定分散 3 处独立实现——改存储方式必须同步，否则计时器失准。
- **读取点**：planner 排序/过滤（`planner/mod.rs:27-45,737-738,846-858`）、decision prompt 注入（`decision.rs:504-513`）、gateway churn 审计（`gateway.rs:2573-2580`）、memory 健康分（`memory.rs:244-246,915-922`）、shared 画像渲染（`shared.rs:475-476`）。
- **序列化**：对外 LLM 契约 **camelCase**（`customerStage`），DB 容器内 **snake_case**（`customer_stage`），由反序列化层桥接——新增 `domain_signals` 容器须沿用同一约定。
- **金标**（不破坏判定标准，仅换输入管道）：`decision_taxonomy.rs:191-336`、`planner/mod.rs:1536-1542`（顶层无 key、必须在 domain_attributes 容器）、`shared.rs:1188-1231`（同左 + stage 未变不刷 updated_at）、`tests/m018_backfill_domain_stage.rs`、`tests/real_llm_*`。

### 1.5 顺带发现的非通用化缺陷（独立问题，本设计标注但不混入）

- **D1** ✅**Phase 2 已解（写侧）**：`enforce_domain_attributes` 纯函数（required/enum/alias）+ `apply_chunk_revision` 第 6.5 步接线（有 active schema 才校验，无则 no-op）。文档宣称的"写入侧按 active schema 校验"已落地。注：其余写入站点（chat/lessons_learned/reaction/escalation）留后续分批接入。原性质：`DomainSchema` 运行时零消费——CRUD/版本/校验都建了但无运行时读取。
- **D2**：verify/reject/auto_verify/PUT/chat 五类写入**绕过** `apply_chunk_revision`，verify 这个关键状态转移**不写 chunk_revisions 历史、不更新 provenance**——审计链在"升级为 verified"处断裂。
- **D3**：关系图谱 BFS 遍历忽略 `relation_kind`，contradicts 与 references 无差别扩散；superseded_by 不做版本 redirect。

这三个是已有功能的实现缺陷，**与通用化正交**，列入「后续清理」不进本设计主线（除非 §6 分期里顺手）。

### 1.6 CRM 客观业务事实缺口（2026-06-11 双 Opus 子代理审查结论）

用户要求"全面审查数据缺口：有了 CRM 才能知道客户阶段(已购买/售后期)、产品是否购买"。经两个 Opus 子代理交叉核实（证据级 file:line），结论分两层：

**结论一 · 不需要独立 CRM 系统。** `Contact` 事实上已是一个 CRM 数据层：`commitments: Vec<CommitmentRepr>`（带 `due_at`，`models.rs:167`）、`deal_events: Vec<DealEvent>`（带 amount/currency，`models.rs:199`）、`operation_state` 状态机、`intent_trajectory`、全套时间戳、`workspace_id→account_id→contact` 三层多租户。再建独立 CRM 是重复造轮子。多租户层（`WechatAccount` capacity/persona_tag/off_hours 调度）是全仓最完整的实体维度。

**结论二 · 缺一整类「客观购买事实」数据。** 现有三个状态字段全是「对话推进状态」，且都由 LLM 从聊天推断，**没有任何字段回答"买没买/买了什么/在不在售后期"**：

| 字段 | 实际表达 | 性质 | 证据 |
| --- | --- | --- | --- |
| `customer_stage` | 9 档销售对话漏斗 | LLM 推断的主观标签 | `m006:80-135`、`prompts.rs:585-690` |
| `operation_state` | 同 9 档，驱动 AI 动作的状态机 | LLM 推断 + 状态机约束 | `prompts.rs:585-690` |
| `agent_status` | AI 是否托管(Normal/Managed) | 运营开关 | `models.rs:6-9` |

`DealEvent`（`models.rs:216-239`）是唯一成交锚点，却：**无 `product_id`（只有金额）**、**全代码库只写不读**（注释自述"只采集、不参与任何评分"`models.rs:194-197`）。

**确认缺失的客观事实数据点**：
- G1 客观购买生命周期状态（未购买/已购买/售后期/复购期）——真缺失
- G2 结构化产品目录实体（产品 ID/名称/价格/SKU）——真缺失（只有 `product_tags` 字符串标签 + 知识 chunk 非结构化描述）
- G3 成交关联产品（DealEvent 无 product_id）——真缺失
- G4 当前产品持有状态（entitlement，区别于历史成交事件）——真缺失
- G5 售后/续费/保修时间与状态——真缺失
- G6 客户价值分层（LTV/RFM/tier；`product_fit_score` 是易失单轮 LLM 评分，不在 Contact 上）——真缺失

**与通用化内核的关系（决定为什么"先做内核"是对的）**：缺口分两类，处理方式相反——

1. **生命周期阶段语义**（G1：未购买/已购买/售后期/复购期）**因行业而异**：种植牙是"咨询→面诊→种植→修复期→维护"，SaaS 是"试用→付费→续费→流失"。**绝不能写死成 `purchase_status: enum` 或第二套状态机——那就是又一个 H1 硬编码点。** 它必须作为**一个 profile 维度**（取值存 `system_taxonomies`），由引导层按行业生成。换言之 G1 是"内核的应用"，不是独立功能。
2. **结构性事实容器**（G2/G3/G4：产品目录、订单关联 product_id、持有状态）**形状跨行业通用**（任何行业都有"产品/订单/持有"概念）。建法遵循现有"稳定结构 + `domain_attributes` 可变容器"哲学，属平行于内核的客观事实增强。

**关键风险（本节固化的目的）**：若不先打好内核就补 CRM，几乎必然有人加 `purchase_status` 枚举制造第 8 个硬编码点。故 **G1 推迟到内核就绪后作为 profile 维度落地；G2/G3/G4 作为独立的"客观事实增强"专题（Phase 3 之后），不混入内核主线**。G5/G6 在"AI 自主运营微信私域"定位下属可后置项。

### 1.7 系统化深审新增硬编码点 H10–H17（2026-06-11 六 Opus 子代理 + 深挖）

H1–H9 是顺对话场景"碰"出来的；为穷尽，按六层（状态机/review五闸/知识分类/采集度量/默认内容/横切边缘）各派一个 Opus 子代理证据级排查，再对"成功/极性"暗线追加一个深挖代理。结论：销售世界观分布在**六个层面**，是同一套漏斗价值观的镜像。新增 H10–H17：

| # | 硬编码点 | 位置 | 风险/性质 |
| --- | --- | --- | --- |
| **H10** | **成功事件写死成成交**（deal_events 带 amount，注释自陈"PU-learning 唯一正例"） | `models.rs:194-239`、`routes/contacts.rs:556` | **假锚点·改它零风险**：深挖证实全库**只写不读**，PU-learning 纯注释零实现，连 `ApiContact` 都不映射。非紧急 |
| **H11** | **负反应/极性词表写死销售**（objection/unsubscribed/complaint） | `reaction.rs:310-359`(`is_negative_outcome`/`reaction_outcome_status`) | **真锚点·改它高风险**：单一真相源，横向渗透**三条已落地回路**（见下）。情感域优质回复永远判不出 Hit→自学习失效 |
| **H12** | **出厂人格=销售人设**（默认 soul 76 行顾问灵魂 + playbook"成交准备度/复购转介绍"方法论） | `prompts.rs:743-818`、`443-480` | ✅**Phase 1.5 已解**：DomainProfile 加 `soul_override`/`methodology_override`，decision 层 Some 替换 / None 回落硬编码兜底，DEFAULT 销售人格逐字不变。原性质：H3 被严重低估的真身，人格主体是编译期 `&'static str` |
| **H13** | **状态机 9 态定义本体写死**（goal/信号/风险全锚定异议/成交/复购） | `prompts.rs:585-690` + 初始态 `new_contact` 字面量散落 6 处 + `cooldown` 特例 `m013` | ✅**Phase 1.5 已解**：state 加 `initial`/`forbidsProactive` 标志，引擎+两份 PBT 闭式参考三方原子泛化，写侧 4 处/读侧 5 处走 `initial_operation_state_key`，cooldown 特例改读标志；配套 C2 令 operation_state 派生自 customer_stage + 接回 check_state_transition(fail-soft)。状态机本体随 profile 选属 Phase 3 引导层（仍活 operation_domain_configs） |
| **H14** | **grounding/ProductAccuracy 硬闸无条件**（每条回复都要 grounding≥7） | `gates.rs:28,114` | ✅**Phase 2 已解**：classify_dual_gate grounding 软分数硬闸条件化（profile.grounding_gate_bypass_without_claim + per-msg claim_analysis），DEFAULT bypass=false 字节等价。**blocked_unverified_product_claim 红线（2026-06-14 修订）**：R5.4 reviewer 自报 `requiresProductKnowledge=true` 路径仍强制 block；finalize 漏判探针（ProductEffect 分支，reviewer 未自报 ∧ 含硬承诺 ∧ 无 verified 背书）从强制 block 改为**仅观测**——成交弧"保证/效果"类词高频，知识稀缺场景下硬闸导致全程哑火。先观测漏判率，有统计证据后由运营决策是否抬回硬闸（配置化入口待 H14 配套）。原性质：纯情感回复靠 reviewer 满分兜底脆弱 |
| **H15** | **经营公式+rubric 销售化**（ConversionReadiness/ProductFit 公式 + 逼单打分锚点） | `prompts.rs:964-969`、`review/mod.rs:244-247` | 不进硬闸但占 reviewer 注意力 + 被 `/evaluations` 当 ground-truth 度量。归 1E |
| **H16** | **chunk_type+answeringMode 产品框架**（product_fact 分段 prompt、product_safe 三态） | `knowledge_router.rs:190-235`、`catalog.rs:550-581` | ✅**Phase 2 已解（chunk_type 部分）**：抽象为 ChunkRole「用途角色」(key/header/order/is_fallback)，profile.chunk_roles 驱动分桶渲染,DEFAULT 销售四态逐字等价(chunk_type_routing_pbt 1024 cases 绿)。注：answeringMode 三态文案仍写死 catalog.rs,留后续 |
| **H17** | **memoryCard schema+intent_trajectory 销售化**（objections/businessContext 一等字段；情绪史/纪念日无槽） | `memory.rs:62-97`、`models.rs:2812-2821`(`objection_type` typed) | 情感维度只能挤进泛化文本。记忆维度 schema 应随 profile |
| **H18** | **触达节奏全局写死**（debounce 窗口 4s、account off_hours 用 UTC 小时） | `webhooks.rs:582`、`account_scheduler.rs:47` | 去抖/账号作息节奏全行业共用，未随范式可配。归 1F/Phase 2（off_hours 时区错位是 C 类缺陷） |
| **H19** | **作息门控无关系类型/contact 维度**（quiet_hours 全域单值，默认 22→8 销售作息） | `runtime.rs:72-79,132-135`、`agent/mod.rs:510-513` | 情感陪伴"晚上是黄金时段"会被作息门压制到次日 8 点；H8/H9 做完仍失效。**必须纳入 `resolve_operation_mode` override 链，否则数字分身落地受阻**。归 1F |

> **2026-06-11 终极审查补充**：第八/九层（入口/作息/persona/迁移/基线/MCP/outbox）扫描发现 H18/H19。其中 **H19（作息门控）是前六层完全没碰、却会阻塞情感陪伴落地的真遗漏**——quiet_hours 是 domain_config 级单值，无 per-relationship_type/contact 维度。**已并入 1F**（resolve_operation_mode 把 quiet_hours 纳入 override 链）。另确认：`persona_tag` 是"路由池标签"≠"对话人格"（`account_scheduler.rs:33-125` 只用于同 persona 账号互替路由），数字分身的关系类型人格走 contact 级 relationship_type→OperationMode，**实施时勿把 relationship_type 复用 persona_tag**（轴混淆）。

**「成功/极性」暗线消费网全图（深挖结论，决定 H10/H11 处置）**：

```
正例：deal_events ──→ ✗ 无任何消费方（PU-learning 仅注释；前端不映射）  ← H10 假锚点·零风险
      user_replied_buying_signal ──→ classify_outcome_label→Hit ──→ dynamic_confidence↑ ──→ 知识召回排序  ← 真正活跃的正例
过程"成功"：send_success_rate=reviewer放行率 ──→ evolution promote/rollback  ← 与业务结果脱钩，跨域错配无告警
负极性：is_negative_outcome(硬编码5销售词) ──→ ①dynamic_confidence↓召回降级 ②negative_example反向训练 ③escalation卡死判定   ← H11 真锚点·高风险
```

**H10/H11 处置裁断（修正"优先级"认知）**：
- **H10**：零消费方，改它当前不影响任何行为 → **可随手做、不紧急**，不值得"抢优先级"。
- **H11**：是数字分身能否自我学习的**命门**（情感域现在优质回复零正反馈、自学习事实失效），但它是**高风险"动学习逻辑"**——单一真相源喂进召回排序+反例训练+请示，跨域语义错配会**静默污染知识召回且无业务指标兜底告警**。故 **H11 不该"抢做"，应重排到「自学习回路解耦」专门 Phase，配最强护栏（DEFAULT 逐字等价 + 三条回路各自等价性测试 + 召回排序回归基线）**。贸然早做比不做更危险。

**三个正交缺陷（非通用化，但实施时绊脚）**：
- **C1**：`pressure_risk_block_at=7` 是五闸唯一不走 typed 配置的写死阈值（`runtime.rs:113,211`）——**H9 想放宽情感场景压力阈值在 runtime 层没入口**，是 H9 隐藏前置，须先给 `RuntimeParametersTyped` 加字段。
- **C2**：`operation_state`（FSM 态）与 `customer_stage`（taxonomy 标签）**双轨冗余**，消费方分裂（planner 只读 customer_stage、gateway 只读 operation_state），可漂移；通用化只改一轨会引入不一致。
- **C3**：引导层 `PLAYBOOK_METHODOLOGY_SYSTEM`（`prompts.rs:1950`）自带"消费心理学、顾问式销售"偏见——违反 §7"引导层 prompt 不得写死行业词"护栏，会污染 AI 生成的非销售 profile，Phase 3 须清。

**战略结论**：Phase 1 原 1A–1F 只覆盖 H1–H9（决策+planner+prompt 三层）。H10–H17 揭示销售世界观还盘踞在**状态机本体（H13）、review 闸/公式（H14/H15）、知识分类（H16）、记忆/采集/度量与自学习极性（H11/H17）**。其中 H11（极性→自学习）是最深、风险最高的命门，单列高护栏 Phase；H12/H13（出厂人格+状态机本体）是 1E/Phase 2 必须补的人格主体；H10 假锚点随手清。详见 §6 重排分期。


---

## 2. 核心设计决策

### 2.1 关键分野：两种「LLM 协作」

你提出"引入 LLM 协作适配行业"，有两种做法，**必须选正解、排除陷阱**：

| | 陷阱：运行时自由发挥 | 正解：对话生成稳定配置 |
| --- | --- | --- |
| 做法 | 决策 agent 每轮自由理解行业、自由决定追踪哪些维度 | 运营对话 + 文档 → AI 生成候选配置 → 人审 → 落稳定配置 → 运行时消费 |
| 稳定性 | ❌ 维度每轮漂移，planner 停滞计时器/状态机/数据分析全废 | ✅ 配置稳定可版本化，运行时仍走现有 guard |
| 与现有红线 | ❌ 违反"画像更新须保守""禁止过拟合" | ✅ 兼容，candidate→审核流原样保留 |
| 可回滚 | ❌ 无 | ✅ 复用 publish/rollout/rollback |

**本设计采用正解。** LLM 的角色是「配置的生成者/建议者」（离线、一次性、人审把关），不是「运行时维度的发明者」。

### 2.2 三层架构

```
┌─ ① 引导层（新建·你的核心想法）────────────────────────────┐
│  运营对话："我做口腔种植牙私域获客" + 上传行业文档/产品手册   │
│  → AI 阅读 + 多轮澄清对话，生成候选：                         │
│      • 画像维度（就诊阶段/治疗意向/顾虑类型 + 取值 + 中文别名）│
│      • chunk 自定义字段表（适应症/禁忌/价格区间…）            │
│      • 该行业的 prompt 片段（如何理解这些维度）               │
│      • grounding 承诺词表（本行业的绝对化承诺词：根治/保过…）  │
│      • completeness coverage 维度（本行业"答全了吗"的标准）    │
│  → 全部走【已有的】candidate → admin 审核 → publish 流程       │
└───────────────────────────────────────────────────────────┘
                          ↓ 人工审核确认
┌─ ② 配置层（大半已存在，少量扩展）──────────────────────────┐
│  system_taxonomies   ← 画像维度字典（kind 已泛化 + scope 分层）│
│  DomainSchema        ← chunk 自定义字段表（CRUD/校验已存在）   │
│  prompt_templates    ← 行业 prompt 片段（locale 机制已存在）   │
│  domain_profile(新)  ← 声明"本行业有哪些画像维度参与决策"      │
│  版本灰度：publish / rollout / rollback（已存在，全部复用）    │
└───────────────────────────────────────────────────────────┘
                          ↓ 运行时按 active 配置加载
┌─ ③ 运行时消费层（解耦写死维度·Phase 1 解耦的部分）─────────┐
│  AgentDecision.customer_stage/intent_level（写死）            │
│         → domain_signals: Document（泛化容器，已有先例）      │
│  TAGGED_FIELDS const 表                                       │
│         → 从 active domain_profile 动态加载维度列表           │
│  check_value(kind,...) 原样复用（已泛化）                     │
│  prompt 维度语义段 → 从 active 配置注入（替代 H3 写死文案）    │
│  grounding 词表 / completeness 维度 → 从 active 配置读（H4/H5）│
└───────────────────────────────────────────────────────────┘
```

### 2.3 为什么不合并 DomainSchema 和 system_taxonomies

你最初问过这个。答案：**不合并，让引导层同时驱动它俩**。两者职责不同：

- `system_taxonomies` 治理「**agent 给客户打什么画像标签**」（需 alias 归一/候选发现/canonical id/进程缓存热路径）。
- `DomainSchema` 治理「**知识 chunk 有哪些内容字段**」（静态字段校验，面向写入）。

合并会把两套不同生命周期的东西揉在一起。正确做法是新增一个轻量的 `domain_profile`（§3.1）作为「行业总装配单」，引用这两者 + prompt 片段，形成单一 active 入口。

---

## 3. 数据模型变更

### 3.1 新增 `domain_profiles` 集合（行业总装配单）

```rust
/// 一个行业/产品的完整画像配置装配单。每 workspace 同时 1 条 is_active=true。
/// 由引导层 AI 生成候选 → admin 审核 → publish。运行时按 active 加载。
pub struct DomainProfile {
    pub id: Option<ObjectId>,
    pub profile_id: String,          // slug，如 "dental-implant-private"
    pub workspace_id: String,
    pub display_name: String,        // "口腔种植牙私域获客"
    pub description: String,         // AI 生成的行业画像说明（人可读）

    /// 参与决策的画像维度声明（替代 H2 的 TAGGED_FIELDS const 表）。
    /// 每个维度的取值字典仍存 system_taxonomies（按 kind 关联）。
    pub profile_dimensions: Vec<ProfileDimension>,

    /// 关联的 chunk 字段表（引用 DomainSchema.schema_id）。
    pub domain_schema_id: Option<String>,

    /// 行业 prompt 片段（替代 H3 写死文案）。
    pub prompt_fragment: Option<String>,

    /// 本行业绝对化承诺词表（替代 H4 写死中文销售词）。
    pub commitment_markers: CommitmentMarkers,

    /// completeness 审计维度（替代 H5 写死五维）。
    pub coverage_dimensions: Vec<CoverageDimension>,

    // 版本灰度四字段（与 system_taxonomies 等三表对齐，复用 E5-T1 机制）
    pub version: i32,
    pub current_version: bool,
    pub previous_version: Option<i32>,
    pub seeded_by: Option<String>,   // generated_by_ai / manual / default

    pub is_active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

pub struct ProfileDimension {
    pub kind: String,            // snake_case，对应 system_taxonomies.kind，如 "visit_stage"
    pub display_name: String,    // "就诊阶段"
    pub participates_in_decision: bool,  // 是否进 TAGGED_FIELDS 校验
    pub description: String,     // 注入 prompt 的语义说明
}

pub struct CommitmentMarkers {
    pub product_effect: Vec<String>,  // 本行业 ProductEffect 词（"根治率"/"保过"）
    pub tone_only: Vec<String>,       // 本行业 ToneOnly 词
}

pub struct CoverageDimension {
    pub key: String,             // "indication" / "contraindication"
    pub display_name: String,    // "适应症" / "禁忌人群"
    pub required: bool,
}
```

**R11 向后兼容**：旧库无 `domain_profiles` 时，运行时 fallback 到一个内置的 `DEFAULT_PROFILE`（等价于当前 customer_stage/intent_level + 现有词表/五维），保证零配置启动行为与今天**完全一致**。这是关键安全网——不配置照样跑，配置了才换行业。

### 3.2 `AgentDecision` 解耦（H1/H2）

```rust
// 现状（写死）：
pub customer_stage: Option<String>,
pub intent_level: Option<String>,

// 改为（泛化，已有 profile_attributes: Document 先例）：
pub domain_signals: Document,   // { "visit_stage": "consult", "treatment_intent": "high" }
```

**兼容策略**：保留 `customer_stage`/`intent_level` 作为 `#[serde(default)]` 可选字段，反序列化时若 LLM 仍输出它们，由一个 normalize 步骤迁移进 `domain_signals`（DEFAULT_PROFILE 下两者等价）。gateway 写 contact.domain_attributes 时遍历 `domain_signals` 而非读固定字段。

### 3.3 `decision_taxonomy::TAGGED_FIELDS` 动态化（H2）

```rust
// 现状：const 表 + 函数指针 getter/setter
// 改为：从 active DomainProfile.profile_dimensions 过滤 participates_in_decision=true，
//       对 decision.domain_signals 的对应 key 做 check_value（逻辑本身已泛化）。
```

`check_value(kind, raw, scope, cache)` **完全不动**——它早已按 kind 泛化（`taxonomy.rs:196`）。只把「遍历哪些 kind」从 const 表换成「读 active profile」。同一改造**必须同步两处**：`decision_taxonomy.rs:38` 与 `gateway.rs:3262` 的 finalize soft gate（H7：消灭两份漂移的列表）。

### 3.4 planner 漏斗权重/终态/停滞配置化（H6）+ 数据模型扩展

planner 的销售漏斗逻辑（`stage_priority_weight` 9 阶段、`intent_level_weight` 三档、`TERMINAL_STAGES`、stagnation 计时）是最深的行业耦合。通用化把「权重/终态」下沉到**维度取值字典**，把「哪个维度驱动停滞计时」下沉到 **profile**：

```rust
// models.rs：TaxonomyValue 增量加字段（旧数据 #[serde(default)] 兼容）
pub struct TaxonomyValue {
    pub id: String,
    pub display_name: String,
    // ...既有...
    #[serde(default)] pub priority_weight: Option<i32>,  // 该取值的跟进优先级权重
    #[serde(default)] pub is_terminal: bool,             // 是否终态（替代 TERMINAL_STAGES 常量）
}

// DomainProfile 增量：声明哪个维度驱动 planner 停滞计时（替代写死 customer_stage）
pub struct DomainProfile {
    // ...既有...
    #[serde(default)] pub stagnation_dimension: Option<String>,  // 如 "customer_stage"
}
```

planner 取权重从「match 写死的销售 canonical 值」改为「读 taxonomy 取值的 `priority_weight`，缺省 fallback 现有默认」；终态从「`TERMINAL_STAGES.contains`」改为「读取值的 `is_terminal`」；stagnation 过滤的 dotted-key 从写死 `customer_stage` 改为读 `profile.stagnation_dimension`。

**DEFAULT_PROFILE 等价**：m006 seed 给现有 9 个 customer_stage / 3 个 intent_level 取值回填与当前 `stage_priority_weight` / `intent_level_weight` **逐字相等**的 `priority_weight`，现有终态回填 `is_terminal=true`，`stagnation_dimension="customer_stage"`。planner 金标（权重档位、dotted-key 结构）判定值不变。

`customer_stage_updated_at` 时间戳：通用化后改为 `<stagnation_dimension>_updated_at` 动态 key；DEFAULT 下即 `customer_stage_updated_at`，与现状一致。3 处 `stage_changed` 写实现收敛为一个共享 helper（消除散落不一致风险）。

### 3.5 写入收敛

gateway 手写 dotted-key（`gateway.rs:2457-2476`）与 `shared.rs:76-91` helper 两套写实现，通用化时收敛为遍历 `domain_signals` 的单一 helper，避免「typed 字段写旧 key、map 写新 key」的双写不一致（穷尽排查 #2 风险）。

### 3.6 运营范式可配化（H8）—— planner 不再假设「人人都是沙漏」

> **2026-06-11 探索补充**：用户指出"不一定 100% 沙漏型——有些行业是情绪价值/陪伴、
> 有些是维护某个微信联系人的长期关系，且要能**因用户而异**地调整"。这戳中 planner
> 最深的隐藏假设：**整套主动触达逻辑都建立在"陌生→成交→维护"单向漏斗上**。H6（漏斗
> 内部权重/终态可配）不够，必须让**范式本身可换**。

**关键发现（读码结论）**：planner 现有三个扫描器（`scan_silent` / `scan_commitments` /
`scan_stage_stagnation`，`planner/mod.rs:70-78`）各自回答一种「主动触达的驱动力」，
其中只有 `stage_stagnation` 是漏斗专属：

| 扫描器 | 驱动力 | 漏斗专属？ |
| --- | --- | --- |
| `scan_silent` | 「太久没说话」时间沉默 | ❌ 跨范式通用 |
| `scan_commitments` | 「答应的事到期」事件/承诺 | ❌ 跨范式通用 |
| `scan_stage_stagnation` | 「卡在阶段没推进」漏斗推进 | ✅ 沙漏专属 |

所以"范式可配"落到代码 = **三个驱动力各自可开关 + 可调参**，而非把漏斗推倒重写。

**数据模型（两级声明，contact 覆盖 profile）**：

```rust
/// 运营范式：声明启用哪些「主动触达驱动力」+ 各自参数。
/// 全字段 #[serde(default)]，缺省即「沿用全局 config」——DEFAULT 下零行为变化。
pub struct OperationMode {
    #[serde(default)] pub funnel: FunnelMode,          // 沙漏推进(stage_stagnation)
    #[serde(default)] pub silence: SilenceMode,        // 沉默唤醒(scan_silent)
    #[serde(default)] pub commitment: CommitmentMode,  // 承诺到期(scan_commitments)
}
pub struct FunnelMode {
    #[serde(default = "default_true")] pub enabled: bool,
    #[serde(default)] pub stagnation_threshold_days: Option<i64>, // None→回落全局 config
}
// SilenceMode { enabled, threshold_hours: Option<i64> }
// CommitmentMode { enabled, imminent_window_hours: Option<i64> }  同构

// DomainProfile 增量：行业默认范式
pub struct DomainProfile { /* ...既有... */
    #[serde(default)] pub operation_mode: OperationMode,
}
// Contact 增量：单客户覆盖（优先级高于 profile，承接「因用户而异」）
pub struct Contact { /* ...既有... */
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_mode_override: Option<OperationMode>,
}
```

**三种范式怎么落**：
- **销售型**（DEFAULT）：三驱动力全开 + 阈值缺省回落全局 config → 与现状**逐字等价**，planner 全部金标零变化。
- **陪伴/情绪型**：`funnel.enabled=false`（不推进阶段，不被 stagnation 催），silence 开但阈值更长更温和，commitment 仍开（约的事还记）。
- **关系维护型**：funnel 关，silence+commitment 开，阈值按「长期保鲜」调。
- **因用户而异**：某老客户「只维护不推进」→ 在他 Contact 上设 `operation_mode_override.funnel.enabled=false`，单独对他关漏斗，不影响同号其他客户。

**实现要点（决定本设计安全的根据）**：
- 关 funnel = `scan_stage_stagnation` 对该 contact 短路 `return false`。**纯减法**，不碰 silent/commitment、不碰任何排序权重、不碰 `stage_priority_weight`。
- silent/commitment 的 enabled/阈值：现为全局 config（`strategic_planner_silent_threshold_hours` 等，`config.rs`）。改为「effective = contact override ?? profile ?? 全局 config」三级回落；缺省即现状。
- 三扫描器在 loop 里仍都被调用；变化在**每个扫描器内部**——按 contact 解析出的 effective mode 决定「这个 contact 跑不跑我这段」。解析 helper：`resolve_operation_mode(contact, profile) -> OperationMode`，单一来源避免散落。

**明确划入「做加法」边界、不进当前内核**：上述全是「开关 + 阈值 + 短路」=减法与调参，今天能干净落地。但「陪伴型**主动发起**情绪关怀」需要**新扫描器**（情绪/节奏信号采集），是独立专题，列 Phase 3 之后。当前内核只把「范式可声明、漏斗可关、阈值两级可调」的地基打牢。

**DEFAULT 等价护栏**：`OperationMode::default()` = 三驱动力 enabled=true + 所有阈值 None（回落全局 config）。新增测试断言 `resolve_operation_mode` 在无 profile/无 override 时产出的 effective 参数 = 当前全局 config 值，且 funnel 全开 → planner 现有金标逐条不变。

### 3.7 数字分身：关系类型 × 驱动力组合（H8 的产品兑现）

> **2026-06-11 头脑风暴**：用户进一步定位——这个 agent **不只是"客户运营工具"，而是
> "微信号本人的 AI 化身"**，要能托管机主**除客户外的社交**：朋友、同行，处理节假日
> 祝福、社交钩子等，"让人放心托管"。身份范围明确 = **客户 + 同行 + 朋友**三层
> （家人暂不纳入）。

**核心定位修正**：「数字分身」不是第四个 `OperationMode` 枚举值，而是把 §3.6 的
「驱动力自由组合」从"按行业"扩展到"按**关系类型**"。每个 contact 有一个
`relationship_type`（**走 system_taxonomies 维度**——用户已拍板，因行业而异：有些行业
还有供应商/合作方，不写死枚举），profile 为每种关系类型声明一套 `OperationMode`。
三层差异 = 三套驱动力组合 + 阈值 + 口吻：

| 关系类型 | 启用驱动力 | 典型配置 |
| --- | --- | --- |
| 客户 | 漏斗 + 沉默 + 承诺 + 日历 | 漏斗全开，沉默阈值短（怕丢单），祝福偏商务 |
| 同行 | 沉默(长) + 承诺 + 日历 | 漏斗关，低频，祝福偏行业节点 |
| 朋友 | 沉默(长) + 日历 | 漏斗关，承诺可选，祝福偏个人情感、口吻最像本人 |

这比"N 个预设范式"灵活：不是在预设里选，而是每种关系类型**自由组合驱动力**。

**关键设计哲学（用户拍板，必须固化，防止回退）**：

1. **三层全部 100% 全自动发送，无"待确认/草稿/审批"状态。** 理由（用户原话逻辑）：
   (a) 发出的内容本人在自己手机上**自然可见**，事后能发现纠正；(b) 人的干预点在
   **"事前配置"**（运营接入时手动调画像 + 调 operation_mode），不在"事中逐条确认"。
   **"放心托管"= 把规则配好后信任它自动跑，不是每条都盯。**
2. **因此与 `check-no-human-takeover` 红线零冲突**：没有任何"待人确认/接管"语义。
   `autonomy_level`/"草稿待确认"这类概念**明确排除出设计**——人的控制权在配置层行使。
3. **客户侧自主性不变**：朋友/同行是机主的社交，不是"客户对话被人接管"——客户侧
   仍 100% AI 自主。社交侧也是 AI 自动发，只是机主在配置时定了口吻/驱动力/阈值。

**地基增量（落在现有体系，无平行新系统）**：
- `relationship_type` = system_taxonomies 的又一维度（复用 check_value / 候选 / 版本灰度）。
- 三层运营策略 = profile 为每个 relationship_type 配一套 `OperationMode`（§3.6 已建模）。
- **节日祝福（日历驱动）/ 社交钩子（外部事件驱动）= 给 planner 加新扫描器**——这是
  **做加法**（新信号采集），是独立专题，**不进当前内核解耦**。当前内核只确保：
  关系类型可声明、每类关系的驱动力组合可配、漏斗可关、阈值可调。新扫描器在地基就绪
  后增量挂载，不改 `resolve_operation_mode` 架构。

**待澄清（不阻塞内核）**：日历祝福依赖"生日/纪念日"数据从哪来；社交钩子（朋友圈/群）
依赖 MCP 能否拉到外部事件——这两个新驱动力落地前需先摸清 MCP 能力边界，列入专题。

### 3.8 conversationMode + 模式×5闸阈值写死销售域价值观（H9）

> **2026-06-11 压力测试发现**：用户给"加好友→托管→像男朋友一样对待她、主动提供
> 情绪价值、推动亲密关系"这条指令做边界测试。读码核实结论 = **情感陪伴是正式目标
> 场景**（用户拍板），但现有 `conversationMode` 机制为它埋了一个深层障碍。

**核实结论（先纠正一处误判）**：情感对话**不会**被产品类安全门误杀——
`casual_relationship` 模式下 FactRisk/ProductAccuracyScore 几乎不参与（`prompts.rs:946`），
承诺词硬闸也把语气类（保证/一定能/绝对）标为"最易误杀情感承诺，仅观测不拦"
（`guards.rs:322`）。`custom_agent_instructions` 末位最高优先级注入且能直接指定
conversationMode（`prompts.rs:935`、`decision.rs:380-391`）——指令链通。

**真正的硬编码点 H9**：`conversationMode` 的**判定规则**（`prompts.rs:935-940` 六条
if-else）+ **模式→5闸阈值映射**（`prompts.rs:944-950`）把**销售域价值观写死在 prompt
文案里**：

- 判定规则锚定销售语义：「customer_stage∈{方案匹配/异议处理/承诺跟进}→consultative」
  「用户问产品/价格/方案→consultative」——非销售行业无"方案/异议/采购"概念。
- 模式→闸映射写死「**casual_relationship：PressureRisk 阈值收紧 ≥5 即拦，杜绝寒暄里夹
  推销**」。这把"闲聊"等同于"该克制、给空间、压制主动性"——是销售域价值观（闲聊不
  该推进）。但**情感陪伴场景价值观相反**：主动提供情绪价值、积极推动亲密是**正当的**。
  现有机制只有"销售推进(consultative)"和"克制寒暄(casual)"两种人格，**缺"主动经营
  情感关系"这第三种**——casual 被设计成了 passive。

**影响**：当前 agent 能**被动**把情感对话回应得不错，但**主动经营亲密关系做不到**，且
casual 模式的"收紧压力门"会与"热烈推进"打架（非误杀，是模式设计就要压制主动性）。

**改造方向（归属 H3/1E，同文件同类改造）**：H9 与 H3（prompt 维度语义注入）都在
`prompts.rs`、都是"把写死的销售域语义抽到 profile 注入"，故**合入 1E 一起做**：
- conversationMode 的**取值集合 + 判定规则**从 `profile` 注入（替代写死六条 if-else 和
  四模式枚举）；情感陪伴 profile 可声明 `intimate_companion` 之类模式。
- **模式→5闸阈值映射**从 `profile` 读（替代写死「casual=PressureRisk≥5」）；情感 profile
  可声明「亲密推进模式 PressureRisk 阈值放宽、EmotionalValue 权重提高」。
- **DEFAULT_PROFILE 逐字复刻当前四模式 + 当前映射**（含 casual=≥5），销售域行为零变化；
  real-LLM conversationMode 金标判定标准不变。

**红线守护**：H9 放宽的是**情感/社交场景**的主动性阈值，**不触碰** boundary_protection
模式那套"严禁承诺真人/上级/转交"的反接管硬规则（`prompts.rs:950`）——那条无论什么
行业都不松动，继续写死。H9 只让"用什么模式 + 各模式 5 闸阈值"可配，不让"反人工接管
红线"可配。

**Phase 3 端到端验证场景**：情感陪伴（"像男朋友一样"指令 → intimate_companion 模式 →
主动情绪价值触达 → 不被 PressureRisk 压制）列为非销售行业验证样例之一。




---

## 4. 引导层（①）详细设计

### 4.1 交互流程

```
运营进入「行业配置向导」（控制台新增入口）
  1. 对话开场："你的业务是什么行业、卖什么产品/服务、客户是谁？"
  2. 运营答 + 上传行业文档（产品手册/SOP/话术集，走已有 import 管道）
  3. AI 阅读文档 + 多轮澄清（"你的客户从接触到成交一般经历哪几个阶段？"）
  4. AI 产出候选配置草案（结构化 JSON，对应 DomainProfile 各字段）
  5. 运营在 UI 逐项审核/编辑（维度名、取值、别名、词表、coverage）
  6. 确认 → 走已有 candidate→publish 流，落 domain_profiles + system_taxonomies + DomainSchema
  7. activate → 运行时下一轮决策即用新 profile
```

### 4.2 LLM 产物形态：patch-only / 结构化输出

复用现有 `generate_agent_json`（`agent/mod.rs` 唯一 LLM JSON 入口）。AI 只返结构化候选 JSON，**不直接写库**——和 chunk 的 patch-only 协议同理，人审后由后端落库。

### 4.3 红线继承

- AI 生成的所有配置 = **候选**，必须人工审核才生效（继承"AI 永不自动 verify"精神）。
- 文档导入产出的 chunk 仍 `draft + needs_review`（红线不变）。
- 候选不阻塞任何运行时路径（继承 taxonomy candidate "SHALL NOT 阻塞" 约束）。

---

## 5. 分工边界

| 改动 | 归属 | 说明 |
| --- | --- | --- |
| `domain_profiles` 集合 + 模型 + CRUD + 版本灰度 | 知识库后端 | 新集合，沿用 E5-T1 版本机制 |
| `DomainSchema` 运行时接线（修 D1） | 知识库后端 | 让 active schema 真正校验 chunk 写入 |
| 引导层 AI 生成配置（prompt + 结构化输出 + 审核 UI 后端） | 知识库后端 + 运营 agent 协作 | 跨界，需对齐 |
| `AgentDecision` 解耦 domain_signals（H1） | **运营 agent（我）** | 核心路径 |
| `TAGGED_FIELDS` + gateway soft gate 动态化（H2/H7） | **运营 agent（我）** | 核心路径，两份列表同步 |
| prompt 维度语义动态注入（H3） | **运营 agent（我）** | 核心路径 |
| **conversationMode + 模式×5闸阈值配置化（H9）** | **运营 agent（我）** | prompts.rs，与 H3 合入 1E；boundary_protection 反接管红线不可配 |
| grounding 词表配置化（H4） | **运营 agent（我）** | guards.rs |
| completeness 维度配置化（H5） | 知识库后端 | catalog.rs |
| **planner 权重/终态/停滞配置化（H6）** | **运营 agent（我）** | planner/mod.rs + TaxonomyValue 扩字段 |
| **planner 运营范式可配化（H8）** | **运营 agent（我）** | planner/mod.rs + DomainProfile.operation_mode + Contact.operation_mode_override |
| 引导层前端向导 UI | **前端（我）** | 控制台新增 |

**已裁决**：用户授权由我**统一推进**整个工程（横跨运营 agent + 知识库后端 + 前端，作为一个整体）。知识库侧改动在 commit 信息标注。

---

## 6. 分期落地（每期可独立交付、可回滚）

### Phase 0：安全网先行（低风险，纯增量）
- 新增 `domain_profiles` 集合 + `DomainProfile` 模型 + `DEFAULT_PROFILE` 内置常量。
- 运行时**仍走写死路径**，只是并行加载 active profile（无则 DEFAULT）。
- 验证：零行为变化，基线测试全绿。

### Phase 1：运行时消费层全面解耦（核心，覆盖 H1–H9）

**执行纪律**：每个写死点 = 一个独立 commit；每步在 DEFAULT_PROFILE 下**零行为变化**，跑全基线（lib≥350/0 + 4 PBT≥33/0）+ 三禁词闸验证绿，再进下一步。金标的**判定标准一字不改**——只把「喂数据的管道」从写死常量换成喂 DEFAULT_PROFILE（逐字复刻当前销售值）；每处在 commit 标注。范围**覆盖全部 H1–H9，不中途收工**。

子步（**按依赖顺序执行：1B→1D→1A→1C→1E→1F→1G**；数据模型/容器先行，维度列表动态化必须在 domain_signals 容器就绪后，否则二次返工。每步独立 commit + 验证）：

- **1B · 数据模型扩展** ✅**已完成（working tree，待与设计一起提交）**：`TaxonomyValue` 加 `priority_weight`/`is_terminal`，`DomainProfile` 加 `stagnation_dimension`（`#[serde(default)]` 兼容）；m006 seed 回填现有取值的权重/终态使其等价当前 planner 硬编码值；新增等价护栏测试 `seeded_weights_match_planner_hardcoded_verbatim`。lib 963/0。
- **1D · H1 domain_signals 容器 + 写入收敛** ✅**已完成**：`AgentDecision` 加 `domain_signals: Document`（`#[serde(default)]`），**保留 `customer_stage`/`intent_level` typed 字段（红线：删了会破 lib 基线 + state_transition_pbt）**；新模块 `agent/domain_signals.rs` 提供 `normalize_domain_signals`（typed↔容器双向同步，typed 取 canonical 后为权威）+ `insert_domain_signal_values`（遍历容器写 `domain_attributes.<key>` dotted-key + `stage_changed` 刷 `customer_stage_updated_at` 的单一写入内核）。gateway 决策落库（`apply_agent_updates`，clone+normalize 后经内核，捕获第二道 taxonomy 软闸的 canonical 改写）与 `routes::shared::insert_domain_stage_fields`（admin 改画像，wrapper 保留 typed 签名供 6 处调用方 + 既有「容器时间戳总刷」契约）两套写实现收敛到同一内核。normalize 注入点 = `decide_reply_with_promote` 在 taxonomy 规整之后。lib 974/0，PBT 累计 34/0，禁词/model-hint 闸净。
- **1A · H2+H7 维度列表动态化** ✅**已完成**：`decision_taxonomy::classify_decision_tags`（删 `TAGGED_FIELDS` getter/setter const 表）与 `gateway::compute_taxonomy_guard_outcome`（删两维 named rewrite 字段，改 `rewrites: Vec<(kind,canonical)>`）两份硬编码列表统一改读 `decision_dimension_kinds(active_profile)`，取值读写经 `domain_signals::get_dimension`/`set_dimension`（销售两维走 typed、其它行业维度走容器）。两条生产入口（`decide_reply_with_promote` + gateway 软闸）各 `load_active_domain_profile(workspace_id)` 取维度集。`check_value` 不动；DEFAULT 返 `["customer_stage","intent_level"]` 逐字等价，原 9+9 测试经测试桥（`classify_with_cache_for_tests` 默认两维 / `guard()` + `rewrite_of()`）零改语义保留，新增 4 测覆盖非销售维度（容器读取/alias 改写/未知值进候选）。lib 976/0，PBT 累计 34/0，禁词/model-hint 闸净。
- **1C · H6 planner 漏斗内部配置化** ✅**已完成**：planner 新增 `PlannerStageConfig`（stage/intent 权重表 + 终态集 + stagnation_dimension），每 tick 由 `resolve_planner_stage_config(workspace,account)` 从 active profile（取 `stagnation_dimension`）+ taxonomy 缓存（新增 `dimension_value_weights` 读出 1B seed 的 `priority_weight`/`is_terminal`）构造一次（避免 N+1）。`stage_weight`/`intent_weight`/`is_terminal_stage` 字典命中用字典值、否则回落写死 `stage_priority_weight`/`intent_level_weight`/`TERMINAL_STAGES`（写死函数**保留为 fallback**，`seeded_weights_match_planner_hardcoded_verbatim` 锁住二者逐字相等）。三消费点（`stage_stagnation_passes_in_memory` 终态判定 + 两个 priority_key 排序键）改读 config；停滞计时维度从 `customer_stage` 写死改读 `config.stagnation_dimension`（DEFAULT 仍 customer_stage，新增 `contact_stagnation_updated_at(dim)` helper）。`taxonomy::CachedEntry` 扩 `priority_weight`/`is_terminal` 两字段（reload + 两测试构造点回填）。MongoDB 端 `$nin TERMINAL_STAGES` 粗过滤 + 写死 dotted-key 留作 DB 预过滤（权威终态判定在内存 config，DEFAULT 等价；DB dotted-key 动态化留后续 milestone）。lib 978/0（`-D warnings` 净，+2 config 等价/覆盖测试），PBT 累计 34/0，禁词/model-hint 闸净。
- **1E · H3+H9 prompt 语义 + conversationMode 注入** ✅**已完成**：（H3）`DomainProfile.prompt_fragment` 现作为独立「业务上下文」层注入决策系统提示，位置在 Policy 与 Operator Instruction 之间（`decision.rs` 系统提示拼装），DEFAULT_PROFILE `prompt_fragment=None` → 空串、系统提示逐字等价改造前。（H9）conversationMode 取值集合不再写死，`UserRuntimeParameters` 加 `allowed_conversation_modes: Vec<String>`（`from_config`/`Default`/全部显式构造点给内置默认四模式 = `default_conversation_modes()` helper），`validate_and_promote` 改读 `runtime.allowed_conversation_modes`（空时 fallback 到 const `CONVERSATION_MODE_VALUES` 四模式）做严格枚举校验；`decide_reply_with_promote` 在函数顶部一次性 `load_active_domain_profile`（H2/H3/H9 三处复用），用 `profile.conversation_modes`（非空时）覆盖 runtime 后再 promote。DEFAULT 销售域四模式逐字等价。**实测发现「模式→5闸阈值映射」是 prompt 散文、非强制代码（`review/gates.rs` 用单一 `pressure_risk_block_at=7` 不分模式）**，故 1E 的代码改动聚焦 conversationMode 枚举本身；模式差异化阈值的 runtime 入口（C1 `pressure_risk_block_at` typed 化）留 1G、prompt 散文文案的 profile 化随 H12（Phase 1.5 人格延伸）。**boundary_protection 边界保护硬规则不进 prompt_fragment、不可配、继续由 `user.reply.policy` 写死守护**（红线）。新增 4 条确定性单测（`h9_*`：默认四模式逐字锁 / 默认集合拒非销售模式 / profile 注入 `intimate_companion` 放行且销售模式反被拒 / 空集合 fallback 四模式）+ `default_profile_conversation_modes_match_const_verbatim`（DEFAULT_PROFILE 四模式 == const）。lib 983/0（`-D warnings` 净），PBT 累计 34/0 + conversation_mode_decision_schema 7/0，禁词/model-hint 闸净。
- **1F · H8 运营范式可配化（§3.6）** —— 拆为 1F-a（H8 planner 范式）+ 1F-b（H19 作息门控）两个独立 commit：
  - **1F-a · H8 三驱动力可配化** ✅**已完成**：新增 `OperationMode { funnel, silence, commitment }` + 三子结构 `FunnelMode`/`SilenceMode`/`CommitmentMode`（各 `enabled: bool`（serde 默认 `true`）+ 一个 `Option<i64>` 阈值，缺省回落全局 config）。落 `DomainProfile.operation_mode` + `Contact.operation_mode_override: Option<OperationMode>`。新增纯函数 `resolve_operation_mode(contact, profile_mode)`：`contact.override ?? profile`（**整组替换**，不逐驱动力 merge）。三扫描器各**每 tick 加载一次 active profile**（`scan_silent` 新加载；`scan_commitments`/`scan_stage_stagnation` 把原 `resolve_planner_stage_config` 改为先 `load_active_domain_profile` 再 `build_planner_stage_config(&profile)`，一次加载兼供排序配置 + operation_mode，避免双重 load），逐 contact `resolve_operation_mode` 后：① 对应驱动力 `enabled=false` → `continue`（关 funnel = `scan_stage_stagnation` 短路，纯减法）；② 有效阈值 = `override ?? profile ?? 全局 config`，在内存按 `silent_hours_for`/`idle_days_since`/imminent window 再过滤一次。**DEFAULT 等价**：`OperationMode::default()` = 三全开 + 阈值 None → enabled 短路不触发、阈值恒等于全局 config（DB 粗筛即按全局阈值，in-memory 复核恒真），planner 金标逐条不变。新增 4 测（`h8_*`：默认全开+阈值 None / 无 override 回落 profile / override 整组替换 / 默认阈值==全局）+ `default_profile_operation_mode_is_all_enabled_default`。lib 988/0（`-D warnings` 净），PBT 累计 34/0，禁词/model-hint 闸净。17 处 `Contact` 构造点（含 14 个集成测试）补 `operation_mode_override: None`。
  - **1F-b · H19 作息门控纳入范式链** ✅**已完成**：`OperationMode` 加第四子结构 `QuietHoursMode { enabled_override: Option<bool> }`（仅覆盖「是否启用静默」，起止小时/时区偏移继续走全局 runtime，避免在 contact 上重复整套作息参数）。新增纯函数 `quiet_hours::effective_quiet_hours_enabled(contact, global_enabled)`：`contact.operation_mode_override.quiet_hours.enabled_override ?? global_enabled`，**不查 DB**（覆盖来自 contact 已加载字段，热路径零额外 IO）。两处 `is_quiet_now` 强制点改用它：webhook 入站延迟（`webhooks.rs`）+ gateway 主动发送 precheck（`gateway.rs`）。**DEFAULT 等价**：`QuietHoursMode::default().enabled_override = None` → 回落全局 `runtime.quiet_hours_enabled`，与改造前逐字一致；情感陪伴 contact 设 `Some(false)` → 夜间黄金时段不被 22→8 压制，`Some(true)` 强制开。新增 4 测（`h19_*`：无 override 跟随全局 / Some(false) 关 / Some(true) 强开 / 默认 None）。lib 992/0（`-D warnings` 净），PBT 累计 34/0，禁词/model-hint 闸净。
- **1G · H10 假锚点清理 + C1 前置 + 性能/兼容护栏** —— 拆为 1G-a/1G-b/1G-c 三个独立 commit：
  - **1G-a · C1 pressure_risk_block_at typed 化** ✅**已完成**：五闸里唯一写死在 `UserRuntimeParameters`（=7）不走 typed 配置的阈值，加入 `RuntimeParametersTyped`（serde 默认 7）+ defaults 模块 + Default impl；`UserRuntimeParameters` 的 `from_config`/`Default` 改读 `typed.pressure_risk_block_at` 替代写死 7。这是 H9 隐藏前置——情感/陪伴场景经运营域配置放宽压力阈值（如 9）的 runtime 入口。DEFAULT=7 逐字等价。扩充既有 2 个 typed 测试。lib 992/0。
  - **1G-b · H10 deal_events 泛化** ✅**已完成**：`DealEvent`→`OutcomeEvent`、`Contact.deal_events`→`outcome_events`（`#[serde(alias = "deal_events")]` 让改名前写入的旧库文档继续可读 = R11 兼容），销售域"成交"语义注释泛化为行业中性"成效/结果"。深挖证实全库**只写不读**（唯一写入方 `add_deal_event` 路由 + S5 采集，PU-learning 纯注释零实现，前端零引用），故零行为风险。路由路径 `/contacts/:id/deal-events` + 请求类型名 `DealEventRequest` 保持不变（API 兼容，无外部消费方依赖语义）；审计事件 kind 改 `outcome_event_marked`。~30 处构造点（含 14 集成测试）+ 路由 + smoke 测试更新；smoke 测试加 alias 向后兼容断言（旧 `deal_events` key 写库经 alias 读入 `outcome_events`）。lib 992/0，PBT 累计 34/0，集成测试 crate `-D warnings` 编译净，禁词/model-hint 闸净。
  - **1G-c · profile 缓存 + 版本切换兼容护栏** ✅**已完成**：（缓存）`load_active_domain_profile` 改走与 `TaxonomyCache` 同款进程级 `DomainProfileCache`（按 `workspace_id` 索引，30s TTL + `init_global_domain_profile_cache` 启动预热 + `invalidate_global_domain_profile_cache` publish 失效钩子留 Phase 3 引导层接线），治 1A/1C/1E/1F 引入的"每决策 / 每 planner tick 都查 `domain_profiles`"N+1。命中返回真实 profile clone，未命中 / DB 空 / DB 错误 / 重载失败均回落 `default_domain_profile`，与接缓存前 `find_one` 的 Ok(None)/Err 分支**逐字等价**（`get_or_load` 与单测共用 `lookup_or_default` 同一回落口径）。`get_or_load` 复用 `is_stale` TTL 判定（与 taxonomy 同口径），main.rs 在 taxonomy 预热后接入预热。（版本切换兼容）`contact_stagnation_updated_at` 在配置维度 `<dim>_updated_at` 缺失时**回落旧 `customer_stage_updated_at`**：否则运营把 profile 的 `stagnation_dimension` 从 customer_stage 换到新维度后，尚无 `<新维度>_updated_at` 的存量 contact 会被 `stage_stagnation_passes_in_memory` 判 None 排除、主动触达静默冻结。DEFAULT dim=customer_stage 时主查与回落同 key，逐字等价（金标零变化）；新维度时间戳存在则优先用新维度（回落只兜底）。新增 8 测（4 cache：空缓存 stale / TTL 到期转 stale / invalidate 重置 / 未命中回落 default+命中返真实；4 stagnation：DEFAULT 维度逐字读 / 存量 contact 回落旧字段 / 已迁移优先新维度 / 两者皆缺为 None）。lib 1000/0（`-D warnings` 净），禁词/model-hint 闸净。**Phase 1（H1–H10 + C1 + 性能/兼容护栏）全部完成。**

### Phase 1.5：出厂人格 + 状态机本体配置化（H12/H13，1E 的人格延伸）✅**已完成**
- **H12** ✅：DomainProfile 加 `soul_override` / `methodology_override` 两个 `#[serde(default)] Option<String>`（H12-1，纯 additive）。`decision.rs` soul 解析改 `match non_empty_override(profile.soul_override) { Some→替换, None→走 DB published + 硬编码兜底 }`（H12-2）；playbook 同款经 `methodology_override` 注入 user-message"当前运营方法"段（H12-3，等价测试断言 user-message 拼装而非 system 串）。DEFAULT_PROFILE 两字段 = None → soul/方法论逐字回落原硬编码，销售人格字节不变。
- **H13** ✅：`OperationStateTyped` 加 `initial` / `forbids_proactive` 两个 `#[serde(default)] bool`（camelCase 序列化 `initial`/`forbidsProactive`）；`default_user_operation_state_machine` 仅 `new_contact` 标 `initial:true`、仅 `cooldown` 标 `forbidsProactive:true`（H13-1，配 migration m019 回填存量 config）。引擎 `check_state_transition` L177 `to=="new_contact"` → `target.get_bool("initial")`，与两份 PBT 闭式参考三方原子同 commit 泛化（H13-2★）。`new_contact` 初始态字面量写侧 4 处 + 读侧兜底 5 处全改走 `initial_operation_state_key`（active 状态机取 `initial:true` 的 key），signature 改造串 `initial_state` 参数（H13-3）。`cooldown` 禁回复特例 m013 改读 `forbidsProactive`（H13-4，**不碰** planner TERMINAL_STAGES——那是 customer_stage 终态轴，已由 1C taxonomy is_terminal 泛化，与 FSM 轴正交）。
- **C2** ✅（强制同步 + 接回校验闸）：gateway `apply_agent_updates` 令 `operation_state` 派生自归一后的 `customer_stage`（同一 canonical id 空间，消除双轨漂移），缺 stage 时回落 `decision.operation_state`（C2-1）；同处补调 check_state_transition 把准死代码引擎接回生产路径，**fail-soft**：非法迁移不阻断 reply、仅拒写 operation_state（留旧 state）+ 写 `agent.operation_state_transition_rejected` 审计事件（与 transitioned 事件互斥），domain_config=None 时 fail-open 逐字等价；transitioned 事件改据**实际写入值** `applied_operation_state` 判定保库/事件一致（C2-2）。CLAUDE.md:134 硬规则注释更新为接回后真实语义。
- DEFAULT 等价：销售域状态机/人格逐字复刻，现有状态机金标 + real-LLM 套件零变化。lib 1007/0（`-D warnings` 净）；state_transition_pbt 6/0；string_fact_risk_guard 13/0；禁词/model-hint 双闸净。**Phase 1.5（H12 + H13 + C2）全部完成。**

### Phase 2：grounding / completeness / 知识分类配置化（H4/H14/H5/H16 + 修 D1）✅**已完成**
- **H4** ✅：`commitment_claim_class` 改签名吃 `&CommitmentMarkers`（来自 active profile），两组词表皆空回落内置销售 const；唯一消费方 finalize grounding 漏判探针接 `active_profile.commitment_markers`。DEFAULT 词表逐字复刻 → 字节等价。
- **H14** ✅：`classify_dual_gate` 的 grounding 软分数硬闸条件化——`grounding_gate_applies = !runtime.grounding_gate_bypass_without_claim || claim_requires_product_knowledge(review.claim_analysis)`。新 profile/runtime 字段 `grounding_gate_bypass_without_claim`（default false=DEFAULT 无条件硬闸字节等价），gateway 加载 profile 后覆盖。**红线（2026-06-14 修订）**：R5.4 verified 强约束（reviewer 自报 `requiresProductKnowledge=true` 路径）不变；**finalize 漏判探针（ProductEffect 分支）从强制 block 改为仅观测**——成交弧高频承诺词在知识稀缺场景下导致全程哑火，先观测漏判率积累统计证据。落 `kind="grounding_probe_reviewer_missed" / status="observe"` 事件，不改发送判定。
- **H5** ✅（拆 a/b）：completeness coverage 五维从写死改读 `profile.coverage_dimensions`。a=结构化骨架（fallback 对象 + prompt JSON 骨架由维度动态生成，`build_coverage_skeleton` 对齐规则逐字复刻）；b=命中锚点散文（`CoverageDimension` 加 `anchor_hint`，`build_coverage_anchors` 按维度生成）。两份 byte-equivalence 快照测试锁死 DEFAULT 五维 prompt 字节不变。
- **H16** ✅（拆 a/b）：chunk 分段从写死销售四态抽象为「用途角色」。a=`ChunkRole{key,header,order,is_fallback}` 模型 + `DomainProfile.chunk_roles` + `default_chunk_roles()` seed（逐字复刻四态）；b=`format_operation_knowledge_for_prompt_with_roles` 按角色分桶/排序/渲染，decision+reviewer 传 `profile.chunk_roles`，无参 wrapper 委托 DEFAULT 供 PBT。`chunk_type_routing_pbt` 1024 cases 全绿。
- **修 D1** ✅（拆 a/b）：DomainSchema 运行时接回写侧。a=`enforce_domain_attributes(schema, attrs)` 纯函数（required 缺失/enum 越界 reject、alias→canonical rewrite，无 IO）；b=`apply_chunk_revision` 第 6.5 步当且仅当存在 active schema 时校验 domain_attributes，无 active schema（DEFAULT/`domain_schema_id=None`）→ no-op 直通零行为变化。其余写入站点（chat/lessons_learned/reaction/escalation）留后续分批。
- 验证：lib 1007→1026/0（每步递增等价/行为测试）；4 baseline PBT + chunk_type_routing_pbt 全绿；RUSTFLAGS=-D warnings 净；禁词/model-hint 双闸净。**Phase 2（H4/H14/H5/H16/D1）全部完成。**

### Phase 2.5：自学习回路极性配置化（H11，最深命门·最高护栏·独立 Phase）
> H11 是数字分身能否自我学习的命门，但是**高风险"动学习逻辑"**：`is_negative_outcome` 单一真相源横向渗透知识召回排序 + negative_example 反向训练 + 请示卡死三条已落地回路，跨域语义错配会**静默污染召回且无业务指标兜底告警**。故单列、配最强护栏。
- `is_negative_outcome` / `reaction_outcome_status` 的正负极性词表从 profile 声明（DEFAULT 逐字复刻当前 5 销售词）。
- H17：memoryCard 结构化字段集 + memoryCandidate type 集 + intent_trajectory 维度从 profile 读（情感域可声明情绪史/纪念日槽）。
- **强护栏**：三条回路（dynamic_confidence 召回排序 / negative_example 入队 / escalation 卡死）各自加等价性测试；DEFAULT 下知识召回排序回归基线逐条不变；先补一个"业务结果兜底指标"避免静默污染无告警。

> **进度（2026-06-13 更新）**：Phase 2.5 设计硬交付**已全部落地**——三回路等价测（pre-2 / main-1~3）+ DEFAULT 逐字等价 + 业务结果兜底**观测**指标 `negative_reaction_rate`（pre-3，写 post_release 事件 details 供 admin 察觉错配，仅观测不判决）。
> **main-4（additive 增强，非设计硬交付）已落地**：在 C5 `auto_release` 在线决策路径加**默认关闭**的「客户负反应强制门」（`evolution_auto_release_negative_reaction_gate_enabled` / `evolution_auto_release_max_negative_reaction_rate`，默认 0.30 绝对阈值）。开启后 auto_release 判定放行**之后、实际 release 之前**，若回看窗口当前绝对负反应率超阈 → 强制 SKIP、退回 admin（拒绝自动放行，**非回滚**，不触碰 Req 9.7）。复用 pre-3 的 `compute_negative_reaction_rate`（同窗口、同极性源）。门关时字节等价。原计划落点 `significance`/shadow 被 pre-3 证伪（`ShadowReplay` 无 `outcome_status`），故改落 `auto_release`。

### Phase 3：引导层（核心想法，价值兑现）+ H15 + 清 C3
- AI 对话 + 文档 → 生成候选配置 → 审核 UI → publish。
- **H15** ✅**已完成（3A-1，拆 a/b/c）**：经营公式从 profile 注入。`DomainProfile.business_formulas: Vec<BusinessFormula{key,expression,display_name,eval_score_key}>`（3A-1a，纯 additive + seed 四公式逐字）；`/evaluations` 的 formulas 数组 + `score_key_for` 映射走 profile（3A-1b，会跨行业坏的真消费方）；公式散文**单一真相源**——建 `render_business_formulas_self_check`/`_json_example` 渲染函数（3A-1c-1），reviewer formulaBreakdown 走渲染（3A-1c-2），policy 内联公式段退役改运行时 `strip_legacy_formula_self_check_section`+`build_policy_formula_section` 自愈注入（3A-1c-3a/3b，**不 bump PROMPT_PACK_VERSION、不清运营编辑**）。playbook 中文公式段归 H12 `methodology_override` 已覆盖。护栏从「字节等价」放宽为「公式内容快照等价 + 全基线绿」。
- **清 C3** ✅**已完成（3A-2）**：`PLAYBOOK_METHODOLOGY_SYSTEM` 改写为**领域中性**（删「消费心理学/顾问式销售/异议/顾问朋友/机械营销」销售偏见词，加「不预设行业、行业语义来自运营输入」第 7 条）；`DomainProfile.methodology_generator_preamble: Option<String>` 让特定行业声明生成偏好，generate/optimize 两端点 None 回落中性 DEFAULT。C3 是引导层生成器（不在运行时决策路径），去偏见不影响现有销售运营行为。
- 前端向导。
- 端到端验证：用**两个非销售行业**跑通——(a) 一个有转化目标的行业（如教培）；(b) **情感陪伴**（"像男朋友一样"指令 → intimate_companion 模式 → 主动情绪价值触达 → 不被 PressureRisk 压制 → 优质回复能拿到正反馈进自学习）。

### Phase 3 后（客观事实增强 + 新驱动力，做加法专题）
- CRM 客观事实（§1.6 G2/G3/G4）：产品目录实体、订单关联 product_id、持有状态。
- 数字分身新驱动力（§3.7）：日历祝福 scan_calendar、社交钩子 scan_external_hook（先摸清 MCP 能力边界）。

### Phase 4（可选）：清理 D2/D3 审计与图谱缺陷。

---

## 7. 风险与护栏

| 风险 | 等级 | 护栏 |
| --- | --- | --- |
| 改 decision/gateway/prompt 核心路径破坏现有行为 | 高 | Phase 0 安全网 + DEFAULT_PROFILE 逐字等价 + 现有基线全绿才进下一期 |
| 对单一目标行业过拟合（违反反过拟合红线） | 高 | DEFAULT_PROFILE = 当前销售域配置，新行业只是「另一份 profile」，不改通用逻辑；引导层 prompt 不得写死任何具体行业词 |
| 召回算法天花板（字符匹配无语义） | 中 | **本设计不解决**，独立列出。术语专业化行业召回质量受限是已知上限 |
| 多租户隔离靠手写过滤、无框架强制 | 中 | 本设计不引入新跨租户查询；新集合沿用 workspace_id 模式 + 复用 ACL |
| LLM 生成配置质量参差 | 中 | 全部候选人审；审核 UI 逐项可改；版本可回滚 |
| 配置膨胀/维度爆炸 | 低 | 沿用 DomainSchema 的 fields≤64 类约束思路，profile_dimensions 设上限 |

---

## 8. 待你拍板的开放问题

1. **推进方式**：已定 = 先出本设计文档对齐 ✅。审完是否进 Phase 0？
2. **分工**：我统一推（知识库侧列清单 review）vs 严格分头？（§5）
3. **目标行业**：已定 = 纯通用、不绑定具体行业 ✅。但 Phase 3 端到端验证需要**一个真实行业样例**跑通，你指定一个？（仅用于验证，不写进代码）
4. **DEFAULT_PROFILE 去留**：长期是否保留销售域 DEFAULT_PROFILE 作为兜底，还是出厂即"空 profile + 强制引导"？（影响新部署首次体验）

---

## 9. 附：本设计不碰的红线（继承）

- AI 永不自动 verify（新知识/配置候选都需人审）。
- 画像更新须保守、禁止过拟合、系统性思维找最优杠杆。
- 新增测试只增量叠加，不删改旧维度/旧金标。
- check-no-human-takeover / check-no-model-hint / check-no-sales-domain 三闸。
- D2 锚定 quote→anchor verify gate 不削弱。
