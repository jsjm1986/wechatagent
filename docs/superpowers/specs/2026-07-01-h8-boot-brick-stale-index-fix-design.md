# H8 启动砖(boot-brick)——ensure_indexes 残留旧 unique 索引修复设计

> 日期：2026-07-01
> 分支：`fix/h8-boot-brick-stale-index`（off origin/main 113b57f）
> 来源：终极审判审计 H8 项（`indexes.rs:301` boot-brick）
> 前置核实：worktree HEAD = origin/main = 113b57f（无漂移）；已亲自读码 + Explore subagent(opus) 交叉核查，纠正原描述偏差。

## 1. 漏洞描述

`src/db/indexes.rs` 的 `ensure_all`（由 `Database::ensure_indexes` 调用，`main.rs:59` 启动期 `?` 致命）内存在两处**自相矛盾**的索引创建：

| 行号 | 集合 | 创建的索引 | 失败语义 |
| --- | --- | --- | --- |
| 301-309 | `operation_domain_configs` | `(workspace_id, domain)` 2-key **unique** | `.await?`（致命） |
| 313-321 | `operation_state_policies` | `(workspace_id, domain, state_key)` 3-key **unique** | `.await?`（致命） |

紧接着 line 326 调用的 `ensure_ops_versioned_indexes`（855-982）**立即**把这两个旧 unique `drop_index` 掉（best-effort，`let _ =` 吞错），换成含 `version` 的 4-tuple unique + `current_version` 部分索引：

- `op_domain_ws_domain_version_unique` = `(workspace_id, domain, version)`（indexes.rs:861-874）
- `op_state_policy_ws_domain_state_version_unique` = `(workspace_id, domain, state_key, version)`（indexes.rs:895-913）

即：**301/313 建的旧 unique 索引，在同一次 `ensure_all` 调用内建完即被 326 drop 掉**。它们是 Phase E5-T1 迁移到 4-tuple 多版本索引时**漏删的残留死代码**。

### boot-brick 触发机理（精确）

Phase E5-T1 的多版本特性让这两表可以有同 `(workspace_id, domain[, state_key])` 的**多个 version 行**。真实写入路径：`routes/admin_ops_versions.rs:370 publish_state_machine_version`（及 `publish_operation_domain_version` / `publish_operation_state_policy_version` / rollout / rollback）——`next_version_for_scope`（取现存 max+1）分配新 version，`insert_new_current_domain_config` 先 insert 新 current 行、再 demote 其余，同 scope 遗留多行不同 version。

一旦库里存在这样的多版本行，**下次启动**：
1. line 301/313 建 2-key/3-key unique → 同 `(ws, domain[, state_key])` 多行触发 **E11000 duplicate key**
2. `.await?` 抛错 → `ensure_all` 返 `Err` → `ensure_indexes` 返 `Err` → **`main.rs:59` `?` → 进程启动崩溃**

旧索引根本活不到 line 326 被 drop——它在建的那一刻就把启动害死了。

**触发时机（三场景）：**
- 首次启动（空库 / migrations 只 seed 单 version 底座）：`(ws,domain)` 唯一，301/313 建 unique 成功 → **不触发**。
- 二次启动 + 从没 admin publish 过：仍单 version → **不触发**。
- **二次及以后启动 + 期间发生过 admin publish/rollout（攒下多 version 行）：E11000 → boot-brick**。这正是 prod-117 生产部署的典型场景：跑一段时间、admin 发布过新状态机版本后重启即炸。

### 反讽铁证

`admin_ops_versions.rs:347-349` 的 judgment-call 注释明确写道：

> `current_version=true` 唯一分区索引会强制 demote-then-insert 顺序，且 `ensure_indexes` 用 `?` 非 best-effort，存量脏 current 行会**直接 brick 启动**（prod-117 部署炸雷），无配套清理不可加。

团队**已知**"在 `ensure_indexes` 里加会撞存量数据的 unique = 启动炸雷"这个陷阱，却漏掉了既存的 301/313 这两处旧 unique create。line 368 同时确认 `(workspace, domain, version)` 4-tuple unique 是挡"重复 version"竞态的正解。

### 根因

E5-T1 迁移到 4-tuple 多版本索引时，**漏删了 301-309 / 313-321 这两处应被淘汰的旧 unique `create_index`**。它们带着 `.await?` 致命语义卡在多版本数据的必经启动路径上。

## 2. 方案选型

### 方案 A（选定）：删除残留旧 create_index

删掉 301-309 和 313-321 两段，让这两表的索引由 line 326 `ensure_ops_versioned_indexes` **单一来源**负责（它已完整承担 drop 旧索引 + 建 4-tuple unique + current_version 部分索引）。同步清理 322-325 的过时注释（它描述的正是被删逻辑）。

**为什么选 A：**
- 根治零残留：删的是"建完即被自己 drop"的死代码，消除自相矛盾。
- 与 E5-T1 迁移原意一致：4-tuple 取代 2-key/3-key，本就该只保留 4-tuple。
- 唯一性不降级：真正的约束是 4-tuple unique（含 version），完整覆盖多版本需求；无任何读/写路径按旧索引名依赖（全仓 grep `workspace_id_1_domain_1` / `workspace_id_1_domain_1_state_key_1` 只出现在 drop_index 处）。
- blast radius 最小：只动一个文件的两段删除 + 一处注释。

**否决方案 B（保留 301/313 但降级不硬失败）：** 把旧 unique 改非 unique，或 `.await?` → `let _ =`。仍保留"建一个马上被 drop 的索引"这种无意义冗余动作，治标不治本、留死代码。否决。

**否决方案 C（启动头先 drop 再建）：** 在 `ensure_all` 开头先 drop 两个旧 unique 再走原流程。堆叠更多启动期索引操作、可读性下降，且与 `ensure_ops_versioned_indexes` 的 drop 重复。否决。

## 3. 核心改动

落点：`src/db/indexes.rs` `ensure_all` 函数。

**删除段 1（301-309）：**
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

**删除段 2（313-321）：**
```rust
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

**改注释（322-325）：** 原注释描述"把旧 unique 下线换 4-tuple"，删除旧 create 后悬空。改成简明说明：`operation_domain_configs` / `operation_state_policies` / `system_taxonomies` 三表的唯一性索引统一由 `ensure_ops_versioned_indexes` 负责（4-tuple unique + current_version 部分索引），此处不再单独建旧 2-key/3-key unique。

**不动：**
- `ensure_ops_versioned_indexes`（855-982）—— 一字不改，已独家负责这两表索引。
- system_taxonomies 相关（`ensure_system_taxonomies_indexes` 824-838 只建非 unique；`ensure_ops_versioned_indexes` 934-980 的 drop+4tuple）—— 已确认无对称 boot-brick（见 §6）。

## 4. 三场景行为验证（改动后）

| 场景 | 数据 | 改动前 | 改动后 |
| --- | --- | --- | --- |
| A 首次启动 | 空库 / 单 version | 301/313 建 unique 成功→326 drop→建 4-tuple | 直接 326：drop 找不到旧索引→`let _` 吞 IndexNotFound(27)→建 4-tuple。**正常** |
| B 二次启动/单 version | 每 scope 一行 | 不撞重复 | 无变化 |
| C 二次+多 version | admin publish 攒下多行 | **E11000→boot-brick** | 跳过旧 unique；4-tuple 含 version 不冲突。**启动正常** |

**drop_index 安全性（已亲验）：** `ensure_ops_versioned_indexes` line 857-860 / 891-894 用 `let _ = ...drop_index(...).await;`，返回值整个丢弃（含 Err）。目标索引不存在时 Mongo 返回 IndexNotFound(code 27)被吞，不阻塞。删 301/313 后首次启动 drop 找不到旧索引 = 静默 noop，不会崩。

**唯一性保证不降级：**
- 真正的唯一约束 = 4-tuple unique（含 version），挡住"重复 version"这个真正危险的竞态（admin_ops_versions.rs:368 确认）。
- "至多一条 current_version=true" **本就不由任何 unique 索引保证**（admin_ops_versions.rs:341-369 明确 judgment call：不加 current_version 唯一分区索引，理由正是会 brick 启动）。删除旧 2-key/3-key unique 不改变这点。

## 5. 测试设计

**问题根源：** 现有调用 `ensure_indexes()` 的测试全用**空库**——`tests/common/mod.rs` `TestApp::start()` 每次独立 UUID 空库，migrations 只 seed 单 version 底座，`ensure_indexes` 时无多版本数据 → 301/313 建 unique 成功 → boot-brick 从未被触发。这是 bug 潜伏至今的直接原因。

**新增：** `tests/ops_versioned_index_boot_brick.rs`（`#[ignore]` + Docker testcontainers，与 H3 集成测试同档；本地不跑，CI integration job 跑）。

**测试 1 —— operation_domain_configs 多版本下 ensure_indexes 不崩（核心红线）：**
```
1. TestApp::start()  // migrations + 首次 ensure_indexes(空库单 version 底座)
2. 手工 insert operation_domain_configs 第 2 行:同 (workspace_id, domain), version=2
   (模拟 admin publish 攒下的多版本行)
3. app.state.db.ensure_indexes().await   // 模拟二次启动
4. assert!(result.is_ok(), "多版本数据下 ensure_indexes 必须成功,不得 E11000 boot-brick")
```
真护栏：旧 bug 下第 3 步必 E11000 Err → 断言失败；修复后 Ok。非 tautology。

**测试 2 —— operation_state_policies 对称覆盖：** seed 同 `(ws, domain, state_key)` 多 version 行 → 重跑 `ensure_indexes` → assert `is_ok`。锁死 313 那处。

**测试 3（正向）—— 4-tuple unique 仍挡重复 version：** seed 两行完全相同 `(ws, domain, version)` → 重跑 `ensure_indexes` 建 4-tuple unique → 断言唯一约束仍生效（建索引 Err 或写入被拒），证明唯一性没被削弱、只是维度对了。

**基线影响：** 新测试全 `#[ignore]`，不进 `cargo test --lib` 计数，lib≥350/0 与 4 PBT≥33/0 不受影响。

## 6. 范围边界

**operation_state_policies（313）纳入本次修复：** 与 301 是同一个 E5-T1 残留 bug 的对称面——同样先建旧 unique（`.await?` 致命）、随后被 `ensure_ops_versioned_indexes` drop 换 4-tuple，同样会在多版本数据下 E11000 boot-brick。一次修干净。

**system_taxonomies 不在范围（已核实，与原描述不同）：** `ensure_system_taxonomies_indexes`（824-838）**只建非 unique** 辅助索引（`sys_tax_scope_kind_status_idx`），**不再 create** 旧的 `scope_1_kind_1_value.id_1` unique（注释 820-823 说明该 unique 已迁到 `ensure_ops_versioned_indexes` 的 4-tuple）。因此 system_taxonomies 当前**没有任何代码创建旧 unique**，无与 301/313 同型的 boot-brick——它已完成迁移，只在 `ensure_ops_versioned_indexes:939-942` 留一个 best-effort drop 残留（无害）。这与 ops 二表的本质区别：**301/313 仍在主动 `create` 旧 unique（`.await?` 致命），sys_tax 早已删除对应 create。**

**不做（YAGNI）：** 不动 `ensure_ops_versioned_indexes` 逻辑、不动 system_taxonomies、不加 MongoDB 事务、不加 `current_version=true` 唯一分区索引（admin_ops_versions.rs:341-369 已论证在本部署形态下不可加）。只删残留 + 加回归测试。

## 7. 实现约束

- **分支：** 从最新 main（113b57f）切 `fix/h8-boot-brick-stale-index`。绝不 push main，只在 worktree `e4-f21-closure` 干活，不碰主仓根目录。
- **改动文件：** `src/db/indexes.rs`（删两段 + 改注释）+ 新增 `tests/ops_versioned_index_boot_brick.rs`。仅此两个。
- **验证：** 本地 `cargo test --lib`（`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `CARGO_INCREMENTAL=0`）+ 编译新集成 binary（`--no-run`）确认无编译错；多版本 boot-brick 断言留 CI Docker 跑。磁盘紧时先删 `target/debug/incremental`。
- **基线：** lib≥350/0、4 PBT≥33/0 不回归。
- **禁词 lint：** indexes.rs 改动不涉禁词（人工/接管/takeover/hand-off），无风险。
- **commit：** 具名 `git add` 两文件，commit 消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- **CI 双门：** Baseline gate（R11.6）+ Integration tests（Docker）均 success 后 squash 合并。
