# H1 ingest worker not-due 误刷 last_fetched_at 修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 `src/knowledge_wiki/ingest_worker.rs`:not-due(没到 schedule_minutes、未发请求)与真 304 返回同一 `SourceOutcome::NotModified`,导致 `run_one_round` 对 not-due 也 `mark_success` 刷 `last_fetched_at`,worker interval < schedule 时源首拉后永不更新。拆一个 `Skipped` 变体让 not-due 不写 DB。

**Architecture:** `process_source` 的 not-due 早退改返新变体 `SourceOutcome::Skipped`(不触任何 DB);真 304 仍返 `NotModified` → `mark_success`(304 是成功探测,该刷 last_fetched_at)。`run_one_round` 的 match 加 `Skipped => {}`(什么都不做)。改动集中在 ingest_worker.rs 一个文件的枚举定义 + not-due return + match 分支。304/失败/状态机/is_due/mark_* 全部不变。

**Tech Stack:** Rust 2021 / MongoDB(mongodb 2.8)/ tokio worker loop / testcontainers + wiremock(集成回归测试)。

## Global Constraints

- 分支:`fix/h1-ingest-worker-not-due`(已从 H7 合并后的 origin/main 40d8a65 切;spec commit 6f08426 已在其上)。绝不 push main,只在 worktree `E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure` 干活,不碰主仓根目录。
- cargo 命令前:`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `export CARGO_INCREMENTAL=0`(worktree 共享 target,不设会 clobber test binary)。磁盘紧先删 `target/debug/incremental`。
- 基线不回归:`cargo test --lib` ≥ 350 passed / 0 failed;4 PBT 文件累计 ≥ 33 passed / 0 failed。本任务新增的回归测试是 `#[ignore]` + Docker 集成测试(不进 lib 计数),本地只编译(`--no-run`),断言留 CI integration job 跑。
- 本地只跑 `cargo test --lib` + 编译集成 binary(`--no-run`);绝不本地全量 `cargo test`(磁盘 os error 112)。
- 过拟合红线:绝不为过测试改业务逻辑。本任务改的是 worker 结果分类(拆变体,数据完整性 bug)+ 加回归测试;测试锁"not-due 不写 DB"这一真实不变量。
- 禁词 lint:ingest_worker.rs / 测试文件不涉禁词(人工/接管/takeover/hand-off),无风险。
- commit:具名 `git add`,绝不 `-A`/`.`;commit 消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- **subagent 红线(用户 2026-07-01 点名强调):** 实现时遇到任何不理解的地方,先自己 Read/Grep 读代码、亲自验证,再执行——绝不基于猜测动手。产出必须带 file:line 证据。尤其:动手前先 Read `src/knowledge_wiki/ingest_worker.rs` 全文,确认 `SourceOutcome` 枚举定义、`process_source` 的 not-due 早退行、真 304 return 行、`run_one_round` 的 match、`mark_success`/`is_due` 与本计划一致(行号可能漂移,以 string anchor 为准)。

---

## 文件结构

- **Modify:** `src/knowledge_wiki/ingest_worker.rs` — 枚举 `SourceOutcome` 加 `Skipped` 变体 + `process_source` not-due 早退改返 `Skipped` + `run_one_round` match 加 `Skipped => {}` 分支。
- **Modify:** `tests/ingest_worker_smoke.rs` — 追加 2 个 `#[ignore]` + Docker 集成回归测试(not-due 不刷 last_fetched_at 核心红线 + due 到点仍正常拉取的对照)。

单任务:拆变体 + 加回归测试是对同一行为契约("not-due 不写 DB")的一次内聚改动,不可分割评审。TDD 在任务内:先写 not-due 回归测试(旧 bug 下 last_fetched_at 会被刷成 now→断言失败;但本地无 Docker 只编译,红态靠 CI/逻辑推演证明是真护栏),再拆变体修好,一次 commit。

---

## Task 1: 拆 SourceOutcome::Skipped 变体 + not-due 不写 DB + 集成回归测试

**Files:**
- Modify: `src/knowledge_wiki/ingest_worker.rs`
- Modify: `tests/ingest_worker_smoke.rs`

**Interfaces:**
- Consumes: `enum SourceOutcome`(ingest_worker.rs:80,私有);`process_source(state, client, src) -> anyhow::Result<SourceOutcome>`(:88);`run_one_round(state) -> anyhow::Result<()>`(:46,pub,已被 smoke 测试用);`is_due(src) -> bool`(:140,不改);`mark_success(state, src, chunk_count)`(:273,不改);`TestApp`(tests/common/mod.rs)、`IngestSource`(models.rs:1673)。
- Produces: `SourceOutcome` 新增公有(模块内)变体 `Skipped`;`run_one_round` 行为变化(not-due 不再写 DB)。函数签名全不变。

- [ ] **Step 1: 动手前先读码验证(不猜)**

Read `src/knowledge_wiki/ingest_worker.rs` 确认下列与本计划一致(以 string anchor 为准,行号可能漂移):
- `enum SourceOutcome`(约 :80-86):当前只有 `NotModified` 和 `Ingested { chunk_count, etag }` 两个变体。
- `process_source`(约 :88-95):`if !is_due(src) { return Ok(SourceOutcome::NotModified); }` 这个 not-due 早退。
- 真 304(约 :101-103):`if resp.status().as_u16() == 304 { return Ok(SourceOutcome::NotModified); }`。
- `run_one_round`(约 :55-74):match 有 `Ok(SourceOutcome::NotModified) => { let _ = mark_success(...); }` 和 `Ok(SourceOutcome::Ingested {..}) => {...}` 和 `Err(err) => {...}`。
- `mark_success`(约 :273-292):无条件 `$set last_fetched_at = BsonDateTime::now()`。
- `is_due`(约 :140-148):`last_fetched_at` None→true;否则 `(now-last)/60_000 >= schedule_minutes.max(1)`。

再 Read `tests/ingest_worker_smoke.rs` 确认 `ingest_source(...)` helper(约 :43-62,seed 一个 IngestSource,`schedule_minutes: 60`、`last_fetched_at: None`)、`insert_source` / `reload_source` helper、`TestApp::start()` 用法、`app.state.config.default_workspace_id` 访问路径。若与本计划不符,以读到的为准修正,并在 report 记明分歧。

- [ ] **Step 2: 先写失败的 not-due 集成回归测试(TDD 红)**

用 Edit 在 `tests/ingest_worker_smoke.rs` **末尾**追加下面两个测试。它们复用文件顶部已有的 `ingest_source` / `insert_source` / `reload_source` helper 与 `RSS_BODY` 常量、`TestApp`、wiremock imports(Step 1 已确认存在)。

注意 `ingest_source` helper 固定 `last_fetched_at: None`。not-due 测试需要一个**已设过 last_fetched_at 且未到点**的 source,故测试内 seed 后直接用 `update_one` 把 `last_fetched_at` 设成"10 分钟前"(schedule_minutes=60,故未到点)。用 `mongodb::bson::DateTime::from_millis` 构造精确时刻以便断言未被改动。

```rust

/// 场景 3(H1 回归):source 未到 schedule_minutes(not-due)时,run_one_round 必须
/// 跳过、**不刷 last_fetched_at**。旧 bug 下 not-due 与真 304 共用 NotModified→
/// mark_success 无条件把 last_fetched_at 刷成 now→worker interval<schedule 时源
/// 首拉后永不更新。修复后 not-due 返 SourceOutcome::Skipped,不写任何 DB。
#[tokio::test]
#[ignore]
async fn run_one_round_skips_not_due_source_without_touching_last_fetched_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    // 不挂任何 wiremock:not-due 本就不该发请求;若代码错误地发了请求会因无 mock
    // 连接失败,但那条路径也不会走到(is_due=false 早退在发请求之前)。
    let src = ingest_source(&ws, "ing_not_due", "rss", "http://127.0.0.1:1/never".to_string());
    insert_source(&app, &src).await;

    // 把 last_fetched_at 设成 10 分钟前(schedule_minutes=60 → 未到点 not-due),
    // 用固定毫秒时刻以便精确断言未被改动。
    let ten_min_ago_ms = mongodb::bson::DateTime::now().timestamp_millis() - 10 * 60 * 1000;
    let pinned = mongodb::bson::DateTime::from_millis(ten_min_ago_ms);
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": "ing_not_due" },
            doc! { "$set": { "last_fetched_at": pinned } },
            None,
        )
        .await
        .expect("pin last_fetched_at to 10min ago");

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_not_due").await;
    // 核心红线:not-due 源的 last_fetched_at 必须原封不动(旧 bug 下会被刷成 now)。
    assert_eq!(
        reloaded.last_fetched_at.map(|d| d.timestamp_millis()),
        Some(ten_min_ago_ms),
        "not-due 源的 last_fetched_at 不得被 run_one_round 刷新(旧 bug 会刷成 now)"
    );
    assert_eq!(reloaded.ingest_count, 0, "not-due 源不应产 chunk / 累加 ingest_count");
    assert_eq!(reloaded.status, "active", "not-due 源状态不变");

    // 且没落任何 chunk。
    let chunk_count = app
        .state
        .db
        .operation_knowledge_chunks()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count chunks");
    assert_eq!(chunk_count, 0, "not-due 源不应产任何 chunk");
}

/// 场景 4(对照):source 已过 schedule_minutes(due)时,run_one_round 仍正常拉取。
/// 确认拆 Skipped 变体没误伤"到点该拉"的正常路径。
#[tokio::test]
#[ignore]
async fn run_one_round_still_ingests_due_source() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"due-etag\"")
                .set_body_string(RSS_BODY),
        )
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    let src = ingest_source(&ws, "ing_due", "rss", url);
    insert_source(&app, &src).await;

    // 设成 120 分钟前(schedule_minutes=60 → 已过点 due)。
    let two_hours_ago_ms = mongodb::bson::DateTime::now().timestamp_millis() - 120 * 60 * 1000;
    app.state
        .db
        .ingest_sources()
        .update_one(
            doc! { "source_id": "ing_due" },
            doc! { "$set": { "last_fetched_at": mongodb::bson::DateTime::from_millis(two_hours_ago_ms) } },
            None,
        )
        .await
        .expect("pin last_fetched_at to 120min ago");

    run_one_round(&app.state).await.expect("run_one_round ok");

    let reloaded = reload_source(&app, "ing_due").await;
    // due 源被真实拉取:last_fetched_at 前移(> 120min 前的旧值)、产 chunk。
    assert!(
        reloaded.last_fetched_at.map(|d| d.timestamp_millis()).unwrap_or(0) > two_hours_ago_ms,
        "due 源应被拉取,last_fetched_at 前移"
    );
    assert!(reloaded.ingest_count >= 1, "due 源应产 chunk 并累加 ingest_count");
}
```

- [ ] **Step 3: 编译测试 binary(--no-run,证明测试代码可编译)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --test ingest_worker_smoke --no-run 2>&1 | tail -20`
Expected: `Finished` + `Executable tests/ingest_worker_smoke.rs (...)`,无编译错。本地无 Docker 不跑测试体(`#[ignore]` + Docker 断言留 CI)。若报 `doc!` / `DateTime` 未导入,读文件顶部 imports 补齐(文件已 `use mongodb::bson::{doc, DateTime as BsonDateTime}`,`from_millis`/`timestamp_millis` 是 BsonDateTime 方法,`mongodb::bson::DateTime` 全路径即可,无需新 import)。

**红态说明:** 本地无 Docker 无法跑断言。旧 bug 下(变体未拆)not-due 走 mark_success→last_fetched_at 刷成 now→测试 3 的 `assert_eq!(... Some(ten_min_ago_ms))` 会失败。这是真护栏。CI integration job(Docker)会先在旧代码上见红——但本任务 Step 4 立即拆变体修好,正常执行序下 CI 只跑到修复后的绿态。

- [ ] **Step 4: 拆 SourceOutcome::Skipped 变体(TDD 绿·其一)**

用 Edit 改 `enum SourceOutcome`(约 :80-86),加 `Skipped` 变体。

old_string:
```rust
enum SourceOutcome {
    NotModified,
    Ingested {
        chunk_count: usize,
        etag: Option<String>,
    },
}
```

new_string:
```rust
enum SourceOutcome {
    /// 没到 schedule_minutes、本轮跳过(未发请求)——绝不刷 last_fetched_at。
    /// 与 NotModified(真 304,发了条件 GET、内容未变、是一次成功探测)语义区分:
    /// 只有 NotModified 才走 mark_success 刷 last_fetched_at。
    Skipped,
    NotModified,
    Ingested {
        chunk_count: usize,
        etag: Option<String>,
    },
}
```

- [ ] **Step 5: not-due 早退改返 Skipped(TDD 绿·其二)**

用 Edit 改 `process_source` 的 not-due 早退(约 :93-95)。

old_string:
```rust
    if !is_due(src) {
        return Ok(SourceOutcome::NotModified);
    }
```

new_string:
```rust
    if !is_due(src) {
        return Ok(SourceOutcome::Skipped);
    }
```

注意:真 304 的 `if resp.status().as_u16() == 304 { return Ok(SourceOutcome::NotModified); }`(约 :101-103)**保持不变**——304 是成功探测,该刷 last_fetched_at。

- [ ] **Step 6: run_one_round match 加 Skipped 分支(TDD 绿·其三)**

用 Edit 改 `run_one_round` 的 match(约 :55-58),在 `NotModified` 分支前加 `Skipped => {}`。

old_string:
```rust
            match process_source(state, &client, &src).await {
                Ok(SourceOutcome::NotModified) => {
                    let _ = mark_success(state, &src, 0).await;
                }
```

new_string:
```rust
            match process_source(state, &client, &src).await {
                // not-due:本轮没到点、未发请求 → 不写任何 DB(尤其不刷 last_fetched_at,
                // 否则 worker interval<schedule 时把节流基准无限前推,源永不更新)。
                Ok(SourceOutcome::Skipped) => {}
                Ok(SourceOutcome::NotModified) => {
                    let _ = mark_success(state, &src, 0).await;
                }
```

（`Ingested` 与 `Err` 分支不变。）

- [ ] **Step 7: 编译验证(lib + 集成 binary)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo build --lib 2>&1 | tail -8 && cargo test --test ingest_worker_smoke --no-run 2>&1 | tail -8`
Expected: 两条都 `Finished`,无 error。match 现已穷尽(Skipped/NotModified/Ingested/Err 全覆盖),不应有 non-exhaustive 报错;`Skipped` 变体已被构造(Step 5)和匹配(Step 6),无 dead_code 警告。

- [ ] **Step 8: 跑 lib 基线确认不回归**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350(新增集成测试 `#[ignore]` 不进 lib 计数,N 应与改前一致)。

- [ ] **Step 9: Commit**

```bash
git add src/knowledge_wiki/ingest_worker.rs tests/ingest_worker_smoke.rs
git commit -m "$(cat <<'EOF'
fix(ingest): not-due 源不刷 last_fetched_at,拆 SourceOutcome::Skipped(H1)

process_source 的 not-due 早退与真 304 共用 SourceOutcome::NotModified,
run_one_round 对 NotModified 无条件 mark_success 刷 last_fetched_at。worker
interval(默认 60min)< schedule_minutes 时,每 tick not-due 都把 last_fetched_at
往前推 → is_due 的 elapsed 永远追不上 schedule → 源首拉后永不再更新。schedule
越大越必中招。现有 smoke 测试都从 last_fetched_at=None(is_due 恒 true)进入,
从没走 not-due 分支,故潜伏至今。

拆 SourceOutcome::Skipped 变体表达"没到点、未发请求":not-due 返 Skipped,
run_one_round 对 Skipped 什么都不做(不写 DB)。真 304 仍返 NotModified 走
mark_success(304 是成功探测,该刷 last_fetched_at)。304/失败/状态机/is_due/
mark_* 逻辑全不变。加 2 个集成回归:not-due 不刷 last_fetched_at(核心红线)+
due 到点仍正常拉取(对照)。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage(逐节核对 spec):**
- §3 方案 A(拆 Skipped)→ Task 1 Step 4 ✓
- §4.1 枚举加变体 → Step 4 ✓;§4.2 not-due 返 Skipped → Step 5 ✓;§4.3 run_one_round 加 Skipped 分支 → Step 6 ✓
- §4 "不动 is_due/mark_*/304/失败/状态机" → Step 5 明确 304 不变、Step 6 明确 Ingested/Err 不变;计划全程不碰 is_due/mark_success/mark_failure 定义 ✓
- §6 测试(not-due 不刷 last + due 对照)→ Step 2 两个测试 ✓
- §7 YAGNI/过拟合/禁词/多租户无关 → Global Constraints 覆盖 ✓

**2. Placeholder scan:** 无 TBD/TODO;每个改动步骤给完整 old/new_string;两个测试给完整代码;commit 消息完整。✓

**3. Type consistency:** `SourceOutcome::Skipped` 无字段(match 用 `Skipped => {}`);`process_source -> anyhow::Result<SourceOutcome>` 签名不变(返 `Ok(SourceOutcome::Skipped)` 类型匹配);`run_one_round -> anyhow::Result<()>` 不变;测试用 `mongodb::bson::DateTime::from_millis(i64) -> DateTime` + `.timestamp_millis() -> i64`(BsonDateTime 真实方法);`reload_source` 返 `IngestSource`,`.last_fetched_at: Option<DateTime>`(models.rs:1690),`.map(|d| d.timestamp_millis())` 得 `Option<i64>`,与 `Some(ten_min_ago_ms)` 断言同型 ✓。

**注意(留给 SDD controller):** Task 1 的 not-due 测试在**变体未拆时**(Step 2 后)于 Docker 上会失败(last 被刷成 now)——预期红态,Step 4-6 修后转绿。本地无 Docker 只编译验证(Step 3/7);断言真值留 CI integration job。commit(Step 9)在 lib 基线绿 + 集成 binary 编译过后进行。

