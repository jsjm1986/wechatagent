# 全量通讯录 + 批量托管 + 头像展示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户运营频道内新增「通讯录」视图，把整个微信好友列表（含头像）拉出来供勾选，批量进入 Agent 托管运营；批量托管用一份共享运营备注、异步生成初始画像，不阻塞。

**Architecture:** 后端加 `avatar_url` 全链路 + 两个新端点（`GET /contacts/roster` 拉全量左连接标注、`POST /contacts/batch-enable` 批量 upsert+入队）；批量托管把初始画像生成从同步改成异步 `AgentTask`（kind=`initial_profile`）由 worker 跑；同步单个托管 `enable_agent` 语义不变，两条路径共用抽出的画像落库 helper 防漂移。前端在用户运营频道加通讯录视图（头像网格 + 多选 + 共享备注）。

**Tech Stack:** Rust (Axum) + MongoDB (mongodb crate, BSON serde) + 外部 MCP JSON-RPC（Streamable-HTTP）+ React 19 + Vite + TypeScript + zustand + CSS Modules。

## Global Constraints

- **无人工接管红线**：新增前端文案 / 状态标签 / 后端字符串**不得**含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`（`scripts/check-no-human-takeover.{sh,ps1}` 扫 `git diff` 新增行，覆盖 `src/agent/` `src/routes/` `src/evolution/` `frontend/src/`）。批量托管、异步画像的措辞用 AI 自治口径（如「已加入 Agent 运营」「画像后台生成中」）。
- **测试基线不得回退**：`cargo test --lib` ≥ 350 passed 0 failed；四个 PBT 文件（`state_transition_pbt` / `memory_card_invariants` / `wiki_chunk_revision_pbt` / `llm_retry_jitter`）累计 ≥ 33 passed 0 failed。新增测试只增量叠加，绝不删改旧维度。
- **workspace 隔离 fail-closed**：所有 contacts 端点经 `AuthenticatedAdmin`，DB 查询恒含 `workspace_id` 过滤；account 必须属于当前 workspace。
- **本地磁盘纪律**：本地只跑 `cargo test --lib` 与单个 PBT（`cargo test --test <name>`）；重的 `#[ignore]` 集成套件留 CI。撞盘先删 `target/debug/incremental`。
- **前端约定**：CSS Modules（`.module.css` + `import styles`）+ 相对 import + `lib/api` + zustand。**不用** `@/components/ui/*` UI 库，**不用** `@/` 路径别名（tsc/vite 无此别名，会编译失败）。wire 键 camelCase。
- **提交纪律**：每个 Task 末尾 commit；只提交本 Task 具名文件，不 `git add -A`。commit message 末尾带 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **子 agent**：若派 subagent 实现，`model` 参数省略（继承主会话 opus）；指令须要求先读码验证再改、产出带 file:line 证据。

---

## File Structure

**后端（修改）**
- `src/models.rs` — `Contact`（:138）加 `avatar_url`；`WechatAccount`（:58）加 `avatar_url`；`ApiContact`（:3276）加 `avatar_url` + `From` 映射（:3318）。
- `src/mcp.rs` — 新增 `fetch_roster_for_account`（调 `contacts_fetch_cache`，解析 items + 头像 key fallback）。
- `src/routes/contacts.rs` — 新增 `roster_endpoint`、`batch_enable_endpoint`、`handle_initial_profile_task`（`pub`，供 tasks.rs 调）；抽出 `apply_generated_profile_to_contact`（`pub(super)`）并让 `enable_agent` 复用。
- `src/routes/mod.rs` — 挂载两个新路由；导出新 handler。
- `src/tasks.rs` — worker 分发加 `kind == "initial_profile"` 分支。

**前端（修改/新增）**
- `frontend/src/types/index.ts` — `Contact` 加 `avatarUrl?`；新增 `RosterEntry`。
- `frontend/src/stores/userOpsStore.ts` — 加 `loadRoster` / `batchEnable`。
- `frontend/src/features/user-ops/RosterView.tsx`（新）+ `RosterView.module.css`（新）。
- `frontend/src/features/user-ops/index.tsx` — 接入通讯录视图（第三视图）。
- `frontend/src/features/user-ops/legacy.tsx` — `ContactsView`（:424）行内补头像。

**测试**
- `src/mcp.rs`（`#[cfg(test)]` mod）— roster 解析单测。
- `src/routes/contacts.rs`（`#[cfg(test)]` mod 或既有 tests）— `apply_generated_profile_to_contact` 纯逻辑（set_doc 组装）单测。
- `tests/contacts_batch_enable.rs`（新，`#[ignore]` 需 Docker）— batch-enable 集成测试。
- `frontend/src/__tests__/features/user-ops/roster.test.tsx`（新）— vitest。

---

## Task 1: `avatar_url` 全链路（模型 + DTO）

**Files:**
- Modify: `src/models.rs:138`（Contact）、`src/models.rs:58`（WechatAccount）、`src/models.rs:3276`（ApiContact struct）、`src/models.rs:3318`（`impl From<Contact> for ApiContact`）

**Interfaces:**
- Produces: `Contact.avatar_url: Option<String>`、`WechatAccount.avatar_url: Option<String>`、`ApiContact.avatar_url: Option<String>`（DTO 序列化为 `avatarUrl`，因 ApiContact 若带 `#[serde(rename_all="camelCase")]`——确认后照办）。

- [ ] **Step 1: 写失败测试（Contact serde 默认值向后兼容）**

在 `src/models.rs` 底部的 `#[cfg(test)]` 区（若无则新建 `mod avatar_field_tests`）加：

```rust
#[cfg(test)]
mod avatar_field_tests {
    use super::Contact;

    #[test]
    fn contact_deserializes_without_avatar_url_field() {
        // 旧文档没有 avatar_url，必须 serde(default) 反序列化为 None（向后兼容红线）。
        let json = serde_json::json!({
            "workspace_id": "w", "account_id": "a", "wxid": "wxid_1",
            "agent_status": "normal"
        });
        let doc = mongodb::bson::to_document(&json).unwrap();
        let contact: Contact = mongodb::bson::from_document(doc).unwrap();
        assert_eq!(contact.avatar_url, None);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib avatar_field_tests`
Expected: FAIL —编译错误 `no field avatar_url on type Contact`。

- [ ] **Step 3: 加字段**

`Contact`（`src/models.rs:138` 结构体内，紧跟 `alias` 字段后加）：

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
```

`WechatAccount`（`src/models.rs:58` 结构体内，紧跟 `nick_name` 后加）：

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
```

`ApiContact`（`src/models.rs:3276` 结构体内，紧跟 `alias` 后加）：

```rust
    pub avatar_url: Option<String>,
```

`impl From<Contact> for ApiContact`（`src/models.rs:3318`，紧跟 `alias: contact.alias,` 后加）：

```rust
            avatar_url: contact.avatar_url,
```

- [ ] **Step 4: 修所有 WechatAccount / Contact / ApiContact 字面量构造点**

编译会因 `WechatAccount { .. }` / `Contact { .. }` / `ApiContact { .. }` 全字段字面量缺 `avatar_url` 报 E0063。逐个补 `avatar_url: None,`（生产构造点 + tests helper）。定位：

Run: `cargo check --tests 2>&1 | grep -E "missing field .avatar_url|error\[E0063\]"`

对每个报错文件加字段。注意 `src/routes/accounts.rs:87` 的 `WechatAccount { .. }`（sync_accounts）——见 Task 3 会回填真实头像，此处先 `avatar_url: None,`。

- [ ] **Step 5: 跑测试确认通过 + 全 lib 基线**

Run: `cargo test --lib avatar_field_tests && cargo test --lib 2>&1 | tail -3`
Expected: avatar 测试 PASS；lib 总数 ≥ 350 passed 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/routes/accounts.rs
# 若 Step4 还改了其它文件，一并具名 add
git commit -m "$(cat <<'EOF'
feat(models): 补 avatar_url 全链路(Contact/WechatAccount/ApiContact)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: MCP 全量好友拉取 + 头像解析

**Files:**
- Modify: `src/mcp.rs`（新增 `fetch_roster_for_account` + `#[cfg(test)]` 解析单测）

**Interfaces:**
- Consumes: `logged_call_for_account(state, account_id, tool_name, arguments)`（`src/mcp.rs:319`，已自动注入 `account_alias`）。
- Produces:
  ```rust
  pub struct RosterFriend {
      pub wxid: String,
      pub nickname: Option<String>,
      pub remark: Option<String>,
      pub avatar_url: Option<String>,
  }
  pub async fn fetch_roster_for_account(state: &AppState, account_id: &str) -> AppResult<Vec<RosterFriend>>;
  // 纯解析（供单测，不碰网络）：
  fn parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend>;
  ```

**背景（已核实）**：MCP 好友类工具真实名为 `contact_list`（主）/ `im_sync`（备选），**非** `contacts_fetch_cache`（来源：2026-07-07 拉取 `http://117.72.54.28:3001/mcp-guide.html` 工具清单）。指南页未列出参 schema，头像字段真实 key 当日无法确认（探测时网关 502）。故解析器对头像 key 用**优先级 fallback**（`bigHeadImg` → `smallHeadImg` → `headImgUrl` → `avatarUrl` → `headimgurl`），对数组路径也做多候选（`structuredContent.contacts` / `structuredContent.friends` / `structuredContent.list` / 顶层 `contacts` / `content[0].text` 内嵌 JSON），取第一个非空。**实现前若 MCP 已恢复，先打一次真实 `contact_list` 调用把真实 key 提到 fallback 列表首位并在此注释记录。**

- [ ] **Step 1: 写失败测试（解析器对多种 key 形态都能提取）**

在 `src/mcp.rs` 底部 `#[cfg(test)]` 区加：

```rust
#[cfg(test)]
mod roster_parse_tests {
    use super::parse_roster_items;

    #[test]
    fn parses_structured_contacts_with_big_head_img() {
        let v = serde_json::json!({
            "structuredContent": { "contacts": [
                { "wxid": "wxid_a", "nickName": "小明", "remark": "客户A", "bigHeadImg": "http://img/a" },
                { "userName": "wxid_b", "nickname": "小红", "smallHeadImg": "http://img/b" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_a");
        assert_eq!(out[0].nickname.as_deref(), Some("小明"));
        assert_eq!(out[0].remark.as_deref(), Some("客户A"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/a"));
        // 第二条用 userName 作 wxid、smallHeadImg 作头像。
        assert_eq!(out[1].wxid, "wxid_b");
        assert_eq!(out[1].avatar_url.as_deref(), Some("http://img/b"));
    }

    #[test]
    fn skips_entries_without_wxid() {
        let v = serde_json::json!({ "contacts": [ { "nickName": "无id" } ] });
        assert_eq!(parse_roster_items(&v).len(), 0);
    }

    #[test]
    fn returns_empty_on_unknown_shape() {
        let v = serde_json::json!({ "unexpected": true });
        assert_eq!(parse_roster_items(&v).len(), 0);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib roster_parse_tests`
Expected: FAIL —`cannot find function parse_roster_items`。

- [ ] **Step 3: 实现解析器 + 拉取函数**

在 `src/mcp.rs`（`logged_call_for_account` 附近，非 test 区）加：

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct RosterFriend {
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub avatar_url: Option<String>,
}

fn first_str<'a>(obj: &'a serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend> {
    // 数组路径多候选：结构化内容优先，其次顶层，最后 content[0].text 内嵌 JSON。
    let arr = result
        .pointer("/structuredContent/contacts")
        .or_else(|| result.pointer("/structuredContent/friends"))
        .or_else(|| result.pointer("/structuredContent/list"))
        .or_else(|| result.get("contacts"))
        .or_else(|| result.get("friends"))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| {
            // content[0].text 内嵌 JSON 字符串形态。
            let text = result.pointer("/content/0/text")?.as_str()?;
            let inner: serde_json::Value = serde_json::from_str(text).ok()?;
            inner
                .pointer("/contacts")
                .or_else(|| inner.get("friends"))
                .and_then(|v| v.as_array())
                .cloned()
        })
        .unwrap_or_default();

    arr.iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let wxid = first_str(obj, &["wxid", "userName", "UserName", "username"])?;
            Some(RosterFriend {
                wxid,
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
            })
        })
        .collect()
}

pub async fn fetch_roster_for_account(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<RosterFriend>> {
    // contacts_fetch_cache 是全量好友工具（线上 tools/list 亲验；contact_list 不存在）。
    let result =
        logged_call_for_account(state, account_id, "contacts_fetch_cache", serde_json::json!({}))
            .await?;
    Ok(parse_roster_items(&result))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib roster_parse_tests`
Expected: 3 tests PASS。

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs
git commit -m "$(cat <<'EOF'
feat(mcp): contacts_fetch_cache 全量好友拉取 + 头像字段 fallback 解析

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `GET /api/contacts/roster` 端点

**Files:**
- Modify: `src/routes/contacts.rs`（新增 `roster_endpoint`）、`src/routes/mod.rs`（导出 + 挂路由）

**Interfaces:**
- Consumes: `mcp::fetch_roster_for_account`（Task 2）、`validate_account`（`src/routes/shared.rs:138`）。
- Produces: `GET /api/contacts/roster?accountId=<id>` → `{ "items": [{ wxid, nickname, remark, avatarUrl, agentStatus }], "total": <n> }`。`agentStatus ∈ { "managed", "normal", "not_imported" }`。

- [ ] **Step 1: 实现 handler**

在 `src/routes/contacts.rs`（`enable_agent` 之前，import 区确保有 `use serde::Deserialize;` 及 `crate::mcp`）加：

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RosterQuery {
    pub account_id: String,
}

pub(super) async fn roster_endpoint(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    axum::extract::Query(query): axum::extract::Query<RosterQuery>,
) -> AppResult<Json<Value>> {
    // account 必须属于当前 workspace（fail-closed）。
    validate_account(&state, &admin.current_workspace, &query.account_id).await?;

    let friends = crate::mcp::fetch_roster_for_account(&state, &query.account_id).await?;

    // 本地已入库联系人：wxid -> agent_status。
    let mut cursor = state
        .db
        .contacts()
        .find(
            doc! { "workspace_id": &admin.current_workspace, "account_id": &query.account_id },
            None,
        )
        .await?;
    let mut status_by_wxid: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    use futures::TryStreamExt;
    while let Some(c) = cursor.try_next().await? {
        let status = match c.agent_status {
            crate::models::AgentStatus::Managed => "managed",
            _ => "normal",
        };
        status_by_wxid.insert(c.wxid, status.to_string());
    }

    let items: Vec<Value> = friends
        .into_iter()
        .map(|f| {
            let agent_status = status_by_wxid
                .get(&f.wxid)
                .cloned()
                .unwrap_or_else(|| "not_imported".to_string());
            json!({
                "wxid": f.wxid,
                "nickname": f.nickname,
                "remark": f.remark,
                "avatarUrl": f.avatar_url,
                "agentStatus": agent_status,
            })
        })
        .collect();
    let total = items.len();
    Ok(Json(json!({ "items": items, "total": total })))
}
```

> 注：确认 `AgentStatus` 变体名——若非 `Managed`，`cargo check` 会报错，按真实变体名改（`src/models.rs` grep `enum AgentStatus`）。

- [ ] **Step 2: 挂路由 + 导出**

`src/routes/mod.rs`：`use contacts::{...}`（:172 起）加入 `roster_endpoint`；路由链（`.route("/contacts", ...)` :344 附近）加：

```rust
        .route("/contacts/roster", get(roster_endpoint))
```

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 0 errors（如报 `AgentStatus` 变体名或 `Query` 导入，按提示修）。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "$(cat <<'EOF'
feat(contacts): GET /contacts/roster 拉全量好友+本地左连接标注 agentStatus

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: 抽出 `apply_generated_profile_to_contact` 并让 `enable_agent` 复用

**Files:**
- Modify: `src/routes/contacts.rs`（抽函数 + 重构 `enable_agent` :361-463）

**Interfaces:**
- Consumes: `is_previously_operated`（shared.rs:134）、`validate_generated_stage_intent`（contacts.rs:778）、`insert_domain_stage_fields`（shared.rs:93）、`commitments_with_optional_text`（shared.rs:1376）、`agent::load_user_operation_domain_config`、`agent::initial_operation_state_key`、`GeneratedOperationProfile`（types.rs:23）。
- Produces:
  ```rust
  pub(super) async fn apply_generated_profile_to_contact(
      state: &AppState,
      workspace_id: &str,
      contact: &Contact,               // 已加载(取 _id / account_id / commitments / is_previously_operated)
      note: &str,                      // human_profile_note
      playbook_id: Option<ObjectId>,
      playbook_version: Option<i32>,
      generated: &GeneratedOperationProfile,
  ) -> AppResult<()>;
  ```

> **动机**：批量异步路径（Task 5）与同步 `enable_agent` 若各写一份画像落库逻辑必然漂移（stage/intent 校验、老客户保留、operation_state 初始态）。抽成一个函数两路共用。这是本计划**唯一重构**，直接服务批量目标。

- [ ] **Step 1: 写函数（把 enable_agent :403-461 的落库逻辑原样搬入）**

在 `src/routes/contacts.rs` 加（`enable_agent` 之上）：

```rust
pub(super) async fn apply_generated_profile_to_contact(
    state: &AppState,
    workspace_id: &str,
    contact: &Contact,
    note: &str,
    playbook_id: Option<ObjectId>,
    playbook_version: Option<i32>,
    generated: &GeneratedOperationProfile,
) -> AppResult<()> {
    let object_id = contact.id.ok_or_else(|| AppError::BadRequest("contact missing _id".into()))?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    let mut set_doc = doc! {
        "agent_status": "managed",
        "human_profile_note": note,
        "agent_profile": to_bson(&generated.agent_profile)?,
        "playbook_id": playbook_id,
        "playbook_version": playbook_version,
        "profile_attributes": generated.profile_attributes.clone(),
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = Document::new();
    if !is_previously_operated(contact) {
        let domain_config =
            agent::load_user_operation_domain_config(state, workspace_id).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        let (gen_stage, gen_intent) = validate_generated_stage_intent(
            state,
            &contact.account_id,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
        )
        .await?;
        insert_domain_stage_fields(&mut set_doc, gen_stage.as_deref(), gen_intent.as_deref(), true);
        set_doc.insert("commitments", commitments_bson);
        set_doc.insert("follow_up_policy", generated.follow_up_policy.clone());
        set_doc.insert("operation_state", initial_state);
        set_doc.insert("operation_state_reason", "初次纳入 Agent 运营，等待后续互动确认阶段");
        set_doc.insert("operation_state_confidence", 6);
        set_doc.insert("operation_state_updated_at", DateTime::now());
        unset_doc.insert("last_commitment", "");
    }
    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }
    state
        .db
        .contacts()
        .update_one(doc! { "_id": object_id }, update_doc, None)
        .await?;
    Ok(())
}
```

- [ ] **Step 2: 重构 `enable_agent` 调用新函数**

`enable_agent`（contacts.rs:361）中把 `:403-461`（`commitments_bson` 到 `update_one`）整段替换为：

```rust
    apply_generated_profile_to_contact(
        &state,
        &admin.current_workspace,
        &contact,
        &payload.human_profile_note,
        playbook.id,
        playbook.version,
        &generated,
    )
    .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
```

保留 `:372-402`（object_id 解析、account 注册校验、playbook 解析、`build_initial_operation_profile`）不动。删掉不再用的局部 `object_id`/`commitments_bson` 变量（编译告警会指出）。

- [ ] **Step 3: 跑既有 enable_agent 相关测试确认零行为变化**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: lib ≥ 350 passed 0 failed（既有测试守住 enable_agent 行为——这是重构不改语义的证据）。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contacts.rs
git commit -m "$(cat <<'EOF'
refactor(contacts): 抽出 apply_generated_profile_to_contact,enable_agent 复用

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: 异步初始画像任务（handler + worker 分发）

**Files:**
- Modify: `src/routes/contacts.rs`（新增 `pub async fn handle_initial_profile_task`）、`src/tasks.rs`（:230-236 加分支）

**Interfaces:**
- Consumes: `AgentTask`（models.rs:814，字段 `account_id`/`contact_wxid`/`content`）、`resolve_playbook_for_contact`（shared.rs:429）、`agent::build_initial_operation_profile`（decision.rs:48）、`apply_generated_profile_to_contact`（Task 4）。
- Produces: `pub async fn handle_initial_profile_task(state: &AppState, task: &crate::models::AgentTask) -> AppResult<()>`。

> **落点说明（相对 spec 的细化）**：spec §3b 写 `agent::handle_initial_profile_task`，但画像落库依赖的 helper（`apply_generated_profile_to_contact` 等）都在 `routes` 模块且 `pub(super)`。为不破坏可见性、不重复落库逻辑，handler 放 `routes::contacts` 并标 `pub`，`tasks.rs` 以 `crate::routes::contacts::handle_initial_profile_task` 调用。`routes/mod.rs:46` 已是 `pub mod contacts`，可达。

- [ ] **Step 1: 实现 handler**

在 `src/routes/contacts.rs` 加（`pub`，因跨模块调用）：

```rust
pub async fn handle_initial_profile_task(
    state: &AppState,
    task: &crate::models::AgentTask,
) -> AppResult<()> {
    // 按 wxid+account 定位联系人（worker 上下文没有 _id）。
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "wxid": &task.contact_wxid,
            },
            None,
        )
        .await?;
    let Some(contact) = contact else {
        // 联系人已被删/清理：视为完成，不报错重试。
        return Ok(());
    };
    // 批量后又被手动取消托管的，跳过画像回填。
    if !matches!(contact.agent_status, crate::models::AgentStatus::Managed) {
        return Ok(());
    }
    let playbook = resolve_playbook_for_contact(
        state,
        &task.workspace_id,
        &task.account_id,
        None,
    )
    .await?;
    let generated = agent::build_initial_operation_profile(
        state,
        &task.workspace_id,
        &task.content, // = sharedNote
        Some(&playbook),
    )
    .await?;
    apply_generated_profile_to_contact(
        state,
        &task.workspace_id,
        &contact,
        &task.content,
        playbook.id,
        playbook.version,
        &generated,
    )
    .await
}
```

> 注：`resolve_playbook_for_contact` 传 `None` 用账号默认 playbook；批量若指定了 playbookId，见 Task 6 会把 playbook_id 通过 contact.playbook_id 预置，此处可改为读 `contact.playbook_id`。**为简单起见 batch-enable 用统一 playbook 时在入队前已 upsert 到 contact.playbook_id**——故这里应传 `contact.playbook_id.map(|o| o.to_hex()).as_deref()`。修正调用：

```rust
    let playbook = resolve_playbook_for_contact(
        state,
        &task.workspace_id,
        &task.account_id,
        contact.playbook_id.map(|o| o.to_hex()).as_deref(),
    )
    .await?;
```

- [ ] **Step 2: worker 分发加分支**

`src/tasks.rs:230-236` 改为：

```rust
        let result = if task.kind == "memory_consolidation" {
            agent::handle_memory_consolidation_task(state, task).await
        } else if task.kind == "outcome_aggregation" {
            handle_outcome_aggregation_task(state, task).await
        } else if task.kind == "initial_profile" {
            crate::routes::contacts::handle_initial_profile_task(state, &task).await
        } else {
            agent::handle_follow_up_task(state, task).await
        };
```

> 注：确认 `task` 在此作用域是值还是引用——现有分支 `handle_follow_up_task(state, task)` 传的是 `task`（值/move）。若 `handle_initial_profile_task` 需 `&task` 而后续分支已 move，调整为在 `initial_profile` 分支内先用引用（该分支是 if/else 互斥，不冲突）。按 `cargo check` 实际报错修正所有权。

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 0 errors。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contacts.rs src/tasks.rs
git commit -m "$(cat <<'EOF'
feat(agent): initial_profile 异步任务,worker 分发回填批量托管画像

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `POST /api/contacts/batch-enable` 端点

**Files:**
- Modify: `src/routes/contacts.rs`（新增 `batch_enable_endpoint`）、`src/routes/mod.rs`（挂路由 + 导出）、`src/models.rs`（新增请求体 `BatchEnableRequest`）
- Test: `tests/contacts_batch_enable.rs`（新，`#[ignore]`）

**Interfaces:**
- Consumes: `WechatAccount`（校验注册）、`AgentTask`（入队）、`AgentStatus`。
- Produces: `POST /api/contacts/batch-enable`，body `{ accountId, candidates: [{ wxid, nickname?, remark?, avatarUrl? }], sharedNote, playbookId? }` → `{ "enabled": <n>, "queued": <n> }`。

- [ ] **Step 1: 加请求体结构**

`src/models.rs`（`EnableAgentRequest` 附近 :3192）加：

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEnableCandidate {
    pub wxid: String,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEnableRequest {
    pub account_id: String,
    pub candidates: Vec<BatchEnableCandidate>,
    pub shared_note: String,
    #[serde(default)]
    pub playbook_id: Option<String>,
}
```

- [ ] **Step 2: 实现 handler**

`src/routes/contacts.rs` 加：

```rust
pub(super) async fn batch_enable_endpoint(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<crate::models::BatchEnableRequest>,
) -> AppResult<Json<Value>> {
    if payload.shared_note.trim().is_empty() {
        return Err(AppError::BadRequest("sharedNote is required".to_string()));
    }
    if payload.candidates.is_empty() {
        return Err(AppError::BadRequest("candidates is empty".to_string()));
    }
    // account 必须在 wechat_accounts 注册(否则 webhook 入站会被 resolve_account_context 拒收)。
    if state
        .db
        .accounts()
        .find_one(
            doc! { "workspace_id": &admin.current_workspace, "account_id": &payload.account_id },
            None,
        )
        .await?
        .is_none()
    {
        return Err(AppError::BadRequest(format!(
            "account_id={} 在 wechat_accounts 中未注册，无法批量启用 Agent 运营",
            payload.account_id
        )));
    }
    // 可选统一 playbook：校验存在(用 enable 的解析器,None 则账号默认)。
    let playbook = resolve_playbook_for_contact(
        &state,
        &admin.current_workspace,
        &payload.account_id,
        payload.playbook_id.as_deref(),
    )
    .await?;

    let mut enabled = 0i32;
    let mut queued = 0i32;
    for cand in &payload.candidates {
        // upsert 联系人:置 managed + sharedNote + avatar + playbook。
        let set_doc = doc! {
            "workspace_id": &admin.current_workspace,
            "account_id": &payload.account_id,
            "wxid": &cand.wxid,
            "nickname": &cand.nickname,
            "remark": &cand.remark,
            "avatar_url": &cand.avatar_url,
            "agent_status": "managed",
            "human_profile_note": &payload.shared_note,
            "playbook_id": playbook.id,
            "playbook_version": playbook.version,
            "updated_at": DateTime::now(),
        };
        // 幂等:已 managed 的不重复入队(但仍刷新 note/avatar)。
        let existing = state
            .db
            .contacts()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "wxid": &cand.wxid,
                },
                None,
            )
            .await?;
        let already_managed = existing
            .as_ref()
            .map(|c| matches!(c.agent_status, crate::models::AgentStatus::Managed))
            .unwrap_or(false);

        state
            .db
            .contacts()
            .update_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "wxid": &cand.wxid,
                },
                doc! {
                    "$set": set_doc,
                    "$setOnInsert": { "created_at": DateTime::now() },
                },
                mongodb::options::UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        enabled += 1;

        if !already_managed {
            // 入队异步初始画像任务。
            state
                .db
                .tasks()
                .insert_one(
                    crate::models::AgentTask {
                        id: None,
                        workspace_id: admin.current_workspace.clone(),
                        account_id: payload.account_id.clone(),
                        contact_wxid: cand.wxid.clone(),
                        kind: "initial_profile".to_string(),
                        run_at: DateTime::now(),
                        expires_at: None,
                        content: payload.shared_note.clone(),
                        status: "pending".to_string(),
                        source_decision_id: None,
                        review_required: false,
                        attempt_count: 0,
                        max_attempts: state.config.task_max_attempts as i32,
                        next_retry_at: None,
                        gateway_status: None,
                        cancel_reason: None,
                        error: None,
                        claimed_at: None,
                        claim_recovery_count: 0,
                        created_at: DateTime::now(),
                        updated_at: DateTime::now(),
                    },
                    None,
                )
                .await?;
            queued += 1;
        }
    }
    Ok(Json(json!({ "enabled": enabled, "queued": queued })))
}
```

> 注：`max_attempts` 用现有 follow_up 入队相同的配置字段——确认字段名（`src/agent/gateway.rs:4514` 的 `AgentTask { .. kind: "follow_up" .. }` 附近看 `max_attempts` 取值），照抄。`AgentTask` 完整字段以 `src/models.rs:814` 为准，缺字段会 E0063，按编译修。

- [ ] **Step 3: 挂路由 + 导出**

`src/routes/mod.rs`：`use contacts::{...}` 加 `batch_enable_endpoint`；路由链加：

```rust
        .route("/contacts/batch-enable", post(batch_enable_endpoint))
```

- [ ] **Step 4: 编译验证**

Run: `cargo check`
Expected: 0 errors。

- [ ] **Step 5: 写集成测试（`#[ignore]`，需 Docker）**

新建 `tests/contacts_batch_enable.rs`，参照现有 contacts 集成测试的 TestApp 建法（grep `tests/` 里已有的 contacts 相关测试作模板）。核心断言：

```rust
// 1) sharedNote 空 → 400
// 2) account 未注册 → 400
// 3) 正常批量:2 个候选 → enabled=2, queued=2;contacts 集合出现 2 条 managed;
//    tasks 集合出现 2 条 kind="initial_profile" status="pending"
// 4) 幂等:对已 managed 的 wxid 再批量一次 → enabled 计数但 queued 不增(不重复入队)
```

标 `#[ignore]`（同其它 testcontainers 用例）。本地不跑，留 CI `--ignored`。

- [ ] **Step 6: 跑可编译 + lib 基线**

Run: `cargo test --lib 2>&1 | tail -3 && cargo test --test contacts_batch_enable -- --list 2>&1 | tail -5`
Expected: lib ≥ 350 passed 0 failed；集成测试可编译（--list 列出用例）。

- [ ] **Step 7: Commit**

```bash
git add src/routes/contacts.rs src/routes/mod.rs src/models.rs tests/contacts_batch_enable.rs
git commit -m "$(cat <<'EOF'
feat(contacts): POST /contacts/batch-enable 批量托管+异步画像入队(幂等)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: 前端 types + store actions

**Files:**
- Modify: `frontend/src/types/index.ts`（`Contact` 加 `avatarUrl?` + 新增 `RosterEntry`）、`frontend/src/stores/userOpsStore.ts`（加 `loadRoster` / `batchEnable`）

**Interfaces:**
- Produces:
  ```ts
  export interface RosterEntry {
    wxid: string;
    nickname?: string | null;
    remark?: string | null;
    avatarUrl?: string | null;
    agentStatus: "managed" | "normal" | "not_imported";
  }
  // store:
  loadRoster(accountId: string): Promise<RosterEntry[]>;
  batchEnable(payload: { accountId: string; candidates: {wxid:string;nickname?:string|null;remark?:string|null;avatarUrl?:string|null}[]; sharedNote: string; playbookId?: string }): Promise<{ enabled: number; queued: number }>;
  ```

- [ ] **Step 1: 加类型**

`frontend/src/types/index.ts`：`Contact` 接口加 `avatarUrl?: string | null;`；文件末尾加 `RosterEntry`（见 Interfaces）。

- [ ] **Step 2: 加 store actions**

`frontend/src/stores/userOpsStore.ts`（参照现有 `loadContacts` :440 / `importContacts` :446 的 `api` 用法）：

```ts
  loadRoster: async (accountId: string) => {
    const data = await api.get<{ items: RosterEntry[] }>(
      `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}`
    );
    return data.items;
  },
  batchEnable: async (payload) => {
    return await api.post<{ enabled: number; queued: number }>(
      "/api/contacts/batch-enable",
      payload
    );
  },
```

在 store 的 state 接口里补这两个方法签名（若该 store 用显式接口）。import `RosterEntry`。

- [ ] **Step 3: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 errors。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts
git commit -m "$(cat <<'EOF'
feat(user-ops): roster/batchEnable store actions + RosterEntry 类型

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: 前端通讯录视图（头像网格 + 多选 + 共享备注）

**Files:**
- Create: `frontend/src/features/user-ops/RosterView.tsx`、`frontend/src/features/user-ops/RosterView.module.css`
- Modify: `frontend/src/features/user-ops/index.tsx`（接第三视图）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`（新）

**Interfaces:**
- Consumes: `loadRoster` / `batchEnable`（Task 7）、account 列表（`useAccountStore`，见 `frontend/src/features/account-management/index.tsx:11`）。

- [ ] **Step 1: 写组件**

`RosterView.tsx`：account 选择器 + 拉全量 + 头像网格（每条头像+昵称/备注+agentStatus 徽标+checkbox，managed 置灰不可选）+ 底部共享备注输入框 + 「加入 Agent 运营」按钮。用 CSS Modules，相对 import，无 `@/` 别名。头像 `<img src={avatarUrl}>` 无值时首字母占位。提交成功 toast「已加入 N 人，画像后台生成中」。全部文案避开红线词表。

（组件完整代码在实现时按现有 `ContactsView` / `AccountManagementFeature` 的样式与 store 用法产出——参照 `legacy.tsx:424` 与 `account-management/index.tsx`。）

- [ ] **Step 2: 接入 index.tsx 第三视图**

`frontend/src/features/user-ops/index.tsx`：在 smart 模式的视图切换里加「通讯录」入口，渲染 `<RosterView />`。

- [ ] **Step 3: 写 vitest 测试**

`roster.test.tsx`：mock fetch 返回 3 条 roster（1 managed / 2 not_imported）→ 断言渲染 3 行 + managed 行 checkbox 禁用；勾选 2 条 + 填备注 + 点按钮 → 断言 POST `/api/contacts/batch-enable` body 含 `candidates`(len 2)、`sharedNote`、camelCase 键。参照 `frontend/src/__tests__/features/knowledge/knowledge.test.tsx` 的 fetch mock 范式。

- [ ] **Step 4: 跑测试 + 类型检查**

Run: `cd frontend && npx tsc --noEmit && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: tsc 0 errors；vitest 全 PASS。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/user-ops/RosterView.tsx frontend/src/features/user-ops/RosterView.module.css frontend/src/features/user-ops/index.tsx frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "$(cat <<'EOF'
feat(user-ops): 通讯录视图(头像网格+多选+共享备注批量托管)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: 现有 ContactsView 行内补头像

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（`ContactsView` :424）+ 其 module.css

- [ ] **Step 1: 加头像小圆图**

`ContactsView` 每行在 name 前加头像：`avatarUrl` 有则 `<img class={styles.avatar} src={...}>`，无则首字母占位圆。加对应 `.avatar` 样式（24-28px 圆）。

- [ ] **Step 2: 类型检查 + 既有测试**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: tsc 0 errors；既有 vitest 全 PASS（无回归）。

- [ ] **Step 3: Commit**

```bash
git add frontend/src/features/user-ops/legacy.tsx
# 若改了 module.css 一并 add
git commit -m "$(cat <<'EOF'
feat(user-ops): ContactsView 联系人行补头像展示

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: 全链路验证 + 红线 lint

- [ ] **Step 1: 后端基线门**

Run: `scripts/check-baseline.sh`（或 `.ps1`）
Expected: lib ≥ 350、PBT 累计 ≥ 33，exit 0。

- [ ] **Step 2: 无人工接管 lint**

Run: `scripts/check-no-human-takeover.sh`（或 `.ps1`）
Expected: exit 0（新增行无红线词）。

- [ ] **Step 3: 前端全量**

Run: `cd frontend && npx tsc --noEmit && npx vitest run && npm run build`
Expected: 全绿，`frontend/dist` 产出。

- [x] **Step 4: MCP 真实字段核对（部署后，线上 tools/list 已亲验）**

2026-07-07 线上 `tools/list` 亲验：工具名 `contact_list` 被证伪（`Forbidden tool`），改用 `contacts_fetch_cache`（已修 `src/mcp.rs`）。⚠️ 头像 / 数组真实 key **仍开放**：线上测试账号缓存为空（`structuredContent:{}`），无 populated 样本。已加内容识别兜底（`contact_like_array`）扛未知数组 key。**残留 TODO**：待某账号缓存非空时，再打一次 `contacts_fetch_cache` 核对 ①真实数组 key ②头像真实 key，若在 fallback 列表外则补入（spec §7）。

---

## Self-Review

**Spec coverage：**
- §1 通讯录第三视图 → Task 8。全量 `contacts_fetch_cache` + 左连接标注 → Task 2 + Task 3。纯浏览不写库 → Task 3（roster 只读）。✓
- §2a avatar_url 全链路 → Task 1。✓
- §2b GET /contacts/roster → Task 3。✓
- §2c POST /contacts/batch-enable（upsert+managed+sharedNote+account校验+异步入队+幂等+老客户保留） → Task 6（老客户保留经 Task 4 的 `is_previously_operated` 分支）。✓
- §2d 单个 enable_agent 不变 → Task 4（仅重构不改语义，既有测试守护）。✓
- §3a initial_profile AgentTask → Task 6 入队 + Task 5 定义。✓
- §3b worker 分发 → Task 5 Step 2。✓（落点 routes::contacts 而非 agent，已在 Task 5 注明理由）
- §3c 画像未就绪窗口沿用既有降级 → 无新代码，符合 YAGNI。✓
- §4a 通讯录视图 → Task 8；§4b ContactsView 头像 → Task 9；§4c store/types → Task 7。✓
- §5 测试 → Task 2/6/8 各含测试 + Task 10 门。✓
- §6 非目标 → 计划无头像转存、无群/朋友圈、无 enable_agent 语义改动、无窗口特判、无分页。✓
- §7 待核对 → Task 2 注释 + Task 10 Step 4。✓

**Placeholder scan：** Task 8 Step 1 的组件完整代码留到实现时按现有组件产出——这是刻意的（要照现有 `ContactsView`/`AccountManagement` 视觉，凭空写死样式反而偏离设计系统）。已给出明确的结构、数据源、红线约束、参照文件。其余步骤均含可执行代码/命令。

**Type consistency：** `RosterFriend`(Rust)↔`RosterEntry`(TS) 字段对齐（wxid/nickname/remark/avatarUrl/agentStatus）；`apply_generated_profile_to_contact` 签名在 Task 4 定义、Task 5 消费一致；`AgentTask` 字段以 models.rs:814 为准，Task 6 入队全字段。`agentStatus` 三值 `managed`/`normal`/`not_imported` 前后端一致。

**待实现者注意的两处编译期确认（已在对应 Task 标注）：** ①`AgentStatus` 枚举变体名（Task 3/5/6 用了 `Managed`，按真实名改）；②`tasks.rs` 分支里 `task` 的所有权（值 vs 引用，按 cargo check 修）。
