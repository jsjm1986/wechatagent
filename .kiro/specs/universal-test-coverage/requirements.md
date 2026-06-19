# 通用化后测试体系重建 — 实施 Spec

> 2026-06-16。配套体检：`docs/universal-domain-test-gap-audit.md`（覆盖缺口）+ `docs/real-llm-test-authenticity-audit.md`（真实性定级）。

## 背景与目标

通用化改造（universal-domain-adaptation Phase 0→2.5）让 agent 适配任意行业（销售/情感陪伴/同行/朋友/数字分身），但测试体系停在改造前：t4–t18 全跑销售域、judge 标尺写死销售、非销售域行为零真模型覆盖、且大量测试可被 skip/eprintln 吞成假绿。

**目标**：建立「全域覆盖 + 真实 LLM 业务流程 + 业务行为对齐」的测试体系，让「适配任意行业」从未验证声明变成可持续验证的事实。

**北极星（验收总纲，源自 `docs/real-task-runbook.md`）**：每个被测能力，在每个目标域下，都能用真实 LLM 跑完真实业务闭环，且断言对齐真实业务契约——**即使 agent 业务行为全错（转真人/报假价/丢温度/画像不更新）也必须能让测试变红**。

## 核心原则（贯穿所有阶段）

1. **真实 LLM，不接受假绿**：涉及 LLM 的测试必须真发真模型、真断言。skip/judge失败/缺 key 不得静默变绿。
2. **业务行为对齐，非链路形状**：断言验「业务契约」（红线守住/画像真更新/温度达标/状态合法迁移），不验「status ∈ 闭集」这种与业务无关的壳。
3. **反过拟合**（[[no-overfitting-methodology]]）：断言走契约级（行为随域**有差异**、红线命中即 fail），不锁单条回复措辞。
4. **agent-first**（[[agent-first-no-keyword-filters]]）：judge 标尺、身份生成都走配置/LLM 驱动，不硬编码关键词。
5. **DEFAULT 等价单测是资产不动**：它们护「换行业不破坏销售域」。

## 阶段与验收标准

### R0 · P0 总开关：堵假绿根因（地基，先做）

**R0.1 缺 key 即 fail**：CI 每个 real-llm job 在跑测试前显式断言 `REAL_LLM_API_KEY`（及 judge key）非空，缺失 → job fail。不再靠 test 内 `require_real_llm!` 静默 skip 让套件全绿。
- 验收：CI 故意清空 key 时对应 job 红；有 key 时正常跑。

**R0.2 transient-skip 可观测化 + 上限**：`unwrap_or_skip_transient!` 每次 skip 把原因/计数落 ledger；CI 校验单 job skip 率 ≤ 阈值（如 30%），超限 job 红。
- 验收：构造高 skip 场景 CI 能红；正常运行 skip 率达标绿。

**R0.3 judge/reviewer 端点 4xx 不当抖动吞**：`http_4xx`（401/402 账户级除外）不进 transient-skip，直接 fail（暴露端点配错，如漏 /v1 的 405）。reviewer 调用同理。
- 验收：端点配错时测试红而非 skip 绿（405 案例不再假绿）。

### R1 · judge profile 化（横切 P0）

**R1.1 judge 标尺随域走**：judge system prompt 的评分维度/锚点从 active DomainProfile 的 `business_formulas` + `coverage_dimensions` 派生，而非写死「微信私域销售运营语境」。
- 销售域：维持现有标尺（成交准备度等），作基准对照。
- 情感陪伴域：标尺=情绪承接/边界尊重/陪伴质量，「没推进成交」不扣分。
- 验收：同一条情感陪伴回复，profile 化 judge 给分合理（不因「没成交」被误判），销售 judge 仍按销售标尺；两域标尺确有差异。

**R1.2 judge 失败语义分级**：以 judge 为唯一质量门的测试，judge 失败 → fail（不静默 pass）；红线类测试（不依赖 judge）judge 失败仅丢观测可接受。

### R2 · 非销售域全链长程闭环能力测试 + 随机身份生成器

**R2.1 LLM 随机身份/场景生成器**：用 LLM 生成多样 agent 身份（行业 × 性格 × 诉求随机组合）+ 对应客户场景。主动暴露固定样本测不到的场景不足。生成的身份落 DomainProfile + Contact。
- 验收：一次跑能覆盖 N 个随机身份；生成器可复现（seed 可控）；身份覆盖 ≥4 大类（销售/陪伴/社交/正式业务）。

**R2.2 全链长程闭环断言**（非单轮）：覆盖 `webhook→决策→review→改写→outbox→MCP 送达→画像更新→记忆固化→承诺/跟进→状态机迁移→Planner 主动触达→冷启动复活`。断言对齐业务契约（见缺口表「业务闭环对齐」节）：
- **画像/记忆更新走条件式断言（业务审纠正：非"必更新"）**：[[feedback_cautious_profiling]] 红线=不因一句话盲目画像、写侧无置信门待修。故断言改为「**当且仅当** `consolidation_needed=true` / 出现足够置信新事实时才更新；单句寒暄/闲聊轮**不应**触发画像字段变更」+ **反向断言**「无新信息时画像保持不变」（防过度画像，对齐保守画像红线，绝不把"必更新"做成红断言去固化写侧缺陷）。
- 承诺产生跟进任务、状态机合法迁移（合法 stage 转移走字典校验）
- Planner 在静默后真的主动触达、冷启动真的复活
- 全程 hold/block 走 AI 内部状态名、无转真人
- **情绪温度每轮硬门**（业务审补，prompts.rs:797）：执行运营指令/推进的同时 `emotionalValue ≥ 阈值`；异议/焦虑/沉默轮情绪承接优先于信息密度。
- **不暴露 AI 固定红线**（业务审补，policy:53）：被直接问"你是不是 AI"时不复读身份模板、不哑火（命中露馅模式即 fail）。
- **casual_relationship/寒暄模式不推产品**（业务审补，prompts.rs:453/764）：该模式下不主动推产品/不灌信息。
- 验收：每条断言能在 agent 行为错误时变红；多轮弧 approved/sent 真实达标。

**R2.3 跨域行为差异**：同一输入在对立 profile 下行为实质不同（销售推进 vs 陪伴承接 vs 同行互惠），非仅逐字不等。

### R2.5 · 自运营主动半场 + 治理红线（t4–t18 框架装不下的全新维度）

> t4–t18 全是「被动响应单轮」框架。以下三个维度是真实业务闭环里 t4–t18 **零真模型覆盖**的全新角度——不是换域、不是加断言，是被动框架装不下的主动/治理半场。源：`docs/agent-policy.md` 作息门控 + `docs/real-task-runbook.md` 北极星「自运营=Planner 主动触达」+ `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`。

**R2.5.1 作息门控（quiet hours）真模型业务流**：验「像真人睡觉时不回、醒来一次性回完」。静默时段入站→不立即回+排 `deferred_inbound_reply`+写 `quiet_hours_deferred_inbound` 事件；连发去重（同 contact 仅 1 条 wake）；醒来基于累积消息走完整决策链回 1 次；静默时段主动发送**重排不取消**。
- 验收：静默窗内 inbound 不产生 outbound 且排了 deferred 任务；醒来后真的回；时区走整数运算不依赖宿主。错误时（半夜真发了/醒来不回）能变红。

**R2.5.2 Planner 主动触达（自运营主动半场）**：验 agent **主动**找用户的全链——承诺到期跟进（`commitment_due`）、沉默催进（`silent_followup`）、按对话模式选触达内容。这是「无人值守自运营」被动响应之外的另一半。
- 验收：构造「承诺到期/用户沉默」状态，Planner 真的产出主动触达 run 并走完 gateway→outbox→送达；触达内容合理（不打扰、贴画像）；quiet hours 内重排。错误时（该催不催/骚扰）能变红。

**R2.5.3 幕后请示通道（principal decision channel，治理红线命门）**：验产品定位红线「永不转真人，超职权走幕后领导请示、拿回结论 AI 转述」。遇到超出 agent 职权/能力的事项→触发请示通道→拿到决策源结论→AI 用自己口吻转述，**全程不暴露真人、不说"转人工/找领导"**。
- 验收：超职权场景真的走请示通道（escalation）而非硬答或转真人；转述话术过 `check-no-human-takeover` 禁词；relay 不泄露「这是真人决定的」。这是「无人工接管」定位的命门，错误时（承诺转真人/暴露幕后）必须变红。

**R2.5.4（可选，低优先）并发/多账号鲁棒性**：t4–t18 单 contact 串行；补多 contact 并发、多账号轮休（round_robin）下的 claim 锁、outbox 幂等不串台。属基础设施压测，优先级低于上述三个业务维度。

### R5 · LLM 驱动动态博弈范式（详见 `docs/test-paradigm-llm-driven-analysis.md`）

> 现有所有「多轮」测试客户台词 100% 写死，博弈链是断的——客户不真实反应 agent，turn-level judge 测不出累积业务价值。这是测试范式的天花板。

**定位修正（2026-06-16 可行性分析 + 对抗审查后）**：经核实，「全部转动态当回归主力」依赖三个不成立的前提——①seed 不可复现（`llm.rs` 4 处 temperature 硬编码无 seed 通道，中转网关+高温下不保证确定）②三角色易同源（judge 默认回落 `state.llm` 与 agent 同模型）③成本爆炸（单场景~43 次 LLM 调用，全矩阵上千次/run，进不了 PR 合并门）。**故范式不是「动态取代固定」，是「动态发现 + 固定回归」两阶段流水线**：
- **动态发现层（nightly/手动，LLM 演客户）**：跑真实博弈**发现** agent 短板/新失败模式——固定脚本永远发现不了，这是动态的核心价值。进 ledger 观测 + 软门，**不进 PR 合并门**（成本+flaky）。
- **固定回归层（PR 门，确定性）**：①确定性业务逻辑（状态机迁移/画像更新/承诺→任务/记忆固化/quiet hours 时间运算）**保留固定脚本**——动态化纯属自找 flaky，是减分；②红线契约硬断言（出现转真人/报未验证价格即 fail）；③动态发现的好场景**固化**进来（见铁律①：只固化红线契约，不固化措辞）。
- **复现机制改 transcript 回放**（非 re-seed）：动态对话存档客户侧台词成 fixture，调试/复现时回放存档（客户不再调 LLM），只让 agent 重跑——精确定位"agent 这次为什么变"。

**R5.0 反过拟合四铁律（前置根约束，违反任一条 = 整个 R5 作废；详见 [[feedback_dynamic_test_anti_overfitting]]）**：
> 试金石：问题虽 LLM 生成，但若 prompt/方法论**真正普适正确**，本就该应对任何这类场景——所以正确修复=让方法论在抽象层更对。
1. **不固化对话当标准答案**：动态失败只用来**抽象方法论/prompt 的普适缺陷**，对话是"问题的例子"可弃；固化只固化红线契约（品类级跨对话成立），不固化措辞。
2. **修复改抽象层 + 多变体验证泛化**：改完让 roleplayer 把"同一类刁难"生成几个**语义不同的变体**（非 re-seed，llm.rs 无 seed 通道）重跑；全过=真提升，只原条过=过拟合回去重抽象。**动态范式天然支持泛化检验，是它相比固定脚本的核心优势**。
3. **评测器靠人工金标锚定，不朝结果迎合调**：judge 只能朝"更接近人工标注"改，不朝"让 agent 看起来更好"改（=驯化考官）；roleplayer 难度由人设契约定义，不由"agent 能否扛住"定义。judge 须先有人工金标+相关性达标才进软门，否则只进 ledger。
4. **守过拟合最终责任在 Claude 判断**：能看全对话、改也是 Claude，每次改前自问「修一类还是迎合一条」；单次低分只观测，跨多样本同类失败复现≥N 次才动 agent。

**R5.0-机械门（四铁律落地为可机检约束，非纯自律——终极审查方法论维度要求；详见 tasks R5.0）**：自律是"红线中的红线"不能只靠主观，补五道机械门：①**control set 阴性对照**：冻结正常/友好场景黄金集，每次抽象修复后回归，正常场景退化即打回（防误伤，[[feedback_no_overfitting]] 撒娇教训）②**变体 pre-registration**：抽象假设+验证变体在改 prompt 的 diff 前冻结、来自独立对抗库（斩确认偏误）③**先证伪根因再验修复**：同根因不同表面的场景修复前也应失败（防抽错方向假阳性）④**held-out 对抗集**：Claude 修复时看不到⑤**diff 机检**：禁 prompt 新增 few-shot 与失败对话字面/语义高重叠。统计机制（金标≥30/复现≥N）须与 nightly 跑量对账，撑不起的降级为观测不冒充统计结论。

**R5.0.1 三角色异族硬门（硬 fail）**：roleplayer / agent / judge **强制三个不同 provider/模型家族**，三方指纹写 report 且**同源/回落 state.llm → job 红**（非仅观测）。锁死 roleplayer/judge 禁止回落 `state.llm`（缺独立 key 直接 fail 非降级）。需先定异族判定清单（不同基座+不同机构+不同 RLHF，至少禁同 provider 同系列）。承认异族门防不了生态级共享盲区——补人工对抗注入（模型生不出的刁难）+ 三家族对同一 agent 行为高度一致且全正面时警惕共谋盲区。agent 用 claude-opus-4-8、judge 用 gpt-5.4（已验证），**roleplayer 第三族 key 待定（R5-T0 落实，现不存在）**。

**R5.1 LLM Roleplayer（演客户，落地 roleplay-fuzz 设计 P3）**：客户由 LLM 动态扮演——给定身份/场景/目标，**每轮真实反应 agent 上一句**（agent 接得好就软化推进、接不好就升级刁难）。roleplayer 只看对话历史不看 agent 内部决策（防作弊）。有明确人设契约+场景目标（防"乱演"）。需新增 temperature 可配的测试侧 client（生产 `LlmClient` temperature 硬编码 0.2，roleplayer 需 ~0.8）。
- 验收：同一身份下 agent 不同表现导致客户不同后续（博弈链通）；roleplayer 不越出人设；transcript 可存档回放复现。

**R5.2 Trajectory Judge（轨迹级业务价值评判）**：judge 评整段对话而非单条——「这 N 轮里：信任是否累积 / 意向是否推进 / 关系是否前进 / 全程红线是否守住 / 人设是否一致 / 像不像真人长期关系」。业务结果导向，对齐 5 大方法论公式（Trust/ConversionReadiness/EmotionalValue 的轨迹变化）。
- **前置校准硬门（R5.0 铁律③）**：trajectory judge 比 turn-level 更抽象、方差更大、最易"看着很懂其实在编"。投用前必须有一批**人工标注金标 trajectory**（高/中/低各若干段），测 judge 与人工的相关性（Spearman）+ 重测稳定性。**相关性达标前，trajectory 分只进 ledger 观测，不进任何软门**。
- 验收：judge↔人工金标相关性达标；一段"逐轮都还行但整体没推进关系"的对话能被识别为低分（turn-level 测不出）；红线全程任一轮破即整轨迹判负。

**R5.3 动态对抗（roleplayer 主动刁难）**：roleplayer 主动试探 AI 身份/嘲讽模板/诱导越界/情绪反扑，并**跟随 agent 的失误升级**——测「守红线的方式是否仍像真人」而非「红线字样有没有出现」。覆盖业务分析的 5 大复合场景（身份试探+要真人 / 愤怒升级逼问 / 空知识库追问 / 情绪未平问价 / 翻供比价）。
- 验收：agent 露馅（复读身份/官腔/哑火）时客户真的升级且 trajectory judge 扣分；agent 守住且有温度时客户软化。

**R5.4 跨会话长期关系弧**：同一客户多次会话间真有记忆/画像/承诺沉淀（非手工预置假数据），测"老客户回来 agent 记得上次、承接历史"。
- 验收：会话 2 的 agent 行为依赖会话 1 真实沉淀的记忆/画像；断了沉淀（清记忆）行为应退化。

### R3 · 深命门跨域行为（H11 自学习极性 / C2 状态派生）

**R3.1 H11 自学习极性跨域**：正反应→Hit 喂回召回置信度、负反应→Block、沉默→删失（不当负例），在非销售域语义正确（极性词表随 profile，非写死销售）。gap-audit 列为 P0 最深命门（单一真相源横向渗透召回排序+反向训练+escalation 三回路）。
- 验收：非销售 profile 下正/负/沉默三类反应分别正确分类为 Hit/Block/Censored；极性错配能被检出。

**R3.2 C2 operation_state 派生跨域**：operation_state 派生自 customer_stage 接回 check_state_transition，非法迁移拒写+审计事件，在非销售 FSM 下正确（fail-soft：illegal 不阻断已发送，跳过 state 写+发审计事件）。
- 验收：非销售状态机下合法迁移成功、非法迁移被拒且发 `operation_state_transition_rejected` 审计、已发送回复不受影响。

### R4 · 知识库域适配（运营+知识库都做）

**R4.1 召回基准补真断言**：recall_benchmark_cross_industry/smoke/maintenance 加 recall@k 下限、跨轮稳定占比、漂移率上限（堵「自称基准却零硬断言」）。
**R4.2 知识问答跨域**：知识库问答/抽取/完整度在非销售域语义正确。

## 反过拟合 / 边界

- 红线硬断言走「命中禁忌即 fail」（转真人/泄露系统提示/报价格数字），不锁措辞。
- 离线纯函数单测、mock 集成测不动（定位正确，确定性地基）。
- 知识库测试动前留意他人在跑的（[[division-of-labor]] 本工程已授权扩到知识库，但避免撞车）。
- 端点用 rsxermu666.cn 不限流（主 claude-opus-4-8 / judge gpt-5.4）。

## 阶段顺序与里程碑

**两条线并行，不是一条队列：**
- **固定回归线（PR 合并门，先做，守下限）**：R0 总开关 → R1 judge profile化 → R2 确定性业务闭环（状态机/画像/承诺/记忆/quiet hours，固定脚本）+ R2.5 主动半场/治理命门 → R3 深命门 → R4 知识库。这条线全程可复现、进 PR 门、是回归锁。**t15-18 等现有固定脚本不退役**，作确定性回归基线保留。
- **动态发现线（nightly/手动，后做，探下限之上的能力）**：R5.0 四铁律+R5.0.1 异族硬门（前置）→ R5.1 Roleplayer → R5.2 Trajectory Judge（先人工金标校准）→ R5.3 动态对抗 → R5.4 跨会话。这条线进 ledger+软门、**不进 PR 门**、用于发现固定脚本测不出的对话质量/抗刁难短板。

**两线连接点**：动态发现线跑出的真短板 → 按四铁律抽象成方法论缺陷 → 修 prompt → 多 seed 变体验证泛化 → 好场景的**红线契约**固化进固定回归线。动态负责发现未知，固定负责守住已知。

**归因纪律（继承 roleplay-fuzz 设计 §3）**：动态失败必须按 8 层 suspected_layer（fixture/reply_agent/reviewer/gate/knowledge/roleplayer/judge/ci_provider）归因；每阶段只引入一个新变量，否则无法定位是 agent 退步还是 roleplayer/judge 抖动。先 fixed scene 跑稳，再逐步上 roleplayer（不一步到位）。

R2.5.3 幕后请示通道是「无人工接管」命门，治理红线优先级高，在固定回归线里可与 R1 并列前置。

