# 批A家族① 修复实施计划：审计/旁路事件 fail-soft 对齐 + reaction 解耦

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 让回复入队（`gateway.rs` outbox enqueue）之前的纯审计/旁路写在 DB 瞬时故障时不再吞掉本轮客户回复（B-02 + C-01 + H-01）。

**Architecture:** (1) `gateway.rs` `apply_agent_updates` 内 5 处纯审计 `write_event_for_account` 的 `.await?` 降级为 `let _ = ...await`，与同函数孪生（`dimension_dropped`/`stage_transition_rejected` 用 `let _`、bayesian 用 `if Err{warn}`）对齐；(2) `webhooks.rs` 把步骤 (e) 网关聚合回复从步骤 (d) reaction 的 `else` 分支解耦、无条件执行。测试：在既有 `c2_operation_state_derivation_e2e.rs` 加一个用 MongoDB validator 注入审计写失败的确定性 e2e，断言 outbox 仍入队。

**Tech Stack:** Rust 2021 / Axum / MongoDB (mongodb 2.8) / testcontainers。本地 `cargo test --lib` 跑单测；集成测试 `#[ignore]` 需 Docker，本地只 `--no-run` 编译，执行留 CI integration job。

## Global Constraints
- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——每个改码 Task 的 Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `gateway.rs` 那 5 处纯审计 emit + `webhooks.rs` 的 (e) 解耦 + 加一个 e2e 测试函数。**不改**：`reaction.rs` 内部 `?`、`gateway.rs` 内真实业务写（follow-up `insert_one` / 画像 `update` / bayesian 写回本身 / `pending_follow_up_count`）、KD-04 已改的 escalation、前端 / API 契约 / 配置 / 迁移。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed（scripts/check-baseline）。
- **no-human-takeover lint**：src/ 新增行不得含 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`。本修复用「审计/旁路/fail-soft/回复已异步发出/反应分析」既有措辞。
- **设计文档**：`docs/superpowers/specs/2026-07-12-audit-events-failsoft-alignment-design.md`。
- **台账**：`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（B-02/C-01/H-01）。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）
- `apply_agent_updates` 定义 `gateway.rs:3816`，调用点 `:2356`（`.await?` 于 `:2365`），**早于** 回复 enqueue `:2586`（`outbox_enqueue(state, enqueue_req)`）。故此函数内任一 `?` 失败 → 回复未入队即被吞。
- 函数内 **5 处纯审计 `.await?`**（gateway.rs，亲验当前行）：`4411` g1_correction、`4480` profile_churn_observed、`4503` operation_state_transition_rejected、`4525` operation_state_transitioned、`4551` follow_up_run_at_degraded。
- **孪生 fail-soft 铁证**（同函数）：`3942` `dimension_dropped` 用 `let _ = ...await`、`3988` `stage_transition_rejected` 用 `let _ = ...await`、`~4260` bayesian 用 `if let Err(...){warn}`。注释 `:3987`/`:3966` 明写"审计写失败不阻断主流程（回复已异步发出）"。
- `webhooks.rs:188-224`：步骤 (d) `if let Err(error) = agent::record_user_reaction(&state, &contact, &inbound).await { ...warn... } else { (e) }`——(e) `handle_managed_message_aggregated(&state, contact, &inbound, Some(guard))` 嵌在 `else` 里。(d) 借 `&contact`；(e) 移动 `contact`。(f)(g)（`:227+`）只用 `st.generation`/`PENDING`，不用 contact。
- 测试基建：`common::TestApp::start()`（`tests/common/mod.rs:130`）给 `app.state`(AppState) + `app.llm`(mock，`push_response(json)` 入队、`calls()` 计数)。`app.state.db.raw()`（`src/db/mod.rs:70`）暴露底层 `mongodb::Database`，可跑 `run_command`/`create_collection`。accessor：`app.state.db.contacts()`/`.messages()`/`.events()`/`.agent_run_logs()`/`.collection_agent_send_outbox()`。
- `c2_operation_state_derivation_e2e.rs` 已有 helper：`make_managed_contact(wxid,state)`、`make_inbound(contact,id,content)`、`reply_decision_json(customer_stage,operation_state)`、`review_pass_json()`，及 `handle_managed_message` 入口。其 `illegal_transition_keeps_old_state_and_audits_failsoft`（:323）用非法迁移 `customerStage="customer_success"` from `new_contact` 触发 `operation_state_transition_rejected` 审计写。

---

## Task 1: 加确定性 e2e——审计写失败时回复仍入队（编码 C-01/H-01 不变量）

**Files:**
- Modify: `tests/c2_operation_state_derivation_e2e.rs`（在文件末尾追加一个 `#[tokio::test] #[ignore]` 函数，复用现有 helper）

**Interfaces:**
- Consumes: 该文件现有 `make_managed_contact` / `make_inbound` / `reply_decision_json` / `review_pass_json` / `handle_managed_message`；`app.state.db.raw()` / `.contacts()` / `.messages()` / `.agent_run_logs()` / `.collection_agent_send_outbox()`。
- Produces: 无（纯测试）。

**为什么加在 c2 文件里**：本测试针对的正是 `apply_agent_updates` 在 `operation_state_transition_rejected` 写点的 fail-soft 行为，与 c2 文件主题一致；复用其全套 helper，避免 ~80 行 harness 重复（DRY）。

- [ ] **Step 1: 先读懂（红线）**

Read `tests/c2_operation_state_derivation_e2e.rs:1-60`（文件 doc + import）+ `:311-432`（`illegal_transition_keeps_old_state_and_audits_failsoft` 全貌——本测试复用它的 contact/decision/断言范式）。Grep 确认 `app.state.db.raw`（`grep -n "pub fn raw" src/db/mod.rs`）返回 `&mongodb::Database`、`collection_agent_send_outbox`/`agent_run_logs` accessor 名。确认 `handle_managed_message` 非法迁移路径会写 `agent.operation_state_transition_rejected`（c2:367 断言其存在）。**说不清就继续读。**

- [ ] **Step 2: 追加测试函数**

在 `tests/c2_operation_state_derivation_e2e.rs` 文件末尾追加：

```rust
/// 批A家族① C-01/H-01：apply_agent_updates 内**纯审计事件**写失败时，本轮回复仍须入队
/// （fail-soft）。用 MongoDB collection validator 让 `agent.operation_state_transition_rejected`
/// 的插入确定性失败，走非法迁移路径触发该审计写，断言 `agent_send_outbox` 仍有本轮回复一行。
///
/// - 修复前：gateway.rs `operation_state_transition_rejected` 写用 `.await?`，validator 拒写
///   → Err 冒泡出 `apply_agent_updates`（enqueue 之前）→ 回复不入队 → outbox 空 → 本测试失败。
/// - 修复后：该写降级 `let _ = ...await`，吞错继续 → 回复照常 enqueue → outbox 有一行 → 通过。
///
/// validator 仅拒该一个 kind，其余审计事件（stage_transition_rejected/profile_churn 等）与
/// `agent_send_outbox` 集合均不受影响。`#[ignore]`，需 Docker，由 CI integration job 跑。
#[tokio::test]
#[ignore]
async fn audit_write_failure_does_not_drop_reply_failsoft() {
    let app = common::TestApp::start().await;

    // 装 validator：拒绝 kind == agent.operation_state_transition_rejected 的插入。
    // create_collection 若集合已存在会报错，用 let _ 忽略；随后 collMod 装校验器。
    let _ = app
        .state
        .db
        .raw()
        .create_collection("agent_events", None)
        .await;
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "agent_events",
                "validator": { "kind": { "$ne": "agent.operation_state_transition_rejected" } },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install agent_events validator");

    let contact = make_managed_contact("user_audit_failsoft", "new_contact");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");
    let inbound = make_inbound(&contact, "msg_audit_failsoft_001", "你好，先简单了解一下。");
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    // 非法迁移（new_contact → customer_success）→ 触发 operation_state_transition_rejected
    // 审计写（被 validator 拒）。2 次 LLM：Reply + Review。
    app.llm
        .push_response(reply_decision_json("customer_success", "need_discovery"));
    app.llm.push_response(review_pass_json());

    // 不 .expect handle 的返回：本不变量是"回复入队"，与顶层 Result 是否上抛无关，
    // 用 let _ 使断言对修复前/后两种上抛行为都稳健。
    let _ = handle_managed_message(&app.state, contact.clone(), &inbound).await;

    // 核心断言：审计写失败时回复仍入队 outbox（证明未被吞）。
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    let outbox = app
        .state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "run_id": &log.run_id }, None)
        .await
        .expect("query outbox by run_id")
        .expect("审计写失败时回复仍须入队 outbox 一行（fail-soft 未吞回复）");
    assert_eq!(
        outbox.contact_wxid, contact.wxid,
        "outbox.contact_wxid 不一致：{:?}",
        outbox
    );
}
```

- [ ] **Step 3: 编译确认（本地无 Docker，只 --no-run）**

Run: `cargo test --test c2_operation_state_derivation_e2e --no-run 2>&1 | tail -15`
Expected: 编译成功（0 error）。**不本地跑**（`#[ignore]` + 需 Docker）；红→绿由 CI integration job 在最终 PR 上验证（Task 2 修复前该测失败、修复后通过）。若编译报 `run_command`/`create_collection` 签名不符，Read `src/` 内现有 mongodb 2.8 用法或查 `cargo doc` 修正参数（mongodb 2.8：`Database::run_command(Document, impl Into<Option<SelectionCriteria>>)`、`Database::create_collection(&str, impl Into<Option<CreateCollectionOptions>>)`）。

- [ ] **Step 4: Commit**

```bash
git add tests/c2_operation_state_derivation_e2e.rs
git commit -m "test(gateway): 审计写失败时回复仍入队的确定性e2e(批A家族①C-01/H-01)

用 MongoDB validator 拒写 operation_state_transition_rejected 审计事件,非法迁移路径触发,
断言 agent_send_outbox 仍有本轮回复一行。修复前 .await? 冒泡吞回复→失败;修复后 let _→通过。
#[ignore] 需 Docker,由 CI integration job 跑红→绿。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: gateway.rs 5 处纯审计 emit 降级 fail-soft

**Files:**
- Modify: `src/agent/gateway.rs`（5 处 `.await?` → `let _ = ...await;`，当前约 :4411/:4480/:4503/:4525/:4551）

**Interfaces:**
- Consumes: 无新增。
- Produces: 无（行为层降级：happy path 逐字不变，仅错误路径吞错继续）。

- [ ] **Step 1: 先读懂（红线）**

Grep 亲验 5 处纯审计 emit 的当前真实行号与上下文：
`grep -nE "purchase_lifecycle_corrected_by_objective|profile_churn_observed|operation_state_transition_rejected|operation_state_transitioned|follow_up_run_at_degraded" src/agent/gateway.rs`
逐个 Read 每处 `write_event_for_account(...).await?` 确认它是**纯审计**（前后无真实业务写依赖其 Ok）。同时 Read 孪生 `:3942 dimension_dropped`（`let _`）与 `:3988 stage_transition_rejected`（`let _`）确认目标写法。**核对**：这 5 处的 `.await?` 后面紧跟的都是 `}` 或纯观测代码，不是真实业务写。**说不清就继续读，不动手。**

⚠️ **只改这 5 处审计 emit**。其后 `follow_up` 任务 `insert_one`（约 :4557）、画像 contact `update`、`pending_follow_up_count` 等真实业务写**保持 `?` 不动**。

- [ ] **Step 2: 逐处改写**

把这 5 处的 `.await?;` 改为 `.await;` 并在该 `write_event_for_account(` 前保留/补一行 fail-soft 注释，与孪生对齐。每处形如：

```rust
    // fail-soft：纯审计写失败不阻断主流程（回复稍后异步入队），与 dimension_dropped 同风格。
    let _ = write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "agent.operation_state_transition_rejected",  // ← 各处 kind 不同，按原样保留
        // ...原有 status/summary/details 参数逐字不变...
    )
    .await;
```

对以下 5 个 emit 分别执行（kind 字符串各异，参数体逐字保留，仅 `write_event_for_account(` 前加 `let _ = `、结尾 `.await?;` → `.await;`）：
1. `agent.purchase_lifecycle_corrected_by_objective`（g1_correction）
2. `agent.profile_churn_observed`
3. `agent.operation_state_transition_rejected`
4. `agent.operation_state_transitioned`
5. `agent.follow_up_run_at_degraded`

第 5 处（follow_up_run_at_degraded）在更深缩进的 `if degraded {` 块内，注意缩进对齐。

- [ ] **Step 3: 编译确认**

Run: `cargo build --lib 2>&1 | tail -15`
Expected: 0 error。可能有"unused Result"类告警——`let _ =` 已显式丢弃，不应告警；若有 `#[must_use]` 相关告警说明某处漏了 `let _ =`，补上。

- [ ] **Step 4: baseline 不回退**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。`let _` 只丢 Result、写操作照常执行，happy-path 单测行为不变。

- [ ] **Step 5: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "fix(gateway): apply_agent_updates 内5处纯审计emit降级fail-soft(批A家族①C-01/H-01)

.await? → let _ = ...await,与同函数孪生 dimension_dropped/stage_transition_rejected(let _)对齐。
根治:enqueue(:2586)之前的纯审计写(g1纠偏/profile churn/state迁移拒绝+成功/follow_up降级)
在 Mongo 瞬时故障时 return Err 冒泡→本轮回复未入队即被吞。真实业务写保持 ? 不动。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: webhooks.rs 把 (e) 从 (d) 的 else 解耦

**Files:**
- Modify: `src/webhooks.rs`（约 :187-224，去掉 (e) 外层 `else {}`，使其无条件执行）

**Interfaces:**
- Consumes: 无。
- Produces: reaction 失败不再阻断本轮聚合回复。

- [ ] **Step 1: 先读懂（红线）**

Read `src/webhooks.rs:185-240` 亲验当前结构：(d) `if let Err(error) = agent::record_user_reaction(&state, &contact, &inbound).await { <warn> } else { <(e) guard + handle_managed_message_aggregated + warn> }`。确认 (d) 借 `&contact`、(e) 移动 `contact`；确认 (e) 之后（(f)/(g)）不再使用 `contact`（只用 `st.generation`/`PENDING`/`key`/`gen_at_start`）。**这决定解耦后借用检查能过。说不清就继续读。**

- [ ] **Step 2: 去掉 else，(e) 平级无条件执行**

把 (d) 的 `else {` 块拆开——(d) 只保留其 `if let Err {...warn...}`（无 else），(e) 的内容（guard 构造 + `handle_managed_message_aggregated` + 其 warn）移到 (d) 之后同一缩进层，无条件执行。改后形如：

```rust
            // (d) 一次反应分析（旁路：失败只 warn，绝不阻断本轮回复）。
            if let Err(error) = agent::record_user_reaction(&state, &contact, &inbound).await {
                let _ = agent::write_event_for_account(
                    &state,
                    &account_id,
                    Some(&from_wxid),
                    "agent_error",
                    "failed",
                    &format!("record_user_reaction failed: {error}"),
                    app_id.clone().map(|v| doc! { "app_id": v }),
                )
                .await;
            }

            // (e) 一次聚合网关（无条件执行——与 (d) 解耦：reaction 是对上一轮结果的旁路分析，
            // 与生成本轮回复无因果依赖，其失败绝不该吞本轮应答）。带协作式抢占 guard。
            let guard_state = st.clone();
            let guard: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
                barge_in_triggered(gen_at_start, guard_state.generation.load(Ordering::Acquire))
            });
            if let Err(error) = agent::handle_managed_message_aggregated(
                &state,
                contact,
                &inbound,
                Some(guard),
            )
            .await
            {
                let _ = agent::write_event_for_account(
                    &state,
                    &account_id,
                    Some(&from_wxid),
                    "agent_error",
                    "failed",
                    &error.to_string(),
                    app_id.clone().map(|v| doc! { "app_id": v }),
                )
                .await;
            }
```

**注意**：`reaction.rs` 内部 `?` **不动**（解耦后其失败已不吞回复，改它属范围外）。仅动 webhooks.rs 这段控制流。

- [ ] **Step 3: 编译确认（含借用检查）**

Run: `cargo build --lib 2>&1 | tail -15`
Expected: 0 error。若报 `use of moved value: contact` 或 `borrow of moved value`，说明 (e) 之后仍有代码用 contact——回 Step 1 重新亲验 (f)/(g)，不要靠 clone 绕过（应确认结构确实不再用 contact）。

- [ ] **Step 4: baseline 不回退**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。

- [ ] **Step 5: Commit**

```bash
git add src/webhooks.rs
git commit -m "fix(webhooks): (e)网关从(d)reaction的else解耦,无条件执行(批A家族①B-02)

reaction 是对上一轮结果的旁路分析,与本轮回复无因果依赖。原将(e)嵌在(d)的else里,
使 reaction.rs 内部 ? 上抛(Mongo瞬时故障)时(e)根本不执行→本轮客户回复被吞。
解耦后 reaction 失败只 warn、不再阻断(e)。reaction.rs 内部 ? 不动。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 全量 baseline + lint + push + PR

**Files:** 无改动（纯验证 + 交付）

- [ ] **Step 1: 全量 lib 测试**

Run: `cargo test --lib 2>&1 | tail -10`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350（baseline 不回退）。

- [ ] **Step 2: no-human-takeover lint 自检**

Run: `git diff origin/main -- src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`。

- [ ] **Step 3: 集成测试编译确认（本地上限）**

Run: `cargo test --test c2_operation_state_derivation_e2e --no-run 2>&1 | tail -8`
Expected: 编译成功。执行留 CI（本地无 Docker）。

- [ ] **Step 4: push + 开修复 PR**

```bash
git push -u origin fix/audit-events-failsoft-alignment
gh pr create --title "fix: 审计/旁路事件 fail-soft 对齐,DB抖动不再吞回复 (批A家族①)" --body "$(cat <<'EOF'
## Summary
修复深度审查批A 跨环节根因家族①（B-02 + C-01 + H-01）：回复入队（gateway outbox enqueue :2586）之前的纯审计/旁路写用了 `?`，Mongo 瞬时故障即把本轮客户回复吞掉。

- **C-01/H-01**（gateway.rs）：`apply_agent_updates`（调用点 :2356 早于 enqueue :2586）内 5 处纯审计 `write_event_for_account` 的 `.await?` 降级 `let _ = ...await`（g1纠偏/profile churn/operation_state 迁移拒绝+成功/follow_up 降级），与同函数孪生 dimension_dropped/stage_transition_rejected(`let _`) 对齐。真实业务写（follow-up insert/画像 update）保持 `?` 不动。
- **B-02**（webhooks.rs）：(e) 网关聚合回复从 (d) reaction 的 `else` 解耦、无条件执行。reaction 是旁路分析，其 DB 故障不该吞本轮回复。reaction.rs 内部 `?` 不动。

## Test plan
- [x] cargo test --lib（baseline ≥ 350 / 0 failed 不回退；`let _` 只丢 Result、happy-path 行为逐字不变）
- [x] no-human-takeover lint clean
- [x] 新增确定性 e2e（c2_operation_state_derivation_e2e.rs::audit_write_failure_does_not_drop_reply_failsoft）：MongoDB validator 拒写审计事件 → 断言 outbox 仍入队。`#[ignore]`，CI integration job 跑红→绿。
- [ ] B-02：控制流重构 + 借用安全性亲验，不单写测（用户裁定）

设计：docs/superpowers/specs/2026-07-12-audit-events-failsoft-alignment-design.md
台账：docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md B-02/C-01/H-01

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论
- **Spec coverage**：设计三块（C-01/H-01 五处降级 / B-02 解耦 / 测试策略）↔ Task2（gateway 5 处）/ Task3（webhooks 解耦）/ Task1（e2e）+ Task4（baseline+PR），全覆盖。B-02"亲验不单测"在 Task3 落实（无测试 step，靠借用检查 + 亲验）。
- **Placeholder scan**：无 TBD/TODO；每 Step 给完整代码/命令/预期。测试代码用亲验的 c2 helper + TestApp accessor。
- **Type consistency**：`app.state.db.raw()`→`&mongodb::Database`（db/mod.rs:70）；`run_command(Document, None)`/`create_collection(&str, None)`（mongodb 2.8）；`collection_agent_send_outbox()`/`agent_run_logs()` accessor 与 c2 一致；`make_managed_contact`/`reply_decision_json`/`review_pass_json` 签名与 c2 现有定义一致。
- **TDD**：Task1 先写测试（编码不变量）→ Task2/3 实现 → 红→绿在 CI 验证（本地无 Docker，`--no-run` 只编译，已显式说明）。Task2/3 各以 `cargo test --lib` baseline 守回退。
- **红线**：每个改码 Task 的 Step 1 都是"先读懂 + 亲验当前行号"，且明确圈出"只改这 N 处、真实业务写不动"。
