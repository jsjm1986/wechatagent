# P3 家族⑥ provider/MCP 配置加固设计（KD-10 + KD-09 + KE-06）

> P3 桶B/C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KD-10（:965-973）+ KD-09（:954-963）+ KE-06（:1116-1124）。三条 Low，provider/MCP 配置层就绪债/防御补强，各自独立。全部行号亲验于分支基点 origin/main `d027fb9`（含 #198）。

## 背景与定位

三条 provider/MCP 配置就绪债，一个 PR：
- **KD-10（DB↔运行时非事务）**：provider 热切换 swap 失败时 DB 已置新 active、运行时仍旧 client、返 Err 掩盖"DB 已改"。
- **KD-09（配置校验缺失）**：base_url 缺 /v1 无前置校验，openai 形态漏填 /v1 → 405。
- **KE-06（保护策略不对称）**：sync_accounts 用 $set 覆盖手配 mcp_base_url，与 mcp_api_key 的 $setOnInsert 保护不对称。

**用户裁决（brainstorming）**：KD-10 先 swap 再写 DB；KD-09 openai 形态软 warning（不 hard block）；KE-06 mcp_base_url 移 $setOnInsert 对齐 api_key。

## 关键亲验事实（决定方案，全部主控当场 Read 亲验）

1. **swap_registry 无 DB 副作用**（KD-10 修法可行性）：`swap_registry`（llm_providers.rs:552-578）= `LlmFormat::parse` → `LlmClient::with_format`（构造，可失败）→ `reg.swap(client, meta).await`（原子替换，无返回值、无 DB 写）。**纯构造 + 原子 swap**，故提到 DB update 之前无害——swap 成功才写 DB。
2. **activate_provider 现序**（llm_providers.rs:305-346）：find target（:312）→ update_many 清旧 active（:325）→ update_one 置新 active（:334）→ swap_registry（:341）。swap 失败则 DB 已在 :325/:334 提交、返 Err。
3. **LlmFormat 仅 2 形态 + 路径不同**（KD-09 修法正确性）：`Openai`（llm.rs:15）拼 `{base_url}/chat/completions`（llm.rs:364，base_url 需含 /v1）；`Anthropic`（llm.rs:19）拼 `{base_url}/v1/messages`（base_url 不该含 /v1，它自带 /v1 前缀）。故 /v1 校验**只对 openai 形态**有意义，anthropic 不校验。`LlmFormat::parse`（llm.rs:35）在 create/update 已调（:154/:210）。
4. **create/update 返回体无测试断言**（KD-09 加 warning 字段安全）：`create_provider` 返 `json!({"item":...})`（:200）、`update_provider` 返 `json!({"item":...})`（:270）；亲验 tests/ 无断言这两端点返回体形态（"item" 断言都属 ask_human 等其它端点）。加可选 `warning` 字段无回归。llm_providers.rs 已有 `mod tests`（:584+，纯函数测），KD-09 纯函数测可 append。
5. **KE-06 移位安全**（accounts.rs:140-162）：sync_accounts upsert 里 `mcp_base_url`（:147）在 $set（值恒 config.mcp_base_url）、`mcp_api_key`（:158）在 $setOnInsert。注释（:152-153）声明"必填字段都在 $set 或 $setOnInsert"——移到 $setOnInsert 仍满足"字段存在于 upsert"，反序列化完整性不破。`update_account_mcp_key`（:171）是管理员手配 base_url 的正确入口（该处本就该写，不动）。

## 目标

- KD-10：activate_provider 先 swap 成功再写 DB active，让"返 Err ⟺ DB 未改"一致。
- KD-09：openai 形态 base_url 缺 /v1 时软 warning（不阻断保存），前置提醒管理员避免 405 坑。
- KE-06：mcp_base_url 从 $set 移到 $setOnInsert，与 mcp_api_key 对称保护手配值。

## 架构：三条独立加固

### KD-10 —— activate_provider 先 swap 再写 DB（llm_providers.rs:305-346）

把 swap_registry 提到两个 update 之前：

```rust
    // ...find target（:312-320）不变...
    // KD-10：先 swap（成功才写 DB），让"返 Err ⟺ DB 未改"一致。swap_registry 是纯
    // client 构造 + 原子替换、无 DB 副作用，提前无害；swap 失败即返 Err，DB 未被触碰
    // （原序 update 在前 → swap 失败留下"DB 已翻但运行时旧 client"的假失败）。
    if let Some(reg) = &state.llm_registry {
        swap_registry(reg, &target).await?;
    }
    let now = DateTime::now();
    state.db.llm_provider_configs()
        .update_many(
            doc! { "workspaceId": &workspace_id, "isActive": true, "providerId": { "$ne": &provider_id } },
            doc! { "$set": { "isActive": false, "updatedAt": now } },
            None,
        ).await?;
    state.db.llm_provider_configs()
        .update_one(
            doc! { "workspaceId": &workspace_id, "providerId": &provider_id },
            doc! { "$set": { "isActive": true, "updatedAt": now } },
            None,
        ).await?;
    Ok(Json(json!({ "ok": true, "item": LlmProviderView::from(&target) })))
```

性质：swap 成功路径行为与原序等价（都是 swap+写 DB，结果一致）。唯一变化=swap 失败时 DB 不再被改（修复目标）。swap 成功后 DB 写失败虽反向不一致（运行时新/DB 旧），但 DB 写失败远比 swap 构造失败罕见 + 重启 ensure_default_llm_provider 按 DB active 重建自愈。

### KD-09 —— openai 形态 base_url 软 warning（纯函数 + create/update 共用）

新增私有纯函数：
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

create/update 里（`LlmFormat::parse` 后已有 fmt）：算 warning → 塞返回体 + tracing::warn（不阻断）：
```rust
    let fmt = LlmFormat::parse(&body.format)?;   // create :154 / update :210 已有，复用其结果
    // ...原保存逻辑不变...
    let warning = base_url_v1_warning(fmt, &cfg.base_url);   // 用已 trim 的 base_url
    if let Some(w) = &warning {
        tracing::warn!("provider {} base_url 软校验: {w}", cfg.provider_id);
    }
    Ok(Json(json!({ "item": LlmProviderView::from(&cfg), "warning": warning })))
```
（`warning: Option<String>` → serde 序列化为 `null` 或字符串；前端可选读。若要 None 时不出现该键，用 `json!` 条件构造或返回体结构体带 `skip_serializing_if`——实现者择一，保持"无警告时不误导"。）

### KE-06 —— mcp_base_url 移 $setOnInsert（accounts.rs:140-162）

```rust
// $set 里删除 "mcp_base_url": &account.mcp_base_url（原 :147）
// $setOnInsert 里加（与 mcp_api_key :158 并列）：
    "$setOnInsert": {
        // KE-06：mcp_base_url + mcp_api_key 都仅插入时写 config 默认，后续 sync 不覆盖
        // 管理员经 update_account_mcp_key 手配的值（对称保护，防 sync 抹掉手配 base_url）。
        "mcp_base_url": &account.mcp_base_url,
        "mcp_api_key": &account.mcp_api_key,
        "created_at": account.created_at,
        "capacity": 0,
    }
```

行为变化=已存在账号 sync 时不再重置 base_url（修复目标）；新账号首次 sync 仍写 config 默认（$setOnInsert 插入生效）。与 mcp_api_key 完全对称。

## 改动面

- **Modify** `src/routes/llm_providers.rs`：`activate_provider`（:305-346）swap 提前（KD-10）；新增 `base_url_v1_warning` 纯函数 + `create_provider`（:148）/`update_provider`（:203）算 warning 塞返回体（KD-09）；`mod tests`（:584+）append 纯函数单测。
- **Modify** `src/routes/accounts.rs`：`sync_accounts`（:140-162）mcp_base_url 从 $set 移 $setOnInsert（KE-06）。

## 测试计划

- **KD-09 纯函数 lib 单测（本地可跑）**：`base_url_v1_warning` 真哨兵：
  - openai + `https://api.deepseek.com`（缺 /v1）→ Some（含提示）；
  - openai + `https://api.deepseek.com/v1`（含 /v1）→ None；
  - openai + `https://api.deepseek.com/v1/`（尾斜杠）→ None（trim 后含 /v1）；
  - anthropic + 任意 → None（不校验）。
  回退（去掉 openai 判断或 /v1 判断）即断言变红。
- **KD-10 / KE-06**：均依赖 DB（+registry）交互，无独立纯函数 → 集成测（Docker）成本高；靠**终审亲验**顺序调换 / $setOnInsert 移位正确 + 现有 provider/accounts 集成测无回归。若既有集成测有轻量断言点可扩展，否则终审代码级确认（改动直白：KD-10 纯移动 3 个语句、KE-06 移 1 个键）。

## 回归风险

1. **KD-10 纯顺序调换**：swap 无 DB 副作用（亲验），成功路径行为与原序等价。唯一变化=swap 失败时 DB 不再被改（修复目标）。
2. **KD-09 纯加 warning**：不阻断任何保存；openai 含 /v1 或 anthropic → warning=None 行为完全不变。返回体加可选 `warning` 字段（亲验现有 create/update 返回体无测试断言，前端不读也无害）。
3. **KE-06 移位**：已存在账号 sync 不再重置 base_url（修复目标）；新账号首次 sync 仍写 config 默认。与 api_key 对称。
4. **baseline**：`cargo test --lib` ≥ 350 / 0 不回退（KD-09 纯函数测 append）。
5. **check-no-human-takeover lint**：llm_providers.rs / accounts.rs 在 src/routes/ **扫描范围内**——warning 文案/注释用中性词（服务商/路径/配置/账号/手配），无禁词（人工/接管/takeover/hand-off）。

## 非目标（YAGNI）

- KD-09 不做 hard block（各家路径不一，误伤 Azure/代理合法非 /v1 端点）；不自动补全 /v1（无法可靠判断，补错更糟）。
- KD-10 不做"swap 失败回滚 active 标记"（先 swap 已让"返 Err ⟺ DB 未改"，回滚方案改动大且回滚本身可能失败）。
- KE-06 不动 update_account_mcp_key（管理员手配路径正确，本就该写 base_url）。
- 不碰 test_provider / classify_llm_error_for_user（现有缓解机制保留）。
- 不改 ensure_default_llm_provider 重启自愈逻辑（KD-10 的自愈兜底保留）。
