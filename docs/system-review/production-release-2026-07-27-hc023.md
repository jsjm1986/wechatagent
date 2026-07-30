# HC-023 Guide protocol v3 部署后验证记录（2026-07-27）

当前正式 release 的 `guides.rs`、`guide_profile.rs`、UserOps Store、ConfigureView 与 Guide 契约文件和本地逐字一致；运行中后端含 frozen plan、candidate hash、强确认、事务 finalize 与稳定 receipt 协议。本轮没有切换二进制或修改正式业务库。

release 的 `transactional_admin_flows.rs` 整文件因本地后续追加其它用例而与本地哈希不同，但精确 Guide 红线函数块 418 行逐字一致，双方 SHA-256 均为 `a3cac908ded91e51849079e943e7616e64d7919b204b25bc05ff92a637e4cd85`。服务器以真实 `wa_session` Cookie Router 和 `rs0` 随机库执行该精确用例 1/1：错账号、错 candidate hash、缺强确认均在 lease claim 前完整零写；审计 validator 故障使 Contact、OperatingMemory、共享 Playbook 与 Preview 整体回滚；修正故障后成功提交，同 hash 重放返回逐字相同 receipt，Playbook 不重复升版。前端 Store/UI/契约三文件 37/37。

测试库 126→127→126，本轮唯一随机库按差集删除。正式服务前、中、后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc023-guide-20260727T173000Z`。`SHA256SUMS.final` 已逐文件验证通过，包含 release 源码哈希与测试块 provenance，其 SHA-256 为 `ce1fc888297e828012a9f362ab4da8cfe34e698f7a6e88ee454239503d9d9d7f`。

本证据不等同于真实管理员浏览器跨账号、强确认或迟到响应交互已经完成。该动态用例使用共享 Playbook 且 `domain_runtime_parameters=None`，因此也不替代 SR-094 的专用 runtime 参数 Mongo 写入验证。
