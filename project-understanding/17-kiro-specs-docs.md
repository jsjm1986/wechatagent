# kiro specs 与 docs 顶层深读记录（核证日期 2026-08-13）

> 阅读范围：`.kiro/specs/` 全部 5 个 spec 目录（requirements.md + design.md + tasks.md，universal-test-coverage 无 design.md）+ 顶层 `task-status-manifest.json`；`docs/` 顶层全部 26 个 .md；`docs/smoke/`（5 个文件全读）与 `docs/system-review/`（26 个文件，小文件全读、4 个超大文件读头部+结论段）。
> 记录纪律：文档内容如实转述并标注出处；本人无法核实代码处标注"**文档声称，未对码**"；少量代码锚点做了亲验（见 §5 与 §6），核验命令均为只读。
> 已知代码事实对照基线（父任务给定 + 本次亲验）：① 5 闸已改为分数闸（亲验 `src/agent/review/gates.rs` 存在 `human_like_rewrite_below / emotional_value_rewrite_below / pressure_risk_block_at`；`src/agent/runtime.rs` 确认 `fact_risk_block_at` 承载 `hallucinationBlockAt` 别名）；② 路由数 235（亲验 `src/routes/mod.rs` 中 `.route(` 恰 235 处）；③ 迁移到 m058（亲验 `src/db/migrations/` 存在 m001–m058 共 58 个文件，最新 `m058_llm_provider_active_invariant.rs`）；④ "user.reply.task 已退役"这一给定对照点，本记录初判"不符"，**经主会话全量无截断搜索裁决：初判为误报**——`"user.reply.task"` 在 src/ 的全部 16 处命中均为种子 spec 定义（prompts.rs:1302）、prompt_guard 治理面、单元测试 fixture，**无任何生产加载/调用点**；生产决策链只调 `user.reply.fast.task`（decision.rs:460,1321），`agent/mod.rs:230` 注释明写 "the retired full task"。精确表述：该模板处于"种子包仍种入 DB、编辑治理仍覆盖、运行时无消费方"的退役态；user-ops tool-loop（reply_with_tools_loop）的物理删除是另一件独立的退役事实。详见 §5-Q1（已修正）。

---

## 1. 文档地图（性质 + 一句话）

### 1.1 `.kiro/specs/`（5 spec + 1 manifest）

| 文件 | 性质 | 一句话 |
|---|---|---|
| `task-status-manifest.json`（238 行） | **活跃权威**（asOf 2026-07-24） | 自声明为交付状态唯一权威（"statusAuthority"），覆盖 autonomy/evolution/hardening 三 spec 的任务状态（partial / production_wired / implemented / sunset_not_shipped），由 `scripts/check-task-status-manifest.py` 在 CI baseline 门强制。 |
| `agent-autonomy-loop/requirements.md`（593 行） | **历史存档**（顶部 2026-05-25 Sunset Notice） | 自治回路 R0–R13 + N1–N7：Run Envelope、9 字段自治协议、Single-Shot Revision、tool-calling 知识协议、verified knowledge 强约束、MemoryFact 强类型、双层标签、outbox 发送闭环；销售域字段章节已下线。 |
| `agent-autonomy-loop/design.md`（1648 行） | 历史存档（同 sunset notice） | 把 R0–R13 映射为 W0–W6 七波实施、数据模型（outbox/taxonomies/candidates schema）、finalize_review_for_send 单一安全汇总层、幂等 key 公式与 6 个关键设计决策。 |
| `agent-autonomy-loop/tasks.md`（474 行） | 历史存档 | 72 个任务全部为 `[~]`（顶部声明 `[~]` 仅是历史规划标记、非完成态，权威=manifest）；含 SR-179 状态权威注记 + sunset notice。 |
| `user-ops-agent-hardening/requirements.md`（378 行） | **历史存档**（2026-05-25 Sunset Notice） | 20 项修复需求（HP-1..4 产线 bug / MP-5..10 设计层 / LP-11..17 代码质量 / S-18..20 战略），含 worker 回收、reaction claim 锁、RunBudget、字符串 fact-risk 兜底、状态机 allowedFrom、memoryCard 分层、dry-run。 |
| `user-ops-agent-hardening/design.md`（1403 行） | 历史存档（同 sunset notice） | 20 项需求的模块级方案 + migrations 框架 + 5 批部署顺序 + 6 条 Correctness Properties。 |
| `user-ops-agent-hardening/tasks.md`（455 行） | 历史存档 | 24 任务 5 批全部 `[~]`；顶部同样挂 SR-179 权威注记 + sunset notice。 |
| `agent-self-evolution/requirements.md`（270 行） | **半活跃**（无 sunset notice；tasks 有历史注记） | M4 自我演化：阈值自适应 + Prompt 演化，shadow eval 为发布前置、admin 一键发布、R9.6 本期禁止自动发布、R9.3 Critic prompt 不自我演化。 |
| `agent-self-evolution/design.md`（767 行） | 半活跃 | `src/evolution/` 模块边界（禁止依赖 gateway/outbox/mcp）、4 张新表 schema、prompt_templates 多版本化、prompt_pack_version LRU 失效、release/rollback 事务。 |
| `agent-self-evolution/tasks.md`（430 行） | 历史存档 | 全 `[~]`；顶部 4 条注记：①2026-05-25 gate_key 与运行时 3 闸解耦说明；②Historical Done Notice（2026-05-27，被 SR-179 manifest 取代）；③Phase A 弃用项（2026-05-26 commit 031d442 删除 4 个 evolution E2E + significance PBT）；④CI 已接入 check-evolution-isolation + check-no-model-hint。 |
| `knowledge-digest-workstation/requirements.md`（169 行） | **活跃规格**（design 有 2026-05-25 术语更正注记） | 知识库日报工作站：每日 09:00 digest worker、三栏布局（目录树 25%/画布 45%/chat 30%）、long-running task、operator memory 物理隔离、4 个 tool。 |
| `knowledge-digest-workstation/design.md`（363 行） | 活跃规格 | 11 个文件改动清单、数据流闭环、3 条 PromptSpec、SSE、失败模式表、文案禁词防御。 |
| `knowledge-digest-workstation/tasks.md`（130 行） | 活跃规格（无 checkbox，分 5 Phase） | Phase1–5 各自验收（lib ≥80/86/92/96——旧基线时代数字）+ 全局验收 10 条 + 回滚方案。**不在 task-status-manifest 覆盖范围**。 |
| `universal-test-coverage/requirements.md`（139 行） | **活跃规格**（2026-06-16） | 通用化后测试体系重建：R0 堵假绿总开关 → R1 judge profile 化 → R2/R2.5 跨域闭环+主动半场+治理命门 → R3 深命门 → R4 知识库 → R5 动态博弈发现线（两线定位：固定回归进 PR 门 / 动态 nightly 不进 PR 门）。 |
| `universal-test-coverage/tasks.md`（111 行） | 活跃规格 | 全部任务未勾选 `[ ]`；R0.1 指出 CI 有 17 处 `|| 'nvapi-...'` 明文 key 回落（=机密泄露须轮换）。无 design.md。 |
| `universal-test-coverage/real-llm-findings-2026-06-18.md`（302 行） | 历史存档（findings 快照） | 18 个真模型测试文件清单 + agent 短板归集（C2 helpfulness 偏低可信度高、C1 已证伪、D1 已修、J1–J6 评判输入失真、T1–T4 探针缺陷）。 |
| `universal-test-coverage/audit-status-manifest.json`（约 240 行） | **活跃权威** | 把 47 域 deepread 审计结果全部改判 `inconclusive`（legacy_inconclusive：无冻结 run manifest/模型指纹/逐 claim 证据/静默丢失败域）；summary：total 47 / complete 0 / inconclusive 47。 |
| `universal-test-coverage/` 其余（biz-domains json 、audit-anchors json、deepread-verify-result json 760KB、deepread-verify-workflow.mjs） | 历史数据/脚本 | 47 域清单、锚点、被改判 inconclusive 的原始审计结果、生成 workflow（其缺陷被 SR-183 记录）。仅概览未逐字读。 |

### 1.2 `docs/` 顶层（26 个 .md）

| 文件 | 性质 | 一句话 |
|---|---|---|
| README.md（48） | 活跃索引（部分陈旧） | 阅读顺序 + 文档维护规则；"当前阶段"描述停留在早期能力清单。 |
| product-modules.md（250） | 活跃产品文档 | 8 个一级模块划分；群/朋友圈默认不做自动动作；知识库三件套 + 渐进披露。 |
| architecture.md（444） | **活跃**（快照 2026-07-24，commit d60d3d8 + SR 收口工作树） | 现行架构：单 Rust 进程 + durable Outbox + 二次安全门 + MCP；14 条 supervised worker 清单；evolution/digest/wiki 三个子系统流程图与隔离红线。 |
| ai-agent-system.md（267） | 半活跃（含大量"建议/方向"性质段落） | Management vs Operations 两类 Agent、Prompt Stack v2、工具风险分层、Command Center UI 原则。 |
| agent-policy.md（749） | **活跃（与代码最同步的一篇）** | 自动化边界、quiet hours、运营大脑公式、评审闸门现状（顶部注明旧 enforce_* 已移除）、辅助模式/名片引荐、自我演化章节、自学习采集管道两阶段、digest、knowledge-wiki 方法论、Phase 0→E5-T1 changelog。 |
| data-and-api.md（707） | 活跃（快照式） | 声明不再手抄 collection 全集（权威=src/db/mod.rs，"当前 61 个"typed accessor）；evolution 4 表、digest 3 表 + 路由、knowledge-wiki 字段/路由/import-apply 事务协议、Phase G 前端交付。 |
| development-roadmap.md（154） | 历史存档（早期路线图） | Phase 1–6 规划；不含 evolution/digest/wiki 等后来落地的系统。 |
| frontend-design-system.md（194） | 活跃设计规范（频道清单陈旧） | 白色企业控制台语言、CSS token、四级层级、响应式规则；"Current channel model" 仅列 5 频道（实际已 13+）。 |
| knowledge-wiki.md（202） | **活跃** | 9 类 wiki_type 决策表、provenance 矩阵（5 种 source）、lifecycle 状态机、三层写入保护、feedback worker 9 类 lint、domain_schemas、D3 redirect 已落地注记（2026-06-17）。 |
| ci-known-gaps.md（17） | 活跃（待决策项） | G1：纯前端 PR 绕过 check-no-human-takeover lint（paths-filter 把 baseline job 跳过），修法三选一待用户拍板。 |
| sunset-plan.md（153） | 活跃（含已兑现注记） | 自治协议灰度开关 D/D+7/D+14/D+21 下线时间表；**D+21 已兑现（2026-06-23）注记：knowledgeRoutingMode/knowledgeMaxToolLoops 字段与 reply_with_tools_loop（src/agent/tool_loop.rs）、dispatch_tool_call user-ops 半边已删除**；PR review 9 条"双轨违规"检查项。 |
| real-task-runbook.md（1562） | **历史存档**（顶部 sunset notice：不再随主线更新） | kefu-b × Jsjm 真实流量压测底稿：北极星 4 问、S1–S15 场景矩阵、§4.0 全量覆盖矩阵、Round 1–16 迭代日志（baseline 381→416）、ISSUE-001..013。 |
| real-task-loop-prompt.md（132） | 历史存档（配套 runbook） | /goal 启动 7 步、Round 号四分支决策表、4 条流程硬约束、反模式清单。 |
| real-llm-test-authenticity-audit.md（91） | 历史存档（2026-06-16 审计） | 三大假绿根因（缺 key 静默绿 / transient-skip 吞 / judge 失败照 pass）+ 逐测试定级表 + P0–P2 修复清单。 |
| test-paradigm-llm-driven-analysis.md（78） | 历史存档（含定位修正） | "固定脚本假动态"范式天花板分析；第六节修正为两条线（固定回归 PR 门 + 动态发现 nightly），权威定位归 universal-test-coverage spec。 |
| universal-domain-test-gap-audit.md（79） | 历史存档（2026-06-16 体检） | 通用化能力 × 测试覆盖矩阵；P0=judge profile 化 / 非销售域骨架 / H11+C2 命门；"业务闭环对齐"断言表。 |
| universal-domain-base-extensibility-audit.md（147） | 历史存档（2026-06-18，含证伪附录） | 4 扩展轴审查：A1' 单 domain 锚定、B1 维度扩展点散落、C3 无中央接线点、D3 relationship_type 人工标注瓶颈；附录记录被证伪的初判。 |
| universal-domain-base-robustness-roadmap.md（120） | 半活跃路线图（含落地进度注记） | 5 主题排序；2026-06-18/19 注记：主题 1+2 核心已落码（dimension_registry + 三写入路径校验 + D3 LLM 识别保守闭环）；五闸可配=过度设计已砍。 |
| mcp-deployment-guide.md（260） | 活跃运维文档 | MCP Server（117.72.54.28:3001）对接：env、webhook URL 配置、HMAC-SHA256 签名、登录流程、7 FAQ。 |
| mcp-integration-audit-2026-07-07.md（158） | 历史存档（审计报告） | 活体测试全绿 + 逐项兼容核对；关键缺口=Workspace Key 需 account_alias 注入（后已修）。 |
| mcp-integration-completion-report.md（204） | 历史存档 | Workspace Key 支持 + P0 缺口补全（登录流程/文档）两阶段完成报告。 |
| mcp-integration-final-delivery.md（195） | 历史存档 | MCP 接入最终交付总结（3 commits）+ 部署清单 + 待验证项。 |
| remaining-issues-summary.md（226） | 历史存档（2026-07-07 快照） | P0（webhook URL 配置/端到端/前端集成）/P1/P2 剩余问题与工作量估算。 |
| DEPLOYMENT-STEPS.md（483） | 活跃运维文档（含实录） | 11 步部署手册 + **2026-07-25 生产发布实录**（后端 SHA-256 539eff…、m049 applied、12/12 健康）。 |
| FINAL-SUMMARY-2026-07-07.md（324） | 历史存档（含 07-25 补充节） | 端到端测试 + MCP 接入总结：13 频道四方对账、B-1 升档预算修复（run_token_budget_escalated=100000）、lib 1814/前端 448 全绿。 |
| wechatagent-project-knowledge.md（134） | 特殊：知识库测试素材兼产品自述 | 项目定位/核心原则/安全表达边界/典型客户问题回应方向；文末明示"用这份文档导入知识库后 Agent 应该能……"。 |

### 1.3 `docs/smoke/`（5 个，全读）

| 文件 | 性质 | 一句话 |
|---|---|---|
| 2026-07-05-full-project-smoke-findings.md（64） | 历史存档 | lib 1814/0；三家 LLM provider 同时不可用致业务冒烟 BLOCKED；影子验证 HIGH bug（字段契约漂移）已修 6822ffb；"已核实知识解锁产品红线"capstone（grounding 4→10）。 |
| 2026-07-05-newuser-journey-four-way-audit.md（165） | 历史存档 | 13 组频道四方对账（UI/源码/stdout/Mongo）；B-1 升档预算 CONFIRMED→已修（方案 c，upgrade 上限 100000）；3 项 MCP C 类 BLOCKED。 |
| user-ops-smoke-runbook.md（350） | 半活跃 runbook | Phase 0→E 收口验证：启动顺序、HMAC 签名脚本、五条 webhook 用例、reaction_hint/operator_memory/negative_example 三链路验证、outbox 5 状态闭环；checklist 引用 baseline 350/33。 |
| biztest-article-edu.md（33） | 测试素材 | 教培课程介绍（故意含"保证学会/包教包会"违规宣传语，供红线测试）。 |
| knowledge-smoke-doc.md（31） | 测试素材 | OpsDesk SRE 值班手册节选（刻意非销售内容，验证 AI 不硬塞销售模板）。 |

### 1.4 `docs/system-review/`（26 个文件）

| 文件 | 性质 | 一句话 |
|---|---|---|
| README.md（9，全读） | 活跃索引 | 全系统 100% 审查目录说明；冻结对象 PR #223 head `12d99b3`；SR-001~183 两轮复审完成 → 36 项人类决策面。 |
| review-plan.md（152，全读） | 活跃协议 | 100% 阅读完成定义、FACT/CONTRACT/RUNTIME_UNVERIFIED/UNKNOWN 标记、阶段 0–10 + 11（真实业务复审）+ 12（反过度工程复审）。 |
| baseline.json（15，全读） | 冻结元数据 | PR #223：base 9d28b73 / head 12d99b3 / 冻结 2026-07-17 / 跟踪 1243 文件。 |
| findings.md（1643，头 60 + 尾 60） | **活跃发现库** | SR-001~183：头部 SR-001/002 凭证泄露 P0、SR-004 集成不阻断、SR-006 文档落后；尾部 SR-177 webhook durable 缺口、SR-178 红线硬门零样本假绿、SR-179 任务表账实不符、SR-180 auto-release 契约冲突、SR-181 operator memory 无撤销、SR-182 memoryCard cap 矛盾、SR-183 47 域审计不可复现。 |
| two-pass-review-ledger.md（480，头 40） | 活跃账本 | SR→HC 一对一映射规则 + HC-003/014 等后续实施证据（m048 单 current、SR-056 DomainSchema、SR-094 typed runtime 边界等）。 |
| human-confirmation-checklist.md（642，头 70 + 尾 44） | **活跃决策清单** | 36 个 HC 决策项：HC-001 凭证轮换（未完成硬门 8 条）、HC-002 Evolution 默认关闭（已实施）、HC-003 迁移一致性（SR-008 已部署）、HC-004 正式多租户契约、HC-033 文档对齐（350/33、13 个 spawn_supervised）、HC-034/35/36 前端修复已完成。 |
| reading-notes.md（2101，头 30 + 尾 32） | 审查笔记 | B01 起批次亲读事实（main.rs 启动序列、12 类 worker@PR#223、必填 env 仅 MCP_API_KEY/OPENAI_API_KEY）；尾部为两轮复审方法与 R01 首批修复（HC-002/034/035/036）验证。 |
| architecture.md（1584，头 40 + 尾 36） | 审查产物 | PR #223 时点架构图 + 启动顺序；尾部为 SR-172/173 的调用链图与 SSE 状态机建议。 |
| data-model.md（142，头 30 + 尾 28） | 审查产物 | 集合分组×租户键×约束总表 + 逐集合风险索引（SR-109~132 等）；尾注声明"集合—全部读写方"反查未完成。 |
| file-ledger.csv / automation-read-evidence.json / build-ledger.ps1 / check-ledger.ps1 / update-ledger.ps1 | 机器台账/脚本 | 逐文件哈希状态账、自动化脚本读取证据、账本维护脚本（非 md，未逐字读）。 |
| automation-script-review.md（56，全读） | 审查产物 | 65 个自动化脚本全读结论：biz-test run_all 不传播失败退出码、`cache_hit` 被当模型成功证据、SSH AutoAddPolicy 等安全边界。 |
| hc001-credential-rotation-runbook.md（139，头 40） | 活跃运维手册 | 凭证轮换 5 条完成标准 + 角色授权边界 + 分步操作（值只经 owner-only 文件/stdin）。 |
| security-incident-hc001-2026-07-30.md（43，全读） | 活跃事件记录 | 已确认事实（公开仓库两个历史提交树含有效 LLM 凭证、69 个 CI run 日志泄漏 1795 次、服务器 10 文件+29 Git blob+17 压缩载体命中）+ 已完成无中断控制 + 8 条未完成硬门。 |
| production-release-2026-07-25.md（141，头 40） | 部署证据 | SR-008/m049 生产切换实录（新后端 SHA-256 539eff…、12/12 健康、SR-165 provider 端点 530 阻断未记通过）。 |
| production-release-2026-07-27-hc014.md（59，全读） | 部署证据 | DomainProfile 协议发布（SR-043/044/072/073/074/089/090）；SR-072 Policy 短暂 fail-open 边界仍开放。 |
| production-release-2026-07-27-hc015/016/019/020/021/022/023.md（11–47，全读） | 部署证据 | Taxonomy 身份 4/4、Shadow 零副作用 1/1、关系审核/成交审批/Lesson 晋升/统一收件箱、Management execution_unknown 协议 2/2、Campaign 11/11、联系人纳管 13/13、Guide v3 1/1 各专项服务器真实 rs0 验证；每篇均明确"不等同真实模型/浏览器验证"边界。 |
| production-release-2026-07-27-sr056.md（28，全读） | 部署证据 | DomainSchema 精确版本激活 1/1（m044 applied + 双唯一索引）。 |
| production-release-2026-07-27-sr129130.md（53，全读） | 部署证据 | Auto-Verify DTO deny_unknown_fields + Review Chat OCC 零写；5 文件精确发布。 |
| production-release-2026-07-28-hc026-m039.md / hc029.md / wave1.md（23/27/39，全读） | 部署证据 | 评测预算隔离+m039 scoped 索引、SR-135/136 主动触达配额+ImportJob fencing、Wave1（SR-097/132/138/139/141）；wave1 明确记录真实模型门未通过（provider 530/超时/欠费）。 |
| production-release-2026-07-30-hc004-sr025.md（50，全读） | 部署证据 | 账号日发送软上限查询补 workspace_id；切换前 migrations=57 条（时点数据）。 |

---

## 2. 五个 kiro spec 深读

### 2.1 agent-autonomy-loop（用户运营 Agent 自治回路）

**Sunset/历史注记（三件套全部带）**：requirements/design/tasks 顶部均有 **⚠️ Sunset Notice (2026-05-25)**——本 spec 写于销售域知识库时代，文中 `customer_stage / intent_level / objection_type / fact_risk / pressure_risk / product_accuracy / safe_claims / forbidden_claims / routing_card` 相关章节（含 R8.x / P4 / P6 / P7）属**已下线的销售域形态**；运行时已在 knowledge-cleanup 收敛为 3 闸（`enforce_knowledge_grounding / enforce_hallucination / enforce_run_budget`，"详见 src/agent/guards.rs"），业务可变字段全量下沉 `domain_attributes` + `DomainSchema`；**spec 保留作历史档案，不再作为代码对照源**。tasks.md 另有 SR-179 权威注记：`[~]` 只是历史规划标记，唯一状态权威是 `../task-status-manifest.json`。
（注意：sunset notice 中"3 闸 enforce_* 见 guards.rs"的说法本身也已被 agent-policy.md 顶注推翻——旧 `enforce_*` 函数已移除，在线入口是 `review::review_passed / classify_dual_gate / route_dual_gate / finalize_review_for_send`。见 §5-Q2。）

**Requirements（R 编号全表）**：

| R | 主题 | 要点 |
|---|---|---|
| R0 | Run Envelope | 入口先 insert_one lifecycle="started"（先于任何 LLM 调用、try/catch 之外）；后续只 update_one 禁 re-insert（run_id 唯一索引）；lifecycle 闭集 7 值（started/running/completed/failed_before_decision/failed_after_decision/aborted_by_budget/aborted_by_external_signal）；panic 兜底；(account_id, lifecycle, started_at) 索引。 |
| R1 | 自治协议 9 字段 | userUnderstanding 等 9 个 String 字段（≤600 字符）；7 个恒必填 + whyShouldReply/whySkipReply 互斥必填（≥10 Unicode 字符含 ≥6 汉字）；低风险常规轮允许 5 字段 "unchanged" 短形式；关键变化轮 ≥20 字符；`decision_phase ∈ {tool_calling, final}` 门控（tool_calling 中间轮跳过 R1.3–R1.6）。 |
| R2 | SelfCritique + Single-Shot Revision | Review 新增 needsRevision/revisionDirection(≤1024)/shouldHold/holdReason/holdCategory/selfCritiqueAddressed；单 run 最多 1 次 revision；holdCategory 严格三选一 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`，**严禁** held_for_human 等；R2.7 = check-no-human-takeover CI lint 的需求源头。 |
| R3 | Rust 不兜底业务字段 | 7 个必填字段（risk_level/knowledge_need/run_mode/autonomy_mode/needs_review/operation_state/consolidation_needed）；枚举保持现状（risk_level 无 critical；knowledge_need=not_required/required/insufficient；run_mode=fast_chat/memory_candidate/knowledge_grounded/high_risk）；新增 autonomy_mode=auto/assisted/blocked；预算超额二态（needs_review=true→blocked / false→low_risk 通道 + `local_review_low_risk_only`）。 |
| R4 | Knowledge 工具化 | toolCalls 协议（knowledge.list_catalog/search/open_slice 三工具）；`reply_with_tools_loop` 多轮派发（MAX_TOOL_LOOPS 默认 3，[1,5]）；tool_calls_used 计入 RunBudget（默认 6，[1,16]）；单轮 toolCalls ≤4；连击 3 错强断；循环总超时 30s → tool_loop_timeout；结果注入 ≤8000 chars；toolTrace ≤32 条；classic_router 灰度回退（R11 sunset）。 |
| R5 | Verified knowledge 强约束 | 唯一判定 = `OperationKnowledgeChunk.integrity_status == "verified"`（禁新增 verified:bool）；requiresProductKnowledge=true ∧ verified_chunks=∅ → fact_risk≥6 + block（blocked_unverified_product_claim）；claim_analysis 缺失走 R5.3.a fail-closed（knowledge_need=required / used_knowledge_ids 非空 / string marker 命中任一 → blocked_by_safety_guard）或 R5.3.b 仅标 risk；safe_claims 反向门只加 risk 不单独 block。 |
| R6 | MemoryFact 强类型 | id=UUIDv4 / text 1..500 / evidence / confidence / importance / mayExpire / deprecatedAt / sourceMessageIds(≤5) / sourceRunId / createdAt / updatedAt；OperatingMemory.memory_card 整层 Document→MemoryCardTyped（coreFacts cap6 / recentFacts cap10 / deprecatedFacts cap20 / extra 兜底）；`#[serde(untagged)] MemoryFactRepr{Plain,Structured}` 一次性兼容（D+14 移除）；迁移 `2026_05_005_memory_facts_to_structured`。 |
| R7 | Memory 冲突处理 | consolidator 输出 deprecatedFacts（按 id 引用非 text）/conflicts；id 不存在→warning 不写入；cap20 按 deprecatedAt 升序丢弃；conflict winner≠none 写 `memory_conflict_resolved` 事件；prompt 注入最近 5 条 deprecatedFacts。 |
| R8 | 双层标签 | system_taxonomies 严格字典（customer_stage/intent_level/objection_type，唯一索引 (scope,kind,value.id)）；agent_generated_signals 自由层不进聚合；taxonomy_candidates 候选审（**不阻塞 run** 是与旧硬阻塞的关键差别）；operation_state 仍严格走状态机校验（与 R8 互斥）；admin approve/reject API；迁移 seed 全局字典。 |
| R9 | 审计字段 | AgentRunLog 新增 revisionApplied/revisionReason/pre+postRevisionSummary/selfCritique/autonomyMode/finalReviewStatus；finalReviewStatus 闭集 10 值（approved/revision_applied_approved/revision_failed/held_by_ai_policy/blocked_by_safety_guard/ai_waiting_for_more_context/blocked_by_required_field/blocked_by_budget/blocked_unverified_product_claim/legacy_mode_unchecked）；R9.10.e 写库拒收闭集外值。 |
| R10 | 前端自治监控 | /outcome/autonomy Tab：7 指标卡（revision_trigger_rate 等）+ AI 暂缓分布图（严禁"人工接管"分类）+ 近 50 条 revision 列表；total_runs=0 时比率返回 null。 |
| R11 | 一次性迁移 + sunset | 灰度开关 autonomyProtocolEnabled / knowledgeRoutingMode（默认 auto_tool_loop）D+14 物理移除；D+7 升级度量；D+21 移除迁移脚本；**R11.6 基线门：cargo test --lib ≥78 + 4 PBT（state_transition_pbt/memory_card_invariants/string_fact_risk_guard/llm_retry_jitter）累计 ≥33**（这是当时的数字与 PBT 集合，后来演进，见 §5-Q3）；R11.9 任何新灰度开关不入 R11.5 即协议违规。 |
| R12 | PBT P1–P7 | tests/autonomy_protocol_pbt.rs 七条性质（必填/revision 上限/预算不发送/产品声明/记忆可追溯/candidate 不阻塞/工具循环不死锁），每条 ≥64 例 ≤60s；happy_path 扩 2 case。 |
| R13 | 可靠发送闭环 | agent_send_outbox schema + 4 索引（含 idempotency_key 唯一）；**幂等 key = SHA256(source_event_id:contact_wxid:content_hash)，不含 run_id**；worker atomic claim + locked_until lease 崩溃恢复；发送前二次安全门 4 类取消（cooldown/stop_requested/reaction stop/30min stale）；重试 backoff (2^attempt)*5s+jitter，attempt≥3 → **failed_terminal**（统一枚举，禁用 failed）；用户 reaction stop 批量取消；admin cancel API（非 pending/in_flight 409）。 |
| N1–N7 | 实现序约束 | N1 先删 normalize_decision_runtime 兜底；N2 RawAgentDecision(Option) + validate_and_promote 双层；N3 finalize_review_for_send 单一最终安全汇总层（不被任何 approved=true 上游绕过）；N4 gateway 不直发、send_outbound_message 仅 dispatcher 可调；N5 MemoryCard 整层替换是边界工程；N6 基础设施波独立；N7 check-baseline 脚本。 |

**Design 要点**：W0–W6 七波实施表（W0 infra → W1 协议骨架 → W2 校验+安全门 → W3 工具+字典 → W4 outbox → W5 memory 替换 → W6 监控收口）；模块改造清单（约 20 文件）；GatewayStatusFinal 枚举严格映射需求状态表；关键设计决策 §11：raw/typed 双层结构理由、幂等 key 不含 run_id 的推理（revision 改内容→新 key 合理）、finalize 单点安全门（P1/P3/P4 PBT 成立前提）、atomic claim+lease 优于分布式锁、memory 整层替换防两套表示、decision_phase 由 Agent 显式声明而非 Rust 推断、taxonomy 不阻塞 vs operation_state 严格阻塞的语义分层。

**Tasks 完成状态**：72 个 checkbox 全部 `[~]`（历史标记）。**真实状态以 manifest 为准**：2.1–2.6（envelope）、3.1–3.7（finalize/二态）、4.1/4.2/4.6–4.9（knowledge_tools/taxonomy）、5.1–5.6（outbox）、6.1–6.7（memory）、7.1/7.2/7.7（outcomes/前端/lint）= `production_wired`；4.4/4.10/4.12–4.15、5.7、6.8–6.10 = `implemented`（有测试无生产接线断言）；**4.3/4.5 = `sunset_not_shipped`（"reply_with_tools_loop 生产路径从未接线、后被移除；存在的是另一个 knowledge-chat tool loop"）**；1.x 波父任务、3.8/4.16/5.9/6.11/7.9 检查点、4.11、5.8、7.3–7.6/7.8 = `partial`（历史验收声明未在冻结版本重跑 / 声明的 P6 性质不存在 / 端到端声明未绑定阻断 artifact）。

### 2.2 user-ops-agent-hardening（鲁棒性强化）

**Sunset/历史注记**：三件套顶部均有 2026-05-25 Sunset Notice（`enforce_string_fact_risk_guard / safe_claims / forbidden_claims / routing_card / fact_risk_block / pressure_risk_block / product_accuracy_block_below` 章节已下线，收敛 3 闸，销售域字段下沉 domain_attributes）；tasks.md 另挂 SR-179 权威注记。

**Requirements 全表（20 项）**：

| R | 代号 | 要点 |
|---|---|---|
| 1 | HP-1 | worker running 超时回收：claimed_at + TASK_CLAIM_TIMEOUT_SECONDS（默认 300）；回收不增 attempt_count；24h 内回收 ≥3 → failed（claim_recovery_exhausted）。 |
| 2 | HP-2 | last_message_at 拆 last_inbound_at/last_outbound_at（last_message_at=max 兼容）；follow-up context_changed 改比 last_inbound_at；一次性回填迁移（migrations 集合记版本）。 |
| 3 | HP-3 | record_user_reaction claim 锁（outcome_status pending→analyzing 原子 CAS）；60s 卡死回收；N=10 并发至多 1 次 LLM。 |
| 4 | HP-4 | LLM 指数退避 + jitter + Retry-After 取 max；**AppError::Json 不可重试**（fail-fast 降级）；llm_call_logs 加 retry_count/final_status；retry_base_ms=1000。 |
| 5 | MP-5 | 单 run RunBudget：runTokenBudget 默认 30000 / runMaxLlmCalls 默认 6 / simulationTokenBudget 默认 60000；超额降级链（review→local、rewrite 跳过、router 二次跳过）+ degraded_reasons + run_budget_exceeded 事件；后台改配置即时生效。 |
| 6 | MP-6 | Rust 字符串级 fact-risk 兜底 guard：标记词（保证/一定能/绝对/百分比/金额正则/案例/成功率/见效/回款）+ 白名单前置短语豁免（窗口 8 字符）；命中且无知识引用 → fact_risk=max(,6)、product_accuracy=min(,6)、risks 加 string_guard: 前缀；列表存 prompt_templates key=user.review.product_claim_markers。 |
| 7 | MP-7 | 状态机 allowedFrom/allowFromAny（cooldown allowFromAny=true）；check_state_transition 非法 → fact_risk≥6 + approved=false + `state_transition_invalid: from=<a> to=<b>`；contact 缺 state 时仅 new_contact 合法；PBT 全组合。 |
| 8 | MP-8 | memoryCard activeFacts → coreFacts(cap6, importance 倒序)+recentFacts(cap10, recency)；合并语义（未 discarded 的 coreFact 保留）；一次性迁移；PBT：`最终 coreFacts ⊇ S`（该性质与 cap6 数学上不可同时满足——SR-182 记录，见 §5-Q8）。 |
| 9 | MP-9 | 知识未验证冷启动告警（total>0∧verified=0，日去重）+ POST /auto-verify 批量校验（threshold 默认 7、抽样 0.1、串行、受预算约束）。 |
| 10 | MP-10 | operation_state_confidence < 阈值（默认 4）强制 full review；缺失视为 10。 |
| 11 | LP-11 | routes.rs / agent.rs mega 文件拆分（机械重构、API 形状不变、一文件一 commit）。 |
| 12 | LP-12 | RuntimeParameters/MemoryCard 等强类型化（camelCase wire 不变、serde default 兼容缺字段、渐进迁移）。 |
| 13 | LP-13 | 补 4 组索引（accounts.app_id sparse、tasks 复合、decision_reviews partial（outcome_status in pending/analyzing）、events 复合）；索引创建集中 ensure_indexes。 |
| 14 | LP-14 | webhook per-account 令牌桶限流（60s/30，超限恒 429 + Retry-After）；in-memory 单实例（多实例已知限制）。 |
| 15 | LP-15 | LLM_EXACT_CACHE → LRU 256（去掉整体 clear）；仅 4 个 prompt key 适用不扩运行时。 |
| 16 | LP-16 | mockall + testcontainers 测试基础设施；happy path 集成测试；≥3 条 PBT（状态机/记忆/claim 幂等）；HP1–4 各配回归；cargo test ≤90s。 |
| 17 | LP-17 | group/moment 种子改 draft（不再 published/active）；ensure_prompt_pack_v2 把 draft 视为已存在；UI 灰色草稿徽章。 |
| 18 | S-18 | 公式遵守度评测脚手架（evaluation_scenarios 集合 + formula-adherence 接口 + 内置 example_high_intent_user；空集合降级 200+degraded）。 |
| 19 | S-19 | 长 horizon outcome 指标（outcome_aggregation task kind；agent_outcome_metrics 集合 TTL 90d；_id="{account}:{horizon}:{date}" 幂等）。注：R19.2 指标名含 `human_handoff_success_rate`——这是**早于"无人工接管"定位收紧的历史遗留措辞**（后续 lint 禁词——文档内部自相龃龉，历史档案未回写）。 |
| 20 | S-20 | Management dry-run（session.dry_run + 请求级覆盖；非 read 工具返回 would_execute；read 工具豁免清单；dry-run 隔离 PBT：业务集合 byte-equal）。 |

**Design 要点**：现有链路 mermaid + 改造节点图；目录树拆分目标（routes/ 18 子模块、agent/ 11 子模块）；migrations 框架（Migration{id,run} + MIGRATIONS 数组，设计中的 id 形如 `2026_05_001_split_last_message_at`，对应现实代码 m001_split_last_message_at——命名演进）；错误处理表（RateLimited→429、BudgetExceeded 内部不暴露 webhook）；测试金字塔与 tests/ 文件布局（happy_path_run/worker_reclaim/last_inbound_split/reaction_claim_lock/llm_retry_jitter/string_fact_risk_guard/state_transition_pbt/memory_card_invariants/dry_run_isolation）；5 批部署顺序 + 回滚（revert commit + 删 migrations 版本记录）；6 条 Correctness Properties（状态机 iff、memoryCard 不变量、claim 幂等、Budget 单调、dry-run byte-equal、webhook 去重×限流互斥）。

**Tasks 完成状态**：24 任务全 `[~]`。manifest：1/2/3/5–20 = `production_wired`；4/21/22/23 = `implemented`；**24 = `partial`**（注："若干测试现在确实驱动生产 helper，但原清单还含复制生产 update 语句的测试，且 integration job 是 soft（continue-on-error）"——与 SR-179 对 worker_reclaim/dry_run_isolation/reaction_claim_lock/last_inbound_split "不调用生产入口"的批评相呼应）。

### 2.3 agent-self-evolution（M4 自我演化）

**Sunset/历史注记**：requirements/design **无** sunset notice（相对较新）；tasks.md 顶部 4 条注记——① SR-179 权威注记；② **2026-05-25 Note**：knowledge-cleanup 把运行时收敛为 3 闸后，演化器仍按旧 gate_key 字符串工作（`fact_risk_block / pressure_risk_block / human_like_score_rewrite / emotional_value_rewrite / product_accuracy_score_block`），这些 key 是 threshold_overrides 的**稳定持久化标识符，与运行时 guard 解耦**；切到 hallucination_score/knowledge_grounding_score 维度留下一轮；③ **Historical Done Notice（2026-05-27，已被 SR-179 manifest 取代）**：W0–W4+收口曾被旧流程标全部落地，列出当前生效模块（src/evolution/13 个文件、routes/evolution.rs、EvolutionCenterTab.tsx、check-evolution-isolation、m009）；④ **Phase A 弃用项（2026-05-26 commit 031d442）**：原 5.9 的 4 个 testcontainer 集成测试（evolution_{isolation,prompt_e2e,rollback,threshold_e2e}.rs）+ 4.8 的 evolution_significance_pbt.rs 在 sales-domain 收敛中删除，恢复须基于 3 闸+wiki 知识库重写 fixture。

**Requirements 全表（R1–R10）**：

| R | 要点 |
|---|---|
| R1 | Evolutionary worker 独立 tokio loop（EVOLUTION_TICK_SECONDS 默认 21600）；tick 全程 try/catch，失败写 evolution_tick_failed 不影响主进程；experiment 信封 insert 一次 + update 推进；EvolutionBudget（60000 token/30 calls）超限降级（先停 prompt eval 再停 threshold）；EVOLUTION_ENABLED=false 跳过 spawn 但已发布 overrides 继续生效；**R1.6 演化器 SHALL NOT 调 gateway/outbox/MCP**。 |
| R2 | 只读 7 张既有 collection；threshold cohort=72h 窗口 completed run（排除 legacy_mode_unchecked/blocked_by_required_field/tool_loop_timeout/mcp_error）；prompt cohort=failure only（revision_failed/blocked_unverified_product_claim/held_by_ai_policy/blocked_by_safety_guard/ai_waiting_for_more_context）；样本 < EVOLUTION_MIN_REPLAYS(30) 跳过；按 contact 去重 cap3；不跨 workspace/account。 |
| R3 | 阈值候选纯统计（0 LLM）：6 gate 命中率 vs 合理区间常量表（FactRisk 0.05–0.20 ±1 … PlannerBlockRate 0.10–0.40 ±0.05）；单 tick ≤4 条；同 gate 24h release cooldown；proposed_value clamp 硬界（5 闸 [1,10]）。 |
| R4 | Critic LLM 走 generate_agent_json（prompt key=evolution_critic_v1，ensure_evolution_prompt_pack seed）；输出严格 JSON diffs[]（promptTemplateKey/section/diffSnippet ≤4000 等），schema 违规整批 drop；单 tick ≤4 条 prompt proposal；Critic 失败跳过本轮 prompt 阶段不重试。 |
| R5 | Shadow replay：并发 ≤EVOLUTION_REPLAY_CONCURRENCY(4)；模拟 gateway 不调 revision/不写 outbox/不调 MCP/不写 agent_run_logs；显著性：threshold send_success_delta ≥ +0.05；prompt self_critique_addressed_delta ≥ +0.10 且任一 5gate 命中率增幅 ≤ +0.10；replay 失败率 > 0.30 直接 reject。 |
| R6 | threshold_overrides 集合 + 读序 `threshold_overrides → runtime_parameters → 代码默认`（集中在 runtime.rs::resolve_thresholds）；prompt release = 新版本 + current_version 切换 + prompt_pack_version 原子 +1（LRU 失效）；rollback 单方向（指针回退 + rolled_back_at，历史不删）；run 中途不重读阈值。 |
| R7 | 前端 EvolutionCenterTab（与 AutonomyOutcomesTab 同级）；4 条 API（GET experiments/GET proposal/POST release/POST rollback）；发布 modal 需输入 "RELEASE" 确认串；按钮按 status 启用；文案禁"接管/takeover"。 |
| R8 | 13 个新 agent_events kind；check-no-human-takeover 扫描目录加 src/evolution/；**R8.6 基线：cargo test --lib ≥ 既有 313 + 新增、4 PBT 累计 ≥ 37**（时代数字，见 §5-Q3）；所有演化表写路径集中 src/evolution/ helper。 |
| R9 | 安全边界：EVOLUTION_ENABLED 默认 false（.env.example）；发布复用 admin auth；Critic prompt 是常量不进演化循环（自我引用悖论红线）；proposal 写入前运行时禁词 lint；不主动推送 admin；**R9.6 本期不引入自动发布开关（放宽须独立 M5+ spec）**；R9.7 release 后 24h 自动对比窗口评测（不自动回滚）。 |
| R10 | 明确不做：不调"调阈值的阈值"、不跨 workspace、Critic 不写代码/schema/MCP、不给 run_logs/outcome_metrics 加字段、不做 embedding 相似度、不加路由库、无自动发布、演化器不处理实时消息、不推送通知、不扩群/朋友圈。 |

**Design 要点**：`src/evolution/` 13 模块清单（mod/budget/cohort/threshold/prompt_critic/replay/significance/release/post_release/lint 等）+ 顶部 FORBIDDEN 注释锚；4 张表 schema（Experiment/Proposal/ShadowReplay/ThresholdOverride）与索引；prompt_templates 多版本化（(key,version) unique + current_version 指针 + m4 一次性迁移 v_legacy）；prompt_pack_version=AtomicU64 复合 cache key；release/rollback 用 Mongo session transaction；风险矩阵（Critic 教 agent 绕 5 闸→显著性上限+禁词 lint+admin 确认三重防）；回滚路径（关停不回退已发布/单条回滚/rollback_all 输入 ROLLBACK_ALL）；测试策略 ≥18 单测 + 4 个 testcontainer E2E + significance PBT（**后两者已按 Phase A 注记删除**）。

**Tasks 完成状态**：全 `[~]`。manifest：1.1–1.5、2.1–2.5、3.1–3.5、4.1–4.6、5.1–5.8 = `production_wired`；1.6/2.6/3.6/4.7/5.10 = `implemented`；**4.8/5.9 = `sunset_not_shipped`（significance PBT 与 4 个 E2E 已删除、后续测试不等价原验收）**；1..6 波父任务、6.5/6.6 = `partial`（历史 baseline/手工烟测声明未绑定冻结 commit）。6.1–6.4（文档/lint 收口）= `implemented`。

**与生产现状的关键对照**（文档声称，未对码）：architecture.md 声称当前 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 强制人工发布且 evolution 是 env+Mongo 双闸；但 findings SR-180 记录生产存在 `auto_release_eligible_thresholds`（双闸默认关的阈值自动放量），与本 spec R9.6"本期禁止自动发布"是**未回写 spec 的契约冲突**；agent-policy.md 自我演化章补充了 spec 中没有的"安全闸放松约束 #152"（放松 block 类闸须过零容忍安全回归门 EVOLUTION_MAX_SAFETY_REGRESSION_RATE=0.0）与 evolution_runtime_flags（Phase C 把 env 切到 Mongo flag + 哈希分桶灰度）。

### 2.4 knowledge-digest-workstation（知识库日报工作站）

**Sunset/历史注记**：design.md 顶部 **⚠️ Note (2026-05-25)**：文中示例 JSON 的 `blockReason="fact_risk"` / `gateKey="fact_risk_block"` 是历史稿写法；运行时已收敛 3 闸；新实现应读 `held_by_ai_policy / blocked_by_safety_guard` 等运行时实际状态。requirements/tasks 无 sunset notice。**该 spec 不在 task-status-manifest 覆盖范围内**（manifest 只管 autonomy/evolution/hardening）。

**Requirements（R1–R9 + Out of scope）**：R1 节奏（每日 09:00 KnowledgeDigestWorker 扫 4 数据源 → knowledge_daily_reports；无事件驱动 push；失败落 status=failed 不空白；tick 预算 token≤24000/calls≤8，超额 partial）。R2 画布=紧凑卡片列表（卡片 schema：kind 7 枚举 / title≤60 / summary≤200 / targetRefs / suggestedAction / severity / metric；勾选批量派工注入 chat；卡片排序 critical > chunk_caused_block > chunk_missing_field > 其他；画布不直接编辑 chunk）。R3 chat 升级为常驻 30% 面板（per-turn ≤16000 token/≤6 call；LlmUnavailableError 统一路径；chat 产草稿强制 draft+needs_review——AI 永不 verify）。R4 long-running task（≥3 cards 或预估 >6 LLM call 落 knowledge_chat_tasks；worker 同 sessionId 串行；进度 turn kind=task_progress/task_summary；fail-soft）。R5 operator memory（独立 collection，**禁止**复用 contacts.memory_card / agent soul memory；注入 ≤5 条；写入必显式确认 turn 附 memoryId）。R6 4 个 tool（audit_completeness/search_chunks/propose_repair/analyze_logs；per-turn tool ≤6）。R7 红线（baseline 78/33 不跌——旧数字；no-human-takeover 0 命中；设计 token 强制；零新依赖；AI 永不 verify；不碰 prompt_templates/threshold_overrides；LlmUnavailable 唯一错误样式）。R8 数据模型（3 新 collection + knowledge_chat_turns 扩展）。R9 8 条路由。

**Design 要点**：11 文件改动 + 5 项配置（KNOWLEDGE_DIGEST_ENABLED 默认 false 等）；完整数据流闭环图（cron → 4 analyzer → compose_cards LLM → 校验/外键/排序/截断 50 → upsert 报告 → 运营勾选派工 → intent=digest_action → plannedSteps → task worker 串行 → SSE 进度 → 二次审核 verify）；关键不变量（AI 永不写 verified、三层 RunBudget 卡死、fail-soft）；PromptSpec 三条；失败模式表 14 行；文案防御表。

**Tasks 完成状态**：Phase1–5 顺序编号清单（非 checkbox），每 phase 一个 commit + 验收命令；全局验收 10 条（含拔网线故障演练与 KNOWLEDGE_DIGEST_ENABLED=false 回归）。文档本身不含完成标记；**该 spec 的实际落地证据在 docs 侧**：architecture.md/data-and-api.md/agent-policy.md 已把 digest worker/routes/collections 写为现状（文档声称，未对码）；system-review data-model 尾部记录 digest 相关已修/开放项（SR-119 已部署、SR-120/121/122/123/125 开放）。

### 2.5 universal-test-coverage（通用化后测试体系重建）

**结构特殊**：无 design.md；requirements（139 行）+ tasks（111 行）+ findings md + 4 个 JSON/mjs 数据文件。无 sunset notice；2026-06-16 建立，是 5 个 spec 中最新的。

**Requirements**：背景=通用化（DomainProfile 适配任意行业）后测试停在改造前（t4–t18 全销售域、judge 标尺写死、非销售域零真模型覆盖、大量 skip/eprintln 假绿）。北极星（源自 real-task-runbook）："每个被测能力，在每个目标域下，都能用真实 LLM 跑完真实业务闭环……**即使 agent 业务行为全错（转真人/报假价/丢温度/画像不更新）也必须能让测试变红**"。5 核心原则（真实 LLM 不接受假绿 / 业务行为对齐非链路形状 / 反过拟合契约级断言 / agent-first 配置驱动 / DEFAULT 等价单测是资产不动）。
- **R0** 堵假绿总开关：R0.1 CI 缺 key 即 job fail；R0.2 transient-skip 落 ledger + 单 job skip 率 ≤30% 硬门；R0.3 judge/reviewer http_4xx（401/402 除外）不进 transient-skip 直接 fail。
- **R1** judge profile 化（标尺从 DomainProfile business_formulas+coverage_dimensions 派生；judge 失败语义分级）。
- **R2** LLM 随机身份生成器 + 全链长程闭环断言（画像/记忆**条件式**断言——当且仅当 consolidation_needed / 足够置信新事实才更新，含反向断言防过度画像；承诺→跟进任务；状态机合法迁移；Planner 主动触达；冷启动复活；全程 AI 内部状态名；情绪温度每轮硬门；不暴露 AI 固定红线；casual 模式不推产品）+ 跨域行为实质差异。
- **R2.5** 主动半场 + 治理红线（t4–t18 被动框架装不下的维度）：R2.5.1 quiet hours 真模型业务流；R2.5.2 Planner 主动触达全链；**R2.5.3 幕后请示通道（principal decision channel）——"无人工接管"定位命门**（超职权走 escalation、AI 转述、relay 不泄露真人、过禁词 lint）；R2.5.4 并发/多账号（可选低优先）。
- **R3** 深命门：H11 自学习极性跨域（正/负/沉默=Hit/Block/Censored）；C2 operation_state 派生跨域（fail-soft + transition_rejected 审计）。
- **R4** 知识库域适配（召回基准补 recall@k 下限等真断言）。
- **R5** LLM 驱动动态博弈（**定位修正后=两条线**：动态发现 nightly/手动不进 PR 门 + 固定回归 PR 门守下限，t15–18 不退役）：R5.0 反过拟合四铁律（不固化对话当标准答案/修抽象层+多变体验证泛化/评测器人工金标锚定/守过拟合责任判断）+ 五道机械门（control set 阴性对照/变体 pre-registration/先证伪根因/held-out 对抗集/diff 机检）；R5.0.1 三角色异族硬门（roleplayer/agent/judge 三个不同 provider 家族，同源→job 红；roleplayer 第三族 key 当时不存在）；R5.1 Roleplayer（transcript 回放复现，非 seed——llm.rs 无 seed 通道）；R5.2 Trajectory Judge（人工金标 ≥30/桶 + IAA α≥0.6 + train/dev/test split，校准达标前只进 ledger）；R5.3 动态对抗；R5.4 跨会话长期弧。
- 归因纪律：8 层 suspected_layer；每阶段只引入一个新变量。

**Tasks 完成状态**：**全部未勾选 `[ ]`**（R0–R5 + 附录 P1/P2 收尾）。R0.1 揭露 CI 17 处 `${{ secrets.X || 'nvapi-...' }}` 明文字面量回落（=泄露，须轮换 NVIDIA key）——与 system-review HC-001 事件记录的"Workflow 明文字面量 fallback 根因"相互印证（后者称该根因已修 + CI 硬门 workflow-secret-must-be-direct 已接线）。

**findings（2026-06-18）**：18 个真模型测试文件分类清单；agent 短板：C1 转真人首轮哑火=**已证伪**（跨 run 不复现，反过拟合纪律拦下误报）；**C2 helpfulness 系统性偏低（5–6 分，"守得住底线给不出抓手"）=可信度较高**（跨 3 弧 3 裁判 + 跨 run 复现）；D1 过期 FollowUp 撞 quiet hours 被续命=真缺陷**已修**（expired 判定移到 quiet_hours 之前）；D3 合同条款抽取 5.0<6（同家族裁判待复核）；E1–E3 正向能力（抗翻供/不乱报价/情绪按需承接——"C2 优化时勿破坏"）；J1–J6 评判输入失真（judge 判 grounding 却看不到知识库、consistency 缺 memory 锚点、roleplayer 零校准、无对话级总评、情绪单句判、轨迹裁判未校准悬空）；T1–T4 探针缺陷（judge 单采样离群、"在不在"裸 contains 误伤、t17 转人工红线 contains_unnegated 后半句否定误伤——"agent 做对反而红"最优先修、q6 空响应待复核）。

**audit-status-manifest.json**：47 域 deepread 审计（2026-06-30）全部改判 `inconclusive`；legacy 结果只作 `research_leads`；limitations：no_frozen_run_manifest_or_model_identifier / no_per_claim_path_locator_evidence / workflow_silently_dropped_failed_domains / D4_falsify_missing_required_arrays——与 findings SR-183 一致。

---

## 3. docs 顶层逐篇深读

> 每篇：核心主张 / 关键数字与规则 / 与代码可能不符或存疑处（对照点：5 闸已改分数闸、user-ops tool-loop 退役、路由 235、迁移 m058、baseline 350/33 等）。

### 3.1 README.md（48 行）
- 主张：文档阅读顺序（product-modules → architecture → ai-agent-system → agent-policy → data-and-api → roadmap → frontend-design-system）；文档维护规则（新增模块/Agent/后端能力/自动化行为/前端频道前先更新对应文档）。
- 存疑：`当前阶段`能力清单停在早期（无 evolution/digest/wiki/planner）；smoke、system-review、mcp-*、universal-* 等大量文档不在阅读顺序中。历史上 README 曾写 78 基线与 CLAUDE 350 冲突（SR-006），HC-033 记录已修复对齐（**指根 README.md**；docs/README.md 本身不含基线数字）。

### 3.2 product-modules.md（250 行）
- 主张：8 一级模块（工作台/用户运营/群运营/朋友圈/内容资产/系统策略/AI Command Center/任务与日志/账号与系统——列表实为 9 项但标题写 8 类结构）；运营知识库节描述三件套（documents/items/chunks）+ 4 个 Agent 内部工具（list_catalog/search/open_slice/open_evidence）；群/朋友圈默认禁自动动作。
- 存疑："Agent 内部工具"描述对应旧知识路由；当前生产为 single-pass route_operation_knowledge + knowledge_agent（sunset-plan D+21 注记），工具面是否仍与此四个名称一致**未对码**。knowledge items 实体在 system-review SR-112 中被称"已删除的 items 实体"（准入仍依赖它）——本文档"主题知识包 operation_knowledge_items"仍作为现状描述，存疑。

### 3.3 architecture.md（444 行）
- 主张：顶部快照声明（2026-07-24 commit d60d3d8 + 未提交 SR-001–183 收口工作树；"源码优先于本文档"）；当前架构=React → Axum（同进程 supervised workers）→ Mongo/LLM/durable Outbox→二次安全门→MCP；webhook 流程（落库+pending handoff marker ACK 前持久化 → durable inbound_reply AgentTask → claim token+generation+lease → gateway → outbox → post-hoc delivery verification）；evolution worker 流程图（env+Mongo 双闸、`CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 强制人工发布）；digest 流程图；knowledge-wiki 子系统（apply_chunk_revision 三层保护 + 双 worker + 探索注入默认关）；**14 条 supervised worker 清单**；Phase 0→E5-T1 时代图 + 新增 collection 速查 + 模块隔离红线。
- 关键数字：worker 14 条（task/import/outbox/media_reconciler/planner/cold_contact/silence/evolution/digest/knowledge_task/catalog_rebuild/knowledge_feedback/ingest/management_command_sweeper）；supervisor 熔断（60s 内 5 次 panic → open）。
- 存疑：worker 数在不同文档/时点为 12（reading-notes@PR#223）/13（HC-033 修订说明"按 src/main.rs 的 13 个 spawn_supervised 调用点对账"）/14（本文现文）——随时间演进，当前真实数**未对码**。"webhook→durable inbound_reply AgentTask"描述的是 SR-177 修复后形态（文档声称，未对码——SR-177 记录的是修复前的进程内 PENDING 缺口，本文已按修复后写）。

### 3.4 ai-agent-system.md（267 行）
- 主张：Management Agent 与 Operations Agents 分离（prompt/权限/日志/成功指标都分开）；Soul Prompt 稳定人格原则；Prompt 分层（System Contract/Agent Soul/Policy/Business/Operator Instruction）；Prompt Stack v2（agent_souls/prompt_templates/operation_playbooks；默认包 wechatagent_prompt_pack_v2_2026_05；reset-system-pack 物理删除重建、非每次启动覆盖）；工具风险五级（Read/Draft/Configure/Act/Dangerous）；Command Center UI 与 API 方向。
- 存疑：Data Model Direction / API Direction 是"建议新增"式（其中多数已落地，如 management_agent_sessions 等——文档措辞仍是"后续建议"）；prompt 的多版本形态（M4 引入 (key,version)+current_version）未在本篇反映。

### 3.5 agent-policy.md（749 行）—— 与代码最同步
- 顶部"当前实现说明"：历史演进曾把销售域 5 闸收敛为 3 类硬约束，但**旧 enforce_* 函数已经移除**；在线 Review 入口=`review::review_passed`、`review::classify_dual_gate / route_dual_gate`、`review::finalize_review_for_send`；§自我演化中的旧 gate_key 仍是持久化协议字符串（不代表存在同名运行时函数）；Contact/Chunk 业务字段位于 domain_attributes。
- 自动发送约束现状：`HallucinationScore ≥ block 阈值`（review_passed/finalize 阻断）、`KnowledgeGroundingScore < 阈值`（产品声明时 Review+verified-claim 兜底阻断）、RunBudget 超限终止；**HumanLike/EmotionalValue/PressureRisk 是软闸（route_dual_gate）→ single-shot revision，二次仍败落闭集阻断态**。2026-06-14 修订：R5.4 reviewer 自报路径的 blocked_unverified_product_claim 强约束不变；finalize 漏判探针（ProductEffect 分支）改**仅观测**（grounding_probe_reviewer_missed 事件）。**阈值别名注记：UserRuntimeParameters.fact_risk_block_at 实际承载 hallucination block 阈值；product_accuracy_block_below 实际承载 knowledge_grounding 阈值**——本次亲验 `src/agent/runtime.rs`（"factRiskBlockAt"→"hallucinationBlockAt" 映射、fact_risk_block_at: typed.hallucination_block_at）与 `src/agent/review/gates.rs`（human_like_rewrite_below/emotional_value_rewrite_below/pressure_risk_block_at 判分）**证实此描述**，即"5 闸已改分数闸"的准确形态。
- quiet hours（#69）：默认开启；quietHoursStart/End 默认 22/8；时区纯整数运算不依赖宿主（src/agent/quiet_hours.rs）；入站排 deferred_inbound_reply + 去重 wake；主动发送重排不取消。max_daily_touches 仅约束 AI 主动触达（FollowUp-only），被动回复不受限。
- 辅助模式/名片引荐：全自治默认红线不动；账号级 assist_mode_enabled + 客户级 override；名片必须 admin approved+enabled（AI 永不自我核验）；已引荐态转被动答疑；台前顾问 ≠ 幕后决策源；outbox 幂等键含 referral_card_id、MCP message_send_namecard。
- 自我演化章：EVOLUTION_ENABLED 默认 false、关停不回退已发布；5 gate_key 与 THRESHOLD_REASONABLE_BANDS；**安全闸放松约束（#152）**：三个 block 类闸放松须过反向显著性门=安全回归率 ≤ EVOLUTION_MAX_SAFETY_REGRESSION_RATE（默认 0.0 零容忍）；release/rollback 用 Mongo 事务；cooldown 24h；rollback_all。
- 自学习采集管道：第一阶段（7 条铁律：观察/解释分层、沉默=删失、幂等 dedupe_key partial unique **禁 $in 否则 Error 67 panic**、S1–S7 落点、明确不做清单）；第二阶段（P1 dynamic_confidence 换血为真实用户 outcome（DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED 默认 true）、P2 双时间戳、P3 采集健康度默认关、P4 召回探索注入默认关只记 propensity 不消费）。
- digest / knowledge-wiki 两章（与对应 spec/docs 一致，见 §2.4/§3.9）。
- **Phase 0→E5-T1 changelog（明确标注"历史记录，非当前契约"）**：Phase 0 紧急修复（write_agent_run_log_with_finalize 走 envelope、check_state_transition fail-closed）；Phase A（reaction hint 注入、operator_memory、taxonomy 预热、tool_loop 仅作 chat 支持模块保留——"user-ops 入口走 decide_reply_with_promote → review，不再 user-side tool-calling"）；Phase B（human_like/pressure_risk 软闸补回、reviewer 输入遮罩 draft.reasoning、chunk_type 4 类枚举、operation_state_policies）；Phase C（reviewer_misjudge_signal、negative_example 自动入 review queue、evolution_runtime_flags Mongo 化+哈希分桶、threshold close-loop）；Phase D（intent_trajectory cap50、last_outbound_style 风格指纹、cold_contact_worker、account_scheduler 轮休、lessons_learned→peer_case admin 手工晋升）；Phase E（locale 元数据保留不分叉、LlmProvider trait、REVIEWER_DUAL_ENABLED 双模 reviewer 分歧触发 revision、ops 三表版本收敛唯一 current fail-closed、OpsDomain trait 边界声明、MCP 工具动态注册+白名单审计）。**本段确认基线=lib ≥350 / 4 PBT ≥33，且 PBT 集合已换为 state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter（string_fact_risk_guard 被 wiki_chunk_revision_pbt 替换），与 scripts/check-baseline.{sh:25,ps1:17} 同步**（与 CLAUDE.md 一致；脚本行号未对码）。
- 存疑：无明显与代码冲突处（该文档 8/5 更新，是现状叙述最可信的一篇）；但其中大量运行细节（如 #152 常量名、S1–S7 落点文件名）**未对码**。

### 3.6 data-and-api.md（707 行）
- 主张：collection 全集不再手抄（权威=src/db/mod.rs typed accessors，**"当前 61 个"**、indexes.rs、migrations/）；Contact 模型现状字段与 domain_attributes 下沉说明（旧 customer_stage 等由 DomainSchema 校验）；chunk 旧销售域字段（routing_card/safe_claims/forbidden_claims/evidence_items 等）全部下沉 domain_attributes；evolution 4 表 schema 与 /api/evolution/* 5 路由；prompt_templates 多版本形态（(prompt_key,version) 唯一 + current 至多一行 + seeded_by=evolution_release）；digest 3 集合+8 路由+6 事件 kind；knowledge-wiki 子系统全字段/集合/路由（chunk_revisions/knowledge_gap_signals/domain_schemas/catalog_rebuild_jobs；chunk 编辑 7 路由 + catalog 双轨 + gap-signals 4 路由 + domain-schemas 5 路由；import-apply 事务协议：previewId/previewHash/candidateId 封印、服务端重建全部 server-owned 字段、同事务提交、重放稳定回执）；Phase G（G1 档案馆美学 / G2 documents CRUD / G3 chunk 9 动作+referrers+batch-verify / G4 ChatWorkbench+Observability 5 卡 / G5 admin 治理三件套 + $facet 元信息聚合 / G P1 supersededBy 链+Graph view+Answer cache stats）；G 红线表（AI 永不自动 verify、0 新依赖、隔离、状态闭集 R9.10.e、不留兼容层、三 lint）。
- 关键数字：61 accessors（2026-07-24 时点）；LLM Output Contract 一节仍展示**旧的简化决策 JSON**（shouldReply/replyText/profileUpdate/memoryUpdate/followUp）——与自治协议 9 字段+RawAgentDecision 现实差距大，属早期遗留段落。
- 存疑：61 个 accessor 是快照值（现在可能更多，**未对码**）；"未来建议" API 段落与已实现路由混排；`/api/evolution/proposals GET` 列表路由与 evolution spec 的 4 路由 + rollback_all 有小差异（本文含 rollback_all，spec design 的 evolution_routes 骨架无、由 tasks 5.5 补）——以代码为准（未对码）。

### 3.7 development-roadmap.md（154 行）
- 主张：Phase 1 用户运营闭环（已完成清单）→ Phase 2 内容资产+策略 → Phase 3 Command Center → Phase 4 朋友圈 → Phase 5 群 → Phase 6 多账号矩阵；近期工程任务 8 条。
- 存疑：**整篇为早期路线图**；未反映 autonomy-loop/evolution/digest/wiki/planner/universal-domain 等全部后续主线；"近期任务"多数已完成或被更大的 spec 取代。作为"路线图"已失真，作为历史意图记录有效。

### 3.8 frontend-design-system.md（194 行）
- 主张：白色企业控制台视觉论纲；CSS token（--sidebar-width:264px 等）；频道=一级产品区（不做锚点长页）；四级层级；色板（--accent:#2563eb / --ai:#0f766e）；排版字号表；组件规则（卡片/面板/列表/表单/表格）；<860px 响应式；扩展 checklist。
- 存疑："Current channel model"仅列 5 频道（AI Command Center/Workbench/User Operations/Agent Profile/Tasks & Logs），而 smoke 四方对账已核对 13 组频道（含账号管理、Autonomy、Evolution、质量、发送成效、收件箱等）——频道清单显著过时；中文标签列表（AI 总控/工作台/…）也仅 8 项。设计 token 与规则本身仍被后续文档（digest R7.3）引用为有效约束。

### 3.9 knowledge-wiki.md（202 行）
- 主张：为什么不是销售话术 RAG（9 类 wiki_type 稳定 + domain_attributes 下沉，**2026-05-25 收敛注记**：Contact 与 Chunk 主表销售域字段已全量下沉、换行业=切 schema）；LLW 借鉴对照表 10 项；9 类 wiki_type 决策表（销售/教培/医疗三域示例 + 经验法则）；provenance 矩阵（imported/ai/human/rule/principal_authorized 五 source；**source=ai 强制 draft+needs_review、AI 永不自动 verify；auto-verify 最多到 needs_human_audit**）；lifecycle 状态机（含 rollback 不删历史）；三层写入保护（锁定字段 / 数组 union / body/summary/answer 70% 阈值）；patch-only 协议；feedback_worker（600s、每 workspace 300s lease+60s heartbeat；dynamic_confidence=base×0.6+hit_rate×0.4−penalties，最小样本门；9 类 structural lint 规则表；stage2 LLM 裁决留接口不进热路径）；domain_schemas 校验红线（fields≤64 等）与切换语义（切 active 不重校验既有 chunk）；catalog 双轨（persisted O(1) / live O(N×M)；写后 <3s 反映）；删除级联（normalize_ref_key 防子串误伤）；自检清单（查得到/改得了/优化得了）；不在范围清单 + **D3 已落地注记（2026-06-17）**：superseded_by redirect（防环+8 跳上限、新版须 verified）+ 关系图谱按 relation_kind 分流（contradicts 跟随但标警示不作支撑引用）。
- 存疑：hit/block 标签描述与 agent-policy 第二阶段（真实用户 outcome join）一致；细节（resolve_superseded 函数名、8 跳上限）**未对码**。

### 3.10 ci-known-gaps.md（17 行）
- 主张：G1（已亲验 2026-07-01）：`changes` job paths-filter 的 backend 过滤器不含一般 frontend/src/** → 纯前端 PR 跳过 baseline job → check-no-human-takeover 不跑 → 前端新增禁词不被拦；push 事件全量跑故 main 不失守；修法三选一（独立 job / 纳入 filter / frontend job 双挂）属 CI 拓扑决策待用户拍板。
- 存疑：现状是否已修**未对码**（文档态=待决策）。

### 3.11 sunset-plan.md（153 行）
- 主张：D/D+7/D+14/D+21 时间表 + 回滚原则（7 指标显著退化即回锚点）；5 组开关行为对照（autonomyProtocolEnabled、knowledgeRoutingMode、MemoryFactRepr::Plain、OutboxStatus failed→failed_terminal 兼容、finalReviewStatus held_for_human 拒收）；一次性迁移脚本清单（2026_05_001..005，与代码 m001..m005 对应）；**"双轨长期维护"PR review checklist 9 条**（新增灰度开关 / Plain 分支复活 / 老枚举复活 / 绕过统一网关 / 绕过 assert_final_review_status_valid / 复活 Vec<String> coreFacts 输入 / 绕过 RunBudget / 删 PBT 用例 / 前端禁词）。
- **D+21 已兑现注记（2026-06-23）**：knowledgeRoutingMode / knowledgeMaxToolLoops 字段及 reply_with_tools_loop（src/agent/tool_loop.rs）、dispatch_tool_call（user-ops 半边）**已删除**；生产统一走 single-pass route_operation_knowledge + knowledge_agent（gateway.rs）；兼容=serde 静默忽略旧字段（回归测试 runtime_parameters_typed_ignores_dropped_legacy_routing_fields 钉住）；原计划的 legacy_runtime_parameter_dropped 启动日志未实装（serde 忽略已足够）。
- 存疑：**本文 §2 的开关枚举值写 `"tool_calling"/"prompt_inline"`，与 autonomy spec R11.3 的 `auto_tool_loop / classic_router` 不一致**（同一开关两套名称，见 §5-M2）；该开关已删除故仅史料意义。

### 3.12 real-task-runbook.md（1562 行）
- 主张（历史档案，sunset notice 声明不再随主线更新）：真实流量压测执行底稿。头部**状态枚举速查**：final_review_status 10 项（与 run_envelope.rs:67-78 对齐）、gateway pre-block 8 项、GATEWAY_STATUS_VALUES 24 项（run_envelope.rs:86-113）、local_decision_review 是 review_mode 模式名非终态、禁词 regex 全文。北极星 4 问（自运营/自优化/自治理/全量覆盖）+ 0.1 打分锚点 + 0.2 停止决策树。环境事实表（MCP=47.108.57.147:3001、DeepSeek deepseek-v4-flash、RunBudget 30000/6/6）。§4.0 全量能力覆盖矩阵（Gateway 9 类 pre-block / 5 闸 / Review / RunBudget / Outbox / Knowledge / 状态机 / 双层标签 / Memory / Reaction / Commitment / Planner 7 类 / Outcomes / Evolution 8 类 / Prompt 分层 / 2 个安全 lint——每格给实现位 file:line + 触达手段 + 期望 db 证据）。S1–S15 场景。§5 七步循环 + §5.2 探针模式。§6 红线 R1–R9（含 R3：lib 跌破最近一轮 baseline、最低不低于历史下限 78）。Round 1–16 日志：R1 happy 全被 blocked_by_required_field（prompt 缺 14 必填字段）→R2 修 prompt v2、baseline 381/37 →R5 S8/S9 烟雾 PASS（发现 planner 需 DEFAULT_ACCOUNT_ID 匹配、silent 要求 last_outbound<last_inbound）→R9/R10 把 ISSUE-004/005 改判"文档误用非 src bug"（string guard 只扫 reply_text；state-guard observability 完整、LLM 自我修正致不触发）→R12 修 ISSUE-001（gateway 短路顺序致 context_changed 信号被 finalize_review_blocked 覆盖）→R13 修 ISSUE-003（R5.3.a 三 trigger 强弱分层 + inbound 无 marker 时软化）→R14 修 evolution cohort phantom 枚举（budget_exceeded→blocked_by_budget+补 blocked_by_required_field）与 planner clock-skew →R15 销售文档 8 步全链路 PASS + 修 ISSUE-006/008/009（LLM JSON 容错 / domain 白名单归一 / auto-verify 专属预算 240000/100）→**R16 重跑发现 ISSUE-012：知识有据可查红线三层守卫自然路径联动失效（Reply Agent 内联 knowledgeNeed=not_required 短路路由 + R5.7 反向门空集合永真 + review LLM 对"保证 50% 提升"factRisk=1），红线靠 LLM 自我克制兜住，架构三选一留专轮**；ISSUE-013 items 不回填 document_id。§9 ISSUE 完整档案；§10 自动补全模板。
- 存疑：ISSUE-012/013 终态**未对码**（runbook 不再更新；后续 smoke 2026-07-05 的"已核实知识解锁产品红线 grounding 4→10"与 B-1 升档链路表明知识路由/grounding 行为已演进，ISSUE-012 的三选一是否落地无文档直接回答）；baseline 数字（381→416）是当时实测总数，与后期 1814+ 或阈值 350 不同概念。

### 3.13 real-task-loop-prompt.md（132 行）
- 主张：/goal 启动 7 步（Step 0 并行读 runbook/CLAUDE/run_envelope.rs；Step 1 Round 号四分支表——footer JSON 是唯一信号，自然语言"用户收尾"一律不参与；Step 2 环境自检 6 步；…）；4 条流程硬约束（UTF-8 投递 / 只发 Jsjm / 全部经 gateway / 禁词 0 命中）；git 安全；子代理使用规则；反模式清单。
- 存疑：与 runbook 同属历史压测体系；引用的 run_envelope.rs 行号（60–180）为当时值（**未对码**）。

### 3.14 real-llm-test-authenticity-audit.md（91 行）
- 主张：三大贯穿假绿根因（require_real_llm! 缺 key eprintln+return 让 200+ 测试静默 ok；unwrap_or_skip_transient! 吞核心调用、http_4xx 被当抖动吞——405 案例；judge 失败照 pass）；运营侧/知识库侧逐测试定级表（真实性最高=smoke t2 引用接地、calibration、ops t11、q2 抽取；假绿风险=adversarial 6 弧观测-only、t_judge_calibration 零 assert、recall_benchmark 自称基准零硬断言、k6/q3 vision 空转）；修复优先级 P0（CI 显式断言 key/skip ledger 硬门/4xx panic）P1（转真人红线升硬断言+修 prev_reply 错位）P2（recall@k 下限等）；反过拟合边界。
- 存疑：修复是否全部落地**未对码**（后续 findings-2026-06-18 显示 redline 6 文件硬门与 skip-gate 已加；SR-178 又指出这批"硬门"允许零样本 pass——审计→加固→再审计的迭代链）。

### 3.15 test-paradigm-llm-driven-analysis.md（78 行）
- 主张：所有"多轮"测试客户台词 100% 写死（逐测试 file:line 证据）、博弈链断裂；业务价值恰在动态博弈的 5 个场景；三个范式天花板（turn-level judge 量不到轨迹价值 / 客户零真实刁难 / 无跨会话）；升级方向=Roleplayer+Trajectory Judge+跨会话；**第六节定位修正（覆盖第五节）**：seed 不可复现/三角色易同源/成本爆炸三个硬约束 → 两条线定位，权威归 universal-test-coverage spec。
- 存疑：无（分析文档；其结论已被 spec 吸收）。

### 3.16 universal-domain-test-gap-audit.md（79 行）
- 主张：通用化能力×测试覆盖矩阵（H11 极性=P0 最深命门、C2 状态派生=P0、t4–t18 全销售域=P0 体系级、judge rubric 写死=P0 横切；H14/H8/H19/H2 P1；其余 P2）；修复顺序；§3.5 业务闭环对齐表（每能力"断言该验什么"vs 现状差距；"空测试的特征=agent 全错测试照绿"）；反过拟合边界（DEFAULT 等价单测是资产不动）。
- 存疑：矩阵状态为 2026-06-16 时点；此后 redline 6 文件、judge rubric profile 化（build_judge_rubric 已存在，findings J 节提到）等已部分推进（**未对码**）。

### 3.17 universal-domain-base-extensibility-audit.md（147 行）
- 主张（2026-06-18，HEAD cff6e88，全部 file:line 亲核 + 证伪附录）：执行摘要——运行时引擎层真通用（状态机/五闸/极性/记忆/override 全真消费 + None 回落 + DEFAULT 字节等价），摩擦在"配置如何诞生/扩展点是否收拢"。轴 A：新行业=复用 user_operations domain 换 profile+状态机（A1' 整条私聊链路硬绑 domain="user_operations" 字面量；profile 与状态机两套 admin 入口无关联校验）。轴 B：维度扩展点散落 ≥4 处无 registry（B1 high）；objection_type 声明字典实现裸 string（B2 降级 medium）。轴 C：22 字段仅 domain_schema_id 死字段；C3 无中央接线点（high）；C4-a/C4-b 初判被证伪。轴 D：relationship_type 三级回落正确，缺 LLM 自动识别（D3 medium）。三张扩展成本标准动作表 + 修复方向 6 条 + 证伪附录 5 条。
- 存疑：B1/C3/D3 的后续状态在 robustness-roadmap 落地注记中部分更新（dimension_registry 落地、D3 保守闭环落地）；本篇矩阵为当时快照。

### 3.18 universal-domain-base-robustness-roadmap.md（120 行）
- 主张：合并两份审查 → 5 主题（扩展点收拢/数据完整性/数字分身闭环/规模多租户前置/快赢）+ 推进顺序表 + 验证基线（**lib ≥350 / 四 PBT ≥33**）。**落地进度注记**：2026-06-18 主题 1+2 核心落码（src/agent/dimension_registry.rs 单一真相源；Contact 三写入路径接 validate_dimension_value——admin reject / LLM drop+审计 / objection_type 归一；WriteIntent 正交轴；lib 1308/0、四 PBT 36/0）；2026-06-19 第二轮（prompt 话术随 profile 替换：mode_gate_policy_override + reviewer_fewshot_override；**D3 relationship_type LLM 识别落地**——LLM 产建议→不直接生效→运营 REST approve 的保守闭环；lib 1336/0）；实证修正：五闸可配=过度设计已砍、driver 框架=高风险低收益已缓。未做：C3 全量中央接线、前端审核面板、主题 4 规模。
- 存疑：planner 只扫 default_account（主题 4 "功能黑洞"）后续是否修**未对码**（system-review SR-085 记录 worker 只消费 default scope，属 HC-004 多租户修复面）。

### 3.19–3.23 mcp-* 五篇
- **mcp-deployment-guide.md**：MCP_BASE_URL=http://117.72.54.28:3001；MCP_API_KEY 双用途（Bearer + webhook HMAC-SHA256 X-MCP-Signature）；两种 webhook 配置方式；签名验证代码引用 src/webhooks.rs:295-308（行号**未对码**）；账号登录两路径；7 FAQ；安全建议（生产必开签名、key 轮换 3-6 个月）。
- **mcp-integration-audit-2026-07-07.md**：活体测试全绿（initialize/auth_whoami 显示 Workspace Key 管理 1 账号 t-1/tools/list 136 工具）；兼容 8 项核对表；关键缺口=Workspace Key 调账号类工具须注入 account_alias（修复路径 A 推荐）；次要=无状态 server 容错未兑现注释。
- **mcp-integration-completion-report.md**：两阶段完成（9c34d80 account_alias 自动注入 + 35a6125 登录流程/文档）；当前状态表（已实现 8 项/待验证 3 项/P1 4 项）。
- **mcp-integration-final-delivery.md**：P0 三项全完成（含 2966497 前端账号管理频道）；交付物 8 文档；部署 4 阶段清单；"技术基础 100% 到位、业务闭环缺最后一公里（MCP Server 侧配置 webhook URL）"。
- **remaining-issues-summary.md**：P0 3 项（配置 webhook URL/端到端/前端集成——第 3 项随后完成）；P1 4 项（账号管理增强/定时同步 worker/自动 sync/login_status 字段）；P2 3 项；历史 C 类 BLOCKED 说明。
- 存疑：五篇均为 2026-07-07 时点交付文档；MCP 地址与更早的 runbook/smoke（47.108.57.147:3001）不同——**属服务器变迁而非矛盾**；P1/P2 项后续是否完成**未对码**。

### 3.24 DEPLOYMENT-STEPS.md（483 行）
- 主张：11 步部署手册（拉代码/合并/env/编译/重启/验证/配置 webhook/端到端/检查点/解除 BLOCKED/常见问题/回滚）；env 含 AUTH_RATE_LIMIT_* 四参数（限流按直接 TCP 对端、各副本独立、反代场景须边缘层另设——不信 X-Forwarded-For）与 RUN_TOKEN_BUDGET_ESCALATED=100000；**2026-07-25 生产发布实录节**：后端 SHA-256 b0b1a0…→539eff…、m049 applied（group.policy/moment.policy 保持 draft+current_version=false）、12/12 健康、指向 system-review/production-release-2026-07-25.md。
- 存疑：步骤 9.4 查询示例用 `db.agent_runs`（集合名应为 agent_run_logs——文档笔误或旧名，**未对码**）。

### 3.25 FINAL-SUMMARY-2026-07-07.md（324 行）
- 主张：Goal 100% 完成（13 频道四方对账/A 类 0 新增/B-1 已修复验/C 类 3 项归类）；B-1 升档预算修复细节（grant_escalated_ceiling + run_token_budget_escalated 默认 100000 + gateway 两处升档分支；验证 run 完整跑 Lean 24759→Full 31034→review→rewrite→re-review，最终 blocked_unverified_product_claim 是另一条正确红线）；测试覆盖：**前端 448 测试全绿 / 后端 cargo test --lib 1814 passed**；交付 10 文档 ~2100 行；2026-07-25 补充节（生产切换完成、"本次发布不能替代仍标为待 Actions/真实模型/真实 MCP 验证的审查项"）。
- 存疑：1814 是当时 lib 实测总数（与阈值 350 是两个概念）；"业务闭环缺最后一公里"的 webhook URL 配置完成与否**未对码**（07-25 已生产发布 + 后续 production-release 系列表明生产运行中）。

### 3.26 wechatagent-project-knowledge.md（134 行）
- 主张：项目自述知识文档（定位=长期运行 AI 私域运营 Agent 而非群发工具/客服机器人；核心原则=只运营 managed、独立上下文、像真人、不编造、知识有据、可审计）；产品模块口语版；运营公式（注意其 NextBestActionScore 公式与 agent-policy 版本**不同**：本文=RelationshipGain+ConversionProgress+EmotionalValue+ProductFit−PressureRisk−FactRisk；agent-policy=RelationshipGain+UserValue+ConversionProgress+ProductFit+Timing−DisturbanceCost−HallucinationRisk−GroundingRisk）；知识事实边界（可安全表达/禁止表达清单）；典型客户问题回应方向；文末"当前测试期望"暴露其**双重身份：知识库导入测试素材**。
- 存疑：公式老版本（含 FactRisk/PressureRisk 旧词）——作为知识库素材如实反映旧时代；不应作为当前方法论对照源。

---

## 4. 事实卡速查

### 4.1 各 spec 红线条款汇总

**agent-autonomy-loop**
- 全 AI 自治：任何 gateway_status/events/文案禁 `human/人工/接管/takeover/hand-off`（R2.7→check-no-human-takeover lint）；holdCategory 严格三选一；finalReviewStatus 闭集 10 值、写库拒收（R9.10.e）。
- 不让 Agent 自由生成 operation_state（状态机字典 + check_state_transition）；双层标签候选**不阻塞** run。
- verified knowledge 唯一判定 = integrity_status=="verified"；产品声明无 verified → block。
- RunBudget 单 run 预算（超额降级不 5xx）；outbox 强幂等 key（**不含 run_id**）+ 二次安全门 + failed_terminal 统一枚举；决策 approved 必先写 outbox 再 MCP；用户拒绝/cooldown 取消 pending outbox。
- 灰度开关必须有 sunset（R11.9 新开关不入 sunset 计划=协议违规）。
- 当时基线：lib ≥78 / 4 PBT ≥33（PBT 集合含 string_fact_risk_guard）。

**user-ops-agent-hardening**
- claim 类操作原子幂等（task claim / reaction claim ≤1 成功）；JSON 解析错误不重试；BudgetExceeded 不外泄 5xx。
- 字符串 fact-risk 兜底（marker+白名单）；状态机 allowedFrom iff 性质；coreFacts 合并语义（未 discarded 保留——与 cap6 冲突见 §5-Q8）；dry-run 业务集合 byte-equal；webhook 429 恒 429。
- （注：本 spec 的 5 闸/string guard 相关红线已随 knowledge-cleanup 下线，见 sunset notice。）

**agent-self-evolution**
- src/evolution/ 物理隔离：禁止引用 gateway/outbox/mcp（check-evolution-isolation CI）；只读 7 表、写自己 4 表；shadow replay 零业务副作用（100 次后 outbox size 不变）。
- shadow eval + 显著性是 release 前置；**release 永远 admin 手动 + RELEASE 确认串（R9.6 本期禁自动发布）**；rollback 单方向；Critic prompt 不进演化循环（自我引用悖论）；proposal 文本过禁词 lint。
- EVOLUTION_ENABLED 默认 false；关停不回退已发布 overrides。
- 当时基线：lib ≥313 / 4 PBT ≥37。

**knowledge-digest-workstation**
- AI 永不自动 verify（digest/chat/task 三路径产出一律 draft+needs_review；verify 走 sourceQuote→anchor gate）。
- 不写 prompt_templates / threshold_overrides（不碰演化器）；operator memory 与 contact/agent 记忆物理隔离且写入必显式确认。
- 三层 RunBudget（worker tick 24000/8、chat per-turn 16000/6、task per-step 8000/4）；LLM 错误统一 LlmUnavailableError；节奏 1 无事件驱动 push；零新依赖。
- 当时基线引用：lib ≥78 / PBT ≥33（旧数字）。

**universal-test-coverage**
- 真实 LLM 不接受假绿（缺 key 即 fail / skip 落 ledger 设上限 / 4xx 不吞）；红线断言=命中禁忌即 fail（转真人/泄系统提示/报价格数字）不锁措辞。
- R2.5.3 幕后请示通道="无人工接管"命门（超职权走 escalation + AI 转述 + 不泄真人）。
- R5 动态线不进 PR 门；反过拟合四铁律+五机械门；三角色异族硬门；trajectory judge 校准达标前只进 ledger；DEFAULT 等价单测是资产不动。

### 4.2 关键闭集/常量速查（文档声称）

- finalReviewStatus 10 值 / GATEWAY_STATUS_VALUES 24 值 / gateway pre-block 8 值（runbook 头 + run_envelope.rs:60-180——行号未对码）。
- outbox 状态 5 值：pending/in_flight/sent/failed_terminal/canceled（禁 failed/queued）。
- lifecycle 7 值；holdCategory 3 值；autonomy_mode 3 值；risk_level 3 值（无 critical）；knowledge_need 3 值；run_mode 4 值。
- wiki_type 9 类；provenance source 5 类；gap_signals kind 10 类（9 lint + recall_miss）；chunk_revisions op 10 类。
- 当前合并基线（CLAUDE.md/agent-policy/check-baseline 声称）：lib ≥350 / 4 PBT ≥33，PBT 集合=state_transition_pbt/memory_card_invariants/**wiki_chunk_revision_pbt**/llm_retry_jitter。
- 迁移：m001–m058（**亲验 58 个文件**）；HC-004-SR-025（07-30）时点记录 57 条——此后又 +1，一致。
- 路由：src/routes/mod.rs `.route(` 恰 235 处（**亲验**）。
- collection accessors："当前 61 个"（data-and-api，2026-07-24 快照，未对码现值）。
- 生产部署链（system-review production-release 系列）：07-25 SHA 539eff…（m049）→07-26/27 dabddf…（HC-014 批）→07-27 c98f24…（SR-129/130）→07-28 5df573…（wave1）→11d9b6…（HC-029）→f0ead4…（HC-026/m039）→07-29/30 d0b7ff…（HC-004/SR-025，PID 2410064）。

### 4.3 文档间互相矛盾处清单（详证见 §5）

| # | 矛盾 | 两侧 |
|---|---|---|
| M1 | 合并基线数字与 PBT 集合 | autonomy spec 78/33（含 string_fact_risk_guard）vs evolution spec 313/37 vs digest spec 78/33 vs CLAUDE/agent-policy/roadmap/smoke-runbook 350/33（含 wiki_chunk_revision_pbt）——时代演进链，历史 spec 未回写 |
| M2 | knowledgeRoutingMode 枚举值 | autonomy spec：auto_tool_loop/classic_router vs sunset-plan：tool_calling/prompt_inline（同一开关两套名；开关本体已删除） |
| M3 | 闸门形态三个版本 | CLAUDE.md"5 分数闸现行规则" vs 三 spec sunset notice"收敛 3 闸 enforce_*（见 guards.rs）" vs agent-policy"旧 enforce_* 已移除、review_passed/dual_gate/finalize 入口 + 阈值别名"——亲验支持 agent-policy 版本 |
| M4 | evolution 自动发布 | spec R9.6 禁止 vs 生产存在 auto_release_eligible_thresholds（SR-180）vs architecture.md"当前政策 false 强制人工"——契约冲突未回写 spec |
| M5 | worker 数量 | reading-notes（PR#223）最多 12 vs HC-033 修订"13 个 spawn_supervised" vs architecture.md 现文 14 条——时点演进，未对码现值 |
| M6 | NextBestActionScore 公式 | agent-policy 版（…+Timing−DisturbanceCost−HallucinationRisk−GroundingRisk）vs wechatagent-project-knowledge 旧版（…−PressureRisk−FactRisk） |
| M7 | tasks 勾选 vs 交付事实 | 三 spec tasks 曾全 `[x]`（SR-179 记录）→现改 `[~]`+manifest 权威；evolution tasks 内部"Historical Done Notice"与"Phase A 弃用项"并存 |
| M8 | 47 域审计权威性 | deepread-verify-result 自称 47 域 completed vs audit-status-manifest 全部改判 inconclusive（SR-183） |
| M9 | MCP server 地址 | 5–6 月文档 47.108.57.147:3001 vs 7 月文档 117.72.54.28:3001（迁移非矛盾，但读者易混） |
| M10 | product-modules 的知识工具面/items 实体 | "list_catalog/search/open_slice/open_evidence 四工具 + items 主题包"现状式描述 vs tool-loop 退役（sunset-plan）与 SR-112"已删除的 items 实体"——未对码 |

---

## 5. 偏差与疑点

### Q1（对照点校正）"user.reply.task 已退役"——本节初版结论为误报，已由主会话裁决修正
本记录初版依据"`src/prompts.rs:1302` 存在活跃 PromptSpec + 守护测试钉住其 final-only 形态"断言该 key "仍在生产主链路（首发/rewrite/revision 三站点共用）"。**主会话裁决（2026-08-13，全量无截断 Grep src/）：此断言错误**——`"user.reply.task"` 全部 16 处命中分布为：prompts.rs spec 定义与其自身单元测试（1302/2770/2839/2918/2996）、prompt_guard 治理面（42/64）及其测试（251/380）、run_audit/budget/prompt_template_versions/m043 的测试 fixture。**没有任何 `load_prompt*` 或 `generate_agent_json` 生产调用点使用该 key**；生产决策链三站点（首发/targeted rewrite/revision）统一走 `user.reply.fast.task`（decision.rs:460,1321），`agent/mod.rs:230-232` 注释明确称其为 "the retired full task"。守护测试钉的是模板内容本身（防种子被改坏），不构成"被生产消费"的证据。正确结论：① `user.reply.task` 处于"种子仍种入、治理仍覆盖、运行时零消费"的退役态；② user-ops tool-loop（reply_with_tools_loop）的物理删除（sunset-plan 2026-06-23）是另一件独立退役事实，两者不应混同。方法论教训：判断"是否在生产使用"必须验证调用点，spec/测试的存在性不是证据。

### Q2 三个 spec 的 sunset notice 自身已部分过时
三份 sunset notice（2026-05-25）说"运行时收敛为 3 闸 enforce_knowledge_grounding/enforce_hallucination/enforce_run_budget，详见 src/agent/guards.rs"。但 agent-policy.md（更新至 8/5）顶注说明**旧 enforce_* 函数已移除**，在线入口是 review_passed / classify_dual_gate / route_dual_gate / finalize_review_for_send，且 HumanLike/EmotionalValue/PressureRisk 已作为软闸补回（Phase B"恢复 5→3 闸缺口"）。亲验 gates.rs/runtime.rs 支持后者（分数阈值字段 + hallucination/knowledge_grounding 别名映射真实存在）。即：**读 sunset notice 也要打折——它描述的"3 闸 enforce_*"是中间态，现状是"评分硬门（hallucination/grounding）+ 三软闸（humanLike/emotionalValue/pressureRisk）+ RunBudget"的分数闸体系**。

### Q3 基线数字演进链未回写历史 spec
78/33（autonomy，PBT 含 string_fact_risk_guard）→313/37（evolution）→350/33（knowledge-cleanup 后，PBT 集合把 string_fact_risk_guard 换成 wiki_chunk_revision_pbt；CLAUDE.md/agent-policy/check-baseline 同步）→实测总数另一体系（runbook 381→416；FINAL-SUMMARY 1814；smoke 1821）。digest spec R7.1 与 tasks 验收（80/86/92/96）沿用 78 时代。**风险**：按旧 spec 数字设门会放水；按"总数"理解"阈值"会误判回归。runbook 红线 R3 的表述（"跌破最近一轮 baseline、最低不低于 78"）是滚动基线语义，与固定 350 阈值也不同。

### Q4 evolution 自动发布契约冲突（SR-180）
spec R9.6 明令"本期不引入自动发布开关，放宽须独立 M5+ spec"；agent-policy 演化章只描述 admin 手动 release；但 findings SR-180（FACT）：生产 tick 末尾存在 `auto_release_eligible_thresholds`（EVOLUTION_AUTO_RELEASE_ENABLED env + workspace flag 双闸、默认关、仅 threshold、synthetic actor）。architecture.md 加注"当前代码政策 CURRENT_AUTO_RELEASE_POLICY_ENABLED=false 强制所有 proposal 人工发布"。**同仓库同时把"永远需要 admin"与"可配置自动 release"当权威**；HC 决策（reading-notes 首轮人类确认）="Evolution 默认关闭；现阶段所有发布人工确认；未来只有安全收紧类阈值可进显式白名单"——但 requirements-first spec 未修订。

### Q5 任务账本失真已被制度化纠正，但历史文件保留误导面
SR-179（FACT）详列三 spec 任务表把未接线（Run Envelope started 信封当时未接线）、已删除（4 evolution E2E、significance PBT、P7）、验收不等价（worker_reclaim 不调私有 reclaim、dry_run_isolation 手插状态行、reaction_claim_lock 不调 record_user_reaction、last_inbound_split 复制 update 语句）的工作标 `[x]`。纠正=manifest 权威 + tasks 顶部注记 + `[~]` 重标。**残留疑点**：manifest 仅覆盖三 spec；digest 与 universal-test-coverage 无 manifest 条目（前者靠 docs 现状叙述、后者全部未勾选）；manifest asOf 2026-07-24，此后（07-25~07-30 密集生产发布）状态是否漂移未更新。

### Q6 47 域"事实底座"不可作为上线证据（SR-183 + audit-status-manifest）
deepread-verify-result-2026-06-30.json 声称 47 域全部 completed、300 条 verified_gaps/190 孤儿；audit-status-manifest 把全部 47 域改判 inconclusive（无冻结 commit/模型指纹/输入哈希；agentRetry 失败静默丢域；FALSIFY_SCHEMA 必填不含证伪数组；统计 300/190 与机器复算 300/182 不符）。**任何引用"47 域权威行为清单"的下游文档（含 2026-06-30 上线前测试方法论 design——本次未读该 superpowers 文件）都应视为引用 research_leads 而非事实**。

### Q7 真模型"红线硬门"允许零样本绿（SR-178）
findings 记录 nightly real-llm-redline 六文件中多数把"没有产生可检查产物"（身份生成 None/roleplayer 全 fallback/零 escalation/零 task/零 reply）编码为普通成功返回且不记 skip；skip-gate 文件缺失时输出"0 skip 全部真跑"。唯一正向见证模板=principal_relay 与 cross_domain（非空回复→outbox→MCP 反查）。**与 universal-test-coverage R0 的立意（堵假绿）形成"加固后仍有洞"的第二层疑点**；修复态未对码。

### Q8 memoryCard 双不变量数学不可满足（SR-182）
hardening R8 同时要求 coreFacts cap=6 与"未 discarded 的初始集合 S 最终 coreFacts ⊇ S"；超 cap 时生产直接 truncate(6)（新值前缀优先，静默挤掉旧核心事实、不迁 deprecated 不留痕）；PBT 在 total>6 时主动跳过该性质，绿测掩盖矛盾。属"spec 合同缺陷 + 实现静默仲裁 + 测试规避"三连；文档侧无任何一处回写修正。

### Q9 operator memory"随时可撤销"承诺无撤销路径（SR-181）
digest spec R5.4 要求写入附 memoryId 让运营可撤销；生产只有新增/读取路径（intent 闭集无 revoke、无删除/失效 API、expires_at 默认 None）。agent-policy digest 章复述了"AI 不静默写"但同样未提撤销缺口。

### Q10 runbook ISSUE-012（知识红线三层联动失效）终态悬置
R16 实测：Reply Agent 内联 knowledgeNeed=not_required 短路知识路由 + R5.7 反向门 verified_chunks=[] 永真 + review LLM 对绝对承诺 factRisk=1——红线靠 LLM 自我克制兜住。runbook 已 sunset 不再更新；后续证据链（07-05 smoke"已核实知识解锁产品红线 grounding 4→10 / kcov=missing→enough"、agent-policy 的 grounding 分数闸描述、B-1 升档链路完整跑 review/rewrite）表明知识 grounding 机制已重构，但**三选一（prompt 强制/无条件路由/反向门 fail-closed）具体采纳哪条无文档直接回答**——未对码。

### Q11 文档快照类声明的时效
data-and-api"61 个 accessor"、architecture"14 workers"、findings/HC 系列的 file:line 与生产 SHA/PID 等均为各自时点冻结值；本记录只转述不外推。migrations 57（07-30 记录）vs 58（今日亲验）即一例——文档正确于当时。

### Q12 小型笔误/龃龉
- DEPLOYMENT-STEPS 步骤 9.4 用 `db.agent_runs`（应为 agent_run_logs，未对码）。
- hardening R19.2 指标名 human_handoff_success_rate 含后来被禁的词根（历史档案未清理；no-human-takeover lint 针对新增行不追溯历史文件）。
- product-modules 一级模块列表 9 项与"8 模块"结构表述不一致（工作台+8）。
- hardening design 迁移 id（2026_05_001_*）与代码迁移文件名（m001_*）两套命名（内容对应）。
- biztest-article-edu.md 内含"保证学会/包教包会"等违规话术——**测试素材有意为之**，不应被文档扫描类工具误判为产品文案。

---

## 6. 覆盖自证（读过的文件 + 行数 + 读法）

### 6.1 `.kiro/specs/`（15 个文本对象；md 全部逐字全文）

| 文件 | 行数 | 读法 |
|---|---|---|
| task-status-manifest.json | 238 | 全文 |
| agent-autonomy-loop/requirements.md | 593 | 全文 |
| agent-autonomy-loop/design.md | 1648 | 全文（1–850 + 850–1649 两段连续） |
| agent-autonomy-loop/tasks.md | 474 | 全文 |
| user-ops-agent-hardening/requirements.md | 378 | 全文 |
| user-ops-agent-hardening/design.md | 1403 | 全文（1–760 + 760–1404） |
| user-ops-agent-hardening/tasks.md | 455 | 全文 |
| agent-self-evolution/requirements.md | 270 | 全文 |
| agent-self-evolution/design.md | 767 | 全文 |
| agent-self-evolution/tasks.md | 430 | 全文 |
| knowledge-digest-workstation/requirements.md | 169 | 全文 |
| knowledge-digest-workstation/design.md | 363 | 全文 |
| knowledge-digest-workstation/tasks.md | 130 | 全文 |
| universal-test-coverage/requirements.md | 139 | 全文 |
| universal-test-coverage/tasks.md | 111 | 全文 |
| universal-test-coverage/real-llm-findings-2026-06-18.md | 302 | 全文 |
| universal-test-coverage/audit-status-manifest.json | ~240 | 头 80 行（结构+summary+records 样例；records 为 47 条同构行） |
| universal-test-coverage/{biz-domains,audit-anchors,deepread-verify-result}.json、deepread-verify-workflow.mjs | 大型数据 | 未逐字（性质经 manifest+SR-183 记录） |

### 6.2 `docs/` 顶层（26/26 全文逐字）

| 文件 | 行数 | | 文件 | 行数 |
|---|---|---|---|---|
| README.md | 48 | | mcp-deployment-guide.md | 260 |
| product-modules.md | 250 | | mcp-integration-audit-2026-07-07.md | 158 |
| architecture.md | 444 | | mcp-integration-completion-report.md | 204 |
| ai-agent-system.md | 267 | | mcp-integration-final-delivery.md | 195 |
| agent-policy.md | 749 | | remaining-issues-summary.md | 226 |
| data-and-api.md | 707 | | DEPLOYMENT-STEPS.md | 483 |
| development-roadmap.md | 154 | | FINAL-SUMMARY-2026-07-07.md | 324 |
| frontend-design-system.md | 194 | | wechatagent-project-knowledge.md | 134 |
| knowledge-wiki.md | 202 | | real-llm-test-authenticity-audit.md | 91 |
| ci-known-gaps.md | 17 | | real-task-loop-prompt.md | 132 |
| sunset-plan.md | 153 | | test-paradigm-llm-driven-analysis.md | 78 |
| real-task-runbook.md | 1562（全文，1–800 + 800–1562） | | universal-domain-test-gap-audit.md | 79 |
| universal-domain-base-extensibility-audit.md | 147 | | universal-domain-base-robustness-roadmap.md | 120 |

### 6.3 `docs/smoke/`（5/5）

| 文件 | 行数 | 读法 |
|---|---|---|
| 2026-07-05-full-project-smoke-findings.md | 64 | 全文 |
| 2026-07-05-newuser-journey-four-way-audit.md | 165 | 全文 |
| user-ops-smoke-runbook.md | 350 | 头 120 行 + 尾 72 行（§0–2.2 与 §3.5–5 checklist；中段 webhook 用例细节略） |
| biztest-article-edu.md | 33 | 全文 |
| knowledge-smoke-doc.md | 31 | 全文 |

### 6.4 `docs/system-review/`（26 文件；md 全部至少头部+结论段）

| 文件 | 行数 | 读法 |
|---|---|---|
| README.md | 9 | 全文 |
| review-plan.md | 152 | 全文 |
| baseline.json | 15 | 全文 |
| findings.md | 1643 | 头 60（SR-001~008）+ 尾 61（SR-177~183 全文） |
| human-confirmation-checklist.md | 642 | 头 70（HC-001~004）+ 尾 44（HC-033 尾~HC-036） |
| two-pass-review-ledger.md | 480 | 头 40（判定规则 + HC-003/014 实施证据） |
| reading-notes.md | 2101 | 头 30（B01）+ 尾 32（两轮复审收口 + R01） |
| architecture.md | 1584 | 头 40 + 尾 36 |
| data-model.md | 142 | 头 30 + 尾 29（逐集合风险索引+未完成反查声明） |
| automation-script-review.md | 56 | 全文 |
| hc001-credential-rotation-runbook.md | 139 | 头 40（完成标准+授权边界+步骤 1–2） |
| security-incident-hc001-2026-07-30.md | 43 | 全文 |
| production-release-2026-07-25.md | 141 | 头 40（含结论边界节） |
| production-release-2026-07-27-hc014.md | 59 | 全文 |
| production-release-2026-07-27-hc015.md | 20 | 全文 |
| production-release-2026-07-27-hc016.md | 20 | 全文 |
| production-release-2026-07-27-hc019-relationship.md | 47 | 全文 |
| production-release-2026-07-27-hc020.md | 11 | 全文 |
| production-release-2026-07-27-hc021.md | 13 | 全文 |
| production-release-2026-07-27-hc022.md | 11 | 全文 |
| production-release-2026-07-27-hc023.md | 11 | 全文 |
| production-release-2026-07-27-sr056.md | 28 | 全文 |
| production-release-2026-07-27-sr129130.md | 53 | 全文 |
| production-release-2026-07-28-hc026-m039.md | 23 | 全文 |
| production-release-2026-07-28-hc029.md | 27 | 全文 |
| production-release-2026-07-28-wave1.md | 39 | 全文 |
| production-release-2026-07-30-hc004-sr025.md | 50 | 全文 |
| file-ledger.csv / automation-read-evidence.json / build-ledger.ps1 / check-ledger.ps1 / update-ledger.ps1 | 机器台账/脚本 | 未逐字（性质见 §1.4） |

### 6.5 代码锚点核验（只读）

| 锚点 | 命令/位置 | 结果 |
|---|---|---|
| 迁移至 m058 | Glob `src/db/migrations/m0*.rs` | 58 个文件（m001–m058，最新 m058_llm_provider_active_invariant.rs）✅ |
| 路由数 235 | `rg -c '\.route\(' src/routes/mod.rs` | 235 ✅ |
| 分数闸现状 | Grep `src/agent/review/gates.rs`、`src/agent/runtime.rs` | human_like_rewrite_below / emotional_value_rewrite_below / pressure_risk_block_at 判分存在；runtime.rs 存在 "factRiskBlockAt"→"hallucinationBlockAt" 别名映射与 `fact_risk_block_at: typed.hallucination_block_at` ✅（支持 agent-policy 阈值别名注记） |
| user.reply.task | Grep src/（12 文件命中）+ prompts.rs 上下文 | ~~key 仍为活跃 PromptSpec，对照点需修正~~ **主会话裁决：初判误报。全部命中为 spec 定义/治理面/测试 fixture，无生产调用点；生产只走 user.reply.fast.task（见 §5-Q1 修正版）** |
