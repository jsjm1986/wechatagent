# H8 启动砖修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除 `src/db/indexes.rs` 里两处 E5-T1 漏删的旧 unique `create_index`(operation_domain_configs 301-309 / operation_state_policies 310-321),消除多版本数据下 `ensure_indexes` E11000 启动崩溃(boot-brick),并加集成回归测试锁死。

**Architecture:** 这两表的唯一性索引本应由 line 326 `ensure_ops_versioned_indexes` 独家负责(建含 version 的 4-tuple unique + current_version 部分索引)。301/313 的旧 2-key/3-key unique 建完即被该函数 drop,是残留死代码,却带 `.await?` 致命语义卡在多版本数据的启动路径上。删除它们即根治;唯一性由 4-tuple unique 完整保证,无任何读/写路径按旧索引名依赖。

**Tech Stack:** Rust 2021 / Axum / mongodb 2.8 / MongoDB / testcontainers(集成测试)。

## Global Constraints

- 分支:`fix/h8-boot-brick-stale-index`(已从 origin/main 113b57f 切;spec commit b71c323 已在其上)。绝不 push main,只在 worktree `E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure` 干活,不碰主仓根目录。
- cargo 命令前:`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `export CARGO_INCREMENTAL=0`(worktree 共享 target,不设会 clobber test binary)。磁盘紧先删 `target/debug/incremental`。
- 基线不回归:`cargo test --lib` ≥ 350 passed / 0 failed;4 PBT 文件(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)累计 ≥ 33 passed / 0 failed。新测试全 `#[ignore]`,不进 lib 计数。
- 本地只跑 `cargo test --lib` + 编译集成 binary(`--no-run`);多版本 boot-brick 断言留 CI Docker(integration job,`--ignored`)。绝不本地全量 `cargo test`(磁盘 os error 112)。
- 过拟合红线:绝不为过测试改业务逻辑/prompt/guards/阈值。本任务改的是索引创建(删残留死代码)+ 加回归测试,测试证明修复有效 + 唯一性未降级。
- 禁词 lint:indexes.rs / 测试文件不涉禁词(人工/接管/takeover/hand-off),无风险。
- commit:具名 `git add` 指定文件,绝不 `-A`/`.`;commit 消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- **subagent 红线(用户 2026-07-01 点名强调):** 实现时遇到任何不理解的地方,先自己 Read/Grep 读代码、亲自验证,再执行——绝不基于猜测动手。产出必须带 file:line 证据。

---

## 文件结构

- **Modify:** `src/db/indexes.rs` — 删 301-309(operation_domain_configs 旧 unique)+ 310-321(operation_state_policies 旧 unique 含 Phase B/B4 注释)+ 改写 322-325 注释 + 顺修 326 挤行格式。
- **Create:** `tests/ops_versioned_index_boot_brick.rs` — 3 个 `#[ignore]` + Docker 集成回归测试。

两个 task:Task 1 = 源码删除(改 indexes.rs);Task 2 = 回归测试(新文件)。先删源码后加测试,因为测试要验证的正是"删后多版本下不崩"——但注意 Task 2 的测试在旧 bug 下也应能表达意图(见 Task 2 说明)。

---

## Task 1: 删除 indexes.rs 残留旧 unique create_index

**Files:**
- Modify: `src/db/indexes.rs`(删 301-321 两块 + 改 322-325 注释 + 修 326 格式)

**Interfaces:**
- Consumes: 无(纯删除 + 注释)。
- Produces: 改动后 `ensure_all` 里 `operation_domain_configs` / `operation_state_policies` 的索引仅由 `ensure_ops_versioned_indexes(db).await?`(line 326,不改)创建。函数签名不变,`ensure_all(db) -> anyhow::Result<()>` 不变。

- [ ] **Step 1: 动手前先读码验证当前确切内容**

先 Read `src/db/indexes.rs:299-327`,亲自确认下列待删/待改内容与本计划一致(行号可能因上游变动漂移,以 string anchor 为准,不靠行号)。若发现不一致,停下核对,不猜。

待删块 1(operation_domain_configs 旧 unique):
```rust
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
```

待删块 2(operation_state_policies 旧 unique,含前导 Phase B/B4 注释):
```rust
    // Phase B / B4：operation_state_policies 唯一索引——
    //   (workspace_id, domain, state_key) 复合 unique。
    // enforce 路径单次 find_one，索引保命中。
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "state_key": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
```

- [ ] **Step 2: 用 Edit 删除块 1 + 块 2,并改写紧随的 Phase E5-T1 注释,顺修 326 挤行**

用一个 Edit 覆盖 301-326 这段(old_string 取 `db.operation_domain_configs()` 那行开头一直到 `ensure_ops_versioned_indexes(db).await?;    db.operating_memories()` 的挤行处),替换为:

old_string(从块 1 开头到 326 挤行):
```rust
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // Phase B / B4：operation_state_policies 唯一索引——
    //   (workspace_id, domain, state_key) 复合 unique。
    // enforce 路径单次 find_one，索引保命中。
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "state_key": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // Phase E5-T1：ops 三表 active_versions 灰度——
    //   把 (workspace_id, domain[, state_key/value.id]) 旧 unique 索引下线，
    //   换成包含 `version` 的 4-tuple unique，让多版本可同时驻留 collection。
    //   `(..., current_version=true)` 部分索引快路径，给读路径筛 active 集合。
    ensure_ops_versioned_indexes(db).await?;    db.operating_memories()
```

new_string:
```rust
    // Phase E5-T1：operation_domain_configs / operation_state_policies /
    //   system_taxonomies 三表的唯一性索引统一由 `ensure_ops_versioned_indexes`
    //   负责——(workspace_id, domain[, state_key/value.id], version) 4-tuple unique
    //   + (..., current_version=true) 部分索引。这里不再单独建旧的 2-key/3-key
    //   unique:那两处 create_index 会被 ensure_ops_versioned_indexes 立即 drop
    //   掉,且在多版本数据(admin publish 攒下同 (ws,domain[,state_key]) 多 version
    //   行)下建旧 unique 会 E11000 → ensure_indexes 返 Err → 启动崩溃(H8 boot-brick)。
    ensure_ops_versioned_indexes(db).await?;
    db.operating_memories()
```

注意:new_string 把 326 的挤行拆成两行(`ensure_ops_versioned_indexes(db).await?;` 换行后 `db.operating_memories()`)。

- [ ] **Step 3: 编译验证(lib)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo build --lib 2>&1 | tail -15`
Expected: `Finished`,无 error(纯删除 + 注释,不应有编译错;若报 unused import 如 `IndexOptions`,读码确认该 import 是否仍被文件其它索引使用——大概率仍在用,不应触发)。

- [ ] **Step 4: 跑 lib 基线确认不回归**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib 2>&1 | tail -15`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350。

- [ ] **Step 5: Commit**

```bash
git add src/db/indexes.rs
git commit -m "$(cat <<'EOF'
fix(db): 删 ensure_indexes 残留旧 unique 索引(H8 boot-brick 根治)

operation_domain_configs (workspace_id,domain) 2-key unique 与
operation_state_policies (workspace_id,domain,state_key) 3-key unique 是
Phase E5-T1 迁 4-tuple 多版本索引时漏删的残留:建完即被 ensure_ops_versioned_
indexes drop,却带 .await? 致命语义。多版本数据(admin publish 攒下同
(ws,domain[,state_key]) 多 version 行)下,二次启动建旧 unique 撞 E11000 →
main.rs:59 ? → 进程启动崩溃(boot-brick,prod-117 部署炸雷)。

删除两处旧 create_index,唯一性由 ensure_ops_versioned_indexes 的 4-tuple
unique(含 version)独家保证;无任何读/写路径按旧索引名依赖。顺修 326 挤行格式。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: 集成回归测试锁死多版本下 ensure_indexes 不崩

**Files:**
- Create: `tests/ops_versioned_index_boot_brick.rs`
- 参考(只读,勿改):`tests/products_workspace_isolation.rs`(测试范式模板)、`tests/common/mod.rs`(TestApp)

**Interfaces:**
- Consumes: `crate::common::TestApp`(`TestApp::start() -> TestApp`,`app.state.db` 是 `wechatagent::db::Database`);`app.state.db.ensure_indexes().await -> anyhow::Result<()>`;`app.state.db.raw()` 返回 `&mongodb::Database`(直接写 raw BSON,避免构造完整 typed struct)。
- Produces: 3 个 `#[ignore]` 测试,CI integration job 跑。

**说明(为什么用 raw doc 而非 typed struct):** `OperationDomainConfig` / `OperationStatePolicy` 有大量字段;boot-brick 只取决于唯一索引涉及的键(`workspace_id` / `domain` / `state_key` / `version`)。用 `db.raw().collection::<Document>(...)` 只插这几个键即可复现 E11000,精准且不臆造字段。`db/mod.rs` 注释明确 `raw()` 正为集成测试直写 raw BSON 而设。集合名亲验:`operation_domain_configs` / `operation_state_policies`(Read `src/db/mod.rs` 的 `fn operation_domain_configs` / `fn operation_state_policies` 确认 `self.db.collection("...")` 里的字面量,不靠记忆)。

- [ ] **Step 1: 动手前先读码验证**

先 Read 三处,确认与本计划一致(不猜):
1. `tests/common/mod.rs` 里 `TestApp::start()` 的返回结构与 `app.state.db` 访问路径,以及它是否已跑 migrations + ensure_indexes。
2. `src/db/mod.rs` 里 `fn operation_domain_configs` / `fn operation_state_policies` 对应的集合名字面量(传给 `self.db.collection(...)` 的字符串)。
3. `src/db/mod.rs` 里 `fn raw(&self)` 的返回类型。
若集合名/访问路径与下方测试代码不符,以读到的为准修正测试代码。

- [ ] **Step 2: 写测试文件(3 个测试)**

Create `tests/ops_versioned_index_boot_brick.rs`:
```rust
//! H8 回归:多版本数据下 ensure_indexes 不得因残留旧 unique 索引 E11000 boot-brick。
//!
//! 背景:Phase E5-T1 前,operation_domain_configs / operation_state_policies 各有一个
//! 旧 unique(2-key / 3-key)索引由 ensure_all 用 .await? 创建;E5-T1 迁 4-tuple 多版本
//! 索引后这两处 create 是残留(建完即被 ensure_ops_versioned_indexes drop)。多版本数据下
//! 旧 unique create 撞 E11000 → ensure_indexes 返 Err → main.rs:59 ? → 启动崩溃。
//! 删除残留后,唯一性由 4-tuple unique(含 version)独家保证。
//!
//! 全部 #[ignore],需 Docker(testcontainers MongoDB)。
//! CI:`cargo test --test ops_versioned_index_boot_brick -- --ignored`。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, Document};

use crate::common::TestApp;

/// 红线:operation_domain_configs 存在同 (workspace_id, domain) 多 version 行时,
/// 重跑 ensure_indexes 必须成功(旧 bug 下 2-key unique 会 E11000)。
#[tokio::test]
#[ignore]
async fn ensure_indexes_survives_multi_version_domain_configs() {
    let app = TestApp::start().await;
    // TestApp::start 已跑首次 ensure_indexes(空库单 version 底座)。
    let coll = app.state.db.raw().collection::<Document>("operation_domain_configs");
    // 手工插同 (ws, domain) 的第 2 行 version=2,模拟 admin publish 攒下的多版本行。
    coll.insert_one(
        doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "version": 2_i32,
            "current_version": false,
        },
        None,
    )
    .await
    .expect("seed v2 domain config");

    // 模拟二次启动:重跑 ensure_indexes。旧 bug 下这里会 E11000 Err。
    let result = app.state.db.ensure_indexes().await;
    assert!(
        result.is_ok(),
        "多版本 operation_domain_configs 下 ensure_indexes 必须成功,不得 boot-brick,实际 {result:?}"
    );
}

/// 红线:operation_state_policies 存在同 (workspace_id, domain, state_key) 多 version 行时,
/// 重跑 ensure_indexes 必须成功(旧 bug 下 3-key unique 会 E11000)。
#[tokio::test]
#[ignore]
async fn ensure_indexes_survives_multi_version_state_policies() {
    let app = TestApp::start().await;
    let coll = app.state.db.raw().collection::<Document>("operation_state_policies");
    coll.insert_one(
        doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "state_key": "new_contact",
            "version": 2_i32,
            "current_version": false,
        },
        None,
    )
    .await
    .expect("seed v2 state policy");

    let result = app.state.db.ensure_indexes().await;
    assert!(
        result.is_ok(),
        "多版本 operation_state_policies 下 ensure_indexes 必须成功,不得 boot-brick,实际 {result:?}"
    );
}

/// 正向:4-tuple unique 仍挡"重复 version"——同 (ws, domain, version) 两行不合法。
/// 证明删旧 2-key unique 后唯一性没被削弱,只是维度对了(含 version)。
#[tokio::test]
#[ignore]
async fn four_tuple_unique_still_blocks_duplicate_version() {
    let app = TestApp::start().await;
    let coll = app.state.db.raw().collection::<Document>("operation_domain_configs");
    // 先确保 4-tuple unique 已建(TestApp::start 已跑 ensure_indexes;此处再跑一次幂等)。
    app.state.db.ensure_indexes().await.expect("ensure indexes");
    // 插第一行 (ws=dup_ws, domain=dup_domain, version=1)。
    coll.insert_one(
        doc! { "workspace_id": "dup_ws", "domain": "dup_domain", "version": 1_i32, "current_version": true },
        None,
    )
    .await
    .expect("insert first version row");
    // 插完全相同 (ws, domain, version) 的第二行 → 4-tuple unique 必须拒绝。
    let dup = coll
        .insert_one(
            doc! { "workspace_id": "dup_ws", "domain": "dup_domain", "version": 1_i32, "current_version": false },
            None,
        )
        .await;
    assert!(
        dup.is_err(),
        "同 (workspace_id, domain, version) 重复行必须被 4-tuple unique 拒绝(唯一性未降级)"
    );
}
```

- [ ] **Step 3: 编译测试 binary(--no-run,不需 Docker)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --test ops_versioned_index_boot_brick --no-run 2>&1 | tail -20`
Expected: `Finished` + `Executable tests/ops_versioned_index_boot_brick.rs (...)`,无编译错。(本地无 Docker 不跑测试体,只验证编译;`#[ignore]` + Docker 断言留 CI。)

- [ ] **Step 4: 跑 lib 基线确认不回归**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350(新集成测试 `#[ignore]` 不影响 lib 计数,应与 Task 1 Step 4 一致)。

- [ ] **Step 5: Commit**

```bash
git add tests/ops_versioned_index_boot_brick.rs
git commit -m "$(cat <<'EOF'
test(db): H8 回归——多版本数据下 ensure_indexes 不 boot-brick

3 个 #[ignore]+Docker 集成测试:
- ensure_indexes_survives_multi_version_domain_configs:seed 同 (ws,domain)
  多 version 行 → 重跑 ensure_indexes 断言 is_ok(旧 bug 下 2-key unique E11000)
- ensure_indexes_survives_multi_version_state_policies:对称覆盖 3-key
- four_tuple_unique_still_blocks_duplicate_version:正向,证明 4-tuple unique
  仍挡重复 version,唯一性未降级

现有测试全用空库跑 ensure_indexes,故 boot-brick 从未被触发——本测试用 raw doc
seed 多版本行精准复现。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage(逐节核对 spec):**
- §2 方案 A(删 301-309 + 313-321) → Task 1 Step 2 ✓
- §3 改注释 322-325 + 不动 ensure_ops_versioned_indexes → Task 1 Step 2(注释改写)✓;计划全程不碰 855-982 ✓
- §5 测试设计(测试 1/2/3) → Task 2 Step 2 三个测试一一对应 ✓
- §6 范围:313 纳入(Task 2 test 2)✓;system_taxonomies 不碰 ✓
- §7 实现约束 → Global Constraints 全覆盖 ✓

**2. Placeholder scan:** 无 TBD/TODO;每个代码步骤给完整代码;commit 消息完整;无"similar to Task N"。✓

**3. Type consistency:** `ensure_all(db) -> anyhow::Result<()>` 不变;`ensure_indexes().await -> anyhow::Result<()>`(测试断言 `result.is_ok()` 匹配);`raw().collection::<Document>(...)` 返回 `Collection<Document>`,`insert_one(doc!, None).await -> Result<_, Error>`(测试 `.expect()` / `.is_err()` 匹配)。集合名 `operation_domain_configs` / `operation_state_policies` 在 Task 2 Step 1 要求实读 db/mod.rs 确认。✓

**注意:** Task 2 的三个测试在**旧 bug 未修时**(若有人单独跑 Task 2)test 1/2 会失败(E11000)——这是期望的(证明测试是真护栏)。正常执行序 Task 1 先删残留,Task 2 测试即应全绿(CI Docker)。
