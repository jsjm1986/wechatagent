# 运营 Agent 回复 prompt 瘦身 — 设计

**日期**：2026-07-13
**类型**：性能优化（prompt 体积瘦身，A/B 灰度分批上线）
**根因来源**：2026-07-13 systematic-debugging 生产诊断（见 `memory/bug_reply_slow_prompt_exceeds_budget.md`）

## 问题陈述

生产 117 上运营 agent 回复客户很慢：单次 `user.reply.task` LLM 调用 45-149 秒（prompt 39-43k tokens）。根因诊断（生产真实数据亲验）：

- prompt 体积大头是**静态骨架 + 动态大块**，不是知识切片（全库 95 chunk body 仅 9254 字符）也不是 memory（operating_memory 仅 2.7KB）。
- 近 30 天 inbound run 100% 超 `run_token_budget`（原 30000），avg 实际用 67003（2.2 倍）→ review 被跳过、频繁 `blocked_by_budget`/`held_by_ai_policy`/`no_reply`、恶性重算（`superseded_by_new_inbound`）。

**已实施的止血（方案 A，本 spec 之外）**：已把生产 active `user_operations` domain config 的 `runTokenBudget` 30000→**300000**、`runTokenBudgetEscalated`→**600000**、`runMaxLlmCalls` 6→**10**（DB 热更新，即时生效）。止血消除了"超预算导致的降级/拒答/重算"，但**没有提速**——单次调用仍慢。

**本 spec 的目标（方案 B）**：给 reply prompt 瘦身以真正减少单次 LLM 调用耗时，在**不损害回复质量**的前提下降低 prompt token 量。

## 约束与红线（贯穿全设计）

- **碰生产 prompt 是红线区**：只沉淀可复现的抽象精简（删死字段/去重复/合并冗余表述/按相关性截断），**绝不为单条对话/单次样本点对点修补**（过拟合红线）。
- **规则语义一字不丢**：所有"精简"只删**冗余表述/重复/无消费字段/调试元数据**，业务规则本身、枚举值、长度阈值、红线段一律保留。
- **`validate_and_promote`（types.rs:637-896）只校验字段存在性+枚举+长度，不校验注释**——这是"注释可精简、字段不可删"的安全边界（已亲验）。
- **check-no-human-takeover 合并门**：新增/改动行不得踩禁词（接管/人工/takeover/hand-off）。

## 验证策略：A/B 灰度并行（数据驱动，非拍脑袋）

系统已有 A/B 灰度设施（`prompts.rs:476-542`）：同一 `(workspace, prompt_key)` 下多条 `status=active` 的 prompt_template，按 `hash(客户wxid)%count` 分桶，同一客户永远拿同一份。

**灰度机制**：每个中/高风险瘦身批作为**第二个 active 版本**上线，与原版 50/50 分桶跑真实流量。

**判定指标**（`agent_run_logs` 已具备，`prompt_versions` 记 reply.system/policy/task 三 key 版本号，gateway.rs:4713-4759）：
- **token 下降**：`tokens_used`（瘦身版应显著低于原版）。
- **质量不退化**（瘦身版对比原版不得变差）：
  - `blocked_by_budget` / `no_reply` / `held_by_ai_policy` 占比不上升
  - `review_skipped_budget_exceeded` 等 `degraded_reasons` 不上升
  - 字段漏填类 risks（`missing_required_field` / `insufficient_detail_in_critical_turn`）不上升
- 数据证明不退化 → 原版下线、瘦身版全量。任一批退化 → 回滚该批（原版始终在，零风险）。

## 批次 1：零风险清理（单测即可，不需 A/B）

代码零风险（无消费/零信息损失/纯重复），单测验证后可直接全量。

| 项 | 改动 | 依据（已亲验 file:line） |
|---|---|---|
| 删 4 死字段 | task 契约移除 `intentAnalysis` / `productFitScore` / `forbiddenClaimRisk` / `recommendedResourceIds` 的字段声明+注释（prompts.rs:1249/1330/1333/1335） | 全库 grep 仅 types.rs 定义+carry_through 透传，无任何独立读取点；`intentAnalysis` 更被 reviewer 刻意排除（gates.rs:55/1296）。均 Option 透传，LM 不输出不影响 promote/闸门 |
| 去 context_pack 重复注入 | 删 user 段槽7 `memory_card_text`（decision.rs:545-549） | context_pack 已完整嵌在槽6 `memory_text` 的 `memoryCard` 字段（decision.rs:535），第二份是纯冗余；doNotDo/commitments 也在槽6，安全语义不丢（decision.rs:527-528 注释佐证） |
| 删知识路由调试元数据 | 槽11 `knowledge_route_text`（decision.rs:489-493，现 `serde_json::to_string(knowledge_route)` 全 13 字段）**仅**去掉 3 个纯落库/调试字段 `toolTrace`/`selectedChunkRankings`/`evidenceExcerpts`，其余 10 字段（含 `missingKnowledge`/`requiresEvidence`/`selected*Ids` 等对 LLM 有语义者）**保留** | 这 3 字段是纯调试/采集元数据：`selectedChunkRankings` 注释明写"只采集落库不参与加权"（types.rs:1367-1368）、`toolTrace` 是路由器工具轨迹、`evidenceExcerpts` 是证据摘录快照——回复文本生成不消费。**注意**：`KnowledgeRouteResult` 带 `#[serde(rename_all="camelCase")]`（types.rs:1337），新函数输出 key 必须保持 camelCase，与原 `to_string` 逐字一致，不得偷改 LLM 看到的字段名 |
| 删硬运行参数 | 删 user 段槽5 `runtime_text`（decision.rs:479-483） | 系统运行参数（recentMessageLimit 等），对 LLM 回复语义无用 |

**批次 1 不动**：Soul 红线冗余（反接管/不编造，属刻意安全设计）、任何有消费的字段。

> **2026-07-13 writing-plans 亲验剔除**：原计划批次 1 含第 5 条"删纯跨层重复"（policy:1173/1168/1160、system:1117↔soul:934、policy:1165-1167↔soul:971-976）。写 plan 前逐行核对，5 处所谓"一字不差重复"**全部不成立**：① policy:1173"不暴露AI…内部评分。" vs system:1118"…内部评分**或数据库字段**"——不同，且 policy:1173 是紧接的 1174【隐私/内部画像】详解段的引子标题，删了会让 1174 悬空；② policy:1168 是"用户问清单/步骤时"的**场景专属**规则（含"不要说我发你却没给内容"独有语义），非 system:1121 的通用 markdown 禁令；③ policy:1160"枚举小写"在 system 段（1116-1121）**根本不存在**，它属 policy"决策协议字段"段；④ system:1117"长期关系经营者" vs soul:935"长期关系优先"——措辞不同；⑤ soul:971-976 是带好例/差例的完整"多轮连续性"教学段，policy:1165-1167 是精简条目，**颗粒度/表述均不同**，非纯重复。删除有真实语义损失风险，踩"规则语义一字不丢"红线，故整条剔除。跨层去重本就 <1% token（诚实局限已述），非杠杆，剔除不影响批次 1 价值。

## 批次 2：中风险（A/B 灰度）— Task 长注释精简 + 观测字段注释压缩

预计省 ~2400 字符。作为一个 A/B 批次整体灰度。

| 项 | 改动 | 边界（已亲验） |
|---|---|---|
| Task 内双写去重 | R1.3 长度规则（行内1216-1222 ↔ 要求段1384-1385）、承诺必填（1285-1289 ↔ 1382）、素材规则（1344-1349 ↔ 1386-1390）、引荐规则（1351-1354 ↔ 1391-1395）——每条规则**只保留一处**（留更醒目的要求段，行内留字段名+一句指针） | 规则文字一字不改，只消除同一 prompt 内的双写 |
| escalationRequest 近义合并 | 合并 1357/1358 的判据重复表述 | **保留** self-check 硬约束（1360）——无代码兜底（gateway.rs:2894 只认结构化字段），漏填=请示丢失、口头承诺落空，红线 |
| 转述模式精简 | "不泄漏内部字段"压一句（代码 fail-closed 兜底，logic.rs:211/256） | **保留** verdict→基调映射（1371-1374）——无代码兜底，删了 LM 不知 rejected 婉拒/delegated_back 自答 |
| 7 观测字段注释压一行 | `safeClaimsUsed`/`matchedKnowledgeIds`/`objectionsDetected`/`nextBestAction`/`bayesianObservations`/`agentGeneratedSignals`/`conversationModeReason` | 只喂 reviewer 或纯观测，填错无业务后果（字段消费报告：gates.rs:65-68 仅作 reviewer 输入；bayesian types.rs:108-112 永不驱动决策） |
| 素材/名片"禁止编造id"删注释 | assetsToSend/namecardToSend 防幻觉注释删 | 代码二次准入会 reject 幻觉 id（gateway.rs:2686/2829 亲验）；**何时发/引荐的业务判断保留** |

**批次 2 红线守护（6 条绝不可删/一字不动）**：escalation self-check(1360)、转述 verdict 映射(1371-1374)、R1.3 关键轮≥20字规则(保留于要求段)、need_clarification 只出澄清问句硬约束(1283-1284)、"窗口序号"定义(1270)、单发 final 形态声明(1195-1196，测试锁死防 tool_calling 回退)。

## 批次 3：高风险（A/B 灰度，逐项单独成批）

每一项**单独作为一个 A/B 批次**上线（不合并），因各自影响不同质量维度，混改无法归因。这是提速的主要杠杆，也是风险最高处，必须逐项数据验证。

| 子批 | 改动 | 风险与守护 |
|---|---|---|
| 3a 状态机精简 | 槽4 `state_machine_text`（decision.rs:1226-1228，现 `serde_json::to_string(整个state_machine)`）改为只注入**当前 operation_state + 其合法邻接转移集**，而非全字典 | 风险最高单独一批：需确认 LM 判 nextState 只需当前态+可达集。灰度对比 `operation_state_transition_rejected` 审计事件率不上升 |
| 3b playbook/域策略按回复相关性截断 | 槽2 playbook（decision.rs:1230-1257）保留 method_prompt/reply_style/forbidden_rules，砍 profile_method/tag_method/stage_method/intent_method（"画像更新方法"非单轮回复必需）；槽3 域策略（decision.rs:1205-1224）保留 name/goal/methodology，砍 workflow/automation_policy/review_policy/tool_policy | 可能影响运营判断细腻度；灰度对比标签/阶段写入质量与 review 通过率 |
| 3c Soul 行为样例压缩 | 反接管话术正反例(soul:996-1007,~1500字符)、理性客户失败样例、压力轮展开——压缩示例，**保留规则本身** | Soul 是人格本源，样例削多了弱化执行力；**反接管红线规则(soul:994-995)一字不动**，只压 ❌/✅ 示例。灰度重点盯 check-no-human-takeout 相关行为（承诺真人率）不上升 |
| 3d history 单条截断 | 槽38 `history`（decision.rs:741-763，单条无截断）加单条字符上限（防客户粘贴长文撑爆）；`recent_message_limit`（默认12）保持可配 | 影响上下文连贯，保守设阈值（如单条 ≤800 字符截断+省略号）；灰度对比回复连贯性（无客观指标，靠抽样人工+ no_reply 率） |

## 分期与依赖

1. **批次 1** 先做（单测全量，无 A/B）→ 立即减一部分 token，且清理死字段让后续批次的 prompt 更干净。
2. **批次 2** A/B 灰度 → 数据通过后全量。
3. **批次 3 逐子批（3a→3b→3c→3d）** 各自 A/B 灰度 → 逐项数据通过后全量。

每批之间不并行改 prompt（A/B 归因要求单变量）。预算已调至 300000（方案A），批次 3 即使提速渐进也不会再触发 blocked——**本 spec 是渐进优化，不是救火，可从容按批验证**。

## 诚实的局限

- 批次 1+2 省的 token 有限（reviewer 报告：task 瘦身 ~25% ≈ 省 5-6k token，跨层去重 <1%）。**真正把 40k 打回合理区间靠批次 3 的动态大块**，而那是风险最高处——所以逐项灰度、数据说话。
- 中文 token 密度高，字符省比 ≠ token 省比，最终以 `agent_run_logs.tokens_used` 实测为准。
- 批次 3d 的"回复连贯性"无纯客观指标，需抽样人工核对辅助 no_reply 率判断。

## 改动范围

- prompt 模板：`user.reply.task` / `user.reply.policy`（DB `prompt_templates`，通过 A/B 新增 active 版本，不物理覆盖原版）。
- 后端代码：`src/agent/decision.rs`（user 段注入槽的截断/去重逻辑：槽5/7/11 删除、槽2/3/4/38 截断）、`src/routes/knowledge` 无关。
- `src/agent/types.rs`：4 死字段**保留 struct 字段不动**（`RawAgentDecision` 里都是 `Option`/`#[serde(default)]`，LM 不输出时反序列化为 None、carry_through 透传 None、零副作用），**仅从 prompt 模板移除字段说明与生成要求**。这样零反序列化风险、零 promote 影响——最保守。不删 struct 字段（删字段要动 carry_through/AgentDecision，风险大于收益）。
- 存储层/索引/前端：零改动。

## 测试

- 批次 1：单测断言瘦身后 prompt 不含死字段键、不含重复段；`cargo test --lib` 基线不回归（≥350）；契约/PBT 不破。
- 批次 2/3：单测断言红线段仍在（escalation self-check/转述映射/R1.3规则/窗口序号定义等关键子串存在）；A/B 上线后按 `prompt_versions` 分桶查 `agent_run_logs` 对比指标。
- check-no-human-takeover lint 绿。
