# 决策请示通道设计（Principal Decision Channel）

- **日期**：2026-06-05
- **状态**：已批准设计，待写实施计划
- **范围**：用户(私聊)运营 Agent 在遇到**超出自身职权/能力**的事项时，向幕后真人决策源请示、拿到决策后用 AI 口吻向客户推进的闭环通道。Phase 1 用户运营域内；不涉及群/朋友圈。
- **一句话**：把"Agent 撞墙"分成两类——缺知识走知识库闭环，缺决策/授权走本通道请示真人——并让真人的决策在合适时沉淀回知识库，使请示频率随时间衰减。

---

## 1. 背景与动机

### 1.1 用户意图（原话锚定）

> "我来修改这个转真人的业务逻辑，如果用户让我们转认真等运营 agent 处理不要的问题和事情，可以对用户说 这个事情我要与领导商量等类似 这样的回复（具体要根据场景灵活）——这个时候我需要让 agent 真实给人类发送相关异议 让人类来处理，但是这个地方的边际和交互方式 如何让真人更容使用，我们需要头脑风暴一下。"

拆解：当客户提出 Agent 不该/不能独自处理的事项时，Agent 应当 (a) 给一个**场景化**的灵活回复（例如"这个事情我要与领导商量"），并且 (b) **真实地**把异议上报给真人处理。边际（何时上报）与交互方式（如何让真人好用）经过脑暴逐一定型，见下。

### 1.2 与现有"无人工接管"红线的关系

项目长期红线：**客户侧 AI 全自主，无人工接管**（`CLAUDE.md` + `scripts/check-no-human-takeover.{sh,ps1}` lint 在字符串层强制）。本设计**不破**这条红线，而是用**幕后领导模式**重新框定：真人之于 Agent，等价于一个"内部决策知识源"——Agent 向它请示、拿回结论后用**自己的口吻**转述。客户永远只跟 Agent 对话，真人从不出现在客户会话里。详见 §3 与 §9。

---

## 2. 已核实的生产现状（设计据此，不重写生产代码）

设计前已逐一核实下列锚点，确保新功能**复用现有链路、零绕过**：

| 锚点 | 位置 | 设计如何依赖 |
| --- | --- | --- |
| 统一发送网关 | `src/agent/gateway.rs` `run_user_operation_gateway` | 第二次 run（转述）经 `handle_follow_up_task` → 网关，零新发送路径 |
| follow-up worker 分发 | `src/tasks.rs:225-230`（按 `task.kind` 分发，else 落 `handle_follow_up_task`） | 新 kind `principal_decision_relay` 自动经 else 分支流入 `handle_follow_up_task`，无需改分发 |
| AgentTask 状态闭集 | `src/models.rs:402` `ALLOWED_AGENT_TASK_STATUS` | **不新增 status**，复用现有；`kind` 是开放 string，新 kind 合法 |
| hold category 系统 | `src/agent/types.rs:968-974` `HOLD_CATEGORY_VALUES` + `assert_hold_category_valid` | 新等待态 `ai_awaiting_principal_decision` **必须**加入 `HOLD_CATEGORY_VALUES`，否则被 `assert_hold_category_valid` 强制改写 |
| MCP 文本发送 | `src/mcp.rs` `message_send_text` | 请示卡推送给真人 wxid 复用此工具，零新通道 |
| webhook 入站 | `src/webhooks.rs` | 真人 wxid 入站在进入客户 agent 链路**之前**先做请示码匹配分流 |
| 五闸门 | `src/agent/guards/` + `review/` | 高风险件触发器复用闸门判定；转述 run 由真人授权满足 grounding |

**红线 lint 禁词集**（`check-no-human-takeover`）：`human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`，扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 的 diff 新增行。本功能命名全程避开，用 **真人 / 领导 / principal / escalation / decision**（已验证不触发 lint）。

---

## 3. 定位：幕后领导模式

- 真人**只**是幕后决策源，永远不直接面对客户。
- Agent 始终是唯一的客户侧声音；真人的决策由 Agent 用 AI 口吻转述，绝不原话转发。
- 客户侧"无人工接管"红线在**字面**（无禁词）与**语义**（客户从不面对真人）上都成立。
- 用户最初的话术例子"我要与领导商量"本身就向客户提到了"领导"——所以 Agent **可以**对客户说"我帮你跟领导申请一下"，这是真人销售的自然话术，且客户依然只跟 Agent 对话。

---

## 4. 触发边界：实质驱动，三类

决策 Agent 在 decision 阶段输出结构化字段：

```jsonc
"escalation": {
  "needed": true,
  "category": "out_of_scope_decision | high_risk_gated | stuck_or_undelivered",
  "reason": "客户要求 8 折，超出标准最低 9 折权限",
  "question_for_principal": "是否同意 8 折？或给一个底线折扣？"
}
```

三类触发：

1. **超职权/需决策事项**（`out_of_scope_decision`）：合同变更、特殊折扣、退款纠纷、法律承诺、定制需求。
2. **闸门拦截的高风险件**（`high_risk_gated`）：原本被五闸门静默 hold 的件（FactRisk≥6 / 未验证产品声明 / PressureRisk≥7），升级为 hold + 请示。
   - **实现（hold→升级接线）**：gateway hold 分支末尾调用 `escalation::escalate_held_decision`，按 workspace 的 `high_risk_escalation_mode` 配置决定是否升级——`blocked_by_safety_guard` / `blocked_unverified_product_claim` **无条件升级**；`held_by_ai_policy` **仅 `all` 模式升级**（保守默认 `decision_only` 不打扰领导）；`ai_waiting_for_more_context` / `blocked_by_required_field` / `blocked_by_budget` / `context_changed` 一律不升级（非决策墙，是 AI 自身可恢复状态）。判定逻辑落在纯函数 `should_escalate_held` 便于单测。
   - 升级时**补发安全占位**（`fallback_holding_reply()`，不含任何转接类措辞）安抚客户——hold 路径无 outbox，直发；体验与 approved 占位一致。同时写 `awaiting_principal_decision` 标记（hold 路径不走 `apply_agent_updates`，需单独 `$set`）。
3. **多轮卡死/失败兑动**（`stuck_or_undelivered`）：**同一议题连续 N 轮（默认 3，可配）Agent 未推进 + 客户出现负面反应**，gateway 前置 pre-check 识别。不靠纯轮数，避免误触发骚扰真人。

**明确不触发**：客户嘴上说"转真人 / 我要找人工"本身**不**构成上报，Agent 继续自己处理（保留现有 t8 场景的正确行为——见 §11）。触发取决于事项**实质**是否超职权，而非客户字面用词。

---

## 5. 真人 ↔ Agent 交互协议

### 5.1 请示卡（Agent → 真人）

经 MCP `message_send_text` 推送给配置的领导 wxid，结构化、带短码：

```
【请示 #E1A2】客户「张三」（老客户·已成交 2 单·在谈第 3 单 ¥12000）
诉求：能不能再便宜点，给到 8 折？
卡点：超出标准折扣权限（标准最低 9 折）
请示：是否同意？或给个底线？
```

- 短码 `#E1A2`：人类可读、可在自然语言回复中弱匹配。
- 对真人**不脱敏**（真人是决策者，需要客户全貌）。

### 5.2 真人回复（真人 → Agent）

真人是忙人，**绝不学命令语法**。协议 = **自然语言 + 回执码弱匹配**：

- 真人回什么都行：`行` / `可以但这周付款` / `最多 95 折` / `先稳住，我问下财务`。
- Agent 用 LLM 把这句自然语言**解读成结构**：`{ verdict: 同意/拒绝/有条件/暂缓, substance, constraints[] }`。**绝不原话转发给客户**。
- 匹配规则：
  - 回复含 `#E1A2` → 按码精确匹配。
  - 回复不带码（常见）→ 回落"该 wxid **最近一条未决**上报"（单一真人，通常同时只挂一条）。
  - 既不带码、也无任何未决上报 → 不当成客户决策回流，落"待 admin 确认的真人主动指令"，不自动生效（见 §9.4）。

### 5.3 真人回复模糊

真人回"你看着办 / 看情况"等 → 解读为**权限交回 Agent**：Agent 在标准政策内自行决断；若标准政策仍不足以答复 → 只向**真人**再追问一次（有界，不无限循环），绝不去问客户。

---

## 6. 拿到决策后怎么推进客户侧

第二次 run：`principal_decision_relay` task → `tasks.rs` else 分支 → `handle_follow_up_task` → `run_user_operation_gateway`。

- **先重读客户最新状态再转述**（跨天关键，见 §8）。
- 决策 Agent 把「原诉求 + 真人裁决 + 约束条件」重组为 AI 口吻回复，过 HumanLike / EmotionalValue 闸门；FactRisk / 产品准确性这里由**真人授权作为 grounding 来源**满足，不再拦截。
- **同意带条件**："跟领导申请下来了，可以给你 8 折，不过得麻烦你这周内完成付款，可以吗？" 条件 + `authorization_expires_at` 写进该客户 state/memory 并存入台账 `decision`，**授权过期后该条决策自动失效**（防止 Agent 拿过期授权乱承诺）。授权过期只让"这条授权不可再用"，不改台账条目状态（条目仍是 `resolved`）。
- **拒绝**：保关系优先，先用真人给的替代方案，否则回落标准报价："8 折确实做不了，但我能给你争取个赠品 + 包邮，你看？"
- relay 发出后：清 `ai_awaiting_principal_decision` 等待态，台账标 `resolved`，客户回到正常 managed 流。

> **relay 必须豁免频控类 precheck（关键闭环）**：relay 走合成 Inbound 重入网关，而触发请示时的占位 reply 刚把 `last_agent_run_at` 刷成 now；领导通常几秒~几分钟内回复，relay task 到达时距上次 run < 最小回复间隔（默认 20s），若不豁免必被 `rate_limited` 拦掉，**领导裁决永远送不到客户**。故 `precheck_send_gateway` 对 relay trigger 豁免 `cooldown` / `operation_policy` / `rate_limited` / `daily_limit` 四道频控（这些针对"主动打扰/触达频控"，而 relay 是客户期待内的被动应答）；`not_managed` 仍对所有 trigger 生效。识别靠纯函数 `is_principal_relay_trigger`——合成消息逐字以哨兵 `PRINCIPAL_RELAY_SENTINEL` 开头，真实客户消息经 prompt 隔离不会以该哨兵开头。

---

## 7. 等待时序与超时兜底

### 7.1 触发即时

触发时 Agent 先发**场景化**安抚占位（按场景生成，**非固定模板**），run 进入新等待态 `ai_awaiting_principal_decision`（加入 `HOLD_CATEGORY_VALUES`）。

### 7.2 等待期可分答

等待真人期间，Agent **先把客户消息里"非越权、能自主答"的部分推进**——只把越权点挂起请示，不冻结整段对话。

> 例：客户同一条消息问"能给 8 折吗（越权）+ 发货要多久（可自主）" → Agent 先答发货时效，8 折部分挂起请示。

> **三信号注入 decision prompt（实现）**：等待期的"可分答"行为靠 decision Agent 感知三个信号——纯函数 `escalation::build_decision_signals_text(contact, domain_config)` 从 contact + workspace 配置组装一段"请示通道信号"注入 decision user message（`decision.rs`）：①**等待领导决策中**（读 `awaiting_principal_decision` 标记——该标记由 approved 路径 emit escalation 或 hold→升级路径写入，此处首次被读取消费）→ 提示"勿就同一越权点反复请示、勿替领导拍板、非越权部分照常答"；②**多轮卡死**（`intent_trajectory` 尾部连续未推进 ≥ `DEFAULT_STUCK_THRESHOLD=3` 且末轮经 `reaction::is_negative_outcome` 判定负面，两条件 AND）→ 提示"避免硬推、换角度或如实告知需向领导确认"；③**高风险全量升级模式**（`high_risk_escalation_mode==all`）→ 提示"高风险件主动 emit escalationRequest"。三信号全缺返回空串，不污染 prompt。

### 7.3 超时兜底：永不代决

真人迟迟不回 → Agent **绝不**自动替真人决策，**无限等待**。等待窗口内：

- 客户发**非追问**消息 → **保持沉默不回**（不在等待窗里反复打扰客户）。
- 客户**主动追问** → 再发一次**场景化**安抚（如"还在帮你加急确认"）。

---

## 8. 跨天场景：先重评再转述

真人回复时客户可能已改主意 / 换话题 / 流失。第二次 run **先重读客户最新状态再决定怎么说**：

- 客户已改主意 → 不硬转述，顺势应对。
- 客户已换话题 → 先接新话题，再自然带出决策。
- 客户已流失 → 可能不发（避免"隔一天突然冒出一句八折"的鲁莽）。

---

## 9. 信息归属、知识闭环与防护

### 9.1 归属：接触级 + 知识缺口提案

- 真人决策**立即**写该客户的 memory/state（**接触级**，本次及后续对该客户可用）。
- 若决策**可泛化** → **另发一条知识缺口提案**，`status=draft` + `integrity_status=needs_review`，交给**现有**知识 Agent 的审核流。

### 9.2 知识闭环与红线

- 红线"**AI 永不自动验证知识**"不破：真人决策只能生成 `draft + needs_review` 提案，**人工审过才转验证**。
- 本通道只对接知识子系统**现有的 draft 契约**，**绝不**搅动正在测试优化的召回 / K 套件 / 质量套件。
- 闭环效果：真人通过决策"训练"Agent——专票政策被真人答过一次并沉淀为已验证知识后，下次任何客户问专票，Agent 直接答，不再请示。**请示频率随知识沉淀自然衰减。**

### 9.3 两类墙的统一模型

```
运营 Agent 撞墙
  ├── 墙 = 缺事实/知识  → 报知识缺口 → 知识 Agent → KB draft→审核→验证 → 以后能自主答
  └── 墙 = 缺决策/授权  → 请示真人 → 真人决策 → relay 转述
        └── 若决策可泛化 → 同时变成一条知识缺口提案（draft+needs_review）→ 喂回 KB
```

### 9.4 防护默认

1. **发送前二次校验**目标 wxid 严格等于该工作区配置的 `principal_decider`，绝不可能发到客户。
2. 请示卡对真人不脱敏；台账落库受 workspace 隔离 + admin 鉴权（沿用 IDOR sweep 模式）。
3. 同客户 + 同议题已有 `resolved` 且**授权未过期**的决策 → 复用，不再骚扰真人；已有 `pending` → 去重，不重复推送。
4. 真人主动发"以后都按 X"（不匹配任何未决码）→ 落"待 admin 确认的真人指令"，**不自动全局生效**（避免一句随口话变成全局策略）。

---

## 10. 多主管路由（MVP 收窄）

- MVP 配**单一领导 wxid** 接所有请示。
- 架构预留"按 escalation `category` 路由到不同真人"（折扣→销售总监、发票/退款→财务、合同/法律→法务）的**接口**，但**不先实现**（YAGNI），待真实多主管需求出现再扩。

---

## 11. 红线 / lint 合规（方案 A）

- 保留 `CLAUDE.md` 客户侧自主红线 **不变**。
- 保留 `check-no-human-takeover.{sh,ps1}` lint **不变**。
- 新功能命名全程避开禁词集，用 真人 / 领导 / principal / escalation / decision（含代码注释）。
- 新增 AI-内部等待态 `ai_awaiting_principal_decision`（AI-internal 风格命名，不触发 lint）。
- 仅给 `CLAUDE.md` 补一句澄清：**"无人工接管"指客户永不面对真人；AI 向幕后决策源请示不是接管。**

---

## 12. 新增组件清单

| 组件 | 类型 | 说明 |
| --- | --- | --- |
| `agent_principal_escalations` | MongoDB collection | 请示台账：短码、客户、议题、状态(`pending`/`resolved`)、`decision`(resolved 时含 verdict/substance/constraints/`authorization_expires_at`)、workspace 隔离 |
| `principal_decider` | workspace 级配置 | 领导 wxid（须是业务号好友） |
| `ai_awaiting_principal_decision` | hold category | 加入 `HOLD_CATEGORY_VALUES`（`src/agent/types.rs:971`） |
| `principal_decision_relay` | AgentTask kind | 开放 string，经 `tasks.rs` else 分支流入 `handle_follow_up_task`；**不新增 status** |

每新增 collection 需同时加 typed accessor + index 条目（`src/db/`）。

---

## 13. 端到端数据流

```
客户提越权诉求
  → webhook → 客户 agent 链路 → 决策 Agent emit escalation{needed:true,...}
  → gateway：发场景化安抚占位给客户 + run 入 ai_awaiting_principal_decision
            + 落 agent_principal_escalations(pending, #E1A2)
            + MCP message_send_text 推请示卡给 principal_decider wxid
  → [等待期] 客户非追问→沉默；客户追问→再安抚；非越权部分先答
  →（可能跨天）真人微信回复 → webhook 见 principal wxid 入站
            → 先匹配未决上报码（带码精确/不带码回落最近未决）
            → LLM 解读为 {verdict, substance, constraints}
            → 台账标 resolved + 写客户 memory/state(+authorization_expires_at)
            → 若可泛化：发知识缺口提案(draft+needs_review)
            → 起 principal_decision_relay task
  → follow-up worker → handle_follow_up_task → gateway
            → 先重读客户最新状态 → AI 口吻转述（过 HumanLike/EmotionalValue 闸门）
            → 清等待态 → 客户回正常 managed 流
```

---

## 14. 测试（仅增量，不动旧维度/旧弧/旧金标）

新增单测：

1. 三类触发各一条（out_of_scope_decision / high_risk_gated / stuck_or_undelivered）。
2. 回执码回流：带码精确匹配 + 不带码回落最近未决。
3. 真人自然语言 → `{verdict, substance, constraints}` 解读。
4. 跨天先重评再转述（客户已改主意/换话题）。
5. 等待期分答（非越权部分先推进）。
6. 超时兜底：非追问沉默 / 追问再安抚。
7. 知识缺口提案落 `draft + needs_review`。
8. wxid 误配防护：目标非 `principal_decider` 拒发。
9. 等待态 `ai_awaiting_principal_decision` 经 `assert_hold_category_valid` **不被强制改写**。
10. **relay 豁免 precheck**：`is_principal_relay_trigger` 对合成 relay 消息 true、对普通 Inbound false；`#[ignore]` 集成——managed contact + `last_agent_run_at=now` 下 relay trigger 跑 precheck 得 `allowed=true`，同条件普通 Inbound 得 `rate_limited`。
11. **三信号组装** `build_decision_signals_text`：awaiting 标记出等待信号；连续 3 轮同 intent + 末轮负面出卡死信号；不足 3 轮 / 末轮非负面不出；全缺返回空串。
12. **hold→升级判定** `should_escalate_held`：`blocked_by_safety_guard` / `blocked_unverified_product_claim` 两模式都升；`held_by_ai_policy` 仅 `all` 升、`decision_only` 不升；`ai_waiting_for_more_context` / `blocked_by_required_field` / `blocked_by_budget` / `context_changed` 均不升。
13. hold 路径补发占位红线：`fallback_holding_reply` 不含任何转接类措辞（crate-外测试 §14.9b，避免 src/ 内字面量被 no-human-takeover lint 误判）。

**t8 现有断言不动**——实质驱动边界下，客户嘴说"转真人"仍由 Agent 自己处理，t8 行为仍正确。

测试遵守 additive-only：只 append，不删改旧维度。

---

## 15. 待实施计划阶段细化的开放项（非设计悬空）

下列为实现细节，留给 writing-plans 阶段拆解，**非设计层未决**：

- `agent_principal_escalations` 的精确 BSON schema 与索引（短码唯一性、(workspace, status, contact) 复合索引）。
- 短码生成算法与碰撞处理。
- ~~多轮卡死 pre-check 的具体信号实现（复用哪个负面反应判定）~~ → 已实现：`consecutive_unprogressed_turns`（intent 轨迹尾部连续未推进）+ `reaction::is_negative_outcome`（末轮负面），两条件 AND，见 §7.2。
- workspace 配置 `principal_decider` 的存储位置（复用现有 workspace config collection）与 admin 配置 UI。
- 等待态在 admin 台账卡片的前端展示（遵守 `docs/frontend-design-system.md`）。
