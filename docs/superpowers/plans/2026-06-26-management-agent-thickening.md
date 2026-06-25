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
    // verify 类无视开关恒需确认（spec §4.3：AI 调 verify 会落 source=Human，
    // 守"AI 永不自动 verify"——确认门不随第一期 dangerous 开关放行）
    assert!(plan_requires_confirmation(&["wechatagent.verify_knowledge_chunk"], false));
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

/// verify 类工具：把 chunk 推向 verified 的动作。它写 source=Human（verify.rs:101），
/// 包成 AI 工具会"AI 调用被记成人确认"——故恒强制确认，不随第一期开关放行（spec §4.3）。
pub(super) fn tool_always_requires_confirmation(tool_name: &str) -> bool {
    matches!(tool_name, "wechatagent.verify_knowledge_chunk")
}

/// 第一期权限放大：dangerous_confirm_enabled 默认 false（见 spec §1.2），
/// 此时即便有 dangerous 工具也不强制确认，先跑通功能。开关为后续收紧预留。
/// 但 irreversible（reset/delete/销毁）+ verify 类（AI 永不自动 verify）无视开关
/// 恒需确认——第一期即便放权也保留（spec §4.2/§4.3）。
pub(super) fn plan_requires_confirmation(
    tool_names: &[&str],
    dangerous_confirm_enabled: bool,
) -> bool {
    tool_names.iter().any(|name| {
        let risk = tool_effect(name).risk;
        risk == ToolRisk::Irreversible
            || tool_always_requires_confirmation(name)
            || (dangerous_confirm_enabled && risk == ToolRisk::Dangerous)
    })
}
```

注意：原 `tool_effect` 的调用点（management.rs:291 `!tool_effect(&c.tool_name).read_only`、:667 `is_read_tool`、:671 `should_dry_run_tool`）依赖 `read_only` 字段——保持 `read_only` 字段名不变，已兼容。**实现期真实函数名是 `is_read_tool`（management.rs:667），不是 `is_read_only_tool`**（旧草案笔误，已核实）。ToolEffect 真实区域是 management.rs:648-673（含中间函数 `is_read_tool`:667），替换时勿误删。

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
> - dangerous/irreversible 工具调用已有写入逻辑复用，**不新建执行端点**——找到对应 routes 模块的写入函数复用其内部逻辑或 DB 写入。这些 handler 多为 `pub(super)`，management.rs 同属 `crate::routes` 可直接命名调用（mod.rs 已有大量 `pub use` 直调先例）；`AuthenticatedAdmin`(auth/mod.rs:59 仅 user_id/username/current_workspace 三 String) 可平凡构造，请求体 derive Deserialize 可 `serde_json::from_value` 从工具 arguments 构造。
> - **需小重构的 3 个 handler**（核实发现）：`cancel_outbox`(admin_outbox.rs:117)、`approve_taxonomy_candidate`(admin_taxonomy_candidates.rs:123)、`approve_relationship_suggestion`(admin_relationship_suggestions.rs:109) 返回的是 `Result<Response>` 而非 `Json<Value>`，execute_management_tool 拿不到结构化 Value 喂 assert_tool_outcome → 需先抽内部 fn 返回 Value。另 `approve_taxonomy_candidate` **没有 `Extension<AuthenticatedAdmin>` 参数**，复用前要先确认它的 workspace 来源。
> - **import-apply-pdf 包不动**：`import_operation_knowledge_apply_pdf`(import.rs:428) 用 `Multipart` 提取器，无法从 JSON 工具参数构造；改用字节级 helper `import_pdf_bytes`（已是公开 helper）。其余 import-apply / import-apply-image 直接可复用。
> - **verify_knowledge_chunk 红线（spec §4.3，亲核 verify.rs:101/110）**：该 handler 写 `source=ProvenanceSource::Human, actor=admin.username`，假设调用方=人点按钮。包成工具时 **verify 类恒强制确认**（Task 1 的 plan_requires_confirmation 把 verify 类与 irreversible 同档恒拦，不随 dangerous 开关放行），且执行时 actor 标管理者本人——保留"人确认"真实语义，守"AI 永不自动 verify"。知识类不接 auto-verify 工具。
> - reset_domain / delete_* / reset_system_pack 标 Irreversible（Task 1），Task 5 confirm 仍可执行但 plan_requires_confirmation 恒拦确认。
> - 每个新工具执行后，结果交 Task 2 的 assert_tool_outcome 核实。

每个新工具配 build_management_plan 的 prompt 让 LLM 知道何时选它。**核实结论（原草案"内联字面量"判断错）**：build_management_plan(management.rs:1119) 是 LLM 调用(`generate_agent_json`:1146)，其 system/policy prompt 走 `prompts::load_prompt(db,ws,"management.plan.system"/"management.plan.policy")`(:1129/1135) —— 是 **PROMPT_PACK_VERSION 版本化的 PromptSpec**(prompts.rs:1499/1521)，不是 routes 内联字面量。工具清单是**动态序列化进 user 消息**(:1142-1145)、不写在 system/policy 文本里，所以**只加工具、不改这两个 PromptSpec 文本 → 可不 bump**；但若为让 LLM 更好认识新工具去改 system/policy 文本，**必须 bump PROMPT_PACK_VERSION**(prompts.rs:15)。

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

> **本任务经 4 路 opus 实证核实重写**。原草案有红线漏洞：①key 命名半数错（agent_soul/operation_playbook 不是 template_key，user.review.policy 不存在，reset_system_pack 是 handler）；②锚闸只校验 `DEFAULT_MODE_GATE_POLICY`，而该锚**故意不含反接管红线**（亲核 prompts.rs:29-34 + 测试 `default_mode_gate_policy_excludes_human_takeover_redline` prompts.rs:2506-2511）——真红线在 user.reply.policy 正文 :1123/:1146、soul 正文 :968，旧锚闸根本没在查。修正核心：**先抽红线为独立锚常量再扩锚闸**（用户 2026-06-26 决策；行号为合并 origin/main 后真实值）。

**Files:**
- Modify: `src/prompts.rs`（新增红线锚常量 `DEFAULT_REPLY_REDLINE_ANCHORS` + 锚漂移护栏测试）
- Create: `src/routes/management_prompt_edit.rs`（三层分级纯函数 + 双闸校验）
- Modify: `src/routes/prompt_templates.rs:138 update_prompt_template`（真正拦截点：在 `validate_prompt_template_input`(:144) 之后插双闸 + `$set` 加 `seeded_by="manual"` 防 PR#42 启动对齐覆盖）
- Modify: `src/routes/mod.rs`（`mod management_prompt_edit;`）
- Test: `src/routes/management_prompt_edit.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::evolution::lint::passes_forbidden_words`（**定义在 src/evolution/lint.rs:33**，返回 true=干净；prompt_critic.rs:396 是调用点）；`crate::prompts::{normalize_prompt_content(:162), DEFAULT_MODE_GATE_POLICY(:29), DEFAULT_REVIEWER_FEWSHOT(:47), DEFAULT_REPLY_REDLINE_ANCHORS(新增), PROMPT_EVOLUTION_FORBIDDEN_KEYS(:2261)}`。
- Produces: `enum PromptEditTier { FreelyEditable, ConstrainedEditable, Forbidden }`；`fn prompt_edit_tier(template_key: &str) -> PromptEditTier`；`fn required_anchors(template_key: &str) -> &'static [&'static str]`；`fn validate_prompt_edit(template_key: &str, new_content: &str) -> Result<(), String>`（双闸，命中即 Err，fail-closed）。

> 设计依据 spec §4.4（已按核实修正）。三层（按**真实 template_key**）：
> - 🔴**禁止改**：`evolution_critic_v1`（PROMPT_EVOLUTION_FORBIDDEN_KEYS）。`reset-system-pack` 是 route handler 不是 key，靠不接入工具来禁、不在本函数判。
> - ⚠️**可改但需强约束**（落库前过双闸，红线锚逐字保留）：`user.reply.policy`（含反接管红线 :1123/:1146）、`user.reply.system`、`user.review.system`、`user.reply.task`（soul 红线 :968 注入此层）。
> - ✅**可自由改**（仍过禁词闸）：其余业务话术 key。
> 注：soul/playbook 在独立集合（`agent_souls`/`operation_playbooks`，标识字段 `agent_kind`），**不走 prompt_templates 编辑通路**，故不在本函数的 template_key 分类内；它们的编辑若开放，另在 souls/playbooks handler 处接同一套 `validate_prompt_edit`（用 agent_kind 映射到对应红线锚）。本任务先覆盖 prompt_templates 通路。

- [ ] **Step 0: 先在 prompts.rs 抽红线锚常量 + 护栏测试**

亲核确认（合并 origin/main 后真实行号）：反接管红线在 `user.reply.policy` 正文 prompts.rs:**1123**（"用户要求真人…严禁承诺安排真人…"整段）和 :**1146**（"用户主动要真人…严禁承诺…"整段）。把这两段**逐字**抽成常量（保持与正文字节一致，正文保留副本 + 护栏测试锁死一致性，仿 DEFAULT_MODE_GATE_POLICY 的 `..._anchor_matches_pack` 模式）：

```rust
// prompts.rs：新增（红线锚——锚闸据此校验写回后红线逐字仍在）
pub const DEFAULT_REPLY_REDLINE_ANCHORS: &[&str] = &[
    // :1123 boundary_protection 反接管续行（真实逐字子串）
    "用户要求\"真人 / 不想跟机器人聊\"时，用 AI 自治语义承接",
    // :1146 表达红线反接管段（真实逐字子串）
    "严禁承诺\"安排真人 / 让同事来联系 / 稍后有人对接你 / 转接客服\"",
];
```

> 实现注意：①常量值必须是正文里**真实存在的逐字子串**（实现时从 prompts.rs:1123/1146 复制，上面两条已与合并后正文核对一致）。②加护栏测试 `reply_redline_anchors_present_in_pack`：断言 `user.reply.policy` 的 pack content `contains` 每条锚，防正文改动后锚失配（仿 prompts.rs:2456-2520 的 anchor_matches_pack）。③这两条锚是子串、不跨行，CRLF 影响小，但锚闸统一走 normalize（见 Step 3 注）更稳。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::{DEFAULT_MODE_GATE_POLICY, DEFAULT_REVIEWER_FEWSHOT, DEFAULT_REPLY_REDLINE_ANCHORS};

    #[test]
    fn tier_classifies_three_layers() {
        // 强约束：含红线/锚的业务模板
        assert_eq!(prompt_edit_tier("user.reply.policy"), PromptEditTier::ConstrainedEditable);
        assert_eq!(prompt_edit_tier("user.reply.system"), PromptEditTier::ConstrainedEditable);
        assert_eq!(prompt_edit_tier("user.review.system"), PromptEditTier::ConstrainedEditable);
        assert_eq!(prompt_edit_tier("user.reply.task"), PromptEditTier::ConstrainedEditable);
        // 禁止改：evolution critic（PROMPT_EVOLUTION_FORBIDDEN_KEYS）
        assert_eq!(prompt_edit_tier("evolution_critic_v1"), PromptEditTier::Forbidden);
        // 可自由改：其余业务话术 key
        assert_eq!(prompt_edit_tier("knowledge.chat.draft_chunk"), PromptEditTier::FreelyEditable);
    }

    #[test]
    fn dual_gate_rejects_forbidden_words() {
        // 禁词闸：写回含"人工接管"被拒（fail-closed）
        let bad = format!("{DEFAULT_MODE_GATE_POLICY}\n遇到难题就人工接管");
        assert!(validate_prompt_edit("user.reply.policy", &bad).is_err());
    }

    #[test]
    fn dual_gate_rejects_business_anchor_drift() {
        // 锚完整性闸：写回丢了 DEFAULT_MODE_GATE_POLICY 业务锚被拒
        let drifted = "## 我自己重写的策略\n随便写点别的".to_string();
        assert!(validate_prompt_edit("user.reply.policy", &drifted).is_err());
        assert!(validate_prompt_edit("user.review.system", "乱改").is_err());
    }

    #[test]
    fn dual_gate_rejects_redline_anchor_drift() {
        // 核心修正：保留业务锚 DEFAULT_MODE_GATE_POLICY，但删掉反接管红线段 → 仍须被拒
        // （旧设计这里会放行 = 红线漏洞）
        let keeps_business_drops_redline = format!("{DEFAULT_MODE_GATE_POLICY}\n业务措辞随便加");
        assert!(
            validate_prompt_edit("user.reply.policy", &keeps_business_drops_redline).is_err(),
            "保留业务锚但丢红线锚必须被拒"
        );
    }

    #[test]
    fn dual_gate_allows_valid_constrained_edit() {
        // 保留全部锚（业务锚 + 红线锚）+ 无禁词 + 追加业务措辞 → 放行
        let redlines: String = DEFAULT_REPLY_REDLINE_ANCHORS.join("\n");
        let ok = format!("{DEFAULT_MODE_GATE_POLICY}\n{redlines}\n\n补充：本行业语气更稳重。");
        assert!(validate_prompt_edit("user.reply.policy", &ok).is_ok());
        let ok2 = format!("{DEFAULT_REVIEWER_FEWSHOT}\n\n补充标尺：本域不逼单。");
        assert!(validate_prompt_edit("user.review.system", &ok2).is_ok());
    }

    #[test]
    fn forbidden_tier_always_rejected() {
        assert!(validate_prompt_edit("evolution_critic_v1", "任何内容").is_err());
    }

    #[test]
    fn freely_editable_only_checks_forbidden_words() {
        // 可自由改层：仍过禁词闸，但不要求锚段
        assert!(validate_prompt_edit("knowledge.chat.draft_chunk", "随便写业务话术").is_ok());
        assert!(validate_prompt_edit("knowledge.chat.draft_chunk", "必要时人工接管").is_err());
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
    normalize_prompt_content, DEFAULT_MODE_GATE_POLICY, DEFAULT_REPLY_REDLINE_ANCHORS,
    DEFAULT_REVIEWER_FEWSHOT, PROMPT_EVOLUTION_FORBIDDEN_KEYS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptEditTier {
    FreelyEditable,
    ConstrainedEditable,
    Forbidden,
}

/// 强约束层 key → 写回后必须逐字保留的全部锚段（业务锚 + 红线锚）。
/// 返回 slice：user.reply.policy 既要保留业务锚 DEFAULT_MODE_GATE_POLICY，
/// **也要保留反接管红线锚 DEFAULT_REPLY_REDLINE_ANCHORS**（核心修正——
/// 旧设计只查业务锚，红线被删能放行）。
fn required_anchors(template_key: &str) -> Vec<&'static str> {
    match template_key {
        "user.reply.policy" => {
            // 业务锚 + 反接管红线锚（红线在正文 :1123/:1146，旧锚闸漏查）
            let mut v = vec![DEFAULT_MODE_GATE_POLICY];
            v.extend_from_slice(DEFAULT_REPLY_REDLINE_ANCHORS);
            v
        }
        "user.review.system" => vec![DEFAULT_REVIEWER_FEWSHOT],
        // user.reply.system / user.reply.task 含红线但暂无独立 DEFAULT_* 锚常量：
        // 仍归强约束层（tier 判定里列出），靠禁词闸兜底；如需更硬可后续为其抽锚。
        _ => Vec::new(),
    }
}

pub(super) fn prompt_edit_tier(template_key: &str) -> PromptEditTier {
    // 禁止改：evolution critic（与 PROMPT_EVOLUTION_FORBIDDEN_KEYS 同源）。
    // 注：reset-system-pack 是 route handler 不是 template_key，靠不接入工具来禁，不在此判。
    if PROMPT_EVOLUTION_FORBIDDEN_KEYS.contains(&template_key) {
        return PromptEditTier::Forbidden;
    }
    // 可改但需强约束：含红线/锚的业务模板（真实 key，已核实存在）
    if !required_anchors(template_key).is_empty()
        || matches!(
            template_key,
            "user.reply.policy" | "user.reply.system" | "user.review.system" | "user.reply.task"
        )
    {
        return PromptEditTier::ConstrainedEditable;
    }
    // 其余业务话术 key，可自由改（仍过禁词闸）
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
    // 闸 2：锚完整性闸——强约束层的全部锚段（业务锚 + 红线锚）必须逐字仍在。
    // CRLF 归一（冲突修正）：锚常量是 Windows 工作树的 r#"..."# 多行串，git autocrlf
    // 跨构建 LF↔CRLF 互转；管理者提交的 new_content 换行风格也不受控。裸 contains 会
    // 因换行字节不同失配、误拒合法编辑 → 两边都过 normalize_prompt_content 再比
    // （复用 PR#42 引入的 prompts.rs:162 同一归一函数）。
    let normalized = normalize_prompt_content(new_content);
    for anchor in required_anchors(template_key) {
        if !normalized.contains(&normalize_prompt_content(anchor)) {
            return Err(format!(
                "提示词 '{template_key}' 的红线/业务锚段缺失或被改，已拒绝（防 replace 静默失配 + 防红线被删）"
            ));
        }
    }
    Ok(())
}
```

在 `src/routes/mod.rs` 加 `mod management_prompt_edit;`。**真正的拦截点是 `update_prompt_template`(prompt_templates.rs:138)**——它当前只调 `validate_prompt_template_input`(查空，:144)、零红线校验。在它 `validate_prompt_template_input(&payload)?` 之后插一行（字面双闸，前置硬门）：

```rust
// prompt_templates.rs update_prompt_template，validate_prompt_template_input 之后
crate::routes::management_prompt_edit::validate_prompt_edit(&payload.prompt_key, &payload.content)
    .map_err(AppError::BadRequest)?;
```

**冲突修正（必做，否则改动活不过一次重启）**：`update_prompt_template` 的 `$set` 当前**不写 `seeded_by`**，被编辑的系统种子行 `seeded_by` 仍是 `"system"`。而 main 刚合并的 PR#42 启动对齐 `align_prompt_specs`(prompts.rs:175) 会对 `seeded_by="system"` 且内容≠DEFAULT 的行**归档 + 重种回 DEFAULT**（`is_refreshable_prompt_seeded_by` 只认 "system"，prompts.rs:152；"manual" 返回 false，测试 :2366 坐实）。所以经双闸通过、确实要落库的编辑，`$set` **必须把 `seeded_by` 置成 `"manual"`**，让 align 跳过它（:237-238 `continue`）：

```rust
// update_prompt_template 的 $set 增加（防 PR#42 启动对齐覆盖管理者改动）：
"seeded_by": "manual",
```

> 注：`"manual"` 入 align 白名单之外（永不被归档重种）。这是管理者"自然语言改 prompt"能持久化、活过重启的关键。reset-system-pack（显式销毁性重种）不受影响、也不接入工具。

这样无论走管理 agent 工具、还是管理员直接调 REST PUT /prompt-templates/:id，双闸都拦得住（单点拦截，不靠每个调用方自觉）。execute_management_tool 的 edit_prompt_template 工具分支复用 update_prompt_template 即自动过闸 + 自动置 manual。**LLM 第三闸（语义审查）在 Task 6.6 接在同一拦截点的双闸之后**——本任务先把快、确定、无依赖的字面双闸落地。

> **本任务额外回归测试**（守 PR#42 兼容）：`constrained_edit_marks_seeded_by_manual`——断言经双闸的 update 落库 `seeded_by="manual"`（防 align 覆盖）；`anchor_gate_normalizes_crlf`——断言锚常量用 CRLF 提交仍能通过（防换行误拒）。

- [ ] **Step 4: 运行测试确认通过 + 全量 lib 不回归**

Run: `cargo test --lib management_prompt_edit 2>&1 | tail -20`
Expected: PASS（7 个 tier/双闸用例全绿，含 `dual_gate_rejects_redline_anchor_drift` 红线锚漏洞回归锁）
Run: `cargo test --lib reply_redline_anchors_present_in_pack 2>&1 | tail -10`
Expected: PASS（红线锚漂移护栏）
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: 提交**

```bash
git add src/prompts.rs src/routes/management_prompt_edit.rs src/routes/prompt_templates.rs src/routes/mod.rs
git commit -m "feat(mgmt-agent): 提示词三层分级+字面双闸(抽反接管红线锚扩锚闸,拦截点update_prompt_template,fail-closed)"
```

---

## Task 6.6: 提示词编辑第三闸 — LLM 红线语义审查 + 三态降级人确认

> 堵字面双闸挡不住的**插入型语义绕过**（保留锚段、无字面禁词，但插入"转给后台老师跟进"这类变相接管）。spec §4.4 第三闸。用户决策：加入第一期；LLM 不可用时降级人确认（非 fail-closed 死路、非 fail-open 放水）；两条编辑路径都有人确认兜底、体验一致。

**Files:**
- Modify: `src/routes/management_prompt_edit.rs`（加三态 `PromptEditVerdict` + async `review_prompt_edit`，复用 `generate_agent_json` judge）
- Modify: `src/prompts.rs`（加第三闸 judge 的 PromptSpec `management.prompt_redline_review.system`，bump PROMPT_PACK_VERSION）
- Modify: `src/routes/prompt_templates.rs:138 update_prompt_template`（双闸后接第三闸，按三态分流；加 `force: Option<bool>` 入参）
- Modify: `src/routes/management.rs`（execute_management_tool 的 edit_prompt_template 工具分支：NeedsHumanConfirm → 走 command pending_confirmation）
- Test: `src/routes/management_prompt_edit.rs` 内 tests（三态判定纯函数部分 + diff 提取）

**Interfaces:**
- Consumes: `crate::agent::generate_agent_json`（项目唯一 LLM JSON 入口，带重试/退避/RunBudget）；字面双闸 `validate_prompt_edit`（Task 6.5）。
- Produces: `enum PromptEditVerdict { Pass, Reject(String), NeedsHumanConfirm { diff: String, reason: String } }`；`fn extract_diff(old: &str, new: &str) -> String`（提取新增/改动增量，纯函数）；`async fn review_prompt_edit(state, template_key, old, new) -> PromptEditVerdict`。

- [ ] **Step 1: 写失败测试（diff 提取 + 三态壳，纯函数部分）**

```rust
#[test]
fn extract_diff_isolates_added_lines() {
    let old = "第一行\n第二行";
    let new = "第一行\n第二行\n遇到难题转给后台老师跟进";
    let d = extract_diff(old, new);
    assert!(d.contains("转给后台老师跟进"));
    assert!(!d.contains("第一行")); // 只出增量，不重复未改部分
}

#[test]
fn verdict_variants_shape() {
    // 三态可构造（编译期锁形状；LLM 行为留真模型 nightly 套件）
    let _p = PromptEditVerdict::Pass;
    let _r = PromptEditVerdict::Reject("命中变相接管".into());
    let _h = PromptEditVerdict::NeedsHumanConfirm { diff: "x".into(), reason: "LLM 审查不可用".into() };
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib management_prompt_edit 2>&1 | tail -15`
Expected: FAIL（`extract_diff` / `PromptEditVerdict` 未定义）

- [ ] **Step 3: 实现三态 + diff 提取 + LLM 第三闸**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PromptEditVerdict {
    Pass,
    Reject(String),
    NeedsHumanConfirm { diff: String, reason: String },
}

/// 提取新增/改动增量（行级朴素 diff——只要 new 中不在 old 的行）。
/// 审增量比审整篇好判、省 token（spec §4.4）。
pub(super) fn extract_diff(old: &str, new: &str) -> String {
    let old_lines: std::collections::HashSet<&str> = old.lines().collect();
    new.lines()
        .filter(|l| !old_lines.contains(l) && !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 第三闸：LLM 语义审查 diff 增量。先过字面双闸（调用方保证），本函数只做语义层。
/// 三态：Pass / Reject(理由) / NeedsHumanConfirm（LLM 重试退避后仍不可用 → 降级人确认）。
pub(super) async fn review_prompt_edit(
    state: &AppState,
    workspace_id: &str,
    template_key: &str,
    old: &str,
    new: &str,
) -> PromptEditVerdict {
    let diff = extract_diff(old, new);
    if diff.trim().is_empty() {
        return PromptEditVerdict::Pass; // 无增量（纯删减已被锚闸挡过）
    }
    // generate_agent_json 自带重试/退避（项目主链路 primary_max_retries）。
    // judge 只判 diff 增量是否变相引入真人接管/削弱 grounding/绕过 verify。
    let judge = generate_agent_json(
        state, workspace_id,
        "management.prompt_redline_review.system",
        &json!({ "template_key": template_key, "diff": diff }),
        /* schema: { "violation": bool, "reason": string } */
    ).await;
    match judge {
        Ok(v) if v.get("violation").and_then(Value::as_bool) == Some(true) => {
            let reason = v.get("reason").and_then(Value::as_str)
                .unwrap_or("LLM 判定 diff 变相引入真人接管/削弱红线").to_string();
            PromptEditVerdict::Reject(reason)
        }
        Ok(_) => PromptEditVerdict::Pass,
        // 重试退避后仍失败（503/空/不可解析）→ 降级人确认，不 fail-closed 死路也不 fail-open 放水
        Err(_) => PromptEditVerdict::NeedsHumanConfirm {
            diff,
            reason: "红线语义审查服务暂不可用，请逐字核对本次改动有无变相引入真人接管再确认".to_string(),
        },
    }
}
```

> 实现注意：`generate_agent_json` 的真实签名/schema 传法以 agent/mod.rs 为准（实现时对齐）；judge prompt 措辞要包含 :1123/:1146 红线的语义要点（变相承认真人后台、承诺转交、削弱 grounding、绕过 verify），但**判定靠 LLM 语义不靠词表**（守 agent-first）。

- [ ] **Step 4: 拦截点按三态分流 + force 覆盖 + 两路径消费**

`update_prompt_template`(prompt_templates.rs:138) 双闸之后：

```rust
// 双闸已过（Task 6.5）。第三闸语义审查：
let force = payload.force.unwrap_or(false);
let old_content = /* 库内现有 content（find_one 取，update 前读一次）*/;
if !force {
    match management_prompt_edit::review_prompt_edit(
        &state, &admin.current_workspace, &payload.prompt_key, &old_content, &payload.content
    ).await {
        PromptEditVerdict::Pass => {}
        PromptEditVerdict::Reject(reason) =>
            return Err(AppError::BadRequest(format!("红线语义审查拒绝：{reason}（确认无误可带 force 覆盖）"))),
        PromptEditVerdict::NeedsHumanConfirm { diff, reason } =>
            // 路径B：返回需二次确认的响应（非错误），前端弹框显示 diff+reason，勾选后带 force=true 重提
            return Ok(Json(json!({
                "status": "needs_human_confirm", "reason": reason, "diff": diff
            }))),
    }
}
// force=true 或 Pass → 继续原 update_one 写入
```

`PromptTemplateRequest` 加 `#[serde(default)] force: Option<bool>`。路径A（execute_management_tool 的 edit_prompt_template 分支）：拿到 `needs_human_confirm` 时，把它转成 command 的 `pending_confirmation`（plan 暂存改动），走 Task 5 confirm 循环——管理者在对话里确认后带 force 重放。**两路径同一判定函数、两个消费端，不 drift**。

- [ ] **Step 5: 运行测试 + cargo check + 全量 lib + no-takeover lint**

Run: `cargo test --lib management_prompt_edit 2>&1 | tail -15`
Expected: PASS
Run: `cargo check 2>&1 | tail -5`
Expected: Finished
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 无禁词（judge prompt 用 AI 内部口径，prompts.rs 不在扫描目录但仍守）

- [ ] **Step 6: 提交**

```bash
git add src/prompts.rs src/routes/management_prompt_edit.rs src/routes/prompt_templates.rs src/routes/management.rs src/routes/mod.rs
git commit -m "feat(mgmt-agent): 提示词第三闸LLM红线语义审查(审diff增量,三态降级人确认,两路径一致)"
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

## Task 8: 前端 prompt 编辑器（路径B）二次确认弹框

> 路径B = 管理员在 prompt 编辑器直接调 `PUT /prompt-templates/:id`。Task 6.6 后端在 LLM 第三闸不可用时返回 `{status:"needs_human_confirm", reason, diff}`（200，非错误）。前端要识别这个响应、弹框显示 diff + 风险、勾选后带 `force=true` 重提——与路径A 体验一致（用户决策）。

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（prompt 编辑器保存处，真实频道见 system-strategy）
- Test: `frontend/src/__tests__/`（对应 system-strategy 测试，按现有目录结构放）

**Interfaces:**
- Consumes: `PUT /prompt-templates/:id` 的 `needs_human_confirm` 响应（含 reason/diff）+ `Reject` 的 BadRequest。
- Produces: 保存时若收 needs_human_confirm → 弹确认框（显示 reason + diff 增量）→ 管理员勾"我已逐字核对有无变相引入真人接管"→ 带 `force:true` 重提。

- [ ] **Step 1: 读现有 prompt 编辑器保存逻辑**

Run: `grep -rn "prompt-templates" frontend/src/features/system-strategy/`
定位保存 prompt 的 fetch 调用 + 现有错误处理，遵守 docs/frontend-design-system.md（不自由发挥，复用现有弹框/确认原语）。

- [ ] **Step 2: 写失败测试**

mock `PUT /prompt-templates/:id` 返回 `{status:"needs_human_confirm", reason:"审查服务不可用", diff:"+转给后台老师"}` → 断言渲染确认弹框含 diff；mock 勾选后重提断言请求体带 `force:true`。mock 返回 BadRequest「红线语义审查拒绝」→ 断言显示拒绝理由 + force 覆盖入口。

- [ ] **Step 3: 运行确认失败**

Run: `cd frontend && npx vitest run --no-file-parallelism <对应测试文件> 2>&1 | tail -15`
Expected: FAIL

- [ ] **Step 4: 实现保存流的三态处理**

保存 prompt 的 handler：解析响应——`ok:true` 正常；`status==needs_human_confirm` 弹框（显 reason + diff，勾选确认后带 `force:true` 重发同一 PUT）；BadRequest「红线语义审查拒绝」显拒绝理由 + 「确认无误，强制保存」入口（同样带 force）。弹框文案守 no-human-takeover 禁词（用"变相引入真人接管"描述风险是在 frontend/src 扫描目录内——注意措辞：用"红线/边界"等中性词，避开裸"人工接管"，参照现有红线文案）。

> ⚠️ 禁词 lint：frontend/src 在 `check-no-human-takeover.sh` 扫描范围。弹框提示文案不能出现裸"人工接管/接管/人工"，用"边界红线 / 变相引入真人 / 自治红线"等表述。实现后跑 lint 确认。

- [ ] **Step 5: 运行测试 + tsc + no-takeover lint**

Run: `cd frontend && npx vitest run --no-file-parallelism <对应测试文件> 2>&1 | tail -15`
Expected: PASS
Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -10`
Expected: 无类型错误
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 无禁词

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/system-strategy/ frontend/src/__tests__/
git commit -m "feat(mgmt-agent): prompt编辑器(路径B)二次确认弹框(needs_human_confirm+force覆盖,与路径A一致)"
```

---

## Task 9: 部署到服务器 + 真实大模型全量冒烟（管理 agent 链路，不真发微信）

> 用户决策（2026-06-26）：完整计划落地后部署到生产服务器，用真实大模型跑全量冒烟。**冒烟范围 = 管理 agent 链路（不真发微信消息）**——验证本次做厚的核心链能在真环境跑通，需真 LLM（build_management_plan + 第三闸都真调大模型），但不需真 MCP 发送 / 真微信号。

**前置：需用户提供的信息（落笔时为占位，开工前由用户填实，绝不进 git）**

| 信息 | 用途 | 记忆里的旧值（需用户确认是否仍有效） |
| --- | --- | --- |
| 服务器 IP / SSH 端口 / user | 部署目标 | `117.72.54.28` : 22 : root（[[deploy_server_117]]，15 天前） |
| root 密码 | `_remote_run.py` 的 `DEPLOY_PASS` env | **记忆未存**，会话内 `! export DEPLOY_PASS=...` 注入，不进记忆/代码 |
| 应用端口 | APP_PORT | 3003（8080 也空闲） |
| `OPENAI_BASE_URL` | 冒烟用真 LLM 端点 | **待用户给**（生产记忆默认 deepseek，但值未存） |
| `OPENAI_API_KEY` | 真 LLM key | **待用户给**（永不进 git/记忆/日志） |
| `OPENAI_MODEL` | 真模型名 | **待用户给**（生产记忆 deepseek-v4-flash，可能过期） |
| `MCP_API_KEY` | 启动必填项（不真发也要能启动） | **待用户给**（启动校验需要，可填占位/只读 key） |

> 启动必填仅 `MCP_API_KEY` + `OPENAI_API_KEY`（CLAUDE.md）。冒烟不真发微信，MCP key 只要能让进程启动即可；真 LLM 必须真 key（管理对话/第三闸要真调）。

**Files:**
- 复用 `scripts/_remote_run.py`（paramiko 远程驱动，读 env `DEPLOY_PASS`/`DEPLOY_PORT`）；远程脚本必须 ASCII-only（中文经 heredoc→Python stdin 会 UnicodeEncodeError）。
- 不新增代码文件；本任务是部署 + 冒烟脚本 + 人工核对。

- [ ] **Step 1: 同步代码到服务器 + 配 .cargo 镜像 + 构建**

把本分支部署到 `/opt/wechatagent`（git pull 或 clone）。**首构建前必配国内 sparse 镜像**（`/opt/wechatagent/.cargo/config.toml` USTC，否则 crates.io 卡死——[[project_deploy_117_first_deploy]] 坑1）。`source ~/.cargo/env` 后 `cargo build --release`。前端 `npm install && npm run build` 出 dist。

- [ ] **Step 2: 写 .env（chmod 600，绝不进 git）**

`/opt/wechatagent/.env`：`MCP_API_KEY` / `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_MODEL`（用户提供值）+ `APP_PORT=3003` + `APP_BASE_URL`。独立 MongoDB 库名 `wechatagent`。`chmod 600 .env`。

- [ ] **Step 3: 启动 + 健康检查**

systemd `wechatagent.service`（Restart=always，RUST_MIN_STACK=8388608，仿现有 unit）。`systemctl restart wechatagent` 后：
- `curl -s localhost:3003/` 返回 200（前端）
- 现有健康端点返回正常
- `journalctl -u wechatagent --since "2 min ago"` 无 panic（特别核启动期 `run_active_domain_state_machine_sanity_check`——[[project_deploy_117_first_deploy]] 坑2，空状态机 active 会 bail）

- [ ] **Step 4: 真 LLM 冒烟——管理对话出 plan**

登录管理后台拿 session cookie，调 `POST /management-agent/sessions` + `/sessions/:id/messages` 发一句自然语言指令（如"查一下最近的运营 run"）：
- 断言 build_management_plan **真调通 LLM**（journalctl 见 llm_call_logs status=success，非 json_error/failed）
- 返回的 plan 含合理 tool_calls（readonly 的 query_runs）
- readonly 工具**真执行**返回真实数据

- [ ] **Step 5: 真 LLM 冒烟——提议→确认→执行循环**

发一句触发需确认的指令：
- 用 verify 类指令测**恒确认**（不随第一期 dangerous 开关放行）→ 断言返回 `pending_confirmation`
- 调 `POST /management-agent/commands/:id/confirm` → 真执行 + outcome 核实（成功/失败/executed_unverified 如实汇报，非回放 plan.summary）

- [ ] **Step 6: 真 LLM 冒烟——提示词三闸真拦（核心红线验证）**

本次做厚最关键的红线验证，必须真环境真大模型跑：
- **字面双闸**：调 `PUT /prompt-templates/:id` 改 `user.reply.policy`，故意删反接管红线锚段 → 断言被拒（400，锚完整性闸）；故意写"人工接管" → 断言被拒（禁词闸）。
- **LLM 第三闸**：保留全部锚段、不用字面禁词、插入"遇到难题转给后台老师跟进"（变相接管）→ 断言**真大模型判 violation → Reject**（journalctl 见第三闸 judge 真调通）。
- **降级人确认**：临时把 OPENAI_BASE_URL 改不可达地址重启（模拟 LLM 挂），重做上一步 → 断言返回 `needs_human_confirm`（非 fail-closed 报错、非 fail-open 放行）；恢复端点。
- **正常编辑放行**：保留全部锚 + 无禁词 + 合理业务措辞 → 断言放行写入。

- [ ] **Step 7: 冒烟结论 + 回收**

汇总每步真实结果，journalctl 抓 llm_call_logs 确认真调而非 skip/mock。临时 .env 改动回收。**不动 agime-* 服务/端口/库**（[[deploy_server_117]]）。冒烟若发现 bug，回对应 Task 修复后重新部署。

---

**Spec coverage**：
- §2 闭合循环 → Task 5（confirm/reject 端点）✓
- §3 执行结果核实 → Task 2（assert_tool_outcome）+ Task 3（汇报基于真实结果）✓
- §4.1 工具集扩（6 类全量接入）→ Task 6 ✓
- §4.2 风险档代码裁定（四档含 irreversible 恒拦）→ Task 1 ✓
- §4.3 verify 红线（恒确认 + actor 标人）→ Task 1（plan_requires_confirmation + tool_always_requires_confirmation）+ Task 6 注 ✓
- §4.4 提示词三层分级 + 三闸校验（字面双闸 + LLM 语义第三闸）→ Task 6.5（双闸）+ Task 6.6（第三闸三态降级）✓
- §1.2 第一期放权 → Task 1（plan_requires_confirmation dangerous 开关默认关、irreversible/verify 恒拦）+ Task 6（全接）✓
- §5.1 status 闭集 → Task 4 ✓
- §5.3 前端 → Task 7（command-center 路径A 确认 UI）+ Task 8（prompt 编辑器路径B 二次确认弹框）✓
- 红线（AI 永不自动 verify）→ Task 1（verify 恒确认）+ Task 6 注 + Task 6.5（字面双闸 fail-closed、新抽红线锚堵反接管漏洞）+ Task 6.6（LLM 语义第三闸堵插入型绕过）✓

**4 路 opus 核实修正记录（2026-06-26，落笔前亲核 origin/main）**：
- Task 1：函数名 `is_read_only_tool`→`is_read_tool`（笔误）；新增 `tool_always_requires_confirmation`（verify 类恒确认，守 AI 永不自动 verify）。
- Task 6：build_management_plan 走 PROMPT_PACK_VERSION 版本化 prompt（非"内联字面量"，原判断错）；标出 3 个需小重构 handler（cancel_outbox/approve_taxonomy_candidate/approve_relationship_suggestion 返回 Response 非 Json<Value>，approve_taxonomy_candidate 缺 Extension<Admin>）；import-apply-pdf 用 Multipart 包不动需走 import_pdf_bytes；verify 红线 actor 处理。
- Task 6.5（改动最大）：原草案 key 命名半数错已修（agent_soul/operation_playbook 不是 template_key→soul/playbook 在独立集合；user.review.policy 不存在；reset_system_pack 是 handler 非 key）；**核心红线漏洞已修**——原锚闸只查 `DEFAULT_MODE_GATE_POLICY`，而该锚故意不含反接管红线（测试 prompts.rs:2506-2511 坐实），真红线在 :1123/:1146 旧闸漏查；现 Step 0 先抽 `DEFAULT_REPLY_REDLINE_ANCHORS` 红线锚 + 护栏测试，`required_anchors` 返回多锚（业务锚+红线锚），新增 `dual_gate_rejects_redline_anchor_drift` 回归锁；拦截点明确为 `update_prompt_template`(prompt_templates.rs:138) 单点拦截。插入型语义绕过由 Task 6.6 LLM 第三闸堵（非留后续）。

**Placeholder scan**：无 TBD/TODO；execute_plan_tool_calls 抽取在 Task 5 明确；新工具分发在 Task 6 指明"复用已有写入函数"（具体函数实现时 grep 定位，因 routes 模块多）；Task 6.5 Step 0 红线锚常量值标注"实现时从 prompts.rs:1123/1146 逐字复制"（已与合并后正文核对一致）。

**main 合并冲突修正记录（2026-06-26 合并 origin/main 25 提交后，opus 核实 + 亲核）**：合并了 PR#41（evolution/知识 workspace 隔离）+ PR#42（prompt-pack 启动对齐）。两个会让"自然语言改 prompt"白做的真冲突已修进 Task 6.5：
- **冲突1 CRLF 误判**：锚闸 `contains` 前对 new_content 与每条锚常量都过 `normalize_prompt_content`(prompts.rs:162，PR#42 引入)——否则 Windows autocrlf 下换行字节差异误拒合法编辑。
- **冲突2 启动对齐覆盖（最严重）**：PR#42 `align_prompt_specs`(prompts.rs:175) 每次重启把 `seeded_by="system"` 且内容≠DEFAULT 的行归档重种回 DEFAULT。`update_prompt_template` 的 `$set` 必须加 `seeded_by="manual"`（align 白名单只认 system，manual 跳过——测试 prompts.rs:2366 坐实），否则管理者改动活不过一次重启。
- **行号对齐**：prompts.rs :175 后引用 +123（红线正文 :1123/:1146，FORBIDDEN_KEYS :2261，护栏测试 :2456-2520）；update_prompt_template :138；management.rs/lint.rs/mod.rs 未变。

**Type consistency**：ToolRisk 四档（Task 1，含 Irreversible）/ ToolOutcome（Task 2）/ build_execution_summary（Task 3）/ TOOL_CALL_STATUSES（Task 4）/ build_confirm_filter + execute_plan_tool_calls（Task 5）/ PromptEditTier + validate_prompt_edit（Task 6.5）签名一致；read_only 字段名保留兼容既有调用点；plan_requires_confirmation 的 irreversible 恒拦逻辑与 Task 1 测试一致。

**已知留实现期定位**：Task 6 各 dangerous/irreversible 工具复用的具体 REST 写入函数名（需 grep 对应 routes 模块）；Task 6 prompt 改动是否 bump 版本（依 prompt_versions 机制）；Task 6.5 reset_system_pack 的真实 template_key 名（实现时 grep reset-system-pack 路由确认，本计划按 "reset_system_pack" 占位）。这些是定位类，非设计缺口。
