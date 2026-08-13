# 优化第四波 · 文档准确性收敛 实施计划（线 F + 线 G）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 消灭全部已知文档误导性：以 `project-understanding/29-doc-code-divergence-master.md`（71 条偏差权威总表）为需求清单修正/标注，并把本轮优化工程（S0 + 三线 + S5 两线）带来的新事实同步进活文档。

**Architecture:** 不追求删行数——"瘦身"的实质是让每份文档要么准确、要么显式标注失真与权威源。两线并行：F=核心活文档（读者会照着做的），G=规格/历史/营销文档（读者会被误导认知的）。全部为 md/HTML 文本改动，零代码。

## Global Constraints（两线共用）

- 模型：仅 Fable 5 1M max（不派生 subagent）。
- **修正纪律（凌驾一切）**：每处修正前必须核对代码现状（29 号每条带证据锚点，但工作区在优化工程中又演进了——锚点涉及的行为若被 A/B/C/D/E 线改过，以当前代码为准，动手前重验）；修正后的表述必须是"当前为真"的事实，不确定就写"以 `<file:line>` 为准"的指针而非断言。
- **CLAUDE.md 特殊边界（仅线 F）**：只修事实性描述（架构/机制/数字/环境/命令），**指令性条款一字不动**（红线中的红线、Superpowers、Communication、Subagents 四节的规则语句不许改语义——其中的过时事实引用可在句内修正，如"5 闸"表述，但规则意图零变化）。
- 禁词红线：全部新增文案过 `bash scripts/check-no-human-takeover.sh`；website 修正不引入模型品牌名。
- 每任务独立 commit（`docs(...)` / `fix(website)` 前缀）；两线不得触碰对方文件集；收尾各自跑禁词 lint。
- 本轮新事实清单（两线按需引用，同步时以此为准）：金标回归环（`scripts/quality-regression.sh`、105 场景、nightly quality-gold 软门）；毒丸行 quarantine；deferred_inbound_reply 分支已删；manual_send 保守语义；滞留请示卡超时收敛；referrers/products 参数 alias；escalation 时间 RFC3339；锚点口径统一；知识窗外引用携带修复；user.reply.task 种子退役；execute_step 死路径删除；pressure 统计源修复（该闸不再产候选）；双脑 parse 回退；auto_release 已删；灰度旗服务端身份；静默时段交易意图豁免；发送间隔长度加权（35ms/字封顶 max+6s、0/0 恒 0）；请示预授权底线（standing_order 双字段成对）；寒暄轮跳 ClaimGate（七条件+审计标记）；基线现状 lib=2562/pbt=41。

---

### 线 F · 核心活文档修正

**文件所有权**：`CLAUDE.md`、`README.md`、`AGENTS.md`、`docs/README.md`、`docs/agent-policy.md`、`docs/architecture.md`、`docs/data-and-api.md`、`docs/product-modules.md`、`docs/ai-agent-system.md`、`docs/frontend-design-system.md`、`docs/knowledge-wiki.md`。

#### Task F1: CLAUDE.md 事实性修正
- 按 29 号反向索引该文件全部条目逐条处理，至少包括：五闸字符串守卫表述（→ 分数闸 + R5.4 现实）；"shell 是 bash on Windows"（→ macOS/zsh，或改为环境中立表述）；`check-baseline` 行号引用漂移；lib 基线数字表述（门槛 350 与当前实际 2562 分开表述）；worker 数量若有提及；`user.reply.task` 相关（若有）。
- 增补一小节（事实性导航，非指令）：`project-understanding/` 档案的存在与"改动前使用顺序"指针、金标回归环一键命令。
- 指令性四节（红线中的红线/Superpowers/Communication/Subagents）规则语义零变化。
- Commit：`docs(claude): align factual descriptions with current gate system, environment, and archive`

#### Task F2: README.md 全面对齐
- 数字类：路由 235、迁移 m001–m058、worker 16、集合约 79（以 30 号事实卡口径）、基线叙述更新（含前端 750 测试）。
- 行为类：webhook→durable inbound 任务模型（含 quarantine）、quiet-hours 交易意图豁免、长度加权节奏、standing order、寒暄跳 ClaimGate、金标回归环（能力范围表+开发验证节补 `scripts/quality-regression.sh`）。
- Worker 矩阵表核对 16 项现状；"文档同步基线"行更新为本轮 commit。
- Commit：`docs(readme): sync capabilities, worker matrix, and verification numbers with optimization waves`

#### Task F3: docs 七篇核心逐篇修正
- `agent-policy.md`：五闸章节头注更新（分数闸+pressure 无终态统计源）；quiet hours 节补交易意图豁免；Ask-Human 节补 standing order（预授权底线语义：执行人类预授权非代决）；演化章节补 auto_release 已删、pressure 候选不再生成；自学习章节数字核对。
- `architecture.md` / `data-and-api.md`：模块表/集合数/挂载数对齐 30 号事实卡；新增 quality_gold 资产位置。
- `product-modules.md` / `ai-agent-system.md` / `frontend-design-system.md` / `knowledge-wiki.md`：按 29 号反向索引各自条目修正（若该文件无高/中条目则快速核读后标注"2026-08-13 核对"于文首日期行，不硬改）。
- 每篇一个 commit：`docs(<name>): correct <n> divergences per master table`

#### 线 F 收尾
- `bash scripts/check-no-human-takeover.sh` 0 违规；交付报告 ≤12 行（每文件修正条数、跳过条目与理由）。

---

### 线 G · 规格/历史/营销文档

**文件所有权**：`.kiro/specs/**`（仅头部注记）、`BUSINESS_EFFECT_REVIEW.md`、`SWARM_BUSINESS_LOGIC_REVIEW.md`、`CODE_REVIEW_FINDINGS.md`、`docs/sunset-plan.md`、`docs/ci-known-gaps.md`、`website/**`、`project-understanding/29-doc-code-divergence-master.md`（仅授权追加终态节/终态列）。

#### Task G1: kiro specs sunset 注记刷新
- 三大 spec（autonomy-loop/hardening/self-evolution）的 sunset notice 本身已过时（17 号发现：所写"3 闸 enforce_*"是中间态）——在每份 requirements.md 头部**追加**一段"2026-08-13 现状注记"（现行=分数闸+R5.4；基线门数字；指向 29 号与 30 号为权威），**不改写正文**（历史存档原则）。
- `agent-self-evolution` 追加：auto_release 通道已物理删除、pressure gate 候选不再生成（SR-180 契约冲突的最终处置）。
- Commit：`docs(specs): refresh stale sunset notices with 2026-08-13 reality pointers`

#### Task G2: 三份评审台账"已关闭"标注
- `BUSINESS_EFFECT_REVIEW.md`：B4（长度加权已落地）、B6（交易意图豁免已落地——注明保守收窄版）、B1/B2/B3/B5 等原"已修"条目保持；文首加一段 2026-08-13 处置摘要。
- `CODE_REVIEW_FINDINGS.md`：文首加处置摘要（本轮工程关闭的条目映射：S2-07 未动/S3-32 未动等仍开放项如实保留）。
- `SWARM_BUSINESS_LOGIC_REVIEW.md`：核对三结论是否仍准（standing order 新增了 resolved_via 第三值——若文中有 via 闭集断言需标注）。
- Commit：`docs(reviews): annotate items closed by the 2026-08-13 optimization waves`

#### Task G3: website 事实错误修正（21 号发现的 4 处 + 数字过时）
- `evolution.html` emotional_value 基线 5.0 → 6.0；debounce"约 4 秒"→ 2 秒；trust vs engineering 页 CI 门数矛盾统一（以 `check-ci-gate-policy.py` 的 HARD_JOBS 现状为准，动手前数一遍）；trust 页 PressureRisk"先改写"表述与代码对齐（软闸 revision 表述本就对？21 号说 trust 页写成"硬拦写成先改写"——重验 gates 现状：pressureRisk≥7 是软闸触发 revision——那"先改写"是对的、写"硬拦"的才错？以 21 号记录原文与代码复核为准，谁错改谁）。
- 数字过时项（18 频道→20、收件箱 8 流→9、11 万行→19 万等）：改为不易过时的表述（"20 个管理频道"或去掉精确数字），`agents.html:116` 的 user.reply.task 展示改为 fast.task。
- 默认关闭能力（演化/冷激活/续费提醒）在营销页补"需显式启用"角标或脚注——诚实营销。
- Commit：`fix(website): correct factual drift and mark default-off capabilities`

#### Task G4: 29 号偏差表终态化（授权的档案追加）
- 在 29 号末尾追加"终态核销表（2026-08-13 优化工程后）"：71 条逐条标注——已修（commit/线）/文档已标注/仍存在（含理由：历史存档不改/产品决策项）/已过时失效。凡"仍存在"的高严重度条目必须有归属（谁负责、何时）。
- Commit：`docs(understanding): add closure ledger to divergence master table`

#### 线 G 收尾
- 禁词 lint + `check-no-model-hint.sh`（website 改动）0 违规；交付报告 ≤12 行。
