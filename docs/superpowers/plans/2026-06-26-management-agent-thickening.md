# 指挥中心做厚：统一管理对话入口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把现有 command-center/management.rs 雏形做厚成"管理者用自然语言操控整个项目"的统一入口——补齐确认执行循环、扩工具集、核实执行结果。

**Architecture:** 复用现有 `management.rs`（NL→plan→执行→审计骨架已在）。三块增量：①新增 `confirm` 端点闭合"提议→人确认→执行"循环；②`tool_effect` 加 `risk` 档 + `merge_product_tools` / `execute_management_tool` 扩配置/策略/知识工具；③`outcome assertion` 核实工具真实结果，汇报基于真实结果而非 plan.summary。与客户侧 principal escalation 同构（pending→确认→resolve→执行）。

**Tech Stack:** Rust 2021 / Axum / MongoDB (mongodb 2.8 bson)。后端为主，前端 command-center 频道做厚（React 19 + TS + Zustand）留最后。

## Global Constraints

- 守测试基线：`cargo test --lib` ≥ 350 passed, 0 failed，不回归。
- 第一期权限放大先跑通：工具集尽量全接（含 send / 改全局 prompt / 切 provider）；`risk` 字段静态声明就位（为后续收紧留挂点），但初期 dangerous 默认放行，不靠确认门挡功能。
- 执行结果核实是第一期底线，不因放权而省：agent 汇报的成功/失败必须基于核实后的真实结果，不确定标 `executed_unverified`，绝不假报成功。
- 状态枚举闭集：`AgentToolCall.status` / `AgentCommandRun.status` 写入未知值必须拒写（项目惯例）。
- `execute_management_tool` 兜底分支已有 `advertised` 白名单防 LLM 幻觉工具名，新增工具不得绕过该防线。
- 不引入 no-human-takeover 禁用词（src/agent|routes|evolution|frontend/src 扫描）。
- 不碰客户侧 principal escalation 链。
- 工作在 worktree `worktree-mgmt-agent-design`（已创建）。

---

## File Structure

- `src/routes/management.rs`（修改主体）：`ToolEffect` 加 `risk`；`tool_effect` 表补新工具风险档；`merge_product_tools` 补工具声明；`execute_management_tool` 补新工具分发 + outcome assertion；新增 `confirm_management_command` / `reject_management_command` handler；`post_management_message` 的结果汇报改为基于真实结果。
- `src/routes/mod.rs`（修改）：注册 `POST /management-agent/commands/:id/confirm` + `/reject`。
- `src/models.rs`（修改）：`AgentToolCall.status` 注释补 `executed_unverified`（status 是 String，无需改类型，补闭集校验常量）。
- 测试：`management.rs` 内 `#[cfg(test)] mod tests`（纯函数：risk 裁定、outcome assertion 判定）。

---

## Task 1: ToolEffect 加 risk 档 + 纯函数风险裁定

**Files:**
- Modify: `src/routes/management.rs:649-662`（ToolEffect 结构 + tool_effect 表）
- Test: `src/routes/management.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `enum ToolRisk { Readonly, Low, Dangerous }`；`fn tool_effect(tool_name: &str) -> ToolEffect`（ToolEffect 加 `risk: ToolRisk` 字段）；`fn plan_requires_confirmation(tool_names: &[&str], dangerous_confirm_enabled: bool) -> bool`。

- [ ] **Step 1: 写失败测试**

在 `src/routes/management.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
#[test]
fn tool_effect_classifies_risk() {
    assert_eq!(tool_effect("wechatagent.search_contacts").risk, ToolRisk::Readonly);
    assert_eq!(tool_effect("wechatagent.create_follow_up_task").risk, ToolRisk::Low);
    assert_eq!(tool_effect("wechatagent.send_contact_message").risk, ToolRisk::Dangerous);
    assert_eq!(tool_effect("wechatagent.publish_domain_profile").risk, ToolRisk::Dangerous);
    // 只读工具同时 read_only=true（与既有 dry-run 逻辑兼容）
    assert!(tool_effect("wechatagent.search_contacts").read_only);
}

#[test]
fn confirmation_gate_off_by_default_phase1() {
    // 第一期权限放大：dangerous_confirm_enabled=false 时即便有 dangerous 工具也不强制确认
    assert!(!plan_requires_confirmation(&["wechatagent.send_contact_message"], false));
    // 开关打开后 dangerous 触发确认（为后续阶段预留）
    assert!(plan_requires_confirmation(&["wechatagent.send_contact_message"], true));
    // 全 readonly 永不需确认
    assert!(!plan_requires_confirmation(&["wechatagent.search_contacts"], true));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib tool_effect_classifies_risk confirmation_gate_off_by_default_phase1 2>&1 | tail -20`
Expected: FAIL（`ToolRisk` 未定义 / `risk` 字段不存在 / `plan_requires_confirmation` 未定义）

- [ ] **Step 3: 实现 ToolRisk + 改 ToolEffect + tool_effect 表 + 裁定函数**

替换 `src/routes/management.rs:649-662` 的 ToolEffect 区域：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolRisk {
    Readonly,
    Low,
    Dangerous,
}

pub(super) struct ToolEffect {
    pub read_only: bool,
    pub risk: ToolRisk,
}

pub(super) fn tool_effect(tool_name: &str) -> ToolEffect {
    use ToolRisk::*;
    let risk = match tool_name {
        // 只读查询
        "wechatagent.search_contacts"
        | "wechatagent.query_runs"
        | "wechatagent.query_metrics"
        | "wechatagent.query_health"
        | "wechatagent.query_inbox" => Readonly,
        // 低风险可逆写
        "wechatagent.import_contacts"
        | "wechatagent.enable_contact_agent"
        | "wechatagent.disable_contact_agent"
        | "wechatagent.create_follow_up_task"
        | "wechatagent.update_contact_profile"
        | "wechatagent.update_operation_domain"
        | "wechatagent.set_assist_mode" => Low,
        // 高风险/宽影响
        "wechatagent.send_contact_message"
        | "wechatagent.publish_domain_profile"
        | "wechatagent.activate_domain_profile"
        | "wechatagent.publish_prompt_template"
        | "wechatagent.verify_knowledge_chunk"
        | "wechatagent.reject_knowledge_chunk" => Dangerous,
        // 未知（含 MCP 透传工具）：保守按 Low，read_only=false
        _ => Low,
    };
    let read_only = matches!(risk, Readonly);
    ToolEffect { read_only, risk }
}

/// 第一期权限放大：dangerous_confirm_enabled 默认 false（见 spec §1.2），
/// 此时即便有 dangerous 工具也不强制确认，先跑通功能。开关为后续收紧预留。
pub(super) fn plan_requires_confirmation(
    tool_names: &[&str],
    dangerous_confirm_enabled: bool,
) -> bool {
    dangerous_confirm_enabled
        && tool_names
            .iter()
            .any(|name| tool_effect(name).risk == ToolRisk::Dangerous)
}
```

注意：原 `tool_effect` 的调用点（management.rs:291 `!tool_effect(&c.tool_name).read_only`、:668 `is_read_only_tool`、:671 `should_dry_run_tool`）依赖 `read_only` 字段——保持 `read_only` 字段名不变，已兼容。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib tool_effect_classifies_risk confirmation_gate_off_by_default_phase1 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "feat(mgmt-agent): ToolEffect 加 risk 档 + plan_requires_confirmation 裁定(第一期开关默认关)"
```

---

## Task 2: outcome assertion — 核实工具真实结果（第一期底线）

**Files:**
- Modify: `src/routes/management.rs`（新增 assert_tool_outcome 纯函数）
- Test: `src/routes/management.rs` 内 tests

**Interfaces:**
- Consumes: `execute_management_tool` 返回的 `Ok(Value)` response。
- Produces: `enum ToolOutcome { Succeeded, Failed(String), Unverified(String) }`；`fn assert_tool_outcome(tool_name: &str, response: &Value) -> ToolOutcome`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn outcome_assertion_detects_business_failure() {
    use serde_json::json;
    // send: MCP RPC 返 Ok 但 success=false（账号离线）→ Failed
    let r = json!({"success": false, "error": "account offline"});
    assert!(matches!(assert_tool_outcome("wechatagent.send_contact_message", &r), ToolOutcome::Failed(_)));
    // send: success=true 且有 msgId → Succeeded
    let r = json!({"success": true, "msgId": "m123"});
    assert!(matches!(assert_tool_outcome("wechatagent.send_contact_message", &r), ToolOutcome::Succeeded));
    // update: matched=0 → Failed（未命中、实际没改）
    let r = json!({"matched": 0, "modified": 0});
    assert!(matches!(assert_tool_outcome("wechatagent.update_contact_profile", &r), ToolOutcome::Failed(_)));
    // update: modified>=1 → Succeeded
    let r = json!({"matched": 1, "modified": 1});
    assert!(matches!(assert_tool_outcome("wechatagent.update_contact_profile", &r), ToolOutcome::Succeeded));
    // 无断言规则的工具 + response 无明显信号 → Unverified（诚实暴露）
    let r = json!({"weird": "shape"});
    assert!(matches!(assert_tool_outcome("wechatagent.some_unknown_tool", &r), ToolOutcome::Unverified(_)));
    // readonly 查询：有数据即 Succeeded
    let r = json!({"items": []});
    assert!(matches!(assert_tool_outcome("wechatagent.query_runs", &r), ToolOutcome::Succeeded));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib outcome_assertion_detects_business_failure 2>&1 | tail -20`
Expected: FAIL（`ToolOutcome` / `assert_tool_outcome` 未定义）

- [ ] **Step 3: 实现 assert_tool_outcome**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ToolOutcome {
    Succeeded,
    Failed(String),
    Unverified(String),
}

/// 核实工具调用的"业务结果"——区别于"调用返回 Ok"。返回 Ok 不等于业务成功
/// （如 MCP send 返 Ok 但 success=false=账号离线）。无法判定的诚实标 Unverified，
/// 绝不假报成功（spec §3）。
pub(super) fn assert_tool_outcome(tool_name: &str, response: &Value) -> ToolOutcome {
    // MCP 发送类：核实 success + msgId
    if tool_name == "wechatagent.send_contact_message" {
        let success = response.get("success").and_then(Value::as_bool);
        match success {
            Some(true) => return ToolOutcome::Succeeded,
            Some(false) => {
                let err = response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP 返回 success=false");
                return ToolOutcome::Failed(err.to_string());
            }
            None => {
                return ToolOutcome::Unverified(
                    "MCP 响应无 success 字段，无法确认是否送达".to_string(),
                )
            }
        }
    }
    // 写库类：核实 matched/modified
    if let Some(matched) = response.get("matched").and_then(Value::as_i64) {
        if matched == 0 {
            return ToolOutcome::Failed("未命中任何记录，实际没有改动".to_string());
        }
        return ToolOutcome::Succeeded;
    }
    // 显式 ok:true 的产品工具
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return ToolOutcome::Succeeded;
    }
    // 只读查询：返回了结构即视为成功
    if matches!(tool_effect(tool_name).risk, ToolRisk::Readonly) {
        return ToolOutcome::Succeeded;
    }
    // 兜底：无法判定 → 诚实标 Unverified
    ToolOutcome::Unverified(format!(
        "工具 '{tool_name}' 已执行，但响应结构无法确认业务结果，请核对"
    ))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib outcome_assertion_detects_business_failure 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "feat(mgmt-agent): assert_tool_outcome 核实工具真实业务结果(区分调用Ok与业务成功)"
```

---

## Task 3: 执行结果汇报基于真实结果，并接入 outcome assertion

**Files:**
- Modify: `src/routes/management.rs:205-296`（执行循环 status 判定）+ `:312-324`（assistant_text 生成）
- Test: `src/routes/management.rs` 内 tests（纯函数 build_execution_summary）

**Interfaces:**
- Consumes: `assert_tool_outcome`（Task 2）、`ToolOutcome`。
- Produces: `fn build_execution_summary(results: &[(String, ToolOutcome)]) -> String`（基于真实结果生成汇报文本）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn execution_summary_reports_real_outcomes() {
    let results = vec![
        ("wechatagent.update_contact_profile".to_string(), ToolOutcome::Succeeded),
        ("wechatagent.send_contact_message".to_string(), ToolOutcome::Failed("账号离线".to_string())),
    ];
    let s = build_execution_summary(&results);
    assert!(s.contains("update_contact_profile"));
    assert!(s.contains("失败") || s.contains("账号离线"));
    // 不假报全部成功
    assert!(!s.contains("全部成功"));

    let unv = vec![("wechatagent.x".to_string(), ToolOutcome::Unverified("无法确认".to_string()))];
    let s2 = build_execution_summary(&unv);
    assert!(s2.contains("待核实") || s2.contains("无法确认"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib execution_summary_reports_real_outcomes 2>&1 | tail -20`
Expected: FAIL（`build_execution_summary` 未定义）

- [ ] **Step 3: 实现 build_execution_summary + 在执行循环接入 outcome assertion**

新增纯函数：

```rust
/// 基于真实执行结果生成汇报（spec §3.2：不回放 plan.summary，区分打算做与做成了什么）。
pub(super) fn build_execution_summary(results: &[(String, ToolOutcome)]) -> String {
    if results.is_empty() {
        return "没有需要执行的操作。".to_string();
    }
    let mut lines = Vec::new();
    for (tool, outcome) in results {
        match outcome {
            ToolOutcome::Succeeded => lines.push(format!("✅ {tool}：已完成")),
            ToolOutcome::Failed(why) => lines.push(format!("❌ {tool}：失败——{why}")),
            ToolOutcome::Unverified(why) => lines.push(format!("⚠️ {tool}：已执行待核实——{why}")),
        }
    }
    lines.join("\n")
}
```

在执行循环（management.rs:227-251 `Ok(response)` 分支）里，把 `succeeded_status` 的判定从"只要 Ok 就 succeeded"改为接 `assert_tool_outcome`：

```rust
Ok(response) => {
    let outcome = if should_dry_run_tool(&planned.tool_name, effective_dry_run) {
        ToolOutcome::Succeeded // dry_run 不核实真实结果
    } else {
        assert_tool_outcome(&planned.tool_name, &response)
    };
    let status_str = match (&outcome, should_dry_run_tool(&planned.tool_name, effective_dry_run)) {
        (_, true) => "dry_run",
        (ToolOutcome::Succeeded, _) => "succeeded",
        (ToolOutcome::Failed(_), _) => "failed",
        (ToolOutcome::Unverified(_), _) => "executed_unverified",
    };
    // ... 写 tool_calls status = status_str，response = response_doc
    // 收集 (planned.tool_name.clone(), outcome) 进一个 Vec<(String, ToolOutcome)> outcomes
    // Failed 时仍 break（与原逻辑一致）
}
```

执行循环结束后，assistant_text（原 :312-324）改为：非确认且非空 outcomes 时用 `build_execution_summary(&outcomes)`，确认态保持原 "待确认：{summary}"。

> 实现注意：原 :285 `failed.is_some()` 的 final_status 逻辑保留；新增 `executed_unverified` 出现时 command run 仍按 succeeded（已执行，只是结果待核实，非失败）。`AgentToolCall.status` 写入 "executed_unverified" 需在 Task 4 加入闭集校验。

- [ ] **Step 4: 运行测试确认通过 + 全量 lib 不回归**

Run: `cargo test --lib execution_summary_reports_real_outcomes 2>&1 | tail -20`
Expected: PASS
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "feat(mgmt-agent): 执行汇报基于真实outcome(接assert_tool_outcome,executed_unverified态)"
```

---

## Task 4: AgentToolCall.status 闭集加 executed_unverified

**Files:**
- Modify: `src/models.rs`（找 AgentToolCall status 闭集校验常量，grep `executed_unverified` 邻近的 status 校验；若无集中校验则在 management.rs 写入处保证只写枚举内值）
- Test: `src/routes/management.rs` 内 tests

**Interfaces:**
- Produces: status 字符串闭集含 `running/dry_run/succeeded/failed/executed_unverified`。

- [ ] **Step 1: 定位现有 status 闭集**

Run: `git grep -n "\"succeeded\"\|\"dry_run\"\|TOOL_CALL_STATUS\|AgentToolCall" src/models.rs | head`
判断 status 是否有集中校验常量。若 `AgentToolCall.status` 是裸 String 无闭集校验（很可能），则本任务改为：在 management.rs 加一个 `const TOOL_CALL_STATUSES: &[&str]` + debug_assert，并写测试锁定 5 个合法值。

- [ ] **Step 2: 写测试锁定闭集**

```rust
#[test]
fn tool_call_status_closed_set() {
    const EXPECTED: &[&str] = &["running", "dry_run", "succeeded", "failed", "executed_unverified"];
    for s in EXPECTED {
        assert!(TOOL_CALL_STATUSES.contains(s), "缺少状态 {s}");
    }
    assert_eq!(TOOL_CALL_STATUSES.len(), 5);
}
```

- [ ] **Step 3: 运行确认失败 → 加常量 → 确认通过**

```rust
pub(super) const TOOL_CALL_STATUSES: &[&str] =
    &["running", "dry_run", "succeeded", "failed", "executed_unverified"];
```

Run: `cargo test --lib tool_call_status_closed_set 2>&1 | tail -10`
Expected: 先 FAIL 后 PASS

- [ ] **Step 4: 提交**

```bash
git add src/routes/management.rs src/models.rs
git commit -m "feat(mgmt-agent): tool_call status 闭集加 executed_unverified"
```

---

## Task 5: confirm / reject 端点闭合"提议→确认→执行"循环

**Files:**
- Modify: `src/routes/management.rs`（新增 confirm_management_command / reject_management_command handler）
- Modify: `src/routes/mod.rs:773-784`（注册路由）
- Test: `src/routes/management.rs` 内 tests（乐观锁条件构造纯函数）

**Interfaces:**
- Consumes: `AgentCommandRun`（plan 暂存在 `plan` 字段，Document）、execute 循环逻辑（抽成可复用的 `execute_plan_tool_calls`）、`build_execution_summary`。
- Produces: `POST /management-agent/commands/:id/confirm` / `/reject`。乐观锁仿 `escalation/ledger.rs:138 resolve_escalation` 的 `find_one_and_update` 条件 `status==pending_confirmation`。

- [ ] **Step 1: 写失败测试（乐观锁过滤条件纯函数）**

```rust
#[test]
fn confirm_filter_only_targets_pending_confirmation() {
    let filter = build_confirm_filter("workspace1", &test_object_id());
    // filter 必须含 status: pending_confirmation（防二次确认 / 防确认非待确认命令）
    assert_eq!(filter.get_str("workspace_id").unwrap(), "workspace1");
    assert_eq!(filter.get_str("status").unwrap(), "pending_confirmation");
}
```

（`test_object_id()` 用 `mongodb::bson::oid::ObjectId::new()`；`build_confirm_filter(workspace, id) -> Document`。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib confirm_filter_only_targets_pending_confirmation 2>&1 | tail -10`
Expected: FAIL（`build_confirm_filter` 未定义）

- [ ] **Step 3: 实现 filter 纯函数 + confirm/reject handler**

```rust
pub(super) fn build_confirm_filter(workspace_id: &str, run_id: &mongodb::bson::oid::ObjectId) -> Document {
    doc! { "_id": run_id, "workspace_id": workspace_id, "status": "pending_confirmation" }
}
```

confirm handler 骨架（复用 resolve_escalation 乐观锁模式——`find_one_and_update` 仅命中 pending_confirmation，二次确认返回 None 即幂等）：

```rust
pub(super) async fn confirm_management_command(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let run_id = parse_object_id(&id)?;
    // 乐观锁：仅 pending_confirmation 可领取，原子改 running，防并发双执行
    let run = state.db.command_runs().find_one_and_update(
        build_confirm_filter(&admin.current_workspace, &run_id),
        doc! { "$set": { "status": "running", "updated_at": DateTime::now() } },
        None,
    ).await?;
    let Some(run) = run else {
        // 已确认过 / 不存在 / 非待确认 → 幂等返回
        return Ok(Json(json!({ "status": "already_processed_or_not_found" })));
    };
    // 取出暂存 plan，执行其 tool_calls（第一期：放权全执行），接 assert_tool_outcome
    let plan: ManagementPlan = run.plan.as_ref()
        .and_then(|d| mongodb::bson::from_document(d.clone()).ok())
        .unwrap_or_default();
    let tools = merge_product_tools(mcp::list_tools_for_account(&state, &run.account_id).await?);
    let advertised = advertised_tool_names(&tools);
    let outcomes = execute_plan_tool_calls(
        &state, &admin.current_workspace, &run.account_id, &plan.tool_calls,
        run_id, false /* not dry_run on confirm */, &advertised,
    ).await?;
    let summary = build_execution_summary(&outcomes);
    let final_status = if outcomes.iter().any(|(_, o)| matches!(o, ToolOutcome::Failed(_))) {
        "failed"
    } else { "succeeded" };
    state.db.command_runs().update_one(
        doc! { "_id": run_id },
        doc! { "$set": { "status": final_status, "summary": &summary, "updated_at": DateTime::now() } },
        None,
    ).await?;
    // 落 assistant message（基于真实结果）
    // ... insert ManagementAgentMessage role=assistant content=summary
    Ok(Json(json!({ "status": final_status, "summary": summary })))
}
```

reject handler：乐观锁同 filter，`$set status=canceled`，落一条 assistant message "已取消该计划，未执行。"。

> 实现注意：把 `post_management_message` 里 :192-281 的执行循环抽成 `execute_plan_tool_calls(state, workspace, account, tool_calls, run_id, dry_run, advertised) -> AppResult<Vec<(String, ToolOutcome)>>`，confirm 与 post_message 共用，避免两份执行逻辑 drift（项目历史踩过 dual-path drift）。

mod.rs 注册（:773-784 现有 management-agent 路由块内追加）：

```rust
.route("/management-agent/commands/:id/confirm", post(confirm_management_command))
.route("/management-agent/commands/:id/reject", post(reject_management_command))
```

并在 mod.rs:232 的 use 列表加 `confirm_management_command, reject_management_command`。

- [ ] **Step 4: 运行测试 + cargo check + 全量 lib**

Run: `cargo test --lib confirm_filter_only_targets_pending_confirmation 2>&1 | tail -10`
Expected: PASS
Run: `cargo check 2>&1 | tail -5`
Expected: Finished（无编译错误）
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs src/routes/mod.rs
git commit -m "feat(mgmt-agent): confirm/reject 端点闭合提议→确认→执行循环(乐观锁仿principal resolve)"
```

---

## Task 6: 扩工具集 — 配置/策略/知识工具声明 + 分发

**Files:**
- Modify: `src/routes/management.rs`（merge_product_tools 加工具声明 + execute_management_tool 加 match 分支 + build_management_plan prompt 让 LLM 认识新工具）
- Test: `src/routes/management.rs` 内 tests

**Interfaces:**
- Consumes: 已有 REST 写入函数（domain_profiles publish/activate、knowledge verify/reject、operation_domains update 等）。
- Produces: 新工具在 catalog 公布 + execute 分发到对应已有逻辑。

> 范围（第一期放权全接，spec §4.1）：query_runs / query_metrics / query_health / query_inbox（readonly）；update_operation_domain / set_assist_mode（low）；publish_domain_profile / activate_domain_profile / publish_prompt_template / verify_knowledge_chunk / reject_knowledge_chunk（dangerous）。

- [ ] **Step 1: 写失败测试（catalog 含新工具 + 分发不打到未公布工具）**

```rust
#[test]
fn merged_catalog_includes_new_admin_tools() {
    use serde_json::json;
    let merged = merge_product_tools(json!({ "tools": [] }));
    let names = advertised_tool_names(&merged);
    for t in ["wechatagent.query_runs", "wechatagent.publish_domain_profile",
              "wechatagent.verify_knowledge_chunk", "wechatagent.update_operation_domain"] {
        assert!(names.contains(t), "catalog 缺工具 {t}");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib merged_catalog_includes_new_admin_tools 2>&1 | tail -10`
Expected: FAIL（新工具未在 merge_product_tools 声明）

- [ ] **Step 3: merge_product_tools 加工具声明 + execute_management_tool 加分发**

在 `merge_product_tools` 的 `product_tools` vec（management.rs:415-444）追加（每个 `json!({"name":..., "description":...})`，描述写清参数）。execute_management_tool 的 match（:700）加分支，调用已有 REST handler 的内部逻辑或直接复用其 DB 写入。

> 实现注意：dangerous 工具（publish/verify）调用已有写入逻辑，**不新建执行端点**——找到对应 routes 模块的写入函数复用。verify_knowledge_chunk 调已有 chunk verify 逻辑（人确认动作，非 AI auto-verify，守红线）。每个新工具执行后，结果交 Task 2 的 assert_tool_outcome 核实。

每个新工具配 build_management_plan 的 prompt（management.rs build_management_plan 内的工具说明）让 LLM 知道何时选它——但 prompt 措辞改动需确认是否 bump prompt 版本（grep PROMPT_PACK_VERSION，若 management.plan prompt 走 prompt_versions 机制则照其约定）。

- [ ] **Step 4: 运行测试 + cargo check + 全量 lib + no-takeover lint**

Run: `cargo test --lib merged_catalog_includes_new_admin_tools 2>&1 | tail -10`
Expected: PASS
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 无禁词（PASS）

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "feat(mgmt-agent): 扩工具集接配置/策略/知识端点(第一期放权全接,守AI永不自动verify)"
```

---

## Task 7: 前端 command-center 做厚 — 确认 UI + 真实结果展示

**Files:**
- Modify: `frontend/src/features/command-center/index.tsx`
- Modify: `frontend/src/stores/commandStore.ts`
- Test: `frontend/src/__tests__/features/command-center/commandCenter.test.tsx`

**Interfaces:**
- Consumes: 后端 confirm/reject 端点、command run 的 status（pending_confirmation / succeeded / failed）、tool call 的 status（含 executed_unverified）。
- Produces: 确认/否决按钮 + 每个 tool 真实终态展示。

- [ ] **Step 1: 读现有 command-center 结构**

Run: `cat frontend/src/features/command-center/index.tsx | head -80; cat frontend/src/stores/commandStore.ts`
理解现有对话流渲染 + store action，遵守 docs/frontend-design-system.md。

- [ ] **Step 2: 写失败测试（pending_confirmation 显示确认按钮 + executed_unverified 显示待核实）**

在 commandCenter.test.tsx 加用例：mock 一个 status=pending_confirmation 的 command → 断言渲染确认/否决按钮；mock tool call status=executed_unverified → 断言显示"待核实"。

- [ ] **Step 3: 运行确认失败**

Run: `cd frontend && npx vitest run --no-file-parallelism src/__tests__/features/command-center/commandCenter.test.tsx 2>&1 | tail -15`
Expected: FAIL

- [ ] **Step 4: 实现 — store 加 confirm/reject action + 组件渲染确认卡和真实终态**

commandStore 加 `confirmCommand(id)` / `rejectCommand(id)` 调后端端点 + refetch。组件：command status=pending_confirmation 时渲染 plan 预览 + 确认/否决按钮；tool call 按 status 显 ✅/❌原因/⚠️待核实（复用 ask-human 收件箱的 ReviewQueue/确认原语若适用）。

- [ ] **Step 5: 运行测试 + tsc**

Run: `cd frontend && npx vitest run --no-file-parallelism src/__tests__/features/command-center/commandCenter.test.tsx 2>&1 | tail -15`
Expected: PASS
Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -10`
Expected: 无类型错误

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/command-center/ frontend/src/stores/commandStore.ts frontend/src/__tests__/features/command-center/
git commit -m "feat(mgmt-agent): command-center 做厚-确认UI+真实结果展示(✅/❌/⚠️待核实)"
```

---

## Self-Review

**Spec coverage**：
- §2 闭合循环 → Task 5（confirm/reject 端点）✓
- §3 执行结果核实 → Task 2（assert_tool_outcome）+ Task 3（汇报基于真实结果）✓
- §4.1 工具集扩 → Task 6 ✓
- §4.2 风险档代码裁定 → Task 1 ✓
- §1.2 第一期放权 → Task 1（plan_requires_confirmation 开关默认关）+ Task 6（全接）✓
- §5.1 status 闭集 → Task 4 ✓
- §5.3 前端 → Task 7 ✓
- 红线（AI 永不自动 verify）→ Task 6 注（verify 是人确认动作，无 auto-verify 工具）✓

**Placeholder scan**：无 TBD/TODO；execute_plan_tool_calls 抽取在 Task 5 明确；新工具分发在 Task 6 指明"复用已有写入函数"（具体函数实现时 grep 定位，因 routes 模块多）。

**Type consistency**：ToolRisk（Task 1）/ ToolOutcome（Task 2）/ build_execution_summary（Task 3）/ TOOL_CALL_STATUSES（Task 4）/ build_confirm_filter + execute_plan_tool_calls（Task 5）签名一致；read_only 字段名保留兼容既有调用点。

**已知留实现期定位**：Task 6 各 dangerous 工具复用的具体 REST 写入函数名（需 grep 对应 routes 模块）；Task 6 prompt 改动是否 bump 版本（依 prompt_versions 机制）。这些是定位类，非设计缺口。
