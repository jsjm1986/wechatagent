# 系统优化工程 · 三线并行设计（方案 C）

- 日期：2026-08-13
- 状态：设计已获用户方向批准（方案 C + 全程 Fable 5 1M max）；执行模式 full_auto（spec/plan/实施自主推进，产品级取舍回询用户）
- 依据：`project-understanding/` 两轮深读档案（32 份记录、236 条疑点终裁、71 条文档偏差、17+ 条已核证缺陷）；总台账 `PROJECT_UNDERSTANDING_LEDGER.md`
- 用户已拍板的四项取舍：①首要目标=让 AI 对话质量可快速迭代（变回真 harness）；②尚未服务真实客户=速度优先可大胆重构；③金标先合成、上客户后换真实对话；④full_auto 执行

## 1. 背景与问题定义

两轮全仓深读的结论：系统的缺陷来源已从"缺防御"转为"防御之间的缝隙"（72 条实锤缺陷几乎全部是层间缝隙/防御自伤/制度空转）；核心循环（改 prompt→看质量）是全系统最慢路径而防御是 95 分；地质层堆积（退役结构不删）与文档反噬（71 条偏差）使认知成本超线性增长。

优化目标：把优化目标从"系统不出错"重新对准"客户对话质量每周变好"。

## 2. 总体架构：S0 前置 + 三线并行 + 两波收尾

```
S0 未提交工作收口（串行前置，见 §6 用户授权点）
  ├─ 线 A「发送链」   worktree: opt/line-a-send-chain
  ├─ 线 B「知识+前端」 worktree: opt/line-b-knowledge-frontend   ——三线并行
  └─ 线 C「演化器+金标环」worktree: opt/line-c-evolution-quality
按完成顺序合并（预期零冲突，见 §3 所有权矩阵）→ 金标环基线校准
  → 第四波 S5：文档瘦身（71 条偏差驱动）+ 产品决策项打包回询用户
```

**并行冲突控制的核心设计**：三线按**文件所有权**切分而非按工作性质切分。每个文件唯一归属一条线；跨线依赖（如 B 线知识窗口修复影响 C 线金标场景的召回表现）通过"合并后统一校准基线"吸收，不在线间同步。

## 3. 文件所有权矩阵（冲突控制的硬边界）

| 线 | 独占文件/目录 | 禁触 |
|---|---|---|
| A | `src/webhooks.rs`、`src/tasks.rs`、`src/agent/gateway.rs`、`src/agent/outbox.rs`、`src/agent/outbox_dispatcher.rs`、`src/agent/escalation/**`、`src/agent/quiet_hours.rs` | evolution/review/knowledge/前端 |
| B | `src/routes/knowledge/**`、`src/knowledge_wiki/**`、`src/agent/knowledge_*.rs`、`src/agent/chat_tool_loop.rs`、`src/prompts.rs`、`src/prompt_guard.rs`、`src/routes/products.rs`、`src/routes/principal_escalations.rs`、`src/routes/ask_human_inbox.rs`、`frontend/**`、`src/knowledge_task/**` | gateway/outbox/evolution/review |
| C | `src/evolution/**`、`src/agent/review/**`、`src/routes/evolution.rs`、`tests/**`（新增与修改）、`scripts/**`（新增）、evaluation 相关路由 | gateway/webhooks/knowledge/前端 |
| 共享只读 | `src/models.rs`、`src/agent/mod.rs`、`src/config.rs` 等——三线均不得修改；若确需改（如新增 config 项），登记到主会话由合并时统一处理 | — |

例外协议：任何线发现必须触碰非所有权文件时，停下该项、登记回主会话仲裁，绝不越界改。

## 4. 三线工作清单（引用档案编号，实施前须按台账纪律重验锚点）

### 线 A · 发送链（修复 2 + 清淤 3）
- A1【缺陷#2】毒丸消息行：`reconcile_pending_inbound_handoffs` decode 失败改为"跳过该行 + 标记 `handoff_status=quarantined` + warn 事件"，不再 `?` 中止整个 tick（`webhooks.rs:819-822`）。补集成测试：坏行在场时 tick 仍处理后续行。
- A2【终裁 01-2】deferred_wake legacy 清淤：删除 `DEFERRED_INBOUND_REPLY_KIND` 的 4 处 gateway 判定分支与 webhooks reconcile 的 legacy 分支（全仓无创建点，代码自标 legacy）；未上客户，DB 残留行按 reconcile 收敛语义自然消化，不写迁移。
- A3【缺陷#5】manual_send 两门矛盾：采用保守语义（二次门保持拦截），删除下游 `check_contact_status_pure` 的 manual_send 死代码豁免与误导注释，语义决策记录进代码注释。
- A4【23 号终裁】delivery_unknown 请示卡滞留：超时扫描的 filter 纳入 `delivery_state != sent` 的滞留卡（或加独立收敛分支）。
- A5 过时注释清理：`gateway.rs` apply_agent_updates 段、`:2915` 退役 key 残留、`5659-5662` 计数注释补全。

### 线 B · 知识+前端（修复 5 + 清淤 3）
- B1【缺陷#14】referrers 恒 400：后端 `ChunkReferrersQuery` 加 `#[serde(alias = "target_id")]` 兼容 + 前端改发 `targetId`（双保险）。
- B2【缺陷#15】products 过滤失效：同款双保险（后端 alias + 前端改 `activeOnly`）。
- B3【缺陷#17】DateTime 前端崩溃：`principal_escalations`/`ask_human_inbox` 两处裸 bson DateTime 改序列化为毫秒数或 ISO 字符串（对齐 `domain_profiles.rs:521-528` 已修方案）；前端 formatExpiry 防御性兼容。
- B4【缺陷#9】锚点口径统一：crud/verify/digest_inbox/catalog 四处裸 `!is_empty()` 改 `chunk_has_citable_anchor`。
- B5【缺陷#4】知识窗口错位：`cited_in_corpus` 不再与 200 条窗口求交——agent cited 的 chunk 改为按 id 直查并验证 `active+verified` 后放行（保持 verified-only 红线，消除窗口错位降格）。
- B6【25 号确认】knowledge_task `execute_step` 死路径删除，统一走两阶段提交路径。
- B7【偏差#2】`user.reply.task` 退役清理：种子包移除该 spec（或标注 retired 不再种入）、prompt_guard 治理面收缩、prompts.rs 守护测试同步更新；保留 m043 等迁移测试的历史 fixture 引用不动。
- B8 chat 更新映射表死字段（`safe_claims/forbidden_claims`）清理、crud PUT 响应注释精确化。

### 线 C · 演化器+评审+金标环（建设 1 + 修复 3 + 清淤 2）
- C1【核心建设】金标回归环 v1：
  - 场景：复用 roleplay 体系（第三族 LLM 演客户）批量生成五类场景（寒暄/异议/压力/知识/边界）各 20-30 条，人工可后筛；落 `tests/fixtures/quality_gold/` 版本化 JSON。
  - 评分：复用 `tests/common/judge.rs` 多裁判仪器（K 采样取中位、校准、分歧剔除）。
  - 入口：`scripts/quality-regression.sh`——本地一键跑 simulation×场景×judge，输出五类分布与总分，目标分钟级。
  - 门槛：先软门收集基线分布（每次跑落 ledger），基线稳定后（≥3 次运行方差可接受）升硬门（overall floor），写入 CI nightly；PR 门暂不挂（成本考量，未上客户）。
  - 金标偏合成的事实随场景文件 metadata 记录，上客户后按用户已拍板策略换血。
- C2【缺陷#16】演化器 pressure 统计源修复：`classify_gate_hit` 移除 `blocked_by_safety_guard → pressure_risk_block` 错误映射；pressure 候选生成降级为观测（无正确统计源前不产候选），significance/auto_release 同口径修正。
- C3【缺陷#3】双脑 parse 失败改回退主 review（对齐"增益机制不成为故障源"注释意图），保留 warn 与审计事件。
- C4【终裁 10-x】演化器死代码：`auto_release` 政策硬闸恒关模块、`schedule_post_release_review`、`is_evolution_enabled_for` 无调用方 API 删除（保留 post_release 本体）。
- C5【缺陷#8】空壳测试落实：`memory_card_write_occ` 实现（Docker 可测，OCC 并发断言）；`revision_recheck_action_gate` 用 mock LLM 路径实现最小可执行版本；实现不了的部分显式删除文件并在 28 号无守护清单记录。
- C6【缺陷#12】复刻漂移测试修复：`escalation_push_time_reassign` 改断言 `$unset` 语义；`autonomy_protocol_pbt` P2 模型补 `apply_revision_fallback` 分支；`dry_run_isolation` 幻影值 `"completed"` 改 `succeeded`。
- C7【缺陷#6】`evolution.rs` 灰度旗 `updated_by` 改用 `admin.username`（服务端身份）。

## 5. 执行协议

- **模型约束（用户指令）**：主会话与全部 subagent 一律 Fable 5 thinking 1M max；subagent 统一 `model: inherit`，禁用任何其他模型。
- **隔离**：三线各自 git worktree + 分支（`opt/line-{a,b,c}-*`），互不可见；主会话协调与仲裁。
- **红线不变**：CLAUDE.md 全部红线照守（改前 100% 读懂受影响路径、file:line 亲验、发送必经 gateway/outbox、AI 永不 verify、无"人工接管"语义词——所有新增代码/注释/文案过 `check-no-human-takeover` 词表）。
- **每线交付标准**：①`cargo test --lib` ≥350 + 四 PBT ≥33（基线门）；②该线改动的针对性新增/修正测试全绿；③`cargo check --tests`（-D warnings）过；④禁词 lint 过；⑤深读档案增量回写（各线写自己域的记录更新，主会话验收）。
- **合并协议**：按完成顺序合并；预期零冲突（所有权矩阵）；每次合并后跑基线门；三线全合并后跑金标环校准基线分数。
- **验证边界**：本地只跑轻量套件（CLAUDE.md 磁盘纪律）；Docker 集成测试标 `--ignored` 交 CI；biz-test 生产机验收在 S5 前统一跑一轮（需用户提供环境时机）。
- **档案纪律**：实施中发现档案与代码不符 → 先修档案再动代码；所有 file:line 引用动手前当场重验。

## 6. 用户交互点（仅三处）

1. **S0 提交授权（开工前唯一阻塞项）**：47 个未提交文件按 19 号分析为"6 组后端行为变更 + biz-test 硬化"的同一波工作（cargo check 通过、src 与 scripts 必须同批合入），且含两处禁词"人工"需先改措辞（`import.rs` 两处改"运营核验"）。需用户确认：修禁词后将这批工作提交为基线 commit（git 红线：不经确认不提交）。
2. **S5 产品参数打包**：B4 发送节奏加权、B6 静默时段高意向豁免、请示预授权底线、成本分级路由——三线合并后一次性带方案回询。
3. **业务语义变更即时回询**：任何实施中发现"修复会改变客户可感知行为语义"的项，暂停该项回询（如 A3 的 manual_send 语义已按保守预定，若实施中发现更优语义则回询）。

## 7. 成功标准

- 三线合并后：17 条已核证缺陷中的代码类全部关闭（或显式记录为设计决策）；死代码清单归零；基线门全绿。
- 金标回归环：一条命令、分钟级出分、五类场景 ≥100 条、基线分数落档；"改 prompt → 出质量信号"的循环时间从"数小时 nightly"降到"分钟级本地"。
- 档案同步：全部改动反映进 project-understanding/ 与总台账；不新增未终裁疑点。
- 复杂度方向逆转：本轮净删除行数 > 净新增行数（金标环新增除外）。

## 8. 自审记录

- 占位/TBD 扫描：无。
- 一致性：三线清单与所有权矩阵逐项核对——B7 触碰 prompts.rs/prompt_guard.rs（B 线所有权）✓；C6 触碰 tests/（C 线所有权）✓；A 线不触 review/**（双脑修复在 C）✓。
- 范围：每线规模适中（修复 2-5 项+清淤 2-3 项/线，C 线含一项建设），单线单 plan 可执行。
- 歧义：A2 的 DB 残留行处理已明确"不写迁移"；C1 门槛升级条件已量化（≥3 次运行方差可接受）。
