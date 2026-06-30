# wiki 审查两个 High 缺陷修复设计（#1 domain_schemas serde 错配 + #2 prompt 红线闸缺口）

> 来源：2026-06-30 wiki 子系统 100% 全覆盖逐行审查。本 spec 只收口审查清单里的两个 High：
> - **#1**：`src/routes/domain_schemas.rs` serde 字段名错配，导致动态字段校验（required/enum）静默失效（R2 连带）。
> - **#2**：`src/routes/prompt_templates.rs` 的 `create` / `publish` 端点绕过 prompt 红线锚校验（update 已有三道闸）。
>
> 其余审查缺陷（#3 ingest 无去重、#4 SSRF、#5 R5.4 观测降级、#6 APP_ENV 守卫、#7 patch 绕 D2 门、chunk_locks 两个 Med 等）不在本 spec 范围，另行排期。

## 背景与目标

**目标**：修复两个 High 缺陷，使其行为与同类已正确实现的路径对齐，并用真实 Mongo round-trip 集成测试锁住回归——这两个 bug 之所以能瞒过现有测试套件，正因为它们是 IO 层缺陷，纯函数测试覆盖不到。

**关键约束**：
- 不为通过测试而改业务逻辑/阈值/断言（过拟合红线）。
- 不碰 CLAUDE.md 的"无人工接管"红线词表；新增代码不内联禁用词字面量（`prompt_guard.rs` 在 CI lint 扫描区，测试构造禁用词须用字符拼接）。
- 全项目 Mongo 字段命名约定为 snake_case（`db/indexes.rs` 所有集合一致）；对外 JSON 响应为 camelCase（serde rename / 手写 json! 投影）。

---

## 缺陷 #1：domain_schemas serde 字段名错配

### 根因（已逐行核对）

- `DomainSchema` 模型（`src/models.rs:1706-1723`）**无** `#[serde(rename_all = "camelCase")]`，故 BSON 序列化为纯 snake_case：`workspace_id` / `is_active` / `updated_at` / `created_at` / `schema_id` / `version`。
- `create_domain_schema`（`domain_schemas.rs:229`）用 `insert_one(&cfg)` 写入，落库即为 snake_case。
- 但路由层的查询 filter / `$set` / sort 文档里，有三个键用了 camelCase：
  - `workspaceId`（:176, :252, :277, :286, :308, :324, :345, :358, :371, :406, :522）
  - `isActive`（:178, :359, :361, :374）
  - `updatedAt`（:266, :361, :374）
- 注意：`schema_id` / `version` 在路由层用的是 snake_case（正确）；错配的仅这三个键。

### 后果链

- `list_domain_schemas`：filter `{ workspaceId }` 永远 miss → 列表恒空。
- `update` / `delete` / `activate`：filter `{ workspaceId, schema_id }` 永远 miss → 恒 404 / no-op。
- `activate` 的 `$set { isActive: true }`：写进一个 camelCase 幽灵字段，真正的 snake_case `is_active` 始终保持 `false`。
- `load_active_domain_schema`（:515-527）：filter `{ isActive: true }` 永远 miss → 恒返 `None` → 写侧 chunk 入库时 `enforce_domain_attributes`（required 缺失 reject / enum 越界 reject / alias rewrite）**从不执行**。这是 R2 连带：行业动态字段约束静默失效。
- 路由确实挂载在 `require_session` middleware 之后（`routes/mod.rs:967-976`）→ 这是线上活跃 bug，不是死代码。
- 纯函数测试（`domain_schemas.rs:600-787` 的 `validate_schema_payload` / `enforce_domain_attributes` 测试）全过，因为它们不经过 IO 层，掩盖了字段名错配。

### 修复方案（用户裁定：查询改回 snake_case；不迁移数据）

把 `domain_schemas.rs` 中所有出现在 Mongo 查询 filter / `$set` / sort 文档里的三个 camelCase 键改为 snake_case：

- `"workspaceId"` → `"workspace_id"`
- `"isActive"` → `"is_active"`
- `"updatedAt"` → `"updated_at"`

不改动：
- `DomainSchema` 模型（保持无 rename_all，纯 snake_case 序列化，与全项目约定一致）。
- `DomainSchemaView`（对外 JSON 响应 struct，`#[serde(rename_all="camelCase")]`，是给前端的，正确）。
- `ListQuery` / `UpsertRequest` / `DomainFieldPayload`（请求体 payload，camelCase，是前端传入的，正确）。
- `next_version_for` 用的 `schema_id` / `version`（本就 snake_case，正确）。

### 残留（用户已知情接受）

线上若存在用旧（错配）代码 activate 过的 schema，其真正的 `is_active` 字段是 `false`（旧 activate 只写进了 camelCase 幽灵字段）。修复上线后，该 schema 需在 UI 手动重新激活一次才会真正生效。新建的 schema 不受影响（create 走 insert_one 落的就是 snake_case，修复后 list/activate 立即正常）。不做数据迁移。

### 测试（新增）

`tests/domain_schema_persistence_e2e.rs`（`#[ignore]` + testcontainers MongoDB，本地编译 / CI `--ignored` 跑）：

1. **create → list round-trip**：调 `create_domain_schema` 写入一条，再调 `list_domain_schemas`（同 workspace），断言能查到刚写入的那条（修复前恒空）。
2. **activate → load_active round-trip**：create 一条 → activate → 调 `load_active_domain_schema(db, workspace)`，断言返回 `Some` 且 `is_active == true`、schema_id 匹配（修复前恒 `None`）。
3. **enforce 真生效**：在有 active schema（含一个 `required=true` 字段）的前提下，对缺该字段的 `domain_attributes` 调 `enforce_domain_attributes`，断言 `BadRequest`（验证 load 链路打通后约束真的执行；enforce 纯函数本身已有单测，这里验证的是"active schema 真能被加载到"这一 IO 链路）。
4. **activate 互斥**：create 两条 → activate A → activate B → 断言 `load_active_domain_schema` 返回 B，且 A 的 `is_active` 被置回 `false`（验证 `update_many { is_active: true } → false` 这条 $set 真命中）。

> **测试调用方式（单一路径，避免实施者犹豫）**：domain_schemas 的 handler 是 `pub(super)`（模块私有），集成测试（独立 crate）无法直接调用；`load_active_domain_schema` 是 `pub`（可调）。因此测试走"模拟 handler 的 DB 写入 + 调 pub 函数读取"来验证 round-trip：
> - 写入：测试构造 `DomainSchema` struct（与 `create_domain_schema` 构造的等价），用 `state.db.domain_schemas().insert_one(&cfg)` 写入；activate 用与 `activate_domain_schema` 等价的 `update_many`/`update_one`（修复后的 snake_case 字段名）。
> - 读取：调 `wechatagent::routes::domain_schemas::load_active_domain_schema(&db, ws)`（pub）+ `wechatagent::routes::domain_schemas::enforce_domain_attributes(&schema, &attrs)`（pub）。
> - 核心证明点：写入用的字段名（insert_one 序列化出的 snake_case）与读取 filter（`load_active_domain_schema` 内 `{ "is_active": true }`，**本次修复后**）一致 → 能查到。修复前 `load_active_domain_schema` 用的是 `{ "isActive": true }`，这条测试会 fail（红→绿验证）。
> - 注意：这验证的是"写入字段名 ↔ 读取字段名一致"这一根因。handler 内部的 filter 字段名（list/update/delete/activate）与 `load_active_domain_schema` 同源同改，改一处对则全对；测试锁住 load 链路即锁住根因。为额外覆盖 handler 自身的 filter，实施者可酌情把 `load_active_domain_schema` 之外的 list 查询也用等价 DB 操作验证一遍（可选，非必须）。
>
> **测试覆盖边界（诚实声明）**：5 个 handler（list/create/update/delete/activate）均为 `pub(super)`，独立 crate 的集成测试**够不到**，无法直接断言它们的 filter 字段名。集成测试只能锁住 `pub` 的 `load_active_domain_schema` 这条读链路。handler 内 11 处字段名的正确性靠"全部同源改为 snake_case + 全分支终审逐处核对"人工保证——这是本修复的已知测试盲区，终审任务须显式逐处核验这 11 处。字段名采用裸字面量直接改（与项目其它路由的 `doc!` 裸字面量风格一致），不抽模块常量。

---

## 缺陷 #2：prompt_templates create/publish 绕过红线闸

### 根因（已逐行核对）

`prompt_guard.rs`（即 `management_prompt_edit` 模块）提供三道闸：
- `validate_prompt_edit(key, content)`：字面双闸 = 禁用词闸（`passes_forbidden_words`）+ 锚完整性闸（强约束层 key 的业务锚 + 红线锚必须逐字仍在，CRLF 归一后比对）。纯函数，fail-closed。
- `review_prompt_edit(state, ws, key, old, new)`：LLM 语义第三闸，审 diff 增量，三态 = Pass / Reject / NeedsHumanConfirm（LLM 不可用时降级人确认，不 fail-open 放水）。

`prompt_templates.rs` 三个写端点的现状：
- `update_prompt_template`（:141）：✅ 调了 `validate_prompt_edit`（:150）+ `review_prompt_edit`（:169），三态处理完整，支持 `force` 跳过 LLM 闸。
- `create_prompt_template`（:89）：❌ 只调 `validate_prompt_template_input`（:94，仅查字段非空），零红线校验。
- `publish_prompt_template`（:226）：❌ 只把 status 改 active（:261）+ 删同 key 旧版本（:247），零红线校验。

### 绕过链

管理员 `create` 一份删掉红线锚 / 含变相真人转介措辞的 draft（create 不拦）→ `publish` 激活它（publish 不拦）→ 触碰红线的 prompt 上线生效。完全绕开 update 的三道闸。

### 修复方案（用户裁定：create+publish 补字面双闸，publish 再加 LLM 三闸）

**`create_prompt_template`**：在 `validate_prompt_template_input(&payload)?` 之后、构造 `PromptTemplate` 之前，插入字面双闸：

```rust
crate::routes::management_prompt_edit::validate_prompt_edit(&payload.prompt_key, &payload.content)
    .map_err(AppError::BadRequest)?;
```

- 与 `update:150` 完全一致。
- create 是写入一份全新内容，对整篇过字面双闸（禁词 + 锚完整性）语义正确。
- create **不**加 LLM 第三闸：它无 old 基线可做 diff（全文都是"增量"会让语义审查退化为审整篇，且成本高）；且该 draft 最终必须经 publish 才生效，publish 关口会兜 LLM 闸（见下），不漏。

**`publish_prompt_template`**：在加载 template（:232）之后、改 active（:256）之前，对 `template.content` 补**字面双闸 + LLM 第三闸**：

1. 字面双闸：`validate_prompt_edit(&template.prompt_key, &template.content)`，命中即 `BadRequest`，不激活。
2. LLM 第三闸：`review_prompt_edit(state, ws, key, old, &template.content)`：
   - `old` 基线 = 当前 `current_version == true`（回退：`status == "active"`）那条的 content；查不到则 `old = ""`（全文当增量审，与 update:156-168 加载 old 的逻辑同构）。
   - 三态处理与 update:178-195 一致：
     - `Pass` → 继续激活。
     - `Reject(reason)` → `BadRequest("红线语义审查拒绝：{reason}（确认无误可带 force 覆盖）")`，不激活。
     - `NeedsHumanConfirm { diff, reason }` → 返回 `Ok(Json({ status: "needs_human_confirm", reason, diff }))`，不激活；前端勾选后带 `force=true` 重提。

**publish 端点签名变更**：当前 `publish_prompt_template` 是无 body 的 POST。改为接收一个可选 JSON body 携带 `force`：

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublishRequest {
    #[serde(default)]
    force: Option<bool>,
}
```

`force == Some(true)` 时跳过 LLM 第三闸（但**仍过字面双闸**——禁词 / 锚完整性是确定性硬闸，force 不可绕；与 update 中 force 只跳 LLM 闸、字面双闸恒过的语义一致）。用 axum 的可选 body 提取（无 body 时落 `PublishRequest::default()`，force=None=false）。

### 前端配合

`publish` 调用点（前端 prompt 模板管理页）需处理 `needs_human_confirm` 响应：弹框显示 `diff` + `reason` → 管理员确认 → 带 `force: true` 重新 POST publish。照 update 已有的 needs_human_confirm 前端处理模式实现（同形响应）。

> 若现有前端 publish 调用未传 body，新增可选 body 向后兼容（无 body → force=None）。前端改动作为独立任务。

### 测试（新增）

`tests/prompt_template_redline_gate_e2e.rs`（`#[ignore]` + testcontainers，仿 `tests/evolution_release_redline.rs` 的 mock-LLM 模式；禁用词用字符拼接构造）：

**create 路径**：
1. create 含禁用词的 content → 被拒（`BadRequest`），collection 中无该 prompt_key 新行。
2. create 强约束层 key（如 `user.reply.policy`）但删掉红线锚的 content → 被拒。
3. create 干净 content（自由改层 key，或强约束层 key 且保留全部锚 + 无禁词）→ 成功入库（status=draft）。

**publish 路径**：
4. 先直插一条删了红线锚的 draft（raw insert 绕过 create 闸模拟历史脏数据）→ publish 它 → 被拒，该行 status 仍为 draft（未变 active）。
5. publish 一条干净 draft + mock LLM 判 `violation=false` → 成功，status=active，旧版本被删。
6. publish 同样干净 draft + mock LLM 判 `violation=true` → 被拒，status 不变。
7. publish + mock LLM 不可用（不排队响应 → review_prompt_edit 返回 NeedsHumanConfirm）→ 返回 `{status:"needs_human_confirm"}`，status 不变；再带 `force=true` 重提 → 成功 active。

### 范围边界（YAGNI）

- 不动 `update_prompt_template`（三道闸已正确）。
- 不动 `reset_system_prompt_pack`（CLAUDE.md 明列的显式销毁性维护动作，靠不接入管理 agent 工具来约束，非自然语言编辑入口）。
- 不动 evolution release 路径（`src/evolution/release.rs` 已有三道闸 + `tests/evolution_release_redline.rs` 覆盖）。

---

## 验证

1. 后端编译 + dead-code 门：`RUSTFLAGS="-D warnings" cargo check --tests`（EXIT=0，复刻 CI baseline step2）。
2. 单元基线：`cargo test --lib`（≥350/0；touch `src/lib.rs` 强制 relink，规避共享 target stale 二进制）。
3. 新增集成测试本地编译过（`cargo test --test domain_schema_persistence_e2e --no-run`、`cargo test --test prompt_template_redline_gate_e2e --no-run`）；实际跑 `--ignored` 留 CI（本地磁盘 / Docker 受限）。
4. 双 lint：`bash scripts/check-no-human-takeover.sh`（0 violations）；命名红线 `git diff origin/main...HEAD` 新增行 0 命中禁词。
5. 前端（若改 publish 调用点）：`cd frontend && npx tsc --noEmit`（0 error）+ 相关 vitest `--pool=forks`。

## 执行方式

Subagent-Driven。任务切分见实施计划（writing-plans 阶段产出）。预计：后端 #1（1 任务）+ 后端 #2（1 任务）+ 前端 publish 配合（1 任务）+ 集成测试（并入各自后端任务或独立 1 任务）+ 全分支终审 + PR + CI 绿后合并。
