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

（主控亲验后填入）

## D 组 findings（indexes.rs 唯一性/覆盖）

（主控亲验后填入）
