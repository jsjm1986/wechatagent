# 记忆/反应/投影/运行支撑 深读记录（核证日期 2026-08-13）

> 覆盖 16 个文件、共 15597 行，全部逐行读完（分段全文，无跳读）。所有断言均附 `file:line`，行号以 2026-08-13 工作区状态为准（git status 显示这些文件带未提交修改，行号对应当前工作副本）。

## 1. 模块地图

| 层 | 文件 | 行数 | 一句话职责 |
| --- | --- | --- | --- |
| 长期记忆 | `src/agent/memory.rs` | 5056 | memoryCard 默认/种子/compact 全规则、consolidator LLM 固化、OCC + prepared-commit 恢复、候选记忆写入与触发策略、operator 偏好记忆 CRUD |
| 记忆辅助 | `src/agent/consolidation_window.rs` | 77 | 归并宽窗口按"字符预算+条数"双上限截取（纯函数） |
| 证据锚 | `src/agent/tag_evidence.rs` | 119 | LLM 序位 → msg_id 证据锚（fail-closed）；强/弱证据客观判定 |
| 用户反应 | `src/agent/reaction.rs` | 1798 | 入站反应分析：确定性 stop/buying 词表、claim 原子锁、LLM 分析、outcome、intent_trajectory 滑窗、reviewer 误判信号、负例 chunk 入队 |
| 发送后投影 | `src/agent/post_decision.rs` | 1435 | 投影快照 prepared→pending→processing→completed 生命周期、投影 LLM、contact lease 单飞、stale 降级 append-only |
| 投影台账 | `src/agent/projection_observations.rs` | 138 | (entity, run) 严格幂等观测台账 + 聚合对账 pipeline |
| 运行信封 | `src/agent/run_envelope.rs` | 2059 | AgentRunLog 生命周期状态机、gateway/finalReview 状态闭集断言、恢复 insert、panic hook |
| 运行审计 | `src/agent/run_audit.rs` | 538 | run 内 LLM 日志/观测事件缓冲 + 批量 flush（失败按稳定 _id 重放）、阶段计时、路径分类 |
| 预算 | `src/agent/budget.rs` | 607 | task-local RunBudget：token/LLM 调用/tool 调用三维计数、升档抬顶、shadow 快照 task-local |
| 档位自评 | `src/agent/sufficiency.rs` | 342 | 三档 prompt 升档判定、Full 强升理由、used_knowledge_ids 只在 Full 档记录 |
| 影子模拟 | `src/agent/simulation.rs` | 312 | 多轮 shadow 演练（run_mode="shadow"，只读加载，never 发送） |
| Prompt 影子 | `src/agent/prompt_shadow.rs` | 788 | 单源样本 A/B 真模型对照：冻结模板+快照+依赖指纹三重隔离 |
| 影子终结 | `src/agent/shadow_finalize.rs` | 196 | shadow 复用生产 claim gate/终评聚合/状态动作策略，零持久化 |
| 注入隔离 | `src/agent/prompt_isolation.rs` | 763 | 不可信文本定界/剥 tag/剥 relay 哨兵、历史预算截断、时效性档期事实视图 |
| 系统事故 | `src/agent/system_incident.rs` | 1202 | LLM 账户不可用事故：generation 因果收敛、outage/recovery 双相通知物化、发送前授权 |
| 多模态地基 | `src/agent/multimodal.rs` | 167 | 入站媒体下载打桩（恒 None）、vision 描述封装、非文本过渡话术 |

依赖走向（本组内）：`reaction`/`simulation`/`prompt_shadow` → `memory`（读 memoryCard）与 `budget`（RUN_BUDGET scope)；`post_decision` → `memory`（append-only 写入）+ `gateway::apply_agent_updates`；`memory`/`reaction` → `tag_evidence` + `prompt_isolation`；`system_incident` → `outbox::enqueue`；`run_envelope`/`run_audit`/`budget` 是全体 run 的横切支撑。

## 2. 逐文件深读

### 2.1 `src/agent/memory.rs`（5056 行）

#### 默认结构与读取投影

- `default_context_pack()`（memory.rs:41-57）：返回 13 键空 Document（confirmedFacts/preferences/painPoints/objections/commitments/doNotDo/relationshipTimeline/recentSignals/openQuestions/importantQuotes/stalenessWarnings/deprecatedFacts/conflicts）。
- `default_memory_card()`（memory.rs:63-98）：typed 三数组全空；`extra` 兜底历史 wire shape：coreProfile 四空串、relationshipState（stage/trustLevel/temperature="unknown"、lastEmotion=""）、六个空数组、`source="memory_card"`、`version=0`。
- `effective_memory_card(memory)`（memory.rs:105-114）：优先 `memory.memory_card` 非空 → compact(card, None, [])；否则 `context_pack` 非空 → `MemoryCardTyped::from_document` 后 compact（历史兼容）；都空 → default。**每次读取都过一遍 compact**（纯投影，无写库副作用）。
- `effective_memory_card_for_contact(memory, contact, initial_state)`（memory.rs:119-136）：`memory_card_has_signal` 为真用生效卡，否则改用 `memory_card_from_contact` 种子卡；最后把 `memory.memory_card_version` 注入 `extra.version`（memory.rs:134）。
- `memory_card_has_signal(card)`（memory.rs:145-186）：三类信号任一即真——typed 三数组非空（146-151）；`extra` 中 9 个数组键任一非空（152-170，含历史 coreFacts/recentFacts 镜像）；`extra.recentEpisodeSummary` 非空或 coreProfile 四文本字段任一非空（171-184）。
- `memory_card_from_contact(contact, memory, initial_state)`（memory.rs:193-299）：种子卡构造。identity 取 `human_profile_note` → `memory_summary` → `agent_profile.summary` 链（199-204）。core_facts 只放权威来源：`human_profile_note`（source="operator_manual"，221-224）、`manual_tags`（≤6 条截断，225-230）、`confirmed_tags`（source="confirmed_tag"，231-236）；**`memory_summary` 不进 core_facts**，归位 `extra.recentEpisodeSummary`（286-289，⑨真因修正，测试 memory.rs:4296-4320 锁定）。relationshipState.stage 回落链：`domain_attributes.customer_stage` → `operation_state` → 状态机初始态（260-266，H13）。
- 小工具：`push_seed_fact`（301-311，`MemoryFact::from_plain_text` + extra.source，按 text 去重）；`push_unique_text`（313-320）；`non_empty_text`（322-333，压空白）；`string_array_from_doc`（335-347）。

#### compact 全规则（HP-2 / Task 8 / SR-182 / H17）

- `compact_memory_card(card)`（349-351）与 `compact_memory_card_with_previous(card, previous, discarded)`（371-381）都是 `compact_memory_card_with_dimensions` 的包装，后者传 `default_memory_dimensions()`（379-380，DEFAULT 销售八维，与写死 cap 表逐字等价）。
- `compact_memory_card_with_dimensions(card, previous, discarded, dimensions)`（387-587），步骤顺序：
  1. **discarded 全局黑名单**：先剔 incoming `core_facts` 中命中 discarded 的（398-402）。
  2. **previous 合并救回**（404-433）：逐条旧 core fact——命中 discarded 跳过（407-409）；**⑨件一 dimension 感知救回**：旧 fact 带非空 dimension 且 incoming 已有同 dimension 的 Structured fact → 不救回（防改口双值，414-424；dimension=None 退回纯 text 去重）；text 不重复才 push（425-431）。
  3. **统一排序 + cap 6**：`sort_by(compare_core_fact_priority)`（438），`split_off(6)` 得容量淘汰项（440-444），每条淘汰项打 `evictionReason="core_fact_capacity"/evictedAt/evictedFrom/coreFactRank(7+offset)` 注记（445-447 → `annotate_core_fact_eviction` 657-665）。
  4. **coreFactEvictions 审计**（449-515）：本次淘汰审计 + 卡上已有 + previous 上的合并（456-463）；过滤规则——fact 回到当前 core（按 id 或 text）则删除旧淘汰记录（485-494）、显式 discard 强于容量淘汰（495-503）、按 `id:` / `text:` 键去重（504-510）；**cap 20**（511）。
  5. **recent 合并**：淘汰项排在原 recent 前（519），去掉与 core 重复及 recent 内部重复（520-528），**cap 10**（529）；`deprecated_facts` **cap 20**（531）。
  6. **extra.coreFacts 历史 String 数组路径**：previous 存在时与 incoming 合并（discarded 同样过滤两侧），写回 `extra.coreFacts`（538-573）。
  7. **extra cap**：镜像键写死 `coreFacts 6 / recentFacts 10 / deprecatedFacts 6`（580-582，注意 extra 镜像 deprecatedFacts cap=6 ≠ typed cap=20，见 §5 疑点 2）；业务八槽由 `dimensions` 驱动 `limit_extra_array(key, cap)`（583-585）。
- 排序键 `compare_core_fact_priority`（647-655）：source_authority ↓ → importance ↓ → confidence ↓ → updated_at ↓ → stable_key ↑ → text ↑。`core_fact_source_authority`（589-617）：`operator_manual`=100、`confirmed_tag`=20；其它基础 1 + 有 source_message_ids +8 + 有 evidence +4 + 有 source_run_id +2（LLM 事实最高 15，永远低于运营权威）。Plain fact：authority=0、importance=5、confidence=7、updated_at=0（619-638）。运营种子事实不会被 LLM 事实挤出 core（测试 memory.rs:4090-4119）。
- `limit_extra_array`（684-690）：仅当数组超长时 truncate。

#### 非原子 blob 防御（⑨件二）

- `fact_is_non_atomic(text)`（701-714）：三判据 OR——`\n`≥2；句界标点（。！？;；）≥2；chars>80。纯结构度量，零关键词零 LLM。
- `value_has_non_atomic_fact(value)`（719-737）：扫 consolidator 原始 JSON 的 `memoryCard/memory_card` 下 `coreFacts`/`recentFacts`（兼容对象 `{text}` 与老 String 数组），任一非原子即真。
- `should_drop_non_atomic(warnings)`（745-749）：warning 含 `non_atomic_fact_persists_after_retry` 或 `non_atomic_fact_retry_call_failed` → 落库前丢弃非原子条。

#### 同维冲突自动裁决与 deprecation 应用

- `deprecate_same_dimension_conflicts(card, now)`（758-814）：core_facts 里按非空 dimension 分组（764-771）；同组≥2 条时保留 `updated_at` 最新一条（平手取索引大者，778-784），其余移入 `deprecated_facts`（deprecated_at=now、reason="superseded by newer fact in same dimension"、extra.supersededBy=胜者 id，801-811），deprecated cap 20（812）；每条产 warning `same_dimension_conflict_deprecated:<dim>:idx<i>`（793）。dimension=None 完全不参与。机制侧兜底：A/B 已证 LLM 不可靠主动填 discarded（756-757 注释）。
- `memory_conflict_audit_events(previous, current, model_conflicts, run_id, prev_ver, next_ver)`（838-912）：两类事件——model_conflicts 原样带 `auditSource="model_conflict"`（863-870）；从 before/after deprecated_facts 差集推导 `auditSource="memory_card_diff"`（871-910，带 deprecatedFactId/dimension/deprecationReason/supersededBy），确定性裁决即使 LLM 不填 conflicts 也可观测。
- `apply_consolidator_deprecations(card, previous, consolidator_value)`（914-1038）：消费 LLM 输出 `deprecatedFacts:[{id,reason,deprecatedAt,supersededBy}]`。行为（对齐 R6.5/R7.2-R7.7）：id 空 → warning `deprecated_fact_id_not_found:<empty>` 跳过（950-953）；reason 截 200 字符（954-962）；deprecatedAt 非法 RFC3339 → 回退 now + warning `invalid_deprecated_at:<id>:<raw>`（963-973）；在 previous 的 core+recent 按 id 找原 fact，找不到 → warning + 不写（975-987）；supersededBy 不在新 active 集 → warning `superseded_by_id_not_found:<id>:<sup>` 但仍写入（991-996）；同 id 同时 active+deprecated → warning `fact_simultaneously_active_and_deprecated:<id>` + 从 active 两数组移除仅留 deprecated（997-1008）；合并旧 deprecated 后按 (deprecated_at asc, id) 排序，**cap 20 丢最旧**（1012-1035）。
- `compact_memory_card_typed`（1042-1051）：`#[deprecated]` 兼容别名。

#### OCC 与 operating memory 装载

- `next_memory_card_version(memory)`（1053-1055）：`saturating_add(1)`（i32::MAX 处饱和，测试 4616-4621：版本空间耗尽后 OCC 永不命中，可观测可治理）。
- `occ_memory_filter(ws, acct, wxid, prev_version)`（1062-1074）：四键 filter，`memory_card_version` 是乐观锁谓词；update_one 单文档原子 + (workspace_id, account_id, contact_wxid) 唯一索引 = 全局至多一个 winner（1057-1061 注释）。
- `load_or_create_operating_memory(state, contact)`（1079-1258）：
  - 先加载 domain_config 取状态机初始态（1083-1091）。
  - 已存在且生效卡无信号 → 用种子卡 OCC 升级（1105-1163）：compact → 注 version → `to_document` → `update_one(occ_filter, $set)`；`modified_count==1` 本地同步（1141-1144），否则**重读最新版**吃对方结果（1145-1162）。
  - 不存在 → 构造全新 OperatingMemory（1167-1220，next_action.currentState 回落初始态 1202）；种子卡有信号则 version=1 否则 0（1221-1230）；insert 撞 (ws,acct,wxid) 唯一索引 11000 → **不透传**（CONC-3：透传会让整轮 run 失败），落到重读分支返回赢家文档（1231-1257）。
- `load_operating_memory_read_only(state, contact)`（1263-1327）：shadow 安全版——存在原样返回；缺失只在内存合成（种子卡 + version 0/1），**永不 insert、永不 OCC 种子升级**。

#### consolidator LLM 全流程

- 入口：`handle_memory_consolidation_task`（1329-1331）→ `_with_claim`（1333-1355，按 task 三键找 contact，找不到 NotFound）→ `consolidate_contact_memory[_with_claim]`（1577-1620）：从 domain_config runtime 取 `run_token_budget / run_max_llm_calls / knowledge_max_tool_calls` 建 RunBudget（波 C3，1591-1606），`RUN_BUDGET.scope` 包住 inner。
- `consolidate_contact_memory_inner`（1622-2201）分步：
  1. **拉候选**：`memory_candidates` 中 status="pending"，created_at 升序，**limit 30**（1631-1654）。
  2. **空候选**：有 claim → `finish_running_memory_task_window(state, claim, "no_candidates")`（1655-1658）；无 claim 有 task_id → 直接置 task sent/gateway_status="no_candidates" 并 unset claim 字段（1660-1682）。
  3. **prompt 组装**（1685-1800）：加载 active profile（复用于维度指引+合并 cap，1688-1690）；system/task prompt 从 `user.memory_consolidator.system/task` 加载，带内置 fallback（1691-1708）；`render_memory_dimensions_guidance`（1367-1396）只在维度偏离 DEFAULT 八维时追加（DEFAULT 返回空串保 prompt 字节等价；date_dimension 槽要求结构化对象 {label,date,recurring}，1388-1393）；宽窗口 = `load_recent_messages`（倒序）反转升序 → `take_window_by_budget`（默认 6000 字/60 条，1719-1731）；`render_window_numbered`（1402-1419）产 `[i] 客户/你: content` 且客户原文过 `history_prompt_content` 注入隔离（1414）；注入当前 confirmed_tags、source="tag_observation" 的候选、**升级后的当前卡**（`injected_card.auto_upgrade_plain_facts()` 把历史 Plain 一次性升 Structured 带稳定 id，1745-1746，⑨治上游：与 prev-merge 同一实例保证 LLM 引用 id 命中）、已有维度名清单（1749-1757）。
  4. **LLM + 非原子重试**（1801-1853）：`generate_agent_json(prompt_key="user.memory_consolidator.task")`；输出含非原子 blob → **至多重试一次**全新调用；重试干净 → `non_atomic_fact_resolved_by_retry`；仍脏 → 用重试结果 + `non_atomic_fact_persists_after_retry`；重试调用失败 → **不阻断固化**（既成事实纪律）+ `non_atomic_fact_retry_call_failed`（首次 value 已确认非原子，走落库前丢弃）。
  5. **解析与合并**（1854-1930）：card 取 `memoryCard`/`memory_card`/整体（1858-1863）→ `from_document` → `auto_upgrade_plain_facts`（计数进 warning `memory_facts_auto_upgraded:<n>`，1873-1874/1926-1930）；`should_drop_non_atomic` 命中 → core/recent retain 掉非原子条 + `non_atomic_facts_dropped:<n>`（1879-1893）；`discarded` 字符串数组（1896-1904）；prev-merge 用注入同源的 `injected_card`（1906）；`compact_memory_card_with_dimensions(card, prev, discarded, profile.memory_dimensions)`（1910-1915）；`apply_consolidator_deprecations` → `deprecate_same_dimension_conflicts`，warnings 累加（1918-1925）。
  6. **审计纯数据化**（1931-1961）：model conflicts 只取 winner 非空/非"none" 的（1934-1951）；`memory_conflict_audit_events` 绑定 run_id + 前后 version（1954-1961）；`extra.version=next / source="memory_consolidator_agent"`（1962-1963）。
  7. **标签重判 + 人格**（1965-2019）：`parse_reconfirmed_tags`（1427-1466：每条 {value, evidenceTurns}，经 `resolve_evidence` 映射 msg_id 锚，**证据空 fail-closed 丢弃**，confirmed_by="consolidation"）；`parse_discarded_tags`（1470-1487：显式弃用不需证据锚）；`merge_confirmed_tags`（1493-1508：reconfirmed 权威 + 旧标签未显式弃用即保留，A-02 对称 core_facts 的"未显式弃用即保留"；manual_tags 不受影响）；大五 OCEAN：`parse_personality`（1544-1559）五维 `parse_facet`（1519-1542，**证据空 → confidence 强制 0**，诚实置信铁律；只写 `Contact.personality_profile` 旁路，永不驱动决策）；快照 `append_snapshot_capped` FIFO **cap 50**（`MAX_PERSONALITY_SNAPSHOTS`，1562-1575，写回时基于旧 profile append，1981-2019）。
  8. **prepared commit（有 claim 生产路径）**（2021-2051）：把全部产物（prev/next version、memory_card doc、confirmed_tags、personality、candidate_ids、run_id、warnings、conflicts、summary、discarded、candidate_count）打成 `prepared` Document → `prepare_task_commit_if_owned(state, claim, "memory_consolidation", prepared)`（running→committing CAS；失权返回 false 即静默退出）→ `reconcile_memory_consolidation_commit`。
  9. **无 claim 兼容路径**（2053-2200）：warnings 直写 `agent_run_logs.memory_consolidator_warnings`（2054-2065）；conflict 事件逐条 `write_event_for_account`（2066-2078）；OCC `update_one(occ_filter(prev_version), $set memory_card/version/updated)`（2084-2104）；**输 OCC** → 候选留 pending、task 置 retry/gateway_status="memory_card_occ_conflict"（2105-2128，避免"候选被吞但卡未更新"撕裂）；赢 → contact 侧投影 `$set confirmed_tags[+personality_profile]`（fail-soft，2130-2149）→ 候选 `update_many status="consolidated"`（2150-2160）→ `memory_consolidated` 事件（2161-2177）→ task sent/gateway_status="consolidated" + unset active_task_key/rerun_requested（2178-2199）。

#### prepared commit 恢复协议（P1-5 / SR029）

- filter 构造器：`memory_commit_apply_filter`（2203-2215，= occ filter）；`memory_commit_applied_filter`（2217-2234，`memory_applied_commits` $elemMatch {task_id, claim_generation}）；`memory_commit_marker`（2236-2241）；`memory_task_rerun_filter`（2243-2250，rerun_requested=true / $ne:true 两窗）。
- `finish_running_memory_task_window`（2252-2306）：owned running filter + 无 rerun → sent（unset claimed_at/claim_token/active_task_key/rerun_requested）；miss 则试 rerun 窗 → retry/gateway_status="memory_candidates_arrived"/next_retry_at=now。
- `finish_committing_memory_task_window`（2308-2363）：committing filter 同样双窗（sent+"consolidated" 清 prepared_commit*，或 retry 重跑），返回是否完成转换。
- `memory_contact_projection_filter`（2365-2388）：contact 投影单调性——`memory_projection_version` 不存在 / < next / （== next 且同 task 且 claim_generation ≤ 本次）三支 $or，防旧 generation 回写、允许同 generation 重放。
- `reconcile_memory_consolidation_commit(state, task_id)`（2400-2648，`#[doc(hidden)] pub` 供集成红线重放崩溃窗口）：
  1. 只认 `status="committing" && prepared_commit_kind="memory_consolidation"` 的 task（2404-2419）；从行上取 claim_token/claim_generation 重建 TaskClaim（2420-2434）；prepared 字段缺失 → AppError::External（2435-2456）。
  2. **记忆主写**：先查 applied marker（2459-2464）；未应用 → OCC `update_one(apply_filter(prev_version), $set 卡+version+来源 task 字段, $addToSet memory_applied_commits marker)`（2465-2484）；OCC miss 且 marker 仍不在 → `requeue_task_commit_if_owned(claim, "memory_card_occ_conflict")` 后返回（2487-2495，让位并发新版本）。
  3. **contact 投影**：confirmed_tags + memory_projection_version/source_task/claim_generation（personality 非 Null 才带），经单调 filter 写入（2498-2524）。
  4. **候选固化**：`{_id ∈ candidate_ids, status:"pending"}` → consolidated + consolidated_by_task_id/claim_generation（2526-2550，只从 pending 转移=幂等）。
  5. **warnings/事件**：run log warnings + memory_commit_task_id（2552-2571）；conflict 事件与完成事件用 dedupe_key `memory_commit:<task>:<gen>:conflict:<i>` / `:complete`，duplicate key 吞掉（2573-2627）。
  6. **收尾**：`finish_committing_memory_task_window` 成功转换后 `$pull` applied marker（2629-2646，marker 只在 committing 窗口存活）。

#### 候选写入与触发策略

- `upsert_projected_memory_candidate`（2650-2676）：按 `(ws, acct, wxid, projection_key)` upsert `$setOnInsert`——同 run 重放不重复插入。
- `write_memory_candidates(state, contact, decision, run_id)`（2678-2728）：decision.memory_candidates 空则回落单条 operating_memory_update（2687-2691）；逐条 `validated_memory_candidate`（2930-2949：必须有 type/content/evidence；importance/confidence clamp 0-10 且都非 0；**A-01 弱证据 cap**：evidence trim 后 <4 字符 → importance 封顶 7 `IMPORTANCE_CAP_WHEN_WEAK`，堵"空证据+自报高分"进救援通道，2914-2928）；status 由 `decide_candidate_status(write_score, max_importance)`（2833-2841）判：`write_score>=6 || max_importance>=8` → "pending" 否则 "ignored_low_score"（#73 高重要度救援）；projection_key=`{run_id}:decision_memory`（2725）。
- `build_tag_observation_docs(dimension, tags, evidences)`（2733-2752）：每值一条 {dimension, value, hitCount:1, evidences}，多值共享本轮证据。
- `write_tag_observations`（2760-2792）：decision.tags 经 `resolve_evidence(window, tag_evidence_turns)`（窗口须升序）；**证据空整批丢弃**（fail-closed，2772-2775）；写 source="tag_observation"、status="pending"、projection_key=`{run_id}:tag_observation`。不写 confirmed_tags（那是压缩重判产物）。
- `write_stage_observation`（2799-2830）：弱证据 customer_stage 落暂定层（dimension="customer_stage"），projection_key=`{run_id}:stage_observation:{stage}`；证据空跳过。
- **consolidation 触发条件全集** `memory_consolidation_due`（2848-2864），任一为真即触发：
  1. `decision.consolidation_needed == true`（LLM 显式请求）；
  2. `decision.memory_write_score >= 6`；
  3. pending 候选数 `>= 4`（`CONSOLIDATION_PENDING_COUNT_THRESHOLD`，2843）；
  4. 最老 pending 年龄 `>= 6h`（`CONSOLIDATION_MAX_PENDING_AGE_MS = 6*60*60*1000`，2844）。
  `contact_memory_consolidation_due`（2868-2912）：前两条命中直接 true 免查库；否则用 scoped 索引最多查 4 条 pending 取 count + 最老时间再判（防低分候选饿死且不做无界 count）。
- `schedule_memory_consolidation_task`（2951-3127）single-flight（ACTIVE_KEY="memory_consolidation"）四段瀑布：① 复活 failed 行（→retry，attempt_count/claim_recovery_count 归零，2963-2992）；② 唤醒既有 active 行（$set rerun_requested=true，2994-3013）；③ 收养 pre-single-flight 遗留行（active_task_key 不存在且 status ∈ 5 态 → 打 key + rerun；duplicate key = 并发赢家已装 key → 给赢家标 rerun，3015-3066）；④ 新建 task（kind="memory_consolidation"，max_attempts=3，带 active_task_key；duplicate key → 给赢家标 rerun，3068-3126）。
- `run_manual_memory_consolidation`（3134-3211）：管理员手动路径**刻意不唤醒既有 task**（把别人的进行中工作当同步结果返回是造假，3129-3133 注释）——直接 insert 带 ACTIVE_KEY 的新行，duplicate key → `Conflict("memory_consolidation_already_active")`；`run_due_task_by_id` 同步驱动，claim 不到 → Conflict("not_claimable")；按最终 status 映射：sent→Ok(task_id)、committing→External("durably pending recovery")、retry/failed→External(error)、其它→unexpected（3196-3210）。

#### operator 偏好记忆（knowledge-digest Phase 5 / A2）

与 memoryCard 物理隔离——只触达 `knowledge_operator_memory` 集合，两者禁止互读写（3215-3218 注释）。

- `load_operator_memory`（3225-3233，touch=true）/ `load_operator_memory_read_only`（3237-3245，touch=false，shadow 不续期）→ `load_operator_memory_with_touch`（3247-3316）：now 优先取 shadow 快照的 `evaluated_at`（3259-3261，保证 A/B 两支同一时刻判过期）；filter = 未撤销 && （无 expires_at 或 > now）；按 `last_used_at desc` limit top_n；touch 时批量 bump `last_used_at = now`（仅未撤销行，3296-3314）。
- `format_operator_memory_for_reply_prompt`（3322-3334）：过滤 revoked，按 `- (kind) content` 渲染，空输入空串。
- `record_operator_memory`（3341-3410）：kind 必须 ∈ {preference, rejection, context}（3351-3355）；content 空 → BadRequest；同 (ws,acct,operator,kind,content) 且未撤销 → 只 bump last_used_at 不重复插入（3373-3390）；否则插新行。
- `revoke_operator_memory`（3423-3495）：revokedBy 非空、reason 1..=200 字符（3434-3443）；scope 外 id 一律 NotFound（防跨租户探测，3420-3422 注释）；已撤销幂等返回 `already_revoked=true` 保留首次审计（3456-3461）；软删 $set revoked_at/by/reason。
- 测试段（3497-5056）：候选状态/触发策略、tag observation、R7 deprecation 全分支、⑨件一救回、⑨件二检测/丢弃 gate、memory_summary 归位、P5 PBT（64 case deprecation 不变量）、OCC filter 形状、H17 维度指引渲染、窗口渲染注入剥离、reconfirmed/personality 解析、A-01/A-02。

### 2.2 `src/agent/reaction.rs`（1798 行）

- `ReactionOutcome`（30-35）：{claimed, outcome_status, stop_requested}。
- 入口 `record_user_reaction[_with_outcome]`（37-79）：**第一步**即确定性 stop 检查——`explicit_stop_intent` 命中直接 `apply_deterministic_stop`（52-58，在加载 prompt/config/claim 之前，保证预算耗尽/LLM 故障/畸形输出永远不能把明确 stop 翻成 unclassified）；否则建 reaction 专属 RunBudget（`reaction_token_budget` 默认 8000 / `reaction_max_llm_calls` 默认 2，runtime.rs:950-951）进 `RUN_BUDGET.scope`。
- **确定性 stop 词表** `explicit_stop_intent`（84-162）：归一化 = trim + 小写 + 去空白与中英标点（86-96）；空或 >96 字符 → false；**否定/引用防误判**（"没有说/没说/不是说/并不是/并非/别误会/举例/比如" 任一命中 → false，101-115）；DIRECT 19 词（别再联系我/取消订阅/退订/unsubscribe/stopmessagingme/donotcontactme/removemefromyourlist 等，116-137）；省略宾语式（"别再发了"等 4 词）必须与终结语境（"不想聊了/到此为止/别打扰我"等 8 词）同现才算（140-162）。
- **确定性 buying 词表** `explicit_buying_intent`（170-240）：>120 字符 false；否定/假设集 26 词（"如果/比如/不买/先不付款/取消订单/再考虑一下"等，186-217）；COMMITMENTS 17 词（"我要买/帮我下单/现在付款"等，218-240）。只作反应标签，**never** 产生成交/支付事实（164-169 注释）。
- `apply_deterministic_stop`（242-332）：写 contact `cooldown_until = now + 100 年` + `operation_policy.explicitStopRequested[At]`（249-277，**DB 失败仍返回 stop_requested=true**——库故障绝不能翻成允许发送，274-277）；find_one_and_update 最新一条 `status="sent" && outcome_status ∈ [null,pending,analyzing]` 的 review → outcome=`user_replied_stop_requested` + analysis{deterministic:true,confidence:100} + **unset reaction_claim 字段**（可覆盖他人 analyzing claim——确定性 stop 优先级最高，旧 LLM 结果回来 CAS 会 miss 被丢弃）（279-314）；`outbox::cancel_for_contact_on_user_reaction`（316-325）；返回 {claimed:true, stop_requested:true}。
- `record_user_reaction_inner`（334-602）：
  1. **stuck 兜底**：`outcome_status="analyzing" && reaction_claimed_at < now - reaction_analysis_claim_timeout_seconds(默认 60s，config.rs:534-536)` → 批量重置 pending + unset claim 字段（340-366）。
  2. **claim 原子性**（368-400）：find_one_and_update（sort created_at:-1 取最新 sent review），filter `outcome_status ∈ [null,"pending"]`，$set analyzing + reaction_claimed_at + 新 uuid `reaction_claim_token`，`$inc reaction_claim_generation`；拿到 Some 才是锁主，否则直接返回 default（本次 webhook 不调 LLM）。
  3. **分析三分支**（411-462）：`deterministic_buying = profile.transaction_facts_enabled && explicit_buying_intent`（428-429）→ `{buyingSignal:true, deterministic:true, confidence:100}`；预算超额 → `user_replied_unclassified` + degraded + `mark_degraded("reaction_skipped_budget_exceeded")`；否则 `analyze_user_reaction`，Err 回落 unclassified/confidence 0（458-461）。
  4. **outcome + misjudge**：`reaction_outcome_status_with_polarity`（463）；`compute_reviewer_misjudge_signal_with_polarity(claimed_review.approved, outcome, effective_negative_outcomes(polarity))`（470-474）。
  5. **CAS 提交**（492-515）：filter `{_id, outcome_status:"analyzing", reaction_claim_token}` —— token 不符（被 stuck 重置或他人重 claim）→ matched 0 → **丢弃 stale 结果**（旧执行者无法覆盖新结果，508-515）；$set outcome_status/reaction_analysis/send_gateway_result.userReaction*，misjudge 有则 set 无则 unset。
  6. **intent_trajectory**（517-535）：`push_intent_trajectory_entry` best-effort，失败仅 warn。
  7. **负例入队**（537-561）：misjudge=="approved_but_user_negative" 且 reply_text 非空 → `enqueue_negative_example_chunk` best-effort。
  8. **outbox 取消**（563-596）：`outbox::outcome_signals_stop(outcome)` 为真 → 取消同 contact pending/in_flight outbox，best-effort。
- `analyze_user_reaction`（604-664）：prompt = `user.reaction.system/task`（无 fallback，load 失败即 Err）+ 极性 addendum + 轨迹 addendum + memoryCard(to_document) + 整个 OperatingMemory JSON + 入站原文经 `inbound_prompt_content`（隔离+按来源剥哨兵，647-650）。`generate_agent_json(prompt_key="user.reaction.task")`。
- **outcome_status 推导** `reaction_outcome_status_with_polarity`（678-701）优先级：① 显式 `outcomeStatus/outcome_status` 字符串直接 passthrough（域无关，682-685）；② `stopRequested` → `user_replied_stop_requested`（**域不变红线**，极性不可覆盖，687-688）；③ `buyingSignal` → `polarity.positive.first()`（空回落 `user_replied_buying_signal`，689-695）；④ `objection` → `user_replied_objection`（696-697，刻意保留 DEFAULT 字面量：flag 是销售 prompt 专属输出）；⑤ 兜底 `user_replied_unclassified`（删失态，绝不臆测正负）。
- 极性基建：`default_outcome_polarity_for_reaction`（712-720，本地第三份手抄，测试 1791-1797 钉死与 `domain_profile::default_outcome_polarity()` 相等）；`reaction_polarity_prompt_addendum`（730-745，与 DEFAULT 相等 → None 字节等价）；`reaction_trajectory_prompt_addendum`（763-788，空集或单维 objection_type → None；否则列 `snake_to_camel(kind)`+display_name 指示 LLM 额外输出）。
- misjudge：`compute_reviewer_misjudge_signal_with_polarity(approved, outcome, negative)`（820-833）——approved=true 且 outcome ∈ 负极集 → `Some("approved_but_user_negative")`；`blocked_but_user_positive` 分支刻意不在此计算（留给 feedback_worker，796-798）。`DEFAULT_NEGATIVE_OUTCOMES` 5 词（838-844：objection/stop_requested/unsubscribed/negative/complaint）；`effective_negative_outcomes`（850-861）：profile 负极空 → 回落 5 词。
- `enqueue_negative_example_chunk`（875-989）：chunk_type="negative_example"、status="draft"、integrity_status="needs_review"（**不直接进 verified 池**，admin 复核后才生效）；确定性 `_id` = SHA256("reviewer-misjudge-negative-example-v1", ws, acct, review_id) 前 12 字节（991-1010）；chunk + create revision 在**同一 Mongo 事务**里原子落地（934-961）；duplicate key → abort 后按完整业务身份查存在即幂等 Ok，身份不符 → `Conflict("negative_example_identity_conflict")`（962-983）。幂等边界 = (workspace, source_review_id)（873-874）。
- `format_reaction_hint`（1025-1052）：最近 ≤3 轮 reaction 渲染 prompt 段（status/buying/objection/stop/摘要），空输入空串。
- `snake_to_camel`（1060-1074）：写侧与 addendum 共用的字节锚点；characterization 测试锁定边界行为（`foo_`→`foo`、`_foo`→`Foo`、`a__b`→`aB`，1759-1780）。
- `push_intent_trajectory_entry`（1082-1181）：turn_index = 该 contact inbound 消息总数 count（best-effort，1091-1104）；维度集空 → 回落 `default_trajectory_dimensions()`（单维 objection_type，1117-1123）；每维读 `reaction_analysis[camelCase(kind)]`（兜底 snake），**过 `dimension_registry::validate_dimension_value(MachineWrite)` 字典归一**，Drop 静默丢弃（轨迹是观测数据不进闸门，1131-1147）；objection_type 写旧字段 `objectionType`（字节等价），其它维度进 `dimensions` 容器（1149-1155）；Mongo `$push + $each + $slice:-50` 原子滑窗（`IntentTrajectoryEntry::MAX_ITEMS = 50`，models.rs:5494；1160-1179）。
- `cap_intent_trajectory`（1188-1200）：纯函数镜像 $slice:-50 供 PBT。
- `format_intent_trajectory_hint`（1206-1231）：最近 5 项按时间序渲染（旧在前），DEFAULT objection_type 渲染字节等价、profile 维度从 dimensions 容器（BTreeMap 键序稳定）。

### 2.3 `src/agent/sufficiency.rs`（342 行）

- `PromptTier`（12-19）：Lean / Relational / Full 三档。`TierDecision`（23-30）：Enough / Escalate(tier) / Clarify。
- `decide_tier_escalation(decision)`（33-48）：sufficiency=="enough"→Enough；"need_more_context"→按 missing_tier（"relational"/"full"，**非法值回落 Full** 保守）；"need_clarification"→Clarify；其它（畸形）→Enough 兜底。
- `forced_full_context_reason(decision, has_cited_knowledge, has_referral)`（55-73）：仅在 sufficiency=="enough" 时判——`decision_requires_knowledge` → "lean_declared_knowledge_required"；有引用型知识上下文 → "knowledge_route_cited_context"；显式引荐上下文 → "explicit_referral_context_requested"；只选上下文档位，**不选业务动作**（50-54 注释）。
- `is_sufficiency_recognized`（78-83）：三态闭集谓词，false=LLM 输出畸形（观测埋点 ptier_self_assessment_malformed 用）。
- `is_coverage_optimism(decision, coverage)`（90-94）：enough && coverage=="weak" && 需知识 → 观测灰区（正向 =="weak"，绝不 !=；先观测后判罚，不改档位）。
- `should_record_used_knowledge_ids(forced_full, escalated_to_full)`（103-105）：**只有 Full 档才记**（Lean/Relational 没读切片，记路由 id 会让 grounding 硬闸把"没读过的切片"当已读，架空 `blocked_unverified_product_claim` 红线，96-102 注释）。`resolve_used_knowledge_ids`（110-120）：非 Full 一律清空（**含 LLM 经 carry_through 自报的 id**，KB-01）。端到端测试 321-341 证明非 Full + 自报 verified id → `compute_verified_chunks` 交集为空。

### 2.4 `src/agent/post_decision.rs`（1435 行）

常量（22-29）：`MIN_CLAIM_LEASE_MS=120_000`、`POLL_MS=500`、`MAX_BACKOFF_MS=300_000`、`MAX_SNAPSHOT_MESSAGES=20`、`MAX_SNAPSHOT_PRODUCTS=100`、`MAX_MESSAGE_CHARS=4_000`、lease 集合 `post_decision_contact_leases`、`SCRUB_POLL_SECONDS=3600`。

- **快照构造**：`compact_authorized_decision`（59-70，replyText 截 4000）；`compact_contact_snapshot`（72-91，humanProfileNote/memorySummary 截 2000，intentTrajectory 取最近 10、outcomeEvents 最近 20）；`compact_memory_snapshot`（93-102）；`compact_messages`（104-119，尾部 20 条、content 截 4000）；`compact_products`（121-137，前 100、summary 截 1000）。
- `persist_projection_snapshot`（140-214）：Gateway 在发送授权路径调用。读 contact 的 `profile_revision` 作 baseline（153-171）；payload 含 authorized_decision/memory_snapshot/context_pack/domain_config/active_profile/active_products/contact_snapshot/product_snapshot/ascending_window/locale/run_id/baseline_profile_revision（172-188）；`ensure_snapshot_size`（44-53，超 `post_decision_snapshot_max_bytes` 默认 2MiB → Err）+ SHA256 hash（31-34）；`$set post_decision_status="prepared"` + payload + bytes + sha + attempts=0（191-207）。**客户投递永不等投影**：准备失败走 `mark_preparation_failed`（216-235，failed_terminal + error_kind="snapshot_preparation" + scrub_at=now+retention(默认 14 天，config.rs:523-528)）。
- **生命周期推进**：`activate_projection`（586-601）：prepared→pending + next_retry_at=now（**只在 no-reply 落定或文本发送持久授权后调用**，585 注释）；`discard_projection`（603-617）：prepared→discarded + $unset payload（决策被取消时）。
- `runnable_filter(now)`（619-641）：review `status ∈ [outbox_enqueued, sent, no_reply]` 且（post_decision_status ∈ [prepared,pending,retry] 且 next_retry_at 缺失/≤now）或（processing 且 locked_until 缺失/≤now = 前任 lease 过期可抢）。
- **contact lease（跨进程单飞）**：`contact_lease_id = ws\u{1f}acct\u{1f}wxid`（643-645）；`acquire_contact_lease`（652-710）：`_id=lease_id` 上 find_one_and_update upsert，filter `claim_token==token || locked_until 缺失/null/≤now`——同 token 可重入、过期可夺；upsert 撞唯一 _id（并发首建）→ Ok(false)（704-709）。`release_contact_lease`（712-727）：delete_one 限定自己 token。
- `claim_one`（729-808）：候选扫描 limit 32、排序 next_retry_at→created_at→_id（730-743）；`seen_contacts` 每 contact 只考虑最老一条（热点 contact 不能吃满所有 lane、不反复改写 review 状态，750-766）；先拿 contact lease 再做 review CAS（$set processing/claim_token/locked_until/last_claimed_at + $inc attempts，778-799）；review CAS 输 → 释放自己 lease（803-806）。
- `claim_lease_ms`（810-823）：`llm_timeout*max_retries*1000 + retry_base*(retries-1) + 60s`，下限 120s——lease 覆盖最坏 LLM 时长。`renew_claim_and_contact_lease`（825-859）：review + lease 双续，任一 miss → `claim_lost()`（Conflict）。
- **投影 LLM**：`load_or_freeze_projection_prompts`（321-398）：payload 已有冻结 prompt 直接用；否则按 locale 加载 `user.projection.system/task` 并**冻结进 payload**（版本也记 post_decision_prompt_versions）——重试期间 prompt 变更不影响本条。`projection_user_payload`（243-319）：JSON 输入按字符预算逐级裁剪，顺序 contextPack→domainConfig→activeProducts→activeProfile→operatingMemory（264-277）→ 会话窗口对半砍留最新（280-299）→ contactSnapshot（304-310）；再超 → Err "projection prompt too large"（永不裁 authorizedDecision）。`load_or_generate_projection`（400-522）：已有持久化结果直接复用（崩溃恢复不再调 LLM，411-413）；否则子预算 RunBudget(`{run_id}:projection`, `post_decision_token_budget` 默认 32000（config.rs:520-522）, **max_calls=1**, tool=0)（448-453）；生成期间 **20s 心跳续 lease**（tokio::select 循环，467-475）；结果 CAS 持久化（filter 带 `post_decision_projection_result $exists:false`，476-504）+ `post_decision_safe_to_regenerate=true`；未知字段记 `post_decision_unknown_fields`（507-519）；`DeferredProjectionDecision::from_value` 校验失败 → "validate projection result"（永久错误）。
- `guard_projection_taxonomy`（524-583）：投影产出的分类字段过全局 taxonomy cache + FSM stage 键集 `compute_taxonomy_guard_outcome`/`apply_taxonomy_guard_outcome`；候选词 `upsert_candidate_once_per_run`（失败仅 warn）；`normalize_domain_signals` 收尾。
- `process_claimed`（951-1205）主流程：解 payload（contact 消失 → discarded/"contact_not_found"，984-991）；重建窗口 ConversationMessage（1005-1042）；runtime 从快照 domain_config + active_profile（1043-1044）；投影生成后 `renew`（1058）；**关闭再生成门** `$set post_decision_safe_to_regenerate=false`（1061-1073，从此业务效应可能持久化，换模型结果会混代际）；`guard_projection_taxonomy`（1075-1083）；
  - **profile 段**（1086-1158）：`has_newer_applied_projection`（877-903：同 contact 存在 created_at 更新且 profile_done=true 的 review → stale）；stale → `apply_append_only_projection`（905-949：只写 memory_candidates/tag_observations/stage_observation + 仅 agent_generated_signals 的 `apply_agent_updates`，**清掉一切 stateful 画像字段**，930-933）+ conflict_kind="newer_projection"；非 stale → `apply_agent_updates(..., ProjectionWriteGuard{baseline_profile_revision, review_id})`，结果三态：Applied / AlreadyApplied（同 review 重放）/ FencedConflict（revision 栅栏冲突 → 转 stale 走 append-only）（1109-1144）；段位标记 `post_decision_profile_done=true + skipped_stale + conflict_kind` CAS（1146-1157）。
  - **memory 段**（1160-1187）：非 stale 才 `apply_operating_memory_update`；标记 `post_decision_memory_done=true`。
  - **完成**（1189-1204）：completed + $unset payload/projection_result/claim_token/locked_until/next_retry_at/error。
- 错误处理：`projection_error_kind`（1207-1227，claim_lost/invalid_projection/prompt_too_large/invalid_snapshot/llm_unavailable/database/processing）；`permanent_projection_error`（1229-1234：invalid_projection/invalid_snapshot/prompt_too_large 三类永久）；`settle_failure`（1236-1278）：terminal =永久错误或 attempts ≥ `post_decision_max_attempts`（默认 8，config.rs:511-513）→ failed_terminal + scrub_at；否则 retry + 指数退避 `1000ms << min(attempts-1, 8)` 封顶 5min（1261-1262）；CAS 限定自己 token。
- 清扫：`scrub_expired_terminal_snapshots`（1280-1307）：failed_terminal/discarded 且 scrub_at≤now → $unset payload/projection_result/safe_to_regenerate（**保留审计行**只脱敏大字段）；`run_snapshot_scrubber` 每小时（1309-1320）。`run_worker`（1345-1350）：`post_decision_worker_concurrency`（默认 4，config.rs:508-510）条 lane + 1 scrubber；lane 循环 claim→process→settle→release（1322-1343）。

### 2.5 `src/agent/projection_observations.rs`（138 行）

- 目的：崩溃重放的 post-decision 效应需要严格 (entity, run) 幂等。主实体只留聚合计数，本集合持有严格身份（1-5 注释）。
- `record_and_count`（25-70）：把 legacy_run_ids ∪ 当前 run_id 全部 insert（duplicate key 吞），再 count 返回台账权威计数——先补台账再截显示缓存，极晚重放的历史 run 也保持幂等。
- `reconcile_stages(ledger_count, run_id, legacy_count)`（75-111）：聚合管道两段——① `observation_ledger_baseline = ifNull(已有, max(0, occurrences - legacy_count))`（吸收台账化之前的存量）；② `occurrences = max(现值, baseline + ledger_count)`（$max 保并发单调），`source_run_ids` 过滤掉本 run 后 append 再 `$slice:-32`（`RECENT_RUN_IDS_LIMIT=32`，15）。
- `source_run_ids(doc)`（113-124）：读数组去重保序。

### 2.6 `src/agent/consolidation_window.rs`（77 行）

- `take_window_by_budget(msgs, char_budget, max_messages)`（6-27）：入参须升序；从最新往回累积，条数先到或（非首条时）字符将超预算即停；**首条永远收下**（即使单条超预算，19 行 `!picked.is_empty()` 条件）；返回升序子集。微信碎消息适配：字符预算保信息量下限，条数防全寒暄垃圾号空耗（5 注释）。

### 2.7 `src/agent/run_envelope.rs`（2059 行）

- **source_kind 六常量**（34-45）：inbound_message / follow_up_task / manual_send / principal_escalation / principal_clarification / system_incident（后三者是内部通知，不继承客户会话副作用）。
- **lifecycle 七态**（48-54）：started / running / completed / failed_before_decision / failed_after_decision / aborted_by_budget / aborted_by_external_signal。
- **finalReviewStatus 闭集 10 值**（68-79）：approved / revision_applied_approved / revision_failed / held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context / blocked_by_required_field / blocked_by_budget / blocked_unverified_product_claim / legacy_mode_unchecked。
- **gateway_status 闭集**（87-147）共 40 值：核心（pending/approved/allowed/sent/no_reply/review_blocked/revision_* 3 种/hold 三类/blocked_* 3 种/tool_loop_timeout/legacy_mode_unchecked）+ precheck 集（not_managed/cooldown/rate_limited/daily_limit/expired/context_changed/policy_cooldown/policy_wait_user_reply）+ S5 补录（gateway_blocked/precheck_blocked）+ outbox 四态（outbox_enqueuing/enqueued/enqueue_failed/enqueue_partial_failure）+ 外部信号（stale_task_claim/skipped_duplicate/admin_cancelled/superseded_by_new_inbound/user_reaction_stop_requested/quiet_hours_deferred）+ internal_error。
- **禁词集**（152-158）：held_for_human / human_required / waiting_for_human / handoff_to_human / manual_takeover。
- 断言：`assert_final_review_status_valid`（165-186）与 `assert_gateway_status_valid`（192-212）——空串合法（envelope-started 占位）；禁词或闭集外 → tracing::error + `AppError::External`（fail-closed 不写库）。`assert_lifecycle_valid`（219-237）——**空串不合法**（终态写入必须显式 lifecycle）。
- `derive_lifecycle_from_status(gateway_status, error)`（249-280）：blocked_by_budget 或 error 含 budget_exceeded/BudgetExceeded → aborted_by_budget（优先级最高，250-256）；outbox_enqueuing → running；sent/no_reply/approved/allowed/outbox_enqueued/skipped_duplicate → completed；precheck 9 态 → failed_before_decision；superseded_by_new_inbound/user_reaction_stop_requested/quiet_hours_deferred/stale_task_claim → aborted_by_external_signal；**其余全部 → failed_after_decision**（hold 三类也在此，P2-5 修正，测试 1234-1273）。
- `is_valid_lifecycle_transition(from, to)`（296-355）：未知字符串双向非法；同态幂等合法；五终态是吸收态（不可转出，含终态互转非法）；started→{running, 全部终态}；running→{全部终态}（不可回 started）。**注意：此函数是纯函数，`update_run_envelope_terminal` 并未调用它做 CAS**（见 §5 疑点 1）。
- `write_run_envelope_started`（384-435）：任何 LLM 调用前 insert lifecycle="started"、status="pending"、gateway_result={gatewayStatus:"pending"}、final_review_status=""、9 个 R3 字段占位空。**刻意不 try/catch**——它本身是兜底层，Err 直接向上（357-361）。
- `mark_run_envelope_running(db, run_id, decision)`（442-459）：CAS `{run_id, lifecycle:"started"}` → running + decision 快照；只命中开放 started 行，绝不重开终态。
- `AgentRunLogTerminalFields`（468-503）：全 Option 字段集；`to_set_document`（508-611）None 字段不出现在 $set → 多次部分 update 互不覆盖（如先落 lifecycle 再异步落 outbox_status，测试 909-931）。
- `fail_run_envelope_if_open(db, run_id, summary)`（616-648）：summary 截 1024；两次 CAS——started→failed_before_decision、running→failed_after_decision；status/gateway_result 都写 internal_error；终态行不受影响；返回是否实际关闭。
- `update_run_envelope_terminal(db, run_id, fields)`（657-743）：写前枚举校验（lifecycle/final_review_status/status/gateway_result.gatewayStatus 四处，663-681）；空 set 直接返回；`update_one({run_id})` matched=0 → **恢复路径**：`insert_envelope_recovery`（746-812，构造最小 AgentRunLog，trigger_kind 缺省 "envelope_recovered"，用 `update_one + $setOnInsert + upsert` 而非 insert_one 防与迟到信封撞 run_id 唯一索引）+ 写 `agent_events kind="run_envelope_recovered_via_insert"` status="warning"（含 lifecycle/final_review_status/autonomy_mode/error_summary details，707-741）。
- panic hook：`install_panic_hook_for_envelope`（830-848）：`Once` 全局仅装一次；hook 只 tracing::error（message+location），**不做 async DB 更新**（panic hook 不能 .await；lifecycle 推进由 catch_unwind 包装层做，818-828）；链式调用先前 hook 保留 backtrace。`panic_message_from_info`（851-860）：&str / String 双 downcast。
- 测试段（862-2059）：$set 形态、R9 audit 五场景、禁词严格拒收、S5 补录回归门、autonomy_mode 三合法值、lifecycle FSM 全套、tool_calling 中间轮跳过校验/final 轮 7+7 字段必填、invalid_enum（risk_level=critical、autonomy_mode=manual）、前端契约 fixture 对齐（1172-1177）。

### 2.8 `src/agent/run_audit.rs`（538 行）

- `StageTiming`（26-30）：count/total_ms/max_ms。`RunPathMetadata`（33-71）：run 路径分类，`kind()` 优先级 manual > no_reply > revision > rewrite > escalated（tier≠lean）> direct。
- `LlmAuditFlushReport`（75-100）：queued/persisted/batch_succeeded/fallback_used/failed/error(截 512)/latency_ms → `gateway_result.performance` 子文档。
- `RunAuditBuffer`（104-311）：短同步锁（parking_lot Mutex，绝不跨 .await 持锁，102-103）。`push_llm_log`/`push_observability_event` 入队时补稳定 ObjectId（123-137，使 insert_many 部分成功可通过 replace_one(upsert) 幂等重放）。`record_stage`（159-166）聚合计时。`performance_document`（168-192）：totalMs + path + stages + llmLogFlush + eventLogFlush。
- `flush_llm_logs`（198-254）：先 `std::mem::take` 清空缓冲；`insert_many` 快路径；**任何失败**（含部分成功）→ 逐条 `replace_one({_id}, upsert)` 重放（已插入的被替换、缺失的被补插，无重复）；失败计数与错误串联进 report。`flush_observability_events`（256-310）同构。
- task-local `RUN_AUDIT_BUFFER`（313-315）；`try_buffer_llm_log`（319-327）：gateway scope 内入队 Ok，scope 外 Err(log) 归还调用方走既有即时 insert。
- `BUFFERED_OBSERVABILITY_EVENT_KINDS`（329-337）：仅 7 种 `ptier_*` 事件可缓冲（self_assessment_malformed/forced_full/coverage_optimism/relational_optimism/escalated/clarify/run_tier）；其它 kind 一律 Err 走即时写（339-350）。
- `mark_tier/rewrite/revision/no_reply/manual`（358-376）：`with_current_audit` 静默 no-op（scope 外）。`RunStageTimer`（379-399）：RAII，Drop 时记录（正常返回/错误传播/unwind 三路都覆盖）。`record_llm_queue_wait`（401-415）：总排队 + 按 Foreground/Background 分桶。

### 2.9 `src/agent/budget.rs`（607 行）

- `BudgetError`（30-47）：LlmCallLimitReached / ToolCallsExceeded / TokensExceeded（R4.3 硬上限，dispatcher 转成 `{"error":"budget_exceeded"}` 工具结果回传）。
- `RunBudget`（55-90）字段：run_id、`run_mode`（"live"/"shadow"，task-local 传导使嵌套读写共享副作用边界）、token_budget、max_llm_calls、tool_call_budget（knowledge_max_tool_calls 注入，clamp [1,16] 默认 6；测试/无 tool-loop 传 i32::MAX）、五个计数 Mutex（tokens/llm_calls/unknown_usage/tool_calls）、`escalation_bonus`（B-1：升档只抬判定上限不改真实累计）、`llm_call_bonus`（升档/rewrite/revision 显式授予的额外 LLM 槽）、degraded_reasons、prompt_versions。
- 关键方法：
  - `try_reserve_llm_call`（137-148）：**dispatch 前原子占位**（`calls >= effective_max` 即拒），并发不会冲破硬上限（测试 354-374：8 线程抢 3 槽恰好 3 成功）；`record_reserved_call_usage`（151-156）完成后只加 tokens 不重复计数；usage 未知 → `unknown_usage_calls += 1`（tokens_used 刻意只累计已报告值，69-73）。
  - `effective_max_llm_calls`（158-162）= max_llm_calls + bonus；`grant_additional_llm_calls`（165-170）；`available_llm_calls_before_tail(reserved_tail)`（173-178）：为必跑的 Reviewer/ClaimGate 保尾。
  - `grant_escalated_ceiling(escalated_total)`（183-186）：bonus = max(0, escalated_total - token_budget)，幂等、绝不缩小上限。
  - `record_tool_call(tokens_consumed)`（202-223）：负数 clamp 0；锁固定顺序 tokens→tool_calls 防死锁（199-201）；先查 tool 上限再查 token 上限（用有效上限=base+bonus），**失败不污染计数器**（原子语义）；成功同时 +1 tool +N tokens。
  - 三层判定：`is_llm_or_token_exhausted`（229-233）——token 或 LLM 维度耗尽（**tool 额度刻意排除**：知识工具用完不得压制已保留容量的强制 Reviewer/ClaimGate，225-228）；`is_exceeded`（237-239）= 前者 ∪ tool 耗尽（可选工作总闸）；`should_stop_optional_llm_calls`（245-247）= is_exceeded ∪ unknown_usage_calls>0（token 余量不可证明时停可选工作，但绝不用于拒绝已生成回复或替代必需安全审查）。
  - `record_prompt_version`（255-259）：None（内置 fallback）不写伪版本；同 key 重写幂等。`snapshot`（265-281）全字段快照。
- task-local：`RUN_BUDGET`（302-307，仅 gateway/simulate/consolidate 等入口 scope）；`ShadowEvaluationSnapshot`（316-321：active_profile/active_products/evaluated_at/taxonomy_cache——Prompt Shadow 双支比较的不可变共享输入，防 TTL 刷新与墙钟越界引入第二变量）+ `SHADOW_EVALUATION_SNAPSHOT`（323-325）+ `current_shadow_evaluation_snapshot`（327-329）；`current_run_budget`（332-334）；`current_run_mode`（338-342）：scope 外默认 "live"（只有显式 shadow scope 才可抑制业务写）。

### 2.10 `src/agent/simulation.rs`（312 行）

- `simulate_user_dialogue`（39-47）→ `_with_budget`（59-102）：加载 domain_config + resolve_thresholds（与生产 review 同阈值）+ active profile；RunBudget 用 `simulation_token_budget`（默认 300000）+ `.with_run_mode("shadow")`（77-85）；`SimulationRunOutcome` 把 `AppResult<turns>` 包在结果里（失败的模拟也可能已消耗 LLM，配方评估须先记账再处理业务错误，49-57）。
- `simulate_user_dialogue_inner`（105-312）逐轮：合成 inbound（message_id=`shadow-{n}`、raw={runMode:"shadow"}，123-137）；`precheck_send_gateway` 真闸（139）；memory 走 `load_operating_memory_read_only`（115）；**永远先跑知识路由**（WB5 与生产对齐）——预算超额则空路由 + `mark_degraded("simulation_knowledge_route_skipped_budget_exceeded")`（163-180）；`decide_reply_with_promote(PromptTier::Full)`（183-202）；normalize state/runtime + planner（203-214）；review：`is_llm_or_token_exhausted` → `local_decision_review` 降级（217-220），否则真 `review_decision`（221-241）；`finalize_shadow_decision`（242-254）；状态映射：gateway 不通过 → "gateway_blocked"，final=="approved" → **"would_send"**，否则透传 final_status（257-263）；每轮打包 `UserOperationSimulationTurn`（决策/评审/gateway/知识路由/context_pack/memory_preview/state_transition from→to，274-291）；would_send 时把回复以 outbound 合成消息推进 history 供下一轮（293-309）。全程零写库。

### 2.11 `src/agent/prompt_shadow.rs`（788 行）

- `PromptShadowSample`（60-93）：source_run_id、status（completed/failed）、failure_reason、original/new scores、original/new final_review_status、new_review_risks、self_critique_addressed 两侧。
- `shadow_replay_prompt_one(state, proposal, source_run_id)`（101-318）失败语义：取不到源 run/contact/inbound、proposal 缺 key/snippet/base_revision、模板校验失败、预算超额 → 返回 `status="failed"` 样本**不抛错**；真正 DB/LLM 故障才 Err（95-99）。步骤：
  1. 源 run 反查（三键 filter，320-330）；候选片段必须齐（proposed_template_key + diff_snippet，129-141）；contact 反查（142-172）。
  2. inbound 用**真实历史消息**：只支持 `source_kind==inbound_message`（follow_up 的 source_event_id 是 task hex 必 miss，显式短路，174-193）；空串/`synthetic:` 前缀 → source_message_unavailable。
  3. **冻结模板**：`proposal.base_revision` token 解析出 template_id/version/content_sha256，三者与库中行任一不符 → fail-closed `prompt_base_revision_unavailable`（217-260）。
  4. runtime + resolve_thresholds + active profile/products；taxonomy 进程缓存预热后 `snapshot_copy` 进 `ShadowEvaluationSnapshot`（evaluated_at 固定一次，277-289）；**pin LLM provider generation**（registry snapshot 替换进克隆 state，防 A/B 中途换供应商，291-300）；整个对照包进 `SHADOW_EVALUATION_SNAPSHOT.scope`（302-317）。
- `shadow_replay_inner`（382-513）：`shadow_dependency_fingerprint` 在 prepare 前、baseline 后、candidate 后**共 3 次校验**，任何不符 → `shadow_dependencies_changed`（393/413-418/455-460/494-499）；prepare（独立 budget scope）→ baseline 支（override 片段为空串 + 冻结 base content）→ candidate 支（同 base + append_snippet）；各支独立 `new_shadow_budget`（simulation_token_budget，run_mode="shadow"，369-379）；支内失败枚举 `BudgetExceeded` / `TargetNotApplied`（364-367）。
- `prepare_prompt_shadow`（515-578）：与 simulation 同一套只读加载链；预算两处检查（549-551/563-565）返回 None → budget_exceeded。
- `run_prompt_shadow_branch`（581-674）：decide_reply_with_promote（PromptTier::Full + `Some(prompt_override)`）→ normalize → review_decision（同 override）→ **`prompt_override.was_applied()` 为假 → TargetNotApplied**（652-654，目标模板没被实际装载即判失败）→ `finalize_shadow_decision` → 产 scores/final_status/risks/self_critique_addressed。
- `shadow_dependency_fingerprint`（676-768）：对 **19 个集合**（contacts/conversation_messages/agent_tasks/operating_memories/agent_decision_reviews/agent_send_ledger/knowledge_operator_memory/operation_playbooks/operation_domain_configs/operation_state_policies/prompt_templates/domain_profiles/agent_souls/products/system_taxonomies/content_assets/referral_cards/operation_knowledge_documents/operation_knowledge_chunks）按 tenant filter 全量拉行（_id 排序）+ prompt_pack_version 原子计数一起 SHA256。**无 limit 全表段扫描**（见 §5 疑点 7 性能注记）。

### 2.12 `src/agent/shadow_finalize.rs`（196 行）

- `finalize_shadow_decision`（28-126）：复用生产 `ensure_independent_claim_gate`（59-72）+ `finalize_review_for_send_at`（74-86）+ 状态动作策略 `enforce_state_action_policy`（93-116，违规 → should_reply=false / autonomy_mode="blocked" / final="held_by_ai_policy" + risk "state_action_policy_blocked"）。active_profile/products/evaluated_at 优先取 shadow 快照，无快照现载（40-58）。**pending_events 只是诊断描述，绝不持久化**（88-90）。
- `shadow_terminal_status`（128-146）：Approved 且 !should_reply → "no_reply"；Approved 且 needs_revision 且 revision_direction 非空 → **"revision_required"**（生产此时会跑一次 revision，shadow 不谎称未修订稿可发，测试 153-169）；Approved 其余 → "approved"；非 Approved → gateway_status 字符串。

### 2.13 `src/agent/prompt_isolation.rs`（763 行）

- 定界符：`<<<USER_TURN>>>` / `<<<END_USER_TURN>>>`（24-25）。
- 预算常量（30-40）：`TEMPORAL_CHAT_EVIDENCE_MAX_AGE_MS = 48h`；`HISTORY_MESSAGE_MAX_CHARS=800`；`REPLY_HISTORY_TOTAL_CHARS=4000`；`FULL_REVIEW_HISTORY_TOTAL_CHARS=4000`（12 条）；`LIGHT_REVIEW_HISTORY_TOTAL_CHARS=2000`（6 条）；占位 `[省略]`。这些只约束 prompt 输入，**ClaimGate 仍拿全量服务端证据目录**（32-33）。
- `budget_history_contents(contents, per_msg_max, total, preserve_positions)`（47-79）：从最新往旧分配；preserve_positions=true 时超预算旧位写 `[省略]` 占位（保序位对齐，Reply Agent 序位契约）；截断加 `…`（unicode 按 chars）。
- **时效档期权威**（服务端推导，Reply/Reviewer/ClaimGate 共享）：
  - `SchedulePosition`（102-118）：Denied / Confirmed / GenericInquiry / ConcreteProposal；label 映射 denied/confirmed/question_or_request。
  - `temporal_chat_evidence_is_fresh`（120-129）：age ≤ 48h。
  - `customer_statement_form`（131-160）：问句/请求标记词表（?？吗么能不能…）→ "question_or_request" 否则 "statement"。
  - `schedule_position(text)`（162-270）：先判否认词表（"没有预约/取消预约/不去了/no appointment"等 16 词 → Denied）；再判显式档期词（"预约/到店/几点/appointment"等 13 词）或具体时间词（"今天/周一/上午/三点/tomorrow"等 40+ 词，或数字+点/时/月/日/:/am/pm）；两者都无 → None；是问句 → 有具体时间 ConcreteProposal 否则 GenericInquiry；陈述 → Confirmed。
  - `message_matches_inbound`（272-287）：同 ObjectId、同 message_id、或（双方均无 id 时）created_at+direction+content 全等。
  - `build_temporal_fact_view(inbound, recent, evaluated_at)`（292-411）：候选 = 当前消息（synthetic relay 除外）+ 历史 inbound 前 12 条，按 created_at desc（当前消息平手优先）排序（316-350）；从最新往旧扫：**Denied / ConcreteProposal → 立即 break（否决更旧确认）**；GenericInquiry → continue；Confirmed → fresh 则设 active fact（当前消息文本用 "[见下方最新消息]"，历史文本经隔离+800 字预算）否则记 expired，然后 break（355-385）；`may_assert_concrete_schedule` = 有 active fact 且当前消息 position ∈ {Confirmed, GenericInquiry}（392-396，**无关消息不能让旧档期复活**）；不满足时 active_temporal_facts 输出空（398-402）。
  - `render_temporal_fact_view`（413-417）JSON 序列化（失败回退固定空视图串）；`history_temporal_metadata`（421-439）：每条历史行前缀 createdAtMillis/ageHours/temporalStatus=fresh|stale（Reply/Reviewer 与 ClaimGate 共享时间锚，"明天"不能漂到今天）。
- **隔离核心**：`isolate_untrusted(raw)`（450-453）= 剥已知 tag 后包 `<<<USER_TURN>>>…<<<END_USER_TURN>>>`；`strip_injection_tags`（457-459）只剥不包（已有外层 wrapper 的 callee 用）；`strip_known_tags`（487-496）剥 8 个固定子串：两哨兵 tag + `<user></user><system></system><assistant></assistant>`（见 §5 疑点 5 大小写）；`strip_relay_sentinel`（464-466）剥 `__PRINCIPAL_RELAY__`（models.rs:841）；`inbound_prompt_content(content, is_synthetic_relay)`（471-478）：合法 relay 保留哨兵触发转述模式（与 isolate_untrusted 逐字等价），客户来源剥哨兵（伪造哨兵无从触发转述，H10）；`history_prompt_content`（483-485）：history 行 = strip_injection_tags + 无条件剥哨兵（合法 relay 合成消息不落库不进 recent，零误伤）。

### 2.14 `src/agent/system_incident.rs`（1202 行）

- 定位：运维事故通知，**独立于 Ask-Human 业务升级**——只借用其受众配置，不产生客户决策/principal 卡/审批记录（1-5 注释）。当前唯一 kind：`llm_account_unavailable`（27）；两相固定文案 `LLM_OUTAGE_NOTIFICATION`/`LLM_RECOVERY_NOTIFICATION`（33-36，测试 694-701 保证不回显敏感上下文）。
- 身份：`incident_key = "llm_account_unavailable:{provider_id}"`（38-40）；`source_event_id = "system-incident:{oid}:{generation}:{phase}:{recipient_index}"`（58-68），`parse_source_event_id`（70-83）严格五段 + generation≥1。
- `freeze_recipients`（85-111）：decider 链 wxid trim 非空、account 缺省回落触发账号（再回落 `default_account_id`），(account, wxid) 去重保序。`resolve_recipients`（113-127）：从 domain_config 的 ask_human policy 取 decider_chain。
- `observe_llm_account_unavailable`（139-289）loop + 三分支：
  - **active 已存在**：`request_started_at < first_failure_started_at` 的迟到旧失败直接忽略（163-165）；否则 CAS（_id+active+generation）$set last_observed/reason + `$max last_failure_started_at` + `$inc occurrence_count`——并发观测收敛到一个 generation 只加计数（166-192）；CAS miss → loop 重读。
  - **recovered 已存在**：`request_started_at <= recovery_probe_started_at` 的迟到旧失败**不得重开**（195-203，因果栅栏）；否则 CAS（recovered+旧 generation）重开为 active、generation+1、recipients 重新冻结、计数/时间戳重置、$unset 两相 marker 与 probe（205-247）。
  - **不存在**：insert generation=1 新事故；duplicate key（并发首建）→ loop（250-288）。
  - 三分支成功后都 `materialize_incident_notifications`。
- `observe_llm_recovery(state, ws, request_started_at)`（293-352）：**成功的非缓存上游请求即恢复探针**；恢复所有 `last_failure_started_at < request_started_at` 的 active 事故（因果更老才算，恢复 CAS filter 再验一次，315-335）；先 `wake_provider_blocked_tasks_if_workspace_recovered`（354-395：workspace 无 active 事故时，把 `status="retry" && gateway_status="blocked_provider_unavailable"` 且 next_retry_at 在未来的任务全部提前到 now）**再**物化通知——崩在中间时 reconcile 会先重放同一幂等唤醒（341-347）。
- `materialize_incident_notifications`（397-408）：先物化 outage 相；**recovery 相只在事故已 recovered 且 outage 相全收件人已达终态时物化**（快恢复不得让告警消失、恢复通知不得先于告警，402-405）。`materialize_phase`（410-470）：逐收件人按 source_event_id 查 outbox 已存在即跳过，否则 `outbox::enqueue`（source_kind=system_incident、max_attempts=5、run_id=source_event_id）；完成后 CAS（_id+generation）打 `{phase}_enqueued_generation=generation` marker。`phase_is_terminal`（472-512）：每收件人 outbox 行 status ∈ [sent, failed_terminal, canceled, delivery_unknown] 恰好 1 条。
- `reconcile_notifications`（515-582）：崩溃恢复扫描——active 且 outage marker≠generation，或 recovered 且 recovery marker≠generation，按 updated_at 升序 limit 100；每条先做幂等唤醒（recovered 时）再物化；**无论成败都 bump updated_at 轮转**（毒行不能饿死后来者，marker 未补的行下轮仍可重试，546-569）。
- 发送前授权 `send_is_authorized(state, entry)`（587-626）：非 system_incident 直接放行；source_event_id 解析失败 / phase 非法 / 事故按 (id, ws, generation) 查不到 → false；`notification_identity_is_authorized`（628-657）：generation 必须相等；recipient_index 必须命中冻结快照且 account/wxid 全等；**outage 相在 active 或 recovered 都可发完**（已持久观测的告警不因快速恢复被压制，644-649），recovery 相仅 recovered；content 必须与固定模板逐字相等。claim 后与不可逆远端发送前**各调一次**（584-586）。
- 测试段（659-1202）：文案脱敏、source id round-trip、收件人冻结去重、授权栅栏矩阵、恢复 filter 形状、及一个 `#[ignore]` 的 Docker 真 Mongo 并发/因果/崩溃窗口全链路回归（872-1201：8 并发观测收敛 1 条通知、旧成功不恢复、新成功恢复+唤醒 blocked 任务、recovery 通知等待 outage 终态、崩溃窗口 reconcile 重放唤醒）。

### 2.15 `src/agent/tag_evidence.rs`（119 行）

- `resolve_evidence(window, turn_indices)`（11-29）：LLM 序位（0-based，窗口须与 prompt 呈现同序）→ `Evidence{turn, msg_id=oid.to_hex()}`；负数/越界/消息无 _id 一律丢弃（**fail-closed：锚不上不放水**）。
- `evidence_strength(evidences, window, explicit_intent)`（33-52）：`explicit_intent=false` → Weak；否则至少一条证据锚到 Inbound（客户本人）消息 → Strong，否则 Weak。强弱由 direction + explicit 标志客观决定，**不读 LLM 自称置信**。

### 2.16 `src/agent/multimodal.rs`（167 行）

- `MediaContent`（23-39）：bytes+mime，`to_base64`。
- `fetch_inbound_media`（49-55）：**打桩恒 `Ok(None)`**——MCP server 无确认的"下载入站媒体"tool（仓内零书面依据不能凭空实现，referral-card 同款纪律）；fail-soft 让下游走过渡话术，绝不 panic。
- `describe_inbound_image`（69-105）：真复用知识库 vision 链（`select_vision_provider` + `vision_generate_json` → `generate_json_with_image`）；system prompt 强约束"只描述真实可见、不编造"；输出 `{"description": …}`，空描述 → Err。地基阶段因下载未接通暂不会被触发。
- `non_text_transition_reply(msg_type)`（113-127）：image/voice/video/link|appmsg/miniprogram/file 各一条 AI 自治口吻过渡话术 + `_` 兜底，全部非空（客户永远只跟 AI 对话；措辞合规由 CI 文本门兜底）。

## 3. 跨文件机制：一轮对话的记忆/画像/轨迹数据从候选到固化的完整旅程

1. **决策轮（gateway 内）**：Reply Agent 输出 `memory_candidates / tags+tag_evidence_turns / customer_stage+stage_evidence_turns / agent_generated_signals / memory_write_score / consolidation_needed`。发送授权路径上 gateway 调 `persist_projection_snapshot`（post_decision.rs:140-214）把授权决策+运行上下文冻结成 `post_decision_status="prepared"` 快照挂在 decision_review 上；**客户投递从不等待投影**——快照失败只 `mark_preparation_failed`。发送落定（durable 授权或 no_reply）后 `activate_projection` prepared→pending（post_decision.rs:586-601）。
2. **投影轮（post_decision worker）**：`claim_one` 以 contact lease（跨进程同联系人单飞）+ review CAS 认领（post_decision.rs:729-808）；`load_or_generate_projection` 用冻结 prompt + 冻结输入跑**恰好 1 次**投影 LLM（子预算 32000 token，20s 心跳续租）并把结果持久化（崩溃恢复直接复用不再调 LLM）；`guard_projection_taxonomy` 过字典闸；然后分两段幂等推进：
   - **profile 段**：若已有更新的 review 完成投影（`has_newer_applied_projection`）或 `apply_agent_updates` 撞 revision 栅栏（FencedConflict）→ 判 stale，降级 `apply_append_only_projection`（post_decision.rs:905-949）——只写三类 append-only 数据：`write_memory_candidates`、`write_tag_observations`、`write_stage_observation`（全部在 memory.rs:2678-2830，带 projection_key 幂等 upsert）+ 仅弱信号的 apply_agent_updates；非 stale 才带 `ProjectionWriteGuard{baseline_profile_revision}` 全量写画像。
   - **memory 段**：非 stale 时 `apply_operating_memory_update`。两段各有 `post_decision_profile_done / memory_done` 标记，崩溃重放跳过已完成段；重放的实体级计数幂等由 `projection_observations::record_and_count + reconcile_stages`（(entity, run) 严格台账 + $max 单调对账）兜底。
3. **候选沉淀**：`write_memory_candidates` 经 `validated_memory_candidate`（importance/confidence 非 0 + 弱证据 importance 封顶 7）后由 `decide_candidate_status`（memory.rs:2833-2841）分流——`write_score>=6 或 max_importance>=8` → **status="pending"**，否则 "ignored_low_score"（终态，不再参与）。tags/stage 观察以 source="tag_observation" 恒 pending 入暂定层，证据锚空则 fail-closed 整批丢弃（tag_evidence.rs:11-29）。
4. **触发归并**：gateway 侧用 `contact_memory_consolidation_due`（memory.rs:2868-2912）判定（LLM 显式请求 / write_score≥6 / pending≥4 / 最老 pending≥6h 任一）→ `schedule_memory_consolidation_task`（memory.rs:2951-3127）single-flight：优先复活 failed、唤醒既有（rerun_requested）、收养遗留行，最后才建新 task（active_task_key 唯一索引仲裁）。
5. **归并固化（consolidator run）**：task worker claim 后进 `consolidate_contact_memory_inner`（memory.rs:1622-2201）：拉 ≤30 条 pending 候选 + 宽窗口对话（6000 字/60 条），把"升级后带稳定 id 的当前卡 + 候选 + 对话 + 当前确信标签 + 待重判观察 + 行业维度指引"喂给 consolidator LLM；输出经 非原子检测(至多重试1次)→丢弃兜底 → typed 解析+Plain 升级 → `compact_memory_card_with_dimensions`（prev 合并 + discarded 黑名单 + dimension 感知救回 + core cap6/recent cap10/deprecated cap20 + 淘汰审计 cap20 + extra 维度 cap）→ `apply_consolidator_deprecations`（显式弃用）→ `deprecate_same_dimension_conflicts`（机制兜底裁决）；同一份 LLM 输出还搭车产出 `parse_reconfirmed_tags`（证据锚 fail-closed）+ `merge_confirmed_tags`（未显式弃用即保留）与大五人格（无证据维 confidence=0，快照 cap 50）。
6. **两阶段提交**：生产路径把全部产物打进 `prepared_commit` 并经 `prepare_task_commit_if_owned` 做 running→committing CAS（admin cancel 与提交在此线性化），然后 `reconcile_memory_consolidation_commit`（memory.rs:2400-2648）可反复重放：OCC 写 operating_memories（prev_version filter + `memory_applied_commits` marker 幂等）→ contact 投影（confirmed_tags/personality 按 `memory_projection_version` 单调 filter）→ 候选 pending→**consolidated**（只从 pending 转移）→ warnings/dedupe 事件 → task committing→sent（或 rerun_requested → retry 再跑一轮）→ $pull marker。OCC 输掉即 `requeue_task_commit_if_owned("memory_card_occ_conflict")`，候选留 pending 由下轮重算。
7. **反应回路（与 2-6 并行）**：下一条入站消息进 `record_user_reaction`（reaction.rs:37-79）——确定性 stop 短路（100 年 cooldown + outbox 取消）或 claim 最新 sent review（analyzing + token + generation）→ LLM/确定性分析 → outcome 经 token CAS 写回（stale 结果丢弃）→ `push_intent_trajectory_entry` 把 outcome+维度追加进 `contact.intent_trajectory`（$slice:-50）→ reviewer 误判信号驱动负例 chunk（needs_review 待复核）→ stop 类 outcome 级联取消 outbox。轨迹与 reaction hint（`format_reaction_hint`/`format_intent_trajectory_hint`）在下一轮 reply prompt 中回注，形成学习闭环。
8. **读取回注**：下一轮决策通过 `effective_memory_card_for_contact`（memory.rs:119-136）拿 cap 后卡（无信号回落 contact 种子卡），operator 偏好记忆经 `load_operator_memory`（bump last_used_at 续期）+ `format_operator_memory_for_reply_prompt` 注入。所有 LLM 调用被 task-local `RUN_BUDGET` 记账（budget.rs:302-307），run 全程由 `run_envelope`（started→running→终态）+ `run_audit`（LLM 日志/ptier 事件缓冲、阶段计时）留痕。

## 4. 事实卡速查

**memoryCard cap**（memory.rs:387-587）
| 项 | cap | 出处 |
| --- | --- | --- |
| core_facts（typed） | 6 | memory.rs:440-444 |
| recent_facts（typed） | 10 | memory.rs:529 |
| deprecated_facts（typed） | 20 | memory.rs:531 / 1028-1031 / 812 |
| extra.coreFactEvictions 审计 | 20 | memory.rs:511 |
| extra 镜像 coreFacts / recentFacts / deprecatedFacts | 6 / 10 / **6** | memory.rs:580-582 |
| DEFAULT 八维业务槽 | preferences 8、doNotDo 10、commitments 8、objections 8、openLoops 8、openQuestions 8、confirmedFacts 12、conflicts 6 | domain_profile.rs:86-98 |
| 种子卡 core_facts | ≤6 | memory.rs:226-236 |
| personality snapshots | 50（FIFO） | memory.rs:1562 |
| consolidator 单轮候选 | 30 条 pending | memory.rs:1643 |

**core 排序权威分**（memory.rs:589-617）：operator_manual=100 > confirmed_tag=20 > LLM（1 + msg_ids 8 + evidence 4 + run_id 2，最高 15）；Plain=0。

**consolidation 触发全集**（memory.rs:2848-2864）：consolidation_needed=true ∨ memory_write_score≥6 ∨ pending≥4（2843）∨ 最老 pending≥6h（2844）。候选分流（2833-2841）：write_score≥6 ∨ max_importance≥8 → pending，否则 ignored_low_score；弱证据（<4 字符）importance 封顶 7（2919-2928）。

**候选生命周期**：pending →（consolidator 赢 OCC/commit）→ consolidated（memory.rs:2156/2540-2549）；或建档即 ignored_low_score（终态）。OCC 输 → 留 pending 重跑。

**非原子判据**（memory.rs:701-714）：换行≥2 ∨ 句界（。！？;；）≥2 ∨ >80 chars；重试至多 1 次。

**reaction outcome_status 闭集**（推导层，reaction.rs:678-701）：显式 passthrough 任意域词 / `user_replied_stop_requested` / `polarity.positive[0]`（默认 `user_replied_buying_signal`）/ `user_replied_objection` / `user_replied_unclassified`。DEFAULT 负极 5 词（838-844）：objection / stop_requested / unsubscribed / negative / complaint；DEFAULT 正极 1 词（gap_signals.rs:778）。misjudge 信号：`approved_but_user_negative`（820-833）。

**reaction 时序参数**：claim 超时（stuck 重置阈值）`reaction_analysis_claim_timeout_seconds` 默认 60s（config.rs:534-536）；确定性 stop cooldown = 100 年（reaction.rs:251-254）；stop 词表长度上限 96 字符、buying 上限 120 字符（reaction.rs:97/183）；intent_trajectory 滑窗 50（models.rs:5494）；trajectory hint 取最近 5（reaction.rs:1214）；reaction hint 取最近 3（reaction.rs:1030）。

**run_envelope lifecycle 闭集**（run_envelope.rs:48-54）：started / running / completed / failed_before_decision / failed_after_decision / aborted_by_budget / aborted_by_external_signal；终态吸收（296-355）。finalReviewStatus 闭集 10 值（68-79）；gateway_status 闭集 40 值（87-147）；禁词 5 个（152-158）。外部信号吸收态 4 个：superseded_by_new_inbound / user_reaction_stop_requested / quiet_hours_deferred / stale_task_claim（274-277）。

**budget 默认**（runtime.rs:946-951 + models.rs defaults）：run_token_budget=150000、run_token_budget_escalated=500000、run_max_llm_calls=6、simulation_token_budget=300000、reaction_token_budget=8000、reaction_max_llm_calls=2、knowledge_max_tool_calls=6（clamp [1,16]）。consolidation 窗口默认 6000 字 / 60 条（runtime.rs:964-965，clamp [1000,16000] / [10,200]）。

**post_decision 投影**：LLM `max_llm_calls=1`、token 预算 `POST_DECISION_TOKEN_BUDGET` 默认 32000（post_decision.rs:448-453 + config.rs:520-522）；prompt 字符上限默认 80000（config.rs:517-519）；快照上限默认 2MiB（config.rs:514-516）；max_attempts 默认 8（config.rs:511-513）；worker 并发默认 4（config.rs:508-510）；lease = llm_timeout×retries + retry_base×(retries−1) + 60s，下限 120s（post_decision.rs:810-823）；心跳续租 20s（468）；退避 1s×2^n 封顶 5min（1261-1262）；扫描 limit 32（730）；快照消息 20 条/4000 字符、产品 100 条（25-27）；terminal 快照保留默认 14 天后脱敏（config.rs:523-528）、scrubber 每 1h（29）。

**其它 TTL/上限**：temporal 档期证据 48h（prompt_isolation.rs:30）；prompt 历史预算 800/4000/4000(12 条)/2000(6 条)（prompt_isolation.rs:34-39）；temporal 候选历史 12 条（prompt_isolation.rs:326）；projection_observations 显示缓存 32 个 run（projection_observations.rs:15）；system_incident outbox max_attempts=5（system_incident.rs:451）、reconcile 批 100（532）；negative_example title 截 30 字符（reaction.rs:889-891）、deprecation reason 截 200（memory.rs:958-961）、operator memory revoke reason ≤200（memory.rs:3439-3443）；run_audit 缓冲事件仅 7 种 ptier_*（run_audit.rs:329-337）；fail_run_envelope summary 截 1024（run_envelope.rs:621）。

## 5. 偏差与疑点

1. **`update_run_envelope_terminal` 不做 lifecycle 转换 CAS**（run_envelope.rs:690-693）：filter 只有 `{run_id}`，`is_valid_lifecycle_transition`（296-355）虽为纯函数存在，但该写路径只做**枚举**校验（663-667），不校验 from→to 合法性——理论上一次迟到的 update 可把 completed 覆写为 failed_after_decision。终态吸收在库层只由 `fail_run_envelope_if_open`（629 filter 带 lifecycle）与 `mark_run_envelope_running`（450）保证；`update_run_envelope_terminal` 的吸收性依赖调用方"每 run 只 finalize 一次"的纪律。判定：设计权衡（文档注释未明说），非 bug 证据，标疑点。
2. **extra 镜像 deprecatedFacts cap=6 与 typed cap=20 不一致**（memory.rs:580-582 vs 531）：注释自称"typed 三数组固定 cap 6/10/20 的 wire 兼容形态"（575-579），但镜像键给 deprecatedFacts 的 cap 是 6 而非 20。老数据若在 `extra.deprecatedFacts` 残留 >6 条会被截到 6，而 typed 层保 20。行为是显式写死的（非笔误的可能性高，H17 注释称"历史镜像 cap 保持写死"），但 6≠20 的语义差异未见解释——标疑点。
3. **`apply_append_only_projection` 用默认 runtime**（post_decision.rs:934）：stale 降级路径 `UserRuntimeParameters::from_config(None, state)`，不用快照里的 domain_config/active_profile runtime（主路径 1043-1044 用）。对 append-only 写入影响有限（这些函数基本不消费 runtime 阈值），但与主路径不对称——标疑点（低风险）。
4. **`activate_projection` 与 `runnable_filter` 的 prepared 语义有冗余**：`runnable_filter`（post_decision.rs:619-641）本身就接受 `post_decision_status="prepared"`（只要 review.status 已 ∈ [outbox_enqueued,sent,no_reply] 且 next_retry_at 缺失即 runnable），所以 prepared 行不经 activate 也能被 worker 认领；`discard_projection`（603-617）只匹配 prepared——若 worker 已把 prepared 抢成 processing，discard 会 miss。窗口极窄（review.status 翻到三态之一 = 发送已授权，此时按语义本就该投影），但 activate 的"必要性"与 discard 的竞态语义值得在 spec 里核对——标疑点。
5. **`strip_known_tags` 大小写敏感且只剥固定 8 子串**（prompt_isolation.rs:487-496）：`<SYSTEM>`、`<System>` 等大小写变体和其它 tag 形态不会被剥。模块注释（14-16）明说不做关键词黑名单、由模型策略层兜底，属既定取舍——记录为已知边界而非缺陷。
6. **reaction turn_index 是全量 inbound count**（reaction.rs:1091-1104）：与证据序位的"窗口 0-based 下标"不是同一坐标系（后者在 tag_evidence 里）；注释自认 best-effort，intent_trajectory 仅观测——已知偏差，无一致性风险，但阅读时易混淆两种"turn"。
7. **`shadow_dependency_fingerprint` 全量段扫描无 limit**（prompt_shadow.rs:749-766）：19 个集合按 tenant filter 拉全部行做 SHA256。大 workspace（如 conversation_messages、operation_knowledge_chunks）下每个 sample 要跑 3 次指纹，代价可观。正确性无虞（宁可 fail `shadow_dependencies_changed`），性能标注。
8. **`deprecate_same_dimension_conflicts` 的 warning 泄漏索引而非 id**（memory.rs:793）：`same_dimension_conflict_deprecated:{dim}:idx{i}` 用的是移除前的数组下标，事后从审计里无法稳定回指具体 fact（audit events 另有 id，可交叉）。可用性小瑕疵。
9. **`explicit_stop_intent` 的否定标记先于 DIRECT 判定**（reaction.rs:101-115）：包含"比如"的真实指令（如"比如别再联系我——不，我认真的，退订"）会被放回 LLM 路径。fail-open 到模型侧是刻意保守（高精度地板），记录为语义边界。
10. **`consolidate_contact_memory_inner` 无 claim 路径的候选/事件非事务**（memory.rs:2130-2200）：OCC 赢后 contact 投影 fail-soft、候选置 consolidated 与事件写入无原子性；崩在中间会出现"卡已更新但候选仍 pending"（下轮会重复喂给 LLM，由 compact 去重吸收）。生产路径已由 prepared-commit 协议闭合，此兼容路径的弱一致是已接受的取舍（注释 2053 明说不参与取消协议）。
11. **`memory_card_has_signal` 对 `extra.coreFacts` 历史镜像也算信号**（memory.rs:152-170）：一张 typed 全空但 extra 残留镜像的卡不会被种子卡覆盖——正确（防覆盖真数据），但意味着彻底清空记忆需同时清 extra 镜像。
12. **post_decision `runnable_filter` 对 processing 的 lease 抢占只看 `post_decision_locked_until`**（633-637）：与 contact lease（独立集合）双层锁并存，review 层锁过期但 contact lease 未过期时，新 worker 会先在 `acquire_contact_lease` 被挡（claim_token 不同且 locked_until 未到）→ 候选被 defer，行为自洽。

## 6. 覆盖自证

| # | 文件 | 行数 | 读取方式 |
| --- | --- | --- | --- |
| 1 | `src/agent/memory.rs` | 5056 | 6 段全文（1-900 / 901-1800 / 1801-2700 / 2701-3600 / 3601-4399 / 4400-5056） |
| 2 | `src/agent/reaction.rs` | 1798 | 2 段全文（1-950 / 951-1798） |
| 3 | `src/agent/sufficiency.rs` | 342 | 1 次全文 |
| 4 | `src/agent/post_decision.rs` | 1435 | 2 段全文（1-750 / 751-1435） |
| 5 | `src/agent/projection_observations.rs` | 138 | 1 次全文 |
| 6 | `src/agent/consolidation_window.rs` | 77 | 1 次全文 |
| 7 | `src/agent/run_envelope.rs` | 2059 | 3 段全文（1-750 / 751-1509 / 1510-2059） |
| 8 | `src/agent/run_audit.rs` | 538 | 1 次全文 |
| 9 | `src/agent/budget.rs` | 607 | 1 次全文 |
| 10 | `src/agent/simulation.rs` | 312 | 1 次全文 |
| 11 | `src/agent/prompt_shadow.rs` | 788 | 1 次全文 |
| 12 | `src/agent/shadow_finalize.rs` | 196 | 1 次全文 |
| 13 | `src/agent/prompt_isolation.rs` | 763 | 1 次全文 |
| 14 | `src/agent/system_incident.rs` | 1202 | 2 段全文（1-650 / 651-1202） |
| 15 | `src/agent/tag_evidence.rs` | 119 | 1 次全文 |
| 16 | `src/agent/multimodal.rs` | 167 | 1 次全文 |

合计 15597 行，`wc -l` 核对一致。跨文件引用另经 Grep 亲验：`IntentTrajectoryEntry::MAX_ITEMS=50`（models.rs:5494）、`PRINCIPAL_RELAY_SENTINEL`（models.rs:841）、`MemoryCardTyped::{to_document,from_document,auto_upgrade_plain_facts,live_dimension_names}`（models.rs:5270-5334）、`default_memory_dimensions` 八槽 cap（domain_profile.rs:86-98）、`default_outcome_polarity`（domain_profile.rs:160-172）、`DEFAULT_POSITIVE_OUTCOMES`（gap_signals.rs:778）、post_decision/reaction 配置默认（config.rs:508-536）、runtime 预算默认（runtime.rs:946-967）。
