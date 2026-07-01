# wiki 三项 P0 修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 堵住 wiki 知识库两个绕过"AI 永不自动 verify"红线的口子、打通导入类型透传、让派工 fix_chunk 真产可审草稿。

**Architecture:** 后端为主的三条独立修复线。① 收紧 auto-verify + 冷启动过滤（纯函数 + 查询 filter）；② 请求体加类型字段 + 转换函数读取 + 精简抽取 prompt；③ 抽 `propose_chunk_repair_inner` 纯业务函数供 worker 复用，fix_chunk 调它生成草稿并借现有 `chunk_id→needsReviewChunkIds` 机制落审。三条互不阻塞，独立成 commit。

**Tech Stack:** Rust 2021 (Axum + MongoDB)，无 Cargo workspace。测试 = `cargo test --lib` 单测 + testcontainers 集成测试（`#[ignore]`，留 CI 跑）。

## Global Constraints

- **红线（CLAUDE.md 硬规则）**：AI 永不自动 verify 知识。所有新增/改动写路径 chunk 一律 `status=draft` + `integrity_status=needs_review`；auto-verify 不对任何 chunk_type 自动 `verified`。
- **禁用词 lint（`check-no-human-takeover`，扫 `src/agent`/`src/routes`/`src/evolution`/`frontend/src` 新增行）**：新增**代码内文案/prompt 字符串**不得含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。**陷阱：禁词集含单字"人工"**——文案统一用「运营」（"请运营在 chunk 编辑器审核""待运营审核"），绝不出现"人工"二字。
- **测试基线不回归**：`cargo test --lib` ≥350/0；4 PBT 文件（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33/0。新增测试只 append，不改旧维度。
- **向后兼容**：新增请求体字段用 `#[serde(default)]`。
- **本机无 Docker**：只跑 `cargo test --lib` 和单 PBT 文件；`#[ignore]` 集成测试留 CI（`cargo test --test <name> -- --ignored`）。磁盘满先删 `target/debug/incremental`。
- **import.rs 的 prompt 是内联字符串**（`import.rs:61/688`，非 seed pack），改它不需 bump PROMPT_PACK_VERSION（已亲验）。

## 文件结构（改动映射）

| 文件 | 责任 | 本计划改动 |
| --- | --- | --- |
| `src/routes/knowledge/verify.rs` | auto-verify Agent | Task 1：`enforce_*` 扩到拦所有类型 |
| `src/cold_contact_worker.rs` | 冷启动重激活 | Task 2：peer_case 查询加 verified 过滤 |
| `src/routes/knowledge/mod.rs` | 请求体 + 转换函数 | Task 3：加 wiki_type/chunk_type 字段 + 读取 |
| `src/knowledge_wiki/page_merge.rs` | 锁定字段 | Task 4：chunk_type 加入 DEFAULT_LOCKED_FIELDS |
| `src/routes/knowledge/import.rs` | 抽取 prompt | Task 5：prompt 加类型输出 + 删死字段 |
| `src/routes/knowledge/repair.rs` | AI 修复 | Task 6：抽 `propose_chunk_repair_inner` |
| `src/knowledge_task/mod.rs` | 派工 worker | Task 7：fix_chunk 调 inner 产草稿 |

---

### Task 1: ①-a auto-verify 对所有 chunk_type 都不自动 verified

**Files:**
- Modify: `src/routes/knowledge/verify.rs:395`（调用点）、`:553-558`（函数定义）、`:564-578`（现有单测）

**Interfaces:**
- Produces: `enforce_verified_needs_human_audit(final_status: String) -> String`（重命名 + 删 chunk_type 参数）。语义：`final_status=="verified"` → `"needs_human_audit"`；其余原样返回。

**背景**：`enforce_product_claim_human_audit`（verify.rs:553）现在只拦 `product_fact`。改为拦所有类型——auto-verify 对任何 chunk_type 都不自动 verified（过闸的判 needs_human_audit）。**这会改变旧测试 `non_product_fact_verified_kept`（:573-578）的断言**（它断言非产品类保留 verified，正是要修的行为），必须更新该测试而非新增——属"行为定义变更"，不违反增量叠加铁律。

- [ ] **Step 1: 改单测断言（TDD 红）**

在 `verify.rs` `mod tests`（:560）里：
- 把 `non_product_fact_verified_kept`（:571-578）**改写**为断言"所有类型 verified 都降级"：
```rust
    /// ①-a：auto-verify 对**所有** chunk_type 的 verified 都强制降级 needs_human_audit
    /// （AI 永不自动 verify 适用所有类型，不只 product_fact）。
    #[test]
    fn all_types_verified_forced_to_human_audit() {
        for ct in ["product_fact", "style_template", "peer_case", "negative_example"] {
            let _ = ct; // 类型不再影响判定；保留循环表达"覆盖全类型"意图
            let s = enforce_verified_needs_human_audit("verified".to_string());
            assert_eq!(s, "needs_human_audit", "所有类型的 verified 都必须降级");
        }
    }
```
- 把 `product_fact_verified_forced_to_human_audit`（:565-569）的调用改为新签名：`enforce_verified_needs_human_audit("verified".to_string())`。
- 把 `product_fact_non_verified_passthrough`（:582-588）改为 `enforce_verified_needs_human_audit(st.to_string())`（删 chunk_type 参数）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib enforce_verified 2>&1 | tail -15`
Expected: 编译失败（`enforce_verified_needs_human_audit` 未定义）。

- [ ] **Step 3: 改函数定义 + 调用点（TDD 绿）**

`verify.rs:553-558` 替换为：
```rust
/// ①-a：auto-verify 的最终状态若为 `verified`，强制降级 `needs_human_audit`——
/// **对所有 chunk_type 生效**。依据：CLAUDE.md 红线「AI 永不自动 verify」适用于
/// 所有类型知识；auto-verify 仅凭 LLM 自评 + 证据闸不足以替代人工核验。auto-verify
/// 退化为"预审分诊"：过闸的挑出来等运营重点看，绝不自动放行。
///
/// 性质：仅当 `final_status == "verified"` 时降级；其它（rejected / needs_review /
/// needs_human_audit）一律原样返回。
pub fn enforce_verified_needs_human_audit(final_status: String) -> String {
    if final_status == "verified" {
        return "needs_human_audit".to_string();
    }
    final_status
}
```
`verify.rs:395` 调用点替换：
```rust
        final_status = enforce_verified_needs_human_audit(final_status);
```
（删去原 `&chunk.chunk_type` 实参；注意 :395 上方 :389-394 的注释也要更新为"对所有类型强制人审"。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib -p wechatagent verify 2>&1 | tail -15`（或 `cargo test --lib enforce_verified` + `cargo test --lib decide_auto_verify`）
Expected: `all_types_verified_forced_to_human_audit`、`product_fact_verified_forced_to_human_audit`、`product_fact_non_verified_passthrough`、`decide_auto_verify_*` 全 PASS。

- [ ] **Step 5: 全量 lib + lint**

Run: `cargo test --lib 2>&1 | tail -5`（≥350/0）；`bash scripts/check-no-human-takeover.sh HEAD~1 HEAD 2>&1 | tail -3`（0 violations——注意新注释无"人工"字面？"人工核验"含"人工"！**改注释用"运营核验/人工把关"→ 一律用"运营"**）。
Expected: lib ≥350/0；lint 0 violations。

> ⚠️ 上面 Step 3 的函数文档注释我写了"人工核验/人工把关"——**这些含"人工"是禁词**。实际落地时注释改为："auto-verify 仅凭 LLM 自评不足以替代**运营核验**"、"过闸的挑出来等**运营**重点看"。提交前必跑 lint 自检。

- [ ] **Step 6: Commit**

```bash
git add src/routes/knowledge/verify.rs
git commit -m "fix(knowledge): ①-a auto-verify 对所有类型都不自动 verified,退为预审分诊"
```

---

### Task 2: ①-b 冷启动 peer_case 推送加 integrity_status=verified 过滤

**Files:**
- Modify: `src/cold_contact_worker.rs:328-332`（`load_peer_case_hooks` 的 filter）
- Test: `tests/cold_reactivation_*.rs`（若已有）或新增 `#[ignore]` 集成测试

**Interfaces:**
- Consumes: 无。
- Produces: `load_peer_case_hooks` 行为变更——只返回 `integrity_status="verified"` 的 peer_case summary。

**背景**：`load_peer_case_hooks`（cold_contact_worker.rs:322-345）取 peer_case summary 作冷启动推送文案，filter 只看 `status`，**不看 integrity_status**（§1.1 ①-b 松动点）。加 verified 过滤，让对客推送内容也守 integrity 门。

- [ ] **Step 1: 改 filter**

`src/cold_contact_worker.rs` `load_peer_case_hooks` 的 `.find(doc! {...})`（当前 :328-332）改为：
```rust
        .find(
            doc! {
                "workspace_id": workspace_id,
                "chunk_type": "peer_case",
                "status": { "$in": ["active", "approved"] },
                "integrity_status": "verified",
            },
            None,
        )
```

- [ ] **Step 2: 更新函数文档注释**

`load_peer_case_hooks` 上方注释（:317-321）补一句：
```rust
/// ①-b 红线：只取 integrity_status="verified" 的 peer_case——对客推送文案必须内容
/// 已核实（active/approved 是启用门，verified 是内容门，双门都过才可推送给客户）。
```
（注释无"人工"字面，安全。）

- [ ] **Step 3: 编译确认**

Run: `cargo check --tests 2>&1 | tail -5`
Expected: EXIT 0，无 warning。

- [ ] **Step 4: 集成测试（留 CI 跑，本机无 Docker）**

若 `tests/` 下已有冷启动相关集成测试文件（grep `load_peer_case_hooks` / `cold_reactivation` 确认），append 一个 `#[ignore]` 测试：seed 三个 peer_case（都 status=active，integrity 分别 verified / needs_review / rejected）→ 调 `load_peer_case_hooks`（若为 `pub(crate)` 可直调；否则测 filter 等价查询）→ 断言只返回 verified 那条的 summary。若无现成文件，本 Task 不新建独立测试文件（避免 target 膨胀），改为在 Task 1/其它已有集成文件里 append，或纯靠 Step 3 编译 + code review 核 filter 正确性 + CI 全量 --ignored 覆盖。

Run（CI 或有 Docker 时）: `cargo test --test <file> -- --ignored 2>&1 | tail -10`
Expected: 新增测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/cold_contact_worker.rs
git commit -m "fix(cold-reactivation): ①-b peer_case 冷启动推送加 integrity=verified 过滤"
```

---

### Task 3: ②-a 请求体加 wiki_type/chunk_type 字段 + 转换函数读取

**Files:**
- Modify: `src/routes/knowledge/mod.rs:169-212`（`OperationKnowledgeChunkRequest`）、`:463-509`（`operation_knowledge_chunk_from_request`）
- Test: `src/routes/knowledge/mod.rs` 的 `#[cfg(test)] mod tests`（若无则新建）或 append 到现有 mod.rs 测试

**Interfaces:**
- Produces: 请求体新增 `wiki_type: Option<String>` + `chunk_type: Option<String>`（均 `#[serde(default)]`）；转换函数读取它们落库。

**背景**：请求体（mod.rs:169-212）无 wiki_type/chunk_type 字段，导入 LLM 吐的类型被 serde 静默丢弃（§1.4）。加字段 + 转换函数读取，打通类型透传。用 `Option` + `#[serde(default)]` 保向后兼容。

- [ ] **Step 1: 写 round-trip 单测（TDD 红）**

在 `src/routes/knowledge/mod.rs` 找到（或新建）`#[cfg(test)] mod tests`，append：
```rust
    #[test]
    fn chunk_request_carries_wiki_type_and_chunk_type() {
        // 带类型的 JSON → 请求体 → chunk：类型落值
        let json = serde_json::json!({
            "domain": "user_operations",
            "title": "T",
            "body": "B",
            "wikiType": "methodology",
            "chunkType": "peer_case",
        });
        let req: OperationKnowledgeChunkRequest =
            serde_json::from_value(json).expect("deserialize");
        let state = /* 见下：转换函数只用 _state，可传占位；若无现成 AppState 构造，
                       改测 req.wiki_type/req.chunk_type 字段本身 */ ();
        let _ = state;
        assert_eq!(req.wiki_type.as_deref(), Some("methodology"));
        assert_eq!(req.chunk_type.as_deref(), Some("peer_case"));
    }

    #[test]
    fn chunk_request_defaults_type_fields_when_absent() {
        // 老 JSON 不带类型 → None（向后兼容）
        let json = serde_json::json!({ "domain": "user_operations", "title": "T", "body": "B" });
        let req: OperationKnowledgeChunkRequest =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(req.wiki_type, None);
        assert_eq!(req.chunk_type, None);
    }
```
> 注：`operation_knowledge_chunk_from_request` 第一参 `_state: &AppState` 实际未使用（下划线前缀）。若测试里难构造 `AppState`，就只断言请求体字段（如上），转换函数的落值行为靠 Task 3 的 `cargo check` + 现有集成测试（chunk_batch_ops.rs 等已 seed 类型）覆盖。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib chunk_request_carries 2>&1 | tail -12`
Expected: 编译失败（`OperationKnowledgeChunkRequest` 无 wiki_type/chunk_type 字段）。

- [ ] **Step 3: 加请求体字段**

`src/routes/knowledge/mod.rs` 的 `OperationKnowledgeChunkRequest`（:211 `priority` 字段后、结构体闭合 `}` 前）加：
```rust
    #[serde(default)]
    wiki_type: Option<String>,
    #[serde(default)]
    chunk_type: Option<String>,
```

- [ ] **Step 4: 转换函数读取**

`operation_knowledge_chunk_from_request`（mod.rs:470-509）里，把末尾的 `..Default::default()`（:508）**上方**插入两字段赋值（放在 `priority: payload.priority,` 之后、`created_at` 之前，或任意 struct 字段位）：
```rust
        wiki_type: payload.wiki_type.filter(|s| !s.trim().is_empty()),
        chunk_type: payload
            .chunk_type
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(default_chunk_type),
```
（`default_chunk_type` 已在 `crate::models` / 本文件可见；确认 import 路径。`..Default::default()` 保留——它现在只兜底 wiki_type/chunk_type 之外的字段如 provenance/domain_attributes 等。**注意**：显式写了 chunk_type 后，`..Default::default()` 不能再覆盖它——Rust struct update 语法里显式字段优先，安全。）

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --lib chunk_request 2>&1 | tail -12`
Expected: 两个新测试 PASS。

- [ ] **Step 6: 全量 lib 确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥350/0（现有 preserve/PUT 测试如 `chunk_put_preserves_unmodeled_fields` 不回归——它测 PUT 时 wiki_type/chunk_type 从 existing 保留，与本 Task 加请求体字段不冲突，因为 preserve 在转换后覆盖）。

- [ ] **Step 7: Commit**

```bash
git add src/routes/knowledge/mod.rs
git commit -m "feat(knowledge): ②-a 请求体加 wiki_type/chunk_type 字段,转换函数读取(导入类型透传)"
```

---

### Task 4: ②-b chunk_type 加入 DEFAULT_LOCKED_FIELDS（消除不对称）

**Files:**
- Modify: `src/knowledge_wiki/page_merge.rs:30-43`（`DEFAULT_LOCKED_FIELDS` + 上方注释）
- 同步注释：`tests/wiki_chunk_revision_pbt.rs:5,50`（文件头 + PBT 注释里"7 个"→"8 个"）

**Interfaces:**
- Consumes: 无。
- Produces: `DEFAULT_LOCKED_FIELDS` 从 7 项变 8 项（加 `chunk_type`）。

**背景**：`wiki_type` 在 locked_fields（创建后 patch 不可改），`chunk_type` 只在 preserve、不在 locked（§1.4 不对称）。加入 chunk_type 对齐——类型创建后一致锁定。

**牵连点（已亲验，必须全部同步）**：
- `wiki_chunk_revision_pbt.rs:52` `locked_field_rejection` PBT（**基线 4 PBT 之一**）遍历 `DEFAULT_LOCKED_FIELDS`——加 chunk_type 后**自动覆盖新字段**且行为一致（chunk_type 作 patch key 被拒），PBT 会绿，但注释 :50"7 个"、文件头 :5 字段列表要同步。
- `page_merge_pbt.rs:125` 同样遍历——自动覆盖，无需改逻辑。
- create/import 路径走 insert **不经** `apply_field_patch`，不受 locked 影响 → Task 3 的导入设 chunk_type 仍有效。
- 对话 update（`chat.rs` `apply_update_chunk`）走裸 `$set` **不经** apply_chunk_revision → 也不受 locked 影响（这是既有的另一条绕过路径，本 Task 不碰）。

- [ ] **Step 1: 加字段到 const**

`src/knowledge_wiki/page_merge.rs:35-43` 的 `DEFAULT_LOCKED_FIELDS` 数组加 `"chunk_type"`（放 `"wiki_type"` 之后，语义相邻）：
```rust
pub const DEFAULT_LOCKED_FIELDS: &[&str] = &[
    "chunk_id",
    "wiki_type",
    "chunk_type",
    "created_at",
    "source_anchor",
    "verified_at",
    "verified_by",
    "approved_at",
];
```
同步 :30-34 的注释：把"`chunk_id` / `wiki_type` / `created_at`：身份/类型/创建时间永不变"改为"`chunk_id` / `wiki_type` / `chunk_type` / `created_at`：身份/类型/创建时间永不变"。

- [ ] **Step 2: 同步 PBT 注释**

`tests/wiki_chunk_revision_pbt.rs`：
- 文件头 :5 的字段列表 `chunk_id/wiki_type/created_at/source_anchor/...` 加 `chunk_type`。
- :50 注释"patch 含任意 **7** 个锁定字段之一"→"**8** 个"。

- [ ] **Step 3: 跑基线 PBT 确认不回归**

Run: `cargo test --test wiki_chunk_revision_pbt 2>&1 | tail -10`
Expected: 全 PASS（locked_field_rejection 现遍历 8 项，chunk_type 也被拒——自动覆盖）。

- [ ] **Step 4: page_merge 单测**

Run: `cargo test --lib page_merge 2>&1 | tail -8` + `cargo test --test page_merge_pbt 2>&1 | tail -8`
Expected: 全 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/knowledge_wiki/page_merge.rs tests/wiki_chunk_revision_pbt.rs
git commit -m "fix(knowledge): ②-b chunk_type 加入 DEFAULT_LOCKED_FIELDS,类型创建后一致锁定"
```

---

### Task 5: ②-c 抽取 prompt 加类型输出 + 删已删死字段指令

**Files:**
- Modify: `src/routes/knowledge/import.rs`
  - 长文本 chunks JSON 模板（:113-133）：加 wikiType/chunkType，删 routingCard/safeClaims/forbiddenClaims/evidenceItems
  - 长文本 items JSON 模板（:96-104）：删 suitableFor/notSuitableFor/customerStages/operationStates/intentLevels/safeClaims/forbiddenClaims/commonQuestions/commonObjections/evidenceItems/routingCard（items→chunk 请求体全丢，且喂 preview 显示——先 grep 前端核查空显示无碍）
  - 长文本 requirement bullets（:141-144）：删死字段名，保留"忠于原文/不编造"护栏
  - 图片 vision system_prompt（:688-694）：加 wikiType/chunkType 输出要求（护栏 :693 已有，不重复加）
- Test: `src/routes/knowledge/import.rs` `#[cfg(test)] mod tests` append 字符串锁定测试

**Interfaces:**
- Consumes: Task 3 的请求体 wiki_type/chunk_type 字段（长文本 chunks / 图片 fence 的 JSON 经 `from_value::<OperationKnowledgeChunkRequest>` 落库时接住类型）。
- Produces: 无导出符号变化，纯 prompt 字符串调整。

**背景（已亲验）**：
- 长文本导入 prompt 是**内联字符串**（import.rs:61 system / :66-152 user），**非 seed pack** → 不需 bump PROMPT_PACK_VERSION。
- `safe_claims/forbidden_claims/evidence_items` 在 `models.rs` 仅测试出现（:5778-5780），`routing_card` grep 零命中 → 是 2026-05-25 已删死字段（§1.5）。prompt 让 LLM 产它们纯浪费 token + 误导。
- items 与 chunks 落库**都走 `OperationKnowledgeChunkRequest`**（import.rs:336 items 分支 / :848 fence 分支）→ 请求体没有的字段 apply 时静默丢弃。
- items JSON 的字段还喂 preview 显示（`normalize_operation_knowledge_preview_item` mod.rs:550-598，每字段 `.unwrap_or_default()`）→ 删 prompt 字段后 LLM 不产 → normalize 取空默认 → 预览空显示（不崩）。删前 grep 前端确认无硬依赖。

- [ ] **Step 1: grep 前端 preview 对死字段的消费（核查非破坏）**

Run: `grep -rn "safeClaims\|forbiddenClaims\|evidenceItems\|suitableFor\|customerStages\|routingCard\|commonQuestions\|commonObjections\|operationStates\|intentLevels\|notSuitableFor" frontend/src --include=*.tsx --include=*.ts | grep -iv test`
Expected: 记录命中。这些字段若前端有渲染，删 prompt 后会显示空（normalize `.unwrap_or_default()` 兜底，不报错）——因它们 apply 时本就丢弃、属预审死字段，空显示比误导显示更诚实。**若发现前端把某字段当必填/会因空崩溃**，STOP 报告用户再决定（预期不会——preview 字段都是可选展示）。

- [ ] **Step 2: 写字符串锁定测试（TDD 红）**

`src/routes/knowledge/import.rs` `#[cfg(test)] mod tests` append（若无 mod tests 则在文件末尾新建）。**关键**：测试要能拿到 prompt 文本——`import_operation_knowledge_preview` 里 user prompt 是函数内 `format!` 局部变量，无法直接取。故把长文本 user prompt 的**静态模板部分**抽成模块级 `const LONG_IMPORT_PROMPT_TEMPLATE: &str`（含 `{}` 占位），`format!` 引用它，测试对该 const 断言：
```rust
    #[test]
    fn long_import_prompt_carries_types_and_drops_dead_fields() {
        // ②-c：prompt 让 LLM 产 wikiType/chunkType（类型透传的源头）
        assert!(LONG_IMPORT_PROMPT_TEMPLATE.contains("wikiType"), "chunks 模板须含 wikiType");
        assert!(LONG_IMPORT_PROMPT_TEMPLATE.contains("chunkType"), "chunks 模板须含 chunkType");
        // 已删死字段不得再出现在 prompt（防未来回退）
        for dead in ["safeClaims", "forbiddenClaims", "evidenceItems", "routingCard"] {
            assert!(
                !LONG_IMPORT_PROMPT_TEMPLATE.contains(dead),
                "已删字段 {dead} 不应再出现在抽取 prompt"
            );
        }
    }
```
> 若抽 const 改动过大（`format!` 的多个 `{}` 位置参数需保持顺序一致），退化方案：测试改为对 `import.rs` **文件内容**做字符串断言（`include_str!("import.rs")` 在测试里读本文件源码），断言含 wikiType/不含 safeClaims。二选一，抽 const 更干净、优先。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib long_import_prompt_carries 2>&1 | tail -12`
Expected: 失败（当前 prompt 无 wikiType、含 safeClaims）。

- [ ] **Step 4: 改 chunks JSON 模板（加类型 + 删死字段）**

`import.rs` chunks 模板（:113-133），把 `"domain": "user_operations",` 之后**紧接**插入 wikiType/chunkType（放醒目决策位——memory A/B 铁证：结构化字段指令位置决定 LLM 是否认真填），并**删除** `routingCard`/`safeClaims`/`forbiddenClaims`/`evidenceItems` 四行：
```
  "chunks": [
    {{
      "domain": "user_operations",
      "wikiType": "9 类之一：source/entity/concept/comparison/synthesis/methodology/finding/query/thesis。按知识形态选：有步骤/分支的方法→methodology；具体数据点/案例事实→finding；纯定义→concept；多源综述→synthesis；带论据的判断/主张→thesis；FAQ→query；单一实体→entity；原始出处→source；对比→comparison",
      "chunkType": "4 类之一：product_fact（可对客户承诺的产品事实，需核验背书）/ style_template（语气模板）/ peer_case（同行案例参考，不作产品承诺）/ negative_example（不该做的反例）。绝大多数产品/服务事实类知识填 product_fact",
      "knowledgeType": "AI 自主生成的切片类型",
      "businessContext": "业务上下文",
      "title": "",
      "summary": "",
      "body": "可被 Agent 按需打开的原文要点或经过整理的知识正文",
      "applicableScenes": [],
      "notApplicableScenes": [],
      "productTags": ["如：WechatAgent / AI 私域销售助手；最多 5 个；可空"],
      "businessTopics": ["如：产品定位差异 / 竞品对比；最多 3 个；可空"],
      "sourceQuote": "如有必要，保留支撑该切片的原文短句",
      "status": "draft",
      "priority": 0
    }}
  ]
```
> chunkType 描述用「核验背书」而非「人工背书」——**禁词"人工"陷阱**，Step 8 lint 自检。

- [ ] **Step 5: 改 items JSON 模板（删死字段）**

`import.rs` items 模板（:82-111），删除这些**apply 时丢弃 + 属已删语义**的行：`routingCard`（:92）、`suitableFor`/`notSuitableFor`（:95-96）、`customerStages`/`operationStates`/`intentLevels`（:97-99）、`safeClaims`/`forbiddenClaims`（:100-101）、`commonQuestions`/`commonObjections`（:102-103）、`evidenceItems`（:104）。**保留**：domain/category/businessType/knowledgeType/businessContext/title/summary/body/applicableScenes/notApplicableScenes/productTags/businessTopics/sourceType/sourceName/status/priority（这些 preview 显示要用或 chunk 请求体接得住）。
> category/businessType 虽非 chunk 字段，但 normalize_preview_item 用作显示分类（mod.rs:563-570），保留。

- [ ] **Step 6: 改 requirement bullets（删死字段名，留护栏）**

`import.rs` :141-144 四条 bullet：
- :141 `只忠于原文：body、summary、safeClaims、evidenceItems 只能包含原文已陈述的内容，**禁止补充...推断**。拿不准是否在原文里，就不写。` → 改为 `只忠于原文：body、summary 只能包含原文已陈述的内容，**禁止补充原文没有的描述、范围、功能、优惠条件或推断**。拿不准是否在原文里，就不写。`（保留"忠于原文"护栏，删 safeClaims/evidenceItems 字段名）
- :142 `- safeClaims 必须是有依据、可安全对客户表达的事实。` → **整行删除**
- :143 `- forbiddenClaims 必须列出不能承诺、不能暗示、不能编造的内容。` → **整行删除**
- :144 `- 案例、报价、效果数据必须进入 evidenceItems；没有证据不要编造成案例。` → 改为 `- 案例、报价、效果数据必须完整落入对应 chunk 的 body；没有证据不要编造成案例。`（保留"不编造案例"护栏，落点从死字段 evidenceItems 改为 body）

- [ ] **Step 7: 图片 vision prompt 加类型输出**

`import.rs:688-694` system_prompt。当前 fence 块要求"至少含 title 字段，且 body/summary/answer 至少一个非空"。在该句后补一句类型输出要求（拼进现有字符串，注意 `\n\` 续行风格）：
```
块体 JSON 可选带 "wikiType"（9 类知识形态之一：source/entity/concept/comparison/synthesis/methodology/finding/query/thesis）与 "chunkType"（4 类运营用途之一：product_fact/style_template/peer_case/negative_example，产品事实类填 product_fact）；拿不准可省略，省略时系统按默认处理。
```
> 图片 fence 走 `ingest_chunked_text`→`OperationKnowledgeChunkRequest`（:848），Task 3 加字段后能接住 fence JSON 里的 wikiType/chunkType。护栏 :693"绝不编造/看不清标注为不确定"已存在，不重复加。

- [ ] **Step 8: 跑测试 + lint + 全量 lib**

Run:
- `cargo test --lib long_import_prompt_carries 2>&1 | tail -12` → PASS
- `cargo test --lib 2>&1 | tail -5` → ≥350/0
- `bash scripts/check-no-human-takeover.sh HEAD~1 HEAD 2>&1 | tail -3` → 0 violations（**重点核 chunkType 描述无"人工"二字**——用了"核验背书"）

- [ ] **Step 9: Commit**

```bash
git add src/routes/knowledge/import.rs
git commit -m "feat(knowledge): ②-c 抽取 prompt 加 wiki_type/chunk_type 输出,删已删死字段指令"
```

---

### Task 6: ③-a 抽 `propose_chunk_repair_inner` 纯业务函数

**Files:**
- Modify: `src/routes/knowledge/repair.rs:201-378`（`propose_chunk_repair` handler 抽 inner）

**Interfaces:**
- Produces:
  ```rust
  pub(crate) async fn propose_chunk_repair_inner(
      state: &AppState,
      workspace_id: &str,
      chunk_object_id: ObjectId,
      run_id: &str,
  ) -> AppResult<Value>
  ```
  返回**已 parse 的 repair JSON**（`parse_repair_response` 的产物：`{ interpretation, patch, missingFields, followupQuestions, stillMissing, confidenceHint }`）。**不含** chunkId/sessionId/turn/promptKey/budget 外层信封——那些由 handler 拼。
  - **不自建 `RUN_BUDGET.scope`**：假定调用方已在某个 scope 内（handler 建自己的 4000-token scope；worker 复用 STEP scope）。inner 内不 new budget、不 scope。
  - **红线**：inner 只 load chunk+doc → generate_agent_json → parse → 写 usage_log + repair_event → 返回 JSON。**绝不改 chunk 本身**（与现有 handler 完全一致）。

**背景（已亲验）**：
- handler（repair.rs:201-378）用 `admin.current_workspace` 5 处（:213/:229/:247/:334/:349）→ inner 全部换成 `workspace_id: &str` 参数。
- `account_id` 在 :240-243 从 `chunk.account_id` 派生（`unwrap_or state.config.default_account_id`）→ inner 内部自己派生，不需入参。
- handler 自建 run_id/session_id/budget + `RUN_BUDGET.scope`（:292-314）→ **留在 handler**，inner 只收 `run_id: &str`。
- `parse_repair_response`（:77）、`write_repair_usage_log`（:156，已参数化 workspace_id/account_id）、`record_repair_event`、`budget_document`（mod.rs:214）都可复用。
- `generate_agent_json` 本身会读 ambient `RUN_BUDGET`（task_local）累计 token——inner 不 scope 也能正确计入调用方的 scope（`current_run_budget()` budget.rs:182 是 ambient 读取）。

- [ ] **Step 1: 抽 inner 函数（重构，行为不变）**

在 `repair.rs` `propose_chunk_repair` 之前新增 inner。把 handler :206-363 的核心（load chunk → load document → load prompt → build user → generate_agent_json → parse → 写 log/event）搬进 inner，签名如上。要点：
- `let chunk = ...find_one(doc!{"_id": chunk_object_id, "workspace_id": workspace_id})...` —— workspace 用入参，id 用入参 ObjectId。
- document 查询同样用 `workspace_id` 入参。
- `let id_str = chunk_object_id.to_hex();` —— 原 handler 的 `id: String` 来自 path，inner 内从 ObjectId 生成 hex 供 prompt/log 的 targetId 用（原 :293 `run_id` 也含 id，但 run_id 现由入参传入，不在 inner 生成）。
- **不建 budget、不 scope**：直接 `agent::generate_agent_json(&state, Some(&account_id), None, Some(run_id), "knowledge.chunk.repair.propose", &system, &user).await?`（去掉 :301-314 的 `RUN_BUDGET.scope` 包裹）。
- parse → 取 confidence/missing/followup → `write_repair_usage_log(&state, workspace_id, &account_id, run_id, chunk.id, "chunk_repair_session", "knowledge.chunk.repair.propose", &id_str, 1, confidence, &missing, followup.len()).await;`
- `record_repair_event(&state, workspace_id, &account_id, "knowledge_repair_proposed", format!("AI 自主修复 chunk:{id_str} 第 1 轮"), doc!{...}).await;` —— 其中 `budget` 字段：inner 内 `let budget_doc = agent::current_run_budget().map(|b| budget_document(&b)).unwrap_or_default();`（ambient budget；不在 scope 内则空 doc，fail-soft）。
- inner 返回 `Ok(parsed)`（`parse_repair_response` 的完整产物 Value）。

- [ ] **Step 2: handler 改为薄壳调 inner**

`propose_chunk_repair`（:201-378）body 替换为：
```rust
pub async fn propose_chunk_repair(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let run_id = format!("repair-chunk-{}-{}", id, session_id);
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        REPAIR_TOKEN_BUDGET_PER_TURN,
        REPAIR_MAX_LLM_CALLS_PER_TURN,
        i32::MAX,
    ));
    let parsed = agent::RUN_BUDGET
        .scope(budget.clone(), async {
            propose_chunk_repair_inner(&state, &admin.current_workspace, object_id, &run_id).await
        })
        .await?;
    Ok(Json(json!({
        "chunkId": id,
        "sessionId": session_id,
        "turn": 1,
        "promptKey": "knowledge.chunk.repair.propose",
        "interpretation": parsed.get("interpretation"),
        "patch": parsed.get("patch"),
        "missingFields": parsed.get("missingFields"),
        "followupQuestions": parsed.get("followupQuestions"),
        "stillMissing": parsed.get("stillMissing"),
        "confidenceHint": parsed.get("confidenceHint"),
        "budget": budget_document(&budget),
    })))
}
```
> handler 建 scope + budget（对客 4000 token 门不变），inner 在 scope 内跑，`generate_agent_json` 的 token 正确计入这个 budget（`budget_document(&budget)` 拿到真实用量）。前端 ChunkRepairPanel 消费的 JSON 信封（chunkId/sessionId/turn/promptKey/interpretation/patch/missingFields/followupQuestions/stillMissing/confidenceHint/budget）**逐字段不变**——零破坏（已亲验 frontend/src/features/knowledge/ChunkRepairPanel.tsx + trustTypes.ts:196-202 消费这些键）。

- [ ] **Step 3: 编译确认（重构无行为变化）**

Run: `cargo check --tests 2>&1 | tail -8`
Expected: EXIT 0，无 warning（inner 被 handler 调用，无 dead-code；`current_run_budget`/`budget_document` 已 pub(crate)/可见）。

- [ ] **Step 4: 现有 repair 测试不回归**

Run: `cargo test --lib repair 2>&1 | tail -10`（`parse_repair_response` 等纯函数单测）
Expected: 全 PASS（inner 未动 parse 逻辑）。

- [ ] **Step 5: 全量 lib + lint**

Run: `cargo test --lib 2>&1 | tail -5`（≥350/0）；`bash scripts/check-no-human-takeover.sh HEAD~1 HEAD 2>&1 | tail -3`（0 violations——inner/handler 新增行无"人工"字面，原 prompt 文案 :283 含"运营确认"已合规）。
Expected: lib ≥350/0；lint 0。

- [ ] **Step 6: Commit**

```bash
git add src/routes/knowledge/repair.rs
git commit -m "refactor(knowledge): ③-a 抽 propose_chunk_repair_inner 纯业务函数供 worker 复用"
```

---

### Task 7: ③-b execute_step 的 fix_chunk 调 inner 产可审草稿

**Files:**
- Modify: `src/knowledge_task/mod.rs:431-434`（`StepOutcome` 加 details 字段）、`:270-283`（外层 merge details）、`:445-457`（fix_chunk 分支）
- Test: `tests/` 集成测试（`#[ignore]`，留 CI）；本机跑 `cargo check --tests` + code review

**Interfaces:**
- Consumes: Task 6 的 `propose_chunk_repair_inner(state, workspace_id, chunk_object_id, run_id)`。
- Produces: `StepOutcome` 新增 `details: Option<Document>` 字段。

**背景（已亲验）**：
- `execute_step`（mod.rs:437-486）每 step 跑在外层 `RUN_BUDGET.scope(STEP budget)` 里（:258-262）→ fix_chunk 调 inner 时，inner 的 `generate_agent_json` token 正确计入 STEP budget（8000/4 calls，:30-31），超额 fail-soft（现有 :285-297 Err 分支兜底）。
- `StepOutcome { chunk_id, message }`（:431-434）**无 details 字段** → patch 草稿无处放。progress turn 的 details 由外层硬编码（:316-321，只 taskId/phase/stepIndex/total）。**必须给 StepOutcome 加 `details: Option<Document>`**，外层把它 merge 进 progress turn details。
- `chunk_id` Some 时外层自动 `needs_review.push`（:272-274）→ 收尾 summary 的 `needsReviewChunkIds`（:384）现成落审机制，fix_chunk 只要返回 chunk_id 即入池。
- 红线（模块头 :12-15）：严禁写 verified，任何 apply 强制 needs_review。worker **不改 chunk**——只把 inner 产的 patch 草稿呈现给运营。

- [ ] **Step 1: 给 StepOutcome 加 details 字段**

`mod.rs:431-434`：
```rust
struct StepOutcome {
    chunk_id: Option<String>,
    message: String,
    /// ③-b：fix_chunk 产的 AI 修复草稿（patch/missingFields/confidenceHint）。
    /// merge 进本 step 的 progress turn details，供运营在 chunk 编辑器审核。
    /// 其它 action 为 None。
    details: Option<Document>,
}
```

- [ ] **Step 2: 现有 5 处 StepOutcome 构造补 `details: None`**

`execute_step`（:444-483）里现有 6 个 action 分支返回 5 个 `StepOutcome{...}`（fix_chunk/add_chunk/retag/review_evolution/analyze_logs/dismiss）。除 fix_chunk（Step 4 改）外，其余每个 `StepOutcome{ chunk_id, message }` 补 `details: None`：
- `add_chunk`（:460-463）、`retag`（:465-471）、`review_evolution`（:472-475）、`analyze_logs`（:476-479）、`dismiss`（:480-483）各加 `details: None,`。

- [ ] **Step 3: 外层把 details merge 进 progress turn（TDD 前先改承载）**

`mod.rs` 外层 Ok 分支（:270-283）。当前 details 在 :316-321 构造。改为在 Ok 分支拿到 outcome.details 后 merge。把 `match outcome { Ok(StepOutcome { chunk_id, message }) => {...} }`（:271）解构加 details：
```rust
            Ok(StepOutcome { chunk_id, message, details }) => {
                if let Some(cid) = chunk_id.as_deref() {
                    entry.insert("chunkId", cid);
                    needs_review.push(cid.to_string());
                }
                entry.insert("status", "ok");
                if let Some(d) = details.as_ref() {
                    entry.insert("repairDraft", d.clone());
                }
                progress_msg = format!(
                    "第 {}/{} 步完成 · {} · {}",
                    idx + 1, total, action,
                    if message.is_empty() { summary_text.clone() } else { message }
                );
                step_details = details; // 供下方 write_progress_turn 使用
            }
```
并在 match 前声明 `let mut step_details: Option<Document> = None;`（Err 分支保持 None），把 :316-321 的 details doc 改为按需插入 repairDraft：
```rust
            doc! {
                "taskId": task_id,
                "phase": "step",
                "stepIndex": idx as i32 + 1,
                "total": total as i32,
            }
```
→ 改为构造后 `if let Some(d) = step_details { turn_details.insert("repairDraft", d); }`（turn_details 为可变 doc）。
> 这样 patch 草稿同时进 `completed_steps[].repairDraft`（持久化到 task 文档）和 progress turn 的 details（SSE 推前端）。

- [ ] **Step 4: fix_chunk 分支调 inner**

`mod.rs:445-457` fix_chunk 分支改为：
```rust
        "fix_chunk" => {
            let chunk_id = step.get_str("targetChunkId").ok().map(|s| s.to_string());
            let Some(cid) = chunk_id.clone() else {
                // 无 targetChunkId：退回文案，不阻断（fail-soft）。
                return Ok(StepOutcome {
                    chunk_id: None,
                    message: "缺 targetChunkId，未生成修复草稿".to_string(),
                    details: None,
                });
            };
            let Ok(object_id) = mongodb::bson::oid::ObjectId::parse_str(&cid) else {
                return Ok(StepOutcome {
                    chunk_id: Some(cid.clone()),
                    message: format!("targetChunkId={cid} 非法，未生成修复草稿"),
                    details: None,
                });
            };
            let run_id = format!("knowledge-task-fix-{}", cid);
            // 已在外层 RUN_BUDGET.scope(STEP budget) 内；inner 不自建 scope。
            match crate::routes::knowledge::repair::propose_chunk_repair_inner(
                _state, _workspace_id, object_id, &run_id,
            )
            .await
            {
                Ok(parsed) => {
                    let details = parsed.as_object().map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| {
                                mongodb::bson::to_bson(v).ok().map(|b| (k.clone(), b))
                            })
                            .collect::<Document>()
                    });
                    let missing_n = parsed
                        .get("missingFields")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    Ok(StepOutcome {
                        chunk_id: Some(cid),
                        message: format!(
                            "已为 chunk {cid} 生成 AI 修复草稿（含 {missing_n} 个待补字段），请在 chunk 编辑器审核后 apply"
                        ),
                        details,
                    })
                }
                // budget 超额 / LLM 失败：fail-soft，仍把 chunk 推入待审池。
                Err(err) => Ok(StepOutcome {
                    chunk_id: Some(cid.clone()),
                    message: format!("chunk {cid} 修复草稿生成失败（{err}，fail-soft），请在编辑器手动处理"),
                    details: None,
                }),
            }
        }
```
> `_state`/`_workspace_id` 现为下划线未用参数——本 Task 用到它们，去掉下划线前缀改为 `state: &AppState, workspace_id: &str`（`execute_step` 签名 :437-442 的 `_state`/`_workspace_id`/`_account_id` 中前两个改名；`_account_id` 仍未用保留下划线）。调用点 :260 `execute_step(state, &workspace_id, &account_id, &action, step)` 实参不变。
> 文案用「运营在 chunk 编辑器审核」——**无"人工"二字**，Step 7 lint 自检。
> **红线**：inner 只产 JSON（Task 6 已保证不改 chunk），worker 把它放 details + 把 chunkId 推 needs_review，**不 apply**。

- [ ] **Step 5: 编译确认**

Run: `cargo check --tests 2>&1 | tail -8`
Expected: EXIT 0。可能需在 `mod.rs` 顶部确认 `Document` 已 import（:21 `use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};` 已含）。

- [ ] **Step 6: 集成测试（留 CI，本机无 Docker）**

在 `tests/` 找现有 knowledge_task 集成测试（grep `execute_step`/`knowledge_chat_tasks`），append `#[ignore]` 测试：seed 一个 needs_review chunk + 一个 fix_chunk step 的 task → mock LLM 返回 patch → 跑 worker → 断言：(a) chunk **仍 needs_review 未被改**（integrity_status 不变）；(b) task summary details 的 `needsReviewChunkIds` 含该 chunkId；(c) 该 step 的 `completed_steps[].repairDraft` 含 patch。若无现成文件，靠 Step 5 编译 + code review 核红线（worker 不 apply）+ CI 全量 `--ignored`。

Run（CI）: `cargo test --test <name> -- --ignored 2>&1 | tail -10`
Expected: PASS。

- [ ] **Step 7: 全量 lib + lint**

Run: `cargo test --lib 2>&1 | tail -5`（≥350/0）；`bash scripts/check-no-human-takeover.sh HEAD~1 HEAD 2>&1 | tail -3`（0 violations——fix_chunk 文案用"运营"）。
Expected: lib ≥350/0；lint 0。

- [ ] **Step 8: Commit**

```bash
git add src/knowledge_task/mod.rs tests/
git commit -m "feat(knowledge): ③-b fix_chunk 调 repair inner 产可审草稿,借 needs_review 落审"
```

---

## Self-Review（写完计划后自检，已执行）

**1. Spec 覆盖**（逐 §核对）：
- §2.1 ①-a auto-verify 拦所有类型 → Task 1 ✅
- §2.2 ①-b 冷启动 verified 过滤 → Task 2 ✅
- §3.1 ②-a 请求体加类型字段 → Task 3 ✅
- §3.2 ②-b chunk_type 锁定 → Task 4 ✅
- §3.3 ②-c prompt 加类型 + 删死字段 → Task 5 ✅（含图片 prompt）
- §4.1 ③-a 抽 inner → Task 6 ✅
- §4.2 ③-b fix_chunk 调 inner → Task 7 ✅（add_chunk 按 §4.2/§5 不做，Task 7 Step 2 仅补 `details: None`）
- §5 不做清单 → 计划无任何 Task 触碰（无 model 加回死字段 / 无改类型可变性 / 无 add_chunk 起草 / 无 Reclassify / 无前端逻辑 / 无召回向量化）✅

**2. Placeholder 扫描**：无 TBD/TODO；每个改代码步都有完整代码块与确切 file:line。✅

**3. 类型一致性**：
- Task 1 `enforce_verified_needs_human_audit(String) -> String` — Task 无他处引用旧名 ✅
- Task 3 请求体字段 `wiki_type: Option<String>` / `chunk_type: Option<String>` 与 Task 5 prompt 输出的 `wikiType`/`chunkType`（camelCase wire）经 `#[serde(rename_all="camelCase")]` 对应 ✅
- Task 6 `propose_chunk_repair_inner(&AppState, &str, ObjectId, &str) -> AppResult<Value>` 与 Task 7 调用点签名一致 ✅
- Task 7 `StepOutcome.details: Option<Document>` 与外层 merge / 5 处 `details: None` 一致 ✅

**4. 执行顺序**：① (Task 1-2) → ② (Task 3-5) → ③ (Task 6-7)，与 spec §8 一致；③ 依赖 Task 6 先于 Task 7。三条互不阻塞可独立 commit / review。

## Execution Handoff

计划已保存到 `docs/superpowers/plans/2026-07-01-wiki-three-p0-fixes.md`，共 7 个 Task。

**基线警告**：本 worktree（`prompt-evolution`）落后于 origin/main（缺 #74/#76/#77/#78）。执行前必须基于**最新 origin/main** 开新分支/worktree，避免在旧分支堆叠无关历史。

---
