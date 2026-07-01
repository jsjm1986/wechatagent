# M13 前端 saveOperationProfile 清空 profile_attributes 修复设计

> 日期：2026-07-02
> 分支：`fix/m13-profile-attributes-preserve`（从 origin/main b19df42 切，含 H7/H1/H11）
> 来源：终极审判审计 M13（原报告标 High，UPHELD）

## 1. 漏洞描述（对最新代码亲验）

`PUT /api/contacts/:id/operation-profile`(`update_operation_profile`,contacts.rs:759)是运营在前端「运营画像」表单点保存时调的端点。它无条件用 payload 覆写 contact 的多个字段,其中 contacts.rs:799:

```rust
let mut set_doc = doc! {
    "tags": payload.tags,
    "commitments": commitments_bson,
    "follow_up_policy": normalize_optional(payload.follow_up_policy),
    "profile_attributes": payload.profile_attributes,   // ← 无条件覆写
    "profile_updated_at": DateTime::now(),
    "updated_at": DateTime::now(),
};
```

`OperationProfileRequest.profile_attributes`(contacts.rs:42)带 `#[serde(default)]`,请求缺该字段时反序列化成**空 Document**。

**触发链(亲验)**:前端 `saveOperationProfile`(userOpsStore.ts:592-596)的 PUT body 只发 3 个字段:
```ts
await api.put(`/api/contacts/${selected.id}/operation-profile`, {
  relationshipType: relationshipType || undefined,
  lastCommitment: profileEditDraft.lastCommitment || undefined,
  followUpPolicy: profileEditDraft.followUpPolicy || undefined,
});
```
**不带 `profileAttributes`**。于是:请求到后端 → `payload.profile_attributes` = 空 Document → contacts.rs:799 `$set profile_attributes = {}` → **AI 在 gateway 侧积累的 `profile_attributes` 被清空**。

运营点保存 relationship_type / commitment / follow-up policy 是**极常见**操作,每次都会误清 AI 画像属性。

**数据用途(亲验)**:`profile_attributes` 确实喂给决策 prompt——`decision.rs:836-837` 在 `include_relational` 档把 `contact.profile_attributes` 序列化进 `profile_attributes_text` 注入 Reply Agent。清空 = AI 丢失已积累的客户画像维度。

**对比正确范式(亲验)**:AI 侧 gateway 写 `profile_attributes` 是**非空才写**——gateway.rs:4034:
```rust
if !decision.profile_attributes.is_empty() {
    set_doc.insert("profile_attributes", decision.profile_attributes.clone());
}
```
两路对同一字段的写策略不一致:gateway 非空守卫、admin handler 无条件覆写。M13 = admin handler 缺了这个守卫。

## 2. 根因

`update_operation_profile` 对 `profile_attributes` 无条件 `$set`,而该端点的前端调用方从不发送此字段(前端表单不管理 profile_attributes,它由 AI 在 gateway 积累)。缺少 gateway.rs:4034 那样的「非空才写」守卫,导致「运营保存别的字段」误伤「AI 积累的画像属性」。

## 3. 方案选型

### 方案 A（选定）：admin handler 镜像 gateway 的非空守卫

把 contacts.rs:799 的无条件 `$set` 改成「`payload.profile_attributes` 非空才写」,与 gateway.rs:4034 完全对齐。空(前端不发/发空)→ 不写 → 保留 AI 积累的现值。

**为什么选 A**:
- 与 gateway.rs:4034 现成正确范式一字对齐,消除两路写策略不一致。
- 根治:即便未来别的调用方也省略该字段,AI 属性都不会被误清。
- 前端**无需改**——前端本就不管理 profile_attributes,不发送是正确的;真正的 bug 是后端不该把「没发」当成「要清空」。
- 改动最小:一个 handler 内一处 `doc!` 字段挪成条件插入。

### 否决方案 B：前端补发全量 profile_attributes
让前端 saveOperationProfile 带上当前的 profileAttributes 一起 PUT。缺点:①前端表单根本不编辑 profile_attributes,让它回传是把「只读展示值」当「可写值」往返,一旦前后端时序错位(AI 刚更新、前端拿的是旧快照)反而覆写掉更新的值;②没解决「后端把缺字段当清空」的根因,别的调用方仍会中招。否决。

### 否决方案 C：去掉 profile_attributes 的 `#[serde(default)]`,改必填
请求缺字段直接 400。太激进:会破坏「只想改 relationship_type 就不用带 profile_attributes」的合理用法,且把契约收紧到所有调用方都必须回传。否决。

## 4. 核心改动

落点:`src/routes/contacts.rs` 的 `update_operation_profile`。

### 4.1 set_doc 移除无条件 profile_attributes + 非空才插入（contacts.rs:795-802）
```rust
    let mut set_doc = doc! {
        "tags": payload.tags,
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    // 与 gateway.rs 写回一致:profile_attributes 非空才写。前端「运营画像」表单
    // 不管理 profile_attributes(它由 AI 在 gateway 积累),PUT 时不带该字段 →
    // payload 反序列化为空 Document。无条件 $set 会把 AI 积累的画像清空(M13),
    // 故空则跳过、保留现值。
    if !payload.profile_attributes.is_empty() {
        set_doc.insert("profile_attributes", payload.profile_attributes);
    }
```
（`profile_updated_at` 保持无条件写——该 handler 总会更新 tags/commitments 等其它画像字段,时间戳语义正确。其余字段与流程一字不动。）

### 4.2 测试可见性：handler + 请求体改 pub
为直调 handler 集成测试（与 `contact_manual_tags_integration.rs` 同范式,该测试直调 `pub` 的 `update_manual_tags`）:
- `pub(super) async fn update_operation_profile` → `pub async fn update_operation_profile`（contacts.rs:759）
- `pub(super) struct OperationProfileRequest` → `pub struct OperationProfileRequest`（contacts.rs:31）

（字段保持私有——测试用 `serde_json::from_value` 构造,不需字段级 pub,同 manual_tags 范式。）

**不动**:前端 userOpsStore.ts（不发 profile_attributes 是正确的）、gateway.rs:4034（已是正确范式）、OperationProfileRequest 其它字段、handler 其余逻辑（stage/intent 校验、relationship_type 写入、跨 workspace 过滤）。

## 5. 行为验证（改动后）

| 场景 | 改动前 | 改动后 |
| --- | --- | --- |
| 前端保存(不带 profileAttributes) | payload 空 → $set {} → **AI 画像被清空** | 空 → 跳过 → **AI 画像保留** |
| 调用方带非空 profile_attributes | $set 覆写 | 非空 → 写入(合法覆写,不回归) |
| tags/commitment/follow_up 正常更新 | 写入 | 不变(仍写入) |
| 跨 workspace | NotFound | 不变 |

## 6. 测试设计

新增 `tests/contact_operation_profile_integration.rs`（`#[ignore]` + Docker,与 `contact_manual_tags_integration.rs` 同范式;直调 handler + `test_admin` + `serde_json::from_value` 构造私有请求体）。

**测试 1（M13 核心红线）—— 前端式请求(不带 profileAttributes)不清空 AI 画像:**
1. seed 一个 contact,`profile_attributes = {"budget": "high", "decision_role": "owner"}`(模拟 AI 积累)。
2. 调 `update_operation_profile`,payload 只含 `{"relationshipType":"customer","lastCommitment":"下周回复"}`(不带 profileAttributes,同前端)。
3. reload contact,断言 `profile_attributes` **仍等于** `{"budget":"high","decision_role":"owner"}`(未被清空)。
旧 bug 下:$set {} → profile_attributes 变空 → 断言失败。真护栏,非 tautology。

**测试 2（对照）—— 带非空 profile_attributes 时正常写入:**
1. seed contact,profile_attributes 空。
2. 调 handler,payload 含 `profileAttributes: {"budget":"low"}`。
3. 断言 profile_attributes == `{"budget":"low"}`(合法写入,证明守卫不误伤真实写)。

**测试 3（可选,不回归)—— tags 与 profile_attributes 保留并存:**
seed profile_attributes 非空 + 调 handler 带 tags → 断言 tags 被更新 **且** profile_attributes 保留。确认「更新其它字段」与「保留画像」同时成立。

**基线影响**:新测试全 `#[ignore]`,不进 `cargo test --lib` 计数,lib≥350/0 与 4 PBT≥33/0 不受影响。

## 7. 范围边界

- **不做(YAGNI)**:不改前端、不动 gateway、不改 profile_attributes 的 `#[serde(default)]` 契约、不碰 handler 其它字段/校验逻辑、不加 profile_updated_at 条件化。只加非空守卫 + 测试可见性 + 回归测试。
- **过拟合红线**:测试锁「空 payload 不清空现值 / 非空正常写」两个真实不变量,不为过测试改业务逻辑。
- **禁词 lint**:不涉禁词。
- **多租户**:handler 现有 `workspace_id` 过滤(contacts.rs:844)不动,跨租户仍 NotFound。
