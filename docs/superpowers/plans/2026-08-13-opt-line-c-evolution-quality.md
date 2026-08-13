# 优化线 C · 演化器+评审+金标回归环 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建成金标质量回归环 v1（合成场景×多裁判×一键命令）；修复演化器统计源失真、双脑 parse 拉闸、灰度旗审计身份；删除演化器死代码；落实/修正五个测试缺口。

**Architecture:** 金标环全部为新增文件（fixtures/脚本/一个新集成测试 target），零侵入生产链路；演化器修复以"错误映射移除 + pressure 候选降级为观测"为界，不做目标函数换血（那是后续波次）；全部改动限线 C 所有权。

**Tech Stack:** Rust、testcontainers、真实 LLM（金标环运行时需 REAL_LLM_* env，环缺省时脚本 fail-fast 提示）。

## Global Constraints（每任务隐含）

- 模型：仅 Fable 5 1M max（不派生 subagent）。金标环运行期调用的业务 LLM 按 env 配置（与生产同源），judge 需异族时由 env 指定——脚本不得硬编码任何模型名（`check-no-model-hint.sh` 约束）。
- 红线：动手前重验锚点；evolution 隔离红线（不触 gateway/outbox/MCP，CI lint 守）；禁词 lint；金标场景文案不得含模型品牌名。
- 文件边界：只许修改 `src/evolution/**`、`src/agent/review/**`、`src/routes/evolution.rs`、`tests/**`、`scripts/**`（新增）。越界登记不执行。
- 每任务收尾：`cargo check --tests`（-D warnings）+ 相关测试绿 + 独立 commit。
- 全线收尾：`cargo test --lib` ≥2530/0、四 PBT、evolution 隔离 lint、禁词 lint。

---

### Task C1: 金标回归环 v1（核心建设，三个子任务）

#### C1a: 场景 fixture 体系

**Files:**
- Create: `tests/fixtures/quality_gold/README.md`（格式说明+metadata 约定）、`tests/fixtures/quality_gold/{casual,objection,pressure,knowledge,boundary}.json`（五类各 20-30 条）
- Create: `scripts/quality-gold-generate.py`（可选辅助：驱动 roleplay 生成候选场景的离线脚本，标注 synthetic 来源）

**行为契约:**
- 场景 schema（每条）：`{ id, category, description, contactSeed(阶段/意向/画像), inboundMessages[1..3], knowledgeSeeds[]（knowledge 类必填：需先 seed 并 verify 的 chunk 内容）, expectations { mustNotViolate[]（红线类硬断言：禁词/不编造/不转接语义）, qualityFloor（judge overall 下限，默认继承全局）}, metadata { source: "synthetic-v1", generatedAt } }`。
- 首版场景内容由实施者基于 Soul v3 四模式与 `tests/common/roleplayer.rs` 的对抗轮类型撰写/生成（禁止从生产库抄真实客户数据——当前也没有）；每类覆盖该模式的典型+边界情形（如 pressure 类须含"要真人/要负责人"逼问轮）。
- fixture 版本化进 git；上客户后按 spec 用真实对话换血（metadata.source 字段即为换血追踪点）。

**Steps:**
- [ ] 读 `tests/common/{judge.rs,roleplayer.rs}`、`src/agent/simulation.rs`（`simulate_user_dialogue` 入参/返回契约）、`prompts.rs` Soul v3 与 policy 全文——金标环消费这三者，契约必须先吃透。
- [ ] 定稿 schema（写进 README）→ 撰写五类场景（≥100 条总量）→ JSON schema 自校验小测试（`tests/quality_gold_fixtures_smoke.rs`，非 ignore：纯文件解析+schema 断言）。
- [ ] Commit：`feat(quality): add versioned synthetic gold scenario fixtures (5 categories, 100+ cases)`

#### C1b: 回归执行器

**Files:**
- Create: `tests/quality_gold_regression.rs`（`#[ignore]` 集成 target：真实 LLM + testcontainers）
- Create: `scripts/quality-regression.sh`（一键入口）

**行为契约:**
- 执行器逐场景：seed contact/knowledge（复用 `tests/common::TestApp` 与 roleplay_fixtures 的 seed 原语）→ `simulate_user_dialogue`（shadow，零真实发送）→ 收集最终回复与 run 元数据 → 红线硬断言（mustNotViolate：复用 `tests/common/redline` 断言库）→ judge 打分（复用 `tests/common/judge.rs` 多裁判仪器，`REAL_LLM_JUDGE=1` 时启用，否则只跑红线硬断言并在报告标注 judge skipped）。
- 输出：每场景一行 JSON ledger（`target/quality_gold/run-{timestamp}.jsonl`：id/category/scores/violations/latency）+ 终端五类汇总表（均值/中位/floor 命中数）。
- 门槛：v1 为**软门**——红线违规即 fail（这是硬的），judge 分数只记录不 fail；ledger 累积 ≥3 次运行且方差可接受后由主会话决策升硬门（floor 值写进 fixture 或 env，不硬编码）。
- `scripts/quality-regression.sh`：检查 env（REAL_LLM_API_KEY 等）缺省 fail-fast 并给出设置指引 → `cargo test --test quality_gold_regression -- --ignored --nocapture` → 汇总输出 ledger 路径与本次五类分布。

**Steps:**
- [ ] 写执行器骨架（先 1 条场景端到端跑通：seed→simulate→redline→judge→ledger）→ 扩到全量 fixture 驱动 → 本机有 key 则跑一次全量并留 ledger（无 key 则 `cargo check --tests` 过并注明留待有环境时跑）。
- [ ] Commit：`feat(quality): add gold-scenario regression runner with redline hard-asserts and judge ledger`

#### C1c: CI 接入（nightly 软门）

**Files:**
- Modify: `.github/workflows/ci.yml`（nightly 链追加 `quality-gold` job，`continue-on-error: true` 起步）——**注意：ci.yml 不在三线所有权矩阵，此改动登记回主会话执行**；本任务只产出 job 定义片段放 `scripts/quality-regression-ci-snippet.yml` 供主会话合并。

**Steps:**
- [ ] 写 job 片段（复用 real-llm 链的 secrets/env 模式）→ Commit：`chore(quality): provide nightly quality-gold job snippet for main-session CI merge`

### Task C2: 演化器 pressure 统计源修复（缺陷 #16）

**Files:**
- Modify: `src/evolution/threshold.rs`（`classify_gate_hit` :67-72 移除 `"blocked_by_safety_guard" => pressure_risk_block` 映射；pressure 候选生成降级——无正确统计源时不产候选，band 表保留注释说明）、`src/evolution/significance.rs` 与 `src/evolution/auto_release.rs` 同口径修正（重验各自对该映射的引用）、`src/evolution/post_release.rs`（其口径已正确——重验后保持）
- Test: threshold.rs 现有单测更新（`classify_gate_hit("blocked_by_safety_guard")` 断言改为映射到事实来源或 None）

**行为契约:**
- 依据：`blocked_by_safety_guard` 的生产来源是 R5.3.a fail-closed 与 unsupported business claim（`gates.rs:473,779,818`），pressure 是软闸不产终态。
- 修复语义：该状态在 threshold 统计里归入 `product_accuracy_score_block` 类（其真实语义近产品声明门）**或** 不归任何 gate（保守）——实施者读 `gates.rs` 两个写点的语义后选定并在注释说明依据；pressure_risk_block 无终态统计源 → `generate` 对该 gate 跳过候选生成（写 tick 事件说明 reason=`no_terminal_signal_source`），revision_applied 路径的 rewrite 类补判不变。
- #152 安全反向门对 pressure 的空转随之消除（该 gate 不再产候选）。

**Steps:**
- [ ] 重验 `classify_gate_hit` 全部调用方与 `gates.rs` 两写点语义 → 失败测试 → 实现 → evolution 全部单测绿 → Commit：`fix(evolution): stop misattributing blocked_by_safety_guard to the pressure gate`

### Task C3: 双脑 parse 失败改回退（缺陷 #3）

**Files:**
- Modify: `src/agent/review/mod.rs`（:4409-4415 分支）
- Test: review 单测补：second parse 失败时返回主 review（approved 状态保持）且 risks 含 `second_reviewer_schema_failed` 观测标记

**行为契约:** parse 失败对齐 LLM 调用失败的处理（warn + 回退主 review），补一条 risk 标记与 warn 日志供审计；`hold_for_review_schema_failure` 仅保留给主 reviewer parse 失败路径（fail-closed 语义对主脑仍正确）。

**Steps:**
- [ ] 重验 → 失败测试 → 实现 → Commit：`fix(review): fall back to primary review when second reviewer output fails schema (dual-brain must not gate)`

### Task C4: 演化器死代码删除（终裁 10-x）

**Files:**
- Modify: `src/evolution/auto_release.rs`（政策硬闸恒关模块——删除模块与其在 mod.rs tick 的调用位、runtime flag 校验中对 `CURRENT_AUTO_RELEASE_POLICY_ENABLED` 的引用改为固定拒绝文案；重验 `routes/evolution.rs` :749-752 的引用一并处理——该文件线 C 所有权）、删 `schedule_post_release_review` 与 `is_evolution_enabled_for`（全仓 grep 确认零调用后删）
- Test: 相关单测同步删除/更新

**行为契约:** post_release 本体保留（有真实消费）；只删"永不可达的自动发布通道"与两个无调用方 API。删除后 `put_evolution_runtime_flag` 对 `threshold_auto_release_enabled=true` 仍拒绝（硬编码拒绝理由不再引用被删常量）。

**Steps:**
- [ ] 全仓 grep 三个符号确认封闭 → 删除 → evolution 单测+隔离 lint 绿 → Commit：`chore(evolution): remove permanently-gated auto-release path and two orphan APIs`

### Task C5: 灰度旗审计身份修复（缺陷 #6）

**Files:**
- Modify: `src/routes/evolution.rs`（:742-746：`updated_by` 改用 `admin.username`，请求体字段废弃——保留反序列化兼容但忽略）
- Test: 路由单测断言落库 updated_by == 会话身份

**Steps:**
- [ ] 重验 → 失败测试 → 实现 → Commit：`fix(evolution): stamp runtime-flag updates with server-side admin identity`

### Task C6: 空壳与漂移测试落实（缺陷 #8、#12）

**Files:**
- Modify: `tests/memory_card_write_occ.rs`（实现真实 OCC 并发断言：双路并发写 memory_card，断言零 Err、版本单调、恰一路 modified）——**注意 memory.rs 是线 A 域外/线 C 域外的共享只读**：本测试只经公共 API（TestApp + gateway 入口或 memory pub 函数）驱动，不改生产代码；若无法不改生产代码而实现，降级为删除该文件并在报告注明。
- Modify: `tests/revision_recheck_action_gate.rs`（用 mock LLM 路径实现最小版：seed forbidden state policy + 构造 revision 场景断言 held_by_ai_policy 与零 outbox；mock LLM 按 `tests/common/mod.rs` 锚文本路由协议注入 revision 响应；确实无法确定性驱动则删除文件并在 28 号无守护清单确认记录）
- Modify: `tests/escalation_push_time_reassign.rs`（用例 1 改断言 `$unset` 语义：改派后 `last_pushed_at_ms` 为空、送达确认后重置）
- Modify: `tests/autonomy_protocol_pbt.rs`（P2 模型补 `apply_revision_fallback` 分支——对照 `gates.rs:1258-1275` 现实现补"纯风格失败回退原稿 approved"路径，断言方向修正）
- Modify: `tests/dry_run_isolation.rs`（`"completed"` 幻影值改 `"succeeded"`；若测试意图是驱动生产 dry-run 分支而未达成，按 28 号建议改走 post_management_message 真路径或在文件头如实降级注释）

**Steps:**
- [ ] 逐文件重验对应生产行为（gates.rs revision fallback、ledger.rs reassign、models.rs 闭集）→ 逐个实现/修正 → 各测试可本地跑的跑绿、需 Docker 的 `cargo check --tests` 过留 CI → 分别 commit（`test(...)` 前缀，一文件一 commit）。

### 收尾

- [ ] 全线验证：`cargo test --lib` ≥2530/0、四 PBT、evolution 隔离 lint、禁词 lint、`-D warnings` check。
- [ ] 交付报告：C1 环的首次 ledger（或环境缺省说明）、C2 语义选定依据、C6 各测试的落实/降级结论、锚点重验记录、档案回写要点、ci.yml 片段位置（留主会话合并）。
