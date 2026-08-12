# 业务面路由深读记录（核证日期 2026-08-13）

> 本记录基于对 `src/routes/` 下 26 个业务面文件的 100% 逐行通读（合计 22,995 行，含测试）。
> 所有断言均附 `file:line`（行号为核证日当天工作区文件的实际行号）。读不懂/存疑之处集中在第 5 节。
> 范围外文件（knowledge/ 子树、admin_* 系列、observability、evolution、llm_providers、auth、chunk_locks、ask_human_inbox、principal_escalations、contract_snapshot、health、worker_controls、outcomes_autonomy、behavior_signal_metrics、lessons_learned、domain_schemas、admin_ops_versions）仅在挂载表与调用关系处按 mod.rs 事实列出，不做逐行深读。

---

## 1. AppState 全字段与 api_router 完整挂载表

### 1.1 AppState 全字段（src/routes/mod.rs:284-334）

| 字段 | 类型 | 用途（依据注释与用法） |
| --- | --- | --- |
| `db` | `Database` | Mongo 连接 + 全部 typed collection 访问器（mod.rs:286） |
| `mcp` | `McpClient` | MCP JSON-RPC 客户端（mod.rs:287） |
| `llm` | `Arc<dyn LlmProvider>` | 默认 LLM provider（mod.rs:288） |
| `llm_registry` | `Option<Arc<LlmRegistry>>` | workspace 级 active-provider registry；测试可 None 直接用 `llm`（mod.rs:289-292） |
| `llm_concurrency` | `Arc<LlmConcurrencyGovernor>` | 共享 provider 准入控制，后台任务不得抢占客户面 permits（mod.rs:293-295） |
| `config` | `AppConfig` | 全量环境配置（mod.rs:296） |
| `prompt_pack_version` | `Arc<AtomicU64>` | prompt 包版本计数；seed/publish/release/rollback/reset 时 fetch_add，折进 `generate_agent_json` 的 LRU cache key 实现原子缓存失效（mod.rs:297-304；写点：souls.rs:133-135、prompt_templates.rs:319-321,344-347、souls.rs:148-151） |
| `chat_progress_bus` | `Arc<ChatProgressBus>` | knowledge chat SSE 进度总线（mod.rs:305-308） |
| `second_reviewer_llm` | `Option<Arc<dyn LlmProvider>>` | Phase E 双 reviewer 第二 provider；进程生命周期内不热切（mod.rs:309-318） |
| `chunk_locks` | `chunk_locks::ChunkLockMap` | chunk advisory presence 表，进程内 DashMap（mod.rs:319-321） |
| `chunk_event_bus` | `broadcast::Sender<ChunkEvent>` | chunk 事件总线，WS 订阅（mod.rs:322-325） |
| `jwt_keys` | `Option<Arc<JwtKeys>>` | RS256 keypair，`jwt_enabled=false` → None（mod.rs:326-328） |
| `auth_rate_limiter` | `Arc<AuthRateLimiter>` | 登录/token 限流器，login 与 token 两个公共端点共享一个预算（mod.rs:329-331） |
| `completeness_cache` | `CompletenessCache` = `Arc<DashMap<(String,String),(i64,Value)>>` | F-013 completeness 进程内 TTL 缓存，key=(workspace_id, account_id)（mod.rs:276-282,332-333） |

鉴权总闸：整个 `api_router` 末尾套 `require_session` middleware（mod.rs:1060-1065），白名单 `/health` + `/auth/login`（注释 mod.rs:1060）；所有 handler 通过 `Extension<AuthenticatedAdmin>` 拿 `user_id/username/current_workspace`。系统只有这一个鉴权角色（operation_view.rs:25-27 注释）。

### 1.2 api_router 完整挂载表（mod.rs:336-1067，按挂载顺序）

方法+路径 → handler（所在文件）。`[外]` = handler 在本组深读范围之外的文件。

| # | 方法 路径 | handler（文件:挂载行） |
|---|---|---|
| 1 | GET /health | health（[外]health.rs；mod.rs:338） |
| 2 | POST /auth/login | auth::login（[外]auth.rs；mod.rs:339） |
| 3 | POST /auth/logout | auth::logout（[外]；mod.rs:340） |
| 4 | GET /auth/me | auth::me（[外]；mod.rs:341） |
| 5 | POST /auth/workspace | auth::switch_workspace（[外]；mod.rs:342） |
| 6 | POST /auth/token | auth::issue_token（[外]；mod.rs:343） |
| 7 | GET /accounts | list_accounts（accounts.rs；mod.rs:344） |
| 8 | POST /accounts/sync | sync_accounts（accounts.rs；mod.rs:345） |
| 9 | POST /accounts/login/begin | login_begin（accounts.rs；mod.rs:346） |
| 10 | GET /accounts/login/poll | login_poll（accounts.rs；mod.rs:347） |
| 11 | PUT /accounts/:id/mcp-key | update_account_mcp_key（accounts.rs；mod.rs:348） |
| 12 | GET /contacts | list_contacts（contacts.rs；mod.rs:349） |
| 13 | GET /contacts/counts | count_contacts（contacts.rs；mod.rs:350） |
| 14 | POST /contacts/search | search_contacts_endpoint（contacts.rs；mod.rs:351） |
| 15 | POST /contacts/import | import_contacts_endpoint（contacts.rs；mod.rs:352） |
| 16 | POST /contacts/search-import | search_import_contacts（contacts.rs，DEPRECATED；mod.rs:353） |
| 17 | GET /contacts/roster | roster_endpoint（contacts.rs；mod.rs:354） |
| 18 | POST /contacts/batch-enable | batch_enable_endpoint（contacts.rs；mod.rs:355） |
| 19 | GET /contacts/:id | get_contact（contacts.rs；mod.rs:356） |
| 20 | POST /contacts/:id/enable-agent | enable_agent（contacts.rs；mod.rs:357） |
| 21 | POST /contacts/:id/disable-agent | disable_agent（contacts.rs；mod.rs:358） |
| 22 | POST /contacts/:id/hide-from-pool | hide_from_pool（contacts.rs；mod.rs:359） |
| 23 | POST /contacts/:id/revoke-principal-exemption | revoke_principal_exemption（contacts.rs；mod.rs:360-363） |
| 24 | PUT /contacts/:id/profile-note | update_profile_note（contacts.rs；mod.rs:364） |
| 25 | PUT /contacts/:id/assist-override | update_assist_override（contacts.rs；mod.rs:365） |
| 26 | POST /contacts/:id/clear-referral | clear_referral（contacts.rs；mod.rs:366） |
| 27 | PUT /contacts/:id/custom-agent-instructions | update_custom_agent_instructions（contacts.rs；mod.rs:367-370） |
| 28 | PUT /contacts/:id/manual-tags | update_manual_tags（contacts.rs；mod.rs:371） |
| 29 | PUT /contacts/:id/operation-profile | update_operation_profile（contacts.rs；mod.rs:372-375） |
| 30 | POST /contacts/:id/deal-events | add_deal_event（contacts.rs；mod.rs:376） |
| 31 | GET /contacts/:id/outcome-events | list_outcome_events（contacts.rs；mod.rs:377） |
| 32 | GET /contacts/:id/entitlements | list_entitlements（contacts.rs；mod.rs:378） |
| 33 | POST /contacts/:id/analyze-profile | analyze_contact_profile（contacts.rs；mod.rs:379-382） |
| 34 | GET+PUT /contacts/:id/operating-memory | get_operating_memory / update_operating_memory（contacts.rs；mod.rs:383-386） |
| 35 | GET /contacts/:id/memory-card | get_contact_memory_card（contacts.rs；mod.rs:387） |
| 36 | GET /contacts/:id/memory-candidates | list_contact_memory_candidates（contacts.rs；mod.rs:388-391） |
| 37 | POST /contacts/:id/memory-consolidation/run | run_contact_memory_consolidation（contacts.rs；mod.rs:392-395） |
| 38 | GET /contacts/:id/operation-health | get_operation_health（contacts.rs；mod.rs:396） |
| 39 | POST /user-operations/guide/preview | preview_user_operation_guide（guides.rs；mod.rs:397-400） |
| 40 | POST /user-operations/guide/apply | apply_user_operation_guide（guides.rs；mod.rs:401-404） |
| 41 | POST /user-operations/simulations/dialogue | simulate_user_operation_dialogue（simulations.rs；mod.rs:405-408） |
| 42 | POST /user-operations/evaluations/run | run_user_operation_evaluation（simulations.rs；mod.rs:409-412） |
| 43 | GET /conversations/:contact_id/messages | list_messages（conversations.rs；mod.rs:413） |
| 44 | GET /events | list_events（events.rs；mod.rs:414） |
| 45 | GET /tasks | list_tasks（tasks.rs；mod.rs:415） |
| 46 | GET /agent-runs | list_agent_runs（tasks.rs；mod.rs:416） |
| 47 | GET /llm-usage | list_llm_usage（tasks.rs；mod.rs:417） |
| 48 | POST /agent-tasks/:id/review-now | review_task_now（tasks.rs；mod.rs:418） |
| 49 | POST /agent-tasks/:id/cancel | cancel_agent_task（tasks.rs；mod.rs:419） |
| 50 | GET+POST /content-assets | list_content_assets / create_content_asset（assets.rs；mod.rs:420-423） |
| 51 | POST /content-assets/upload | media_assets::upload_media_asset（media_assets.rs；mod.rs:424-432，单独抬高 body limit 到 `media_max_file_size_mb`） |
| 52 | POST /content-assets/:id/review | media_assets::review_media_asset（mod.rs:433-436） |
| 53 | PUT+DELETE /content-assets/:id | media_assets::update_content_asset_meta / delete_content_asset（mod.rs:437-441） |
| 54 | POST /content-assets/:id/file | media_assets::replace_content_asset_file（mod.rs:442-449，同样抬高 body limit） |
| 55 | POST /content-assets/:id/toggle | media_assets::toggle_content_asset_sendable（mod.rs:450-453） |
| 56 | POST+GET /referral-cards | referral_cards::create_referral_card / list_referral_cards（mod.rs:454-457） |
| 57 | POST /referral-cards/:id/review | referral_cards::review_referral_card（mod.rs:458-461） |
| 58 | POST /referral-cards/:id/toggle | referral_cards::toggle_referral_card（mod.rs:462-465） |
| 59 | DELETE /referral-cards/:id | referral_cards::delete_referral_card（mod.rs:466-469） |
| 60 | GET /contacts/:wxid/send-history | send_ledger::contact_send_history（mod.rs:470-473） |
| 61 | GET /send-ledger/stats | send_ledger::send_ledger_stats（mod.rs:474） |
| 62 | GET /operation/active-view | operation_view::active_view（mod.rs:475） |
| 63 | GET /send-ledger/overview | send_ledger::send_ledger_overview（mod.rs:476-479） |
| 64-129 | /operation-knowledge/**、/knowledge/**（共 66 条挂载） | [外]knowledge/ 子树 handler（mod.rs:480-726）。含 chunks CRUD/verify/reject/repair/patch/archive/restore/rollback/revisions/split/merge/relate/lock、review-queue、batch-verify、batch-archive、referrers、catalog(+persisted)、completeness(GET/POST)、integrity-report、tools/search、tools/open-slice、tools/open-evidence、auto-verify、gap-signals(list/dismiss/apply/sweep)、ask(+stream)、metrics、operator-memory(+revoke)、import-preview(+job/jobs)、import-apply(+pdf/+image)、extract-tags、test-match、usage、logs/analyze、repair/applied、chat(+history/apply/discard)、inbox、metadata、digest(today/regenerate/cards dismiss)、chat/tasks(list/create/get/cancel)、chat/sessions stream、ingest-sources CRUD、/ws/chunks WebSocket |
| 130 | GET /decision-reviews | list_decision_reviews（reviews.rs；mod.rs:727） |
| 131 | GET /decision-reviews/:id | get_decision_review（reviews.rs；mod.rs:728） |
| 132 | POST /decision-reviews/:id/post-decision/retry | retry_post_decision（reviews.rs；mod.rs:729-732） |
| 133 | POST /decision-reviews/:id/post-decision/regenerate | regenerate_post_decision（reviews.rs；mod.rs:733-736） |
| 134 | POST /decision-reviews/:id/post-decision/discard | discard_post_decision（reviews.rs；mod.rs:737-740） |
| 135 | GET /agent-outcome-metrics | list_agent_outcome_metrics（outcome_metrics.rs；mod.rs:741） |
| 136 | GET /behavior-signal-metrics | list_behavior_signal_metrics（[外]behavior_signal_metrics.rs；mod.rs:742-745） |
| 137 | GET /outcomes/autonomy | get_autonomy_outcomes（[外]outcomes_autonomy.rs；mod.rs:746） |
| 138 | GET /outcomes/autonomy/revisions | list_autonomy_revisions（[外]；mod.rs:747） |
| 139 | GET+POST /evaluation-scenarios | list/create_evaluation_scenario（evaluations.rs；mod.rs:748-751） |
| 140 | PUT+DELETE /evaluation-scenarios/:id | update/delete_evaluation_scenario（evaluations.rs；mod.rs:752-755） |
| 141 | POST /user-operations/evaluations/formula-adherence | run_formula_adherence_evaluation（evaluations.rs；mod.rs:756-759） |
| 142 | GET+POST /agent-souls | list/create_agent_soul（souls.rs；mod.rs:760-763） |
| 143 | PUT /agent-souls/:id | update_agent_soul（souls.rs；mod.rs:764） |
| 144 | POST /agent-souls/:id/publish | publish_agent_soul（souls.rs；mod.rs:765） |
| 145 | GET /operation-domains | list_operation_domains（domains.rs；mod.rs:766） |
| 146 | GET+PUT /operation-domains/:domain | get/update_operation_domain（domains.rs；mod.rs:767-770） |
| 147 | GET+PUT /operation-domains/:domain/state-machine | get/update_operation_domain_state_machine（domains.rs；mod.rs:771-774） |
| 148 | POST /operation-domains/:domain/reset | reset_operation_domain（domains.rs；mod.rs:775-778） |
| 149 | PUT /operation-domains/:domain/ask-human-policy | put_ask_human_policy（domains.rs；mod.rs:779-782） |
| 150 | GET+POST /prompt-templates | list/create_prompt_template（prompt_templates.rs；mod.rs:783-786） |
| 151 | PUT /prompt-templates/:id | update_prompt_template（prompt_templates.rs；mod.rs:787） |
| 152 | POST /prompt-templates/:id/publish | publish_prompt_template（prompt_templates.rs；mod.rs:788-791） |
| 153 | POST /prompt-templates/reset-system-pack | reset_system_prompt_pack（prompt_templates.rs；mod.rs:792-795） |
| 154 | GET+POST /operation-playbooks | list/create_operation_playbook（playbooks.rs；mod.rs:796-799） |
| 155 | POST /operation-playbooks/generate | generate_operation_playbook（playbooks.rs；mod.rs:800-803） |
| 156 | POST /operation-playbooks/:id/optimize | optimize_operation_playbook（playbooks.rs；mod.rs:804-807） |
| 157 | PUT /operation-playbooks/:id | update_operation_playbook（playbooks.rs；mod.rs:808） |
| 158 | POST /operation-playbooks/:id/set-default | set_default_operation_playbook（playbooks.rs；mod.rs:809-812） |
| 159 | GET+POST /products | list/create_product（products.rs；mod.rs:814） |
| 160 | PUT /products/:product_id | update_product（products.rs；mod.rs:815） |
| 161 | POST /products/:product_id/archive | archive_product（products.rs；mod.rs:816） |
| 162 | POST /products/:product_id/restore | restore_product（products.rs；mod.rs:817） |
| 163 | POST+GET /campaigns | create/list_campaigns（campaigns.rs；mod.rs:818） |
| 164 | PATCH /campaigns/:id | update_campaign_draft（campaigns.rs；mod.rs:819） |
| 165 | POST /campaigns/:id/preview | preview_campaign（campaigns.rs；mod.rs:820） |
| 166 | POST /campaigns/:id/dispatch | dispatch_campaign（campaigns.rs；mod.rs:821） |
| 167 | GET /campaigns/:id/sends | campaign_sends_report（campaigns.rs；mod.rs:822） |
| 168 | POST /management-agent/sessions | create_management_session（management.rs；mod.rs:823-826） |
| 169 | POST /management-agent/sessions/:id/messages | post_management_message（management.rs；mod.rs:827-830） |
| 170 | GET /management-agent/commands/:id | get_management_command（management.rs；mod.rs:831-834） |
| 171 | POST /management-agent/commands/:id/confirm | confirm_management_command（management.rs；mod.rs:835-838） |
| 172 | POST /management-agent/commands/:id/reject | reject_management_command（management.rs；mod.rs:839-842） |
| 173 | GET /management-agent/tool-catalog | get_tool_catalog（management.rs；mod.rs:843） |
| 174-235 | /admin/**、/evolution/** 等（共 60+ 条挂载） | [外]worker_controls、admin_taxonomies、admin_taxonomy_candidates、admin_relationship_suggestions、admin_suspected_deals、admin_state_policies、admin_ops_versions（publish/rollout/rollback ×3 资源）、principal_escalations、ask_human_inbox、admin_outbox、lessons_learned、observability（phase-rollup/performance/worker-health）、llm_providers、domain_schemas、domain_profiles（含 /admin/domain-profiles/generate → guide_profile.rs::generate_domain_profile_candidate，mod.rs:1030-1034）、evolution（mod.rs:844-1059） |

死路由防线：`no_orphan_pub_async_route_handlers` 测试（mod.rs:1080-1248）用 `include_str!` 静态扫描全部 route 文件的 `pub async fn`，比对 mod.rs 挂载文本；`KNOWN_NON_ROUTE_HANDLERS`（mod.rs:1132-1211）列出 30 个"框架/复用 helper"豁免（如 `upsert_contact_from_value`、`add_outcome_event_inner`、`handle_initial_profile_task_with_claim`、`reconcile_initial_profile_commit` 等）。

---

## 2. 逐文件深读

### 2.1 mod.rs（1249 行）——组装层

- 模块可见性策略：默认 `mod` 私有；集成测试需直调真函数的标 `pub`，每处附注释说明哪个测试（mod.rs:20-94）。`contract_snapshot` 仅 `#[cfg(test)]`（mod.rs:48-49）。
- `ext_knowledge` 导出块（mod.rs:103-147）把 knowledge 内部 handler/请求体暴露给测试 crate（绕过 axum extractor），注释逐条解释原因（multipart 无法手工构造、real-LLM smoke 等）。
- `pub use shared::upsert_contact_from_value` + `apply_contact_changes`/`SkippedField`（mod.rs:148-149）：webhooks.rs 与集成测试的入口。
- `CompletenessCache` 类型定义（mod.rs:281-282）。
- `api_router` 内联两处 `DefaultBodyLimit::max(media_max_file_size_mb * 1MB)`：只给 upload 与 replace-file 两条路由抬 body 上限，其余保留 axum 默认 2MB（mod.rs:424-432,442-449 注释）。

### 2.2 shared.rs（2919 行）——跨路由 helper（每个 helper 的调用方）

**身份与安全类**
- `SystemReviewActor`/`ReviewActor`（shared.rs:24-55）：受信审核身份封装，内值不可由请求 JSON 提供。`from_admin` 空 username → `Unauthorized`（shared.rs:36-41）；`system(ManagementAgent)`→"system:management_agent"（shared.rs:45-50）。调用方：management.rs:3137-3139,3154-3156（管理 agent 的 approve_relationship_suggestion / approve_taxonomy_candidate 工具分支），admin_relationship_suggestions / admin_taxonomy_candidates 的 inner 函数消费。
- `resolve_authorized_workspace`（shared.rs:1905-1946）：#H3 防认证后水平越权。`override_ws` trim 非空优先，否则回落 `admin.current_workspace`；**每个请求都校验 ACL**。关键分支：`admin.user_id` 为空 ⟹ crate 内部合成 admin（`management_admin`，见 management.rs:3266-3272），**跳过 ACL**（shared.rs:1920-1922，注释论证全仓唯一空 user_id 构造点即 management_admin，真实会话 user_id 恒非空）。调用方：llm_providers 等带 workspaceId 覆盖的路由（本组外）。
- `find_contact_by_id`（shared.rs:218-233）：**强制 workspace 隔离**——签名要求 workspace_id，编译期 fail-closed；跨 workspace 返回 404 不泄漏存在性。全 contacts/conversations/reviews/simulations/guides 读路径调用。
- `find_contact_by_id_for_account`（shared.rs:239-262）：多一层 account 精确匹配；账号不匹配返回 `Conflict("contact_account_conflict")`（409 区分"身份漂移"与"不存在"）。调用方：contacts.rs 高权威写端点（enable_agent:1939-1941、update_profile_note:2238-2240、update_assist_override:2384、update_custom_agent_instructions:2431、update_manual_tags:2493、update_operation_profile:2623-2625、add_deal_event:2777-2779、update_operating_memory:2905-2907）、evaluations.rs:270-277。
- `validate_account`（shared.rs:189-209）：account 必须在本 workspace 的 wechat_accounts 注册，否则 404。调用方遍布 management/campaigns/evaluations/reviews/send_ledger/simulations/contacts。
- `escape_regex_literal`（shared.rs:93-105）：#154 转义 15 个正则元字符，防 `list_contacts` 搜索框 `q` 注入 Mongo `$regex` 造成 ReDoS。调用方：contacts.rs:196。

**联系人写入类**
- `upsert_contact_from_value`（shared.rs:264-324，pub）：从 MCP contact JSON upsert 联系人。wxid 取 userName/username/wxid 任一（shared.rs:270-274）；`is_operatable_person` 拦非真人（shared.rs:283-285）；identity patch 语义（`contact_identity_patch` shared.rs:326-342）：nickname/remark/alias **只在候选带非 null 值时 $set**（缺失=不动，清除需专门操作，注释 shared.rs:279-282）；`$setOnInsert` 初始 `agent_status:"normal"`。调用方：contacts.rs（import 三端点 472,520-526）、management.rs:2088-2090（import_contacts 工具）、webhooks.rs（经 pub use）。
- `apply_contact_changes`（shared.rs:785-808，pub）+ `prepare_contact_changes`（shared.rs:810-957）：guide apply 的 contact 字段落库内核。逐字段处理 suggestedChanges：humanProfileNote 直写；tags → normalize+validate 后写 manual_tags（shared.rs:820-824）；customerStage/intentLevel 走 `dimension_registry::validate_dimension_value(AdminWrite)`，**guide 路径 Reject → 记 SkippedField 跳过**（不像手动表单硬拒 400，shared.rs:825-913）；operationState 过 `check_state_transition` 状态机闸，非法迁移记 skipped（问题 F 修复，shared.rs:917-946；domain_config=None fail-open 照写）；operationStateReason/operationPolicy 直写。set_doc 非空才补 updated_at（shared.rs:953-955）。
- `insert_domain_stage_fields`（shared.rs:136-159）：customer_stage/intent_level 以 dotted-key 写 `domain_attributes.*` 容器（顶层字段已删除，写顶层会被 serde 丢弃）；stage_changed=true 时联动刷 `customer_stage_updated_at`（planner stagnation 计时器依赖）；容器级 `domain_attributes_updated_at` 恒刷新；内核已下沉 `agent::domain_signals::insert_domain_signal_values`，admin 路径传 stagnation profile=None（shared.rs:149-157）。调用方：shared.rs:876,884,911（guide）、contacts.rs:815,2277,2397(通过validate后),2701,2855、management.rs:2195,2397。
- `apply_admin_dim_validation`（shared.rs:168-177）：DimValidation 三通道→写入决策（Accept→Some、DropSilently→None、Reject→400）。调用方：contacts.rs（update_operation_profile 2642-2652,2688-2698、relationship_type 2711-2721、validate_generated_stage_intent 2585-2609）、management.rs:2166-2191,2346-2356,2384-2394。
- `is_previously_operated`（shared.rs:185-187）：`last_agent_run_at` 或 `last_outbound_at` 非空 = 曾运营过（#72 老客户重新启用不覆盖历史画像的判据）。调用方：contacts.rs:800,1792,2264,2842、management.rs:2158。

**记忆类**
- `ensure_operating_memory`（shared.rs:459-523）：读取或创建 operating_memories 行；缺种子记忆卡时经 `seed_operating_memory_projection`（shared.rs:344-363）从 contact 播种并回写（版本 +1）。GET 端点会写库（get_operating_memory/get_contact_memory_card 使用）。
- `read_operating_memory`（shared.rs:438-457）：**纯只读投影**，缺行/未播种在内存中投影同样默认值，绝不写库——观测端点（get_operation_health contacts.rs:3027、guide preview guides.rs:515）用它避免 GET 变 mutation。
- `latest_decision_review`（shared.rs:525-544）：取该 contact 最新一条 decision_review。
- `prepare_memory_changes`（shared.rs:959-988）：guide 的 memory patch merge（4 组 merge_document 合并非 Null 键）。
- `prepare_playbook_changes`（shared.rs:990-1013）：guide 的 playbookPatch → (playbook_id, set_doc)，写 created_by="guide_optimized"。
- `prepare_domain_changes`（shared.rs:1015-1047）：guide 的 domainRuntimeParameters patch：先 `validate_guide_runtime_parameter_patch` 白名单校验（shared.rs:1026-1027），merge 现 runtime 后再 `validate_and_normalize_user_runtime_parameters` 全量校验，返回 (config_id, merged_runtime, updated_at, version) OCC 基线；目标恒 current_version=true 的 user_operations 行（shared.rs:1049-1055）。

**健康度类**
- `operation_health_json`（shared.rs:577-594）：scores + items + 作息三值。
- `compute_quiet_hours_view`（shared.rs:604-636）：只读算 inQuietHours/nextWakeAt/quietHoursEnabled；jitter_seed 用 contact.wxid、与 gateway 重排 wake 同口径。调用方：contacts.rs:3029-3030、guides.rs:518-519。
- `health_scores_document`（shared.rs:717-770）：userUnderstanding/relationshipQuality/productFit 按字段在场率打分（`score_presence` shared.rs:772-783，值="unknown" 视为缺席）；rhythmRisk 由 cooldown_until 在场(55)/否(20) + 出过站没回消息(+10) 构成；knowledgeGrounding/hallucinationRisk/pressureRisk 取 review.scores 的 0-10 分 ×10 到 0-100（shared.rs:763-768，P0-4 与三闸/软闸对齐，旧 factRisk 键已下线）。
- `health_items_from_scores`（shared.rs:644-690）：canonical 7 项；`health_item`（shared.rs:692-715）tone 方向：key 以 "Risk" 结尾 → 高分=danger（≥70 danger/≥40 warn），否则高分=good（≥75 good/≥45 warn）。

**guide/JSON 投影类**
- `build_guide_preview_prompt`（shared.rs:1057-1202）：LLM 生成"修改预览"的 user prompt——注入 contact 画像/记忆/playbook 简介/最近复盘/健康度 + **合法枚举取值**（operationState 状态机 keys、customerStage/intentLevel 字典 canonical(label)，空时"暂无受控取值"兜底），要求 impactScope 默认 current_contact、playbookPatch/domainRuntimeParameters 仅当明确说全局。
- `guide_preview_json`（shared.rs:1215-1257）：预览响应投影；`requiresStrongConfirmation = impact_scope != "current_contact"`（shared.rs:1231）；同时下发新 `health:{scores,items}` 与兼容旧 `healthScores` 键。
- `operating_memory_json`（shared.rs:1259-1274）、`memory_candidate_json`（1308-1323）、`llm_call_log_json`（1325-1353，usage_known 推导）、`agent_run_json`（1460-1478，35 字段投影 15 键）。
- `decision_review_json`（shared.rs:1355-1400）+ `decision_review_phase`（shared.rs:1404-1458）：把 review.status × final_review_status × outbox_status 投影成单一 UI phase。优先级：outbox 终态（sent/queued/partially_sent/delivery_failed/delivery_canceled/delivery_unknown）> gateway/precheck/enqueue-failed → "gateway_blocked" > final_review_status（approved 系→approved/auto_rewrite_approved；held/blocked 系→final_blocked/auto_rewrite_failed）> rewrite in progress > queued > review_recorded。`rewrite_requested` 恒为中间态。
- `effective_route_memory_card_typed`（shared.rs:1283-1306）：memory_card 优先、context_pack 兜底、否则空骨架。
- `commitments_with_optional_text`（shared.rs:1599-1617）：单字符串承诺升级为结构化 Vec<CommitmentRepr>，按 text 去重、上限 8 条从前淘汰。调用方：contacts.rs:769,2250,2663,2831、management.rs:2140,2369。

**成效事件类（单一落库真相源）**
- `OutcomeEventInput`/`PreparedOutcomeEvent`（shared.rs:1648-1679）。
- `prepare_outcome_event`（shared.rs:1705-1828）：校验闭集——amount 非负整数分（is_valid_minor_amount）、currency ISO-4217、verification ∈ {staff_confirmed(默认), payment_verified}（**conversation_inferred 绝不走直登**，shared.rs:1633-1642）、event_kind ∈ {deal(默认), reversal}（shared.rs:1620-1628）、reversal 必须带 product_id（shared.rs:1740-1744）；product 解引用 filter 含 workspace_id（IDOR §3.5），正向成交只认 active 产品、reversal 放宽任意 status（shared.rs:1748-1772）；冻结订单式快照 OutcomeProductRef（含 entitlement_days，G4 #4，shared.rs:1773-1782）；构造审计事件 `outcome_event_marked`。
- `persist_prepared_outcome_event`（shared.rs:1830-1861）/`..._with_session`（1865-1898）：$push outcome_events + 插审计事件；matched≠1 → Conflict("contact_changed_before_outcome_append")。
- `add_outcome_event_inner`（shared.rs:1690-1699，pub）= prepare + persist。调用方：contacts.rs::add_deal_event:2780-2797、management.rs::write_deal_events:2743-2762、admin_suspected_deals（approve，经 with_session 变体，[外]）。

### 2.3 management.rs（4254 行）——管理 Agent：会话/计划冻结/确认闸/工具白名单/租约执行

**数据流总览（自然语言指令 → 执行）**：`POST /management-agent/sessions/:id/messages`（post_management_message，management.rs:681-920）：
1. content 非空 + validate_account + session 按 (id, workspace, account) 三键查回（687-704）。
2. 落 user message（705-720）。
3. `mcp::list_tools_for_account` 拉 MCP 工具目录 → `merge_product_tools`（722-724）→ `advertised_tool_names` 白名单（725）。
4. `management_context`（726；实现 3360-3474）：最近 30 联系人 + 20 内容资产 + 30 playbook（含 id/accountId/version，"写操作必须复制"）+ 20 个 draft/previewed 活动（含 specVersion/specHash，"dispatch 必须复制"）。
5. `build_management_plan`（729-738；实现 3476-3510）：prompts `management.plan.system`+`management.plan.policy` 拼 system，操作员指令+上下文+工具目录拼 user，经 **`agent::generate_agent_json`（prompt_key="management.plan"）** 产出 `ManagementPlan{intent,risk_level,requires_confirmation,missing_information,summary,tool_calls}`；空 tool_name 过滤；`validate_management_plan` 上限 12 个 tool_calls（103,202-210）。
6. `apply_locked_send_content`（739；实现 1645-1674）：指令含"内容必须完全等于："等 12 个标记（1678-1691）时**逐字锁定** send_contact_message 的 content（防 LLM 改写），写 `originalContentLocked:true`；带引号取引号体，无引号但含歧义分隔（"。这是"/"；不要"/换行等）→ 400 要求用户加引号（1712-1731）；dry-run 下参数非对象不报错而是落 `lockedContentError` 可视化（1657-1665）。
7. `management_plan_hash`：**plan 冻结 hash = SHA-256(serde_json::to_vec(plan))**（212-217）。
8. 确认判定（747-750）：`requires_confirmation = !dry_run && (plan.requires_confirmation || risk_level=="dangerous"(忽略大小写) || plan_requires_confirmation(工具名))`。
9. 写 `AgentCommandRun`：status = pending_confirmation / running；**只有不需确认时才生成 execution_token（uuid）+ execution_started_at**（751-757）；记录 prompt_versions（759-766）。
10. 不需确认 → 立即 `execute_plan_tool_calls`（791-807，传 tool_calls；需确认传空 slice）。
11. 终态落库（812-875）：needs-confirm 分支只 touch updated_at 且 filter 锁 status=pending_confirmation；执行分支 filter 锁 (status=running + execution_token)，$set 终态（failed/dry_run/succeeded）并 **$unset execution_token/execution_started_at**；matched≠1 → Conflict("management_command_finalize_conflict")。
12. assistant 汇报（876-891）：需确认 → "待确认：{summary}"；失败 → "执行失败：{error}"；有 outcome → `build_execution_summary`（spec §3.2 按真实 outcome 汇报，**不回放 plan.summary**）。

**确认闸（confirm_management_command，926-1135）**：
- 入参 accountId+planHash 必填；validate_account；先按 (id, workspace, account, plan_hash) 找 candidate——找不到 → Conflict("management_command_binding_mismatch_or_legacy")（945-962，legacy 无 hash 的命令**故意不可执行**）。
- **重算 hash 验证冻结完整性**：`management_plan_hash(stored_plan) != requested_hash` → Conflict("management_plan_hash_mismatch")（963-976）+ 再次 validate_management_plan。
- 非 pending/running 终态 → 幂等回放已存 status+summary+toolCalls（979-991）。
- **租约夺取**（993-1028）：find_one_and_update，filter = `_id+workspace+account+plan_hash + (status=pending_confirmation OR (status=running AND execution_started_at ≤ now-5min))`——即支持夺取**过期租约**（MANAGEMENT_EXECUTION_LEASE_MILLIS=5min，management.rs:101）；$set status=running + 新 execution_token + confirmed_by/confirmed_at。抢锁失败 → 回读当前状态幂等返回（1029-1054）。
- 夺取成功 → 重新拉工具目录 → `execute_plan_tool_calls(dry_run=false, confirmed_admin=Some(admin), execution_token=Some)`（1055-1072）。
- 终态：execution_unknown / failed / succeeded；finalize filter 再次锁 running+token（1082-1113）；落 assistant message。

**驳回（reject_management_command，1139-1184）**：`build_confirm_filter`（638-651，锁 pending_confirmation）原子改 canceled；幂等 None → "already_processed_or_not_found"；落"已取消该计划，未执行。"。

**租约执行引擎（execute_plan_tool_calls，336-381 + execute_plan_tool_calls_owned，383-636）**：
- 非空 tool_calls 但无 execution_token → Conflict（348-352，杜绝无租约执行）。
- 心跳：`spawn_management_execution_heartbeat`（142-171）每 60s（102）用 owner_filter（`_id+workspace+account+status=running+execution_token`，114-123）touch execution_started_at；丢租约 → cancelled 原子标志置位；执行循环每个 call 前后 `ensure_management_execution_owned`（173-196，407-409,538-542）——外部调用后**必须再次确权才 finalize**，防脑裂双写。
- **每 call 一条持久 intent**（agent_tool_calls 集合）：`intent_key = "management-tool:v1:{command_run_id}:{plan_hash}:{call_index}"`（219-225）；insert status="prepared"，DuplicateKey 容忍（431-435）后按 intent_key 读回（load_tool_call_by_intent，311-330）。
- **重放语义（persisted_tool_outcome，265-309）**：已有终态 intent 直接复用——succeeded/dry_run → Succeeded（send 工具重放 response 重新断言）；**accepted 重放绝不降级为 Succeeded**（280-291，response 缺失也保持 Accepted）；failed → Failed；executed_unverified → Unverified；execution_unknown → ExecutionUnknown。Failed/ExecutionUnknown → 失败即止 break（442-455）。
- **崩溃恢复**：stored.status=="executing"（上个进程执行中崩溃）→ **绝不重放**，原子改 execution_unknown 并停止余下 plan（458-491）。
- claim：prepared → executing CAS（modified≠1 → Conflict claim_conflict，499-527）；执行 `execute_management_tool`；结果按 `assert_tool_outcome` 断言业务真相后 finalize（executing → 终态 CAS，模糊即 Conflict finalize_conflict，544-628）。
- `PlanExecution{calls,outcomes,failed,execution_unknown}`（94-99）。

**工具目录与白名单**：
- `is_forbidden_raw_send_tool`：`message_send_*` 前缀（1256-1258）；`remove_forbidden_raw_send_tools`（1260-1281）递归从 MCP 目录**摘除原生发送工具**（绕过产品网关的资格/内容锁/评审/台账/幂等，注释 1284-1286）；执行兜底分支独立再拒一次（3238-3243）。
- `merge_product_tools`（1283-1591）：注入 60 个 `wechatagent.*` 产品工具声明（联系人 7 + 观测查询 5 + 版本灰度 17 + 运营态 8 + 调参 2 + 策略编辑 8 + 知识维护 13 + 批4 6 + campaign 2），兼容 tools/allowed_tools/auth.allowed_tools/非对象四种目录形态（1557-1589）。
- `advertised_tool_names`（1596-1643）：递归收集全部公布名，用于兜底分支拦 LLM 幻觉工具名（3244-3250：不在白名单 → 400；在 → `mcp::logged_call_for_account` 透传）。
- **风险分级（tool_effect，1793-1904）**：四档 Readonly/Low/Dangerous/Irreversible + `explicitly_classified`。Readonly=16 个（含 raw MCP 只读 6 个）；Low=schedule_create/cancel、media_get、import/enable/disable/create_follow_up_task/update_contact_profile、publish_* 出草稿、provider_test、运营态单对象写、知识维护落 draft、cancel_outbox、approve_relationship_suggestion、approve_taxonomy_candidate、import_knowledge_pdf、preview_campaign 等；Dangerous=send_contact_message、edit/set_default/generate/optimize_playbook、publish_prompt_template、edit_state_machine、provider_activate、verify/reject/batch_verify chunk、publish_soul、全部 rollout/rollback/activate、release/rollback_evolution_proposal、dispatch_campaign、update_ask_human_policy；Irreversible=reset_domain、delete_knowledge_chunk、reset_system_pack；**未分类 fail-closed 归 Dangerous+explicitly_classified=false**（1894-1896）。
- **确认规则（plan_requires_confirmation，2014-2020）**：未显式分类 → 必确认；非只读且不在 {media_get, provider_test} 豁免 → 必确认。即**所有真实副作用默认要求确认**（注释 2008-2013：verify 类写 source=Human 恒确认，守"AI 永不自动 verify"）。
- **dry-run（should_dry_run_tool，2028-2030 + 2041-2057）**：dry_run 且非只读 → 返回 `{dry_run:true, would_execute:{...}}` 不实际执行；只读工具 dry-run 下照常执行以便看查询结果。

**execute_management_tool（2032-3261）逐工具要点**：
- 联系人类：`search_contacts`→MCP contacts_search 只读（2059-2069）；`import_contacts`→search+`upsert_contact_from_value`（2070-2097）；`enable_contact_agent`（2098-2236）：**账号不能运营自己**（is_self_account 拒绝+审计 2103-2128），resolve playbook → `agent::build_initial_operation_profile`（LLM）→ 老客户保留 stage/state/commitments（is_previously_operated 分支 2158-2211，全新客户走状态机 initial 态 + MachineWrite 校验 stage/intent）→ $inc profile_revision；`disable_contact_agent`（2237-2261）切 normal+审计；`create_follow_up_task`（2262-2303）插 AgentTask{kind=follow_up, review_required:true, max_attempts:3, expires=run_at+48h}；**`send_contact_message`（2304-2326）**：经 **`agent::send_contact_message_gateway`**（生产发送网关，带 ManualContactSend{content, source, original_content_locked}）——管理台发消息**不绕 gateway**；`update_contact_profile`（2327-2417）：stage/intent 过 MachineWrite 校验（T8 旁路修复），stage 实际未写入绝不算变更（2363-2366），commitments 合并、$unset last_commitment、$inc profile_revision。
- 观测查询 5 个（2422-2479）：**直接构造 axum 提取器调兄弟模块真 handler**，Extension 塞 `management_admin(workspace_id)`（3263-3272：user_id 空、username="management-agent"、current_workspace=传入 workspace——多租户隔离关键）。
- 版本灰度 17 个（2480-2672）：同样直调 admin_ops_versions/domain_profiles/evolution/llm_providers 的真 handler；`release_evolution_proposal` 由本分支**补足确认串 "RELEASE"**（2610-2624，注释：管理 agent 已过 plan 确认门）；`provider_activate`/`provider_test` 强制覆盖 workspaceId 为可信值，**丢弃 LLM 注入的 workspaceId**（2638-2672，防跨租户读 apiKey）。
- 运营态批 2（2674-2819）：update_assist_override/custom_instructions/manual_tags 经 `management_expected_account_bound_arguments`（3309-3326）**强制 expectedAccountId=当前命令账号**（不匹配 → Conflict binding_mismatch）；**`write_deal_events`（2719-2764）要求 confirmed_admin**（确认后才有真人身份，marked_by=admin.username，verification 硬编码 staff_confirmed）；resolve_principal_escalation 剥 shortCode 进 Path（2802-2819）。
- 调参批 2：update_operation_domain / update_ask_human_policy 直调 domains.rs handler（2821-2846）。
- 策略编辑批 3（2848-2957）：edit_soul/publish_soul/edit_playbook/set_default_playbook/generate_playbook/optimize_playbook/edit_state_machine/promote_lesson——playbook 类经 `management_account_bound_arguments`（3293-3307）锁 accountId。
- 知识维护批 3（2959-3118）：verify/reject/archive/patch/split/merge/relate/batch_verify/apply_gap/dismiss_gap/import_text/import_image；**`import_knowledge_text` 要求 confirmed_admin 且 user_id 非空**（3089-3097，import-preview 由真人会话生成，Extension 塞 admin.clone() 而非合成身份 3102）。
- 批 4（3119-3236）：cancel_outbox → `admin_outbox::cancel_outbox_inner`（带 workspace+account 过滤）；approve_relationship_suggestion/approve_taxonomy_candidate → inner 函数 + ReviewActor::system；import_knowledge_pdf → base64 解码喂 `import_pdf_bytes`；`preview_campaign` = create_campaign + preview_campaign 两步（3193-3216）；`dispatch_campaign` 转发 (campaignId, specVersion, specHash) 三元组（3217-3236）。
- `resolve_contact_arg`（3328-3358）：contactId 优先（find_contact_by_id + account 匹配），否则 wxid/recipient 按 (workspace,account,wxid) 查。
- **ToolOutcome 断言（assert_tool_outcome，1919-1962）**：send 工具读 gatewayStatus——outbox_enqueued/skipped_duplicate → Accepted（持久受理≠送达）、sent → Succeeded、其它 → Failed、缺失 → Unverified；写库类 matched=0 → Failed；ok:true → Succeeded；只读返回结构即 Succeeded；兜底 Unverified（诚实优于好看）。
- **状态映射（tool_call_status_for_outcome，1972-1986）**：dry_run 优先；Accepted → "accepted" **绝不落 succeeded**（回归测试 3928-3963 锁死）。
- `build_execution_summary`（1989-2006）：✅已完成/📨已受理/❌失败/⚠️待核实/⛔执行结果未知 分行汇报。

### 2.4 contacts.rs（3432 行）——纳管/画像/记忆/成交/辅助覆盖全端点

**列表/计数/搜索**
- `list_contacts`（176-276）：filter = workspace+account+hidden_from_pool≠true+可选 agent_status+q（`escape_regex_literal` 转义后 4 字段 $or regex i）；排序 last_inbound_at↓,updated_at↓，limit 1-500；读时双保险过滤非真人 `is_operatable_person`（242-248）；**读时兜底富化**：读 roster 快照一次补空 nickname/avatar（只补空不覆盖，222-271）；最新入站预览走**一次聚合**（`latest_inbound_preview_pipeline` 309-328：$match inbound + $sort + $group $first）防 N+1，失败 fail-soft 空 map（278-307）；预览生成 `preview_label_for_type`（142-157）：text/None 原文截 30 字符（`truncate_preview` 128-135），其它类型固定标签（[图片][语音]…绝不读 XML content），**纯静态非 LLM**（normal 联系人不调 LLM 产品红线，注释 127,140-141）。
- `count_contacts`（360-379）：`contact_count_filters`（334-352）与 list 同源 workspace+account+hidden 过滤 + **$nor 非真人 DB 侧排除**（单一数据源 `mcp::non_human_exclusion_filter`，与 list 的读时过滤等价口径）；normal = all - managed 饱和减。
- `search_contacts_endpoint`（386-419）：纯查询不写库（波 A3）；`import_contacts_endpoint`（427-479）：candidates 直导或 query 搜后导，经 upsert_contact_from_value；`search_import_contacts`（483-537）：DEPRECATED 旧合并入口，响应带 deprecationNote。
- `roster_endpoint`（637-748）：快照优先策略——force=触发后台单飞刷新+立即回旧快照（refreshing:true）；非 force stale 自刷；无快照 syncing:true 后台拉。拼装本地 agent_status（not_imported/normal/managed）；`roster_identity_rank`（628-635）身份完整度稳定排序（有名>有头像>无）；返回 fetchedAt 供前端感知快照龄。

**纳管链（enable/batch/disable/hide）**
- `enable_agent`（1927-2050）：expectedAccountId 必填 + humanProfileNote 必填 + `find_contact_by_id_for_account`；非真人 400；**account 必须已注册**（否则 webhook 会拒收 AI 永不回复，P1 注释 1947-1963）；**自身 wxid 拒绝**（1964-1980）；**轮换 enrollment_token（uuid）**（1988-2005）→ `agent::build_initial_operation_profile`（LLM）→ `apply_generated_profile_to_contact`（同步路径 task_claim=None）；未 commit（enrollment 已漂移）→ Conflict（2026-2030）。
- `apply_generated_profile_to_contact`（755-852）：同步 enable 与异步 worker 共用落库内核。task_claim=None 才写 `agent_status:"managed"`（**异步 worker 绝不授予 managed**，防迟到 worker 复活已 disable 联系人，注释 783-788）；#72 老客户保留 stage/operation_state/commitments，全新客户走状态机 initial 态（`initial_operation_state_key`）+ `validate_generated_stage_intent`（2576-2613，MachineWrite 越界 drop）；commit filter（`initial_profile_contact_commit_filter` 854-887）绑定 enrollment_token + claim generation（$or：token 不同可覆盖 / 同 token 且 task_id 匹配且 generation ≤ claim 的可重放）。
- `batch_enable_endpoint`（1573-1918）：sharedNote/candidates 必填；source ∈ {pool, roster}；pool 源整批预校验——contactId 必填去重 + 一次 $in 查回（agent_status=normal + hidden≠true + workspace+account 锁定），数量不齐/任一 wxid 不匹配 → **整批零写 Conflict**（1611-1676）；统一 playbook 解析；**初始 operation_state 同步写入**（竞态修复：等异步回填时客户来消息 gateway 推 last_agent_run_at → is_previously_operated 误判老客户 → 永拿不到 initial 态，注释 1686-1696）；逐候选：非真人拒（1705-1719）、自身 wxid 拒（1721-1735）、已 managed 只刷新元数据（1803-1818）、新纳管走 **durable enrollment intent**（`create_initial_profile_enrollment_intent` 1472-1571：uuid token + task 单飞键 active_task_key="initial_profile" + prepared_commit 两阶段，insert DuplicateKey → 复用现存 intent 并校验 allow_contact_insert 策略一致）→ `reconcile_initial_profile_enrollment` 立即推进；stale 一代把单飞键占住时**重试一次**（1831-1877）；pool 源失败 → Conflict version_conflict，roster 源 continue；返回 {enabled,queued,rejectedSelf,rejectedNonHuman}。
- `reconcile_initial_profile_enrollment`（1213-1470，pub(crate)，task worker 也调）：恢复 committing 态的 enrollment——contact CAS（filter：wxid+hidden≠true+agent_status≠managed+可选 _id/updated_at OCC + upsert 仅 roster 源）；DuplicateKey 当 OCC miss 再查本代 token 是否已赢（1374-1406）；未 commit → task 转 cancelled(stale_enrollment)；已 commit → task 释放为 pending(enrollment_committed)。无 committing 行时校验单飞键现任者是否同代 managed，不是则退休该 stale task（cancelled/stale_enrollment_generation）（1230-1298）。
- `handle_initial_profile_task(_with_claim)`（943-1054）+ `reconcile_initial_profile_commit`（1056-1180）：worker 异步画像——contact 没了/已取消托管/token 缺失 → 直接标 sent(contact_gone/unmanaged/missing_enrollment_token)（`mark_initial_profile_task_sent` 897-933，修复 W-Batch3 [1-01] 漏写终态被反复 reclaim 的失效链）；有 claim 时走 prepared-commit 两阶段（prepare_task_commit_if_owned → reconcile 从 prepared_commit 重建参数 → apply → committing_filter CAS 终态 sent(profiled/stale_enrollment)）。
- `disable_agent`（2052-2090）：切 normal + **轮换 enrollment_token**（旧 token 的迟到 worker 无法 commit）→ `cancel_active_initial_profile_tasks`（2145-2184：update_many 单飞键任务 → cancelled+清 claim/prepared 字段）+ 审计。`hide_from_pool`（2100-2140）：同上再加 hidden_from_pool=true（不删记录防 webhook 重建档；单向移除无恢复端点）。
- `revoke_principal_exemption`（2191-2228）：$unset domain_attributes.PRINCIPAL_PRODUCT_EXEMPTION_ATTR + 审计（豁免长期有效须显式撤销）。

**画像/记忆/健康**
- `update_profile_note`（2230-2321）：重新生成画像（LLM build_initial_operation_profile）；老客户只写 note+agent_profile+profile_attributes，不写 tags（裸字段已废）不碰 manual_tags；OCC filter 锁 workspace+account。
- `update_operation_profile`（2615-2752）：手动表单——stage/intent/relationship_type 全走 **AdminWrite**（越界 Reject → 400 硬拒，与 guide 路径 skipped 不同）；alias 归一先于 stage_changed 判定（M1，2638-2662）；`stage_changed` 纯函数（2568-2570：None 绝不算变更防误刷 stagnation 计时）；profile_attributes 空则跳过（M13 防清空 AI 积累，2680-2686）；playbook 换绑校验存在；$unset last_commitment + $inc profile_revision。
- `add_deal_event`（2765-2804）：见 shared `add_outcome_event_inner`；expectedAccountId 冻结防切号误记。`list_outcome_events`（559-573）：occurred_at??marked_at 倒序全量。`list_entitlements`（584-613）：运行时投影 `project_entitlements`（read 端点 cap=usize::MAX 不受 prompt 软上限）。
- `analyze_contact_profile`（2806-2886）：AI 重析画像（note 缺省用备注/昵称构造），老客户保留逻辑同上。
- `get_operating_memory`（2888-2896）：ensure（会播种写库）；`update_operating_memory`（2898-2929）：F-019 四组 Option 部分更新（`build_operating_memory_set_doc` 2934-2949，缺组不 $set 防清空）。
- `get_contact_memory_card`（2951-2971）：effective_memory_card_for_contact + 状态机 initial 态回落。
- `list_contact_memory_candidates`（2973-3004）；`run_contact_memory_consolidation`（3006-3017）→ **`agent::run_manual_memory_consolidation`**（LLM 记忆整合任务）。
- `get_operation_health`（3019-3039）：**read_operating_memory 只读投影**（GET 不建行）+ latest_review + quiet_hours 三值。
- `update_assist_override`（2371-2413）：mode 闭集 {default→$unset 回落账号级, force_on/force_off→$set}（`is_valid_assist_mode` 2325-2327）写 domain_attributes.ASSIST_MODE_OVERRIDE_ATTR。
- `clear_referral`（2347-2366）：$unset referred_specialist_at + referred_card_id 两键（红线 §6.3，`build_clear_referral_update` 2333-2342，两键都清才彻底退回主动运营态）。
- `update_custom_agent_instructions`（2423-2473）：上限 1000 字符，trim 空 → Null 清空；非空存原文（保留内部空白/换行）；注入下一轮 user.reply 的 Operator Instruction 层。
- `update_manual_tags`（2485-2525）：normalize（trim/去空/去重保序 2528-2537）+ validate（≤32 条、单条 ≤64 字符 2547-2562）；写 manual_tags+updated_at+by；**AI 永不覆盖本字段**。

### 2.5 campaigns.rs（1876 行）——冻结规格与派发扇出

- **圈人两阶段**：粗筛 `build_segment_coarse_filter`（33-72）：workspace+account+managed + 可选 domain_attributes.customer_stage + product_ids 非空时 outcome_events $elemMatch（productRef.productId $in + KC-05 "缺字段=默认值"显式化：verification $in 白名单 OR $exists:false；eventKind $ne reversal 一箭双雕）；精筛 `contact_matches_segment`（75-124）：复用 G4 `project_entitlements` 判净持有（退款抵消）/售后(in_aftercare/expired/any)/价值分层（`compute_customer_value_cents`+`classify_value_tier`，阈值来自 config `value_tier_mid/high_threshold_cents`，264-269）。`resolve_segment_contacts`（285-315）：cursor limit=max_audience+1，超 `campaign_max_audience` → 400 要求细化条件（KC-04/07 防全量驻内存 + 防静默截断）。
- **生命周期**：create（317-374，spec_hash=SHA256(workspaceId+accountId+title+intentText+segmentFilter 的 canonical JSON)，177-192；segment normalize：trim/排序/去重，156-175）→ update_campaign_draft（376-452，OCC expectedSpecVersion，`campaign_spec_version_filter` 511-520 对 v1 兼容缺字段老行；成功 version+1、状态回 draft、$unset targetCount）→ preview（454-502，仅 draft/previewed 可预览，回 targetCount+5 个抽样+当前 specVersion/specHash）→ dispatch（522-590）。
- **dispatch 派发**（KC-02 `dispatch_allowed_from_status` 507-509：draft/previewed/dispatching 可派、completed 拒、未知 fail-safe 拒）：
  - 非重入：hash+version 与当前 spec 严格匹配否则 Conflict("campaign_spec_confirmation_mismatch")（555-560）→ 重圈一次受众（命中 0 → 400）→ `freeze_campaign_dispatch`（610-676）：find_one_and_update CAS（filter：status ∈ draft/previewed + specVersion 匹配(v1 兼容) + specHash 匹配或缺失）→ $set status=dispatching + dispatchGeneration+1 + **冻结 dispatchSpecHash/dispatchAudience/dispatchIntentText/dispatchStartedAt/targetCount**；抢锁失败回读，若已是 dispatching 且快照匹配（`validate_frozen_dispatch` 592-608）则接管重入，否则 Conflict。
  - `materialize_campaign_dispatch`（847-921）：逐人 `ensure_campaign_task`（678-845）：
    1. campaign_sends 插 `{campaignId, contactWxid, dispatchGeneration, specHash, taskId, status:"prepared"}`，taskId=**确定性 ObjectId**（SHA256("campaign-task:v1:{id}:{generation}:{wxid}") 前 12 字节，211-222）；DuplicateKey → 校验现存行同代/同 hash/同 task 否则 Conflict identity_conflict（712-731）。
    2. tasks 插 follow_up 任务（`build_campaign_follow_up_task` 226-260：**status="committing"** + prepared_commit_kind="campaign_fanout"，48h expiry，review_required=true——发送链路完全复用 task worker→gateway→outbox→MCP）；DuplicateKey → 按五键 count 校验（753-776）。
    3. send prepared→enqueued CAS（777-797）→ task committing→pending 释放 CAS（798-843，已释放的幂等 count 校验）。
  - 全员 enqueued 计数核对 `ready != expected` → Conflict("campaign_fanout_incomplete:{r}/{e}")（859-878）→ campaign dispatching→completed CAS（879-919，已完成幂等）。
  - `reconcile_campaign_dispatches`（923-944，pub）：worker 扫 status=dispatching 的活动续跑 materialize（崩溃恢复），错误只 log 延后。
- **结果报表**：`campaign_sends_report`（1091-1204）：台账 + 一次 $in 拉 run logs（`campaign_run_logs_filter` 1083-1089：workspace+source_event_id $in taskId.hex+source_kind=follow_up_task）内存取每 task 最新（max _id）+ 批量补客户名 → `classify_send_outcome`（953-1036）7 桶归类：skipped_duplicate→skipped；无 log→pending(not_yet_run)；outbox sent→sent（**最高优先级压过一切 status**）；failed_terminal/canceled→canceled；delivery_unknown→unknown；partially_sent→canceled(保留原因)；pending/in_flight→pending；run status 逐值归桶（allowed/enqueue/quiet_hours→pending；频控硬约束 14 值→blocked；四个 AI 请示态→**escalated**（走幕后决策源，非失败漏推）；取消 6 值→canceled；未知→unknown 绝不强划 sent）；`build_sends_summary`（1040-1072）标量+reason 二级 map。
- `list_campaigns`（1242-1261）：`CampaignListItem` 投影（1209-1238）**不泄漏 workspace_id/segmentFilter/intentText/accountId**。
- 注意：campaigns/campaign_sends 集合字段是 **camelCase**（workspaceId/campaignId/dispatchGeneration…），与 contacts/tasks 的 snake_case 不同（Campaign/CampaignSend 模型 serde rename）。

### 2.6 guides.rs（1254 行）——预览冻结 / apply 幂等

- **preview_user_operation_guide**（498-648）：instruction 非空 + validate_account + contact 归属校验；**read_operating_memory 只读**（预览绝不创建业务状态，缺行冻结为 memory_insert 仅在 Apply 时插入，注释 513-515）；聚合 health/playbook/quiet_hours；**注入合法值**（状态机 states keys + taxonomy cache 的 customer_stage/intent_level canonical(label) 对，528-554，治 LLM 产越界值的源头）；经 `generate_agent_json`（prompt_key="user.guide.preview"）产出 JSON → **`freeze_guide_plan`**（251-441）冻结为 `GuideFrozenPlan`：
  - prepare_contact_changes/prepare_memory_changes/prepare_playbook_changes/prepare_domain_changes 四路 set_doc；`strip_timestamp_fields`（199-209）摘出 *_updated_at 冻结为字段名单（apply 时统一 materialize now）；`retain_effective_changes`（211-235）**逐键 diff 当前值，无实际变化的键剔除**，有变化的记 `GuideAuthoritativeChange{target,field,before,after}`；
  - playbook patch 必须绑定当前 playbook id（否则 Conflict guide_playbook_changed）；domain patch 校验 current config id+version 匹配；`playbook_affected_contacts` = 绑同 playbook 的 contact 数（349-366）；
  - applied_fields（BTreeSet 排序）+ skipped_fields（越界字段 + 不在 allowed 11 键白名单的 + 无有效 playbook 变更的 playbookPatch，382-410）+ OCC 基线（contact_updated_at/memory_updated_at/memory_insert/playbook id+version/domain id+version+updated_at）。
  - `plan_impact`（443-466）从**冻结产物推导权威 scope**：domain runtime → workspace_user_operations；playbook_set 非空 → shared_playbook；否则 current_contact。`candidate_hash`（77-90）=SHA256(bson(workspace+account+contact_id+frozen_plan))。落库 status="pending"。
- **apply_user_operation_guide**（650-816）：
  - 身份四键 filter（_id+workspace+account+contact），不匹配区分 identity_conflict(409)/NotFound（671-693）。
  - **applied 幂等回放**：status=applied → `validated_apply_receipt`（104-151：重算 hash、scope 重推导、receipt 与 plan 全字段核对）返回原 receipt（695-698）。
  - hash 三重校验（存 hash==请求 hash==重算 hash，703-718）+ scope 一致 + **强确认闸**：scope≠current_contact 且未带 confirm_global_impact → 400 guide_global_confirmation_required（725-729）。
  - **租约夺取**：`guide_claim_filter`（48-75）——pending / failed(同协议版) / applying 且 apply_started_at < now-5min（GUIDE_APPLY_LEASE_MS，44）三态可夺；$set status=applying + apply_token(uuid) + apply_protocol_version=3；夺取失败回读（applied 幂等 / 其它 Conflict not_pending）。
  - **`apply_claimed_user_operation_guide_v3`**（818-1105）：**单个 Mongo 多文档事务**内完成——contact update（filter 带 updated_at OCC，matched≠1 → guide_contact_changed）；memory insert（冻结基线 memory_insert：insert 前查重，已存在 → guide_memory_changed）或 update（updated_at OCC）；playbook update（id+version+published OCC，有变更时 $inc version）；domain config update（id+version+updated_at+current_version OCC）；插审计事件 `user_operation_guide_applied`（dedupe_key="guide_apply:{preview_id}"）；preview applying→applied CAS（`guide_owned_apply_filter` 481-496 锁 apply_token，modified≠1 → guide_preview_lease_lost）。全部写带 GUIDE_APPLY_GUARD_FIELD=preview_id.to_hex()（46）。事务 abort on error；commit 循环重试 UnknownTransactionCommitResult（1097-1103）。
  - 失败落终态：Conflict 含 changed/stale → status="stale"，其它 → "failed"（含 apply_error 前 500 字符），filter 锁本次 apply_token（778-813）。

### 2.7 guide_profile.rs（944 行）——AI 生成候选 DomainProfile

- `generate_domain_profile_candidate`（324-489）：businessDescription/profileId 必填；system = active profile 的 `methodology_generator_preamble` 或领域中性 `PLAYBOOK_METHODOLOGY_SYSTEM`（C3 去销售偏见，344-355）；上下文注入最近 40 条知识切片标题（`gather_knowledge_titles` 195-232，只标题不灌全文控 token）；经 `generate_agent_json`（prompt_key="guide.domain_profile.draft"）。
- 后处理管线（关键顺序）：① **stateMachine 在 normalize 之前整体抽出**（384-387，H13：状态机内层 key 是 camelCase `allowedFrom/allowFromAny/initial`，运行时引擎按 camelCase 读，过 normalize 会被 snake 化静默失效——测试 883-943 钉死）；② `extract_suggested_values`（138-179）提取各维度 suggestedValues 并 remove（防污染 ProfileDimension 反序列化）；③ `normalize_json_keys`（62-77 递归 camelCase→snake_case；`to_snake_case` 42-59 已知限制：末尾连续大写不分隔，测试 530-535 锁定）；④ `coerce_scalar_string_fields`（92-125）把 LLM 偶发给成对象/数组的标量字段（description/prompt_fragment/soul_override/methodology_override/conversation_mode_policy + 嵌套 profile_dimensions[].description）压平成 JSON 文本（G32）。
- 落库策略：候选强制 version=0/current_version=false/release_status="draft"/is_active=false/seeded_by="generated_by_ai"（409-420）——**AI 生成 = 候选，必须人审 publish+activate**；stateMachine 过 `domains::validate_state_machine` 才落 generated_state_machine，失败回落 None 只 warn 不阻断（426-455）；经 `domain_profiles::append_domain_profile_draft` 落库；suggestedValues 落 **taxonomy 候选层**（`upsert_candidate` scope="global" confidence=10，绝不直进 system_taxonomies，失败软化 let _，460-480）。

### 2.8 evaluations.rs（916 行）——评测场景 CRUD + 公式遵从度

- 场景 CRUD（64-201）：list（workspace+可选 tag/status）；create/update 前置 `validated_scenario_status`（579-604：status ∈ {active,draft}；**active 必须 groundTruth 完整**——每个公式数值 0..10，`validate_ground_truth` 546-563 + `strict_score` 565-577 只认 Int32/Int64/Double/Decimal128 且有限且在 [0,10]）+ `validate_scenario_request`（606-636：scenarioId/title/inboundMessages 非空，accountId 给定时 validate_account）；delete 按 _id+workspace。
- `run_formula_adherence_evaluation`（207-503）：场景 filter=workspace+active+account 三态（`active_scenario_filter` 534-544：无 account 字段/null/精确匹配）+ 可选 scenario_ids/tags；空场景 → 200 degraded=true（CI 不中断）；**预算**：total = runtime.simulation_token_budget × 场景数（264-266），循环内超额 break 标 degraded=evaluation_budget_exceeded（300-304）；公式规格 `evaluation_formula_specs`（518-532：active profile 的 business_formulas，空则回落内置销售四公式 + `score_key_for` fallback 映射 762-771；单一真相源一致性由测试 870-883 锁）；contact 来源：payload.contact_id（find_for_account）或 `scenario_contact_from_seed`（638-743：种子字段拼 Contact，operation_state 缺省回落状态机 initial 态 H13）；逐场景 `agent::simulate_user_dialogue_with_budget`（**LLM 全链路影子跑**）→ 取 last turn 的 review.formulaBreakdown[公式] 或 scores[score_key] 与 groundTruth 求 |delta|，adherence = max(0, 1 - meanDelta/10)（371-428）；全公式缺失 → invalid=all_formulas_missing 不以 0 分参与平均（405-423）；unknown_usage_calls>0 → degraded break；写审计事件 `formula_adherence_evaluated`（452-485）。

### 2.9 simulations.rs（371 行）——影子对话/场景评估

- `simulate_user_operation_dialogue`（37-70）：apply_memory=true → 400（影子模拟不落记忆）；messages trim 去空取前 12；contact 归属校验；→ **`agent::simulate_user_dialogue`**（影子跑 decision+review+gateway，不真发）。
- `run_user_operation_evaluation`（72-138）：按 active profile 的 transaction_facts_enabled 选四场景组（`evaluation_scenarios` 140-190：transactional 组含产品质疑/成交推进，relationship 组去销售词）；`judge_user_operation_scenario`（192-254）：**复用生产同源终态**——turn.status ∈ {would_send,no_reply} = passed，review_blocked/gateway_blocked/blocked_by_safety_guard/blocked_unverified_product_claim/held_by_ai_policy/其它 = failed（S1.3：不再自算 0-100 死阈值，旧幻觉闸/grounding 闸死规则已移除，注释 206-221）；scores 仍透传展示。

### 2.10 accounts.rs（352 行）

- `list_accounts`（33-63）：workspace 过滤，alias 排序；`mcpKeyConfigured` = 账号级 key 非空 OR 全局 config key 非空（57）。
- `sync_accounts`（65-174）：MCP account_list → 逐账号 upsert；**KE-06：mcp_base_url/mcp_api_key 仅 $setOnInsert**（158-166，后续 sync 不覆盖管理员手配值）；$set 补全必填字段防不完整记录反序列化失败（153-156）。
- `update_account_mcp_key`（176-215）：mcpApiKey+expectedAccountId 必填；filter _id+workspace+account 三键，matched=0 → Conflict("account_identity_conflict")。
- `login_begin`（259-299）/`login_poll`（316-352）：MCP login_begin/login_poll 透传；带 account_alias 时按 alias 查回账号走 `logged_call_for_account`，否则默认 credentials `logged_call`。

### 2.11 products.rs（390 行）

- workspace 级产品实体（product_id 业务主键 slug）；`OutcomeEvent.product_ref` 快照引用（下单拷贝名/价/sku），改名/下架不污染历史成交（注释 1-18）。
- list（activeOnly 可选）/create（product_id+name 必填 + `validate_product_money` 119-135：price 非负整数分 + currency ISO-4217；(workspace_id, product_id) 唯一索引，DuplicateKey → 友好 400，192-204）/update（**全量 PUT 语义**：price/currency/sku/summary None → $set Null 显式清空，310-330）/archive/restore（status 闭集 {active,archived}，debug_assert 268）。IDOR：workspace_id 恒由会话注入（179-180）。

### 2.12 playbooks.rs（945 行）

- list（100-128）：先 `ensure_default_playbook`（773-802：无默认则种 `prompts::default_playbook` 并 make_default；并发竞态输者 4 次×10ms 有界收敛窗口读赢者）。
- create（130-184）：显式 is_default 或该账号无 published 默认 → make_default；**默认权只由事务授予**（`insert_playbook` 540-645：无现任直接插（DuplicateKey→Conflict）；换默认走事务——limit(2) 查重防 multiple_default、demote 现任 + insert 新任原子可见；commit 循环重试 UnknownTransactionCommitResult，29-41）。
- update（186-266）：accountId+expectedVersion(>0) 必填；`playbook_mutation_filter`（526-538）_id+workspace+account+version OCC；**isDefault 不可经 update 改**（专用 set-default 端点，220-227）；成功 version+1、release_status 恒回 published。
- set_default（268-291）→ `switch_default_playbook`（656-771）：事务内 target 按 version OCC 查回（release_status ∈ draft/published）→ 已是默认幂等返回 → demote 现任 + promote target（$set published+is_default+$版本不变）。
- generate（293-369）：**LLM**（prompt_key="playbook.generator"，system 可被 active profile 的 methodology_generator_preamble 覆盖 C3）→ 落 **created_by="agent" + release_status="draft" + is_default=false**（AI 生成绝不获默认指针，注释 333-335）。
- optimize（371-491）：expectedVersion OCC 查回现行 → **LLM**（prompt_key="playbook.optimizer"）→ 生成**新的非默认 draft 候选**（created_by="agent_optimized"，version=expected+1 checked_add），不修改现行生产方法论。

### 2.13 souls.rs（153 行）

- list（30-63）：先 `ensure_default_souls`（143-153 = ensure_prompt_pack_v2，写了则 bump prompt_pack_version）。
- create（65-86）→ `soul_versions::append_version`（seeded_by="manual"）；update（88-118）：三字段必填 → `append_edited_draft`（只出草稿）；publish（120-141）→ `soul_versions::publish_version` + **bump prompt_pack_version**（LRU 缓存失效）。注意 publish 提取器顺序 State+Path+Extension（management.rs:2862 注释亦确认）。

### 2.14 domains.rs（730 行）

- list（52-77）：先 `ensure_operation_domains`（483-528：缺省 seed default_domain_configs；user_operations 状态机为空时补 default 状态机）；默认只回 current_version≠false，`?includeAllVersions=true` 回全版本流水（Phase E 灰度面板）。
- get（79-87）/get state-machine（169-177）：`find_operation_domain`（530-559）current_version=true 优先，回落无字段老行。
- `update_operation_domain`（89-167）：七文本字段必填（380-394）；user_operations 的 runtime_parameters 全量校验归一；`validate_state_machine`（399-466，pub(crate) 供 guide_profile 复用）：states 元素必须对象、key 非空唯一、**非空 states 必须至少一个 initial:true**（H13，427-440）、allowedFrom 只引已知态；`normalize_state_machine_allow_from_any`（469-481：allowFromAny=true → allowedFrom 清空）；$set 只打 current_version≠false 行；**G06 联动**：改状态机后 `reconcile_state_policies_for_machine`（重派 policy，seeded_by="statemachine_edit:{domain}"，144-153）；**作息联动**：user_operations 保存后 `webhooks::reconcile_workspace_reply_obligations` 重排未跨发送边界的被动回复义务（157-162），返回 reconciledReplyObligations。
- `update_operation_domain_state_machine`（179-233）：裸状态机 Document PUT，同样 validate+normalize+policy 重派+作息重排。
- `put_ask_human_policy`（237-315）：**决策链强校验**——每位 decider wxid 非空 + 必须绑定本 workspace 真实账号 + 该 wxid 必须在该账号通讯录中（245-291，防内部卡从错误账号发出）；quiet_hours 小时 0-23；$set 到 current_version=true 行（不 bump 版本）。
- `reset_operation_domain`（317-353）：从 default_domain_configs 取默认 → `append_default_operation_domain_version`（事务式版本追加，[外]admin_ops_versions）+ policy 重派（seeded_by="statemachine_publish:admin_reset"）。

### 2.15 prompt_templates.rs（436 行）——三闸

- 三闸体系（实现在 `crate::prompt_guard`，经 management_prompt_edit.rs:7 re-export）：闸 1+2 = `validate_prompt_edit` 字面双闸（禁用词 + 锚完整性，确定性硬闸 **force 不可绕**）；闸 3 = `review_prompt_edit` LLM 红线语义审查（审 diff 增量，force=true 可跳过——管理者已逐字核对）。
- list（72-110）：先 ensure_prompt_pack_v2（写了 bump 版本）；agent_kind/layer 过滤。
- create（112-153）：字面双闸（整篇过闸语义正确，无 old 基线不加 LLM 闸——publish 关口兜底，注释 118-121）→ `append_version`（draft）。
- update（155-243）：字面双闸 → **prompt_key 不可变**（179-181 Conflict）→ 非 force 走 LLM 闸三分支：Pass 继续 / Reject → 400 / **NeedsHumanConfirm → 200 返回 {status:"needs_human_confirm", reason, diff}**（前端弹框确认后带 force=true 重提，200-210）→ append_edited_draft。
- publish（245-327）：draft→active 最终生效点——字面双闸（force 不可绕）+ 非 force LLM 闸（old = `load_current_for_publish` 当前生效版内容）→ publish_version + **bump prompt_pack_version**。
- reset-system-pack（329-351）：**服务端确认串** confirmation 必须逐字 == "RESET PROMPT PACK"（55-70，deny_unknown_fields 拒带多余字段）→ `reset_prompt_pack_v2_as_actor`（物理删除重种，显式销毁性维护动作）→ bump 版本。

### 2.16 events.rs / conversations.rs / tasks.rs / reviews.rs

- `list_events`（events.rs:28-74）：workspace+account+可选 kind 精确过滤（高频事件挤出窗口问题的解法）+ limit 1-500；details 走 relaxed extjson 防 BSON 包装泄漏。
- `list_messages`（conversations.rs:17-53）：find_contact_by_id（workspace 隔离）→ 按 (workspace, account, contact_wxid) 查 messages 最近 100 条。
- `list_tasks`（tasks.rs:64-111）：**kind 白名单只展示客户触达类**（follow_up/inbound_reply/deferred_inbound_reply/principal_decision_relay，F-003 隐藏内部作业）。
- `list_agent_runs`（113-145）：workspace+account+可选 contact_wxid，limit 1-200。
- `list_llm_usage`（147-252）：聚合 summary（totalCalls/totalTokens/cache hit/miss/knownUsageCalls——usage_known 或 cache_hit 或任一 token 字段非零）+ 明细 limit 1-300 + itemsTruncated 标记。
- `review_task_now`（277-300）：`claim_task_by_id_for_account`（workspace+account 锁定 claim）→ `execute_claimed_task` **同步驱动 task worker 执行链**（间接触发 gateway→LLM）。
- `cancel_agent_task`（302-404）：CAS pending/retry/failed/running/outbox_enqueued → cancelled(admin_cancelled)（ReturnDocument::Before 拿被作废的 decision 绑定）；**级联**：kind=initial_profile → 轮换 contact enrollment_token（363-389）；有 outbox_decision_id → `agent::cancel_for_decision` 取消未发 outbox 行（390-400）。
- reviews.rs：`list_decision_reviews`（106-160）workspace+account 必填 + contact_id（校验归属）/contact_wxid 过滤；每条 review 调 `fetch_run_status`（337-391）关联同 run_id 的 AgentRunLog 取 final_review_status/holdCategory/**autonomy_protocol**（301-325：decision 里 9 个 R1.1 自治协议字段，全空 → None 优雅降级）/outbox_status → `decision_review_json`。`get_decision_review`（162-201）同构单条。post-decision 恢复三动作（271-296 → `recover_post_decision` 203-269）：前置 post_decision_status=="failed_terminal"；retry/regenerate 需 payload 快照在（233-239）；regenerate 需 safe_to_regenerate==true（240-246）；CAS 锁 failed_terminal（`recovery_update` 46-104：retry 保留投影/regenerate 清投影/discard 清 payload 落 discarded）+ 审计 last_recovery。

### 2.17 media_assets.rs / assets.rs / referral_cards.rs

- media_assets.rs 安全红线（1-7 注释）：落盘路径只经 `safe_relative_path`（workspace/sha 分片，原始文件名只存 DB 展示）；大小/扩展名白名单；**上传默认 review_status="draft"（AI 不自我核验，必须人 approve 才可发）**。
- scope 冻结（`AssetScopeRequest` + `content_asset_scope_filter` 44-83 + `find_content_asset_for_scope` 85-120）：expectedScope ∈ {account(必带 expectedAccountId), workspace(必不带)}；workspace 资产只匹配 account_id:null 绝不从当前账号推断；scope 不符 → Conflict("content_asset_scope_conflict")，协议非法保持 400。
- `upload_media_asset`（122-320）：multipart 解析（title/mediaType 必填、target_stages/tags 逗号分隔、requiresPrincipalApproval）；大小 ≤ media_max_file_size_mb；mediaType 闭集 {image,file,video}（25-27）；`sanitize_ext` 白名单；**target_stages 归一前移到落盘之前**（越界 400 不留孤儿文件，226-236）；**文件协议**：lock_paths（同 SHA 进程锁）→ stage_bytes（写 pending）→ Mongo insert → publish_staged（原子 rename）；DB 失败 → settle 补偿；publish 失败 → delete DB 回滚（回滚不确定则保留 pending 交 reconciler，300-313）。
- `review_media_asset`（331-385）：status ∈ {approved,draft}；scope 校验后 $set；**审计 media_asset.reviewed fail-soft**（写失败只 warn 不回滚，360-383）。
- `update_content_asset_meta`（407-479）：部分更新（Some 才 $set；serde 不区分缺失与 null，清空走 ""/[]，417-419 注释）；不动 file_*/media_id/review_status/sendable；target_stages 归一（scope 取 asset 自身 account_id）；min_inject_tier 走 `assets::normalize_min_inject_tier` 闭集归一。
- `replace_content_asset_file`（500-709）：**换文件三大副作用**（`file_replace_effects` 485-495 纯函数钉死）：review_status 退 "draft" 强制重审 + media_id 清 Null（防 ensure_media_uploaded TTL 内发旧文件）+ 新 file_*。双路径锁（新旧 path）→ 锁下重读校验并发（580-585）→ OCC filter 带旧 file_path+updated_at → publish 失败**逐字段回滚**（restore_optional 宏，642-686）→ 旧文件无兄弟引用才物理删（fail-soft，689-707）。
- `toggle_content_asset_sendable`（721-743）：sendable 与 review_status 正交（停用不动审核态）。`delete_content_asset`（748-803）：锁下重读 → 删 DB → 同 file_path 零引用才物理删（防误删共享文件）。
- assets.rs：`list_content_assets`（55-122）workspace + 可选 accountId（$or null/精确——workspace 共享资产可见）/kind/tag；`create_content_asset`（124-168）纯文本资产（kind/title 必填，min_inject_tier 归一 {lean,relational,full} 非法落 full，26-32）。
- referral_cards.rs：create（51-92）**恒 draft+enabled=false**（红线：管理员审核+启用后 AI 才可引荐）+ target_stages 归一；list（95-120）；review（123-184）status ∈ {approved,draft} + 审计 fail-soft；toggle（187-248）+ KE-05 审计；delete（251-306）**先查快照后删**（删后拿不到字段写审计）。

### 2.18 send_ledger.rs / operation_view.rs / outcome_metrics.rs

- send_ledger.rs（全只读）：`contact_send_history`（54-90）(workspace,account,wxid) 最近 100 条；`send_ledger_stats`（93-137）按 target_id $group 排行榜（sentCount/contactCount/responseRate/stageAdvanceRate，**率以已评估条目为分母**防新发未评估拉低）；`send_ledger_overview`（140-176）总量聚合；`agg_count`（17-22）兼容 $sum 返回 i32/i64（否则静默清零）。
- operation_view.rs：`active_view`（27-110）只读聚合——active profile 维度声明 + taxonomy 取值字典；kind 集 = profile_dimensions ∪ {relationship_type, conversation_mode, objection_type, value_tier, churn_reason, purchase_lifecycle}（52-80，admin 直写维度与 seed 维度恒补下发）；先 find_or_load 预热 taxonomy cache（82-86）。
- outcome_metrics.rs：`list_agent_outcome_metrics`（29-71）workspace+account+可选 horizon/日期区间，limit 1-365；4 个 Option<f64> 率序列化 number|null（前端 null 显示"暂无数据"）。

---

## 3. 跨文件机制

### 3.1 管理台一次危险操作：自然语言 → 执行的完整链路

以"给客户 X 发一条消息"为例（涉及 Dangerous 工具 `wechatagent.send_contact_message`）：

1. **建会话**：POST /management-agent/sessions（management.rs:653-679）→ management_sessions 插 {workspace, account, dry_run 默认}。
2. **发指令**：POST /sessions/:id/messages（681-920）→ 校验 account/session → 落 user message → 拉 MCP 工具目录 → `merge_product_tools` **先摘除 message_send_* 原生发送工具**（1287）再注入产品工具 → `management_context` 聚合联系人/资产/playbook/活动上下文 → `build_management_plan` 经 `generate_agent_json`（LLM，prompt_key="management.plan"）产出 plan → `apply_locked_send_content` 从指令提取逐字锁定内容覆盖 LLM 产出的 content（originalContentLocked=true）→ `management_plan_hash` SHA-256 冻结 → `plan_requires_confirmation(["wechatagent.send_contact_message"])` = true（tool_effect=Dangerous 非豁免）→ AgentCommandRun{status:"pending_confirmation", plan, plan_hash} 落库，**不生成 execution_token、不执行任何 tool_call**（791-807 传空 slice）→ assistant 回"待确认：{summary}"。
3. **人确认**：POST /commands/:id/confirm（926-1135）带 {accountId, planHash} → 按四键找 candidate → **重算存储 plan 的 hash 必须逐字等于请求 hash**（963-976，plan 落库后被改 → Conflict）→ find_one_and_update 原子夺取（pending_confirmation → running + 新 execution_token + confirmed_by；或夺取 5 分钟过期的 stale running 租约）→ 并发第二个确认拿 None 幂等回放。
4. **租约执行**：`execute_plan_tool_calls`（336-381）起 60s 心跳续租；逐 call：确权 → 写 prepared intent（intent_key 绑 run+hash+index，唯一索引幂等）→ 已有终态复用（accepted 绝不降级 succeeded）/ executing 遗留 → execution_unknown 停机 → prepared→executing CAS → `execute_management_tool` 分派。
5. **发送本身走生产网关**：send_contact_message 分支（2304-2326）→ `agent::send_contact_message_gateway`（资格/内容锁/评审/outbox/幂等全链路）→ 响应 gatewayStatus 经 `assert_tool_outcome` 断言：outbox_enqueued → **Accepted("已受理，等待异步送达回执")** 而非成功。
6. **收口**：外呼后再确权 → intent executing→accepted CAS → command running→succeeded/failed/execution_unknown CAS（锁 token）→ $unset token → assistant 落 `build_execution_summary`（📨 已受理——非"✅ 已送达"）。
7. **拒绝路径**：POST /commands/:id/reject → 锁 pending_confirmation 原子 canceled，"已取消该计划，未执行。"

防御要点汇总：LLM 幻觉工具名被 advertised 白名单拦（3244-3250）；原生 send 工具双重拦（目录摘除 + 执行兜底 3238-3243）；plan 冻结 hash 防确认与执行间被篡改；per-call durable intent 防重放/防双写；execution_unknown 诚实语义（跨过不可逆边界不重放）；write_deal_events/import_knowledge_text 追加 confirmed_admin 真人身份要求。

### 3.2 路由层触发 agent/LLM 的全部入口清单（本组文件）

| 入口 | 位置 | LLM 调用 |
| --- | --- | --- |
| POST /management-agent/sessions/:id/messages | management.rs:729-738 → build_management_plan:3494 | generate_agent_json("management.plan") |
| （management 工具间接）analyze_profile / generate_playbook / optimize_playbook / import_knowledge_* / provider_test 等 | management.rs:2765+ 直调兄弟 handler | 见各自行 |
| enable_agent / update_profile_note / analyze_contact_profile / handle_initial_profile_task / management enable_contact_agent | contacts.rs:2006,2242,2823,1001；management.rs:2132 | agent::build_initial_operation_profile（内部 LLM） |
| POST /contacts/:id/memory-consolidation/run | contacts.rs:3012 | agent::run_manual_memory_consolidation |
| POST /user-operations/guide/preview | guides.rs:568-578 | generate_agent_json("user.guide.preview") |
| POST /admin/domain-profiles/generate | guide_profile.rs:364-374 | generate_agent_json("guide.domain_profile.draft") |
| POST /operation-playbooks/generate | playbooks.rs:322-332 | generate_agent_json("playbook.generator") |
| POST /operation-playbooks/:id/optimize | playbooks.rs:422-432 | generate_agent_json("playbook.optimizer") |
| POST /user-operations/simulations/dialogue | simulations.rs:64 | agent::simulate_user_dialogue（影子全链路，多次 LLM） |
| POST /user-operations/evaluations/run | simulations.rs:101-106 | agent::simulate_user_dialogue × 场景数 |
| POST /user-operations/evaluations/formula-adherence | evaluations.rs:337 | agent::simulate_user_dialogue_with_budget × 场景数（带预算熔断） |
| PUT /prompt-templates/:id、POST /prompt-templates/:id/publish（非 force） | prompt_templates.rs:185-211,284-309 | review_prompt_edit（LLM 第三闸） |
| POST /agent-tasks/:id/review-now | tasks.rs:298 | execute_claimed_task → gateway 全链路（间接） |

会发出站消息（直接或经队列）的端点：management send_contact_message 工具（经 gateway/outbox）；POST /campaigns/:id/dispatch（扇出 follow_up task → worker → gateway）；management create_follow_up_task / review_task_now（task → gateway）。guide apply、contacts 全部写端点**不直接发消息**（只改配置/画像，影响后续决策）。

### 3.3 其它跨文件不变量

- **enrollment_token 代际协议**（contacts.rs + tasks.rs）：enable/disable/hide/cancel-task 都轮换 token；异步画像 commit filter 绑 token+claim generation；任何迟到 worker 对已轮换代无法写入（编不出复活）。
- **管理 agent 合成身份**（management.rs:3266-3272 ↔ shared.rs:1920-1922）：user_id="" 是 crate 内唯一合成点，resolve_authorized_workspace 据此跳过 ACL；隔离靠调用方强制传入可信 workspace_id。
- **AI 永不自动 verify 红线在本组的落点**：media 上传/换文件恒 draft（media_assets.rs:274,601）；referral card 创建恒 draft+disabled（referral_cards.rs:82-83）；AI playbook 恒 draft 非默认（playbooks.rs:353-354,475-476）；AI domain profile 恒候选草稿（guide_profile.rs:414-418）；suggestedValues 只进 taxonomy 候选层（guide_profile.rs:466-480）；management verify 类工具恒确认（management.rs:2008-2020）。
- **guide 与手动表单对越界值的双轨处置**：同一 `validate_dimension_value`，guide/AI 路径（LLM 产值）越界 → skipped/drop 继续，手动表单（AdminWrite）越界 → 400 硬拒（shared.rs:825-827 注释 + contacts.rs:2638-2652）；operation_state 同理（shared.rs:917-946 skip vs 表单侧无直写入口）。
- **prompt_pack_version 的 bump 点**：souls publish（souls.rs:133-135）、prompt publish（prompt_templates.rs:319-321）、reset pack（344-347）、ensure seed 写入时（souls.rs:148-151、prompt_templates.rs:83-87）——generate_agent_json 缓存原子失效。

---

## 4. 事实卡速查

### 4.1 端点分类速查

- **纯只读（零写库）**：GET contacts/counts/roster/:id/outcome-events/entitlements/memory-candidates/operation-health（用 read_operating_memory）、conversations messages、events、tasks、agent-runs、llm-usage、decision-reviews(+id)、agent-outcome-metrics、send-ledger 三端点、operation/active-view、campaigns list/sends、products list、playbooks list（注：会 ensure 默认 playbook = 可能写）、souls list（ensure seed 可能写）、operation-domains list/get（ensure seed 可能写）、prompt-templates list（ensure seed 可能写）、evaluation-scenarios list、accounts list、content-assets list、referral-cards list。
- **GET 但可能写库（seed/ensure 语义）**：GET /operation-playbooks（ensure_default_playbook）、GET /agent-souls、GET /operation-domains(+/:domain)、GET /prompt-templates、GET+PUT /contacts/:id/operating-memory 与 GET memory-card（ensure_operating_memory 播种）。
- **调 LLM 的端点**：见 3.2 表。
- **发消息/入发送队列**：management send 工具（gateway）、campaigns dispatch（task 扇出）、management create_follow_up_task、review-task-now（驱动执行）。
- **改全局/宽影响配置**：operation-domains PUT ×2 + reset + ask-human-policy；prompt-templates publish/reset-pack；souls publish；playbooks set-default/update；guide apply（scope=shared_playbook/workspace_user_operations 需强确认）；campaigns dispatch；management 各 rollout/rollback/activate/provider_activate 工具（转发 [外] handler）。

### 4.2 status/闭集清单（本组文件核证）

- `AgentCommandRun.status`（management.rs 用值）：pending_confirmation / running / succeeded / failed / dry_run / canceled / execution_unknown（写前均 `validate_agent_command_run_status` 断言，198-200）。
- `agent_tool_calls.status` 闭集（8 值，management.rs:4221-4237 测试锁死 ALLOWED_TOOL_CALL_STATUS）：prepared / executing / dry_run / succeeded / accepted / failed / executed_unverified / execution_unknown。**accepted ≠ succeeded**。
- `ToolRisk`：Readonly / Low / Dangerous / Irreversible + explicitly_classified（fail-closed 兜底 Dangerous+false）。
- campaign.status：draft / previewed / dispatching / completed（dispatch 可入态 = draft/previewed/dispatching；assert_campaign_status_valid 写前断言）。campaign_sends.status：prepared / enqueued。~~skipped_duplicate（classify 读到）~~【26 号交叉验证修正 2026-08-13：`skipped_duplicate` **不是** status 值域——campaigns.rs 全部写点仅 insert "prepared"(:706) 与 CAS "enqueued"(:789)；:958 的 skipped_duplicate 分支是 classify 的防御性输入分支，当前不可达】。
- sends 报表 7 桶：sent / pending / skipped / blocked(reason map) / canceled(reason map) / escalated(reason map) / unknown。
- guide preview.status：pending / applying / applied / failed / stale（apply_protocol_version=3）。
- 评测场景 status：active / draft（active 必须 groundTruth 完整数值 0-10）。
- media review_status：draft / approved；media_type：image / file / video；min_inject_tier：lean / relational / full（非法落 full）。
- referral card review_status：draft / approved；enabled bool。
- assist override mode：default / force_on / force_off。
- 成效事件 verification：staff_confirmed / payment_verified（conversation_inferred 拒直登）；event_kind：deal / reversal（reversal 必须带 product_id）。
- decision_review_phase（UI 投影闭集）：sent / auto_rewrite_sent / queued / auto_rewrite_queued / partially_sent / delivery_failed / delivery_canceled / delivery_unknown / gateway_blocked / approved / auto_rewrite_approved / final_blocked / auto_rewrite_failed / auto_rewrite_in_progress / review_recorded。
- AgentTask（本组写点用值）：pending / running / retry / failed / cancelled / sent / committing（+ gateway_status 自由文本如 profiled/stale_enrollment/enrollment_committed/admin_cancelled）。
- 管理执行常量：MAX_MANAGEMENT_TOOL_CALLS=12；租约 5min；心跳 60s。guide 租约 5min。campaign 受众上限 config.campaign_max_audience。

### 4.3 集合写入面（本组 handler 直接写的集合）

contacts（contacts.rs 全写端点、management 工具、shared guide apply/outcome append）；operating_memories（ensure/update/guide apply）；management_sessions/management_messages/command_runs/agent_tool_calls（management.rs）；campaigns/campaign_sends/tasks（campaigns.rs、management create_follow_up_task、tasks cancel）；user_operation_guide_previews（guides.rs）；domain_profiles（guide_profile 经 append_domain_profile_draft）；taxonomy_candidates（guide_profile upsert_candidate）；evaluation_scenarios（evaluations.rs）；wechat_accounts（accounts.rs）；products（products.rs）；operation_playbooks（playbooks.rs、guide apply）；agent_souls（souls.rs 经 soul_versions）；operation_domain_configs（domains.rs、guide apply）+ operation_state_policies（经 reconcile 重派）；prompt_templates（prompt_templates.rs 经版本模块）；content_assets（assets/media_assets）；referral_cards；events（各审计写点）；decision_reviews（reviews.rs post-decision 恢复字段）；llm_call_logs（间接经 generate_agent_json/mcp logged_call）。

---

## 5. 偏差与疑点

1. **conversations.rs:44-48 JSON 重复键**：`json!` 宏里 `msgType`/`mediaRef` 各出现两次（41-49），serde_json 后值覆盖前值，功能无碍但属明显复制粘贴瑕疵。
2. **reviews.rs list 的 N+1**：`list_decision_reviews`（106-160）循环内逐条 `fetch_run_status`（每条一次 agent_run_logs find_one），limit 最大 300 → 最多 300 次点查。与 contacts.rs 用聚合消 N+1（251-255）、campaigns.rs 用一次 $in（1121-1149）的做法不一致。性能取向问题，非正确性。
3. **management 观测查询工具的 account 边界**：`query_runs/query_metrics/query_send_ledger` 的 accountId 取自 LLM arguments（management.rs:2422-2479），未强制绑定当前命令账号（对比 update_assist_override 等走 `management_expected_account_bound_arguments` 强绑定）。validate_account/workspace filter 保证不跨 workspace，但**同 workspace 内可查其它账号**的观测数据。REST 面（如 GET /agent-runs）本就接受任意 accountId query，两面口径一致，推断是有意的"workspace 内账号不互相保密"设计——但与写类工具的强绑定形成不对称，记为设计非缺陷。
4. **list_contacts 与 count_contacts 的非真人过滤实现不同构**：list 读时 `is_operatable_person` 过滤（contacts.rs:245），count 用 DB 侧 `$nor`（346-348）。注释声称两者等价（343-345），等价性依赖 `webhooks::is_operatable_person` 与 `mcp::non_human_exclusion_filter` 两处逻辑始终同步——本组文件内无编译期/测试级钉死两者一致（count 测试 3297-3318 只验证 $nor 形状）。潜在漂移点。
5. **products.rs update 的 PUT 全量语义**：price/currency/sku/summary 传 None → 显式 $set Null 清空（310-330），与 media_assets meta 的"缺失=不动"部分更新语义相反。前端如果按部分更新理解会误清字段。文档（PUT 注释 12）未强调此点。
6. **referral_cards.rs 错误码风格**："card not found" 用 `BadRequest`（150,208,279）而非 `NotFound`，与其余文件（contact not found → 404）不一致。
7. **evaluations.rs:397 的 expect**：`truth.values.get(formula).expect("validated ground truth contains every formula")`——正确性依赖 305 行 `truth.is_valid()` 短路先行，两处逻辑距离 90 行，改动时易破坏（panic 面）。
8. **media 上传/文本资产的 accountId 不校验注册**：upload_media_asset（193-196）与 create_content_asset（137）接受任意 accountId 字符串落库，不走 `validate_account`（对比 campaigns/evaluations 等都校验）。错拼 accountId 会产生对任何账号都不可见的孤儿 account 私有资产。轻微。
9. **shared.rs guide 路径中 intentLevel 嵌套解析**：`prepare_contact_changes` 只有当 customerStage 存在时才在同分支解析 intentLevel（849-871），customerStage 缺席时走 else-if 单独解析 intentLevel（888-913）——两条路径行为一致但结构易读性差，曾是回归高发区（大量注释痕迹）。
10. **management.rs plan 终态与 requires_confirmation 分支的 update 不对称**：需确认分支 finalize 只 `$set updated_at`（832-842）且靠 filter 锁 pending_confirmation——若期间 plan 被并发 confirm 掉，matched=0 → Conflict("management_command_finalize_conflict")（871-875），此时 user 视角的 post_message 返回 500 段错误但 confirm 已在执行。窗口极窄（同一 plan 落库与响应之间），语义上"确认竞速"可见但无害（命令本身继续执行）。记为可接受竞态。
11. **guide preview 的 readableChanges 来源变化**：preview 响应的 readableChanges 现取自 frozen_plan.authoritative_changes 的 "target / label" 拼接（guides.rs:596-600），LLM 产的 readableChanges 文案被丢弃——与 build_guide_preview_prompt 要求 LLM 输出 readableChanges 的 prompt（shared.rs:1094-1098）不再对齐（LLM 字段成了摆设）。功能一致性疑点（前端显示的是机器拼接键名，不是业务话术）。
12. **campaigns 与 contacts 集合命名约定分裂**：campaigns/campaign_sends 用 camelCase 字段存库，contacts/tasks 用 snake_case（见 2.5 末注）——management_context 查询 campaigns 时也必须用 camelCase（management.rs:3443-3447）。跨集合写查询时极易拼错 key（Mongo 静默空结果），是全仓已知的高危坑位。
13. **events.rs kind 过滤只支持精确匹配**：`strategic_planner_*tick` 高频事件的挤出问题靠调用方传精确 kind 解决（22-25 注释），无前缀/正则匹配能力；审计场景跨多 kind 需多次请求。功能局限非缺陷。
14. **operation_view.rs 的 scope 传参**：`dimension_values_with_labels` 第三参传 `""`（97）而 guides.rs 传 `contact.account_id`（546,552）——operation_view 故意走 global 回落（注释 88-91），guides 允许 account 私有值。两处口径不同是有意的（全局视图 vs 单联系人视图），但读代码时易误判为 bug。
15. **contacts.rs `update_manual_tags` 不刷 `updated_at`**：$set 只写 manual_tags 三件套（2506-2512），不含顶层 `updated_at`——与其它写端点（恒带 updated_at）不一致。若下游依赖 updated_at 判断 contact 变更（如 guide apply 的 OCC 基线 contact_updated_at），manual-tags 修改不会使 guide preview 过期。倾向认为是缺陷（guide OCC 漏检口），但也可能是有意让标签修改不作废 preview——**未能从代码内证实意图，列为疑点**。

---

## 6. 覆盖自证

| 文件 | 总行数 | 读取方式 |
| --- | --- | --- |
| src/routes/mod.rs | 1249 | 全文（1-650, 651-1249） |
| src/routes/shared.rs | 2919 | 全文（1-750, 751-1500, 1501-2250, 2251-2919） |
| src/routes/management.rs | 4254 | 全文（1-750, 751-1500, 1501-2250, 2251-3000, 3001-3700, 3701-4254） |
| src/routes/contacts.rs | 3432 | 全文（1-720, 721-1440, 1441-2160, 2161-2880, 2881-3432） |
| src/routes/campaigns.rs | 1876 | 全文（1-700, 701-1400, 1401-1876） |
| src/routes/guides.rs | 1254 | 全文（1-649, 650-1254） |
| src/routes/guide_profile.rs | 944 | 全文（1-550, 551-944） |
| src/routes/evaluations.rs | 916 | 全文（1-500, 501-916） |
| src/routes/simulations.rs | 371 | 全文一次 |
| src/routes/accounts.rs | 352 | 全文一次 |
| src/routes/products.rs | 390 | 全文一次 |
| src/routes/playbooks.rs | 945 | 全文（1-500, 501-945） |
| src/routes/souls.rs | 153 | 全文一次 |
| src/routes/domains.rs | 730 | 全文（1-450, 451-730） |
| src/routes/prompt_templates.rs | 436 | 全文一次 |
| src/routes/events.rs | 74 | 全文一次 |
| src/routes/conversations.rs | 53 | 全文一次 |
| src/routes/tasks.rs | 404 | 全文一次 |
| src/routes/reviews.rs | 451 | 全文一次 |
| src/routes/media_assets.rs | 850 | 全文（1-500, 501-850） |
| src/routes/assets.rs | 183 | 全文一次 |
| src/routes/referral_cards.rs | 306 | 全文一次 |
| src/routes/send_ledger.rs | 216 | 全文一次 |
| src/routes/operation_view.rs | 110 | 全文一次 |
| src/routes/outcome_metrics.rs | 120 | 全文一次 |
| src/routes/management_prompt_edit.rs | 7 | 全文一次 |
| **合计** | **22,995** | 26/26 文件 100% 覆盖，无跳读 |

---

## 追记：27 号 API 三方对账回写（2026-08-13，主会话执行）

- 挂载计数修正：知识段挂载数 66→**70**、admin 段 62→**58**（序号偏移 4，27 号以 rg 精确计数为准：全文件 235 个 `.route(`、272 个方法+路径端点）。
- 27 号另产出：幽灵调用 0、方法不匹配 0、33 个孤儿端点逐个归类（其中 9 个全仓零消费：worker resume、post-decision 恢复三动作、产品/评测场景编辑等——功能缺口而非死代码）。
