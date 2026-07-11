# 用户运营池显示/内容问题修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复运营池（ContactsView）三个已 Playwright 复现的显示/内容问题：三档 tab 挤成畸形椭圆、消息预览吐原始 XML、系统号混入池带「启用 Agent」；并加媒体号手动「从池移除」入口。

**Architecture:** 四层最小改动。①后端纯函数 `preview_label_for_type` 按 `msg_type` 出友好标签替代无脑截断 XML；②抽 `is_system_account(wxid)` 纯函数扩展 `is_operatable_person` 让建档 + list 读时双拦系统号；③`hidden_from_pool` **doc-only 标记**（不进 Contact struct，Mongo filter + `$set` 层面处理——已亲验 contacts 无 `replace_one` 全 struct 写路径、Contact 无 `deny_unknown_fields`，故 doc-only 安全且避免 160 处 `Contact{}` 构造点编译改动）+ 新端点 + list/count 过滤 + 前端「从池移除」按钮；④CSS 把 `.segmented` 从 `.panelHead` 的 `space-between` 横行中解放，下移独占一行。

**Tech Stack:** Rust (Axum) 后端；React 19 + TypeScript + Zustand 前端；MongoDB；vitest（前端）+ cargo test（后端）；Python Playwright（截图验证）。

## Global Constraints

- `cargo test --lib` ≥ 350 passed / 0 failed（merge gate，`scripts/check-baseline.sh` LIB_BASELINE=350）。
- `cargo check --tests` 必须过（复刻 CI baseline step2，`cargo test --lib` 不编译集成测试）。
- `cd frontend && npm run build` 必须过；`cd frontend && npx vitest run` 必须全绿。
- `scripts/check-no-human-takeover.sh` 文案门：新增文案不得含 `人工/接管/takeover/hand-off`（扫 src/routes/、frontend/src/ 等 git diff 新增行）。「从池移除」「[系统消息]」等已确认不含禁用词。
- 红线：改任何一行前先 100% 读懂相关代码，引用必亲验 file:line，绝不猜测。
- `normal` 联系人**绝不调 LLM**——预览是静态标签/原文截断，非智能摘要（产品红线）。
- 不改 webhook 消息落库 / gateway / principal 决策通道 / quiet-hours。
- 不新增 migration：`hidden_from_pool` 靠 doc-only + `$ne:true` 兼容缺字段旧文档；系统号存量靠 list 读时过滤清理。
- 不动通讯录 RosterView。
- YAGNI：不做「已移除」视图 / 恢复端点（单向移除）；不自动判媒体号（无可靠信号）。
- **部署时机需协调**：117 当前挂并行会话 `verify-batch2` 分支，不贸然覆盖；部署必须同时 `npm run build` 重建前端。

**已亲验关键 file:line（实现时据此，勿凭记忆）：**
- `truncate_preview`：`src/routes/contacts.rs:107`（`pub(crate)`，按字符截断）。
- `list_contacts` 填预览：`src/routes/contacts.rs:212-213`，`INBOUND_PREVIEW_MAX_CHARS=30`（:103）。预览查询取 `msg`（`ConversationMessage`），其 `msg_type: Option<String>` 在 `models.rs:775`、`content: String` 在 `models.rs:772`。
- `classify_inbound_msg_type`：`src/webhooks.rs:925`，归一为 text/image/voice/video/namecard/emoji/location/appmsg/voip/statussync/system/unknown。
- `is_operatable_person`：`src/webhooks.rs:1037-1039`，现 `!(wxid.starts_with("gh_") || wxid.contains("@chatroom"))`；已有单测 `is_operatable_person_rejects_official_and_group`（:1431）+ `_accepts_real_wxid`（:1438）。
- `WECHAT_SYSTEM_ACCOUNTS`：`src/mcp.rs:496-500`（私有 const）；`is_non_human_account`：`src/mcp.rs:503`（私有 fn）。
- `contact_count_filters`：`src/routes/contacts.rs:224-229`；`count_contacts`：`:237-250`。list `filter` 构造：`:121-145`。
- `enable_agent`：`src/routes/contacts.rs:842`；`disable_agent`（update_one `$set` 单字段范式）：`:897-919`。`parse_object_id`：`src/routes/shared.rs:40`；`find_contact_by_id`：`src/routes/shared.rs:167`（`super::shared::*` 已 `use`，contacts.rs:26）。
- 路由挂载：`src/routes/mod.rs:357-366`（contacts 路由簇）。
- `AppError::NotFound`/`BadRequest`：`src/error.rs:11,13`。
- webhook 建档 upsert（`$set`/`$setOnInsert`，无 replace_one）：`src/webhooks.rs:1111-1132`；判据早退：`:1049-1051`。
- 前端 ContactsView：`frontend/src/features/user-ops/legacy.tsx`——`panelHead`/`poolHeadText`/`segmented`:552-572；行渲染 552-691；预览渲染 :659-661；单人「启用 Agent」按钮 :672-684；`onBatchEnable?` prop :508；`selectable`:516。
- ContactsView 挂载点：`frontend/src/features/user-ops/index.tsx:267-294`（`onBatchEnable` 注入范式）。
- store：`frontend/src/stores/userOpsStore.ts`——`batchEnable`:494-499、`loadContacts`:455-457、`loadContactCounts`:461-471、`refreshContacts`:287-300（写 `useContactStore.getState().setContacts`）、`api` import :22。
- api client：`frontend/src/lib/api.ts`——`post`:58、`get`:53（无 `put`，端点用 POST）。
- 前端 Contact 类型：`frontend/src/types/index.ts`——`lastInboundPreview?`:131、`agentStatus`:94、`operationState?`:116。
- CSS：`.panelHead`:444（`display:flex; justify-content:space-between`）、`.segmented`:1476、`.poolHeadText`:1638（`display:grid`）、`.poolTierSub`:1657。
- 前端契约测试：`frontend/src/__tests__/features/user-ops/contactsView.test.tsx`。
- Playwright before 图 `pool_before.png` + harness `frontend/pool_harness.html` + `frontend/src/pool_harness.tsx` + `scripts/e2e/pool_shot.py`（截 `section.panel`，URL `http://localhost:5173/pool_harness.html`）。

---

### Task 1: `preview_label_for_type` 纯函数 + 单测（后端预览标签化）

**Files:**
- Modify: `src/routes/contacts.rs`（新增纯函数，紧邻 `truncate_preview` 之后，约 :114 后）
- Modify: `src/routes/contacts.rs:212-213`（`list_contacts` 改调新函数）
- Test: `src/routes/contacts.rs`（文件内 `#[cfg(test)] mod` —— 已有 `contact_count_filters_isolate_workspace_and_account` 在 :1594，加同 mod 或新建 mod）

**Interfaces:**
- Consumes: `truncate_preview(text: &str, max_chars: usize) -> String`（:107，现有）；`INBOUND_PREVIEW_MAX_CHARS`（:103，现有）。
- Produces: `preview_label_for_type(msg_type: Option<&str>, content: &str) -> String` —— 供 `list_contacts` 调用。

- [ ] **Step 1: 写失败测试**

在 `src/routes/contacts.rs` 的 test mod（复用 :1594 那个 `mod tests` 或就近同名 mod）加：

```rust
#[test]
fn preview_label_text_truncates_content() {
    // text / None 走原文截断（现有语义）
    assert_eq!(preview_label_for_type(Some("text"), "你好呀"), "你好呀");
    assert_eq!(preview_label_for_type(None, "旧消息无类型"), "旧消息无类型");
    let long: String = "字".repeat(40);
    let out = preview_label_for_type(Some("text"), &long);
    assert!(out.ends_with('…'));
    assert_eq!(out.chars().count(), INBOUND_PREVIEW_MAX_CHARS + 1); // 30 + 省略号
}

#[test]
fn preview_label_non_text_uses_static_label_not_content() {
    // 非 text 类型绝不读 content（content 可能是 XML 垃圾）
    let xml = "<msg><appmsg appid=\"\" sdk...";
    assert_eq!(preview_label_for_type(Some("appmsg"), xml), "[链接]");
    assert_eq!(preview_label_for_type(Some("system"), "<sysmsg type=\"functionmsg\">"), "[系统消息]");
    assert_eq!(preview_label_for_type(Some("image"), "irrelevant"), "[图片]");
    assert_eq!(preview_label_for_type(Some("voice"), ""), "[语音]");
    assert_eq!(preview_label_for_type(Some("video"), ""), "[视频]");
    assert_eq!(preview_label_for_type(Some("namecard"), ""), "[名片]");
    assert_eq!(preview_label_for_type(Some("emoji"), ""), "[表情]");
    assert_eq!(preview_label_for_type(Some("location"), ""), "[位置]");
    assert_eq!(preview_label_for_type(Some("voip"), ""), "[通话]");
    assert_eq!(preview_label_for_type(Some("statussync"), ""), ""); // 状态同步无展示意义
    assert_eq!(preview_label_for_type(Some("unknown"), ""), "[消息]");
    assert_eq!(preview_label_for_type(Some("某新类型"), ""), "[消息]"); // 兜底
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib preview_label`
Expected: FAIL —— `cannot find function preview_label_for_type`

- [ ] **Step 3: 写最小实现**

在 `src/routes/contacts.rs` `truncate_preview` 之后加（映射键与 `classify_inbound_msg_type`（webhooks.rs:925）产出的字符串完全对齐）：

```rust
/// 按入站消息类型出运营池预览。text/None（含旧库无类型消息）走原文截断；
/// 其它类型出固定中文标签，**绝不读 content**——appmsg/sysmsg 的 content 本身就是
/// XML 串（`<msg><appmsg.../<sysmsg type=...>`），截断后是 XML 垃圾。
/// 纯静态映射，非 LLM 摘要——normal 联系人不调 LLM（产品红线）。
/// 类型字符串来源：webhooks::classify_inbound_msg_type。
pub(crate) fn preview_label_for_type(msg_type: Option<&str>, content: &str) -> String {
    match msg_type {
        None | Some("text") => truncate_preview(content, INBOUND_PREVIEW_MAX_CHARS),
        Some("image") => "[图片]".to_string(),
        Some("voice") => "[语音]".to_string(),
        Some("video") => "[视频]".to_string(),
        Some("namecard") => "[名片]".to_string(),
        Some("emoji") => "[表情]".to_string(),
        Some("location") => "[位置]".to_string(),
        Some("appmsg") => "[链接]".to_string(),
        Some("voip") => "[通话]".to_string(),
        Some("statussync") => String::new(),
        Some("system") => "[系统消息]".to_string(),
        Some(_) => "[消息]".to_string(),
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib preview_label`
Expected: PASS（2 tests）

- [ ] **Step 5: 接线 list_contacts**

改 `src/routes/contacts.rs:212-213`：

```rust
            api.last_inbound_preview = Some(preview_label_for_type(
                msg.msg_type.as_deref(),
                &msg.content,
            ));
```

- [ ] **Step 6: 编译验证**

Run: `cargo check --tests`
Expected: 0 errors

- [ ] **Step 7: Commit**

```bash
git add src/routes/contacts.rs
git commit -m "fix(user-ops): 运营池预览按msg_type标签化(appmsg→[链接]/sysmsg→[系统消息]),非text不吐XML"
```

---

### Task 2: `is_system_account` 纯函数 + 扩展 `is_operatable_person`（系统号自动拦）

**Files:**
- Modify: `src/mcp.rs:496-505`（`WECHAT_SYSTEM_ACCOUNTS` const + 新增 `is_system_account` pub(crate) fn；`is_non_human_account` 内部复用新函数保持单一来源）
- Modify: `src/webhooks.rs:1037-1039`（`is_operatable_person` 扩展）
- Test: `src/webhooks.rs`（已有 test mod :1431-1443，扩充）；`src/mcp.rs`（就近 test mod）

**Interfaces:**
- Consumes: `WECHAT_SYSTEM_ACCOUNTS: &[&str]`（mcp.rs:496，现有）。
- Produces: `crate::mcp::is_system_account(wxid: &str) -> bool` —— 供 webhooks `is_operatable_person` 复用。判据同源，杜绝两份系统号清单漂移。

- [ ] **Step 1: 写失败测试（mcp.rs 纯函数）**

在 `src/mcp.rs` 的 `#[cfg(test)] mod` 内加（若无就近 mod 则新建 `#[cfg(test)] mod system_account_tests { use super::*; ... }`）：

```rust
#[test]
fn is_system_account_matches_wechat_reserved() {
    assert!(is_system_account("weixin"));      // 微信团队
    assert!(is_system_account("fmessage"));    // 朋友推荐消息
    assert!(is_system_account("newsapp"));
    assert!(is_system_account("filehelper"));
    // 真人 wxid_* 不命中
    assert!(!is_system_account("wxid_ydzaomn4scsb12"));
    assert!(!is_system_account("wxid_8874178741811")); // 福州晚报=媒体号,wxid_*,不靠此拦
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib is_system_account`
Expected: FAIL —— `cannot find function is_system_account`

- [ ] **Step 3: 写最小实现（mcp.rs）**

改 `src/mcp.rs:502-505`，抽出 `is_system_account` 并让 `is_non_human_account` 复用它（单一来源）：

```rust
/// 判定 wxid 是否微信官方保留系统账号（业界通用白名单）。这些不是能运营的真人
/// 私聊——建档判据（webhooks::is_operatable_person）与 roster 非真人标记共用此判据，
/// 杜绝两份清单漂移。注意：公众号（gh_ 前缀）、媒体号（wxid_* 好友，如福州晚报）
/// 无可靠字段识别，**不在此列**——公众号靠 gh_ 前缀单独拦，媒体号只能人工移除。
pub(crate) fn is_system_account(wxid: &str) -> bool {
    WECHAT_SYSTEM_ACCOUNTS.contains(&wxid)
}

/// 判定是否非真人账号：type=="system" 或 wxid 命中微信保留白名单。
fn is_non_human_account(user_name: &str, item_type: Option<&str>) -> bool {
    item_type == Some("system") || is_system_account(user_name)
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib is_system_account`
Expected: PASS

- [ ] **Step 5: 扩展 is_operatable_person + 写测试**

改 `src/webhooks.rs:1037-1039`：

```rust
/// 判定 wxid 是否能进运营池的私聊真人：排除公众号（gh_ 前缀）、群（@chatroom）、
/// 微信官方系统保留号（weixin/fmessage/... 复用 mcp::is_system_account 同源判据）。
/// 建档 upsert（:1049）+ list_contacts 读时（contacts.rs:177）双处同源调用。
pub(crate) fn is_operatable_person(wxid: &str) -> bool {
    !(wxid.starts_with("gh_")
        || wxid.contains("@chatroom")
        || crate::mcp::is_system_account(wxid))
}
```

在 `src/webhooks.rs` test mod（:1431 那个）扩充：

```rust
#[test]
fn is_operatable_person_rejects_system_accounts() {
    assert!(!is_operatable_person("weixin"));   // 微信团队
    assert!(!is_operatable_person("fmessage")); // 朋友推荐消息
    assert!(!is_operatable_person("newsapp"));
    // 真人 + 媒体号 wxid_* 仍放行（媒体号靠手动移除，非此拦）
    assert!(is_operatable_person("wxid_8874178741811")); // 福州晚报
}
```

- [ ] **Step 6: 运行确认通过**

Run: `cargo test --lib is_operatable_person is_system_account`
Expected: PASS（原有 2 测试 + 新增 2 测试全绿）

- [ ] **Step 7: 编译验证**

Run: `cargo check --tests`
Expected: 0 errors

- [ ] **Step 8: Commit**

```bash
git add src/mcp.rs src/webhooks.rs
git commit -m "fix(user-ops): 系统号(weixin/fmessage等)不进运营池——is_operatable_person复用is_system_account同源判据"
```

---

### Task 3: `hidden_from_pool` doc-only 标记 + 隐藏端点 + list/count 过滤（后端）

**Files:**
- Modify: `src/routes/contacts.rs`（新增 `hide_from_pool` handler；`list_contacts` filter :121 加过滤；`contact_count_filters` :225 加过滤）
- Modify: `src/routes/mod.rs:366` 后（挂新路由）
- Test: `src/routes/contacts.rs`（test mod：`contact_count_filters` 断言含 hidden 过滤）

**Interfaces:**
- Consumes: `parse_object_id`（shared.rs:40）；`find_contact_by_id`（shared.rs:167）；`ApiContact::from`（models.rs:3377）；`disable_agent` 的 update_one `$set` 范式（contacts.rs:903-916）。
- Produces: `POST /api/contacts/:id/hide-from-pool` → `{ "item": ApiContact }`；`hidden_from_pool` **doc-only** 字段（不进 Contact struct）。

**为何 doc-only（关键设计，已亲验）：** Contact 无 `#[serde(deny_unknown_fields)]`（models.rs:148），且 contacts 集合**无任何 `replace_one` 全 struct 写路径**（webhook upsert 用 `$set`/`$setOnInsert`，enable/disable 用 `$set` 单字段）——故 doc 里多一个 `hidden_from_pool` 字段既不破坏反序列化、也永不被后续写覆盖。这样避免给 Contact 加字段引发的 160 处 `Contact{}` 构造点编译改动（config-field-add trap）。

- [ ] **Step 1: 写失败测试（count filter 含 hidden 过滤）**

`contact_count_filters` 现返回 `(base, managed)`。改为 base 带 `hidden_from_pool: {$ne: true}`。先改测试 :1594：

```rust
#[test]
fn contact_count_filters_isolate_workspace_and_account() {
    let (base, managed) = contact_count_filters("ws1", "acct1");
    assert_eq!(base.get_str("workspace_id").unwrap(), "ws1");
    assert_eq!(base.get_str("account_id").unwrap(), "acct1");
    // 隐藏的联系人不计入任何 tab 计数（与 list_contacts 口径一致）
    assert!(base.get_document("hidden_from_pool").is_ok());
    assert_eq!(managed.get_str("agent_status").unwrap(), "managed");
    assert!(managed.get_document("hidden_from_pool").is_ok());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib contact_count_filters`
Expected: FAIL —— `get_document("hidden_from_pool")` 返回 Err

- [ ] **Step 3: 改 contact_count_filters + list_contacts filter**

改 `src/routes/contacts.rs:224-229`：

```rust
fn contact_count_filters(workspace_id: &str, account_id: &str) -> (Document, Document) {
    let base = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        // 手动「从池移除」的联系人（hidden_from_pool=true）不计入——与 list_contacts 同源口径。
        // $ne:true 兼容缺字段旧文档（doc-only 字段，非 Contact struct 成员）。
        "hidden_from_pool": { "$ne": true },
    };
    let mut managed = base.clone();
    managed.insert("agent_status", "managed");
    (base, managed)
}
```

改 `src/routes/contacts.rs:121`（`let mut filter = doc! {};` 后紧接，或直接初始化时加）——在 workspace/account insert 之后加：

```rust
    // 手动移除的联系人不出现在列表（doc-only 标记，$ne:true 兼容旧文档）。
    filter.insert("hidden_from_pool", doc! { "$ne": true });
```

（放在 `:126` `filter.insert("account_id", &account_id);` 之后。）

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib contact_count_filters`
Expected: PASS

- [ ] **Step 5: 写 hide_from_pool handler**

在 `src/routes/contacts.rs`（`disable_agent` :919 之后就近）加：

```rust
/// `POST /api/contacts/:id/hide-from-pool`
///
/// 手动把联系人从运营池移除（媒体号等无法自动判定的非目标）。**不删记录**
/// ——删了下次对方发消息 webhook 又会重新建档。改标 doc-only `hidden_from_pool=true`，
/// list_contacts / count_contacts 读时过滤（$ne:true）。单向移除，无恢复端点（YAGNI）。
/// workspace 隔离：filter 带 current_workspace，杜绝跨租户改写（IDOR）。
pub(super) async fn hide_from_pool(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let result = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! { "$set": { "hidden_from_pool": true, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound("contact not found".to_string()));
    }
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}
```

注意：确认 `DateTime` 已在 contacts.rs 顶部 `use`（disable_agent :911 已用 `DateTime::now()`，故已在作用域）。

- [ ] **Step 6: 挂路由**

改 `src/routes/mod.rs:366` 后（`disable-agent` 那行之后）加：

```rust
        .route("/contacts/:id/hide-from-pool", post(hide_from_pool))
```

确认 `hide_from_pool` 在 `use` 列表（同 `enable_agent`/`disable_agent` 的 `pub(super)` 可见，mod.rs 内 `use` contacts 模块处补名）。

- [ ] **Step 7: 编译验证**

Run: `cargo check --tests`
Expected: 0 errors（若报 `hide_from_pool` 未导入，在 mod.rs contacts `use` 处补上）

- [ ] **Step 8: 全量 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed

- [ ] **Step 9: Commit**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "feat(user-ops): 从池移除端点hide-from-pool(doc-only标记不删记录)+list/count读时过滤"
```

---

### Task 4: CSS 修 tab 布局（`.segmented` 下移独占一行）

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx:554`（`.panelHead` 加修饰类 `poolHead`）
- Modify: `frontend/src/styles.css`（新增 `.panelHead.poolHead` 纵向布局 + `.poolHead .segmented` 独占一行）

**Interfaces:**
- Consumes: 现有 `.panelHead`（styles.css:444）、`.poolHeadText`（:1638 `display:grid`）、`.segmented`（:1476 `inline-flex` 圆角胶囊）。
- Produces: `.panelHead.poolHead` 修饰类——只作用于运营池，不影响其它 8 频道复用的 `.panelHead`。

**方案（最小改动，遵守 docs/frontend-design-system.md）：** 不改全局 `.panelHead`（其它频道依赖 `space-between`）。给运营池的 `.panelHead` 加 `poolHead` 修饰类，改为纵向：`flex-direction:column; align-items:stretch`，让 `poolHeadText`（标题 + 2 行副标题）占一整行、`.segmented` 换到下方独占一行，三 button `flex:1` 均分。不新建嵌套 panel、不改色板。

- [ ] **Step 1: JSX 加修饰类**

改 `frontend/src/features/user-ops/legacy.tsx:554`：

```jsx
      <div className="panelHead poolHead">
```

（其余 poolHeadText/segmented 结构不变。）

- [ ] **Step 2: 加 CSS**

在 `frontend/src/styles.css` 的 `.poolHeadText`（:1638）附近加：

```css
/* 运营池头部：标题块 + 三档 tab 纵向堆叠。全局 .panelHead 是 space-between 横行，
   在智能模式 ~360px 窄左栏里会把 .segmented 挤成竖排畸形椭圆，故运营池单独纵向。 */
.panelHead.poolHead {
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
}

.panelHead.poolHead .segmented {
  display: flex;
  width: 100%;
}

.panelHead.poolHead .segmented button {
  flex: 1;
  text-align: center;
  white-space: nowrap;
}
```

- [ ] **Step 3: 前端构建验证**

Run: `cd frontend && npm run build`
Expected: build 成功，无 TS/CSS 报错

- [ ] **Step 4: 前端契约测试（结构未回归）**

Run: `cd frontend && npx vitest run contactsView`
Expected: PASS（三 tab 文案「待启用/Agent/全部」计数断言仍绿——纯 CSS+className 改动不动文案）

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/user-ops/legacy.tsx frontend/src/styles.css
git commit -m "fix(user-ops): 运营池三档tab下移独占一行(.poolHead纵向),窄栏不再挤成畸形椭圆"
```

---

### Task 5: 前端「从池移除」按钮 + store action（媒体号手动移除）

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`（`UserOpsActions` interface 加 `hideFromPool` 签名 :114 附近；实现 :499 附近，batchEnable 之后）
- Modify: `frontend/src/features/user-ops/index.tsx:294`（ContactsView 注入 `onHideFromPool`）
- Modify: `frontend/src/features/user-ops/legacy.tsx`（props 加 `onHideFromPool?`；行尾加「从池移除」按钮）
- Test: `frontend/src/__tests__/features/user-ops/contactsView.test.tsx`

**Interfaces:**
- Consumes: `api.post`（api.ts:58）；`refreshContacts`（userOpsStore.ts:287）；`loadContactCounts`（:461）；`toast`（index.tsx 已 import）。
- Produces: store `hideFromPool(accountId: string, contactId: string): Promise<void>`；ContactsView prop `onHideFromPool?: (contact: Contact) => void`。

- [ ] **Step 1: store 加 action 签名 + 实现**

在 `frontend/src/stores/userOpsStore.ts` `UserOpsActions` interface（:114 `loadContactCounts` 附近）加：

```typescript
  hideFromPool: (accountId: string, contactId: string) => Promise<void>;
```

在实现区 `batchEnable`（:494-499）之后加：

```typescript
  // 手动把联系人从运营池移除（媒体号等无法自动判定的非目标）。调后端标记
  // hidden_from_pool（不删记录），成功后刷新列表 + 计数。
  hideFromPool: async (accountId, contactId) => {
    await api.post(`/api/contacts/${encodeURIComponent(contactId)}/hide-from-pool`);
    await refreshContacts(accountId || null, get().searchQuery);
    await get().loadContactCounts(accountId);
  },
```

- [ ] **Step 2: ContactsView props 加回调**

改 `frontend/src/features/user-ops/legacy.tsx:508` 附近（`onBatchEnable?` 之后）加 prop：

```typescript
  onHideFromPool?: (contact: Contact) => void;
```

确认组件参数解构处（约 :464-508 props 列表）加入 `onHideFromPool`。

- [ ] **Step 3: 行尾加「从池移除」按钮**

改 `frontend/src/features/user-ops/legacy.tsx:672-684`——在现有 `selectable && 启用 Agent 按钮` 之后、行 `</div>`（:685）之前加。仅当传了 `onHideFromPool` 时渲染，二次确认防误点：

```jsx
                {onHideFromPool && (
                  <button
                    type="button"
                    className="poolHideBtn"
                    title="从运营池移除（不影响好友关系）"
                    onClick={(event) => {
                      event.stopPropagation();
                      if (window.confirm(`把「${name}」从运营池移除？（不删好友，仅不再出现在池中）`)) {
                        onHideFromPool(contact);
                      }
                    }}
                  >
                    从池移除
                  </button>
                )}
```

- [ ] **Step 4: 加按钮 CSS（低调行尾样式）**

在 `frontend/src/styles.css` `.contact` 相关区（:1548 附近）加：

```css
.poolHideBtn {
  border: 1px solid var(--line);
  background: transparent;
  color: var(--muted);
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 6px;
  white-space: nowrap;
}

.poolHideBtn:hover {
  color: var(--ink);
  background: var(--surface-soft);
}
```

- [ ] **Step 5: index.tsx 注入回调**

改 `frontend/src/features/user-ops/index.tsx:294`（`onBatchEnable` 闭合之后、`/>` 之前）加。先在组件顶部 store 解构处（:122 附近 `loadContacts` 那组）加 `hideFromPool`：

```typescript
    hideFromPool,
```

再在 ContactsView JSX 注入：

```jsx
            onHideFromPool={async (contact) => {
              if (!effectiveAccountId) return;
              try {
                await hideFromPool(effectiveAccountId, contact.id);
                toast.success("已从运营池移除");
              } catch (e) {
                toast.error(e instanceof Error ? e.message : "移除失败");
              }
            }}
```

- [ ] **Step 6: 写契约测试**

在 `frontend/src/__tests__/features/user-ops/contactsView.test.tsx` 加：

```tsx
it("传 onHideFromPool 时行尾有「从池移除」按钮，点击弹确认后调回调", () => {
  const contacts = [
    {
      id: "c1", wxid: "wxid_media", nickname: "福州晚报", agentStatus: "normal",
      lastInboundPreview: "[链接]", tags: [], operationPolicy: {}, profileAttributes: {},
      updatedAt: "2026-07-11T00:00:00Z"
    }
  ] as any;
  const onHideFromPool = vi.fn();
  const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
  render(
    <ContactsView
      {...baseProps}
      contactTab="normal"
      contacts={contacts}
      onBatchEnable={vi.fn().mockResolvedValue(undefined)}
      onHideFromPool={onHideFromPool}
    />
  );
  screen.getByText("从池移除").click();
  expect(confirmSpy).toHaveBeenCalled();
  expect(onHideFromPool).toHaveBeenCalledWith(contacts[0]);
  confirmSpy.mockRestore();
});

it("预览为标签时行内渲染标签而非 XML", () => {
  const contacts = [
    {
      id: "c2", wxid: "wxid_x", nickname: "某公众号内容", agentStatus: "normal",
      lastInboundPreview: "[链接]", tags: [], operationPolicy: {}, profileAttributes: {},
      updatedAt: "2026-07-11T00:00:00Z"
    }
  ] as any;
  render(<ContactsView {...baseProps} contactTab="normal" contacts={contacts} onBatchEnable={vi.fn()} />);
  expect(screen.getByText("[链接]")).toBeInTheDocument();
  expect(screen.queryByText(/<msg>|<appmsg|<sysmsg/)).toBeNull();
});
```

- [ ] **Step 7: 运行前端测试**

Run: `cd frontend && npx vitest run contactsView`
Expected: PASS（原有测试 + 新增 2 测试全绿）

- [ ] **Step 8: 前端构建**

Run: `cd frontend && npm run build`
Expected: 成功，无 TS 报错

- [ ] **Step 9: Commit**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/index.tsx frontend/src/features/user-ops/legacy.tsx frontend/src/styles.css frontend/src/__tests__/features/user-ops/contactsView.test.tsx
git commit -m "feat(user-ops): 运营池每行「从池移除」按钮(二次确认+乐观刷新)手动清媒体号"
```

---

### Task 6: Playwright after 截图验证（对比 pool_before.png）

**Files:**
- Modify: `frontend/src/pool_harness.tsx`（mock 数据补 `onHideFromPool`，确保系统号 mock 仍在以验证——注意 harness 直渲 ContactsView，不走后端过滤，故系统号会显示，验证重点是 tab 布局 + 预览标签 + 从池移除按钮存在）
- Use: `scripts/e2e/pool_shot.py`（现有，截 `section.panel`）

**Interfaces:**
- Consumes: harness 挂载的 ContactsView（pool_harness.tsx）；`scripts/with_server.py`（webapp-testing helper）。

- [ ] **Step 1: harness 补 onHideFromPool（验证按钮渲染）**

改 `frontend/src/pool_harness.tsx:36`（`onBatchEnable` 之后）加：

```jsx
          onHideFromPool={(c) => console.log("hide", c.wxid)}
```

并把 mock 预览改成标签形态验证渲染（`lastInboundPreview` 从 XML 改成后端会产出的标签，模拟真实 API 返回）：把 id:2/3/4 的 `lastInboundPreview` 改为 `"[链接]"`/`"[系统消息]"`/`"[链接]"`。

- [ ] **Step 2: 跑 after 截图**

Run:
```bash
python "C:\Users\jsjm\.claude\skills\webapp-testing\scripts\with_server.py" --server "cd frontend && npm run dev" --port 5173 -- python scripts/e2e/pool_shot.py pool_after.png
```
Expected: 生成 `pool_after.png`，stdout 无 PAGE ERRORS

- [ ] **Step 3: 人工核对 after 图（Read 截图）**

Read `pool_after.png`，逐项确认（对比 `pool_before.png`）：
1. 三档 tab（待启用/Agent/全部）**正常横排**，不再竖排畸形椭圆。
2. 预览显示 `[链接]`/`[系统消息]` 标签，**无** `<msg>`/`<appmsg`/`<sysmsg` XML。
3. 每行有「从池移除」按钮。

若任一项未达成，回对应 Task（1/4/5）修正后重截。

- [ ] **Step 4: 记录验证结论**

在本 plan 或 commit message 记录 after 图三项达成。**不 commit harness/截图产物**（临时验证物，见 Task 7 清理）。

---

### Task 7: 全量验证门 + 清理临时产物

**Files:**
- Delete: 临时 harness 与失败脚本（不 ship）。

- [ ] **Step 1: 后端全量门**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed

Run: `cargo check --tests`
Expected: 0 errors

- [ ] **Step 2: 前端全量门**

Run: `cd frontend && npx vitest run`
Expected: 全绿

Run: `cd frontend && npm run build`
Expected: 成功

- [ ] **Step 3: 文案门**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: exit 0（「从池移除」「[系统消息]」等不含禁用词）

- [ ] **Step 4: 清理临时产物**

删除临时验证脚手架（这些**绝不 ship**）：

```bash
git rm --cached frontend/pool_harness.html frontend/src/pool_harness.tsx 2>/dev/null || true
rm -f frontend/pool_harness.html frontend/src/pool_harness.tsx
rm -f scripts/e2e/pool_shot.py scripts/e2e/pool_repro.mjs scripts/e2e/pool_repro.py scripts/e2e/pool_diag.py scripts/e2e/pool_head_harness.html
rm -f pool_before.png pool_after.png
```

（若其中某些从未 git add 过，`git rm --cached` 报错可忽略，`rm -f` 兜底删工作区文件。）

- [ ] **Step 5: 确认无临时物残留在 git**

Run: `git status --short | grep -i "pool_harness\|pool_shot\|pool_before\|pool_after\|pool_repro\|pool_diag"`
Expected: 无输出（临时物已清）

- [ ] **Step 6: Commit 清理**

```bash
git add -u
git commit -m "chore(user-ops): 清理运营池显示修复的临时Playwright验证脚手架"
```

---

## Self-Review

**1. Spec coverage：**
- spec 改动1（CSS tab）→ Task 4 ✓
- spec 改动2（预览标签化）→ Task 1 ✓（映射表逐类对齐 classify_inbound_msg_type）
- spec 改动3（系统号判据扩展）→ Task 2 ✓（抽 is_system_account 同源，webhooks 复用）
- spec 改动4（hidden_from_pool 端点 + list/count 过滤 + 前端入口）→ Task 3（后端）+ Task 5（前端）✓
- spec 测试要求（纯函数单测 / 前端契约 / Playwright before-after）→ Task 1/2/5 单测 + Task 6 截图 ✓
- spec 验证门（lib≥350 / check --tests / build / vitest / 文案门）→ Task 7 ✓

**偏差说明（对 spec 的合理优化，已亲验依据）：** spec 改动4 原写「Contact 加 `#[serde(default)] pub hidden_from_pool: bool`」。实现改为 **doc-only**（不进 struct）——依据：Contact 无 `deny_unknown_fields`（models.rs:148）+ contacts 无 `replace_one` 全 struct 写路径（webhook/enable/disable 全 `$set`）。doc-only 既满足「标记非删 + list/count 过滤」全部语义，又避免 160 处 `Contact{}` 构造点编译改动（config-field trap）。行为等价、风险更低。集成测试（#[ignore] 待 CI Docker）留待部署前 CI 覆盖，本地纯函数 + filter 单测已锁核心口径。

**2. Placeholder scan：** 无 TBD/TODO；每个改码 step 都给了完整代码块与确切命令 + 预期输出。

**3. Type consistency：**
- `preview_label_for_type(msg_type: Option<&str>, content: &str) -> String`：Task 1 定义，Task 1 Step 5 调用签名一致（`msg.msg_type.as_deref()`, `&msg.content`）。
- `is_system_account(wxid: &str) -> bool`：Task 2 定义（mcp.rs pub(crate)），Task 2 Step 5 webhooks 调 `crate::mcp::is_system_account(wxid)` 一致。
- `hideFromPool(accountId, contactId)`：Task 5 store 签名，index.tsx 调 `hideFromPool(effectiveAccountId, contact.id)` 一致；`onHideFromPool?: (contact: Contact) => void`：legacy.tsx prop 与 index.tsx 注入的 `async (contact) => ...` 一致。
- `hidden_from_pool` doc key：Task 3 端点写入、list filter、count filter 三处同名一致。
- `POST /api/contacts/:id/hide-from-pool`：Task 3 挂路由、Task 5 store `api.post` 路径一致。

## Execution Handoff

见对话中的执行选择提示。
