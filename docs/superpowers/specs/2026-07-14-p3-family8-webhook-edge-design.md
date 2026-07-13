# P3 家族⑧ webhook 入口边缘加固设计（A-03 / A-04 / A-05 / A-06）

> P3 桶B/C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` A-03（:78）/ A-04（:90）/ A-05（:102）/ A-06（:114）。四条 Low，webhook 入口边缘。全部行号亲验于最新 origin/main（含 #201）。

## 背景与定位

四条 webhook 入口 finding，一个 PR。经全链路亲验后分两类处理（用户裁决）：

- **A-06（真实功能缺陷）→ 实修**：`last_inbound_at` 统计 update 用 `?` 抛错，Mongo 瞬时错误吞掉本轮客户回复。
- **A-05（加固）→ 实修（最优解：verify 开关收敛）**：无 appId 回落 default account，多账号部署张冠李戴。
- **A-03 + A-04（生产不触发 / 已被幂等缓解）→ doc 标注**：代码注释 + 台账状态更新为 WontFix，不改逻辑。

## 关键亲验事实（决定方案，全部主控当场 Read）

1. **A-06 边界**（webhooks.rs）：inbound `insert_one`（:515-521）—— dup 幂等短路、其它错误 `Err`（fail-close 正确，写入失败不盲发）；contact `find_one`（:523-534）/ `upsert_webhook_contact`（:536-538）用 `?`（后续 Agent 决策依赖 contact 实体）；两者之间的 `last_inbound_at/last_message_at/updated_at` `update_one(...).await?`（:555-569）是纯统计/信号旁路，紧邻其后的 `collect_inbound_behavior_signals`（:573）已是 best-effort（:572 注释"任何一段失败仅 warn"）。故这个统计 update 的 `?` 是唯一"旁路却能拦掉应答"的点。
2. **A-05 完整威胁模型**（webhooks.rs `resolve_account_context` :980-1006）：
   - 有 appId + 查到 → 返 account（:992）。
   - 有 appId + 查不到 → **已是 400**（:997-999，P1 已修）。
   - 无 appId → 无条件回落 `(default_workspace_id, default_account_id, None)`（:1001-1005）。
   - **决定性**：`resolve_account_context`（handler :322）在验签门（:336）**之前**执行；返回的第三元 `webhook_secret` 供验签门用。verify=true（config.rs:722 默认）时无 appId → secret=None → `verify_webhook_signature`（:1774-1777）首行 `secret.ok_or(SecretNotConfigured)?` → 400。**故 verify=true 下无 appId 已被验签门天然挡死，default 回退到不了副作用点。A-05 真实危害面仅 = verify=false + 多账号 + 无 appId**。
   - `default_account_id` 默认字符串 `"default"`（config.rs:438），单账号部署可将其配成真账号 id，无 appId 回落是其正常工作路径。
3. **A-03**（webhooks.rs :486-489 + `stable_payload_hash` :749）：`dedupe_key` 无 `effective_message_id` 时回落 `payload:{hash}`；同内容连发第二条 hash 相同命中 unique 索引 → `duplicate:true` 静默丢（:517-518）。但真实 GeWe AddMsg 恒带 NewMsgId → `effective_message_id` 必有值 → 走 `message:{id}` 分支，payload-hash 分支仅自测 / 无 ID payload 触发。
4. **A-04**（webhooks.rs `verify_webhook_signature` :1766-1799）：校验 secret 存在 + `abs(now-ts) > skew*1000`（:1787，±300s）+ HMAC-SHA256，无 nonce / 一次性记录。300s 内重放理论可能，但 AddMsg 重放命中 message-id dedupe 幂等短路（:517）、Offline/Online 重放幂等 `$set online`、领导回复经 `resolve_escalation` 幂等 → 无重复副作用。

## 用户裁决（brainstorming）

1. **范围 = A-05/A-06 实修 + A-03/A-04 doc 标注**（同家族⑤模式）。
2. **A-05 修法 = 多账号才 400 / 单账号回退**，且**最优解 = 多账号 count 收敛到 `if !webhook_verify_signature` 分支内**——verify=true 时无 appId 已被验签门 400，无需付 count 代价；仅 verify=false（default 回退是唯一防线）时才 count，多账号 → 400。精准命中唯一危害窗口，生产（verify=true）零额外查询。
3. **A-06 = 仅那一个统计 update（:555-569）降 best-effort**，inbound insert / contact 查询的 fail-close 不动。

## 目标

- A-06：`last_inbound_at` 统计 update 落库失败仅 warn、不拦本轮应答（与 :573 旁路纪律对齐）。
- A-05：无 appId 且未开验签且多账号时 400（防 default account 张冠李戴）；verify=true 或单账号不变。
- A-03/A-04：代码注释 + 台账状态标注"已知边界 / WontFix"，不改逻辑。

## 架构：webhooks.rs 三处独立改动 + docs

### A-06 —— last_inbound_at 统计 update 降 best-effort（webhooks.rs:555-569）

```rust
    // A-06：last_inbound_at/last_message_at/updated_at 是统计/信号旁路字段，落库失败不应连累
    // 本轮应答（inbound 已在上方 insert 成功、去重已保证）。降 best-effort：失败仅 warn，与紧邻的
    // collect_inbound_behavior_signals（下方）旁路纪律对齐。
    if let Err(e) = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! { "$set": { "last_inbound_at": now, "last_message_at": now, "updated_at": now } },
            None,
        )
        .await
    {
        tracing::warn!(contact_wxid = %from_wxid, error = ?e, "更新 last_inbound_at 失败（统计旁路，不影响应答）");
    }
```

inbound `insert_one`（:515-521）fail-close 不动；contact `find_one`/`upsert`（:523-538）`?` 不动。

### A-05 —— 多账号 count 收敛到 verify=false 分支（webhooks.rs:1001-1005）

```rust
    // A-05：无 appId 时的账号归属防线。验签门（handler 处 webhook_verify_signature 块）在本函数
    // 之后执行——verify=true 时无 appId → 返回 secret=None → verify_webhook_signature 必
    // SecretNotConfigured → 400，default 回退到不了副作用点，无需在此付 count 代价。仅当未开验签
    // （default 回退是唯一防线）时才校验：多账号无 appId 无法判断消息归属 → 400（防落到 default
    // account 张冠李戴）；单账号（≤1）无歧义 → 回落 default，不打断上游确实不带 appId 的单账号部署。
    if !state.config.webhook_verify_signature {
        let account_count = state.db.accounts().count_documents(doc! {}, None).await?;
        if account_count > 1 {
            return Err(AppError::BadRequest(
                "webhook 缺 appId 且存在多个账号，无法判断消息归属".into(),
            ));
        }
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
```

`emit_unknown_app_id_event`（:1031-1058）仅在 `resolve_account_context` 返 `BadRequest` 时被 handler（:325-329 BadRequest 分支）调用。改前无 appId 恒返 Ok → 该函数的 `None` 分支是死路径；A-05 新增的多账号 400 激活它——故 `None` 分支文案须反映"多账号无法判归属、已拒收"（不能沿用旧的"已按 default account 处理"，那与 rejected 状态矛盾）。单账号无 appId 返 Ok、不进 BadRequest 臂、不写此事件。

### A-03 / A-04 —— doc 标注（不改逻辑）

- A-03（webhooks.rs dedupe_key 回落处 :485-489 附近）加注释："无任何 msgId 时回落 payload-hash；同内容连发第二条会被当 dup 丢弃。生产 GeWe AddMsg 恒带 NewMsgId 走 message:{id} 分支，此路径仅自测 / 无 ID payload 触发 —— 已知边界，不修（掺时刻/nonce 会削弱重放去重）。"
- A-04（webhooks.rs `verify_webhook_signature` :1766 附近或 doc 注释）加说明："仅校验 secret + ±skew 时间窗，无 nonce/一次性记录；300s 内重放理论可能，但 AddMsg 命中 message-id dedupe 幂等短路、Offline/Online 幂等 $set、领导回复经 resolve_escalation 幂等 → 无重复副作用 —— 已知边界，不修（加 nonce 需状态存储，收益不抵成本）。"
- 台账 `2026-07-11-deep-logic-audit-findings.md` 的 A-03 / A-04 "状态"字段从 Open 更新为 "WontFix（已知边界，doc 标注）"。

## 改动面

- **Modify** `src/webhooks.rs`：A-06 统计 update 降级（:555-569）；A-05 resolve_account_context 无 appId 分支（:1001-1005）；A-03（:485-489 附近注释）；A-04（:1766 附近注释）。
- **Modify** `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：A-03 / A-04 状态更新为 WontFix 标注。

## 测试计划

- **A-05 / A-06**：均为 async DB 交互函数（`resolve_account_context` 依赖 state.db + state.config；A-06 是 update_one 降级），无独立纯函数可 lib 单测。**验证方式**：终审代码级亲验（A-05 verify 开关分支逻辑正确、A-06 降级只覆盖统计 update、inbound insert/contact 查询 fail-close 不变）+ 现有 webhook 集成测无回归。实现者查是否有既有 webhook / resolve_account_context 集成测可轻量扩展一条"verify=false + 多账号 + 无 appId → 400"断言；若无低成本扩展点则终审亲验（改动直白：一个 if 分支 + 一个 `?`→best-effort）。与家族⑥ KD-10/KE-06 同性质（DB 交互语义调整靠终审 + 集成测）。
- **A-03 / A-04**：纯 doc 标注，无测试。
- **baseline**：`cargo test --lib` ≥ 350 / 0 不回退（本族不新增/删除 lib 单测，除非找到可扩展的纯函数点）。

## 回归风险

1. **A-06 纯降级**：inbound insert / contact 查询 fail-close 不变；仅统计 update 从 `?` 变 best-effort（修复目标）。verify 无关。
2. **A-05 verify 开关收敛**：verify=true 路径**完全不变**（不进 count 分支，无 appId 仍由验签门 400）；verify=false 单账号不变（≤1 回落 default）；仅 verify=false + 多账号从"回落 default"变"400"（修复目标）。
3. **A-03/A-04 纯注释**：零逻辑变更。
4. **check-no-human-takeover + check-no-model-hint lint**：webhooks.rs 在 src/ 扫描范围——新增注释用中性词（账号/消息/归属/验签/统计/旁路），无禁词（人工/接管/takeover/hand-off）、无模型品牌名。

## 非目标（YAGNI）

- 不加 nonce / 一次性签名记录（A-04；需状态存储，dedupe 已缓解）。
- 不改 payload-hash dedupe 逻辑（A-03；掺时刻会削弱重放去重）。
- 不动 inbound `insert_one` 的 fail-close（写入失败不盲发是对的）。
- 不动验签门本身、不动 contact find_one/upsert 的 `?`。
- A-05 不引入账号数缓存（verify=false + 无 appId 是极边缘路径，一次 count 可接受；缓存引入失效复杂度）。
