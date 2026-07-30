# Wave1 生产发布与部署后复验（2026-07-28）

## 发布范围与可回滚切换

本批只结算 Wave1 中已冻结的 SR-097、SR-132、SR-138、SR-139、SR-141 部署边界，不自动关闭它们所属的 HC-018、HC-019、HC-027，也不改写其它 SR 的既有结论。候选由已部署 release 基线叠加已审阅的后端、前端与迁移文件构建；正式后端 SHA-256 从 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 切换为 `5df573cf5aef14e5919e13157c5213d58e24219cc424098610fc5ee7f29a558b`，69 项前端静态文件逐项与候选一致。

切换前冻结旧后端、旧前端、候选后端、候选前端以及 `lessons_learned`、`operation_knowledge_chunks` 的 `mongodump`。发布脚本在任一门失败时会恢复旧二进制和旧前端、删除本批 m055 marker/索引并重启旧服务；本次脚本以 `WAVE1_RELEASE_OK=1` 正常提交，自动回滚未触发。回滚材料保留于服务器 `/opt/wechatagent/releases/wave1-20260728t020000z/{old,new,db-backup}`。

## 数据与运行态

- 正式 m055 `2026_07_055_lesson_promotion_identity` 恰好一条；`uniq_lessons_learned_ws_lesson` 与 `uniq_kchunks_lesson_promotion_source` 均为 unique，后者只对 `provenance.source=lesson_promotion` 且 source id 为字符串的行生效。
- 切换后正式 PID `2086752`、`NRestarts=0`、状态 `active/running`；磁盘与运行中二进制哈希均为 `5df573cf…a558b`。内外网健康均为 HTTP 200、`ok=true`、`evolutionEnabled=false`。
- 最终复验再次逐字校验 69 项静态资源；没有活动 Wave1 transient unit。所有 `wave1-postdeploy-*` workspace 行、临时管理员/会话、随机 Prompt 和 `wechatagent_wave1_*_probe_*` 数据库均为 0。

## 已通过的正式 Router/Mongo/WebSocket 业务门

部署后二次探针使用正式 `wa_session` Cookie middleware、正式 `api_router`、正式 Mongo 副本集与正式二进制；所有测试对象使用随机 workspace/身份并在结束时精确清理。四条必需检查均通过：

1. 两个管理员会话分别绑定两个随机 workspace，`/api/auth/me` 返回各自 scope。
2. Prompt pack reset 的缺 body、错误确认短语、未知字段三类请求全部拒绝，Prompt、Soul、Playbook、Domain Config 的治理快照前后逐字一致。
3. Chunk create、patch、lock、unlock 均产生预期原始 WebSocket 事件；review queue 的 pricing 维度只返回 pricing 行、不混 capability 行，未知维度 400，外 workspace 队列为空。
4. 同一 Lesson 的两个并发晋升请求与后续重放收敛到 Lesson `_id` 对应的同一个 Chunk；持久事实恰好一条 promotion Chunk、一条审计事件，Lesson 终态为 promoted，外 workspace 列表不可见。

业务探针清理了其创建的 34 条 Prompt、3 条 Domain Config、3 条 Chunk、1 条 Playbook、4 条 Soul、1 条 Lesson、3 条 revision、1 条事件及两组临时身份；最终基线恢复且零残留。该结果关闭 SR-097、SR-132 的正式 Router/Mongo 部署门，并证明 SR-138 的拒绝路径已在正式服务 fail-closed。SR-141 的运行制品与原始 WebSocket事件链已复验，但真实浏览器/代理 `lagged` 注入仍是独立未完成边界。

## 真实模型边界

本批没有获得成功的部署后模型判定，因此不把 SR-139 的真实模型门记为通过。已串行触达并冻结以下事实，期间从未切换正式 active provider：

- 正式 active `kr/claude-opus-4.7`：约 1.5 秒返回 Cloudflare HTTP 530 / error 1016；Prompt 编辑安全降级为人工确认。
- 隔离 Router 的 NVIDIA `deepseek-ai/deepseek-v4-flash`：单次请求 120 秒未返回，客户端超时。
- 隔离 Router 的 `claude-opus-4.8`：调用失败，未产生成功审查日志。
- 隔离 Router 的 DashScope `qwen3.7-max-2026-05-17`：86 ms 返回 `Arrearage`（账号欠费）HTTP 400。

四次探针均先停隔离 Router、删除精确随机库并复核正式 PID/哈希；DashScope 在连通性门失败后没有创建临时 Prompt。结论是“真实路径已触达且失败时安全降级，成功响应受外部 provider 状态阻塞”，不是“模型质量通过”或“代码失败”。恢复外部模型可用性后仍需补一条成功的完整 `BEFORE/AFTER` 语义审查证据。

## 冻结证据

服务器证据目录为 `/opt/wechatagent/releases/wave1-20260728t020000z/postdeploy`。最终 `SHA256SUMS.final` 共 11 项，`sha256sum -c` 为 11/11，清单 SHA-256 为 `d16d0ce7f529c279fee16374107eeba990921b6f867135a74a5370a8f62b677a`。权威裁定文件 `final-adjudication.json` 明确记录 `realModel.verifiedSuccess=false`、四条 Router检查通过、零残留、正式运行态与 provider阻塞；发布日志终态为 `WAVE1_RELEASE_OK=1`。
