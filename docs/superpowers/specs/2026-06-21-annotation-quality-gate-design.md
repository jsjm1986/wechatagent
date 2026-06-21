# 标注质量门（annotation quality gate）设计

> 簇 B / 8 缺口补全的第 2 个子项目。为素材库 + 专属顾问名片引荐两个对称功能补 3 个"让人类标注更可靠 / 更可控"的缺口。簇 A（主动发送台账 agent_send_ledger）已完成。

**Date:** 2026-06-21
**Status:** 设计已获批，待落实现计划（writing-plans）
**Scope:** 仅簇 B（标注质量门）。簇 C（素材 CRUD 补全）、簇 D（结构化组织：知识库关联 / 标签）各自后续独立 spec。

## 1. 背景与动机

素材库（content-assets）与专属顾问名片引荐（referral-card）是两个**形态对称**的"AI 按触发条件主动发送物 + 人类标注审核"功能。当前三条"标注可靠性 / 可控性"链上有缺口：

- **缺口 2（客户级辅助模式 override 入口）**：后端 `assist_mode_active`（`src/agent/referral.rs:10`）逻辑齐全——客户级 `assist_mode_override`（`force_on`/`force_off`）> 账号级 `assist_mode_enabled` > 默认关。但 `ASSIST_MODE_OVERRIDE_ATTR` 全仓**零写入路径**（只有 `gateway.rs:2094`、`decision.rs:335` 两处读），运营完全碰不到这个客户级开关。
- **缺口 3（审核痕迹）**：`review_media_asset`（`media_assets.rs:174`）、`review_referral_card`（`referral_cards.rs:108`）approve/draft 后只 `$set review_status`，**无任何审计**——谁在何时把哪份素材改成 approved 查不到。与"AI 不自我核验、人类把关"红线的严肃度不匹配。
- **缺口 6（target_stages 归一校验）**：两个上传端点（`media_assets.rs:152`、`referral_cards.rs:66`）原样存运营手填的 `target_stages` 字符串，**不校验也不归一**。

### 缺口 6 的真实危险：不是拼写错，是 alias / canonical 漂移

`target_stages` 在当前业务里**不是发送硬门，是"进 prompt 候选清单"的软过滤**。`filter_sendable_candidates`（`media_send.rs:42`）/ `filter_referral_candidates`（`referral.rs:30`）：`target_stages` 为空 = 总命中；非空则要求精确包含当前 `customer_stage` 才进候选清单。进清单后发不发仍由 LLM 按 `send_trigger_hint` 判断。

所以"阶段填错"的后果是：该素材/名片**永远进不了候选、AI 永远选不到、静默不发、零报错**。

更隐蔽的是：`customer_stage` 写到 contact 时走 `validate_dimension_value`，会把 alias **归一成 canonical id** 再存（`dimension_registry.rs:122`，如 "需求挖掘" → "need_discovery"）；而两个上传端点对 `target_stages` **不归一**。于是：

- `contact.customer_stage = "need_discovery"`（canonical，LLM 写入已归一）
- `asset.target_stages = ["需求挖掘"]`（alias，运营手填未归一）
- 运行时 `s == cs` 精确比较 → 永不相等 → **永不命中**

即：运营哪怕填了字典里**合法存在**的 alias（不是拼错），只要不是 canonical id，素材照样永远沉默。单纯校验"在不在字典里"治不了这个——**必须归一化存储**才能与 contact 的 canonical 对齐。`validate_dimension_value` 的 `Accept(canonical)` 正好天然提供归一。

## 2. 已锁定的关键决策（brainstorming 产出）

| # | 决策点 | 选择 | 理由 |
|---|---|---|---|
| B1 | 代码组织 | **不新建模块，三缺口贴现有对称结构落地**，只抽 1 个跨两功能复用的 `normalize_target_stages` 纯壳函数 | override 是 contact 路由、校验是两个 assets 上传端点、审计是两个 review 端点——物理分属 5 个端点，强行抽模块反而打散内聚 |
| B2 | 缺口 6 校验严格度 | **归一 + 越界硬拒**：每个 target_stage 走 `validate_dimension_value(AdminWrite)`，Accept 存归一 canonical、Reject 整单 400、字典未配置 fail-soft 全 Accept | 归一是治 alias drift 的唯一解（不归一则合法 alias 也永不命中）；AdminWrite 越界硬拒对齐 relationship_suggestions approve 处置 |
| B3 | 缺口 3 范围 | **只补审核历史**（复用 `write_event_for_account` 写审计事件），不引 RBAC | 全仓无 role 字段（`AuthenticatedAdmin` 只有 user_id/username/current_workspace），多 admin 权限分级是独立后续工程 |
| B4 | 缺口 2 入口形态 | **单客户视图三态下拉**（跟随账号默认 / 强制开 / 强制关），复用 relationship_suggestions 的 `$set domain_attributes.X` + workspace 隔离写法 | 三态表达力最全（force_off = 账号开但该客户不引荐，二态开关表达不了） |
| B5 | 缺口 3 审计 fail-soft | 审计写失败只 `tracing::warn!`，不回滚 review、不返 Err | review 已生效 = 既成事实，审计是旁路痕迹，同送达后 DB 失败降级纪律 |

## 3. 架构总览

簇 B 不新建模块。涉及文件：

- **缺口 6**：`src/routes/media_assets.rs`（upload）+ `src/routes/referral_cards.rs`（create）各调归一；新增 `normalize_target_stages` 壳函数放 `src/agent/dimension_registry.rs`（校验逻辑同源，两端点共用一份避免漂移）。
- **缺口 3**：`src/routes/media_assets.rs`（review）+ `src/routes/referral_cards.rs`（review）各加一行 fail-soft 审计。
- **缺口 2**：在 `src/routes/contacts.rs` 新增 `PATCH /api/contacts/:id/assist-override` 端点（含 `is_valid_assist_mode` 闭集校验纯函数）+ 在 `src/routes/mod.rs` 挂载路由 + 前端 `frontend/src/features/user-ops/legacy.tsx` 单客户视图三态下拉。`:id` = contact ObjectId，对齐既有 `/contacts/:id/enable-agent`、`/contacts/:id/profile-note` 等 contact 写入端点主流（**不用 `:wxid`**——仅簇 A 新加的 send-history 用 wxid）。

## 4. 缺口 6：target_stages 归一校验

### 4.1 数据流

```
upload_media_asset / create_referral_card
  → 收到运营手填 target_stages: Vec<String>（逗号分隔已 trim）
  → 空数组 = "全阶段命中" → 直接存空，不进校验循环
  → 非空：normalize_target_stages(db, account_id_or_empty, raw_stages)
        对每项 validate_dimension_value("customer_stage", stage, scope, AdminWrite):
          Accept(canonical) → 收集 canonical（alias 已归一，治 drift）
          Reject(reason)    → 整个函数返 Err（BadRequest 列出哪个 stage 非法）
          DropSilently      → 空串项跳过（split 已过滤空，防御性）
  → Ok(归一后 Vec<String>) → 存库（与 contact.customer_stage 同 canonical 空间）
  → Err → 整单 400，运营当场改对
```

### 4.2 `normalize_target_stages` 壳函数

放 `src/agent/dimension_registry.rs`，`pub(crate) async`。是薄壳：循环 + match，无新判断逻辑，完全复用 `validate_dimension_value`。

```rust
/// 归一 + 校验 target_stages（缺口 6）：运营手填的客户阶段标注必须与 contact 的
/// canonical customer_stage 同空间，否则运行时 `s == cs` 永不命中、素材/名片静默不发。
/// 每项走 AdminWrite：Accept 收集 canonical（alias 归一）、Reject 整体报错、
/// 字典未配置 fail-soft 放行原值（KindUnconfigured → Accept）。空串项跳过。
pub(crate) async fn normalize_target_stages(
    db: &crate::db::Database,
    scope_account_id: &str,
    raw_stages: &[String],
) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(raw_stages.len());
    for stage in raw_stages {
        match validate_dimension_value(db, "customer_stage", stage, scope_account_id, WriteIntent::AdminWrite).await {
            DimValidation::Accept(canonical) => out.push(canonical),
            DimValidation::Reject(reason) => return Err(reason),
            DimValidation::DropSilently => {} // 空串，跳过
        }
    }
    Ok(out)
}
```

### 4.3 关键决策点

1. **scope_account_id 来源**：upload 有 `account_id: Option<String>`、create 同（`referral_cards.rs:26`）。两端 account_id 缺失时传**空串**——`validate_dimension_value` → taxonomy 查询走 global scope 回退（与现有 taxonomy account-scope 语义一致）。
2. **字典未配置 = fail-soft 全 Accept 原值**：`KindUnconfigured → Accept(trimmed)`（`dimension_registry.rs:128` 既有语义），不误拒。保证未配字典的新部署照常能上传。
3. **空 target_stages 不变**：空数组 = "全阶段命中"，直接存空，不进校验循环。
4. **归一只对 customer_stage 维度**（target_stages 语义即客户阶段）。
5. **AdminWrite intent**：运营是权威写入方，越界硬拒（对齐 `admin_relationship_suggestions.rs:138` approve 的 AdminWrite 处置）。
6. **不改 `validate_dimension_value` 本身**，只新增调用方。

## 5. 缺口 3：审核历史（只补审计，不引 RBAC）

### 5.1 数据流

```
review_media_asset / review_referral_card
  → $set review_status 成功（matched_count>0，既有逻辑）
  → 回查该 asset/card 拿 account_id（events 需要 account_id；update 入参只有 oid）
  → write_event_for_account(
        account_id, contact_wxid=None,
        kind = "media_asset.reviewed" / "referral_card.reviewed",
        status = payload.status,           // approved | draft
        summary = "管理员审核：{title/display_name} → {status}",
        details = { asset_id/card_id, review_note, reviewed_by: admin.username }
    )
  → 审计写失败：只 tracing::warn!，不回滚 review、不返 Err（既成事实纪律）
```

### 5.2 关键决策点

1. **reviewed_by 取 `admin.username`**：`AuthenticatedAdmin` 有 username（无 role），记录"哪个 admin 审的"，作为日后引入 RBAC 的数据基础。
2. **workspace_id 约束**：`write_event_for_account`（`gateway.rs:3963`）内部把 events 的 workspace_id 写死 `default_workspace_id`（`gateway.rs:3978` 既有）。本期**沿用，不改其签名**——单 workspace 部署语义正确；多 workspace 是"后续"，和"无角色系统"同属未来项。
3. **审计可读**：events 集合已有展示通道（任务日志 / 执行审计频道），新 kind 自动进流；本期**不新建审计展示 UI**（缺口 3 口径 = 只补审计历史，不是建审计页）。
4. **account_id 缺失**：content_assets / referral_cards 的 account_id 是 Option；缺失时审计 account_id 传空串（事件仍落库，只是 account 维度为空）。

## 6. 缺口 2：客户级 override 入口（单客户三态下拉）

后端 `assist_mode_active`（`referral.rs:10`）：`force_on`→true、`force_off`→false、其它/None→回落账号级 `assist_mode_enabled`。

### 6.1 数据流

```
前端单客户视图三态下拉：跟随账号默认 / 强制开 / 强制关
  → PATCH /api/contacts/:id/assist-override  { mode: "default"|"force_on"|"force_off" }
  → 后端校验 mode ∈ 闭集，否则 400
  → parse_object_id(:id) + find_contact_by_id(workspace, id)（跨 workspace / 不存在 → 404，复用既有 helper）
  → "default"  → $unset domain_attributes.assist_mode_override（清键回落账号级）
     "force_on" / "force_off" → $set domain_attributes.assist_mode_override = mode
  → filter 带 workspace_id 隔离（复用 contacts.rs 既有 contact-by-id 写法）
```

### 6.2 关键决策点

1. **三态映射**：`default` 走 `$unset`（不是写空串），让 `assist_mode_active` 的 `_ => account_enabled` 分支干净回落。
2. **闭集校验**：mode 只接受三个字面量，守 gateway 状态枚举闭集纪律。新增纯函数 `is_valid_assist_mode(&str) -> bool`。
3. **路由位置**：在 `contacts.rs` 新建 `PATCH /api/contacts/:id/assist-override`，`:id` = contact ObjectId（对齐 enable-agent / profile-note 等既有 contact 写入端点，复用 `parse_object_id` + `find_contact_by_id` + workspace 隔离）。
4. **前端**：`legacy.tsx` 单客户视图（已渲染 domainAttributes，1951 行附近）加三态下拉 + 当前值回显；读取走现有 contact 详情（`domainAttributes.assist_mode_override`）。设计语言遵现有 user-ops 页表单样式，不新造风格。

### 6.3 红线

客户级压过账号默认，**不改全自治红线**——override 只在辅助模式（账号已显式开）语境下决定单客户引荐与否，force_off 反而更保守（该客户不引荐）。守 no-human-takeover 禁词（命名用"辅助模式 / 引荐"，不出现禁词）。

## 7. 测试策略

遵项目铁律（纯函数确定性为主、不接受 skip 假绿、新增只 append 不删旧维度、不过拟合单条样本）：

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | `normalize_target_stages`：全 Accept 归一（alias→canonical）、含越界→Err、字典未配置→原值放行、空数组→空、account_id 空串→global scope | lib 单测（复用 dimension_registry 已有 cache / 字典测试设施） |
| 纯函数 | `is_valid_assist_mode`：default/force_on/force_off→true，其它→false | lib 单测 |
| 集成（CI / `#[ignore]`） | 缺口 6：upload 带 alias stage → 库里存 canonical；带越界 stage → 400；字典空 → 原值存 | testcontainers |
| 集成（CI / `#[ignore]`） | 缺口 3：review approved → events 落一条 kind=media_asset.reviewed；审计写失败不影响 review 成功 | testcontainers |
| 集成（CI / `#[ignore]`） | 缺口 2：PATCH force_on → domain_attributes.assist_mode_override=force_on；default → 键被 unset；跨 workspace contact id → 404（IDOR） | testcontainers |
| 前端 | `npm run build` 通过、无 TS 错误；三态下拉用现有 user-ops 表单样式，不新造风格 | 构建 + 人工对照 |

`assist_mode_active` 三态已有 `assist_mode_override_beats_account_flag`（`referral.rs:199`）覆盖——**不重复**，本簇只补 override 写入端点的 mode 闭集校验。

## 8. 边界 / 不做（YAGNI）

- **不引 RBAC / 角色系统**：缺口 3 只补审计历史。多 admin 权限分级是独立后续工程（全仓无 role 字段）。
- **不建审计展示页**：审计进现有 events 流（任务日志频道），不新做 UI。
- **不改 `write_event_for_account` 签名 / workspace 语义**：沿用其 default_workspace_id 现状。
- **不改 `validate_dimension_value` / `assist_mode_active`**：只新增调用方与写入端点。
- **不动账号级辅助开关**（`legacy.tsx:1081` 已有）：缺口 2 是叠加客户级粒度，不替换账号级。
- **不做 target_stages 运行时再校验**：归一在写入时一次完成，运行时 filter 不变（`s == cs` 精确比较）。

## 9. 红线守卫汇总

- **缺口 6**：归一只对 customer_stage、AdminWrite 越界硬拒、不改校验内核、字典未配置 fail-soft 不误拒。
- **缺口 3**：审计 fail-soft（不回滚 review、不返 Err）、沿用既成事实纪律。
- **缺口 2**：不改全自治红线、mode 闭集、workspace 隔离防 IDOR、守 no-human-takeover 禁词。

## 10. 与后续簇衔接

- 缺口 3 记录的 `reviewed_by`（admin.username）是未来 RBAC 落地时的数据基础（已知谁审的）。
- 簇 C（素材 CRUD 补全：edit/delete/disable）独立——但缺口 6 的 `normalize_target_stages` 在簇 C 的 edit 端点会**复用**（编辑 target_stages 同样要归一），本簇先把壳函数建好。
- 簇 D（结构化组织）独立。
