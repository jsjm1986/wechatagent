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

（主控亲验后填入）

## 簇1 派生任务执行 worker findings

（主控亲验后填入）

## 簇2 主动触达调度 worker findings

（主控亲验后填入）

## 簇3 信号/画像 worker findings

（主控亲验后填入）
