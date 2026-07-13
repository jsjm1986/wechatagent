# P3 家族④ campaign 规模就绪债 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 campaign 圈人加受众规模硬上限（粗筛层，超限拒绝，治 OOM/请求超时/DB 写洪峰根因 KC-04/07）+ dispatch 回刷本次命中数消除三义计数误导（KC-06）。

**Architecture:** 两条独立关注点各一 task。KC-04/07：`resolve_segment_contacts` 粗筛 cursor 加 `.limit(max+1)` 探测法 + 循环内粗筛计数守卫，超限返 400，上限来自新 config `campaign_max_audience`；两调用点（preview/dispatch）传 config 值。KC-06：Campaign + `last_dispatch_target_count: Option<i64>` 字段，dispatch completed update 回刷 `hits.len()`，CampaignListItem 透传前端。全部纵深加固/可观测性，不改发送正确性、不动 #183 补偿回滚 / #188 KC-05 口径。

**Tech Stack:** Rust 2021，Axum，MongoDB（`FindOptions::builder().limit()`）。KC-04 守卫逻辑嵌在 async DB 循环、无独立纯函数 → 靠 Docker 集成测（CI）覆盖；config 默认值可 lib 测。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-13-p3-family4-campaign-scale-design.md`（已获批 commit c21a6ee）。所有行号亲验于分支 `fix/p3-family4-campaign-scale`（基于 origin/main `bbc8b7e` 含 #195）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- **不改发送正确性 / 不动 #183 的 KC-01/02/03 补偿回滚 + status 门 / 不改 #188 的 KC-05 粗筛口径**。KC-04 是纯加保护（受众 < max 默 500 时行为完全不变）；KC-06 是纯加字段 + 一处 update 回刷。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为 / 签名变更被迫更新"，绝不为过测试改业务逻辑/降上限。
- **config 加字段的 E0063 铁律**（memory `config_field_add_test_helpers`）：`AppConfig` 加字段必须补全**所有全字段字面量构造点**——已亲验恰 **4 处**需补：`src/config.rs` 的 `from_env` 构造、`src/evolution/budget.rs:61`、`src/routes/evolution.rs:864`、**`tests/common/mod.rs:256 test_config`**（后三处均全字段字面量、无 `..Default` spread；`test_config` 是集成测基础设施，`--lib` 不编译它=盲区，最易漏）。漏补即 E0063，须 `cargo check --tests`（`--lib` 不编译集成测=盲区）。
- **受众上限语义 = 粗筛扫描量上限**（不是精筛后受众数）：`.limit(max+1)` 在 Mongo 层截断扫描量（防 OOM），循环内 `coarse_count > max` 报错（防 limit 静默截断受众）。
- check-no-human-takeover lint 扫 `src/routes/` 新增行禁词；错误文案/注释用中性词（受众/圈选/细化/命中），无禁词。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用，误写主仓会污染他人分支）。
- 本地若撞 LNK1318 PDB（已知 Windows-only 非代码错），`cargo check --lib` / `cargo check --tests` 已足够验证编译；集成测 `#[ignore]` 靠 CI Docker 跑。

## File Structure

- `src/config.rs`：AppConfig struct + `campaign_max_audience: i64` 字段（放在规模/上限相关字段附近，如 `account_daily_send_soft_cap` 旁）；`from_env` 构造补 `env_or("CAMPAIGN_MAX_AUDIENCE","500").parse()?`。
- `src/evolution/budget.rs:61` + `src/routes/evolution.rs:864`：两处全字段测试构造补 `campaign_max_audience: 500`（占位值，这两个测试不关心 campaign）。
- `src/models.rs`：`Campaign` struct + `last_dispatch_target_count: Option<i64>` 字段（`#[serde(default, skip_serializing_if="Option::is_none")]`，放在 `dispatched_count` 旁）。
- `src/routes/campaigns.rs`：`resolve_segment_contacts`（:190-209）加 `max_audience: i64` 参 + 粗筛计数守卫；`preview_campaign`（:264）/`dispatch_campaign`（:333）传 `state.config.campaign_max_audience`；dispatch completed update（:425-433）回刷 `lastDispatchTargetCount`；`CampaignListItem`（:660-686）加字段 + From 映射。
- `.env.example`：+ `CAMPAIGN_MAX_AUDIENCE=500` 文档行。
- 集成测（若既有 campaign 集成测存在则扩展；否则新建 `tests/campaign_scale_guard.rs`）。

Task 1 = KC-04/07（config + resolve_segment_contacts 守卫 + 两调用点 + 集成测），Task 2 = KC-06（model + dispatch 回刷 + CampaignListItem）。两 task 有序：Task 1 先落 config/守卫，Task 2 独立加字段。

---

## Task 1: KC-04/07 —— 受众规模硬上限（config + 粗筛守卫 + 两调用点）

**Files:**
- Modify: `src/config.rs`（AppConfig + `campaign_max_audience` 字段 + `from_env` 构造）
- Modify: `src/evolution/budget.rs:61`（测试构造补字段）
- Modify: `src/routes/evolution.rs:864`（测试构造补字段）
- Modify: `src/routes/campaigns.rs:190-209`（`resolve_segment_contacts` 加参 + 守卫）
- Modify: `src/routes/campaigns.rs:264`（`preview_campaign` 调用点传参）
- Modify: `src/routes/campaigns.rs:333`（`dispatch_campaign` 调用点传参）
- Modify: `.env.example`（+ CAMPAIGN_MAX_AUDIENCE=500）
- Test: 集成测（Docker，见 Step 8-9）

**Interfaces:**
- Consumes: `AppState`（`state.config: AppConfig`，routes/mod.rs:309）；`AppConfig`（config.rs:10）；`SegmentFilter`；`Contact`；`FindOptions`（`mongodb::options::FindOptions`）。
- Produces: `AppConfig.campaign_max_audience: i64`；新签名 `async fn resolve_segment_contacts(state: &AppState, workspace_id: &str, account_id: &str, filter: &SegmentFilter, max_audience: i64) -> AppResult<Vec<Contact>>`。

- [ ] **Step 1: 亲验 config.rs 字段区 + from_env 构造区**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "account_daily_send_soft_cap" src/config.rs`
Expected: 命中 struct 字段声明处（约 :159）+ from_env 构造处（约 :580）。**实现者亲验真实行号后**在这两处附近加新字段（struct 声明 + from_env 构造）。

- [ ] **Step 2: config.rs 加 campaign_max_audience 字段 + 构造**

在 `AppConfig` struct（config.rs）里 `account_daily_send_soft_cap` 字段附近加：

```rust
    /// 单个 campaign 活动的受众规模硬上限（粗筛扫描量）。粗筛命中候选超过此值即拒绝
    /// preview/dispatch，防单请求全量 contacts 驻内存 + dispatch 串行千次 DB 写超时
    /// （KC-04/07）。默认 500，与 account_daily_send_soft_cap 同量级（单活动受众与账号
    /// 日发能力匹配）。env `CAMPAIGN_MAX_AUDIENCE`。
    pub campaign_max_audience: i64,
```

在 `from_env` 构造处 `account_daily_send_soft_cap: ...` 附近加：

```rust
            campaign_max_audience: env_or("CAMPAIGN_MAX_AUDIENCE", "500").parse()?,
```

- [ ] **Step 3: 补全三处全字段测试构造（防 E0063）**

三处全字段 `AppConfig` 字面量各补一行（放在 `account_daily_send_soft_cap` 同区，值填占位 500）：

1. `src/evolution/budget.rs:61` 的 `AppConfig { ... }`：
2. `src/routes/evolution.rs:864` 的 `crate::config::AppConfig { ... }`：
3. `tests/common/mod.rs:256 test_config` 的 `AppConfig { ... }`（**集成测基础设施，最易漏——`account_daily_send_soft_cap: 500` 在其 :325 附近**）：

三处都补同一行：

```rust
            campaign_max_audience: 500,
```

（连同 `src/config.rs` 的 `from_env` 构造 = 共 4 处构造点，Step 2 已含 from_env。）

- [ ] **Step 4: 编译确认 config 加字段无遗漏构造点**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --tests 2>&1 | tail -20`
Expected: `Finished`（无 E0063 missing field）。若报 E0063 指出还有别的构造点，实现者亲验后补全（本计划已亲验恰 3 处，若 CI 环境有差异以编译器为准）。若本地撞 LNK1318（Windows-only），`cargo check --lib` + 人工核对 3 处已补即可。

- [ ] **Step 5: 改 resolve_segment_contacts 加 max_audience 参 + 粗筛守卫**

把 `src/routes/campaigns.rs:189-209`：

```rust
/// 跑两阶段圈人，返回命中的 contacts。粗筛 Mongo + 内存精筛复用 G4。
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> AppResult<Vec<Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
    let mut cursor = state.db.contacts().find(coarse, None).await?;
    let active_products: Vec<Product> =
        entitlements::load_active_products(&state.db, workspace_id).await;
    let now = DateTime::now();
    let (mid, high) = value_tier_thresholds(&state.config);
    let mut hits = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        if contact_matches_segment(&c, &active_products, filter, now, mid, high) {
            hits.push(c);
        }
    }
    Ok(hits)
}
```

替换为（加 `max_audience` 参、cursor `.limit(max+1)` 探测、循环内粗筛计数守卫）：

```rust
/// 跑两阶段圈人，返回命中的 contacts。粗筛 Mongo + 内存精筛复用 G4。
///
/// KC-04/07：受众规模硬上限（粗筛扫描量）。cursor `.limit(max_audience+1)` 在 Mongo
/// 层截断扫描量（防全量 contacts 驻内存）；循环内 `coarse_count > max_audience` 报错
/// （防 limit 静默截断受众——运营会误以为圈到的就是全部）。上限加在粗筛层而非精筛后，
/// 治的正是"全量驻内存 + dispatch 串行千次 DB 写超时"的根因。preview/dispatch 共用。
async fn resolve_segment_contacts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
    max_audience: i64,
) -> AppResult<Vec<Contact>> {
    let coarse = build_segment_coarse_filter(workspace_id, account_id, filter);
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

- [ ] **Step 6: 改 preview_campaign 调用点（campaigns.rs:264）传 max_audience**

把 `preview_campaign` 里（约 :264-270）：

```rust
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
    )
    .await?;
```

替换为（补第 5 参）：

```rust
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
        state.config.campaign_max_audience,
    )
    .await?;
```

- [ ] **Step 7: 改 dispatch_campaign 调用点（campaigns.rs:333）传 max_audience**

把 `dispatch_campaign` 里（约 :333-339）：

```rust
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
    )
    .await?;
```

替换为（补第 5 参）：

```rust
    let hits = resolve_segment_contacts(
        &state,
        &campaign.workspace_id,
        &campaign.account_id,
        &campaign.segment_filter,
        state.config.campaign_max_audience,
    )
    .await?;
```

- [ ] **Step 8: .env.example 补文档**

在 `.env.example` 里 `ACCOUNT_DAILY_SEND_SOFT_CAP` 附近加：

```
# 单个 campaign 活动的受众规模硬上限（粗筛扫描量）。粗筛命中超过此值即拒绝 preview/dispatch。默认 500。
CAMPAIGN_MAX_AUDIENCE=500
```

- [ ] **Step 9: 扩展 campaign 集成测加受众上限守卫哨兵（Docker，#[ignore]）**

**在既有 `tests/campaign_dispatch_integration.rs` 扩展**（append，不改既有 5 个测试弧）。已亲验该文件范式：直调 handler（`dispatch_campaign(State(app.state.clone()), Extension(test_admin(&ws)), Path(cid.to_hex()))`）、`make_contact(ws, acc, wxid)` 建 managed contact、`make_campaign(ws, acc)` 建空 filter campaign、`TestApp::start()` 起 testcontainers Mongo。

**config 注入法**（已亲验）：`AppState` 是 `#[derive(Clone)]`、`config: AppConfig` 是直接持有的 pub 字段（非 Arc），可改：`let mut app = TestApp::start().await; app.state.config.campaign_max_audience = 3;`。故用小上限 + seed 上限+1 个 contact 即可确定性触发超限，无需 seed 501 个。

先补 import（该文件已 `use wechatagent::routes::campaigns::dispatch_campaign;`，加 preview）：

```rust
use wechatagent::routes::campaigns::preview_campaign;
```

新增两个测试（真回归哨兵）：

```rust
/// KC-04/07：受众粗筛候选超过 campaign_max_audience → preview 返 BadRequest（不静默截断受众）。
#[tokio::test]
#[ignore]
async fn preview_rejects_when_coarse_audience_exceeds_max() {
    let mut app = TestApp::start().await;
    app.state.config.campaign_max_audience = 3; // 小上限便于确定性触发
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // seed 4 个 managed contact（> 上限 3）→ 粗筛候选超限
    for wx in ["wx_1", "wx_2", "wx_3", "wx_4"] {
        app.state.db.contacts().insert_one(make_contact(&ws, &acc, wx), None).await.expect("seed contact");
    }
    let result = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await;
    assert!(
        matches!(result, Err(wechatagent::error::AppError::BadRequest(_))),
        "粗筛候选超过上限须 BadRequest（回退守卫即绿变红），实际 {:?}",
        result.map(|r| r.0.clone())
    );
}

/// KC-04/07：粗筛候选正好等于上限 → preview 成功、targetCount == 上限（探测法边界）。
#[tokio::test]
#[ignore]
async fn preview_succeeds_at_exactly_max() {
    let mut app = TestApp::start().await;
    app.state.config.campaign_max_audience = 3;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    // seed 正好 3 个 → 不超限（空 filter：粗筛=精筛，全部命中）
    for wx in ["wx_1", "wx_2", "wx_3"] {
        app.state.db.contacts().insert_one(make_contact(&ws, &acc, wx), None).await.expect("seed contact");
    }
    let resp = preview_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("正好等于上限应成功");
    assert_eq!(resp.0["targetCount"].as_i64(), Some(3), "targetCount 应为 3");
}
```

**注意**：`make_campaign`（该文件 :81-97）是全字段字面量构造 `Campaign` → Task 2 加 `last_dispatch_target_count` 字段后此处也须补 `last_dispatch_target_count: None`（Task 2 Step2 的 `cargo check --tests` 会抓到；此处提前标注供实现者知悉）。

- [ ] **Step 10: 全 lib 测确认无回归**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 11: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/config.rs src/evolution/budget.rs src/routes/evolution.rs src/routes/campaigns.rs tests/common/mod.rs tests/campaign_dispatch_integration.rs .env.example && git commit -m "fix(campaign): 受众规模硬上限(粗筛层limit+计数守卫)防OOM/超时/DB写洪峰 (KC-04/07 P3家族④)"
```

---

## Task 2: KC-06 —— dispatch 回刷本次命中数（消三义误导）

**Files:**
- Modify: `src/models.rs`（`Campaign` + `last_dispatch_target_count` 字段）
- Modify: `src/routes/campaigns.rs:425-433`（dispatch completed update 回刷）
- Modify: `src/routes/campaigns.rs:660-686`（`CampaignListItem` + From 映射）

**Interfaces:**
- Consumes: `Campaign`（models.rs:572）；`hits`（dispatch 内 `Vec<Contact>`，:333）。
- Produces: `Campaign.last_dispatch_target_count: Option<i64>`；`CampaignListItem` 新增同名字段（camelCase `lastDispatchTargetCount`）。

- [ ] **Step 1: Campaign 加字段**

把 `src/models.rs` 的 `Campaign` struct（:589-590）：

```rust
    #[serde(default)]
    pub dispatched_count: i64,
```

改为（在其后加新字段）：

```rust
    #[serde(default)]
    pub dispatched_count: i64,
    /// KC-06：最近一次 dispatch 的粗筛命中人数（回刷）。与 dispatched_count（本次去重后
    /// 新入队数）区分——两者差 = 去重跳过数，消除"targetCount 三义"误导。旧文档缺此字段
    /// serde 补 None（无破坏，Campaign 无 deny_unknown_fields）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_dispatch_target_count: Option<i64>,
```

- [ ] **Step 2: 编译确认 Campaign 加字段无遗漏构造点**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -rn "Campaign {" src/ tests/ && cargo check --tests 2>&1 | tail -20`
Expected: 命中 Campaign 全字段构造点——已亲验 **2 处**：`create_campaign`（campaigns.rs:227，新建活动尚未 dispatch → `None`）+ `tests/campaign_dispatch_integration.rs:81 make_campaign`（测试 helper → `None`）。`cargo check --tests` 报这些构造点缺 `last_dispatch_target_count`（E0063）。实现者在**每个** Campaign 全字段字面量构造处补 `last_dispatch_target_count: None`。补全后 `Finished`。（若 CI 环境报别的构造点，以编译器为准补全。）

- [ ] **Step 3: dispatch completed update 回刷 lastDispatchTargetCount**

把 `src/routes/campaigns.rs:425-433`（dispatch 的 completed update）：

```rust
    state
        .db
        .campaigns()
        .update_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            doc! { "$set": { "status": "completed", "dispatchedCount": dispatched, "updatedAt": DateTime::now() } },
            None,
        )
        .await?;
```

替换为（$set 补 `lastDispatchTargetCount`）：

```rust
    state
        .db
        .campaigns()
        .update_one(
            doc! { "_id": oid, "workspaceId": &admin.current_workspace },
            doc! { "$set": {
                "status": "completed",
                "dispatchedCount": dispatched,
                "lastDispatchTargetCount": hits.len() as i64,
                "updatedAt": DateTime::now(),
            } },
            None,
        )
        .await?;
```

- [ ] **Step 4: CampaignListItem 加字段 + From 映射**

把 `src/routes/campaigns.rs:660-686`（`CampaignListItem` struct + From）：

```rust
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CampaignListItem {
    campaign_id: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_count: Option<i64>,
    dispatched_count: i64,
    created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

impl From<&Campaign> for CampaignListItem {
    fn from(c: &Campaign) -> Self {
        Self {
            campaign_id: c.id.map(|i| i.to_hex()).unwrap_or_default(),
            title: c.title.clone(),
            status: c.status.clone(),
            target_count: c.target_count,
            dispatched_count: c.dispatched_count,
            created_by: c.created_by.clone(),
            created_at: crate::models::dt_to_string(c.created_at),
        }
    }
}
```

替换为（struct + From 各加一行 `last_dispatch_target_count`）：

```rust
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CampaignListItem {
    campaign_id: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_count: Option<i64>,
    dispatched_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_dispatch_target_count: Option<i64>,
    created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

impl From<&Campaign> for CampaignListItem {
    fn from(c: &Campaign) -> Self {
        Self {
            campaign_id: c.id.map(|i| i.to_hex()).unwrap_or_default(),
            title: c.title.clone(),
            status: c.status.clone(),
            target_count: c.target_count,
            dispatched_count: c.dispatched_count,
            last_dispatch_target_count: c.last_dispatch_target_count,
            created_by: c.created_by.clone(),
            created_at: crate::models::dt_to_string(c.created_at),
        }
    }
}
```

- [ ] **Step 5: 编译 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。（若既有 campaign lib 测断言 CampaignListItem 序列化形态，新增 Option 字段 `skip_serializing_if=None` 时不出现在 JSON → 既有断言不受影响；实现者亲验既有 campaign_list_item 测无回归。）

- [ ] **Step 6: 扩展集成测断言 lastDispatchTargetCount（KC-06 哨兵）**

在 `tests/campaign_dispatch_integration.rs` 新增一个测试（append，用类型化 `Campaign` 读回而非 raw Document——memory：raw 掩盖副作用）。已亲验 `Campaign` 已在该文件 `use` 导入：

```rust
/// KC-06：dispatch 成功后回刷 lastDispatchTargetCount == 本次命中人数，与 dispatchedCount
/// （去重后新入队数）区分，消 targetCount 三义误导。
#[tokio::test]
#[ignore]
async fn dispatch_backfills_last_dispatch_target_count() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    let campaign = make_campaign(&ws, &acc);
    let cid = campaign.id.unwrap();
    app.state.db.campaigns().insert_one(&campaign, None).await.expect("seed campaign");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_x"), None).await.expect("seed wx_x");
    app.state.db.contacts().insert_one(make_contact(&ws, &acc, "wx_y"), None).await.expect("seed wx_y");

    dispatch_campaign(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(cid.to_hex()),
    )
    .await
    .expect("dispatch 应成功");

    // 类型化读回 Campaign，断言回刷字段。
    let reloaded = app
        .state
        .db
        .campaigns()
        .find_one(doc! { "_id": cid }, None)
        .await
        .expect("query campaign")
        .expect("campaign exists");
    assert_eq!(
        reloaded.last_dispatch_target_count,
        Some(2),
        "命中 2 人应回刷 lastDispatchTargetCount=2"
    );
    assert_eq!(reloaded.dispatched_count, 2, "首次全新命中 dispatchedCount=2");
}
```

- [ ] **Step 7: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/models.rs src/routes/campaigns.rs tests/ && git commit -m "fix(campaign): dispatch 回刷 lastDispatchTargetCount 消 targetCount 三义误导 (KC-06 P3家族④)"
```

---

## Self-Review 结论

- **Spec coverage**：KC-04+KC-07（受众硬上限粗筛守卫 + config）→ Task 1；KC-06（dispatch 回刷命中数）→ Task 2。三条 finding 全覆盖。设计"非目标 YAGNI"（不 insert_many/不 worker 扇出/不分页/不动 KC-01/02/03/KC-05）在计划里通过"只改指定行"落实。
- **Placeholder scan**：无 TBD/TODO。Step 9（集成测）因依赖既有 campaign 集成测的 TestApp/config-override 范式，明确要求实现者亲验既有范式后择优——非 placeholder，是"亲验真实测试基础设施"的红线要求（守卫逻辑嵌 async DB 循环无纯函数可 lib 测，集成测是唯一确定性覆盖）。
- **Type consistency**：`resolve_segment_contacts` 新签名 5 参在定义（T1 Step5）、两调用点（Step6/7）一致；`campaign_max_audience: i64` 在 config struct/from_env/2 测试构造（T1 Step2/3）+ 传参处（Step6/7）类型一致；`last_dispatch_target_count: Option<i64>` 在 Campaign（T2 Step1）、dispatch update（`hits.len() as i64` → BSON i64，Step3）、CampaignListItem struct+From（Step4）一致；camelCase 落库/序列化键 `lastDispatchTargetCount` 统一。
- **E0063 铁律落实**：config 加字段列了 **4 处**构造补全（from_env + budget.rs:61 + evolution.rs:864 + tests/common/mod.rs:256 test_config，已亲验穷举——test_config 是集成测基础设施最易漏）；Campaign 加字段 Step2 用 `grep src/ tests/ + cargo check --tests` 让编译器穷举构造点（create_campaign + make_campaign 必补 None），不靠记忆。集成测 config 注入用 `app.state.config.campaign_max_audience = 3`（已亲验 AppState derive Clone、config 直接持有非 Arc）。
- **反过拟合**：`resolve_segment_contacts` 加参 → 调用点被迫补参（签名变更被迫更新）。KC-06 纯加字段 `skip_serializing_if=None` 对既有序列化断言无冲击。无为过测试改逻辑。
- **红线合规**：不动 #183 补偿回滚 / #188 KC-05 口径；错误文案中性无禁词；baseline 不回退；worktree 绝对路径。
