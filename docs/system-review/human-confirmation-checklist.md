# 人类逐项确认清单

本清单由 `findings.md` 的 SR-001～SR-183 经两轮循环复审生成：第一轮核对真实业务场景、生产可达性与实际影响；第二轮核对最小可行处理和反过度工程边界。逐 SR 结论见 `two-pass-review-ledger.md`。合并只合并同一业务取舍，原始证据不删除。

## 使用方式

- 每项请选择：`修复 / 接受风险 / 暂缓 / 非问题 / 需补业务事实`。
- “推荐”是审查建议，不替代产品、安全或运维所有者决定。
- 若选择接受风险或暂缓，请填写适用部署边界、到期日和重新触发条件。
- 负责人、决定和理由留空，等待人类填写。

汇总：36 个决策项覆盖 183 条 SR，映射恰好一次；其中部署模式、开关状态、凭证是否已轮换等仓库外事实均显式列出。

## HC-001：轮换并收口仓库与 Workflow 中的凭证

- 来源：SR-001、SR-002、SR-005
- 两轮结论：凭证进入 Git/普通 workflow input 的暴露事实成立；是否仍有效、是否已泄漏只能外部核实。三项共用同一个密钥生命周期决策。
- 推荐：`修复（立即）`
- 最小处理：先在上游轮换/撤销，再改 GitHub Secret 或部署环境注入，最后审计历史 run、fork、artifact、服务器副本并按必要性清理历史。
- 不过度工程边界：不以建设新 secret 平台作为轮换前置；现有 GitHub/environment secret 足够完成首轮闭环。
- 不处理代价：仍有效的 key 可被仓库/历史读取者使用；只删当前文件不能消除历史暴露。
- 已确认外部事实：旧 key 当前应已失效；Actions、服务器、镜像是否仍留副本尚未核验。
- 人类决定：`修复（按已失效凭证处理：清除仓库/脚本中的值并改 secret 注入；历史副本审计列为上线前检查）`
- 负责人：
- 决定理由/边界/到期日：项目尚未生产上线；旧值失效降低即时风险，但不消除历史暴露和未来误复用风险。

## HC-002：确定 Evolution 唯一默认开关语义

- 来源：SR-003
- 两轮结论：冻结代码与 `.env.example` 默认开启，README 写默认关闭；这是契约冲突，不等于 Evolution 本身必须删除。
- 推荐：`修复文档与默认值；由产品选择开或关`
- 最小处理：确定唯一默认，统一 config 常量、`.env.example`、README、运维说明和回归测试。
- 不过度工程边界：不借机重构 Evolution；只消除双真相。
- 不处理代价：运维可能在错误预期下启用或关闭生产演化。
- 人类决定：`默认关闭`
- 实施状态：`已完成（2026-07-18）`
- 负责人：实现：Kiro；产品决定：用户确认
- 决定理由：系统仍处测试优化阶段；Evolution 必须由显式开关启用。`EVOLUTION_ENABLED_DEFAULT`、`.env.example`、启动注释与回归测试现已统一为默认关闭；仅显式设置 `EVOLUTION_ENABLED=true` 后，Mongo runtime flag 才能进一步放行。
- 验证：`cargo test --lib evolution_enabled_defaults_to_false -- --exact` 通过；`cargo check --lib` 通过。

## HC-003：修复迁移与版本指针的存量一致性

- 来源：SR-007、SR-008、SR-009、SR-010
- 两轮结论：迁移可选中 draft、跨租户回填身份、跳过后仍记完成，且三张版本表允许零/多 current；这些是升级和异常恢复中的真实数据风险。
- 推荐：`修复`
- 最小处理：先跑存量审计和校正；迁移选择 active/current 的明确优先级；scope 使用 `(workspace,account,entity)`；跳过迁移记录 `blocked/pending`；清理后增加 partial unique 或 CAS。
- 不过度工程边界：不新建迁移编排服务；沿用现有 migration collection 和 Mongo 约束。
- 不处理代价：升级可激活草稿、污染租户身份或让运行时随机/回落配置。
- 人类决定：`待确认（修复 / 接受仅全新库风险 / 暂缓 / 非问题）`
- 负责人：
- 决定理由/存量库范围：

## HC-004：选择并落实单租户或多租户产品契约

- 来源：SR-011、SR-012、SR-013、SR-018、SR-021、SR-024、SR-025、SR-026、SR-027、SR-036、SR-080、SR-085、SR-110、SR-116、SR-119、SR-124、SR-134、SR-137、SR-152
- 两轮结论：代码和 UI 暴露 workspace/account 能力，但 MCP、LLM、Prompt、事件、Worker、幂等键、统计与部分知识路径丢失租户维度。若生产只允许单 workspace，部分风险不可达；若支持多 workspace，则有错账号发送、跨租户元数据/知识污染和能力缺失。
- 推荐：`先确认产品契约；多租户则修复，单租户则显式禁止第二租户`
- 最小处理：多租户模式统一传递 `(workspace_id,account_id)`，补 filter/key/index/cache namespace，worker 枚举启用 scope；单租户模式在启动、管理 API 和 UI 拒绝第二 workspace，不再假装支持。
- 不过度工程边界：不做抽象的“租户平台重构”；优先修改现有公共 helper 签名，由编译器暴露漏点。
- 不处理代价：隐式混合模式既没有单租户约束，也没有多租户隔离，是当前最高系统性风险之一。
- 已确认产品契约：正式支持多租户；当前部署是一个系统/单实例，但同一系统必须安全承载多个 workspace。
- 人类决定：`正式支持多租户并修复`
- 负责人：
- 决定理由/部署边界：不能用“当前单实例”豁免 workspace/account 隔离；先修同进程多租户作用域，横向多副本能力另按未来部署演进。
- SR-036 实施状态（2026-07-19）：`已修复并验证 / 尚未部署`。Outcome task 唯一键和 m017 去重分组均已纳入 `workspace_id`，m033 安全退役旧跨租户索引；索引契约 5/5、m017 workspace 分组 1/1、迁移序列 2/2 与 `cargo check --lib` 通过。授权测试服务器随机隔离 Mongo 库运行 `tests/outcome_task_workspace_dedupe.rs` 1/1 通过：跨 workspace 同账号/内容可并存，同 workspace 重复被唯一索引拒绝，最终恰有两行；测试库零残留，运行服务 PID/二进制哈希未变化，`/api/health` 200。该结论只关闭 SR-036，不代表 HC-004 的其它租户问题已完成。

## HC-005：收紧认证撤权、appId 唯一性与登录频控

- 来源：SR-014、SR-015、SR-016
- 两轮结论：ACL 收缩不会即时影响既有 session/JWT，appId 被当唯一键却无唯一约束，登录/token 无应用级频控；外围 WAF 是否补偿未知。
- 推荐：`修复`
- 最小处理：敏感请求重读 ACL或权限版本；appId 存量查重后建 partial unique；login/token 共用轻量 IP+账号限流与失败审计。
- 不过度工程边界：短 JWT TTL/权限版本足够，不先建设完整撤销中心或复杂反欺诈系统。
- 不处理代价：已撤权用户仍有窗口，重复 appId 可错路由，公网登录可被猜密/消耗 CPU。
- 已确认外部事实：当前无 WAF/反向代理限流；项目尚未生产上线，仍在测试优化阶段。
- 人类决定：`分阶段修复：ACL 撤权与 appId 唯一性纳入当前修复；登录/token 轻量限流作为公网生产上线前硬门，当前不阻塞测试优化`
- 负责人：
- 决定理由：现阶段没有公网生产流量，不需要提前建设复杂风控；但上线前不能依赖尚不存在的外围补偿。

## HC-006：定义媒体文件与元数据的一致性和部署边界

- 来源：SR-017
- 两轮结论：文件系统和 Mongo 非原子；单机可通过补偿降低风险，多副本还要求共享存储。
- 推荐：`修复最小补偿；按部署决定是否迁对象存储`
- 最小处理：临时文件→DB 提交→原子改名，失败清理；定期扫描孤儿/缺失对象。
- 不过度工程边界：未确认多副本前不强制对象存储；确认多副本后才要求共享卷或对象存储。
- 不处理代价：孤儿文件、悬空资产、备份不一致；多副本下文件不可见。
- 已确认部署边界：当前是一个系统/单实例，暂无多副本要求。
- 人类决定：`先实现单实例补偿协议；对象存储/共享卷延后到明确需要多副本或高可用时`
- 负责人：
- 决定理由：临时文件、原子改名、失败清理和定期扫描足以覆盖当前阶段；现在迁对象存储属于过度工程。

## HC-007：让运行、指标与审计陈述真实发生的事实

- 来源：SR-019、SR-068、SR-078、SR-079、SR-100、SR-140、SR-154、SR-156
- 两轮结论：run 首段失败可消失；DB 错误被显示为零；非快照计数和错误时间字段/样本上限被包装成精确指标；A/B实际版本丢失。多数不会直接改变客户消息，但会误导运营和事故判断。
- 推荐：`分级修复，先修运行追踪和错误伪零`
- 最小处理：入口先写 started、统一 terminal；错误返回 unknown/error；指标返回 asOf/window/sample/truncated；记录真实 selected version；修正误导字段名。
- 不过度工程边界：不建设实时数仓或全量 event sourcing；Mongo 聚合和明确不确定状态足够。
- 不处理代价：故障、成本和版本归因不可相信，运营可能依据错误百分比决策。
- 人类决定：`待确认（全部修复 / 仅关键审计 / 接受展示误差 / 暂缓）`
- 负责人：
- 决定理由/优先子项：

## HC-008：使 LLM 精确缓存感知租户和 provider 版本

- 来源：SR-020
- 两轮结论：热切 provider 后，完全相同输入可命中旧 provider 产物；范围限于白名单精确缓存，但违反切换预期。
- 推荐：`修复`
- 最小处理：cache key 加 workspace、provider/model 和 provider generation；激活时 bump generation。
- 不过度工程边界：保留进程内缓存，不引入 Redis 或全局失效服务。
- 不处理代价：切换后的短期结果和日志仍来自旧 provider。
- 人类决定：`待确认（修复 / 接受短 TTL 陈旧 / 暂缓 / 非问题）`
- 负责人：
- 决定理由：

## HC-009：统一客户发送前的事实与安全 fail-closed 边界

- 来源：SR-022、SR-023、SR-031、SR-042、SR-051、SR-175
- 两轮结论：Reviewer漏报/畸形可使产品声明和三类危险评分失效；revision失败可恢复已知危险原稿；quote、价格和数字护栏不完整。均可进入真实发送链。
- 推荐：`修复（P0/P1）`
- 最小处理：严格 Reviewer wire DTO；安全类 revision失败 hold；服务端校验 opened/quote/anchor 和结构化产品价格；holding reply复用现有数字护栏；产品效果类漏判强制复审。
- 不过度工程边界：不新增第二套通用审核 Agent或关键词大全；复用现有 finalize、目录和确定性检查。
- 不处理代价：编造事实、压迫话术、内部信息或错误价格可能被标安全并发送。
- 人类决定：`修复（已实现；真实模型验证阻塞）`
- 实施状态：`production-wired / deterministic-verified / real-model-blocked（2026-07-18）`
- 负责人：Kiro
- 决定理由/允许回退的纯风格类别：安全与事实失败一律 fail-closed；只有不涉及事实、隐私、压迫或授权边界的纯风格改写失败才允许恢复原稿。SR-022/175 以严格实时 Reviewer wire DTO + 独立语义 Claim Gate 收口；SR-023 的安全类 revision 失败保持 hold；SR-031 只接受 verified、opened、非 contradiction 且 quote/anchor 闭环的证据；SR-042 的 holding reply 经独立语义审查与全场景数字授权校验并统一走 Outbox；SR-051 由 Claim Gate 语义抽取目录事实，服务端逐字段核验 productId/name/amountMinor/currency/SKU，完整性失败直接 hold。
- 生产接线：客户主链、revision 二审、管理发送、Knowledge Agent answer、授权过期与链尾 holding reply 均已切入上述边界；旧的“任一 quoted product id 命中即目录背书”入口已删除。
- 确定性验证：`cargo test --lib strict_evidence_`（5/5）、`cargo test --test knowledge_agent_pbt`（17/17）、`cargo test --lib agent::review::`（97/97）、`cargo test --lib agent::escalation::`（96/96）、`cargo check --lib` 均通过；`git diff --check` 通过。
- 真实模型验证：`blocked`。2026-07-18 21:06 +08:00 对授权端点 `127.0.0.1:9090` 只读检查得到 `NO_LISTENER_9090`，模型列表请求被拒绝；未发送项目代码、数据库内容或测试任务，不能据此宣称真实模型通过。服务恢复后须补跑合成恶意任务集，才可把本项改为 fully-verified。

## HC-010：给异步任务与发送建立最小所有权和不确定送达语义

- 来源：SR-028、SR-034、SR-050、SR-052、SR-066、SR-172、SR-177
- 两轮结论：Reaction/Task/ledger/Outbox缺 owner-generation 或完整业务键；取消无法撤回已经越过门的发送；名片重试无送达核对；Webhook ACK 后只有内存 runner。
- 推荐：`修复核心发送与任务 fencing；按部署决定 durable webhook`
- 最小处理：现有记录加 owner/token/generation，finalize CAS；台账绑 outbox/account；取消后 worker在 MCP 前再检查 token；无法核对的送达标 uncertain 而非自动重发；Webhook可用 messages+pending generation 形成持久工作项。
- 不过度工程边界：不引入 Kafka/新调度平台；先扩展现有 Mongo task/outbox/message 集合。
- 不处理代价：重复发送、取消后仍发、消息永久不回复、反馈/任务旧执行者覆盖新状态。
- 已确认部署边界：当前单实例、尚未生产上线；即使单实例，ACK 后进程退出导致永久不回复仍不可作为正式业务语义。
- 建议方案：不用 Kafka。Webhook 落消息时，在 Mongo 中 upsert 每联系人一条 durable inbound job（`workspace+account+contact` 唯一，含 generation、debounce_deadline、status、owner、lease）；内存 debounce 只作加速，后台 worker 到期 claim 并聚合处理，崩溃后按 lease 重领，finalize 以 generation CAS 防旧执行者提交。Outbox/Task/Reaction 复用同样的 owner-generation-fencing 模式。
- 人类决定：`方案待确认：采用 Mongo durable inbox + 现有 Outbox/Task fencing，不引入消息中间件`
- 实施状态：`部分实现`。SR-052/SR-066 为 `production-wired / deterministic-compiled / docker-run-pending / real-model-run-pending`（2026-07-18）；SR-028/SR-034/SR-050 为 `working-tree-wired / deterministic-verified / isolated-real-mongo-verified / deployment-pending`，SR-172 为 `working-tree-wired / deterministic-verified / local-real-router-mongo-verified / deployment-pending`（2026-07-19）。
- 本批边界：Outbox 已增加 claim token/generation、最后可取消 CAS、`send_started_at` 与 `delivery_unknown`；名片跨远端边界后崩溃不再回 pending，in-flight 取消在远端成功时如实收敛 sent、回执不明时停止自动重放。Task 与 Reaction 均已增加 owner token/generation 和 owner-scoped 提交；Reaction 的轨迹、负例与 Outbox 取消只在当前 owner 成功提交后执行。SR-050 新台账以不可变 `outbox_id` partial unique + `$setOnInsert` 幂等收敛，所有历史、回扫、API 与前端统计均固定 workspace/account，存量无锚行保持兼容。SR-172 管理 DTO 已投影互斥 typed payload、不可变对象 id、恢复/取消边界，并按 workspace/account 批量解析素材或顾问名称；前端取消确认展示业务号、客户、精确目标和不可撤回风险。管理员取消、stale reclaim、prepared commit 与 Memory single-flight/rerun 交接均不允许旧 owner 提交。SR-177 durable inbound job 仍需单独修复和验证，不能因其它子项完成而关闭 HC-010。
- 验证状态：SR-034 本地 Task owner 10/10、Memory fencing 11/11、索引 5/5、迁移 3/3 与 `cargo check --lib` 通过；授权测试服务器随机隔离 Mongo 运行 `tests/sr034_task_send_fencing.rs` 4/4 通过。SR-028 的 `tests/reaction_claim_lock.rs` 本地编译通过、服务器随机隔离 Mongo 运行 2/2 通过，覆盖并发 claim 抑制与 ABA 重领后旧 owner 不能覆盖结果或触发轨迹/Outbox 取消。SR-050 本地台账契约 18/18、前端测试 2/2 与前端生产构建通过，服务器离线构建 `tests/send_ledger_integration.rs` 成功并在随机隔离 Mongo 运行 `sr050_*` 3/3 通过。SR-172 前端组件/契约 9/9、生产构建与格式门通过；`tests/sr172_outbox_projection.rs` 以真实 Cookie 调生产 Router，在本机随机 Mongo 库运行 1/1 通过，覆盖 typed payload、同联系人多对象、恢复计数、账号过滤和跨账号私有名称不泄漏，测试库零残留。以上均未部署工作树版本。SR-052/SR-066 的四条 `delivery_redline_*` 仍只有编译证据；GitHub `smoke_t4` 与真实模型 T4仍待执行。
- 负责人：
- 决定理由：比纯内存 runner 可靠，又符合当前单实例和不过度工程边界；未来多副本时协议仍可复用。

## HC-011：定义长期记忆的提交、撤销与容量契约

- 来源：SR-029、SR-181、SR-182
- 两轮结论：Memory consolidation附属写可半提交；operator memory承诺撤销却无入口；coreFacts cap与永久保留合同不可同时满足。
- 推荐：`修复并简化契约`
- 最小处理：consolidation id/phase+幂等副作用；operator memory软撤销；coreFacts保持有限窗口，按权威/重要度排序，被淘汰项进入可追溯 archive/recent。
- 不过度工程边界：不做无限版本树或把全部历史塞进 prompt；持久事实库与 prompt 投影分层即可。
- 不处理代价：错误偏好无法撤销、事实静默消失、崩溃后主卡与派生视图分裂。
- 人类决定：`待确认（采用有限投影+可追溯归档 / 要求永久保留并扩容 / 接受现状 / 暂缓）`
- 负责人：
- 决定理由/核心事实淘汰规则：

## HC-012：收口 Knowledge Agent 的可见域、缓存和整体 deadline

- 来源：SR-030、SR-032、SR-033
- 两轮结论：账号私有知识可在正文下钻时进入模型；内容编辑后缓存可陈旧；工具循环deadline和最终payload不一致。
- 推荐：`修复`
- 最小处理：所有 open/follow/version 接受同一 visibility scope；corpus签名含revision generation；一个 deadline 包住 LLM+工具并在返回前检查 final。
- 不过度工程边界：复用当前 Agent和进程缓存，不建平行检索链或新缓存基础设施。
- 不处理代价：跨账号内容进入模型、知识修订短期不生效、超时请求仍拖延或返回非终态。
- 人类决定：`待确认（修复 / 接受短期缓存陈旧但修隔离 / 暂缓 / 非问题）`
- 负责人：
- 决定理由：

## HC-013：把请示、领导裁决与客户转述收成一个可恢复闭环

- 来源：SR-035、SR-037、SR-038、SR-039、SR-040、SR-041、SR-054、SR-162、SR-163、SR-164
- 两轮结论：过期转述可重复裸发，首卡失败会留下幽灵 pending，timeout scanner 可重复推送，账号/domain 身份在后续阶段丢失，resolved 与 relay task 半提交；同时领导解释不应自动变成 verified 知识。配置页还有加载失败覆盖和“空即关闭”契约漂移。
- 推荐：`修复`
- 最小处理：Escalation 持久化 account/domain/policy version 和 delivery state；首卡、改派、客户转述统一走可重试 Outbox/Task；claim/finalize CAS；裁决与 relay intent 同事务或幂等提交；知识沉淀仍走 draft+needs_review；前端区分加载失败与明确关闭。
- 不过度工程边界：沿用现有 escalation/task/outbox，不新建另一套审批系统或消息总线。
- 不处理代价：领导已裁决但客户收不到、重复推卡/转述、错账号或错策略裁决、未经人审知识进入生产。
- 人类决定：`待确认（修复 / 仅修可靠投递 / 接受单账号单副本风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-014：统一 DomainProfile、DomainSchema 与运行配置的生效协议

- 来源：SR-043、SR-044、SR-056、SR-072、SR-073、SR-074、SR-089、SR-090、SR-094、SR-168
- 两轮结论：current/active 由多步写维持，激活可半提交且始终回成功；动态路径和 runtime BSON缺少强校验；AI草稿不可达人审；reset物删历史；前端编辑还会清掉终态/再激活标志。
- 推荐：`修复`
- 最小处理：每个 scope 一个可约束的 current/active 指针或 partial unique+CAS；激活返回分步骤真实结果并可重试；动态 kind/path和 runtime用共享 validator；草稿进入现有列表；reset 改显式维护动作；DTO完整投影运行时字段。
- 不过度工程边界：不建设通用配置平台；三类现有集合采用同一个小型发布模式即可。
- 不处理代价：运行时随机读取错误画像、坏配置生效、激活半成功、草稿无人可审、普通编辑破坏 Planner 语义。
- 人类决定：`待确认（修复统一协议 / 仅加约束和校验 / 接受单管理员串行风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-015：让 Taxonomy 与画像信号只写一次且映射确定

- 来源：SR-045、SR-046、SR-047、SR-061
- 两轮结论：同一未知值被两路径累计，alias可映射到多个 canonical，同轮重复证据可越过门，合并候选又不真正写 alias。
- 推荐：`修复`
- 最小处理：保留单一候选写入点或同 run 幂等；写 alias 时检测 canonical/alias冲突；每 run/dimension 去重；“合并”事务内真正追加 alias。
- 不过度工程边界：不设计通用知识图或重写贝叶斯模型。
- 不处理代价：运营看到的 occurrences、标签可信度和 canonical 映射不可靠。
- 人类决定：`待确认（修复 / 接受仅统计误差但修映射 / 暂缓 / 非问题）`
- 负责人：
- 决定理由：

## HC-016：决定 Shadow/Simulation 是否允许任何生产副作用

- 来源：SR-048
- 两轮结论：标称演练路径会写生产记忆和整改队列，且没有执行生产 finalize 硬门；这不是“纯影子”。
- 推荐：`修复为发送与业务状态零副作用；保留明确标记的观测日志`
- 最小处理：传递 `run_mode=shadow`，共享 mutation helper 在该模式拒绝业务写；允许写独立、带 shadow 标签的成本/诊断日志；模拟终态调用同一 finalize。
- 不过度工程边界：复用真实决策链，不复制一套 simulation Agent；无需隔离数据库。
- 不处理代价：运营演练会污染真实记忆、缺口队列和评估结论。
- 人类决定：`零生产业务副作用`
- 负责人：
- 决定理由/允许的副作用：当前不允许 Shadow/Simulation 写生产记忆、客户状态、知识整改队列、Outbox 或其它业务状态；仅允许写带 `run_mode=shadow` 的独立诊断、成本和评测日志。

## HC-017：定义 Evolution 的冻结基线、发布、回滚和自动放量政策

- 来源：SR-049、SR-086、SR-087、SR-088、SR-133、SR-170、SR-171、SR-180
- 两轮结论：proposal不绑定评估基线，旧 proposal可回滚掉新版本，并发 release可重复生效；父开关不能阻止 auto-release；7天指标实际只取20条；正式需求与代码对自动发布互相冲突。
- 推荐：`先决定自动发布政策，再修发布协议`
- 最小处理：proposal 保存 base version/hash和scope；release/rollback用 expected current CAS及唯一 override；父开关一票否决所有副作用；事件/review intent与发布同提交；时间窗服务端聚合。若拒绝 auto-release，删除调用和配置；若接受，正式写入需求、权限和审计。
- 不过度工程边界：不复制 Git 或实验平台；用现有 proposal/version/override 集合增加不可变身份和CAS。
- 不处理代价：旧证据应用到新基线、错误回滚、关闭后仍发布、运营无法判断审批边界。
- 分级业务规则建议：Prompt、Soul、DomainProfile、知识/安全语义和任何“放宽安全阈值”的变更始终人工发布；纯阈值“收紧安全边界”可在证据量、回归门和小流量灰度均通过后自动发布；普通业务效果阈值在未上线阶段先人工，积累生产证据后再逐项加入自动白名单。父开关关闭时任何类型都不得发布。
- 人类决定：`阶段性决定：Evolution 默认关闭；当前全部人工发布。保留按 proposal 类型+方向配置自动白名单的能力，待上线并有真实样本后再启用“安全收紧类阈值”自动发布`
- 负责人：
- 决定理由/允许自动发布的阈值范围：不同业务逻辑应分级，不能用一个全局布尔统一处理；现阶段无生产证据，不开放自动发布最稳妥且实现最简单。

## HC-018：把 Prompt、Soul 与 Playbook 简化为同一种不可变发布模型

- 来源：SR-053、SR-055、SR-069、SR-070、SR-071、SR-138、SR-139、SR-151
- 两轮结论：Soul/Prompt会物删历史或双指针漂移；Playbook前后端契约坏且AI候选可直接改生产；默认指针非原子；Prompt pack读失败可破坏性重置；语义闸看不见纯删除；风格下拉不提交。
- 推荐：`修复并统一最小发布模式`
- 最小处理：内容 append-only，唯一 active/current pointer以CAS切换；AI生成只落 draft，显式发布；reset仅显式维护且读错误fail-closed；Prompt审查传 before/after；前后端共享DTO并提交实际选择。
- 不过度工程边界：复用三张现有表和现有审核 UI；不建设新 CMS 或全局配置中心。
- 不处理代价：人格/方法论历史丢失、AI草稿直接影响生产、瞬时读错可抹配置、UI显示与运行值分离。
- 人类决定：`待确认（统一发布模式 / 仅修破坏性与契约故障 / 接受单管理员风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-019：让所有人审动作绑定对象、actor 和可重试提交

- 来源：SR-057、SR-058、SR-059、SR-060、SR-067、SR-097、SR-169
- 两轮结论：疑似成交先 approved再校验，关系审核可与已生效画像相反且永久占槽，actor可伪造，Lesson晋升可重复，统一收件箱漏成交核实，React key漂移可把A草稿提交给B。
- 推荐：`修复`
- 最小处理：审核对象用 id+expected status/version原子认领；actor来自认证上下文；业务校验先于终态；副作用与终态同事务或可重试 `applying`；ReviewQueue稳定 key并在 id变化时清草稿；成交核实加入统一收件箱。
- 不过度工程边界：不新建总审批引擎；抽一个共享 claim/actor helper并修各 handler。
- 不处理代价：错误对象被批准、成交漏登/假审计、同一建议永久不可再审、重复知识候选。
- 人类决定：`待确认（修复 / 仅修成交与对象错绑 / 接受低并发风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-020：限制 Management Agent 的真实副作用边界

- 来源：SR-062、SR-063、SR-064、SR-065、SR-143
- 两轮结论：Management可把裸发送当低风险、默认不强制危险确认、无可恢复提交协议，并能用LLM计划写 staff_confirmed 成交；切账号后还可确认旧账号命令。
- 推荐：`修复（P1）`
- 最小处理：发送只走 typed gateway/outbox；危险工具代码层默认确认；命令保存 account+plan hash+idempotency并以状态CAS执行；staff_confirmed只允许显式管理员动作；切号清 pending command并在确认时校验expected account。
- 不过度工程边界：不重写 Management Agent；只收窄 tool catalog、增加共享确认/提交 helper。
- 不处理代价：绕过发送安全与可靠投递、AI自证成交、崩溃后重复副作用、跨账号执行旧计划。
- 人类决定：`待确认（修复 / 禁用写工具仅保留只读 / 接受管理员内网风险 / 暂缓）`
- 负责人：
- 决定理由/允许的写工具：

## HC-021：使 Campaign 的预览、冻结规格和扇出提交可恢复

- 来源：SR-075、SR-076、SR-077、SR-157、SR-158
- 两轮结论：preview有生产写副作用且能重开completed活动；前端编辑不会更新spec；fanout多步写可半提交；迟到报表可显示错活动；CSV有公式注入。
- 推荐：`修复`
- 最小处理：preview纯计算或显式 draft preview，不改终态；dispatch只消费不可变 campaign spec/version；每目标用唯一 send intent并可补偿/重试；前端按 campaignId/generation接收响应；CSV中和公式前缀。
- 不过度工程边界：不引入营销平台或复杂工作流引擎；沿用 campaigns/campaign_sends/tasks。
- 不处理代价：实际推送使用旧受众、重复或漏任务、completed活动被重开、运营导出错活动或触发CSV公式。
- 人类决定：`待确认（修复 / 先修不可变spec和CSV / 接受低并发半提交 / 暂缓）`
- 负责人：
- 决定理由：

## HC-022：收口联系人导入、纳管、画像任务与列表性能

- 来源：SR-081、SR-082、SR-083、SR-084、SR-147
- 两轮结论：重复导入会清空身份；托管与画像任务半提交；旧画像任务可重新纳管已禁用联系人；系统账号可被批量纳管；列表存在N+1。
- 推荐：`修复`
- 最小处理：导入使用patch语义；任务先幂等创建或同事务切managed；画像写回带agent generation CAS；后端统一真人准入；最近消息批量聚合。
- 不过度工程边界：不新建联系人同步服务或画像队列；扩展现有Contact/Task即可。
- 不处理代价：身份数据被清空、无人画像的联系人开始自动回复、已禁用联系人复活、系统账号被误运营、列表随规模变慢。
- 人类决定：`待确认（修复 / 仅修数据与纳管安全 / 暂缓性能 / 非问题）`
- 负责人：
- 决定理由：

## HC-023：让 Guide 预览成为服务端可验证的冻结变更

- 来源：SR-091、SR-092、SR-093、SR-095、SR-150
- 两轮结论：旧preview可覆盖新配置，标签写到废弃字段，副作用范围由模型自报，commit后响应失败会诱导重试，前端迟到A预览可应用到B。
- 推荐：`修复`
- 最小处理：preview保存对象id、base versions/hash和服务端计算diff；apply校验expected contact/account和基线；写权威字段；成功返回稳定receipt；前端用generation绑定当前对象。
- 不过度工程边界：不做全库快照或通用变更管理平台；冻结直接受影响对象即可。
- 不处理代价：人类确认的内容与真正应用对象/范围不同，已提交动作被误判失败并重试。
- 人类决定：`待确认（修复 / 禁用全局Guide只保留单联系人 / 接受旧候选风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-024：选择 Chunk 编辑的权威协议并修复越权与伪回滚

- 来源：SR-096、SR-101、SR-102、SR-103、SR-104、SR-105、SR-106、SR-107、SR-108、SR-113、SR-114
- 两轮结论：通用PUT/Patch/Split/Merge可绕人审或跨租户写active+verified；多对象动作半提交；revision不是完整快照却被当回滚；前端契约错误；物理CRUD绕历史；软锁既不授权又误导。
- 推荐：`修复（P1）`
- 最小处理：所有mutation强制从服务端保留tenant/审核字段，AI来源只能draft+needs_review；revision保存可恢复snapshot或基于正向补丁重放；split/merge在事务或staging后一次commit；前端共享DTO；软锁明确advisory，若需要互斥再让写端校验token。
- 不过度工程边界：不重写Wiki或上CRDT；Mongo事务/expected version与不可变revision足够。
- 不处理代价：跨租户/绕审核知识进入生产、回滚恢复错误内容、半提交归档源、历史审计失真。
- 人类决定：`待确认（修复全部 / 先封越权与自动verify / 暂缓协作锁和历史完善 / 接受风险）`
- 负责人：
- 决定理由：

## HC-025：为外部摄取、Chat apply 与 Catalog Worker 补最小可恢复协议

- 来源：SR-109、SR-111、SR-112、SR-115、SR-117
- 两轮结论：任意摄取 URL 可形成持久 SSRF；Chat apply 与导入 Apply 可重复或半提交；Catalog/Ingest Worker 缺 lease generation，崩溃后可永久卡住或用旧结果覆盖新 checkpoint。
- 推荐：`先封 SSRF，再补现有任务的 claim/finalize`
- 最小处理：摄取 URL 统一执行 scheme、DNS/IP、重定向逐跳和响应大小限制；Chat/Import apply 以 turn/import id 原子认领并用业务唯一键提交；Catalog/Ingest 在现有行增加 owner、generation、lease、attempt，heartbeat/finalize 均以 generation CAS。
- 不过度工程边界：不引入 Kafka、通用工作流引擎或独立抓取平台；沿用现有 source/job/session/revision 集合。
- 不处理代价：内网资源可被后台抓取，知识对象重复或半提交，目录永久陈旧，旧抓取结果覆盖运营新配置。
- 人类决定：`待确认（全部修复 / 先封 SSRF 与重复写 / 接受单副本恢复风险 / 暂缓）`
- 负责人：
- 决定理由/允许的摄取网络范围：

## HC-026：把评测金标和预算变成真实评测输入

- 来源：SR-098、SR-099
- 两轮结论：active 场景缺 ground truth 时被当作零分真值；评测预算读取生产 run，空闲时恒零、并发时受污染。
- 推荐：`修复`
- 最小处理：场景激活前校验最小 ground-truth schema，缺失时标 `unscored`；评测执行在自身上下文累计 token/call 预算并返回明确 degraded 状态。
- 不过度工程边界：不建设新评测平台或统一所有业务公式；只修场景准入和本批预算归属。
- 不处理代价：评测分数和提前终止都不能代表被测行为，运营可能据此错误发布或否决配置。
- 人类决定：`待确认（修复 / 暂停公式评测 / 接受仅展示性质 / 非问题）`
- 负责人：
- 决定理由：

## HC-027：统一 Knowledge Cockpit 的对象身份、响应契约与实时收敛

- 来源：SR-129、SR-130、SR-132、SR-141、SR-173
- 两轮结论：自动校验参数命名错误而被静默忽略；审核对话未绑定当前 Chunk；默认队列类别和筛选错误；WebSocket lagged 与 SSE gave-up 都会让界面长期停在旧事实。
- 推荐：`修复`
- 最小处理：前后端共享 DTO并拒未知字段；审核请求绑定 chunkId/expectedVersion；修正队列枚举/filter；收到 lagged 就重拉对象；SSE 以连续失败计数并在 gave-up 后有界轮询任务详情。
- 不过度工程边界：保留现有 HTTP+WS+SSE，不换实时协议、不引入前端数据平台。
- 不处理代价：运营设置不生效、审核错对象、归档数据混入队列、长任务已经完成但页面永久显示旧状态。
- 人类决定：`待确认（修复 / 先修对象身份与参数 / 暂缓实时体验 / 接受风险）`
- 实施状态：`部分实现`。SR-173 为 `working-tree-wired / deterministic-verified / deployment-pending`（2026-07-19）；SR-129、SR-130、SR-132、SR-141 仍需分别修复和验证，不能因实时收敛子项完成而关闭 HC-027。
- SR-173 实施：共享 SSE 重连器在原生 `open` 或业务事件后重置连续失败预算，隔离旧 EventSource 迟到事件；TaskRail 显示重连/放弃状态，每个 turn 回读权威任务详情，gave-up 或无 EventSource 时执行 5 秒一次、最多 12 次的有限轮询，终态、切任务与卸载均立即停止。
- SR-173 验证：状态机与 TaskRail 专项 13/13、Knowledge 相关回归 18/18、前端生产构建和定向差异门通过；覆盖成功重连不累计断线、连续建连失败才放弃、terminal close 零重连、turn 更新主进度、轮询收敛终态及上限耗尽后停止。当前未部署工作树版本，部署后真实浏览器/代理断流回归仍待执行。
- 负责人：
- 决定理由：

## HC-028：让 Digest 与 Knowledge Task 只报告真实、可恢复的结果

- 来源：SR-120、SR-121、SR-122、SR-123、SR-125
- 两轮结论：Digest 预算配置不生效且失败重算会覆盖成功卡；Knowledge Task 无 lease/幂等 step并把确定失败包装成成功；派工也未绑定运营选中卡片。
- 推荐：`修复`
- 最小处理：Digest 使用现有 RunBudget配置，先生成新版本成功后再切 current；Task 加 owner/generation/lease，step按 task+step id 幂等并返回结构化 outcome；创建任务必须验证 cardId/version/hash。
- 不过度工程边界：复用现有 Digest/Task/Card 集合，不新建任务平台；不要求所有 LLM 质量问题机械判定。
- 不处理代价：成功日报被空卡覆盖、任务永久 running或重复改库、失败被显示为成功、模型可派发非运营所选对象。
- 人类决定：`待确认（修复 / 先修虚假成功和数据覆盖 / 接受人工重跑 / 暂缓）`
- 负责人：
- 决定理由：

## HC-029：给主动触达与 ImportJob 复用同一最小所有权模式

- 来源：SR-135、SR-136
- 两轮结论：Planner 没有持久业务 intent，串行或并发 tick 可重复触达并突破每日配额；ImportJob 重领后旧 worker仍可续租和覆盖新结果。
- 推荐：`在启用这些 Worker 前修复；未启用可暂缓但保留上线前门`
- 最小处理：每次主动触达建立稳定业务幂等键，数据库日桶原子预留配额；ImportJob claim产生 generation，heartbeat/finalize必须匹配 generation。
- 不过度工程边界：不建设中央调度系统；沿用现有 Planner intent/AgentTask/ImportJob 数据面。
- 不处理代价：客户被重复主动触达、声明配额失效，旧导入执行覆盖新执行事实。
- 人类决定：`待确认（修复后启用 / 当前关闭并延期 / 接受单副本风险 / 需补开关事实）`
- 负责人：
- 决定理由/当前开关与副本数：

## HC-030：用一个前端身份守卫消除切账号后的跨对象写错

- 来源：SR-142、SR-144、SR-145、SR-146、SR-148、SR-149、SR-153、SR-155、SR-159、SR-160、SR-161
- 两轮结论：联系人、密钥、Playbook、标签、运营池、任务、Outbox、素材和成交等状态都可能在切号后保留 A 对象，并通过当前 B 页面发出合法请求；扫码登录另有确定字段漂移。
- 推荐：`修复为共享模式，并优先处理密钥、成交、素材和Outbox`
- 最小处理：所有账号私有 query key/state包含 accountId；切号清 selection/draft/secret/pending，异步响应提交前校验 account+object generation；动作带 expected account，后端复核；扫码登录改用共享 DTO。
- 不过度工程边界：不重写全部前端 Store；提供一个 account-scoped request/state helper，再逐页迁移这 11 个已证实入口。
- 不处理代价：管理员在 B 上下文修改 A 的密钥、生产方法论、标签、素材、任务、待发消息或高可信成交。
- 人类决定：`待确认（全部修复 / 先修高危写动作 / 接受单管理员低切号频率 / 暂缓）`
- 负责人：
- 决定理由/账号切换是否为正式工作流：

## HC-031：把 LLM Provider 编辑和视觉指派当作生产发布

- 来源：SR-165、SR-166、SR-167
- 两轮结论：编辑 active provider 会未经确认/连通测试立即热切全进程；清空超时/重试不能恢复默认；视觉指派可在关闭能力或删除时静默失效。
- 推荐：`修复`
- 最小处理：active行变更先测试成功并确认，再原子替换或通过草稿激活；nullable PATCH区分清除与未修改并回显effective来源；维护 `visionActive => supportsVision`，删除/关闭前显式清除或改派。
- 不过度工程边界：不建设供应商发布平台；复用现有 test、activate 与 registry swap，增加一个受控提交步骤即可。
- 不处理代价：一次普通保存可让全进程 LLM 故障或切到错误端点，视觉导入能力可无提示消失。
- 人类决定：`待确认（修复 / 禁止编辑active仅允许新建激活 / 接受内网管理员风险 / 暂缓）`
- 负责人：
- 决定理由：

## HC-032：把测试门改成“目标行为真的发生”，而非扩大测试数量

- 来源：SR-004、SR-126、SR-128、SR-174、SR-176、SR-178
- 两轮结论：全量 Docker job soft-fail；部分知识/租户测试重写生产逻辑；真模型红线可零产物绿色；进程级 cache使独立测试库串扰。它们证明的是证据不可信，不直接证明生产一定故障。
- 推荐：`修复少数上线红线门，保留长尾发现线`
- 最小处理：挑选发送、租户、知识审核、worker恢复等关键 case走真实 Router/Handler/Worker并 hard gate；每个动态 case输出 attempted/artifact/assertions/verdict/skip；cache按AppState或database identity隔离。其余慢测继续 soft/nightly。
- 不过度工程边界：不把所有 ignored/真模型测试改成 PR 硬门，不追求零波动；只要求关键红线有正向见证和明确 inconclusive。
- 不处理代价：CI绿色仍不能说明最关键能力被执行，生产回归可能长期藏在“零样本通过”后。
- 人类决定：`按推荐分层`（2026-07-19；用户在审阅“先收口 SR-128/178、再固化 SR-004 分层、随后远程验证”的计划后明确指示“执行”）。
- 实施状态：`工作树分层与动态证据协议已接线 / 本地确定性验证完成 / Actions与部署待证`。发送/Reaction T4 专项严格门已接线、远程运行待证；SR-174 为 `working-tree-wired / deterministic-verified / local-real-mongo-verified / deployment-pending`；SR-176 为 `working-tree-wired / deterministic-verified / local-real-router-mongo-verified / hard-gate-wired / actions-run-pending / deployment-pending`；SR-126 为 `working-tree-wired / deterministic-verified / hard-gate-wired / local-real-mongo-blocked / actions-run-pending / deployment-pending`；SR-004 为 `working-tree-policy-wired / deterministic-verified / actions-run-pending`；SR-128/SR-178 为 `working-tree-wired / deterministic-verified / hard-gate-wired / real-model-run-pending / deployment-pending`（2026-07-19）。各专用门的 Actions/部署证据仍缺失，不能关闭 HC-032。
- SR-004/T4 证据：手动 `smoke_t4` job 不使用 `continue-on-error`，先要求四条 `delivery_redline_*` 名称覆盖恰好为 4 并全部执行，再运行一条必须产出两轮回复、Reviewer 审计、Reaction 停止识别、Outbox 取消和单次 MCP 发送的真实模型任务。完整 transcript 在断言前打印并上传，模型错误不再 transient-skip；远程运行仍待证。
- SR-174 实施：`Database::connect` 分配 clone 共享、独立连接不同的进程内 identity；Taxonomy 与 DomainProfile cache按 identity 分片，写后失效绑定当前数据库，registry 和 taxonomy 初始化 guard 通过 `Weak` 生命周期回收已释放测试库状态。现有 TTL与纯查表 API保持不变，不以测试串行化代替隔离。
- SR-174 验证：registry 单测 1/1、`cargo check --lib`、格式与差异门通过；真实 Mongo红线以两个随机数据库、相同 workspace和不同 profile/taxonomy，在双库都预热后反向交错读取；生命周期 guard 修正后的最终回归 1/1 通过（55.76 秒），测试库零残留。当前未部署，且本证据不结算 SR-004/126/128/178；SR-176 由下述独立批次推进。
- SR-176 实施：新增真实 Cookie + session middleware + `api_router` 红线，代表性覆盖 Contact 跨租户读、Product 跨租户写后不变/本租户正向更新、真实过期 Cookie 401；旧手抄 Mongo filter 用例明确降级为 collection/index 局部约束，旧 session 用例改为直接插入过期 `AdminSession`。没有为每个 CRUD 复制全矩阵。
- SR-176 验证与门禁：真实 Router 1/1（23.22 秒）、真实过期 session 1/1（31.01 秒）、专项类型/格式/差异门通过，随机测试库零残留。CI 新增无 `continue-on-error` 的 `tenant-isolation-security` hard gate，机械断言两条测试各恰好存在一次后只运行这两条；本机缺少 YAML parser，工作树也未推送，因此 Actions YAML/Ubuntu/testcontainers 真绿与部署后证据仍待补，不能按 fully-verified 结算。
- SR-126 实施：召回评测由生产 catalog 首项驱动 open/cite，金标只用于事后 recall@1；真实维护闭环经过 Chat Handler 起草、apply 草稿、运营 verify revision 与 Knowledge Agent grounded citation；Worker 单步结果改为 `committed|noop|needs_manual|failed`，失败原因持久化，任务 summary 分桶不再把 Rust `Ok` 冒充业务成功。
- SR-126 验证与门禁：三个专用测试二进制联合类型检查通过，Worker 确定性契约 6/6、定向格式与差异门通过；本机真实测试在 testcontainers 创建 Mongo 前因无 Docker socket 且无外部测试 URI 退出，业务断言未运行，不记为绿色。CI 新增无 `continue-on-error` 的 `knowledge-evidence-gate`，机械断言五条目标行为各恰好存在一次后按 exact name 运行；Actions、真实 Mongo 和部署后证据待补。
- SR-128 实施与门禁：共享 typed outcome协议接入 K2/K3/K6/K7/K10、Q3/Q4、T3 和三条 Recall闭环共 11 个关键 case；每个 case必须有真实调用、明确分支、非空产物和已执行断言。统一汇总器拒绝缺失、重复、零调用、零产物、零断言、非 pass、schema/SHA/run/attempt 不匹配；producer恢复缓存后先清空 ledger，旧证据不能冒充本轮结果。
- SR-178 实施与门禁：Cross-domain、Principal、Proactive、Dynamic、Digital Twin 与 Roleplay 共 11 个 redline case复用同一协议。全 fallback、零回复/任务/请示/转述、transient和 helper early-return不再正常退出为绿；Principal Channel要求真实请示或明确安全 fail-closed，Relay要求 resolved→task→非空 AI转述并保留真实调用数。nightly redline矩阵扩为 7 个串行 shard，Roleplay Arc不再只存在于手动入口。
- SR-004 分层防漂移：新增 `scripts/check-ci-gate-policy.py` 并接入 baseline。机器契约要求 7 个关键门保持 hard、6 个长尾/波动 job保持 soft，五类波动真模型套件仅 nightly；同时锁定 redline→skip-gate 依赖及22-case汇总器接线。不把全量 ignored/真模型测试改成 PR硬门。
- SR-004/128/178 本地验证：七个 redline crate联合类型检查通过；22-case manifest和11个 redline witness映射通过；checker完整/缺失/陈旧 run三态通过；格式、定向差异、PyYAML、CI结构及 gate policy `hard=7 soft=6 failures=0` 通过。没有实际调用外部真模型、推送分支、运行 Actions或部署测试服务器，因此这些结果不得写成 fully verified。
- 负责人：
- 决定理由/允许的PR时长预算：

## HC-033：修正交付与审计账本，不把文档治理做成平台

- 来源：SR-006、SR-179、SR-183
- 两轮结论：架构文档过期；任务表把 implemented/wired/verified混为完成；47域审计允许空证伪和静默漏域却被称事实底座。
- 推荐：`修复治理记录，但不阻塞运行时P0/P1修复`
- 最小处理：README/架构标当前commit和默认值；任务使用有限状态并绑定生产入口/测试artifact；AI审计输出强制 `complete|inconclusive|failed`、证据字段、输入/模型/hash manifest和机器汇总。
- 不过度工程边界：不采购需求平台、不建设通用审计数据库；Markdown/JSON schema加CI校验足够。
- 不处理代价：维护者会继续把未接线能力和不完整AI审阅当作已验证上线事实，错误安排修复与发布。
- 人类决定：`待确认（修复 / 只修高风险任务状态 / 接受历史文档不权威 / 暂缓）`
- 负责人：
- 决定理由：

## HC-034：阻止文档编辑在详情未加载时清空原文

- 来源：SR-131
- 两轮结论：前端把 `{item}` 包络当扁平对象，正式保存可向 undefined id发请求并准备以空值覆盖原文，是独立、稳定、低成本的真实故障。
- 推荐：`立即修复`
- 最小处理：正确解包响应；id/rawContent未加载或对象不匹配时禁用保存；使用dirty PATCH并加一个组件/契约测试。
- 不过度工程边界：不并入全前端重构，不改后端文档模型。
- 不处理代价：运营正常编辑即可失败或清空知识原文。
- 人类决定：`修复（已实现）`
- 实施状态：`已完成（2026-07-18）`
- 负责人：Kiro
- 决定理由：沿用后端现有整替换 PUT，不扩展新 PATCH 协议；前端按真实 `{item}` 包络解包，只有详情 id 与当前列表对象一致时才开放编辑，并原样回带 `rawContent/contentHash/lineIndex/sectionIndex` 等不可丢字段。
- 验证：`DocumentEdit.test.tsx` 使用真实响应包络并通过；前端生产构建通过。

## HC-035：修正知识诊断页的 Catalog 响应解包

- 来源：SR-118
- 两轮结论：两个 Catalog 响应包络被误读，有真实数据时诊断页仍稳定显示0/0；这是独立小修，不需要业务重构。
- 推荐：`立即修复`
- 最小处理：按后端真实响应解包并由共享类型/fixture锁定，空数据与有数据各测一例。
- 不过度工程边界：不重写诊断统计或 Catalog 后端。
- 不处理代价：运营误以为知识库为空或覆盖为零，做出错误维护判断。
- 人类决定：`修复（已实现）`
- 实施状态：`已完成（2026-07-18）`
- 负责人：Kiro
- 决定理由：不改 Catalog 后端契约；前端按 `{documents:[...]}` 与 `{item:{documents,...}}` 两个真实包络分别计数，持久化数只统计已有 `catalogSummaryPersisted` 的文档。
- 验证：新增 `ObservabilityDashboard.test.tsx`，真实包络下断言持久化 1、实时 2、偏差 +1；前端生产构建通过。

## HC-036：让 Knowledge Ask 的失败成为可见终态

- 来源：SR-127
- 两轮结论：Agent失败被编码为普通trace+close，正式界面表现为无答案、无错误地结束；这是协议错误，不是模型质量问题。
- 推荐：`立即修复`
- 最小处理：SSE发送 typed terminal `completed|failed|cancelled`及可显示错误码，前端在未收到completed时不得把close当成功。
- 不过度工程边界：不更换SSE、不暴露内部堆栈、不重写Knowledge Agent。
- 不处理代价：真实故障被用户理解为系统没有回答，无法区分重试、配置或知识问题。
- 人类决定：`修复（已实现）`
- 实施状态：`已完成（2026-07-18）`
- 负责人：Kiro
- 决定理由：保留现有 SSE；新增独立 `TraceEvent::Failed` / `event:failed` 终态，只在整条 Agent 执行返回错误时触发。轮内可恢复的 `tool:error` 仍是普通 trace；详细错误只写服务端日志，前端仅展示稳定错误码与通用文案。正常 answer、连接异常、主动取消和无结果 close 分别处理。
- 验证：`exploreNoTenant.test.tsx` 覆盖 failed 终态并通过；`cargo check --lib` 和 `cargo test --test knowledge_ask_stream_e2e --no-run` 通过；前端生产构建通过。
