# 批D relay 数字护栏修复实施计划：删除错误的字符级数字 backstop（KD-01 + KD-03）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 删除 `src/agent/gateway.rs` relay 出站守卫里的数字护栏（`relay_introduces_unauthorized_number`）fail-closed 用法，保留载荷泄漏守卫，使 relay 转述忠实度交还给 LLM（prompt）+ 独立 Review Agent —— 同源根治 KD-01（字符护栏中文盲区/双向失效）与 KD-03（误杀致裁决黑洞）。

**Architecture:** 单文件外科式删除。gateway.rs 的 relay 守卫块里 `leaks_payload`（载荷泄漏，确定性字符串检测，威胁模型正确，**保留**）与 `unauthorized_number`（数字护栏，威胁模型错误，**删除**）耦合在同一 if。只摘数字护栏分支、把条件收敛回 `if leaks_payload`、简化命中文案、同步注释。`logic.rs` 三个数字函数及其单测**不删**（`holding_reply.rs:26` 仍是合法调用者）。

**Tech Stack:** Rust 2021 / Axum。改动只在 `src/agent/gateway.rs`。验证 `cargo test --lib`（baseline ≥ 350 / 0 failed 不回退）。

## Global Constraints

- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `src/agent/gateway.rs`（relay 出站守卫块）。**不改** `logic.rs` 的 `relay_introduces_unauthorized_number` / `extract_number_tokens` / `normalize_number_token` 三函数及其单测（holding_reply 仍用，删则破坏编译）；**不改** `holding_reply.rs`（其调用语义正确、有兜底非黑洞）；**不改** `relay_output_leaks_internal_payload`（载荷泄漏守卫，保留 fail-closed）；**不改** relay task 生命周期 / `clear_awaiting_principal_state` / `run_user_operation_gateway` 返回类型 / prompt / review。
- **不引入** sent 回传接口、KD-03 补偿话术、中文数字归一化、review 增补维度（spec 已论证全部 YAGNI）。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。
- **no-human-takeover lint**：gateway 删除/改注释的新增行用「转述/裁决/授权/载荷」措辞，无禁词（`人工接管/接管/人工` 等）。
- **设计文档**：`docs/superpowers/specs/2026-07-12-kd-family-relay-number-guard-design.md`。
- **台账**：`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KD-01/KD-03。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）

- `src/agent/gateway.rs` relay 出站守卫块 **:2473-2520**（Task 前主控亲验）：
  - `:2470-2472` `let mut outbox_eligible = ...`（should_reply && !reply_text.trim().is_empty() && status∈{approved,revision_applied_approved}）。
  - `:2480` `if outbox_eligible && escalation::is_principal_relay_trigger(&trigger) {`。
  - `:2481-2484` `let authorized_payload = match &trigger { Inbound(m)=>m.content.as_str(), FollowUp(_)=>"" };`（**只被数字护栏用**，删数字护栏后应一并删除，否则 unused 变量告警）。
  - `:2485-2486` `let leaks_payload = escalation::relay_output_leaks_internal_payload(&final_decision.reply_text);`（**保留**）。
  - `:2487-2490` `let unauthorized_number = escalation::relay_introduces_unauthorized_number(&final_decision.reply_text, authorized_payload);`（**删除**）。
  - `:2491` `if leaks_payload || unauthorized_number {` → 改为 `if leaks_payload {`。
  - `:2492` `outbox_eligible = false;`（保留，leaks 命中仍 fail-closed）。
  - `:2493-2503` `let (warn_reason, event_reason) = if leaks_payload { (...) } else { (...) };`（三元分支现只剩 leaks 情形 → 简化为两个固定 `let` 绑定，删掉 else 的"授权外数字"文案）。
  - `:2504-2508` `tracing::warn!(... "{warn_reason}")`（保留，用简化后的 warn_reason）。
  - `:2509-2518` `write_event_for_account(... "blocked_review", "blocked_by_safety_guard", event_reason, None).await?;`（保留，用简化后的 event_reason）。
  - `:2519-2520` `}` `}`。
- `escalation::relay_introduces_unauthorized_number` 的另一调用点 `src/agent/escalation/holding_reply.rs:26`（**不动**）→ 删 gateway 调用点后该函数不会变死代码。
- `relay` 调用 `run_user_operation_gateway` 传 `should_abort_send = None`（gateway.rs:768-774）→ 去抖中止对 relay 不生效（KD-03 残留窗口分析支柱，无需本计划动作，仅确认）。
- **tests/ 目录无针对数字护栏 fail-closed 的集成测**（主控 grep 全 tests/ 确认；`ask_human_phase1_e2e.rs` 测 relay task 生命周期，与数字护栏无关）。数字护栏单测只在 `logic.rs`（测函数本身，保留）。

---

## Task 1: 删除 gateway relay 数字护栏 fail-closed 用法（唯一 task）

**Files:**
- Modify: `src/agent/gateway.rs`（relay 出站守卫块，约 :2473-2520）

**Interfaces:**
- Consumes: `escalation::relay_output_leaks_internal_payload`（保留调用）、`escalation::is_principal_relay_trigger`（保留）。
- Produces: 无新接口。行为层：relay 转述含"授权外数字"不再被 fail-closed 拦截（交 LLM/review）；含内部载荷标记仍 fail-closed。

- [ ] **Step 1: 先读懂（红线）**

Read `src/agent/gateway.rs:2465-2525`（`outbox_eligible` 定义 → relay 守卫块 → 块后紧邻代码）。Grep 亲验：
- `grep -n "relay_introduces_unauthorized_number\|relay_output_leaks_internal_payload\|authorized_payload\|unauthorized_number" src/agent/gateway.rs` —— 确认数字护栏在 gateway 仅此一处、`authorized_payload` 仅被数字护栏用。
- `grep -rn "relay_introduces_unauthorized_number" src/agent/escalation/` —— 确认 holding_reply.rs 仍调它（故 logic.rs 函数不删）。
确认当前真实行号（行号可能已漂）。**说不清就继续读，不动手。**

- [ ] **Step 2: 外科式删除数字护栏，保留载荷泄漏守卫**

把 relay 守卫块（当前 :2480-2520，`if outbox_eligible && is_principal_relay_trigger { ... }`）**整体替换**为下面版本（删 `authorized_payload` + `unauthorized_number` + else 文案分支，条件收敛为 `if leaks_payload`，注释同步）：

```rust
    // relay 出站红线守卫（代码级兜底）：relay 转述绝不透传内部载荷
    // （__PRINCIPAL_RELAY__/verdict=/substance=/constraints=）。命中即 fail-closed：
    // 不入队该文本（宁可客户这轮收不到，也绝不把内部载荷标记发给客户），记 event + warn
    // 供运维定位。非 relay run 不受影响。
    //
    // 注：转述是否忠于领导授权（不编造授权外折扣/数字）由生成侧 prompt（substance 是
    // 唯一事实源）+ 独立 Review Agent（已同时看到授权 substance 与拟发转述）做语义级把关，
    // 不再用字符级数字白名单 backstop——后者威胁模型错误（既漏中文数字、又误杀无害的
    // 时间/序数/等价折扣数字，其 fail-closed 曾致 KD-03 裁决黑洞），已删除（KD-01/03）。
    if outbox_eligible && escalation::is_principal_relay_trigger(&trigger) {
        let leaks_payload =
            escalation::relay_output_leaks_internal_payload(&final_decision.reply_text);
        if leaks_payload {
            outbox_eligible = false;
            tracing::warn!(
                %run_id,
                contact_wxid = %contact.wxid,
                "relay 转述拟发文本疑似泄漏内部载荷，已拦截不发（fail-closed）"
            );
            write_event_for_account(
                state,
                &contact.account_id,
                Some(&contact.wxid),
                "blocked_review",
                "blocked_by_safety_guard",
                "relay 转述输出含内部载荷标记，安全门拦截不发送",
                None,
            )
            .await?;
        }
    }
```

- [ ] **Step 3: cargo check 确认编译 + 无 unused 告警**

Run: `cargo check --lib 2>&1 | tail -20`
Expected: 0 error、0 warning（尤其无 `unused variable: authorized_payload` / `unused: relay_introduces_unauthorized_number` —— 后者因 holding_reply 仍用故不应告警）。若报 `relay_introduces_unauthorized_number` unused，说明 holding_reply 调用点判断有误，**停下报告**（不得为消告警删 logic.rs 函数或改 holding_reply）。

- [ ] **Step 4: baseline 不回退**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: `ok. N passed; 0 failed`，N ≥ 350。（logic.rs 的 `relay_introduces_unauthorized_number` / holding_reply 数字守卫单测应仍全绿——证明函数存活、holding_reply 未受波及。）

- [ ] **Step 5: no-human-takeover lint 自检**

Run: `git diff origin/main -- src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`。

- [ ] **Step 6: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "fix(escalation): 删 relay 数字护栏 fail-closed,忠实度交 LLM/review (KD-01/03)

relay_introduces_unauthorized_number 是威胁模型错误的字符级 backstop:判断转述是否
忠于授权是上下文依赖的语义问题,逐字提数字白名单比对既漏中文八折又误杀 24小时/等价折扣,
其 fail-closed 是 KD-03 裁决黑洞唯一现实误杀源。删 gateway 该用法,忠实度交还生成侧
prompt(substance 唯一事实源)+独立 review(已看到 substance+拟发转述)。保留载荷泄漏守卫
(确定性字符串检测,威胁模型正确)。logic.rs 三函数+holding_reply 调用不动(仍合法用)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: baseline 复核 + PR（主控做 push/PR）

**Files:** 无代码改动（收尾）。

- [ ] **Step 1: 全量 baseline 复核**

Run: `cargo test --lib 2>&1 | tail -8` → `ok. N passed; 0 failed`，N ≥ 350。
（相关集成测若涉及 relay 出站，`cargo test --test ask_human_phase1_e2e --no-run 2>&1 | tail -5` 本地编译确认，执行留 CI Docker。）

- [ ] **Step 2: push + PR（主控执行，实现者到 Task 1 commit 即止）**

```bash
git push -u origin fix/kd-relay-number-guard
gh pr create --title "fix: 删 relay 数字护栏 fail-closed,忠实度交 LLM/review (KD-01/03)" --body "$(cat <<'EOF'
## Summary
修复深度审查批D relay 授权外数字护栏家族（KD-01 + KD-03）：`relay_introduces_unauthorized_number` 是威胁模型错误的字符级 backstop——判断"转述是否忠于领导授权"是上下文依赖的语义问题，逐字提数字做白名单差比对是范畴错误。

- **KD-01**（字符护栏双向失效）：只认 ASCII 数字 → 漏中文"八折"（绕过编造折扣）、误杀"9折"vs 授权"九折"（拦正确转述）。
- **KD-03**（误杀致裁决黑洞）：数字护栏误杀（含"24小时""第2个问题"等无害数字）→ 不发 + 无条件清 awaiting → 领导裁决永久丢失。
- **修复**：删 gateway 数字护栏 fail-closed 用法，relay 忠实度交还生成侧 prompt（substance 唯一事实源）+ 独立 Review Agent（已同时看到授权 substance 与拟发转述，具备语义判断全部上下文）。
- **保留**：载荷泄漏守卫 `relay_output_leaks_internal_payload`（确定性字符串检测，威胁模型正确，仍 fail-closed）；`holding_reply.rs` 数字调用（有兜底非黑洞，语义正确）；logic.rs 三函数（holding_reply 仍用）。
- **YAGNI 砍掉**：中文归一化 / KD-03 补偿话术 / sent 回传 / review 增补维度（去抖对 relay 不生效，删护栏即根治黑洞）。

## Test plan
- [x] cargo test --lib（baseline ≥ 350 / 0 failed 不回退；logic.rs 数字函数单测+holding_reply 守卫单测仍绿=函数存活、holding_reply 未受波及）
- [x] no-human-takeover lint clean
- [x] 集成测编译（ask_human_phase1_e2e --no-run）
- 接受的窄窗口：载荷泄漏守卫命中→不发→清 awaiting（极罕见且属正确拦截，见设计 §KD-03 残留窗口分析），YAGNI 不为它引入 sent 回传接口。

设计：docs/superpowers/specs/2026-07-12-kd-family-relay-number-guard-design.md
台账：docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md KD-01/03

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论

- **Spec coverage**：设计唯一改动（删 gateway 数字护栏 fail-closed、保留载荷泄漏守卫）↔ Task 1 Step 2。测试策略（改断言不存在→保留 logic 单测/holding_reply 单测、baseline 不回退）↔ Task 1 Step 4 + Task 2 Step 1。范围边界（不动三函数/holding_reply/返回类型）↔ Global Constraints。全覆盖。
- **Placeholder scan**：无 TBD/TODO；Step 2 给完整可编译替换代码 + 确切命令 + 预期。
- **Type consistency**：`leaks_payload: bool`、`relay_output_leaks_internal_payload(&str)->bool`、`write_event_for_account(...).await?` 与现有一致；删除的 `authorized_payload`/`unauthorized_number` 一并移除无残留引用。
- **红线**：Step 1 先读懂 + 亲验行号 + Grep 确认 holding_reply 仍调（决定不删函数）；Step 3 显式检查 unused 告警作为"函数是否真被 holding_reply 用"的编译级验证，误判即停报告不擅自删函数。
- **YAGNI**：单 task 单文件删除，无新增接口/补偿/归一化。
