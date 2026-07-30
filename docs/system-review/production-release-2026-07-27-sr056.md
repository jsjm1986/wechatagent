# SR-056 DomainSchema 部署后验证记录（2026-07-27）

## 结论

SR-056 已随当前正式后端部署，并完成服务器真实 MongoDB 副本集专项验证。本次没有再次切换二进制：正式 release 中 `domain_schemas` 路由、m044、索引实现与副本集测试源码均与本地逐字一致。

正式库迁移 `2026_07_044_domain_schema_single_active` 为 `applied`。数据库存在两个目标约束：`domain_schemas_ws_id_version_unique` 对 `(workspace_id,schema_id,version)` 唯一，`domain_schemas_ws_active_unique` 对 `is_active=true` 的 workspace 建 partial unique。

## 动态红线

服务器以 Rust 1.92 release 目标在随机隔离库运行：

- `exact_version_activation_is_atomic_unique_and_preserves_history`：1/1；
- 从 v1 active、v2 inactive 精确激活 v2，事务后 active 恰好一条；
- v1、v2 两条不可变历史均保留；
- 激活不存在的 v99 返回 `domain_schema_version_changed`，原 active 指针不变。

测试目标源码 SHA-256 为 `33ea0fa6d5190b9b20216c5c0e55723451ad0b4d8adaa11f982ab7e4e6d1d053`。测试前、清理前和测试后随机库数量均为 126，没有遗留新库。

## 服务与证据

测试前后正式服务 PID 均为 `2021387`，`NRestarts=0`。磁盘与运行中后端 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康检查通过且 Evolution 关闭。

服务器冻结证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/sr056-domain-schema-20260727T123000Z`。`SHA256SUMS.final` 已逐文件校验通过，其 SHA-256 为 `c1e644e214f568cb3624884f8e8f4d3a9a72e35bc7f3eaa5c836d7d832200c29`。

## 边界

本记录只结算 SR-056，不自动关闭 HC-014 的 SR-094 或其它仍显式开放的风险，也不把 mock/确定性 Mongo 事务证据外推成真实模型或 MCP 验证。
