# 后台 worker 群深度逻辑审查（第三批）—— 设计

> 接续第一批 agent 旁挂能力审查（20 findings，PR#207 已合）+ 第二批 auth+routes 安全隔离面审查（17 findings，PR#209 已合 0485e20）之后的第三批。用户裁定范围 = **后台 worker 群**——outbox 元家族的直接延伸，风险实感最强（worker 出错=静默漏发/重发/卡单）。方法论沿用前两轮，只审不修，产出 findings 台账后按 P0-P3 分批修。

## 背景与核心命题

上轮主链路 8 环节 + 第一批 agent 旁挂能力 + 第二批 auth+routes 都未深审后台 worker 群。本批圈定 `main.rs` 用 `spawn_supervised` 拉起的长驻 worker 中、memory 记载优先级最高的那组。

**体量（亲验）**：审查对象 ≈ 3131 行 —— `tasks.rs`(854) + `cold_contact_worker.rs`(581) + `behavior_signals.rs`(504) + `account_scheduler.rs`(390) + `silence_signal_worker.rs`(317) + `import_worker.rs`(300) + `supervisor.rs`(185)。

**worker 拉起点（亲验 main.rs:204-306）**：`spawn_supervised(state, "task_worker"|"import_worker"|"outbox_dispatcher"|"strategic_planner"|"cold_contact_worker"|"silence_signal_worker"|"evolutionary_worker"|"knowledge_digest_worker"|"knowledge_task_worker"|"catalog_rebuild_worker"|"knowledge_feedback_worker"|"ingest_worker", ...)`。supervisor 用 `catch_unwind` + 指数退避（1s→30s）兜 panic，正常返回不重启。

**核心命题不是安全隔离，而是数据一致性/幂等/崩溃恢复的正确性**：worker 都是 `loop { tick; sleep }`，共享的元家族是 **claim/CAS 竞态 + 崩溃回收 + 幂等 + 调度去重**——这正是上轮 outbox（幂等键 + second-pass 门 + 重试）元家族在**主动侧 worker** 的延伸。已亲验线索：`tasks.rs` reclaim_stale_running_tasks 用 CAS `{_id, status:"running"}` 回收 + recovery_count≥3 防死循环 + claimed_at 缺失回落 `APP_STARTED_AT`；`cold_contact_worker.rs:375` 声称 `cold_reactivation_idempotent` 不变量（"从下次调用起必然返回 Skip"）。找这些**声称的幂等/CAS/崩溃恢复不变量在实现层的竞态窗口/非原子写/回收遗漏/去重旁路**。

**边界排除（防与后续批次重叠）**：`outbox_dispatcher`（上轮主链路已深审 + F-01 补漏）、`strategic_planner`（第一批 agent 旁挂能力已审 planner）、`evolutionary_worker`（留 evolution 专批）、`knowledge_digest/knowledge_task/catalog_rebuild/knowledge_feedback/ingest_worker`（留 knowledge_wiki 专批）——本批不重复审，仅在 claim/幂等模式对照时引用。

## 范围与分簇（根因层优先 + 资源域分簇，4 簇）

- **簇S 根因层（worker 生命周期 + claim/幂等共享基准）**：`supervisor.rs`（全部 185：panic 兜底/backoff/重启语义/哪些 worker 不接入 supervisor）+ `tasks.rs` 的 reclaim_stale_running_tasks + claim/CAS 骨架（提炼共享基准）。**先单独审并等结论**——supervisor 重启语义是否安全（factory 重建 state、正常返回不重启的判定）、claim CAS 的原子性、崩溃回收的边界（claimed_at 缺失/recovery_count 上限）。此簇结论作为**审查基准**喂给簇 1-3。
- **簇1 派生任务执行 worker**：`tasks.rs`（854，follow-up 任务 tick/claim/执行/重试/stale 回收全链）+ `import_worker.rs`（300，import job 队列消费）。重心=claim CAS 竞态、幂等（同一任务不重复执行）、崩溃恢复（running 卡死回收）、recovery_count 死循环防护、attempt_count 与状态机一致性。
- **簇2 主动触达调度 worker**：`cold_contact_worker.rs`（581）+ `account_scheduler.rs`（390）+ `silence_signal_worker.rs`（317）。重心=调度去重（cold_reactivation_idempotent 不变量）、触达节流/冷却窗竞态、时间窗判定的边界、account 分配一致性、静默信号触发的幂等。
- **簇3 信号/画像 worker**：`behavior_signals.rs`（504）。重心=信号写入一致性、并发累积正确性（是否有 read-modify-write 竞态）、无界增长（write 侧是否有界，memory feedback_cautious_profiling 记 memory_summary 无界 append 待修的同类）、信号是否误进决策（tag_trust_reform 铁律"只写不进决策"）。

## 审查方法（沿用前两轮）

- **簇 S 先派并等回**，其结论喂给簇 1-3 当基准；簇 1-3 拿到基准后并行审。4 subagent 全继承 Opus（省略 model 参数——`model:"opus"` 报 400，省略即继承）。用 general-purpose（非 Explore——Explore 读摘录漏 read window 外内容不适合审计）。
- **subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；凭猜测打回。
- **一致性/幂等审查检查清单**（喂给簇 1-3）：每个 tick/claim/写数据的路径——①claim 是否用 CAS（find_one_and_update / update_one 带状态前置）保证并发唯一占用 ②幂等：同一触发重复执行是否产生重复副作用（重复发送/重复建 task/重复计数）③崩溃恢复：running/claimed 中途进程死，重启后能否回收（stale 回收边界、claimed_at 缺失处理）④防死循环：recovery_count/attempt_count 是否有上限终态 ⑤时间窗竞态：冷却/节流/静默窗的判定是否有 TOCTOU（读判定与写副作用之间的窗口）⑥非原子跨集合写：多集合写是否有中间失败导致的不一致（同上轮 A-03 元家族）。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造竞态/重复副作用/回收遗漏的触发序列）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 因果链成立性，驳回夸大。
- **元家族聚焦**：声称的幂等/CAS/崩溃恢复不变量，实现层是否有竞态窗口/非原子写/回收遗漏/去重旁路——上轮 outbox 元家族在主动侧 worker 的延伸。

## 严重度校准（一致性语境，仍防夸大）

- **High**：推荐配置（单租户默认、单进程部署）下**确定性可达的重复发送、丢失任务、数据损坏、或永久卡死**。
- **Medium**：需并发/崩溃时机叠加才触发、或仅在多进程/多副本部署下成立（当前默认单进程）、或有自愈路径兜底的短暂不一致。
- **Low**：观测/边缘/防御纵深/无界增长但无立即后果/就绪债。
- **⚠️ 关键校准原则**：**单进程默认部署下不可达的多副本竞态 = 水平扩展就绪债，不夸大成 High**（同第二批"单租户不可达=就绪债"口径的部署维度版本）。生产当前是单进程（117 单机 systemd），多副本竞态属就绪债。每条严重度带主控裁定理由。

## 台账格式与产出

- 新建 `docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md`。
- 字段沿用前两轮 + 复用"越权链"位置改为 **失效链**（谁在什么时机触发什么不一致后果）：入口 worker/所属簇/类型(claim竞态|幂等|崩溃恢复|时间窗竞态|非原子写|无界增长|就绪债)/严重度(带裁定理由)/现象风险/失效链/根因(亲验 file:line)/复现设想/验证状态(PLAUSIBLE|CONFIRMED)/修复建议/状态(Open)。
- **只审不修**：出完整台账 → 合并 docs PR（像前两轮 PR#207/#209）。

## 后续修复路径

台账产出后按严重度定 P0-P3。**若发现 High（确定性可达的重复发送/丢任务/卡死），优先级高于第一批遗留的 5 个 Medium + 第二批的 2 个 Medium**。每 finding 独立走 brainstorming→writing-plans→SDD→PR。

## 约束

- 纯代码/设计审查，绝不为"发现问题"改业务逻辑（反过拟合红线）。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）。
- 审查分支 docs/worker-fleet-audit 基于含 #209 的最新 origin/main（0485e20）。
- 本批产出纯 docs（台账）。

## 非目标

- 不审 outbox_dispatcher（上轮主链路 + F-01 已深审）、strategic_planner（第一批已审）、evolutionary_worker（留 evolution 专批）、knowledge_* worker（留 knowledge_wiki 专批）——仅在 claim/幂等模式对照时引用。
- 不审 worker 触发的下游业务逻辑（agent 决策/gateway/知识抽取，前几轮已覆盖），只审 worker 调度/一致性骨架本身。
- 不在本批做任何修复（只出台账）。
- 本批聚焦一致性/幂等/崩溃恢复主线；纯性能/可观测性若量大留后续。
