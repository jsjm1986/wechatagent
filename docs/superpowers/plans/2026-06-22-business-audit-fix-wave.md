# 业务审查修复波 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复业务审查发现的 11 条缺陷（4 Critical + 7 Important + Minor），分六组（A 请示通道闭环 / B 知识时效与缺口 / C 决策写入可靠性 / D 跟进节奏 / E 账号健康防护 / F 多模态入站地基）。

**Architecture:** 全部为后端 Rust（Axum + MongoDB）修复。A–E 是纯代码修复（护栏只兜客观边界：状态机字典/数字事实/知识时效/账号在线，不夺 LLM 语义判断）；F 做多模态入站代码地基 + 外部依赖打桩。设计依据：`docs/superpowers/specs/2026-06-22-business-audit-fix-wave-design.md`。

**Tech Stack:** Rust 2021 / Axum / MongoDB (mongodb crate, BSON) / tokio。测试 = `cargo test --lib`（纯函数）+ testcontainers 集成测试（`#[ignore]`，CI 跑）。

## Global Constraints

- **不夺 LLM 语义判断**（agent-first）：护栏只兜客观边界（状态机字典、数字白名单、知识时效、账号在线状态）。凡语义判断（"这句话是否做了承诺"）不加确定性硬拦。
- **反过拟合**：护栏是可复现抽象方法论，不针对单条对话/单次样本点对点修补。测试用多变体验证泛化。
- **客户永不被晾死、永不直接面对真人**：A 组话术全用 AI 自治措辞（"我再帮您同步/确认"），绝不出现 `人工接管/转人工/hand-off/接管/人工` 等词。受 `scripts/check-no-human-takeover.{sh,ps1}` lint 约束（扫 `git diff` 新增行）。
- **AI 永不自动 verify 知识**：B 组只收紧"过期知识不可背书"，不放宽任何 verify 路径。
- **测试基线不可回退**：`cargo test --lib` ≥ 350 passed / 0 failed；四 PBT 文件（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥ 33 / 0。新增测试只增量叠加，不删改旧维度。
- **改 prompt 必 bump `PROMPT_PACK_VERSION`**（prompts.rs）：C 组 ⑥ 涉及。
- **新字段向后兼容**：MongoDB struct 新增字段必须 `#[serde(default)]` 或 `Option<T>`，兼容老文档。
- **fail-soft 审计**：回复路径已决后的审计/画像写失败用 `let _ =`，不阻断主流程（参照 gateway.rs `agent.dimension_dropped` 模式）。
- **磁盘纪律**：本地只跑 `cargo test --lib` + 单 PBT 文件（`cargo test --test <name>`）；整合套件交 CI。编译前可 `rm -rf target/debug/incremental`。
- **提交边界**：精确 `git add` 仅本任务改的文件，绝不 `git add -A`/`.`。排除并行会话产物（`.kiro/specs/universal-test-coverage/*`、`AGENTS.md`、`agent_t*.txt`、`t15_single.txt`、`docs/superpowers/plans/2026-06-18-*.md`、`docs/superpowers/plans/2026-06-21-sales-media-asset-send.md`）。git 操作从仓库根带 `-C "E:/yw/agiatme/工作项目/wechatagent"`。
- **子代理 model:opus**；回复中文。

---

## A 组 · 请示通道闭环（②③⑩）

### Task A1: ⑩ 转述编造授权外数字护栏（纯函数 + 接线）

**Files:**
- Modify: `src/agent/escalation/logic.rs`（新增纯函数 + 单测，紧邻现有 `relay_output_leaks_internal_payload` :174）
- Modify: `src/agent/gateway.rs:1769-1830`（relay 出站门接线，紧邻现有泄漏检测调用）

**Interfaces:**
- Produces: `pub(crate) fn relay_introduces_unauthorized_number(reply_text: &str, authorized_substance: &str) -> bool`
- Consumes: 现有 `relay_output_leaks_internal_payload`（同文件 :174）的调用点（gateway.rs:1778）

**背景**：`relay_output_leaks_internal_payload`（logic.rs:174）只检测载荷泄漏（哨兵 / `verdict=` / `substance=` / `constraints=`），不检测转述里出现授权 substance 之外的数字/金额。领导授权"9 折"，AI 转述成"8 折"或加"再送一年质保" → 当前无拦。

- [ ] **Step 1: 写失败测试（纯函数真值表 + 多形态变体）**

在 `src/agent/escalation/logic.rs` 的 `#[cfg(test)] mod tests`（文件末尾现有测试模块）加：

```rust
#[test]
fn relay_unauthorized_number_detects_fabricated_discount() {
    // 领导授权"9折"，转述说"8折" → 引入授权外数字 → 拦。
    let substance = "可以给客户9折优惠";
    assert!(relay_introduces_unauthorized_number("我帮您申请到8折了", substance));
}

#[test]
fn relay_unauthorized_number_allows_authorized_number() {
    // 转述里的数字都在授权 substance 内 → 放行。
    let substance = "可以给客户9折，质保2年";
    assert!(!relay_introduces_unauthorized_number("给您9折，质保2年", substance));
}

#[test]
fn relay_unauthorized_number_allows_no_numbers() {
    // 转述无任何数字 → 放行（纯定性转述）。
    let substance = "可以适当让利";
    assert!(!relay_introduces_unauthorized_number("我帮您争取了一些优惠", substance));
}

#[test]
fn relay_unauthorized_number_detects_added_percentage() {
    // 授权无百分比，转述编造"95%成功率" → 拦。
    let substance = "这个方案可行";
    assert!(relay_introduces_unauthorized_number("成功率有95%", substance));
}

#[test]
fn relay_unauthorized_number_allows_substance_superset() {
    // 转述只用了授权数字的子集 → 放行。
    let substance = "9折，满3000减500，质保2年";
    assert!(!relay_introduces_unauthorized_number("给您9折优惠", substance));
}

#[test]
fn relay_unauthorized_number_ignores_non_quantitative_digits() {
    // 边界：授权含"9折"，转述复述"9折"但措辞不同 → 同一数字 token → 放行。
    let substance = "9折";
    assert!(!relay_introduces_unauthorized_number("可以9折", substance));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib relay_unauthorized_number 2>&1 | tail -15`
Expected: FAIL（函数未定义，编译错误 `cannot find function relay_introduces_unauthorized_number`）

- [ ] **Step 3: 实现纯函数**

在 logic.rs `relay_output_leaks_internal_payload`（:179 结束）后插入。数字 token 提取规则：连续数字（含小数点、百分号、中文"折"前的数字）归一化为可比较 token。实现：

```rust
/// 提取文本中的"数量事实" token：阿拉伯数字串（含小数点）。归一化为去前导零的数字字符串。
/// 用于 relay 转述的数字白名单核验——只看客观数量，不碰措辞。
fn extract_number_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || (ch == '.' && !cur.is_empty()) {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(normalize_number_token(&cur));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(normalize_number_token(&cur));
    }
    out
}

/// 归一化数字 token：去尾随小数点、去多余前导零（保留至少一位）。
fn normalize_number_token(s: &str) -> String {
    let trimmed = s.trim_end_matches('.');
    let (int_part, frac_part) = match trimmed.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (trimmed, None),
    };
    let int_norm = int_part.trim_start_matches('0');
    let int_norm = if int_norm.is_empty() { "0" } else { int_norm };
    match frac_part {
        Some(f) => format!("{}.{}", int_norm, f),
        None => int_norm.to_string(),
    }
}

/// relay 转述数字护栏：转述文本若出现授权 substance 里没有的数量事实（数字/百分比/
/// 金额）→ 返回 true（fail-closed，网关据此不发该转述）。领导授权"9折"，转述编成
/// "8折"或追加授权外的"95%成功率"都会命中。只兜客观数量，不判断措辞语义。
/// 纯函数，反过拟合（测多种数字形态变体）。
pub(crate) fn relay_introduces_unauthorized_number(
    reply_text: &str,
    authorized_substance: &str,
) -> bool {
    let authorized: std::collections::HashSet<String> =
        extract_number_tokens(authorized_substance).into_iter().collect();
    extract_number_tokens(reply_text)
        .into_iter()
        .any(|tok| !authorized.contains(&tok))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib relay_unauthorized_number 2>&1 | tail -15`
Expected: PASS（6 个测试全绿）

- [ ] **Step 5: 接线到 relay 出站门**

读 `src/agent/gateway.rs:1769-1830` 现有 relay 泄漏检测块。当前（:1778）：
```rust
&& escalation::relay_output_leaks_internal_payload(&final_decision.reply_text)
```
在该 fail-closed 条件旁并联数字护栏。**需要拿到 authorized_substance**：relay run 的授权实质来自 escalation entry 的 decision.substance（经 `relay_substance_if_usable` 取得）。定位 relay run 上下文里持有的 substance 变量（grep gateway.rs relay 块里 `substance`），把它传入。若 relay 块当前未持有 substance 字符串，从 `final_decision` 所属 relay 上下文取（实现者需在该函数作用域内找到 substance 来源；若不可得，报告 NEEDS_CONTEXT 说明 gateway relay 块的 substance 可见性）。

接线形态（示意，实际变量名以现场为准）：
```rust
if escalation::relay_output_leaks_internal_payload(&final_decision.reply_text)
    || escalation::relay_introduces_unauthorized_number(&final_decision.reply_text, &authorized_substance)
{
    // fail-closed：不发泄漏/编造数字的转述，回落安全话术（沿用现有泄漏处置分支）
}
```

- [ ] **Step 6: 编译 + lint**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `bash "E:/yw/agiatme/工作项目/wechatagent/scripts/check-no-human-takeover.sh" 2>&1 | tail -3`
Expected: lint 0（新增行无禁词）

- [ ] **Step 7: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/escalation/logic.rs src/agent/gateway.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(escalation): relay转述数字白名单护栏(授权外数字fail-closed,⑩)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task A2: ② 授权过期清 awaiting + 发中性收尾话术

**Files:**
- Modify: `src/agent/gateway.rs`（`clear_awaiting_principal_state` :598 附近提为 `pub(crate)`）
- Modify: `src/agent/escalation/mod.rs:181-185`（早退分支）
- Test: `tests/principal_decision_channel.rs`（集成测试，`#[ignore]`）

**Interfaces:**
- Consumes: `gateway::clear_awaiting_principal_state`（提为 pub(crate)）、`mcp::logged_call_for_account`、`fallback_holding_reply` 或新中性收尾话术常量
- Produces: 早退分支不再裸 `return Ok(())`，而是清 awaiting + 发收尾话术

**背景**：`escalation/mod.rs:182` `relay_substance_if_usable(...).is_none()` 时直接 `return Ok(())`，清 awaiting 在其后的 `relay_principal_decision_to_customer`（gateway.rs:598）里 → 授权过期时客户零反馈 + awaiting 永久残留（下一轮 `build_decision_signals_text` 仍读到"等待裁决"，永久压制对该议题的自主回复）。

- [ ] **Step 1: 确认 clear_awaiting_principal_state 现状**

读 `src/agent/gateway.rs:590-610`（clear_awaiting 实现）。确认其签名与可见性。它当前应做 `$unset domain_attributes.<AWAITING_PRINCIPAL_DECISION_ATTR>`。记录其精确签名（参数：state、contact 或 workspace/account/wxid 三元组）。

- [ ] **Step 2: 提为 pub(crate)（若当前更窄）**

若 `clear_awaiting_principal_state` 当前是 `fn`（模块私有）或 `async fn` 私有，改为 `pub(crate) async fn`，使 escalation 模块可调。不改其实现逻辑。

- [ ] **Step 3: 写集成测试（失败）**

在 `tests/principal_decision_channel.rs` 加（参照现有 §14.10 系列测试的 fixture 构造方式，如 `insert_pending_with_updated_at` / `find_escalation` / `find_contact`）：

```rust
/// §14.11（②授权过期闭环）：relay task 跑时授权已过期 → 不发过期承诺，但必须
/// ①清客户 awaiting 标记 ②发一条不含 substance 的中性收尾话术。否则客户零反馈 +
/// awaiting 永挂、永久压制对该议题的自主回复。
#[tokio::test]
#[ignore]
async fn t_relay_expired_authorization_clears_awaiting_and_sends_neutral() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    // 构造一条 resolved escalation，decision 带 substance，但 authorization_expires_at 已过期。
    // 客户 domain_attributes.awaiting_principal_decision = true。
    // （沿用本文件现有 helper 插 escalation + 设 contact awaiting 标记；
    //  authorization_expires_at = now - 1h）
    // ... fixture 构造（实现者按本文件现有 helper 风格写）...

    // 触发 relay task 处理。
    // ... 调 handle_principal_decision_relay 或经 task worker 入口 ...

    // 断言①：客户 awaiting 标记已清。
    let contact = find_contact(&app, /* wxid */).await;
    let awaiting = contact.domain_attributes
        .as_ref()
        .and_then(|d| d.get_bool(wechatagent::models::AWAITING_PRINCIPAL_DECISION_ATTR).ok())
        .unwrap_or(false);
    assert!(!awaiting, "授权过期早退也必须清 awaiting 标记");

    // 断言②：客户收到一条消息（中性收尾），且不含过期 substance 的具体内容。
    // （查 mcp mock 收到的 send_text，或查 conversation_messages 出站记录）
    // ... 断言收到中性话术、不含 substance 数字 ...
}
```

> 实现者注：fixture 细节（如何插 resolved+过期 escalation、如何设 awaiting）参照本文件 §14.10 系列与 `minimal_pending_escalation`。若现有 helper 不足以构造"resolved + decision.substance + 过期授权"，新增一个 helper（增量叠加，不改旧 helper）。

- [ ] **Step 4: 运行测试确认失败**

Run: `cargo test --test principal_decision_channel t_relay_expired_authorization -- --ignored 2>&1 | tail -20`
Expected: FAIL（当前早退不清 awaiting、不发话术）。**若本地无 Docker** → 跳过执行，标注"待 CI 验证"，继续 Step 5（纯逻辑改动可先实现）。

- [ ] **Step 5: 改早退分支**

`src/agent/escalation/mod.rs:181-185`，把：
```rust
let now = mongodb::bson::DateTime::now();
if relay_substance_if_usable(&decision, entry.authorization_expires_at, now).is_none() {
    // 授权过期：不拿过期授权乱承诺，结束。
    return Ok(());
}
```
改为（先取 contact，再清 awaiting + 发中性收尾）：
```rust
let now = mongodb::bson::DateTime::now();
if relay_substance_if_usable(&decision, entry.authorization_expires_at, now).is_none() {
    // 授权过期：不拿过期授权乱承诺，但议题已被领导处理过——必须清 awaiting 标记
    // （否则永久压制对该议题的自主回复）+ 发一条不含 substance 的中性收尾话术
    // （否则客户零反馈）。下一轮客户来消息正常对话接管。
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid
            },
            None,
        )
        .await?;
    if let Some(contact) = contact {
        crate::agent::gateway::clear_awaiting_principal_state(state, &contact).await?;
        let _ = mcp::logged_call_for_account(
            state,
            &contact.account_id,
            "message_send_text",
            serde_json::json!({
                "recipient": &contact.wxid,
                "content": expired_authorization_neutral_reply()
            }),
        )
        .await;
    }
    return Ok(());
}
```

> `clear_awaiting_principal_state` 的精确调用签名以 Step 1 确认的为准（可能是 `(state, &contact)` 或 `(state, workspace, account, wxid)`）。

- [ ] **Step 6: 加中性收尾话术常量**

在 `escalation/mod.rs`（或 escalation 话术常量所在处，与 `fallback_holding_reply` 同处）加：
```rust
/// 授权过期的中性收尾话术：不复述任何过期承诺/数字，只表达"会继续跟进"。
/// AI 自治口吻，绝不出现"人工/转人工/接管"等禁词。
fn expired_authorization_neutral_reply() -> String {
    "关于您之前问的那件事，我这边再帮您核实下最新情况，有确切消息第一时间同步您～".to_string()
}
```

- [ ] **Step 7: 编译 + lint**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `bash "E:/yw/agiatme/工作项目/wechatagent/scripts/check-no-human-takeover.sh" 2>&1 | tail -3`
Expected: lint 0

- [ ] **Step 8: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/escalation/mod.rs src/agent/gateway.rs tests/principal_decision_channel.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(escalation): 授权过期早退也清awaiting+发中性收尾话术(②客户不被晾死)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task A3: ③ 领导链尾失联发 AI 延期安抚话术（去重）

**Files:**
- Modify: `src/models.rs:3072`（`AgentPrincipalEscalation` 新增 `last_holding_reply_ms`）
- Modify: `src/agent/escalation/mod.rs`（`scan_escalation_timeouts` 链尾 None 分支）
- Modify: `src/config.rs`（新增 `holding_reply_min_interval_hours`）
- Modify: `src/agent/escalation/ledger.rs`（新增更新 last_holding_reply_ms 的 helper）
- Test: `tests/principal_decision_channel.rs`（集成，`#[ignore]`）

**Interfaces:**
- Consumes: `next_decider_on_timeout`（policy.rs:95，返回 None 即链尾）、`push_allowed`、`fallback_holding_reply` 或新安抚话术
- Produces: `AgentPrincipalEscalation.last_holding_reply_ms: Option<i64>`；ledger helper `touch_last_holding_reply_ms`

**背景**：`policy.rs:105` `next_decider_on_timeout` 链尾返回 None，`scan_escalation_timeouts` 遇 None `continue`。客户只在最初 hold 时收到一次 `fallback_holding_reply`（mod.rs:141），领导一直不回 → 永久静默。

- [ ] **Step 1: 新增模型字段**

`src/models.rs` `AgentPrincipalEscalation`（:3072-3109），在 `knowledge_proposal_emitted`（:3101）后加：
```rust
    /// 链尾失联时最近一次给客户发安抚话术的时刻（epoch ms）。去重用：每
    /// holding_reply_min_interval_hours 最多发一条，防 worker tick 刷屏。`#[serde(default)]` 兼容旧文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_holding_reply_ms: Option<i64>,
```

- [ ] **Step 2: 新增配置项**

`src/config.rs`，仿现有 ask-human 相关配置加 `holding_reply_min_interval_hours: f64`，默认 `6.0`。env 名 `HOLDING_REPLY_MIN_INTERVAL_HOURS`。定位现有 config 结构体里 escalation/ask_human 相关字段附近插入，并在 from_env/默认构造处给默认值。

- [ ] **Step 3: 新增 ledger helper**

`src/agent/escalation/ledger.rs`（仿 `reassign_escalation` 模式），加：
```rust
/// 更新链尾安抚话术发送时刻（去重用）。仅 pending 可更新。
pub(crate) async fn touch_last_holding_reply_ms(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    now_ms: i64,
) -> AppResult<()> {
    state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            doc! { "$set": { "last_holding_reply_ms": now_ms } },
            None,
        )
        .await?;
    Ok(())
}
```

- [ ] **Step 4: 写集成测试（失败）**

`tests/principal_decision_channel.rs` 加（参照 §14.10e 双 tick 模式）：

```rust
/// §14.12（③链尾失联安抚去重）：单决策人链 [boss]，timeout 1h，boss 一直不回。
/// 第一次 scan（age 超时、next_decider 返回 None=链尾）→ 发一条安抚话术 + 记
/// last_holding_reply_ms。紧接第二次 scan（min_interval 未到）→ 不重复发（去重）。
/// 台账保持 pending。
#[tokio::test]
#[ignore]
async fn t_timeout_chain_tail_sends_holding_reply_once_within_interval() {
    let app = common::TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp.uri());

    set_ask_human_policy(
        &app,
        &AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "boss".into(), display_name: None }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: Some(1.0),
        },
    )
    .await;

    let two_hours_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 2 * 3600 * 1000);
    insert_pending_with_updated_at(&app, "T12", "boss", two_hours_ago).await;

    // 第一次 scan：链尾 → 发安抚 + 记 last_holding_reply_ms。
    wechatagent::agent::escalation::scan_escalation_timeouts(&state).await.expect("scan 1");
    let after1 = find_escalation(&app, "T12").await;
    assert_eq!(after1.status, "pending", "链尾安抚后台账仍 pending");
    assert!(after1.last_holding_reply_ms.is_some(), "应记录安抚发送时刻");
    // mcp mock 应收到 1 条发给客户的安抚（实现者按本文件 mock 计数方式断言）。

    // 第二次 scan（紧接，min_interval=6h 未到）：不重复发。
    wechatagent::agent::escalation::scan_escalation_timeouts(&state).await.expect("scan 2");
    let after2 = find_escalation(&app, "T12").await;
    assert_eq!(
        after2.last_holding_reply_ms, after1.last_holding_reply_ms,
        "min_interval 内不重复发安抚，时刻不变"
    );
    // mcp mock 客户安抚消息计数仍为 1（去重生效）。
}
```

- [ ] **Step 5: 运行测试确认失败**

Run: `cargo test --test principal_decision_channel t_timeout_chain_tail -- --ignored 2>&1 | tail -20`
Expected: FAIL。**本地无 Docker** → 跳过执行标注"待 CI 验证"，继续实现。

- [ ] **Step 6: 改 scan_escalation_timeouts 链尾分支**

读 `src/agent/escalation/mod.rs` `scan_escalation_timeouts` 里 `let Some(next) = next_decider_on_timeout(...) else { continue; };`（约 mod.rs:345）。把 `else { continue; }` 改为处理链尾安抚：

```rust
let Some(next) = next_decider_on_timeout(&policy, &entry.principal_wxid, age_hours) else {
    // 链尾：无更多决策人可改派。客户不能被永久晾着——发 AI 自主延期安抚话术，
    // 台账保持 pending 继续等领导。去重：每 holding_reply_min_interval_hours 最多一条。
    let min_interval_ms =
        (state.config.holding_reply_min_interval_hours * 3600.0 * 1000.0) as i64;
    let should_send = entry
        .last_holding_reply_ms
        .map_or(true, |last| now_ms - last >= min_interval_ms);
    if should_send {
        let _ = mcp::logged_call_for_account(
            state,
            &entry.account_id,
            "message_send_text",
            serde_json::json!({
                "recipient": &entry.contact_wxid,
                "content": chain_tail_holding_reply()
            }),
        )
        .await;
        if let Err(e) =
            ledger::touch_last_holding_reply_ms(state, &cfg.workspace_id, &entry.short_code, now_ms).await
        {
            tracing::warn!(short_code = %entry.short_code, error = ?e, "链尾安抚已发但更新 last_holding_reply_ms 失败");
        }
    }
    continue;
};
```

> 注：安抚发给**客户**（非领导推卡），故**不过** `push_allowed`（quiet_hours 是约束打扰领导的；客户安抚受 min_interval 去重约束即可——见 spec 未决细节 1 已定稿为"不过"）。

- [ ] **Step 7: 加链尾安抚话术**

`escalation/mod.rs`（与 `fallback_holding_reply` 同处）：
```rust
/// 链尾失联（无更多决策人）时给客户的延期安抚话术。AI 自治口吻，表达"还在跟进"，
/// 不自答越权点、不复述任何未授权内容、绝不出现"人工/转人工/接管"禁词。
fn chain_tail_holding_reply() -> String {
    "您这个问题我还在帮您核实确认，需要一点时间，麻烦您稍等下～一有结果我马上同步您。".to_string()
}
```

- [ ] **Step 8: 编译 + lib 测试 + lint**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed
Run: `bash "E:/yw/agiatme/工作项目/wechatagent/scripts/check-no-human-takeover.sh" 2>&1 | tail -3`
Expected: lint 0

- [ ] **Step 9: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/models.rs src/config.rs src/agent/escalation/mod.rs src/agent/escalation/ledger.rs tests/principal_decision_channel.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(escalation): 领导链尾失联发AI延期安抚话术+去重(③客户不被永久搁置)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## B 组 · 知识时效与缺口（①⑨）

### Task B1: ① 过期 verified 知识不再背书产品宣称

**Files:**
- Modify: `src/agent/guards.rs:308`（`is_verified` 加 now + valid_to 判定）、`:330`（`compute_verified_chunks` 透传 now）
- Modify: `src/agent/review/gates.rs:640` 和 `:687`（两个调用点传 now）
- Test: `src/agent/guards.rs` tests mod（纯函数真值表）

**Interfaces:**
- Produces: `is_verified(chunk: &OperationKnowledgeChunk, now: DateTime) -> bool`、`compute_verified_chunks(used_knowledge_ids, chunks, now) -> Vec<&OperationKnowledgeChunk>`
- Consumes: `OperationKnowledgeChunk.valid_to: Option<DateTime>`（models.rs:1202）

**背景**：`is_verified`（guards.rs:308）只判 `integrity_status=="verified"`，忽略 `valid_to`。过期知识仍通过 grounding 闸背书报价。`compute_verified_chunks` 有**两个**调用点（gates.rs:640 R5.4 硬闸 + gates.rs:687 漏判探针），改签名两处都要传 now。

- [ ] **Step 1: 写失败测试（纯函数真值表）**

`src/agent/guards.rs` 的 `#[cfg(test)] mod tests`（文件末尾），加（仿现有 chunk 构造 helper；若无则按 `OperationKnowledgeChunk` 字段最小构造）：

```rust
#[test]
fn is_verified_true_when_verified_and_no_valid_to() {
    let mut c = make_test_chunk();
    c.integrity_status = Some("verified".into());
    c.valid_to = None;
    let now = mongodb::bson::DateTime::now();
    assert!(is_verified(&c, now));
}

#[test]
fn is_verified_true_when_verified_and_not_expired() {
    let mut c = make_test_chunk();
    c.integrity_status = Some("verified".into());
    c.valid_to = Some(mongodb::bson::DateTime::from_millis(
        mongodb::bson::DateTime::now().timestamp_millis() + 24 * 3600 * 1000,
    ));
    let now = mongodb::bson::DateTime::now();
    assert!(is_verified(&c, now));
}

#[test]
fn is_verified_false_when_verified_but_expired() {
    let mut c = make_test_chunk();
    c.integrity_status = Some("verified".into());
    c.valid_to = Some(mongodb::bson::DateTime::from_millis(
        mongodb::bson::DateTime::now().timestamp_millis() - 24 * 3600 * 1000,
    ));
    let now = mongodb::bson::DateTime::now();
    assert!(!is_verified(&c, now), "过期 verified 知识不得背书");
}

#[test]
fn is_verified_false_when_draft() {
    let mut c = make_test_chunk();
    c.integrity_status = Some("draft".into());
    c.valid_to = None;
    let now = mongodb::bson::DateTime::now();
    assert!(!is_verified(&c, now));
}
```

> 实现者注：`make_test_chunk()` 若 guards.rs tests 已有则复用；否则新建一个最小构造（仅设必填字段 + integrity_status/valid_to）。参照 models.rs:5273 附近的 chunk 构造示例。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib is_verified 2>&1 | tail -15`
Expected: FAIL（签名不匹配，`is_verified` 现无 now 参数）

- [ ] **Step 3: 改 is_verified 签名 + 实现**

`guards.rs:308`：
```rust
/// R5.1：chunk 是否 `integrity_status == "verified"`（trim + 大小写不敏感）且未过期
/// （valid_to 为 None=永久有效，或 valid_to >= now）。过期知识不得背书产品声明。
pub(crate) fn is_verified(chunk: &OperationKnowledgeChunk, now: mongodb::bson::DateTime) -> bool {
    let status_ok = chunk
        .integrity_status
        .as_deref()
        .map(str::trim)
        .map(|s| s.eq_ignore_ascii_case("verified"))
        .unwrap_or(false);
    let not_expired = chunk.valid_to.map_or(true, |vt| vt >= now);
    status_ok && not_expired
}
```

- [ ] **Step 4: 改 compute_verified_chunks 透传 now**

`guards.rs:330`，签名加 `now`，:345 调用 `is_verified(chunk, now)`：
```rust
pub(crate) fn compute_verified_chunks<'a>(
    used_knowledge_ids: &[String],
    chunks: &'a [OperationKnowledgeChunk],
    now: mongodb::bson::DateTime,
) -> Vec<&'a OperationKnowledgeChunk> {
    // ... 不变 ...
        if !is_verified(chunk, now) {  // :345
            continue;
        }
    // ...
}
```

- [ ] **Step 5: 改两个调用点传 now**

`src/agent/review/gates.rs:640`（R5.4 硬闸）和 `:687`（漏判探针），各加 `now` 实参。now 来源：定位 `finalize_review_for_send` 或所在函数是否已有 now 变量；若无，在调用前 `let now = mongodb::bson::DateTime::now();`（两处可共用一个函数级 now）。
```rust
let verified_chunks = crate::agent::guards::compute_verified_chunks(
    &decision.used_knowledge_ids,
    knowledge_chunks,
    now,
);
```

- [ ] **Step 6: 运行测试 + 编译**

Run: `cargo test --lib is_verified 2>&1 | tail -15`
Expected: PASS（4 测试绿）
Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error（两个调用点签名匹配）

- [ ] **Step 7: lib 基线**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 8: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/guards.rs src/agent/review/gates.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(guards): is_verified加valid_to时效判定,过期知识不再背书产品宣称(①)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task B2: ⑨ 产品宣称被拦时写知识缺口信号

**Files:**
- Modify: `src/knowledge_wiki/gap_signals.rs`（`GapSignalCandidate` 加 pub 构造函数）
- Modify: `src/agent/review/gates.rs:644-668`（R5.4 block 分支写 gap_signal）
- Test: `tests/`（集成，验缺口落库 + 收件箱可见；或纯函数测 candidate 构造）

**Interfaces:**
- Consumes: `persist_recall_signal(db, workspace_id, candidate)`（gap_signals.rs:567）、`GapSignalCandidate`
- Produces: `GapSignalCandidate::recall_miss_from_product_block(...)`（pub 构造）

**背景**：R5.4 拦截点（gates.rs:644-668）只发瞬时 `product_claim_blocked` 事件，不写 gap_signal。`recall_miss` 只覆盖知识 agent 弃答，`suggestion` 因无命中 chunk 失效 → 运营不知缺什么。基础设施齐全（gap_signals 集合 + 收件箱 ask_human_inbox.rs:172 已展示 source=gap_signal）。

- [ ] **Step 1: 加 pub 构造函数**

`GapSignalCandidate::new`（gap_signals.rs:411）是私有的。加一个 pub 构造（同 impl 块）：
```rust
/// 产品宣称被 blocked_unverified_product_claim 拦截时构造的缺口信号：
/// kind=recall_miss（携带客户问句进 search_queries，给运营对话式补录线索）。
pub fn recall_miss_from_product_block(customer_query: String) -> Self {
    let title = format!("产品宣称缺 verified 知识背书：{}",
        customer_query.chars().take(40).collect::<String>());
    let mut c = Self::new("recall_miss", title, "medium", Vec::new(), None::<String>);
    if !customer_query.trim().is_empty() {
        c.search_queries.push(customer_query);
    }
    c
}
```

> severity 取值（"medium"）与 dedup_key 行为参照本文件现有 recall_miss 用法（`persist_recall_signal` 调用处 :567 上游）。实现者核对现有 recall_miss candidate 的 severity 惯例后对齐。

- [ ] **Step 2: 拿客户问句来源**

读 `gates.rs:644-668` block 分支的可用上下文。客户当前问句来源：定位 `finalize_review_for_send`（或所在函数）签名里是否有 inbound message / 客户文本。若有（如 `decision` 关联的 inbound 或函数入参里的客户消息文本）→ 用之。若 block 分支作用域拿不到客户问句 → 退而用 `decision.reply_text`（AI 拟回复，至少含主题）或报告 NEEDS_CONTEXT 说明该函数可见的客户文本来源。记录最终选用的 query 来源。

- [ ] **Step 3: 在 block 分支写 gap_signal**

`gates.rs:663`（`review.final_review_status = "blocked_unverified_product_claim"` 之前或之后、`return` 之前）。但注意 `finalize_review_for_send` 可能是同步函数 / 这段在 pending_events 收集模式下——**确认该函数是否 async + 能否拿到 `db`/`state`**：
- 若该函数 async 且持有 `state`/`db`：直接 `let _ = crate::knowledge_wiki::gap_signals::persist_recall_signal(&state.db, workspace_id, candidate).await;`（fail-soft，写失败不阻断）。
- 若该函数**非 async**（纯 finalize 逻辑，副作用经 `pending_events` 外抛）：不能在此 await。改为往 `pending_events` 加一个新事件类型，或在调用 `finalize_review_for_send` 的 async 上游（gateway）落 gap_signal。**实现者先确认 finalize_review_for_send 的 async 性与 db 可见性**，据此选落点；若需上游落，在 gateway 处理 `BlockedUnverifiedProductClaim` status 的分支调 persist_recall_signal。记录最终落点。

- [ ] **Step 4: 写测试**

依 Step 3 落点决定测试形态：
- 纯函数可测部分：`gap_signals.rs` tests 加 `recall_miss_from_product_block_carries_query`，断言构造的 candidate kind=recall_miss、search_queries 含客户问句、dedup_key 稳定。
- 集成部分（若落点在 async 上游，需 Docker）：验一次 blocked_unverified_product_claim run 后 `knowledge_gap_signals` 多一条 pending recall_miss。`#[ignore]`，本地无 Docker 标"待 CI"。

```rust
#[test]
fn recall_miss_from_product_block_carries_query() {
    let c = GapSignalCandidate::recall_miss_from_product_block("你们产品保几年质保".into());
    assert_eq!(c.kind, "recall_miss");
    assert!(c.search_queries.iter().any(|q| q.contains("质保")));
    assert!(!c.dedup_key().is_empty());
}
```

- [ ] **Step 5: 运行测试 + 编译**

Run: `cargo test --lib recall_miss_from_product_block 2>&1 | tail -10`
Expected: PASS
Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error

- [ ] **Step 6: lib 基线**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 7: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/knowledge_wiki/gap_signals.rs src/agent/review/gates.rs
# 若落点在 gateway 也 add src/agent/gateway.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(knowledge): 产品宣称被拦时写recall_miss缺口信号(⑨收件箱可见,闭环不再纯人工)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## C 组 · 决策写入可靠性（⑧⑥）

### Task C1: ⑧ customer_stage 写入过状态机合法性门

**Files:**
- Modify: `src/agent/gateway.rs:3068-3084`（stage 写入前加 check_state_transition）
- Test: `tests/`（集成验非法跳转被 skip）

**Interfaces:**
- Consumes: `guards::check_state_transition(domain_config, from, to) -> Option<String>`（guards.rs:156，pub）、`apply_agent_updates` 的 `domain_config: Option<&OperationDomainConfig>`（gateway.rs:2927）
- Produces: 非法 stage 跳转 fail-soft skip + `agent.stage_transition_rejected` 事件

**背景**：`gateway.rs:3076` `insert_domain_signal_values` 写 customer_stage 只做 `stage_changed` 判断（:3075），不过状态机校验。同源派生的 operation_state 已过 `check_state_transition`（:3149）。两者同属一套 canonical id 空间，stage 却能被 LLM 任意跳转 → 漂移。`apply_agent_updates` 已持有 `domain_config`（:2927）。

- [ ] **Step 1: 读现状确认变量**

读 `gateway.rs:3068-3084`。确认 `prev_stage`（:3069，旧 customer_stage）、`new_stage`（:3074，归一后新 stage）两变量在该作用域可用，`domain_config` 为函数参数（:2927）可用。

- [ ] **Step 2: 写集成测试（失败）**

集成测试（`#[ignore]`，需 Docker）：构造 contact 旧 stage = `new_contact`，LLM 决策给非法跳转（如直跳 `closed_won`，DEFAULT 销售域 closed_won 的 allowedFrom 不含 new_contact）→ 跑 gateway → 断言 customer_stage **未**被写成 closed_won（保持旧值）+ 有 `agent.stage_transition_rejected` 事件。放到现有集成测试文件（grep 写 customer_stage 的集成测试，如 c2_operation_state_derivation_e2e.rs 同域）。本地无 Docker 标"待 CI 验证"。

```rust
// 伪代码骨架（实现者按目标测试文件 TestApp 风格补全 fixture）：
// 1. seed domain_config（DEFAULT 销售域状态机，m006）
// 2. contact.domain_attributes.customer_stage = "new_contact"
// 3. mock LLM 决策 domain_signals.customer_stage = "closed_won"（非法跳）
// 4. 跑 apply_agent_updates / gateway
// 5. assert contact reload 后 customer_stage 仍 == "new_contact"
// 6. assert 有 kind="agent.stage_transition_rejected" 事件
```

- [ ] **Step 3: 加 stage 状态机校验**

`gateway.rs:3074` 取 `new_stage` 后、:3076 `insert_domain_signal_values` 前插入校验。非法则从 `signals_decision.domain_signals` 移除 customer_stage（不写入），记事件：

```rust
let new_stage = signals_decision.domain_signals.get_str("customer_stage").ok();
// ⑧：customer_stage 与 operation_state 同属一套 canonical id 空间（m006），后者已过
// check_state_transition（:3149 起 C2 派生），stage 也须过同一状态机，否则 LLM 可任意
// 跳转致两字段漂移。非法跳转 fail-soft：移除 stage 不写（保持旧值）+ 记审计，与
// operation_state_transition_rejected 对称。reply 已发，不阻断。
if let Some(to_stage) = new_stage {
    if prev_stage != Some(to_stage) {
        if let Some(reason) =
            crate::agent::guards::check_state_transition(domain_config, prev_stage, to_stage)
        {
            signals_decision.domain_signals.remove("customer_stage");
            let _ = write_event_for_account(
                &state,
                &contact.account_id,
                Some(&contact.wxid),
                "agent.stage_transition_rejected",
                "rejected",
                &format!("customer_stage 非法跳转被拒：{}", reason),
                Some(doc! { "from": prev_stage.unwrap_or(""), "to": to_stage, "reason": reason }),
            )
            .await;
        }
    }
}
```

> 时序：这段必须在 :3074 取 new_stage **之后**、:3075 `stage_changed` 计算与 :3076 `insert_domain_signal_values` **之前**插入。插入后重新计算 `stage_changed`（stage 可能已被 remove）。实现者据现场调整 :3074-3080 顺序，确保 remove 生效后 stage_changed/insert 读到移除后的 domain_signals。

- [ ] **Step 4: 编译 + lib 基线**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/gateway.rs tests/
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(gateway): customer_stage写入过状态机校验,非法跳转fail-soft skip(⑧与operation_state对称)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task C2: ⑥ 承诺兑现 prompt 强化 + 观测事件

**Files:**
- Modify: `src/prompts.rs`（decision prompt 强化承诺必填 + bump PROMPT_PACK_VERSION）
- Modify: `src/agent/gateway.rs:3116`（commitment 字段空但 reply 含时间承诺特征 → 观测事件）
- Test: 纯函数测时间承诺特征检测

**Interfaces:**
- Produces: `agent.commitment_field_missing` 观测事件（不拦）；prompt 文案强化
- Consumes: `decision.last_commitment`（gateway.rs:3116）、`decision.reply_text`

**背景**：⑥ 真缺口 = LLM 在 reply_text 口头承诺却没填 last_commitment 字段 → 无 follow-up。"reply 是否做了承诺"是语义判断，确定性护栏做不到。降级为 prompt 强化（主）+ 观测事件（辅，不硬拦）。

- [ ] **Step 1: prompt 强化**

读 `src/prompts.rs` decision prompt（reply Agent 主决策 prompt）。找到 commitment/last_commitment 字段说明处，强化文案：凡向客户做了任何时间相关承诺（"明天发您""下周回复"等），必须同时填 `last_commitment`（及可选结构化 `commitment.dueAt`），否则系统无法在到期时提醒跟进。

- [ ] **Step 2: bump PROMPT_PACK_VERSION**

`src/prompts.rs` 找 `PROMPT_PACK_VERSION` 常量，版本号 +1。改 prompt 必 bump（全局约束）。

- [ ] **Step 3: 写纯函数测试（时间承诺特征检测）**

```rust
#[test]
fn reply_has_time_commitment_feature_detects_relative_dates() {
    assert!(reply_has_time_commitment_feature("我明天发您资料"));
    assert!(reply_has_time_commitment_feature("下周给您答复"));
}
#[test]
fn reply_has_time_commitment_feature_negative() {
    assert!(!reply_has_time_commitment_feature("好的，我了解了"));
}
```

- [ ] **Step 4: 实现观测函数 + 接线**

纯函数（弱启发，仅观测）：
```rust
/// 弱启发：reply 是否含"时间相关承诺"特征。仅用于 ⑥ 观测覆盖率，不进任何门、
/// 不改变发送判定。非红线护栏（语义判断交 LLM + prompt）。
fn reply_has_time_commitment_feature(reply: &str) -> bool {
    const MARKERS: [&str; 8] = ["明天", "后天", "下周", "下个月", "稍后", "晚点", "回头", "马上"];
    MARKERS.iter().any(|m| reply.contains(m))
}
```

接线 `gateway.rs:3116`：在 `if let Some(value) = non_empty_option(&decision.last_commitment)` 块加 else 观测分支：
```rust
} else if reply_has_time_commitment_feature(&decision.reply_text) {
    // ⑥观测：reply 像做了时间承诺但 LLM 没填 commitment 字段 → 无 follow-up。
    // 仅观测 prompt 强化是否生效，不阻断、不改写。
    let _ = write_event_for_account(
        &state, &contact.account_id, Some(&contact.wxid),
        "agent.commitment_field_missing", "observed",
        "回复疑似含时间承诺但未填 commitment 字段（观测，未拦截）", None,
    ).await;
}
```

> 观测函数是弱启发，可能误报/漏报——这正是它**只观测不拦**的原因。运营据事件频率判断 prompt 强化是否足够。

- [ ] **Step 5: 测试 + 编译 + lib 基线**

Run: `cargo test --lib reply_has_time_commitment 2>&1 | tail -10`
Expected: PASS
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 6: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/prompts.rs src/agent/gateway.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(commitment): prompt强化承诺必填+commitment缺失观测事件(⑥,语义不硬拦)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## D 组 · 跟进节奏（⑦）

### Task D1: ⑦ 唤醒任务 per-contact 确定性 jitter

**Files:**
- Modify: `src/agent/quiet_hours.rs:55`（`next_wake_utc_ms` 加 jitter_ms）、`:78`（`next_wake_at` 加 jitter_seed）
- Modify: `src/config.rs`（新增 `wake_jitter_max_seconds`）
- Modify: `src/agent/gateway.rs:686`、`:1551`、`src/webhooks.rs:661`（三处调用传 contact.wxid）
- Test: `src/agent/quiet_hours.rs` tests（纯函数确定性 + 分布）

**Interfaces:**
- Produces: `next_wake_utc_ms(now_utc_ms, end, tz_offset_hours, jitter_ms) -> i64`、`next_wake_at(end, tz_offset_hours, jitter_seed: &str, jitter_max_seconds: u32) -> DateTime`、`jitter_ms_for_seed(seed: &str, max_seconds: u32) -> i64`
- Consumes: contact.wxid（三处调用点均可得）

**背景**：静默期延后的发送全部重排到 `next_wake_at`（quiet_hours.rs:78），同一 workspace 多客户唤醒时刻全 = 次日 quiet_hours_end（如 8:00）→ 整点齐发。三处调用：gateway.rs:686、:1551、webhooks.rs:661。

- [ ] **Step 1: 写失败测试（纯函数）**

`src/agent/quiet_hours.rs` tests mod（现有 :180+ 测试区）加：

```rust
#[test]
fn jitter_is_deterministic_per_seed() {
    let now = 1_700_000_000_000;
    let a = next_wake_utc_ms(now, 8, 8, jitter_ms_for_seed("wxid_alice", 900));
    let b = next_wake_utc_ms(now, 8, 8, jitter_ms_for_seed("wxid_alice", 900));
    assert_eq!(a, b);
}
#[test]
fn jitter_differs_across_seeds() {
    assert_ne!(jitter_ms_for_seed("wxid_alice", 900), jitter_ms_for_seed("wxid_bob", 900));
}
#[test]
fn jitter_within_bounds() {
    for seed in ["a", "b", "c", "xyz", "wxid_123456"] {
        let j = jitter_ms_for_seed(seed, 900);
        assert!(j >= 0 && j <= 900 * 1000, "jitter 越界: {}", j);
    }
}
#[test]
fn jitter_zero_max_is_noop() {
    assert_eq!(jitter_ms_for_seed("anything", 0), 0);
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib jitter 2>&1 | tail -15`
Expected: FAIL（`jitter_ms_for_seed` / 新签名未定义）

- [ ] **Step 3: 实现 jitter 纯函数 + 改签名**

`quiet_hours.rs` 加纯函数：
```rust
/// 从 contact 标识派生确定性 jitter（毫秒），落在 [0, max_seconds*1000]。同一 seed
/// 恒定（可复现、可测），不同 contact 散开，把整点唤醒打散避免齐发。max_seconds=0 → 恒 0。
pub(crate) fn jitter_ms_for_seed(seed: &str, max_seconds: u32) -> i64 {
    if max_seconds == 0 {
        return 0;
    }
    let mut h: u64 = 0xcbf29ce484222325; // FNV-1a，避免 DefaultHasher 跨版本不稳定
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let max_ms = (max_seconds as u64) * 1000;
    (h % (max_ms + 1)) as i64
}
```

改 `next_wake_utc_ms`（:55）加 `jitter_ms`：
```rust
pub(crate) fn next_wake_utc_ms(now_utc_ms: i64, end: u32, tz_offset_hours: i32, jitter_ms: i64) -> i64 {
    // ... 现有计算不变 ...
    (local_target - off) + jitter_ms  // 末尾加 jitter
}
```

改 `next_wake_at`（:78）加 `jitter_seed` + `jitter_max_seconds`：
```rust
pub(crate) fn next_wake_at(end: u32, tz_offset_hours: i32, jitter_seed: &str, jitter_max_seconds: u32) -> mongodb::bson::DateTime {
    let jitter = jitter_ms_for_seed(jitter_seed, jitter_max_seconds);
    mongodb::bson::DateTime::from_millis(next_wake_utc_ms(
        Utc::now().timestamp_millis(), end, tz_offset_hours, jitter,
    ))
}
```

> 现有测试（:184/:192/:200/:209 调 `next_wake_utc_ms(now, 8, 8)`）签名变了 → 补 `, 0`（jitter=0，保持原断言）。增量叠加不删旧维度。

- [ ] **Step 4: 新增配置 wake_jitter_max_seconds**

`src/config.rs` 加 `wake_jitter_max_seconds: u32`，默认 `900`（15min）。env `WAKE_JITTER_MAX_SECONDS`。

- [ ] **Step 5: 改三处调用点**

- `gateway.rs:686`：`next_wake_at(runtime.quiet_hours_end, runtime.quiet_hours_tz_offset_hours, &contact.wxid, state.config.wake_jitter_max_seconds)`
- `gateway.rs:1551`：同上（该作用域有 contact）
- `webhooks.rs:661`：`next_wake_at(wake_hour, tz_offset_hours, &<contact wxid 变量>, state.config.wake_jitter_max_seconds)`（实现者确认 webhooks.rs:661 作用域内 contact 标识变量名）

- [ ] **Step 6: 测试 + 编译 + lib 基线**

Run: `cargo test --lib jitter 2>&1 | tail -15`
Expected: PASS
Run: `cargo test --lib quiet 2>&1 | tail -10`
Expected: PASS（现有 quiet_hours 测试补 jitter=0 后不回归）
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 7: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/quiet_hours.rs src/config.rs src/agent/gateway.rs src/webhooks.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(quiet_hours): 唤醒任务加per-contact确定性jitter,避免整点齐发(⑦)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## E 组 · 账号健康防护（⑪④）

### Task E1: ⑪ webhook Offline 落库建状态源 + 发送前 defer

**Files:**
- Modify: `src/webhooks.rs:330-338`（Offline 事件落库 online=false）
- Modify: `src/agent/outbox_dispatcher.rs`（发送前查 online，掉线 defer/reschedule）
- Test: `tests/`（集成）

**Interfaces:**
- Consumes: `WechatAccount.online`（models.rs:70）
- Produces: webhook Offline → `online=false` 落库；dispatcher 发送前 gate

**背景**：`webhooks.rs:330-338` 收到 `TypeName=Offline` 直接 ack 丢弃（仅 `POST /accounts/sync` 手动刷新 online，无定时器）；`send_outbound_message`（gateway.rs:2209）发送前不查 online。重试有 3 次上限+几何退避（outbox.rs:457），非风暴，但掉线期间盲发。

- [ ] **Step 1: 确认 dispatcher 结构 + defer 落点**

读 `src/agent/outbox_dispatcher.rs`（发送主循环 + schedule_retry_or_terminal :588-639）。确认：①dispatcher 调 MCP 发送前的位置、能拿到 account_id；②是否有 reschedule/defer 机制（区别于 retry——defer 是"账号掉线，稍后整体重试"，不该消耗 max_attempts）。

> **落点判断**：⑪ defer 应加在 **dispatcher 取出 pending outbox、发送前**（那里能 reschedule next_retry_at 而不增 attempt），而非底层 `send_outbound_message`（无 reschedule 语义）。实现者据 dispatcher 实际结构确认落点；若 dispatcher 无"不增 attempt 的 defer"原语，新增一个（把 next_retry_at 推后但 attempt 不变），记录方案。

- [ ] **Step 2: webhook Offline 落库**

`src/webhooks.rs:330-338`，把直接丢弃 Offline 改为落库 `online=false`：
```rust
// 收到账号离线事件：落库 online=false（建状态源），供发送前 gate 判断。此前直接 ack
// 丢弃 → online 字段长期陈旧、掉线期间盲发。（上线事件若 payload 有，对称落 online=true）
state.db.accounts().update_one(
    doc! { /* 按 appId/account 定位，参照本文件账号解析方式 */ },
    doc! { "$set": { "online": false, "last_sync_at": DateTime::now() } },
    None,
).await?;  // 或 fail-soft let _ = ，按本文件 webhook 错误处理惯例
```
> 实现者确认 webhooks.rs:330 处账号定位字段（appId → account）的现有解析方式，对齐。

- [ ] **Step 3: dispatcher 发送前 gate**

在 Step 1 确认的落点，发送前查 account.online：
```rust
// ⑪：账号掉线时不盲发——defer（推后 next_retry_at，不增 attempt），online 恢复后照常发。
let account = state.db.accounts().find_one(doc! { "account_id": &entry.account_id }, None).await?;
if let Some(acc) = &account {
    if !acc.online {
        // defer：推后重试、记事件，不消耗 attempt、不 terminal。
        let _ = write_event_for_account(state, &entry.account_id, Some(&entry.contact_wxid),
            "agent.send_deferred_account_offline", "deferred",
            "账号离线，本条发送推迟（不盲发）", None).await;
        continue; // 或 defer 后跳过本条
    }
}
```
> account 定位字段（account_id vs appId）以 WechatAccount 实际唯一键为准（实现者核对 models.rs WechatAccount + accounts() 查询惯例）。

- [ ] **Step 4: 写集成测试**

`#[ignore]` 集成：① webhook 发 Offline → 账号 online 变 false；② online=false 时 dispatcher 不调 MCP 发送（defer），online=true 时正常发。本地无 Docker 标"待 CI"。

- [ ] **Step 5: 编译 + lib 基线**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 6: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/webhooks.rs src/agent/outbox_dispatcher.rs tests/
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(account): webhook Offline落库建状态源+发送前defer掉线不盲发(⑪)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task E2: ④ 账号级发送软上限告警

**Files:**
- Modify: `src/config.rs`（新增 `account_daily_send_soft_cap`）
- Modify: `src/agent/gateway.rs` 或 outbox_dispatcher（发送前查当日总量，超则记 warning 事件）
- Test: 集成测事件

**Interfaces:**
- Consumes: `OutboxEntry`（models.rs:2476：account_id :2480 / status :2500 / sent_at :2515）
- Produces: `agent.account_daily_send_soft_cap_exceeded` warning 事件（不拦）

**背景**：`gateway.rs` `daily_touch_count`（:2631 附近）以 contact_wxid 过滤 = 仅 per-contact。无账号级总量限制。用户选"软上限只告警"（不拦不排队）。

- [ ] **Step 1: 新增配置**

`src/config.rs` 加 `account_daily_send_soft_cap: i64`，默认保守高值（建议 `500`，仅告警用，不拦）。env `ACCOUNT_DAILY_SEND_SOFT_CAP`。

- [ ] **Step 2: 实现当日总量查询 helper**

查 `agent_send_outbox`（`OutboxEntry`）当日 `status=sent` 计数。仿现有 daily_touch_count 查询模式，但过滤改 account 级（不带 contact_wxid）：
```rust
/// 账号当日已发送总量（status=sent，sent_at >= 当日起点）。④ 软上限告警用。
async fn account_daily_sent_count(state: &AppState, account_id: &str, since_ms: i64) -> AppResult<i64> {
    let count = state.db.agent_send_outbox()
        .count_documents(
            doc! {
                "account_id": account_id,
                "status": "sent",
                "sent_at": { "$gte": DateTime::from_millis(since_ms) },
            },
            None,
        )
        .await?;
    Ok(count as i64)
}
```
> 集合 accessor 名以 Database 实际方法为准（grep outbox accessor；CLAUDE.md 称集合 agent_send_outbox，类型 OutboxEntry）。

- [ ] **Step 3: 发送前查 + 记事件（不拦）**

在发送主路径（send_outbound_message 或 dispatcher 发送前），查当日总量，超软上限记 warning 事件，**不拦不 return**：
```rust
let day_start_ms = /* 当日 0 点 ms（对齐现有日界惯例，可用 UTC 或 quiet_hours tz）*/;
if let Ok(sent) = account_daily_sent_count(state, &contact.account_id, day_start_ms).await {
    if sent >= state.config.account_daily_send_soft_cap {
        let _ = write_event_for_account(state, &contact.account_id, Some(&contact.wxid),
            "agent.account_daily_send_soft_cap_exceeded", "warning",
            &format!("账号当日发送量 {} 已达软上限 {}（仅告警，未拦截）", sent, state.config.account_daily_send_soft_cap),
            Some(doc! { "sent": sent, "cap": state.config.account_daily_send_soft_cap })).await;
    }
}
```

- [ ] **Step 4: 写测试**

集成：account 发送量超 cap → 有 warning 事件；未超 → 无。`#[ignore]` 本地无 Docker 标"待 CI"。

- [ ] **Step 5: 编译 + lib 基线**

Run: `cargo check --lib 2>&1 | tail -8`
Expected: 0 error
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 6: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/config.rs src/agent/gateway.rs tests/
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "fix(account): 账号级发送软上限告警(④只告警不拦,防封号观测先行)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## F 组 · 多模态入站地基（⑤，代码地基 + 外部依赖打桩）

### Task F1: 入站 msgType 解析 + media_ref 落库

**Files:**
- Modify: `src/webhooks.rs:464-474`（解析 msgType，落 msg_type + media_ref，不再写死 None）
- Test: 纯函数测 msgType 解析

**Interfaces:**
- Produces: 入站 `conversation_messages` 带 `msg_type` + `media_ref`（非文本消息可识别）
- Consumes: webhook payload 的消息类型字段

**背景**：`webhooks.rs:472-474` 落库时 `msg_type:None`/`media_ref:None` 写死，完全不解析 msgType → 非文本入站被当空文本硬答。

- [ ] **Step 1: 确认 payload 消息类型字段**

读 `src/webhooks.rs:440-490`（inbound 解析区）。确认 webhook payload 里消息类型字段名（如 `msgType`/`MsgType`/数字 type 码）+ 媒体引用字段（图片 url / 文件 id / 语音 path 等）的位置。记录字段路径。

- [ ] **Step 2: 写纯函数测试（msgType 解析）**

把 raw payload 类型码 → 归一化 msg_type 枚举/字符串的映射做成纯函数 + 测试：
```rust
#[test]
fn classify_inbound_msg_type_maps_known() {
    assert_eq!(classify_inbound_msg_type("text"), "text");
    assert_eq!(classify_inbound_msg_type("image"), "image");
    assert_eq!(classify_inbound_msg_type("voice"), "voice");
    // 未知类型归 "unknown"（不崩、不当 text）
    assert_eq!(classify_inbound_msg_type("某新类型"), "unknown");
}
```
> 实际类型码以 Step 1 确认的 payload 格式为准（可能是数字码 1/3/34/... 微信类型）。测试用例对齐真实码值。

- [ ] **Step 3: 实现解析 + 落库**

实现 `classify_inbound_msg_type` 纯函数，webhooks.rs:472 落库改为写真实 msg_type + media_ref（从 payload 提取媒体引用）：
```rust
// 不再写死 None：解析 msgType，非文本消息落 media_ref（媒体引用，供后续理解链路取内容）。
msg_type: Some(classify_inbound_msg_type(raw_type)),
media_ref: extract_media_ref(&payload),  // 图片url/文件id/语音path，text 消息为 None
```

- [ ] **Step 4: 测试 + 编译 + lib 基线**

Run: `cargo test --lib classify_inbound_msg_type 2>&1 | tail -10`
Expected: PASS
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed

- [ ] **Step 5: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/webhooks.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "feat(webhook): 入站解析msgType+落media_ref,非文本消息可识别(⑤地基1)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task F2: 图片理解封装（复用 vision）+ 媒体下载打桩 + 非文本过渡话术

**Files:**
- Create/Modify: `src/agent/`（图片理解封装，复用 llm.rs:525 / generate_json_with_image）
- Modify: `src/mcp.rs` 或新接口模块（媒体下载 trait 打桩）
- Modify: `src/agent/gateway.rs` 或 webhooks 触发链（非文本消息走过渡话术）
- Test: 纯函数 / 桩行为测试

**Interfaces:**
- Consumes: `llm.rs` `generate_json_with_image`（:882）、import.rs:560-608 VisionProvider 选择逻辑
- Produces: `describe_inbound_image(...)` 封装；`fetch_inbound_media(...)` 打桩接口；非文本过渡话术

**背景**：vision 调用底层已存在（llm.rs:525 OpenAI vision 消息体 + DB llm_provider_configs.supports_vision），知识库导入 import_apply_image 在用。但 MCP 当前无"下载入站媒体"tool（仓内零调用），语音 ASR 零能力。本波做图片理解封装 + 媒体下载打桩 + 非文本过渡话术；媒体下载真实接通、ASR 待独立立项。

- [ ] **Step 1: 媒体下载接口打桩**

定义"拉取入站媒体内容"的接口（trait 或 async fn），实现打桩返回明确的"未接通"错误/None：
```rust
/// 拉取入站媒体（图片/语音/文件）的二进制内容。当前 MCP server 媒体下载 tool 未确认，
/// 打桩返回 NotConfigured。实现前必打 server tools/list 确认能力（参照 referral-card
/// message_send_namecard：仓内零书面依据不能凭空实现）。
async fn fetch_inbound_media(_state: &AppState, _media_ref: &str) -> AppResult<Option<MediaContent>> {
    // TODO(⑤完整feature): server tools/list 确认媒体下载 tool 后接通。
    Ok(None) // 打桩：未接通
}
```

- [ ] **Step 2: 图片理解封装（复用现有 vision）**

封装一个"理解客户图片→文字描述"调用，复用 `generate_json_with_image`（llm.rs:882）+ import.rs:560-608 的 VisionProvider 选择逻辑（选 supports_vision 的模型）。拿到图片 base64 即可调；media 下载未接通时此封装暂不会被实际触发（依赖 Step 1）。

```rust
/// 理解客户发来的图片，返回文字描述供决策链路使用。复用知识库导入的 vision 能力
/// （generate_json_with_image + supports_vision 模型选择）。
async fn describe_inbound_image(state: &AppState, image_base64: &str, mime: &str) -> AppResult<String> {
    // 复用 import.rs VisionProvider 选择：优先 active 文字模型若 supports_vision，
    // 否则查 workspace supports_vision 副模型。调 generate_json_with_image 取描述。
    // ... 实现复用 ...
}
```

- [ ] **Step 3: 非文本消息过渡话术接线**

在入站触发链路（webhooks → gateway），当消息为非文本（msg_type != "text"）且媒体理解未接通（图片下载桩返回 None / 语音 / 链接）时，AI 发自然过渡话术请客户文字补充，**不硬答空串/原始 XML、不崩**：
```rust
// 非文本消息且理解链路未接通：发过渡话术请客户文字补充关键信息（不硬答、不崩）。
// AI 自治口吻，绝不"人工/转人工/接管"。
fn non_text_transition_reply(msg_type: &str) -> String {
    match msg_type {
        "image" => "我看到您发的图片啦，方便简单文字描述下您想了解什么吗？这样我能更准确帮您～".to_string(),
        "voice" => "收到您的语音啦，方便文字打一下吗？我好第一时间帮您看～".to_string(),
        _ => "收到～方便文字简单说下您的需求吗？我好帮您处理～".to_string(),
    }
}
```
> 接线位置：webhook 处理非文本入站时，在进决策链路前判断。实现者确认接线点（webhook 触发 gateway 处 / gateway 入口），保证非文本消息有兜底回复。

- [ ] **Step 4: 写测试**

纯函数：`non_text_transition_reply` 各类型返回非空话术且无禁词；`fetch_inbound_media` 桩返回 None。图片理解封装若依赖 LLM 不做单测（标注集成/真模型待 server 接通）。

```rust
#[test]
fn non_text_transition_reply_covers_types_no_forbidden_words() {
    for t in ["image", "voice", "link", "miniprogram", "unknown"] {
        let r = non_text_transition_reply(t);
        assert!(!r.is_empty());
        // 无禁词（人工/接管等）由 check-no-human-takeover lint 兜底
    }
}
```

- [ ] **Step 5: 测试 + 编译 + lib 基线 + lint**

Run: `cargo test --lib non_text_transition 2>&1 | tail -10`
Expected: PASS
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed, 0 failed
Run: `bash "E:/yw/agiatme/工作项目/wechatagent/scripts/check-no-human-takeover.sh" 2>&1 | tail -3`
Expected: lint 0

- [ ] **Step 6: Commit**

```bash
git -C "E:/yw/agiatme/工作项目/wechatagent" add src/agent/ src/mcp.rs src/webhooks.rs
git -C "E:/yw/agiatme/工作项目/wechatagent" commit -m "feat(multimodal): 图片理解封装(复用vision)+媒体下载打桩+非文本过渡话术(⑤地基2)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 收尾：全量验证 + 自审

完成全部任务后（subagent-driven-development 的最终 whole-branch review 前）：

- [ ] **基线门**：`cargo test --lib 2>&1 | tail -5`（≥ 350/0）+ 四 PBT（`cargo test --test state_transition_pbt`、`memory_card_invariants`、`wiki_chunk_revision_pbt`、`llm_retry_jitter`，累计 ≥ 33/0）。
- [ ] **lint 门**：`bash scripts/check-no-human-takeover.sh`（0）——A 组与 F 组话术全过。
- [ ] **配置文档**：三个新配置项（holding_reply_min_interval_hours / wake_jitter_max_seconds / account_daily_send_soft_cap）加进 `.env.example`，注明默认值与用途。
- [ ] **集成测试待 CI**：所有 `#[ignore]` 集成测试（principal_decision_channel 新增、stage transition、Offline defer、软上限）本地无 Docker → 推送后由 CI integration job 验证。
- [ ] **磁盘纪律**：本地若 `os error 112` → 先 `rm -rf target/debug/incremental`。

## 提交与推送

- 全程精确 `git add`，绝不 `git add -A`/`.`，排除并行会话产物（见 Global Constraints）。
- **推送需用户显式批准**（CLAUDE.md + git_safety）。当前分支 `feat/ask-human-phase1` 是共享分支，注意并行会话交错提交——每次提交前 `git rev-parse <sha>^` 核对真实 parent。
- 子代理 model:opus；回复中文。

## Self-Review（writing-plans 自检结果）

- **Spec 覆盖**：spec 六组 11 条全部映射到任务（A1=⑩/A2=②/A3=③/B1=①/B2=⑨/C1=⑧/C2=⑥/D1=⑦/E1=⑪/E2=④/F1+F2=⑤）。Minor（已拒绝客户不唤醒等）未单列任务——spec 标"writing-plans 阶段评估，倾向附带"，留待执行中按需附加，不阻塞主线。
- **占位符**：三个配置默认值已给具体建议值（6.0 / 900 / 500）。集成测试 fixture 因依赖现场 helper，标注"参照现有 §14.10 系列"+ 给出断言骨架，非空泛 TBD。F 组媒体下载/ASR 明确打桩（非占位，是有意延迟）。
- **类型一致**：`is_verified(chunk, now)` / `compute_verified_chunks(..., now)` 两调用点（gates.rs:640/687）一致传 now；`check_state_transition(domain_config, from, to)` 签名与 guards.rs:156 一致；`next_wake_at` 新签名三处调用点一致。
- **已知现场确认点**（实现者需 grep/read 确认，已在对应 Step 标注）：A1 relay substance 可见性、A2 clear_awaiting 签名、B2 finalize async 性与落点、E1 dispatcher defer 原语、E2 outbox accessor 名、F1 payload 类型字段、F2 接线点。这些是"现场核对"而非"设计缺口"——签名/落点以现场为准，逻辑已定。

