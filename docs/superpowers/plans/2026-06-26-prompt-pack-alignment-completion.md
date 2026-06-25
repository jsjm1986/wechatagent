# Prompt Pack 启动对齐补全（兑现 spec #1）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `ensure_prompt_pack_v2` 真正兑现 spec 承诺「改 spec 重启必生效，不靠版本号」——把"版本盲三分支"重构为"空库分流"，非空库每次启动都跑内容对齐 + archived GC。

**Architecture:** 单文件改动（`src/prompts.rs::ensure_prompt_pack_v2`）。`PROMPT_PACK_VERSION` 常量从"生效闸"降级为"仅 stamp 溯源"；生效判定完全交给 `align_prompt_specs` 的内容比对。`ensure_missing_prompt_templates`（只补缺不比内容）被 align 取代后成死代码，删除。配套 2 个集成测试证明不再版本盲、GC 不停摆。

**Tech Stack:** Rust 2021 / Axum / MongoDB (mongodb crate) / tokio。后端无 workspace、单 crate。

## Global Constraints

- **agent-first**：决策用正向 `==`/`matches!` 匹配，绝不用 `!=` 否定式。
- **归档而非删除**：对齐替换旧行用 `status="archived"`，绝不物理 `delete`。可回溯。
- **执行顺序铁律**：非空库路径必须 `delete_redundant_prompt_data`（清上一轮 archived）**先**、`align_prompt_specs`（产生本轮 archived）**后**。顺序颠倒会让 align 刚归档的行被立刻物理删除，破坏"可回溯"不变量。沿用现有 `Ok(Some)` 分支 prompts.rs:107-108 的同序。
- **evolution / manual 不可动**：`align_prompt_specs` 内部已守（evolution_release 跳过 key + 告警；非 `seeded_by="system"` 行不动）。本计划不改 `align_prompt_specs` 自身逻辑。
- **闭集 status**：只写 `"active"`/`"archived"`/`"draft"`。
- **本地验证**：`RUSTFLAGS=-Dwarnings cargo check --tests` 必 0 err 0 warn（复刻 CI baseline step2）；`cargo test --lib` 必 ≥350/0。集成测试 `#[ignore]` 需 Docker，逻辑断言留 CI。
- **提交纪律**：只 `git add` 指定文件，绝不 `git add -A`；commit message 末尾加 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **分支**：`feat/prompt-pack-alignment`（隔离 worktree `.claude/worktrees/prompt-pack-alignment`），基 cae6393 + spec 增补 3a96b1d。

---

### Task 1: 重构 ensure_prompt_pack_v2 为空库分流 + 删除 ensure_missing_prompt_templates

**Files:**
- Modify: `src/prompts.rs:85-157`（`ensure_prompt_pack_v2` 函数体）
- Delete: `src/prompts.rs:159-203`（`ensure_missing_prompt_templates` 函数，整段删除）

**Interfaces:**
- Consumes: `align_prompt_specs(db, workspace_id, account_id)`（现有 prompts.rs:235，签名 `async fn(&Database, &str, &str) -> AppResult<()>`，逻辑不变）、`delete_redundant_prompt_data(db, workspace_id)`（现有 prompts.rs:461）、`reset_prompt_pack_v2(db, workspace_id, default_account_id)`（现有）。
- Produces: `ensure_prompt_pack_v2(db, workspace_id, default_account_id)` 行为变更——非空库每次走 `delete_redundant` + `align_prompt_specs`，不再依赖版本号 lookup 判生效。

**背景（实现者必读）：** 现状 `ensure_prompt_pack_v2` 按 `prompt_pack_version == PROMPT_PACK_VERSION` 的 lookup 三分支：`Ok(Some)`→`delete_redundant`+`ensure_missing`（**只补缺失 key、绝不比对已存在 key 的内容**，所以改 spec 不 bump 版本不生效）；`Ok(None)`→按 `any_existing` 分 align/reset；`Err`→warn+reset。本任务把它改成：先判 `any_existing`（库里有无任何 prompt_templates 行），空库→reset，非空库→`delete_redundant`+`align_prompt_specs`。版本号 lookup 仅保留在 `Err` 兜底所需的最小形态——实际上新结构连 lookup 都不需要了（生效判定交给 align 内容比对），但 `Err` 兜底（查询失败时 warn+reset）要保留：改用 `any_existing` 的查询结果做 `Err` 兜底触发。

- [ ] **Step 1: 写失败测试（验证 Ok(Some) 等价场景也对齐——见 Task 2，本 Task 先改实现）**

本 Task 是纯重构，其正确性由 Task 2/Task 3 的新测试 + 现有 3 个 align 测试守护。本 Step 跳过独立失败测试，直接进 Step 2 实现（Task 2 会写"版本号匹配仍对齐"的红测）。

> 说明：subagent-driven 要求每 Task 有测试周期。本 Task 的测试周期 = 现有 3 个 align 集成测试（`align_refreshes_drifted_system_row_and_archives_old` 等）在重构后仍编译通过 + Task 2 的 #8 测试转绿。本 Task 的验收 = `cargo check --tests` 0 err 0 warn + `cargo test --lib` 不回归。

- [ ] **Step 2: 改 `ensure_prompt_pack_v2` 函数体**

把 `src/prompts.rs:85-157` 整个函数替换为：

```rust
pub async fn ensure_prompt_pack_v2(
    db: &Database,
    workspace_id: &str,
    default_account_id: &str,
) -> AppResult<()> {
    // spec 为真相的启动对齐：不再用 PROMPT_PACK_VERSION 做"生效闸"，而是按
    // "库里有无任何 prompt_templates 行"分流——
    // - 全新空库 → reset_prompt_pack_v2（首次种四集合：souls/playbook/configs/templates）
    // - 非空库   → delete_redundant（清上一轮 archived）+ align_prompt_specs（逐 key 内容对齐）
    // 生效判定完全交给 align 的 normalize 内容比对，所以改 spec 重启必生效、不靠版本号。
    // 顺序铁律：delete_redundant 先、align 后——否则 align 刚归档的行会被立刻物理删除。
    let any_existing = db
        .prompt_templates()
        .find_one(doc! { "workspace_id": workspace_id }, None)
        .await;
    match any_existing {
        Ok(Some(_)) => {
            // 非空库：清理上一轮归档行（GC），再逐 key 内容对齐。
            delete_redundant_prompt_data(db, workspace_id).await?;
            align_prompt_specs(db, workspace_id, default_account_id).await
        }
        Ok(None) => {
            // 全新空库：首次种四集合。
            reset_prompt_pack_v2(db, workspace_id, default_account_id).await
        }
        Err(error) => {
            // 查询异常（连接抖动、字段错乱等）时进入兜底：
            // 重新种入默认模板，宁可短暂存在重复条目，也要保证模板始终可用。
            // 同步写一条 agent_events 留痕，便于事后排查。
            let summary =
                format!("ensure_prompt_pack_v2 detect query failed, fallback to reseed: {error}");
            let details = doc! {
                "promptPackVersion": PROMPT_PACK_VERSION,
                "error": error.to_string(),
            };
            let _ = db
                .events()
                .insert_one(
                    crate::models::AgentEvent {
                        id: None,
                        workspace_id: workspace_id.to_string(),
                        account_id: default_account_id.to_string(),
                        contact_wxid: None,
                        kind: "prompt_pack_reseed_fallback".to_string(),
                        status: "warn".to_string(),
                        summary,
                        details: Some(details),
                        created_at: DateTime::now(),
                        dedupe_key: None,
                    },
                    None,
                )
                .await;
            reset_prompt_pack_v2(db, workspace_id, default_account_id).await
        }
    }
}
```

- [ ] **Step 3: 删除死代码 `ensure_missing_prompt_templates`**

删除 `src/prompts.rs:159-203` 整个 `ensure_missing_prompt_templates` 函数（含其 doc 行若有）。它唯一的调用点是旧 `Ok(Some)` 分支，已被 `align_prompt_specs` 取代（align 的 step 3 `None => true` + step 4b 覆盖了"不存在则种入"的补缺能力）。

删除后 grep 确认无残留调用：
```bash
grep -rn "ensure_missing_prompt_templates" src/ tests/
```
预期：0 命中。若有命中，说明还有其它调用方——停下来报告（BLOCKED），不要强删。

- [ ] **Step 4: 验证编译 + lib 基线不回归**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests`
Expected: `Finished` — 0 error 0 warning。（删 ensure_missing 后若它真有其它调用方会在此报 E0425，那时回到 Step 3 处理。）

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

- [ ] **Step 5: 提交**

```bash
git add src/prompts.rs
git commit -m "$(cat <<'EOF'
refactor(prompt-pack): ensure_prompt_pack_v2 版本盲三分支→空库分流(兑现#1改spec重启生效)

非空库每次跑 delete_redundant(GC)+align_prompt_specs(内容对齐),不再用
PROMPT_PACK_VERSION 做生效闸;删死代码 ensure_missing_prompt_templates(被align取代)。
顺序铁律 delete_redundant 先 align 后(否则刚归档行被即刻物删)。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 加 #8 回归测试——版本号匹配但内容漂移仍对齐

**Files:**
- Test: `tests/prompt_pack_seeding.rs`（末尾追加）

**Interfaces:**
- Consumes: `wechatagent::prompts::ensure_prompt_pack_v2`、`prompt_specs_for_test()`（现有 pub fn，返回 `Vec<(String, String)>`）、`common::TestApp`、`make_user_template`（文件内 helper）。

**背景（实现者必读）：** 这是终审 #1 的核心回归。与现有测试 `align_refreshes_drifted_system_row_and_archives_old` 的**本质区别**：那个测试在 setup 里特意把 `prompt_pack_version` 改成旧值 `"pre_align_old_pack"` 制造 `Ok(None)` 才触发 align（旧结构 align 只挂 Ok(None)）。本测试**不改版本号**——DB 保持 TestApp 种入的当前 `PROMPT_PACK_VERSION`（即旧结构会走 `Ok(Some)`→ensure_missing→不对齐的场景），直接验证"版本号匹配时内容漂移也被对齐"。若实现回退成版本盲（align 只在 Ok(None) 跑），本测试必失败。

- [ ] **Step 1: 写失败测试**

在 `tests/prompt_pack_seeding.rs` 末尾追加：

```rust
/// 终审 #1 核心回归：版本号匹配（不改 prompt_pack_version）但 system 行内容漂移时，
/// 重跑 ensure_prompt_pack_v2 仍应对齐回 spec。
/// 与 align_refreshes_drifted_system_row_and_archives_old 的区别：那个测试改旧版本号制造
/// Ok(None)；本测试保持当前版本号（旧结构会走 Ok(Some) 不对齐），验证不再版本盲。
#[tokio::test]
#[ignore]
async fn align_refreshes_drift_even_when_pack_version_matches() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    let specs = wechatagent::prompts::prompt_specs_for_test();
    let key = specs.first().expect("at least one spec").0.clone();

    // 关键：不改 prompt_pack_version（保持 TestApp 种入的当前 PROMPT_PACK_VERSION）。
    // 只把该 key 的 current system 行 content 改脏。
    app.state
        .db
        .prompt_templates()
        .update_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true },
            doc! { "$set": { "content": "DRIFT_WHILE_VERSION_MATCHES" } },
            None,
        )
        .await
        .unwrap();

    // 重跑 ensure_prompt_pack_v2——新结构走非空库路径必对齐；旧版本盲结构会走 Ok(Some) 不对齐。
    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // current active 行 content 不再是脏值（被 spec 覆盖）。
    let current = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "current_version": true, "status": "active" },
            None,
        )
        .await
        .unwrap()
        .expect("current row exists");
    assert_ne!(
        current.content, "DRIFT_WHILE_VERSION_MATCHES",
        "版本号匹配时内容漂移也必须被对齐（不再版本盲）"
    );

    // 脏行被归档而非物删。
    let archived = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": &key, "content": "DRIFT_WHILE_VERSION_MATCHES" },
            None,
        )
        .await
        .unwrap();
    assert!(archived.is_some(), "脏行应被归档保留可回溯");
    assert_eq!(archived.unwrap().status, "archived");
}
```

- [ ] **Step 2: 验证编译（本地无 Docker，逻辑断言留 CI）**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests`
Expected: 0 error 0 warning。（确认新测试编译过、helper/符号都在。）

> 本地无 Docker 跑不了 `#[ignore]` 集成测试的运行时断言。Step 1 的红/绿留 CI：`cargo test --test prompt_pack_seeding align_refreshes_drift_even_when_pack_version_matches -- --ignored`。在 Task 1 实现之前此测试逻辑上会失败（版本盲）；Task 1 之后转绿。因为 Task 1 已先实现，CI 上应直接绿。

- [ ] **Step 3: 提交**

```bash
git add tests/prompt_pack_seeding.rs
git commit -m "$(cat <<'EOF'
test(prompt-pack): #8 版本号匹配但内容漂移仍对齐(终审#1核心回归,不再版本盲)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 加 #9 测试（GC 非空库每次跑）+ 全量验证

**Files:**
- Test: `tests/prompt_pack_seeding.rs`（末尾追加 #9）

**Interfaces:**
- Consumes: 同 Task 2 + `make_user_template(workspace, key, status)`（文件内 helper，返回 `PromptTemplate`，`seeded_by="manual"`）。

**背景（实现者必读）：** 验证终审 Minor #3 已修——`delete_redundant_prompt_data`（删 `status="archived"` 行）现在在非空库路径每次启动都跑，不再绑死在已删除的 `Ok(Some)` 分支。手法：预置一条孤立的 `status="archived"` 行，重跑 `ensure_prompt_pack_v2`（不改版本号、走非空库路径），断言该 archived 行被清除。

- [ ] **Step 1: 写失败测试**

在 `tests/prompt_pack_seeding.rs` 末尾追加：

```rust
/// 终审 Minor #3 回归：archived GC（delete_redundant）在非空库路径每次启动都跑。
/// 预置一条孤立 archived 行 → 重跑 ensure（不改版本号，走非空库）→ 该行被清除。
#[tokio::test]
#[ignore]
async fn delete_redundant_runs_on_nonempty_db_each_startup() {
    let app = common::TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();

    // 预置一条孤立的 archived 行（key 不在 spec 中，不参与对齐）。
    let mut archived_row = make_user_template(&workspace, "user.custom.archived_orphan", "archived");
    archived_row.current_version = false;
    app.state
        .db
        .prompt_templates()
        .insert_one(&archived_row, None)
        .await
        .unwrap();

    // 确认预置成功。
    let before = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": "user.custom.archived_orphan" },
            None,
        )
        .await
        .unwrap();
    assert!(before.is_some(), "预置 archived 行应存在");

    // 重跑 ensure_prompt_pack_v2（不改版本号→非空库路径→delete_redundant 跑）。
    wechatagent::prompts::ensure_prompt_pack_v2(&app.state.db, &workspace, &account)
        .await
        .expect("rerun ensure");

    // archived 行应被 GC 清除。
    let after = app
        .state
        .db
        .prompt_templates()
        .find_one(
            doc! { "workspace_id": &workspace, "prompt_key": "user.custom.archived_orphan" },
            None,
        )
        .await
        .unwrap();
    assert!(after.is_none(), "archived 孤立行应被 delete_redundant 在非空库路径清除");
}
```

- [ ] **Step 2: 验证编译**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests`
Expected: 0 error 0 warning。

- [ ] **Step 3: 全量本地基线门**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed。

Run（4 个 PBT 文件，累计 ≥ 33/0）:
```bash
for t in state_transition_pbt memory_card_invariants wiki_chunk_revision_pbt llm_retry_jitter; do cargo test --test $t 2>&1 | grep "test result:"; done
```
Expected: 各文件 0 failed，累计 passed ≥ 33。

Run（禁词 lint）: `bash scripts/check-no-human-takeover.sh`
Expected: `ok: 0 violations`。

- [ ] **Step 4: 提交**

```bash
git add tests/prompt_pack_seeding.rs
git commit -m "$(cat <<'EOF'
test(prompt-pack): #9 archived GC 非空库路径每次跑(终审Minor#3回归)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage（对照 spec 改动 2bis）：**
- 版本盲三分支→空库分流 → Task 1 Step 2 ✅
- `PROMPT_PACK_VERSION` 降级非生效闸 → Task 1（新结构不再用它判生效，仅 Err 兜底 details 里 stamp）✅
- GC 移非空库路径每次跑 → Task 1 Step 2（delete_redundant 在非空库 Ok(Some) 臂）+ Task 3 测试 ✅
- ensure_missing 去留 → Task 1 Step 3 删除 + grep 确认 ✅
- 测试 #8（版本号匹配仍对齐）→ Task 2 ✅
- 测试 #9（GC 每次跑）→ Task 3 ✅
- domain_config/playbook/souls 不纳入 → 本计划不含它们的任务（spec 改动 3 已记理据）✅

**2. Placeholder scan：** 无 TBD/TODO；每个 code step 含完整 verbatim 代码；命令带预期输出。✅

**3. Type consistency：** `ensure_prompt_pack_v2(db, workspace_id, default_account_id)` 三参贯穿；`align_prompt_specs(db, workspace_id, account_id)` 三参（不改）；`make_user_template(workspace, key, status)` 与文件现有 helper 签名一致；`prompt_specs_for_test() -> Vec<(String, String)>` 与现有一致。✅

**顺序铁律核查：** Task 1 Step 2 的非空库臂 `delete_redundant` 先于 `align_prompt_specs`——与 Global Constraints 一致，与现有 prompts.rs:107-108 同序。✅
