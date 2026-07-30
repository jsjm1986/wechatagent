# SR-129 / SR-130 生产发布记录（2026-07-27）

## 发布范围

本次只发布 HC-027 中的 SR-129 与 SR-130，不关闭 SR-132、SR-141、SR-173 或整个 HC-027。候选从已部署源码基线构建，并由脚本机械限制为以下 5 个生产文件：

- `src/routes/knowledge/chat.rs`
- `src/routes/knowledge/verify.rs`
- `src/knowledge_task/mod.rs`
- `frontend/src/features/knowledge/cockpit/AutoVerifyPanel.tsx`
- `frontend/src/features/knowledge/cockpit/ReviewChat.tsx`

未发布本地测试文件或工作树中的其它改动；本批没有数据库 migration。

## 行为闭环

- SR-129：Auto Verify 请求统一使用 `confidenceThreshold`、`humanAuditSampleRate`、`limit`；后端 camelCase DTO 启用 `deny_unknown_fields`，错误命名不能再静默回落默认值。
- SR-130：Review Chat 冻结 `accountId + chunkId + expectedUpdatedAt + operation=update`，只接受匹配 `targetChunkId + expectedUpdatedAt` 的顶层 `draftPreview`。Apply 在 Mongo 事务内按冻结 `updated_at` 做 OCC；陈旧版本返回 `chat_chunk_snapshot_stale`，业务对象与成功回执均零写。

## 验证证据

- 最终 5 个生产文件上传后与本地 SHA-256 逐字一致，范围统计与必需契约标记通过。
- 授权服务器 `rs0` 精确枚举并运行 `stale_chunk_snapshot_rejects_chat_apply_with_zero_write`：`1 passed; 0 failed`。测试库集合与正式运行态前后一致；证据目录 `audit/sr129130-isolated-mongo-20260727T034200Z`，清单 SHA-256 `3990137e9c7687982d344cbf045f32b44cf333a736c005a50b942eff83702001`。
- 后端 release 构建与前端 production build 通过。候选证据目录 `audit/sr129130-candidate-20260727T040500Z`，清单 SHA-256 `c7fcd3b444c716d3475446018a23716f23d47bef8fc5c11a721dedfecea6a109`。
- 候选以 transient systemd unit、仅回环网络和随机库运行；双健康、69 项静态资源逐字校验、准备后的目标库与源演练库零变化，unit 与临时库均清理。证据目录 `audit/sr129130-candidate-smoke-20260727T044500Z`，清单 SHA-256 `3eeb3e7c0f8f0c70e16a88ca90af3b62970eb043882bf9fc16b536f3795bb07b`。

## 切换与回滚

切换脚本先以纯只读 dry-run 核验候选/当前哈希、三份冻结证据、六类活动队列、正式健康和完整 migration 快照；apply 时停服后二次确认活动队列为 0，保存数据库、旧二进制、旧前端与 5 个旧源码文件，再同盘替换。失败自动恢复运行产物与源码；因本批无 migration，数据库备份只作为人工取证后的回滚材料，不执行自动 `mongorestore --drop`。

- 旧后端 SHA-256：`f4863fa4401ead96a2c8cecbcadfe6328474d5020d22b115269abf67196119e8`
- 新后端 SHA-256：`c98f24a34404cd39bc5427cdfe4c25af84a984fc820e27867cb75f772e7a54ab`
- 新前端首页 SHA-256：`9c60cad33e403b12478c6fea263e4e6b5ccd5cbd7253fc012f6385313d9c1b0f`
- 切换后 PID：`1942220`；`NRestarts=0`
- 健康：`ok=true`，`evolutionEnabled=false`
- 活动队列：切换前、停服后、切换后均为 0
- migration：切换前后均为 55 条且快照逐字一致
- 数据库备份：39,620,604 字节，SHA-256 `78204333f1474555c7821604c1af335bf5749f38f73f2c6af03a185eb359c8c3`
- 切换证据目录：`switch-sr129130-20260726T204856Z`
- 冻结清单 SHA-256：`2d3377e78f7d1cbfbb9dbe739cbfc09db26991f6b3bd3f7862966ef524fe4ea7`

自动回滚未触发，旧二进制、旧前端、旧源码与数据库备份均保留。

## 部署后正式路径复验

直接使用 `/opt/wechatagent/target/release/wechatagent` 与 `/opt/wechatagent/frontend/dist`，从本次冻结数据库备份恢复到全新随机库后清空 worker 队列与账号枚举行，再以仅回环 transient unit 启动：双健康通过，69 项静态资源逐字一致，随机目标库运行前后全库哈希不变；unit 停止、临时库删除，常驻 PID、重启数、二进制与健康响应保持不变。

证据目录为 `audit/sr129130-deployed-smoke-20260727T051500Z`，20 项清单全部自校验，SHA-256 `a4c52d38d0b9593d44d3eaac843cbaac25d1a44f5402c977d50402f6f5d564ce`。

## 保留边界

- 本批没有执行真实模型内容质量任务；SR-129 是确定性 DTO 配置传递，SR-130 的关键动态门是副本集 OCC 与零写语义，不能用模型波动替代。
- SR-132 的队列枚举/filter、SR-141 的 WS lagged 重拉、SR-173 的部署后真实浏览器/代理断流仍开放，HC-027 不能标记完成。
