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

## 环节汇总（收尾时填）

- 总 findings 数：（待填）
- 严重度分布：H / M / L（待填）
- 一致性类元家族归纳：（待填）
- 后续 P0-P3 修复路线建议：（待填）

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

（主控亲验后填入）

## 簇2 主动触达调度 worker findings

（主控亲验后填入）

## 簇3 信号/画像 worker findings

（主控亲验后填入）
