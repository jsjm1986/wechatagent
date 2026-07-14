# knowledge_wiki 子系统深度审查（第四批）Findings 台账

> **接续**：主链路53（[[project-deep-logic-audit-remediation]]）→ 第一批 agent 旁挂能力20 → 第二批 auth+routes 安全隔离面17（PR#209）→ 第三批后台 worker 群18（PR#210）。本批圈定 **knowledge_wiki 子系统**（5272 行 11 文件），前几轮只审过 `knowledge_router` 召回算法 + `routes::knowledge` HTTP 端点层，子系统内部逻辑从未深审——覆盖缺口最大。
>
> **只审不修**：本批产纯 docs 台账，绝不改任何 `.rs`。

## 审查范围（4 簇 / 5272 行 / 11 文件）

模块隔离红线（mod.rs 明示）：本子系统**禁止**引用 `crate::agent::gateway/outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。四职责：**质量**（schema 校验+锁定字段+provenance）/ **可检索**（wiki 分层+frontmatter+wikilinks+catalog）/ **可修改**（字段级 patch+编辑历史不可变+删除级联）/ **可优化**（usage/hit/blocked 回写+两阶段 sweep）。

| 簇 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **S 写入根因层**（先派等回） | `page_merge.rs`(482) + `chunk_revisions.rs`(513) | 995 | 纯函数预校验（union/lock/70%阈值/canonical hash）正确性 + `apply_chunk_revision` 七动作状态机 + **双写非事务**（revisions→chunks 中间失败）+ 级联删除 `cleanup_dangling_refs` best-effort + AI-source draft 强制 + domain schema 校验。产出**审查基准**喂簇1-3。 |
| **1 信号生成消解** | `gap_signals.rs`(2170) + `structural_proposals.rs`(208) | 2378 | 8 类 signal 生成幂等（重复 lint 是否重复建 signal）+ 两阶段 sweep 消解正确性（stage1 规则/stage2 LLM 桩）+ dedupe_key 唯一性 + confidence 重算 read-modify-write 竞态 + structural proposal「只产 pending_review 绝不 apply」红线 |
| **2 摄取源头** | `ingest_worker.rs`(489) + `block_parser.rs`(476) + `catalog_rebuild.rs`(357) | 1322 | ingest 幂等（304/failure_streak 状态机）+ not-due 不刷 last_fetched_at + AI 摄取强制 draft+needs_review 红线 + catalog rebuild job claim CAS + block 解析边界 |
| **3 反馈闭环** | `feedback_worker.rs`(189) + `lessons_learned.rs`(173) + `reviewer_stats.rs`(186) | 548 | 编排层 best-effort 吞错是否掩盖真错 + 滑窗聚合 upsert 幂等（$set 覆盖 vs 累加）+ list_workspaces 空 fallback + 跨 workspace 隔离 |

## 方法论（沿用前三批）

- **簇S 根因层先派并等回**：写入路径纯函数 + 状态机是所有 chunk 编辑的必经层，其正确性结论（union 幂等/lock 守门/双写失败语义/hash 稳定性）作基准喂簇1-3。
- **subagent 只读 + 继承 Opus（省略 model 参数）+ general-purpose（非 Explore）**：每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；凭猜测打回。报告写 `.superpowers/audit4/cluster-{S,1,2,3}-report.md`（git-ignored scratch）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列/竞态窗口/一致性破坏）。

## 严重度校准口径（防夸大，沿用第三批部署拓扑维度）

- **High**：推荐配置（单进程单机 systemd / 单租户默认）下**确定性可达**的：知识损坏（既有内容丢失/覆写）/ 红线绕过（AI 自动 verify / auto-apply 结构化写）/ 永久卡死 / 数据不一致确定性发生。
- **Medium**：需多条件叠加 / 崩溃时机 / 多租户或多副本启用才触发 / 有自愈兜底 / 仅信息层。
- **Low**：观测 / 边缘输入 / 无界增长无立即后果 / 就绪债（多租户/多副本/未接线功能）。
- **⚠️ 单进程默认不可达的多副本竞态 = 水平扩展就绪债，不夸 High**；**未接线功能（如 structural_proposals 无 apply worker）= 功能就绪债，不夸 High**（红线正确的一面）。

## 元家族聚焦

设计声称的不变量（union 幂等 / lock 守门 / 双写原子性 / AI 永不 verify / 只产提案不 apply / 幂等 dedupe / best-effort 不掩错）在实现层是否有：非原子写窗口 / 竞态 / 去重旁路 / 红线缝隙 / 吞错掩盖真错 / 新旧路径不对称。

## 边界排除（防与前后批次重叠）

- `agent/knowledge_router.rs` 召回算法（catalog→list_chunks→open_slice）——mod.rs 明示本轮零改动，仅对照不重审。
- `routes::knowledge` HTTP 端点层——Batch 2 簇5 已审，仅在调用 `apply_chunk_revision` 的姿势对照时引用。
- `import_worker.rs`（Phase G 多模态导入 job）——第三批 worker 群已审，不重审。

---

## 环节汇总（收尾时填）

- 总 findings 数：（TaskE 填）
- 严重度分布：（H/M/L，TaskE 填）
- 元家族归纳：（TaskE 填）
- 后续 P0-P3 路线：（若有 High 优先级高于前三批遗留 Medium，TaskE 填）
- 交叉去重留痕：（TaskE 填）
- 正向 HOLDS（主控亲验）：（TaskE 填）

---

## 字段模板

```
### [X-NN] 一句话标题
- 入口: —（函数/文件:line）
- 所属簇: S|1|2|3
- 类型: 非原子写|幂等|红线缝隙|竞态|去重旁路|吞错掩盖|边界|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 失效链: （谁在什么时机触发什么不一致/损坏/绕过后果；非失效类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 簇 S findings（写入根因层）

> 审查对象：`page_merge.rs`(482) + `chunk_revisions.rs`(513)。subagent 自评 4 Medium + 4 Low，**主控逐条亲验后校准为 1 Medium + 7 Low**（3 条 Medium 降级：S-01 设计已知留痕、S-02 组合不可达、S-07 下游 lint 兜底）。

### [S-08] `apply_chunk_revision` 读-改-写无乐观锁——同一 chunk 并发编辑后写者胜（lost update）
- 入口: `apply_chunk_revision`（所有 chunk 编辑必经）
- 所属簇: S
- 类型: 非原子写/丢更新
- 严重度: **Medium**（主控裁定：软锁不阻写 + replace filter 无版本前置，admin 并发协作可丢一次编辑；但有 `chunk_revisions` 完整历史可 rollback，非静默不可恢复丢失，且需并发时机叠加——不达 High）
- 现象/风险: `find_one`(:162) 读既有 → 应用 patch → `replace_one`(:300) 写回，读改写非原子且 `replace_one` filter 只有 `{_id, workspace_id}`（:301-304），无 `before_hash`/版本前置。两个并发 patch 同一 chunk，后 replace 者以自己的 v1 基线覆盖，前者变更丢失。
- 失效链: 请求A `find_one`→v1；请求B `find_one`→v1；A `replace(v1+patchA)`；B `replace(v1+patchB)`——B 的 before_hash 基于 v1 未见 patchA → 覆盖丢 A。经典 read-modify-write 竞态。`chunk_locks`（chunk_locks.rs:1/:17）是**软锁**（前端协作提示 + broadcast 事件，"失败不阻塞写入主流程"），不阻止后端 replace。
- 根因（亲验 file:line）: `chunk_revisions.rs:162` find_one 与 `:300` replace_one 之间无乐观锁；`:301-304` replace filter 无 before_hash；`chunk_locks.rs:1` 软锁语义。
- 复现设想: 两个 admin 并发 PUT 同一 chunk（wiki_edit.rs patch 路径），或 AI 知识对话 patch（chat.rs:1786）与 admin PUT 并发。
- 验证状态: PLAUSIBLE（能构造触发序列，未在生产观测；admin 并发编辑同一 chunk 低频）
- 修复建议: `replace_one` filter 加 `before_hash` 前置（乐观锁 CAS），`modified_count==0` → 409 冲突让前端重取重试；或复用软锁升级为写前校验 lock owner。
- 状态: Open

### [S-01] `apply_chunk_revision` 双写非事务——revision 写成功而 chunk 替换失败留孤儿历史行
- 入口: `apply_chunk_revision`
- 所属簇: S
- 类型: 非原子跨集合写
- 严重度: **Low**（主控裁定：模块 doc:15-17 明说此设计"便于人工查 last_revision != current_state"——**已知设计**；revision 是不可变历史行非当前态，读取路径读 chunks 当前态不受影响，孤儿 revision 仅审计噪音，无正确性/客户可见后果。subagent 自评 Medium 夸大，降 Low）
- 现象/风险: 先 `insert` revision(:294) 后 `replace_one` chunks(:300)，非事务。若 :300 失败（网络/反序列化校验），revision 已落库，after_hash 指向从未写入 chunks 的状态。
- 失效链: :294 insert revision 成功 → :300 replace 失败 → revision.after_hash 指向未落盘态；下次读 last_revision 与 current chunk 对不上（但 current chunk 仍是改前的合法态，读取无损）。
- 根因（亲验 file:line）: `chunk_revisions.rs:294`（insert revision）在 `:296-308`（replace chunk）之前，无事务/补偿。
- 复现设想: 构造 replace_one 失败（merged_typed 反序列化触发 schema 校验失败或 mongo 瞬时断连）。
- 验证状态: PLAUSIBLE
- 修复建议: 无需修（设计已知且无正确性后果）。可选：给 revision 加 `applied:bool` 标志，replace 成功后二次 update 置 true，便于对账区分"孤儿历史"。
- 状态: Open

### [S-02] AI-source draft 降级与 op=Archive/Restore 的 status 覆盖顺序——组合不可达（防御性）
- 入口: `apply_chunk_revision` status 覆盖块
- 所属簇: S
- 类型: 状态机绕过（不可达）
- 严重度: **Low**（主控裁定：**失效链要求 `op=Restore/Archive + source=Ai` 同时出现，全代码库无此组合构造点**——唯一 `source=Ai` 调用点 chat.rs:1786 硬编码 `op=Patch`；所有 Restore(:177)/Archive(:134,:1050) 硬编码 `source=Human`。subagent 只读函数内部代码顺序未核调用点约束，夸大成 Medium，降 Low 防御性建议）
- 现象/风险: 函数内 `source=ai → status=draft`(:223-225) 在前、`op=Restore → status=active`(:232-233) 在后，同一 merged 后写者胜。**若** AI 能调 Restore，则可把 chunk 从 draft 直接恢复 active 绕过 needs_review。
- 失效链: 理论：apply_chunk_revision(op=Restore, source=Ai) → status 先 draft 后被 active 覆盖 → source=ai + active 组合。**实际不可达**：无任何调用点构造此组合（亲验全部 RevisionRequest 构造点，Restore/Archive 恒 source=Human）。
- 根因（亲验 file:line）: `chunk_revisions.rs:222-236` status 覆盖顺序（AI 降级在前 op 覆盖在后）；调用点约束见 `chat.rs:1785-1786`（唯一 AI 源恒 Patch）、`wiki_edit.rs:134-135/:177-178`（Restore/Archive 恒 Human）。
- 复现设想: 需新增一个 `op=Restore + source=Ai` 调用点才可达——当前不存在。
- 验证状态: PLAUSIBLE（代码顺序客观存在，但触发组合不可达）
- 修复建议: 防御性：op=Archive/Restore 分支加 `debug_assert!(!matches!(source, Ai))`，或 AI source 时对 Restore 仍强制 draft，防未来新增 AI-restore 调用点误破红线。
- 状态: Open

### [S-07] `cleanup_dangling_refs` 逐 chunk 调 apply_chunk_revision 吞错——批量清理部分失败静默
- 入口: `cleanup_dangling_refs`（archive 级联）
- 所属簇: S
- 类型: best-effort 吞错/一致性
- 严重度: **Low**（主控裁定：best-effort 设计 + `gap_signals` 的 broken_link/missing_chunk lint 下游兜底可发现残留。subagent 自评 Medium，因下游有兜底降 Low）
- 现象/风险: cleanup 对每个含被 archive chunk 引用的 chunk 调 apply_chunk_revision(:441)，失败仅 warn(:443) 继续。批量场景部分清理失败 → 残留 related_chunks 指向已 archived chunk（悬空引用）。
- 失效链: :406-451 遍历所有含 related_chunks 的 chunk；:441 某条 apply 失败 → :443 warn 继续 → 该条引用残留。下游靠 gap_signals `missing_chunk` lint 补发信号（指向 archived chunk 的引用）。
- 根因（亲验 file:line）: `chunk_revisions.rs:441-450` match Err 仅 warn 不冒泡不计数。
- 复现设想: 10 个 chunk 引用被 archive 的 chunk，第 5 个 apply 失败 → 前 4 清、含失败的第 5 及之后残留。
- 验证状态: PLAUSIBLE
- 修复建议: cleanup 返回 `(cleaned, failed)` 计数，archive handler 把 failed>0 回填响应，让运营知晓需重跑；下游 gap_signals lint 已能发现残留（兜底充分）。
- 状态: Open

### [S-03] `text_payload_len` 取 body/summary 较长者——patch 只改 summary 时分母失真绕过 70% 阈值
- 入口: `apply_chunk_revision` body 截断守门
- 所属簇: S
- 类型: 逻辑边界
- 严重度: **Low**（单字段保护弱化，无级联后果；body 主体保护正常）
- 现象/风险: `text_payload_len`(:348-360) 返回 body/summary 较长者。patch 只改 summary 且 body 更长时，old_len/new_len 都取 body 长度，summary 大幅截断也不触发拒收。
- 失效链: :207-219 touched_text_field 含 summary → old_len=max(body,summary)。patch 把 summary 100→10 但 body=500 → new_len=max(500,10)=500 > 350 → 不拒收。
- 根因（亲验 file:line）: `chunk_revisions.rs:348-360` text_payload_len 用 max 而非按 patch 实际触及字段分别判定。
- 复现设想: patch={summary:"短"} 对 body 长的 chunk → summary 截断保护失效。
- 验证状态: PLAUSIBLE
- 修复建议: 按 patch 实际触及字段（body/summary/answer 各自）分别算长度阈值，而非统一 max。
- 状态: Open

### [S-04] `related_chunks` 不走 union 而整体覆盖——设计正确（标注供簇1参考）
- 入口: `union_array_fields` / `apply_chunk_revision`
- 所属簇: S
- 类型: 文档/实现一致（非缺陷）
- 严重度: **Low**（设计正确，仅标注）
- 现象/风险: related_chunks 是结构数组（RelatedRef），不在 DEFAULT_UNION_ARRAY_KEYS(page_merge.rs:51-59)，patch 携带时整体覆盖而非 union——与其它字符串数组字段行为不一致，但符合注释说明（结构数组需按 chunk_id 去重，简单 string union 不适用）。
- 失效链: 非失效——设计如此（page_merge.rs:50 注释明说）。
- 根因（亲验 file:line）: `page_merge.rs:51-59` DEFAULT_UNION_ARRAY_KEYS 不含 related_chunks；`chunk_revisions.rs:430-431` 注释说明。
- 复现设想: —
- 验证状态: PLAUSIBLE
- 修复建议: 无需修（设计正确）。标注供簇1 broken_link/missing_chunk 审查时理解 related_chunks 的写入语义。
- 状态: Open

### [S-05] `compute_chunk_hash` volatile 集含 `"id"`——防御冗余（非缺陷）
- 入口: `compute_chunk_hash`
- 所属簇: S
- 类型: 边界（防御冗余）
- 严重度: **Low**（无害；`id` 字段恒 `#[serde(rename="_id")]`，序列化产物无裸 `id` 键，剔除是冗余非误剔）
- 现象/风险: VOLATILE_FIELDS(page_merge.rs:247-254) 同列 `"id"` 与 `"_id"`。若 chunk 有业务 `id` 字段会被误剔 → 内容变 hash 不变。
- 失效链: 亲验 OperationKnowledgeChunk(models.rs:1583) 唯一 `id` 字段是 `#[serde(rename="_id")]`(models.rs:1584-1585)——序列化恒写 `_id`，无裸 `id` 键落库。故剔除 `"id"` 永远命不中真实字段，是防御冗余。
- 根因（亲验 file:line）: `page_merge.rs:254` VOLATILE_FIELDS 含 "id"；`models.rs:1584-1585` id rename _id。
- 复现设想: 需 chunk 结构新增独立业务 `id` 字段才可达——当前不存在。
- 验证状态: PLAUSIBLE（当前不可达）
- 修复建议: 无需修（防御冗余无害）。若未来 chunk 新增业务 id 字段需重审此剔除。
- 状态: Open

### [S-06] domain schema alias 改写后 unchanged 误判——边缘几乎不可达
- 入口: `apply_chunk_revision` domain schema 校验块
- 所属簇: S
- 类型: 逻辑边界
- 严重度: **Low**（边缘到几乎不可达）
- 现象/风险: `if !unchanged`(:296) 才 replace + enqueue。domain schema alias 改写(:264-274) 在 after_hash 计算(:276) 之前，正常 alias 改写会计入 hash；仅当改写字段恰落在 VOLATILE_FIELDS 才 unchanged 误判跳过写入。
- 失效链: :264-274 enforce domain attributes → :276 after_hash → :277 unchanged。正常 alias 改写变 hash；仅改写字段在 volatile 集才误判（domain_attributes 不在 volatile，几乎不可能）。
- 根因（亲验 file:line）: `chunk_revisions.rs:276-277` + `:264-274`。
- 复现设想: domain_attributes 改写后 hash 不变（需改写字段恰在 volatile，几乎不可达）。
- 验证状态: PLAUSIBLE（几乎不可达）
- 修复建议: 无需修。标注供参考。
- 状态: Open

## 簇 1 findings（信号生成消解）

（主控亲验后填入）

## 簇 2 findings（摄取源头）

（主控亲验后填入）

## 簇 3 findings（反馈闭环）

> 审查对象：`feedback_worker.rs`(189) + `lessons_learned.rs`(173) + `reviewer_stats.rs`(186)。主控逐条亲验：`lessons_learned` filter 字段回溯真实写点、`derive_lifecycle_from_status` 派生规则、三表幂等姿势、观测-only 红线。**校准结论：2 Medium + 6 Low**（subagent 自评 1H/2M/4L+1标注，[3-01] High→Medium：observation-only 学习闭环恒空、[3-06] 已证不反哺决策、无客户面/正确性/资源损害，较 Batch3 High[1-01] 轻一档；[3-03] Medium→Low：单租户默认 workspace 恒在列表不可达，仅多租户+活跃无chunk边缘触发的面板缺行=就绪债）。

### [3-01] `lessons_learned` success/failure 两支 filter 引用幽灵字段路径 `review.reaction_analysis.user_polarity`——三重落空致恒命中 0 条
- 入口 worker: feedback_worker → `aggregate_lessons_for_workspace`
- 所属簇: 3
- 类型: 死过滤（filter 字段全链路从未被写入）
- 严重度: **Medium**（主控裁定：CONFIRMED 且单进程确定性可达——`lessons_learned` success/failure 两类模式 100% 不产出，admin 面板 + peer_case 候选池永久空白。但降 subagent 自评 High 一档：这是 **observation-only 学习闭环**（lessons→admin review→才晋升 peer_case chunk），[3-06] 已亲验其**不反哺 agent 决策链**，恒空无客户可见后果、无数据污染、无资源浪费。对比 Batch3 High[1-01]（丢任务+3×LLM 重复消耗+画像旧值覆写=确定性正确性/资源损害）明显轻一档，故 Medium。）
- 现象/风险: success（正反应）+ failure（reviewer 误判负反应）两支各在 `agent_run_logs` 上按 `review.reaction_analysis.user_polarity ∈ {positive,constructive}` / `=negative` 过滤，该字段路径三重不成立 → count 恒 0。
- 失效链（三重落空，逐环主控亲验）:
  1. **集合选错**：filter 查 `agent_run_logs`（lessons_learned.rs:99 `.collection::<Document>("agent_run_logs")`），而 `reaction_analysis` 只写进**另一集合** `decision_reviews`（reaction.rs:190-194 `.decision_reviews().update_one`，主控亲验 `$set` 含 `"reaction_analysis": reaction_analysis`）。
  2. **子结构缺失**：`agent_run_logs.review` = `DecisionReviewResult` 序列化（models.rs:1186 起，主控亲验全字段），**无 `reaction_analysis` 子文档**。
  3. **字段名从不存在**：`user_polarity`/`userPolarity` 全仓**零写点**（主控 grep 写侧空结果）——reaction 子文档真实极性键是 `outcome_status`（reaction.rs:180 `"outcome_status": outcome`）。
- 根因（亲验 file:line）: lessons_learned.rs:44 + :61（幽灵字段 filter）；reaction.rs:190-194（reaction_analysis 写 decision_reviews 非 run_logs）；models.rs:1186（DecisionReviewResult 无该子文档）。
- 复现设想: 任意 workspace 跑满 14d 真实对话（含 approved+正反应），feedback_worker 一轮后 `db.lessons_learned.find({pattern_kind:{$in:["success","reviewer_misjudge_negative"]}})` 恒空；日志 `lessons_learned aggregate done` 因 count 全 0 永不打印（lessons_learned.rs:101 门槛 `>0`）。
- 验证状态: CONFIRMED
- 修复建议: filter 改到正确集合+真实字段：聚合源改 `decision_reviews`，极性用真实存在的 `reaction_analysis.outcomeStatus`（或 `reviewer_misjudge_signal`/`outcome_status`）；移除全链路不存在的 `user_polarity` 键。单测须覆盖"filter 对真实文档形状能命中"，非仅测 id 拼接。
- 状态: Open

### [3-02] `lessons_learned` blocked 模式 filter 自相矛盾：`lifecycle="completed"` 与 `final_review_status="blocked_by_safety_guard"` 由同一派生函数保证互斥——恒命中 0 条
- 入口 worker: feedback_worker → `aggregate_lessons_for_workspace`
- 所属簇: 3
- 类型: 死过滤（字面量与状态机派生规则冲突）
- 严重度: **Medium**（主控裁定：CONFIRMED、单进程确定性可达；与 [3-01] 同族，两条联合使 `lessons_learned` 整模块 inert。单独看是三类模式的第 3 类失效、observation-only、无客户面后果，故 Medium 不夸 High。）
- 现象/风险: blocked 模式要求 run 同时 `lifecycle="completed"` 且 `final_review_status="blocked_by_safety_guard"`，但任何 `blocked_by_safety_guard` 派生的 lifecycle 恒为 `failed_after_decision`，交集为空。
- 失效链: blocked filter（lessons_learned.rs:73-74）`lifecycle="completed"` + `final_review_status="blocked_by_safety_guard"`；`derive_lifecycle_from_status`（run_envelope.rs:257 `completed` 分支只含 `sent|no_reply|approved|allowed|outbox_enqueued`，:267 `_ => LIFECYCLE_FAILED_AFTER_DECISION`）——主控亲验文档亦明写（run_envelope.rs:246-248 `blocked_by_safety_guard → failed_after_decision`）。两条件恒无交集。
- 根因（亲验 file:line）: lessons_learned.rs:73-74；run_envelope.rs:257/267（派生规则保证互斥）。
- 复现设想: 造被安全门拦截的 run，feedback_worker 一轮后 `db.lessons_learned.find({pattern_kind:"blocked_by_safety_guard"})` 空。
- 验证状态: CONFIRMED
- 修复建议: blocked 模式去掉 `lifecycle="completed"` 约束（或改 `failed_after_decision`），仅以 `final_review_status="blocked_by_safety_guard"` + 窗口定位；与 [3-01] 一并修使三类模式全可命中。
- 状态: Open

### [3-03] `list_workspaces` 以「有 chunk 的 workspace」驱动——有对话流量但无知识 chunk 的 workspace 的 reviewer_stats/lessons/deal_attribution 永不聚合
- 入口 worker: feedback_worker → `run_one_round` / `list_workspaces`
- 所属簇: 3
- 类型: 就绪债（工作集来源与聚合对象错配）
- 严重度: **Low**（主控裁定：降 subagent 自评 Medium——单租户默认部署 `default_workspace_id` 恒在列表中（有 chunk 或 :183-185 空则 fallback），缺口**单租户不可达**；仅多租户 + "活跃有 decision_reviews 但零 chunk 的 workspace" 边缘触发，且后果是 observation-only admin 面板对该 ws 静默缺行，无数据污染/客户面损害。属多租户就绪债。）
- 现象/风险: `run_one_round` 遍历的 workspace 来自 `operation_knowledge_chunks.distinct("workspace_id")`，而 reviewer_stats（源 `decision_reviews`）/lessons（源 `agent_run_logs`）的聚合对象与 chunk 存在与否无因果——有流量无 chunk 的 ws 不进列表 → 统计永不刷新。
- 失效链: feedback_worker.rs:41 `list_workspaces`；:176-178 `.operation_knowledge_chunks().distinct(...)`；:183-185 仅整体为空才 fallback default，非空不并入无 chunk 的活跃 ws；reviewer_stats.rs:62 聚合源 `decision_reviews()` 与 chunk 无关。
- 根因（亲验 file:line）: feedback_worker.rs:176-178 + :183-188。
- 复现设想: 多租户下新 workspace A 无 chunk 但有 decision_reviews，同时 ws B 有 chunk → 列表=[B]，`db.reviewer_stats.find({workspace_id:"A"})` 恒空。
- 验证状态: CONFIRMED（缺口存在，单租户默认不可达）
- 修复建议: 工作集由「全部业务活跃面」并集驱动（并入 decision_reviews/run_logs 的 distinct workspace_id 或权威 workspace 目录）。注意 [3-06/关联 3-08] >100 分页边界。多租户就绪债。
- 状态: Open

### [3-04] `reviewer_stats` 误判分子未在查询层显式 scope `approved:true`+窗口——依赖 reaction.rs 单一写点隐式不变量
- 入口 worker: feedback_worker → `aggregate_reviewer_stats_for_workspace`
- 所属簇: 3
- 类型: 就绪债（隐式不变量非显式约束）
- 严重度: **Low**（当前不变量成立、numerator ⊆ denominator、misjudge_rate ≤ 1；仅"查询未自我防御、依赖上游写点唯一性"的健壮性债，无当前正确性后果。）
- 现象/风险: `approved_but_user_negative` 计数（reviewer_stats.rs:88-97）只按 `reviewer_misjudge_signal="approved_but_user_negative"`+窗口，不显式要求 `approved:true`/`outcome_status` 非空，靠"该信号仅在 approved==true 时写"的隐式不变量保证分子 ⊆ 分母。
- 失效链/不变量（当前成立）: 信号唯一写点 reaction.rs:173-177（`compute_reviewer_misjudge_signal_with_polarity` 以 `approved &&` 守卫）+ :185-186 仅 Some 才 insert；与 `outcome_status` 同一 `$set`；gateway.rs 初始写 None 不产假分子。
- 根因（亲验 file:line）: reviewer_stats.rs:88-97；reaction.rs:173-177。
- 验证状态: CONFIRMED（缺口为隐式耦合，不变量当前 HOLDS）
- 修复建议: 分子 filter 增补 `"approved": true` + `"outcome_status": {$exists:true,$ne:null}` + 窗口，使查询自洽。低优先。
- 状态: Open

### [3-05] 三张滚动统计（reviewer_stats/deal_attribution_stats/lessons_learned）幂等姿势正确——设计正确标注
- 所属簇: 3 ｜ 类型: 设计正确(标注) ｜ 严重度: **Low**（标注，无缺陷）
- 结论: 三者均 `$set` 覆盖瞬时值（非 `$inc` 累加）+ stat_id/lesson_id 定位 + upsert，重复跑同窗口结果一致无累加漂移，为 0 也写稳定锚点。reviewer_stats.rs:107-132（唯一索引 indexes.rs:1659-1673）；deal_attribution feedback_worker.rs:148-165（indexes.rs:1680-1692）；lessons_learned.rs:125-149（因 [3-01/02] count 恒 0 实际不落文档，幂等姿势本身正确）。
- 验证状态: CONFIRMED ｜ 状态: Open（仅标注）

### [3-06] reviewer/lessons/deal_attribution 均纯观测 upsert 不反哺 agent 决策链——red line HOLDS（设计正确标注）
- 所属簇: 3 ｜ 类型: 设计正确(标注) ｜ 严重度: **Low**（标注）
- 结论: 三张统计表消费端全在 `src/routes/`（admin 面板/observability），`src/agent/**` 无一读取（主控亲验 grep）——符合"统计只观测不进决策"红线（同 tag_trust "只写不进决策"）。reviewer_stats 仅 observability.rs:258；lessons_learned 仅 routes/lessons_learned.rs+ask_human_inbox.rs+observability.rs 读，晋升 chunk 需 admin review；deal_attribution 仅 observability.rs:280。
- 验证状态: CONFIRMED ｜ 状态: Open（仅标注）

### [3-07] feedback_worker best-effort 逐步吞错无跨步累积漂移——设计正确标注（含 deal_attribution 依赖 refresh 报告边缘）
- 所属簇: 3 ｜ 类型: 设计正确(标注)+吞错累积(边缘) ｜ 严重度: **Low**
- 结论: 5 步（refresh/lint/sweep/lessons/reviewer）各自独立从 DB 读当前态，无 in-memory 中间态跨步传递（feedback_worker.rs:43/69/83/99/118 各 `match...Err=>warn` 继续）；唯一跨步耦合 deal_attribution 依赖 refresh 报告（:61），refresh 失败该轮不写（停上轮值，稳定锚点），无系统性偏差。某步反复失败仅表现该指标停更，不污染其它 ws/指标。
- 验证状态: CONFIRMED ｜ 状态: Open（标注；仅在 [3-01/02/03] 修复后配套"指标长期停 0/缺行"告警属加法）

### [3-08] `list_workspaces` 全量 distinct 拉内存，>100 workspace 无界——就绪债标注
- 所属簇: 3 ｜ 类型: 就绪债 ｜ 严重度: **Low**（注释已声明 <100 假设；单/少 workspace 生产无害；大规模多租户才成问题）
- 现象: feedback_worker.rs:172-182 一次性 `distinct`+`collect` 到 `Vec<String>`，无分页/上限。
- 根因（亲验 file:line）: feedback_worker.rs:173-182。
- 验证状态: CONFIRMED ｜ 修复建议: 规模上量改游标分批或权威 workspace 目录，与 [3-03] 工作集来源一并考虑。 ｜ 状态: Open
