# HC-029 生产发布与部署后闭环（2026-07-28）

## 发布范围与结论

本次只关闭 HC-029 对应的 SR-135 与 SR-136，不扩大到其它待办，也不改变运营开关。正式后端从 SHA-256 `5df573cf5aef14e5919e13157c5213d58e24219cc424098610fc5ee7f29a558b` 切换为 `11d9b6fd943eb67b48f9a4b5d4fa13c2e50e1612899c77d7478d7723cdb36954`；前端未重建，沿用既有 69 项正式静态资源。Planner、Cold Contact、Silence Signal 三个开关均未设置，继续按默认 `false` 关闭。本次结论是“代码与部署门关闭”，不是“主动触达 Worker 已获运营启用”。

## 部署前真实副本集验证

- SR-136：完整测试 ELF SHA-256 `db389e4d7b65e948fcf7639b4215b5ae1095758feee5eddd6d1ecbc02d33fed1` 在本机 `rs0` 随机库运行 m056 幂等与 ImportJob ABA fencing 红线 2/2，通过后集合清单不变。
- SR-135 首次真实运行 6/7：32 个同 intent 并发中，task/event 两次独立读取跨过 winner 的原子 commit，制造短暂 `1/2` 观察并误报 partial commit。修复把两次读取放入同一 `ReadConcern::snapshot()` 事务；同一快照的真实 `1/2` 仍 fail-closed，不掩盖持久损坏。
- 修正版测试 ELF SHA-256 `7a186e6952db68588b1cf69e554c730af59adb79c7a3fab7e657795fb8561573` 运行 7/7：同 intent 并发、不同 intent 总 cap、事件写失败回滚、旧事件基线追平、UTC 跨日、段/总 cap、Signal 重放全部通过。测试前后 126 个历史库清单均为 SHA-256 `cf9e290f7a2e44f46b749d9eb2de3a06b4d9d607e0ca31f89ffe394899e8c187`。

## 候选与切换

Release 候选以14个 HC-029关键文件身份门、离线单 job和800 MiB磁盘熔断构建，ELF为 `99,361,376` bytes。随机库候选冒烟验证：首次启动完整迁移（含 m056）、第二次启动幂等、五类队列全空、网络仅回环、健康 `ok=true/evolutionEnabled=false`、69项静态逐字服务；随机库随后精确删除，正式 PID与哈希未漂移。

切换前正式库数据约232.6 MiB、存储约119.1 MiB、索引约6.6 MiB；五类活跃队列为0，ImportJob为0，m056与配额集合均处于合法部署前基线。发布脚本停服后二次确认空队列，生成并 dry-run 校验全库 gzip archive，再原子切换后端。任一未提交退出都会恢复旧 ELF；若新进程可能修改数据库，则先删除正式库并从全库 archive恢复。自动回滚未触发。

## 部署后状态

- 正式 PID `2166141`，`NRestarts=0`，状态 `active/running`；磁盘 ELF、运行中 ELF和候选 ELF均为新哈希。
- m056 `2026_07_056_import_job_claims` 为 `applied`，现有 ImportJob 0 行、升级0行；`proactive_daily_quotas` 已创建，`proactive_daily_quotas_expires_ttl` 的 `expireAfterSeconds=0`，当前桶0行。
- 五类活跃队列均为0；内外健康逐字一致；69项静态资源逐字服务通过。
- 启动日志明确记录 Cold/Silence disabled，不存在 strategic planner、Cold或Silence loop started，也无 panic/fatal。
- 全库压缩备份 `39,608,276` bytes、旧 ELF、14文件源码快照和切换清单均保留于 `/opt/wechatagent/releases/hc029-20260728t090000z`。

部署后权威清单共10项，SHA-256为 `305a1f3caa6747a2bdbf4fc873c04cc8de0d1be17e860efbae60c2bb94231013`。工作区证据位于 `audit/hc029-source-audit/`；其中数据库证据哈希为 `cf50de80a8471ab8066b165e3b70823e8286c0b518274cff3aa01d00d26ac6d2`，启动日志证据哈希为 `46a62174f507e769b0f5fc608d5bba28416ff6d9529cec01c4fb82d9d8f46ab0`。
