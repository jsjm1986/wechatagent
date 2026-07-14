# knowledge_wiki 子系统深度审查（第四批）Implementation Plan

> **For agentic workers:** 审查工程，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 的 implementer→reviewer 双裁决；改由**主控编排**：TaskS 根因层先派并等回 → 结论作基准喂 Task1-3 并行派 → 主控逐条亲验 file:line → 填台账。步骤用 checkbox 跟踪。

**Goal:** 对 knowledge_wiki 子系统（写入强约束 + 反馈闭环 + 编辑历史 + 信号消解四职责）做纯代码/设计审查，产出经主控逐条亲验的 findings 台账，合并 docs PR。

**Architecture:** 簇S（写入根因层 page_merge 纯函数 + chunk_revisions 状态机）先审并等回，其结论（union/lock/70%/hash 纯函数正确性、apply_chunk_revision 七动作、双写非事务一致性、级联删除吞错、AI 强制 draft、domain schema 校验）作"写入正确性基准"喂给簇1-3；簇1-3 并行审信号面/摄取源头/反馈闭环是否守住幂等/一致性/隔离/红线 → 主控 Read/Grep 逐条复核失效链 + 驳回夸大 → 汇总进单一台账 → docs PR。只审不修。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象 Rust（src/knowledge_wiki/ 11 文件 5272 行）。

## Global Constraints

- 分支 `docs/knowledge-wiki-audit`，基于含 #210 的最新 origin/main（856ec9d 之后）。
- **只审不修**：本批绝不改任何 .rs。产出纯 docs（台账）。
- **subagent 只读**：不改任何文件；每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；凭猜测打回。
- **subagent 全部继承主会话 Opus**：省略 model 参数（`model:"opus"` 报 400，省略即继承）。用 general-purpose（非 Explore——Explore 读摘录漏 read window，不适合审计）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造竞态/重复副作用/数据不一致的触发序列）。
- **严重度校准防夸大**：High=单进程默认部署确定性可达的数据损坏/丢知识/红线破洞/永久卡死；Medium=需并发/崩溃时机叠加或多副本/多租户启用才触发或有自愈兜底；Low=观测/边缘/无界增长无立即后果/就绪债。**⚠️ 单进程默认部署下不可达的多副本竞态 = 水平扩展就绪债，不夸大成 High；单租户默认不可达的隔离缺陷 = 多租户就绪债**（生产 117 单机 systemd 单进程、默认单 workspace）。每条带主控裁定理由。
- **元家族聚焦**：设计声称的写入不变量/幂等/隔离红线（union 不丢 tag、锁定字段守门、AI 永不自动 verify、双写一致性、structural 只产 pending_review、模块隔离禁引用 gateway/outbox/mcp），实现层是否有旁路/竞态窗口/非原子写/去重遗漏/新旧不对称——上轮"统一 claim/终态纪律层间不对称"元家族在知识写入侧的延伸。
- **边界排除（防与既往批次重叠）**：knowledge_router 召回算法（mod.rs 明示本轮零改动）/ routes::knowledge HTTP 端点层（Batch 2 簇5 已审）——仅在写入契约对照时引用，不重审。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）；本会话在 worktree fix-full-system-remediation。

---

### Task 0: 建台账骨架

**Files:** Create `docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md`

- [ ] **Step 1: 写台账头部 + 字段模板**

头含审查范围（4 簇 + 文件清单 5272 行）、方法论、一致性/幂等/隔离检查清单 7 问、严重度校准口径（含单进程/单租户不可达=就绪债原则）、元家族说明、边界排除清单。字段模板逐字：

```
### [X-NN] 一句话标题
- 入口: —（函数/worker）
- 所属簇: S|1|2|3
- 类型: 写入不变量旁路|幂等|一致性(非原子写)|隔离红线|时间窗竞态|无界增长|就绪债
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
git add docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md
git commit -m "docs(audit): knowledge_wiki子系统审查台账骨架(第四批)"
```

---

### Task S: 写入根因层（page_merge 纯函数 + chunk_revisions 状态机）审查 —— 先派并等回

**审查对象:** `src/knowledge_wiki/page_merge.rs`（全 482）+ `src/knowledge_wiki/chunk_revisions.rs`（全 513）。

**Interfaces:**
- Produces: 簇S findings（S-NN）+ **写入正确性基准**（喂给 Task1-3）：union_array_fields 三方入参解耦是否真防 tag 丢失（KB-09）、锁定字段守门（apply_field_patch 硬拒 + enforce_locked_fields 末次防线双层）、70% body 阈值边界、compute_chunk_hash 稳定性（volatile 剔除 + 字段序无关）、apply_chunk_revision 七动作双写顺序（先 revisions 后 chunks 非事务的中间失败语义）、cleanup_dangling_refs 递归 apply + best-effort 吞错、AI source 强制 draft+needs_review、domain schema 校验分支、workspace_id 守门。"写入该怎么算安全"的正确姿势。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus），等它回**

dispatch 指令要点：
- 审 page_merge.rs 全（union_array_fields KB-09 三方入参、effective_locked_fields DEFAULT∪运营锁去重、enforce_locked_fields 末次覆盖、is_body_truncated 边界、apply_field_patch 硬拒锁定字段、compute_chunk_hash canonical JSON + volatile 剔除）+ chunk_revisions.rs 全（apply_chunk_revision 七动作分发、双写 revisions→chunks 顺序与中间失败、unchanged 短路、AI source draft 强制、archive/restore status 覆盖、provenance 覆盖、domain schema enforce、normalize_ref_key 防 substring 误伤、cleanup_dangling_refs 递归 + 吞错）。
- **重心**：①**写入不变量旁路**：三层保护（锁定/union/70%）有无绕过路径？union 三方入参真的防 tag 丢失还是有回退？②**非原子双写一致性**：先 revisions 后 chunks，chunks replace 失败时 revisions 已落 → last_revision != current_state 的后果、catalog_rebuild enqueue best-effort 失败后 catalog 永久陈旧？③**AI 永不自动 verify 红线**：source=Ai 强制 draft 的分支有无被 archive/restore/domain-schema 后续 insert 覆盖翻转？principal_authorized 分支绕过 draft 是否正确？④**级联删除**：cleanup_dangling_refs 吞错导致悬空引用残留、normalize_ref_key 误判、递归 apply_chunk_revision 自身失败静默。⑤**workspace 隔离**：find_one/replace_one 都带 workspace_id 守门是否完整。
- **产出基准**：明确回答"knowledge_wiki 写入该怎么算安全（不变量/幂等/一致性/隔离/红线）"，供 Task1-3 当审查标尺。
- 硬约束（先读懂+file:line+只读+两态+严重度带理由+单进程/单租户不可达=就绪债不夸 High）。报告写 `.superpowers/audit4/cluster-S-report.md`。

- [ ] **Step 2: 主控逐条亲验 + 提炼基准**

对 subagent 每个 finding Read/Grep 复核 file:line + 失效链；提炼"安全写入姿势"作为 Task1-3 dispatch 要点。

- [ ] **Step 3: 填台账（簇S）+ Commit**

```bash
git add docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md
git commit -m "docs(audit): 簇S 写入根因层(page_merge+chunk_revisions) findings(主控亲验)"
```

---

### Task 1-3: 资源域簇审查（TaskS 完成后并行派）

> 三簇结构相同，仅审查对象与重心不同。每簇：派只读 subagent（继承 Opus，带 TaskS 写入正确性基准 + 一致性/幂等/隔离检查清单 7 问）→ 主控逐条亲验 → 填台账 → commit。报告写 `.superpowers/audit4/cluster-{1,2,3}-report.md`。

**一致性/幂等/隔离检查清单 7 问（喂给每簇 subagent，逐 tick/写路径核）：**
1. 幂等：同一触发重复执行是否产生重复副作用（重复建 signal / 重复摄取 chunk / 重复计数 / 重复提案）？find-then-insert 有无 TOCTOU？
2. 一致性（非原子跨集合/跨字段写）：多写是否有中间失败导致的不一致（同上轮 A-03 元家族在知识侧）？
3. 崩溃恢复：有 job 队列的（catalog_rebuild）claim 中途死能否回收？无队列的 worker 一轮失败下一轮能否自愈？
4. 时间窗竞态：schedule_minutes / valid_to / 30d 窗 / not-due 判定与写副作用之间的窗口。
5. 隔离红线：本模块禁引用 gateway/outbox/mcp（mod.rs 明示）——有无越界引用？workspace 隔离是否每个查询都带 workspace_id？
6. 红线（AI 永不自动 verify）：摄取/信号/提案入口是否都写 draft+needs_review，structural_proposals 是否恒 pending_review 无 apply 字段？
7. 无界增长 / best-effort 吞错：signal / proposal / stats 写侧是否有界？编排层 `if let Err → warn` 是否掩盖了应上报的真错？

**每簇 subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带理由（**单进程默认不可达的多副本竞态=水平扩展就绪债、单租户默认不可达的隔离缺陷=多租户就绪债，均不夸成 High**）；元家族=找声称的写入不变量/幂等/隔离红线在实现层的旁路/竞态窗口/非原子写/去重遗漏。

- [ ] **Task 1 信号生成消解**：`src/knowledge_wiki/gap_signals.rs`（2170，最大）+ `src/knowledge_wiki/structural_proposals.rs`（208）。重心=8 类 signal kind 生成正确性、structural lint 纯规则去重（重复 lint 是否重复建同一 signal，find-then-insert 幂等）、两阶段 sweep（stage1 规则消解 / stage2 LLM 桩）、dynamic_confidence 重算 read-modify-write 竞态、悬空 anchor 软降格罚分、contradiction/missing_chunk/suggestion 三新 kind 纯规则路径、structural_proposals 只产 pending_review 红线（序列化层无 apply 字段、就绪债 KB-06 无消费方）。派→亲验→填台账（1-NN）→commit `docs(audit): 簇1 信号生成消解 findings`。
- [ ] **Task 2 摄取源头**：`src/knowledge_wiki/ingest_worker.rs`（489）+ `src/knowledge_wiki/block_parser.rs`（476）+ `src/knowledge_wiki/catalog_rebuild.rs`（357）。重心=RSS/HTML 摄取幂等（etag / If-None-Match、重复摄取同 URL 是否重复建 chunk）、failure_streak 状态机（3→failing / 168h→disabled）、not-due 不刷 last_fetched_at 的正确性（否则节流基准无限前推）、block_parser 解析健壮性（畸形 HTML/RSS）、catalog_rebuild job claim CAS（claim_one_job status queued→processing、崩溃回收、attempts 上限）、AI 永不自动 verify 红线在摄取入口 HOLDS（ingest_chunked_text 默认 draft）。派→亲验→填台账（2-NN）→commit `docs(audit): 簇2 摄取源头 findings`。
- [ ] **Task 3 反馈闭环**：`src/knowledge_wiki/feedback_worker.rs`（189）+ `src/knowledge_wiki/lessons_learned.rs`（173）+ `src/knowledge_wiki/reviewer_stats.rs`（186）。重心=编排层 best-effort 吞错是否掩盖真错（run_one_round 逐步 `if let Err → warn` 继续）、滑窗聚合 upsert 一致性（$set 瞬时值 vs 累加、stat_id 唯一性）、list_workspaces distinct 假设 <100 全量拉内存、跨 workspace 隔离（逐 ws 循环内查询是否都带 ws）、14d/30d 窗边界、fallback default_workspace_id 语义。派→亲验→填台账（3-NN）→commit `docs(audit): 簇3 反馈闭环 findings`。

---

### Task E: 台账收尾 + push + PR

- [ ] **Step 1: 汇总头** —— 总 findings 数、严重度分布（H/M/L）、写入不变量/隔离类元家族归纳、后续 P0-P3 修复路线（若有 High 优先级高于第一批 5 Med + 第二批 2 Med + 第三批已列 P0[1-01]）。
- [ ] **Step 2: 交叉去重** —— 扫全台账去重跨簇重复（如 page_merge 纯函数缺陷被多簇各报一次、best-effort 吞错模式跨 worker 重复），归并留痕。
- [ ] **Step 3: Commit + push（显式 refspec）+ PR**

```bash
git add docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md
git commit -m "docs(audit): knowledge_wiki子系统审查台账收尾(严重度分布+修复路线)"
LOCAL=$(git rev-parse HEAD)
git push origin HEAD:refs/heads/docs/knowledge-wiki-audit -u
git ls-remote origin refs/heads/docs/knowledge-wiki-audit   # 亲验 tip==LOCAL
gh pr create --head docs/knowledge-wiki-audit --base main --title "..." --body "..."
gh pr view docs/knowledge-wiki-audit --json number,headRefName,baseRefName,headRefOid  # 核身份
```

- [ ] **Step 4:** docs-only PR 走 paths-ignore，后端 job 大概率 skip（同 PR#207/#209/#210）。核 CI 无意外 FAILURE 后 squash merge（不带 --delete-branch，worktree 铁律）。写 memory。

---

## Self-Review

**1. Spec coverage:** 4 簇 → TaskS + Task1-3 一一对应；只审不修+台账格式 → Task0 + 各 Step 填台账；主控亲验 → 各 Step 2；一致性/幂等/隔离清单 7 问+元家族+严重度校准 → Global Constraints + 各簇要点；失效链字段 → Task0 模板；后续修复路径 → TaskE Step1；边界排除（knowledge_router/routes HTTP）→ Global Constraints。✓ 无遗漏。

**2. Placeholder scan:** 无 TBD/TODO。台账字段模板、检查清单 7 问、各簇审查对象+重心、commit message 均具体。TaskE 的 PR title/body `"..."` 执行时按实际 findings 填（届时才知内容），非计划占位。✓

**3. Type consistency:** finding 编号 S-NN/1-NN/2-NN/3-NN 全计划一致；台账路径 `docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md` 与报告文件 `.superpowers/audit4/cluster-{S,1,2,3}-report.md` 命名一致。✓

## 备注

- TaskS **必须先派并等回**（其基准喂后续），Task1-3 在 TaskS 完成后可一次性并行派 3 个 subagent。主控亲验各簇串行做保质量。
- 审查 subagent 用 general-purpose（只读指令约束，非 Explore），继承 Opus。
- 报告文件 `.superpowers/audit4/*.md` 是 git-ignored scratch，不进 commit。
- 本批零代码改动、零 CI 风险（纯 docs）。
