# P3 家族④ campaign 规模就绪债设计（KC-04 + KC-07 + KC-06）

> P3 桶C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KC-04（:699-709）+ KC-06（:740-749）+ KC-07（:751-758）。三条 Low，全在 `routes/campaigns.rs`，两条同源（KC-04/07 受众规模保护）+ 一条独立（KC-06 计数命名对齐）。全部行号亲验于分支基点 origin/main `bbc8b7e`（含 #195）。

## 背景与定位

campaign（管理台运营触达）链路：圈人（两阶段：Mongo 粗筛 + 内存精筛）→ preview（预览命中数+抽样，status→previewed）→ dispatch（重跑圈人→逐 contact 建 campaign_send 占去重位 + 建 follow_up task + 回填 taskId，status→completed）→ report（每人明细归桶）。#183（KC-01/02/03）已为 dispatch 建了**每-contact 补偿回滚**（insert send→insert task→回填 taskId，任一步失败各自回滚，逐 contact all-or-nothing）+ status 前置门。本家族三条 finding 都是这条链上的**规模就绪债 / 可观测性缺口**（非发送正确性 bug）：

- **KC-04 + KC-07（同源）**：`resolve_segment_contacts`（campaigns.rs:190-209）粗筛 cursor 无 limit，全量 collect 进 Vec；dispatch 循环对每 hit 顺序 await 三次 DB 写。几千上万 contact 时单 HTTP 请求内串行几千~上万次往返 → 请求超时 + 全量 contacts 驻内存。preview 与 dispatch 共用 `resolve_segment_contacts`。
- **KC-06**：preview 写 `targetCount`（:289 命中总人数）；dispatch 只写 `dispatchedCount`（:430 本次去重后新入队数），**不回刷 targetCount**；report 的 `build_sends_summary` `targetCount`（:523 = items.len() 台账总行数）。三个相近命名三种含义，运营会误读"有人没发出去"（实为受众漂移变少或去重跳过）。

## 关键亲验事实（决定方案，全部主控当场 Read 亲验）

1. **发送本就异步**：`build_campaign_follow_up_task`（campaigns.rs:139-170）建的是 `AgentTask{kind:"follow_up", status:"pending", run_at:now}`，真正的发送由 `tasks.rs` worker loop（tasks.rs:175 `status:{$in:["pending","retry"]}` claim → :237 `handle_follow_up_task`）**异步分批消费**。所以 KC-04 的真实痛点是**单 HTTP 请求内串行几千次 DB 写导致请求超时 + 全量 contacts 驻内存**，**不是发送洪峰**（发送侧本就异步、由 worker 节流）。
   - **推论**：受众硬上限 + 拒绝是对症根因的修复；"insert_many 批量"会推翻 #183 刚过终审的每-contact 补偿回滚（高风险重构，对一条 Low 不划算）；"后台 worker 扇出"是过度设计（worker 本就异步消费 pending task，YAGNI）。
2. **圈人是两阶段**：粗筛 `build_segment_coarse_filter`（:31-70）= workspace+account+managed [+ product `$elemMatch`]（product_ids 为空时退化为扫本账号全部 managed contact）；精筛 `contact_matches_segment`（:73-122）在内存做净持有/售后/价值分层过滤，可能大幅缩小。**受众上限加在粗筛层**（限制 Mongo 扫描量），治的正是"全量 contacts 驻内存 + dispatch 串行千次 DB 写"的根因；精筛在内存无法反推粗筛是否超限，故不在精筛层判上限。
3. **Campaign serde 安全**：`Campaign`（models.rs:572-594）`#[serde(rename_all="camelCase")]`，**无 `deny_unknown_fields`**。加 `Option` 字段 + `#[serde(default, skip_serializing_if="Option::is_none")]`（同现有 `target_count` :587-588 范式）→ 旧文档缺字段 serde 补 None，无反序列化破坏。
4. **config 范式 + 锚点**：config.rs 用 `env_or("KEY", "default").parse()?` 范式；已有 `account_daily_send_soft_cap`（默 500，config.rs:580）= 账号日发送软上限，可作单活动受众上限的量级锚点（单活动受众与账号日发能力同量级）。
5. **FindOptions limit 范式**：`FindOptions::builder().limit(N).build()` 是项目标准（decision.rs:226/1394/1441、catalog_rebuild.rs:140 等多处在用）。

## 目标

- KC-04/07：给 campaign 圈人加受众规模硬上限（粗筛层），超限拒绝，治 OOM/请求超时/DB 写洪峰根因。
- KC-06：dispatch 回刷本次命中数到 campaign，消除三义计数误导。

## 架构：两条独立加固

### KC-04 + KC-07 —— 受众规模硬上限（粗筛层 limit + 探测计数）

`resolve_segment_contacts`（campaigns.rs:190-209）加 `max_audience: i64` 参数，粗筛 cursor 加 `.limit(max_audience + 1)` 探测法 + 循环内粗筛计数守卫：

```rust
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
    max_audience: i64,           // 新增：来自 state.config.campaign_max_audience
) -> AppResult<Vec<Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
    // 探测法：多取 1 条，用于区分"正好 max"与"超过 max"。limit 在 Mongo 层封顶
    // 扫描量（防 OOM/全量驻内存）。
    let opts = mongodb::options::FindOptions::builder()
        .limit(max_audience + 1)
        .build();
    let mut cursor = state.db.contacts().find(coarse, opts).await?;
    let active_products: Vec<Product> =
        entitlements::load_active_products(&state.db, workspace_id).await;
    let now = DateTime::now();
    let (mid, high) = value_tier_thresholds(&state.config);
    let mut coarse_count = 0i64;
    let mut hits = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        coarse_count += 1;
        // 超上限：拒绝（发生在建任何 send/task 之前，不产生半批触达）。
        // 在粗筛计数判，而非精筛后——limit 已在 Mongo 层截断，若不在此报错，
        // 运营会误以为圈到的就是全部（实际被 limit 静默截断）。
        if coarse_count > max_audience {
            return Err(AppError::BadRequest(format!(
                "受众粗筛候选超过 {max_audience} 人，请细化圈选条件（产品/阶段/价值分层）后重试"
            )));
        }
        if contact_matches_segment(&c, &active_products, filter, now, mid, high) {
            hits.push(c);
        }
    }
    Ok(hits)
}
```

两调用点传 `state.config.campaign_max_audience`：
- `preview_campaign`（:264-270）：`resolve_segment_contacts(..., campaign.config...)` → 超限 `?` 冒泡返 400。
- `dispatch_campaign`（:333-339）：同上，超限拒绝发生在置 dispatching + 建 send/task 之前。

**语义**：上限 = 粗筛扫描量上限（"匹配粗筛条件的候选 > N 则拒"）。精筛后实际受众可能小于 N。这是有意选择——直接命中 KC-04 根因（全量驻内存 + 串行 DB 写都源于粗筛命中太多），防的正是 OOM/超时。

**安全性质**：纯加保护。当前受众 < max（默 500）时行为完全不变（`account_daily_send_soft_cap` 默 500 佐证生产受众量级在此之下）。绝不改发送正确性、绝不动 #183 补偿回滚逻辑。

### KC-06 —— dispatch 回刷本次命中数（消三义误导）

- Campaign 加字段 `last_dispatch_target_count: Option<i64>`（models.rs，camelCase → `lastDispatchTargetCount`，`#[serde(default, skip_serializing_if="Option::is_none")]`）。
- `dispatch_campaign` 的 completed update（campaigns.rs:425-433）在写 `dispatchedCount` 同时写 `lastDispatchTargetCount: hits.len() as i64`。
- CampaignListItem（:667-681）+ 对应字段透传前端（`#[serde(skip_serializing_if="Option::is_none")]`）。
- 语义自明：`lastDispatchTargetCount`（本次实际命中人数）− `dispatchedCount`（本次去重后新入队数）= 去重跳过数。运营不再误读"有人没发出去"。
- preview 的 `targetCount`（:289）+ report 的 `targetCount`（:523）语义各自清晰，靠这个新字段消歧，不动。

## config 加字段

```rust
// config.rs struct AppConfig：
pub campaign_max_audience: i64,
// 构造处：
campaign_max_audience: env_or("CAMPAIGN_MAX_AUDIENCE", "500").parse()?,
```

加字段须补全**所有 AppConfig 测试构造点**（memory `config_field_add_test_helpers`：漏补 E0063，须 `cargo check --tests` 验证，`--lib` 不编译集成测）。`.env.example` 补 `CAMPAIGN_MAX_AUDIENCE=500` + 文档行。

## 改动面

- **Modify** `src/config.rs`：AppConfig + `campaign_max_audience` 字段 + `env_or` 解析 + 全测试构造点补全。
- **Modify** `src/models.rs`：`Campaign` + `last_dispatch_target_count: Option<i64>` 字段。
- **Modify** `src/routes/campaigns.rs`：`resolve_segment_contacts` 加 `max_audience` 参 + 粗筛计数守卫；`preview_campaign` / `dispatch_campaign` 传 `state.config.campaign_max_audience`；dispatch completed update 回刷 `lastDispatchTargetCount`；`CampaignListItem` 透传新字段。
- **Modify** `.env.example`：+ `CAMPAIGN_MAX_AUDIENCE=500`。

## 测试计划

- **lib 单测（本地可跑，无 Docker）**：config 默认值解析（若有 config 解析测范式则加 `campaign_max_audience` 默 500）。守卫逻辑嵌在 async DB 循环里、无独立纯函数可单测，靠集成测覆盖。
- **集成测（Docker / testcontainers，CI 跑，`#[ignore]`）**：
  - 种 max+1 个粗筛命中的 managed contact → `preview_campaign` 返 400（真回归哨兵：回退 limit/守卫即绿变红）。
  - 种正好 max 个 → preview 成功、targetCount 正确。
  - dispatch 后断言 campaign 文档 `lastDispatchTargetCount == hits.len()`、`dispatchedCount` = 去重后数（KC-06 哨兵）。
  - 若既有 campaign 集成测存在，**扩展而非新建**（memory `feedback_additive_tests`）；用类型化 struct 读回而非纯 raw Document（memory：raw 读回掩盖副作用）。

## 回归风险

1. **KC-04 纯加保护**：受众 < max（默 500）时行为完全不变。
2. **KC-06 纯加字段 + 一处 update 回刷**：不改现有 `dispatchedCount` / `targetCount` 写入。
3. **config 加字段**：漏补测试构造点 = E0063（`cargo check --tests` 拦截）。
4. **既有测试冲击（反过拟合边界）**：`resolve_segment_contacts` 加参数 → 调用点 + 既有测被迫补参（签名变更被迫更新，合规）。若既有测断言"无上限全量圈人"，本修复有意引入上限废除该行为 → 只改被废除的旧断言，绝不为过测试降上限/改逻辑。
5. **check-no-human-takeover lint**：campaigns.rs 在扫描范围；新增错误文案/注释用中性词（受众/圈选/细化/命中），无禁词。
6. **baseline**：`cargo test --lib` ≥ 350 / 0，不触 4 PBT。

## 非目标（YAGNI）

- 不做 insert_many 批量写（会推翻 #183 每-contact 补偿回滚，高风险重构超 Low 范围）。
- 不做后台 worker 扇出（worker 本就异步消费 pending task，过度设计）。
- 不做分页/游标续传（硬上限 + 拒绝已治根因；分页是更大的 UX 改造，超范围）。
- 不改 KC-01/02/03 补偿回滚 + status 门（#183 已过终审）。
- 不改 KC-05 粗筛口径（#188 已修）。
- 不动 report/preview 的既有 targetCount 语义（靠新字段消歧，不重命名既有字段避免前端契约破坏）。
