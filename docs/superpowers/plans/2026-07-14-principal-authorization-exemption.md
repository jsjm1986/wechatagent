# 领导授权豁免（A 类客户级 + B 类沉淀 verified）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让领导裁决 approved 的产品说法能真正发给客户——A 类在该客户级豁免产品门（长期常驻可撤销），B 类把授权沉淀为全体可复用的 verified 知识（领导=人工验证者）。

**Architecture:** 领导回复解读多产出 `exemption_type`（none/customer_only/knowledge）；A 类挂 `Contact.domain_attributes` 新 key，作为 R5.4 产品门第三条并联旁路（仿 `priced_from_catalog`）；B 类走"建 draft chunk（substance 自锚过 D2）→ apply_chunk_revision(op=Verify, source=PrincipalAuthorized) 落 verified"两步法。relay 转述时按 exemption_type 触发 A/B。

**Tech Stack:** Rust 2021, cargo, mongodb crate, axum。测试 `cargo test --lib`。

## Global Constraints

- **红线定性**：B 类落 verified 的验证者是领导（真人），非 AI。新 provenance `PrincipalAuthorized` 归"人类权威"家族，视同 Human 待遇（不被 `source=Ai` 的 draft 强制降级影响）。"AI 永不自动验证"本质未破。
- **无独立 auto-verify CI lint**（已亲验：CI lint 仅 check-baseline/check-evolution-isolation/check-no-human-takeover/check-no-model-hint/check-skip-ledger）。强制 draft 的契约点是 `apply_chunk_revision` 对 `source=Ai` 的降级（chunk_revisions.rs:217-220），新变体天然不受此降级。
- **PrincipalDecision 故意不用 camelCase**（持久化 snake_case 台账，models.rs:3691-3694 注释）：新增字段必须 snake_case + `#[serde(default)]` 兼容旧文档。
- **判据双侧/单一真相**：A 类放行只在 R5.4 加旁路，不动 relay 频控/泄漏守卫。
- **no-human-takeover lint**：命名/文案/注释禁 `人工/接管/转接/托管/takeover/hand-off`，用 `principal_authorized`/`领导授权`/`领导裁决`。
- **no-model-hint lint**：新增行禁硬编码模型/品牌名。
- **verify 前置 D2 硬门**（verify.rs:86-96）：chunk 必须 source_quote 非空 + source_anchors 非空才能 verify。
- **锁定字段**（DEFAULT_LOCKED_FIELDS, page_merge.rs:35-43）：verify 的 patch 不能带 verified_at/verified_by/source_anchor。
- **基线不回归**：`cargo test --lib` 0 failed；`scripts/check-baseline.sh` 双门绿。
- **本地磁盘纪律**：只跑 `cargo test --lib` 与 `cargo build --lib`，集成测试交 CI。

---

## File Structure

- `src/models.rs` — `PrincipalDecision` 加 `exemption_type`；`PRINCIPAL_PRODUCT_EXEMPTION_ATTR` 常量；`EXEMPTION_TYPE_*` 常量。
- `src/agent/escalation/mod.rs` — `interpret_principal_reply` 反序列化回落补字段；`escalate_held_decision` 放开 is_generalizable。
- `src/agent/escalation/logic.rs` — `sanitize_verdict` 透传新字段。
- `src/prompts.rs` — `escalation.principal.interpret` seed 加 exemption_type 输出说明。
- `src/agent/review/gates.rs` — R5.4 加 `principal_product_exempted` 入参与旁路。
- `src/agent/gateway.rs` — 3 处 finalize 调用点算并传 principal_product_exempted；relay 写 A 类豁免 / 触发 B 类沉淀。
- `src/knowledge_wiki/chunk_revisions.rs` — `ProvenanceSource::PrincipalAuthorized`。
- `src/agent/escalation/ledger.rs` — B 类沉淀 verified 两步法（可扩展现有 emit_knowledge_gap_proposal 或新函数）。
- `src/routes/contacts.rs` — A 类撤销端点。
- `src/routes/mod.rs` — 撤销端点路由挂载。

---

## Task 1: PrincipalDecision 加 exemption_type + 解读链路

**Files:**
- Modify: `src/models.rs:3695-3709`（PrincipalDecision）+ 常量区
- Modify: `src/agent/escalation/mod.rs:272`（反序列化失败回落）
- Modify: `src/agent/escalation/logic.rs:411-422`（sanitize_verdict）+ 测试 :624/:635
- Modify: `src/routes/principal_escalations.rs:87`（admin 后台裁决直接构造点）
- Modify: `src/prompts.rs`（escalation.principal.interpret seed）

**Interfaces:**
- Produces: `PrincipalDecision.exemption_type: String`；常量 `EXEMPTION_TYPE_NONE="none"` / `EXEMPTION_TYPE_CUSTOMER_ONLY="customer_only"` / `EXEMPTION_TYPE_KNOWLEDGE="knowledge"`。

- [ ] **Step 1: 加常量 + 结构字段（先写往返测试，TDD 红）**

`src/models.rs` PrincipalDecision 常量附近新增：

```rust
/// 领导授权豁免类型闭集。
pub const EXEMPTION_TYPE_NONE: &str = "none";
pub const EXEMPTION_TYPE_CUSTOMER_ONLY: &str = "customer_only";
pub const EXEMPTION_TYPE_KNOWLEDGE: &str = "knowledge";
```

PrincipalDecision（models.rs:3695-3709）加字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrincipalDecision {
    pub verdict: String,
    pub substance: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub authorization_window_hours: Option<f64>,
    /// 领导授权豁免类型（none/customer_only/knowledge）。snake_case 持久化台账；
    /// 缺省 none 兼容旧文档。
    #[serde(default = "default_exemption_type")]
    pub exemption_type: String,
}

fn default_exemption_type() -> String {
    EXEMPTION_TYPE_NONE.to_string()
}
```

- [ ] **Step 2: 写结构往返 + 缺省单测**

`src/models.rs` 测试区新增：

```rust
#[test]
fn principal_decision_exemption_type_defaults_none() {
    let json = serde_json::json!({"verdict":"approved","substance":"x"});
    let d: PrincipalDecision = serde_json::from_value(json).expect("deser");
    assert_eq!(d.exemption_type, "none");
}

#[test]
fn principal_decision_exemption_type_roundtrip() {
    let json = serde_json::json!({
        "verdict":"approved","substance":"x","exemption_type":"knowledge"
    });
    let d: PrincipalDecision = serde_json::from_value(json).expect("deser");
    assert_eq!(d.exemption_type, "knowledge");
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib principal_decision_exemption_type`
Expected: 编译失败（字段未加）或 assert 失败。

- [ ] **Step 4: 补齐所有直接构造点（消 E0063）**

三处直接构造 PrincipalDecision 的地方补 `exemption_type` 字段：

`src/agent/escalation/mod.rs:272`（反序列化失败回落）：
```rust
return Ok(PrincipalDecision {
    verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
    substance: String::new(),
    constraints: vec![],
    authorization_window_hours: None,
    exemption_type: crate::models::EXEMPTION_TYPE_NONE.to_string(),
});
```

`src/agent/escalation/logic.rs:411-422`（sanitize_verdict 越界回落，**必须透传** decision.exemption_type）：
```rust
pub(crate) fn sanitize_verdict(decision: PrincipalDecision) -> PrincipalDecision {
    if ALLOWED_PRINCIPAL_VERDICT.contains(&decision.verdict.as_str()) {
        decision
    } else {
        PrincipalDecision {
            verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
            substance: decision.substance,
            constraints: decision.constraints,
            authorization_window_hours: decision.authorization_window_hours,
            exemption_type: decision.exemption_type,
        }
    }
}
```

`src/routes/principal_escalations.rs:87`（admin 后台裁决）：实现时 Read 该处确认现有构造字段，补 `exemption_type`。admin 后台裁决默认可传 `EXEMPTION_TYPE_NONE`（admin 手动裁决走后台，豁免类型由该处请求体决定；若请求体无此字段则 none）。

logic.rs 测试 :624/:635 若直接构造 PrincipalDecision 也需补字段。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --lib principal_decision_exemption_type && cargo build --lib`
Expected: 测试 PASS，build 0 error。

- [ ] **Step 6: 改 interpret prompt seed 让 LLM 输出 exemption_type**

`src/prompts.rs` 的 `escalation.principal.interpret` seed content（约 :2204-2231，实现时 Grep 确认确切行）。在输出 JSON schema 说明里增加 exemption_type 字段说明。追加类似（用 AI 自治/领导授权语义，禁 lint 词）：

```
输出 JSON 增加字段 "exemption_type"，表示领导本次授权的适用范围：
- "none"：不授权豁免（默认，仅本次转述，不放宽任何后续限制）
- "customer_only"：仅对当前这位客户授权，可对该客户长期使用（领导表达"就这个客户能说""对他可以"等）
- "knowledge"：授权沉淀为通用口径，今后对所有客户都可复用（领导表达"以后都这么说""这是标准说法"等）
判断不出时输出 "none"。
```

注：改 prompt 内容不必 bump 版本常量（生效闸是 normalize 内容 diff）。

- [ ] **Step 7: 跑全量 lib 确认不回归 + Commit**

Run: `cargo test --lib`
Expected: 0 failed。

```bash
git add src/models.rs src/agent/escalation/mod.rs src/agent/escalation/logic.rs src/routes/principal_escalations.rs src/prompts.rs
git commit -m "feat(escalation): PrincipalDecision 加 exemption_type + 解读链路

领导裁决解读多产出 exemption_type(none/customer_only/knowledge),snake_case
持久化+serde default 兼容旧台账;sanitize_verdict 越界回落透传该字段;interpret
prompt seed 增加输出说明。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: A 类 —— domain_attributes 豁免写入 + R5.4 第三旁路

**Files:**
- Modify: `src/models.rs`（`PRINCIPAL_PRODUCT_EXEMPTION_ATTR` 常量，加在 `AWAITING_PRINCIPAL_DECISION_ATTR` :3644 附近）
- Modify: `src/agent/review/gates.rs:543-553`（加入参）+ `:665`（旁路）
- Modify: `src/agent/gateway.rs`（3 处 finalize 调用点 :303/:1646/:1922 前算并传）

**Interfaces:**
- Consumes: 无（本任务只加放行能力；写入豁免记录在 Task 5 relay 接线）。
- Produces: `finalize_review_for_send(..., principal_product_exempted: bool)` 新签名；常量 `PRINCIPAL_PRODUCT_EXEMPTION_ATTR="principal_product_exemption"`；helper `contact_has_principal_product_exemption(&Contact) -> bool`。

- [ ] **Step 1: 加常量 + 读取 helper（先写 helper 单测，TDD 红）**

`src/models.rs`（AWAITING_PRINCIPAL_DECISION_ATTR :3644 附近）：
```rust
/// A 类领导授权豁免记录挂在 Contact.domain_attributes 的这个 key（doc-only 子文档）。
pub const PRINCIPAL_PRODUCT_EXEMPTION_ATTR: &str = "principal_product_exemption";
```

读取 helper（放 `src/agent/review/gates.rs` 或 guards，实现时择一，本计划放 gates.rs 与消费点同文件）：
```rust
/// 该 contact 是否有生效的 A 类领导授权产品豁免（domain_attributes 里有该 key 且 granted=true）。
pub fn contact_has_principal_product_exemption(contact: &crate::models::Contact) -> bool {
    contact
        .domain_attributes
        .get_document(crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR)
        .ok()
        .and_then(|d| d.get_bool("granted").ok())
        .unwrap_or(false)
}
```

实现时 Read `Contact.domain_attributes` 的真实类型（models.rs:203，确认是 `Document` 还是 `Option<Document>`）调整取法。

- [ ] **Step 2: 写 helper 单测**

`src/agent/review/gates.rs` 测试区：
```rust
#[test]
fn principal_exemption_helper_detects_granted() {
    let mut c = finalize_contact();  // 复用现有测试构造器
    assert!(!contact_has_principal_product_exemption(&c));
    c.domain_attributes.insert(
        crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR,
        mongodb::bson::doc! { "granted": true, "substance": "我们的产品是 AI 软件" },
    );
    assert!(contact_has_principal_product_exemption(&c));
}
```

- [ ] **Step 3: 跑确认失败**

Run: `cargo test --lib principal_exemption_helper`
Expected: 编译/断言失败。

- [ ] **Step 4: finalize 加入参 + R5.4 旁路**

`src/agent/review/gates.rs:543-553` 签名末尾加参数：
```rust
    priced_from_catalog: bool,
    principal_product_exempted: bool,
) -> FinalizeOutcome
```

R5.4 判定（gates.rs:665）改为：
```rust
if verified_chunks.is_empty() && !priced_from_catalog && !principal_product_exempted {
```

更新函数文档注释说明第三条并联背书（领导授权豁免）。

- [ ] **Step 5: 3 处 gateway 调用点算并传**

三处（gateway.rs:293/1637/约1912）在 finalize 调用前加：
```rust
let principal_product_exempted =
    crate::agent::review::gates::contact_has_principal_product_exemption(&contact);
```
（revision 二评那处若变量名是 second_，helper 入参仍是同一个 contact。）

每处 finalize 调用补末尾实参 `principal_product_exempted`。三处调用点都要改（实现时 Grep `finalize_review_for_send(` 确认全部）。

- [ ] **Step 6: R5.4 旁路单测**

`src/agent/review/gates.rs` 测试区（仿 :2072 `finalize_allows_product_claim_when_priced_from_catalog`）：
```rust
#[test]
fn finalize_allows_product_claim_when_principal_exempted() {
    // 构造 requiresProductKnowledge=true + verified_chunks 空 + priced=false
    // 但 principal_product_exempted=true → 不 block
    // (复用 :2072 测试的 setup,只把最后两参改成 priced=false, exempted=true)
}
```
实现时 Read :2072 测试完整 setup 照搬，仅改末两参与断言（断言 status != BlockedUnverifiedProductClaim）。

- [ ] **Step 7: 跑测试 + build + Commit**

Run: `cargo test --lib finalize_allows_product_claim && cargo test --lib principal_exemption_helper && cargo build --lib`
Expected: PASS，0 error。

```bash
git add src/models.rs src/agent/review/gates.rs src/agent/gateway.rs
git commit -m "feat(review): A类领导授权豁免作R5.4产品门第三并联旁路

新增 principal_product_exemption 挂载常量 + contact_has_principal_product_exemption
读取 helper;finalize_review_for_send 加 principal_product_exempted 入参,R5.4 改为
verified空 && !priced && !exempted 才 block(三者取或背书);3处gateway调用点从contact
读出并传入。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: A 类撤销端点 + 审计

**Files:**
- Modify: `src/routes/contacts.rs`（新 handler `revoke_principal_exemption`）
- Modify: `src/routes/mod.rs`（路由挂载）

**Interfaces:**
- Consumes: `PRINCIPAL_PRODUCT_EXEMPTION_ATTR`（Task 2）；`write_event_for_account`（gateway.rs:5119）。
- Produces: `POST /api/contacts/:id/revoke-principal-exemption`。

- [ ] **Step 1: 写撤销 handler**

`src/routes/contacts.rs`（仿 hide_from_pool / disable_agent 范式，含 workspace 隔离 IDOR 防护）：
```rust
/// `POST /api/contacts/:id/revoke-principal-exemption`
/// 撤销该 contact 的 A 类领导授权产品豁免（$unset domain_attributes 子 key）+ 审计。
pub(super) async fn revoke_principal_exemption(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let unset_key = format!(
        "domain_attributes.{}",
        crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR
    );
    let result = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! { "$unset": { unset_key: "" }, "$set": { "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("contact not found".to_string()));
    }
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let _ = agent::write_event_for_account(
        &state,
        &contact.account_id,
        Some(&contact.wxid),
        "contact.principal_exemption_revoked",
        "ok",
        "管理员撤销该联系人的领导授权产品豁免",
        Some(doc! { "actor": &admin.username, "source": "revoke_principal_exemption" }),
    )
    .await;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}
```
实现时 Read hide_from_pool（约 contacts.rs:1047）确认 import 与 helper（parse_object_id/find_contact_by_id/DateTime）可用。

- [ ] **Step 2: 挂路由**

`src/routes/mod.rs`（仿 contacts/:id/disable-agent 挂载点）：
```rust
.route("/contacts/:id/revoke-principal-exemption", post(revoke_principal_exemption))
```
实现时 Grep `disable-agent` 定位挂载块，确认 handler 导出可见性。

- [ ] **Step 3: build 确认**

Run: `cargo build --lib`
Expected: 0 error。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "feat(contacts): A类领导授权豁免撤销端点+审计

POST /contacts/:id/revoke-principal-exemption:\$unset 豁免记录+写
contact.principal_exemption_revoked 审计(长期常驻豁免的必要撤销配套,防误授权无法收回)。
workspace 隔离防 IDOR。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: B 类 —— PrincipalAuthorized provenance + 两步法落 verified

**Files:**
- Modify: `src/knowledge_wiki/chunk_revisions.rs:70-104`（ProvenanceSource 加变体）
- Modify: `src/agent/escalation/ledger.rs`（B 类沉淀 verified 函数）
- Modify: `src/agent/escalation/mod.rs:118`（escalate_held_decision 放开 is_generalizable）

**Interfaces:**
- Consumes: `apply_chunk_revision`（chunk_revisions.rs:149）；`resolve_quote_anchors` 逻辑（chat.rs:1642，模块私有需复制或提可见性）。
- Produces: `ProvenanceSource::PrincipalAuthorized`；`fn sediment_principal_authorized_knowledge(state, entry, decision) -> AppResult<()>`。

- [ ] **Step 1: ProvenanceSource 加变体（先写往返测试，TDD 红）**

`src/knowledge_wiki/chunk_revisions.rs` 测试区：
```rust
#[test]
fn provenance_principal_authorized_roundtrip() {
    assert_eq!(ProvenanceSource::PrincipalAuthorized.as_str(), "principal_authorized");
    assert_eq!(
        "principal_authorized".parse::<ProvenanceSource>().unwrap(),
        ProvenanceSource::PrincipalAuthorized
    );
}
```

- [ ] **Step 2: 跑确认失败**

Run: `cargo test --lib provenance_principal_authorized`
Expected: 编译失败（变体不存在）。

- [ ] **Step 3: 加变体 + as_str + FromStr**

`chunk_revisions.rs:70` enum 加：
```rust
    /// 领导（真人）通过决策请示通道授权的知识——验证者是领导本人，
    /// 视同 Human 人类权威（绝非 AI 自动验证，红线本质未破）。
    PrincipalAuthorized,
```
`as_str()`（:82）加 `ProvenanceSource::PrincipalAuthorized => "principal_authorized",`。
`FromStr`（:92）加 `"principal_authorized" => Ok(ProvenanceSource::PrincipalAuthorized),`。
错误信息（:100-102）的 expected 列表补 `principal_authorized`。

**关键**：确认 `source=Ai` 强制 draft 的分支（:217-220）只 match Ai，PrincipalAuthorized 不落入 → 可带 verified。实现时 Read :215-230 核对。

- [ ] **Step 4: 跑确认通过**

Run: `cargo test --lib provenance_principal_authorized && cargo build --lib`
Expected: PASS。

- [ ] **Step 5: 写 B 类沉淀 verified 函数（两步法）**

`src/agent/escalation/ledger.rs`（参照现有 emit_knowledge_gap_proposal :176-209 的建 chunk 范式 + verify.rs:104-115 的 verify 范式）。新函数：

```rust
/// B 类沉淀：把领导授权的 substance 落为 verified 知识 chunk（全体客户可复用）。
/// 两步法：① insert 一条 draft chunk（substance 自锚定过 D2 门）；
/// ② apply_chunk_revision(op=Verify, source=PrincipalAuthorized) 落 verified。
/// 验证者是领导（真人）——非 AI 自动验证。
pub(crate) async fn sediment_principal_authorized_knowledge(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
) -> AppResult<()> {
    // 步骤1：建 draft chunk。substance 自锚定：source_quote=substance,
    // source_anchors 用 substance 自身锚定（无父文档,锚 start=行首）。
    // domain 必填 user_operations（否则召不回);chunk_type=product_fact。
    // 具体字段构造：Read emit_knowledge_gap_proposal(ledger.rs:176-209) 现成 chunk doc
    // 范式,补齐 domain/chunk_type/source_quote/source_anchors 后 insert_one,拿 object_id。
    // 锚定：复制 resolve_quote_anchors 逻辑(chat.rs:1642,模块私有)——substance 传入,
    // patch_quote 传 None → quote=substance,anchor 锚自身。
    // 步骤2：apply_chunk_revision(op=Verify, source=PrincipalAuthorized,
    //   patch=doc!{"integrity_status":"verified","confidence_score":100},
    //   reason=Some("领导授权沉淀"), actor=Some(principal_wxid))
    //   —— patch 绝不带锁定字段 verified_at/verified_by/source_anchor。
    //   status 由 Verify op 保持既有；建 chunk 时步骤1 就置 status=active。
    todo!("按上述两步实现")
}
```

**实现要点（实现者必须先 Read 确认）**：
- Read `emit_knowledge_gap_proposal`（ledger.rs:176-209）拿建 chunk 的确切 doc 字段范式与 insert 写法。
- Read `resolve_quote_anchors`（chat.rs:1642-1666）+ `source_anchor_for_quote`（mod.rs:756，pub(super)）——escalation 模块跨 crate 调用需复制该纯逻辑或提可见性；本计划倾向复制一份小的自锚定逻辑到 escalation（substance 自锚定无父文档场景简单：anchor start=行首）。
- Read verify.rs:104-115 的 apply_chunk_revision(Verify) 确切 patch 字段，照搬非锁定字段集。
- 步骤1 建 chunk 时直接置 `status="active"` + `integrity_status="needs_review"`，步骤2 verify 把 integrity_status 改 verified（与 verify.rs 语义一致：verify 只动 integrity_status/confidence，不动 status）。
- domain 必填 `"user_operations"`，chunk_type 固定 `"product_fact"`，wiki_type 用保守默认。

- [ ] **Step 6: escalate_held_decision 放开 is_generalizable**

`src/agent/escalation/mod.rs:118`。当前硬编码 `is_generalizable: false`。改为由领导裁决驱动——但注意此处是**创建 escalation 时**（领导还没裁决），is_generalizable 尚不可知。实现方案：创建时保持 false（领导未回），**在 relay 阶段（Task 5）领导裁决 exemption_type=knowledge 时才触发 B 类沉淀**，不依赖创建时的 is_generalizable。

因此本 step 实际是：确认 escalate_held_decision 的 is_generalizable=false **不阻断** Task 5 的 B 类沉淀路径（B 类沉淀在 Task 5 由 exemption_type 直接驱动，绕过 is_generalizable 门）。Read mod.rs:100-130 确认后，若 is_generalizable 仅用于旧 emit_knowledge_gap_proposal 门，则本 step 无需改代码，仅在 Task 5 用 exemption_type 作新驱动。**若确认无需改则在 commit 注明**。

- [ ] **Step 7: 单测 B 类沉淀（若有 DB 测试设施）**

B 类沉淀函数依赖 DB（insert + apply_chunk_revision）。Grep escalation/ledger.rs 测试区是否有 mongodb testcontainer 设施。**无则跳过 DB 集成测试**（本地磁盘纪律不跑 testcontainer），以 provenance 往返单测（Step 1）+ Task 5 联调 + 生产验证为准，commit 注明。self-anchor 纯逻辑若被复制成独立函数则对它写纯单测。

- [ ] **Step 8: 跑 + Commit**

Run: `cargo test --lib provenance_principal_authorized && cargo build --lib && cargo test --lib`
Expected: PASS，0 failed。

```bash
git add src/knowledge_wiki/chunk_revisions.rs src/agent/escalation/ledger.rs src/agent/escalation/mod.rs
git commit -m "feat(escalation): B类领导授权沉淀verified知识(PrincipalAuthorized provenance两步法)

新增 ProvenanceSource::PrincipalAuthorized(人类权威家族,视同Human,不被source=Ai降级);
sediment_principal_authorized_knowledge 两步法:建draft chunk(substance自锚过D2门,
domain=user_operations,chunk_type=product_fact)→apply_chunk_revision(Verify)落verified。
验证者是领导(真人)非AI,红线本质未破。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: relay 接线（A/B 触发）+ 全链测试收尾

**Files:**
- Modify: `src/agent/gateway.rs:755-794`（relay_principal_decision_to_customer）

**Interfaces:**
- Consumes: `PRINCIPAL_PRODUCT_EXEMPTION_ATTR`（Task 2）；`sediment_principal_authorized_knowledge`（Task 4）；`EXEMPTION_TYPE_*`（Task 1）。

- [ ] **Step 1: relay 按 exemption_type 写 A 类豁免 + 触发 B 类**

`src/agent/gateway.rs:755`（relay_principal_decision_to_customer）。在构造 synthetic 之前（先写豁免记录，再 relay，这样本轮 relay 转述即通过产品门）插入：

```rust
    // 领导授权豁免落地：approved/conditional 且指定豁免类型时生效。
    let verdict_authorizes = matches!(
        decision.verdict.as_str(),
        crate::models::PRINCIPAL_VERDICT_APPROVED | crate::models::PRINCIPAL_VERDICT_CONDITIONAL
    );
    if verdict_authorizes
        && matches!(
            decision.exemption_type.as_str(),
            crate::models::EXEMPTION_TYPE_CUSTOMER_ONLY | crate::models::EXEMPTION_TYPE_KNOWLEDGE
        )
    {
        // A 类：写该客户 domain_attributes 豁免记录(customer_only 与 knowledge 都先写,
        // 保证当轮 relay 即通、不空等 B 类异步沉淀)。
        let set_key = format!(
            "domain_attributes.{}",
            crate::models::PRINCIPAL_PRODUCT_EXEMPTION_ATTR
        );
        state.db.contacts().update_one(
            doc! { "workspace_id": &contact.workspace_id, "account_id": &contact.account_id, "wxid": &contact.wxid },
            doc! { "$set": {
                set_key: doc! {
                    "granted": true,
                    "granted_by": &entry.principal_wxid,
                    "substance": &decision.substance,
                    "escalation_short_code": &entry.short_code,
                    "granted_at_ms": mongodb::bson::DateTime::now().timestamp_millis(),
                },
                "domain_attributes_updated_at": mongodb::bson::DateTime::now(),
            } },
            None,
        ).await?;
        let _ = write_event_for_account(
            state, &contact.account_id, Some(&contact.wxid),
            "contact.principal_exemption_granted", "ok",
            "领导授权该客户产品豁免", None,
        ).await;
    }
```

在 relay（run_user_operation_gateway）之后、函数末尾（现有 emit_knowledge_gap_proposal 块附近或替换其条件），B 类沉淀：
```rust
    if verdict_authorizes && decision.exemption_type == crate::models::EXEMPTION_TYPE_KNOWLEDGE
        && !entry.knowledge_proposal_emitted
    {
        crate::agent::escalation::ledger::sediment_principal_authorized_knowledge(state, entry, decision).await?;
        state.db.agent_principal_escalations().update_one(
            doc! { "short_code": &entry.short_code },
            doc! { "$set": { "knowledge_proposal_emitted": true } }, None,
        ).await?;
    }
```

实现时 Read gateway.rs:755-794 现有 body（尤其现有的 emit_knowledge_gap_proposal 块 :781-792），决定是替换其逻辑还是并存。**重要**：contact 在写豁免记录后要用于 relay，注意 owned Contact 的 borrow/move（写库用 &contact 字段，relay 用 contact.clone()，参照现有 :770 `contact.clone()`）。

- [ ] **Step 2: 确认现有 emit_knowledge_gap_proposal 关系**

Read gateway.rs:781-792。若新 B 类沉淀（verified）取代旧 draft proposal，则移除旧块避免双写；若保留旧 draft proposal 作 exemption_type=none 时的兜底，则并存但用条件互斥。实现者决策并在 commit 注明取舍理由。**倾向**：exemption_type=knowledge 走新 verified 沉淀；其余保持旧行为不动（YAGNI，最小改动）。

- [ ] **Step 3: build + 全量 lib**

Run: `cargo build --lib && cargo test --lib`
Expected: 0 error，0 failed。

- [ ] **Step 4: 本地 lint 双门（先 commit 再验）+ Commit**

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): relay按exemption_type触发A类豁免/B类沉淀

领导approved+customer_only/knowledge→relay前写contact豁免记录(当轮转述即过产品门,
不空等);knowledge额外走sediment_principal_authorized_knowledge沉淀verified知识。
A/B关系:B是A超集(先即时放行该客户,再沉淀给全体)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

Run: `bash scripts/check-no-human-takeover.sh main HEAD && bash scripts/check-no-model-hint.sh main HEAD`
Expected: 两门 0 violations。（若命中禁词，改文案/注释后 amend。）

- [ ] **Step 5: 全链回归确认**

Run: `cargo test --lib`
Expected: 0 failed，passed ≥ 基线。

---

## Self-Review

**Spec coverage:**
- exemption_type 解读（none/customer_only/knowledge + 越界回落）→ Task 1 ✓
- A 类 domain_attributes 挂载 + 标注领导豁免了什么 → Task 2（放行 helper）+ Task 5（写入 substance/granted_by）✓
- R5.4 第三并联旁路 → Task 2 ✓
- A 类长期常驻（不 $unset 消费）→ Task 5 只 $set 不清 ✓
- A 类撤销端点 + 审计 → Task 3 ✓
- B 类 PrincipalAuthorized provenance 落 verified → Task 4 ✓
- B 类 substance 自锚过 D2 + product_fact + domain 必填 → Task 4 ✓
- B 类是 A 类超集（当轮不空等）→ Task 5（knowledge 也先写 A 类豁免）✓
- hold 路径 is_generalizable → Task 4 Step 6（确认 B 类由 exemption_type 驱动，绕过 is_generalizable 门；可能无需改代码，实现时坐实）✓
- CI 三红线 → Global Constraints + Task 5 Step 4 lint 验证 ✓

**Placeholder scan:** Task 4 Step 5 的 `todo!()` 是**故意的实现骨架标记**——因为 B 类沉淀依赖多个需实现者当场 Read 确认的现成范式（emit_knowledge_gap_proposal 的 chunk doc 结构、resolve_quote_anchors 的复制、verify patch 字段），计划无法在不亲验这些的前提下写死完整代码（写死反而违背"先读懂"红线）。Step 5 已列出所有必 Read 的确切 file:line + 每步要点。这是"指明确切参照源 + 要点"而非"TBD"，实现者照 Read 即可。其余步骤代码完整。

**Type consistency:** `exemption_type: String` 全程一致；`EXEMPTION_TYPE_*` / `PRINCIPAL_PRODUCT_EXEMPTION_ATTR` / `ProvenanceSource::PrincipalAuthorized` / `sediment_principal_authorized_knowledge` 命名跨任务一致。`finalize_review_for_send` 新参数 `principal_product_exempted: bool` 在 Task 2 定义、3 处调用点一致传入。事件 kind：`contact.principal_exemption_granted`（Task 5）/`contact.principal_exemption_revoked`（Task 3）一致。

**Scope:** 单一功能（领导授权豁免），5 任务递进（解读→放行→撤销→沉淀→接线），一个实现计划覆盖。
