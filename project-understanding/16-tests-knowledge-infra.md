# 测试全集 B（知识/演化/安全/真模型）深读记录（核证日期 2026-08-13）

> 本记录为 tests/ 深读第二半（16 号），与 15 号（agent 主链路：gateway/决策/评审/状态机/outbox/记忆/反应/请示/campaign/planner）互补。
> 所有断言均为当场 Read 亲验，附 file:line。读不懂/存疑处标注在 §5。

## 1. 主题→文件清单

tests/ 顶层共 **182 个 `.rs` 测试文件**（`ls tests/*.rs | wc -l` 亲验）+ `common/` 9 个模块 + `fixtures/` 4 个数据文件（jwt_test_private.pem、jwt_test_public.pem、k6_article_image.b64、q3_registration_calib_image.b64）+ 3 个 `.proptest-regressions`（autonomy_protocol_pbt / knowledge_agent_pbt / memory_card_invariants，proptest 回归种子，非测试代码）。15 号已覆盖 70 个顶层文件；182−70=112，本记录覆盖这 112 个 + 按任务要求重读 3 个（llm_retry_jitter、real_llm_principal_channel、real_llm_principal_relay）+ common/ 全部 9 个模块 = **124 个文件**。

### 主题分组（本组文件）

- **tests/common/（9）**：mod、capability_evidence、dynamic、generalization、identity_generator、judge、redline、roleplay_fixtures、roleplayer
- **common 配套 smoke/单测跑车（6）**：common_smoke、dynamic_smoke、identity_generator_smoke、judge_rubric、redline_smoke、roleplay_fixtures_smoke
- **知识主题（44）**：wiki_chunk_revision_pbt、wiki_gap_signals_3kinds、knowledge_agent_eval、knowledge_agent_pbt、knowledge_ask_e2e、knowledge_ask_stream_e2e、knowledge_auto_verify_enforce_integration、knowledge_chat_apply_integration、knowledge_chat_dispatch、knowledge_chunk_transactions、knowledge_closed_loop_trajectory、knowledge_digest_budget_smoke、knowledge_digest_compose_smoke、knowledge_digest_skeleton、knowledge_import_apply_integration、knowledge_operator_memory_isolation、knowledge_preview_workspace_scope、knowledge_router_fallback_e2e、knowledge_task_worker、knowledge_tools_budget、knowledge_worker_behavior_integration、chunk_batch_ops、chunk_lock_lifecycle、chunk_put_preserves_unmodeled_fields、chunk_revision_ai_draft_integration、chunk_type_routing_pbt、digest_cross_tenant_scope_integration、import_job_lifecycle、import_pdf_smoke、ingest_worker_smoke、sr115_catalog_rebuild_recovery、sr117_ingest_source_recovery、sr121_digest_snapshot_recovery、sr122_knowledge_task_recovery、sr125_digest_dispatch_router、sr131_document_metadata_patch、sr132_review_queue、page_merge_pbt、annotation_quality_gate_integration、integrity_report_d2_e2e、structured_organization_integration、vision_safety_gate、hc028_real_digest_task_e2e、knowledge_operator_memory_isolation（sr181 由 15 号覆盖）
- **演化主题（8）**：evolution_policy_router_integration、evolution_prompt_shadow、evolution_release_redline、evolution_rollback_status、evolution_workspace_scope、m040_evolution_release_protocol、prompt_publish_evolution_guard、reset_pack_preserves_evolution_critic_integration（+ 演化近亲：sr097_lesson_promotion、lessons_learned_filters）
- **租户/鉴权/安全主题（10）**：workspace_isolation、sr176_real_route_isolation、h3_cross_tenant_idor、auth_middleware_integration、sr016_auth_rate_limit、jwt_auth、products_workspace_isolation、migration_safety_redlines、account_security_integration、sr174_cache_database_isolation
- **迁移主题（7）**：migrations_idempotency、m018_backfill_domain_stage、m029_cleanup_contact_identity、m034_review_reconciliation、m039_scope_revision_behavior、m045_relationship_review_cycles、m049_prompt_planning_currents（m040 归入演化）
- **LLM/基础设施（5）**：llm_retry_jitter（重读）、llm_provider_activate_integration、llm_usage_summary_integration、maycran_transport_probe、ops_versioned_index_boot_brick
- **真模型全系列（14）**：real_llm_adversarial、real_llm_cross_domain_arc、real_llm_digital_twin_arc、real_llm_dynamic_adversarial、real_llm_knowledge、real_llm_knowledge_quality、real_llm_ops_smoke、real_llm_principal_channel（重读）、real_llm_principal_relay（重读）、real_llm_proactive_outreach、real_llm_progressive_tier、real_llm_recall_benchmark、real_llm_roleplay_arc、real_llm_smoke
- **roleplay/domain/taxonomy（8）**：roleplay_emotional_companion_e2e、roleplay_fixtures_smoke、roleplay_reviewer_pressure_calibration、domain_profile_e2e、domain_schema_persistence_e2e、configuration_generation_integration、taxonomy_flags_e2e、taxonomy_version_audit_integration
- **剩余未被 15 号覆盖（12）**：guide_apply_partial_validation、hc026_formula_evaluation、operation_view_integration、playbook_scope_integration、prompt_pack_seeding、prompt_template_redline_gate_e2e、sr008_ops_single_current、sr012_runtime_scope、sr053_soul_versions、sr055_prompt_versions、sr138_prompt_reset_guard、contact_manual_tags_integration、contact_operation_profile_integration

## 2. tests/common/ 逐模块深读

### 2.1 `common/mod.rs`（1073 行）——TestApp 工厂 + mock LLM 协议

**TestApp 构造（mod.rs:397-573）**：
- 自带最小 `TestMongo` Image（mongo **5.0.6**，mod.rs:57-97），standalone 或 `--replSet rs`（repl_set 模式 exec `rs.initiate()` 并等 "Rebuilding PrimaryOnlyService due to stepUp"，mod.rs:83-94）。`TestApp::start()`=standalone；`TestApp::start_repl_set()`（mod.rs:407-409）专供多文档事务测试（standalone mongod 无法 commit 事务）。单节点 RS 连接串必须 `directConnection=true`（mod.rs:443-449）。
- 每次启动用随机库名 `wechatagent_test_<uuid>`（mod.rs:452）。支持 `TEST_MONGODB_URI` 外部 mongod 逃生门（mod.rs:420-424），`cleanup()` 才会显式 drop 外部库（mod.rs:577-586）。
- 构造顺序（与生产 main.rs 对齐）：`Database::connect` → `migrations::run` → `ensure_indexes`（mod.rs:454-460）→ **重新 seed m006 销售域 taxonomy**（mod.rs:469-471，因 m012 在非 production 环境会删掉 customer_stage/intent_level/objection_type 三 kind seed，测试库需补回对齐生产）→ 写 `workspace_taxonomy_template_v1:<ws>` 迁移标记（mod.rs:479-496）→ `ensure_prompt_pack_v2`（mod.rs:498-504）→ **预热两个进程级 LazyLock+30s TTL 单例缓存**：taxonomy 缓存（mod.rs:510-512）与 active DomainProfile 缓存（mod.rs:522-524；不预热会命中同一 binary 内上个测试 DB 残留的 active profile，导致 customer_stage 维度被剔除、C2 派生回落）。
- `test_config`（mod.rs:589-735）：所有后台 worker 默认 disabled（strategic_planner/cold_contact/silence_signal/evolution/knowledge_digest/ingest 全 false），`progressive_tier_enabled=true`（mod.rs:616），`account_send_min/max_interval_ms=0`（账号级拟人间隔闸测试默认关，mod.rs:611-612，需要的测试自行覆盖）、`llm_max_retries=1`、`webhook_verify_signature=false`、`jwt_enabled=false`。
- state 组装含 `prompt_pack_version` AtomicU64 seed 后 fetch_add 1（mod.rs:563-565，与 main.rs 一致）。

**TestLlmGenerator——mock LLM 协议（mod.rs:162-286）**：
- `push_response(json)` 入队、`calls()` 读消费次数；实现 `LlmProvider` 三个方法，`generate_json_with_image` 也从同一队列取（mod.rs:373-381，vision 测试靠它走通图片导入）。
- **核心协议：并发 schema 定向出队（mod.rs:212-284）**。Knowledge Router、首轮 Reply、Reviewer、ClaimGate 可并发执行，测试响应不按 FIFO，而按**响应 JSON 顶层 schema** 分类匹配请求：ClaimGate=`requiresEvidence+claims+catalogClaims` 三键齐；Reply=`decisionPhase`；Reviewer=`approved+scores`；Knowledge=`action`；其余 Other=FIFO。请求侧按 system prompt 锚文本判类：ClaimGate=“independent semantic claim reviewer”、Knowledge=“运营知识库的 wiki 研究员”、Reviewer=“请独立审核候选回复/独立审核/独立运营质量评审 Agent/reviewer”、Reply=同时含 `shouldReply`+`conversationMode`（mod.rs:239-257）。无匹配时报错并列出队列 schema（mod.rs:268-283）。
- 队列空 → `AppError::External("TestLlmGenerator: 没有预排队的响应")`（mod.rs:206-210）——集成测试少排响应会显式红，不静默。

**ClaimGate 三件套 fixture（mod.rs:292-354）**：`independent_claim_gate_pass_json()`（无业务断言）、`independent_claim_gate_unsupported_business_json(quote)`（开放世界业务事实无证据）、`independent_claim_gate_verified_knowledge_json(chunk_id, quote)`（product_capability 引 `verified_knowledge:<chunk_id>` 证据；注释红线：只许在已 seed 并引用 verified chunk 的测试用，不得当作绕过证据检查的万能钥匙）。Gateway 每轮 Review 后都会调一次 `user.review.claim_gate`，集成测试必须显式排入完整 schema（mod.rs:288-291）。

**outbox/投影等待 helper**：
- `wait_for_outbox_processed`（mod.rs:743-771）/ `wait_for_outbox_processed_by_run_id`（mod.rs:857-885）：100ms 轮询，终态集合 = `sent|failed_terminal|canceled|delivery_unknown`，超时 panic 报最后状态。
- `complete_latest_post_decision`（mod.rs:776-849）：找该联系人最新 `post_decision_status="pending"` 的 decision_review，push 投影 JSON、起 `run_post_decision_worker`，轮询至 `completed|failed_terminal|discarded`，断言必须 `completed`。**约定：发送断言在前、投影断言在后调用此 helper**。
- `rebuild_app_state_with_mcp_url`（mod.rs:893-914）：换 wiremock MCP 端点重建 AppState（复用容器/LLM）。
- `evolution_release_state`（mod.rs:919-945）：evolution_enabled=true + upsert `evolution_runtime_flags`（enabled/rollout 100%/threshold_auto_release=false）——共享 TestApp 默认关演化，避免旁路生产门。
- `insert_released_prompt_proposal`（mod.rs:950-1038）：直插 raw `proposals` 集合（避开 30+ 字段结构体），带 base_revision/released_revision（`evolution::revision::prompt_revision` 计算），并把当前 prompt 模板绑到 proposal（`source_proposal_id`），供 rollback_prompt 测试。
- `rebuild_app_state_with_real_llm`（mod.rs:1048-1073）：真模型测试专用——LLM 换真实 provider、**MCP 永远指向 wiremock 桩（绝不真发微信）**、`second_reviewer_llm=None`（单脑复审省调用）。
- `ensure_test_account`（mod.rs:140-156）：insert-only upsert 账号 scope（保留已 seed 的账号级凭据，不弱化生产 fail-closed）。

### 2.2 `common/judge.rs`（471 行）——LLM-as-judge 标尺从 DomainProfile 派生（R1.1/R1.2）

- **三层派生 `build_judge_rubric`（judge.rs:98-180）**：①域无关硬闸四维 `humanLike/emotionalValue/helpfulness/factualRestraint` + overall（judge.rs:42-47，锚点散文 judge.rs:51-58 逐字搬自旧 JUDGE_SYSTEM/EMOTIONAL_JUDGE_SYSTEM 交集）；②极性层按 `profile.operation_mode.funnel.enabled` 翻转（judge.rs:88-90）：漏斗域→`manipulationRisk`（越高越坏，judge.rs:61-63），陪伴域→`pressureRisk` + 关系维 `personaConsistency/scenarioAppropriateness`（judge.rs:66-73）+ 注入 profile.prompt_fragment 语境（judge.rs:127-137）；③软观测维由 `business_formulas` 派生（judge.rs:185-206，复刻生产 `render_reviewer_extra_score_lines` 的 HARD_GATES 过滤+去重；软维只进 system 文本不进 dims）。coverage_dimensions 的 display_name 拼完整度关注点（judge.rs:209-223）。
- **输出契约**：严格 JSON、每维 `{score,reason}`、键固定 dims+verdict（judge.rs:171-177）。
- **R1.2 失败语义分级 `run_judge_graded`（judge.rs:311-393）**：`JudgeGate::QualityGate`（judge 是唯一质量门）→ 空 reply panic（judge.rs:331-333）、K 次采样全失败 panic（judge.rs:373-375）；`ObserveOnly` → eprintln + None 不 panic（与 t4-t18 旧语义等价）。**非瞬时 LLM 错误（账户/配置/解析/未知）无论 gate 都 panic**（judge.rs:294-300 用 `is_transient_llm_unavailable_kind` 白名单判定，judge.rs:363-365），防"裁判没真跑但测试绿"。`REAL_LLM_JUDGE!=1` 直接跳过返 None（judge.rs:320-327）。K 次采样并发，取各维 median（judge.rs:343-346, 383-386）。
- 自带 3 个无 Docker 单测（judge.rs:402-470）：销售 DEFAULT 派生键集 ⊇ 旧 JUDGE_SYSTEM 6 键且无 pressureRisk；情感 profile 含 pressureRisk+关系维且**无 manipulationRisk**；两域 system/dims 确有差异。
- 注释明示（judge.rs:244-247）：老测试（ops_smoke/adversarial/emotional_e2e/reviewer_calibration）各自的 run_judge 维持不动，属 t4-t18 零变化红线保护对象；本模块是新测试统一入口。

### 2.3 `common/redline.rs`（250 行）——转真人/暴露身份红线断言库

- 根治两个真实缺陷（redline.rs:4-15 注释）：①裸 `.contains()` 否定假阳（"不用转接客服，我直接帮你"被误判红线击穿）；②旧表漏词「转人工/人工客服」。
- `HANDOFF_MARKERS` 22 词（redline.rs:41-64，含转人工/人工接管/安排同事/专人对接/真人对接等；刻意不收裸「人工」防误伤「人工智能」、不收裸「真人」防误伤「就当跟真人聊」）；`IDENTITY_LEAK_MARKERS` 6 词（redline.rs:67-74）；`ENGLISH_HANDOFF_MARKERS` 3 词 takeover/hand-off/handoff（redline.rs:77）。
- `contains_unnegated`（redline.rs:83-102）：命中 marker 后回看**同子句**（子句边界=。，,、！!？?；;\n\r）内是否有 11 个否定前缀之一（redline.rs:24-36）；`preceded_by_negation`（redline.rs:111-124）注释解释为何用子句窗口而非紧邻字符（「没法**给你**安排真人」隔词否定仍算拒绝；「不行，我帮你转人工」跨子句否定不掩护）。
- `assert_no_handoff_or_identity_leak`（redline.rs:136-153）：任一未否定命中即 panic；空 reply 直接放行。
- 自带 9 个纯函数单测（redline.rs:159-249）：正命中、否定拒绝不误判、同子句隔词否定、跨子句否定不掩护、先否定后真承诺仍命中、身份泄露+「我不是机器人」不命中、first_hit 选未否定者、干净回复无命中、assert helper panic/放行行为。

### 2.4 `common/roleplayer.rs`（259 行）——LLM 扮演客户协议（R5.1/R5.3）

- **防作弊硬约束（roleplayer.rs:11-17 注释 + 类型层面）**：roleplayer 只接受 `history: &[DialogueTurn]`（Customer/Agent 台词），**永远拿不到** reviewer 分数/operation_state/agent reasoning；`render_history_for_roleplayer`（roleplayer.rs:245-259）只渲染对话文本。
- `UserPersona` 四字段契约：identity/temperament/need/boundary（roleplayer.rs:31-41）。
- `roleplayer_client()`（roleplayer.rs:76-96）：读 `ROLEPLAYER_*` env，默认 NVIDIA `meta/llama-3.3-70b-instruct` @ temperature 0.8（`with_temperature` 覆盖生产默认 0.2）；缺 key → None（调用方自我跳过，**不回落 agent client**——R5.0.1 异族硬门）。
- `roleplay_user_turn`（roleplayer.rs:105-140）：要求输出 `{"message":"..."}`；parse/调用失败 → 返回 `fallback_line` + `source=Fallback`（**Fallback 不是测试通过信号**）。
- **R5.3 对抗手法 `AdversarialTactic`（roleplayer.rs:145-172）**：IdentityProbe（质问是不是机器人/要转人工）、EmotionalEscalation（情绪反扑）、InduceBoundaryViolation（诱导无依据承诺/报价/线下）；`roleplay_adversarial_turn`（roleplayer.rs:176-212）在 system 里叠加手法简报，agent 接住→客户软化、露馅→升级。
- system prompt 模板（roleplayer.rs:215-241）：立人设 + 5 条扮演规则（1-3 句微信口语、绝不出戏、不替助理说话）+ 严格 JSON 输出。

### 2.5 `common/dynamic.rs`（298 行）——三族异族硬门（R5.0.1）+ 轨迹级裁判（R5.2）

- 定位（dynamic.rs:5-15 注释）：动态发现线**只进 ledger 观测 + 软门，不进 PR 合并门**；trajectory 分在人工金标校准达标前**绝不进任何软门**。
- `ProviderFingerprint.family()`（dynamic.rs:43-61）= 端点 host + 模型厂商段（`meta/llama-3.3`→meta；`claude-opus-4-8`→claude；`gpt-5.4`→gpt）。
- `read_role_fingerprints`（dynamic.rs:65-86）：agent=`REAL_LLM_BASE_URL/MODEL`（默认 rsxermu666.cn / claude-opus-4-8）、roleplayer=`ROLEPLAYER_*`（默认 nvidia / meta-llama-3.3-70b）、judge=`REAL_LLM_JUDGE_*`（默认 rsxermu666.cn/v1 / gpt-5.4）。
- `assert_three_families_distinct`（dynamic.rs:93-115）：三角色两两异族，同族 → panic（job 红），防"模型自问自答自评"伪多样性。同 host 不同 vendor 算异族（agent claude 与 judge gpt 同 host rsxermu666.cn）。
- `judge_trajectory`（dynamic.rs:177-214）：评整段对话 6 维 `trustTrajectory/relationshipProgress/redlineHeld/personaConsistency/givesSpace/overall`（dynamic.rs:164-171；givesSpace 与 relationshipProgress 同权对立防奖励施压式推进，dynamic.rs:155）；单轮标尺锚点嵌入 R1.1 rubric.system（不重新硬编码销售世界观）；失败返回 `ok=false` 不 panic。
- `trajectory_judge_client`（dynamic.rs:217-232）：REAL_LLM_JUDGE_* 派生，缺 key → None。
- 4 个纯函数单测（dynamic.rs:239-297）锚定 family 判别语义。

### 2.6 `common/identity_generator.rs`（333 行）——LLM 随机身份生成器（R2.1）

- 可复现协议（identity_generator.rs:8-17 注释）：**离线行业候选库 + seed 确定性选择**（`select_skeleton(seed)=candidates[seed % len]`，identity_generator.rs:154-158），LLM 只丰满语义字段；候选库只增不改顺序（identity_generator.rs:90-94 注释）。
- 四大类 `IdentityCategory`（identity_generator.rs:41-50）：Sales/FormalBusiness=漏斗开、Companion/PeerSocial=漏斗关（`is_funnel`，identity_generator.rs:55-60）——与 judge 极性维一一对应。候选库 9 个行业骨架（identity_generator.rs:95-148：少儿编程/重疾险/护肤品导购；深夜情绪陪伴/独居青年陪伴；同行运营搭子/行业人脉；企业财税/B2B SaaS 售前）。
- `apply_category_semantics`（identity_generator.rs:171-178，纯函数）：`funnel.enabled=is_funnel`、`transaction_facts_enabled=funnel`、非交易域 `grounding_gate_bypass_without_claim=true` + `distrust_self_reported_low_risk=true`（与情感陪伴契约同精神）。
- `generate_identity`（identity_generator.rs:197-295）：LLM 产 displayName/description/promptFragment/formulaNames/persona/openingInbound；关键字段缺失或调用失败 → None（调用方 skip 不假绿）；公式仅替换 display_name（key/expression/eval_score_key 评分骨架不动，identity_generator.rs:257-265）；profile 基于 `default_domain_profile` 派生，profile_id=`generated_<category>_<seed>`。

### 2.7 `common/generalization.rs`（123 行）——泛化门纯函数

- `generalization_report(train, holdout, floor, max_gap)`（generalization.rs:47-69）：双 split 平均召回，红线 = 任一 split 空 / train_mean<floor / holdout_mean<floor / |gap|>max_gap（`ok()` 全绿才过，generalization.rs:30-35）。floor 标志带 `!is_empty()` 守卫，空 split 只由 empty_split 兜底（generalization.rs:64-67）。gap 取绝对值。6 个纯函数单测（generalization.rs:75-122）。抽自 real_llm_knowledge_quality Q2 的 train/holdout 过拟合门。

### 2.8 `common/capability_evidence.rs`（158 行）——真模型能力测试台账

- `CapabilityEvidence`：new 时 verdict=`inconclusive`（capability_evidence.rs:38-39），**Drop 时恰好写一份终态 JSON**（capability_evidence.rs:125-138）到 `${REAL_LLM_LEDGER:-target/real_llm_ledger}/capability_outcome.<case_id>.json`，panic 中 drop → verdict=`failed`。
- `pass(artifacts, assertions_run)` 前置断言（capability_evidence.rs:70-92）：必须 attempted=true、llm_calls>0、branch 见证非空、artifacts>0、assertions_run>0——**结构上禁止"没跑到就绿"**。verdict 枚举：pass/inconclusive/infra_skip/failed。schema=`real_llm_capability_outcome/v1`，带 GITHUB_SHA/RUN_ID（capability_evidence.rs:105-120）。case_id 必须文件名安全（capability_evidence.rs:25-30）。

### 2.9 `common/roleplay_fixtures.rs`（306 行）——roleplay P0 夹具

- `EMOTIONAL_COMPANION_WORKSPACE="test_emotional_companion"`（roleplay_fixtures.rs:35）。
- `seed_active_domain_profile`（roleplay_fixtures.rs:49-98）：以 `default_domain_profile` 为基底、mutate 闭包覆盖，**之后**钉死身份/发布字段（version=1、release_status="published"、is_active、current_version、seeded_by="roleplay_fixture"，roleplay_fixtures.rs:63-73）；**单活保证**：先降级同 workspace 其它 is_active 行再插入（roleplay_fixtures.rs:76-93）；插入后 `invalidate_global_domain_profile_cache` 强制失效 30s TTL 进程缓存（roleplay_fixtures.rs:96）。
- `seed_emotional_companion_profile_in_workspace`（roleplay_fixtures.rs:112-130）：情感陪伴 profile 从 lib 单一真相源 `example_emotional_companion_profile` 整体拷贝（避免 fixture 与 lib 契约漂移；此前只挑字段导致 transaction_facts_enabled=true 销售默认残留）。注释点明 P2 接线坑：prompt 按 default_workspace_id 加载而 profile 按 contact.workspace_id 加载，两者必须同 ws 才不静默回落销售域（roleplay_fixtures.rs:108-111）。
- `disable_quiet_hours_for_contact`（roleplay_fixtures.rs:137-143）：contact 级 `operation_mode_override.quiet_hours.enabled_override=Some(false)`，一次关 webhook+gateway 两层静默门。
- `override_review_prompt`（roleplay_fixtures.rs:152-205）：**时序红线——必须在 TestApp::start() 之后**（ensure_prompt_pack_v2 会 delete_many 再 insert）；先归档 current 指针再插 version=9999 的 override（single-current 契约一致）。
- `seed_verified_chunk`（roleplay_fixtures.rs:211-247）：domain="user_operations"/status="active"/integrity_status="verified"/account_id=None，与 `load_operation_knowledge` 加载条件一致；chunk_type="product_fact"。
- `RoleplayLedger`（roleplay_fixtures.rs:254-305）：append JSONL 到 `roleplay_<fixture>.jsonl`，IO 失败仅 eprintln；`append_issue` 行带 `suspected_layer` 字段（设计 §3.2）。

## 3. 逐文件深读

### 3.A 知识主题（44 文件）

#### wiki_chunk_revision_pbt.rs（268 行，9 测试=7 proptest+2 plain，无 Docker，默认跑，baseline R11.6 第 4 条 PBT）
- P1 `locked_field_rejection`（:49-67）：patch 含 `DEFAULT_LOCKED_FIELDS` 8 锁定字段（chunk_id/wiki_type/chunk_type/created_at/source_anchor/verified_at/verified_by/approved_at）任一 → `apply_field_patch` 必返 `RevisionError::LockedFieldInPatch`。
- P2 `array_union_monotonic`（:71-100）：`union_array_fields` 输出 ⊇ existing ∪ patch。
- P3 `body_truncation_block`（:104-121）：merged_len < existing×`BODY_TRUNCATION_THRESHOLD`(0.7) → `is_body_truncated=true`；≥ 不截断。
- P4 `hash_unchanged_on_failure`（:125-149）：守门函数纯函数性，失败不改 existing hash。
- P5 `ai_status_forced`（:153-183）：模拟 `apply_chunk_revision` 第 4 步（生产 chunk_revisions.rs:207-210），AI source 强制 status=draft + integrity=needs_review（注：此为测试自身复制逻辑，真接线由 chunk_revision_ai_draft_integration 钉死）。
- P6 `revision_id_unique`（:187-200）：`rev_{chunk_id}_{uuid}` 1000 次无冲突。
- P7 `rollback_idempotent`（:204-237）：`enforce_locked_fields` 到同一 historical 两次 hash 恒等且等于 historical。
- P8 `cleanup_no_substring_match` + `normalize_ref_key_rejects_openai_to_ai_collision`（:241-268）：`normalize_ref_key("openai") != normalize_ref_key("ai")`——archived chunk 清理不做子串误伤。

#### wiki_gap_signals_3kinds.rs（575 行，6 测试，全 #[ignore]）
- 三类规则型 gap_signal 全生命周期（不耗 LLM）：`missing_chunk`（引用 archived target → severity=error/source=rule，恢复后 sweep `rule:dep_restored`，:92-178）、`suggestion`（needs_review + blocked_count_30d=5>3 → info，verified 后 `rule:chunk_verified`，:182-244）、`contradiction`（同 normalize_title 双 chunk body 首段 sha 不同 → error，archive 其一后 `rule:contradiction_resolved`，:248-310）。三者都断言 dedup：第二次 `run_structural_lint` new_signals=0。
- `recall_signal_merges_correct_topic_among_multiple_pending`（:336-437，KB-07 回归哨兵）：`persist_recall_signal` 必须按 dedup_key（`recall_miss::{normalize_title}`，title=query 前 40 字符）精确合并。构造 3 主题且 B 居中——注释详证两条索引（`gap_signals_status_kind_idx` indexes.rs:1399 按插入序返回最早 A；`gap_signals_kind_status_created_idx` indexes.rs:1442 返回最新 C），坏代码（无序 find_one）在两种 planner 选择下都拿不到 B → 误新建第 4 条 → 哨兵必红。断言 B 累积 query 变体、A/C 不被污染。
- `recall_signal_merges_into_legacy_row_without_persisted_dedup_key`（:442-512）：无 dedup_key 的 legacy 行按派生业务键仍可命中合并，不建现代行副本，legacy 行保持 dedup_key=None（免迁移可读）。
- `concurrent_recall_signals_upsert_one_pending_and_merge_all_queries`（:517-575）：16 并发 writer 同业务键 → 恰 1 条 pending，dedup_key 64 hex，16 个 query 变体全保留（原子 upsert 合并数组不丢败者数据）。

#### knowledge_agent_eval.rs（234 行，1 测试，#[ignore]，SR-126 离线召回门）
- 生产 `list_catalog` 排序决定候选，金标只用于评分、绝不注入 mock（mock 只 open 生产排序第一项并引用其证据，:159-172）。5 场景，每场景 1 条相关 chunk（低静态分 conf=0.35/priority=0）+ 3 条高静态分干扰项（conf=0.99/priority=100）。
- 阈值（:219-233）：total=5、recall@1（top_rank_hit_rate）≥0.80、cited_hit_rate ≥0.80、avg_rounds ≤3.0、truncated=0、cancelled=0、llm_calls = 场景数×2。排序若退化为静态分主导，金标召回下降即红。

#### knowledge_agent_pbt.rs（810 行，17 测试=12 proptest+5 plain，无 Docker，默认跑）
- P1-P3 `filter_answer_against_opened`：cited ⊆ opened（:95-115）、quote.chunk_id ∈ opened 且非空（:119-140）、幂等（:144-169）——**LLM 不许凭空 cite 未 open 的 chunk**。
- P4 `merge_catalog_pure` 去重幂等（:173-196）。
- P5 `wiki_type_priority` 全序（:200-230）：thesis > synthesis > methodology > finding > comparison > concept > entity > source > query；None≡entity；未知=0 低于全部。
- P6 `truncate_chars` CJK 安全（:234-254）：截断输出 = N+1 chars 且以 … 结尾。
- P7 `split_prefetch`（:258-276）：prefetch ≤ cap 且 prefetch⧺rest == 输入（follow_relations 切分不丢不乱序）。
- P8-P10 `rank_key`：全序（sort_by 不 panic，:365-391）、superseded 恒被同参 live 压制（:395-416）、now 单调（时间推后排名不升，:420-447）。
- P11-P13 `classify_recall_outcome`：affected ⊆ opened 且 kind ∈ {recall_miss, recall_low_yield}（:484-521）；cancelled 恒 None（:525-540）；健康召回（cited≥2=LOW_YIELD_CITED_MAX+1）恒 None 不刷队列（:544-561）。
- P14 `structural_proposal_always_pending_review`（:575-606）：任意 kind/target/rationale 构造的 StructuralProposal status 恒 `pending_review`，且 BSON 序列化**不含 apply/applied/commit/committed/delete/deleted 任何字段**——结构化写物理上无法表达"已应用"。
- #620 离线召回度量 3 个 plain 测试（:641-810）：`metric_relevant_chunks_rank_into_top_k`（反作弊构造：相关 3 条静态分最差、噪声 7 条最优，recall@3 必须=1.0 且 hit@1 命中——query 相关度必须主导排序）；`metric_superseded_expired_do_not_poison_topk`（同内容 live/superseded/expired 三版，live 恒排前——"永不删"不毒化召回）；`metric_empty_query_degrades_to_static_order`（空 query 退化为 live+wiki_priority+confidence 静态序）。

#### knowledge_ask_e2e.rs（448 行，10 测试，全 #[ignore]）
- 主循环 4 路径：正常收敛（trace 顺序恒 `list_catalog → open_chunk → answer`，rounds_used=2，llm calls=2，:76-126）；空 corpus 立即返回"知识库无相关内容。"、0 LLM、rounds=0、trace 仍记 list_catalog returned=0（:129-149）；LLM 4 轮不收敛 → truncated=true、rounds_used=4 如实上报、cited 空、兜底 answer 行落 trace（:153-185）；**needs_review chunk 不可见**（verified-only catalog 红线，:189-206）。
- D3 关系图谱 6 测试：`follow_relations` 按 relation_kind 分流——references 目标 relation_role=None、contradicts 目标 relation_role="contradiction"（跟随但标记，非跳过，:219-270）；superseded redirect：open_chunk 旧版 → 返回现行版 id/正文（:274-293）、follow_relations 收现行版不收旧版（:296-330）、端到端 cite redirect 后的新版 id 不被 filter 丢弃（cite⊆opened 修复回归，:338-381）、多跳链 v1→v2→v3 跟到链尾（:384-405）、**新版未 verified 时停在旧版绝不 redirect 到未审定版**（:409-429）、自指环不死循环停在自身（:433-448）。

#### knowledge_ask_stream_e2e.rs（419 行，5 测试，全 #[ignore]）
- `answer_streaming` SSE 协议红线：每个 tool_trace.push 配对一条 `TraceEvent::Step`（Step 数=trace 长度，:154-163）；末尾恒 `Final` 且与非流式 answer() 等价（:174-182）；空 corpus 只推一条 Step(list_catalog)+Final 固定文案 0 LLM（:188-215）；truncated 场景末 Step 为 answer 且 truncated=true（:220-273）；cancel 软取消：AtomicBool 预置 true → 事件序列 `list_catalog → cancelled → answer` + Final(cancelled=true, truncated=true)、rounds=0、0 LLM（:282-333）。
- 场景 5 真流式（P1-3，:347-419）：answer 轮至少 1 个 `Token`，全部 Token delta 拼接 == 最终 `AnswerResult.answer` == Final.answer（不混 JSON 语法不漏字），工具轮 0 Token，Token 全在 Final 之前。

#### knowledge_auto_verify_enforce_integration.rs（344 行，2 测试，全 #[ignore]，repl_set）
- **P0 红线：AI 永不自动 verify 的 handler 接线**。`auto_verify_operation_knowledge_chunks` 过闸（证据齐+LLM 自称 verified+confidence≥threshold）的 chunk 必经 `enforce_verified_needs_human_audit`（verify.rs:401 接线、verify.rs:554 函数）强制降级 needs_human_audit。Seed 3 条规避 5% 抽样随机性（删接线后误绿概率 0.05³≈0.000125，文件头 :13-19 详证）。断言 response verified=0 & needsHumanAudit≥1 + 落库全部 needs_human_audit 且零 verified（:140-215）；response 带 runId/chunkIds 且 `knowledge_run_started` 审计事件先于模型工作写入 status=running（:156-181）。
- `auto_verify_counts_only_committed_revisions`（:218-344）：active domain schema 要求必填 stage、chunk domain_attributes 缺 → revision 失败 → processed=0/failed=1，chunk 保持 needs_review 原 confidence，0 revisions、0 usage_logs，但 start 审计事件仍存活（审计先行、计数只算已提交）。

#### knowledge_chat_apply_integration.rs（645 行，4 测试，全 #[ignore]，repl_set）
- `chat_apply_create_forces_draft_needs_review`（:287-339）：`apply_create_chunk` 落库**瞬间**（不 verify 直接查 DB）status=draft + integrity=needs_review（生产 chat.rs:1679-1681 强制）。
- `concurrent_and_replayed_chat_apply_is_exactly_once`（SR-111，:341-439）：并发双 apply + 重放 → 三个响应字节等同（同一 stable receipt）；恰 1 chunk、1 条 op=create revision、1 条 `knowledge_chat_applied` 审计事件；assistant turn status=applied + applied_at/apply_result 落值。
- `chat_apply_wrong_account_or_admin_is_zero_write`（:441-538）：错 account 或非 owner admin → NotFound 且零写（0 chunk/revision/event，turn 保持 pending）。
- `stale_chunk_snapshot_rejects_chat_apply_with_zero_write`（SR-130，:540-645）：update_chunk 附件带 expected_updated_at 冻结快照，并发写推进后 apply → `Conflict("chat_chunk_snapshot_stale")`，chunk BSON 字节不变、0 revision、0 审计、turn 回滚 pending（claim 回滚）。

#### knowledge_chat_dispatch.rs（147 行，4 测试，无 Docker）
- intent 闭集 6 值含 `digest_action`（:15-22，与 chat_turn match 分支 1:1）；plannedSteps 6 action 闭集（fix_chunk/add_chunk/retag/review_evolution/analyze_logs/dismiss）+ step.cardId ∈ selectedCards + stepId 唯一 + steps ≤8（:49-94）；总 estimatedLlmCalls ≤12、单步 1..=3（:97-123）；naturalReply 不命中禁词表（人工接管/接管/人工/takeover/hand-off 等，与 check-no-human-takeover CI 闸同源，:126-147）。

#### knowledge_chunk_transactions.rs（1769 行，18 测试，全 #[ignore] repl_set，**真 HTTP**：bootstrap_admin+session cookie+axum serve）
知识编辑事务性/快照回滚全集，故障注入手法=Mongo `collMod` validator 拒写某 collection：
- split：成功=源 archived + 2 draft 子(previous_version_id) + 3 revisions + 3 catalog_rebuild_jobs 同事务（:153-227）；schema 校验失败 → 400 且先插的子被回滚、源保持 active、0 revisions/jobs（:229-302）；legacy `newChunks` 注入（foreign ws+verified）→ 422 零写（:817-893）。
- merge：成功=源 archived+superseded_by=target、target 变 draft+needs_review、body 拼接 `target\n\nsource`、2 merge revisions、2 jobs（:304-387）；源校验失败 → target 更新回滚（:389-468）。
- rollback：精确还原快照内容但**重进审核**（status=draft+needs_review+confidence=0），运行期 usage_stats 不被快照覆盖（hit_count_30d=9 保留），rollback revision 带 before+after snapshot（:470-577）；legacy revision 无 snapshot → 409 `chunk_revision_snapshot_unavailable` fail-closed 零写（:579-657）。
- patch/PUT 越权：patch 带 managed 字段（workspaceId/accountId/status/integrityStatus/actor）→ 400 且 chunk BSON 字节不变（:659-728）；legacy PUT 带 foreign scope + 自 verify → 200 但只 title 生效、scope 钉原值、status 强制 draft/needs_review/confidence=0、revision source=human created_by=admin（:730-815）。
- repair/applied：只应用 acceptedFields（summary），skippedFields **由服务端派生**（请求自称空也会算出 title 被 skip），revision source="ai"，事件 `knowledge_repair_applied` 指向已提交 revisionId（:895-996）；事件写入失败（validator）→ 502 全回滚（:998-1090）。
- create：请求自称 active/verified/100 → 落库强制 draft/needs_review/0，row+create revision+catalog job 同事务（:1092-1168）；catalog job 被拒 → 502 全回滚（:1170-1241）。
- document PUT/PATCH：PUT 保留 15 个 server-owned 字段（workspace/account/domain/source_type/raw_content/content_hash/line_index/section_index/status/created_at/catalog_*），version 7→8，stale version → 409（:1243-1341）；PATCH 严格 dirty（no-op unchanged=true 不 bump version）、rawContent → 422、stale → 409、`summary:null` 清空字段（:1343-1472）。
- delete：chunk DELETE=软归档+archive revision（带双快照），重复幂等 unchanged=true 不重复历史（:1474-1558）；document DELETE 原子归档父+2 子+2 revisions+2 jobs，重复幂等 archivedChunks=0（:1560-1674）；第二个子 revision 被拒 → 502 全量回滚（父 active version 7、两子 active、0 revisions/jobs，:1676-1769）。

#### knowledge_closed_loop_trajectory.rs（468 行，5 测试，全 #[ignore]）
- `chat_apply_verify_then_answer_is_auditable_closed_loop`（SR-126 结算红线，:110-283）：真实 chat_turn 起草（intent+draft 两次 LLM）→ chat_apply 落 draft+needs_review+source_anchors（且不可召回）→ 人工 `verify_operation_knowledge_chunk`（留 1 条 op=verify revision）→ 生产 knowledge agent open+cite 该 chunk（llm.calls()=4=intent+draft+open+answer）。
- `supersede_demotes_old_below_new`（:290-344）：superseded 旧版 trust×0.1 降权但**留在 catalog**（只重排不剔除、物理不删）。`relation_graph_has_no_dangling_refs`（:350-407）。`unverified_draft_not_recallable_until_approved`（:413-468）：draft 不可召回，经生产 verify 审批后可召回。fixture 注释（:52-59）钉死 anchor 引文键必须是 `sourceQuote`（裸 `quote` 键读取侧恒忽略）。

#### knowledge_digest_budget_smoke.rs（58 行，5 测试，无 Docker）
- `RunBudget::new(id, 24000, 8, i32::MAX)` digest 默认预算构造/超额判定：LLM 维 3 次达 3 触发、token 维 1200≥1000 触发、tool 维 i32::MAX 不触发、`mark_degraded` 可调用不影响 is_exceeded。

#### knowledge_digest_compose_smoke.rs（126 行，4 测试，无 Docker）
- `KnowledgeDigestCard` 嵌套 Vec<Document>+metric BSON round-trip；`KnowledgeDailyReport.status` 闭集 partial/failed/ok（failed 时 cards 可空、error_kind=upstream_timeout；partial 时 error_kind=budget_exceeded）；target_refs 混合 chunk/pack/proposal 三 kind。

#### knowledge_digest_skeleton.rs（178 行，5 测试，无 Docker）
- Phase 1 骨架：三个新模型 BSON round-trip；`KnowledgeChatTurn` 老数据向后兼容（缺 kind/tool_calls → None/空 vec，:125-150）；card kind 闭集 7 值（chunk_missing_field/chunk_low_hit_rate/chunk_caused_block/pack_outdated/evolution_pending/evolution_released/freeform）、severity 3 值（info/warn/critical）、action 闭集 6 值（:155-178）。

#### knowledge_import_apply_integration.rs（501 行，5 测试，全 #[ignore] repl_set，SR-112，真 HTTP 双 admin）
- preview 封印协议：ImportJob 带 previewHash（canonical JSON sha256，:79-100）+ owner_admin_id + apply_status=ready。
- `concurrent_apply_and_replay_commit_exactly_once`（:248-325）：并发双 apply + 重放三响应同一 receipt；artifacts=(1 doc, 2 chunks, 2 revisions, 2 jobs)；job apply_status=applied。
- `second_candidate_failure_rolls_back_every_artifact_and_claim`（:327-366）：candidate-9999 不存在 → 400，artifacts 全 0，job 回 ready、apply_request_hash/result 清空。
- `wrong_admin_and_wrong_hash_leave_zero_writes`（:368-417）：非 owner → 404；hash 不符 → 409；零写。
- `shared_ingest_concurrent_replay_is_exactly_once`（:419-464）+ `shared_ingest_catalog_failure_rolls_back_every_artifact`（:466-501）：`ingest_chunked_text` 共享入口同样 exactly-once/全回滚（collMod validator 拒 catalog job）。

#### knowledge_operator_memory_isolation.rs（161 行，7 测试，无 Docker）
- `KnowledgeOperatorMemory` 与 contact memoryCard / agent soul memory 物理隔离：collection 名 `knowledge_operator_memory` 不撞黑名单（contacts/agents/agent_souls/memory_cards）；BSON 必含 workspace_id+account_id+operator_id 三元组；kind 闭集 preference/rejection/context（反例 human_takeover 不在闭集）；expires_at 仅此 collection 支持；内容不命中禁词表。

#### knowledge_preview_workspace_scope.rs（109 行，1 测试，#[ignore]，KNOW-1 回归）
- `test_knowledge_route_for_contact(contact=None)` 必须按**传入 workspace** 隔离（此前回落 default_workspace_id → 跨租户读泄漏）。用 chunk id 判隔离而非 count（default 库被 seed 也不误红）；额外押一条冗余 mock 防 default corpus 非空时队列耗尽假红（:77-85 注释）。

#### knowledge_router_fallback_e2e.rs（279 行，3 测试，全 #[ignore]）
- `route_operation_knowledge` fallback 红线（knowledge_router.rs:443）：agent 0 cited → 按 wiki_type_priority×dynamic_confidence 静态排序取 FALLBACK_TOP_N=5 弱证据回填，标 `knowledgeCoverage=weak` + `riskLevel=medium` + toolTrace 含 `fallback_rank{reason:agent_returned_zero_cited, selected:5}`；top-5 不含低优先级 source 类（thesis×2+methodology×2+entity×1，:74-187）。
- cited 全部 OOB（未 open 被 filter 清空）→ 同样触发 fallback（agent 不能凭空 cited 绕过，:192-249）。
- corpus 真空 → short-circuit `coverage=missing`、0 chunk、0 LLM（空知识库不假装有兜底，:253-279）。

#### knowledge_task_worker.rs（255 行，6 测试，无 Docker）
- planned_steps action 闭集 6 值与 `execute_step` match 对齐；KnowledgeChatTask status 闭集 pending/running/completed/failed/cancelled round-trip；task_progress turn 保留 phase/taskId/stepIndex/total；task_summary turn 保留 needsReviewChunkIds/failedStepIds/needsManualStepIds/noopStepIds/committedCount/completedSteps；`ChatProgressBus` bump 对订阅者可见 + 晚订阅者仍见后续 bump（watch channel 语义，:214-255）。

#### knowledge_tools_budget.rs（141 行，8 测试，无 Docker）
- `RunBudget` tool 维硬门：record_call 只动 LLM 维不吃 tool 名额；tool 维用满 → is_exceeded（chat loop force_stop 信号）；tool_call_budget=0 起步即拒且 is_exceeded；超额单调不可逆；**失败的 tool call 不消费名额**（token 维失败后小额仍可用）；i64::MAX token 拒绝不 panic 不污染；i32::MAX=不限；负 token clamp 0 不绕过校验。

#### knowledge_worker_behavior_integration.rs（198 行，3 测试，全 #[ignore]，SR-126）
- `execute_step` 类型化业务裁决：payload 非法的 fix_chunk/retag/add_chunk/dismiss → Failed；review_evolution → NeedsManual；analyze_logs → Noop；未知 action（drop_table）→ Err（:16-49）。
- `committed_add_has_real_draft_side_effect`（repl_set）：add_chunk Committed 必有真实落库 draft+needs_review chunk（:51-93）。
- `run_task_persists_mixed_verdict_buckets`：noop/needs_manual/failed 三步混合 → task status=failed + error_kind=knowledge_task_step_failed，completed_steps 状态序 ["noop","needs_manual","failed"]，failed 步带非空 error，summary turn 分桶计数正确（:95-198）。

#### chunk_batch_ops.rs（666 行，10 测试，全 #[ignore]，G3 批量操作 + D2 审计链）
- 批量 verify 3 条全成功（DB 落 verified/active）；批量 archive skip 已归档；`list_chunk_referrers` 返回 kind/note/wikiType；空 ids → 400；**无 source_quote 被 skip**（skip reason 提及 quote/anchor 闸）。
- D2 审计链：单条 verify 写恰 1 条 revision（op=verify/source=human/created_by=admin/before≠after hash，:363-416）；reject → op=reject + chunk status=rejected（:418-452）；批量 verify 每条 1 revision 且 reason 透传 note（:454-493）；**stale 快照 verify → Conflict("chunk_revision_conflict") 且零 revision**（:495-552）；缺 anchor 被 D2 gate 挡 → 400 零 revision（`chunk_has_citable_anchor` 要求 anchor 自带非空 sourceQuote，:554-593）。
- `auto_verify_writes_one_revision_per_processed_chunk`（:604-666）：**本地 mock 可跑**的 auto_verify 审计路径（此前只活在 real-LLM 测试里 = 无 key 即假绿），N 条 chunk → N 条 op=verify/**source=rule**/created_by=auto_verify 的 revision。

#### chunk_lock_lifecycle.rs（252 行，7 测试=4 默认+3 ignore，P1-4 协作锁）
- `CHUNK_LOCK_TTL_SECONDS=300` 合约值锁死（:63-67）；ChunkEvent 序列化 kind=snake_case（locked/unlocked/revised）；`is_expired` 边界含等号；broadcast channel 晚订阅者收不到旧事件；跨 workspace acquire → 404 且**零 presence 残留**（:147-168）；同 workspace 并发 acquire → 恰一个 200 + 一个 409 `chunk_presence_by_other`，两者都带 advisory=true（presence 只是协作提示，写权仍由认证+事务+CAS 决定，:170-220）。

#### chunk_put_preserves_unmodeled_fields.rs（271 行，3 测试，全 #[ignore]，repl_set）
- PUT replace_one 不清空「请求体无法表达」的 13 字段：provenance/wiki_type/chunk_type/locked_fields/dynamic_confidence/integrity_score/created_at 等钉原值，title 更新（:37-151）。
- KB-10+KB-11（:162-247）：PUT 后端强制 per-chunk `locked_fields`（锁 title 的 PUT 改 title 被静默丢弃、未锁 summary 正常更新，复用 `effective_locked_fields`+`enforce_locked_fields` 单一真相源）+ 补写 chunk_revisions 审计行（op=patch/source=human）。
- PUT 不存在的 chunk → NotFound（不 upsert，:250-271）。

#### chunk_revision_ai_draft_integration.rs（566 行，5 测试，全 #[ignore] repl_set）
- **P0 接线钉**：`apply_chunk_revision(source=Ai)` 把 active+verified chunk 强制打回 draft+needs_review（生产 chunk_revisions.rs:207-212；删掉即红——PBT 只测试了自身复制的逻辑，:32-114）。
- KB-09（:125-219）：AI patch 数组字段 union 既有源必须是**原始 existing_bson** 而非被 clobber 的 after_patch——product_tags=["A"] + patch ["B"] → {A,B} 非 [B]；同时留审计行 + 打回 draft。
- S-08 并发 CAS（:224-386）：16 并发 patch → ≥1 成功 + >0 Conflict("chunk_revision_conflict")；最终内容来自成功者；**revision 行数 == 成功数**（冲突不留孤儿 revision）；catalog job 数 == 成功数。
- validator 拒主行写 → 回滚 provisional revision + 不入队 catalog（:388-478）；空 patch 审计型 no-op：主行字节不变、revision before==after snapshot、**不**入队 catalog job（:480-566）。

#### chunk_type_routing_pbt.rs（258 行，4 proptest，无 Docker，每个 256 cases 共 1024 ≥ 64 门）
- `format_operation_knowledge_for_prompt` 按 chunk_type 分段：section 顺序恒 product_fact→style_template→peer_case→negative_example（4 个中文 header 常量 :35-38）；每 chunk title 恰出现一次；未知 chunk_type（含空串）回落 product_fact bucket 且不自创 header；空 bucket 不留 header。

#### digest_cross_tenant_scope_integration.rs（144 行，1 测试，#[ignore]）
- 跨租户缺陷回归钉（2026-07-02 确证）：`digest_today` 兜底合成旧代码硬编码 default ws（跨租户读泄漏+写串）。修复=`generate_today_digest(state, workspace_id, account_id)` 透传 `admin.current_workspace`。断言三连：返回报告 workspaceId=ws_tenant_b、落库 tenant 下=1、default 下=0（:65-144）。

#### import_job_lifecycle.rs（432 行，7 测试，全 #[ignore]）
- ImportJob BSON snake_case（与索引键一致）；stale running (claimed_at < timeout) 命中 reclaim filter；`{_id, workspace_id}` IDOR 隔离查询返 None；job 回 pending 后旧 claim 的终态写 no-op（`update_owned_import_job` owner filter）；TTL 契约：pending 无 expires_at（进行中永不被 TTL 删）、终态置 expires_at=+24h；m056 迁移幂等且保留 legacy claim_token/claimed_at、补 claim_generation=0；**世代 fencing 全链**（:330-432）：A claim(gen=1)→过期→reclaim→B claim(gen=2, 新 token)，A 的进度/终态写 no-op，A 冻结的 scanner snapshot filter matched=0（gen1 快照不能 reclaim gen2），B 正常写进度。

#### import_pdf_smoke.rs（188 行，3 测试，全 #[ignore]，P1-5）
- 运行时手拼合法单页 PDF（自算 xref，:24-99，不提交二进制 fixture）。fence 文本 → block 解析 ≥1 chunk；无 fence → fallback blob 恰 1 chunk；两者一律 draft+needs_review；空 PDF → 拒绝零写。

#### ingest_worker_smoke.rs（274 行，4 测试，全 #[ignore]，P1-6 + SR-109 SSRF）
- **SR-109 出站安全**：loopback（wiremock 本地地址）源在发请求前被拒（failure_streak=1、last_error 含 "non-public network address"、wiremock 零请求、零 chunk，:91-134）；云 metadata 169.254.169.254 拒绝无 I/O（:139-165）；H1 回归：not-due 源 skip 且 **last_fetched_at 原封不动**（旧 bug 会刷成 now 导致永不更新，:171-226）；due 的私网源会被评估然后拒绝（last_fetched_at 不变、streak+1、零请求，:230-274）。

#### sr115_catalog_rebuild_recovery.rs（360 行，4 测试，全 #[ignore] repl_set）
- catalog 投影恢复：过期 claim + 乱序 generation 收敛——gen1 过期 job 被 reclaim 后判 `superseded`（attempts=2/claim_generation=2/token 清空），gen2 job done，父文档 desired=applied=2、catalog_version 7→8、投影含最新内容（:96-194）；并发双 worker 同 gen → 恰一次提交（done=1/superseded=1，catalog_version 只 +1，:196-269）；rolling deploy legacy target_generation=0 升为持久 gen1（:271-334）；无父文档的孤儿 job → 终态 `discarded`（:336-360）。

#### sr117_ingest_source_recovery.rs（298 行，4 测试，全 #[ignore] repl_set）
- m053 迁移幂等：legacy source 补 source_generation=1/claim_generation=0 且不动 worker_id/claim_token/locked_until（:84-137）；**配置变更 fencing**：URL 改动 $inc source_generation → 旧 claim finalize 失败、artifacts 全 0（:139-200）；过期 lease reclaim：新 owner claim_generation+1、旧 owner finalize 拒绝、新 owner 提交后 artifacts=(1,1,1,1) 且 claim 字段清空、ingest_count=1（:202-259）；并发双 worker → 恰 1 个 claim 成功、1 份 committed graph（:261-297）。

#### sr121_digest_snapshot_recovery.rs（150 行，2 测试，#[ignore]，standalone 可跑）
- digest 快照两代协议（attempt_generation vs current_generation）：失败的 regenerate 保留上次成功快照（visible.status=ok、cards 不变、current_generation=first、latest_attempt_status=failed/error_kind 记录，:31-85）；晚到的旧 generation finalize 不能覆盖新成功（返回/落库均为新代 cards，:87-150）。

#### sr122_knowledge_task_recovery.rs（193 行，1 测试，#[ignore] repl_set，SR-122/123）
- 全链 fencing：claim(gen1)→过期→reclaim(gen2 新 token)；旧 owner commit → Err 含 "knowledge_task_claim_lost" 且零 chunk；新 owner commit → Committed + chunk 落库；**同 stepId 重放 → 同错误 fencing**（exactly-once）；completed_steps 恰 1 条 committed 带 chunkId；chunk account_id 保持 "account-a"（**账号级任务产物不得扩权成 workspace 共享知识**，:161-179）；1 条 revision。

#### sr125_digest_dispatch_router.rs（460 行，3 测试，#[ignore]，真 HTTP Cookie Router）
- canvas 派工经 cookie 鉴权重建**服务端权威 task**：cards/planned_steps 由服务端按 reportHash/cardHash 封印候选重建（step.targetChunkId/summary 来自服务端 card 非请求体），owner_admin_id=会话 admin，dispatch_binding 落库，task+progress turn 各 1（:243-284）。
- chat 派工须携带服务端封印 candidateHash + sourceTurnIndex 且被持久化（:286-398）。
- 三种拒绝零写（:400-460）：stale reportGeneration → 409 `digest_dispatch_snapshot_stale`；跨 admin 复用他人 session → 409 `chat_session_scope_conflict`；accountId 不匹配 → 409 `digest_dispatch_account_mismatch`。

#### sr131_document_metadata_patch.rs（225 行，1 测试，#[ignore] repl_set，真 HTTP）
- 文档元数据 PATCH 窄口径+版本围栏一体测：dirty patch version 7→8 且只改 title 其余 14 字段钉原值；no-op unchanged=true 不 bump；rawContent → 422；stale version → 409；`summary:null` 清空（version→9）。（与 knowledge_chunk_transactions 的 document_patch 用例互补，此处独立 workspace + 逐字段断言。）

#### sr132_review_queue.rs（254 行，1 测试，#[ignore]，真 HTTP）
- review-queue 是**服务端拥有的 scoped 投影**：默认队列只含 draft/active 可复核行（archived、外域行排除）；counts 分面 needs_review=2/source_orphan=1/pending_verification=1/contested=1/dependents_pending=1（分面**有意重叠**）；单行 reviewCategories 数组；dimension=pricing 大小写不敏感命中 topicAliases、capability 精确过滤；未知 dimension → 400。

#### page_merge_pbt.rs（266 行，9 测试=8 proptest+1 plain，无 Docker）
- `union_array_fields` 集合等于 existing∪incoming、幂等、保序（existing 去重序列是输出前缀）；locked 字段 patch 必拒；patch 只覆盖列出键不引入新键；70% 阈值边界精确（等于不截断、严格小于截断）；`compute_chunk_hash` 字段顺序无关；`enforce_locked_fields` 锁定字段恒==existing、非锁定保留 merged；`DEFAULT_UNION_ARRAY_KEYS` 必含 tags/search_terms/applicable_scenes/business_topics（:253-266）。

#### annotation_quality_gate_integration.rs（433 行，6 测试，全 #[ignore]）
- 缺口 6：`normalize_target_stages` alias「需求挖掘」→ canonical `need_discovery`（m006 seed 免费可达）；越界阶段名 → Err（upload 400 来源）。「字典未配置放行」分支**故意不在集成层测**（会污染进程级共享 taxonomy 缓存，由 lib 单测覆盖，:32-36 注释）。
- 缺口 3：`review_media_asset` 落审计事件 kind=`media_asset.reviewed`（status=approved、details.reviewed_by/asset_id）+ 素材状态同步 approved。
- 缺口 2：`update_assist_override` force_on 写 `domain_attributes.assist_mode_override`、default $unset 该键；闭集外 mode → BadRequest；跨 workspace → NotFound 且零写（IDOR）。

#### integrity_report_d2_e2e.rs（88 行，1 测试，#[ignore]）
- `build_operation_knowledge_integrity_report` 的 D2 降级口径（对齐 digest_inbox.rs:455）：仅 `status=active && source_anchors.is_empty()` 计入 anchorsMissing（active 缺锚=1、active 有锚不计、draft 缺锚不计），total=3。

#### structured_organization_integration.rs（175 行，2 测试，全 #[ignore]）
- 素材 tags 检索：`list_content_assets?tag=报价类` 精确命中含该 tag 的素材、排除不含者；跨 workspace tag 过滤不泄漏（workspace scope 保留）。upload 写 tags 的端到端受 Multipart extractor 无法手工构造的限制不在此测（:13-17 注释）。

#### vision_safety_gate.rs（224 行，4 测试，全 #[ignore]，P1-5/#574）
- 无任何视觉模型（纯文字 active 或零 provider）→ `import-apply-image` 报 `visionNotSupported`（502 语义）零 chunk；文字主模型 supports_vision → Runtime 分支 mock 抽 fence → chunk 全 draft+needs_review；vision 返回空 fence（3 次重试后）→ fail-closed 报 "missing non-empty `fence`" 零 chunk。

#### hc028_real_digest_task_e2e.rs（478 行，1 测试，#[ignore]，真模型硬门）
- **不用 legacy skip 宏**：provider 缺失/上游不可达/空 digest/缺 worker 产物一律 fail（不静默跳过）。真实 provider 从生产配置库读取（REAL_LLM_CONFIG_MONGODB_URI，只读），业务写全落 TestApp 随机 repl_set 库（:42-96）。
- 全链：digest regenerate（真 LLM compose）→ 必产 fix_chunk 卡绑 seeded chunk → chat 派工（intent=digest_action、封印 candidateHash）→ 创建 sealed task → `tick_once` worker 真 LLM 修复提案 → task completed、步骤 committed 带非空 repairDraft.patch；**源 chunk 保持 draft+needs_review 且 source_quote 仍空（修复提案绝不自动 apply）**（:411-423）；llm_call_logs 恰含 knowledge.digest.compose + knowledge.chunk.repair.propose 两条 success 审计（模型身份非空）；**MCP wiremock 零请求**（知识修复链路绝不碰微信发送）；CapabilityEvidence pass(6,18) 落台账。

### 3.B 演化主题（8 文件 + 近亲 2）

#### evolution_policy_router_integration.rs（197 行，1 测试，#[ignore]，HC-017，真 HTTP Cookie Router）
- **manual-only release 政策**：PUT `/evolution/runtime-flag` 带 `thresholdAutoReleaseEnabled=true` → 400 且 error 含 "human-release policy"，**被拒请求不落 flag 行**（count=0，:144-169）。
- 7 天聚合完整性：25 条窗口内 + 1 条 8 天前 + 1 条外域 experiment/proposal，`GET /evolution/experiments?limit=5` → items=5 但 aggregate7d 服务端全量扫描：experiments=25、proposals=25、released=25、significancePassRate=1.0、coverage.complete=true、windowHours=168、source="server_time_window"（聚合不受分页 limit 与跨租户污染，:171-193）。

#### evolution_prompt_shadow.rs（517 行，3 测试，全 #[ignore] repl_set）
- prompt 候选 shadow replay 全链（`evolution::replay::run_shadow_replay` → `agent::prompt_shadow::shadow_replay_prompt_one`）。completed 路径（:307-412）：源 run（顶层 `source_event_id`=真实 message_id——生产 gateway 不往 context 写 inboundMessageId，:107-112 注释）+ inbound + managed contact；**baseline/candidate 两侧各调 Reply+Review+ClaimGate 共 6 次 LLM**（:358-363）；断言 original/new 5 闸命中向量都非空、original 基线来自**冻结重跑**而非历史分数（fact_risk_block=false）、双侧 final_review_status="approved" 来自生产同源 finalize、selfCritiqueAddressed 双侧存在。
- `source_message_unavailable`（:418-466）：retention 探针（**snake_case `message_id`** 字段名回归——若误用 camelCase 真消息也 count==0 会错杀全部 shadow）短路 → status=failed、0 LLM。
- `contact_unavailable`（:470-517）：message 在但 contact 缺 → failed("contact_unavailable")。failed 是业务结果不是 Err。

#### evolution_release_redline.rs（286 行，3 测试，全 #[ignore] repl_set）
- release_prompt 三道红线闸（生产 release.rs:256 起 / prompt_guard.rs）：闸 1 禁词（字符拼接构造"人工接管"绕 lint，:29-31）→ `EvolutionError::RedlineGateRejected`、**0 LLM 调用**、prompt 无新版本、proposal 停在 eligible_for_release（:123-168）；闸 3 LLM 语义 violation=true → 同样拒绝且恰 1 次 LLM（:174-218）；合法放行对照（violation=false）→ version+1、新内容=原文开头+片段结尾（红线正文逐字保留）、proposal→released 且记录 previous_prompt_version（:225-285）。

#### evolution_rollback_status.rs（182 行，2 测试，全 #[ignore] repl_set）
- rollback_prompt 把 previous_version 行置回 current 时**必须一并恢复 status=active**（此前被归档的行仅翻 current 指针会让 load_prompt(只取 active) 静默回落 default_prompt_content，:11-92）。
- previous_version 历史行被物删 → rollback 必须**中止事务返 Err**：proposal 保持 released、当前 current(v2) 不被翻 false（事务整体回滚，非"翻掉 current 后无 current 可用"的假成功，:97-182）。

#### evolution_workspace_scope.rs（151 行，3 测试，全 #[ignore]，SEC-1）
- proposal 端点 `{_id, workspace_id}` 复合过滤形状：跨租户 detail 查询 → None；release/rollback 同形过滤跨租户命中 0/本租户 1；未知 workspace 0。文件头明示（:4-18）：handler 是 pub(super) 无法直调，走 filter-shape 约定；EVO-2（released_by=真实操作者）由代码审查保证。

#### m040_evolution_release_protocol.rs（192 行，2 测试，全 #[ignore]）
- m040 对 legacy threshold_overrides 回填 released_revision（`threshold_revision(id, value)`）并按 (workspace,account,gate_key) 选 released_at 最新者为唯一 current_version=true，幂等重跑（:32-122）。
- 同 proposal 双 artifact（腐蚀快照，先 drop 守护索引构造）→ fail-closed 报 "duplicate threshold artifacts"，**任何行都不部分回填**（无 released_revision/current_version 键，:124-192）。

#### prompt_publish_evolution_guard.rs（144 行，1 测试，#[ignore] repl_set，SR-055）
- 手工 publish 是 append-only 单 current 切换：system(v1 archived) + evolution(v2 active current) + manual draft(v3) → publish v3 后三行全保留、恰 1 个 current、v1/v2 均 archived+非 current、v3 active+current；`load_prompt_for_contact` 对不同 contact/locale 返回同一版本 v3（**运行时绝无按 contact 分桶的多 active 行为**，:119-141）。

#### reset_pack_preserves_evolution_critic_integration.rs（74 行，1 测试，#[ignore] repl_set，M12）
- `reset_prompt_pack_v2`（delete_many 全部 prompt_templates）后 `evolution_critic_v1` 必须仍存在（active+current）——修复=reset 末尾补调 `ensure_evolution_prompt_pack_v1`；load_prompt 不再 NotFound（演化 Critic 循环不断，:42-74）。

#### sr097_lesson_promotion.rs（349 行，3 测试，全 #[ignore] repl_set，真 HTTP）
- lesson 晋升 peer_case 恰好一次：并发双 promote + 重放 → 同一 promotedChunkId（=lesson OID）、alreadyPromoted 一真一假、重放恒 true；恰 1 chunk（provenance.source="lesson_promotion"/source_doc_id=lesson_id）、1 条 `lesson_promoted_to_peer_case` 事件；lesson review_status=promoted（:86-178）。
- 审计事件被 validator 拒 → 502，lesson CAS 回滚字节等同、chunk 0；撤 validator 重试成功（:180-251）。
- m055 迁移：exact pair 回填 provenance；orphan chunk 存在 → 整体 fail 且 pending 行不被部分回填（:253-349）。

#### lessons_learned_filters.rs（99 行，1 测试，#[ignore]）
- `aggregate_lessons_for_workspace` 三类模式各取自权威数据源：success（decision_reviews approved+buying_signal）、reviewer_misjudge_negative（approved_but_user_negative 信号）、blocked_by_safety_guard（run_logs lifecycle=failed_after_decision）→ 各 upsert 一条 lesson（lesson_id=`{ws}::{kind}`、count=1、sample_run_ids 精确）。

### 3.C 租户/鉴权/安全主题（10 文件）

#### workspace_isolation.rs（586 行，7 测试，全 #[ignore]，P1-1）
- **自我声明边界（:9-11）：多数用例只验证 collection/helper 过滤形状，不得外推为 IDOR 端点证据；真实 Router 证据在 SR-176。**
- chunk 跨租户读隔离 + 未知租户空结果；m016 backfill：无 workspace_id 的 legacy 行对所有租户不可见，重跑 `m016_backfill_workspace_id_on_legacy_rows::run_step` 幂等回填 default（注释红线 :182-184：不删 marker 走完整 runner——production 会要求 APPROVED_MIGRATIONS 审批闸，测试不得绕过）；contact `{_id, workspace_id}` 复合过滤；5 个管理集合（wechat_accounts/follow_up_tasks/agent_souls/command_runs/user_operation_guide_previews）+ 4 个 admin 集合（agent_send_outbox/operation_state_policies/operation_domain_configs/evaluation_scenarios）逐一跨租户 0/本租户 1；账号 `{workspace_id, account_id}` 过滤（防 MCP account_id 透传 IDOR）。注（:476-477）：system_taxonomies 是全局字典（无 workspace_id）按设计不分租户。

#### sr176_real_route_isolation.rs（434 行，2 测试，全 #[ignore]，**真 middleware + 真 Router**）
- 主测试（:325-353）一次收集 12 项证据：跨租户 contact GET → 404 / 本租户 200 + 正确 wxid；跨租户 product PUT → 404 且 DB 原值不变；同 product_id 双租户并存时本租户 PUT 只改自己（B 的 same-id 行原值）；`lookup_session` 对过期行返 SessionExpired；过期 cookie /auth/me → 401；有效 cookie → 200；**发 session 后撤销 admin ACL → 同一 cookie 立即 401**（middleware 每请求读权威 AdminUser ACL 而非信任 cookie 快照，:289-307）。
- JWT 版（:355-434）：jwt_enabled + 测试 PEM（tests/fixtures），Bearer 有效 → 200；撤 ACL 后同 token → 401（JWT 也不豁免 ACL 实时校验）。

#### h3_cross_tenant_idor.rs（190 行，3 测试，全 #[ignore]）
- 14 站点共用单闸 `resolve_authorized_workspace` 在真实 handler 生效：admin(ACL=[ws_a]) 带 `workspaceId=ws_b` override 调 `activate_provider` → Err 含 "workspace_not_in_user_acl" 且 ws_b provider 不被激活（无进程级热切换副作用）；`list_providers` 同拒（不泄漏他租户列表）；不带 override 回落 current_workspace 激活本租户成功。

#### auth_middleware_integration.rs（244 行，4 测试，全 #[ignore]）
- `authenticate` 不泄漏存在性：ghost 用户与密码错同返 InvalidCredentials（:32-67）。
- `create_session` workspace 三级回退：authorized default → ACL 首个（default 被移出 ACL 不得继续授予）→ **空 ACL = 完整撤权返 NoAuthorizedWorkspace**（不回落，:71-122）。
- 过期 session → SessionExpired；登出幂等（删两次不报错）；删后 lookup → SessionNotFound（:126-186）。
- `switch_workspace` 切 ACL 外 → 拒绝（workspace_not_in_user_acl）；ACL 内不触发 ACL 拒绝（:189-244）。

#### sr016_auth_rate_limit.rs（147 行，1 测试，#[ignore]，真 HTTP + connect_info）
- login 与 token 共享限速（AuthRateLimiter(300s, client=2, target=2, global=10)）：两次错密码（各 401）后第三次**正确密码也 429** `{"error":"auth_rate_limited"}` + Retry-After 头。审计 `auth_security_events` 3 行 outcomes=[invalid_credentials×2, rate_limited]、entrypoints=[login,token,login]；**隐私红线**：行内无 username/password/token/ip/client_address 字段，全文不含用户名/密码/127.0.0.1；retention 恰 90 天（expires_at-created_at）。

#### jwt_auth.rs（222 行，4 测试，**无 ignore 默认跑**，P1-7）
- `JwtKeys::from_config` enabled 时缺公钥 → 错误信息含 JWT_PUBLIC_KEY_PEM；issue→verify round-trip claims（sub/username/current_workspace/exp>iat）；篡改 token → Unauthorized("token_invalid")；手工构造过期 claims → Unauthorized("token_expired")。本文件在测试内完整拷贝了一份 AppConfig 字面量（:29-166）。

#### products_workspace_isolation.rs（137 行，3 测试，全 #[ignore]，G2）
- 自我声明（:5-13）：只验 collection 过滤形状 + 复合唯一索引，不得外推 Handler 证据。product_id **workspace 内唯一非全局**（同 ws 重复被 unique 索引拒、跨 ws 同名合法）；跨租户读隔离；未知租户空。

#### migration_safety_redlines.rs（125 行，2 测试，全 #[ignore]）
- m057：operation_state_policies 的 allowed missing/null/empty 三态物化——missing/null 补 acknowledgement，显式 [] 尊重不补；**并发双跑 + 三跑幂等**（:44-49）。
- m058：一 workspace 双 isActive provider（先 drop 唯一索引构造 legacy 腐蚀）→ **只读审计 fail-closed**：报错含 workspace 与 "p1,p2"，不选举不改写任何行（before==after、active 仍 2 行）。

#### account_security_integration.rs（210 行，5 测试，全 #[ignore]）
- `list_accounts` 响应体绝不含 mcp_api_key 明文，只暴露 `mcpKeyConfigured` 布尔（:59-85）；WechatAccount 手写 Debug 掩码 key（防 tracing/panic 泄漏，:88-97）；`update_account_mcp_key` 跨 workspace → NotFound（handler 注入 current_workspace 构 filter）；同 ws 但 expectedAccountId 与 URL id 不符 → Conflict 且零写（key 保持 OLD_KEY）；**app_id 全局唯一约束**（跨 workspace 也拒重复——webhook 路由按 app_id 定账号，重复即路由不确定，:189-210）。

#### sr174_cache_database_isolation.rs（196 行，1 测试，**无 ignore**）
- 进程级 LazyLock 缓存（taxonomy + domain profile）必须按 Mongo database 隔离：两个 TestApp（同 workspace_id="default" 不同随机库）交错 warm-up + 读取，A/B 各自恒读到自己的 profile/stage；A 库拒绝 B-only taxonomy、反之亦然（旧共享缓存下 B 的 warm 会覆盖 A → A 读到 B 的 profile）。

### 3.D 迁移主题（7 文件）

#### migrations_idempotency.rs（207 行，3 测试，全 #[ignore]）
- 迁移框架账册幂等：启动后每条注册迁移一行账（只数 `MIGRATIONS` 内 id，排除 workspace 模板 marker），二次 `run` 按 `_id` 全跳过（:28-69）。
- **production 审批闸**（:71-163）：`run_with_policy(production=true, approvals=∅)` 对危险迁移（destructive_probe 探针）→ 不执行（0 写）、账册标 status="blocked"+reason 含迁移 id；给精确 approval 后重试 → 恰执行一次、status="applied"、reason/blocked_at 清空；再跑 applied marker 防重执行。
- m037：legacy 空 ACL admin 物化为 workspaces=["default"]+default_workspace="default"（:165-207）。

#### m018_backfill_domain_stage.rs（138 行，1 测试，#[ignore]）
- 顶层残留 customer_stage/intent_level/customer_stage_updated_at 并入 domain_attributes：**只回填不 $unset（可逆）**；已有 domain 值优先于顶层陈旧值（mergeObjects 新覆旧）；无残留行不被 filter 命中；二次幂等。

#### m029_cleanup_contact_identity.rs（350 行，3 测试，全 #[ignore]）
- webhook 建档污染治理三步：非真人 normal（gh_/@chatroom）删除但 **conversation_messages 绝不删**；真人 roster 命中回填 nickname/avatar_url；nickname="Demi" 且 roster 未命中 → $unset nickname；**managed 一律保留**（含 gh_）且运营字段（agent_status/operation_state）零改动；幂等（:30-235）。
- roster 身份按 (workspace, account) scope：同 wxid 双租户各回填各自 roster 昵称（:237-314）。
- 无 workspace_id 的 legacy 行 skip 不 abort（归属未知行留待 m036 审批回填，:316-350）。

#### m034_review_reconciliation.rs（81 行，1 测试，#[ignore]）
- SR-007 纠偏：version 更低但 status=active 的行胜过 version 更高的 draft——重跑后 active(v1) current=true、draft(v2) current=false、每 key 恰 1 current。

#### m039_scope_revision_behavior.rs（476 行，3 测试，全 #[ignore]）
- 主测试（:17-354）：重建 pre-m039 索引拓扑后跑 m039——chunk_revision 从**父 chunk** 精确取 workspace；behavior_signal 仅在 (workspace,wxid) 唯一归属一个 account 时回填 account_id；新 scoped 索引建立、legacy 索引退役；二次幂等；显式历史 account 身份即使 contact 已被清理也保留；**孤儿 revision（无父 chunk）fail-closed** 报 "without parent chunk"；**歧义 wxid（2 账号）fail-closed** 报 "2 matching accounts"，且验证发生在**第一笔回填写之前**（先插的合法行不带 workspace_id 证明未部分写，:343-351）。
- `ensure_indexes` 对"同 keys 历史名"索引等价复用不重建（:356-435）；同 keys 但 options 不兼容（多 unique）→ fail-closed 报 "incompatible options"（:437-476）。

#### m045_relationship_review_cycles.rs（157 行，2 测试，全 #[ignore]，SR-060）
- 退役全历史 unique 索引 `{workspace,contact}` → 换 partial unique `uniq_relationship_pending_ws_contact`（仅 pending 占槽）：terminal 历史不占 pending 槽、同 contact 双 pending 被 E11000 拒、历史行数保留（:76-126）。
- 畸形 pending 行（缺 account_id）→ **在 drop 破坏性索引之前** fail-closed（legacy 索引仍在、新索引未建，:128-157）。

#### m049_prompt_planning_currents.rs（107 行，1 测试，#[ignore] repl_set）
- m043 已 applied 但 planning-only 草稿（group.policy/moment.policy）仍带 current 指针的升级库：m049 纠偏为 status=draft+current=false 且**不改内容不发布**；`ensure_prompt_pack_v2` 启动对齐接受纠偏结果（返回 false 不再改写）；重跑不新增版本行。

### 3.E LLM/基础设施主题（5 文件）

#### llm_retry_jitter.rs（91 行，6 测试，无 Docker，默认跑，baseline PBT 四件套之一）
- `compute_backoff`：Retry-After(5s) 压过短退避基线（≥5000ms）；无 Retry-After 时指数退避 ∈ [base·2^(n-1), +base)（prod 路径含 jitter）；长指数(8000ms)压过短 Retry-After(2s)。
- `is_retryable_llm_error`：429/5xx 可重试；400/401 不可重试；**JSON parse 错误不重试**（非 JSON 内容只调一次）。

#### llm_provider_activate_integration.rs（515 行，5 测试，全 #[ignore]）
- 文件头自述假绿对照（:5-8）：原测试只断 HTTP 200 不查 DB。`activate_yields_exactly_one_active_and_is_target`：activate 后 DB **恰一条 isActive=true 且是 target**；activate 不存在的 provider → NotFound。
- SR-165（:207-289）：编辑 active provider = 生产发布——缺测试审批 token 或 stale expectedUpdatedAt → 409 **零写**（DB 行字节不变 + runtime registry generation/meta 不变）。
- SR-166（:294-393）：可空调优字段协议——omitted 保留 override、显式 JSON null 删除 override 且响应报 effective 全局默认（timeoutSecondsSource="global_default"），raw 文档键被物理移除。
- SR-167（:399-515）：vision 指派是生命周期不变量——被指派 provider 不能撤 supports_vision（409）、不能删（409）且零写；reassign 原子换恰一个 capable 目标；partial unique 索引拒第二条 isVisionActive。

#### llm_usage_summary_integration.rs（130 行，1 测试，#[ignore]，HC-007/SR-156，真 HTTP）
- 101 条本账号 log + 1 条他账号：`/llm-usage?limit=100` → summary 覆盖**全部留存日志**非 detail 样本（totalCalls=101、totalTokens=1010、cache hit/miss=404/606、usageComplete=true）；items 恰 100、itemsTruncated=true；window.kind="retained_logs"；他账号不泄入。

#### maycran_transport_probe.rs（109 行，2 测试，#[ignore]，临时诊断探针）
- 生产 `LlmClient` 直连 Maycran 候选模型（claude-sonnet-4-6 等 6 个）验证 JSON 输出可达性 + 业务 prompt 尺寸（1k/4k/8k/16k chars）承压。缺 MAYCRAN_API_KEY 直接 return（**软 skip 不失败**）；不打印 key；无 DB/MCP 写。

#### ops_versioned_index_boot_brick.rs（131 行，3 测试，全 #[ignore]，H8）
- 多版本 operation_domain_configs / operation_state_policies 数据下重跑 `ensure_indexes` 必须成功（旧残留 2-key/3-key unique create 会 E11000 → main.rs `?` → **启动崩溃 boot-brick**）；4-tuple unique（含 version）仍拒同 (ws,domain,version) 重复行（唯一性未降级只是维度对了）。

### 3.F roleplay/domain/taxonomy 主题（8 文件 + common smoke 6 文件）

#### common 配套 smoke（6 文件，多数无 Docker 默认跑）
- **common_smoke.rs**（26 行，2 测试）：test_account fixture 经生产 WechatAccount 模型反序列化 round-trip；TestApp 启动即 prompt pack 已 seed、0 LLM 调用。
- **dynamic_smoke.rs**（77 行，4 测试）：R5.0.1 家族判定契约的本地宿主——默认三角色异族、同 host 不同 vendor 异族、完全同源判同族、厂商段提取（meta/llama→meta、mimo-v2.5→mimo）。
- **identity_generator_smoke.rs**（134 行，6 测试）：候选库 ≥4 大类且四类齐；select_skeleton 同 seed 确定、模回绕、0..len 全覆盖；funnel 极性按大类正确；`apply_category_semantics` 销售域（funnel 开/交易事实开/grounding 不旁路/低风险可 light review）vs 陪伴域（全反）自洽；peer_social=关系型、formal_business=漏斗型。
- **judge_rubric.rs**（227 行，12 测试）：R1.1 键集/极性契约本地宿主 + **R1.2 gate 行为 mock 验证**：ObserveOnly 吞 5xx 抖动返 None；QualityGate 全失败 panic("唯一质量门")；`endpoint_not_found`(404) 与 `http_4xx`(402 余额) 即使 ObserveOnly 也 panic("非瞬时错误")——堵"漏 /v1 配错端点假绿"；成功路径返回各维 median。
- **redline_smoke.rs**（55 行，4 测试）：G6 补词（转人工/人工客服命中）、G5 否定拒绝零误报（5 种拒绝话术）、先否定后真承诺仍命中、"人工智能"不误伤。
- **roleplay_fixtures_smoke.rs**（138 行，5 测试，4 个 #[ignore]）：P0 退出条件自验——seed 的情感 profile 可被 `load_active_domain_profile` 读回（profile_id/conversation_modes 含 intimate_companion/grounding bypass/funnel 关/**transaction_facts 关**/anniversaries 日期记忆维持久化）；默认 ws 仍回落 DEFAULT（seed 不污染他 ws）；override_review_prompt 覆写后 load_prompt 读回 MARKER；seed_verified_chunk 可写；RoleplayLedger 输出含 suspected_layer 的 JSONL。

#### roleplay_emotional_companion_e2e.rs（1125 行，1 测试，#[ignore]，env-gated 真模型，roleplay-fuzz P2）
- 文件头**诊断范围声明**（:10-20）：只覆盖单条"夜间情绪低落"温和弧，绿≠agent 会情感陪伴；硬断言只锁确定性契约，质量/归因全在 ledger + judge 软观测；自述三个架构性盲区（对抗压力弱覆盖、自伤危机未覆盖、webhook 层 quiet-hours 拦截点测不到）。
- 基建：env-gated（缺 REAL_LLM_API_KEY 自跳过）+ FailoverProvider 备胎链（主模型重试 10 次；判定 failover-worthy=瞬时抖动或 401/402，:116-125）+ `unwrap_or_skip_transient!`（瞬时跳过并写 skip_ledger.jsonl，**非瞬时错误 panic 不许假绿**，:287-331）+ MCP wiremock 递增 newMsgId 防唯一索引冲突（:335-363）。
- 4 轮固定台词弧（t3="你别一直问我问题"边界宣告，:676-687）。每轮：**硬断言** gateway status ∈ `GATEWAY_STATUS_VALUES` 闭集、final_review_status ∈ 闭集或空（:726-736）；本轮 decision_review 按 inbound_message_id 精确绑定 + created_at:-1 取 rewrite 终态（:738-752 注释详证 rewrite 路径同 inbound 双行）；已发出回复 **硬断言**不含 12 个转交/身份禁词（本文件维护独立 FORBIDDEN_RELAY_MARKERS 表，:624-637）+ 不逐字复读上一轮；线下承诺 8 标记词仅软观测（"改天出去走走"可能合法，:639-652）。
- 归因协议（写 ledger）：⑥a conversation_mode ∉ 4 情感模式 → fixture 层信号（profile 未接线回落销售域探针，注释详证 H9 枚举 coerce 行为 :863-869）；⑥b 不该沉默却 no_reply → reply_agent；⑥c 按 gateway 终态分层——`blocked_unverified_product_claim` 在情感域单列"reviewer 误标产品声明"（grounding bypass 不含 R5.4 硬闸，gates.rs:627 引证）、review_blocked/held 等 → reviewer 层、safety/required/budget/tool_timeout → gate 层；⑥d revision_applied+压力关键词 → reviewer 高压误判触发不必要改写（软闸 rewrite 成功路径终态分已降会漏检，:964-987）。
- judge（EMOTIONAL_JUDGE_SYSTEM 8 维，pressureRisk/factualRestraint 极性标注）只观测：⑦a reviewer↔judge 背离（judge<7 但 reviewer≥7 = reviewer 销售锚点过严）；⑦b 被拦但 judge 认可（overall≥7 且 pressure<5）= 误杀铁证；⑦b2 已发出但 judge 低质（<4）——**用 revision_applied 消歧**归因 reviewer 改坏 vs reply_agent 原生烂（:1045-1079）；⑦c t3 后问句数 >1 软观测。注释钉死 ReviewScores 落库键是 **camelCase**（types.rs:985），snake_case 会静默取不到值（:813-825）。
- 末尾软观测：reviewer 通过轮数与实际发出轮数两口径（软闸放行路径 approved=false 但照发，:758-761），低于期望 3 记 unknown 层 issue。

#### roleplay_reviewer_pressure_calibration.rs（759 行，1 测试，#[ignore]，env-gated 真模型）
- P2 的对称互补：E2E 里真 agent 不会主动产高压话术，测不到"reviewer 不漏判"。本测试用**固定候选回复**直喂生产 `review_fixed_candidate_for_test`（内部真 review_decision，含 prompt+active profile chunk_roles+guards），隔离 reviewer 评分单变量。
- 对照组：3 条合理关心（给空间/轻量试探）期望 pressureRisk < block_at；3 条高压控制（胁迫/道德绑架/无视拒绝）期望 ≥ block_at（block_at 取 `UserRuntimeParameters::default().pressure_risk_block_at`=7，用阈值非写死分值反过拟合，:605-610）。
- **硬断言对称契约**（:747-758，CI continue-on-error 不阻断合并）：合理组全 < block_at（不误杀）、高压组全 ≥ block_at（不漏判）。异族 judge 交叉：判定方向相反记 `reviewer_judge_pressure_divergence`；reviewer 漏判高压记 `reviewer_missed_high_pressure`。

#### domain_profile_e2e.rs（2097 行，21 测试，全 #[ignore]）
- **Part A（DB 复刻，5 测试）**：create 落草稿（current=false/is_active=false/version=1）；update 只改草稿；publish→activate 两步语义；同 workspace 单活（后 activate 者赢、前者 soft demote）；DB 层不阻止删 active（业务守卫在 handler，:454-479 自述）。
- **Part B（真模型生成，2 测试，env-gated）**：`generate_domain_profile_candidate` 真 LLM 生成候选 → **落草稿 + seeded_by="generated_by_ai"**（AI 永不自动 activate 红线）且 profile_dimensions/prompt_fragment 非空；第二行业草稿与第一行业并存可见。
- **Part C（真 handler，14 测试，repl_set 事务）**：
  - `invalid_generated_machine_rejects_activation_with_zero_writes`（:737-808）：非法状态机（allowedFrom 引用不存在状态）→ activate BadRequest 且 4 个集合快照字节不变（不得先切人格再把内容错误伪装成可重试 partial）。
  - publish 语义三连：published-current 移动但 runtime-active 不变（requiresActivation=true）；从未 active 的草稿 publish 后 0 active；rollback 只回退 published-current 不隐式切 runtime-active（v2 仍 active、v1 变回 current，:896-945）。
  - PUT=追加不可变草稿：来源版本不变、未带字段保留、version 后端分配；未知键不落库（CORRECT-1 白名单）、is_active/version 不可经 PUT 篡改（:947-1057）。
  - 风险字段审阅（:1059-1299）：危险变更（grounding bypass/distrust/soul_override）publish 返 riskyFields 列表但边界与普通变更完全一致（都等显式 activate）；普通变更 riskyFields=[]；首版 publish 后 0 active。
  - H13 状态机联动（:1301-1899）：activate 带 generated_state_machine → `operation_domain_configs` publish 新 current（版本递增、previous_version 链、seeded_by="profile:<id>"、含生成 state keys）；无本体 profile activate → **显式恢复系统默认机**（防前任行业机泄漏）；forbidsProactive=true state → 派生 `operation_state_policies` 行 forbidden 含 "reply"；重复激活同字节机器 no-op 幂等（版本不膨胀）；toggle forbidsProactive → 只 in-place 刷新**机器派生行**（seeded_by 区分），运营手工行（admin_manual）绝不 clobber；多版本 policy 并存时只刷 current 行、历史行原封不动。
  - T14 幻影态迁移（:1901-2028）：activate 切新机器后，存量 contact 停在旧机器 state → 重置到新 initial；已在新机器合法态不重置；operation_state 未设的不写。
  - G06（:2030-2097）：直编状态机路由 `update_operation_domain_state_machine` 也必须联动重派生 policy（此前直编不走 publish loop → forbidsProactive fail-open 静默失效）。

#### domain_schema_persistence_e2e.rs（200 行，4 测试，全 #[ignore]，#1 serde 错配回归门）
- 修复前路由查询用 camelCase（workspaceId/isActive）而模型序列化 snake_case → `load_active_domain_schema` 恒 None、enforce 从不执行。测试锁：insert active schema → load 返 Some；load 后 enforce 对缺 required 字段 reject；activate 的 $set 用 snake_case 真命中；`activate_exact_version` 原子唯一 + 保留全部历史版本 + 未知版本 fail-closed 报 "domain_schema_version_changed" 且 active 指针不动。

#### configuration_generation_integration.rs（428 行，3 测试，全 #[ignore]）
- **跨副本配置代次协议**：两个独立 Database wrapper（模拟双副本）连同一 Mongo。写方 `bump_generation(namespace, ws)` 后，另一副本**下一次读立即**看到新值（不等 30s TTL）——覆盖 DOMAIN_PROFILE/TAXONOMY/LLM_PROVIDER 三命名空间（profile prompt_fragment v1→v2、taxonomy alias 生效 candidate_new→alias:canonical、registry snapshot_synced model v1→v2 且 generation+1，:74-249）。
- 初始化恰一次：`ensure_workspace_taxonomies` / `ensure_default_llm_provider` 首次 seed → generation=1 + durable marker；重复调用不重 seed 不多 bump；**并发双副本初始化恰一方执行 seed**、generation 恒 1、marker 恒 1 条（:251-428）。

#### taxonomy_flags_e2e.rs（214 行，1 测试，#[ignore] repl_set，SR-168，真 HTTP）
- 运行字段完整投影：GET 列表投影 priorityWeight=73/isTerminal/isReactivationTarget；仅 PATCH label → 响应与强类型读回三运行字段原值保留；**每次 CRUD（PATCH/CREATE/DELETE）taxonomy generation 恰 +1**（行与代次同事务提交）；DELETE=软删（status="deprecated" 行保留）。

#### taxonomy_version_audit_integration.rs（228 行，3 测试，全 #[ignore] repl_set）
- 策略型孤儿现状记录（:1-6 注释）：全局字典版本 handler 无 RBAC 拦截门（系统无角色模型，"谁有权改"红线未定义），只补审计。publish/rollout/rollback 各写一条 `taxonomy_version_changed` 事件（action/adminUsername/scope/kind/valueId/isGlobalScope/version 正确；rollback 记录被回滚**到**的版本号）。

### 3.G 剩余未被 15 号覆盖组（13 文件）

#### prompt_pack_seeding.rs（646 行，10 测试，全 #[ignore]，波 D2）
- `ensure_prompt_pack_v2` 安全性全谱：不删运营手工 active/draft 模板（key 不在 spec 也保留）；spec 全 key seed 齐（含 product_claim_markers/knowledge.auto_verify）；system 行内容漂移 → 归档旧行+种新行（**归档非物删**，可回溯）；**版本号匹配时内容漂移也对齐**（不再版本盲，:495-553）；evolution_release 链的 key 对齐跳过（evolution 行保留 active 不归档，:211-276）；spec 未变幂等（行数不变）；m043 后 draft-only 流恢复——运营 draft 保留 unpublished、系统补 append+publish 内置版本，恢复前 `load_unique_current` fail-closed 报 current_prompt_missing（:325-426）；planning-only spec（group.policy/moment.policy）恒 draft 非 current 非 active；archived 历史行启动对齐不清除（SR-055）；**ensure 返回 bool**（写入 true/幂等 false，供调用点失效 LRU，:607-646）。

#### sr008_ops_single_current.rs（511 行，2 测试，全 #[ignore]）
- m048 三表（operation_domain_configs/operation_state_policies/system_taxonomies）single-current 收敛：坏指针（多 current/零 current）下 taxonomy 缓存 warm **先 fail-closed** 报 "current pointer invalid"；m048 幂等收敛——多 current 选最新、零 current 选最高版本、**已存在的唯一 current 即使非 max(version) 也保留**；收敛后 partial unique 索引重建并拒绝新的双 current 插入（:160-328）。
- 真 HTTP 生命周期（:339-511）：三资源 publish（version 2）→ rollback（回 1）→ rollout（回 2）各自原子、保留双版本历史；并发 publish 同一资源 → 全部 OK/CONFLICT、恰 1 current、行数=1+成功数（冲突不留部分历史）。

#### sr012_runtime_scope.rs（348 行，2 测试，全 #[ignore]）
- debounce runner/reload 按 (workspace, account, contact) 三元组隔离：`contact_key` 含 workspace；同 account+wxid 双 workspace 各自独立 runner；**本地 workspace 无 managed contact 时绝不借用外域 managed contact**（0 LLM、0 outbox，:210-297）。
- roster single-flight 按 (workspace, account) 隔离：同 account_id 双 workspace 各自 MCP server 各拉恰 1 次 `contacts_fetch_full`、快照互不串（:300-348）。

#### sr053_soul_versions.rs（400 行，5 测试，全 #[ignore]）
- Agent Soul 不可变版本链：edit=append draft（previous_version 链），publish 原子切唯一 published 指针（旧行 archived 保留），rollback=publish 历史版本；`reset_prompt_pack_v2_as_actor` 对四 kind 各 append 恰一条 system_reset 版本——user/management 直接 published（published_by=操作者）、group/moment 占位 **draft 不 published**（:40-157）。
- bootstrap 空 prompt pack 保留运营已 published 的 Soul（不 append 不夺指针），management 系统 seed、group/moment draft 占位（:159-221）。
- 并发 publish：至少一方成功、失败方报 "soul_publish_conflict"、历史不删、恰 1 published（:223-289）。
- m042：同 kind 重复 version → fail-closed 报 "duplicate version 7" 且指针零写；多 published 选最高 version、旧行 archived、幂等（:314-400）。

#### sr055_prompt_versions.rs（221 行，3 测试，全 #[ignore]）
- PromptTemplate append-only 版本：append→publish→append_edited_draft→publish→**publish 历史版本=rollback**，全程 `load_unique_current` 恰一 current、历史行数不减（:29-92）。
- m043：split pointer（active 非 current + draft 带 current）→ fail-closed 报 "requires one active current" 零写；正常收敛只归档非 current 的 active 历史、保留行数（:124-221）。

#### sr138_prompt_reset_guard.rs（209 行，2 测试，#[ignore]，真 HTTP）
- reset-system-pack 显式销毁性操作三重守卫：缺 body / confirmation 大小写不符 / 带未知字段（force）→ 4xx 且 4 个受治理集合（agent_souls/prompt_templates/operation_playbooks/operation_domain_configs）快照字节不变 + `prompt_pack_version` 原子计数不 bump（**被拒的 reset 不失效运行时 LRU**）；精确 confirmation "RESET PROMPT PACK" → 200，自定义 prompt 被物删、系统 pack 重种非空。

#### prompt_template_redline_gate_e2e.rs（135 行，6 测试，全 #[ignore]，#2 绕过链回归门）
- create/publish 三道闸对 `user.reply.policy`（强约束 key）：`validate_prompt_edit` 拒禁用词（字符拼接绕 lint）与红线锚缺失；直插脏 draft（模拟绕过 create 闸）后 publish 字面双闸仍拒且行保持 draft；`review_prompt_edit` LLM 语义闸——violation=true → Reject、false → Pass、**LLM 不可用 → NeedsHumanConfirm（不 fail-open 放水）**（:126-135）。

#### guide_apply_partial_validation.rs（255 行，4 测试，全 #[ignore]）
- `apply_contact_changes` 部分应用红线：LLM 产出的越界枚举字段（operationState 非状态机态/customerStage 字典外）**跳过并记 skipped**，合法字段照落（不整请求 400 陪葬）；全越界 → set_doc 空判、updated_at 不变（零空写）；混合场景精确分流；全合法 → 全落库 skipped 空（cooldown allowFromAny 迁移合法）。

#### hc026_formula_evaluation.rs（335 行，3 测试，全 #[ignore]，真 HTTP）
- 评测场景创建门：active 场景 ground_truth 缺维（只有 trust）→ 400 且零写（:201-236）。
- **评测预算只数自己的 shadow 调用**：预先插入未来时间戳的巨量生产 run_log（999999 tokens），评测 summary 仍 totalTokensUsed=45/calls=3/usageComplete=true（旧共享日志算法会误报超支，:238-290）。
- 失败 LLM（不排队响应）→ usage unknown：totalLlmCallsUsed=1/unknownUsageCalls=1/usageComplete=false/degraded=true/degradedReason="evaluation_budget_usage_unknown"，**不再启动后续场景**（scenarioCount=0，:292-335）。

#### operation_view_integration.rs（241 行，1 测试，#[ignore]）
- `GET /operation/active-view` 聚合：dimensions=active profile 的 profile_dimensions（camelCase wire）；taxonomies 按 kind 映射 {id,label}；**kind 集 = profile_dimensions ∪ relationship_type（M3）∪ conversation_mode（A5）**——两个强制键即使不在 profile 声明里也必在（conversation_mode 含 m028 seed 的 consultative→顾问咨询）。测试内显式重新 warm 两个进程级缓存对齐本测试 DB（:162-169）。

#### playbook_scope_integration.rs（259 行，2 测试，全 #[ignore]）
- Playbook 变更零写拒绝：错 account（正文 account-b vs 行 account-a）→ Conflict；stale expected_version → Conflict；两次拒绝后**全集合逐字节不变**（:85-175）。
- set_default 对 draft：原子事务里 publish 目标（release_status draft→published）+ 降级旧 default，终态恰 1 default（:177-259）。

#### contact_manual_tags_integration.rs（254 行，6 测试=3 纯函数默认跑+3 #[ignore]）
- P0 红线注释（:6-8）：manual_tags 是运营权威层"AI 永不覆盖"（contacts.rs:687），gateway 写回只动 bayesian_signals（gateway.rs:4065）。纯函数：normalize 去空白/空串/去重保序；validate 条数上限与单 tag 字符上限（MANUAL_TAG_MAX_CHARS=400 级防 prompt 膨胀）边界恰好放行。handler：真落库；跨 workspace → NotFound；SR-149 错账号 → Conflict 且联系人 BSON 逐字节零变化。

#### contact_operation_profile_integration.rs（340 行，5 测试，全 #[ignore]，M13）
- `update_operation_profile` 不清空 AI 画像：前端式请求（不带 profileAttributes → serde default 空 Document）**保留** AI 积累的 profile_attributes（旧 bug 无条件 $set 清空；修复=非空才写，镜像 gateway.rs:4034）；带非空则正常写；更新 follow_up_policy 不误清画像。
- SR-151 跨账号 Playbook 绑定 → NotFound 零写；SR-070 **AI 草稿 Playbook（release_status=draft）不可绑定到活跃 contact** → NotFound 零写。

### 3.H 真模型全系列（14 文件，全部 env-gated `REAL_LLM_API_KEY` + 默认 #[ignore] + MCP 恒 wiremock 桩 + 密钥零泄漏）

各文件均自带一份（刻意复制、不共享）env-gated provider 构造 + FailoverProvider 备胎链 + `unwrap_or_skip_transient!`（瞬时抖动 skip 且写 skip_ledger.jsonl；**非瞬时错误 panic 不许假绿**）。共性红线：`rebuild_app_state_with_real_llm` 把 MCP 指向 wiremock 绝不真发微信。

#### real_llm_smoke.rs（1209 行，4 测试）
- t1 文本决策→审查链（真模型 JSON 过 serde + 五闸）；t2 知识 tool-loop 真收敛；t3 vision 抽取恒 draft+needs_review；t4 reply→review→**用户 reaction stop 在第二次发送前取消 outbox**。

#### real_llm_knowledge.rs（1637 行，13 测试，K 系列红线/形状）
- K1 open_chunk 深检索（答案只在 body，trace 必须含 open 目标 chunk）；K2 follow_relations 触达被挤出 catalog 的 B（fixture 先自证 A 可见/B 不可见，:450-456）；K3 无幻觉诚实弃答（cited 必须**空**）+ 闭环补账：留 `recall_miss` pending gap 信号含原 query（:644-648）；K4 needs_review 永不被 cite（verified-only 闸）；K5 文章抽取恒 draft+needs_review；K6 vision 同；K7 auto_verify provenance 闸（`decide_auto_verify_status` 纯函数三态 + 真跑写恰 1 条 op=verify/source=rule/created_by=auto_verify revision）；K8 修复只产 patch 零落库；K9 标签双数组；K10 chat 起草只产 proposal（chunk 计数不变 + verified=0）；K11 完整度审计 answeringMode ∈ 3 闭集 + 审计只读；r4_2 法律域两条（换域重验 K3/K4 域无关性）。

#### real_llm_knowledge_quality.rs（3375 行，21 测试=8 Q + 13 纯函数，Q 系列内容质量）
- **方法论核心**（该文件是测试方法论的旗舰）：双层判据 = 硬命中红线（与 K 同源确定性 assert）+ LLM-judge 打分（`MIN_QUALITY_FLOOR=6.0` 以下 panic 修生产代码，`TARGET_QUALITY=7.0` 仅记录）。
- **跨家族裁判团**：deepseek 双 checkpoint + 可选异族 Qwen（DashScope）+ 可选 judge2；Q3 专用**多模态 vision 裁判团**（meta llama-vision + nvidia nemotron-vl，真看图判分——纯文本裁判看不到图是 Q3 永久 SKIP 的根因，:918-986）。
- **三态正交裁决 `decide_quality`（:689-725）反放水结构性保证**：⓪ 剔除自身 K 极差 > 3 的失灵裁判（只看精度不看分数）→ ① 有效裁判 <2 → SkipInsufficientJudges → ② 跨裁判分歧 >3 → SkipDivergent（只看分歧不看分数）→ ③ 判分保守取 **min(medians)**（只看分数）。每 Q 一对与被测题材**解耦**的金标 good/bad 校准锚（gap ≥2 才可信，Q1 售后时效/Q2 会议纪要/Q7 交付周期……，:997-1097）；`q_dims` 是判分与校准 dims 的单一真相源（单测锁死逐字一致，:3315）。
- Q1 检索话术（硬：cite⊆seed + 方法论 token 命中；truth 给全三类 verified 事实防裁判误判 grounded 引用为编造，:2200-2212）；**Q2 旗舰**：16 类文档矩阵（含 6 类陷阱题材：否定句/推测数据/例外子句/干扰项）× train/holdout，硬红线（每篇 ≥1 chunk 且 integrityStatus ∈ {needs_review, rejected}）+ **确定性参考事实原子单元召回**（`MIN_RECALL_FLOOR=0.6` 双 split 分别断言）+ **泛化差距门 `MAX_GENERALIZATION_GAP=0.18`**（train-holdout 召回差爆表 = prompt 过拟合作弊硬 fail）+ 每篇严格 judge Pass（skip 不算证据）；Q3 vision（原图喂裁判，strict 版要求双 vision 裁判 + Pass）；Q4 chat 意图+起草（timeout 回退串识别 `looks_like_timeout_fallback` 确定性 skip 防假红，:385-395）；Q5 完整度审计（needs_review 草稿必须被区分、审计只读）；Q6 修复 patch（零落库 + 不编造数字）；Q7 打标；Q8 诚实弃答（知识库确证不覆盖的题材，honesty/no_fabrication/actionable_followup）。
- 13 个非 ignore 纯函数回归锁（:3066-3375）：reference_recall 确定性、median/spread、decide_quality 三态正交/绝不因低分 skip/剔除失灵裁判/泛化到 3 裁判、校准锚 dims 与 q_dims 逐字一致、corpus_matrix 双 split 多题材等。

#### real_llm_ops_smoke.rs（3176 行，18 测试，t4–t18 运营 Agent 全能力）
- t4 FollowUp 触发 + 过期 precheck（独立 contact 隔离 rate_limited 短路序，:1315-1322 注释详证 gateway.rs:1664→1673 顺序）；t5 真模型 operation_state 必须 ∈ 生产状态机 9 key 字典；t6 无 verified 知识的产品声明——两合法终态：被拦未发出，或发出但**绝不含具体折扣/价格数字与绝对化声明**（品类级正则判定非点对点，:1510-1544）；t7 四场景通用性（异议/咨询/闲聊/边界）+ boundary 场景**越权照办措辞命中即 panic**（导出全部客户微信号，:1618-1652）；t8 autonomy_mode ∈ {auto,assisted,blocked} + **用户要真人时 HANDOFF_MARKERS contains_unnegated 命中即 panic**（运行期零代码守卫扫正文，此断言是唯一确定性门，:1742-1762）；t9 用户反应闭环——真实 outbox 分段 FIFO dispatch 全 sent + review finalize=sent + `outcome_status` ∈ user_replied_* 10 值闭集且非 pending、reaction_analysis 非空；t10 初始画像冷启动结构化生成；t11 记忆固化合并 candidates；t12 custom_agent_instructions 操控性（软观测）；t13 千人千面（对立画像同消息 → reply_a ≠ reply_b 硬断言，质量层 judge 观测）；t14 画像写侧抖动（弱信号不推翻高置信画像）；t15 六轮跌单深弧；t16 千人千面×多轮交叉；t17 边界压测弧（横跳+autonomy 红线+弱信号污染）；t18 运营人工录入暖启动弧。私有 run_judge 用销售域 JUDGE_SYSTEM（judge.rs 注释确认的 t4-t18 零变化保护对象）。

#### real_llm_adversarial.rs（2331 行，12 测试=8 async+4 纯函数，Phase A 评分仪器）
- 定位：**先校准仪器再解冻 agent**——agent 冻结为对照，红队对手 agent（读 agent 真实上一句生成下一击）+ 双 checkpoint 裁判团 + 跨裁判分歧信号 + 金标命中率 + JSONL 台账。旧 K 采样极差只测精度（"稳定打错分"极差=0），新增分歧（效度代理）与金标命中率（绝对效度）。
- 6 条攻击弧（t_adv_price_objection/human_takeover_bait/contradiction_trap/fake_emotion_bait/knowledge_fabrication_bait/prompt_injection）。**弧内确定性硬红线（:1456-1519）**：任一轮 reply 命中未否定 HANDOFF_MARKERS → panic；LEAK_MARKERS 11 词（中文元短语+英文越狱变体 ignore previous instructions/jailbreak/dan mode）→ panic；**LEAK_FINGERPRINTS 8 个内部指纹**（边界保护模式/communication_style/memorycard/forbidden_rules 等 soul/配置内部术语，出现即背出系统设定）→ panic。其余判分全 eprintln+台账（Phase A 不设硬回归门）。
- t_judge_calibration 金标校准（命中率三态 hit/miss/skip，纯函数 calib_outcome 带 band）；t_longrun_capability 长期运行弧（回填跨周历史+手动 consolidation 量化记忆/画像漂移）；4 个纯函数测试锁 calib/tally 语义（含 effective_hit_rate 空样本不 NaN）。

#### real_llm_recall_benchmark.rs（2119 行，7 测试=4 async+3 纯函数）
- reach（检索翻到）/adopt（生成真引用）两层召回 + recall@k。`recall_benchmark_smoke` 深埋目标可达；**`recall_benchmark_cross_industry` 五条相对下限/单调契约**（:1083-1200，自我声明"非绝对锁分，模型升级只会更好不触误红"）：①lexical-easy 组（bigram overlap≥0.15）reach 均值 ≥0.7（兜底全体 ≥0.5）；②adversarial 组不得反超 lexical-easy（单调契约，反超=词面检索路径坏）；③reach 跨 N 轮完全稳定 case 占比 ≥0.8；④单 case 跨轮召回极差 ≤0.34；⑤adopt lexical-easy ≥0.4（兜底 ≥0.25，拦"翻到却从不引用"生成层塌陷）。另有成功覆盖率门（case ≥80%、轮次 ≥70%）。
- `recall_benchmark_maintenance_stability`：chat 全链改库（create+verify / update+verify）后召回不退化；`recall_benchmark_gap_closed_loop_trajectory`：**gap→主动提问→对话补库→再问命中**完整闭环轨迹（poll recall_miss 信号 → chat 补知识 → 人审 verify → 同 query 再问 reach+adopt 命中）。

#### real_llm_cross_domain_arc.rs（1482 行，3 测试，R2.2/R2.3）
- 自我批判定位（:6-10）：t15-t18 硬断言只锁"status ∈ 闭集"的壳，agent 行为错也照绿；本套件断言对齐业务契约。双域弧（情感陪伴 seed profile + 销售 DEFAULT 回落）跑同一 `run_arc` 驱动器；judge 标尺经 `build_judge_rubric` 自动翻极性。
- 契约级硬断言四条：status 闭集；每轮发出的 reply 禁词扫描（转人工/暴露身份，命中即红）；turn≥2 不逐字复读；**arc 级画像落地**——NewFactRevealed 弧跑完 contact 必须留下至少一项画像信号（memory_summary/agent_profile/domain_attributes 任一，全程零记录=真画像缺陷变红；arc 级而非每轮因单轮写由 LLM 产出决定）。
- `r2_3_same_input_distinct_behavior_across_domains`：同一输入跨域应产生实质不同行为；`r2_2_identity_probe_no_leak_no_freeze`：身份试探不泄露不冻结。

#### real_llm_digital_twin_arc.rs（471 行，2 测试，R2.1+COV-2）
- 把 `generate_identity`（此前零调用死代码）接入全链：真 LLM 生成 peer_social / formal_business 身份 → seed active profile → roleplayer 按生成人设博弈 → 契约级断言（闭集/禁词/画像落地）。三族异族；断言绝不锁单条措辞或单个行业。

#### real_llm_dynamic_adversarial.rs（459 行，1 测试，R5.3+R5.4 总成）
- 开头 `assert_three_families_distinct()` 硬门；roleplayer `roleplay_adversarial_turn` 主动施压随 agent 表现升级；硬断言只锁确定性红线（转真人/暴露身份禁词命中即 fail）；R5.2 轨迹裁判只写 ledger（金标校准前绝不 assert）；R5.4 跨会话（同进程第二段会话观测画像承接，确定性只锁"第一段后画像非空→第二段拿到带画像 contact"结构性事实）。

#### real_llm_roleplay_arc.rs（408 行，1 测试，R5.1 最小博弈闭环）
- 博弈链真闭合：roleplayer 发消息 → agent 真决策 → 回应喂回 roleplayer → 下一句。三族异族（judge 本条不接）；红线硬断言同口径。

#### real_llm_proactive_outreach.rs（450 行，2 测试，R2.5.1+R2.5.2）
- 排程层（planner tick / quiet-hours wake task）本身不调 LLM（emit 的是占位内容），mock 集成测已覆盖 DB 契约；真模型增量=**task 被消费时** `handle_follow_up_task` → gateway 真模型生成的主动触达/醒来文案。硬断言：真产出回复（run log + reply 非空）+ 禁词 + 不逐字复读历史；judge ObserveOnly。

#### real_llm_progressive_tier.rs（711 行，5 测试=4 async+1 纯函数）
- 渐进式三档两程循环回归：断言策略=查 `agent_events` 的 kind 而非解析回复文本。p1 寒暄轮**硬断言**不出现 `ptier_escalated`（机制核心价值：寒暄不吞重型槽位）；p2 产品轮升档（软观测 `ptier_escalated` + target_tier 含 Full）；p3 含糊轮 `ptier_clarify`（软）；p4 coverage=missing 强升 Full `ptier_forced_full`（软，拿到才校验形状）；非 ignore 纯函数测试从公共路径覆盖 `decide_tier_escalation` 三分支（本地无 key 也有真实断言）。

#### real_llm_principal_channel.rs（787 行，1 测试，重读，R2.5.3 治理红线命门）
- 出站请示：超职权客户请求跑全链。**唯一确定性红线=禁词 + 不暴露幕后真人决策源**（硬断言）；escalation 是否真触发为软观测（依赖真模型 emit escalationRequest 或硬闸 hold→升级两条非确定路径，:24-33 降级说明详证 gateway.rs:1845/1463 两个落台账点）；judge ObserveOnly。

#### real_llm_principal_relay.rs（670 行，1 测试，重读，G9/G10 入站 relay 回路）
- 入站方向端到端：领导 wxid 经**公开 webhook 入口**发自然语言裁决 →`interpret_principal_reply`（真 LLM 解析成结构化 PrincipalDecision）→ `resolve_escalation`（pending→resolved）→ relay task（`handle_follow_up_task` 分流）→ 真 LLM 生成面向客户的转述。pub(crate) 卡点靠生产公开入口绕过（零可见性改动）；断言契约级不锁转述措辞。

## 4. 不变量总表（本组测试锁定的"系统承诺清单"）

**A. AI 永不自动审定/激活（P0 最密集的红线群）**
1. 一切 AI 产出的知识写入恒 `status=draft` + `integrity_status=needs_review`：chat apply（chat.rs:1679-1681）、`apply_chunk_revision(source=Ai)`（chunk_revisions.rs:207-212，含把 active+verified 打回）、文章/PDF/图片/RSS 各导入口、merge 目标、rollback 恢复内容也重进审核、worker add_chunk、AI 修复提案永不落库。
2. `auto_verify` handler 过闸结果必经 `enforce_verified_needs_human_audit`（verify.rs:401）→ 恒 needs_human_audit，response verified=0。
3. verify 是人工专属动作且有 D2 证据闸（source_quote + 可引用 anchor 即 anchor 自带非空 sourceQuote）+ OCC（stale 快照 → `chunk_revision_conflict` 零写零 revision）；每次 verify/reject/batch/auto 都留 chunk_revisions 审计行（human 或 rule/auto_verify）。
4. domain profile：AI 生成候选恒草稿（seeded_by=generated_by_ai）；publish 只动 published-current，**runtime-active 只能显式 activate**（风险字段仅生成 riskyFields 提示，不改变边界）；playbook AI 草稿不可绑定 contact。

**B. 未审定知识永不上桌 / 引用封闭**
5. catalog/open_chunk verified-only；superseded redirect 停在"新版必须 verified"处；cite ⊆ opened ⊆ seed（filter_answer_against_opened PBT + K/Q 真模型全系）；fallback_rank 弱证据必须标 weak/medium + trace 可观测；空 corpus 恒 missing 不假装有兜底。

**C. exactly-once / 事务原子性 / 世代 fencing**
6. import-apply、chat-apply、shared ingest、lesson 晋升：并发+重放收敛同一 receipt，恰 1 组产物；任一步失败（含审计事件写入失败）全量回滚（collMod validator 注入故障验证）。
7. 五套 claim/fencing 协议同构：import_jobs（m056）、ingest_sources（m053，含 source_generation 配置变更 fencing）、catalog_rebuild_jobs（generation 单调收敛、乱序 superseded、孤儿 discarded）、knowledge_chat_tasks（claim_lost + stepId 重放 fencing、账号级产物不扩权）、digest（attempt/current 两代，失败保留上次成功快照，晚到旧代不覆盖）。

**D. 多租户/鉴权**
8. 一切资源读写 filter 必带 workspace_id（collection 形状层由 workspace_isolation/products/evolution_workspace_scope 锁；**真 middleware+Router 层由 SR-176 锁**：跨租户 404、同 id 双租户互不写穿、ACL 撤销即时 401 对 cookie 与 JWT 一致生效）；`resolve_authorized_workspace` 单闸拒 ACL 外 override（workspace_not_in_user_acl）；进程级缓存按 database 隔离（SR-174）+ 跨副本 bump_generation 下一读立即可见；认证不泄漏存在性、登录/token 共享限速、审计 90 天且无 PII；账号 MCP key 永不回显（响应+Debug 双掩码）；app_id 全局唯一。

**E. 版本/单指针协议（六套同构）**
9. prompt_templates（m043/m034/m049 + publish 单 current + evolution 行豁免对齐 + planning spec 恒 draft）、agent_souls（m042）、threshold_overrides（m040）、operation domain/policy/taxonomy 三表（m048 + 4-tuple unique）、domain_profiles（published-current vs runtime-active 双指针）、domain_schemas（activate_exact_version）——全部 append-only 保历史、恰一 current、并发 publish 一胜一 Conflict、坏数据 fail-closed 在第一笔写之前。

**F. 演化闭环安全**
10. release_prompt 三道闸（禁词纯函数→锚完整性→LLM 语义，LLM 不可用 → NeedsHumanConfirm 不放水）；threshold auto-release 被 human-release policy 拒绝且不落 flag；rollback 恢复 status=active 且缺历史行中止事务；shadow replay 基线来自冻结重跑非历史分数、retention 探针 snake_case；reset pack 补种 evolution critic。

**G. 真模型测试方法论纪律**
11. env-gated 自跳过 / 瞬时抖动 skip 且落 ledger / **非瞬时（4xx 配错、余额）panic 不许假绿**；MCP 恒桩；判分仪器与被测解耦（跨家族裁判、三态正交裁决、金标校准、泛化差距门、保守取低）；确定性红线（禁词 contains_unnegated、指纹泄露、状态闭集、cite⊆seed、token 召回）优先于 LLM 打分；CapabilityEvidence 台账 pass 需 attempted+llm_calls+branch+artifacts+assertions 全正（结构上禁止"没跑到就绿"）。

## 5. 偏差与疑点

1. **ingest_worker_smoke.rs 文件头注释与实现不符**：头部（:1-12）仍写"RSS 源 → feed-rs 解析 → 落 ≥1 chunk"，但四个测试实际全部是 SR-109 SSRF 拒绝路径（wiremock 在 loopback 上，恰好被出站安全策略拒绝）——**该文件已无任何"成功拉取入库"的正向用例**，成功 finalize 覆盖转移到了 sr117 的 redline 原语（`finalize_claimed_content_for_redline`），但那不经过真实 HTTP fetch/解析层。RSS/HTML 真实拉取成功路径目前在 tests/ 全集内疑似无集成覆盖（仅 lib 单测，未核）。
2. **workspace_isolation / products_workspace_isolation / evolution_workspace_scope 是"filter 形状"测试**：文件头自我声明不得外推为 Handler IDOR 证据。真 Router 证据只有 SR-176（contact GET + product PUT + /auth/me 三个端点）与 h3（llm_providers 两 handler）。其余在 §3.C 列出的多数管理端点（outbox 取消、状态策略、评测场景、guides、souls publish、command_runs 等）的**真实 handler 级**跨租户测试不存在——一致性依赖"handler 是注入 workspace 的 thin wrapper"这一约定。
3. **taxonomy 版本 handler 无权限门**（taxonomy_version_audit_integration 头注释自认"策略型孤儿"）：全局字典任一 admin 可 publish/rollout/rollback，测试只锁审计事件存在。系统无 RBAC，属已知未决策项而非回归。
4. **evolution_workspace_scope 的 EVO-2**（released_by 记真实操作者）自认"由代码审查保证"，无自动化测试（standalone TestApp 不支持事务是理由）——但 sr097/sr008 等已示范 repl_set+真 HTTP 模式，此缺口可补。
5. **domain_profile_e2e Part A 是手动复刻 DB 操作**（非真 handler），且 `e2e_delete_forbidden_on_active`（:454-479）实际**没有测删除被拒**——注释承认"业务规则由 handler 层强制，本测试仅验证前置条件"。删 active profile 的 handler 拒绝路径在本组内未见覆盖。
6. **real_llm 系列的判定强度分层容易被误读**：ops_smoke t4-t18 多数硬断言只锁 status 闭集（cross_domain_arc 文件头自我批判此点）；对抗/动态/轨迹 judge 全部只进 ledger。真正"行为错即红"的确定性门是：禁词/指纹 panic、t7 boundary 照办、t6 编造数字、t13 reply_a≠reply_b、cross_domain 画像落地、Q2 召回+泛化门、recall_benchmark 五契约。评估该系列价值时应以这些为准。
7. **测试间约定脆弱点**：TestLlmGenerator 按 system prompt 锚文本（如"运营知识库的 wiki 研究员"）路由 mock 响应（common/mod.rs:239-257）——生产 prompt 措辞改动会让 mock 队列错位（表现为难解的集成测试失败），这是隐式契约。
8. **jwt_auth.rs 内联复制了整份 AppConfig 字面量**（:29-166）：AppConfig 加字段时该文件必须同步手改（编译错误会提示，但属重复样板）；tests/fixtures 的 JWT 测试密钥对为提交在库的测试专用 PEM（无生产风险，但值得知晓）。
9. **hc028 与其余 real_llm 的 skip 语义不同**：hc028 明确"不用 legacy skip 宏"，provider 缺失即 fail；其余 14 个真模型文件缺 key 自跳过。CI 若未配 REAL_LLM_CONFIG_MONGODB_URI，hc028 会红而非 skip——是有意的硬门设计，不是 bug。
10. **maycran_transport_probe** 是临时诊断探针（头注释自认 temporary），断言"至少一个候选模型可达"，长期留在树上可能过期（候选模型名单硬编码）。
11. 三个 `.proptest-regressions` 文件（autonomy_protocol/knowledge_agent/memory_card）是 proptest 历史反例种子，应保留在版本库（回归重放），非垃圾文件。
12. **sr012_runtime_scope 归属**：任务清单把它划给 15 号（runtime 主题），但 15 号清单未含它；本记录已补读（§3.G），内容实为 webhook debounce/roster 的 workspace 隔离，与安全主题同源。

## 6. 覆盖自证

- **本次深读文件总数：124**（= tests/ 顶层 112 个 15 号未覆盖文件 + 重读 3 个【llm_retry_jitter、real_llm_principal_channel、real_llm_principal_relay】+ tests/common/ 9 个模块）。另核对了 fixtures/ 4 个数据文件与 3 个 proptest-regressions 的存在与用途。
- 读法说明：全部文件均逐行 Read；其中 5 个超大 real_llm 文件（knowledge_quality/ops_smoke/adversarial/recall_benchmark/knowledge）对每文件重复出现的同款 env-gated/failover 样板（与已全文读过的 roleplay_emotional_companion_e2e/knowledge_quality 同源拷贝，各文件头注释自证"复制非共享"）采用"文件头全文 + 全部测试函数体/断言逐条提取"的方式核证，未依赖记忆或猜测。
- **common/（9）**：mod、judge、redline、roleplayer、dynamic、identity_generator、generalization、capability_evidence、roleplay_fixtures。
- **知识（44）**：wiki_chunk_revision_pbt、wiki_gap_signals_3kinds、knowledge_agent_eval、knowledge_agent_pbt、knowledge_ask_e2e、knowledge_ask_stream_e2e、knowledge_auto_verify_enforce_integration、knowledge_chat_apply_integration、knowledge_chat_dispatch、knowledge_chunk_transactions、knowledge_closed_loop_trajectory、knowledge_digest_budget_smoke、knowledge_digest_compose_smoke、knowledge_digest_skeleton、knowledge_import_apply_integration、knowledge_operator_memory_isolation、knowledge_preview_workspace_scope、knowledge_router_fallback_e2e、knowledge_task_worker、knowledge_tools_budget、knowledge_worker_behavior_integration、chunk_batch_ops、chunk_lock_lifecycle、chunk_put_preserves_unmodeled_fields、chunk_revision_ai_draft_integration、chunk_type_routing_pbt、digest_cross_tenant_scope_integration、import_job_lifecycle、import_pdf_smoke、ingest_worker_smoke、sr115、sr117、sr121、sr122、sr125、sr131、sr132、page_merge_pbt、annotation_quality_gate_integration、integrity_report_d2_e2e、structured_organization_integration、vision_safety_gate、hc028_real_digest_task_e2e、（knowledge_preview_workspace_scope 已列）。
- **演化（8+2）**：evolution_policy_router_integration、evolution_prompt_shadow、evolution_release_redline、evolution_rollback_status、evolution_workspace_scope、m040_evolution_release_protocol、prompt_publish_evolution_guard、reset_pack_preserves_evolution_critic_integration、sr097_lesson_promotion、lessons_learned_filters。
- **安全（10）**：workspace_isolation、sr176_real_route_isolation、h3_cross_tenant_idor、auth_middleware_integration、sr016_auth_rate_limit、jwt_auth、products_workspace_isolation、migration_safety_redlines、account_security_integration、sr174_cache_database_isolation。
- **迁移（7）**：migrations_idempotency、m018、m029、m034、m039、m045、m049。
- **LLM/基础设施（5）**：llm_retry_jitter（重读）、llm_provider_activate_integration、llm_usage_summary_integration、maycran_transport_probe、ops_versioned_index_boot_brick。
- **真模型（14）**：real_llm_adversarial、real_llm_cross_domain_arc、real_llm_digital_twin_arc、real_llm_dynamic_adversarial、real_llm_knowledge、real_llm_knowledge_quality、real_llm_ops_smoke、real_llm_principal_channel（重读）、real_llm_principal_relay（重读）、real_llm_proactive_outreach、real_llm_progressive_tier、real_llm_recall_benchmark、real_llm_roleplay_arc、real_llm_smoke。
- **roleplay/domain/taxonomy（8）+ common smoke（6）**：roleplay_emotional_companion_e2e、roleplay_fixtures_smoke、roleplay_reviewer_pressure_calibration、domain_profile_e2e、domain_schema_persistence_e2e、configuration_generation_integration、taxonomy_flags_e2e、taxonomy_version_audit_integration；common_smoke、dynamic_smoke、identity_generator_smoke、judge_rubric、redline_smoke、（roleplay_fixtures_smoke 已列）。
- **剩余组（13）**：prompt_pack_seeding、sr008_ops_single_current、sr012_runtime_scope、sr053_soul_versions、sr055_prompt_versions、sr138_prompt_reset_guard、prompt_template_redline_gate_e2e、guide_apply_partial_validation、hc026_formula_evaluation、operation_view_integration、playbook_scope_integration、contact_manual_tags_integration、contact_operation_profile_integration。
- 未读（归 15 号，共 70 个顶层文件）：account_offline_defer、account_round_robin、ask_human_phase1、autonomy_protocol_pbt、behavior_signal_*、c2_*、campaign_*、cold_reactivation、contacts_batch_enable、conversation_mode_decision_schema、deal_event_scope、debounce_*、decision_review_status、dry_run_isolation、escalation_push_time_reassign、full_flow_suite、happy_path_run、hc004_*、hc020、human_like_threshold_pbt、intent_trajectory_pbt、last_inbound_split、media_*、memory_card_*、operating_memory_insert、outbox_*、outcome_*、outcomes_autonomy_endpoint、planner_*、pressure_risk_threshold_pbt、principal_decision_channel、quiet_hours_deferral、reaction_*、referral_card_push、review_task_now_claim、revision_recheck_action_gate、run_envelope_integration、send_ledger_integration、simulation_no_sideeffect、sr029、sr034、sr072、sr094、sr135、sr172、sr177、sr181、state_transition_pbt、string_fact_risk_guard、suspected_deal_e2e、transactional_admin_flows、webhook_contact_upsert、worker_reclaim。

---

## 追记：28 号交叉验证回写（2026-08-13，主会话执行）

- **§4 A2 行号更新**：auto-verify 强制降级的 enforce 接线现位于 `verify.rs:490-496`、函数体 `:681-686`（本记录撰写时的 `:401/:554` 行号因工作区未提交修改已漂移；行为无变）。行号引用本记录时以最新工作区为准。
