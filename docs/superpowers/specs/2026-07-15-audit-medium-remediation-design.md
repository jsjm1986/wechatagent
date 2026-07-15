# 审计遗留 Medium findings 修复设计（2026-07-15）

## 背景

六批深度审计（[[project-deep-logic-audit-remediation]] 起，共 101 findings）收官后，唯一 High（Batch3 worker-fleet [1-01]）已修（PR#216），worker S-01/S-02 已修（PR#217）。本设计处理**剩余确定性可达的纯代码类 Medium**，用户裁定范围 = Batch1 五条（A-01/A-02/A-03/C-01/C-02）+ Batch5 [2-01]，共 6 条。

排除项（用户不纳入本批）：Batch2 [1-01]/[4-01]（多租户就绪债，`project_multitenant_isolation_debt` 口径「启用才修」）、Batch5 [S-01]（灰度门，双闸默认关+admin 兜底）、Batch6 [C-01]（m011 清库，修复入口是运维设 APP_ENV 非代码）。

**A-02 取舍留痕**：对抗式交叉验证（PR#215）曾将 A-02 降为 Low（软标签/可自愈/fail-closed 刻意设计）；台账原标 Medium（偏 High、最值得修）。两份产物冲突，用户裁定**保留 A-02 修全六条**。

所有行号基于最新 origin/main（HEAD=fee3115），经三组 subagent + 主控逐条 Read 亲验；审计写作时的旧行号多已漂移，本文档用亲验后的当前行号。

## 红线遵循

- 全部 6 条修复遵守「DEFAULT 销售域字节等价」：非默认行业才生效，不劣化现有生产行为。逐条给出等价性论证。
- 不为过测试改业务逻辑/阈值/prompt/guards（反过拟合红线）；本批是修真实缺陷，测试钉住修复后的正确行为。
- `check-no-human-takeover` / `check-no-model-hint` / `check-evolution-isolation` 三 lint 门：涉及 `src/agent/`、`src/evolution/` 的改动新增行不得含禁词。

## 分组与 PR 策略

三条子系统独立、互不牵连（subagent 亲验确认），分 **3 个 PR** 隔离风险，各自 CI 绿再合：

| PR | 范围 | 文件 |
| --- | --- | --- |
| PR-A | A-01 + A-02 + A-03 | `src/agent/memory.rs`（同一 consolidation 落库函数）|
| PR-C | C-01 + C-02 | `src/agent/domain_signals.rs` + `gateway.rs` + `routes/shared.rs`（C-01）/ `src/agent/decision.rs`（C-02）|
| PR-E | 2-01 | `src/evolution/post_release.rs`（+ 复用 `significance.rs` 常量）|

---

## PR-A：记忆层 consolidation 三缺陷

三条同处 `consolidate_contact_memory` 的 OCC winner 落库段（memory.rs:1542-1700），共用一组集成测试。

### A-03（先修，最小）：confirmed_tags 写改 fail-soft

**缺陷**：落库顺序 memory_card OCC 写（:1560-1580，`?`）→ confirmed_tags 写（:1612-1624，硬 `?`：:1621 `to_bson?` + :1624 `.await?`）→ personality 写（:1630-1688，fail-soft warn）→ 候选标 consolidated（:1690-1700，`?`）。memory_card 已写成功后，confirmed_tags 硬 `?` 失败 → 整函数返 Err → 候选未标 consolidated → task retry（tasks.rs:255 attempt<max→retry）→ 候选二次并入已推进的卡。

**修复**：把 confirmed_tags 写（memory.rs:1612-1624）从硬 `?` 改 **fail-soft**（`match ... Err→tracing::warn`），与同段 personality 写（:1656-1688）对齐。memory_card 已是权威落库，confirmed_tags 是 best-effort 搭车写，失败不应触发整轮重放。`to_bson(&reconfirmed)?`（:1621）也一并进 match。

**等价性**：正常路径（写成功）行为完全不变；仅错误路径从"返 Err 触发 retry"变成"warn 不阻断"，消除重放。

### A-02：消费 discardedTags，confirmed_tags 改「保留 unless 显式弃用」

**缺陷**：:1552-1553 replace 语义——`reconfirmed = parse_reconfirmed_tags(&value, &window)`（:1553）在截断窗口（60 条/6000 字，consolidation_window.rs）上整体重判，`$set confirmed_tags`（:1621）整体覆盖。证据滚出窗口的持久标签因无法给窗内 evidenceTurn 被 fail-closed 丢弃（parse_reconfirmed_tags:1086-1090 evidences.is_empty()→None）。与 core_facts「未显式弃用即自动保留」（prompts.rs:1461）形成不对称。

**现成通道**：LLM 已被要求输出 `discardedTags:[{value,reason}]`（prompts.rs:1443/1455），但**全仓零消费**（subagent + 主控 grep 确认，仅 prompt 文本 + schema 存在性测试引用）。

**修复**：在 OCC winner 分支（:1612 confirmed_tags 写之前）新增合并逻辑：
1. 解析 LLM 输出的 discardedTags（新增 `parse_discarded_tags(&value) -> HashSet<String>`，取 value 字段）。
2. 合并集 = `reconfirmed`（本轮重判保留的）∪（`contact.confirmed_tags` 旧标签中**不在 discardedTags** 的）。即：旧确信标签，除非被 LLM 显式列入 discardedTags 推翻，否则保留。
3. `$set confirmed_tags: 合并集`。

对称 core_facts 的保护语义（prompts.rs:1461「系统自动保留未显式弃用的旧事实」）。

**等价性**：DEFAULT 无变化前提——若 LLM 把所有不再支撑的旧标签都放进 discardedTags（prompt 已如此要求），合并集 == 原 reconfirmed。差异仅在「LLM 遗漏显式弃用」时：旧行为静默丢、新行为保留（更符合「不因证据滚出窗口而丢持久标签」的意图）。manual_tags（运营权威层）单键隔离不受影响（:1610 `$set` 只碰 confirmed_tags）。

### A-01：候选证据弱 → importance 天花板

**缺陷**：memory_candidates→core_facts 通道无证据锚定门。`validated_memory_candidate`（memory.rs:1897-1913）只要求 evidence 是非空字符串（:1900 `doc_string(...,"evidence")?`）、importance/confidence>0（:1903），均 LLM 自报；`decide_candidate_status`（:1887-1895）write_score≥6 或 max_importance≥8→pending。core_facts 落库路径（compact_memory_card_with_dimensions:373-480）**无 resolve_evidence**。对比 confirmed_tags（parse_reconfirmed_tags:1086-1090）/personality（parse_facet:1123-1129）都强制 resolve_evidence、证据空即 fail-closed。

**为何不用重方案**：候选 prompt schema（prompts.rs:1318）的 evidence 是**自由文本**「来自用户哪句话或哪个行为」，**无 evidenceTurns 序号**；resolve_evidence（tag_evidence.rs:11）签名 `(window, turn_indices)` 依赖序号。给候选加 evidenceTurns 需改 prompt schema + 落库 + 消费，改动大且可能扰动已调好的记忆行为（过拟合风险）。用户裁定取轻方案。

**修复（轻方案·importance 天花板）**：在 `validated_memory_candidate`（:1897）加约束——evidence 文本过弱（trim 后长度 < 阈值，如空或极短）时，对该候选的 importance 设天花板（clamp 到 < IMPORTANCE_RESCUE_THRESHOLD=8），使其无法凭 max_importance≥8 走 pending 救援通道（decide_candidate_status:1890）。即：LLM 自报高 importance 但拿不出实质 evidence 文本的候选，不享受高分快速通道，仍需 write_score≥6 常规通道。

**阈值定为纯函数 + 常量**，便于单测、反过拟合（不对单条对话点修）。阈值取值在实现时定（保守：evidence.trim().is_empty() 或长度 < 一个小常量如 4 字），作为「弱证据」的抽象判据，不针对具体样本。

**等价性**：evidence 充实（正常运营记忆都有原话）的候选完全不受影响；仅"空/极短 evidence + 高自报 importance"的噪声候选被降级。

### PR-A 测试

测试重点是把三条的核心判定抽成纯函数，用 lib 单测覆盖（快、无 Docker、进 baseline 计数、符合本地验证条件）：
- **A-02**：核心逻辑抽为纯函数 `merge_confirmed_tags(old, reconfirmed, discarded) -> Vec<...>`，lib 单测覆盖：旧标签不在 discarded 则保留、在 discarded 则移除、reconfirmed 覆盖同名、空 discarded 时等价于旧 replace 行为（DEFAULT 等价）。这是 A-02 的可测核心。
- **A-01**：`validated_memory_candidate` 的 importance 天花板逻辑抽为纯函数（或直接对该函数补单测），lib 单测覆盖：弱 evidence（空/极短）+ 高自报 importance → importance 被 clamp 到 < 8；充实 evidence → importance 不变。
- **A-03**：是控制流改动（confirmed_tags 写 `?`→fail-soft warn），与同段 personality 写完全对齐。DB 错误注入的集成测试成本高、收益低；**不强求独立测试**——正确性由「与 personality 写同构」+ code review 保证，A-02 的纯函数测试已覆盖同一落库段的行为。这是防御性 fail-soft，不引入新分支逻辑。

---

## PR-C：通用化底座两处半接线

C-01/C-02 互不牵连，同 PR 因同主题（domain profile 接线）。

### C-01：stagnation 写侧动态化

**缺陷**：DomainProfile.stagnation_dimension 让行业指定"哪个维度驱动 planner 停滞计时"。读侧已全动态（planner/mod.rs:1022-1023 `format!("domain_attributes.{dim}_updated_at")`），写侧写死（domain_signals.rs:148-149 只写 `customer_stage_updated_at`）。配非 customer_stage 停滞维度的行业，计时永远跟踪 customer_stage。

**修复（只动 AI 路径，wrapper 传 None）**：
1. 内核 `insert_domain_signal_values`（domain_signals.rs:128）签名加 `stagnation_dimension: Option<&str>`；`stage_changed` 时写 `domain_attributes.{stagnation_dimension.unwrap_or("customer_stage")}_updated_at`。
2. AI 决策路径 gateway.rs:4128-4133：`active_profile` 已在 :3923 载入，传 `Some(active_profile.stagnation_dimension.as_str())`。
3. admin 直写 wrapper `insert_domain_stage_fields`（shared.rs:106）：传 `None`（保持现状，admin 路径不载 active_profile，避免波及其 8 个调用点）。

**等价性**：DEFAULT `stagnation_dimension="customer_stage"`（domain_profile.rs:791 亲验）→ `Some("customer_stage")` 与 `None` 都写 `customer_stage_updated_at`，销售域字节等价。非默认行业的 AI 路径才写对应维度时间戳。

**测试**：`insert_domain_signal_values` 纯函数单测（lib）——传 `Some("relationship_closeness")` 写 `relationship_closeness_updated_at`、传 `None`/`Some("customer_stage")` 写 `customer_stage_updated_at`。

### C-02：初始画像接 dimension/memory guidance

**缺陷**：build_initial_operation_profile（decision.rs:69-77）只接 active_profile.prompt_fragment，未接 live reply 路径已有的 render_memory_candidate_types_guidance（decision.rs:654-656）+ render_decision_dimensions_guidance（decision.rs:679-686）。非销售域声明的 profile_dimensions 建档时不被采集（首条 inbound 后 live reply 才自愈）。

**修复**：
1. 追加 `render_memory_candidate_types_guidance`（domain_profile.rs:134，只需 `&[MemoryDimension]`，从 active_profile.memory_dimensions 现成可得）到初始画像 user prompt 组装处。
2. `render_decision_dimensions_guidance`（domain_profile.rs:1182，需 `dimensions, scope_account_id, cache`）：build_initial_operation_profile 签名（decision.rs:48-53）只有 workspace_id、无 account_id。调用点（contacts.rs:739 等 5 处）有 task.account_id。**改签名穿 account_id + 载 taxonomy_cache**，波及 5 个调用点。

**等价性**：两渲染函数 DEFAULT 均返空串（domain_profile.rs:134 DEFAULT 空、:1182 DEFAULT extra 空→空串），销售域首屏 prompt 字节不变。非默认行业才注入本行业维度指引。

**测试**：两渲染函数已有 DEFAULT 空串守卫（若无则补 lib 单测）；集成层验证初始画像 prompt 在非默认 profile 下含维度指引（`#[ignore]` Docker，或纯函数验证 prompt 片段拼接）。

---

## PR-E：post_release 5 闸映射对调 + pressure 口径

**缺陷（subagent 精确验证，修正台账「三文件三向不一致」的夸大）**：真相是 threshold.rs:67-68 与 significance.rs:53-54 **彼此一致**，只有 post_release.rs:55-56 把 fact_risk_block/pressure_risk_block 两行的 status **对调**：
- post_release：fact_risk_block→blocked_by_safety_guard、pressure_risk_block→held_by_ai_policy
- threshold/significance：fact_risk_block→held_by_ai_policy、pressure_risk_block→blocked_by_safety_guard

**生产真相**：fact_risk 硬闸→`held_by_ai_policy`（gates.rs:873-875 + 测试 finalize_keeps_hard_gate_failure_in_held）；`blocked_by_safety_guard` 只来自产品声明 fail-closed（gates.rs:450-451）+ relay 泄漏（gateway.rs:2581）。故 **threshold/significance 为对，post_release 错**。

compute_window_metrics（post_release.rs:309-322）用这份错映射查生产真实 final_review_status 计数填面板 delta → admin 看到的 fact_risk/pressure_risk 命中率变化贴反标签。纯观测不反哺 promote/rollback（post_release.rs:167/220），故 Medium 非 High。

**pressure_risk 更深问题**：pressure_risk 是**软闸**（gates.rs:160-173 走 soft_risks→revision，成功=revision_applied_approved / 失败=revision_failed），生产侧**不产任何 block 终态**。故 pressure_risk_block 映射到任何 block status 都对不上，其"命中率"本质无意义——光对调不解决根因。

**修复（对调修正 + pressure 改 revision 口径）**：
1. **fact_risk_block**：post_release 复用 significance.rs:52 的权威 pub 常量 `SAFETY_GATE_BLOCK_STATUS` / `safety_block_status_for()`（:59），修正 fact_risk_block→held_by_ai_policy、product_accuracy_score_block→blocked_unverified_product_claim（本就对，保持）。消除对调，三文件统一到一个权威源。
2. **pressure_risk_block**：改口径——不再映射到 block status，改为统计 `revision_failed`（pressure 软闸触发 revision 且失败）或 revision 相关终态，反映 pressure 闸的真实生产终态。若 revision 口径在 post_release 窗口统计中无干净对应，则该 gate 的 delta 明确标注为「软闸无 block 终态」并从 5 闸命中率移除/置 N/A（实现时按 compute_window_metrics 的可得数据定，优先 revision_failed 计数）。

**等价性**：post_release 纯观测，改映射不影响任何自动决策；修复后面板 delta 标签与生产真实语义一致。

**测试**：`post_release.rs` 的 FIVE_GATE_KEYS 映射与 significance 权威源一致性单测（lib）——断言 fact_risk_block 映射 == significance 的映射（钉住三文件不再漂）；pressure_risk 口径的单测按实现方案定。

---

## 落地顺序与验证

1. **PR-A → PR-C → PR-E** 顺序落地（A 最独立、E 最独立，顺序不强制，但逐个 PR 走完 CI 再下一个，避免并发 worktree 爆盘）。
2. 每个 PR 本地必跑（吸取 PR#217 教训 [[config-field-add-test-helpers]]）：`cargo test --lib`（新纯函数单测）+ `RUSTFLAGS="-D warnings" cargo check --tests`（复刻 baseline step2，兜 must_use 等 warning）。有 Docker 时真跑相关 `#[ignore]` 集成测试。
3. 推送用显式 refspec + ls-remote 亲验 tip；PR squash merge 不带 --delete-branch（worktree 铁律）。
4. CI 全绿（Baseline + Integration + 三 lint 门）再合。

## 磁盘纪律

E: 盘紧（修复期间曾降到 1.9G）。已删已合并 PR#217 worktree 的可再生 target 释放 17G。本批在主仓分支 `fix/audit-medium-batch1` 做（复用主仓 target，不起独立 worktree 翻倍占盘）。编译前 `df -h .` 查余量；紧时先删 `target/debug/{incremental,deps,build}`（可再生），绝不碰用户个人数据。
