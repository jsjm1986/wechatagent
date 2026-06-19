# 通用化改造后的测试覆盖缺口体检

> 2026-06-16 体检。背景：universal-domain-adaptation（Phase 0→2.5）把硬编码销售世界观抽象成可配置 `DomainProfile`，让同一 agent 适配任意行业（销售/情感陪伴/同行/朋友/数字分身）。本表系统对照「通用化引入的能力」×「现有测试覆盖」，定位缺口与优先级。**只体检不改码。**

## 一、核心结论

通用化是**能力侧的深改造**，但**测试侧基本停留在改造前**：

1. **运营 agent 全能力测试（`real_llm_ops_smoke` t4–t18，15 个）100% 跑在销售域**——contact 无 profile（走 DEFAULT 销售），judge 写死「请基于微信私域销售运营语境打分」（`real_llm_ops_smoke.rs:542`）。这套是通用化**之前**写的，只回答「销售域 agent 行不行」。
2. **通用化新能力主要靠 DEFAULT 等价性单测护着**——证明「改造后销售域字节不变」（反过拟合护栏，有价值），但**几乎没有「激活非销售 profile 后 agent 行为真的随域改变」的真模型行为测试**。
3. **`domain_profile_e2e`（7 个）全是 CRUD/生命周期**（create/update/publish/activate/delete/generate/生成第二行业），**测的是 profile 数据对象能否增删改查，不是激活后 agent 行为是否真的变**。
4. **非销售域真模型行为测试只有情感陪伴一个域**——`roleplay_emotional_companion_e2e` + `roleplay_reviewer_pressure_calibration`（近两日刚建/优化）。同行/朋友/数字分身等其它定位**零真模型覆盖**。
5. **judge 标尺单一**：rubric 写死销售语境（成交准备度/推进业务）。拿它评情感/陪伴回复，「没推进成交」会被系统性误判 → judge 本身需要 profile 化。

## 二、能力 × 测试覆盖矩阵

性质：**等价**=只有 DEFAULT 销售域字节等价单测；**跨域行为**=有「换 profile 行为变」的测试；空=无。

| 能力 | 作用 | DEFAULT 等价单测 | 跨域行为测试 | 缺口 |
|---|---|---|---|---|
| **H11 自学习极性** outcome_polarity | 正/负极词表渗透召回排序+反向训练+escalation 三回路 | ✅ 等价性锁 | ❌ | **P0 最深命门**：跨域语义错配静默污染召回，无业务指标告警 |
| **C2 operation_state 派生+校验接回** | FSM 态派生自 customer_stage，接回 check_state_transition | ✅ state_transition_pbt | ⚠️ 仅 DEFAULT FSM | **P0**：唯一「等价→真行为变更」点，非法迁移拒写/审计互斥回归风险最高 |
| **H14 grounding 硬闸条件化** | bypass_without_claim + per-msg claim 分析，影响 send/block | ✅ | ⚠️ 情感域间接 | **P1**：安全闸语义变更，需验销售无条件硬闸不被削弱、情感域旁路只在 claim=false 生效 |
| **H8/H19 运营范式+作息门控** | 控制主动触达是否发生（funnel/silence/commitment/quiet_hours） | ✅ | ❌ | **P1**：跨域行为差异最大，错配→该发不发/不该发骚扰 |
| **H2/H1 维度动态化** decision_dimension_kinds | 通用化地基，维度集错→下游连锁失灵 | ✅ | ❌ | **P1**：typed↔容器双写不一致高发区 |
| **H12 人格覆盖** soul/methodology_override | 替换销售人设为任意行业 | ✅ None 回落 | ⚠️ 情感域 e2e 间接 | P2 |
| **H15 经营公式 profile 化** business_formulas | 四公式注入 reviewer/evaluations | ✅ | ❌ | P2 |
| **H17 记忆维度 schema** memory_dimensions | memoryCard 槽位随 profile | ✅ + H17-e 情感域 | ⚠️ 情感域 | P2（相对好） |
| **H4/H5/H16 词表/completeness/chunk 角色** | 承诺词/审计维度/切片角色配置化 | ✅ 各等价 | ❌ | P2 |
| **M2 五闸阈值覆盖** threshold_overrides | 逐字段覆盖 runtime | ✅ + ProfileEditor 前端 | ❌ | P2 |
| **t4–t18 运营全能力** | 跟进/状态机/产品门/画像/记忆/可操控/千人千面/多轮弧 | — | ❌ 全销售域 | **P0 体系级**：能力测试与「适配任意行业」定位脱节 |
| **judge rubric** | 评分标尺 | — | ❌ 写死销售 | **P0 横切**：非销售域评分系统性误判 |

## 三、缺口优先级（建议修复顺序）

**P0（先做，撬动面最大）**
1. **judge profile 化** — 让评分标尺随域走（销售看成交准备度、情感看情绪承接/边界尊重、同行看专业互惠）。这是横切：不修它，任何非销售域能力测试的分数都不可信。最高 ROI。
2. **建非销售域端到端能力测试骨架** — 复用 t4–t18 结构，激活情感陪伴/数字分身 profile，验证「同样的能力（画像/可操控/多轮）在非销售域行为正确」。先 1 个域跑通骨架。
3. **H11 极性 + C2 状态派生的跨域行为测试** — 两个最深命门补真模型/集成行为覆盖（不止等价性）。

**P1（地基与安全闸）**
4. H14 grounding 跨域、H8/H19 触达门控跨域、H2 维度动态化双写一致性。

**P2（配置化能力）**
5. H12/H15/H16/H17/M2 各补「激活后行为变」的最小验证，多数已有等价性+部分前端，缺真模型行为这一环。

## 三点五、业务闭环对齐（测试必须懂真实业务逻辑，否则空测）

> 来源：`docs/real-task-runbook.md` 北极星四问 + `docs/agent-policy.md` 自动发送约束。测试断言必须对齐**真实业务契约**，而不是只验"链路 Ok / 状态闭集"这种与业务无关的形状。

**真实业务闭环（自运营全链，权威定义）**：
```
webhook 入站 → 决策(Reply Agent) → 独立 Review → 改写(≤1次) → outbox(幂等) → MCP 真实送达
  → 画像更新 → 记忆固化 → 承诺/跟进任务 → 状态机迁移 → Planner 主动触达 → (冷启动复活)
全程无人值守，hold/block 全走 AI 内部状态名
```

**现有测试的闭环盲区**：t4–t18 多数只覆盖**前半段单轮**（webhook→决策→review→单条 reply），断言落在 `status ∈ 闭集`。**后半段长程闭环**（送达后画像/记忆/承诺→Planner 主动触达→冷启动）测得很薄——只有 t11(记忆整理)/t15-t18(多轮弧) 沾边，且断言弱。这是"测了流程的壳、没测业务的实"。

**每个能力对应的真实业务契约（断言应对齐到这一层，非形状层）**：

| 能力 | 真实业务逻辑（断言该验什么） | 当前断言 vs 业务契约的差距 |
|---|---|---|
| 可操控性(t12) | 运营写"先问预算"=对该客户的**业务策略覆盖**，agent 必须在推进前真的执行该约束，且**不丢情感温度**(policy: 情绪价值是每轮硬要求) | 只 eprintln 是否问预算；没验"执行指令的同时温度达标"(judge emotionalValue) |
| 产品门(t6) | policy 红线：未 verified 知识**禁止**报价格/承诺/数据 → 必须 `blocked_unverified_product_claim` 或 reply 不含具体数字 | 有条件断言但 None 分支放过；该强制"要么 sent 且无数字、要么 blocked" |
| autonomy(t8/t17) | 产品定位红线：**永不转真人**，超职权走幕后请示再 AI 转述 → 任何"安排真人/转人工"承诺即违规 | 只 eprintln；必须命中即 fail(已在真实性审计 P1) |
| 状态机(t5) | policy: 每次决策必输出 operationState，且迁移必须在状态机字典内(fail-soft 不阻断但记审计) | if let Some 守卫放过 None；该强制产出合法 state |
| 多轮闭环(t15-18) | 自运营后半段：送达→画像更新→记忆固化(version bump)→承诺跟进→不冷启动复读→Planner 主动触达 | approved_turns≥2 等弱断言；缺"画像/记忆/承诺真的随轮推进"的闭环断言 |
| quiet hours | policy: 静默时段入站不立即回、排 deferred 任务、醒来一次性回；主动发送重排不取消 | 现有真模型测试**完全没覆盖**(只有 quiet_hours_deferral 集成测,非 LLM 业务流) |
| 自学习闭环(H11) | 用户正反应→Hit 喂回召回置信度；负反应→Block;沉默=删失不当负例 | 零真模型行为测试,只有纯函数单测 |

**结论**：新增/改造测试时，断言必须问"这条验的是**真实业务行为**还是只是**链路形状**"。空测试的特征 = 即使 agent 业务行为完全错误（转真人/报假价/丢温度/画像不更新）测试照样绿。

## 四、反过拟合 / 边界提醒

- 现有 DEFAULT 等价性单测**不要动**——它们是「换行业不破坏销售域」的护栏，是资产不是缺口。
- 新增测试遵循 [[no-overfitting-methodology]]：断言契约级（行为随域**有差异**、状态闭集、红线守住），不点对点锁单条回复措辞。
- judge profile 化要复用现有 DomainProfile.business_formulas / coverage_dimensions 作标尺来源，不另起一套（[[agent-first-no-keyword-filters]] 同源：配置驱动非硬编码）。
- 知识库相关测试（recall/quality/knowledge_*）可能与他人职责线重叠，动前确认边界（[[division-of-labor]]）。
