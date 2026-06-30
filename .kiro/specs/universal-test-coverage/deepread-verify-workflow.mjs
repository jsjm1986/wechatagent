export const meta = {
  name: 'biz-logic-deepread-verify',
  description: '逐业务域深读+对抗证伪,派生上线前测试矩阵(≤5并发分批+每agent3次重试容错,单域失败不阻断)',
  phases: [
    { title: '按域深读', detail: '每域读代码派生设计行为/红线/现有测试可信度/孤儿行为' },
    { title: '对抗证伪', detail: '每域结论交独立agent grep实证,推翻夸大结论' },
  ],
}

// 业务域全集(47域,内联常量;沙箱不支持 import/require/fs)。
// 权威副本 .kiro/specs/universal-test-coverage/biz-domains-2026-06-30.json,改动两处同步。
const ALL_DOMAINS = [
  {"id":"A1","name":"Webhook入站主链路","newish":false,"loop_type":"e2e_closed_loop","entry":"src/webhooks.rs:295 wechat_webhook","core_rule":"HMAC常时间比对→控制事件(testMsg/Offline/Online)短路→appId解account(未注册400不静默回退)→per-account令牌桶限流→领导回复分流请示通道→入站类型解析+媒体引用提取→11000原子去重落库→contact upsert+T1行为信号→managed门(仅managed回复)→作息时段defer跟进任务","redlines":["未注册appId明确400不回退default","仅agent_status=managed自动回复,normal仅持久化","HMAC失败400不泄露原因"],"focus":"签名校验/去重幂等/managed门/作息defer是否真按设计;非文本入站不崩;领导消息不被当客户入站"},
  {"id":"A2","name":"去抖聚合调度","newish":true,"loop_type":"e2e_closed_loop","entry":"src/webhooks.rs:614 register_inbound / run_debounce_pipeline:130","core_rule":"按联系人单runner(PENDING DashMap)天然串行;每条入站刷新deadline+generation+1;首条entry在shard锁内原子决定spawn防double-spawn;runner运行期generation变了即barge_in抢占重算;退休用remove_if谓词复核generation;runner panic捕获后移除state下条重spawn","redlines":["进程内DashMap,串行仅单副本成立","边界期到达消息不丢"],"focus":"连发消息聚合一次回/抢占重算/不丢消息;这是新机制旧测试很可能没覆盖,重点查覆盖gap"},
  {"id":"A3","name":"统一发送网关(决策→评审→发送编排)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/gateway.rs:120,574 run_user_operation_gateway","core_rule":"所有发送同一通道:reload context→RunBudget注入→resolve_thresholds写回runtime→precheck门(cooldown/min-interval/日上限/expiry/作息)→知识路由→Reply Agent→独立Review→可选一次revision→finalize五闸→state_action门→outbox→MCP;should_abort_send guard让过时生成放弃","redlines":["任何发送都必须走网关,绕过是bug","webhook自动回复与follow-up任务同一网关"],"focus":"全链路编排顺序/precheck各门/finalize硬闸强制should_reply=false/单次revision;查端到端集成测试是否真断言每道门"},
  {"id":"A4","name":"Reply Agent决策(渐进式三档)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/decision.rs:48 / gateway.rs:1088 decide_reply_with_promote","core_rule":"第一程Lean瘦档自评sufficiency→decide_tier_escalation决定进闸/升Full重生成/澄清;自评enough但coverage=missing且需知识→强制升Full(最多一次);仅经Full知识档才记used_knowledge_ids防架空grounding;PROGRESSIVE_TIER_ENABLED关→退单程Full;build_initial_operation_profile据运营备注生成初始画像注入active DomainProfile","redlines":["非销售域不被销售schema框住,DEFAULT域prompt_fragment=None字节等价","used_knowledge_ids仅Full档记防架空grounding"],"focus":"升档时机(已知盲区:客户索取业务动作时自评enough停Lean看不到素材/escalation清单);澄清触发;查③⑤域biz-test覆盖"},
  {"id":"A5","name":"独立Review Agent+双闸","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/review/mod.rs:1 review_decision / gates.rs review_passed","core_rule":"决策后跑独立Review Agent(异于决策评分:HumanLike/Emotional/FactRisk/Pressure/ProductAccuracy+grounding/style);reviewer输入只暴露候选事实面遮罩reply-agent自洽推理;route_dual_gate区分软闸(needs_revision重写一次)与硬闸(hallucination/grounding写非空rewrite_instruction);review_passed收敛单一approved布尔;预算超额降级local_decision_review","redlines":["reviewer不喂reply-agent自洽推理字段防追认幻觉","硬闸失败不触发single-shot revision","独立Review未通过不发送"],"focus":"reviewer独立性/软硬闸分流/预算降级;查reviewer是否真遮罩、是否会误杀合理回复或漏判高压"},
  {"id":"A6","name":"知识路由(catalog→search→open_slice)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/knowledge_router.rs:35 / gateway.rs:1057 route_operation_knowledge","core_rule":"每轮先跑知识路由(含硬关键词快路径)再让Reply Agent single-pass决策;load_operation_knowledge仅取integrity_status=verified chunk;Knowledge Tool Planner LLM规划打开哪些文档/切片渐进披露非全塞;预算超额降级empty_knowledge_route标degraded;maybe_emit_unverified_warning当日去重","redlines":["仅verified chunk进决策","tool_call预算超限抛BudgetError降级"],"focus":"渐进披露三层/仅verified入决策/预算降级;查router_fallback与tools_budget集成测试可信度"},
  {"id":"A7","name":"Outbox发送+幂等+崩溃恢复","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/outbox.rs:166 enqueue / outbox_dispatcher.rs:1021 tick","core_rule":"approved决策必先写outbox(idempotency_key=sha256(source_event:contact:content_hash))再调MCP;dispatcher 5s轮询reclaim过期租约(reclaimed_in_flight标记)→atomic_claim_pending原子抢占→二次安全门(cooldown/stop/30min stale)→撤管状态门→账号掉线/节奏defer不耗attempt→MCP 30s timeout→post-hoc核对mcp_call_logs防重发;用户拒绝/cooldown取消pending","redlines":["幂等键先于MCP发送","撤管即停(contact_status_changed_unmanaged)","MCP成功后DB失败只记审计不返Err防重发","post-hoc核对防超时重复发送"],"focus":"幂等/崩溃恢复reclaim/二次安全门/防重发;查outbox与send_ledger集成测试是否真验幂等与防重发"},
  {"id":"A8","name":"Follow-up任务worker","newish":false,"loop_type":"e2e_closed_loop","entry":"src/main.rs:204 → src/tasks.rs:12 run_task_worker / gateway.rs:136 handle_follow_up_task","core_rule":"spawn_supervised常驻无gating;每TASK_WORKER_INTERVAL_SECONDS(默认30s)tick拉取并执行follow-up任务,lease claim+stale回收(HP-1),到期任务走同一网关;tick失败仅log不退出循环","redlines":["到期任务走同一网关(precheck跳过冷却类门)"],"focus":"lease claim并发安全/stale回收/到期走网关;查worker_reclaim测试"},
  {"id":"B1","name":"长期记忆固化(memoryCard)","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/memory.rs:191 / 路由POST /contacts/:id/memory-consolidation/run","core_rule":"候选→consolidated闭环+OCC版本;compact_memory_card_with_previous保证core(上限6)/recent(上限10)截留与上版合并(未discarded保留,discarded显式移除);非原子fact检测→重试→丢弃;同维裁决(改口旧值退场);coreFacts须兼容旧Vec<String>","redlines":["coreFacts兼容旧Vec<String>反序列化(R11)","memory_write_score不进决策门(只ledger)"],"focus":"已知缺口:改口后旧事实未弃用(LLM不填deprecatedFacts);非原子检测;同维裁决;查⑨域biz-test与memory_card_invariants PBT"},
  {"id":"B2","name":"用户反应分析(claim lock)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/reaction.rs:28 record_user_reaction","core_rule":"对最新入站异步分析(购买信号/反对/停止/不分类),atomic claim防并发webhook重复触发,reclaim_stuck重置卡死review;reaction_outcome_status纯函数优先取模型outcomeStatus再用结构化flags(无关键词);整路进RUN_BUDGET,超额降级user_replied_unclassified","redlines":["停止意图必判stop_requested(触发outbox取消)","无关键词判定靠语义"],"focus":"停止/购买信号判定/claim lock防并发;查⑧域biz-test与reaction_claim_lock测试"},
  {"id":"B3","name":"决策请示通道(幕后领导,非人工接管)","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/escalation/mod.rs:40 escalate_held_decision / policy.rs:21","core_rule":"Agent撞决策墙(超职权/高风险/多轮卡死)向幕后真人请示拿裁决后用AI口吻转述;hold路径只推领导卡+落pending台账+写awaiting,不向客户发消息(安抚由网关ensure_customer_acknowledged解耦);骚扰门push_allowed(daily_cap/dedupe_window/quiet_hours);超时next_decider_on_timeout转链;relay经出站守卫(leaks_internal_payload/introduces_unauthorized_number)+sanitize_verdict","redlines":["客户永远只跟AI对话,真人绝不直接面对客户","relay不泄露内部payload/不引入未授权号码","fallback话术不含转接类措辞"],"focus":"已知缺口:LLM软表达'向上反馈'却不emit结构化escalationRequest.needed;查⑥域biz-test与principal_decision_channel/relay测试"},
  {"id":"B4","name":"名片引荐(辅助模式受控例外)","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/referral.rs:17 assist_mode_active / review/mod.rs REVIEWER_ASSIST_YIELD_NOTE","core_rule":"辅助模式(账号级默认关)受控例外:AI识别契合客户(签约/到店/深度对接)主动推真人顾问名片,AI退辅助;assist_mode_active客户级override>账号级enabled>默认关;validate_card_sendable仅enabled+approved可发;filter_referral_candidates按target_stages过滤;REVIEWER_ASSIST_YIELD_NOTE仅assist开账号注入(关账号字节等价)消解hold与产品准确度两路径","redlines":["assist关绝不发卡(全自治红线)","关账号字节等价","引荐≠全自治红线让步,对话始终AI在说","台前顾问≠幕后决策源"],"focus":"assist双路径(关不发/开识别推卡)/reviewer让位;查④域biz-test与referral_card_push测试"},
  {"id":"B5","name":"双层标签字典+候选(taxonomy)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/taxonomy.rs:1 check_value / upsert_candidate","core_rule":"严格字典层system_taxonomies按(scope,kind)任意维度枚举,候选层taxonomy_candidates承接非字典取值由后台审核并入;check_value纯函数返回Active/AliasActive(改写canonical)/Deprecated(加risk)/CandidateNew(加risk+异步upsert);候选SHALL NOT阻塞Reply Agent;TaxonomyCache进程级TTL 30s","redlines":["customer_stage/intent_level/objection_type必来自system_taxonomies","未审候选不得阻断run","AI永不覆盖manual运营权威标签"],"focus":"字典命中分类/候选不阻断/缓存失效;查taxonomy_flags测试与tag_trust改造"},
  {"id":"B6","name":"决策守卫+状态机迁移(guards)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/guards.rs:15 normalize_decision_state / check_state_transition:156","core_rule":"旧销售域5闸(fact/pressure/product_accuracy/human_like/emotional)已2026-05-25删除,方法论切wiki+3闸;剩状态机字典对齐纯函数:normalize归一状态名;check_state_transition校验合法性(读initial/allowFromAny/allowedFrom行业无关);fail-closed拒未配状态机/未知目标态;fail-soft非法迁移不阻断已发回复只跳operation_state写;enforce_state_action_policy行校验forbidden/allowlist","redlines":["状态机空fail-closed","未知目标态fail-closed防幻影态旁路policy","Agent不得发明新state key","非法迁移fail-soft不阻断已发回复"],"focus":"状态机迁移合法性/fail-closed vs fail-soft边界/action策略门;查state_transition_pbt与c2_operation_state测试"},
  {"id":"B7","name":"RunBudget预算计数","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/budget.rs:1 RunBudget","core_rule":"task_local注入RunBudget到run子future;generate_agent_json自动累加token与调用次数;is_exceeded进降级路径(跳review/跳rewrite/跳二次知识路由);record_tool_call原子检查超额抛BudgetError(ToolCallsExceeded/TokensExceeded对应R4.3);tool_call_budget由knowledge_max_tool_calls(默认6 clamp[1,16])注入","redlines":["预算超限绝不对webhook返5xx,须fail-soft","两类硬上限(tokens/tool_calls)"],"focus":"预算超限降级路径/不返5xx/工具调用上限;查tools_budget测试"},
  {"id":"B8","name":"Shadow模拟演练","newish":false,"loop_type":"e2e_closed_loop","entry":"src/agent/simulation.rs:38 simulate_user_dialogue / 路由POST /user-operations/simulations/dialogue","core_rule":"运营人员不真实发消息地演练完整Reply Agent链路:复用真实decide_reply/route_knowledge/review_decision但发送阶段只输出would_send;每轮打包UserOperationSimulationTurn;经precheck_send_gateway走同样前置检查;起独立RunBudget(simulation_token_budget)","redlines":["发送阶段只would_send零真实副作用","复用生产决策链路不另起一套"],"focus":"影子零副作用/复用真实链路;查dry_run_isolation测试"},
  {"id":"C1","name":"campaign营销活动派发","newish":true,"loop_type":"e2e_closed_loop","entry":"src/routes/campaigns.rs:281 dispatch_campaign","core_rule":"create/preview/dispatch/sends_report/list五端点;dispatch重跑圈人防漂移,命中0人返BadRequest;活动级去重靠campaign_sends唯一索引(campaignId,contactWxid)DuplicateKey跳过;每命中一人建follow-up task(注入intent_text)回填taskId;status经assert_campaign_status_valid闭集;classify_send_outcome归桶sent/pending/blocked/canceled/escalated/skipped;真实推送走统一gateway","redlines":["命中0人拒绝派发","去重唯一索引防重复推送","推送走统一gateway不绕过","status闭集"],"focus":"端到端空白区!biz-test零覆盖;圈人去重/分类结果/走网关;重点查有无任何集成测试"},
  {"id":"C2","name":"evolution prompt自进化","newish":true,"loop_type":"e2e_closed_loop","entry":"src/evolution/mod.rs:80 run_one_tick / release.rs:198 release_prompt","core_rule":"信封→runtime_flag灰度分桶(hash%100<rollout,读失败按disabled)→cohort过滤→threshold候选(纯统计不消预算)→prompt_critic候选(消EvolutionBudget)→replay影子重放(prompt走prompt_shadow真采样,新旧唯一变量推5闸)→significance定级→awaiting_admin;release_prompt仅eligible可发经三红线闸(禁词+锚点完整性原文逐字保留+LLM语义审查review_prompt_edit);LLM不可用fail-closed中止release;阈值通道resolve_thresholds在gateway真消费","redlines":["演化器与主链路物理隔离(R9.4 CI lint)","Critic prompt永不进演化循环(R10.1)","放松安全闸零容忍回归门(EVOLUTION_MAX_SAFETY_REGRESSION_RATE=0.0)","release经三红线闸,LLM不可用fail-closed不放水","prompt通道placeholder/半实装边界"],"focus":"端到端空白区!biz-test零覆盖;阈值通道真闭环vs prompt通道半实装;三红线闸;查evolution_*集成测试与隔离lint可信度"},
  {"id":"C3","name":"content-assets分档注入","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/decision.rs:1483 load_context_assets / asset_visible_at_tier:1458","core_rule":"按PromptTier(Lean<Relational<Full)分档注入文本资产:当前档序>=资产min_inject_tier序时可见,min_tier=None/非法→按full(仅Full可见);visible_min_tiers_for派生$in下推保单值/集合语义一致;Full档额外纳入min_inject_tier缺失老数据;query过滤workspace+account+kind∈{text,faq,script,brand_voice,forbidden_expression};load_sendable_assets取sendable+approved媒体limit30","redlines":["min_tier缺失按full(等价改造前)","sendable+approved才可发"],"focus":"分档可见性/$in语义一致/老数据兼容;仅③域边缘触及,查分档注入完整覆盖gap"},
  {"id":"C4","name":"digital-twin关系类型建议","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/gateway.rs:4943 extract_relationship_type_suggestion / admin_relationship_suggestions.rs:126","core_rule":"决策后提kind=relationship_type弱信号经dimension_registry::validate_dimension_value(MachineWrite)校验:臆造非字典值Reject/Drop不污染队列(canonical customer/peer/friend);合法upsert建议表锚(workspace,contact,status=pending)已审不复活$inc occurrences;fail-soft;approve仅pending可批AdminWrite校验越界恒Reject,先写contact.domain_attributes.relationship_type再mark approved","redlines":["臆造非字典值Drop不污染审核队列","已审不复活(幂等)","稳定属性不每轮臆测只新证据报","审核才回写contact"],"focus":"LLM识别→字典校验→审核回写闭环;查relationship建议集成测试"},
  {"id":"C5","name":"suspected-deals疑似成交核实","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/gateway.rs:4958 extract_suspected_deal_signal / admin_suspected_deals.rs:139 approve","core_rule":"决策后提kind=suspected_deal信号upsert待核实专表status=pending锚(workspace,contact,status=pending)已审不复活$inc;render_suspected_deal_guidance仅本workspace有active产品时注入引导弱信号通道+主动求证;approve CAS-first先原子CAS pending→approved(matched==0即并发/已审/跨ws挡防双计)再落正式成交verification强制staff_confirmed;reject须非空reason","redlines":["AI永不直写outcome_events(红线§2.1)","CAS-first防append-only财务双计","verification强制staff_confirmed","失败留漏登假阴不双计假阳"],"focus":"弱信号→CAS核实→落成交;AI永不直写outcome是硬红线;查suspected_deal_e2e测试"},
  {"id":"C6","name":"account send-pacing节流","newish":true,"loop_type":"e2e_closed_loop","entry":"src/agent/outbox_dispatcher.rs:719 / pacing.rs:15 account_send_interval_ms","core_rule":"发送前账号级最小间隔闸:account_last_sent_at_ms查上次实发,now-last<随机间隔则defer_account_pacing reschedule;间隔由纯函数把fastrand抖动线性映射[min,max](clamp[0,1],max<min退min);位置在reclaim幂等门之后发送之前;查询失败fail-soft放行(宁漏限不丢消息)","redlines":["fail-soft查询失败放行宁漏限不丢消息","位置在幂等门之后不误拦post-hoc标sent"],"focus":"随机间隔拟人化/fail-soft;pacing.rs有5纯函数单测,查DB路径(defer)覆盖"},
  {"id":"D1","name":"知识导入(import/PDF/image vision)","newish":false,"loop_type":"e2e_closed_loop","entry":"src/routes/.../import.rs / 路由import-preview|import-apply|-pdf|-image","core_rule":"import-preview LLM析出chunk/sourceQuote/forbiddenClaims不落库;import-apply/pdf/image一律落draft+integrity_status=needs_review;PDF多模态/image视觉模型识图","redlines":["所有AI导入路径一律draft+needs_review,AI永不自动verify","vision安全门"],"focus":"导入红线(永不自动verify)/三入口字节一致;查import_pdf/vision_safety_gate与①域biz-test"},
  {"id":"D2","name":"知识chunk修复闭环","newish":true,"loop_type":"e2e_closed_loop","entry":"路由/operation-knowledge/chunks/:id/repair[/answer]|patch|rollback / chunk_revisions.rs","core_rule":"chunk修复propose仍走review;patch写revision+广播事件总线;rollback回滚历史revision;apply_chunk_revision与chunks双写先写一行;PUT不清空provenance/locked_fields/created_at;ProvenanceSource::Ai强制draft+needs_review","redlines":["AI修复propose仍走review不自动verify","PUT不清空provenance/locked_fields","chunk_revisions不可变审计历史"],"focus":"修复闭环propose→review/revision审计/PUT字段保护;查wiki_chunk_revision PBT与knowledge_closed_loop测试"},
  {"id":"D3","name":"知识对话工作台(chat_turn→proposal)","newish":true,"loop_type":"e2e_closed_loop","entry":"路由/operation-knowledge/chat[+/:sid/apply|discard] / knowledge/chat.rs","core_rule":"chat_turn真模型意图分类+切片起草只产proposal永不自动落库;两步preview→apply落draft chunk回填createdChunkId;discard丢弃草稿","redlines":["chat只产proposal永不自动落库","apply落draft+needs_review"],"focus":"对话起草两步preview→apply/永不自动落库;查chat_dispatch测试"},
  {"id":"D4","name":"知识缺口信号+sweep","newish":false,"loop_type":"e2e_closed_loop","entry":"路由/knowledge/gap-signals[+/:id/dismiss|apply|sweep]","core_rule":"structural+semantic lint待办;两阶段sweep状态pending→auto_resolved/llm_resolved/applied/dismissed;apply生成补缺动作","redlines":["缺口信号不自动改chunk(apply才动作)"],"focus":"缺口lint/两阶段sweep/apply闭环;查digest与gap_signals测试"},
  {"id":"D5","name":"知识日报digest","newish":true,"loop_type":"e2e_closed_loop","entry":"src/main.rs:247 → knowledge_digest/mod.rs:37 worker_loop(KNOWLEDGE_DIGEST_ENABLED默认关) / 路由/knowledge/digest/today","core_rule":"worker默认关;开启后每天RUN_HOUR整点扫数据源合成卡片;today读/regenerate重生成/cards dismiss忽略","redlines":["worker gated默认关"],"focus":"日报合成/卡片来源;worker默认关须确认flag,查digest测试"},
  {"id":"D6","name":"知识完整度审计+自动核验","newish":false,"loop_type":"e2e_closed_loop","entry":"路由/operation-knowledge/completeness|auto-verify","core_rule":"completeness LLM完整度审计answeringMode闭集绝不触发auto-verify;auto-verify批量自动核验强制人审抽样下限5%,product_fact全量人审,禁100%无人审clamp","redlines":["completeness绝不auto-verify","auto-verify强制抽样人审下限5%","product_fact全量人审禁100%无人审"],"focus":"完整度审计不触发verify/自动核验抽样下限红线;查⑬域biz-test与annotation_quality_gate"},
  {"id":"D7","name":"知识问答(ask/stream)","newish":false,"loop_type":"e2e_closed_loop","entry":"路由/knowledge/ask|ask/stream / knowledge_agent","core_rule":"基于已核验知识问答产品声明须grounding;SSE流式;无依据保守降级","redlines":["产品声明须grounding","仅verified知识"],"focus":"grounding/流式/保守降级;查knowledge_agent/ask与real_llm_knowledge K套件"},
  {"id":"D8","name":"自动采集ingest worker","newish":false,"loop_type":"worker_gated","entry":"src/main.rs:296 → ingest_worker.rs:29(INGEST_WORKER_ENABLED默认关,双层gating)","core_rule":"双层gating(spawn位点bool+内部interval==0);开启后跨workspace扫active IngestSource→条件GET→解析→ingest_chunked_text落chunks draft+needs_review","redlines":["AI采集落draft+needs_review永不自动verify","双层gated默认关"],"focus":"RSS/HTML采集落库红线;worker默认关须确认flag,查ingest_worker测试"},
  {"id":"D9","name":"知识对话长任务+catalog重建+feedback worker群","newish":false,"loop_type":"worker","entry":"main.rs:255 knowledge_task / :266 catalog_rebuild / :284 knowledge_feedback","core_rule":"knowledge_task(interval默认30s开)取pending任务按sessionId串行执行plannedSteps经SSE(注:execute_step Phase4占位桩6action不真改chunk);catalog_rebuild(默认3s开)消费队列把active chunk渲染markdown落catalog_summary_persisted+自增catalog_version;feedback(默认600s开)逐workspace 30d滑窗usage_stats回写+dynamic_confidence+structural lint+stage1 sweep","redlines":["knowledge_task execute_step是占位桩不真改chunk(需确认)","feedback stage2 LLM不入热路径"],"focus":"三worker默认开;长任务占位桩真相/catalog重建/动态置信;查chat_task与feedback覆盖"},
  {"id":"E1","name":"管理Agent(AI Command Center)","newish":true,"loop_type":"e2e_closed_loop","entry":"src/routes/management.rs / 路由/management-agent/sessions|messages|commands/:id/confirm|reject|tool-catalog","core_rule":"自然语言总控产出待确认命令(command)不直接执行;高风险动作requiresConfirmation=true须二次确认才真执行;只读规划用只读工具;tool-catalog读可用工具清单","redlines":["高危动作须二次确认才执行(写操作闸)","命令不直接执行先pending"],"focus":"危险动作pending_confirmation/只读直答;查⑩域biz-test"},
  {"id":"F1","name":"DomainProfile行业总装配单","newish":false,"loop_type":"e2e_closed_loop","entry":"src/routes/domain_profiles.rs / 路由/admin/domain-profiles[+generate|publish|rollout|rollback|activate]","core_rule":"每workspace 1条active运行时加载(无则fallback DEFAULT_PROFILE);decision/guards/judge全消费(distrust_self_reported_low_risk等);generate引导层AI对话生成候选须人审才activate;activate联动写operation_domain_configs新current版本;多版本灰度","redlines":["AI生成profile须人审才activate(继承永不自动verify)","DEFAULT profile字节等价非销售域","active唯一"],"focus":"行业总装配真消费/AI生成须人审/灰度发布;查domain_profile_e2e与批B行业画像biz-test"},
  {"id":"F2","name":"domain_schemas+operation_domain_configs+state_policies","newish":false,"loop_type":"crud_versioned","entry":"路由/admin/domain-schemas|operation-domains|operation-state-policies[+publish|rollout|rollback]","core_rule":"domain_schemas每workspace 1条active驱动chunk domain_attributes校验;operation_domain_configs状态机字典底座(prompts.rs种非migration);state_policies每(workspace,domain,state)允许/禁止动作+节奏;三表多版本灰度(publish→rollout→rollback)","redlines":["config底座由prompts.rs::ensure_prompt_pack_v2种非migration","状态机initial/allowedFrom驱动迁移行业无关","版本灰度原子"],"focus":"三表版本灰度原子性/状态机字典来源;查taxonomy_flags与版本灰度集成测试"},
  {"id":"F3","name":"引导层AI生成配置(guide)","newish":false,"loop_type":"e2e_closed_loop","entry":"路由/user-operations/guide/preview|apply / guide_profile.rs","core_rule":"引导层AI对话生成运营引导配置;preview不落库;apply应用AI生成配置;继承AI永不自动生效","redlines":["preview不落库","AI生成配置须应用动作"],"focus":"AI生成引导/preview→apply;查guide相关测试"},
  {"id":"G1","name":"Prompt体系(pack/templates/souls/playbooks)","newish":false,"loop_type":"crud_versioned","entry":"src/prompts.rs ensure_prompt_pack_v2 / 路由/prompt-templates|agent-souls|operation-playbooks[+publish|generate|optimize|reset-system-pack]","core_rule":"分层(Soul→System Contract→Policy→Business Context→Operator Instruction)版本化;ensure_prompt_pack_v2启动种v2默认pack;publish bump prompt_pack_version触发LRU失效;reset-system-pack物理删除重种(显式维护非每启动幂等覆盖防clobber运营编辑);generate/optimize AI生成playbook候选","redlines":["reset-system-pack非每启动幂等覆盖(会clobber运营编辑)","publish bump版本失效缓存","种子改draft非active(防空状态机崩溃)"],"focus":"分层版本化/发布失效缓存/reset语义;查prompt_pack_alignment与种子测试"},
  {"id":"H1","name":"评测体系(场景+公式遵从)","newish":false,"loop_type":"e2e_closed_loop","entry":"路由/evaluation-scenarios / /user-operations/evaluations/run|formula-adherence","core_rule":"评估场景CRUD;evaluations/run跑用户运营评估;formula-adherence跑business_formula遵从度评估","redlines":["评测不真实发送"],"focus":"评测场景/公式遵从评分;查evaluation相关测试"},
  {"id":"I1","name":"账号管理","newish":false,"loop_type":"crud_admin","entry":"路由/accounts[+sync|/:id/mcp-key]","core_rule":"列账号/从MCP同步账号/写账号级MCP密钥;密钥敏感不回显","redlines":["MCP密钥不回显","workspace隔离"],"focus":"账号CRUD/MCP密钥安全/同步;查workspace_isolation"},
  {"id":"I2","name":"联系人管理+画像+手动标签","newish":false,"loop_type":"crud_admin","entry":"路由/contacts/:id/[profile-note|manual-tags|operation-profile|assist-override|custom-agent-instructions|deal-events|operating-memory]","core_rule":"联系人CRUD+画像备注+手动标签(运营权威)+运营画像+assist覆盖+custom指令(system末位最高优先级口吻分化)+成交事件(staff_confirmed)+运行记忆","redlines":["manual_tags运营权威AI永不覆盖","deal-events staff_confirmed唯一T0真相源AI永不直写","custom-instructions system末位最高优先级"],"focus":"画像保守不凭单句盲断/手动标签权威/成交手登红线;查contact相关测试"},
  {"id":"I3","name":"产品目录(products)","newish":true,"loop_type":"crud_admin","entry":"路由/products[+/:id/archive|restore] / objective-purchase-facts G2","core_rule":"运营录入结构化商品(product_id/name/price/sku);成交product_ref是下单快照拷贝不实时引用;(workspace,product_id)unique;archive/restore","redlines":["成交快照拷贝非实时引用","product_id workspace内unique","报价命中active产品目录可作背书(priced_from_catalog)"],"focus":"产品目录/快照语义/作为grounding背书源;查products_workspace_isolation"},
  {"id":"I4","name":"LLM provider热切换","newish":false,"loop_type":"crud_admin","entry":"路由/admin/llm-providers[+/:id/activate|vision|test]","core_rule":"把.env LLM配置抬升为前端可编辑DB(format=openai|anthropic);is_active那条由LlmRegistry启动/切换加载为AppState.llm后端;activate原子热切换整进程LLM调用切新配置;vision设激活视觉模型;test连通性","redlines":["activate原子热切换","不暗示模型品牌(check-no-model-hint)","DB provider热替换非.env"],"focus":"热切换原子性/品牌中立;查provider配置"},
  {"id":"I5","name":"鉴权(auth/session/JWT/workspace)","newish":false,"loop_type":"crud_admin","entry":"路由/auth/[login|logout|me|workspace|token] / auth中间件 / auth/jwt.rs","core_rule":"会话cookie(Argon2)登录唯一鉴权入口;/health与/auth/login白名单免鉴权;其余经require_session;Bearer JWT仅JWT_ENABLED;workspace切换后续读写按此隔离","redlines":["除白名单全require_session","JWT仅JWT_ENABLED时生效","Argon2密码哈希"],"focus":"鉴权门/JWT gating/workspace隔离;查jwt_auth与workspace_isolation"},
  {"id":"I6","name":"可观测/日志/成效","newish":false,"loop_type":"crud_admin","entry":"路由/[events|agent-runs|llm-usage|decision-reviews|send-ledger|agent-outcome-metrics|behavior-signal-metrics|outcomes/autonomy]+/admin/observability/*","core_rule":"只读可观测:agent审计事件/运行轨迹/token用量/决策审查/发送台账/成效指标/行为信号/自治成效;observability一次RTT拉齐worker健康","redlines":["只读不改业务状态","成效指标AI永不直写正式成交"],"focus":"可观测只读/成效来源;查observability测试"},
  {"id":"J1","name":"多租户workspace隔离(横切)","newish":false,"loop_type":"crosscutting","entry":"全admin handler + db typed accessor + auth/middleware","core_rule":"几乎所有集合带workspace_id;admin handler普遍workspace隔离;切换workspace后读写按此隔离;IDOR防护","redlines":["跨workspace读写隔离(IDOR防护)","几乎所有集合workspace_id维度"],"focus":"横切!查所有admin端点是否真隔离;查workspace_isolation/products_workspace_isolation/IDOR sweep"},
  {"id":"J2","name":"红线CI lint守门(横切)","newish":false,"loop_type":"crosscutting","entry":"scripts/check-no-human-takeover.{sh,ps1} / check-no-model-hint.sh / check-evolution-isolation.sh / check-baseline.{sh,ps1}","core_rule":"四CI守门:no-human-takeover(src/agent|routes|evolution|frontend新增行禁转人工词表);no-model-hint(禁硬编码模型品牌名);evolution-isolation(evolution禁引用gateway/outbox/mcp主链路符号);baseline(lib≥350+4PBT≥33硬门)","redlines":["四lint命中即exit 1","baseline lib≥350/PBT≥33不回归","新work加测试不降基线"],"focus":"横切!字面级红线守门是否真拦;查四脚本逻辑与是否被绕过(如G6漏'转人工'历史问题)"},
  {"id":"J3","name":"DB迁移+索引(横切)","newish":false,"loop_type":"crosscutting","entry":"src/db/migrations/ + indexes.rs;main.rs先migrations::run后ensure_indexes","core_rule":"Database::connect不跑迁移/建索引;main.rs先migrations::run(某些重建集合)后ensure_indexes;m011/m012/m014带APP_ENV=production守卫(非prod才删);unique/partial unique索引是去重幂等基石","redlines":["migrations先于indexes顺序","APP_ENV=production守卫(生产117疑似未设潜在隐患)","唯一索引承载幂等去重"],"focus":"横切!迁移顺序/APP_ENV守卫生产隐患/索引幂等;查m018_backfill等迁移测试与生产APP_ENV"},
  {"id":"J4","name":"死代码识别(横切)","newish":false,"loop_type":"crosscutting","entry":"operation_knowledge_items(已死) / pack repair 410死桩 / knowledge_task execute_step占位桩","core_rule":"operation_knowledge_items typed accessor已删,m011/m014物理清空,全src仅占位注释+410端点+migration零业务读写;pack修复propose依赖已删集合永返400死桩;knowledge_task execute_step Phase4占位桩6action不真改chunk","redlines":["死代码不应被测试当活路径误报","死桩端点行为(返400)是已知非bug"],"focus":"横切!识别死代码避免测试误判;确认哪些是真死桩(测试该跳过)哪些是占位待实装"},
]


const BG =
  '项目背景:WechatAgent 是长期运行的微信私域运营 AI agent 系统,单体 Rust(Axum)后端 + React admin,' +
  '接 MongoDB + 外部 MCP server + DeepSeek/OpenAI 兼容 LLM。Phase 1 范围是用户私聊运营,产品定位"全 AI 自治、无人工接管"。' +
  '重要:本项目两条旧 spec(agent-autonomy-loop / user-ops-agent-hardening)已挂 Sunset Notice 2026-05-25,描述过时不可信;' +
  '正确性来源只能是【代码实现 + CLAUDE.md + docs/agent-policy.md 红线】。' +
  '你是只读研究员,绝不写代码/改文件,只用 Read/Grep/Glob。实事求是,找不到就说找不到,不脑补。'

const DEEPREAD_SCHEMA = {
  type: 'object',
  required: ['domain', 'design_behavior', 'correctness_layer'],
  properties: {
    domain: { type: 'string' },
    design_behavior: { type: 'string' },
    redlines: { type: 'array', items: { type: 'string' } },
    existing_coverage: { type: 'string' },
    test_trust: { type: 'string' },
    correctness_layer: { type: 'string' },
    gaps: { type: 'array', items: { type: 'string' } },
    suspected_orphans: { type: 'array', items: { type: 'string' } },
  },
}

const FALSIFY_SCHEMA = {
  type: 'object',
  required: ['domain', 'verdict', 'test_priority'],
  properties: {
    domain: { type: 'string' },
    verified_gaps: { type: 'array', items: { type: 'string' } },
    refuted: { type: 'array', items: { type: 'string' } },
    confirmed_orphans: { type: 'array', items: { type: 'string' } },
    test_priority: { type: 'string', enum: ['P0_redline', 'P1_closed_loop', 'P2_quality', 'P3_crud'] },
    verdict: { type: 'string' },
  },
}

const MAX_CONC = 5

// 重试包装:agent 抛错或返 null 都重试;耗尽仍失败返 null(不抛错,不拖垮工作流)
async function agentRetry(prompt, opts, tries) {
  const n = tries || 3
  for (let i = 0; i < n; i++) {
    try {
      const r = await agent(prompt, opts)
      if (r !== null && r !== undefined) return r
      log('[重试] ' + (opts.label || '') + ' 返回空 ' + (i + 1) + '/' + n)
    } catch (e) {
      log('[重试] ' + (opts.label || '') + ' 异常 ' + (i + 1) + '/' + n + ': ' + String(e).slice(0, 100))
    }
  }
  log('[放弃] ' + (opts.label || '') + ' ' + n + ' 次仍失败,跳过该步(不阻断工作流)')
  return null
}

// 分批并发:每批 ≤size 并行,批间串行(并发上限硬控)
async function inBatches(items, size, fn) {
  const out = []
  const batches = Math.ceil(items.length / size)
  for (let i = 0; i < items.length; i += size) {
    const batch = items.slice(i, i + size)
    log('=== 批次 ' + (Math.floor(i / size) + 1) + '/' + batches + '(' + batch.length + ' 域)===')
    const res = await parallel(batch.map((it, j) => () => fn(it, i + j)))
    out.push(...res)
  }
  return out
}

// 域清单:内联常量(沙箱不支持 import/require/fs,故清单直接内嵌脚本)
// 权威副本在 .kiro/specs/universal-test-coverage/biz-domains-2026-06-30.json,改清单两处同步
let domains = ALL_DOMAINS
// 可选 args.only:["C2","J2"] 仅跑指定 id(子集试跑/补跑)
if (args && Array.isArray(args.only) && args.only.length) {
  const set = new Set(args.only)
  domains = domains.filter((d) => set.has(d.id))
  log('only 过滤后 ' + domains.length + ' 域:' + args.only.join(','))
}
if (domains.length === 0) {
  log('错误:未取得业务域清单,终止')
  return { error: 'no_domains' }
}
log('收到 ' + domains.length + ' 个业务域,≤' + MAX_CONC + ' 并发分批;每域 深读→证伪,各带 3 次重试')

const findings = await inBatches(domains, MAX_CONC, async (domain) => {
  const deep = await agentRetry(
    BG +
      ' 任务:深读业务域【' + domain.name + '】。入口:' + domain.entry + '。核心规则:' + domain.core_rule +
      '。已知红线:' + JSON.stringify(domain.redlines || []) +
      '。本域审计重点:' + (domain.focus || '端到端输入→决策→副作用链路') +
      '。读该域相关代码后产出:design_behavior(按设计应有的行为,围绕上述审计重点,具体到可测步骤);' +
      'redlines(硬红线数组);existing_coverage(查 scripts/biz-test/*.py 与 tests/ 下 .rs 的文件名+内容,说清现在测了什么、没测什么);' +
      'test_trust(可信|假绿|缺失 三选一+理由,特别警惕"测试通过但根本没断言实质行为"的假绿);' +
      'correctness_layer(该域正确性主要属哪层:红线否定式|设计意图正向|正向质量主观|孤儿行为无定义);' +
      'gaps(未覆盖的关键行为);suspected_orphans(代码有实现但红线和文档都没定义对错的孤儿行为)。',
    { schema: DEEPREAD_SCHEMA, label: '深读:' + domain.name, phase: '按域深读', model: 'opus' },
    3
  )
  if (!deep) return null
  const falsify = await agentRetry(
    BG +
      ' 任务:你是对抗验证者,独立证伪另一 agent 对业务域【' + domain.name + '】的深读结论(下方 JSON)。不要附和,主动找反证。' +
      '逐条核对:①它列的 gaps 是真缺口,还是其实已有测试或代码覆盖、只是它没找到?用 grep 验证。' +
      '②它判的 test_trust(尤其假绿)是否属实——亲自打开那个测试读断言。' +
      '③suspected_orphans 是否真无定义,还是红线/文档其实有写、它漏读了?④补它漏掉的缺口。' +
      '产出 verified_gaps(证实的真缺口)/refuted(被推翻的夸大结论+理由)/confirmed_orphans(证实的孤儿行为)/' +
      'test_priority(P0_redline 红线硬门|P1_closed_loop 端到端闭环|P2_quality 正向质量|P3_crud)/verdict(一句话)。' +
      '必须用 Grep/Read 实证,不空口。深读结论:\n' + JSON.stringify(deep),
    { schema: FALSIFY_SCHEMA, label: '证伪:' + domain.name, phase: '对抗证伪', model: 'opus' },
    3
  )
  return { domain: domain.name, entry: domain.entry, newish: domain.newish === true, deep, falsify }
})

const ok = findings.filter(Boolean)
log('深读+证伪完成:' + ok.length + '/' + domains.length + ' 域有结论(失败已跳过,不阻断)')
return { total: domains.length, completed: ok.length, findings: ok }
