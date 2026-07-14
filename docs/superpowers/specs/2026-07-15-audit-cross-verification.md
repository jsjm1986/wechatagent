# 六批审查台账 High/Medium findings 对抗式交叉验证报告

> 用 workflow 扇出，每条 High/Medium finding 派一个独立 agent 亲读实际代码、**对抗式试图证伪**（主动找台账忽略的兜底/守卫/上游约束），逐条核实缺陷真实性并校准严重度。每个验证 agent 均带完整红线（先 100% 读懂 + file:line 亲验 + 部署拓扑校准），明确要求「绝不轻信台账断言」。数据源=workflow journal.jsonl 的 18 条 per-agent 结论（synthesis agent 声称写报告但未落盘，本报告由主控据 journal 权威数据整理）。

## 总账

- **验证条数**：18（六批全部 High/Medium findings）
- **代码事实**：**100% 属实**——18 条台账断言的 file:line 引用与代码行为逐条亲验成立，无一条凭空捏造或引用错位。
- **Verdict 分布**：CONFIRMED 14 / OVERSTATED 4 / UNDERSTATED 0 / REFUTED 0 / UNVERIFIABLE 0
- **偏差方向**：单向——4 条 Medium 高估，校正为 Low；无一条低估。严重度校准准确率 14/18；4 条偏差全部是「观测/桩层、无客户面后果的功能缺陷被定成 Medium，实为 Low」。
- **唯一 High（Batch3 [1-01]）经独立复核站得住**，且验证 agent 发现比台账更严重的细节。

## 校正项（4 条 Medium → Low）

| finding | 批次 | 校正理由（对抗式复核） |
| --- | --- | --- |
| **[A-02]** confirmed_tags 截断窗口 replace 静默清除持久标签 | 1 agent-capabilities | 丢的仅是 AI 推断「确信层」软标签；运营录入的权威 `manual_tags` 单键隔离完全不受影响；下游全为软影响（prompt 渲染「AI 判断标签（可能调整）」+ churn 漂移信号，无硬闸/发送/状态机读取）；证据在后续对话复现即自愈；且「忘掉旧结论、无对话依据不保留」是标签可信度改造**刻意的 fail-closed 设计意图**（prompt:1453-1456），非意外数据损坏。→ 观测/设计张力级 Low |
| **[1-01]** knowledge_gap_signals 无业务去重 unique 索引 | 4 knowledge-wiki | 代码事实属实（find-then-insert 无唯一索引，且单进程内 route-vs-worker/并发 webhook 确有真并发源，非纯水平扩展债），但 knowledge_gap_signals 是**纯运营侧观测/待审集合**（LintView 仪表盘/ask_human_inbox/observability 消费），网关/决策/发送链从不读取；重复信号后果仅是仪表盘多一行，无数据丢失/重复发送/决策影响。→ observability 去重质量债 Low |
| **[3-01]** lessons_learned 幽灵字段 `user_polarity` 恒命中 0 | 4 knowledge-wiki | 代码事实属实（success/failure 两支查错集合 agent_run_logs + 字段全仓零写点，count 恒 0，闭环 inert），但 lessons_learned 是 **admin-review-gated 的 peer_case 候选池**，永不自动晋升 chunk、零客户面；且 failure 支目标的 reviewer-misjudge 信号已由并行的 reviewer_stats 路径独立捕获（部分冗余）。→ 观测层功能缺陷 Low |
| **[3-02]** lessons_learned blocked 模式 filter 自相矛盾恒 0 | 4 knowledge-wiki | 代码事实属实（`lifecycle=completed` 与 `blocked_by_safety_guard` 被 derive_lifecycle_from_status 保证互斥，交集恒空），但同属 observation-only 候选池、下游不反哺 decision/gateway，后果仅是一个 analytics category 永远为 0（相当于错接线的桩）。→ 桩未接线 Low |

## 唯一 High 复核（Batch3 worker-fleet [1-01]）—— 站得住，且更严重

**initial_profile 任务成功后无终态写入** 经独立 agent 逐条亲验 CONFIRMED，严重度 High 恰当：

- 四个 task kind 里 memory_consolidation/outcome_aggregation/follow_up 都在各自 handler 内部写 tasks 终态，**唯独 initial_profile**（contacts.rs:668-717）全函数只碰 contacts、三条早退全 Ok(())、成功路径也不写 tasks；tick 的 Ok 分支（tasks.rs:242）只写事件不代写终态 → 任务永停 running。
- 确定性失效链：claimed_at 已写 → heartbeat 于 handler 返回后 abort → 默认 300s 后 reclaim 判 stale → 重置 retry 重跑（build_initial_operation_profile 真实烧 LLM + apply 覆写画像）→ 累计 recovery≥3 强制误判 failed。
- **验证 agent 发现比台账更严重的细节**：AgentTask.claimed_at 是 **null-present**（无 `skip_serializing_if`，对照 ImportJob 显式 skip），故 reclaim 分支 B（`$exists:false`）对正常任务恒为死分支——任务**重启也不回收**（台账原说「仅重启才兜回」过于乐观）。
- 危害落在**批量托管核心 onboarding 路径**（batch_enable 每客户入队 1 条），每次执行确定性命中：3×全量 LLM（2 次纯浪费）+ 成功任务误标 failed + 对已被 gateway 互动富化的 agent_profile/profile_attributes 的覆盖（活跃 managed 联系人在 300-900s 窗口内互动属常态 → 真实画像数据损坏）。

**此条仍是六批工程的全局 P0**，优先级高于所有 Medium。

## CONFIRMED 且 Medium 校准恰当的 13 条（摘要）

- **[A-01]** core_facts 缺证据锚定门（与 tags/personality fail-closed 不对称，且 core_facts 被读进每轮 reply prompt）— 需 LLM 采信无佐证陈述，非确定性，Medium。
- **[A-03]** consolidation 跨集合写非原子（memory_card 写成功后 confirmed_tags 失败 → 候选重放）— 需故障注入 + 有合并去重自愈，Medium。
- **[C-01 批1]** stagnation_dimension 读写不对称（读侧全动态、写侧写死 customer_stage_updated_at）— 需运营配非默认停滞维度 + 有 fail-soft 回落，Medium。
- **[C-02 批1]** 初始画像通用化只做一半 + 残留销售 schema — 非销售域功能门显式开启才触发 + 首条 inbound 自愈，Medium。
- **[1-01 批2]** enable_agent account 存在性校验漏 workspace 作用域 — 多租户 + 共享 account_id 才触发、内部只读无 PII，单租户不可达，Medium（多租户就绪债）。
- **[4-01 批2]** taxonomy 7 handler 零 workspace/scope 隔离 — 多租户 + 多 admin 才可达、无 RBAC、字典非 PII，但可写（invalidate 全局缓存重驱动 agent），Medium（多租户就绪债）。
- **[S-01 批3]** review_task_now 绕 claim CAS 与串行 worker 真并发 — admin 手动触发 + 两次 LLM 文本相异才击穿 outbox 幂等，Medium。
- **[S-02 批3]** review_task_now 置 running 不写 claimed_at 落 reclaim 双分支盲区 — admin 手动 + handler Err，可 cancel 兜底，Medium。
- **[S-08 批4]** apply_chunk_revision 读-改-写无乐观锁 lost update — 单进程 axum 真并发可达，但仅标量字段受损（数组 union 兜住）+ chunk_revisions 历史可 rollback，Medium。
- **[2-01 批4]** ingest 落库零内容去重 — INGEST_WORKER_ENABLED 默认关不可达 + 开启后重复 chunk 恒 draft 不进 verified 池，Medium。
- **[S-01 批5]** runtime_flag=None 被 cohort 当全量收（灰度 fail-safe 反向）— 默认可达但产出一律 awaiting_admin + auto_release 双闸默认关，Medium。
- **[2-01 批5]** post_release 面板 5 闸 delta 用对调 status 映射贴错标签 — 纯观测不反哺 promote/rollback，但喂给人工回滚决策的反转数据，Medium。
- **[C-01 批6]** m011 delete_many({}) 清空 operation_knowledge_chunks（wiki 存活集合）— 需 m011 未入账 + APP_ENV 未设 production 双条件（灾备恢复场景），已入账永不重跑，Medium；一旦触发即永久丢失 verified 知识。**生产实证需求**：查 117 migrations 集合是否已入账 m011 + APP_ENV 是否设 production。

## 结论

1. **六批审查零虚报**：18 条 High/Medium 代码事实 100% 属实，无证伪、无引用错位——台账质量经独立对抗验证过关。
2. **严重度略偏保守**：4 条 Medium 应为 Low（全在 Batch1/Batch4 的观测/桩层），偏差单向（宁高不低），无低估——符合审查工程「不夸大」红线的安全侧。
3. **修复优先级不变**：唯一 High（Batch3 [1-01] initial_profile 无终态写）仍是全局 P0，且比台账所述更严重（重启也不回收）。其次是有真实后果的 Medium（[C-01 批6] m011 需生产实证 + 设 APP_ENV=production；[2-01 批5] post_release 面板贴错标签误导人工回滚；[S-08 批4] chunk lost update）。4 条降级为 Low 的观测/桩层缺陷优先级最低。
