# 数据层交叉验证（核证日期 2026-08-13）

> 任务：审计 02 号（models-db）与全部引用它的记录（01/03/05/06/07/08/10/11）之间的一致性。
> 方法：先通读 02 号全文（882 行）与 8 份引用方记录全文，提取每一处对同一数据对象的双方断言；凡有出入者当场用 Read/Grep 亲读 `src/models.rs`（8352 行版）、`src/db/indexes.rs`（3444 行版）、`src/db/migrations/**` 及相关业务写点源码裁决。**本文所有"裁决证据"栏的 file:line 均为本次亲读，不转引任何记录。**
> 行号基准：2026-08-13 工作树（含未提交改动），与 02 号同基准。

---

## 1. 模型断言交叉审计表

判定用语：**一致**（双方与源码三方吻合）；**02 误**（02 号与源码不符）；**引用方误**（引用方与源码不符）；**口径不同**（双方各自准确、描述层面不同，需澄清防误读）；**注释滞后**（双方都如实转写了模型注释，但注释本身已落后于实现——按任务红线以实现为准裁决）。

### 1.1 AgentTask 状态闭集与 claim 字段 ↔ 03 号

| # | 断言 | 02 号说法 | 03 号说法 | 判定 | 裁决证据（亲读） |
|---|---|---|---|---|---|
| 1 | status 闭集 | 9 态含 `committing`（L918-928） | 同，"亲验 models.rs:918-928" | **一致** | models.rs:918-928 逐值核对：pending/running/committing/retry/failed/cancelled/sent/completed/outbox_enqueued，9 值无误 |
| 2 | doc 注释/单测滞后 | 注释（L913-917）与单测（L948-963）只列 8 值、缺 committing（§6 疑点 3） | 未提 | **一致（02 疑点属实）** | models.rs:913-917 注释确无 committing；测试 closed_set_covers_all_known_writers（947-963）列表确为 8 值 |
| 3 | claim 字段 | DTO 含 `claimed_at`/`claim_recovery_count`（均 default），无 claim_token | "claim_token 不在 AgentTask DTO"（tasks.rs:810-812 注释转引） | **一致** | models.rs:878-907 逐字段核对，struct 无 claim_token；claimed_at:901-902、claim_recovery_count:903-904 均 `#[serde(default)]` |
| 4 | `completed` 语义 | 闭集含 completed | "completed 在本两文件无写点，保留为 R10 reset alias（models.rs:916）" | **一致** | models.rs:916 注释原文"`completed`：保留为 R10 reset 一致 alias" |
| 5 | 断言函数行为 | L932-941 debug panic / release error | 03 号 4.1 同 | **一致** | models.rs:932-941：debug_assert + tracing::error!（release 下仅记日志，**并不物理拒写**——两记录"拒绝写入"措辞略强于实现，注释自称"拒绝写入"但函数体无返回值，写入是否继续取决于调用方；属注释措辞问题，双方等价转写，不另立矛盾） |

### 1.2 OutboxEntry ↔ 05 号

| # | 断言 | 02 号说法 | 05 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | status 闭集 6 态 | L3503-3505 注明 pending/in_flight/sent/failed_terminal/canceled/delivery_unknown | OutboxStatus enum（outbox.rs:43-85）同 6 值，as_str 唯一字面量源 | **一致** | models.rs:3503-3505 注释逐字核对 6 值 |
| 2 | 幂等键 | (ws,acct,idempotency_key) unique，m038 重写 scoped 形态 | v2 key=`sha256("outbox-idempotency-v2"‖len-prefix…)`，tenant-scoped unique | **一致** | indexes.rs:246-260 `uniq_outbox_ws_account_idempotency` 三键 unique；m038:78-94 亲读（空/前后空白 fail-closed 炸启动） |
| 3 | 字段全集 | §2.21 列 30 字段 | EnqueueRequest/claim 协议/取消两段式与之吻合 | **一致** | models.rs:3506-3582 逐字段核对：全部命中，无遗漏无多列 |
| 4 | delivery_priority/run_sequence | "只影响领取顺序不参与幂等/授权"（L3520-3523）；未列数值 | 数值表 manual 100 > inbound 90 > principal 80 > follow_up 60 > incident 40 > 其它 50 > 媒体/名片 20 | **一致（互补）** | models.rs:3520-3527 注释与 02 转写一致；数值在 outbox.rs（05 号范围），02 不列不算缺 |
| 5 | delivery_finalize_pending | raw 字段，struct 未声明，outbox_dispatcher.rs 写 | commit_sent_if_owned 写 `sent + delivery_finalize_pending=(decision_id.is_some && 纯文本)` | **一致** | models.rs:3506-3582 确无此字段；outbox_dispatcher.rs:763-765 写点亲读（条件=decision_id.is_some && media_asset_id.is_none && referral_card_id.is_none，即"纯文本"） |

### 1.3 AgentRunLog / lifecycle / gateway_status ↔ 01 号

| # | 断言 | 02 号说法 | 01 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | lifecycle 7 态 | L3418-3422 注释 7 值 | run_envelope.rs:48-54 常量 7 值 | **一致** | models.rs:3418-3424 注释与 run_envelope.rs:48-54 常量逐值核对，两处同 7 值 |
| 2 | source_kind 闭集 | **3 值**（inbound_message/follow_up_task/manual_send，L3429-3431） | **6 值**（run_envelope.rs:34-45，另含 principal_escalation/principal_clarification/system_incident） | **口径不同（双方均准）** | models.rs:3429-3432 注释确为 3 值；run_envelope.rs:31-45 亲读：注释明说"允许三选一"，随后定义 6 常量，后 3 个注明是 internal notification。`write_run_envelope_started` 全仓仅 2 个调用点（gateway.rs:371 manual_send、gateway.rs:1757 inbound/follow_up）——**agent_run_logs.source_kind 实际写入面 = 3 值，模型注释准确**；后 3 常量只流入 `agent_send_outbox.source_kind`（05 号语境）。02 宜补注一句防止读者以为全系统 source_kind 只有 3 值 |
| 3 | outbox_status 闭集 | 5 值（pending/in_flight/sent/failed_terminal/canceled，L3472-3475 注释） | 05 号：run 级聚合另产 `partially_sent`，delivery_unknown 也会写回 | **注释滞后（02 需回写）** | models.rs:3472-3476 注释确为 5 值；但 outbox_dispatcher.rs:980-1023 亲读：`aggregate_run_outbox_status` 可返回 `"partially_sent"`（:1008）与 `DeliveryUnknown.as_str()`（:1016-1021），经 refresh 写回 run log。**实际值域 7 值，模型注释与 02 §2.20/§5.1 事实卡均滞后** |
| 4 | gateway_status | 02 引 spec R9 未列全 | 40 值闭集（run_envelope.rs:87-147） | **一致（不冲突）** | 02 未做闭集断言，无矛盾；01/06 的 40 值口径已由其主会话抽查背书，本次不重复展开 |
| 5 | run_id unique | §3.3 "run_id unique" | 01 号写集合表同 | **一致** | indexes.rs（agent_run_logs 段）已在 02 主会话抽查覆盖，本次抽验未复核该单条（见 §3 范围说明） |

### 1.4 MemoryCardTyped / MemoryFactRepr / OperatingMemory ↔ 06 号

| # | 断言 | 02 号说法 | 06 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | typed 三数组 cap | 写侧 cap 6/10/20（L4985-4987） | core 6（memory.rs:440-444）/recent 10（:529）/deprecated 20（:531） | **一致** | models.rs:4985-4987 注释"各自有写入侧 cap（6 / 10 / 20…）"亲读吻合 |
| 2 | extra flatten | catch-all 承接全部未声明顶层键；曾有重复 BSON 键 bug | 同（06 引 compact 行为） | **一致** | models.rs:5003-5019 struct + 4991-4995 注释亲读吻合 |
| 3 | extra 镜像 deprecatedFacts cap=6 | 02 未提 | 06 疑点 2：镜像 6 ≠ typed 20（memory.rs:580-582） | **一致（互补非矛盾）** | 两者描述不同层：02 写 typed 层 cap（正确），06 写 memory.rs extra 镜像层行为。不构成 02 的错误，但 02 §5.1 cap 表可补一行防误读 |
| 4 | Plain 升级参数 | fresh UUID + conf 7 + imp 5 | 同（memory.rs / helpers.rs 两处） | **一致** | models.rs:5177-5248 段（02 主会话已抽查）+ 06 号 memory.rs:1745-1746 互证，本次未再复读 |
| 5 | OperatingMemory 字段与 OCC | (ws,acct,contact_wxid) unique；四 Document 槽+双版本号 | occ_memory_filter 四键含 memory_card_version | **一致** | models.rs:1668-1695 逐字段核对（含 memory_card: MemoryCardTyped default、memory_card_version）；索引 (ws,acct,contact_wxid) unique 见 02 §3.3（indexes.rs:1081-1089，本次未复读该单条） |
| 6 | **active_task_key 归属** | §2.14/§6.2：**operating_memories** 的 raw 动态字段（memory.rs 写） | 06 号：schedule_memory_consolidation_task 把 active_task_key 写进 **task 行**（agent_tasks） | **02 误（06 正确）** | 铁证三条：① indexes.rs:859-865 `db.tasks().create_index(memory_active_task_key_index(), ...)`——索引建在 **agent_tasks**；② 全仓 grep `active_task_key` 写点全部落在 agent_tasks（webhooks.rs:136/189 inbound_reply、memory.rs:3092/3114/3166 等 memory_consolidation、contacts.rs:1530/1550 等 initial_profile、tasks.rs:741/875 unset）；③ operating_memories 无任何写点。**02 号自己的 §3.3/§5.2 把该索引正确列在 agent_tasks 下，与 §2.14/§6.2 自相矛盾——§2.14/§6.2 为误** |

### 1.5 OperationKnowledgeChunk / Document ↔ 07/08 号

| # | 断言 | 02 号说法 | 07/08 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | wiki_type/chunk_type 闭集 | 9 类 / 4 类（L1865-1883），缺省 product_fact | 同 | **一致** | models.rs:1865-1875（9 值）、1878-1883（4 值）、1857-1861（default_chunk_type）逐值核对 |
| 2 | B3 可引用性谓词 | anchor 含非空 `sourceQuote`（camelCase 键）；chunk_has_citable_anchor 任一可引用 | 07/08 多处引用同口径（含 4 处裸 !is_empty() 偏差） | **一致** | models.rs:1915-1928 亲读：`get_str("sourceQuote")` 非空 trim；08 号的裸判定偏差在读点层，与模型谓词定义不冲突 |
| 3 | **locked_fields 默认项数** | "默认 7 项"（照抄 models.rs:1837 注释） | 07 号：DEFAULT_LOCKED_FIELDS **8 个**（page_merge.rs:35-44） | **注释滞后（02 需回写；07 正确）** | page_merge.rs:35-44 亲数：_id/workspace_id/account_id/document_id/item_id/wiki_type/chunk_type/created_at = **8 项**；models.rs:1837 注释"默认 7 项"与实现不符。02 转写注释未核实现 |
| 4 | **provenance.source 闭集** | 注释 {ai,human,rule,imported}，§6.6 补 m055 的 lesson_promotion（共 5） | 07 号：ProvenanceSource 枚举 **5 值含 principal_authorized**（chunk_revisions.rs:97-139）；05 号：PrincipalAuthorized revision | **02 不完整（07/05 正确）** | chunk_revisions.rs:98-138 亲读：枚举 5 值，as_str/FromStr 均含 "principal_authorized"；ledger.rs:714-733 亲读：请示知识沉淀以 PrincipalAuthorized 建 Create revision（chunk 无既有 provenance 时以该 source 初始化）。**持久化 source 实际值域 ≥6 种（4 注释值 + principal_authorized + lesson_promotion），models.rs:1993 注释过时，02 §6.6 只补了一半** |
| 5 | dynamic_confidence 公式 | "base×0.6+hit_rate×0.4−stale_penalty clamp[0,1]"（L1830 注释） | 07 号：完整公式含 min_samples 小样本分支 + dangling penalty 0.3 叠加（gap_signals.rs:1314-1333） | **注释滞后（轻微；07 权威）** | models.rs:1830-1833 注释确为简化式；07 号公式为实现权威。02 引注释未标注"简化版" |
| 6 | Document catalog 四字段 | persisted/version/desired_generation/applied_generation | 07（finalize CAS desired==target）/08（catalogFresh=persisted∧desired>0∧applied==desired） | **一致** | 02 主会话已抽查 L1749-1762；本次经 m052 侧面核证（见 §2 第 6 行），未再复读 |
| 7 | m052 marker 字段 | `catalog_m052_reconciliation_generation`（m052:21） | 07 号未提（范围外） | **一致** | m052_catalog_rebuild_leases.rs:21 亲读：常量定义逐字命中，行号精确 |

### 1.6 AskHumanPolicy / AgentPrincipalEscalation ↔ 05 号

| # | 断言 | 02 号说法 | 05 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | AskHumanPolicy 四开关默认 | safety_guard/unverified=true、ai_policy_hold=false、stuck=true（serde default） | resolve 回落默认同为 true/true/仅all/true（policy.rs） | **一致（两层同向）** | models.rs:1429-1449 亲读：`default_true`×3（safety/unverified/stuck）+ `#[serde(default)]`（ai_policy_hold=false）。05 描述的是 policy.rs 的 None-回落层，缺省方向与模型 serde 层一致 |
| 2 | 状态闭集三组 | escalation 3 / relay 3 / card delivery 5（L4491-4508） | 同（models.rs:4491-4508 转引） | **一致** | models.rs:4491-4508 亲读逐值核对（pending/resolved/delivery_failed；pending/enqueued/terminal；pending_enqueue/queued/sent/failed_terminal/delivery_unknown） |
| 3 | Protocol 冻结结构 | 9 字段含 delivery_generation(default)/failure_cleanup_completed_at | 初始 generation=1、pending_enqueue，failure cleanup 先清后写 | **一致** | models.rs:4514-4530 逐字段核对；初始值 1 是 ledger.rs 写点行为（05 号范围），与模型 default 0 不冲突（insert 时显式赋 1） |
| 4 | 台账字段 | short_code 全局唯一、last_pushed_at_ms（m031 回填=created_at）、relay 四字段等 | KD-05 骚扰门以 last_pushed_at_ms 为口径、reassign 刷新 | **一致** | models.rs:4634-4705 逐字段核对：21 个字段全部命中 02 §2.12 清单；4675-4678 注释与两记录的 KD-05 语义吻合 |
| 5 | verdict/exemption/category 闭集 | 5/3/3（L4533-4580） | 同 | **一致** | models.rs:4532-4534 亲读（category 前 2 常量在读段内）；其余在 02 主会话抽查范围，本次不重复 |

### 1.7 Campaign / ReferralCard / ContentAsset ↔ 11 号

| # | 断言 | 02 号说法 | 11 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | Campaign status 闭集 | 6 值 draft/previewed/confirmed/dispatching/completed/canceled（L683-690） | 生命周期只走 draft/previewed/dispatching/completed；dispatch 可入态 3 种 | **一致（视角不同）** | models.rs:683-690 亲读 6 值无误；11 号是写点/生命周期视角，闭集 ⊇ 实际使用集，不冲突（confirmed/canceled 当前无写点属另一层事实，双方均未断言错误） |
| 2 | **CampaignSend.status** | 闭集 prepared/enqueued（L674 注释级） | §4.2 列 "prepared / enqueued / **skipped_duplicate**（classify 读到）" | **02 正确；11 表述失真** | models.rs:674 注释亲读仅 2 值；campaigns.rs 全部写点亲查：insert `status:"prepared"`（:706）、CAS `$set status:"enqueued"`（:789），**无任何 skipped_duplicate 写点**；campaigns.rs:958 的 `send_status=="skipped_duplicate"` 分支是 classify 的防御输入分支（当前不可达）。11 号把它列进 status 值域会误导 |
| 3 | camelCase 落库 | Campaign/CampaignSend camelCase BSON；索引键必须 camel | §2.5 末注同 + "跨集合易拼错 key" | **一致** | models.rs:659 `rename_all="camelCase"`（CampaignSend）；indexes.rs:2048-2077 亲读：注释明说 camel 红线，(campaignId, contactWxid) unique |
| 4 | ReferralCard | snake_case；必须 approved+enabled 才可选 | create 恒 draft+enabled=false；validate_card_sendable 同口径（05 号） | **一致** | models.rs:1451-1478 亲读：无 rename_all（snake），注释"必须人类标 review_status=approved 且 enabled=true 才可被选"；字段清单与 02 §1.4 吻合 |
| 5 | ContentAsset | 文件六字段/发送标注五字段/min_inject_tier None=full | 11 号 media_assets 行为（draft 强制/换文件清 media_id）与字段对应 | **一致** | 02 主会话未抽查本条，本次核对 §2.10 与 11 号 §2.17 逐字段无冲突；模型体（L1249-1306）属 02 通读范围，抽验预算分配给矛盾点，未再复读（见 §5 残余风险） |

### 1.8 proposals / experiments / threshold_overrides ↔ 10 号

| # | 断言 | 02 号说法 | 10 号说法 | 判定 | 裁决证据 |
|---|---|---|---|---|---|
| 1 | Experiment.status 闭集 | 5 值（L5635） | envelope.rs:60-63 同 5 值；tick 实际只走 collecting→awaiting_admin | **一致** | models.rs:5635 注释亲读 5 值逐字命中 |
| 2 | Proposal.status 闭集 | 6 值含 evaluating（L5666-5673）；prompt eligible=证据就绪待管理员 | 写点清单 5 值（evaluating 合法但无写点） | **一致（互补）** | models.rs:5666-5673 亲读：6 值 + "prompt 不自动放行"注释与两记录吻合 |
| 3 | Proposal 字段 | base_revision（legacy 不可 release）/released_revision/previous_prompt_version/eval_* | release.rs OCC 用法一致 | **一致** | models.rs:5694-5711 逐字段核对 |
| 4 | ShadowReplay 字段 | 含 original_5gate_hit（G4）与 similarity_to_original_text | ReplayOutcome 无 similarity（10 号未列） | **一致（互补）** | models.rs:5722-5755 亲读：similarity 字段存在且注释"W3 task 4.1 写 0.0 占位"——10 号不列非矛盾 |
| 5 | ThresholdOverride/Audit | current_version partial unique；audit action 3 种/decided_by 4 形态 | release/rollback 事务行为一致 | **一致** | models.rs:5760-5782、5791-5816 亲读逐值核对 |
| 6 | **PostReleaseReview 字段** | 模型 4 业务字段：scheduled_at/completed/actual_send_success_rate_delta/actual_5gate_hit_delta | 10 号：process_one_review 写"**三** delta"（含负反应 delta） | **双方均准；暴露 02 raw 清单漏项** | models.rs:5820-5837 亲读：typed 模型确无第三 delta；post_release.rs:232-236 亲读：`$set` 同时写 `actual_negative_reaction_rate_delta`——**这是一个 typed 未声明的 raw 写入字段，02 §6.2 清单未收**（详见 §2 表第 7 行） |

---

## 2. raw Document 字段全表（索引-写点-拼写三方核验）

02 号 §6.2 的核心结构性断言："多个带索引字段不在 typed 模型、由 raw Document 写入"。逐字段亲验结果：

| # | 字段 | 实际集合 | 索引（亲读 indexes.rs） | 写入点（亲读/grep） | 读取点（代表） | 拼写核验 |
|---|---|---|---|---|---|---|
| 1 | `delivery_finalize_pending` | agent_send_outbox | `outbox_delivery_finalize_pending_idx`：keys (status, delivery_finalize_pending, updated_at, _id)，partial {status:"sent", delivery_finalize_pending:true}（indexes.rs:196-214；建于 collection_agent_send_outbox，:2126-2128） | outbox_dispatcher.rs:763（commit_sent_if_owned `$set`）；:1275（clear `$unset`） | outbox_dispatcher.rs:1272（decision 维度扫描）、:1743（reconcile 扫描 filter） | ✅ 逐字一致（索引/写/读三方均 `delivery_finalize_pending`） |
| 2 | `post_decision_status` / `post_decision_next_retry_at` / `post_decision_locked_until` / `post_decision_scrub_at` / `post_decision_profile_done` | agent_decision_reviews | 四条：`decision_post_projection_idx`（indexes.rs:2151-2168）、`decision_post_projection_claim_v2_idx`（:2172-2190，前缀加 status）、`decision_post_projection_scrub_idx`（:2194-2208，刻意非 TTL）、`decision_post_projection_order_fence_idx`（:2212-2230，(ws,acct,wxid,profile_done,created_at↓,_id↓)） | post_decision.rs:198（prepared）、:226/:230（failed_terminal+scrub_at）、:378/:788（processing+locked_until）、:594-595（pending+next_retry_at）、:893/:1149（profile_done）、:1193（completed）、:1265-1266（retry+next_retry_at）等 | post_decision.rs:624-637（runnable_filter）；routes/reviews.rs（恢复端点，12 处）；routes/observability.rs（4 处只读） | ✅ 五字段在索引与全部写/读点逐字一致 |
| 3 | `handoff_status` | conversation_messages | `inbound_handoff_pending_idx`：keys (handoff_status, created_at, _id)，partial {direction:"inbound", handoff_status:"pending"}（indexes.rs:379-392） | webhooks.rs:1488（insert 同批写 "pending"）、:67（mark CAS `$set`） | webhooks.rs:65（CAS filter $in [pending,deferred]）、:811（恢复扫描） | ✅ 逐字一致 |
| 4 | `active_task_key` | **agent_tasks**（02 §2.14/§6.2 误记为 operating_memories） | `uniq_memory_active_task_key`：keys (workspace_id, account_id, contact_wxid, active_task_key)，partial unique {$type:"string"}（indexes.rs:359-375）；**建于 `db.tasks()`**（:863-865，注释"Terminal transitions remove active_task_key atomically"） | webhooks.rs:136/:189（inbound_reply 值 `DURABLE_INBOUND_ACTIVE_KEY`）；agent/memory.rs:3092/:3114/:3166（值 "memory_consolidation"）；routes/contacts.rs:1530/:1550（值 INITIAL_PROFILE_ACTIVE_KEY）；终态 `$unset`：tasks.rs:741/:875、memory.rs:1675/:2192/:2272/:2328、contacts.rs:925/:1172/:1291/:1424/:2172、webhooks.rs:734、routes/tasks.rs:337 | memory.rs:2965/:2995/:3019/:3049（单飞瀑布 filter）；contacts.rs:1236/:1279/:1550/:2161（enrollment filter）；tasks.rs:1031（revive filter） | ✅ 拼写逐字一致（含索引 partial filter）；**归属集合 02 需回写** |
| 5 | `projection_key` | memory_candidates | `uniq_memory_projection_key`：keys (workspace_id, account_id, contact_wxid, projection_key)，partial unique {$type:"string"}（indexes.rs:1294-1322 段，keys/name/filter 亲读 :1306-1317） | agent/memory.rs:2659（`insert.insert("projection_key", …)`，upsert `$setOnInsert`，集合 `db.memory_candidates()` 亲证 :2662） | memory.rs:2665-2670（upsert 四键 filter） | ✅ 逐字一致 |
| 6 | `catalog_m052_reconciliation_generation` | operation_knowledge_documents | 无索引（一次性 reconciliation marker，02 亦未声称有索引） | m052_catalog_rebuild_leases.rs:21（常量 RECONCILIATION_GENERATION_FIELD 定义；CAS 分配写点在同文件） | m052 内部 CAS 回读 | ✅ 常量单点定义，无拼写分叉面；02 引 "m052:21" 行号精确 |
| 7 | **（02 清单遗漏）** `actual_negative_reaction_rate_delta` | post_release_reviews | 无索引 | post_release.rs:236（process_one_review `$set`；模型 models.rs:5820-5837 无此字段） | 同文件事件 details（:259）；admin 面板读 | ✅ 拼写单点一致；**建议补入 02 §6.2** |
| 8 | **（02 清单遗漏·伴生字段）** `handoff_updated_at`（conversation_messages，webhooks.rs:68 写）；`delivery_finalized_at`（agent_send_outbox，outbox_dispatcher.rs:1274 写）；agent_tasks 协议字段族（`claim_token`/`prepared_commit_kind`/`prepared_commit`/`outbox_decision_id`/`rerun_requested`/`latest_inbound_id`/`latest_inbound_created_at`/`obligation_started_*`/`covered_through_*`/`manual_reply_*`/`enrollment_token`/`allow_contact_insert`/`manual_requested_by`/`proactive_intent_hash`）；decision_reviews 的 `post_decision_claim_token`/`post_decision_payload`/`post_decision_error`/`post_decision_projection_result` 等 | 各对应集合 | 无独立索引（proactive_intent_hash 等参与业务身份校验但非索引键） | 各业务文件（03/05/06/10/11 号已详述） | — | 02 §6.2 的口径是"**有索引支撑**的 raw 字段"，按此口径原清单本身成立；但未明示口径边界，且 §6.2 结语"切勿以为 models.rs 是字段全集"应补充：**无索引的 raw 协议字段数量远大于清单所列**（agent_tasks 尤甚） |

**结论**：02 §6.2 六条字段的"索引存在性、写入方文件、拼写一致性"全部核验通过，唯一实质错误是第 4 行的**集合归属**（operating_memories → 应为 agent_tasks）；另有第 7/8 行两类补充项。

---

## 3. 锚点抽验结果

从 02 号抽取 25 个 file:line 锚点（模型 13 / 索引 8 / 迁移 4，超出任务要求的 20 个），全部当场亲读：

| # | 锚点（02 号引用） | 02 号断言 | 亲读结果 |
|---|---|---|---|
| 1 | models.rs:918-928 | ALLOWED_AGENT_TASK_STATUS 9 值含 committing | ✅ 精确命中 |
| 2 | models.rs:913-917 | doc 注释历史清单仅 8 值 | ✅ 命中（无 committing） |
| 3 | models.rs:948-963 | 单测 closed_set_covers_all_known_writers 仅列 8 值 | ✅ 命中（测试体 947-963，列表 951-960） |
| 4 | models.rs:683-690 | ALLOWED_CAMPAIGN_STATUS 6 值 | ✅ 精确命中 |
| 5 | models.rs:1052 | ALLOWED_IMPORT_JOB_STATUS 4 值 | ✅ 精确命中 |
| 6 | models.rs:1865-1875 / 1878-1883 | wiki 9 值 / chunk_type 4 值 | ✅ 精确命中 |
| 7 | models.rs:1903-1928 | anchor_is_citable（sourceQuote 非空）/ chunk_has_citable_anchor | ✅ 命中（函数体 1915-1928） |
| 8 | models.rs:3418-3422 / 3429-3431 | lifecycle 7 值 / source_kind 3 值注释 | ✅ 命中（注释转写准确；source_kind 口径澄清见 §1.3-2） |
| 9 | models.rs:3472-3475 | outbox_status 注释 5 值 | ✅ 命中（注释本身滞后于实现，见 §1.3-3） |
| 10 | models.rs:3503-3505 | OutboxEntry status 6 值注释 | ✅ 精确命中 |
| 11 | models.rs:4491-4508 | 请示三组状态常量（3/3/5） | ✅ 精确命中 |
| 12 | models.rs:5934-5935 | ALLOWED_TASK_STATUS 5 值（finished→completed） | ✅ 精确命中 |
| 13 | models.rs:841 | PRINCIPAL_RELAY_SENTINEL = `__PRINCIPAL_RELAY__` | ✅ 精确命中 |
| 14 | models.rs:1802 | 注释引用幽灵迁移 `2026_05_W1_001_chunks_wiki_type_default` | ✅ 命中；grep 全仓仅此 1 处出现，MIGRATIONS 注册表无此 id（02 疑点 12 属实） |
| 15 | indexes.rs:751-759 | contacts (ws,acct,wxid) unique | ✅ 精确命中 |
| 16 | indexes.rs:760-775 | `outcome_events.productRef.productId` 混合大小写 multikey | ✅ 精确命中（注释 760-763 与 02 转写一致） |
| 17 | indexes.rs:2069-2077 | campaign_sends (campaignId, contactWxid) unique（camel） | ✅ 精确命中（含 2053-2056 camel 红线注释） |
| 18 | indexes.rs:1967-1977 | llm_provider 开机 drop 两条 snake 历史错索引 | ✅ 精确命中 |
| 19 | indexes.rs:379-392 | inbound_handoff_pending_idx 名称与 partial filter | ✅ 精确命中 |
| 20 | indexes.rs:246-260 | uniq_outbox_ws_account_idempotency 三键 unique | ✅ 精确命中 |
| 21 | migrations/mod.rs:305-538 注册表 | 58 条迁移 | ✅ `Migration {` 计数 = 58 |
| 22 | migrations/mod.rs:545-556 / 558-566 / 571-579 | 审批闸两 gate id 及其守卫集合 / env 切分符 / 生产 fail-closed 判定 | ✅ 三段精确命中（gate 守 {V3_002,V3_003,W4_002,自身} 与 {X1_001,自身}；切分 `,;空格\n\t`；仅 development/dev/test/local 豁免） |
| 23 | m030:47-56 | 过滤器逐字段隔离、绝不共享 $or（duplicate_field 崩溃论证） | ✅ 精确命中（backfill_filter + 注释；其注释内引 models.rs:248 已漂移、alias 实际在 models.rs:251——02 §6.5 的"注释行号过期"判断亲证属实） |
| 24 | m038:78-94 | 字段缺失/空/前后空白 fail-closed 炸启动 | ✅ 精确命中（required_non_empty 两个 External 分支） |
| 25 | db/mod.rs:111 / 281 | tasks→agent_tasks accessor / collection_agent_send_outbox accessor | ✅ 两处行号精确命中 |

**通过率：25/25（100%）**。其中 #8/#9 两条的"内容"是模型注释——锚点与转写均准确，但注释本身滞后于实现（已在 §1 单列裁决，不计为锚点失败）。

---

## 4. 需回写修正清单

### 4.1 对 02 号（按严重度排序）

1. **【错误·必须改】§2.14 与 §6.2：`active_task_key` 归属集合**。改为：`agent_tasks.active_task_key`（索引 `uniq_memory_active_task_key` 建于 `db.tasks()`，indexes.rs:863-865；写入方 webhooks.rs:136,189 / agent/memory.rs:3092,3114,3166 / routes/contacts.rs:1530,1550；三种取值 inbound_reply / memory_consolidation / initial_profile）。operating_memories 集合上无此字段。02 号 §3.3/§5.2 本已正确，修 §2.14/§6.2 即可消除内部自相矛盾。
2. **【补充·应改】§2.15：locked_fields "默认 7 项" → 8 项**。page_merge.rs:35-44 DEFAULT_LOCKED_FIELDS 实为 8 项；models.rs:1837 注释过时，02 应改为"默认 8 项（models.rs:1837 注释写 7 项已滞后）"。
3. **【补充·应改】§6.6 / §5.1：ChunkProvenance.source 值域**。除注释 4 值与 m055 的 `lesson_promotion` 外，还有 `principal_authorized`（chunk_revisions.rs:98-138 枚举第五值；ledger.rs:733 请示沉淀写点）。持久化值域 ≥6 种。
4. **【补充·应改】§2.20 与 §5.1 事实卡：AgentRunLog.outbox_status**。模型注释 5 值（models.rs:3472-3475）滞后：dispatcher 聚合实际还写 `partially_sent`（outbox_dispatcher.rs:1008）与 `delivery_unknown`（:1016-1021）。事实卡应标注"注释 5 值 + 实际另有 2 值"。
5. **【澄清·建议】§2.20：source_kind**。补一句："models.rs 注释 3 值 = envelope 写入面（write_run_envelope_started 仅 gateway.rs:371/1757 两个调用点）；run_envelope.rs:34-45 另有 principal_escalation/principal_clarification/system_incident 三常量，仅流入 agent_send_outbox.source_kind"。消除与 01 号"6 值闭集"的表面冲突。
6. **【补充·建议】§6.2 raw 字段清单**。(a) 明示口径为"有索引支撑的 raw 字段"；(b) 补 `post_release_reviews.actual_negative_reaction_rate_delta`（post_release.rs:236 写，typed 无）；(c) 提及伴生无索引 raw 字段面（`handoff_updated_at`、`delivery_finalized_at`、agent_tasks 协议字段族、decision_reviews 的 post_decision_claim_token/payload 等），强化"models.rs 非字段全集"的警示。
7. **【轻微·可选】§2.15：dynamic_confidence**。标注 L1830 是简化注释，权威实现（gap_signals.rs:1314-1333）含 min_samples 分支与 dangling penalty。

### 4.2 对引用方记录

8. **11 号 §4.2**：campaign_sends.status 值域应去掉 `skipped_duplicate`（无写点；campaigns.rs 写点仅 prepared:706 / enqueued:789；classify 的该分支是防御输入，当前不可达）。
9. **06 号（无需改，备案）**：疑点 2（extra 镜像 deprecatedFacts cap 6 vs typed 20）与 02 的 typed cap 描述是不同层，交叉无矛盾；引用双方时须注明层面。

---

## 5. 综合可信度评估

- **02 号总体可信度：高**。本次覆盖 8 组对象约 40 项断言对照 + 25 个锚点抽验：锚点 100% 命中；字段清单/闭集枚举/索引形状/迁移协议的转写精度极高（多处行号精确到起止行）。发现的 4 处实质问题中，只有 1 处是**事实性错误**（active_task_key 集合归属，且 02 号内部另两节写法正确——属局部笔误而非理解错误）；其余 3 处（locked_fields 项数、provenance 闭集、outbox_status 值域）共同根因是 **02 忠实转写了 models.rs 的注释，而注释本身滞后于实现**。这提示使用 02 号时的一条纪律：**02 中凡"注释级闭集/注释级数值"（未标 const/enum 定义行的），引用前应到实现侧复核**；02 中凡 const/断言函数级的闭集，可靠性经抽验为 100%。
- **引用方与 02 的接口一致性：好**。8 份记录对共享模型的描述与 02 基本吻合；唯一实质失真在 11 号（campaign_sends 值域）。01/05/06/07/10 各自与源码的吻合度在本次涉及断言内为 100%（07 号的 locked_fields 8 项、05 号的 6 态闭集、10 号的三 delta 均比 02 更贴近实现——引用方深读业务文件时天然以实现为准，反而校准了 02 的注释转写）。
- **残余未复核面**：02 号对 Contact（~40 字段）、DomainProfile（30+ 字段）、RuntimeParametersTyped（33 默认值）等大模型的逐字段清单未在本次逐一复读（非引用方争议点、02 主会话已有部分抽查）；ContentAsset 模型体（L1249-1306）仅做了与 11/05 号的语义比对未逐行复读。这些区域按本次抽验的错误率外推风险很低，但引用其**注释级数值**时仍应遵循上一条纪律。
- **对"数据契约地基"的判定**：02 号可以作为改动参考的主索引使用，前提是先应用 §4.1 的 1-4 项回写修正（尤其第 1 项——按 02 §2.14 原文去 operating_memories 找单飞锁会走错集合）。

---

## 6. 覆盖自证

**记录侧（全文通读）**：
| 文件 | 行数 | 读法 |
|---|---|---|
| 02-models-db.md | 882 | 4 段全读（1-220 / 221-460 / 461-700 / 701-882） |
| 03-webhooks-tasks.md | 609 | 一次全读 |
| 05-outbox-escalation-send.md | 570 | 一次全读 |
| 01-agent-gateway.md | 646 | 一次全读 |
| 06-memory-reaction-runtime.md | 391 | 一次全读 |
| 07-knowledge-engine.md | 615 | 一次全读 |
| 08-knowledge-routes-workers.md | 550 | 一次全读 |
| 10-evolution-workers.md | 491 | 一次全读 |
| 11-routes-business.md | 546 | 一次全读 |

**源码侧（本次裁决亲读，Read 精读段 + Grep 定位）**：
- src/models.rs：875-975（AgentTask+闭集+单测）、3405-3585（AgentRunLog 尾段+OutboxEntry 全体）、1404-1518（DeciderRef/AskHumanQuietHours/AskHumanPolicy/ReferralCard/AgentSendLedger）、1660-1719（EvolutionRuntimeFlag 尾+OperatingMemory+MemoryCandidate）、1796-1810（wiki_type 注释/幽灵迁移）、1820-1950（chunk 尾段+双闭集+B3 谓词+coerce）、1988-2019（ChunkProvenance/RelatedRef）、655-704（CampaignSend+闭集+断言）、4485-4534（请示常量+Protocol）、4633-4705（AgentPrincipalEscalation）、4980-5024（MemoryCardTyped+cap 注释）、5628-5844（演化五表全体）、1048-1057（import 闭集）、5930-5939（chat task 闭集）、836-847（relay 哨兵）、245-253（outcome_events alias，grep）。
- src/db/indexes.rs：196-214+1720-1738（finalize idx+测试）、359-392（active_task_key idx+handoff idx）、240-260（proactive 尾+outbox 幂等）、745-780（contacts 两条+messages 首条）、855-866（tasks() 建索引点）、1306-1317（projection_key）、1833-1851（active_task_key 测试）、1962-1983（llm_provider drop+重建）、2048-2079（campaigns/campaign_sends）、2145-2236（post_decision 四条+proactive touch）。
- src/db/migrations/：mod.rs:540-585（审批闸/env/生产判定）+ `Migration {` 计数（=58）；m030:40-61；m038:74-94；m052:21（grep）。
- 业务写点：src/webhooks.rs（active_task_key/handoff_status 各写点，grep 全文）、src/agent/memory.rs:2648-2676（projection_key 写点全文）+ active_task_key 各写点（grep）、src/agent/post_decision.rs（post_decision_* 全部写点，grep 40 条）、src/agent/outbox_dispatcher.rs:975-1030（聚合函数全文）+ delivery_finalize_pending 写点（grep）、src/agent/run_envelope.rs:30-59（source_kind/lifecycle 常量）、src/agent/escalation/ledger.rs:711-736（PrincipalAuthorized 写点）、src/evolution/post_release.rs（actual_* 写点，grep）、src/routes/campaigns.rs（campaign_sends 全部写点 + skipped_duplicate 全部出现点，grep×2）、src/knowledge_wiki/page_merge.rs:28-62（LOCKED/UNION 常量）、src/knowledge_wiki/chunk_revisions.rs:95-142（ProvenanceSource）、src/db/mod.rs:105-118+275-288（accessor 行号）、gateway.rs write_run_envelope_started 调用点（grep）。

**边界声明**：本记录只裁决"02 与引用方之间存在出入、或本次抽中的锚点"所涉断言；未覆盖的 02 号内容（如 Contact/DomainProfile 逐字段清单、§4 迁移表 58 行的逐条语义、§5.3 runtime 默认值表）沿用 02 号原文与其主会话抽查结论，引用前请按 §5 的"注释级数值须复核"纪律执行。
