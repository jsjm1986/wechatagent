# HC-015 Taxonomy 与画像信号部署后验证记录（2026-07-27）

## 范围与部署事实

本批闭环 SR-045、SR-046、SR-047、SR-061。没有再次切换正式二进制：当前正式后端 SHA-256 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 的 release 源码已包含四条生产实现；核对的核心文件与本地工作树逐字一致。生产库迁移 `2026_07_050_taxonomy_identity_claims` 为 `applied`，索引 `uniq_sys_tax_ws_scope_kind_active_identity` 是 `(workspace_id,scope,kind,value.identityClaims)` unique multikey，partial filter 为 `current_version=true + value.status=active`。

## 真实副本集验证

- SR-045/SR-047 联合 Gateway 红线 1/1：在随机 `wechatagent_test_*` 库中，以 mock LLM 驱动真实 `handle_managed_message`。同一 run 产出未知 `customer_stage` 及两条同维同值 Bayesian 观测；候选只落一次、`occurrences=1`，Bayesian history 只落一个点、confidence 取 `0.9`，该点 `sourceRunId` 与实际 AgentRun id 相同。
- SR-046 m050/索引红线 1/1：真实迁移为 current 与 historical 行分别回填 canonical+alias claims；共享 active alias 的 legacy 歧义使迁移在首写前失败，sentinel 无变化；清除歧义、恢复生产索引后，第二 active owner 由 E11000 拒绝。
- SR-061 claim 回滚红线 1/1：真实 Cookie Router 审批在字典插入故障时回滚候选 claim、保持 pending；清除故障后同请求重试成功，持久 actor 为认证管理员而不是请求体伪造值。
- SR-061 合并红线 1/1：current v3、历史最大 v9 合并后返回 `mergedIntoExisting=true`，追加 v10/previous v3；原始候选值与手工 alias 均持久化，显示名、描述、status、priority、terminal、reactivation 字段保持，候选与字典指针同事务终结。

四条精确测试合计 4/4。新增两个随机库均由包装器删除，测试库数量 126→126。测试目标源码 SHA-256 为 `22fb290791fa6564e2378430f6480d6911da4827c62dd392c88c53350b35c299`。

## 生产零扰动与证据

验证前后正式服务均为 PID `2021387`、`NRestarts=0`；磁盘与 `/proc/<pid>/exe` SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`。`/api/health` 持续返回 `ok=true`，Evolution 保持关闭。验证没有连接或修改生产业务库。

服务器冻结证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc015-taxonomy-20260727T103000Z`。`SHA256SUMS.final` 已逐文件 `sha256sum -c` 通过，其 SHA-256 为 `bdb514b609c188429c4d038c6057bd467395504b2771bec9f377e1c6a6117df7`。
