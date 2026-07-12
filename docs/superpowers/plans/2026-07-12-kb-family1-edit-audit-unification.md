# 批B家族① 修复实施计划：知识编辑统一接回 apply_chunk_revision + locked_fields 后端强制（KB-09/10/11）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 让知识 chunk 内容编辑的两条绕过路径（AI 会话应用草稿 apply_update_chunk、admin PUT）都获得 chunk_revisions 审计 + locked_fields 后端强制，兑现 apply_chunk_revision "唯一编辑落库入口 + 尊重运营字段锁" 的设计声称（KB-09/10/11）。

**Architecture:** 三组件。组件1（KB-11 核心枢纽）：apply_chunk_revision 把 `existing.locked_fields` 并入 `enforce_locked_fields`(:234) 的锁集（**只进静默覆盖，不进 apply_field_patch:173 硬拒集**——避免锁定字段连坐毙整条编辑）。组件2（KB-09）：apply_update_chunk 落库改走 `RevisionRequest{op:Patch, source:Ai}`。组件3（KB-10）：admin PUT 保留 replace_one，前补锁字段强制（复用同一 enforce_locked_fields）+ 后补 revision 审计行。组件1 是 2/3 的共同底座，先做。

**Tech Stack:** Rust 2021 / Axum / MongoDB。纯函数 `cargo test --lib`；接回行为集成测 `#[ignore]` CI Docker。

## Global Constraints

- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——每个改码 Task 的 Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `src/knowledge_wiki/chunk_revisions.rs`（组件1）、`src/routes/knowledge/chat.rs`（组件2 apply_update_chunk）、`src/routes/knowledge/crud.rs`（组件3 PUT）+ 相应测试。**不动** apply_field_patch 硬拒语义 / DEFAULT_LOCKED_FIELDS 常量 / union_array_fields / resolve_quote_anchors / compute_chunk_hash（只复用）/ PUT 的 apply_chunk_integrity·coerce_d2·preserve_unmodeled 链路 / 前端 locked_fields 表单 / apply_chunk_revision 其余步骤（70%截断·domain_schema·catalog_rebuild）。
- **关键一致性**：KB-10 与 apply_chunk_revision 复用**同一** `enforce_locked_fields` 纯函数，绝不各写一份锁字段逻辑（避免新 dual-path drift——修复本身不能再造旁路）。
- **红线保持**：KB-09 用 source=Ai → apply_chunk_revision 自动强制 draft+needs_review（"AI 永不自动 verify" 不破）。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。新增测试只增。
- **no-human-takeover lint**：新增行用「知识编辑/审计/锁定/修订」措辞，无禁词（接管/人工等）。
- **设计文档**：`docs/superpowers/specs/2026-07-12-kb-family1-edit-audit-unification-design.md`。台账 KB-09/10/11。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）

- `apply_chunk_revision`（chunk_revisions.rs:149）：existing_bson 在 :166 序列化；:173 `apply_field_patch(&existing_bson, &req.patch, DEFAULT_LOCKED_FIELDS)`；:190 union_array_fields；:234 `enforce_locked_fields(&merged, &existing_bson, DEFAULT_LOCKED_FIELDS)`。RevisionRequest{op,source,patch,reason,actor}(:128)。ProvenanceSource::{Ai,Human,Rule,Imported}(:70)，Ai 触发 :209 强制 draft+needs_review。
- `enforce_locked_fields`（page_merge.rs:140-153）：`for &k in locked { existing.get(k) → out.insert(k,v) | out.remove(k) }`——**静默覆盖**语义。`apply_field_patch`（:185-203）：patch 含任一 locked key → `Err(LockedFieldInPatch)`——**硬拒**语义。`DEFAULT_LOCKED_FIELDS`(:35，8字段)。现有单测 mod（:288）含 enforce_locked_overrides_when_merged_diverges(:334)/apply_patch_rejects_locked_field(:356)/apply_patch_overrides_non_locked_fields(:369)。
- `existing.locked_fields: Option<Vec<String>>`（models.rs:1618）；BSON 里字段名 `locked_fields`（snake，chunk 结构 serde 默认）。
- `apply_update_chunk`（chat.rs:1711-1795+）：camelCase→snake 映射(:1721-1752)构造 update_doc；source_quote→source_anchors 重算(:1767-1774，resolve_quote_anchors，写 `source_anchors` 复数)；手写 status/integrity/updated_at(:1776-1778)；`$set update_one`(:1779-1790)；返回 `{updatedChunkId,fieldsTouched,status,integrityStatus}`。**亲验** `source_anchors`(复数,models.rs:1571) ≠ DEFAULT 锁的 `source_anchor`(单数)。
- `update_operation_knowledge_chunk`（crud.rs:212-279）：apply_chunk_integrity(:241)/coerce_d2(:245)/find existing(:251)/operation_knowledge_chunk_from_request(:264)/preserve_unmodeled_chunk_fields(:265)/replace_one(:266-277)。
- 集成测基建：`tests/chunk_revision_ai_draft_integration.rs`（KB-09 AI draft 场景）、`tests/chunk_put_preserves_unmodeled_fields.rs`（KB-10 PUT 场景）现成可复用。

---

## Task 1: KB-11 —— existing.locked_fields 并入 enforce_locked_fields 后端强制（纯函数底座）

**Files:**
- Modify: `src/knowledge_wiki/chunk_revisions.rs`（apply_chunk_revision :234 锁集）
- Modify: `src/knowledge_wiki/page_merge.rs`（追加 KB-11 单测）

**Interfaces:**
- Consumes: `enforce_locked_fields(&Document,&Document,&[&str])`（page_merge.rs:140，静默覆盖，不改函数本身）。
- Produces: 行为层——apply_chunk_revision 尊重 existing.locked_fields（运营锁定字段任何路径写入都被覆盖回 existing 值）。

- [ ] **Step 1: 先读懂（红线）**

Read `chunk_revisions.rs:149-240`（apply_chunk_revision 主体，尤其 :166 existing_bson / :173 apply_field_patch / :234 enforce_locked_fields）+ `page_merge.rs:140-153`（enforce_locked_fields 静默覆盖）+ `page_merge.rs:185-203`（apply_field_patch 硬拒）+ `page_merge.rs:288-395`（现有单测范式）。Grep 亲验 `existing.locked_fields` BSON 字段名（`grep -n "locked_fields" src/models.rs`，确认 snake `locked_fields`）。**说不清就继续读。**

- [ ] **Step 2: 写失败单测（KB-11 底座语义）**

在 `page_merge.rs` 的 `#[cfg(test)] mod`（:288）内追加：

```rust
    #[test]
    fn enforce_locked_honors_runtime_locked_fields() {
        // KB-11：运营在 chunk 上锁定 "title"（内容字段，非 DEFAULT 集）→ merged 改了 title
        // → enforce_locked_fields 传入 DEFAULT ∪ ["title"] 时，title 被覆盖回 existing 值，
        //   未锁字段 summary 正常保留（静默覆盖，不毙整条）。
        let existing = doc! { "title": "旧标题", "summary": "旧摘要" };
        let merged = doc! { "title": "AI改的新标题", "summary": "AI改的新摘要" };
        let locked: Vec<&str> = vec!["title"]; // 模拟 DEFAULT ∪ existing.locked_fields 里的运营锁
        let out = enforce_locked_fields(&merged, &existing, &locked);
        assert_eq!(out.get_str("title").unwrap(), "旧标题", "锁定字段 title 须覆盖回 existing");
        assert_eq!(out.get_str("summary").unwrap(), "AI改的新摘要", "未锁字段 summary 正常保留");
    }
```

- [ ] **Step 3: 跑测试确认通过（纯函数本身已支持，此测证语义）**

Run: `cargo test --lib enforce_locked_honors_runtime 2>&1 | tail -8`
Expected: PASS（enforce_locked_fields 本就是静默覆盖，本测锁定其对"运行期传入锁集"的语义契约，作 KB-11 回归哨兵）。

- [ ] **Step 4: 接线 apply_chunk_revision —— existing.locked_fields 并入 :234 锁集**

在 apply_chunk_revision :234 的 enforce_locked_fields 调用**之前**，构造 effective_enforce_locked（从 existing_bson 读 locked_fields）：

```rust
    // KB-11：运营 per-chunk locked_fields 后端强制。existing.locked_fields 只并入
    // enforce_locked_fields（末次静默覆盖，锁定字段改动被丢弃、其余字段正常写），
    // **不并入 :173 apply_field_patch 的硬拒集**——否则 patch 碰锁定字段会整条 Err，
    // 连坐毙掉同一 patch 里的合法字段。DEFAULT_LOCKED_FIELDS 两处维持不变。
    let mut effective_enforce_locked: Vec<&str> = DEFAULT_LOCKED_FIELDS.to_vec();
    let runtime_locked: Vec<String> = existing_bson
        .get_array("locked_fields")
        .ok()
        .map(|a| {
            a.iter()
                .filter_map(|b| b.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    effective_enforce_locked.extend(runtime_locked.iter().map(|s| s.as_str()));
```
把 :234 改为 `let mut merged = enforce_locked_fields(&merged, &existing_bson, &effective_enforce_locked);`。:173 apply_field_patch 仍传 `DEFAULT_LOCKED_FIELDS`（不变）。

- [ ] **Step 5: cargo check + baseline**

Run: `cargo check --lib 2>&1 | tail -15` → 0 error。
Run: `cargo test --lib 2>&1 | tail -8` → `ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/knowledge_wiki/chunk_revisions.rs src/knowledge_wiki/page_merge.rs
git commit -m "fix(knowledge): apply_chunk_revision 后端强制运营 locked_fields (KB-11)

existing.locked_fields 并入 enforce_locked_fields(:234) 末次静默覆盖锁集(锁定字段
改动被丢弃、其余字段正常写),不并入 apply_field_patch 硬拒集(避免锁定字段连坐毙整条 patch)。
DEFAULT_LOCKED_FIELDS 两处不变。字段锁后端从此真生效,任何写入路径(AI补丁/PUT/API直调)都拦。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: KB-09 —— apply_update_chunk 接回 apply_chunk_revision（source=Ai）

**Files:**
- Modify: `src/routes/knowledge/chat.rs`（apply_update_chunk 落库段）

**Interfaces:**
- Consumes: `apply_chunk_revision`、`RevisionRequest`、`RevisionOp::Patch`、`ProvenanceSource::Ai`、`RevisionApplied`（组件1 已强化 locked_fields）。
- Produces: apply_update_chunk 落库经统一入口，获审计+union+锁字段+draft红线，返回形状不变。

- [ ] **Step 1: 先读懂（红线）**

Read `chat.rs:1711-1800`（apply_update_chunk 全函数）+ `chunk_revisions.rs:128-165`（RevisionRequest/apply_chunk_revision 签名 + RevisionApplied 结构）+ 一个现有调用范式（`grep -n "RevisionRequest {" src/routes/knowledge/wiki_edit.rs | head`，看 op/source/patch/reason/actor 怎么填）。亲验 RevisionApplied 有哪些字段可拼回 `{updatedChunkId,fieldsTouched,status,integrityStatus}`。**说不清就继续读。**

- [ ] **Step 2: 改造落库段**

保留 :1721-1774（camelCase→snake 映射构造 update_doc + source_quote→source_anchors 重算，anchors 已在 update_doc 里）。**删除** :1776-1790（手写 status/integrity/updated_at + `$set update_one`），替换为：

```rust
    // KB-09：落库改走统一入口 apply_chunk_revision（op=Patch, source=Ai）——获 chunk_revisions
    // 审计行 + 数组字段 union（既有 tag 不被整体替换丢弃）+ locked_fields 守门（KB-11）；
    // source=Ai 自动强制 status=draft + integrity_status=needs_review（"AI 永不自动 verify"红线不破）。
    // update_doc 已含 patch 前重算的 source_anchors（复数,不撞 DEFAULT 锁的 source_anchor 单数）。
    let applied = crate::knowledge_wiki::chunk_revisions::apply_chunk_revision(
        &state.db,
        workspace_id,
        oid,
        crate::knowledge_wiki::chunk_revisions::RevisionRequest {
            op: crate::knowledge_wiki::chunk_revisions::RevisionOp::Patch,
            source: crate::knowledge_wiki::chunk_revisions::ProvenanceSource::Ai,
            patch: update_doc,
            reason: Some("知识对话应用草稿".to_string()),
            actor: Some("knowledge_chat".to_string()),
        },
    )
    .await?;
    let _ = applied; // 若返回值字段用于拼装响应则取用；否则回填固定 draft/needs_review 形状
    Ok(json!({
        "updatedChunkId": chunk_id,
        "fieldsTouched": fields_touched,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
```
（实现者按 Step 1 亲验的 RevisionApplied 真实字段决定 status/integrityStatus 取 applied 还是固定值——source=Ai 恒 draft+needs_review，固定值语义正确。）

- [ ] **Step 3: cargo check + baseline**

Run: `cargo check --lib 2>&1 | tail -15` → 0 error（注意 import 路径，可能需 `use` 或全路径调用）。
Run: `cargo test --lib 2>&1 | tail -8` → N ≥ 350 / 0 failed。

- [ ] **Step 4: Commit**

```bash
git add src/routes/knowledge/chat.rs
git commit -m "fix(knowledge): apply_update_chunk 接回 apply_chunk_revision,补审计+数组union (KB-09)

AI 会话应用草稿落库从自建 \$set update_one 改走统一入口(op=Patch,source=Ai):获 chunk_revisions
审计行 + applicable_scenes/product_tags 等数组 union(既有运营 tag 不被整体替换丢弃)+ locked_fields
守门。保留 source_quote→source_anchors 重算(patch 前)。source=Ai 自动 draft+needs_review 红线不破。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: KB-10 —— admin PUT 补锁字段强制 + revision 审计行（保留 replace_one）

**Files:**
- Modify: `src/routes/knowledge/crud.rs`（update_operation_knowledge_chunk）

**Interfaces:**
- Consumes: `enforce_locked_fields`（复用同一纯函数）、`compute_chunk_hash`、`DEFAULT_LOCKED_FIELDS`、chunk_revisions 集合写入（参照 apply_chunk_revision 的 revision 行结构）。
- Produces: PUT 后 chunk_revisions 多一行审计（op=Patch/source=Human）+ 运营锁定字段不被 PUT 覆盖。

- [ ] **Step 1: 先读懂（红线）**

Read `crud.rs:212-279`（PUT 全函数）+ `chunk_revisions.rs` 里 apply_chunk_revision 写 chunk_revisions 行的段落（`grep -n "chunk_revisions()" src/knowledge_wiki/chunk_revisions.rs`，看 revision 行的字段结构：chunk_id/op/source/before_hash/after_hash/created_by/reason/created_at 等真实字段名）+ `page_merge.rs:140`（enforce_locked_fields）。亲验 next（:264）是 Document 还是 typed struct（决定能否直接喂 enforce_locked_fields/compute_chunk_hash——它们收 &Document）。**说不清就继续读。**

- [ ] **Step 2: 锁字段强制（replace 前）**

在 next 构造（:264）+ preserve_unmodeled（:265）之后、replace_one（:266）之前插入：把 existing 与 next 转 Document（若非），构造 `DEFAULT_LOCKED_FIELDS ∪ existing.locked_fields`，调 `enforce_locked_fields(&next_doc, &existing_doc, &effective_locked)` 得锁字段回填后的 doc 用于 replace。（复用 Task 1 同款 effective_locked 构造逻辑——考虑抽一个 `fn effective_locked_fields(existing: &Document) -> Vec<String>` 到 page_merge 或 chunk_revisions 供两处共用，避免 dual-path；实现者按 Step 1 亲验的类型决定最简接法。）

- [ ] **Step 3: 补 revision 审计行（replace 后，fail-soft）**

replace_one 成功后，按 Step 1 亲验的 chunk_revisions 行结构补写一条：op=patch, source=human, before_hash=compute_chunk_hash(&existing_doc), after_hash=compute_chunk_hash(&replaced_doc), created_by=admin.user_id, reason=Some("admin 直接编辑")。**fail-soft**：审计写失败 `let _`/记 warn，不回滚 replace、不返 Err（replace 已成功数据正确，审计缺一行是可观测运维问题）。

- [ ] **Step 4: cargo check + baseline**

Run: `cargo check --lib 2>&1 | tail -15` → 0 error。
Run: `cargo test --lib 2>&1 | tail -8` → N ≥ 350 / 0 failed。

- [ ] **Step 5: Commit**

```bash
git add src/routes/knowledge/crud.rs src/knowledge_wiki/page_merge.rs
git commit -m "fix(knowledge): admin PUT 补 revision 审计行 + 锁字段强制 (KB-10)

保留 replace_one 整条替换语义,replace 前复用同一 enforce_locked_fields 把运营锁定字段
从 existing 覆盖回 next(不造新 dual-path),replace 后补写一条 chunk_revisions(op=patch/
source=human/before-after hash,fail-soft 不回滚)。审计链补齐 + 字段锁 PUT 路径兑现。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 集成测（KB-09/KB-10 落库行为）+ baseline + PR

**Files:**
- Modify: `tests/chunk_revision_ai_draft_integration.rs`（KB-09）+ `tests/chunk_put_preserves_unmodeled_fields.rs`（KB-10），或新增测按现有基建范式。

- [ ] **Step 1: 先读懂（红线）**

Read `tests/chunk_revision_ai_draft_integration.rs` 全文 + `tests/chunk_put_preserves_unmodeled_fields.rs` 全文（helper/TestApp/seed chunk 范式）。亲验如何 seed 一个带 locked_fields 的 chunk、如何调 apply_update_chunk / PUT handler、如何 count chunk_revisions。**说不清就继续读。**

- [ ] **Step 2: 追加 KB-09 集成测**

在 `chunk_revision_ai_draft_integration.rs` 追加 `#[tokio::test] #[ignore]`：seed 一个 chunk（含既有 product_tags=["A"]）→ 调 apply_update_chunk（patch product_tags=["B"]）→ 断言：(1) chunk_revisions 多一行；(2) chunk.product_tags 是 union ["A","B"] 非替换 ["B"]；(3) status=draft。

- [ ] **Step 3: 追加 KB-10 + KB-11 集成测**

在 `chunk_put_preserves_unmodeled_fields.rs` 追加 `#[tokio::test] #[ignore]`：seed chunk（locked_fields=["title"], title="锁定标题"）→ admin PUT 改 title="试图改" + summary="新摘要" → 断言：(1) chunk_revisions 多一行（op=patch/source=human）；(2) chunk.title 仍="锁定标题"（KB-11 后端强制生效）；(3) chunk.summary="新摘要"（未锁字段正常）。

- [ ] **Step 4: 编译 + lint + baseline**

Run: `cargo test --test chunk_revision_ai_draft_integration --no-run 2>&1 | tail -10` 与 `cargo test --test chunk_put_preserves_unmodeled_fields --no-run 2>&1 | tail -10` → 0 error（不本地跑，CI Docker 跑）。
Run: `git diff origin/main -- src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"` → lint clean。
Run: `cargo test --lib 2>&1 | tail -8` → N ≥ 350 / 0 failed。

- [ ] **Step 5: Commit（主控做 push+PR）**

```bash
git add tests/chunk_revision_ai_draft_integration.rs tests/chunk_put_preserves_unmodeled_fields.rs
git commit -m "test(knowledge): 批B家族① 集成测(KB-09 union+审计 / KB-10 PUT审计 / KB-11 锁字段强制)

Co-Authored-By: Claude <noreply@anthropic.com>"
```
**实现者到此止,不 push/不开 PR**（主控做）。

---

## Self-Review 结论

- **Spec coverage**：组件1(KB-11)↔Task1；组件2(KB-09)↔Task2；组件3(KB-10)↔Task3；测试策略↔Task1 Step2/Task4。全覆盖。
- **一致性**：Task1/Task3 复用同一 enforce_locked_fields + effective_locked 构造（Task3 Step2 建议抽共用 fn 避免 dual-path）；KB-09 source=Ai 保 draft 红线；KB-11 existing.locked 只进 :234 不进 :173（亲验 apply_field_patch 硬拒后的修正）。
- **Placeholder scan**：无 TBD；每 Step 给可编译代码/确切命令/预期。
- **Type consistency**：RevisionRequest{op:Patch,source:Ai}、enforce_locked_fields(&Document,&Document,&[&str])、compute_chunk_hash(&Document) 与亲验签名一致；source_anchors 复数不撞 source_anchor 单数锁。
- **红线**：每改码 Task Step1 先读懂+亲验行号；范围边界明确圈出不动项。
