# main 健康度审查 batch2 修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 main 健康度审查净清单剩余 6 条 finding(CONC-1/2/3 写侧并发硬化 + GATE-1 动作闸复检 + KNOW-2 告警口径 + EVO-3 死字段轻量接线)。

**Architecture:** 全部为既有代码的局部硬化。CONC 系列把"快照 RMW 整体 $set"改为针对性的原子/OCC 写;GATE-1 把动作闸逻辑抽成可复用单元在 revision 后复检;KNOW-2 两行补 filter;EVO-3 让 per-workspace 字段端到端通(针对 default workspace,不动 worker 多租户循环)。

**Tech Stack:** Rust 2021 / Axum / MongoDB(生产 mongo8)。

**来源**:`docs/superpowers/specs/2026-06-26-main-health-audit-batch2-design.md`。每条方案均经改动点完整业务逻辑深度核实定稿,推翻了 CONC-2(弃 pipeline)、CONC-1(不套整个 update)两处原始方案。

## Global Constraints

- baseline:`cargo test --lib` ≥ 350 passed / 0 failed;4 PBT 文件(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)累计 ≥ 33 / 0。
- 禁词 lint(no-human-takeover):`git diff` 新增行(src/agent,src/routes,src/evolution,frontend/src,**含注释**)零命中 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。
- `cargo check --tests -Dwarnings` exit 0(集成 binary 全编译)。
- 既成事实纪律:回复/业务动作成功后的 DB 写失败 → warn 不返 Err(防重发)。
- 精确 git add(列具体文件,**不用 -A / .**)。
- 本地仅 `cargo test --lib` + 单 PBT 文件(磁盘紧,os error 112 风险);集成 `#[ignore]` 套件留 GitHub CI。
- agent-first:不引入词表/关键词匹配。
- 分支自检:每个 Task 开始前 `git rev-parse --abbrev-ref HEAD` 确认在 `fix/main-health-audit-batch2`(并行工作线可能抢 HEAD)。

---

## Task 1: CONC-3 — load_or_create insert 捕获 E11000

**Files:**
- Modify: `src/agent/memory.rs:798-802`(create 分支的 insert_one)
- Test: 同文件 `#[cfg(test)]` 或 `tests/`(集成,见 Step 5)

**Interfaces:**
- Consumes: `crate::agent::escalation::logic::is_duplicate_key_error(&mongodb::error::Error) -> bool`(现成 pub(crate),escalation/logic.rs:355);下方已存在的 find_one 重读块(memory.rs:803-815)。
- Produces: 无新公开接口(行为修复)。

**背景**:`load_or_create_operating_memory` 的 create 分支当前是:
```rust
    state
        .db
        .operating_memories()
        .insert_one(&memory, None)
        .await?;          // ← 并发输者在这里 E11000 透传成 AppError
    state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("operating memory missing after insert".to_string()))
```
首次触达时 webhook(发送前)与后台任务并发 insert,输者 E11000 经 `?` 透传 → 回复客户前整轮 run 失败(不受既成事实保护)。下方 find_one 块已经存在 —— 只需让 dup-key 错误**落到它**而非透传。

- [ ] **Step 1: 写失败测试(集成,验并发双 insert 都返回文档而非 Err)**

在 `tests/` 新建 `tests/operating_memory_insert_idempotent.rs`:
```rust
//! CONC-3:load_or_create_operating_memory 的 create 分支并发 insert 时,
//! 输给唯一索引的一方应回落 find_one 返回赢家文档,而非 E11000 透传失败。
#![cfg(test)]

mod common;

use futures::future::join_all;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB"]
async fn concurrent_first_touch_inserts_all_succeed() {
    let app = common::TestApp::spawn().await;
    let contact = common::seed_managed_contact(&app, "wxid_conc3").await;

    // 同 contact 并发触发 create 分支(首次触达,库里无 operating_memory)。
    let futs = (0..4).map(|_| {
        let state = app.state.clone();
        let contact = contact.clone();
        async move {
            wechatagent::agent::memory::load_or_create_operating_memory(&state, &contact).await
        }
    });
    let results = join_all(futs).await;

    // 全部返回 Ok(同一 contact_wxid 的文档);无一返回 Err。
    for r in &results {
        let mem = r.as_ref().expect("load_or_create 不应因并发 dup-key 失败");
        assert_eq!(mem.contact_wxid, "wxid_conc3");
    }
}
```

> 执行注意:`load_or_create_operating_memory` 的可见性 / `common::seed_managed_contact` 的真实名,执行时 grep 确认(`grep "pub.*fn load_or_create_operating_memory" src/agent/memory.rs`、看 `tests/common/mod.rs` 现有 helper)。若无 seed helper,用现有最接近的 contact 播种方式;若函数非 pub,测试改放 `src/agent/memory.rs` 的 `#[cfg(test)]` 内并用 `super::`。

- [ ] **Step 2: 跑测试确认失败(当前并发会有一方 E11000)**

Run: `cargo test --test operating_memory_insert_idempotent -- --ignored`(需 Docker)
Expected: 无 Docker 时本地不可跑 → 标注此测试由 CI 验证;逻辑失败点是"输者 insert 返回 Err 透传"。

- [ ] **Step 3: 实现 —— insert 命中 dup-key 时不透传,落到 find_one**

把 create 分支的 insert 改为:
```rust
    if let Err(err) = state
        .db
        .operating_memories()
        .insert_one(&memory, None)
        .await
    {
        // CONC-3:首次触达 webhook(发送前)与后台任务并发 create,输给
        // 唯一索引 (workspace_id, account_id, contact_wxid) 的一方收到 11000。
        // 不透传(透传会让回复客户之前整轮 run 失败,且不受既成事实纪律保护),
        // 落到下方既有的 find_one 重读分支返回赢家文档。其余错误仍透传。
        if !crate::agent::escalation::logic::is_duplicate_key_error(&err) {
            return Err(err.into());
        }
    }
    state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("operating memory missing after insert".to_string()))
```

> 执行注意:确认文件顶部 `use` 是否已能解析 `AppError`(原 `.ok_or_else` 已用),无需新增。`err.into()` 把 `mongodb::error::Error` 转 `AppError`——确认 `From<mongodb::error::Error> for AppError` 存在(原 `?` 已隐式用它,故必然存在)。

- [ ] **Step 4: 本地编译 + lib 测试不回归**

Run: `cargo check --tests 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: check 通过;lib ≥ 350 / 0(本改动不碰 lib 测试,数字应与基线一致)。

- [ ] **Step 5: Commit**

```bash
git add src/agent/memory.rs tests/operating_memory_insert_idempotent.rs
git commit -m "fix(memory): load_or_create insert 命中 dup-key 回落 find_one(CONC-3,消首触达并发整轮失败)"
```

---

## Task 2: KNOW-2 — unverified-warning 计数补 status=active

**Files:**
- Modify: `src/agent/knowledge_router.rs:100-112`(total count)、`:119-132`(verified count)
- Test: `tests/`(集成,见 Step 5)

**Interfaces:**
- Consumes: 无。
- Produces: 无新接口(口径修正)。

**背景**:`maybe_emit_unverified_warning` 两处 `count_documents` 不带 status 过滤,而运行时注入口径 `load_operation_knowledge`(同文件 :50 `status="active"`、:70-71 `status="active" AND integrity_status="verified"`)。归档(status≠active)的已核验切片仍计入 verified>0 → :133 提前 return 抑制告警,但它们不被注入 → 运营得不到"有切片却全不可注入"告警。

当前 total count filter(:100-108):
```rust
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
```
当前 verified count filter(:120-128):
```rust
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "integrity_status": "verified",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
```

- [ ] **Step 1: 写失败测试(集成,验归档已核验切片不计入 verified、触发告警)**

在 `tests/` 新建 `tests/unverified_warning_status_scope.rs`:
```rust
//! KNOW-2:maybe_emit_unverified_warning 的 verified 计数须对齐注入口径
//! (status=active AND integrity_status=verified)。归档(status=archived)的
//! 已核验切片不应计入 verified,否则会抑制"有切片却全不可注入"告警。
#![cfg(test)]

mod common;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB"]
async fn archived_verified_chunk_does_not_suppress_warning() {
    let app = common::TestApp::spawn().await;
    let contact = common::seed_managed_contact(&app, "wxid_know2").await;

    // 播 1 条 status=archived + integrity_status=verified 的切片(归档已核验)。
    // 注入口径不会取它(status!=active),但旧 verified count 会把它算进去。
    common::seed_operation_knowledge_chunk(&app, &contact, "archived", "verified").await;

    wechatagent::agent::knowledge_router::maybe_emit_unverified_warning(&app.state, &contact)
        .await
        .expect("warning 计算不应失败");

    // 修复后:active+verified 切片为 0 → 应写 knowledge_unverified_warning 事件。
    let warned = common::count_events(&app, "knowledge_unverified_warning").await;
    assert_eq!(warned, 1, "归档已核验切片不该抑制告警");
}
```

> 执行注意:`maybe_emit_unverified_warning` 是 `pub(crate)`(:92),`tests/` 外部 crate 不可达 → 此测试须改放 `src/agent/knowledge_router.rs` 的 `#[cfg(test)] mod tests` 内(用 `super::`),或暴露一个 `#[cfg(test)]` 薄封装。`seed_operation_knowledge_chunk` / `count_events` 若无现成 helper,执行时按 `tests/common/mod.rs` 现有播种风格自建,或内联 insert_one + count_documents。先 grep 确认现有 helper 清单再决定。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test unverified_warning_status_scope -- --ignored`(需 Docker)
Expected: 修复前 verified count 把归档切片算进去 → verified>0 → 提前 return 不写告警 → 断言 warned==1 失败。无 Docker 时由 CI 验证。

- [ ] **Step 3: 实现 —— 两处 count 补 status=active**

total count filter 改为(加 `"status": "active"`):
```rust
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "status": "active",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
```
verified count filter 改为(加 `"status": "active"`,与 load_operation_knowledge :70-71 完全对齐):
```rust
            doc! {
                "workspace_id": &contact.workspace_id,
                "domain": "user_operations",
                "status": "active",
                "integrity_status": "verified",
                "$or": [
                    { "account_id": null },
                    { "account_id": &contact.account_id }
                ]
            },
```

- [ ] **Step 4: 本地编译 + lib 不回归**

Run: `cargo check --tests 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: check 通过;lib ≥ 350 / 0。

- [ ] **Step 5: Commit**

```bash
git add src/agent/knowledge_router.rs tests/unverified_warning_status_scope.rs
git commit -m "fix(knowledge): unverified-warning 计数补 status=active 对齐注入口径(KNOW-2)"
```

> 若测试放进了 knowledge_router.rs 的 `#[cfg(test)]`,则只 `git add src/agent/knowledge_router.rs`。

---

## Task 3: CONC-2 — commitments $push 治并发丢失

**Files:**
- Modify: `src/agent/gateway.rs:3710-3732`(commitments 块)、`:3828-3832`(大 $set update,移除 commitments key)
- Test: `src/agent/gateway.rs` `#[cfg(test)]`(纯函数,见 Step 1)

**Interfaces:**
- Consumes: `crate::models::{CommitmentRepr, CommitmentEntry}`(models.rs:3955/4010);`crate::agent::types::parse_rfc3339_to_bson`。
- Produces: 一个纯函数 `fn build_commitment_push_update(entry: &crate::models::CommitmentEntry) -> mongodb::bson::Document`,返回 `{ "$push": { "commitments": { "$each": [<entry bson>], "$slice": -8 } } }`。

**背景**:commitments 当前在大 `set_doc` 里:从 run 起始快照 `contact.commitments.clone()` → `already_present` 去重 → push 新 entry → `if len>8 drain(0..drop)` → `set_doc.insert("commitments", bson)`。最后整体 `update_one(doc!{"_id": contact.id}, {"$set": set_doc})`。并发 writer 各自从陈旧快照 append 互相覆盖丢累积项。

**方案**(深度核实定稿):去重保留应用层快照判定(并发下可能写重复,**接受此代价**——planner `pick_commitment_emit_target` 单选 + `commitment_recently_emitted` 按 id 幂等,重复项最多占 cap8 槽位不重复 emit);用 `$push`+`$slice:-8` 治丢失(`$slice:-8` 保留最新 8 条,与 `drain(0..drop)` 丢最旧语义一致)。commitments 从大 `$set` 拆出,仅在有新 entry 时发一次独立 `$push` update。

当前 commitments 块(:3710-3732):
```rust
    if let Some(value) = non_empty_option(&decision.last_commitment) {
        let mut commitments: Vec<crate::models::CommitmentRepr> = contact.commitments.clone();
        let already_present = commitments.iter().any(|c| c.text() == value.as_str());
        if !already_present {
            let mut entry = crate::models::CommitmentEntry::from_plain_text(value.clone());
            if let Some(c) = &decision.commitment {
                if c.text.trim() == value.as_str() {
                    entry.due_at = crate::agent::types::parse_rfc3339_to_bson(&c.due_at);
                }
            }
            commitments.push(crate::models::CommitmentRepr::Structured(entry));
            if commitments.len() > 8 {
                let drop = commitments.len() - 8;
                commitments.drain(0..drop);
            }
        }
        let bson_commitments = mongodb::bson::to_bson(&commitments).unwrap_or(mongodb::bson::Bson::Array(Vec::new()));
        set_doc.insert("commitments", bson_commitments);
    } else if reply_has_time_commitment_feature(&decision.reply_text) {
        // ... 观测事件,不动
    }
```

- [ ] **Step 1: 写失败测试(纯函数,验 $push doc 形态)**

在 `src/agent/gateway.rs` 的 `#[cfg(test)] mod tests`(若无则建)加:
```rust
    #[test]
    fn build_commitment_push_update_shape() {
        let entry = crate::models::CommitmentEntry::from_plain_text("明天回电".to_string());
        let update = super::build_commitment_push_update(&entry);
        let push = update.get_document("$push").expect("有 $push");
        let commitments = push.get_document("commitments").expect("有 commitments");
        // $slice 必须是 -8(保留最新 8 条,丢最旧,与原 drain(0..drop) 语义一致)
        assert_eq!(commitments.get_i32("$slice").unwrap(), -8);
        let each = commitments.get_array("$each").expect("有 $each");
        assert_eq!(each.len(), 1, "一次只追加一条新 entry");
        // entry 序列化为子文档(Structured 形态),text 字段可取
        let entry_doc = each[0].as_document().expect("entry 是子文档");
        assert_eq!(entry_doc.get_str("text").unwrap(), "明天回电");
    }
```

- [ ] **Step 2: 跑测试确认失败(函数未定义)**

Run: `cargo test --lib build_commitment_push_update_shape 2>&1 | tail -5`
Expected: FAIL —— `build_commitment_push_update` 未定义,编译错。

- [ ] **Step 3: 实现纯函数 + 改 commitments 写**

在 gateway.rs 加纯函数(放近 commitments 块或文件的辅助函数区):
```rust
/// CONC-2:构造 commitments 的原子追加 update。`$push`+`$slice:-8` 保证并发
/// writer 各自追加不互相覆盖(治"快照 RMW 丢累积项"),`$slice:-8` 保留最新 8
/// 条(丢最旧,与原 `drain(0..drop)` 语义一致)。去重仍在应用层快照判定(并发
/// 下可能写重复——接受:planner pick_commitment_emit_target 单选 +
/// commitment_recently_emitted 按 id 幂等,重复项最多占槽不重复 emit)。
fn build_commitment_push_update(entry: &crate::models::CommitmentEntry) -> mongodb::bson::Document {
    let entry_bson = mongodb::bson::to_bson(entry)
        .unwrap_or_else(|_| mongodb::bson::Bson::Document(mongodb::bson::Document::new()));
    doc! {
        "$push": {
            "commitments": {
                "$each": [entry_bson],
                "$slice": -8i32,
            }
        }
    }
}
```

把 commitments 块(:3710-3732)的 `if !already_present { ... }` 改为:构造 entry 后**不再** push 进快照 / 不再 insert 进 set_doc,改为收集一个待写的 entry(块外用)。最小改法 —— 在块内直接发独立 update:
```rust
    if let Some(value) = non_empty_option(&decision.last_commitment) {
        // CONC-2:去重仍用快照判定(应用层),命中已存在则不追加。
        let already_present = contact.commitments.iter().any(|c| c.text() == value.as_str());
        if !already_present {
            let mut entry = crate::models::CommitmentEntry::from_plain_text(value.clone());
            if let Some(c) = &decision.commitment {
                if c.text.trim() == value.as_str() {
                    entry.due_at = crate::agent::types::parse_rfc3339_to_bson(&c.due_at);
                }
            }
            // 原子追加:不再走大 set_doc 的整体覆盖($set),避免并发快照 RMW 丢累积项。
            // 既成事实纪律:reply 已送出,此写失败只 warn 不阻断。
            if let Err(e) = state
                .db
                .contacts()
                .update_one(
                    doc! { "_id": contact.id },
                    build_commitment_push_update(&entry),
                    None,
                )
                .await
            {
                tracing::warn!(error = %e, contact_wxid = %contact.wxid, "commitment $push 失败;reply 已送出,跳过");
            }
        }
    } else if reply_has_time_commitment_feature(&decision.reply_text) {
        // ... 观测事件,保持不动
    }
```
并**删除**原 `set_doc.insert("commitments", ...)`(commitments 不再进大 $set)。

> 执行注意:确认此函数体内 `state` / `contact` 在该作用域可见(:3710 上文已在用 `contact.commitments` / `state.db`,故可见)。确认 `doc!` 宏已 import(文件大量使用,必然在)。**关键:确认这段代码位于 `update_one(大 set_doc)` 之前还是之后**——它现在改成独立 update,与 :3828 那次大 $set 是两次写,顺序不影响正确性(不同字段),但要确保 commitments key 已从 set_doc 移除避免双写。

- [ ] **Step 4: 跑测试 + lib 不回归**

Run: `cargo test --lib build_commitment_push_update_shape 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: 新测试 PASS;lib ≥ 350 / 0(若有断言"set_doc 含 commitments"的旧测试会暴露,需相应更新——执行时 grep `set_doc.*commitments` 与相关测试确认)。

- [ ] **Step 5: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "fix(gateway): commitments 改原子 \$push+\$slice(-8) 治并发丢累积项(CONC-2)"
```

---

## Task 4: CONC-1 — memory_card 写拆出走 OCC

**Files:**
- Modify: `src/agent/gateway.rs:4176-4204`(apply_operating_memory_update 末尾)
- Test: `tests/`(集成,见 Step 5)

**Interfaces:**
- Consumes: `crate::agent::memory::{occ_memory_filter, next_memory_card_version}`(memory.rs:632 / :623,均 pub(crate));`effective_memory_card_for_contact` / `memory_card_has_signal` / `effective_memory_card`(同 gateway 作用域已在用)。
- Produces: 无新公开接口。

**背景**:`apply_operating_memory_update` 末尾:
```rust
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if !memory_card_has_signal(&effective_memory_card(memory)) {
        let initial_state = super::decision::initial_operation_state_for_contact(state, contact).await?;
        set_doc.insert("memory_card", to_document(&effective_memory_card_for_contact(memory, contact, &initial_state)).unwrap_or_default());
        set_doc.insert("memory_card_version", next_memory_card_version(memory));
        set_doc.insert("memory_card_updated_at", DateTime::now());
    }
    if decision.consolidation_needed || decision.memory_write_score >= 6 {
        schedule_memory_consolidation_task(state, contact, run_id).await?;
    }
    state.db.operating_memories().update_one(
        doc! { "workspace_id": &contact.workspace_id, "account_id": &contact.account_id, "contact_wxid": &contact.wxid },
        doc! { "$set": set_doc },
        None,
    ).await?;
    Ok(())
```
门控块内写 memory_card(+version),门控外恒写 updated_at + operating_memory_update/context_pack(实际 context_pack 在更上文已合入 set_doc — 执行时确认)。给整个 update 套版本谓词会误拦门控外不 bump version 的写。

**方案**(深度核实定稿):memory_card 三字段从共享 `set_doc` 拆出。门控触发时,这三字段单独用 `occ_memory_filter(ws, acct, wxid, prev_version)` 写一次(镜像 memory.rs:684-729 的现成 OCC 模板:`modified_count==1` 才认、否则跳过)。门控外的 updated_at + 其余字段仍走原三键 filter update,**不动**。

- [ ] **Step 1: 写失败测试(集成,验并发 memory_card 写 lost-race 跳过不报错)**

在 `tests/` 新建 `tests/memory_card_write_occ.rs`:
```rust
//! CONC-1:apply_operating_memory_update 写 memory_card 应走 OCC(版本谓词),
//! 并发输者 modified_count==0 时跳过而非 last-write-wins 覆盖。门控外的字段写不受影响。
#![cfg(test)]

mod common;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB"]
async fn concurrent_memory_card_write_does_not_lose_race_error() {
    let app = common::TestApp::spawn().await;
    // 走完整 gateway run 触发 apply_operating_memory_update 的 memory_card 门控写。
    // 断言:并发两路 run 都不返回 Err(lost-race 静默跳过),且最终 memory_card_version 单调不回退。
    // 具体播种 + 双 run 触发按 tests/ 现有 gateway 集成测试范式(grep tests/ 找 run_user_operation_gateway 调用样例)。
    let _ = &app;
}
```

> 执行注意:这是 gateway 深层路径,集成测试搭建成本高。若 tests/ 已有触发 apply_operating_memory_update 的范式(grep `apply_operating_memory_update` 与 `operating_memories` 在 tests/),复用之;否则此测试作为 `#[ignore]` 骨架 + 详细注释说明验证点,实际并发断言留 CI/后续。**核心交付是 Step 3 的 OCC 写正确性**,由代码审查 + 镜像现成模板保证。

- [ ] **Step 2: 跑测试确认(占位/CI)**

Run: `cargo test --test memory_card_write_occ -- --ignored`(需 Docker)
Expected: 无 Docker 本地跳过;CI 验证。

- [ ] **Step 3: 实现 —— memory_card 拆出走 OCC**

把末尾改为:
```rust
    // CONC-1:memory_card(+version)走 OCC 单独写,镜像 memory.rs 的 occ_memory_filter
    // 模板;门控外的 updated_at / operating_memory_update / context_pack 仍走原三键
    // filter(它们不 bump memory_card_version,不能套版本谓词,否则永久 lost-race)。
    if decision.consolidation_needed || decision.memory_write_score >= 6 {
        schedule_memory_consolidation_task(state, contact, run_id).await?;
    }
    if !memory_card_has_signal(&effective_memory_card(memory)) {
        let initial_state = super::decision::initial_operation_state_for_contact(state, contact).await?;
        let prev_version = memory.memory_card_version;
        let next_version = crate::agent::memory::next_memory_card_version(memory);
        let card_doc = to_document(&effective_memory_card_for_contact(memory, contact, &initial_state)).unwrap_or_default();
        let res = state
            .db
            .operating_memories()
            .update_one(
                crate::agent::memory::occ_memory_filter(
                    &contact.workspace_id,
                    &contact.account_id,
                    &contact.wxid,
                    prev_version,
                ),
                doc! { "$set": {
                    "memory_card": card_doc,
                    "memory_card_version": next_version,
                    "memory_card_updated_at": DateTime::now(),
                    "updated_at": DateTime::now(),
                }},
                None,
            )
            .await?;
        if res.modified_count != 1 {
            // 输给并发 writer:对方已写入更新版本,本次 memory_card 写跳过(不覆盖、不报错)。
            tracing::debug!(contact_wxid = %contact.wxid, "memory_card OCC lost race; skip");
        }
    }
    // 门控外的其余字段(operating_memory_update / context_pack 等)走原三键 filter。
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    // ... 这里保留原本写入 set_doc 的非 memory_card 字段(operating_memory_update / context_pack);
    //     执行时核对 :4176 上文实际往 set_doc 里 insert 了哪些 key,逐字保留,仅移除 memory_card 三键。
    state
        .db
        .operating_memories()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            doc! { "$set": set_doc },
            None,
        )
        .await?;
    Ok(())
```

> **执行关键**:Step 3 的伪代码假定 set_doc 里除 memory_card 三键外还有别的字段。执行前**必须**读 :4173-4204 完整确认:`if decision.operating_memory_update.is_empty() && context_pack.is_empty() { return Ok(()) }`(:4173)早返,以及 set_doc 里到底 insert 了哪些非 memory_card key。若**除 memory_card 三键 + updated_at 外没有其他字段**,则门控不触发时第二次 update 只写 updated_at(可保留,语义不变);若有 operating_memory_update / context_pack 的 insert,逐字保留。不要删任何现有字段写入。

- [ ] **Step 4: 编译 + lib 不回归**

Run: `cargo check --tests 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: check 通过;lib ≥ 350 / 0。

- [ ] **Step 5: Commit**

```bash
git add src/agent/gateway.rs tests/memory_card_write_occ.rs
git commit -m "fix(gateway): memory_card 写拆出走 occ_memory_filter,门控外字段不动(CONC-1)"
```

---

## Task 5: GATE-1 — revision 后复检动作闸

**Files:**
- Modify: `src/agent/gateway.rs:1398-1435`(动作闸块)、`:1686` 后(second_passed 分支)
- Test: `tests/`(集成,见 Step 5)

**Interfaces:**
- Consumes: `load_operation_state_policy_for_contact`、`classify_decision_action`、`enforce_state_action_policy`、`write_event_for_account`(均 gateway 作用域现有);`GatewayStatusFinal`、`DecisionReviewResult`、`AgentDecision`。
- Produces: 一个可复用的 async 辅助 `async fn apply_state_action_gate(...)`,对给定 final_decision/review/finalize_status 做一次动作闸校验,命中 forbidden 时置 held。

**背景**:动作闸(:1398-1435)`enforce_state_action_policy`(全仓仅此一处调用)包在初次 finalize 的 `if matches!(finalize_status, Approved)` 块。revision 块(:1590)在其后,revision 通过后 final_decision 整条替换(:1644),只重跑 finalize_review_for_send(:1666 安全闸),不重跑动作闸 → operation_state 迁入禁 reply 态时绕过。

**方案**:把动作闸逻辑(load policy + classify + enforce + held 处理 + 审计事件)抽成 async fn,初次位置调一次,二次 finalize 的 `second_passed`(:1686)为 Approved 后再调一次。taxonomy 软闸**不抽不复检**(本就有意非阻断)。

- [ ] **Step 1: 写失败测试(集成,验 revision 迁入禁态→held)**

在 `tests/` 新建 `tests/revision_recheck_action_gate.rs`:
```rust
//! GATE-1:single-shot revision 改写后若 operation_state 迁入"禁止 reply"的态,
//! 动作闸须在二次 finalize 后复检,把结果置 held_by_ai_policy(而非放行进 outbox)。
#![cfg(test)]

mod common;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB + 真实 LLM(revision 路径)"]
async fn revision_into_forbidden_state_is_held() {
    // 播种一个 operation_state_policies:把某 state 的 reply 动作列 forbidden。
    // 构造一次会触发 revision 且 revision 后迁入该 forbidden state 的 run。
    // 断言:最终 gateway 状态为 held_by_ai_policy,未进 agent_send_outbox。
    // 具体搭建按 tests/ 现有 operation_state_policies + revision 集成测范式。
    // 此路径依赖真实 LLM(revision 调完整 Reply Agent),作为 #[ignore] CI 验证。
}
```

> 执行注意:revision 路径依赖真实 LLM,难在本地确定性复现。测试作 `#[ignore]` 骨架 + 详细验证点说明,核心交付是 Step 3 的复检逻辑由代码审查保证。若能用 mock decision 在不调 LLM 下触发 second_passed 分支(grep tests/ 找 review_fixed_candidate_for_test 之类的注入点),优先做确定性测试。

- [ ] **Step 2: 跑测试确认(占位/CI)**

Run: `cargo test --test revision_recheck_action_gate -- --ignored`
Expected: CI 验证。

- [ ] **Step 3: 抽 async fn + 二次调用**

读 :1398-1435 完整逻辑后,抽成(放 gateway.rs 私有 fn 区):
```rust
/// GATE-1:终态动作闸 —— 按 contact 当前 operation_state 校验"该状态是否允许本次
/// action"。命中 forbidden 时置 held_by_ai_policy + should_reply=false + 落审计。
/// 初次 finalize 与 single-shot revision 后各调一次(revision 可能迁移 operation_state)。
#[allow(clippy::too_many_arguments)]
async fn apply_state_action_gate(
    state: &AppState,
    contact: &Contact,
    final_decision: &mut AgentDecision,
    review: &mut DecisionReviewResult,
    finalize_status: &mut GatewayStatusFinal,
    run_id: &str,
) -> AppResult<()> {
    let policy_opt = load_operation_state_policy_for_contact(
        state,
        &contact.workspace_id,
        final_decision.operation_state.as_deref().unwrap_or(""),
        &contact.wxid,
    )
    .await?;
    let action = classify_decision_action(final_decision);
    if let Err(reason) = enforce_state_action_policy(policy_opt.as_ref(), action) {
        review.approved = false;
        review.final_review_status = "held_by_ai_policy".to_string();
        final_decision.should_reply = false;
        final_decision.autonomy_mode = "blocked".to_string();
        if !review.risks.iter().any(|r| r == "state_action_policy_blocked") {
            review.risks.push("state_action_policy_blocked".to_string());
        }
        *finalize_status = GatewayStatusFinal::Held("held_by_ai_policy".to_string());
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "state_action_policy_blocked",
            "blocked",
            &reason,
            Some(doc! {
                "run_id": run_id,
                "action": action,
                "operation_state": final_decision.operation_state.clone().unwrap_or_default(),
                "reason": reason.clone(),
            }),
        )
        .await?;
    }
    Ok(())
}
```
初次位置(:1398 块内)替换为调用 `apply_state_action_gate(...)`;taxonomy 软闸(:1437起)留在原 `if Approved` 块**不动**。在二次 finalize 的 second_passed(:1686)判为 Approved 后,加:
```rust
                    if second_passed {
                        // GATE-1:revision 可能迁移 operation_state,对改写后的 final_decision 复检动作闸。
                        apply_state_action_gate(
                            state, &contact, &mut final_decision, &mut review, &mut second_finalize_status, &run_id,
                        ).await?;
                    }
```

> **执行关键**:必须读 :1398-1435 与 :1560-1720 完整上下文,确认:(a) 抽出后初次调用点的变量名(review / final_decision / finalize_status)与函数参数对应;(b) second_passed 之后 final_decision/review/second_finalize_status 的可变性与后续使用 —— apply_state_action_gate 把 second_finalize_status 改成 Held 后,下游 enqueue outbox 的分支必须据此不发送(确认 second_passed 之后到 enqueue 之间如何用 second_finalize_status / review.approved 决定发送)。这是本 Task 最易错处,实现者务必读全 revision 块尾到发送决策的完整链路。

- [ ] **Step 4: 编译 + lib 不回归**

Run: `cargo check --tests 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: check 通过;lib ≥ 350 / 0。

- [ ] **Step 5: Commit**

```bash
git add src/agent/gateway.rs tests/revision_recheck_action_gate.rs
git commit -m "fix(gateway): 动作闸抽 fn,revision 改写后复检 operation_state(GATE-1)"
```

---

## Task 6: EVO-3 — threshold_auto_release_enabled 轻量接线

**Files:**
- Modify: `src/evolution/auto_release.rs:41-48`(开关检查)、`src/routes/evolution.rs:548-556`(请求体)、`:602-608`(PUT $set)、`:689-698`(runtime_flag_json)
- Test: `src/evolution/auto_release.rs` `#[cfg(test)]`(纯函数,见 Step 1)

**Interfaces:**
- Consumes: `crate::evolution::runtime_flag::load_runtime_flag(state, workspace_id) -> AppResult<Option<EvolutionRuntimeFlag>>`(runtime_flag.rs:32);`EvolutionRuntimeFlag.threshold_auto_release_enabled`(models.rs:1220)。
- Produces: 无新公开接口(字段端到端接通)。

**背景**:`auto_release_eligible_thresholds`(:41)只读 env `evolution_auto_release_enabled`(:42)+ 写死 default_workspace_id;PUT 不接受/不写 `threshold_auto_release_enabled`;GET runtime_flag_json 不序列化它。worker 不遍历 workspace(本 batch 不动 —— 真多租户灰度是独立工程)。

**方案**:env `evolution_auto_release_enabled` 保留为全局总闸;总闸开时再读 default workspace 的 `flag.threshold_auto_release_enabled` 作子闸(双闸 AND,镜像 is_evolution_enabled_for 顺序)。PUT 加字段、GET 补输出。

- [ ] **Step 1: 写失败测试(纯函数,验双闸判定)**

把双闸判定抽成纯函数便于测。在 auto_release.rs `#[cfg(test)] mod tests` 加:
```rust
    #[test]
    fn auto_release_dual_gate() {
        // 总闸关 → 不论子闸都 false
        assert!(!super::auto_release_gate_open(false, Some(true)));
        assert!(!super::auto_release_gate_open(false, None));
        // 总闸开 + 子闸关/缺失 → false(默认保守)
        assert!(!super::auto_release_gate_open(true, Some(false)));
        assert!(!super::auto_release_gate_open(true, None));
        // 总闸开 + 子闸开 → true
        assert!(super::auto_release_gate_open(true, Some(true)));
    }
```

- [ ] **Step 2: 跑测试确认失败(函数未定义)**

Run: `cargo test --lib auto_release_dual_gate 2>&1 | tail -5`
Expected: FAIL —— `auto_release_gate_open` 未定义。

- [ ] **Step 3: 实现 —— 纯函数 + auto_release 接线 + PUT/GET 字段**

auto_release.rs 加纯函数:
```rust
/// EVO-3:auto_release 双闸 —— env 全局总闸 AND per-workspace 子闸。
/// 总闸关:整段不跑。总闸开:再看该 workspace 的 threshold_auto_release_enabled
/// (文档缺失视作 None→关,默认保守不自动 release,镜像 is_evolution_enabled_for)。
fn auto_release_gate_open(env_enabled: bool, flag_threshold_enabled: Option<bool>) -> bool {
    env_enabled && flag_threshold_enabled.unwrap_or(false)
}
```
`auto_release_eligible_thresholds`(:42)开头改为:
```rust
    if !state.config.evolution_auto_release_enabled {
        return Ok(0);
    }
    let workspace_id = state.config.default_workspace_id.clone();
    // EVO-3:总闸(env)开后,再读该 workspace 的子闸。子闸关/文档缺失 → 不自动 release。
    let flag_threshold = crate::evolution::runtime_flag::load_runtime_flag(state, &workspace_id)
        .await
        .ok()
        .flatten()
        .map(|f| f.threshold_auto_release_enabled);
    if !auto_release_gate_open(state.config.evolution_auto_release_enabled, flag_threshold) {
        return Ok(0);
    }
```
> 执行注意:保留原 `let account_id` / `cap` / `window_hours` 等后续行不变。`load_runtime_flag` 返回 `AppResult`,这里用 `.ok().flatten()` 把读失败也视作"子闸未开"(保守);若项目惯例是读失败 warn,按惯例调整 —— 但不要让读失败 panic 或透传(auto_release 整体是 best-effort,调用方 run_one_tick 已 unwrap_or_else 兜底)。

PUT 请求体(:548-556)加字段:
```rust
pub(super) struct UpdateRuntimeFlagRequest {
    enabled: bool,
    rollout_percent: u32,
    #[serde(default)]
    updated_by: Option<String>,
    /// EVO-3:per-workspace 自动 release 子闸。None 时不改(保持 upsert 既有值)。
    #[serde(default)]
    threshold_auto_release_enabled: Option<bool>,
}
```
PUT $set(:602-608)在 Some 时写入 —— 因 $set 是固定 doc,改为先构造再条件 insert:
```rust
    let mut set_fields = doc! {
        "workspace_id": &workspace_id,
        "enabled": payload.enabled,
        "rollout_percent": rollout_percent as i64,
        "updated_by": updated_by,
        "updated_at": now,
    };
    if let Some(v) = payload.threshold_auto_release_enabled {
        set_fields.insert("threshold_auto_release_enabled", v);
    }
    state.db.evolution_runtime_flags().update_one(
        doc! { "workspace_id": &workspace_id },
        doc! { "$set": set_fields },
        mongodb::options::UpdateOptions::builder().upsert(true).build(),
    ).await?;
```
GET runtime_flag_json(:690)补输出键:
```rust
    json!({
        "workspaceId": f.workspace_id,
        "enabled": f.enabled,
        "rolloutPercent": f.rollout_percent_clamped(),
        "rolloutPercentRaw": f.rollout_percent,
        "thresholdAutoReleaseEnabled": f.threshold_auto_release_enabled,
        "updatedBy": f.updated_by,
        "updatedAt": datetime_to_rfc3339(f.updated_at),
    })
```

- [ ] **Step 4: 跑测试 + lib 不回归**

Run: `cargo test --lib auto_release_dual_gate 2>&1 | tail -5` 然后 `cargo test --lib 2>&1 | tail -5`
Expected: 新测试 PASS;lib ≥ 350 / 0。

- [ ] **Step 5: Commit**

```bash
git add src/evolution/auto_release.rs src/routes/evolution.rs
git commit -m "feat(evolution): threshold_auto_release_enabled 轻量接线(EVO-3,env总闸+per-workspace子闸)"
```

---

## Task 7: 全量验证 + baseline 门

**Files:** 无改动(验证 only)。

- [ ] **Step 1: 禁词 lint**

Run: `scripts/check-no-human-takeover.sh`(或 .ps1)
Expected: 0 violations(本 batch 新增行不含禁词;CONC-1/2 注释含"承诺/记忆"等正常词,确认不撞 `接管`/`人工`)。

- [ ] **Step 2: lib baseline**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 3: PBT 门**

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter 2>&1 | tail -10`
Expected: 累计 ≥ 33 passed / 0 failed。

- [ ] **Step 4: 集成 binary 全编译**

Run: `cargo check --tests 2>&1 | tail -5`
Expected: Finished,exit 0(含 4 个新建测试文件)。

- [ ] **Step 5: 收尾**

确认所有 Task commit 在 `fix/main-health-audit-batch2` 分支、顺序正确(`git log --oneline main..HEAD`)。无独立 commit(纯验证 Task)。

---

## 非目标(本 batch 明确不做)

- evolution worker 多租户遍历(EVO-3 真灰度上游)——独立工程。
- memory_summary 并发 OCC(接受 last-write-wins,纯文本后续复述自愈)。
- commitments 应用层去重原子化(接受并发重复,planner 单选+id 幂等兜底)。
- taxonomy 软闸 revision 后复检(本就有意非阻断)。
