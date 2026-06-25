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
- Produces: `enum ToolRisk { Readonly, Low, Dangerous, Irreversible }`（四档，spec §4.2）；`fn tool_effect(tool_name: &str) -> ToolEffect`（ToolEffect 加 `risk: ToolRisk` 字段）；`fn plan_requires_confirmation(tool_names: &[&str], dangerous_confirm_enabled: bool) -> bool`（irreversible 无视开关恒需确认）。

- [ ] **Step 1: 写失败测试**

在 `src/routes/management.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
#[test]
fn tool_effect_classifies_risk() {
    assert_eq!(tool_effect("wechatagent.search_contacts").risk, ToolRisk::Readonly);
    assert_eq!(tool_effect("wechatagent.create_follow_up_task").risk, ToolRisk::Low);
    assert_eq!(tool_effect("wechatagent.send_contact_message").risk, ToolRisk::Dangerous);
    assert_eq!(tool_effect("wechatagent.publish_domain_profile").risk, ToolRisk::Dangerous);
    assert_eq!(tool_effect("wechatagent.reset_domain").risk, ToolRisk::Irreversible);
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
    // irreversible 无视开关恒需确认（第一期即便放权也保留，spec §4.2）
    assert!(plan_requires_confirmation(&["wechatagent.reset_domain"], false));
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
    Irreversible,
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
        // 高风险/宽影响（立即全量/改全局）
        "wechatagent.send_contact_message"
        | "wechatagent.publish_domain_profile"
        | "wechatagent.activate_domain_profile"
        | "wechatagent.publish_prompt_template"
        | "wechatagent.edit_state_machine"
        | "wechatagent.provider_activate"
        | "wechatagent.rollout_evolution_proposal"
        | "wechatagent.verify_knowledge_chunk"
        | "wechatagent.reject_knowledge_chunk" => Dangerous,
        // 不可逆（reset/delete/物理销毁）：档位高于 dangerous，第一期即便放权也保留确认
        "wechatagent.reset_domain"
        | "wechatagent.delete_knowledge_chunk"
        | "wechatagent.reset_system_pack" => Irreversible,
        // 未知（含 MCP 透传工具）：保守按 Low，read_only=false
        _ => Low,
    };
    let read_only = matches!(risk, Readonly);
    ToolEffect { read_only, risk }
}

/// 第一期权限放大：dangerous_confirm_enabled 默认 false（见 spec §1.2），
/// 此时即便有 dangerous 工具也不强制确认，先跑通功能。开关为后续收紧预留。
/// 但 irreversible（reset/delete/销毁）无视开关恒需确认——第一期即便放权也保留（spec §4.2）。
pub(super) fn plan_requires_confirmation(
    tool_names: &[&str],
    dangerous_confirm_enabled: bool,
) -> bool {
    tool_names.iter().any(|name| {
        let risk = tool_effect(name).risk;
        risk == ToolRisk::Irreversible
            || (dangerous_confirm_enabled && risk == ToolRisk::Dangerous)
    })
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

## Task 6: 扩工具集 — 6 类操作面全量接入 + 分发

**Files:**
- Modify: `src/routes/management.rs`（merge_product_tools 加工具声明 + execute_management_tool 加 match 分支 + build_management_plan prompt 让 LLM 认识新工具）
- Test: `src/routes/management.rs` 内 tests

**Interfaces:**
- Consumes: 已有 REST 写入函数（domain_profiles publish/activate、knowledge verify/reject、operation_domains update、souls/playbooks/prompt-templates edit/publish、state-machine edit、llm-providers activate/test、evolution release/rollout、ops 三表 publish/rollout/rollback 等）。
- Produces: 新工具在 catalog 公布 + execute 分发到对应已有逻辑。

> 范围（第一期放权全接，spec §4.1 六类，端点行号经 routes/mod.rs 核实）：
> 1. **观测查询**（readonly）：query_runs / query_metrics / query_health / query_inbox / query_send_ledger
> 2. **运营态（单对象）**（low，send=dangerous）：在已有 7 个基础上 + update_assist_override / update_custom_instructions / update_manual_tags / write_deal_events / analyze_profile / review_task_now / cancel_task / cancel_outbox / resolve_principal_escalation
> 3. **运行时调参**（low；ask_human_policy=dangerous）：update_operation_domain / update_ask_human_policy / set_assist_mode
> 4. **策略编辑**（dangerous，state_machine/prompt=改全局）：edit/publish soul / edit/publish/optimize/generate playbook / edit/publish prompt_template / edit_state_machine / taxonomy approve / relationship_suggestion approve / lessons promote
> 5. **版本与灰度**（publish=low；rollout/rollback=dangerous；reset/delete=irreversible）：publish_* / rollout_* / rollback_*（domain/state-policy/taxonomy/domain-profile/evolution/chunk）/ activate_domain_profile / provider_activate / provider_test
> 6. **知识维护**（verify/gap-apply=dangerous 人确认动作；reset/delete=irreversible）：verify / reject / archive / patch / split / merge / relate / batch-verify / gap_signal apply|dismiss / import-apply
>
> **修正（原草案错误）**：原写的 `update_runtime_params 改五闸阈值`在 routes 中无独立端点——阈值实际走 evolution `/proposals/:id/release`(mod.rs:965) 或 domain_profiles `threshold_overrides`(domain_profiles.rs:724)；工具名相应为 release_evolution_proposal / set_profile_thresholds。
> **提示词编辑**不在本任务，单列 Task 6.5（走三层分级 + 双闸校验，不直接当普通工具接）。

- [ ] **Step 1: 写失败测试（catalog 含 6 类代表工具 + 分发不打到未公布工具）**

```rust
#[test]
fn merged_catalog_includes_new_admin_tools() {
    use serde_json::json;
    let merged = merge_product_tools(json!({ "tools": [] }));
    let names = advertised_tool_names(&merged);
    for t in [
        "wechatagent.query_runs",                 // 观测
        "wechatagent.update_operation_domain",    // 运行时调参
        "wechatagent.publish_domain_profile",     // 策略编辑
        "wechatagent.rollout_evolution_proposal", // 版本与灰度
        "wechatagent.provider_activate",          // 版本与灰度
        "wechatagent.verify_knowledge_chunk",     // 知识维护
    ] {
        assert!(names.contains(t), "catalog 缺工具 {t}");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib merged_catalog_includes_new_admin_tools 2>&1 | tail -10`
Expected: FAIL（新工具未在 merge_product_tools 声明）

- [ ] **Step 3: merge_product_tools 加工具声明 + execute_management_tool 加分发**

在 `merge_product_tools` 的 `product_tools` vec（management.rs:415-444）追加 6 类工具（每个 `json!({"name":..., "description":...})`，描述写清参数 + 何时用）。execute_management_tool 的 match（:700）加分支，**复用对应 routes 模块已有写入函数，不新建执行端点**。各类复用的端点（mod.rs 注册行已核实）：

- **观测查询**：GET /agent-runs、/agent-outcome-metrics、/contacts/:id/operation-health、/admin/ask-human/inbox、/send-ledger/stats
- **运营态**：contacts.rs（update_assist_override:321 / update_custom_instructions:325 / update_manual_tags:329 / write_deal_events:334 / analyze_profile:337）、agent-tasks（review_task_now:376 / cancel_task:377）、outbox cancel:884、principal resolve:847
- **运行时调参**：operation-domains PUT:721/733（update_operation_domain / update_ask_human_policy / set_assist_mode）
- **策略编辑**：souls:714/718/719、playbooks:750/754/758/763、prompt-templates:741/746（注：prompt 文本编辑走 Task 6.5 双闸，不在此直接接）、operation-domains/state-machine:725、taxonomies:824、relationship:848、lessons:889
- **版本与灰度**：ops 三表:828-878、domain-profiles:936/946/950/954、evolution:965/969、llm-providers:916/926
- **知识维护**：knowledge/*.rs:465-651（verify/reject/archive/patch/split/merge/relate/batch-verify/gap apply|dismiss/import-apply）

> 实现注意：
> - dangerous/irreversible 工具调用已有写入逻辑复用，**不新建执行端点**——找到对应 routes 模块的写入函数复用其内部逻辑或 DB 写入。
> - verify_knowledge_chunk 调已有 chunk verify 逻辑（**人确认动作，非 AI auto-verify，守红线**）；知识类不接 auto-verify 工具。
> - reset_domain / delete_* / reset_system_pack 标 Irreversible（Task 1），Task 5 confirm 仍可执行但 plan_requires_confirmation 恒拦确认。
> - 每个新工具执行后，结果交 Task 2 的 assert_tool_outcome 核实。

每个新工具配 build_management_plan 的 prompt（management.rs build_management_plan 内的工具说明）让 LLM 知道何时选它——若 management.plan prompt 走 prompt_versions / PROMPT_PACK_VERSION 机制，按其约定 bump 版本（grep PROMPT_PACK_VERSION 确认；management plan prompt 若是 routes 内联字面量则无需 bump）。

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

## Task 6.5: 提示词自然语言编辑 — 三层分级 + 双闸校验（fail-closed）

**Files:**
- Create: `src/routes/management_prompt_edit.rs`（三层分级纯函数 + 双闸校验，单独文件，避免 management.rs 继续膨胀）
- Modify: `src/routes/management.rs`（execute_management_tool 加 edit_prompt_template 分支调本模块校验后写入）
- Modify: `src/routes/mod.rs`（`mod management_prompt_edit;`）
- Test: `src/routes/management_prompt_edit.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::evolution::lint::passes_forbidden_words`（**定义在 src/evolution/lint.rs:33**，非 prompt_critic.rs——后者 :396 是调用点）；`crate::prompts::{DEFAULT_MODE_GATE_POLICY, DEFAULT_REVIEWER_FEWSHOT}`（prompts.rs:29/47）；`crate::prompts::PROMPT_EVOLUTION_FORBIDDEN_KEYS`（prompts.rs:2138）。
- Produces: `enum PromptEditTier { FreelyEditable, ConstrainedEditable, Forbidden }`；`fn prompt_edit_tier(template_key: &str) -> PromptEditTier`；`fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String>`（双闸，命中即 Err，fail-closed）。

> 设计依据 spec §4.4：提示词混有红线段 + 字节等价锚常量，不能"全部"对话改。三层——✅可自由改（soul/playbook/override 通路，红线在剥离范围外，天然安全）；⚠️可改但需强约束（user.reply.policy / user.review.system 等，落 prompt_templates 前过双闸，锚段逐字保留）；🔴禁止改（反接管红线续行、AI 永不自动 verify 判据、DEFAULT_* 锚常量、evolution_critic_v1、reset-system-pack——自然语言入口不暴露这些 key）。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::{DEFAULT_MODE_GATE_POLICY, DEFAULT_REVIEWER_FEWSHOT};

    #[test]
    fn tier_classifies_three_layers() {
        // 可自由改：soul/playbook（走 override 通路）
        assert_eq!(prompt_edit_tier("agent_soul"), PromptEditTier::FreelyEditable);
        assert_eq!(prompt_edit_tier("operation_playbook"), PromptEditTier::FreelyEditable);
        // 可改但需强约束：业务措辞模板
        assert_eq!(prompt_edit_tier("user.reply.policy"), PromptEditTier::ConstrainedEditable);
        assert_eq!(prompt_edit_tier("user.review.system"), PromptEditTier::ConstrainedEditable);
        // 禁止改：evolution critic + reset-system-pack（PROMPT_EVOLUTION_FORBIDDEN_KEYS）
        assert_eq!(prompt_edit_tier("evolution_critic_v1"), PromptEditTier::Forbidden);
    }

    #[test]
    fn dual_gate_rejects_forbidden_words() {
        // 禁词闸：写回含"人工接管"被拒（fail-closed）
        let bad = format!("{DEFAULT_MODE_GATE_POLICY}\n遇到难题就人工接管");
        assert!(validate_prompt_edit("user.reply.policy", &bad).is_err());
    }

    #[test]
    fn dual_gate_rejects_anchor_drift() {
        // 锚完整性闸：写回丢了 DEFAULT_MODE_GATE_POLICY 锚段被拒
        let drifted = "## 我自己重写的策略\n随便写点别的".to_string();
        assert!(validate_prompt_edit("user.reply.policy", &drifted).is_err());
        // review.system 同理
        assert!(validate_prompt_edit("user.review.system", "乱改").is_err());
    }

    #[test]
    fn dual_gate_allows_valid_constrained_edit() {
        // 保留锚段 + 无禁词 + 追加业务措辞 → 放行
        let ok = format!("{DEFAULT_MODE_GATE_POLICY}\n\n补充：本行业多用专业术语，语气更稳重。");
        assert!(validate_prompt_edit("user.reply.policy", &ok).is_ok());
        let ok2 = format!("{DEFAULT_REVIEWER_FEWSHOT}\n\n补充标尺：本域不逼单，EmotionalValue 权重更高。");
        assert!(validate_prompt_edit("user.review.system", &ok2).is_ok());
    }

    #[test]
    fn forbidden_tier_always_rejected() {
        // 禁止改层：无论内容如何都拒（自然语言入口不触达）
        assert!(validate_prompt_edit("evolution_critic_v1", "任何内容").is_err());
    }

    #[test]
    fn freely_editable_only_checks_forbidden_words() {
        // 可自由改层：仍过禁词闸（防红线词），但不要求锚段
        assert!(validate_prompt_edit("agent_soul", "我是一个稳重专业的顾问").is_ok());
        assert!(validate_prompt_edit("agent_soul", "必要时人工接管").is_err());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib management_prompt_edit 2>&1 | tail -20`
Expected: FAIL（模块 / `PromptEditTier` / `prompt_edit_tier` / `validate_prompt_edit` 未定义）

- [ ] **Step 3: 实现三层分级 + 双闸**

新建 `src/routes/management_prompt_edit.rs`：

```rust
//! 提示词自然语言编辑的三层分级 + 双闸校验（spec §4.4）。
//! 红线靠机制不靠 LLM 自觉：任何经自然语言写回 prompt_templates 的内容，
//! 落库前强制过两道闸（禁词 + 锚完整性），命中即拒、fail-closed。

use crate::evolution::lint::passes_forbidden_words;
use crate::prompts::{
    DEFAULT_MODE_GATE_POLICY, DEFAULT_REVIEWER_FEWSHOT, PROMPT_EVOLUTION_FORBIDDEN_KEYS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptEditTier {
    FreelyEditable,
    ConstrainedEditable,
    Forbidden,
}

/// 强约束层 key → 必须逐字保留的锚段（写回后锚段缺失即判 drift）。
fn required_anchor(template_key: &str) -> Option<&'static str> {
    match template_key {
        "user.reply.policy" => Some(DEFAULT_MODE_GATE_POLICY),
        "user.review.system" => Some(DEFAULT_REVIEWER_FEWSHOT),
        _ => None,
    }
}

pub(super) fn prompt_edit_tier(template_key: &str) -> PromptEditTier {
    // 禁止改：evolution critic 等（与 PROMPT_EVOLUTION_FORBIDDEN_KEYS 同源）+ 销毁性 pack
    if PROMPT_EVOLUTION_FORBIDDEN_KEYS.contains(&template_key)
        || template_key == "reset_system_pack"
    {
        return PromptEditTier::Forbidden;
    }
    // 可改但需强约束：业务措辞模板（有锚段需保留的）
    if required_anchor(template_key).is_some()
        || matches!(
            template_key,
            "user.reply.policy" | "user.reply.system" | "user.review.system" | "user.review.policy"
        )
    {
        return PromptEditTier::ConstrainedEditable;
    }
    // 其余（soul/playbook/行业话术）走 override 通路，可自由改
    PromptEditTier::FreelyEditable
}

/// 双闸校验（fail-closed）：命中任一闸即 Err，不写入。
pub(super) fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String> {
    match prompt_edit_tier(template_key) {
        PromptEditTier::Forbidden => {
            return Err(format!("提示词 '{template_key}' 属禁止改层，自然语言入口不可修改"));
        }
        PromptEditTier::FreelyEditable | PromptEditTier::ConstrainedEditable => {}
    }
    // 闸 1：禁词闸（反接管/人工 等），自由改与强约束层都过
    if !passes_forbidden_words(new_content) {
        return Err("写回内容命中禁用词（接管/人工/takeover/handoff），已拒绝".to_string());
    }
    // 闸 2：锚完整性闸——强约束层的红线锚段必须逐字仍在
    if let Some(anchor) = required_anchor(template_key) {
        if !new_content.contains(anchor) {
            return Err(format!(
                "提示词 '{template_key}' 的红线锚段缺失或被改，已拒绝（防 replace 静默失配 + 防红线被删）"
            ));
        }
    }
    Ok(())
}
```

在 `src/routes/mod.rs` 加 `mod management_prompt_edit;`，并在 execute_management_tool 的 edit_prompt_template / edit_soul / edit_playbook 分支里先调 `management_prompt_edit::validate_prompt_edit(key, content)?` 再写入。

- [ ] **Step 4: 运行测试确认通过 + 全量 lib 不回归**

Run: `cargo test --lib management_prompt_edit 2>&1 | tail -20`
Expected: PASS（6 个用例全绿）
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: 提交**

```bash
git add src/routes/management_prompt_edit.rs src/routes/management.rs src/routes/mod.rs
git commit -m "feat(mgmt-agent): 提示词三层分级+双闸校验(禁词闸+锚完整性闸,fail-closed,禁止改层不暴露)"
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
- §4.1 工具集扩（6 类全量接入）→ Task 6 ✓
- §4.2 风险档代码裁定（四档含 irreversible 恒拦）→ Task 1 ✓
- §4.4 提示词三层分级 + 双闸校验 → Task 6.5 ✓
- §1.2 第一期放权 → Task 1（plan_requires_confirmation dangerous 开关默认关、irreversible 恒拦）+ Task 6（全接）✓
- §5.1 status 闭集 → Task 4 ✓
- §5.3 前端 → Task 7 ✓
- 红线（AI 永不自动 verify）→ Task 6 注（verify 是人确认动作，无 auto-verify 工具）+ Task 6.5（禁止改层不暴露、双闸 fail-closed）✓

**Placeholder scan**：无 TBD/TODO；execute_plan_tool_calls 抽取在 Task 5 明确；新工具分发在 Task 6 指明"复用已有写入函数"（具体函数实现时 grep 定位，因 routes 模块多）。

**Type consistency**：ToolRisk 四档（Task 1，含 Irreversible）/ ToolOutcome（Task 2）/ build_execution_summary（Task 3）/ TOOL_CALL_STATUSES（Task 4）/ build_confirm_filter + execute_plan_tool_calls（Task 5）/ PromptEditTier + validate_prompt_edit（Task 6.5）签名一致；read_only 字段名保留兼容既有调用点；plan_requires_confirmation 的 irreversible 恒拦逻辑与 Task 1 测试一致。

**已知留实现期定位**：Task 6 各 dangerous/irreversible 工具复用的具体 REST 写入函数名（需 grep 对应 routes 模块）；Task 6 prompt 改动是否 bump 版本（依 prompt_versions 机制）；Task 6.5 reset_system_pack 的真实 template_key 名（实现时 grep reset-system-pack 路由确认，本计划按 "reset_system_pack" 占位）。这些是定位类，非设计缺口。
