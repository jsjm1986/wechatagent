# 未提交改动与脚本/CI 深读记录（核证日期 2026-08-13）

> 方法：`git status` / `git diff --stat` / 逐文件 `git diff -- <file>` 逐 hunk 读完；对每个改动文件结合当前工作区代码（Read/Grep 亲验）判断意图；scripts 与 CI 全集逐个读完。所有断言附 file:line 或 diff hunk 证据；无法从代码直接证实的判断标注"推断"。
>
> 实际快照：git status 共 **47 个 M 文件**（scripts/biz-test 24 + src 14 + tests 9），`git diff --stat` 合计 **+3285/−1428**（47 files changed）。任务描述中的"45 个文件 +2986/−1394"与此不符——推断为任务方统计时点更早、其后又有增量编辑（`ls -la scripts/biz-test` 显示 `_lib.py`/`batch_a_domain13.py` 等 mtime 到 2026-08-13 01:36，仍在活跃修改中）。另有 untracked：`PROJECT_UNDERSTANDING_LEDGER.md`、`project-understanding/`（本记录所在，之前理解任务的产物，不属于业务改动）。
>
> 编译核证：`cargo check` 于 2026-08-13 在含全部未提交改动的工作区通过（exit 0，29.6s，无 warning 输出到 tail）。

---

## 1. 未提交改动全景（分组）

47 个文件的改动可归纳为 **6 组后端业务工作 + 1 组贯穿性的 biz-test 验收体系硬化**。它们不是互不相干的杂项：biz-test 组（G7）的大量新断言**直接依赖** G1–G6 的后端新行为，两侧是同一波"生产验收精确化"工作的两半。所有 src 改动均带新单测（diff 内可见），`cargo check` 通过；集成测试多为 `#[ignore]`（需 Docker/CI），本机未跑。

### G1：Lean/Full 业务上下文强升重构 + 引荐卡分层可见性

- **文件**：`src/agent/sufficiency.rs`、`src/agent/referral.rs`、`src/agent/decision.rs`、`src/agent/gateway.rs`；验收侧 `scripts/biz-test/batch_a_domain4.py`、`batch_a_domain5.py`
- **意图**：把"什么时候必须强制升 Full 档"从单一谓词（自评 enough + 知识覆盖 missing + 需知识）扩展为"存在任何需要加载的业务上下文证据即升"，并把引荐卡（referral card）从 Full 档懒加载改为 gateway 统一预加载、按档位分级暴露（Lean 只见无 id 概览、Full 才见可选 id 清单）。
- **关键证据**：
  - `sufficiency.rs` diff：删除 `should_force_full_on_missing`，新增 `forced_full_context_reason(decision, has_cited_knowledge_context, has_explicit_referral_context) -> Option<&'static str>`，三个原因：`lean_declared_knowledge_required` / `knowledge_route_cited_context` / `explicit_referral_context_requested`；注释明言"This chooses only the context tier, never the business action"。旧的 4 个 force_full 单测删除，换 1 个新单测（非 enough 决策归普通升档分支管）。
  - `referral.rs` diff：新增 `render_referral_overview`（"可引荐的专属顾问线索（仅概览…）"，单测断言 overview 含 display_name/hint 但**绝不含 ObjectId hex 与 target wxid**）与 `explicitly_requests_referral_context`（窄确定性谓词：命中"安排顾问/推荐顾问/专属顾问"等 7 个请求词，排除"如果/比如/示例/怎么回复"引用语境与"不用/暂不"等否定前缀）。
  - `gateway.rs` diff：`GatewayBusinessInputs` 增 `referral_cards`；`load_gateway_business_inputs` 增 `assist_on` 参数，assist 开才 `load_referral_cards`（tokio::join! 并行装载）；主流程新算 `has_cited_knowledge_context =` 选中 chunk 非空且非 fallback、`has_eligible_referral_candidate`、`has_explicit_referral_context`，`ptier_forced_full` 事件 details 新增 `reason` 及三个布尔佐证。7 处 `DecisionRunSnapshot { .. }` 构造点全部补 `referral_cards: &referral_cards`。
  - `decision.rs` diff：`FullReplyContext` 删除 `referral_block` 字段（`ReplyContextCache::full` 不再拼引荐）；`decide_reply_with_promote` 主流程改为：assist 开时优先取 snapshot 的冻结卡（`snapshot.referral_cards.to_vec()`），Full（include_business）渲染 `referral_block`、非 Full 渲染 `referral_overview`，两者互斥拼进 prompt：`format!("{referral_block}{referral_overview}{assist_escalation_hint}{assist_redline_yield}")`。
- **完成度**：看起来完成。逻辑、事件、单测、biz-test 验收（domain4 断言"明确顾问请求先 `ptier_forced_full` 再名片入 outbox 且精确匹配 card_id"；domain5 断言"寒暄精确停 Lean（tiers==['lean']）、复杂咨询因命中审定知识必升 Full"）四层齐备。
- **风险点**：
  1. 强升条件放宽（`lean_declared_knowledge_required` 不再要求 coverage==missing，只要 Lean 自评 enough 且声明需知识即强升）→ Full 触发率上升 → 每轮 LLM 成本上升（推断，无量化数据）。
  2. `explicitly_requests_referral_context` 是字符串谓词，词表窄（7 个短语），漏报时回退到原有 LLM 自主判断路径，属安全方向；误报侧已用否定/引用过滤。
  3. `render_referral_overview` 向 Lean 暴露顾问 display_name 与 send_trigger_hint——若运营在 hint 里写敏感话术会进低档 prompt（推断，属产品口径问题非代码缺陷）。

### G2：确定性 reaction 下限（省略式停发 + 显式购买承诺）+ roleplay fixture 加固

- **文件**：`src/agent/reaction.rs`；`tests/reaction_claim_lock.rs`、`tests/reaction_stop_cancels_outbox_integration.rs`、`tests/common/roleplay_fixtures.rs`、`tests/roleplay_fixtures_smoke.rs`；验收侧 `scripts/biz-test/batch_a_domain8.py`
- **意图**：两个高价值用户信号从"LLM 分析"降为"确定性字符串下限（floor）"：(a) 中文省略式停发（"别再发了"）在同消息含终止语境（"不想聊了/到此为止"）时判持久 opt-out；(b) 显式购买/付款承诺（"我要买/帮我下单/现在付款"等 17 个短语）在交易域 profile（`transaction_facts_enabled`）下跳过 LLM 直接产出 `buyingSignal:true, deterministic:true, confidence:100` 的 reaction 标签。
- **关键证据**：
  - `reaction.rs` diff：`explicit_stop_intent` 追加 `ELLIPTICAL_SEND_STOP`×`TERMINAL_CONTEXT` 双条件（注释明言防"这个文件别再发了"误判）；新增 `explicit_buying_intent`（长度 ≤120 字、排除"如果/客户说/不买/取消订单/再考虑"等 25 个反例 marker）；`record_user_reaction_inner` 中 `deterministic_buying = active_profile.transaction_facts_enabled && explicit_buying_intent(..)` 优先于 budget 分支，注释强调"never an outcome_event and never proof that payment actually completed"。
  - `roleplay_fixtures.rs` diff：`seed_active_domain_profile` 改为 **mutate 先执行、之后统一 pin identity/release 字段**（id/workspace_id/profile_id/version/release_status/is_active/current_version/seeded_by/created_at/updated_at）；`seed_emotional_companion_profile_in_workspace` 从"抄 7 个字段"改为 `*profile = template` 整体复制，注释点名旧法"left transaction_facts_enabled=true and other sales defaults active in 'emotional' tests"——这直接服务于 G2 的 profile 门槛测试。
  - `reaction_claim_lock.rs` diff：新增 164 行大测试 `explicit_buying_floor_is_claim_scoped_transaction_only_and_zero_llm`：买家轮 `app.llm.calls()==0` + outcome=`user_replied_buying_signal` + `reaction_analysis.deterministic==true` + 无 dealVerified/paymentVerified；无前序 sent Review 时零 review/trajectory/LLM；否定语句走模型路径（llm.calls==1）；情感陪伴模板下同样字面走模型（llm.calls==2、outcome=`user_emotion_opened_up`）。
  - `reaction_stop_cancels_outbox_integration.rs` diff：确定性 stop 红线测试的输入换成新短语组合"别再发了，我不想聊了，到此为止吧"。
- **完成度**：完成。谓词+调用点+lib 单测+集成测试+biz-test（domain8 重写为"一段消息即验证 stop run 终态 `user_reaction_stop_requested` + `contact.operation_policy.explicitStopRequested==true` + 在途 outbox==0"，购买场景验证前序 sent review 的 outcome 被确定性改写）。
- **风险点**：确定性词表绕过了 LLM 语义判断，买卖双向都可能有未覆盖的表达形态；设计上漏报回退 LLM 路径（安全），误报侧靠反例 marker 拦截，marker 列表维护成本长期存在（推断）。

### G3：knowledge 运行归因（runId 贯穿）+ import 绝对承诺 riskNotes 确定性下限

- **文件**：`src/routes/knowledge/mod.rs`、`import.rs`、`repair.rs`、`verify.rs`、`src/routes/management.rs`；`tests/knowledge_auto_verify_enforce_integration.rs`；验收侧 `batch_a_domain1.py`、`batch_a_domain13.py`、`batch_c_management.py`、`cleanup.py`（知识/管理级联清理）
- **意图**：(a) 每个 knowledge 工作流（auto-verify / repair propose / repair followup）在**任何 LLM 调用之前**先写一条持久 `knowledge_run_started` 事件（kind/status=running/details 含 runId+operation+chunkIds），注释明言 fail-closed 动机："without it, contact-less failed runs cannot be attributed or safely reconciled/cleaned after an interruption"；响应与完成事件回传 runId/chunkIds。(b) 导入预览的 `document.riskNotes` 增加确定性下限：原文出现"保证学会/包教包会/全市第一/无条件退款/百分百有效/稳赚不赔"等 17 个绝对承诺 marker（带否定前缀过滤、逐行引用原文、去重）时必然落 riskNotes，与模型自报 notes 合并；prompt 模板同步加规则。(c) management plan 的 LLM 调用带 run_id（= management session id 的 hex），供 `assert_llm_success_for_run` 精确取证。
- **关键证据**：
  - `mod.rs` diff：`deterministic_import_risk_notes`（MARKERS×NEGATED_PREFIXES，note 格式"原文含需人工核验的绝对承诺：{line}"）+ `merge_import_risk_notes`；`normalize_operation_knowledge_preview_document` 与 `default_operation_knowledge_preview_document`（LLM 失败的 fallback 文档）都接入；新增 `record_knowledge_run_started`；3 个新单测（模型 notes 保留+确定性补足 / 否定句跳过+去重 / fallback 文档也有 floor）。
  - `verify.rs` diff：cursor 先 `try_collect` 成 candidates → 非空则 `record_knowledge_run_started("knowledge.auto_verify", candidate_chunk_ids)` → 循环处理累计 `processed_chunk_ids` → 完成事件与 JSON 响应都带 `runId`/`chunkIds`。
  - `repair.rs` diff：propose 与 followup 各自 `record_knowledge_run_started`；`chunk_repair_session` 事件 details 与响应 JSON 增 `runId`。
  - `management.rs` diff：`build_management_plan` 新增 `run_id` 参数，`post_management_message` 传 `session_id.to_hex()`，LLM 调用第 4 参从 `None` 变 `Some(run_id)`。
  - `knowledge_auto_verify_enforce_integration.rs` diff：断言响应 runId/chunkIds（3 条）与 `knowledge_run_started` 事件 `$all` 匹配、status=="running"；失败批次（revision 不 commit）**start 审计仍存活**。
- **完成度**：完成，且 biz-test domain13 已重写为按 runId/chunkIds/revision/usage-ledger 精确绑定取证（`av.get("chunkIds") == [auto_id]`、`usage.run_id == auto_run_id`、`route.revisionId == revision.revision_id`、repair runId 前缀 `repair-chunk-{id}-`）。
- **风险点（高）**：`import.rs` 模板新增行与 `mod.rs` 生产代码新增行含"人工"字样，将命中 CI 禁词 lint——详见 §6-疑点 1，**提交前必须改措辞**。

### G4：Guide Preview 只读化（missing-memory 冻结插入协议）

- **文件**：`src/models.rs`、`src/routes/guides.rs`；`tests/sr094_runtime_parameters.rs`、`tests/transactional_admin_flows.rs`；验收侧 `batch_c_guide.py`
- **意图**：修复 Guide Preview 的写副作用——旧代码 `preview_user_operation_guide` 调 `ensure_operating_memory`（不存在则**创建** operating_memory 行）。新代码 Preview 改调只读 `read_operating_memory`（`src/routes/shared.rs:438`，已存在的共享函数，亲验），缺失的 memory 在进程内投影；若 Preview 时无持久化行，把整个投影文档（去 `_id`）冻结进 `GuideFrozenPlan.memory_insert`；Apply 确认时走 insert-if-absent 分支（session 内 `find_one` 有行即 `guide_memory_changed` 冲突，无行则 `frozen_insert + memory_set` 插入），有行场景保留原 `updated_at` OCC update 协议不变。
- **关键证据**：`models.rs` diff `GuideFrozenPlan` 新增 `memory_insert: Option<Document>`（serde default + skip_serializing_if，向后兼容旧 plan 文档）；`guides.rs` diff `freeze_guide_plan` 的 `memory.id.is_none()` 分支与 `apply_claimed_user_operation_guide_v3` 的双分支写入；`sr094` 测试改为断言 Preview 前后 memory 计数均为 0、Apply 后恰为 1；`transactional_admin_flows.rs` 补 `memory_insert: None` 字段适配。
- **完成度**：完成。biz-test `batch_c_guide.py` 重写为 v3 全协议验收：Preview 零业务写入（contact 不变 + memory 计数 0）、篡改 candidateHash 被拒且零副作用、Apply 后 `_memory_count==1`（"确认后恰好创建冻结记忆基线"）、重复 Apply 幂等返回同一持久化回执、审计事件恰 1 条。
- **风险点**：insert 分支的冲突语义是"Preview 后有人先建了 memory → 整单拒绝（guide_memory_changed）"，符合冻结候选哲学；`memory_insert` 冻结的是投影文档全量，若 Preview 与 Apply 之间 OperatingMemory 结构演进（新增必填字段），插入的旧形文档可能缺字段（推断，受 serde default 保护程度取决于字段定义）。

### G5：DomainProfile activate 显式恢复默认状态机

- **文件**：`src/routes/domain_profiles.rs`；`tests/domain_profile_e2e.rs`；配套 `batch_b_industry.py`（崩溃恢复协议）
- **意图**：修复"无内嵌状态机的 profile（如 DEFAULT 销售域）activate 时不动 `operation_domain_configs` → 上一个自定义行业的 workspace-global 状态机**残留生效**"的缺陷。新逻辑：每次 activate 都解析有效状态机——profile 无 `generated_state_machine` 时用 `crate::prompts::default_user_operation_state_machine()`（`src/prompts.rs:760`，亲验存在）显式 publish 成新 current 版本。
- **关键证据**：`domain_profiles.rs` diff：`let effective_machine = target.generated_state_machine.clone().unwrap_or_else(crate::prompts::default_user_operation_state_machine);` 替代 `if let Some(machine) = ...`；注释明言"A profile without an embedded machine means 'use the system default', not 'keep whichever industry's workspace-global machine happened to be active before'"。`state_machine_status` 从 `mut ... = "skipped"` 改为必赋值。e2e 测试从 `e2e_activate_without_machine_leaves_configs_unchanged`（断言 config 表不变）**反转**为 `e2e_activate_without_machine_restores_default_machine`（先激活自定义状态机 → 再激活无机 profile → 断言 current.version 递增且 `state_machine == default_user_operation_state_machine()`）。
- **完成度**：完成。语义反转有测试锁定；`batch_b_industry.py` 的精确恢复协议（按 immutable row id rollout+activate）依赖此行为成立。
- **风险点**：每次 activate DEFAULT 都会新增一个 config 版本（版本表增长）；若有外部流程依赖旧行为"activate 无机 profile 不动状态机表"会被破坏（推断，未发现此类依赖）。

### G6：记忆冲突审计事件持久化（run/版本绑定 + 权威 diff 派生）

- **文件**：`src/agent/memory.rs`；`tests/sr029_memory_commit_recovery.rs`；验收侧 `batch_a_domain9.py`
- **意图**：memory consolidation 的 `memory_conflict_resolved` 审计事件从"只транскри模型自报的 conflicts 数组"升级为双源：模型自报（`auditSource:"model_conflict"`）+ 从前后两版 memory card 的权威 diff 派生新弃用事实（`auditSource:"memory_card_diff"`，含 deprecatedFactId/dimension/deprecationReason/supersededBy），全部绑定 runId/previousVersion/memoryCardVersion——即使模型漏报 conflicts，确定性同维裁决也可观测。
- **关键证据**：`memory.rs` diff 新增纯函数 `memory_conflict_audit_events(previous, current, model_conflicts, run_id, previous_version, memory_card_version)`（fact_identity 以 structured id 优先、文本兜底，跳过上一版已弃用的）；`consolidate_contact_memory_inner` 中旧 `conflict_events` 改名 `model_conflicts`，最终 `conflict_events = memory_conflict_audit_events(...)`；新单测 `conflict_audit_is_bound_to_committed_run_and_versions` 断言 2 条事件（模型上下文 + 权威 diff）各带 run/版本。`sr029` 恢复测试补断言：恢复重放后的 conflict 事件 details 含 runId/previousVersion/memoryCardVersion/auditSource。
- **完成度**：完成。`batch_a_domain9.py` 已重写为按 `memory_commit_events(task_id, claim_generation)`（dedupe_key 前缀 `memory_commit:{task}:{gen}:`）精确断言"恰 1 条完成审计绑定 run/version + 冲突事件绑定第二次 task 的 run、前后版本、auditSource ∈ {model_conflict, memory_card_diff}"，且 `deprecatedFacts` 权威归档从"可选观测"升为硬断言。
- **风险点**：低。纯审计侧加宽，commit 协议（prepared/CAS）未动。

### G7：biz-test 生产验收体系硬化（贯穿全部 24 个 biz-test 文件）

- **意图**（多条主线拧成一股）：
  1. **消灭假绿**：`_lib.expect` 语义大改——非 `low` 级断言失败当场抛 `BizTestAssertionError` 使脚本非零退出（`_lib.py:943-956`，docstring 直言旧行为"allowing those scripts to return zero made earlier matrices false-green"）；`assert_llm_success` 也改走 expect。batch_c_management 把"LLM 未规划危险工具"从 low 观察升为硬失败（"权威矩阵未形成 pending_confirmation 不能把 confirm 路径记为已验"）。
  2. **run 级精确取证**：新增 `decision_review_for_run`/`outbox_for_run`/`ptier_events_for_run`/`assert_llm_success_for_run`/`wait_review_status`/`wait_projection_terminal`/`relationship_suggestions_for_run`（经 projection_observations 台账解析）/`projection_llm_logs`（run_id+":projection"）/`memory_task_evidence`/`memory_commit_events`——所有断言绑定精确 run/task/decision/outbox 身份，"contact 级最新一条"式取证全部废除。
  3. **冻结身份绑定**：`campaign_dispatch_body`（specHash+specVersion）、`management_command_binding`（accountId+planHash）、`guide_apply_binding`（previewId/expectedAccountId/expectedContactId/candidateHash/confirmGlobalImpact）、`domain_profile_identity`——这些对齐的是 **HEAD 已提交**的服务端契约（亲验：`campaigns.rs:152/177/372` spec_hash、`management.rs:63/98/212` plan_hash+execution_unknown、`guides.rs:39/77/604` candidate_hash 均已在库），脚本重写是追平契约 + 增加"篡改哈希被拒 + 零副作用"的负向断言。
  4. **环境显式绑定**：`BIZTEST_APP_PORT`（1024-65535 校验）/`BIZTEST_DATABASE`（正则 + 禁 admin/config/local）替代硬编码 `localhost:3003`/`mongosh wechatagent`；`step0_preflight` 的 `TEST_ACCOUNT_ID` 不再默认 "2"，改为显式 env 或"唯一 online+active+齐备 app_id/webhook_secret 账号"fail-closed 自动选择（`select_test_account`）。
  5. **BLOCKED 独立记账**：`record_blocked` 写 `target/biztest_blocked.jsonl`（环境/能力受阻与业务断言失败分流）；`run_all.py` 起跑清台账、结束输出机器可读 JSON `{status: failed|passed_with_blocked|passed, exitCode, blocked}`；vision 无 provider 或无图片 fixture 一律 BLOCKED 不假绿。
  6. **BATCH_C 入正式套件**：campaign/management/guide/digital_twin/evaluation 从独立脚本升为 `run_all` 的正式批次（注释："keeping them as standalone scripts only would allow Guide/Campaign/Management regressions to go green"），batch_b（切全局 runtime）仍最后跑。
  7. **崩溃恢复协议**：`batch_b_industry.py` 切全局 active profile 前把原 active 的 immutable row id 持久化到 `biztest_control` 集合（marker `biztest_industry_profile_restore`）；`cleanup.py` 新增 `restore_interrupted_industry_profile`（读 marker → rollout（若非 current）→ activate → 验证 active 指回原 row → 删 marker；有 active biztest profile 却无 marker 则 fail-closed 抛错），且 cleanup 脚本首行加"active biztest profile 拒删"保险丝、`domain_profiles` 只删 `is_active:false`。
  8. **走正式 API 生命周期**：referral 卡从直插 mongo 改 create(draft)→review(approved)→toggle(enable)；knowledge fixture 从直插 verified 改 `seed_citable_knowledge_chunk`（强制 draft+needs_review）→ API `verify`（带 `expectedUpdatedAt` OCC token）→ `patch`（验证自动降级 draft+needs_review）；ask_human_policy 从直改 mongo 改 PUT API（DeciderRef 必须绑 accountId 且在通讯录）；行业 profile 走 generate→publish→activate 全程并断言每步（draft 未激活、publish 只动 current、activate 状态机 completed）。
  9. **生产节流尊重**：`send_and_wait` 发送前依次 `wait_contact_idle`（防 barge-in 抢占）→ `wait_contact_reply_window`（读生产 `minReplyIntervalSeconds`——runtime_parameters 优先、部署 .env 兜底——等窗口自然过期，"never clear its state"）；`reset_contact_conversation` 撤销上一轮确定性 stop 写入的持久屏障（cooldown_until + operation_policy.explicitStopRequested*，仅 biztest_ 前缀）。
  10. **修真 bug**：cleanup 旧版 f-string 转义产出 `...{wxid:/^biztest_/}]}})` 括号不平衡的 mongosh 脚本（新测试 `test_contact_root_delete_filters_are_balanced` 断言 `assertNotIn("{wxid:/^biztest_/}]}})")` 锁死修复；推断旧脚本整段 JS 语法错误、按 `mongo()` 的 fail-closed 语义从未成功执行过该段清理）；`batch_c_management.py` 的 FAKE_CHUNK 从含非 hex 字符的 `"0000000000000000biztest1"` 改为合法 `"000000000000000000000001"` 并断言其确实不存在（推断旧值会让 ObjectId 解析失败使"假 chunk no-op"验证形同虚设）。
- **完成度**：主体完成（py 单测 test_lib/test_cleanup/test_run_all/test_step0 齐备），但存在一处内部不一致（domain8 的 severity="BLOCKED" 传给 expect，见 §6-疑点 2），且整体依赖 G1–G6 的 src 行为——**src 与 scripts 必须同时合入**，否则 biz-test 断言（如 `ptier_forced_full` 原因、确定性 stop/buy、riskNotes、runId 字段、guide memory 计数）会失败。
- **风险点**：`expect` 抛异常改变了"跑完全部断言收集完整问题清单"的旧行为——现在第一处非 low 失败即中止该域脚本（后续断言不执行），发现面变窄换取信号可信度（设计取舍，非缺陷）。

---

## 2. 逐文件改动记录（47 个）

### src/（14 个）

| # | 文件 | 改了什么 | 组 |
|---|---|---|---|
| 1 | `src/agent/sufficiency.rs`（+99/−? 行） | `should_force_full_on_missing` → `forced_full_context_reason`（3 原因闭集，返回 `Option<&'static str>`）；`is_coverage_optimism` 注释同步；删 4 个旧单测、增 1 个新单测 | G1 |
| 2 | `src/agent/referral.rs`（+91） | 新增 `render_referral_overview`（Lean 概览，不暴露 card id/wxid）与 `explicitly_requests_referral_context`（显式请求谓词，7 请求词×7 否定词×7 引用 marker）；2 个新单测 | G1 |
| 3 | `src/agent/decision.rs`（±122） | `FullReplyContext` 删 `referral_block`；`ReplyContextCache::full` 签名去掉 assist/stage 参数、不再拼引荐；`DecisionRunSnapshot` 增 `referral_cards: &[ReferralCard]`（注释：Lean 只收无 id 概览、Full 收可选 id）；主流程按 include_business 二选一渲染 block/overview 进 prompt 占位 | G1 |
| 4 | `src/agent/gateway.rs`（+75） | `GatewayBusinessInputs.referral_cards` + `load_gateway_business_inputs(.., assist_on)` 预载；主流程前置解析 assist_on；计算 cited-knowledge / eligible-candidate / explicit-referral 三证据；`ptier_forced_full` 事件带 reason+3 布尔；7 处 snapshot 构造补字段 | G1 |
| 5 | `src/agent/reaction.rs`（+168） | `explicit_stop_intent` 增省略式停发×终止语境双条件；新增 `explicit_buying_intent`；`record_user_reaction_inner` 增 deterministic_buying 短路（门槛 `transaction_facts_enabled`）；a6_tests 增正反例与 2 个新单测 | G2 |
| 6 | `src/agent/memory.rs`（+138） | 新增 `memory_conflict_audit_events`（model_conflict + memory_card_diff 双源、绑 runId/前后版本）；`consolidate_contact_memory_inner` 接线；r7 测试模块增 `conflict_audit_is_bound_to_committed_run_and_versions` | G6 |
| 7 | `src/models.rs`（+4） | `GuideFrozenPlan` 增 `memory_insert: Option<Document>`（serde default，跳空序列化；注释：仅 Preview 无持久化 memory 行时的冻结插入基线，None 保持旧 OCC 契约） | G4 |
| 8 | `src/routes/guides.rs`（+61/−?） | `freeze_guide_plan` 产出 memory_insert；`preview_user_operation_guide` 从 `ensure_operating_memory` 改 `read_operating_memory`（注释：Preview 是能力提案不得创建业务状态）；`apply_claimed_user_operation_guide_v3` 双分支（insert-if-absent / OCC update）；单测 fixture 补字段 | G4 |
| 9 | `src/routes/domain_profiles.rs`（+17/−12） | activate 恒解析 effective_machine（无内嵌则 `default_user_operation_state_machine()`），无条件走 publish_state_machine_version | G5 |
| 10 | `src/routes/knowledge/mod.rs`（+178） | `deterministic_import_risk_notes` + `merge_import_risk_notes` + 两个 normalize/default 文档构造接入；`record_knowledge_run_started`（fail-closed 前置审计）；3 个新单测 | G3 |
| 11 | `src/routes/knowledge/import.rs`（+6） | `LONG_IMPORT_PROMPT_TEMPLATE` 增"绝对承诺必须写入 document.riskNotes"规则行；模板单测补断言 | G3 |
| 12 | `src/routes/knowledge/repair.rs`（+22） | propose/followup 各调 `record_knowledge_run_started`；session 事件与响应增 runId | G3 |
| 13 | `src/routes/knowledge/verify.rs`（+26/−2） | cursor→collect candidates；前置 `record_knowledge_run_started`（带全部候选 chunkIds）；累计 processed_chunk_ids；完成事件与响应带 runId/chunkIds | G3 |
| 14 | `src/routes/management.rs`（+5/−2） | `build_management_plan` 增 run_id 参数（= session id hex），LLM 调用从 None 改 Some(run_id) | G3 |

### tests/（9 个）

| # | 文件 | 改了什么 | 组 |
|---|---|---|---|
| 15 | `tests/common/roleplay_fixtures.rs` | mutate 先行 + identity/release 字段后置 pin；emotional 模板整体复制（防销售默认残留） | G2 |
| 16 | `tests/roleplay_fixtures_smoke.rs` | 增断言：完整模板 `transaction_facts_enabled==false`、`anniversaries` 日期记忆维度持久化 | G2 |
| 17 | `tests/reaction_claim_lock.rs`（+164） | 新增 explicit buying floor 大测试（zero-LLM/claim-scoped/transaction-only/negation/companion 四场景） | G2 |
| 18 | `tests/reaction_stop_cancels_outbox_integration.rs`（±2） | 确定性 stop 输入换新省略式短语组合 | G2 |
| 19 | `tests/knowledge_auto_verify_enforce_integration.rs`（+46） | 断言 runId/chunkIds 回传 + knowledge_run_started 事件（成功与失败批次都存活） | G3 |
| 20 | `tests/sr094_runtime_parameters.rs`（+33/−?） | 改走 missing-memory 路径：Preview 前/后 memory 计数 0、Apply 后恰 1 | G4 |
| 21 | `tests/transactional_admin_flows.rs`（+1） | GuideFrozenPlan fixture 补 `memory_insert: None` | G4 |
| 22 | `tests/domain_profile_e2e.rs`（±34） | `..._leaves_configs_unchanged` 反转为 `..._restores_default_machine`（先装自定义机再验证恢复 default） | G5 |
| 23 | `tests/sr029_memory_commit_recovery.rs`（+35） | prepared conflicts 与恢复断言补 auditSource/runId/previousVersion/memoryCardVersion | G6 |

### scripts/biz-test/（24 个）

| # | 文件 | 改了什么 | 关联 |
|---|---|---|---|
| 24 | `_lib.py`（+468/−?，现 1049 行） | 见 §1-G7 十条主线；另：`mongo()` 用 `shlex.quote(BIZTEST_DATABASE)`；`mongo_json` 用 sentinel 修复"合法 JSON null 被当 unparseable"歧义；`send_webhook` 强制 biztest_ 前缀（sender+msg_id）+ 每账号 `webhook_secret` HMAC；`seed_citable_knowledge_chunk` 文档注明"cannot create a verified row"（不可绕过人审生命周期） | G7 |
| 25 | `run_all.py` | BATCH_C 列表入正式套件（A→C→B 顺序，industry 最后）；BLOCKED 台账清零+汇总；末尾输出机器可读 JSON 状态 | G7 |
| 26 | `step0_preflight.py` | `select_test_account` fail-closed 唯一选择；去掉 server git HEAD 打印（本机 release 路径替代）；curl 全部走 `APP_BASE_URL`；证据脱敏（inventory 只回传布尔 has_app_id/has_webhook_secret） | G7 |
| 27 | `cleanup.py` | `_ensure_admin_cookie`（崩溃恢复先登录）+ `restore_interrupted_industry_profile`（marker 协议）；active biztest profile 拒删保险丝；知识 run（usage/started 事件/llm_call_logs by runId）与 management session（tool_calls→runs→messages→sessions 子先删）精确级联；投影实体先冻 id 再删；profiles 只删 is_active:false；括号 bug 修复；main 尾部才 logout | G7/G3 |
| 28 | `batch_a_domain1.py` | forbiddenClaims 断言（旧契约已删）→ document.riskNotes ≥3 个 marker 留痕断言 + apply 后 `risk_notes` 持久化断言（critical）；documentId 合法性硬校验 | G3 |
| 29 | `batch_a_domain2.py` | fixture 从直插 verified 改"citable draft → API verify → 正式 patch 自动降级 → 再 verify"全生命周期；`assert_llm_success_for_run`（prompt key `user.reply.fast.task`）；`decision_review_for_run` 取证 | G7 |
| 30 | `batch_a_domain3.py` | outbox/review/LLM 证据全部改 run 级绑定（`outbox_for_run` 等） | G7 |
| 31 | `batch_a_domain4.py` | 名片走 create→review→toggle 正式 API；路径1 显式 `force_off`；路径2 新增 `ptier_forced_full` 断言（"Lean 停档会导致模型永远看不到带 cardId 的已审候选"）+ outbox 精确匹配 card_id；从"LLM 单次不稳可多跑"软断言升硬断言 | G1 |
| 32 | `batch_a_domain5.py` | 种审定知识 fixture；寒暄轮硬断言 `tiers==["lean"]`、复杂咨询硬断言升 escalated/full（依赖 G1 cited-context 强升）；全 run 级取证 | G1 |
| 33 | `batch_a_domain6.py` | ask_human_policy 从直改 mongo → GET/PUT API 安装与恢复（leader 必须 ensure_managed_contact + 绑 accountId）；relay 断言从"outbox 数量增"→ durable identity 链（escalation._id==relay_task_id==agent_tasks._id → task.outbox_decision_id → review.source_task_id → outbox(decision_id+run_id)）；误报反向改"有 verified 依据的问询须 grounded sent reply 且 0 escalation" | G7 |
| 34 | `batch_a_domain8.py` | 停止：三段对话→一段即验（run 终态 `user_reaction_stop_requested` + explicitStopRequested 屏障 + 在途 outbox==0）；购买：等前序 review sent → 发确定性承诺 → 断言前序 review outcome==`user_replied_buying_signal`（依赖 G2） | G2 |
| 35 | `batch_a_domain9.py` | `_require` 硬失败风格；固化 task 断言 `status==sent && gateway_status==consolidated`；memory row 绑 `memory_source_task_id`；deprecatedFacts 归档从观测升硬断言；`memory_commit_events` 绑 run/前后版本/auditSource（依赖 G6） | G6 |
| 36 | `batch_a_domain1011.py` | 全重写：D10 收窄为只读规划验收（危险矩阵归 batch_c_management）、断言 `management.plan` 走 session id 的 run 级 LLM 证据；D11 从"如实记录无运行时闸"反转为断言 Prompt 字面双闸（`prompt_templates.rs:47/118/162/267`，HEAD 已有，亲验）首写前拒绝：禁词编辑被拒、不追加 draft、不动 current 指针/正文；禁词运行时拼接 `"".join(("人","工","接","管"))` 规避仓库字面 lint | G7 |
| 37 | `batch_a_domain13.py` | 全重写：auto-verify limit=1 独立 fixture + runId/chunkIds 冻结身份 + usage/rule-revision 精确绑定 + "AI 不放行 verified"红线；repair 独立 fixture + proposal 不改 chunk（5 个 immutable 字段前后一致）+ runId 前缀校验；completeness 必须看到 needs_review fixture 且不得 fully_supported；vision 一律 `record_blocked` | G3 |
| 38 | `batch_b_industry.py` | 全重写：biztest_control 恢复 marker（切换前持久化原 active id）；generate→publish→activate 全生命周期断言；operation_state 白名单校验（∈ 该版本生成状态机 key 集）；`_restore_original` 按 immutable row id 精确恢复、失败保 marker 不吞异常 | G5/G7 |
| 39 | `batch_c_campaign.py` | 全重写：用 preflight 账号（弃 default 账号错配路线，campaigns API 现支持 accountId——HEAD 已有 specHash 契约）；dispatch 缺/篡改 specHash 被拒 + 零副作用；campaign_send 绑 taskId/specHash；等待确定性 task 到达 Gateway 吸收终态；outbox 精确 run 取证 + 安全终态豁免 | G7 |
| 40 | `batch_c_digital_twin.py` | 全重写：弃确定性 seed 建议路线，改要求真实 post-decision Projection 两轮内自主产出（`wait_projection_terminal` + `projection_llm_logs`(run:projection) + `relationship_suggestions_for_run` 走 projection_observations 台账）；审核前不改 contact；approve 事务写回 + 台账可追溯 | G7 |
| 41 | `batch_c_evaluation.py` | 全重写：终态闭集扩展为生产同源 shadow 终态（11 值 VALID_TERMINALS，含 held_by_ai_policy 等 AI-internal 名）；`passed == (status in {would_send, no_reply})` 一致性；旧死规则字符串否定断言保留；汇总=明细核对 | G7 |
| 42 | `batch_c_guide.py` | 全重写：v3 冻结绑定（guide_apply_binding）；Preview 零业务写入（含 memory 计数 0——依赖 G4）；篡改 candidateHash 拒 + 零副作用；Apply 后 memory 恰 1 + committed 回执；重复 Apply 幂等同回执 + 审计事件恰 1 | G4 |
| 43 | `batch_c_management.py` | FAKE_CHUNK 改合法 hex + 预检不存在；confirm/reject 带 planHash 绑定；新增篡改 planHash 被拒 + 零工具副作用；二次 confirm 断言从 `already_processed_or_not_found` 改幂等返回 `canceled`（推断：服务端行为已在先前提交改为幂等回显）；第二轮独立 session；未规划危险工具从 low 观察升硬失败；conf 终态接受 `execution_unknown`（management.rs:98，HEAD 已有） | G7 |
| 44-47 | `test_cleanup.py` / `test_lib.py` / `test_run_all.py` / `test_step0_preflight.py` | 对应新协议的单测：括号平衡、知识/管理级联顺序（assertLess 语句序）、marker 恢复流、端口/DB 校验、单一 target 绑定、mongo_json null 语义、reply window、send 前置顺序（idle→window→count→send→wait）、BLOCKED 台账、expect 抛错语义、冻结身份 helper 反例、seed chunk 字段契约（body 非 content、needs_review+draft）、verify OCC、reset 只清 stop 屏障、run 级查询绑定、账号唯一选择四场景 | G7 |

---

## 3. scripts 逐个深读

### 3.1 顶层 check-* 系列（CI 门与 lint）

- **`check-baseline.sh`（137 行）/ `check-baseline.ps1`（等价 pwsh 版）**：合并硬门。step1 `cargo test --lib`（≥350 passed、0 failed，`LIB_BASELINE=350`:28）；step2 `RUSTFLAGS="-D warnings" cargo check --tests`（把 tests/ 编译纳入 warning 即失败）；step3 四个 PBT 文件（state_transition_pbt/memory_card_invariants/wiki_chunk_revision_pbt/llm_retry_jitter）累计 ≥33 passed、0 failed；step4 可选 `DOCKER_AVAILABLE=1` 时跑 `wiki_gap_signals_3kinds -- --ignored`（3 个纯 Mongo 无 LLM 集成测试，≥3 passed）。解析用 awk 抽 "test result:" 行，不信 cargo 退出码。
- **`check-no-human-takeover.sh`（91 行）/ .ps1**：字面禁词 lint。扫 `BASE..HEAD` **新增行**（默认 origin/main..HEAD），目录限 `src/agent/ src/routes/ src/evolution/ frontend/src/`；正则 `(human[_ -]?takeover|takeover|hand[_ -]?off|人工接管|人工介入|人工托管|接管|人工)`（:35，含单词"人工"与"接管"）；排除图片/测试路径（`*/tests/*`、`*.test.*` 等）与 `src/evolution/lint.rs`（词典自身）。命中即 exit 1。
- **`check-no-model-hint.sh`（106 行）**：新增行禁模型/品牌字面（gpt-N/claude-N/gemini/anthropic/deepseek-x/qwen/kimi/chatgpt/千问/豆包/文心一言/ChatGLM/模型推荐…），扫 `src/ frontend/src/ docs/`；白名单 `.env.example/config.rs/llm.rs/error.rs/tests//llm-config.md/README/real-task-*/-design.md/superpowers/plans`。
- **`check-evolution-isolation.sh`（69 行）/ .ps1**：全量扫 `src/evolution/**/*.rs` 生产段（首个 `#[cfg(test)]` 前、去注释行）禁引 `crate::agent::gateway|outbox`、`crate::mcp::`、`agent_send_outbox.insert`、`mcp_client.send`、`run_user_operation_gateway`、`handle_managed_message`、`handle_follow_up_task`——演化器不得与发送链耦合。
- **`test-ci-lints.sh`（136 行）**：lint 的 lint。动态 eval 被测脚本里那份 `FORBIDDEN_PATTERN`（不抄写防平行漂移）、sed 抽 `src/evolution/lint.rs` 的 `FORBIDDEN_LITERALS_LOWER` 词表；正向 9 fixture 必命中（含 `hand_off` 下划线历史 bug 锁死）、负向 3 必放行、lint.rs 每词必被 shell 正则覆盖（三方一致性）。
- **`check-ci-gate-policy.py`（186 行）**：SR-004 硬/软门守门器（无 YAML 依赖，自己按缩进切 job 块）。硬门集合 `HARD_JOBS={baseline, credential-probe, delivery-protocol, knowledge-evidence-gate, tenant-isolation-security, frontend-contract, real-llm-smoke-t4, real-llm-redline, skip-gate}`（:13-23，不得 continue-on-error）；软门 `SOFT_JOBS={integration, real-llm, real-llm-recall, real-llm-ops, real-llm-quality, real-llm-adversarial}`（必须 continue-on-error:true）；夜跑软门必须支持 schedule+nightly_full；`real-llm-redline` 必须 needs adversarial、`skip-gate` 必须 needs redline 且执行 capability checker；delivery-protocol 必须逐条 count+执行 12 个具名红线测试（`--ignored --exact --nocapture`）+ 执行 delivery-boundary checker + 有手动 dispatch 目标；baseline 必须执行本 checker + check-secrets + 两个 manifest checker；credential-probe 必须绑定 RSXERMU_KEY/gateway.oeezzk.cn/gpt-5.6-auto。
- **`check-delivery-boundary.py`（154 行）**：MCP 出站调用图硬白名单——`logged_send_call_for_account` 定义仅 `src/mcp.rs`×1；调用仅 outbox_dispatcher/gateway/media_send/referral 各×1；text/media/namecard 三个 delivery helper 定义与唯一调用点（outbox_dispatcher）精确计数（**exact count**，多一处少一处都红）。自带 --self-test。
- **`check-secrets.py`（291 行）**：凭据扫描（只报 path/line/rule 不回显值）。规则：credential-prefix（sk/nvapi/ghp/xoxb…+16 位体）、literal/env/workflow 赋值高熵值（entropy≥3.5、len≥20、占位词豁免）、URI 内凭据、私钥 marker（豁免 `tests/fixtures/jwt_test_private.pem`）、`.env.e2e` 禁入库；workflow 内 secret 必须 `${{ secrets.X }}` 直引；**HC-001 rotated binding**：任何用 `secrets.RSXERMU_KEY` 的 workflow，其 `REAL_LLM_*` 绑定必须逐项等于公开表（BASE_URL=gateway.oeezzk.cn/v1、MODEL=gpt-5.6-auto、JUDGE=codex-auto-review、VISION=gpt-5.6-auto 等）——防"换了密钥仍打旧网关"。
- **`check-skip-ledger.sh`（48 行）**：R0.2 skip 率硬门。统计 `$REAL_LLM_LEDGER/skip_ledger.jsonl` 行数（`unwrap_or_skip_transient!` 每次 skip 追加一行），按 kind/test 分布打印，超 `REAL_LLM_MAX_SKIP`（默认 6，CI skip-gate 用 12）exit 1——大面积 transient skip=没验证到能力=假绿。
- **`check-capability-outcomes.py`（97 行）**：SR-128/SR-178 正向证据门。22 个具名 case（k2/k3/k6/k7/k10、q3/q4、t3、recall×3、redline×11）每个必须恰有 1 份 `capability_outcome.<case>.json`，schema=`real_llm_capability_outcome/v1`，且 sha/run_id/run_attempt 与当前 GitHub run 一致（防陈旧缓存充数）、verdict==pass、attempted==true、llm_calls>0、artifacts>0、assertions_run>0。
- **`check-task-status-manifest.py`（178 行）**：SR-179。`.kiro/specs/task-status-manifest.json`（schemaVersion 1）必须覆盖各 spec tasks.md 里解析出的全部 task id 恰一次；tasks.md 不得再用 `- [x]` 权威标记；状态闭集 {planned, implemented, production_wired, verified, partial, sunset_not_shipped}；implemented+ 需 implementation 证据（file#selector 存在性校验）、production_wired+ 需生产入口、verified 需 40 位 frozenCommit + tests + 绑定的**非软门** CI job 且 job 文本内点名测试工件。
- **`check-audit-status-manifest.py`（660 行）**：SR-183 可复现 47 域审计协议。校验域清单恰 47 个唯一 id、锚点/工作流文件 SHA-256 完整性（CRLF/LF 归一化后哈希）、legacy 2026-06-30 结果一律 inconclusive（形状冻结校验）；可选 `--result` 接 schema-v2 结果（runManifest 输入哈希 + 每域固定槽位 + deepread/falsify 双 phase + evidence locator 必须真实存在于仓库文件）；内置正/负自测（3 negative + 2 v2）。

### 3.2 顶层 smoke_* / rt_send / 性能 / 远程 / historical

- **`smoke_knowledge_full_loop.py`（189 行）**：本地全链冒烟：health → import-preview（真 LLM）→ import-apply（previewId+previewHash+候选全收）→ 列 packs/chunks → 选 needs_review 跑 repair propose（有 followup 则 answer 第二轮）→ POST repair/applied 审计事件。env：`WECHATAGENT_API`（默认 127.0.0.1:8080/api）、`SMOKE_ACCOUNT_ID`（默认 1）；输入 `docs/smoke/knowledge-smoke-doc.md`。HTTP 错误即 SystemExit。
- **`smoke_knowledge_no_llm.py`（225 行）**：绕 LLM 冒烟——手工构造 document/pack/chunk 落库、验证 CRUD+apply 审计事件路径（历史背景：本机 reqwest→DeepSeek connect 502）。
- **`smoke_reimport_docs.py`（135 行）**：把 4 份 docs/ 正式文档（project-knowledge/sales-positioning/product-modules/agent-policy）逐份 import-preview+apply（真 LLM），打印累计 documents/packs/chunks 与填充率。
- **`smoke_reimport_split.py`（241 行）**：同上 4 文档但按 H2 边界切 ≤2000 字符段逐段导入（防 LLM 长生成 chunked body_decode_error），段标题"父文档 · H2"。
- **`smoke_repair_rejected.py`（183 行）**：真 LLM 批量修复存量 rejected chunks：source→repair→answer（操作员答"无更多信息"）→merge patch→PUT→verify（sourceQuote→anchor 严格 gate）。`SMOKE_REPAIR_LIMIT` 控量（默认 1）；失败打 ✗ 不 raise 让批量继续。
- **`rt_send.py`（63 行）/ `rt-send.sh`（24 行）**：真实测试投递（Windows GBK 安全版/Bash 版）——写 `target/rt-payload.json` 后 curl POST `/webhooks/wechat`。**硬编码** `APP_ID="wx_wi_8NITtM8d0csT6tYDYX"`、`FROM_WXID="fengrui86"`、`localhost:8080`，且**不带 HMAC 签名头**——只适用于 `WEBHOOK_VERIFY_SIGNATURE=false` 的本地联调环境（生产验签开启时会被拒）。
- **`gateway_performance_report.py`（67 行）与 `performance_report.py`（83 行）**：几乎相同的脚本（后者仅多换行格式化与一条注释）——都打认证过的 `GET /api/admin/observability/performance?hours&accountId&path`，session cookie（WA_SESSION）或 Bearer（WA_BEARER_TOKEN）二选一，打印 overall/byPath/stages 的 n/mean/p50/p95/p99/max。重复实现，见 §6-疑点 5。
- **`_remote_run.py`（55 行）**：paramiko SSH 执行（stdin `-` 支持大命令），env DEPLOY_HOST/PORT/USER/PASS。**注意 :18 `port = int(os.environ.get("DEPLOY_PORT", "3003"))` 默认 3003 是 app 端口不是 SSH**——`_lib.py:38` 因此强制 `os.environ["DEPLOY_PORT"]="22"`。get_pty + 合并 stdout/stderr，exit=远端码。
- **`_remote_put.py`（30 行）**：paramiko SFTP 上传（DEPLOY_PORT 默认 22），put+stat 校验。
- **`_push_bundle.py`（52 行）**：绕 SFTP 的大文件推送——本地 base64 → 每 30KB 一段 `printf '%s' >> file.b64`（shell arg 传参避 ARG_MAX/stdin 断流）→ 远端 `base64 -d`。
- **`analyze.py`/`count.py`/`list_fns.py`/`split_routes.py`**：historical 一次性工具——为 `src/routes.rs` 拆分成 `src/routes/` 模块树服务（brace-counting 定位 item 行程、manifest 驱动生成子模块）。`src/routes.rs` 已不存在，纯历史留档。
- **`backfill_knowledge_tags.py`（121 行）**：v3 prompt-pack Task 271 回填——扫 tag 三字段缺失的 chunk，逐条 POST `/api/operation-knowledge/extract-tags` 后 pymongo 写回；`--dry-run/--limit`；幂等。
- **`cleanup_non_human_managed.js`（25 行）**：一次性存量清理——5 个具名非真人号（新闻/电台/营销/自反身/企微）的 managed→normal，带前后回读。
- **`fix_sediment_titles.js`（141 行）**：一次性修正领导授权沉淀/待审核提案 chunk 的 title/body（误用 reviewer 质检点评）——备份到 `_sediment_title_backup_<ts>` → 按与 `ledger.rs::derive_sediment_title_fallback` 等价的确定性逻辑重算 → 回读校验。
- **`stay-awake.ps1`（30 行）**：Windows 防休眠（SetThreadExecutionState + 光标微动 + F15），本地运维辅助。
- **`check-audit-status-manifest.py` 已在 3.1**；`scripts/biz-test/`、`scripts/e2e/`、`scripts/diag/`、`scripts/deploy/` 见下。

### 3.3 scripts/biz-test/ 协议细节（_lib 为核）

**运行拓扑**：默认远程模式——本机经 `_remote_run.py`（paramiko，DEPLOY_HOST 默认 117.72.54.28，DEPLOY_PORT 被 _lib 强制 22，DEPLOY_PASS 必须外部 export）到 server 上执行 bash；`BIZTEST_LOCAL=1` 本机模式（脚本已托管在 server 上时直接 bash -c）。所有 API 调用是 server 上 `curl http://127.0.0.1:$BIZTEST_APP_PORT`（默认 3003），mongo 是 server 上 `mongosh $BIZTEST_DATABASE`（默认 wechatagent）。

**环境变量协议**（本次改动确立）：
| 变量 | 默认 | 校验 | 用途 |
|---|---|---|---|
| `DEPLOY_HOST/USER` | 117.72.54.28 / root | — | SSH 目标 |
| `DEPLOY_PORT` | 强制覆写 22 | — | 防 _remote_run 默认 3003 |
| `DEPLOY_PASS` | 无（必须 export） | 缺则 KeyError | SSH 密码，绝不落盘 |
| `BIZTEST_LOCAL` | 未设 | ==1 才本机 | server 本机托管模式 |
| `BIZTEST_APP_PORT` | 3003 | int ∈ [1024,65535] | API 端口显式绑定 |
| `BIZTEST_DATABASE` | wechatagent | `[A-Za-z0-9_-]+` 且 ∉{admin,config,local} | Mongo 库显式绑定（shlex.quote 后进命令） |
| `BIZTEST_ACCOUNTID` | 无（旧默认 "2" 已删） | 显式时必须唯一命中可用账号 | 测试账号；未设则自动唯一选择 fail-closed |
| `ADMIN_USER/ADMIN_PASS` | admin/admin | — | 登录 `/tmp/biztest_cookie` |
| `AGENT_MIN_REPLY_INTERVAL_SECONDS` | 未设时从部署 .env 读（fallback 20） | ∈[0,3600] | 生产限流窗口等待 |

**核心纪律**（docstring 与实现同源）：业务断言查 mongo；LLM 真调铁证查 `llm_call_logs`（run 级 `assert_llm_success_for_run` 优先，时间窗版仅存量场景）；`expect` 非 low 失败抛 `BizTestAssertionError`；环境受阻走 `record_blocked` → `target/biztest_blocked.jsonl`；发现走 `record` → `docs/superpowers/specs/2026-06-26-full-business-logic-test-findings.md`（表格 append）。webhook 注入走生产 HMAC 协议（`sign_webhook_body`：`hmac_sha256(secret, "<ts_ms>.<raw_body>")`，secret 按账号从 `wechat_accounts.webhook_secret` 读），sender/msgId 强制 `biztest_` 前缀。`api_bg` 把长 LLM 请求写成 server 端 setsid runner（脱离 SSH PTY），轮询 done 文件。`send_and_wait`＝idle（防 barge-in superseded）→ reply window（尊重生产 min-interval，绝不清状态）→ 计数基线 → 注入 → 轮询 run log（超时自动 `diagnose_no_run_log` 六路根因取证），端点偶发故障（`endpoint_glitch_recent` 识别 tool_use 劫持/5xx/timeout）自动重试 ≤2 次，业务终态不重试。

**probe 族（未改动）**：`_probe_split/_probe2/_probe3/_probe4.py` 是 2026-06 定位"consolidator 吐 blob"根因的一次性科学探针（单一职责 vs 多职责 / 脏卡沿用 / 并发争用 / 完整复刻生产调用含 ANTHROPIC_JSON_GUARD+max_tokens=8192），`probe_domain4_clean.py` 是域④引荐链路干净样本取证探针。均只读生产 prompt/provider、不改业务。

### 3.4 scripts/e2e/（41 个非图片文件）

分五族（全部为历史验证/走查工具，node 侧依赖仅 mongodb+playwright-core，见 package.json）：
1. **前端 Playwright 走查**（server 本机 chrome 打 localhost:3003）：`smoke.mjs`（登录+逐频道导航抓 console error/4xx/截图，CHANNELS 表与 channels.ts 对齐）、`sweep_tabs.mjs`（每频道点全部非提交类可点元素，跳过"删除/保存/推送/发送/确认/归档/停用"）、`batch1_monitor.mjs`（operations/autonomy/quality/sendAnalytics 只读深度走查+API 计时）、`b3_walk.mjs`（批3 运营组）、`b3_userops_walk.mjs`（userOps smart/roster/traditional 三模式）、`deep_crud.mjs`（referral/content 表单创建）、`deep_products.mjs`（产品创建）、`deep_kchat.mjs`（ChatWorkbench 起草一轮真 LLM）、`verify_fix_ui.mjs`（F-005/F-020 前端修复验证）。
2. **API+DB 级深测**（fetch+MongoClient，多针对本地 e2e 库或 117）：`deep_userops_llm.mjs`（analyze-profile）、`deep_profile_happy.mjs`（富 note→initial profile）、`deep_memory_consolidate.mjs`（种候选→consolidation run）、`deep_playbook_optimize.mjs`（版本 bump）、`deep_guide.mjs`（preview→apply）、`deep_domain_profile.mjs`（generate 落 draft 红线）、`deep_taxonomy_review.mjs`（approve/reject 写回）、`deep_simulation.mjs`（字段错配复现）、`deep_content_asset.mjs`（CRUD 往返）、`deep_knowledge_loop.mjs`（capstone：chat 起草→apply→verify→重问不再 blocked_unverified_product_claim）、`verify_fix_api.cjs`（F-013 completeness 缓存计时/F-003 tasks kind 分布）、`test_mcp_account_alias.mjs`（Workspace Key+account_alias 注入）。
3. **webhook 链路**：`seed_and_webhook.mjs`（本地 e2e 库 seed account+contact→无签名 webhook→decision/review/outbox 三段验证）、`b4_realsend.cjs`（生产 HMAC 签名真发闭环，secret 从 DB 读不落盘）、`fresh_contact_budget.mjs`/`fresh_greeting.mjs`（零历史 contact 预算/问候对照）。
4. **知识库 Wiki 专线**：`wiki_recon.py`/`wiki_recon2_console.py`（DOM 侦察截图）、`wiki_import_preview.py`/`wiki_import_e2e.py`（29KB 真实 MD 全程导入，Playwright 驱动 117 前端；路径硬编码 E:/yw/... 为历史 Windows 工作站）、`wiki_import_chunked_smoke.cjs`/`wiki_apply_test.cjs`（直连 API 验分块/计时）、`wiki_retrieval_dbcheck.cjs`(+.mongo.js)/`wiki_retrieval_test.cjs`/`wiki_retrieval_tooltrace.cjs`（#89 检索：verified 门/三类问法召回/无关问题走 fallback_rank 铁证）。
5. **清理/恢复**：`b3_cleanup.cjs`、`b3_db_check.cjs`、`b3_llm_serial.mjs`（严格串行 LLM 写操作）、`b3_userops_restore.cjs`（走查联系人画像/记忆还原，PII 走 env）、`cleanup_verify.cjs`（验证期 draft 活动清理）。
另有产物：`_real_import_user.txt`（真实导入样本文本）、`e2e_result_*.json`、十余张 png 截图（不读）。

### 3.5 scripts/diag/（11 个）

- `probe_mcp_tools.py`：拉 gewe MCP `tools/list` 权威清单+schema（117 本机、.env 凭据、只读）。
- `probe_mcp_ready.py`：轮询 `contacts_fetch_cache` 就绪后测 `contact_get_detail`+`contacts_search_remote`（昵称/头像可得性）。
- `probe_mcp_detail.py` / `probe_mcp_retry.py` / `probe_mcp_call.py` / `probe_detail_single.py`：同族——缓存结构确认 / 多次重试抓稳定形态 / 完整未剥壳 JSON-RPC 回显（带 account_alias=t-1）/ 429 冷却后单发。
- `audit_knowledge_legacy.py`：只读审计旧知识数据与通用 schema 的不兼容面（缺 catalog_summary/routing_map/triggerKeywords、chunk 缺 sourceQuote+anchors、integrity 分布）。
- `clean_knowledge_legacy.py`：上述审计的清理执行器，默认 dry-run，`CLEAN_KNOWLEDGE_APPLY=1` 实删（C 档范围：旧格式 documents 级联、孤儿 packs、verify-gate 永过不了的 chunks；顺序 chunks→packs→documents）。
- `compare_managed.py`：managed contacts 的 gateway 相关状态全景对比（cooldown/last_*/operation_policy/近 24h 进出站）。
- `drill_silent.py`：两个"沉默"嫌疑 contact 的定点钻取（72h 入站分组+全文档 keys+agent_events）。
- `llm_providers_e2e.sh`：LLM provider admin API 的 curl E2E（list/create 掩码/masked-key 更新保旧/真 key 更新/activate 幂等/test/delete），PASS/FAIL 计数。

### 3.6 scripts/deploy/（12 个）

- `candidate_smoke.py`（384 行）：发布候选在**隔离 Mongo 库**上冒烟——transient systemd 服务 + 出网封锁（loopback-only）；子进程先加载部署 .env 再叠 ISOLATION_OVERRIDES（:39-59：全部后台 worker 关闭/拉长、MCP/OPENAI 指到 127.0.0.1:9 黑洞）；因 task/import/outbox worker 无全局开关，调用方必须队列为空（QUEUE_COLLECTIONS 五集合检查）。
- `rotate_llm_credential.py`（775 行）：HC-001 泄漏 LLM 凭据轮换。preflight 只读；apply 需确认短语 `HC001-ROTATE-LEAKED-LLM-CREDENTIAL` + owner-only（0600）key 文件 + 生产队列为空 + 协议正确探针成功；.env 与 Mongo 引用在 restart/health/验证失败时自动回滚；不负责上游吊销。
- `audit_hc001_server_carriers.py`（365 行）：服务器侧泄漏 key 载体只读审计（普通文件/Git 对象/归档流式扫描 ≤4GB/服务状态），key 从 0600 env 文件读、永不打印。
- `delete_hc001_actions_logs.py`（169 行）：只删 2026-07-30 审计确认的 69 个 GitHub Actions 运行日志（EXPECTED_ID_SET_SHA256 锁定输入完整性；run/artifact/commit/branch 永不删）；确认短语 `HC001-DELETE-69-LEAKED-ACTIONS-LOGS`。
- `sync_github_secret.py`（130 行）：stdin 喂 `gh secret set`（默认 RSXERMU_KEY），值不过 argv/env，子进程输出全丢弃；确认短语 `HC001-SYNC-GITHUB-RSXERMU-KEY`。
- `ssh_put.py` / `ssh_run.py`：paramiko 上传/执行（SSH_HOST/USER/PASS/PORT），部署流小工具。
- `test_audit_hc001_server_carriers.py` / `test_candidate_smoke.py` / `test_delete_hc001_actions_logs.py` / `test_rotate_llm_credential.py` / `test_sync_github_secret.py`：上述五工具的单测（隔离序、fail-closed 分支、哈希锁）。

### 3.7 deploy.sh（仓库根，148 行）

交互式生产部署脚本（**过时快照**，头注 :3-4 固定"分支 fix/dispatcher-send-timeout-alignment (15 commits) 日期 2026-07-07"）：PROJECT_DIR=/root/wechatagent；fetch → 列待合并 commits → 人工 y 确认 → merge 到 main 并 push → 补 .env 缺失键（MCP_BASE_URL=**http://117.72.54.28:3001**、MCP_API_KEY 从部署环境注入（:18 `:?` 强制）、RUN_TOKEN_BUDGET_ESCALATED/WEBHOOK_VERIFY_SIGNATURE/AUTH_RATE_LIMIT_*）→ cargo build --release → 前端 npm build → systemctl 重启 → `curl localhost:8080/api/health` 健康检查。注意端口口径 8080 与现生产 3003（_lib/diag/deploy 工具全用 3003）不一致——推断此脚本对应旧部署形态，现已被 systemd + 手动流程替代。

### 3.8 .env.example 与 Cargo.toml

- `.env.example`（284 行，全文读）：分组见文件（应用/Mongo/MCP/媒体/LLM 并发与重试/workspace 默认/Agent 行为参数/Worker/webhook 限流/Strategic Planner（calendar/renewal/reactivation/LTV 分层/block-rate）/cold_contact/演化器（auto-release+负反应门）/Digest/Reviewer 双模/鉴权 Session/JWT/auto-ingest/自学习采集（dynamic_confidence 换血/行为信号健康度/召回探索）/`PROGRESSIVE_TIER_ENABLED=true`）。与 config.rs 差集见 §5。
- `Cargo.toml`：单 crate `wechatagent` 0.1.0（无 workspace）。依赖含精确锁定版本：`anyhow=1.0.104`、`feed-rs=2.4.0`、`lru=0.18.2`、`pdf-extract=0.12.0`、dev 侧 `serial_test=3.5.0`、`testcontainers=0.28.0`（与 cb60d1b "update vulnerable build and parser dependencies" 提交一致——推断这些 `=` 钉版本是安全修复的产物）。axum 0.7（ws+multipart）、mongodb 2.8、tokio 1.48、reqwest 0.12（native-tls+http2+stream）。

---

## 4. CI workflows 逐个深读

### 4.1 `.github/workflows/ci.yml`（1577 行，主 workflow）

**触发**：`pull_request→main` 与 `push→main`（均 paths-ignore docs/**、**/*.md）；`schedule cron '17 19 * * *'`（UTC，北京 03:17 nightly 全量真模型）；`workflow_dispatch`（dispatch_target ∈ {credential_probe, delivery_protocol, ops, nightly_full, smoke_t4, roleplay_docker, roleplay_p2, reviewer_calibration, roleplay_arc} + ops_test 文本参数）。concurrency 按 PR/ref 取消旧 run。permissions 只读（contents+pull-requests read）。全局 env：`RUSTFLAGS=-Dwarnings`、`RUST_MIN_STACK=8388608`（与生产 systemd 对齐，防深 async 栈溢出）。

**job 依赖图**：

```
changes(dorny/paths-filter: backend|frontend)
 ├─ baseline               [硬门, backend 变更, 30min]
 ├─ delivery-protocol      [硬门, backend 变更 或 手动 delivery_protocol, always(), 60min]
 ├─ integration            [软门 continue-on-error, backend, 90min]
 ├─ knowledge-evidence-gate[硬门, backend, 60min]
 ├─ tenant-isolation-security [硬门, backend, 45min]
 └─ frontend-contract      [硬门, frontend 变更, 15min]

nightly(schedule 或 dispatch=nightly_full) 串行链（守 rsxermu 并发≤2）:
 real-llm → real-llm-recall(matrix×4, max-parallel:1)
          → real-llm-ops(matrix×15, max-parallel:1)
          → real-llm-quality(matrix×8, max-parallel:1)
          → real-llm-adversarial(matrix×8, max-parallel:1, calibration 弧 120min 其余 90)
          → real-llm-redline(matrix×7, 硬门无 continue-on-error)
          → skip-gate(硬门: 汇总 ledger, MAX_SKIP=12 + capability-outcomes 22 case)

手动单跑(无前置链): credential-probe / real-llm-ops-single / real-llm-smoke-t4(硬门)
                  / roleplay-docker(无LLM) / roleplay-p2 / reviewer-calibration / roleplay-arc
```

**硬门/软门**（与 check-ci-gate-policy.py 的集合一致，该 checker 本身在 baseline 里执行形成自锁）：
- **硬门**（continue-on-error 禁用）：baseline、delivery-protocol、knowledge-evidence-gate、tenant-isolation-security、frontend-contract、credential-probe、real-llm-smoke-t4、real-llm-redline、skip-gate。
- **软门**（continue-on-error:true，诊断性）：integration（101 个 `#[ignore]` 集成测试全量 Docker 体检，`--no-fail-fast --ignored`，唯一 `--skip real_digest_chat_task_worker_produces_committed_repair_artifact`——HC-028 专用真模型硬门需生产配置）、real-llm/recall/ops/quality/adversarial 五组夜跑能力套件。

**各 job 跑什么**：
- **baseline**（fetch-depth:0）：`cargo test --lib` → `check-baseline.sh` → `check-no-human-takeover.sh`（PR 比 origin/BASE、push 比 HEAD~1）→ `check-no-model-hint.sh` → `check-evolution-isolation.sh` → `test-ci-lints.sh` → `check-secrets.py --self-test` + 正跑 → `python3 scripts/biz-test/test_cleanup.py` → `check-ci-gate-policy.py` → `check-task-status-manifest.py` → `check-audit-status-manifest.py`。
- **delivery-protocol**：`check-delivery-boundary.py`（self-test+正跑）→ 12 个具名红线测试先 `--list` 计数==1 再 `--ignored --exact --nocapture` 逐个跑（determinstic stop 屏障 / task send fencing / durable inbound handoff / principal 澄清不直发 MCP / webhook 限流 scope / provider activate 唯一 / configuration generation×3 / 迁移安全×2 / taxonomy label-only patch）。
- **knowledge-evidence-gate**：5 个具名知识证据测试（agent_eval 阈值 / chat 闭环审计 / worker 类型化裁决×3），同样 count==1 + exact。
- **tenant-isolation-security**：3 个（SR-176 真 Router 读写/过期会话边界、auth middleware 过期拒绝+登出幂等、SR-016 登录/令牌共享限流+脱敏审计）。
- **frontend-contract**：npm ci → `tsc --noEmit` → `vitest run`。
- **real-llm 五组**：被测主模型 `gpt-5.6-auto`（`https://gateway.oeezzk.cn/v1`，secret RSXERMU_KEY），文本 judge `codex-auto-review`；vision 复用 gpt-5.6-auto 多模态；可选第二/三异族裁判 NVIDIA（qwen3-next-80b / glm-5.1 / llama vision，NVIDIA_KEY 缺省自动回落不 fail）。缺 REAL_LLM_API_KEY 一律先 fail（R0.1 不假绿）；每 shard 先 `rm -rf target/real_llm_ledger`（防缓存陈旧证据）后 upload artifact（retention 30d）。MCP 永远 wiremock 桩。
- **real-llm-redline**（硬门×7 文件）：cross_domain_arc / principal_channel / proactive_outreach / dynamic_adversarial / digital_twin_arc / principal_relay / roleplay_arc——历史上这些红线只在无 key 的 integration 里被 skip=假绿（注释点名"深度审查 G1 critical"），本 job 补 key 真跑；确定性红线 panic 卡合并，端点抖动走 `unwrap_or_skip_transient!` 进 ledger。ROLEPLAYER 第三族缺 key 也 fail。
- **skip-gate**（硬门）：download 全部 `*ledger*` artifact 合并 → `check-skip-ledger.sh`（≤12）→ `check-capability-outcomes.py`（22 case 正向终态，sha/run 绑定当前 run）。
- **磁盘/缓存策略**：所有编译型 job 先 Free disk space（删 dotnet/ghc/android/boost/powershell/jvm + docker image prune，腾 ~30GB）；`Swatinem/rust-cache@v2` 缓存 registry；integration job 特意 `cache-targets:false`（100+ 测试二进制的 target 会撑爆 runner）+ `CARGO_INCREMENTAL=0` + `CARGO_PROFILE_TEST_DEBUG=0`；真模型 job 统一 `RUSTFLAGS=""`（陈年测试 warning 不阻断真实跑测，生产 -Dwarnings 由 baseline 守）。

### 4.2 `nightly-dynamic.yml`（92 行）

动态发现线夜跑（R5-T0）：`schedule '17 18 * * *'`（北京 02:17）+ 手动。单 job `roleplay-arc-nightly` 跑 `real_llm_roleplay_arc`（agent=gpt-5.6-auto，roleplayer=NVIDIA llama-3.3-70b @temperature 0.8，三族异构）。**与 ci.yml 的 R0.1 有意相反**：缺 key 只 `::warning` + 后续 step 全 if-skip、job 不 fail（发现性、不进合并门，"缺 key/端点抖动只 skip 进 ledger 不染红"）。artifact `roleplay-arc-nightly-ledger`。

### 4.3 `maycran-fix-validation.yml`（63 行，temporary）

push 到 `test/maycran-llm-backfill-20260802` 分支触发：`cargo fmt --check` → 12 个具名 action_policy/import-contract/chat-draft-contract lib 单测（--locked）→ failover 分类回归测试 → 13 个真模型集成 target `--no-run` 编译验证。临时分支专用验证线。

### 4.4 `maycran-llm-backfill.yml`（295 行，temporary）

手动 dispatch：target 选单个真模型测试文件或 all（17 个 target matrix，max-parallel:1 串行，timeout 360min）。**换供应商跑全量回填**：主模型 `claude-sonnet-4-6 @ api.maycran.com/v1`（MAYCRAN_KEY），judge=qwen3-coder-next、judge-lite=deepseek-v4-pro（同端点），第三族 judge/roleplayer=NVIDIA llama；带同端点 failover 链（`REAL_LLM_SAME_ENDPOINT_FAILOVER_MODELS: qwen3-coder-next,deepseek-v4-pro,claude-opus-4-6`）与 vision backup（nemotron-nano-12b-v2-vl）。跑法 `--test-threads=1 --nocapture`（可选 TEST_FILTER --exact）；结束后强制"real execution and positive evidence"检查（cargo exit 0 + python 校验产物）。注：此 workflow 用 MAYCRAN_KEY 而非 RSXERMU_KEY，因此不受 check-secrets 的 rotated-binding 表约束（该表只锁 `secrets.RSXERMU_KEY` 的使用处）。
- 备注：任务清单里“REAL_LLM_MODEL: claude-sonnet-4-6”与 check-no-model-hint 不冲突——lint 只扫 src/frontend/docs 三目录，workflows 不在扫描范围（亲验 `check-no-model-hint.sh:35-39` SCAN_DIRS）。

### 4.5 `maycran-model-probe.yml`（200 行，temporary）

push 到同一临时分支触发：5min 内探测 8 个候选模型 id（claude-sonnet-4-6/4.6/4-5、claude-opus-4-6、gpt-5.6-auto、codex-auto-review、qwen3-coder-next、deepseek-v4-pro）在 maycran 网关的可用性（HTTP 200 + choices 形状），支持直连/代理对比。为 backfill 选型服务。

---

## 5. 事实卡速查

### 5.1 全部 CI 硬门清单（合并被否决的确切条件）

| 硬门 | 触发 | 内容 |
|---|---|---|
| baseline | PR/push(backend) | lib ≥350/0fail；4 PBT ≥33/0fail；tests/ -Dwarnings 编译；no-human-takeover / no-model-hint 新增行 lint；evolution 隔离；lint 自测；check-secrets（含 RSXERMU 绑定表）；biz-test cleanup 单测；ci-gate-policy 自锁；task-status/audit-status manifest |
| delivery-protocol | PR/push(backend)+手动 | Dispatcher-only MCP 调用图精确计数；12 具名红线 `--ignored --exact` |
| knowledge-evidence-gate | PR/push(backend) | 5 具名知识证据测试 |
| tenant-isolation-security | PR/push(backend) | 3 具名租户隔离/鉴权红线 |
| frontend-contract | PR/push(frontend) | tsc --noEmit + vitest |
| credential-probe | 手动 | RSXERMU key + 公开绑定单次合成探针 |
| real-llm-smoke-t4 | 手动 | outbox delivery_redline_×5 + reaction_redline_×2 + T4 严格全链 transcript |
| real-llm-redline | nightly/nightly_full | 7 个真模型红线文件（确定性 panic 卡红） |
| skip-gate | nightly/nightly_full | skip ≤12 + 22 个 capability outcome 正向终态（绑当前 run/sha） |

本地对应物：`scripts/check-baseline.{sh,ps1}`（merge gate）与 `scripts/check-no-human-takeover.{sh,ps1}`（字面 lint）是 CLAUDE.md 点名的两道本地 merge gate。

### 5.2 biz-test 环境变量协议

见 §3.3 表。要点：`DEPLOY_PASS` 唯一必须外部注入；`BIZTEST_APP_PORT/BIZTEST_DATABASE` 带 fail-closed 校验（端口范围、库名正则+禁系统库）；`BIZTEST_ACCOUNTID` 不设则要求"全库唯一可用账号"，歧义即拒跑；`BIZTEST_LOCAL=1` 切 server 本机模式。产物文件：`/tmp/biztest_cookie`（admin 会话）、`/tmp/biztest_account`（`account_id|app_id`）、`target/biztest_blocked.jsonl`（BLOCKED 台账）、findings md（见 §3.3）。库内控制集合：`biztest_control`（行业切换恢复 marker `biztest_industry_profile_restore`）。

### 5.3 `.env.example` 与 `config.rs` 差异（2026-08-13 实测 comm 差集）

- **config.rs 读取但 .env.example 未列**（有默认值、未文档化）：`COMPLETENESS_CACHE_TTL_SECONDS`、`DYNAMIC_CONFIDENCE_MIN_SAMPLES`、`EVOLUTION_MAX_SAFETY_REGRESSION_RATE`、POST_DECISION 族 6 个（`POST_DECISION_WORKER_CONCURRENCY / _MAX_ATTEMPTS / _TOKEN_BUDGET / _PROMPT_MAX_CHARS / _SNAPSHOT_MAX_BYTES / _FAILED_SNAPSHOT_RETENTION_DAYS`）、SILENCE_SIGNAL 族 4 个（`SILENCE_SIGNAL_WORKER_ENABLED / _INTERVAL_SECONDS / _DAILY_CAP / SILENCE_THRESHOLD_SECONDS`）。（提取噪声已剔除：CALLS/TOKEN 是字符串片段、`MEDIA_*_UNSET_XYZ` 是测试用不存在变量名。）
- **.env.example 有但 config.rs 不读**：仅 `APP_ENV`——它由 `src/db/migrations/mod.rs:583` 读取（破坏性迁移 fail-closed 守卫），不是漂移。
- 注：`candidate_smoke.py` 的 ISOLATION_OVERRIDES 恰好用到了 SILENCE_SIGNAL_WORKER_ENABLED——运维脚本知道这些未文档化变量的存在。

### 5.4 其它高频事实

- 生产 app 端口 **3003**（deploy 工具/diag/_lib/rotate 的 health URL 一致），`.env.example` 默认 8080，`deploy.sh` 用 8080（过时）。
- MCP 网关地址两个版本：`.env.example:19` = `http://47.108.57.147:3001`；`deploy.sh:46`/`test_mcp_account_alias.mjs` = `http://117.72.54.28:3001`（推断：117 是现役 server，47.108 为更早/另一环境示例）。
- `_remote_run.py` 的 `DEPLOY_PORT` 默认 **3003 不是 22**——任何直接使用它的新脚本必须显式设 22（_lib 已强制）。
- 真模型 CI 供应商现状：主链 RSXERMU_KEY→gateway.oeezzk.cn（gpt-5.6-auto / codex-auto-review），第三族 NVIDIA_KEY→integrate.api.nvidia.com；临时 backfill 线 MAYCRAN_KEY→api.maycran.com（claude-sonnet-4-6）。绑定表由 check-secrets.py 硬锁（仅对 RSXERMU_KEY）。
- outbox status 闭集 5 值（pending/in_flight/sent/failed_terminal/canceled）；`held_by_ai_policy/blocked_by_safety_guard` 等是 run/final_review 侧的值不入 outbox（`_lib.latest_outbox` docstring，与 evaluation VALID_TERMINALS 一致）。

---

## 6. 偏差与疑点

1. **【高危·提交阻断】未提交改动将命中 no-human-takeover lint**。亲验：`git diff` 新增行中 `src/routes/knowledge/import.rs`（"…供运营**人工**核验…"，LONG_IMPORT_PROMPT_TEMPLATE 文案）与 `src/routes/knowledge/mod.rs`（`format!("原文含需**人工**核验的绝对承诺：{line}")`，生产代码 + 同文件单测断言串）含"人工"，两文件均在 lint 扫描目录 `src/routes/` 且不匹配任何排除模式（排除仅 `*/tests/*` 路径级，不识别 `#[cfg(test)]`）。`FORBIDDEN_PATTERN` 含独立词"人工"（check-no-human-takeover.sh:35）。**这批改动进 PR 时 baseline job 必红**，需先改措辞（如"运营核验/需复核"）。
2. **domain8 的 BLOCKED severity 用法与新 expect 语义冲突**。`batch_a_domain8.py` 对"前序回复未 sent"场景调 `_lib.expect(eligible, .., "BLOCKED", ..)`；新 `expect` 只豁免 `low`，"BLOCKED" 会 `raise BizTestAssertionError` → 其后 `if not eligible: return` 是死代码，且该场景本应走 `record_blocked`（jsonl 台账）而非 findings 表 + 抛异常。语义上环境性受阻被计成断言失败。推断为改造遗漏。
3. **文件数/行数与任务描述不符**：实际 47 文件 +3285/−1428 vs 任务所述 45 文件 +2986/−1394。推断任务基于更早快照，其后（8-12 至 8-13 凌晨）biz-test 仍在继续修改（文件 mtime 为证）。
4. **`batch_c_management.py` 二次 confirm 断言语义反转**（`already_processed_or_not_found` → 幂等返回 `canceled`）依赖服务端行为，HEAD 的 management.rs 是否已实现"reject 后 confirm 幂等回显 canceled"未逐行核验（脚本与其它冻结绑定同批对齐 HEAD 契约，推断已实现；若未实现该断言会在真跑时暴露）。
5. **`performance_report.py` 与 `gateway_performance_report.py` 近乎完全重复**（同端点/同参数/同输出，仅格式差异）——维护冗余，推断一个是另一个的整理版而旧版未删。
6. **`deploy.sh` 已过时**：固定 2026-07-07 的分支名、8080 端口、117 的 MCP 地址与"合并到 main"交互流程，与现状（3003 端口、CI 驱动合并、candidate_smoke/rotate 等新部署工具链）不符。作为"事实源"应以 `scripts/deploy/` 新工具为准（推断）。
7. **`rt_send.py`/`rt-send.sh` 硬编码真实生产 appId 与真人 wxid（fengrui86）且无 HMAC 头**——`WEBHOOK_VERIFY_SIGNATURE=true` 的环境必被拒；该对脚本仅适用于本地关签名联调，且硬编码真人 wxid 有向真实联系人注入测试消息的误用风险（需 export 关签名 + 本地库才生效，风险受限）。
8. **biz-test 旧 cleanup 从未成功清理过 direct_contact_collections（推断）**：旧 f-string 产出 `]}}` 括号不平衡的 mongosh 一行脚本，按 `mongo()` fail-closed 语义整段抛错。新测试 `test_contact_root_delete_filters_are_balanced` 反向锁死。若推断成立，历史 biztest 残留可能仍在生产库（新 cleanup 首跑会补清）。
9. **evaluation 的 `assert_llm_success` 仍是时间窗版**（`batch_c_evaluation.py` 用 elapsed 窗口查 `user.reply.fast.task`）——与其它域的 run 级绑定纪律不一致；模拟器多场景多 run 无单一 run_id 可绑，属可解释的折衷（推断）。
10. **`check-no-human-takeover` 的 `Suspicious 单词"人工"极宽**（会拦"人工核验/人工审核"等正常运营用语）——这是产品红线的既定设计（CLAUDE.md 同款词表），非缺陷，但意味着 src 侧任何面向运营的中文文案都必须避开该词（疑点 1 即实例）。
11. **`batch_a_domain1011.py` 用 `"".join(("人","工","接","管"))` 运行时拼接禁词**——scripts/ 不在 lint 扫描目录本无需规避（推断作者为双保险或防未来扩目录）；侧证作者对 lint 词表的敏感度，反衬疑点 1 更可能是疏忽。
12. `.env.example` 未文档化 POST_DECISION/SILENCE_SIGNAL 两族共 10 个 config.rs 变量（见 §5.3）——digital_twin 验收（wait_projection_terminal）恰依赖 post-decision worker，运维排障时缺文档入口（低危）。

---

## 7. 覆盖自证

**A. 未提交改动**：47/47 个 M 文件全部 `git diff -- <file>` 逐 hunk 读完（src 14、tests 9、scripts/biz-test 24；大 diff 分段补读被截断部分：_lib.py 中段以现行全文 1049 行补全、batch_a_domain6 头段、batch_a_domain9 尾段、batch_c_campaign 中段均二次读取）。关联上下文亲验：`src/routes/shared.rs:438`（read_operating_memory）、`src/prompts.rs:760`（default_user_operation_state_machine）、`src/routes/prompt_templates.rs:47/118/162/267`（字面双闸）、`src/routes/campaigns.rs:152/177/337-372`（spec_hash）、`src/routes/management.rs:63/98/212`（plan_hash/execution_unknown）、`src/routes/guides.rs:39/77/604+`（candidate_hash）、`src/db/migrations/mod.rs:583`（APP_ENV）、explicitStopRequested 写点归属（仅 src/agent/reaction.rs）。`cargo check` exit 0。禁词命中用 awk+grep 复刻 lint 逻辑实测。

**B. scripts 与 CI 全集**（含读取深度）：
- scripts/ 顶层 40 文件：check-baseline.sh 全文、check-baseline.ps1 头 40、check-no-human-takeover.sh 全文、.ps1 头 40、check-no-model-hint.sh 全文、check-evolution-isolation.sh 全文（.ps1 与 sh 等价、未单独展开——推断自其文件头声明）、test-ci-lints.sh 全文、check-ci-gate-policy.py 全文、check-delivery-boundary.py 全文、check-secrets.py 全文、check-skip-ledger.sh 全文、check-capability-outcomes.py 全文、check-task-status-manifest.py 全文、check-audit-status-manifest.py 前 120+后 140（中段 validate 函数族按结构推断）、smoke_knowledge_full_loop.py 全文、smoke_knowledge_no_llm.py 头 60、smoke_reimport_docs.py 头 50、smoke_reimport_split.py 头 50、smoke_repair_rejected.py 头 50、rt_send.py 全文、rt-send.sh 全文、gateway_performance_report.py 全文、performance_report.py 全文、_remote_run.py 全文、_remote_put.py 全文、_push_bundle.py 全文、analyze.py 全文、count.py 全文、list_fns.py 全文、split_routes.py 头 20、backfill_knowledge_tags.py 头 40、cleanup_non_human_managed.js 全文、fix_sediment_titles.js 头 20、stay-awake.ps1 全文。
- scripts/biz-test/ 30 文件：改动的 24 个逐 hunk diff + _lib.py 现行全文；未改动 5 个 probe（_probe2/3/4/_probe_split/probe_domain4_clean）读 docstring+头部；test_* 4 个随 diff 全读。
- scripts/e2e/ 41 个非图片文件：全部读头部（12-30 行）提炼用途，代表性脚本（smoke/b3_walk/b3_userops_walk/batch1_monitor/b4_realsend/seed_and_webhook/deep_knowledge_loop/deep_guide/wiki_import_e2e）读到 25-30 行；png/json 产物按名归档不读。
- scripts/diag/ 11 文件：audit/clean/compare/drill/llm_providers/probe_mcp_ready 读 40-50 行，probe 族 5 个读 docstring+头 10。
- scripts/deploy/ 12 文件：candidate_smoke/rotate_llm_credential 头 80、audit_hc001 头 50、delete_hc001 头 40、sync_github_secret 头 60、ssh_put/ssh_run 头 40；5 个 test_ 按名与被测对象对应（未逐行）。
- .github/workflows/ 5 文件：ci.yml 1577 行全文（分 4 段）、nightly-dynamic.yml 全文、maycran-fix-validation.yml 全文、maycran-llm-backfill.yml 头 160（矩阵/env/步骤主体）、maycran-model-probe.yml 头 80。
- 根：deploy.sh 全文、.env.example 全文（284 行）、Cargo.toml 全文；.env.example×config.rs 差集用 rg+comm 实测。

**局限声明**：`#[ignore]` 集成测试与 biz-test 脚本未实际运行（需 Docker/生产 server/DEPLOY_PASS）；对"旧 cleanup 从未成功执行""management 二次 confirm 幂等已在 HEAD 实现"等 4 处结论标注了推断；check-audit-status-manifest.py 中段与 maycran 两个 temporary workflow 尾段为结构化略读。
