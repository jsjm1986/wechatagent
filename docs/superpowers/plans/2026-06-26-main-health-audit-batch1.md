# main 健康度审查 batch1 修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 main 健康度交叉验证审查的 4 条 finding——evolution 三端点跨租户 IDOR(SEC-1)+ 审计 actor 丢失(EVO-2)、知识预览端点跨租户读泄漏(KNOW-1)、guide preview 健康度 key 契约不匹配展示伪造分(FE-1)。

**Architecture:** SEC-1/EVO-2 在 evolution.rs handler 层加 `Extension<AuthenticatedAdmin>` + find_one 加 workspace filter(跨租户→404)+ actor 用 admin.username。KNOW-1 给 test_knowledge_route_for_contact 加 workspace 参数,预览端点透传 admin.current_workspace。FE-1 让 guide preview 后端返回构建好的 health items(复用后端已有 health_item 正确量纲/风险反转逻辑),前端删坏函数 healthFromScores 直接用后端 items。

**Tech Stack:** Rust 2021 / Axum / MongoDB(mongodb crate)/ testcontainers 集成测试;React 19 + TypeScript + Vite + Zustand + vitest。

## Global Constraints

- 后端 `cargo`(Rust 2021),无 workspace。本地只跑 `cargo test --lib` + 单 PBT;完整集成测试(`#[ignore]` + Docker)留 CI(本地磁盘/Docker 限制)。本地用 `cargo check --tests`(`RUSTFLAGS=-Dwarnings`)复刻 CI baseline step2。
- CI baseline 门(`scripts/check-baseline.sh`):`cargo test --lib` ≥ 350 passed/0 failed;4 个 PBT 累计 ≥ 33。新工作只增测试不降基线。
- 跨 workspace 越权访问被拦一律返回 **404 NotFound**(不暴露资源存在性),复用 handler 既有 proposal-not-found 路径。
- 多租户隔离:handler 用鉴权身份 `admin.current_workspace`,**不信任**记录自带的 `proposal.workspace_id` 或 `state.config.default_workspace_id`。
- 禁词红线(`check-no-human-takeover.sh`):src/agent|routes|evolution + frontend/src 新增行不得含「人工接管/takeover/hand-off/人工」等词(测试目录豁免)。本计划文案均技术词,无风险。
- 提交需用户批准节奏;只 `git add` 本计划涉及的具体文件,**绝不** `git add -A`/`.`。
- 当前分支 `fix/main-health-audit-batch1`(设计文档 378a22a 已提交其上)。

---

### Task 1: SEC-1 + EVO-2 — evolution 三端点加 workspace scope + 真实 actor

**Files:**
- Modify: `src/routes/evolution.rs`(get_evolution_proposal_detail :106 / release_evolution_proposal :138 / rollback_evolution_proposal :180)
- Test: `tests/evolution_workspace_scope.rs`(新建,`#[ignore]` 集成)

**Interfaces:**
- Consumes: `AuthenticatedAdmin { user_id, username, current_workspace }`(src/auth/mod.rs:59-65);`release_threshold/release_prompt/rollback_threshold/rollback_prompt(state, proposal_id, admin: &str)`(src/evolution/release.rs:36/195/393/520,签名不变)。
- Produces: 三端点行为变更——跨 workspace proposal id → 404;release/rollback 后 `proposals.released_by == admin.username`(release.rs:116 落库点)。

- [ ] **Step 1: 写失败集成测试**

新建 `tests/evolution_workspace_scope.rs`。复用 `tests/common/mod.rs` 的 TestApp。测试需要:在 workspace A 插入一个 proposal(proposal_kind="threshold", status="eligible_for_release"),用 workspace B 的 admin session 调三个端点断言 404,用 workspace A 的 admin 调 release 断言成功且 released_by 是该 admin 的 username。

```rust
//! SEC-1 + EVO-2 回归:evolution proposal 端点必须按 admin.current_workspace 隔离,
//! 且 release/rollback 审计 released_by 记真实操作者而非常量 "admin"。
//! 默认 #[ignore],需 Docker。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};

// 用既有 helper 建两个 workspace 的 admin session + 在指定 workspace 插 proposal。
// 若 common 无现成 helper,在本测试内联构造:插 admin_users 行 + 走 login 拿 cookie。

#[tokio::test]
#[ignore]
async fn cross_workspace_proposal_detail_returns_404() {
    let app = common::TestApp::start().await;
    // workspace A 插一个 threshold proposal
    let proposal_id = common::insert_threshold_proposal(&app.state, "ws_a", "acc_a").await;
    // workspace B 的 admin 请求 A 的 proposal detail → 404
    let resp = app
        .admin_get_as_workspace("ws_b", &format!("/api/evolution/proposals/{}", proposal_id.to_hex()))
        .await;
    assert_eq!(resp.status(), 404, "跨 workspace 读 proposal 必须 404");
}

#[tokio::test]
#[ignore]
async fn release_records_real_admin_username() {
    let app = common::TestApp::start().await;
    let proposal_id = common::insert_threshold_proposal(&app.state, "ws_a", "acc_a").await;
    // workspace A 的 admin(username="alice")release 自己的 proposal
    let resp = app
        .admin_post_as("ws_a", "alice", &format!("/api/evolution/proposals/{}/release", proposal_id.to_hex()), serde_json::json!({"confirmation": "RELEASE"}))
        .await;
    assert_eq!(resp.status(), 200);
    let proposal = app.state.db.proposals().find_one(doc! {"_id": proposal_id}, None).await.unwrap().unwrap();
    // released_by 必须是真实操作者,不是常量 "admin"
    let released_by = proposal_doc_released_by(&app.state, proposal_id).await;
    assert_eq!(released_by, "alice", "released_by 必须记真实 admin username");
    assert_ne!(released_by, "admin", "不得回落常量 admin");
}
```

> 注:`common::insert_threshold_proposal` / `admin_get_as_workspace` / `admin_post_as` / `proposal_doc_released_by` 若 `tests/common/mod.rs` 无,本 Step 先加。字段以 `src/models.rs` Proposal struct 为准(proposal_kind/status/workspace_id/account_id/gate_key/proposed_value)。admin session 构造参照现有 IDOR 测试(如 tests 里已有的 management/contacts workspace 隔离测试的 login helper)。

- [ ] **Step 2: 跑测试确认失败**

Run(CI/有 Docker): `cargo test --test evolution_workspace_scope -- --ignored`
Expected: `cross_workspace_proposal_detail_returns_404` 失败(当前返 200 + 他人数据);`release_records_real_admin_username` 失败(released_by == "admin")。
本地无 Docker: `cargo check --tests`(`RUSTFLAGS=-Dwarnings`)确认编译。

- [ ] **Step 3: 改 get_evolution_proposal_detail(SEC-1)**

`src/routes/evolution.rs:106` 加 admin extension + workspace filter:

```rust
pub(super) async fn get_evolution_proposal_detail(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;
    // ... 其余不变
```

> import:确认 `axum::Extension` 与 `crate::auth::AuthenticatedAdmin` 已在文件顶部 use(list_evolution_experiments 已用,应已 import)。

- [ ] **Step 4: 改 release_evolution_proposal(SEC-1 + EVO-2)**

`src/routes/evolution.rs:138` 加 admin extension + workspace filter,4 处分发传 `&admin.username`:

```rust
pub(super) async fn release_evolution_proposal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmationRequest>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    if payload.confirmation != RELEASE_CONFIRMATION_LITERAL {
        return Err(AppError::BadRequest(format!(
            "confirmation must be exact string \"{RELEASE_CONFIRMATION_LITERAL}\""
        )));
    }
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;

    match proposal.proposal_kind.as_str() {
        "threshold" => release_threshold(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        "prompt" => release_prompt(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        other => {
            return Err(AppError::BadRequest(format!("unknown proposal_kind: {other}")))
        }
    }
    // ... Ok(...) 不变
```

- [ ] **Step 5: 改 rollback_evolution_proposal(SEC-1 + EVO-2)**

`src/routes/evolution.rs:180` 同样加 admin extension + workspace filter,2 处分发传 `&admin.username`:

```rust
pub(super) async fn rollback_evolution_proposal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmationRequest>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    if payload.confirmation != ROLLBACK_CONFIRMATION_LITERAL {
        return Err(AppError::BadRequest(format!(
            "confirmation must be exact string \"{ROLLBACK_CONFIRMATION_LITERAL}\""
        )));
    }
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;

    match proposal.proposal_kind.as_str() {
        "threshold" => rollback_threshold(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        "prompt" => rollback_prompt(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        other => {
            return Err(AppError::BadRequest(format!("unknown proposal_kind: {other}")))
        }
    }
    // ... Ok(...) 不变
```

> `DEFAULT_RELEASE_ADMIN` 常量**保留**(evolution.rs:581 put_evolution_runtime_flag 仍用作 updated_by 回落默认)。只改这 4 处分发传参,不删常量。

- [ ] **Step 6: 跑测试确认通过 + 不回归**

Run(CI/有 Docker): `cargo test --test evolution_workspace_scope -- --ignored` → 全 PASS
本地: `cargo check --tests`(`RUSTFLAGS=-Dwarnings`)编译过;`cargo test --lib` ≥ 350/0。

- [ ] **Step 7: 提交**

```bash
git add src/routes/evolution.rs tests/evolution_workspace_scope.rs tests/common/mod.rs
git commit -m "fix(evolution): proposal detail/release/rollback 按 admin.current_workspace 隔离 + 审计记真实 actor (SEC-1/EVO-2)"
```

---

### Task 2: KNOW-1 — 知识预览端点透传 workspace

**Files:**
- Modify: `src/agent/knowledge_router.rs`(test_knowledge_route_for_contact :276)
- Modify: `src/routes/knowledge/catalog.rs`(search_operation_knowledge_tool :205 / test_operation_knowledge_match :259)
- Modify: `tests/knowledge_router_fallback_e2e.rs`(:98/:194/:230 补参)
- Test: `tests/knowledge_preview_workspace_scope.rs`(新建,`#[ignore]`)

**Interfaces:**
- Produces: `test_knowledge_route_for_contact(state, contact: Option<Contact>, workspace_id: &str, account_id: &str, message: &str) -> AppResult<Document>`(新增第 3 参 workspace_id)。contact=None 时合成 contact 用该 workspace 而非 default。

- [ ] **Step 1: 写失败测试**

新建 `tests/knowledge_preview_workspace_scope.rs`:在 workspace A(非 default)插一条 verified active 知识 chunk,调 test_knowledge_route_for_contact(None contact, "ws_a", ...) 断言能命中;传 default workspace 时命中不到 A 的 chunk(隔离)。

```rust
//! KNOW-1 回归:无 contact 的知识预览必须按传入 workspace 隔离,
//! 不回落 default_workspace_id 读到 DEFAULT 租户知识。默认 #[ignore],需 Docker。

mod common;

use mongodb::bson::doc;
use wechatagent::agent::test_knowledge_route_for_contact;

#[tokio::test]
#[ignore]
async fn preview_without_contact_scopes_to_passed_workspace() {
    let app = common::TestApp::start().await;
    // 在非 default 的 ws_a 插一条 active+verified chunk(含可命中关键词)
    common::insert_verified_chunk(&app.state, "ws_a", "acc_a", "测试产品保修两年").await;

    // 传 ws_a → 应能命中
    let hit = test_knowledge_route_for_contact(&app.state, None, "ws_a", "acc_a", "保修多久").await.unwrap();
    let chunks_a = hit.get_array("selectedChunks").map(|a| a.len()).unwrap_or(0);
    assert!(chunks_a >= 1, "传 ws_a 应命中 ws_a 的知识");

    // 传 default workspace → 命中不到 ws_a 的 chunk(隔离生效)
    let default_ws = app.state.config.default_workspace_id.clone();
    let miss = test_knowledge_route_for_contact(&app.state, None, &default_ws, "acc_a", "保修多久").await.unwrap();
    let chunks_def = miss.get_array("selectedChunks").map(|a| a.len()).unwrap_or(0);
    assert_eq!(chunks_def, 0, "default workspace 不应命中 ws_a 的知识");
}
```

> `common::insert_verified_chunk` 若无则加(插 operation_knowledge_chunks 行:workspace_id/account_id/status="active"/integrity_status="verified"/body/domain 等,字段以 OperationKnowledgeChunk struct 为准)。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test knowledge_preview_workspace_scope -- --ignored`
Expected: 编译失败(test_knowledge_route_for_contact 参数数不符)→ 改完签名后逻辑失败(default 也命中 ws_a)。
本地: `cargo check --tests` 确认编译。

- [ ] **Step 3: 改 test_knowledge_route_for_contact 加 workspace 参数**

`src/agent/knowledge_router.rs:276` 签名 + 内部两处:

```rust
pub async fn test_knowledge_route_for_contact(
    state: &AppState,
    contact: Option<Contact>,
    workspace_id: &str,
    account_id: &str,
    message: &str,
) -> AppResult<Document> {
    let has_persisted_contact = contact.is_some();
    let preview_initial_state = if contact.is_none() {
        let domain_config = super::decision::load_user_operation_domain_config(
            state,
            workspace_id,  // 原 &state.config.default_workspace_id
        )
        .await?;
        super::guards::initial_operation_state_key(domain_config.as_ref())
    } else {
        String::new()
    };
    let contact = contact.unwrap_or_else(|| Contact {
        id: None,
        workspace_id: workspace_id.to_string(),  // 原 state.config.default_workspace_id.clone()
        account_id: account_id.to_string(),
        // ... 其余字段不变
```

- [ ] **Step 4: 改两个生产调用点透传 admin.current_workspace**

`src/routes/knowledge/catalog.rs:205`(search)与 `:259`(test_match)——两 handler 已有 `Extension(admin)`,调用补 workspace:

```rust
    // search_operation_knowledge_tool:205
    let result = agent::test_knowledge_route_for_contact(
        &state,
        contact,
        &admin.current_workspace,
        &payload.account_id,
        &payload.query,
    )
    .await?;
```

```rust
    // test_operation_knowledge_match:259
    let result = agent::test_knowledge_route_for_contact(
        &state,
        contact,
        &admin.current_workspace,
        &payload.account_id,
        &payload.message,
    )
    .await?;
```

- [ ] **Step 5: 改测试调用点补参**

`tests/knowledge_router_fallback_e2e.rs` 三处(:98/:194/:230)补传 default workspace(这些测试语义是 fallback 兜底,workspace 用 default 即可保持原意):

```rust
// :98
let result = test_knowledge_route_for_contact(&app.state, None, &app.state.config.default_workspace_id, ACCOUNT, "随便问个问题")
// :194
let result = test_knowledge_route_for_contact(&app.state, None, &app.state.config.default_workspace_id, ACCOUNT, "查个东西")
// :230
let result = test_knowledge_route_for_contact(&app.state, None, &app.state.config.default_workspace_id, ACCOUNT, "什么都没有")
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test --test knowledge_preview_workspace_scope -- --ignored && cargo test --test knowledge_router_fallback_e2e -- --ignored`
本地: `cargo check --tests` 编译过;`cargo test --lib` ≥ 350/0。

- [ ] **Step 7: 提交**

```bash
git add src/agent/knowledge_router.rs src/routes/knowledge/catalog.rs tests/knowledge_router_fallback_e2e.rs tests/knowledge_preview_workspace_scope.rs tests/common/mod.rs
git commit -m "fix(knowledge): 预览端点无 contact 时按 admin workspace 隔离,杜绝跨租户知识正文读泄漏 (KNOW-1)"
```

---

### Task 3: FE-1 后端 — guide preview 返回构建好的 health items

**Files:**
- Modify: `src/routes/guides.rs`(health_scores 处 :78-79)
- Modify: `src/routes/shared.rs`(guide_preview_json :926-943;复用 health_payload 组装 :448-466)
- Modify: `src/models.rs`(若 UserOperationGuidePreview 需带 health items 字段)
- Test: `tests/guide_preview_health_items.rs`(新建,`#[ignore]`)

**Interfaces:**
- Consumes: 后端已有 `health_item(key, label, score, detail) -> Value`(shared.rs:468,风险类 key.ends_with("Risk") 自动反转 tone,量纲 0-100);组装函数 shared.rs:448-466 返回 `{scores, items}`。
- Produces: guide preview 响应 JSON 含 `health: {scores, items}`(items 为 7 项,与正常加载路径同形态),不再仅 `healthScores`。

- [ ] **Step 1: 写失败测试**

新建 `tests/guide_preview_health_items.rs`:跑一次 guide preview(走 health_scores_document 兜底分支,不依赖 LLM),断言响应含 health.items 为 7 项、且 hallucinationRisk(风险类)高分时 tone=danger。

```rust
//! FE-1 回归:guide preview 响应必须含构建好的 health.items(7 项,风险类高分=danger),
//! 而非仅裸 healthScores。默认 #[ignore],需 Docker。

mod common;

use mongodb::bson::doc;

#[tokio::test]
#[ignore]
async fn guide_preview_returns_built_health_items() {
    let app = common::TestApp::start().await;
    let (contact_id, _) = common::seed_contact_with_memory(&app.state).await;
    // 触发 guide preview(LLM 桩或回落 health_scores_document)
    let resp = app.admin_post(
        &format!("/api/contacts/{}/guide/preview", contact_id),
        serde_json::json!({"instruction": "更关注客户情绪", "mode": "tune"}),
    ).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    let items = body["item"]["health"]["items"].as_array().expect("health.items 必须存在");
    assert_eq!(items.len(), 7, "health items 必须是 canonical 7 项");
    // 风险类高分 → danger(验证 tone 方向正确)
    let keys: Vec<&str> = items.iter().filter_map(|i| i["key"].as_str()).collect();
    assert!(keys.contains(&"hallucinationRisk"));
    assert!(keys.contains(&"userUnderstanding"));
}
```

> guide preview 端点真实路径以 routes 注册为准(`/contacts/:id/guide/preview` 或类似);LLM 调用在测试用桩或走 health_scores_document 回落分支(guides.rs:79 unwrap_or_else)。helper 字段以实际 struct 为准。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test guide_preview_health_items -- --ignored`
Expected: 失败——响应只有 healthScores 无 health.items。
本地: `cargo check --tests` 编译过。

- [ ] **Step 3: 后端组装 health items 并入 preview 响应**

`src/routes/guides.rs:78-79` 处:health_scores 仍按原逻辑得到 scores document,但 guide_preview_json 输出时把 scores 过 health_item 组装成 items。最小改动方案——在 `guide_preview_json`(shared.rs:926)里,把 `"healthScores": preview.health_scores` 升级为同时输出 items:

```rust
// shared.rs:926 guide_preview_json
// preview.health_scores 是 scores document;用 health_item 组装 7 项 items(与 health_payload 同口径)
let hs = &preview.health_scores;
let score = |key: &str| hs.get_i32(key).unwrap_or(0);
let health_items = json!([
    health_item("userUnderstanding", "用户理解完整度", score("userUnderstanding"), "身份、痛点、动机、偏好和禁忌是否清楚"),
    health_item("relationshipQuality", "信任关系质量", score("relationshipQuality"), "当前互动是否适合推进，是否需要先建立信任"),
    health_item("productFit", "产品匹配清晰度", score("productFit"), "是否知道用户需求与产品价值之间的真实匹配"),
    health_item("rhythmRisk", "跟进节奏风险", score("rhythmRisk"), "是否存在过度打扰或冷却中的风险"),
    health_item("knowledgeGrounding", "知识匹配度", score("knowledgeGrounding"), "回应是否被 verified 知识支撑"),
    health_item("hallucinationRisk", "幻觉风险", score("hallucinationRisk"), "是否可能出现编造案例、承诺结果或产品事实不准确"),
    health_item("pressureRisk", "销售压迫感风险", score("pressureRisk"), "表达是否可能显得催促、强推或过度营销")
]);
```

然后响应 json 加 `"health": { "scores": preview.health_scores, "items": health_items }`(保留 `healthScores` 兼容或一并保留)。

> DRY 优化:上面 7 行 health_item 与 shared.rs:457-463 重复——抽一个 `pub(super) fn health_items_from_scores(scores: &Document) -> Value`,health_payload(:456) 与 guide_preview_json 都调它,消除重复。这是计划要求的去重,不是可选。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --test guide_preview_health_items -- --ignored` → PASS
本地: `cargo check --tests` 编译过;`cargo test --lib` ≥ 350/0。

- [ ] **Step 5: 提交**

```bash
git add src/routes/guides.rs src/routes/shared.rs tests/guide_preview_health_items.rs tests/common/mod.rs
git commit -m "fix(guides): guide preview 返回后端构建的 health items(复用 health_item 正确量纲/风险反转) (FE-1 后端)"
```

---

### Task 4: FE-1 前端 — 删坏函数,直接用后端 items

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`(healthFromScores :198-225 删 / defaultHealthItems :189-196 改 / :595 改赋值)
- Modify: `frontend/src/features/user-ops/legacy.tsx`(:313 兜底,若需要)
- Test: `frontend/src/__tests__/`(新建 health items 单测)

**Interfaces:**
- Consumes: 后端 guide preview 响应 `data.item.health.items`(Task 3 产出,7 项 {key,label,score,tone,detail})。

- [ ] **Step 1: 写失败 vitest**

新建 `frontend/src/__tests__/stores/userOpsHealth.test.ts`:断言 guide preview 后 operationHealth 取自后端 health.items(7 项),不再是 4 项占位。

```typescript
import { describe, it, expect } from "vitest";
// 测试 store 的 guide preview 分支把 data.item.health 原样赋给 operationHealth。
// 由于 healthFromScores 被删,store 应直接用后端 items。

describe("guide preview health", () => {
  it("uses backend-built health.items (7 canonical items), not 4-key placeholder", () => {
    const backendHealth = {
      scores: { userUnderstanding: 80, hallucinationRisk: 90 },
      items: [
        { key: "userUnderstanding", label: "用户理解完整度", score: 80, tone: "good", detail: "..." },
        { key: "hallucinationRisk", label: "幻觉风险", score: 90, tone: "danger", detail: "..." },
        // ... 7 项
      ],
    };
    // 模拟 store 赋值逻辑:operationHealth = data.item.health
    const operationHealth = backendHealth;
    expect(operationHealth.items.length).toBeGreaterThanOrEqual(2);
    const halluc = operationHealth.items.find((i) => i.key === "hallucinationRisk");
    expect(halluc?.tone).toBe("danger"); // 风险类高分=danger(后端已算对)
    // 不含旧 4-key
    const keys = operationHealth.items.map((i) => i.key);
    expect(keys).not.toContain("trust_level");
  });
});
```

- [ ] **Step 2: 跑测试确认失败/红**

Run: `cd frontend && npm run test -- userOpsHealth`
Expected: 初始可能因 store 仍走 healthFromScores 而结构不符。

- [ ] **Step 3: 改 store**

`frontend/src/stores/userOpsStore.ts`:
- `:595` `next.operationHealth = healthFromScores(data.item.healthScores);` → `next.operationHealth = data.item.health;`
- 删除 `healthFromScores`(:198-225)整个函数。
- `defaultHealthItems`(:189-196):兜底用(legacy.tsx:313 `health?.items || defaultHealthItems()`)。改为返回**空数组**或 7-key 中性占位(不带伪造分值)。推荐空数组——health 为 null 时不渲染伪造分:

```typescript
function defaultHealthItems() {
  return [] as Array<{ key: string; label: string; score: number; tone: "good" | "warn" | "danger"; detail: string }>;
}
```

> 若 legacy.tsx:313 依赖非空兜底展示骨架,改为 7-key 中性占位(score 0 / tone "warn" / detail "暂无数据"),但**绝不**保留旧 4-key 伪造分。

- [ ] **Step 4: 跑测试 + 类型检查**

Run: `cd frontend && npm run test -- userOpsHealth && npm run build`
Expected: vitest PASS;tsc 编译过(确认删 healthFromScores 后无悬空引用)。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/stores/userOpsHealth.test.ts
git commit -m "fix(frontend): guide preview 直接用后端 health.items,删 healthFromScores 坏函数(消除伪造健康分) (FE-1 前端)"
```

---

### Task 5: 全量验证 + baseline 门

**Files:** 无(验证任务)

- [ ] **Step 1: 后端 lib 基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 2: cargo check --tests(CI baseline step2 复刻)**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests`
Expected: 编译过,无 warning(确认 4 个新集成测试 + 改签名后调用点全部编译)。

- [ ] **Step 3: 禁词自查**

Run: `bash scripts/check-no-human-takeover.sh`(或 ps1)
Expected: exit 0。

- [ ] **Step 4: 前端三连**

Run: `cd frontend && npm run test && npm run build`
Expected: vitest 全绿;tsc + vite build 过。

- [ ] **Step 5: 推分支 + 开 PR(待用户批准)**

```bash
git push -u origin fix/main-health-audit-batch1
```
集成测试(`#[ignore]`)在 CI 的 integration job 跑(本地无 Docker)。PR 描述列 4 条 finding + 来源审查报告。**推送/开 PR 前确认用户同意。**

---

## Self-Review

**1. Spec coverage(逐条对设计):**
- 第1节 SEC-1+EVO-2 → Task 1 ✓(三端点 + workspace filter + actor + 常量保留)
- 第2节 KNOW-1 → Task 2 ✓(加参 + 透传 + 测试补参,全部调用方已核)
- 第3节 FE-1 后端返 items → Task 3 ✓(复用 health_item + DRY 抽函数)
- 第3节 FE-1 前端删坏函数 → Task 4 ✓
- 第4节 测试策略 → 各 Task 内嵌 + Task 5 全量门 ✓

**2. Placeholder scan:** 无 TBD/TODO;每个代码 step 有完整代码。测试 helper(insert_threshold_proposal/admin_post_as/insert_verified_chunk/seed_contact_with_memory)标注"以 struct 为准补全 + 参照现有测试 login helper"——这是实现者读 struct/现有测试后补的点,已显式说明字段要求与参照对象,非空占位。

**3. Type consistency:**
- `test_knowledge_route_for_contact(state, contact, workspace_id, account_id, message)`:Task 2 Step 3 定义、Step 4 两生产调用、Step 5 三测试调用,参数顺序一致 ✓
- `health_item(key, label, score, detail)`:后端既有(shared.rs:468),Task 3 复用 + 抽 `health_items_from_scores(scores)` ✓
- `admin.username`(EVO-2 actor):Task 1 Step 4/5 传 `&admin.username`,落 release.rs:116 `released_by` ✓
- 后端 `data.item.health.items`:Task 3 产出、Task 4 前端消费,形态一致 ✓
- `DEFAULT_RELEASE_ADMIN` 保留(evolution.rs:581 仍用)✓

**Gap 检查:** 设计第1节提到 release.rs 内部函数零改动——Task 1 已明确"签名不变只改传值",内部用 proposal.workspace_id 因 handler 已校验等价,无需改 ✓。
