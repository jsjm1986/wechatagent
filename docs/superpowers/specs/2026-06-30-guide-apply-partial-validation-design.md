# guide apply 部分应用 + prompt 注入合法值 设计文档

> 状态:设计已与用户逐节确认,待用户复核 spec → writing-plans。
> 日期:2026-06-30。所有代码落点已经 6 路 workflow 100% 核对到行号,无猜测。

## 一、问题陈述

运营在用户运营驾驶舱用自然语言下指令(如"把这个客户标记成高意向重点跟进,补一句画像备注说明他关注价格"),系统走两步:

1. **preview**(`POST /api/user-operations/guide/preview`):调 LLM 生成一份可确认的 `suggestedChanges` 配置修改预览,落 `user_operation_guide_previews` 表(status=pending),**不碰业务库**。
2. **apply**(`POST /api/user-operations/guide/apply`):运营确认后,把 `suggestedChanges` 真映射到 contact / operating_memory / playbook / domain_config。

**实测 bug(server 117,2026-06-30 biz-test)**:LLM 一次产出 9 个字段的 suggestedChanges,其中 `operationState="active"` 是状态机字典里不存在的越界值。apply 时该字段触发 `check_state_transition` 硬拒,整个请求返回 400,**前 8 个合法字段(humanProfileNote/tags/customerStage/intentLevel/...)全部未落库**。前端走 catch 显示错误条、preview 不清空,运营再点还是同样错误 → 功能卡死。

## 二、根因(两个独立缺陷叠加)

### 缺陷 A:prompt 不告知合法值(源头)
`build_guide_preview_prompt`(`src/routes/shared.rs:811`,同步纯函数)的 JSON 模板(:841-870)里三个枚举字段只标注"可选":
- `"customerStage": "可选客户阶段"`
- `"intentLevel": "可选意向等级"`
- `"operationState": "可选运营状态"`

prompt 既没注入该 contact 所在行业状态机的合法态 key(`new_contact/relationship_building/need_discovery/...`),也没注入 customer_stage / intent_level 字典的 canonical 值。LLM 只能凭常识猜 → 产出 `"active"` 这类通用词,几乎必然越界。

### 缺陷 B:apply 单字段越界连坐全部合法字段(放大)
`apply_contact_changes`(`src/routes/shared.rs:572-681`)所有字段共用一个 `set_doc`,最后一次 `update_one` 原子写。三个校验点任一越界即提前返回 400:

| 字段 | 行号 | 校验机制 | 越界后果 |
|---|---|---|---|
| `customerStage` | :589-598 | `apply_admin_dim_validation(validate_dimension_value(..., AdminWrite))?` → `Reject` → `?` | 整体 400 |
| `intentLevel`(stage 分支内) | :600-609 | 同上 | 整体 400 |
| `intentLevel`(stage 缺席 else-if) | :622-631 | 同上 | 整体 400 |
| `operationState` | :653-661 | `check_state_transition` 返回 `Some(reason)` → `return Err` | 整体 400 |

`set_doc` 是所有字段共享的,任一 `?`/`return Err` 在写库前触发,**已 insert 的合法字段(humanProfileNote :578 / tags :581 / followUpPolicy :636 等)全部丢弃**。

**这是真 bug,不是设计权衡**:校验闸 reject 越界值本身正确(防脏值、防 planner/policy 口径漂移,是修复项 F/M1 刻意为之),但"因一个 LLM 拍脑袋的越界字段丢弃所有运营认可的合法字段"不是任何人想要的语义。而 LLM 产越界值又几乎必然(prompt 没给字典)。两者叠加 → guide apply 真实环境高频整体失败。

## 三、设计原则与边界

- **LLM 产的值不该连坐合法字段;人手填的值越界该当场拒**。这是核心语义边界:
  - **guide 路径**(`apply_contact_changes`,吃 LLM 生成的 `preview.suggested_changes`):越界字段跳过 + 记录,合法字段照落。
  - **手动表单路径**(`contacts.rs::update_operation_profile`,运营逐字段手填)+ **审批路径**(`admin_relationship_suggestions::approve_relationship_suggestion_inner`):AdminWrite 越界**保持硬拒 400**,人是权威,当场报错正确。**本次改动绝不触碰这两条路径**。AI 建档路径(`management.rs` 后台管理 Agent)用的是 MachineWrite,越界本就 DropSilently(不报错),同样不受影响。
- **不过拟合**:不为某条对话/某个越界值打补丁。注入合法值是普适机制(任意行业的字典/状态机都适用),跳过逻辑对任意越界字段一致。
- **agent-first**:前端提示文案由 `skippedFields` 动态拼接,不硬编码 "operationState" 等字段名。
- **新增测试只增量 append**,不删改旧维度。
- **复用现成机制**:prompt 注入照搬 `domain_profile.rs:1210-1230` 范式;taxonomy 取值照搬 `operation_view.rs:66-85` 范式。

## 四、改动 1:apply_contact_changes 三字段越界跳过 + 记录(治缺陷 B)

文件:`src/routes/shared.rs`

### 4.1 新增 SkippedField 结构体
在 shared.rs 顶部(或 apply_contact_changes 上方)新增:
```rust
/// guide apply 中被跳过的越界字段(LLM 产出但不在字典/状态机内)。
/// 仅 guide 路径产出——手动表单路径(contacts.rs)仍硬拒不收集。
#[derive(Debug, Clone)]
pub(super) struct SkippedField {
    pub field: String,   // camelCase 字段名,如 "operationState"
    pub reason: String,  // 人类可读原因,如 "状态机里无此态: active"
}
```

### 4.2 签名变更
```rust
// 之前:
pub(super) async fn apply_contact_changes(state, contact, changes) -> AppResult<()>
// 之后:
pub(super) async fn apply_contact_changes(state, contact, changes) -> AppResult<Vec<SkippedField>>
```
函数开头 `let mut skipped: Vec<SkippedField> = Vec::new();`,末尾 `Ok(skipped)`(替换两处 `Ok(())`,含 :672 set_doc 空判 early return)。

### 4.3 三个校验点改"就地捕获 Reject/非法迁移 → 记 skipped → 跳过",不改共用 helper
**核实确认(全仓 grep 落实调用方)**:`apply_admin_dim_validation`(shared.rs:105)是纯映射 helper,被**三处**复用——guide 路径(本函数 :589/:600/:622)、手动表单(`contacts.rs::update_operation_profile` :771/:804/:826,AdminWrite)、AI 建档(`management.rs` 后台管理 Agent 纳入 :1403/:1416/:1553/:1590,**MachineWrite → 越界恒 DropSilently 不 Reject**)。**绝不能改它**(改它会同时波及手动表单与 AI 建档两条无关路径)。审批路径(`admin_relationship_suggestions.rs:150`)**不走此 helper、直接 match `DimValidation`**,与本改动无交集。正确切口是在 apply_contact_changes 调用点**绕过 helper、直接 match 原始 `DimValidation`**——因为 helper 已把 `Reject` 转成 `Err`,经 helper 就拿不到"跳过"语义了。

- **customerStage**(:589):不再调 `apply_admin_dim_validation(...)?`,改成直接 match `validate_dimension_value(&state.db, "customer_stage", &value, &contact.account_id, AdminWrite).await`:
  ```rust
  use crate::agent::dimension_registry::DimValidation::*;
  let validated_stage = match validate_dimension_value(...).await {
      Accept(s) => Some(s),
      DropSilently => None,
      Reject(r) => { skipped.push(SkippedField{field:"customerStage".into(), reason:r}); None }
  };
  ```
  validated_stage 越界时取 None,与现有 :612 `if let Some` 门控天然衔接。
- **intentLevel**(两条路径 :600 和 :622):同样直接 match `DimValidation`,`Reject` 记 skipped + 取 None。
- **operationState**(:653-661):`check_state_transition` 返回 `Some(reason)` 时,不再 `return Err`,改为 `skipped.push(SkippedField{field:"operationState".into(), reason}); ` 然后**跳过** :662-663 的两行 insert(把 :662-663 包进 `if check_state_transition(...).is_none() { ... }` 的 else,或 None 分支才写)。

### 4.4 守住 insert_domain_stage_fields 空调用不变量
**核实确认的关键风险**:`insert_domain_stage_fields`(:81-96)无条件写 `domain_attributes_updated_at`。若 stage 和 intent 都越界被跳过却仍调用它,会凭空写时间戳 → 破坏 :671-673 的 `set_doc.is_empty()` 空判 → 产生只更新时间戳的无意义写库。

**改法**:复用现有 :612/:617 三分支门控——`if let Some(stage)` 调用;`else if intent.is_some()` 调用;**两者皆 None(都被跳过)→ 不调用**(走 implicit 第三分支)。改 customerStage/intentLevel 为"越界取 None"后,这个门控天然守住不变量,`insert_domain_stage_fields` 本身不用改。

## 五、改动 2:apply 收集 skipped 并回流(治可观测)

文件:`src/routes/guides.rs:125-213`

**核实确认**:apply_contact_changes 全仓只有 guides.rs:158 一处调用,零测试调用。改签名只波及这一行。

- :158 绑定结果:`let skipped = apply_contact_changes(&state, &contact, &preview.suggested_changes).await?;`
  - 其余三个 apply_*_changes(memory/playbook/domain)签名不变(它们不涉枚举校验,核实确认不会硬失败),仍 `.await?`。
- 响应体(:206-212)新增两个字段:
  ```rust
  "appliedFields": <从 suggested_changes keys 减去 skipped 的字段名列表>,
  "skippedFields": skipped.iter().map(|s| json!({"field": s.field, "reason": s.reason})).collect::<Vec<_>>(),
  ```
- 审计事件(:171-196)的 details doc 补记 `"skippedFields"`(post-hoc 可查谁的哪个字段被跳过)。

## 六、改动 3:prompt 注入合法值(治缺陷 A)

文件:`src/routes/guides.rs`(handler 查数据)+ `src/routes/shared.rs::build_guide_preview_prompt`(注入文本)

### 6.1 handler 侧查合法值(照搬 operation_view.rs:66-85 范式)
在 `preview_user_operation_guide`(guides.rs:41-51 加载段)新增:
```rust
// 状态机合法态 key(operationState 合法值)
let domain_config = agent::load_user_operation_domain_config_for_contact(
    &state, &contact.workspace_id, &contact.wxid).await?;
let legal_states: Vec<String> = agent::operation_states(domain_config.as_ref())
    .iter().filter_map(|d| d.get_str("key").ok().map(String::from)).collect();
// 字典合法值(customerStage / intentLevel)
let cache = agent::taxonomy::global_taxonomy_cache();
cache.find_or_load(&state.db).await;  // 冷/过期缓存返回空,必须先 load(幂等自愈)
let stage_pairs = agent::taxonomy::dimension_values_with_labels(
    "customer_stage", &admin.current_workspace, cache.as_ref());
let intent_pairs = agent::taxonomy::dimension_values_with_labels(
    "intent_level", &admin.current_workspace, cache.as_ref());
```
**核实确认**:`load_user_operation_domain_config_for_contact` 已在 shared.rs:647 被 guide 路径用过(同范式);`operation_states`(guards.rs:105,`pub(crate)`)**未再导出**,需在 `src/agent/mod.rs` 加一行 `pub(crate) use guards::operation_states;`(与 :86 `initial_operation_state_key` 同模式)。taxonomy 三函数(`global_taxonomy_cache`/`find_or_load`/`dimension_values_with_labels`)均 `pub(crate)`,routes 可达。kind 字符串确认是 `"customer_stage"`/`"intent_level"`(m006 seed,global scope)。

### 6.2 build_guide_preview_prompt 加切片入参(保持同步纯函数)
**核实确认**:此函数是同步纯函数,只 `format!` 拼字符串。推荐 handler 查好数据按 `&[String]`/`&[(String,String)]` 切片传入,保持纯函数可测形态(与现有 `health: &Value`/`playbook: Option<&_>` 风格一致),不改 async、不传 state。

新增 3 个入参:`legal_states: &[String]`、`stage_values: &[(String,String)]`、`intent_values: &[(String,String)]`。

在 :904 `format!` 的"当前健康度"段后追加三段提示文本,照搬 `domain_profile.rs:1210-1230` 范式(含"无字典时暂无受控取值"措辞):
```
operationState 合法值(只能从中选,留空则不改):new_contact / relationship_building / ...
customerStage 合法值:first_contact(初次接触) / qualified(已确认意向) / ...
intentLevel 合法值:low(低) / mid(中) / high(高) / ...
```
列表为空时输出"暂无受控取值,留空此字段"。

## 七、改动 4:前端 skipped 回流提示

**核实确认**:apply 响应当前在 store 用内联泛型、无命名类型;user-ops 频道根组件**没挂 ToastProvider**(只挂 ConfirmProvider),`useToast` 会抛错;store action 在 React 之外无法直接调 `useToast`。

### 7.1 类型(`frontend/src/types/index.ts:451-473` 邻近)
```ts
export type GuideSkippedField = { field: string; reason: string };
export type UserOperationGuideApplyResult = {
  contact: Contact; operatingMemory: OperatingMemory; health: OperationHealth;
  appliedFields: string[]; skippedFields: GuideSkippedField[];
};
```

### 7.2 store(`frontend/src/stores/userOpsStore.ts:709-737` applyGuidePreview)
- 内联泛型换成 `UserOperationGuideApplyResult`。
- action 改为返回 `Promise<UserOperationGuideApplyResult | null>`(把结果交给组件侧拼 toast,因 store 不能调 useToast)。成功仍清空 guidePreview、刷新 contacts。

### 7.3 频道挂 ToastProvider + 回调拼提示(`frontend/src/features/user-ops/index.tsx`)
- 根组件补挂 `<ToastProvider>`(与现有 ConfirmProvider 并列)。
- apply 回调包一层:`const res = await applyGuidePreview(); if (res?.skippedFields.length) toast.success(动态文案)`。
- **文案动态拼接,不硬编码字段名**:`已应用${res.appliedFields.length}项${res.skippedFields.length ? `,跳过 ${res.skippedFields.map(s=>s.field).join('、')}` : ''}`。

展示组件 `legacy.tsx:467-486` preview 区块不动(apply 成功即清空 guidePreview,skipped 走 toast 一次性反馈最契合)。

## 八、测试(只增量 append)

### 8.1 集成单测(`tests/`,#[ignore] 需 Docker testcontainers,直调 apply_contact_changes)
1. **`apply_skips_invalid_keeps_valid`**:changes = `{humanProfileNote:"关注价格", operationState:"active"(越界), customerStage:"瞎填"(越界)}`,seed 一个 DEFAULT 状态机 contact → 调 apply_contact_changes → 断言:contact.human_profile_note 真落库 == "关注价格";返回 skipped 含 operationState 与 customerStage 两项(field 名正确)。
2. **`apply_all_invalid_no_empty_write`**:三枚举字段全越界、无其它合法字段 → 断言 set_doc 空判生效(contact 完全不变,不产生只更新时间戳的空写)、返回 skipped 三项。
3. **`apply_intent_valid_stage_skipped`**:customerStage 越界 + intentLevel 合法 → 断言 intent 落库(走 :617-619 分支)、stage 进 skipped、不破坏 insert_domain_stage_fields 门控。
4. **`apply_legal_values_all_persist`**(正向回归):三字段全给合法值 → 断言全部落库、skipped 为空(证明改动不影响 happy path)。

### 8.2 prompt 纯函数单测(`src/routes/shared.rs` mod tests)
5. **`guide_prompt_injects_legal_values`**:传入 legal_states=["new_contact","need_discovery"]、stage_values、intent_values → 断言 build_guide_preview_prompt 输出含状态机 key 字符串 + 字典中文标签 + "合法值"字样;传空切片 → 含"暂无受控取值"。

### 8.3 biz-test(`scripts/biz-test/batch_c_guide.py` 已就绪,改断言)
6. 改第一轮断言:apply 不再因 LLM 越界整体 400;若有越界字段,验响应 skippedFields 回流 + 合法字段(updated_at 刷新/human_profile_note 落库)真生效。保留 not-pending 幂等、preview 不碰业务库两条铁证。

### 8.4 基线
- `cargo test --lib` ≥ 350 passed/0 failed(新增纯函数测试只增不减)。
- 4 PBT 累计 ≥ 33/0 不回归。
- `cargo check --tests` 0 error(复刻 CI step2,防集成测试编译挂)。

## 九、边界与错误处理

- **仍保留的真错误**(非字段越界,继续 400/404):DB 写失败、preview not-pending(guides.rs:140)、contact/account 不存在、序列化错(:582 to_bson)。
- **domain_config=None**(未配状态机):`check_state_transition` fail-open 返回 None → operationState 照写,行为不变(与改造前逐字等价)。
- **字典冷启动空**:`find_or_load` 幂等自愈;若仍空(字典未 seed),注入空列表 → prompt 显示"暂无受控取值,留空此字段",不阻断 preview。
- **relationship_type**:guide 路径根本不写它(apply_contact_changes 无此分支),无影响。它在手动表单/审批路径恒 Reject 的行为不动。

## 十、非目标(YAGNI)

- 不改 `contacts.rs::update_operation_profile` 手动表单的 AdminWrite 硬拒(人是权威)。
- 不改 `admin_relationship_suggestions` 审批的硬拒。
- 不改 `apply_admin_dim_validation` 共用 helper(它同时服务手动表单 contacts.rs 与 AI 建档 management.rs,改它会误伤这两条)。
- 不给 followUpPolicy/operationPolicy 等自由文本字段加约束(它们不过校验闸、不会 400)。
- 不做 preview 阶段预演校验(运营在 apply 后看 skipped 提示即可,preview 预演是更大改动,YAGNI)。

## 十一、改动文件清单

| 文件 | 改动 |
|---|---|
| `src/routes/shared.rs` | 新增 SkippedField;apply_contact_changes 签名+三校验点改跳过;build_guide_preview_prompt 加 3 切片入参+注入文本;新增 prompt 单测 |
| `src/routes/guides.rs` | preview handler 查合法值;apply handler 绑定 skipped+响应/审计回流 |
| `src/agent/mod.rs` | 加 `pub(crate) use guards::operation_states;` |
| `frontend/src/types/index.ts` | 新增 GuideSkippedField / UserOperationGuideApplyResult |
| `frontend/src/stores/userOpsStore.ts` | applyGuidePreview 用命名类型+返回结果 |
| `frontend/src/features/user-ops/index.tsx` | 挂 ToastProvider + 回调拼 skipped 提示 |
| `tests/` | 新增 apply 部分应用集成测试(4 个) |
| `scripts/biz-test/batch_c_guide.py` | 改断言验 skippedFields 回流 |
