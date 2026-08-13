# 知识路由与 workers 深读记录（核证日期 2026-08-13）

> 覆盖范围：`src/routes/knowledge/` 全部 10 个文件（16,724 行）、`src/knowledge_task/mod.rs`（1,765 行）、`src/knowledge_digest/`（mod.rs 1,853 行 + labels.rs 35 行）、`src/import_worker.rs`（485 行），合计 20,862 行，全部逐行读完。所有断言均附 `file:line`（行号为核证日当天工作区状态）。路由挂载路径见 `src/routes/mod.rs:480-725`（本记录第 4 节引用）。

---

## 1. 模块地图（每个文件的端点/职责清单）

### src/routes/knowledge/mod.rs（2411 行）— 子域装配 + 共享 DTO/工具函数
- 模块声明与 re-export（`mod.rs:28-53`）：catalog / crud / import / repair / verify / chat（pub，集成测试直调 apply_create_chunk）/ digest_inbox / sources_meta / wiki_edit。
- 共享请求 DTO：`OperationKnowledgeDocumentRequest`（88-120）、`OperationKnowledgeChunkRequest`（228-266）、`DocumentMetadataPatch<T>` 三态 Missing/Null/Value（122-143）、文档 PATCH 请求（147-165）、旧 items 请求（170-224，dead_code 兼容占位）。
- 三大 JSON 投影（前后端契约唯一真相源）：`operation_knowledge_document_json`（283-304）、`operation_knowledge_chunk_json`（306-373，33 键 camelCase，source_anchors/usage_stats/provenance 特殊桥接）、`knowledge_usage_json`（375-398）。
- 导入 preview 归一化族：`normalize_operation_knowledge_preview_item/document/chunk`（758-793 / 882-911 / 939-972）、`default_operation_knowledge_preview_document`（913-937）、确定性风险词提取 `deterministic_import_risk_notes`（801-867）。
- 锚定与完整性内核：`stable_text_hash` FNV-1a（1006-1013）、`build_line_index/build_section_index`（1015-1050）、`source_anchor_for_quote` + 模糊定位（1052-1137）、`integrity_report_for_preview`（1156-1201，红线：preview 恒 0 verified）、`apply_chunk_integrity`（1203-1233）。
- D2 verify 闸：`chunk_verify_gate_reason`（1324-1339）+ 唯一生产入口 `chunk_verify_gate_reason_for`（1351-1358）。
- 受控 patch：`normalize_editable_chunk_patch`（453-540，11 个可编辑字段白名单）、`apply_controlled_chunk_patch`（545-598）。
- 预算常量：REPAIR（1390-1392）、CHAT（1490-1495）。
- 共享审计写入：`record_knowledge_run_started`（1410-1442，fail-closed）、`record_repair_event`（1444-1471，fail-soft）。

### src/routes/knowledge/crud.rs（1011 行）— 文档/切片基础 CRUD + 审核队列
| Handler | 语义 |
|---|---|
| `list_operation_knowledge`（27-34） | 旧 items 列表，恒返空（兼容形状） |
| `list_operation_knowledge_documents`（36-73） | 文档列表，limit 200 |
| `create/get/update/patch/delete_operation_knowledge_document`（75-413） | 文档 CRUD；PUT/PATCH 都带 version OCC；DELETE=事务级联归档 |
| `list_operation_knowledge_chunks`（517-525）/`list_operation_knowledge_document_chunks`（680-698） | 切片列表（limit 300 于 mod.rs:1269） |
| `list_operation_knowledge_review_queue`（564-678） | 5 类审核分类聚合 + 维度过滤 |
| `create_operation_knowledge_chunk`（700-780） | 事务新建，强制 draft+needs_review |
| `update_operation_knowledge_chunk`（782-835） | 旧 PUT 收窄为受控 patch |
| `delete_operation_knowledge_chunk`（837-889） | 软归档（Archive revision） |
| `get_operation_knowledge_chunk_source`（891-928）/`get_operation_knowledge_chunk`（931-950） | 详情（含父文档原文）/单条 |
| `create/update/delete_operation_knowledge`（952-982） | 旧 items 写端点恒 400 |

### src/routes/knowledge/verify.rs（856 行）— 单条 verify/reject + 批量 auto-verify
- `verify_operation_knowledge_chunk`（151-173）→ 内核 `verify_chunk_at_version`（60-133）：事务 + 版本绑定 + D2 证据闸。
- `reject_operation_knowledge_chunk`（175-200）。
- `auto_verify_operation_knowledge_chunks`（208-254）+ inner（313-643）：批量 LLM 预审分诊，**全类型强制 needs_human_audit**。
- 纯函数：`clamp_sample_rate`（39-43）、`decide_auto_verify_status`（654-672）、`enforce_verified_needs_human_audit`（681-686）。

### src/routes/knowledge/import.rs（3013 行）— 导入全链路
- 同步/异步 preview：`import_operation_knowledge_preview`（507-609）；job 轮询 `get_import_preview_job`（613-634）、`list_import_preview_jobs`（638-661）。
- 抽取内核：`run_import_extraction`（691-698）→ `run_import_extraction_controlled`（880-1084）；分块 `split_import_content`（325-367）；段级 checkpoint（732-878）；LLM 抽取 prompt `LONG_IMPORT_PROMPT_TEMPLATE`（124-192）。
- 落库：`import_operation_knowledge_apply`（1223-1279）→ `import_apply_in_transaction`（1396-1633）。
- 封印：`seal_import_preview_result`（90-118）、`import_preview_hash`（77-84）。
- 多模态：`import_operation_knowledge_apply_pdf`（1651-1704）+ `import_pdf_bytes`（1709-1730）；`import_operation_knowledge_apply_image`（2042-2103）+ 视觉模型选择/调用（1752-2040）。
- 幂等 ingest：`ingest_chunked_text`（2548-2587）/`ingest_chunked_text_with_session`（2530-2543）+ `prepare_ingest`（2156-2265）+ `enforce_ingest_server_owned_fields`（2267-2286）。
- 标签抽取：`extract_knowledge_tags_inner`（1142-1197）+ HTTP `extract_operation_knowledge_tags`（1201-1221）。

### src/routes/knowledge/repair.rs（1022 行）— AI 自主修复三段式
- `propose_chunk_repair`（417-451）+ 可复用内核 `propose_chunk_repair_inner`（231-415，红线：绝不改 chunk）。
- `answer_chunk_repair`（453-627，followup 轮，≤3 轮）。
- `record_repair_apply`（666-845，唯一落库端点，source=Ai → draft+needs_review）。
- `parse_repair_response`（90-167，LLM 输出规整）。

### src/routes/knowledge/chat.rs（4132 行）— 对话补库 + 派工 + SSE
- 单轮：`chat_turn`（151-507）→ `run_chat_turn_pipeline`（1496-1670）→ intent 分类（2174-2226）→ 六分支（draft/update/digest_action/operator-memory×2/clarify）。
- 会话：`chat_history`（523-571）、`chat_apply`（579-631）+ 事务内核（700-914）、`chat_discard`（945-989）。
- tool loop：`run_chat_with_tools`（1783-1955）+ 协议增广（1751-1768）。
- 落库内核（chat/task 共用）：`apply_create_chunk(_with_session)`（2704-2808）、`apply_update_chunk(_with_session)`（2810-2968）、D2 锚定纯函数 `resolve_quote_anchors`（2678-2702）。
- 派工：`dispatch_digest_action_for_chat`（2504-2659）、`resolve_digest_selection`（3086-3176）、`chat_task_create`（3219-3408）、`chat_task_list/get/cancel`（3427-3601）、SSE `chat_session_stream`（3609-3662）。

### src/routes/knowledge/catalog.rs（1006 行）— 目录/完整度/检索
- `get_operation_knowledge_catalog`（45-56）+ `build_operation_knowledge_catalog`（316-392）。
- `get_operation_knowledge_catalog_persisted`（64-109，读 worker 持久化快照）。
- `get/refresh_operation_knowledge_completeness`（111-157，进程内 TTL 缓存）+ `build_operation_knowledge_completeness`（497-804，LLM 审计）。
- `get_operation_knowledge_integrity_report`（159-171）+ builder（177-235）。
- 工具端点：`search_operation_knowledge_tool`（237-259）、`open_operation_knowledge_slices`（261-290）、`test_operation_knowledge_match`（292-314）。
- 确定性护栏纯函数：`clamp_answering_mode`（406-412）、`merge_completeness_gaps`（419-432）、coverage 骨架渲染（439-495）。

### src/routes/knowledge/wiki_edit.rs（1085 行）— wiki 编辑 8 路由 + 批量
patch（76-101）/ archive（106-129）/ restore（132-155）/ rollback（161-202）/ revisions 分页（214-274）/ split（290-452）/ merge（463-645）/ relate+unrelate（681-836）/ referrers 反查（849-895）/ batch-verify（916-965）/ batch-archive（977-1014）。全部走 `knowledge_wiki::chunk_revisions` harness。

### src/routes/knowledge/digest_inbox.rs（818 行）— 日报 HTTP 面 + AI Inbox
- `digest_today`（33-75，未命中同步合成）、`digest_regenerate`（86-122）、`digest_dismiss_card`（145-187）。
- `serialize_digest_report`（189-233，带 cardHash/reportHash）。
- `knowledge_inbox`（366-572，四类信号只读聚合）。

### src/routes/knowledge/sources_meta.rs（1370 行）— 问答/信号/摄取源/记忆/统计
- usage 列表（23-50）、`analyze_operation_knowledge_logs`（68-152）、`knowledge_aggregate_metadata`（163-329，单次 $facet×2）。
- gap-signals：list/dismiss/apply/sweep（334-486）。
- `ask_knowledge`（522-577）+ SSE 流式版（619-761）、`knowledge_metrics`（767-774）。
- operator-memory：list（800-894）、revoke（896-949）。
- ingest sources CRUD：list/create/update/delete（1022-1196）+ SSRF 校验（990-995）。

### src/knowledge_task/mod.rs（1765 行）— chat 长任务 worker
`ChatProgressBus`（44-143，SSE watch 总线 + per-session 锁 + 延迟清理）、claim/lease/heartbeat（166-297）、step intent 两阶段提交（347-736）、主循环 `worker_loop`/`tick_once`/`run_claimed_task`（147-164 / 800-813 / 831-1162）、`execute_step`（1202-1585）、原子 turn_index（1591-1621）、progress/summary turn 写入（1623-1715）。

### src/knowledge_digest/（1853 + 35 行）— 日报合成 worker
快照哈希（36-80）、`worker_loop`（88-108）、4 路只读分析（234-572）、`compose_cards`（575-666）、卡片解析/排序/稳定 id（676-925）、attempt claim/finalize 世代栅栏（973-1153）、`do_generate` 主流程（1261-1446）；labels.rs：拦截码中文映射（5-14）。

### src/import_worker.rs（485 行）— 异步导入 worker
`ImportJobClaim` 所有权指纹（30-58）、fencing 写 `update_owned_import_job`（62-72）、`run_import_worker`/tick（106-125）、孤儿回收 `reclaim_stale_running_jobs`（132-194）、`claim_one`（198-224）、`run_job`（228-394）、终态 CAS `finish_owned_import_job`（396-443）、心跳（448-485）。

---

## 2. 逐文件深读

### 2.1 mod.rs — 共享内核

**投影层（前后端契约）**
- `operation_knowledge_chunk_json`（mod.rs:306-373）：`source_anchors` 是 `Vec<bson::Document>`，必须走 `.into_relaxed_extjson()` 桥接，否则前端会收到 `{"$numberInt":"42"}`（mod.rs:307-315）。`usage_stats` 手工映射 hitCount30d/blockedCount30d（319-324）；`provenance` 逐字段映射 camelCase（328-337）。契约由 fixture 测试锁死（1837-1905，33 键；`created_at`/`integrity_score`/`domain_attributes` **不下发**，见 1834-1835 注释）。
- `knowledge_usage_json`（375-398）：route_result/tool_trace 同样走 relaxed extjson。

**标签与校验**
- `normalize_knowledge_tags`（404-429）：trim → 可选 lowercase → 保序去重 → max_len 截断。product_tags 上限 5、business_topics 上限 3（document_from_request 647-648、chunk_from_request 700-701）。
- `validate_operation_knowledge_document/chunk`（431-447）：唯一校验是 title 非空。
- `normalize_operation_domain`（1292-1304）：白名单 `user_operations`/`group_operations`/`moments_operations`，其余（含 LLM 输出的"私域运营"、大小写不符）强制归一 `user_operations`，防"孤儿知识"（1284-1291 注释）。

**受控编辑 patch**
- `normalize_editable_chunk_patch`（453-540）：白名单 11 字段（title/summary/body/knowledgeType/businessContext/applicableScenes/notApplicableScenes/productTags/businessTopics/sourceQuote/priority），camelCase 与 snake_case 双别名收敛，重复别名报 400（516-520）；title 必非空字符串（478-485）、数组字段元素必须全 string（493-503）、priority 必须 i32（504-513）；scope/lifecycle/review/provenance 字段全部拒绝（459-476）。
- `apply_controlled_chunk_patch`（545-598）：patch 含 `source_quote` 时读原 chunk → 读父 document.raw_content → `source_anchor_for_quote` 重算 `source_anchors` 一并写入（555-584）——保证"quote 改了 anchor 必跟上"；随后走 `apply_chunk_revision`（op=Patch）。

**导入风险词确定性下限**
- `deterministic_import_risk_notes`（801-867）：17 个绝对承诺 marker（"保证学会/包教包会/全市第一/无条件退款/百分百有效/稳赚不赔/包治百病"等 802-820），12 个否定前缀豁免（"不/不能/无法/不提供/未承诺"等 821-834）；命中行去重后生成"原文含需人工核验的绝对承诺：{line}"。`merge_import_risk_notes`（869-880）把 LLM riskNotes 与确定性下限求并集——LLM 漏报不会丢。

**锚定内核**
- `source_anchor_for_quote`（1052-1089）：先精确 `find()`，失败走 `fuzzy_locate_quote`（1094-1137，压缩空白后子串匹配再回推 byte offset）；命中产 anchor 文档：startOffset/endOffset/startLine/endLine/sourceQuote/quoteHash（+documentId）。
- `apply_chunk_integrity`（1203-1233）：触发重建锚点的条件是"无**可引用** anchor"（`chunk_has_citable_anchor`，B3 修复，1209-1216）而非"数组为空"；无锚点时写 distortion_risks 提示；**恒置 `integrity_status=Some("needs_review")`**（1230）、confidence 90（有锚）/45（无锚）（1231）。注释明示"Anchoring establishes provenance location only"（1221-1222）。
- `integrity_report_for_preview`（1156-1201）：preview 恒 `verified=0`（1158）；anchor 命中只保留 anchors+confidence 90 作审计线索，`integrityStatus` 恒 `needs_review`（1176-1180）。

**D2 verify 闸**
- `chunk_verify_gate_reason`（1324-1339）：quote 与 anchor 双非空才放行，缺失项列入拒绝文案。
- `chunk_verify_gate_reason_for`（1351-1358）：**唯一生产入口**，内部用 `chunk_has_citable_anchor` 算谓词，杜绝调用方用裸 `!is_empty()` 传错（1341-1350 注释解释 B3 潜伏结构原因）。

**预算常量**
- 修复：`REPAIR_TOKEN_BUDGET_PER_TURN=4_000`、`REPAIR_MAX_LLM_CALLS_PER_TURN=4`、`REPAIR_MAX_TURNS=3`（1390-1392）。
- chat：`CHAT_MAX_LLM_CALLS_PER_TURN=4`、`CHAT_TOKEN_BUDGET_PER_LLM_CALL=6_000`、`CHAT_TOKEN_BUDGET_PER_TURN=24_000`、`CHAT_MAX_TURNS_PER_SESSION=8`、`CHAT_MAX_FOLLOWUPS=3`（1490-1495）。

**审计写入**
- `record_knowledge_run_started`（1410-1442）：LLM 调用前先写 `AgentEvent kind="knowledge_run_started"`，**fail-closed**（`?` 传播错误）——保证无 contact 的失败 run 可归因清理。
- `record_repair_event`（1444-1471）：`let _ =` fail-soft。

### 2.2 crud.rs — 基础 CRUD

- `list_operation_knowledge_documents`（36-73）：filter=workspace+domain=user_operations；account_id 存在时 `$or:[{account_id:null},{account_id}]`（共享+私有）；可选 status；sort updated_at desc limit 200。鉴权：`Extension<AuthenticatedAdmin>`（全文件所有 handler 一致，走 session 注入 workspace）。
- `update_operation_knowledge_document`（122-194）：`version` 必传（130-132）；先读校验 version（145-149），再 `update_one` filter 带 version 做二次 CAS（162-192）；**PUT 不再整条替换**，只 `$set` 运营可编辑元数据（source_name/title/summary/catalog_summary/routing_map/risk_notes/product_tags/business_topics），tenant/raw_content/index/lifecycle/catalog 状态全部服务端持有（158-161 注释）；updated_at 走 `monotonic_chunk_updated_at`（153-156）。
- `patch_operation_knowledge_document`（210-372）：`DocumentMetadataPatch` 三态语义——Missing 不动、Null 清空（title 除外，253 拒绝）、Value 归一化后 diff 才写；数组字段 max_len：routing_map/risk_notes 50、product_tags 5、business_topics 3（288-313）；无变化返回 `unchanged:true` 不 bump version（330-336）。
- `delete_operation_knowledge_document`（374-413）→ `archive_document_with_chunks`（415-515）：事务内先收集全部非 archived 子 chunk id（435-455，session cursor 必须先耗尽再写），逐个 `apply_chunk_revision_with_session(op=Archive, source=Human)`（458-474），最后文档 status→archived + version+1（476-513）；提交后逐 chunk 广播 WebSocket `chunk_revised`（398-406）。
- `list_operation_knowledge_review_queue`（564-678）：加载 active DomainProfile（569-570）；dimension 参数校验（571-598，未知维度 400 `unknown_knowledge_review_dimension`；别名集=key+display_name+review_topic_aliases 规范化）；扫描全部非 archived chunk（600-617），仅保留 draft/active（631-634）；分类 `review_categories_for_chunk`（531-562）：`contested`（rejected）、`needs_review`、`source_orphan`（缺 quote 或缺 anchor——注意 547 用裸 `!source_anchors.is_empty()`，见第 5 节疑点 2）、`pending_verification`（有源但 needs_review）、`dependents_pending`（related 指向不存在 chunk）；返回 counts+effectiveFilter。
- `create_operation_knowledge_chunk`（700-780）：**红线落点**——无条件 `payload.status="draft"`、`integrity_status=Some("needs_review")`、`confidence_score=Some(0)`（708-710，注释："The dedicated /verify route is the only way into active+verified"）。事务内：父 document 必须存在且非 archived（716-733）→ insert → `apply_chunk_revision_with_session(op=Create, source=Human)`（742-755）→ commit → 广播。
- `update_operation_knowledge_chunk`（782-835）：兼容 URL 但语义收窄——只投影可编辑内容字段进 patch（793-818，空数组/priority=0 不写入），走 `apply_controlled_chunk_patch(source=Human)`；响应硬编码 `"status":"draft","integrityStatus":"needs_review"`（832-833，与 harness 对 Human patch 的真实行为是否一致见疑点 1）。
- `delete_operation_knowledge_chunk`（837-889）：已 archived 幂等返回（855-862）；否则 op=Archive revision + 广播。

### 2.3 verify.rs — 核验（事务 + 版本绑定 + D2 + auto-verify 降级）

- `KnowledgeVerifyRequest`（45-51）：`expected_updated_at` 必传，管理员看到的版本令牌；RFC3339 解析（53-56）。
- `verify_chunk_at_version`（60-133）——verify 的事务内核，被单条 verify 与 batch-verify 共用：
  1. 事务内读 chunk（73-82）；
  2. **版本绑定**：`chunk.updated_at != expected_updated_at`（毫秒级）→ `Conflict("chunk_revision_conflict")`（83-85）——阻止"管理员看 A、实际批准 B"；
  3. **D2 证据闸**：`chunk_verify_gate_reason_for(source_quote, source_anchors)`（92-97）——同一事务快照内检查，阻止"闸后证据被并发清空"；
  4. `apply_chunk_revision_with_session(op=Verify, source=Human)`，patch：`integrity_status="verified"`、`confidence_score=100`、`verified_claims`、`unsupported_claims=[]`、**`status="active"`**（99-118）——这是全系统唯一进入 active+verified 的写点；
  5. 失败 abort + `map_chunk_transaction_error`（122-131）。
- `reject_operation_knowledge_chunk`（175-200）：op=Reject/source=Human，patch integrity=rejected/confidence=0/status=rejected。
- **auto-verify**（208-643）：
  - 参数：threshold 默认 7 clamp[0,10]（217）；`sample_rate=clamp_sample_rate(...)`（223）——默认 0.3，**硬下限 0.05**（32-43，传 0 也不许 100% 无人审）；limit 默认 50 clamp[1,500]（224）。
  - 预算独立于 user-ops：`autoVerifyTokenBudget` 默认 240_000、`autoVerifyMaxLlmCalls` 默认 100（256-277，R15/ISSUE-009 防被 runMaxLlmCalls=6 掐死）；`RunBudget` tool 维度 i32::MAX（229-237）。
  - 候选：`integrity_status ∈ {needs_review, null}`，共享+本账号，updated_at desc limit N（323-342）；非空即写 `knowledge_run_started`（347-357）。
  - prompt：system 从 `knowledge.auto_verify` PromptSpec 加载，fallback 字面量强调"只有 sourceQuote 非空且 sourceAnchors 可定位来源时，才允许 verified"（359-367）；user 含切片全文与 anchors JSON，要求输出 `{confidenceScore, integrityStatus, verifiedClaims, distortionRisks}`（399-421）。
  - 判定链：`decide_auto_verify_status(has_quote, has_anchor, confidence, threshold, model_status)`（480-486，654-672）——verified 需四条件齐；rejected 采纳 LLM 明示；其余 needs_review。抽样命中改 needs_human_audit（487-489）。**最后 `enforce_verified_needs_human_audit`（496，681-686）把所有 verified 强制降级 needs_human_audit——auto-verify 对全部 chunk_type 都不直 verified，退化为"预审分诊"**（490-495 注释，红线"AI 永不自动 verify"）。
  - 落库：`apply_chunk_revision(op=Verify, **source=Rule**, actor="auto_verify")`（505-524，如实标注规则化批处理而非人审）；写失败计 failed 不计业务裁决（527-537）。每条写 `knowledge_usage_logs`（kind=knowledge_auto_verify，547-584）。
  - 全部 LLM 失败且 0 处理→上抛首个结构化错误（299-311, 587-594）；budget 断路 fail-soft 标 degraded（383-391）；收尾事件 `knowledge_auto_verify_done`（596-629）。

### 2.4 import.rs — 导入全链路

**preview 同步/异步分界**（`import_operation_knowledge_preview`，507-609）
- 校验 `validate_import_content`（421-442）：非空、≤200_000 chars、分段 ≤64（IMPORT_MAX_TOTAL_CHARS/IMPORT_MAX_SEGMENTS，204-205）；accountId 存在则 `validate_account`。
- **content ≤3000 chars（IMPORT_SINGLE_CALL_MAX_CHARS，195）→ 同步**：`run_import_extraction` → `seal_import_preview_result` → 直接 insert 一条 **completed** 的 ImportJob（status="completed"、progress 1/1、`preview_hash`、`apply_status="ready"`、`result`、`expires_at=now+24h`，517-562）→ 返回 preview JSON。
- **>3000 chars → 异步**：insert `status="pending"` 的 ImportJob（segments_total、owner_admin_id，564-598）→ 返回 `{jobId, async:true, segmentsTotal}`（604-608），worker 接手。
- job 查询（613-661）：单查/列表都带 `workspace_id + owner_admin_id` 双过滤（IDOR 收口）；列表默认 status=running、limit 50。

**分块策略**（`split_import_content`，325-367）
1. ≤3000 chars 单段原样（零回归）；2. markdown 标题行切原子块（332-340）；3. 贪心打包至 TARGET=3000，单块 >HARD_MAX=5000 先 flush 再按段落窗口切（344-362，`split_oversized_by_paragraph` 372-397，绝不切断句子；单段落超限才 `split_by_char_limit` 399-419）；4. 无损：`debug_assert_eq!(segments.concat(), content)`（437）。

**抽取 prompt**（`LONG_IMPORT_PROMPT_TEMPLATE`，124-192）
- 输出结构：document（目录入口：catalogSummary/routingMap/riskNotes/productTags≤5/businessTopics≤3）+ items（主题包）+ chunks（运行时按需打开的切片）。
- chunk 要求 LLM 判 `wikiType` 9 类（source/entity/concept/comparison/synthesis/methodology/finding/query/thesis，162）与 `chunkType` 4 类（product_fact/style_template/peer_case/negative_example，163）。
- 强约束（183-189）：量化事实连**限定条件**一起保留；每个**离散信息单元**（决议/待办+责任人+截止日）各自落地；只忠于原文禁止推断；绝对承诺逐条写入 document.riskNotes。
- 内容合约：`import_extraction_has_content_or_reason`（211-224）——items/chunks 至少一个非空，或 noKnowledgeReason 非空；违约走修复 prompt 重试 ≤3 次（`import_user_prompt_for_attempt` 226-266、`generate_import_segment` 268-315，每次先查 ambient budget 281-286）。

**受控抽取主流程**（`run_import_extraction_controlled`，880-1084）
- budget：`IMPORT_RUN_TOKEN_BUDGET=600_000`、max_calls=段数×3（700-730）。
- checkpoint 命中段直接复用（898-919）；未完成段 `buffered(IMPORT_EXTRACT_CONCURRENCY=2)` 并发抽取（931-993）；worker 场景每段成功即 `persist_import_segment_checkpoint`（960-974）——事务内先 touch owner 行（claim filter 匹配失败=claim 丢失 → `import_job_claim_lost`，816-828），再 upsert `import_job_segments`（content_hash 绑定 schema v1+源名+序号+内容，732-742；TTL 48h，208）；瞬时事务错误指数退避重试 ≤5 次（806-877）。
- 收尾：保序收集，单段失败 warning 跳过、全失败上抛首错（1000-1032）；document 确定性合并 `merge_preview_documents`（458-505：标量取首个非空、summary/catalogSummary 拼接、数组并集去重）；items/chunks 逐段 normalize 拼接（1044-1055）；**D2 锚定对完整原文再跑 `integrity_report_for_preview`（1070-1071，红线不动）**；返回带 importReport{totalSegments,succeeded,failed}（1072-1083）。

**seal 机制**（90-118）
- 写入 `previewId`；chunks 逐个注入 `candidateId = "candidate-%04d"`（103-111）；移除旧 previewHash 后按 canonical JSON（键排序，53-67）算 sha256 存回 `previewHash`（112-117）。apply 端凭 (previewId, previewHash) 校验储存体，不信任客户端提交的 chunks 内容。

**apply 落库**（1223-1633）
- 请求：previewId + previewHash + chunks[{candidateId, patch}]；`import_apply_request_hash`（1311-1336）：candidateId 排序去重、trim 校验、patch 白名单校验（`validate_import_candidate_patch` 1338-1363，EDITABLE_FIELDS 与 chunk patch 白名单一致）→ 请求指纹 sha256。
- 重试：TransientTransactionError 指数退避重试 ≤6 次，每轮先查 `committed_import_apply_receipt`（1281-1309，按 previewId+owner+preview_hash+request_hash 收敛幂等回执）（1235-1279）。
- 事务内（`import_apply_in_transaction`，1396-1633）：
  1. 读 job（filter：completed+workspace+owner，1406-1420）；stored preview 与 hash 双重校验（客户端 hash 与库中 hash 相等 **且** 重算 hash 相等，1429-1434）；
  2. 幂等：`apply_status="applied"` 且 request_hash 相同→返回旧 receipt；不同→`import_preview_already_applied_with_different_selection`（1436-1445）；非 ready →`import_preview_not_ready:*`（1446-1451）；
  3. CAS ready→**applying**（1453-1480）；
  4. document：从 stored preview 反序列化，account/source_name/raw_content/content_hash/line/section 全部服务端重填，**status 强制 "active"**（1482-1495，文档级激活不等于知识可用——可用性在 chunk 层）；
  5. 每个选中 candidate：查 stored 候选（unknown id 400）→ 移除 candidateId → 覆盖 patch 字段（1529-1540）→ 反序列化为 ChunkRequest → **服务端强制 account/document_id/item_id=None/domain + `status="draft"` + `integrity_status="needs_review"` + `confidence=0`（1545-1551）→ `apply_chunk_integrity` 全文锚定（1552）→ 锚定后再次强制 draft+needs_review+0（1553-1556，注释"Anchoring imported text is evidence location, never verification"）** → insert + `apply_chunk_revision_with_session(op=Create, **source=Imported**)`（1559-1587）；
  6. CAS applying→**applied** + apply_result + applied_at + expires_at=now+24h（1600-1631）。

**PDF 入口**（1651-1730）：multipart 解析 file/sourceName/accountId；`import_pdf_bytes`：空字节 400；`pdf_extract::extract_text_from_mem` 在 `spawn_blocking`（1720-1723）；空文本 400（扫描件/加密）；→ `ingest_chunked_text`。

**图片入口**（2042-2103）：imageBase64 必填；`select_vision_provider`（1788-1866）：a) active 文字模型 supports_vision → Runtime snapshot（provider_id 与 DB 不一致时报 `workspace_provider_mismatch`，1804-1815）；b) 否则收集全部 supportsVision 副模型，isVisionActive desc→updatedAt desc 组成候选链；c) 空 → `visionNotSupported`。`vision_generate_json`（1881-2040）：required_text_field（"fence"）非空合约 ≤3 次重试（1771）；Dedicated 链上瞬时不可达（LlmUnavailable）自动切下一候选，非瞬时立即失败；system prompt（2058-2064）要求 fence 块格式 `---CHUNK: id---`…`---END CHUNK---`、原子信息单元穷尽枚举、保留原文 token 粒度、不编造、**"所有 chunk 默认 needs_review，不要写 verified"**；priority=Background（2074）→ `ingest_chunked_text`。

**ingest_chunked_text 事务**（2105-2587）
- `prepare_ingest`（2156-2265）：`ingest_identity_hash`（protocol v1 + workspace+account+source+text，2141-2154）派生**确定性** document/chunk ObjectId（sha256 前 12 字节，2129-2139）→ 相同输入重试幂等；document status="active" 原文全存（2171-2197）；`parse_chunk_blocks` fence 解析，块 JSON 反序列化失败记 parse_warnings；**零块 → fallback_blob**：整段文本落一个 `title="{source} · 待切分 blob"`、wiki_type=source 的兜底 chunk（2204-2219）；每块 `enforce_ingest_server_owned_fields`（2267-2286）：account/document_id/item=None/domain 服务端接管 + **draft+needs_review+confidence 0（锚定前后各强制一次，2277-2285）**；全部块非法 → 400（2254-2258）。
- 提交（2548-2587）：先 `read_committed_ingest`（2288-2365）重放检测——同 id 文档存在则校验 workspace/account/source/raw/hash 全等（不等=`ingest_identity_collision`），每 chunk 校验存在+归属+create/imported revision 存在，catalog_rebuild_jobs 计数 ≥ chunk 数（缺=`ingest_commit_incomplete`）；未提交过则事务写入 document+chunks+revisions（`persist_prepared_ingest_with_session` 2454-2495，source=Imported）；Transient/duplicate-key 重试 ≤3 次，每次重试前再做重放检测收敛（2560-2583）。

**标签抽取**（1134-1221）：prompt 要求 productTags 只放正文确实出现的产品名（可空不硬塞）、businessTopics 至少 1 个（1156-1173）；输出走 `normalize_knowledge_tags`（5/3 上限）。HTTP 端点 body 必填。

### 2.5 repair.rs — AI 自主修复

- 设计红线（1-9 头注释）：AI 只输出 patch 不写库；落库走独立 apply 端点；每 turn 独立 budget 4000 token/4 calls。
- `parse_repair_response`（90-167）：patch 非对象→空对象；missingFields/stillMissing 兼容字符串与 {field,reason} 两形态统一规整（108-141）；followupQuestions 仅对象项、截断 ≤REPAIR_MAX_TURNS=3（148-152）；confidenceHint clamp[0,100]（153-158）。
- `propose_chunk_repair_inner`（231-415）：**contract**——不自建 RUN_BUDGET scope（handler 建 4000-token scope；knowledge_task worker 复用其 STEP scope，218-222）；account_scope 传入时 chunk/document 查询都加 domain+账号可见性过滤（238-282）；父文档 rawText 截断 4000 字（308）；prompt 要求先"事实源体检"（父文档空+quote 空时 missingFields 首位必须是 sourceQuote，禁止编造填满，326-340）；LLM 走 `knowledge.chunk.repair.propose`；先 `record_knowledge_run_started`（fail-closed，315-323）；产物写 usage log（turn=1）+ `knowledge_repair_proposed` 事件（376-412）。**全程不写 chunk**。
- `propose_chunk_repair`（417-451）：session_id=uuid、run_id=`repair-chunk-{id}-{session}`；返回 patch/missingFields/followupQuestions/confidenceHint/budget。
- `answer_chunk_repair`（453-627）：turn clamp[2,3]（473）；prompt 携带上一轮 patch + 操作员回答（每条截断 600 字，495-505）；**最后一轮强制清空 followupQuestions（569-577，忽略 LLM 再追问）**；isFinalTurn 标志（624）。
- `record_repair_apply`（666-845）——修复 patch 的唯一提交端点：
  - target_kind 仅 "chunk"（671-675）；**then_verify=true 直接 400**（676-680，"repair apply cannot verify knowledge; use the dedicated verify route"）；
  - acceptedFields：非空、无重复、每项必须存在于 patch（686-705）；接受集走 `normalize_editable_chunk_patch` 白名单（706）；skipped 由服务端从 patch−accepted 推导，不信任客户端上报（707-713）；
  - 事务内：patch 含 source_quote 时用父文档原文重算 anchors（743-768）→ `apply_chunk_revision_with_session(op=Patch, **source=Ai**)`（769-786，harness 对 Ai source 强制 draft+needs_review）→ 同事务写 `knowledge_repair_applied` 事件（accepted/skipped/confidenceHint/extras 快照，790-824）；
  - extras 仅进审计 details 不落业务字段（60-88 DTO 注释、716-721）；响应 `status:"draft"/integrityStatus:"needs_review"`（837-844）。

### 2.6 chat.rs — 对话式补库 + 派工 + SSE

**chat_turn 主流程**（151-507）
- 输入：content 非空；session_id 缺省 uuid（160-166）；account 校验；`freeze_chat_chunk_attachments`（54-111）：operation 必须带 chunkId（61-72）；expectedUpdatedAt 若传则与库中毫秒级相等否则 `chat_chunk_snapshot_stale`（99-104）；无论是否传都**冻结**为当前 updated_at（105-108）——后续 apply 用它做 OCC。
- 会话归属：`ensure_chat_session_identity`（1028-1089）：`knowledge_chat_session_seqs` 行 `_id="{ws}|{session}"` upsert，绑定 workspace/session/account/owner_admin；`$or` 兼容旧无 scope 行（1044-1058）；归属不符（duplicate key / 匹配失败）→ `chat_session_scope_conflict`。读路径 `require_chat_session_identity`（1091-1117）。
- turn_index：`allocate_next_turn_indices(count=2)` 原子 `$inc seq`（1130-1165），user=assistant_index-1；**assistant 轮数 ≥8 拒绝**（211-216）。
- budget：24000 token/4 calls scope 包裹 pipeline（251-277）。
- pipeline（`run_chat_turn_pipeline`，1496-1670）：先取 operator memory（≤5 条，1510-1517）；intent 优先级：digest_selection 存在→强制 digest_action（1520-1521）＞attachment.operation=update→强制 update_chunk（1522-1523）＞LLM `classify_intent`（2174-2226，闭集 create_chunk/update_chunk/clarify_chunk/digest_action/update_operator_memory/revoke_operator_memory/freeform）；**attachment 的 chunkId 权威，LLM 不能重定向编辑目标（1543-1550）**；分支产物统一带 intent/naturalReply，空回复用 `synthesize_natural_reply_from_patch` 兜底合成（1659-1668，1318-1382）。
- assistant turn 落库（396-414）：attachments 回带 chunk/pack + expectedUpdatedAt + operation；digest_action 有完整候选时把 `{kind:"digest_dispatch_candidate", candidateHash, digestSelection, plannedSteps}` **封印进 turn attachments**（370-394，SR-125：派工确认时回验）。
- 审计：4 把 chat prompt key 的版本号（419-432）+ usage log（blocked_reason=`chunk_chat_session_pending_operator_apply`，445-465）+ `knowledge_chat_turn` 事件（466-484）。
- 响应：canApply = patch 非空 ∧ missingFields 空 ∧ draftKind 存在（338）。

**tool loop**（1721-2010）
- 常量：mutation（draft/update）1 轮、clarify 3 轮（1748-1749）；协议增广 prompt：decisionPhase tool_calling/final、toolCalls ≤6、中间轮禁业务字段（1751-1768）。约束表（1731-1739 注释）：单 dispatch 5s、失败连击 ≥3 强停、总 30s 硬超时、budget 断路、**永不写库/outbox/mcp**。
- `run_chat_with_tools`（1783-1955）：知识运行时快照——documents（active，limit 80）+ chunks（**active+verified**，limit 200）（1804-1847）；budget 取 ambient（1853-1860）；`last_raw` 暂存每轮原始 JSON（1862-1866）；reply_fn 把累计 `[system tool result]` 注入 user prompt（1892-1898）；出口 `finalize_chat_tool_loop_payload`（1957-1976）：raw 与归一化 phase 都是 final 才透传原始 payload，否则替换为截断 final（stopReason=budget_exhausted/tool_failure_streak/loop_exhausted/forced_stop，1978-1994）；Timeout 返回温和 final 不上抛（1943-1951）。
- 草稿合约修复：`chat_draft_requires_contract_repair`（1386-1393，无可用 patch 且未声明 missingFields=矛盾空提案）→ 修复 prompt 重试 ≤2 次（2289-2314、2426-2452，预算不足则标 degraded 停）；仍违约 → `mark_chat_draft_contract_incomplete`（1439-1458，missingFields=["patch"] 不可 apply）。
- update 分支（2322-2459）：expectedUpdatedAt 作为查询条件（陈旧→`chat_chunk_snapshot_stale`，2342-2350）；响应回传冻结 `expectedUpdatedAt`（2457）。

**chat_apply**（579-914）
- 重试：Transient ≤6 次，退避 20ms<<n（633-637），每轮先查已提交 receipt（`load_committed_chat_apply_receipt` 639-670：最后一条带 patch 的 assistant turn status=applied → apply_result）。
- 事务（700-914）：session 行归属校验（707-722）→ 取最后一条带 patch 的 assistant turn（732-740）→ applied 幂等返回 receipt（741-747）；非 pending → `chat_draft_not_applicable:*`（748-753）→ CAS pending→**applying**（763-782）→ intent 分流：
  - create_chunk → `apply_create_chunk_with_session`（812-827；account=默认账号时传 None 落共享域，814-815）；
  - update_chunk → `apply_update_chunk_with_session`（828-843，chunkId 必须来自 attachments）；
  - 其它 intent → 400（844-848）。
  - operator_statement = 全部 user turn 文本 join（784-790，作溯源陈述）。
- 收尾：`knowledge_chat_applied` 事件（dedupe_key=`knowledge_chat_apply:{turnId}`，858-884）→ CAS applying→**applied**+apply_result（885-912）。
- `chat_discard`（945-989）：update_many pending→discarded。

**落库内核（chat/task 共用）**
- `resolve_quote_anchors`（2678-2702）——D2 锚定纯函数：statement 空→quote=None（不动原 quote）+anchors 空（verify 合法拒绝）；否则 patch_quote 能在 statement 锚上就用它、锚不上回退 statement 全文；不变量"返回 quote 必有配对 anchor"由测试锁死（3918-3941）。
- `apply_create_chunk_with_session`（2738-2808）：patch→ChunkRequest（`chunk_request_from_chat_patch` 2972-3018，title 缺省"AI 对话产物（草稿）"）；**强制 draft+needs_review（2751-2752）**；resolve_quote_anchors 用运营陈述锚定（2754-2760）；insert + Create revision（**source=Ai**, actor="knowledge_chat"，2771-2784）；返回 updatedAt（apply 后重读，2785-2799）。
- `apply_update_chunk_with_session`（2842-2968）：expectedUpdatedAt 可选 OCC（2862-2870）；camelCase patch → snake_case 11 键映射表（2872-2903，含 routing_card/safe_claims/forbidden_claims/evidence_items 残留键，见疑点 3）；无可识别字段 no-op（2904-2910）；source_quote 变更强制重算 anchors（2918-2925）；`apply_chunk_revision(op=Patch, **source=Ai**)`——注释明示"source=Ai 自动强制 status=draft + integrity_status=needs_review"（2927-2944）。

**digest 派工**
- `dispatch_digest_action_for_chat`（2504-2659）：digest_selection 传入时校验 account+今天日期（2522-2528）；当日报告必须存在（2529-2541）；无 binding→LLM（`knowledge.digest.dispatch`）从未 dismiss 卡片（≤20）挑 cardId（2546-2597）→ 服务端按权威卡片重建 selected_cards（去重、cardHash、≤8，2598-2619）；`resolve_digest_selection` 重建步骤 + candidateHash（2643-2657）。
- `resolve_digest_selection`（3086-3176）：绑定校验五元组（account/reportId/reportDate/**report_generation=current_generation**/reportHash，3093-3102）；selectedCards 1..=8、无重复；卡必须存在、未 dismiss、cardHash 相等（`digest_dispatch_card_changed`）；suggestedAction 闭集 {fix_chunk,add_chunk,retag,review_evolution,dismiss}，freeform 不可派工（3136-3155）；step={stepId,cardId,action,summary,reportDate[,targetChunkId]}（3157-3166，targetChunkId 从卡 target_refs 第一个 kind=chunk 提取，3045-3058）。
- `chat_task_create`（3219-3408）：session 归属+digest_selection.account 校验（3242-3246）；**重读当前权威日报重建步骤，客户端 plannedSteps/cardIds 仅做一致性校验绝不作为写入来源**（3248-3264，`legacy_dispatch_payload_matches` 3178-3217）；candidate_hash 不符→409（3266-3272）；带 sourceTurnIndex 时回读原 assistant turn 的 `digest_dispatch_candidate` 封印校验（3276-3307）；落 `KnowledgeChatTask{status:"pending", dispatch_binding(protocol=digest_dispatch_v1), planned_steps, cards}`（3319-3350）→ 写一条 task_progress turn + bus.bump（3352-3400）。
- task 查询/取消（3427-3601）：list/get 都过滤 owner_admin_id；cancel 只对 pending/running 生效并 `$unset` claim 字段（3539-3564），终态幂等返回 `alreadyTerminated:true`（3565-3597）。
- SSE `chat_session_stream`（3609-3662）：订阅 ChatProgressBus watch；值=CLOSE_SENTINEL 时发 `close` 事件后断流；否则发 `turn` 事件（版本号），前端回拉 history。

### 2.7 catalog.rs — 目录/完整度/integrity 报告

- `build_operation_knowledge_catalog`（316-392）：documents（active，limit 100，catalogSummary 回退 summary）+ chunks（**active 且 integrity_status=verified**，limit 200——目录只暴露可用知识）；items 恒空（351-353，旧库已删）。
- `get_operation_knowledge_catalog_persisted`（64-109）：读 `catalog_summary_persisted` 快照 O(1)；`catalogFresh = persisted.is_some() ∧ desired>0 ∧ applied==desired`（94-96）；未跑过 worker 时前端回退 live 聚合。
- completeness（111-157）：GET 走进程内 DashMap 缓存（TTL=`completeness_cache_ttl_seconds` 默认 300s，config.rs:497；命中即回，123-135）；refresh 强制重算写回缓存。
- `build_operation_knowledge_completeness`（497-804）：
  - 统计：total / verified（active+verified）/ evidence（verified 且 evidence_items 非空）/ anchored（verified 且 source_anchors 非空）/ needs_review（511-590）；verified 摘要 limit 80 **含 body**（552-576，注释：可验证事实住 body，缺 body 会误判方法论）；pending limit 40（591-613）。
  - fallback：mode= relationship_only（verified=0）/product_safe（evidence=0）/fully_supported（614-620）；fallback_gaps 确定性下限——verified=0 缺口 + needs_review>0 的"待审定知识"缺口（含前 5 个标题，621-647）。
  - coverage 维度动态化：读 active DomainProfile（665-666）；initial_signal 决定初值（verified/evidence/恒 false，671-677）；prompt 骨架/锚点按维度渲染（H5-a/b，439-471，DEFAULT 销售五维字节等价，测试 814-857 锁死）。
  - LLM：`knowledge.digest` 无关，system 是完整度 Auditor（688）；user prompt（703-751）规定四类认知状态、三布尔位可并存（verifiedFact/methodologyOnly/pendingDraft）、methodologyOnly 防滥标、gaps 三要素整改指令；**Foreground 并发优先级**（752-757）。
  - 两道确定性护栏：`clamp_answering_mode`（406-412）——needs_review>0 时 fully_supported 强制降 product_safe（LLM 失败也走 fallback 769）；`merge_completeness_gaps`（419-432）——确定性下限恒在前，LLM 空 gaps 抹不掉。
  - 响应带 answeringModeLabels（随 profile）+ dimensionList（维度声明序，缺失维度回落 state="missing"，478-495）。
- `build_operation_knowledge_integrity_report`（177-235）：limit 500；`anchorsMissing` = active ∧ anchors 空（209-211，与 digest_inbox 口径对齐）；items 只含非 verified。
- 检索工具（237-314）：search/test-match 都调 `agent::test_knowledge_route_for_contact`（走生产 knowledge_router）；open-slice 按 ids 取 limit 50。

### 2.8 wiki_edit.rs — wiki 编辑

- 顶部注释（26-33）声明 harness 六保证：锁定字段守门 / 数组 union / 70% body 长度阈值 / **AI source 强制 draft+needs_review** / 同事务 revisions+chunks+updated_at CAS / 同事务推进父 catalog generation。
- patch/archive/restore（76-155）：全部 source=Human、走 harness、成功后 WebSocket 广播。
- rollback（161-202）：事务内 `rollback_chunk_revision_with_session` 恢复到指定 revision 之前的状态；注释：identity/tenant/lock/runtime-stat 服务端持有、**恢复内容必须重新 review**、无快照的旧 revision fail-closed（157-160）。
- revisions 分页（214-274）：父 chunk 归属校验 + revision workspace 双重过滤（222-231）；limit clamp[1,200] 默认 20；patch 走 canonical extjson。
- split（278-452）：offset 必须在 (0, char_count) 开区间（319-324）且两侧 trim 非空（327-331）；**两个子块：title 加 "（n/2）"、summary=None、`source_quote=None`+anchors 清空（342-347，"原始证据不能假定证明子块"）、强制 draft+needs_review+confidence 0（348-350）、previous_version_id=源 id、related/usage/dynamic 清空（353-358）**；子块先建成功后源块才 op=Split patch status=archived（384-399）；顺序保证失败不丢源（333 注释）。
- merge（456-645）：不能自并（472-476）；双方非 archived（497-501）；**同 domain+同 account 才可并**（502-506）；body/summary 用 `join_distinct` 拼接（508-524）；数组字段并入 patch（union 由 harness 做，525-539）；**target 的 source_quote 置空、source_anchors 清空（545-547，"merged claim set needs fresh evidence"）**；target 的 locked_fields 命中 patch 键则整体 400（不允许静默丢内容再归档源，549-566）；target op=Merge patch、源 op=Merge patch{status:archived, superseded_by:target}（568-598）。
- relate（647-774）：kind 闭集 6 种（superseded_by/references/requires/contradicts/clarifies/refines，649-669）；自指拒绝；**可见性收口：私有源可指 shared+同账号，shared 源只能指 shared（707-717，防跨账号读隧道）**；同 (target,kind) 幂等更新 note（737-742）；落库走 op=Patch 写整个 related_chunks 数组。unrelate（777-836）：过滤后无变化返回 removed=0。
- referrers（849-895）：`related_chunks.chunk_id` 反查，limit 50，不物化反向边（846-848）。
- batch-verify（916-965）：≤100 条；逐条走 `verify_chunk_at_version`（与单条同一事务内核，每条独立 expectedUpdatedAt）；部分成功语义 verified[]/skipped[{id,reason}]；注释重申红线：批量入口仍是 admin 手工触发（915）。batch-archive（977-1014）：≤100，逐条 op=Archive。

### 2.9 digest_inbox.rs — 日报 HTTP + Inbox

- `digest_today`（33-75）：按 (workspace, account, report_date[默认今天]) 查 `knowledge_daily_reports`；**未命中同步调 `generate_today_digest` 合成**（62-71，Phase 2 起不再 404）。
- `digest_regenerate`（86-122）：force=false 时已存在直接返回（不重复烧 LLM）；force=true 强制重算。
- `digest_dismiss_card`（145-187）：accountId 必填；report_date 恒今天（156）；`$addToSet dismissed_card_ids`；未命中 404。
- `serialize_digest_report`（189-233）：每卡带 `cardHash`（快照哈希）、报告带 `reportHash`、attempt/current generation 与 latest_attempt_* 审计字段全量下发。
- `knowledge_inbox`（366-572）——四类只读信号聚合，**不写库**：
  1. digest_card：当日未 dismiss 卡片（380-441，severity→priority：critical=high/warn=mid/其它=low，285-291；kind 映射 306-316；suggestedActions 永含 dismiss，294-303）；
  2. pending_review：integrity ∈ {needs_review, needs_human_audit} 且 7 天内更新（484-513）；`chunk_type=negative_example` 升 high+origin=negative_example_review（333-339，reaction 误判反馈链路的 admin 二次确认入口）；needs_human_audit 标 "AI预审通过待复核"+origin=human_audit_pending（343-353）；
  3. quote_missing：active 且无 quote → high（516-529）；
  4. anchors_missing：active 且 anchors 空 → high（531-545，裸 `!is_empty()` 口径）。
  - chunk 扫描：status ∈ {active,draft}、共享+本账号、updated_at desc limit 200（444-464）；priority 过滤→稳定排序（priority_rank 降序，319-326）→截断 limit（默认 24 clamp[1,100]）→stats。

### 2.10 sources_meta.rs — 问答/信号/摄取源/记忆

- `analyze_operation_knowledge_logs`（68-152）：hours 默认 24 上限 72（74）；only_blocked_or_held 默认 true（`review_approved=false` 或 blocked_reason 存在，91-101）；limit 50；输出 totalRuns/blockedOrHeldRuns/topChunks（≤8）/items——与 chat tool `knowledge.analyze_logs` 同语义（62-67 注释）。
- `knowledge_aggregate_metadata`（163-329）：chunks 上 $facet（wikiTypeCounts + verifiedRatioByType），chunk_revisions 上 $facet（topEditors ≤10 + recentActivity 7d 按 date×op 分组）；只读。
- gap-signals（334-486）：list 默认 status=pending、可选 kind、limit 100；dismiss/apply 都 CAS `status:"pending"`→dismissed/applied + resolution_note + resolved_at（未命中 404）；sweep 手动触发 `run_structural_lint` + `sweep_stale_signals`（knowledge_wiki::gap_signals）。
- `ask_knowledge`（522-577）：**忽略 body 的 workspace_id 一律用 session 的 current_workspace（531-534，防跨租户）**；account 传入则校验；filter.include_unverified=false（verified-only 语义，547-550）；调 `agent::knowledge_agent::answer`；tool_trace 走 relaxed extjson（555-563）；tookMs 后端计时。
- `ask_knowledge_stream`（619-761）：EventSource 只支持 GET，filter 走逗号分隔 query（586-611）；spawn 任务跑 `answer_streaming`，失败发稳定 `failed` 事件（内部错误只进日志，662-677）；`CancelOnDrop`（684-694）：客户端断开→body 流 drop→cancel 置 true→agent 提前退出；事件类型 trace/token/failed/answer/close（706-757）。
- operator-memory list（800-894）：expiry（无/null/未来）+ 未 revoked 过滤（include_revoked 可放开，818-841）；kind 闭集校验 preference/rejection/context（845-849）；limit 50 clamp[1,200]、last_used_at desc。revoke（896-949）：委托 `agent::revoke_operator_memory`（幂等：already_revoked），非幂等重放时写事件。
- ingest sources（951-1196）：kind 仅 rss/html（1045-1049）；**URL 走 `outbound_fetch::validate_public_http_url` SSRF 校验（990-995，测试 1352-1369 锁死拒绝 127.0.0.1/169.254.169.254/[::1]/带凭证 URL）**；schedule_minutes ≥1；status 写路径只接受 "active"（重置 failing，failure_streak=0 + last_error=null；failing/disabled 是 worker 闭集自迁移，1125-1134）；更新走 `source_generation` OCC（+1 并 `$unset` worker claim 三字段，1135-1171）；URL 变更清空 last_etag/last_content_hash（980-988）。

### 2.11 knowledge_task/mod.rs — chat 长任务 worker

- 隔离红线（12-15 头注释）：严禁 gateway/outbox/mcp；任何 chunk apply 强制 draft+needs_review；每 step 独立 RUN_BUDGET fail-soft。
- 常量：STEP_TOKEN_BUDGET=8_000、STEP_MAX_LLM_CALLS=4、TASK_LEASE_SECONDS=120、TASK_HEARTBEAT_SECONDS=20（30-33）。
- `ChatProgressBus`（44-143）：senders=HashMap<bus_key, watch::Sender<u64>>；bump 对已 close 的 sender 保持哨兵不回退（81-87）；close 发 CLOSE_SENTINEL=u64::MAX（58, 94-100）；`lock_for` per-session Arc<Mutex>（102-109，同 session 多 task 串行）；终态后 `schedule_cleanup` 延迟 300s（53, 115-122），清理校验 receiver_count==0 与 Arc::strong_count<=1（124-142）。
- claim 协议：`task_claim_filter`（177-195）= pending **或** running+lease 过期（锁没续上=崩溃遗留）；`claim_task`（197-232）find_one_and_update（created_at 最旧优先）置 running+worker_id+claim_token+locked_until+heartbeat/started_at 并 `$inc attempts+claim_generation`；`active_task_claim_filter`（244-248）=identity（_id+running+worker_id+claim_token+claim_generation）+`locked_until>now`。
- 心跳（258-297）：20s tick，`transaction_gate` 互斥（避免与 step 事务交错触发写冲突）；CAS 续约失败（被 reclaim/cancel）即退出。
- **step intent 两阶段提交**（防 LLM 产物在重试中翻倍落库）：`prepare_mutating_step`（402-559）只跑 LLM 产 payload（add_chunk 起草 patch / retag 抽标签 / dismiss 校验 cardId），`persist_step_intent`（358-395）把 payload 挂进 task.step_intents（stepId 唯一栅栏），重放时 `find_step_intent` 直接复用（937-938）；`commit_mutating_step_once`（561-736）事务内：ownership+completed_steps 无该 stepId 双重检查（573-586）→ 业务写（add_chunk→`apply_create_chunk_with_session`；retag→`apply_update_chunk_with_session`；dismiss→`$addToSet dismissed_card_ids`）→ **同事务 `$push completed_steps`**（708-721）→ commit；claim 丢失=anyhow 错误中止。
- `run_claimed_task`（831-1162）：起跑写 "started" progress turn（861-870）；重放已完成 step（881-895）；每 step 前 claim 活性检查，miss 时区分 cancelled（终止循环）与 reclaim（静默退出）（906-921）；mutating（add_chunk/retag/dismiss，935）走 intent→persist→commit；非 mutating（fix_chunk/review_evolution/analyze_logs）走 `execute_step` + `persist_nonmutating_outcome_once`（738-759，同样 stepId 栅栏，失败=claim 丢失即退出）；每步写 progress turn（含 repairDraft details，1016-1034）。
- 终态：`task_final_status`（761-767）——failed 或 needs_manual 非空 → "failed"（SR-123 不虚报成功）；CAS（active claim filter）置 completed/failed + error_kind（knowledge_task_step_failed/needs_manual）+`$unset` claim 四字段（1060-1078）；cancel 已改状态时 CAS 不命中不覆盖（1050-1052 注释）；summary turn（needsReviewChunkIds 去重 + failed/needsManual/noop step 清单，1090-1119）+ `knowledge_chat_task_finished` 事件（1121-1152）→ bus.close → schedule_cleanup（1154-1159）。
- `execute_step`（1202-1585）：
  - **fix_chunk**（1210-1275）：调 `propose_chunk_repair_inner` 产修复草稿塞进 details（repairDraft），chunkId 推入待审池；**worker 不 apply**（1213 注释红线）；LLM 失败→NeedsManual fail-soft。
  - add_chunk（1276-1391）：draft prompt 起草 → `apply_create_chunk`（**account=None 落 workspace 共享域**，operator_statement=卡片 summary 驱动锚定，1337-1348）。
  - retag（1392-1477）：`extract_knowledge_tags_inner` → `apply_update_chunk`（patch 仅 productTags/businessTopics；statement 传空不触发重锚定，1448-1457）。
  - review_evolution（1478-1484）：恒 NeedsManual（"AI 不自动放量"）。
  - analyze_logs（1485-1530）：events 24h 内 status ∈ {blocked, blocked_by_safety_guard, warning, warn} limit 200 按 kind 聚合；0 条=Noop。
  - dismiss（1531-1582）：`$addToSet dismissed_card_ids`；matched=0 Failed / modified=0 Noop / 否则 Committed。
- turn 写入（1591-1715）：`next_turn_index_atomic` 与 chat 路由共用 seqs 行 `$inc`（owner_admin_id 必须为 string 类型，1608）；role=system、intent=digest_action、kind=task_progress/task_summary；每写必 bump。

### 2.12 knowledge_digest/ — 日报合成

- 快照哈希：`digest_card_snapshot_hash`（36-51，覆盖运营批准所见的全部字段）；`digest_report_snapshot_hash`（56-80，reportId+scope+date+**current_generation**+cardHashes+dismissed，故 dismiss 或换代都会变 hash——供 dispatch 防漂移）。
- `worker_loop`（88-108）：`KNOWLEDGE_DIGEST_ENABLED=false`（默认，config.rs:741）直接 return；启用时 sleep 到 `KNOWLEDGE_DIGEST_RUN_HOUR`（默认 9，min(23)）整点 → `generate_all_account_digests`（112-129，逐账号隔离失败）；`duration_until_next_run`（141-169）至少 60s 防边界死循环。
- 4 路只读分析（171-179 准则：全部只读、结构化中间信号、统一 generate_agent_json 挂 RUN_BUDGET）：
  1. `analyze_chunks_health`（234-309）：needs_review/missing_evidence 或缺 quote 或 draft≥7d；产 missing_fields（sourceQuote/integrityStatus）+age_days；≤200 条。
  2. `analyze_usage_logs`（312-353）：24h usage 命中率；hit=review_approved ∧ 无 blocked_reason；per_chunk (used,blocked) 计数；miss 样本 ≤5 条 ×60 字。
  3. `analyze_run_logs`（358-470）：`agent_run_logs.final_review_status ∈` 4 个 block 状态（364-369：blocked_by_required_field/blocked_by_budget/blocked_unverified_product_claim/blocked_by_safety_guard）；反查 `knowledge_route.selectedChunkIds` 分桶（每桶 run_ids ≤8）；**前 6 大桶走 LLM `summarize_block_runs`（472-519，输入已脱敏，空 summary 报 LlmUnavailable），其余用 labels::block_reason_zh fallback 文案**（431-460）。
  4. `analyze_evolution`（522-572）：proposals 24h 内 eligible_for_release/rolled_back（或恒 eligible），≤50。
- `compose_cards`（575-666）：chunk_health 前 80 条 + usage 摘要（lowHitRateChunkIds 判据：used+blocked≥3 ∧ blocked×2>used，602-607）+ blocked + evolution → `knowledge.digest.compose` LLM → `digest_card_items`（676-694，兼容数组 / {cards:[]} / 单卡对象三形态）→ `parse_cards_from_llm_array`。
- `parse_cards_from_llm_array`（807-925）：闭集校验——kind 7 种、severity 3 种、suggestedAction 6 种（812-829），违例**整卡丢弃**；title 非空 ≤60、summary ≤200 截断；target_refs 项必须 kind+非空 id；metric 认 i64/f64；**`stable_card_id`（701-729）= sha256(account|date|kind|refs 签名|title) 前 12 字节 → regenerate 后同语义卡片 id 稳定（R5：dismiss 不复活）**；≤50 张；排序 `compare_digest_cards`（795-799）：severity（critical>warn>info，756-763）→ metric.value 降序（NaN/缺失按 f64::MIN 垫底，774-784，total_cmp 防 panic）。
- attempt 世代协议：
  - `claim_digest_attempt`（973-1050）：upsert `$setOnInsert`（初始 status="failed" 占位骨架）+ `$set latest_attempt_status="running"` + **`$inc attempt_generation`**；首插撞唯一键的并发者按 update 重试（1023-1047）。
  - `do_generate`（1261-1446）：4 路分析+compose 任一失败→整体 status 归类：ok / failed（LlmUnavailable→kind；其它→"internal"）/ **partial**（BudgetExceeded→error_kind="budget_exceeded"）（1310-1335），失败/partial 时 cards 为空。
  - `finalize_digest_attempt`（1053-1153）：filter 追加 `attempt_generation`（世代栅栏，1077）；**status=ok 才覆盖可见快照**（cards/current_generation=attempt/last_success_at，1093-1104）；失败且从未成功→失败可见（1105-1117）；失败但曾成功→只更新 latest_attempt_*（保留旧成功快照）；ok 时 `$addToSet` 迁移 dismissed（`migrated_dismissed_card_ids` 731-753：老 dismissed 卡按 kind+title+target_refs 语义匹配映射到新 id，SR-124）；晚到的 attempt 匹配不到行→返回当前权威快照（1138-1152）。
  - 审计（1381-1443）：usage log（kind=digest_compose，superseded attempt 标 blocked_reason=digest_attempt_superseded）+ `knowledge_digest_generated` 事件，均 fail-soft。
- budget：config `KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET` 默认 24_000、`KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS` 默认 8（config.rs:743-754）；run_id=`digest_{ws}_{acc}_{date}_g{generation}`（942-945）。
- labels.rs（5-14）：4 个拦截码→中文（必填信息缺失/本轮算力预算耗尽/产品说法未经核实/安全门拦截），未知回落原值。

### 2.13 import_worker.rs — 异步导入 worker

- `ImportJobClaim`（30-58）：job_id+claim_generation+claim_token 三元组；`filter()` 恒带 `status:"running"`——所有写都以此为前提（fencing）。
- `update_owned_import_job`（62-72）：matched≠1 = fencing 事件，调用方必须停产。
- `reclaim_stale_running_jobs`（132-194）：`claimed_at < now - import_job_claim_timeout_seconds`（默认 600s，config.rs:532）视为孤儿；快照 filter 冻结 _id+claimed_at+generation+token（76-104，后来者 claim 会改这些字段，旧扫描器抢不回）；`claim_recovery_count+1 ≥3` → 直接 failed+expires 24h（146-169）；否则 CAS 重置 pending（171-188）。**认领粒度是整 job；段级续传由 checkpoint（import.rs）承担，重跑只重抽未缓存段**（130-131 注释与 import.rs:898-919 行为叠加）。
- `claim_one`（198-224）：pending 最旧，置 running+token+claimed_at，`$inc claim_generation`。
- `run_job`（228-394）：
  - 心跳 `spawn_claim_heartbeat`（448-485）：间隔=timeout/2 clamp[5,60]（复用 tasks::claim_heartbeat_interval_seconds，454）；CAS 失败/DB 错→置 cancelled 退出。
  - 进度桥（247-304）：unbounded channel → 单 drainer 串行写 progress_done/succeeded/failed（并发回调乱序用 max_done 单调守护，256-264）；写失败/fencing→置 cancelled。
  - 抽取：`run_import_extraction_for_job`（import.rs:1092-1130，带 claim 启用 checkpoint、cancelled 标志贯穿）。
  - 收尾：cancelled=true 抑制终态写（323-330）；成功→`seal_import_preview_result`→CAS 置 completed+result+preview_hash+**apply_status="ready"**+expires_at=now+24h（339-357）；seal/序列化/抽取失败→failed+error+expires（358-393）。
  - `finish_owned_import_job`（396-443）：status 先过 `validate_import_job_status`；CAS 成功后**删除该 job 的全部 checkpoint**（418-428）；fenced 时仅记日志（429-434）。
- 主循环 `run_import_worker`（106-116）：每 tick 间隔 `import_worker_interval_seconds` 默认 2s（config.rs:530）；tick=先回收再认领一个（118-125，单 worker 一次一 job）。

---

## 3. 跨文件机制

### 3.1 异步导入 job 完整生命周期
1. **建 job**：`POST /api/operation-knowledge/import-preview`，content >3000 chars → insert `import_jobs{status:"pending", owner_admin_id}`（import.rs:564-608）；≤3000 chars 同步完成（不经 worker，import.rs:517-562，直接落 completed+ready）。
2. **认领**：import_worker 每 2s tick（import_worker.rs:106-125）；先 `reclaim_stale_running_jobs`（600s 超时→pending，累计 3 次→failed，132-194），再 `claim_one` CAS pending→running（+generation+token+claimed_at，198-224）。
3. **执行**：心跳续约 claimed_at（timeout/2）；`run_import_extraction_for_job`（import.rs:1092-1130）→ budget 600k token → 命中段级 checkpoint 复用（import.rs:898-919），未完成段并发 2 抽取，每段成功事务内 touch owner + upsert checkpoint（import.rs:793-878）；进度经 channel→drainer 回写 job（max_done 单调 + fencing，import_worker.rs:250-304）。claim 丢失（心跳/进度写失败）→ cancelled 贯穿抽取（import.rs:941-946, 996-998 返回 `import_job_claim_lost`）→ 抑制终态写（import_worker.rs:323-330）。
4. **完成**：`seal_import_preview_result`（previewId+candidateId+previewHash，import.rs:90-118）→ CAS running→completed + result + preview_hash + apply_status="ready" + expires_at=24h（import_worker.rs:339-357）→ 删 checkpoint（418-428）。前端轮询 `GET import-preview-job/:id`（owner 隔离，import.rs:613-634）。
5. **apply**：`POST import-apply`（import.rs:1223-1633）——previewHash 双重校验、request_hash 幂等回执、ready→applying→applied 两段 CAS、document status="active"、每 chunk 强制 draft+needs_review+confidence 0 + 全文锚定 + Create revision(source=Imported)。
6. **清扫**：终态（completed/failed/applied）都置 expires_at=now+24h，Mongo TTL 索引清理（import_worker.rs:332-338 注释）；pending/running 无 expires_at 绝不被删。

### 3.2 chat 长任务生命周期
1. **候选**：`chat_turn` intent=digest_action → `dispatch_digest_action_for_chat`（chat.rs:2504-2659）——运营勾选（digest_selection binding）或 LLM 挑卡后服务端重建 → `resolve_digest_selection` 按当日权威日报重建 plannedSteps + candidateHash（3086-3176）→ 候选封印进 assistant turn attachments（kind=digest_dispatch_candidate，370-394）。
2. **落任务**：`POST /api/knowledge/chat/tasks`（3219-3408）——重读权威日报重建（客户端 payload 仅一致性校验）、candidateHash/封印校验 → `knowledge_chat_tasks{status:"pending"}` + task_progress turn + bus.bump。
3. **执行**：knowledge_task worker 30s tick（interval=0 禁用，knowledge_task:147-164）→ claim（lease 120s、心跳 20s）→ per-session bus 锁串行（800-813）→ 逐 step：mutating 走"LLM 准备→intent 持久化→事务 commit+completed_steps 同写"两阶段（397-736）；非 mutating 走 execute_step + stepId 栅栏持久化（738-759, 968-991）；每步 STEP budget 8000/4 fail-soft；每步写 task_progress turn（原子 turn_index，与 chat_turn/chat_task_create 三方共用 seqs 行 `$inc`，1591-1621）。
4. **取消**：`POST tasks/:id/cancel` 置 cancelled + $unset claim（chat.rs:3526-3601）；worker 每步前 claim 活性检查发现即停（908-921）；终态 CAS 用 running filter 不覆盖 cancelled（1050-1052, 1060-1078）。
5. **收尾**：final_status=completed（全部 committed/noop）/failed（存在 failed 或 needs_manual，761-767）→ task_summary turn（needsReviewChunkIds 等）→ `knowledge_chat_task_finished` 事件 → bus.close（SSE 发 close 断流，chat.rs:3645-3660）→ 300s 后清理 sender/lock（knowledge_task:115-142）。
6. **产物审核**：add_chunk/retag/fix_chunk 的产物全部是 draft+needs_review（chunk 或修复草稿 details），运营在编辑器审核后走 `/verify` 闸。

### 3.3 digest 生成旅程
1. **触发**：三入口——worker 每日 run_hour 整点全账号（digest:88-129）；`GET digest/today` 未命中同步合成（digest_inbox.rs:62-71）；`POST digest/regenerate`（force 语义，86-122）。
2. **claim**：`claim_digest_attempt` upsert + `$inc attempt_generation` + latest_attempt_status="running"（973-1050），并发首插竞态用 duplicate-key 重试收敛。
3. **分析**：RUN_BUDGET（24000/8）scope 下 4 路只读分析（chunks health / usage 24h / run_logs 4 block 状态分桶 + 前 6 桶 LLM 摘要 / evolution proposals）→ `compose_cards`（knowledge.digest.compose）→ 闭集校验 + stable_card_id + severity/metric 排序 + ≤50（1261-1296, 575-925）。
4. **finalize**：attempt_generation 栅栏——ok 才覆盖可见快照并推进 current_generation + 迁移 dismissed；失败不抹已成功快照；superseded attempt 返回权威快照（1053-1153, 1337-1379）。
5. **审计**：usage log kind=digest_compose + `knowledge_digest_generated` 事件（1381-1443）。
6. **消费**：`digest_today` 下发 cardHash/reportHash → 运营勾卡派工（3.2 流程，reportHash 绑定 current_generation 与 dismissed 集合，任何漂移→`digest_dispatch_snapshot_stale`/`digest_dispatch_card_changed`）；`dismiss` `$addToSet`（stable_card_id 保证 regenerate 后 dismiss 不复活）。

---

## 4. 事实卡速查

### 4.1 全部端点表（挂载于 src/routes/mod.rs:480-725，前缀 /api）

| 方法+路径 | Handler（文件） |
|---|---|
| GET/POST `/operation-knowledge` | list_operation_knowledge / create_operation_knowledge（crud，旧端点空/400） |
| GET/POST `/operation-knowledge/documents` | list/create_operation_knowledge_document（crud） |
| GET/PUT/PATCH/DELETE `/operation-knowledge/documents/:id` | get/update/patch/delete_operation_knowledge_document（crud） |
| GET `/operation-knowledge/documents/:id/chunks` | list_operation_knowledge_document_chunks（crud） |
| GET/POST `/operation-knowledge/chunks` | list/create_operation_knowledge_chunk（crud） |
| GET `/operation-knowledge/review-queue` | list_operation_knowledge_review_queue（crud） |
| GET/PUT/DELETE `/operation-knowledge/chunks/:id` | get/update/delete_operation_knowledge_chunk（crud） |
| GET `/operation-knowledge/chunks/:id/source` | get_operation_knowledge_chunk_source（crud） |
| POST `/operation-knowledge/chunks/:id/verify` | verify_operation_knowledge_chunk（verify） |
| POST `/operation-knowledge/chunks/:id/reject` | reject_operation_knowledge_chunk（verify） |
| POST `/operation-knowledge/chunks/:id/repair` | propose_chunk_repair（repair） |
| POST `/operation-knowledge/chunks/:id/repair/answer` | answer_chunk_repair（repair） |
| POST `/operation-knowledge/chunks/:id/patch` | patch_operation_knowledge_chunk（wiki_edit） |
| POST `/operation-knowledge/chunks/:id/archive` | archive_operation_knowledge_chunk（wiki_edit） |
| POST `/operation-knowledge/chunks/:id/restore` | restore_operation_knowledge_chunk（wiki_edit） |
| POST `/operation-knowledge/chunks/:id/rollback/:revision_id` | rollback_operation_knowledge_chunk（wiki_edit） |
| GET `/operation-knowledge/chunks/:id/revisions` | list_operation_knowledge_chunk_revisions（wiki_edit） |
| POST `/operation-knowledge/chunks/:id/split` | split_operation_knowledge_chunk（wiki_edit） |
| POST `/operation-knowledge/chunks/:id/merge` | merge_operation_knowledge_chunk（wiki_edit） |
| POST/DELETE `/operation-knowledge/chunks/:id/relate[/:target_id]` | relate/unrelate_operation_knowledge_chunk（wiki_edit） |
| POST/DELETE `/operation-knowledge/chunks/:id/lock` | chunk_locks::acquire/release（chunk_locks.rs，非本次范围） |
| GET `/operation-knowledge/chunks/referrers` | list_chunk_referrers（wiki_edit） |
| POST `/operation-knowledge/chunks/batch-verify` | batch_verify_chunks（wiki_edit） |
| POST `/operation-knowledge/chunks/batch-archive` | batch_archive_chunks（wiki_edit） |
| GET `/operation-knowledge/catalog` / `/catalog/persisted` | get_operation_knowledge_catalog / _persisted（catalog） |
| GET `/operation-knowledge/completeness`（+POST refresh 同路由 mod.rs:598-601） | get/refresh_operation_knowledge_completeness（catalog） |
| GET `/operation-knowledge/integrity-report` | get_operation_knowledge_integrity_report（catalog） |
| POST `/operation-knowledge/tools/search` | search_operation_knowledge_tool（catalog） |
| POST `/operation-knowledge/tools/open-slice`、`/tools/open-evidence` | open_operation_knowledge_slices（catalog，双路由同 handler） |
| POST `/operation-knowledge/test-match` | test_operation_knowledge_match（catalog） |
| POST `/operation-knowledge/auto-verify` | auto_verify_operation_knowledge_chunks（verify） |
| POST `/operation-knowledge/import-preview` | import_operation_knowledge_preview（import） |
| GET `/operation-knowledge/import-preview-job/:id`、`/import-preview-jobs` | get/list_import_preview_job(s)（import） |
| POST `/operation-knowledge/import-apply` | import_operation_knowledge_apply（import） |
| POST `/operation-knowledge/import-apply-pdf`、`/import-apply-image` | import_operation_knowledge_apply_pdf/_image（import） |
| POST `/operation-knowledge/extract-tags` | extract_operation_knowledge_tags（import） |
| GET `/operation-knowledge/usage` | list_knowledge_usage（sources_meta） |
| GET `/operation-knowledge/logs/analyze` | analyze_operation_knowledge_logs（sources_meta） |
| POST `/operation-knowledge/repair/applied` | record_repair_apply（repair） |
| POST `/operation-knowledge/chat` | chat_turn（chat） |
| GET `/operation-knowledge/chat/:session_id` | chat_history（chat） |
| POST `/operation-knowledge/chat/:session_id/apply` / `/discard` | chat_apply / chat_discard（chat） |
| GET `/operation-knowledge/inbox` | knowledge_inbox（digest_inbox） |
| GET `/operation-knowledge/metadata` | knowledge_aggregate_metadata（sources_meta） |
| GET `/knowledge/digest/today` | digest_today（digest_inbox） |
| POST `/knowledge/digest/regenerate` | digest_regenerate（digest_inbox） |
| POST `/knowledge/digest/cards/:id/dismiss` | digest_dismiss_card（digest_inbox） |
| GET+POST `/knowledge/chat/tasks` | chat_task_list / chat_task_create（chat） |
| GET `/knowledge/chat/tasks/:id`、POST `.../cancel` | chat_task_get / chat_task_cancel（chat） |
| GET `/knowledge/chat/sessions/:sid/stream` | chat_session_stream（chat，SSE） |
| GET `/knowledge/gap-signals`、POST `.../:id/dismiss|apply`、POST `.../sweep` | gap-signal 族（sources_meta） |
| POST `/knowledge/ask`、GET `/knowledge/ask/stream` | ask_knowledge / ask_knowledge_stream（sources_meta） |
| GET `/knowledge/metrics` | knowledge_metrics（sources_meta） |
| GET `/knowledge/operator-memory`、POST `.../:id/revoke` | list_operator_memory / revoke_operator_memory（sources_meta） |
| GET+POST `/knowledge/ingest-sources`、PATCH+DELETE `.../:id` | ingest sources CRUD（sources_meta） |
| PUT/DELETE `/operation-knowledge/:id` | update/delete_operation_knowledge（crud，恒 400） |

### 4.2 status 闭集
- `import_jobs.status`：pending / running / completed / failed（models.rs:1052-1071，写点全部过 validate/assert）。
- `import_jobs.apply_status`：ready / applying / applied（import.rs:545, 1467, 1616；legacy 无值报 `import_preview_not_ready:legacy` import.rs:1449）。
- `knowledge_chat_tasks.status`：pending / running / completed / failed / cancelled（models.rs:5934-5935；worker debug_assert knowledge_task:1045-1048）。
- Step verdict：committed / noop / needs_manual / failed（knowledge_task:1178-1199）。
- `knowledge_chat_turns.status`：pending / applying / applied / discarded（chat.rs:775, 898, 980）。
- digest report `status`：ok / failed / partial；`latest_attempt_status` 另含 running；audit 侧另有 superseded（digest:1002, 1310-1335, 1370-1379）。
- digest 卡 kind 7 种 / severity 3 种 / suggestedAction 6 种（digest:812-829）；可派工 action 5 种（chat.rs:3142-3149，freeform 排除）。
- chunk `integrity_status` 观察值：needs_review / needs_human_audit / verified / rejected / missing_evidence（digest:252）/ null；chunk `status`：draft / active / rejected / archived。
- gap signal status：pending / dismissed / applied（sources_meta:340, 418, 453）。
- ingest source status：active / failing / disabled（写路径仅 active，sources_meta:1125-1134）。

### 4.3 预算 / 上限 / 超时
| 项 | 值 | 出处 |
|---|---|---|
| repair 每轮 | 4_000 token / 4 calls / ≤3 轮 | mod.rs:1390-1392 |
| chat 每轮 | 24_000 token / 4 calls；session ≤8 assistant 轮；followups ≤3 | mod.rs:1490-1495 |
| chat tool loop | mutation 1 轮、clarify 3 轮、toolCalls ≤6/轮、dispatch 5s、总 30s、失败连击 3 | chat.rs:1748-1749, 1731-1737 |
| chat 合约修复 | ≤2 次（1+1+2=4 与 4 calls 对齐） | chat.rs:1384, 3800-3814 |
| import 分块 | 单段阈值 3000 chars、目标 3000、硬上限 5000、并发 2、合约重试 3 | import.rs:195-202 |
| import 总量 | ≤200_000 chars、≤64 段、run budget 600_000 token、max_calls=段数×3 | import.rs:204-206, 708-716 |
| import checkpoint | 集合 import_job_segments、TTL 48h、schema v1、事务重试 5 | import.rs:207-209, 802 |
| import job TTL | 终态 expires_at=+24h（preview 同步/异步、apply 后同样） | import.rs:523, 1601; import_worker.rs:339-340 |
| import worker | tick 2s、claim 超时 600s、recovery ≥3 → failed、心跳 timeout/2 clamp[5,60] | config.rs:530-533; import_worker.rs:146, 454 |
| auto-verify | threshold 默认 7 [0,10]、limit 默认 50 [1,500]、抽样默认 0.3 硬下限 0.05、budget 240_000 token/100 calls | verify.rs:32-34, 217-224, 274-275 |
| vision | required-field 合约重试 3 | import.rs:1771 |
| knowledge task | step 8_000 token/4 calls、lease 120s、心跳 20s、bus 清理延迟 300s、worker tick 默认 30s（0=停） | knowledge_task:30-33, 53; config.rs:755-759 |
| digest | run budget 默认 24_000 token/8 calls、run_hour 默认 9、默认关停；health ≤200（进 prompt 80）、LLM 摘要桶 ≤6、evolution ≤50、卡 ≤50、title ≤60、summary ≤200、miss 样本 5×60 字 | config.rs:741-754; digest:304, 431, 567, 591, 857-868, 917 |
| completeness | 缓存 TTL 300s、verified 摘要 80、pending 40、chunk 报告 limit 500 | config.rs:497; catalog.rs:559, 598, 197 |
| inbox | limit 默认 24 [1,100]、chunk 扫描 200、pending_review 窗口 7d | digest_inbox.rs:375, 460, 466 |
| logs/analyze | hours 默认 24 max 72、limit 50、topChunks 8 | sources_meta:74, 110, 140 |
| 列表 limit | documents 200、chunks 300、revisions 20[1,200]、referrers 50、batch 100、usage 100、gap signals 100、operator memory 50[1,200]、task list 50[1,200]、import job list 50 | crud.rs:64, mod.rs:1269, wiki_edit.rs:232/886/924/985, sources_meta:41/355/815, chat.rs:3419-3421, import.rs:653 |
| 标签上限 | productTags ≤5、businessTopics ≤3（document 级同） | mod.rs:647-648, 700-701 |
| digest 派工 | selectedCards 1..=8、LLM 挑卡输入 ≤20 张 | chat.rs:3103-3107, 2558 |

### 4.4 红线落点清单（强制 draft + needs_review 的写入点）
| # | 位置 | 说明 |
|---|---|---|
| 1 | crud.rs:708-710 | 通用 create chunk 无条件收敛（+confidence 0） |
| 2 | import.rs:1549-1551 与 1554-1556 | import-apply 每 chunk 锚定前后各强制一次 |
| 3 | import.rs:2277-2279 与 2283-2285 | ingest（PDF/image/RSS/HTML fence）server-owned 字段接管，锚定前后各一次 |
| 4 | mod.rs:1230-1231 | apply_chunk_integrity 恒写 needs_review（confidence 90/45） |
| 5 | mod.rs:1158, 1176-1184 | preview integrity 报告恒 verified=0 / needs_review |
| 6 | chat.rs:2751-2752 | chat apply_create_chunk 强制（+source=Ai revision） |
| 7 | chat.rs:2927-2944 | chat apply_update_chunk 走 source=Ai → harness 强制（注释明示） |
| 8 | repair.rs:769-786, 837-844 | repair applied 走 source=Ai → draft+needs_review；then_verify 直接 400（676-680） |
| 9 | verify.rs:496, 681-686 | auto-verify 全类型 verified 强制降 needs_human_audit（AI 永不自动 verify） |
| 10 | wiki_edit.rs:346-350 | split 两子块 quote/anchors 清空 + draft+needs_review+0 |
| 11 | wiki_edit.rs:545-547 | merge target quote/anchors 清空（新主张需重新过 D2 才能 verify） |
| 12 | knowledge_task:595-605, 636-645, 1337-1348, 1448-1457 | task worker add_chunk/retag 复用 chat 落库内核（同 6/7 强制） |
| 13 | knowledge_task:1210-1274 | fix_chunk 只产草稿进 details，worker 不 apply |
| 14 | import.rs:2058-2064 | vision prompt 明文"所有 chunk 默认 needs_review，不要写 verified" |
| 15 | verify.rs:99-118 | 唯一进入 active+verified 的写点=人工 /verify（事务+版本绑定+D2 闸） |

---

## 5. 偏差与疑点

1. **crud.rs PUT 响应硬编码状态可能与库内实际不一致**：`update_operation_knowledge_chunk` 响应写死 `"status":"draft","integrityStatus":"needs_review"`（crud.rs:829-834），但它走 `apply_controlled_chunk_patch(source=Human)`；wiki_edit.rs:27-33 注释表明 harness 只对 **AI source** 强制 draft+needs_review。若 Human patch 对 active+verified chunk 不重置状态（`chunk_revisions.rs` 不在本次范围未核），该响应体是错误宣称。疑点，待读 harness 确认。
2. **has_anchor 口径不一致（B3 修复未全覆盖）**：B3 之后可引用性判定应统一走 `chunk_has_citable_anchor`（mod.rs:1351-1358），但仍有四处用裸 `!source_anchors.is_empty()`：crud.rs:547（review-queue source_orphan 分类）、verify.rs:398（auto-verify 的 has_source_anchor——影响有限，最终一律 needs_human_audit）、digest_inbox.rs:480（inbox anchors_missing）、catalog.rs:209（integrity 报告 anchorsMissing）。畸形 anchor（非空但缺 sourceQuote 键）在这些读点会被误判为"有锚"。verify 主闸（verify.rs:92-97）已是正确口径，故不能通过 verify，但报表/队列会漏报。
3. **chat apply_update_chunk 的字段映射表含已删死字段**：chat.rs:2872-2884 仍映射 `routing_card/safe_claims/forbidden_claims/evidence_items`；而 import.rs:2773-2783 的测试断言这些是"已删字段，不应再出现在抽取 prompt"。LLM patch 若携带这些键会被写进 chunk 文档（除非 harness 锁挡）。疑似演进遗留，未见其它路径清理。
4. **merge 后 target 的 integrity 状态未在本文件强制降级**：wiki_edit.rs:545-547 只清 quote/anchors（让 D2 挡住 re-verify），若 target 原本已是 active+verified，merge 事务本身是否把 integrity_status 降回 needs_review 取决于 harness 的 Merge op 行为（未核）。若不降级，会存在"verified 但无锚点"的中间态（会被 integrity 报告的 anchorsMissing 捕获，catalog.rs:209）。
5. **inbox 禁词测试与实际文案漂移**：digest_inbox.rs:724-731 测试的 candidates 文案（"缺 sourceQuote""sourceAnchors 为空"）与实现的实际文案（L522"缺原文出处"、L538"原文定位锚点为空"）不一致——测试锁的是旧文案副本，覆盖失真（不影响功能）。
6. **execute_step 的 add_chunk/retag/dismiss 分支疑似死路径**：run_claimed_task 对这三个 action 判定 is_mutating=true（knowledge_task:935）走 prepare/commit 两阶段，不会进 `execute_step`（其内 1276-1391/1392-1477/1531-1582 存在完整重复实现，与 prepare_mutating_step/commit_mutating_step_once 逻辑双份）。`execute_step` 是 pub（1202），可能仅被测试/旧调用触达。两份实现存在漂移风险（如 execute_step 的 dismiss 缺 account_id 过滤，1539-1545 filter 只有 workspace+cardId，而 commit 路径有 account 过滤 662-668）。
7. **chat_turn 的 turn_index 预分配空洞**：user+assistant 两个 index 预分配（chat.rs:201-210），pipeline 失败（如 budget 超限抛错）时 user turn 已写、assistant index 成为空洞。读路径按 turn_index 排序不受影响，但 CHAT_MAX_TURNS_PER_SESSION 按 assistant 计数（211-216）不受空洞影响，仅审计上 index 不连续。
8. **digest 同步合成无互斥**：digest_today 未命中即同步 `generate_today_digest`（digest_inbox.rs:62-71），多 admin 并发刷新会并行跑多次 4 路分析+compose LLM（各自烧 token），靠 attempt_generation 栅栏收敛可见性（digest:1077）——正确性有保证，成本无互斥。
9. **labels.rs 注释行号漂移**：labels.rs:2 说 block 状态来源在 "knowledge_digest/mod.rs:277-282"，实际在 mod.rs:364-369（代码演进后注释未同步）。
10. **ask 的 max_rounds clamp 未在本层核实**：sources_meta.rs:501 注释称 clamp 到 [1, MAX_ROUNDS=4]，但本文件只透传 `Option<i32>`（551, 651），实际 clamp 在 `agent::knowledge_agent`（不在本次范围）。
11. **import-apply 的 document 直接落 status="active"**（import.rs:1495）与 ingest 的 document status="active"（import.rs:2189）：文档层直接激活（目录可见），知识可用性靠 chunk 层 verified 闸控制（catalog 只列 verified chunk，catalog.rs:362）——是设计而非缺陷，但意味着"文档 active ≠ 内容可对客"。
12. **preview 候选 patch 的 `apply_chunk_integrity` 在 import-apply 时以 job.content（全文）锚定**（import.rs:1552），而候选 body 可能来自某一段——若 LLM 的 sourceQuote 恰在其它段中出现语义重复文本，锚点可能命中错误位置（fuzzy 匹配首个命中，mod.rs:1120）。低概率精度问题，非安全问题。

---

## 6. 覆盖自证

| 文件 | 行数（wc -l） | 阅读方式 |
|---|---|---|
| src/routes/knowledge/mod.rs | 2,411 | 分 4 段读全（1-700 / 700-1600 / 1600-2411 / 尾部 2409-2412 复核） |
| src/routes/knowledge/crud.rs | 1,011 | 分 2 段读全（1-550 / 550-1012） |
| src/routes/knowledge/verify.rs | 856 | 分 2 段读全（1-450 / 450-857） |
| src/routes/knowledge/import.rs | 3,013 | 分 6 段读全（1-560 / 560-1119 / 1119-1678 / 1678-2237 / 2237-2797 / 2797-3014） |
| src/routes/knowledge/repair.rs | 1,022 | 分 2 段读全（1-520 / 520-1023) |
| src/routes/knowledge/chat.rs | 4,132 | 分 7 段读全（1-560 / 560-1130 / 1130-1699 / 1699-2268 / 2268-2837 / 2837-3406 / 3406-4133） |
| src/routes/knowledge/catalog.rs | 1,006 | 分 2 段读全（1-510 / 510-1007） |
| src/routes/knowledge/wiki_edit.rs | 1,085 | 分 2 段读全（1-545 / 545-1086） |
| src/routes/knowledge/digest_inbox.rs | 818 | 分 2 段读全（1-450 / 450-819） |
| src/routes/knowledge/sources_meta.rs | 1,370 | 分 2 段读全（1-690 / 690-1370） |
| src/knowledge_task/mod.rs | 1,765 | 分 3 段读全（1-600 / 600-1199 / 1199-1765） |
| src/knowledge_digest/mod.rs | 1,853 | 分 3 段读全（1-620 / 620-1239 / 1239-1853） |
| src/knowledge_digest/labels.rs | 35 | 一次读全 |
| src/import_worker.rs | 485 | 一次读全 |
| **合计** | **20,862** | 全部逐行读完，无跳读 |

辅助核证（不在深读范围但按需查证）：`src/routes/mod.rs:480-725`（路由挂载）、`src/models.rs:1052-1090, 5934-5984`（status 闭集）、`src/config.rs:41-76, 308-322, 495-759`（config 默认值）。

---

## 追记：25 号交叉验证回写（2026-08-13，主会话执行）

25 号知识域交叉验证对本记录疑点的终裁：
- **疑点 1（crud PUT 响应硬编码）关闭——不成立**：patch 恒含 title 属敏感字段，Human-source 同样强制降级 draft+needs_review，响应硬编码与 harness 实际行为一致。
- **疑点 4（merge 后 target 状态）关闭——无中间态**：merge target 恒降级，不存在"verified 无锚"中间态。
- **疑点 10（clamp 位置）关闭**：clamp 在 `knowledge_agent.rs:672`。
- **疑点 6（knowledge_task execute_step 与两阶段提交双份漂移）确认成立**：execute_step 的 dismiss 缺 account 过滤，属真实死路径/漂移。
- 红线落点表补充：本记录 15 处漏列 rollback 强制降级（`chunk_revisions.rs:659-661`），合并后共 19 处见 25 号 §3。
