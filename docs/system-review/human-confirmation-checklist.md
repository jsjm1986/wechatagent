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
- 两轮结论：凭证进入公开 Git 历史/普通 workflow input 的暴露事实成立；2026-07-30 外部核验进一步确认该历史 LLM 凭证仍有效且正式服务仍在使用。三项共用同一个密钥生命周期决策。
- 推荐：`修复（立即）`
- 最小处理：先在上游轮换/撤销，再改 GitHub Secret 或部署环境注入，最后审计历史 run、fork、artifact、服务器副本并按必要性清理历史。
- 不过度工程边界：不以建设新 secret 平台作为轮换前置；现有 GitHub/environment secret 足够完成首轮闭环。
- 不处理代价：仍有效的 key 可被仓库/历史读取者使用；只删当前文件不能消除历史暴露。
- 已确认外部事实：仓库为公开仓库；GitHub API 当前可复现两个仍可达的历史提交树包含同一非占位 LLM 凭证，先前记录的第三个短哈希当前无法解析并已排除出可复现计数；最小合成鉴权请求仍被上游接受。正式服务器当前 102,259 个普通文件中有 10 个精确副本；正式克隆的 5,117 个 Git blob 中另有 29 个命中（249 次）；25 个压缩载体中有 17 个命中（38 次，其中 16 个数据库归档、1 个含 6 个环境文件成员的配置包）。服务器 `/opt` 的 101 个 Git 仓库中只有 `/opt/wechatagent` 为同源克隆。807/807 个当前可枚举 Actions run 日志完成精确扫描，其中 69 个 CI run 在 2026-06-02 至 06-08 合计原值命中 1,795 次；313/313 个当前可下载 artifact 零命中，另有 1,933 个已过期 artifact 无法下载；当前 fork 为 0。未发现项目容器镜像构建/发布链，但 GitHub Packages 元数据因令牌权限不足不可证实。GitHub Secret Scanning 当前未启用。
- 人类决定：`修复（按已确认的有效凭证泄露处理：立即轮换/撤销，再收口服务器副本、Git 历史与 Actions 工件）`
- 实施状态：`安全事件及 Actions 日志泄露已确认 / 当前受跟踪树旧值零命中 / 高置信 secret CI 硬门已接线 / 轮换、GitHub Secret 同步与日志清理工具已验证 / 生产轮换、撤销及远端日志删除未执行（2026-07-30）`
- 负责人：实现：Kiro；历史与远端审计：待运维确认
- 决定理由/边界/到期日：有效密钥已在公开历史暴露，必须按已泄露凭证处理；删除当前文件不能撤销访问能力。轮换是生产认证变更，须使用候选凭证预检、原子替换、健康/模型回归和失败回滚协议。
- 已完成：删除受跟踪的 `.env.e2e` 与明文探测 Workflow；当前受跟踪树对已知旧值精确扫描为零；新增 `scripts/check-secrets.py` 高置信扫描器并接入 baseline；新增经定向测试的生产原子轮换器、GitHub Secret stdin 同步器、冻结证据绑定的 Actions 日志清理器、无 mutation 模式的服务器载体预检器和运行手册。Actions 审计已完整扫描 807 个当前可枚举 run 日志和 313 个当前可下载 artifact；清理器真实只读预检确认 69/69 个目标，尚未调用删除接口。服务器载体预检器本地联合门 28 项中 26 通过、2 项仅在 Windows 跳过 POSIX 权限语义，Linux 专项 8/8；正式只读预检完整扫描 102,259 个普通文件、14,448 个 Git 对象和 25 个压缩载体，准确返回普通文件、Git 对象、压缩载体三项 blocker，扫描无错误且生产 PID、重启数和健康零漂移。历史日志根因已定位为 Workflow 的明文字面量 fallback，当前所有凭证型 Workflow 赋值均为直接 `${{ secrets.* }}` 或显式空值，并由 `workflow-secret-must-be-direct` CI 硬门防回归。
- 未完成硬门：由上游生成并安全暂存替代凭证；执行生产只读预检与经批准的原子切换；同步 `RSXERMU_KEY` 并用最小 Actions 真实模型任务验证；撤销旧凭证并双向证明；按普通文件、压缩备份、Git blob 三类经授权收口服务器载体；经明确授权删除并复验 69 个泄露日志；处理无法下载的过期 artifact 与不可枚举的 GitHub Packages 元数据边界；决定是否改写公开 Git 历史；启用仓库 Secret Scanning、push protection 和 main 保护。详见 [HC-001 轮换手册](hc001-credential-rotation-runbook.md)与[安全事件记录](security-incident-hc001-2026-07-30.md)。

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
- 人类决定：`修复；ops 三表保留版本历史，但每个逻辑 scope 只允许一个 current，不提供隐式多-current A/B`
- 实施状态：`SR-007/009/010 已修复并验证；SR-008 已正式部署、真实副本集与部署后观察通过（2026-07-25）`
- 负责人：实现：Kiro；产品语义：用户已确认单-current契约
- 决定理由/存量库范围：m009 与 m034 统一为 active 优先、同 scope 重跑收敛；m029 全程按 `(workspace_id, account_id, wxid)` 处理并跳过归属未知行；m035/m036 对生产破坏性清理与归属回填使用精确 `APPROVED_MIGRATIONS` 审批。SR-008 由 m048 在索引前全表验证并确定性收敛零/多 current，合法唯一 current 保持不动；三表版本历史保留，运行时不再把损坏指针解释为灰度或默认回落。
- SR-008 验证：既有格式、编译、m048 单测 2/2，以及授权服务器 `rs0` 隔离库的迁移/唯一索引/fail-closed 与三类 publish/rollback/rollout 动态用例均通过。2026-07-25 用户明确确认生产切换；后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf` 上线，12/12 次内外网健康、PID 无重启、Mongo 主节点、开放 Outbox=0、近期失败任务=0 和日志门均通过。切换点备份及回滚材料保留，详见 [生产发布证据](production-release-2026-07-25.md)。该结论只关闭 SR-008 的部署门，不自动关闭其它动态验证待办。

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
- SR-024/025/026/027 历史专项证据（2026-07-29）：`已正式部署 / 部署后同源真实 Webhook、Dispatcher、MCP、Reaction 全链 + rs0 矩阵 4/4 / 零测试库与进程残留`。SR-024 验证同名账号跨 workspace 限流桶和事件归属隔离；SR-025 当时只证明外域发送历史不阻塞本域发送且 `message_send_text` 恰一次；SR-026 验证跨 workspace 幂等身份可并存、同 workspace 重复仍 skip；SR-027 验证 stop 只取消同 workspace/account Outbox。首轮 3/4 红灯被完整保留：测试把 MCP `initialize` 握手和一次 `tools/call` 合计的两个 HTTP 请求误判成两次发送；仅修正测试 oracle 为统计 `tools/call:message_send_text`，生产代码未改，重编译后 4/4 通过。所有 finalizer RC 为 0，数据库/测试进程零残留，正式 ELF `efe5e1e1…baf4`、PID `2264975`、`NRestarts=0`、运行态前后不变；可再生产物已精确清理。证据归档 SHA-256 `edd2d3f6…04e3`（[归档](../../audit/reconciliation/hc004-sr024027-evidence-20260729T123000Z.tar.gz)）。后续专门红线证明该批证据不足以关闭 SR-025 的“日软上限告警计数”租户作用域；SR-024/026/027 的结论不受影响。
- SR-025 补充闭环（2026-07-30）：`旧实现 expected-red / 修复后 green 4/4 / 候选回环-only smoke / 正式原子切换 / 部署后精确回归 1/1 / 零测试库与进程残留`。旧实现会把外域同 `account_id` 的 sent 历史计入本域日软上限并错误发出本域告警；修复将 `account_daily_sent_count` 查询收紧为 `(workspace_id, account_id)`，不改变“软上限只告警、不作为发送硬门”的产品语义。正式 ELF 从 `efe5e1e1…baf4` 切换为 `d0b7ffc6…e027b`，PID `2410064`、`NRestarts=0`，内外健康、69 项静态资源、生产数据库基线与活跃队列均通过。部署后同源测试 ELF `8d336638…e5cb36` 在随机隔离库精确运行 SR-025 用例 `1 passed; 0 failed`，四项 finalizer RC 均为 0，生产 PID/ELF/健康前后逐字一致。旧 ELF 与 `39,593,713` 字节全库压缩备份保留；详见 [SR-025 发布记录](production-release-2026-07-30-hc004-sr025.md)。该结论关闭 SR-025 的剩余软上限作用域缺口；HC-004 因其它来源项继续开放。
- Campaign 账号归属闭环（2026-07-23）：创建草稿在任何写入前用 `(admin.current_workspace, resolved_account_id)` 校验账号；兼容省略 `accountId` 的旧客户端，但显式账号和默认回退都不能借用其它 workspace 的同名账号。真实 handler + 本机独立 MongoDB 8.0 随机库红线 1/1 通过：错误租户账号返回 `NotFound` 且 `campaigns` 零写；测试库由 `TestApp::cleanup` 删除，独立 mongod 已关闭，系统 MongoDB 服务保持原 `Stopped`。
- SR-110/137 的 m039 子项状态（2026-07-28）：`已正式部署 / 真实 rs0 红线 3/3 / 生产历史索引复用与部署后复验通过`。m039 先完整规划 Revision 与 Signal 两张表，脏类型、缺父 Chunk、workspace 不一致及零/多账号歧义均在首写前 fail-closed；显式 Signal 账号作为历史身份，不错误依赖可能已被 m029 删除的当前 Contact。正式库 94 条 Revision 与 3,206 条 Signal scope 完整，重复 scoped dedupe 组为 0；启动按完整选项语义复用 scoped key 的历史索引名，同 key 非等价选项仍拒绝。正式 ELF `f0ead4f7…cde9b`、PID `2196929`、`NRestarts=0`，部署后归档 `5f5f53e9…9fd9`。详见 [HC-026 / m039 发布记录](production-release-2026-07-28-hc026-m039.md)。
- SR-116/119/152 专项闭环（2026-07-28）：`已正式部署 / 候选与部署后同源 rs0 红线各 4/4 / 零测试库与进程残留`。SR-116 成交追认按 `(account_id, contact_wxid)` 聚合并以 workspace+account+wxid 读取 Contact；SR-119 的定时 helper 真实枚举全部持久账号、逐账号失败隔离，Chunk health 可见域为当前账号私有行加 `account_id=null` 的 workspace 共享行，Prompt/LLM 审计/报告均继承真实 workspace/account；SR-152 列表和详情强制显式 `accountId`，错误账号拒绝且 Review/RunLog 全链按 workspace+account 收口。候选和部署后矩阵均覆盖成交归因、共享 Chunk、真实定时账号枚举和复盘 Router；正式 ELF `efe5e1e1…baf4`、PID `2264975`、`NRestarts=0`，数据库关键基线与 69 项静态资源不变。Digest worker 继续遵守现有 `KNOWLEDGE_DIGEST_ENABLED=false`，本次部署未擅自启用调度。回滚保留旧 ELF `f0ead4f7…cde9b` 与 39,602,771 字节数据库备份；部署后证据归档 SHA-256 `86ce5ad5…cbb6`（[归档](../../audit/reconciliation/hc004-sr119-postdeploy-evidence-20260729T000500Z.tar.gz)）。该结论只关闭 SR-116/119/152，不代表 HC-004 其它租户项已完成，HC-004 继续开放。
- SR-036/124 专项闭环（2026-07-29）：`已正式部署 / 部署后同源 rs0 红线 1/1 + 扩展矩阵 6/6 / 零测试库与进程残留`。SR-036 的 Outcome task 唯一键与迁移去重均含 workspace；部署后同 account/content 跨 workspace 可并存、同 workspace 重复由 Mongo unique key 拒绝。SR-124 的 cardId 与直接/Worker dismiss 均绑定 account；同一 workspace 两账号复用同 cardId 时，真实 Cookie Router 直接忽略和 fenced Worker 事务忽略都只修改目标账号，非目标日报完整 BSON 不变。所有 finalizer RC 为 0，正式 ELF `efe5e1e1…baf4`、PID `2264975`、`NRestarts=0`、健康与运行态前后不变。联合证据归档 SHA-256 `784e141c…2735`（[归档](../../audit/reconciliation/hc004-sr036-sr124-evidence-20260729T181500Z.tar.gz)）。该结论关闭 SR-036/124；HC-004 因其它来源项继续开放。
- SR-080 专项闭环（2026-07-29）：`已正式部署 / 部署后同源真实 Cookie Router + rs0 矩阵 7/7 / 零测试库与进程残留`。红线在两个 workspace 复用同一 `account_id`，并故意令外域账号 self-wxid 等于目标联系人、本域账号 self-wxid 不同；本域启用成功且只修改本域 Contact/事件，外域账号完整 BSON与事件数不变，证明账号注册与 self 判断未借用外域同名账号。所有 finalizer RC 为 0，正式 ELF `efe5e1e1…baf4`、PID `2264975`、`NRestarts=0`、健康与运行态前后不变；可再生测试产物已精确清理。证据归档 SHA-256 `c5c16d0e…cb4b`（[归档](../../audit/reconciliation/hc004-sr080-evidence-20260729T190000Z.tar.gz)）。该结论关闭 SR-080；HC-004 因其它来源项继续开放。
- SR-011 实施状态（2026-07-21）：`工作树已修复 / Rust 类型、格式、diff 与调用形状检查通过 / 真实 MCP 全链与部署后专项证据待办`。所有账号型 MCP API 强制接收 workspace+account，凭证查询和 `McpCallLog` 使用相同 scope；非默认 workspace 缺显式账号 URL/key 时 fail closed，禁止部署级默认凭证回退。文本、媒体、名片、Escalation、Management、联系人、登录、roster、`chat_search` 和 post-hoc 送达核对调用点均已迁移；共享媒体按实际发送账号上传，私有媒体强制账号一致，媒体成功日志按 workspace 过滤。SR-012 的服务器证据只覆盖 debounce/reload 与 roster single-flight，不外推关闭本项；仍需用两个 workspace 的独立 MCP 凭证验证调用目标、日志及 timeout/reclaim 不串租户。
- SR-012 专项闭环（2026-07-30）：`已进入正式 ELF / 部署同源 release 编译 / 服务器真实 Mongo 动态 2/2 / 生产零扰动 / 构建残留精确清理`。在两个 workspace 复用同一 account/wxid 的场景下，真实 debounce runner 只重载并修改目标 workspace 的 Contact，外域 Contact 完整 BSON 不变；roster single-flight 对两个 workspace 分别取得独立 permit，不再因同名 account 互相阻塞。测试结果为 `2 passed; 0 failed`（14.43 秒），随机测试库 126→126、无测试进程残留；生产 PID `2410064`、`NRestarts=0`，磁盘与运行中 ELF SHA-256 均为 `d0b7ffc6…e027b`，健康前后不变。共享 Cargo target 的 5,584 项基线恢复通过，超时编译遗留的变异 target 与安全备份已精确清理，根盘可用空间恢复至约 4.63 GB。最小证据归档为 [hc004-sr012-final-evidence-20260730.tar.gz](../../audit/reconciliation/hc004-sr012-final-evidence-20260730.tar.gz)，SHA-256 `3a0b48cbdae214d5b8cd2523353979ff761d04d453a15b25f63496b70b08475a`；该结论只关闭 SR-012，SR-011 与 HC-004 其它来源项继续开放。
- SR-013 实施状态（2026-07-21）：`工作树已修复 / Rust 类型、格式、diff 与生产调用形状检查通过 / 动态、Mongo、真实模型与部署待办`。`LlmRegistry` 已按 workspace 隔离 client、provider metadata 与 generation；启动加载所有 workspace 的 active provider，同 workspace 多 active 直接拒绝启动。公共 Agent、Knowledge Agent/Catalog、视觉 Runtime 与 Evolution Prompt Critic 均以真实 workspace 获取不可变 snapshot，生产缺槽位 fail closed；源码中剩余 `state.llm` 只位于 `llm_registry=None` 的测试 mock 回退。管理端 runtime meta、active 编辑、激活与删除均绑定目标 workspace；单实例内用同一变更锁串行化，先构造 client，再写 DB，确认 `matched_count/deleted_count` 后才替换 runtime，激活中途失败恢复旧 active 标记。`cargo check --tests`、`cargo fmt --all -- --check`、`git diff --check` 通过；定向 Registry 隔离测试完成 Rust 编译后在链接阶段被本机 `ring 0.17.14` 的 84 个原生符号缺失阻断，断言未执行，不能记为动态通过。服务器复验需覆盖两个 workspace 使用不同 mock/真实 endpoint、分别热切换后 generation 与调用目标互不影响、重启装载一致，以及 Mongo 写失败/目标消失时 DB 与 runtime 不分叉。
- SR-134 实施状态（2026-07-21）：`工作树已修复 / Rust 类型、格式、diff 与 scope 调用形状检查通过 / 动态、Mongo与服务器待办`。共享账号 scope 枚举从 `wechat_accounts` 返回稳定排序、去重且拒绝空标识的 `(workspace_id,account_id)`；Planner 每轮动态枚举全部注册账号并对六个扫描段逐 scope 失败隔离，Cold/Silence 按注册账号投影后的 workspace 扫描，审计使用该 workspace 的真实代表账号，候选、任务和信号仍使用联系人自身账号。三个 worker 已无运行时 `default_workspace_id/default_account_id` 读取；`cargo check --tests`、`cargo fmt --all -- --check`、`git diff --check` 通过。服务器复验需用两个 workspace、同名 account/contact 与不同阈值/知识配置，确认各自产生且只产生本租户任务、信号、事件，并验证一个 scope 注入失败不阻断另一个 scope。
- SR-085 实施状态（2026-07-21）：`工作树已修复 / Rust 类型、格式、diff 与静态 scope 验证通过 / 动态、Mongo与服务器待办`。Evolution worker 每轮动态枚举全部注册账号，逐 `(workspace_id,account_id)` 隔离失败与预算；实验 ID 改为全局唯一 ObjectId 型字符串，但信封、候选、Threshold、Prompt Critic、Replay、Prompt Shadow、显著性、到期复评、自动发布和事件仍显式校验完整 scope。管理 API 展示当前 workspace 的全部账号并在响应保留 `accountId`；发布/回滚的内部重读、override/prompt/proposal 事务更新均使用 scope 与预期状态 CAS，零命中在 commit 前中止；Evolution 关联索引改为 scope 前缀复合键。`cargo check --tests`、`cargo fmt --all -- --check`、`git diff --check` 通过；生产 Evolution 目录已无运行时默认 scope 读取。两个新增纯单测的执行在本机 Windows 测试二进制链接阶段 244 秒超时，断言未启动且遗留进程已定向清理，不能记为动态通过。服务器复验需以两个 workspace 复用同名 account/contact，分别配置 runtime flag、Prompt/LLM 与阈值，确认实验/候选/replay/release/audit 全链不串租户，并覆盖错误 scope 发布、并发 CAS 失败及单 scope 故障隔离。
- SR-018/021/080 实施状态（2026-07-21）：`工作树已修复 / Rust 类型、格式、diff 与静态 scope 检查通过 / 动态、Mongo、真实模型与部署待办`。SR-018 的公共 LLM 非流式/流式入口已强制 workspace，cache hit、success、failure 日志写真实租户；SR-021 的 Reply 三层 prompt、published Soul、Reviewer、Reaction 与 Memory consolidator 均读取 contact/run workspace；SR-080 的单联系人启用账号查询改为 workspace+account，self-wxid 判断不再借用其它租户同名账号。新增账号 filter 纯函数断言；需在服务器以两个 workspace 复用相同 account/contact 标识，分别发布不同 Prompt/Soul，并验证 LLM 日志、决策/审核/Reaction/Memory 产物与 enable/self-account 结果不串租户。

## HC-005：收紧认证撤权、appId 唯一性与登录频控

- 来源：SR-014、SR-015、SR-016
- 两轮结论：ACL 收缩不会即时影响既有 session/JWT，appId 被当唯一键却无唯一约束，登录/token 无应用级频控；外围 WAF 是否补偿未知。
- 推荐：`修复`
- 最小处理：敏感请求重读 ACL或权限版本；appId 存量查重后建 partial unique；login/token 共用轻量 IP+账号限流与失败审计。
- 不过度工程边界：短 JWT TTL/权限版本足够，不先建设完整撤销中心或复杂反欺诈系统。
- 不处理代价：已撤权用户仍有窗口，重复 appId 可错路由，公网登录可被猜密/消耗 CPU。
- 已确认外部事实：当前无 WAF/反向代理限流；项目尚未生产上线，仍在测试优化阶段。
- 人类决定：`分阶段修复：ACL 撤权与 appId 唯一性纳入当前修复；登录/token 轻量限流作为公网生产上线前硬门，当前不阻塞测试优化`
- 实施状态：`SR-014/015/016 已完成；SR-016 已进入正式 ELF 并完成部署后随机库动态闭环（2026-07-28）`
- 负责人：实现、服务器隔离复验与部署后回归：Kiro
- 决定理由：现阶段没有公网生产流量，不需要提前建设复杂风控；但上线前不能依赖尚不存在的外围补偿。
- 已完成：Cookie 与 Bearer JWT 每次受保护请求重读 `AdminUser` ACL；用户删除、workspace 移除或清空最后权限后立即 401。历史空 ACL 由 m037 一次性物化为默认 workspace，迁移后空 ACL 明确表示无权限；登录/session/token 不再接受脏 default 或空 ACL。`wechat_accounts.app_id` 已替换为全局 partial unique index。SR-016 让 `/auth/login` 与 `/auth/token` 在 Argon2 前共享客户端、规范化目标和进程级三维窗口；并发预约原子化，请求取消自动释放，正确凭据只清同客户端+目标失败。429 不回显账号或指纹；失败审计只保存进程盐化指纹，拒绝审计按窗口去重，Mongo TTL 90 天清理。应用只信直连 TCP 对端，多副本或反向代理部署仍必须保留独立边缘限流。
- 验证：既有认证单元 13/13、真实 Router Cookie/JWT 撤权 2/2、认证集成 4/4、账号密钥隔离/Debug 掩码/appId 唯一性 4/4。SR-016 纯限流与脱敏/TTL 契约 8/8、私有 429 响应 1/1继续有效；部署前授权服务器 `rs0` 红线与部署后正式 ELF `f0ead4f7…cde9b` 回环-only UUID 随机库门均通过。部署后精确覆盖 `login 401 → token 401 → correct login 429`、`Retry-After=300`、三条脱敏审计和严格 90 天 retention；正式 PID `2196929`、重启数 0、ELF 与健康响应前后逐字不变，随机库已删除。证据归档 SHA-256 `5146d63c…e10a`；反向代理/多副本边缘限流边界保持不变。

## HC-006：定义媒体文件与元数据的一致性和部署边界

- 来源：SR-017
- 两轮结论：文件系统和 Mongo 非原子；单机可通过补偿降低风险，多副本还要求共享存储。
- 推荐：`修复最小补偿；按部署决定是否迁对象存储`
- 最小处理：临时文件→DB 提交→原子改名，失败清理；定期扫描孤儿/缺失对象。
- 不过度工程边界：未确认多副本前不强制对象存储；确认多副本后才要求共享卷或对象存储。
- 不处理代价：孤儿文件、悬空资产、备份不一致；多副本下文件不可见。
- 已确认部署边界：当前是一个系统/单实例，暂无多副本要求。
- 人类决定：`先实现单实例补偿协议；对象存储/共享卷延后到明确需要多副本或高可用时`
- 实施状态：`已修复并验证 / 尚未部署（2026-07-20）`
- 负责人：实现：Kiro；多副本存储边界：待部署形态变化时复审
- 决定理由：临时文件、原子改名、失败清理和定期扫描足以覆盖当前单实例阶段；现在迁对象存储属于过度工程。
- 已完成：上传与换文件使用同目录 pending、fsync、Mongo CAS 和原子 rename；失败按实时引用补偿。上传、换文件、删除、发送读取和扫描器共用内容路径级进程锁，关闭单实例内 count=0 后并发新引用的删除竞态。启动即扫描且每小时重跑：恢复完整 pending，清理无引用 final/pending，校验内容寻址 SHA，损坏/缺失/非法路径素材退回 draft、停用并清 media_id；fail-closed 写入幂等，重复扫描不刷新时间。换文件更新与回滚均绑定 `updated_at` generation，补偿不会覆盖后续管理员动作。
- 验证：真实 Axum multipart + Mongo validator 故障注入 1/1；媒体一致性 4/4（失败补偿、崩溃/损坏恢复、孤儿/缺失/非法路径扫描、并发屏障）；既有媒体 CRUD 7/7；媒体存储单元 10/10；普通 `cargo check --tests` 通过；随机 Mongo 测试库归零。`RUSTFLAGS=-Dwarnings` 触发新依赖指纹后被本机安全软件以 `radium` build-script `Access denied (os error 5)` 阻断，未产生代码 warning 诊断。一个早期失败测试目录中的 10-byte 合成文件因执行策略拒绝删除而留在系统 Temp，不属于仓库、Mongo 或业务数据。
- 保留边界：该结论只覆盖当前单实例且 `MEDIA_STORAGE_DIR` 位于持久卷的部署；一旦启用多副本/高可用，必须先迁共享持久卷或对象存储并将进程锁升级为跨副本对象生命周期协议。

## HC-007：让运行、指标与审计陈述真实发生的事实

- 来源：SR-019、SR-068、SR-078、SR-079、SR-100、SR-140、SR-154、SR-156
- 两轮结论：run 首段失败可消失；DB 错误被显示为零；非快照计数和错误时间字段/样本上限被包装成精确指标；A/B实际版本丢失。多数不会直接改变客户消息，但会误导运营和事故判断。
- 推荐：`分级修复，先修运行追踪和错误伪零`
- 最小处理：入口先写 started、统一 terminal；错误返回 unknown/error；指标返回 asOf/window/sample/truncated；记录真实 selected version；修正误导字段名。
- 不过度工程边界：不建设实时数仓或全量 event sourcing；Mongo 聚合和明确不确定状态足够。
- 不处理代价：故障、成本和版本归因不可相信，运营可能依据错误百分比决策。
- 人类决定：`待确认（全部修复 / 仅关键审计 / 接受展示误差 / 暂缓）`
- 实施状态：`实现闭环 / Rust 完整库测试与服务器隔离复验通过 / Mongo 集成、真实链路与部署后回归待办（2026-07-21）`
- 已完成：Gateway 入口在任何 LLM 前写 started，首份业务决策后 CAS 到 running，正常/错误/panic 均收敛同一 run 行并区分决策前后失败；统一待审汇总按来源返回 complete/partial/error、失败值 null 与 errors，不再把 DB 错误显示为零；Observability 统一带 asOf，并逐指标声明非快照/缓存一致性和窗口；联系人列表最近入站由最多 500 次逐行查询改为页内一次聚合并补匹配索引；Reply A/B 实际命中版本沿 run-local 上下文写入 decision review，fallback 不伪造版本；停滞展示使用 Planner 同源维度及真实维度变更时间；LLM 成本总量由完整保留日志聚合，明细限制与截断状态独立展示。
- 验证证据：现有回归覆盖 ask-human partial/error/null、Observability metricScopes、任意停滞维度写时钟与 API 投影、101 条日志全量汇总但仅返 100 条明细、前端范围文案；本批新增真实 Gateway 探针测试覆盖“模型调用时已存在 started”“首份决策后 Review panic 时已是 running”及单行终结，并新增联系人聚合管道、prompt 版本和 usage known/unknown 边界单测。`cargo fmt --all -- --check`、`cargo check --lib`、三个相关集成目标 `--no-run`、前端定向 28/28 与 `tsc + vite build` 通过。授权 Linux 服务器在独立 `/tmp` 源码和 target 中运行完整 `cargo test --lib`，最终 `2116 passed; 0 failed`；`/opt/wechatagent` 无 tracked 修改，运行服务保持原 PID `1144911` 且健康检查正常。
- 验证边界：Windows 在工作区和两个全新 target 中均被系统策略以 build-script `Access denied (os error 5)` 阻断，已由 Linux 链接级库测试补足；但本轮没有执行随机 Mongo 集成、真实模型/外部链路、工作树部署或部署后回归，不能把库单测写成这些层级已通过。服务器 `/tmp` 中本批临时验证目录与归档待明确授权后清理。
- 负责人：实现与 Linux 隔离复验：Kiro；Mongo/真实链路与部署：待后续统一验证批次
- 决定理由/边界：用 Mongo 聚合、明确不确定状态、真实维度时间戳和单行 run 信封恢复事实可信度；不建设实时数仓或全量 event sourcing。

## HC-008：使 LLM 精确缓存感知租户和 provider 版本

- 来源：SR-020
- 两轮结论：热切 provider 后，完全相同输入可命中旧 provider 产物；范围限于白名单精确缓存，但违反切换预期。
- 推荐：`修复`
- 最小处理：cache key 加 workspace、provider/model 和 provider generation；激活时 bump generation。
- 不过度工程边界：保留进程内缓存，不引入 Redis 或全局失效服务。
- 不处理代价：切换后的短期结果和日志仍来自旧 provider。
- 人类决定：`修复（按最小进程内代际方案实施）`
- 实施状态：`工作树已修复 / Rust 类型检查与 Linux 隔离动态验证通过 / 部署后回归待办（2026-07-21）`
- 负责人：实现与隔离验证：Kiro；部署后回归：待统一上线批次
- 决定理由：精确缓存规模仅 256，继续使用进程内 LRU；通过 workspace、provider/model、单调 generation、Prompt pack 和输入哈希隔离，不引入 Redis 或跨副本失效服务。请求在缓存查找前固定 Registry 快照，cache miss 也使用同一 client，因此并发热切有明确线性化点；每次 swap 都递增 generation，覆盖同 provider id 下 API key/base URL/format 等配置变化。
- 验证：`cargo check --lib`、`cargo check --tests`、格式与差异门通过；授权 Linux 隔离环境运行缓存身份 5/5、Registry 快照/代际 1/1。测试前后生产服务保持原 PID `1144911`、健康正常，生产仓库无 tracked 修改。当前未部署，也未执行部署后的真实 provider 热切回归。

## HC-009：统一客户发送前的事实与安全 fail-closed 边界

- 来源：SR-022、SR-023、SR-031、SR-042、SR-051、SR-175
- 两轮结论：Reviewer漏报/畸形可使产品声明和三类危险评分失效；revision失败可恢复已知危险原稿；quote、价格和数字护栏不完整。均可进入真实发送链。
- 推荐：`修复（P0/P1）`
- 最小处理：严格 Reviewer wire DTO；安全类 revision失败 hold；服务端校验 opened/quote/anchor 和结构化产品价格；holding reply复用现有数字护栏；产品效果类漏判强制复审。
- 不过度工程边界：不新增第二套通用审核 Agent或关键词大全；复用现有 finalize、目录和确定性检查。
- 不处理代价：编造事实、压迫话术、内部信息或错误价格可能被标安全并发送。
- 人类决定：`修复（已实现；真实模型验证阻塞）`
- 实施状态：`production-wired / deterministic-verified / real-model-capacity-blocked（2026-07-25）`。SR-175 的实时 Reviewer 严格 wire、必审不变量与 fail-closed 收口已接入工作树；部署和真实模型复验仍未完成。
- 负责人：Kiro
- 决定理由/允许回退的纯风格类别：安全与事实失败一律 fail-closed；只有不涉及事实、隐私、压迫或授权边界的纯风格改写失败才允许恢复原稿。任何 `should_reply=true` 的正文都必须执行独立 Reviewer；自报 `needs_review=false`、低风险或高 confidence 只能影响 light/full 深度，不能跳过审核。默认销售域低风险走 light Reviewer，高敏域强制 full Reviewer；预算耗尽或意外本地兜底时正文分别落 `blocked_by_budget` 或 `blocked_by_safety_guard`，只有主动沉默可本地完成。SR-175 将实时 Reviewer 与历史持久化读取分层：实时 `approved`、六个发送闸评分和 `requiresProductKnowledge` 必须存在且类型正确，评分必须为 0..10 整数，alias/canonical 不得同时出现；缺失、错类型、越界或歧义均形成可审计 safety hold。主 Reviewer 和已启用的第二 Reviewer 都执行同一严格解析；实时明确给出的 `pressureRisk=0` 或 `boundaryPrivacySafety=0` 不再享受历史缺字段的零值豁免。SR-022 另由独立语义 Claim Gate 收口；SR-023 的安全类 revision 失败保持 hold；SR-031 只接受 verified、opened、非 contradiction 且 quote/anchor 闭环的证据；SR-042 的 holding reply 经独立语义审查与全场景数字授权校验并统一走 Outbox；SR-051 要求每条目录声明携带最终回复完整 clause 的精确 `sourceQuote`，服务端逐字段核验 productId/name/amountMinor/currency/SKU，并反向覆盖正文中每个 active 产品名/SKU clause，完整性失败直接 hold。
- 生产接线：客户主链、revision 二审、管理发送、Knowledge Agent answer、授权过期与链尾 holding reply 均已切入上述边界；revision 对改写后的最终正文重新执行 ClaimGate。旧的“任一 quoted product id 命中即目录背书”入口已删除，`quoted_product_ids` 只作历史协议兼容。
- 确定性验证：既有 `strict_evidence_` 5/5、Knowledge Agent PBT 17/17、ClaimGate 契约 12/12、Escalation 96/96 与 `cargo check --lib` 证据继续有效；本轮又实跑 Claim Gate 12/12 与 revision fallback 3/3。SR-175 收口后严格 wire 专项 6/6、必审契约 5/5、完整 `agent::review` 107/107、`cargo check --tests`、格式与 scoped `git diff --check` 通过；覆盖缺字段、null/字符串/小数/越界、alias 歧义、主/第二 Reviewer fail-closed、历史零值兼容、实时零值拦截、默认/高敏域不可自我豁免、默认 low-risk 选择 light / 高敏域选择 full、本地兜底不批准正文和主动沉默兼容。预算 PBT 源码通过 `cargo check --test autonomy_protocol_pbt`；动态 PBT 在 Windows 链接阶段超过 15 分钟，断言未启动且遗留进程已定向清理，不记为失败或通过。SR-051 的目录声明覆盖证据保持不变。
- 真实模型验证：`blocked`。2026-07-21 在授权测试服务器确认生产实际文本端点为 NVIDIA HTTPS 配置（旧 `127.0.0.1:9090` 记录已过时）；只读 `/models` 鉴权探测返回 HTTP 200、118 个模型，耗时约 40 秒。随后在独立 `/tmp` 源码/target、随机 `wechatagent_test_*` 数据库和 WireMock MCP 下运行合成恶意样本：六轮知识编造弧在 30 分钟内未完成，已定向终止；单轮 `t6_real_unverified_product_claim_is_gated` 在 9 次重试后收到上游 `503 ResourceExhausted` 并走测试框架 transient-skip。两次均未执行到可判定的 Reply/Reviewer/Claim Gate 业务断言，因此不得记为通过或失败。测试进程与随机库已清零；生产 PID、二进制、仓库状态保持不变，健康 200。待端点容量恢复后重跑单轮 T6 并要求非 skip，才能改为 fully-verified。

## HC-010：给异步任务与发送建立最小所有权和不确定送达语义

- 来源：SR-028、SR-034、SR-050、SR-052、SR-066、SR-172、SR-177
- 两轮结论：Reaction/Task/ledger/Outbox缺 owner-generation 或完整业务键；取消无法撤回已经越过门的发送；名片重试无送达核对；Webhook ACK 后只有内存 runner。
- 推荐：`修复核心发送与任务 fencing；按部署决定 durable webhook`
- 最小处理：现有记录加 owner/token/generation，finalize CAS；台账绑 outbox/account；取消后 worker在 MCP 前再检查 token；无法核对的送达标 uncertain 而非自动重发；Webhook可用 messages+pending generation 形成持久工作项。
- 不过度工程边界：不引入 Kafka/新调度平台；先扩展现有 Mongo task/outbox/message 集合。
- 不处理代价：重复发送、取消后仍发、消息永久不回复、反馈/任务旧执行者覆盖新状态。
- 已确认部署边界：当前单实例、尚未生产上线；即使单实例，ACK 后进程退出导致永久不回复仍不可作为正式业务语义。
- 建议方案：不用 Kafka。Webhook 落消息时，在 Mongo 中 upsert 每联系人一条 durable inbound job（`workspace+account+contact` 唯一，含 generation、debounce_deadline、status、owner、lease）；内存 debounce 只作加速，后台 worker 到期 claim 并聚合处理，崩溃后按 lease 重领，finalize 以 generation CAS 防旧执行者提交。Outbox/Task/Reaction 复用同样的 owner-generation-fencing 模式。
- 人类决定：`采用 Mongo durable inbox + 现有 Outbox/Task fencing，不引入消息中间件`
- 实施状态：`核心协议已实现，验证与部署分层收口中`。SR-052/SR-066 为 `production-wired / deterministic-verified / isolated-server-mongo-redline-verified / real-model-run-pending / deployment-pending`（2026-07-26）；SR-028/SR-034/SR-050/SR-177 为 `working-tree-wired / deterministic-verified / isolated-real-mongo-verified / deployment-pending`；SR-172 已于 2026-07-25 完成部署后已发布二进制真实 Cookie Router+Mongo 动态闭环。
- 本批边界：Outbox 已增加 claim token/generation、最后可取消 CAS、`send_started_at` 与 `delivery_unknown`；名片跨远端边界后崩溃不再回 pending，in-flight 取消在远端成功时如实收敛 sent、回执不明时停止自动重放。Task 与 Reaction 均已增加 owner token/generation 和 owner-scoped 提交；Reaction 的轨迹、负例与 Outbox 取消只在当前 owner 成功提交后执行。SR-050 新台账以不可变 `outbox_id` partial unique + `$setOnInsert` 幂等收敛，所有历史、回扫、API 与前端统计均固定 workspace/account，存量无锚行保持兼容；SR-172 管理 DTO 已投影互斥 typed payload、不可变对象 id、恢复/取消边界。SR-177 已把入站事实与 `handoff_status=pending` 同文档持久化；worker 恢复扫描物化每租户/账号/联系人唯一 durable task，周期扫描与即时唤醒共用到期原子 claim，后到消息按 `(created_at,_id)` 刷新同一 task 并清除旧 claim。真实 Inbound 触发保留 task token；主回复、非文本过渡和 Dispatcher 均执行 decision bind→enqueue→authorize fencing，旧 generation 不能写画像/记忆或触达 MCP。若进程在 Outbox 已写入但 Task 尚未授权时退出，新 owner 仅可接管同一 durable task 中 `attempt=0`、无旧授权 marker、未跨 `send_started_at` 且非人工取消的旧行；接管后仍须以当前 claim 完成 authorize。
- 验证状态：SR-034 本地 Task owner 10/10、Memory fencing 11/11、索引 5/5、迁移 3/3 与 `cargo check --lib` 通过；授权测试服务器随机隔离 Mongo 运行 `tests/sr034_task_send_fencing.rs` 4/4 通过。SR-028 服务器随机隔离 Mongo 2/2 通过；SR-050 定向单测 19/19 与服务器随机隔离 Mongo `sr050_*` 3/3 证据继续有效；SR-066 在服务器 `rs0` 与 localhost MCP mock 中五条投递红线 5/5 真实通过。首轮旧夹具缺 scoped MCP 账号导致 1/5，只补测试前置账号后完整重跑，不放宽生产 fail-closed 门。SR-172 已用已发布正式二进制完成真实 Cookie Router+Mongo 动态复验。SR-177 的三条隔离真实 Mongo 红线 3/3 证据继续有效。上述新增工作树均未部署；GitHub `smoke_t4`、真实模型 T4、真实外部 MCP/WeChat 和部署后 SR-066 回归仍待执行。batch2 证据清单 SHA-256 为 `aa6f91f0521dd5ffbbcef636122fcdd3994f7d4b5c1fa04e776e77cca71e08c7`。
- SR-172 部署后专项（2026-07-25）：使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅回环临时实例和随机库 `wechatagent_test_4123a8d865b549fbb6779f939ef56137` 中通过真实 `wa_session` Cookie 中间件执行。账号 A 列表返回 200 且严格 4 条：text、同账号 media、referralCard 均携不可变对象 id 与可读元数据，`reclaimedInFlight=true/reclaimCount=2` 保持；账号 A 引用账号 B 私有素材时只返回 assetId，不泄露标题和文件名，账号 B 控制行不可见。以错误 `expectedAccountId` 取消返回 409，完整 Outbox BSON 与取消事件计数逐字不变；正确账号取消返回 200，仅目标行转 `canceled`，并精确写入一条同账号 `outbox_canceled` 审计事件。临时单元 `Result=success/ExecMainStatus=0`，35 项原始证据哈希全部通过；成功证据库确认保留，正式 PID `1686295`、重启计数 0、运行中/磁盘哈希、健康及正式库随机 marker 前后不变。证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr172-deployed-isolation-20260725T053911Z`。此前 `053442Z` 与 `053703Z` 两次诊断分别因 mongosh 警告污染 JSON、canonical EJSON 整数封装与证据断言不一致而退出；均未暴露业务失败，临时单元正常退出、正式服务零漂移，诊断目录和随机库原样保留。该专项只关闭 SR-172，不自动结算 HC-010 其它 SR。
- 负责人：
- 决定理由：比纯内存 runner 可靠，又符合当前单实例和不过度工程边界；未来多副本时协议仍可复用。

## HC-011：定义长期记忆的提交、撤销与容量契约

- 来源：SR-029、SR-181、SR-182
- 两轮结论：Memory consolidation附属写可半提交；operator memory承诺撤销却无入口；coreFacts cap与永久保留合同不可同时满足。
- 推荐：`修复并简化契约`
- 最小处理：consolidation id/phase+幂等副作用；operator memory软撤销；coreFacts保持有限窗口，按权威/重要度排序，被淘汰项进入可追溯 archive/recent。
- 不过度工程边界：不做无限版本树或把全部历史塞进 prompt；持久事实库与 prompt 投影分层即可。
- 不处理代价：错误偏好无法撤销、事实静默消失、崩溃后主卡与派生视图分裂。
- 人类决定：`采用有限投影+可追溯归档（SR-029/SR-181/SR-182 均已完成实现、动态验证与部署闭环，HC-011 已结算）`
- 负责人：Kiro
- 决定理由/核心事实淘汰规则：运营记忆采用 `revoked_at/by/reason` 软撤销，active 读取与两处 prompt formatter 均排除 revoked；Chat 与 Atlas 共用完整 workspace/account/operator/memoryId scope。coreFacts 采用 cap 6 的有限投影：候选按结构化来源、重要度、置信度、更新时间和稳定键统一排序；容量淘汰项优先进入 recent，并写入最多 20 条的 `coreFactEvictions` 审计。审计跨 consolidation 继承，显式 discarded 或重新升入 core 后移除；长期记忆下钻显示内容、原因、原排名和时间，但归档不重新注入 prompt。SR-181/182 联合前端专项 7/7、TypeScript、rustfmt、scoped diff、撤销接线 5/5 与容量契约接线 6/6 通过。SR-181 在部署后 HTTP 专项之外，又以与部署冻结源码同哈希的生产实现完成本机隔离 Mongo `record → revoke → re-add` 1/1；SR-182 完整 `memory_card_invariants` 20/20 通过，且实现与测试文件均和部署冻结源码逐字同哈希。SR-029 后台任务使用 durable `running → committing` 线性化点、prepared payload 与幂等恢复；管理员手动入口也必须创建 contact-scoped single-flight task，并经统一 claim/heartbeat/committing 协议同步执行。SR-029 已完成本机崩溃窗口 2/2、候选隔离验证、正式切换及正式路径部署后 Router 动态复验，因此 HC-011 三项来源均已闭环。
- SR-181 部署后专项（2026-07-25）：使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在仅回环临时实例和随机库 `wechatagent_test_5532df823add4c229ef3120b5256d2f9` 中通过真实 `wa_session` Cookie Router 执行。active 列表初始只返回目标记忆；错误账号和错误 operator 撤销均返回 404，完整记忆 BSON 与撤销事件数前后逐字一致。正确撤销返回 200 并写入首次 actor/reason，默认列表随后为空，`includeRevoked=true` 仍返回审计行；重复撤销返回 `alreadyRevoked=true`，完整记忆与事件数逐字不变且总事件恰为 1。临时单元 `Result=success/ExecMainStatus=0`，47 项证据哈希全部通过；证据库确认保留，正式 PID、重启计数、运行中/磁盘哈希、健康及正式库随机 marker 前后不变。证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr181-deployed-isolation-20260725T055632Z`。该服务器 Router 专项自身不经过模型，也不把手工种数冒充新增 helper 验证；`record/re-add` 红线与 SR-182 随后由下一条同源动态补证独立关闭。
- SR-181/182 本机同源动态补证（2026-07-25）：服务器冻结部署源码与本地 `src/agent/memory.rs` SHA-256 均为 `7dbadb50add6f1e029760abf3fbee516c34fd4c51edd506c2f590d41b598669c`，SR-182 测试文件均为 `cbd9e0b5fd440485cb0c045b4b57176c4116074a7ae83ae7d944eafa64010901`。`memory_card_invariants` 20/20 通过，覆盖 cap、稳定优先级、容量淘汰迁入 recent、跨轮审计继承、显式 discarded 与重新升入 core 后清理。SR-181 生产 helper 在本机 MongoDB 8.0、全新回环 dbpath 中 1/1 通过完整 add/revoke/scope/re-add 生命周期。首次成功运行暴露测试末尾漏调 `app.cleanup()`，旧随机库作为诊断证据保留；仅补测试清理后在第二个全新 dbpath 再次 1/1，通过后只读清单为 `[]`。两次结束后系统 MongoDB 服务均保持 `Stopped`、27017 无监听、无残留 mongod；最终证据目录为 `target/sr181-mongo-40de1a10157a4ce392e0026dbbd7dcb0` 与 `target/sr182-pbt-20260725T142158`。该补证关闭 SR-181 helper 红线及 SR-182 确定性契约，不外推关闭 SR-029。
- SR-029 完整闭环（2026-07-25）：管理员 `POST /contacts/:id/memory-consolidation/run` 不再直接调用无 claim 的多集合写链，而是在 `active_task_key=memory_consolidation` 唯一约束下创建独占任务；已有 pending/running/committing owner 时明确 409，创建成功后只经统一 `run_due_task_by_id` claim、heartbeat、prepared commit 与恢复协议执行，任务未到 `sent` 不返回成功。本机 MongoDB 8.0 全新回环 dbpath 中 2/2 通过：手动路径只产生一条 durable task且并发 owner零写；prepared commit 从全未应用、仅主卡已应用、联系人/候选/事件部分完成三个崩溃窗口连续重放后，主卡、联系人投影、候选、两类事件与任务均恰好一次收敛；证据目录 `target/sr029-mongo-4a0895ef9fb342e2b3df70c412bd7f0a`。服务器从冻结部署源码构造只含三份 SR-029 文件差异的候选，release 二进制 SHA-256 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b`；候选先通过随机库迁移、49 条迁移台账、五类空队列、仅回环健康及 69 个静态文件逐字验证，再通过真实 `wa_session` Cookie Router 的 `200 sent/no_candidates` 与 active owner `409/零写` 红线。经用户确认后从旧哈希 `539effe4...` 原子切换到新哈希，自动回滚门未触发；正式 PID 为 `1740029`、重启计数 0。随后直接使用正式路径新二进制，在全新随机库再次完成通用烟测及 Router `200/409/零写`；正式数据库随机 marker、常驻 PID、磁盘/运行中哈希与健康前后不变。部署后证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr029-deployed-smoke-20260725T090452Z`、`.../sr029-deployed-router-20260725T090845Z`，清单 SHA-256 分别为 `9bc5043f0024be250b6c44c25611725d32e2dbfb84646a8b9e3568cb63097dd2`、`89c2443511492c0be20775b3f317fed33ebbb0218fbf25758e638bdbfe3a7b30`；切换证据位于 `.../switch-sr029-20260725T090151Z`。

## HC-012：收口 Knowledge Agent 的可见域、缓存和整体 deadline

- 来源：SR-030、SR-032、SR-033
- 两轮结论：账号私有知识可在正文下钻时进入模型；内容编辑后缓存可陈旧；工具循环deadline和最终payload不一致。
- 推荐：`修复`
- 最小处理：所有 open/follow/version 接受同一 visibility scope；corpus签名含revision generation；一个 deadline 包住 LLM+工具并在返回前检查 final。
- 不过度工程边界：复用当前 Agent和进程缓存，不建平行检索链或新缓存基础设施。
- 不处理代价：跨账号内容进入模型、知识修订短期不生效、超时请求仍拖延或返回非终态。
- 人类决定：`修复（已按推荐方案完成本地代码，待 Mongo/测试服务器真实链路复验）`
- 负责人：Codex 实施；人工负责人待填写
- 决定理由：复用现有 Knowledge Agent、进程缓存与 Chat 工具循环即可同时消除跨账号正文泄漏、修订后短期旧答和超时非终态，不需要新检索链或缓存基础设施。
- 实施证据（2026-07-21）：
  - 所有 Knowledge 下钻、关系遍历和版本跳转均携带账号可见域；archived 旧版仅可用于读取 `superseded_by`，最终正文仍强制 active+verified+可见。
  - Chat Mongo 工具、apply、关系写侧、KnowledgeTask fix/retag、Cold Contact hooks、`/knowledge/ask` 与 SSE 入口均补账号归属/可见域校验；会话含多个账号 pending 草稿时必须显式选取已有草稿账号。
  - 答案缓存使用完整可见 chunk 的 `_id+updated_at` 签名，并纳入 provider/model/generation、prompt-pack generation、入口 filter、query、轮数；模型后续 filter 只能收紧入口 capability。
  - Chat LLM 与工具 await 共用单一绝对 deadline；仅真实 final 原 payload 可透传，强制停止返回明确 final fallback。
- 本地验证：`cargo fmt --all -- --check` 通过；`cargo check --lib` 与 `cargo check --tests` 通过；新增 deadline/final/scope/cache/version 回归代码均完成类型检查。
- 待复验：本机 `cargo test --lib` 在链接阶段被既有 `ring 0.17.14` MSVC 原生符号缺失（LNK2019/LNK1120）阻断，未执行测试断言；需在测试服务器修复工具链后运行 Rust 单测、Mongo 跨账号集成测试、archived→active 版本跳转和真实模型 30 秒 deadline 场景。

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
- SR-057 实施：疑似成交 approve 改为 validate-first + transaction。共享 `prepare_outcome_event` 在任何 signal 状态写入前完成联系人存在性、金额/币种闭集、产品 workspace/status 归属、订单式产品快照和 BSON 审计构造；Mongo 事务内再以 `_id + workspace_id + status=pending` CAS，追加 `contact.outcome_events`、插入 `outcome_event_marked` 审计并终结 signal。CAS、contact append 或审计任一步失败均 abort，重复审批不能双计，非法请求保持 pending 可修正重试。普通手工登记继续复用同一 prepare/persist 规则，不建立第二套成交逻辑。
- SR-057 部署后验证（2026-07-27）：当前正式 release 的成交审批 Handler 与三条专项测试源码和本地逐字一致。服务器真实 Mongo `rs0` 运行 3/3：成功审批将 signal 终态、`staff_confirmed` outcome 与审计同事务提交，重复审批不增加 outcome/audit 且伪造 `reviewedBy` 被认证管理员覆盖；负金额在任何状态写前拒绝并保留 pending；审计 validator 拒写时 signal、outcome、audit 三处整体回滚。三个随机库均删除，测试库 126→129→126；正式 PID `2021387`、重启数 0、运行中/磁盘 SHA-256 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 与健康状态不变。服务器证据目录 `audit/sr057-suspected-deal-20260727T130000Z` 的最终清单 SHA-256 为 `e98047eeb2c78d62b8fe868aa08681cec6b1db50d9ab65ba7697671e909fe1b4`。本结果同时关闭 SR-058 的疑似成交 actor 子路径；不自动结算 SR-067、SR-097、SR-169。
- SR-058 实施：新增私有 `ReviewActor` 可信身份类型。三类 REST 审核只能从 `AuthenticatedAdmin.username` 构造 actor，空用户名直接拒绝；请求 JSON 中旧 `reviewedBy` 字段可被兼容忽略但不能进入模型或持久化。Management 审核只能从闭集 `SystemReviewActor::ManagementAgent` 构造固定 `system:management_agent`，内部 helper 不再接收任意 actor 字符串。媒体与名片审核原本已直接使用认证管理员，无需改动。
- SR-058 验证：`cargo fmt --all -- --check`、`cargo check --lib`、`cargo check --test suspected_deal_e2e`、`cargo check --test transactional_admin_flows` 与 scoped `git diff --check` 通过；可信 actor 闭集/空用户名单测 1/1、三类请求反伪造契约 3/3 通过。副本集持久化用例覆盖疑似成交、关系建议和 Taxonomy 请求显式携带伪造 `reviewedBy` 后仍分别记录真实认证管理员；测试已编译，本机无 Docker 未执行动态断言，不记 fully verified。当前工作树未部署；本批不自动结算 SR-059、SR-060、SR-067、SR-097、SR-169。
- SR-059 实施：关系建议 approve 改为 validate-first + transaction。事务前完成建议状态、Taxonomy canonical 值、联系人 workspace/account 归属校验；事务内以建议 `_id`、完整租户/联系人身份、`suggested_value`、`last_seen_at` 观察代际及 `status=pending` 做 CAS，再以 update pipeline 将 canonical `relationship_type` merge 进联系人画像并终结 approved。缺失或显式 `null` 的画像容器按空文档初始化，非文档脏值让事务失败；其它画像键保留，`domain_attributes_updated_at` 与 `updated_at` 同步。建议 CAS、联系人写或 commit 任一步失败均 abort，不留下画像/审核终态分裂。
- SR-060 实施：新增 m045，在索引变更前全量审计 pending 行的 canonical workspace/account/contact 与同联系人重复 pending；只退役键顺序和 options 都精确匹配的旧全量 unique，反向键序或未知 options fail-closed。启动随后建立命名 partial unique `(workspace_id, contact_id) WHERE status=pending`。Gateway 继续只刷新现有 pending；终态历史不复活也不占槽，新证据 upsert 为下一周期 pending，同周期并发仍由唯一索引收敛。审核历史属于不可伪造的运营审计事实，本批不设置 TTL；若未来需要冷热归档，应另定保留期与合规策略，不能在本修复中静默删除。
- SR-059/060 验证：`cargo fmt --all -- --check`、`cargo check --lib`、`cargo check --test transactional_admin_flows`、`cargo check --test m045_relationship_review_cycles` 与 scoped `git diff --check` 通过；partial unique、m045 精确索引识别/身份审计、请求反伪造完整名纯单测 4/4 实际执行通过。副本集真实 HTTP 用例覆盖首次审批同时写画像与终态、终态后新 pending 可共存、第二条 pending 被 E11000 拒绝、联系人 validator 拒写后建议仍 pending 且旧画像不变；m045 动态用例覆盖精确退役/替换和脏 pending 在删索引前零破坏失败。动态用例均已编译，但本机无 Docker、mongod 且 `TEST_MONGODB_URI` 未设置，未执行业务断言，不记 fully verified；当前工作树未部署。该次验证时 HC-019 的 SR-067、SR-097、SR-169 仍开放，后续状态以下方独立记录为准。
- SR-054 实施：新协议裁决 CAS 同行写入 `relay_state=pending` 与确定性 `relay_task_id`；admin/微信裁决共用入口。relay task 以 escalation `_id` 为 `_id` 做 `$setOnInsert` 幂等物化，成功确认后标 `enqueued`；task worker 每 tick 在 claim 前补偿 pending intent。首次物化失败、进程中断、请求重试和多副本扫描均不会吞转述或重复创建 task；已有 running/终态 task 不被重置。旧 resolved 行缺少协议字段时不猜测重放。
- SR-054 验证：`cargo fmt --all -- --check`、`cargo check --all-targets`、索引/迁移契约与 scoped `git diff --check` 通过。授权服务器以随机副本集 Mongo 实际运行 `ask_human_phase1_e2e` 14/14，其中 `admin_resolve_enqueues_relay_and_marks_resolved`、`admin_resolve_is_idempotent`、`resolved_relay_intent_recovers_exactly_one_task` 均通过；与 principal channel 15/15 合计 HC-013 动态回归 29/29。随机库全部清理，正式生产切换仍待确认。
- HC-013 实施收口：Escalation 同行冻结 domain、policy version/快照、领导账号、卡片正文与 delivery generation。首卡和改派只物化确定性 Outbox；scanner 用 generation CAS，多副本收敛为一代一条，dispatcher 在最后远端边界复核 generation，旧卡失权即取消。只有 Outbox `sent` 对账后才启动 timeout；`failed_terminal/canceled` 收敛为 `delivery_failed` 并释放 pending 唯一槽。pending 唯一键和领导回复匹配均加入 account，Ask-Human 保存时验证 `accountId→contact` 真实归属。
- HC-013 relay/awaiting 收口：裁决同行写 durable relay intent，正常客户转述确认送达或授权过期时再写 `relay_state=terminal + terminal_at/reason`。Contact 不再靠单个易误清布尔值，而以 escalation `_id` 集合作为 awaiting owner；创建只 `$setUnion` 自己，终结只 `$filter` 自己，因此不同类别并发等待互不覆盖。m047 在首写前验证联系人唯一归属，只回填 pending、新协议未终结 relay、或仍处于 pre-Outbox `pending/retry/running` relay task 的旧行；旧 `outbox_enqueued` 可能已经送达但未回写 task 终态，属于歧义事实，明确不据此制造 awaiting。迁移不创建任务、不重放普通旧 resolved 历史。
- HC-013 配置/治理收口：每位决策人必须持久化可发送 `accountId`；加载失败时配置页禁保存，空决策链可显式关闭，quiet hours 可显式清除。领导裁决只授权本次客户转述；可泛化内容仍进入 draft/needs_review，不直接成为 verified 知识。
- HC-013 验证：`cargo fmt --all -- --check`、`cargo check --all-targets`、scoped `git diff --check` 通过；principal escalation、索引、m046/m047、generation/owner/relay 状态等库级契约及迁移 ID 门通过，Ask-Human 前端配置与决策人编辑器 17/17 通过。授权服务器以本机副本集 Mongo、随机 `wechatagent_test_*` 库、8 MiB Rust 线程栈实际运行 `principal_decision_channel` 15/15 与 `ask_human_phase1_e2e` 14/14，共 29/29；覆盖真实 WireMock MCP 成功/失败分支、queued 不启动 timeout、sent 后完整窗口、终态失败释放槽、并发 scanner 单 generation、旧 generation 远端前取消、relay intent 崩溃恢复、授权过期终态、双 owner 独立终结及 admin resolve 幂等。初跑暴露旧测试夹具缺客户/账号身份，补齐真实前置条件后完整回归全绿，未放宽生产 fail-closed 校验；随机测试库全部清空，运行服务 PID/二进制哈希全程不变且健康。
- HC-013 上线演练：隔离 release 与前端生产构建通过；新增显式 `migrate_only` 运维入口，只执行 migrations+indexes、拒绝系统库且需数据库名确认串。对生产库完整克隆执行 m040–m047 两轮演练：首轮发现 legacy `outbox_enqueued` task 会误造 awaiting owner，结合真实 MCP 成功事实收紧 m047 后重做；第二轮 8/8 迁移成功，contacts/escalations/tasks 数量均不变，awaiting owner 0→0，旧 principal pending 索引被 account-scoped partial unique 精确替换。最终新 release 又以 `env -i`、随机空库、回环临时端口和不可达测试 MCP/LLM 完成真实启动：47 条迁移与全索引成功，`/api/health` 返回 ok，Evolution 保持关闭；隔离进程、随机库、演练库与归档均已删除。生产库未写、服务未重启，正式二进制/前端切换与生产迁移仍待显式确认。

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
- SR-056 实施：DomainSchema POST 只创建全新 lineage；PUT 必须携带所选 `expectedVersion`，精确读取来源并追加 inactive 新版本，历史版本不可变。activate/delete 以 `schemaId + expectedVersion` 指向具体版本；激活在 Mongo 事务内 CAS demote 旧 active 并 promote 目标，删除只允许命中所选 inactive 行。运行时发现 workspace 多 active 时 fail-closed。m044 在唯一索引前只读审计 canonical identity、正版本、lineage 版本唯一与 workspace active 至多一条，歧义数据不选赢家、不写数据；随后 `(workspace_id,schema_id,version)` unique 与 workspace partial unique active 索引锁住协议。前端同 slug 多版本卡片把所点版本分别传给编辑、激活与删除。
- SR-056 验证（2026-07-27）：当前正式 release 的 DomainSchema 路由、m044、索引与副本集测试源码已和本地逐字核对；正式库 m044 为 `applied`，版本 unique 与 workspace active partial-unique 索引形状正确。服务器真实 `rs0` 精确生命周期用例 1/1：v1 active + v2 inactive 事务切换到 v2，active 始终恰好一条，两条不可变历史均保留；不存在 v99 返回 `domain_schema_version_changed` 且指针零变化。随机库 126→126，正式 PID `2021387`、`NRestarts=0`、运行中与磁盘 SHA-256 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 一致。证据见 [SR-056 部署后验证记录](production-release-2026-07-27-sr056.md)；本结论不自动结算 HC-014 其它 SR。
- SR-094 实施状态（2026-07-30）：`已部署制品观测 / 确定性验证完成 / 真实 Cookie Router+副本集动态闭环`。`user_operations.runtime_parameters` 的手工编辑与 Guide 写入共用同一 typed 边界：手工编辑先规范化旧字段名，再拒绝未知键、错误 BSON 类型、越界值及跨字段预算倒置；Guide 只能修改节奏、上下文与 quiet-hours 白名单，安全阈值、模型预算、投递 lease 和协议开关即使类型合法也会被拒绝。Guide 冻结完整 runtime 文档，Apply 以 domain `_id + version + updated_at + current_version=true` CAS，并在事务中让 Domain guard、审计、Preview 终态和稳定 receipt 同行提交。本批只新增专项测试和动态证据，正式制品中的生产实现未改、未切换。
- SR-094 动态验证：授权服务器真实 `wa_session` Cookie Router+`rs0` 随机库专项 1/1。手工 PUT 验证合法 cadence 写入与 legacy→canonical 规范化，未知键 400 且目标 Domain 完整 BSON 零写；Guide 高风险 `runTokenBudget` 输出 400 且不创建 Preview capability，合法 `maxDailyTouches=2` Preview 被服务端推导为 workspace-wide，缺强确认 Preview/Domain 零写，确认后事务提交 runtime 和单条审计，同 candidate 重放返回稳定 receipt。首轮成功 Apply 后仅因测试把合法 BSON Int64 限定为 Int32 而红灯；只修 oracle 为整数数值等价，生产代码未改，最终重跑 1/1（16.33 秒）。`SCRIPT_RC/CLEANUP_RC/FINAL_RC=0/0/0`，随机库差集、挂载和临时构建根均为 0，共享 target 5,584 项哈希通过；正式 PID `2410064`、零重启、ELF `d0b7ffc63ce93a0e4f3ec09e1d8b64c06c578fd08879908c25d04caa531e027b` 与健康前后不变。证据 `/tmp/hc014-sr094-evidence-20260730T073000Z`，清单 SHA-256 `4aac2a22e50a08613c35140a6b3bd7bbc9d584a383eab82034a2891c9afd550d`。
- SR-168 实施状态（2026-07-25）：`已部署 / 确定性验证完成 / 服务器已发布二进制真实 Cookie Router+Mongo 动态通过`。Taxonomy API 完整投影 `priorityWeight/isTerminal/isReactivationTarget`；前端打开编辑器时冻结完整基线，保存时规范化并逐字段比较，只 PATCH 真正变化的字段。普通改名不再携带默认 `false` 覆盖 Planner 使用的终态/再激活语义，显式切换 flag 仍可独立提交；后端继续使用既有局部 `$set`，未把字典改成自由 BSON。
- SR-168 验证：既有 Taxonomy Rust 投影/请求/契约单测 5/5、前端系统策略交互 23/23、前端 taxonomy 契约 5/5、TypeScript、格式、专项编译与 scoped diff 门继续有效。部署后使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf` 在仅回环临时实例和随机库 `wechatagent_test_93cc4a38200440c991942ed2da005b66` 中通过真实 Cookie 中间件执行：GET 返回 200 并完整投影 `73/true/true`；请求正文只含 `label` 的 PATCH 返回 200，响应及 Mongo 读回均保持 `priorityWeight=73`、`isTerminal=true`、`isReactivationTarget=true`、`currentVersion=true`。临时单元成功退出，29 项原始证据哈希全部通过；正式 PID `1686295`、重启计数 0、运行中/磁盘哈希和健康前后不变，正式库随机探针计数前后均为 0。证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr168-deployed-isolation-20260725T051725Z`，测试库保留。该结论只关闭 SR-168，不自动结算 HC-014 其余 SR。
- DomainProfile 协议实施状态（更新至 2026-07-30）：SR-043/044/072/073/074/089/090 已部署并完成服务器专项验证。SR-072 的核心 active 指针、永久非法输入零写边界及运行时 Policy fail-closed 补强均已进入正式 ELF；启动对账已把生产 9 态/0 Policy 精确收敛为 9 态/9 条唯一 active current Policy，原短暂 fail-open 窗口已封闭。版本/current/active 由 m051 审计、三个 unique/partial-unique 索引和事务/CAS 约束；编辑只追加 draft，publish 只移动 published-current，activate 才移动 runtime-active。动态维度 kind 经过 canonical 单路径、重复与保留 namespace 校验；Operation Domain reset 改为 append-only 新版本；Unicode key normalizer 按字符索引；草稿与待激活版本进入主列表及统一待审富卡。本批不自动结算 SR-056 或 SR-094。
- SR-072 补强：激活事务在 active 首写前从同一快照校验目标全部永久属性；`generated_state_machine=Some` 必须有非空 states 且结构合法，非法内容返回 `BadRequest` 并 abort。新增纯函数回归覆盖 None/合法/空/非法引用，并接入真实 Handler 副本集零写红线，对 `domain_profiles`、`operation_domain_configs`、`operation_state_policies`、`contacts` 四集合完整 BSON 做前后比较。核心提交后的状态机、Policy 与联系人同步继续返回分步骤 `completed/partial`、`retryable` 与错误明细，前端保留重试入口。
- SR-072 补强、验证与正式部署（2026-07-30）：不把四集合写侧扩成大事务；Policy loader 仅在完全没有 current 状态机的旧部署兼容缺行，一旦 current 状态机存在，缺失、零/多 current或非 active Policy 一律 fail-closed。普通 Reply/Revision/Shadow 与管理发送在 Outbox 前共用该门；缺状态按 Contact 当前态、再按状态机 initial 态回落。Prompt pack bootstrap/reset/启动对齐在状态机写入后复用同源 Policy reconcile，partial 即失败，因而空库初始化仍可用且人工 Policy 不被覆盖。授权服务器 `rs0` 随机库真实用例 1/1：先证明 bootstrap active Policy，再删 `need_discovery` Policy 模拟派生丢行；普通 Reply 与管理发送均命中 `missing_current_operation_state_policy`，Outbox/MCP 维持 0。部署前生产克隆演练证明候选唯一写集为 `operation_state_policies`，精确从 0 补为 9；正式 ELF 从 `d0b7ffc6…e027b` 原子切换为 `155bb77c…2ce67`，PID `2517121`、`NRestarts=0`、内外健康与 69 项静态资源通过。全库压缩备份 39,585,322 字节且哈希复核通过；独立部署后审计确认 9/9 Policy 逐态语义、current partial unique 索引、零活跃队列、静态配置不变和零测试库残留，证据清单 SHA-256 `096a4ab292d5fb6dcd5f95f7cf7d605f73da19ed444491a662bac5b883697868`。匿名 Management 确认端点返回 401，Command/ToolCall/Outbox/MCP/Task/Event 六类计数前后不变，拒绝证据清单 SHA-256 `490e2aa2c065ce35826780bed20de59cba33691e3214461182191d54baecbae9`。

## HC-015：让 Taxonomy 与画像信号只写一次且映射确定

- 来源：SR-045、SR-046、SR-047、SR-061
- 两轮结论：同一未知值被两路径累计，alias可映射到多个 canonical，同轮重复证据可越过门，合并候选又不真正写 alias。
- 推荐：`修复`
- 最小处理：保留单一候选写入点或同 run 幂等；写 alias 时检测 canonical/alias冲突；每 run/dimension 去重；“合并”事务内真正追加 alias。
- 不过度工程边界：不设计通用知识图或重写贝叶斯模型。
- 不处理代价：运营看到的 occurrences、标签可信度和 canonical 映射不可靠。
- 人类决定：`完整修复（2026-07-25 用户确认）`
- 负责人：实现、部署核对与服务器副本集验证：Kiro
- 决定理由：候选 occurrence、alias 归属和跨轮 hit 都是运营与后续策略的事实基础，不接受“仅统计误差”。保持现有 Taxonomy/贝叶斯模型，只收紧持久化责任、唯一身份与每 run 时间语义。
- SR-045/046/047 实施状态（2026-07-27）：`production-deployed / deterministic-verified / replica-set-verified / post-deploy-observed`。SR-045 删除 Decision 阶段 fire-and-forget upsert，候选只在 Gateway 最终决策阶段写一次；SR-046 为每个 typed Taxonomy 版本派生 `identityClaims=[canonical, aliases…]`，m050 在任何写前全表审计 canonical/alias 规范性及 current+active 同 scope 唯一归属，审计通过后才回填所有历史版本；partial unique multikey 索引在数据库层阻止 alias↔alias 与 alias↔canonical 并发冲突，create/patch/candidate approve 的重复键统一返回 409，缓存也对歧义 fail-closed。SR-047 为走势点增加 `sourceRunId`；更新函数防御性按 run/dimension 归并，同值只取最高置信与一个强证据点，冲突值整维零计数，同 run 重试不追加。
- SR-045/046/047 验证：既有 Decision Taxonomy 10/10、贝叶斯红线 7/7、m050 审计 3/3、Taxonomy BSON 1/1、迁移注册 2/2、唯一索引契约 1/1 与格式门继续有效。当前正式后端源码逐字包含上述实现，正式库 m050 为 `applied` 且 `uniq_sys_tax_ws_scope_kind_active_identity` 是带 active/current partial filter 的 unique multikey 索引。部署后在服务器真实 `rs0` 随机库运行完整 mock‑LLM Gateway 1/1：同一 run 的未知 Taxonomy 值只落 `occurrences=1`，重复同值 Bayesian 观测只落一个 history 点、取最高 confidence，并以该 AgentRun 的 `sourceRunId` 锚定。m050/索引红线 1/1：全部历史版本 claims 回填、alias 歧义首写前失败、真实 E11000 阻断第二 active owner 均通过。
- SR-061 实施：Taxonomy 候选选择已有 canonical 时，REST 与 Management 共用的事务 helper 会将候选原始值及请求 aliases 规范化、保序去重并排除 canonical id；若 alias 集发生变化，则以 `_id + version + current_version=true` CAS 退役实际 current，复制其完整运行字段追加新 current 版本，并在同一事务中终结候选。新版本号按该 canonical 全历史最大版本 +1 分配，`previous_version` 精确指向被退役的 current；多个 current 时 fail-closed。已有 alias 已覆盖候选时不制造空版本，但候选仍可在同一事务中正常终结。成功统一返回 200，并以 `mergedIntoExisting=true` 告知前端，不再把已完成的合并伪装成 409。
- SR-061 验证（2026-07-27）：已部署源码的真实 Cookie Router+`rs0` 两条事务红线各 1/1。故障注入先证明字典插入失败会回滚候选 claim、候选保持 pending，修正后重试成功且认证 actor 覆盖伪造 `reviewedBy`；合并路径构造 current v3、历史最高 v9，真实返回 `mergedIntoExisting=true`，生成 v10/previous v3，原始值和手工 alias 持久化，显示名、描述、status、priority、terminal、reactivation 字段保持不变，候选终态与字典指针同事务提交。四条 HC‑015 红线共 4/4，两个随机库已清理，测试库数量前后均为 126；正式 PID `2021387`、`NRestarts=0`、运行中与磁盘 SHA-256 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 一致，健康与 Evolution 关闭状态不变。证据见 [HC‑015 部署后验证记录](production-release-2026-07-27-hc015.md)。

## HC-016：决定 Shadow/Simulation 是否允许任何生产副作用

- 来源：SR-048
- 两轮结论：标称演练路径会写生产记忆和整改队列，且没有执行生产 finalize 硬门；这不是“纯影子”。
- 推荐：`修复为发送与业务状态零副作用；保留明确标记的观测日志`
- 最小处理：传递 `run_mode=shadow`，共享 mutation helper 在该模式拒绝业务写；允许写独立、带 shadow 标签的成本/诊断日志；模拟终态调用同一 finalize。
- 不过度工程边界：复用真实决策链，不复制一套 simulation Agent；无需隔离数据库。
- 不处理代价：运营演练会污染真实记忆、缺口队列和评估结论。
- 人类决定：`零生产业务副作用`
- 实施状态：`已部署 / 确定性验证完成 / 部署后真实 Mongo 全库快照红线通过（2026-07-27）`
- 负责人：实现、本机与服务器部署后验证：Kiro；真实模型/真实 MCP 外部链路不属于本次确定性闭环
- 决定理由/允许的副作用：当前不允许 Shadow/Simulation 写生产记忆、客户状态、知识整改队列、Outbox 或其它业务状态；仅允许写带 `run_mode=shadow` 的独立诊断、成本和评测日志。Simulation 与 Prompt Replay 复用 Reply、Review、独立 ClaimGate、finalize 和状态动作策略；需要 single-shot revision 的未改写草稿只记 `revision_required`，不得冒充可发送。Shadow 使用只读 memory / knowledge route / taxonomy 加载，未知 taxonomy 值只保留内存风险，不物化 workspace 模板、不写候选。
- 验证：`cargo check --tests`、格式与 diff 门通过；Shadow 终态纯单测 2/2、Taxonomy/候选 Live 回归 58/58、前端 LLM 日志契约 9/9 通过。本机真实 Mongo 随机隔离库与部署后服务器 `rs0` 均运行 `simulation_has_no_business_side_effects` 1/1：完整 mock‑LLM Reply→Review→ClaimGate 后，除 3 条 `run_mode=shadow` 成本日志外，全库逐文档不变，预置运营偏好的 `last_used_at` 也未刷新；测试曾真实捕获并促成修复 taxonomy 惰性模板 marker 隐藏写面。服务器测试库 126→126，正式 PID `2021387`、`NRestarts=0`，磁盘与运行中 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。证据见 [HC‑016 部署后验证记录](production-release-2026-07-27-hc016.md)。本结论证明已部署确定性路径零业务写，不冒充真实模型或真实 MCP 外部链路通过。

## HC-017：定义 Evolution 的冻结基线、发布、回滚和自动放量政策

- 来源：SR-049、SR-086、SR-087、SR-088、SR-133、SR-170、SR-171、SR-180
- 两轮结论：proposal不绑定评估基线，旧 proposal可回滚掉新版本，并发 release可重复生效；父开关不能阻止 auto-release；7天指标实际只取20条；正式需求与代码对自动发布互相冲突。
- 推荐：`先决定自动发布政策，再修发布协议`
- 最小处理：proposal 保存 base version/hash和scope；release/rollback用 expected current CAS及唯一 override；父开关一票否决所有副作用；事件/review intent与发布同提交；时间窗服务端聚合。若拒绝 auto-release，删除调用和配置；若接受，正式写入需求、权限和审计。
- 不过度工程边界：不复制 Git 或实验平台；用现有 proposal/version/override 集合增加不可变身份和CAS。
- 不处理代价：旧证据应用到新基线、错误回滚、关闭后仍发布、运营无法判断审批边界。
- 分级业务规则建议：Prompt、Soul、DomainProfile、知识/安全语义和任何“放宽安全阈值”的变更始终人工发布；纯阈值“收紧安全边界”可在证据量、回归门和小流量灰度均通过后自动发布；普通业务效果阈值在未上线阶段先人工，积累生产证据后再逐项加入自动白名单。父开关关闭时任何类型都不得发布。
- 人类决定：`阶段性决定：Evolution 默认关闭；当前全部人工发布。保留按 proposal 类型+方向配置自动白名单的能力，待上线并有真实样本后再启用“安全收紧类阈值”自动发布`
- 实施状态：`已部署 / SR-170、SR-171、SR-180 已完成已发布二进制服务器动态闭环 / HC-017 其余发布协议与真实模型条目仍按各自证据边界开放（2026-07-25）`
- 负责人：实现：Kiro；产品决定：用户确认
- 决定理由/允许自动发布的阈值范围：不同业务逻辑应分级，不能用一个全局布尔统一处理；现阶段无生产证据，不开放自动发布最稳妥且实现最简单。
- 实施摘要：proposal 持久化不可变 `base_revision/released_revision`；Prompt Shadow 按 workspace/account/contact/message 完整作用域读取源数据，并在同一 provider generation、Profile、产品集、Taxonomy 深快照、评估时刻和准备上下文上分别完整重跑 baseline/candidate，历史 run 分数不再作为 baseline；依赖指纹漂移时 fail-closed 为 `shadow_dependencies_changed`。threshold 与 prompt 发布/回滚按 workspace+account+当前 revision 做事务 CAS，回滚仅允许撤销仍为 current 的本 proposal 产物。m040 对历史 threshold override 先全量只读校验，再回填确定性 revision、选举唯一 current；重复 proposal 产物 fail-closed。partial unique 索引锁住每 scope/gate 唯一 current、每 proposal 唯一产物和每 proposal 唯一 protocol-v1 复评 intent。发布产物、proposal 状态、审计事件和复评 intent 同事务提交。自动发布代码政策硬闸固定关闭，管理 API 拒绝开启 workspace 子闸；配置字段仅为未来“类型+方向白名单”保留兼容。近 7 天指标改由服务端完整 168 小时时间窗聚合并返回 coverage，前端不再从 20 条浏览列表推算；coverage 缺失 fail-closed。开关 PUT 仅在服务端返回权威 flag 后更新 UI，失败保持原状态。
- 验证：`cargo check --lib`、`cargo check --test evolution_prompt_shadow` 通过；provider generation 固定、源消息完整作用域、未改写草稿终态、空候选片段字节级 no-op、Taxonomy 深快照隔离等定向单测通过。`cargo check --test m040_evolution_release_protocol`、`evolution_release_redline`、`evolution_rollback_status` 通过；revision 2 项、人工发布政策闸 1 项、m040 helper 1 项 Rust 单测通过；Evolution 前端定向 2 文件/16 用例通过；`npm run build`、`cargo fmt --all -- --check`、受影响文件 `git diff --check` 通过。Docker CLI 在本机不存在，SR-049 的 baseline/candidate 各 Reply→Review→ClaimGate 六调用 Mongo 回归，以及新增 m040 Mongo 幂等/唯一 current/重复产物零写入与 Prompt 事务回归均仅完成编译、尚未执行；这些开放项不因下述 SR-170/171/180 专项而结算。
- SR-170/171 部署后专项（2026-07-25）：使用正式后端 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`，在两个仅回环临时实例和随机库 `wechatagent_test_72fa02d5048e4326a3571e19081f6f4a` 中通过真实 `wa_session` Cookie 中间件执行。SR-170 的自动发布子闸开启请求、env 父闸关闭下人工 release、workspace 父闸关闭下人工 release 均返回 400；两阶段前后 runtime flag/proposal 完整 BSON 逐字一致，threshold override、不可变审计、post-release review 与 release event 均为零。SR-171 在浏览 `limit=20` 时只返回 20 项，但服务端独立 168 小时窗口完整扫描并统计 25 个实验和 25 个 proposal（released=5、rolledBack=3，coverage complete/source/窗口/扫描数均正确）。两个临时单元均 `Result=success/ExecMainStatus=0` 退出，35 项原始证据哈希全部通过；证据库保留，正式 PID `1686295`、重启计数 0、运行中/磁盘哈希、健康及正式库随机 marker 前后不变。证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr170-171-deployed-isolation-20260725T053014Z`。该专项只关闭 SR-170/171，不自动结算 HC-017 其余 SR。
- SR-180 部署后专项（2026-07-25）：使用同一正式后端，在仅回环临时实例和随机库 `wechatagent_test_b06bd78443934d9590409e944befb033` 中把运行进程 `EVOLUTION_ENABLED=true`、`EVOLUTION_AUTO_RELEASE_ENABLED=true` 与 workspace `threshold_auto_release_enabled=true` 全部实值开启，并种入一条 `eligible_for_release` threshold proposal。真实 60 秒 Evolution worker tick 完成后写出 `auto_released_count=0`；候选完整 BSON 逐字不变，threshold override、不可变审计、post-release review 与 release event 均为零。临时单元 `Result=success/ExecMainStatus=0`，32 项原始证据哈希全部通过；成功证据库确认保留，正式 PID `1686295`、重启计数 0、运行中/磁盘哈希、健康及正式库随机 marker 前后不变。证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/sr180-deployed-isolation-20260725T055044Z`。此前 `054558Z` 诊断运行因 `findOne` 的第二参数被误作 projection，轮询脚本未识别已实际完成的两轮 tick；只读诊断确认两轮均为 `auto_released_count=0`，但该次不作为最终通过证据，诊断目录和随机库原样保留。该专项只关闭 SR-180，不自动结算 HC-017 其余 SR。

## HC-018：把 Prompt、Soul 与 Playbook 简化为同一种不可变发布模型

### 2026-07-26 Playbook 发布、默认指针与运营风格服务器验证

- 状态：`working-tree-wired / deterministic-verified / server-rs0-integration-verified / deployment-pending`。AI 生成与优化只追加 `release_status=draft` 候选；联系人绑定、Agent 运行时和默认读取只接受 `published`。人工保存与“设为默认”是显式发布动作。首默认由账号级 partial unique index 原子仲裁；替换已有默认在副本集事务中同时降级旧默认、发布并提升目标。
- 运营风格选择器已有真实持久化语义。直接启用冻结 `expectedAccountId + playbookId`；已托管联系人保存运营风格提交同一 Playbook。服务端在任何 Contact 写入前按 `_id + workspace_id + account_id + release_status=published` 解析，并在最终账号级 CAS 中保存 `playbook_id + playbook_version`；跨账号或 draft Playbook 均零写拒绝。
- 本地确定性验证：`cargo fmt --all -- --check`、`cargo check`、Prompt 守卫 20/20、Playbook 迁移/索引/契约 4/4、前端专项 40/40 与生产构建通过。授权服务器从当前工作树最小源码归档（SHA-256 `1691e0e13844564d672d8adfb8e598db1b32a35a29f2ddb4441d775f2d58e466`）在独立 `/tmp` target 离线编译两个测试二进制，并以 `TEST_MONGODB_URI=mongodb://127.0.0.1:27017/?directConnection=true`、仅回环临时 systemd 单元串行运行 4/4：错误账号/旧版本完整 BSON 零写、draft 发布并原子切默认、跨账号绑定完整 Contact 零写、draft 绑定完整 Contact 零写。
- 服务器证据：两个拒绝用例保留随机库 `wechatagent_test_83b1df4a51de4dee8e36874579cedd5e` 与 `wechatagent_test_930769e2d3e44a99814f32339d21154f`；只读投影确认联系人 `playbook_id=null`、`follow_up_policy=null`，跨账号行和 draft 行保持原状态。正式服务测试前后 PID `1740029`、重启计数 0、运行中与磁盘 SHA-256 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b`、健康响应及 `rs0` 可写主节点状态一致。19 项原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/playbook-release-20260726T1918Z`，清单 SHA-256 `afb1c67d781178cfe0ff16d89847e230e30b153d4396cafa6bc0e01a750be297`，全部复验通过。该结论关闭 SR-070/071/151A 的 Mongo 动态门，但当前工作树尚未正式部署，也未执行浏览器业务验收。

- 来源：SR-053、SR-055、SR-069、SR-070、SR-071、SR-138、SR-139、SR-151
- 两轮结论：Soul/Prompt会物删历史或双指针漂移；Playbook前后端契约坏且AI候选可直接改生产；默认指针非原子；Prompt pack读失败可破坏性重置；语义闸看不见纯删除；风格下拉不提交。
- 推荐：`修复并统一最小发布模式`
- 最小处理：内容 append-only，唯一 active/current pointer以CAS切换；AI生成只落 draft，显式发布；reset仅显式维护且读错误fail-closed；Prompt审查传 before/after；前后端共享DTO并提交实际选择。
- 不过度工程边界：复用三张现有表和现有审核 UI；不建设新 CMS 或全局配置中心。
- 不处理代价：人格/方法论历史丢失、AI草稿直接影响生产、瞬时读错可抹配置、UI显示与运行值分离。
- 人类决定：`待确认（统一发布模式 / 仅修破坏性与契约故障 / 接受单管理员风险 / 暂缓）`
- 负责人：
- 决定理由：
- SR-053 实施：Agent Soul 复用现有 `agent_souls` 集合形成不可变版本流；编辑追加 draft，事务发布以状态/version CAS 归档旧 published 并切换目标，旧版本保留且可重新发布回滚。m042 在索引建立前全量只读校验 scope/version/status，重复版本 fail-closed 零改写，多 published 确定性保留最高版本；唯一版本索引和 partial published 索引锁住运行时不变量。Reply 运行时按 contact workspace 严格读取唯一 published，不再静默回落。启动只补缺失 kind，显式 reset 才追加并发布内置版本且记录认证 actor；列表 DTO 下发版本链与发布审计。前端 PUT 后把发布目标切到返回的新 draft id，失败即清空旧目标。
- SR-053 验证：`cargo fmt --all -- --check`、`cargo check --lib`、`cargo check --test sr053_soul_versions`、Soul/m042 Rust 定向单测 2/2、前端 Soul 身份回归 3/3、`npm run build`、受影响文件 `git diff --check` 均通过。专用测试目标已枚举 5 条 Mongo/副本集用例，覆盖不可变编辑、发布/回滚/重置保留历史、启动不覆盖运营 Soul、并发发布唯一指针、m042 重复版本零改写与多指针选举；本机无 Docker、mongod 和 `TEST_MONGODB_URI`，故动态断言未执行，不记 fully verified。
- SR-055 实施：Prompt 复用现有 `prompt_templates` 集合形成 append-only 版本流；POST/PUT 只追加 draft，手工发布以 Mongo 事务归档旧 current 并提升目标，历史 system/manual/evolution 行全部保留。运行时、contact 读取与版本审计只认唯一 `current_version=true + status=active`；已有历史却缺 current、多个 current、current 非 active 或残留非 current active 均 fail-closed，不再按 contact/locale 对多个 active Prompt 隐式分桶。启动 spec 对齐复用同一追加/事务发布 helper；人工/Evolution current 不被系统覆盖，Evolution 发布/回滚同步切换 status/current。m043 在索引建立前先全表验证，再归档合法旧流的非 current active，歧义或重复版本零写失败。前端保存成功后把发布目标切到 PUT 返回的新 draft id；异常或缺 id 清空旧目标，待确认/语义拒绝保留来源 id 供 force 重试。
- SR-055 验证：`cargo fmt --all -- --check`、`cargo check --lib`、`cargo check --test sr055_prompt_versions`、`cargo check --test prompt_publish_evolution_guard`、`cargo check --test prompt_pack_seeding` 通过；Prompt 发布状态与 m043 scope/parser Rust 定向单测各 2/2，前端保存三态/新 id 接力 6/6，`npm run build` 通过。三条专用 Mongo/副本集用例启动时均在创建 testcontainers 容器前因本机 Docker daemon/命名管道不存在退出，业务断言未执行；该基础设施阻塞不记动态通过或代码失败。当前工作树未部署，SR-055 状态为“确定性已验证，Mongo 动态与部署待办”。
- SR-138/139 实施状态（2026-07-28）：`production-deployed / deterministic-verified / post-deploy-router-rejection-verified / successful-real-model-review-blocked`。Prompt pack 探测只有明确 `Ok(None)` 才授权 bootstrap，读错误原样返回并保持首写前失败；显式 reset 后端严格要求 `RESET PROMPT PACK` 且拒绝未知字段，前端列明替换范围并要求输入同一短语，取消零请求。语义第三闸对所有非 CRLF 等价变化提交完整 `BEFORE/AFTER`，覆盖纯删除、行内改写、重排和重复行删减；Reply system/task 增加最小契约锚，语义审查器自身禁止自然语言修改。
- SR-138/139 部署后验证与边界：正式 `wa_session` Cookie Router 对缺 body、错短语、未知字段三类 reset 请求全部拒绝，Prompt、Soul、Playbook、Domain Config 治理快照逐字零变化；正式运行制品、内外健康与 69 项静态资源通过复验。SR-139 的模型路径已触达且失败时安全降级，但四个可用配置分别遇到 Cloudflare 530/1016、120 秒超时、无成功审查日志和 DashScope `Arrearage`，因此不记成功真实模型判定。显式 reset 仍非跨 Prompt/Playbook/Domain Config 的单事务或可恢复 saga，生产未执行破坏性成功 reset；HC-018 保持开放。完整证据见 [Wave1 生产发布记录](production-release-2026-07-28-wave1.md)，冻结清单 SHA-256 `d16d0ce7f529c279fee16374107eeba990921b6f867135a74a5370a8f62b677a`。
- Playbook 边界更新（2026-07-26）：2026-07-23 的“SR-070/071 仍开放”边界已被上方发布态与默认指针实现取代。SR-070/071 已完成工作树实现、确定性验证和授权服务器 `rs0` 隔离动态验证；剩余边界仅为正式部署与部署后浏览器/业务回归，不把隔离测试冒充生产上线。

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
- SR-058/059/060 部署后验证（2026-07-27）：当前正式 release 已逐字包含关系审核事务、可信 actor 与 m045 周期索引实现；正式库 m045 为 `applied`，`(workspace_id,contact_id) WHERE status=pending` partial unique 索引形状正确。服务器真实 `wa_session` Cookie Router+`rs0` 联合红线 1/1：请求伪造 `reviewedBy` 最终持久化认证管理员；首次审批将建议终态与联系人 `relationship_type` 同事务提交；终态历史不占 pending 槽，下一周期可创建，而同周期第二条 pending 由 E11000 拒绝；联系人 validator 故障发生在建议 CAS 之后时，事务整体回滚，建议仍 pending、审核字段为空且旧画像保持不变。
- SR-058/059/060 证据边界：业务测试 `1 passed; 0 failed`，本轮随机库已删除，测试库集合 126→127→126，正式 PID `2021387`、重启数 0、运行中/磁盘 SHA-256 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096` 与健康状态均不变。原始 systemd 单元 `rc=1` 只因测试成功后的证据 `sha256sum` 未切换到证据目录；没有重跑业务，裁定文件明确保留该后处理失败。冻结清单位于服务器 `audit/hc019-relationship-20260727T124500Z`，SHA-256 为 `03716865cbfa27e089483bb4dca50d64e965d9ea27ba4cdfcaae0a413c0060a6`。本结果只结算 SR-058/059/060 的关系审核路径；SR-057、SR-067、SR-097、SR-169 的独立边界按各自记录推进。
- SR-067 部署与验证（2026-07-27）：`已部署制品观测 / 确定性验证完成 / 服务器真实 Mongo Handler 验证完成`。统一收件箱新增 `suspected_deal` 第九源，collector、source filter 与 summary 共用 `workspace_id + status=pending` 口径；专用富卡展示证据、联系人、置信度和出现次数，并复用 SR-057/058 的事务 approve/reject 与认证 actor。前端收件箱/卡片/API/Store 专项 16/16、TypeScript 与生产构建通过；服务器真实 Handler+Mongo 1/1，证明当前 workspace pending 集合、过滤 inbox 和 summary 一致，终态及外 workspace 均排除。
- SR-067 证据边界：当前正式后端进程含 `suspected_deal`/`suspectedDealReview` 标记，正式前端静态制品含筛选与专用卡片，release 源码与本地逐字一致。服务器无可复用认证会话，故不声称本轮完成生产 Cookie HTTP 交互。测试库 126→126、差集为空；正式 PID `2021387`、重启数 0、二进制哈希与健康均不变。证据目录 `audit/sr067-unified-inbox-20260727T150000Z`，清单 SHA-256 `90965551a934c737231937149bc231d0eef14f79b5b60908535cba0819b062cb`。
- SR-097 实施状态（2026-07-28）：`production-deployed / deterministic-verified / replica-set-router-verified / post-deploy-router-verified`。晋升使用 Lesson `_id` 作为确定性 Chunk `_id`，并持久化 `lesson_promotion + lesson_id` 来源锚；Chunk、Lesson expected-state CAS 与审计事件在同一事务提交。并发/重放收敛到同一 Chunk，已 promoted 的来源关系不一致时拒绝。m055 在任何回填前验证完整存量关系，只回填精确旧配对；Lesson identity unique 与来源 partial unique 索引共同锁住一对一关系，不建立通用审批工作流。
- SR-097 验证：部署前 m055/索引纯单测 4/4、真实 Cookie Router+`rs0` 3/3 继续有效。Wave1 正式切换后 m055 marker 恰好一条、Lesson identity unique 与 promotion provenance partial unique 索引形状正确；正式 Router 中两个并发晋升与后续重放返回同一 Lesson `_id` Chunk，持久事实恰好一 Chunk/一审计、Lesson 为 promoted，外 workspace 不可见。正式 PID `2086752`、零重启、后端 SHA-256 `5df573cf5aef14e5919e13157c5213d58e24219cc424098610fc5ee7f29a558b`，临时数据零残留。详见 [Wave1 生产发布记录](production-release-2026-07-28-wave1.md)；SR-169 仍按其独立浏览器交互边界推进。
- SR-169 实施状态（2026-07-27）：`已部署制品观测 / 确定性前端验证完成 / 认证浏览器交互待办`。共享 `ReviewQueue` 由队列本身以 `getId(item)` 作为直属 React key，条目删除或重排时不再按位置复用上一对象的本地审核草稿；每次成功接纳列表提升 generation，`runAction` 在调用副作用函数前校验 generation、对象仍在当前列表且当前不在刷新，陈旧卡片闭包直接提示重试并零请求。该修复统一覆盖 Taxonomy、领导裁决、Lesson、Chunk、Profile 等复用队列，不逐卡复制身份协议。
- SR-169 验证：正式 release 的 `ReviewQueue` 与三份专项测试源码和本地逐字一致，正式静态制品含陈旧列表拒绝运行标记。真实 `ReviewQueue + TaxonomyCandidateReviewCard` 回归复现 `[A,B]→[B]`，锁定 B 的 URL 与 canonical body；独立旧 generation 回归锁定副作用零调用且无 React key warning。三份专项 17/17、TypeScript、生产构建与 scoped diff 门通过。
- SR-169 证据边界：服务器无可复用认证浏览器会话，因此不声称部署后真实点击交互通过；该纯前端对象身份修复不需要 Mongo 动态断言。正式 PID `2021387`、重启数 0、后端哈希与健康不变。证据目录 `audit/sr169-review-queue-20260727T153000Z`，清单 SHA-256 `5885a0e3e0f3cb17a663f5987cb3980f2823bd71d2c343c38df8de8c0277dbd0`。

## HC-020：限制 Management Agent 的真实副作用边界

- 来源：SR-062、SR-063、SR-064、SR-065、SR-143
- 两轮结论：Management可把裸发送当低风险、默认不强制危险确认、无可恢复提交协议，并能用LLM计划写 staff_confirmed 成交；切账号后还可确认旧账号命令。
- 推荐：`修复（P1）`
- 最小处理：发送只走 typed gateway/outbox；危险工具代码层默认确认；命令保存 account+plan hash+idempotency并以状态CAS执行；staff_confirmed只允许显式管理员动作；切号清 pending command并在确认时校验expected account。
- 不过度工程边界：不重写 Management Agent；只收窄 tool catalog、增加共享确认/提交 helper。
- 不处理代价：绕过发送安全与可靠投递、AI自证成交、崩溃后重复副作用、跨账号执行旧计划。
- 人类决定：`采用修复；保留经过代码分类与显式确认的写工具，不退化为只读总控`
- 实施状态（2026-07-27）：`production-deployed-artifacts-observed / deterministic-verified / post-deploy-isolated-router-mongo-mcp-verified`。
- 决定理由/允许的写工具：所有真实副作用默认确认；只读工具、`media_get` 与 provider 连通测试可直接执行，未知工具 fail-closed。原始 `message_send_*` 不对计划器公布且执行兜底拒绝，主动文本只走生产 Gateway/Review/Outbox。命令冻结 workspace/account/plan hash，确认与否决都校验 account/hash；前端切号清旧计划并隔离迟到响应。正式 release 四份关键源码与本地逐字一致，运行制品含对应协议。
- 崩溃与成交边界：每个工具先落 scoped partial-unique intent，再从 `prepared` CAS 到 `executing`；终态复用，崩溃遗留 `executing` 只收敛 `execution_unknown` 并停止后续调用，绝不自动重放未知副作用。`write_deal_events` 必须由确认端点传入真实 `AuthenticatedAdmin`，LLM 仅起草冻结参数，最终 `staff_confirmed.marked_by` 取认证用户名。
- 部署后验证：服务器真实 `wa_session` Cookie middleware +生产 `api_router` +随机 MongoDB +回环 MCP 红线 2/2；low/无需确认写计划仍待确认，错账号/错 hash 409 且零写，正确绑定只写一条认证管理员成交；stale intent 收敛 `execution_unknown`，联系人零写且 `tools/call=0`。前端 Command Center 专项 14/14；测试库 126→126，正式服务零漂移。证据目录 `audit/hc020-management-20260727T160500Z`，清单 SHA-256 `25b0ead03b764c73cdfa4be1b2454a6812b52fe104353d68782f281e500e4c20`。
- 未完成边界：尚未以真实外部 MCP 目录和远端副作用执行本协议，也未完成真实管理员浏览器切号或杀进程级崩溃恢复演练。本结论不关闭被 Management 调用的 Campaign、知识修复或 Provider 下游自身缺陷。
- 负责人：

## HC-021：使 Campaign 的预览、冻结规格和扇出提交可恢复

- 来源：SR-075、SR-076、SR-077、SR-157、SR-158
- 两轮结论：preview有生产写副作用且能重开completed活动；前端编辑不会更新spec；fanout多步写可半提交；迟到报表可显示错活动；CSV有公式注入。
- 推荐：`修复`
- 最小处理：preview纯计算或显式 draft preview，不改终态；dispatch只消费不可变 campaign spec/version；每目标用唯一 send intent并可补偿/重试；前端按 campaignId/generation接收响应；CSV中和公式前缀。
- 不过度工程边界：不引入营销平台或复杂工作流引擎；沿用 campaigns/campaign_sends/tasks。
- 不处理代价：实际推送使用旧受众、重复或漏任务、completed活动被重开、运营导出错活动或触发CSV公式。
- 人类决定：`修复（已按推荐最小方案实施；不引入新营销平台或队列产品）`
- 负责人：工程实现 / 上线批次复验
- 决定理由：Campaign 已改为 `specVersion + specHash` 绑定确认，preview 为零写纯计算；首次 dispatch 以 CAS 冻结 generation、意图和受众快照。`campaign_sends` 作为 `prepared → enqueued` durable intent，确定性 task 先以不可 claim 的 `committing` 持久化，send 提交后才放行为 `pending`，task worker reconciler 可恢复任一中断点。前端编辑先 PATCH 完整草稿再 preview；报表按 request generation + campaignId 提交；CSV 统一中和公式前缀。
- 部署后验证（2026-07-27）：正式 release 的 Campaign Handler、集成目标、Store 与创建页四份源码和本地逐字一致；服务器真实 Handler/Mongo 完整目标 11/11，覆盖终态 preview 完整 BSON 零写、PATCH 后按新规格预览、dispatch 冻结身份、prepared intent 故障恢复、重复派发拒绝、受众上下限及 workspace/account 隔离。前端 Campaign 定向重跑 33/33；既有 Rust 协议 16/16、前端完整专项 36/36与生产构建证据继续有效。测试库 126→136→126，正式服务零漂移；证据目录 `audit/hc021-campaign-20260727T163000Z`，清单 SHA-256 `bb99b1c1a991a3b93230782ae04a897b2440d2500e718859436a4a1bd796669a`。
- 未完成边界：尚未执行真实 worker 并发与杀进程级恢复、Management 确认链和真实浏览器导出/打开复验；本轮同进程故障注入与 reconciler 恢复不能冒充进程崩溃演练。

## HC-022：收口联系人导入、纳管、画像任务与列表性能

- 来源：SR-081、SR-082、SR-083、SR-084、SR-147
- 两轮结论：重复导入会清空身份；托管与画像任务半提交；旧画像任务可重新纳管已禁用联系人；系统账号可被批量纳管；列表存在N+1。
- 推荐：`修复`
- 最小处理：导入使用patch语义；任务先幂等创建或同事务切managed；画像写回带agent generation CAS；后端统一真人准入；最近消息批量聚合。
- 不过度工程边界：不新建联系人同步服务或画像队列；扩展现有Contact/Task即可。
- 不处理代价：身份数据被清空、无人画像的联系人开始自动回复、已禁用联系人复活、系统账号被误运营、列表随规模变慢。
- 人类决定：`修复（已按推荐最小方案实施；不新建联系人同步服务或画像队列）`
- 负责人：工程实现与服务器部署后隔离验证完成 / 真实 worker 与浏览器复验待办
- 决定理由：导入统一走字段存在性 patch，缺失/null 身份字段不再清空旧值；非真人在共享导入 helper、batch-enable 服务端与 Roster UI 三层 fail-closed。新纳管先写不可 claim 的 `initial_profile_enrollment` durable intent，再以 Contact 版本/隐藏状态 CAS 切 managed，最后释放任务为 pending；worker 在 claim 前恢复 committing intent。画像提交同时校验 managed + enrollment token + task claim generation，disable/hide 先旋转 token 再取消仍占单飞 key 的旧代任务，迟到结果不能复活联系人；旧代 key 的窄崩溃窗口由下次纳管在同一请求内退休并重建。联系人最近入站改为页内一次聚合。
- 部署后验证（2026-07-27）：当前正式 release 的三份后端核心文件与五份前端源码和本地逐字一致，运行中二进制含 enrollment/reconciler/fencing 协议。服务器当前完整 `contacts_batch_enable` 目标机械计数与实际输出均为 13/13，覆盖稀疏导入保值、非真人/错账号零写、durable enrollment intent、task insert 故障、并发单飞、代际旋转与旧画像任务安全终结；前端 Roster、联系人视图和 Contact Store 专项 22/22。测试库 126→139→126，正式服务零漂移；证据目录 `audit/hc022-contacts-20260727T170000Z`，最终清单 SHA-256 `d895a5bd26020001f71fd28143ad2e8148fb6a6fd4f058d6171b959492e45c1a`。
- 未完成边界：真实生产 worker 杀进程级恢复与认证管理员浏览器业务复验仍待执行；本轮同进程故障注入和 reconciler 恢复不能冒充进程崩溃演练。

## HC-023：让 Guide 预览成为服务端可验证的冻结变更

### 2026-07-25 Guide apply protocol v3 收口

- 状态（2026-07-27）：`已部署制品观测 / 确定性验证完成 / Cookie Router+副本集动态通过`。Preview 在服务端把 Contact、OperatingMemory、所绑定 Playbook 与当前 user-operations Domain 的直接基线和已验证写计划冻结为 `GuideFrozenPlan`；candidate hash 同时绑定 workspace/account/contact 与完整计划。旧 Preview 缺 frozen plan/hash 时要求重新生成，不走兼容旁路。
- 授权真相：服务端从冻结计划计算逐字段 `before → after`、真实 scope、共享 Playbook 影响人数和风险提示；前端只把 `authoritativeChanges` 当修改清单。模型 summary/readable/suggested 字段不决定授权范围。共享 Playbook 或 workspace runtime 必须显式二次确认，服务端再从计划独立推导 scope，scope 行被篡改或缺强确认均在 claim 前零写失败。
- 提交协议：Apply 必须携 `previewId + expectedAccountId + expectedContactId + candidateHash`。lease claim、失败回写和事务 finalize 同时匹配 workspace/account/contact/token；Contact/Memory/Playbook/Domain 以冻结版本或时间戳 CAS，事务 guard 让“仅依赖、无业务字段变化”的对象也参与 Mongo 文档写冲突。标签写入权威 `manual_tags`，未知建议字段明确进入 skipped，不再写废弃运行字段。
- 稳定成功语义：业务写、审计事件、Preview `applied` 与 `apply_receipt` 在同一事务提交。成功响应只序列化 receipt，前端另行刷新详情；同 hash 重放重新校验 frozen plan、身份、权威 scope，以及 receipt 的 preview/hash/applied/skipped 字段后返回同一 receipt，不重复应用。迟到 Preview/Apply 仍由 account/contact/generation 隔离。
- 部署后验证：六份生产/前端文件与正式 release 逐字一致；release 与本地测试整文件因后续其它用例不同，但 Guide 红线函数块 418 行逐字一致，SHA-256 同为 `a3cac908ded91e51849079e943e7616e64d7919b204b25bc05ff92a637e4cd85`。服务器真实 `wa_session` Cookie Router+`rs0` 精确红线 1/1：错账号、错 hash、缺强确认均在 claim 前完整零写；审计故障使 Contact/Memory/Playbook/Preview 整体回滚，修正后成功，同 hash 重放返回相同稳定 receipt。前端 3 文件 37/37；测试库 126→127→126，正式服务零漂移。证据目录 `audit/hc023-guide-20260727T173000Z`，最终清单 SHA-256 `ce1fc888297e828012a9f362ab4da8cfe34e698f7a6e88ee454239503d9d9d7f`。
- 未完成边界：真实管理员浏览器跨账号、强确认和迟到响应交互仍待复验；本用例未写 `domain_runtime_parameters`，不结算 SR-094 的专用 runtime 参数 Mongo 动态门。

- 来源：SR-091、SR-092、SR-093、SR-095、SR-150
- 两轮结论：旧preview可覆盖新配置，标签写到废弃字段，副作用范围由模型自报，commit后响应失败会诱导重试，前端迟到A预览可应用到B。
- 推荐：`修复`
- 最小处理：preview保存对象id、base versions/hash和服务端计算diff；apply校验expected contact/account和基线；写权威字段；成功返回稳定receipt；前端用generation绑定当前对象。
- 不过度工程边界：不做全库快照或通用变更管理平台；冻结直接受影响对象即可。
- 不处理代价：人类确认的内容与真正应用对象/范围不同，已提交动作被误判失败并重试。
- 人类决定：`按推荐最小冻结协议修复（已部署制品观测并完成副本集红线）`
- 负责人：实现、确定性与服务器副本集验证：Kiro；真实浏览器交互复验：待办
- 决定理由：保留单联系人、共享 Playbook 与 workspace runtime 三类 Guide 能力，但授权范围必须由服务端验证后的冻结计划决定；不接受模型自报 scope、旧候选兼容应用或提交后重建易失败响应。

## HC-024：选择 Chunk 编辑的权威协议并修复越权与伪回滚

- 来源：SR-096、SR-101、SR-102、SR-103、SR-104、SR-105、SR-106、SR-107、SR-108、SR-113、SR-114
- 两轮结论：通用PUT/Patch/Split/Merge可绕人审或跨租户写active+verified；多对象动作半提交；revision不是完整快照却被当回滚；前端契约错误；物理CRUD绕历史；软锁既不授权又误导。
- 推荐：`修复（P1）`
- 最小处理：所有mutation强制从服务端保留tenant/审核字段，AI来源只能draft+needs_review；revision保存可恢复snapshot或基于正向补丁重放；split/merge在事务或staging后一次commit；前端共享DTO；软锁明确advisory，若需要互斥再让写端校验token。
- 不过度工程边界：不重写Wiki或上CRDT；Mongo事务/expected version与不可变revision足够。
- 不处理代价：跨租户/绕审核知识进入生产、回滚恢复错误内容、半提交归档源、历史审计失真。
- 人类决定：`按推荐 P1 分批修复；HC-024 后端实现已进入正式 ELF 并完成部署后副本集矩阵，剩余边界为 SR-096 advisory-only 前端与 SR-105 共享 DTO 前端的正式切换`
- 负责人：后端实现、正式源码身份与服务器副本集复验：Kiro；前端生产切换：待用户明确授权
- 决定理由：SR-096 后端 presence 已完成独立部署后动态闭环；SR-101/102/103/104/106/107/108/113/114 与 Catalog Worker 的 SR-115 又以正式 ELF `f0ead4f7…cde9b` 的226项构建输入和4个同源测试二进制完成18/5/2/4、合计29/29条真实 `rs0` 红线。覆盖字段投影与强制待审、Split/Merge事务、完整快照回滚、repair事件原子性、committed-only auto-verify、Document/Chunk CAS与软归档、revision/catalog intent原子性，以及 Catalog lease/generation fencing。29个 UUID 随机库均返回删除成功，数据库清单和正式 PID `2196929`、重启数0、ELF与健康响应前后逐字一致；最终归档 SHA-256 `b30cd56e…ff3c`。正式前端尚未切换，故 SR-096/105 的前端边界仍开放。

## HC-025：为外部摄取、Chat apply 与 Catalog Worker 补最小可恢复协议

- 来源：SR-109、SR-111、SR-112、SR-115、SR-117
- 两轮结论：任意摄取 URL 可形成持久 SSRF；Chat apply 与导入 Apply 可重复或半提交；Catalog/Ingest Worker 缺 lease generation，崩溃后可永久卡住或用旧结果覆盖新 checkpoint。
- 推荐：`先封 SSRF，再补现有任务的 claim/finalize`
- 最小处理：摄取 URL 统一执行 scheme、DNS/IP、重定向逐跳和响应大小限制；Chat/Import apply 以 turn/import id 原子认领并用业务唯一键提交；Catalog/Ingest 在现有行增加 owner、generation、lease、attempt，heartbeat/finalize 均以 generation CAS。
- 不过度工程边界：不引入 Kafka、通用工作流引擎或独立抓取平台；沿用现有 source/job/session/revision 集合。
- 不处理代价：内网资源可被后台抓取，知识对象重复或半提交，目录永久陈旧，旧抓取结果覆盖运营新配置。
- 人类决定：`按推荐分批修复；SR-109/111/112/115/117 均已进入正式 ELF或与正式226项构建输入同源，并完成部署后真实公网/副本集动态闭环；HC-025 当前协议范围已闭环`
- 负责人：实现、正式源码身份、真实公网与服务器副本集复验：Kiro
- 决定理由/允许的摄取网络范围：摄取仅允许解析结果全部为公网单播地址的无凭据 http(s)；保存与每次请求/重定向均重验并固定 DNS，禁代理，正文上限4 MiB，未引入内网 allowlist或测试后门。SR-111 正式同源矩阵4/4覆盖 AI草稿、并发/重放 exactly-once、错身份零写与陈旧 snapshot；SR-109 真实公网3/3覆盖成功摄取、跨域重定向、公开 DNS 名解析到 `169.254.169.254` 后请求前拒绝、错误 Content-Type及超4 MiB拒绝。公共重定向器对私网目标主动返回403，故该结果没有被冒充为“第二跳私网30x”证据；逐跳重解析仍由正式源码循环与确定性门绑定。SR-112/117 的5/5与4/4归档为 `da214eb1…416a`，SR-115 的4/4归档为 `b30cd56e…ff3c`，SR-109/111 联合归档为 `9ff6f55b…ef89`。所有异常测试库均按冻结基线精确回收，最终正式 PID `2196929`、重启数0、ELF `f0ead4f7…cde9b` 与健康响应前后不变；两次失败尝试也保留在联合归档中。

## HC-026：把评测金标和预算变成真实评测输入

- 来源：SR-098、SR-099
- 两轮结论：active 场景缺 ground truth 时被当作零分真值；评测预算读取生产 run，空闲时恒零、并发时受污染。
- 推荐：`修复`
- 最小处理：场景激活前校验最小 ground-truth schema，缺失时标 `unscored`；评测执行在自身上下文累计 token/call 预算并返回明确 degraded 状态。
- 不过度工程边界：不建设新评测平台或统一所有业务公式；只修场景准入和本批预算归属。
- 不处理代价：评测分数和提前终止都不能代表被测行为，运营可能据此错误发布或否决配置。
- 人类决定：`按推荐最小修复；缺真值不评分，预算只认本评测私有 RunBudget`
- 实施状态：`已正式部署 / 前端与 Linux 编译通过 / 正式 Cookie Router + rs0 隔离红线 3/3 / 部署后运行态复验通过（2026-07-28）`
- 负责人：实现、隔离复验、正式部署与证据封存：Kiro
- 决定理由：场景保存与运行共用当前 DomainProfile 的公式闭集；active 必须具备每项有限 0..10 数值金标，未显式状态且金标不完整时只存 draft，存量脏 active 在任何模型调用前标 `unscored` 并排除均值。场景查询仅允许全局或请求账号，显式 Contact 也以 workspace+account 绑定。Simulation 保留原公共签名，并新增返回私有 `RunBudgetSnapshot` 的内部入口；评测对成功和业务失败都累计本子 run 的 reported token/call，usage 未报告时不估算为 0，而是返回 `evaluation_budget_usage_unknown` 并停止后续场景。共享生产 `agent_run_logs` 已完全退出预算控制面。
- 验证与开放边界：前端场景专项 5/5、TypeScript+Vite 生产构建、Linux 编译及正式 Cookie middleware + `api_router` + Mongo `rs0` 随机库红线 3/3 通过：① active 缺完整金标在首写前 400 且零写；② 生产 run 不污染本评测，汇总严格为本次 Shadow 3 calls / 45 tokens；③ 首次 LLM 失败记 1 次 unknown usage 并停止第二场景。正式后端已切换为 SHA-256 `f0ead4f7…cde9b`，PID `2196929`、`NRestarts=0`，内外健康与 69 项静态资源通过；部署后归档 SHA-256 `5f5f53e9…9fd9`。确定性 mock LLM 只证明预算归属和失败计费不变量，不冒充真实模型质量验证。详见 [HC-026 / m039 发布记录](production-release-2026-07-28-hc026-m039.md)。

## HC-027：统一 Knowledge Cockpit 的对象身份、响应契约与实时收敛

- 来源：SR-129、SR-130、SR-132、SR-141、SR-173
- 两轮结论：自动校验参数命名错误而被静默忽略；审核对话未绑定当前 Chunk；默认队列类别和筛选错误；WebSocket lagged 与 SSE gave-up 都会让界面长期停在旧事实。
- 推荐：`修复`
- 最小处理：前后端共享 DTO并拒未知字段；审核请求绑定 chunkId/expectedVersion；修正队列枚举/filter；收到 lagged 就重拉对象；SSE 以连续失败计数并在 gave-up 后有界轮询任务详情。
- 不过度工程边界：保留现有 HTTP+WS+SSE，不换实时协议、不引入前端数据平台。
- 不处理代价：运营设置不生效、审核错对象、归档数据混入队列、长任务已经完成但页面永久显示旧状态。
- 人类决定：`按推荐修复；各 SR 独立结算，不以部分上线关闭整个 HC-027`
- 实施状态：`已闭环（SR-129/130/132/141/173 均已部署并完成对应动态复验，2026-07-28）`。SR-129、SR-130 已于 2026-07-27 完成正式切换与部署后复验；SR-132、SR-141 于 2026-07-28 随 Wave1 正式切换并完成正式 Router/Mongo/WebSocket 复验。当前正式 69 项前端制品逐项与 Wave1 冻结清单一致，并已用真实 Chrome 补齐 SR-141 WebSocket `lagged` 与 SR-173 SSE gave-up 浏览器故障路径，因此不再重复切换前端。
- SR-129 实施与验证：Cockpit 只发送 `confidenceThreshold/humanAuditSampleRate/limit`；后端 camelCase DTO 使用 `deny_unknown_fields`，未知或旧 snake_case 字段不再静默回落默认值。前端回归锁定实际请求体，后端 DTO/边界回归与生产构建通过。
- SR-130 实施与验证：审核对话冻结 `accountId + chunkId + expectedUpdatedAt + operation=update`，服务端响应只以顶层 `targetChunkId/expectedUpdatedAt/draftPreview` 为权威；目标或版本漂移时前端拒绝展示候选。Apply 在事务内按冻结 `updated_at` 做 OCC，陈旧快照返回 Conflict，Chunk、revision、成功审计及 assistant receipt 全部零写。授权服务器 `rs0` 精确红线 1/1 通过。
- SR-132 实施与验证：专用 review queue 由服务端绑定当前 workspace、`user_operations` 和未归档 `draft|active`，下发可重叠 facets/counts/effectiveFilter；维度筛选只精确匹配 active DomainProfile coverage 的 key/display name/可配置 aliases，Profile 内规范化主题全局唯一，未知维度 400。前端不再自行分类。前端专项 3/3、行业配置专项 20/20、Rust 单测 2/2、真实 Cookie Router+本地随机 Mongo 红线 1/1与生产构建通过。
- SR-141 实施与验证：WebSocket `lagged`/`revised` 统一发集合失效；Inspector、知识树和评审队列先撤下旧快照，再以 generation fence 和合并重拉恢复，连续失效最多一个请求在途并保留一次尾随重拉，失败不回显旧事实。本地写入口复用同一事件；定向红线覆盖事件桥、去并发、尾随恢复和失败可见性。真实 Chrome 对与生产逐文件一致的 69 项制品建立 1 条 WebSocket，注入 2 次 `lagged`：旧评审卡在重拉完成前即撤下，503 后仍不回显，第二次失效后恢复为新快照；队列读取严格为 3 次。
- SR-129/130 部署：最小候选仅包含 3 个后端与 2 个前端生产文件；后端 SHA-256 `c98f24a34404cd39bc5427cdfe4c25af84a984fc820e27867cb75f772e7a54ab`、前端首页 `9c60cad33e403b12478c6fea263e4e6b5ccd5cbd7253fc012f6385313d9c1b0f`。切换后三次活动队列均为 0、migration 台账保持 55 条、PID `1942220`、`NRestarts=0`、Evolution 关闭；正式路径随机库严格冒烟通过双健康、69 项静态资源和目标库零变化。完整证据见 [SR-129/SR-130 生产发布记录](production-release-2026-07-27-sr129130.md)。
- SR-132/141 Wave1 部署：正式后端 SHA-256 `5df573cf5aef14e5919e13157c5213d58e24219cc424098610fc5ee7f29a558b`、PID `2086752`、零重启，内外健康与 69 项静态资源通过。正式 Router 验证 pricing/capability 精确分流、未知维度 400、外 workspace 空队列；Chunk create/patch/lock/unlock 原始 WebSocket 事件均到达。后续真实 Chrome 故障注入已补齐浏览器 `lagged` 收敛边界；浏览器探针使用生产同构静态制品和硬隔离、契约形状的内存 API，不冒充生产数据写入。详见 [Wave1 生产发布记录](production-release-2026-07-28-wave1.md)及 `audit/hc027-final/final-adjudication.json`。
- SR-173 实施：共享 SSE 重连器在原生 `open` 或业务事件后重置连续失败预算，隔离旧 EventSource 迟到事件；TaskRail 显示重连/放弃状态，每个 turn 回读权威任务详情，gave-up 或无 EventSource 时执行 5 秒一次、最多 12 次的有限轮询，终态、切任务与卸载均立即停止。
- SR-173 验证：状态机与 TaskRail 专项 13/13、Knowledge 相关回归 18/18、前端生产构建和定向差异门通过；覆盖成功重连不累计断线、连续建连失败才放弃、terminal close 零重连、turn 更新主进度、轮询收敛终态及上限耗尽后停止。真实 Chrome 对当前正式同构制品连续制造 7 次 EventSource 建连失败，TaskRail 严格读取 2 次权威详情并收敛到“已完成 / 1/1 步”，终态后额外轮询为 0。正式 69 项制品本已包含该实现，故无需重复部署。
- 负责人：SR-129/130/132/141/173 实现、验证、部署与浏览器闭环：Kiro
- 决定理由：对象身份、冻结版本、权威评审投影和实时失效已分批上线；正式 Router/Mongo/原始 WebSocket 证据与生产同构静态制品上的真实 Chrome 故障注入现已互补闭合。两份浏览器探针均以硬网络隔离运行、`productionRequests=0`，不改写生产数据；最终裁定见 `audit/hc027-final/final-adjudication.json`。

## HC-028：让 Digest 与 Knowledge Task 只报告真实、可恢复的结果

- 来源：SR-120、SR-121、SR-122、SR-123、SR-125
- 两轮结论：Digest 预算配置不生效且失败重算会覆盖成功卡；Knowledge Task 无 lease/幂等 step并把确定失败包装成成功；派工也未绑定运营选中卡片。
- 推荐：`修复`
- 最小处理：Digest 使用现有 RunBudget配置，先生成新版本成功后再切 current；Task 加 owner/generation/lease，step按 task+step id 幂等并返回结构化 outcome；创建任务必须验证 cardId/version/hash。
- 不过度工程边界：复用现有 Digest/Task/Card 集合，不新建任务平台；不要求所有 LLM 质量问题机械判定。
- 不处理代价：成功日报被空卡覆盖、任务永久 running或重复改库、失败被显示为成功、模型可派发非运营所选对象。
- 人类决定：`修复（已部署并完成部署后回归）`
- 实施状态：`已修复、已部署、已完成部署后真实模型业务回归（2026-07-27）`。部署前的确定性与隔离动态证据继续有效：SR-121 快照恢复 2/2，SR-122 的 lease/fencing/恰好一次及账号域红线 1/1，SR-123 完整 Worker 3/3，SR-125 真实 Cookie Router 3/3；隔离候选 HC-028 硬门为 `1 passed; 0 failed`（379.62s），清单 SHA-256 `bba45706c94aa4f3d770908ede815e8954008bae3188473edf11492a2b709029`。正式切换将后端 SHA-256 从 `3a7d9bab07cc6ff70bfe6771fdff78bcda730c8819ca9a188b743948f4f4980b` 更新为 `f4863fa4401ead96a2c8cecbcadfe6328474d5020d22b115269abf67196119e8`，前端 69 项逐文件校验通过，m050～m054 在生产数据副本两轮幂等后于正式库全部 `applied`；切换后 PID `1923420`、`NRestarts=0`、健康 `ok=true/evolutionEnabled=false`。部署后二进制通用烟测在全新随机库完成迁移、仅回环启动与 69 项静态资源逐字验证；随后从已部署源码重新编译的唯一 HC-028 硬门以显式未激活的 NVIDIA Llama Provider 运行，结果 `1 passed; 0 failed`（60.94s），能力证据 `llm_calls=2 / artifacts=6 / assertions_run=18 / verdict=pass`，Digest 1 卡、候选封印、Task completed、committed 两字段 repair patch、原 Chunk 未自动应用且 MCP 零请求。成功随机库自动清理，测试库计数前后均为 101；正式 active Provider仍恰好 1 条且未切换。部署后证据 12 项清单 SHA-256 为 `8337dad337b8454852c1853c3bc20a8400017719d406a0fd2fdb348513ed3d9b`，因此 HC-028 无剩余部署门。
- 负责人：Kiro（实现、验证、部署与证据封存）
- 决定理由：复用现有 Digest、Chat、Task 与 Mongo 事务/租约协议关闭虚假成功、快照覆盖、重复副作用和越权派工，不引入第二套任务平台；部署前后均以真实入口、真实 Provider、随机业务库和零 MCP 副作用证明闭环。

## HC-029：给主动触达与 ImportJob 复用同一最小所有权模式

- 来源：SR-135、SR-136
- 两轮结论：原 Planner 没有持久业务 intent，串行或并发 tick 可重复触达并突破每日配额；ImportJob 的旧 worker/旧 scanner 也可覆盖新执行。两项现均按最小所有权模式修复、通过真实副本集红线并完成正式部署。
- 推荐：`在启用这些 Worker 前修复；未启用可暂缓但保留上线前门`
- 最小处理：SR-135 已为 Planner 六段、Cold Contact 与 Silence Signal建立稳定业务幂等 identity，并在同一事务内提交任务/信号、审计事件和数据库日桶配额；SR-136 已完成 claim generation/token、heartbeat/progress/finalize/reclaim fencing 与 legacy 迁移。
- 不过度工程边界：不建设中央调度系统；沿用现有 Planner intent/AgentTask/ImportJob 数据面。
- 剩余代价：代码与部署门已关闭；Planner、Cold Contact 与 Silence Signal 仍按原配置保持关闭。未来启用属于独立运营决策，需按实际账号、配额和发送政策另行确认，不由本次部署自动开启。
- 人类决定：`按最小所有权模式修复；经真实副本集红线后再正式部署/启用`
- 实施状态：`已修复、已完成真实副本集验证、已正式部署并完成部署后闭环（2026-07-28）`。SR-136 的迁移幂等与 ABA fencing 红线 2/2；SR-135 首次真实并发运行暴露跨集合探测跨过 commit 的短暂 `1/2` 误判，改为同一 Mongo `snapshot` 只读事务后 7/7 红线通过，126 个历史测试库前后逐字一致。release 候选 SHA-256 `11d9b6fd943eb67b48f9a4b5d4fa13c2e50e1612899c77d7478d7723cdb36954` 先在随机库完成迁移、双回环启动、五类空队列和 69 项静态资源逐字冒烟，再以全库压缩备份和自动回滚脚本停服切换。部署后 m056 为 `applied`，`proactive_daily_quotas_expires_ttl` 为 TTL 0 秒，五类活跃队列均为 0，PID `2166141`、`NRestarts=0`，内外健康一致；证据清单 SHA-256 `305a1f3caa6747a2bdbf4fc873c04cc8de0d1be17e860efbae60c2bb94231013`。
- 负责人：Kiro（实现、真实副本集验证、正式切换与证据封存）
- 决定理由/当前开关与副本数：当前仍为单实例停服切换；全库备份 `39,608,276` bytes 与旧 ELF 均保留。Planner/Cold/Silence 三个开关未设置并按默认 `false` 运行，启动日志明确记录 Cold/Silence disabled，且不存在 planner/worker loop started；实现使用数据库事务、snapshot 探测与持久幂等，不以单实例作为正确性豁免，也未引入中央调度系统。

## HC-030：用一个前端身份守卫消除切账号后的跨对象写错

### 2026-07-24 deterministic closure addendum (SR-142/145/146/149/153)

- Status: `working-tree-wired / deterministic-verified / local-real-mongo-pending / deployment-pending`.
- SR-142: contact snapshots now carry `dataAccountId + requestGeneration`; account switches hide stale rows immediately and late A responses cannot replace B.
- SR-146/SR-149: contact detail state carries account/contact/generation identity. Six detail writes freeze `expectedAccountId`; REST handlers pre-read the exact account and repeat `_id + workspace_id + account_id` matching at the final Mongo write. Management injects the frozen command account for its three shared handlers. Manual-tag drafts freeze the Contact entity and stale saves issue no request.
- SR-153: pool enrollment freezes `source=pool + accountId + contactId + wxid`. The server validates the complete batch before Playbook, Contact, Task, or Audit side effects, ignores client identity metadata, and persists `expected_contact_id + expected_contact_updated_at + allow_contact_insert=false` into the durable enrollment intent. A pool request cannot reuse a roster/upsert-capable intent; final OCC failure is Conflict. Roster enrollment remains explicitly `source=roster` and may create a Contact.
- SR-145: AccountLogin now consumes the published `session_id / qr_data_url / login_page_url` contract. Missing `session_id` fails closed. Polling freezes alias plus a local generation, so cancel, unmount, or a newer login drops late success/error responses and does not trigger sync.
- Verification: frontend TypeScript passed; account-login contract tests 3/3; account/contact/user-ops focused tests 43/43; Rust contact route unit tests 21/21; `cargo test --lib --no-run`, `cargo check --test contacts_batch_enable`, and `cargo fmt --all -- --check` passed. Mechanical audit found both production batch-enable callers carry `source` and all six detail PUT callers carry `expectedAccountId`.
- Dynamic boundary: `pool_candidate_from_other_account_is_conflict_and_zero_write` and `update_manual_tags_wrong_account_is_conflict_and_zero_write` compile, but were not executed because Docker is unavailable and the local MongoDB Windows service was already Stopped. The service was not started or modified. These Mongo redlines and deployment/browser/MCP verification remain pending.

- 来源：SR-142、SR-144、SR-145、SR-146、SR-148、SR-149、SR-153、SR-155、SR-159、SR-160、SR-161
- 两轮结论：联系人、密钥、Playbook、标签、运营池、任务、Outbox、素材和成交等状态都可能在切号后保留 A 对象，并通过当前 B 页面发出合法请求；扫码登录另有确定字段漂移。
- 推荐：`修复为共享模式，并优先处理密钥、成交、素材和Outbox`
- 最小处理：所有账号私有 query key/state包含 accountId；切号清 selection/draft/secret/pending，异步响应提交前校验 account+object generation；动作带 expected account，后端复核；扫码登录改用共享 DTO。
- 不过度工程边界：不重写全部前端 Store；提供一个 account-scoped request/state helper，再逐页迁移这 11 个已证实入口。
- 不处理代价：管理员在 B 上下文修改 A 的密钥、生产方法论、标签、素材、任务、待发消息或高可信成交。
- 人类决定：`待确认（全部修复 / 先修高危写动作 / 接受单管理员低切号频率 / 暂缓）`
- 负责人：
- 决定理由/账号切换是否为正式工作流：
- SR-144 实施状态（2026-07-23）：`已修复并本机真实 Mongo 验证 / 尚未部署`。MCP 密钥表单同时绑定账号记录 ObjectId 与不可变业务 `accountId`；父组件按两者设 React `key`，组件自身也在 scope 变化时销毁密钥、Base URL、成功/错误和 busy 状态，并用 generation 隔离迟到响应。保存请求使用冻结的记录 ID 作为 URL、冻结的业务账号作为 `expectedAccountId`，后端以 `_id + workspace_id + account_id` 原子匹配，避免同 workspace 内旧秘密草稿写入另一账号。
- SR-144 验证：McpKeyForm 专项 3/3、TypeScript `tsc --noEmit`、`cargo check --test account_security_integration` 通过。普通用户权限独立 MongoDB 8.0 随机库直调真实 handler 1/1 通过：URL 指向 `acct_a` 记录而正文冻结 `acct_b` 时返回 Conflict，`acct_a.mcp_api_key` 保持 `OLD_KEY`。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`；该结论只关闭 SR-144，不关闭 HC-030 其余入口。
- SR-148 实施状态（2026-07-23）：`已修复并本机真实 Mongo 验证 / 尚未部署`。UserOps Store 为 Playbook 列表保存 account scope 与请求 generation；切号在请求发出前清空旧列表、编辑 id、冻结账号/版本及草稿，A/B 乱序响应只允许当前 generation 提交，视图 scope 不一致时不渲染写面板。保存、优化、设默认从所选实体冻结 `accountId + expectedVersion`，创建/生成的迟到响应也不得回填新账号界面。后端列表/创建先校验账号归属，编辑、优化、设默认统一按 `_id + workspace_id + account_id + version` CAS；Optimize 在 LLM 返回后仍用初始版本提交。Management 上下文下发 Playbook id/account/version，执行端强制绑定命令账号，四类 Playbook 写均按真实生产副作用列为需确认。
- SR-148 验证：`cargo check --lib`、`cargo check --test playbook_scope_integration`、`cargo fmt --all -- --check`、`git diff --check`、TypeScript `tsc --noEmit` 通过；UserOps Store 专项 14/14 通过，覆盖 A 慢/B 快和切号后旧编辑零写调用。普通用户权限独立 MongoDB 8.0 随机库直调真实 handler 1/1 通过：错账号编辑与旧版本设默认均返回 Conflict，拒绝前后 `operation_playbooks` 完整 BSON 文档一致。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`；测试临时目录因执行策略拒绝递归删除而保留，未触碰系统服务数据。该批当时只关闭 SR-148，不关闭 HC-030 其余入口；SR-070/071 随后已于 2026-07-26 按 HC-018 的发布态协议完成授权服务器 `rs0` 动态验证，见本节上方 Playbook 边界更新。
- SR-155 实施状态（2026-07-24）：`已修复并本机真实 Mongo 验证 / 尚未部署`。Operations Store 把任务、事件、复核、运行日志、LLM 成本、loading 与请求 generation 绑定当前账号；账号切换以 React `key` 重挂载工作台，并在同一渲染帧隐藏旧快照，A/B 乱序响应不得覆盖当前账号。任务 DTO 显式返回实体 `accountId`；立即复核与取消动作接收完整任务实体，只有当前账号、页面账号、快照账号与任务账号一致才发请求，正文冻结 `expectedAccountId`。后端立即复核使用 `_id + workspace_id + account_id + 可认领状态` 原子 claim，取消使用 `_id + workspace_id + account_id + 可取消状态` 原子迁移；Management 工具从已冻结命令账号构造同一请求，不信任模型自报账号。普通后台 worker 的调度 claim 保持不变。
- SR-155 验证：Operations Store + Operations 页面专项 25/25、TypeScript `tsc --noEmit`、`cargo check --test review_task_now_claim`、`cargo fmt --all -- --check` 与受影响文件 `git diff --check` 通过。普通用户权限独立 MongoDB 8.0 随机库直调真实 handler 1/1 通过：对 `account-a` pending 任务以 `account-b` 分别执行立即复核和取消，两条路径均返回 Conflict；拒绝前后任务及其关联 pending Outbox 完整 BSON 文档一致。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`。该结论只关闭 SR-155，不改变后台 worker 调度协议；工作树尚未部署。
- SR-159 实施状态（2026-07-23）：`已修复并本机真实 Mongo 验证 / 尚未部署`。Outbox 面板把列表快照、loading/error 与请求 generation 绑定当前账号；切号同一渲染帧隐藏旧行，A/B 乱序响应不得覆盖当前账号。取消动作冻结条目账号与 generation，确认弹窗打开后若切号则动作失效；POST 正文携 `expectedAccountId`。REST handler 与 Management 工具共用的取消核心都接收冻结账号，pending 原子取消和 in-flight 取消请求两个分支均以 `_id + workspace_id + account_id + status` CAS。
- SR-159 验证：OutboxPanel 专项 6/6、TypeScript `tsc --noEmit`、`cargo fmt --all -- --check`、`cargo check --test outbox_scope_integration` 与受影响文件 `git diff --check` 通过。普通用户权限独立 MongoDB 8.0 随机库运行真实取消核心 1/1 通过：URL 指向 `account-a` 的 pending 与 in-flight 条目、expected account 为 `account-b` 时均返回 Conflict；pending 保持原状态，in-flight 的 `cancel_requested/cancel_reason/cancel_requested_at` 均不变，且 `outbox_canceled/outbox_cancel_requested` 审计零新增。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`。该结论只关闭 SR-159；SR-066/HC-010 的远端发送边界竞态仍独立开放，工作树尚未部署。
- SR-160 实施状态（2026-07-24）：`已修复并本机真实 Mongo 验证 / 尚未部署`。Content Store 把列表快照、请求 generation 与新增草稿绑定当前账号；账号切换以 React `key` 重挂载工作台，销毁筛选、文件、编辑草稿及旧 busy/error，A/B 乱序响应不得覆盖当前账号。每个写动作接收完整资产实体：账号私有资产冻结 `expectedScope=account + expectedAccountId=<实体账号>`，workspace 共享资产冻结 `expectedScope=workspace`，界面明确显示“全账号共享”。后端审核、元数据修改、换文件、启停、删除五入口复用同一 scope DTO/filter；私有项最终 CAS 匹配精确 `account_id`，共享项只匹配 `account_id:null`。换文件在暂存字节前校验 scope，并在最终更新、发布失败回滚中保留同一 scope 与原文件并发 CAS。
- SR-160 验证：Content Store + ContentAssets 专项 15/15、TypeScript `tsc --noEmit`、`cargo check --test media_asset_crud_integration`、`cargo check --test annotation_quality_gate_integration`、`cargo fmt --all -- --check` 与受影响文件 `git diff --check` 通过。普通用户权限独立 MongoDB 8.0 随机库直调真实 handler 1/1 通过：对 `account-a` 私有资产分别以错误账号审核、伪装 workspace 修改元数据、错误账号启停、伪装 workspace 删除，四条路径均返回 Conflict；拒绝前后完整 BSON 文档一致，`media_asset.reviewed` 审计零新增。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`。该结论只关闭 SR-160，不改变 workspace 共享资产产品契约；工作树尚未部署。
- SR-161 实施状态（2026-07-23）：`已修复并本机真实 Mongo 验证 / 尚未部署`。成交页只认当前账号下的联系人选择；账号变化在同一渲染帧隐藏旧成交表单，并清空选择、草稿、记录、错误和 busy。联系人列表及成交记录请求以 account/contact generation 提交，迟到响应不得恢复旧选择。人工成交/退款正文携冻结的 `expectedAccountId`；后端在事件构造前以联系人 `_id + workspace_id + account_id` 精确预读，并把 `account_id` 冻结进 `PreparedOutcomeEvent`，普通与事务持久化的最终 `$push` CAS 都复核同一身份。
- SR-161 验证：Products/Deals 前端专项 2/2、TypeScript `tsc --noEmit`、`cargo fmt --all -- --check`、`cargo check --test deal_event_scope_integration` 与受影响文件 `git diff --check` 通过。普通用户权限独立 MongoDB 8.0 随机库直调真实 `add_deal_event` handler 1/1 通过：URL 指向 `account-a` 联系人而正文冻结 `account-b` 时返回 Conflict，联系人 `outcome_events` 与 `agent_events` 均零新增。独立 `mongod` 已关闭，系统 MongoDB 服务保持原先 `Stopped`。该结论只关闭 SR-161，不关闭 HC-030 其余入口；工作树尚未部署。

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
- SR-165 实施状态（2026-07-25）：`已部署 / 确定性验证完成 / 服务器真实 Router+Mongo+Registry 拒绝路径动态通过 / 合法许可成功热切与真实模型链路待复验`。active Provider 的测试成功响应签发 10 分钟一次性 capability，绑定 workspace、provider、admin、完整有效配置（含真实密钥、超时/重试与视觉能力）和旧 `updatedAt`；active PUT 必须携 capability、显式确认与相同版本，并以完整旧配置身份做最终 CAS，成功后才替换该 workspace 的 Registry generation。前端测试许可随任一草稿变化立即失效，测试期间修改草稿会丢弃迟到成功响应。
- SR-165 验证：既有 Provider 令牌/helper Rust 单测 11/11、前端交互 5/5、TypeScript、格式、专项编译与 scoped diff 门通过。部署后在服务器 `rs0` 随机隔离库直跑真实 Router 红线：缺许可与伪造/错版本许可均返回 409，拒绝前后完整 Provider BSON、Registry generation/provider/model 全部不变。成功发布链也已真实尝试：当前模型 `kr/claude-opus-4.7` 与调用日志中另外 3 个历史成功模型均对同一严格 JSON 合成请求返回 HTTP 530、无模型正文；用户提供的 `127.0.0.1:9090` 在本机不可达，服务器 `/v1/models` 与 `/health` 也均为连接失败。因此“连通测试成功→签发合法一次性 capability→active PUT 成功→Registry generation 单调增加→重启装载一致”当前被外部模型端点阻断，不能记为通过，也没有改用 mock 冒充。
- SR-166 实施状态（2026-07-25）：`已部署 / 确定性验证完成 / 服务器真实 Router+Mongo 动态通过`。Provider 的 timeout/maxRetries/retryBaseMs 使用显式三态：字段缺失保持现值，JSON `null` 通过 `$unset` 清除覆盖并恢复全局默认，数值写入 Provider 覆盖；连通测试与 active Provider 测试许可使用同一三态有效配置。列表和保存响应同时返回原始覆盖值、实际生效值及 `provider/global_default` 来源，前端编辑已有 Provider 时留空明确发送 `null`，新建留空仍不写覆盖。
- SR-166 验证：既有 Provider Rust 定向单测 13/13、前端交互 6/6、TypeScript、格式、专项编译与 scoped diff 门通过。部署后服务器动态用例真实执行两次 PUT：省略三个字段后原覆盖值保持；显式 `null` 后三个 BSON 字段消失，响应 effective 值回落当前全局默认且来源均为 `global_default`。
- SR-167 实施状态（2026-07-25）：`已部署 / 确定性验证完成 / 服务器真实 Router+Mongo 副本集动态通过`。被指派为视觉模型的 Provider 不能直接关闭 `supportsVision` 或删除，写前检查与最终 Mongo CAS 都拒绝并发身份变化；改派在 Mongo 事务内重读目标、撤销旧指派并提升新目标，任一步失败整体回滚。数据库在 `workspaceId` 上用仅覆盖 `isVisionActive=true` 的 partial unique 索引阻止多副本并发双指派；建索引前审计存量双指派及能力不一致并 fail closed，不自动猜选赢家。
- SR-167 验证：既有 Provider Rust 定向单测 13/13、视觉索引单测 1/1、前端交互 8/8、TypeScript、格式、专项编译与 scoped diff 门通过。部署后服务器 `rs0` 动态用例确认关闭能力与删除均 409 且完整 BSON 零写；事务改派后恰一条目标指派且目标具备视觉能力；直接制造第二条指派被 partial unique 索引拒绝。
- HC-031 部署后专项证据（2026-07-25）：部署快照源码 SHA-256 `888f1bc5f6fff2aa7ef520f9579f6baa07005b26e2789f98d0afd20b22072cb` 编译出的测试二进制 SHA-256 为 `a3cc29c5e374918f50e595cfdf9aeb8e625a489200d9f3ff1dea54172fced589`；机械枚举恰好 5 条目标用例，串行执行结果 `5 passed; 0 failed`，20.98 秒。5 个 `wechatagent_test_<UUID>` 隔离库原样保留为证据；正式服务测试前后 PID `1686295`、重启计数 0、运行中及磁盘 SHA-256 `539effe4f0cc1f6962c495f7454cd51375a35c854ca246901a9de1b5375e8acf`、健康响应均一致。原始证据位于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/provider-lifecycle-20260725T050238Z`，25 项内容哈希全部复验通过。
- HC-031 汇总状态（2026-07-25）：`SR-166 / SR-167 已完成部署后目标动态闭环；SR-165 拒绝路径已动态闭环，成功热切链已真实执行但被外部模型 HTTP 530 / 9090 不可达阻断`。阻断证据固化于 `/opt/wechatagent/releases/deploy-20260724T202505Z/audit/provider-sr165-model-blocker-20260725T20260725T051337Z`，`SHA256SUMS` 的 19 项内容全部复验通过；探测前后正式 PID `1686295`、重启计数 0、运行中和磁盘哈希及健康响应均不变。该结论不把外部阻断解释为代码通过或失败，也不把同一测试文件中的其它 24 个 filtered 用例解释为已验证。

## HC-032：把测试门改成“目标行为真的发生”，而非扩大测试数量

- 来源：SR-004、SR-126、SR-128、SR-174、SR-176、SR-178
- 两轮结论：全量 Docker job soft-fail；部分知识/租户测试重写生产逻辑；真模型红线可零产物绿色；进程级 cache使独立测试库串扰。它们证明的是证据不可信，不直接证明生产一定故障。
- 推荐：`修复少数上线红线门，保留长尾发现线`
- 最小处理：挑选发送、租户、知识审核、worker恢复等关键 case走真实 Router/Handler/Worker并 hard gate；每个动态 case输出 attempted/artifact/assertions/verdict/skip；cache按AppState或database identity隔离。其余慢测继续 soft/nightly。
- 不过度工程边界：不把所有 ignored/真模型测试改成 PR 硬门，不追求零波动；只要求关键红线有正向见证和明确 inconclusive。
- 不处理代价：CI绿色仍不能说明最关键能力被执行，生产回归可能长期藏在“零样本通过”后。
- 人类决定：`按推荐分层`（2026-07-19；用户在审阅“先收口 SR-128/178、再固化 SR-004 分层、随后远程验证”的计划后明确指示“执行”）。
- 实施状态：`工作树分层与动态证据协议已接线 / 本地确定性验证完成 / Actions与部署待证`。发送/Reaction T4 专项严格门已接线、远程运行待证；SR-174 为 `working-tree-wired / deterministic-verified / local-real-mongo-verified / deployment-pending`；SR-176 为 `working-tree-wired / deterministic-verified / local-real-router-mongo-verified / hard-gate-wired / actions-run-pending / deployment-pending`；SR-126 为 `working-tree-wired / deterministic-verified / hard-gate-wired / local-real-mongo-blocked / actions-run-pending / deployment-pending`；SR-004 为 `working-tree-policy-wired / deterministic-verified / actions-run-pending`；SR-128/SR-178 为 `working-tree-wired / deterministic-verified / hard-gate-wired / real-model-run-pending / deployment-pending`（2026-07-19）。各专用门的 Actions/部署证据仍缺失，不能关闭 HC-032。
- SR-004/T4 证据：手动 `smoke_t4` job 不使用 `continue-on-error`，先要求五条 `delivery_redline_*` 名称覆盖恰好为 5 并全部执行，再运行一条必须产出两轮回复、Reviewer 审计、Reaction 停止识别、Outbox 取消和单次 MCP 发送的真实模型任务。完整 transcript 在断言前打印并上传，模型错误不再 transient-skip；远程运行仍待证。
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
- 人类决定：`按最小 Markdown/JSON + CI 方案修复（SR-006/SR-179/SR-183 均已在工作树实现）`
- 实施状态：`SR-006/SR-179/SR-183 working-tree-wired / deterministic-verified / hard-gate-wired / actions-run-pending / deployment-pending（2026-07-24）`
- 负责人：Kiro
- 决定理由：三份历史任务正文保留，但旧 `[x]` 全部降级为 `[~]`，仅表示“曾被旧流程勾选”；`.kiro/specs/task-status-manifest.json` 才是状态权威。清单使用 `planned|implemented|production_wired|verified|partial|sunset_not_shipped` 六态，精确覆盖 144/144 个任务 ID，并绑定实现、生产入口、测试和 CI job。`verified` 额外要求 40 位冻结 commit、生产入口、测试 artifact 和非 soft CI job；当前脏工作树与未执行远程门下保守保持 `verified=0`，不把存在代码或 soft 测试冒充完整交付。
- SR-179 验证：`scripts/check-task-status-manifest.py` 正向检查 `expected=144 covered=144 verified=0 failures=0`；状态分布为 `production_wired=83 / implemented=23 / partial=34 / sunset_not_shipped=4`。临时副本负例证明删除一个任务覆盖、或把记录伪造成无冻结 commit 的 `verified` 均被拒绝；原清单恢复后再次 144/144 通过。baseline 已执行该检查，`check-ci-gate-policy.py` 同时锁定接线，CI 策略自检 `hard=7 soft=6 failures=0`。Actions 与部署尚未运行。
- SR-183 实施与验证：47 域工作流改为强 schema，深读/证伪均要求 `domain_id`、完整字段和至少一条可定位证据；每域固定保留 `complete|inconclusive|failed` 槽位，彻底移除 `filter(Boolean)` 静默漏域。运行前强制提供 run id、40 位 source commit、模型标识及域清单/锚点/工作流 SHA-256，并输出机器汇总。旧 2026-06-30 工件不重写、不冒充重跑结果，权威状态 manifest 将其诚实固定为 `complete=0 / inconclusive=47 / failed=0`。`scripts/check-audit-status-manifest.py` 正向通过，漏域/伪造 complete/哈希漂移负例 3/3、未来 v2 合法 complete/错误 locator 正反例 2/2 通过；baseline 和 CI policy 已锁定 checker，策略自检 `hard=7 soft=6 failures=0`。尚未运行 Actions，也未按 v2 协议重新执行 47 域审计，因此旧线索仍不能作为上线事实底座。
- SR-006 实施与验证：根 `README.md` 与 `docs/architecture.md` 已绑定 commit `d60d3d85f8e193160dca8df185de0daef004a6b6` + 当前未提交闭环工作树，并明确该标记不是部署验证；拓扑改为 Gateway → durable Outbox → second safety gate → MCP，Evolution 明确 env + Mongo 双闸且当前自动发布代码政策关闭，启动必填环境变量与 `src/config.rs` 一致，baseline 同步为 lib 350 / PBT 33。架构 worker 清单按 `src/main.rs` 的 13 个 `spawn_supervised` 调用点对账，旧直发流程和 9-worker 陈述已替换；`git diff --check` 通过。Actions 与部署文档复验仍待后续统一执行。

## HC-034：阻止文档编辑在详情未加载时清空原文

- 来源：SR-131
- 两轮结论：前端把 `{item}` 包络当扁平对象，正式保存可向 undefined id发请求并准备以空值覆盖原文，是独立、稳定、低成本的真实故障。
- 推荐：`立即修复`
- 最小处理：正确解包响应；id/rawContent未加载或对象不匹配时禁用保存；使用dirty PATCH并加一个组件/契约测试。
- 不过度工程边界：不并入全前端重构，不改后端文档模型。
- 不处理代价：运营正常编辑即可失败或清空知识原文。
- 人类决定：`修复（严格 dirty PATCH 已实现并部署）`
- 实施状态：`已修复、已部署、真实副本集与部署后常驻 Router 验证完成（2026-07-27）`
- 负责人：Kiro
- 决定理由：前端按真实 `{item}` 包络解包，只有详情 id、非空 title 与整数 version 匹配时才开放编辑；请求 generation 隔离迟到详情响应。保存只发送 `version + dirty metadata`。后端新增窄 PATCH 路由与 `deny_unknown_fields` 三态 DTO：缺失保持、`null` 清除可选字段、值执行规范化；只允许 `sourceName/title/summary/catalogSummary/routingMap/riskNotes/productTags/businessTopics`，租户、原文、hash、索引、生命周期和 catalog worker 字段始终由服务端保留。兼容 PUT 继续存在，但前端编辑不再依赖整替换或回传隐藏原文。
- 验证：前端 `DocumentEdit.test.tsx` 4/4；真实 Cookie Router + Mongo `rs0` 红线 1/1，覆盖 dirty 更新、no-op BSON 零变化、未知字段 422 零写、陈旧 version 409 零写与 `null` 清除；隔离 release 候选双健康并逐字服务 69 项前端资源。部署后常驻 Cookie Router 状态为 `GET 200 / dirty 200 / no-op 200 / unknown 422 / stale 409 / clear 200 / GET 200`，临时文档、会话和管理员均精确清理，队列、PID、重启数、后端/前端哈希和健康前后一致。切换证据清单 SHA-256 `600db09053d90c6d46eb54768762578400a9cb9427f04859c25774d7e2f4367a`；部署态 Router 证据清单 SHA-256 `dc77dc21564dad3cb83953a84df008744921f221e9143a384d61d17110dd6bf7`。

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
