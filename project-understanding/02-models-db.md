# models.rs 与 DB 层深读记录（核证日期 2026-08-13）

> 读取方式：Read 工具逐段读完全文（无跳读）。所有断言均附 `file:line`（行号为 2026-08-13 工作树版本；注意 git status 显示 `src/models.rs` 等文件有未提交修改，行号对应当前工作树）。
> 涉及文件：`src/models.rs`（8353 行）、`src/db/mod.rs`（456 行）、`src/db/indexes.rs`（3444 行）、`src/db/config_generation.rs`（108 行）、`src/db/migrations/mod.rs`（721 行）、`src/db/migrations/helpers.rs`（263 行）、m001–m058 全部迁移文件（共 7536 行）。

---

## 1. 模型总地图（按行号，src/models.rs）

### 1.1 核心业务实体
| 行号 | 名称 | 一句话 |
|---|---|---|
| 6-17 | `AgentStatus`(enum) | 联系人接管态：`normal`（仅持久化）/`managed`（AI 运营），serde lowercase，Default=Normal |
| 19-30 | `AgentProfile` | AI 生成的联系人画像四字段（summary/interests/communication_style/operation_goal），camelCase |
| 32-55 | `string_or_vec`(fn) | interests 反序列化容错：字符串按逗号/分号/换行切分成 Vec |
| 57-99 | `WechatAccount` | 微信账号（`wechat_accounts` 集合），含 MCP 连接密钥、webhook 验签密钥、D4 多账号调度字段 |
| 103-136 | `impl Debug for WechatAccount` | 手写 Debug：mcp_api_key/webhook_secret 走 mask_secret 掩码防日志泄漏 |
| 140-149 | `HourRange` | 账号"勿打扰"小时区间 [start,end)，camelCase，跨午夜合法 |
| 151-260 | `Contact` | 联系人主实体（`contacts`），~40 字段：三层标签、贝叶斯旁路、OCEAN 画像、domain_attributes 容器、commitments、意图轨迹、outcome_events |
| 264-269 | `Evidence` | 标签证据引用（turn+msg_id），不拷贝原文 |
| 272-281 | `ConfirmedTag` | AI 确信层标签（压缩归并 replace 写回，必带证据） |
| 286-304 | `ApiConfirmedTag`(+From) | API 投影：confirmed_at 转 RFC3339 字符串 |
| 307-320 | `BayesianPoint` | 贝叶斯观测点（append-only ledger，source_run_id 幂等锚） |
| 324-334 | `BayesianSignal` | 贝叶斯维度槽（≤6 槽，locked 才画走势，永不驱动行为） |
| 337-344 | `PersonalityFacet` | 大五单维度（score/confidence/evidence_refs） |
| 348-354 | `PersonalitySnapshot` | 人格演化快照（scores/confidences 固定 [O,C,E,A,N] 序） |
| 357-368 | `PersonalityProfile` | OCEAN 画像（仅压缩归并时更新，不驱动行为） |
| 371-418 | `ApiPersonalitySnapshot`/`ApiPersonalityProfile`(+From) | API 投影（DateTime→string，D4-F1 契约） |

### 1.2 交易/产品/活动
| 行号 | 名称 | 一句话 |
|---|---|---|
| 429-475 | `OutcomeEvent` | 成效事件（嵌入 Contact.outcome_events，camelCase 子文档）：G3 verification 分级 + §4.5 deal/reversal |
| 477-483 | `default_outcome_kind`/`default_outcome_verification`(fn) | 缺省 "deal" / "staff_confirmed" |
| 489-515 | `OutcomeProductRef`(+default_quantity) | 成交时的产品订单式快照（非活引用），quantity 缺省 1 |
| 528-561 | `Product`(+default_product_status) | workspace 级产品目录（`products`，纯 snake_case——索引键逐字一致红线），status 缺省 active |
| 567-576 | `is_valid_currency_code`/`is_valid_minor_amount`(fn) | ISO-4217 形态校验；金额（分）非负校验 |
| 581-636 | `Campaign`(+default_campaign_spec_version) | 定向推送活动（`campaigns`，**camelCase BSON**），spec_version/spec_hash/dispatch 冻结四件套 |
| 639-654 | `SegmentFilter` | 圈人条件（productIds/aftercare/valueTier/customerStage，各维 AND） |
| 658-679 | `CampaignSend` | 活动每人推送台账（`campaign_sends`，camelCase），(campaignId,contactWxid) 唯一去重闸 |
| 683-703 | `ALLOWED_CAMPAIGN_STATUS`+`assert_campaign_status_valid` | 6 态闭集 + 写入站点断言 |

### 1.3 行为信号/消息/任务
| 行号 | 名称 | 一句话 |
|---|---|---|
| 723-772 | `BehaviorSignal` | 自学习行为信号（`behavior_signals`，snake_case 铁律——partial unique 按 snake 键匹配），观察/解释分层，silence=删失 |
| 783-802 | `BehaviorSignalMetric` | 采集健康度日计数（`behavior_signal_metrics`，`_id="{ws}:{date}"`） |
| 804-809 | `MessageDirection`(enum) | inbound/outbound，lowercase |
| 811-838 | `ConversationMessage` | 会话消息（`conversation_messages`）；`is_synthetic_relay` skip_serializing+skip_deserializing 双保险（L835-836） |
| 841 | `PRINCIPAL_RELAY_SENTINEL`(const) | relay 合成消息哨兵前缀 `__PRINCIPAL_RELAY__` |
| 843-876 | `impl ConversationMessage::synthetic_principal_relay` | 构造"领导已裁决"合成 inbound（仅内存，不落客户可见会话） |
| 878-907 | `AgentTask` | 跟进任务（`agent_tasks`），claim/retry/gateway_status 字段 |
| 918-941 | `ALLOWED_AGENT_TASK_STATUS`+assert | 9 态闭集（含 `committing`）+ 断言 |
| 943-971 | `agent_task_status_tests`(mod) | 单测锁定 8 个历史写入面 + 越界 panic |
| 973-1047 | `ImportJob` | 异步知识导入 job（`import_jobs`，snake_case），apply 生命周期独立（ready→applying→applied）、claim_generation/claim_token fencing、expires_at TTL 锚 |
| 1052-1071 | `ALLOWED_IMPORT_JOB_STATUS`+validate+assert | 4 态闭集（pending/running/completed/failed） |
| 1073-1090 | `import_job_status_tests`(mod) | 闭集单测 |
| 1092-1126 | `chunk_classification_tests`(mod) | coerce_wiki_type/coerce_chunk_type 归一契约单测 |
| 1128-1182 | `anchor_citability_tests`(mod) | B3 锚点可引用性契约单测（sourceQuote 非空才可引用） |
| 1184-1202 | `AgentEvent` | 审计事件（`agent_events`），可选 dedupe_key 参与 partial unique |
| 1204-1220 | `MigrationRecord` | 迁移记录（`migrations`，`_id`=迁移 id），status applied/blocked，legacy 行无 status 视为 applied |
| 1222-1233 | `McpCallLog` | MCP 调用日志（`mcp_call_logs`） |
| 1238-1247 | `RosterSnapshot` | 通讯录全量快照（`roster_snapshots`，每 ws+account 一条覆盖写） |

### 1.4 素材/人格/Prompt/引荐
| 行号 | 名称 | 一句话 |
|---|---|---|
| 1249-1306 | `ContentAsset` | 素材（`content_assets`）：文本+文件双形态、发送标注（sendable/target_stages）、审核态、min_inject_tier 分档注入 |
| 1308-1333 | `AgentSoul` | Agent 人格（`agent_souls`），版本化 + published_at/by 审计 |
| 1335-1371 | `PromptTemplate` | prompt 模板（`prompt_templates`），canonical current 指针（每 (ws,key) 恰一条 current+active）、source_proposal_id 演化所有权 |
| 1373-1401 | `OperationPlaybook`(+default_playbook_release_status) | 运营方法论（`operation_playbooks`），is_default 每账号至多一条，release_status 缺省 published |
| 1404-1415 | `DeciderRef` | 请示决策人引用（wxid+display_name+account_id），camelCase |
| 1418-1424 | `AskHumanQuietHours` | 请示推送静默时段 |
| 1427-1449 | `AskHumanPolicy` | 请示通道策略（决策人链/四 escalate 开关/去重窗/日推上限/超时），camelCase |
| 1454-1478 | `ReferralCard` | 专属顾问名片（`referral_cards`，snake_case），必须人审 approved+enabled 才可被选 |
| 1483-1517 | `AgentSendLedger` | 主动发送台账（`agent_send_ledger`），outbox_id 不可变送达锚，responded/stage_advanced 回扫填 |

### 1.5 运营域配置/演化开关
| 行号 | 名称 | 一句话 |
|---|---|---|
| 1519-1571 | `OperationDomainConfig`(+default_version_one) | 运营域配置（`operation_domain_configs`）：runtime_parameters+state_machine 两 Document + E5-T1 四版本字段 + 请示通道三字段 + assist_mode_enabled |
| 1575-1577 | `default_true`(fn) | serde 默认 true 辅助 |
| 1592-1625 | `OperationStatePolicy` | 状态动作策略（`operation_state_policies`），allowed/forbidden/recommended_pace，仅 active 参与拦截 |
| 1639-1666 | `EvolutionRuntimeFlag`(+rollout_percent_clamped) | 演化器运行时开关（`evolution_runtime_flags`），hash(contact)%100<rollout 灰度 |
| 1668-1695 | `OperatingMemory` | 运营记忆（`operating_memories`）：context_pack + memory_card(typed) 双版本号 |
| 1697-1714 | `MemoryCandidate` | 记忆候选（`memory_candidates`） |

### 1.6 知识子系统
| 行号 | 名称 | 一句话 |
|---|---|---|
| 1716-1763 | `OperationKnowledgeDocument` | 知识文档（`operation_knowledge_documents`）：catalog 持久化四字段（persisted/version/desired_generation/applied_generation） |
| 1765-1855 | `OperationKnowledgeChunk` | 知识切片（`operation_knowledge_chunks`）：wiki 方法论 14 个前向兼容字段 + chunk_type（缺省 product_fact） |
| 1859-1861 | `default_chunk_type`(fn) | 缺省 "product_fact"（最保守 verified-only） |
| 1865-1883 | `ALLOWED_WIKI_TYPE`/`ALLOWED_CHUNK_TYPE`(const) | 9 类知识形态 / 4 类运营用途闭集（正交） |
| 1888-1946 | `coerce_wiki_type`/`anchor_is_citable`/`chunk_has_citable_anchor`/`coerce_chunk_type`(fn) | 归一与 B3 可引用性谓词（写读两侧单一真相源） |
| 1948-1989 | `impl Default for OperationKnowledgeChunk` | 全空默认（chunk_type=product_fact） |
| 1995-2007 | `ChunkProvenance` | 写入来源（source∈{ai,human,rule,imported}，llm_model_alias 禁具体模型名） |
| 2010-2016 | `RelatedRef` | 关系引用（6 种 kind 闭集：superseded_by/references/requires/contradicts/clarifies/refines，见 L1824） |
| 2019-2029 | `UsageStats` | 30 天滑窗使用统计 |
| 2033-2060 | `ChunkRevision` | 不可变编辑历史（`chunk_revisions`），op 10 种闭集、before/after hash+snapshot |
| 2066-2096 | `KnowledgeGapSignal` | 知识缺口信号（`knowledge_gap_signals`），8 类 kind、status 5 态流转、dedup_key partial unique |
| 2105-2157 | `IngestSource` | 自动 ingest 源（`ingest_sources`）：kind∈{rss,html}、status∈{active,failing,disabled}、source_generation/claim_generation 双代次 fencing |
| 2164-2181 | `DomainSchema` | 行业可配 schema（`domain_schemas`），alias_dict 透明 rewrite，每 ws 一条 active |
| 2184-2195 | `DomainField` | schema 字段定义（kind∈{string,enum,number,date,reference}） |

### 1.7 DomainProfile 总装配单及其子结构
| 行号 | 名称 | 一句话 |
|---|---|---|
| 2239-2457 | `DomainProfile`(+default_domain_profile_release_status) | 行业总装配单（`domain_profiles`）：30+ 可配字段分三类接线（runtime 标量/prompt 文本/有意分散读取），DEFAULT 销售 profile 字节等价红线 |
| 2472-2485 | `ProfileThresholds` | 五闸阈值 per-profile 覆盖（camelCase，全 None=不覆盖） |
| 2498-2515 | `ReviewerOrientation` | 评审取向覆盖（review_focus/balance_principle/reviewer_fewshot_override） |
| 2522-2547 | `AnsweringModeProfile`/`AnsweringModeDescriptor` | completeness 三档释义/标签覆盖（key 恒定不可配） |
| 2563-2611 | `OperationMode`(+Default) | 运营范式七驱动力容器（funnel/silence/commitment/quiet_hours/calendar/renewal/reactivation） |
| 2614-2630 | `FunnelMode` | 漏斗推进（enabled 默认 **true**） |
| 2633-2649 | `SilenceMode` | 沉默唤醒（默认 true） |
| 2652-2669 | `CommitmentMode` | 承诺到期（默认 true） |
| 2678-2690 | `QuietHoursMode` | 作息门控覆盖（enabled_override 三态 None/Some(false)/Some(true)） |
| 2698-2720 | `CalendarMode` | 主动情绪关怀（enabled 默认 **false**，销售域 no-op） |
| 2728-2753 | `RenewalMode` | 续费推进（默认 **false**） |
| 2762-2789 | `ReactivationMode` | 再激活（默认 **false**，dormant_days/cadence_days/daily_cap） |
| 2790-2800 | `ProfileDimension` | 画像维度声明（kind/display_name/participates_in_decision/description） |
| 2806-2820 | `RESERVED_PROFILE_DIMENSION_KINDS`(const) | 13 个系统保留 domain_attributes 键名（防维度声明劫持系统状态） |
| 2829-2920 | `validate_profile_dimension_kinds`/`validate_domain_profile_dimensions`(fn) | 维度 kind 形态校验（^[a-z][a-z0-9_]{0,63}$、禁 `_updated_at` 后缀、去重）+ coverage 键/别名唯一性 |
| 2922-2994 | `profile_dimension_validation_tests`(mod) | 上述校验单测 |
| 2997-3005 | `CommitmentMarkers` | 绝对化承诺词表（product_effect/tone_only 两类） |
| 3008-3037 | `CoverageDimension` | completeness 维度（review_topic_aliases/anchor_hint/initial_signal∈{verified,evidence,None}） |
| 3045-3058 | `ChunkRole` | 知识切片用途角色（key/header/order/is_fallback，H16） |
| 3069-3079 | `AnniversaryEntry` | 结构化纪念日条目（date="MM-DD"或"YYYY-MM-DD"+recurring，camelCase） |
| 3092-3119 | `MemoryDimension` | memoryCard 记忆维度声明（key/cap/candidate_type/date_dimension，H17） |
| 3140-3150 | `OutcomePolarity` | 自学习极性声明（positive/negative 集，camelCase；default=空集→消费方回落内置销售极性；删失语义不可配） |
| 3167-3189 | `BusinessFormula` | 经营公式声明（key/expression/display_name(alias="display_name")/eval_score_key，不进硬闸） |

### 1.8 catalog/知识使用/评审/运行日志
| 行号 | 名称 | 一句话 |
|---|---|---|
| 3193-3225 | `CatalogRebuildJob` | catalog 重建 durable intent（`catalog_rebuild_jobs`），status 6 态、owner/token/lease fencing |
| 3227-3245 | `KnowledgeUsageLog` | 知识使用日志（`knowledge_usage_logs`，35d TTL） |
| 3247-3295 | `KnowledgeChatTurn` | 知识 chat 轮次（`knowledge_chat_turns`），status 4 态、apply_result 稳定回执 |
| 3297-3364 | `AgentDecisionReview` | 决策评审（`agent_decision_reviews`）：scores/formula_breakdown/五快照 + reaction claim token/generation + source_task 授权链 + reviewer_misjudge_signal |
| 3366-3483 | `AgentRunLog` | 运行日志（`agent_run_logs`）：R0 Run Envelope（lifecycle 7 态/source_event_id/source_kind）+ R9 审计（revision/self_critique/autonomy_mode/conversation_mode/final_review_status/outbox_status） |

### 1.9 Outbox/事故/字典
| 行号 | 名称 | 一句话 |
|---|---|---|
| 3506-3582 | `OutboxEntry` | 发送 outbox（`agent_send_outbox`）：idempotency_key、claim_token fencing、cancel 两段式、send_started_at 最后可取消点、reclaimed_in_flight 崩溃恢复标记、reclaim_count 止损 |
| 3587-3591 | `SystemIncidentRecipient` | 事故通知接收人（account_id+wxid 冻结） |
| 3596-3632 | `SystemIncident` | 系统事故生命周期（`system_incidents`），generation + 请求时间因果排序防迟到观察 |
| 3639-3666 | `TaxonomyEntry` | 字典条目（`system_taxonomies`），E5-T1 四版本字段 |
| 3670-3698 | `TaxonomyValue` | 字典取值（camelCase）：id/displayName/aliases/status + H6 priority_weight/is_terminal/is_reactivation_target |
| 3703-3717 | `taxonomy_identity_claims`(fn) | canonical id + aliases 去重构造 identity 命名空间 |
| 3722-3740 | `impl Serialize for TaxonomyValue` | **手写序列化**：派生持久化字段 `identityClaims`（供 multikey unique 索引） |
| 3748-3779 | `TaxonomyCandidate`(+default_taxonomy_workspace_id) | 字典候选（`taxonomy_candidates`），status 3 态，workspace 缺省读 env DEFAULT_WORKSPACE_ID |
| 3790-3813 | `RelationshipTypeSuggestion` | 关系类型建议（`relationship_type_suggestions`），(ws,contact) 仅 pending 唯一 |
| 3824-3847 | `SuspectedDealSignal` | 疑似成交待核实（`suspected_deal_signals`），AI 永不直写 outcome 红线载体 |
| 3849-3894 | `LlmCallLog` | LLM 调用日志（`llm_call_logs`）：三段延迟（queue_wait/provider/latency）、cache 命中 token、usage_known、retry_count/final_status |

### 1.10 引导/管理 Agent/评测
| 行号 | 名称 | 一句话 |
|---|---|---|
| 3896-3910 | `GuideSkippedField`/`GuideAuthoritativeChange` | 引导预览跳过字段/权威变更（before/after Bson） |
| 3912-3952 | `GuideFrozenPlan` | 引导冻结写计划（OCC 时间戳基线 + contact/memory/playbook set + memory_insert 基线） |
| 3954-3965 | `GuideApplyReceipt` | apply 回执（preview_id+candidate_hash+committed_at） |
| 3967-4003 | `UserOperationGuidePreview` | 运营引导预览（`user_operation_guide_previews`），Protocol v3 frozen_plan+candidate_hash+apply_receipt |
| 4005-4018 | `ManagementAgentSession` | 管理 agent 会话（`management_agent_sessions`），dry_run 默认位 |
| 4020-4030 | `ManagementAgentMessage` | 管理会话消息（`management_agent_messages`） |
| 4032-4061 | `AgentCommandRun` | 管理命令 run（`agent_command_runs`），plan_hash 冻结绑定 + execution_token 租约 |
| 4063-4081 | `ALLOWED_AGENT_COMMAND_RUN_STATUS`+validate | 7 态闭集 |
| 4083-4107 | `AgentToolCall` | 工具调用（`agent_tool_calls`），intent_key partial unique 防重试双写 |
| 4113-4134 | `ALLOWED_TOOL_CALL_STATUS`+assert | 8 态闭集（含 executed_unverified/accepted"诚实优于好看"态） |
| 4139-4174 | `AgentOutcomeMetric` | 长 horizon 指标（`agent_outcome_metrics`，`_id="{ws}:{acct}:{horizon}:{date}"` 幂等），4 指标 Option 化（None=无数据≠0），ai_hold_cleared_rate alias 兼容旧名 |
| 4181-4207 | `EvaluationScenario`(+default status) | 公式遵守度评测场景（`evaluation_scenarios`），status 缺省 active |

### 1.11 请求体/API 投影/请示通道
| 行号 | 名称 | 一句话 |
|---|---|---|
| 4209-4292 | `EnableAgentRequest`/`BatchEnableCandidate`/`BatchEnableRequest`/`ProfileNoteRequest`/`CustomAgentInstructionsRequest`/`SearchImportRequest`/`ImportContactsRequest`/`ContactQuery` | 8 个 HTTP 请求体（Deserialize-only，camelCase 或 rename 单字段） |
| 4294-4321 | `ApiCommitment`(+From<&CommitmentRepr>) | 承诺 API 投影（Plain→空 id/无 due_at） |
| 4323-4481 | `ApiContact`(+From<Contact>+apply_stagnation_dimension) | 联系人 API 投影：tags=manual+confirmed 合并、时间字段全 RFC3339、stagnation 三字段投影（planner 同源） |
| 4484-4486 | `dt_to_string`(fn) | DateTime→RFC3339 Option |
| 4491-4508 | 请示状态常量 | escalation status 3 态、relay_state 3 态、card delivery 5 态 |
| 4514-4530 | `PrincipalEscalationProtocol` | 请示协议冻结（policy_version+policy+principal_account_id+delivery generation/state/content/outbox_id+failure_cleanup_completed_at） |
| 4533-4580 | 请示类别/attr 键/裁决/豁免常量 | category 3 类、AWAITING_* 两个 domain_attributes 键、verdict 5 种、exemption 3 种 |
| 4583-4604 | `EscalationRequest` | 决策阶段请示意图（needed 缺省 false LLM 容错） |
| 4610-4632 | `PrincipalDecision`(+default_exemption_type) | 真人裁决解读（**snake_case 持久化**，注释明示不加 rename_all 的原因 L4607-4609） |
| 4635-4705 | `AgentPrincipalEscalation` | 请示台账（`agent_principal_escalations`）：short_code 全局唯一、protocol/decision/relay 状态机、last_pushed_at_ms 骚扰门 |

### 1.12 typed 强类型模块（L4716-5600）
| 行号 | 名称 | 一句话 |
|---|---|---|
| 4725-4829 | `RuntimeParametersTyped` | runtime_parameters 强类型版（camelCase）：33 个参数字段全带 default 函数 |
| 4831-4875 | Default+From<→Document> | 默认构造与 BSON 转换 |
| 4877-4978 | `defaults`(mod) | 33 个默认值函数（五闸阈值/预算/作息/贝叶斯门等，见 §5 事实卡） |
| 5003-5019 | `MemoryCardTyped` | memoryCard 强类型（camelCase）：core/recent/deprecated 三 facts 数组 + `extra` **flatten catch-all** |
| 5032-5062 | `MemoryFactRepr`(untagged enum)+impls | Plain(String)/Structured(MemoryFact) 双形态兼容 |
| 5067-5069 | `default_epoch_dt`(fn) | DateTime 缺省 epoch 0 |
| 5099-5175 | `MemoryFact`(+手写 Default) | 完整事实结构（UUIDv4 id/text≤500/confidence 0-10/importance 0-10/dimension/source_message_ids≤5/extra flatten） |
| 5177-5248 | `impl MemoryFact`(from_plain_text/validate) | Plain 升级工厂（fresh UUID+conf 7+imp 5）与 bounds 校验 |
| 5256-5263 | `From<MemoryFactRepr> for MemoryFact` | Plain→fresh UUID；Structured→透传 |
| 5265-5342 | `impl MemoryCardTyped` | to/from_document、is_empty、has_plain_facts、auto_upgrade_plain_facts、live_dimension_names |
| 5351-5397 | `CommitmentRepr`(untagged)+impls | Plain/Structured 承诺双形态（Plain 无 due_at/id） |
| 5406-5456 | `CommitmentEntry`(+from_plain_text/validate/From) | 结构化承诺（UUIDv4 id+due_at+created_at+extra flatten） |
| 5460-5464 | `TrajectoryDimension` | 轨迹维度声明（kind+display_name，snake_case） |
| 5475-5495 | `IntentTrajectoryEntry`(+MAX_ITEMS=50) | 意图轨迹元素（camelCase）：turn_index/intent/objection_type+dimensions BTreeMap 容器 |
| 5501-5545 | `OperationStateMachineTyped`/`OperationStateTyped`(+From) | 状态机强类型（camelCase）：key/allowed_from/allow_from_any/initial/forbids_proactive 等 |
| 5547-5593 | `live_dimension_names_tests`(mod) | 维度名去重保序单测 |
| 5596-5600 | `pub use typed::{...}` | 对外导出 10 个 typed 类型 |
| 5602-5616 | `impl OperationDomainConfig` | runtime_parameters_typed()/state_machine_typed()（失败回落 default） |

### 1.13 自演化/知识日报/LLM 配置
| 行号 | 名称 | 一句话 |
|---|---|---|
| 5628-5653 | `Experiment` | 演化实验信封（`experiments`），status 5 态、cohort run ids、预算计数 |
| 5657-5718 | `Proposal` | 演化候选（`proposals`）：threshold/prompt 双 kind、status 6 态、base_revision/released_revision 不可变身份 |
| 5722-5755 | `ShadowReplay` | 影子重放（`shadow_replays`），新旧 5 闸命中向量对照（original_5gate_hit G4 修复） |
| 5759-5782 | `ThresholdOverride` | 阈值覆盖层（`threshold_overrides`），current_version partial unique |
| 5797-5816 | `ThresholdOverrideAudit` | 阈值变更审计（`threshold_overrides_audit`，append-only），action 3 种、decided_by 4 形态 |
| 5820-5837 | `PostReleaseReview` | 发布后 +24h 对比评测（`post_release_reviews`） |
| 5847-5880 | `KnowledgeDigestCard` | 日报卡片（camelCase）：kind 7 种/suggested_action 6 种/severity 3 档闭集；target_refs 不查库外键（fail-soft） |
| 5884-5930 | `KnowledgeDailyReport` | 知识日报（`knowledge_daily_reports`），(ws,acct,report_date) 唯一，attempt/current generation 防迟到覆盖 |
| 5934-5935 | `ALLOWED_TASK_STATUS`(const) | KnowledgeChatTask 5 态闭集（"finished"已更名"completed"） |
| 5939-5995 | `KnowledgeChatTask` | 知识长任务（`knowledge_chat_tasks`）：cards 快照、dispatch_binding、planned/completed_steps、step_intents、claim fencing |
| 5999-6020 | `KnowledgeOperatorMemory` | 运营偏好记忆（`knowledge_operator_memory`），kind 3 种、expires_at TTL、软撤销三字段 |
| 6026-6060 | `LlmProviderConfig` | LLM 服务商配置（`llm_provider_configs`，**camelCase**）：format∈{openai,anthropic}、is_active/is_vision_active 双正交指针 |
| 6064-6085 | `impl Debug for LlmProviderConfig` | api_key 掩码 Debug |

### 1.14 尾部测试模块（全部读完，契约见 §2 各模型）
`typed_tests`(6087-7362)、`principal_escalation_model_tests`(7364-7426)、`objective_purchase_facts_model_tests`(7428-7539)、`relationship_type_suggestion_tests`(7541-7577)、`generated_state_machine_tests`(7579-7593)、`content_asset_compat_tests`(7595-7658)、`referral_card_compat_tests`(7660-7720)、`send_ledger_compat_tests`(7722-7768)、`tag_trust_tests`(7770-8091)、`campaign_model_tests`(8093-8163)、`conversation_message_relay_tests`(8165-8238)、`avatar_field_tests`(8240-8262)、`wechat_account_debug_tests`(8264-8301)、`roster_snapshot_tests`(8303-8353)。

---

## 2. 逐模型深读

> 通用规律（先记住这三条，下面不再逐个重复）：
> ① **BSON 命名三分**：绝大多数 struct **无** `#[serde(rename_all)]` → BSON 键 = Rust snake_case 字段名；显式 camelCase 的 collection 级 struct 只有 `Campaign`(L582)、`CampaignSend`(L659)、`LlmProviderConfig`(L6027)；大量**嵌入式子文档**（OutcomeEvent、AskHumanPolicy、TaxonomyValue、MemoryCardTyped、RuntimeParametersTyped、KnowledgeDigestCard、IntentTrajectoryEntry、AnniversaryEntry、DeciderRef、SegmentFilter、GuideAuthoritativeChange/ApplyReceipt、EscalationRequest、OutcomePolarity、BusinessFormula、ProfileThresholds、ReviewerOrientation、AnsweringMode*、OperationStateMachineTyped 等）是 camelCase。索引键必须与实际 BSON 键逐字一致（`Product` L527 注释、indexes.rs:2053-2056 注释都强调此红线）。
> ② **主键惯例**：`pub id: Option<ObjectId>` + `#[serde(rename="_id", skip_serializing_if="Option::is_none")]`（L3495 设计注记）；例外是 `_id` 为业务字符串的 `MigrationRecord`(L1206)、`BehaviorSignalMetric`(L785)、`AgentOutcomeMetric`(L4141)。
> ③ **向后兼容**：所有模型均无 `deny_unknown_fields` → 旧库多余字段静默忽略（L7127-7163 legacy chunk 测试即证明）；新增字段一律 `#[serde(default)]`（缺字段回落默认）；改名用 `alias`（叠加式，旧 wire 键仍认）。

### 2.1 `WechatAccount` → 集合 `wechat_accounts`（L57-99）
snake_case。字段：`id`；`workspace_id`/`account_id`（租户二元组，唯一索引）；`alias`/`display_name`（运营标注）；`app_id: Option<String>`（gewe 应用 id，partial unique 索引 `uniq_wechat_accounts_app_id`，webhook 路由据此定位账号）；`wxid`/`nick_name: Option`；`avatar_url: Option`(default)；`mcp_base_url`/`mcp_api_key: Option`（每账号可覆盖全局 MCP）；`webhook_secret: Option`(default，L68-76)——方案 B 回调验签密钥，None+验签开=fail-closed 拒绝；`online: bool`（实时连接态）与 `status: Option<String>`(default，L78-82)（MCP 侧生命周期态 active/inactive）语义区分；`last_sync_at`；`capacity: u32`(default 0=不参与多账号轮询，L84-87)；`persona_tag: Option`(default，同 persona 才互替 L88-92)；`off_hours: Vec<HourRange>`(default，命中即 round-robin 跳过 L93-96)；`created_at`/`updated_at`。手写 Debug 掩码两个密钥（L103-136；测试 L8293-8300 锁定 webhook_secret 不泄漏）。

### 2.2 `Contact` → 集合 `contacts`（L151-260）
snake_case（L151 无 rename_all）。唯一索引 `(workspace_id, account_id, wxid)`（indexes.rs:751-759）。逐字段：
- 身份：`id`；`workspace_id`/`account_id`/`wxid`（租户三元组）；`nickname`/`remark`/`alias: Option`；`avatar_url: Option`(default，旧文档缺→None，测试 L8246-8261)；`sex: Option<i32>`(default)。
- 运营态：`agent_status: AgentStatus`（webhook 只对 managed 自动回复）；`human_profile_note: Option`；`custom_agent_instructions: Option`(default，L167-171，≤1000 字符 Operator Instruction 层，PUT /api/contacts/:id/custom-agent-instructions 写)；`operation_mode_override: Option<OperationMode>`(default，L172-178，单客户范式覆盖，`planner::resolve_operation_mode` 读，优先级 contact>profile>default)。
- 画像：`agent_profile: Option<AgentProfile>`；`memory_summary: Option`；`playbook_id: Option<ObjectId>`/`playbook_version: Option<i32>`。
- 三层标签（tag-trust 改造）：`manual_tags: Vec<String>`(default，L183-186 运营权威层，AI 写路径不触达)+`manual_tags_updated_at`/`manual_tags_by`(default)；`confirmed_tags: Vec<ConfirmedTag>`(default，L191-193 AI 确信层，压缩归并 replace 写回)；`bayesian_signals: Vec<BayesianSignal>`(default，L194-196 ≤6 槽纯观测)；`personality_profile: Option<PersonalityProfile>`(default)；`tags_version: i64`(default 0，压缩归并递增)。m027 物理回填空默认值。
- 业务维度：`domain_attributes: Option<Document>`(default，L203-206 业务字段 JSON 容器，DomainSchema 校验，取代硬编码 customer_stage/intent_level/objection_type)+`domain_attributes_updated_at`(default)。**注意**：该容器内每个维度 `<kind>` 拥有配套时钟键 `<kind>_updated_at`（validate_profile_dimension_kinds L2847 保留后缀），另有系统保留键 13 个（L2806-2820）。
- 承诺：`commitments: Vec<CommitmentRepr>`(default，L210-218，cap 8，m008 从 last_commitment 迁移)。
- 状态轴：`follow_up_policy`/`operation_state`/`operation_state_reason`/`operation_state_confidence`/`operation_state_updated_at`/`cooldown_until`（全 Option，无 default——**旧文档必须有这些键或值为 null**；m008 等管道写入时代已保证）；`operation_policy: Document`(default)；`profile_attributes: Document`(default)；`profile_updated_at: Option`。
- 时间轴：`last_message_at`/`last_inbound_at`/`last_outbound_at`/`last_agent_run_at`（m001 把 last_message_at 回填进 last_inbound_at）。
- 风格/轨迹：`last_outbound_style: Option`(default，L234-239 D2 风格指纹弱参考)；`intent_trajectory: Vec<IntentTrajectoryEntry>`(default，L240-244 滑窗 50)。
- 成效：`outcome_events: Vec<OutcomeEvent>`(default+**alias="deal_events"**，L245-252，admin 手动标记正例-only；alias 兼容改名前旧库——m030 因此对两个键名分别回填且过滤器绝不共享 $or，否则两键同现触发 serde duplicate_field 崩溃，见 m030:47-56)。
- `locale: Option`(default，L253-257 BCP-47)；`created_at`/`updated_at`（必填）。
- 单测锁定契约：最小文档反序列化新字段全默认（L7844-7885）；ApiContact 投影 manual+confirmed 合并去重保序（L7941-7999）；嵌套 DateTime wire 必须是 string（L8001-8090）。

### 2.3 `OutcomeEvent`/`OutcomeProductRef`/`Product`（L429-561）
- `OutcomeEvent`（camelCase 子文档）：`marked_at`（admin 点击时刻）；`occurred_at: Option`(default)；`amount: Option<i64>`(default，**最小币种单位整数分**，reversal 下为退款正向量级 L437-441)；`currency: Option`(default ISO-4217)；`source: String`(default 空，本阶段恒 "manual")；`marked_by`(default)；`note: Option`(default)；`verification: String`(default fn→"staff_confirmed"，L456-461——旧库缺键视为已核实，新写 conversation_inferred 必须显式，测试 L7436-7483)；`product_ref: Option<OutcomeProductRef>`(default，订单式快照非活引用)；`event_kind: String`(default fn→"deal"，L467-474，reversal 不删原 deal 审计红线)。
- `OutcomeProductRef`（camelCase）：`product_id`（软引用 Product.product_id）；`name`（快照）；`unit_price: Option<i64>`(default，分)；`sku: Option`(default)；`quantity: u32`(default fn→1，测试 L7487-7492)；`entitlement_days: Option<i64>`(default，G4#4 成交时冻结的售后天数快照，产品 archived 后仍正确判 in_aftercare L505-510)。
- `Product` → `products`（**纯 snake_case**，L527 注释明示与索引键逐字一致红线；测试 L7497-7517 锁定序列化键名）：`workspace_id`/`product_id`（业务主键，(ws,product_id) unique）/`name`/`price: Option<i64>`(default，分)/`currency`/`sku: Option`(default)/`status`(default fn→"active"；active/archived)/`summary: Option`(default)/`attributes: Document`(default，行业可变容器，G4 读 entitlement_days)/created_at/updated_at。
- 纯函数：`is_valid_currency_code`（3 大写 ASCII，仅形态 L564-569）、`is_valid_minor_amount`（None 合法/Some 非负 L571-576），调用方转 BadRequest。

### 2.4 `Campaign`/`SegmentFilter`/`CampaignSend`（L581-703）
**camelCase BSON**（与前端 JSON 契约一致，L580）。`Campaign`：`workspace_id`/`account_id`/`title`/`intent_text`（注入 follow_up content）；`segment_filter`(default)；`spec_version: i64`(default fn→1，可编辑草稿单调身份)；`spec_hash: Option`(default，规范化后 SHA-256，legacy 行读路径重算)；`status`（闭集 6 态 `ALLOWED_CAMPAIGN_STATUS` L683-690：draft/previewed/confirmed/dispatching/completed/canceled；写入站点断言 L694-703，debug panic/release tracing error）；`target_count: Option<i64>`(default)；`dispatched_count: i64`(default，本次去重后新入队数)；`last_dispatch_target_count: Option`(default，KC-06 粗筛命中数回刷，两者差=去重跳过数 L609-613)；**dispatch 冻结四件套**（L614-628）：`dispatch_generation`(default)/`dispatch_spec_hash`/`dispatch_audience: Vec<String>`(default)/`dispatch_intent_text`——首次 dispatch CAS 一次写入，恢复消费冻结受众而非重跑活动查询；`dispatch_started_at`/`completed_at`(default)；`created_by`/`created_at`/`updated_at`。
`SegmentFilter`（camelCase，Default 全空=不约束）：`product_ids: Vec`($in 并集)/`aftercare: Option`(in_aftercare/expired/any)/`value_tier: Option`(high/mid/low)/`customer_stage: Option`(走字典)。
`CampaignSend`：`campaign_id: ObjectId`/`contact_wxid`+(campaignId,contactWxid) unique 活动级去重闸（indexes.rs:2069-2077）；`dispatch_generation`(default)/`spec_hash`(default)；`task_id: Option<ObjectId>`；`status`（prepared=durable intent 已落 / enqueued=确定性 task 已存在，L674）。

### 2.5 `BehaviorSignal`/`BehaviorSignalMetric`（L705-802）
`BehaviorSignal`（**snake_case 铁律**，L719-722：partial filter 按 snake 键匹配，camelCase 会让 unique 形同虚设，曾被 behavior_signal_smoke 逮到）：`workspace_id`/`account_id`/`contact_wxid`；`signal_type`（4 种：reply_latency/reply_length/reactivation/silence L730-731）；`observed_at`（event_time，训练按此切片防 label leakage L732-735）；`source`（恒 "system_observed"）；`confidence: f64`（系统观测恒 1.0）；`censored: bool`（silence=true，删失≠负例 Law②）；`dedupe_key`（幂等键，约定格式 L743-746）；按 signal_type 选填客观量：`latency_ms`/`char_len`/`silence_since`/`silence_ms`/`unanswered`/`reactivated_at`（全 Option default）；`ingest_time: Option`(default，落库时刻，与 observed_at 配对 P2 双时间戳)。
`BehaviorSignalMetric`：`_id: String`="{workspace_id}:{date}"（$inc 幂等）；`persisted`/`dedupe_skipped`/`errors: i64`(default，三态计数——去重撞键≠失败)；`last_success_at: Option`(default，新鲜度)；`updated_at`。

### 2.6 `ConversationMessage`（L811-876）
snake_case。`message_id: Option<String>`（微信 msgId，(ws,acct,message_id) sparse+unique）；`dedupe_key: Option`(default，(ws,acct,dedupe_key) partial unique)；`direction: MessageDirection`(lowercase)；`content`；`msg_type: Option`(default，"text"缺省/"media")；`media_ref: Option`(default，content_assets._id hex)；`raw: Option<Document>`；**`is_synthetic_relay: bool`（L830-836）：`#[serde(default, skip_serializing, skip_deserializing)]`——绝不写库、一切反序列化来源恒 false，客户 payload 塞 true 无效，relay 身份与外部输入彻底脱钩**（三条单测 L8189-8237 锁定：不落库/伪造忽略/缺键 false）；`created_at`。`synthetic_principal_relay` 构造器（L846-876）：以 `__PRINCIPAL_RELAY__\nverdict=..\nsubstance=..\nconstraints=..` 为 content 的内存-only inbound。另有 raw 写入的动态字段 `handoff_status`（webhooks.rs 写，SR-177 crash recovery 扫描，partial index inbound_handoff_pending_idx，indexes.rs:379-392——**typed struct 未声明**，见 §6）。

### 2.7 `AgentTask`（L878-971）→ `agent_tasks`
snake_case。`kind`（follow_up/outcome_aggregation/memory_consolidation/principal_decision_relay 等，字符串开放）；`run_at`/`expires_at: Option`；`content`；`status`（**闭集 9 态** L918-928：pending/running/**committing**/retry/failed/cancelled/sent/completed/outbox_enqueued；断言 L932-941）；`source_decision_id: Option<ObjectId>`；`review_required: bool`(default)；`attempt_count`/`max_attempts: i32`(default)；`next_retry_at`/`gateway_status`/`cancel_reason`/`error: Option`；`claimed_at: Option`(default)；`claim_recovery_count: i32`(default)。注意：doc 注释的历史值清单（L913-917）与内嵌测试（L948-963）都只列 8 值，`committing` 是后加的第 9 值（见 §6 疑点 3）。

### 2.8 `ImportJob`（L973-1090）→ `import_jobs`
snake_case（L978-981 注明索引键对齐要求；前端进度走 GET 手工 camelCase json，不直接序列化本 struct）。分块进度：`segments_total`/`progress_done`/`progress_succeeded`/`progress_failed`(default)；`status` 闭集 4 态（L1052）。**apply 生命周期与抽取 worker 生命周期独立**（L1006-1010）：`owner_admin_id`(default，preview 属主，新 apply 必须精确匹配、legacy 无主行不可消费 L998-1001)/`preview_hash`(服务端封印 SHA-256，拒 stale/伪造 preview body)/`apply_status`(ready→applying→applied，与 imported artifacts 单事务提交)/`apply_request_hash`(重放须同 hash 才拿回执)/`apply_result: Option<serde_json::Value>`(稳定回执)/`applied_at`。claim 协议：`claimed_at`/`claim_generation: i64`(default，每次 claim 递增)/`claim_token: Option`(不可伪造 owner 身份，L1027-1036)/`claim_recovery_count`。`expires_at: Option`(default，L1039-1044：终态置 now+24h，TTL expireAfterSeconds=0 只删终态，防 result 无界堆积)。`result`/`error: Option`。

### 2.9 `AgentEvent`/`MigrationRecord`/`McpCallLog`/`RosterSnapshot`（L1184-1247）
- `AgentEvent`：`kind`/`status`/`summary`/`details: Option<Document>`；`dedupe_key: Option`(default，L1196-1201 P1-2 携带才参与 (ws,dedupe_key) partial unique，防并发 TOCTOU 双写)。
- `MigrationRecord`：`_id: String`（迁移 id）；`applied_at: Option`(default，等待生产审批期间缺失)；`status: Option`("applied"/"blocked"，legacy 无 status 视为 applied L1212-1215)；`reason`/`blocked_at: Option`(default)。
- `McpCallLog`：tool_name/request/response/error 快照。
- `RosterSnapshot`：`friends: Vec<crate::mcp::RosterFriend>`（外部类型，含 sex/is_non_human，round-trip 测试 L8309-8352）/`total`/`fetched_at`；快照龄>24h 后台自刷（L1235-1237）。

### 2.10 `ContentAsset`（L1249-1306）→ `content_assets`
snake_case。基础：`kind`/`title`/`body`/`tags`(default)/`url`/`media_id`/`usage_scene`。文件资产六字段（全 default Option）：`media_type`("image"/"file"/"video")/`file_path`(MEDIA_STORAGE_DIR 相对)/`file_name`/`file_size`/`mime_type`/`file_sha256`（去重索引）。发送标注五字段（default Option）：`sendable`/`send_trigger_hint`/`target_stages: Option<Vec>`/`expression_pref`("file_primary"/"file_support")/`requires_principal_approval`。审核：`review_status`("draft"/"approved")/`review_note`。`min_inject_tier: Option`(default，L1296-1302："lean"/"relational"/"full"；None 按 "full"=改造前只 Full 注入逐字等价；仅文本型 kind 有意义)。兼容测试：旧行 sendable=None 不被误判可发送（L7601-7618）。

### 2.11 `AgentSoul`/`PromptTemplate`/`OperationPlaybook`（L1308-1401）
- `AgentSoul`：(ws,agent_kind,version) unique + published 每 scope 至多一条 partial unique（indexes.rs:480-503）。`status`（draft/published/archived——m042 校验闭集）；`seeded_by: Option`(default，"system"/"manual")；`previous_version`/`published_at`/`published_by`(default)。
- `PromptTemplate`：`prompt_key`/`agent_kind`/`layer`/`prompt_pack_version`；**`current_version: bool`(default，L1352-1356：同 (ws,prompt_key) 恰一条 current=true+status=active，纯 draft 流可为零，共享事务发布 helper 保证，m043 收敛)**；`previous_version: Option<i32>`（rollback 取回）；`seeded_by`（"system"/"legacy_migration"/"evolution_release"）；`locale: Option`(default)；`source_proposal_id: Option<ObjectId>`(default，演化产物所有权，rollback 验证归属，partial unique uniq_prompt_artifact_per_proposal)。
- `OperationPlaybook`：8 个方法论文本字段（method_prompt/profile_method/tag_method/stage_method/intent_method/follow_up_method/reply_style/forbidden_rules/success_criteria）；`release_status`(default fn→"published"，m054 回填)；`is_default`（(ws,acct) partial unique is_default=true）；`version: i32`。

### 2.12 请示通道结构（L1404-1449, 4491-4705）
- `DeciderRef`(camelCase)：wxid/display_name/`account_id: Option`(default，L1410-1414 发卡收复账号身份，legacy 缺省回落客户账号)。
- `AskHumanPolicy`(camelCase)：`decider_chain: Vec<DeciderRef>`(default，空=未启用)；四开关 `escalate_safety_guard`/`escalate_unverified_product`(default **true**)/`escalate_ai_policy_hold`(default false)/`escalate_stuck`(default true)；`dedupe_window_hours`/`daily_push_cap`/`quiet_hours`/`timeout_hours`(全 Option default)。m025 从旧 (principal_decider, high_risk_escalation_mode) 映射回填，all_mode→escalateAiPolicyHold。
- 状态闭集（L4491-4508）：escalation status={pending,resolved,delivery_failed}；relay_state={pending,enqueued,terminal}；card delivery={pending_enqueue,queued,sent,failed_terminal,delivery_unknown}。
- `PrincipalEscalationProtocol`（L4514-4530，snake_case）：`domain`/`policy_version`/`policy`（**冻结**的 AskHumanPolicy 快照——timeout worker 不猜 later config）/`principal_account_id`/`delivery_generation`(default)/`delivery_state`/`delivery_content`/`delivery_outbox_id: Option`/`failure_cleanup_completed_at: Option`(default，durable acknowledgement，清理先行此字段后写，reconciler 可安全重试 L4526-4529)。
- category 闭集 3（out_of_scope_decision/high_risk_gated/stuck_or_undelivered，L4533-4540）；verdict 闭集 5（approved/rejected/conditional/deferred/delegated_back，L4564-4575）；exemption 3（none/customer_only/knowledge，L4578-4580）。
- domain_attributes 系统键（L4545-4561）：`awaiting_principal_decision`（粗布尔）+`awaiting_principal_decision_ids`（每请示自持 id，防并发清错 L4546-4549）+`principal_product_exemption`+`referred_specialist_at`+`referred_card_id`+`assist_mode_override`("force_on"/"force_off")。
- `EscalationRequest`(camelCase)：`needed`(default false——LLM 漏字段安全回落不请示 L4586)；category/reason/question_for_principal/self_serviceable_part/`is_generalizable`(default)。测试：空对象→not needed（L7405-7409）。
- `PrincipalDecision`（**snake_case**，L4606-4609 注释明示：持久化进 snake_case 台账 decision 字段，interpret prompt 须输出 snake 键）：verdict/substance/`constraints: Vec`(default)/`authorization_window_hours: Option<f64>`(default，只控 relay 时效不控长期豁免)/`exemption_type`(default fn→"none")。
- `AgentPrincipalEscalation`（L4635-4705，snake_case）：`short_code`（人类可读全局唯一如 "E1A2"）；`status`/`category`/`reason`/`question_for_principal`/`principal_wxid`；`protocol: Option<PrincipalEscalationProtocol>`(default，legacy 行 None 且刻意不被 timeout 扫描器自动路由 L4654-4657)；`decision: Option<PrincipalDecision>`/`authorization_expires_at`(resolved 时填)；`is_generalizable`/`knowledge_proposal_emitted`(default)；`last_holding_reply_ms: Option<i64>`(default，安抚话术去重)；`last_pushed_at_ms: Option<i64>`(default，KD-05 骚扰门锚，改派刷新，m031 回填=created_at)；`resolved_at`/`resolved_via`("wechat"/"admin")；relay 四字段：`relay_state`/`relay_task_id`(确定性 AgentTask id)/`relay_enqueued_at`/`relay_terminal_at`+`relay_terminal_reason`("delivered"/"authorization_expired")。

### 2.13 `OperationDomainConfig`/`OperationStatePolicy`/`EvolutionRuntimeFlag`（L1519-1666）
- `OperationDomainConfig`：六文本字段（name/goal/methodology/workflow/tool_policy/automation_policy/review_policy）；`runtime_parameters: Document`(default，typed 视图见 RuntimeParametersTyped)；`state_machine: Document`(default，typed 视图 OperationStateMachineTyped)；`status`；E5-T1 四件套：`version`(default fn→1)/`current_version`(default false，(ws,domain) partial unique)/`previous_version`/`seeded_by`；请示三字段：`principal_decider: Option`(default，None=未启用)/`high_risk_escalation_mode: Option`("all"/"decision_only"缺省保守)/`ask_human_policy: Option<AskHumanPolicy>`(default，None 回落旧两字段)；`assist_mode_enabled: Option<bool>`(default，None/false=纯全自治，测试 L7708-7719)。
- `OperationStatePolicy`：`state_key`；`allowed: Vec`(default，空=全部允许白名单不启用)；`forbidden: Vec`(default，命中即拦截优先于 allowed)；`recommended_pace: Option`("slow"/"normal"/"hold" 软提示)；`status`（active 才参与拦截，老库无集合时 enforce fallthrough L1590-1591）；E5-T1 四件套。m013 seed、m057 补 acknowledgement。
- `EvolutionRuntimeFlag`：`enabled`（env EVOLUTION_ENABLED=false 仍可硬关停优先级更高 L1644-1645）；`rollout_percent: u32`(default，hash(contact_id)%100<pct 桶稳定 L1633-1635)+clamp 方法(L1663-1665)；`updated_by: Option`；`threshold_auto_release_enabled`(default false，L1653-1657 HC-017 现政策全人工发布，true 也不能越过代码硬闸，管理 API 拒绝新写 true——仅存量兼容)。

### 2.14 `OperatingMemory`/`MemoryCandidate`（L1668-1714）
- `OperatingMemory` → `operating_memories`，(ws,acct,contact_wxid) unique：四 Document 槽（user_understanding/relationship_state/product_fit/next_action，default）+`context_pack: Document`+`context_pack_version: i32`(default)+`context_pack_updated_at`；`memory_card: MemoryCardTyped`(default)+`memory_card_version: i32`+`memory_card_updated_at`。~~raw 动态字段 `active_task_key`（memory.rs 写，partial unique uniq_memory_active_task_key，单飞锁——typed 未声明，见 §6）~~ **【26 号交叉验证修正 2026-08-13：此句归属错误——`active_task_key` 是 `agent_tasks` 集合的 raw 字段（索引建于 `db.tasks()`，indexes.rs:863-865；写点为任务行：webhooks.rs:136,189 / memory.rs:3092+ / contacts.rs:1530+），operating_memories 无任何写点。本记录 §3.3/§5.2 的归属才是正确的。】
- `MemoryCandidate`：`run_id: Option<String>`/`source`/`candidates: Vec<Document>`(default)/`memory_write_score: i32`(default)/`status`/`reason`。raw 动态字段 `projection_key`（partial unique uniq_memory_projection_key，crash-replay 单写，indexes.rs:1302-1322）。

### 2.15 知识文档/切片（L1716-2029）
- `OperationKnowledgeDocument`：`source_type`/`source_name`/`title`/`summary`/`catalog_summary`；`routing_map`/`risk_notes: Vec`(default)；`product_tags`(≤5 聚合并集)/`business_topics`(≤3)(default，m010 回填)；`raw_content`/`content_hash: Option`；`line_index`/`section_index: Vec<Document>`(default)；`status`/`version`。catalog 持久化四字段（L1749-1762）：`catalog_summary_persisted: Option`(worker 写 markdown 快照 O(1) 直读)/`catalog_version: Option<i64>`(If-None-Match 304)/`catalog_desired_generation: i64`(default，chunk 事务承诺代次)/`catalog_applied_generation`(小于 desired=快照陈旧)。旧文档兼容测试 L7335-7361。m052 另写 raw 标记字段 `catalog_m052_reconciliation_generation`（typed 未声明，见 §6）。
- `OperationKnowledgeChunk`（L1765-1855）：基础字段 `document_id`/`item_id: Option<ObjectId>`/`knowledge_type`/`business_context`/`title`/`summary`/`body`/`applicable_scenes`/`not_applicable_scenes`/`product_tags`(≤5)/`business_topics`(≤3)/`source_quote`/`source_anchors: Vec<Document>`(default)/`integrity_status: Option`("verified"/"needs_review"等)/`confidence_score: Option<i32>`/`status`("draft"/"active"/"archived"等)/`priority: i32`。wiki 方法论 14 字段（全 default，旧文档 None，L1800-1839）：`wiki_type`(9 类闭集)/`domain_attributes`/`provenance: Option<ChunkProvenance>`/`valid_from`/`valid_to`(过期减 stale_penalty)/`superseded_by`/`previous_version_id`/`related_chunks: Option<Vec<RelatedRef>>`(6 种 kind)/`usage_stats: Option<UsageStats>`/`dynamic_confidence: Option<f64>`(base×0.6+hit_rate×0.4−stale_penalty clamp[0,1] L1830)/`integrity_score`/`locked_fields: Option<Vec>`(patch 触碰即 4xx，默认 7 项)。`chunk_type: String`(default fn→"product_fact"，L1841-1854：4 类运营用途，与 wiki_type 正交；product_fact 仅 verified 可背书产品声明)。
- 归一纯函数：`coerce_wiki_type`（闭集外/空→None 留痕，不丢整条 chunk L1885-1901）；`coerce_chunk_type`（闭集外→product_fact 最保守 L1930-1946）；**B3 可引用性谓词**（L1903-1928）：`anchor_is_citable`=anchor 含非空 `sourceQuote` 字符串键（注意 camelCase 键——source_anchors 元素内部是 camelCase）；`chunk_has_citable_anchor`=任一可引用。写侧 verify 闸与读侧 quote_is_chunk_evidence 共享此单一真相源（历史 bug：只查数组非空导致永远无法被引用的 chunk 通过 verify）。
- `ChunkProvenance`(snake_case)：`source`∈{ai,human,rule,imported}（m055 另用 "lesson_promotion"，见 §6 疑点 6）/`source_doc_id`/`source_quote`/`llm_model_alias`（用 provider 别名，**禁具体模型名**L1993-1994）/`edited_at`/`edited_by`。
- `ChunkRevision`：`chunk_id`/`revision_id`/`op`（10 种闭集 L2040）/`patch: Document`/`before_hash`/`after_hash`/`before_snapshot`/`after_snapshot: Option`(default，legacy 缺失可读但不可精确 rollback L2047-2049)/`source`/`reason`/`created_by`。
- `KnowledgeGapSignal`：`signal_id`(unique)/`dedup_key: Option`(default，业务去重键 SHA-256，pending+string 才入 partial unique)/`kind`(8 类：orphan/broken_link/no_outlinks/contradiction/stale/missing_chunk/suggestion/low_confidence——见 db/mod.rs:384-386)/`severity`("warning"/"info")/`source`("rule"/"llm")/`status`(pending→auto_resolved|llm_resolved|applied|dismissed)/`affected_chunk_ids`/`search_queries`/`resolution_note`/`resolved_at`。
- `IngestSource`：`source_id`(unique)；**双代次 fencing**（L2110-2119）：`source_generation`(admin 配置代次，CRUD 递增并吊销 worker claim)+`claim_generation`(执行尝试代次)+`worker_id`/`claim_token`/`locked_until`；`kind`∈{rss,html}；`schedule_minutes`；`last_fetched_at`/`last_etag`(If-None-Match)/`last_content_hash`/`last_error`；`status`∈{active,failing(连败≥3),disabled(≥7 天不可达)}；`failure_streak`/`ingest_count`。红线：worker 落 chunk 全部 draft+needs_review（L2103-2104）。

### 2.16 `DomainSchema`/`DomainField`（L2159-2195）
`DomainSchema`：`schema_id`/`name`/`version`/`fields: Vec<DomainField>`(default)/`alias_dict: Document`(default，`{"客户阶段":"customer_stage"}` 透明 rewrite)/`guard_dsl: Option`(简版 `field OP value` AND 组合)/`is_active`（每 ws 一条 active，partial unique）。`DomainField`：name/label/`kind`∈{string,enum,number,date,reference}/`required`(default)/`allowed_values: Option<Vec>`/`alias_of: Option`。

### 2.17 `DomainProfile`（L2197-2457）——行业总装配单（最大模型之一）
文档级设计注释（L2212-2238）声明字段接线三分类：**runtime 标量类**（apply_active_profile 收敛）、**prompt 文本类**（domain_profile.rs 两条收敛链）、**有意分散读取类**（各业务点 if profile.xxx 就地守门，带 Gxx 收口注释——"分散是设计，不要误判成漏接线强行收口"）。反回归红线：DEFAULT 销售 profile（override 全 None/空）下每条接线原样回落、销售域**字节等价**。
逐字段：`profile_id`/`workspace_id`/`display_name`/`description`(default)；`profile_dimensions: Vec<ProfileDimension>`(default，替代 TAGGED_FIELDS)；`prompt_fragment: Option`(叠加业务上下文层)；`soul_override: Option`(替换人格本体，红线：boundary_protection 不在此、由 policy 写死 L2258-2259)；`methodology_override: Option`(替换运营方法段)；`conversation_mode_policy: Option`(H9 模式判定规则剥离+注入，红线：反接管语义恒由 policy 后续段写死 L2276-2278)；`commitment_markers: CommitmentMarkers`(default)；`coverage_dimensions: Vec<CoverageDimension>`(default)；`stagnation_dimension: Option`(H6，None→customer_stage)；`conversation_modes: Vec<String>`(H9 允许模式集，空→内置四模式)；`operation_mode: OperationMode`(default，H8 三驱动力+四扩展)；`per_relationship_operation_mode: Option<BTreeMap<String,OperationMode>>`(§3.7 按关系类型覆盖，BTreeMap 保 BSON 键序稳定 L2313-2314；解析 contact override ?? per_rel ?? operation_mode)；`grounding_gate_bypass_without_claim: bool`(H14，default false=每条判 grounding 硬闸；true 仅 requiresProductKnowledge 才纳闸；红线：blocked_unverified_product_claim 恒不变 L2322-2325)；`distrust_self_reported_low_risk: bool`(false 允许 light Reviewer/true 强制 full；旧"needs_review=false 跳审"语义已禁用 L2327-2331)；**`transaction_facts_enabled: bool`（L2333-2342：唯一一个 default false ≠ 销售等价的开关——销售域行为是注入，default_domain_profile 必须显式 true；default false 取"失败方向安全"）**；`chunk_roles: Vec<ChunkRole>`(H16，空→内置销售四态)；`outcome_polarity: OutcomePolarity`(H11，空集→消费方回落内置销售极性；正极优先于负极；删失不可配)；`business_formulas: Vec<BusinessFormula>`(H15，不进硬闸)；`memory_dimensions: Vec<MemoryDimension>`(H17，仅 extra 容器业务槽，coreFacts 三 typed 数组不纳入)；`trajectory_dimensions: Vec<TrajectoryDimension>`(H17)；`debounce_window_ms_override: Option<u64>`(H18)；`methodology_generator_preamble: Option`(C3 引导层生成器引导语，§7 护栏：引导层 prompt 不写死行业词)；`threshold_overrides: Option<ProfileThresholds>`(M2 五闸阈值覆盖，字段内 None 仍回落 config)；`reviewer_orientation: Option<ReviewerOrientation>`；`mode_gate_policy_override: Option`(A/T1 模式-闸说明段整段替换，不动 boundary_protection 续行)；`answering_mode_profile: Option<AnsweringModeProfile>`(I，三档 key 恒定只换释义/标签)；`generated_state_machine: Option<Document>`(H13 draft 暂存料，activate 时 validate+publish 新 OperationDomainConfig，**发布后运行时只读 operation_domain_configs 不读本字段——不造双真相源** L2427-2431)；E5-T1：`version`(default 1)/`current_version`(default false)/`previous_version`；`release_status`(default fn→"published"——legacy 行保守视为已发布不可误当草稿 L2440-2445，m051 回填)；`seeded_by`("generated_by_ai"/"manual"/"default")；`is_active`（每 ws 一条 active partial unique）。
维度校验（L2806-2920）：kind 必须 `^[a-z][a-z0-9_]{0,63}$`、非保留键（13 个 L2806-2820）、非 `_updated_at` 后缀、profile 内去重；coverage key `^[a-z][A-Za-z0-9_]{0,63}$`、display_name trim 1..=64、review topic（key/display/alias 归一小写）全 profile 唯一。

### 2.18 `OperationMode` 七驱动力（L2549-2789）
默认开关表（金标）：funnel/silence/commitment `enabled` 默认 **true**（缺省=沿用全局 config，DEFAULT 逐字等价）；quiet_hours `enabled_override` 默认 None（沿用全局 runtime.quiet_hours_enabled，仅覆盖开关不覆盖起止小时 L2671-2677）；calendar/renewal/reactivation `enabled` 默认 **false**（交易/情感域专属，DEFAULT 下对应 scanner 天然 no-op）。各阈值字段全 `Option<i64>`，None 回落 config 键：`stagnation_threshold_days`→strategic_planner_stage_stagnation_threshold_days；`threshold_hours`→…silent_threshold_hours；`imminent_window_hours`→…commitment_imminent_window_hours；calendar `lookahead_days`(默认1)/`daily_cap`(默认3)；renewal `lookahead_days`(14)/`grace_days`(7)/`daily_cap`(3)；reactivation `dormant_days`(30)/`cadence_days`(30)/`daily_cap`(3)。三扫描器对应：funnel→scan_stage_stagnation、silence→scan_silent、commitment→scan_commitments（L2550-2551）。

### 2.19 `AgentDecisionReview`（L3297-3364）→ `agent_decision_reviews`
snake_case。`run_id`/`inbound_message_id: Option`；`reply_text`/`approved`；`scores`/`formula_breakdown: Document`(default)；`risks: Vec`(default)；`rewrite_instruction`/`review_summary`；`playbook_id`/`playbook_version`；`used_knowledge_ids: Vec<ObjectId>`(default)；`prompt_versions: Document`；`operation_state: Option`；`next_best_action`/`context_pack_snapshot`/`domain_config_snapshot`/`runtime_parameters_snapshot`/`send_gateway_result: Document`(default，五快照审计)；`outcome_status: Option`（reaction 归一结果，如 user_replied_buying_signal/analyzing 等——开放字符串，闭集在 agent 层）；`reaction_analysis: Document`(default)；reaction claim 协议（L3336-3344）：`reaction_claimed_at`+`reaction_claim_token: Option`(不可复用 fencing token，提交须 `_id+outcome_status=analyzing+token` 三匹配)+`reaction_claim_generation: i64`(单调审计)；task 授权链（L3345-3351）：`source_task_id`+`source_task_claim_token`(dispatcher 发送前核对同 owner 授权 outbox_enqueued，失权 worker 写的 Outbox 不能触达 MCP)；`reviewer_misjudge_signal: Option`(L3352-3357：approved_but_user_negative/blocked_but_user_positive，feedback_worker 汇总，C2 选 negative_example)；`expected_text_segments: i32`(default，L3358-3361 入队时固化分段数，dispatcher 不得按当前配置重切段)；`status`。**raw 动态字段**：`post_decision_status`/`post_decision_next_retry_at`/`post_decision_locked_until`/`post_decision_scrub_at`/`post_decision_profile_done`（post_decision.rs 写，四条索引支撑，typed 未声明——见 §6）。

### 2.20 `AgentRunLog`（L3366-3483）→ `agent_run_logs`
snake_case（L3414 注明与索引一致）。`run_id`(unique)/`trigger_kind`/`status`；六 Document 段（planner/context/knowledge_route/decision/review/gateway_result，default）；预算四字段：`token_budget`/`tokens_used`/`llm_calls_used`/`unknown_usage_calls`(L3398-3403：非零时真实总量未知，不能把未报告当 0)/`degraded_reasons: Vec`。W1 Run Envelope + R9 审计（全 default 向后兼容 L3409-3412）：`lifecycle`（7 态：started/running/completed/failed_before_decision/failed_after_decision/aborted_by_budget/aborted_by_external_signal）/`source_event_id`（R13 幂等键核心成分）/`source_kind`（inbound_message/follow_up_task/manual_send）/`error_summary`(≤1024)/`abort_reason`(≤256)/`revision_applied`/`revision_reason`(≤1024)/`pre_revision_summary`/`post_revision_summary`(≤2048)/`self_critique`(≤2048)/`autonomy_mode`（auto/assisted/blocked）/`conversation_mode`（四模式）+`conversation_mode_reason: Option`/`final_review_status`（闭集见 spec R9 映射表）/`outbox_status: Option`（pending/in_flight/sent/failed_terminal/canceled，dispatcher 反写）/`memory_consolidator_warnings: Vec`。

### 2.21 `OutboxEntry`（L3498-3582）→ `agent_send_outbox`
snake_case。身份：`run_id`/`decision_id: Option`/`source_event_id`/`source_kind`/`content`/`content_hash`/**`idempotency_key`**（(ws,acct,idempotency_key) unique，m038 把旧 singleton key 重写为 scoped 形态）。排序：`delivery_priority: i32`(default 0，只影响领取顺序不参与幂等/授权 L3520-3523)/`run_sequence: i32`(default，同 run 稳定序，文本 0 递增媒体名片靠后)。载荷类型：`media_asset_id: Option`(发 ContentAsset 文件)/`referral_card_id: Option`(发名片)。重试：`attempt`/`max_attempts`/`status`（闭集 6 态 pending|in_flight|sent|failed_terminal|canceled|delivery_unknown，L3503-3505 注明）/`cancel_reason`/`last_error`/`next_retry_at`。claim 协议：`worker_id`/`locked_until`/`claim_token: Option`(L3545-3548 每次 claim 不可复用，状态推进须 `_id+status=in_flight+worker_id+claim_token` 四匹配)/`claim_generation: i64`(单调审计)。取消两段式（L3552-3560）：`cancel_requested`+`cancel_requested_at`（先记请求，worker 进 MCP 前原子复查）；`send_started_at`（跨过最后可取消点；之后崩溃无法核验→delivery_unknown 禁自动重发）。`task_send_authorization_token: Option`(SR-034 task claim 提交发送意图的不可变授权标记)。崩溃恢复：`reclaimed_in_flight: bool`(L3566-3572，lease 过期改回 pending 时置 true→dispatcher 重发前对这条跑 mcp_already_succeeded post-hoc 核对防重复消息)/`reclaim_count: i32`(超 OUTBOX_MAX_RECLAIMS 转 failed_terminal 止损，独立于 max_attempts L3573-3578)。`sent_at: Option`。**raw 动态字段** `delivery_finalize_pending`（outbox_dispatcher.rs 写，partial index status=sent+finalize_pending=true——见 §6）。round-trip 测试 L6654-6726。

### 2.22 `SystemIncident`（L3584-3632）→ `system_incidents`
snake_case。`incident_key`（(ws,incident_key) unique）/`kind`/`status`/`generation: i64`（代次化事故）/`provider_id`/`model`/`reason`/`recipients: Vec<SystemIncidentRecipient>`（开事故代次时冻结，dispatcher 不得换账号 L3584-3586）/`occurrence_count`；因果时序（L3610-3615）：`first_failure_started_at`/`last_failure_started_at`（**请求发起时刻**而非响应到达时刻排序，拒绝迟到成败观察）；`outage_enqueued_generation`/`recovery_enqueued_generation: Option<i64>`(该代次通知已全部 durable 入队)；`recovered_at`/`recovery_probe_started_at: Option`(L3626-3629 恢复探针发起时刻，更旧请求的迟到失败不能重开)。设计红线：LLM 断供是基础设施事故，与 principal 业务请示刻意分表（L3593-3595）。

### 2.23 Taxonomy 三件套（L3634-3779）
- `TaxonomyEntry`：`workspace_id`(default fn→env DEFAULT_WORKSPACE_ID 或 "default"——**serde default 依赖运行时环境变量**，L3643-3644 注明仅滚动升级窗口兜底，m032 物理回填)；`scope`（"global" 或 account_id）；`kind`（维度名）；`value: TaxonomyValue`；E5-T1 四件套（版本唯一 (ws,scope,kind,value.id,version)，current partial unique）。
- `TaxonomyValue`（camelCase）：`id`（字典 key 非 BSON _id）/`display_name`/`description`(default)/`aliases: Vec`(default)/`status`("active"/"deprecated")/H6 三字段：`priority_weight: Option<i32>`(None→planner 回落内置权重)/`is_terminal`(default false，替代 TERMINAL_STAGES 常量)/`is_reactivation_target`(default false，与 is_terminal 正交 L3691-3697)。**手写 Serialize（L3719-3740）额外写出派生字段 `identityClaims`**（= canonical id 首位 + aliases 去重，taxonomy_identity_claims L3703-3717）——供 `uniq_sys_tax_ws_scope_kind_active_identity` multikey partial unique（current+active）在 DB 层拒绝 alias↔alias/alias↔canonical 撞名；序列化侧派生保证所有 typed 写入方（seed/create/merge/publish）同一不变量。注意 Deserialize 是 derive 的（L3670），identityClaims 键在读回时被忽略（无对应字段）。round-trip 测试锁定 camelCase 键（L6732-6782）。
- `TaxonomyCandidate`：`raw_value`/`evidence`/`confidence: i32`(default)/`first_seen_at`/`last_seen_at`/`occurrences`(default)/`status`(pending/approved/rejected)/`reviewed_at`/`reviewed_by`/`suggested_display_name: Option`(default，流 C AI 建议中文名，运行时候选路径写 None L3769-3774)。(ws,scope,kind,raw_value) unique 幂等。红线：unreviewed 候选不阻塞 run（L3745-3746）。

### 2.24 `RelationshipTypeSuggestion`/`SuspectedDealSignal`（L3781-3847）
同构的"LLM 建议→人审→生效"保守闭环表（snake_case）：`contact_id`（字符串 hex）/建议值/`evidence`/`confidence`/`status`(pending/approved/rejected)/`occurrences`/首末见时间/审核两字段。关键索引语义：**仅 status=pending 参与 (ws,contact_id) partial unique**——终态历史不占槽，二次证据可开新审核周期（RelationshipTypeSuggestion 天生如此 L3786-3788；SuspectedDealSignal 由全量 unique 改为 partial，indexes.rs:2567-2590 + 注释 2531-2537）。SuspectedDealSignal 是 F23"AI 永不直写 outcome_events"红线载体（L3817-3823）：LLM suspected_deal 弱信号 upsert pending，运营 approve 才 add_outcome_event_inner(verification=staff_confirmed)。round-trip 测试 L6832-6874、L7548-7576。

### 2.25 `LlmCallLog`（L3849-3894）→ `llm_call_logs`
snake_case。`run_mode`(default，live/shadow/诊断模式，legacy 空)；`prompt_key`/`model`/`status`；延迟三段（L3864-3871）：`latency_ms`(端到端=排队+上游)/`queue_wait_ms`(default，进程内并发闸等待)/`provider_latency_ms`(default，上游+重试)；`priority`(default，foreground/background)；token 五字段：prompt/completion/total/`prompt_cache_hit_tokens`/`prompt_cache_miss_tokens`；`usage_known: bool`(default，L3880-3885 仅上游返回 usage 或构造性已知 cache-hit=0 才 true；API 投影额外认 legacy 非零 token 行)；`retry_count: i32`(default)；`final_status: Option`(success|failed|json_error|cache_hit)。

### 2.26 Guide/Management 结构（L3896-4134）
- `GuideFrozenPlan`（snake_case）：OCC 基线 `contact_updated_at`/`memory_updated_at`+`memory_insert: Option<Document>`(L3916-3919 Preview 无持久 memory 行时的冻结插入基线，None 保持 legacy OCC 契约)；`playbook_id`/`playbook_version`/`domain_config_id`/`domain_version`/`domain_updated_at`；三组 set+timestamp_fields（contact/memory/playbook）；`domain_runtime_parameters: Option<Document>`；`applied_fields`/`skipped_fields: Vec<GuideSkippedField>`/`authoritative_changes: Vec<GuideAuthoritativeChange>`(camelCase，before/after Bson)/`playbook_affected_contacts: i64`。
- `UserOperationGuidePreview`：`instruction`/`mode`/`status`/`summary`/`impact_scope`/`scope_reason`/`readable_changes`/`health_scores`/`suggested_changes`/`risk_warnings`；Protocol v3：`frozen_plan: Option`(legacy 预览可读不可 apply L3991-3993)/`candidate_hash`/`apply_receipt: Option<GuideApplyReceipt>`(camelCase，与业务变更同事务写，重放直接返回回执 L3997-4000)。
- `AgentCommandRun`：`operator_message`/`status`(7 态闭集 L4063-4071：pending_confirmation/running/dry_run/succeeded/failed/execution_unknown/canceled)/`plan: Option<Document>`+`plan_hash: Option`(L4043-4045 冻结 ManagementPlan SHA-256，legacy 无绑定不可 confirm 须重 plan)/`execution_token`(租约)/`execution_started_at`/`confirmed_by`/`confirmed_at`/`prompt_versions`。
- `AgentToolCall`：`command_run_id`/`intent_key: Option`(L4090-4093 partial unique 防重试/恢复双写副作用)/`call_index`/`tool_name`/`arguments`/`status`(8 态闭集 L4113-4122，`executed_unverified`=已执行结果无法核实、`accepted`=持久受理但送达异步——"诚实优于好看"纪律)/`response`/`error`/`execution_started_at`/`finalized_at`。

### 2.27 `AgentOutcomeMetric`/`EvaluationScenario`（L4136-4207）
- `AgentOutcomeMetric`：`_id="{ws}:{acct}:{horizon}:{date}"`（m004 从 3 段升 4 段；幂等聚合重跑覆盖同 _id）。四指标全 `Option<f64>`(default)：`reply_rate`/`conversation_depth`/`ai_hold_cleared_rate`(**alias="human_handoff_success_rate"** L4156-4159——旧字段名违反全自治定位退役，读兼容写新名)/`agent_block_rate`——None=无数据≠0（测试 L6521-6581）；`daily_run_count`/`daily_run_token_total`；fencing：`source_task_id: Option`+`source_task_claim_generation: i64`(更旧 owner 不得覆盖更新结果 L4167-4172)。BSON snake_case，前端 JSON 由 outcome_metric_json 单独 camelCase（L6541-6543 测试注释）。
- `EvaluationScenario`：`scenario_id`（(ws,scenario_id) unique）/`contact_seed: Document`/`inbound_messages: Vec<String>`/`ground_truth: Document`/`tags`/`status`(default "active")。

### 2.28 `ApiContact` 投影契约（L4323-4481）
`tags` = manual_tags（在前保序）+ confirmed_tags.value 去重追加（L4398-4407）；所有时间字段 RFC3339 string（dt_to_string）；`stagnation_dimension/value/updated_at` 三字段投影与 planner 同源（L4352-4357）：`apply_stagnation_dimension(dim)` 从 domain_attributes 取 `<dim>` 与 `<dim>_updated_at`，缺时钟回落 `customer_stage_updated_at`（与 planner::contact_stagnation_updated_at 一致，L4452-4481；测试 L7888-7939 断言容器级 updated_at 绝不冒充停滞时钟）；`last_inbound_preview` 由 list_contacts 单独查询填充，From 恒 None（L4372-4375）。

### 2.29 typed 模块核心契约（L4716-5600）
- `RuntimeParametersTyped`（camelCase，33 字段全 default fn）：完整默认值表见 §5.3。旧文档缺字段走 default（测试 L6102-6111）；已删除字段 knowledgeRoutingMode/knowledgeMaxToolLoops 残留静默忽略（L6170-6186）。
- `MemoryCardTyped`：`core_facts`/`recent_facts`/`deprecated_facts: Vec<MemoryFactRepr>`（写侧 cap 6/10/20，L4985-4987）+`extra: Document`(**flatten catch-all**，L4996-5002：承接 coreProfile/relationshipState/preferences/doNotDo/commitments/objections/openLoops/openQuestions/confirmedFacts/conflicts/source/version 等全部未声明顶层键，历史数据零丢失；曾因 typed 出同名字段与 flatten 冲突产生重复 BSON 键 bug，修复后只留 extra 一份 L4992-4995)。方法：`to_document`(失败回空防 panic)/`from_document`(失败回 Default)/`is_empty`/`has_plain_facts`/`auto_upgrade_plain_facts`(返回升级数，幂等，测试 L6437-6483)/`live_dimension_names`(去重保序，仅 core+recent 的 Structured 非空 dimension)。
- `MemoryFactRepr`（untagged）：`Plain(String)`（历史形态）|`Structured(MemoryFact)`。**`coreFacts` 必须继续反序列化 legacy `Vec<String>`——CLAUDE.md R11 红线**（测试 L6209-6233、L6401-6432 mixed 形态）。
- `MemoryFact`（camelCase）：`id`（UUIDv4，**身份锚——禁用 text 作 key**；Plain 升级 fresh UUID 防多次升级不同 id 失配 L5090-5094）/`text`(1..=500)/`evidence`(≤1000)/`confidence`(0..=10，Plain→7)/`importance`(0..=10，Plain→5)/`may_expire`/`deprecated_at`/`deprecation_reason`(≤200)/`dimension: Option`(⑨语义维度归类：同 dimension 冲突新值胜旧值自动 deprecated；None→按 text 去重旧行为 L5127-5133)/`source_message_ids: Vec<ObjectId>`(≤5，skip_if empty)/`source_run_id`/`created_at`/`updated_at`(default epoch)/`extra`(flatten)。手写 Default（bson DateTime 无 Default，L5151-5175）。`validate()` 返回违规列表（测试 L6289-6359）。
- `CommitmentRepr`（untagged）/`CommitmentEntry`（camelCase）：`id`(UUIDv4=Planner emit 幂等键 agent_events.details.commitment_id L5402)/`text`(1..=500)/`due_at: Option`/`created_at`/`extra`(flatten)。Plain 无 due_at→scan_commitments 跳过（L5353-5356）。
- `IntentTrajectoryEntry`（camelCase）：`turn_index`/`intent`（reaction 归一 outcomeStatus）/`objection_type: Option`（DEFAULT 销售旧字段）/`dimensions: BTreeMap<String,String>`(H17 通用容器，空 skip；legacy 兼容测试 L6093-6099)/`recorded_at`(default epoch)。MAX_ITEMS=50 滑窗。
- `OperationStateMachineTyped`/`OperationStateTyped`（camelCase）：`key`/`name`/`goal`/`allowed_actions`/`allowed_from`/`allow_from_any`/`initial`(H13 初始态标志，DEFAULT 仅 new_contact)/`forbids_proactive`(DEFAULT 仅 cooldown)/`advance_signals`/`cooldown_signals`/`risk_rules`/`success_criteria`。默认机器 round-trip 测试锁定标志位分布（L6586-6642）。

### 2.30 自演化五表（L5618-5837）
- `Experiment`：`experiment_id`(unique)/`status`(5 态 L5635)/`window_hours`/`cohort_threshold_run_ids`/`cohort_prompt_run_ids: Vec<ObjectId>`/预算两计数/proposals 两计数。
- `Proposal`：`proposal_kind`("threshold"/"prompt")；`status` 6 态（L5666-5673：pending_eval/evaluating/eligible_for_release/rejected_below_threshold/released/rolled_back；prompt 类 eligible=证据就绪待管理员，绝不自动放行）；threshold 类：`gate_key`/`current_value`/`proposed_value`/`cohort_notes`；prompt 类：`proposed_template_key`/`proposed_section`(soul|system_contract|policy|operator_instruction)/`diff_summary`/`diff_snippet`/`critic_reasoning`/`expected_improvement_on`/`risk_note`；**不可变身份**：`base_revision: Option`(L5694-5697 评估基线，legacy 缺失刻意不可 release)/`released_revision`/`previous_prompt_version`；评估：`eval_metrics: Document`/`eval_replays_completed`/`failed`/`significance_passed: Option<bool>`/`failure_reason`；release/rollback 审计四字段。
- `ShadowReplay`：`proposal_id`/`source_run_id`；`original_final_review_status`+`original_5gate_hit: Document`(G4 修复：此前只记 new 侧致显著性假基线恒 false L5735-5739)+`original_self_critique_addressed`；new 侧五字段+`similarity_to_original_text: f64`。
- `ThresholdOverride`：`gate_key`/`value: f64`/`source_proposal_id`（(ws,acct,source_proposal_id) partial unique 一提案一工件）/`base_revision`/`released_revision`/`current_version`(partial unique 每 scoped gate 至多一条 current)/release/rollback 审计。
- `ThresholdOverrideAudit`（append-only 永不更新）：`action`∈{released,rolled_back,auto_released}；`decided_by`∈{"admin:<id>","evolution_auto","evolution_release","evolution_rollback"}（L5790-5795）；`previous_value`/`new_value`/`hit_rate_observed`/`significance_metrics: Option`。
- `PostReleaseReview`：`scheduled_at`(+24h)/`completed`/`actual_send_success_rate_delta`/`actual_5gate_hit_delta`。

### 2.31 知识日报三表 + 运营记忆 + LLM 配置（L5839-6085）
- `KnowledgeDigestCard`（camelCase 嵌入）：`card_id: ObjectId`（持久 id）；`kind` 7 种（L5852-5853）；`title`≤60/`summary`≤200（后端截断，超长丢卡）；`target_refs: Vec<Document>`（L5859-5870：kind 枚举以 prompt 为准 chunk/pack/proposal，**不查库做外键校验**，ref 指向已删对象可能，下游 fail-soft）；`suggested_action` 6 种；`severity` 3 档；`metric: Option<Document>`。
- `KnowledgeDailyReport`：`report_date`("YYYY-MM-DD" 运营时区，(ws,acct,report_date) unique)；`generated_by`("worker"/"manual")；`status`(ok/partial/failed)；`error_kind`（与 AppError::LlmUnavailable.kind 同源）；`budget_snapshot`/`cards`/`dismissed_card_ids`/`prompt_versions`；防迟到覆盖（L5912-5918）：`attempt_generation`(尝试开始时分配，finalize 须匹配)+`current_generation`(当前可见成功快照代次)+latest_attempt 四字段+`last_success_at`。
- `KnowledgeChatTask`：`session_id`（worker 按 sessionId 串行）；`owner_admin_id: Option`(L5946-5949 新任务必带，历史缺失不被新管理员入口枚举)；`cards` 快照；`dispatch_binding: Option<Document>`(SR-125 服务端绑定 report/card hash+候选 hash)；`planned_steps`（服务端从已绑定 cards 重建，LLM/客户端不得覆盖 action/target L5958-5960）/`completed_steps`（step status 闭集 committed|noop|needs_manual|failed，L5962-5963 不得用 Rust Ok 冒充成功）/`step_intents`(按 stepId 持久化 mutation payload，reclaimed worker 重放同一意图 L5966-5969)；`status` 5 态闭集 ALLOWED_TASK_STATUS（"finished"已更名"completed" P2-12）；claim 五件套+`heartbeat_at`。
- `KnowledgeOperatorMemory`：`operator_id`/`kind`(preference/rejection/context)/`content`/`last_used_at`/`expires_at`(TTL)；软撤销三字段（revoked 行可审计查询但**绝不注入 prompt** L6012-6014）。
- `LlmProviderConfig`（camelCase）：`provider_id`(业务 slug，(workspaceId,providerId) unique)/`format`("openai"→/chat/completions；"anthropic"→/v1/messages L6023-6025)/`base_url`/`api_key`(Debug 掩码)/`model`/`is_active`(每 ws 至多一条 partial unique——跨副本权威指针)/`timeout_seconds`/`max_retries`/`retry_base_ms`/`supports_vision`(default false)/`is_vision_active`(default false，每 ws 至多一条 partial unique；文字主模型与视觉副模型可为两条记录，正交 L6052-6057)。

---

## 3. Database accessor 与索引全表

### 3.1 `Database` 结构与连接（src/db/mod.rs）
- `Database { db, client, cache_identity: u64, cache_lifetime: Arc<()> }`（L38-49）：cache_identity 进程内自增（L51, L62），隔离同 workspace id 不同连接的运行时缓存；`cache_lifetime()` 返回 Weak，注册表仅在 Database 存活时保强缓存（L97-101）。
- `connect` **无副作用**（不建索引不跑迁移，L1-10 模块注释）；调用顺序红线：`migrations::run` 先、`ensure_indexes` 后（部分迁移重建集合/收敛指针后 partial unique 才能建）。
- `client()`（L73-79）暴露给 release.rs 起跨集合事务；`raw()`（L85-89）供集成测试写原始 BSON。

### 3.2 accessor → 集合全表（src/db/mod.rs，行号=accessor 定义处）
| accessor | 集合 | typed 模型 | 行号 |
|---|---|---|---|
| accounts | wechat_accounts | WechatAccount | 81 |
| contacts | contacts | Contact | 103 |
| messages | conversation_messages | ConversationMessage | 107 |
| tasks | agent_tasks | AgentTask | 111 |
| import_jobs | import_jobs | ImportJob | 117 |
| events | agent_events | AgentEvent | 121 |
| system_incidents | system_incidents | SystemIncident | 125 |
| behavior_signals | behavior_signals | BehaviorSignal | 133 |
| behavior_signal_metrics | behavior_signal_metrics | BehaviorSignalMetric | 139 |
| mcp_logs | mcp_call_logs | McpCallLog | 143 |
| content_assets | content_assets | ContentAsset | 147 |
| agent_souls | agent_souls | AgentSoul | 151 |
| operation_playbooks | operation_playbooks | OperationPlaybook | 155 |
| operation_domain_configs | operation_domain_configs | OperationDomainConfig | 159 |
| operation_state_policies | operation_state_policies | OperationStatePolicy | 166 |
| prompt_templates | prompt_templates | PromptTemplate | 170 |
| operating_memories | operating_memories | OperatingMemory | 174 |
| operation_knowledge_documents | operation_knowledge_documents | OperationKnowledgeDocument | 178 |
| operation_knowledge_chunks | operation_knowledge_chunks | OperationKnowledgeChunk | 182 |
| knowledge_usage_logs | knowledge_usage_logs | KnowledgeUsageLog | 186 |
| knowledge_chat_turns | knowledge_chat_turns | KnowledgeChatTurn | 190 |
| knowledge_chat_session_seqs | knowledge_chat_session_seqs | Document（`_id="{ws}|{session}"`+seq 原子自增，L194-200） | 198 |
| knowledge_daily_reports | knowledge_daily_reports | KnowledgeDailyReport | 202 |
| knowledge_chat_tasks | knowledge_chat_tasks | KnowledgeChatTask | 206 |
| knowledge_operator_memory | knowledge_operator_memory | KnowledgeOperatorMemory | 210 |
| decision_reviews | agent_decision_reviews | AgentDecisionReview | 214 |
| agent_run_logs | agent_run_logs | AgentRunLog | 218 |
| agent_principal_escalations | agent_principal_escalations | AgentPrincipalEscalation | 222 |
| referral_cards | referral_cards | ReferralCard | 226 |
| agent_send_ledger | agent_send_ledger | AgentSendLedger | 230 |
| llm_call_logs | llm_call_logs | LlmCallLog | 234 |
| memory_candidates | memory_candidates | MemoryCandidate | 238 |
| user_operation_guide_previews | user_operation_guide_previews | UserOperationGuidePreview | 242 |
| management_sessions | management_agent_sessions | ManagementAgentSession | 246 |
| management_messages | management_agent_messages | ManagementAgentMessage | 250 |
| command_runs | agent_command_runs | AgentCommandRun | 254 |
| tool_calls | agent_tool_calls | AgentToolCall | 258 |
| outcome_metrics | agent_outcome_metrics | AgentOutcomeMetric | 262 |
| evaluation_scenarios | evaluation_scenarios | EvaluationScenario | 266 |
| migrations | migrations | MigrationRecord | 270 |
| collection_agent_send_outbox | agent_send_outbox | OutboxEntry | 281 |
| collection_system_taxonomies | system_taxonomies | TaxonomyEntry | 286 |
| collection_taxonomy_candidates | taxonomy_candidates | TaxonomyCandidate | 291 |
| collection_relationship_type_suggestions | relationship_type_suggestions | RelationshipTypeSuggestion | 298 |
| collection_suspected_deal_signals | suspected_deal_signals | SuspectedDealSignal | 308 |
| experiments | experiments | Experiment | 319 |
| proposals | proposals | Proposal | 324 |
| shadow_replays | shadow_replays | ShadowReplay | 329 |
| threshold_overrides | threshold_overrides | ThresholdOverride | 334 |
| threshold_overrides_audit | threshold_overrides_audit | ThresholdOverrideAudit | 341 |
| post_release_reviews | post_release_reviews | PostReleaseReview | 348 |
| evolution_runtime_flags | evolution_runtime_flags | EvolutionRuntimeFlag | 356 |
| llm_provider_configs | llm_provider_configs | LlmProviderConfig | 364 |
| chunk_revisions | chunk_revisions | ChunkRevision | 380 |
| knowledge_gap_signals | knowledge_gap_signals | KnowledgeGapSignal | 387 |
| domain_schemas | domain_schemas | DomainSchema | 392 |
| domain_profiles | domain_profiles | DomainProfile | 399 |
| catalog_rebuild_jobs | catalog_rebuild_jobs | CatalogRebuildJob | 405 |
| ingest_sources | ingest_sources | IngestSource | 412 |
| products | products | Product | 420 |
| campaigns | campaigns | Campaign | 425 |
| campaign_sends | campaign_sends | CampaignSend | 430 |
| roster_snapshots | roster_snapshots | RosterSnapshot | 435 |
| background_worker_controls | background_worker_controls | Document（`_id`=worker 名，熔断状态机字段演进中，刻意不 typed，L439-447） | 445 |
| background_worker_leases | background_worker_leases | Document（`_id="<kind>::<ws>"`，token/locked_until 按 _id CAS，L449-455） | 453 |

**无 typed accessor、仅 raw 访问的集合**（indexes.rs / migrations 中出现）：`webhook_rate_limit_windows`(indexes.rs:736)、`import_job_segments`(909)、`proactive_daily_quotas`(869)、`admin_users`(3172)、`admin_sessions`(3188)、`auth_security_events`(3223)、`reviewer_stats`(3329)、`deal_attribution_stats`(3349)、`lessons_learned`(3369)、`projection_observations`(2543-2545，集合名常量 src/agent/projection_observations.rs:14)、`configuration_generations`(config_generation.rs:39)、`operation_knowledge_items`（legacy，m010/m011/m014 触达，typed accessor 已删）。

### 3.3 索引全表（src/db/indexes.rs；`ensure_all` L705-1469 + 五个子 helper）
> 幂等机制：`ensure_index_or_equivalent_name`（L601-662）对同 keys 索引做**语义等价比较**（忽略 name 与 v，L575-585），等价即复用历史名，不等价即 bail 启动失败；NamespaceNotFound 视为新集合直接创建。`retire_indexes_with_keys`（L664-681）按精确 keys 匹配退役旧索引。

#### wechat_accounts
- `(workspace_id, account_id)` unique（L706-714）
- `app_id` partial unique（filter `$type:"string"`，名 uniq_wechat_accounts_app_id；启动先 best-effort drop 旧非唯一 `app_id_1`，重复 app_id 显式炸启动防 webhook 路由不确定，L715-733）

#### webhook_rate_limit_windows（raw）
- `expires_at` TTL=0（L736-750；`_id` 即配额身份，TTL 清理最终一致、不参与授权）

#### contacts
- `(workspace_id, account_id, wxid)` unique（L751-759）
- `(workspace_id, account_id, outcome_events.productRef.productId)` multikey（L760-775；**混合大小写路径**：外层 snake、内层 camel）

#### conversation_messages
- `(ws, acct, contact_wxid, created_at desc)`（L776-783）
- `(ws, acct, direction, contact_wxid, created_at desc, _id desc)`（列表批量取每人最新入站，L784-800）
- `(ws, acct, message_id)` sparse+unique（L801-809）
- `(ws, acct, dedupe_key)` partial unique（`$type:"string"`，L810-823）
- `inbound_handoff_pending_idx`：`(handoff_status, created_at, _id)` partial（direction=inbound+handoff_status=pending，SR-177 恢复扫描，L379-392, 824-828）

#### agent_tasks
- `(status, run_at)`（L829-836）；`(ws, acct, contact_wxid, kind, status)`（L837-850）
- `uniq_outcome_aggregation_ws_kind_account_content`：`(ws, kind, acct, content)` partial unique（kind=outcome_aggregation，防 TOCTOU 双写，L308-324, 857-859；m017 先去重、m033 退役旧无 ws 版本）
- `uniq_memory_active_task_key`：`(ws, acct, contact_wxid, active_task_key)` partial unique（`$type:"string"`，memory 归并单飞租约，终态原子移除键即出索引，L359-377, 863-865）

#### proactive_daily_quotas（raw）
- `expires_at` TTL=0（短命并发控制桶，非审计真相源，L326-336, 869-872）

#### import_jobs / import_job_segments
- `(workspace_id, status)`（L874-881）；`(status, claimed_at)`（孤儿重认领，L883-890）；`expires_at` TTL=0（终态 24h 清扫，L895-908）
- segments：`(job_id, segment_index)` unique + `expires_at` TTL=0（L16-38, 909-915）

#### agent_events
- `(ws, acct, contact_wxid, created_at desc)`（L917-929）
- `uniq_events_workspace_dedupe_key`：`(ws, dedupe_key)` partial unique（`$type:"string"`，L930-948）

#### behavior_signals / behavior_signal_metrics
- `uniq_behavior_signals_ws_account_dedupe_key`：`(ws, acct, dedupe_key)` partial unique（**必须 `$type` 不能 `$in`——Error 67 会炸 ensure_indexes**，L527-540, 949-956）
- `(ws, acct, contact_wxid, observed_at desc)`（L542-551）；建完两条后才退役 m039 前的 workspace-only 旧索引（L683-692, 960-963）
- metrics：`(workspace_id, date desc)`（`_id` 天然唯一无需 unique，L964-974）

#### content_assets / referral_cards / agent_send_ledger / agent_souls / operation_playbooks / prompt_templates
- assets：`(ws, acct, kind, updated_at desc)`；`(ws, sendable, review_status)`；`file_sha256`（L975-1000）
- referral_cards：`(ws, acct, enabled, review_status)`（L1002-1014）
- send_ledger：`(ws, acct, contact_wxid, sent_at desc)`（L505-514）；`(ws, acct, send_kind, target_id)`（L516-525）；`uniq_send_ledger_outbox_id`：`outbox_id` partial unique（`$type:"objectId"`，一次送达一条台账，历史无锚行不入约束，L467-478, 1024-1026）；`(outcome_evaluated_at, sent_at)`（回扫全局扫描形状，L1027-1037）
- souls：`(ws, agent_kind, status, version desc)`；`uniq_agent_soul_ws_kind_version` unique；`uniq_agent_soul_published_ws_kind` partial unique（status=published，L480-503, 1038-1051）
- playbooks：`(ws, acct, is_default, updated_at desc)`；`uniq_operation_playbook_default_per_account`：`(ws, acct)` partial unique（is_default=true，L102-113, 1052-1064）
- prompt_templates：`(ws, prompt_key, status, version desc)`（L1065-1072）；另在 evolution helper：`(ws, prompt_key, current_version)`（L2811-2818）；`uniq_prompt_current_pointer`：`(ws, prompt_key)` partial unique（current_version=true，L2819-2833）；`uniq_prompt_artifact_per_proposal`：`(ws, source_proposal_id)` partial unique（`$type:"objectId"`，L2834-2850）；`(ws, prompt_key, version)` unique（L2851-2859）

#### ops 三表版本化（`ensure_ops_versioned_indexes` L2309-2462；先 drop 旧 2/3-key unique 防 H8 boot-brick，见 L1073-1080 注释）
- operation_domain_configs：`op_domain_ws_domain_version_unique`（(ws,domain,version) unique）+`uniq_op_domain_ws_domain_current`（(ws,domain) partial unique current_version=true）
- operation_state_policies：同构 (ws,domain,state_key,version) unique + current partial unique
- system_taxonomies：`sys_tax_ws_scope_kind_value_version_unique`（(ws,scope,kind,value.id,version) unique）+`uniq_sys_tax_ws_scope_kind_value_current`（(ws,scope,kind,value.id) partial unique current=true）+**`uniq_sys_tax_ws_scope_kind_active_identity`**（(ws,scope,kind,**value.identityClaims**) multikey partial unique，filter current_version=true+value.status="active"——SR-046 alias/canonical 同名空间提交时拒绝，L338-357, 2451-2456）；另 `sys_tax_scope_kind_status_idx`（(ws,scope,kind,value.status) 非唯一列表辅助，L2276-2295）

#### operating_memories / knowledge 文档与切片
- operating_memories：`(ws, acct, contact_wxid)` unique（L1081-1089）
- documents：`(ws, acct, domain, status, updated_at desc)`（L1090-1097）
- chunks：`(ws, acct, domain, status, priority desc, updated_at desc)`（L1098-1105）；`uniq_kchunks_lesson_promotion_source`：`(ws, provenance.source_doc_id)` partial unique（provenance.source="lesson_promotion"+source_doc_id string，L164-178, 1106-1108）；`(document_id, item_id, status)`（L1109-1116）；wiki 三条 sparse：`kchunks_wiki_type_idx`（(ws,wiki_type)）/`kchunks_valid_to_idx`（(ws,valid_to,status)）/`kchunks_dynamic_confidence_idx`（(ws,dynamic_confidence desc)）（L3127-3168）

#### knowledge_usage_logs / knowledge_chat_turns
- usage：`(ws, acct, contact_wxid, created_at desc)`+`created_at` **TTL=35d**（略大于回路① 30d 滑窗，防 feedback_worker 内存被无界日志拖垮，L1117-1143）
- turns：`kchat_turns_session_idx`（(ws,acct,session_id,turn_index)）/`kchat_turns_recent_idx`（(ws,acct,created_at desc)）（L1144-1169）

#### agent_decision_reviews
- `(ws, acct, contact_wxid, created_at desc)`（L1170-1177）
- `(ws, acct, contact_wxid, status, outcome_status)`（**不能用 $in partial filter——Error 67**；reaction claim 走前缀+等值，放弃体积优化换合法性，L1178-1197）
- `run_id`（非 unique，H11 回路① join，L1198-1206）
- post-decision 投影四条（raw 字段）：`decision_post_projection_idx`（(post_decision_status, post_decision_next_retry_at, post_decision_locked_until, created_at, _id)，L2151-2169）；`decision_post_projection_claim_v2_idx`（前缀加 status，旧名保留滚动升级，L2172-2191）；`decision_post_projection_scrub_idx`（(post_decision_status, post_decision_scrub_at)——**刻意非 TTL**：只字段擦洗，review 本体永久审计，L2194-2209）；`decision_post_projection_order_fence_idx`（(ws,acct,contact_wxid,post_decision_profile_done,created_at desc,_id desc)，顺序栅栏，L2212-2231）

#### agent_run_logs
- `(ws, acct, contact_wxid, created_at desc)`；`run_id` unique；`agent_run_log_outbox_enqueuing_idx`（(status,created_at,_id) partial status=outbox_enqueuing，L296-306, 1224-1226）；W6 三条监控：`(account_id, lifecycle, created_at desc)`/`(account_id, final_review_status, created_at desc)`/`(account_id, autonomy_mode, created_at desc)`（L1240-1263；W0 曾规划 started_at 但 W1 落地只写 created_at，残留空 started_at 索引可手工 drop，L1232-1239）

#### llm_call_logs / mcp_call_logs / memory_candidates / guide / management
- llm：`(ws, acct, prompt_key, created_at desc)`；`(run_id, created_at desc)`（L1264-1281）
- mcp：`(ws, acct, tool_name, created_at desc)`（crash-recovery post-hoc 核对热路径，L1282-1293）
- memory_candidates：`(ws, acct, contact_wxid, status, created_at desc)`；`uniq_memory_projection_key`（(ws,acct,contact_wxid,projection_key) partial unique `$type:"string"`，L1294-1322）
- guide previews：`(ws, acct, contact_id, created_at desc)`（L1323-1332）
- management messages：`(session_id, created_at)`；command_runs：`(ws, acct, created_at desc)`+`management_stale_execution`（(status, execution_started_at)，L40-49, 1341-1351）；tool_calls：`(command_run_id, created_at)`+`uniq_management_tool_intent`（(ws,acct,intent_key) partial unique string，L262-277, 1352-1362）

#### TTL 组（可调）
- outcome_metrics：`created_at` TTL=`OUTCOME_METRICS_TTL_DAYS`（默认 90 天，L1363-1380）+`(ws, acct, horizon, date desc)`（L1381-1388）
- 诊断日志 TTL=`DIAGNOSTIC_LOG_TTL_DAYS`（默认 30 天，0=禁用）：llm_call_logs/agent_run_logs/mcp_call_logs 各一条 `created_at` TTL（L1389-1427；只清诊断日志不动业务事实表）

#### evaluation_scenarios
- `(ws, scenario_id)` unique（L1428-1437）

#### agent_send_outbox（`ensure_agent_send_outbox_indexes` L2087-2237）
- `(account_id, status, next_retry_at)`（dispatcher 扫描）；`uniq_outbox_ws_account_idempotency`（(ws,acct,idempotency_key) unique，L246-260）；`(status, locked_until)`（过期 lease 恢复）；`(source_event_id, contact_wxid)`（链路追溯）；`(ws, acct, status, sent_at desc)`（账号级发送间隔闸防内存 SORT，L2115-2125）；`outbox_delivery_finalize_pending_idx`（(status, delivery_finalize_pending, updated_at, _id) partial sent+true，L196-214）；`outbox_priority_claim_idx`（(status, next_retry_at, delivery_priority desc, created_at, run_sequence, _id)，L2129-2149）；`outbox_contact_proactive_touch_idx`（(ws,acct,contact_wxid,source_kind,status)，B1 每日主动触达配额 precheck 热路径，**刻意无 partial filter**——闭集扩展防静默漏索引，L216-244, 1749-1773）

#### system_incidents / taxonomy_candidates / relationship_type_suggestions / suspected_deal_signals / projection_observations
- incidents：`uniq_system_incident_identity`（(ws,incident_key) unique）+`system_incident_reconcile`（(status, updated_at)，L2239-2268）
- candidates：`tax_candidate_ws_scope_kind_status_idx`+`tax_candidate_ws_scope_kind_raw_unique`（(ws,scope,kind,raw_value) unique 幂等，L2469-2503）
- suggestions：`uniq_relationship_pending_ws_contact`（(ws,contact_id) partial unique status=pending，L139-150）+`(ws,status)`（L2514-2527）
- suspected：先 drop 旧全量 unique `workspace_id_1_contact_id_1`（code 85 IndexOptionsConflict 防护），再建 `uniq_suspected_deal_pending_ws_contact`（partial unique pending）+`(ws,status)`（L2567-2599）
- projection_observations：`uniq_projection_observation_entity_run`（(ws,entity_type,entity_id,run_id) unique，L2543-2564）

#### evolution + 其余（`ensure_evolution_indexes` L2618-3444——**注意：函数名叫 evolution，实际包含知识日报/wiki/auth/ingest/统计/lessons/请示台账的全部索引**，见 §6 疑点 4）
- experiments：`(ws,acct,started_at desc)`/`(ws,acct,experiment_id)`/`experiment_id` unique（L2619-2648）
- proposals：`(ws,acct,status,created_at desc)`/`(ws,acct,experiment_id)`（L2650-2675）
- shadow_replays：`(ws,acct,proposal_id)`/`(ws,acct,started_at desc)`（L2677-2701）
- threshold_overrides：`(ws,acct,gate_key,released_at desc)`（resolve_thresholds 核心路径）/`uniq_threshold_current_per_scoped_gate`（(ws,acct,gate_key) partial unique current=true）/`uniq_threshold_artifact_per_proposal`（(ws,acct,source_proposal_id) partial unique objectId，L2703-2756）
- threshold_overrides_audit：`(ws,acct,gate_key,decided_at desc)`（append-only 无 unique，L2758-2773）
- post_release_reviews：`(ws,acct,scheduled_at,completed)`/`uniq_post_release_review_protocol_v1`（(ws,acct,proposal_id) partial unique protocol_version=1，L2775-2807）
- knowledge_daily_reports：`(ws,acct,report_date desc)` unique（一天一份，L2866-2878）
- knowledge_chat_tasks：`(status,locked_until,created_at)`/`(session_id,status)`/`(ws,created_at desc)`（L2884-2907）
- knowledge_operator_memory：`(ws,acct,operator_id,last_used_at desc)`+`kop_memory_expires_ttl`（expires_at TTL=0，L2909-2943）
- chunk_revisions：`chunk_revisions_ws_chunk_rev_idx`（(ws,chunk_id,revision_id desc)）/`chunk_revisions_ws_created_at_idx`（(ws,created_at desc)）经 ensure_index_or_equivalent_name 建，后退役 m039 前无 ws 旧索引（L553-573, 2959-2966）
- knowledge_gap_signals：`gap_signals_status_kind_idx`（(ws,status,kind)）/`gap_signals_created_at_idx`/`gap_signals_signal_id_unique`/`uniq_gap_signals_pending_ws_dedup`（(ws,dedup_key) partial unique pending+string，L180-194）/`gap_signals_kind_status_created_idx`（(ws,kind,status,created_at desc) LintView，L2967-3025）
- domain_schemas：drop 旧非唯一两条→`domain_schemas_ws_id_version_unique`（(ws,schema_id,version) unique）+`domain_schemas_ws_active_unique`（(ws) partial unique is_active=true，L3026-3062）
- domain_profiles：drop 旧两条→version unique（(ws,profile_id,version)）+current partial unique（(ws,profile_id) current=true）+active partial unique（(ws) is_active=true）（L64-100, 3063-3077）
- catalog_rebuild_jobs：drop 旧→`catalog_jobs_retry_claim_idx`（(status,next_retry_at,target_generation,queued_at)）/`catalog_jobs_lease_reclaim_idx`（(status,locked_until)）/`catalog_jobs_job_id_unique`（L3078-3126）
- admin_users：`username` unique；admin_sessions：`session_id` unique+`expires_at` TTL=0；auth_security_events：`created_at desc`/`expires_at` TTL=0/`(client_fingerprint,created_at desc)`/`(target_fingerprint,created_at desc)`（L3170-3278）
- ingest_sources：`(ws,kind,status)`/`(status,locked_until,last_fetched_at)`/`source_id` unique（L3280-3324）
- reviewer_stats：`stat_id` unique（`<ws>::reviewer`）；deal_attribution_stats：`stat_id` unique（L3326-3363）
- lessons_learned：`(ws,updated_at desc)`+`uniq_lessons_learned_ws_lesson`（(ws,lesson_id) unique，L152-162, 3365-3386）
- agent_principal_escalations：`idx_principal_escalation_ws_status_contact`（(ws,status,contact_wxid)）/`uniq_principal_escalation_short_code`/`principal_relay_pending_idx`（(relay_state,resolved_at,_id) partial resolved+pending，L394-407）/`principal_card_delivery_reconcile_v2_idx`（(status,protocol.delivery_state,_id) partial pending；v1 用 $in 非法已 drop，L409-425, 3419-3428）/`principal_card_timeout_idx`（(status,protocol.delivery_state,last_pushed_at_ms,_id) partial pending+sent+timeoutHours number+last_pushed number，L427-447）/`uniq_principal_escalation_pending_ws_account_contact_category`（(ws,acct,contact_wxid,category) partial unique pending——防 follow-up worker 与 webhook debounce 并发 TOCTOU 双推卡，insert 侧捕 11000 静默跳过，L449-465, 3432-3441）

#### llm_provider_configs（`ensure_llm_provider_indexes` L1966-2018）
- 开机 best-effort drop 两条 snake_case 历史错索引（模型层是 camelCase→旧索引把所有文档当 (null,null) 重复键，L1967-1977）
- `(workspaceId, providerId)` unique；`(workspaceId, isActive)`；`llm_provider_one_active_per_workspace`（(workspaceId) partial unique isActive=true）；`validate_llm_vision_assignments`（启动校验：vision active 必须 supports_vision 且每 ws 一条，L115-137）+`uniq_llm_vision_active_workspace`（(workspaceId) partial unique isVisionActive=true，L51-62）

#### products / campaigns / roster_snapshots
- products：`(workspace_id, product_id)` unique+`(workspace_id, status)`（snake，L2020-2046）
- campaigns：`(workspaceId, accountId, status)`+`campaign_dispatch_recovery_idx`（(workspaceId,accountId,status,updatedAt) partial dispatching，L279-294）；campaign_sends：`(campaignId, contactWxid)` unique（camel，L2048-2078）
- roster_snapshots：`(workspace_id, account_id)` unique（L1458-1467）

### 3.4 config_generation（src/db/config_generation.rs，全 108 行）
跨副本轻量配置代次：集合 `configuration_generations`，`_id = "{namespace}\0{workspace_id}"`（L20-22，NUL 分隔）。namespace 常量 3 个（L16-18）：`domain_profile` / `taxonomy` / `llm_provider`。API：`read_generation`（无行→0；generation 容忍 i32/i64，L36-53）、`bump_generation`（findOneAndUpdate upsert `$inc generation +1` 返回 After，L55-72）、`bump_generation_with_session`（事务内 upsert $inc，与权威变更同事务，L74-90）。语义（L1-7）：运行时读方先比对小行再查本地缓存；直接手改 DB 依赖各缓存有界 TTL 恢复、非即时一致 API。

---

## 4. 迁移逐个记录（m001–m058）

### 4.0 迁移框架（src/db/migrations/mod.rs）
- **注册表**：`MIGRATIONS: &[Migration]` 58 条（L305-538），id 必须字典序递增（单测 L710-720）且唯一（L696-708）。id 与文件名不一一对应（如 m009 → id `2026_05_M4_001_prompt_template_versioned`；models.rs L7113-7120 有单测锁定 `2026_05_009... < 2026_05_M4_001...` 的字典序）。
- **执行协议**（`run_with_policy` L597-667）：逐条查 `migrations` 集合；记录存在且 status≠"blocked" → skip；生产态且命中审批闸且未批准 → upsert `{status:"blocked", reason, blocked_at}` 并 `$unset applied_at`，**继续下一条**（不炸启动）；执行成功 → upsert `{status:"applied", applied_at}` 并清 reason/blocked_at。blocked 行下次启动重试。
- **生产判定**（L571-579）：`APP_ENV` ∈ {development, dev, test, local}（大小写不敏感、trim）才豁免；缺失/空/production/staging/未知一律 **fail-closed 视为生产**（单测 L673-694）。
- **审批闸**（`production_approval_gate` L545-556）：两个 gate id——`2026_07_035_reconcile_legacy_cleanup` 守 {m011, m012, m014, m035}；`2026_07_036_reconcile_workspace_backfill` 守 {m016, m036}。`APPROVED_MIGRATIONS` env 按 `,;空格\n\t` 切分（L558-566）。
- **workspace 懒 seed**（`ensure_builtin_taxonomies_for_workspace` L150-293）：非启动路径——新 workspace 首次被访问时在**多文档事务**里（marker `workspace_taxonomy_template_v1:{ws}` + $setOnInsert upsert 全部内置字典模板 + bump taxonomy generation）种入 m006+m020+m021+m023+m024+m028 六组 seed；TransientTransactionError 重试 ≤5 次、UnknownTransactionCommitResult 按 marker 回查判定。
- **helpers.rs**（263 行）：`merge_allowed_from_defaults`（只补缺失 allowedFrom/allowFromAny=true，不覆盖运营值，L14-49）；`merge_state_flag_defaults`（只补缺失且默认为 true 的 initial/forbidsProactive，false 不落库，L57-83）；`upgrade_fact_array`+`structured_fact_doc`（字符串/无 id 文档→结构化 fact，fresh UUID+conf 7+imp 5，L88-141）。三组纯函数单测 L149-262。

### 4.1 逐条迁移
> 每条：**做什么 / 条件门 / 破坏性 / 幂等性**。"门=无"表示无 APPROVED_MIGRATIONS/APP_ENV 守卫（所有环境执行）。

| # | id（注册名） | 做什么 | 门 | 破坏性 | 幂等性 |
|---|---|---|---|---|---|
| m001 | 2026_05_001_split_last_message_at | contacts：last_message_at 回填 last_inbound_at（仅缺失/null 时），aggregation pipeline update_many | 无 | 无 | filter 二次不命中（m001:12-35） |
| m002 | 2026_05_002_split_active_facts | operating_memories：memory_card.activeFacts 拆 coreFacts(前6)+recentFacts(其余) 后 $unset activeFacts | 无 | 形态迁移（源字段删除但内容保留） | $exists 过滤，二次不命中 |
| m003 | 2026_05_003_state_machine_allowed_from | user_operations 状态机补 allowedFrom/allowFromAny 默认（helpers::merge_allowed_from_defaults，不覆盖运营值） | 无 | 无 | merge 无改动返回 false 跳写 |
| m004 | 2026_05_004_outcome_metrics_workspace_in_id | agent_outcome_metrics._id 3 段→4 段（insert新+delete旧，MongoDB 禁改 _id） | 无 | 低（重写 _id） | $expr split 段数≠4 才命中；upsert 新 id |
| m005 | 2026_05_005_memory_facts_to_structured | coreFacts/recentFacts 字符串元素→结构化 MemoryFact（fresh UUID+conf7+imp5），memory_card_version +1 | 无 | 无 | 元素含 id 即跳过 |
| m006 | 2026_05_006_taxonomy_seed | seed 销售域三 kind 字典：customer_stage 9 值（权重/终态/再激活标志与 planner 写死值逐字相等——单测 m006:432-485 锁死）、intent_level 3 值、objection_type 7 值；scope=global | 无 | 无 | $setOnInsert upsert（不覆盖运营改动）；`pub` 供测试重 seed（mod.rs:37-42） |
| m007 | 2026_05_007_outbox_indexes | no-op marker（outbox 索引契约纳入迁移轨道声明；真实索引在 ensure_indexes） | 无 | 无 | 纯日志 |
| m008 | 2026_05_008_contact_commitments_reshape | contacts.last_commitment(String)→commitments 单元素数组（id=$toString(_id)，due_at 留空）+$unset 旧字段 | 无 | 形态迁移 | filter commitments $exists:false |
| m009 | 2026_05_M4_001_prompt_template_versioned | prompt_templates 补多版本字段（current_version=false/previous_version=null/seeded_by="legacy_migration"），再按 (active 优先, version desc) 每 scope 选主置 current=true、其余 false；**先 promote 后 demote**（中断不留零 current，m009:76-78） | 无 | 无 | stage1 $exists 过滤；stage2 收敛性重跑 |
| m010 | 2026_05_V3_001_contact_custom_instructions_and_knowledge_tags | contacts 补 custom_agent_instructions:null；知识三集合（documents/items/chunks）补 product_tags:[]/business_topics:[] | 无 | 无 | $exists 过滤 |
| m011 | 2026_05_V3_002_drop_legacy_sales_collections | **清空**知识三集合全部文档（delete_many {}，集合保留） | **APPROVED: 2026_07_035** | **高（删数据）** | 全量删，二次 matched=0 |
| m012 | 2026_05_V3_003_drop_legacy_taxonomy_seed | **删** system_taxonomies 三销售 kind 全部行（其它 kind 不动） | **APPROVED: 2026_07_035** | **高** | 命中即删 |
| m013 | 2026_05_W4_001_seed_user_operation_state_policies | 按状态机每 state 建默认 policy；`derive_state_policy_lists`（forbidsProactive→allowed=[ack,silent,follow_up]/forbidden=[reply]；否则全允许，m013:23-44——**唯一真相**，publish 路径复用） | 无 | 无 | 已存在 (ws,domain,state_key) 跳过 |
| m014 | 2026_05_W4_002_drop_trigger_keywords | 知识三集合 $unset trigger_keywords（agent-first 渐进披露下线关键词快路径） | **APPROVED: 2026_07_035** | **中（删字段）** | $exists 过滤 |
| m015 | 2026_05_W4_003_ops_tables_active_versions | ops 三表补 E5-T1 四字段（version=1/current=true/previous=null/seeded_by） | 无 | 无 | current_version $exists:false |
| m016 | 2026_05_X1_001_backfill_workspace_id_on_legacy_rows | 52 个 snake 集合补 workspace_id + 3 个 camel 集合补 workspaceId = DEFAULT_WORKSPACE_ID；表宇宙由内嵌审计单测锁定（snake/camel 归类、两表不相交、admin_users 与 chunk_revisions 绝不回填，m016:132-267） | **APPROVED: 2026_07_036**（归属决策） | 中（所有权指派） | $exists 过滤 |
| m017 | 2026_05_X1_002_dedupe_outcome_aggregation_tasks | outcome_aggregation 任务按 (ws,acct,content) 分组，保留 created_at 最早一条删其余（为 partial unique 铺路） | 无 | 低（删重复 task，最差丢一条未跑副本、当日 tick 幂等重建） | 二次无重复组 |
| m018 | 2026_06_X2_001_backfill_domain_stage_from_legacy_top | contacts 顶层残留 customer_stage/intent_level/customer_stage_updated_at 合并进 domain_attributes（$mergeObjects 顶层为底、现有 domain 在后覆盖=只补缺失键）；**只回填不 $unset**（可逆，m018:8-11） | 无 | 无 | 二次 merge 结果不变 |
| m019 | 2026_06_X3_001_state_machine_state_flags | 状态机补 initial/forbidsProactive 标志（merge_state_flag_defaults，只补缺失 true） | 无 | 无 | 无改动跳写 |
| m020 | 2026_06_X4_001_seed_purchase_lifecycle | seed purchase_lifecycle 4 值字典（id 与 entitlements::G1_* 常量逐字一致）+ seed 示例 profile `sales-with-lifecycle-example`（**draft/inactive**，transaction_facts_enabled 显式 true，不改零配置行为） | 无 | 无 | $setOnInsert |
| m021 | 2026_06_X5_001_seed_churn_reason | seed churn_reason 6 值字典 + 给示例 profile $addToSet churn_reason 参与决策维度（**仅 is_active=false 草稿**；维度 doc 字段名由单测锁定防 serde 静默丢字段，m021:211-225） | 无 | 无 | $setOnInsert / $addToSet |
| m022 | 2026_06_X6_001_backfill_dormant_allow_from_any | 存量状态机回填 dormant_reactivation.allowFromAny=true（复用 merge_allowed_from_defaults；修 m003 已跑过的库不再吃新默认值的问题） | 无 | 无 | 只补缺失 |
| m023 | 2026_06_X7_001_seed_value_tier | seed value_tier 3 值字典（客观计算派生值，不经 LLM 通道、不加 profile 维度） | 无 | 无 | $setOnInsert |
| m024 | 2026_06_X8_001_seed_relationship_type | seed relationship_type 3 值字典（customer/peer/friend，admin 直写通道） | 无 | 无 | $setOnInsert |
| m025 | 2026_06_X9_001_backfill_ask_human_policy | 把 (principal_decider, high_risk_escalation_mode) 映射成 ask_human_policy（camelCase 键；escalateAiPolicyHold=all_mode 其余 true；**与 resolve_ask_human_policy None 路径字节等价**；不删旧字段） | 无 | 无 | ask_human_policy $exists:false |
| m026 | 2026_06_Y0_001_seed_sales_with_relationships | seed `sales_with_relationships` 示例 profile（per_relationship_operation_mode 三套范式，draft/inactive） | 无 | 无 | $setOnInsert |
| m027 | 2026_06_Y1_001_contact_trust_fields | contacts 物理回填 manual_tags:[]/confirmed_tags:[]/bayesian_signals:[]/tags_version:0（防 $push 到不存在数组报错） | 无 | 无 | manual_tags $exists:false |
| m028 | 2026_06_Y2_001_seed_conversation_mode | seed conversation_mode 4 值字典（canonical→中文 label，与 profile.conversation_modes 解耦） | 无 | 无 | $setOnInsert（注意 tracing 里 migration_id 写的是 "m028_seed_conversation_mode"，与注册 id 不一致——仅日志，见 §6） |
| m029 | 2026_07_029_cleanup_contact_identity | 三步清理身份污染：①删非真人 normal 行（is_operatable_person 判定 gh_/chatroom；managed 一律保留）②按 roster 快照回填 nickname/avatar_url（**仅同 ws/acct 内**，绝不用全局 wxid map，m029:57-59）③nickname=="Demi" 且 roster 未命中→$unset。跳过缺租户三元组的行（m036 获批前归属未知，m029:25-31） | 无（注释明示无条件对所有环境生效） | 中（删 normal 非真人行；不删消息） | 重跑安全（`pub` 供集成测试直调） |
| m030 | 2026_07_030_backfill_outcome_event_defaults | outcome_events/deal_events 数组元素 $map+$mergeObjects 补 verification="staff_confirmed"/eventKind="deal"（默认为底元素覆盖=只补缺失键）；**过滤器逐字段隔离绝不共享 $or**——否则给只有 outcome_events 的文档凭空造 deal_events:[]，serde alias 撞 duplicate_field 崩掉 typed 读（m030:47-56，单测锁死） | 无（注释明示语义保持型不加守卫，误加会致生产静默 SKIP） | 无 | 二次 merge 不变 |
| m031 | 2026_07_031_backfill_escalation_last_pushed_at | 台账补 last_pushed_at_ms = $toLong($created_at)（KD-05 骚扰门锚，历史行近似取创建时刻） | 无 | 无 | $exists:false |
| m032 | 2026_07_032_backfill_taxonomy_workspace | system_taxonomies/taxonomy_candidates 补 workspace_id=DEFAULT（语义保持，生产也必须执行，m032:7） | 无 | 无 | $exists:false |
| m033 | 2026_07_033_task_commit_indexes | 退役 pre-tenant 唯一索引 uniq_outcome_aggregation_kind_account_content（缺 ws 维度会跨租户误撞）；替代索引由 ensure_indexes 建 | 无 | 无（只删索引） | 索引不存在即 no-op |
| m034 | 2026_07_034_reconcile_review_fixes | 两件纠偏：①reconcile_prompt_currents（同 m009 算法但**先 demote 后 promote**——在 unique 索引已存在的升级库上避免 E11000，m034:53-56）②重跑 m029 | 无 | 同 m029 | 收敛性重跑 |
| m035 | 2026_07_035_reconcile_legacy_cleanup | **重跑 m011+m012+m014**（修旧守卫写假阳性 marker 的库）；自身就是审批 gate id | **APPROVED: 自身** | **高** | 同被包裹迁移 |
| m036 | 2026_07_036_reconcile_workspace_backfill | **重跑 m016**，随后重跑 m034（先派 ws 再纠偏，避免曾无租户的行永远躲在已 applied 的 m034 marker 后面，m036:11-15）；自身是 gate id | **APPROVED: 自身** | 中 | 同被包裹迁移 |
| m037 | 2026_07_037_materialize_admin_acl | admin_users 空/缺 workspaces → [default_ws]+default_workspace（在"空 ACL=无权限"语义切换前物化旧回落） | 无 | 无 | $or 过滤二次不命中 |
| m038 | 2026_07_038_scope_outbox_idempotency | 逐行把 pre-v2 idempotency_key 用运行时 helper（`scoped_outbox_idempotency_key`）重写为 ws/acct 前缀形态（CAS `_id+旧key`，丢 CAS 即炸）；然后退役 keys=={idempotency_key:1} 的 singleton 索引。字段缺失/空/前后空白 → **fail-closed 炸启动**（m038:78-94） | 无 | 低（重写键值） | is_scoped 判定跳过已迁移行 |
| m039 | 2026_07_039_scope_revision_and_behavior_identity | chunk_revisions 补 workspace_id（由父 chunk 推导；已有值与父不符→炸）；behavior_signals 补 account_id（仅 (ws,wxid) 在 contacts 恰一账号才推导，歧义→炸）。**两集合计划先全部构建再写**（半物化边界不暴露，m039:29-31）；写完先建新 scoped 索引再退旧（升级期两族索引不同时缺席，m039:90-92）。CAS matched≠1 炸 | 无 | 无 | plan 空即 no-op；`pub` 供测试 |
| m040 | 2026_07_040_evolution_release_protocol | threshold_overrides 全量校验（租户/gate/value 有限性/released_at/重复 proposal 工件/revision 冲突全 fail-closed），补 released_revision（threshold_revision(id,value) 确定性派生），再 demote-all→按 (released_at,_id) 最大者逐 scope promote current=true（CAS 失败炸） | 无 | 无 | 读侧校验全过才写；重跑收敛 |
| m041 | 2026_07_041_audit_send_ledger_anchors | **纯只读审计**：anchored 行必须 ObjectId outbox_id+canonical 租户；outbox_id 全局重复→炸（跨账号也拒，m041 测试 126-146）；legacy 无锚行放行不猜测。为 uniq_send_ledger_outbox_id 铺路 | 无 | 无（零写入） | 纯读 |
| m042 | 2026_07_042_agent_soul_versions | agent_souls 全量校验（ObjectId/canonical scope/version>0/status 闭集 draft\|published\|archived/同 scope 版本重复→炸）；每 scope 多 published 时保留 (version,_id) 最高者、其余 CAS archive（内容永不删，m042:5-6） | 无 | 低（改 status） | 校验先行；重跑收敛 |
| m043 | 2026_07_043_prompt_single_current | prompt_templates 全量校验（version 重复/闭集 status/current bool 缺失→炸）；每 scope 不变量：有 active 必须恰 1 current 且 current∈active（**不选赢家，不满足→炸**）；写侧仅两类修复：active 非 current→archive；draft-only 流的 legacy current 标记→清 false（m034 可能放上的） | 无 | 低 | 校验先行；CAS |
| m044 | 2026_07_044_domain_schema_single_active | **纯只读审计** domain_schemas：version>0 且 lineage 内唯一；每 ws 至多 1 active；违规炸（不选赢家） | 无 | 无 | 纯读 |
| m045 | 2026_07_045_relationship_review_cycles | 审计 pending 行 (ws,contact) 无重复+canonical；识别并退役旧全量 unique 索引（**仅精确 legacy 形态**：unique 且无 partial filter；替代形态跳过；不认识的形态→炸，m045:38-56） | 无 | 无（只删索引） | 收敛 |
| m046 | 2026_07_046_scope_principal_escalation_pending | 审计 pending 台账 (ws,acct,contact,category) 全租户身份无重复；退役缺 account_id 的旧 3-key partial unique（精确名+形态匹配，不识→炸） | 无 | 无（只删索引） | 收敛 |
| m047 | 2026_07_047_backfill_principal_awaiting_owners | 给 contacts.domain_attributes 回填 awaiting_principal_decision(=true)+awaiting_principal_decision_ids（$setUnion 合并，pipeline 处理 attrs 非 object/owners 非数组的兜底，m047:140-168）。owner 证据三源：pending 台账；resolved+relay∈{pending,enqueued}；legacy resolved 无 relay_state 且仍有 pre-Outbox 活跃 relay task（**outbox_enqueued 刻意不算**——旧 worker 可能已送达未终结，歧义不造永久标记，m047:8-11）。写前逐 key 校验 contact 恰一条 | 无 | 无 | $setUnion 幂等 |
| m048 | 2026_07_048_ops_single_current | ops 三表（SPECS 声明 scope 字段，含 value.id 点路径）全量解析校验（canonical/version>0/scope 内版本唯一→炸）；恰 1 current 的 scope 保留；0/多 current → 确定性选 (version,_id) 最大者 promote+其余 demote（SR-008；为 partial unique 铺路） | 无 | 低 | plan-then-apply；收敛 |
| m049 | 2026_07_049_reconcile_prompt_planning_currents | **直接复用 m043::run_step 重跑**（修 m043 marker 早于 planning-only prompt 不变量的升级库；复用保全量校验规则不二义，m049:6-7） | 无 | 同 m043 | 同 m043 |
| m050 | 2026_07_050_taxonomy_identity_claims | 全量校验（canonical id/alias 非空不重复/alias==canonical→炸/status∈{active,deprecated}/current+active 行的 claims 在 (ws,scope,kind) 命名空间无歧义占有→炸）后给**所有历史版本**回填 value.identityClaims（旧版本 rollout 也受同一 unique 约束，m050:6-8） | 无 | 无 | 校验先行；$set 幂等 |
| m051 | 2026_07_051_domain_profile_release_invariants | 审计 domain_profiles（version 唯一/每 lineage ≤1 current/每 ws ≤1 active；current 与 active 刻意独立，m051:5-7）+ 回填 release_status="published"（仅缺失行） | 无 | 无 | 审计+$exists 回填 |
| m052 | 2026_07_052_catalog_rebuild_leases | 每个知识文档经 CAS 分配一次性 reconciliation generation（marker 字段 `catalog_m052_reconciliation_generation` 写在文档上防重复分配；并发输 CAS 则重读收敛，m052:51-108）→ 以父 `_id` 为 job `_id` upsert 一条 `crj_m052_*` reconciliation job（$setOnInsert+回读全字段核验冲突）；随后把非 m052 的 queued/processing/failed 且无有效 target_generation 的 legacy job 批量置 superseded 并 $unset 租约字段（completed 保留审计） | 无 | 低（retire legacy job） | marker+upsert+回读校验 |
| m053 | 2026_07_053_ingest_source_claims | ingest_sources 补 source_generation=1/claim_generation=0（仅缺失/null；已有值校验非负整数否则炸；有效活跃 claim 原样保留） | 无 | 无 | $or exists/null 过滤 |
| m054 | 2026_07_054_playbook_single_default | 审计 operation_playbooks（canonical scope/is_default bool/release_status 闭集{draft,published}或缺失/draft default→炸/每 (ws,acct) >1 default→炸——缺 default 可由懒 bootstrap 恢复故放行）+ 回填 release_status="published"（仅缺失） | 无 | 无 | 审计+回填 |
| m055 | 2026_07_055_lesson_promotion_identity | 全量校验 lessons↔chunks 晋升图：lesson 身份唯一/review_status∈{pending_review,promoted}（缺省=pending）/pending 不得有 promoted_chunk/promoted 必须有且 chunk 存在、同 ws、promotion 形态（chunk_type=peer_case+business_context 前缀 lessons_learned::）、context 与 pattern_kind 一致/一 chunk 不得被两 lesson 引用/孤儿 promotion 形态 chunk→炸/无主 lesson anchor→炸；然后仅对 provenance 缺失/null 的精确配对回填 `{source:"lesson_promotion", source_doc_id, edited_at}`（CAS modified≠1 炸） | 无 | 无 | 校验先行；conflicting provenance 拒绝 |
| m056 | 2026_07_056_import_job_claims | import_jobs 补 claim_generation=0（仅缺失/null；已有值校验非负；活跃 claim token/时间戳保留） | 无 | 无 | exists/null 过滤 |
| m057 | 2026_08_057_explicit_acknowledgement_action | 把 acknowledgement 物化进**每个历史版本**的 state policy allowed 列表（此前靠代码级例外授予，使 allowed 不完整）：空 allowed→KNOWN_STATE_ACTIONS 全集减 forbidden；非空→追加 ack（除非 forbidden）；按 KNOWN_STATE_ACTIONS 序重排去重（m057:22-46）。**双字段 BSON 形态 CAS**（allowed+forbidden 原形匹配，missing/null/数组三形态分别表达，防并发编辑下用陈旧 forbidden 计算，m057:65-94）；CAS 失败重读只接受"恰为目标形态"否则炸 | 无 | 无 | 目标形态即跳过；CAS 收敛 |
| m058 | 2026_08_058_llm_provider_active_invariant | **纯只读审计**：isActive=true 行 identity 非空；每 workspace >1 active → Conflict 炸（报告精确 ws/provider 清单，不选举）；为 partial unique 铺路 | 无 | 无（零写入） | 纯读 |

### 4.2 迁移工程规律（横向总结）
1. **两代风格**：m001-m032 多为"回填/seed/形态升级"（$exists 过滤 + $setOnInsert/pipeline 幂等）；m038 起进入"reconciliation 世代"——全量**校验先于首写**（validation-before-write）、canonical 字符串校验（非空、无前后空白）、CAS 匹配数断言、歧义 fail-closed 炸启动而非猜赢家（m041/m044/m058 甚至纯只读）。
2. **索引配合**：改唯一性语义的迁移都在 `ensure_indexes` 之前收敛数据（m017→outcome dedupe、m038→scoped idempotency、m043/m048→single current、m050→identityClaims、m051/m054/m055/m058→审计），否则 partial unique 创建会 E11000 炸启动（indexes.rs:1073-1080 H8 boot-brick 注释）。
3. **重跑包装**：m034/m035/m036/m049 是"corrective rerun"——直接调用旧 step 函数以复用其校验逻辑，修"假阳性 marker"库。
4. **删除类三条**（m011/m012/m014）+ 所有权指派类一条（m016）受审批闸保护；其余 54 条无条件执行（多条注释明确说明"误加守卫会致生产静默 SKIP"，如 m030:10-14、m031:8-9）。

---

## 5. 事实卡速查

### 5.1 闭集枚举取值全表（models.rs 内定义的）
| 枚举/常量 | 取值 | 出处 |
|---|---|---|
| `AgentStatus` | normal, managed | L6-11 |
| `MessageDirection` | inbound, outbound | L804-809 |
| `ALLOWED_CAMPAIGN_STATUS` | draft, previewed, confirmed, dispatching, completed, canceled | L683-690 |
| `CampaignSend.status` | prepared, enqueued | L674（注释级闭集） |
| `ALLOWED_AGENT_TASK_STATUS` | pending, running, **committing**, retry, failed, cancelled, sent, completed, outbox_enqueued（9 值） | L918-928 |
| `ALLOWED_IMPORT_JOB_STATUS` | pending, running, completed, failed | L1052 |
| `ImportJob.apply_status` | ready, applying, applied | L1006-1010（注释级） |
| `ALLOWED_WIKI_TYPE` | source, entity, concept, comparison, synthesis, methodology, finding, query, thesis（9 值） | L1865-1875 |
| `ALLOWED_CHUNK_TYPE` | product_fact, style_template, peer_case, negative_example（4 值；缺省/越界→product_fact） | L1878-1883 |
| `RelatedRef.kind` | superseded_by, references, requires, contradicts, clarifies, refines（6 值） | L1824, 2009 |
| `ChunkRevision.op` | create, patch, split, merge, rollback, archive, restore, verify, unverify, reject（10 值） | L2040 |
| `ChunkProvenance.source` | ai, human, rule, imported（m055 另有 "lesson_promotion"，见 §6.6） | L1993, 2053 |
| `KnowledgeGapSignal.kind` | orphan, broken_link, no_outlinks, contradiction, stale, missing_chunk, suggestion, low_confidence（8 值） | db/mod.rs:384-386 |
| `KnowledgeGapSignal.status` | pending, auto_resolved, llm_resolved, applied, dismissed | L2089 |
| `KnowledgeGapSignal.severity` / `.source` | warning, info / rule, llm | L2085-2088 |
| `IngestSource.kind` / `.status` | rss, html / active, failing, disabled | L2126-2127, 2147-2148 |
| `DomainField.kind` | string, enum, number, date, reference | L2183 |
| `DomainProfile.release_status` | draft, published（缺省 published） | L2440-2446 |
| `CoverageDimension.initial_signal` | verified, evidence, None(恒 missing) | L3026-3034 |
| `CatalogRebuildJob.status` | queued, processing, done, superseded, discarded, failed | L3204 |
| `KnowledgeChatTurn.status` | pending, applying, applied, discarded | L3273 |
| `AgentRunLog.lifecycle` | started, running, completed, failed_before_decision, failed_after_decision, aborted_by_budget, aborted_by_external_signal（7 值） | L3418-3422 |
| `AgentRunLog.source_kind` | inbound_message, follow_up_task, manual_send | L3429-3431 |
| `AgentRunLog.autonomy_mode` | auto, assisted, blocked | L3456 |
| `conversation_mode`（四模式） | casual_relationship, value_exchange, consultative, boundary_protection | L3459-3461（字典 seed m028） |
| `AgentRunLog.outbox_status` / `OutboxEntry.status` | pending, in_flight, sent, failed_terminal, canceled(+outbox 侧 delivery_unknown) | L3472-3475, 3503-3505 |
| `ALLOWED_PRINCIPAL_ESCALATION_STATUS` | pending, resolved, delivery_failed | L4491-4498 |
| relay_state | pending, enqueued, terminal | L4500-4502 |
| card delivery | pending_enqueue, queued, sent, failed_terminal, delivery_unknown | L4504-4508 |
| `ALLOWED_ESCALATION_CATEGORY` | out_of_scope_decision, high_risk_gated, stuck_or_undelivered | L4533-4540 |
| `ALLOWED_PRINCIPAL_VERDICT` | approved, rejected, conditional, deferred, delegated_back | L4564-4575 |
| exemption_type | none, customer_only, knowledge | L4578-4580 |
| `Experiment.status` | collecting, evaluating, awaiting_admin, released, aborted | L5635 |
| `Proposal.status` | pending_eval, evaluating, eligible_for_release, rejected_below_threshold, released, rolled_back | L5666-5667 |
| `Proposal.proposed_section` | soul, system_contract, policy, operator_instruction | L5685 |
| `ShadowReplay.status` | completed, failed | L5731 |
| `ThresholdOverrideAudit.action` / `.decided_by` | released, rolled_back, auto_released / admin:<id>, evolution_auto, evolution_release, evolution_rollback | L5790-5796 |
| `KnowledgeDigestCard.kind` | chunk_missing_field, chunk_low_hit_rate, chunk_caused_block, pack_outdated, evolution_pending, evolution_released, freeform | L5852-5853 |
| `KnowledgeDigestCard.suggested_action` / `.severity` | fix_chunk, add_chunk, retag, review_evolution, dismiss, freeform / info, warn, critical | L5871-5875 |
| `KnowledgeDailyReport.status` / `.generated_by` | ok, partial, failed / worker, manual | L5893-5895 |
| `ALLOWED_TASK_STATUS`（KnowledgeChatTask） | pending, running, completed, failed, cancelled | L5934-5935 |
| ChatTask step status | committed, noop, needs_manual, failed | L5962-5963 |
| `KnowledgeOperatorMemory.kind` | preference, rejection, context | L6006-6007 |
| `LlmProviderConfig.format` | openai, anthropic | L6035-6036 |
| `LlmCallLog.final_status` | success, failed, json_error, cache_hit | L3890-3892 |
| `ALLOWED_AGENT_COMMAND_RUN_STATUS` | pending_confirmation, running, dry_run, succeeded, failed, execution_unknown, canceled（7 值） | L4063-4071 |
| `ALLOWED_TOOL_CALL_STATUS` | prepared, executing, dry_run, succeeded, accepted, failed, executed_unverified, execution_unknown（8 值） | L4113-4122 |
| `OutcomeEvent.verification` / `.event_kind` | conversation_inferred, staff_confirmed, payment_verified / deal, reversal | L456-458, 468 |
| `Product.status` | active, archived | L546 |
| `TaxonomyValue.status` | active, deprecated | L3680-3681 |
| 候选/建议/疑似成交 status | pending, approved, rejected | L3765, 3803, 3837 |
| `AgentSoul.status` | draft, published, archived（m042 校验闭集） | m042:140 |
| `PromptTemplate.status` | draft, active, archived（m043 校验闭集） | m043:172 |
| `MemoryFact` 边界 | text 1..=500 / evidence ≤1000 / confidence & importance 0..=10 / deprecation_reason ≤200 / source_message_ids ≤5 | L5075-5088 |
| memoryCard 骨架 cap | coreFacts 6 / recentFacts 10 / deprecatedFacts 20（写侧强制） | L4985-4987 |
| `IntentTrajectoryEntry::MAX_ITEMS` | 50 | L5494 |
| Contact.commitments cap | 8 | L210 |
| m057 `KNOWN_STATE_ACTIONS` | reply, acknowledgement, silent, follow_up, cooldown | m057:14-20 |

### 5.2 唯一索引 / 幂等键全表（unique 与 partial unique；名称→键→filter）
| 集合 | 索引 | 键 | partial filter |
|---|---|---|---|
| wechat_accounts | (默认名) | (workspace_id, account_id) | — |
| wechat_accounts | uniq_wechat_accounts_app_id | app_id | app_id: $type string |
| contacts | (默认名) | (workspace_id, account_id, wxid) | — |
| conversation_messages | (默认名, sparse) | (ws, acct, message_id) | sparse |
| conversation_messages | (默认名) | (ws, acct, dedupe_key) | dedupe_key: $type string |
| agent_tasks | uniq_outcome_aggregation_ws_kind_account_content | (ws, kind, acct, content) | kind="outcome_aggregation" |
| agent_tasks | uniq_memory_active_task_key | (ws, acct, contact_wxid, active_task_key) | active_task_key: $type string |
| agent_events | uniq_events_workspace_dedupe_key | (ws, dedupe_key) | $type string |
| behavior_signals | uniq_behavior_signals_ws_account_dedupe_key | (ws, acct, dedupe_key) | $type string |
| import_job_segments | uniq_import_job_segment | (job_id, segment_index) | — |
| agent_send_ledger | uniq_send_ledger_outbox_id | outbox_id | $type objectId |
| agent_souls | uniq_agent_soul_ws_kind_version | (ws, agent_kind, version) | — |
| agent_souls | uniq_agent_soul_published_ws_kind | (ws, agent_kind) | status="published" |
| operation_playbooks | uniq_operation_playbook_default_per_account | (ws, acct) | is_default=true |
| prompt_templates | (默认名) | (ws, prompt_key, version) | — |
| prompt_templates | uniq_prompt_current_pointer | (ws, prompt_key) | current_version=true |
| prompt_templates | uniq_prompt_artifact_per_proposal | (ws, source_proposal_id) | $type objectId |
| operation_domain_configs | op_domain_ws_domain_version_unique | (ws, domain, version) | — |
| operation_domain_configs | uniq_op_domain_ws_domain_current | (ws, domain) | current_version=true |
| operation_state_policies | op_state_policy_ws_domain_state_version_unique | (ws, domain, state_key, version) | — |
| operation_state_policies | uniq_op_state_policy_ws_domain_state_current | (ws, domain, state_key) | current_version=true |
| system_taxonomies | sys_tax_ws_scope_kind_value_version_unique | (ws, scope, kind, value.id, version) | — |
| system_taxonomies | uniq_sys_tax_ws_scope_kind_value_current | (ws, scope, kind, value.id) | current_version=true |
| system_taxonomies | uniq_sys_tax_ws_scope_kind_active_identity | (ws, scope, kind, value.identityClaims)（multikey） | current=true + value.status="active" |
| operating_memories | (默认名) | (ws, acct, contact_wxid) | — |
| operation_knowledge_chunks | uniq_kchunks_lesson_promotion_source | (ws, provenance.source_doc_id) | provenance.source="lesson_promotion" + $type string |
| memory_candidates | uniq_memory_projection_key | (ws, acct, contact_wxid, projection_key) | $type string |
| agent_run_logs | (默认名) | run_id | — |
| evaluation_scenarios | (默认名) | (ws, scenario_id) | — |
| agent_send_outbox | uniq_outbox_ws_account_idempotency | (ws, acct, idempotency_key) | — |
| system_incidents | uniq_system_incident_identity | (ws, incident_key) | — |
| taxonomy_candidates | tax_candidate_ws_scope_kind_raw_unique | (ws, scope, kind, raw_value) | — |
| relationship_type_suggestions | uniq_relationship_pending_ws_contact | (ws, contact_id) | status="pending" |
| suspected_deal_signals | uniq_suspected_deal_pending_ws_contact | (ws, contact_id) | status="pending" |
| projection_observations | uniq_projection_observation_entity_run | (ws, entity_type, entity_id, run_id) | — |
| experiments | (默认名) | experiment_id | — |
| threshold_overrides | uniq_threshold_current_per_scoped_gate | (ws, acct, gate_key) | current_version=true |
| threshold_overrides | uniq_threshold_artifact_per_proposal | (ws, acct, source_proposal_id) | $type objectId |
| post_release_reviews | uniq_post_release_review_protocol_v1 | (ws, acct, proposal_id) | protocol_version=1 |
| knowledge_daily_reports | (默认名) | (ws, acct, report_date) | — |
| knowledge_gap_signals | gap_signals_signal_id_unique | signal_id | — |
| knowledge_gap_signals | uniq_gap_signals_pending_ws_dedup | (ws, dedup_key) | status="pending" + $type string |
| domain_schemas | domain_schemas_ws_id_version_unique | (ws, schema_id, version) | — |
| domain_schemas | domain_schemas_ws_active_unique | (ws) | is_active=true |
| domain_profiles | domain_profiles_ws_id_version_unique | (ws, profile_id, version) | — |
| domain_profiles | domain_profiles_ws_id_current_unique | (ws, profile_id) | current_version=true |
| domain_profiles | domain_profiles_ws_active_unique | (ws) | is_active=true |
| catalog_rebuild_jobs | catalog_jobs_job_id_unique | job_id | — |
| ingest_sources | ingest_sources_source_id_unique | source_id | — |
| llm_provider_configs | (默认名) | (workspaceId, providerId) | — |
| llm_provider_configs | llm_provider_one_active_per_workspace | (workspaceId) | isActive=true |
| llm_provider_configs | uniq_llm_vision_active_workspace | (workspaceId) | isVisionActive=true |
| products | (默认名) | (workspace_id, product_id) | — |
| campaign_sends | (默认名) | (campaignId, contactWxid) | — |
| roster_snapshots | (默认名) | (workspace_id, account_id) | — |
| lessons_learned | uniq_lessons_learned_ws_lesson | (ws, lesson_id) | — |
| agent_principal_escalations | uniq_principal_escalation_short_code | short_code | — |
| agent_principal_escalations | uniq_principal_escalation_pending_ws_account_contact_category | (ws, acct, contact_wxid, category) | status="pending" |
| management(agent_tool_calls) | uniq_management_tool_intent | (ws, acct, intent_key) | $type string |
| admin_users | admin_users_username_unique | username | — |
| admin_sessions | admin_sessions_session_id_unique | session_id | — |
| reviewer_stats / deal_attribution_stats | *_stat_id_unique | stat_id | — |
| migrations（隐式） | `_id` = 迁移 id | — | — |

**逻辑幂等键（非索引）**：`AgentOutcomeMetric._id`="{ws}:{acct}:{horizon}:{date}"；`BehaviorSignalMetric._id`="{ws}:{date}"；`knowledge_chat_session_seqs._id`="{ws}|{session_id}"；`configuration_generations._id`="{namespace}\0{ws}"；`background_worker_controls._id`=worker 名；`background_worker_leases._id`="{kind}::{ws}"；behavior_signals.dedupe_key 约定格式（models.rs L743-746）；m052 job `_id`=父文档 ObjectId、job_id="crj_m052_{hex}"。

### 5.3 RuntimeParametersTyped 默认值表（models.rs L4877-4978，运营域 runtime_parameters 真相源）
recent_message_limit=12；min_reply_interval_seconds=20；max_daily_touches=3；max_pending_follow_ups=3；follow_up_expires_hours=48；cooldown_after_no_reply_hours=24；**hallucination_block_at=6（FactRisk≥6 block）**；**pressure_risk_block_at=7**；**knowledge_grounding_block_below=7**；**human_like_rewrite_below=6**；**emotional_value_rewrite_below=6**；operation_state_confidence_full_review_below=4；run_token_budget=150000；run_token_budget_escalated=500000；run_max_llm_calls=6；simulation_token_budget=300000；reaction_token_budget=8000；reaction_max_llm_calls=2；autonomy_protocol_enabled=true；knowledge_max_tool_calls=6(clamp 1-16)；knowledge_open_slice_max_k=4(clamp 1-16)；knowledge_search_top_k=8(clamp 1-32)；outbox_poll_interval_seconds=5(clamp 1-60)；outbox_lease_seconds=60(clamp 10-600)；quiet_hours_enabled=true；quiet_hours_start=22；quiet_hours_end=8；quiet_hours_tz_offset_hours=8；consolidation_window_char_budget=6000(clamp 1000-16000)；consolidation_window_max_messages=60(clamp 10-200)；bayesian_slot_min_hits=3(clamp 1-20)；bayesian_slot_min_strong=2(clamp 0-20)。

### 5.4 config_generation 命名空间
`domain_profile` / `taxonomy` / `llm_provider`（config_generation.rs:16-18）；集合 `configuration_generations`；`_id="{namespace}\0{workspace_id}"`（NUL 分隔，L20-22）；读容忍 i32/i64；bump 事务版与权威变更同事务（taxonomy 懒 seed 即用之，migrations/mod.rs:229-235）。

### 5.5 TTL 索引一览
| 集合.字段 | TTL | 出处 |
|---|---|---|
| webhook_rate_limit_windows.expires_at | 0（到点即删） | indexes.rs:736-750 |
| import_jobs.expires_at | 0（终态 +24h 才设字段） | 891-908 |
| import_job_segments.expires_at | 0 | 28-38 |
| proactive_daily_quotas.expires_at | 0 | 326-336 |
| knowledge_usage_logs.created_at | 35 天（>回路① 30d 窗） | 1125-1143 |
| agent_outcome_metrics.created_at | env OUTCOME_METRICS_TTL_DAYS（默认 90d） | 1363-1380 |
| llm_call_logs / agent_run_logs / mcp_call_logs .created_at | env DIAGNOSTIC_LOG_TTL_DAYS（默认 30d，0 禁用） | 1389-1427 |
| knowledge_operator_memory.expires_at | 0 | 2925-2943 |
| admin_sessions.expires_at | 0 | 3203-3219 |
| auth_security_events.expires_at | 0 | 3239-3252 |

---

## 6. 偏差与疑点

1. **【偏差·函数名误导】`ensure_evolution_indexes` 内容远超 evolution**（indexes.rs:2602-3444）：该 helper 除 evolution 五表外还包含 prompt_templates 多版本、知识日报三表、operator memory、knowledge-wiki 四表、admin 鉴权三表、ingest_sources、reviewer_stats、deal_attribution_stats、lessons_learned、agent_principal_escalations 的全部索引。纯组织问题不影响行为，但按名找索引会漏一大半。
2. **【事实·typed 模型未声明的 raw 动态字段】**以下字段有索引支撑但不在对应 struct 中，由业务代码以 raw Document 写入（已 Grep 亲验写入方）：`agent_send_outbox.delivery_finalize_pending`（src/agent/outbox_dispatcher.rs）；`agent_decision_reviews.post_decision_status / post_decision_next_retry_at / post_decision_locked_until / post_decision_scrub_at / post_decision_profile_done`（src/agent/post_decision.rs）；`conversation_messages.handoff_status`（src/webhooks.rs）；`agent_tasks.active_task_key`（**26 号修正：原误记 operating_memories**——索引建于 db.tasks()，写点在任务行，memory.rs/webhooks.rs/contacts.rs 写的都是 agent_tasks 文档）；`memory_candidates.projection_key`；`operation_knowledge_documents.catalog_m052_reconciliation_generation`（m052:21）。能共存是因为所有模型无 deny_unknown_fields。**恢复理解时切勿以为 models.rs 是字段全集**。
3. **【疑点·闭集与测试/注释不同步】`ALLOWED_AGENT_TASK_STATUS` 含 9 值（含 `committing`，L920），但其 doc 注释的"历史值清单"（L913-917）与单测 `closed_set_covers_all_known_writers`（L948-963）都只列 8 值、未含 committing**。committing 合法且在闭集内，但注释/测试未跟进——不影响运行，属文档滞后。
4. **【偏差·日志 id 不一致】m028 的 `tracing::info!(migration_id = "m028_seed_conversation_mode")`（m028:110）与注册 id `2026_06_Y2_001_seed_conversation_mode`（mod.rs:415）不一致**；其余迁移日志均用注册 id。仅影响日志检索。
5. **【偏差·注释行号过期】**多处注释引用的 models.rs 行号已漂移：m016 测试注释 "LlmProviderConfig(models.rs:4732)/Campaign(552)/CampaignSend(596)"（m016:199-201，实际 6026/581/658）；m030 注释 "models.rs:451/464/248"（m030:3-5,19，实际 460/473/251）；models.rs L204 提到迁移名 `2026_05_008_contact_commitments_reshape` 写成 `2026_05_008`（注册 id 一致）。**恢复理解时以本记录行号为准，不信旧注释行号**。
6. **【疑点·provenance.source 闭集外值】**`ChunkProvenance.source` 注释声明闭集 {ai,human,rule,imported}（L1993），但 m055 与 lesson 晋升索引使用 `source="lesson_promotion"`（m055:19、indexes.rs:171-173）。实际是第五个合法值，模型注释未更新。
7. **【疑点·operation_state 与 customer_stage 双轴】**`Contact.operation_state`（L220）按 CLAUDE.md 由 gateway 从 canonical `customer_stage` 派生同步（C2/m006 同一 id 空间），仅无 customer_stage 时回落 decision 自身值；models.rs 字段旁没有注释说明这一点，仅靠 CLAUDE.md/网关代码承载。本次未读 gateway 写点（不在任务范围），此结论转引自 CLAUDE.md，标注为**待亲验**。
8. **【疑点·`default_taxonomy_workspace_id` 反序列化读 env】**（L3777-3779）TaxonomyEntry/TaxonomyCandidate 的 workspace_id serde default 在**反序列化时刻**读 `DEFAULT_WORKSPACE_ID` 环境变量——同一文档在不同 env 的进程里可反序列化出不同 workspace_id（仅影响缺字段的滚动升级窗口行；m032 已物理回填）。设计上可接受但属隐式环境耦合。
9. **【疑点·Contact 无 default 的 Option 字段】**`follow_up_policy`/`operation_state`/`operation_state_reason`/`operation_state_confidence`/`operation_state_updated_at`/`cooldown_until`/`profile_updated_at`/`last_*_at`/`agent_profile`/`memory_summary`/`playbook_id`/`playbook_version`/`nickname`/`remark`/`alias`（L158-233）均为 `Option` 且**无** `#[serde(default)]`。BSON 缺键时 serde 对 Option 天然回 None（mongodb bson 反序列化容忍 missing→None），因此实际兼容——但与其它字段"显式 default"风格不一致；tag_trust 测试（L7848-7856）的最小文档确实不含这些键且反序列化成功，证明兼容成立。
10. **【疑点·`operation_knowledge_items` 幽灵集合】**typed accessor 已删（m011 注释 m011:4-5），但 m010/m011/m014 仍触达它，m016 回填表不含它。若某库只跑过部分历史版本，可能存在无人维护的 items 残留数据。对新代码无影响。
11. **【事实·两个"同函数不同顺序"的 current 收敛】**m009 先 promote 后 demote（中断保"至少一条 current"）；m034 先 demote 后 promote（在 unique 索引已存在的库上避免 E11000）。方向相反皆有意为之（m009:76-78、m034:53-56），不是 bug。
12. **【疑点·models.rs 注释里的迁移名与注册名不完全一致】**L214-216 说 "迁移 `2026_05_008_contact_commitments_reshape`"，注册表实为 `2026_05_008_contact_commitments_reshape`（一致）；但 L1802 说 "migration `2026_05_W1_001_chunks_wiki_type_default` 把缺字段 chunk 默认填 entity"——**该迁移不在 MIGRATIONS 注册表中（mod.rs:305-538 无此 id）**。wiki_type 缺省回填的实际机制是读取侧 `wiki_type_priority` 兜底（L1887）。此注释描述的迁移疑似从未落地或已被移除，**疑点**。
13. **【事实·索引/字段大小写陷阱清单】**（历史真实事故沉淀）：llm_provider_configs 曾用 snake 键建索引→全文档 (null,null) 撞唯一（indexes.rs:1967-1977）；campaigns/campaign_sends 必须 camel（indexes.rs:2053-2056）；behavior_signals 必须 snake（models.rs L719-722）；contacts 的 outcome 索引是混合路径 `outcome_events.productRef.productId`（indexes.rs:760-763）；partial filter 只接受 $eq/$exists/$type 等，$in/$or 会 Error 67 炸 ensure_indexes（indexes.rs:1188-1193, 949-953）。
14. **【疑点·`m055` 引用 `lessons_learned.review_status` 闭集 {pending_review, promoted}**（m055:98）与 lessons_learned 集合无 typed 模型**：lessons 的完整 schema 只存在于 routes/evolution 代码（本次任务范围外），此处仅记录迁移侧读到的形状。
15. **【事实·工作树状态】**本记录基于未提交的工作树（git status 显示 src/models.rs、src/agent/*、tests/* 等 47 文件 modified）。行号与已提交版本可能有差异。

---

## 7. 覆盖自证（读过的文件与行号区段）

| 文件 | 总行数 | 读取区段（Read offset/limit） | 覆盖 |
|---|---|---|---|
| src/models.rs | 8353 | 1-1200, 1200-2400, 2400-3600, 3600-4800, 4800-6000, 6000-7200, 7200-8353 | 100% |
| src/db/mod.rs | 456 | 1-456（一次全读） | 100% |
| src/db/indexes.rs | 3444 | 1-1200, 1200-2400, 2400-3444 | 100% |
| src/db/config_generation.rs | 108 | 1-108 | 100% |
| src/db/migrations/mod.rs | 721 | 1-721 | 100% |
| src/db/migrations/helpers.rs | 263 | 1-263 | 100% |
| m001(42)/m002(49)/m003(62)/m004(62)/m005(90)/m006(486)/m007(19)/m008(53) | — | 各全读 | 100% |
| m009(105)/m010(69)/m011(34)/m012(31)/m013(153)/m014(37)/m015(58)/m016(268)/m017(83) | — | 各全读 | 100% |
| m018(129)/m019(61)/m020(252)/m021(226)/m022(65)/m023(147)/m024(163)/m025(51) | — | 各全读 | 100% |
| m026(51)/m027(40)/m028(143)/m029(132)/m030(127)/m031(67)/m032(55)/m033(36)/m034(74)/m035(13)/m036(16)/m037(40) | — | 各全读 | 100% |
| m038(113)/m039(287)/m040(227)/m041(168)/m042(190)/m043(265)/m044(138)/m045(164)/m046(165)/m047(274)/m048(295) | — | 各全读 | 100% |
| m049(13)/m050(239)/m051(171)/m052(263)/m053(126)/m054(146)/m055(343)/m056(87)/m057(182)/m058(91) | — | 各全读 | 100% |
| 辅助核验（Grep） | — | delivery_finalize_pending / post_decision_* / handoff_status / active_task_key / projection_key 写入方定位；projection_observations::COLLECTION（src/agent/projection_observations.rs:14）；is_operatable_person（src/webhooks.rs，被 routes/contacts.rs 与 m029 引用） | 点核验 |

合计精读约 20,900 行（13,345 行核心文件 + 7,536 行迁移 + 少量交叉核验）。除 §6.7 标注"待亲验"的 gateway 写点外，本记录所有断言均出自上述亲读内容。

---

## 八、26 号交叉验证回写修正（2026-08-13，主会话执行）

26 号数据层交叉验证对本记录做了 8 组约 40 项断言对照 + 25 锚点抽验（全过）。除已原地修正的 `active_task_key` 归属错误外，以下 3 处属"照抄过时代码注释"的滞后失真，以实现为准：

1. **locked_fields 默认实为 8 项**（非注释所写 7 项）：`page_merge.rs:35-44` 亲数 _id/workspace_id/account_id/document_id/item_id/wiki_type/chunk_type/created_at；`models.rs:1837` 注释本身过时。
2. **provenance.source 持久值域 ≥6 种**（非注释 4 种）：枚举 5 值含 `principal_authorized`（`chunk_revisions.rs:98-138`）+ m055 的 `lesson_promotion`；`models.rs:1993` 注释过时。
3. **AgentRunLog.outbox_status 实际 7 值**（非注释 5 值）：`aggregate_run_outbox_status` 另产 `partially_sent`（`outbox_dispatcher.rs:1008`）与 `delivery_unknown`（`:1016-1021`）并写回 run log；`models.rs:3472-3476` 注释滞后。
4. **§6.2 口径补充**：该清单为"有索引支撑"的 raw 字段；无索引的 raw 协议字段数量远大于此（agent_tasks 的 claim/prepared_commit/obligation 族尤甚，见 03/05/06 号），另补 `post_release_reviews.actual_negative_reaction_rate_delta`（`post_release.rs:236` 写，typed 无）。
5. **source_kind 口径澄清**：`agent_run_logs.source_kind` 实际写入面=3 值（模型注释准确）；run_envelope 的 6 常量中后 3 个仅流入 `agent_send_outbox.source_kind`——读本记录时勿把两个集合的 source_kind 值域混同。

**使用纪律（26 号结论）**：本记录 const/断言级闭集可直接引用；注释级数值必须到实现侧复核。


### 八.补（25/30 号追加，2026-08-13）

- **gap signal kind 注释滞后**：models.rs 注释 8 种，实现为 9 种结构 lint + 3 种在线信号（gap_signals.rs 实现侧为准，25 号裁决）。
- **source_kind 全集口径（30 号统一）**：run_envelope 常量全集 6 值；`agent_run_logs.source_kind` 写入面 3 值、后 3 值仅流入 `agent_send_outbox.source_kind`——引用时必须先说明哪个集合。
