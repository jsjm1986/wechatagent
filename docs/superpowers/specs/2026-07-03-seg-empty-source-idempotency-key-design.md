# 修复：多段回复空 source_event_id 时幂等 key 丢失 run_id 隔离

> 分支 `fix/seg-empty-source-idempotency-key`（从 origin/main 切）
> 来源：本会话重跑审计（send-path agent）候选 #3，已逐行亲验为真 bug

## 1. 缺陷（对最新代码 100% 亲验）

`run_user_operation_gateway_inner` 把整条回复拆成多段逐条 enqueue（gateway.rs:2488-2527）。每段的 `source_event_id` 派生：

```rust
let source_event_id = match &trigger {
    AgentTrigger::Inbound(msg) => msg.message_id.clone().unwrap_or_default(), // 可为空串
    AgentTrigger::FollowUp(task) => task.id.map(|id| id.to_hex()).unwrap_or_default(),
};
let seg_source_event_id = if total > 1 {
    format!("{source_event_id}#seg{idx}")   // 空 source → "#seg0" / "#seg1" ...（非空）
} else {
    source_event_id.clone()
};
```

`outbox::enqueue` 用 `media_routes_synthetic(media=None, card=None, source_event_id)` 判定路由（outbox.rs:198、449）：仅当 `source_event_id.trim().is_empty()` 才走 synthetic key（`synthetic:{run_id}:{contact}:{hash}`，**含 run_id**）；否则走 `{source_event_id}:{contact}:{hash}`（**不含 run_id**）。

后果：`source_event_id` 为空且多段时，`seg_source_event_id = "#seg0"` 非空 → 走非 synthetic 分支 → key = `"#seg0:{contact}:{content_hash}"`，**丢掉 run_id**。两次不同 run 对同一 contact 产出雷同分段内容（如都含一句"好的"）→ content_hash 相同 → idempotency_key 相同 → 第二次 run 的该段被 `IdempotentSkip` 静默吞掉、**客户少收一段回复**。

单段空 source（`total==1`）走 synthetic、含 run_id，正确。**只有多段空 source 破**——两条路径不对称。

## 2. 可达性

`source_event_id` 空的条件：`AgentTrigger::Inbound` 且 `message_id == None`。真实 GeWe 推送 9 个字段（webhooks.rs:451-466）总有其一 → 生产极罕见，属畸形/兜底 payload。多段 + 跨 run 雷同分段进一步收窄。低频，但确为静默丢消息的正确性 bug，且是与本会话已修一系列「同类兄弟路径不对称」同源缺陷。

## 3. 方案（最小改动）

空 `source_event_id` 时，分段 key 的 base 回落 `run_id`，保证多段 key 仍按 run 隔离；非空时保持用真实 `source_event_id`（它是 message_id、本身即正确幂等锚：同消息重放需命中同 key 去重，**绝不能掺 run_id**，否则破坏重放去重致重复发送）。

抽纯函数便于单测：

```rust
fn segment_idempotency_base<'a>(source_event_id: &'a str, run_id: &'a str) -> &'a str {
    if source_event_id.is_empty() { run_id } else { source_event_id }
}
```

循环内：
```rust
let seg_source_event_id = if total > 1 {
    format!("{}#seg{idx}", segment_idempotency_base(&source_event_id, &run_id))
} else {
    source_event_id.clone()
};
```

- 非空 source 多段：`{message_id}#seg{idx}` —— **字节等价**现状。
- 空 source 多段：`{run_id}#seg{idx}` → 非 synthetic 分支 key = `{run_id}#seg{idx}:{contact}:{hash}` —— run 隔离 ✓ 且 seg 隔离 ✓（#68 防雷同段撞键仍成立）。
- 单段（空/非空）：不变。

**不动** synthetic key 函数 / 媒体 / 名片路径（后者凭 asset_id/card_id 恒走 synthetic、已含 run_id，不受影响，已亲验）。

## 4. 测试

`src/agent/gateway.rs` 既有 `#[cfg(test)]`，加：
- `segment_idempotency_base_falls_back_to_run_id_when_source_empty`：`("","run123")==​"run123"`。
- `segment_idempotency_base_keeps_source_when_present`：`("msg456","run123")==​"msg456"`。

（分段 key 的 collision-freedom 已由 outbox 现有单测覆盖，此处只锁 base 选择这一新不变量。）

## 5. 验证
- `cargo build --lib` + `cargo test --lib` ≥350 通过、0 失败。
- CI 双门（baseline + integration）。
