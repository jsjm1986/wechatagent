# HC-022 联系人导入与纳管协议部署后验证记录（2026-07-27）

当前正式 release 的 `contacts.rs`、`shared.rs`、`contacts_batch_enable.rs`、Roster 页面、Contact Store 及三份直接前端测试源码和本地逐字一致；运行中后端包含 `initial_profile_enrollment` durable intent、reconciler、enrollment token 与 task claim generation fencing。本轮没有切换二进制或修改正式业务库。

服务器在真实 `rs0` 随机库运行当前完整 `contacts_batch_enable` 目标，机械计数和实际输出均为 13/13。覆盖稀疏导入保留身份字段、非真人和错账号候选完整零写、任务插入失败不留下 managed 半提交、并发单飞、代际旋转后同请求重新纳管、幂等处理，以及联系人缺失或取消纳管时旧画像任务安全终结。前端 Roster、联系人视图与 Contact Store 专项 22/22。

测试库 126→139→126，13 个本轮随机库均按测试前后差集删除，无新增或误删。正式服务前、中、后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc022-contacts-20260727T170000Z`。`SHA256SUMS.final` 已逐文件验证通过并包含 8 份 release 源码哈希，其 SHA-256 为 `d895a5bd26020001f71fd28143ad2e8148fb6a6fd4f058d6171b959492e45c1a`。包装器最初摘要曾按辅助函数误估为 14，未重跑业务；已依据 `#[ignore]` 机械计数和测试输出共同纠正为 13/13并重算清单。

本证据不等同于真实生产 worker 杀进程级恢复或认证管理员浏览器业务复验已经完成；同进程 validator 故障与 reconciler 恢复不能冒充进程崩溃演练。
