# db/migrations + indexes 深度审查（第六批·末批）Design

> 只审不修 · 纯 docs 台账 · 沿用前五批范式，但**全程主控亲审**（本会话 subagent 派发连续 API mid-response 失败，改主控逐行读码——本就是「主控逐条亲验」职责，仅省 subagent 初筛，完全守「先 100% 读懂 + file:line 亲验」红线）。

## 背景与定位

第六批圈定 **`src/db/migrations/`（32 文件 3810 行）+ `src/db/indexes.rs`（1765 行）+ `src/db/mod.rs`（405 行）≈ 5980 行**，是全仓最后一个未深审领域，接续：

- 第一批 agent 旁挂能力（20 findings/1H）PR#207
- 第二批 auth+routes 安全隔离面（17 findings）PR#209
- 第三批后台 worker 群（18 findings/1H）PR#210
- 第四批 knowledge_wiki 子系统（32 findings/0H/5M）PR#211
- 第五批 evolution 自优化演化器（11 findings/0H/2M）PR#213

前五批多次引用 [[prod-app-env-guard-migrations-risk]]（m011/m012/m014 带 APP_ENV=production 守卫，非 prod 才删；生产 117 疑似未设 → 可能误删），但迁移子系统本身从未系统性深审。本批收口。

## 子系统职责（mod.rs + helpers 亲验）

- **run_with（mod.rs:223-246）**：按 `MIGRATIONS`（:90-215，m001-m031 共 31 条）顺序，`find_one({_id:migration.id})` 存在即跳过（:229），否则跑 `run_step` 再 `insert MigrationRecord`（:237-242）。**先跑 step 后写标记非事务**——step 成功但标记 insert 失败会重跑，故要求每条 step 幂等（模块头 :4 明示）。main.rs 调用顺序：`migrations::run` 先，`ensure_indexes` 后（部分迁移重建集合）。
- **APP_ENV 守卫**（m011/m012/m014/m016 亲验）：非 production 才执行破坏性操作（drop/unset/backfill），生产靠 `APP_ENV=production` 跳过。守卫用 **warn+Ok(())** 而非 Err——注释明示返 Err 会在 mod.rs:237 insert 标记前中断→迁移永不入账→每次启动重试重错（boot-brick）。这是刻意设计。
- **helpers（257 行）**：3 纯函数（merge_allowed_from_defaults / merge_state_flag_defaults / upgrade_fact_array）全「只补缺失不覆盖 + changed 门控空写」，有 mod tests 直接单测。

## 审查范围（4 组 / 主控亲审 · 无 subagent）

| 组 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **A 迁移框架根因层** | `mod.rs`(277) + `helpers.rs`(257) | 534 | run_with 编排：先跑后记非事务窗口、幂等纪律、id 唯一/时序单测、APP_ENV 守卫的 warn+Ok vs Err(boot-brick) 语义、helpers 纯函数正确性。产出**迁移安全基准**。 |
| **B 数据变形迁移** | reshape/split/backfill 类（m001/m002/m005/m008/m016/m017/m018/m022/m025/m027/m029/m030/m031） | ~1000 | 每条 filter 幂等条件（$exists/字段缺失）是否真幂等；pipeline/update 是否只补不覆盖运营值；m016 多租户回填表审计基准；m029 清理 contact 身份（删非真人）语义。 |
| **C seed/drop 迁移** | seed（m003/m006/m009/m013/m019/m020/m021/m023/m024/m026/m028）+ drop（m011/m012/m014）+ m004/m007/m010/m015 | ~2000 | seed upsert 幂等（重跑不产重复/不覆盖运营编辑）；**drop 三兄弟 APP_ENV 守卫**（m011 清空 operation_knowledge_chunks=当前 wiki 存活集合！）；m006↔m012 seed/drop 拉锯的测试补种依赖。 |
| **D indexes 唯一性/覆盖** | `indexes.rs`(1765) + `mod.rs`(405 accessor) | 2170 | 唯一索引 vs 应用层去重是否一致（前四批 [1-01] gap_signals 无唯一索引的对照面）；partial/unique 索引键与 model 字段是否对齐；ensure_indexes 幂等；typed accessor 集合名与 migrations 字面量一致性。 |

## 方法论

1. **主控逐行读 → file:line 亲验 → 两态（PLAUSIBLE 读码 / CONFIRMED 能构造触发序列）**。
2. **严重度校准（沿用前五批 + 迁移维度）**：
   - **High**：推荐配置（生产 117 单机 systemd 单进程单 workspace）下**确定性可达**的：迁移误删/误改生产存活数据、boot-brick（启动砖机无恢复）、唯一索引缺失致确定性重复/数据损坏、幂等破坏致重跑损坏数据。
   - **Medium**：需 APP_ENV 未设 + 迁移未入账叠加、或多租户/多副本才触发、或有兜底/需运维误操作。
   - **Low**：观测/就绪债/注释漂移/防御纵深。
   - **关键校准**：APP_ENV 守卫的 drop 迁移，其危险性取决于「生产 migrations 集合是否已入账该 id」——**已入账则永不重跑（mod.rs:229 skip），破坏不可达**。所以「m011 会清空知识库」这类要落到 Medium 并显式标注「待生产实证：117 的 migrations 集合是否已记录该 id + 当时 APP_ENV」，不凭「非 prod 会删」直接夸 High。
3. **元家族聚焦**：**迁移声称的幂等/守卫不变量在实现层的旁路**——先跑后记非事务窗口、$exists 幂等条件是否真幂等、APP_ENV 守卫是否覆盖所有破坏性迁移、seed/drop 拉锯的时序依赖、唯一索引缺失（前四批 [1-01]/[2-01] 元家族在 DB 层的根源）。

## 边界排除（防与前批重叠）

- 各迁移**改的业务字段语义**（如 customer_stage 归一化、outcome 聚合）前批已审——本批只审迁移的**幂等/守卫/时序**机制，不重审业务规则。
- indexes 服务的**查询正确性**（handler 是否用对索引）第二批 routes 已碰——本批只审索引**定义本身**（唯一性/键对齐/幂等）。
- evolution 集合的索引在第五批对照过，这里只补索引定义面。

## 产出

- 台账：`docs/superpowers/specs/2026-07-15-db-migrations-indexes-audit-findings.md`。
- 分支 `docs/db-migrations-audit`，基于含 #213 的最新 origin/main。纯 docs 命中 paths-ignore 无 CI 风险。
- 只审不修：绝不改任何 `.rs`。

## 全局约束

- **worktree 铁律**：push 显式 refspec `HEAD:refs/heads/docs/db-migrations-audit` + ls-remote 亲验 tip==本地；`gh pr create` 显式 `--head/--base`；建后+merge 前核 head/base/headOid；squash merge 不带 `--delete-branch`；不碰主仓在途工作（主仓被并行会话占 `feat/principal-auth-exemption`）。
- **反过拟合红线**：audit-only，绝不为发现问题改业务逻辑/迁移/索引/阈值。
- **生产实证待办**：涉及 APP_ENV 守卫的 finding，严重度依赖生产 migrations 入账状态 + APP_ENV 真值——台账显式标注为「待远程实证」，不在无证据下夸大。
