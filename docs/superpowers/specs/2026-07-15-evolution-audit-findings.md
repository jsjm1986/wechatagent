# evolution 自优化演化器深度审查（第五批）Findings 台账

> 只审不修 · 纯 docs 台账 · 沿用前四批「根因层先派→资源域并行 + 主控逐条亲验」范式。

## 审查范围（4 簇 / 6095 行 14 文件）

| 簇 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **S 演化根因层**（先派等回） | `mod.rs`(386) + `budget.rs`(223) + `runtime_flag.rs`(195) + `cohort.rs`(232) + `envelope.rs`(111) + `error.rs`(28) + `lint.rs`(79) | 1254 | worker 门控 + run_one_tick 9 步编排；EvolutionBudget 硬上限；runtime_flag 灰度门读失败兜底；cohort 分桶；envelope 状态机；隔离红线。产出**演化安全基准**喂簇1-3 |
| **1 候选生成** | `threshold.rs`(467) + `prompt_critic.rs`(606) | 1073 | threshold 纯统计候选正确性；prompt_critic LLM 候选（消 budget + silent skip）；候选落 pending_eval 不直接生效；proposal 幂等 |
| **2 shadow评估** | `replay.rs`(909) + `significance.rs`(996) | 1905 | replay shadow 隔离（不碰真实发送/gateway）；eval 预算；significance 统计显著性判定正确性；坏候选误判 eligible；eval 幂等 |
| **3 放量闭环** | `release.rs`(855) + `auto_release.rs`(519) + `post_release.rs`(489) | 1863 | **自动化越界红线**（release 需 admin 二次确认 / auto_release 唯一自动放量 / 绝不自动回滚 R9.7）；release 应用候选原子性；post_release +24h 窗；rollback 全 admin 手工；release 幂等 |

## 方法论（沿用前四批）

1. **簇S 根因层先派并等回** —— 提炼「演化安全基准」（自动化边界/预算硬上限/灰度门兜底/隔离红线/失败隔离）喂 Task1-3 当审查标尺。
2. **Task1-3 TaskS 完成后并行派** —— 各簇只读 subagent（general-purpose，继承 Opus 省略 model），带 TaskS 基准 + 检查清单 7 问。
3. **主控逐条亲验** —— 每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大严重度。
4. **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列）。

## 演化安全/幂等/统计检查清单 7 问

1. **幂等**：同一 experiment/proposal 重复 tick / 重复 eval / 重复 release 是否重复副作用？
2. **统计正确性**：threshold 统计口径、significance 显著性判定是否有算错/误判致坏候选晋升？
3. **自动化越界红线**：所有 release 都经 admin 二次确认？auto_release enabled 门控严格？**绝不自动回滚**（R9.7）HOLDS？
4. **预算旁路**：replay/prompt_critic 的 LLM 消耗是否都过 EvolutionBudget？
5. **shadow 隔离**：replay/eval 是否真 shadow（不碰真实发送/生产 chunk/gateway）？
6. **一致性（非原子写）**：envelope 多字段分步 update、proposal 状态流转是否有中间失败留不一致？
7. **无界增长/崩溃恢复/best-effort 吞错**：experiment/proposal 无界堆积无 TTL？post_release 扫描崩溃能否回收？unwrap_or_else 吞错掩盖真错？

## 严重度校准（防夸大，沿用前四批口径 + 自动化越界维度）

- **High**：推荐配置（EVOLUTION_ENABLED 默认 true + 单进程 + admin 二次确认闭环）下**确定性可达**的：生产 prompt/threshold 被错误放量、绕过 admin 二次确认自动 release、**自动回滚**（R9.7 明禁）、演化越界写生产链路（隔离红线破洞）、统计显著性判定错误致坏候选晋升。
- **Medium**：需并发/崩溃时机叠加 / 多副本 / 多租户才触发，或有自愈兜底。
- **Low**：观测/边缘/无界增长无立即后果/就绪债/桩未接线。
- **单进程默认不可达的多副本竞态=水平扩展就绪债；单租户默认不可达的隔离缺陷=多租户就绪债——都不夸成 High**（[[project-multitenant-isolation-debt]] 口径 + 第三批部署拓扑维度）。

## 元家族聚焦

本批预期主线元家族=**「自优化闭环声称的安全不变量（绝不自动回滚/release 需 admin 确认/预算硬上限/shadow 隔离/统计正确）在实现层的旁路/自动化越界」**——前四批「声称不变量实现层有旁路/层间不对称」元家族在自优化闭环侧的延伸。

## 边界排除（防与既往批次重叠）

- `agent::*` 生产链路（前四批已审）——仅在隔离红线对照时引用不重审。
- `routes::evolution` HTTP 授权面（第二批 auth/routes 已审）——本批仅审 worker 侧逻辑。
- `prompt_critic` 调 `generate_agent_json` 的 LLM 入口本身（主链路已审）——仅审 evolution 侧调用契约 + budget 记账。

## 环节汇总（收尾时填）

- 总 findings 数：（TaskE 填）
- 严重度分布：（H/M/L，TaskE 填）
- 元家族归纳：（TaskE 填）
- 后续 P0-P3 路线：（若有 High 优先级高于前四批遗留 Medium，TaskE 填）
- 交叉去重留痕：（TaskE 填）
- 正向 HOLDS（主控亲验）：（TaskE 填）

---

## 字段模板

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

---

## 簇 S findings（演化根因层）

> 4 findings（0 High / 1 Medium / 3 Low）。主控逐条 Read/Grep 亲验：S-01 门语义分叉 CONFIRMED（`runtime_flag.rs:90 Ok(None)=>false` vs `cohort.rs:65,103-108 None=全量收`，`evolution_runtime_flags` 无启动 seed → 唯一写点 `routes/evolution.rs:616` admin PUT + m016 是 backfill 非 seed）；S-02/S-03/S-04 均 Low 就绪债。subagent 本簇未夸大（唯一 Medium 定级恰当，未误标 High）。

### [S-01] runtime_flag=None 被 cohort 当「全量收」，与文档声称的「全员排除」相反 → 默认部署演化跑全流量而非灰度桶
- 入口: `run_one_tick` → `select_cohorts_filtered`（`src/evolution/mod.rs:106-117` + `src/evolution/cohort.rs:61-108`）
- 所属簇: S
- 类型: 一致性(门语义分叉)|fail-safe 反向
- 严重度: **Medium**（主控裁定：默认部署 EVOLUTION_ENABLED=true + runtime_flag 文档未 seed 下**确定性可达**——演化器对全量 completed run 选 cohort 产 proposal，非设计意图的灰度桶/None→全排除。但产出一律推 `awaiting_admin`，auto_release 双闸默认全关，**不会自动改生产 prompt/threshold**。故不构成「绕过 admin 自动放量」的 High；实际后果=灰度 fail-safe 网默认失效 + 演化拿全流量样本 + LLM 预算按全量消耗，有 admin 兜底 → Medium）
- 现象/风险: 运维依 `mod.rs:101-105`/`runtime_flag.rs:29-31` 注释以为「不配 mongo flag=演化不选样本」，实际对全部客户对话产 proposal；mongo 抖动读失败同样落 None→全量收，`mod.rs:105` 注释「避免 mongo 抖动让灰度门误开」未兑现。
- 失效链: `load_runtime_flag` 文档不存在/读失败 → `Ok(None)`/warn→None（`mod.rs:106-112`）→ `select_cohorts_filtered(..., None)` → `cohort.rs:103 if let Some(flag)` 为 None 时跳过整个桶过滤，全量入 `threshold_pool`（`cohort.rs:104-109`）。
- 根因（亲验 file:line）:
  - `src/evolution/cohort.rs:64-65` 文档「`None` 等价于"不过滤、全量收"，保持 W1 行为」。
  - `src/evolution/cohort.rs:103-108` `if let Some(flag) = runtime_flag { ... continue; }`——None 时不进过滤分支。
  - `src/evolution/mod.rs:101-105` 注释矛盾：「`enabled=false` 或文档不存在 → 全员排除」。
  - 对照 `src/evolution/runtime_flag.rs:88-90` `is_evolution_enabled_for` 里 `Ok(None) => false`（正确排除）——两函数 None 语义分叉。
  - `evolution_runtime_flags` 无启动 seed：唯一写点 `routes/evolution.rs:616` admin PUT upsert；m016（`m016_backfill_workspace_id_on_legacy_rows.rs:65,192`）是给已有行 backfill workspace_id 的迁移，集合空时无操作，非 seed。
- 复现设想: 全新部署，不调 `PUT /api/evolution/runtime-flag`；窗口内积累 ≥30 条 completed run；一次 tick 后 `experiments.cohort_threshold_run_ids` 含全部客户 run（未按桶过滤），`proposals` 出现 awaiting_admin 候选。
- 验证状态: **CONFIRMED**（代码路径 + 无 seed 双证默认可达；无自动放量后果亦亲验）
- 修复建议: 二选一——(a) 把 `select_cohorts_filtered` 的 `None` 语义改「全员排除」（与 `is_evolution_enabled_for` 及 mod.rs 注释对齐），未配 flag 的 workspace 跑空 tick（推荐，与 kill-switch 语义一致、保守）；或 (b) 保留 W1「全量收」则修正 `mod.rs:101-105`/`runtime_flag.rs:29-31` 注释，并评估默认全量选样本对 LLM 预算与样本隐私面的影响。
- 状态: Open

### [S-02] config.rs:212 注释声称 evolution_enabled 安装态默认 false，与真实默认 true 矛盾（stale 注释误导运维）
- 入口: `AppConfig`（`src/config.rs:210-217`）
- 所属簇: S
- 类型: 就绪债(文档/代码漂移)
- 严重度: **Low**（主控裁定：纯注释与代码常量矛盾，无运行时后果；但误导运维对「默认是否跑演化」的判断，与 S-01 叠加放大误解）
- 现象/风险: `config.rs:212` 写「`evolution_enabled=false` 是安装态默认」，实际 `EVOLUTION_ENABLED_DEFAULT="true"`（`config.rs:7`）+ 测试锁死 true（`config.rs:781-783`）；且同段 `config.rs:215` 字段 doc 又说「默认 true（允许）」——同段内自相矛盾。
- 失效链: —（纯文档）
- 根因（亲验 file:line）: `src/config.rs:7 ="true"`；`src/config.rs:212` 注释 `=false 是安装态默认`；`src/config.rs:215-216` 字段 doc `默认 true（允许）`；`src/config.rs:781-783 assert_eq!(EVOLUTION_ENABLED_DEFAULT,"true")`。取 :7/:782/:215 为真，:212 为 stale。
- 复现设想: N/A（静态矛盾）
- 验证状态: CONFIRMED
- 修复建议: 删/改 `config.rs:212` 那句，与 `config.rs:215-216` 字段 doc 统一为「默认 true=允许 UI 开演化中心；false=运维硬锁定」。
- 状态: Open

### [S-03] 隔离 lint 用路径字符串子串匹配，grouped import 形态可绕过（当前无违规）
- 入口: `scripts/check-evolution-isolation.sh`（`FORBIDDEN_PATTERNS`，`:33-42`）
- 所属簇: S
- 类型: 就绪债(lint 健壮性)|隔离红线
- 严重度: **Low**（主控裁定：lint 靠 `grep -E 'crate::agent::gateway'` 等连续字符串子串；`use crate::agent::{gateway, outbox}` 形态不含该连续子串可规避。但 Grep 亲验当前 evolution 目录无任何 grouped import，实际未被绕过 → 无现存后果，纯防御纵深债）
- 现象/风险: 未来若有人以 grouped import 引入 gateway/outbox/mcp，CI lint 可能漏报，破隔离红线不被拦。
- 失效链: 仅「有人主动用 grouped import 写破红线代码」时；正常单符号 import 全被覆盖。
- 根因（亲验 file:line）: `scripts/check-evolution-isolation.sh:33-42` 模式为完整路径字符串，`:49-50 grep -n -E` 逐行匹配。Grep `use crate::agent::\{` on `src/evolution` → 无匹配。
- 复现设想: 某 evolution 文件写 `use crate::agent::{gateway, budget};` → `crate::agent::gateway` 模式因中间 `{` 不命中 → 漏报。
- 验证状态: PLAUSIBLE（规避理论成立；当前无触发实例）
- 修复建议: lint 补 grouped import 形态（如 `crate::agent::\{[^}]*gateway`），或改用 cargo 层依赖检查（cargo-deny/模块可见性）。低优先，可与其它 lint 加固合并。
- 状态: Open

### [S-04] tick 硬编码 default_workspace_id/default_account_id — 多租户下仅演化 default 租户
- 入口: `run_one_tick`（`src/evolution/mod.rs:88-89`）
- 所属簇: S
- 类型: 就绪债(多租户)
- 严重度: **Low**（主控裁定：单进程默认单 workspace 下无害——正是唯一被演化的租户。多租户启用后其它 workspace 不被演化=功能缺失非正确性/安全问题。多租户就绪债，按校准铁律不夸 High/Medium）
- 现象/风险: 多租户下非 default workspace/account 的 run 永不进演化 cohort，不产 proposal；auto_release 同样只扫 default（`auto_release.rs:52,65`）。
- 失效链: 多租户部署 → 非 default 租户演化能力静默缺失。
- 根因（亲验 file:line）: `src/evolution/mod.rs:88-89 default_workspace_id/default_account_id`；`src/evolution/auto_release.rs:52,65` 只取 default。
- 复现设想: 配 2 workspace，非 default 的积累 run，观察其永无 experiments/proposals。
- 验证状态: PLAUSIBLE
- 修复建议: 多租户化时改遍历活跃 workspace（各跑独立 tick）；当前单租户不动，标记水平扩展/多租户就绪债。
- 状态: Open

**演化安全基准（喂簇1-3）**：①EVOLUTION_ENABLED 真实默认 **true**（config.rs:7），worker 默认跑起来且默认产 proposal 推 awaiting_admin——拦「自动改生产」的是 auto_release 双闸（默认全关）+ admin 二次确认，非 EVOLUTION_ENABLED。②tick 单次失败不传播（mod.rs:66-70）、EvolutionBudget 耗尽正确 silent skip（mod.rs:148-156/189-194，budget.rs:44-52 saturating+max(0)）、分桶确定性/单调性/边界全 HOLDS（runtime_flag.rs:51-68）。③隔离红线 HOLDS：CI lint 已接线（ci.yml:137-139），全目录无 gateway/outbox/mcp/发送链引用，只用只读/纯计算符号，当前无 grouped import 规避。④R9.7 禁自动回滚 HOLDS：唯一自动放量点是 threshold auto_release（双闸默认关+方向门 auto_release.rs:222-242），prompt 永远 admin 确认，无任何自动回滚。⑤簇1-3 重点核查：critic/replay 每次 LLM 调用是否 check_or_fail+record_call（预算旁路面）+ replay 是否真短路 gateway/outbox/mcp（隔离红线延伸面）+ significance 显著性判定与 min delta 门。

## 簇 1 findings（候选生成）

（主控亲验后填入）

## 簇 2 findings（shadow评估）

（主控亲验后填入）

## 簇 3 findings（放量闭环）

> 本簇（release/auto_release/post_release）是最可能出 High 的簇（自动放量红线所在）。主控逐条亲验后**无 High/无 Medium，3 条全 Low**，R9.7「禁自动回滚」HOLDS。

### [3-01] release_threshold/release_prompt 的 eligible 校验在事务外、update 无 status CAS 守卫 → 并发双重放量
- 入口: `release_threshold`（`release.rs:40-155`）、`release_prompt`（`release.rs:229-372`）
- 所属簇: 3
- 类型: 幂等
- 严重度: **Low**（主控裁定：并发窗口存在但效果良性 + unique 索引双重兜底，非确定性可致错；需 admin 双击/多 tab 并发才触发，有兜底）
- 现象/风险: proposal `status="eligible_for_release"` 校验（`release.rs:55-60`，事务前 find_one 读）与放量 update（`release.rs:106-124`，filter 仅 `{_id: proposal_id}` 无 status CAS）之间无原子守卫。两并发 release 可都通过校验、都执行放量。
- 失效链: admin 双击「放量」或多 tab 并发 → 两请求都读到 eligible → 都写 override + 都推 proposal 状态。threshold 情形：两次写同 override value（`$set` 同值幂等良性）；prompt 情形：`prompt_templates` `(workspace_id,prompt_key,version)` unique 索引第二次 insert 撞 E11000 → 事务 abort → 仅一次成功。
- 根因（亲验 file:line）:
  - `release.rs:55-60`：`if proposal.status != "eligible_for_release"` 校验在 `start_transaction`（`release.rs:82-85`）**之前**。
  - `release.rs:106-124`：override insert + proposal 状态推进（`update_one_with_session` filter 仅 `{_id: proposal_id}`，无 `status` 前置条件）。
  - `release.rs:87-104`：override 用 insert_one（`threshold_overrides`）；proposal `$set` 同值重放幂等。
  - prompt 对照：`release.rs:330-345` insert `prompt_templates` 撞 unique 索引兜底。
- 复现设想: admin UI 双击放量，两请求几乎同时到达；threshold 两次写同 override（无害），prompt 第二次 E11000 事务回滚。
- 验证状态: PLAUSIBLE（并发窗口存在；无 CAS 但效果良性 + unique 索引双重兜底）
- 修复建议: update filter 加 `status: "eligible_for_release"` 作 CAS 守卫，`matched_count==0` 中止返 409；或事务内重读 proposal 状态。属防御纵深，非当前可致错。
- 状态: Open

### [3-02] post_release review 先置 completed=true 再写 agent_event，事件写失败则审计永久丢失且 will-retry 不可达
- 入口: `process_one_review`（`post_release.rs:158-238`）
- 所属簇: 3
- 类型: 一致性(非原子写)
- 严重度: **Low**（主控裁定：纯观测子系统，delta 已落 review 文档本身，丢失的仅是冗余审计 agent_event，无功能/正确性损害）
- 现象/风险: `run_due_reviews` 扫 `completed:false` 到期 review（`post_release.rs:82-93`），`process_one_review` 先置 review `completed=true`+写 delta 到 review 文档（:190-205）再写 `evolution_post_release_delta` agent_event（:210-233）。若事件写失败，审计 event 永久丢失，且 review 已 completed → 下 tick 不再命中，"will retry" 不可达。
- 失效链: mongo 瞬时抖动致 event insert 失败 → delta 审计 event 丢失（但 delta 已在 review 文档留档 :190-205）→ 面板/审计事件流缺一条，无功能损害。
- 根因（亲验 file:line）: `post_release.rs:190-205`（先置 completed+写 delta 字段）→ `:210-233`（后写 agent_event，Err 仅 warn）→ `:82-93`（下 tick 只扫 `completed:false`）。
- 复现设想: mongo 在 :190 update 成功、:210 event insert 失败的瞬时窗口。
- 验证状态: PLAUSIBLE
- 修复建议: 先写 event 再置 completed；或 delta 已在 review 文档留档，审计 event 视为纯冗余观测可接受丢失。属观测保真度。
- 状态: Open

### [3-03] 放量 commit 后 write_release_event 冒泡 Err，auto_release 误记 will-retry（实则 status 已 released 不会重试）
- 入口: `auto_release_eligible_thresholds`（`auto_release.rs:249-320` 循环体）
- 所属簇: 3
- 类型: 一致性
- 严重度: **Low**（主控裁定：阈值已成功 live，仅审计 event 写失败致日志误导性 retry 语义，幂等安全无二次放量）
- 现象/风险: `release_threshold` 事务 commit 成功（阈值已 live，`release.rs:147` commit 在 `:149` write_release_event 之前）后，`write_release_event` 若 Err 冒泡，auto_release 循环体 catch 记 "will retry next tick"，但 proposal 已 released，下 tick query 只选 `eligible_for_release` 不再命中 → 不会真重试。仅日志误导。
- 失效链: 阈值已放量 live → 仅 release_event 审计写失败 → 日志说 will retry 实则不重试 → 无二次放量（幂等安全），仅日志/审计保真度问题。
- 根因（亲验 file:line）: `release.rs:147`（事务 commit）先于 `release.rs:149-` write_release_event；auto_release 循环体 catch Err→warn "will retry"；下 tick query 硬编码 `status:"eligible_for_release"`（`auto_release.rs:78`）released 的不再选。
- 复现设想: auto_release 放量 threshold 成功、write_release_event 遇 mongo 抖动 Err。
- 验证状态: PLAUSIBLE
- 修复建议: write_release_event 改 best-effort（Err 仅 warn 不冒泡），与 [3-02] 同理——审计 event 写失败不应触发误导性 retry 语义。属日志保真度。
- 状态: Open

**簇3 正向 HOLDS（主控亲验）**：
- **R9.7 禁自动回滚 HOLDS**：`rollback_threshold`/`rollback_prompt` 仅在 `release.rs:428/555` **定义**，evolution/ 内**零调用**；唯一调用点 `routes/evolution.rs:210/211`（AuthenticatedAdmin + workspace scope）。tick/auto_release/post_release 内零 rollback 符号。
- **auto_release 双闸默认全关**：env 总闸 `evolution_auto_release_enabled` 默认 "false"（`config.rs:655-658`）AND per-workspace 子闸 `threshold_auto_release_enabled`（读失败/缺失→`.ok().flatten()`→None→`unwrap_or(false)`→关，`auto_release.rs:39-64`，fail-safe 方向正确、**未踩 [S-01] 反向坑**）。
- **prompt 绝不自动放量**：auto_release query 硬编码 `proposal_kind="threshold"`（`auto_release.rs:77-78`），prompt 永远 admin 二次确认。
- **release 三写同事务原子**：override insert + proposal 推进 + audit 行同一 transaction（`release.rs:87-147`），commit 前完成（#155 P1 修过 commit 后 best-effort 审计漏写）。
- **红线三闸不 fail-open**：release_prompt NeedsHumanConfirm→RedlineGateRejected 中止（`release.rs:281-284`）；rollback_prompt 目标版本缺失早返中止事务（`release.rs:658-662`）。
- **隔离红线守住**：三文件无 gateway/outbox/mcp 符号（post_release 唯一 agent 引用是只读 domain_profile 加载 :384，非发送链）。
