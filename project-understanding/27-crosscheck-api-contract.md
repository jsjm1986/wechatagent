# 前后端 API 契约三方对账（核证日期 2026-08-13）

> 任务：前端调用 ↔ 后端路由 ↔ 源码三方对账，作为后续 API 改动的影响面地图。
> 方法：后端以 `src/routes/mod.rs` 当日工作树全文亲读为准（含未提交改动）；前端以 `frontend/src` 全域 ripgrep + 逐调用点亲读为准（排除 `__tests__`）；每个幽灵/孤儿/不匹配候选均两侧源码亲验后才下结论。
> 关键结论先行：**方法+路径级幽灵调用 0 个；孤儿端点 33 个（逐个归类，无一误判为死）；方法不匹配 0 个；但发现 2 个 query 参数名错配的真缺陷（`target_id`/`active_only`，13/14 号记录均漏检），其中 referrers 一处会导致前端功能 100% 运行时 400。**

---

## 1. 后端端点权威表（与 11/12 号记录的差异标注）

### 1.0 权威计数与挂载拓扑

- `api_router`（mod.rs:336-1067）共 **235 个 `.route(` 挂载**（`rg -c '\.route\(' src/routes/mod.rs` = 235，与逐行枚举一致），其中 30 个路径挂 2 个方法、2 个路径挂 3 个方法（`/operation-knowledge/chunks/:id`、`/admin/domain-profiles/:id`）、1 个路径挂 4 个方法（`/operation-knowledge/documents/:id`），合计 **272 个"方法+路径"端点**。
- 全部挂在 `/api` 前缀下（main.rs:357 `nest("/api", api_router)`）；`/api` 之外仅有 **`POST /webhooks/wechat`**（main.rs:358-361，MCP 回调，走 HMAC 不经 admin auth，不参与前端对账）与 SPA 静态回落（main.rs:354-363）。
- 鉴权：`require_session` layer 包住全部 api_router 路由（mod.rs:1062-1065）；白名单 `/health`、`/auth/login`、`/auth/token`（auth/middleware.rs:32-34 亲验）。
- 工具注意事项：本轮发现 Cursor Grep 工具对本文件恰在 100 条匹配处静默截断（count 模式报 100、offset 翻页失效），改用 shell `rg` 得到正确的 235。后续任何对 mod.rs 的计数核证都应以 shell `rg -c` 为准。

### 1.1 全量端点表（按 mod.rs 挂载顺序；行号 = 当日工作树 mod.rs 的 `.route(` 行）

| # | 方法 | 路径（/api 前缀省略） | handler | 行 |
|---|---|---|---|---|
| 1 | GET | /health | health | 338 |
| 2 | POST | /auth/login | auth::login | 339 |
| 3 | POST | /auth/logout | auth::logout | 340 |
| 4 | GET | /auth/me | auth::me | 341 |
| 5 | POST | /auth/workspace | auth::switch_workspace | 342 |
| 6 | POST | /auth/token | auth::issue_token | 343 |
| 7 | GET | /accounts | list_accounts | 344 |
| 8 | POST | /accounts/sync | sync_accounts | 345 |
| 9 | POST | /accounts/login/begin | login_begin | 346 |
| 10 | GET | /accounts/login/poll | login_poll | 347 |
| 11 | PUT | /accounts/:id/mcp-key | update_account_mcp_key | 348 |
| 12 | GET | /contacts | list_contacts | 349 |
| 13 | GET | /contacts/counts | count_contacts | 350 |
| 14 | POST | /contacts/search | search_contacts_endpoint | 351 |
| 15 | POST | /contacts/import | import_contacts_endpoint | 352 |
| 16 | POST | /contacts/search-import | search_import_contacts（DEPRECATED） | 353 |
| 17 | GET | /contacts/roster | roster_endpoint | 354 |
| 18 | POST | /contacts/batch-enable | batch_enable_endpoint | 355 |
| 19 | GET | /contacts/:id | get_contact | 356 |
| 20 | POST | /contacts/:id/enable-agent | enable_agent | 357 |
| 21 | POST | /contacts/:id/disable-agent | disable_agent | 358 |
| 22 | POST | /contacts/:id/hide-from-pool | hide_from_pool | 359 |
| 23 | POST | /contacts/:id/revoke-principal-exemption | revoke_principal_exemption | 360 |
| 24 | PUT | /contacts/:id/profile-note | update_profile_note | 364 |
| 25 | PUT | /contacts/:id/assist-override | update_assist_override | 365 |
| 26 | POST | /contacts/:id/clear-referral | clear_referral | 366 |
| 27 | PUT | /contacts/:id/custom-agent-instructions | update_custom_agent_instructions | 367 |
| 28 | PUT | /contacts/:id/manual-tags | update_manual_tags | 371 |
| 29 | PUT | /contacts/:id/operation-profile | update_operation_profile | 372 |
| 30 | POST | /contacts/:id/deal-events | add_deal_event | 376 |
| 31 | GET | /contacts/:id/outcome-events | list_outcome_events | 377 |
| 32 | GET | /contacts/:id/entitlements | list_entitlements | 378 |
| 33 | POST | /contacts/:id/analyze-profile | analyze_contact_profile | 379 |
| 34 | GET+PUT | /contacts/:id/operating-memory | get_operating_memory / update_operating_memory | 383 |
| 35 | GET | /contacts/:id/memory-card | get_contact_memory_card | 387 |
| 36 | GET | /contacts/:id/memory-candidates | list_contact_memory_candidates | 388 |
| 37 | POST | /contacts/:id/memory-consolidation/run | run_contact_memory_consolidation | 392 |
| 38 | GET | /contacts/:id/operation-health | get_operation_health | 396 |
| 39 | POST | /user-operations/guide/preview | preview_user_operation_guide | 397 |
| 40 | POST | /user-operations/guide/apply | apply_user_operation_guide | 401 |
| 41 | POST | /user-operations/simulations/dialogue | simulate_user_operation_dialogue | 405 |
| 42 | POST | /user-operations/evaluations/run | run_user_operation_evaluation | 409 |
| 43 | GET | /conversations/:contact_id/messages | list_messages | 413 |
| 44 | GET | /events | list_events | 414 |
| 45 | GET | /tasks | list_tasks | 415 |
| 46 | GET | /agent-runs | list_agent_runs | 416 |
| 47 | GET | /llm-usage | list_llm_usage | 417 |
| 48 | POST | /agent-tasks/:id/review-now | review_task_now | 418 |
| 49 | POST | /agent-tasks/:id/cancel | cancel_agent_task | 419 |
| 50 | GET+POST | /content-assets | list_content_assets / create_content_asset | 420 |
| 51 | POST | /content-assets/upload | media_assets::upload_media_asset（独立 body limit） | 424 |
| 52 | POST | /content-assets/:id/review | media_assets::review_media_asset | 433 |
| 53 | PUT+DELETE | /content-assets/:id | update_content_asset_meta / delete_content_asset | 437 |
| 54 | POST | /content-assets/:id/file | replace_content_asset_file（独立 body limit） | 442 |
| 55 | POST | /content-assets/:id/toggle | toggle_content_asset_sendable | 450 |
| 56 | POST+GET | /referral-cards | create_referral_card / list_referral_cards | 454 |
| 57 | POST | /referral-cards/:id/review | review_referral_card | 458 |
| 58 | POST | /referral-cards/:id/toggle | toggle_referral_card | 462 |
| 59 | DELETE | /referral-cards/:id | delete_referral_card | 466 |
| 60 | GET | /contacts/:wxid/send-history | send_ledger::contact_send_history | 470 |
| 61 | GET | /send-ledger/stats | send_ledger_stats | 474 |
| 62 | GET | /operation/active-view | operation_view::active_view | 475 |
| 63 | GET | /send-ledger/overview | send_ledger_overview | 476 |
| 64 | GET+POST | /operation-knowledge | list/create_operation_knowledge（legacy 顶层） | 480 |
| 65 | GET+POST | /operation-knowledge/documents | list/create documents | 484 |
| 66 | GET+PUT+PATCH+DELETE | /operation-knowledge/documents/:id | get/update/patch/delete document | 488 |
| 67 | GET | /operation-knowledge/documents/:id/chunks | list_operation_knowledge_document_chunks | 495 |
| 68 | GET+POST | /operation-knowledge/chunks | list/create chunk | 499 |
| 69 | GET | /operation-knowledge/review-queue | list_operation_knowledge_review_queue | 503 |
| 70 | GET+PUT+DELETE | /operation-knowledge/chunks/:id | get/update/delete chunk | 507 |
| 71 | GET | /operation-knowledge/chunks/:id/source | get_operation_knowledge_chunk_source | 513 |
| 72 | POST | /operation-knowledge/chunks/:id/verify | verify_operation_knowledge_chunk | 517 |
| 73 | POST | /operation-knowledge/chunks/:id/reject | reject_operation_knowledge_chunk | 521 |
| 74 | POST | /operation-knowledge/chunks/:id/repair | propose_chunk_repair | 525 |
| 75 | POST | /operation-knowledge/chunks/:id/repair/answer | answer_chunk_repair | 529 |
| 76 | POST | /operation-knowledge/chunks/:id/patch | patch_operation_knowledge_chunk | 534 |
| 77 | POST | /operation-knowledge/chunks/:id/archive | archive_operation_knowledge_chunk | 538 |
| 78 | POST | /operation-knowledge/chunks/:id/restore | restore_operation_knowledge_chunk | 542 |
| 79 | POST | /operation-knowledge/chunks/:id/rollback/:revision_id | rollback_operation_knowledge_chunk | 546 |
| 80 | GET | /operation-knowledge/chunks/:id/revisions | list_operation_knowledge_chunk_revisions | 550 |
| 81 | POST | /operation-knowledge/chunks/:id/split | split_operation_knowledge_chunk | 554 |
| 82 | POST | /operation-knowledge/chunks/:id/merge | merge_operation_knowledge_chunk | 558 |
| 83 | POST | /operation-knowledge/chunks/:id/relate | relate_operation_knowledge_chunk | 562 |
| 84 | DELETE | /operation-knowledge/chunks/:id/relate/:target_id | unrelate_operation_knowledge_chunk | 566 |
| 85 | POST+DELETE | /operation-knowledge/chunks/:id/lock | acquire/release_chunk_lock | 571 |
| 86 | GET(WS) | /ws/chunks | chunk_locks::chunk_event_websocket | 575 |
| 87 | GET | /operation-knowledge/chunks/referrers | list_chunk_referrers | 577 |
| 88 | POST | /operation-knowledge/chunks/batch-verify | batch_verify_chunks | 581 |
| 89 | POST | /operation-knowledge/chunks/batch-archive | batch_archive_chunks | 585 |
| 90 | GET | /operation-knowledge/catalog | get_operation_knowledge_catalog | 589 |
| 91 | GET | /operation-knowledge/catalog/persisted | get_operation_knowledge_catalog_persisted | 593 |
| 92 | GET+POST | /operation-knowledge/completeness | get/refresh completeness | 597 |
| 93 | GET | /operation-knowledge/integrity-report | get_operation_knowledge_integrity_report | 602 |
| 94 | POST | /operation-knowledge/tools/search | search_operation_knowledge_tool | 606 |
| 95 | POST | /operation-knowledge/auto-verify | auto_verify_operation_knowledge_chunks | 610 |
| 96 | GET | /knowledge/gap-signals | list_knowledge_gap_signals | 614 |
| 97 | POST | /knowledge/gap-signals/:id/dismiss | dismiss_knowledge_gap_signal | 615 |
| 98 | POST | /knowledge/gap-signals/:id/apply | apply_knowledge_gap_signal | 619 |
| 99 | POST | /knowledge/gap-signals/sweep | sweep_knowledge_gap_signals | 623 |
| 100 | POST | /knowledge/ask | ask_knowledge | 627 |
| 101 | GET(SSE) | /knowledge/ask/stream | ask_knowledge_stream | 628 |
| 102 | GET | /knowledge/metrics | knowledge_metrics | 629 |
| 103 | GET | /knowledge/operator-memory | list_operator_memory | 630 |
| 104 | POST | /knowledge/operator-memory/:id/revoke | revoke_operator_memory_http | 631 |
| 105 | POST | /operation-knowledge/tools/open-slice | open_operation_knowledge_slices | 635 |
| 106 | POST | /operation-knowledge/tools/open-evidence | open_operation_knowledge_slices（**同 handler 别名**） | 639 |
| 107 | POST | /operation-knowledge/import-preview | import_operation_knowledge_preview | 643 |
| 108 | GET | /operation-knowledge/import-preview-job/:id | get_import_preview_job | 647 |
| 109 | GET | /operation-knowledge/import-preview-jobs | list_import_preview_jobs | 651 |
| 110 | POST | /operation-knowledge/import-apply | import_operation_knowledge_apply | 655 |
| 111 | POST | /operation-knowledge/import-apply-pdf | import_operation_knowledge_apply_pdf | 659 |
| 112 | POST | /operation-knowledge/import-apply-image | import_operation_knowledge_apply_image | 663 |
| 113 | POST | /operation-knowledge/extract-tags | extract_operation_knowledge_tags | 667 |
| 114 | POST | /operation-knowledge/test-match | test_operation_knowledge_match | 671 |
| 115 | GET | /operation-knowledge/usage | list_knowledge_usage | 675 |
| 116 | GET | /operation-knowledge/logs/analyze | analyze_operation_knowledge_logs | 676 |
| 117 | POST | /operation-knowledge/repair/applied | record_repair_apply | 680 |
| 118 | POST | /operation-knowledge/chat | chat_turn | 684 |
| 119 | GET | /operation-knowledge/inbox | knowledge_inbox | 685 |
| 120 | GET | /operation-knowledge/metadata | knowledge_aggregate_metadata | 686 |
| 121 | GET | /operation-knowledge/chat/:session_id | chat_history | 690 |
| 122 | POST | /operation-knowledge/chat/:session_id/apply | chat_apply | 691 |
| 123 | POST | /operation-knowledge/chat/:session_id/discard | chat_discard | 695 |
| 124 | GET | /knowledge/digest/today | digest_today | 699 |
| 125 | POST | /knowledge/digest/regenerate | digest_regenerate | 700 |
| 126 | POST | /knowledge/digest/cards/:id/dismiss | digest_dismiss_card | 701 |
| 127 | GET+POST | /knowledge/chat/tasks | chat_task_list / chat_task_create | 705 |
| 128 | GET | /knowledge/chat/tasks/:id | chat_task_get | 709 |
| 129 | POST | /knowledge/chat/tasks/:id/cancel | chat_task_cancel | 710 |
| 130 | GET(SSE) | /knowledge/chat/sessions/:sid/stream | chat_session_stream | 711 |
| 131 | GET+POST | /knowledge/ingest-sources | list/create_ingest_source | 715 |
| 132 | PATCH+DELETE | /knowledge/ingest-sources/:id | update/delete_ingest_source | 719 |
| 133 | PUT+DELETE | /operation-knowledge/:id | update/delete_operation_knowledge（legacy） | 723 |
| 134 | GET | /decision-reviews | list_decision_reviews | 727 |
| 135 | GET | /decision-reviews/:id | get_decision_review | 728 |
| 136 | POST | /decision-reviews/:id/post-decision/retry | retry_post_decision | 729 |
| 137 | POST | /decision-reviews/:id/post-decision/regenerate | regenerate_post_decision | 733 |
| 138 | POST | /decision-reviews/:id/post-decision/discard | discard_post_decision | 737 |
| 139 | GET | /agent-outcome-metrics | list_agent_outcome_metrics | 741 |
| 140 | GET | /behavior-signal-metrics | list_behavior_signal_metrics | 742 |
| 141 | GET | /outcomes/autonomy | get_autonomy_outcomes | 746 |
| 142 | GET | /outcomes/autonomy/revisions | list_autonomy_revisions | 747 |
| 143 | GET+POST | /evaluation-scenarios | list/create_evaluation_scenario | 748 |
| 144 | PUT+DELETE | /evaluation-scenarios/:id | update/delete_evaluation_scenario | 752 |
| 145 | POST | /user-operations/evaluations/formula-adherence | run_formula_adherence_evaluation | 756 |
| 146 | GET+POST | /agent-souls | list/create_agent_soul | 760 |
| 147 | PUT | /agent-souls/:id | update_agent_soul | 764 |
| 148 | POST | /agent-souls/:id/publish | publish_agent_soul | 765 |
| 149 | GET | /operation-domains | list_operation_domains | 766 |
| 150 | GET+PUT | /operation-domains/:domain | get/update_operation_domain | 767 |
| 151 | GET+PUT | /operation-domains/:domain/state-machine | get/update state machine | 771 |
| 152 | POST | /operation-domains/:domain/reset | reset_operation_domain | 775 |
| 153 | PUT | /operation-domains/:domain/ask-human-policy | put_ask_human_policy | 779 |
| 154 | GET+POST | /prompt-templates | list/create_prompt_template | 783 |
| 155 | PUT | /prompt-templates/:id | update_prompt_template | 787 |
| 156 | POST | /prompt-templates/:id/publish | publish_prompt_template | 788 |
| 157 | POST | /prompt-templates/reset-system-pack | reset_system_prompt_pack | 792 |
| 158 | GET+POST | /operation-playbooks | list/create_operation_playbook | 796 |
| 159 | POST | /operation-playbooks/generate | generate_operation_playbook | 800 |
| 160 | POST | /operation-playbooks/:id/optimize | optimize_operation_playbook | 804 |
| 161 | PUT | /operation-playbooks/:id | update_operation_playbook | 808 |
| 162 | POST | /operation-playbooks/:id/set-default | set_default_operation_playbook | 809 |
| 163 | GET+POST | /products | list/create_product | 814 |
| 164 | PUT | /products/:product_id | update_product | 815 |
| 165 | POST | /products/:product_id/archive | archive_product | 816 |
| 166 | POST | /products/:product_id/restore | restore_product | 817 |
| 167 | POST+GET | /campaigns | create/list_campaigns | 818 |
| 168 | PATCH | /campaigns/:id | update_campaign_draft | 819 |
| 169 | POST | /campaigns/:id/preview | preview_campaign | 820 |
| 170 | POST | /campaigns/:id/dispatch | dispatch_campaign | 821 |
| 171 | GET | /campaigns/:id/sends | campaign_sends_report | 822 |
| 172 | POST | /management-agent/sessions | create_management_session | 823 |
| 173 | POST | /management-agent/sessions/:id/messages | post_management_message | 827 |
| 174 | GET | /management-agent/commands/:id | get_management_command | 831 |
| 175 | POST | /management-agent/commands/:id/confirm | confirm_management_command | 835 |
| 176 | POST | /management-agent/commands/:id/reject | reject_management_command | 839 |
| 177 | GET | /management-agent/tool-catalog | get_tool_catalog | 843 |
| 178 | GET | /admin/worker-controls | list_worker_controls | 844 |
| 179 | POST | /admin/worker-controls/:worker/resume | resume_worker_control | 845 |
| 180 | GET+POST | /admin/taxonomies | list/create_taxonomy | 850 |
| 181 | PATCH+DELETE | /admin/taxonomies/:id | patch/delete_taxonomy | 854 |
| 182 | GET | /admin/taxonomy-candidates | list_taxonomy_candidates | 858 |
| 183 | POST | /admin/taxonomy-candidates/:id/approve | approve_taxonomy_candidate | 859 |
| 184 | POST | /admin/taxonomy-candidates/:id/reject | reject_taxonomy_candidate | 863 |
| 185 | GET | /admin/relationship-type-suggestions | list_relationship_suggestions | 868 |
| 186 | POST | /admin/relationship-type-suggestions/:id/approve | approve_relationship_suggestion | 872 |
| 187 | POST | /admin/relationship-type-suggestions/:id/reject | reject_relationship_suggestion | 876 |
| 188 | GET | /admin/suspected-deals | list_suspected_deals | 884 |
| 189 | POST | /admin/suspected-deals/:id/approve | approve_suspected_deal | 885 |
| 190 | POST | /admin/suspected-deals/:id/reject | reject_suspected_deal | 889 |
| 191 | GET | /admin/operation-state-policies | list_operation_state_policies | 898 |
| 192 | GET | /admin/operation-state-policies/:id | get_operation_state_policy | 902 |
| 193 | POST | /admin/operation-domains/:id/publish | publish_operation_domain_version | 906 |
| 194 | POST | /admin/operation-domains/:id/rollout | rollout_operation_domain_version | 910 |
| 195 | POST | /admin/operation-domains/:id/rollback | rollback_operation_domain_version | 914 |
| 196 | GET | /admin/principal-escalations | list_principal_escalations | 920 |
| 197 | POST | /admin/principal-escalations/:short_code/resolve | resolve_principal_escalation | 924 |
| 198 | POST | /admin/principal-escalations/:short_code/reassign | reassign_principal_escalation | 928 |
| 199 | GET | /admin/ask-human/inbox | ask_human_inbox | 933 |
| 200 | GET | /admin/ask-human/summary | ask_human_summary | 934 |
| 201 | POST | /admin/operation-state-policies/:id/publish | publish_operation_state_policy_version | 935 |
| 202 | POST | /admin/operation-state-policies/:id/rollout | rollout_operation_state_policy_version | 939 |
| 203 | POST | /admin/operation-state-policies/:id/rollback | rollback_operation_state_policy_version | 943 |
| 204 | POST | /admin/taxonomies/:id/publish | publish_taxonomy_version | 947 |
| 205 | POST | /admin/taxonomies/:id/rollout | rollout_taxonomy_version | 951 |
| 206 | POST | /admin/taxonomies/:id/rollback | rollback_taxonomy_version | 955 |
| 207 | GET | /admin/outbox | list_outbox | 960 |
| 208 | POST | /admin/outbox/:id/cancel | cancel_outbox | 961 |
| 209 | GET | /admin/lessons-learned | list_lessons_learned | 963 |
| 210 | POST | /admin/lessons-learned/:lesson_id/promote-to-peer-case | promote_lesson_to_peer_case | 966 |
| 211 | GET | /admin/observability/phase-rollup | phase_rollup | 972 |
| 212 | GET | /admin/observability/performance | performance_summary | 973 |
| 213 | GET | /admin/observability/worker-health | worker_health | 976 |
| 214 | GET+POST | /admin/llm-providers | list/create_provider | 978 |
| 215 | PUT+DELETE | /admin/llm-providers/:id | update/delete_provider | 982 |
| 216 | POST | /admin/llm-providers/:id/activate | activate_provider | 986 |
| 217 | POST | /admin/llm-providers/:id/vision | set_vision_active | 987 |
| 218 | POST | /admin/llm-providers/test | test_provider | 988 |
| 219 | GET+POST | /admin/domain-schemas | list/create_domain_schema | 990 |
| 220 | PUT+DELETE | /admin/domain-schemas/:id | update/delete_domain_schema | 994 |
| 221 | POST | /admin/domain-schemas/:id/activate | activate_domain_schema | 998 |
| 222 | GET+POST | /admin/domain-profiles | list/create_domain_profile | 1003 |
| 223 | GET | /admin/domain-profiles/active | active_domain_profile | 1007 |
| 224 | GET+PUT+DELETE | /admin/domain-profiles/:id | get/update/delete_domain_profile | 1008 |
| 225 | POST | /admin/domain-profiles/:id/publish | publish_domain_profile | 1014 |
| 226 | POST | /admin/domain-profiles/:id/rollout | rollout_domain_profile | 1018 |
| 227 | POST | /admin/domain-profiles/:id/rollback | rollback_domain_profile | 1022 |
| 228 | POST | /admin/domain-profiles/:id/activate | activate_domain_profile | 1026 |
| 229 | POST | /admin/domain-profiles/generate | generate_domain_profile_candidate | 1031 |
| 230 | GET | /evolution/experiments | list_evolution_experiments | 1036 |
| 231 | GET | /evolution/proposals/:id | get_evolution_proposal_detail | 1037 |
| 232 | POST | /evolution/proposals/:id/release | release_evolution_proposal | 1041 |
| 233 | POST | /evolution/proposals/:id/rollback | rollback_evolution_proposal | 1045 |
| 234 | GET | /evolution/threshold-overrides/audit | list_threshold_override_audit | 1050 |
| 235 | GET+PUT | /evolution/runtime-flag | get/put_evolution_runtime_flag | 1056 |

### 1.2 与 11/12 号记录的差异

| # | 记录 | 差异 | 裁决 |
|---|---|---|---|
| D-1 | 11 号 §1.2 表格第 64-129 行 | 声称知识段（mod.rs:480-726）"共 66 条挂载"——**实际 70 条**（本表 #64-#133 逐条枚举），少计 4 | 11 号计数错误，需回写 |
| D-2 | 11 号 §1.2 表格第 174-235 行 | 声称 admin/evolution 段"共 60+ 条挂载"（按其编号推算 62 条）——**实际 58 条**（本表 #178-#235） | 11 号计数错误，需回写；两处误差恰好抵消使总数 235 巧合一致 |
| D-3 | 11 号 §1.2 自 130 号起 | 序号系统性偏移 4（其"130 GET /decision-reviews"实为第 134 个挂载），但**路径/方法/handler/行号内容逐条核对全部正确、无遗漏** | 仅编号问题，内容可信 |
| D-4 | 12 号 §4 端点表 | 逐条与源码一致（含挂载行号），无错误 | 无需修正 |
| D-5 | 11/12 号共同 | 均未把 `/operation-knowledge/tools/open-evidence` 标注为 open-slice 的**同 handler 别名**（mod.rs:635-642 两条路由绑同一 `open_operation_knowledge_slices`）；11 号 101 行列举里两者并列易被误读为两个实现 | 信息补充 |

---

## 2. 前端调用权威表（与 13/14 号记录的差异标注）

### 2.0 权威计数与搜索口径

- 运行时调用点：**257 处**（`rg "api\.(get|post|put|patch|delete|postForm|postRaw)\s*[<(]|(^|[^a-zA-Z.])fetch\(|new EventSource\(|new WebSocket\(" frontend/src -g '*.ts' -g '*.tsx' -g '!**/__tests__/**'`）。
- 归并为 **239 个唯一"方法+路径"模式**（动态段归一为 `:x`，query 剥离）。**239 + 33 孤儿 = 272 = 后端全集，精确自洽。**
- 调用通道 5 种：`lib/api.ts` 封装（get/post/put/patch/delete/postForm/postRaw，全为对应 HTTP 方法，postForm/postRaw=POST）、裸 `fetch(`（knowledge 域为主，方法看 init.method，缺省 GET）、`EventSource`（3 处：explore.tsx:125 ask/stream、today.tsx 经 useSseReconnect.ts:42 sessions stream、api.ts:130 openEventSource——**后者零消费方，13 号 §5-2 dead code 结论复证维持**）、`WebSocket`（App.tsx:82，URL=`{ws|wss}://host/api/ws/chunks`，App.tsx:79-80 亲验）、`originalFetch`（main.tsx 4 条 auth）。
- 搜索陷阱记录：ripgrep 排除 glob 必须写 `-g '!**/__tests__/**'`；首轮误用 `-g '!__tests__/**'`（相对路径不匹配）导致测试 mock URL（含 `/api/admin/relationship-suggestions/…` 等误导性字符串）混入清单，已全部剔除复核。

### 2.1 全量调用表（方法+路径 ← 代表调用点；同一路径多处调用只列代表）

**auth / 账号（9）**
- POST /api/auth/login ← main.tsx:45（originalFetch）
- GET /api/auth/me ← main.tsx:56,125
- POST /api/auth/logout ← main.tsx:154
- POST /api/auth/workspace ← main.tsx:180
- GET /api/accounts ← App.tsx:149；Shell.tsx:16；account-management/index.tsx:21
- POST /api/accounts/sync ← Shell.tsx:15；account-management/index.tsx:34；AccountLogin.tsx:87
- POST /api/accounts/login/begin ← AccountLogin.tsx:52
- GET /api/accounts/login/poll?loginSessionId&accountAlias ← AccountLogin.tsx:80
- PUT /api/accounts/:id/mcp-key ← McpKeyForm.tsx:63

**contacts（24）**
- GET /api/contacts ← contactStore.ts:86（limit=500）；products-deals/index.tsx:380（ContactPicker limit=100）
- GET /api/contacts/counts ← userOpsStore.ts:574
- GET /api/contacts/roster ← userOpsStore.ts:604
- POST /api/contacts/batch-enable ← userOpsStore.ts:635
- POST /api/contacts/import ← ask-human-config/DeciderChainEditor.tsx:126
- POST /api/contacts/:id/hide-from-pool ← userOpsStore.ts:644
- POST /api/contacts/:id/enable-agent ← userOpsStore.ts:674
- POST /api/contacts/:id/disable-agent ← userOpsStore.ts:697
- PUT /api/contacts/:id/profile-note ← userOpsStore.ts:717
- PUT /api/contacts/:id/custom-agent-instructions ← userOpsStore.ts:740
- PUT /api/contacts/:id/assist-override ← userOpsStore.ts:763
- PUT /api/contacts/:id/operation-profile ← userOpsStore.ts:786
- GET /api/contacts/:id/operating-memory ← userOpsStore.ts:478
- PUT /api/contacts/:id/operating-memory ← userOpsStore.ts:814
- POST /api/contacts/:id/clear-referral ← userOpsStore.ts:832
- PUT /api/contacts/:id/manual-tags ← userOpsStore.ts:851
- POST /api/contacts/:id/analyze-profile ← userOpsStore.ts:872
- GET /api/contacts/:id/memory-candidates ← userOpsStore.ts:479,1016
- POST /api/contacts/:id/memory-consolidation/run ← userOpsStore.ts:1013
- GET /api/contacts/:id/operation-health ← userOpsStore.ts:481
- GET /api/contacts/:id/outcome-events ← products-deals/index.tsx:492
- GET /api/contacts/:id/entitlements ← products-deals/index.tsx:828
- POST /api/contacts/:id/deal-events ← products-deals/index.tsx:605
- GET /api/contacts/:wxid/send-history ← legacy.tsx:2009（SendHistorySection；调用方 CockpitPanel.tsx:181 传 `selected.wxid` ✓）

**conversations / user-operations（5）**
- GET /api/conversations/:contactId/messages ← userOpsStore.ts:477（传 contact.id，后端 :contact_id 即 ObjectId ✓）
- POST /api/user-operations/guide/preview ← userOpsStore.ts:896
- POST /api/user-operations/guide/apply ← userOpsStore.ts:958
- POST /api/user-operations/simulations/dialogue ← userOpsStore.ts:1041
- POST /api/user-operations/evaluations/formula-adherence ← quality/index.tsx:267

**events / tasks / runs / reviews / llm（7）**
- GET /api/events ← operationsStore.ts:104
- GET /api/tasks ← operationsStore.ts:105；commandStore.ts:57
- GET /api/decision-reviews ← operationsStore.ts:106；userOpsStore.ts:480
- GET /api/llm-usage ← operationsStore.ts:107
- GET /api/agent-runs ← operationsStore.ts:108,153
- POST /api/agent-tasks/:id/review-now ← operationsStore.ts:66（`${action}`，action ∈ {review-now,cancel} 亲验）
- POST /api/agent-tasks/:id/cancel ← operationsStore.ts:66

**content-assets / referral-cards（13）**
- GET /api/content-assets ← contentStore.ts:125；commandStore.ts:53
- POST /api/content-assets ← contentStore.ts:163
- POST /api/content-assets/upload ← contentStore.ts:193（postForm）
- POST /api/content-assets/:id/review ← contentStore.ts:210
- PUT /api/content-assets/:id ← contentStore.ts:228
- POST /api/content-assets/:id/file ← contentStore.ts:252（postForm）
- POST /api/content-assets/:id/toggle ← contentStore.ts:269
- DELETE /api/content-assets/:id?expectedScope… ← contentStore.ts:287
- GET /api/referral-cards ← referralCardStore.ts:36
- POST /api/referral-cards ← referralCardStore.ts:50
- POST /api/referral-cards/:id/review ← referralCardStore.ts:80
- POST /api/referral-cards/:id/toggle ← referralCardStore.ts:93
- DELETE /api/referral-cards/:id ← referralCardStore.ts:106

**send-ledger / active-view（3）**
- GET /api/send-ledger/overview ← sendAnalyticsStore.ts:37
- GET /api/send-ledger/stats ← sendAnalyticsStore.ts:53
- GET /api/operation/active-view ← profileStore.ts:65

**campaigns / products（9）**
- GET /api/campaigns ← campaignStore.ts:110
- POST /api/campaigns ← CampaignCreate.tsx:49
- PATCH /api/campaigns/:id ← CampaignCreate.tsx:60
- POST /api/campaigns/:id/preview ← CampaignCreate.tsx:71
- GET /api/campaigns/:id/sends ← campaignStore.ts:85
- GET /api/products ← products-deals/index.tsx:185,542；ProductMultiSelect.tsx:15（**?active_only= 参数名错配，见 §3.3-P2**）
- POST /api/products ← products-deals/index.tsx:203
- POST /api/products/:productId/archive ← products-deals/index.tsx:224（`${action}` ∈ {archive,restore} 亲验）
- POST /api/products/:productId/restore ← 同上

**management-agent（4）**
- POST /api/management-agent/sessions ← commandStore.ts:81
- POST /api/management-agent/sessions/:id/messages ← commandStore.ts:88
- POST /api/management-agent/commands/:id/confirm ← commandStore.ts:135
- POST /api/management-agent/commands/:id/reject ← commandStore.ts:181

**operation-knowledge：documents / chunks（28）**
- GET /api/operation-knowledge/documents ← steward.tsx:109
- POST /api/operation-knowledge/documents ← steward.tsx:127
- GET /api/operation-knowledge/documents/:id ← steward.tsx:172
- PATCH /api/operation-knowledge/documents/:id ← steward.tsx:226
- DELETE /api/operation-knowledge/documents/:id ← steward.tsx:158
- GET /api/operation-knowledge/documents/:id/chunks ← steward.tsx:274,289
- GET /api/operation-knowledge/chunks ← shared.tsx:48,109；explore.tsx:404
- POST /api/operation-knowledge/chunks ← steward.tsx:246（写死 draft+needs_review）
- GET /api/operation-knowledge/chunks/:id ← ChunkReviewCard.tsx:65（deep-link）
- POST /api/operation-knowledge/chunks/:id/verify ← shared.tsx:858；useGoLive.ts:39；ChunkReviewCard.tsx:88（`${verb}` ∈ {verify,reject} 亲验）
- POST /api/operation-knowledge/chunks/:id/reject ← shared.tsx:756；ReviewChat.tsx:138；ChunkReviewCard.tsx:88
- POST /api/operation-knowledge/chunks/:id/patch ← shared.tsx:743
- POST /api/operation-knowledge/chunks/:id/archive ← shared.tsx:767
- POST /api/operation-knowledge/chunks/:id/restore ← shared.tsx:890
- POST /api/operation-knowledge/chunks/:id/split ← shared.tsx:786
- POST /api/operation-knowledge/chunks/:id/merge ← shared.tsx:803
- POST /api/operation-knowledge/chunks/:id/relate ← shared.tsx:834
- DELETE /api/operation-knowledge/chunks/:id/relate/:targetId ← shared.tsx:156
- POST /api/operation-knowledge/chunks/:id/lock ← shared.tsx:593
- DELETE /api/operation-knowledge/chunks/:id/lock ← shared.tsx:679
- GET /api/operation-knowledge/chunks/:id/source ← shared.tsx:439
- GET /api/operation-knowledge/chunks/:id/revisions ← shared.tsx:1021；steward.tsx:3269
- POST /api/operation-knowledge/chunks/:id/rollback/:revisionId ← shared.tsx:1054
- POST /api/operation-knowledge/chunks/:id/repair ← ChunkRepairPanel.tsx:27
- POST /api/operation-knowledge/chunks/:id/repair/answer ← ChunkRepairPanel.tsx:61
- GET /api/operation-knowledge/chunks/referrers ← shared.tsx:933（**?target_id= 参数名错配，见 §3.3-P1**）
- POST /api/operation-knowledge/chunks/batch-verify ← steward.tsx:1685（path 变量亲验）
- POST /api/operation-knowledge/chunks/batch-archive ← steward.tsx:1686

**operation-knowledge：目录/审计/导入/对话（24）**
- GET /api/operation-knowledge/review-queue ← steward.tsx:1604
- GET /api/operation-knowledge/catalog ← steward.tsx:2415
- GET /api/operation-knowledge/catalog/persisted ← steward.tsx:2414
- GET /api/operation-knowledge/completeness ← CockpitView.tsx:24；steward.tsx:2416；（walkthrough.py 亦读）
- GET /api/operation-knowledge/integrity-report ← CockpitView.tsx:28；steward.tsx:2417
- POST /api/operation-knowledge/tools/search ← steward.tsx:1153
- POST /api/operation-knowledge/tools/open-slice ← steward.tsx:1170
- POST /api/operation-knowledge/auto-verify ← AutoVerifyPanel.tsx（cockpit）；quality/index.tsx:175
- POST /api/operation-knowledge/import-preview ← steward.tsx:779
- GET /api/operation-knowledge/import-preview-job/:id ← steward.tsx:748
- GET /api/operation-knowledge/import-preview-jobs ← steward.tsx:665
- POST /api/operation-knowledge/import-apply ← steward.tsx:817
- POST /api/operation-knowledge/import-apply-pdf ← steward.tsx:928
- POST /api/operation-knowledge/import-apply-image ← steward.tsx:977
- POST /api/operation-knowledge/extract-tags ← steward.tsx:694
- POST /api/operation-knowledge/test-match ← steward.tsx:3194
- GET /api/operation-knowledge/logs/analyze ← steward.tsx:2418
- POST /api/operation-knowledge/repair/applied ← lib/applyAiRepairPatch.ts:27（thenVerify 恒 false）
- POST /api/operation-knowledge/chat ← today.tsx:233；ReviewChat.tsx:166
- GET /api/operation-knowledge/chat/:sessionId ← today.tsx:153
- POST /api/operation-knowledge/chat/:sessionId/apply ← today.tsx:273；useGoLive.ts:25
- POST /api/operation-knowledge/chat/:sessionId/discard ← today.tsx:313
- GET /api/operation-knowledge/inbox ← today.tsx:546
- GET /api/operation-knowledge/metadata ← atlas.tsx（MetadataDashboard）

**knowledge 族（21）**
- GET /api/knowledge/gap-signals ← steward.tsx:1355；CockpitView.tsx:32
- POST /api/knowledge/gap-signals/:id/dismiss ← steward.tsx:1397（`${action}` ∈ {dismiss,apply} 亲验）；ask-human SIMPLE_ENDPOINTS（index.tsx:162）
- POST /api/knowledge/gap-signals/:id/apply ← steward.tsx:1397
- POST /api/knowledge/gap-signals/sweep ← steward.tsx:1375,2455
- POST /api/knowledge/ask ← explore.tsx:99
- GET(SSE) /api/knowledge/ask/stream ← explore.tsx:125（裸 EventSource，一次性 RPC 流不重连）
- GET /api/knowledge/metrics ← steward.tsx:2419；atlas.tsx（MetricsTab）
- GET /api/knowledge/operator-memory ← atlas.tsx:1456
- POST /api/knowledge/operator-memory/:id/revoke ← atlas.tsx:1487
- GET /api/knowledge/digest/today ← today.tsx:850
- POST /api/knowledge/digest/regenerate ← today.tsx:867
- POST /api/knowledge/digest/cards/:cardId/dismiss ← today.tsx:890
- GET /api/knowledge/chat/tasks ← today.tsx:1116
- POST /api/knowledge/chat/tasks ← today.tsx:342,821
- GET /api/knowledge/chat/tasks/:taskId ← today.tsx:1173
- POST /api/knowledge/chat/tasks/:taskId/cancel ← today.tsx:1306
- GET(SSE) /api/knowledge/chat/sessions/:sid/stream ← today.tsx（经 useSseReconnect.ts:42）
- GET /api/knowledge/ingest-sources ← steward.tsx:2160
- POST /api/knowledge/ingest-sources ← steward.tsx:2178
- PATCH /api/knowledge/ingest-sources/:id ← steward.tsx:2200
- DELETE /api/knowledge/ingest-sources/:id ← steward.tsx:2221

**指标/评测（7）**
- GET /api/agent-outcome-metrics ← quality/index.tsx:90
- GET /api/behavior-signal-metrics ← steward.tsx:2422（limit=14）
- GET /api/outcomes/autonomy ← autonomy/index.tsx:106
- GET /api/outcomes/autonomy/revisions ← autonomy/index.tsx:107
- GET /api/evaluation-scenarios ← EvaluationScenariosPanel.tsx:48
- POST /api/evaluation-scenarios ← EvaluationScenariosPanel.tsx:107
- DELETE /api/evaluation-scenarios/:id ← EvaluationScenariosPanel.tsx:133

**souls / prompts / playbooks / domains（20）**
- GET /api/agent-souls ← strategyStore.ts:127；commandStore.ts:55
- POST /api/agent-souls ← strategyStore.ts:147
- PUT /api/agent-souls/:id ← strategyStore.ts:167
- POST /api/agent-souls/:id/publish ← strategyStore.ts:191
- GET /api/prompt-templates ← strategyStore.ts:128；quality/index.tsx:398
- POST /api/prompt-templates ← strategyStore.ts:208
- PUT /api/prompt-templates/:id ← strategyStore.ts:228；quality/index.tsx:451
- POST /api/prompt-templates/:id/publish ← strategyStore.ts:265；quality/index.tsx:482
- POST /api/prompt-templates/reset-system-pack ← strategyStore.ts:292
- GET /api/operation-playbooks ← userOpsStore.ts:537
- POST /api/operation-playbooks ← userOpsStore.ts:1073
- PUT /api/operation-playbooks/:id ← userOpsStore.ts:1131
- POST /api/operation-playbooks/:id/optimize ← userOpsStore.ts:1174
- POST /api/operation-playbooks/generate ← userOpsStore.ts:1232
- POST /api/operation-playbooks/:id/set-default ← userOpsStore.ts:1281
- GET /api/operation-domains ← userOpsStore.ts:652；atlas.tsx（domains 面板）
- GET /api/operation-domains/:domain ← ask-human-config/index.tsx:35（domain=user_operations）
- PUT /api/operation-domains/:domain ← userOpsStore.ts:1345
- POST /api/operation-domains/:domain/reset ← userOpsStore.ts:1361
- PUT /api/operation-domains/:domain/ask-human-policy ← ask-human-config/index.tsx:82

**admin 族（57）**
- GET /api/admin/ask-human/inbox ← lib/inboxApi.ts:77
- GET /api/admin/ask-human/summary ← lib/inboxApi.ts:84
- GET /api/admin/principal-escalations ← ResolvedEscalations.tsx（status=resolved）
- POST /api/admin/principal-escalations/:code/resolve ← EscalationInline.tsx:36
- POST /api/admin/principal-escalations/:code/reassign ← EscalationInline.tsx:50
- POST /api/admin/relationship-type-suggestions/:id/approve ← ask-human/index.tsx:156（SIMPLE_ENDPOINTS）
- POST /api/admin/relationship-type-suggestions/:id/reject ← ask-human/index.tsx:158
- GET /api/admin/suspected-deals ← products-deals/index.tsx:911（status=pending）
- POST /api/admin/suspected-deals/:id/approve ← products-deals/index.tsx:939；SuspectedDealReviewCard.tsx:54
- POST /api/admin/suspected-deals/:id/reject ← products-deals/index.tsx:958；SuspectedDealReviewCard.tsx:72
- GET /api/admin/taxonomies ← system-strategy（TaxonomiesAdmin）；atlas.tsx；StageSelect.tsx:15
- POST /api/admin/taxonomies ← system-strategy index.tsx:733（postRaw 处理 409）
- PATCH /api/admin/taxonomies/:id ← system-strategy index.tsx:763
- DELETE /api/admin/taxonomies/:id ← system-strategy（废弃）
- POST /api/admin/taxonomies/:id/publish|rollout|rollback ← atlas.tsx:1028（PublishBar `${resourceKind}/${id}/${action}`，resourceKind ∈ {taxonomies, operation-state-policies, operation-domains} × action ∈ {publish,rollout,rollback} 9 组合亲验）；system-strategy ActiveVersionsBar（index.tsx:944 endpointPrefix=/api/admin/taxonomies）
- GET /api/admin/taxonomy-candidates ← system-strategy（TaxonomyCandidatesAdmin）
- POST /api/admin/taxonomy-candidates/:id/approve ← TaxonomyCandidateReviewCard.tsx（postRaw）
- POST /api/admin/taxonomy-candidates/:id/reject ← TaxonomyCandidateReviewCard + 批量驳回循环
- GET /api/admin/operation-state-policies ← system-strategy（StatePolicyAdmin）；atlas.tsx（policies）
- POST /api/admin/operation-state-policies/:id/publish|rollout|rollback ← atlas PublishBar；system-strategy ActiveVersionsBar（index.tsx:646）
- POST /api/admin/operation-domains/:id/publish|rollout|rollback ← legacy.tsx:777（ActiveVersionsBar endpointPrefix=/api/admin/operation-domains，legacy.tsx:908 亲验）；atlas PublishBar
- GET /api/admin/outbox ← OutboxPanel.tsx:127
- POST /api/admin/outbox/:id/cancel ← OutboxPanel.tsx:203
- GET /api/admin/lessons-learned ← system-strategy（LessonsLearnedAdmin）；LessonPromoteCard.tsx
- POST /api/admin/lessons-learned/:lessonId/promote-to-peer-case ← LessonPromoteCard.tsx
- GET /api/admin/observability/phase-rollup ← steward.tsx:2420
- GET /api/admin/observability/performance ← steward.tsx:2423（hours=24）
- GET /api/admin/observability/worker-health ← steward.tsx:2421
- GET /api/admin/llm-providers ← llm-providers/index.tsx:157
- POST /api/admin/llm-providers ← llm-providers/index.tsx:246
- PUT /api/admin/llm-providers/:providerId ← llm-providers/index.tsx:260
- DELETE /api/admin/llm-providers/:providerId ← llm-providers/index.tsx:286
- POST /api/admin/llm-providers/:providerId/activate ← llm-providers/index.tsx:302
- POST /api/admin/llm-providers/:providerId/vision ← llm-providers/index.tsx:326
- POST /api/admin/llm-providers/test ← llm-providers/index.tsx:352
- GET /api/admin/domain-schemas ← atlas.tsx（DomainSchemaTab）
- POST /api/admin/domain-schemas ← atlas.tsx
- PUT /api/admin/domain-schemas/:schemaId ← atlas.tsx
- DELETE /api/admin/domain-schemas/:schemaId ← atlas.tsx
- POST /api/admin/domain-schemas/:id/activate ← atlas.tsx
- GET /api/admin/domain-profiles ← strategyStore.ts:348
- POST /api/admin/domain-profiles ← strategyStore.ts:439
- GET /api/admin/domain-profiles/active ← profileStore.ts:50；EvaluationScenariosPanel.tsx:49
- GET /api/admin/domain-profiles/:id ← ProfilePublishCard.tsx:43
- PUT /api/admin/domain-profiles/:id ← strategyStore.ts:431
- DELETE /api/admin/domain-profiles/:id ← strategyStore.ts:497
- POST /api/admin/domain-profiles/:id/publish ← strategyStore.ts:464；ProfilePublishCard.tsx:65
- POST /api/admin/domain-profiles/:id/rollout ← system-strategy ActiveVersionsBar（index.tsx:2333，endpointPrefix=/api/admin/domain-profiles）
- POST /api/admin/domain-profiles/:id/rollback ← 同上
- POST /api/admin/domain-profiles/:id/activate ← strategyStore.ts:480；ProfilePublishCard.tsx:88
- POST /api/admin/domain-profiles/generate ← strategyStore.ts:364
（上表中三处"publish|rollout|rollback"各展开 3 条，admin 族合计 57 条方法+路径）

**evolution / WS（8）**
- GET /api/evolution/runtime-flag ← EvolutionCenterTab.tsx:155（apiGet=GET 亲验 :73）
- PUT /api/evolution/runtime-flag ← EvolutionCenterTab.tsx:175,199（apiPut=PUT 亲验 :79）
- GET /api/evolution/experiments ← EvolutionCenterTab.tsx:247
- GET /api/evolution/proposals/:id ← ProposalReleaseCard.tsx
- POST /api/evolution/proposals/:id/release ← ProposalReleaseCard.tsx（`${action}` ∈ {release,rollback}）
- POST /api/evolution/proposals/:id/rollback ← 同上
- GET /api/evolution/threshold-overrides/audit ← EvolutionCenterTab.tsx:227
- GET(WS 升级) /api/ws/chunks ← App.tsx:80-82

### 2.2 与 13/14 号记录的差异

| # | 记录 | 差异 | 裁决 |
|---|---|---|---|
| F-1 | 13 号 §4.1 | store→端点映射逐条复核**一致，无错误**；`openEventSource` dead code（§5-2）复证维持（全库仅 api.ts:119 定义处） | 无需修正 |
| F-2 | 14 号 §4.1 user-ops 行 | 把 `POST /api/contacts/import` 列在 user-ops 行并注"（经 ask-human-config 复用）"——实际唯一调用点在 `ask-human-config/DeciderChainEditor.tsx:126`（该行也已列），user-ops feature 自身零调用；复用方向标反 | 轻微，建议回写澄清 |
| F-3 | 14 号 §2.11 / §4.1 | 忠实照录了前端 `referrers?target_id=` 与 `products?active_only=true` 的行为，但**未对照后端 Query serde 定义，漏检两处参数名错配真缺陷**（见 §3.3） | 需回写补录 |
| F-4 | 13/14 号共同 | 均未标注 `/api/knowledge/chat/sessions/:sid/stream` 与 `/api/operation-knowledge/chat/:session_id` 的路径参数命名不一致（:sid vs :session_id，同一 session id） | 信息补充 |
| F-5 | 14 号 §4.1 knowledge 行 | 端点列举与本表核对**无遗漏**（含 chat/:sid、rollback/:revisionId、referrers 等易漏项） | 无需修正 |

---

## 3. 对账结果

### 3.1 幽灵调用清单（前端调用但后端不存在）

**方法+路径级：0 个。** 前端 239 个方法+路径全部命中后端挂载表（239 + 33 孤儿 = 272 精确自洽）。历史上首轮 grep 中出现的 `/api/admin/relationship-suggestions/…`（少 `-type`）、`/api/contacts/C1/decision-reviews` 等疑似幽灵，经定位**全部来自 `__tests__` 的 mock 字符串**，生产代码不存在。

**query 参数级幽灵：2 个（真缺陷，见 §3.3）。**

### 3.2 孤儿端点清单（后端存在、前端零调用；33 个，逐个归类）

**A. 脚本消费（smoke / biz-test / e2e，7 个）——非死，改动需同步脚本**

| 端点 | 消费方（亲验） |
|---|---|
| GET /health | scripts/smoke_knowledge_full_loop.py:61、smoke_reimport_docs.py:106（`_request` base 含 /api）；docs/real-task-runbook.md 检查点 A。biz-test step0 探活打的是 `/`（SPA 根，step0_preflight.py:84-88），不打 /health |
| POST /user-operations/evaluations/run | scripts/biz-test/batch_c_evaluation.py:49 |
| POST /campaigns/:id/dispatch | scripts/biz-test/batch_c_campaign.py:68-134（5 处）。生产真实派发走 AI 总控 `wechatagent.dispatch_campaign` 工具 → management.rs **in-process 直调 handler**（3217-3236），不经 HTTP；前端活动频道明示"确认推送在 AI 总控完成" |
| PUT /operation-knowledge/chunks/:id | scripts/smoke_repair_rejected.py:131 |
| GET /operation-knowledge（legacy 顶层列表） | scripts/smoke_reimport_split.py:189,231、smoke_reimport_docs.py:125、smoke_knowledge_full_loop.py:107 |
| POST /operation-knowledge（legacy 顶层创建） | scripts/smoke_knowledge_no_llm.py:113 |
| POST /operation-knowledge/completeness（强制重算） | scripts/e2e/verify_fix_api.cjs:63（前端只 GET，走 TTL 缓存） |

**B. 集成测试 HTTP 层消费（2 个）——非死，是隔离红线的测试锚点**

| 端点 | 消费方 |
|---|---|
| GET /contacts/:id | tests/sr176_real_route_isolation.rs:192,199（真 Router HTTP 跨租户 404 断言） |
| GET /decision-reviews/:id | tests/hc004_scope_redlines.rs:1024,1034 |

**C. API 文档在案 / curl 运维（12 个）——API-only 面，前端有替代路径**

| 端点 | 在案证据 | 备注 |
|---|---|---|
| POST /contacts/search-import | docs/real-task-runbook.md:104 curl 示例 | handler 自带 deprecationNote（DEPRECATED） |
| GET /contacts/:id/memory-card | docs/real-task-runbook.md:296 观察项 | 前端记忆卡数据来自 operating-memory 端点 |
| GET /operation-knowledge/usage | runbook:131 + docs/data-and-api.md:334 | 知识 toolTrace 观察；`knowledgeUsageLog.contract.ts` 契约存在但前端零调用 |
| GET /management-agent/commands/:id | data-and-api.md:297 | 前端 confirm/reject 响应即含回执，无需单查 |
| GET /management-agent/tool-catalog | data-and-api.md:298 | 工具目录调试用 |
| GET /operation-domains/:domain/state-machine | data-and-api.md:347 | 前端读整域配置（GET /operation-domains）内嵌状态机 |
| PUT /operation-domains/:domain/state-machine | data-and-api.md:348 | 前端状态机编辑打包进 PUT /operation-domains/:domain 载荷 |
| POST /operation-knowledge/tools/open-evidence | data-and-api.md:332 | open-slice 同 handler 别名（mod.rs:639-642） |
| PUT /operation-knowledge/:id | data-and-api.md:315 | legacy 顶层知识 CRUD |
| DELETE /operation-knowledge/:id | data-and-api.md:316 | 同上 |
| PUT /operation-knowledge/documents/:id | data-and-api.md:320 | 前端用 PATCH（部分更新）；PUT 是全量替换变体 |
| DELETE /operation-knowledge/chunks/:id | data-and-api.md:326 | 硬删；前端一律走 POST archive（软删）。management.rs 风险表提及 `delete_knowledge_chunk`（:1892 全文件唯一出现），但工具执行器**无该分支**、目录亦不注入——该 HTTP 端点是 handler 唯一入口 |

**D. 设计保留（spec 依据的 API-only，3 个）**

| 端点 | 依据 |
|---|---|
| POST /auth/token | JWT Bearer 公共签发入口（auth/middleware.rs:32-34 白名单成员，JWT_ENABLED 门控）；前端 cookie-only 是设计（13 号 §3.1） |
| POST /contacts/search | docs/superpowers/specs/2026-07-10-user-ops-pool-redesign-design.md:124 明示"运营池 UI 入口删除、后端端点无害保留"。management 工具 search_contacts 走 MCP contacts_search，不经此端点 |
| POST /contacts/:id/revoke-principal-exemption | docs/superpowers/specs/2026-07-14-principal-authorization-exemption-design.md:71 设计的豁免撤销配套；前端未接线，目前撤销只能手工 curl |
| （附注）POST /webhooks/wechat | /api 之外的 MCP 回调入口（main.rs:358-361），本就不属于前端契约面 |

**E. 全仓零 HTTP 消费（前端/脚本/docs/tests-HTTP 四面皆无；9 个）——"真·未接线"，其中三组有产品语义后果**

| 端点 | 亲验结论与后果 |
|---|---|
| GET /admin/worker-controls | 全仓（frontend/scripts/docs/tests）rg 零命中；`worker_control.fixture.json` 后端 bless 但前端零 import（13 号 §5-3 单侧闭环复证）。**后果：supervisor 熔断（status=open）后唯一恢复手段 resume 无 UI、无脚本、无 curl 文档**——运维盲区 |
| POST /admin/worker-controls/:worker/resume | 同上 |
| POST /decision-reviews/:id/post-decision/retry | 全仓零消费零文档（rg `post-decision/(retry\|regenerate\|discard)` 除 reviews.rs 自身零命中）。**后果：post_decision_status=failed_terminal 的恢复三动作运营无任何入口**（operations reviews tab 只读、worker-health 面板只展示计数）——功能缺口 |
| POST /decision-reviews/:id/post-decision/regenerate | 同上 |
| POST /decision-reviews/:id/post-decision/discard | 同上 |
| PUT /evaluation-scenarios/:id | 前端评测场景只有新建/删除（EvaluationScenariosPanel），**编辑场景未接线**（改金标只能删了重建） |
| PUT /products/:product_id | 前端产品目录只有新建/归档/恢复，**改名/改价未接线**；docs/data-and-api.md 亦未记录 products 端点族 |
| GET /admin/relationship-type-suggestions | 直连列表零消费——统一收件箱聚合器（9 源之一）取代；approve/reject 有消费 |
| GET /admin/operation-state-policies/:id | 详情零消费（列表有前端消费） |

> E 类"零 HTTP 消费"指 HTTP 面；多数 handler 仍被 Rust 单测/集成测试**直调函数**覆盖（如 worker_controls.rs:89-115、ext_knowledge 导出），故不是编译期死代码——mod.rs 的 `no_orphan_pub_async_route_handlers` tripwire 也不会报它们。

### 3.3 方法与路径（参数）不匹配清单

**HTTP 方法不匹配：0 个。** 前端全部方法（含 PATCH campaigns/taxonomies/documents/ingest-sources、DELETE 带 query 的 content-assets、postForm 两处）与后端挂载方法逐条一致。

**query 参数名错配（camelCase serde vs 前端 snake_case）：2 个真缺陷（本次交叉验证新发现，13/14 号均漏检）**

- **P1【功能必坏】`GET /operation-knowledge/chunks/referrers`**
  - 后端：`ChunkReferrersQuery` 带 `#[serde(rename_all = "camelCase")]` 且 `target_id: String` **必填无 default**（src/routes/knowledge/wiki_edit.rs:840-844 亲读）→ wire 只认 `targetId`。
  - 前端：`fetch("/api/operation-knowledge/chunks/referrers?target_id=…")`（frontend/src/features/knowledge/shared.tsx:933 亲读）。
  - 后果：必填字段缺失 → axum `Query` 反序列化拒绝 → **该请求 100% 返回 400**。Inspector"被引用"折叠区展开即报错（错误被组件 error state 吸收，页面不崩，故长期未暴露）。
  - 佐证矛盾链：后端 doc 注释（wiki_edit.rs:846）自己写 `?target_id=...`（与 serde 行为相矛盾）；docs/data-and-api.md:659 写 `?targetId=`（与 serde 一致）。集成测试经 `ext_knowledge` 直调 handler 传结构体，**绕过 query 解析层**；前端测试 mock fetch——两侧测试都测不出此错配。
- **P2【过滤静默失效】`GET /products?active_only=true`**
  - 后端：`ListQuery` 带 `#[serde(rename_all = "camelCase")]` + `#[serde(default)] active_only: bool`（src/routes/products.rs:40-46 亲读）→ wire 只认 `activeOnly`，收不到时默认 false（= admin 全量含归档）。
  - 前端：`api.get("/api/products?active_only=true")`（frontend/src/features/campaign/ProductMultiSelect.tsx:15 亲读）。
  - 后果：参数被忽略 → **campaign 圈人"买过的产品"多选静默列出已归档产品**（不报错）。危害小于 P1（归档产品的历史成交在 segment 匹配上仍是合法语义），但违背 active_only 意图。
  - 穷尽性：全前端生产代码 snake_case query 参数仅此两处（`rg '[?&][a-z]+_[a-z_]+=' frontend/src` 穷尽核证）；其余 query 全为 camelCase 或单词，抽查 `DecisionReviewQuery`（reviews.rs:23-30）、`LoginPollQuery`（accounts.rs:301-306）均与前端一致。

**路径参数命名不一致（信息性，前端两侧均已正确处理，无功能影响）**

| 项 | 说明 |
|---|---|
| `/contacts/:id` vs `/contacts/:wxid/send-history` | 同一 `/contacts/` 前缀两套 id 空间（ObjectId hex vs 微信号）。前端正确传 `selected.wxid`（CockpitPanel.tsx:181 → legacy.tsx:2009） |
| `/operation-knowledge/chat/:session_id` vs `/knowledge/chat/sessions/:sid/stream` | 同一 session id，参数名 :session_id / :sid 不统一 |
| `/tasks`（列表）vs `/agent-tasks/:id/…`（动作） | 同一资源两个资源名（历史命名） |
| `/admin/principal-escalations/:short_code` | 业务 short_code 非 ObjectId；inbox 下发的 item.id 即 short_code，前端一致 |
| `/admin/lessons-learned/:lesson_id` | 业务 lesson_id 非 `_id` hex；ask_human_inbox 已按 lesson_id 下发（12 号 §2.18） |
| `/products/:product_id` | 业务 slug；前端传 productId ✓ |
| `/accounts/:id/mcp-key` | :id 是账号记录 ObjectId；前端明确用 `accountRecordId` 传（McpKeyForm.tsx:63）✓ |
| `/conversations/:contact_id/messages` | :contact_id 是 contact ObjectId（handler 内 find_contact_by_id）；前端传 contact.id ✓ |

---

## 4. 需回写修正清单

1. **11 号 §1.2**：知识段挂载数 "66" → **70**（mod.rs:480-726，本表 #64-#133）；admin/evolution 段 "60+（62）" → **58**；并注明自"130"号起序号整体 +4 偏移（内容本身无错）。
2. **14 号 §2.11/§4.1**：补录两处 query 参数错配缺陷（§3.3 P1/P2）；P1 直接影响其"ChunkReferrersList 懒加载"描述的功能有效性（实际恒 400）。
3. **14 号 §4.1 user-ops 行**：`POST /api/contacts/import` 归属澄清（唯一调用点在 ask-human-config/DeciderChainEditor.tsx:126）。
4. **PROJECT_UNDERSTANDING_LEDGER**：登记 P1/P2 两个新缺陷（源码级修复建议：后端对两个 Query 结构加 `#[serde(alias = "target_id")]` / `#[serde(alias = "active_only")]`（兼容两拼写），或前端改发 `targetId`/`activeOnly` 并同步修正 wiki_edit.rs:846 doc 注释——两侧择一，需同步 docs/data-and-api.md:659 口径）。
5. **docs/data-and-api.md**（文档不全，供后续文档任务）：缺 products 端点族、evaluation-scenarios 族、post-decision 三动作、worker-controls、outcomes/autonomy、observability 三端点等的记录；其 :659 `?targetId=` 是对的（与 serde 一致），反而是后端源码注释（wiki_edit.rs:846）错。
6. **13 号**：无需修正（逐表复核一致）；其 §5-3"三个 fixture 无前端对账"之一 `worker_control.fixture.json` 由本对账升级为"端点全仓零消费"（§3.2-E）。
7. **产品/工程决策项**（非记录回写）：§3.2-E 的三组"真·未接线"（worker resume 无任何操作路径、post-decision 恢复三动作无 UI、产品/评测场景编辑无 UI）是否补 UI 或明示 curl 运维文档，需产品决策；management.rs:1892 风险表含 `delete_knowledge_chunk` 但工具执行器无分支、目录不注入，属 11 号范围的悬空条目，一并提示。

---

## 5. 覆盖自证（搜索方式、核对条数）

**后端侧**
- `src/routes/mod.rs` 全文 Read（1-1250 行）；`rg -c '\.route\('` = **235** 与逐行枚举一致（Cursor Grep 工具在该文件恰于 100 条处静默截断，已弃用并记录）；多方法路由 33 处逐一登记（30×2 + 2×3 + 1×4）→ **272 方法+路径**。
- `src/main.rs` 路由装配段 grep 核证（nest /api、POST /webhooks/wechat、SPA fallback）。
- 针对性亲读：`src/routes/knowledge/wiki_edit.rs:839-852`（ChunkReferrersQuery + doc 注释矛盾）、`src/routes/products.rs:38-97`（ListQuery/Create/Update）、`src/routes/reviews.rs:23-30`、`src/routes/accounts.rs:299-312`、`src/routes/management.rs`（工具直调面 8 处 + delete_knowledge_chunk 全文唯一出现:1892）。

**前端侧**
- 调用点搜索：`rg "api\.(get|post|put|patch|delete|postForm|postRaw)\s*[<(]|(^|[^a-zA-Z.])fetch\(|new EventSource\(|new WebSocket\(" frontend/src -g '*.ts' -g '*.tsx' -g '!**/__tests__/**'` = **257 处**；URL 字面量提取 `rg -o '["\`]/api/[^"\`]*["\`]'` + 动态段归一（sed `${…}`→`:X`、剥 query）→ 唯一模式清单；首轮 glob 误用（`!__tests__/**` 不生效）导致测试 mock 混入，已用 `!**/__tests__/**` 修正并将全部疑似幽灵逐一定位为测试文件字符串。
- 动态拼接点逐个亲读（15 处）：ChunkReviewCard `${verb}`、atlas PublishBar `${resourceKind}/${id}/${action}`（3 调用点×3 动作）、system-strategy ActiveVersionsBar（3 调用点 endpointPrefix）、legacy ActiveVersionsBar（endpointPrefix=/api/admin/operation-domains）、steward batchAction path、steward gap `${action}`、products `${action}`、operationsStore `${action}`、ProposalReleaseCard `${action}`、SIMPLE_ENDPOINTS 表、withAccountScope、App.tsx WS URL、EvolutionCenterTab apiGet/apiPut、useGoLive、ReviewChat；EventSource 3 处与 WebSocket 1 处方法归 GET。
- snake_case query 参数穷尽：`rg '[?&][a-z]+_[a-z_]+='` 仅 2 处（即 P1/P2）。

**对账侧**
- 前端 239 方法+路径逐条在后端 272 表中命中 → 幽灵 0；33 孤儿 = 272 − 239 精确闭合。
- 33 孤儿逐个跑 `rg` 于 frontend/src（复核零调用）→ scripts/ → docs/ → tests/（HTTP base_url 面）四层归类：A 脚本 7、B 集成测试 HTTP 2、C 文档/curl 12、D 设计保留 3、E 全仓零消费 9（7+2+12+3+9=33）。
- 两个参数错配均两侧源码亲读（后端 struct + 前端调用行 + 后端 doc 注释 + docs/data-and-api.md 四方交叉）。
- 与 11/12/13/14 号逐表核对：11 号 2 处计数错误 + 序号偏移；12 号无错误；13 号无错误；14 号 1 处归属标注 + 2 处缺陷漏检。

**残余边界**：`frontend/src/__tests__` 内的 mock URL 不代表运行时契约（已排除）；`frontend/walkthrough.py`（演练脚本）消费的 5 个端点均为前端已用端点，不影响孤儿判定；Rust 集成测试对 handler 的**函数直调**（绕过 HTTP）未逐一清点——它不构成 HTTP 契约消费，只影响"handler 是否死代码"的判定（本对账在 §3.2-E 已注明）。
