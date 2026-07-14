# evolution 自优化演化器深度审查（第五批）Implementation Plan

> **For agentic workers:** 审查工程，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 的 implementer→reviewer 双裁决；改由**主控编排**：TaskS 根因层先派并等回 → 结论作基准喂 Task1-3 并行派 → 主控逐条亲验 file:line → 填台账。步骤用 checkbox 跟踪。

**Goal:** 对 evolution 自优化演化器（cohort 选择 → threshold/prompt 候选 → shadow replay + 显著性 → release/auto_release/post_release 放量闭环）做纯代码/设计审查，产出经主控逐条亲验的 findings 台账，合并 docs PR。

**Architecture:** 簇S（演化根因层：worker 主循环 tick 编排 + EvolutionBudget + runtime_flag 灰度门 + cohort 选择 + envelope 信封）先审并等回，其结论（tick 单次失败不传播、budget 耗尽 silent skip、runtime_flag 读失败按 disabled 兜底、cohort 灰度分桶 hash(contact)%100、隔离红线禁引 gateway/outbox/mcp）作"演化安全基准"喂给簇1-3；簇1-3 并行审候选生成/shadow评估/放量闭环是否守住幂等/统计正确性/红线（绝不自动回滚 + 绝不自动 verify）→ 主控 Read/Grep 逐条复核失效链 + 驳回夸大 → 汇总进单一台账 → docs PR。只审不修。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象 Rust（src/evolution/ 14 文件 6095 行）。

## Global Constraints

- 分支 `docs/evolution-audit`，基于含 #211 的最新 origin/main（102db83）。
- **只审不修**：本批绝不改任何 .rs。产出纯 docs（台账）。
- **subagent 只读**：不改任何文件；每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；凭猜测打回。
- **subagent 全部继承主会话 Opus**：省略 model 参数（`model:"opus"` 报 400，省略即继承）。用 general-purpose（非 Explore——Explore 读摘录漏 read window，不适合审计）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造竞态/重复副作用/统计错误/红线绕过的触发序列）。
- **严重度校准防夸大**：High=推荐配置（EVOLUTION_ENABLED 默认 true + 单进程 + admin 二次确认闭环）下确定性可达的：生产 prompt/threshold 被错误放量、绕过 admin 二次确认自动 release、自动回滚（Requirements 9.7 明禁）、演化越界写生产链路（隔离红线破洞）、统计显著性判定错误导致坏候选晋升。Medium=需并发/崩溃时机叠加或多副本/多租户才触发或有兜底。Low=观测/边缘/无界增长无立即后果/就绪债/桩未接线。**⚠️ 单进程默认不可达的多副本竞态=水平扩展就绪债；单租户默认不可达的隔离缺陷=多租户就绪债——都不夸成 High**（生产 117 单机 systemd 单进程、默认单 workspace）。每条带主控裁定理由。
- **元家族聚焦**：设计声称的演化安全不变量（绝不自动回滚只 admin 手工、release 需 admin 二次确认、EvolutionBudget 硬上限、隔离红线禁引 gateway/outbox/mcp、runtime_flag 读失败按 disabled 兜底防误开、shadow eval 不碰真实发送），实现层是否有旁路/竞态窗口/非原子写/统计错误/自动化越界——前四批"声称不变量实现层有旁路/层间不对称"元家族在自优化闭环侧的延伸。
- **边界排除（防与既往批次重叠）**：`agent::*` 生产链路（前四批已审，本批仅在隔离红线对照时引用不重审）；`routes::evolution` HTTP 端点层（若存在，第二批 auth/routes 已审授权面，本批仅审 worker 侧逻辑）；`prompt_critic` 调 `generate_agent_json` 的 LLM 入口本身（主链路已审，仅审 evolution 侧调用契约与 budget 记账）。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）；本会话在 worktree fix-full-system-remediation。

---

### Task 0: 建台账骨架

**Files:** Create `docs/superpowers/specs/2026-07-15-evolution-audit-findings.md`

- [ ] **Step 1: 写台账头部 + 字段模板**

头含审查范围（4 簇 + 文件清单 6095 行）、方法论、演化安全/幂等/统计检查清单 7 问、严重度校准口径（含单进程/单租户不可达=就绪债原则、自动化越界=High 判据）、元家族说明、边界排除清单。字段模板逐字：

```
### [X-NN] 一句话标题
- 入口: —（函数/worker）
- 所属簇: S|1|2|3
- 类型: 自动化越界|统计正确性|幂等|一致性(非原子写)|隔离红线|预算旁路|时间窗竞态|无界增长|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 失效链: （谁在什么时机触发什么错误放量/统计错误/红线绕过后果；非失效类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

台账含"环节汇总（收尾时填）"占位段（总数/严重度分布/元家族归纳/P0-P3 路线）+ 4 簇 findings 段（各留"（主控亲验后填入）"）。

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-15-evolution-audit-findings.md docs/superpowers/specs/2026-07-15-evolution-audit-design.md docs/superpowers/plans/2026-07-15-evolution-audit.md
git commit -m "docs(audit): evolution演化器审查设计+计划+台账骨架(第五批)"
```

---

### Task S: 演化根因层（worker 主循环 + budget + runtime_flag + cohort + envelope）审查 —— 先派并等回

**审查对象:** `src/evolution/mod.rs`（全 386，run_evolutionary_worker + run_one_tick 编排）+ `src/evolution/budget.rs`（223）+ `src/evolution/runtime_flag.rs`（195）+ `src/evolution/cohort.rs`（232）+ `src/evolution/envelope.rs`（111）+ `src/evolution/error.rs`（28）+ `src/evolution/lint.rs`（79）。

**Interfaces:**
- Produces: 簇S findings（S-NN）+ **演化安全基准**（喂给 Task1-3）：EVOLUTION_ENABLED 硬上限语义（false→return 不进 tick）、tick 单次失败不传播（catch + 写 event）、EvolutionBudget from_config 上限 + 耗尽 silent skip 语义、runtime_flag 读失败按 None(disabled) 兜底防灰度误开、cohort 灰度分桶 hash(contact_id)%100<rollout_percent、experiment envelope 状态机（evaluating→awaiting_admin）、隔离红线（禁引 gateway/outbox/mcp/tasks/webhooks，CI check-evolution-isolation.sh 静态扫描）。"演化该怎么算安全"的正确姿势。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus），等它回**

dispatch 指令要点：
- 审 mod.rs 全（run_evolutionary_worker 门控 + run_one_tick 9 步编排：envelope→runtime_flag→cohort→threshold→prompt_critic→budget 记账→replay+significance→awaiting_admin→post_release→auto_release，各步 unwrap_or_else 吞错语义）+ budget.rs 全（EvolutionBudget token/call 上限、from_config、消耗记账、BudgetExceeded 触发点）+ runtime_flag.rs 全（load_runtime_flag、bucket_for_contact、rollout_bucket_index、is_evolution_enabled_for、读失败兜底）+ cohort.rs 全（select_cohorts / select_cohorts_filtered 灰度过滤、cohort 去重、run_id 采样）+ envelope.rs 全（insert_experiment_envelope、update_experiment_status 状态机）+ error.rs + lint.rs。
- **重心**：①**自动化越界红线**：run_one_tick 直接推到 awaiting_admin（:204）——是否真的所有 release 都需 admin 二次确认？auto_release（:235）是唯一自动放量点，其 enabled 门控与"绝不自动回滚"（Requirements 9.7）边界。②**EvolutionBudget 硬上限**：token/call 上限是否真拦住 prompt_critic + replay 的 LLM 消耗？BudgetExceeded 是否被正确 silent skip（不向上传播炸 tick）？记账有无漏计/重复计？③**runtime_flag 灰度门**：读失败按 disabled（:106-112）防误开是否完整？hash 分桶是否稳定（同 contact 恒同桶）？rollout_percent 边界（0/100）？④**隔离红线**：evolution 全目录禁引 gateway/outbox/mcp/tasks/webhooks——有无越界？CI 扫描是否真覆盖？⑤**tick 失败隔离**：单 tick 失败 catch 后继续（:66-70）是否漏掉了应终止的场景？unwrap_or_else 吞错（post_release/auto_release）是否掩盖真错？⑥**workspace/account 隔离**：tick 用 default_workspace_id/default_account_id（:88-89）——多租户下是否只演化 default？
- **产出基准**：明确回答"evolution 演化该怎么算安全（自动化边界/预算/灰度/隔离/失败隔离）"，供 Task1-3 当审查标尺。
- 硬约束（先读懂+file:line+只读+两态+严重度带理由+单进程/单租户不可达=就绪债不夸 High；自动化越界/绕过 admin/自动回滚=High 判据）。报告写 `.superpowers/audit5/cluster-S-report.md`。

- [ ] **Step 2: 主控逐条亲验 + 提炼基准**

对 subagent 每个 finding Read/Grep 复核 file:line + 失效链；提炼"演化安全姿势"作为 Task1-3 dispatch 要点。

- [ ] **Step 3: 填台账（簇S）+ Commit**

```bash
git add docs/superpowers/specs/2026-07-15-evolution-audit-findings.md
git commit -m "docs(audit): 簇S 演化根因层(worker+budget+runtime_flag+cohort) findings(主控亲验)"
```

---

### Task 1-3: 资源域簇审查（TaskS 完成后并行派）

> 三簇结构相同，仅审查对象与重心不同。每簇：派只读 subagent（继承 Opus，带 TaskS 演化安全基准 + 演化安全/幂等/统计检查清单 7 问）→ 主控逐条亲验 → 填台账 → commit。报告写 `.superpowers/audit5/cluster-{1,2,3}-report.md`。

**演化安全/幂等/统计检查清单 7 问（喂给每簇 subagent，逐候选/eval/release 路径核）：**
1. 幂等：同一 experiment/proposal 重复 tick / 重复 eval / 重复 release 是否产生重复副作用（重复候选、重复放量、重复计数）？
2. 统计正确性：threshold 生成的统计口径（hold_rate/样本量）、significance 显著性判定（p 值/效应量/最小样本）是否有算错/误判导致坏候选晋升的路径？
3. 自动化越界红线：是否所有 release 都经 admin 二次确认？auto_release 的 enabled 门控 + close-loop 条件是否严格？**绝不自动回滚**（Requirements 9.7）是否 HOLDS（rollback 全 admin 手工）？
4. 预算旁路：replay/prompt_critic 的 LLM 消耗是否都过 EvolutionBudget？有无绕过预算的调用点？
5. shadow 隔离：replay/eval 是否真的 shadow（不碰真实发送 / 不写生产 chunk / 不触 gateway）？eval 结果写入是否只落 experiment/proposal 集合？
6. 一致性（非原子写）：experiment envelope 多字段分步 update、proposal 状态流转（pending_eval→eligible/rejected→released）是否有中间失败留不一致态？
7. 无界增长 / 崩溃恢复 / best-effort 吞错：experiment/proposal 是否无界堆积无 TTL？post_release_reviews 到期扫描崩溃能否回收？unwrap_or_else 吞错是否掩盖真错？

**每簇 subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带理由（**单进程默认不可达的多副本竞态=水平扩展就绪债、单租户默认不可达的隔离缺陷=多租户就绪债，均不夸成 High；自动化越界/绕过 admin/自动回滚=High 判据**）；元家族=找声称的演化安全不变量（不自动回滚/需 admin 确认/预算硬上限/shadow 隔离/统计正确）在实现层的旁路/竞态窗口/非原子写/统计错误。

- [ ] **Task 1 候选生成**：`src/evolution/threshold.rs`（467）+ `src/evolution/prompt_critic.rs`（606）。重心=threshold 候选纯统计生成正确性（hold_rate 样本量/阈值调整方向/边界）、prompt_critic LLM 候选生成（消 EvolutionBudget、BudgetExceeded silent skip、生成的 prompt 候选是否落 pending_eval 不直接生效）、proposal 幂等（同 cohort 重复 tick 是否重复建候选）、候选写入是否只落 proposals 集合不碰生产 prompt_templates。派→亲验→填台账（1-NN）→commit `docs(audit): 簇1 候选生成 findings`。
- [ ] **Task 2 shadow评估**：`src/evolution/replay.rs`（909）+ `src/evolution/significance.rs`（996）。重心=replay shadow eval 隔离（不碰真实发送/gateway、只读历史 run 重放）、eval 预算消耗过 EvolutionBudget、significance 统计显著性判定正确性（p 值/效应量/最小样本量门槛、aggregate_and_grade 的 eligible/rejected 判定、坏候选是否可能误判 eligible）、eval 结果写入幂等（重复 eval 同 proposal）、非原子写（significance 聚合多字段）。派→亲验→填台账（2-NN）→commit `docs(audit): 簇2 shadow评估 findings`。
- [ ] **Task 3 放量闭环**：`src/evolution/release.rs`（855）+ `src/evolution/auto_release.rs`（519）+ `src/evolution/post_release.rs`（489）。重心=**自动化越界红线**（release 需 admin 二次确认、auto_release 唯一自动放量点的 enabled 门控 + hold_rate close-loop 条件、**绝不自动回滚** Requirements 9.7 是否 HOLDS）、release 应用候选到生产的原子性（threshold/prompt 写生产配置是否有中间失败）、post_release +24h 对比窗口 run_due_reviews 崩溃回收、rollback 全 admin 手工无自动路径、release 幂等（重复 release 同 proposal）。派→亲验→填台账（3-NN）→commit `docs(audit): 簇3 放量闭环 findings`。

---

### Task E: 台账收尾 + push + PR

- [ ] **Step 1: 汇总头** —— 总 findings 数、严重度分布（H/M/L）、自动化越界/统计正确性/隔离类元家族归纳、后续 P0-P3 修复路线（若有 High 优先级高于前四批遗留 Medium）。
- [ ] **Step 2: 交叉去重** —— 扫全台账去重跨簇重复（如 EvolutionBudget 缺陷被多簇各报一次、隔离红线 / best-effort 吞错模式跨文件重复），归并留痕。
- [ ] **Step 3: Commit + push（显式 refspec）+ PR**

```bash
git add docs/superpowers/specs/2026-07-15-evolution-audit-findings.md
git commit -m "docs(audit): evolution演化器审查台账收尾(严重度分布+修复路线)"
LOCAL=$(git rev-parse HEAD)
git push origin HEAD:refs/heads/docs/evolution-audit -u
git ls-remote origin refs/heads/docs/evolution-audit   # 亲验 tip==LOCAL
gh pr create --head docs/evolution-audit --base main --title "..." --body "..."
gh pr view docs/evolution-audit --json number,headRefName,baseRefName,headRefOid  # 核身份
```

- [ ] **Step 4:** docs-only PR 走 paths-ignore，后端 job 大概率 skip（同 PR#207/#209/#210/#211）。核 CI 无意外 FAILURE 后 squash merge（不带 --delete-branch，worktree 铁律）。写 memory。

---

## Self-Review

**1. Spec coverage:** 4 簇 → TaskS + Task1-3 一一对应；只审不修+台账格式 → Task0 + 各 Step 填台账；主控亲验 → 各 Step 2；演化安全/幂等/统计清单 7 问+元家族+严重度校准 → Global Constraints + 各簇要点；失效链字段 → Task0 模板；后续修复路径 → TaskE Step1；边界排除（agent 生产链路/routes 授权面）→ Global Constraints。✓ 无遗漏。

**2. Placeholder scan:** 无 TBD/TODO。台账字段模板、检查清单 7 问、各簇审查对象+重心、commit message 均具体。TaskE 的 PR title/body `"..."` 执行时按实际 findings 填（届时才知内容），非计划占位。✓

**3. Type consistency:** finding 编号 S-NN/1-NN/2-NN/3-NN 全计划一致；台账路径 `docs/superpowers/specs/2026-07-15-evolution-audit-findings.md` 与报告文件 `.superpowers/audit5/cluster-{S,1,2,3}-report.md` 命名一致。✓

## 备注

- TaskS **必须先派并等回**（其基准喂后续），Task1-3 在 TaskS 完成后可一次性并行派 3 个 subagent。主控亲验各簇串行做保质量。
- 审查 subagent 用 general-purpose（只读指令约束，非 Explore），继承 Opus。
- 报告文件 `.superpowers/audit5/*.md` 是 git-ignored scratch，不进 commit。
- **本会话教训（第四批）**：subagent 从 transcript 恢复（SendMessage 续派）常 tool_uses=0 空返回；报告未落盘时直接重派全新 Agent（明确"先 Write 报告再返回 + 不删/不重建 audit5 目录只写自己的报告文件"），别反复续派。并行 subagent 共写同目录时先启动者建目录、后启动者勿重建（第四批 audit4 目录被后启动 subagent 重建清空过）。
- 本批零代码改动、零 CI 风险（纯 docs）。
