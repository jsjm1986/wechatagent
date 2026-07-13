# P3 家族⑥ provider/MCP 配置加固 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** KD-10（activate_provider 先 swap 成功再写 DB active，消 DB↔运行时假失败）+ KD-09（openai 形态 base_url 缺 /v1 软 warning，前置提醒不阻断）+ KE-06（sync_accounts mcp_base_url 移 $setOnInsert，与 mcp_api_key 对称保护手配值）。

**Architecture:** 三条独立配置加固。KD-10/KD-09 同在 `src/routes/llm_providers.rs`（Task 1）：KD-10 把 swap_registry 提到两个 DB update 之前（swap 无 DB 副作用，亲验）；KD-09 新增纯函数 `base_url_v1_warning` + create/update 塞可选 warning 返回体。KE-06 在 `src/routes/accounts.rs`（Task 2）：sync_accounts upsert 把 mcp_base_url 从 $set 移 $setOnInsert。不碰发送/agent 主逻辑。

**Tech Stack:** Rust 2021，Axum。KD-09 是纯函数 → lib 单测（本地可跑）；KD-10/KE-06 是 DB 交互语义调整 → 靠终审亲验代码 + 现有集成测无回归（改动直白）。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-14-p3-family6-provider-mcp-config-design.md`（已获批 commit fe2a931）。所有行号亲验于分支 `fix/p3-family6-provider-mcp-config`（基于 origin/main d027fb9 含 #198）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆（行号可能漂移，以 Read 到的真实代码为准）。
- **KD-10**：swap_registry（llm_providers.rs:552）亲验为纯 client 构造 + reg.swap 原子替换、无 DB 副作用，故可提到 DB update 之前。只调换 activate_provider（:305-346）内 swap 与两个 update 的顺序，不改 swap_registry 本身、不改 update 的 filter/doc。
- **KD-09**：只对 openai 形态（LlmFormat::Openai）校验；anthropic 形态拼 {base_url}/v1/messages 自带 /v1，**不校验**（返 None）。软 warning **不阻断保存**（各家路径不一，hard block 误伤 Azure/代理合法非 /v1 端点）。warning=None 时返回体**不出现** warning 键（避免 "warning":null 误导前端）。
- **KE-06**：只把 mcp_base_url 从 $set 移到 $setOnInsert（与 mcp_api_key :158 并列），不改其它字段位置、不改 update_account_mcp_key。
- 反过拟合红线：真 bug 才修；KD-09 纯函数单测驱动真函数、真哨兵（回退去 openai 判断或 /v1 判断即变红）。绝不为过测试改业务逻辑。
- check-no-human-takeover lint 扫 src/routes/ 新增行禁词（人工接管/接管/人工/takeover/hand-off 等）——warning 文案/注释用中性词（服务商/路径/配置/账号/手配），无禁词。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用）。
- 本地若撞 LNK1318 PDB（Windows-only 非代码错），`cargo check --lib` / `cargo check --tests` 足够验证编译。

## File Structure

- `src/routes/llm_providers.rs`：KD-10 activate_provider（:305-346）swap 提前；KD-09 新增 `base_url_v1_warning` 纯函数 + create_provider（:148-201）/update_provider（:203-271）塞 warning；mod tests（:584+）append KD-09 单测。
- `src/routes/accounts.rs`：KE-06 sync_accounts（:140-162）mcp_base_url 移 $setOnInsert。

Task 1 = KD-10 + KD-09（同文件 llm_providers.rs，一实现者一次读懂）。Task 2 = KE-06（accounts.rs 单键移位，独立）。两 task 独立。

---

## Task 1: KD-10 + KD-09 —— activate 先 swap + base_url 软 warning（llm_providers.rs）

**Files:**
- Modify: `src/routes/llm_providers.rs:305-346`（activate_provider swap 提前，KD-10）
- Modify: `src/routes/llm_providers.rs`（新增 base_url_v1_warning 纯函数，KD-09）
- Modify: `src/routes/llm_providers.rs:200`（create_provider 返回体塞 warning）
- Modify: `src/routes/llm_providers.rs:270`（update_provider 返回体塞 warning）
- Modify: `src/routes/llm_providers.rs:584+`（mod tests append 纯函数单测）

**Interfaces:**
- Consumes: `LlmFormat`（llm.rs，create/update 已 `LlmFormat::parse` :154/:210）；`swap_registry(reg, cfg)`（:552，不改）；`LlmProviderConfig`（cfg.base_url 已 trim）。
- Produces: `fn base_url_v1_warning(fmt: LlmFormat, base_url: &str) -> Option<String>`（新私有纯函数）。

- [ ] **Step 1: 亲验 activate_provider / create / update / swap_registry 真实现状**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn activate_provider\|fn create_provider\|fn update_provider\|fn swap_registry\|LlmFormat::parse\|trim_end_matches\|json!({ \"item\"\|swap_registry(reg" src/routes/llm_providers.rs`
Expected: 确认 activate_provider（约 :305）的 update_many→update_one→swap 顺序、create（约 :148 返回 :200）/update（约 :203 返回 :270）的 LlmFormat::parse 位置与返回体、swap_registry（约 :552）签名。**实现者 Read 这几段全貌**后再改（行号以真实为准）。

- [ ] **Step 2: KD-10 —— activate_provider swap 提到两个 update 之前**

把 `src/routes/llm_providers.rs` 的 activate_provider 里（find target 之后、现 :321-342 的 update_many→update_one→swap 段）：

```rust
    let now = DateTime::now();
    state
        .db
        .llm_provider_configs()
        .update_many(
            doc! { "workspaceId": &workspace_id, "isActive": true, "providerId": { "$ne": &provider_id } },
            doc! { "$set": { "isActive": false, "updatedAt": now } },
            None,
        )
        .await?;
    state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        )
        .await?;
    if let Some(reg) = &state.llm_registry {
        swap_registry(reg, &target).await?;
    }
```

调整为（swap 先，成功才写 DB；DateTime::now() 移到 swap 之后）：

```rust
    // KD-10：先 swap（成功才写 DB active），让"返 Err ⟺ DB 未改"一致。swap_registry 是纯
    // client 构造 + 原子替换、无 DB 副作用，提前无害；swap 失败即返 Err、DB 未被触碰
    // （原序 update 在前 → swap 失败会留下"DB 已翻但运行时仍旧 client"的假失败）。
    if let Some(reg) = &state.llm_registry {
        swap_registry(reg, &target).await?;
    }
    let now = DateTime::now();
    state
        .db
        .llm_provider_configs()
        .update_many(
            doc! { "workspaceId": &workspace_id, "isActive": true, "providerId": { "$ne": &provider_id } },
            doc! { "$set": { "isActive": false, "updatedAt": now } },
            None,
        )
        .await?;
    state
        .db
        .llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        )
        .await?;
```

（Ok(Json(...)) 返回不变。实现者按亲验的真实代码块精确替换。）

- [ ] **Step 3: KD-09 —— 新增 base_url_v1_warning 纯函数**

在 `src/routes/llm_providers.rs` 合适位置（如 swap_registry 附近或文件私有函数区）加：

```rust
/// openai 形态 base_url 软校验（KD-09）：不以 /v1 结尾时返回 warning 文案（None=无警告）。
/// anthropic 形态请求路径 {base_url}/v1/messages 自带 /v1，不校验（返 None）。软提示不阻断
/// 保存——各家兼容端点路径不一（Azure/代理网关可能非 /v1），hard block 会误伤合法配置。
fn base_url_v1_warning(fmt: LlmFormat, base_url: &str) -> Option<String> {
    if fmt != LlmFormat::Openai {
        return None;
    }
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        return None;
    }
    Some(format!(
        "baseUrl \"{trimmed}\" 不以 /v1 结尾：OpenAI 形态请求路径为 {{baseUrl}}/chat/completions，\
         多数服务商需 baseUrl 含 /v1（如 https://api.deepseek.com/v1）。若你的服务商路径确不含 /v1 可忽略此提示。"
    ))
}
```

**已亲验**：`LlmFormat` derive `Debug, Clone, Copy, PartialEq, Eq`（llm.rs:21），故 `fmt != LlmFormat::Openai` 直接可用（Copy 传值无所有权问题）。收 `LlmFormat` 参即可，无需改 `matches!`、无需 &str。

- [ ] **Step 4: KD-09 —— create_provider 返回体塞 warning（:200）**

把 create_provider 的返回（约 :200 `Ok(Json(json!({ "item": LlmProviderView::from(&cfg) })))`）改为（先算 warning + tracing::warn，None 时不出现 warning 键）：

```rust
    let warning = base_url_v1_warning(LlmFormat::parse(&cfg.format)?, &cfg.base_url);
    if let Some(w) = &warning {
        tracing::warn!("provider {} base_url 软校验: {w}", cfg.provider_id);
    }
    let mut resp = json!({ "item": LlmProviderView::from(&cfg) });
    if let Some(w) = warning {
        resp["warning"] = json!(w);
    }
    Ok(Json(resp))
```

（`cfg.format` 已在 create 里、`LlmFormat::parse` 上方 :154 调过一次成功；这里重解析同一值不会失败，或实现者把 :154 的 parse 结果存 let fmt 复用——择一，保证只 parse 合法值。`cfg.base_url` 已 trim :178。）

- [ ] **Step 5: KD-09 —— update_provider 返回体塞 warning（:270）**

把 update_provider 的返回（约 :270 `Ok(Json(json!({ "item": LlmProviderView::from(&refreshed) })))`）改为（用 refreshed.base_url/refreshed.format，反映实际存库值）：

```rust
    let warning = base_url_v1_warning(LlmFormat::parse(&refreshed.format)?, &refreshed.base_url);
    if let Some(w) = &warning {
        tracing::warn!("provider {} base_url 软校验: {w}", refreshed.provider_id);
    }
    let mut resp = json!({ "item": LlmProviderView::from(&refreshed) });
    if let Some(w) = warning {
        resp["warning"] = json!(w);
    }
    Ok(Json(resp))
```

（update 里 :210 已 `LlmFormat::parse(&body.format)?` 成功；refreshed.format 是存库值同样合法。若用 `matches!` 判断则不需再 parse——实现者据 Step 3 的实现方式对齐：若 base_url_v1_warning 收 LlmFormat 参则这里要 parse；可考虑让纯函数直接收 `&str` format 内部 parse，减少调用点重复——实现者择更简洁者，但纯函数须可单测。）

- [ ] **Step 6: KD-09 —— mod tests append 纯函数单测**

在 `src/routes/llm_providers.rs` 的 `mod tests`（约 :584）内 append：

```rust
    #[test]
    fn base_url_v1_warning_openai_missing_v1_warns() {
        let w = base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com");
        assert!(w.is_some(), "openai 缺 /v1 应 warning");
        assert!(w.unwrap().contains("/v1"));
    }

    #[test]
    fn base_url_v1_warning_openai_with_v1_ok() {
        assert!(base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com/v1").is_none());
        // 尾斜杠 trim 后仍含 /v1
        assert!(base_url_v1_warning(LlmFormat::Openai, "https://api.deepseek.com/v1/").is_none());
    }

    #[test]
    fn base_url_v1_warning_anthropic_never_warns() {
        // anthropic 拼 /v1/messages 自带 /v1，base_url 不含 /v1 也不警告。
        assert!(base_url_v1_warning(LlmFormat::Anthropic, "https://api.anthropic.com").is_none());
    }
```

（若 Step 3 让纯函数收 `&str` format，测试第一参改传 "openai"/"anthropic" 字符串。实现者与 Step 3 实现对齐。真哨兵：回退去掉 openai 判断 → anthropic case 变红；去掉 /v1 判断 → missing_v1 case 变红。）

- [ ] **Step 7: 编译 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib base_url_v1_warning 2>&1 | tail -15 && cargo test --lib 2>&1 | tail -5`
Expected: base_url_v1_warning 3 测 PASS；全 lib `test result: ok.` ≥ 350 passed / 0 failed。若 LNK1318（Windows-only）→ `cargo check --lib` + 人工核对。

- [ ] **Step 8: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/routes/llm_providers.rs && git commit -m "fix(provider): activate 先swap再写DB + openai base_url缺/v1软warning (KD-10/KD-09 P3家族⑥)"
```

---

## Task 2: KE-06 —— sync_accounts mcp_base_url 移 $setOnInsert（accounts.rs）

**Files:**
- Modify: `src/routes/accounts.rs:140-162`（sync_accounts upsert 的 $set/$setOnInsert）

**Interfaces:**
- Consumes: 无新接口。`account.mcp_base_url`（已在 :147 用）。
- Produces: 无。纯 upsert doc 结构调整。

- [ ] **Step 1: 亲验 sync_accounts upsert 现状**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn sync_accounts\|mcp_base_url\|mcp_api_key\|\\$set\|\\$setOnInsert" src/routes/accounts.rs | head -20`
Expected: 确认 sync_accounts（约 :64）upsert（约 :140-162）里 mcp_base_url 在 $set、mcp_api_key 在 $setOnInsert。**实现者 Read :140-165 全貌**确认 $set/$setOnInsert 各含哪些键。

- [ ] **Step 2: 把 mcp_base_url 从 $set 移到 $setOnInsert**

在 `src/routes/accounts.rs` 的 sync_accounts upsert doc：
- **从 $set 删除**：`"mcp_base_url": &account.mcp_base_url,`（原 :147）
- **在 $setOnInsert 加**（与 mcp_api_key :158 并列，加注释）：

```rust
                    "$setOnInsert": {
                        // KE-06：mcp_base_url + mcp_api_key 都仅插入时写 config 默认，后续 sync
                        // 不覆盖管理员经 update_account_mcp_key 手配的值（对称保护，防 sync
                        // 抹掉手配 base_url）。
                        "mcp_base_url": &account.mcp_base_url,
                        "mcp_api_key": &account.mcp_api_key,
                        "created_at": account.created_at,
                        "capacity": 0,
                    }
```

（$set 里保留 alias/display_name/app_id/wxid/nick_name/online/status/last_sync_at/updated_at/workspace_id/account_id——实现者按亲验的真实 $set 内容删 mcp_base_url 一行、其余不动。$set 里的 workspace_id/account_id 保留，保证反序列化完整性注释仍成立。）

- [ ] **Step 3: 编译 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --lib 2>&1 | tail -8 && cargo test --lib 2>&1 | tail -5`
Expected: `Finished` + `test result: ok.` ≥ 350 passed / 0 failed（upsert doc 结构调整不影响 lib 测）。

- [ ] **Step 4: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/routes/accounts.rs && git commit -m "fix(accounts): sync mcp_base_url 移 setOnInsert 对齐 api_key,防抹手配 (KE-06 P3家族⑥)"
```

---

## Self-Review 结论

- **Spec coverage**：KD-10（activate swap 提前）+ KD-09（openai base_url 软 warning）→ Task 1；KE-06（mcp_base_url 移 $setOnInsert）→ Task 2。三条 finding 全覆盖。设计非目标（不 hard block/不自动补 /v1/不回滚 active/不动 update_account_mcp_key/不碰 test_provider）在计划里通过"只改指定段"落实。
- **Placeholder scan**：无 TBD/TODO。Step 3/5 的"纯函数收 LlmFormat vs &str format"留实现者择一——非 placeholder，是明确的两个等价实现选项 + 约束（纯函数须可单测），实现者按 LlmFormat 是否 derive PartialEq 亲验后定（优先 matches! 或收 &str）。
- **Type consistency**：`base_url_v1_warning` 签名在定义（Step 3）、create 调用（Step 4）、update 调用（Step 5）、单测（Step 6）一致；返回体 warning 键条件插入（None 不出现）在 create/update 一致。KD-10 只移动语句不改类型。
- **反过拟合**：KD-09 纯函数单测真哨兵（回退去 openai/‌v1 判断即变红）。KD-10/KE-06 是 DB 语义调整无独立纯函数，靠终审亲验 + 现有集成测无回归（改动直白：移动语句/移动键）。
- **红线合规**：swap_registry 本身不动、update filter/doc 不动、update_account_mcp_key 不动；warning 文案/注释中性词无禁词；baseline 不回退；worktree 绝对路径。
