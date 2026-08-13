# 项目理解台账（核证记录）

> **用途**：AI 会话对本仓库的理解档案。所有条目都经过代码核证，按证据等级标注；后续会话应先读本文件恢复上下文，但**任何 file:line 在引用前仍须当场重验**（代码会演进）。
>
> **核证时间**：2026-08-12 ~ 2026-08-13（基于当时工作区：**分支 `fix/dependency-security-remediation` @ a637b61**——2026-08-13 修正：早期台账误写"main @ a637b61"，实际 main 停在 38766e2，主 workspace 全程在该修复分支上；未提交改动终版快照为 **47 个文件 +3285/−1428**）
>
> **方法论教训（第三条）**：环境状态（当前分支/worktree 拓扑）与代码事实同样必须亲验——`git status` 不显示分支名时必须补 `git branch` 确认，不得默认在 main。
>
> **S0 收口与三线工程（2026-08-13）**：47 文件已按 6+1 分组提交（S0，8 commits，禁词已修）+ 深读档案 commit + 三线 plans commit，全部落在 `fix/dependency-security-remediation` 分支（S0 后基线 d99b6e7，`cargo test --lib` 2530/0）。三线优化从 d99b6e7 分出并行执行。
>
> **深读工程第一轮（2026-08-13）**：全仓 19 份逐行级深读记录，见 `project-understanding/`（README 为索引含验收状态）。后端 src 约 19.3 万行、tests 182 文件、前端 5.3 万行（core+features+__tests__）、全部规格与文档逐行/逐篇读完。
>
> **深读工程第二轮（2026-08-13）**：交叉验证与终裁 12 任务完成——236 条疑点全部终裁（72 实锤/22 不成立/128 设计/7 存疑，权威=22/23 号）；五路交叉验证 200+ 锚点抽验通过率 96.7%-100%，记录间矛盾全部裁决回写；新增 4 个运行时缺陷发现；plans 149 篇与 system-review 26 文件补盲完成。
>
> **改动前使用顺序**：① 查 `30-global-fact-cards.md`（闭集/阈值/键约定速查）→ ② 查 `29-doc-code-divergence-master.md` 反向索引（防文档误导）→ ③ 查 `28-crosscheck-tests-vs-prod.md` §4（无测试守护清单=高危区）→ ④ 读对应领域深读记录（01-19 号）→ ⑤ 疑点以 22/23 号终裁为准 → ⑥ 任何 file:line 动手前当场重验。
>
> **两条固化的方法论教训**：① 判断"是否在生产使用"必须验证调用点（spec/测试/种子的存在性不是证据）；② 缺陷判断必须同时验证分支内行为与分支可达性（创建点）。
>
> **证据等级**：
> - **[A]** 主会话亲读源码/亲跑命令验证
> - **[B]** 探索子代理通读 + 主会话抽查关键行确认
> - **[C]** 探索子代理通读，主会话未逐行抽查（方向可信，引用前重验）
> - 仅有 md 文档依据、代码未证实的结论**不写入本台账**
>
> **维护约定**：后续会话新增理解时按同格式追加，修正旧条目时保留修正痕迹（"原记 X → 实为 Y"）。

---

## 一、文档 vs 代码的已知偏差（防误导清单）

这些是"读文档会得出错误结论"的实锤，遇到相关话题**必须以代码为准**：

| # | 文档说法 | 代码事实 | 证据 |
|---|---|---|---|
| 1 | CLAUDE.md / 多处文档描述"FactRisk≥6 / ProductAccuracyScore<7 等 5 闸字符串守卫" | 销售域字符串级守卫已于 2026-05-25 删除；现行为 Review 分数闸（`hallucination_score` alias `factRisk`、`knowledge_grounding_score` alias `productAccuracy`）+ R5.4 结构化兜底 | [A] `guards.rs:1-7` 头注释亲读；[B] `review/gates.rs:117-221` |
| 2 | 文档描述 9 字段自治协议思考链每轮输出（user.reply.task 契约） | **完整版已退役**：生产只调 `user.reply.fast.task`（紧凑 schema，思考字段仅 riskSelfCheck/conversationModeReason/whyShouldReply 等 3 个）；画像/标签/记忆移到发送后投影。**精确化（2026-08-13 二次裁决）**：`user.reply.task` 模板处于"种子包仍种入 DB、prompt_guard 治理面仍覆盖、守护测试仍钉内容，但运行时零消费"的退役态——看到库里/测试里有此 key 不代表生产在用；生产三站点（首发/rewrite/revision）统一 fast.task | [A] 全量无截断 Grep src/ 共 16 处命中逐一分类亲证（spec 定义/治理面/测试 fixture，无 load/generate 调用点）；`decision.rs:460,1321`；`agent/mod.rs:230-232` "the retired full task"。注：17 号记录初版曾误判"仍在生产"，已裁决修正——教训：判断在用与否必须验证调用点 |
| 3 | README 写"231 条路由" | 实际 `routes/mod.rs` 中 `.route(` 出现 **235** 次 | [A] 亲跑 rg 计数 |
| 4 | README 写"56 个迁移" | 迁移目录为 m001–m058 | [B] 数据层报告逐个读过 |
| 5 | CLAUDE.md 写"lib 基线 2359 passed"与"≥350"并存、"shell 是 bash on Windows" | 脚本门槛 `LIB_BASELINE=350`/`PBT_BASELINE=33`；当前环境是 macOS | [A] `scripts/check-baseline.sh:28-29,84` 亲读 |
| 6 | agent-autonomy-loop spec 引言写第四个基线 PBT 是 `string_fact_risk_guard` | 现行第四个是 `wiki_chunk_revision_pbt` | [A] `check-baseline.sh:84` 亲读 |
| 7 | 简化架构图暗示 webhook 同步跑 Agent | webhook 只落库+物化 durable `inbound_reply` 任务（稳定 _id 单飞）+ spawn 低延迟唤醒；执行在 worker | [A] `webhooks.rs:26,81,99` 亲证；`webhooks.rs:1585-1642` 亲读 |
| 8 | 部分文档仍描述"user-ops 主链路 tool-calling 多轮知识检索" | user-ops 单发路径若 LLM 误输出 tool_calling 会被强制转 final；知识获取由 gateway 预先路由（knowledge_agent 多轮在路由内部）；`knowledge_tools` 的 catalog→search→open_slice 主要服务管理台 chat | [C] agent 核心报告 `gateway.rs:2912-2937`、`chat_tool_loop.rs:14-15` |

---

## 二、生产主链路锚点表（发送一条消息的真实路径）

全部 [A/B] 级，按执行顺序：

1. **Webhook 入站**：`webhooks.rs:1245` 入口；验签 HMAC-SHA256（`2849-2884`，fail-closed）；跨副本限流（Mongo fixed-window，`929-987`）；领导回复分流在客户链路之前（`1400-1425`）；幂等落库靠 `dedupe_key` 唯一索引（`1469-1503`）。
2. **静默时段判定**：`webhooks.rs:1602-1623` [A]——`quiet_hours_enabled && is_quiet_now` 时**所有** managed 入站一律 defer 到 `next_wake_at`（**无高意向/高风险例外分支**）；唤醒 jitter 上限 `WAKE_JITTER_MAX_SECONDS=900s`（`config.rs:812` [A]）。静默期多条入站合并为 1 个 wake 任务，醒来聚合回 1 次（`1655-1664`）。
3. **任务认领**：`tasks.rs:186-244` claim_token + claim_generation CAS；inbound_reply 专用 worker 250ms 轮询（`552-558`）；tick 顺序含 `scan_escalation_timeouts`（`tasks.rs:1138` [A]）。
4. **Gateway 编排**：`gateway.rs` 约 9152 行。入口分发 `178-217`（relay → escalation；inbound_reply → durable handler；其余 → FollowUp）。precheck `5245-5365`：not_managed / cooldown / policy / rate_limited(20s) / daily_limit(仅 FollowUp) / expired / quiet_hours(仅 FollowUp) / context_changed。**relay 豁免整段频控与 quiet_hours**（`5254-5340` [B]，识别靠 `PRINCIPAL_RELAY_SENTINEL` 哨兵）。
5. **决策**：`decision.rs` 组装 Soul+system+policy+fast.task（`438-460,976-1054`）→ `generate_agent_json(..., "user.reply.fast.task")`（`1315-1324` [A]）。渐进三档 Lean/Relational/Full（`PROGRESSIVE_TIER_ENABLED` 默认 true）。
6. **评审**：`should_run_review` 只看 `decision.should_reply`——**可发送正文永远不能自免审**（`review/mod.rs:3277-3285` [A]）。light/full 由 `planner_from_decision`（`guards.rs:48-68`）+ `effective_review_mode`（`review/mod.rs:3236-3258`）决定：needs_review/high risk/knowledge_required/低 confidence(<4) → full。Review 与 ClaimGate 并行（`gateway.rs:2980-3022`）。Reviewer 看不到 Reply Agent 自我推理字段（防串供，`review/gates.rs:50-63`）。
7. **阈值**（`runtime.rs:939-945`、测试 fixture `agent/mod.rs:880-912` [A]）：hallucination≥6 硬拦、grounding<7 硬拦、humanLike<6 软闸、emotionalValue<6 软闸、pressureRisk≥7 软闸、boundaryPrivacy≤3 软闸；软闸触发 single-shot revision 一次。
8. **R5.4 产品声明兜底**：`gates.rs:826-861` [A] 亲读——reviewer 自报 `requiresProductKnowledge=true` 时，**三路并联背书取或**：verified_chunks ∪ priced_from_catalog（产品目录结构化定价）∪ principal_product_exempted（领导授权豁免，客户级）。三者皆空才 `blocked_unverified_product_claim`。claim_analysis 缺失按"非产品声明"放行（2026-05-25 清理后 R5.3 fail-closed 推断不在恢复范围，注释 `846-849`）。
9. **Outbox**：幂等键 `(workspace, account, idempotency_key)` 唯一索引；状态闭集 pending/in_flight/sent/failed_terminal/canceled/delivery_unknown（`outbox.rs:44-84`）；分段 `#seg{idx}`；dispatcher 二次安全门（not_managed_at_send / cooldown / stop-after-decision / stale_30min，`outbox_dispatcher.rs:918-977`）；escalation/clarification 类跳过二次门（`922-928`）。
10. **发送节奏**：`pacing.rs:15-19` [A] 亲读——`account_send_interval_ms(jitter01, min_ms, max_ms)` 纯随机线性映射 [1000,4000]ms，**签名无文本长度参数**。
11. **发送后**：投影快照在写 review 后即持久化、发送授权后激活（`gateway.rs:4056-4119`）；投影 worker 每次 **1 次 LLM**（`post_decision.rs:400-465`，RunBudget max_calls=1）[B]。记忆 consolidation 触发：`consolidation_needed` ∨ `memory_write_score≥6` ∨ pending≥4 ∨ 最老 pending≥6h（`memory.rs:2848-2864`）[C]。
12. **反应分析**：下一条 inbound 时对上一轮 review 原子 claim（`reaction_claim_token`）；stop 走确定性词表（不依赖 LLM），取消在途 outbox + 写长冷却（`reaction.rs:251-267`）[B]。

**单轮成本口径** [B]：普通寒暄全链路约 3–5 次 LLM 调用（decision 1 + review 1 + claim_gate 1 + 投影 1 + reaction 视情况 0–1）；知识零本地相关时跳过知识 LLM（`knowledge_router.rs:661-679`）。`run_max_llm_calls` 默认 6（不含 reaction/投影的独立预算）。

---

## 三、核心机制事实卡

### LLM 层 [A/B]
- `generate_agent_json`（`agent/mod.rs:254`）是唯一 JSON 入口：LRU 精确缓存**白名单仅 4 个 key**（knowledge.import.preview / playbook.generator / playbook.optimizer / user.guide.preview，`mod.rs:828-835` [A]）——**reply/review 热路径无缓存**；shadow 模式永不缓存（`295-297`）。
- 输出 token 上限：fast reply 8192 / light review 3072 / full review 8192 / claim_gate 3072（`mod.rs:233-245` [A]）。
- LlmRegistry 按 workspace 单 active provider，30s TTL + generation 同步热切换（`llm.rs:2126-2219`）[C]；重试指数退避+jitter 封顶 60s，JSON 解析错误不重试（`llm.rs:1327-1370`）[C]。

### 知识红线 [B]
- AI 写路径（import/PDF/vision/RSS/chat/repair/长任务）一律强制 `draft + needs_review`，核心落点 `chunk_revisions.rs:201-244`（source=Ai 或敏感字段 patch → 强制降级+confidence=0）；auto-verify 的"verified"判定强制降为 `needs_human_audit`（`verify.rs:490-496`）；唯一进 verified 的通道是人工 verify + D2 证据闸（`verify.rs:60-132`）。刻意例外：`ProvenanceSource::PrincipalAuthorized`（领导授权，非 AI 自证）。
- 生产召回只暴露 `active + verified`（`knowledge_router.rs:64-85`）；knowledge_agent 工具循环 MAX_ROUNDS=4。

### 学习回路现状（重要——两代信号并存）[A/B]
- **动态置信度已换血**：`DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED` 默认 **true**（`config.rs:664-667` [A] 亲读）；Hit=`user_replied_buying_signal`、Block=五类负向、沉默=删失（`gap_signals.rs:760-811`）；置信度真实参与召回排序 `rank_key`（`knowledge_agent.rs:1825-1842`）。
- **演化器仍用过程指标**：`SEND_SUCCESS_STATUSES = ["approved", "revision_applied_approved"]`（`significance.rs:41-42` [A] 亲读）= 5 闸评审放行率，非业务结果；`negative_reaction_rate` 仅观测不参与 promote/rollback（`config.rs:269-276`）[B]。threshold shadow replay 不调 LLM，用源 run scores 重推闸门（`replay.rs:293-346`）[B]。
- 疑似成交：Reply Agent 输出 `suspected_deal` 弱信号（prompt 指引 `entitlements.rs:285-306`）→ pending 专表 → 管理员 approve 才落 `staff_confirmed` outcome_event（`admin_suspected_deals.rs:118-219`）[B]。

### 请示通道（Ask-Human）[A/B]
- 三类触发 category：out_of_scope_decision / high_risk_gated / stuck_or_undelivered（`models.rs:4536-4540`）。
- `AskHumanPolicy` 完整字段：decider_chain / escalate_safety_guard(默认开) / escalate_unverified_product(默认开) / escalate_ai_policy_hold(默认关) / escalate_stuck(默认开) / dedupe_window_hours / daily_push_cap / quiet_hours(推卡用，与客户触达 quiet hours 是两套) / timeout_hours（`models.rs:1429-1448` [B]）。
- 超时行为：`scan_escalation_timeouts`（`tasks.rs:1138` [A]）→ 有下一位则改派重推卡（`escalation/mod.rs:559-658`），链尾则周期 ChainTail 安抚、**不自动关单**；**全库无"预授权底线/standing order"机制**（子代理全库搜索无命中）[B]。
- relay 转述：`__PRINCIPAL_RELAY__` 哨兵合成 inbound 重入 gateway；出站守卫禁止泄漏内部载荷、禁止引入 substance 之外的数字（`escalation/logic.rs:206-269`）；授权带 `authorization_expires_at`（领导口述时限才写）。
- 客户嘴上"找人工/转真人"**不构成**请示触发（看事项实质）；AI 回复中除"我"不得出现任何可兜底/接收诉求的角色（Soul 红线，`prompts.rs:1058-1071` [A] 亲读）。

### 辅助模式引荐 [A/B]
- 判定优先级：客户级 `assist_mode_override`(force_on/off) > 账号级 `assist_mode_enabled` > 默认关。
- 名片必须 `enabled + approved` 双门才可被 AI 选（gateway 二次校验）；推送后写 `referred_specialist_at`，AI 转被动答疑；台前顾问 ≠ 幕后决策源（解耦）。

### 对话模式与 Soul [A]（`prompts.rs:993-1299` 亲读）
- 四模式判定优先级（policy v3）：运营特别指令 > 客户阶段(评估/决策→consultative) > 产品问题 > 明确边界 > 有价值可分享 > 默认 casual。
- shouldReply=false 仅三种：用户明说先不聊 / AI 刚回且用户未表态 / 非真人探测；寒暄一律必回。
- 承诺必填联动：replyText 有时间承诺必须写 lastCommitment/commitment；正式承诺与 followUp 在送达确认后生效。
- escalation 自洽自检：回复里说了"要请示"就必须 emit escalationRequest，否则自相矛盾。

### 评测体系现状 [B]（子代理核证 + 关键行抽查）
- **仪器厚**：nightly 真模型 14 套件（ops 15 项、knowledge_quality Q1–Q8 带 overall<6.0 硬门、adversarial 8 弧含 19 条 judge 校准金标、roleplay 动态博弈 4 轮）；judge 仪器完整（K 采样取中位、good/bad 校准 gap≥2.0、跨裁判分歧>3.0 剔除、有效裁判取 min 保守裁决，`real_llm_knowledge_quality.rs:403-433,685-724`）；shadow replay 支持对历史 cohort 批量重放 prompt 候选（evolution 路径）。
- **金标回归环薄**：生产 `evaluation_scenarios` 启动只种 1 条（`main.rs:412-457` [A]）；hardening spec R18 明文"不构建大规模标注集"；ops/adversarial 的质量分是纯诊断不断言（`real_llm_ops_smoke.rs:650`）；厚测全在 nightly 软链（仅 redline/skip-gate 硬），不进 PR 门；无"改 Soul → 分钟级金标硬回归"一等入口。formula-adherence 路径（UI→simulation→比 ground_truth）技术上最快但场景默认仅 1 条。

### 规模事实 [A]（亲跑统计，2026-08-13）
- 后端 src 192,582 行 Rust；tests 85,273 行（182 个 .rs）；前端 67,839 行；docs+.kiro 文档 141,146 行（165 篇 superpowers spec）；scripts 16,222 行。
- 路由 235 条；集合约 70；迁移 m001–m058；supervisor worker 16 个（6 个默认关）；前端 20 频道（2 个 comingSoon 占位）。
- `auto_release.rs:40-58`：`CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 政策硬闸，自动发布恒返回 0（刻意死代码）[B]。

---

## 四、评审结论存档（v2，2026-08-13）

### 被代码修正的 v1 判断（引以为戒）
1. "9 字段独白形式主义" → 已退役（见偏差表 #2），批评撤回。
2. "评测几乎没有" → 收窄为"仪器厚、金标快速回归环薄"。
3. "请示超时无产品化" → 超时转移已实现；残余缺口仅"预授权底线"。
4. "成交纯手动" → 已半自动（嗅探+排队，认定人工）。
5. "置信度建在 reviewer 自评噪声上" → 已换血为真实用户反应（默认开）。

### 核证后成立的核心缺陷
1. 演化器目标函数是评审放行率（过程指标），与已换血的置信度回路两代标准并存——最实锤的改进点。
2. 静默时段入站无差别延迟（最长约 10h15m），而 relay 豁免证明精细豁免是现成模式，可低成本扩展到高意向 inbound。
3. 发送节奏与文本长度无关（拟人穿帮点）。
4. 寒暄轮成本：review+claim_gate 硬绑 should_reply 无分级跳过；热路径无缓存；单 provider 无模型分级。
5. 冷启动哑火：无 verified 背书的 consultative 只能澄清/承诺核实——但注意 R5.4 有三路背书（产品目录定价也算），录入产品目录可部分缓解；真正缺的是知识批量授权工具链。
6. 过度工程化实锤：6 个默认关 worker + auto_release 死代码 + 单实现 trait + 多租户骨架配单实例 + 文档量≈后端代码 73%。

### 建议优先级（v2 修正后）
- P0-1 金标回归环 = 拼装工作（judge 仪器/simulation/CRUD 全现成，缺场景库+硬门+一键命令）。
- P0-2 静默时段高意向豁免（照抄 relay 豁免模式）。
- P0-3 演化器目标函数换血（复用 `classify_outcome_label` 材料）。
- P0-4 寒暄轮 light-review-only 开关。
- P1 减法：auto_release 死代码、未消费采集形状、文档台账瘦身、拆 gateway.rs、前端 IA 收敛（项目有做减法先例：retired full task、5 闸收敛、探针改观测）。

---

## 五、已核证缺陷/矛盾清单（2026-08-13 深读工程产出，主会话亲证级 [A]）

以下每条均经主会话亲读源码确认，非子代理转述：

| # | 缺陷/矛盾 | 亲证锚点 | 严重度 |
|---|---|---|---|
| 1 | ~~静默醒来任务被 daily_limit 取消→客户消息永远得不到回复（高）~~ **【22 号终裁重大修正 2026-08-13：主路径不成立，降级为 legacy 残留】**`DEFERRED_INBOUND_REPLY_KIND` 全仓**无创建点**且代码自标 legacy（`webhooks.rs:716-717` reconcile 的 `is_legacy` 判定）；现行静默唤醒物化的是 `inbound_reply`（`webhooks.rs:117` 主会话亲证）→ Inbound 语义 → daily_limit/rate_limited **天然豁免**（`5289-5296` 只拦 FollowUp）。gateway 的 deferred_wake 分支 4 处是防御历史残留行的死代码。原核证的三个锚点**行为均真但分支不可达**。残余：DB 历史 deferred 行可被 cancel（reconcile 会收敛） | 22 号终裁+主会话二次亲证 | 低（legacy 残留）。**方法论教训（第二次）：缺陷判断必须同时验证分支内行为与分支可达性（创建点）** |
| 2 | ~~毒丸消息行~~ **【已修复 2026-08-13 线 A commit 8d51b70】**decode 失败行按 `_id` 直更 `handoff_status=quarantined`（raw 路径+CAS 防并发）+ 审计事件，tick 继续处理后续行；quarantine best-effort 失败留 pending 下轮重试。集成测试 `poison_inbound_handoff_integration` 本地 Docker 跑绿 | 原锚点 `webhooks.rs:819-822` | 已关闭 |
| 3 | 双脑 second reviewer **parse 失败拉闸整个 run**，与同函数注释"调用失败仅 warn 不阻塞、双脑不应成为新故障源"矛盾（LLM 调用失败确实只 warn） | `review/mod.rs:4390`（注释）vs `4409-4415`（行为） | 中（仅 REVIEWER_DUAL_ENABLED 开启时） |
| 4 | ~~知识窗口错位~~ **【已修复 2026-08-13 B5 commit 326706a】**cited 复核改为与窗口同口径的 DB `$in` 直查；窗外 verified 文档经 `KnowledgeRouteResult.cited_verified_chunks`（`#[serde(skip)]` 运行时字段，四处持久化面零变化+键名单测守卫）在 `select_operation_knowledge_chunks` 内并入下游（gateway 零改动）。反事实红态验证+201 条场景集成测试绿 | 原锚点 router/agent 窗口 | 已关闭 |
| 5 | ~~manual_send 两道门裁决矛盾~~ **【已定案修复 2026-08-13 线 A commit bfc0395】**保守语义定案：manual_send 与托管发送同受"撤管即停"约束，删除不可达豁免死代码、两处注释改为如实描述 | 原锚点 dispatcher 两门 | 已关闭（语义定案） |
| 6 | evolution 灰度旗 `updated_by` 取请求体可伪造审计身份（手边有 `Extension(admin)` 未用，违背 ReviewActor 服务端身份先例） | `evolution.rs:742-746` | 中（审计完整性） |
| 7 | `models.rs:1802` 注释引用的迁移 `2026_05_W1_001_chunks_wiki_type_default` 不在注册表（幽灵引用，wiki_type 旧文档实际恒 None） | `models.rs:1795-1804` + migrations 注册表 | 低（注释误导） |
| 8 | **两个空壳测试永远绿**：`revision_recheck_action_gate.rs`（零断言纯注释）、`memory_card_write_occ.rs`（仅启动容器）——revision 后动作闸复检与 memory_card OCC 两条安全不变量无可执行守护（作者注释诚实自认"骨架"） | 两文件亲读全文 | 中（覆盖幻觉） |
| 9 | 锚点口径不一致：B3 修复统一 `chunk_has_citable_anchor` 后，crud/verify/digest_inbox/catalog 四处报表仍用裸 `!is_empty()`——畸形锚漏报进错误分类（verify 主闸不受影响） | `crud.rs:543-547` 亲证一处 + `models.rs:1926` | 低-中 |
| 10 | **未提交改动含禁词**：`import.rs` 新增行两处"人工核验"命中 CI no-human-takeover lint（正则含独立词"人工"、扫 src/routes/）——**这批工作提交进 PR 时 baseline 必红**，需先改措辞 | `git diff` 亲验 grep 命中 2 处 | 高（对当前工作） |
| 11 | 自动摄入（ingest）**正向成功链路零集成覆盖**：smoke 四测试全为 SSRF 拒绝/跳过路径，文件头注释与实现不符（worker 默认关，启用前应补） | `tests/ingest_worker_smoke.rs` 测试函数名亲验 | 低（默认关） |

| 12 | **复刻式测试实质漂移 2 处 + 幻影状态值 1 处**：`escalation_push_time_reassign` 用例 1 锁 `$set last_pushed_at_ms` 而生产为 `$unset`（`ledger.rs:1111-1114` 主会话亲证）；`autonomy_protocol_pbt` P2 模型缺 `apply_revision_fallback` 分支、断言与生产相反（`gates.rs:1258-1275`）；`dry_run_isolation` 用例 2 的 `status="completed"` 不在生产闭集（生产终态 succeeded）——三者给出假信心 | 28 号裁决 + 主会话抽证 | 中（测试可信度） |
| 13 | **生产行为无测试守护清单共 19 条**（改坏不会红的区域地图，按风险排序含 deferred_wake 取消、毒丸行、知识窗口错位、HP-1 回收、GATE-1 复检、revision fallback 接线等） | 28 号 §4 | 改动前必查 |
| 14 | ~~"被引用"功能恒 400~~ **【已修复 2026-08-13 线 B】**后端加 `serde(alias="target_id")` 双认 + 前端改发 `targetId` + 注释改正 | 线 B commit（B1） | 已关闭 |
| 15 | ~~产品 active 过滤静默失效~~ **【已修复 2026-08-13 线 B】**同款双保险（alias + 前端 `activeOnly`），归档产品不再混入圈人下拉 | 线 B commit（B2） | 已关闭 |
| 16 | **演化器 pressure gate 统计源失真**：`threshold.rs:69` 把 `blocked_by_safety_guard` 归因 pressure_risk_block 命中，但生产该终态来源是 R5.3.a fail-closed/业务声明拦截（`gates.rs:473,779,818`），pressure 是软闸不产此态——pressure 阈值候选建立在错误数据上、#152 反向门对该闸空转（significance/auto_release 同口径失真；post_release 侧口径已修正） | 23 号终裁+主会话双侧亲验 | 中-高（演化器可信度） |
| 17 | ~~领导带时限裁决 → 前端崩溃~~ **【已修复 2026-08-13 线 B】**两处序列化改 RFC3339 字符串（线 B 重验发现 domain_profiles 已修方案实为 `dt_to_string`→RFC3339 而非计划所写毫秒，按"逐字对齐实际"采用；前端 formatExpiry 防御性兼容毫秒/对象/字符串三形态；附契约快照 fixture） | 线 B commit（B3） | 已关闭 |

**排除的疑点**（核证后不成立）：前端 DomainProfileDraft 不回传 `generated_state_machine` 会丢 AI 状态机草稿——不成立，后端 PUT 是剥离管理键的部分 `$set` 更新，未编辑字段原值保持（`domain_profiles.rs:1149-1153,1341`）。

**裁决的误报**：17 号记录初判"user.reply.task 仍在生产"——误报，见偏差表 #2 精确化表述。

**D/E 组小尾巴清理（2026-08-14，主会话直做）**：29 号 §6.4 D/E 两组 5 条全闭，"仍存"清单归零（除 A 组已由 chore 尾波关闭外无余量）。DIV-36：tripwire include 名单补 12 文件（29 号漏数 contract_snapshot.rs），抓出的 2 个 pub async fn 经仲裁均为合法 helper 进 KNOWN 名单（reconcile_campaign_dispatches / append_domain_profile_draft——后者首判降私有被编译器纠错，rg `-r` 参数污染教训入档），apply_update_chunk 死条目删除（线 B 滞留登记闭）。DIV-15/33/34/35：集合名修正、plan 勘误注记、deploy.sh 弃用横幅、.env.example 补两族 10 变量。验证：lib 2563/0（+1 系用户 WIP 自带测试，agent 改动零测试数变化）、`-D warnings` 绿、禁词 0 违规。

**chore 尾波终局（2026-08-14，merge 3f2a88c，两阶段完成）**：首发 runner 因单上下文装载过重中断（遗留 9d22429/fac9fe9 两个有效 commit + 若干散件），续做 worker 完成残余并逐一处置散件（回退 `finalize_ingested_content` pub 化半成品——`ingest_worker.rs` 本有 redline 测试入口，sr117 在用；回退无关的 quality_gold.rs 纯格式化）。**29 号 §6.4 A/B/C 三组 35 条全闭**：A 组 26 条注释漂移对齐（9d22429，被触及文件采纳"注释不写行号"约定——DIV-43 治本首批）；B 组 4 条测试幻觉全部落实为真测试（DIV-30 fac9fe9 真回收 3/0、DIV-31 a0ca8c7 真 claim→finalize 正向链 5/0、DIV-32 6cff3ce 真删除请求 1/0、DIV-71 17b45bc 治本抽 `COPY_*` 常量使生产与禁词测试同源）；C 组 5 条前端清理（0551343），其中 **DIV-67 半翻案**：`Contact.lastConversationMode` 经亲验为幽灵键（后端 ApiContact 不下发），29 号原"类型落后于下发"描述对该键不成立。D 组（DIV-36 tripwire 名单）与 E 组（4 条未授权文件）继续挂账。合并验证 lib 2562/0 + 禁词 0 违规；worker 侧前端 750/750 + build、三组 Docker ignore 绿。**合并时主 workspace 存用户进行中新 WIP**（`settled_ai_reply_update` @ webhooks.rs + outbox_dispatcher + biz-test 扩展，8 文件 260+/43-，系 19 号"未提交工作全景"之后用户的持续开发）——经 stash→merge→pop 零损保护，agent 未触碰其内容。

**第四波·文档准确性收敛终局（2026-08-13，线 F 706fa5d + 线 G 4e0ec6f）**：核心活文档 11 篇约 56 处修正（CLAUDE.md 四指令节零变化经主会话独立 diff 复核；一次自我纠错恢复误删的 open-evidence 别名行）；specs sunset 注记刷新（正文零改写）；三评审台账已关闭条目标注；website 11 文件修正（"谁错改谁"复核：trust 页对、technology 页错；6 处默认关闭能力诚实角标；数字全面更新）；**29 号偏差表 71 条终态核销：已修 16 / 已标注 10 / 仍存 43（全部给出归属）/ 失效 2，高严重度 4 条全部收口**。仍存 43 条的主体（26 后端注释 + 4 测试幻觉 + 5 前端死代码）交由 chore 尾波处置。

**S5 波次终局（2026-08-13，全部完成）**：任务 2 ✅（分支推送、PR #280 全貌更新）；任务 7 ✅（字段族分叉裁决入 30 号；models.rs 注释补写**取消**——struct 本体已无该字段族，仅存 legacy 兼容测试 fixture，档案记录已足够）；**线 E 合并**：S5-5 预授权底线落地（`apply_standing_order_if_due` 前置于链尾安抚，复用 resolve→relay 零新发送路径，`resolved_via="standing_order_policy"` 第三值——锚点修正：PrincipalDecision 无 decided_by 字段；双字段成对校验防静默误配；前端 policyForm 新 section）；S5-6 寒暄成本刀落地（`should_skip_claim_gate` 七条件纯函数，`outcome=None` 既有"未评估"语义跳过——R5.4 主源亲验为 Reviewer 的 claim_analysis、ClaimGate merge 仅增强，hold_for_claim_gate_failure 不可误达单测锁死；审计标记 `claim_gate_skipped_casual_low_risk` 落 risks；仅主程首稿分流，rewrite/revision/管理发送恒照跑）。**终态验证：baseline OK lib=2562/pbt=41、前端 750/750、三 lint 全绿**。任务 1（金标首跑）仍挂起等用户 key。**线 D 合并（c35f4c2）**：S5-3 静默时段显式交易意图豁免落地（`bypass_deferral_for_explicit_buying_intent` 于 quiet_hours.rs，与 reaction 确定性购买下限同词表同 profile 门；两处 defer 分支收敛为 `schedule_managed_inbound_obligation` 消灭双写漂移；bypass 事件带 dedupe）；S5-4 长度加权节奏落地（`account_send_interval_ms` 四参签名：35ms/字、封顶 max+6000、**0/0 闸关恒 0 且 0 字符与旧行为逐值等价**——台账缺陷 #10 相关"pacing 签名无长度参数"的旧事实已过期）。lib 基线 2534→2540（+6 新单测）。新登记（环境性旧问题，非本线引入）：`outbox_integration` 在 macOS 默认线程栈爆栈，`RUST_MIN_STACK=16MB` 全绿——与 principal_decision_channel 爆栈同族，待统一处置。

**线 C 合并追记（2026-08-13，66dd451 + 主会话收尾 cb90cb4）**：缺陷 #3（双脑 parse 回退）/#6（updated_by 服务端身份）/#8（两空壳测试落实——`memory_card_write_occ` 4 路并发 OCC 经公共 API 驱动零生产改动、`revision_recheck_action_gate` mock 六段编排驱动真 gateway）/#12（三处复刻漂移修正）/#16（pressure 统计源——`blocked_by_safety_guard` 终裁归 None，pressure 候选降级为不生成带 skip 事件，#152 空转消除）全部关闭；auto_release 死代码净删 709 行（lib 基线相应 2541→2531，官方门 OK）；**金标回归环 v1 建成**：105 条合成场景（五类×21，metadata.source=synthetic-v1）+ `tests/quality_gold_regression.rs`（shadow 零发送、红线硬门+judge 软门+JSONL ledger）+ `scripts/quality-regression.sh` 一键入口 + ci.yml nightly `quality-gold` job（软门起步，缺 key 真 fail 不假绿；judge 默认关，首批积累红线信号）。首次真跑 ledger 待有 REAL_LLM key 环境。主会话顺手清理：`db/mod.rs` 死 API 注释、`routes/tasks.rs` 死 kind 字符串（线 A 登记项闭）。10 号 §5 四条与 28 号 §4 对应项随之关闭。

**线 B 合并追记（2026-08-13，ce1c4d5，B5 增量另计）**：缺陷 #14/#15/#17 关闭（上表）；#9 锚点口径四处统一（收紧方向，红线 19 处零弱化）；execute_step 死路径删除（B6，两阶段提交为唯一路径，dismiss 的 account 过滤缺失随之消灭）；`user.reply.task` 种子退役落地（B7——align 语义经亲验安全：只遍历 spec 清单不枚举 DB，历史行零触碰；30 号事实卡该行需更新"种子不再种入、治理面已收缩"）。**B8 半项重要翻案**：`safe_claims/forbidden_claims/evidence_items/routing_card` 字段族在 chat/repair/catalog 链路是**活的**（prompt 仍要求输出、`catalog.rs:535` 生产查询 evidence_items）——"已删死字段"认知仅对 user-ops 主链成立，跨链路口径分叉归档为**产品决策项**（S5 打包）。新登记：`routes/mod.rs` KNOWN_NON_ROUTE_HANDLERS 的 `apply_update_chunk` 滞留条目、`website/agents.html:116` 仍展示 user.reply.task（越界未改）、DIV-40/48 的 gateway/decision 注释残留（线 A 已清 gateway 侧）。

**线 A 合并追记（2026-08-13，e2e59ba）**：缺陷 #2/#5 关闭（上表）；deferred_wake legacy 分支已物理删除（净删 103 行，缺陷 #1 残余风险归零）；delivery_unknown 请示卡滞留已修（新增 `list_stranded_delivery_escalations`，pending ∧ {failed_terminal, delivery_unknown, sent 缺推送时刻} 按 created_at 计龄进超时改派，`escalation_stranded_delivery_timeout` 3/3 绿）；两条新登记：`routes/tasks.rs:81` 残留死 kind 字符串（越界未改，待 B/C 后仲裁）、`principal_decision_channel::blocked_relay_preserves_awaiting_...` 单测在 debug 构建确定性爆栈（基线 d99b6e7 即存在，非线 A 引入，待修）。

**重要待重验疑点 [C]**（子代理发现、主会话未逐条抽查，引用前必须重验）：hold 请示被骚扰门拦时零台账（`escalation/mod.rs:231-233`）；~~delivery_unknown 请示卡不进超时改派静默滞留~~（已修，见上）；run_envelope 终态写无 lifecycle CAS（`run_envelope.rs:690-693`）；prompt shadow 真实 LLM 消耗不计入 EvolutionBudget；quiet-hours runtime 加载粒度 contact/workspace 分裂；biz-test domain8 `severity="BLOCKED"` 误用致降级分支死代码；`.env.example` 缺 POST_DECISION/SILENCE_SIGNAL 两族变量；登录限流不解析 XFF（反代下全体共享一个槽）；两处 bson DateTime 扩展 JSON wire 残留；死路由 tripwire 名单缺 11 文件。

## 六、深读新增的重要机制事实（跨域级，详情见对应记录）

1. **ClaimGate 是治理体系外的硬编码审查器**：prompt 为代码内嵌常量（`review/mod.rs:340-354`），不在 prompt_templates/演化/编辑三闸管辖内——有意为之的不可篡改性（09 号）。
2. **BSON 命名三分**是历史事故高发区：多数 snake_case、Campaign/CampaignSend/LlmProviderConfig 全 camelCase、少数混合路径索引；多个带索引字段（post_decision_*、handoff_status、active_task_key 等）不在 typed 模型中、由 raw Document 写入（02 号）。
3. **webhook→worker 交接**是"确定性 _id 单行 durable 任务 + (created_at,_id) 水位 CAS"模型：单飞/复活/fence 三合一（03 号）。
4. **MCP 防重发三层超时不等式**：reqwest 60s < dispatcher send timeout 150s < lease 180s；错误二分 SafeToRetry/DeliveryUncertain；名片发送恒 Inconclusive→delivery_unknown 禁自动重放（05/09 号）。
5. **finalize 是九道硬门的顺序判定树**（协议违规→insufficient_detail→预算→证据时效→开放世界业务门→R5.4 三路背书→观测探针→hold 矫正→末端四分支）；fast 契约不校验 why_should_reply 长度使 insufficient_detail 分支主链路不可达（04 号）。
6. **迁移两代风格**：m001–m032 回填/seed；m038 起"全量校验先于首写、歧义 fail-closed 炸启动"；仅 6 条破坏性迁移受 APPROVED_MIGRATIONS 双 gate（02 号）。
7. **task-status-manifest.json 是任务完成状态唯一权威**（曾发生三 spec 全勾 [x] 账实不符，SR-179）；47 域历史审计全判 inconclusive（SR-183）只能当线索（17 号）。
8. **守卫哲学单向演进**："字符匹配→语义判断"贯穿 165 篇 spec（评测词表下线、删 relay 数字护栏、泄漏词表裁决不加）；确定性代码闸只留给客观集合运算（18 号）。
9. **测试强度分布与代码防御强度同构**：送达可靠性/红线=硬断言（outbox 23 测试用阻塞式 MCP 精确控制越界时刻），语义质量=台账观测；mock LLM 按 system prompt 锚文本路由是隐式契约，生产 prompt 措辞改动会让集成测试队列错位（15/16 号）。
10. **当前未提交工作全景**：6 组后端行为变更（Lean/Full 强升+引荐分层、确定性 stop/buy 下限、knowledge runId 归因、Guide 只读化、activate 恢复默认状态机、记忆冲突审计）+ biz-test 验收硬化，src 与 scripts 必须同批合入（19 号）。

## 六.五、生产运维开放事项（来自 20b 号 system-review 亲证，2026-08-13）

- **HC-001 生产凭证轮换 8 项硬门仍开放**：据台账记载 07-30 暴露过的凭证仍有效未撤销——最高优先级运维待办（非代码改动）。
- 真实模型成功证据 7 项、浏览器/杀进程/真实外部 MCP 演练 5 项、Actions 真跑等 4 项仍开放（外部环境阻断类）。
- 读历史遗留表注意：2026-07-26 曾全量 1340 文件切生产，大量"deployment-pending"条目**代码已上线仅缺部署后验证**，与"未实现"是两种状态。
- 21 条历史遗留经 20b 当日代码亲证已解决（含 smoke 全部 findings、B-1 预算、SR-138 破坏性 reset）。

## 七、未核证/存疑清单（诚实边界，禁止当事实引用）

1. MCP server（GeWe）实际工具清单/schema 未见书面依据（referral spec 自注 `message_send_namecard` 仅口头确认）。
2. 生产环境真实运行数据（客户量、日消息量、真实成本、闸门实际命中率）完全未知——所有成本/延迟判断是代码推算口径。
3. 前端各频道的实际使用频率未知（"三套交互冗余"是结构推断，非使用数据）。
4. biz-test 依赖生产机 SSH 环境，本会话未实际执行任何测试/构建（19 号记录含 `cargo check` 通过一项，为子代理执行）。
5. 残余未逐行区：frontend/__tests__ 部分文件体、docs/system-review 26 文件仅头部+结论、superpowers plans 149 篇仅清单+抽读、website/ 营销站。
6. 各深读记录的 [C] 级疑点（见第五部分末尾清单）在用于决策前必须逐条重验。
