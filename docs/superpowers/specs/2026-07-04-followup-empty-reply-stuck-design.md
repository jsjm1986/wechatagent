# 修复：FollowUp 任务在 should_reply=true+空 reply_text 时卡 running

> 分支 `fix/followup-empty-reply-stuck`（从 origin/main 切）
> 来源：本会话定向审计（webhooks+tasks agent）候选 #1，已逐行亲验为真 bug（低危、安全失败）

## 1. 缺陷（对最新代码 100% 亲验）

`run_user_operation_gateway_inner` 在 `finalize_status == Approved` 分支（此前的 `!Approved` / 第二道 precheck / superseded 三处都已 `return Ok(())` 提前返回，故 task 终态逻辑只对 Approved 生效）里，用两处判定决定 FollowUp 任务的终态：

- **置 `outbox_enqueued`**（gateway.rs:2296）：`should_reply && !reply_text.trim().is_empty()`
- **`cancel_task("no_reply")`**（gateway.rs:2385，原）：`!should_reply`

两字段在 `promote_raw_to_decision`（types.rs:936-937）里**独立**赋值；R1.4 校验（types.rs:760）只在 `should_reply=true` 时查 `why_should_reply` 长度，**从不**校验 `reply_text` 非空。故 LLM 可输出 `should_reply=true` + 空 `reply_text` 的退化决策。

此时：
- 2296 不置 `outbox_enqueued`（文本空）
- 2385 不 cancel（should_reply 真）
- 媒体/名片路径也不发（`media_send_allowed` 依赖 `outbox_eligible`，后者要求非空文本），且它们从不写 task 状态

→ 函数返回 `Ok(())`，task 仍停在 `running`。`reclaim_stale_running_tasks`（tasks.rs:33）超时重置为 `retry` 反复重跑，`claim_recovery_count >= 3` 后强制 `failed`（tasks.rs:66）。

## 2. 严重度与可达性

- **无错误发送**：outbox 从不入队，客户不会收到空消息。失败模式安全。
- **可达性低**：需 LLM 输出「想回复却给空正文」这种自相矛盾决策，且空回复还得通过 reviewer 的 human_like/emotional 软闸（空串几乎必被判低分 → needs_revision → 空改写 → revision_failed → `should_reply=false` → 触发 2385 cancel）。软闸在绝大多数情况先兜住，真正卡死需 reviewer 给空回复打过关分——复合退化。
- 属与 PR#103/#105 同类的「兄弟路径不对称」遗漏：终态判定漏了一个 should_reply 与 reply_text 解耦的组合。

## 3. 方案（最小改动）

抽纯函数把「本 run 是否会投递文本」这一真实谓词显式化，两处终态判定共用：

```rust
fn text_send_eligible(should_reply: bool, reply_text: &str) -> bool {
    should_reply && !reply_text.trim().is_empty()
}
```

- 2296：`if text_send_eligible(should_reply, reply_text)` 置 `outbox_enqueued`（**字节等价**现状，只是抽函数）。
- 2385：`if !text_send_eligible(should_reply, reply_text)` cancel。原本只有 `!should_reply` 落 cancel；现在 `should_reply=true` 但空文本也落 cancel（reason 分两句：无需触达 / 想回复但正文为空）。`cancel_task` 写 `status="cancelled"`（闭集内）+ `gateway_status="no_reply"`（GATEWAY_STATUS_VALUES 内），无新枚举值。

**关键不回归**：`should_reply=false`（含空/非空文本）与 `should_reply=true`+非空文本两种主路径行为**完全不变**；仅新增 `should_reply=true`+空文本 → cancel 这一此前卡死的分支。媒体/名片门（`media_send_allowed → outbox_eligible`）不受影响（本就要求非空文本）。

## 4. 测试（纯函数单测，gateway.rs `#[cfg(test)]`）

- `text_send_eligible_true_only_when_should_reply_and_nonempty`
- `text_send_eligible_false_when_should_reply_but_text_empty_or_blank`（空串 + 纯空白，锁住本次修复的核心分支）
- `text_send_eligible_false_when_not_should_reply`

## 5. 验证
- `cargo test --lib text_send_eligible` 3/0；`cargo test --lib` ≥ 350 / 0（实测 1806 / 0）。
- CI 双门（baseline + integration）。
