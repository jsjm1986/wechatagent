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

- **总 findings 数：32**（簇S 8 + 簇1 9 + 簇2 7 + 簇3 8）
- **严重度分布：0 High / 5 Medium / 27 Low**（subagent 自评合计 3 High/10 Medium；主控亲验后 3 个 High 全降 Medium、多个 Medium 降 Low——见下"主控校准降级"）。
- **5 Medium**：
  - **[3-01]** `lessons_learned` success/failure 两支 filter 引用幽灵字段 `review.reaction_analysis.user_polarity`（三重落空：查错集合 agent_run_logs 非 decision_reviews / DecisionReviewResult 无 reaction_analysis 子文档 / 字段名从不存在真实是 outcomeStatus）→ 恒命中 0，学习闭环 inert。
  - **[3-02]** `lessons_learned` blocked 支 filter `lifecycle="completed" AND final_review_status="blocked_by_safety_guard"` 互斥（`derive_lifecycle_from_status` 保证 blocked_by_safety_guard 恒派生 failed_after_decision）→ 恒 0。与 [3-01] 联合使 lessons_learned 三类模式全 inert。
  - **[2-01]** ingest 落库入口 `content_hash:None` + 无条件 insert，幂等完全押 HTTP 条件 GET(ETag) 单点 → 不返 ETag 的源每轮全量重复落库、无界增长（**需 `INGEST_WORKER_ENABLED` 显式开启，默认关**）。
  - **[1-01]** `knowledge_gap_signals` 无 (workspace,chunk,kind) 业务去重唯一索引，structural lint 走 find-then-insert 应用层查重 → 并发/多副本双插同信号（单进程串行 worker 默认不触发）。
  - **[S-08]** `apply_chunk_revision` 读-改-写非原子（find_one → replace_one 无乐观锁 CAS），软锁不阻写 → 两个并发 patch 同一 chunk lost update（admin 协作场景，有 chunk_revisions 历史可 rollback）。
- **元家族归纳**：本批主线元家族 = **「设计声称的闭环/去重不变量，实现层 filter 字段与真实写点数据形状/状态机派生规则不对齐，或缺唯一索引兜底，导致恒空/可重复」**。三个维度：①**幽灵字段/自相矛盾 filter**（[3-01]/[3-02]，凭设计意图写 filter 未亲验写侧真实字段/状态机派生规则，致聚合恒空）②**幂等靠应用层 find-then-insert 而非唯一索引/内容指纹兜底**（[1-01] 信号无去重唯一索引、[2-01] ingest 落库无 content_hash 去重——外部协议/串行假设一旦不成立即失效，与主动侧 outbox sha256 幂等键、silence partial unique 的强幂等姿势层间不对称，延续 Batch3 元家族到 knowledge 侧）③**读改写/双写非原子窗口**（[S-08] chunk patch 无乐观锁、[S-01] revisions→chunks 双写非事务、[1-02] usage refresh 与编辑并发——但 [1-02] 亲验 $set 限定字段无 body 覆盖降 Low）。
- **后续 P0-P3 路线**（本批 0 High，优先级**低于**前三批遗留：Batch3 [1-01] High initial_profile 终态仍是最高 P0）：
  - **P1（Medium）**：[3-01]+[3-02] 一并修（lessons_learned filter 改到正确集合 decision_reviews + 真实字段 outcomeStatus + blocked 支去掉 lifecycle=completed 约束），使学习闭环真正可命中——这是"整模块 inert"，修复价值最高的 Medium。
  - **P2（Medium）**：[2-01]（ingest 落库加 content_hash 去重，即便 worker 默认关也应治本，防开启即灌爆）+ [1-01]（gap_signals 加 partial unique 索引）+ [S-08]（chunk patch replace_one filter 加 before_hash 乐观锁 CAS）。
  - **P3（Low 批量）**：27 Low 就绪债/边界/标注批量收口（TTL/阈值可配/contradiction normalize/orphan age 下限等）。
- **交叉去重留痕**：
  - **[S-01]（双写非原子）与 [S-08]（读改写 lost update）** 同属"非原子写窗口"元家族但根因不同（S-01=跨集合双写无事务、S-08=同集合读改写无乐观锁），各自独立保留。
  - **[3-01] 与 [3-02]** 同致 lessons_learned inert 但根因正交（字段路径落空 vs 状态机字面量互斥），独立保留，P1 一并修。
  - **[1-01]（信号无唯一索引）与 [2-01]（ingest 无内容去重）** 同属"幂等靠应用层非索引/指纹兜底"元家族，跨簇呼应但审查对象不同（信号生成 vs 摄取落库），各自保留、元家族段归纳。
  - **[S-04] related_chunks 不 union** 与 [1-08] broken_link sweep 语义一致标注（都涉 related_chunks 生命周期），S-04 已明确"设计正确"，不重复计为缺陷。
- **正向 HOLDS（主控亲验）**：①**"AI 永不自动 verify" 红线跨全摄取入口 HOLDS**——chunk_revisions AI-source 强制 draft+needs_review（:223-225）、ingest block/fallback 路径无条件压 draft+needs_review（import.rs:1192/1242/1248）、结构化写只产 pending_review（structural_proposals 序列化层锁死无 apply/commit/delete 字段）②**信号/统计"只观测不进决策" 红线 HOLDS**——Grep `src/agent/` 全目录零读取 knowledge_gap_signals/structural_proposals/lessons_learned/reviewer_stats/deal_attribution_stats，消费端全在 routes/ admin 面板③**page_merge 纯函数正确**——union 幂等/保序、锁定字段双层防护（patch 硬拒 + enforce 末次覆盖）、70% 截断阈值、canonical hash 字段序无关（15 单测覆盖 KB-09/KB-11 回归哨兵）④**catalog job claim CAS HOLDS**——find_one_and_update `{status:queued}→processing` 原子 + signal_id/job_id 唯一索引无 TOCTOU⑤**normalize_ref_key 无 substring 误伤**（openai≠ai）⑥**三张滚动统计幂等姿势正确**（$set 快照覆盖非 $inc + 唯一 stat_id/lesson_id upsert）⑦**S-02 AI restore 绕 needs_review 不可达**（唯一 source=Ai 调用点 chat.rs:1786 恒 op=Patch，所有 restore/archive 硬编码 source=Human）⑧**not-due 不写任何 DB HOLDS**（Skipped 分支不刷 last_fetched_at，防节流基准前推）。
- **主控校准降级留痕**（防夸大红线）：subagent 自评 3 High（[3-01]/[2-01] + 簇S subagent 无 High，实为簇3+簇2 各 1 + 另一说法合计），主控亲验后**全部降 Medium**——判据：三者均 **observation-only/功能门后确定性可达但无客户面可见后果、无数据污染、不反哺决策、有兜底**，较 Batch3 唯一 High [1-01]（initial_profile 丢任务 + 3×LLM 浪费 + 画像旧值覆写=资源浪费+数据正确性损害+功能生命周期确定性破坏）轻一档。[3-01]=学习闭环 inert 但经 admin review 才晋升 chunk 无正确性损害；[2-01]=INGEST_WORKER_ENABLED 默认关需显式开启 + draft 不进 verified 池不误发；[S-08]=admin 协作低频 + chunk_revisions 历史可 rollback。多个 Medium 亦降 Low：[2-02]/[2-03]（同门后/实时聚合兜底）、[1-02]（$set 限定字段无 body 覆盖）、[3-03]（单租户 default_workspace 恒在列表不可达）。

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

> 审查对象：`gap_signals.rs`(2170) + `structural_proposals.rs`(208)。主控逐条亲验：`knowledge_gap_signals` 索引定义（`gap_signals_signal_id_unique` 是 UUID `signal_id` 唯一，非业务去重键）、find-then-insert 去重模式（:610 全量 find pending → :621 应用层 `dedup_key` 匹配 → :663-682 insert）、refresh 写回姿势（`update_one({_id},{$set})` 只限定 `usage_stats`/`dynamic_confidence`/`updated_at`，非整文档 replace）、信号只观测不进决策红线。**校准结论：1 Medium + 8 Low**（subagent 自评 0H/3M/6L；[1-02] Medium→Low：写回是 `$set` 限定字段、与 chunk 编辑 body 不重叠、无 lost update；[1-01] 保 Medium；[1-03] 保 Low）。

### [1-01] `knowledge_gap_signals` 无业务去重唯一索引，信号生成走 find-then-insert——并发/交错窗口可重复插同一 (workspace,kind,dedup_key) 信号
- 入口 worker: `feedback_worker` → `run_structural_lint` / `persist_signals`
- 所属簇: 1
- 类型: 幂等/去重旁路
- 严重度: Medium（主控裁定：无业务去重唯一索引=确定性去重缺失，靠应用层 find-then-insert；单进程默认 `feedback_worker` 单实例串行 `run_one_round` 顺序 await，并发窗口默认不存在→当前不可达，但多副本/未来 admin 手动触发 lint 端点即真实双插。属"幂等靠应用层查重而非唯一索引兜底"的确定性缺失，故 Medium 非 Low）
- 现象/风险: 八类 structural 信号 + recall_trace 在线信号均走"全量 find 同 kind pending → 应用层按 `dedup_key` 精确匹配 → 命中则 `$set` 合并 affected_chunk_ids/search_queries、未命中则 insert 新信号"。`knowledge_gap_signals` 无 `(workspace_id, kind, dedup_key/title)` 唯一索引兜底。
- 失效链: 两轮 lint 并发（或 lint 与在线 `persist_signal` 交错）→ 两次 find 都返空（同一候选尚未落库）→ 双 insert_one 成功 → 同一 (chunk,kind) 两条 pending。默认单实例串行下不发生。
- 根因（亲验 file:line）: `gap_signals.rs:610-620` 全量 find pending（`{workspace_id,status:"pending",kind}`）；`:621-623` 应用层 `signal_dedup_key` 匹配；`:663-682` 未命中 insert_one。`db/indexes.rs:1396-1445` `knowledge_gap_signals` 仅 4 个索引（`gap_signals_status_kind_idx`/`created_at_idx`/`signal_id_unique`/`kind_status_created_idx`）——`signal_id_unique` 键是每条新生成的 `sig_<uuid>`（:665），**非** (workspace,kind,dedup_key) 业务去重键。
- 复现设想: 并发跑两轮 feedback_worker（多副本或手动触发 lint 撞车）→ 同一 orphan chunk 两条 pending 信号。
- 验证状态: CONFIRMED（无业务唯一索引 + find-then-insert 双向亲验）
- 修复建议: 加 `(workspace_id, kind, dedup_key)` partial unique 索引（partial: status=pending），或改 find-then-insert 为原子 upsert。
- 状态: Open

### [1-02] `refresh_usage_stats_and_confidence` read-modify-write 非原子——但写回是 $set 限定字段，无 lost update
- 入口 worker: `feedback_worker` → `refresh_usage_stats_and_confidence`
- 所属簇: 1
- 类型: 时间窗竞态/非原子写
- 严重度: Low（主控裁定：subagent 标 Medium 并自注"若 $set 限定则降 Low"。亲验写回是 `update_one({_id},{$set:&set})`、set 只含 `usage_stats`/`dynamic_confidence`/`updated_at`——与 `apply_chunk_revision` 编辑的 body/tags 字段不重叠，并发不产生 lost update。read-modify-write 窗口只影响 confidence 自身瞬时值（下轮全窗口重算即自愈），无跨字段覆盖后果，故 Low）
- 现象/风险: 对每个 active chunk 读 usage_stats → 算 30d hit/blocked → 重算 dynamic_confidence → 写回，read-modify-write 非原子。
- 失效链: refresh 读 chunk v1 → 并发 `apply_chunk_revision` 改 body(v1→v2) → refresh 写回。因写回 `$set` 只限定 usage/confidence 字段，v2 的 body 不被覆盖；confidence 自身若被并发 refresh 覆盖，下轮全窗口重算自愈。
- 根因（亲验 file:line）: `gap_signals.rs:1035-1055` 写回 `update_one(doc!{"_id":oid}, doc!{"$set":&set})`，`set` 只含 `usage_stats`(子对象)/`dynamic_confidence`/`updated_at`。
- 复现设想: feedback_worker refresh 时并发 admin 编辑同一 chunk——body 不受影响，仅 confidence 瞬时值可能被下轮重算覆盖（自愈）。
- 验证状态: CONFIRMED（写回 $set 限定字段亲验，无 lost update）
- 修复建议: 无需修（当前姿势正确）；仅标注 read-modify-write 窗口的 confidence 瞬时值靠下轮重算收敛。
- 状态: Open

### [1-03] `knowledge_gap_signals` 无 TTL、resolved 不物理删——无界累积（就绪债）
- 入口 worker: `feedback_worker` → `sweep_stale_signals`
- 所属簇: 1
- 类型: 无界增长/就绪债
- 严重度: Low（主控裁定：resolved/auto_resolved 只标 status 不物理删 + 无 TTL 索引，长期累积；单进程单/少 workspace 生产无立即后果，属就绪债）
- 现象/风险: 信号 resolved 后只标 `status=resolved/auto_resolved`，不物理删除；集合无 TTL 索引。长期 pending+resolved 无界累积。
- 根因（亲验 file:line）: `gap_signals.rs` sweep 只 update status；`db/indexes.rs:1396-1445` `knowledge_gap_signals` 四索引均无 TTL（`expireAfterSeconds`）。
- 复现设想: 长期运行，resolved 信号越积越多。
- 验证状态: CONFIRMED
- 修复建议: resolved 信号加 TTL（如 30d 后删）或定期归档。
- 状态: Open

### [1-04] contradiction 信号 body 首段 sha256 纯字节比对——格式化差异（空格/标点/大小写）产生假阳性
- 入口 worker: `feedback_worker` → `run_structural_lint`（contradiction 类）
- 所属簇: 1
- 类型: 逻辑边界
- 严重度: Low
- 现象/风险: contradiction 检测对同 `normalize_title` 的多 chunk 比对 body 首段 sha256，纯字节比对；格式化差异（多空格/标点/大小写）即算不同 → 假阳性 contradiction 信号（仅多产一条待运营处理的信号，无红线/正确性后果）。
- 根因（亲验 file:line）: `gap_signals.rs` contradiction 检测 `sha256(body 首段)`。
- 验证状态: PLAUSIBLE
- 修复建议: 比对前 normalize（去空格/标点/小写）。
- 状态: Open

### [1-05] low_confidence 阈值 `LOW_CONFIDENCE_THRESHOLD=0.3` 硬编码——不可 per-workspace 配
- 入口 worker: `feedback_worker` → `run_structural_lint`
- 所属簇: 1
- 类型: 就绪债
- 严重度: Low
- 现象/风险: 阈值硬编码常量，不可按 workspace 配置。
- 根因（亲验 file:line）: `gap_signals.rs:47` `const LOW_CONFIDENCE_THRESHOLD: f64 = 0.3;`
- 验证状态: CONFIRMED
- 修复建议: 提为 workspace 可配。
- 状态: Open

### [1-06] orphan 判定「30d 无命中」——新导入 chunk 冷启动期可能被误判
- 入口 worker: `feedback_worker` → `run_structural_lint`（orphan 类）
- 所属簇: 1
- 类型: 逻辑边界
- 严重度: Low
- 现象/风险: orphan 判定"既无入链也无 30d 命中"。新导入 chunk 冷启动期（<30d 无命中正常）可能被误判 orphan（仅多产信号，运营可忽略）。
- 根因（亲验 file:line）: `gap_signals.rs` orphan 检测 30d 窗口。
- 验证状态: PLAUSIBLE
- 修复建议: orphan 判定加 chunk age 下限（如 `created_at > 30d` 才判）。
- 状态: Open

### [1-07] suggestion 信号 `blocked_count_30d > 3` 阈值硬编码
- 入口 worker: `feedback_worker` → `run_structural_lint`（suggestion 类）
- 所属簇: 1
- 类型: 就绪债
- 严重度: Low
- 现象/风险: suggestion 类触发阈值硬编码，不可配。
- 根因（亲验 file:line）: `gap_signals.rs` suggestion 检测。
- 验证状态: CONFIRMED
- 修复建议: 提为可配。
- 状态: Open

### [1-08] sweep stage1 broken_link auto_resolve——target 恢复即消解，不校验语义一致（chunk_id 复用场景）
- 入口 worker: `feedback_worker` → `sweep_stale_signals`
- 所属簇: 1
- 类型: 逻辑边界
- 严重度: Low
- 现象/风险: broken_link 信号 sweep 时若 target chunk 恢复存在即 auto_resolved，不校验恢复的 chunk 是否真是原引用目标（chunk_id 复用/重建的边缘场景）。
- 根因（亲验 file:line）: `gap_signals.rs` `sweep_stale_signals` broken_link 分支。
- 验证状态: PLAUSIBLE
- 修复建议: auto_resolve 前校验 target chunk 语义一致。
- 状态: Open

### [1-09] structural_proposals 只产 pending_review 无 apply 消费方（就绪债 KB-06，红线正确的一面）
- 入口 worker: —（`gap_signals` 触发 `propose_structural_change`）
- 所属簇: 1
- 类型: 就绪债（红线正确的一面）
- 严重度: Low
- 现象/风险: structural_proposals 只产 `pending_review` 提案，全仓无 apply worker/人审 UI 消费。功能未闭环（模块头注释 KB-06 自认）。这是红线**正确**的一面（AI 绝不自动 apply split/merge），非缺陷。
- 根因（亲验 file:line）: `structural_proposals.rs:1-18` 模块头注释 KB-06 + 无消费方；序列化层无 apply/commit/delete 字段（:63-110 `StructuralProposal` + `STATUS_PENDING_REVIEW` 唯一构造口）。
- 验证状态: CONFIRMED
- 修复建议: 接线 apply worker + 人审 UI（下一轮）。
- 状态: Open（红线正确的一面，非缺陷）

## 簇 1 正向 HOLDS（主控亲验）
- **信号只观测不进决策红线 HOLDS**：Grep `src/agent/` 无对 `knowledge_gap_signals`/`structural_proposals` 的读取；消费端仅 `routes/`（admin 面板/timeline）。
- **structural_proposals 序列化层锁死**：无 apply/commit/delete 字段，`status` 恒 `pending_review`（唯一构造口 `StructuralProposal::new`）。
- **normalize_ref_key 无 substring 误伤**：`openai ≠ ai`（簇S 同款 normalize）。
- **stage2 LLM sweep 是预留桩**：未串入热路径，不产生实际 LLM 调用（模块头 doc:28-31）。

## 簇 2 findings（摄取源头）

> 审查对象：`ingest_worker.rs`(489) + `block_parser.rs`(476) + `catalog_rebuild.rs`(357)，消费入口对照 `routes/knowledge/import.rs` `ingest_chunked_text`。主控逐条亲验：ingest 去重链路、`content_hash:None`、worker 门控默认值、catalog job claim CAS + stale 回收。**校准结论：1 Medium + 6 Low**（subagent 自评 1H/2M/4L；[2-01] High→Medium：`INGEST_WORKER_ENABLED` 默认关、需显式开启才可达 + 后果受限；[2-02]/[2-03] Medium→Low：同门后/有实时聚合兜底）。

### [2-01] ingest 落库入口零内容去重，幂等完全押 HTTP 条件 GET(ETag)单点——不返 ETag 的源每轮全量重复落库、无界增长
- 入口 worker: `ingest_worker.rs` → `import.rs::ingest_chunked_text`
- 所属簇: 2
- 类型: 幂等
- 严重度: **Medium**（主控裁定：subagent 标 High，但 `INGEST_WORKER_ENABLED` **默认 false**（`config.rs:706` default "false" + `.env.example:244` + `main.rs:304` 门控 spawn，因触及 deploy topology 默认关），生产 117 单机默认部署下 ingest worker **根本不 spawn**→**默认不可达**。需管理员显式开启才触发；开启后确定性无界增长，但后果受限：新落 chunk 恒 `draft+needs_review` 不进 verified 召回池、不误发客户，仅灌爆 needs_review 审核队列 + document 集合膨胀。"功能门后确定性可达 + 后果受限"=Medium，非"默认部署确定性可达的丢任务/损坏"=High。）
- 现象/风险: 自动 ingest 去重只在 HTTP 层 ETag/If-None-Match；源不返 ETag（只带 Last-Modified 或无）→ 每过 `schedule_minutes` 同篇内容重抓→重分块→全量 `insert_one`，落库层零 content-hash/URL 去重。
- 失效链: `process_source` due 判定纯时间窗（`ingest_worker.rs:236-238` `now - last_fetched >= schedule`，无内容指纹）→ 条件 GET（`:104-105` `If-None-Match`，仅 `:109` 304→NotModified 才 skip）→ 200/无 ETag → `ingest_chunked_text` 每 block 无条件 `insert_one`（`import.rs:1154` `content_hash:None`，无查重）→ 同源每 `schedule_minutes` 落完整副本、无界增长。
- 根因（亲验 file:line）: `ingest_worker.rs:236-238`(due 纯时间窗) + `:104-109`(条件 GET 仅 304 skip) + `import.rs:1154`(content_hash:None) + `:1148`(无条件 insert_one)；全仓 content_hash 去重只在发送侧 `outbox.rs:179`，知识摄取侧零去重。
- 复现设想: 配返回 200 但无 ETag 头的源，schedule=60min，开启 ingest worker → 24h 后同篇文章 24 份 draft chunk。
- 验证状态: CONFIRMED（默认门控关，触发需显式启用）
- 修复建议: `ingest_chunked_text` 落库前按 `(source_id, content_hash)` 或 `(url, block_hash)` 查重跳过；`content_hash` 填真实指纹而非 None；或 ingest 层按 body sha256 与上轮比对未变则 skip。
- 状态: Open

### [2-02] `mark_success_with_etag` 返回值被 `let _ =` 吞错——写回失败下轮重复落库
- 入口 worker: `ingest_worker.rs`
- 所属簇: 2
- 类型: 幂等/吞错
- 严重度: **Low**（主控裁定：subagent 标 Medium，降 Low——同在 ingest 默认关门后（[2-01] 前提），且需叠加 `mark_success_with_etag` 的 update_one 瞬时失败才触发；与 [2-01] 同源，非常态。）
- 现象/风险: 落库成功后 `mark_success_with_etag` 写 `last_etag`/`last_fetched_at`，返回值被 `ingest_worker.rs:66` `let _ =` 吞掉；写库失败→etag 不落→下轮无 If-None-Match→200 全量重抓（叠加 2-01）。
- 根因（亲验 file:line）: `ingest_worker.rs:66` `let _ = mark_success_with_etag(...)`；`:302-320` 内 update_one 可失败。
- 验证状态: CONFIRMED
- 修复建议: 至少 warn；理想上写回失败应影响下轮判定或与落库同事务。
- 状态: Open

### [2-03] `catalog_rebuild` job claim 成 processing 后进程崩溃→无 stale 回收，job 永久 processing
- 入口 worker: `catalog_rebuild.rs`
- 所属簇: 2
- 类型: 崩溃恢复
- 严重度: **Low**（主控裁定：subagent 标 Medium，降 Low——catalog worker 默认开（interval=3，`.env.example:194`）故 claim-崩溃窗口可达，但 `build_operation_knowledge_catalog` 实时聚合兜底（persisted 仅缓存优化）→ catalog 功能不失效仅退化 O(N)；且需进程恰在 processing 窗口崩溃（低频）；后果仅单 document 缓存陈旧，非丢任务/损坏/误发。）
- 现象/风险: `claim_one_job` 原子 CAS `{status:queued}→processing`（亲验成立），但 `rebuild_one_document` 执行中崩溃→job 停 processing，无 reclaim/超时重置（`attempts` 只 `$inc` 不用于回收判定）→该 document catalog 摘要永不重建。
- 根因（亲验 file:line）: `catalog_rebuild.rs` claim CAS（`find_one_and_update {status:queued}→processing` + sort queued_at）无配套 stale-processing 回收；全文件无 reclaim。
- 复现设想: catalog job 领取后 kill 进程→重启后该 job 永 processing。
- 验证状态: CONFIRMED
- 修复建议: 加 processing 超时回收（类 `tasks.rs::reclaim_stale_running_tasks`），或 job 用 claimed_at + stale 窗口。
- 状态: Open

### [2-04] `block_parser` frontmatter 解析对畸形 YAML 静默降级——可能吞结构化元数据进正文
- 入口 worker: `block_parser.rs`
- 所属簇: 2
- 类型: 输入校验
- 严重度: Low
- 现象/风险: 遇畸形 frontmatter/分块标记静默 fallback 到整块 body，可能把结构化元数据吞进正文。不碰红线，属解析鲁棒性。
- 根因（亲验 file:line）: `block_parser.rs` 解析路径 fallback 优先无严格校验。
- 验证状态: PLAUSIBLE
- 修复建议: 畸形 frontmatter 至少产 warning 或 gap_signal。
- 状态: Open

### [2-05] `ingest_chunked_text` 无 workspace 配额/速率限制——单源可灌满 workspace
- 入口 worker: `ingest_worker.rs` → `import.rs`
- 所属簇: 2
- 类型: 就绪债
- 严重度: Low
- 现象/风险: 无 per-workspace chunk 上限/ingest 配额，配合 2-01 单源即可无界灌满；多租户下失控源影响整库。
- 根因（亲验 file:line）: `import.rs::ingest_chunked_text` 无配额检查。
- 验证状态: PLAUSIBLE
- 修复建议: 加 per-workspace chunk 上限/ingest 速率配额。
- 状态: Open

### [2-06] ingest 落库 document 创建与 chunk 插入非原子——中途失败留半截 document
- 入口 worker: `ingest_worker.rs` → `import.rs`
- 所属簇: 2
- 类型: 非原子写
- 严重度: Low
- 现象/风险: 先建 document 再逐块插 chunk，中途失败留 document + 部分 chunk（同 S-01 双写族）。非致命（下轮重跑/人工清理），留不一致态。
- 根因（亲验 file:line）: `import.rs` document/chunk 分步插入无事务。
- 验证状态: PLAUSIBLE
- 修复建议: 记录 ingest 批次 id，失败可回滚/标记。
- 状态: Open

### [2-07] 正向 HOLDS 汇总（红线/claim CAS/not-due 不写库）——设计正确标注
- 类型: 设计正确(标注)
- 严重度: Low（标注）
- 内容（逐条亲验成立）:
  1. **AI 永不自动 verify HOLDS**：block 路径（`import.rs:1242/1248`）+ fallback-blob（`:1192-1193`）无条件写 `status=draft`+`integrity_status=needs_review`，无绕过。
  2. **catalog job claim CAS HOLDS**：`find_one_and_update({status:queued}→processing)` 原子 + job_id 全局唯一索引，无 TOCTOU 双 claim。
  3. **not-due 不写任何 DB HOLDS**：`SourceOutcome::Skipped` 分支绝不刷 last_fetched_at（`ingest_worker.rs:58`），防节流基准无限前推（该行为在 mod.rs 注释明列为正确设计）。
  4. worker 单实例无并发 tick；block_parser 无 panic/数组越界。
- 验证状态: CONFIRMED
- 状态: Open（标注）

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
