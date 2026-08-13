# 30 · 全局事实卡手册（单一速查入口）

> **用途**：改任何东西之前，先查这里的闭集/阈值/键约定/红线。本手册从 01–19 号深读记录的"事实卡速查"节（15/16 号为"不变量总表"节）抽取合并；同类合并、冲突处已亲读源码裁决。
> **来源标注**：每条末尾 `[NN]` = 来源记录号；`✅` = 本次汇编时亲读源码核验（2026-08-13，未提交工作树，行号以当日为准）。
> **裁决声明**：§0 列出全部 8 处记录间冲突及裁决依据；被裁决否定的旧口径不再出现在正文表格中。

---

## 0. 冲突裁决台账（8 处，全部亲读源码裁决）

| # | 冲突 | 各方主张 | 裁决（✅亲验） |
|---|---|---|---|
| C1 | `EVOLUTION_ENABLED` 默认值 | [09][10][17] false vs [18] 转述 07-15 审计 [S-02]"真实默认 true" | **false**。`EVOLUTION_ENABLED_DEFAULT="false"`（config.rs:7）+ 测试 `evolution_enabled_defaults_to_false`（config.rs:889-892）锁定。[18] 转述的是历史审计快照，不代表现状 |
| C2 | run source_kind 闭集 | [02] 3 值（models.rs 注释）vs [01] 6 值 | **6 值**：`inbound_message / follow_up_task / manual_send / principal_escalation / principal_clarification / system_incident`（run_envelope.rs:34-45）。models.rs L3429-3431 字段注释滞后（只列前 3） |
| C3 | outbox status 闭集 | [19]（biz-test `_lib` docstring）与 [17]（docs）5 值 vs [02][05][12] 6 值 | **6 值**：`pending / in_flight / sent / failed_terminal / canceled / delivery_unknown`（OutboxStatus enum，outbox.rs:44-57；models.rs:3503-3505 注释同）。5 值是 biz-test 常规终态窄口径 |
| C4 | GATEWAY_STATUS_VALUES 大小 | [06] 40 值 vs [13]（前端 39 labels）vs [17]（docs 24 值） | **39 值**（逐一亲数 run_envelope.rs:87-147）。[06] 计数误差 +1；[17] 是文档旧快照 |
| C5 | gap signal kind 总数 | [02] 8（db/mod.rs 注释）vs [17] 10（docs）vs [07] 12 | **12 值** = 结构 lint 9 类 + 在线 3 类。亲验 `dangling_anchor`（gap_signals.rs:346）、`recall_miss`（gap_signals.rs:443）、在线三类构造点（knowledge_agent.rs:1932/1966/1985）。db/mod.rs:384-386 注释滞后 |
| C6 | supervised worker 数量 | [17] 文档链 12/13/14 vs [09] 16 | **16 个**（`SUPERVISED_WORKERS`，supervisor.rs:34-51 逐一亲数）。文档系时点快照 |
| C7 | 知识 provenance source 第五值 | [02]"m055 另有 lesson_promotion" vs [07]"principal_authorized" | **两者是不同层字段，都对**：`chunk_revisions.source` 闭集 5 值 = `ai / human / rule / imported / principal_authorized`（ProvenanceSource enum，chunk_revisions.rs:98-138）；`lesson_promotion` 是 **chunk 文档 `provenance.source`** 的第 5 个观察值（routes/lessons_learned.rs:47、indexes.rs:172/1619 partial filter），不属 revision source 闭集 |
| C8 | campaign status 取值 | [11] 4 值（handler 观察）vs [02] 6 值 | **闭集 6 值**：`draft / previewed / confirmed / dispatching / completed / canceled`（models.rs:683-690）。[11] 记录的是本组 handler 实际写的子集，非冲突 |

另记两处**口径并存**（非冲突，排障时两种都要匹配）：
- `AgentRunLog.outbox_status` 字段注释只列 5 值（models.rs:3473-3474），但 dispatcher run 级聚合实际还回写 `partially_sent` 与 `delivery_unknown`（聚合优先级 in_flight > pending > sent/partially_sent > failed_terminal > delivery_unknown > canceled，outbox_dispatcher.rs:980-1023）[05]。
- source_kind 双口径：outbox.source_kind 用 `trigger.kind()`（`inbound / follow_up` 两值，types.rs:1852-1857），envelope 用 SOURCE_KIND_*（6 值）；`PROACTIVE_TOUCH_SOURCE_KINDS=["follow_up","follow_up_task"]` 同时覆盖两口径（gateway.rs:5630-5635）[01]。

---

## 1. 状态闭集总表

> 写库断言点约定：**A** = 写前 assert/validate 函数（debug panic / release 拒写），**E** = 闭集枚举类型（编译期），**V** = 写库校验拒收，**N** = 注释级约定（无强制）。

### 1.1 主链路（contact / task / run / outbox / review）

| 枚举 | 合法取值 | 定义 | 断言点 |
|---|---|---|---|
| `contacts.agent_status` | `normal / managed`（仅 managed 获自动回复） | models.rs L6-11 ✅（AgentStatus enum） | E [02][03] |
| `conversation_messages.direction` | `inbound / outbound` | models.rs L804-809 | E [02] |
| `conversation_messages.handoff_status` | `pending / materialized / deferred(遗留只读) / ignored_not_managed`；状态机 pending/deferred→materialized\|ignored_not_managed 终态不可回 | webhooks.rs:28-31 | V（mark filter 限 `$in [pending,deferred]`，webhooks.rs:65）[03] |
| `agent_tasks.status` | `pending / running / committing / retry / failed / cancelled / sent / completed / outbox_enqueued`（9 值）✅ | models.rs:918-928 ✅ | A `assert_agent_task_status_valid`（models.rs:933-941 ✅，全部写点前置）[02][03] |
| run lifecycle | `started / running / completed / failed_before_decision / failed_after_decision / aborted_by_budget / aborted_by_external_signal`（7 值）✅ | run_envelope.rs:48-54 ✅ | V（终态吸收由 `fail_run_envelope_if_open`/`mark_running` filter 保证；`update_run_envelope_terminal` 只做枚举校验不做转移 CAS——[06] 疑点 1）[01][06] |
| run source_kind | 6 值（见 §0-C2）✅ | run_envelope.rs:34-45 ✅ | N（envelope 写入不阻断，W2 finalize 判违规）[01][06] |
| `final_review_status` | `approved / revision_applied_approved / revision_failed / held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context / blocked_by_required_field / blocked_by_budget / blocked_unverified_product_claim / legacy_mode_unchecked`（10 值）✅ | run_envelope.rs:68-79 ✅ | V（R9.10.e 闭集写库拒收）+ 禁值表（见 §7）[01][04][06] |
| `gateway_status` | 39 值闭集（见 §0-C4）✅：pending/approved/allowed/sent/no_reply/review_blocked/revision_failed/revision_skipped_invalid_direction/revision_skipped_budget_exceeded/revision_llm_failure/held_by_ai_policy/blocked_by_safety_guard/ai_waiting_for_more_context/blocked_by_required_field/blocked_by_budget/blocked_unverified_product_claim/tool_loop_timeout/legacy_mode_unchecked/not_managed/cooldown/rate_limited/daily_limit/expired/context_changed/policy_cooldown/policy_wait_user_reply/gateway_blocked/precheck_blocked/outbox_enqueuing/outbox_enqueued/outbox_enqueue_failed/outbox_enqueue_partial_failure/stale_task_claim/skipped_duplicate/admin_cancelled/superseded_by_new_inbound/user_reaction_stop_requested/quiet_hours_deferred/internal_error | run_envelope.rs:87-147 ✅ | V + 禁值表 [01][06]。注意：precheck 内部态 `policy_consecutive_limit`（gateway.rs:5406-5418 [01]）不在此闭集（不落 run log 终态）；task.gateway_status 另有自由文本（`{kind}_committing / aggregated / policy_reconciled / profiled` 等）[03][11] |
| hold_category | `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`（3 值）✅ | types.rs:1507-1516 ✅ | E + 禁值常量（types.rs:1520+）[01][04] |
| `agent_send_outbox.status` | 6 值（见 §0-C3）✅；终态 `sent / canceled / failed_terminal / delivery_unknown` 无出边，delivery_unknown 禁自动重放 | outbox.rs:44-57 ✅ | E（OutboxStatus enum + as_str 锁字面量，outbox.rs:62-70）；转移矩阵见 [05] §4.1 |
| outbox run 级聚合 | 上 6 值 + `partially_sent` | outbox_dispatcher.rs:980-1023 | N [05] |
| `agent_tasks` 转移要点 | claim CAS：pending/retry/failed→running；prepare/finalize/requeue：running→committing→sent\|retry；授权：running→outbox_enqueued；durable 行可从任意态复活为 pending | tasks.rs:203/60/87/119/516；webhooks.rs:178-204 | [03] §4.1 全矩阵 |

### 1.2 决策/评审/反应/记忆

| 枚举 | 合法取值 | 定义 | 断言点 |
|---|---|---|---|
| `decision_phase` | `tool_calling / final` | types.rs:708-709 ✅ | E [04] |
| `risk_level` | `low / medium / high`（无 critical） | types.rs:711 ✅ | promote 非法→risk 标签+清空 [04] |
| `knowledge_need` | `not_required / required / insufficient`；guards.rs:70-75 另容忍遗留 `knowledge_required`（planner 宽、协议严） | types.rs:712 ✅ | 同上 [04] |
| `run_mode` | `fast_chat / memory_candidate / knowledge_grounded / high_risk` | types.rs:713-718 ✅ | 同上 [04] |
| `autonomy_mode` | `auto / assisted / blocked` | types.rs:719 ✅ | 同上 [04] |
| `conversation_mode` | `casual_relationship / value_exchange / consultative / boundary_protection`（默认第一个；profile 可整体替换集合） | types.rs:720-725 ✅（字典 seed m028） | 非法落最保守默认 [02][04] |
| 决策 tool 名 | `knowledge.list_catalog / knowledge.search / knowledge.open_slice` | types.rs:726-730 ✅ | `invalid_tool_call:<tool>` risk [04] |
| 状态动作（state policy action） | `reply / acknowledgement / silent / follow_up / cooldown` | guards.rs:260-266；m057:14-20 `KNOWN_STATE_ACTIONS` | [02][04] |
| reaction outcome_status | 推导层：域词 passthrough / `user_replied_stop_requested` / 正极首词（DEFAULT `user_replied_buying_signal`）/ `user_replied_objection` / `user_replied_unclassified`；DEFAULT 负极 5 词 objection/stop_requested/unsubscribed/negative/complaint；misjudge 信号 `approved_but_user_negative` | reaction.rs:678-701/838-844；gap_signals.rs:778 | [06] |
| memory 候选生命周期 | pending →（consolidator OCC 赢）consolidated；或建档即 `ignored_low_score`（终态）；OCC 输留 pending | memory.rs:2156/2540-2549/2833-2841 | [06] |
| `principal authorizationMode` | `affirm_or_condition / deny_only / none` | review/mod.rs:414-423 | [04] |
| ReviewScores 字段/alias | humanLike、emotionalValue、hallucinationScore(=factRisk)、knowledgeGroundingScore(=productAccuracy)、pressureRisk、boundaryPrivacySafety；live wire 六键各恰一形 0..=10，alias+canonical 同现→invalid | types.rs:1420-1445；review/mod.rs:3011-3036 | V [04] |

### 1.3 escalation / referral / 管理面

| 枚举 | 合法取值 | 定义 | 断言点 |
|---|---|---|---|
| escalation status | `pending / resolved / delivery_failed`（3 值）✅ | models.rs:4491-4498 ✅ | E 常量 [02][05][12] |
| escalation category | `out_of_scope_decision / high_risk_gated / stuck_or_undelivered`（3 值）✅ | models.rs:4533-4540 ✅ | E [02][05] |
| principal verdict | `approved / rejected / conditional / deferred / delegated_back`（5 值）✅ | models.rs:4564-4575 ✅ | V：越界 sanitize 回落 deferred（logic.rs:413-425）[02][05] |
| exemption_type | `none / customer_only / knowledge` ✅ | models.rs:4578-4580 ✅ | [02][05] |
| relay_state | `pending → enqueued → terminal` ✅ | models.rs:4500-4502 ✅ | [02][05] |
| 请示卡投递态 | `pending_enqueue → queued → sent \| failed_terminal \| delivery_unknown` ✅ | models.rs:4504-4508 ✅ | 转移：ledger.rs:169-185/241-282；reassign 回 pending_enqueue+代数+1 [05] |
| relay 哨兵 | `__PRINCIPAL_RELAY__`（models.rs:841）；出站泄漏守卫另拦 `verdict=/substance=/constraints=` | logic.rs:214-219 | [05][06] |
| `agent_command_runs.status` | `pending_confirmation / running / dry_run / succeeded / failed / execution_unknown / canceled`（7 值）✅ | models.rs:4063-4071 ✅ | A `validate_agent_command_run_status`（models.rs:4073-4081 ✅）[02][11] |
| `agent_tool_calls.status` | `prepared / executing / dry_run / succeeded / accepted / failed / executed_unverified / execution_unknown`（8 值；**accepted≠succeeded**：入队≠送达）✅ | models.rs:4113-4122 ✅ | A（models.rs:4124+ ✅）[02][11] |
| ToolRisk | `Readonly / Low / Dangerous / Irreversible` + explicitly_classified（fail-closed 兜底 Dangerous+false） | management.rs | [11] |
| campaign.status | 6 值（§0-C8）✅；dispatch 可入态 = draft/previewed/dispatching | models.rs:683-690 ✅ | A `assert_campaign_status_valid`（models.rs:692+ ✅）[02][11] |
| `campaign_sends.status` | `prepared / enqueued`（+读侧 `skipped_duplicate`） | models.rs L674 注释级 | N [02][11] |
| guide preview.status | `pending / applying / applied / failed / stale`（apply_protocol_version=3） | guides.rs | [11] |
| 候选/建议/疑似成交审核态 | `pending →(approving 瞬态，仅 taxonomy 事务内)→ approved / rejected` | models.rs:3765/3803/3837；admin_taxonomy_candidates.rs:287 | 部分唯一索引限一 pending [02][12] |
| media/referral 审核 | media review_status `draft/approved`、media_type `image/file/video`、min_inject_tier `lean/relational/full`（非法落 full）；referral card `draft/approved`×enabled；assist override `default/force_on/force_off` | routes/media_assets.rs、referral_cards.rs | [11] |
| 成效事件 | verification 直登闭集 `staff_confirmed / payment_verified`（conversation_inferred 拒直登）；event_kind `deal / reversal`（reversal 必须带 product_id）；进投影闭集同（entitlements.rs:51-53） | models.rs:456-468 | [02][04][11] |

### 1.4 知识子系统

| 枚举 | 合法取值 | 定义 | 断言点 |
|---|---|---|---|
| chunk `status` | `draft / active / rejected / archived` | [07][08] 观察值 | 经 revision harness 控制写 |
| chunk `integrity_status` | `verified / needs_review / needs_human_audit / rejected / missing_evidence / draft`（观察全集；核心三态 verified/needs_review/needs_human_audit） | agent.rs:1246-1247；digest:252 | verify 主闸见 §7 [07][08][12] |
| `chunk_revisions.op` | `create / patch / split / merge / rollback / archive / restore / verify / unverify / reject`（10 值） | chunk_revisions.rs:65-94；models.rs L2040 | E RevisionOp [02][07] |
| `chunk_revisions.source` | 5 值（§0-C7）✅ | chunk_revisions.rs:98-138 ✅ | E + FromStr 拒收 ✅ |
| chunk 文档 `provenance.source` | 观察值 `ai / human / rule / imported / lesson_promotion`（§0-C7）✅ | models.rs L1993 注释 + lessons_learned.rs:47 ✅ | N |
| `wiki_type` | `source / entity / concept / comparison / synthesis / methodology / finding / query / thesis`（9 值）；权重序 thesis 90>synthesis 80>methodology 70>finding 60>comparison 50>concept 40>entity 30(None 同)>source 20>query 10>未知 0 | models.rs L1865-1875；agent.rs:1781-1794 | 读侧 wiki_type_priority 兜底 [02][07] |
| `chunk_type` | DEFAULT 销售四态 `product_fact(fallback) / style_template / peer_case / negative_example`；可被 DomainProfile.chunk_roles 替换 | models.rs L1878-1883 | 缺省/越界→product_fact [02][07] |
| RelatedRef.kind | `superseded_by / references / requires / contradicts / clarifies / refines`（6 值）→ 角色映射：contradicts=Contradiction（可看不可 cite）、superseded_by=Version（redirect 不扩散）、其余+未知=Support | models.rs L1824；agent.rs:1534-1541 | [02][07] |
| gap signal kind | **12 值**（§0-C5）✅：结构 9 `orphan / broken_link / no_outlinks / contradiction / stale / missing_chunk / suggestion / low_confidence / dangling_anchor` + 在线 3 `recall_miss / recall_low_yield / citation_format_rejected` | gap_signals.rs:200-396 ✅；knowledge_agent.rs:1893-1997 ✅ | dedup 键见 §3 [07] |
| gap signal status | `pending / auto_resolved / llm_resolved / applied / dismissed`（5 值） | models.rs L2089；observability.rs:1362-1368 | [02][08][12] |
| gap severity/source | `error/high/warning/medium/info` 映射见 [07] §4；source `rule / llm` | models.rs L2085-2088 | [02][07] |
| `import_jobs.status` | `pending / running / completed / failed`；`apply_status`：`ready / applying / applied`（legacy 无值报 `import_preview_not_ready:legacy`） | models.rs:1052-1071/1006-1010 | A validate/assert 全写点 [02][08] |
| `knowledge_chat_tasks.status` | `pending / running / completed / failed / cancelled`；step verdict `committed / noop / needs_manual / failed` | models.rs:5934-5935；knowledge_task:1178-1199 | worker debug_assert [02][08] |
| `knowledge_chat_turns.status` | `pending / applying / applied / discarded` | models.rs L3273；chat.rs:775/898/980 | [02][08] |
| digest | report status `ok / partial / failed`（latest_attempt 另有 running；audit 另有 superseded）；卡 kind 7：`chunk_missing_field / chunk_low_hit_rate / chunk_caused_block / pack_outdated / evolution_pending / evolution_released / freeform`；suggestedAction 6：`fix_chunk / add_chunk / retag / review_evolution / dismiss / freeform`（可派工 5，排 freeform）；severity `info / warn / critical` | models.rs:5852-5895；digest:812-829/1002 | [02][08] |
| `ingest_sources` | kind `rss / html`；status `active / failing / disabled` | models.rs:2126-2148 | [02][07] |
| `catalog_rebuild_jobs.status` | `queued / processing / done / superseded / discarded / failed` | models.rs L3204 | [02] |
| operator memory kind | `preference / rejection / context` | models.rs:6006-6007 | [02] |
| lessons | pattern `success / reviewer_misjudge_negative / blocked_by_safety_guard`；review_status `pending_review → promoted` | observability.rs:1510-1514；lessons_learned.rs:232/259 | [12] |
| chat 工具名闭集（9） | `knowledge.list_catalog / search / open_slice / audit_completeness / search_chunks / propose_repair / analyze_logs / open_document / verify_anchor` | knowledge_tools.rs:91-101 | [07] |
| knowledge_agent action（5） | `list_catalog / open_document / open_chunk / follow_relations / answer` | knowledge_agent.rs:225-257 | [07] |
| StructuralKind（5） | `split / merge / reclassify / mark_superseded / rewrite_directory_intent`（提案只进不出，KB-06 就绪债） | structural_proposals.rs:28-52 | [07] |

### 1.5 版本/配置/基础设施

| 枚举 | 合法取值 | 定义 | 断言点 |
|---|---|---|---|
| `agent_souls.status` | `draft / published / archived` | m042:140 校验闭集 | V [02] |
| `prompt_templates.status` | `draft / active / archived` | m043:172 | V [02] |
| `domain_profiles.release_status` | `draft / published`（缺省 published）× current_version × is_active 三轴 | models.rs:2440-2446；domain_profiles.rs:9-18 | [02][12] |
| DomainField.kind | `string / enum / number / date / reference`；fields≤64 | models.rs L2183；domain_schemas.rs:83/571 | [02][12] |
| `llm_provider_configs.format` | `openai / anthropic` | models.rs:6035-6036 | [02] |
| `llm_call_logs.final_status` | `success / failed / json_error / cache_hit` | models.rs:3890-3892 | [02] |
| `experiments.status` | `collecting / evaluating / awaiting_admin / released / aborted`（生产 tick 只走 collecting→awaiting_admin） | envelope.rs:60-63；mod.rs:263 | [02][10] |
| `proposals.status` | `pending_eval / evaluating / eligible_for_release / rejected_below_threshold / released / rolled_back`（6 值；写点矩阵见 [10] §4） | models.rs:5666-5667；evolution.rs:545-552 | [02][10][12] |
| proposal.proposed_section | `soul / system_contract / policy / operator_instruction` | models.rs L5685 | [02] |
| `shadow_replays.status` | `completed / failed` | models.rs L5731 | [02] |
| threshold audit action / decided_by | `released / rolled_back / auto_released` / `admin:<id> / evolution_auto / evolution_release / evolution_rollback` | models.rs:5790-5796 | [02] |
| post_decision_status | `prepared / pending / retry / processing / completed / failed_terminal / discarded` | observability.rs:1102-1110 | [06][12] |
| worker 熔断状态 | `closed / open / half_open / probing`；控制行 `_id=worker::{name}` | supervisor.rs | [09] |
| `taxonomy_values.status` | `active / deprecated`；check_value 四分支 Active/AliasActive(改写)/Deprecated(risk 不改写)/CandidateNew(risk+候选收集) | models.rs:3680-3681；taxonomy.rs:393-441 | scope 回落恒 [account_id,"global"] [02][04] |
| `admin` 会话/用户 | admin_users 唯一 username；admin_sessions 唯一 session_id；TTL 见 §2 | indexes.rs | [02][12] |
| decision_review_phase（UI 投影） | `sent / auto_rewrite_sent / queued / auto_rewrite_queued / partially_sent / delivery_failed / delivery_canceled / delivery_unknown / gateway_blocked / approved / auto_rewrite_approved / final_blocked / auto_rewrite_failed / auto_rewrite_in_progress / review_recorded`（15 值） | reviews.rs | [11] |
| performance path | `direct / escalated / rewrite / revision / no_reply / manual` | observability.rs:51-58 | [12] |

---

## 2. 阈值与默认值总表

### 2.1 五闸阈值（评分闸体系；runtime 可覆盖，evolution 可提案）

全部默认亲验于 models.rs:4877-4978 ✅；判定式亲验记录 [04]（gates.rs）；evolution 侧两处硬常量同值（threshold.rs:294-304、replay.rs:380-389 [10]）；UI 基线同值（evolution.rs:514-524 [12]）。

| 闸 | runtime 参数 | 默认 | 判定式（触发方向） | 性质 |
|---|---|---|---|---|
| 事实风险 | `hallucination_block_at` | **6** ✅ | hallucinationScore（alias factRisk）**≥6 拦**（gates.rs:126） | 硬拦（安全闸→held_by_ai_policy） |
| 压迫风险 | `pressure_risk_block_at` | **7** ✅ | pressureRisk **≥7 拦**；0=legacy 豁免（gates.rs:172-173） | 软闸（安全闸→blocked_by_safety_guard） |
| 知识着地 | `knowledge_grounding_block_below` | **7** ✅ | knowledgeGroundingScore（alias productAccuracy）**<7 拦**（gates.rs:140-141） | 硬拦（安全闸→blocked_unverified_product_claim） |
| 拟人度 | `human_like_rewrite_below` | **6** ✅ | humanLike **<6 重写一次**（gates.rs:154） | 软闸 rewrite |
| 情绪价值 | `emotional_value_rewrite_below` | **6** ✅ | emotionalValue **<6 重写一次**（gates.rs:186） | 软闸 rewrite |
| 边界隐私（无参数） | — | 固定 | ≤3 拦 / ≥4 放 / 0 豁免（gates.rs:200-201） | 固定 [04] |

gate 命中方向常量：fact/pressure=GTE、human/emotional/product=LT（replay.rs:42-52）[10]。安全闸→status 权威映射（#152）：fact→held_by_ai_policy、pressure→blocked_by_safety_guard、product→blocked_unverified_product_claim（significance.rs:52-59）[10]。`SEND_SUCCESS_STATUSES=["approved","revision_applied_approved"]`（significance.rs:42，send-success 唯一口径）[10]。

### 2.2 runtime 参数默认（RuntimeParametersTyped，models.rs:4877-4978 ✅ 全亲验）

| 参数 | 默认（clamp） | 参数 | 默认（clamp) |
|---|---|---|---|
| recent_message_limit | 12 | run_token_budget | 150_000 |
| min_reply_interval_seconds | 20 | run_token_budget_escalated | 500_000（须≥run，runtime.rs:255） |
| max_daily_touches | 3 | run_max_llm_calls | 6 |
| max_pending_follow_ups | 3 | simulation_token_budget | 300_000 |
| follow_up_expires_hours | 48 | reaction_token_budget | 8_000 |
| cooldown_after_no_reply_hours | 24 | reaction_max_llm_calls | 2 |
| operation_state_confidence_full_review_below | 4（<4 强制 full review，review/mod.rs:3251） | autonomy_protocol_enabled | true |
| knowledge_max_tool_calls | 6（1..16） | knowledge_open_slice_max_k | 4（1..16） |
| knowledge_search_top_k | 8（1..32） | outbox_poll_interval_seconds | 5（1..60） |
| outbox_lease_seconds | 60（10..600） | quiet_hours_enabled | true |
| quiet_hours_start / end / tz | 22 / 8 / +8（tz clamp -12..14；start==end 永不静默） | consolidation_window | 6000 字（1000..16000）/ 60 条（10..200） |
| bayesian_slot_min_hits / min_strong | 3（1..20）/ 2（0..20） | planner_block_rate_threshold override | 限 [0.05,0.95]（runtime.rs:686-691） |

R1.4/R1.5/R1.6 决策长度门：常规回复理由 ≥10 chars ≥6 汉字；critical 轮 7 字段 ≥20 chars 禁 unchanged、回复理由 ≥30 chars ≥12 汉字；low_routine 仅 knowledge_need_reason/self_critique ≥6 chars（types.rs:1017-1082）[04]。

### 2.3 config 全表（env → 默认 →clamp；config.rs:465-814。★=本次亲验行）

**基础/身份**：APP_HOST=0.0.0.0；APP_PORT=8080；APP_BASE_URL=http://localhost:8080；MONGODB_URI=mongodb://localhost:27017；MONGODB_DATABASE=wechatagent；MCP_BASE_URL=http://47.108.57.147:3001（生产现役另为 117.72.54.28:3001，[19] M9）；**MCP_API_KEY / OPENAI_API_KEY 必填缺失启动失败**；OPENAI_BASE_URL=https://api.openai.com/v1；OPENAI_MODEL=gpt-4.1-mini；DEFAULT_WORKSPACE_ID / DEFAULT_ACCOUNT_ID=default [09]

**回复节奏**：AGENT_RECENT_MESSAGE_LIMIT=12；AGENT_MIN_REPLY_INTERVAL_SECONDS=20；ACCOUNT_SEND_MIN/MAX_INTERVAL_MS=1000/4000（0=关）；AGENT_REPLY_MAX_SEGMENT_CHARS=120（≥1）；AGENT_REPLY_MAX_SEGMENTS=4（≥1）；MESSAGE_DEBOUNCE_WINDOW_MS=2000 clamp[1000,10000]★；COMPLETENESS_CACHE_TTL_SECONDS=300★ [03][09]

**LLM/worker**（★config.rs:489-541 全亲验）：LLM_TIMEOUT_SECONDS=45；LLM_MAX_RETRIES=5；LLM_RETRY_BASE_MS=1500；LLM_MAX_CONCURRENCY=4 clamp[1,64]；LLM_FOREGROUND_RESERVED=2 clamp[1,64]；TASK_WORKER_INTERVAL_SECONDS=30；INBOUND_REPLY_WORKER_CONCURRENCY=4 clamp[1,32]；TASK_CLAIM_TIMEOUT_SECONDS=300；IMPORT_WORKER_INTERVAL_SECONDS=2；IMPORT_JOB_CLAIM_TIMEOUT_SECONDS=600；REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS=60；POST_DECISION_WORKER_CONCURRENCY=4 clamp[1,32]；POST_DECISION_MAX_ATTEMPTS=8 clamp[1,100]；POST_DECISION_SNAPSHOT_MAX_BYTES=2_097_152 clamp[256KiB,8MiB]；POST_DECISION_PROMPT_MAX_CHARS=80_000 clamp[8000,500000]；POST_DECISION_TOKEN_BUDGET=32_000 clamp[1000,500000]；POST_DECISION_FAILED_SNAPSHOT_RETENTION_DAYS=14 clamp[1,365]；WEBHOOK_RATE_LIMIT_WINDOW_SECONDS/CAPACITY=60/30

**Strategic Planner 全系**：INTERVAL=600s；SILENT_THRESHOLD=72h；DAILY_EMIT_CAP=20；COMMITMENT_IMMINENT_WINDOW=8h；COMMITMENT_FALLBACK_DUE=72h（0=禁）；COMMITMENT_EMIT_DEDUP=24h；STAGE_STAGNATION=14d / RECENT_INBOUND=24h；CALENDAR lookahead 1d/cap 3/tz+8；RENEWAL 14d/grace 7d/cap 3；REACTIVATION dormant 30d/cadence 30d/cap 3；BLOCK_RATE window 24h/min_runs 3/threshold 0.6 [09][10]

**发送治理**：HOLDING_REPLY_MIN_INTERVAL_HOURS=6.0★；HOLDING_REPLY_TOKEN_BUDGET=3000★；ACCOUNT_DAILY_SEND_SOFT_CAP=500★（仅告警不拦）；CAMPAIGN_MAX_AUDIENCE=500★（硬上限，**0=全拒非不限**）；WAKE_JITTER_MAX_SECONDS=900（0=恒整点）；COLD_CONTACT_THRESHOLD_HOURS=168★/DAILY_EMIT_CAP=5★ [05][09]

**自学习/换血**：SILENCE_THRESHOLD_SECONDS=86400★/INTERVAL=600★/DAILY_CAP=500★；DYNAMIC_CONFIDENCE_MIN_SAMPLES=5★；VALUE_TIER_MID/HIGH_THRESHOLD_CENTS=50000/300000；KNOWLEDGE_EXPLORATION_TEMPERATURE=1.0★ [09]

**演化器**（★config.rs:684-739 全亲验）：EVOLUTION_TICK_SECONDS=21600；RUN_TOKEN_BUDGET=60000；RUN_MAX_LLM_CALLS=30；EVAL_WINDOW=72h；MIN_REPLAYS=30；MIN_SEND_SUCCESS_DELTA=0.05；MAX_5GATE_HIT_INCREASE=0.10（边界含）；MAX_SAFETY_REGRESSION_RATE=0.0（零容忍）；REPLAY_CONCURRENCY=4 / MAX_FAIL_RATE=0.30；THRESHOLD_RELEASE_COOLDOWN=24h；COHORT_PER_CONTACT_CAP=3 / SAMPLE_PER_FAILURE_BUCKET=10；MAX_NEGATIVE_REACTION_INCREASE=0.05；AUTO_RELEASE_WINDOW=336h / PER_TICK_CAP=1 / MAX_NEGATIVE_REACTION_RATE=0.30

**知识**：KNOWLEDGE_DIGEST_RUN_HOUR=9★ / RUN_TOKEN_BUDGET=24000★ bounded[1,1e6] / RUN_MAX_LLM_CALLS=8★ bounded[1,100]；KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=30★（0=停）；CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=3★（0=停）；KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=600★（0=停）；INGEST_WORKER_INTERVAL_SECONDS=3600★ [08][09]

**鉴权/JWT/webhook/媒体**：SESSION_TTL_HOURS=168；SESSION_COOKIE_SECURE=false；SYSTEM_OPERATOR_USERNAMES=""（**空=全拒 fail-closed**）；AUTH_RATE_LIMIT window 300s / client 20 / target 10 / global 100；WEBHOOK_TIMESTAMP_SKEW_SECONDS=300（±5min 含边界）；JWT_TTL_MINUTES=60（开启须双 PEM 否则 panic）；MEDIA_STORAGE_DIR=./media；MEDIA_MAX_FILE_SIZE_MB=50；MEDIA_ID_CACHE_TTL_HOURS=24；REVIEWER_SECOND_PROVIDER_FORMAT=openai [09][12]

**.env.example 缺文档**（config.rs 读但未列，[19] §5.3）：COMPLETENESS_CACHE_TTL_SECONDS、DYNAMIC_CONFIDENCE_MIN_SAMPLES、EVOLUTION_MAX_SAFETY_REGRESSION_RATE、POST_DECISION 族 6 个、SILENCE_SIGNAL 族 4 个。`APP_ENV` 反向：example 有、config.rs 不读（由 migrations/mod.rs:583 读，破坏性迁移守卫）。

### 2.4 budget 限额

| budget | 值 | 出处 |
|---|---|---|
| 主 run | 150k token / 6 calls（escalated 500k） | runtime 默认 ✅ [06] |
| rewrite/revision 预算包 | `4 + (second_reviewer?1:0)` 次调用；forced_full +1 | gateway.rs:3049/3261/2728 [01] |
| reaction | 8k / 2 calls | ✅ [06] |
| simulation | 300k | ✅ [06] |
| post_decision 投影 | 32k token / **1 call** | post_decision.rs:448-453 [06] |
| holding reply | 3000 token / 2 calls | config ★ + holding_reply.rs:144-149 [05] |
| evolution tick | 60k / 30 calls（实际只约束 Critic；shadow 走 per-replay RunBudget=simulation 预算，[10] 疑点 4） | ★ [10] |
| knowledge：repair 每轮 | 4k / 4 calls / ≤3 轮 | knowledge mod.rs:1390-1392 [08] |
| knowledge：chat 每轮 | 24k / 4 calls；session ≤8 assistant 轮；followups ≤3；合约修复 ≤2 | mod.rs:1490-1495 [08] |
| knowledge：task step | 8k / 4 calls | knowledge_task:30-33 [08] |
| knowledge：digest run | 24k / 8 calls ★ | config [08] |
| knowledge：import | 总 600k token、max_calls=段数×3、≤200k chars、≤64 段 | import.rs:204-206/708-716 [08] |
| knowledge：auto-verify | 240k / 100 calls；threshold 默认 7 [0,10]；limit 50 [1,500]；抽样 0.3 硬下限 0.05 | verify.rs:32-34/217-224/274-275 [08] |

### 2.5 LLM 输出 token 上限（关键路径）

`user.reply.fast.task`=8192、`user.review.light.system`=3072、`user.review.system`=8192、`user.review.claim_gate`=3072（agent/mod.rs:233-243 ✅）[09]。Anthropic 形态 max_tokens 硬顶 8192（llm.rs:505/760）[09]。

### 2.6 超时/重试/退避全数值

| 机制 | 数值 | 出处 |
|---|---|---|
| LLM 重试 | 5 次 ★ / base 1500ms ★ / 单次退避封顶 60s（尊重更长 Retry-After）/ 指数移位 min(attempt-1,10) / jitter 0..base；429/5xx 重试、400/401/JSON 解析失败不重试 | config ✅ + llm.rs:1361-1386 [09][15] |
| LLM JSON 修复 | 3 层确定性 + 回喂修复 2 次（llm.rs:290） | [09] |
| LLM reqwest 超时 | 45s ★；Registry TTL 30s；provider init 5 attempts | [09] |
| MCP client | 60s（mcp.rs:25）；时序不变量 `60×2 ≤ 150(send timeout) < 180(lease)` | [05][09] |
| dispatcher | poll 5s（runtime ✅）/ send 外层 150s / lease 180s / tick 处理 cap 16 / aging FIFO 每 10 次 claim 一次 | outbox_dispatcher.rs:154/3227 [05] |
| outbox 重试 | `base=(2^clamp(attempt,0,10))×5s ± 20% jitter`（attempt 1/2/3→10/20/40s）；max_attempts=3（enqueue clamp ≤0→3, min 10；gateway 全部条目=3 ✅ gateway.rs:4325）；reclaim 上限 5 次→failed_terminal；每 entry 事件 cap 20 | outbox.rs:599-607/311-315 [01][05] |
| 不耗 attempt 的 defer | task 授权 +1s；账号离线 +60s；pacing 到 last_sent+interval | outbox_dispatcher.rs [05] |
| task 重试 | max_attempts=3（≤0 修 3）；退避 `min(60·2^(attempt-1),900)s ±20%`（60/120/240/480/900 封顶）；LLM 不可用恒 300s 且 `$inc attempt_count:-1` 不耗预算；claim lease 300s ★；心跳 `(timeout/2).clamp(5,60)`；回收 ≥3 次→failed | tasks.rs:1207-1214/1183-1204/865 [03] |
| import worker | tick 2s ★ / claim 600s ★ / recovery ≥3→failed / 心跳 clamp[5,60]；checkpoint TTL 48h、事务重试 5；job 终态 TTL +24h | [08] |
| post_decision | lease=`llm_timeout×retries+retry_base×(retries−1)+60s` 下限 120s；心跳 20s；退避 1s×2^n 封顶 5min；扫描 limit 32；快照消息 20 条/4000 chars、产品 100 条；terminal 快照 14d 后脱敏 ★；scrubber 每 1h | post_decision.rs:810-823/468/1261-1262 [06] |
| reaction | claim 超时 60s ★；确定性 stop cooldown=100 年；stop 词 ≤96 chars / buying ≤120；trajectory 滑窗 50、hint 取最近 5/3 | reaction.rs:97/183/251-254 [06] |
| webhook | 去抖 2000ms ★ clamp[1000,10000]；限流 30 次/60s/(ws,acct) ★ 超限仍计数；限流事件每账号每 UTC 天 1 条；验签 ±300s 含边界；唤醒 jitter ≤900s per-wxid 确定性；manual 孤儿宽限 5min；dispatcher 二次唤醒 authorize 后 1050ms；inbound reply worker 固定 250ms tick | webhooks.rs:929-987/2870 [03] |
| supervisor 熔断 | 退避 1s×2 封顶 30s；60s 窗 5 次 panic→open；轮询 30s；probe 锁 120s / 稳定期 60s ✅ | supervisor.rs:28-32 ✅ [09] |
| knowledge_agent | MAX_ROUNDS=4；catalog 页 30/候选窗 400/摘要 120 chars/open 批 8/follow ≤16/预取 3/redirect 8 跳/catalog 合并 ≤60；cache TTL 300s 容量 256 | agent.rs:45；cache.rs:26-29 [07] |
| chat tool loop | max_loops 4（mutation 1、clarify 3）；toolCalls ≤6/轮；总 30s；单工具 5s（dispatch 5s）；失败连击 3；context 8000 chars；trace 32 | chat_tool_loop.rs:36-44 [07][08] |
| router 预算保留 | GeneratedReply=4（+1 dual）/ ExistingCandidate=3（+1 dual）/ PreviewOnly=1；fallback top 5 | router.rs:568-576/788 [07] |
| roster | 同步内短重试 3×2s；后台 5 次退避 3·2^n；快照过期 24h | mcp.rs:899-923 [09] |
| 版本分配重试 | prompt/soul 各 8 次 | prompt_template_versions.rs:16、soul_versions.rs:20 [09] |
| outbound_fetch | DNS 5s / connect 10s / request 30s / 重定向 ≤5 / body ≤4MiB | outbound_fetch.rs:17-21 [09] |
| gateway 常量 | revision LLM 30s；评审摘要截断 160；请示骚扰门窗 24h；daily_touch 滚动 24h；consecutive 检查深度 20 条；管理发送上下文窗 `(limit×6).clamp(24,80)`；句末标点集 `。！？!?；;.`；超长段硬切 2×max_chars；PROFILE_SUMMARY_SOFT_CAP 2000；MEMORY_SUMMARY 12 行/1200 字节；taxonomy candidate 初始权重 50；commitments 保留 `$slice:-8` | gateway.rs [01] |
| memory cap | core 6 / recent 10 / deprecated 20（extra 镜像 6/10/**6**，[06] 疑点 2）；DEFAULT 八维槽 preferences 8/doNotDo 10/commitments 8/objections 8/openLoops 8/openQuestions 8/confirmedFacts 12/conflicts 6；personality snapshots 50；consolidator 单轮 30 候选；权威分 operator_manual=100>confirmed_tag=20>LLM≤15；consolidation 触发：needed=true ∨ write_score≥6 ∨ pending≥4 ∨ 最老≥6h | memory.rs；domain_profile.rs:86-98 [06] |
| MemoryFact 边界 | text 1..=500 / evidence ≤1000 / confidence & importance 0..=10 / deprecation_reason ≤200 / source_message_ids ≤5；IntentTrajectory MAX_ITEMS=50；Contact.commitments cap 8 | models.rs:5075-5088/5494/210 [02] |
| dynamic_confidence | `base=integrity_score??0.5`；penalty=过期 0.3+悬空锚 0.3；样本<min_samples(5★)→clamp(base−penalty)；否则 `base×0.6+hit_rate×0.4−penalty`；成交追认窗 3 条；rank 降格 superseded×0.1、过期×0.5 | gap_signals.rs:1314-1333/882 [07] |
| TTL 索引 | webhook_rate_limit_windows/import_jobs/import_job_segments/proactive_daily_quotas/knowledge_operator_memory/admin_sessions/auth_security_events=到点即删；knowledge_usage_logs=35d；agent_outcome_metrics=OUTCOME_METRICS_TTL_DAYS 默认 90d；llm_call_logs/agent_run_logs/mcp_call_logs=DIAGNOSTIC_LOG_TTL_DAYS 默认 30d（0=禁） | indexes.rs [02] |

---

## 3. 幂等键与唯一索引总表

### 3.1 键构造规则（✅=本次亲验构造函数）

| 机制 | 键构造 | 唯一索引 |
|---|---|---|
| **outbox 幂等（v2 scoped）** ✅ | `v2:{sha256("outbox-idempotency-v2" ‖ len-prefix(ws) ‖ len-prefix(acct) ‖ len-prefix(legacy_key))hex}`（outbox.rs:458-475 ✅；长度前缀防分隔符歧义）。legacy_key = source_event_id；空则 synthetic（见下） | `uniq_outbox_ws_account_idempotency (ws,acct,idempotency_key) unique`（indexes.rs:247-257 ✅）。**键不含 run_id**（同业务身份跨 run 去重，[15][17]） |
| **synthetic 四形态** ✅ | 名片 `synthetic_namecard:{run_id}:{contact}:{card_id}`；媒体 `synthetic_media:{run_id}:{contact}:{asset_id}`；manual_send `synthetic_manual:{acct}:{contact}:{content_hash}:{day_bucket}`（双击当天去重、次日可重发）；其余 `synthetic:{run_id}:{contact}:{content_hash}`（outbox.rs:543-568 ✅）+ `outbox_synthetic_idempotency_key` warning 事件 | 同上 |
| **文本分段** ✅ | 总段 >1 时每段 `{segment_idempotency_base(source_event_id,run_id)}#seg{idx}`（gateway.rs:4306-4313 ✅；base 空时按 run 隔离防跨 run 撞键 gateway.rs:2339） | 同上。**配额/去重查询不得按带 #seg 的 source_event_id 过滤**（gateway.rs:8094-8100 测试锁） |
| **ack 占位** ✅ | `{source_event_id}#ack-placeholder`、decision_id=None（gateway.rs:5578-5598 ✅） | 同上 |
| **入站消息去重** | `effective_message_id`（顶层 9 键深度递归 `newMsgId/new_msg_id/msgId/msg_id/messageId/id/NewMsgId/MsgId/MessageId` .or(_mcp.sourceMsgId)）→ `message:{id}`；无 id → `payload:{FNV-1a 64bit hex}`（A-03 已知边界：无 id 同内容第二条误判重复） | `conversation_messages (ws,acct,dedupe_key) partial unique $type string`（indexes.rs:810-822）；另 `(ws,acct,message_id)` sparse unique（webhooks.rs:1442-1458）[03] |
| **行为信号** | `bs::build_*` 以 message_id（缺失退化 observed_at 毫秒）为后缀；silence 按 outbound 毫秒 | `uniq_behavior_signals_ws_account_dedupe_key (ws,acct,dedupe_key) partial $type string`（indexes.rs:527-540）[03][10] |
| **事件去重** | 可选携带；rate_limit 事件 `rate_limit:{acct}:{day_bucket}`（day_bucket=epoch_ms/86400000，UTC 天） | `uniq_events_workspace_dedupe_key (ws,dedupe_key) partial unique $type string`（indexes.rs:931-941 ✅）[03] |
| **主动触达 intent** ✅ | `sha256("proactive-follow-up:v1", ws, acct, wxid, segment, subject)` 前 12 字节作 `_id`（proactive_outreach.rs:120-130 ✅）；task+event+quota 三写事务、并发恰一 Emitted | `_id` 天然唯一 [10][15] |
| **主动触达配额** | `_id=hex(sha256("proactive-daily-quota:v1", namespace, …UTC 日桶…))`（proactive_outreach.rs:132+ ✅ 部分）；namespace：strategic_planner（account 桶）/cold_contact/silence_signal（workspace 桶） | `_id` + TTL expires_at [10] |
| **escalation** | 短码 `E`+4 位 base31（无 0/O/1/I/L），碰撞重试 5 | `uniq_principal_escalation_short_code`；pending 槽 `(ws,acct,contact_wxid,category) partial status="pending"`（同类同联系人恰一 pending）[02][05] |
| **管理工具意图** | `intent_key` 稳定 per-command 调用身份（重试/恢复不产生第二个副作用意图） | `uniq_management_tool_intent (ws,acct,intent_key) partial $type string`（models.rs:4090-4093 ✅ 注释）[02][11] |
| **gap signal 去重** | link 类 `link::{from}::{to}` 跨 kind 共享；其余 `{kind}::{normalize_title}`；持久键=sha256 | `uniq_gap_signals_pending_ws_dedup (ws,dedup_key) partial status="pending"`（indexes.rs:180-194）+ `gap_signals_signal_id_unique` [07] |
| **memory 单飞** | 活跃任务键 active_task_key；候选投影键 projection_key | `uniq_memory_active_task_key (ws,acct,contact_wxid,active_task_key) partial $type string`；`uniq_memory_projection_key (ws,acct,contact_wxid,projection_key) partial $type string` [02] |
| **outcome 聚合任务** | (ws,kind,acct,content) 单飞 | `uniq_outcome_aggregation_ws_kind_account_content partial kind="outcome_aggregation"` [02][03] |
| **投影观察** | (entity_type,entity_id,run_id) 恰一 | `uniq_projection_observation_entity_run (ws,entity_type,entity_id,run_id) unique` [02] |
| **send_ledger** | outbox_id 锚（一次投递不可归因两账号） | `uniq_send_ledger_outbox_id partial $type objectId`（indexes.rs:467-474 ✅）[02][15] |
| **逻辑幂等 _id（非索引）** | `AgentOutcomeMetric._id="{ws}:{acct}:{horizon}:{date}"`；`BehaviorSignalMetric._id="{ws}:{date}"`；`knowledge_chat_session_seqs._id="{ws}\|{session_id}"`；`configuration_generations._id="{namespace}\0{ws}"`（NUL 分隔；namespace∈{domain_profile,taxonomy,llm_provider}）；`background_worker_controls._id="worker::{name}"`；`background_worker_leases._id="{kind}::{ws}"`；m052 job_id="crj_m052_{hex}" | [02][09] |

### 3.2 唯一索引全表（unique/partial unique；[02] §5.2 权威，此处按域分组压缩）

- **租户/账号**：wechat_accounts `(ws,account_id)` + `uniq_wechat_accounts_app_id`（app_id 全局唯一，partial $type string）；contacts `(ws,acct,wxid)`；roster_snapshots `(ws,acct)`；products `(ws,product_id)`。
- **消息/任务/事件**：conversation_messages 双索引（见 3.1）；agent_tasks 两 partial（outcome_aggregation、active_task_key）；agent_events dedupe；behavior_signals dedupe；agent_run_logs `run_id` unique；import_job_segments `(job_id,segment_index)`。
- **发送链**：agent_send_outbox（见 3.1）；agent_send_ledger outbox_id；system_incidents `(ws,incident_key)`；agent_principal_escalations 短码 + pending 槽。
- **版本/单指针六套同构**（[16] E 组：append-only 保历史、恰一 current、并发 publish 一胜一 Conflict）：agent_souls `(ws,agent_kind,version)` + published 单指针 partial status="published"；prompt_templates `(ws,prompt_key,version)` + `uniq_prompt_current_pointer` partial current_version=true + `uniq_prompt_artifact_per_proposal` partial $type objectId；operation_domain_configs / operation_state_policies / system_taxonomies 各 `(…,version)` + current 单指针（taxonomy 另有 active identity multikey partial）；domain_profiles `(ws,profile_id,version)` + current + **workspace 级 is_active 单例** `domain_profiles_ws_active_unique (ws) partial is_active=true`；domain_schemas 同型 active 单例；threshold_overrides `(ws,acct,gate_key)` partial current_version=true + per_proposal；llm_provider_configs `(workspaceId,providerId)` + isActive 单例 + isVisionActive 单例（**camelCase 键**）。
- **审核单 pending 槽**：relationship_type_suggestions / suspected_deal_signals `(ws,contact_id) partial status="pending"`（approved 历史不阻塞下一周期）；taxonomy_candidates `(ws,scope,kind,raw_value)` unique。
- **知识**：operation_knowledge_chunks `uniq_kchunks_lesson_promotion_source (ws,provenance.source_doc_id) partial provenance.source="lesson_promotion"`（同 lesson 恰一晋升）；knowledge_gap_signals 双索引；knowledge_daily_reports `(ws,acct,report_date)`；catalog_rebuild_jobs job_id；ingest_sources source_id；lessons_learned `(ws,lesson_id)`；operating_memories `(ws,acct,contact_wxid)`（首触达并发 create 输家回落 find_one 不透传 E11000 [15]）。
- **evolution**：experiments experiment_id；post_release_reviews `(ws,acct,proposal_id) partial protocol_version=1`。
- **admin/评测**：admin_users username；admin_sessions session_id；evaluation_scenarios `(ws,scenario_id)`；reviewer_stats / deal_attribution_stats stat_id；migrations `_id`。
- **大小写陷阱**（历史事故，[02] §6.13）：llm_provider_configs/campaigns/campaign_sends 必须 **camelCase** 键；behavior_signals/contacts 等 **snake_case**；partial filter 只接受 $eq/$exists/$type（$in/$or 会 Error 67 炸 ensure_indexes）。

---

## 4. feature flag 总表

> ✅ = 默认值本次亲验于 config.rs（:7/489-541/542/636-745/770-801）。runtime flag（Mongo）见表尾。

| flag（env） | 默认 | gate 的行为/worker | 打开前置条件 |
|---|---|---|---|
| `EVOLUTION_ENABLED` | **false** ✅（§0-C1） | evolutionary_worker 实质运行（spawn 无条件、函数内检） | env=true 是硬上限，还需 Mongo `evolution_runtime_flags`（enabled+rollout_percent）二级放行 [09][10] |
| `EVOLUTION_AUTO_RELEASE_ENABLED` | false ✅ | threshold 自动 release（代码级 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 恒否决，HC-017） | 双闸 env+workspace flag；当前政策恒人工 release [09][10][17] |
| `STRATEGIC_PLANNER_ENABLED` | false ✅ | strategic_planner worker（**条件 spawn**，main.rs:263-267 ✅） | 显式 env=true 才 spawn [09][10] |
| `STRATEGIC_PLANNER_PRIORITY_ENABLED` | true ✅ | planner 跨联系人优先级排序 | — [09] |
| `COLD_CONTACT_WORKER_ENABLED` | false ✅ | cold_contact_worker（spawn 无条件、函数内检） | env=true [09][10] |
| `SILENCE_SIGNAL_WORKER_ENABLED` | false ✅ | silence_signal_worker（函数内检；且 interval≠0） | env=true；只写信号绝不发消息 [09][10] |
| `KNOWLEDGE_DIGEST_ENABLED` | false ✅ | knowledge_digest_worker 日报合成 | env=true；run_hour 默认 9 [08][09] |
| `INGEST_WORKER_ENABLED` | false ✅ | ingest_worker（**条件 spawn**，main.rs:344-352 ✅） | env=true；P1-6 自动摄取，产物恒 draft+needs_review [09] |
| `KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0` / `CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=0` / `KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=0` | 30/3/600 ✅ | 对应三个 worker（0=停用，间隔即开关） | — [08][09] |
| `JWT_ENABLED` | false ✅ | auth 链额外接受 `Authorization: Bearer`（P1-7） | true 须配齐 JWT_PRIVATE/PUBLIC_KEY_PEM 双 PEM 否则启动 panic [09][12] |
| `WEBHOOK_VERIFY_SIGNATURE` | **true** ✅ | webhook HMAC-SHA256(body, MCP_API_KEY) 验签 X-MCP-Signature | 关闭仅限本地联调；fail-closed 见 §7 [03][09] |
| `REVIEWER_DUAL_ENABLED` | false ✅ | E2 双脑第二 reviewer | true 但缺 REVIEWER_SECOND_PROVIDER_BASE_URL → 启动拒绝 [09] |
| `DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED` | **true** ✅ | P1 换血：chunk 置信用真实用户反应（decision outcome 三态）替代 reviewer 自评 | false 即秒回滚旧口径 [07][09] |
| `PROGRESSIVE_TIER_ENABLED` | **true** ✅ | 渐进三档 prompt tier（kill switch） | — [09] |
| `REACTION_GATEWAY_PARALLEL_ENABLED` | **true** ✅ | reaction 分析与 gateway 并行 | — [09] |
| `BEHAVIOR_SIGNAL_METRICS_ENABLED` | false ✅ | P3 采集健康度日指标 | — [09] |
| `KNOWLEDGE_EXPLORATION_ENABLED` | false ✅ | P4 受控探索（仅 verified 池内） | temperature 默认 1.0 [09] |
| `SESSION_COOKIE_SECURE` | false ✅ | cookie Secure 属性 | 生产必须 true [12] |
| `autonomy_protocol_enabled`（runtime 参数） | true ✅ | 决策 12 协议字段校验 | — [04] |
| `quiet_hours_enabled`（runtime 参数） | true ✅ | 作息门（workspace 开关权威，contact/profile 级已不改行为——[05] 疑点 4） | — [05] |

**Mongo runtime flag**（`evolution_runtime_flags`）：分桶 `DefaultHasher(contact_id) % 100 < rollout_percent.min(100)`；enabled=false 一票否决；文档缺失=不灰度；同 contact 跨 percent 单调稳定；**灰度只影响哪些 contact 的 run 进演化 cohort，不影响生产回复**（runtime_flag.rs:51-101）[10]。`PUT /api/evolution/runtime-flag` 的 updated_by 取自请求体可伪造（[12] 疑点 8）。

**sunset 纪律**：R11.9 新灰度开关必须有 sunset 计划，否则协议违规 [17]。

---

## 5. prompt key 总表

> 36 key 全清单亲验 ✅（prompts.rs grep `key: "`：35 个业务 spec + 1 个 evolution pack）。layer/用途出自 [09] §4.2。**可演化白名单** `EVOLVABLE_PROMPT_KEYS` 5 值 ✅（revision.rs:26-32；prompt_critic.rs:55-61 同内容双定义，[10] 疑点 8）；**演化禁改** `PROMPT_EVOLUTION_FORBIDDEN_KEYS=["evolution_critic_v1"]` ✅（prompts.rs:2508）。全部经 `generate_agent_json` 调用（agent/mod.rs:248+ ✅）。

| key | layer | 生产调用点/状态 | 输出限额 | 可演化 |
|---|---|---|---|---|
| user.initial_profile.system / .task | system_contract / task_template | 初始画像（decision.rs） | — | 否 |
| user.reply.system | system_contract | 回复运行时契约（红线锚×2） | — | **是** |
| user.reply.policy | policy | 模式判定树+5 闸+表达红线 | — | **是** |
| user.reply.fast.task | task_template | **生产决策主链唯一 task**：首发/targeted rewrite/revision 三站点（decision.rs:460/1321） | 8192 ✅ | **是** |
| user.reply.task | task_template | **退役已落地**（2026-08-13 线 B commit 5f96159）：种子**不再种入**、prompt_guard 治理面已收缩、守护测试转靶 fast.task；DB 历史行保留不删（align 只遍历 spec 清单不枚举 DB）。运行时零消费不变 | — | 否 |
| user.projection.system / .task | post_decision_projection | post_decision worker（无发送控制权） | — | 否 |
| user.memory_consolidator.system / .task | memory_consolidator | memory.rs consolidator | — | 否 |
| user.reaction.system / .task | reaction_analysis | reaction.rs | — | 否 |
| user.review.system | review | 独立评审 full 档 | 8192 ✅ | **是** |
| user.review.light.system | review | 轻量评审档 | 3072 ✅ | **是** |
| user.review.product_claim_markers | review_guard | 字符串兜底 guard 词表（JSON；quality 页自述"当前未启用"编辑不影响评审 [14]） | — | 否 |
| `user.review.claim_gate`（**非模板 key**） | — | ClaimGate prompt 为代码内嵌英文常量（review/mod.rs:340-354），此 key 仅作记账/限额名——**有意置于 prompt 治理与演化体系之外**（[09] 疑点 2 主会话核证） | 3072 ✅ | 否（物理不可） |
| knowledge.auto_verify | knowledge_integrity | verify.rs auto-verify | — | 否 |
| eval.user_operation_judge.system | evaluation | shadow 回归 Judge | — | 否 |
| management.plan.system / .policy | system_contract / policy | management.rs 计划/风险分级 | — | 否 |
| management.prompt_redline_review.system | system_contract | 提示词编辑第三闸 judge（禁自改） | — | 否 |
| playbook.generator.system | methodology_generator | playbooks 生成 | — | 否 |
| group.policy / moment.policy | policy | **draft 占位**（Phase 2 域；soul reset 对 draft 只 append 不 publish，[09] 疑点 10） | — | 否 |
| knowledge.chunk.repair.propose / .followup | knowledge_repair | repair.rs 首轮/追问 | — | 否 |
| knowledge.pack.repair.propose | knowledge_repair | 文档级批量修复 | — | 否 |
| knowledge.chat.intent / .draft_chunk / .update_chunk / .clarify | knowledge_chat | chat.rs 4 分路 | — | 否 |
| knowledge.digest.compose / .dispatch / .summarize_logs | knowledge_digest | digest worker/派工 | — | 否 |
| escalation.principal.interpret / escalation.sediment.title | escalation | 领导裁决解读/沉淀标题 | — | 否 |
| evolution_critic_v1 | critic（独立 pack `wechatagent_evolution_pack_v1_2026_05` ✅ prompts.rs:2512） | evolution prompt_critic | — | **禁**（自指悖论） |

**LLM 精确缓存白名单 4 key**（记账名非模板 key）：`knowledge.import.preview / playbook.generator / playbook.optimizer / user.guide.preview`（agent/mod.rs:828-836）[09]。
**prompt 治理三闸**（prompt_guard）：禁词纯函数 → 锚点完整性 → LLM 语义审查（Reject 与 NeedsHumanConfirm 均中止；LLM 不可用→NeedsHumanConfirm 不放水）；`reset-system-pack` 是显式维护动作物理删除重种，非幂等启动覆盖 [09][10][16]。

---

## 6. worker 总表（16 个 supervised worker）

> 名单亲验 ✅ `SUPERVISED_WORKERS`（supervisor.rs:34-51）；spawn 点亲验 ✅（main.rs:217-352，其中 **strategic_planner 与 ingest_worker 是条件 spawn**，其余无条件 spawn + 函数内检 flag）。熔断：panic 退避 1s×2 封顶 30s；60s 窗 5 panic→open；恢复走 `resume_worker_circuit`（open→half_open，操作者限 `SYSTEM_OPERATOR_USERNAMES` 白名单，空=全拒）✅ [09]。

| # | worker | gate | 间隔 | 核心职责 | 主要写集合 |
|---|---|---|---|---|---|
| 1 | task_worker | 常开 | 30s ✅ | follow-up/聚合任务认领执行（claim CAS→gateway）、reclaim、committing 对账 | agent_tasks、agent_outcome_metrics、agent_events [03] |
| 2 | inbound_reply_worker | 常开 | 固定 250ms | 入站积压恢复（durable inbound reply 义务），并发 4 | agent_tasks、conversation_messages(handoff) [03] |
| 3 | import_worker | 常开（inert 空轮询） | 2s ✅ | 知识导入 job 认领→分块抽取→checkpoint→回写 | import_jobs、import_job_segments、operation_knowledge_* [08] |
| 4 | outbox_dispatcher | 常开 | poll 5s（runtime ✅） | outbox 状态机推进：claim→二次安全门→pacing→MCP 发送→回执三分法→重试/终态；run 级聚合反写 | agent_send_outbox、agent_run_logs、agent_send_ledger、conversation_messages、agent_events [05] |
| 5 | post_decision_worker | 常开 | 并发 4 ✅ | 发送后投影（画像/记忆/观察回放），独立预算 32k/1call，contact lease 双层锁 | contacts、operating_memories、memory_candidates、projection_observations、agent_decision_reviews(post_decision_*)、post_decision_contact_leases [06] |
| 6 | media_storage_reconciler | 常开 | 3600s | 内容寻址媒体对账：修 crash 残留、删孤儿、缺对象 fail-close | content_assets、媒体目录 [09] |
| 7 | strategic_planner | `STRATEGIC_PLANNER_ENABLED`（**条件 spawn** ✅） | 600s | 六段主动触达扫描（silent/commitment/stage/calendar/renewal/reactivation）+ block-rate backoff + 优先级 | agent_tasks、agent_events、proactive_daily_quotas、behavior_signals [10] |
| 8 | cold_contact_worker | `COLD_CONTACT_WORKER_ENABLED`（函数内检） | 复用 planner 间隔 max(60) | D3 冷激活：outbound 老于 168h 的 managed 联系人 emit follow_up | 同上（namespace=cold_contact）[10] |
| 9 | silence_signal_worker | `SILENCE_SIGNAL_WORKER_ENABLED` 且 interval≠0 | 600s ✅ | 沉默删失信号（恒 censored=true），只采集不发送 | behavior_signals、proactive_daily_quotas [10] |
| 10 | evolutionary_worker | `EVOLUTION_ENABLED`（env 硬上限）+ Mongo flag | `evolution_tick_seconds.max(60)`（默认 21600 ✅） | cohort→threshold/prompt 候选（各 ≤4/tick）→shadow replay→显著性→awaiting_admin | experiments、proposals、shadow_replays、threshold_overrides(经 release)、post_release_reviews [10] |
| 11 | knowledge_digest_worker | `KNOWLEDGE_DIGEST_ENABLED` | 每天 run_hour=9 ✅ | 日报四路分析+LLM 合成卡片（attempt/current 两代） | knowledge_daily_reports、background_worker_leases [08] |
| 12 | knowledge_task_worker | interval=0 停 | 30s ✅ | knowledge_chat_tasks 认领，按 sessionId 串行执行 plannedSteps（prepare/commit 两阶段），SSE 进度 | knowledge_chat_tasks、knowledge_chat_turns、operation_knowledge_* [08] |
| 13 | catalog_rebuild_worker | interval=0 停 | 3s ✅ | catalog_rebuild_jobs 消费（generation 单调收敛），重建文档 catalog_summary | catalog_rebuild_jobs、operation_knowledge_documents [07] |
| 14 | knowledge_feedback_worker | interval=0 停 | 600s ✅ | Phase F 反馈回路：usage→dynamic_confidence 刷新、reviewer/deal 统计 | operation_knowledge_chunks(usage_stats/dynamic_confidence)、reviewer_stats、deal_attribution_stats [07] |
| 15 | ingest_worker | `INGEST_WORKER_ENABLED`（**条件 spawn** ✅） | 3600s ✅ | RSS/HTML 自动摄取（SSRF 白名单 fail-closed），产物恒 draft+needs_review；failing 阈值 3、disabled 168h | ingest_sources、operation_knowledge_* [07] |
| 16 | management_command_sweeper | 常开 | 60s 硬编码 | 过期执行租约收敛为 execution_unknown（**绝不重放 MCP**）；租约 5min、批 100 | agent_command_runs、agent_tool_calls [10][11] |

非 supervised 的常驻机制：webhook 内联去抖 spawn（裸 tokio::spawn，panic 无 supervisor 痕迹，[03] D10）；escalation 超时扫描与 reconciler 挂在 dispatcher/tasks tick 内 [05]。

---

## 7. 红线机制总表（执行点 file:line）

### 7.1 AI 永不 verify 知识（15 个强制落点，[08] §4.4 权威）

唯一进 `active+verified` 的写点 = **人工 `/verify`**：事务 + OCC 版本绑定（updated_at 不符→`chunk_revision_conflict` 零写）+ D2 证据闸（source_quote 非空 + anchor 自带非空 sourceQuote，`chunk_verify_gate_reason_for`）→ `apply_chunk_revision_with_session(op=Verify, source=Human)`（verify.rs:83-118 ✅）。
强制 `draft+needs_review` 的写点：crud create（crud.rs:708-710）；import-apply 锚定前后各一（import.rs:1549-1556）；ingest fence 前后各一（import.rs:2277-2285）；apply_chunk_integrity 恒 needs_review（mod.rs:1230-1231）；chat apply create/update（chat.rs:2751-2752/2927-2944）；repair applied（repair.rs:769-786；then_verify 直接 400，:676-680）；split 子块/merge 目标清 quote+anchors（wiki_edit.rs:346-350/545-547）；task worker add_chunk/retag 复用 chat 内核（knowledge_task:595-645/1337-1457）；vision prompt 明文禁 verified（import.rs:2058-2064）。
auto-verify 全类型 verified 强制降级：`enforce_verified_needs_human_audit`（verify.rs:681-686 ✅，调用点 :496 ✅；revision source=Rule 留审计）——auto-verify 只是"预审分诊"。
**受控例外**：`ProvenanceSource::PrincipalAuthorized`（chunk_revisions.rs:107-110 ✅）——领导（真人）裁决沉淀可直 verified，验证主体是真人，红线本质未破 [18]。
召回面闭环：catalog/open_chunk verified-only；cite ⊆ opened ⊆ seed；产品声明无 verified 背书→`blocked_unverified_product_claim`（R5.4，gates.rs:874；旁路仅 priced_from_catalog 与 principal_product_exempted）[04][07][16][18]。

### 7.2 无人工接管（闭集 + lint 双层）

- **状态闭集层**：`FORBIDDEN_HUMAN_HANDOFF_VALUES = ["held_for_human","human_required","waiting_for_human","handoff_to_human","manual_takeover"]`（run_envelope.rs:152-158 ✅）——finalReviewStatus/gateway_status 取这些值=协议违规写库阻断；hold_category 同款禁值（types.rs:1520+ ✅）。
- **CI lint 层**：`check-no-human-takeover.{sh,ps1}` 扫 git diff 新增行，目录 `src/agent/ src/routes/ src/evolution/ frontend/src/`，词表 `(human[_ -]?takeover|takeover|hand[_ -]?off|人工接管|人工介入|人工托管|接管|人工)` 大小写不敏感（check-no-human-takeover.sh:26-35 ✅）；豁免：tests 路径（`*/tests/*` 等 4 形态）+ `src/evolution/lint.rs`（自身即禁词词典）（:57-63 ✅）。**"人工"单词极宽**：src 侧任何面向运营的中文文案必须避开（当前未提交改动即有两处命中，[19] 疑点 1）。
- **语义层**：幕后请示（principal channel）≠转人工——客户永远只面对 AI，relay 用 AI 口吻转述；客户可见文案禁词 13+9 词硬断言（principal_decision_channel 测试 [15] §3.8）；evolution 产物文本过 `evolution::lint::FORBIDDEN_WORDS` 运行期黑名单 [10]。
- 前端措辞单一真相源 `lib/reviewLabels.ts`（held_by_ai_policy→"AI 策略主动暂缓"等）[13][14]。

### 7.3 evolution 隔离

- 物理隔离：`src/evolution/` 禁 import gateway/outbox/mcp/tasks/webhooks —— CI `check-evolution-isolation.sh` 静态扫描（子串匹配，grouped import 理论可绕，S-03 Low）[17][18][19]。
- 数据面：只读 7 表、写自己 4 表 + threshold_overrides/prompt_templates（经 release 通道）；shadow replay 零业务副作用（100 次后 outbox size 不变）[16][17]。
- release 纪律：release 永远 admin 手动 + `RELEASE` 确认串；rollback 单方向（恢复 status=active、缺历史行中止事务）；prompt 绝不自动放量；auto-release 被 human-release policy 代码级恒拒（HC-017）[10][16][17]。
- 灰度只影响 cohort 采样，不影响生产回复（§4 runtime flag）[10]。

### 7.4 送达边界（delivery 红线）

- **回执三分法**（outbox_integration 锁 [15]）：业务回执 ok=false→重试不写出站记录；无可信回执/已越远端边界的 HTTP 失败→`delivery_unknown` **禁自动重放**（OutboxStatus 枚举注释 outbox.rs:55-56 ✅"等待离线核验"；名片类无权威 post-hoc 查询，崩溃恢复也停 delivery_unknown）；明确未发出→正常重试至 failed_terminal。
- **Dispatcher-only MCP**：发送类 MCP 调用只允许出现在 dispatcher（CI delivery-protocol job 调用图精确计数 + 12 具名红线测试）[19]。
- **reclaim 幂等门**：reclaimed_in_flight 必须先过幂等核对（权威 chat_search 优先、本地 mcp_call_logs 兜底）再过 pacing 闸——已发过标 sent 不重发 [05][15]。
- **二次安全门**（发送前 fresh 复核，纯函数 outbox.rs:640-670）：not_managed_at_send / contact_cooldown_active / user_stop_requested_after_decision / outbox_stale_30min；豁免仅 principal_escalation/clarification/system_incident 三类内部通知（**manual_send 不豁免本门**——[05] 疑点 1 的已知设计矛盾）。
- **取消语义**：用户 stop 取消同 contact 全部 pending；claim 后未越边界的 stop 赢最后 CAS；已越边界的迟到取消 best-effort 不重放 [15]。
- approved 决策**必先**入 outbox（带幂等键）再 MCP；task 发送 fencing：decision 批次未封口不发送、stale claim 取消、同 claim 授权恰一次 [15][17]。

### 7.5 验签与鉴权 fail-closed

- webhook 验签：`WEBHOOK_VERIFY_SIGNATURE=true` 默认开 ✅；HMAC-SHA256(body, MCP_API_KEY) 比对 X-MCP-Signature；时间戳窗 ±300s 含边界；secret 未配置/缺签名/缺时间戳/格式坏/越窗/不匹配全部显式 Err 变体拒绝（webhooks.rs:2849-2883 ✅）；无 nonce，窗内可重放，靠下游幂等消化（A-04 已知取舍）[03]。
- `SYSTEM_OPERATOR_USERNAMES` 空=全拒（worker 熔断恢复面 fail-closed）✅ [09]。
- workspace 隔离：一切读写 filter 带 workspace_id；`resolve_authorized_workspace` 单闸拒 ACL 外 override；跨租户 404 不泄漏存在性；账号 MCP key 永不回显 [16]。
- 状态机 fail-closed 面：有 config 但状态机空→拒；未知目标态→拒（防幻影态旁路 policy）；缺 current state policy→回复与管理发送都在 outbox/MCP 前拦停（sr072）[15]。

### 7.6 其它硬红线

- **每次发送必经统一网关** `run_user_operation_gateway`（webhook 回复、follow_up、campaign 扇出、管理发送同链），绕过即 bug；请示卡推领导走 logged_call 不走 outbox（不面向客户）[01][18]。
- **operation_state fail-soft**：非法迁移不拦回复（已发出），跳写留旧值 + `agent.operation_state_transition_rejected` 审计；operation_state 从 canonical customer_stage 派生（C2），两字段不漂移 [01][15]。
- **RunBudget 不 5xx**：超预算 `AppError::BudgetExceeded` → 网关降级（local_decision_review/跳过 rewrite），绝不外泄 5xx 给 webhook [01][17]。
- **双层标签**：customer_stage/intent_level/objection_type 必须来自 system_taxonomies；自由发挥进 agent_generated_signals（decision 字段）与 taxonomy_candidates；未审候选**不得阻塞 run**；证据 fail-closed [17][18]。
- **AI 永不直写 outcome**：疑似成交必须 admin approve 才落 verification="staff_confirmed"；审批+成交+审计同事务 [15]。
- **coreFacts 向后兼容**：必须继续反序列化 legacy `Vec<String>`（R11，CI 基线锁）[17][18]。
- **DEFAULT 字节等价**：一切通用化改造对 DEFAULT 销售域行为逐字节等价（快照测试锁）[18]。
- **合并基线**：`cargo test --lib` ≥350/0fail + 4 PBT（state_transition_pbt/memory_card_invariants/wiki_chunk_revision_pbt/llm_retry_jitter）≥33/0fail（check-baseline.{sh,ps1}）；第二道 gate 即 7.2 的 lint [17][19]。
- **管理面**：执行越过副作用边界的死租约→execution_unknown 绝不重放 MCP（hc020）；plan 冻结 planHash+账号 CAS+确认 [11][15]。

---

## 8. 集合-写入方矩阵（79 个集合）

> 写入方为**模块级主要写者**（读者不列）；来源为各记录归纳 + 本次 `rg .collection(` 全量统计 ✅（db/mod.rs 63 个 typed accessor + 按名/const 访问集合全集）。粗体 = 高频核心集合。

### 8.1 主链路（webhook→agent→发送）

| 集合 | 主要写入方 |
|---|---|
| **contacts** | webhooks（upsert 建档/roster 富化）、gateway（apply_agent_updates：stage/state/tags/记忆摘要/频控锚/commitments）、post_decision worker（投影回放）、routes/contacts+shared（手工编辑/guide apply/成交追加 outcome_events）、management 工具、mcp（roster）[01][02][03][06][11] |
| **conversation_messages** | webhooks（入站落库+handoff_status）、outbox_dispatcher（出站 record）、escalation relay、管理发送 [03][05] |
| **agent_tasks** | webhooks（durable 物化/复活/策略重排）、tasks.rs（claim/退避/终态）、gateway（cancel/reschedule/授权）、campaigns（dispatch 扇出）、planner/cold worker（emit）、management（create_follow_up_task）、routes/tasks（cancel/review-now）[03][10][11] |
| **agent_send_outbox** | outbox.rs enqueue（gateway 文本/媒体/名片/ack 占位/过渡、管理发送、escalation 澄清、system_incident、holding_reply）、outbox_dispatcher（状态机全部推进）、admin_outbox（cancel）、outbox cancel_for_decision/contact [01][05][12] |
| **agent_decision_reviews** | gateway（write_decision_review 全终态）、post_decision worker（post_decision_* raw 字段）、routes/reviews（恢复字段）[01][06][11] |
| **agent_run_logs** | gateway/run_envelope（信封+终态）、outbox_dispatcher（outbox_status/performance 反写）[01][05][06] |
| **agent_events** | 全仓 write_event 族：gateway、outbox、webhooks、tasks、各 worker、routes 审计写点 [01]-[12] |
| agent_send_ledger | send_ledger.rs（dispatcher sent 后落账、回扫对账）[05] |
| system_incidents | system_incident.rs（identity upsert + outbox 推送）[06] |
| behavior_signals | webhooks（入站信号）、silence_signal_worker、planner、reaction [03][10] |
| behavior_signal_metrics | 日聚合（BEHAVIOR_SIGNAL_METRICS_ENABLED gate）[09][10] |
| webhook_rate_limit_windows | webhooks.rs 限流窗口 ✅ [03] |
| agent_principal_escalations | escalation（insert pending/resolve/reassign/terminalize）、webhooks（领导回复路由）[05] |
| operating_memories | memory.rs（consolidation 两阶段提交）、gateway（operating_memory_update merge）、routes/contacts+shared（ensure 播种/手工编辑/guide apply）[06][11] |
| memory_candidates | gateway/decision（候选建档）、memory consolidator（置 consolidated）、post_decision [06] |
| projection_observations | gateway ProjectionWriteGuard/upsert、post_decision（const COLLECTION 访问 ✅）[01][06] |
| post_decision_contact_leases | post_decision.rs（contact 级租约 ✅ const）[06] |
| proactive_daily_quotas | proactive_outreach.rs（planner/cold/silence 三 namespace 事务配额）✅ [10] |
| relationship_type_suggestions | gateway（T6 建议 upsert）、admin_relationship_suggestions（approve/reject）[11][12] |
| suspected_deal_signals | gateway（F23 弱信号 upsert）、admin_suspected_deals（approve/reject 事务）[11][12] |

### 8.2 知识子系统

| 集合 | 主要写入方 |
|---|---|
| **operation_knowledge_documents** | routes/knowledge（crud/import/wiki_edit）、ingest、catalog_rebuild worker（catalog_summary_persisted）[07][08] |
| **operation_knowledge_chunks** | 一切经 chunk_revisions harness 控制写：crud、import、chat apply、repair、wiki_edit（patch/split/merge/…）、knowledge_task worker、lessons promote、verify/auto-verify、feedback worker（usage_stats/dynamic_confidence）[07][08] |
| chunk_revisions | chunk_revisions.rs harness（每次控制写恰一条审计行）[07] |
| knowledge_usage_logs | knowledge_router（record_chunk_hit）、auto-verify、chat/ask [07][08] |
| knowledge_gap_signals | gap_signals（结构 lint 9 类）、knowledge_agent（在线 3 类）、admin（dismiss/apply）、sweep [07][08] |
| catalog_rebuild_jobs | wiki_edit/import/chat（入队）、catalog_rebuild worker（消费收敛）[07][08] |
| ingest_sources | sources_meta CRUD、ingest worker（failing/disabled 状态）[07][08] |
| knowledge_chat_turns / knowledge_chat_session_seqs | routes/knowledge/chat、knowledge_task worker（进度回写）[08] |
| knowledge_chat_tasks | chat（派工创建/cancel）、knowledge_task worker（claim/step/终态）[08] |
| knowledge_daily_reports | knowledge_digest worker + digest_regenerate [08] |
| knowledge_operator_memory | chat apply（显式确认写入）、sources_meta（revoke）[08] |
| lessons_learned | reaction/gateway（lesson 沉淀：success/misjudge/blocked 三 pattern）、routes/lessons_learned（promote-to-peer-case）✅ 访问文件定位 [06][12] |
| reviewer_stats / deal_attribution_stats | knowledge_wiki/reviewer_stats.rs、feedback_worker ✅ [07] |
| structural_proposals | structural_proposals.rs（只进不出，KB-06）✅ [07] |
| import_jobs / import_job_segments | routes/knowledge/import（创建/checkpoint ✅ const）、import_worker（claim/终态/TTL）[08] |

### 8.3 配置/版本/策略

| 集合 | 主要写入方 |
|---|---|
| operation_domain_configs | routes/domains（PUT/reset/ask-human-policy）、guide apply、admin_ops_versions（publish/rollout/rollback）、seed [11][12] |
| operation_state_policies | 域 reconcile 重派、admin_ops_versions [11][12] |
| system_taxonomies | 迁移 seed（m028 等）、admin_taxonomies CRUD、admin_ops_versions 版本操作、候选 approve（事务晋升）[02][12] |
| taxonomy_candidates | gateway（决策终稿 upsert 候选）、guide_profile、admin approve/reject [01][11][12] |
| prompt_templates | prompts.rs（ensure_prompt_pack_v2 seed）、prompt_template_versions（publish/append）、evolution release（artifact 行）、routes/prompt_templates [09][10][11] |
| agent_souls | prompts seed、soul_versions、routes/souls [09][11] |
| operation_playbooks | routes/playbooks（含 ensure 默认）、guide apply [11] |
| domain_profiles | guide_profile（generate 草稿）、routes/domain_profiles（CRUD/publish/activate）[11][12] |
| domain_schemas | routes/domain_schemas（CRUD/activate）[12] |
| llm_provider_configs | llm.rs（ensure_default seed/选举）、routes/llm_providers（CRUD/activate/vision）[09][12] |
| configuration_generations | db/config_generation（taxonomy/domain_profile/llm_provider bump，事务内）✅ [02][09] |
| user_operation_guide_previews | routes/guides（preview/apply 状态机）[11] |
| evaluation_scenarios | routes/evaluations [11] |
| products | routes/products [11] |
| campaigns / campaign_sends | routes/campaigns（CRUD/preview/dispatch/reconciler）[11] |
| content_assets | routes/assets+media_assets、media_storage_reconciler [09][11] |
| referral_cards | routes/referral_cards [11] |
| wechat_accounts | routes/accounts（sync/login/mcp-key）、webhooks（默认账号）[03][11] |
| roster_snapshots | mcp.rs（roster 同步）[09] |

### 8.4 evolution / 观测 / 基础设施

| 集合 | 主要写入方 |
|---|---|
| experiments / proposals / shadow_replays | evolution worker（tick：envelope/候选/replay）[10] |
| threshold_overrides / threshold_overrides_audit | evolution release/rollback（routes/evolution 触发）[10][12] |
| post_release_reviews | release 事务内直插（schedule_post_release_review 已死代码，[10] 疑点 1）[10] |
| evolution_runtime_flags | routes/evolution PUT [12] |
| llm_call_logs | agent/mod generate_agent_json、llm.rs（全部 LLM 调用记账）[09] |
| mcp_call_logs | mcp.rs logged_call（写失败静默 `let _=`，[09] 疑点 6）[09] |
| agent_outcome_metrics | tasks.rs outcome_aggregation（逻辑幂等 _id）[03] |
| agent_command_runs / agent_tool_calls | management.rs（plan/confirm/执行）、management_command_sweeper（租约收敛）[11] |
| management_agent_sessions / management_agent_messages | management.rs [11] |
| admin_users / admin_sessions / auth_security_events | auth（bootstrap/login/session/rate_limit 审计 ✅ 访问文件定位）[12] |
| background_worker_controls / background_worker_leases | supervisor（熔断状态）、digest 等 worker（lease）[08][09] |
| migrations | db/migrations::run（m001–m058）[02] |
| operation_knowledge_items | **幽灵/遗留**：typed accessor 已删，仅历史迁移触达（m010/m011/m014），新代码不用 [02] |

> 注意：`agent_generated_signals` 是 **decision JSON 字段**而非集合 ✅（弱信号从字段提取后写入 relationship_type_suggestions / suspected_deal_signals）；`biztest_control` 仅 biz-test 脚本使用不在 src [19]。

---

## 附：亲验清单（本手册写作时逐条 Read/Grep 的源码位置）

- 闭集：models.rs:918-941（task status+断言）/683-690（campaign）/4063-4081（command_run）/4113-4122（tool_call）/4491-4580（escalation 全组）/3503-3505（outbox 注释）；types.rs:708-730（decision 六闭集）/1507-1516（hold）；run_envelope.rs:31-158（source_kind/lifecycle/finalReview/gateway/禁值）；outbox.rs:44-70（OutboxStatus）；chunk_revisions.rs:98-138（ProvenanceSource）；gap_signals.rs:346/443 + knowledge_agent.rs:1932-1985（gap kind 12）。
- 阈值：models.rs:4877-4978（runtime 全默认）；config.rs:7/489-541/542/636-745/746-801（config 默认与 clamp）；agent/mod.rs:233-246（输出限额）；gateway.rs:4325（max_attempts=3）。
- 幂等：outbox.rs:458-481/536-568（v2 键+synthetic 四形态）；gateway.rs:4300-4326/5578-5598（seg/ack）；indexes.rs:247-257/467-474/931-941（三唯一索引）；proactive_outreach.rs:120-136（intent/quota 键）。
- flag：config.rs:7+889-892（EVOLUTION false+测试）及上列 config 段 16 项。
- prompt：prompts.rs 36 key grep + :2505-2512（FORBIDDEN+pack 版本）；revision.rs:26-32（EVOLVABLE）；agent/mod.rs:230-246（retired full task 注释+限额）。
- worker：supervisor.rs:28-51（熔断参数+16 名单）；main.rs:213-352（16 spawn 点，2 个条件 spawn）。
- 红线：verify.rs:83-118/479-508/674-686（人工 verify 唯一入口+auto-verify 强制降级）；outbox.rs:55-56（delivery_unknown）；webhooks.rs:2849-2883（验签 fail-closed）；check-no-human-takeover.sh:26-63（lint 词表/目录/豁免）；run_envelope.rs:152-158（禁值）。
- 集合：db/mod.rs 63 typed accessor grep；全 src `.collection("…")` 按名统计；lessons_learned/reviewer_stats/deal_attribution_stats/structural_proposals/proactive_daily_quotas/relationship_type_suggestions/auth_security_events/webhook_rate_limit_windows/configuration_generations/post_decision_contact_leases 访问文件定位；projection_observations/IMPORT_CHECKPOINT/CONTACT_LEASE 三 const 解析；agent_generated_signals 判定为字段。

---

## 追记：chunk 声明字段族的跨链路口径分叉（S5-7 裁决，2026-08-13 主会话）

**裁决：维持分叉、显式化记录，不动行为**（用户批复"做"后的方向选定：真正统一需产品层决定 chat 链是否也收敛 verified-only 语义，属更大工程；激进删活字段或给主链复活废字段均高风险低收益）。

分叉现状（线 B B8 重验 + 25 号修正 6）：

| 字段族 | user-ops 主链（decision/review/finalize） | chat/repair/catalog 链 |
|---|---|---|
| `safe_claims` / `forbidden_claims` / `evidence_items` / `routing_card` | **不消费**（2026-05-25 知识清理后主链改分数闸+R5.4 结构化背书） | **活跃**：chat/repair prompt 仍要求输出、`catalog.rs:535` 生产查询 `evidence_items` |

使用纪律：改知识 chunk 模型或导入/修复 prompt 时，这些字段按"chat 链活字段"对待，删除前必须核 chat/repair/catalog 三处消费；主链侧永远不要为它们加消费逻辑（已被 R5.4 取代）。`models.rs` 内注释补写待 E 线合并后执行（该文件在途）。
