# 决策/评审/守卫/类型/领域 深读记录（核证日期 2026-08-13）

> 范围：`src/agent/decision.rs`、`src/agent/review/{mod,gates,style}.rs`、`src/agent/guards.rs`、`src/agent/types.rs`、`src/agent/runtime.rs`、`src/agent/taxonomy.rs`、`src/agent/decision_taxonomy.rs`、`src/agent/domain.rs`、`src/agent/domain_profile.rs`、`src/agent/domain_signals.rs`、`src/agent/dimension_registry.rs`、`src/agent/bayesian_slots.rs`、`src/agent/entitlements.rs`。共 15 文件 23814 行，全部逐行读完（覆盖自证见 §6）。所有断言均附 file:line；读不懂/存疑处记入 §5。

---

## 1. 模块地图

```
Reply Agent 决策
  decision.rs        — decide_reply_with_promote（prompt 组装 + LLM 调用 + promote + taxonomy 归一）
                       build_initial_operation_profile（初始画像）、prompt/资产/名片/素材加载器
  types.rs           — AgentDecision / RawAgentDecision（validate_and_promote / validate_reply_critical）
                       DecisionReviewResult / ReviewScores / KnowledgeRouteResult / hold_category 校验
  runtime.rs         — UserRuntimeParameters（阈值/预算/静默时段）、写侧 schema 校验、
                       ResolvedThresholds（threshold_overrides 覆盖链）
独立评审
  review/mod.rs      — review_decision（light/full 双模 + 双脑并行）、parse_live_review 严格 wire、
                       Independent ClaimGate（run/parse/harden/apply）、local_decision_review、
                       effective_review_mode / should_run_review / should_run_targeted_rewrite
  review/gates.rs    — review_passed、classify_dual_gate / route_dual_gate（双闸）、
                       双 reviewer 分歧、finalize_review_for_send（最终硬门汇总）、
                       decide_revision / derive_revision_failure / apply_revision_fallback
  review/style.rs    — 出站风格指纹（extract/observe/render，仅审计不拦截）
守卫（纯函数）
  guards.rs          — check_state_transition（状态机）、enforce_state_action_policy（action 门禁）、
                       classify_decision_action / classify_reviewed_decision_action、
                       is_verified / compute_verified_chunks、commitment_claim_class
taxonomy 双层标签
  taxonomy.rs        — TaxonomyCache（进程级 TTL30s + generation 比对）、check_value 四分支、
                       upsert_candidate（幂等候选）
  decision_taxonomy.rs — classify_decision_tags（决策侧 4 路分支 + alias 改写）
domain 系列（行业通用化）
  domain.rs          — OpsDomain trait 边界声明（USER_OPS_DOMAIN_ID）
  domain_profile.rs  — DomainProfile 加载器 + DomainProfileCache（TTL30s）+ DEFAULT_PROFILE +
                       全部 prompt 类 override 注入函数（apply_* 家族）
  domain_signals.rs  — typed 两维 ↔ domain_signals 容器双向同步 + 画像写入内核
  dimension_registry.rs — 7 维度元数据单一真相源 + classify_validation（Accept/Reject/Drop）
  bayesian_slots.rs  — 贝叶斯观察旁路占槽（6 槽、min_hits+min_strong 双门）
  entitlements.rs    — G4 持有投影（outcome_events 派生）、产品目录渲染、G1 客观锚纠偏、LTV 分层
```

依赖方向：`gateway.rs`（不在本次范围）是唯一编排者——它调用 decision → review → claim gate → finalize → revision。decision.rs 调 domain_profile / decision_taxonomy / domain_signals / entitlements；review/mod.rs 调 gates / style / guards / domain_profile；gates.rs 调 guards / types / runtime；均不反向依赖 gateway。

---

## 2. 逐文件深读

### 2.1 `src/agent/decision.rs`（2721 行）

#### PromptOverride（decision.rs:40-80）
- 职责：Prompt Shadow（critic 候选）回放时对单一 prompt key 的"冻结基线正文 + 追加片段"覆盖。
- `use_frozen_base_if_matches`（:60-66）：key 命中 → 返回冻结正文；否则原样。
- `append_if_matches`（:68-75）：key 命中 → `applied` 置 true（Release 序）并经 `prompt_guard::compose_appended_content` 追加；`was_applied`（:77-79）Acquire 读，证明 replay 真经过注入点。

#### build_initial_operation_profile（decision.rs:82-206）
- 输入运营备注 note + 可选 playbook，产出 `GeneratedOperationProfile`。
- 流程：playbook 文本（无则固定兜底文案 :89-91）→ `load_user_operation_domain_config`（:92）→ domain_text → **加载 active DomainProfile**（:103-104，H3 修复点：此前初始画像是唯一漏接 profile 的 prompt 构造点）→ `render_business_context_fragment`（:105-108，header 为"本行业业务上下文（运营配置，补充运营方法与域策略）："）→ 加载 `user.initial_profile.system` / `user.initial_profile.task` 两 prompt（:109-112）→ **C-02**：task 末尾追加 `render_memory_candidate_types_guidance` + `render_decision_dimensions_guidance`（:119-130，DEFAULT 销售域两者均空串 → 字节等价）→ 拼 user prompt（:131-143，顺序：task / 运营方法 / 用户运营域策略+business_context / 运营人员描述）→ `generate_agent_json`（:144-154，prompt key `user.initial_profile.task`，无 account/contact/run_id）。
- 解析（:155-205）：`agentProfile` 兼容 camel/snake 双形；`summary` 缺失回落 note 原文（:164）；`tags` 走 `string_array`；`customer_stage`/`intent_level`/`last_commitment`/`follow_up_policy` 均 camel-or-snake 双查；`profile_attributes` 经 `to_document` 容错。
- 错误路径：任何 prompt 加载失败 / LLM 失败 → `?` 直接向上抛 AppError。

#### load_recent_reaction_hint（decision.rs:224-256）+ query 形状纯函数（:260-292）
- 从 `decision_reviews` 集合按 `(workspace_id, account_id, contact_wxid, reaction_analysis 存在且非空 doc)` 过滤（`build_reaction_hint_filter` :265-276，`$exists:true + $ne:{}` 双条件），`created_at:-1` 排序（:278-280），投影仅 `reaction_analysis`（:282-284），limit=`REACTION_HINT_LIMIT`=3（:260）。
- best-effort：find/collect 出错 → warn + 返回空串（:242-245, :249-252），不阻塞决策。
- `extract_reaction_analyses`（:288-292）：丢弃缺字段/类型错的行，交 `format_reaction_hint` 渲染。

#### ReplyContextCache（decision.rs:297-424）
Gateway 单 run 懒加载缓存，三组 `parking_lot::Mutex<Option<...>>` 单元：
- `context_assets`（:325-348）：按 tier（Lean/Relational/Full 各一格）缓存 `(可引用资产文本, 禁语文本)`，miss 时 `load_context_assets`，DB 失败 `unwrap_or_default`。
- `relational`（:350-394）：缓存 `RelationalReplyContext { reaction_hint_text, operator_memory_text }`；operator memory 在 shadow run（`current_run_mode()=="shadow"`）用只读加载 `load_operator_memory_read_only`，否则 `load_operator_memory`（:361-379），两 future `tokio::join!` 并行（:384-385）。
- `full`（:396-423）：缓存 `FullReplyContext { recent_media_text }`——最近 10 条 media 发送记录渲染。

#### load_reply_prompt_snapshot（decision.rs:438-477）
三层 prompt（`user.reply.system` / `user.reply.policy` / `user.reply.fast.task`）经 `load_prompt_for_contact`（带 contact wxid + locale）`tokio::join!` 并行加载；错误优先级保持 system → policy → task（:465-468 依次 `?`）。返回含版本号的 `ReplyPromptSnapshot`（:426-434）。

#### DecisionRunSnapshot（decision.rs:479-491）
Gateway 固定的 run 级配置快照：`active_profile` / `active_products` / `published_soul`（仅当 profile 无 soul override 时为 Some）/ `sendable_assets` / `referral_cards` / `reply_prompts` / `reply_context`。Shadow/Simulation 传 `None` 走各自加载。

#### render_reply_history（decision.rs:496-543）
- 输入 recent_messages 为 newest-first（调用方语义），`iter().rev()` 转 oldest-first（:501）。
- 当前 inbound 在历史里出现时正文置空串（:505-510，专用最新消息槽全文渲染，不重复计预算）→ 渲染为 `[见下方最新消息]`（:534-535）。
- 其余行经 `history_prompt_content` 剥哨兵 + `budget_history_contents`（单条上限 `HISTORY_MESSAGE_MAX_CHARS`、总预算 `REPLY_HISTORY_TOTAL_CHARS`，newest-first 预算，超预算老行变 None 被 `filter_map` 丢掉 :537 `safe?`）。
- **序号不变量**：`enumerate` 在过滤前进行（:523），省略行造成编号缺口而非重编号——`tagEvidenceTurns` 等证据序位引用原始窗口位置（测试 :569-586 锁死）。
- 每行格式 `[index] 客户/我方 (temporal): content`，temporal 含 createdAtMillis/ageHours/temporalStatus（:529-532）。

#### decide_reply_with_promote（decision.rs:589-1370）——Reply 决策主入口
签名（:589-608）：17 个参数，返回 `(AgentDecision, Vec<String> promote_risks)`。`decide_reply`（薄壳）不存在于本文件——文件头注释提及的 `decide_reply` 实际由调用方直接用本函数（simulation 等丢弃 risks）。¹（见 §5 疑点 1）

**三档裁剪总纲**（:610-625 注释 + 代码）：
- `include_relational = tier ∈ {Relational, Full}`（:620-624）；`include_business = tier == Full`（:625）。
- 恒注入组：soul/三层 prompt/task 契约/history/deprecated_facts/请示通道信号/最新消息/客户身份字段。
- 关系组（Relational+Full）：完整 memory/memory_card/意图轨迹/反应提示/运营记忆/画像各字段。
- 业务组（仅 Full）：知识/知识路由/产品目录/持有投影/疑似成交/可发素材完整清单/名片完整清单/运营方法/域策略/状态机/硬运行参数。
- 例外：内容资产按每条 `min_inject_tier` 分档；禁语恒注入；素材/名片"概览"仅 Lean/Relational（与完整清单互斥）。

**逐步流程**：
1. `active_profile` 加载（:632-641）：优先 run_snapshot → shadow snapshot → `load_active_domain_profile`。
2. **Soul**（:644-650）：`non_empty_override(profile.soul_override)` 优先；否则 run_snapshot.published_soul；否则 `load_published_soul`（:1743-1753，经 `soul_versions::load_unique_published` fail-closed——缺失/多指针报错，注释 :642-643）。
3. 内容资产（:653-663）：run_snapshot 走 `reply_context.context_assets`，否则 `load_context_assets`（失败 → 默认空）。
4. 可发送素材（:671-698）：**恒加载**（任何档，:664-670 注释：让 Lean 档也能看到概览催升档）；`filter_sendable_candidates` 按 contact.domain_attributes.customer_stage 过滤 target_stages（:677-685）；Full 档渲染完整清单 `sendable_candidates_text`（:687-691），非 Full 渲染概览 `sendable_overview_text`（:694-698），两者互斥。
5. 名片引荐（:702-747）：`assist_on = referral::assist_mode_active(domain_config.assist_mode_enabled, contact 级 override)`（:702-709）；assist 开才加载 referral_cards（:710-719）；`already_referred` 从 domain_attributes 的 REFERRED_CARD_ID_ATTR 解出（:725-737）；Full 档 `referral_block` 完整清单，非 Full `referral_overview`（:738-747）。
6. Full-only 懒加载（:748-768）：`full_context.recent_media_text`（最近 10 条已发媒体）。
7. **辅助模式两段注入**（:769-794）：`assist_escalation_hint` 仅 assist_on && !include_business（Lean/Relational 催升档，:785-789）；`assist_redline_yield` 任何档只要 assist_on（:790-794）——对抗 prompts.rs:1142/1165 两句恒注入反向承接红线（注释 :771-784）。
8. 运营方法（:798-807）：`non_empty_override(profile.methodology_override)` 整段替换；否则 playbook 经 `format_playbook_for_prompt`；否则固定兜底文案。仅 Full。
9. domain_text / state_machine_text（:808-821）：`format_operation_domain_config_for_prompt` / `format_operation_state_machine_for_prompt`（JSON 序列化状态机）。仅 Full。
10. 知识（:822-834）：`format_operation_knowledge_for_prompt_with_roles(chunks, profile.chunk_roles)` + `format_knowledge_route_for_prompt`（见下）。仅 Full。
11. deprecated_facts（:838-865）：memory_card.deprecated_facts 中 Structured 变体，按 deprecated_at 降序 take(5)，只序列化 id/text/deprecation_reason/deprecated_at。恒注入。
12. `safety_donts_commitments_text`（:872 → :1695-1713）：**仅 Lean 档**注入 doNotDo+commitments 安全子片（Relational/Full 已含在 memory_text 里，返回空串避免重复；恒注入铁律测试 :2274-2327）。
13. memory_text（:874-885）：Relational+Full 才序列化 `{memoryCard: context_pack, userUnderstanding, relationshipState, productFit, nextAction}`。
14. rewrite_text（:889-892）：`review::render_style_continuity_hint(rewrite_instruction, contact.last_outbound_style)`——显式 Reviewer 改写指令排前、风格弱参考排后。
15. intent_trajectory_text（:897-901）：Relational+Full，`format_intent_trajectory_hint`（最近 5 项）。
16. 交易三段（:914-940）：`active_products` 仅 include_business && profile.transaction_facts_enabled 才加载（G4 #5 交易域闸）；`render_transaction_facts_sections(enabled, products, contact.outcome_events, now)` 返回（目录/持有/疑似成交）三元组——注意 `_suspected_deal_text` 在此**被丢弃**（:928，下划线变量），疑似成交指引实际由 :1030 的 `render_suspected_deal_reply_guidance` 注入 task。²（§5 疑点 2）
17. relational_context（:943-979）：reaction_hint + operator_memory（同 ReplyContextCache 逻辑，非 Gateway 路径直接加载）。
18. **三层 prompt 载入与 override**（:982-1027）：run_snapshot 或 `load_reply_prompt_snapshot`；记录 prompt 版本到 RunBudget（:996-1000）；system 层 frozen/append（:1001-1006）；policy 层顺序：frozen base（:1016-1018）→ `strip_projection_only_policy_section`（:1019，剥「## 标签与画像」投影尾段）→ `apply_reply_policy_prompt_overrides`（:1020-1021，四步固定顺序，见 domain_profile.rs）→ append（:1022-1024）；task 层 frozen（:1025-1027）。
19. **task 追加段**（:1028-1041）：`render_suspected_deal_reply_guidance(active_products)` + 写死的「通用事实来源边界（跨行业硬约束）」+「最近聊天窗口序号补充」两段（:1031-1040，全文 verbatim 在 :1033-1040）。
20. `apply_conversation_mode_enum_list`（:1048-1051）：task 里竖线枚举随 profile.conversation_modes 替换；再 task append override（:1052-1054）。
21. operator_instruction（:1060-1071）：contact.custom_agent_instructions 非空时包裹为「运营关于本联系人的特别指令（最高优先级，覆盖 Soul + Policy）」，**末位注入利用 recency bias**（注释 :1055-1059）。
22. business_context（:1079-1082）：profile.prompt_fragment 渲染，header「# 本行业业务上下文（运营配置，补充 Soul + Policy）」。
23. **系统提示总装**（:1083-1089 → assemble_system_prompt :1676-1684）：`{soul}\n\n{system_contract}\n\n{policy}{business_context}{operator_instruction}`——五层固定顺序 Soul → System Contract → Policy → Business Context → Operator Instruction，后两段自带 `\n\n` 前缀（测试 :2164-2181 锁分隔符）。
24. temporal_fact_view（:1090-1097）：服务端生成的时间/预约授权视图。
25. history（:1098）+ pending task 文本（:1099-1103）。
26. 画像字段组（:1106-1179）：agent_profile/memory_summary/tags（`render_tags_for_prompt` :1654-1667 双层标注「运营确认标签（权威）」/「AI 判断标签（可能调整）」）/customer_stage/intent_level/purchase_lifecycle/value_tier/commitments（最后一条）/follow_up_policy/profile_attributes——全部 Relational+Full。
27. **user prompt 总装**（:1180-1313）：占位顺序 verbatim（:1181-1256）——task_template → 当前运营方法 → 用户运营域策略 → 运营状态机 → 长期运营记忆 → 最近5条已弃用记忆(+安全子片) → 产品知识 → 知识路由 → 产品目录 → 客户当前持有 → 意图轨迹 → 最近用户反应 → 请示通道信号（`escalation::build_decision_signals_text`，含 `effective_negative_outcomes(profile.outcome_polarity)` :1270-1274）→ 运营偏好记忆 → 改写要求 → 客户 wxid/昵称/运营备注/当前画像/长期记忆/标签/客户阶段/意向等级/购买生命周期/客户价值层级/最近承诺/跟进策略/自由画像字段 → 可引用内容资产 → 素材(清单|概览) → 已发素材 recent_media → 禁语 → 可引荐顾问(名片块|概览|催升|让位 4 拼) → 未完成跟进 → 时间/预约授权视图 → 最近聊天 → 最新消息（`inbound_prompt_content(content, is_synthetic_relay)`——合法 relay 保留哨兵，客户消息剥哨兵 :1307-1312）。
28. LLM 调用（:1315-1325）：`generate_agent_json(state, ws, account, wxid, run_id, "user.reply.fast.task", system, user)`。
29. **promote**（:1333-1345）：`serde_json::from_value::<RawAgentDecision>` → H9：profile.conversation_modes 非空时覆盖 runtime.allowed_conversation_modes（:1338-1344）→ **`raw.validate_reply_critical(&runtime_for_promote)`**（:1345，注意不是 validate_and_promote——fast reply 契约只验发送关键字段，见 types.rs）。
30. **taxonomy 归一**（:1355-1364）：`decision_dimension_kinds(profile)` → `validate_and_normalize_decision`（alias 改写发生在 reviewer 之前，:1346-1354 注释）→ risks 并入 promote_risks。
31. `normalize_domain_signals`（:1368）：typed ↔ 容器镜像。返回 `(decision, promote_risks)`。

#### 配置加载器（decision.rs:1372-1569）
- `load_operation_playbook_for_contact`（:1372-1412）：contact.playbook_id 指定且 published → 用之；否则该 account 的 published+is_default 最新一条。
- `load_user_operation_domain_config_for_contact`（:1442-1490）：**SR-008 唯一 current 语义**——`current_version:true` 恰 1 条返回；>1 条 `Conflict("multiple_current_operation_domain_configs")`；0 条但 scope 有历史 → `Conflict("missing_current...")`；scope 全空 → `Ok(None)`。
- `initial_operation_state_for_contact`（:1425-1435）：H13，经 `guards::initial_operation_state_key` 取标 `initial:true` 的 state key。
- `load_operation_state_policy_for_contact`（:1501-1569）：同形唯一 current loader；额外校验 `status=="active"`（:1531-1535 否则 Conflict inactive）；state 无任何版本但 domain config 存在时也 fail-closed（:1559-1565，防状态保护门静默失效）。

#### 渲染纯函数（decision.rs:1571-1741）
- `format_operation_domain_config_for_prompt`（:1571-1590）：名称/目标/方法论/工作流/工具边界/自动化策略/复盘规则/运行参数 8 行。
- `format_knowledge_route_for_prompt`（:1635-1650）：序列化后 **remove `toolTrace` / `evidenceExcerpts` / `selectedChunkRankings` / `reason`** 4 键（reason 防知识 Agent 越权承接措辞回流，注释 :1625-1634）。
- `assemble_system_prompt`（:1676-1684）/ `render_safety_donts_commitments`（:1695-1713）/ `render_business_context_fragment`（:1723-1729，trim 后空 → 空串）/ `non_empty_override`（:1736-1741，trim 后空视为 None）。

#### 资产/名片 query 形状（decision.rs:1759-1983）
- `load_sendable_assets`（:1759-1783）：filter=`build_sendable_assets_filter`（:1928-1939，workspace + account(null|=) + sendable:true + review_status:"approved"），updated_at 降序 limit 30。
- `build_referral_cards_filter`（:1787-1801）：enabled:true + review_status:"approved"；`load_referral_cards`（:1806-1830）limit 20。
- tier 可见性（:1833-1866）：`tier_rank` Lean=0<Relational=1<Full=2；`asset_visible_at_tier` min_tier None/非法按 "full"；`visible_min_tiers_for` 派生 $in 集合。
- `build_referable_assets_filter`（:1890-1911）：kind ∈ {text,faq,script,brand_voice}，Full 档 tier_cond 带 `$exists:false` 兜底（老数据按 full），非 Full 仅显式 $in。
- `build_forbidden_assets_filter`（:1915-1925）：kind="forbidden_expression"，**无 tier 无 limit**（恒注入全量）。
- `load_context_assets`（:1941-1983）：两查询 `tokio::try_join!` 并行；可引用 limit 16。

其余 :1985-2721 为 `#[cfg(test)]`（reaction_hint_loader / referral_loader / persona_override / prompt_override / tier_injection / render_assets / context_assets_filter 七个测试 mod），锁 query 形状、层序、字节等价护栏。

### 2.2 `src/agent/review/mod.rs`（4536 行）

#### 模块结构（mod.rs:1-63）
re-export：gates 的 `apply_dual_reviewer_disagreement / apply_revision_fallback / build_reviewer_decision_view / decide_revision / derive_revision_failure / detect_dual_reviewer_disagreement / finalize_review_for_send_at / route_dual_gate / RevisionDecision`（pub(crate)），`contact_has_principal_product_exemption / finalize_review_for_send / review_passed / FinalizeOutcome / GatewayStatusFinal / PendingFinalizeEvent`（pub）；style 的三函数（:26-39）。

#### Independent ClaimGate 数据结构（mod.rs:65-133）
- `CatalogClaim`（:66-74）：product_id + source_quote（候选正文精确子串）+ name/amount_minor/currency/sku 四可选断言字段。
- `AtomicClaim`（:76-91）：source_quote / claim（脱语境语义归一）/ scope（开放语义，非行业枚举）/ subject（ClaimSubject 四值 customer|business|third_party|general，:93-120）/ product_claim / requires_evidence / evidence_refs（只能来自服务端 evidenceCatalog）/ reason。
- `IndependentClaimVerdict`（:122-133）：requires_evidence / reason / claim_kinds（开放标签）/ claims_complete / claims / has_catalog_claims / catalog_coverage_complete / has_non_catalog_evidence_claims / catalog_claims。

#### parse_independent_claim_verdict（mod.rs:135-219）
严格 schema：所有键必填（缺失/类型错 → `AppError::External("claim_gate_schema_invalid:<field>")`）。**一致性交叉校验**（:197-206），任一为真即 `claimConsistency` 错误：
1. `!has_catalog_claims && (catalog_claims 非空 || !catalog_coverage_complete)`；
2. `has_catalog_claims && catalog_claims.is_empty()`；
3. `has_catalog_claims && !requires_evidence`；
4. `has_non_catalog_evidence_claims && !requires_evidence`；
5. `requires_evidence && !has_catalog_claims && !has_non_catalog_evidence_claims`；
6. `requires_evidence != any(claim.requires_evidence)`。
- `parse_atomic_claim`（:221-270）：sourceQuote/claim/scope/subject/reason 必填非空字符串；requiresEvidence/productClaim 必填 bool；evidenceRefs 必填数组（元素非空）。
- `parse_catalog_claim`（:272-326）：productId/sourceQuote 必填；name/currency/sku 必须显式 `null` 或非空串（缺键报错 :293-301）；amountMinor 必须显式 null 或 ≥0 整数（缺键报错 :310）；**四断言字段全 null → `assertedFields` 错误**（:315-317，禁 productId-only 空壳）。

#### run_independent_claim_gate（mod.rs:328-403）
- SYSTEM prompt 写死英文（:340-354）：语义判断非关键词；sourceQuote 必须精确子串；subject 四分类规则；requiresEvidence 开放世界语义（含预约/时间/日程即便措辞为"建议"仍需证据 :345；具体服务承诺需能力+授权证据、透明"先核对"承诺除外 :346）；证据只能取自 evidenceCatalog（客户提问≠肯定答案证据、历史 AI 消息刻意缺席、模型常识非证据 :347）；domainRiskContext 仅助识别语义、不能作证据/降门槛（:349）；catalog 形事实逐字段提取（:350-352）。
- user payload（:375-390）：triggerMessage（relay 保哨兵）/ triggerKind（principal_decision|customer_message）/ candidateReply / domainRiskContext（profile 的 display_name/description/prompt_fragment/transaction_facts_enabled）/ evidenceCatalog / activeCatalog。
- LLM key `user.review.claim_gate`（:397）。

#### 证据目录 build_claim_evidence_catalog（mod.rs:481-606）
- 首源：relay 时 `principal_decision`（:504-521，verdict 从 `verdict=` 行解析且必须在 ALLOWED_PRINCIPAL_VERDICT 闭集 :405-412；authorizationMode= approved|conditional→"affirm_or_condition"、rejected→"deny_only"、其它→"none" :414-423）；非 relay 时 `current_user_message`（:522-537，带 temporalFresh/statementForm/temporalAuthorized）。
- 历史客户入站（:538-567）：**只取 Inbound**（历史 AI 出站刻意排除，测试 :2373-2425）；排除当前 inbound；按 created_at/id/message_id 降序归一（生产 newest-first 与 shadow oldest-first 一致化 :545-554）；take(12)，id 形如 `recent_user_message:{index}`。
- verified knowledge（:569-593）：仅 `used_knowledge_ids ∩ chunks` 且 `guards::is_verified(chunk, evaluated_at)`；text 取 source_quote ?? body ?? summary。
- active catalog（:594-604）：每产品一条 `catalog:{product_id}`，authorityBoundary 限定"仅产品身份/精确价格/币种/SKU"。

#### 服务端硬化 harden_evidence_claims（mod.rs:844-910）
对模型 verdict 的确定性再判：
1. `claim_is_deterministically_non_evidentiary`（:728-816）：非 product_claim 且 scope 命中白名单（acknowledg/greeting/empathy/apology/negative_action/no_action/contact_invitation）且 source_quote 命中社交/否定/联系邀请短语，且**整句剥掉这些短语后只剩标点空白**（contains_only_phrases_and_punctuation :697-723）→ 强制 requires_evidence=false 并清空 refs（纠模型假阳性）。
2. `claim_is_temporally_sensitive`（:608-648）：scope 或文本命中预约/时间词，或"数字+时间单位"启发式 → 强制 requires_evidence=true，refs 仅保留 `temporal_evidence_ref_is_current_fact`（:818-840）通过者——聊天源需 temporalFresh && statementForm=="statement" && temporalAuthorized；principal 源需 fresh+authorized+极性匹配；verified/catalog 恒可。
3. `claim_is_concrete_service_commitment`（:650-695）：承诺 scope 或第一人称履约短语（"我带你进去"等）且非透明核对（"先核对"等）→ requires_evidence=true、**subject 强制改写为 Business**（:869，客户聊天不能授予业务履约权），refs 仅留 verified_knowledge 或授权极性匹配的 principal_decision（:871-882）。
4. 命中项把 `temporal_schedule` / `service_commitment` 追加进 claim_kinds（:890-900）；重算顶层 requires_evidence 与 has_non_catalog_evidence_claims（:901-909）。
- `claim_has_explicit_negative_polarity`（:425-468）：scope 负面标记词 或 文本中英否定短语；`principal_decision_authorizes_claim`（:470-479）：仅 Business 主体；affirm_or_condition 恒真、deny_only 需负极性。

#### 完整性校验 + 合并（mod.rs:912-1107）
- `evidence_ref_authorized`（:918-950）：客户陈述源仅授权 Customer 主体；principal 源走极性判定；verified 恒授权（语义蕴含归 AI 闸）；catalog 源需 product_claim 且存在 sourceQuote 完全相等的 catalog_claim。
- `atomic_claim_evidence_refs_invalid`（:953-963）：任一 claim 存在未授权 ref 即 true。**在 harden 之前先算**（apply :1529-1531），防伪造 ref 被 harden 洗白（测试 :2035-2057）。
- `atomic_claim_integrity_failed`（:965-981）：!claims_complete 或 任一 source_quote 不是 reply_text 子串 或 refs 未授权。
- `unsupported_atomic_claims`（:983-989）：requires_evidence 且 refs 空。
- `merge_independent_claim_verdict`（:991-1107）：把 verdict 落进 review.claim_analysis——requiresProductKnowledge = 主 reviewer 判定 OR 独立闸有 requires_evidence 的 product_claim（:997-1006）；requiresBusinessEvidence / unsupportedBusinessClaimCount / unsupportedNonProductBusinessClaimCount / claimsComplete / claimManifest（全字段清单 :1027-1046）/ independentClaimGate 系列元数据（:1047-1076）。**非产品无证据 claim 存在时**（:1083-1106）：approved=false；非 hold 时 needs_revision=false（证据修复走更早的 targeted rewrite，优先级高于 style revision :1085-1089）；hallucination_score = max(6)；追加 `unsupported_business_claim` risk；写定向 rewrite_instruction（引用原句，:1103-1105）。
- catalog 背书判定 `catalog_claims_are_backed`（:1109-1119）：has_catalog_claims && coverage_complete && !has_non_catalog_evidence_claims && catalog_claims 非空 && `catalog_claims_match_reply`。
- `catalog_integrity_failed`（:1121-1133）：回复提到目录产品但模型没报 catalog claim；或报了但 coverage 不完整 / 与 reply 不匹配。
- `catalog_claim_matches_product_reply`（:1163-1219）**服务端逐字段核验**：product_id 匹配 + quote 是 reply 子串 + quote 里含产品名或 SKU；name 断言则必须 == 产品名且 quote 含之，name=null 则 quote 不得含产品名（:1180-1183）；sku 同理（:1187-1194）；amount 断言必须 == 产品价（:1198-1200）；`catalog_fact_remainder`（quote 去掉名/SKU）的全部数字 token 必须匹配 minor 金额（`quote_numbers_match_amount` :1253-1268——断言 Some 时所有数字须等于 major 或 major.minor 形；断言 None 时不得有数字）；币种断言须 == 产品币种且 quote 有该币种记号、无他币种记号；无断言则 quote 不得有任何币种记号（:1205-1217）。
- `catalog_claims_match_reply`（:1135-1161）：所有 catalog_claims 都能配到产品 + **reply 每个提及目录产品的子句都必须被某条 claim 的 normalized quote 覆盖**（防漏报第二句错价，测试 :2976-2984）。
- 三个 hold helper（:1385-1436）：`hold_for_catalog_integrity_failure` / `hold_for_claim_manifest_integrity_failure` / `hold_for_claim_gate_failure` 均置 approved=false、should_hold=true、hold_category=blocked_by_safety_guard、final_review_status="blocked_by_safety_guard"，risk 分别 `catalog_claim_integrity_failed` / `claim_manifest_integrity_failed` / `independent_claim_gate_unavailable`（后者另写 independentClaimGate=false + 截断 160 字符错误）。

#### 评估/应用分离（mod.rs:1438-1586）
- `IndependentClaimGateEvaluation`（:1443-1449）：candidate_reply 快照 + evidence_catalog + outcome（should_reply=false 时 None）。
- `evaluate_independent_claim_gate`（:1452-1499）：gateway 可与最终 reviewer 并行执行；不改 review、不产生授权。
- `apply_independent_claim_gate`（:1505-1557）：**candidate_reply != decision.reply_text → 直接 hold（claim_gate_candidate_mismatch）**（:1511-1520，rewrite/revision 后必须重跑闸）；Ok 分支：先记 original_evidence_refs_invalid → harden → catalog_backed / catalog_failed / manifest_failed → merge → 两 hold 按需（:1543-1548）→ 返回 catalog_backed（供 R5.4 `priced_from_catalog`）；Err 分支 fail-closed hold。
- `ensure_independent_claim_gate`（:1559-1586）：evaluate+apply 串行便捷入口（管理发送用）。

#### parse_live_review（mod.rs:2993-3061）——Reviewer 严格 wire 契约
- approved 必填 bool；scores 必填对象；六 gate 键按 `(canonical, accepted[])` 表校验（:3011-3036）：humanLike / emotionalValue / factRisk(别名对 hallucinationScore) / productAccuracy(别名对 knowledgeGroundingScore) / pressureRisk / boundaryPrivacySafety——**每键恰好出现一个**（同时给 alias+canonical 判 invalid :3029），值必须 0..=10 整数；claimAnalysis.requiresProductKnowledge 必填 bool（:3037-3056）。通过后反序列化并打 `reviewScoreStatus:"valid"`（:3058-3060）。
- `hold_for_review_schema_failure`（:3069-3102）：schema 失败转结构化 hold——scores 全恶化（hallucination=10/pressure=10/其余 0）、requiresProductKnowledge=true、reviewScoreStatus=missing|invalid（按错误前缀 :3071-3075）、risk `review_schema_invalid`、blocked_by_safety_guard 终态。

#### 模式决策（mod.rs:3236-3285）
- `effective_review_mode`（:3236-3259）：force_full || runtime.distrust_self_reported_low_risk || planner.risk_level=="high" || planner.knowledge_required → "full"；decision.operation_state_confidence（默认 10）< runtime.operation_state_confidence_full_review_below → "full"；planner.review_mode=="light" → "light"；否则 "full"。
- `should_run_targeted_rewrite`（:3266-3275）：should_reply && !should_hold && !review_passed && !needs_revision——ClaimGate 证据修复优先于 style revision；安全 hold 绝不再花 LLM。
- `should_run_review`（:3277-3285）：**恒等于 decision.should_reply**——可发送正文永远不能自证跳过评审（旧 needs_review 自报语义已废）。

#### local_decision_review（mod.rs:3300-3365）
- !should_reply → approved=true 满分（无出站体无风险，:3305-3318）。
- 预算耗尽（budget.is_llm_or_token_exhausted）→ approved=false、恶化分、risk `budget_exceeded_no_review`（finalize 映射 blocked_by_budget，:3320-3340）。
- 其它（该跑 reviewer 却没跑）→ approved=false + should_hold + blocked_by_safety_guard + risk `required_reviewer_not_executed`（:3342-3364）。

#### reviewer 上下文渲染（mod.rs:3430-3717）
- `render_reviewer_recent_history_bounded_at`（:3448-3519）：排除 inbound 可选；稳定排序 created_at→id→message_id→direction→content→输入序（:3464-3483）；take 尾部 max_messages；预算裁剪；oldest-first 重新编号（reviewer 不消费 Reply 序位，可安全丢老行 :3430-3433）。full 档上限 FULL_REVIEW_HISTORY_MAX_MESSAGES/TOTAL_CHARS，light 档 LIGHT_*（6 条，测试 :3846-3851）。
- `reviewer_memory_card_text`（:3521-3523）完整 context_pack；`reviewer_operating_memory_text`（:3525-3532）只留 relationshipState/productFit/nextAction（**不含 memoryCard 重复、不含 userUnderstanding 自我推理**，测试 :3751-3799）。
- `reviewer_operator_instruction_text`（:3534-3540）：trim 后空 → "（无）"。
- `light_memory_card_text`（:3560-3577）：白名单 7 键 coreFacts/recentFacts/doNotDo/commitments/objections/deprecatedFacts/conflicts。
- `reviewer_temporal_fact_section`（:3579-3593）：时间授权视图 + 核验要求文案。
- `build_light_reviewer_user`（:3595-3688）：light 档 user prompt——严格 JSON 模板 + 8 条审核规则（:3649-3657，含"客户提问不是答案证据"开放世界规则、boundaryPrivacy 拦截规则、有界快照语义）+ 最新消息 + 时间视图 + ≤6 条历史 + 候选 + 关键记忆 + 联系人指令 + 阈值 doc（factRiskBlockAt 等 5 键 :3616-3622）+ 知识路由摘要（含 evidenceExcerpts take3 + usedKnowledgeIds :3608-3615）。
- `reviewer_recent_history_section`（:3690-3717）：full 档历史段 + 5 条「历史事实核验规则」（长期记忆缺失≠事件未发生、窗口不足判"无法核验"而非虚构）。

#### append_assist_yield（mod.rs:3995-4004）
assist_on → system 末尾追加 `referral::REVIEWER_ASSIST_YIELD_NOTE`（解第三方角色红线 + 名片误判 factRisk 两条 hold 路径）；否则字节等价。

#### ReviewerPromptCache（mod.rs:4029-4061）
light/full 两格懒加载 `user.review.light.system` / `user.review.system`；Shadow/Simulation 传 None 保持隔离。

#### review_decision（mod.rs:4063-4442）——评审主流程
1. !should_reply → 直接 approved 满分返回（:4084-4097）。
2. system 载入（cache 或直读 :4098-4110）→ prompt_override frozen（:4114-4116）→ active_profile（override 参数 → shadow snapshot → 缓存加载 :4119-4131）→ `apply_review_system_prompt_overrides`（:4138-4141，D 评审重点行 + T3 few-shot 段）→ assist 让位追加（:4145-4153）→ prompt_override append（:4154-4156）。
3. 上下文准备（:4157-4197）：runtime_text=`runtime.as_document()`；memory_card_text（完整）+ memory_text（去重三键）；operator_instruction；knowledge_route_text（**复用 decision 侧同一净化函数** :4168）；`build_reviewer_decision_view`（B2 事实面投影，见 gates）；温度视图+历史段（:4178-4183）；`render_business_formulas_json_example` + `render_reviewer_extra_score_lines`（H15/第19点，:4187-4197）。
4. user prompt（:4198-4320）：light 走 build_light_reviewer_user；full 为写死模板——JSON 输出契约（approved/scores{humanLike,emotionalValue,productAccuracy,boundaryPrivacySafety,(extra),pressureRisk,factRisk}/formulaBreakdown/claimAnalysis{hasProductClaim,requiresProductKnowledge,knowledgeSupported,reason}/risks/rewriteInstruction/reviewSummary）+ **21 条评审原则**（:4237-4259，verbatim 摘录：转化平衡；禁虚假稀缺恐惧营销；humanLike/pressureRisk 是硬评分软闸触发 single-shot revision；产品承诺无知识 → 提 factRisk 降 productAccuracy；开放世界一般业务事实同审；知识切片只作导航、产品声明须 verifiedClaims/sourceAnchors/evidenceItems 支撑；claimAnalysis 语义判断非关键词；doNotDo/commitments/coreFacts 违背检查；历史核验规则引用；重复追问抬 pressureRisk；"我发你"无实体降 Reliability；绝对化表述抬 factRisk；boundaryPrivacySafety 三类泄露判 ≤3）+ 槽位（最新消息剥/保哨兵 :4299-4302 → 历史段 → 候选回复 → 决策事实面 → 长期运营记忆 → 长期记忆卡片 → 联系人特别指令（"不得覆盖事实准确、安全、隐私或产品证据硬门"）→ 运营方法 → 域策略 → 硬运行参数 → 产品知识（按 profile.chunk_roles 渲染 :4314-4317）→ 知识路由）。
5. `apply_reviewer_balance_principle`（:4324-4330，user prompt 的 D 取向条）。
6. **双脑并行**（:4331-4427）：primary = generate_agent_json（LRU cache + llm_call_logs）；`state.second_reviewer_llm` 存在时 second = `generate_agent_json_with_provider`（prompt key `user.review.second_provider`，max tokens 按 light/full 选 LIGHT_REVIEWER_MAX_OUTPUT_TOKENS / REVIEWER_MAX_OUTPUT_TOKENS :4362-4366）；`tokio::join!` 并发（:4368）。primary parse 失败 → `hold_for_review_schema_failure` 返回（:4370-4379）；primary 过 `route_dual_gate`（:4385）；second Ok+parse Ok → second 也 route_dual_gate → `detect_dual_reviewer_disagreement` → Some 时 `apply_dual_reviewer_disagreement`（:4391-4408）；**second parse 失败 → 整体 hold_for_review_schema_failure**（:4409-4416）³（§5 疑点 3）；second LLM 调用失败 → 仅 warn 回退 primary（:4417-4422）。
7. 单脑路径（:4425-4441）：primary await → parse → route_dual_gate → 返回。

### 2.3 `src/agent/review/gates.rs`（3317 行）

#### review_passed（gates.rs:20-47）——发送判定统一布尔
`live_scores_are_valid = claim_analysis.reviewScoreStatus == "valid"`（:21-24）。全部 AND：
1. `review.approved`；
2. `hallucination_score < runtime.fact_risk_block_at`（默认 <6）；
3. `human_like >= human_like_rewrite_below`（默认 ≥6）；
4. `emotional_value >= emotional_value_rewrite_below`（默认 ≥6）；
5. grounding 项（:34-36）：`(grounding_gate_bypass_without_claim && !claim_requires_product_knowledge) || knowledge_grounding_score >= product_accuracy_block_below`（默认 ≥7；bypass=false 时退化为纯阈值判定，与 classify_dual_gate:138-147 同源对偶）；
6. pressure 项（:39-41）：`(!live_valid && pressure==0) || (pressure>0 && pressure < pressure_risk_block_at)`（默认 <7；0 仅对老数据豁免——live 有效评分中的 0 视为未评分不通过）；
7. boundary 项（:45-46）：`(!live_valid && boundary==0) || boundary > 3`（0 老数据豁免；1-3 拦截；≥4 放行）。

#### build_reviewer_decision_view（gates.rs:64-81）——B2 事实面
只序列化 shouldReply / matchedKnowledgeIds / safeClaimsUsed / usedKnowledgeIds / objectionsDetected / customerStage / intentLevel / operationState / decisionPhase / autonomyMode / runMode / riskLevel / knowledgeNeed。**不含 reply_text（独立槽注入）与 9 个自我推理字段 + intent_analysis/next_best_action/operating_memory_update**（防 reviewer 追认 reply-agent，测试 :1621-1705）。

#### classify_dual_gate（gates.rs:117-221）——双闸分类精确逻辑
- **硬闸**（优先，:125-150）：`hallucination_score >= fact_risk_block_at` → risk `hallucination_score_{s}_ge_{t}`；grounding_gate_applies（= !bypass || requiresProductKnowledge，:138-139）且 `knowledge_grounding_score < product_accuracy_block_below` → risk `knowledge_grounding_{s}_lt_{t}`。任一命中 → `HardGateFailure{risks}` 短路。
- **软闸**（:152-220）：
  - `human_like < human_like_rewrite_below` → risk `human_like_{s}_lt_{t}` + 中文改写方向；
  - `live_valid && pressure==0` → risk `pressure_risk_0_unscored`（live 明确 0 分=未建立安全性）；`pressure!=0 && pressure >= pressure_risk_block_at` → risk `pressure_risk_{s}_ge_{t}`（方向文案刻意"问句增减由你按语境判断"，不预判 :179-184）；
  - `emotional_value < emotional_value_rewrite_below` → risk `emotional_value_{s}_lt_{t}`；
  - `(live_valid || boundary!=0) && boundary <= 3` → risk `boundary_privacy_safety_{s}_le_3`（:200-212）。
  - 无软 risk → AllPass；有 → `SoftGateFailure{direction=各段拼接, risks}`。

#### route_dual_gate（gates.rs:237-277）
`approved = review_passed(...)` 先按老语义写（:245-246）；SoftGateFailure 时：revision_direction 空则填分类方向（reviewer 自带方向不覆盖 :256-258）→ 追加 `reply_objective_features(reply_text)`（:262-268，问句数/字数/共情词命中数三客观量，10 共情词表 :297-308，**只报数不判罚** :279-289 注释）→ needs_revision=true → risks 保序去重追加。Hard/AllPass 不加 risks 不设 needs_revision。

#### 双 reviewer 分歧（gates.rs:317-428）
- `DualReviewerDisagreement`：ApprovedMismatch（review_passed 布尔不同，最高优先）/ DualGateMismatch（分类变体不同）/ SoftRiskDelta（双方软闸但命中子项集合不同，排序后比较 :392-403）。双 AllPass、双 Hard、双 Soft 同集合 → None。
- `apply_dual_reviewer_disagreement`（:416-428）：needs_revision=true；空方向填枚举内置方向文案（:346-361）；risks 去重追加 `reviewer_dual_disagree:{approved_mismatch|dual_gate_mismatch|soft_risk_delta}`。

#### GatewayStatusFinal / FinalizeOutcome（gates.rs:454-558）
- 枚举：Approved / BlockedByRequiredField / BlockedByBudget / BlockedUnverifiedProductClaim / BlockedBySafetyGuard / Held(String)。`gateway_status_str`（:481-492）与 `final_review_status_str`（:495-500）一一对应（revision 路径的 `revision_applied_approved` 改写归 gateway task 3.4）。
- `PendingFinalizeEvent`（:510-516）：kind/status/summary/details，由 gateway 持久化。
- `contact_has_principal_product_exemption`（:548-558）：domain_attributes.<PRINCIPAL_PRODUCT_EXEMPTION_ATTR>.granted == true；缺任何一层 → false（fail-closed）。

#### finalize_review_for_send(_at)（gates.rs:589-1044）——最终安全汇总，硬门全序
纯函数（不写库不调 LLM），`_at` 变体注入 evaluated_at（Prompt Shadow 双分支同刻）。**判定顺序**：
1. `extend_risks_unique(review.risks, promote_risks)`（:637）。
2. **R3.5/R3.6 协议硬违规**（:640-665）：`has_protocol_violation`——前缀 ∈ {`missing_required_field:`, `invalid_enum_value:`, `invalid_type:`, `decision_phase_invalid:`}（is_protocol_violation_tag :1063-1068）→ approved=false、should_reply=false、autonomy_mode="blocked"、事件 `autonomy_field_violation`、终态 **BlockedByRequiredField**，return。**刻意不含 insufficient_detail**（:1057-1062 注释，t15 跌单弧根因）。
3. **R1.5 insufficient_detail 降级**（:667-698）：`has_insufficient_detail_only`（有 `insufficient_detail_in_critical_turn:` 且无结构违规 :1080-1083）&& should_reply && **!hard_gate_failed**（:684-688，classify_dual_gate 非 HardGateFailure——安全不变量：硬闸失败绝不被降级矫正）→ needs_revision=true + 空方向时填"补全推理痕迹"方向。不 return（继续走后续门）。
4. **R3.7 预算**（:701-721）：risks 含 `budget_exceeded_no_review` → blocked、事件 `budget_exceeded_no_review`、终态 **BlockedByBudget**，return。
5. **非产品证据发送时效复验**（:723-785）：`compute_verified_chunks(used_ids, chunks, evaluated_at)` 得 finalize 时刻 verified 集合；claimManifest 中 requiresEvidence && !productClaim 的 claim 的 evidenceRefs 里有 `verified_knowledge:` 前缀但不在集合内（=评审后过期）→ approved=false、hallucination=max(6)、risk `business_claim_evidence_expired_before_send`、事件 `business_claim_evidence_expired`、终态 **BlockedBySafetyGuard**，return。
6. **开放世界业务证据门**（:787-824）：claim_analysis.unsupportedNonProductBusinessClaimCount > 0 → 同上恶化 + risk `unsupported_business_claim` + 事件 `unsupported_business_claim_blocked`（details 含 manifest）→ **BlockedBySafetyGuard**，return。
7. **R5.4 产品声明强约束**（:826-881）：`claim_requires_product_knowledge(claim_analysis)`（camel/snake 双键 doc_bool，guards.rs:530-533）为 true 时——verified_chunks 为空 **且** !priced_from_catalog **且** !principal_product_exempted（三路并联背书取或）→ approved=false、hallucination=max(6)、risk `product_claim_without_verified_knowledge`、事件 `product_claim_blocked`、终态 **BlockedUnverifiedProductClaim**，return。
8. **grounding 漏判观测探针**（:883-948，非拦截）：reviewer 未自报 requiresProductKnowledge 且 `commitment_claim_class(reply_text, markers)` != None 且 verified 交集空 → 落 `grounding_probe_reviewer_missed` 事件（status="observe"，ProductEffect/ToneOnly 两文案），**不改任何判定**。
9. **R2.6 hold 校验**（:950-984）：`assert_hold_category_valid`（types.rs）矫正非法 hold_category → Coerced 时事件 `autonomy_hold_category_invalid`；should_hold=true → should_reply=false、final_review_status=category、终态 **Held(category)**，return。
10. **末端四分支**（:986-1043）：
    a. approved && should_reply → final_review_status="approved"，**Approved**；
    b. needs_revision && 方向非空 && !should_hold && should_reply →（soft-gate-only 失败）**approved 矫正回 true**、"approved"、**Approved**（供 decide_revision Proceed；硬闸失败到不了这里因 route_dual_gate 不会设 needs_revision :999-1006）；
    c. approved && !should_reply →（A3 主动沉默）"approved"、**Approved**（gateway 按 should_reply 分流 no_reply :1014-1032）；
    d. 其它（approved=false 无硬门）→ final_review_status="held_by_ai_policy"、**Held(held_by_ai_policy)**。

#### revision 控制流（gates.rs:1094-1275）
- `decide_revision`（:1163-1196）：finalize 非 Approved → NotEligible；!needs_revision → NotEligible；should_hold → NotEligible；方向 trim 空 → Skip{reason:"revisionDirection_empty", event:"revision_skipped_invalid_direction"}；budget_exceeded（调用方从 RunBudget 算好传入）→ Skip{"budget_exceeded_before_revision","revision_skipped_budget_exceeded"}；否则 **Proceed**（调 Reply Agent 第二次）。
- `derive_revision_failure`（:1218-1223）：任意 reason → `(reason, Held(held_by_ai_policy))`；finalReviewStatus="revision_failed" 由调用方写。
- `revision_fallback_is_safe_style_only`（:1232-1254）：risks 含 `reviewer_dual_disagree:` 前缀 → false；classify=Hard → false；classify=Soft → 全部 risk 前缀 ∈ {human_like_, emotional_value_} 才 true（pressure/boundary 失败不可回退）；classify=AllPass → 仅当 risks 含 `style_diverged`（机械风格检测是唯一触发源）。
- `apply_revision_fallback`（:1258-1275）：allow → approved=true、revision_applied=false、"revision_applied_approved"、finalize_status=Approved（发原稿）；不 allow → approved=false、"revision_failed"、Held(held_by_ai_policy)。

其余 :1277-3317 为测试（revision_fallback / review_passed_dual_gate / reviewer_decision_view / dual_gate_classification / dual_reviewer_disagreement 五个 mod），覆盖 M1 管理发送两不变量（:3058-3132：review_passed 会放行的无背书产品声明由 finalize 拦；软闸 Approved 由 `&& review_passed` guard 挡）。

### 2.4 `src/agent/review/style.rs`（315 行）
- `extract_outbound_style_fingerprint`（:18-63）：长度桶 xs≤30/s≤80/m≤200/l + emoji（U+1F300..1FAFF|2600..27BF）+ qmark + excl + 句末符号类（跳过尾部 emoji/空白后归类 q/e/./~/x）+ 换行数 min(9)。输出 `len:s|emoji:0|qmark:1|excl:0|tail:.|nl:0`。
- `observe_style_continuity`（:77-101）：任一为空 → NoSignal；按 `|` 分段逐位比较，差异段数 + 段数差 ≥3 → AuditOnly（**仅审计，无 Revise/Block 变体** :65-71）；<3 → NoSignal。
- `render_style_continuity_hint`（:135-164）：无历史指纹 → rewrite 原文逐字返回（含边界空白）；`stable_hint_values`（:114-128）只认 len∈{xs,s,m,l}+emoji∈{0,1}+nl≤9，损坏指纹不注入；有效时生成"风格连续性弱参考（不得覆盖本轮语义）……"提示，rewrite 非空则 `{rewrite}\n\n{hint}`（显式改写优先）。**qmark/excl/tail 刻意不进提示**（:147-148）。

### 2.5 `src/agent/guards.rs`（1214 行）
- `normalize_decision_state`（:17-35）：decision.operation_state 非空且不在状态机 key 中时，按 name 反查 key 归一（`operation_state_key_by_name` :97-105）。
- `normalize_decision_runtime`（:42-46）：memory_write_score==0 且 operating_memory_update 非空 → 回填 planner.memory_change_importance。
- `planner_from_decision`（:48-68）：risk_level 空 → "medium"；`decision_requires_knowledge`（:70-75，knowledge_need ∈ {required, insufficient, knowledge_required}）；review_mode = needs_review||high||knowledge_required ? "full" : "light"。
- `operation_state_exists`（:86-95）：**states 空时返回 true**——#155(P2) 有意 fail-open，仅用于 normalize 是否归一，真正迁移闸在 check_state_transition（:78-85 注释）。
- `action_policy_state_key`（:115-134）：proposed 存在于状态机且（== current 或迁移合法）才用 proposed，否则回落 current——防用不存在态查 policy。
- `initial_operation_state_key(_in_machine)`（:155-173）：取 `states[].initial==true` 的 key，缺省回落 `"new_contact"`。
- **check_state_transition**（:187-254）：
  1. domain_config=None → None（simulation fail-open）；
  2. states 空 → Some("state_transition_invalid: state_machine_empty…")（S1.2 fail-closed）；
  3. 目标 key 不存在 → Some("… unknown_target to=…")（问题 E 修复：防 CandidateNew 幻影态旁路 policy，:212-221 注释）；
  4. 目标 allowFromAny → None；
  5. from 空（None/空白）→ 目标标 initial 才 None，否则 Some("from=<empty>")；
  6. 否则 from 必须 ∈ 目标 allowedFrom，否则 Some("from={f} to={t}")。
- action 词表 `OPERATION_STATE_ACTION_VALUES`（:260-266）：reply/acknowledgement/silent/follow_up/cooldown 闭集。
- `classify_decision_action`（:279-298）：should_reply→"reply"；follow_up.needed→"follow_up"；cooldown_until 非空→"cooldown"；否则 "silent"。
- `classify_reviewed_decision_action`（:306-454）：post-review 的窄 acknowledgement 判定——须同时满足：review.approved && !should_hold；文本含确认词（:314-329）或**仅负向排程**（SchedulePolarity=NegativeOnly——负向短语剥离后剩余文本再查正向排程词 :375-435，Mixed/Positive 一律 "reply"）；≤60 字；无数字无问号；零副作用（无 follow_up/commitment/last_commitment/cooldown/assets/namecard/escalation :330-348）；ClaimGate manifest 安全（independentClaimGate && claimsComplete && 所有 claim requiresEvidence=false :349-366）。否则 "reply"。
- `enforce_state_action_policy`（:466-494）：policy None 或 status!="active" → Ok；forbidden 命中 → Err("state_action_forbidden: state=… action=…")（优先级最高）；allowed 非空且不含且 **action != "acknowledgement"**（legacy 白名单豁免，显式 forbidden 仍赢 :482-487）→ Err("state_action_not_allowed:…")；否则 Ok。
- `is_verified`（:517-526）：integrity_status trim 后**精确等于小写 "verified"**（KB-03 与写入/召回侧口径统一，大写不认）且 valid_to None 或 ≥ now。
- `compute_verified_chunks`（:541-571）：used_ids trim 去空 → 与 chunks 求交（hex ObjectId），仅 verified，按 chunks 原序去重。
- `CommitmentClass`（:574-582）：ProductEffect（效果/数据类，漏判+无 verified 曾设想硬拦、现仅观测）/ ToneOnly / None。
- fallback 词表（:590-592）：PRODUCT_EFFECT_MARKERS=["成功率","见效","回款","百分之","百分百"]；TONE_ONLY_MARKERS=["保证","一定能","绝对"]。
- `commitment_claim_class`（:603-631）：markers（profile.commitment_markers）两组空时回落 const；ProductEffect 优先于 ToneOnly；空文本 None。
- 测试（:633-1214）：policy_tests / cross_domain_state_machine_tests（医疗 FSM 验证引擎行业无关，G09）/ is_verified_tests（时效真值表 + 大小写收窄哨兵）。

### 2.6 `src/agent/types.rs`（2946 行）

#### 基础类型
- `OutboundSendError`（:23-29）SafeToRetry/DeliveryUncertain；`DeliveryVerification`（:34-39）Delivered/NotDelivered/Inconclusive（缺证据≠NotDelivered）。
- `GeneratedOperationProfile`（:62-74）。`ToolCallRequest`（:83-90）。`AgentSignal`（:97-108，R8 自由信号，不参与聚合）。
- 默认值函数：`default_decision_phase`="final"（:112-114）；`default_conversation_mode`="casual_relationship"（:116-119，最保守）。

#### AgentDecision（:121-313，Default :354-430）
全字段（serde camelCase）：run_mode / risk_level / knowledge_need / needs_review / should_reply / reply_text / profile_update / tags(string_or_vec) / tag_evidence_turns / stage_evidence_turns / stage_explicit_intent / bayesian_observations / customer_stage / intent_level / **domain_signals**(Document, H1 开放容器 :156-165) / dimension_display_names(:166-173) / last_commitment / commitment(CommitmentDecision) / follow_up_policy / profile_attributes / intent_analysis / next_best_action / operation_state / operation_state_reason / operation_state_confidence(optional_i32) / cooldown_until / product_fit_score / matched_knowledge_ids / safe_claims_used / forbidden_claim_risk / objections_detected / recommended_resource_ids / operating_memory_update / memory_candidates(document_vec) / memory_write_score / consolidation_needed / used_knowledge_ids / quoted_product_ids(:211-217，历史兼容，**不作目录背书授权**) / memory_update / context_pack_version / follow_up / **9 自治协议字段**(user_understanding/relationship_read/operation_goal/knowledge_need_reason/memory_update_reason/self_critique/why_should_reply/why_skip_reply/risk_self_check :229-246) / autonomy_mode / decision_phase(default "final") / tool_calls / agent_generated_signals / conversation_mode(default casual_relationship) / conversation_mode_reason / escalation_request / assets_to_send / namecard_to_send / sufficiency / missing_tier / clarification_intent（:303-312）。
- `AssetSendDirective`（:317-324）asset_id+reason；`NamecardDirective`（:329-336）card_id+reason；`BayesianObservationRaw`（:341-352）dimension/value/confidence(f64)/evidence_turns。

#### RawAgentDecision（:445-554）
全 Option<T> 边界结构（区分"未输出"与"输出空/false"）；customer_stage/intent_level 带 `#[serde(alias)]` 接受 snake_case（:502-505，D-01 修复）。

#### DeferredProjectionDecision（:556-678）
发送后投影契约：**FORBIDDEN 11 键**（replyText/shouldReply/needsReview/review/assetsToSend/namecardToSend/escalationRequest/lastCommitment/commitment/followUp/toolCalls，:591-603）出现即拒；`unknown_fields`（:616-646）报未知分析键但不拒；`into_agent_decision`（:649-677）截断 tags24/bayes6/memory_candidates6/signals12、memory_write_score clamp 0..10、其余走 Default（**不可能授权发送**：should_reply=false 等，测试 :2529-2546）。

#### validate_and_promote 体系（:680-1393）
- 枚举闭集常量（:711-730）：RISK_LEVEL=[low,medium,high]；KNOWLEDGE_NEED=[not_required,required,insufficient]；RUN_MODE=[fast_chat,memory_candidate,knowledge_grounded,high_risk]；AUTONOMY_MODE=[auto,assisted,blocked]；CONVERSATION_MODE=[casual_relationship,value_exchange,consultative,boundary_protection]；ALLOWED_TOOL_NAMES=[knowledge.list_catalog,knowledge.search,knowledge.open_slice]。
- 校验原语：`check_required_string`（:746-754，trim 空 → `missing_required_field:<n>` + 空串）；`check_required_enum`（:758-782，None/空→missing；非法→`invalid_enum_value:<n>:<v>` + 空串）；`check_required_bool`（:786-794，None→missing+false）；`is_valid_reply_reason`（:1165-1170，≥min_chars unicode 且 ≥min_hanzi 汉字）；`count_hanzi` U+4E00..9FFF（:738-742）。
- **validate_reply_critical**（:799-889，fast reply 精简契约）：phase 解析（tool_calling 不认，非 final/空即 `decision_phase_invalid`）；allowed_modes 从 runtime 或 const；五枚举 + operation_state + needs_review/should_reply 必填 + risk_self_check 必填；should_reply=true 且 reply_text 空 → missing reply_text；should_reply=false 且 why_skip_reply 空 → missing；`build_minimal_decision` + 覆盖发送关键字段 + **clear_deferred_fields**（:1131-1162：清空 profile_update/tags/证据序位/bayes/stage/domain_signals/display_names/follow_up_policy/profile_attributes/intent_analysis/next_best_action/cooldown/product_fit/objections/recommended/operating_memory_update/memory_candidates/memory_write_score/consolidation/memory_update/agent_generated_signals/7 推理字段——投影字段即使老 prompt 输出也被丢弃，仅保留发送指令类 assets/namecard/commitment 等 carry-through）。**注意 R1.4 的 why_should_reply 在此路径不校验**（只查 skip 侧），R1.3 七字段除 risk_self_check 外均不校验。
- **validate_and_promote**（:894-1128，完整契约）：
  1. `!runtime.autonomy_protocol_enabled` → build_minimal_decision + 空 risks（R11 sunset :901-903）。
  2. phase 解析（:908-915）：tool_calling 认；非法值 → risk + 回落 final。
  3. **tool_calling 中间轮**（:918-927）：只验 tool 名闭集（非法 → `invalid_tool_call:<t>`）；`build_tool_calling_decision`（:1174-1187）强制 reply_text="" + should_reply=false；跳过 R1/R3 全部校验。
  4. final 轮：五必填枚举（conversation_mode 集合可被 runtime.allowed_conversation_modes 覆盖，空回落 const :960-968）+ needs_review/consolidation_needed 必填 bool + operation_state 必填串。
  5. R1.3：7 字段必填（user_understanding/relationship_read/operation_goal/knowledge_need_reason/memory_update_reason/self_critique/risk_self_check :985-1010）。
  6. R1.4 互斥（:1013-1023）：should_reply（unwrap_or false）侧的 why_*_reply 须 `is_valid_reply_reason(…, 10, 6)`（≥10 unicode ≥6 汉字），否则 missing。
  7. R1.5/R1.6 条件长度（:1026-1082）：`is_low_routine = low && not_required && !consolidation_needed`；`is_critical_turn = high || run_mode==high_risk || required || insufficient || consolidation_needed`。critical：7 字段禁 "unchanged" 且 ≥20 unicode chars（空的已由 R1.3 标记不重复 :1046-1048），命中 → `insufficient_detail_in_critical_turn:<f>`；回复理由 ≥30 unicode ≥12 汉字（:1056-1064）。low_routine：仅 knowledge_need_reason/self_critique 需 ≥6 chars（:1070-1081）；其它情形（medium 等）无长度要求。
  8. 构造 decision + `carry_through_fields`（:1237-1393）：Some 才覆盖；knowledge_route 被显式吞掉（:1247-1252，AgentDecision 尚无该字段）；commitment 空 text 不透传，非空且 last_commitment 空时回填（:1296-1312）；assets/namecard/sufficiency 等全量透传；9 协议字段不再覆盖（:1391-1392 防 trim 后值被原始空白覆盖）。
- `build_minimal_decision`（:1191-1233）：全部 unwrap_or_default，conversation_mode 空回落默认，phase 只认 tool_calling/其它归 final。

#### 评审结果类型（:1420-1599）
- **ReviewScores**（:1420-1445）：human_like / emotional_value / hallucination_score(**alias "factRisk"**) / knowledge_grounding_score(**alias "productAccuracy"**) / pressure_risk（B1 恢复，缺省 0 兼容 R11）/ boundary_privacy_safety（缺省 0 最保守）。全部 number_i32 宽容反序列化。
- **DecisionReviewResult**（:1447-1501）：approved / scores / formula_breakdown / claim_analysis / risks(string_or_vec) / rewrite_instruction / review_summary + W2 扩展：needs_revision / revision_direction / should_hold / hold_reason / hold_category / self_critique_addressed / revision_applied / final_review_status（全 default 向后兼容）。
- hold_category 闭集（:1507-1516）：held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context；禁用集（:1520-1526）held_for_human/human_required/waiting_for_human/handoff_to_human/manual_takeover。
- `assert_hold_category_valid`（:1566-1590）：!should_hold && trim 空 → Unchanged（标准化空串）；trim 后 ∈ 闭集 → Unchanged（标准化去空白）；其它（禁用值/未知/should_hold 空）→ 强制 held_by_ai_policy + Coerced{original}（调用方写 `autonomy_hold_category_invalid` 事件）。

#### 其余（:1601-1946）
- KnowledgeRouteResult（:1601-1653）：含 selected_chunk_rankings（S4 propensity 采集）与 `selected_chunks_are_fallback`（:1637-1652——回填候选**不可**进 used_knowledge_ids 授权链，服务端专属赋值 LLM 无法伪造）。
- RunPlannerResult（:1686-1705）。宽容反序列化器：string_or_vec（数组或逗号/中文逗号/分号/换行切分 :1713-1736）/ number_i32 / optional_i32 / document_vec（字符串转 `{tool:"knowledge.search",reason}` 兜底 :1754-1775）/ value_to_i32（数值/字符串数字 round+clamp :1777-1793）。
- doc_i64/doc_i32/doc_bool/doc_string/string_array/optional_string/non_empty_option（:1860-1928）。
- `parse_follow_up_run_at`（:1936-1938）：仅 RFC3339，非法/空 → None（**拒绝而非立即执行**，防未来提醒变即时重复）。

### 2.7 `src/agent/runtime.rs`（1308 行）

#### 写侧 schema（:24-274）
- `USER_RUNTIME_PARAMETER_RULES`（:24-165）33 键规则表（Integer{min,max} 或 Boolean），精确范围：recentMessageLimit 1..200；minReplyIntervalSeconds 0..86400；maxDailyTouches 0..100；maxPendingFollowUps 0..100；followUpExpiresHours 1..8760；cooldownAfterNoReplyHours 0..8760；hallucinationBlockAt / pressureRiskBlockAt / knowledgeGroundingBlockBelow / humanLikeRewriteBelow / emotionalValueRewriteBelow / operationStateConfidenceFullReviewBelow 各 1..10；runTokenBudget / runTokenBudgetEscalated / simulationTokenBudget 1000..2000000；runMaxLlmCalls 1..20；reactionTokenBudget 1000..500000；reactionMaxLlmCalls 1..10；autonomyProtocolEnabled Bool；knowledgeMaxToolCalls 1..16；knowledgeOpenSliceMaxK 1..16；knowledgeSearchTopK 1..32；outboxPollIntervalSeconds 1..60；outboxLeaseSeconds 10..600；quietHoursEnabled Bool；quietHoursStart/End 0..23；quietHoursTzOffsetHours -12..14；consolidationWindowCharBudget 1000..16000；consolidationWindowMaxMessages 10..200；bayesianSlotMinHits 1..20；bayesianSlotMinStrong 0..20。
- `validate_and_normalize_user_runtime_parameters`（:200-261）：legacy 别名 factRiskBlockAt→hallucinationBlockAt、productAccuracyBlockBelow→knowledgeGroundingBlockBelow（与 canonical 同现且值不同 → Err 冲突 :211-216）；未知键 Err；类型/范围违规 Err；escalated < run budget Err（:247-259）。
- `validate_guide_runtime_parameter_patch`（:265-274）：Guide 白名单 10 键（GUIDE_RUNTIME_PARAMETER_KEYS :167-178，仅节奏/上下文/静默时段），越权 Err。

#### UserRuntimeParameters（:276-370）与解析
- 全 36 字段（阈值/预算/知识工具/outbox/静默时段/allowed_conversation_modes/grounding_gate_bypass_without_claim/distrust_self_reported_low_risk/归并窗/贝叶斯门）。
- `from_config`（:385-457）：走 `config.runtime_parameters_typed()` typed 单源；recent_message_limit/min_reply_interval **仅当 Document 显式含键**才用 typed，否则回落 AppConfig env（:394-409）；clamp：knowledge_max_tool_calls(1,16,默认6)/open_slice_max_k(1,16,4)/search_top_k(1,32,8)/outbox_poll(1,60,5)/outbox_lease(10,600,60)（`clamp_i32` :546-551——低于 min 回落 default 再 min(max)）；quiet start/end min(23)；tz clamp(-12,14)；conversation_modes= 内置四模式（gateway 加载 profile 后覆盖）；bypass/distrust=false；归并窗 clamp(1000..16000 / 10..200)；贝叶斯 clamp(1..20 / 0..20)。
- `as_document`（:459-492）：31 键 camelCase 回写（**不含** allowed_conversation_modes / 归并窗 / 贝叶斯 4 键——注入 reviewer 的硬参数 doc 也就没有这些）。
- `apply_profile_threshold_overrides`（:498-519）：M2，Some 字段 clamp(1,10) 覆盖五闸（G13 防极值禁用硬闸），None 逐字段保留。
- `apply_active_profile`（:535-539）：派生 bypass + distrust + threshold_overrides 三项（conversation_modes 刻意不并入，红线注释 :533-534）。
- `Default`（:553-609）：与 RuntimeParametersTyped::default 同源。**Typed 默认值**（models.rs:4877-4978 已核证）：recent 12 / interval 20 / touches 3 / pending 3 / expires 48 / cooldown 24 / **hallucination_block_at 6 / pressure 7 / grounding 7 / human_like 6 / emotional 6 / confidence_full_review 4** / budget 150000 / escalated 500000 / max_calls 6 / sim 300000 / reaction 8000+2 / autonomy true / 工具 6/4/8 / outbox 5/60 / quiet true 22→8 tz+8 / 归并 6000/60 / 贝叶斯 3/2。

#### ResolvedThresholds 覆盖链（:640-844）
- 6 gate_key（:654-661）：fact_risk_block / pressure_risk_block / human_like_score_rewrite / emotional_value_rewrite / product_accuracy_score_block / planner_block_rate_threshold。
- `baseline`（:666-675）从 runtime 5 闸 + AppConfig planner 率。
- `apply_override`（:680-703）：review 5 闸只收 1..=10 **整数**（fract==0，:789-791），planner 率只收 [0.05,0.95] 有限小数；非法 → Err BadRequest（历史脏值 fail-closed）；未知 gate_key 静默忽略。
- `apply_to_runtime`（:709-715）：5 闸写回 runtime（planner 率不写回）。
- **resolve_thresholds**（:731-773）读取顺序：① `load_user_operation_domain_config_for_resolve`（:797-844，与 decision.rs SR-008 同形唯一 current 校验）构造 runtime baseline；② `threshold_overrides` 集合 filter{workspace, account, current_version:true, rolled_back_at:null} 按 released_at 降序，**每 gate_key 取第一条（seen 去重）** apply（:746-771）。单 run 取一次不重读。

### 2.8 `src/agent/taxonomy.rs`（1474 行）
- `TaxonomyMatch`（:46-56）：Active / AliasActive(canonical) / Deprecated / CandidateNew。
- TTL=30s（:39）。`CachedEntry`（:81-98）：canonical_id/aliases/status/priority_weight/is_terminal/is_reactivation_target/display_name。
- `build_workspace_entries`（:100-156）：**current 指针不变量**——同 (scope,kind,value_id) 的 current_version 计数必须恰 1，否则 Conflict（:118-123）；active 条目的 identity claims（id+aliases）跨条目歧义 → Conflict（:125-141）。
- 缓存刷新：`warm_up/reload_all_from_db`（:214-261）全库审计 + 每 workspace 读 config_generation；`find_or_load`（:318-337，生产读路径）：先 `ensure_workspace_taxonomies` 幂等 seed（:786-800，进程本地 guard + builtin seed，seed 过则失效本 workspace shard）→ 读 generation → seeded || workspace_is_stale(TTL) || generation 变化 → `reload_workspace_from_db`；`find_or_load_read_only`（:340-362，shadow 不物化 builtin）；`invalidate/invalidate_workspace`（:195-211）。
- **check_value**（:393-441）命中优先级：scope 迭代 [account, "global"]（account 私有优先，命中即 return）——每 scope 内：① canonical==raw && active → Active；② canonical==raw && deprecated → Deprecated；③ alias 含 raw && active → AliasActive(canonical)；④ alias 含 raw && deprecated → Deprecated；两 scope 均 miss → CandidateNew。
- `dimension_value_weights`（:453-482）：所有条目（active+deprecated）的 (canonical, weight, is_terminal, is_reactivation_target)，account 先插入者赢。
- `dimension_values_with_labels`（:487-511）：仅 active 的 (id, display_name)。
- `kind_has_entries`（:524-537）：任一 scope 存在 **status=="active"** 条目才 true——纯 deprecated 残留视同未配置（F-009 fail-soft）。
- `upsert_candidate`（:551-658）：existing rejected → 仅刷 last_seen_at（不加 occurrences）；approved → warn + 刷时间（理论不该发生）；pending/其它 → $inc occurrences + 刷时间；不存在 → insert pending(occurrences=1, confidence clamp 0..10)；E11000 竞态 → 忽略。
- `upsert_candidate_once_per_run`（:664-760）：projection worker 变体——occurrences 以 run 为幂等单位（先 insert occurrences=0，经 `projection_observations::record_and_count` ledger 计数 + reconcile_stages 聚合）；approved/rejected 只刷时间。
- 进程级 registry（:770-831）：按 `db.cache_identity()` 分片（同连接 clone 共享、独立测试库隔离），Weak 生命周期自动清理；`init_global_taxonomy_cache` 启动预热；`invalidate_global_taxonomy_cache` 提交后失效。
- `inspect_taxonomy_value`（:836-853）诊断只读入口；`taxonomy_cache_for_tests`（:861-889）测试灌注 helper。

### 2.9 `src/agent/decision_taxonomy.rs`（358 行）
- `classify_decision_tags`（:44-75）纯函数四分支：对 dimension_kinds 每维经 `domain_signals::get_dimension` 取值（空跳过）→ check_value：Active 不动；AliasActive → `set_dimension` 写回 canonical（**reviewer 看到 canonical**，:5-12 注释）；Deprecated → risk `taxonomy_deprecated_value:{kind}:{raw}`；CandidateNew → risk `taxonomy_candidate:{kind}:{raw}` + 收集候选。**不触发 review fail、不写盘**。
- `validate_and_normalize_decision`（:83-106）生产入口：shadow snapshot 的 cache 或 global cache（shadow 不 find_or_load）；只返回 risks——**候选 occurrence 唯一写入责任点在 Gateway 终稿**（防 Decision/Gateway 双计 :77-82）。

### 2.10 `src/agent/domain.rs`（107 行）
`USER_OPS_DOMAIN_ID = "user_operations"`（:27）；`OpsDomain` trait（id + state_machine_domain_key 默认=id，:44-54）；`UserOpsDomain` 第一实现（:68-74）。纯边界声明，不做 trait 分发（单实现期，:13-16 注释）；现存 22 处字面量按 R11 保留（:18-19）。

### 2.11 `src/agent/domain_profile.rs`（2756 行）

#### DEFAULT 常量族
- `DEFAULT_PROFILE_ID = "__default__"`（:42）。
- `default_chunk_roles`（:49-76）：product_fact(fallback)/style_template/peer_case/negative_example 四桶 + header verbatim（复刻 knowledge_router）。
- `default_memory_dimensions`（:86-112）：八槽 (key,name,cap,candidate_type)=preferences(8,T)/doNotDo(10,T)/commitments(8,T)/objections(8,T)/openLoops(8,T)/openQuestions(8,F)/confirmedFacts(12,F)/conflicts(6,F)，全部 date_dimension=false。
- `default_trajectory_dimensions`（:116-121）：单维 objection_type「异议类型」。
- `default_outcome_polarity`（:160-174）：直接引用 gap_signals 的 DEFAULT_POSITIVE_OUTCOMES（=[user_replied_buying_signal]）/ DEFAULT_NEGATIVE_OUTCOMES（5 词，测试 :1634-1674 锁）。
- `default_business_formulas`（:191-221）：trust / conversionReadiness / emotionalValue / nextBestActionScore 四公式（expression 逐字对齐 policy 散文，eval_score_key 分别 humanLike/conversionReadiness/emotionalValue/relationshipProgress）。

#### prompt 注入函数族（apply_* 约定 :579-612）
统一约定：None/空 → 原样返回（DEFAULT 字节等价）；Some 非锚 → 精确子串替换；锚失配 → 原样不强插；幂等。
- 公式段：`POLICY_FORMULA_SECTION_HEADING`（:255）/ `strip_legacy_formula_self_check_section`（:265-283，剥到下一 `\n## ` 前，幂等）/ `build_policy_formula_section`（:288-293）/ `render_business_formulas_self_check`（:238-251，空集回落 DEFAULT，PascalCase key）。
- `strip_projection_only_policy_section`（:301-306）：从「## 标签与画像」起截尾。
- 对话模式判定段：`strip_conversation_mode_section`（:318-335）/ `apply_conversation_mode_policy`（:342-354，override 无标题时补锚，注入到剥离后文本最前）。
- `apply_conversation_mode_enum_list`（:393-412）：数组形 `["a", "b"]` 与竖线形 `a | b` 两种精确子串替换（旧串由 default_conversation_modes 构造；modes 空回落默认 → 不替换）。
- `render_business_formulas_json_example`（:417-435，reviewer formulaBreakdown 行）/ `render_reviewer_extra_score_lines`（:464-494，eval_score_key 排除 5 硬闸 [humanLike,emotionalValue,productAccuracy,pressureRisk,factRisk] 去重后每行 `"key": 6,`；**已知豁免**：DEFAULT 两行顺序与改造前 prompt 相反，语义等价非逐字节，:454-463 D2-1 批准注释）。
- reviewer 取向：`REVIEWER_REVIEW_FOCUS_LABEL`+`DEFAULT_REVIEWER_REVIEW_FOCUS`（:498-500）/ `apply_reviewer_review_focus`（:514-524，只换冒号后取向）/ `DEFAULT_REVIEWER_BALANCE_PRINCIPLE`（:504）/ `apply_reviewer_balance_principle`（:531-539，整条含标签替换）/ `apply_reviewer_fewshot`（:550-558，替换 prompts::DEFAULT_REVIEWER_FEWSHOT 整段）/ `apply_mode_gate_policy`（:569-577，替换 prompts::DEFAULT_MODE_GATE_POLICY 整段；boundary_protection 红线续行不在锚内恒守护）。
- **两条收敛链**：`apply_reply_policy_prompt_overrides`（:624-638，顺序固定：①公式段剥+注 ②conversation_mode_policy ③mode_gate_policy ④enum_list）；`apply_review_system_prompt_overrides`（:650-660，①review_focus ②fewshot；balance_principle 属 user prompt 另一注入点）。
- answeringMode：三档 rule/label 常量（:665-675）+ `render_answering_mode_rules`（:683-705，逐档回落）+ `answering_mode_labels`（:710-733）。

#### default_domain_profile（:742-864）
全字段 seed：两 ProfileDimension（customer_stage/intent_level 均 participates_in_decision=true）；prompt_fragment/soul_override/methodology_override/conversation_mode_policy=None；commitment_markers 复刻 guards const；五维 coverage（capability/pricing/caseEvidence/effectClaims/deliveryBoundary，含 anchor_hint 与 initial_signal——capability/deliveryBoundary="verified"、caseEvidence/effectClaims="evidence"、pricing=None，测试 :1776-1794）；stagnation_dimension=Some("customer_stage")；conversation_modes 四模式；operation_mode=OperationMode::default()（funnel/silence/commitment 开、calendar 关）；per_relationship=None；grounding_bypass=false；distrust=false；**transaction_facts_enabled=true**（G4 #5 销售域显式开）；chunk_roles/outcome_polarity/business_formulas/memory_dimensions/trajectory_dimensions=各 default；debounce/methodology_generator_preamble/threshold_overrides/reviewer_orientation/mode_gate_policy_override/answering_mode_profile/generated_state_machine=None；version=1 current published active。
- `example_emotional_companion_profile`（:879-940）：+intimate_companion 模式；bypass=true；distrust=true；transaction_facts=false；funnel 关 calendar 开；anniversaries date_dimension 记忆维；prompt_fragment 陪伴语义；per_relationship 三套（customer/peer/friend）。
- `example_sales_with_relationships_profile`（:952-977）：销售框架 + 三关系范式。

#### 加载与缓存（:979-1295）
- `load_active_domain_profile`（:987-994）→ `DomainProfileCache::get_or_load`。
- `validate_active_profile`（:1024-1044）：workspace/profile_id 非空未 trim 漂移、version>0、`validate_domain_profile_dimensions`（维度 key 安全），违规 Conflict。
- `DomainProfileCache`（:1012-1228）：TTL 30s（:1009）；`warm_up`（:1081-1120，全库 is_active:true，**同 workspace 多 active → Conflict**）；`reload_workspace`（:1122-1156，单 workspace find_one + generation）；`get_or_load`（:1175-1197，读 DOMAIN_PROFILE_NAMESPACE generation，stale 或 generation 变化才 reload）；`lookup_or_default`（:1199-1205，**无行回落 default_domain_profile(workspace)**）。
- registry（:1230-1274）：同 Database identity 共享；`invalidate_global_domain_profile_cache` 为 pub（集成测试 seed 后强制失效）。
- `resolve_debounce_window_ms`（:1277-1284）；`decision_dimension_kinds`（:1288-1295，participates_in_decision 过滤）。
- `render_decision_dimensions_guidance`（:1321-1386）：非 typed 的参与决策维度 → 指引 LLM 走 domainSignals 容器，逐维注入字典合法取值（`dimension_values_with_labels`）或「暂无受控取值」提示；DEFAULT 两 typed 维 → 空串。
- `render_memory_candidate_types_guidance`（:134-151）：维度 == DEFAULT 八槽或空 → 空串；否则列 fact + candidate_type=true 维 + conflict。

### 2.12 `src/agent/domain_signals.rs`（530 行）
- `known_typed_dims`（:27-29）→ registry 派生（=[customer_stage,intent_level]）。
- `normalize_domain_signals`（:42-63）：typed 非空 → 写容器（typed 权威，taxonomy 已 canonical 化）；typed 空且容器有 → 回填 typed。
- `get_dimension`/`set_dimension`（:89-109）：两 typed 维走字段，其它走容器；`remove_dimension`（:114-121）两侧同删（防 normalize 复活被隔离值）。
- `insert_domain_signal_values`（:138-169）画像写入内核：每个非空字符串维度写 `domain_attributes.<key>` dotted-key（trim；非字符串跳过）；`stagnation_changed && signals 含 stagnation_dimension（默认 customer_stage）` 才写 `{dim}_updated_at`（纵深守卫：维度没写就绝不刷计时 :155-167）；容器级 updated_at 由调用方管。
- `dimension_value_changed`（:173-175）：new.is_some() && prev != new（新值缺失不算变化）。
- `retain_declared_dimensions`（:192-201）：G1 写侧白名单——剔除 profile 未声明维度键；allowed 空 → 清空容器（保守）。

### 2.13 `src/agent/dimension_registry.rs`（537 行）
- 7 维 DIMENSION_REGISTRY（:61-104）：customer_stage(LlmSignals,typed,Taxonomy) / intent_level(同) / purchase_lifecycle(LlmSignals,Taxonomy) / churn_reason(LlmSignals,Taxonomy) / value_tier(GatewayDerived,CodeEnum) / relationship_type(AdminDirect,Taxonomy) / objection_type(ReactionDerived,Taxonomy)。
- `classify_validation`（:147-182）纯决策：空串 → DropSilently；CodeEnum/FreeText → Accept(trim)；Taxonomy：Known → Accept 原值；Alias → Accept(canonical)；**KindUnconfigured → Accept 原值**（字典未配置=未约束，admin/机器一致回退信任，:162-167——区别于 Miss）；Miss → AdminWrite 或 AdminDirect 通道 → Reject("… 不在字典内")，否则（机器写）DropSilently。
- `match_to_dict`（:187-194）：Active|Deprecated → Known（**Deprecated 是合法历史值不算越界**，红线）；AliasActive → Alias；CandidateNew → Miss。
- `lookup_dict`（:199-228）：check_value → Miss 时再用 `kind_has_entries` 细分 KindUnconfigured。
- `validate_dimension_value`（:231-257）：未知 kind → 直通 Accept；空串短路 Drop；Taxonomy 才查字典（失败 → Reject("taxonomy unavailable")）。
- `fold_stage_validations`（:262-272）：Accept 收集 / Reject 短路 Err / Drop 跳过；`normalize_target_stages`（:280-301）：运营 target_stages 逐项 AdminWrite 校验归一。

### 2.14 `src/agent/bayesian_slots.rs`（375 行）
- 常量：MAX_BAYESIAN_SLOTS=6（:15）/ HISTORY_CAP=100（:17）/ STRONG_POINT_MARKER="strong"（:20）。
- `SlotPromotionThreshold` 默认 min_hits=3, min_strong_evidence=2（:29-36）。
- `should_promote`（:48-54）：hits ≥ min_hits && strong ≥ min_strong（**单次提及 hit=1 永不占槽**）。
- `apply_bayesian_update`（:69-149）两遍算法：
  - 预处理 `normalize_observed_dimensions`（:155-180）：同 run 同维度重复相等值合并（取 max confidence + max strong bit）；**冲突值 → 整维作废不计 hit**；保首见序。
  - 第一遍：已有信号 → **同 run_id 已有 history 点则跳过（幂等重试）**（:86-91）；否则 push BayesianPoint{turn, source_run_id, value, confidence, value_changed, confidence_changed, reason=strong_evidence_count≥1 时 "strong"}，history 截断至 100（头删 :106-108）；新维度 → push 新 BayesianSignal(locked=false)。
  - 第二遍占槽（:129-148）：budget = 6 - 已 locked 数；未 locked 信号按序，hits=history.len()、strong=history 中 strong 标记点数（**代码侧证据，不信 LLM confidence**，测试 :261-311），should_promote → locked=true、budget-=1，budget 耗尽 break。

### 2.15 `src/agent/entitlements.rs`（1320 行）
- `ENTITLEMENTS_PROMPT_CAP=8`（:32）。`Entitlement`（:36-48）。
- `verification_drives_entitlement`（:51-53）：仅 staff_confirmed / payment_verified（conversation_inferred 红线排除）。
- `project_entitlements`（:64-174）：过滤可信度 + 有 product_ref → 按 product_id 聚合（正向累加/reversal 负减 quantity；owned_since/快照名跟最早**正向**笔；**每笔正向成交各贡献 (occurred, snapshot_days) 到期锚**，reversal 不贡献 :92-121）→ 净件数 ≤0 剔除（:125）→ owned_since 倒序 take(cap)（:130-131）→ 解引用活名（archived 回落快照名）；到期 = max(各锚 occurred + (snapshot_days ?? active_days)，days≤0/缺失丢锚)（:144-153，A 修复：续费独立续窗取最晚）；in_aftercare = now ≤ expires。返回 (list, cap 前 total)。
- `format_entitlements_hint`（:189-210）：`- 已购买「名」，共 N 件，售后/有效期内（至 日期）`；超 cap 加省略行。
- `load_active_products`（:224-242）：workspace+status:"active"，best-effort 空。
- `fmt_minor_as_major`（:248-252）：分→元两位小数（**19900→"199.00"，防 100 倍错价命门**）。`format_product_catalog_for_prompt`（:258-283）：名/id/价+币种/SKU/简述。
- `render_suspected_deal_guidance`（:297-308）：产品非空才注入——AI 永不自断成交，走 agentGeneratedSignals kind=suspected_deal 弱信号 + 主动求证话术两动作。`render_suspected_deal_reply_guidance`（:330-335）：fast-reply 精简版（只保留向客户求证义务，信号提取归投影 worker）。`render_relationship_type_suggestion_guidance`（:337-349）：常驻，仅新证据时产 relationship_type 信号。
- `render_transaction_facts_sections`（:358-377）：enabled=false → 三段全空（双重保险闸）；true → 目录/持有/疑似成交三段组合。
- G1 常量（:383-395）：purchase_lifecycle 的 not_purchased/purchased/aftercare/repurchase；value_tier 的 high/mid/low。
- `reconcile_g1_with_entitlements`（:422-446）：投影空 → None（无客观证据不纠偏）；客观值 = 任一 in_aftercare=Some(true) ? aftercare : purchased；LLM 空 → Some((客观值, ""))补锚；purchased/aftercare/repurchase → None；not_purchased → Some((客观值, "not_purchased"))纠偏；未知值 → None（交 taxonomy 候选）。
- `confirmed_deal_timestamps`（:461-470）：已核实正向成交时刻升序（reversal/inferred 排除）。
- `compute_customer_value_cents`（:482-493）：CNY（或未设币种）已核实 amount 求和，reversal 负减，clamp ≥0。
- `classify_value_tier`（:500-514）：min/max 归一防误配（mid>high 时 mid 档不被吞），≥hi→high、≥lo→mid、否则 low。

---

## 3. 跨文件机制：一次决策从 prompt 组装到 finalize 终态的完整判定树

以 gateway 主链路（webhook inbound → `run_user_operation_gateway`）为骨架（gateway.rs 行号为调用点证据）：

```
[0] runtime 构造
    UserRuntimeParameters::from_config(domain_config, state)        runtime.rs:385
    → resolve_thresholds 叠加 threshold_overrides（5 闸 + planner 率） runtime.rs:731
    → apply_active_profile(profile)（bypass/distrust/threshold M2）  runtime.rs:535
    → profile.conversation_modes 覆盖 allowed_conversation_modes    decision.rs:1338-1344

[1] Reply 决策  decide_reply_with_promote                            decision.rs:589
    prompt 组装（见 §2.1 第 23/27 步；system 五层 = Soul→Contract→Policy→Business→Operator）
    → generate_agent_json("user.reply.fast.task")                   decision.rs:1315
    → RawAgentDecision::validate_reply_critical（发送关键契约）      types.rs:799
        risks: missing_required_field / invalid_enum_value / decision_phase_invalid
    → decision_taxonomy::validate_and_normalize_decision            decision.rs:1356
        Active 通过 | AliasActive 改写 canonical（发生在 reviewer 前）
        | Deprecated → risk | CandidateNew → risk（不阻塞）
    → normalize_domain_signals（typed↔容器镜像）                     decision.rs:1368
    ⇒ (decision, promote_risks)

[2] 评审与 ClaimGate（并行）                                          gateway.rs:626/645, 3172
    should_run_review = decision.should_reply（不可自证跳过）        review/mod.rs:3277
    review_mode = effective_review_mode(planner, decision, runtime) review/mod.rs:3236
        full ⟸ force_full | distrust | planner.high | knowledge_required
             | operation_state_confidence < 阈值(默认4)
    ├─ review_decision（light/full prompt；双脑 tokio::join!）       review/mod.rs:4063
    │    parse_live_review 严格 wire（失败→blocked_by_safety_guard）  review/mod.rs:2993
    │    route_dual_gate（primary；second 亦各自 route 后比对分歧）    review/gates.rs:237
    │    双脑分歧 → needs_revision + reviewer_dual_disagree:* risk    review/gates.rs:371/416
    │    ※ 无 reviewer 场景 → local_decision_review fail-closed      review/mod.rs:3300
    └─ evaluate_independent_claim_gate（不改 review）                review/mod.rs:1452
         证据目录 = 当前消息/12 条历史客户入站/verified 知识/产品目录/principal relay
         （历史 AI 出站刻意排除）                                    review/mod.rs:481

[3] apply_independent_claim_gate（两者齐后合并）                     review/mod.rs:1505
    candidate 与最终正文不一致 → 直接 blocked_by_safety_guard
    LLM verdict → 先记原始非法 ref → harden（时间/服务承诺强化、假阳性纠正）
    → merge（unsupported 非产品 claim ⇒ approved=false + hallucination≥6
       + rewrite_instruction 定向证据修复）
    → catalog 完整性失败/manifest 失败 ⇒ should_hold(blocked_by_safety_guard)
    ⇒ priced_from_catalog（catalog_backed）

[4] targeted rewrite（可选，一次）                                    gateway.rs:3040
    should_run_targeted_rewrite = should_reply ∧ ¬should_hold
        ∧ ¬review_passed ∧ ¬needs_revision                          review/mod.rs:3266
    （证据修复优先于 style revision；rewrite 后须重跑 ClaimGate——[3] 的正文快照校验强制之）

[5] finalize_review_for_send（最终硬门，顺序不可换）                  review/gates.rs:589; gateway.rs:3194
    ①协议硬违规(missing/invalid_enum/invalid_type/phase) → BlockedByRequiredField
    ②insufficient_detail-only ∧ 软闸无硬失败 → 降级标 needs_revision（不返回）
    ③budget_exceeded_no_review → BlockedByBudget
    ④非产品证据发送时效复验（verified 过期）→ BlockedBySafetyGuard
    ⑤unsupportedNonProductBusinessClaimCount>0 → BlockedBySafetyGuard
    ⑥R5.4：requiresProductKnowledge ∧ verified=∅ ∧ ¬catalog ∧ ¬principal豁免
        → BlockedUnverifiedProductClaim
    ⑦grounding 漏判探针（仅观测事件）
    ⑧hold_category 矫正 + should_hold → Held(category)
    ⑨approved∧should_reply → Approved
      | needs_revision∧方向非空∧should_reply → approved 矫正 true → Approved
      | approved∧¬should_reply → Approved（A3 主动沉默→gateway 落 no_reply）
      | 其它 → Held(held_by_ai_policy)

[6] single-shot revision                                             gateway.rs:3313
    decide_revision(finalize_status, review, budget)                 review/gates.rs:1163
      Approved∧needs_revision∧¬hold ∧ 方向非空 ∧ 预算未超 → Proceed
      → Reply Agent 第二次（rewrite_instruction = revision_direction）
      → 二次 ClaimGate + 二次 finalize                                gateway.rs:3426/3435
      失败（空方向/预算/超时/LLM 错/二审仍败）→ apply_revision_fallback  review/gates.rs:1258
        纯 humanLike/emotionalValue 软闸或 style_diverged → 发原稿
          （revision_applied_approved）
        pressure/boundary/硬闸/双脑分歧/未知 → revision_failed
          + Held(held_by_ai_policy)

[7] 发送前 state/action 门（gateway 侧调 guards）
    check_state_transition（fail-soft：拒绝仅跳过 operation_state 写 + 审计事件，
        不拦已批回复）                                               guards.rs:187
    classify_reviewed_decision_action / enforce_state_action_policy   guards.rs:306/466
    → approved 决策进 agent_send_outbox（幂等键）→ MCP 发送
```

**Reviewer 可见/被遮罩字段总表**（B2 epistemic distance）：
- **进 reviewer**：候选回复正文（独立槽）；decision 事实面 13 键（gates.rs:64-81）；最新消息（剥/保哨兵）；时间授权视图 + 有界历史；memoryCard（完整或 light 7 键投影）；operating memory 的 relationshipState/productFit/nextAction；联系人特别指令（声明不可覆盖硬门）；playbook/domain_config/runtime.as_document()（full 档）；产品知识（按 chunk_roles）；净化后的知识路由。
- **被遮罩**：9 个自我推理字段 + intent_analysis/next_best_action/operating_memory_update（gates.rs:52-63）；operating memory 的 userUnderstanding（review/mod.rs:3766, 测试 :3795-3798）；知识路由的 reason/toolTrace/evidenceExcerpts/selectedChunkRankings（decision.rs:1640-1648；light 档例外——route 摘要含 evidenceExcerpts take(3)，review/mod.rs:3613）；reply_text 不在事实面重复。
- **ClaimGate 可见**：候选正文 + 触发消息 + domainRiskContext（profile 四字段）+ 服务端证据目录 + activeCatalog；**看不到** reviewer 评分、决策推理、memory。

**alias 改写时机**：decision.rs:1356（decide 返回前）→ reviewer/ClaimGate/finalize 全程见 canonical id；候选 occurrence 由 gateway 终稿唯一写入（decision_taxonomy.rs:77-82）。

---

## 4. 事实卡速查

### 4.1 ReviewScores 全字段与 alias（types.rs:1420-1445）
| 字段 | wire 主键 | alias | 缺省 | 语义 |
|---|---|---|---|---|
| human_like | humanLike | — | 0 | 越高越好 |
| emotional_value | emotionalValue | — | 0 | 越高越好 |
| hallucination_score | hallucinationScore | **factRisk** | 0 | 越高越危险 |
| knowledge_grounding_score | knowledgeGroundingScore | **productAccuracy** | 0 | 越高越好 |
| pressure_risk | pressureRisk | — | 0（老数据豁免） | 越高越危险 |
| boundary_privacy_safety | boundaryPrivacySafety | — | 0（老数据豁免） | 越高越安全，≤3 拦 |

live wire（parse_live_review）要求六键各恰一形、0..=10 整数；alias+canonical 同现 → invalid（review/mod.rs:3011-3036）。

### 4.2 final_review_status 产生条件矩阵
| 终态字面量 | 产生条件 | 出处 |
|---|---|---|
| approved | finalize ⑨a/b/c（含软闸矫正与主动沉默） | gates.rs:988/1008/1027 |
| blocked_by_required_field | promote_risks 含结构协议违规 | gates.rs:659 |
| blocked_by_budget | risks 含 budget_exceeded_no_review | gates.rs:715 |
| blocked_by_safety_guard | 证据过期 / unsupported 业务事实 / ClaimGate 不可用、正文不匹配、manifest/catalog 完整性失败 / reviewer schema 失败 / required_reviewer_not_executed / hold_category=blocked_by_safety_guard | gates.rs:779,818; mod.rs:1389,1405,1421,3099,3362 |
| blocked_unverified_product_claim | R5.4：产品声明 ∧ verified=∅ ∧ ¬目录 ∧ ¬豁免 | gates.rs:874 |
| held_by_ai_policy | approved=false 无硬门 / hold_category 该值 / revision 失败 | gates.rs:1037,978; 1272 |
| ai_waiting_for_more_context | reviewer 输出该 hold_category | gates.rs:978 |
| revision_applied_approved | revision 成功应用 或 安全 style-only 回退原稿 | gateway 改写; gates.rs:1268 |
| revision_failed | revision 失败且不可回退 | gates.rs:1271 |

### 4.3 全部阈值默认值（models.rs:4877-4978 + runtime.rs 核证）
| 参数 | 默认 | 判定式（出处） |
|---|---|---|
| fact_risk_block_at | 6 | hallucination ≥6 硬拦（gates.rs:126） |
| pressure_risk_block_at | 7 | pressure ≥7 软闸（gates.rs:172-173） |
| product_accuracy_block_below | 7 | grounding <7 硬拦（gates.rs:140-141） |
| human_like_rewrite_below | 6 | humanLike <6 软闸（gates.rs:154） |
| emotional_value_rewrite_below | 6 | emotional <6 软闸（gates.rs:186） |
| boundary_privacy（无参数） | 固定 ≤3 拦 / ≥4 放 / 0 豁免（gates.rs:200-201, :46） |
| operation_state_confidence_full_review_below | 4 | confidence <4 强制 full review（review/mod.rs:3251） |
| run_token_budget / escalated | 150000 / 500000 | escalated ≥ run 校验（runtime.rs:255） |
| run_max_llm_calls | 6 | |
| simulation / reaction budget | 300000 / 8000（calls 2） | |
| knowledge tool 三参 | 6 / 4 / 8（clamp 1..16/1..16/1..32） | |
| outbox poll/lease | 5s / 60s（clamp 1..60/10..600） | |
| quiet hours | 开，22→8，tz+8（clamp -12..14） | |
| recent_message_limit / min_reply_interval | 12 / 20s（Document 无键时回落 env） | runtime.rs:394-409 |
| max_daily_touches / pending / expires / cooldown | 3 / 3 / 48h / 24h | |
| consolidation 窗 | 6000 字 / 60 条（clamp） | |
| bayesian_slot_min_hits / strong | 3 / 2（clamp 1..20 / 0..20） | |
| planner_block_rate_threshold | AppConfig 提供；override 限 [0.05,0.95] | runtime.rs:686-691 |

R1.4/R1.5/R1.6 长度常量：回复理由常规 ≥10 chars ≥6 汉字；critical 轮 7 字段 ≥20 chars 禁 unchanged、回复理由 ≥30 chars ≥12 汉字；low_routine 仅 knowledge_need_reason/self_critique ≥6 chars（types.rs:1017-1082）。

### 4.4 AgentDecision 全字段表（types.rs:121-313）
发送控制：should_reply / reply_text / needs_review / autonomy_mode / decision_phase / tool_calls / conversation_mode(+reason) / sufficiency / missing_tier / clarification_intent。
发送指令：assets_to_send / namecard_to_send / escalation_request / commitment / last_commitment / follow_up / cooldown_until。
画像/记忆投影：profile_update / tags(+tag_evidence_turns) / customer_stage(+stage_evidence_turns/stage_explicit_intent) / intent_level / domain_signals / dimension_display_names / follow_up_policy / profile_attributes / intent_analysis / next_best_action / operating_memory_update / memory_candidates / memory_write_score / consolidation_needed / memory_update / bayesian_observations / agent_generated_signals。
知识：knowledge_need / matched_knowledge_ids / used_knowledge_ids / safe_claims_used / quoted_product_ids（不授权）/ recommended_resource_ids / forbidden_claim_risk / product_fit_score / context_pack_version。
状态：operation_state(+reason/confidence) / risk_level / run_mode。
9 推理字段：user_understanding / relationship_read / operation_goal / knowledge_need_reason / memory_update_reason / self_critique / why_should_reply / why_skip_reply / risk_self_check。

### 4.5 闭集枚举
- conversation_mode：casual_relationship | value_exchange | consultative | boundary_protection（默认前者；profile 可整体替换集合，types.rs:720-725, 960-968）
- autonomy_mode：auto | assisted | blocked（types.rs:719）
- run_mode：fast_chat | memory_candidate | knowledge_grounded | high_risk（types.rs:713-718）
- knowledge_need：not_required | required | insufficient（types.rs:712；guards.rs:70-75 另容忍遗留 "knowledge_required"）
- risk_level：low | medium | high（types.rs:711）
- decision_phase：tool_calling | final（types.rs:708-709）
- hold_category：held_by_ai_policy | blocked_by_safety_guard | ai_waiting_for_more_context（types.rs:1512-1516）
- action：reply | acknowledgement | silent | follow_up | cooldown（guards.rs:260-266）
- tool 名：knowledge.list_catalog | knowledge.search | knowledge.open_slice（types.rs:726-730）
- principal authorizationMode：affirm_or_condition | deny_only | none（review/mod.rs:414-423）
- OutcomeEvent verification 进投影闭集：staff_confirmed | payment_verified（entitlements.rs:51-53）

### 4.6 taxonomy 命中分支枚举（taxonomy.rs:393-441 + 消费方）
| check_value 结果 | decision_taxonomy（机器写） | dimension_registry classify_validation |
|---|---|---|
| Active | 通过不动 | Known → Accept 原值 |
| AliasActive(c) | set_dimension(c) 改写 | Alias → Accept(canonical) |
| Deprecated | risk `taxonomy_deprecated_value:k:v`（不改写） | Known → Accept（合法历史值） |
| CandidateNew | risk `taxonomy_candidate:k:v` + 候选收集（Gateway 终稿 upsert） | kind 有 active 条目 → Miss（Admin/AdminDirect Reject；机器 DropSilently）；kind 全空/纯 deprecated → KindUnconfigured → Accept 回退信任 |

scope 回落顺序恒 [account_id, "global"]，account 命中即短路。缓存：TTL 30s + config_generation 比对 + 显式 invalidate 三重刷新（taxonomy.rs:318-337）。

---

## 5. 偏差与疑点

1. **文件头注释与实际入口不符（decision.rs:1-9）**：模块 doc 说"该模块负责构造 prompt 调用 LLM 生成 AgentDecision"并两处提到 `decide_reply`（:9, :217 "单纯 decide_reply 把 promote_risks 默默丢掉"），但本文件只存在 `decide_reply_with_promote`，全仓 `pub(crate) fn decide_reply\b` 无定义——该注释描述的旧薄壳已被移除，doc 未同步。属文档漂移，不影响行为。
2. **`render_transaction_facts_sections` 的第三段被丢弃（decision.rs:928-940）**：返回值第三位绑定为 `_suspected_deal_text`，实际注入 task 的是 `render_suspected_deal_reply_guidance`（decision.rs:1030，fast-reply 精简版）。完整版 `render_suspected_deal_guidance`（entitlements.rs:297）在 decision 主路径无消费点（投影 worker 侧是否使用不在本次范围）。三段渲染做了全量工作但一段弃用——疑似历史演进残留（fast/projection 契约拆分后未收缩函数签名），非 bug 但浪费一次渲染。
3. **双脑 second reviewer 的 schema 失败会拉闸整个 run（review/mod.rs:4409-4416）**：second LLM **调用失败**仅 warn 回退 primary（:4417-4422），但 second 调用成功而 **parse 失败** → 整体 `hold_for_review_schema_failure` 返回（发送被拦）。注释 :4390 说"第二 provider 调用失败仅 warn 不阻塞——双脑是增益机制，不应成为新故障源"，而 parse 失败路径与该设计意图存在张力（一个输出不规范的次级模型可以持续压制发送）。是否有意（"给了 verdict 但不可信 = 必须拦"）无法从代码内文档确证——存疑。
4. **`validate_reply_critical` 不校验 why_should_reply（types.rs:857-869）**：should_reply=true 时只查 reply_text 非空，不查 why_should_reply（validate_and_promote 的 R1.4 有查）。fast 契约刻意精简还是遗漏，代码未注明；配套 R1.5 insufficient_detail 在该路径也整段缺失——finalize 的 insufficient_detail 降级分支（gates.rs:688）在 fast 主链路上因此实际不可达（promote_risks 不会含该前缀）。
5. **`operation_state_exists` 的 fail-open（guards.rs:86-95）与 `check_state_transition` 的 fail-closed 并存**：#155(P2) 注释已解释（normalize 只管归一，闸在 check），逻辑自洽，但要求所有新调用方必须理解这层分工——`action_policy_state_key`（guards.rs:127-130）先 exists 再 check，依赖此约定成立。记为需守护的隐式契约而非缺陷。
6. **quoted_product_ids 双轨语义**（types.rs:211-217, 483-484）：字段仍在 wire/持久化里流转但被声明"不能作为目录背书授权证据"，R5.4 的 priced_from_catalog 完全由 ClaimGate 服务端核验产生。历史字段留存有混淆风险（新人可能误用），文档已标注但无编译期防护。
7. **`runtime.as_document()` 不含 4 组新字段（runtime.rs:459-492）**：allowed_conversation_modes / consolidation 窗 / bayesian 双阈不回写 wire doc——reviewer full 档注入的「硬运行参数」（review/mod.rs:4157, 4288）看不到这些值。多为无害（reviewer 不消费），但「硬运行参数」注入的完整性与结构体字段集已漂移。
8. **finalize ⑤ 开放世界门依赖 merge 写入的计数**（gates.rs:791-795）：`unsupportedNonProductBusinessClaimCount` 只有 ClaimGate 跑过（merge_independent_claim_verdict）才存在；ClaimGate 因 should_reply=false 未跑时计数缺失按 0 处理——与"无正文无风险"语义一致，安全。但若未来某路径跳过 ClaimGate 而保留正文，此门会静默失效（依赖 [3] 的 candidate 快照校验兜底）。记为结构性耦合点。
9. **light reviewer 的 evidenceExcerpts 例外**（review/mod.rs:3613）：decision/full-reviewer 侧统一剥离 evidenceExcerpts（防调试元数据回流），light 档 route_summary 却注入 take(3)——口径不一致，或为 light 档补偿上下文的有意设计，未见注释说明。
10. **guards.rs `decision_requires_knowledge` 容忍 "knowledge_required"**（guards.rs:70-75）：该值不在 KNOWLEDGE_NEED_VALUES 闭集（types.rs:712），promote 会对其打 invalid_enum_value 并清空——两处口径不同（planner 侧宽、协议侧严），推测为遗留输出的兼容垫，行为影响极小（枚举清空后 planner 读到空串按不需要知识处理）。

---

## 6. 覆盖自证

全部文件逐行读毕（Read 工具分段全文，无跳读）；行数与 `wc -l`（2026-08-13 实测）一致：

| 文件 | 行数 | 读取段 |
|---|---|---|
| src/agent/decision.rs | 2721 | 1-950 / 951-1870 / 1871-2721 |
| src/agent/review/mod.rs | 4536 | 1-950 / 951-1900 / 1901-2850 / 2851-3750 / 3751-4536 |
| src/agent/review/gates.rs | 3317 | 1-950 / 951-1900 / 1901-2850 / 2851-3317 |
| src/agent/review/style.rs | 315 | 1-315 |
| src/agent/guards.rs | 1214 | 1-700 / 701-1214 |
| src/agent/types.rs | 2946 | 1-1000 / 1001-2000 / 2001-2946 |
| src/agent/runtime.rs | 1308 | 1-700 / 701-1308 |
| src/agent/taxonomy.rs | 1474 | 1-800 / 801-1474 |
| src/agent/decision_taxonomy.rs | 358 | 1-358 |
| src/agent/domain.rs | 107 | 1-107 |
| src/agent/domain_profile.rs | 2756 | 1-950 / 951-1900 / 1901-2756 |
| src/agent/domain_signals.rs | 530 | 1-530 |
| src/agent/dimension_registry.rs | 537 | 1-537 |
| src/agent/bayesian_slots.rs | 375 | 1-375 |
| src/agent/entitlements.rs | 1320 | 1-700 / 701-1320 |
| **合计** | **23814** | |

辅助核证（非清单文件，仅定点查证）：`src/models.rs:4831-4978`（RuntimeParametersTyped 默认值）；`src/agent/gateway.rs` 仅 grep 调用点行号（:626/645/868/882/2999/3025/3032/3040/3172/3194/3313/3426/3435）用于 §3 判定树，未通读。
