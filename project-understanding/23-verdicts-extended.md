# 疑点终裁 II（07-19 号记录，核证日期 2026-08-13）

> 任务：对 07-19 号深读记录"偏差与疑点"节的全部疑点逐条终裁。每条结论三选一：【属实·缺陷】/【不成立】/【属实·设计】（= 属实·但为刻意设计、已声明取舍或需产品决策）。已标注"主会话已抽查/已核证/已裁决"的条目跳过（在汇总表中以【已核证】标注，不重复终裁）。
> 方法红线：每条终裁均基于当日（2026-08-13）工作区源码的当场亲读（Read/Grep），不依赖记录复述；无法当场核验的写【仍存疑】+ 原因。工作区含未提交改动（见 19 号），行号为当日实际值。
> 编号约定：`NN-k` = 第 NN 号记录第 5 节第 k 条疑点；17 号用 `17-Qk`；18 号 5.1 节对照点用 `18-①②③`。

## 终裁总计

| 结论 | 条数 |
|---|---|
| 【属实·缺陷】 | 52 |
| 【不成立】 | 17 |
| 【属实·设计】（刻意设计/已声明取舍/记录性事实） | 80 |
| 【仍存疑】 | 1 |
| 【已核证】（主会话已抽查/裁决，跳过） | 13 |
| **合计** | **163** |

---

## 1. 疑点汇总表

### 07 号（知识引擎，13 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 07-1 | DocEntry 死代码 / #619 文档级目录未落地 | 【已核证】主会话亲证属实 |
| 07-2 | AnswerStreamer 注释宣称 depth 计大括号层级，实现无 | 【属实·缺陷】 |
| 07-3 | corpus 200/400 窗口错位可把合法引用降格 fallback | 【已核证】主会话三处亲证属实 |
| 07-4 | user-ops 三件套 snake_case vs chat-only camelCase 两套入参命名 | 【属实·设计】（测试锁定现状的设计债） |
| 07-5 | open_slice 的 redact 分支在生产预载集合内不可达 | 【属实·设计】（防御性代码，无害） |
| 07-6 | write_knowledge_usage_log 注释称 fire-and-forget，实为顺序 await | 【属实·缺陷】 |
| 07-7 | block_parser 左侧缩进的 `---END CHUNK---` 也会终止块 | 【属实·缺陷】（极低概率边界） |
| 07-8 | gap_signals 模块 doc 说 8 类，实际结构 lint 9 类 | 【属实·缺陷】（文档过期） |
| 07-9 | render_one_document 的 persisted catalog 无 verified 门 | 【属实·设计】（管理导航面，非注入 prompt） |
| 07-10 | knowledge_router fallback 六行注释重复两遍 | 【属实·缺陷】（复制残留，无行为影响） |
| 07-11 | resolve_superseded/follow_relations 等 DB 放大性能观察 | 【属实·设计】（注释已声明规模假设） |
| 07-12 | structural_proposals 无消费方（KB-06） | 【已核证】模块自认+主会话亲证 |
| 07-13 | list_catalog filter.status 可经 ask 接口透传 archived | 【属实·设计】（admin 面；verified 门在，include_unverified 恒 false 已亲证） |

### 08 号（知识路由与 workers，12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 08-1 | crud PUT 响应硬编码 draft/needs_review 可能与库内不一致 | 【不成立】（title 恒在 patch → 恒降级，响应恒真） |
| 08-2 | has_anchor 裸 `!is_empty()` 四处与 B3 统一口径不一致 | 【属实·缺陷】（主会话核证 1 处 + 本次亲证其余 3 处） |
| 08-3 | chat apply_update_chunk 映射表含 4 个已删死字段 | 【属实·缺陷】（死字段可被写入文档） |
| 08-4 | merge 后 target integrity 可能不降级留"verified 无锚"中间态 | 【不成立】（patch 恒含 2 个敏感键 → 恒降级） |
| 08-5 | inbox 禁词测试锁旧文案副本，与实现文案漂移 | 【属实·缺陷】（测试覆盖失真，极轻） |
| 08-6 | knowledge_task execute_step 的 add_chunk/retag/dismiss 疑似死路径 | 【属实·缺陷】（死路径实锤 + dismiss filter 已漂移） |
| 08-7 | chat_turn turn_index 预分配空洞 | 【属实·设计】（审计不连续，读路径/上限计数不受影响） |
| 08-8 | digest_today 未命中同步合成无请求级互斥 | 【属实·设计】（正确性有 attempt_generation 栅栏，成本无互斥） |
| 08-9 | knowledge_digest/labels.rs 注释行号漂移（277-282 → 实际 364-369） | 【属实·缺陷】（注释行号漂移，极轻） |
| 08-10 | ask 的 max_rounds clamp 未在路由层核实 | 【不成立】（clamp 在 agent 层亲证存在） |
| 08-11 | import-apply/ingest 的 document 直接落 status="active" | 【属实·设计】（可用性由 chunk 层 verified 闸控制） |
| 08-12 | import-apply 全文锚定可能命中语义重复文本的首个位置 | 【属实·设计】（find 首个命中的算法边界；锚点仅溯源定位，verify 仍需运营） |

### 09 号（LLM/MCP/infra/prompts，12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 09-1 | supervisor 文件头注释说 8 个 worker，实际 16 个 | 【属实·缺陷】（注释过时） |
| 09-2 | user.review.claim_gate 有限额无 spec | 【已核证】主会话裁决：代码内嵌常量，有意游离于 prompt 治理外 |
| 09-3 | LLM 精确缓存白名单 4 key 均不在 prompt_specs | 【属实·设计】（缓存 key 是记账名，两套命名并存但行为正确） |
| 09-4 | parse_or_repair"层数"口径注释混乱 | 【属实·缺陷】（文档口径混乱，代码一致） |
| 09-5 | repair_loose_json 控制字符转义使"None=无修改"语义失真 | 【不成立】（行为正确；严格模式本就拒收该输入） |
| 09-6 | mcp_logs 写失败静默；媒体/名片 timeout 兜底核对缺权威通道 | 【属实·缺陷】（静默属实；已被统一核对层缓解为保守方向） |
| 09-7 | ensure_default_llm_provider 选举提升不跟随 .env | 【属实·设计】（DB 为真相源） |
| 09-8 | Anthropic max_tokens 硬顶 8192 | 【属实·设计】（兼容性保守钳制；新长输出模型适配债） |
| 09-9 | fetch_raw_text（repair 路径）无 HTTP 重试循环 | 【属实·设计】（修复失败成本低，可接受） |
| 09-10 | soul reset 对 draft spec 在已有流上 append 不 publish → 版本膨胀 | 【属实·缺陷】（轻微版本膨胀） |
| 09-11 | outbound_fetch IPv6 未显式排除 NAT64 | 【不成立】（fail-closed 白名单天然覆盖，记录自证） |
| 09-12 | config.rs 两个内容完全相同的测试 | 【属实·缺陷】（无害冗余） |

### 10 号（演化器与 workers，15 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 10-1 | schedule_post_release_review 死代码 + 模块注释与实际路径不符 | 【属实·缺陷】 |
| 10-2 | is_evolution_enabled_for 无生产调用方 | 【属实·缺陷】（死代码；注释三路设想只有 cohort 路生效） |
| 10-3 | rewrite 闸命中口径两处差 2 倍（+0.5 vs +1） | 【属实·缺陷】 |
| 10-4 | prompt shadow LLM 消耗不计入 EvolutionBudget，注释过时 | 【属实·缺陷】 |
| 10-5 | post_release 三 gate 同值观测；blocked_by_safety_guard 跨模块口径分歧 | 【属实·缺陷】（本次裁决：post_release 侧为真实口径，threshold/significance/auto_release 失真） |
| 10-6 | cohort.rs 注释描述空 contact 分支实际不可达 | 【属实·缺陷】（注释失实，极轻） |
| 10-7 | release cooldown 查询不排除已回滚 release | 【属实·缺陷】（两处口径不一，偏保守方向） |
| 10-8 | EVOLVABLE 白名单双定义无同步护栏 | 【属实·缺陷】（漂移风险） |
| 10-9 | Candidate.proposed_raw 存的是 clamp 后值，命名误导 | 【属实·缺陷】（命名误导，无行为影响） |
| 10-10 | cold worker 每 tick 重复写 assignment 审计虚耗 capacity 计数 | 【属实·缺陷】 |
| 10-11 | planner 六段 N+1 查询形态 | 【属实·设计】（性能取舍，quota 事务兜底） |
| 10-12 | mod.rs 启动日志仍写 "M4 W1 skeleton — empty tick by design" | 【属实·缺陷】（文案过时） |
| 10-13 | planner 排序键 i32 取负理论溢出 | 【不成立】（权重来源不可能达 i32::MIN，纯理论） |
| 10-14 | stage_stagnation_passes_in_memory 的 `let _ = now`，冷却判定读真实时钟 | 【属实·缺陷】（可测性弱点） |
| 10-15 | grade_prompt 证据门槛（1 条）与 threshold（30 条+失败率）差异巨大 | 【属实·设计】（阶段二证据制，注释明示管理员把关） |

### 11 号（业务面路由，15 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 11-1 | conversations.rs json! 里 msgType/mediaRef 重复键 | 【属实·缺陷】（复制瑕疵，值覆盖无行为影响） |
| 11-2 | reviews list 循环内逐条 fetch_run_status（N+1，≤300 次点查） | 【属实·设计】（性能取向，非正确性） |
| 11-3 | management 观测查询工具不强绑当前命令账号 | 【属实·设计】（与 REST 面"workspace 内不互相保密"口径一致） |
| 11-4 | list_contacts 读时过滤与 count $nor 双实现等价性无护栏 | 【属实·设计】（同文件共享白名单常量，风险低；规则对齐无护栏测试） |
| 11-5 | products PUT 全量语义（None → $set Null 清空） | 【属实·设计】（PUT 语义；与 media meta 部分更新语义不一致有契约误用风险） |
| 11-6 | referral_cards "card not found" 用 BadRequest 而非 NotFound | 【属实·缺陷】（错误码风格不一致，轻微） |
| 11-7 | evaluations.rs:397 expect panic 面依赖 90 行外的前置校验 | 【属实·设计】（受前置 is_valid 保护的脆弱耦合） |
| 11-8 | media 上传/文本资产 accountId 不走 validate_account | 【属实·缺陷】（错拼产生孤儿私有资产，轻微） |
| 11-9 | guide 路径 intentLevel 嵌套+else-if 双路径解析结构重复 | 【属实·设计】（本次亲读确认两路径行为一致，可读性问题） |
| 11-10 | management needs-confirm 分支 finalize 与并发 confirm 竞速窗口 | 【属实·设计】（可接受竞态；confirm 侧幂等回放已亲证） |
| 11-11 | guide preview readableChanges 丢弃 LLM 话术，改用机器拼接键名 | 【属实·缺陷】 |
| 11-12 | campaigns camelCase vs contacts snake_case 集合命名分裂 | 【属实·设计】（已知结构性坑位，serde rename 有注释背书） |
| 11-13 | events kind 过滤只支持精确匹配 | 【属实·设计】（功能局限非缺陷） |
| 11-14 | operation_view 与 guides 对 dimension_values scope 传参不同 | 【属实·设计】（全局视图 vs 单联系人视图，注释声明） |
| 11-15 | update_manual_tags 不刷顶层 updated_at → guide apply OCC 漏检口 | 【属实·缺陷】 |

### 12 号（管理面路由与鉴权，15 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 12-1 | "TaxonomyEntry 无 workspace_id"注释失实 | 【属实·缺陷】（过期注释误导隔离边界判断） |
| 12-2 | 死路由 tripwire include 名单缺 11 个路由文件 | 【属实·缺陷】（护栏缺口） |
| 12-3 | principal_escalations/ask_human_inbox 裸 bson DateTime 扩展 JSON 残留 | 【属实·缺陷】（一处有真实前端崩溃路径） |
| 12-4 | gap_signals "sweep 命中率" 三处注释与实际输出矛盾 | 【属实·缺陷】（注释矛盾；以 historicalResolvedShare 为准） |
| 12-5 | observability worker_health 注释说 "workspace_id 强制 default" 过时 | 【属实·缺陷】（过期注释） |
| 12-6 | broadcast_chunk_revised 死函数（workspace 写死空串，零调用） | 【属实·缺陷】（死代码） |
| 12-7 | 登录限流 client 维度只认直连 IP，不解析 X-Forwarded-For | 【属实·设计】（安全默认正确；反代部署有挤兑风险，需运维决策） |
| 12-8 | put_evolution_runtime_flag 的 updated_by 取请求体可伪造 | 【已核证】主会话亲证属实 |
| 12-9 | REST 侧 approve_taxonomy_candidate 无 scope 校验 | 【属实·设计】（注释声明"维持现状不回归"） |
| 12-10 | /ws/chunks 鉴权依赖 cookie，JWT-only 客户端无法订阅 | 【属实·设计】（事实性边界） |
| 12-11 | reject 类 handler 终态回读缺 workspace 过滤 | 【属实·设计】（风格不一致，无越权后果——前置 update 已 matched=1） |
| 12-12 | outcomes_autonomy accountId 不 validate、horizon 非法静默回退 | 【属实·缺陷】（轻微，与兄弟模块严格 400 风格不一致） |
| 12-13 | CORS allow_origin(Any) | 【属实·设计】（cookie 模式实际不可跨站；改 credentials 时需警惕） |
| 12-14 | OPERATION_STATE_ACTION_VALUES 取值未读定义 | 【不成立】（本次亲证定义 5 值与前端契约一致，疑虑排除） |
| 12-15 | llm_providers 进程锁多副本不互斥 | 【属实·设计】（文档自认限制，事务+CAS 兜底） |

### 13 号（前端 core，16 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 13-1 | wa.authed sessionStorage 键只写不读 | 【属实·缺陷】（dead state + 注释漂移） |
| 13-2 | openEventSource 无消费方 | 【属实·缺陷】（dead code） |
| 13-3 | domain_profile/knowledge_chat_turn/worker_control 三 fixture 前端零对账 | 【属实·缺陷】（契约护栏半闭环） |
| 13-4 | DomainProfileDraft 缺 generated_state_machine 疑丢草稿 | 【已核证】主会话裁决不成立 |
| 13-5 | disableAgent/analyzeProfile 等写操作防护弱；disable 后端无账号绑定 | 【属实·缺陷】 |
| 13-6 | sendAnalytics/campaigns list/strategy 无 generation 防护 | 【属实·设计】（workspace 级或低频页，已知取舍） |
| 13-7 | 401 拦截器只匹配相对路径 /api/ | 【不成立】（当前全部相对路径，纯边界事实） |
| 13-8 | api.delete/post 强制 response.json()，204 空体会抛错 | 【属实·设计】（现约定全端点返回 JSON 体的契约脆弱点） |
| 13-9 | referralCardStore 列表不带 accountId、创建可定向 | 【属实·设计】（workspace 级资源读写不对称，类型注释背书） |
| 13-10 | visibleWhen 谓词全体未使用 | 【属实·设计】（自认扩展点） |
| 13-11 | 演示文案硬编码为初始 state | 【属实·设计】（刻意引导默认值；换行业部署原样出现） |
| 13-12 | WS snake_case/REST camelCase/Profile snake/内层 camel 四套命名并存 | 【属实·设计】（均有注释背书；契约测试只覆盖 camelCase 投影面） |
| 13-13 | ContactTab 类型与 contactCounts 键一致性 | 【不成立】（两侧同构一致） |
| 13-14 | StrictMode 双跑防护只盖 App 引导 | 【属实·设计】（WS 清理函数完备，可接受） |
| 13-15 | inboxStore.load errors 合并读旧 summary | 【属实·设计】（降级数据可容忍，注释声明） |
| 13-16 | walkthrough.py/styles.css 未逐行 | 非疑点（范围声明） |

### 14 号（前端 features，6 组）

| # | 一句话 | 终裁 |
|---|---|---|
| 14-1 | 4 项 dead code（RefreshCw import/REVIEW_CATEGORY_LABELS/readableChangeItems/loadAgentRuns） | 【属实·缺陷】（本次抽验 2 项 grep 复证） |
| 14-2a | USER_RUNTIME_PARAMETER_FIELDS 双份定义，已现 label 漂移 | 【属实·缺陷】 |
| 14-2b | auto-verify 双入口抽样率口径不一致（quality 可填 0） | 【属实·缺陷】（轻微：UI 契约误导；后端 clamp 0.05 硬下限兜底，红线不受影响） |
| 14-2c | DomainPromptPanel/ActiveVersionsBar/yuanToCents 等平行实现 | 【属实·设计】（维护债） |
| 14-3 | pendingTasks 写死 0、overview spark 静态、KnowledgeInbox 本地乐观隐藏等占位 | 【属实·设计】（注释自认占位/后端无对应端点不发死请求） |
| 14-4 | DIGEST_TARGET_REF_KIND 前后端口径：prompt 3 值 vs models 注释 5 值 | 【属实·缺陷】（后端两处口径不一且运行时无枚举校验；前端并集防御正确） |
| 14-5 | TryRecallView placeholder 误导/gap 与 digest 两套 severity 域 | 【属实·设计】（轻微文案/领域划分问题） |
| 14-6 | steward.tsx 3342 行等规模观察 | 非缺陷（维护性观察） |

### 15 号（tests agent 主链路，10 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 15-1 | 两个空壳测试永远绿（revision_recheck/memory_card_write_occ） | 【已核证】主会话亲读全文证实 |
| 15-2 | worker_reclaim 名不副实（只 insert+断言 running，无 reclaim 驱动） | 【属实·缺陷】（本次亲读 :41-72 实锤） |
| 15-3 | 约 6 处复刻式测试与生产逻辑脱节风险 | 【属实·设计】（作者显式声明的漂移风险） |
| 15-4 | autonomy_protocol_pbt P2 是测试内复刻控制流的模型测试 | 【属实·设计】（模型测试局限，作者注明快照行号） |
| 15-5 | Multipart 端点无法集成层测，副作用由代码审查保证 | 【属实·设计】（本次亲读 :12-18 自认；框架限制） |
| 15-6 | 测试注释中的生产行号是写作时快照 | 记录性事实 |
| 15-7 | real_llm 对 escalation 触发只软观测 | 【属实·设计】（诚实降级，有弱化终局断言） |
| 15-8 | 约 9 成集成测试 #[ignore]，行为守护在 CI | 记录性事实（与 CLAUDE.md 分工一致） |
| 15-9 | happy_path_run.rs:623 `let _ = (outbox, Duration...)` 残留 | 【属实·缺陷】（无效残留；存在性断言已由 expect 完成） |
| 15-10 | 多份测试文件处于未提交修改态 | 记录性事实 |

### 16 号（tests 知识/演化/安全/真模型，12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 16-1 | ingest 正向拉取零集成覆盖 | 【已核证】主会话亲证 |
| 16-2 | workspace_isolation 等是 filter 形状测试 | 【已核证】主会话亲证 |
| 16-3 | taxonomy 版本 handler 无权限门 | 【属实·设计】（本次亲证 admin_ops_versions.rs:1235-1240 注释显式"不加拦截门"；待产品定义 RBAC） |
| 16-4 | EVO-2（released_by=真实操作者）无自动化测试 | 【属实·设计】（自认审查代测；可用既有 repl_set+真 HTTP 模式补） |
| 16-5 | 删 active profile 的 handler 拒绝路径无测试 | 【属实·设计】（测试缺口；本次亲证 handler 守卫存在 domain_profiles.rs:694-698） |
| 16-6 | real_llm 判定强度分层易被误读 | 记录性评估指引 |
| 16-7 | TestLlmGenerator 按 prompt 锚文本路由 mock | 【已核证】主会话亲证隐式契约 |
| 16-8 | jwt_auth.rs 内联整份 AppConfig 字面量 | 【属实·设计】（样板重复，编译错误兜底） |
| 16-9 | hc028 与其余 real_llm 的 skip 语义不同 | 【属实·设计】（有意硬门） |
| 16-10 | maycran_transport_probe 临时探针长期留树 | 【属实·设计】（自认 temporary，有过期风险） |
| 16-11 | 3 个 .proptest-regressions 应保留 | 非疑点 |
| 16-12 | sr012 归属勘误 | 记录归属勘误（16 号已补读，无遗漏） |

### 17 号（kiro specs 与 docs，12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 17-Q1 | "user.reply.task 已退役"初判误报 | 【已核证】主会话裁决修正 |
| 17-Q2 | 三个 sunset notice 描述的"3 闸 enforce_*"是中间态 | 【属实·设计】（文档过时；现状=分数闸体系，本次 gates.rs 亲读复证） |
| 17-Q3 | 基线数字演进链未回写历史 spec | 【属实·设计】（文档滞后；现行权威=check-baseline.sh LIB_BASELINE=350） |
| 17-Q4 | evolution 自动发布契约冲突（SR-180） | 【属实·设计】（spec 未修订；代码已以 CURRENT_AUTO_RELEASE_POLICY_ENABLED=false 编译期硬闸裁决人工发布，本次亲证） |
| 17-Q5 | 任务账本失真已制度化纠正但有残余误导面 | 【属实·设计】（manifest 权威已建；覆盖面与 asOf 时效残留） |
| 17-Q6 | 47 域审计全判 inconclusive，不可作上线证据 | 【属实·设计】（按 audit-status-manifest 权威裁定，引用者须知） |
| 17-Q7 | 真模型红线硬门允许零样本绿（SR-178） | 【不成立】（已修复：capability_outcome 22 case 正向证据硬门 + pass() 前置断言） |
| 17-Q8 | memoryCard 双不变量数学不可满足（SR-182） | 【不成立】（已修复：溢出项迁 recent + coreFactEvictions 审计，不再静默丢弃） |
| 17-Q9 | operator memory "随时可撤销"无撤销路径（SR-181） | 【不成立】（已修复：revoke 端点+agent 实现+sr181 测试+前端入口四层齐备） |
| 17-Q10 | ISSUE-012 知识红线三层联动失效终态悬置 | 【不成立】（已重构修复：R5.4/R5.3.a fail-closed 分支 + 独立 ClaimGate 体系） |
| 17-Q11 | 文档快照类声明的时效 | 记录性事实 |
| 17-Q12 | 小型笔误（agent_runs 集合名/历史禁词根/8 vs 9 模块等） | 【属实·设计】（历史档案笔误，lint 不追溯） |

### 18 号（superpowers specs，对照点② + 12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 18-② | user.reply.task 退役未落 spec（最大文档-代码漂移点） | 【已核证】与 17-Q1 同一事项，主会话已裁决 |
| 18-1 | 07-15→08-05 约 20 天 spec 空窗 | 【属实·设计】（文档缺口事实） |
| 18-2 | 被引用的 2026-07-10-full-system-test-findings.md 不存在 | 【属实·缺陷】（引用悬空，本次 glob 亲证） |
| 18-3 | 全集 file:line 均为写作时快照 | 记录性事实 |
| 18-4 | 149 plans vs 165 specs 非一一对应 | 记录性事实 |
| 18-5 | 文档"已修/Fixed"状态未对码复核 | 元观察（本终裁已对 Q7-Q10 四项典型完成对码） |
| 18-6 | EVOLUTION_ENABLED 默认值文档内部矛盾 | 【不成立】（现行代码已收敛：常量 "false" + 注释一致 + 测试锁定） |
| 18-7 | S-01 runtime_flag=None 注释与实现语义相反 | 【不成立】（现行 mod.rs 注释与 cohort 实现一致：None=全员排除） |
| 18-8 | m011 与存活集合同名的生产安全实证未回填 | 【仍存疑】（代码侧 APP_ENV 审批闸已亲证存在；生产 117 的 migrations 入账状态本地不可核验） |
| 18-9 | PR#216/217 修复无独立 spec | 【属实·设计】（走 plan/PR 的文档声称） |
| 18-10 | 知识 Agent 反接管批次 2/3 无后续 spec | 【属实·设计】（未闭环项） |
| 18-11 | 未闭环清单（KB-06 等 5 项） | 【属实·设计】（其中 KB-06 与 07-12 同源，主会话亲证无消费方） |
| 18-12 | lib 基线 350 vs 实测 1974 | 【不成立】（下限 vs 实测，非矛盾） |

### 19 号（未提交改动与脚本/CI，12 条）

| # | 一句话 | 终裁 |
|---|---|---|
| 19-1 | 未提交改动含禁词"人工"，进 PR 必红 | 【已核证】主会话已验证 |
| 19-2 | domain8 severity="BLOCKED" 与 expect 语义冲突，`return` 成死代码 | 【属实·缺陷】 |
| 19-3 | 文件数/行数与任务描述不符 | 记录性事实（统计时点差异） |
| 19-4 | batch_c_management 二次 confirm 幂等断言依赖服务端未核验行为 | 【不成立】（服务端幂等回放已实现，本次亲证） |
| 19-5 | performance_report.py 与 gateway_performance_report.py 近乎重复 | 【属实·设计】（维护冗余，19 号已全文亲读两件） |
| 19-6 | deploy.sh 过时快照（8080 端口/旧分支/交互合并流程） | 【属实·设计】（过时工具，以 scripts/deploy/ 新链为准） |
| 19-7 | rt_send.py 硬编码真实 appId/wxid 且无 HMAC | 【属实·设计】（仅限关签名本地联调；误用风险受限） |
| 19-8 | 旧 cleanup 括号不平衡 bug（推断从未成功清理） | 【属实·缺陷】（修复+新测试锁死已亲证于 19 号；"从未成功"为合理推断） |
| 19-9 | evaluation 仍用时间窗版 assert_llm_success | 【属实·设计】（多场景无单一 run_id 的可解释折衷） |
| 19-10 | 禁词"人工"极宽会拦正常运营用语 | 【属实·设计】（CLAUDE.md 同款词表的既定产品红线） |
| 19-11 | domain1011 运行时拼接禁词（scripts 本不在扫描目录） | 【属实·设计】（防御性习惯） |
| 19-12 | .env.example 缺 POST_DECISION/SILENCE_SIGNAL 两族等 13 个变量 | 【属实·缺陷】（文档缺口，本次亲证差集） |

---

## 2. 逐条终裁详情

### 2.1 任务指定的 11 条重点疑点

#### ① 10-4 prompt shadow LLM 消耗不计入 EvolutionBudget —— 【属实·缺陷】

- **原疑点**：mod.rs 注释仍说 replay 阶段"prompt 走 placeholder failed 不触发 BudgetExceeded"，但 replay 已接真实 shadow；shadow 的 LLM 开销不回填 tick 级 EvolutionBudget。
- **亲读范围**：`src/evolution/mod.rs:220-260`、`src/evolution/replay.rs:1-260`（头注、eval_all、run_shadow_replay 全文）、`src/agent/prompt_shadow.rs:355-395`。
- **证据链**：
  - `mod.rs:228-231` 注释："replay 现阶段 threshold 不调 LLM、prompt 走 placeholder failed，所以这里不会再触发 BudgetExceeded"——**过时**；
  - `replay.rs:9-13` 头注 + `replay.rs:243-254` 实现：prompt 候选真实调用 `crate::agent::prompt_shadow::shadow_replay_prompt_one` 跑 Reply+Review 演练；
  - `replay.rs:107-109,125-133`：EvolutionBudget 是 `&mut` 不能跨 task，仅在起 task 前做 `budget.exhausted()` 静态预检（:128），task 内消耗零回填；:125-127 注释同样残留旧口径但自认"占位以保持后续接入完整 LLM 时一处控制"；
  - `prompt_shadow.rs:369-379`：`new_shadow_budget` 用 per-replay `RunBudget::new(run_id, runtime.simulation_token_budget, runtime.run_max_llm_calls, ...)`，run_mode="shadow"——shadow 消耗记在 RunBudget，不进 EvolutionBudget。
- **终裁**：属实。EvolutionBudget（envelope 的 budget_used_tokens）不含 shadow 消耗，`EVOLUTION_RUN_TOKEN_BUDGET` 实际只约束 Critic 一次调用；两处注释过时。shadow 消耗并非无界——受 per-replay RunBudget（simulation_token_budget=60000/run_max_llm_calls）× cohort 规模 × 并发信号量约束，但 tick 级预算审计失真。
- **严重度/触发条件**：中低。EVOLUTION_ENABLED=true 且存在 prompt 类 pending_eval 候选时触发；后果=预算遥测少计 + budget_exceeded 事件对 shadow 开销失明，非无界消耗。

#### ② 10-3 rewrite 闸口径两处差 2 倍 —— 【属实·缺陷】

- **原疑点**：threshold::generate 给 rewrite 两闸各 +0.5，auto_release 给各 +1 且注释自称一致。
- **亲读范围**：`src/evolution/threshold.rs:100-129`、`src/evolution/auto_release.rs:284-344`。
- **证据链**：`threshold.rs:114-123`：`revision_applied=true` → human_like/emotional_value 各 **+0.5**（注释："暂按两侧各 0.5 分摊"）；`auto_release.rs:333-336`：同条件各 **+1**；`auto_release.rs:287-289` 注释声称"与 [super::threshold::generate] 内的口径一致"——**注释与实现直接矛盾**，auto_release 的 rewrite 命中率恒为生成侧的 2 倍。
- **终裁**：属实·缺陷（注释错误 + 口径分裂）。
- **严重度/触发条件**：低（休眠）。`CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 编译期恒关（auto_release.rs:36-45 本次亲证），当前零生产影响；一旦未来启用自动放行，KE-01 方向门将用 2 倍失真的命中率做 band 判定，可能错误放行/拒绝升降阈候选。

#### ③ 10-7 release cooldown 不排除已回滚 —— 【属实·缺陷】

- **原疑点**：release 侧 cooldown 计数不带 rolled_back 排除，与生成侧不一致。
- **亲读范围**：`src/evolution/release.rs:258-300`、`src/evolution/threshold.rs:255-289`。
- **证据链**：`release.rs:273-285` cooldown 查询 filter = workspace+account+gate_key+`released_at≥since`，**无** `current_version:true`/`rolled_back_at:null`；`threshold.rs:270-277`（load_gate_cooldowns）filter **带** `current_version:true` + `rolled_back_at:null`。两处对"cooldown 中"语义不一致，且 release 侧无注释说明。
- **终裁**：属实·缺陷（口径不一致；方向保守）。
- **严重度/触发条件**：低。触发 = release 某 gate → 回滚 → cooldown 窗口（默认 24h）内尝试再次 release 同 gate → 被拒（`threshold release cooldown active`）；而生成侧不认为该 gate 在 cooldown、会继续产 pending 候选并可 eligible——管理员会看到"可发布却发不出"的矛盾状态。无安全风险（保守方向），属可用性/一致性缺陷。

#### ④ 14-2b auto-verify 双入口口径不一致 —— 【属实·缺陷】（轻微，红线不受影响）

- **原疑点**：knowledge/AutoVerifyPanel 抽样勾选 0.3/取消 0.05，quality/AutoVerifyTab 自由输入可填 0，后端是否强制下限前端未体现。
- **亲读范围**：`src/routes/knowledge/verify.rs:29-51`、`frontend/src/features/quality/index.tsx:161-218`、`frontend/src/features/knowledge/cockpit/AutoVerifyPanel.tsx:46-70`。
- **证据链**：后端 `verify.rs:39-43` `clamp_sample_rate = requested.unwrap_or(0.3).clamp(0.05, 1.0)`——传 0 也钳到 0.05（:36-38 注释"删下界会让红线被静默关掉"，有单测锁死）；`AutoVerifyPanel.tsx:55-57` 明示"取消 → 仍保留 5% 硬下限（后端不允许 0）"；`quality/index.tsx:163,213-217` `sampleRate` 输入 `min={0}` 可填 0 且无任何提示会被钳制。
- **终裁**：属实·缺陷（轻微）。红线（永远留一批抽审）由后端 clamp 牢牢守住不受影响；缺陷仅在 quality 页 UI 契约误导——运营填 0 以为关闭抽审，实际按 0.05 执行且无反馈。
- **严重度/触发条件**：低。触发=运营在 quality 页填 <0.05 的抽样率。

#### ⑤ 14-2a USER_RUNTIME_PARAMETER_FIELDS 双份漂移 —— 【属实·缺陷】

- **原疑点**：legacy.tsx 与 userOpsDomainHelpers.ts 各有一份近乎相同的 20 项参数表。
- **亲读范围**：`frontend/src/features/user-ops/legacy.tsx:138-167`、`frontend/src/stores/userOpsDomainHelpers.ts:1-60`、消费点 grep（legacy:971,1581-1582；helpers:71-72）。
- **证据链**：两份均为文件私有 `const`（非 export、无交叉 import）；key 集与 defaultValue 逐项一致（20 项）；**已现漂移一处**：`operationStateConfidenceFullReviewBelow` 的 label/detail——legacy:157 "状态置信复盘线/…强制完整复盘" vs helpers:37 "状态置信 Review 线/…强制完整 Review"。消费面：legacy 份渲染表单+known keys 排序；helpers 份仅消费 key（label/detail 在 helpers 内实为死数据，runtimeParametersText 只用 key 排序）。
- **终裁**：属实·缺陷。当前无行为差异（key/default 一致），但已发生文案漂移一处，且新增 runtime 参数漏改一处会造成"表单渲染字段集"与"文本编解码字段集"分叉，无同步护栏。
- **严重度/触发条件**：低；触发=未来单侧新增/改名参数。

#### ⑥ 19-2 biz-test domain8 severity="BLOCKED" 死代码 —— 【属实·缺陷】

- **原疑点**：新 expect 只豁免 low，"BLOCKED" 会 raise，其后 `if not eligible: return` 是死代码，且环境性受阻应走 record_blocked 台账。
- **亲读范围**：`scripts/biz-test/batch_a_domain8.py:60-77`、`scripts/biz-test/_lib.py:905-956`。
- **证据链**：`_lib.py:943-956` expect：cond=false 且 `severity.strip().lower() != "low"` → `raise BizTestAssertionError`（"BLOCKED"≠"low" → 必 raise）；docstring:946-948 明示 "explicit BLOCKED findings are authoritative suite failures"——即 expect 侧把 BLOCKED-fail 写成了**有意语义**；`batch_a_domain8.py:66-70` `expect(eligible, ..., "BLOCKED", ...)` 后跟 `if not eligible: return`——raise 使该 return 永不可达（死代码确定性属实）；`_lib.py:905-917` 另有 `record_blocked`（写 target/biztest_blocked.jsonl，不 raise，run_all 汇总为 passed_with_blocked）——同仓 BLOCKED 概念两套语义并存。
- **终裁**：属实·缺陷。(a) `return` 死代码确定；(b) "前序回复未 sent"这类环境/上游性受阻（LLM 端点抖动即可造成）会被计为 authoritative suite failure 而非 BLOCKED 台账，与 G7 的 BLOCKED 分流设计（`is_api_error` docstring:968 "端点故障应标 BLOCKED 不假绿，不是业务断言失败"）冲突。方向偏保守（多报失败而非假绿）。
- **严重度/触发条件**：低（测试基建内部不一致，无生产影响）。触发=domain8 购买场景前序回复未达 sent。

#### ⑦ 19-12 .env.example 缺两族变量 —— 【属实·缺陷】（文档缺口）

- **亲读范围**：Grep `.env.example`（POST_DECISION*/SILENCE_SIGNAL*/SILENCE_THRESHOLD*/COMPLETENESS_CACHE*/DYNAMIC_CONFIDENCE_MIN_SAMPLES/EVOLUTION_MAX_SAFETY_REGRESSION_RATE 全部零命中）+ `src/config.rs:497-524,655-662,695`。
- **证据链**：config.rs 实际读取且带默认值：POST_DECISION 族 6 个（`POST_DECISION_WORKER_CONCURRENCY:508 / _MAX_ATTEMPTS:511 / _SNAPSHOT_MAX_BYTES:514 / _PROMPT_MAX_CHARS:517 / _TOKEN_BUDGET:520 / _FAILED_SNAPSHOT_RETENTION_DAYS:524`）、SILENCE 族 4 个（`SILENCE_SIGNAL_WORKER_ENABLED:655 / SILENCE_THRESHOLD_SECONDS:658 / SILENCE_SIGNAL_INTERVAL_SECONDS:659 / _DAILY_CAP:661`），另有 `COMPLETENESS_CACHE_TTL_SECONDS:497`、`DYNAMIC_CONFIDENCE_MIN_SAMPLES:662`、`EVOLUTION_MAX_SAFETY_REGRESSION_RATE:695`——共 13 个变量 .env.example 全缺席。
- **终裁**：属实·缺陷（低危文档缺口）。全部有代码默认值，启动不受影响；运维排障/调参缺文档入口（digital_twin 验收依赖的 post-decision worker 即在其列）。

#### ⑧ 12-7 登录限流不解析 XFF —— 【属实·设计】（安全默认正确；反代部署需运维决策）

- **亲读范围**：`src/routes/auth.rs:240-260`、`src/main.rs:381`。
- **证据链**：`auth.rs:242-246` `direct_client_identity` 只取 `ConnectInfo<SocketAddr>` 直连 IP（缺失时 "unknown-direct-peer"）；`main.rs:381` `into_make_service_with_connect_info::<SocketAddr>` 确保直连 IP 可得。全文件无 X-Forwarded-For 解析。
- **终裁**：属实·但为合理的安全默认——XFF 可被客户端伪造，盲信 XFF 会让攻击者伪造任意 IP 绕过 client 维度限流；在无可信代理配置的前提下不解析是 fail-safe 正确选择。当前部署形态（19 号亲证：biz-test/diag 工具直打 `127.0.0.1:3003`，Rust 进程直接对外）无反代层，语义正确。若未来加反向代理：所有真实客户端共享代理 IP 的单一 client 指纹（容量 20/5min）互相挤兑——但 target（按用户名）与 global 维度仍独立生效，是可用性退化而非安全漏洞。需运维在引入反代时决策（加可信代理白名单解析）。

#### ⑨ 12-3 bson DateTime 扩展 JSON 两处残留 —— 【属实·缺陷】（一处有真实崩溃路径）

- **亲读范围**：`src/routes/principal_escalations.rs:26-60`、`src/routes/ask_human_inbox.rs:14-75`、`src/models.rs:4660-4668`、`frontend/src/features/ask-human/ResolvedEscalations.tsx:21-60`、`frontend/src/lib/inboxApi.ts:8-14`、grep ask-human/index.tsx createdAt 消费。
- **证据链**：
  - 后端：`principal_escalations.rs:52,54` 把 `e.created_at`（bson DateTime）与 `e.authorization_expires_at`（`models.rs:4664` `Option<DateTime>`）直接放进 `json!` → wire 上为扩展 JSON `{"$date":{"$numberLong":…}}` 对象；`ask_human_inbox.rs:25` `created_at: Option<DateTime>` 经 serde Serialize 同样输出对象。
  - 前端消费：`ask-human/index.tsx` **零消费** createdAt（排序用 ageHours）→ inbox 侧仅契约脏无 UI 影响（`inboxApi.ts:11` 类型声明 `string|null` 与实际不符）；`ResolvedEscalations.tsx:24,109` 消费 `authorizationExpiresAt` → `formatExpiry`（:29-41）：对象 truthy → `new Date(object)` = Invalid Date → `return value` **把对象原样交给 React 渲染** → "Objects are not valid as a React child" 崩溃。
  - 写点核实：`agent/escalation/ledger.rs:614-632` resolve 带 authorization_window_hours 时会写 `authorization_expires_at`——即"领导给了转述时限"的裁决真实产生 Some 值。
- **终裁**：属实·缺陷。触发路径：任一请示以带时限（authorizationWindowHours）方式 resolve → 运营打开"已裁决历史" → ResolvedEscalations 组件崩溃（同 domain_profiles.rs:521-528 注释记录的白屏事故同款形态，该处已修此两处未修）。`createdAt` 两处为契约脏无即时 UI 影响。
- **严重度**：中。修法同 profile_view：`dt_to_string` 转 RFC3339 + 契约测试。

#### ⑩ 12-2 死路由 tripwire 缺 11 文件 —— 【属实·缺陷】（护栏缺口）

- **亲读范围**：`src/routes/mod.rs:1080-1214`（include_str! 名单全量 + KNOWN_NON_ROUTE_HANDLERS）。
- **证据链**：`mod.rs:1083-1129` 共 44 项 include（含 knowledge/ 10 个子文件）；逐一比对确认缺席 11 个含 `pub async fn` 的路由文件：campaigns.rs、ask_human_inbox.rs、principal_escalations.rs、domain_profiles.rs、guide_profile.rs、media_assets.rs、referral_cards.rs、send_ledger.rs、operation_view.rs、worker_controls.rs、management_prompt_edit.rs（末者仅 re-export 无 handler，缺席无实质影响）。
- **终裁**：属实·缺陷。这 10+1 个文件内新增 `pub async fn` handler 忘挂载不会被该测试抓到；contract_snapshot.rs:103-104 自证该手维护清单"已腐烂"（投影护栏已改运行时扫描，路由护栏未同步重构）。
- **严重度**：低（护栏盲区，非生产行为缺陷）。

#### ⑪ 08-6 knowledge_task execute_step 死路径 —— 【属实·缺陷】（死路径 + dismiss filter 已漂移）

- **亲读范围**：`src/knowledge_task/mod.rs:925-999`（run_claimed_task 分派）、`:1202-1300`（execute_step 头部+fix_chunk+add_chunk 起始）、`:1531-1585`（execute_step dismiss 分支）、`:650-690`（commit_mutating_step_once dismiss 分支）、全仓 grep execute_step 调用方。
- **证据链**：
  - `mod.rs:935` `is_mutating = matches!(action, "add_chunk" | "retag" | "dismiss")` → mutating 走 prepare_mutating_step/persist_step_intent/commit_mutating_step_once 两阶段（:936-967），**不进 execute_step**；仅非 mutating（fix_chunk/review_evolution/analyze_logs/未知）走 execute_step（:968-981）。
  - execute_step（pub，:1202）内仍存在完整的 add_chunk（:1276-）/retag/dismiss（:1531-1582）实现——生产 worker 主路径不可达；调用方仅 `mod.rs:971`（非 mutating）与测试 `tests/knowledge_worker_behavior_integration.rs`（直调，含 add_chunk 非法 payload 路径）。
  - **漂移已现实发生**：execute_step 的 dismiss filter（:1539-1545）= workspace_id + cards.cardId（+可选 report_date），**缺 account_id**；commit 路径的 dismiss filter（:662-668）= workspace_id + **account_id** + cards.cardId。若未来误用 execute_step 路径，可 dismiss 同 workspace 其它账号日报的卡片。
- **终裁**：属实·缺陷。双份实现 + 漂移风险已兑现（account 过滤缺失）；测试文件 knowledge_task_worker.rs:24 还声明"与 execute_step 的 match arms 对齐"，锚定的是死分支。
- **严重度**：低-中（当前生产不可达；维护陷阱 + 测试锚定失真）。建议删除三个死分支或收敛为单一实现。

### 2.2 其余重点【属实·缺陷】详情

**10-5 blocked_by_safety_guard 跨模块口径分歧（本次裁决出真实侧）**
- 亲读：`src/agent/review/gates.rs:33-46,82-110,160-205,995-1005,1371-1455`、`src/agent/review/mod.rs:1385-1421`、`src/agent/gateway.rs:4194-4214`、`src/evolution/post_release.rs:56-69`、`src/evolution/threshold.rs:107-123`、`src/evolution/auto_release.rs:320-336`。
- 裁决依据：gates.rs 证明 **pressure_risk 是软闸**——`classify_dual_gate` 把 pressureRisk≥阈值归 SoftGateFailure（:102-105 注释：保留 approved、写 needs_revision 触发 single-shot revision，finalize 走 Approved），其终态痕迹是 revision_applied/revision_failed；生产 `blocked_by_safety_guard` 的真实来源=claim gate fail-closed（review/mod.rs:1385-1421 三处 hold_for_*）、claim manifest 越界（gates.rs:779,818）、relay 内部载荷泄漏（gateway.rs:4198）、reviewer 显式 hold_category——**与 pressure 分数无关**。
- 结论：`post_release.rs:60-69`（pressure→revision_failed）是修正后的真实口径（[2-01] 注释 + 测试钉死）；`threshold.rs:69`（blocked_by_safety_guard→pressure_risk_block 命中）、`auto_release.rs:328-330` 同款、`significance.rs` SAFETY_GATE_BLOCK_STATUS（pressure→blocked_by_safety_guard）三处是**失真口径**。影响：① threshold 生成器对 pressure gate 的命中率分子实为 claim-gate/relay 类事件数（与 pressure 阈值无因果）且漏掉真实软闸命中 → band 判定失真、可能持续产错误方向候选；② #152 安全反向门对 pressure gate 监控一个 pressure 阈值变化不会影响的状态 → 该 gate 的 safety regression 检查空转；③ replay 的 final_status_from_5gate 标签语义与生产不符（但 KE-02 两侧同口径对比内部自洽，send_delta 计算不受影响）。三 gate 同值观测（post_release pressure/human/emotional 三条 delta 恒相等）为已注释的观测分辨率限制。严重度：中（演化器启用后生效；当前默认关）。
- 三处同值观测中 human/emotional 与 pressure 合并未被注释直接说明——回写建议见 §3。

**11-15 manual_tags 不刷 updated_at → guide apply OCC 漏检**
- 亲读：`src/routes/contacts.rs:2482-2562`、`src/routes/guides.rs:416-426,860-898`。
- 证据：update_manual_tags 的 `$set` 仅 manual_tags/manual_tags_updated_at/manual_tags_by（contacts.rs:2506-2512），无顶层 updated_at；guide apply 的 contact CAS filter 带 `"updated_at": plan.contact_updated_at`（guides.rs:870，miss → guide_contact_changed:897）。preview 冻结（:421 contact_updated_at）后运营改 manual_tags 不会使 apply 失效；若 frozen plan 的 contact_set 含 tags 建议（guide 的 tags 走 manual_tags 写入），apply 会用冻结旧值覆盖运营刚改的标签且 OCC 不拦。
- 终裁：属实·缺陷（丢更新窗口）。严重度低-中：窗口=preview 与 apply 之间；影响面=manual_tags 单字段；代码无任何"有意让标签修改不作废 preview"的注释。修法：manual-tags 端点补刷顶层 updated_at，或 guide OCC 基线纳入 manual_tags_updated_at。

**11-11 guide preview readableChanges 丢弃 LLM 话术**
- 亲读：`src/routes/guides.rs:590-620`、`src/routes/shared.rs:1088-1148,1240-1246`。
- 证据：prompt（shared.rs:1094-1097）仍要求 LLM 输出 readableChanges 且 :1145 有专门约束行（"必须用产品语言"）；guides.rs:596-600 实际用 `frozen_plan.authoritative_changes` 的 `"{target} / {label}"` 机器拼接覆盖，LLM 字段全程未采集。
- 终裁：属实·缺陷。双重代价：LLM 每次 preview 空转生成一段被丢弃的文案（token 浪费）；前端展示机器拼接键名而非业务话术（UX 退化，违背 :1145 自己立的"产品语言"要求）。严重度低。修法二选一：prompt 删该字段，或渲染时采信 LLM 文案（以 authoritative_changes 兜底）。

**08-2 has_anchor 裸口径四处（补证完成）**
- 亲读：grep 四处 + `src/routes/knowledge/mod.rs:1343-1378`。主会话已核证 crud.rs:547；本次亲证 verify.rs:398、digest_inbox.rs:480、catalog.rs:209 三处仍为裸 `!source_anchors.is_empty()`，而 B3 统一谓词 `chunk_has_citable_anchor`（要求 anchor 自带非空 sourceQuote）仅在 verify 主闸（mod.rs:1351-1358 chunk_verify_gate_reason_for）强制。畸形 anchor（非空但缺 sourceQuote 键）在四个读点被误判"有锚"→ 报表/队列/inbox 漏报；verify 主闸口径正确故不会误放行。终裁：属实·缺陷（报表漏报级）。另发现 mod.rs:1376（apply_chunk_integrity 内）同为裸口径但方向 fail-safe（误判有锚时仍恒置 needs_review），无害。

**08-3 chat 死字段映射**
- 亲读：`src/routes/knowledge/chat.rs:2871-2898`。映射表含 routing_card/safe_claims/forbidden_claims/evidence_items 四个已删除字段（camelCase↔snake_case 双向）。这些键不在 DEFAULT_LOCKED_FIELDS（硬拒集）也不在 REVIEW_SENSITIVE（仅影响降级不阻写入），LLM patch 若携带会被写进 chunk 文档成为无消费者的死数据（source=Ai 恒降级 draft 的红线不受影响）。终裁：属实·缺陷（轻微数据卫生）。

**10-1/10-2 演化器两处死代码**
- 亲读：grep src+tests 全量。`schedule_post_release_review`（post_release.rs:76-99）与 `is_evolution_enabled_for`（runtime_flag.rs:80-101）均零调用（仅定义/re-export/注释镜像）；post_release.rs:3 注释"每次 release 后由 release.rs 调 schedule_post_release_review"与实际路径（release.rs 在事务内直接 insert post_release_review_document，10 号亲证 :112-128）不符。终裁：均属实·缺陷（死代码+注释失实）。灰度的实际语义=只影响哪些 contact 的 run 进演化 cohort（经 bucket_for_contact），与 runtime_flag.rs:48 注释的三路设想有差距。

**13-5 前端写操作防护不对称 + disable 后端无账号绑定**
- 亲读：`src/routes/contacts.rs:2052-2090`、13 号记录引用的前端谓词行为（本次 grep 复核前端零 createdAt 消费时顺带确认 store 结构）。
- 证据：disable_agent 后端 update filter 仅 `_id + workspace_id`（contacts.rs:2063），无 account 绑定、请求体为空（无 expectedAccountId）；对比 enable_agent 走 find_contact_by_id_for_account + expectedAccountId 必填。前端 disableAgent/analyzeProfile/runMemoryConsolidation 只判 selected 非空、clearReferral 无谓词（13 号亲读）。组合效应：快速切账号竞态窗口内可对旧选中联系人发 disable 并成功执行（停错人，可恢复）。终裁：属实·缺陷（低severity：影响可恢复、无数据损坏；与同文件高权威写端点的防护标准不对称）。

**12-1/12-4/12-5/12-6/09-1/10-12/08-9 注释与死代码族（合并）**
- 12-1：admin_ops_versions.rs:1235-1237 说 "TaxonomyEntry 无 workspace_id"，models.rs:3643-3645 该字段存在（m032 回填+default）——过期注释会误导隔离边界结论。
- 12-4：observability.rs:1342 行内注释仍称 "auto_resolved/applied/dismissed 之比是 sweep 命中率"，与 :1409-1410（historicalResolvedShare 正确口径，12 号亲证）矛盾。
- 12-5：observability.rs:1037 "workspace_id 强制 default" vs :1045 实际 admin.current_workspace。
- 12-6：chunk_locks.rs:398 broadcast_chunk_revised 死函数（12 处调用全为 _in 变体，本次 grep 复证）。
- 09-1：supervisor.rs:3 "8 个 worker" vs :34-51 实际 16 个。
- 10-12：evolution/mod.rs:62-65 启动日志 "W1 skeleton — empty tick by design" 过时。
- 08-9：knowledge_digest/labels.rs:2 引 "mod.rs:277-282"，实际 4 状态在 mod.rs:364-369（本次亲证）。
- 以上全部【属实·缺陷】（注释/死代码级，无行为影响），批量回写清单见 §3。

**07-2 AnswerStreamer**：knowledge_agent.rs:436 注释宣称"用 depth 计大括号层级忽略嵌套同名键"，:441-448 结构体无 depth 字段、:497-528 locate_answer_value_start 为朴素子串定位。行为风险=answer 轮 JSON 若在顶层 answer 前出现嵌套 `"answer"` 字符串键，token 流会提前把嵌套值当正文下发；当前 answer 轮 schema（citedChunkIds/sourceQuotes 元素无 answer 键）下不可达，属注释失实+潜在脆弱点。【属实·缺陷】（低）。

**07-6 fire-and-forget 失实**：knowledge_router.rs:1126-1128 注释 vs :1134-1147 for 循环顺序 await（`let _=` 仅吞错）——N 个 chunk id = N 次串行 DB 往返仍在调用路径上；对比 knowledge_agent.rs:317 的真 tokio::spawn。【属实·缺陷】（注释失实+轻微延迟）。

**07-7 缩进终止符**：block_parser.rs:93 `trim_start` + :128 `trimmed == FENCE_END_LITERAL`——左侧缩进的终止符也生效，与 doc"不在行首→当正文"口径不严；触发条件=JSON body 被 pretty-print 且某行恰为缩进的 `---END CHUNK---`（单行 JSON 产物不受影响），极低概率。【属实·缺陷】（边界）。

**09-6 mcp_logs 静默 + 送达核对**：mcp.rs:408/452/502 三处 `let _ = state.db.mcp_logs().insert_one(...)` 静默吞错属实。缓解层（本次亲读 outbox_dispatcher.rs:2584-2713）：`verify_delivery` 统一 reclaim 与 timeout 两窗口——文本先权威 chat_search_outbound（15s 独立超时）、出错回落本地 mcp_already_succeeded、仍无证据→Inconclusive；媒体走 media_send::media_delivery_verification（media_id 定位）；名片无权威查询恒 Inconclusive；`settle_ambiguous_send`：Delivered→post-hoc 收敛 sent，NotDelivered→重试/终态，Inconclusive→保守不自动重发。残余窗口=本地日志插入失败+权威通道同时超时的双重失效（文本），及名片路径可能漏发收敛 delivery_unknown（保守方向）。【属实·缺陷】（静默写失败）但重复发送风险已被统一核对层显著压缩。

**10-10 cold 审计虚耗 capacity**：cold_contact_worker.rs:128-137 每 tick 对每个 cold candidate 调 assign_account（结果弃用），内部写 account_scheduler_assignment 事件；account_scheduler.rs:107-110 count_today_assignments 把该事件计入当日分配数进 capacity 判定（:124-131）。冷扫描审计流与真实新分配混在同一计数——capacity>0 的账号会被冷扫描的重复审计虚占当日额度。【属实·缺陷】（触发=cold worker 开+账号配 capacity；capacity=0 不限则无影响）。

**14-4 digest targetRefs kind 枚举分裂**：prompts.rs:2297 教 LLM `"kind": "chunk|pack|proposal"`（3 值）；models.rs:5866-5870 target_refs 为自由 Vec<Document> 且字段注释含 5 值口径（14 号引述）；digest 解析仅校验 kind 存在+id 非空不校验枚举（08 号亲证）——kind 实为开放值。前端取并集 6 值防御（labels.ts:281-291 注释自曝，本次亲读）正确。【属实·缺陷】（后端两处口径漂移+无运行时枚举校验；前端已防御）。

**15-2 worker_reclaim 名不副实**：本次亲读 :40-72——`stale_running_task_is_recovered_to_retry` 仅 insert 后断言 `status=="running"`（:70），注释 :55-61 自认 worker tick 私有无法驱动。HP-1 stale 回收端到端行为在 tests/ 无可执行守护（依赖 lib 侧/CI 其它测试）。【属实·缺陷】（测试名与断言不符）。

**18-2 引用悬空**：glob `docs/superpowers/specs/2026-07-10*` 共 8 件，无 `full-system-test-findings.md`——被 07-11 remediation plan 引用的台账文件确实不存在。【属实·缺陷】（文档引用悬空）。

**其余轻量缺陷**（证据锚点）：07-8（gap_signals.rs:11-19 列 8 类 vs :338-354 第 9 类 dangling_anchor）、07-10（knowledge_router.rs:771-780 六行注释重复两遍）、09-4（llm.rs:288-289 把三样东西称"前两层"）、09-10（prompts.rs:956-973 draft spec 在已有流 append system_reset 版本但 :971-972 仅 published 才 publish）、09-12（config.rs:894-901 与 :916-923 测试体全同）、10-6（cohort.rs:113-115 注释 vs :138-142 `!contact.is_empty()` 前置）、10-8（prompt_critic.rs:55-61 vs revision.rs:26-32 双份 5-key 白名单）、10-9（threshold.rs:165-179 proposed_raw 存 clamp 后值）、10-14（planner/mod.rs:1253 `let _ = now` + :1261-1271 真实时钟）、11-1（conversations.rs:44-48 msgType/mediaRef 双写）、11-6（referral_cards.rs:150/208/279 BadRequest 表 not-found）、11-8（media_assets.rs 全文零 validate_account）、12-12（12 号亲证 outcomes_autonomy.rs:80-90,220-223）、13-1/13-2/13-3（本次 grep：wa.authed 仅 main.tsx:12 写点、openEventSource 仅 api.ts:119 定义、三 fixture 前端零 import）、14-1（labels.ts:62-71 对外零消费且与 steward REVIEW_CATEGORIES 同键不同文；legacy.tsx:1804 readableChangeItems 零调用）、15-9（happy_path_run.rs:623）、08-5（digest_inbox.rs:522/538 实现文案 vs :728/730 测试候选旧文案）。

### 2.3 重要【不成立】反证详情

**08-1 crud PUT 响应硬编码**：`crud.rs:793-794` **无条件** `patch.insert("title", json!(payload.title))`（title 为必填校验字段）；`chunk_revisions.rs:174-187` REVIEW_SENSITIVE_PATCH_FIELDS 含 title；`:189-196` op=Patch ∧ patch 任一键命中 → requires_review；`:207-214` 强制 draft+needs_review。∴ 该 PUT 恒降级，响应硬编码 `"status":"draft","integrityStatus":"needs_review"`（:832-833）恒与库内一致。原疑点设想的"仅 priority patch 不降级"路径不存在（title 恒在）。

**08-4 merge 不降级**：`wiki_edit.rs:546-547` 无条件 `target_patch.insert("source_quote", "")` + `insert("source_anchors", [])`——两键均 ∈ REVIEW_SENSITIVE（chunk_revisions.rs:184-185），op=Merge ∈ 触发集 → target 恒被 harness 打回 draft+needs_review+confidence 0。"verified 但无锚点"中间态不可达。

**17-Q7 零样本绿**：修复已落地——`tests/common/capability_evidence.rs:70-92`（16 号亲证）pass() 要求 attempted/llm_calls>0/branch/artifacts/assertions_run 全正；`scripts/check-capability-outcomes.py`（19 号亲证）22 个具名 case（含 redline×11）每个必须恰有 1 份 verdict==pass 且绑定当前 GitHub run/sha 的 outcome 文件；CI skip-gate 为硬门（ci.yml）。结构上禁止"没跑到就绿"。

**17-Q8 memoryCard 双不变量**：本次亲读 `src/agent/memory.rs:349-531`——SR-182 修复已落地：`:362-370` 合并规则明示"容量淘汰项迁入 recent 并在 extra.coreFactEvictions 留有界审计，不再静默消失"；`:438-446` 排序后 split_off(6) 的溢出项逐条 annotate_core_fact_eviction；`:506-514` coreFactEvictions 审计（≤20）；`:524-530` 溢出项去重后进 recent_facts（≤10）。"core=有限 prompt 投影窗口而非永久事实库"的语义收敛消解了原双不变量矛盾；前端 MemoryDetailView 渲染归档原排名（14 号亲证）。

**17-Q9 operator memory 撤销**：grep 亲证 `revoke_operator_memory` 存在于 sources_meta.rs（路由 :896-949，08 号亲读）、agent/memory.rs（实现）、routes/mod.rs（挂载 POST /knowledge/operator-memory/:id/revoke）；tests/sr181_operator_memory_revocation.rs 存在（15 号覆盖清单）；前端 MemoryDrawer 有 revoke 入口（14 号亲读）。digest spec R5.4 的承诺已兑现，17 号疑点基于旧状态。

**17-Q10 ISSUE-012 三层联动**：现行体系已重构——`gates.rs:469-474` 枚举注释亲证：R5.4（requiresProductKnowledge=true ∧ verified_chunks=∅ → blocked_unverified_product_claim）与 R5.3.a（claim_analysis 缺失/损坏且推断产品声明 → fail-closed blocked_by_safety_guard）两个 fail-closed 分支 + 独立 ClaimGate（09-2 主会话核证的代码内嵌审查器）。ISSUE-012 描述的旧"R5.7 反向门 verified_chunks=[] 永真"病根所在的机制已被 claim-driven 体系替代；runbook 悬置的"三选一"实际采纳=反向门 fail-closed + 独立语义审查双保险。

**18-6 EVOLUTION_ENABLED 张力**：config.rs:5-7 常量 `EVOLUTION_ENABLED_DEFAULT: &str = "false"` 与注释一致，:890-891 测试断言锁定。07-15 audit 记录的注释自相矛盾（:212 vs :215）在现行代码已不存在。

**18-7 S-01 语义分叉**：evolution/mod.rs:125-129 注释"enabled=false 或文档不存在 → 全员排除（worker 仍跑空 tick）"与 cohort.rs:138-142（None → false → 空 cohorts）完全一致。旧分叉已修。

**19-4 confirm 幂等**：management.rs:979-991——candidate.status ∉ {pending_confirmation, running} 时幂等回放 `{status, summary, toolCalls}`（含 canceled）。batch_c_management 的二次 confirm 断言与 HEAD 契约一致。

**08-10/12-14/09-11/10-13/13-7/13-13/09-5/18-12**：knowledge_agent.rs:672 clamp(1,4) 亲证（08-10）；guards.rs:260-266 五值与前端契约一致（12-14）；其余为记录内自答/纯理论/非矛盾，反证见汇总表。

### 2.4 【属实·设计】类要点（凭证锚点）

- **07-9**：catalog_rebuild.rs:444-448 filter 仅 status:"active" 亲证——persisted catalog 为管理导航面（agent 召回走 list_catalog 的 verified 门），若未来接进 agent prompt 需先加门（防护建议保留）。
- **08-8**：digest_inbox.rs:60-72 亲证同步合成（注释声明动机）；正确性由 attempt_generation 栅栏保证（08 号亲证 digest:1077），并发重算只烧 token。
- **11-9**：shared.rs:849-913 本次亲读——customerStage 分支内嵌（:849-871）与 else-if 独立分支（:888-913）对 intentLevel 的校验/降级/写入行为逐一一致，纯可读性问题。
- **11-10**：management.rs confirm 侧幂等回放（:979-991 本次亲证）+ 租约夺取协议使"确认竞速"窗口无害（needs-confirm 分支 matched=0 → Conflict，命令本身继续执行——采信 11 号对 :832-875 的亲读）。
- **12-9/16-3**：admin_ops_versions.rs:1235-1240 注释本次亲证——"无 RBAC 角色模型，'谁有权改全局字典'红线未定义，故不加拦截门（保持策略型孤儿现状），只补审计"。是显式记录的产品决策空缺，非回归。
- **16-5**：domain_profiles.rs:694-698 本次亲证 handler 守卫存在（仅 draft ∧ !current ∧ !active 可删 + delete filter 全条件 CAS :699-707）；缺的只是该拒绝路径的测试。
- **17-Q4**：auto_release.rs:36-45 `CURRENT_AUTO_RELEASE_POLICY_ENABLED: bool = false` 编译期常量 + 三重 AND 闸本次亲证；evolution.rs PUT flag 对 thresholdAutoReleaseEnabled=true 的 400 拒绝（12 号亲证）。代码侧已裁决"人工发布唯一路径"，冲突残留在 spec 文本未修订。

### 2.5 【仍存疑】

**18-8 m011 生产实证**：`m011_drop_legacy_sales_collections` 存在（migrations/mod.rs:47,348 本次亲证）且被 m035 复用（m035:9）；代码侧安全依赖=migrations 账册防重跑 + `APP_ENV` production 审批闸（migrations/mod.rs:583 由 19 号亲证 + migrations_idempotency 测试覆盖，16 号亲读）。**存疑原因**：其安全性最终取决于生产服务器（117）migrations 集合的入账状态与 APP_ENV 真值——本地工作区无法核验生产库，07-15 台账标注的"待生产实证"至今无回填记录。风险敞口有限（该迁移针对已删除的 legacy 集合），但按红线如实标注不可本地终裁。

---

## 3. 需回写源记录的修正清单

1. **10 号 §5-5 升级**：原记"读不出哪侧才反映生产真实（记疑点）"→ 本次已裁决：post_release.rs 的 pressure→revision_failed 是生产真实口径（gates.rs 软闸实现亲证）；threshold.rs:69 / auto_release.rs:328-330 / significance SAFETY_GATE_BLOCK_STATUS 的 pressure→blocked_by_safety_guard 为失真口径。10 号该条应从"疑点"改判"缺陷（三处失真）"。
2. **10 号 §5-7 定性**：cooldown 口径不一从"疑似有意（防抖动）"收敛为"缺陷（无注释、生成侧与发布侧矛盾造成'可发布却发不出'的管理员可见矛盾态）"。
3. **08 号 §5-1 关闭**：PUT 响应硬编码疑点不成立（title 恒在 patch → 恒降级），可回写"已终裁不成立+反证 crud.rs:794"。
4. **08 号 §5-4 关闭**：merge 恒降级（wiki_edit.rs:546-547 两敏感键无条件插入），疑点不成立。
5. **08 号 §5-6 升级**：execute_step 死路径从"疑似待重验"升为实锤，且补充 dismiss filter 漂移证据（:1539-1545 缺 account_id vs :662-668）。
6. **17 号 Q7/Q8/Q9/Q10 关闭**：四条均已被后续代码修复反证（capability-outcomes 硬门 / coreFactEvictions 审计 / revoke 全链 / claim-gate 体系），17 号应回写"已修复"标注，避免下游把旧疑点当现状引用。
7. **18 号 §5.2-6/7 关闭**：EVOLUTION_ENABLED 默认值张力与 S-01 语义分叉在现行代码均已收敛，历史 audit 记录的矛盾不再存在。
8. **12 号 §5-3 补充**：两处 bson DateTime 中，authorizationExpiresAt 已确认有真实前端崩溃路径（ResolvedEscalations.formatExpiry 对象回显），应从"是否被前端解析须由前端任务核证"升级为"缺陷·中（有可触发崩溃路径）"；createdAt 两处确认无前端消费（仅契约脏）。
9. **12 号 §5-2 精确化**：tripwire 缺失清单 11 文件本次逐一比对确认（其中 management_prompt_edit.rs 无 handler，实质缺口为 10 文件）。
10. **14 号 §5-2 补充**：USER_RUNTIME_PARAMETER_FIELDS 双份中 helpers 份的 label/detail 为死数据（消费面仅 key），漂移的实际爆炸半径=字段集分叉而非文案。
11. **19 号 §6-2 精确化**：expect 的 docstring（_lib.py:946-948）已把"BLOCKED=authoritative failure"写成显式设计——domain8 的问题应表述为"死代码 + 与 record_blocked 台账语义并存的两套 BLOCKED 分流冲突"，而非单纯"改造遗漏"。
12. **07 号 §5-2 补充**：AnswerStreamer 当前 answer 轮 schema 下嵌套 answer 键不可达（sourceQuotes 元素无 answer 键），风险定级可从"低"明确为"注释失实+防御缺失的潜在点"。
13. **16 号 §5-5 补充**：删 active profile 的 handler 守卫已亲证存在（domain_profiles.rs:694-698），缺口仅为测试覆盖。
14. **README/总台账**：本记录状态由"进行中"改"已完成"；已核证跳过的 13 条与本次 52 条缺陷、17 条不成立的分布可并入总台账缺陷清单。

---

## 4. 覆盖自证

**输入面**：07-19 号记录的"偏差与疑点"节全部读取——07/08/09/10/11/12/13/14/16/17(§5)/18(§5)/19 全文读入，15 号读取 §5 节（:915-943）与覆盖自证节；README.md 索引全文。合计疑点条目 163 条（含 13 条已核证跳过）。

**当场亲读的源码/脚本/配置**（本次终裁执行的 Read/Grep，均为 2026-08-13 工作区）：

- **evolution**：mod.rs（:54-71,125-144,220-264）、replay.rs（:1-260）、threshold.rs（:100-129,160-189,255-304）、auto_release.rs（:36-45 经 grep 复核,:284-344）、release.rs（:258-300）、cohort.rs（:108-157）、post_release.rs（:56-99）、prompt_critic.rs / revision.rs（EVOLVABLE 双定义段）、runtime_flag.rs（grep :14,80）。
- **agent**：prompt_shadow.rs（:355-395）、review/gates.rs（:33-46,82-110,160-205,995-1005,1371-1455 经 grep 上下文）、review/mod.rs（:1385-1421 经 grep）、gateway.rs（blocked_by_safety_guard 全写点 grep）、run_envelope.rs（终态集 grep）、types.rs（hold 闭集 grep）、guards.rs（:260-266）、memory.rs（:221-236,349-531,682-690 经 grep 上下文）、knowledge_agent.rs（:430-529,:672 及 max_rounds 消费点 grep）、knowledge_router.rs（:770-810,1120-1149）、knowledge_tools.rs（:130-158,438-506）、outbox_dispatcher.rs（:150-158 经 grep,:2584-2713）、escalation/ledger.rs（authorization_expires_at 写点 grep）。
- **knowledge_wiki / knowledge_task / knowledge_digest**：chunk_revisions.rs（:174-217）、block_parser.rs（:85-135）、gap_signals.rs（:1-35 + dangling_anchor grep）、catalog_rebuild.rs（:436-465）、knowledge_task/mod.rs（:650-690,925-999,1202-1300,1531-1585）、knowledge_digest/labels.rs（全文 35 行）、knowledge_digest/mod.rs（:361-369 经 grep）。
- **routes**：mod.rs（:1080-1214）、auth.rs（:240-301）、principal_escalations.rs（:26-65）、ask_human_inbox.rs（:14-78）、admin_ops_versions.rs（:1230-1247）、observability.rs（:1030-1049,1335-1348）、admin_state_policies.rs（ACTION_VALUES 消费 grep）、chunk_locks.rs（broadcast 调用方 grep）、contacts.rs（:334-356,2052-2090,2230-2270,2482-2562,3349-3422）、guides.rs（:416-426,590-620,860-898,1113-1123）、conversations.rs（:36-53）、products.rs（:300-335）、referral_cards.rs（错误码 grep）、evaluations.rs（:390-403）、media_assets.rs（validate_account 零命中 grep）、shared.rs（:845-916 + readableChanges grep :1088-1148,1240-1246）、management.rs（:977-996）、domain_profiles.rs（:680-710）、knowledge/{crud.rs:545-549 经 grep+:782-835, verify.rs:29-55+:396-398, chat.rs:198-220+:2871-2898, wiki_edit.rs:505-575, digest_inbox.rs:55-76+:478-482+:515-545+:725-731, mod.rs:1094-1140+:1343-1378 经 grep, import.rs:2728-2732 经 grep}。
- **基础设施**：config.rs（:1-14,497-524,655-662,695,890-924）、supervisor.rs（:1-55）、mcp.rs（:14-21,381-386,408,452,502 经 grep 上下文）、llm.rs（:284-291,503-506,758-760,808-813 经 grep 上下文）、prompts.rs（:940-976 + targetRefs grep :2295-2325）、main.rs（connect_info grep :381）、models.rs（:3640-3655,4511-4519,4660-4668,5866-5872 及 workspace_id 抽样）、db/migrations（m011 grep）、planner/mod.rs（:1245-1274）、cold_contact_worker.rs（:125-146）、account_scheduler.rs（:100-131）。
- **前端**：features/quality/index.tsx（sampleRate 段 grep）、features/knowledge/cockpit/AutoVerifyPanel.tsx（:45-70）、features/knowledge/labels.ts（:279-294 + REVIEW_CATEGORY grep）、features/user-ops/legacy.tsx（:138-168 + readableChangeItems/USER_RUNTIME grep）、stores/userOpsDomainHelpers.ts（:1-60）、features/ask-human/ResolvedEscalations.tsx（:21-60）、features/ask-human/index.tsx（createdAt 零消费 grep）、lib/inboxApi.ts（:8-14）、main.tsx / lib/api.ts（wa.authed/openEventSource grep）、三 fixture import 零命中 grep。
- **tests / scripts / docs**：tests/worker_reclaim.rs（:40-80）、tests/happy_path_run.rs（:615-624）、tests/media_asset_crud_integration.rs（:1-25）、tests/domain_profile_e2e.rs（:452-480）、tests/{released_by 分布, schedule_post_release/is_evolution_enabled_for 零命中} grep；scripts/biz-test/_lib.py（:905-979）、batch_a_domain8.py（BLOCKED 段）；.env.example 差集 grep；docs/superpowers/specs/2026-07-10* glob。

**方法说明**：每条【属实·缺陷】/【不成立】结论均含本次当场亲读的 file:line 证据；【属实·设计】类中少数条目（11-2/11-12/13-6/13-11/13-14/13-15/14-3/15-3/15-4/15-6/15-7/15-8/15-10/16-6/16-8/16-9/16-10/17-Q3/Q5/Q6/Q11/Q12/18-1/3/4/9/10/19-3/5/6/7/9/10/11）为源记录已附逐行证据的记录性/自答型条目，本次核验其关键锚点或采信其亲读记录并注明；跳过的 13 条均在源记录中带主会话核证标注。唯一不可本地核验项（18-8 生产库状态）如实标注【仍存疑】。
