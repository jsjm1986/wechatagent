# 后台 worker 群深度审查（第三批）Implementation Plan

> **For agentic workers:** 审查工程，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 的 implementer→reviewer 双裁决；改由**主控编排**：TaskS 根因层先派并等回 → 结论作基准喂 Task1-3 并行派 → 主控逐条亲验 file:line → 填台账。步骤用 checkbox 跟踪。

**Goal:** 对后台 worker 群（claim/幂等/崩溃恢复/调度去重一致性）做纯代码/设计审查，产出经主控逐条亲验的 findings 台账，合并 docs PR。

**Architecture:** 簇S（worker 生命周期 + claim/幂等根因层）先审并等回，其结论（supervisor 重启语义是否安全、claim CAS 原子性、崩溃回收边界）作基准喂给簇1-3；簇1-3 并行审各 worker 的 tick/claim/写路径是否守住幂等/崩溃恢复 → 主控 Read/Grep 逐条复核失效链 + 驳回夸大 → 汇总进单一台账 → docs PR。只审不修。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象 Rust（src/supervisor.rs + src/tasks.rs + src/import_worker.rs + src/cold_contact_worker.rs + src/account_scheduler.rs + src/silence_signal_worker.rs + src/behavior_signals.rs）。

## Global Constraints

- 分支 `docs/worker-fleet-audit`，基于含 #209 的最新 origin/main（0485e20）。
- **只审不修**：本批绝不改任何 .rs。产出纯 docs（台账）。
- **subagent 只读**：不改任何文件；每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；凭猜测打回。
- **subagent 全部继承主会话 Opus**：省略 model 参数（`model:"opus"` 报 400，省略即继承）。用 general-purpose（非 Explore）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造竞态/重复副作用/回收遗漏的触发序列）。
- **严重度校准防夸大**：High=单进程默认部署确定性可达的重复发送/丢任务/数据损坏/永久卡死；Medium=需并发/崩溃时机叠加或多副本部署才触发或有自愈兜底；Low=观测/边缘/无界增长无立即后果/就绪债。**⚠️ 单进程默认部署下不可达的多副本竞态 = 水平扩展就绪债，不夸大成 High**（生产 117 单机 systemd 单进程）。每条带主控裁定理由。
- **元家族聚焦**：声称的幂等/CAS/崩溃恢复不变量，实现层是否有竞态窗口/非原子写/回收遗漏/去重旁路——上轮 outbox 元家族在主动侧 worker 的延伸。
- **边界排除（防与后续批次重叠）**：outbox_dispatcher（上轮主链路已审）/strategic_planner（第一批已审）/evolutionary_worker（留 evolution 专批）/knowledge_* worker（留 knowledge_wiki 专批）——仅在 claim/幂等模式对照时引用，不重审。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）。

---

### Task 0: 建台账骨架

**Files:** Create `docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md`

- [ ] **Step 1: 写台账头部 + 字段模板**

头含审查范围（4 簇 + 文件清单 ≈3131 行）、方法论、一致性/幂等检查清单 6 问、严重度校准口径（含单进程不可达=水平扩展就绪债原则）、元家族说明、边界排除清单。字段模板逐字：

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

台账含"环节汇总（收尾时填）"占位段（总数/严重度分布/元家族归纳/P0-P3 路线）+ 4 簇 findings 段（各留"（主控亲验后填入）"）。

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md
git commit -m "docs(audit): 后台worker群审查台账骨架(第三批)"
```

---

### Task S: 根因层（worker 生命周期 + claim/幂等共享层）审查 —— 先派并等回

**审查对象:** `src/supervisor.rs`（全部 185）+ `src/tasks.rs` 的 reclaim_stale_running_tasks（:34 起）+ claim/CAS 骨架（tick 中的 claim 路径）。

**Interfaces:**
- Produces: 簇S findings（S-NN）+ **审查基准**（喂给 Task1-3）：supervisor 重启语义是否安全（catch_unwind + factory 重建 state + 正常返回不重启的判定）、claim CAS 的原子性正确姿势、崩溃回收边界（claimed_at 缺失回落 APP_STARTED_AT、recovery_count≥3 终态）、"worker 该怎么写 claim/幂等/回收才算安全"的正确姿势。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus），等它回**

dispatch 指令要点：
- 审 src/supervisor.rs 全部（spawn_supervised 的 catch_unwind/backoff/重启语义、哪些 worker 有意不接入 supervisor、factory 重建 state 的正确性、正常返回不重启是否漏掉需重启的场景）+ src/tasks.rs 的 reclaim_stale_running_tasks（stale 判定、claimed_at 缺失处理、recovery_count≥3 强制 failed 的 CAS）+ tasks.rs tick 主循环的 claim 路径（是否 CAS 占用、并发两 tick 是否可能双claim）。
- **重心**：①supervisor：panic 兜底是否有漏（哪些 worker 未接入而 panic 即死是否有意为之、backoff 计数器归零逻辑正确性）②claim CAS：find_one_and_update/update_one 带状态前置是否保证并发唯一占用、有无 find-then-update 的 TOCTOU 窗口 ③崩溃回收：running 卡死回收的边界是否安全（会不会误回收正在跑的、会不会漏回收永久卡死的）④防死循环：recovery_count 上限终态是否正确。
- **产出基准**：明确回答"worker 该怎么写 claim/幂等/崩溃回收才算安全"，供 Task1-3 当审查标尺。
- 硬约束（先读懂+file:line+只读+两态+严重度带理由+单进程不可达=就绪债不夸 High）。报告写 `.superpowers/audit3/cluster-S-report.md`。

- [ ] **Step 2: 主控逐条亲验 + 提炼基准**

对 subagent 每个 finding Read/Grep 复核 file:line + 失效链；提炼"安全 claim/幂等/崩溃回收姿势"作为 Task1-3 dispatch 要点。

- [ ] **Step 3: 填台账（簇S）+ Commit**

```bash
git add docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md
git commit -m "docs(audit): 簇S worker生命周期+claim/幂等根因层 findings(主控亲验)"
```

---

### Task 1-3: 资源域簇审查（TaskS 完成后并行派）

> 三簇结构相同，仅审查对象与重心不同。每簇：派只读 subagent（继承 Opus，带 TaskS 基准 + 一致性/幂等检查清单 6 问）→ 主控逐条亲验 → 填台账 → commit。报告写 `.superpowers/audit3/cluster-{1,2,3}-report.md`。

**一致性/幂等检查清单 6 问（喂给每簇 subagent，逐 tick/claim/写路径核）：**
1. claim 是否用 CAS（find_one_and_update / update_one 带状态前置）保证并发唯一占用？有无 find-then-update 的 TOCTOU 窗口？
2. 幂等：同一触发重复执行是否产生重复副作用（重复发送 / 重复建 task / 重复计数）？
3. 崩溃恢复：running/claimed 中途进程死，重启后能否回收？stale 回收边界（claimed_at 缺失、时间窗）是否安全？
4. 防死循环：recovery_count / attempt_count 是否有上限终态？
5. 时间窗竞态：冷却 / 节流 / 静默窗的判定是否有 TOCTOU（读判定与写副作用之间的窗口）？
6. 非原子跨集合写：多集合写是否有中间失败导致的不一致（上轮 A-03 元家族）？

**每簇 subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带理由（**单进程默认不可达的多副本竞态=水平扩展就绪债不夸成 High**）；元家族=找声称的幂等/CAS/崩溃恢复不变量在实现层的竞态窗口/非原子写/回收遗漏/去重旁路。

- [ ] **Task 1 派生任务执行 worker**：`src/tasks.rs`（854，follow-up 任务 tick/claim/执行/重试/stale 回收全链）+ `src/import_worker.rs`（300，import job 队列消费）。重心=claim CAS 竞态、幂等（同任务不重复执行）、崩溃恢复（running 卡死回收）、recovery_count 死循环防护、attempt_count 与状态机一致性。派→亲验→填台账（1-NN）→commit `docs(audit): 簇1 派生任务执行worker findings`。
- [ ] **Task 2 主动触达调度 worker**：`src/cold_contact_worker.rs`（581）+ `src/account_scheduler.rs`（390）+ `src/silence_signal_worker.rs`（317）。重心=调度去重（cold_reactivation_idempotent 不变量，cold_contact_worker.rs:375 声称）、触达节流/冷却窗竞态、时间窗判定边界、account 分配一致性、静默信号触发幂等。派→亲验→填台账（2-NN）→commit `docs(audit): 簇2 主动触达调度worker findings`。
- [ ] **Task 3 信号/画像 worker**：`src/behavior_signals.rs`（504）。重心=信号写入一致性、并发累积正确性（read-modify-write 竞态）、无界增长（write 侧是否有界，同 memory feedback_cautious_profiling 记 memory_summary 无界 append）、信号是否误进决策（tag_trust_reform 铁律"只写不进决策"）。派→亲验→填台账（3-NN）→commit `docs(audit): 簇3 信号画像worker findings`。

---

### Task E: 台账收尾 + push + PR

- [ ] **Step 1: 汇总头** —— 总 findings 数、严重度分布（H/M/L）、一致性类元家族归纳、后续 P0-P3 修复路线（若有 High 优先级高于第一批 5 个 Medium + 第二批 2 个 Medium）。
- [ ] **Step 2: 交叉去重** —— 扫全台账去重跨簇重复（如 supervisor/claim 共享缺陷被多簇各报一次，归并到簇S 留痕）。
- [ ] **Step 3: Commit + push（显式 refspec）+ PR**

```bash
git add docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md
git commit -m "docs(audit): 后台worker群审查台账收尾(严重度分布+修复路线)"
LOCAL=$(git rev-parse HEAD)
git push origin HEAD:refs/heads/docs/worker-fleet-audit -u
git ls-remote origin refs/heads/docs/worker-fleet-audit   # 亲验 tip==LOCAL
gh pr create --head docs/worker-fleet-audit --base main --title "..." --body "..."
gh pr view docs/worker-fleet-audit --json number,headRefName,baseRefName,headRefOid  # 核身份
```

- [ ] **Step 4:** docs-only PR 走 paths-ignore，后端 job 大概率 skip（同 PR#207/#209）。核 CI 无意外 FAILURE 后 squash merge（不带 --delete-branch，worktree 铁律）。

---

## Self-Review

**1. Spec coverage:** 4 簇 → TaskS + Task1-3 一一对应；只审不修+台账格式 → Task0 + 各 Step 填台账；主控亲验 → 各 Step 2；一致性/幂等清单 6 问+元家族+严重度校准 → Global Constraints + 各簇要点；失效链字段 → Task0 模板；后续修复路径 → TaskE Step1；边界排除 → Global Constraints。✓ 无遗漏。

**2. Placeholder scan:** 无 TBD/TODO。台账字段模板、检查清单 6 问、各簇审查对象+重心、commit message 均具体。TaskE 的 PR title/body `"..."` 执行时按实际 findings 填（届时才知内容），非计划占位。✓

**3. Type consistency:** finding 编号 S-NN/1-NN/2-NN/3-NN 全计划一致；台账路径 `docs/superpowers/specs/2026-07-14-worker-fleet-audit-findings.md` 与报告文件 `.superpowers/audit3/cluster-{S,1,2,3}-report.md` 命名一致。✓

## 备注

- TaskS **必须先派并等回**（其基准喂后续），Task1-3 在 TaskS 完成后可一次性并行派 3 个 subagent。主控亲验各簇串行做保质量。
- 审查 subagent 用 general-purpose（只读指令约束，非 Explore——Explore 读摘录漏 read window 外内容不适合审计），继承 Opus。
- 报告文件 `.superpowers/audit3/*.md` 是 git-ignored scratch，不进 commit。
- 本批零代码改动、零 CI 风险（纯 docs）。
