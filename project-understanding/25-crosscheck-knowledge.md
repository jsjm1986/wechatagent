# 知识域交叉验证（核证日期 2026-08-13）

> 审计对象：`07-knowledge-engine.md`（知识引擎）、`08-knowledge-routes-workers.md`（知识路由与 workers），关联 `09-llm-mcp-infra-prompts.md`（知识相关 prompt 段）与 `02-models-db.md`（知识模型段）。
> 方法：全部裁决**亲读当日工作区源码**（含未提交修改），不采信任何记录的转述。锚点核对以「行号精确 ∧ 内容断言与实现一致」为通过标准。

---

## 1. 接口一致性审计表

| # | 接口 | 07 侧断言 | 08 侧断言 | 判定 | 亲验依据（file:line，当日核对） |
|---|---|---|---|---|---|
| 1 | **chunk_revisions 的 op×source 矩阵 ↔ routes 写入点调用方式** | `apply_server_owned_lifecycle`：`requires_review = (source==Ai) ∨ (op∈{Patch,Split,Merge,Rollback} ∧ patch 命中 12 敏感字段)` → 强制 draft+needs_review+confidence 0；Archive→archived；Restore→active；Verify/Unverify/Reject 透传；Split/Merge 额外透传 superseded_by/previous_version_id；Create/Patch/Rollback（不碰敏感字段）无额外写（07 §2.9） | 各路由的 (op, source) 调用组合：crud=(Create/Archive, Human)、wiki_edit 全 Human、chat/repair=Ai、import/ingest=Imported、auto-verify=(Verify, Rule)（08 §2.2-2.8） | **一致** | 矩阵：`chunk_revisions.rs:201-245`（分支逐行比对）；`REVIEW_SENSITIVE_PATCH_FIELDS` 12 字段 `:174-187`；`chunk_patch_requires_review` `:189-196`。调用方抽验：`crud.rs:742-755`（Create,Human）、`wiki_edit.rs` 全部 10 处 `ProvenanceSource::Human`（grep :89/:115/:141/:373/:390/:574/:591/:752/:815/:1000）、`chat.rs:2777`/`:2937`（Ai）、`repair.rs:775`（Ai）、`import.rs:1577`（Imported）、`verify.rs:511`（Rule）、`escalation/ledger.rs:733`（PrincipalAuthorized，域外调用方）。每种组合代入矩阵后的落库状态与两份记录的描述处处吻合 |
| 2 | **catalog rebuild：enqueue（写侧）↔ worker 消费** | 写侧：unchanged(非 create)→不 enqueue；job `target_generation=0` 占位，enqueue 事务内父文档 `desired+1` CAS（0 值三态兼容）、父缺→`catalog_parent_missing` 失败整个变更；消费侧：claim 三态（queued 到期/processing 过期/failed legacy）、finalize 事务内 claim 验证→parent 缺 discarded→`applied>=target ∨ desired>target` superseded→CAS(desired==target ∧ applied==读值) 写快照+applied=target+$inc catalog_version（07 §2.8/2.9） | mod.rs 装配 + catalog.rs 读侧 `catalogFresh = persisted∧desired>0∧applied==desired`；wiki_edit 头注释"同事务推进父 catalog generation"（08 §1/2.7/2.8） | **一致**（两代次协议 desired/applied/target 三方闭环，两记录互相印证零冲突） | 写侧：`chunk_revisions.rs:399-426`（占位+跳过条件）、`:482-540`（`enqueue_catalog_job_with_session` 全文，`:498` parent missing、`:503-513` 0 值三态、`:530-533` CAS conflict）；消费侧：`catalog_rebuild.rs:366-427`（claim filter 三态+原子 find_one_and_update）、`:559-585`（≤3 重试，retryable=Transient∨generation_conflict）、`:587-732`（finalize 事务全文：`:601-603` claim_lost、`:616-641` discarded、`:643-671` superseded、`:673-697` CAS、`:698-721` done） |
| 3 | **"AI 永不 verify"红线两侧清单互补性** | harness 总闸（:207-214）+ rollback 恒强制 + PrincipalAuthorized 唯一例外（07 §2.9/§3.2） | 15 处路由/worker 落点表（08 §4.4） | **互补无矛盾、无重复计数**。08 的 15 处是"路由/worker 落点"视角，07 是"harness 内核"视角，两层正交；合并后发现 08 表**漏列 1 处**（rollback 的 harness 强制，见 §3 合并表 A2），07 §3.2 的归因表述对 Imported+Create 组合**略含糊**（该组合 harness 不强制，红线全靠 routes 显式赋值，见 §4 修正 7） | 合并总表全部 19 处逐一亲验，见 §3 |
| 4 | **knowledge_agent 工具集 ↔ ask 路由调用契约** | `AgentAction` 闭集 5 种（list_catalog/open_document/open_chunk/follow_relations/answer）；`CatalogFilter.include_unverified` 默认 false、"router 路径永远 false"；`AnswerRequest.max_rounds` clamp [1,4]（07 §2.1） | `ask_knowledge` 忽略 body workspace 用 session、`include_unverified=false` 硬编码、max_rounds 只透传（clamp 在 agent 侧，08 疑点 10 存疑）；SSE 版事件 trace/failed/answer/close（08 §2.10） | **一致**（08 疑点 10 本次核实关闭：clamp 确在 agent 侧） | `knowledge_agent.rs:227-257`（5 action 闭集 + OpenDocument `documentId` alias）、`:99-112`（include_unverified 默认 false + 注释）、`:672`（`req.max_rounds.unwrap_or(MAX_ROUNDS).clamp(1, MAX_ROUNDS)`）；`sources_meta.rs:531-534`（workspace 强制 session）、`:549`（include_unverified: false）、`:551`（max_rounds 透传）、`:501-502`（注释与实现相符）。响应 schema `:564-576` 与 `AnswerResult` 字段一致互补（非流式不含 cancelled，合理） |
| 5 | **import 的 fence 解析（block_parser）两侧描述** | 状态机 Outside/Inside、永不 Err、fence 起始 `---CHUNK:`…`---`、结束=trim_start 后整行恰等 `---END CHUNK---`、unsafe id `__unsafe__` 忽略态、5 类 warning、dedup 保留最后（07 §2.7） | 消费侧：`prepare_ingest` 调 `parse_chunk_blocks`，零块→fallback_blob 兜底 chunk，块级反序列化失败进 parse_warnings，全部非法→400；vision prompt 约束 fence 格式（08 §2.4） | **一致**（生产 fence 产物为单行 JSON，07 疑点 7 的缩进终止符边界与消费侧无冲突） | 解析侧：`block_parser.rs:77-191`（状态机全文）、`:194-202`（fence 起始）、`:128`（结束判定，`:93` trim_start——07 疑点 7 亲验成立：缩进的终止符行也生效，与 doc 注释 `:21-23`"必须行首"表述有出入）、`:205-219`（unsafe id 字符集）、`:222-260`（finalize：非 Object/三字段全空丢块）；消费侧：`import.rs:2198`（调用点）、`:2204-2219`（fallback_blob，title="{source} · 待切分 blob"、wiki_type=source）、`:2223-2230`（blockToChunkRequestError 进 warnings）、`:2254-2258`（全非法 400）；vision prompt `import.rs:2058`（"结束符必须是 ---END CHUNK---，不要写 ---END---"，与解析器判定精确对齐） |

**附带裁决（两记录间的隐性接口）**：
- **corpus 窗口错位（07 疑点 3）成立**：`cited_in_corpus` 与 200 条 priority 序预载窗求交（`knowledge_router.rs:752-762`），agent catalog 是 400 条相关度重排窗（`knowledge_agent.rs:81,1141-1170`）——cite 到 corpus 窗外的 verified chunk 会被降格 fallback（`:794-796`）。>200 verified 语料的 workspace 才触发，08 侧无相应描述（不属其范围），无矛盾。
- **`write_knowledge_usage_log` 的"fire-and-forget"注释失实（07 疑点 6）成立**：`knowledge_router.rs:1128` 注释 vs `:1134-1147` 循环内顺序 `await`（仅 `let _=` 吞错）；对照真 spawn `knowledge_agent.rs:317`。

---

## 2. 锚点抽验结果（通过率 + 失配详情）

抽验总计 **61 个锚点**（07 号 24、08 号 22、09 号 8、02 号 7），逐个以 Read/Grep 亲读源码核对行号与内容断言。

### 2.1 通过率汇总

| 记录 | 抽验数 | 完全通过 | 部分失配 | 通过率 |
|---|---|---|---|---|
| 07 | 24 | 24 | 0 | **100%** |
| 08 | 22 | 22 | 0 | **100%** |
| 09（知识 prompt 段） | 8 | 8 | 0 | **100%** |
| 02（知识模型段） | 7 | 5 | 2（注释过期类） | **71%**（锚点行号 7/7 准确） |
| 合计 | 61 | 59 | 2 | **96.7%** |

### 2.2 07 号抽验明细（24/24 通过）

| # | 锚点 | 断言 | 结果 |
|---|---|---|---|
| 1 | `knowledge_agent.rs:45` | MAX_ROUNDS=4 | ✅ |
| 2 | `:48/:51/:56/:63/:66` | PAGE_SIZE 30 / OPEN_BATCH 8 / FOLLOW 16 / PREFETCH 3 / SUMMARY 120 | ✅ 全部精确 |
| 3 | `:81` | CATALOG_CANDIDATE_CAP=400，注释 :79-80 声明 >400 尾部漏召边界 | ✅ |
| 4 | `:225-257` | AgentAction 闭集 5 种，OpenDocument 带 camelCase alias | ✅ |
| 5 | `:270-275` | SYSTEM_PROMPT：wiki 研究员/渐进披露/知识研判非对客话术/全自治 | ✅ 逐句吻合 |
| 6 | `:285-360, :317` | answer wrapper fire-and-forget spawn；:310-311 recall_miss 先同步置 query；:322 确定性落库；:328-339 followup 二次 merge；:340-356 low_yield→Split 提案 | ✅ 行号全部精确 |
| 7 | `:1252-1277` | open_chunk：resolve redirect（:1263）→ verified 硬门（:1271）→ 非 verified 静默 None | ✅ |
| 8 | `:1608-1646` | filter_answer_against_opened_chunks 四步强过滤 | ✅ |
| 9 | `:1648-1654` | eligible = verified ∧ id 非空 ∧ 非 contradiction | ✅ |
| 10 | `:1656-1688` | quote_is_chunk_evidence：子串 → anchor index 必填（:1674-1676）→ 互含（:1687） | ✅ |
| 11 | `:1781-1794` | wiki_type_priority 9 级权重序，None 按 entity=30 | ✅ |
| 12 | `:1814-1845` | RankKey 五元组全序；superseded ×0.1 / expired ×0.5（:1836）；降格不剔除 | ✅ |
| 13 | `cache.rs:26/:29/:31-43` | TTL 300s / MAX_ENTRIES 256 / CacheKey 10 字段 | ✅ |
| 14 | `knowledge_router.rs:36-86` | 预载 docs≤80（updated_at 序）+ verified chunks≤200（priority 序） | ✅ |
| 15 | `:1037-1049` | B2 红线：fallback 时只返 selected_knowledge_ids | ✅ |
| 16 | `:788` | FALLBACK_TOP_N=5 | ✅ |
| 17 | `knowledge_tools.rs:91-101` | 9 工具白名单（3 user-ops + 6 chat-only） | ✅ |
| 18 | `:51/:54/:57-58/:61/:64/:68/:104/:107/:109` | dispatch 5s / 同 kind ≤2 / limit 50-200 / query 200 / snippet 200 / redact 占位 / 每轮 ≤6 / 24h / 32 条 | ✅ 全部精确 |
| 19 | `chat_tool_loop.rs:36-44` | 总 30s / 连击 3 / context 8000 / trace 32 / max loops 4 | ✅ |
| 20 | `chunk_revisions.rs:66-77/:98-111/:125-138` | RevisionOp 10 个 / ProvenanceSource 5 个（含 principal_authorized）/ FromStr 白名单 | ✅ |
| 21 | `:174-187/:189-196/:201-245` | 12 敏感字段 / requires_review 判定 / 生命周期矩阵 | ✅ 矩阵逐分支比对一致 |
| 22 | `:447-476/:482-540` | 先 revision 后 CAS replace 后 enqueue；enqueue 事务内 desired+1 CAS | ✅ |
| 23 | `:621-680` | ROLLBACK_PRESERVED_FIELDS 13 个；:659-661 恒强制 draft+needs_review+0 | ✅ |
| 24 | `block_parser.rs / catalog_rebuild.rs / ingest_worker.rs / page_merge.rs` 常量与协议 | 见 §1 表 #2/#5；ingest 常量 :31-33（3/168h/120s）；page_merge 锁定 8 字段 :35-44、union 7 字段 :51-59、0.7 阈值 :62 | ✅ 全部精确 |

**07 号疑点亲验**（5/5 成立）：疑点 1（`DocEntry` 死代码，全仓 grep 仅 :135 一处定义零使用）✅；疑点 3（corpus 窗口错位，:752-762/:794）✅；疑点 6（顺序 await 非 fire-and-forget，:1134-1147）✅；疑点 7（缩进终止符生效，block_parser.rs:93+128 vs 注释 :21-23）✅；疑点 10（fallback 注释重复，router.rs:771-780 六行两遍）✅。

### 2.3 08 号抽验明细（22/22 通过）

| # | 锚点 | 断言 | 结果 |
|---|---|---|---|
| 1 | `crud.rs:708-710` | create 无条件 draft+needs_review+confidence 0，注释"only way into active+verified" | ✅ |
| 2 | `crud.rs:782-835` | PUT 收窄为受控 patch；响应硬编码 draft/needs_review | ✅（且本次裁决该硬编码**正确**，见 §4 修正 3） |
| 3 | `verify.rs:32-43` | 抽样率硬下限 0.05、默认 0.3、clamp 纯函数 | ✅ |
| 4 | `verify.rs:60-133` | verify 事务内核：版本绑定毫秒级（:83-85）+ D2 闸（:92-97） | ✅ |
| 5 | `verify.rs:99-118` | 唯一 active+verified 写点：op=Verify/source=Human/patch 五键 | ✅ |
| 6 | `verify.rs:480-524` | 判定链 + 抽样（:487-489）+ 全类型强制降级（:496）+ source=Rule 落库（:511） | ✅ |
| 7 | `verify.rs:654-686` | decide_auto_verify_status 四条件 ∧ / enforce_verified_needs_human_audit | ✅ |
| 8 | `import.rs:1495` | apply 的 document 直接 status="active" | ✅ |
| 9 | `import.rs:1545-1556` | 服务端重填 scope + 锚定前后各强制一次 draft+needs_review+0 | ✅ |
| 10 | `import.rs:2267-2286` | enforce_ingest_server_owned_fields：scope 接管 + 锚定前后双强制 | ✅ |
| 11 | `import.rs:195-209` | 3000/3000/5000/并发 2/合约 3/200k/64 段/600k budget/checkpoint 48h v1 | ✅ 全部精确 |
| 12 | `import.rs:2058-2064` | vision prompt 末句"所有 chunk 默认 needs_review，不要写 verified" | ✅ |
| 13 | `mod.rs:453-540` | patch 白名单 11 字段、双别名收敛、重复别名 400、归一到 snake_case | ✅ |
| 14 | `mod.rs:1156-1233` | preview 恒 verified=0（:1158）+ apply_chunk_integrity 恒 needs_review（:1230-1231，confidence 90/45）+ B3 触发条件（:1209-1216） | ✅ |
| 15 | `mod.rs:1324-1358` | D2 闸双谓词 + `chunk_verify_gate_reason_for` 唯一生产入口（谓词内算） | ✅ |
| 16 | `mod.rs:1390-1392/:1410-1442` | REPAIR 预算常量 / record_knowledge_run_started fail-closed（`?` 传播） | ✅ |
| 17 | `chat.rs:2738-2808` | apply_create_chunk：强制 draft+needs_review（:2751-2752）+ Create/Ai revision（:2771-2784） | ✅ |
| 18 | `chat.rs:2842-2968` | apply_update_chunk：OCC（:2862-2870）、quote 变更重算 anchors（:2918-2925）、Patch/Ai（:2931-2944 注释明示 harness 强制） | ✅ |
| 19 | `repair.rs:666-845` | then_verify=400（:676-680）、accepted 白名单（:706）、skipped 服务端推导（:707-713）、Patch/Ai（:769-786）、响应 draft/needs_review（:837-844） | ✅ |
| 20 | `wiki_edit.rs` split/merge | split 子块 quote/anchors 清空+强制 draft（:346-350）、先子后源（:333）、offset 开区间（:319-331）；merge 同域同账号（:502-506）、target quote/anchors 清空（:545-547）、locked 命中整体 400（:549-566）、源 archived+superseded_by（:592） | ✅ |
| 21 | `knowledge_task:30-33/:935/:595-645/:1210-1274` | STEP 预算 8000/4、lease 120s、心跳 20s；is_mutating 三 action；add_chunk/retag 复用 chat 内核；fix_chunk 只产草稿 worker 不 apply | ✅ |
| 22 | `sources_meta.rs:522-577/:990-995` | ask 契约（见 §1 表 #4）；ingest URL SSRF 走 `outbound_fetch::validate_public_http_url` | ✅ |

**08 号疑点裁决**（本次涉及 5 个）：疑点 1 **关闭**（响应正确，见 §4 修正 3）；疑点 3 **细化**（非纯遗留，见 §4 修正 6）；疑点 4 **关闭**（harness 恒降级，见 §4 修正 4）；疑点 6 **成立**（`:935` is_mutating 使 execute_step 的 add_chunk/retag/dismiss 分支生产不可达，双份实现漂移风险真实存在）；疑点 10 **关闭**（clamp 亲验在 `knowledge_agent.rs:672`）。

### 2.4 09 号抽验明细（8/8 通过，知识相关 prompt 段）

| # | 锚点 | 断言 | 结果 |
|---|---|---|---|
| 1 | `prompts.rs:1781-1798` | `knowledge.auto_verify` spec：layer=knowledge_integrity；"只有 sourceQuote 非空且 sourceAnchors 能定位来源时，才允许 verified"（:1791）；输出四键 schema | ✅ 逐字吻合 |
| 2 | `prompts.rs:2095-2141` | `knowledge.chat.intent`：6 分类闭集（:2128）；chunkId 引用优先（:2113）；不硬猜 freeform（:2118）；confidence≤0.6 freeform（:2119）；memoryKind 3 闭集（:2121-2124）；memoryContent ≤80 字提炼否则降级（:2133/:2140） | ✅ |
| 3 | `prompts.rs:2143-2195` | `knowledge.chat.draft_chunk`：sourceQuote 必须真实原文不允许编造（:2161）；缺原文→missingFields+追问；patch 禁 status/integrityStatus/sourceAnchors 系统字段（:2194）；追问 ≤3（:2163） | ✅ |
| 4 | `prompts.rs:2277-2311` | `knowledge.digest.compose`：kind 7/action 6/severity 3 闭集（:2294/:2298/:2299）；同源同目标 1 张 metric 求和（:2304）；≤50（:2305）；targetRefs 不在输入整卡丢弃（:2306）；禁"人工接管/人工介入/人工托管/takeover/hand-off"字面量（:2310） | ✅ |
| 5 | `prompts.rs:2313-2345` | `knowledge.digest.dispatch`：plannedSteps 五键 estimatedLlmCalls 1-3（:2334）；步数 ≤8 总 ≤12 超则合并 freeform（:2342）；naturalReply 禁"人工接管/接管"（:2344） | ✅ |
| 6 | `prompts.rs:2347-2370` | `knowledge.digest.summarize_logs`：≤50 字摘要+topBlockReason+sampleRunIds≤3；不泄原文只说类别频次（:2368）；禁"人工/接管/hand-off"（:2369） | ✅ |
| 7 | `llm_concurrency.rs:33-47` | 后台白名单含 `knowledge.digest.*`/`knowledge.import.*`/`knowledge.auto_verify`（starts_with 无尾点）/`knowledge.tags.*`；未知一律前台 | ✅ |
| 8 | `agent/mod.rs:828-836` | LLM 精确缓存白名单 4 key：`knowledge.import.preview`/`playbook.generator`/`playbook.optimizer`/`user.guide.preview`（均非 prompt_specs 模板 key，09 的"两套命名体系"观察成立） | ✅ |

### 2.5 02 号抽验明细（知识模型段，锚点位置 7/7 准确；内容断言 5 通过 + 2 注释过期失配）

| # | 锚点 | 断言 | 结果 |
|---|---|---|---|
| 1 | `models.rs:1841-1854` | chunk_type 4 类运营用途、与 wiki_type 正交、product_fact 仅 verified 可背书（:1846）、default fn 兜底 | ✅ |
| 2 | `models.rs:1859-1861` | default_chunk_type = "product_fact"（最保守） | ✅ |
| 3 | `models.rs:1865-1883` | ALLOWED_WIKI_TYPE 9 类 / ALLOWED_CHUNK_TYPE 4 类闭集 | ✅ |
| 4 | `models.rs:1903-1928` | B3 谓词：anchor_is_citable = 含非空 camelCase `sourceQuote` 键（:1917）；chunk_has_citable_anchor 任一可引用；"写读两侧单一真相源"注释与历史 bug 描述 | ✅ |
| 5 | `models.rs:2040/:2031-2032` | ChunkRevision.op 10 种闭集注释 | ✅ |
| 6 | `models.rs:2066-2096` | KnowledgeGapSignal："8 类 kind"（:2076）、severity "warning\|info"（:2085）、source "rule\|llm"（:2087）——02 引 db/mod.rs:384-386 同款 8 类清单 | ⚠️ **失配（注释过期类）**：02 忠实转录了 models.rs/db/mod.rs 的注释，但注释滞后于实现——结构 lint 实产 **9 类**（`gap_signals.rs:236-387`，含 `dangling_anchor` :346），在线另有 **3 类**（`knowledge_agent.rs:1932/:1966/:1985`）；severity 实际含 `error/high/medium`（`:1934/:1968/:1987` 亲验 high/high/medium）；source 实际含 `recall_trace`（`gap_signals.rs:744`）。07 §2.11 的 9+3 断言为准 |
| 7 | `models.rs:1993/:2053` | ChunkProvenance.source 与 ChunkRevision.source 注释 "∈{ai,human,rule,imported}"（4 种） | ⚠️ **失配（注释过期类）**：实现闭集 **5 种**（`chunk_revisions.rs:98-111` 含 PrincipalAuthorized，FromStr :125-138 接受 `principal_authorized`；生产调用点 `escalation/ledger.rs:733`）。02 另记 m055 raw 写 `lesson_promotion` 第 6 种值——说明存储层实为开放字符串、闭集只在 enum 层强制，此观察成立且有价值 |

> 两处失配均属"02 转录注释忠实、但未标记注释已过期"，非 02 自身杜撰；矛盾裁决以实现（07 亲验）为准。详见 §4 修正 1/2。

---

## 3. 红线落点合并总表（draft+needs_review 强制点，去重、每处亲验）

> 合并口径：08 §4.4 的 15 处 + 07 的 harness 层与例外描述 + 本次审计补充。分层呈现以消除"重复计数"歧义：**A 层是内核总闸，B-F 层是各写入路径的落点**（部分依赖 A 层生效，"强制方式"列标明依赖关系）。每处均于核证日亲读源码确认仍然成立。

### A. harness 内核层（`knowledge_wiki/chunk_revisions.rs`）——一切走 `apply_chunk_revision*` 写入的总闸

| # | file:line | 触发路径 | 强制方式 | 亲验 |
|---|---|---|---|---|
| A1 | `chunk_revisions.rs:207-214` | 任何 `source=Ai` 的 op；或 `op∈{Patch,Split,Merge,Rollback}` 且 patch 顶层键命中 12 敏感字段（`:174-187`） | `apply_server_owned_lifecycle` 无条件覆写 `status=draft + integrity_status=needs_review + confidence_score=0` 后直接 return（AI 的 Verify op 也被压回）；在 `enforce_locked_fields` 之后执行（`:338→:347`），运营锁锁不住降级 | ✅ |
| A2 | `chunk_revisions.rs:659-661` | rollback（`rollback_chunk_revision_with_session` :705-810 → `build_snapshot_rollback`） | 快照恢复后无条件插入 draft+needs_review+0（"回滚永不自动回到 verified"）；且先按当前锁 enforce（:656-658） | ✅ **08 §4.4 表漏列此处**（08 §2.8 正文有描述），合并表补入 |

**唯一例外**：`ProvenanceSource::PrincipalAuthorized`（`chunk_revisions.rs:107-110`）不落 A1 的 source 分支，可直接带 verified——语义是领导真人裁决（视同 Human 权威家族）。全仓唯一生产调用点：`src/agent/escalation/ledger.rs:733`（grep 亲验，仅此一处 + 本文件 as_str/FromStr/测试）。**任何知识写路径改动如引入新的 PrincipalAuthorized 调用点，须以同等真人裁决语义论证。**

### B. routes/worker 层显式赋值（防御纵深；其中 ★ 标记的 Imported+Create 组合 harness 不强制，红线**完全依赖**这层）

| # | file:line | 触发路径 | 强制方式 | 亲验 |
|---|---|---|---|---|
| B1 | `crud.rs:708-710` | `POST /operation-knowledge/chunks` 手工新建 | handler 无条件改写 payload 三字段后才入库（+Create/Human revision，A1 不触发，红线靠本处） | ✅ |
| B2 ★ | `import.rs:1549-1551` 与 `:1554-1556` | import-apply 每个选中 candidate | 锚定（`apply_chunk_integrity` :1552）**前后各强制一次**；Create/Imported revision（:1571-1587）harness 不强制 | ✅ |
| B3 ★ | `import.rs:2277-2279` 与 `:2283-2285` | `ingest_chunked_text*`（PDF / 图片 vision / RSS / HTML fence / 手动 markdown） | `enforce_ingest_server_owned_fields`：scope 服务端接管（:2273-2276）+ 锚定前后双强制；ingest_worker 事务落库复用（`ingest_worker.rs` finalize → 本函数） | ✅ |
| B4 | `chat.rs:2751-2752` | chat `apply_create_chunk(_with_session)`（含 knowledge_task add_chunk 复用 :595-605） | handler 显式强制 + Create/**Ai** revision（:2771-2784）→ A1 双保险 | ✅ |
| B5 | `wiki_edit.rs:348-350` | split 两个子块 | 子块 insert 前显式 draft+needs_review+0，且 `source_quote=None`+anchors 清空（:346-347，"原始证据不能假定证明子块"）；子块 Create/Human revision 不触发 A1，红线靠本处 | ✅ |

### C. 归一化内核恒定写（`routes/knowledge/mod.rs`）

| # | file:line | 触发路径 | 强制方式 | 亲验 |
|---|---|---|---|---|
| C1 | `mod.rs:1230-1231` | `apply_chunk_integrity`（import-apply/ingest 每 chunk 必经） | 恒写 `integrity_status=needs_review`、confidence 90（有锚）/45（无锚）——"Anchoring establishes provenance location only"（:1221-1222） | ✅ |
| C2 | `mod.rs:1158` + `:1176-1184` | import preview 的 integrity 报告 | `verified = 0` 恒定；anchor 命中只保留 anchors+confidence 90 作审计线索，integrityStatus 恒 needs_review | ✅ |

### D. 判定层降级（auto-verify）

| # | file:line | 触发路径 | 强制方式 | 亲验 |
|---|---|---|---|---|
| D1 | `verify.rs:496` + `:681-686` | `POST /operation-knowledge/auto-verify` 批量预审 | `enforce_verified_needs_human_audit`：verified → needs_human_audit **对全部 chunk_type 生效**（auto-verify 退化为预审分诊）；落库走 Verify/**Rule**（:505-524，A1 不触发——Rule+Verify 走透传分支，透传的已是降级后状态）；叠加抽样（:487-489）与四条件判定（:654-672） | ✅ |

### E. 结构性拒绝口 / 依赖 A1 的落点

| # | file:line | 触发路径 | 强制方式 | 亲验 |
|---|---|---|---|---|
| E1 | `repair.rs:676-680` | repair apply 的 `then_verify=true` | 直接 400："repair apply cannot verify knowledge; use the dedicated verify route" | ✅ |
| E2 | `repair.rs:769-786` | repair apply 落库 | Patch/**Ai** → A1 强制；响应硬编码 draft/needs_review（:837-844）与 A1 行为一致 | ✅ |
| E3 | `chat.rs:2927-2944` | chat `apply_update_chunk(_with_session)`（含 knowledge_task retag 复用 :636-645） | Patch/**Ai** → A1 强制（注释明示） | ✅ |
| E4 | `wiki_edit.rs:545-547` | merge 的 target | patch 无条件插入 `source_quote=""` + `source_anchors=[]`（敏感字段）→ **`chunk_patch_requires_review` 恒 true → A1 恒降级 target**（本次裁决关闭 08 疑点 4：不存在"verified 但无锚"中间态）；locked 命中则整体 400（:549-566） | ✅ |
| E5 | `knowledge_task:935` + `:1210-1274` | digest 派工 fix_chunk step | fix_chunk 归为非 mutating，只调 `propose_chunk_repair_inner` 产修复草稿进 details，**worker 不 apply**（:1213 注释红线）；LLM 失败 NeedsManual fail-soft | ✅ |
| E6 | crud PUT / wiki_edit patch（`crud.rs:819-827`、`wiki_edit.rs:76-101`） | Human 内容编辑 | 走 `apply_controlled_chunk_patch` → Patch/Human；patch 经 `normalize_editable_chunk_patch` 归一为 snake_case，凡含 11 白名单中任一内容字段（title 等，多为敏感字段）即触发 A1 降级——**已 verified 的 chunk 被 Human 编辑同样打回 draft+needs_review**（crud PUT 恒含 title，恒降级，本次裁决关闭 08 疑点 1） | ✅ |

### F. prompt 层约束（软层，不作为强制依据、仅降低 LLM 输出噪声）

| # | file:line | 内容 | 亲验 |
|---|---|---|---|
| F1 | `import.rs:2058-2064` | vision prompt："所有 chunk 默认 needs_review，不要写 verified" | ✅ |
| F2 | `prompts.rs:1791` | auto_verify prompt："只有 sourceQuote 非空且 sourceAnchors 能定位来源时，才允许 verified"（LLM 即使违规输出也被 D1 硬降级） | ✅ |

### G. 唯一晋升口（对照面）

| # | file:line | 内容 | 亲验 |
|---|---|---|---|
| G1 | `verify.rs:99-118`（内核 `:60-133`） | **全系统唯一进入 active+verified 的写点**：事务 + `expected_updated_at` 毫秒级版本绑定（:83-85）+ D2 证据闸 `chunk_verify_gate_reason_for`（:92-97，谓词函数内算防传错）+ Verify/**Human**；batch-verify（`wiki_edit.rs:916-965`）逐条复用同一内核 | ✅ |
| G2 | `escalation/ledger.rs:714-733` | PrincipalAuthorized 例外通道（领导真人裁决知识沉淀，Chunk 与 create revision 原子落地）——非 AI 自动 verify，域外（05 号记录范围），此处仅登记为红线总表的完备性边界 | ✅（调用点存在性 grep 亲验） |

**总计：强制/拒绝落点 17 处（A2+B5+C2+D1+E6…全部去重后）+ 唯一晋升口 1 处 + 真人例外通道 1 处。**
与 08 §4.4 的映射：08#1→B1、08#2→B2、08#3→B3、08#4→C1、08#5→C2、08#6→B4、08#7→E3、08#8→E1+E2、08#9→D1、08#10→B5、08#11→E4、08#12→B4/E3（task 复用）、08#13→E5、08#14→F1、08#15→G1。**08 的 15 处全部保留且亲验成立，无一处重复计数；07 补入 A1/A2 两处内核闸与 G2 例外，合并后无遗漏、无矛盾。**

> **安全底单用法**：任何知识写路径改动，先对照本表——(1) 新写入点必须落在 B 层显式赋值或走 A1 可达的 (op,source) 组合；(2) 触碰 A1 的 12 敏感字段清单（`chunk_revisions.rs:174-187`）或 `REVIEW_SENSITIVE` 判定（:189-196）需重跑本表全部 17 处验证；(3) 新增 PrincipalAuthorized 调用点需真人裁决语义论证；(4) Imported+Create 组合（★）没有 harness 兜底，删掉 B2/B3 的显式赋值 = 红线破洞。

---

## 4. 需回写修正清单

| # | 目标记录 | 修正内容 | 依据 |
|---|---|---|---|
| 1 | **02 号 §2.15/§1.6**（KnowledgeGapSignal 段） | "8 类 kind / severity warning\|info / source rule\|llm" 需加**注释过期**标记：这是 models.rs:2076,2085,2087 与 db/mod.rs:384-386 注释的忠实转录，但实现为结构 9 类（+`dangling_anchor`，gap_signals.rs:346）+ 在线 3 类（recall_miss/recall_low_yield/citation_format_rejected）；severity 实际另有 error/high/medium；source 实际另有 recall_trace（gap_signals.rs:744）。以 07 §2.11 为准 | 本审计 §2.5#6 |
| 2 | **02 号 §2.15**（ChunkProvenance/ChunkRevision.source 段） | source 注释"∈{ai,human,rule,imported}"（models.rs:1993/:2053）已过期：实现闭集 5 种（chunk_revisions.rs:98-111 含 principal_authorized）。02 已注意到 m055 的 lesson_promotion 第 6 种 raw 值，应一并注明"闭集只在 enum 层强制、注释滞后" | 本审计 §2.5#7 |
| 3 | **08 号 §5 疑点 1** | **可关闭**：crud PUT 响应硬编码 draft/needs_review 是正确的——`normalize_editable_chunk_patch` 归一为 snake_case（mod.rs:459-476），PUT 的 patch 恒含 `title`（crud.rs:794 无条件插入），title ∈ REVIEW_SENSITIVE_PATCH_FIELDS → `chunk_patch_requires_review(Patch, …)` 恒 true → A1 恒降级，Human patch 同样打回 | 本审计 §3 E6 |
| 4 | **08 号 §5 疑点 4** | **可关闭**：merge 的 target patch 无条件含 `source_quote`/`source_anchors`（wiki_edit.rs:546-547），两者均在敏感字段清单 → harness 恒降级 target 为 draft+needs_review+0。不存在"verified 但无锚点"中间态 | 本审计 §3 E4 |
| 5 | **08 号 §5 疑点 10** | **可关闭**：max_rounds clamp 亲验在 `knowledge_agent.rs:672`（`unwrap_or(MAX_ROUNDS).clamp(1, MAX_ROUNDS)`），sources_meta.rs:501 注释准确 | 本审计 §1 表 #4 |
| 6 | **08 号 §5 疑点 3** | **表述细化**：chat.rs:2872-2884 映射表中的 `routing_card/safe_claims/forbidden_claims/evidence_items` **不是纯演进遗留**——`knowledge.chat.draft_chunk` prompt（prompts.rs:2171-2178）仍要求 LLM 输出这些字段，chat 链路两端自洽（LLM 产出 → 映射写入 raw 字段，typed model 无这些字段但无 deny_unknown_fields 保留）。真正的不一致是 **chat 链路与 import 链路对该字段族的处置相反**（import.rs 测试断言其为已删字段）。建议按"跨链路口径分叉"重新归档该疑点 | 本审计 §2.4#3 |
| 7 | **07 号 §3.2 步骤 1** | 归因精化："所有 AI/导入产物恒 draft+needs_review（chunk_revisions.rs:17-18 + apply_server_owned_lifecycle :207-214）"对 **Imported+Create** 组合不精确——该组合 harness 不强制（op=Create 不在敏感 op 集、source≠Ai），红线由 routes 侧显式赋值承担（import.rs:1549-1556 / :2277-2285）。07 §2.9 的矩阵本身精确，仅此叙述归因含糊 | 本审计 §3 B2/B3 ★ |
| 8 | （新发现，代码注释非记录错误）`chunk_revisions.rs:12` | 头注释"7 个动作：create/patch/split/merge/archive/restore/rollback"滞后——RevisionOp 实际 10 个（+verify/unverify/reject，:66-77）。07 已正确列 10 个；如做代码清理可同步该注释 | 本审计 §2.2#20 |
| 9 | （新发现，同上）`models.rs:2062-2063` | KnowledgeGapSignal 的 doc 注释（8 类清单）与修正 1 同源滞后；如做代码清理与修正 1 一并处理 | 本审计 §2.5#6 |

---

## 5. 综合可信度评估

| 记录 | 抽验通过率 | 疑点核验 | 评估 |
|---|---|---|---|
| **07（知识引擎）** | 24/24（100%），行号最大偏差 ≤1 行（注释起始行计法差异） | 5 个疑点抽验全部亲验成立（DocEntry 死代码 / corpus 窗口错位 / 顺序 await / 缩进终止符 / 注释重复），无一虚报 | **极高**。矩阵/协议/常量级断言与实现零偏差；疑点部分甚至比代码注释更准确（如疑点 2/6/7 均为"注释与实现不符"类且判对方向）。唯一精化点是 §3.2 叙述归因（修正 7），不影响任何结论 |
| **08（知识路由与 workers）** | 22/22（100%） | 6 个疑点中 3 个本次裁决关闭（1/4/10——均为"存疑但实际安全"方向，谨慎态度值得保留）、疑点 6 成立、疑点 3 需细化、其余（2/5/7/8/9/11/12）未在本次抽验范围但与红线无冲突 | **极高**。红线 15 处落点全部亲验成立且行号精确；"待读 harness 确认"类疑点的开放式表述诚实，本次全部补上裁决 |
| **09（知识 prompt 段）** | 8/8（100%） | 涉及知识域的部分无疑点待裁决 | **极高**。prompt 正文的闭集、字数上限、禁词清单逐字吻合源码 |
| **02（知识模型段）** | 锚点位置 7/7 准确；内容断言 5/7 无保留通过，2/7 为"注释忠实转录但注释已过期"类失配 | — | **高**。失配均非杜撰而是未标记上游注释滞后（02 的核证方法偏"转录声明"，07 偏"核对实现"——方法论差异所致）。按修正 1/2 回写后即与 07 完全一致 |

**跨记录一致性总结**：5 项接口审计全部判定一致/互补，未发现任何一处两份记录对同一事实给出不可调和描述的真矛盾。唯一的表面矛盾（gap signal kind 计数 8 vs 9+3、provenance source 4 vs 5）根因都是**源码注释滞后于实现**，且 07 号已在其疑点 8 预判了同类问题。合并后的红线总表（§3）经逐处亲验可作为知识写路径改动的安全底单。

---

## 6. 覆盖自证

本次审计亲读的源码（全部经 Read 工具直读当日工作区，Grep 仅用于定位与存在性证明）：

| 文件 | 读取范围 | 用途 |
|---|---|---|
| `src/knowledge_wiki/chunk_revisions.rs` | :1-260、:300-600、:621-740（三段） | 矩阵/闭集/prepare/persist/enqueue/rollback/PrincipalAuthorized |
| `src/knowledge_wiki/block_parser.rs` | 全文 468 行 | 接口 #5、07 §2.7 全部断言 |
| `src/knowledge_wiki/catalog_rebuild.rs` | :30-140、:360-540、:555-740（三段） | 接口 #2 消费侧、常量、claim/finalize 协议 |
| `src/knowledge_wiki/ingest_worker.rs` | :28-52 | 常量、loop 门 |
| `src/knowledge_wiki/page_merge.rs` | :28-72 | 锁定 8 字段/union 7 字段/0.7 阈值 |
| `src/knowledge_wiki/gap_signals.rs` | Grep 定位 :236-387（9 类 kind 构造点）、:744、:2138-2144 | kind/severity/source 实现闭集裁决 |
| `src/agent/knowledge_agent.rs` | :40-130、:225-360、:1246-1290、:1600-1694、:1775-1850 + Grep（DocEntry/max_rounds/kind-severity） | 常量/action 闭集/verified 门/证据校验/rank/疑点 1、在线信号 severity |
| `src/agent/knowledge_agent/cache.rs` | :20-50 | TTL/容量/CacheKey |
| `src/agent/knowledge_router.rs` | :30-90、:749-813、:1026-1070、:1086-1150 | 预载/fallback/B2/usage log（疑点 3/6/10 亲验） |
| `src/agent/knowledge_tools.rs` | :45-130 | 9 工具白名单与全部常量 |
| `src/agent/chat_tool_loop.rs` | :30-54 | 5 常量 |
| `src/routes/knowledge/mod.rs` | :450-600、:1150-1250、:1310-1370、:1386-1445 | patch 白名单/integrity 内核/D2 闸/REPAIR 常量/audit fail-closed |
| `src/routes/knowledge/crud.rs` | :695-840 | create/PUT 落点与响应 |
| `src/routes/knowledge/verify.rs` | :28-208、:470-540、:640-690 | verify 内核/auto-verify 判定链/纯函数 |
| `src/routes/knowledge/import.rs` | :193-212、:1480-1594、:2042-2086、:2150-2300 | 常量/apply 强制/vision prompt/prepare_ingest+enforce |
| `src/routes/knowledge/repair.rs` | :660-845 | then_verify 400/apply 事务/响应 |
| `src/routes/knowledge/chat.rs` | :2700-2810、:2837-2968 | apply_create/apply_update 内核（疑点 3 映射表亲验） |
| `src/routes/knowledge/wiki_edit.rs` | :278-452、:456-645 + Grep（10 处 Human） | split/merge 全文、source 调用方式 |
| `src/routes/knowledge/sources_meta.rs` | :495-625、:980-1010 | ask/ask-stream 契约、SSRF |
| `src/knowledge_task/mod.rs` | :25-40、:560-660、:925-995、:1202-1280 | 常量/两阶段 commit/is_mutating（疑点 6 裁决）/fix_chunk 红线 |
| `src/models.rs` | :1841-2160 + :1990-2100（两段） | 02 号全部抽验锚点 |
| `src/db/mod.rs` | :370-400 | gap signal accessor 注释裁决 |
| `src/prompts.rs` | :1781-1812、:2095-2196、:2277-2371 | 09 号 6 个知识 prompt 全文 |
| `src/llm_concurrency.rs` | :25-55 | 后台白名单 |
| `src/agent/mod.rs` | :815-858 | 缓存白名单 |
| `src/agent/escalation/ledger.rs` | Grep :714/:733 | PrincipalAuthorized 唯一生产调用点存在性 |

覆盖声明：接口一致性 5 项全部双侧亲验裁决；锚点抽验 61 个（超出任务要求的 15+15+8+5=43），覆盖 07/08 的红线、协议、常量、闭集四类断言与两记录全部涉知识域疑点中的 10 个；红线合并总表 19 处（17 强制/拒绝 + 1 晋升口 + 1 例外通道）每处均有当日 file:line 亲验。未重验项：08 疑点 2/5/7/8/9/11/12（不涉红线冲突）、07 的性能观察类疑点 11（规模假设类，无需裁决）、feedback_worker/lessons_learned/reviewer_stats/structural_proposals 的内部细节（07 单侧覆盖、08 无对应断言、非接口交叉点）。
