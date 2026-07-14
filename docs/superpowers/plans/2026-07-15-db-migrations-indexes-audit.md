# db/migrations + indexes 深度审查（第六批·最后一个未审领域）Implementation Plan

> **For agentic workers:** 审查工程，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 双裁决；改由**主控亲审**（本会话 subagent 派发连续 API mid-response 失败，第六批放弃派发，主控逐行读 ~5980 行 + file:line 亲验）。步骤用 checkbox 跟踪。

**Goal:** 对 `src/db/migrations/`（32 文件 3810 行，m001-m031 + helpers + mod）+ `src/db/indexes.rs`（1765 行）+ `src/db/mod.rs`（405 行 accessor/connect）做纯代码/设计审查，产出经主控逐条亲验的 findings 台账，合并 docs PR。这是继前五批后**最后一个未审领域**。

**Architecture:** 主控亲审，逻辑分 4 组（不派 subagent，只作台账组织维度）：
- **A 迁移框架根因层**：mod.rs（run_with 编排：find_one→skip/run→insert 标记的非事务序、幂等要求、id 唯一/时序单测）+ helpers.rs（3 纯函数只补缺失不覆盖）+ **APP_ENV 生产守卫家族**（m011/m012/m014/m016/m025 等 `APP_ENV=production`→warn+Ok noop 形态；命门=[[prod-app-env-guard-migrations-risk]]：生产 117 若未设 APP_ENV=production，非 prod 分支会删数据/清 seed）。
- **B 数据变形迁移**：reshape/split/backfill（m001/m002/m005/m008/m016/m018/m022/m025/m027/m029/m030/m031）——幂等 filter 是否真挡二次执行、aggregation pipeline 正确性、回填口径。
- **C seed/drop 迁移**：seed（m006/m013/m020/m021/m023/m024/m026/m028）+ drop（m011/m012/m014）——seed upsert 幂等、drop 全量删的 APP_ENV 守卫、seed 与 prompts.rs::ensure_prompt_pack_v2 的种子源是否漂移（[[project-config-seed-in-prompts-not-migrations]]）。
- **D indexes.rs**：唯一索引覆盖（幂等键/业务去重键是否有 unique 兜底，对照前四批发现的 find-then-insert 无索引缺陷 [[project-knowledge-wiki-audit]] [1-01]）、partial/TTL 索引正确性、ensure_indexes 与 migrations 执行序（main.rs 先 migrations 后 indexes）。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象 Rust（src/db/ ~5980 行）。

## Global Constraints

- 分支 `docs/db-migrations-audit`，基于含 #213 的最新 origin/main（9e9e16e）。
- **只审不修**：本批绝不改任何 .rs。产出纯 docs（台账）。
- **主控亲审**（无 subagent）：每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；两态 PLAUSIBLE/CONFIRMED。
- **严重度校准防夸大**：High=推荐配置（生产 117 单机 systemd 单进程 + 默认单 workspace）下确定性可达的：数据损坏/丢失（迁移误删存活集合）、boot-brick（迁移返 Err 永不入账每次启动重错）、幂等破洞致重跑损坏、唯一索引缺失致业务去重失效且有真实并发写入源。Medium=需多条件叠加/多副本/多租户/生产未设 APP_ENV 才触发或有兜底。Low=观测/边缘/就绪债/已入账迁移不再重跑的历史隐患。
- **⚠️ APP_ENV 守卫家族严重度关键**：这些迁移的 id 早已在生产 migrations 集合**入账**（若已 applied 则 mod.rs:229 existing.is_some()→skip 永不重跑）。故"非 prod 删数据"隐患**仅在该迁移首次跑且当时 APP_ENV 未设 production 时可达**。判定须区分"已入账（历史安全）vs 首次可达"，并标注需生产实证（查 117 migrations 集合 + APP_ENV）。不凭空判 High。
- **元家族聚焦**：迁移声称的幂等/守卫/回填不变量（幂等 filter 真挡二次、APP_ENV 守卫防误删、seed 只补不覆盖、backfill 口径准），实现层是否有非幂等窗口/守卫遗漏/口径漂移/与 seed 源（prompts.rs）不对齐——前五批"声称不变量实现层有旁路/层间不对称"元家族在持久层的延伸。
- **边界排除（防与既往批次重叠）**：evolution 集合的索引（第五批已审逻辑，本批仅看索引定义）；knowledge_gap_signals 去重索引（第四批 [1-01] 已标，本批仅核 indexes.rs 是否补了 unique）；db 之上的业务读写路径（前四批已审，本批仅审 migrations/indexes/connect）。
- 不碰主仓在途工作（主仓被并行会话占 `feat/principal-auth-exemption`）；本会话在 worktree fix-full-system-remediation。

---

### Task 0: 建台账骨架 + 设计 + 计划

**Files:** design + plan + findings 骨架（本文件即 plan）。

- [ ] Step 1: 写台账头部（审查范围 4 组 + 文件清单 ~5980 行、方法论、检查清单、严重度校准含 APP_ENV 已入账判据、元家族、边界排除）+ 字段模板 + 4 组占位段 + 环节汇总占位。
- [ ] Step 2: Commit（design + plan + findings 骨架一起）。

---

### Task A: 迁移框架根因层（mod.rs + helpers + APP_ENV 守卫家族）

**审查对象:** `mod.rs`(277) + `helpers.rs`(257) + 全部带 APP_ENV 守卫的迁移（m011/m012/m014/m016 已亲验，扫全 32 文件找其它守卫）。

- [ ] Step 1: 主控逐行读 + Grep 全 migrations 找 `APP_ENV` / `env::var` 守卫点，核每个守卫形态（warn+Ok vs Err）+ 该迁移非 prod 分支的破坏性（删/清/unset）+ 是否清空存活集合。
- [ ] Step 2: 填台账（A 组）+ Commit。

---

### Task B: 数据变形迁移（reshape/split/backfill）

**审查对象:** m001/m002/m005/m008/m016/m018/m022/m025/m027/m029/m030/m031。

- [ ] Step 1: 逐条核幂等 filter 是否真挡二次执行、pipeline/回填口径正确性、$set 是否误覆盖运营值。
- [ ] Step 2: 填台账（B 组）+ Commit。

---

### Task C: seed/drop 迁移

**审查对象:** seed m006/m013/m020/m021/m023/m024/m026/m028 + drop m011/m012/m014。

- [ ] Step 1: 核 seed upsert 幂等 + 与 prompts.rs::ensure_prompt_pack_v2 种子源是否漂移；drop 的 APP_ENV 守卫 + 破坏性。
- [ ] Step 2: 填台账（C 组）+ Commit。

---

### Task D: indexes.rs 唯一性/覆盖

**审查对象:** `indexes.rs`(1765) + `mod.rs`(405 connect/accessor)。

- [ ] Step 1: 核唯一索引覆盖（幂等键/业务去重键，特别对照第四批 [1-01] knowledge_gap_signals 无 unique 是否已补）、partial/TTL 正确性、ensure_indexes 与 migrations 执行序。
- [ ] Step 2: 填台账（D 组）+ Commit。

---

### Task E: 台账收尾 + push + PR

- [ ] Step 1: 汇总头（总数/严重度分布/元家族/后续 P0-P3 路线/交叉去重/正向 HOLDS）。
- [ ] Step 2: 交叉去重（APP_ENV 守卫家族、幂等模式跨文件重复归并）。
- [ ] Step 3: Commit + push（显式 refspec `HEAD:refs/heads/docs/db-migrations-audit`）+ ls-remote 亲验 tip + gh pr create 显式 --head/--base + 核 headRefOid。
- [ ] Step 4: docs-only 命中 paths-ignore，核 CI 无意外后 squash merge（不带 --delete-branch）。写 memory。

---

## 备注

- **主控亲审全程**：本会话 subagent 三次 API mid-response 失败 + SendMessage 续派空返（见第五批教训），第六批直接主控读码，守住"先 100% 读懂 + file:line 亲验"红线。
- 报告无 scratch 目录（无 subagent）；findings 直接进台账。
- 本批零代码改动、零 CI 风险（纯 docs）。
- **最后一个未审领域**：本批完成后前六批覆盖 agent 旁挂/auth-routes/worker 群/knowledge_wiki/evolution/db 持久层全部深审。
