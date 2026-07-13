# 知识 Agent 反接管红线治本（批次1：堵源头 + 切回流）

日期：2026-07-14
状态：设计待审

## 问题与实测证据

生产定位是「全 AI 自治·无人工接管」——客户永远只跟 AI 对话、永不被转给真人；AI 遇超职权事项向幕后决策源（领导）请示、拿回结论后用自己口吻转述（这不是转人工）。

2026-07-14 用影子模拟（`simulate_user_dialogue`）实测 5 组会触发"转人工"知识的客户问题（价格优惠/直接要真人客服/机构地址/满意后付款/投诉要负责人），发现一个**架构缺口**：

- **面向客户的最终话术（Reply Agent 的 `decision.replyText`）红线守住了**——5 组无一句"转人工/转客服"，都用第一人称承接（"对接的就是我""联系方式我没法发"）。每条 decision 的 `avoid` 字段都显式列着"别把客户推给客服/同事"。
- **但知识 Agent 的中间答案（`knowledgeRoute.reason`）满口"转人工"**：实测原文如"我这就帮您转人工客服""这是服务投诉，按规则应转人工主管处理""我马上把您的情况同步给主管，由 ta 直接和您对接"。

### 根因（已亲验 file:line）

反接管红线的**注入范围没覆盖全部读知识的 Agent**：

1. 反接管红线**只种在 DB 的 prompt 模板**里：`prompts.rs:1147`（consultative 段「绝不把问题交接给运营同事/真人/同事」）、`:1149`（boundary_protection）、`:1172`（表达红线，标注"任何模式任何轮次都适用"）。这些属 `user.reply.policy` 模板正文，经 `prompt_specs()`（prompts.rs:1054）种进 DB `prompt_templates`，Reply/Review 运行时读回。
2. **知识 Agent 用的是代码内联 `const SYSTEM_PROMPT`（knowledge_agent.rs:267-271），根本不读 prompts.rs** → 它是红线的**盲区**。其 system prompt 只讲"渐进式披露/带引用 answer/只输出 JSON/最多 4 轮"，无一字反接管。它忠实复现了知识库里的传统医美"转人工 SOP"（那批 active+verified chunk：bde242「10.2 必须转接的六类情形」、bde241「对话流程六步」、bde237 FAQ-12、bde23e FAQ-19、bde23f FAQ-20 等）。
3. 知识 Agent 产出的 `answer.answer` → `route.reason`（knowledge_router.rs:620），经**两条路回流**到受红线保护的 Agent：
   - **Reply**：`format_knowledge_route_for_prompt`（decision.rs:1248-1259）只 remove 掉 `toolTrace`/`evidenceExcerpts`/`selectedChunkRankings` 三个 key，**保留 reason**；该文本经 decision.rs:914 作为第 9 参数落在 Reply prompt 的"知识路由:"标签下（decision.rs:853-854）。单测 decision.rs:1859 显式断言"保留 reason"。
   - **Review**：review/mod.rs:368 `serde_json::to_string(knowledge_route)` **整条原样序列化**（连字段都不删），reason 全进 reviewer 上下文，比 Reply 更裸。

### 为何目前没出事故 / 为何仍须治本

Reply Agent 的红线足够强，把 reason 里的"转人工"翻译成第一人称话术——但这是**持续对抗**（每轮 Reply 都要顶着一段"建议转人工"的知识答案掰回来），不是根治。实测 5 组里 4 组因 `budget_exceeded_no_review` 走 local 兜底（review 分数全 0），正是红线可能被绕过的真实 edge case：长对话 / 预算降级 / LLM 抖动下，这段"转人工"内容就可能泄漏给客户。

## 设计基石（已亲验）

- Reply prompt 里"产品知识"（第 8 参数 `knowledge_text`=结构化 chunks，decision.rs:481/913）和"知识路由"（第 9 参数 `knowledge_route_text`=含 reason 的 JSON，decision.rs:914）是**两段独立注入**。承载知识事实的是 `knowledge_text`；`knowledge_route_text` 是知识 Agent 的路由/推理副产品。
- `KnowledgeRouteResult`（types.rs:1340-1374）的"知识充分度"信号由**结构化字段**承载：`knowledge_coverage`（enough/weak/missing/not_required）、`requires_evidence`、`missing_knowledge`、`selected_slice_reasons`、`evidence_excerpts`（来自 verified chunk 的 source_quote，干净）、`selected_chunk_ids`。**`reason` 是唯一那段被污染的自然语言总结**。→ 把 reason 从下游剔除后，Reply/Review 仍能从结构化字段拿到等价甚至更干净的信号，**干净切除、不留信号缺口**（已亲验）。
- 知识 Agent 的 `SYSTEM_PROMPT` 是 knowledge_agent.rs:267-271 的 Rust 字面量常量，经 `generate_agent_json` 直传（prompt_key `"knowledge.agent"` 仅作日志/缓存标签，不触 prompts.rs）。→ 给它加红线的**唯一正确改点是这个常量**，改 prompts.rs 无效。

## 方案（批次 1：两层互补·纵深防御）

本批只做低风险的两层（用户已定"分批：先低风险两层"）。数据清洗、红线注入机制架构统一留待后续批次。

### Layer 1 — 知识 Agent 补反接管红线（堵源头）

**改点**：`knowledge_agent.rs:267-271` 的 `const SYSTEM_PROMPT` 末尾追加一段反接管约束。

**措辞策略（反过拟合，关键）**：
- **不用关键词黑名单**。不写"禁止出现'转人工'三字"——那是脆弱的字符级过拟合，会误伤"知识库.md 是人工阅读版"等正当用法，也压不住语义变体（"让同事跟进""转主管"）。
- **约束角色定位，不约束禁词**。根因是知识 Agent 误以为自己在对客户说话、照搬客服 SOP 话术。修法是校准其角色认知——它是"给内部 Reply Agent 的知识研判"，不是对客户说话的人。
- 追加语义（措辞实现时打磨，最终以 lint 合规为准）：

  > 你的 answer 是给内部 Reply Agent 的**知识研判**，不是发给客户的话术，也不是行动指令。当被检索的知识内容包含"转人工客服/转主管/让真人对接"这类**机构内部流程描述**时：可以如实转述"该事项需依据正式政策/由内部核对后确认"这一**事实边界**，但不得把它改写成对客户的行动建议或话术（如"我帮您转接客服"）。本系统定位是 AI 全程自治，超出当前知识的事项统一研判为"需内部核对正式口径"，由 Reply Agent 决定如何向客户表达。

- **恒注入，不设 flag**：写死在常量里，每次调用都带（与 Reply 红线恒注入同理，避免"忘了开"盲区）。
- **普适性论证（为何不过拟合）**：沉淀的是"知识研判 Agent 不应把知识里的对客沟通 SOP 当成自己的行动脚本"这条方法论，对任何行业/任何知识内容都成立（保险/教育/健身知识库同样适用），不依赖"眼袋/团购"这次的样本。

### Layer 2 — 不把 reason 拼进下游（切回流通道）

- **改点 A**：`decision.rs:1248` `format_knowledge_route_for_prompt` —— 在现有 remove 3 字段基础上**再 remove `reason`**。
- **改点 B**：`review/mod.rs:368` —— 不再 `to_string(整条 route)`，改用**净化后的路由视图**（同样剔除 reason，保留 coverage/missing_knowledge/evidence_excerpts/selected_chunk_ids 等结构化字段）。可复用 Layer 2-A 的净化函数，避免两处口径漂移。
- **依据**：见「设计基石」——结构化字段承载充分度信号，reason 只是被污染的自然语言，剔除不丢信号。

### 两层关系

Layer 1 让源头不再产污染；Layer 2 即使 Layer 1 在某次 LLM 抖动下漏网，reason 也不进下游 prompt。双保险纵深防御 = 治本而非对抗。

## 测试

**单元测试（Layer 2，纯函数可测）**：
- `format_knowledge_route_for_prompt`（decision.rs:1248）：构造 reason 含"转人工客服"的 `KnowledgeRouteResult`，断言输出 JSON **不含 reason 字段**，但保留 `knowledgeCoverage`/`missingKnowledge`/`evidenceExcerpts`/`selectedChunkIds`。
- Review 净化路由视图：同样断言剔除 reason、保留结构化字段。
- **decision.rs:1859 的旧断言"保留 reason"须反转成"不含 reason"**——这是行为变更点，显式改测试而非留矛盾。

**Layer 1（prompt 行为，无法单测锁）—— 泛化验证，非调绿**：
- 影子模拟重跑实测那 5 组 + 3-4 组不同行业措辞/不同转接触发点的变体。
- 人工核对：`knowledgeRoute.reason` 不再出现转接话术；`knowledge_coverage`/`missing_knowledge` 不退化。
- **绝不为让某一条样本通过而反向改措辞**（反过拟合红线）。

**基线门**：
- `cargo test --lib` 不回归（当前 1974）；`scripts/check-baseline` 双门绿。
- `check-no-human-takeover` lint：knowledge_agent.rs 在 `src/agent/` 下受该 lint 扫描，SYSTEM_PROMPT 新增行含"转人工/真人"等词须谨慎——措辞用"AI 内部研判/正式口径/机构内部流程"这类合规表述，禁词只在"描述知识内容里的流程"语境出现。实现时**先跑 lint 确认不误伤**，必要时调整措辞或确认行级豁免规则。

## 影响面与风险

- 改 3 处：`knowledge_agent.rs` SYSTEM_PROMPT（+若干行）、`decision.rs:1248`（+1 行 remove reason）、`review/mod.rs:368`（改用净化视图）。
- 行为变更：Reply/Review 的 prompt 不再含知识 Agent 那段自然语言总结（reason）。承载知识事实的结构化字段全保留，实测已确认信号不丢。
- 风险低：两层都是**新增约束/剥离**，不改任何现有稳定逻辑，不动阈值、不动串行结构。
- **回退**：Layer 2 是纯函数剥离字段，改回即恢复；Layer 1 是 prompt 常量追加，删掉即恢复。均向后兼容。

## 不做（YAGNI · 留待后续批次）

- **不改知识数据**（那批转接 SOP chunk 的口径）——留给"清洗知识数据源"批次。
- **不做红线注入机制的架构统一**——留给"治架构"批次（把反接管红线抽成所有 LLM 调用强制经过的统一注入点）。
- **不动 `/api/knowledge/ask`**（sources_meta.rs:553/662 直出 answer 的运营侧知识问答，不面向客户，不在本批红线范围）。
- 不加关键词黑名单、不做 reason 的关键词 replace。
- 不改 reply/review 串行结构、不动阈值。
