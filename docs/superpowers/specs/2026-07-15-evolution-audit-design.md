# evolution/ 自优化演化器深度审查（第五批）Design

> 只审不修 · 纯 docs 台账 · 沿用前四批「根因层先派→资源域并行 + 主控逐条亲验」范式。

## 背景与定位

第五批圈定 **`src/evolution/` 自优化演化器**（6095 行 14 文件），接续：

- [第一批 agent 旁挂能力审](2026-07-14-agent-capabilities-audit-findings.md)（20 findings/1H）—— PR#207
- [第二批 auth+routes 安全隔离面审](2026-07-14-auth-routes-security-audit-findings.md)（17 findings）—— PR#209
- [第三批后台 worker 群审](2026-07-14-worker-fleet-audit-findings.md)（18 findings/1H）—— PR#210
- [第四批 knowledge_wiki 子系统审](2026-07-15-knowledge-wiki-audit-findings.md)（32 findings/0H/5M）—— PR#211

前四批只在 KE 家族（knowledge evolution）对照时碰过 evolution/，**演化器内部 cohort/threshold/prompt_critic/replay/significance/release 全链从未深审**——这是剩余两大未审领域之一（另一为 db/migrations+indexes）。

## 子系统职责（mod.rs 亲验）

`src/evolution/mod.rs:1-16` 声明本模块是 agent-self-evolution M4 演化器：`run_evolutionary_worker`（mod.rs:53）由 main.rs 无条件 spawn，`EVOLUTION_ENABLED` 是硬上限（false→进函数即 return 不进 tick）。单 tick（run_one_tick，mod.rs:82）主流程：信封 → cohort 灰度过滤 → threshold 候选（纯统计不消预算）→ prompt critic 候选（消 EvolutionBudget）→ shadow replay + 显著性 → 状态推 awaiting_admin → post_release 到期复查 → auto_release threshold 自动放量。波次 W1-W4：W1 主循环+预算+信封+cohort / W2 threshold+critic 候选 / W3 shadow eval+显著性 / W4 release+回滚+post-release。

**隔离红线**（mod.rs:3-6/16）：本模块严禁引用 `crate::agent::gateway/outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway/handle_managed_message/handle_follow_up_task`、`tasks/webhooks`。`scripts/check-evolution-isolation.sh` CI 静态扫描强制。审查须核这条红线是否 HOLDS。

**关键红线（mod.rs:234 亲验）**：`auto_release` 是唯一自动放量点；**rollback 永远由 admin 手工——Requirements 9.7 不允许自动回滚**。审查须核 auto_release 是否守住"只自动放量、绝不自动回滚"边界。

**边界排除（防与前后批次重叠）**：
- `agent::knowledge_router` / `knowledge_wiki` 子系统已第四批审——仅对照不重审。
- `routes::evolution` HTTP 端点层（若有）已在第二批 routes 面审——仅在 admin 二次确认/workspace 守门对照时引用，不重审。
- `db/migrations+indexes` 留后续专批。
- `EvolutionBudget` 与 agent 的 `RunBudget` 是两套独立预算——只审 evolution 侧。

## 审查范围（4 簇 / 6095 行）

| 簇 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **S 演化根因层**（先派等回） | `mod.rs`(386) + `runtime_flag.rs`(195) + `budget.rs`(223) + `cohort.rs`(232) + `envelope.rs`(111) + `error.rs`(28) | 1175 | tick 主循环编排（单 tick 失败不传播、best-effort 吞错是否掩盖真错）；EVOLUTION_ENABLED 硬上限 + runtime flag 灰度门（读失败按 disabled 兜底是否 fail-safe）；`bucket_for_contact`/`rollout_bucket_index` 分桶确定性；EvolutionBudget 消耗/耗尽 silent skip；experiment 信封状态机 + cohort 选择去重/隔离 |
| **1 候选生成** | `threshold.rs`(467) + `prompt_critic.rs`(606) + `lint.rs`(79) | 1152 | threshold 纯统计候选正确性（hold_rate 等阈值提议）；prompt critic LLM 候选（消预算、BudgetExceeded 不传播）；lint 校验；proposal 落库幂等（重复 tick 是否重复建 proposal）；候选 status 状态机（pending_eval/eligible/rejected） |
| **2 shadow 评估** | `replay.rs`(909) + `significance.rs`(996) | 1905 | shadow replay 正确性（是否真隔离不碰生产发送/outbox 红线）；显著性统计（aggregate_and_grade 样本量/分级门槛）；replay 预算消耗；eval_all 崩溃恢复；统计判定 TOCTOU |
| **3 放量闭环** | `release.rs`(855) + `auto_release.rs`(519) + `post_release.rs`(489) | 1863 | release 放量原子性（proposal→active 切换）；**auto_release 只自动放量绝不自动回滚红线**（mod.rs:234）；hold_rate close-loop 自动放量门槛；post_release +24h 对比窗口复查；回滚由 admin 手工守门；release 幂等（重复 release 同 proposal） |

## 方法论（沿用前四批）

1. **簇S 根因层先派并等回** —— 提炼「演化器安全基准」（tick 编排失败语义、灰度门 fail-safe、预算耗尽处理、cohort 隔离、隔离红线）喂 Task1-3 当审查标尺。
2. **Task1-3 TaskS 完成后并行派** —— 各簇只读 subagent（general-purpose，继承 Opus 省略 model），带 TaskS 基准 + 检查清单。
3. **主控逐条亲验** —— 每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大严重度。
4. **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列）。

### 检查清单（喂每簇 subagent）

1. **隔离红线**：本模块是否严守禁引 gateway/outbox/mcp/tasks/webhooks？replay 是否真 shadow（不碰真实发送）？有无旁路？
2. **自动放量/回滚边界**：auto_release 是否只自动放量、绝不自动回滚（Requirements 9.7）？放量门槛是否可被绕过？
3. **预算**：EvolutionBudget 消耗/耗尽是否正确 silent skip 不传播 BudgetExceeded？预算计数是否原子？
4. **灰度门 fail-safe**：runtime_flag 读失败是否按 disabled 兜底（不让抖动误开灰度）？分桶是否确定性？
5. **幂等/状态机**：重复 tick 是否重复建 experiment/proposal？proposal status 流转（pending_eval→eligible→rejected→released）是否有非法跳转？release 重复放量？
6. **崩溃恢复/best-effort 吞错**：单 tick 失败不传播是否掩盖真损坏？post_release/auto_release unwrap_or_else 吞错是否留不一致？shadow eval 中途崩溃恢复？

## 严重度校准（防夸大，沿用前四批口径）

- **High**：推荐配置（单租户/单进程默认部署 + EVOLUTION_ENABLED=true 默认允许）下**确定性可达**的：自动回滚红线绕过、未经 admin 确认自动放量到生产、演化器污染生产决策链（破隔离红线）、预算失控、永久卡死。
- **Medium**：需多条件叠加/多副本才触发，或仅信息泄漏，或有自愈兜底，或需 runtime flag 显式开启灰度才可达。
- **Low**：观测/边缘/输入校验无后果/就绪债。
- **单进程默认不可达的并发竞态=水平扩展就绪债；runtime flag 默认关（enabled=false/文档不存在→全员排除）后的缺陷=灰度启用才触发的债——都不夸成 High**（[[project-multitenant-isolation-debt]] 口径 + 第三批部署拓扑维度）。
- **注意 EVOLUTION_ENABLED 默认值张力**：`EVOLUTION_ENABLED_DEFAULT="true"`（config.rs:7），但 config.rs:212 注释称"false 是安装态默认"——审查须亲验真实默认 + main.rs spawn 门控 + runtime flag 二级门，判定"演化器实际是否默认产出 proposal"，据此校准严重度。

## 元家族聚焦

本批预期主线元家族=**「自动化演化闭环的安全边界旁路」**：演化器声称"只自动放量、绝不自动回滚 + 严守隔离红线 + admin 二次确认"，但 auto_release 门槛、replay shadow 隔离、release 状态切换原子性、灰度门 fail-safe 等实现层是否有绕过安全边界的窗口。次要元家族=**幂等/预算底座**（重复 tick 去重、EvolutionBudget 原子性、proposal 状态机非法跳转、best-effort 吞错掩盖）。

## 产出

- 台账：`docs/superpowers/specs/2026-07-15-evolution-audit-findings.md`（Task0 骨架 → 各簇填 → TaskE 收尾）。
- 报告 scratch：`.superpowers/audit5/cluster-{S,1,2,3}-report.md`（git-ignored，不进 commit）。
- 分支 `docs/evolution-audit`，基于含 #211 的最新 origin/main（102db83）。纯 docs 命中 paths-ignore，无 CI 风险。
- 只审不修：本批绝不改任何 `.rs`。

## 全局约束

- **worktree 铁律**：push 显式 refspec `HEAD:refs/heads/docs/evolution-audit` + `ls-remote` 亲验 tip==本地；`gh pr create` 显式 `--head/--base`；建后+merge 前核 head/base/headOid；squash merge 不带 `--delete-branch`；不碰主仓在途工作（主仓被并行会话占 `feat/principal-auth-exemption`）。
- **subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带主控裁定理由；先 Write 报告再返回。全新派发（不用 SendMessage 从 transcript 恢复——第四批经验：恢复常 tool_uses=0 空返回）。
- **反过拟合红线**：audit-only，绝不为发现问题改业务逻辑/prompt/guards/阈值。
