# 批C家族① 修复实施计划：campaign dispatch 补偿回滚 + status 前置门

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 让 `dispatch_campaign` 的每个 contact "占去重位→建 task→回填 taskId"三步在中途 Err 时补偿回滚（全有或全无），并加 status 前置门防 completed 重推、允许 dispatching 重入恢复（KC-01/02/03）。

**Architecture:** 全在 `src/routes/campaigns.rs`、零 schema 改动。组件1：`dispatch_allowed_from_status` 纯函数（status 前置门）；组件2：循环体把裸 `?` 换成"失败即补偿删除已建 send/task 再 return Err"；组件3：重入恢复由 1+2 自然成立（dispatching 放行 + 去重跳过已完成 send + 补偿保证无孤儿）。send 先占位的写序不动（send 是 unique 去重闸）。

**Tech Stack:** Rust 2021 / Axum / MongoDB。纯函数 `cargo test --lib`（无需 Docker）；补偿回滚 + status 门集成测 `cargo test --test campaign_dispatch_integration -- --ignored`（需 Docker，CI 跑）。

## Global Constraints
- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——每个改码 Task 的 Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `src/routes/campaigns.rs`（+status 门纯函数+单测，dispatch_campaign 补偿回滚+status 校验）+ `tests/campaign_dispatch_integration.rs`（+故障注入回滚测+status 门测）。**不改** schema（AgentTask/CampaignSend 不加字段）、report join、`resolve_segment_contacts`、send/task 写序、其它 handler。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。新增测试只增不减。
- **no-human-takeover lint**：src/ + tests 外的新增行不得含 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`（tests 目录被 lint 排除，但仍用「派发/触达/去重/补偿/回滚」措辞）。
- **补偿删除用 `let _`**（best-effort）：补偿本身再失败属极窄双重故障，超出 finding 单次瞬时错范围。
- **设计文档**：`docs/superpowers/specs/2026-07-12-kc-family1-dispatch-atomicity-design.md`。
- **台账**：`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KC-01/02/03。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）
- `src/routes/campaigns.rs`：`dispatch_campaign` 定义 `:289`；置 `dispatching` update 在 `:317-325`（前无 status 校验）；循环体 `:328-371`；三步——`campaign_sends().insert_one(&send)` `:341`、`tasks().insert_one(&task).await?` `:351`、回填 `campaign_sends().update_one(...).await?` `:357-365`；`Err(e) if is_duplicate_key(&e) => {}` `:369`、`Err(e)=>return Err(e.into())` `:370`；completed update `:373-382`。
- `campaigns.rs` 已有 `#[cfg(test)] mod tests`（`:660`，`use super::*;` `:662`）。
- `is_duplicate_key`（campaigns.rs:170，本文件私有）；`assert_campaign_status_valid`（models.rs:646，校验 `ALLOWED_CAMPAIGN_STATUS` 闭集）；`build_campaign_follow_up_task`（campaigns.rs:127）。
- 集合真名：`state.db.tasks()` = `agent_tasks`（db/mod.rs:83）；`state.db.campaign_sends()` = `campaign_sends`（db/mod.rs:392）。`state.db.raw()` = `&mongodb::Database`（KB-08/c2 已用）。
- `CampaignSend`：`#[serde(rename_all="camelCase")]`，字段 `campaign_id`→`campaignId`、`contact_wxid`→`contactWxid`、`task_id`→`taskId`、`status`、`created_at`→`createdAt`（models.rs）。
- 集成测基建：`tests/campaign_dispatch_integration.rs` 直调 `dispatch_campaign(State, Extension, Path)` handler，`TestApp::start()`，helper `make_contact(ws,acc,wxid)` / `make_campaign(ws,acc)`（status="draft"）；validator 注入范式见 `tests/c2_operation_state_derivation_e2e.rs:720-735`（`app.state.db.raw().create_collection(name,None)` 忽略错 + `run_command(doc!{"collMod":name,"validator":{...},"validationAction":"error"},None)`）。
- `Campaign` 有 `pub status: String`（campaigns.rs 用 `campaign.status`）；campaign status 取值 draft/previewed/dispatching/completed。

---

## Task 1: dispatch_allowed_from_status 纯函数 + status 前置门接线（TDD）

**Files:**
- Modify: `src/routes/campaigns.rs`（新增 pub(super) fn + 单测；dispatch_campaign 加 status 校验）

**Interfaces:**
- Produces: `pub(super) fn dispatch_allowed_from_status(status: &str) -> bool`。

- [ ] **Step 1: 先读懂（红线）**

Read `src/routes/campaigns.rs:289-326`（dispatch_campaign 开头到置 dispatching）+ `:660-680`（tests mod 头 + 现有单测范式）。Grep 亲验 `campaign.status` 字段可读（`grep -n "pub status" src/models.rs` 附近 Campaign 定义）、`AppError::BadRequest` 用法。确认置 dispatching 的 update 当前行号。**说不清就继续读。**

- [ ] **Step 2: 写失败单测**

在 `campaigns.rs` 的 `#[cfg(test)] mod tests`（:660）内追加：

```rust
    #[test]
    fn dispatch_allowed_only_from_draft_previewed_dispatching() {
        // KC-02：completed 活动不可再派发（防重复推送）；dispatching 允许重入恢复；未知态 fail-safe 拒。
        assert!(dispatch_allowed_from_status("draft"));
        assert!(dispatch_allowed_from_status("previewed"));
        assert!(dispatch_allowed_from_status("dispatching"), "dispatching 须允许重入恢复");
        assert!(!dispatch_allowed_from_status("completed"), "completed 不可重推");
        assert!(!dispatch_allowed_from_status("canceled"));
        assert!(!dispatch_allowed_from_status("赫赫"), "未知态 fail-safe 拒");
    }
```

- [ ] **Step 3: 跑测试确认失败（编译错误）**

Run: `cargo test --lib dispatch_allowed 2>&1 | tail -15`
Expected: `cannot find function dispatch_allowed_from_status`（TDD red）。

- [ ] **Step 4: 实现纯函数**

在 `dispatch_campaign` 上方（或 `is_duplicate_key` 附近）加：

```rust
/// KC-02：仅这些 status 允许 dispatch。dispatching = 允许重入恢复（配合补偿回滚，
/// 已完成的 send 撞去重跳过、失败/剩余 contact 重建）；completed = 拒绝（防重复推送）；
/// 未知态 = 拒绝（fail-safe）。
pub(super) fn dispatch_allowed_from_status(status: &str) -> bool {
    matches!(status, "draft" | "previewed" | "dispatching")
}
```

- [ ] **Step 5: 接线到 dispatch_campaign**

在 `dispatch_campaign` 里、置 `dispatching` 的 update（约 :317）**之前**（圈人 hits 校验之后或 campaign 取出之后皆可，但须在改 status 前）插入：

```rust
    if !dispatch_allowed_from_status(&campaign.status) {
        return Err(AppError::BadRequest(format!(
            "当前活动状态 {} 不可派发（仅 draft/previewed/dispatching 可派发；completed 需另建活动）",
            campaign.status
        )));
    }
```

- [ ] **Step 6: 跑测试确认通过 + baseline**

Run: `cargo test --lib dispatch_allowed 2>&1 | tail -8` → PASS。
Run: `cargo test --lib 2>&1 | tail -8` → `ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 7: Commit**

```bash
git add src/routes/campaigns.rs
git commit -m "fix(campaign): dispatch status 前置门,拒 completed 重推+允许 dispatching 重入(KC-02)

抽 dispatch_allowed_from_status 纯函数(draft/previewed/dispatching 可派,completed/未知态拒);
dispatch_campaign 置 dispatching 前校验。防已完成活动被反复 dispatch 重推。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: 循环内补偿回滚（KC-01/03 all-or-nothing）

**Files:**
- Modify: `src/routes/campaigns.rs`（dispatch_campaign 循环体 :341-371）

**Interfaces:**
- Consumes: 无新增。
- Produces: 无（行为层：中途 Err 时不留孤儿 send/task）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/routes/campaigns.rs:328-386`（整个循环体 + 循环后 completed update）。亲验：`insert_one(&send)` 返回 `send_res`（`.inserted_id.as_object_id()` 取 send_id）、`tasks().insert_one(&task).await?` 的 `?`、回填 `update_one(...).await?` 的 `?`、`Err(e) if is_duplicate_key => {}` 与 `Err(e)=>return Err`。确认 `campaign_sends().delete_one` / `tasks().delete_one` 的 mongodb 2.8 签名（`delete_one(doc!{...}, None)`）。**说不清就继续读，不动手。**

- [ ] **Step 2: 改写 Ok(send_res) 分支为补偿回滚**

把 `:341` 起 `match ... { Ok(send_res) => { ... }` 分支体（含 :351 建 task、:353-366 回填）整体替换为（`Err(e) if is_duplicate_key(&e) => {}` 与 `Err(e)=>return Err(e.into())` 两分支保持不动）：

```rust
            Ok(send_res) => {
                let send_id = send_res.inserted_id.as_object_id();
                let task = build_campaign_follow_up_task(
                    &campaign.workspace_id,
                    &campaign.account_id,
                    &c.wxid,
                    &campaign.intent_text,
                    now,
                );
                assert_agent_task_status_valid(&task.status);
                // KC-01：建 task 失败 → 补偿删掉刚占位的 send,避免留下 task_id=None 的孤儿 send
                // (重入撞去重跳过→客户永久漏推)。补偿删除 best-effort(let _)。
                let task_res = match state.db.tasks().insert_one(&task, None).await {
                    Ok(r) => r,
                    Err(e) => {
                        if let Some(sid) = send_id {
                            let _ = state
                                .db
                                .campaign_sends()
                                .delete_one(doc! { "_id": sid }, None)
                                .await;
                        }
                        return Err(e.into());
                    }
                };
                // KC-03：回填 taskId 失败 → 补偿删 task + send,保持 all-or-nothing
                // (否则 task 会真发但 report 显 pending 成效虚低)。
                if let (Some(sid), Some(tid)) =
                    (send_id, task_res.inserted_id.as_object_id())
                {
                    if let Err(e) = state
                        .db
                        .campaign_sends()
                        .update_one(
                            doc! { "_id": sid },
                            doc! { "$set": { "taskId": tid } },
                            None,
                        )
                        .await
                    {
                        let _ = state.db.tasks().delete_one(doc! { "_id": tid }, None).await;
                        let _ = state
                            .db
                            .campaign_sends()
                            .delete_one(doc! { "_id": sid }, None)
                            .await;
                        return Err(e.into());
                    }
                }
                dispatched += 1;
            }
```

- [ ] **Step 3: cargo check 确认编译**

Run: `cargo check --lib 2>&1 | tail -20`
Expected: 0 error。（若 `doc!` / `delete_one` 需 import，按报错补 use；本文件已用 `doc!` 与 campaign_sends，通常无需新 import。）

- [ ] **Step 4: baseline 不回退**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 5: Commit**

```bash
git add src/routes/campaigns.rs
git commit -m "fix(campaign): dispatch 循环补偿回滚,消除孤儿 send/task(KC-01/03)

三步非原子写(占去重位→建task→回填taskId)任一失败即补偿删除已建 state:
task insert 失败→删 send;回填失败→删 task+send。每 contact all-or-nothing,
重入干净(dispatching 放行+去重跳过已完成+无孤儿)。补偿删除 best-effort。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 集成测（补偿回滚故障注入 + status 门）+ baseline + PR

**Files:**
- Modify: `tests/campaign_dispatch_integration.rs`（+2 个 `#[ignore]` 用例）

**Interfaces:**
- Consumes: `dispatch_campaign` handler、`TestApp`、`make_contact`/`make_campaign`、`app.state.db.raw()`。

- [ ] **Step 1: 先读懂（红线）**

Read `tests/campaign_dispatch_integration.rs` 全文（helper + 3 现有用例范式）+ `tests/c2_operation_state_derivation_e2e.rs:715-740`（validator 注入范式）。确认 `make_campaign` 返回 status="draft" 的 Campaign、`app.state.db.campaign_sends()`/`campaigns()` accessor。**说不清就继续读。**

- [ ] **Step 2: 追加两个集成测用例**

在 `tests/campaign_dispatch_integration.rs` 末尾追加：

```rust
/// KC-01/03 补偿回滚：dispatch 循环中 agent_tasks insert 被 validator 拒 → task insert 失败
/// → 补偿删掉刚占位的 send → campaign_sends 无孤儿（该 contact 无残留 send 记录）、dispatch 返 Err。
#[tokio::test]
#[ignore]
async fn dispatch_task_insert_failure_rolls_back_send() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_rollback"), None).await.expect("seed contact");

    // 装 validator：让 agent_tasks 的 insert 确定性失败（拒绝所有 kind=follow_up 的插入）。
    let _ = app.state.db.raw().create_collection("agent_tasks", None).await;
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_tasks",
                "validator": { "kind": { "$ne": "follow_up" } },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install agent_tasks validator");

    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "task insert 失败应中断并返 Err");

    // 核心：补偿回滚后无孤儿 send（该 campaign 下 0 条 campaign_sends）。
    let orphan_sends = app
        .state
        .db
        .campaign_sends()
        .count_documents(doc! { "campaignId": cid }, None)
        .await
        .expect("count sends");
    assert_eq!(orphan_sends, 0, "task insert 失败须补偿删除 send,不留孤儿(KC-01)");
}

/// KC-02 status 门：completed 活动 dispatch → BadRequest（防重复推送）。
#[tokio::test]
#[ignore]
async fn dispatch_completed_campaign_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let mut campaign = make_campaign(&ws, &acc);
    campaign.status = "completed".to_string();
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_done"), None).await.expect("seed contact");

    let result = dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(result.is_err(), "completed 活动不可再 dispatch(防重推)");
}
```

- [ ] **Step 3: 编译确认（本地无 Docker，只 --no-run）**

Run: `cargo test --test campaign_dispatch_integration --no-run 2>&1 | tail -15`
Expected: 编译成功 0 error。**不本地跑**（`#[ignore]`+需 Docker），红→绿由 CI integration job 验证。若 `count_documents`/`raw`/`run_command` 签名报错，参照 c2_e2e 现有用法修正。

- [ ] **Step 4: no-human-takeover lint 自检**

Run: `git diff origin/main -- src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`（tests 目录被 lint 排除，仅查 src/）。

- [ ] **Step 5: 全量 baseline**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 6: push + 开 PR**

```bash
git add tests/campaign_dispatch_integration.rs
git commit -m "test(campaign): dispatch 补偿回滚故障注入 + status 门集成测(批C家族①)

validator 拒 agent_tasks insert→断言 campaign_sends 无孤儿(KC-01/03);
completed 活动 dispatch→BadRequest(KC-02)。#[ignore] 需 Docker,CI integration 跑。

Co-Authored-By: Claude <noreply@anthropic.com>"
git push -u origin fix/kc-family1-dispatch-atomicity
gh pr create --title "fix: campaign dispatch 补偿回滚+status门,消孤儿send/防重推 (批C家族①)" --body "$(cat <<'EOF'
## Summary
修复深度审查批C 跨环节根因家族①（触达多步非事务写 KC-01/02/03）：dispatch_campaign 每 contact 三步非原子写（占去重位→建 task→回填 taskId）任一 `?` 失败留孤儿 send（客户永久漏推）/卡 dispatching/report 失真。

- **补偿回滚**（KC-01/03）：task insert 失败→删 send；回填失败→删 task+send。每 contact all-or-nothing,无孤儿。
- **status 前置门**（KC-02）：抽 dispatch_allowed_from_status 纯函数（draft/previewed/dispatching 可派,completed/未知态拒）,防 completed 重推。
- **重入恢复**：dispatching 放行 + 去重跳过已完成 send + 补偿保证无孤儿 → 运营重新 dispatch 干净恢复。
- 不动 schema/report join/写序（send 是 unique 去重闸须最先占位）。

## Test plan
- [x] cargo test --lib（+dispatch_allowed_from_status 纯函数单测,baseline≥350 不回退）
- [x] no-human-takeover lint clean
- [x] 集成测（补偿回滚故障注入：validator 拒 agent_tasks→断言无孤儿 send；status 门拒 completed）,#[ignore] CI integration 跑
- 接受的窄窗口（回填失败删 task 时 worker 恰已 claim）：极罕见双重巧合,非红线破坏,见设计 §接受的窄窗口

设计：docs/superpowers/specs/2026-07-12-kc-family1-dispatch-atomicity-design.md
台账：docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md KC-01/02/03

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论
- **Spec coverage**：设计组件1（status 门纯函数）↔ Task1；组件2（补偿回滚）↔ Task2；组件3（重入恢复，由1+2 成立无额外代码）↔ 已在 Task1/2 注释体现；测试策略（纯函数单测+补偿故障注入集成测+status 门测）↔ Task1 Step2 + Task3。全覆盖。
- **Placeholder scan**：无 TBD/TODO；每 Step 给完整可编译代码 + 确切命令 + 预期。集成测用亲验的 make_campaign/make_contact/validator 范式。
- **Type consistency**：`dispatch_allowed_from_status(&str)->bool` Task1 定义、Task1 Step5 接线、Task1 Step2 单测一致；补偿删除 `delete_one(doc!{"_id":..},None)` 与现有 mongodb 2.8 用法一致；集成测 `campaignId`（camelCase）与 CampaignSend serde 一致；validator 目标集合 `agent_tasks`（db/mod.rs:83 亲验）。
- **TDD**：Task1 先写失败单测→实现→绿；Task2 check+baseline；Task3 集成测编码不变量（红→绿 CI 验证）。
- **红线**：每个改码 Task Step1 先读懂+亲验行号；补偿删除 best-effort（let _）；send/task 写序不动、schema 不动明确圈出。
