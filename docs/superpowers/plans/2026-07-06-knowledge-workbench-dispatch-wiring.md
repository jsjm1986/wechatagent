# 知识库工作台派工链路补通 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让知识库工作台「派工长任务」真正产生结果——两条结构化入口（卡片驱动 + 对话驱动）汇入同一 chat_task_create+worker，targetChunkId 在派工落库时解析，6 个 action 分层真实现，删掉空转的手打框。

**Architecture:** 后端在 `chat_task_create` 按今日日报卡片 `target_refs` 把 `cardId → targetChunkId` 烤进每个 step；worker 的 `execute_step` 6 个 action 分层实现（fix_chunk/add_chunk/retag 真产 draft+needs_review 草稿，analyze_logs 只读摘要，dismiss 真标记，review_evolution 跳转指引）。前端 DigestCanvas 加多选+批量派工、ChatWorkbench 承接 plannedSteps 渲染确认小卡、删手打框。

**Tech Stack:** Rust (Axum + MongoDB)、React 19 + TypeScript + Vite、vitest。

## Global Constraints

- **红线：worker 永不自动 verify。** fix_chunk/add_chunk/retag 写入的 chunk 强制 `status="draft" + integrity_status="needs_review"`（`apply_create_chunk` chat.rs:1680-1681、`apply_update_chunk` chat.rs:1776-1777 已硬编码，复用即得）。
- **红线：worker 严禁引用** `crate::agent::gateway / outbox / mcp::*` 写入路径（`knowledge_task/mod.rs:12-15`）。
- **ALLOWED_TASK_ACTIONS 闭集校验保留**（chat.rs:1894-1901，6 值：fix_chunk/add_chunk/retag/review_evolution/analyze_logs/dismiss）。
- **freeform 卡不可派工**：卡片 suggestedAction 闭集（knowledge_digest/mod.rs:624）含 `freeform`，但不在 ALLOWED_TASK_ACTIONS 内；前端多选须排除 suggestedAction=="freeform" 的卡。
- **无人工接管禁词**：新增前端行不得含 `人工/接管/takeover/hand-off` 等（`scripts/check-no-human-takeover.sh` 扫描）。
- **每 step 在 RUN_BUDGET.scope 内**（mod.rs:251-256），LLM 类 action 超额 fail-soft。
- **三关验证**：`cargo test --lib`（≥350 passed 0 failed）、`cd frontend && npx vitest run`、`bash scripts/check-no-human-takeover.sh`。

---

### Task 1: 后端 — chat_task_create 解析 cardId → targetChunkId

**Files:**
- Modify: `src/routes/knowledge/chat.rs`（chat_task_create，1865-2008；在 report 加载后、steps_doc 构建时注入解析）
- Test: `src/routes/knowledge/chat.rs`（#[cfg(test)] mod，同文件尾部）

**Interfaces:**
- Produces: `pub(in crate::routes) fn extract_chunk_ref(target_refs: &[mongodb::bson::Document]) -> Option<String>` — 从卡片 target_refs 取第一个 `kind=="chunk"` 的 id。worker（Task 3/4）不直接用它，但它是解析的核心纯函数。

- [ ] **Step 1: 写失败测试（纯函数 extract_chunk_ref）**

在 chat.rs 尾部 `#[cfg(test)] mod tests` 内新增（若无 tests mod 则新建）：

```rust
#[cfg(test)]
mod dispatch_resolution_tests {
    use super::extract_chunk_ref;
    use mongodb::bson::doc;

    #[test]
    fn extract_chunk_ref_returns_first_chunk_id() {
        let refs = vec![
            doc! { "kind": "pack", "id": "p1" },
            doc! { "kind": "chunk", "id": "c1" },
            doc! { "kind": "chunk", "id": "c2" },
        ];
        assert_eq!(extract_chunk_ref(&refs), Some("c1".to_string()));
    }

    #[test]
    fn extract_chunk_ref_none_when_no_chunk_ref() {
        let refs = vec![doc! { "kind": "pack", "id": "p1" }];
        assert_eq!(extract_chunk_ref(&refs), None);
    }

    #[test]
    fn extract_chunk_ref_skips_empty_id() {
        let refs = vec![
            doc! { "kind": "chunk", "id": "" },
            doc! { "kind": "chunk", "id": "c9" },
        ];
        assert_eq!(extract_chunk_ref(&refs), Some("c9".to_string()));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib extract_chunk_ref`
Expected: FAIL — `cannot find function extract_chunk_ref`

- [ ] **Step 3: 实现 extract_chunk_ref 纯函数**

在 chat.rs 里 chat_task_create 上方新增：

```rust
/// 从 digest 卡片 target_refs 取第一个 kind=="chunk" 的非空 id。
/// 用于派工落库时把 cardId 解析成 step.targetChunkId（fix_chunk/retag 需要）。
pub(in crate::routes) fn extract_chunk_ref(
    target_refs: &[mongodb::bson::Document],
) -> Option<String> {
    for r in target_refs {
        if r.get_str("kind").ok() == Some("chunk") {
            if let Ok(id) = r.get_str("id") {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib extract_chunk_ref`
Expected: PASS（3 tests）

- [ ] **Step 5: 在 chat_task_create 里接入解析**

当前 chat_task_create 在 1935-1944 构建 `card_snapshots`（仅 body.card_ids）。改为：先加载今日日报（1922-1934 已加载 `report`），用 report.cards 构建 `cardId → chunk id` 映射，在 steps_doc 构建循环（1902-1919）里对每个含 `cardId` 的 step 注入 `targetChunkId`。

把 1902-1919 的 steps_doc 循环替换为（保留原 stepId 补全 + action 闭集校验，新增 targetChunkId 注入）：

```rust
    // 先加载今日日报，用其 cards 的 target_refs 构建 cardId → chunk id 解析表。
    // 卡片驱动/对话驱动两条路的 step.cardId 都引用今日日报卡片。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;
    let mut card_chunk_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    if let Some(r) = report.as_ref() {
        for c in &r.cards {
            if let Some(cid) = extract_chunk_ref(&c.target_refs) {
                card_chunk_map.insert(c.card_id.to_hex(), cid);
            }
        }
    }

    let mut steps_doc: Vec<Document> = Vec::with_capacity(body.planned_steps.len());
    for (idx, step) in body.planned_steps.iter().enumerate() {
        let mut d = bson_from_json(step)
            .map_err(|e| AppError::BadRequest(format!("plannedSteps[{idx}] 非法 JSON: {e}")))?;
        if d.get_str("stepId").is_err() {
            d.insert("stepId", format!("step_{}", idx + 1));
        }
        let action = d.get_str("action").map_err(|_| {
            AppError::BadRequest(format!("plannedSteps[{idx}].action 缺失"))
        })?;
        if !ALLOWED_TASK_ACTIONS.contains(&action) {
            return Err(AppError::BadRequest(format!(
                "plannedSteps[{idx}].action='{action}' 不在允许集合内：{:?}",
                ALLOWED_TASK_ACTIONS
            )));
        }
        // 派工落库时解析 targetChunkId：若 step 已带（对话驱动 LLM 可能直接给）则尊重，
        // 否则按 cardId 从今日日报卡片 target_refs 解析。fix_chunk/retag 需要它。
        if d.get_str("targetChunkId").is_err() {
            if let Ok(card_id) = d.get_str("cardId") {
                if let Some(chunk_id) = card_chunk_map.get(card_id) {
                    d.insert("targetChunkId", chunk_id.clone());
                }
            }
        }
        steps_doc.push(d);
    }
```

然后删除原 1921-1944 段重复的 report_date/report/card_snapshots 加载（已上移合并），改为直接用上面的 `report` 构建 card_snapshots：

```rust
    let mut card_snapshots: Vec<crate::models::KnowledgeDigestCard> = vec![];
    if let Some(r) = report.as_ref() {
        for cid_hex in &body.card_ids {
            if let Ok(oid) = ObjectId::parse_str(cid_hex) {
                if let Some(c) = r.cards.iter().find(|c| c.card_id == oid) {
                    card_snapshots.push(c.clone());
                }
            }
        }
    }
```

- [ ] **Step 6: 编译验证**

Run: `cargo check`
Expected: 通过（无 borrow/move 错误；report 是 Option，用 `.as_ref()` 两次借用）

- [ ] **Step 7: 提交**

```bash
git add src/routes/knowledge/chat.rs
git commit -m "feat(knowledge): 派工落库时按卡片 target_refs 解析 targetChunkId

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 后端 — 抽出 extract_tags_inner（pub(crate)）供 worker 复用

**Files:**
- Modify: `src/routes/knowledge/import.rs`（extract_operation_knowledge_tags，193-247）
- Modify: `src/routes/knowledge/mod.rs`（re-export inner 供 worker 用；normalize_knowledge_tags/json_string_list 已 pub(super)，350/676）

**Interfaces:**
- Produces: `pub(crate) async fn extract_knowledge_tags_inner(state: &AppState, account_id: Option<&str>, title: &str, body: &str) -> AppResult<(Vec<String>, Vec<String>)>` — 返回 (productTags, businessTopics)。路由 handler 与 worker 共用。

- [ ] **Step 1: 抽出 inner 函数**

在 import.rs 里，把 `extract_operation_knowledge_tags` 的 LLM 调用 + normalize 逻辑抽成 `pub(crate)` inner，handler 改为薄封装。替换 193-247：

```rust
/// LLM 抽取单条 chunk 的 productTags / businessTopics。路由 handler 与
/// knowledge_task worker（retag action）共用。返回 (productTags, businessTopics)。
pub(crate) async fn extract_knowledge_tags_inner(
    state: &AppState,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> AppResult<(Vec<String>, Vec<String>)> {
    let title = if title.trim().is_empty() {
        "未命名知识切片"
    } else {
        title.trim()
    };
    let system = "你是企业微信运营知识库的标签抽取 Agent。给定一个知识切片（标题 + 正文），抽取它的 productTags / businessTopics。只输出严格 JSON。";
    let user = format!(
        r#"请基于下面的知识切片抽取两个字段：

知识标题：{}

知识正文：
{}

输出 JSON：
{{
  "productTags": ["产品/品牌/解决方案名称，最多 5 个；正文确无具体产品/品牌时留空数组"],
  "businessTopics": ["业务主题，最多 3 个；既包括产品维度（如 产品定位差异 / 竞品对比 / 部署方式），也包括方法论/沟通维度（如 价格异议处理 / 销售话术 / 客户关系维护 / 需求澄清）"]
}}

要求：
- productTags 只放正文里**确实出现的**具体产品/品牌/解决方案名；纯方法论/话术正文没有产品名时留空数组，**不要硬塞**。
- businessTopics 概括这条知识"讲的是哪个业务主题"，方法论/话术类内容同样有主题（如价格异议处理、客户沟通），**至少抽 1 个**，不要因为没有产品就整体留空。
- 主题用贴合正文的自然语言短语，不跑题、不空泛。
- 只输出 JSON，不要解释。"#,
        title, body
    );
    let value = agent::generate_agent_json(
        state,
        account_id,
        None,
        None,
        "knowledge.tags.extract",
        system,
        &user,
    )
    .await?;
    let product_tags = json_string_list(&value, "productTags")
        .or_else(|| json_string_list(&value, "product_tags"))
        .unwrap_or_default();
    let business_topics = json_string_list(&value, "businessTopics")
        .or_else(|| json_string_list(&value, "business_topics"))
        .unwrap_or_default();
    Ok((
        normalize_knowledge_tags(product_tags, 5, false),
        normalize_knowledge_tags(business_topics, 3, false),
    ))
}

/// `POST /api/operation-knowledge/extract-tags` —— 给单条 chunk 抽取
/// productTags / businessTopics 两字段。
pub async fn extract_operation_knowledge_tags(
    State(state): State<AppState>,
    Json(payload): Json<ExtractKnowledgeTagsRequest>,
) -> AppResult<Json<Value>> {
    if payload.body.trim().is_empty() {
        return Err(AppError::BadRequest("body is required".to_string()));
    }
    let (product_tags, business_topics) = extract_knowledge_tags_inner(
        &state,
        payload.account_id.as_deref(),
        payload.title.as_deref().unwrap_or(""),
        &payload.body,
    )
    .await?;
    Ok(Json(json!({
        "productTags": product_tags,
        "businessTopics": business_topics,
    })))
}
```

- [ ] **Step 2: 确认 mod.rs 内可见性可达**

`extract_knowledge_tags_inner` 是 `pub(crate)`，worker（`src/knowledge_task/mod.rs`）通过 `crate::routes::knowledge::extract_knowledge_tags_inner` 调用。确认 import.rs 的 inner 在 `mod.rs` 的模块树内已 `pub(crate)` 可达（import 是 knowledge 子模块，pub(crate) 全 crate 可见）。

Run: `cargo check`
Expected: 通过（handler 行为不变，仅内部重构）

- [ ] **Step 3: 提交**

```bash
git add src/routes/knowledge/import.rs
git commit -m "refactor(knowledge): 抽出 extract_knowledge_tags_inner 供 worker 复用

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 后端 — execute_step retag 真实现（重抽标签写回草稿）

**Files:**
- Modify: `src/routes/knowledge/chat.rs`（apply_update_chunk，1711 → 改 `pub(crate)`）
- Modify: `src/knowledge_task/mod.rs`（execute_step retag 分支，524-531）

**Interfaces:**
- Consumes: `extract_knowledge_tags_inner`（Task 2）、`apply_update_chunk`（本任务改 pub(crate)）
- Produces: retag 分支产 draft+needs_review 草稿（改 product_tags/business_topics）

- [ ] **Step 1: 把 apply_update_chunk 改 pub(crate)**

chat.rs:1711，`async fn apply_update_chunk` → `pub(crate) async fn apply_update_chunk`。签名不变：`(state, workspace_id, _account_id, chunk_id, patch, operator_statement)`。

- [ ] **Step 2: 实现 retag 分支**

`src/knowledge_task/mod.rs`，把 retag 分支（524-531）替换为：先取 chunk 的 title+body 抽标签，再走 apply_update_chunk 写回草稿。需要 chunk 读取，用 `state.db.operation_knowledge_chunks()`。

```rust
        "retag" => {
            let Some(cid) = step.get_str("targetChunkId").ok().map(|s| s.to_string()) else {
                return Ok(StepOutcome {
                    chunk_id: None,
                    message: "缺 targetChunkId，未重抽标签".to_string(),
                    details: None,
                });
            };
            let Ok(object_id) = ObjectId::parse_str(&cid) else {
                return Ok(StepOutcome {
                    chunk_id: Some(cid.clone()),
                    message: format!("targetChunkId={cid} 非法，未重抽标签"),
                    details: None,
                });
            };
            let chunk = state
                .db
                .operation_knowledge_chunks()
                .find_one(doc! { "_id": object_id, "workspace_id": workspace_id }, None)
                .await?;
            let Some(chunk) = chunk else {
                return Ok(StepOutcome {
                    chunk_id: Some(cid.clone()),
                    message: format!("chunk {cid} 不存在，未重抽标签"),
                    details: None,
                });
            };
            let body = chunk.body.clone().unwrap_or_default();
            match crate::routes::knowledge::extract_knowledge_tags_inner(
                state,
                Some(_account_id),
                &chunk.title,
                &body,
            )
            .await
            {
                Ok((product_tags, business_topics)) => {
                    let patch = doc! {
                        "productTags": product_tags.clone(),
                        "businessTopics": business_topics.clone(),
                    };
                    // apply_update_chunk 强制 status=draft + integrity_status=needs_review。
                    // operator_statement 传空（retag 不改 sourceQuote，不触发重锚定）。
                    crate::routes::knowledge::apply_update_chunk(
                        state, workspace_id, _account_id, &cid, &patch, "",
                    )
                    .await?;
                    Ok(StepOutcome {
                        chunk_id: Some(cid.clone()),
                        message: format!(
                            "已为 chunk {cid} 重抽标签（产品 {} / 主题 {}），落为待确认草稿",
                            product_tags.len(),
                            business_topics.len()
                        ),
                        details: None,
                    })
                }
                Err(err) => Ok(StepOutcome {
                    chunk_id: Some(cid.clone()),
                    message: format!("chunk {cid} 重抽标签失败（{err}，fail-soft）", ),
                    details: None,
                }),
            }
        }
```

- [ ] **Step 3: 确认 chunk.body / chunk.title 字段名**

Run: `grep -n "pub title\|pub body" src/models.rs | head -5`
Expected: 确认 OperationKnowledgeChunk 有 `pub title: String` 与 `pub body: Option<String>`（若 body 非 Option 则去掉 unwrap_or_default 的 clone 链，直接用）。按实际字段类型调整 `chunk.body` 取值。

- [ ] **Step 4: 编译验证**

Run: `cargo check`
Expected: 通过

- [ ] **Step 5: 提交**

```bash
git add src/routes/knowledge/chat.rs src/knowledge_task/mod.rs
git commit -m "feat(knowledge): retag action 真重抽标签并落待确认草稿

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 后端 — execute_step add_chunk 真实现（起草新条目落草稿）

**Files:**
- Modify: `src/knowledge_task/mod.rs`（execute_step add_chunk 分支，516-523）

**Interfaces:**
- Consumes: `agent::generate_agent_json`（pub(crate) mod.rs:215）、`apply_create_chunk`（pub chat.rs:1668）
- Produces: add_chunk 分支产 draft+needs_review 新 chunk

- [ ] **Step 1: 实现 add_chunk 分支**

用 step.summary 作为起草上下文调 generate_agent_json（复用 knowledge.chat.draft_chunk prompt），取 patch → apply_create_chunk 落库。替换 516-523：

```rust
        "add_chunk" => {
            let summary = step.get_str("summary").unwrap_or("").trim().to_string();
            if summary.is_empty() {
                return Ok(StepOutcome {
                    chunk_id: None,
                    message: "缺 summary 上下文，未起草新条目".to_string(),
                    details: None,
                });
            }
            let system = crate::prompts::load_prompt(
                &state.db,
                workspace_id,
                "knowledge.chat.draft_chunk",
            )
            .await
            .unwrap_or_else(|_| {
                "你是知识库对话 Agent，起草新切片草稿。只输出 JSON: {patch, missingFields, followupQuestions, naturalReply}.".to_string()
            });
            let user = format!(
                r#"请基于下面的运营待办摘要起草一条新知识切片草稿。

待办摘要：
{summary}

起草要求：
- patch 必须含非空的 title、summary、body 三者。
- body（正文）承载可验证事实，绝不能留空。
- 信息不足以填某字段时，把字段名写进 missingFields，不要编造内容。

只输出 JSON 起草一条新切片草稿。"#
            );
            let run_id = format!("knowledge-task-add-{}", step.get_str("stepId").unwrap_or(""));
            match crate::agent::generate_agent_json(
                state, Some(_account_id), None, Some(&run_id),
                "knowledge.chat.draft_chunk", &system, &user,
            )
            .await
            {
                Ok(value) => {
                    let patch = value
                        .get("patch")
                        .and_then(|p| mongodb::bson::to_document(p).ok())
                        .unwrap_or_default();
                    if patch.is_empty() {
                        return Ok(StepOutcome {
                            chunk_id: None,
                            message: "AI 未产出可落库的草稿字段".to_string(),
                            details: None,
                        });
                    }
                    // apply_create_chunk 强制 status=draft + integrity_status=needs_review。
                    // account_id 传 None → 落 workspace 共享域（与 chat 新建一致）。
                    // operator_statement=summary 作为溯源陈述驱动 sourceQuote 锚定。
                    match crate::routes::knowledge::apply_create_chunk(
                        state, workspace_id, None, "knowledge-task", &patch, None, &summary,
                    )
                    .await
                    {
                        Ok(res) => {
                            let new_id = res
                                .get("createdChunkId")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            Ok(StepOutcome {
                                chunk_id: new_id.clone(),
                                message: format!(
                                    "已起草新知识切片草稿{}，请运营在编辑器审核",
                                    new_id.map(|i| format!("：{i}")).unwrap_or_default()
                                ),
                                details: None,
                            })
                        }
                        Err(err) => Ok(StepOutcome {
                            chunk_id: None,
                            message: format!("起草落库失败（{err}，fail-soft）"),
                            details: None,
                        }),
                    }
                }
                Err(err) => Ok(StepOutcome {
                    chunk_id: None,
                    message: format!("起草生成失败（{err}，fail-soft）"),
                    details: None,
                }),
            }
        }
```

- [ ] **Step 2: 编译验证**

Run: `cargo check`
Expected: 通过（apply_create_chunk 签名 `(state, workspace_id, account_id: Option<&str>, session_id, patch: &Document, target_pack_id: Option<&str>, operator_statement)` 已核对）

- [ ] **Step 3: 提交**

```bash
git add src/knowledge_task/mod.rs
git commit -m "feat(knowledge): add_chunk action 真起草新条目落待确认草稿

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 后端 — execute_step analyze_logs 真实现（24h block/hold 只读摘要）

**Files:**
- Modify: `src/knowledge_task/mod.rs`（execute_step analyze_logs 分支，537-541；顶部 use 补 futures::TryStreamExt 若无）

**Interfaces:**
- Consumes: `state.db.events()`（AgentEvent 集合）
- Produces: analyze_logs 分支产只读摘要写进 StepOutcome.details（不改任何知识）

- [ ] **Step 1: 实现 analyze_logs 分支**

查 24h 内本 workspace+account 的 block/hold 类事件，按 kind 计数，摘要写进 details。替换 537-541：

```rust
        "analyze_logs" => {
            let cutoff = mongodb::bson::DateTime::from_millis(
                crate::agent::now_millis() - 24 * 3600 * 1000,
            );
            let filter = doc! {
                "workspace_id": workspace_id,
                "account_id": _account_id,
                "created_at": { "$gte": cutoff },
                "status": { "$in": ["blocked", "blocked_by_safety_guard", "warning", "warn"] },
            };
            let mut cursor = state
                .db
                .events()
                .find(
                    filter,
                    mongodb::options::FindOptions::builder()
                        .sort(doc! { "created_at": -1 })
                        .limit(200)
                        .build(),
                )
                .await?;
            let mut by_kind: std::collections::HashMap<String, i32> =
                std::collections::HashMap::new();
            let mut total = 0i32;
            while let Some(ev) = futures::TryStreamExt::try_next(&mut cursor).await? {
                total += 1;
                *by_kind.entry(ev.kind.clone()).or_insert(0) += 1;
            }
            let mut lines: Vec<(String, i32)> = by_kind.into_iter().collect();
            lines.sort_by(|a, b| b.1.cmp(&a.1));
            let top: Vec<Document> = lines
                .iter()
                .take(10)
                .map(|(k, n)| doc! { "kind": k, "count": *n })
                .collect();
            Ok(StepOutcome {
                chunk_id: None,
                message: format!("已汇总近 24h 拦截/暂缓事件 {} 条（详见 turn 详情）", total),
                details: Some(doc! { "analyzeLogsTotal": total, "byKind": top }),
            })
        }
```

- [ ] **Step 2: 确认 now_millis 可用**

Run: `grep -rn "pub fn now_millis\|pub(crate) fn now_millis" src/agent`
Expected: 若存在则用之；若不存在，改用 `mongodb::bson::DateTime::now().timestamp_millis() - 24*3600*1000` 构造 cutoff（DateTime::now() 在库内可用）。按实际调整。

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 通过（events() 返回 Collection<AgentEvent>；AgentEvent 有 kind/status/created_at 字段，models.rs:902 已核）

- [ ] **Step 4: 提交**

```bash
git add src/knowledge_task/mod.rs
git commit -m "feat(knowledge): analyze_logs action 真查 24h 拦截事件产只读摘要

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 后端 — execute_step dismiss + review_evolution 真实现

**Files:**
- Modify: `src/knowledge_task/mod.rs`（execute_step dismiss 分支 542-546、review_evolution 分支 532-536）

**Interfaces:**
- Consumes: `state.db.knowledge_daily_reports()`（dismiss 用）
- Produces: dismiss 真标记卡片；review_evolution 产跳转指引

- [ ] **Step 1: 实现 dismiss 分支**

按 step.cardId 把今日日报对应卡片加入 dismissed_card_ids（复用 digest_dismiss_card 的 $addToSet 语义）。替换 542-546：

```rust
        "dismiss" => {
            let card_id_hex = step.get_str("cardId").unwrap_or("").to_string();
            if let Ok(card_oid) = ObjectId::parse_str(&card_id_hex) {
                let report_date = step
                    .get_str("reportDate")
                    .ok()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let mut filter = doc! {
                    "workspace_id": workspace_id,
                    "cards.cardId": card_oid,
                };
                if !report_date.is_empty() {
                    filter.insert("report_date", &report_date);
                }
                let _ = state
                    .db
                    .knowledge_daily_reports()
                    .update_one(
                        filter,
                        doc! { "$addToSet": { "dismissed_card_ids": card_oid } },
                        None,
                    )
                    .await;
                Ok(StepOutcome {
                    chunk_id: None,
                    message: format!("已忽略卡片 {card_id_hex}"),
                    details: None,
                })
            } else {
                Ok(StepOutcome {
                    chunk_id: None,
                    message: "缺有效 cardId，未忽略卡片".to_string(),
                    details: None,
                })
            }
        }
```

- [ ] **Step 2: 实现 review_evolution 分支**

替换 532-536（产明确指引，不假装自动评估）：

```rust
        "review_evolution" => Ok(StepOutcome {
            chunk_id: None,
            message: "本项需人工评估：请到「自优化中心」查看并裁决候选提案，AI 不自动放量".to_string(),
            details: None,
        }),
```

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 通过

- [ ] **Step 4: 跑后端基线**

Run: `cargo test --lib`
Expected: ≥350 passed, 0 failed（含 Task 1 的 extract_chunk_ref 3 个新测试）

- [ ] **Step 5: 提交**

```bash
git add src/knowledge_task/mod.rs
git commit -m "feat(knowledge): dismiss 真标记卡片 + review_evolution 产人工评估指引

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: 前端 — ChatWorkbench 承接 plannedSteps + 删手打框

**Files:**
- Modify: `frontend/src/features/knowledge/today.tsx`（ChatWorkbench，58-449）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（已有派工测试，86-155）

**Interfaces:**
- Consumes: `chat_turn` 响应的 `plannedSteps`（chat.rs:315 已返回）
- Produces: 派工确认小卡 UI；POST /api/knowledge/chat/tasks {sessionId, plannedSteps, cardIds:[]}

- [ ] **Step 1: ChatTurnResponse 类型补 plannedSteps**

today.tsx:44-56 的 `ChatTurnResponse` 接口内新增：

```typescript
  plannedSteps?: Array<{ stepId?: string; cardId?: string; action: string; summary?: string }> | null;
```

- [ ] **Step 2: 删除手打派工 UI 与状态**

删除以下（均在 ChatWorkbench 内）：
- 状态：`stepsText`/`stepAction`/`dispatching` 及其 useState（today.tsx:67-69）
- 函数：`dispatchTask`（today.tsx:249-294）
- JSX：`<div className="wikiChatDispatch">...</div>` 整块（today.tsx:407-445）
- 未再使用的 import（如 `SendHorizonal` 若仅此处用，检查后删）

- [ ] **Step 3: 新增 plannedSteps 承接状态 + 确认派工**

在 ChatWorkbench 内新增状态与函数：

```typescript
  const [pendingSteps, setPendingSteps] = useState<
    Array<{ stepId?: string; cardId?: string; action: string; summary?: string }>
  >([]);

  async function confirmDispatch() {
    if (!sessionId || pendingSteps.length === 0) return;
    setDispatching(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/chat/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ sessionId, plannedSteps: pendingSteps, cardIds: [] }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { taskId?: string };
      setPendingSteps([]);
      setInfo(`已派工长任务${data.taskId ? `：${data.taskId}` : ""}，可在右侧「派工跟踪」查看进度`);
      if (data.taskId) {
        window.dispatchEvent(new CustomEvent("wikiTrackTask", { detail: { taskId: data.taskId } }));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setDispatching(false);
    }
  }
```

（保留 `dispatching` useState 供 confirmDispatch 用；Step 2 删的是 stepsText/stepAction，不删 dispatching。）

- [ ] **Step 4: submit() 承接 plannedSteps**

在 submit()（today.tsx:150-182）拿到 resp 后，承接 plannedSteps：

```typescript
      const resp = (await r.json()) as ChatTurnResponse;
      if (resp.sessionId !== sessionId) {
        setSessionId(resp.sessionId);
        persistSession(resp.sessionId);
      }
      if (Array.isArray(resp.plannedSteps) && resp.plannedSteps.length > 0) {
        setPendingSteps(resp.plannedSteps);
      }
      setDraft("");
      await loadHistory(resp.sessionId);
```

- [ ] **Step 5: 渲染派工确认小卡**

在 wikiChatStream 之后、footer 之前插入：

```tsx
        {pendingSteps.length > 0 ? (
          <div className="wikiChatDispatch">
            <div className="wikiChatDispatchHead">
              <span className="wikiArchiveTag">待确认派工</span>
              <span className="wikiArchiveTimelineTime">AI 拆出 {pendingSteps.length} 步，确认后交后台执行</span>
            </div>
            <ul className="wikiChatFollowups">
              {pendingSteps.map((s, i) => (
                <li key={s.stepId ?? i}>{s.action} · {s.summary ?? ""}</li>
              ))}
            </ul>
            <div className="wikiChatFooterRow">
              <button type="button" className="primary" onClick={() => void confirmDispatch()} disabled={dispatching || !sessionId}>
                {dispatching ? "派工中…" : "确认派工"}
              </button>
              <button type="button" onClick={() => setPendingSteps([])} disabled={dispatching}>
                取消
              </button>
            </div>
          </div>
        ) : null}
```

- [ ] **Step 6: 更新既有派工测试**

`knowledge.test.tsx:86-155` 现测的是手打派工。改为测对话驱动承接：mock chat_turn 返回含 plannedSteps 的响应 → 断言渲染确认小卡 → 点确认 → 断言 POST /knowledge/chat/tasks body 含 plannedSteps（camelCase）。保留原断言维度（body 含 sessionId + plannedSteps 数组 + action）。

- [ ] **Step 7: 跑前端测试 + build**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿 + build 成功

- [ ] **Step 8: no-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: 0 violations

- [ ] **Step 9: 提交**

```bash
git add frontend/src/features/knowledge/today.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "feat(knowledge): ChatWorkbench 承接 plannedSteps 渲染派工确认小卡，删手打框

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: 前端 — DigestCanvas 多选 + 批量派工

**Files:**
- Modify: `frontend/src/features/knowledge/today.tsx`（DigestCanvas，638-769）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（新增用例）

**Interfaces:**
- Consumes: 今日日报卡片（card.cardId/suggestedAction）
- Produces: POST /api/knowledge/chat/tasks {sessionId(前端生成), plannedSteps(选中卡片), cardIds}

- [ ] **Step 1: DigestCanvas 加多选状态**

DigestCanvas 内新增：

```typescript
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [dispatchingBatch, setDispatchingBatch] = useState(false);

  function toggleSelect(cardId: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(cardId)) next.delete(cardId);
      else next.add(cardId);
      return next;
    });
  }
```

- [ ] **Step 2: 批量派工函数（排除 freeform）**

```typescript
  async function dispatchSelected() {
    if (selected.size === 0) return;
    const steps = visibleCards
      .filter((c) => selected.has(c.cardId) && c.suggestedAction !== "freeform")
      .map((c, idx) => ({
        stepId: `step_${idx + 1}`,
        cardId: c.cardId,
        action: c.suggestedAction,
        summary: c.summary,
        reportDate: report?.reportDate,
      }));
    if (steps.length === 0) {
      setError(new Error("选中的卡片都不可派工（仅查看类卡片无执行动作）"));
      return;
    }
    setDispatchingBatch(true);
    setError(null);
    try {
      const sessionId = crypto.randomUUID();
      const r = await fetch("/api/knowledge/chat/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ sessionId, plannedSteps: steps, cardIds: steps.map((s) => s.cardId) }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { taskId?: string };
      setSelected(new Set());
      if (data.taskId) {
        window.dispatchEvent(new CustomEvent("wikiTrackTask", { detail: { taskId: data.taskId } }));
      }
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setDispatchingBatch(false);
    }
  }
```

- [ ] **Step 3: 卡片加复选框 + 头部加批量派工按钮**

在 wikiDigestHead 的 actions 区（today.tsx:721-728）加：

```tsx
          <button type="button" className="primary" onClick={() => void dispatchSelected()} disabled={dispatchingBatch || selected.size === 0}>
            {dispatchingBatch ? "派工中…" : `批量派工（${selected.size}）`}
          </button>
```

在每张卡片 wikiDigestCardHead（today.tsx:741-744）加复选框：

```tsx
              <input
                type="checkbox"
                checked={selected.has(card.cardId)}
                onChange={() => toggleSelect(card.cardId)}
                disabled={card.suggestedAction === "freeform"}
                aria-label={`选择卡片 ${card.title}`}
              />
```

- [ ] **Step 4: 新增测试**

`knowledge.test.tsx` 新增用例：mock digest/today 返回含 2 张卡（一张 fix_chunk、一张 freeform）→ 勾选 → 点批量派工 → 断言 POST body.plannedSteps 只含 fix_chunk 卡（freeform 被排除）、含 cardIds。

- [ ] **Step 5: 跑前端测试 + build + lint**

Run: `cd frontend && npx vitest run && npm run build && cd .. && bash scripts/check-no-human-takeover.sh`
Expected: 全绿 + build 成功 + 0 violations

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/knowledge/today.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "feat(knowledge): DigestCanvas 卡片多选 + 批量派工（排除 freeform）

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- 两条入口 → Task 7（对话驱动）+ Task 8（卡片驱动）✓
- targetChunkId 解析 → Task 1 ✓
- 6 action 分层：fix_chunk（Task 1 解析后即通）/add_chunk（Task 4）/retag（Task 2+3）/analyze_logs（Task 5）/dismiss+review_evolution（Task 6）✓
- 删手打框 → Task 7 Step 2 ✓
- freeform 边界 → Task 8 Step 2 ✓
- 红线 draft+needs_review → 复用 apply_create_chunk/apply_update_chunk 硬编码 ✓
- 测试三关 → 各 Task 末尾 + Task 6/7/8 ✓

**待实现时现场核验的字段（计划已标注 grep 步骤，非占位符）：**
- Task 3 Step 3：chunk.body 是否 Option（grep models.rs）
- Task 5 Step 2：now_millis 是否存在（grep agent），否则用 DateTime::now().timestamp_millis()

**Type consistency:** extract_chunk_ref（Task 1）、extract_knowledge_tags_inner→(Vec,Vec)（Task 2→3）、apply_update_chunk pub(crate)（Task 3）、apply_create_chunk createdChunkId 返回键（Task 4，chat.rs:1704 已核）、plannedSteps 结构前后端一致（Task 1/7/8）——均对齐。
