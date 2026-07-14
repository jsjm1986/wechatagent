# 领导授权豁免（A 类一次性客户级豁免 + B 类沉淀复用）设计

> 状态：设计已获批，待写实现计划（writing-plans）。
> 日期：2026-07-14

## 背景与根因

系统的「决策请示通道」：AI 遇到超职权的问题（如客户问产品但知识库为空）→ 向幕后领导请示 → 领导裁决 → AI 用自己口吻转述给客户（`docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`）。

**已代码级证实的缺陷**（4 路调研 file:line 亲验，交叉一致）：领导裁决 `approved` 并给出实质内容（如"我们的产品是 AI 软件"）后，`relay_principal_decision_to_customer`（`src/agent/gateway.rs:755`）把 substance 包装成 `synthetic_principal_relay` 合成入站消息，走**同一个** `run_user_operation_gateway`（`gateway.rs:616`，唯一入口）。产品准确性硬门 R5.4（`src/agent/review/gates.rs:658-691`）对这条转述的 substance **重新判定**：`verified_chunks.is_empty() && !priced_from_catalog` → `blocked_unverified_product_claim`（`gates.rs:665/684`，硬 block、无 revision）。由于知识库为空、领导口头授权不写知识库，领导 approved 的产品说法被系统自己的产品门二次拦截，客户永远收到"没拿到资料不敢瞎说"。

生产实证：`agent_principal_escalations` 2026-07-10 一条 `verdict=approved, substance="回复他 我们的产品是 ai软件"`，但吴界（wxid_ydzaomn4scsb12）实际收到的始终是兜底话术。

`is_principal_relay_trigger` 只豁免频控（`gateway.rs:3123-3199`）、只加严泄漏守卫（`gateway.rs:2480-2499`），**不碰产品门**——无任何现成 bypass。

## 设计目标

领导裁决时可选两类豁免，降低人工重复工作：
- **A 类（一次性客户级豁免）**：领导针对**该客户**授权后，该客户后续所有产品说法放行产品门。长期常驻、该客户级全域放行、可 admin 撤销。在联系人画像标注"该用户由领导 xx 豁免了什么"。
- **B 类（沉淀复用）**：领导授权内容沉淀为**所有客户**可复用的 verified 知识 chunk。

## 关键红线定性（用户拍板）

**「领导裁决 = 人工验证」**：B 类落 `integrity_status=verified` 的验证者是**领导（真人）**，不是 AI。项目红线是「**AI** 永不自动验证知识」——B 类把人工验证入口从「事后知识库复核」前移到「领导裁决当下」，验证主体仍是真人，**红线本质未破**。代码层将其建模为一条新的「人类权威」provenance（`PrincipalAuthorized`，等同 `Human` 家族），而非 AI/自动路径。

## 判据与挂载点（4 路调研亲验事实）

- **产品门唯一判定点**：`finalize_review_for_send`（`src/agent/review/gates.rs:543`）R5.4 块（`:658-691`）。纯函数、不读 DB，所有输入由 gateway 从 `gateway.rs:1637` 附近注入（`priced_from_catalog` 就是这样的现成模式）。
- **A 类挂载点**：`Contact.domain_attributes`（`src/models.rs:203`，自由 BSON KV，无 `deny_unknown_fields`，加 key 不改 struct）。读写范式照搬 `AWAITING_PRINCIPAL_DECISION_ATTR`（常量 `models.rs:3644`；写 dotted-key `$set` + `domain_attributes_updated_at`；读走 `build_decision_signals_text`）。`ApiContact.domain_attributes` 原样投影（`models.rs:3532/3594`）→ admin 天然可见。
- **B 类写入点**：`apply_chunk_revision`（`src/knowledge_wiki/chunk_revisions.rs:149`），统一入口含三层保护 + 双写审计 + provenance + catalog rebuild。`ProvenanceSource` 枚举（`chunk_revisions.rs:70-79`）当前 4 值 Ai/Human/Rule/Imported。
- **verified 出口约束**：落 verified 必须过 D2 硬门（`src/routes/knowledge/verify.rs:86-96`：`source_quote` 非空 + `source_anchors` 非空，否则 BadRequest）。召回侧硬过滤 `domain="user_operations" + status="active" + integrity_status="verified"`（`knowledge_router.rs:63-79`）。产品门只认 `used_knowledge_ids` 引用到的 verified chunk（`guards.rs:315-324/339`）。
- **hold 路径缺陷**：`escalate_held_decision`（`src/agent/escalation/mod.rs:118`）硬编码 `is_generalizable=false`，导致最典型的 B 类场景（产品问题走 `high_risk_gated`）当前根本不触发沉淀 —— B 类需放开这处。

## 四部分设计

### 第 1 部分：领导裁决解读 A/B 类型

`interpret_principal_reply`（`src/agent/escalation/mod.rs:243`）用 LLM 解读领导回复，除现有 verdict/substance/constraints 外，多解读一个 `exemption_type` 字段：
- `none`：不豁免（默认）。
- `customer_only`：A 类，仅该客户。
- `knowledge`：B 类，沉淀知识。

领导可用自然语言表达（"就这个客户能说"→A；"以后都能这么说"→B）。**解读失败/越界 → 保守回落 `customer_only`（A 类，最小影响面，只动该客户）**，与现有 `sanitize_verdict` 回落 deferred 的保守范式一致。

`PrincipalDecision`（`models.rs:3695-3709`）新增字段 `exemption_type: String`（snake_case 持久化，与现有字段一致，`#[serde(default)]` 兼容旧台账）。仅 `verdict=approved`（或 conditional）时 `exemption_type` 才有意义。

### 第 2 部分：A 类 —— 该客户级全域放行 + 长期常驻

**数据落点**（`Contact.domain_attributes` 新 key，doc-only，常量 `PRINCIPAL_PRODUCT_EXEMPTION_ATTR` 加在 `models.rs:3644` 附近）：
```json
domain_attributes.principal_product_exemption = {
  "granted": true,
  "granted_by": "<principal_wxid>",
  "substance": "我们的产品是 AI 软件",
  "escalation_short_code": "<关联请示短码>",
  "granted_at_ms": <i64 时间戳>
}
```
`substance`/`granted_by` 供 admin 展示"该用户由领导 xx 豁免了什么"；`granted_at_ms` 作豁免时间。

**写入时机**：`relay_principal_decision_to_customer`（`gateway.rs:755`）中，领导 `verdict=approved` 且 `exemption_type=customer_only` → **relay 转述前**先写这条记录（dotted-key `$set` + 同步 `domain_attributes_updated_at`）。

**放行逻辑**：`finalize_review_for_send`（`gates.rs:543`）新增入参 `principal_product_exempted: bool`（仿 `priced_from_catalog` 模式，由 gateway 从 contact 的 `domain_attributes` 读出后传入；纯函数不碰 Contact 内部）。R5.4 判定（`gates.rs:665`）改为：
```rust
if verified_chunks.is_empty() && !priced_from_catalog && !principal_product_exempted {
```
一旦 granted，该客户所有产品说法放行产品门（该客户级全域放行）。

**长期常驻**：不自动过期、不一次性消费（**不**照搬 awaiting 的 `$unset`）。

**撤销入口**（长期常驻的必要配套，防误授权无法收回）：新增 admin 端点 `POST /api/contacts/:id/revoke-principal-exemption` → `$unset` 该 key + 写审计事件 `contact.principal_exemption_revoked`（复用 `write_event_for_account`，`gateway.rs:5119`）。写入豁免时也写审计 `contact.principal_exemption_granted`。

**admin 可见**：`domain_attributes` 已投影进 `ApiContact`，写入即经 `GET /api/contacts/:id` 暴露；前端读该 key 经 label 映射展示（前端为可选增强，不阻断后端）。

### 第 3 部分：B 类 —— 领导授权即 verified 知识

**新 provenance**：`ProvenanceSource` 枚举（`chunk_revisions.rs:70`）新增变体 `PrincipalAuthorized`，`as_str()="principal_authorized"`，`FromStr` 同步。它归入「人类权威」家族——落 verified 时**视同 Human 待遇**（不像 Ai 被强制降级 draft）。代码注释明确定性：验证者是领导（真人），非 AI 自动验证。

**写入**：领导 `verdict=approved` 且 `exemption_type=knowledge` → 走 `apply_chunk_revision`（`chunk_revisions.rs:149`），以 `source=PrincipalAuthorized` 写一条 chunk：
- `integrity_status="verified"` + `status="active"`（人类权威路径，不降级）。
- `domain="user_operations"`（**必填**，现有 proposal 桩因 domain 空串永远召不回，这里必须补对）。
- `chunk_type="product_fact"`（固定：领导口述的就是产品事实，正是产品门消费的类型；不跑 LLM 再分类，避免历史分类丢失 bug）。`wiki_type` 用保守默认（`entity` 或落库端 coerce 默认）。
- `source_quote` = substance 本身；`source_anchors` = substance 自锚定（仿 `chat.rs:1685` 的 `resolve_quote_anchors`，把 substance 自身作为 quote 源过 D2 门）。**无需领导额外提供依据文本**（符合降人工量目标）。
- body/title 由 substance 生成。

**hold 路径放开**：`escalate_held_decision`（`escalation/mod.rs:118`）不再硬编码 `is_generalizable=false`——改为允许 `high_risk_gated`（产品问题）场景在领导选 B 类时触发沉淀。具体：`is_generalizable` 由领导裁决的 `exemption_type=knowledge` 驱动，而非创建时写死。

**生效链**：写入 verified chunk 后，relay 转述时 `knowledge_router` 检索到它（满足 active+verified+domain 过滤）→ 产品门 `compute_verified_chunks` 命中 → 通过。今后**所有客户**同类产品说法自然过门。

**A/B 关系**：B 类沉淀是异步生效（要等 chunk 写入 + 下轮检索）。为保证**当轮领导授权不空等**，领导选 B 类时**同时**执行 A 类的该客户放行（B 类是 A 类的超集：既即时放行该客户，又沉淀给全体）。即 `exemption_type=knowledge` → 先写 A 类 contact 豁免（当轮 relay 即通），再写 B 类 verified chunk（全体复用）。

### 第 4 部分：CI 红线处理 + 测试

**红线与契约处理**（已亲验，非猜测）：
- `check-no-human-takeover`（CI lint 脚本）：豁免命名/审计文案/注释用 AI 自治与「领导授权」语义，禁 `人工/接管/转接/托管/takeover/hand-off`。用 `principal_authorized` / `领导授权` / `领导裁决`。
- **provenance 闭集**：新 `PrincipalAuthorized` 变体需在所有校验点注册——`as_str()`（`chunk_revisions.rs:82`）、`FromStr`（`chunk_revisions.rs:92`）、序列化往返单测。
- **「AI 永不自动验证」是代码内契约，不是独立 CI lint**（已亲验：CI lint 仅 check-baseline / check-evolution-isolation / check-no-human-takeover / check-no-model-hint / check-skip-ledger 五个，无 auto-verify lint）。该契约的强制点是 `apply_chunk_revision`（`chunk_revisions.rs:218`）**只对 `source=Ai` 强制 draft+needs_review**——新增 `PrincipalAuthorized` 变体**天然不被这里降级**，可带 `verified+active` 写入，无需改动降级逻辑。
- **B 类必须走 `apply_chunk_revision` 直写，不走 `verify.rs` 的 auto-verify 路径**：`enforce_verified_needs_human_audit`（`verify.rs:554`）对所有类型强制把 verified 降级 needs_human_audit，那是 LLM 自评的 auto-verify 批处理专用（凭 LLM 自评不足以替代人核）。B 类的验证者是领导（真人），走 `apply_chunk_revision(source=PrincipalAuthorized)` 直接落 verified，语义与「人工在知识库点 verify」等价，绕开 LLM 自评那条链。代码注释须明确此定性。

**测试**：
- A 类：`finalize_review_for_send` 加 `principal_product_exempted=true` 时，即使 `verified_chunks.is_empty() && !priced_from_catalog` 也不 block（新纯函数分支单测）。豁免记录读写往返。撤销端点 `$unset` + 审计。
- B 类：`ProvenanceSource::PrincipalAuthorized` 的 as_str/FromStr 往返；`apply_chunk_revision(source=PrincipalAuthorized)` 落 `verified+active`（不被降级 draft）；substance 自锚定过 D2 门；domain/chunk_type 正确。
- `exemption_type` 解读：LLM 解读三值 + 越界回落 `customer_only`。
- 基线门 `cargo test --lib` 不回归；`scripts/check-baseline` 双门绿；三 CI 红线 lint 绿。
- 本地磁盘纪律：只跑 `cargo test --lib`，集成测试交 CI。

## 数据流总览

```
客户问产品 → 知识库空 → blocked_unverified_product_claim → 请示领导
领导回复 (approved + substance + exemption_type)
  ├─ none:        当前行为（转述仍可能被产品门拦——此路径不改，除非领导给豁免）
  ├─ customer_only [A]:
  │    写 contact.domain_attributes.principal_product_exemption (+审计)
  │    → relay 转述放行 + 该客户后续全域放行（长期常驻，可 admin 撤销）
  └─ knowledge [B]:
       ① 先写 A 类 contact 豁免（当轮 relay 即通，不空等）
       ② apply_chunk_revision(source=PrincipalAuthorized, verified, active,
          domain=user_operations, chunk_type=product_fact, substance 自锚)
       → knowledge_router 检索到 → 产品门通过 → 全体客户今后同类说法自然过门
```

## 不做的事（YAGNI / 明确排除）

- A 类不做自动过期 / 次数限制 / 话题范围限定（用户定：长期常驻 + 该客户级全域放行，最松弛最记得住）。
- B 类沉淀的 chunk 走正常知识库管理（admin 可在知识库编辑/停用），不另造专用管理界面。
- 前端展示豁免标注为可选增强，不阻断后端交付。
- 不改 `none` 路径的现有转述行为。
- 不豁免频控/泄漏守卫（那些与产品门正交，本设计只碰产品门）。

## 影响面

- `src/models.rs`：`PrincipalDecision` 加 `exemption_type`；`PRINCIPAL_PRODUCT_EXEMPTION_ATTR` 常量。
- `src/agent/escalation/mod.rs`：`interpret_principal_reply` 解读 exemption_type；`escalate_held_decision` 放开 is_generalizable。
- `src/agent/gateway.rs`：`relay_principal_decision_to_customer` 写 A 类豁免 / 触发 B 类沉淀；finalize 调用点传 `principal_product_exempted`。
- `src/agent/review/gates.rs`：R5.4 加第三条旁路 + 新入参。
- `src/knowledge_wiki/chunk_revisions.rs`：`ProvenanceSource::PrincipalAuthorized`。
- `src/routes/contacts.rs`：撤销端点。
- provenance 闭集：`PrincipalAuthorized` 注册到 as_str/FromStr/往返单测。无独立 auto-verify lint 需改（该契约由 apply_chunk_revision 的 source=Ai 降级分支承载，新变体天然不受降级）。
