# 请求体死字段清理 实施计划（正规层）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除 `OperationKnowledgeChunkRequest` 里经正规落库路径 100% 空转的六个死字段，并移除随之恒不可达的死代码分支，不改任何对客/落库 integrity 行为。

**Architecture:** 一次原子清理。删请求体六字段（`routing_card/safe_claims/forbidden_claims/evidence_items/unsupported_claims/verified_claims`）→ 编译强制连带简化 `apply_chunk_integrity`（删恒不可达的 rejected 分支）、`integrity_report_for_preview`（删 claim 读取 + rejected 分支）、`chunk_request_from_chat_patch`（删字段赋值行）、`page_merge.rs` union 死键、两个 preview 测试。删字段后这些连带点必须**同时**改才能编译通过，故合并为**单 Task 一次 TDD 循环**。

**Tech Stack:** Rust 2021（Axum + MongoDB），无 Cargo workspace。测试 = `cargo test --lib` 单测。

## Global Constraints

- **红线「AI 永不自动 verify」**：本次不改任何 integrity 判定行为。`apply_chunk_integrity` 简化经两轮独立审查确认与所有运行时输入**逐字节等价**（删字段后 rejected 分支恒不可达，删它=移除死代码）。**绝不引入新判据**（"body/summary 非空→rejected"经审查证明会误伤正常草稿，已否决）。
- **保留 `distortion_risks`**：它是 preview→前端活 wire 字段（`integrity_report_for_preview` 写、`coerce_integrity_against_d2_gate` 写降级理由、前端 ReviewChat/trustTypes 读），**不在删除清单**。
- **不碰 prompt/旁路层**：repair prompt schema（`prompts.rs`）、consultative 文案（`prompts.rs:33/1140`）、chat 裸 `$set` 旁路（`chat.rs:1720-1752`，字符串 key 直写 Mongo，独立于 struct 字段）、catalog 统计（`catalog.rs:511`）——全部**不动**，另开专题。
- **禁词 lint**（`scripts/check-no-human-takeover.sh` 扫 src/routes 等新增行）：改动/新增行不得含单字"人工"，用「运营」。
- **测试基线不回归**：`cargo test --lib` ≥350/0（当前基线 1777/0；本次改写 1 个测试、清理 1 个测试构造，不减测试数量，仍应 ≥350/0）。
- **向后兼容**：请求体无 `#[serde(deny_unknown_fields)]`（`mod.rs:167-169`），删字段后带旧 wire 键（`safeClaims` 等）的请求被 serde **静默丢弃**，不 400，对现有前端零破坏。

---

### Task 1: 原子清理请求体死字段 + 连带简化

**Files:**
- Modify: `src/routes/knowledge/mod.rs`（请求体 :170-219 删六字段、`apply_chunk_integrity` :944-1004 简化、`integrity_report_for_preview` :866-942 简化、两个 preview 测试 :1240-1281）
- Modify: `src/routes/knowledge/chat.rs`（`chunk_request_from_chat_patch` :1823-1852 删六字段赋值行）
- Modify: `src/knowledge_wiki/page_merge.rs`（`DEFAULT_UNION_ARRAY_KEYS` :51-61 删两死键）

**Interfaces:**
- Consumes: 无（纯删除 + 简化）。
- Produces: `OperationKnowledgeChunkRequest` 不再有 `routing_card/safe_claims/forbidden_claims/evidence_items/unsupported_claims/verified_claims` 字段；`apply_chunk_integrity` 简化为 `has_anchor→verified / else→needs_review` 两态。

**背景（已亲验，全部带 file:line）**：
- 请求体六字段经"请求体→`operation_knowledge_chunk_from_request`（mod.rs:470-522，一个都不读）→model `OperationKnowledgeChunk`（models.rs:1429-1519，无这些字段）"落库路径 100% 空转。
- 删字段后 `apply_chunk_integrity` 分支3（`!has_quote && (safe_claims非空||evidence_items非空)→rejected`，:993-1003）**恒不可达**：请求体无 `deny_unknown_fields`，4 个 apply 调用点（PUT crud.rs:241 + import.rs:292/341/855）的 chunk 全源自 serde 反序列化 → 删字段后 `safe_claims/evidence_items` 恒空 Vec → 分支3 条件恒 false。唯一非-serde 构造点 `chunk_request_from_chat_patch` 不流到 apply。
- `chunk_request_from_chat_patch`（chat.rs:1823-1852）是完整 struct literal，显式赋六字段（编译强制连带）。
- `page_merge.rs:59-60` union 死键 `"safe_claims"/"forbidden_claims"`。

- [ ] **Step 1: 先改测试预期（TDD 红——测试先反映删字段后的目标行为）**

在 `src/routes/knowledge/mod.rs` 测试模块：

(a) 把 `preview_claim_without_source_is_rejected`（:1268-1281）**整体替换**为新语义测试（改名 + 去 claim 构造 + 断言 needs_review）：
```rust
    /// preview：无 sourceQuote/anchor 的 chunk → needs_review（claim 维度已随死字段移除，
    /// 不再硬 reject；红线「AI 永不自动 verify」仍恒 0 verified）。
    #[test]
    fn preview_no_source_is_needs_review() {
        let raw = "本文与产品声明无关的纯背景介绍。";
        let mut chunks = vec![json!({
            "title": "无源切片",
            "body": "一段没有原文引用的正文",
            "sourceQuote": ""
        })];
        let report = integrity_report_for_preview(raw, &mut chunks);
        assert_eq!(chunks[0]["integrityStatus"], json!("needs_review"));
        assert_eq!(report["verified"], json!(0));
    }
```

(b) 把 `preview_anchor_match_never_auto_verifies`（:1242-1247）构造里的 `safeClaims`/`evidenceItems` 两行**删除**（该测试断言只验 verified=0/needs_review/confidence=90/sourceAnchors，不依赖 claims，删构造后断言仍成立）：
```rust
        let mut chunks = vec![json!({
            "title": "企业版能力",
            "sourceQuote": "WechatAgent 企业版提供 7x24 小时自动应答"
        })];
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib preview_no_source_is_needs_review 2>&1 | tail -15`
Expected: 编译失败或断言失败（当前 `integrity_report_for_preview` 对无 quote+有内容的判定仍走旧逻辑；且旧测试名 `preview_claim_without_source_is_rejected` 已被替换）。这一步确认新测试红。

- [ ] **Step 3: 删请求体六字段**

`src/routes/knowledge/mod.rs` 的 `OperationKnowledgeChunkRequest`（:170-219）：
- 删 `routing_card: Option<String>,`（:181）
- 删 `#[serde(default)] safe_claims: Vec<String>,`（:186-187）
- 删 `#[serde(default)] forbidden_claims: Vec<String>,`（:188-189）
- 删 `#[serde(default)] evidence_items: Vec<String>,`（:190-191）
- 删 `#[serde(default)] unsupported_claims: Vec<String>,`（:205-206）
- 删 `#[serde(default)] verified_claims: Vec<String>,`（:207-208）
- 删 struct 上的 `#[allow(dead_code)]`（:169）——已亲验删六字段后剩余字段全被消费（转换函数读大部分含 document_id/item_id 经 `.as_deref()`；distortion_risks 被 apply/coerce 读写 + 测试断言），无新 dead_code warning。
- **保留** `distortion_risks`（:203-204）及其它所有字段。

- [ ] **Step 4: 简化 `apply_chunk_integrity`（mod.rs:944-1004）**

替换为（删分支1 的 verified_claims 死行、删分支3 整块、简化分支2 条件）：
```rust
pub(super) fn apply_chunk_integrity(
    chunk: &mut OperationKnowledgeChunkRequest,
    raw_content: &str,
    document_id: Option<ObjectId>,
) {
    let source_quote = chunk.source_quote.clone().unwrap_or_default();
    if chunk.source_anchors.is_empty() {
        if let Some(anchor) = source_anchor_for_quote(raw_content, document_id, &source_quote) {
            chunk.source_anchors.push(anchor);
        }
    }
    let has_anchor = !chunk.source_anchors.is_empty();
    let has_quote = !source_quote.trim().is_empty();
    if has_anchor {
        chunk.integrity_status = Some("verified".to_string());
        chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(90));
        return;
    }
    // 无 anchor：一律 needs_review（有 quote 但没锚定 = 引用待纠正；无 quote = 缺出处）。
    // 由下游 AI 自主修复流程重新锚定。红线「AI 永不自动 verify」：绝不在此直接 verified。
    if !has_quote && chunk.distortion_risks.is_empty() {
        chunk
            .distortion_risks
            .push("缺 sourceQuote 与原文锚点，建议触发 AI 自主修复".to_string());
    } else if has_quote && chunk.distortion_risks.is_empty() {
        chunk
            .distortion_risks
            .push("sourceQuote 未在原文中精确匹配，建议触发 AI 自主修复以纠正引用".to_string());
    }
    chunk.integrity_status = Some(
        chunk
            .integrity_status
            .clone()
            .filter(|s| matches!(s.as_str(), "needs_review" | "verified" | "rejected"))
            .unwrap_or_else(|| "needs_review".to_string()),
    );
    if matches!(chunk.integrity_status.as_deref(), Some("verified")) {
        chunk.integrity_status = Some("needs_review".to_string());
    }
    chunk.confidence_score = Some(chunk.confidence_score.unwrap_or(45));
}
```
> 等价性（审查确认）：删字段后旧分支3 恒不可达，删它无行为变化；PUT 路径最终 verified 仍由下游 `coerce_integrity_against_d2_gate` 把守（quote+anchor 双全才放行）。**保留** `integrity_status` 已是 rejected 时的透传（:984 的 filter 含 "rejected"）——若上游显式传 rejected 仍尊重，只是 apply 自己不再产 rejected。

- [ ] **Step 5: 简化 `integrity_report_for_preview`（mod.rs:866-942）**

删 `safe_claims`/`evidence_items` 读取（:876-877）、依赖它们的 risk（:887）、rejected 分支（:897-904 的 else 支）、`verifiedClaims`/`unsupportedClaims` 输出（:911-926）。替换为：
```rust
pub(super) fn integrity_report_for_preview(raw_content: &str, chunks: &mut [Value]) -> Value {
    // 红线「AI 永不自动 verify」：preview 路径恒 0 verified，anchor 命中只作审计线索。
    let verified = 0;
    let mut needs_review = 0;
    let rejected = 0;
    let mut items = Vec::new();
    for chunk in chunks.iter_mut() {
        let source_quote = json_string(chunk, "sourceQuote")
            .or_else(|| json_string(chunk, "source_quote"))
            .unwrap_or_default();
        let mut risks = Vec::new();
        let mut anchors = Vec::new();
        if let Some(anchor) = source_anchor_for_quote(raw_content, None, &source_quote) {
            anchors.push(anchor);
        } else if !source_quote.trim().is_empty() {
            risks.push("sourceQuote 未在原文中找到".to_string());
        } else {
            risks.push("缺少原文引用".to_string());
        }
        let anchored = !anchors.is_empty() && risks.is_empty();
        // 红线「AI 永不自动 verify」：anchor 命中只作审计线索，integrityStatus 恒 needs_review。
        needs_review += 1;
        let status = "needs_review";
        let confidence = if anchored { 90 } else { 45 };
        if let Some(object) = chunk.as_object_mut() {
            object.insert("sourceAnchors".to_string(), json!(anchors));
            object.insert("integrityStatus".to_string(), json!(status));
            object.insert("confidenceScore".to_string(), json!(confidence));
            object.insert("distortionRisks".to_string(), json!(risks.clone()));
        }
        items.push(json!({
            "title": json_string(chunk, "title").unwrap_or_default(),
            "integrityStatus": status,
            "confidenceScore": confidence,
            "distortionRisks": risks,
            "sourceAnchors": anchors
        }));
    }
    json!({
        "verified": verified,
        "needsReview": needs_review,
        "rejected": rejected,
        "items": items
    })
}
```
> `rejected` 保留在返回 JSON 里恒 0（前端聚合展示需要该键存在）；删 verifiedClaims/unsupportedClaims 输出（对应已删死字段）；保留 sourceAnchors/integrityStatus/confidenceScore/distortionRisks。若 `let rejected = 0` 触发 unused warning，改用 `#[allow(unused)]` 或直接内联 `"rejected": 0` 到 json!（实现时以 `cargo check` 为准，二者都可）。

- [ ] **Step 6: 连带改 `chunk_request_from_chat_patch`（chat.rs:1823-1852）**

删六字段赋值行：`routing_card: s(patch, "routingCard"),`（:1833）、`safe_claims: arr(patch, "safeClaims"),`（:1836）、`forbidden_claims: arr(patch, "forbiddenClaims"),`（:1837）、`evidence_items: arr(patch, "evidenceItems"),`（:1838）、`unsupported_claims: vec![],`（:1846）、`verified_claims: vec![],`（:1847）。其余字段赋值不动。
> 不碰 chat 裸 `$set` 旁路 `apply_update_chunk`（:1720-1752，字符串 key，独立）。

- [ ] **Step 7: 连带改 `page_merge.rs` union 死键（:51-61）**

`DEFAULT_UNION_ARRAY_KEYS` 删 `"safe_claims",`（:59）、`"forbidden_claims",`（:60）两行。保留其它键（tags/search_terms/sources/applicable_scenes/not_applicable_scenes/business_topics/product_tags）。

- [ ] **Step 8: 编译确认（绿）**

Run: `cargo check --tests 2>&1 | tail -8`
Expected: EXIT 0，无 error、无 warning（尤其确认删 `#[allow(dead_code)]` 后无 dead_code、删字段后无 unused import/field）。若报某处仍引用已删字段（计划未预见的读取点），STOP 报告——不要臆造修法。

- [ ] **Step 9: 跑目标测试 + 全量 lib（绿）**

Run:
- `cargo test --lib preview_no_source_is_needs_review 2>&1 | tail -6` → PASS
- `cargo test --lib preview_anchor_match_never_auto_verifies 2>&1 | tail -6` → PASS
- `cargo test --lib 2>&1 | tail -5` → ≥350/0
Expected: 全绿。特别关注原 `coerce_d2_*` 三个测试（mod.rs:1765/1792/1806，断言 distortion_risks）不回归——本次保留 distortion_risks，应不受影响。

- [ ] **Step 10: 禁词 lint**

Run: `bash scripts/check-no-human-takeover.sh HEAD~1 HEAD 2>&1 | tail -3`
Expected: 0 violations（本次主要删代码；新增注释用「运营」，无"人工"）。

- [ ] **Step 11: Commit**

```bash
git add src/routes/knowledge/mod.rs src/routes/knowledge/chat.rs src/knowledge_wiki/page_merge.rs
git commit -m "refactor(knowledge): 清理请求体死字段(正规层),删恒不可达rejected分支"
```

---

## Self-Review

**1. Spec 覆盖**（逐 §核对）：
- §2.1 删请求体六字段 → Task 1 Step 3 ✅（删 `#[allow(dead_code)]` 依据已在 Step 3 亲验说明）
- §2.2 简化 apply（删死分支3、无新判据、逐字节等价）→ Step 4 ✅
- §2.3 简化 preview → Step 5 ✅（保留 distortion_risks/sourceAnchors/integrityStatus/confidenceScore）
- §2.4 连带 chat 构造函数 → Step 6 ✅（不碰裸 $set 旁路）
- §2.5 page_merge union 死键 → Step 7 ✅
- §2.6 测试改写（rejected→needs_review，改名 preview_no_source_is_needs_review）→ Step 1 ✅
- §5 不做（prompt/consultative/chat旁路/catalog）→ 计划无任何 Step 触碰 ✅

**2. Placeholder 扫描**：无 TBD/TODO；每个改代码 Step 有完整代码块与确切 file:line。Step 5 的 `let rejected` unused 处理给了两个明确可选项（allow(unused) 或内联），非含糊 placeholder ✅

**3. 类型一致性**：删的六字段名全程一致（routing_card/safe_claims/forbidden_claims/evidence_items/unsupported_claims/verified_claims）；保留字段（distortion_risks/source_quote/source_anchors/integrity_status/confidence_score/wiki_type/chunk_type）不动；`apply_chunk_integrity`/`integrity_report_for_preview` 签名不变（仅函数体简化）✅

**4. 单 Task 合理性**：删字段触发连锁编译失败，Step 3-7 必须同批改才能编译过，故合并单 Task。Step 8 `cargo check --tests` 是编译门，Step 9 是行为门，符合"独立可测交付物"。
