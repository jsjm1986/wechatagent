# 知识库 grounding 漏判兜底 + 人工后门 D2 收口 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 收口两处知识库红线：A=发送侧 grounding 漏判（reviewer 漏报时按词类型切分硬闸兜底 + prompt 根因），B=写入侧人工后门 create/PUT 绕过 D2 verify 闸。

**Architecture:** 两条独立链。B 先行（纯函数 + 两个 handler 接入点，零 LLM）。A 随后（guards 纯函数分类器 → review 探针分支改造 → prompt 根因文案）。全部复用既有闭集枚举与既有 D2 纯函数，不新增枚举、不动 `apply_chunk_integrity` 本体。

**Tech Stack:** Rust 2021 / Axum / MongoDB（BSON serde）；测试 `cargo test --lib`（本地纯函数）。

设计依据：`docs/superpowers/specs/2026-06-06-knowledge-grounding-and-verify-gate-fixes-design.md`

---

## 文件结构

| 文件 | 责任 | 改动 |
|---|---|---|
| `src/routes/knowledge.rs` | B：D2 收口纯函数 + create/PUT 接入 + 单测 | Modify |
| `src/agent/guards.rs` | A：承诺词类型分类纯函数 + 单测 | Modify |
| `src/agent/review.rs` | A：探针分支改为「先硬闸后观测」+ 单测 | Modify |
| `src/prompts.rs` | A：reviewer system 增补 requiresProductKnowledge 反向锚点 | Modify |

---

# B 链：人工后门 D2 收口（先做，纯函数，零 LLM）

### Task B1：D2 收口纯函数 `coerce_integrity_against_d2_gate`

**Files:**
- Modify: `src/routes/knowledge.rs`（新增函数，建议紧邻 `chunk_verify_gate_reason`，约 `:3470` 之后）
- Test: 同文件 `#[cfg(test)]` 区（append）

- [ ] **Step 1: 写失败测试**

在 `src/routes/knowledge.rs` 的测试模块（文件已有 `#[cfg(test)] mod tests`；若有多个，选含 `chunk_verify_gate_reason` 或 `apply_chunk_integrity` 相关断言的那个）append：

```rust
#[test]
fn coerce_d2_downgrades_verified_without_quote() {
    let mut p = OperationKnowledgeChunkRequest {
        title: "t".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: None,
        source_anchors: vec![mongodb::bson::doc! { "startOffset": 0i64 }],
        ..Default::default()
    };
    coerce_integrity_against_d2_gate(&mut p);
    assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
    assert!(p.distortion_risks.iter().any(|r| r.contains("D2")));
}

#[test]
fn coerce_d2_downgrades_verified_without_anchor() {
    let mut p = OperationKnowledgeChunkRequest {
        title: "t".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some("原文引用".to_string()),
        source_anchors: vec![],
        ..Default::default()
    };
    coerce_integrity_against_d2_gate(&mut p);
    assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
}

#[test]
fn coerce_d2_keeps_verified_with_quote_and_anchor() {
    let mut p = OperationKnowledgeChunkRequest {
        title: "t".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some("原文引用".to_string()),
        source_anchors: vec![mongodb::bson::doc! { "startOffset": 0i64 }],
        ..Default::default()
    };
    coerce_integrity_against_d2_gate(&mut p);
    assert_eq!(p.integrity_status.as_deref(), Some("verified"));
    assert!(p.distortion_risks.is_empty());
}

#[test]
fn coerce_d2_ignores_non_verified() {
    let mut p = OperationKnowledgeChunkRequest {
        title: "t".to_string(),
        integrity_status: Some("needs_review".to_string()),
        source_quote: None,
        source_anchors: vec![],
        ..Default::default()
    };
    coerce_integrity_against_d2_gate(&mut p);
    assert_eq!(p.integrity_status.as_deref(), Some("needs_review"));
    assert!(p.distortion_risks.is_empty());
}
```

> 注：`OperationKnowledgeChunkRequest` 定义在 `src/routes/knowledge.rs:147`（`#[derive(Debug, Default, Deserialize)]`，字段私有），不在 models.rs。字段私有但本测试在**同文件** `#[cfg(test)] mod tests` 内，可字面量构造 + `..Default::default()`。字段名以 `:150-193` 定义为准（`source_quote: Option<String>` / `source_anchors: Vec<Document>` / `integrity_status: Option<String>` / `distortion_risks: Vec<String>`，已核验一致）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib coerce_d2 2>&1 | tail -20`
Expected: 编译失败 `cannot find function coerce_integrity_against_d2_gate`

- [ ] **Step 3: 写最小实现**

在 `src/routes/knowledge.rs`（`chunk_verify_gate_reason` 函数之后）新增：

```rust
/// 人工后门 D2 收口：create/PUT chunk 落库前，若调用方提交 `integrity_status="verified"`
/// 但缺 sourceQuote 或 source_anchors（未过 D2 闸），降级为 needs_review 并留审计痕迹。
/// 与 import 路径「锚点只作审核线索、最终 needs_review」语义一致；正路仍是走 /verify。
pub(super) fn coerce_integrity_against_d2_gate(payload: &mut OperationKnowledgeChunkRequest) {
    if payload.integrity_status.as_deref() != Some("verified") {
        return;
    }
    let has_quote = payload
        .source_quote
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_anchor = !payload.source_anchors.is_empty();
    if chunk_verify_gate_reason(has_quote, has_anchor).is_some() {
        payload.integrity_status = Some("needs_review".to_string());
        payload.distortion_risks.push(
            "提交为 verified 但缺 sourceQuote/source_anchors，未过 D2 闸，已降级 needs_review"
                .to_string(),
        );
    }
}
```

> `chunk_verify_gate_reason` 当前是 `fn`（私有），与本函数同模块可直接调用，无需改其可见性。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib coerce_d2 2>&1 | tail -20`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: 提交**

```bash
git add src/routes/knowledge.rs
git commit -m "feat(knowledge): D2 收口纯函数 coerce_integrity_against_d2_gate(人工后门 verified 必过 quote+anchor 闸)"
```

---

### Task B2：create / PUT handler 接入 D2 收口

**Files:**
- Modify: `src/routes/knowledge.rs:448`（create，`validate_*` 之后）
- Modify: `src/routes/knowledge.rs:491`（PUT，`apply_chunk_integrity` 之后）

- [ ] **Step 1: create handler 接入**

`create_operation_knowledge_chunk`（`:443`）当前 body：

```rust
    validate_operation_knowledge_chunk(&payload)?;
    let result = state
        .db
        .operation_knowledge_chunks()
        .insert_one(
```

改为在 `validate` 后、`insert_one` 前插一行（注意 `payload` 需 `mut`——当前签名是 `Json(payload)`，改为 `Json(mut payload)`）：

```rust
    validate_operation_knowledge_chunk(&payload)?;
    coerce_integrity_against_d2_gate(&mut payload);
    let result = state
        .db
        .operation_knowledge_chunks()
        .insert_one(
```

把函数签名 `Json(payload): Json<OperationKnowledgeChunkRequest>,` 改为 `Json(mut payload): Json<OperationKnowledgeChunkRequest>,`。

- [ ] **Step 2: PUT handler 接入**

`update_operation_knowledge_chunk`（`:462`，签名已是 `Json(mut payload)`）当前在 `:490-492`：

```rust
            if let Some(raw) = document.raw_content.as_deref() {
                apply_chunk_integrity(&mut payload, raw, Some(document_id));
            }
```

在这个 `if let Some(document) { ... }` 块**之后**、`replace_one` 之前，无条件插一行（放在 document 块外，确保即使无父文档、调用方直接传 verified 也收口）：

```rust
    coerce_integrity_against_d2_gate(&mut payload);
    state
        .db
        .operation_knowledge_chunks()
        .replace_one(
```

即定位到 `:495` 的 `state\n.db\n.operation_knowledge_chunks()\n.replace_one(` 前插入 `coerce_integrity_against_d2_gate(&mut payload);`。

- [ ] **Step 3: 编译 + 全量 lib 测试**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: 编译干净，`test result: ok`，lib 通过数 ≥ 350、0 failed。

- [ ] **Step 4: no-human-takeover lint 自查**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`（Windows 用 `pwsh scripts/check-no-human-takeover.ps1`）
Expected: 通过（新增文案无禁词；本任务未引入「人工/接管」字样）。

- [ ] **Step 5: 提交**

```bash
git add src/routes/knowledge.rs
git commit -m "feat(knowledge): create/PUT chunk 落库前过 D2 收口(verified 缺 quote/anchor 降级 needs_review)"
```

---

# A 链：grounding 漏判兜底（B 之后）

### Task A1：承诺词类型分类纯函数 `commitment_claim_class`

**Files:**
- Modify: `src/agent/guards.rs`（`reply_contains_commitment_claim` 之后，`:333` 后）
- Test: 同文件测试模块（append）

- [ ] **Step 1: 写失败测试**

`src/agent/guards.rs` 已有 `#[cfg(test)] mod policy_tests`（`:335`）。在其中 append（`super::*` 已 use）：

```rust
    #[test]
    fn commitment_class_product_effect_on_data_words() {
        assert_eq!(commitment_claim_class("我们的成功率高达95%"), CommitmentClass::ProductEffect);
        assert_eq!(commitment_claim_class("三天就见效"), CommitmentClass::ProductEffect);
        assert_eq!(commitment_claim_class("保证按时回款"), CommitmentClass::ProductEffect);
    }

    #[test]
    fn commitment_class_tone_only_on_soft_words() {
        assert_eq!(commitment_claim_class("我保证认真对待您的问题"), CommitmentClass::ToneOnly);
        assert_eq!(commitment_claim_class("这事绝对不怪你"), CommitmentClass::ToneOnly);
        assert_eq!(commitment_claim_class("这个方案一定能帮到你"), CommitmentClass::ToneOnly);
    }

    #[test]
    fn commitment_class_product_effect_wins_when_both_present() {
        // 同时含语气词「一定能」和效果词「成功率」→ 取更危险的 ProductEffect
        assert_eq!(commitment_claim_class("一定能把成功率做上去"), CommitmentClass::ProductEffect);
    }

    #[test]
    fn commitment_class_none_on_plain_reply() {
        assert_eq!(commitment_claim_class("好的，我先了解下你的具体情况"), CommitmentClass::None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib commitment_class 2>&1 | tail -20`
Expected: 编译失败 `cannot find type CommitmentClass` / `cannot find function commitment_claim_class`

- [ ] **Step 3: 写最小实现**

`src/agent/guards.rs`，`reply_contains_commitment_claim`（`:333`）之后新增：

```rust
/// 承诺词类型（grounding 漏判兜底硬闸用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitmentClass {
    /// 效果/数据类断言（成功率/见效/回款/百分比）——漏判+无 verified 时硬闸拦截。
    ProductEffect,
    /// 语气类承诺（保证/一定能/绝对）——最易误杀情感承诺，仅观测不拦。
    ToneOnly,
    /// 无承诺词。
    None,
}

/// 把候选回复按承诺词类型分类。ProductEffect 优先（同时命中两类时取更危险者）。
/// 与 `reply_contains_commitment_claim` 的 8 词同源，但切分两类以控制误杀：
/// 效果/数据类几乎只出现在可验证产品断言；语气类大量出现在情感/口语承诺。
pub(crate) fn commitment_claim_class(reply_text: &str) -> CommitmentClass {
    const PRODUCT_EFFECT_MARKERS: [&str; 5] =
        ["成功率", "见效", "回款", "百分之", "百分百"];
    const TONE_ONLY_MARKERS: [&str; 3] = ["保证", "一定能", "绝对"];
    let text = reply_text.trim();
    if text.is_empty() {
        return CommitmentClass::None;
    }
    if PRODUCT_EFFECT_MARKERS.iter().any(|m| text.contains(m)) {
        return CommitmentClass::ProductEffect;
    }
    if TONE_ONLY_MARKERS.iter().any(|m| text.contains(m)) {
        return CommitmentClass::ToneOnly;
    }
    CommitmentClass::None
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib commitment_class 2>&1 | tail -20`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: 提交**

```bash
git add src/agent/guards.rs
git commit -m "feat(guards): 承诺词类型分类 commitment_claim_class(效果类硬闸/语气类仅观测)"
```

---

### Task A2：review 探针分支改为「先硬闸后观测」

**Files:**
- Modify: `src/agent/review.rs:963-999`（现有「item ①先观测」分支）
- Test: `src/agent/review.rs` 探针单测模块（`:2206` 一带，append）

- [ ] **Step 1: 写失败测试**

在 `src/agent/review.rs` 探针单测区（`finalize_no_grounding_probe_when_reply_has_no_commitment` 之后，`:2293` 后）append：

```rust
    #[test]
    fn finalize_blocks_on_product_effect_claim_when_reviewer_missed() {
        // reviewer 漏判 + 回复含效果词「回款」+ 无 verified → 兜底硬闸 block。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": false };
        let mut decision = shouldreply_decision();
        decision.reply_text = "放心，我们保证按时回款".to_string();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review, &mut decision, &runtime, &contact, &[], Vec::new(),
            "你们能保证回款吗",
        );
        assert_eq!(outcome.status, GatewayStatusFinal::BlockedUnverifiedProductClaim);
        assert!(!outcome.review.approved);
        assert!(!decision.should_reply);
        assert!(outcome.pending_events.iter()
            .any(|e| e.kind == "product_claim_blocked_by_probe_fallback" && e.status == "blocked"));
    }

    #[test]
    fn finalize_only_observes_on_tone_only_claim_when_reviewer_missed() {
        // reviewer 漏判 + 回复仅含语气词「保证」(无效果词) + 无 verified → 不拦，仅观测。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": false };
        let mut decision = shouldreply_decision();
        decision.reply_text = "我保证会认真对待你的问题".to_string();
        let contact = finalize_contact();
        let outcome = finalize_review_for_send(
            review, &mut decision, &runtime, &contact, &[], Vec::new(),
            "你会上心吗",
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(outcome.review.approved);
        assert!(decision.should_reply);
        assert!(outcome.pending_events.iter()
            .any(|e| e.kind == "grounding_probe_reviewer_missed" && e.status == "observe"));
        assert!(!outcome.pending_events.iter()
            .any(|e| e.kind == "product_claim_blocked_by_probe_fallback"));
    }

    #[test]
    fn finalize_probe_fallback_skipped_when_verified_present() {
        // reviewer 漏判 + 回复含效果词「成功率」+ 有 verified 交集 → 不误伤,放行。
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review();
        review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": false };
        let mut decision = shouldreply_decision();
        decision.reply_text = "我们的成功率确实不错".to_string();
        decision.used_knowledge_ids = vec![probe_verified_chunk_id()];
        let contact = finalize_contact();
        let chunks = vec![probe_verified_chunk()];
        let outcome = finalize_review_for_send(
            review, &mut decision, &runtime, &contact, &chunks, Vec::new(),
            "成功率怎么样",
        );
        assert_eq!(outcome.status, GatewayStatusFinal::Approved);
        assert!(decision.should_reply);
    }
```

> `probe_verified_chunk()` / `probe_verified_chunk_id()`：构造一个 `integrity_status="verified"`、`id=Some(oid)` 的 `OperationKnowledgeChunk`，其 hex id 与 `used_knowledge_ids` 一致。若测试模块已有等价 helper（查 `:2125-2190` R5.4 既有 verified chunk 测试用的构造），直接复用；否则在测试模块加：

```rust
    fn probe_verified_chunk_id() -> String {
        "5f000000000000000000aa01".to_string()
    }
    fn probe_verified_chunk() -> OperationKnowledgeChunk {
        let mut c = OperationKnowledgeChunk::default();
        c.id = Some(mongodb::bson::oid::ObjectId::parse_str(probe_verified_chunk_id()).unwrap());
        c.integrity_status = Some("verified".to_string());
        c
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib finalize_blocks_on_product_effect 2>&1 | tail -20`
Expected: FAIL —— 现状是 observe 不 block，`outcome.status` 为 `Approved` 而非 `BlockedUnverifiedProductClaim`。

- [ ] **Step 3: 改造探针分支**

`src/agent/review.rs` 现有分支（`:975-999`）：

```rust
    if !super::guards::claim_requires_product_knowledge(&review.claim_analysis)
        && super::guards::reply_contains_commitment_claim(&decision.reply_text)
    {
        let verified =
            super::guards::compute_verified_chunks(&decision.used_knowledge_ids, knowledge_chunks);
        if verified.is_empty() {
            let mut details = Document::new();
            details.insert(
                "reply_excerpt",
                decision.reply_text.chars().take(80).collect::<String>(),
            );
            details.insert("used_knowledge_ids", decision.used_knowledge_ids.clone());
            details.insert("knowledge_chunk_total", knowledge_chunks.len() as i64);
            pending_events.push(PendingFinalizeEvent {
                kind: "grounding_probe_reviewer_missed".to_string(),
                status: "observe".to_string(),
                summary:
                    "观测：回复含绝对化产品承诺且无 verified 背书，但 reviewer 未标 requiresProductKnowledge"
                        .to_string(),
                details,
            });
        }
    }
```

整体替换为（先按词类型分流：ProductEffect→硬闸 return；ToneOnly→维持观测）：

```rust
    if !super::guards::claim_requires_product_knowledge(&review.claim_analysis) {
        let class = super::guards::commitment_claim_class(&decision.reply_text);
        if class != super::guards::CommitmentClass::None {
            let verified = super::guards::compute_verified_chunks(
                &decision.used_knowledge_ids,
                knowledge_chunks,
            );
            if verified.is_empty() {
                match class {
                    super::guards::CommitmentClass::ProductEffect => {
                        // 兜底硬闸：reviewer 漏判效果/数据类承诺且无 verified 背书 → block。
                        review.approved = false;
                        review.scores.hallucination_score =
                            review.scores.hallucination_score.max(6);
                        extend_risks_unique(
                            &mut review.risks,
                            std::iter::once(
                                "product_claim_without_verified_knowledge".to_string(),
                            ),
                        );
                        decision.should_reply = false;
                        decision.autonomy_mode = "blocked".to_string();
                        let mut details = Document::new();
                        details.insert(
                            "reply_excerpt",
                            decision.reply_text.chars().take(80).collect::<String>(),
                        );
                        details.insert("used_knowledge_ids", decision.used_knowledge_ids.clone());
                        details.insert("knowledge_chunk_total", knowledge_chunks.len() as i64);
                        pending_events.push(PendingFinalizeEvent {
                            kind: "product_claim_blocked_by_probe_fallback".to_string(),
                            status: "blocked".to_string(),
                            summary:
                                "兜底硬闸：reviewer 漏判，回复含效果/数据类承诺且无 verified 背书，强制 blocked"
                                    .to_string(),
                            details,
                        });
                        review.final_review_status =
                            "blocked_unverified_product_claim".to_string();
                        return FinalizeOutcome {
                            review,
                            status: GatewayStatusFinal::BlockedUnverifiedProductClaim,
                            pending_events,
                        };
                    }
                    super::guards::CommitmentClass::ToneOnly => {
                        // 语气类：维持现状，仅观测不拦（避免误杀情感承诺）。
                        let mut details = Document::new();
                        details.insert(
                            "reply_excerpt",
                            decision.reply_text.chars().take(80).collect::<String>(),
                        );
                        details.insert("used_knowledge_ids", decision.used_knowledge_ids.clone());
                        details.insert("knowledge_chunk_total", knowledge_chunks.len() as i64);
                        pending_events.push(PendingFinalizeEvent {
                            kind: "grounding_probe_reviewer_missed".to_string(),
                            status: "observe".to_string(),
                            summary:
                                "观测：回复含语气类承诺且无 verified 背书，但 reviewer 未标 requiresProductKnowledge"
                                    .to_string(),
                            details,
                        });
                    }
                    super::guards::CommitmentClass::None => {}
                }
            }
        }
    }
```

> `extend_risks_unique`、`Document`、`PendingFinalizeEvent`、`FinalizeOutcome`、`GatewayStatusFinal` 均已在 `review.rs` 现场可用（R5.4 硬闸 `:932-960` 用的是同一套）。

- [ ] **Step 4: 运行测试确认通过 + 既有探针单测不回归**

Run: `cargo test --lib grounding_probe 2>&1 | tail -20 && cargo test --lib finalize_ 2>&1 | tail -30`
Expected: 新增 3 个 PASS；既有 `finalize_emits_grounding_probe_on_reviewer_missed_commitment`（reply=「一定能」属 ToneOnly）仍 PASS（仍走 observe）、`finalize_no_grounding_probe_when_reviewer_already_flagged` 仍 PASS、`finalize_no_grounding_probe_when_reply_has_no_commitment` 仍 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/agent/review.rs
git commit -m "feat(review): grounding 漏判兜底硬闸(效果类承诺无 verified 强制 block;语气类仍仅观测)"
```

---

### Task A3：reviewer prompt 增补 requiresProductKnowledge 反向锚点（治本）

**Files:**
- Modify: `src/prompts.rs:1289` 一带（`user.review.system` 的 `content` 内，requiresProductKnowledge 说明附近）

- [ ] **Step 1: 定位现有 requiresProductKnowledge 指引**

`user.review.system`（`:1259` 起）的 `content` 已含 grounding 闸说明（`ProductAccuracyScore < 7`，`:1274`）。在 EmotionalValue 段（`:1289`）之前或产品闸说明附近，增补一句反向锚点。具体：在 `:1289` 行（`触发改写时 revisionDirection...`）之前插入一段。

- [ ] **Step 2: 插入反向锚点文案**

在 `user.review.system` 的 content 中（产品准确性相关说明之后）追加（注意保持 `r#"..."#` 原始字符串与既有中文风格一致，不引入 markdown 符号）：

```text
判 requiresProductKnowledge 时：候选回复只要含可被知识库验证的产品断言——效果数据（成功率、见效时间、回款、百分比）、具体价格、客户案例、能力承诺——无论语气是软是硬，都必须置 requiresProductKnowledge=true，交由 grounding 闸核对 verified 知识背书；只有纯情感承接 / 表达理解 / 轻量澄清问题（不含任何可验证产品断言）才置 false。
```

> 该 prompt seed 机制为「存在即跳过」，老库不变，CI 全新库自动生效（与 commit 5a78e94 第③件套同机制）。无纯函数单测，靠 real-LLM 套件观测。

- [ ] **Step 3: 编译 + lib 全量 + 基线门**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: 编译干净，lib ≥ 350 passed / 0 failed。

- [ ] **Step 4: no-human-takeover lint + 基线门**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5 && bash scripts/check-baseline.sh 2>&1 | tail -15`（Windows：`pwsh scripts/check-no-human-takeover.ps1` / `pwsh scripts/check-baseline.ps1`）
Expected: 两者皆通过（lib ≥ 350/0；4 PBT 累计 ≥ 33/0；无禁词）。

- [ ] **Step 5: 提交**

```bash
git add src/prompts.rs
git commit -m "feat(prompts): reviewer system 增补 requiresProductKnowledge 反向锚点(可验证产品断言一律 true)"
```

---

## 收尾验证（A+B 全部完成后）

- [ ] **全量 lib + PBT 基线门**

Run: `bash scripts/check-baseline.sh 2>&1 | tail -20`（Windows：`pwsh scripts/check-baseline.ps1`）
Expected: lib ≥ 350 passed / 0 failed；4 PBT 累计 ≥ 33 / 0 failed；`exit 0`。

- [ ] **no-human-takeover 终扫**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 通过。

- [ ] 整体回看：B 让人工后门 verified 必过 D2；A 让 reviewer 漏判的效果类产品承诺被兜底拦截、语气类不误杀、且 prompt 根因降低漏判率本身。

---

## Self-Review 检查记录

- **Spec coverage**：A.3 第一层 prompt 根因→Task A3；A.3 第二层词切分→Task A1；A.4 review 分支→Task A2；A.5 测试→A1/A2 各 step1；B.2 函数→Task B1；B.2 接入点→Task B2；B.5 测试→B1 step1。全覆盖。
- **Placeholder scan**：无 TBD/TODO；每个代码步给出完整代码；helper 构造给了 fallback 实现。
- **Type consistency**：`CommitmentClass`（A1 定义）→ A2 一致引用 `super::guards::CommitmentClass::{ProductEffect,ToneOnly,None}`；`commitment_claim_class` 签名 A1↔A2 一致；`coerce_integrity_against_d2_gate(&mut payload)` B1↔B2 一致；复用 `chunk_verify_gate_reason`/`GatewayStatusFinal::BlockedUnverifiedProductClaim`/`final_review_status="blocked_unverified_product_claim"`（审计已验存在）。
- **不变量**：未新增闭集枚举；未动 `apply_chunk_integrity` 本体；A 硬闸仅收窄到 5 个效果词，语气类 3 词仍仅观测。
