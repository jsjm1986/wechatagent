# knowledge_wiki 子系统深度审查（第四批）Design

> 只审不修 · 纯 docs 台账 · 沿用前三批「根因层先派→资源域并行 + 主控逐条亲验」范式。

## 背景与定位

第四批圈定 **`src/knowledge_wiki/` 子系统**（5272 行 11 文件），接续：

- [第一批 agent 旁挂能力审](2026-07-14-agent-capabilities-audit-findings.md)（20 findings/1H）—— PR#207
- [第二批 auth+routes 安全隔离面审](2026-07-14-auth-routes-security-audit-findings.md)（17 findings）—— PR#209
- [第三批后台 worker 群审](2026-07-14-worker-fleet-audit-findings.md)（18 findings/1H）—— PR#210

前三批只审过 `agent::knowledge_router`（召回算法）与 `routes::knowledge`（HTTP 端点层，第二批簇5），**子系统内部写入/信号/摄取/反馈逻辑从未深审**——这是覆盖缺口最大的一块。

## 子系统职责（mod.rs 亲验）

`src/knowledge_wiki/mod.rs:3-18` 声明本模块只负责**写入路径强约束 + 反馈闭环 + 编辑历史**四件事：质量（schema 校验 + 锁定字段 + provenance）/ 可检索（wiki 分层 + catalog）/ 可修改（字段级 patch + 不可变编辑历史 + 删除级联）/ 可优化（usage/hit/blocked 回写 + 两阶段 sweep）。

**隔离红线**（mod.rs:10-11）：本模块禁止引用 `crate::agent::gateway/outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。审查须核这条红线是否 HOLDS。

**边界排除（防与前后批次重叠）**：
- `agent::knowledge_router` 召回算法（catalog→list_chunks→open_slice）本轮**零改动**（mod.rs:9 明示）——仅对照不重审。
- `routes::knowledge` HTTP 端点层已在第二批簇5 审——仅在权限/workspace 守门对照时引用，不重审。
- `evolution/` / `db/migrations+indexes` 留后续专批。

## 审查范围（4 簇 / 5272 行）

| 簇 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **S 写入根因层**（先派等回） | `page_merge.rs`(482) + `chunk_revisions.rs`(513) | 995 | 纯函数正确性（union/lock/70%/hash）；`apply_chunk_revision` 七动作状态机；**双写非事务**（先 revisions 后 chunks）中间失败一致性；级联删除 `cleanup_dangling_refs` best-effort 吞错；AI source 强制 draft；domain schema 校验 |
| **1 信号生成消解** | `gap_signals.rs`(2170) + `structural_proposals.rs`(208) | 2378 | 8 类 signal kind 生成正确性；两阶段 sweep（stage2 LLM 预留桩）；structural lint 纯规则查询；信号去重/幂等（重复 lint 是否重复建 signal）；`dynamic_confidence` 重算 read-modify-write；structural_proposals 只产 pending_review 红线 |
| **2 摄取源头** | `ingest_worker.rs`(489) + `block_parser.rs`(476) + `catalog_rebuild.rs`(357) | 1322 | RSS/HTML 摄取幂等（If-None-Match/etag）；failure_streak 状态机（3→failing / 168h→disabled）；not-due 不刷 last_fetched_at 节流；block_parser 解析健壮性；catalog_rebuild job 消费 claim CAS；「AI 永不自动 verify」红线在摄取入口 HOLDS |
| **3 反馈闭环** | `feedback_worker.rs`(189) + `lessons_learned.rs`(173) + `reviewer_stats.rs`(186) | 548 | 编排层 best-effort 吞错是否掩盖真错；滑窗聚合 upsert 一致性（$set 瞬时值 vs 累加）；list_workspaces distinct 假设 <100；跨 workspace 隔离 |

## 方法论（沿用前三批）

1. **簇S 根因层先派并等回** —— 提炼「写入正确性基准」（三层保护是否闭合、双写非原子窗口边界、hash 稳定性、锁定字段末次防线）喂 Task1-3 当审查标尺。
2. **Task1-3 TaskS 完成后并行派** —— 各簇只读 subagent（general-purpose，继承 Opus 省略 model），带 TaskS 基准 + 检查清单。
3. **主控逐条亲验** —— 每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大严重度。
4. **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列）。

### 检查清单（喂每簇 subagent）

1. **写入一致性**：双写（revisions + chunks）/ 多集合写中间失败是否留不一致？best-effort 吞错是否掩盖真损坏？
2. **幂等**：同一触发重复执行是否重复副作用（重复 signal / 重复 chunk / 重复计数 / 重复 catalog job）？
3. **纯函数契约**：union/lock/70%/hash 的不变量（幂等/包含/保序/稳定）是否有边界破例？
4. **红线 HOLDS**：AI source 强制 draft+needs_review、structural 只产 pending_review、模块隔离（禁引 gateway/outbox/mcp）——实现层是否有旁路？
5. **崩溃恢复/时间窗**：catalog_rebuild claim CAS、ingest failure_streak、sweep 时效判定是否有 TOCTOU？
6. **无界增长/多租户**：signal/proposal 是否无界堆积无 TTL？workspace 隔离是否每处守门？

## 严重度校准（防夸大，沿用前三批口径）

- **High**：推荐配置（单租户/单进程默认部署）下**确定性可达**的：知识损坏（既有内容丢失/覆写）、写入不一致（双写撕裂永久留脏）、红线绕过（AI 自动 verify/自动 apply structural）、永久卡死。
- **Medium**：需多条件叠加/多租户/多副本才触发，或仅信息泄漏，或有自愈兜底。
- **Low**：观测/边缘/输入校验无后果/就绪债（如 structural_proposals 未接线 KB-06 已知）。
- **单租户默认不可达的隔离缺陷=多租户就绪债；单进程默认不可达的并发竞态=水平扩展就绪债——都不夸成 High**（[[project-multitenant-isolation-debt]] 口径 + 第三批部署拓扑维度）。

## 元家族聚焦

本批预期主线元家族=**「写入路径强约束的实现层旁路/非原子窗口」**：page_merge 三层保护声称把 LLM 失败模式拦在写库前，但 `apply_chunk_revision` 双写非事务、级联删除吞错、schema 校验时机、hash 稳定性等实现层是否有绕过保护的窗口。次要元家族=**信号/提案幂等底座**（重复 lint 去重、无界增长、claim CAS）。

## 产出

- 台账：`docs/superpowers/specs/2026-07-15-knowledge-wiki-audit-findings.md`（Task0 骨架 → 各簇填 → TaskE 收尾）。
- 报告 scratch：`.superpowers/audit4/cluster-{S,1,2,3}-report.md`（git-ignored，不进 commit）。
- 分支 `docs/knowledge-wiki-audit`，基于含 #210 的最新 origin/main。纯 docs 命中 paths-ignore，无 CI 风险。
- 只审不修：本批绝不改任何 `.rs`。

## 全局约束

- **worktree 铁律**：push 显式 refspec `HEAD:refs/heads/docs/knowledge-wiki-audit` + `ls-remote` 亲验 tip==本地；`gh pr create` 显式 `--head/--base`；建后+merge 前核 head/base/headOid；squash merge 不带 `--delete-branch`；不碰主仓在途工作（主仓被并行会话占 `feat/principal-auth-exemption`）。
- **subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带主控裁定理由；先 Write 报告再返回。
- **反过拟合红线**：audit-only，绝不为发现问题改业务逻辑/prompt/guards/阈值。
