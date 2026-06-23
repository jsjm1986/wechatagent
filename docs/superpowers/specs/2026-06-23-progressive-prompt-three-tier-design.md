# 渐进式三档提示词加载 + 信息充分性自评 设计

日期：2026-06-23
状态：待审
关联记忆：[[project_agent_first_no_keyword_filters]]、[[project_redline_pure_llm_gate]]、[[project_universal_test_coverage]]

## 1. 背景与动机

当前 user-ops 主回复路径（`src/agent/gateway.rs:910-938` → `src/agent/decision.rs:267` `decide_reply_with_promote`）是「组装巨型 prompt → 单程生成 → 五闸事后审」的流水线。`decision.rs:607-746` 的 user prompt 有 **30+ 个注入槽位**（playbook、域策略、状态机、运行参数、记忆、记忆卡、知识、知识路由、产品目录、权益、意图轨迹、用户反应、请示信号、运营偏好、画像、标签、阶段、意向、生命周期、价值层、承诺、跟进策略、资产、可引荐顾问、待办、最近聊天、最新消息……），**每一轮回复都全量拼装**。

三个真实问题：

1. **提示词膨胀**：私域大量轮次是日常寒暄/确认，却同样吞下产品目录、知识、意图轨迹等重型槽位 —— 噪音多、token 高、注意力被稀释。
2. **空转回复**：现有五闸全是「否定式」安全/质量闸（别编、别越界、别太销售、别像机器、别冷漠），缺「有效性」维度。一条安全且讨喜但零信息增量的回复（"我特别理解您的顾虑呢～"）能**高分通过全部五闸**。根因常是 AI 没取到信息就硬答。
3. **隐私泄露**：memory 里的内部画像（trustLevel / objections / 关系评判）被原样注入 reply prompt（`decision.rs:417-424`），且无任何 prompt 约束或运行期守卫阻止 LLM 把这些内部判断**复述给客户本人**（详见 §6）。

### 1.1 不是什么（范围澄清）

- **不复活 tool_loop**：`src/agent/tool_loop.rs` 是从未接生产的 `#[cfg(test)]`-only 死代码，正在被 `2026-06-23-tool-loop-dead-code-sunset-design.md` 删除。本设计**不依赖、不复活**它，是在现有 single-pass 路径上新建三档机制。与 sunset 计划无冲突。
- **不改 loop 形态为 N 轮 ReAct**：经讨论否决了「整条生成改成多轮 tool-loop」（N 倍 input、缓存失效、JSON 多轮稳定性风险）。本设计是「最多两程」：第一程瘦档自评 → 按需第二程升档重生成。
- **不动五闸的安全网**：factRisk / grounding / pressureRisk / humanLike / emotionalValue 五闸与所有硬门（状态机、action policy、verified-knowledge、协议字段、should_hold）全部保留。本设计作用在「生成」这一步的**输入侧**，五闸仍在「生成后、发送前」把关。

## 2. 核心机制：三档加载 + 充分性自评统一循环

经讨论收敛：B+（模型自主决定加载哪些上下文）与 C（信息不足时不硬答）**本质是同一个「信息充分性自评」的两个分支** —— "缺内部信息"就自己升档加载（B+），"缺客户那边的信息"就澄清/等待（C）。共用同一份自评输出，不是两套机制。

### 2.1 三档定义

| 档位 | 注入内容 | 适用场景 |
|---|---|---|
| **小档 (lean)** | 安全/身份恒注入集（见 §3）+ 最近聊天 + 客户消息 | 寒暄、闲聊、确认收到、简单情绪承接 |
| **中档 (relational)** | 小档 + 完整 memory（userUnderstanding/relationshipState/productFit/nextAction）+ 画像 + 标签 + 阶段/意向 + 意图轨迹 + 最近用户反应 + 运营偏好记忆 | 关系维护、轻咨询、需要「懂这个人」但不涉及产品 |
| **完整档 (full)** | 中档 + 知识切片 + 知识路由 + 产品目录 + 持有投影 + 疑似成交 + 可发素材 + 可引荐顾问 + 方法论 + 状态机 + 运行参数 | 推进、成交、异议处理、产品/价格/能力问询 |

### 2.2 统一循环（最多两程）

```
第一程：默认走【小档】瘦 prompt 生成
  Reply Agent 输出回复候选 + 一个【充分性自评】结构：
    - sufficiency: enough | need_more_context | need_clarification
    - missing_tier: none | relational | full   (need_more_context 时指明缺哪一档)
    - clarification_intent: <若 need_clarification，给澄清方向>
  ↓
分支：
  enough            → 直接进入五闸评审（多数寒暄轮走这条，1 次瘦档调用）
  need_more_context → 升到 missing_tier 指定的档位，第二程重新生成（B+）
                      （只升一级：小→中 或 小→完整，按 missing_tier）
  need_clarification→ should_hold + ai_waiting_for_more_context，或输出澄清问句（C）
                      不硬答、不碎答
```

- **B+ = 升档加载**：自评说"缺关系/画像"→ 升中档；"缺产品/知识"→ 升完整档。
- **C = 不硬答**：自评说"缺的是客户那边的信息（客户没说清）"→ 走 `ai_waiting_for_more_context`（该 hold 状态及其全链路 —— 常量 `types.rs:1124`、reviewer 校验 `gates.rs:766`、planner 分类 `planner/mod.rs:1209`、run_envelope 生命周期、前端 observability 面板 —— **均已就绪**，本设计只补「触发源」）。
- 触发权**全部归 LLM 自判**（agent-first），不引入确定性规则/词表。`knowledge_coverage`（`knowledge_router.rs` 已产出 missing/weak/enough）作为**兜底观测**：若 LLM 自判 enough 但 coverage=missing 且本轮需产品知识，记一条观测 telemetry（先观测后判罚，不强拦），用于日后校准自评可靠性。

### 2.3 成本画像（正面回应缓存/成本关切）

- **寒暄/简单轮（多数）**：1 次小档调用，**显著低于**现状每轮全量巨型 prompt。
- **复杂轮**：2 次调用（小档自评 + 升档重生成），但第二程才注入重型槽位，且只在真需要时。
- **平均成本大概率低于现状**。缓存：小档 prompt 高度相似（寒暄态趋同），命中率反而可能优于当前每轮都不同的巨型 prompt。

## 3. 安全槽位恒注入铁律（关键不变量）

**安全/身份类槽位在三档里恒定满注入，可瘦的只有「关系类」和「业务类」。**

恒注入集（任何档位都不删）：
- soul（人格本体，`decision.rs:292`）
- system_contract、policy（**含 boundary_protection 边界硬规则**，`decision.rs:484-501`）
- operator_instruction（运营对该客户的最高优先级特别指令，如"已签约别推销"，`decision.rs:556`）
- business_context（行业业务上下文，`decision.rs:575`）
- memory card 里的 **doNotDo / commitments / deprecated_facts**（已承诺、禁止项、已过期事实，`decision.rs:389-416`）
- 最近聊天 history（`decision.rs:586`）
- 客户最新消息（`decision.rs:745`）

**理由**：降档判断有不确定性。把可瘦切分限定在「非安全必需」槽位后，最坏的降档失误结果是「回复得不够丰富/信息不足」（→ 走 §2.2 升档重答，可恢复的质量问题），而**不会**是「违背承诺/越界/泄露内部信息」（不可恢复的安全事故）。降档失误必须是可恢复的。

实现上：把恒注入集与「可瘦的关系类/业务类」在代码里显式分组，三档函数只在后两组上做增量，恒注入集是所有档位的公共基底。

## 4. 隐私/边界维度（并入本轮）

两处叠加（双管），均为 LLM 语义判断，**非词表**（遵循 [[project_redline_pure_llm_gate]]：用户已多次否决给红线加运行期关键词守卫）：

1. **注入侧硬约束**：在 reply prompt 注入一条硬约束 —— "memory 中对客户的内部画像、信任度评分、异议清单、关系阶段评判等属内部判断，**不得向客户复述或暗示**；只能用于指导你的措辞与策略。" 放在恒注入的 policy 层或紧邻 memory 注入处。
2. **reviewer 增一维**：reviewer 评分增加一个「边界/隐私安全」语义维度，判断候选回复是否：(a) 泄露对客户的内部画像/评判；(b) 暴露 AI 身份（违反全自治"客户永远只跟 AI 对话"）；(c) 暴露幕后决策源（领导）存在或内部系统信息。失败 → hold（`blocked_by_safety_guard`）。

与三档天然协同：小档本就不注入完整内部画像，复述风险随之下降；完整档注入画像时，注入侧约束 + reviewer 维度兜底。

> 注：知识库下钻路径跨 account 越权读取（`open_chunk`/`follow_relations`/`resolve_superseded` 漏 account 过滤，见 §6）是**独立工程漏洞**，与本设计的评审/生成机制无关，单列为后续专项，不在本轮范围。

## 5. 组件与数据流

### 5.1 改动点

| 组件 | 改动 |
|---|---|
| `AgentDecision` / `RawAgentDecision`（`types.rs`） | 新增充分性自评字段：`sufficiency`（enum）、`missing_tier`（enum）、`clarification_intent`（String）。全部 `#[serde(default)]` 向后兼容。 |
| `decide_reply_with_promote`（`decision.rs`） | 拆分槽位准备为三组（恒注入 / 关系 / 业务）；新增 `tier: PromptTier` 参数控制注入哪些组；prompt 模板加充分性自评输出指令。 |
| `gateway.rs` 主路径（:910-938） | 实现两程循环：第一程小档 → 读自评 → 分支（直接进闸 / 升档第二程 / hold-clarify）。 |
| reply prompt 模板（`user.reply.*`） | 加充分性自评输出契约 + 隐私硬约束。bump `PROMPT_PACK_VERSION`。 |
| reviewer（`review/mod.rs` + prompt） | 增「边界/隐私安全」维度（§4.2）。 |
| 充分性自评判定 | 抽纯函数 `decide_tier_escalation(self_assessment, knowledge_coverage) -> TierDecision`（可单测，覆盖 enough/升档/澄清三分支 + coverage 兜底观测）。 |

### 5.2 数据流

```
inbound → gateway 主路径
  → 第一程：decide_reply_with_promote(tier=Lean)
      恒注入集 + 最近聊天 + 客户消息 → LLM → (候选回复, 充分性自评)
  → decide_tier_escalation(自评, knowledge_coverage)
      ├─ Enough         → finalize/五闸（不变）
      ├─ Escalate(tier) → 第二程：decide_reply_with_promote(tier) → finalize/五闸
      └─ Clarify        → should_hold + ai_waiting_for_more_context（或澄清问句）→ 不发送/轻发送
  → 五闸评审（含新增隐私维度）→ outbox（不变）
```

## 6. 已知约束与既有事实（审计佐证）

- `ai_waiting_for_more_context` 全链路已就绪（§2.2），本设计只补触发源 —— 接线，非新建。
- `knowledge_coverage`（missing/weak/enough）`knowledge_router.rs` 已产出，当前只喂 grounding 风控，本设计将其作为自评兜底观测接入。
- 主回复路径当前**不跑前置 planner**（`gateway.rs:894-899` `initial_planner` 是写死占位）—— 故 B+ 的「规划」无现成 planner 可复用，落点选择「复用 C 逆法做规划」（第一程自评即规划），不新增独立规划调用。
- 隐私审计确认（佐证 §4 必要性）：内部画像原样注入无脱敏（`decision.rs:417-424`），无 prompt 约束阻止复述（`prompts.rs:847` 只禁暴露 AI/系统，不禁复述内部画像），reviewer 五维无身份/隐私维度（`types.rs:1043-1060`）。幕后领导泄漏守卫 `relay_output_leaks_internal_payload`（`escalation/logic.rs:188`）只在 relay 路径跑，主路径裸奔。

## 7. 测试策略（遵循 [[project_universal_test_coverage]] 铁律：测试不改生产 prompt/guards 逻辑，纯函数确定性测 + 真模型测能力）

1. **纯函数**：`decide_tier_escalation` 三分支 + coverage 兜底观测 + 默认小档，lib 单测。
2. **恒注入铁律**：断言三档函数在任意 tier 下，恒注入集（soul/policy/operator_instruction/doNotDo/commitments/history/客户消息）**始终存在**；只有关系/业务组随 tier 增减。这是安全不变量，必须有锁死测试。
3. **向后兼容**：`AgentDecision` 新字段缺省反序列化（老 run JSON / Mongo 老数据）不破坏。
4. **真模型**：寒暄轮走小档 enough、产品问询轮升完整档、客户没说清走 clarify —— 真 LLM 行为测（多 seed，不接受 skip 假绿）。
5. **隐私维度**：构造「回复复述了 trustLevel/objections」的候选，reviewer 隐私维度应判失败 → hold。
6. **基线**：lib ≥ 350 / PBT ≥ 33 不回归（新增测试只增量叠加）。

## 8. 风险与权衡

- **降档判断抖动**：三档比二档省，但边界判断更易抖（小档判够了但其实该带知识）。缓解：§3 铁律保证抖动后果可恢复；§2.2 coverage 兜底观测量化自评可靠性，数据够了再决定是否收紧。
- **两程延迟**：复杂轮多一次 LLM 往返。权衡：仅复杂轮付，且换来更聚焦的生成 + 治本的空转修复；多数轮次反而更快更省。
- **自评乐观偏差**：Reply Agent 自评「信息够了」可能偏乐观（利益相关方）。缓解：coverage 兜底观测 + 五闸仍在生成后兜底；未来可叠加「生成者×审查者数值背离检测」（方案 A，本轮不做，留待自评数据积累后）。
- **prompt 契约变更**：bump `PROMPT_PACK_VERSION`，注意 `reset-system-pack` 语义（非每启动覆盖）。
