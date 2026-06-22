# 标注质量门（annotation quality gate）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为素材库 + 专属顾问名片引荐两个对称功能补 3 个"让人类标注更可靠 / 更可控"的缺口：target_stages 归一校验（缺口 6）、审核历史审计（缺口 3）、客户级辅助模式 override 入口（缺口 2）。

**Architecture:** 不新建模块，三缺口贴现有对称结构落地。只抽 1 个跨两功能复用的 `normalize_target_stages` 纯壳函数放 `dimension_registry.rs`。缺口 6 在两个上传端点接线归一；缺口 3 在两个 review 端点加 fail-soft 审计；缺口 2 新建一个 contact 级 PUT 端点 + 前端单客户三态下拉。

**Tech Stack:** Rust (Axum) 后端 + React 19 + TypeScript + Vite 前端 + MongoDB。设计文档：`docs/superpowers/specs/2026-06-21-annotation-quality-gate-design.md`。

## Global Constraints

- **既成事实纪律**：MCP / 业务动作成功后，旁路落库（审计 events）失败绝不返 Err——只 `tracing::warn!`。返 Err 会触发上游 retry / 误判失败。
- **no-human-takeover 禁词**：`src/agent/`、`src/routes/`、`frontend/src/` 新增行禁止出现 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。命名用 AI 内部口径中性词（"辅助模式 / 引荐 / 审核"）。CI lint `scripts/check-no-human-takeover.{sh,ps1}` 扫新增行。
- **workspace_id scope 防 IDOR**：所有 contact / asset / card 查询与更新的 filter 必带 `workspace_id`（取 `admin.current_workspace`）。
- **AI 不自我核验红线**：素材/名片上传默认 `review_status="draft"`，必须人类 approve；本簇不动这条。
- **测试基线不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。本地只跑 `cargo test --lib` + 单 PBT 文件（磁盘紧，整套集成测试交 CI）。新增测试只 append，不删改旧维度。
- **不改 `validate_dimension_value` / `assist_mode_active` / `write_event_for_account` 内核**：只新增调用方与端点。
- **归一只对 `customer_stage` 维度**，intent = `WriteIntent::AdminWrite`（运营是权威写入方，越界硬拒）。
- **字典未配置 fail-soft**：`validate_dimension_value` 在 `customer_stage` 字典整个未配置时回 `Accept(原值)`（`KindUnconfigured` 语义），不误拒——新部署照常能上传。
- **回复语言**：与用户对话用中文；代码 / 标识符 / commit 沿用既有约定。

---

### Task 1: `normalize_target_stages` 归一校验纯壳函数

**Files:**
- Modify: `src/agent/dimension_registry.rs`（在文件末尾 `#[cfg(test)]` 之前加函数；测试加进既有 `mod tests`）

**Interfaces:**
- Consumes: 既有 `validate_dimension_value(db, kind, raw, scope_account_id, intent) -> DimValidation`（同文件 `dimension_registry.rs:180`）；`DimValidation::{Accept(String), Reject(String), DropSilently}`（`dimension_registry.rs:83`）；`WriteIntent::AdminWrite`（`dimension_registry.rs:29`）。
- Produces:
  - `pub(crate) fn fold_stage_validations(results: Vec<DimValidation>) -> Result<Vec<String>, String>`——**可纯测内核**：Accept 收集 canonical / Reject 短路返 Err / DropSilently 跳过。无 IO。
  - `pub(crate) async fn normalize_target_stages(db: &crate::db::Database, scope_account_id: &str, raw_stages: &[String]) -> Result<Vec<String>, String>`——薄壳：逐项 `validate_dimension_value` 收集成 `Vec<DimValidation>` → 调 `fold_stage_validations`。Task 2（media upload）、Task 3（referral create）消费它。

**背景**：`target_stages` 是"进 prompt 候选清单"的软过滤（`media_send.rs:42` / `referral.rs:30` 用 `s == cs` 精确比较）。运营手填的 alias（如"需求挖掘"）若不归一成 canonical（"need_discovery"），与 contact 已归一的 `customer_stage` 永不相等 → 素材静默不发。本函数把每个 stage 走 `validate_dimension_value` 归一。**设计要点**：聚合逻辑（遍历 + Accept/Reject/Drop 处置）抽成不依赖 DB 的纯内核 `fold_stage_validations`，lib 真测它；DB 查询（`validate_dimension_value` 异步 + taxonomy cache）留在外层 `normalize_target_stages` 薄壳，端到端真测在 Task 8 集成。

- [ ] **Step 1: 写失败测试（纯内核 `fold_stage_validations`）**

加到 `src/agent/dimension_registry.rs` 的 `mod tests` 末尾（`}` 之前）。这些测试**真调用被测内核函数**，覆盖 Accept 收集 / Reject 短路 / DropSilently 跳过 / 空输入四种组合行为：

```rust
    #[test]
    fn fold_collects_accepted_canonicals() {
        let r = fold_stage_validations(vec![
            DimValidation::Accept("need_discovery".into()),
            DimValidation::Accept("negotiation".into()),
        ]);
        assert_eq!(r, Ok(vec!["need_discovery".to_string(), "negotiation".to_string()]));
    }

    #[test]
    fn fold_rejects_on_first_reject() {
        // 任一项 Reject → 整体 Err（短路，返该项原因）。
        let r = fold_stage_validations(vec![
            DimValidation::Accept("need_discovery".into()),
            DimValidation::Reject("customer_stage 取值 \"瞎填\" 不在字典内".into()),
            DimValidation::Accept("never_reached".into()),
        ]);
        assert!(matches!(r, Err(ref msg) if msg.contains("不在字典内")));
    }

    #[test]
    fn fold_skips_drop_silently() {
        // DropSilently（空串项）跳过，不进结果、不报错。
        let r = fold_stage_validations(vec![
            DimValidation::Accept("need_discovery".into()),
            DimValidation::DropSilently,
        ]);
        assert_eq!(r, Ok(vec!["need_discovery".to_string()]));
    }

    #[test]
    fn fold_empty_is_ok_empty() {
        let r = fold_stage_validations(vec![]);
        assert_eq!(r, Ok(vec![]));
    }
```

> `DimValidation` 需 `PartialEq` 才能 `assert_eq!`——它已 `#[derive(Debug, Clone, PartialEq, Eq)]`（`dimension_registry.rs:82`），无需改。归一/Reject/未配置的**字典判定**逻辑由既有 `classify_validation` 13 个纯函数测覆盖；本内核只测**聚合处置**。端到端真归一（alias→canonical 经 DB 字典）在 Task 8 集成测。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib fold_collects_accepted_canonicals fold_rejects_on_first_reject fold_skips_drop_silently fold_empty_is_ok_empty`
Expected: FAIL —— `fold_stage_validations` 未定义，编译错误。

- [ ] **Step 3: 实现纯内核 + 异步薄壳**

加到 `src/agent/dimension_registry.rs` 的 `mod tests`（`#[cfg(test)]`）**之前**：

```rust
/// 纯内核（簇 B 缺口 6）：把每个 stage 的 DimValidation 聚合成归一结果。
/// Accept → 收集 canonical；Reject → 短路返 Err（该项原因）；DropSilently（空串）→ 跳过。
/// 无 IO，完全可单测。
pub(crate) fn fold_stage_validations(
    results: Vec<DimValidation>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        match r {
            DimValidation::Accept(canonical) => out.push(canonical),
            DimValidation::Reject(reason) => return Err(reason),
            DimValidation::DropSilently => {} // 空串项，跳过
        }
    }
    Ok(out)
}

/// 归一 + 校验 target_stages（簇 B 缺口 6）：运营手填的客户阶段标注必须与 contact 的
/// canonical customer_stage 同空间，否则运行时 `s == cs` 永不命中、素材/名片静默不发。
/// 每项走 AdminWrite：Accept 收集 canonical（alias 归一）、Reject 整体报错、
/// 字典未配置 fail-soft 放行原值（KindUnconfigured → Accept）。空串项跳过。
/// scope_account_id 为空串时 taxonomy 查询走 global scope（account 维度缺失时的回退）。
/// 聚合逻辑委托纯内核 fold_stage_validations（可单测）；本函数只做 DB 查询。
pub(crate) async fn normalize_target_stages(
    db: &crate::db::Database,
    scope_account_id: &str,
    raw_stages: &[String],
) -> Result<Vec<String>, String> {
    let mut results = Vec::with_capacity(raw_stages.len());
    for stage in raw_stages {
        results.push(
            validate_dimension_value(
                db,
                "customer_stage",
                stage,
                scope_account_id,
                WriteIntent::AdminWrite,
            )
            .await,
        );
    }
    fold_stage_validations(results)
}
```

- [ ] **Step 4: 跑纯内核测试确认通过**

Run: `cargo test --lib fold_collects_accepted_canonicals fold_rejects_on_first_reject fold_skips_drop_silently fold_empty_is_ok_empty`
Expected: 4 个 PASS。再 `cargo build --lib` 确认编译通过。`normalize_target_stages` 未被引用前有预期 dead_code warning，Task 2/3 接线后消失——本 Task 接受这个预期 warning，在 commit message 注明。

- [ ] **Step 5: 提交**

```bash
git add src/agent/dimension_registry.rs
git commit -m "feat(annotation-quality): target_stages 归一校验(纯内核+异步薄壳)(缺口6地基)

fold_stage_validations 纯内核(可单测,4测)聚合 Accept/Reject/Drop;
normalize_target_stages 薄壳逐项 validate_dimension_value(AdminWrite) 归一,
治 alias/canonical drift 致素材静默不发。
normalize 未被引用前有预期 dead_code warning,Task 2/3 接线后消失。"
```

---

### Task 2: media upload 接线归一（缺口 6 素材侧）

**Files:**
- Modify: `src/routes/media_assets.rs:133-160`（`upload_media_asset`，在构造 `ContentAsset` 之前归一 `target_stages`）

**Interfaces:**
- Consumes: `crate::agent::dimension_registry::normalize_target_stages`（Task 1）。当前 `target_stages: Vec<String>`（`media_assets.rs:46` 收集，逗号分隔已 trim 过滤空）；`account_id: Option<String>`（`media_assets.rs:48/94`）。
- Produces: 无新对外接口（端点行为变更：越界 stage → 400）。

- [ ] **Step 1: 确认现状代码**

读 `src/routes/media_assets.rs:100-164`。当前 `target_stages` 在 `:152` 直接 `(!target_stages.is_empty()).then_some(target_stages)` 存库，无归一。`account_id` 在 `:136` 直接 move 进 `ContentAsset`。

- [ ] **Step 2: 实现归一接线**

在 `media_assets.rs` 构造 `ContentAsset` 之前（即 `:133` `let asset = ContentAsset {` 之前）插入归一逻辑。注意 `account_id` 后面要 move 进 asset，所以归一用其引用：

```rust
    // 缺口 6：归一 target_stages 到 canonical（与 contact.customer_stage 同空间），
    // 越界即 400。account_id 缺失 → 空串走 global scope。
    let scope = account_id.as_deref().unwrap_or("");
    let target_stages = crate::agent::dimension_registry::normalize_target_stages(
        &state.db,
        scope,
        &target_stages,
    )
    .await
    .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;
```

这行放在 `:132`（`let ext = ...store_bytes` 之后、`let asset = ContentAsset {` 之前）。`target_stages` 被 shadow 成归一后的 `Vec<String>`，下面 `:152` 的 `(!target_stages.is_empty()).then_some(target_stages)` 不变，自然存归一值。

> 顺序考量：归一在 `store_bytes`（落盘，`:129`）**之后**。可以接受——越界 stage 时文件已落盘但 asset 没入库，留下孤儿文件。本期不做落盘前校验重排（YAGNI，孤儿文件由现有清理机制/可接受），但在 commit message 注明。若 reviewer 认为必须前置，把归一移到 `:106`（大小检查）附近、`store_bytes` 之前——`target_stages` 和 `account_id` 在 multipart 循环（`:100`）后就已就绪，可前移。**实现者：优先前移到 `store_bytes` 之前**（`:124` 落盘注释之前），避免孤儿文件，且 `account_id`/`target_stages` 此时已就绪。

- [ ] **Step 3: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib media_type_whitelist`
Expected: 编译通过（`normalize_target_stages` 现在被引用，Task 1 的 dead_code warning 消失）；既有 `media_assets` 测试仍 PASS。

- [ ] **Step 4: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(annotation-quality): media upload 归一 target_stages,越界400(缺口6素材侧)

归一前置到 store_bytes 之前,越界 stage 直接 400 不落盘不入库。
account_id 缺失走 global scope。"
```

---

### Task 3: referral create 接线归一（缺口 6 名片侧）

**Files:**
- Modify: `src/routes/referral_cards.rs:48-78`（`create_referral_card`，在构造 `ReferralCard` 之前归一 `target_stages`）

**Interfaces:**
- Consumes: `crate::agent::dimension_registry::normalize_target_stages`（Task 1）。当前 `payload.target_stages: Vec<String>`（`referral_cards.rs:32`）；`payload.account_id: Option<String>`（`:26`）。
- Produces: 无新对外接口（端点行为变更：越界 stage → 400）。

- [ ] **Step 1: 确认现状代码**

读 `src/routes/referral_cards.rs:48-78`。当前 `payload.target_stages` 在 `:66` 直接进 `ReferralCard`，`payload.account_id` 在 `:62`，均无归一。

- [ ] **Step 2: 实现归一接线**

在 `create_referral_card` 的 `let card = ReferralCard {`（`:59`）之前、`targetWxid/displayName` 非空校验（`:54-58`）之后插入。注意 `payload.account_id` 后面 move 进 card，用引用做 scope：

```rust
    // 缺口 6：归一 target_stages 到 canonical（与 contact.customer_stage 同空间），
    // 越界即 400。account_id 缺失 → 空串走 global scope。
    let scope = payload.account_id.as_deref().unwrap_or("");
    let target_stages = crate::agent::dimension_registry::normalize_target_stages(
        &state.db,
        scope,
        &payload.target_stages,
    )
    .await
    .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;
```

然后把 `:66` 的 `target_stages: payload.target_stages,` 改为 `target_stages,`（用归一后的本地变量）。

- [ ] **Step 3: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib --package wechatagent referral 2>&1 | tail -20`
Expected: 编译通过；既有 referral 相关 lib 测试 PASS。

- [ ] **Step 4: 提交**

```bash
git add src/routes/referral_cards.rs
git commit -m "feat(annotation-quality): referral create 归一 target_stages,越界400(缺口6名片侧)"
```

---

### Task 4: media review 写审计（缺口 3 素材侧）

**Files:**
- Modify: `src/routes/media_assets.rs:174-201`（`review_media_asset`，在 `$set review_status` 成功后加 fail-soft 审计）

**Interfaces:**
- Consumes: `crate::agent::write_event_for_account(state, account_id, contact_wxid, kind, status, summary, details) -> AppResult<()>`（re-export 自 `agent/mod.rs:83`，签名见 `gateway.rs:3963`）。`admin.username`（`AuthenticatedAdmin`，`auth/mod.rs:61`）。`ContentAsset` 有 `account_id: Option<String>`、`title: String`。
- Produces: 无新对外接口（端点副作用增加：events 落一条审计）。

**背景**：当前 review 端点只 `$set review_status`，无审计痕迹。本 Task 在更新成功后回查 asset 拿 account_id/title，写一条 `media_asset.reviewed` 事件。fail-soft：审计写失败只 warn，不回滚 review（既成事实）。

- [ ] **Step 1: 确认现状代码**

读 `src/routes/media_assets.rs:174-201`。当前 `update_one` 后判 `matched_count == 0` 返 404，然后直接 `Ok(json!({"ok": true}))`，无审计。`update_one` 不返回文档，需回查拿 account_id/title。

- [ ] **Step 2: 实现 fail-soft 审计接线**

把 `media_assets.rs:197-200`（`if res.matched_count == 0 { ... } Ok(...)`）改为：

```rust
    if res.matched_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }
    // 缺口 3：审计审核动作（谁把哪份素材改成什么状态）。回查拿 account_id/title。
    // fail-soft：审计写失败只 warn，不回滚 review（review 已生效=既成事实）。
    if let Ok(Some(asset)) = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await
    {
        let account_id = asset.account_id.clone().unwrap_or_default();
        let details = doc! {
            "asset_id": oid.to_hex(),
            "review_note": payload.note.clone().unwrap_or_default(),
            "reviewed_by": admin.username.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &account_id,
            None,
            "media_asset.reviewed",
            &payload.status,
            &format!("管理员审核素材：{} → {}", asset.title, payload.status),
            Some(details),
        )
        .await
        {
            tracing::warn!("media_asset.reviewed 审计写入失败（不影响审核）: {e}");
        }
    }
    Ok(Json(json!({ "ok": true })))
```

- [ ] **Step 3: 跑 lib 编译 + 既有测试**

Run: `cargo build --lib && cargo test --lib review_status_whitelist`
Expected: 编译通过；既有 `review_status_whitelist` 测试 PASS。

> 审计是 DB 副作用，lib 无法纯测；落库断言在 Task 8 集成测试（testcontainers）。

- [ ] **Step 4: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(annotation-quality): media review 写审计事件,fail-soft(缺口3素材侧)

approve/draft 后回查 asset 写 media_asset.reviewed 事件(记 reviewed_by=admin.username)。
审计写失败只 warn 不回滚 review(既成事实纪律)。"
```

---

### Task 5: referral review 写审计（缺口 3 名片侧）

**Files:**
- Modify: `src/routes/referral_cards.rs:108-138`（`review_referral_card`，在 `$set review_status` 成功后加 fail-soft 审计）

**Interfaces:**
- Consumes: 同 Task 4 的 `write_event_for_account` + `admin.username`。`ReferralCard` 有 `account_id: Option<String>`、`display_name: String`。
- Produces: 无新对外接口。

- [ ] **Step 1: 确认现状代码**

读 `src/routes/referral_cards.rs:108-138`。当前 `update_one` 后判 `matched_count == 0` 返 BadRequest("card not found")，然后 `Ok(json!({"ok": true}))`。注意：与 media 不同，这里 not-found 返的是 `BadRequest` 不是 `NotFound`——**保持现状不改**，只在成功后加审计。

- [ ] **Step 2: 实现 fail-soft 审计接线**

把 `referral_cards.rs:134-137`（`if result.matched_count == 0 { ... } Ok(...)`）改为：

```rust
    if result.matched_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    // 缺口 3：审计审核动作。回查拿 account_id/display_name。fail-soft：写失败只 warn。
    if let Ok(Some(card)) = state
        .db
        .referral_cards()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await
    {
        let account_id = card.account_id.clone().unwrap_or_default();
        let details = doc! {
            "card_id": oid.to_hex(),
            "review_note": payload.note.clone().unwrap_or_default(),
            "reviewed_by": admin.username.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &account_id,
            None,
            "referral_card.reviewed",
            &payload.status,
            &format!("管理员审核名片：{} → {}", card.display_name, payload.status),
            Some(details),
        )
        .await
        {
            tracing::warn!("referral_card.reviewed 审计写入失败（不影响审核）: {e}");
        }
    }
    Ok(Json(json!({ "ok": true })))
```

- [ ] **Step 3: 跑 lib 编译**

Run: `cargo build --lib`
Expected: 编译通过，无新 warning。

- [ ] **Step 4: 提交**

```bash
git add src/routes/referral_cards.rs
git commit -m "feat(annotation-quality): referral review 写审计事件,fail-soft(缺口3名片侧)"
```

---

### Task 6: assist-override 端点 + 闭集校验 + 路由挂载（缺口 2 后端）

**Files:**
- Modify: `src/routes/contacts.rs`（加 `is_valid_assist_mode` 纯函数 + `AssistOverrideRequest` 结构 + `update_assist_override` handler + `mod tests` 加纯函数测试）
- Modify: `src/routes/mod.rs:135-142`（导入 `update_assist_override`）、`:307` 附近（挂载路由）

**Interfaces:**
- Consumes: `parse_object_id(&id) -> AppResult<ObjectId>`（`shared.rs:28`）；`find_contact_by_id(&state, workspace, &id) -> AppResult<Contact>`（`shared.rs:155`）；`crate::models::ASSIST_MODE_OVERRIDE_ATTR`（= `"assist_mode_override"`，`models.rs:3010`）。
- Produces: `pub(super) async fn update_assist_override(...)` handler；`pub(super) fn is_valid_assist_mode(&str) -> bool`；路由 `PUT /api/contacts/:id/assist-override`。前端 Task 7 调它。

**背景**：`assist_mode_active`（`referral.rs:10`）已支持 `force_on`/`force_off`/缺省回落账号级，但 `ASSIST_MODE_OVERRIDE_ATTR` 全仓零写入路径。本 Task 加唯一写入端点。`default` 走 `$unset`（干净回落），`force_on`/`force_off` 走 `$set`。

- [ ] **Step 1: 写失败测试（纯函数 `is_valid_assist_mode`）**

加到 `src/routes/contacts.rs` 的 `#[cfg(test)] mod tests`（文件末尾，若无则新建）：

```rust
    #[test]
    fn assist_mode_closed_set() {
        assert!(is_valid_assist_mode("default"));
        assert!(is_valid_assist_mode("force_on"));
        assert!(is_valid_assist_mode("force_off"));
        assert!(!is_valid_assist_mode("on"));
        assert!(!is_valid_assist_mode("true"));
        assert!(!is_valid_assist_mode(""));
        assert!(!is_valid_assist_mode("Force_On"));
    }
```

> 若 `contacts.rs` 文件末尾还没有 `#[cfg(test)] mod tests { use super::*; ... }`，新建一个；若已有，把测试加进去。实现者先 grep `mod tests` 确认。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib assist_mode_closed_set`
Expected: FAIL —— `is_valid_assist_mode` 未定义，编译错误。

- [ ] **Step 3: 实现纯函数 + 请求结构 + handler**

在 `src/routes/contacts.rs` 加入（请求结构放文件上部 `#[derive(Deserialize)]` 结构区，如 `:56` 附近；纯函数 + handler 放 handler 区）：

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssistOverrideRequest {
    mode: String,
}

/// 客户级辅助模式 override 闭集校验（缺口 2）。三态：default（回落账号级）/
/// force_on / force_off。守 gateway 状态枚举闭集纪律。
pub(super) fn is_valid_assist_mode(mode: &str) -> bool {
    matches!(mode, "default" | "force_on" | "force_off")
}

/// PUT /api/contacts/:id/assist-override：写客户级辅助模式 override。
/// default → $unset（回落账号级 assist_mode_enabled）；force_on/force_off → $set。
/// workspace 隔离防 IDOR。
pub(super) async fn update_assist_override(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<AssistOverrideRequest>,
) -> AppResult<Json<Value>> {
    if !is_valid_assist_mode(&payload.mode) {
        return Err(AppError::BadRequest(
            "mode must be default|force_on|force_off".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    // workspace 隔离：跨 workspace / 不存在均 404（不泄漏存在性）。
    find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let attr = format!("domain_attributes.{}", crate::models::ASSIST_MODE_OVERRIDE_ATTR);
    let now = DateTime::now();
    let update = if payload.mode == "default" {
        doc! { "$unset": { &attr: "" }, "$set": { "updated_at": now } }
    } else {
        doc! { "$set": { &attr: &payload.mode, "updated_at": now } }
    };
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            update,
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true, "mode": payload.mode })))
}
```

- [ ] **Step 4: 跑纯函数测试确认通过**

Run: `cargo test --lib assist_mode_closed_set`
Expected: PASS。

- [ ] **Step 5: 挂载路由**

在 `src/routes/mod.rs:135-142` 的 `use contacts::{...}` 块加入 `update_assist_override`（按字母序插入，如 `update_assist_override,` 放 `update_custom_agent_instructions` 之前）。

在 `:307`（`.route("/contacts/:id/profile-note", put(update_profile_note))`）之后加一行：

```rust
        .route(
            "/contacts/:id/assist-override",
            put(update_assist_override),
        )
```

> 确认 `put` 已在 mod.rs 的 axum routing 导入中（profile-note 用了 `put`，故已导入）。

- [ ] **Step 6: 跑全 lib 编译 + 测试**

Run: `cargo build --lib && cargo test --lib assist_mode_closed_set`
Expected: 编译通过、PASS。

- [ ] **Step 7: 提交**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "feat(annotation-quality): 客户级辅助模式 override PUT 端点(缺口2后端)

PUT /api/contacts/:id/assist-override,三态闭集(default→\$unset/force_on/force_off→\$set)。
workspace 隔离防 IDOR。is_valid_assist_mode 纯函数闭集校验。"
```

---

### Task 7: 前端单客户辅助模式 override 三态下拉（缺口 2 前端）

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`（state `assistOverride` + setter + `hydrateSelected` 注入 + `saveAssistOverride` action）
- Modify: `frontend/src/features/user-ops/index.tsx`（解构 `saveAssistOverride` + 透传两个 prop）
- Modify: `frontend/src/features/user-ops/legacy.tsx`（`SmartWorkspace` props 签名 + 解构 + 单客户 section JSX 加下拉）

**Interfaces:**
- Consumes: 后端 `PUT /api/contacts/:id/assist-override { mode }`（Task 6）。`api.put`（`frontend/src/lib/api.ts`）。`Contact.domainAttributes?: Record<string,unknown>`（`types/index.ts:63`）。
- Produces: 无（纯前端 UI）。

**背景**：照 `customAgentInstructions` 受控链路（store state + setter + `hydrateSelected` 注入 + save action）。**不能**直接从 `selected.domainAttributes` 派生回显——`refreshContacts` 只重拉列表不刷 `selected`，会回显旧值。下拉初值由 `hydrateSelected` 从 contact 注入。

- [ ] **Step 1: store 加 state + setter + hydrate 注入**

`frontend/src/stores/userOpsStore.ts`：

1）state 类型区（`:45` `customAgentInstructions: string;` 旁）加：
```ts
  assistOverride: string; // "default" | "force_on" | "force_off"
```

2）action 类型区（`:74` `setCustomAgentInstructions` 旁）加：
```ts
  setAssistOverride: (mode: string) => void;
  saveAssistOverride: () => Promise<void>;
```

3）state 初值区（`:250` `customAgentInstructions: "",` 旁）加：
```ts
  assistOverride: "default",
```

4）setter 实现区（`:277` `setCustomAgentInstructions` 旁）加：
```ts
  setAssistOverride: (mode) => set({ assistOverride: mode }),
```

5）`hydrateSelected`（`:292-299`）的 `set({...})` 里加一行——从 domainAttributes 读 override，缺失则 "default"：
```ts
      assistOverride:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "assist_mode_override"
        ] as string) || "default",
```

- [ ] **Step 2: store 加 saveAssistOverride action**

照 `saveCustomAgentInstructions`（`:454-474`）写，加在其后：

```ts
  saveAssistOverride: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { assistOverride } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/assist-override`, {
        mode: assistOverride
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
```

- [ ] **Step 3: index.tsx 解构 + 透传**

`frontend/src/features/user-ops/index.tsx`：

1）index.tsx 从 `userOpsStore` 解构处分三块加：
- **setter 块**（`:87-89`，`setProfileNote` / `setCustomAgentInstructions` / `setGuideInstruction` 所在）加 `setAssistOverride,`
- **回调块**（`:104-124`，`saveProfileNote` / `saveCustomAgentInstructions` 所在）加 `saveAssistOverride,`
- **state 值**：`assistOverride` 也来自 `userOpsStore`，加到同一 `userOpsStore` 解构（与 `profileNote` / `customAgentInstructions` 等受控值同处；grep `customAgentInstructions` 在 index.tsx 的解构行确认其位置，紧邻加 `assistOverride,`）

2）`SmartWorkspace` 透传区（`:269/273` 旁）加：
```tsx
            onAssistOverride={setAssistOverride}
            onSaveAssistOverride={saveAssistOverride}
```

3）同区找到传 `assistOverride` 值的地方——`SmartWorkspace` 还需要当前值。在透传区加：
```tsx
            assistOverride={assistOverride}
```
（`assistOverride` 来自 store 解构，Step 1 已加 state；在 index.tsx 的 store 解构里加 `assistOverride,`）

- [ ] **Step 4: legacy.tsx SmartWorkspace props 签名 + 解构**

`frontend/src/features/user-ops/legacy.tsx` 的 `SmartWorkspace` props 类型（`:159-194` 内联对象类型）加：
```ts
  assistOverride: string;
  onAssistOverride: (mode: string) => void;
  onSaveAssistOverride: () => void;
```
并在组件参数解构里加 `assistOverride, onAssistOverride, onSaveAssistOverride`（与 `onSaveProfileNote` 等同处解构）。

- [ ] **Step 5: legacy.tsx 单客户 section 加三态下拉**

在单客户操作 section（`:380-422`，customAgentInstructions / 保存按钮所在区）的合适位置（如 `customAgentInstructions` 的 `</label>` 之后、`buttonRow`（`:396`）之前）加：

```tsx
          {selected.agentStatus === "managed" && (
            <label>
              <span>辅助模式（本客户）</span>
              <small>覆盖账号级默认：跟随账号 / 强制为本客户引荐专属顾问 / 强制不引荐。</small>
              <select
                value={assistOverride}
                onChange={(event) => onAssistOverride(event.target.value)}
              >
                <option value="default">跟随账号默认</option>
                <option value="force_on">强制开启引荐</option>
                <option value="force_off">强制关闭引荐</option>
              </select>
              <button className="secondary" onClick={onSaveAssistOverride} disabled={busy} type="button">
                <SquarePen size={16} />
                保存辅助模式
              </button>
            </label>
          )}
```

> `SquarePen` 已在 legacy.tsx 导入（`:391` 用过）。设计语言遵现有 `<label><span><small><select>` 表单模式，不新造样式。守 no-human-takeover 禁词（文案用"辅助模式 / 引荐"，无禁词）。

- [ ] **Step 6: 前端构建验证**

Run: `cd frontend && npm run build`
Expected: 构建通过，无 TS 错误。

> 注：禁词 lint 扫 `frontend/src/` 新增行——确认下拉文案无 `人工` 等禁词（"辅助模式/引荐/账号"均安全）。

- [ ] **Step 7: 提交**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/index.tsx frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(annotation-quality): 单客户辅助模式 override 三态下拉(缺口2前端)

照 customAgentInstructions 受控链路:store state+setter+hydrateSelected 注入+save action。
下拉初值从 contact.domainAttributes.assist_mode_override 注入(不直接派生 selected,避 refreshContacts 不刷 selected 的回显旧值坑)。"
```

---

### Task 8: 集成测试（三缺口端到端，`#[ignore]` / CI）

**Files:**
- Create: `tests/annotation_quality_gate_integration.rs`

**Interfaces:**
- Consumes: 既有测试设施 `tests/common/mod.rs`（`TestApp` / testcontainers MongoDB / 种子 helper）。Task 1-6 的端点行为。
- Produces: 无（测试）。

**背景**：壳函数归一、审计落库、override 写入都是 DB 副作用，lib 无法纯测。本 Task 用 testcontainers 钉端到端真实行为。**全部 `#[ignore]`**（需 Docker，本地不跑，交 CI `integration` job）。

- [ ] **Step 1: 读测试设施**

读 `tests/common/mod.rs` 确认 `TestApp` 构造、登录拿 admin session、发请求的 helper（如何带 cookie / workspace）、以及种子 base domain config 的 helper（`db_seed_base_domain_config` 应改 upsert 形态——见 [[project_config_seed_in_prompts_not_migrations]]）。**确认 customer_stage 字典种子**：归一测试需要字典里有一个 alias→canonical 映射（如 "需求挖掘"→"need_discovery"）。grep `tests/` 找现有 taxonomy 种子 helper；若无，本测试用"字典未配置 fail-soft 放行原值"路径断言（上传任意 stage → 原样存），避开需要种字典的归一断言（归一逻辑由 lib `classify_alias_normalizes` 覆盖）。

> **实现者决策点**：归一断言（alias→canonical）需要预种 customer_stage 字典含该 alias。若测试设施已有 taxonomy 种子 helper，种一个 alias 测真归一；若没有且自建成本高，退而测「字典未配置 → 原值放行」+「字典有条目但越界 → 400」两条（这两条不需要精确 alias 映射，只需种 ≥1 个 customer_stage 条目让 `kind_has_entries` 为真）。两种都验证了缺口 6 的核心行为。优先测「越界 400」+「未配置放行」。

- [ ] **Step 2: 写缺口 6 集成测试（target_stages 校验）**

```rust
//! 簇 B 标注质量门集成测试：缺口 6（target_stages 归一校验）/ 缺口 3（审核审计）/
//! 缺口 2（客户级 override）。全部 #[ignore]，需 Docker testcontainers。
mod common;
use common::*;

#[tokio::test]
#[ignore]
async fn upload_with_out_of_dict_stage_rejected_when_dict_configured() {
    // 字典已配置 ≥1 个 customer_stage 条目 → 填字典里没有的阶段名 → 400。
    let app = TestApp::spawn().await;
    app.login_admin().await;
    // 种一个 customer_stage 字典条目（让 kind_has_entries 为真）。
    app.seed_customer_stage_taxonomy("need_discovery").await;

    let resp = app
        .upload_media_asset_multipart(/* title */ "测试素材", /* target_stages */ "不存在的阶段名")
        .await;
    assert_eq!(resp.status(), 400, "字典已配置时越界 stage 必须 400");
}

#[tokio::test]
#[ignore]
async fn upload_with_unconfigured_dict_accepts_raw_stage() {
    // customer_stage 字典整个未配置 → fail-soft 放行原值（不误拒）。
    let app = TestApp::spawn().await;
    app.login_admin().await;
    // 不种任何 customer_stage 字典条目。

    let resp = app
        .upload_media_asset_multipart("测试素材", "任意阶段")
        .await;
    assert_eq!(resp.status(), 200, "字典未配置时应 fail-soft 放行");
    // 回查库里 target_stages 含原值。
    let asset = app.find_latest_content_asset().await;
    assert!(
        asset.target_stages.unwrap_or_default().contains(&"任意阶段".to_string()),
        "未配置时原值存库"
    );
}
```

> `upload_media_asset_multipart` / `seed_customer_stage_taxonomy` / `find_latest_content_asset` 若 `common/mod.rs` 没有，在本测试文件内联实现或扩展 common（按既有 helper 风格）。实现者：先看 `tests/media_asset_send_integration.rs` 是否已有 multipart 上传 helper 可复用。

- [ ] **Step 3: 写缺口 3 集成测试（审核审计）**

```rust
#[tokio::test]
#[ignore]
async fn review_media_asset_writes_audit_event() {
    let app = TestApp::spawn().await;
    app.login_admin().await;
    let asset_id = app.upload_draft_media_asset().await; // 返回 hex id

    let resp = app.review_media_asset(&asset_id, "approved", Some("审核通过")).await;
    assert_eq!(resp.status(), 200);

    // events 集合应有一条 kind=media_asset.reviewed、status=approved。
    let evt = app.find_event_by_kind("media_asset.reviewed").await;
    assert!(evt.is_some(), "review 后应落审计事件");
    let evt = evt.unwrap();
    assert_eq!(evt.status, "approved");
    // reviewed_by 记录在 details。
    let details = evt.details.expect("审计 details 应非空");
    assert_eq!(details.get_str("reviewed_by").unwrap_or(""), app.admin_username());
}
```

> 缺口 3 的 fail-soft（审计写失败不影响 review）无法在集成测试里轻易制造"审计写失败"，由代码审查保证 + 上面"成功落库"正向断言覆盖主路径。**不**写依赖故障注入的测试（YAGNI）。

- [ ] **Step 4: 写缺口 2 集成测试（override + IDOR）**

```rust
#[tokio::test]
#[ignore]
async fn assist_override_force_on_then_default_unsets() {
    let app = TestApp::spawn().await;
    app.login_admin().await;
    let contact_id = app.seed_managed_contact().await; // 返回 contact hex id

    // force_on → domain_attributes.assist_mode_override = "force_on"
    let resp = app.put_assist_override(&contact_id, "force_on").await;
    assert_eq!(resp.status(), 200);
    let c = app.find_contact(&contact_id).await;
    assert_eq!(
        c.domain_attributes_str("assist_mode_override"),
        Some("force_on".to_string())
    );

    // default → 键被 $unset
    let resp = app.put_assist_override(&contact_id, "default").await;
    assert_eq!(resp.status(), 200);
    let c = app.find_contact(&contact_id).await;
    assert_eq!(c.domain_attributes_str("assist_mode_override"), None, "default 应 unset 键");
}

#[tokio::test]
#[ignore]
async fn assist_override_invalid_mode_rejected() {
    let app = TestApp::spawn().await;
    app.login_admin().await;
    let contact_id = app.seed_managed_contact().await;
    let resp = app.put_assist_override(&contact_id, "bogus").await;
    assert_eq!(resp.status(), 400, "闭集外 mode 必须 400");
}

#[tokio::test]
#[ignore]
async fn assist_override_cross_workspace_404() {
    // 跨 workspace 的 contact id → 404（IDOR 防护）。
    let app = TestApp::spawn().await;
    app.login_admin().await;
    let other_id = app.seed_contact_in_other_workspace().await;
    let resp = app.put_assist_override(&other_id, "force_on").await;
    assert_eq!(resp.status(), 404, "跨 workspace contact 不可写");
}
```

> helper（`seed_managed_contact` / `put_assist_override` / `find_contact` / `domain_attributes_str` / `seed_contact_in_other_workspace`）按 `tests/common/mod.rs` 既有风格实现或复用。`seed_contact_in_other_workspace` 若 common 无多 workspace 设施，参考既有 IDOR 测试（grep `tests/` 找 `workspace` + `404`）。

- [ ] **Step 5: 编译验证（不跑 ignored）**

Run: `cargo test --test annotation_quality_gate_integration --no-run`
Expected: 编译通过（`#[ignore]` 测试不执行，仅确认编译）。

> 本地磁盘紧 + 无 Docker，**不**跑 `-- --ignored`。CI `integration` job 会跑。

- [ ] **Step 6: 提交**

```bash
git add tests/annotation_quality_gate_integration.rs
git commit -m "test(annotation-quality): 三缺口端到端集成测试(#[ignore]/CI)

缺口6 越界400+未配置放行;缺口3 审核落审计事件;缺口2 override force_on/default unset+闭集400+跨workspace 404(IDOR)。"
```

---

## 执行顺序与依赖

- Task 1 → Task 2/3（缺口 6，2、3 都依赖 1 的 `normalize_target_stages`）
- Task 4、5（缺口 3，互相独立，依赖既有 `write_event_for_account`）
- Task 6 → Task 7（缺口 2，前端依赖后端端点）
- Task 8 最后（依赖 Task 1-6 全部端点行为）

Task 2/3、4/5 形态对称但分文件、可被 reviewer 独立判，故拆开。Task 1-6 是后端、可本地 `cargo test --lib` 验；Task 7 前端 `npm run build` 验；Task 8 仅 `--no-run` 编译验（CI 跑 ignored）。
