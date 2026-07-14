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

（主控亲验后填入）

## 簇 1 findings（信号生成消解）

（主控亲验后填入）

## 簇 2 findings（摄取源头）

（主控亲验后填入）

## 簇 3 findings（反馈闭环）

（主控亲验后填入）
