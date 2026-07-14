# db/migrations + indexes 深度审查（第六批·最后一个未审领域）Findings 台账

> 只审不修 · 纯 docs 台账 · 主控亲审（无 subagent，本会话派发连续 API 失败）。逐条 Read/Grep 亲验 file:line。

## 审查范围（4 组 / ~5980 行）

| 组 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **A 迁移框架根因层** | `mod.rs`(277) + `helpers.rs`(257) + APP_ENV 守卫家族（m011/m012/m014/m016…） | ~534+ | run_with 编排（find_one→skip/run→insert 标记非事务序、幂等要求、id 唯一/时序单测）；helpers 3 纯函数只补不覆盖；**APP_ENV 生产守卫**（warn+Ok noop 防误删；命门=生产未设 APP_ENV=production 时非 prod 分支删数据） |
| **B 数据变形迁移** | m001/m002/m005/m008/m016/m018/m022/m025/m027/m029/m030/m031 | ~1000 | 幂等 filter 真挡二次执行；aggregation pipeline 正确性；回填口径；$set 不误覆盖运营值 |
| **C seed/drop 迁移** | seed m006/m013/m020/m021/m023/m024/m026/m028 + drop m011/m012/m014 | ~1600 | seed upsert 幂等；与 prompts.rs::ensure_prompt_pack_v2 种子源是否漂移；drop 全量删的 APP_ENV 守卫 + 破坏性 |
| **D indexes.rs** | `indexes.rs`(1765) + `mod.rs`(405 connect/accessor) | 2170 | 唯一索引覆盖（幂等键/业务去重键，对照第四批 [1-01] knowledge_gap_signals 无 unique）；partial/TTL 正确性；ensure_indexes 与 migrations 执行序 |

## 方法论（主控亲审）

1. **主控逐行读 + Grep 亲验** —— 无 subagent（本会话派发连续 API mid-response 失败 + SendMessage 续派空返，见第五批教训）；每 finding Read/Grep 复核 file:line + 失效链成立性。
2. **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列/或已在生产实证）。
3. **已入账判据**：APP_ENV 守卫家族的迁移 id 若已在生产 migrations 集合入账，则永不重跑（mod.rs:229 skip）——隐患仅在"首次跑且当时 APP_ENV 未设 production"可达，须标注生产实证需求，不凭空判 High。

## 迁移安全/幂等检查清单

1. **幂等**：filter 是否真挡二次执行（`$exists:false` / `commitments 不存在` / upsert）？重跑不破坏数据？
2. **APP_ENV 守卫**：破坏性迁移（删/清/unset）是否有 `APP_ENV=production`→noop 守卫？形态是 warn+Ok（正确，防 boot-brick）还是 Err（会永不入账每次重错）？
3. **非事务序**：run_with 先跑 step 后 insert 标记——step 成功但标记写失败会重跑，幂等是否兜住？
4. **回填口径**：backfill 的 $set 值/条件是否正确？是否误覆盖运营人员已写值（只补缺失）？
5. **seed 源一致**：migration seed 与 prompts.rs::ensure_prompt_pack_v2 是否同源不漂移？
6. **唯一索引覆盖**：幂等键/业务去重键是否有 unique/partial unique 兜底？find-then-insert 无索引缺陷（第四批 [1-01]）是否已补？
7. **执行序**：ensure_indexes 与 migrations 顺序（main.rs 先 migrations 后 indexes）；unique 索引建前是否已有重复数据致建索引失败？

## 严重度校准（防夸大）

- **High**：生产 117 单机单进程单 workspace 推荐配置下**确定性可达**的：迁移误删/清空存活集合致数据丢失、boot-brick（Err 永不入账每次重错）、幂等破洞致重跑损坏、唯一索引缺失且有真实并发写入源致业务去重失效。
- **Medium**：需多条件叠加/多副本/多租户/**生产未设 APP_ENV** 才触发，或有兜底。
- **Low**：观测/边缘/就绪债/**已入账迁移不再重跑的历史隐患**/单进程默认不可达的并发竞态。
- **⚠️ APP_ENV 守卫家族**：区分"已入账（历史安全）"vs"首次可达"，标注生产实证需求（[[prod-app-env-guard-migrations-risk]] 口径 + 第三批部署拓扑维度）。

## 元家族聚焦

本批预期主线元家族=**「持久层迁移/索引声称的幂等/守卫/回填/唯一性不变量在实现层的旁路」**：幂等 filter 是否真挡二次、APP_ENV 守卫是否覆盖所有破坏性迁移且形态正确、backfill 口径是否漂移、seed 与 prompts.rs 是否同源、业务去重键是否有 unique 索引兜底——前五批"声称不变量实现层有旁路/层间不对称"元家族在持久层的收官延伸。

## 边界排除（防与既往批次重叠）

- evolution 集合逻辑（第五批已审）——本批仅看其索引定义。
- knowledge_gap_signals 去重（第四批 [1-01] 已标）——本批仅核 indexes.rs 是否补了 unique 索引。
- db 之上的业务读写路径（前四批已审）——本批仅审 migrations/indexes/connect。

## 环节汇总（收尾时填）

- 总 findings 数：（TaskE 填）
- 严重度分布：（H/M/L，TaskE 填）
- 元家族归纳：（TaskE 填）
- 后续 P0-P3 路线：（TaskE 填）
- 交叉去重留痕：（TaskE 填）
- 正向 HOLDS（主控亲验）：（TaskE 填）
- 生产实证需求：（APP_ENV 守卫家族需查 117 migrations 集合 + APP_ENV，TaskE 填）

---

## 字段模板

```
### [X-NN] 一句话标题
- 入口: —（迁移 id / 函数）
- 所属组: A|B|C|D
- 类型: 幂等|APP_ENV守卫|非事务序|回填口径|seed源漂移|唯一索引缺失|执行序|数据丢失|boot-brick|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 失效链: （谁在什么时机触发什么数据损坏/丢失/重跑后果；非失效类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## A 组 findings（迁移框架根因层）

> 主控逐行读 `mod.rs`(277 全) + `helpers.rs`(257 全) + APP_ENV 守卫家族（m011/m012/m014/m016）。框架设计稳健，**1 条 Low（非事务序，幂等兜底）**，其余正向 HOLDS。

### [A-01] run_with 先跑 step 后 insert MigrationRecord 非事务 → step 成功但标记写失败会重跑
- 入口: `run_with`（`src/db/migrations/mod.rs:223-246`）
- 所属组: A
- 类型: 非事务序|幂等
- 严重度: **Low**（主控裁定：非事务窗口客观存在，但框架从设计上要求每条 step 幂等（`mod.rs:4` 明示「即使标记丢失，重跑也不破坏数据」），已逐条亲验各 step 幂等 filter 兜住重跑；故重跑不损坏数据 → 观测/边缘，非确定性可致错）
- 现象/风险: `mod.rs:237 (migration.run)(db).await?` 跑完 step 后，`mod.rs:242 collection.insert_one(record)` 才写 MigrationRecord。两步非事务：step 成功但进程在 insert 标记前崩溃/insert 失败 → 该迁移下次启动重跑。
- 失效链: step 成功 → insert 标记前崩溃 → 重启 → `find_one({_id})` 仍不存在 → 重跑同一 step。若 step 非幂等则损坏；但全部 step 经亲验幂等（`$exists:false` / `commitments 不存在` / upsert / delete_many 二次 matched=0）→ 重跑无害。
- 根因（亲验 file:line）: `mod.rs:225-244` 循环体：`find_one`（:226）→ `is_some()` skip（:229）→ `(migration.run)(db).await?`（:237）→ `insert_one(record)`（:242）。step 与标记写非同一 transaction。
- 复现设想: 迁移 step 成功后、insert 标记前 kill 进程；重启观察该 step 重跑（幂等 step 无副作用）。
- 验证状态: PLAUSIBLE（非事务窗口确凿；幂等兜底使后果为零，故 Low）
- 修复建议: 框架无需改——幂等是 step 的契约（`mod.rs:4` 已明示且逐条满足）。若未来引入非幂等 step，需在 step 内自带 CAS/标记，或把 step+标记包进 transaction。当前无缺陷。
- 状态: Open

**A 组正向 HOLDS（主控亲验）**：
- **APP_ENV 守卫形态正确（防 boot-brick）**：m011（`m011:19-25`）/m012（`m012:19-25`）/m014（`m014:15-21`）/m016（`m016:96-102`）四个破坏性/回填迁移的 `APP_ENV=production` 守卫全是 **warn+`return Ok(())`** noop 形态——注释明示（`m011:8-10`/`m012:8-9`）为何不用 `Err`：返 Err 会在 `mod.rs:237 .await?` 处于 insert 标记（:242）前中断，迁移永不入账，每次启动重试重错（boot-brick 无干净恢复路径）。设计正确。
- **id 唯一 + 时序单测兜底**：`mod.rs:252-276` 两单测锁死 MIGRATIONS id 唯一（dedup 前后长度相等）+ 严格时序递增（windows(2) 断言 `[0].id < [1].id`）——防重复 id 致某迁移被跳 / 乱序执行。
- **helpers 3 纯函数「只补缺失不覆盖」**：`merge_allowed_from_defaults`（:14-49）/`merge_state_flag_defaults`（:57-83）/`upgrade_fact_array`（:88-128）均 `!contains_key` 才写 + `changed` 门控空写（false 默认不落库），不覆盖运营人员已写值；6 单测覆盖「不覆盖已写值 / 已完整时不空写 / false 默认不落库」（:149-256）。
- **run 执行序**：`run`（:218-220）→`run_with(MIGRATIONS)`，main.rs 保证 migrations 先于 ensure_indexes（CLAUDE.md 架构约束），迁移间严格 id 顺序串行（:225 for 循环）。

## B 组 findings（数据变形迁移）

> 逐条主控亲验 m001/m002/m005/m008/m016/m018/m022/m025/m027/m029/m030/m031，**无 finding**——幂等 filter 逐条真挡二次执行、回填口径正确、$set 只补缺失不误覆盖运营值。下列为正向 HOLDS。

**B 组正向 HOLDS（主控逐条亲验 file:line）：**
- **m001 backfill last_inbound_at**（`m001:20-35`）：filter `last_message_at 存在非null AND last_inbound_at 缺失/null`，pipeline `$set last_inbound_at=$last_message_at` 靠 filter 兜住，二次执行 filter 不命中 → 幂等。
- **m002 activeFacts 拆分**（`m002:12-42`）：aggregation `$slice[.,6]`(coreFacts) + `$slice[.,6,10000]`(recentFacts) + `$unset activeFacts`；filter `activeFacts:{$exists:true}` 二次不命中；coreFacts 保留 legacy `Vec<String>` 兼容（CLAUDE.md R11）→ 幂等。
- **m005 fact 结构化**（`m005:26-92`）：复用 helpers `upgrade_fact_array`（按元素含 `id` 字段判定，已结构化跳过）；fresh UUIDv4 + `$inc memory_card_version`；`!core_changed && !recent_changed → continue` 门控空写 → 幂等。
- **m008 commitments reshape**（`m008:16-49`）：`last_commitment:Option<String>` → `commitments:[{id,text,createdAt}]` + `$unset`；filter `commitments:{$exists:false}` 二次不命中；`$cond` 处理 missing/null/空串 → 幂等。
- **m016 workspace_id 回填**（`m016:95-143`）：`$exists:false → $set default_ws`，snake/camel 双表；**三单测审计基准兜底**（SNAKE/CAMEL 表 ⊆ 已核实全集、两表不相交、MUST_NOT_BACKFILL 排除 admin_users/chunk_revisions），防拼错/归错类；APP_ENV=production 守卫（warn+Ok noop）→ 幂等 + 有守卫。
- **m018 顶层残留→domain_attributes**（`m018:31-80`）：`$mergeObjects([顶层字段..., $ifNull($domain_attributes,{})])` 现有 domain 值在末位覆盖（新覆旧），只回填不 $unset（可逆）；三纯函数单测覆盖；二次 domain 已有 key 结果不变 → 幂等 + 不误覆盖。
- **m022 dormant/churned allowFromAny**（`m022:16-56`）：只对 key∈{dormant,churned} 且 `allowFromAny` 缺失/false 时 `$set true`，`changed` 门控空写 → 幂等 + 只补不覆盖。
- **m025/m027 契约字段回填**（`m025:14-22`/`m027:14-32`）：`ask_human_policy`→"default"、`trust_level`→"normal"、`authorized_topics`→[]，均 `$exists:false` 命中 → 幂等，无守卫（语义保持型回填，正确）。
- **m029 运营池身份清理**（`m029:22-105`，唯一含删除的数据变形）：删 `agent_status:normal 且 !is_operatable_person(wxid)`（删条件双重限定 normal），**managed 一律保留**；roster 回填 nickname/avatar_url；Demi 误名清 None。**有意无 APP_ENV 守卫**（注释:9-10 明示修 webhook 建档 bug 的存量清理需全环境生效，只删「本不该进池的非真人 normal 行」非业务数据）；不删 conversation_messages；幂等 → 合理。
- **m030 老成交 outcome 默认**（`m030:19-53`）：`deal_verification`→"unverified"、`outcome_event_kind`→"deal_closed"，仅对 `lifecycle_stage=deal_won 或 deal_closed_at 存在` 的成交 contact + `$exists:false` → 幂等。
- **m031 escalation last_pushed_at_ms**（`m031:30-42`）：`$toLong($created_at)` 回填（旧口径字节等价），纯函数+单测，语义保持型无守卫（注释:8-9 明示误加守卫会致生产静默 SKIP）→ 幂等。
- **helpers 三纯函数**（`helpers.rs:14-141`）：`merge_allowed_from_defaults`/`merge_state_flag_defaults` 只补缺失不覆盖 + `changed` 门控空写；`upgrade_fact_array` 按 `id` 字段判定幂等；4 单测覆盖「不覆盖运营值」「已完整时不空写」「false 默认不落库」→ 全 HOLDS。

## C 组 findings（seed/drop 迁移）

> seed 家族（m006/m013/m020/m021/m023/m024/m026/m028）全部 `$setOnInsert` upsert 幂等 + 不覆盖运营编辑，主控逐个亲验 file:line；drop 家族（m011/m012/m014）AP_ENV=production→warn+Ok noop 守卫形态正确。核心 finding=[C-01] m011 清空的 `operation_knowledge_chunks` 是**当前 wiki 知识存活集合**，其破坏性完全押在 APP_ENV 守卫 + 迁移已入账双条件上。

### [C-01] m011 `delete_many({})` 全量清空 `operation_knowledge_chunks`（当前 wiki 知识存活集合），破坏性只靠 APP_ENV 守卫 + 已入账兜住
- 入口: `m011_drop_legacy_sales_collections::run_step`（`src/db/migrations/m011_drop_legacy_sales_collections.rs:19-43`）
- 所属组: C
- 类型: 数据丢失|APP_ENV守卫
- 严重度: **Medium**（主控裁定：m011 对 `operation_knowledge_chunks`/`_documents`/`_items` 三集合无条件 `delete_many({})` 全量清空——而 `operation_knowledge_chunks` 是**当前活跃使用**的 wiki 知识存活集合（`db/mod.rs:149-150` typed accessor + CLAUDE.md 硬规则「产品声明须 verified 知识在 operation_knowledge_chunks」）。若某次启动时 m011 尚未入账 且 `APP_ENV != "production"`，会**清空全部已验证知识库**——生产数据丢失。但严重度非 High：①m011 id `2026_05_V3_002` 若已在生产 migrations 集合入账则**永不重跑**（`mod.rs:229 existing.is_some()→skip`），历史安全；②APP_ENV=production 守卫在设的前提下 noop。故确定性可达需「m011 未入账 + APP_ENV 未设 production」双条件叠加 → 生产实证需求，不凭空判 High）
- 现象/风险: m011 注释（:1-3）称「开发期数据无价值」，但集合名 `operation_knowledge_chunks` 与当前 wiki 子系统存活集合**同名同用**（非 legacy）。`delete_many({})` 是全量删（filter `{}`）。守卫仅 `APP_ENV=="production"`（`:20`）；`unwrap_or_default()` 使未设 env→空串→非 production→执行删除。
- 失效链: 全新/迁移记录丢失的生产实例，启动时 APP_ENV 未设为 "production" → m011 `find_one` 不存在 → run_step 执行 → `operation_knowledge_chunks.delete_many({})` 清空全部 verified 知识 chunk → wiki 召回/产品声明 grounding 全部落空（blocked_unverified_product_claim）。
- 根因（亲验 file:line）:
  - `m011_drop_legacy_sales_collections.rs:20`：`std::env::var("APP_ENV").unwrap_or_default() == "production"` 守卫（未设→空串→不跳过）。
  - `m011_drop_legacy_sales_collections.rs:28-34`：`for name in ["operation_knowledge_items","operation_knowledge_documents","operation_knowledge_chunks"] { coll.delete_many(doc!{}, None) }` 全量删三集合。
  - `src/db/mod.rs:149-150`：`operation_knowledge_chunks` typed accessor 当前活跃。
  - `src/db/migrations/mod.rs:229`：已入账迁移 skip（历史入账即安全）。
- 复现设想: 生产 migrations 集合无 `2026_05_V3_002_drop_legacy_sales_collections` 记录（新实例/记录丢失）+ 启动环境未设 `APP_ENV=production` → 启动跑 m011 → 查 operation_knowledge_chunks 计数归零。
- 验证状态: PLAUSIBLE（代码路径确凿；是否确定性可达取决于生产 117 的 migrations 集合是否已入账 m011 + APP_ENV 是否设 production → **生产实证需求**，见 [[prod-app-env-guard-migrations-risk]]）
- 修复建议: ①最稳=生产 117 显式设 `APP_ENV=production`（一次性运维动作，同时消解 m012/m014 同类风险）；②m011 的集合名若确指 legacy，应与当前 wiki 存活集合物理区分（改名或加 `legacy_` 前缀），避免「同名集合被 legacy 清理迁移误删」；③长期：破坏性 drop 迁移不应依赖运行时 env 判定，改为一次性运维脚本 + 显式确认。
- 状态: Open

**C 组正向 HOLDS（主控亲验）**：
- **seed 家族全幂等**：m006（taxonomy_seed）/m020（purchase_lifecycle）/m021（churn_reason）/m023（value_tier）/m024（relationship_type）/m026（sales_with_relationships）/m028（conversation_mode）全部 `$setOnInsert` + `upsert(true)`（各文件亲验 `$setOnInsert`+`UpdateOptions::builder().upsert(true)`），仅 insert 时写默认值，不覆盖运营人员后续 API 编辑；m006 额外靠 `(scope,kind,value.id)` 唯一索引双层兜底（m006:9-14）。
- **m013 state policies 无漂移**：`derive_state_policy_lists`（m013:23-39）抽成纯函数作**唯一真相**，与 `routes::admin_ops_versions::publish_state_machine_version` 共用（H13），杜绝 m013 与 publish 路径漂移；find_one 存在即 skip 保留运营调整；单测锁死 DEFAULT 字节等价（:119-139）。
- **seed 与 prompts.rs 同源**：m006 数据源明示与 prompts.rs 现有 prompt 文案对齐（m006:16-19）；示例 profile（m020/m026）一律 draft 态不激活（零扰动，无 active profile 时回落 DEFAULT）。
- **drop 家族守卫形态正确**：m011/m012/m014 三破坏性迁移均 `APP_ENV=="production"`→`warn!+Ok(())` noop（非返 Err），注释明示为何用 warn+Ok（返 Err 会在 `mod.rs:237` insert 标记前中断 → 迁移永不入账 → 每次启动重错 boot-brick）——守卫形态本身正确，问题在 [C-01] 的「未设 env + 未入账」窗口 + 集合同名。
- **m012/m014 破坏性弱于 m011**：m012 删 taxonomy seed（可由 m006 重 seed 恢复）、m014 `$unset trigger_keywords`（字段已下线无用）——数据丢失后果远轻于 m011 清空 verified 知识；同守卫，同 [C-01] 生产实证，合并为交叉去重项不单列。

## D 组 findings（indexes.rs 唯一性/覆盖）

> 主控逐段亲验 `indexes.rs`(1765) + `db/mod.rs`(405) 关键面：unique/partial unique/sparse unique/TTL、outbox 幂等键、ops versioned 灰度索引切换、ensure_indexes 编排。1 条 Low（交叉第四批 [1-01]），其余全 HOLDS。

### [D-01] knowledge_gap_signals 仍无 (workspace_id,chunk_id,kind,status) 业务去重 unique 索引（交叉第四批 [1-01]，本批仅核实未补）
- 入口: `ensure_all` → knowledge_gap_signals 索引段（`src/db/indexes.rs:1396-1445`）
- 所属组: D
- 类型: 唯一索引缺失|就绪债
- 严重度: **Low**（主控裁定：与第四批 [1-01] 同一缺陷，本批仅从 indexes.rs 侧核实"未补"。gap_signals 三索引 = `(status,kind)`/`(created_at)`/`signal_id unique` + `(kind,status,created)`，唯一索引仅 `signal_id`（新 UUID 主键，非业务键）。业务去重仍靠 gap_signals.rs:610 应用层 find-then-insert。feedback_worker 单实例串行 run_one_round → 并发窗口单进程默认不可达，无索引兜底=确定性去重缺失但无并发触发源 → Low，同第四批定级）
- 现象/风险: `gap_signals_signal_id_unique`（indexes.rs:1422-1428）锁的是 `signal_id`（每次新生成的 UUID），不是业务去重键。若未来 feedback_worker 多副本/并发，find-then-insert（find pending → 应用层 dedup_key 匹配 → insert）在无 unique 索引兜底下会重复插入同一 (workspace,chunk,kind) 信号。
- 失效链: 多副本 feedback_worker 并发 run_one_round → 两副本同时 find 到无 pending → 都 insert → 重复信号（单进程单副本默认不可达）。
- 根因（亲验 file:line）:
  - `src/db/indexes.rs:1396-1445` knowledge_gap_signals 四索引：`gap_signals_status_kind_idx`(:1402)/`gap_signals_created_at_idx`(:1415)/`gap_signals_signal_id_unique`(:1422-1428，唯一但锁 signal_id UUID)/`gap_signals_kind_status_created_idx`(:1439)——无 (workspace_id,chunk_id,kind,status) partial unique。
  - 对照第四批已亲验 gap_signals.rs:610 find-then-insert 应用层去重。
- 复现设想: 多副本部署 feedback_worker（当前单进程不可达）；或手工并发触发两次 structural lint。
- 验证状态: PLAUSIBLE（索引确实缺失；并发触发源单进程默认不可达）
- 修复建议: 加 `(workspace_id,chunk_id,kind,status)` partial unique 索引（partial filter `status:"pending"`，避免历史 resolved 信号阻挡新 pending），与 outbox idempotency_key / silence dedupe_key 的强幂等姿势对齐。低优先（单副本默认无触发源），与第四批 [1-01] 一并收口。
- 状态: Open

**D 组正向 HOLDS（主控亲验）**：
- **outbox 幂等键 unique HOLDS**：`idempotency_key` unique（indexes.rs:808-817）= 强幂等门，DuplicateKey→IdempotentSkip；配 `(account_id,status,next_retry_at)` 扫描 + `(status,locked_until)` 崩溃恢复 lease + `(account_id,status,sent_at:-1)` pacing guard（避免内存 SORT）。
- **messages 双唯一 HOLDS**：`(workspace_id,account_id,message_id)` sparse unique（:62-70）+ `(workspace_id,account_id,dedupe_key)` partial unique（:71-84，仅 dedupe_key 为 string 时约束）——去重锚点齐备。
- **tasks outcome_aggregation partial unique HOLDS**：`(kind,account_id,content)` partial unique（filter `kind:"outcome_aggregation"`，:112-132）——注释明示替代 TOCTOU find-then-insert 原子去重，且 partial filter 限定 kind 不误伤其他 kind 同 content 合法重复。
- **events dedupe_key partial unique HOLDS**：`(workspace_id,dedupe_key)` partial unique（:184-194，仅携带 dedupe_key 的事件约束，不携带的正常重复写）。
- **import_jobs TTL 不误删进行中 HOLDS**：`expires_at` TTL expireAfterSeconds=0（:155-168），worker 落终态才置 expires_at，pending/running 不设该字段 → TTL 忽略缺失字段绝不误删进行中 job（与 knowledge_operator_memory TTL 同构）。
- **ops versioned 灰度索引切换二次启动安全 HOLDS**：drop 旧 unique（`workspace_id_1_domain_1` 等）用 best-effort `let _ =`（:886-889/920-923/968-971，失败不阻塞），新 4-tuple unique（version 维度）+ current_version partial 索引；MongoDB 对已存在索引静默 noop。注释:339-343 明示为何 ops 三表 unique 统一由 ensure_ops_versioned_indexes 管——避免旧 unique 在多版本驻留下 E11000 致 ensure_indexes 返 Err boot-brick（H8）。
- **执行序 HOLDS**：main.rs 先 `migrations::run` 后 `ensure_indexes`（CLAUDE.md 明示 + db/mod.rs connect 不建索引）；unique 索引建前若已有重复数据会 E11000 → 但 seed 迁移 upsert 幂等不产重复，backfill 只补缺失不产重复 → 建 unique 安全。
- **$in 禁用防 Error 67 HOLDS**：注释:204/452 明示 partial_filter_expression 绝不用 `$in`（会触发 Error 67 让 ensure_indexes panic），改用 `$type`/精确值。
