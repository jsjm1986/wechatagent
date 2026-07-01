# 请求体死字段清理设计（正规层，不碰 prompt/旁路）

> 日期：2026-07-01
> 类型：后端重构（Rust/Axum），无前端逻辑改动、无 prompt 改动
> 前置：基于最新 origin/main（commit e31b8e5，含刚合并的 #83）100% 亲验；本设计的每处断言经**两轮独立 subagent 审查**交叉验证（第一轮抓出 apply 简化的行为漂移疑点，第二轮在"同时删字段"前提下证明该疑点不成立、且推翻了第一轮自己的修正建议）。

## 0. 背景：一次"未完成的删除"的正规层收尾

2026-05-25 从 model `OperationKnowledgeChunk` 删除了 `safe_claims / forbidden_claims / evidence_items / routing_card` 等字段（grounding 改走 verified-chunk 语义闸）。但删除只做了一半：**model 字段删了，请求体 `OperationKnowledgeChunkRequest` 的对应字段、以及依赖它们的 `apply_chunk_integrity` / `integrity_report_for_preview` 判定逻辑没跟着删**。

现状是割裂的：
- **model 层已死**：这些字段不在 `OperationKnowledgeChunk`，转换函数 `operation_knowledge_chunk_from_request` 不读它们 → 经"请求体→转换函数→model"这条**正规落库路径 100% 空转**。
- **但仍有"绕过 model"的活旁路**：repair prompt schema（`prompts.rs`）让 LLM 产这些字段、chat 裸 `$set` 旁路（`chat.rs:1720-1750`）直写 Mongo 额外字段、catalog 裸读统计（`catalog.rs:511`）、consultative 文案（`prompts.rs:33/1140`）引用 `verified_chunks.safe_claims`。

**本设计只做"正规层收尾"**：删请求体经正规路径确认空转的死字段 + 移除随之恒不可达的死代码分支。**明确不碰** prompt schema / chat 裸 `$set` 旁路 / catalog 统计 / consultative 文案——那些是"prompt/旁路层"，风险面和性质不同，另开专题（§5）。

**红线**：本次是纯清理，不改任何对客/落库的 integrity 判定行为。经审查确认 apply 简化后与所有运行时输入**逐字节等价**（§2.2）。

## 1. 已亲验的关键事实（设计地基，全部带 file:line）

### 1.1 六个字段经正规路径 100% 空转（断言 1，审查确认）
- 请求体 `OperationKnowledgeChunkRequest`（`mod.rs:170-219`）含：`routing_card`(:181)、`safe_claims`(:187)、`forbidden_claims`(:189)、`evidence_items`(:191)、`unsupported_claims`(:206)、`verified_claims`(:208)。struct 带 `#[allow(dead_code)]`(:169)。
- 转换函数 `operation_knowledge_chunk_from_request`（`mod.rs:470-522`）**一个都不读**这六个字段（读的是 account_id/domain/title/summary/body/applicable_scenes/.../source_quote/source_anchors/integrity_status/confidence_score/status/priority/wiki_type/chunk_type），其余靠 `..Default::default()`(:520)。
- model `OperationKnowledgeChunk`（`models.rs:1429-1519` + Default `:1527-1568`）**无这六个字段**。
- → 经"请求体→转换函数→model"落库路径，这六个字段有值也被丢弃。

### 1.2 `distortion_risks` 不能删（断言 2，审查确认）
- 它是 preview→前端的**活 wire 字段**：`integrity_report_for_preview` 写返回 JSON（`mod.rs:910/932`）、`coerce_integrity_against_d2_gate` 写降级理由（`mod.rs:1128`）、前端 `trustTypes.ts:167` / `ReviewChat.tsx:99` 真读渲染、前端测试 `ReviewChat.test.tsx:33` / `trustTypes.test.ts:114` 断言。
- → 保留 `distortion_risks`，不进本次删除清单。

### 1.3 apply 分支3（rejected）删字段后恒不可达（断言 3，第二轮审查证明）
- `apply_chunk_integrity`（`mod.rs:944-1004`）三分支：分支1 `has_anchor→verified`(:957-964)、分支2 `has_quote || (safe_claims.is_empty() && evidence_items.is_empty())→needs_review`(:970-992)、分支3 `else（!has_quote && claims非空）→rejected`(:993-1003)。
- 请求体**无 `#[serde(deny_unknown_fields)]`**（`mod.rs:167-169` 仅 `rename_all`+`allow(dead_code)`）→ 删字段后 body 带 `safeClaims` 键被 serde **静默丢弃**。
- apply 的 4 个调用点（PUT `crud.rs:241`、import `import.rs:292/341/855`）的 chunk **全部源自 serde 反序列化** → 删字段后 `safe_claims/evidence_items` **恒空 Vec**。
- 唯一非-serde 构造点 `chunk_request_from_chat_patch`（`chat.rs:1823`）**不流到 apply**（它被 `apply_create_chunk` 调用后强制 needs_review + 直接 insert，从不调 apply）。
- → 删字段后分支3 条件 `!has_quote && (claims非空)` **恒 false**，分支3 是运行期恒不可达死代码。删它 = 移除死代码，**非行为漂移**。

### 1.4 POST create 不调 apply（断言 3 佐证）
- `create_operation_knowledge_chunk`（`crud.rs:192-210`）只调 `validate_*` + `coerce_integrity_against_d2_gate`(:197-198)，**不调 apply** → create 路径本就不触发分支3。

### 1.5 import 三路径 apply 后无条件压回 needs_review（断言 3 佐证）
- `import.rs:292→298-299`、`:341→345-348`、`:855→857-863` 均在 apply 后无条件 `integrity_status=Some("needs_review")` → 分支3 的 rejected 产出在 import 域本就被覆盖、不可观测。

### 1.6 连带编译点与死键（断言 4/5，审查确认）
- `chunk_request_from_chat_patch`（`chat.rs:1823-1852`）是**完整 struct literal**，显式赋六字段（routing_card:1833/safe_claims:1836/forbidden_claims:1837/evidence_items:1838/unsupported_claims:1846/verified_claims:1847）→ 删字段 **E0063 编译强制**连带删这些行。
- `chat.rs` 裸 `$set` 旁路 `apply_update_chunk`（:1720-1752）用**字符串 key**直写 Mongo，是**独立的另一处**，不受删 struct 字段影响 → 本次不碰。
- `page_merge.rs:59-60` `DEFAULT_UNION_ARRAY_KEYS` 含字符串 `"safe_claims"/"forbidden_claims"` → 删字段后成永不命中死键，一并清理。
- `import.rs:797` 构造 `OperationKnowledgeChunkRequest` 走 `..Default::default()`，**不显式赋六字段** → 不受影响（无需改）。
- 测试 `preview_claim_without_source_is_rejected`（`mod.rs:1270`）靠 `safeClaims` 无源触发 rejected 断言、`preview_anchor_match_never_auto_verifies`（`mod.rs:1240`）构造 safeClaims → 删后连带改。

## 2. 改动方案

### 2.1 删请求体死字段（`src/routes/knowledge/mod.rs`）
- `OperationKnowledgeChunkRequest`（:170-219）删六字段：`routing_card`、`safe_claims`、`forbidden_claims`、`evidence_items`、`unsupported_claims`、`verified_claims`。
- 删 struct 上的 `#[allow(dead_code)]`(:169)。**已亲验**：删六字段后剩余全部字段都被消费——转换函数读 account_id/domain/title/.../wiki_type/chunk_type（含 `document_id`/`item_id` 经 `.as_deref()` 链，`mod.rs:474-486`）；`distortion_risks` 被 `apply_chunk_integrity`(:971-999)/`coerce`(:1128) 读写 + 测试断言(:1775/1802/1816)。故删 `#[allow(dead_code)]` 后无新 dead_code warning。
- **保留** `distortion_risks`(:204)、`source_quote`/`source_anchors`/`integrity_status`/`confidence_score`（真落库，D2 闸核心）、`wiki_type`/`chunk_type`（#83 刚加）。

### 2.2 简化 `apply_chunk_integrity`（`mod.rs:944-1004`）
删字段后：
- 分支1（:957-964）：删 `if chunk.verified_claims.is_empty() { chunk.verified_claims = chunk.safe_claims.clone(); }`（:958-960，字段已删，编译强制），保留 `integrity_status=verified` + `confidence_score`。
- 分支2/3：`safe_claims`/`evidence_items` 字段删除后，:970 条件里的 `(chunk.safe_claims.is_empty() && chunk.evidence_items.is_empty())` 引用已删字段编译失败——简化为：无 anchor 时一律走 needs_review（保留 distortion_risks 提示 + verified→needs_review 降级保护 :987-989 + confidence 45）。**删除分支3 整块（:993-1003）**（恒不可达死代码）。
- 结果：`has_anchor→verified / else→needs_review` 两态。
- **等价性保证（审查确认逐字节等价）**：PUT 路径最终 integrity_status 仍由下游 `coerce_integrity_against_d2_gate` 把守（verified 必须 quote+anchor 双全，否则压回 needs_review）；删字段后不存在任何输入能触发旧分支3 的 rejected。**不引入任何新判据**（"body/summary 非空→rejected"经审查证明是误伤正常草稿的错误方向，不采纳）。

### 2.3 简化 `integrity_report_for_preview`（`mod.rs:866-942`）
- 删 `safe_claims`/`evidence_items` 读取（:876-877）+ 依赖它们的 risk 判定（:887）+ 恒不可达的 rejected 分支（:897-904 的 else 支，删字段后 `safe_claims.is_empty() && evidence_items.is_empty()` 恒 true → rejected 不可达）+ `verifiedClaims`/`unsupportedClaims` 输出（:911-926）。
- **保留** sourceAnchors/integrityStatus/confidenceScore/distortionRisks 输出。preview 简化为 needs_review 单态（本就恒 0 verified，:868）。

### 2.4 连带改 `chunk_request_from_chat_patch`（`src/routes/knowledge/chat.rs:1823-1852`）
- 删六字段赋值行：routing_card(:1833)、safe_claims(:1836)、forbidden_claims(:1837)、evidence_items(:1838)、unsupported_claims(:1846)、verified_claims(:1847)。
- **不碰** chat 裸 `$set` 旁路 `apply_update_chunk`（:1720-1752，字符串 key，独立）。

### 2.5 连带改 `page_merge.rs`（`src/knowledge_wiki/page_merge.rs:59-60`）
- `DEFAULT_UNION_ARRAY_KEYS` 删字符串键 `"safe_claims"`、`"forbidden_claims"`（删字段后永不命中死键）。
- 实现前先 Read 确认这两个键的确切位置和该常量的其它用途，只删这两个字符串，不动其它键。

### 2.6 连带改测试
- `preview_claim_without_source_is_rejected`（`mod.rs:1270`）：靠 safeClaims 无源触发 rejected，删字段后语义消失。**决定：改写（不删）**——保留"preview 对无源 chunk 的判定"这条契约的测试覆盖比直接删更有价值，只把过期的 rejected 预期更新为删字段后的正确行为。具体：测试名改为 `preview_no_source_is_needs_review`（或等价名），构造去掉 safeClaims/evidenceItems（改用一个仅有 body、无 sourceQuote 的 chunk），断言 `integrityStatus == "needs_review"`（不再是 rejected），佐证"preview 恒不 auto-verify 且无源不再硬 reject（因 claim 维度已随死字段移除）"。
- `preview_anchor_match_never_auto_verifies`（`mod.rs:1240`）：删构造里的 safeClaims/evidenceItems，保留"anchor 命中也不 auto-verify"的核心断言（这条契约不变）。
- 遵守"测试只增量叠加"铁律的例外说明：本次是**字段删除导致的测试契约更新**（被测字段不复存在），属必要连带，非为调绿而改。

## 3. 测试与验证
1. `cargo check --tests`（无 warning，尤其确认删 `#[allow(dead_code)]` 后无新 dead_code 报错）。
2. `cargo test --lib` ≥350/0（最新基线 1777/0，本次删测试后数字可能略降，但不得低于 350，且 0 failed）。
3. 4 PBT 文件累计 ≥33/0（本次不碰 PBT，应无影响）。
4. `bash scripts/check-no-human-takeover.sh <BASE> HEAD` 0 violations。
5. 前端不改，但 `distortion_risks` 保留确保 ReviewChat 渲染不回归——本机无前端 e2e，靠"未删该字段"+ 前端测试不受影响佐证。

## 4. 全局约束（每 task 隐含遵守）
- Rust 2021；无 Cargo workspace。
- 红线「AI 永不自动 verify」：本次不改任何 integrity 判定行为，apply 简化经审查逐字节等价。
- 禁词 lint：新增/改动行不得含单字"人工"（本次改动主要是删除，注释若改用"运营"）。
- 测试基线不回归；新增测试只 append（本次的测试改写属"被删字段导致的契约更新"例外，已在 §2.6 说明）。
- 向后兼容：请求体无 `deny_unknown_fields`，删字段后带旧 wire 键的请求不会 400（静默丢弃），对现有前端零破坏。

## 5. 不做（YAGNI 边界，明确排除 → 另开专题）
- **不碰 repair prompt schema**（`prompts.rs:1750-2069`）：仍让 LLM 产 safeClaims/forbiddenClaims/evidenceItems。删字段后经正规路径静默丢弃，但 chat 裸 $set 旁路仍会裸写进 Mongo。属"prompt/旁路层"专题。
- **不碰 consultative 文案**（`prompts.rs:33/1140`）：`verified_chunks.safe_claims` 引用是运营 Agent 红线文案，改它涉及 grounding 语义，独立专题。
- **不碰 chat 裸 `$set` 旁路**（`chat.rs:1720-1752`）：字符串 key 直写 Mongo，删它要重设计 chat 更新落库路径，独立专题。
- **不碰 catalog 统计**（`catalog.rs:511` 读 `evidence_items.0 $exists`）：删它要同步 catalog 覆盖率口径，独立专题。
- **留下的语义债（本次接受）**：prompt 产 safeClaims → 正规路径丢弃、chat 旁路裸写 Mongo → catalog 统计。这条"prompt→旁路→统计"链仍活，本次只清正规请求体层，债留专题。

## 6. 执行顺序建议
单一改动线，一个 commit 或按文件拆 2-3 个 commit：
1. 删请求体字段 + 简化 apply + 简化 preview（`mod.rs`，核心）
2. 连带 `chat.rs` 构造函数 + `page_merge.rs` union 键
3. 连带测试改写
三步在同一分支，编译互相依赖（删字段后 2/3 必须同时改才能编译过），建议合并为一次 TDD 循环：先改测试预期 → 删字段（编译红）→ 改所有连带点（编译绿）→ 全量验证。
