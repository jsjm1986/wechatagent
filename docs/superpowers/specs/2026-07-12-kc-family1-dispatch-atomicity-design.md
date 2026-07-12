# 批C家族① 修复设计：campaign dispatch 补偿回滚 + status 前置门

- 日期：2026-07-12
- 分支：`fix/kc-family1-dispatch-atomicity`（基于最新 origin/main 12cdd54）
- 来源：深度审查批C 跨环节根因家族①（触达多步非事务写：KC-01 + KC-02 + KC-03；台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`）
- 优先级：P1
- 方案：补偿回滚 + status 前置门（用户裁定；否决"加反向关联+幂等自愈"，因其需改 15 处 AgentTask 构造点+迁移+report 双向 join，而 finding 触发前提是 Err-return 非进程崩溃，补偿回滚已完整覆盖，改动面小十倍）

## 问题（KC-01/02/03 根因，已主控逐条亲验最新 main 成立）

`dispatch_campaign`（`src/routes/campaigns.rs:289`）对每个命中 contact 做**三步非原子写**（循环体 :328-371）：
1. `campaign_sends().insert_one(&send)`（:341）——占去重位（unique 索引 `(campaignId, contactWxid)`），此时 `task_id=None`、`status="enqueued"`。
2. `tasks().insert_one(&task).await?`（:351）——建 follow_up task。
3. 回填 taskId `campaign_sends().update_one(...).await?`（:357-365）。

第 2/3 步任一 `?` 失败即 `return Err`（沿用 :370 的 `Err(e)=>return Err`）→ 中断整批。

**已亲验的三个 finding**：
- **KC-01（孤儿 send 永久漏推）**：第 2 步失败 → 留下 `enqueued`+`task_id=None` 的 send、无对应 task。重新 dispatch 时该 contact 撞 DuplicateKey（:369）被静默跳过 → task 永远建不出、客户永久漏推。report 侧 `s.task_id=None`（:520-523 `filter_map` 丢弃）→ 归 `pending/not_yet_run` 假象。**无 worker 对账**（主控 grep 零命中）。
- **KC-02（卡 dispatching + 无 status 门）**：中断 → 循环后的 completed update（:373-382）不执行 → campaign 永久停 `dispatching`，无 worker 恢复。且 dispatch 前**无 status 前置校验**（:294-325 只校验存在 + 圈人非空）→ `completed` 活动可被反复 dispatch。
- **KC-03（taskId 回填失败 → 报表失真）**：第 3 步失败 → task 已建、worker 会真发消息，但 `send.task_id=None` → report join 显 `pending`、成效虚低。

**关键约束（已亲验，决定方案）**：`AgentTask`（`models.rs:829`）无 campaign/send 反向关联字段，`#[derive(Serialize, Deserialize)]` 无 `Default`，全仓 15 处显式构造点。故"加反向关联+幂等自愈"改动面大；而 finding 触发是循环中途 Mongo 瞬时**写错误（Err-return）**，非进程崩溃 → 补偿回滚（同一 handler 内 unwinding 时清理已建 state）即可根治。

## 设计

### 组件 1：status 前置门（纯函数，可单测）

```rust
/// 仅这些 status 允许 dispatch。dispatching = 允许重入恢复（配合补偿回滚，
/// 已完成的 send 撞去重跳过、失败/剩余 contact 重建）；completed = 拒绝（防重复推送）。
/// 未知态 = 拒绝（fail-safe）。
pub(super) fn dispatch_allowed_from_status(status: &str) -> bool {
    matches!(status, "draft" | "previewed" | "dispatching")
}
```
- 放置：`src/routes/campaigns.rs`（与 dispatch_campaign 同文件，`pub(super)` 便于单测）。
- 接线：`dispatch_campaign` 在把 status 置 `dispatching`（:317-325）**之前**校验 `campaign.status`；不允许 → 返 `AppError::BadRequest("当前状态 {status} 不可派发")`（拒 completed 重推）。

### 组件 2：循环内补偿回滚（每 contact 全有或全无）

改写循环体 :341-371 的 `Ok(send_res)` 分支，把裸 `?` 换成"失败即补偿删除已建 state 再 return Err"：

```rust
Ok(send_res) => {
    let send_id = send_res.inserted_id.as_object_id();
    let task = build_campaign_follow_up_task(...);
    assert_agent_task_status_valid(&task.status);
    let task_res = match state.db.tasks().insert_one(&task, None).await {
        Ok(r) => r,
        Err(e) => {
            // 补偿：删掉刚占位的 send，避免留下 task_id=None 的孤儿 send（KC-01）。
            if let Some(sid) = send_id {
                let _ = state.db.campaign_sends()
                    .delete_one(doc! { "_id": sid }, None).await;
            }
            return Err(e.into());
        }
    };
    if let (Some(sid), Some(tid)) =
        (send_id, task_res.inserted_id.as_object_id())
    {
        if let Err(e) = state.db.campaign_sends()
            .update_one(doc! { "_id": sid },
                doc! { "$set": { "taskId": tid } }, None).await
        {
            // 补偿：回填失败则删 task + send，保持 all-or-nothing（KC-03）。
            let _ = state.db.tasks().delete_one(doc! { "_id": tid }, None).await;
            let _ = state.db.campaign_sends()
                .delete_one(doc! { "_id": sid }, None).await;
            return Err(e.into());
        }
    }
    dispatched += 1;
}
```
- 补偿删除用 `let _`（best-effort）：补偿删除本身再失败（连续两次 Mongo 错）才留孤儿，属极窄双重故障窗口，超出 finding 单次瞬时错范围。

### 组件 3：重入恢复（组件 1+2 使其成立，无额外代码）

中途失败 → 留 `dispatching` + 失败 contact 已被组件 2 回滚（无孤儿）→ 运营重新 dispatch → 组件 1 放行 `dispatching` → 已完成 send 撞 unique 索引 DuplicateKey 跳过（:369 既有逻辑）→ 失败+剩余 contact 重建 → 循环结束置 `completed`。零孤儿的干净恢复。

### 为什么不调换 send/task 写序

send insert 是去重闸（unique `(campaignId, contactWxid)`），必须最先占位。若先建 task：重入时 send 撞 DuplicateKey 跳过 → 留**孤儿 task**（且会给已推 contact 重发消息）。故保持"send 先占位 + 补偿回滚"，不动写序。

## 接受的窄窗口（知情记录，不修）

回填失败删 task 时，若 worker 恰在这亚毫秒窗口已 claim 该 task——极罕见双重巧合（回填失败 AND 同刻被 claim）。最坏后果："一个 contact 收到消息但无 send 记录"，**非红线破坏、比原 KC-03 更罕见**。Medium finding 不为此过度工程（否则回到需反向关联+事务的重方案）。

## 不改动的（严格限定范围）

- schema：`AgentTask` / `CampaignSend` 不加字段（无迁移）。
- report join（`campaign_report` :500-539 靠 task_id.hex）：补偿回滚保证 all-or-nothing 后，成功 send 必有 taskId、失败 contact 无 send，report 不再有"task 真发但显 pending"的 KC-03 失真，无需改 report 侧。
- 圈人 `resolve_segment_contacts`、其它 handler、send/task 写序。

## 测试策略

- **组件 1 纯函数** `dispatch_allowed_from_status` → **lib 单测**（无需 Docker，进 baseline）：draft/previewed/dispatching → true；completed/未知态 → false。
- **组件 2 补偿回滚** → **集成测**（现成 `tests/campaign_dispatch_integration.rs`，直调 `dispatch_campaign` handler + testcontainers）：用 MongoDB collection validator 让 `agent_tasks` insert 失败注入故障 → 断言 `campaign_sends` 无孤儿（该 contact 无 send 记录），dispatch 返 Err。`#[ignore]`，CI integration job 跑。
- **status 门集成测**：先把 campaign 置 `completed` → dispatch → 断言 BadRequest（防重推）。
- 既有 3 个集成测（zero_hits / cross_workspace / builds_and_dedups）happy-path 行为不变，回归守护。

## 验证

- `cargo test --lib`（+组件1 纯函数单测，baseline lib ≥ 350 / 0 failed 不回退）。
- `cargo test --test campaign_dispatch_integration --no-run`（本地编译；执行留 CI，需 Docker）。
- no-human-takeover lint：新增行用「派发/触达/去重/补偿/回滚」措辞，无禁词。

## 交付

- 单一 src 文件改动：`src/routes/campaigns.rs`（+status 门纯函数+单测，dispatch_campaign 补偿回滚+status 校验）。
- 集成测：`tests/campaign_dispatch_integration.rs`（+故障注入回滚测+status 门测）。
- 独立修复 PR（基于最新 main）。台账 KC-01/02/03 标 Closed（KC-02 的 status 门 + dispatching 重入部分随本修复解决）。
