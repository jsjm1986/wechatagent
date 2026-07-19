# 两轮业务复审逐项账本

本账本覆盖 `findings.md` 的 SR-001～SR-183。

## 判定规则

- 第一轮只回答真实业务：`直接`=正常生产流程可达；`条件`=实现缺陷成立但依赖部署/并发/开关；`治理`=不会直接改变客户业务事实，但会使上线、审计或测试结论失真；`契约`=代码与产品意图冲突，必须由人决定；`合并`=问题成立，但与同一根因共用一个决策项。
- 第二轮只接受最小闭环：优先补作用域、CAS、状态校验、字段投影或失败语义；除非现有数据模型无法恢复，默认不引入新服务、消息总线、分布式事务或平行状态机。
- `外部事实` 表示冻结仓库无法证明的部署事实，不能擅自按“已启用”或“未启用”裁决。
- 每个 SR 恰好归属一个 HC；合并只合并人类取舍，不删除原证据。

## 当前工作树实施证据（不改写冻结审查结论）

### HC-009 实施证据

- 覆盖 SR-022、SR-023、SR-031、SR-042、SR-051、SR-175；六条仍各自保留原始业务结论和最小方案，仅共享同一发送前 fail-closed 决策面。
- 状态：`production-wired / deterministic-verified / real-model-blocked`（2026-07-18）。生产入口已接线；严格证据 5/5、Knowledge Agent PBT 17/17、Review 97/97、Escalation 96/96 与 `cargo check --lib` 通过。
- 核心证据：实时 Reviewer 必填且范围校验；独立语义 Claim Gate 失败即 hold；安全类 revision 不恢复危险原稿；Knowledge 引用验证 opened/verified/relationRole/quote/anchor；holding reply 经独立语义审查、全场景数字授权校验并入 Outbox；目录声明由 AI 语义抽取后由服务端逐字段核验，任一遗漏、串价、币种/SKU 错误或额外非目录事实均不能取得目录背书。
- 未完成验证：授权真实模型端点 `127.0.0.1:9090` 在 2026-07-18 21:06 +08:00 无监听，模型列表请求被拒绝，故真实模型恶意任务集未执行。该环境阻塞不得被解释为通过；服务恢复后补测并更新 HC-009 才能改为 fully-verified。

### HC-010 / HC-032 发送子集实施证据

- 覆盖边界：本批只结算 SR-052（名片不确定送达不得自动重放）与 SR-066（in-flight 取消的最后可取消点和真实送达语义），并为 SR-004/HC-032 增加一个独立严格门。SR-028、SR-034、SR-050、SR-126、SR-128、SR-172、SR-174、SR-176、SR-177、SR-178 均未因本批自动结算。
- 状态：`production-wired / deterministic-compiled / docker-run-pending / real-model-run-pending`（2026-07-18）。Outbox claim 持久化不可复用 `claim_token` 与 `claim_generation`；MCP 前以 CAS 写 `send_started_at` 并拒绝已登记取消的 claim；跨过远端边界后，成功回执收敛为 `sent`，无法核验则进入 `delivery_unknown` 且禁止自动重放；名片无权威查询时，崩溃回收直接进入 `delivery_unknown`。
- 确定性证据设计：`tests/outbox_integration.rs` 的四条 `delivery_redline_*` 真实经过 `enqueue -> atomic_claim_pending -> process_entry/reclaim_expired_leases`。其中两条异步 HTTP barrier 在 MCP 已完整收到 `message_send_text/message_send_namecard`、尚未回包时分别注入取消和 worker 崩溃，硬断物理发送调用恰好一次、终态为 `sent` 或 `delivery_unknown`、不可再次 claim；另两条覆盖发送前取消零调用与 HTTP 歧义结果不重放。`cargo check --test outbox_integration` 与 `cargo test --test outbox_integration --no-run` 已通过；本机无 Docker，四条运行证据仍缺失。
- 真实业务门：`.github/workflows/ci.yml` 新增手动 `smoke_t4` 严格 job，无 `continue-on-error`。它先机械断言 `delivery_redline_*` 恰好四条并全部运行，再单独运行真实模型 T4；T4 要求两轮非空、承接上下文的单问题回复，Reviewer 分数过运行阈值且无残留风险，明确停止被 Reaction 高置信识别，第二轮待发在 MCP 前取消，并把 run/review/reaction/outbox/MCP JSON-RPC 完整轨迹先写 `t4_business_transcript.json` 再执行质量断言。
- 未完成验证：当前改动尚未提交到 GitHub 可见分支，故严格 Actions job、Docker barrier 与真实模型 T4 均未执行；不得把 YAML 存在、测试可列出或本地编译成功解释为运行通过。完成条件是授权创建测试分支并推送后，`smoke_t4` job 真绿且 artifact 经人工复核。

### HC-004 / HC-010 Task 与 Outcome 后续实施证据

- 覆盖边界：本批只结算 SR-034（Task 所有权/取消 fencing）与 SR-036（Outcome task 跨租户去重）；不自动结算 HC-004、HC-010 下的 SR-028、SR-050、SR-177 等其它问题，也不把隔离测试解释为已部署。
- 状态：`working-tree-wired / deterministic-verified / isolated-real-mongo-verified / deployment-pending`（2026-07-19）。Task claim 原子写入不可复用 token 与单调 generation，终态、Outbox 绑定/授权、prepared commit 与 stale reclaim 均以当前 owner CAS 收敛；管理员取消在同一 Task 文档上竞争，旧 worker 不能取得发送授权。Memory consolidation 以 contact-scoped single-flight key、`rerun_requested` 交接和可重放 commit marker 收敛候选到达/终结/OCC 竞争。
- SR-036：Outcome partial unique key 已改为 `(workspace_id, kind, account_id, content)`；m017 历史清理按 `(workspace_id, account_id, content)` 分组，避免升级时误删另一 workspace 的合法任务；m033 只在旧索引存在时删除旧规格，统一索引入口再创建租户作用域新索引。
- 本地确定性验证：索引契约 5/5、Memory OCC/rerun fencing 11/11、Task claim/owner 10/10、m017 workspace 分组 1/1、迁移顺序/唯一性 2/2、`cargo check --lib`、`cargo fmt --check` 与 `git diff --check` 均通过。`tests/sr034_task_send_fencing.rs` 与 `tests/outcome_task_workspace_dedupe.rs` 均已在 Windows 以 `--jobs 1 --no-run` 成功编译。
- 真实 Mongo 验证：在授权测试服务器的独立 `/tmp` 源码与 target 中，以 `TEST_MONGODB_URI=mongodb://127.0.0.1:27017` 和随机 `wechatagent_test_*` 库单线程运行 SR-034 四条生产 helper 红线，4/4 通过：building decision 零远端发送、stale claim 取消且零远端发送、同一 owner 恰好一次发送、pending/in-flight 取消均在远端边界前终止。SR-036 另以完整迁移和索引初始化后的随机库运行 1/1 通过：不同 workspace 的相同 account/content 可各保留一行，同 workspace 重复写命中 Mongo duplicate-key，最终恰有两行。两批测试前后该前缀数据库清单均为空；临时源码、归档与独立 target 均已删除。
- 服务器非部署证明：运行服务 PID、release 二进制 SHA-256 在测试前后完全一致，`/api/health` 返回 200；本批没有修改 `/opt/wechatagent`、重启服务或连接业务数据库。正式测试服务器部署与部署后回归仍待后续上线批次完成。

### HC-010 Reaction fencing 后续实施证据

- 覆盖边界：本批只结算 SR-028，不自动结算 SR-050、SR-172、SR-177 或 HC-010 其它发送问题。
- 状态：`working-tree-wired / deterministic-compiled / isolated-real-mongo-verified / deployment-pending`（2026-07-19）。Reaction claim 使用不可复用 token 与单调 generation；并发入口不能偷取活动 claim，超时重领后旧 owner 的结果 CAS 失败，轨迹、负例与 Outbox 取消等副作用只在当前 owner 成功提交后执行。
- 验证：`tests/reaction_claim_lock.rs` 在 Windows 以 `--jobs 1 --no-run` 编译通过；授权测试服务器以独立 `/tmp` 源码/target 和随机 `wechatagent_test_*` 库运行 2/2 通过，覆盖活动 claim 仅一次 LLM 调用，以及 ABA 重领后旧 stop 结果不能覆盖新结果、追加轨迹或取消 pending Outbox。测试库零残留，临时源码/归档/target 已删除；运行服务 PID 与 release SHA-256 前后不变，`/api/health` 为 200，未部署工作树版本。

### HC-010 发送台账完整业务键后续实施证据

- 覆盖边界：本批只结算 SR-050，不自动结算 SR-052、SR-066、SR-172、SR-177 或 HC-010 其它发送问题。
- 状态：`working-tree-wired / deterministic-verified / isolated-real-mongo-verified / deployment-pending`（2026-07-19）。新台账行保存不可变 `outbox_id`，以 `$setOnInsert` upsert；partial unique 索引只约束带 ObjectId 锚的新行，历史无锚行保持可读。近期发送、响应回扫、阶段回扫、只读 API、管理工具和前端统计均固定 `(workspace_id, account_id)`，缺少 `accountId` 的 API 请求 fail-closed。
- 本地验证：台账/索引/路由/模型契约 18/18、发送历史前端测试 2/2、前端生产构建、`cargo fmt --check` 与 `git diff --check` 均通过。Windows 已生成并成功列出 `send_ledger_integration` 测试二进制；随后 Cargo 为应用主二进制重复链接时因本机 E: 盘仅余约 9 MB 报 `no space on device`，该环境失败不解释为代码失败。
- 真实 Mongo 验证：授权测试服务器在独立 `/tmp` 源码/target 中离线单作业构建成功（RC 0），以 `TEST_MONGODB_URI=mongodb://127.0.0.1:27017` 和随机 `wechatagent_test_*` 库运行三条 `sr050_*` 生产 helper 红线，3/3 通过：同一 Outbox 重放仍恰有一条台账且不能改归另一账号；同 workspace、同 wxid 的近期发送按账号隔离；另一账号的入站回复和阶段推进不能归因到当前账号台账。测试库、源码、归档、target 与启动日志均零残留；运行服务 PID `727743`、release SHA-256 `ad45f055063def7c5f7adbfdf64ab68a53cdbb949c9d6aadc918fd65f5434615` 前后不变，`/api/health` 为 200，未部署工作树版本。

### HC-010 Outbox 运营对象投影后续实施证据

- 覆盖边界：本批只结算 SR-172，不自动结算 SR-052、SR-066、SR-159、SR-177 或 HC-010 其它发送问题。
- 状态：`working-tree-wired / deterministic-verified / local-real-router-mongo-verified / deployment-pending`（2026-07-19）。管理列表投影互斥 typed payload（`text | media | referralCard`），保留不可变素材/名片 id，并按页最多两次批量回表补可读名称；目标不存在或 id 非 ObjectId 时保留 id 回退。双目标异常行显式标 `invalid`，不静默选一类。元数据查询固定 workspace，账号私有目标只有与 Outbox `account_id` 一致时才下发名称，workspace 共享目标仍可解析。
- 前端与取消语义：发件箱按类型显示正文、素材名/文件名或顾问名/目标 wxid及不可变 id；同联系人多行不再混淆。取消前确认框展示业务号、客户、精确发送对象和恢复风险；pending 才提示可停止，in-flight 只提交取消请求，已跨发送边界或曾回收的条目明确可能已经送达且取消不能撤回。`delivery_unknown` 与 `cancel_requested` 继续使用既有状态机，不为显示层另建状态。
- 验证：后端纯函数覆盖 text/media/referralCard/invalid、缺失元数据 id 回退和跨账号私有名称隔离；共享后端 fixture 与前端 canonical key/typed payload 对账。前端组件与契约测试 9/9、生产 `tsc && vite build`、`cargo fmt --check` 与定向 `git diff --check` 均通过。新增 `tests/sr172_outbox_projection.rs` 经真实 `wa_session` Cookie 调生产 `api_router`，在本机 `127.0.0.1:27017` 随机 `wechatagent_test_*` 库运行 1/1 通过（36.18 秒）：同一客户的文本、同账号素材、名片均返回正确 typed payload和 id，恢复计数保留；账号 A 引用账号 B 私有素材时只保留 id、不泄漏名称，`accountId` 过滤排除账号 B 行。测试前后随机库清单均为空。
- 未完成边界：本批未部署工作树版本，也未修改或重启测试服务器服务；远端上传因当前进程无法恢复 `DEPLOY_PASS` 而未发生。本地真实 Router + 随机 Mongo 证据不等同于部署后回归，正式部署仍待后续上线批次。

### HC-027 SSE 连续失败预算后续实施证据

- 覆盖边界：本批只结算 SR-173，不自动结算 SR-129、SR-130、SR-132、SR-141 或 HC-027 其它 Knowledge Cockpit 问题。
- 状态：`working-tree-wired / deterministic-verified / deployment-pending`（2026-07-19）。共享 SSE 重连器把原生 `open` 与业务事件都视为连接恢复，只对连续未成功建连计数；已替换 EventSource 的迟到事件被隔离，terminal `close` 仍立即停止且不重连。
- 任务事实收敛：TaskRail 消费 reconnect/gave-up 状态；每个 `turn` 都回读 `/api/knowledge/chat/tasks/:id` 更新权威进度，任务 id 与 snapshot generation 阻止迟到响应覆盖新任务。gave-up 或浏览器无 EventSource 时立即进入 5 秒一次、最多 12 次的有限轮询；completed/failed/cancelled、切任务或卸载都会停止 timer，耗尽后明确提示手工“拉取”。未更换 HTTP+SSE 协议，也未建立永久轮询。
- 验证：共享重连器与 TaskRail 专项 13/13 通过，覆盖多轮 `open -> error` 不耗尽预算、连续未 open 才 gave-up、旧连接迟到事件隔离、terminal close 零重连、turn 回读主卡、gave-up 轮询收敛终态、无 EventSource 时严格止于 12 次并停止请求；Knowledge 相关回归 18/18、前端生产 `tsc && vite build` 与定向 `git diff --check` 均通过。
- 未完成边界：本批没有后端或数据库迁移，也未部署工作树版本；前端部署后浏览器/代理真实断流回归仍待统一上线批次。

### HC-032 进程级缓存数据库隔离后续实施证据

- 覆盖边界：本批只结算 SR-174，不自动结算 SR-004、SR-126、SR-128、SR-176、SR-178，也不把本地真实 Mongo 结果解释为已部署或整个 HC-032 已关闭。
- 状态：`working-tree-wired / deterministic-verified / local-real-mongo-verified / deployment-pending`（2026-07-19）。每个 `Database::connect` 获得进程内单调 identity；同一 `Database` 的 clone 共享 identity 与生命周期 token，独立连接即使使用相同 workspace id 也获得独立 Taxonomy/DomainProfile cache。缓存 registry 与 taxonomy 初始化 guard 均以数据库 identity 分片，并用 `Weak` 生命周期清理已释放测试数据库的进程内状态。
- 生产接线：所有 Taxonomy 读取、DomainProfile 读取及后台写后失效入口都显式传入当前 `Database`；失效只作用于该连接对应的缓存。保留现有 30 秒 TTL、HTTP/worker 调用关系与纯查表 API，不引入新缓存服务，也不依赖串行测试规避问题。
- 验证：registry 单测 1/1 证明同 identity 共享、不同 identity 隔离且失效不跨库；`tests/sr174_cache_database_isolation.rs` 使用两个随机 Mongo 数据库和相同 workspace，分别写入独有 profile/taxonomy，在 A、B 均预热后按 A→B、B→A 交错读取，生产 `load_active_domain_profile` 与 `normalize_target_stages` 始终只见本库数据；生命周期 guard 修正后的最终强化红线 1/1 通过（55.76 秒），测试前后 `wechatagent_test_*` 数据库清单为空。`cargo check --lib`、`cargo fmt --check` 与定向 `git diff --check` 均通过。
- 未完成边界：当前仍是未部署工作树版本；SR-176 已在后续独立批次补真实入口与专用 hard gate，SR-004 的其余 CI 分层、SR-126 的 Knowledge 真实入口闭环及 SR-128/178 的动态任务正向见证仍须分别修复和验证。

### HC-032 租户隔离真实入口证据后续实施证据

- 覆盖边界：本批只推进 SR-176，不自动结算 SR-004、SR-126、SR-128、SR-178，也不把三类代表性入口外推为全仓所有 CRUD 均已逐端点验证。
- 状态：`working-tree-wired / deterministic-verified / local-real-router-mongo-verified / hard-gate-wired / actions-run-pending / deployment-pending`（2026-07-19）。新增 `tests/sr176_real_route_isolation.rs` 以真实 `wa_session` Cookie 穿过 session middleware 和生产 `api_router`，不提高 Handler 可见性、不复制生产授权 helper。
- 真实入口矩阵：读取面以 workspace A 会话访问 workspace B Contact 得 404、本租户 Contact 得 200 且返回正确 wxid；写入面以 workspace A 会话更新 workspace B 独有 Product 得 404，目标名称保持不变，同时本租户同名 Product 更新成功且 workspace B 同名 Product 不变；认证面直接插入真实过期 `AdminSession`，`lookup_session` 返回 `SessionExpired`，同一 Cookie 请求 `/api/auth/me` 得 401，而有效 Cookie 得 200。
- 证据治理：`workspace_isolation.rs` 与 `products_workspace_isolation.rs` 的名称和注释已明确降级为 collection/filter/index 局部约束，不再声称覆盖未调用的 Handler；旧 auth 用例已改为真实过期记录并在断言前清理随机数据库。保留这些纯 DB 测试，不为每个 CRUD 复制完整矩阵。
- 验证：SR-176 真实 Router 红线 1/1 通过（23.22 秒）；修正后的真实过期 session 用例 1/1 通过（31.01 秒）；两条 CI 列表机械计数均恰好为 1；专项 `cargo check`、`cargo fmt --check` 与定向 `git diff --check` 通过；最终 `wechatagent_test_*` 清单为空。
- 合并门：`.github/workflows/ci.yml` 新增 `tenant-isolation-security` PR/push hard gate，无 `continue-on-error`，先机械要求两条目标测试各恰好存在一次，再只运行这两条 ignored 红线；全量 integration 继续作为 soft 诊断线。当前工作树尚未推送，故 GitHub Actions YAML/Ubuntu/testcontainers 真实运行仍待证；本机缺少 `actionlint` 与可用 YAML parser，已完成 job 边界、测试计数、命令列表和差异静态检查，但不得记为远程绿色。
- 未完成边界：当前未部署，也未修改或重启测试服务器服务；正式结算仍需授权分支上的 hard gate 真绿及统一部署后回归。HC-032 的 SR-004、SR-126、SR-128、SR-178 继续开放。

### HC-032 Knowledge 真实证据后续实施证据

- 覆盖边界：本批只推进 SR-126，不自动结算 SR-004、SR-128、SR-178，也不把 catalog/关系纯约束测试外推为维护 Agent 闭环。
- 状态：`working-tree-wired / deterministic-verified / hard-gate-wired / local-real-mongo-blocked / actions-run-pending / deployment-pending`（2026-07-19）。离线召回评测不再把金标 chunk id 注入 mock；它写入相关条目和高静态分无关干扰项，以生产 `list_catalog` 的首项驱动 open/cite，金标只在返回后计算 recall@1。维护闭环真实调用 `chat_turn -> chat_apply -> verify_operation_knowledge_chunk -> knowledge_agent::answer`，并断言 draft/needs_review 中间态、verify revision、verified 后 grounded citation。Worker 以封闭 `committed|noop|needs_manual|failed` verdict 持久化单步业务结果，failed 保存原因，只有 committed 的真实产物进入待审池。
- 验证：三个硬门测试二进制联合 `cargo check` 通过；`knowledge_task_worker` 确定性契约 6/6 通过；定向格式与差异检查通过。真实召回测试二进制成功生成并启动，但本机没有 Docker CLI/socket，且 `TEST_MONGODB_URI`/`MONGODB_URI` 均未配置，故在 testcontainers 创建 Mongo 容器时以基础设施错误退出，业务断言没有执行；该结果不得记为通过或失败。
- 合并门：`.github/workflows/ci.yml` 新增无 `continue-on-error` 的 `knowledge-evidence-gate`，机械要求召回、真实 Chat 闭环和三条 Worker outcome 测试各恰好存在一次，再按 exact name 运行五条 ignored 红线。全量 integration 继续作为 soft 诊断线。当前工作树尚未推送，故 Ubuntu/Docker Actions 真绿仍待证。
- 未完成边界：当前未部署，也未修改或重启测试服务器服务。SR-126 的代码与门禁修复已完成，但只有在专用 hard gate 真绿并完成部署后回归后才能改为 fully verified；HC-032 仍因 SR-004、SR-128、SR-178 及各批次远程证据缺失而保持开放。

### HC-032 动态能力正向见证与 CI 分层后续实施证据

- 覆盖边界：本批推进 SR-004、SR-128、SR-178；不自动结算 SR-126/174/176 的远程证据，也不把本地类型检查、构造的 checker fixture 或 YAML 静态解析解释成真实模型、GitHub Actions 或部署后绿色。
- 状态：SR-004 为 `working-tree-policy-wired / deterministic-verified / actions-run-pending`；SR-128 与 SR-178 均为 `working-tree-wired / deterministic-verified / hard-gate-wired / real-model-run-pending / deployment-pending`（2026-07-19）。共享 `CapabilityEvidence` 从测试入口记录 `attempted/llm_calls/branch/artifacts/assertions_run/verdict/skipped_reason`，Drop 在普通早退时保留 `inconclusive`、瞬时端点错误显式写 `infra_skip`、panic 写 `failed`；只有真实调用、非空目标产物和实际执行的断言同时存在才可提交 `pass`。
- SR-128 正向见证：Knowledge K2/K3/K6/K7/K10、Quality Q3/Q4、Smoke T3 与 Recall 跨行业/维护/缺口闭环共 11 个 case进入固定 manifest。K2 必须真实走关系跳转；视觉/提案/自动审定必须反查产物；Recall 必须满足最低 case/round 覆盖并以真实维护写链闭环。零调用、零产物、零断言、early return 或重复 outcome 均不能通过。
- SR-178 正向见证：Cross-domain 完整双域弧/同输入差异/身份探针、Principal Channel/Relay、Planner/Wake 主动触达、Dynamic Adversarial、Digital Twin peer/formal 与 Roleplay Arc 共 11 个 case进入同一 manifest。全 fallback、零回复、零 task、零 escalation 且无明确安全 fail-closed、空 relay、端点 transient 或身份生成失败均留下非 pass；Principal Channel 只接受真实 pending请示或明确安全/政策/必填字段阻断，预算耗尽不冒充合规终态；Relay 不再把零调用抬成 1。
- CI 与防陈旧：nightly `real-llm-redline` 扩为 7 个串行 shard（含 Roleplay Arc），每个 shard 在 rust-cache 后先清空 ledger，只上传本 shard 本 run/attempt 的结果。`skip-gate` 下载所有 artifact 后由 `check-capability-outcomes.py` 固定核对 22 个 case，并校验 schema、case id、SHA、GitHub run id/attempt、正调用/产物/断言与唯一性；旧缓存和旧 run不能冒充当前证据。
- SR-004 分层策略：新增 `check-ci-gate-policy.py` 并由 baseline hard gate执行。它固定 baseline、Knowledge evidence、Tenant isolation、Frontend contract、T4、redline 与 skip-gate 共 7 个关键门不得 `continue-on-error`；全量 integration及 smoke/recall/ops/quality/adversarial 六类长尾或波动套件必须保持 soft，其中五类真模型套件必须仅 nightly。检查器同时锁定 redline→skip-gate 依赖和 typed outcome汇总器接线，防止后续 YAML 漂移；没有把全部慢测升级成 PR 阻断门。
- 本地验证：七个 redline 测试 crate联合 `cargo check` 通过；22-case manifest/11-case witness映射通过；checker 的完整通过、缺失拒绝和陈旧 run拒绝三态通过；`rustfmt --check`、定向 `git diff --check`、PyYAML解析、CI redline结构和 gate policy `hard=7 soft=6 failures=0` 均通过。唯一编译告警是既有 `handle_principal_decision_relay` dead_code，不属于本批。
- 未完成边界：本批没有调用外部真模型、没有推送分支、没有运行 GitHub Actions，也没有修改或重启测试服务器。SR-004/128/178 只有在授权分支的 hard/nightly 门真实运行并复核 artifacts，且统一部署后回归完成后，才能改为 fully verified；HC-032 因这些远程证据及 SR-126/176 等既有 deployment/actions pending 继续开放。

## SR-001～SR-061

| SR | 第一轮：真实业务结论 | 第二轮：最小化/不过度工程结论 | 决策项 |
|---|---|---|---|
| SR-001 | 条件但紧急：凭证形态值已进入 Git；是否仍有效需外部核实，历史暴露已成立。 | 先轮换和撤销，再清历史与改 secret 注入；不先建设新密钥平台。 | HC-001 |
| SR-002 | 条件但紧急：部署凭证进入脚本并复制到服务器，暴露面比测试 key 更大。 | 与 SR-001 共用轮换、历史清理和部署 secret；不单建第二套机制。 | HC-001 |
| SR-003 | 契约：Evolution 默认值文档与代码相反；冻结代码和 `.env.example` 实际默认开启。 | 只选定一个默认并同步 README/配置注释/测试，不重构 Evolution。 | HC-002 |
| SR-004 | 治理：完整 Docker 测试失败不阻断合并，不能证明具体业务已坏，但上线证据不足。 | 仅把关键测试拆成 hard gate；保留长尾 soft job，避免把全量慢测都变阻断。 | HC-032 |
| SR-005 | 条件但紧急：普通 workflow input 承载 key；是否已泄漏需审计 Actions 历史。 | 改 GitHub Secret 并审计/轮换；不新增自建 secret service。 | HC-001 |
| SR-006 | 治理：README/架构落后会误导部署，但不是运行时故障。 | 修正当前拓扑和默认值，标生成日期；无需把文档自动生成系统化。 | HC-033 |
| SR-007 | 直接迁移风险：升级旧库可把高版本 draft 置 current。 | 修正一次迁移选择规则并提供存量审计/补偿迁移，不改整个 Prompt 模型。 | HC-003 |
| SR-008 | 条件高影响：ops 三表允许零/多 current；并发或中间失败可触发。 | 先存量检查+partial unique/CAS；只有现有 Mongo 拓扑已支持时才用事务。 | HC-003 |
| SR-009 | 直接历史数据风险：m029 用裸 wxid 跨租户回填。 | 以 `(workspace,account,wxid)` 重跑校正；无需通用身份解析子系统。 | HC-003 |
| SR-010 | 直接运维风险：被跳过迁移仍记完成，账本与数据真相分离。 | 增加 `blocked/pending` marker 和后置校验；不引入独立迁移编排服务。 | HC-003 |
| SR-011 | 条件 P0：只要存在非默认 workspace，发送凭证和 post-hoc 去重即可串租户。 | 所有 MCP API 补必填 workspace，并去掉隐式 fallback；不重写 MCP client。 | HC-004 |
| SR-012 | 条件 P0：跨 workspace 复用 account/wxid 时 debounce reload 可取错联系人。 | runner 保留入口 workspace 并用三元 filter；无需立刻上 durable queue（崩溃恢复另见 SR-177）。 | HC-004 |
| SR-013 | 条件高影响：多 workspace 时单例 LLM Registry 会让一租户切换全进程 provider。 | 若产品只支持单租户则显式拒绝多租户配置；否则 registry 按 workspace map，二选一，不做同时兼容两种隐式模式。 | HC-004 |
| SR-014 | 直接安全窗口：ACL 收缩后 cookie/JWT 快照仍有效。 | 敏感请求统一重读 ACL；JWT 用短 TTL/权限版本即可，不先建设完整 token revocation 平台。 | HC-005 |
| SR-015 | 条件高影响：重复 appId 时 webhook 路由和验签不确定。 | 先查重再建 partial unique；无需额外路由目录服务。 | HC-005 |
| SR-016 | 条件外部事实：应用无登录限流；边缘 WAF 是否补偿未知。 | 在登录/token 共用轻量限流+审计，并保留边缘限流；不做复杂风控画像。 | HC-005 |
| SR-017 | 条件运维风险：本地文件与 Mongo 非原子，多副本还需共享存储。 | 首选临时文件+失败补偿+定期 GC；只有确认多副本才迁对象存储/共享卷。 | HC-006 |
| SR-018 | 条件审计泄露：非默认 workspace 的 LLM 元数据记到默认租户。 | LLM 日志入口补 workspace；与 HC-004 同一租户上下文决定。 | HC-004 |
| SR-019 | 直接可观测缺口：首个 LLM 前失败的 run 消失，事故与漏斗不完整。 | 在入口先写 started、统一 terminal finally；不先做全链 event sourcing。 | HC-007 |
| SR-020 | 条件一致性：provider 热切换后精确缓存可返回旧产物。 | cache key 加 workspace/provider generation，激活时 bump；不引入分布式缓存。 | HC-008 |
| SR-021 | 条件高影响：非默认 workspace 的 Reply/Review/Reaction/Memory 读取默认租户 Prompt/Soul。 | loader 必填 workspace，显式配置才继承全局模板；与 HC-004 合并。 | HC-004 |
| SR-022 | 直接安全风险：产品声明门只信 Reviewer 自报，漏报即不执行。 | 先严格校验 claimAnalysis 并对产品效果类加独立判定/强制复审；不做通用 NLP 规则引擎。 | HC-009 |
| SR-023 | 直接 P0：revision 失败会恢复已知高压/隐私风险原稿并入 Outbox。 | 安全类失败 hold，只有纯风格类允许回退；复用现有分类和 finalize，不增新 Agent。 | HC-009 |
| SR-024 | 条件租户泄露：公共事件写入默认 workspace。 | 写事件 API 强制 workspace 参数并迁调用点；不重做事件系统。 | HC-004 |
| SR-025 | 条件低于消息串租户：同 account_id 的计数和 pacing 跨 workspace 合并。 | 查询/索引加 workspace；日 cap 仍保持告警，不顺带升级硬门。 | HC-004 |
| SR-026 | 条件消息丢失：全局幂等键未含 workspace。 | key 和唯一索引纳入 workspace/account，并做兼容迁移；不更换 Outbox。 | HC-004 |
| SR-027 | 条件客户伤害：非默认租户 stop 不取消本租户 Outbox，可能反而取消默认租户。 | cancel API 接收完整 tenant scope；与 HC-004 合并。 | HC-004 |
| SR-028 | 条件并发：多副本/超时重领时旧 Reaction 可覆盖新结果并重复副作用。 | claim 增 owner/generation，提交 CAS；不因单进程现状搭建队列。 | HC-010 |
| SR-029 | 条件恢复风险：Memory 主卡有 OCC，但附属写和候选消费可半提交。 | 先持久化 consolidation id/phase 并让副作用幂等；事务仅在现有 Mongo 支持时采用。 | HC-011 |
| SR-030 | 条件但真实的数据边界：Knowledge 正文下钻只校验 workspace，账号私有知识可进模型上下文。 | 统一传现有 visibility scope 并逐跳校验；不复制第二套 Knowledge Agent。 | HC-012 |
| SR-031 | 条件安全风险：模型偏离 prompt 时 contradiction/quote 仍可被当证据。 | 服务端对 opened metadata、quote/anchor 做确定性校验；不做另一个“证据 Agent”。 | HC-009 |
| SR-032 | 直接短时陈旧：正文编辑不改变 corpus signature，5 分钟缓存可返旧答案。 | 签名加入 chunk version/updated_at 或 revision generation；无需禁用全部缓存。 | HC-012 |
| SR-033 | 直接协议缺口：循环超时只在轮首检查，强制 final 不改返回 payload。 | 用单一 deadline 包住 LLM+工具并在返回前校验 final；不重写工具循环状态机。 | HC-012 |
| SR-034 | 条件高影响：Task lease/cancel 无 fencing，旧 worker 可继续入 Outbox。 | claim token+generation，handler 提交前复查；复用现有 task collection，不新建调度服务。 | HC-010 |
| SR-035 | 直接边界：过期 principal relay 裸发后不写终态，可被回收重发。 | 过期分支也走统一 task finalize/Outbox；不单建 relay worker。 | HC-013 |
| SR-036 | 条件统计缺失：outcome task unique key 缺 workspace。 | key 加 workspace 并迁移；与租户 scope 决策合并。 | HC-004 |
| SR-037 | 直接治理风险：领导解释可直接成为 workspace verified 知识。 | 所有沉淀先 draft+needs_review；保留领导授权用于单次转述，不等同知识核验。 | HC-013 |
| SR-038 | 直接可靠性：首次请示推卡失败仍留下去重 pending，后续不再推。 | 先 durable intent，再把 delivered/pending 分开；最小可用方案是失败时保持可重试状态。 | HC-013 |
| SR-039 | 条件重复通知：timeout scanner 多副本无 claim/CAS。 | scanner 对 escalation 做 claim token/CAS；不引入全局 scheduler。 | HC-013 |
| SR-040 | 条件错裁决：同 workspace 多账号共用 principal 时回复匹配漏 account。 | 匹配键加 account/short code；不复制每账号请示子系统。 | HC-013 |
| SR-041 | 条件策略错用：同 workspace 多 domain 时 scanner 无 domain 身份。 | escalation 持久化 domain/policy version；不在扫描时猜 current config。 | HC-013 |
| SR-042 | 条件客户风险：holding reply 裸发绕过数字护栏，正确性仅靠 prompt。 | 统一通过现有 deterministic relay/number guard；不增新审核模型。 | HC-009 |
| SR-043 | 条件高影响：DomainProfile current/active 多步写可零/多生效。 | 存量清理+唯一约束/CAS；与其它版本指针统一，但不重建配置平台。 | HC-014 |
| SR-044 | 条件配置注入：误配维度名可覆盖保留路径。 | 对 kind 做闭集/路径字符校验并保留 namespace；不取消动态维度能力。 | HC-014 |
| SR-045 | 直接指标失真：同一 taxonomy 候选被两路径累计两次。 | 只保留一个写入点或统一幂等 occurrence id；不合并整个 Decision/Gateway。 | HC-015 |
| SR-046 | 条件不确定映射：alias 无唯一归属。 | 写入时检测冲突并建可执行唯一约束；不设计复杂 alias 图。 | HC-015 |
| SR-047 | 直接信号失真：同轮重复维度被当多轮证据。 | 每 run/dimension 去重后再计数；不改贝叶斯模型。 | HC-015 |
| SR-048 | 直接语义违约：Shadow/Simulation 会写生产记忆/整改队列且不走最终硬门。 | 增明确 `run_mode=shadow` 并在共享写 helper fail-closed；复用决策链但禁业务副作用。 | HC-016 |
| SR-049 | 条件证据污染：Prompt shadow 源消息缺 tenant scope，且历史基线混入当前配置。 | 取源加完整 scope并冻结评估依赖版本；不构建第二套回放数据库。 | HC-017 |
| SR-050 | 条件发送归因失真：台账漏 account/唯一锚，可能串账号或重复计数。 | 台账 key 加 workspace/account/outbox id 并 upsert；不另建分析仓。 | HC-010 |
| SR-051 | 条件产品事实风险：只验证 product_id 存在，不校验回复价格/事实一致。 | 将正文结构化 product facts 与目录快照逐字段比对；不使用关键词大全。 | HC-009 |
| SR-052 | 条件重复发送：名片 timeout/reclaim 无 post-hoc 核对。 | 名片发送也保存可查询 remote/idempotency 证据；若 MCP 不支持查询，至少标 uncertain 并停自动重发。 | HC-010 |
| SR-053 | 直接版本风险：Soul 发布物删历史且可改已发布行。 | 改为 append-only 版本+active pointer/CAS；复用现有 collection，不上通用配置服务。 | HC-018 |
| SR-054 | 条件客户阻塞：resolved 与 relay task 两步写，崩溃可永久吞转述。 | 同事务或 durable relay intent；不新增消息队列。 | HC-013 |
| SR-055 | 直接配置分裂：Prompt status/current 两套指针且发布先删历史。 | 选一个生效指针、append-only 版本、原子切换；与 Soul/Playbook 共用最小模式。 | HC-018 |
| SR-056 | 直接/条件混合：DomainSchema 更新按最新行而非 active，切换可零/多 active。 | 唯一 active约束+expected version CAS；不做全量配置重构。 | HC-014 |
| SR-057 | 条件成交漏登：信号先 CAS approved，后续校验/落成交失败不可重试。 | 先校验全部参数，再在事务内 CAS+append；无事务则增加 `applying/failed_retryable` 状态。 | HC-019 |
| SR-058 | 直接审计问题：请求可自填 reviewedBy/固定代理 actor，不能证明操作者。 | actor 一律来自认证上下文，system actor用枚举；不建设独立 IAM。 | HC-019 |
| SR-059 | 条件画像分裂：关系建议 approve 写联系人和终态非原子且无 CAS。 | pending→applying token，再幂等写联系人并 finalize；可用事务时更简单。 | HC-019 |
| SR-060 | 直接产品盲点：关系终态永久占唯一槽，新证据不能再审。 | 唯一键加入 cycle/generation 或终态后允许新 pending；不保留无限历史在热表。 | HC-019 |
| SR-061 | 直接功能缺失：候选“合并 canonical”只改状态，未写 alias。 | 审批事务内追加 alias+终态；不另建映射表。 | HC-015 |

## SR-062～SR-100

| SR | 第一轮：真实业务结论 | 第二轮：最小化/不过度工程结论 | 决策项 |
|---|---|---|---|
| SR-062 | 条件 P1：Management 若选择裸 `message_send_text`，会绕过 Outbox、Review 与可靠投递。 | 从动态工具目录删除直接发送，或将其适配到现有发送网关；不再造第二套发送审批。 | HC-020 |
| SR-063 | 直接安全边界缺失：Dangerous 确认门默认关闭，风险判定由同一 LLM 自报。 | 代码层对写/发/删/切换类工具默认确认，显式白名单少量安全动作；不做复杂策略 DSL。 | HC-020 |
| SR-064 | 条件恢复风险：工具副作用与审计分步，崩溃后命令永久 running 或不可安全重试。 | 执行前写 durable intent/idempotency key，完成后 CAS finalize；复用 command/tool 表，不建工作流引擎。 | HC-020 |
| SR-065 | 直接红线冲突：Management LLM 可触发 `staff_confirmed` 成交，AI 成为高可信成交来源。 | 成交写入必须绑定已认证操作者的显式确认；LLM 只可起草 payload，不新增另一套成交系统。 | HC-020 |
| SR-066 | 条件且不可物理撤回：in-flight worker 越过安全门后，取消状态不能阻止远端发送。 | 发送前最后一刻复查 cancel token；进入网络调用后 UI 改称“尽力取消/送达不确定”，不承诺强撤回。 | HC-010 |
| SR-067 | 直接运营割裂：统一待审箱漏掉疑似成交，核实仍需另找入口。 | 将现有队列加入统一投影和计数即可；不合并底层集合。 | HC-019 |
| SR-068 | 条件可观测问题：数据库错误被显示成零待办，会掩盖审核积压。 | 返回 typed `ok/error/partial` 与各源错误；不引入新监控平台。 | HC-007 |
| SR-069 | 直接前端故障：Playbook 生成/优化请求契约漂移，正式 UI 确定失败。 | 对齐一个共享 DTO/契约测试；不保留兼容两套错误 payload。 | HC-018 |
| SR-070 | 直接治理风险：所谓 AI 候选会原地改生产 Playbook，且无可回滚版本。 | 生成只写 draft，新版本显式 publish；复用现有 Playbook collection/version。 | HC-018 |
| SR-071 | 条件配置异常：默认指针多步写，可零/多 default。 | 以 expected version CAS 切默认并加唯一约束；不建设中央配置服务。 | HC-018 |
| SR-072 | 条件高影响：DomainProfile 激活跨四类对象半提交却返回成功。 | 把画像激活与附属动作拆成可见步骤；核心 active 切换原子，附属失败返回 partial/retry，不追求跨全库大事务。 | HC-014 |
| SR-073 | 直接确认失真：高风险确认不绑定冻结内容，确认对象可被替换或绕过。 | 候选保存 content hash/base version，确认时 CAS；不引入签名 capability 服务。 | HC-014 |
| SR-074 | 直接破坏性运维：reset 先删历史再插默认，数据丢失必然且失败可留空。 | 先插新版本并验证，再原子切指针；旧版本归档而非删除。 | HC-014 |
| SR-075 | 直接语义违约：Campaign preview 改生产状态并可重开 completed，dry-run 也有副作用。 | preview 改为纯计算；如需缓存只写独立 preview artifact，不改变 campaign 状态。 | HC-021 |
| SR-076 | 条件触达风险：Campaign 去重、Task、终态分步写，崩溃可丢关联或重复。 | 每目标建立 durable send item，以唯一键+状态推进；复用 campaign_sends，不新建队列产品。 | HC-021 |
| SR-077 | 直接业务错误：复用 draft 时前端编辑条件未保存，最终仍按旧受众推送。 | preview/dispatch 前 PATCH 同一 campaign spec，并显示冻结摘要；不复制“前端草稿”真相。 | HC-021 |
| SR-078 | 条件低危：活跃写入期间多个 count 非同一快照，比率可短暂矛盾。 | 返回 `asOf` 并尽量单次 aggregate；无需为看板引入强事务快照。 | HC-007 |
| SR-079 | 条件性能债：查询/索引不匹配且逐行联系人查询，数据量大时放大延迟。 | 先补复合索引和批量 join，并用实测阈值决定是否物化；不预建分析仓。 | HC-007 |
| SR-080 | 条件租户错误：联系人启用只按裸 account_id 校验。 | 校验改 `(workspace,account)`；与 HC-004 同一 scope 修复。 | HC-004 |
| SR-081 | 直接数据损坏：重复导入把缺失身份字段解释为 null 覆盖。 | 使用 patch 语义，仅更新明确出现字段；不做复杂 merge engine。 | HC-022 |
| SR-082 | 条件业务半提交：先 managed 后建画像任务，失败留下无画像托管联系人。 | 先建幂等任务再切 managed，或同事务提交；失败显式重试。 | HC-022 |
| SR-083 | 条件竞态：在途画像任务可把已禁用联系人重新置 managed。 | 任务携带 expected agent status/generation，写回 CAS；不取消后台画像能力。 | HC-022 |
| SR-084 | 条件性能债：联系人页最多 500 次最近消息查询。 | 当前页一次聚合/batch query；无需维护新的“最近消息”服务。 | HC-022 |
| SR-085 | 条件功能缺失：Evolution 只跑默认 workspace/account，非默认开关无消费者。 | 若产品单租户则禁止其它 workspace 开启；否则 worker 枚举启用 scope，二选一。 | HC-004 |
| SR-086 | 条件发布错误：候选不绑定评估基线，晚发布把旧证据应用到新配置。 | proposal 保存 base version/hash，release CAS 校验；不重新实现完整 Git。 | HC-017 |
| SR-087 | 条件回滚越界：旧 proposal rollback 可翻掉后续合法版本。 | rollback 仅允许当前版本是该 proposal 产物，或创建新回滚 proposal；不物删历史。 | HC-017 |
| SR-088 | 条件审计缺口：生产变更已提交后事件/复盘任务失败，调用方可能见错误且永失观测。 | 事务内写 audit/review intent，后台幂等处理；复用现有表。 | HC-017 |
| SR-089 | 条件稳定性：非 ASCII key 的 camelCase 转换使用错误字节下标可 panic。 | 用 Unicode-safe 字符迭代或成熟 key normalizer；不限制行业字段为 ASCII。 | HC-014 |
| SR-090 | 直接工作流不可达：AI 生成 DomainProfile 草稿被所有正式列表过滤。 | 给 draft 明确列表/审核状态并接现有发布卡；不新建审批中心。 | HC-014 |
| SR-091 | 条件陈旧确认：Guide preview 不绑定 contact/memory/playbook/domain 基线。 | preview 保存 base versions/hash，apply 时 CAS 或重算差异；不冻结整库快照。 | HC-023 |
| SR-092 | 直接无效写：Guide 标签建议写入运行时不消费的旧字段。 | 写入权威 manual/candidate 层或删除该建议能力；不维护双写兼容。 | HC-023 |
| SR-093 | 直接授权失真：全局副作用范围和摘要由模型自报，服务端不展示真实 diff。 | 服务端从 payload 计算 target/diff 并确认；不再让模型生成授权范围。 | HC-023 |
| SR-094 | 直接配置风险：Domain runtime 自由 BSON 可写危险值。 | 共用一个 typed validator/allowlist 于 Guide 和手工编辑；不引入 schema 服务。 | HC-014 |
| SR-095 | 条件响应失真：事务已提交后重建响应失败会返 502，重试又冲突。 | commit 后返回稳定 receipt/id，详情读取另行重试；不回滚已提交事务。 | HC-023 |
| SR-096 | 直接产品误导：软锁跨 workspace 且不是写入门，UI 却称只读互斥。 | 最小选择：明确标 advisory 并修 scope；若业务真需互斥，再让 mutation 校验 owner/token。 | HC-024 |
| SR-097 | 条件重复候选：Lesson promote 建 Chunk 与标来源分步且无 CAS。 | source lesson id 建唯一键/upsert，同事务或幂等 finalize；不建通用工作流。 | HC-019 |
| SR-098 | 直接评测失真：active 场景缺 ground truth 仍被当零分真值。 | active 前校验最小 schema；缺金标标 `unscored`，不要猜 0。 | HC-026 |
| SR-099 | 直接预算错误：公式评测从生产 run 读 token，空闲时恒零、并发时受污染。 | 在评测 run 内本地累计预算；不复用生产日志作控制面。 | HC-026 |
| SR-100 | 直接语义误导：Observability 混合时间窗并把存量比率叫本轮命中率。 | 修字段名并返回每项 window/asOf；无需统一所有指标为一个窗口。 | HC-007 |
| SR-101 | 直接安全风险：通用 Chunk PUT 可把 AI 修复自动置 verified，绕过人审。 | 服务端按来源强制 AI 写入 `draft+needs_review`，verify 只保留专用人工入口；不新增审核系统。 | HC-024 |
| SR-102 | 直接跨租户写入：Split 的服务端 scope 可被请求字段覆盖，生成其它 workspace 的 active verified Chunk。 | 对后端托管字段做白名单剥离并强制继承源 scope/status；不接受客户端整对象反序列化。 | HC-024 |
| SR-103 | 条件数据损坏：Split/Merge 多步提交会留下源已归档而结果不完整。 | 预校验全部参数后用事务；无事务时写 operation intent 并幂等补偿，不另建编排服务。 | HC-024 |
| SR-104 | 直接功能错误：Revision 只存 patch，却被当快照回滚，恢复内容错误。 | 要么存完整 snapshot，要么从初始版本顺序重放 patch；优先选前者的简单可靠实现。 | HC-024 |
| SR-105 | 直接前后端断裂：正式操作栏的 Patch/Split/Merge/Relate 请求不符合后端契约。 | 以共享 DTO/契约测试对齐现有端点；不新增兼容端点长期维护两套协议。 | HC-024 |
| SR-106 | 直接审计失真：`repair applied` 信任客户端自报且吞写失败，不能证明修复发生。 | 由服务端业务提交生成 applied 事件并携 committed revision id；删除独立自报入口优先。 | HC-024 |
| SR-107 | 条件统计失真：Auto-verify 在 revision 成功前计数并吞失败。 | revision 成功后再累计，失败进入明确结果；不增加复杂补偿。 | HC-024 |
| SR-108 | 直接审计绕过：文档/Chunk 物理替换与删除绕开 revision 和引用生命周期。 | 所有编辑走现有 revision；删除改软归档并检查引用，不建设第二套历史库。 | HC-024 |
| SR-109 | 直接高风险：管理员可配置任意 URL，Worker 跟随重定向形成持久 SSRF。 | 统一 URL egress policy：仅 http(s)、解析后拒私网/metadata、每次重定向重验；无需专用抓取微服务。 | HC-025 |
| SR-110 | 直接租户泄露：metadata 的 revision 聚合未筛 workspace。 | Revision 持久化 workspace 并在聚合 filter/index 中强制使用；与租户作用域决策合并。 | HC-004 |
| SR-111 | 条件重复/串账号：Chat apply 无原子认领，sessionId 又不含 account。 | turn 加 workspace/account/status CAS 与 committedChunkId 幂等回执；不重写 Chat。 | HC-025 |
| SR-112 | 直接准入错误兼条件半提交：Import Apply 仍用废弃 items，文档/Chunk 分步写。 | 删除 items 门，先完整校验 preview hash，再事务写 document+chunks；无事务用 import intent。 | HC-025 |
| SR-113 | 直接跨租户写入：Chunk Patch 可改 workspace/status/integrity。 | 请求只允许业务可编辑字段，scope与审核字段服务端继承；与 HC-024 合并。 | HC-024 |
| SR-114 | 条件历史失真：先写 revision 后替换主行会留孤儿，provenance 又使 no-op hash 漂移。 | 主行 CAS 与 revision 同事务；内容 hash 排除审计元数据；无需事件溯源重构。 | HC-024 |
| SR-115 | 条件长期陈旧：Catalog rebuild processing 无 lease/retry。 | 在现有 job 增 owner/lease/attempt 与启动回收；不引入外部队列。 | HC-025 |
| SR-116 | 条件跨账号归因：成交追认按裸 wxid 合并。 | 聚合键补 account；与租户/账号身份簇合并。 | HC-004 |
| SR-117 | 条件重复导入：Ingest source 无 claim generation，旧结果可覆盖新 checkpoint。 | source 加 claim token/generation，finalize CAS；不新增调度器。 | HC-025 |
| SR-118 | 直接前端错误：知识诊断页误读 Catalog 包络，有数据仍显示 0/0。 | 修响应解包并以共享类型生成客户端；这是独立小修。 | HC-035 |
| SR-119 | 直接/条件混合：按需 Digest 使用错误租户 Prompt，定时又只覆盖默认账号。 | 先让按需路径传真实 scope；多租户调度是否扩展由部署事实决定。 | HC-004 |
| SR-120 | 直接配置失效：Digest 暴露预算配置但生产固定 24000/8。 | 要么接入现有 RunBudget配置，要么删除无效配置；不维护假开关。 | HC-028 |
| SR-121 | 条件数据覆盖：失败重算会用空卡覆盖当日成功报告。 | 生成新版本成功后再切 current；失败保留旧报告并记录失败，不建复杂版本平台。 | HC-028 |
| SR-122 | 条件可恢复性：Knowledge Task 无 lease/幂等 step，崩溃永久 running或重放。 | 在现有 task 加 owner/generation/lease，step用 task+step id幂等；不引入新队列。 | HC-028 |
| SR-123 | 直接状态失真：确定失败被包装成 Ok，任务最终虚报成功。 | StepOutcome 改成结构化 `committed|noop|needs_manual|failed` 并按结果汇总；不靠错误字符串推断。 | HC-028 |
| SR-124 | 条件跨账号隐藏：Digest cardId与dismiss filter缺 account。 | card identity/filter补 account；与租户身份簇合并。 | HC-004 |
| SR-125 | 直接授权缺口：对话派工不绑定运营选中的卡片。 | 创建 task 必带 cardId/version/hash，服务端验证后生成 steps；不冻结整个日报。 | HC-028 |
| SR-126 | 治理：Knowledge 质量测试绕过真实入口，自证不能证明业务闭环。 | 只重写少数红线/闭环 case 走真实 handler/worker；不把所有纯函数测试废弃。 | HC-032 |
| SR-127 | 直接 UX 故障：Knowledge Ask 流把 Agent 失败当普通 close，界面无答案也无错误。 | SSE 发送 typed terminal `completed|failed|cancelled`，前端显示失败；无需新流协议。 | HC-036 |
| SR-128 | 治理：真模型套件允许关键能力零发生仍绿色。 | 每 case 要求最小 artifact/assertion 见证，零产物标 inconclusive/skip；与测试门治理合并。 | HC-032 |
| SR-129 | 直接配置失效：Cockpit 发 snake_case，后端忽略阈值/抽审比例。 | 共享 DTO并拒未知字段；不同时兼容多种命名。 | HC-027 |
| SR-130 | 直接对象错绑：审核对话未绑定当前 Chunk且读错响应字段。 | 请求显式 chunkId+expectedVersion，响应按共享 DTO；与前端治理簇合并。 | HC-027 |
| SR-131 | 直接数据损坏风险：文档编辑误读包络，保存 `undefined` 并可能清原文。 | 修解包、未加载/undefined时禁保存、PATCH只发dirty字段；独立紧急小修。 | HC-034 |
| SR-132 | 直接列表错误：默认审核类别不可达，覆盖下钻不筛且混归档。 | 修枚举映射和服务端 filter；无需重做评审队列。 | HC-027 |
| SR-133 | 条件生产配置重复：同 threshold proposal并发 release 可写多条 active override。 | proposal release做状态+base version CAS并给 override唯一键；与 Evolution发布簇合并。 | HC-017 |
| SR-134 | 条件功能缺失：Planner/Cold/Silence只扫描默认 workspace。 | 若单租户则在配置/UI禁止其它租户开启；否则枚举启用 scope，和 HC-004 同一产品决策。 | HC-004 |
| SR-135 | 直接/条件混合：Planner无持久 intent，calendar可串行重复，其它路径并发超配额。 | 每触达建立业务幂等 intent并以数据库日桶原子预留配额；不引入中央调度平台。 | HC-029 |
| SR-136 | 条件竞态：ImportJob lease无 generation，旧 worker可覆盖新执行。 | 现有 lease加 generation并在heartbeat/finalize CAS；与 worker所有权模式复用。 | HC-029 |
| SR-137 | 条件身份碰撞：BehaviorSignal缺 account。 | 模型/唯一键补 account并迁移；与租户作用域簇合并。 | HC-004 |
| SR-138 | 条件但灾难性：Prompt pack探测读失败会触发破坏性重置。 | 读错误必须 fail-closed，只有明确 NotFound且显式维护动作才初始化；不做自动“自愈重置”。 | HC-018 |
| SR-139 | 直接安全治理：Prompt语义闸看不见纯删除且锚不完整。 | diff审查同时传 before/after，扩充最小关键锚；不增加第二个审查 Agent。 | HC-018 |
| SR-140 | 条件审计失真：A/B实际版本被丢弃，统一记最高 active。 | 决策时把真实selected version写run；不改变A/B分桶。 | HC-007 |

## SR-141～SR-183

| SR | 第一轮：真实业务结论 | 第二轮：最小化/不过度工程结论 | 决策项 |
|---|---|---|---|
| SR-141 | 条件界面陈旧：Chunk WS 明示 lagged 后前端吞掉，已开详情可停在旧版本。 | 收到 lagged 就重拉当前对象并显示短暂提示；不做持久事件流。 | HC-027 |
| SR-142 | 直接切号错误：共享联系人 store 无 account 身份，旧列表稳定残留且迟到响应可覆盖。 | store key 加 account、切号清空、响应带 generation；与同类前端身份问题共用模式。 | HC-030 |
| SR-143 | 直接越权操作窗口：B 账号可确认 A 账号留下的 Management 命令。 | 命令 DTO 保留 accountId，切号清 pending，确认请求带 expected account；后端也校验。 | HC-020 |
| SR-144 | 直接秘密错写：A 未提交 MCP key 可在切 B 后保存到 B。 | 组件以 account id 设 key并在变化时清明文；无需全局表单框架。 | HC-030 |
| SR-145 | 直接功能故障：扫码登录读取错误字段，按后端契约不会展示二维码/轮询。 | 共享 login DTO或修正字段映射并加契约测试；独立小修。 | HC-030 |
| SR-146 | 条件数据错写：迟到 A 详情可覆盖 B 草稿并保存。 | 每请求带 object generation，响应仅在 id+account仍匹配时提交；不引入请求状态库。 | HC-030 |
| SR-147 | 直接准入漏洞：系统账号仍可勾选，后端 batch-enable 也不复核真人。 | 后端复用既有真人判定作最终闸，前端只作提示；不扩展身份分类系统。 | HC-022 |
| SR-148 | 直接跨账号写：切号后可继续编辑/设默认 A 的 Playbook。 | 编辑状态绑定 account+id，切号清空，写端校验 expected account；与 HC-030 合并。 | HC-030 |
| SR-149 | 直接权威数据错写：人工标签草稿不绑定联系人，切换后可覆盖 B。 | 草稿按 contact id key或切换即丢弃，提交绑定打开时 id/version；不做复杂草稿同步。 | HC-030 |
| SR-150 | 条件错确认：A Guide 预览迟到后可在 B 页面确认并真实应用。 | 预览响应绑定 contact/account/generation，确认前服务端再校验；业务基线问题仍归 HC-023。 | HC-023 |
| SR-151 | 直接无效 UI：运营风格模板只改本地字符串，启用/保存不提交 Playbook。 | 要么真正发送 playbookId，要么删除该控件；不保留装饰性配置。 | HC-018 |
| SR-152 | 直接错 scope：非默认账号复盘省略 account，固定查默认账号。 | API 强制 accountId并校验归属；与租户/账号上下文统一。 | HC-004 |
| SR-153 | 条件跨账号纳管：A 勾选态切 B 后仍可批量启用。 | selection 以 account+contact 为 key，切号清空，后端校验请求账号；与 HC-030 合并。 | HC-030 |
| SR-154 | 直接展示失真：容器更新时间被当阶段更新时间，任意画像写都重置“阶段未变”。 | 持久化/投影真正 state_changed_at；不构建事件溯源。 | HC-007 |
| SR-155 | 条件跨账号任务操作：B 页面可执行/取消 A 的旧快照任务。 | 任务响应保留 account，切号清空且动作带 expected account；后端拒不匹配。 | HC-030 |
| SR-156 | 直接指标低报：成本页把最近100条样本标成总量。 | 服务端聚合总量，列表另分页；短期必须改文案为“最近100条”。 | HC-007 |
| SR-157 | 条件报表错绑：迟到 A campaign 明细可永久显示/导出为 B。 | 响应提交前核对 campaignId/generation；不建立全局请求取消层。 | HC-021 |
| SR-158 | 条件安全：CSV 外部文本未中和公式前缀。 | 导出层统一对 `=+-@` 前缀加安全转义；不引入第三方报表系统。 | HC-021 |
| SR-159 | 条件错误取消：B 可取消 A 账号旧 Outbox 快照。 | 快照/动作绑定 account，切号清空；发送竞态本体仍归 HC-010。 | HC-030 |
| SR-160 | 条件生产资产错写：B 可操作 A 私有素材旧快照。 | 资产 DTO/动作携 expected account，切号清空；workspace共享资产显式标共享。 | HC-030 |
| SR-161 | 直接高可信事实错写：成交页切号后仍可给 A 联系人追加成交。 | 选择绑定 account+contact，切号清空，后端按两者校验；与 HC-030 合并。 | HC-030 |
| SR-162 | 直接作用域错配：workspace决策链选领导但不保存可发送账号，别的账号请示可成幽灵 pending。 | 策略若 workspace级就为每位决策人保存 account映射；否则下沉为 account级，二选一。 | HC-013 |
| SR-163 | 条件配置破坏：请示策略 GET 失败后默认草稿仍可覆盖生产。 | 加载失败禁保存，区分 unknown与明确空值；不做离线配置缓存。 | HC-013 |
| SR-164 | 直接契约错位：UI 不能保存空决策链或清 quiet hours，无法执行后端“空即关闭”。 | 表单支持显式 null/空关闭并显示 effective值；不增加新状态。 | HC-013 |
| SR-165 | 直接生产变更风险：编辑 active provider 普通保存即热切全进程且未要求连通测试/确认。 | active行编辑走“测试成功→确认→原子替换”，或强制另建草稿再激活；不建复杂发布平台。 | HC-031 |
| SR-166 | 直接配置错觉：超时/重试留空不恢复默认。 | nullable PATCH区分未改与清除，响应回显 effective来源；不保存魔法空字符串。 | HC-031 |
| SR-167 | 条件能力中断：视觉指派与 supportsVision/删除生命周期不一致。 | 维护 `visionActive => supportsVision`，删除/关闭前原子清除或改派；不做能力调度器。 | HC-031 |
| SR-168 | 直接运行语义破坏：字典编辑投影漏 terminal/reactivation，普通保存将其清零。 | DTO完整往返运行时消费字段并用 dirty PATCH；不把整个字典改成自由 BSON。 | HC-014 |
| SR-169 | 直接人审错对象：列表无稳定 key，上一行草稿可提交给下一对象。 | ReviewQueue统一用对象 id做 key，卡片 id变化清草稿并绑定提交 generation；不重做审核中心。 | HC-019 |
| SR-170 | 条件安全控制失效：Evolution父开关不拦 auto-release，PUT失败UI还假关。 | 所有副作用复用同一父闸；UI成功读回后才更新权威状态。 | HC-017 |
| SR-171 | 直接指标低报：7天面板只取20个实验，默认频率约覆盖5天。 | 服务端按时间窗聚合并返回coverage；不拉全量到前端。 | HC-017 |
| SR-172 | 直接运营盲区：媒体/名片 Outbox 显示空白，用户只能盲取消。 | DTO投影typed payload名称/id和uncertain状态；不把完整媒体正文复制进Outbox。 | HC-010 |
| SR-173 | 条件界面停更：SSE成功重连不恢复预算，累计短断线后永久放弃。 | open/心跳后重置连续失败，gave-up时有界轮询兜底；不引入WebSocket替代。 | HC-027 |
| SR-174 | 治理：测试数据库隔离但进程级cache共享，默认并行可假红/假绿。 | cache注入AppState或key含database identity；短期串行仅作过渡。 | HC-032 |
| SR-175 | 直接P1：Reviewer危险字段缺失/畸形被解析为0并当安全，多闸可同时失效。 | 实时wire DTO严格必填/范围校验，失败hold或重试；历史兼容只留读侧。 | HC-009 |
| SR-176 | 治理：大量“隔离测试”手抄Mongo filter，不能证明Handler使用它；不宣称生产当前越权。 | 高风险域改为真实Router/Handler测试，保留少量纯DB测试；不为每个CRUD复制全矩阵。 | HC-032 |
| SR-177 | 条件但客户可见：Webhook已ACK后待处理只在内存，崩溃无新消息时永久不回复，多副本还可重复。 | 单副本且可接受丢失时可明确约束；否则用现有messages加pending/generation和lease worker，不另建消息中间件。 | HC-010 |
| SR-178 | 治理高风险：真模型红线目标行为零发生仍pass且不记skip。 | 每case输出typed outcome并要求artifact>0/assertionsRun>0；不强求所有模型波动都fail。 | HC-032 |
| SR-179 | 治理：任务表把implemented/production-wired/verified混成全勾选。 | 改有限状态+绑定生产入口/测试artifact；不建设大型需求管理平台。 | HC-033 |
| SR-180 | 契约：正式需求要求人工release，代码提供可开启threshold auto-release。 | 人类先选唯一政策；接受则同步spec/闸/审计，拒绝则删调用与配置，不保留双真相。 | HC-017 |
| SR-181 | 直接生命周期缺口：运营记忆承诺可撤销但只有新增/读取，错误偏好无限期注入。 | 增 `revoked_at/by/reason` 软失效+Chat intent+列表动作；不做复杂版本树。 | HC-011 |
| SR-182 | 契约且可触发：coreFacts cap 6与未废弃永久保留不可同时满足，生产静默挤旧值。 | 保持cap但定义稳定排序和归档/eviction reason；不承诺无限prompt保留。 | HC-011 |
| SR-183 | 治理：47域审计允许空证伪、静默漏域且不可复现，却被称事实底座。 | 强schema+complete/inconclusive/failed manifest和机器汇总；不要求重建通用审计平台。 | HC-033 |
