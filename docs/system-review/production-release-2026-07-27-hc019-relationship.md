# HC-019 关系审核部署后验证记录（2026-07-27）

本批只覆盖 SR-058/059/060 的关系建议路径。当前正式 release 已包含可信 actor、关系审核事务与 m045 pending 周期索引；未切换二进制，也未修改正式业务库。

服务器真实 `wa_session` Cookie Router+单节点 `rs0` 随机库运行 `relationship_review_ignores_spoofed_actor_and_uses_authenticated_admin` 1/1：认证管理员覆盖伪造 actor；建议终态与联系人画像同事务提交；终态后可创建下一 pending 周期，同周期第二条被 E11000 拒绝；联系人写故障使建议 CAS 与画像整体回滚。

业务测试输出为 `1 passed; 0 failed`。原始 systemd 单元最终 `rc=1`，原因仅是测试成功后的证据哈希命令使用相对路径却未切换到证据目录。业务未重跑；`evidence-wrapper-adjudication.txt` 保留原始失败阶段并明确裁定。随机测试库已删除，数据库集合 126→127→126。

正式服务验证前后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

冻结证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc019-relationship-20260727T124500Z`。`SHA256SUMS.final` 已逐文件校验通过，其 SHA-256 为 `03716865cbfa27e089483bb4dca50d64e965d9ea27ba4cdfcaae0a413c0060a6`。

## SR-057 成交审批补充验证

同日又以已部署同源 Handler 在服务器真实 `rs0` 运行三条精确用例 3/3：成功审批同行提交 signal、`staff_confirmed` outcome 与审计，重复审批零增量且认证 actor 覆盖请求伪造值；负金额首写前失败并保留 pending；审计 validator 拒写时三处整体回滚。三个随机库均删除，数据库集合 126→129→126，正式服务前后零漂移。

补充证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/sr057-suspected-deal-20260727T130000Z`。`SHA256SUMS.final` 已逐文件校验通过，其 SHA-256 为 `e98047eeb2c78d62b8fe868aa08681cec6b1db50d9ab65ba7697671e909fe1b4`。

## SR-097 Lesson 晋升隔离验证

同日完成 SR-097 工作树实现，但没有切换正式二进制：Lesson `_id` 与 `lesson_promotion + lesson_id` 来源锚形成确定性一对一身份，Chunk、Lesson CAS 和审计事件在一个 Mongo 事务提交；m055 在首写前审计旧关系并只回填精确配对，随后由 Lesson unique 与 Chunk 来源 partial unique 索引锁住不变量。

稳定部署基线目标类型检查通过；m055/索引纯单测 4/4，服务器真实 `wa_session` Cookie Router+`rs0` 3/3。覆盖并发晋升收敛为一 Chunk/一审计、重放返回稳定 Chunk、审计故障整体回滚后可重试、迁移精确回填，以及存在孤儿时在任何待回填行写入前失败。

随机测试库前后均为 126、差集为空；正式服务始终为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

隔离证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/sr097-lesson-promotion-20260727T140000Z`。`SHA256SUMS.final` 已逐文件验证通过，其 SHA-256 为 `169fd6c608e550d1df54f6cd0ce8e6b4fc4cf1a195b2553ba31f64d3959196af`。本节是部署前隔离验证记录，不构成 SR-097 已正式部署的声明。

## SR-067 统一收件箱成交核实部署制品观测

当前正式 release 后端源码、运行中二进制与前端静态制品均包含 `suspected_deal` 第九源及 `suspectedDealReview` 专用卡片。collector、`?source=suspected_deal` 与 summary 使用同一 `workspace_id + status=pending` 口径；卡片展示证据、联系人、置信度和出现次数，并复用已部署的事务 approve/reject 路径。本轮没有切换二进制或修改正式业务库。

前端收件箱/卡片/API/Store 专项 16/16、TypeScript 与生产构建通过。服务器真实 Handler+Mongo 精确用例 1/1：当前 workspace 的 pending 集合、过滤 inbox 与 summary 计数一致，approved 历史和其它 workspace pending 均不进入结果。测试库 126→126、差集为空。

正式服务验证前后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。服务器没有可复用认证会话，因此本轮不声称完成正式 Cookie HTTP 交互；结论边界为“已部署制品观测 + 隔离真实 Handler/Mongo 验证”。

隔离证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/sr067-unified-inbox-20260727T150000Z`。`SHA256SUMS.final` 已逐文件验证通过，其 SHA-256 为 `90965551a934c737231937149bc231d0eef14f79b5b60908535cba0819b062cb`。

## SR-169 ReviewQueue 对象身份部署制品观测

当前正式 release 的 `ReviewQueue` 与三份专项测试源码和本地逐字一致，正式静态制品包含“待办列表已刷新，请在最新条目上重新操作”的运行标记。共享队列以对象 id 作为直属 React key，并在每次接纳新列表时提升 generation；动作前校验 generation、对象仍存在及当前非刷新态，陈旧闭包保持副作用零调用。

本地重跑 `ReviewQueue`、`TaxonomyCandidateReviewCard` 与 Ask‑Human 数据源三份专项 17/17；同一工作树的 TypeScript、生产构建和 scoped diff 门通过。`[A,B]→[B]` 回归锁定 B 的 URL 与 canonical body，独立旧 generation 回归锁定副作用零调用且无 React key warning。

正式服务保持 PID `2021387`、`NRestarts=0`，磁盘与运行中后端 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。服务器无可复用认证浏览器会话，因此本节不声称部署后真实点击交互已通过；结论边界为“正式静态制品观测 + 确定性前端验证”。

制品证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/sr169-review-queue-20260727T153000Z`。`SHA256SUMS.final` 已逐文件验证通过，其 SHA-256 为 `5885a0e3e0f3cb17a663f5987cb3980f2823bd71d2c343c38df8de8c0277dbd0`。
