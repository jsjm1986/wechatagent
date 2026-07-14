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

## 环节汇总

- **总 findings 数：11**（簇S 4 + 簇1 1 + 簇2 1 + 簇3 3；另簇1/2 各因映射同源交叉，见下）。
- **严重度分布：0 High / 2 Medium / 9 Low**。2 Medium=[S-01]（默认 runtime_flag=None 被 cohort 当全量收，灰度 fail-safe 网默认失效但 admin 兜底）、[2-01]（post_release 面板 5 闸命中率 delta 用对调 status 映射查生产真实终态→贴错标签，纯观测不反哺决策）。**本批无 High**——最可能出 High 的簇3（放量闭环/自动放量红线）逐条亲验后 3 条全 Low，R9.7「禁自动回滚」HOLDS。
- **元家族归纳**：本批主线元家族=**「自优化闭环声称的安全不变量在实现层的语义分叉/口径漂移」**，前四批「声称不变量实现层有旁路/层间不对称」在自优化闭环侧的延伸，具体两支：
  1. **门语义分叉 / fail-safe 反向（[S-01]，最尖锐）**：同一 runtime_flag=None 输入，`is_evolution_enabled_for`（正确排除）与 `select_cohorts_filtered`（当全量收）两函数分叉，加之 `evolution_runtime_flags` 无启动 seed，默认部署灰度 fail-safe 网失效。这是「同一安全语义在两个门函数实现不一致」的典型，与第四批 [3-02]「状态机字面量自相矛盾」同族。
  2. **gate↔status 映射三文件三向不一致（[1-01]/[2-01]）**：threshold.rs:67-68 / significance.rs:53-54 一份、post_release.rs:55-56 **恰好对调**，且**都与生产真相不符**（生产：fact_risk 硬闸→held_by_ai_policy、pressure_risk 软闸→revision 不产 block status、blocked_by_safety_guard 来自产品声明 fail-closed/relay）。关键分野：shadow 闭环（threshold/significance/replay）**自造合成 status 且同口径判定**→闭环内自洽零后果（[1-01] Low）；post_release **读生产真实 status 计数填面板**→对调直接贴错标签（[2-01] Medium）。元教训=**同一份领域映射散在多文件各写一份、无单一权威常量，必然漂移**（与第四批「聚合 filter 字段与真实写点不对齐」同族——都是"引用生产语义却未回溯真实写点亲验"）。
- **后续 P0-P3 路线**：本批**无 High**，2 Medium 优先级**低于**前三批遗留（Batch3 High [1-01] initial_profile 无终态写仍是全局 P0；Batch1/2/4 共 12 Medium 在前）。本批 Medium 修复取向：[S-01] 把 `select_cohorts_filtered` 的 None 语义改「全员排除」与 `is_evolution_enabled_for`+mod.rs 注释对齐（推荐 (a)，与 kill-switch 语义一致）；[1-01]+[2-01] 合并收口——三文件统一引用一份**经生产语义核实**的权威 `(gate_key ↔ final_review_status)` 映射常量，post_release 侧尤须修正（唯一读生产真实 status 的路径），并重新定义 pressure_risk 在生产走软闸 revision、无对应 block 终态时其"命中率"口径（可能应改 revision_count/revision_applied）。
- **交叉去重留痕**：[1-01]（簇1）与 [2-01]（簇2）**同源**（同一份 gate↔status 映射漂移），但分列两条因**后果层级不同**——[1-01] 在合成闭环内自洽无后果（Low），[2-01] 在 post_release 读生产真实 status 贴错面板标签（Medium）；[3-02]（簇3 post_release 非原子写）与 [3-03]（簇3 auto_release write_release_event 冒泡）**同族**（审计 event best-effort 写失败触发误导性 will-retry 语义），修复取向一致（审计 event 改 best-effort 不冒泡 / 先写 event 再置终态）。[S-04]（tick 硬编码 default workspace）与前四批多租户就绪债（[[project-multitenant-isolation-debt]]）同族。
- **正向 HOLDS（主控亲验，逐条 file:line）**：①**R9.7 禁自动回滚 HOLDS**——rollback 仅 release.rs:428/555 定义，evolution/ 内零调用，唯一调用点 routes/evolution.rs:210/211（AuthenticatedAdmin）。②**auto_release 双闸默认全关**——env 总闸 config.rs:655-658 默认 false AND 子闸 auto_release.rs:39-64 缺失→None→关（fail-safe 方向正确，未踩 [S-01] 反向坑）。③**prompt 绝不自动放量**——auto_release query 硬编码 proposal_kind="threshold"。④**release 三写同事务原子**——override+proposal+audit 同 transaction（release.rs:87-147）。⑤**红线三闸不 fail-open**——release_prompt NeedsHumanConfirm→RedlineGateRejected 中止（release.rs:281-284）。⑥**隔离红线守住**——CI lint 已接线（ci.yml:137-139），全目录无 gateway/outbox/mcp/发送链引用，只用只读/纯计算符号，当前无 grouped import 规避。⑦**prompt_critic/replay 预算记账完整+shadow 隔离**——check_or_fail+record_call 齐备，直调 llm client 绕开 RunBudget，静态扫描单测兜底禁 outbox/mcp 引用。⑧**tick 失败隔离+budget silent skip**——单 tick 失败不传播（mod.rs:66-70），BudgetExceeded 拦下不炸 tick（mod.rs:148-156/189-194）。⑨**significance 安全回归门零容忍**——max_safety_regression_rate 默认 0.0，任一风险消息 blocked→sent 即否决放松提案。

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

> **审查方式**：subagent 三次派发均遇 API mid-response 失败（未落盘报告），主控亲自逐行读 `threshold.rs`(467) + `prompt_critic.rs`(606) + 追证生产侧 `agent/review/gates.rs`(fact_risk/pressure_risk 闸机制) + `replay.rs`/`significance.rs`(闭环口径) 完成审查。file:line 均主控 Read/Grep 亲验。

### [1-01] pressure_risk_block ↔ block status 映射在生产语义上失配（但演化闭环内自洽 → 无后果）
- 入口: `classify_gate_hit`（`src/evolution/threshold.rs:64-73`）+ `SAFETY_GATE_BLOCK_STATUS`（`src/evolution/significance.rs:52-56`）
- 所属簇: 1
- 类型: 统计正确性（映射与生产语义不符）
- 严重度: **Low**（主控裁定：映射与生产真相不符，但演化 shadow 闭环**自造合成 status 且用同一口径判定**，不读生产真实 status 做安全回归判决 → 闭环内零后果；仅属"注释/映射与生产语义漂移"的可读性债）
- 现象/风险: `threshold.rs:68` 与 `significance.rs:54` 都把 `pressure_risk_block` 映射到 `blocked_by_safety_guard`，`fact_risk_block` 映射到 `held_by_ai_policy`。但生产真相（亲验 `agent/review/gates.rs`）：**fact_risk（hallucination_score≥阈值）是硬闸 → HardGateFailure → `held_by_ai_policy`**（gates.rs:875）；**pressure_risk 是软闸 → SoftGateFailure → 触发 revision，从不直接产 block status**（classify_dual_gate soft path）；`blocked_by_safety_guard` 实际来自产品声明 fail-closed（gates.rs:450 R5.3.a）+ relay 泄漏拦截（gateway.rs:2581），**与 pressure_risk 无因果**。故 pressure_risk_block 被映射到一个它在生产里永不产生的 status。
- 失效链: **不成立（闭环内自洽）**——shadow replay 不读生产真实 `final_review_status`，而是用 `final_status_from_5gate`（`replay.rs:411-430`）把 5 闸命中向量**重推合成 status**：pressure_risk_block 命中→写合成 `blocked_by_safety_guard`（:420-421），与 `significance.rs:54` 的 `safety_block_status_for` 同口径；`grade_safety_regression`（significance.rs:98-119）在合成 original/new status 间比对，两侧口径一致 → 安全回归判定正确。合成世界与生产 status 命名"恰好"用同一份错映射，故闭环内自洽、判决无误。
- 根因（亲验 file:line）:
  - `src/evolution/threshold.rs:67-68` `held_by_ai_policy=>fact_risk_block` / `blocked_by_safety_guard=>pressure_risk_block`（含 :391-395 单测锁死此映射）。
  - `src/evolution/significance.rs:52-56` `SAFETY_GATE_BLOCK_STATUS` 同一份映射。
  - `src/evolution/replay.rs:411-430` `final_status_from_5gate` 合成 status 用同口径（:418-424）。
  - 对照生产：`src/agent/review/gates.rs:120-124`（hallucination_score≥fact_risk_block_at→hard_risks）、:160-173（pressure_risk≥阈值→soft_risks→revision direction，非 block）、:875（HardGateFailure→held_by_ai_policy）、:450（R5.3.a claim fail-closed→blocked_by_safety_guard）、`gateway.rs:2581`（relay 泄漏→blocked_by_safety_guard）。
- 复现设想: N/A（闭环自洽，无可触发的错误判决路径）。
- 验证状态: **CONFIRMED**（映射与生产语义失配已亲验；闭环内自洽消除后果亦亲验——shadow 用合成 status 而非生产真实 status）
- 修复建议: 与 [2-01] 一并收口——统一一份权威 `(gate_key ↔ 生产真实 status)` 映射常量（供三处引用），并在 shadow 侧显式声明"合成 status 命名仅内部对照、不等于生产语义"。当前无功能后果，属可读性/防未来误用债，低优先。
- 状态: Open

**簇1 正向 HOLDS（主控亲验）**：
- **prompt_critic 预算记账完整**：`budget.exhausted()` 预检（`prompt_critic.rs:95`）+ `check_or_fail()`（:126）+ 成功记 `total_tokens`/失败记 0token+1call（:137/:167）；直调 `state.llm.generate_json_with_usage`（:130-133）**故意绕开** `agent::generate_agent_json`（:7 注释——避免读 task-local RunBudget），用独立 EvolutionBudget → 隔离红线 HOLDS，无发送链引用。
- **threshold 纯统计 + 方向正确**：`decide_candidate`（threshold.rs:351-379）方向语义正确（hit_rate<lower→阈值过高→-step；>upper→+step）；hard clamp（:376）、cooldown（:260-289）、per-tick quota（MAX=4，:218）齐备；候选一律 `pending_eval`（:222）不直接生效；current_value 基于当前生效 override（#155，:151-154）非硬编码。
- **classify_gate_hit + revision_applied 补判**：human_like/emotional_value rewrite 类经 `run.revision_applied` 补判（threshold.rs:107-116），不漏 rewrite 信号。

## 簇 2 findings（shadow评估）

> **审查方式**：subagent 做完实质工作（113k tokens/26 工具）但中途返回自问语、SendMessage 续派空返（未落盘）；主控亲自读 `replay.rs`(909) + `significance.rs`(996) + `post_release.rs`(489 交叉) 关键路径完成审查。file:line 均主控亲验。

### [2-01] post_release 面板 5 闸命中率 delta 用「对调的」status 映射查生产真实终态 → fact_risk/pressure_risk 命中率贴错标签
- 入口: `compute_window_metrics`（`src/evolution/post_release.rs:270-335`）经 `FIVE_GATE_KEYS`（`post_release.rs:54-60`）
- 所属簇: 2
- 类型: 统计正确性（观测口径贴错标签）
- 严重度: **Medium**（主控裁定：与 [1-01]/簇内合成世界不同，post_release **读生产真实 `final_review_status` 计数**填面板 delta，映射对调 → 面板上 fact_risk_block / pressure_risk_block 命中率 delta 是**贴错标签的数字**，admin 可能据此误读演化前后的闸命中变化。但 post_release 全程**纯观测**——delta 只写 `agent_events` details 供 admin 察觉，`post_release.rs:167/220` 明示**不参与任何 promote/rollback 判决**，不反哺自动放量/回滚。既非客户面、也不驱动任何自动决策 → 有界的观测面误导，Medium）
- 现象/风险: `post_release.rs:55-56` 的 `FIVE_GATE_KEYS` 把 `fact_risk_block` 映射到 `blocked_by_safety_guard`、`pressure_risk_block` 映射到 `held_by_ai_policy`——**与 threshold.rs:67-68 / significance.rs:53-54 恰好对调**（三文件三向不一致）。`compute_window_metrics:309-322` 用这份映射的 `status` 去 `count_documents` 查窗口内生产真实 `final_review_status`，把计数塞进 `five_gate_hit_rate[gate_key]`。生产真相（[1-01] 已亲验）：`held_by_ai_policy` 来自 fact_risk 硬闸、`blocked_by_safety_guard` 来自产品声明 fail-closed/relay——两个 label 都不对应它们在 post_release 里被赋予的 gate_key。故面板显示的「fact_risk_block 命中率 delta」实为 blocked_by_safety_guard 计数，「pressure_risk_block 命中率 delta」实为 held_by_ai_policy 计数。
- 失效链: 放量某 threshold → +24h 后 `run_due_reviews`→`process_one_review`→`compute_window_metrics` 用对调 status 查前/后窗口生产终态 → `actual_5gate_hit_delta` 面板数字 fact/pressure 两 gate 互换标签 → admin 读 EvolutionCenterTab 时对这两闸的命中率变化判断反向。不影响任何自动判决（纯观测）。
- 根因（亲验 file:line）:
  - `src/evolution/post_release.rs:54-60` `FIVE_GATE_KEYS`：`("fact_risk_block","blocked_by_safety_guard")` / `("pressure_risk_block","held_by_ai_policy")`——与 threshold/significance 对调。
  - `src/evolution/post_release.rs:309-322` `for (gate_key, status) in FIVE_GATE_KEYS { count_documents(final_review_status==*status) → five_gate_hit_rate[gate_key] }`——**用了 status 查生产真实终态**（区别于 process_one_review:174 那处 `_status` 丢弃）。
  - `src/evolution/post_release.rs:167/220` 明示 delta 纯观测不参与 promote/rollback。
  - 对照生产语义见 [1-01] 根因（gates.rs:875/450 + gateway.rs:2581）。
- 复现设想: auto_release 或 admin 放量一个 threshold；+24h 后 tick 触发 post_release；查该 review 的 `evolution_post_release_review` 事件 details.actual_5gate_hit_delta，fact_risk_block 的数字实际反映 blocked_by_safety_guard 的命中率变化。
- 验证状态: **CONFIRMED**（映射对调 + compute_window_metrics 用生产真实 status 计数双证；纯观测不反哺决策亦亲验）
- 修复建议: 与 [1-01] 合并——三文件（threshold/significance/post_release）统一引用一份**经生产语义核实**的权威 `(gate_key ↔ final_review_status)` 映射；post_release 侧尤其需修正（它是唯一读生产真实 status 的路径，对调直接反映到 admin 面板）。注意 pressure_risk 生产走软闸 revision 不产 block status，其"命中率"本身在生产侧无对应终态——该 gate 的 post_release delta 是否有意义需产品重新定义（可能应改用 revision_count/revision_applied 口径）。
- 状态: Open

**簇2 正向 HOLDS（主控亲验）**：
- **replay shadow 隔离 HOLDS**：模块头（`replay.rs:3-19`）明示不调 `run_user_operation_gateway`/`handle_managed_message`/`outbox` enqueue/`mcp::*`/不写 conversation_messages；prompt 候选走 `agent::prompt_shadow::shadow_replay_prompt_one`（:226，纯演练 decide_reply+review_decision）；有静态扫描单测禁 `outbox/mcp::` 引用兜底（:874）。
- **KE-02 两侧同口径消除虚假 send_delta**：`original_final_review_status` 用 5 闸重推（`replay.rs:296-301`）而非源 run 真实终态——避免非-5gate 因素（blocked_by_budget 等）让 original 算"失败"、new 算"成功"凭空 +send_delta 虚假翻越 min_send_success_delta 门（回归测试 :707/:721/:726）。
- **significance 安全回归门自洽**：`grade_safety_regression`（significance.rs:98-119）默认 `max_safety_regression_rate=0.0` 零容忍（:76-77）——任一条风险消息从 blocked 翻 sent 即否决放松提案；与 replay 合成 status 同口径（见 [1-01]）。
- **budget 预检**：replay `eval_all` 预算触顶时未启动的 replay 写 `failed`+`failure_reason="evolution_budget_exceeded"`（:122-128），不炸 tick（mod.rs:189-194 拦 BudgetExceeded）。
- **evaluate_single_gate 双键兼容**：read_gate_score 兼容 factRisk/hallucinationScore 两套历史键名（:357-362），缺分→0.0 保守。

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
