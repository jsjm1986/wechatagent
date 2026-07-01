# H1 ingest worker not-due 误刷 last_fetched_at 修复设计

> 日期：2026-07-01
> 分支：`fix/h1-ingest-worker-not-due`（从 H7 合并后的 origin/main 切）
> 来源：终极审判审计 H1 项（UPHELD High）

## 1. 漏洞描述

`src/knowledge_wiki/ingest_worker.rs` 的 auto-ingest worker 每 tick（默认 `INGEST_WORKER_INTERVAL_SECONDS` → 60min）跨 workspace 扫 `status ∈ {active, failing}` 的 `IngestSource`，对每个 source 调 `process_source`。

`process_source`（:88-138）第一步是节流判断 `is_due(src)`（:93）：距上次拉取不足 `schedule_minutes` 就早退。**缺陷**：not-due 早退（:94）和真 304（:102）返回的是**同一个** `SourceOutcome::NotModified`。而 `run_one_round`（:56-58）对 `NotModified` **无条件**调 `mark_success`，`mark_success`（:273-292）第 281 行**无条件** `$set last_fetched_at = now`。

### 触发机理（已亲验）

`is_due`（:140-148）：`last_fetched_at` 为 None → true（从未拉取）；否则 `(now - last_fetched_at) / 60_000 >= schedule_minutes.max(1)`。

设 worker interval=60min、某 source `schedule_minutes=1440`（每天一次）：

1. 首拉成功：`last_fetched_at = T0`。
2. T0+60min，worker tick：`is_due`? elapsed=60 < 1440 → **false** → 早退返 `NotModified` → `mark_success` 把 `last_fetched_at` 刷成 **T0+60**。
3. T0+120min：elapsed 又只有 60（上轮刚把基准推到 T0+60）< 1440 → false → 再刷成 T0+120。
4. …… `last_fetched_at` 被每个 tick 无限往前推，**elapsed 永远只有 ~60min，永远追不上 1440** → **源首拉后再也不会被拉取更新**。

`schedule_minutes` 越大于 worker interval（越想"温柔"地稀疏拉取），越必然中招。只有 `schedule_minutes <= worker_interval_min` 才侥幸不触发。

### 现有测试为何没抓到

`tests/ingest_worker_smoke.rs` 两个场景（:86 成功首拉、:144 失败源）的 seed 都用 `last_fetched_at: None`（:53 注释明写"None → is_due() 恒 true，本轮立即拉取"）→ **两个测试都从 is_due=true 进入，从没走 not-due 分支**。bug 潜伏至今的直接原因。

## 2. 根因

`SourceOutcome::NotModified` 这一个变体**混淆了两个语义不同的结果**：

- **真 304**：真的发了条件 GET、服务器答 `304 Not Modified`（内容自上次 etag 未变）。这是一次**成功的探测**，`last_fetched_at` **应该**刷新（model 注释 models.rs:1688 明确 last_fetched_at = "上次 200/304 时间"）。
- **not-due**：距上次拉取还没到 `schedule_minutes`，**压根没发请求**。这种情况**绝不应该**碰 `last_fetched_at`——碰了就把节流基准往前推，制造上面的死循环。

两者被塞进同一个变体，`run_one_round` 无从区分，于是 not-due 也走了本只该给 304 的 `mark_success`。

## 3. 方案选型

### 方案 A（选定）：拆出 `Skipped` 变体

给 `SourceOutcome` 加一个 `Skipped` 变体，表达"没到点、本轮跳过、未发请求"。

- `process_source` 的 not-due 早退（:94）返回 `SourceOutcome::Skipped`（而非 `NotModified`）。
- 真 304（:102）仍返 `SourceOutcome::NotModified`。
- `run_one_round` 的 match 增加 `Ok(SourceOutcome::Skipped) => {}`（什么都不做，不写任何 DB）；`NotModified` 分支保持原样调 `mark_success`。

**为什么选 A：**
- 语义最清晰：把"没到点"和"到点了但没变"这两件本就不同的事在类型层面分开，编译器强制 `run_one_round` 显式处理新变体。
- 改动最小、最集中：只动 `ingest_worker.rs` 一个文件的枚举定义 + 两处 match/return。
- 304 语义、失败语义、mark_success/mark_failure 逻辑**全部不变**。

### 否决方案 B：把 due 判断上提到 `run_one_round`

在调 `process_source` 前先 `if !is_due(&src) { continue; }`。也能修，但把单 source 的节流职责从 `process_source` 移到调用点，`process_source` 语义被改成"无条件拉取"，节流逻辑分散到两处。当前 `process_source` 自带节流是内聚的，不必拆散。否决。

### 否决方案 C：`mark_success` 内分叉

让 `mark_success` 分"刷全部"和"只刷 etag 不动 last_fetched_at"两条路。治标不治本——not-due 本就不该进 `mark_success` 这条成功路径，在成功函数里判断"其实没成功"是把逻辑塞错层。否决。

## 4. 核心改动

落点：`src/knowledge_wiki/ingest_worker.rs`。

### 4.1 枚举加变体（:80-86）

```rust
enum SourceOutcome {
    /// 没到 schedule_minutes、本轮跳过（未发请求）——绝不刷 last_fetched_at。
    Skipped,
    NotModified,
    Ingested {
        chunk_count: usize,
        etag: Option<String>,
    },
}
```

### 4.2 not-due 早退改返 Skipped（:93-95）

```rust
    if !is_due(src) {
        return Ok(SourceOutcome::Skipped);
    }
```

（真 304 的 `:101-103` `return Ok(SourceOutcome::NotModified)` 不变。）

### 4.3 run_one_round match 加 Skipped 分支（:55-74）

```rust
            match process_source(state, &client, &src).await {
                Ok(SourceOutcome::Skipped) => {}
                Ok(SourceOutcome::NotModified) => {
                    let _ = mark_success(state, &src, 0).await;
                }
                Ok(SourceOutcome::Ingested { chunk_count, etag }) => {
                    let _ = mark_success_with_etag(state, &src, chunk_count, etag).await;
                }
                Err(err) => { /* 不变：mark_failure */ }
            }
```

**不动**：`is_due` 逻辑、`mark_success` / `mark_success_with_etag` / `mark_failure`、304 判定、拉取/解析/落库、failing/disabled 状态机。

## 5. 行为验证（改动后）

| 场景 | 改动前 | 改动后 |
| --- | --- | --- |
| 从未拉取（last=None） | is_due=true→拉取 | 无变化（Ingested/NotModified/失败按真实结果） |
| 到点了、内容未变（真 304） | NotModified→mark_success 刷 last | **不变**：NotModified→mark_success 刷 last（304 是成功探测，该刷） |
| **没到点（not-due）** | **NotModified→mark_success 误刷 last→死循环** | **Skipped→不写 DB→last 不动→下轮 elapsed 正常累积→到点即拉** |
| 拉取失败 | mark_failure | 无变化 |

关键：`schedule_minutes > worker_interval` 的源，改动前首拉后永不更新；改动后 `last_fetched_at` 只在真实拉取/304 时前移，`elapsed` 正常累积，到 `schedule_minutes` 即被拉取。

## 6. 测试设计

新增测试到 `tests/ingest_worker_smoke.rs`（`#[ignore]` + Docker，与现有两场景同档；本地不跑，CI integration job 跑）。

**测试 3 —— not-due 源本轮跳过、last_fetched_at 不被刷（核心红线）：**
- seed 一个 source：`schedule_minutes = 60`，`last_fetched_at = now - 10min`（未到点），记录该值 `t0`。
- 不挂任何 wiremock（not-due 本就不该发请求；若发了请求会因无 mock 失败，反而能佐证"没发请求"）。
- `run_one_round`。
- 断言：reload 后 `last_fetched_at == t0`（**未被刷新**——旧 bug 下会被 mark_success 刷成 now）；`ingest_count` 不变；`status` 仍 active。

真护栏：旧 bug 下 not-due 走 mark_success→last_fetched_at 变 now→断言 `== t0` 失败；修复后 Skipped 不写→断言通过。非 tautology。

**测试 4（可选补强）—— due 源到点仍正常拉取：** seed `schedule_minutes=60`、`last_fetched_at = now - 120min`（已过点），挂 wiremock 200 → `run_one_round` → 断言产 chunk + last_fetched_at 前移。确认 Skipped 改动没误伤到点该拉的路径。

**基线影响：** 新测试全 `#[ignore]`，不进 `cargo test --lib` 计数，lib≥350/0 与 4 PBT≥33/0 不受影响。

## 7. 范围边界

- **不做（YAGNI）**：不改 worker tick 周期机制、不改 `is_due` 算法、不给 mark_success 加参数、不动 304/失败/状态机逻辑、不碰 CRUD handler。只拆一个变体 + 加回归测试。
- **过拟合红线**：新测试锁的是"not-due 不写 DB"这一真实不变量，不为过测试改任何拉取/节流业务逻辑。
- **禁词 lint**：改动不涉禁词（人工/接管/takeover/hand-off）。
- **多租户无关**：改动在 worker 内部单 source 处理逻辑，与 workspace 过滤无关。
