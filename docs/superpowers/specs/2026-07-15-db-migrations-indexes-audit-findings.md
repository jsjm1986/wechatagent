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

（主控亲验后填入）

## B 组 findings（数据变形迁移）

（主控亲验后填入）

## C 组 findings（seed/drop 迁移）

（主控亲验后填入）

## D 组 findings（indexes.rs 唯一性/覆盖）

（主控亲验后填入）
