# HC-004 / SR-025 生产发布与部署后闭环（2026-07-30）

## 范围与结论

本次只关闭 SR-025 的剩余缺口：账号日发送软上限查询此前只按 `account_id` 统计，会把其它 workspace 的已发送历史计入当前 workspace 并错误发出本域告警。修复在 `account_daily_sent_count` 查询中加入 `workspace_id`，不改变软上限“仅告警、不作为发送硬门”的既有产品语义，也不扩大到 HC-004 的其它开放项。

正式后端从 SHA-256 `efe5e1e13c4b5894bd6f4cb2840657637d72f2259dca866c1275f7681a76baf4` 切换为 `d0b7ffc63ce93a0e4f3ec09e1d8b64c06c578fd08879908c25d04caa531e027b`。前端未重建，继续使用既有 69 项正式静态资源；后台能力开关未改变，`EVOLUTION_ENABLED=false`、Cold Contact、Silence Signal 与 Knowledge Digest 仍关闭。

## Red、Green 与候选身份

- 旧实现 expected-red 保留：同一 `account_id` 的外域 sent Outbox 会使本域 `agent.account_daily_send_soft_cap_exceeded` 计数从期望 0 变成 1。
- 修复源码 `src/agent/gateway.rs` SHA-256 为 `199763780e921fd3086a9328b18305ebefabd0eb48ee0f658ae62c229d4ed543`；回归源码 SHA-256 为 `dec5e17b6bda5b50027246b3feecd090601be86cb8e7af9c77fbd77cfcb5ea0c`。
- Green 矩阵 4/4 通过，覆盖 Webhook 限流、SR-025 pacing/软上限、Outbox 幂等与 Reaction stop；测试数据库集合与生产运行态前后不变。
- release 候选 SHA-256 为 `d0b7ffc63ce93a0e4f3ec09e1d8b64c06c578fd08879908c25d04caa531e027b`，大小 `99,979,424` 字节。构建使用离线、单 job、1 GiB 磁盘熔断；共享 target 在构建后按全量清单恢复并独立复验。

该缺陷位于确定性的 Mongo 查询作用域，不经过 LM 决策路径；因此没有为了形式调用真实模型。动态证据使用真实 Mongo、真实 Dispatcher 和本地 MCP 协议 mock，直接验证业务发送与租户隔离不变量。

## 候选 smoke 与切换前门禁

候选在唯一随机数据库 `wechatagent_hc004_sr025_smoke_<uuid>` 上执行两阶段回环-only smoke：首次启动完成迁移，第二次完整验证健康与 69 项静态资源逐字服务；systemd 网络策略为 `IPAddressDeny=any`、`IPAddressAllow=localhost`。五类工作队列均为 0，随机库随后精确删除，正式 PID/ELF/健康前后不变。

切换前生产五类活跃工作均为 0，无残留 smoke 数据库。数据库基线为 reports 5、chunks 0、accounts 3、contacts 12、revisions 94、behaviorSignals 3,206、migrations 57；数据库 `totalSize` 约 125.7 MB。回滚需求估算约 901 MB，可用空间约 4.85 GB。

## 原子切换与回滚材料

发布脚本持全局发布锁，依次执行：冻结证据复验、二次空队列检查、保存旧 ELF、停止服务、全库 gzip 备份与 `mongorestore --dryRun`、原子替换 ELF、双健康检查、数据库基线比较、69 项静态资源逐字检查和启动日志门。任一未提交退出都会恢复旧 ELF；若新进程可能改写数据库，则先删除固定生产库并从已验证 archive 恢复。自动回滚未触发。

切换目录：`/opt/wechatagent/releases/hc004-sr025-v4-20260729T125134Z/switch-20260729t235000z`。保留：

- 旧 ELF `wechatagent.old`，SHA-256 `efe5e1e13c4b5894bd6f4cb2840657637d72f2259dca866c1275f7681a76baf4`；
- 全库压缩备份 `wechatagent.archive.gz`，大小 `39,593,713` 字节，并已通过 dry-run；
- 切换证据清单 SHA-256 `3212d6e0363cf5a1120cd4d25472e6ac75873ee64ed48f94946e248dd279a619`。

## 部署后状态与精确回归

- 正式 PID `2410064`，`NRestarts=0`，状态 `active/running`；磁盘 ELF 与 `/proc/2410064/exe` 均为新 SHA。
- 回环和公网健康逐字一致：`ok=true`、`evolutionEnabled=false`；69 项静态资源逐字服务通过。
- 生产数据库计数、五类活跃队列和 smoke 数据库清单与切换前一致；启动日志无 panic/fatal，关闭的后台能力没有误启动。
- 部署后同源测试 ELF SHA-256 `8d336638774f35863dc409d66c542ab0af2fc510a51988a505c5d10a93e5cb36` 精确运行 `sr025_pacing_ignores_same_account_history_from_other_workspace`：`1 passed; 0 failed`。测试自身删除随机数据库；`SCRIPT_RC/PROCESS_RC/CLEANUP_RC/FINAL_RC` 均为 0，生产 PID/ELF/健康与数据库基线前后逐字一致。

## 冻结证据

- candidate build manifest：`09f5e8b5c8bece39b93ab4d0c266855dbc8c346d8c434e0c18391fbe1aa35e77`；
- candidate smoke manifest：`35776d4c2c8282dfb156f8c45455698d03ef6925ac045a7309513d60df35939b`；
- switch preflight manifest：`b6574e918167b101ed3b6c9f24a7ab1af3848084acb79e0c1b233e726f6ca091`；
- switch manifest：`3212d6e0363cf5a1120cd4d25472e6ac75873ee64ed48f94946e248dd279a619`；
- independent postdeploy audit manifest：`0ac6fd7d436e41dc0240fc7443818d984d3453a9c3d3266d65fc8bfe0d544dc5`；
- postdeploy SR-025 regression manifest：`70c9ba74f3ab9faac7112fcfd070816b20f6575baa29fd3820daabc246610e6e`。

服务器发布根为 `/opt/wechatagent/releases/hc004-sr025-v4-20260729T125134Z`。本记录关闭 SR-025，不代表 HC-004 其它来源项已完成。
