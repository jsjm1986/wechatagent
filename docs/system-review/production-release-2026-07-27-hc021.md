# HC-021 Campaign 协议部署后验证记录（2026-07-27）

当前正式 release 的 `campaigns.rs`、`campaign_dispatch_integration.rs`、Campaign Store 与创建页四份源码和本地逐字一致；运行制品包含 `specVersion/specHash`、`dispatching` 与 prepared send intent 协议。本轮没有切换二进制或修改正式业务库。

服务器真实 Handler/Mongo 完整 ignored 目标 11/11：终态 preview 保持完整 Campaign BSON 零写，草稿 PATCH 后按新规格预览；首次 dispatch 冻结 generation、spec hash、意图与受众，task insert 故障后 prepared intent 保留并由 reconciler 恢复为唯一任务；重复派发、零命中、受众上限、lastDispatchTargetCount 与 workspace/account 边界均通过。

前端 Campaign Store、创建页、看板与 CSV 定向重跑 33/33；既有完整专项 36/36、Rust 协议 16/16和生产构建证据继续有效。测试库 126→136→126：10 个包装器清理库按本轮差集删除，另 1 条账号边界用例自清理，最终无新增或误删。

正式服务验证前后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc021-campaign-20260727T163000Z`。`SHA256SUMS.final` 已逐文件验证通过，其 SHA-256 为 `bb99b1c1a991a3b93230782ae04a897b2440d2500e718859436a4a1bd796669a`。

本证据不等同于真实 worker 并发、杀进程级崩溃恢复、Management 确认链或浏览器导出/打开已经完成；这些仍保留为部署后业务复验门。
