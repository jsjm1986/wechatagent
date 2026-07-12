# 批B家族① 修复设计：知识编辑统一接回 apply_chunk_revision + locked_fields 后端强制（KB-09/10/11）

- 日期：2026-07-12
- 分支：`fix/kb-family1-edit-audit`（基于最新 origin/main）
- 来源：深度审查批B 知识编辑审计/统一入口家族（KB-09 + KB-10 + KB-11；台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md:549-581`、总评 :624）
- 优先级：P2
- 严重度：均 Medium

## 根因家族（台账 :624，已主控逐条亲验最新 main 成立）

`apply_chunk_revision`（`src/knowledge_wiki/chunk_revisions.rs:149`）本应是知识 chunk 内容编辑的**唯一落库入口**——它提供：不可变审计行（before/after hash + chunk_revisions）、数组字段 union（existing ∪ patch，防历史 tag 丢失）、body 70% 截断守卫、AI source 强制 draft+needs_review、locked 字段守门。但两条内容编辑路径**绕过它直改主集合**，且 per-chunk `locked_fields` 后端从不强制。共性 = "设计声称统一，实现有旁路"。

- **KB-09**（Medium/CONFIRMED）：`apply_update_chunk`（`src/routes/knowledge/chat.rs:1711`，AI 会话「应用草稿」）自建 `$set update_one`（:1779-1790），全函数无 chunk_revisions 写入。后果：①审计链断；②applicable_scenes/product_tags/business_topics 是 `$set` 整体替换而非 union → 运营既有 tag 被 AI 补丁悄悄丢弃；③不读 locked_fields。status 强制 draft+needs_review（未破"AI 永不自动 verify"红线），workspace 隔离在位。
- **KB-10**（Medium/CONFIRMED）：`update_operation_knowledge_chunk`（`src/routes/knowledge/crud.rs:212`，admin PUT）用 replace_one 整条替换（:266-277），无 chunk_revisions 写入。workspace 隔离健全、preserve_unmodeled_chunk_fields 正确回填元字段。缺口 = 审计行缺失 + locked_fields 不强制。
- **KB-11**（Medium/CONFIRMED，需用户裁定 → 已裁定「后端强制」）：设计要求 apply_chunk_revision "尊重 locked_fields"，但三处守门（apply_field_patch:173 / enforce_locked_fields:234）只传编译期常量 `DEFAULT_LOCKED_FIELDS`（page_merge.rs:35，8 个身份/时间戳字段），对 `existing.locked_fields`（运营单条 chunk 字段锁，models.rs:1618 `Option<Vec<String>>`）无任何读取/强制点。字段锁当前只在前端表单禁用输入，后端零兜底。

## 用户裁定（brainstorming 澄清）

1. **KB-11 定位 = 后端强制**（按设计描述）：existing.locked_fields 并入后端守门，任何写入路径（AI 补丁 / admin PUT / API 直调）都拦。
2. **KB-10 接回方式 = 保留 replace_one + 补 revision 行 + 锁字段**（低风险，不动现有 replace 链路）。
3. **KB-09 anchors 重算位置 = patch 前重算，source_anchors 作为 patch 字段一并交入**。

## 设计

### 组件 1：KB-11 —— locked_fields 后端强制（核心枢纽）

**关键亲验（修正了初版设计的一个错误假设）**：`apply_field_patch`（page_merge.rs:185-203）对 patch 里出现**任何** locked 字段都是**硬拒 `LockedFieldInPatch`（Err）**——只有一套语义，没有"静默剥离"。而 `enforce_locked_fields`（page_merge.rs:140-153）才是"locked 字段用 existing 值覆盖 merged"的**静默覆盖**语义。

因此 existing.locked_fields 的接入点必须区分：
- **`DEFAULT_LOCKED_FIELDS`**（身份/时间戳）：维持现状，两处都传（apply_field_patch:173 硬拒 + enforce_locked_fields:234 覆盖）。patch 带身份字段仍硬拒——合理，没人该 patch 它们。
- **`existing.locked_fields`**（运营锁定的内容字段，如 title/body）：**只传给 :234 enforce_locked_fields（静默覆盖），不传给 :173 apply_field_patch（硬拒）**。

**语义**：运营锁了 title 后，AI 补丁同时改 title（锁定）+ summary（未锁）→ apply_field_patch 不拦（existing.locked 没进它）→ union → enforce_locked_fields 把 title 覆盖回 existing 值（title 改动被静默丢弃）→ summary 正常写。**锁定字段的改动被丢弃、其余字段正常写，不因锁定字段连坐毙掉整条合法编辑。**

**实现**（apply_chunk_revision，existing_bson 已在 :166 序列化好，从中读 locked_fields，无额外 DB 读）：
```rust
// DEFAULT 集：apply_field_patch(:173) 与 enforce_locked_fields 都用（现状不变）
// existing.locked_fields：仅并入 enforce_locked_fields(:234) 的锁集
let mut effective_enforce_locked: Vec<&str> = DEFAULT_LOCKED_FIELDS.to_vec();
let existing_locked: Vec<String> = existing_bson
    .get_array("locked_fields").ok()
    .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
    .unwrap_or_default();
effective_enforce_locked.extend(existing_locked.iter().map(|s| s.as_str()));
// :173 apply_field_patch 仍传 DEFAULT_LOCKED_FIELDS（不变）
// :234 改为 enforce_locked_fields(&merged, &existing_bson, &effective_enforce_locked)
```
（去重非必需——enforce_locked_fields 对重复 key 幂等覆盖同一 existing 值。）

### 组件 2：KB-09 —— AI 会话应用草稿接回 apply_chunk_revision

`apply_update_chunk`（chat.rs:1711）改造，保留既有映射与 anchors 重算，只换落库方式：
1. **保留** camelCase→snake_case 字段映射（:1721-1752）构造的 `update_doc` → 作 RevisionRequest.patch。
2. **保留** source_quote→source_anchors 重算（:1767-1774，patch 前重算），重算的 `source_anchors` 作为 patch 字段并入 update_doc。**亲验**：重算写 `source_anchors`（复数，models.rs:1571），DEFAULT 锁的是 `source_anchor`（单数）——不同名，重算不被锁拦。
3. **删除** 手写的 status/integrity_status/updated_at 插入（:1776-1778）——apply_chunk_revision 对 source=Ai 自动强制 draft+needs_review（:209-211）+ 写 updated_at（:223）。
4. **落库改为** `apply_chunk_revision(&state.db, workspace_id, oid, RevisionRequest{op:Patch, source:Ai, patch:update_doc, reason:..., actor:...})`。
5. **返回值**：从 RevisionApplied 取等价信息拼回原 JSON 形状（`{updatedChunkId, fieldsTouched, status, integrityStatus}`），前端契约不变。

**接回后天然获得**：审计行（治后果①）+ 数组 union（治后果②运营 tag 丢失）+ locked_fields 守门（组件1，治后果③）+ 保持 draft+needs_review（红线不破）。

### 组件 3：KB-10 —— admin PUT 补 revision 审计行 + 锁字段（保留 replace_one）

`update_operation_knowledge_chunk`（crud.rs:212）保留 replace_one 整条替换语义（不动 apply_chunk_integrity/coerce_d2/operation_knowledge_chunk_from_request/preserve_unmodeled 链路），在 find existing（:251）之后插入：
1. **锁字段强制**：构造出 next（:264）+ preserve_unmodeled（:265）后、replace_one 前，复用**同一** `enforce_locked_fields` 纯函数，用 `DEFAULT_LOCKED_FIELDS ∪ existing.locked_fields` 把锁定字段从 existing 覆盖回 next（运营锁定字段不被 PUT 覆盖）。**与组件1 走同一套锁字段语义，不各写一份**（避免新 dual-path drift——修复本身不能再造旁路）。
2. **审计行**：replace_one 成功后补写一条 chunk_revisions：`op=Patch, source=Human, before_hash=compute_chunk_hash(existing_bson), after_hash=compute_chunk_hash(next_bson), actor=admin.user_id`（复用 compute_chunk_hash，apply_chunk_revision 同款）。

**接受的窄窗口（诚实记录）**：审计写在 replace_one **之后**（非 apply_chunk_revision 的 revision-先-chunk-后原子序）。replace 成功但审计写失败 → "改了但无审计行"极窄窗口。取舍：admin 人工编辑频率低、replace 已成功（数据正确），审计缺一行是可观测运维问题；审计写失败 fail-soft 记 warn、不回滚 replace。为一条审计行给 admin PUT 引入事务/回滚复杂度不值（YAGNI）。

### 关键一致性

KB-10 与 apply_chunk_revision 复用**同一个 `enforce_locked_fields` 纯函数**（page_merge.rs:140），不各写一份锁字段逻辑——这正是本批要根治的"设计声称统一、实现有旁路"病根，修复本身绝不能再造一个旁路。

## 不改动的（严格限定范围）

- apply_chunk_revision 其余步骤：body 70% 截断阈值、domain_schema 校验、catalog_rebuild enqueue、provenance 标注——不动。
- `apply_field_patch` 的硬拒语义 + DEFAULT_LOCKED_FIELDS 常量本身、`union_array_fields`、`resolve_quote_anchors`、`compute_chunk_hash`——不动（只复用）。
- PUT 的 apply_chunk_integrity / coerce_integrity_against_d2_gate / operation_knowledge_chunk_from_request / preserve_unmodeled_chunk_fields 链路——不动（KB-10 只加锁字段 + 审计行）。
- 前端 locked_fields 表单禁用——不动（后端强制是补兜底，非替代前端 UX）。
- KB-02（PUT 锚点→verified，人类路径 Low）、其余 KB Low/就绪债（KB-03/05/06/07/12）——不在本批。

## 测试策略

- **KB-11（纯函数，进 baseline lib 单测）**：`enforce_locked_fields` 传 "DEFAULT ∪ 运营锁定字段" 时，锁定的内容字段（如 title）被覆盖回 existing、未锁字段正常保留；`apply_field_patch` 仍只对 DEFAULT 集硬拒（existing.locked 不进它 → 运营锁定字段的 patch 不触发整条 Err）。纯函数易单测。
- **KB-09（集成测，需 DB，`#[ignore]` CI Docker）**：apply_update_chunk 接回后——写了 chunk_revisions 行、数组字段 union（既有 tag 不丢）、status 强制 draft。复用现有 knowledge 集成测基建。
- **KB-10（集成测，`#[ignore]` CI Docker）**：admin PUT 后 chunk_revisions 多一行（op=Patch/source=Human/before≠after hash）+ 运营锁定字段未被 PUT 覆盖。
- baseline `cargo test --lib` ≥ 350 passed / 0 failed 不回退。
- no-human-takeover lint：知识编辑措辞，无禁词。

## 验证

- `cargo test --lib`（含 KB-11 纯函数单测，baseline 不回退）。
- 集成测 `cargo test --test <knowledge_edit_test> --no-run`（本地编译；执行留 CI Docker）。
- no-human-takeover lint clean。

## 交付

- 三处 src 改动：`chunk_revisions.rs`（apply_chunk_revision 组件1）、`chat.rs`（apply_update_chunk 组件2）、`crud.rs`（update_operation_knowledge_chunk 组件3）。
- 测试：KB-11 纯函数 lib 单测 + KB-09/KB-10 集成测（`#[ignore]`）。
- 独立修复 PR（基于最新 main）。台账 KB-09/10/11 标 Closed。

## Self-Review 结论

- **Spec coverage**：KB-11↔组件1、KB-09↔组件2、KB-10↔组件3；三裁定（后端强制/PUT 补审计/anchors patch 前重算）全落地。
- **一致性**：KB-10 与组件1 复用同一 enforce_locked_fields（无新 dual-path）；KB-09 保 draft 红线；三组件都不动 apply_chunk_revision 其余步骤。
- **修正记录**：初版"existing.locked_fields 静默剥离进 apply_field_patch"经亲验 apply_field_patch 是硬拒语义后，修正为"existing.locked 只进 enforce_locked_fields 静默覆盖、不进 apply_field_patch 硬拒集"——避免运营锁定字段连坐毙掉整条合法编辑。
- **YAGNI**：KB-10 不整条改 patch 语义（保 replace_one + 补审计）；审计 fail-soft 不引入事务。
