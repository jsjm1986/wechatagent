# 后台 worker 群深度审查 findings 台账（第三批）

> 接续第一批 agent 旁挂能力审查（20 findings，PR#207 已合）+ 第二批 auth+routes 安全隔离面审查（17 findings，PR#209 已合 0485e20）之后的第三批。范围 = 后台 worker 群。核心命题 = **claim/幂等/崩溃恢复的正确性**（outbox 元家族在主动侧 worker 的延伸）。**只审不修**——先出台账，再按 P0-P3 分批修（若有 High 优先级高于第一批 5 个 Medium + 第二批 2 个 Medium）。
>
> 设计：`docs/superpowers/specs/2026-07-14-worker-fleet-audit-design.md`
> 计划：`docs/superpowers/plans/2026-07-14-worker-fleet-audit.md`

## 审查范围（4 簇 ≈3131 行）

- **簇S 根因层**：`supervisor.rs`（185，全部）+ `tasks.rs` 的 reclaim_stale_running_tasks + claim/CAS 骨架 — 先审，结论作基准。
- **簇1 派生任务执行 worker**：`tasks.rs`（854）+ `import_worker.rs`（300）。
- **簇2 主动触达调度 worker**：`cold_contact_worker.rs`（581）+ `account_scheduler.rs`（390）+ `silence_signal_worker.rs`（317）。
- **簇3 信号/画像 worker**：`behavior_signals.rs`（504）。

## 边界排除（防与后续批次重叠，仅对照引用不重审）

outbox_dispatcher（上轮主链路已审 + F-01）/ strategic_planner（第一批已审）/ evolutionary_worker（留 evolution 专批）/ knowledge_digest·knowledge_task·catalog_rebuild·knowledge_feedback·ingest_worker（留 knowledge_wiki 专批）。

## 方法论

4 个只读审查 subagent（继承 Opus）；簇S 先派等回、结论喂基准给簇1-3；簇1-3 并行审 + 主控逐条亲验 file:line（复核失效链，驳回夸大）。两态 PLAUSIBLE(读码)/CONFIRMED(可构造竞态/重复副作用/回收遗漏触发序列)。元家族=声称的幂等/CAS/崩溃恢复不变量，实现层有竞态窗口/非原子写/回收遗漏/去重旁路。

## 一致性/幂等检查清单（逐 tick/claim/写路径）

①claim 是否 CAS（find_one_and_update/update_one 带状态前置）保证并发唯一占用、有无 find-then-update TOCTOU ②幂等：同触发重复执行是否重复副作用（重发/重复建 task/重复计数）③崩溃恢复：running/claimed 中途死能否回收、stale 边界（claimed_at 缺失/时间窗）安全 ④防死循环：recovery_count/attempt_count 有上限终态 ⑤时间窗竞态：冷却/节流/静默窗判定有无 TOCTOU ⑥非原子跨集合写：多集合写中间失败的不一致。

## 严重度校准（防夸大）

- **High**：单进程默认部署下**确定性可达的重复发送/丢任务/数据损坏/永久卡死**。
- **Medium**：需并发/崩溃时机叠加、或多副本部署才触发、或有自愈路径兜底的短暂不一致。
- **Low**：观测/边缘/防御纵深/无界增长无立即后果/就绪债。
- **⚠️ 单进程默认部署下不可达的多副本竞态 = 水平扩展就绪债，不夸大成 High**（生产 117 单机 systemd 单进程；同第二批"单租户不可达=就绪债"口径的部署维度版本）。

## Finding 字段模板

```
### [X-NN] 一句话标题
- 入口 worker: —
- 所属簇: S|1|2|3
- 类型: claim竞态|幂等|崩溃恢复|时间窗竞态|非原子写|无界增长|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 失效链: （谁在什么时机触发什么不一致后果；非失效类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 环节汇总

- **总 findings 数：18**（簇S 5 / 簇1 4 / 簇2 5 / 簇3 4）
- **严重度分布：1 High / 2 Medium / 15 Low**
  - **High（1）**：[1-01] initial_profile 任务成功后无终态写入（唯一 kind 不写 tasks 终态，单进程默认每条确定性命中：停 running→reclaim 反复重置 retry→重跑 3×LLM+3×画像覆写→recovery≥3 误判 failed）。
  - **Medium（2）**：[S-01] review_task_now 绕开 claim CAS 与串行 worker 真并发跑同一任务（双发被 outbox 幂等键兜住，仅两次 LLM 文本不同才击穿）；[S-02] review_task_now 置 running 不写 claimed_at → 崩溃/失败后落入 reclaim 双分支盲区、本进程内永不回收（重启才兜回）。
  - **Low（15）**：S-03/S-04/S-05 + 1-02/1-03/1-04 + 2-01~2-05 + 3-01/3-03/3-04（[3-02] 与 [2-05] 同一发现，去重后计簇2）。
- **一致性类元家族归纳**：本批主线元家族 = **「统一 claim/终态纪律的层间不对称」**——worker 主路径守原子 CAS + claimed_at 同写 + kind 终态契约，但旁路/新增 kind 各有漏项：①[1-01] 新增 kind（initial_profile）漏掉「handler 负责写 task 终态」的隐式契约（其余 3 kind 都写）；②[S-01/S-02] admin 侧门 review_task_now 绕开 claim CAS 与 claimed_at；③[S-04/1-03] 收尾/重试写用 plain `_id` 非 CAS（单进程串行安全，多副本不对称）。次要元家族 = **「幂等底座唯一索引缺列/缺失」**：[S-03/1-02] outcome_aggregation 唯一索引缺 workspace_id、[2-01] follow_up 池根本无唯一索引（find-then-insert TOCTOU）。第三类 = **命名名实不符**（[2-05/3-02] silence_signal_daily_cap 实为 per-tick）+ **无界增长**（[3-01] behavior_signals 无 TTL）。
- **交叉去重留痕**：[S-03]≡[1-02]（outcome_aggregation 唯一索引缺 workspace_id，同根因，簇S 与簇1 各从生命周期/资源域视角各报一次，修复一处即可）；[S-04]≡[1-03]（终态写非 CAS，同族）；[2-05]≡[3-02]（silence_signal_daily_cap 名实不符，同一发现，簇2 计数为准）。
- **正向 HOLDS（逐条主控亲验）**：worker claim 原子 CAS + claimed_at 同写、心跳续约自停、recovery≥3 终态封顶 CAS、outcome_aggregation dup-key 幂等、supervisor 11 worker「正常返回不重启」判定安全、cold_reactivation_idempotent 不变量、silence 幂等落 `(workspace_id, dedupe_key)` partial unique 索引、**behavior_signals「只写不进决策」红线（全仓零生产读取点，决策链零引用）**、import_worker 全 `find_one_and_update` 原子 CAS + preview-only 重跑无副作用。
- **后续 P0-P3 修复路线建议**：
  - **P0（High，优先级高于第一批 5 Medium + 第二批 2 Medium）**：[1-01] initial_profile handler 成功落库后写 CAS 终态 `{_id, status:"running"}→sent`（对齐 outcome_aggregation 范式），早退分支各写终态。独立走 brainstorming→SDD→PR。
  - **P1（Medium）**：[S-01]+[S-02] review_task_now 走 worker 同款 claim CAS（`{_id, workspace_id, status:{$in:[pending,retry]}}→running+claimed_at` + modified_count 检查）或直接复用 worker claim 语义。两条同源，一个 PR 收口。
  - **P2（Low 一致性纵深）**：[S-03/1-02] 唯一索引补 workspace_id（配 migration 清历史）、[2-01] follow_up 池加唯一索引或接受软去重现状（需产品裁定是否值得）。多租户/多副本就绪债，可批量收口或随多租户启用时再修。
  - **P3（Low 观测/命名/健壮性）**：[3-01] behavior_signals 补 TTL、[2-05/3-02] 重命名或补文档、[S-05] config 校验 timeout 下界、[1-04]/[2-02]/[2-03]/[2-04]/[3-03]/[3-04] 观测项。低优先，随手改。

---

## 簇S worker 生命周期 + claim/幂等根因层 findings

> 主控亲验结论：**worker 自身的 claim/回收骨架 HOLDS，无 High**。亲验 worker claim 是真原子 CAS（tasks.rs:197-212 `update_one({_id, status:&task.status}, {$set:{status:"running", claimed_at, ...}})` + `modified_count==0 → continue`），claimed_at 与 status 同一次原子写、心跳按 timeout/2 续约、recovery≥3 强制 failed 的 CAS 正确、outcome_aggregation 幂等已由 partial unique index+dup-key 忽略根治、supervisor「正常返回不重启」判定安全（11 个 supervised worker 逐一核过无一会因瞬时 error 从 loop 提前 return 静默死亡）。**关键前提亲验：run_task_worker 是严格串行 `loop{tick().await;sleep}`，tick 内 cursor 逐个 .await，main.rs 无裸 tokio::spawn——worker 内无并发 tick**，单进程唯一真并发来源是 axum HTTP handler。**两条 Medium 都在 worker 之外的 admin 侧门 review_task_now（routes/tasks.rs:168-198），它绕开统一 claim 纪律（不走 CAS、不写 claimed_at、无 status 前置）——新旧路径不对称元家族**。

### [S-01] review_task_now 绕开 claim CAS 与串行 worker 真并发跑同一任务（可致双跑/双发）
- 入口 worker: admin 侧门 POST /agent-tasks/:id/review-now（非 worker，但与 worker 争同一任务）
- 所属簇: S
- 类型: claim竞态 / 幂等
- 严重度: Medium（主控亲验裁定：单进程可达——axum handler 与串行 worker 同 tokio runtime 真并发；但需管理员手动点「立即复核」精确撞上 worker tick 处理同一任务的窗口，且双发被 outbox 幂等键 sha256(source_event_id:contact:content_hash) 兜住，**仅当两次 LLM 产出不同文本时才击穿**故 Medium 非 High；相同文本→同 key→IdempotentSkip）
- 现象/风险: review_task_now 的置 running update_one filter 是 `{_id, workspace_id}` 无 status 前置、不检查 modified_count，随后直接同步跑 handle_follow_up_task；可与 worker 已 claim 的同一任务并发进 gateway，重复 agent_run_logs/decision_reviews/memory 更新，极端重复发送客户消息。
- 失效链: 任务 pending+run_at<=now → worker CAS claim 成 running 开跑 gateway → 同时 admin 触发 review_task_now，find_one 读到任务后 update_one({_id,workspace_id})→running（恒成功）直接跑 handle_follow_up_task → 两链并发进 gateway，source_event_id 同为 task_id.to_hex()、content_hash=sha256(reply_text)；两次 LLM 文本相同→同 idempotency_key→第二次 IdempotentSkip 兜住；文本不同→两 key→两条都进 outbox→双发。
- 根因（亲验 file:line）: routes/tasks.rs:186-191 `update_one({_id, workspace_id}, {$set:{status:"running", updated_at}})` 无 status 前置无 claimed_at + :195 直接 handle_follow_up_task；对比 worker tasks.rs:197-212 带 `{_id, status:&task.status}` CAS 前置 + modified_count==0 continue；routes/mod.rs:432 路由已挂载可达。
- 复现设想: 单进程；pending follow-up 任务可执行；worker tick 处理它的同一秒调 POST /agent-tasks/:id/review-now；LLM 温度>0 两次文本不同→outbox 两条不同 key→客户收两条相似消息。
- 验证状态: PLAUSIBLE（并发窗口+无 CAS/无 claimed_at 亲验属实；端到端双发依赖 LLM 文本发散击穿幂等键，未构造时序证据）
- 修复建议: review_task_now 也走 claim CAS——`update_one({_id, workspace_id, status:{$in:[pending,retry]}}, {$set:{status:running, claimed_at:now}})` 检查 modified_count==1，为 0 返回「任务正被处理/已终态」；或直接复用 worker claim 语义（含 spawn_claim_heartbeat）。
- 状态: Open

### [S-02] review_task_now 置 running 却不写 claimed_at，handler 失败时落入 reclaim 双分支盲区本进程内永不回收
- 入口 worker: admin 侧门 review_task_now
- 所属簇: S
- 类型: 崩溃恢复 / 时间窗竞态
- 严重度: Medium（主控亲验裁定：单进程可达，但仅「卡在 running 直到下次进程重启」无双发/丢数据；重启后 process_started_at 更新使 reclaim 分支 B 命中兜回，自愈有下界）
- 现象/风险: review_task_now 置 running 且 updated_at=now 但不写 claimed_at；若随后 handle_*_task 返回 Err（如 follow-up contact not found），`?` 直接传播无回滚，任务永久停 running。
- 失效链: 该任务匹配 reclaim 哪条分支？分支 A `claimed_at:{$lt:stale_before}`——Mongo `$lt` 不匹配缺失字段→不匹配；分支 B `claimed_at 缺失 AND updated_at<process_started_at`——updated_at=now>process_started_at→不匹配；**本进程生命周期内两分支皆不命中=不可回收孤儿 running**，直到进程重启（新 process_started_at 晚于该 updated_at→分支 B 命中回收）。
- 根因（亲验 file:line）: routes/tasks.rs:186-196 置 running 无 claimed_at + handle_*_task 用 `?` 无 Err 回滚；reclaim 分支定义 tasks.rs:44-52（A 依赖 claimed_at 存在、B 依赖 claimed_at 缺失且 updated_at 早于启动）对「claimed_at 缺失+updated_at 新」组合无覆盖。
- 复现设想: 对 contact 已删/改名的任务调 review-now，handle_follow_up_task 返 NotFound，任务停 running，观察本进程内 reclaim 永不回收（重启才回收）。
- 验证状态: CONFIRMED（分支 A `$lt` 不匹配缺失字段是 Mongo 明确语义；分支 B `updated_at<process_started_at` 对本进程新写恒 false；两 filter+无回滚亲验）
- 修复建议: 二选一——(a) review_task_now 置 running 时同写 claimed_at:now 使卡死任务落入分支 A（timeout 后自动回收，首选，与 worker claim 对齐）；(b) reclaim 补第三分支覆盖「无 claimed_at 但 status=running 且 updated_at<stale_before」。
- 状态: Open

### [S-03] outcome_aggregation 幂等唯一索引缺 workspace_id（跨 workspace 同 account_id 互吞任务）
- 入口 worker: task_worker（ensure_today_outcome_aggregation_tasks）
- 所属簇: S
- 类型: 幂等 / 就绪债
- 严重度: Low（主控亲验裁定：单租户默认部署 account_id 全局唯一永不触发；多租户且两 workspace 复用同一 account_id 才成立=就绪债，同第二批 S-03/1-01 taxonomy 的多租户就绪债口径）
- 现象/风险: unique index keys=`{kind, account_id, content}`（content={horizon,date}）不含 workspace_id；两 workspace 持相同 account_id 时当日同 horizon 第二条 outcome_aggregation 插入命中 dup-key 被当已存在忽略→该 workspace 当日拿不到聚合任务、指标缺失。
- 失效链: ensure_today_outcome_aggregation_tasks 遍历所有 workspace 所有 account（accounts().find(doc!{}) tasks.rs:408）为每个 (account,horizon) 插入；account_id 跨 workspace 撞名→后插者被去重吞掉。
- 根因（亲验 file:line）: db/indexes.rs:114-126 keys 只有 kind/account_id/content；插入体 tasks.rs:416-438 带了 workspace_id 字段但索引未纳入。
- 复现设想: 多租户；workspace A、B 都有 account_id="acc1"；同一天→A 先插成功，B 插 acc1:7d:date 命中 dup-key 被忽略→B 无 7d 聚合。
- 验证状态: PLAUSIBLE（索引缺列亲验；实际撞名取决于是否允许 account_id 跨 workspace 复用，未取生产数据证实）
- 修复建议: unique index 改为 `{workspace_id, kind, account_id, content}`（配套 migration 先清历史重复，仿 m017_dedupe_outcome_aggregation）；或 content 带 workspace 前缀。单租户可暂缓。
- 状态: Open

### [S-04] tick 重试/终态写入用无状态前置的 `{_id}` filter（与 claim/reclaim 的 CAS 层间不对称）
- 入口 worker: task_worker（tick 收尾写）
- 所属簇: S
- 类型: 非原子写 / 就绪债
- 严重度: Low（主控亲验裁定：单进程串行 worker 下 worker 独占该任务安全；仅多副本部署成立=水平扩展就绪债）
- 现象/风险: tasks.rs:255-307 失败重试写 retry 与终态写 failed 都用 `update_one({_id: task_id}, ...)` 无 status:running 前置；claim/reclaim/heartbeat 都带状态前置 CAS 唯独收尾写不带——层间不对称。
- 失效链: 单副本无害（worker 串行无并发 tick、心跳保持 running 独占）；多副本下若 reclaim 已把它转 retry 且另一副本重 claim 成 running，本副本迟到的 failed 写（无前置）会把对方正在跑的 running 盖成 failed→状态错乱/潜在双跑。
- 根因（亲验 file:line）: tasks.rs:262 `doc!{"_id":task_id}`、tasks.rs:296 `doc!{"_id":task_id}`——对照 tasks.rs:198 claim 的 `{_id, status:&task.status}`。
- 复现设想: 仅多副本；需 heartbeat 失效+timeout+另一副本重 claim+本副本迟到收尾写叠加。
- 验证状态: PLAUSIBLE（filter 无前置亲验；单进程无并发 tick 亲验故单机不可达）
- 修复建议: 多副本化之前无需动；若走多副本，收尾 update_one 加 status:running（或 claim token/lease）前置，modified_count==0 时放弃写并记日志。
- 状态: Open

### [S-05] 心跳间隔在病态小 timeout 下非严格小于 timeout（timeout=5→interval=5）且该边界无测试
- 入口 worker: task_worker（spawn_claim_heartbeat）
- 所属簇: S
- 类型: 时间窗竞态 / 就绪债
- 严重度: Low（主控亲验裁定：默认 timeout=300 安全；仅当把 TASK_CLAIM_TIMEOUT_SECONDS 配成 ≤5 或 <worker 间隔这类病态值才成立=配置健壮性债）
- 现象/风险: claim_heartbeat_interval_seconds=clamp(timeout/2,5,60)；timeout=5→interval=5 心跳周期等于 timeout，续约与 reclaim 判定几乎同时，长任务可能在心跳落地前一刻被 reclaim 误判→回收成 retry 后另一轮重跑与仍在跑的原 claimer 并发（P1-9 本要防的正是这个）；若 timeout(5)<worker 间隔(默认30) reclaim 粒度远粗于 timeout 心跳无从挽救。
- 失效链: 病态配置下健壮性退化；默认值无影响。
- 根因（亲验 file:line）: tasks.rs:346-349 clamp 下界 5；单测 claim_heartbeat_strictly_below_timeout_in_normal_range(tasks.rs:844-853) 覆盖 [10,20,30,60,90,119] 独缺 5，而 claim_heartbeat_interval_clamps(:829) 恰断言 interval(5)==5（即 interval==timeout 非严格小于）。
- 复现设想: 设 TASK_CLAIM_TIMEOUT_SECONDS=5 跑 >5s 的 memory_consolidation 观察心跳与 reclaim 竞争。
- 验证状态: PLAUSIBLE（clamp 与测试覆盖亲验；实际误回收需病态配置+长任务）
- 修复建议: 文档/config 校验强制 task_claim_timeout_seconds >= 2*task_worker_interval_seconds 且 ≥10；或心跳下界 clamp 收紧为 min(timeout-1,...) 保证严格小于。仅健壮性加固非红线。
- 状态: Open

## 簇1 派生任务执行 worker findings

> 主控亲验结论：**worker claim/心跳/回收/import 终态全 CAS 骨架 HOLDS（7 判据逐条过）；本簇报出第三批唯一 High [1-01]**——`initial_profile` 是四个 task kind 里唯一"成功后不写 tasks 终态"的，元家族"新增 kind 与既有 kind 收尾契约不对称"实例。亲验四点全坐实：①tick Ok 分支只写 `follow_up_processed` 事件不写终态(tasks.rs:242-253) ②handle_initial_profile_task 全函数只碰 contacts、三早退全 `Ok(())`、主路径终于 apply_generated_profile_to_contact，**从不写 tasks 集合**(contacts.rs handle_initial_profile_task) ③对比另三 kind 均写终态：outcome_aggregation→sent(tasks.rs:232)、memory_consolidation→consolidate_contact_memory_inner 三处终态(memory.rs:1239 no_candidates→sent / :1599 occ_conflict→retry / :1724 consolidated→sent)、follow_up→gateway outbox_enqueued(gateway.rs:401/418)或 cancel/reschedule(:1099/1101) ④agent_tasks 对 follow_up/initial_profile 无唯一索引兜底(grep uniq 零命中，只 outcome_aggregation 有)。

### [1-01] `initial_profile` 任务成功后无终态写入 → 停 running 被 reclaim 反复重跑 + 画像覆写 + 误判 failed
- 入口频道: 后台 task worker（initial_profile kind）
- 所属簇: 1
- 类型: 崩溃恢复 / 状态机契约不对称（元家族命中）
- 严重度: **High**（主控亲验裁定：单进程默认部署下**每条 initial_profile 任务确定性命中**——非并发/非多副本条件。裁定张力已核：subagent 诚实标注"若严格按客户可见后果判据可降 Medium"（画像首跑已落库非丢失、无客户可见消息重复）；但按第三批校准口径 High=确定性可达的丢任务/卡死——此处功能生命周期确定性破坏+每条必中+3×LLM 浪费+**旧初始画像覆写窗口内累积的画像更新（数据正确性损害）**+recovery_count≥3 误判 failed（丢任务语义），维持 High）
- 现象/风险: initial_profile 任务成功生成并落库画像后，tasks 集合状态永远停在 `running`，被 reclaim_stale_running_tasks 判定 stale 反复重置 retry 重跑，直到 claim_recovery_count≥3 被强制误判 `failed`。
- 失效链:
  1. contacts 批量托管产出 initial_profile 任务，worker CAS claim 成 running+claimed_at(tasks.rs:192-209)。
  2. handle_initial_profile_task 成功：build_initial_operation_profile + apply_generated_profile_to_contact 写 contacts，返回 `Ok(())`——**从不写 tasks 终态**。
  3. tick Ok 分支(tasks.rs:242-253)只写 `follow_up_processed` 事件，也不写 task 终态 → 任务留在 running。
  4. 下一 tick reclaim(tasks.rs:34-53) 按 claimed_at<stale_before 判 stale → CAS 重置 retry。
  5. 反复重跑：每次 3×LLM(build profile) + apply 覆写 contacts 画像。若窗口内客户来消息推进了画像，重跑用新生成的**初始**画像覆盖累积更新。
  6. claim_recovery_count≥3 → 强制 `failed`(tasks.rs:67-115)，功能被误判失败终结。
- 根因（亲验 file:line）: tasks.rs:234-235 分派 initial_profile→handle_initial_profile_task(&task)；该 handler(contacts.rs) 全函数只 find_one contacts + build profile + apply_generated_profile_to_contact，三早退分支 `Ok(())`，无任何 tasks().update_one 终态写；tick Ok 分支 tasks.rs:242-253 不代写终态（契约=各 kind 自写）；对比 memory.rs:1239/1599/1724 + tasks.rs:232 + gateway.rs:401/1099 三 kind 都写。
- 复现设想: 单进程；托管一个 contact 产出 initial_profile 任务；观察成功落库画像后 task 停 running，task_claim_timeout 后被 reclaim 重置 retry 重跑，累计 3 次后 status=failed。
- 验证状态: CONFIRMED（四点断言逐条 Read 亲验；单进程确定性命中）
- 修复建议: handle_initial_profile_task 成功落库后写 CAS 终态 `update_one({_id, status:"running"}, {$set:{status:"sent", gateway_status:"profiled"}})`（对齐 outcome_aggregation 范式）；两个早退分支（contact 不存在 / 非 managed）也各写终态（sent/skipped），使成功与合法早退都脱离 running。这是唯一 High，优先级高于第一批 5 Medium + 第二批 2 Medium。
- 状态: Open

### [1-02] outcome_aggregation 去重唯一索引缺 workspace_id（与 S-03 同根因）
- 入口频道: —
- 所属簇: 1（与簇S S-03 同一索引缺陷，收尾交叉去重归并）
- 类型: 幂等 / 就绪债
- 严重度: Low（主控亲验裁定：单租户默认 account_id 全局唯一永不触发；多租户两 workspace 复用同一 account_id 时第二条聚合任务被 dup-key 误去重致指标缺失）
- 现象/风险: 见 S-03。unique index keys=`{kind, account_id, content}` 不含 workspace_id。
- 越权链: —
- 根因（亲验 file:line）: indexes.rs:117-121 keys 仅 kind/account_id/content；插入体 tasks.rs 带 workspace_id 但索引未纳入。
- 复现设想: 见 S-03。
- 验证状态: PLAUSIBLE（索引缺列亲验；撞名取决于是否允许 account_id 跨 workspace 复用）
- 修复建议: 改 unique index 为 `{workspace_id, kind, account_id, content}` + 清历史重复 migration。与 S-03 同条修复。
- 状态: Open

### [1-03] tasks.rs 终态/重试/取消写用无状态前置的 `{_id}` filter（与 import_worker 全 CAS 不对称）
- 入口频道: —
- 所属簇: 1（与簇S S-04 同族，均"收尾写无状态前置"）
- 类型: 非原子写 / 就绪债
- 严重度: Low（主控亲验裁定：单进程串行 worker 独占任务安全；仅多副本部署下迟到收尾写会盖掉另一副本重 claim 的 running）
- 现象/风险: tasks.rs:262/296 重试写 retry、终态写 failed 用 `update_one({_id: task_id})` 无 `status:running` 前置；对比 import_worker.rs 全 `{_id, status}` CAS。
- 越权链: —
- 根因（亲验 file:line）: tasks.rs:262 `doc!{"_id":task_id}`、:296 `doc!{"_id":task_id}`，对照 claim :198 的 `{_id, status:&task.status}`。
- 复现设想: 仅多副本；heartbeat 失效+timeout+另副本重 claim+本副本迟到收尾写叠加。
- 验证状态: PLAUSIBLE（filter 无前置亲验；单进程无并发 tick 故单机不可达）
- 修复建议: 多副本化前无需动；多副本时收尾 update_one 加 `status:running` 前置。同 S-04。
- 状态: Open

### [1-04] heartbeat 持续续约掩盖"假死未 crash"handler + 串行 loop 单任务卡死阻塞全部
- 入口频道: —
- 所属簇: 1
- 类型: 观测 / 就绪债
- 严重度: Low（主控亲验裁定：上游 IO 均有 timeout 故当前有界；heartbeat 续约设计本意防长任务误回收，副作用是无法区分"真长任务"与"卡死未 panic"，且串行 tick 单任务卡死会阻塞后续任务本 tick）
- 现象/风险: heartbeat 每 timeout/2 bump claimed_at，若 handler 逻辑假死（未 panic 也未返回，如无限等待）则任务永远续约不被 reclaim；且串行 loop 内单任务卡死阻塞本 tick 剩余任务。
- 越权链: —
- 根因（亲验 file:line）: tasks.rs:225-239 spawn_claim_heartbeat + 串行 while-cursor await。
- 复现设想: handler 内出现无 timeout 的等待（当前不存在——LLM/DB/MCP 均有 timeout）。
- 验证状态: PLAUSIBLE（当前上游 timeout 使其有界，属观测项非缺陷）
- 修复建议: 低优先。可加 handler 级总时限 / 单任务处理超时告警，仅观测增强。
- 状态: Open

## 簇2 主动触达调度 worker findings

> 主控亲验结论：**核心不变量 `cold_reactivation_idempotent` HOLDS**（`decide_cold_emit` cold_contact_worker.rs:394-424 确定性返回 AlreadyPending，重复 tick 恒被 `has_pending_follow_up` 挡回）；silence 幂等落在 `(workspace_id, dedupe_key)` partial unique 索引真底座（indexes.rs:213 亲验含 workspace_id）；三 worker 默认全关停（cold/silence enabled=false，account_scheduler 仅 cold 内调用）。**无 High、无 Medium**。5 条 Low 均属"元家族命中但有下游 backstop / 默认关停 / 多副本才放大"的就绪债。

### [2-01] cold worker 与 planner 共享 follow_up 池、去重 find-then-insert 且 agent_tasks 对 follow_up 无唯一索引（TOCTOU 双 emit 窗口）
- 入口频道: cold_contact_worker（默认关）
- 所属簇: 2
- 类型: 幂等 / 就绪债（元家族命中）
- 严重度: Low（主控亲验裁定：亲验 `agent_tasks` 对 follow_up **无任何唯一索引**（grep uniq/unique 零命中，唯一索引只覆盖 outcome_aggregation indexes.rs:117-121），去重是 `has_pending_follow_up` find-then-insert；两 worker 并发 tick 存 TOCTOU 双 emit 窗口。判 Low 而非 High：①需 cold+planner 同时启用（默认均 false）②单进程下多余任务被 task worker 串行 + min_reply_interval 门 gateway.rs:3227-3232 拦住不双发 ③升级为对客双发需多副本 task worker=水平扩展就绪债）
- 现象/风险: cold worker 与 strategic_planner 都往 follow_up 池插任务，靠内存 has_pending_follow_up 判定去重，无 DB 唯一约束兜底。
- 失效链: 两 worker 并发同一 tick 对同一 contact 各自 has_pending_follow_up 读到无 pending → 各插一条 follow_up → 双任务；下游 task worker 串行 + min_reply_interval 拦住不双发（单进程）；多副本 task worker 才对客双发。
- 根因（亲验 file:line）: agent_tasks 无 follow_up 唯一索引（indexes.rs 唯一索引仅 uniq_outcome_aggregation_kind_account_content:117-126）；cold_contact_worker.rs decide_cold_emit find-then-insert。
- 复现设想: 同时启用 cold+planner 多副本 task worker，观察同 contact 双 follow_up 落库并双发。
- 验证状态: PLAUSIBLE（无唯一索引亲验；双 emit 需双 worker 启用 + 多副本，单进程默认不可达）
- 修复建议: 给 follow_up 类任务加 `(workspace_id, account_id, contact_wxid, kind, dedupe)` partial unique 索引作幂等底座（同 outcome_aggregation 范式），把去重从内存判定升级为 DB 约束。属水平扩展就绪债。
- 状态: Open

### [2-02] cold emit 插 task 与写审计事件跨集合非原子（崩溃窗内 daily cap 漏计）
- 入口频道: cold_contact_worker（默认关）
- 所属簇: 2
- 类型: 非原子写 / 就绪债
- 严重度: Low（主控亲验裁定：per-contact 幂等仍由 has_pending_follow_up 兜住，仅 daily cap 计数在崩溃窗内漏计；默认关停）
- 现象/风险: cold emit 先插 follow_up task 再写 account_scheduler/cap 审计事件，两步非原子，中间崩溃则任务已插但 cap 未计。
- 越权链: —
- 根因（亲验 file:line）: cold_contact_worker.rs emit 路径 insert task 与 write_event 分两次 await，无事务边界。
- 复现设想: emit 插 task 后、写 cap 事件前进程崩溃，重启后该 contact 有 pending task 但 daily cap 少计一次。
- 验证状态: PLAUSIBLE
- 修复建议: cap 计数改为从 pending task 派生（幂等），或把 cap 写进 task 文档同一次 insert。属就绪债。
- 状态: Open

### [2-03] cold worker 丢弃 assign_account 返回值却仍写 account_scheduler_assignment 事件（capacity 计数污染）
- 入口频道: cold_contact_worker（默认关）
- 所属簇: 2
- 类型: 输入校验 / 就绪债
- 严重度: Low（主控亲验裁定：多账号才放大；单账号默认部署 account_id 恒 default 无影响；审计误导非数据损坏）
- 现象/风险: cold worker 调 assign_account 拿到 scheduler 选中的 account 后丢弃返回值，仍用旧 account_id 写 account_scheduler_assignment 事件，capacity 计数记的是被丢弃的选择。
- 越权链: —
- 根因（亲验 file:line）: cold_contact_worker.rs 调 assign_account 后未采用其返回 account_id，写事件用原 account_id。
- 复现设想: 多账号 workspace，观察 account_scheduler_assignment 事件的 account_id 与 scheduler 实际选中不符，capacity 计数偏。
- 验证状态: PLAUSIBLE
- 修复建议: 采用 assign_account 返回的 account_id 写审计事件，或移除该冗余事件。属多账号就绪债。
- 状态: Open

### [2-04] assign_account capacity 判定 count-then-pick 非原子（并发可越 capacity 软上限）
- 入口频道: account_scheduler（cold 内调用，默认关）
- 所属簇: 2
- 类型: 时间窗竞态 / 就绪债
- 严重度: Low（主控亲验裁定：capacity 是软上限；单进程 cold worker 串行调用不可达；多副本才放大）
- 现象/风险: assign_account 先 count 各 account 当前负载再 pick 最空的，count 与后续 assign 非原子，并发调用可让多个请求都选中同一"最空" account 越过 capacity。
- 越权链: —
- 根因（亲验 file:line）: account_scheduler.rs assign_account count-then-pick 无锁/无原子占位。
- 复现设想: 多副本并发 assign_account，观察某 account 负载越过 capacity 软上限。
- 验证状态: PLAUSIBLE（单进程串行不可达）
- 修复建议: 多副本化前无需动；若走多副本，用 find_one_and_update 原子占位 + capacity 前置过滤。属水平扩展就绪债。
- 状态: Open

### [2-05] silence_signal_daily_cap 命名为 daily 实为 per-tick 上限（命名语义误导）
- 入口频道: silence_signal_worker（默认关）
- 所属簇: 2
- 类型: 就绪债（命名）
- 严重度: Low（主控亲验裁定：CONFIRMED 纯命名语义误导，dedupe 幂等使其无数据后果）
- 现象/风险: `silence_signal_daily_cap` 字段名含 daily，实为每 tick 内存计数（silence_signal_worker.rs:74 emitted 每 tick 归零），即 per-tick cap 非日上限。
- 越权链: —
- 根因（亲验 file:line）: silence_signal_worker.rs:74 emitted 计数每 tick 重置，cap 判定基于该 per-tick 计数。
- 复现设想: 一天多 tick，每 tick 各发满 cap，实际日发送量 = cap × tick 数远超"daily"字面。
- 验证状态: CONFIRMED（per-tick 归零亲验）
- 修复建议: 重命名为 `silence_signal_per_tick_cap`，或改为跨 tick 累计的真日上限。属命名就绪债（与 [3-02] 同一发现，簇3 从信号侧记录）。
- 状态: Open

## 簇3 信号/画像 worker findings

> 主控亲验结论：**「behavior_signals 只写不进决策」红线 CONFIRMED HOLDS**。全仓 `.behavior_signals()` 生产侧只有 `insert_one` + 建索引 + m016 backfill，**零读取点**；决策链（agent/gateway/review/knowledge_router/cold_contact/tasks）对该集合零引用。唯一读取面 `behavior_signal_metrics` 仅经 admin-only REST 回三态计数做观测，不回原始信号、不入决策（同 bayesian/personality「永不驱动」铁律）。幂等底座 `uniq_behavior_signals_workspace_dedupe_key`（indexes.rs:213）含 workspace_id 亲验属实。4 条 finding 全 Low，无聚合结构故清单第 5 问「聚合口径竞态」本阶段不存在。

### [3-01] behavior_signals 无 TTL/retention → append-only 无界增长
- 入口频道: —
- 所属簇: 3
- 类型: 无界增长 / 就绪债
- 严重度: Low（主控亲验裁定：入站采集 webhooks.rs:583 无 flag 门控恒开，每 inbound ≥2 条信号；索引清单 indexes.rs:206-223 无 TTL——长跑就绪债，无立即后果，对照 import_jobs:161 有 TTL、knowledge 诊断日志 TTL=35d，behavior_signals 独缺）
- 现象/风险: behavior_signals 每 inbound 恒 append ≥2 条，无 TTL 长跑集合无界膨胀。
- 越权链: —
- 根因（亲验 file:line）: indexes.rs:206-223 behavior_signals 索引清单仅 `uniq_..._workspace_dedupe_key` + 查询索引，无 `expireAfterSeconds`；webhooks.rs:583 采集无 flag 门控。
- 复现设想: 长跑生产观察 behavior_signals 集合体积单调增长无回收。
- 验证状态: CONFIRMED（无 TTL 亲验；无界增长为长跑推演）
- 修复建议: 给 behavior_signals 加 TTL 索引（仿 knowledge 诊断日志 TTL=35d），或按 retention 策略定期归档。属长跑运维就绪债。
- 状态: Open

### [3-02] `silence_signal_daily_cap` 名实不符（实为 per-tick cap）
- 入口频道: —
- 所属簇: 3（与簇2 [2-05] 同一发现，交叉留痕）
- 类型: 就绪债（命名语义）
- 严重度: Low（主控亲验裁定：CONFIRMED 字段名含 daily 实为 per-tick——silence_signal_worker.rs:74 emitted 每 tick 归零；纯命名误导，dedupe 幂等使其无数据后果）
- 现象/风险: `silence_signal_daily_cap` 顾名"每日上限"，实际每 tick 内存计数归零，是"每 tick 上限"。
- 越权链: —
- 根因（亲验 file:line）: silence_signal_worker.rs:74 emitted 计数为 tick 局部变量每 tick 重置，非跨 tick 累积的日上限。
- 复现设想: 阅读语义即知；无数据后果（dedupe 幂等兜住）。
- 验证状态: CONFIRMED（per-tick 归零亲验）
- 修复建议: 改名为 `silence_signal_per_tick_cap`，或改成真正的跨 tick 日上限（按需）。纯命名订正。
- 状态: Open

### [3-03] msgId 全缺时 dedupe 回落接收毫秒 → 同毫秒双入站碰撞丢一条信号
- 入口频道: —
- 所属簇: 3
- 类型: 幂等 / 边缘
- 严重度: Low（主控亲验裁定：CONFIRMED 已文档化 A-03 边界；生产 GeWe 恒带 NewMsgId 不可达——msgId 全缺是极端退化）
- 现象/风险: dedupe_key 在 msgId 全缺时回落接收毫秒时间戳，同毫秒无 id 双入站会碰撞、被 `uniq_..._dedupe_key` 去重吞掉一条信号。
- 越权链: —
- 根因（亲验 file:line）: behavior_signals dedupe_key 生成在 msgId 缺失时用接收毫秒兜底，同毫秒无 id 双入站产同 key。
- 复现设想: 构造两条无 NewMsgId 且同毫秒到达的 inbound，观察第二条信号被去重丢弃。
- 验证状态: CONFIRMED（回落逻辑亲验；生产 GeWe 恒带 NewMsgId 使其不可达）
- 修复建议: msgId 全缺时 dedupe_key 追加随机后缀或序号，避免同毫秒碰撞。生产不可达，标边缘就绪债。
- 状态: Open

### [3-04] observed_at 用 server 接收时刻而非事件发生时刻
- 入口频道: —
- 所属簇: 3
- 类型: 就绪债（语义精度）
- 严重度: Low（主控亲验裁定：PLAUSIBLE 与 struct 声称的 event_time 语义存偏差，仅影响未来 ML 切片精度，当前无消费者——behavior_signals 零读取点故 observed_at 当前不被任何决策/分析消费）
- 现象/风险: 信号 observed_at 记 server 接收时刻，非消息真实事件发生时刻，两者在网络延迟/补发下有偏差。
- 越权链: —
- 根因（亲验 file:line）: behavior_signals 写入点用 server 接收 DateTime::now() 填 observed_at，未取消息自带事件时间。
- 复现设想: 延迟到达的消息，其 observed_at 晚于真实发生时刻。
- 验证状态: PLAUSIBLE（语义偏差亲验；当前无消费者故无实际后果）
- 修复建议: observed_at 优先取消息 payload 自带事件时间戳，缺失才回落接收时刻。未来 ML 切片前再处理即可。
- 状态: Open
