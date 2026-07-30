# 2026-07-25 生产发布证据

本记录只描述 2026-07-25 实际执行并观测到的生产事实，不把尚未运行的 Actions、真实模型或其它动态测试外推为已验证。

## 发布对象与上线前验证

- 用户明确回复“切换”，授权停服、切换点备份、原子替换，以及失败时恢复旧后端、旧前端和切换点数据库。
- 发布目录：`/opt/wechatagent/releases/deploy-20260724T202505Z`；切换记录：`switch-20260725T024528Z`。
- 旧后端 SHA-256：`b0b1a0cb56cb4abb03c1bb5d94ec9bc0c7396199a5d70318b82985ba0969198b`；新后端：`539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`。
- 干净前端共 69 个文件、1,205,446 bytes；`index.html` SHA-256 为 `039610c8d5a06206436c02c0bfc464cfee73c9f527d69e20dbcd02a76468c7df`。
- m049 副本集专用测试 1/1 通过，覆盖“m043 已 applied、planning-only draft 仍带 current”的升级形态，并断言内容不改写、指针清理、启动对齐和幂等。
- 候选运行器本地与服务器单测均为 9/9；门禁包括拒绝生产/系统库、迁移账本与空队列预检、`.env` 后置隔离覆盖、回环监听、loopback-only 网络、`PrivateTmp`、随机 `/run` 暂存和静态包逐字节验证。
- 真实升级克隆库联合冒烟通过：候选后端健康，m049 applied，`group.policy` 与 `moment.policy` 保持 `status=draft` 且 `current_version=false`，69 个前端文件均由同一候选进程逐字节返回。
- 冒烟按既有调度协议生成并完成 6 条 `outcome_aggregation`；它们均为 `sent/aggregated`，`agent_send_outbox=0`，且网络被限制为 loopback-only。

## 切换、回滚与生产结果

- 停服后制作归档：`/opt/wechatagent/backups/switch-20260725T024528Z/wechatagent.archive.gz`；SHA-256 `4381ee5b330bb7a6a00f136559ca2f69f74d0c1fd466c720fe81b7042c0e29ba`；39,623,517 bytes。
- 脚本先校验候选、回滚副本和前端哈希，再停服、备份、原子替换并启动；任一健康、静态资源、迁移或 Prompt 门失败都会恢复旧文件和切换点数据库。本次未触发回滚。
- 切换前 PID `1410248`；切换后 PID `1686295`。m049 于 `2026-07-25T02:45:46.087Z` 写入 `status=applied`。
- 生产 `group.policy`、`moment.policy` 均为 `status=draft`、`current_version=false`，业务内容未被迁移改写。
- 稳定观察 12/12：本机和公网 `/api/health` 均 `ok:true`，PID、后端和前端哈希无漂移，systemd 重启计数为 0。
- MongoDB 仍为副本集 `rs0` 可写主节点；观察期 `OUTBOX_OPEN=0`、近期失败任务为 0，日志未命中 panic/fatal/error 门。
- 旧后端、旧前端、切换点归档和切换日志均保留，未在收口阶段删除。

## 结论边界

- 本记录足以关闭 SR-008 的正式部署门，以及 m049 对 planning-only Prompt 指针的定向升级门。
- 它不自动关闭其它仍标注为 Actions、真实模型、真实 MCP、浏览器或特定 Mongo 动态验证待办的条目。
- 测试库和备份仍保留；后续清理必须另行确认。

## 部署后 Provider 生命周期专项

- 2026-07-25 从部署审查源码 `/tmp/wechatagent-sr008-release-20260724T143459Z` 编译 `tests/llm_provider_activate_integration.rs`；源码 SHA-256 `888f1bc5f6fff2aa7ef520f9579f6baa07005b26e2789f98d0afd20b22072cb`，最终测试二进制 SHA-256 `a3cc29c5e374918f50e595cfdf9aeb8e625a489200d9f3ff1dea54172fced589`。机械枚举确认目标恰好 5 条。
- 服务器 MongoDB 为单节点副本集 `rs0`。测试通过 `TEST_MONGODB_URI=mongodb://127.0.0.1:27017/?directConnection=true` 为每条用例创建独立 `wechatagent_test_<32位十六进制 UUID>` 库，在只允许回环网络、CPU 50% 和受限内存的 systemd 单元中串行执行；结果 `5 passed; 0 failed`，20.98 秒。5 个随机库均保留，没有执行 `dropDatabase`。
- 动态断言覆盖：Provider 不存在时拒绝；激活后同 workspace 恰一条 active；active 编辑缺许可或错版本时完整 BSON 与 Registry generation/provider/model 零变化；nullable tuning 省略保值、显式 `null` 真正 `$unset` 并返回全局默认来源；视觉能力关闭/删除零写拒绝、事务原子改派及 partial unique 索引拒绝双指派。
- 证据边界：SR-165 的拒绝路径已动态关闭；成功链随后也已真实执行，但当前模型 `kr/claude-opus-4.7` 与调用日志中另外 3 个历史成功模型对同一严格 JSON 合成请求均返回 HTTP 530、无模型正文，用户提供的 `127.0.0.1:9090` 在本机不可达且服务器 `/v1/models`、`/health` 均连接失败。因此“连通成功→签发合法许可→active 热切成功→Registry generation 增长→重启装载一致”仍为外部端点阻断，不记通过，也未用 mock 替代。同一测试二进制显示另有 24 个 filtered 用例，本记录不把它们算作已执行。
- SR-165 模型阻断原始证据固化于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/provider-sr165-model-blocker-20260725T20260725T051337Z`；`SHA256SUMS` 的 19 项内容全部复验为 `OK`。探测前后正式 PID `1686295`、重启计数 0、运行中与磁盘 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf` 及健康响应均一致。
- 原始证据固化在 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/provider-lifecycle-20260725T050238Z`；`SHA256SUMS` 中 25 项内容均复验为 `OK`。5 个证据库为 `wechatagent_test_23e61f3aeb924a68ab479c81982f653d`、`wechatagent_test_55b518fe7f3745d0a3679aff0c993cdd`、`wechatagent_test_997b82d873a443a38a79dc7d850dd27f`、`wechatagent_test_c5cbbb2d7b654f2ab58c28eec9a591ef`、`wechatagent_test_e2c7e00fb5504a42bd816c292bd5691f`。
- 编译时曾复用正式 Cargo target，使磁盘启动文件被同源码测试构建覆盖为 SHA-256 `746c859fc97cd6b12bbfb5351f3e5b654dfe80457dce6561cd4c9870b1557906`；运行中进程始终保持已发布映像且健康。覆盖文件已保留到 `audit/provider-compile-overwrite-20260725T125657Z`，随后从发布清单已验证副本 `candidate/wechatagent.m049` 原子恢复正式路径；未重启服务。恢复后及专项测试前后 PID 均为 `1686295`、重启计数 0、运行中与磁盘 SHA-256 均为 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，健康响应一致。

## 部署后 SR-168 Taxonomy 运行字段专项

- 使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅允许回环网络的临时 systemd 单元中连接随机隔离库 `wechatagent_test_93cc4a38200440c991942ed2da005b66`；通过真实 `wa_session` Cookie 中间件调用已发布 Router，不重新编译测试目标。
- GET `/api/admin/taxonomies` 返回 200，并投影探针行的 `priorityWeight=73`、`isTerminal=true`、`isReactivationTarget=true`。随后 PATCH 正文严格只有 `{"label":"renamed only"}`，返回 200；响应和 Mongo 读回均保留三个运行字段，且 `currentVersion=true`。
- 隔离单元 `wechatagent-audit-sr168-20260725T051725Z.service` 以 `Result=success/ExecMainStatus=0` 退出。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr168-deployed-isolation-20260725T051725Z`，`SHA256SUMS` 的 29 项内容均复验为 `OK`；随机测试库保留，没有执行数据库删除。
- 正式数据库只对随机探针 ID 做前后只读计数，结果均为 0。测试前后正式 PID `1686295`、重启计数 0、运行中与磁盘哈希及健康响应完全一致。本专项只关闭 SR-168，不外推为 HC-014 其它条目已完成。

## 部署后 SR-170/171 Evolution 发布门与统计窗口专项

- 使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在两个仅允许回环网络的临时 systemd 单元中连接同一随机隔离库 `wechatagent_test_72fa02d5048e4326a3571e19081f6f4a`；通过真实 `wa_session` Cookie 中间件调用已发布 Router，没有重新编译测试目标。
- SR-170：把 `thresholdAutoReleaseEnabled` 设为 true 被管理 API 以 400 拒绝；`EVOLUTION_ENABLED=false` 且 workspace flag 已开时人工 release 返回 400，随后在 env 开启但 workspace flag 关闭的第二实例中人工 release 也返回 400。两阶段前后 runtime flag 与 proposal 完整 BSON 逐字一致，`threshold_overrides`、`threshold_overrides_audit`、`post_release_reviews` 和对应 release event 均为零。
- SR-171：随机库中 25 个近 7 天实验各带一个候选。GET `/api/evolution/experiments?limit=20` 的浏览列表严格为20项，`aggregate7d` 则独立返回 experiments=25、proposals=25、released=5、rolledBack=3；coverage 为 `complete=true`、`source=server_time_window`、`windowHours=168`、`experimentsScanned=25`，证明统计不再由20条浏览列表推算。
- 两个临时单元均以 `Result=success/ExecMainStatus=0` 退出。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr170-171-deployed-isolation-20260725T053014Z`，`SHA256SUMS` 的35项内容全部复验为 `OK`；随机测试库保留，没有执行数据库删除。
- 正式数据库仅对随机 marker 做前后只读计数，experiments/proposals/flags 均为0。测试前后正式 PID `1686295`、重启计数0、运行中与磁盘哈希及健康响应完全一致。本专项只关闭 SR-170/171，不外推为 HC-017 其它发布协议或真实模型条目已完成。

## 部署后 SR-172 Outbox 运营投影与取消账号绑定专项

- 使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅允许回环网络的临时 systemd 单元中连接随机隔离库 `wechatagent_test_4123a8d865b549fbb6779f939ef56137`；通过真实 `wa_session` Cookie 中间件调用已发布 Router，没有重新编译测试目标。为避免 Dispatcher 抢占，夹具的 `next_retry_at` 均置于未来一小时。
- GET `/api/admin/outbox?accountId=account-a&limit=20` 返回 200、total/items 均为 4。text、同账号 media、referralCard 三类 typed payload 均保留不可变对象 id和可读元数据；媒体恢复状态为 `reclaimedInFlight=true/reclaimCount=2`。账号 A 引用账号 B 私有素材时只返回 assetId，title/fileName 均为 null；账号 B 控制行未泄露。
- 对账号 A 文本行以错误 `expectedAccountId=account-b` 取消返回 409；请求前后完整 Outbox BSON 与关联取消事件计数逐字一致。改用正确账号后返回 200，仅该行转为 `canceled`，并精确追加一条 `account-a/outbox_canceled` 审计事件。
- 临时单元 `wechatagent-audit-sr172-20260725T053911Z.service` 以 `Result=success/ExecMainStatus=0` 退出。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr172-deployed-isolation-20260725T053911Z`，`SHA256SUMS` 的35项内容全部复验为 `OK`；成功随机库经只读清单确认仍保留，没有执行数据库删除。
- `053442Z` 首次诊断因 `mongosh` 的 `NumberLong(number)` 弃用警告污染种数 JSON而在业务请求前退出；`053703Z` 第二次已完成全部 Router 动作，但 canonical EJSON 将整数 1 封装为 `$numberInt`，旧证据断言仅接受裸数字而退出。两次均为证据脚本格式问题，不记业务通过或失败；临时单元正常退出，诊断目录及随机库原样保留，正式服务均零漂移。
- 正式数据库只对随机 marker 做前后只读计数，outbox/assets/cards/events 均为0。最终测试前后正式 PID `1686295`、重启计数0、运行中与磁盘哈希及健康响应完全一致。本专项只关闭 SR-172，不外推为 HC-010 其它发送问题已完成。

## 部署后 SR-180 Evolution 人工发布代码政策硬闸专项

- 使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅允许回环网络的临时 systemd 单元中连接随机隔离库 `wechatagent_test_b06bd78443934d9590409e944befb033`，不重新编译测试目标。
- 运行进程只读环境快照严格为 `EVOLUTION_ENABLED=true` 与 `EVOLUTION_AUTO_RELEASE_ENABLED=true` 两行；Mongo 独立读回 workspace flag 为 `enabled=true/rollout_percent=100/threshold_auto_release_enabled=true`。随机库同时种入一个唯一账号和一条 `eligible_for_release` threshold proposal，证明历史三层配置闸与候选条件均已满足。
- 真实 Evolution worker 在 60 秒调度周期完成目标账号 tick，事件明确记录空 cohort 且 `auto_released_count=0`。tick 前后候选完整 canonical BSON 逐字一致，`threshold_overrides`、`threshold_overrides_audit`、`post_release_reviews` 和对应 release event 均为零，证明现有配置不能越过当前全部人工发布的代码政策硬闸。
- 临时单元 `wechatagent-audit-sr180-20260725T055044Z.service` 以 `Result=success/ExecMainStatus=0` 退出。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr180-deployed-isolation-20260725T055044Z`，`SHA256SUMS` 的32项内容全部复验为 `OK`；成功随机库经只读清单确认仍保留，没有执行数据库删除。
- `054558Z` 首次诊断运行中，worker 实际已连续完成两轮 `auto_released_count=0` 的目标 tick，但证据脚本错误地把 `{sort: ...}` 作为 `findOne` projection，轮询只得到 `_id` 并在超时后退出。该次只读诊断不替代最终完整证据，也不记为业务失败；诊断目录和随机库原样保留，临时单元正常退出，正式服务零漂移。
- 正式数据库只对随机 marker 做前后只读计数，accounts/proposals/experiments/events 均为0。最终测试前后正式 PID `1686295`、重启计数0、运行中与磁盘哈希及健康响应完全一致。本专项只关闭 SR-180，不外推为 HC-017 其它发布协议或真实模型条目已完成。

## 部署后 SR-181 运营记忆撤销生命周期专项

- 使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅允许回环网络的临时 systemd 单元中连接随机隔离库 `wechatagent_test_5532df823add4c229ef3120b5256d2f9`；通过真实 `wa_session` Cookie 中间件调用已发布 Router，没有重新编译测试目标。
- active 列表起初只返回目标记忆。以已注册的错误账号或错误 operator 撤销均返回 404；两次请求前后目标记忆完整 canonical BSON 与关联撤销事件计数逐字一致。正确 scope 撤销返回 200，写入首次管理员、原因和时间；默认列表随后为空，`includeRevoked=true` 仍返回完整审计行。
- 对同一行重复撤销返回 200 且 `alreadyRevoked=true`，首次 `revokedBy/revocationReason/revokedAt` 保持不变，撤销事件仍精确为一条。该服务器专项没有通过模型路径新增记忆，也没有用手工 Mongo 重加冒充生产 helper；生产 helper 生命周期由下述同源本机动态补证独立关闭。
- 临时单元 `wechatagent-audit-sr181-20260725T055632Z.service` 以 `Result=success/ExecMainStatus=0` 退出。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr181-deployed-isolation-20260725T055632Z`，`SHA256SUMS` 的47项内容全部复验为 `OK`；成功随机库经只读清单确认仍保留，没有执行数据库删除。
- 正式数据库只对同一随机内容、账号 alias 与实际撤销原因做前后只读计数，均为0。测试前后正式 PID `1686295`、重启计数0、运行中与磁盘哈希及健康响应完全一致。
- 同源本机动态补证：服务器冻结部署源码与本地 `src/agent/memory.rs` SHA-256 均为 `7dbadb50add6f1e029760abf3fbee516c34fd4c51edd506c2f590d41b598669c`。SR-181 生产 helper 在本机 MongoDB 8.0、全新回环 dbpath 中完成 `record → active read → wrong-scope zero-write → revoke → no-injection → repeat-idempotent → re-add` 1/1。首次成功运行暴露测试末尾漏调 `app.cleanup()`，旧随机库作为诊断证据保留；仅修补测试清理后，在第二个全新 dbpath 再次 1/1，随后只读数据库清单为 `[]`。最终证据目录为 `target/sr181-mongo-40de1a10157a4ce392e0026dbbd7dcb0`；系统 MongoDB 服务前后保持 `Stopped`，27017 无残留监听或 mongod 进程。
- SR-182 的生产实现与完整 `tests/memory_card_invariants.rs` 均和服务器冻结部署源码逐字同哈希（测试 SHA-256 `cbd9e0b5fd440485cb0c045b4b57176c4116074a7ae83ae7d944eafa64010901`）；完整目标 20/20 通过，覆盖 cap、稳定优先级、容量淘汰迁入 recent、跨轮审计继承、显式 discarded 与重新升入 core 后清理。证据目录为 `target/sr182-pbt-20260725T142158`。该确定性契约不依赖模型或浏览器；在该批次时仅关闭 SR-181 helper 红线与 SR-182，尚未结算 SR-029；SR-029 与 HC-011 后续已由下方正式部署专项闭环。

## SR-029 部署前 Memory commit 恢复协议补证

- 工作树已移除管理员手动整理的无 claim 多集合写旁路：`POST /contacts/:id/memory-consolidation/run` 现在必须创建 contact-scoped single-flight task，已有 owner 时返回 409；创建成功后只经统一 claim、heartbeat、`running → committing`、prepared payload与幂等 reconcile 执行，任务未到 `sent` 不返回成功。后台 worker 的既有 durable协议保持不变。
- 本机 MongoDB 8.0、全新仅回环 dbpath 运行 `tests/sr029_memory_commit_recovery.rs`，结果 `2 passed; 0 failed`。断言覆盖手动入口独占和并发 owner零写，以及 prepared commit 从全未应用、仅主卡已应用、联系人/候选/事件部分完成三个崩溃窗口连续两次重放后的恰好一次收敛。证据目录为 `target/sr029-mongo-4a0895ef9fb342e2b3df70c412bd7f0a`；测试数据库只读清单为 `[]`，系统 MongoDB 服务前后均为 `Stopped`，27017无监听或残留进程。
- 生产实现文件当前本地 SHA-256：`src/agent/memory.rs=84bad8711847bda2d826182ad366df1935ffea125cf3d0c3081574b5032956aa`、`src/agent/mod.rs=af539951b30e9adcefafc6152d7672cc2296b719fb31448e76af79df38d25e2b`、`src/routes/contacts.rs=76db6b9de1f0a04f4b8d72b82d3d81e9600885d2af44eab1ebbeb4ef0f62b471`；红线文件 SHA-256 为 `e8802ec4762c565968bbfb1db2a773b756f126a914f94582f5dcd2f995c59056`。
- 在该部署前核验时，正式服务仍运行磁盘与进程同哈希的旧二进制 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，PID `1686295`、重启计数0、健康正常；根分区仅余 `2133876736` bytes（约2.13GB）。本段只记录当时的部署前边界，不把本机通过冒充已部署；当时开放的 SR-029 与 HC-011 部署后门随后已由下述正式部署专项关闭。

## SR-029 正式部署与部署后 Memory commit 专项

- 经用户明确授权，先精确清理两个无进程占用、无证据引用的历史构建目录，释放 `18586066944` bytes；正式 PID、重启数、旧哈希和健康均未变化。随后从冻结部署源码 `/tmp/wechatagent-sr008-release-20260724T143459Z` 创建隔离候选，只替换 `src/agent/memory.rs`、`src/agent/mod.rs`、`src/routes/contacts.rs` 三份已逐 hunk 审核文件；全树差异数量严格为 3。独立 target 离线单作业 release 构建成功，候选 SHA-256 为 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b`，96,796,368 bytes。
- 候选先在随机库 `wechatagent_sr029_smoke_f8b56e45191f496cbc78cf34fd349321` 完成 49 条迁移台账、五类 worker 队列全空、仅回环双健康及 69 个静态文件逐字验证；随后真实 `wa_session` Cookie Router 首次手动整理返回 200，唯一 durable task 收敛为 `sent/no_candidates` 且 claim/active key 清理；插入 active owner 后第二次返回 409，blocker 完整 canonical BSON 与任务总数前后不变。正式服务和正式库 marker 全程零漂移。
- 生产切换脚本在停机前及停机后双重确认活动 task、memory task、发送中 Outbox、import 与 knowledge task 均为 0，保存旧二进制并同盘原子替换；两次健康、磁盘/运行中哈希和 fatal 日志门均通过，自动回滚未触发。正式哈希由 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf` 切为 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b`，PID 由 `1686295` 切为 `1740029`，重启计数保持 0。切换证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/switch-sr029-20260725T090151Z`，稳定 `SHA256SUMS` 全部复验为 `OK`。
- 切换后直接使用正式路径新二进制，在全新随机库 `wechatagent_sr029_smoke_184da04d29ce45e489e8ef55d9f1e81f` 再次完成迁移、空队列、仅回环健康和 69 个静态文件验证；随后正式路径 Router 红线再次得到 `FIRST_HTTP=200`、`SECOND_HTTP=409`、`ZERO_WRITE_CONFLICT=1`。常驻 PID `1740029`、重启计数 0、磁盘与运行中哈希及健康前后一致，正式数据库随机 marker 为零。
- 部署后通用烟测证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr029-deployed-smoke-20260725T090452Z`，清单 SHA-256 `9bc5043f0024be250b6c44c25611725d32e2dbfb84646a8b9e3568cb63097dd2`；Router 证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr029-deployed-router-20260725T090845Z`，清单 SHA-256 `89c2443511492c0be20775b3f317fed33ebbb0218fbf25758e638bdbfe3a7b30`。两份清单内容全部复验为 `OK`。本专项关闭 SR-029 与 HC-011 的部署后门，不外推关闭其它 SR。

## Playbook 发布态与默认指针部署前服务器专项（2026-07-26）

- 本专项验证当前工作树候选，不代表已正式部署。最小源码归档不含 `.env`、Git 元数据、target、node_modules 或前端产物，SHA-256 为 `1691e0e13844564d672d8adfb8e598db1b32a35a29f2ddb4441d775f2d58e466`；在 `/tmp/wechatagent-playbook-20260726T184747Z` 独立 target 中以 Rust 1.92 离线单作业完成两个测试目标构建。`playbook_scope_integration` 二进制 SHA-256 为 `28639c573ee6fa9566dc0e0f30a6c55ecc567717f1ff11508fb7dd319c371007`，`contact_operation_profile_integration` 为 `7c84c248f3da0ac785e42f65c3215093e67ec3fd5655eaf393aeed241160fb48`。
- 四个只允许回环网络的临时 systemd 单元以 `TEST_MONGODB_URI=mongodb://127.0.0.1:27017/?directConnection=true` 串行连接服务器单节点副本集 `rs0`，结果 `4 passed; 0 failed`：错误账号/旧版本完整 Playbook BSON 零写；AI draft 发布、旧默认降级与新默认提升同事务完成且最终恰一默认；跨账号 Playbook 绑定完整 Contact BSON 零写；draft Playbook 绑定完整 Contact BSON 零写。
- 两个拒绝用例保留证据库 `wechatagent_test_83b1df4a51de4dee8e36874579cedd5e`、`wechatagent_test_930769e2d3e44a99814f32339d21154f`。只读投影确认联系人 `playbook_id=null`、`follow_up_policy=null`；跨账号候选仍为另一账号的非默认 published 行，AI 候选仍为非默认 draft。另两条调用 `app.cleanup()` 的用例未留下随机库。
- 测试前后正式服务 PID `1740029`、重启计数 0、运行中与磁盘 SHA-256 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b`、健康响应与 `rs0` 可写主节点状态均一致；未修改 `/opt/wechatagent`、未重启服务、未连接业务数据库。19 项原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/playbook-release-20260726T1918Z`，清单 SHA-256 `afb1c67d781178cfe0ff16d89847e230e30b153d4396cafa6bc0e01a750be297`，全部复验为 `OK`。
- 为保留可复核证据，远端 5.8 GB 隔离构建目录和两座随机证据库尚未删除；清理需另行明确授权。本专项关闭 SR-070/071/151A 的副本集动态门，但正式部署和部署后浏览器/业务回归仍待执行。

## 服务器未执行项回填 batch2（2026-07-26）

- 本批验证当前工作树的隔离源码，不代表已部署到常驻服务。所有测试均在临时 systemd 单元中运行，只连接服务器本机单节点副本集 `rs0` 的随机 `wechatagent_test_*` 库；Router/MCP 仅使用 loopback，未访问真实 MCP/WeChat，也未连接业务数据库。
- SR-066：机械枚举五条 `delivery_redline_*`，localhost MCP mock 下完整结果 `5 passed; 0 failed`。首轮 1/5 的四个失败均因旧夹具没有注册 scoped MCP 账号；只补种测试账号后重编译并完整重跑 5/5。最终断言覆盖发送前取消零远端调用、远端接受后的晚取消恰好一次收敛 sent、HTTP 歧义与两种名片不确定送达均进入 `delivery_unknown` 且禁止自动重放。真实模型 T4 与真实外部 MCP 未执行。
- SR-016：临时 TCP Router + 随机 Mongo 红线 `1 passed; 0 failed`，覆盖 login/token 共用窗口、第三次正确凭据仍 429、`Retry-After`、三条脱敏审计及严格 90 天 retention。
- SR-121：两条 Digest 快照恢复红线 `2 passed; 0 failed`，证明失败重算保留上次成功快照，晚到旧代次不能覆盖较新成功。
- SR-123：完整 Worker 目标最终 `3 passed; 0 failed`，覆盖 typed verdict、committed 对应真实 draft 副作用、混合 verdict 持久化与失败 summary。首轮为 `2 passed; 1 failed`，失败原因是测试绕过真实任务入口时漏种 session identity；生产多租户门没有放宽，仅补齐与 Task 同 scope 的测试 Session 前置行后完整重跑。
- 本批运行前后 `wechatagent_test_*` 数据库数量均为 95；这些是前序保留证据库，不属于本批新增清理对象。常驻服务始终为 PID `1740029`、重启计数 0，`/api/health` 返回 `ok=true` 且 `evolutionEnabled=false`；Mongo 始终为 `rs0` 可写主节点。未重启或修改常驻服务。
- 原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/server-backfill-batch2-20260726T1547Z`；`SHA256SUMS` 内容复验通过，清单 SHA-256 为 `aa6f91f0521dd5ffbbcef636122fcdd3994f7d4b5c1fa04e776e77cca71e08c7`。诊断失败轮次、最终日志、测试清单、源码/二进制哈希及只读数据库投影均保留。本批不关闭真实模型、真实外部链路、工作树部署或部署后回归门。

## SR-125 Digest 派工授权服务器专项（2026-07-27）

- 本专项验证当前工作树隔离候选，不代表已部署到常驻服务。测试经真实 `wa_session` Cookie 中间件调用正式 `POST /api/knowledge/chat/tasks` 与 `GET /api/knowledge/digest/today` 路由，只连接服务器本机单节点副本集 `rs0`，临时 systemd 单元仅允许 loopback，不调用 LLM、MCP、WeChat 或业务数据库。
- 最终测试二进制 SHA-256 为 `7f94fda0ff04992d365eca3cdedfe8fa5ab48c41bba5ecd3654beb607e423388`，机械枚举恰好三条，完整结果 `3 passed; 0 failed`：画布直派从服务端 Digest 哈希协议权威重建 action/summary/target；Chat 确认要求并持久化同 workspace/account/session 的候选封印；过期 generation、跨管理员复用 Session 与 Digest/request 账号错配均返回拒绝，Task 与 `task_progress` 前后计数不变。
- 首轮结果为 `2 passed; 1 failed`：过期请求先合法占有 Session 后，另一管理员复用同一 Session 触发 `findOneAndUpdate(upsert)` 的 `_id` 冲突；MongoDB 2.8.2 将该冲突包装为 `ErrorKind::Command(11000)`，旧 helper 只识别 WriteError，因而错误返回 502。生产修复只扩充重复键分类为 Write/BulkWrite/Command 三种载体的 11000/11001，所有权 filter、upsert 与 409 业务协议均不放宽；修复后完整重编译并重跑 3/3。
- 首轮 panic 保留诊断库 `wechatagent_test_0d9f3cfe987346528d7e189049c42a3d`。只读投影确认该库仅有 owner 的 Session identity，Task 与 progress 均为 0；最终重跑调用 cleanup，测试库总数保持 96。常驻服务始终为 PID `1740029`、重启计数 0、健康 `ok=true`、`evolutionEnabled=false`，未替换二进制或重启服务。
- 原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/server-backfill-batch3-sr125-20260727T0030Z`；12 项内容与 `SHA256SUMS` 全部复验通过，清单 SHA-256 为 `3f76ecb896d881b936398eb188402751b93f912561363451fffe655ef450cf10`。该专项当时只关闭 SR-125 的真实 Cookie Router+Mongo 动态门；真实模型驱动任务随后由下方 HC-028 专项关闭，工作树部署与部署后业务回归仍开放。

## HC-028 Digest → Chat → Task → Worker 真实模型业务专项（2026-07-27）

- 本专项验证当前工作树隔离候选，不代表候选已部署到常驻服务。新增 `tests/hc028_real_digest_task_e2e.rs`，不使用历史 real-LLM skip 宏；缺 Provider、上游失败、空 Digest、缺封印候选、Task 非 completed、outcome 非 committed、空 repair patch 或任何 MCP 请求都会令唯一硬门失败。业务写全部进入 `TestApp::start_repl_set()` 的随机 `wechatagent_test_*` 库；正式配置库只读 Provider，外发内容仅为合成退款条款，不含项目代码、业务数据或密钥。
- 首轮 active Provider `kr/claude-opus-4.7` 在 319040ms 后以 Cloudflare 524 失败；Qwen 返回 400 `Arrearage`；Messages Provider 返回认证后端 503。NVIDIA DeepSeek 与 Llama 均真实调用成功，但 Digest 卡片为 0。该现象定位到共享 JSON 解析器会把单元素对象数组 `[{...}]` 归一成对象，而 Digest 只接受数组或 `{cards:[...]}`。生产修复仅在 Digest 边界把具有 `kind/title/suggestedAction/severity` 的单对象恢复为一项，随后仍走原 kind/action/severity/title/targetRefs 闭集；无关对象和非法枚举仍拒绝，正反单测 2/2。
- 修复版测试二进制 SHA-256 为 `f7689734e30b5229217141ef3797cde18d5b33afa7daf5da47c2d43beb1fd66d`，机械过滤恰好 1 条。最终使用隔离配置中的 `meta/llama-3.2-90b-vision-instruct`，结果 `1 passed; 0 failed`（379.62s）：Digest 成功提交 1 张 `fix_chunk` 卡；Cookie Chat 返回并持久化 candidateHash；Task 收敛为 `completed`，唯一 outcome 为 `committed`；`repairDraft.patch` 非空且含 2 字段；原 Chunk 保持 draft/needs_review/sourceQuote=None，未自动应用；`knowledge.digest.compose` 与 `knowledge.chunk.repair.propose` 两条 LLM 审计均成功；MCP 请求账本为空。
- `CapabilityEvidence` 终态为 `attempted=true / llm_calls=2 / branch=cookie_digest_chat_seal_worker_repair / artifacts=6 / assertions_run=18 / verdict=pass`。成功随机库由测试自动清理；5 个失败诊断库及4个隔离 Provider 配置库为复核保留，未执行数据库删除。生产 active Provider仍恰好1条，未修改正式 Provider选择。
- 脱敏证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/hc028-real-digest-task-20260727T0128Z`；12项 `SHA256SUMS` 全部复验为 `OK`，清单 SHA-256 为 `bba45706c94aa4f3d770908ede815e8954008bae3188473edf11492a2b709029`。测试前后常驻服务始终为 PID `1740029`、重启计数0、健康 `ok=true/evolutionEnabled=false`；未替换 `/opt/wechatagent/target/release/wechatagent`，因此只关闭 HC-028 的真实模型业务门，工作树部署与部署后回归仍开放。

## SR-001～183 工作树正式切换与 HC-028 部署后回归（2026-07-27）

- 发布输入由当前工作树中实际存在的全部受跟踪文件与明确源码类未跟踪文件组成，精确排除 `.cargo-target-sr175`、`.tmp`、`frontend/.tmp`、真实环境文件、构建产物与缓存。归档包含 1340/1340 个预期文件、无额外或危险路径，SHA-256 为 `e66f10556d893c5759094399fe63e7a5a95d44acfaa473010e3f93c2d1c28af0`；服务器逐文件展开清单 SHA-256 为 `433eb94088434200a456ba5aca4ce062b164125e5ff07716be4162cee5aafbaa`。
- 独立 release `/opt/wechatagent/releases/deploy-20260726T175122Z` 使用 Rust 1.92 与锁文件完成后端、`migrate_only` 和前端生产构建。候选后端 SHA-256 `f4863fa4401ead96a2c8cecbcadfe6328474d5020d22b115269abf67196119e8`，迁移工具 `b71d9e6c3b5959fa4a7e35565aa7cd7f38590499bb5cc2a8b11eb1be3ceb4143`，前端首页 `3233bf97aa43a374f2a4f85c1fd4e878b36a0d09909d61f5ed4e40e2c0b4ff6d`，69 项前端清单 SHA-256 `780256fa9a29962d09a94c1fe35d6a351f2c60d8aaffe3b881dd00796af5e6ce`；构建0错误，只有既有 dead-code告警。
- 正式库约 233 MB、六类开放队列均为0。部署前先把正式库流式复制到随机测试库，候选 m050～m054 两轮运行均 `applied` 且时间戳/队列状态不变；再以仅回环网络启动候选，连续健康、69 项静态资源逐字校验及72个集合启动前后哈希零变化。迁移演练和生产形状启动证据分别位于 `audit/migration-rehearsal-20260726T181626Z` 与 `audit/production-shape-startup-20260726T182125Z`。
- 正式切换脚本经服务器 `bash -n` 与显式只读 dry-run 后执行；停服前后双重确认活动队列为0，保存39,619,344字节正式库备份、旧二进制和旧前端，再同盘原子替换。后端 SHA-256 从 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b` 切为 `f4863fa4401ead96a2c8cecbcadfe6328474d5020d22b115269abf67196119e8`，PID从 `1740029` 切为 `1923420`，`NRestarts=0`；m050～m054 在正式库全部 `applied`，迁移台账总数55，前端69项与双健康门通过，Evolution仍关闭，自动回滚未触发。切换证据位于 `switch-full-20260726T183112Z`；初始清单因生成后 `tee` 继续追加 `switch.log` 而恰好1项失配，该失败原件已保留，冻结日志后最终88项全部通过，最终清单 SHA-256 `20a592b2e5f9f60663f50d6429b360a8be96d6ac2fb3dcc2fb22cf1ab076d869`。
- 切换后直接使用正式路径二进制在全新随机库完成迁移、仅回环健康和69项静态资源逐字验证；证据位于 `audit/deployed-smoke-20260726T183356Z`。初始清单因生成后 `run.log` 继续追加而恰好1项失配，失败原件与复验输出均保留；冻结日志后最终11项全部通过，稳定清单 SHA-256 `878bd8fb30f96af1f9560eddb11974aac4f9d705525e47a2b732e5c9fe6ecceb`。该烟测只证明部署产物可启动，不替代各专项业务断言。
- 随后从同一已部署源码重新编译唯一 HC-028 测试目标，二进制 SHA-256 `934bd25c683ec95b6a209c191807c0bffce109b12a7656b99b222c1aa62738fa`，机械枚举恰好1条。显式只读选择未激活的 `meta/llama-3.2-90b-vision-instruct`，结果 `1 passed; 0 failed`（60.94s）：Digest 1卡、Cookie Chat候选封印、Task completed、唯一 committed outcome、两字段 repair patch、原 Chunk未自动应用、两阶段 LLM审计成功且MCP零请求；能力账本为 `llm_calls=2/artifacts=6/assertions_run=18/verdict=pass`。随机测试库自动清理，测试库计数前后均101；正式 active Provider始终恰好1条，备用 Provider未激活，常驻服务保持 PID `1923420`、零重启和健康。部署后证据位于 `audit/hc028-deployed-real-20260726T185708Z`，12项清单 SHA-256 `8337dad337b8454852c1853c3bc20a8400017719d406a0fd2fdb348513ed3d9b`。本专项关闭 HC-028 及 SR-120、SR-121、SR-122、SR-123、SR-125 的部署与部署后业务回归门，不外推关闭其它仍明确开放的专项。
