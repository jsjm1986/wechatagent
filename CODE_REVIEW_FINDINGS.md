# 代码审查问题台账

> 审查基线：`e9ba277`（origin/main，2026-08-04）· 审查日期：2026-08-04
> 方法：源码阅读 + `rg` 双向引用追踪 + 现有测试反证检查。`cargo check --all-targets` 在 `-Dwarnings` 下干净通过。
> 复核补充：当前工作区已运行 `cargo test --lib`，结果为 **2,351 passed / 0 failed**，4 组 PBT 为 **41 passed / 0 failed**；前端 Vitest 为 **125 files / 616 tests passed**，production build 通过；Rust `-D warnings` 全目标检查、Evolution 隔离与治理门均通过。CI 定义的 Knowledge evidence 5 条及 Tenant isolation 3 条 Docker/testcontainers hard gate 已在本机真实执行并全部通过。
> 全量 ignored soft suite 在首次全目标链接期间使 `target/` 膨胀至约 34 GiB、磁盘逼近写满，已主动中止并用 `cargo clean` 恢复；因此不能记为全量 integration 通过。真实 LLM/MCP/微信链路、GitHub Actions 和生产迁移演练仍未执行，本台账不等同于部署验收。
>
> **2026-08-13 处置追记（优化工程四波后核对）**：本台账既有条目状态经四波（线 A–E + S5）复核
> 无回退。两条相关新事实：①「已验证正确的关键红线」表中"自动发布不可绕过"一行引用的
> `auto_release.rs:40,44,57` 已失效——`evolution::auto_release` 模块于优化线 C **整体物理删除**，
> 该红线从"编译期 const false 压制"升级为"机制不存在"（管理 API 仍拒写 workspace 子闸 true，
> `src/routes/evolution.rs`）；② 演化器 pressure gate 统计源失真（本台账未单列，见
> `PROJECT_UNDERSTANDING_LEDGER.md` 第五部分缺陷 #16）已终裁修复：`blocked_by_safety_guard`
> 终态不归因任何 gate，pressure 阈值候选停产（`src/evolution/threshold.rs`）。
> 仍开放项如实保留、四波未动：`AUTH-02`、`WK-02`、`S3-18`、`S3-32`；设计选择类
> `S2-07` / `S2-10` / `S3-08` / `S3-11` / `S3-16` / `S3-17` / `S3-26`–`S3-29` 未动；**HC-001 生产
> 凭证轮换 8 项硬门依旧是最高优先运维待办**。基线现状（2026-08-13 S5 终态）：lib 2562 / 四 PBT 41 /
> 前端 750。

## 当前结论

- 本文是统一审查台账；代码事实、缺陷定性、设计选择和部署限制分开标记。
- 本轮已关闭全部已确认的直接功能缺陷：阈值演化、知识审核/修订链、领导授权知识、Taxonomy 缓存窗口、管理发送结果、导入预算与分段、发布 cooldown 等均已修复。
- 同时完成低风险纵深加固：知识统计写入补 workspace scope、ImportJob/AgentCommandRun 状态闭集、Management GET tool-call account scope。
- `S2-07`、`S2-10`、`S3-11/16/17/26/27/28/29` 不是已证实功能缺陷；它们是有意设计、产品取舍或部署约束。
- `D-16` 已由当前工作区 README 修正，保留条目仅用于记录本轮审查闭环。
- **本轮新增（2026-08-04 二次交叉核验批次）**：`MIG-01`（缺 `APP_ENV` 时清库迁移 fail-open，S1）、`OD-1`（送达核验时钟偏移不对称，S2）、`AUTH-01~03`（全局锁死 DoS、定向锁死、session token 明文存储）、`MEM-01/02`（运营权威事实权威度倒挂、blob 检测器对历史 wire 形态失效）、`FE-ENUM-1/2/3`、`S3-31`。这批的每条位置、调用链与反证测试均由我逐一亲验，证据见各条目内的"亲验"小节。
- **本轮明确反驳、不需修改的条目**（记录以免后续被误当缺陷改动）：`MEM-03`（commit marker 泄漏）、`MEM-04`（无 claim filter）、`FE-AUTH-1`（mask 串提交）、`FE-XSS`（渲染安全全量零命中）、`PG`（提示词写回三闸）、`VP`（版本指针一致性）。
- **方法学**：本批采用"先证伪后确认"——每条初判都由独立 agent（不看初判推理）做反证，再由我亲验证据链。结果是**收窄与反驳多于确认**：`FE-ENUM-1` 从 S2 降为 S3、`S1-01(b)` 两次降级、`FE-AUTH-1` 完全反驳。反驳同样留痕。
- **第三批（记忆 / 前端 / 提示词 / 版本指针）**：新增确认缺陷 `MEM-01`（运营手写事实权威度倒挂，S2）、`MEM-02`（`Vec<String>` 形态绕过 blob 检测，S3）、`FE-ENUM-2`（前端闭集守卫测试手抄副本失效，S3）；确认风险 `FE-ENUM-1`、`FE-ENUM-3`。
- **本轮明确被反驳、不需动工的怀疑项**（留档以免后续误改）：`MEM-03`、`MEM-04`、`FE-AUTH-1`、`FE-AUTH-2`、`FE-XSS`、`PG`（提示词三闸）、`VP`（版本指针）。反驳与确认数量相当——这是交叉核验起作用的标志，不是审核不足。
- **方法学**：本批每条 CONFIRMED 都经过一次"反证优先"的独立红队复核（复核 agent 不看初判理由）。红队推翻了我自己的两处定性（`S1-01(b)` 的方向反转、`FE-ENUM-1` 的"主链在途态长期显示英文"），两处均已按红队结论收窄记录。

## 结论标记

| 标记 | 含义 |
| --- | --- |
| ✅ **确认缺陷** | 源码与调用链已交叉确认，行为偏离自身契约或产生错误结果 |
| 🟡 **确认风险** | 代码事实成立，但主要暴露在故障恢复、多副本、扩展或纵深防御场景 |
| 🧭 **设计选择** | 行为是明确、测试覆盖或文档承认的取舍；是否调整需要产品/架构决定 |
| 🧹 **已过时/已处理** | 原说法被后续代码推翻，或当前工作区文档已修正 |
| ⚠️ **待复核** | 尚未完成独立源码核验，不应据此直接动工 |

严重度：**S1** 安全红线或机制整体可信度 · **S2** 功能/数据正确性 · **S3** 一致性、可维护性及较窄风险。严重度与结论标记正交，例如 S2 也可以是 🧭 设计选择。

---

## S1-01 · 阈值演化机制不可信（五类根因）🧹 已修复（当前工作区）

**位置**：`src/agent/runtime.rs:679-689`、`src/agent/review/gates.rs:20-41,126-194`、`src/evolution/threshold.rs:92-180,313-320,371-407`

### (a) 浮点阈值被截断

`ResolvedThresholds::apply_override` 用 `value as i32`（向零截断），而候选步长是 `FIVE_GATE_STEP: f64 = 0.5`，
`ThresholdOverride.value` 存 f64，shadow replay 按精确 f64 评估：

- 收紧 6 → **6.5**，落地成 `6` —— 发布成功、审计齐全、**实际零效果**
- 放宽 6 → **5.5**，落地成 `5` —— **幅度是 shadow 证据覆盖范围的两倍**

从整数基线生成的首轮五闸候选必然落在这两种情况之一；已有半步 override 的后续候选
也可能重新落到整数，因此不能笼统说“每一次发布”都如此。注释写「向下取整」，但没有解决
`f64` 候选与 `i32` 运行时阈值之间的语义不一致。

### (b) `emotional_value_rewrite` 基线错误

`threshold.rs:318` 的无 override 基线写 `5.0`，生产真实默认是 `6`
（`models.rs:4725` `defaults::emotional_value_rewrite_below()`，与 `human_like_rewrite_below` 同为 6）。

危险点：`release_threshold` 的 `current_value == base_revision.value` 一致性校验会**通过**
（两边都取自同一个错误常量）。于是「从 5 收紧到 5.5」发布后经 (a) 截成 `5`，
把线上真实的 `6` **放宽**了一分——方向与意图相反。

### (c) revision 命中率双重计数

`threshold.rs:116-126` 注释声明「每次 revision 算 0.5 给 human_like + 0.5 给 emotional_value」，
代码实际是两个独立的 `*c += 1`。两个 rewrite 闸命中率被推向 1.0（合理带 0.18），
持续产出虚假收紧提案。

### (d) 三个低分闸的候选调整方向相反

生产判定并非注释所称“五闸都是 `score >= threshold` 命中”：

- `hallucination`、`pressure_risk`：`score >= threshold` 命中；
- `human_like`、`emotional_value`、`knowledge_grounding`：`score < threshold` 命中。

`generate` 与 `decide_candidate` 却对五闸统一使用“命中率低则阈值减 0.5，命中率高则加 0.5”。
该规则只适用于前两类；后三类应反向。因此 `human_like_score_rewrite`、
`emotional_value_rewrite`、`product_accuracy_score_block` 会根据证据朝错误方向演化。
现有候选方向单测主要覆盖 `fact_risk` / `pressure`，没有覆盖三种 `< threshold` 语义。

### (e) planner 命中率由无样本的零值伪造

`hit_counts` 从 `THRESHOLD_REASONABLE_BANDS` 的全部 key 初始化为零，候选循环也包含
`planner_block_rate_threshold`，但 run 扫描从未给该 key 计数。于是有 cohort 时它被当成
“观测命中率 0”并产生候选。`auto_release.rs` 的独立统计反而正确地把 planner 记为 `None`
并拒绝自动发布，说明候选生成阶段的 0 不是观测值。planner 的注释还声明其方向与五闸不同，
但实现继续走同一 `current_value - step` 分支。

**建议**：先建立每个 gate 的单一元数据表，至少声明 comparator/direction、数值类型、默认值、
命中信号来源和合理区间，再让 candidate、shadow、release、runtime resolve 共用。五闸应统一整数全链
或迁为 f64 全链，release 边界拒绝不可表示值；planner 在没有真实观测源前不得生成候选。
补齐五个 gate 的方向测试、无样本测试及 release→resolve 端到端测试。

---

## S1-02 · 「AI 永不自动 verify」内部护栏自相矛盾，当前靠调用方补偿 🧹 已修复（当前工作区）

**位置**：`src/routes/knowledge/mod.rs`（`apply_chunk_integrity`）、`src/routes/knowledge/import.rs:1246-1255,1944-1962`

函数在找到锚点时**直接写 `verified`**。当前两个生产调用方都会在调用后重新覆盖为
`draft + needs_review + confidence=0`，所以现状不是“已经自动 verify”，而是红线依赖脆弱的调用方补偿：

```rust
if has_anchor {
    chunk.integrity_status = Some("verified".to_string());
    chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(90));
    return;
}
```

同一函数下方的注释写着「红线『AI 永不自动 verify』：绝不在此直接 verified」——
**这句注释在描述自己时是错的**。

生产仅两个调用方（`import.rs:1251` / `:1957`），都在调用后三行覆盖回 `draft`/`needs_review`/`0`。
该覆盖是**承载性的**，不是冗余。

**主审补充发现（子代理未发现）**：被当作护栏的单测
`shared_ingest_overrides_client_owned_scope_and_review_state`（`import.rs:2360`）
传入 `source_text="body"` 且请求无 `sourceQuote` → `source_anchor_for_quote` 返回 `None`
→ `has_anchor=false` → **危险分支根本没进去**。该测试验证的是「恶意 payload 声明 verified 被覆盖」，
不是「锚定成功后 verified 被覆盖」。

**后果**：若有人把调用方那三行当冗余删除，现有针对 `enforce_ingest_server_owned_fields`
的单测无法覆盖“quote 成功锚定”分支。仓内另有 preview 层红线测试，但不能替代对这两个生产
apply 调用点的直接回归；该设计仍容易在重构时失守。

**建议**：改为让 `apply_chunk_integrity` 返回锚定结果、由调用方决定生命周期，
不要写了再被覆盖。同时补一个**传可锚定 quote** 的单测覆盖该分支。

**相关**：`ProvenanceSource::PrincipalAuthorized` 文档称可直接带 verified，
经 grep 确认当前**无任何生产调用方**（仅定义 + `as_str` + `FromStr` + 两个单测），是预留。

---

## S2-01 · 管理 Agent 发送结果永久被误报为「待核实」🧹 已修复（当前工作区）

**位置**：`src/routes/management.rs:1689-1707`（`assert_tool_outcome`）、`src/agent/types.rs:1568-1577`

`assert_tool_outcome` 对 `wechatagent.send_contact_message` 读 `response["success"]`：

```rust
let success = response.get("success").and_then(Value::as_bool);
match success {
    Some(true)  => ToolOutcome::Succeeded,
    Some(false) => ToolOutcome::Failed(...),
    None        => ToolOutcome::Unverified("MCP 响应无 success 字段，无法确认是否送达"),
}
```

但该 arm（`management.rs:2047-2069`）返回的是 `ContactSendResult`，
`#[serde(rename_all = "camelCase")]`，六个字段为
`sentContent` / `messageId` / `reviewApproved` / `gatewayStatus` / `gatewayReason` / `decisionReviewId`
—— **没有 `success`**（grep 确认 0 处）。

**后果**：必走 `None` 分支 → 状态永久 `executed_unverified` → 运营每次都看到「⚠️ 已执行待核实」。
单测（`:3629`）构造 `{"success": true, "msgId": "m123"}`，是生产**从不产生**的形状。
这是网关路由落地时漏改的遗留（此前该 arm 直调 MCP）。

失败方向安全（不会误报成功），但**侵蚀了 `executed_unverified` 这个信号本身的意义**：
若每次都是待核实，真正需要核实的那次就淹没了。

**建议**：不要把 `outbox_enqueued` 解释成“微信已送达”。先明确该 management tool 的成功契约是
“发送意图已被生产网关受理并持久入队”，再让结果类型/UI 区分 **accepted/queued** 与 **delivered**：
`outbox_enqueued` 和被既有有效 outbox 覆盖的 `skipped_duplicate` 可判为“已受理/幂等覆盖”，
网关拒绝则按 `gatewayReason` 失败；真正送达只能以后续 outbox/send-ledger 终态确认。
若暂不扩 `AgentToolCall.status` 闭集，也至少把 `executed_unverified` 的原因改成“已入队，尚未获得送达回执”，
不能继续声称“MCP 响应无 success 字段”。单测 fixture 必须改成真实 `ContactSendResult` 形状。

---

## S2-02 · 修订哈希链断裂（verify / reject 两个关键状态转移）🧹 已修复（当前工作区）

**位置**：`src/routes/knowledge/verify.rs:114`、`src/knowledge_wiki/chunk_revisions.rs:359,391`、`src/knowledge_wiki/page_merge.rs:252-260`

verify 的 patch 写入 `verified_claims` / `unsupported_claims`，
但 `OperationKnowledgeChunk` **没有这两个字段**（已枚举确认），struct 也无 catch-all flatten。

链路：

1. `after_hash = compute_chunk_hash(&merged)` —— `merged` 含这些字段，
   且它们**不在** `VOLATILE_FIELDS` 排除集（`_id`/`updated_at`/`provenance`/`usage_stats`/`dynamic_confidence`/`integrity_score`/`id`）→ **被计入哈希**
2. 随后 `from_document::<OperationKnowledgeChunk>(merged)` **静默丢弃**这些字段
3. 落库行不含它们

**后果**：`after_hash` 描述的是一份**从未被持久化的文档**，
下一次修订的 `before_hash` 必然与它不相等。`verified_claims`/`distortion_risks`/`unsupported_claims`
这几个字段涉及的 verify / reject / auto-verify 操作，审计哈希链是断的。

**建议**：要么把这些字段建模到 struct 上，要么从 patch 中移除，
要么加入 `VOLATILE_FIELDS`。三选一，但必须让「被哈希的内容」与「被持久化的内容」一致。

---

## S2-03 · 领导授权知识提案不可见且无审计 🧹 已修复（当前工作区）

**位置**：`src/agent/escalation/ledger.rs:701-714`、`src/models.rs:1867`

`emit_knowledge_gap_proposal` 用裸 `insert_one` 写 chunk，且用 `..OperationKnowledgeChunk::default()`，
而 `Default` impl 的 `domain: String::new()`（空串）。

全仓 **57 处**召回与后台过滤器都要求 `"domain": "user_operations"`
（`knowledge_agent.rs:1108,1288,1956`、`crud.rs:43,606` 等）。

**后果**，这条「领导授权」知识提案：

- 对运营 Agent **不可见**（召回过滤器不匹配）
- 对审核 UI **不可见**（后台过滤器不匹配）
- **无** `chunk_revisions` 审计行（绕过 revision funnel）
- **无** `CatalogRebuildJob` 入队（不进文档目录投影）
- `provenance: None` → 首次后续编辑时 `build_chunk_provenance` 会用**那次编辑的 source** 初始化，
  **永久丢失「源自领导裁决」这一事实**

红线结果本身安全（硬编码 `draft` + `needs_review`），但这条知识等于凭空消失。

**建议**：补 `domain: default_user_operations_domain()`，并改走
`apply_chunk_revision_with_session(op=Create, source=PrincipalAuthorized)`
以获得审计行 + catalog 入队 + provenance。

**相关但未合并定性**：`src/routes/lessons_learned.rs:322` 也是裸 insert 无 revision，
但它手工设置了 provenance，业务契约不同；应作为独立问题复核。

---

## S2-04 · 演化器隔离 lint 只能约束直接文本引用 🧹 已修复（当前工作区）

**原风险**：shell lint 只能发现 `src/evolution` 内的直接禁词，无法约束
`replay -> agent::prompt_shadow -> gateway helper` 的传递依赖，也无法证明 shadow 桥不写业务集合。

**当前修复**：`src/evolution/mod.rs::isolation_contract_tests` 把隔离升级为随 `cargo test`
执行的结构契约：

1. 枚举并锁定全部 evolution Rust 模块；新增模块必须显式进入审核清单；
2. 生产代码禁止依赖 gateway/outbox/MCP/tasks/webhooks 等副作用入口；
3. `crate::agent::*` 只允许经审核的 `domain_profile / prompt_shadow / run_envelope / runtime`；
4. `replay.rs` 的持久写集合只允许 `shadow_replays`，禁止更新/删除源业务行；
5. `agent/prompt_shadow.rs` 桥禁止 outbox/MCP 依赖及 insert/update/delete/replace 写调用。

**验证**：5 项结构测试全部通过；原 shell lint 继续作为快速 CI 补充，但不再是唯一边界。

---

## S2-05 · 锁定发送内容可能被静默截短后锁定 🧹 已修复（当前工作区）

**原风险**：`extract_locked_send_content` 会按“。不要”等自然语言停止词截断正文，再把截短值写成 `originalContentLocked=true`，操作者无法发现原文已改变。

**当前修复**：`src/routes/management.rs::extract_locked_send_content` 改为显式失败语义：

- 引号正文必须闭合，且完整保留引号内文字；正文中的“不要”等词不再被当作控制指令；
- 无引号正文只在无歧义时接受；检测到操作说明分隔符或换行即返回 `BadRequest`，要求用引号明确边界；
- 空正文与未闭合引号均拒绝，不再静默修剪；
- `apply_locked_send_content` 传播解析错误，只有成功解析的完整正文才写入工具参数并锁定。

**验证**：表驱动单测覆盖引号正文完整保留、歧义无引号请求拒绝，以及锁定参数覆盖模型生成内容。

---
## S2-06 · 后台统计刷新会使 chunk 编辑 CAS 令牌失效 🧹 已修复（当前工作区）

**原风险**：feedback worker 重算 `usage_stats` 时同时更新 chunk `updated_at`，会无业务内容变化地作废 Wiki 编辑使用的 CAS 令牌；其绝对值 `$set` 还可能覆盖 worker 读取后发生的热路径 `$inc`。

**当前修复**：`src/knowledge_wiki/gap_signals.rs::usage_refresh_pipeline` 使用 aggregation update：

- 只更新 `usage_stats.*` 与 `dynamic_confidence`，不再触碰内容 CAS 字段 `updated_at`；
- 以 worker 读取到的 observed counters 为基线，保留快照读取后发生的正向 hit/blocked 增量；
- 负 delta 不携带，旧窗口计数仍可随 30 天窗口自然老化；
- `last_used_at` 取单调最大值，热路径的新时间不会被旧快照覆盖。

**验证**：单测覆盖快照后增量保留与负 delta 不携带；全量 Rust 单测通过。

---
## S2-07 · 未配置 DomainProfile 时回落销售默认 🧭 设计选择

**位置**：`src/agent/domain_profile.rs`（`DomainProfileCache::lookup_or_default`）

缓存未命中时返回 `default_domain_profile`，其中包含 `transaction_facts_enabled=true`、销售状态机、
承诺词表和 coverage 维度。源码模块说明、DEFAULT 等价测试和跨域真实模型测试都明确把它定义为
向后兼容的销售基线，而非偶然 fallback。

**残余风险**：准备部署非销售域但漏激活 profile 时，系统会继续以销售基线运行；当前没有明显的
运行时信号区分“有意使用 DEFAULT”与“配置遗漏”。DB 加载错误本身会向上传播，并不会静默回落。

**建议**：是否告警应由产品决定。若支持非销售生产部署，可在 workspace 明确声明目标行业后，
仅对“声明非默认但无 active profile”告警；不要对合法 DEFAULT 销售部署持续刷 warn。

---

## S2-12 · 知识导入路径没有 LLM run 预算与专用总量上限 🧹 已修复（当前工作区）

**原风险**：同步预览与异步 worker 均在 `RUN_BUDGET` scope 外执行，导入总字符数、段数和 LLM 调用总量没有业务硬上限。

**当前修复**：`src/routes/knowledge/import.rs` 为两条入口共用同一组强约束：

- `IMPORT_MAX_TOTAL_CHARS = 200_000`；
- `IMPORT_MAX_SEGMENTS = 64`；
- `IMPORT_SEGMENT_HARD_MAX_CHARS = 5_000`；
- `IMPORT_RUN_TOKEN_BUDGET = 600_000`；
- LLM call 上限按“有效段数 × 每段契约尝试上限”计算，并在 `agent::RUN_BUDGET.scope` 内执行。

校验发生在同步预览、异步 job 创建/执行等入口；预算耗尽仍走结构化 `AppError`，不会被伪装成普通空知识结果。

**验证**：单测覆盖超总字符、超段数、无空行超长文本硬切和每段硬上限；全量 Rust 单测通过。

---
## S2-08 · 超长单段落文档仍会产生超硬上限分段 🧹 已修复（当前工作区）

**原风险**：按标题与空行分段后，超过 5,000 字且没有空行的单段落仍可能完整进入一次 LLM 请求，名义 hard max 未被强制执行。

**当前修复**：`split_oversized_by_paragraph` 对仍超限的段落调用字符级 `split_by_char_limit`，按 Unicode `chars()` 切分而非字节切分；`validate_import_content` 再断言所有产出段均不超过 `IMPORT_SEGMENT_HARD_MAX_CHARS`。

**验证**：中英文/无空行长文本测试均断言每段不超过 5,000 字符，同时保持原文顺序与 UTF-8 完整性。

---
## S2-09 · 导入任务故障恢复会从头重复 LLM 抽取 🧹 已修复（当前工作区）

**原风险**：claim 粒度是整个 job，进程崩溃或 ownership 丢失后会从第 0 段重新调用 LLM；
fencing 只能保护终态，不能保护已发生的模型成本。

**当前修复**：异步导入按 `(job_id, segment_index)` 保存成功段 checkpoint：

- checkpoint 同时绑定 `workspace_id` 和包含 schema/source/index/content 的 SHA-256，内容变化不会误复用；
- 只有结果成功持久化后才计入 succeeded/progress；恢复时先装载匹配 checkpoint，只对缺失段调用 LLM；
- 唯一索引防止同段重复实体，`expires_at` TTL 为 48 小时兜底清理；
- 只有 job 终态 owner CAS 成功后才主动删除 checkpoint，陈旧 worker 无权清理新 owner 的恢复数据；
- 同步与 worker 路径共用 200,000 字符、64 段、每段 5,000 字符和 run budget 上限。

**残余语义**：部分段业务抽取失败仍按既有设计生成 partial `completed` 报告，见 S2-10；
checkpoint 只缓存成功段，不把失败永久固化。

---

## S2-10 · 部分成功导入使用 `completed` 终态 🧭 设计选择

**位置**：`src/import_worker.rs:283-297`

**事实**：`run_job` 把 `Ok(value)` 一律映射为 `status:"completed"`，无论多少段失败。
诚实机制在 payload 里：`importReport {totalSegments, succeeded, failed}`
（`import.rs:511-515`），前端 `steward.tsx:1009-1011` 渲染非阻塞横幅
「共 N 段，其中 M 段抽取失败，下方仅为成功段内容，可能不完整」。

**评价**：代码注释明确采用“全成/部分成 = completed、全失败 = failed”语义；payload 有成功/失败计数，
前端在 apply 前突出显示不完整警告。因此不能把 `completed` 直接定性为状态说谎。

**残余风险**：D2 锚定跑的是**完整原文**，而 chunk 只来自成功段。锚定本身仍然正确，
但报告只给数量，不含失败段范围或摘要；管理员无法准确定位缺失内容。

**建议**：把失败段的字符区间记入 `importReport`，让运营知道缺口位置。

---

## S2-11 · `feedback_worker` 无租约，副本数会放大扫描与竞争 🧹 已修复（当前工作区）

**原风险**：多副本会对同一 workspace 重复做全量扫描；离线 `$set` 与热路径 `$inc` 竞争时，
worker 读取后发生的命中增量可能被覆盖。

**当前修复**：

- 每个 workspace 先原子领取 `background_worker_leases` 中的 300 秒 lease，token 作为 fencing 身份；
- 60 秒 heartbeat 续租，续租失败设置 cancellation，后续 lint/sweep/lessons/reviewer 阶段停止；
- 一轮结束仅由 token owner 释放 lease，不同副本可并行处理不同 workspace；
- usage refresh 改为 aggregation pipeline：以 worker 读取时的 observed counter 为基线，保留写回前热路径产生的正增量，同时允许旧窗口数据自然老化；
- usage refresh 不再改 chunk `updated_at`，不会使内容编辑 CAS 令牌失效。

**验证**：lease identity/token 与 heartbeat 间隔有纯函数测试；并发更新 pipeline 有单元测试覆盖。

---

## 第二批：迁移 / 出站幂等 / 认证面（2026-08-04 补充审核）

本批为迁移 runner、outbox 送达核对、认证限流与会话四个此前未覆盖的面。
每条均由本轮亲自 Read 核对，证据行号来自当前工作区。

## MIG-01 · 破坏性迁移的生产审批闸 fail-open 🧹 已修复（当前工作区）

**原风险**：`APP_ENV` 缺失或未知时被当作非生产，m011/m012/m014/m035 可绕过 `APPROVED_MIGRATIONS` 执行破坏性清理。

**当前修复**：`destructive_migrations_require_approval` 默认 fail-closed；缺失、空值、`production`、`staging` 和未知值均要求审批。只有显式 `development/dev/test/local` 才允许无审批运行破坏步骤；`.env.example` 已同步该语义。

**验证**：纯函数测试覆盖缺失/空值/未知环境与四种显式本地环境。

---

## OD-1 · 权威送达核对的时钟容差不对称 🧹 已修复（当前工作区）

**原风险**：文本送达恢复的权威 `chat_search` 从 `entry.created_at` 精确起查且只取 20 条；远端时钟落后或窗口拥挤可能产生假阴性并触发重发。

**当前修复**：`chat_search_outbound` 与本地成功日志统一使用 5 分钟负向时钟容差，服务端查询和本地精确内容判定共用容差后的下界；查询上限由 20 提升为 100，仍保留内容精确相等判据。

**验证**：纯函数测试锁定 5 分钟容差和扩大后的查询上限；既有精确内容/时间边界测试继续通过。

---

## AUTH-01 · 历史失败占满全局登录闸造成全站拒绝服务 🧹 已修复（当前工作区）

**原风险**：全局容量统计窗口内全部失败记录，随机客户端/用户名可累计占满 100 个槽，随后任何管理员登录都在 Argon2 前被拒。

**当前修复**：全局容量只统计 `Pending` 请求，语义改为保护 Argon2 并发；历史 `Failed` 记录只继续参与 client/target 抗爆破维度，不再占用全局槽。

**验证**：测试证明随机历史失败不再填满全局容量，同时两个并发 pending 仍能触发配置为 2 的全局上限。

---

## AUTH-02 · 针对单一用户名的定向锁定 🟡 确认风险（S2）

**位置**：`src/auth/rate_limit.rs:119-125`、`config.rs:728-730`

**事实**：`target_capacity` 默认 **10**（`AUTH_RATE_LIMIT_TARGET_CAPACITY`），维度键是
用户名（`begin_at:103-105` 对 `target.trim().to_ascii_lowercase()` 取指纹）。
攻击者知道管理员用户名即可用 10 次失败尝试把该账号锁死 5 分钟；
合法用户无法自救，因为 `mark_success` 的清理位于 `begin` 成功之后，
而其请求在 `begin` 阶段即被拒。

**定性说明**：per-target 限流本身是对抗密码喷洒的标准做法，单独看属合理取舍；
列为风险是因为它与 AUTH-01 叠加后使「全站锁定」的成本进一步降低，
且当前无白名单/受信来源旁路。与 AUTH-01 同一排期项。

---

## AUTH-03 · 会话令牌明文存储 🧹 已修复（当前工作区）

**原风险**：cookie bearer token 原文存入 `admin_sessions.session_id`，数据库只读泄漏即可直接冒充管理员。

**当前修复**：cookie 继续持有 UUIDv4 原文，Mongo 只保存 `sha256-v1:<hex>`；lookup、logout 和 workspace 切换按摘要定位。升级前明文行仍可使用，首次成功 lookup 时透明迁为摘要；摘要查询优先，登出同时清理摘要与兼容明文候选。

**验证**：纯函数测试覆盖摘要稳定性、无原文泄漏和兼容 filter；现有 cookie API 不变。

---

## MEM-01 · 运营手写权威事实在 coreFacts 淘汰排序中权威度最低 🧹 已修复（当前工作区）

**原风险**：`human_profile_note` 与 `manual_tags` 被包装为无来源的 `Plain` fact，容量排序权威度为 0，可被带证据的模型事实挤出 6 条 coreFacts 窗口。

**当前修复**：联系人种子事实改为 Structured；运营备注/手工标签标记 `extra.source=operator_manual` 并获得最高权威档，confirmed tag 使用独立的次高来源档。滚动 `memory_summary` 仍不进入权威 coreFacts。

**验证**：真实 7 条候选压缩到 6 条的容量测试证明运营事实保留、低权威模型事实进入 recent；另有来源分值测试。

---

## FE-ENUM-1 · 前端 gateway 状态字典缺 6 个后端闭集值 🧹 已修复（当前工作区）

后端 38 个 `GATEWAY_STATUS_VALUES` 已全部有中文标签；本轮补齐 `outbox_enqueuing / outbox_enqueue_failed / outbox_enqueue_partial_failure / stale_task_claim / skipped_duplicate / internal_error`。

---

## FE-ENUM-2 · 前端闭集守卫测试手抄副本失效 🧹 已修复（当前工作区）

Rust 契约测试现在把 `GATEWAY_STATUS_VALUES` 对账到 `frontend/src/contracts/gateway_status_values.fixture.json`；Vitest 直接读取该 fixture 验证每个值都有非原值标签，不再维护手抄数组。后端新增/删除状态而未同步 fixture 或前端标签都会使 CI 失败。

---

## FE-ENUM-3 · `delivery_finalizing` 与 gateway 闭集边界不清 🧹 已澄清并加固（当前工作区）

`delivery_finalizing` 是 `decision_reviews` 的短暂发送终态对账锁，不是 `agent_run_logs.gateway_status`。Rust 与 Vitest 均增加边界测试，明确它不得进入 gateway fixture/标签字典；gateway 的 38 值继续走严格闭集校验。

---

## FE-AUTH-1 · LLM 供应商 mask 串提交 🧹 已核实为非缺陷（前端冗余，后端已正确处理）

**agent 初判**：编辑供应商时 `draftFromItem`（`llm-providers/index.tsx:129`）把
`apiKeyMasked` 灌进草稿，`buildUpsertBody`（`:207`）原样透传，只有 `runTest`（`:348`）
做了 `includes("****")` 剥离，保存路径没有 → 疑似把 mask 字面量写成真实密钥。
定为 PARTIAL，未读后端。

**我的独立核实——反驳成立**：后端两条路径都正确处理 mask。

- 更新：`routes/llm_providers.rs:356-359`，`if is_masked_value(&body.api_key) { existing.api_key.clone() }`
  —— 提交 mask 即沿用旧值，正是 UI 承诺的语义。
- 创建：`:400-404`，提交 mask 直接 `400 BadRequest("apiKey 不能是已 mask 的占位串")`。
- 判据 `is_masked_value`（`:160-161`）就是 `value.contains("****")`，与前端 mask 形态一致。
- 模块头注释（`:16-17`）明确记载了这条契约。

**结论**：前端保存路径缺少剥离是**冗余**而非缺陷，真相源在后端且实现正确。
`:348` 的前端剥离属于双重保险。**不需要修改**，记录此条以免后续被误当缺陷改动。

---

## FE-AUTH-2 · `sessionStorage` 的 `wa.authed` 只写不读 🧹 死状态（非缺陷）

**位置**：`frontend/src/main.tsx:12,19,62,129,132,158`

**事实**：五个写点（含 agent 漏记的 `:132`），**零个读点**。全仓 `getItem` 只出现在
`today.tsx`（`knowledgeChat.sessionId`）与 `accountStore.ts`（`wechatagent.accountId`）。
真实登录态由 `/api/auth/me`（`:125-134`，只看 `r.ok`）与 `wa-auth-expired` 事件驱动，
渲染门是 `if (!me)`（`:165`）。

**为何不是缺陷**：不构成越权——没有代码信任这个标志，可篡改的本地标志无法换取访问。
且 `sessionStorage` 随 tab 关闭清空，注释所称"重启 tab 也能复现"本就不可能成立，
说明这是过期文字而非"描述了却坏掉的机制"。风险只在于注释误导后续改动。

**建议**：删掉常量与 5 处写入，或把注释改成事实。零行为影响。

---

## FE-XSS · 前端渲染安全 ✅ 已核查，未发现漏洞

`frontend/src` 全量模式搜索命中数均为 0：`dangerouslySetInnerHTML`、`innerHTML`/`outerHTML`/
`insertAdjacentHTML`/`document.write`、`eval(`/`new Function`、`srcDoc`/`<iframe`、
Markdown/HTML 渲染库（react-markdown / marked / DOMPurify 全无依赖）、`javascript:`/`data:text/html`
字面量、生产代码里的 `setAttribute`/`textContent=` 写入。对照组 `useState` 命中 580 处，证明搜索生效。

LLM 产出的知识 chunk、客户消息、AI 回复全部走 `{value}` 纯文本插值或 `<pre>{json}</pre>`，
由 React 自动转义。已主动排除两处可疑点：CSV 导出有 `safeCsvCell`（`csv.ts:5-13`）做公式注入防护
且有守卫测试；`<img src={avatarUrl}>` 在 img 上下文无法取得脚本执行。

唯一动态 `href` 是 `AccountLogin.tsx:178` 的 `loginPageUrl`，来源是运维自配的 MCP 服务端
（非客户消息 / 非 LLM 产出），已带 `rel="noopener noreferrer"`，判为 🧭 设计选择。
纵深防御建议：渲染前校验协议为 http/https。

**未覆盖**：CSP / 响应头、`frontend/index.html` 与 vite 配置、后端是否有返回 `text/html` 的接口。

---

## MEM-02 · 非原子 blob 检测器漏掉历史 `Vec<String>` wire 形态 🧹 已修复（当前工作区）

`value_has_non_atomic_fact` 现在同时读取字符串项和 `{text: ...}` 对象项；历史 `coreFacts: ["..."]` 中的多句 blob 会进入既有检测→重试→丢弃链。新增纯字符串数组回归测试。

---

## MEM-03 · commit marker 泄漏 🧹 已核实为非缺陷

**初判**：`memory_applied_commits`（`memory.rs:2369,2530` 写入）可能泄漏进对外 JSON。

**我的独立核实——反驳成立**：
- `OperatingMemory` 结构体（`models.rs:1613-1639`）**没有** `memory_applied_commits`
  字段，serde 反序列化时该键被直接丢弃，业务代码永远拿不到它。
- 路由投影 `operating_memory_json`（`routes/shared.rs:1233-1248`）是**显式白名单**，
  逐键列出 11 个字段，不含任何 commit marker。
- 唯一以 `Document` 类型读该集合的地方是 `memory.rs` 内部的
  `clone_with_type::<Document>()`（幂等判定自用），不经路由下发。
- 提交成功后还会 `$pull` 掉 marker（`:2521-2534`）。

**结论**：不可见、不泄漏。**不需要修改。**

---

## MEM-04 · 无 claim 路径的未授权 filter 🧹 已核实为测试专用路径（非生产缺陷）

**事实**：`memory.rs:2012` 的 task 状态回写 filter 只有 `doc! { "_id": task_id }`，
不带 `claim_token` / `claim_generation`，确实不满足仓内"改 task 必须 CAS own"的纪律。

**但该分支生产不可达（已亲验调用链）**：
- 生产入口 `tasks.rs:542` 调 `handle_memory_consolidation_task_with_claim(..., Some(claim))`。
- 管理员手动整理 `run_manual_memory_consolidation`（`memory.rs:2933`，
  由 `routes/contacts.rs:2958` 调用）走 `run_due_task_by_id` →
  `claim_task_with_filter`（`tasks.rs:562`）→ `process_claimed_task`，同样带 claim。
- 带 claim 时在 `:1911-1941` 就 `return`，**根本走不到** `:2012`。
- 剩余 `task_claim: None` 的调用方全部在 `tests/`（`happy_path_run.rs:141`、
  `real_llm_ops_smoke.rs:2133`、`real_llm_adversarial.rs:2144`）。
- 代码自己的注释（`:1943`）写明"无 claim 的直接调用兼容路径不参与任务取消协议"。

**结论**：定性为**测试专用兼容路径**，非生产缺陷。若要收紧，可把
`consolidate_contact_memory` 的无 claim 重载标记 `#[cfg(test)]` 或 `#[doc(hidden)]`，
消除"生产误用"的可能，但当前无实际风险。

---

## PG · 提示词写回三闸 ✅ 已亲验，未发现绕过面（负面结论留档）

**位置**：`src/prompt_guard.rs:33-46,48-67,70-97,116-120`

**机制（三闸、fail-closed）**：
1. **分层闸**（`:48-67`）：`PROMPT_EVOLUTION_FORBIDDEN_KEYS` 与
   `management.prompt_redline_review.system` 归 `Forbidden`，自然语言入口直接拒。
2. **禁用词闸**（`evolution/lint.rs:33-41`）：先 `to_ascii_lowercase` 再逐个
   `contains`，大小写变形无法绕过。
3. **锚完整性闸**（`:88-95`）：强约束层模板的业务锚 **+ 红线锚**必须逐字仍在。
   两侧都过 `normalize_prompt_content` 归一后再比，避免 CRLF 差异误拒合法编辑。

**设计上值得记录的两点**：
- `required_anchors`（`:33-46`）对 `user.reply.policy` 同时校验业务锚
  `DEFAULT_MODE_GATE_POLICY` **和**红线锚 `DEFAULT_REPLY_REDLINE_ANCHORS`。
  注释（`:30-32`）明确记载旧设计只查业务锚、红线被删却能放行——该缺口已修补。
- `extract_diff`（`:116-120`）刻意返回**完整前后快照**而非按行集合 diff，
  注释说明原因：集合 diff 会漏掉纯重排与"删除重复行之一"，只有 CRLF 等价内容
  才允许跳过语义审查。这是正确的保守取向。
- 第三闸 `review_prompt_edit` 三态（`Pass` / `Reject` / `NeedsHumanConfirm`），
  LLM 不可用时既不 fail-closed 死路也不 fail-open 放水。

**两条写 prompt 的路径都过闸（已核对调用点）**：
`routes/prompt_templates.rs:121,164,268` 与 `evolution/release.rs:544,556-562`
均先调 `validate_prompt_edit` 再按 `PromptEditVerdict` 三态分支。**无绕过入口。**

**自我约束值得注意**：模块头注释（`:8-11`）说明本文件在 CI 禁词扫描区内，
故非测试代码绝不内联禁用词字面量，只 import 定义在 `prompts.rs`（扫描区外）的锚常量。

---

## VP · 提示词 / 灵魂版本指针 ✅ 已亲验，机制成立（负面结论留档）

**位置**：`src/prompt_template_versions.rs:189-204,208-373`、`src/soul_versions.rs:238+`

**机制**：`publish_version` 全程在 Mongo 事务内（`start_transaction`），且每步写入
都带 CAS 前置条件：

- 归档旧 current 的 filter 含 `version` + `current_version: true`
  （`:292-298`），`modified_count != 1` → `Conflict("prompt_publish_pointer_changed")`。
- 提升目标的 filter 含 `version` + `status` + `current_version: false`
  （`:318-325`），`modified_count != 1` → `Conflict("prompt_publish_target_changed")`。
- 发布前先跑 `validate_publish_pointer_state`（`:189-204`），三类不一致
  （多个 current / current 非 active / 存在非 current 的 active）全部拒。
- 幂等：目标已是 current 时直接返回（`:280-282`）。
- 提交带 `UnknownTransactionCommitResult` 重试循环（`:361-371`），失败必 abort。

**结论**：并发发布的落败方拿到 `Conflict` 而非静默覆盖，指针不会分叉。
`soul_versions.rs` 的 `publish_version` 结构同源（事务 + 唯一 published 校验）。
**未发现一致性缺陷。**

**未覆盖**：未在真实副本集上做并发注入验证（本地无 Docker/testcontainers），
故结论限于代码层面的机制正确性。

---

## FE-STORE-1 · `enableAgent` 结束后未复位 `guideBusy` 🧹 已修复（当前工作区）

**原风险**：开启托管无论成功或失败都只复位全局 busy，联系人配置区的 `guideBusy` 永久保持 `true`，导致修改预览按钮持续禁用。

**当前修复**：`enableAgent` 的 `finally` 同时执行 `set({ guideBusy: false })`，不依赖联系人切换或重新 hydrate 自愈。

**验证**：Store 测试覆盖成功和 API 失败两条路径，均断言 `guideBusy=false`。

---

## FE-STORE-2 · `inboxStore.load` 迟到旧响应覆盖新筛选 🧹 已修复（当前工作区）

**原风险**：多个来源筛选请求并发时，旧请求迟到后无条件覆盖新筛选结果，界面筛选与实际列表不一致。

**当前修复**：Inbox store 增加单调 `requestGeneration`；每次 load 捕获 generation，成功和失败分支均只允许当前代际提交状态。

**验证**：deferred 并发测试让新筛选先完成、旧请求后完成，最终列表保持新筛选结果且 loading 正确收口。

---

## FE-STORE-3 · 请示待办计数双真相源 🧹 已修复（当前工作区）

**原风险**：`/inbox` summary 的 `principalEscalation` 同时保存在 Inbox Store 与
UserOps Store；请示频道裁决刷新后，驾驶舱仍可能显示旧计数。

**当前修复**：`inboxStore.summary` 成为唯一事实源，并提供独立、带
`summaryRequestGeneration` 的 `refreshSummary()`；联系人详情只触发该刷新，驾驶舱直接订阅
`summary.counts.principalEscalation`。UserOps Store 的重复字段、动作和网络调用已删除。

**验证**：Store 并发回归让新 summary 先完成、旧 summary 后完成，最终计数保持新值；
TypeScript/build 负责锁定驾驶舱消费链。

---

## WK-01 · `cold_contact_worker` 单条失败中断 workspace 扫描 🧹 已修复（当前工作区）

**原风险**：`has_pending_follow_up` 或 `commit_follow_up` 的单 contact 错误经 `?` 上抛，中断该 workspace 剩余联系人并跳过 tick 审计。

**当前修复**：两处错误均按 contact 记录 warning、递增 `failed` 并继续扫描；成功、重复和配额耗尽语义保持不变。`cold_contact_tick` summary/details 暴露 `failed` 数。

**验证**：纯 details 测试锁定 `scanned/emitted/failed` 三项审计字段。

---

## WK-02 · 两个 worker 均无租约，但重复触达被提交层挡住 🟡 确认风险（S3）

**位置**：`src/cold_contact_worker.rs`、`src/silence_signal_worker.rs`（全文无租约代码）、
`src/proactive_outreach.rs:120-130,281-297`

**事实**：两个 worker 都是裸 `loop { scan; sleep }`，无 lease / claim_token /
generation 栅栏（对照 `knowledge_wiki/ingest_worker.rs` 与 `catalog_rebuild` 均有）。

**为何不造成重复触达（agent 的反驳成立，我采纳）**：幂等不靠租约，靠提交层——
`intent_identity`（`:120-130`）对 `(segment, workspace, account, wxid, subject)`
做 sha256 得**确定性 `ObjectId`**，作为 `agent_tasks._id` 在事务内 `insert_one`
（`:292-297`）；输家撞主键唯一约束 → 事务 abort → 归类 `Duplicate`。
日额度也在同一事务内用 `$lt` 守卫 + `$inc` 原子预留，**不是先读后写**，
故循环外只算一次的 `already_emitted_today` 陈旧值不参与放行判定。

**实际影响**：对客户无影响。代价是多副本下 MongoDB 读放大 N 倍、事务冲突重试
增多，且审计流同一时刻出现 N 条 tick 事件易被误读。与 `S2-11` / `S3-23`
同属"worker 租约"排期项，不应重复立项。

---

## WK-03 · `silence_signal_worker` 提交失败计数不可见 🧹 已修复（当前工作区）

**原风险**：单 contact 错误虽会继续扫描，但 tick 只记录 scanned/emitted，运维无法从审计事件识别持续漏采。

**当前修复**：提交错误递增 `failed`，tick summary/details 同步记录失败数；既有 signal metric 与下轮重试语义不变。

**验证**：纯 details 测试断言失败计数进入审计文档。

---

## 待独立复核队列（⚠️ 尚未逐条亲验，不得据此动工）

以下条目来自本轮多 agent 扫描，为**保留留痕**而记录，但尚未完成本台账要求的
源码/调用链/反证测试三项亲验，故不给出缺陷定性。升级前必须补齐证据。

| 队列项 | 涉及范围 | 待验证的核心断言 |
| --- | --- | --- |
| ~~Q-01~~ | `src/agent/memory.rs` | **已亲验 → `MEM-01`**（结论：不是"被覆盖"，是权威度排序倒挂致 cap 淘汰） |
| ~~Q-02~~ | `src/agent/memory.rs` | **已亲验 → `MEM-02`**（`Vec<String>` 形态绕过 blob 检测，成立） |
| ~~Q-03~~ | `src/agent/memory.rs` | **已亲验 → `MEM-03`**（反驳：结构体无该字段 + 路由白名单投影） |
| ~~Q-04~~ | `src/agent/memory.rs` | **已亲验 → `MEM-04`**（反驳：生产必带 claim，该分支仅测试可达） |
| ~~Q-05~~ | `src/prompt_guard.rs` | **已亲验，未发现绕过面**（见下方 `PG · 提示词写回三闸`） |
| ~~Q-06~~ | `src/prompt_template_versions.rs`、`soul_versions.rs` | **已亲验，机制成立**（见下方 `VP · 版本指针`） |
| ~~Q-07a~~ | `frontend/src/**` | **已亲验**：渲染安全（`FE-XSS`）、enum 漂移（`FE-ENUM-1/2/3`）、认证面（`FE-AUTH-1/2`） |
| ~~Q-07b~~ | `frontend/src/stores/**` | **已亲验 → `FE-STORE-1/2/3`**（两条 S2 成立：`guideBusy` 不复位、inbox 无请求代际） |
| ~~Q-08~~ | `src/cold_contact_worker.rs`、`silence_signal_worker.rs` | **已亲验 → `WK-01/02/03`**（扫描中断成立但半径小于原报告；重复触达被反驳） |

**队列已清空**——本轮全部 8 个待复核项均已完成亲验并给出定性。剩余未审边界见文末「审核边界」。

---

## S3 · 其他一致性问题

| # | 问题 | 位置 | 等级 |
| --- | --- | --- | --- |
| S3-01 | 70% 截断保护取 `max(body, summary)`；较长 summary 可掩盖 body 的异常缩短，字段级不变量未被保护 | `chunk_revisions.rs:840` | 🧹 已修复（当前工作区） |
| S3-02 | `AgentCommandRun.status` 没有与 `AgentToolCall.status` 对等的闭集常量和写入断言；当前写入均受内部流程控制，但未来新增写点易漂移 | `models.rs:3879` | 🧹 已修复（当前工作区） |
| S3-03 | plan 执行用 `.take(12)` 静默截断，而汇总仍参考完整 `plan.tool_calls`；没有在计划生成/确认边界拒绝超长计划或向操作者提示 | `management.rs:211` | 🧹 已修复（当前工作区） |
| S3-04 | seeded policy 同时提到可用网关工具和已被三重封锁的 `message_send_text`，会诱导模型生成必然被拒的计划；不是每个计划必败，但形成可避免失败路径 | `prompts.rs:1733` | 🧹 已修复（当前工作区） |
| S3-05 | `running` 命令没有周期清扫器；5 分钟过期只在再次 confirm 时消费，进程死亡且无人重试时状态会长期滞留 | `management.rs:94,801` | 🧹 已修复：management_command_sweeper 回收孤儿，执行侧 60s heartbeat + token fencing |
| S3-06 | GET command 只按 `_id + workspace_id`，不像变更接口再加 account scope。管理员权限当前是 workspace 级，未证实 IDOR；这是边界一致性/纵深防御缺口 | `management.rs:978-1027` | 🧹 已修复：GET command 的 tool-call 查询补 account_id scope |
| S3-07 | 默认锁字段表含多个模型上不存在的历史名；尤其 `source_anchor` 单数与真实 `source_anchors` 不同。后者可编辑是 chat 更新锚点所依赖的明确设计，因此问题是策略表和文档误导，不能简单改成复数锁死 | `page_merge.rs:35-46`、`chat.rs:2845-2861` | 🧹 已修复：锁字段统一为真实 BSON 身份字段，source_anchors 明确受控重算 |
| S3-08 | auto-verify 强制降为 `needs_human_audit` 后，响应中的 `verified=0`、`review_approved=false` 是当前政策结果；字段名容易被仪表盘误读，但未证明状态迁移错误 | `verify.rs:474,503,610` | 🧭 设计选择 |
| S3-09 | `coerce_integrity_against_d2_gate` 仅在 `#[cfg(test)]` 编译，旧注释却把它描述为生产后门防护 | `knowledge/mod.rs::coerce_integrity_against_d2_gate` | 🧹 已澄清：它是旧请求形态回归 helper；生产 CRUD 无条件强制 `draft + needs_review`，只有 `/verify` 可进入 `active + verified` |
| S3-10 | `plan_requires_confirmation` 的 `_dangerous_confirm_enabled` 参数未参与逻辑，但调用方仍传不同值，制造一个不存在的策略开关 | `management.rs:1757` | 🧹 已修复（当前工作区） |
| S3-11 | 未配置 taxonomy kind 时接受原值是有注释、有测试的兼容策略，用于避免空字典阻断写入；是否改成 fail-closed 是产品治理决策 | `dimension_registry.rs:140-167` | 🧭 设计选择 |
| S3-12 | admin taxonomy 写后 `invalidate()` 会清空缓存；决策路径同步调用纯 `check_value` 而不 `find_or_load`。在其它路径重新加载前，已知标签会被误判为 `CandidateNew`；“下次 check_value 自动重载”的注释不成立 | `taxonomy.rs:126-132`、`decision_taxonomy.rs:83-102` | 🧹 已修复（当前工作区） |
| S3-13 | 时间预筛已动态使用 `<stagnation_dimension>_updated_at` 并回落旧时间戳；剩余耦合是候选必须有 `domain_attributes.customer_stage`，内存和排序也继续读取 sales stage | `planner/mod.rs:1191-1246,1365-1376` | 🧭 设计选择：`stagnation_dimension` 的声明语义只覆盖**计时维度**（`models.rs:2204-2208` 字段注释明载），不覆盖取值；非阶段行业按设计经 `funnel.enabled=false` 关闭本段（`planner/mod.rs:1554` 按 **contact 粒度**短路，3 个测试锁定）。真正的机制缺口已单列为 `S3-32` |
| S3-14 | `evolution_min_self_critique_delta` 仍被配置解析，但显著性评分不消费它；这是无效果配置，相关运维文档不应再暗示它控制发布 | `config.rs:228,632`、`evolution/significance.rs` | 🧹 已修复（当前工作区） |
| S3-15 | shadow 使用 `approved_after_revision`，生产闭集使用 `revision_applied_approved`；`significance` 仍把前者计成功，而 post-release 使用后者，前后指标口径不可比 | `replay.rs:343,462`、`significance.rs:42`、`post_release.rs:36` | 🧹 已修复（当前工作区） |
| S3-16 | 前端 label helper 对未知 enum 值回落原字符串。该策略保持前向兼容但会展示裸英文；是否改成闭集阻断属于 UI contract 取舍 | `frontend/src/lib/reviewLabels.ts` 等 | 🧭 设计选择 |
| S3-17 | `groupOps` / `momentOps` 指向 Overview 是迁移注释和 README 均承认的占位能力，不是意外路由错误；上线对应产品能力前需替换 | `frontend/src/app/channels.ts:61,109-128` | 🧭 设计选择 |
| S3-18 | `features/user-ops/legacy.tsx` 超 2,000 行且被约 12 个生产/测试文件直接引用，名称已不能表达其正式职责，增加拆分成本 | `frontend/src/features/user-ops/legacy.tsx` | 🟡 确认风险 |
| S3-19 | 知识页使用 `LlmErrorBanner`，provider 页手工渲染相同错误字段；同一 payload 已形成两套展示实现 | `frontend/src/components/LlmErrorBanner.tsx`、`features/llm-providers/index.tsx` | 🧹 已修复：Provider 与知识页统一使用共享 LlmErrorBanner |
| S3-20 | cooldown 只在候选生成时检查，release endpoint 不复核；候选等待期间另一版本 release 后，旧候选仍可被发布，违反发布时 24h 约束 | `threshold.rs`、`evolution/release.rs` | 🧹 已修复（当前工作区） |
| S3-21 | `record_chunk_hit` 更新 filter 只有 `_id`。当前 id 来自 workspace-scoped 召回，未证实可利用；补 `workspace_id` 可消除对上游来源的单点信任 | `gap_signals.rs:1289` | 🧹 已修复（当前工作区） |
| S3-22 | feedback 刷新更新 filter 只有 `_id`。oid 当前来自 scoped `load_active_chunks`，未证实跨租户写入；仍与仓内 filter 纪律不一致 | `gap_signals.rs:1123` | 🧹 已修复（当前工作区） |
| S3-23 | 热路径 `$inc` 与 worker 重算曾存在 lost-update 窗口 | `gap_signals.rs::usage_refresh_pipeline` | 🧹 已修复：聚合 pipeline 保留 worker 读取后发生的正向热路径增量 |
| S3-24 | `assert_import_job_status_valid` 在 release 只记录错误而不拒绝写入。当前调用点均传字面量，风险只在未来引入动态值时暴露 | `models.rs:1056-1067` | 🧹 已修复（当前工作区） |
| S3-25 | oversized atom 到达时，若累计区只有空白，`acc.clear()` 会丢掉前导空白，违反分段 lossless 不变量；现有测试未覆盖该形状 | `import.rs:329-349` | 🧹 已修复（当前工作区） |
| S3-26 | `PATH_LOCKS` 仅进程内，跨进程 reconciler 无互斥。README 已要求共享一致存储或单写部署，因此这是部署约束；违反约束时存在 count-then-delete 竞争 | `media_storage.rs:173`、README 多副本限制 | 🧭 设计选择 |
| S3-27 | `DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED` 当前默认及 `.env.example` 都是 `true`；只有显式回滚为 `false` 才使用 reviewer 自评代理并形成反馈闭环风险 | `config.rs:608-612`、`.env.example:262` | 🧭 设计选择 |
| S3-28 | 成交前最后 3 条 usage log 被归因成 Hit 是代码明确承认的弱归因启发式，不能解释为因果效果；是否采用更强归因模型是产品分析选择 | `gap_signals.rs:1041-1049` | 🧭 设计选择 |
| S3-29 | 少于 `min_samples`（默认 5）时公式只用 `base - penalties`，这是明确的冷启动语义；应在指标说明/UI 暴露样本量，不能据此推断“多数 chunk”都失真 | `gap_signals.rs:1219-1249`、`config.rs:606` | 🧭 设计选择 |
| S3-30 | supervisor 持续 panic 无熔断/升级 | `supervisor.rs`、`routes/worker_controls.rs` | 🧹 已修复：5 次快速 panic 持久 open；管理员恢复后单副本 probe，稳定 60s 闭合，probe panic 立即重开 |
| S3-31 | Campaign run-log 二次查询缺少 workspace scope | `routes/campaigns.rs::campaign_run_logs_filter` | 🧹 已修复：查询固定包含 `workspace_id + source_event_id + source_kind`，并有 filter 单测 |
| S3-32 | planner 停滞段的**终态判定维度与计时维度不一致**：计时已按 `stagnation_dimension` 动态化，但终态取值两侧都硬读 `customer_stage`（内存 `:1237` 取值 → `:1241` 查配置终态集；DB 预筛 `:1215` 的 `$nin` 同理）。换维度并声明该维度终态时永不命中 → 已结束的 contact 被持续催停滞。DEFAULT 下两者同名、逐字等价，现网无影响 | `planner/mod.rs:1215,1237,1241` | 🟡 确认风险 |

---

## 文档漂移清单

除明确标为已处理的条目外，下列文档仍与当前代码不一致。建议按同一事实源集中修订，
避免架构、策略和 Wiki 文档继续描述不同的生产行为。

| # | 文档位置 | 文档说法 | 实际代码 | 等级 |
| --- | --- | --- | --- | --- |
| D-01 | `docs/architecture.md` Webhook Flow | 入站走进程内 debounce/generation runner | 🧹 已处理：当前文档改为 durable inbound task + pending handoff 恢复路径 |
| D-02 | `docs/architecture.md` | ops 配置按 contact hash 分桶 | 🧹 已处理：当前文档明确唯一 current，异常指针 fail closed |
| D-03 | `docs/agent-policy.md`、`docs/architecture.md` | 引用已删除的 `enforce_*` guard | 🧹 已处理：现行流程统一引用 `review_passed` / `classify_dual_gate` / `route_dual_gate` / `finalize_review_for_send`；历史 changelog 明示非当前契约 |
| D-04 | 相关 Prompt 文档/注释 | `load_prompt_for_contact` 按 contact/locale 选模板 | 🧹 已处理：当前入口文档明确按 `(workspace_id, prompt_key)` 唯一 current 读取；locale 仅保留为元数据 |
| D-05 | `docs/knowledge-wiki.md`、`docs/agent-policy.md` | 70% 闸曾描述为 `answer` / `explanation` | 🧹 已处理：当前代码与事实文档均按 `body / summary / answer` 逐字段检查 |
| D-06 | `docs/knowledge-wiki.md` 及模块头 | 默认锁字段仍写历史字段 | 🧹 已处理：代码与当前事实文档统一为真实 BSON 身份字段，`source_anchors` 明确为可受控重算字段 |
| D-07 | `docs/knowledge-wiki.md` | 生命周期包含 `integrity_ok` | 🧹 已处理：人工核验终态写为 `active + verified` |
| D-08 | `docs/knowledge-wiki.md` | 未解释 `needs_human_audit` | 🧹 已处理：生命周期和 provenance 表明确 auto-verify 最多进入待真人复核态 |
| D-09 | `docs/knowledge-wiki.md`、`chunk_revisions.rs` 模块头 | revision 先写、chunk 失败仍保留尝试痕迹 | 🧹 已处理：当前文档明确 revision/chunk/catalog intent 同事务原子提交 |
| D-10 | `docs/knowledge-wiki.md`、`docs/architecture.md` | structural lint 仅列旧 5 类 | 🧹 已处理：Wiki 列出 9 类离线规则，并区分在线 `recall_miss` |
| D-11 | `docs/knowledge-wiki.md` | `related_chunks` 曾被描述为参与通用数组 union | 🧹 已处理：文档明确其在 revision 层按 `chunk_id` 单独合并 |
| D-12 | `docs/knowledge-wiki.md`、`docs/agent-policy.md` | 所有 chunk 写入都经过 revision funnel | 🧹 已处理：principal、lessons promotion 与 reaction negative example 均为事务化 create revision |
| D-13 | `docs/agent-policy.md` | `message_send_text` 可按策略执行 | 代码从目录剥除、不在白名单并按名硬拒；Management 发送改走产品网关工具 | 🧹 已处理（当前工作区） |
| D-14 | `docs/agent-policy.md` | 24h cooldown 在 release 时阻止发布 | 代码只在候选生成阶段检查，release 未复核，见 S3-20 | 🧹 已处理（当前工作区） |
| D-15 | `docs/data-and-api.md` | collection 清单仅覆盖 13 个 | 🧹 已处理：不再手抄全集，明确 `src/db/mod.rs`（当前 61 typed accessors）、indexes 与 migrations 为权威来源 |
| D-16 | README | 路由数旧写“约 220” | 当前工作区 README 已更新为 229 个 `.route()` 调用，并说明不等于 HTTP method 端点数 | 🧹 已处理 |
| D-17 | Evolution 隔离测试说明 | 旧文档引用不存在的集成测试 | 🧹 已处理：结构隔离契约位于 `src/evolution/mod.rs::isolation_contract_tests` |
| D-18 | `docs/knowledge-wiki.md`、模型注释 | provenance source 仍只列早期集合 | 🧹 已处理：当前文档与闭集统一为五类并解释 `principal_authorized` |
| D-19 | `docs/knowledge-wiki.md` | 动态置信度公式未说明冷启动门 | 🧹 已处理：明确少于最小样本时只算 `base - penalties` |
| D-20 | `docs/knowledge-wiki.md` | 罚项只描述 stale | 🧹 已处理：明确 stale 与 dangling source quote 两类罚项 |
| D-21 | `docs/knowledge-wiki.md` | 未说明真实结果开关及删失口径 | 🧹 已处理：明确真实 outcome 默认启用，沉默/pending 删失，false 才回退 reviewer 自评 |
| D-22 | `docs/knowledge-wiki.md` | 热路径 `$inc` 被画成同步提升 dynamic confidence | 🧹 已处理：明确热路径只记计数，feedback worker 周期离线重算 |

---

## 命名与语义陷阱（非缺陷，但改代码前必读）

这些是「字段名与实际语义不一致」的地方，动相关代码前务必先确认映射：

| 字段 / 符号 | 实际承载的语义 |
| --- | --- |
| `UserRuntimeParameters.fact_risk_block_at` | **hallucination** 阈值（源自 `typed.hallucination_block_at`） |
| `UserRuntimeParameters.product_accuracy_block_below` | **knowledge_grounding** 阈值（源自 `typed.knowledge_grounding_block_below`） |
| gate_key `fact_risk_block` | 同上，hallucination |
| gate_key `product_accuracy_score_block` | 同上，knowledge_grounding |
| `features/user-ops/legacy.tsx` | 并非 legacy，是多个视图的正式家 |
| `OpsDomain` trait | 仅边界声明，全仓无 bound / dyn / 泛型使用，无分派 |

---

## 已验证正确的关键红线（勿在重构中破坏）

审查中确认这些防护真实有效，列出以防后续改动误伤：

| 红线 | 实现位置 | 保护方式 |
| --- | --- | --- |
| 管理 Agent 不绕过发送网关 | `management.rs:2047-2069` | 该 arm **无 MCP 调用**，走 `send_contact_message_gateway` 全链；强制 `review_mode="full"` + `risk_level="high"`；要求 finalize Approved **且** `review_passed` 双条件（管理发送无 revision 通道）；落 outbox 而非直发 |
| `message_send_*` 不可达 | `management.rs:1048,1075,2972` | 三重：catalog 剥除 + 不在 whitelist + 按名硬拒 |
| LLM 幻觉工具名不可达 MCP | `management.rs:2971-2993` | catch-all arm 校验 `advertised`（来自**实时** `tools/list`，非 LLM 输出） |
| 未分类工具 fail-closed | `management.rs:1668,1761` | `tool_effect` 兜底 `(Dangerous, false)` + `!explicitly_classified` 强制确认 |
| Campaign 群发不批量扇出 | `campaigns.rs:678-820` | 每联系人一条确定性 `_id` 的 follow_up task，先建 `committing` 态防抢占，durable intent 落定后 CAS 放行 → **每条消息各自走完整网关** |
| 崩溃的工具意图不重放 | `management.rs:258-291` | `executing` 收敛到 `execution_unknown` 并 `break` 中止剩余计划 |
| 自动发布不可绕过 | `auto_release.rs:40,44,57` | `CURRENT_AUTO_RELEASE_POLICY_ENABLED` 是**编译期 const false**，位于 `&&` 首项，且入口先检查；API 拒绝把子闸写 true |
| 演化 prompt 不可自指 | `prompt_critic.rs:457,460`、`prompt_guard.rs:51` | 三重：批次丢弃 + `EVOLVABLE_PROMPT_TARGETS` 白名单结构性排除 + 手工编辑拦截 |
| auto-verify 不能产出 verified | `verify.rs:610` | `enforce_verified_needs_human_audit` 无条件降级为 `needs_human_audit`；采样率有 5% 硬下限 |
| 知识引用必须有字面证据 | `knowledge_agent.rs`（`filter_answer_against_opened_chunks`） | 引用须指向已打开+verified+非 contradiction 的 chunk；引文须是该 chunk 字面证据；anchor index 必需且其 `sourceQuote` 须指向同一证据 |
| 画像升级拒绝模型自报 | `tag_evidence.rs:33`、`gateway.rs:4948` | 要求 `explicit_intent=true` **且**至少一条证据锚定 Inbound 消息；代码按消息方向重算 strong count，从不读 LLM 自报 confidence；同 run 内值冲突则整维度作废 |
| 发送去重不靠猜测 | `mcp.rs:228-306`、`gateway.rs:4038` | `SafeToRetry` / `DeliveryUncertain` 边界画在 `send()` 处；`isError:true`+HTTP200 判失败；`Inconclusive` → `delivery_unknown` 且**禁用自动重放** |
| MCP 超时不吞审计 | `mcp.rs:12-25` | `MCP_CLIENT_TIMEOUT_SECONDS=60` 须严格小于 dispatcher 外层超时，否则 `mcp_logs` 行丢失、post-hoc 去重查不到证据 → 重复发送 |
| 发送授权 CAS 顺序 | `tasks.rs:454-514` | 先给所有 outbox 行打 prepared marker，**再**做 task CAS（反序会留下「已提交 task + 无 marker」永久卡住）；marker 本身不构成授权 |
| 路由无死代码 | `routes/mod.rs:1045` | `include_str!` 扫所有 `pub async fn` 与 router 静态文本比对；白名单 40+ 条每条写明理由 |
| 无人工接管 | `scripts/check-no-human-takeover.sh` | CI 扫 git diff 新增行禁词；状态闭集拒绝 `held_for_human` 类取值；前端闭合 label map |

---

## 后续建议动工顺序

本轮已关闭高优先安全批、S3-31、FE-STORE-1/2 与 WK-01/03。真实剩余项按影响排序：

1. **HC-001**：先完成已公开且仍有效的 LLM 凭证轮换、撤销与载体清理；这是生产授权硬门，不由普通代码改动替代。
2. **发布闭环**：冻结当前大工作树，运行 replica-set/CI hard gates，并完成测试环境 Webhook→Task→Review→Outbox→MCP 回归。
3. **S3-32**：正式上线非销售 Domain 前，把 planner 停滞段的终态判定改为按 `stagnation_dimension` 取值（内存 `:1237` 与 DB 预筛 `:1215` 同源改），消除"计时按配置维度、终态按 sales stage"的不一致。DEFAULT 下等价，现网无影响，但换维度即失效。同批复核 `S3-13`（已定性为设计选择）是否仍符合届时的产品语义。
4. **AUTH-02**：公网管理面需单独设计可恢复的 target 限流；不得直接删除抗密码喷洒维度。
5. **WK-02 / S3-18**：分别在多副本读放大成为实际问题、以及发布闭环完成后再治理。
6. 产品选择项（S2-07、S2-10、S3-08/11/16/17/26-29）需产品/架构决策后再改语义。

---

## 审核边界

本轮是针对台账每个断言的**静态源码、调用链和现有测试交叉复核**，不是对每个文件逐行重新通读，
也不是部署验收。已深入核对阈值演化、Review 闸门、知识修订/导入、Management、Taxonomy、Planner、
feedback worker、相关前端契约和文档；以下大模块只读取了与条目直接相关的范围：

- `src/agent/memory.rs` 的 consolidation 主体
- `src/agent/entitlements.rs` 与无关的 Taxonomy 分支
- `src/models.rs` 中与本台账无关的结构定义
- 前端各 feature 的非相关交互实现
- Rust 测试集中与本台账无关的正文

本轮已重新运行后端全量单元测试、前端全量测试、前端 production build、Rust `-D warnings` 全目标检查、格式与隔离 lint，结果见文件顶部。
Docker/testcontainers、真实 LLM/MCP/微信、生产迁移和多副本故障注入仍未验证。`lessons_learned` 裸写入等相关观察已明确保持未合并定性。

**本轮新增批次（MIG / OD / AUTH / S3-31）的核验方式**：每条都由我本人 Read 到具体行号，
并追到调用链两端（写入点与读取点）后才定性；表中未列入任何仅凭 subagent 报告的条目。
未列入的原因是上一批 subagent 报告里有可复现的误判——例如把 `record_chunk_hit`
的 `_id`-only filter 报成"模型可控的跨租户写入"，实际 `chunk_id` 来自
`load_operation_knowledge` 的 workspace-scoped 召回，跨租户不可达（现记为 S3-21/22 的纪律问题）。

**仍未覆盖的范围（下一批目标）**：
- 前端 4 个维度（认证面、状态一致性、渲染安全、enum 与后端闭集漂移）——295 个文件、约 4.9 万行，
  已确认无超大文件，可安全分派；此前批次失败是 agent 从仓库根跑无路径限定的 `grep`/`find`
  扫进了 `target/`（14G）与 `node_modules/`（150M），非源码本身问题。
- `cold_contact_worker.rs` / `silence_signal_worker.rs` 的调度与租约语义。
- 184 个测试文件正文、`models.rs` 剩余约 8,000 行结构定义。

---

## 维护约定

本文件是**持续台账**，不是一次性快照。

- 新发现先标 `⚠️ 待复核`，必须补齐源码位置、调用链、反证测试和影响边界后才能升级
- 复核后按 `✅ 缺陷`、`🟡 风险`、`🧭 设计选择` 分类，不以“代码现象存在”代替缺陷定性
- 修复后不要删除条目，改标 `🔧 已修复（commit）` 并记录验证命令，保留历史原因
- 代码或默认值变化后同步检查对应 D 条目、README、`.env.example` 和相关测试 fixture
