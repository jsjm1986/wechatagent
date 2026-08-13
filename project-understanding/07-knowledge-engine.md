# 知识引擎深读记录（核证日期 2026-08-13）

> 覆盖范围：`src/agent/knowledge_agent.rs`（3048 行）+ `src/agent/knowledge_agent/cache.rs`（377 行）+ `src/agent/knowledge_router.rs`（1741 行）+ `src/agent/knowledge_tools.rs`（1784 行）+ `src/agent/chat_tool_loop.rs`（719 行）+ `src/knowledge_wiki/` 全部 11 个文件（共 8095 行）。全部逐行读完，断言均附 file:line（当日亲验）。

---

## 1. 模块地图

```
┌─ 生产召回面（user-ops，只读，verified-only）─────────────────────────────┐
│ webhooks/tasks → gateway → knowledge_router.rs                            │
│   ├─ load_operation_knowledge        预载 corpus（docs≤80 + verified chunks≤200）
│   ├─ knowledge_prefilter_requires_agent  零相关信号 → 跳过整个多轮推理     │
│   ├─ route_operation_knowledge_inner  拼 query → 调 knowledge_agent::answer│
│   │     └─ fallback_rank（cited 空时静态回填，navigation_only）            │
│   ├─ route_used_knowledge_ids        B2 红线：fallback id 不作产品背书证据 │
│   └─ write_knowledge_usage_log       审计 + record_chunk_hit 回写          │
│                                                                            │
│ knowledge_agent.rs（多轮渐进披露 agent，≤4 轮 LLM）                        │
│   ├─ answer / answer_streaming / answer_read_only                          │
│   ├─ 工具：list_catalog / open_document / open_chunk / follow_relations    │
│   ├─ rank_key（bigram 相关度 × trust/recency）+ CATALOG_CANDIDATE_CAP=400  │
│   ├─ filter_answer_against_opened_chunks（cite⊆opened ∧ quote 锚点强校验） │
│   ├─ classify_recall_outcome → gap_signals::persist_recall_signal（在线闭环）
│   └─ cache.rs（进程级答案缓存，corpus 签名失效）                           │
└────────────────────────────────────────────────────────────────────────────┘
┌─ 管理台 chat 面（知识工作站，可读全状态、永不写库/outbox/mcp）─────────────┐
│ routes/knowledge/chat.rs → chat_tool_loop.rs（≤4 轮循环，30s 硬超时）      │
│   └─ knowledge_tools.rs::dispatch_chat_tool_call（9 工具白名单，5s/次）    │
└────────────────────────────────────────────────────────────────────────────┘
┌─ 写入路径（wiki 式保护，"AI 永不自动 verify"红线）──────────────────────┐
│ page_merge.rs        纯函数层：锁定字段 / 数组 union / 70% 阈值 / 规范 hash │
│ chunk_revisions.rs   apply_chunk_revision 状态机（10 op × 5 source，事务） │
│ block_parser.rs      LLM fence 分块输出解析（导入用）                       │
│ ingest_worker.rs     RSS/HTML 自动 ingest（条件 GET，恒 draft+needs_review）│
│ catalog_rebuild.rs   文档级 catalog 异步重建（generation CAS + lease）      │
└────────────────────────────────────────────────────────────────────────────┘
┌─ 反馈闭环（离线 worker）───────────────────────────────────────────────────┐
│ feedback_worker.rs   每 ws lease 串行跑：usage/confidence → lint → sweep    │
│                      → lessons_learned → reviewer_stats                     │
│ gap_signals.rs       9 类结构信号 + 3 类在线召回信号 + dynamic_confidence   │
│ lessons_learned.rs   success/misjudge/blocked 三类模式聚合                  │
│ reviewer_stats.rs    reviewer 通过率/误判率度量                             │
│ structural_proposals.rs 结构化写意图提案（恒 pending_review，生产未接线）   │
└────────────────────────────────────────────────────────────────────────────┘
```

隔离红线：`knowledge_wiki/mod.rs:9-11` 明文禁止本子系统引用 `agent::gateway/outbox`、`mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`；`knowledge_agent.rs:13-15` 同样声明零耦合（可独立供 `/api/knowledge/ask` 使用）。

---

## 2. 逐文件深读

### 2.1 `src/agent/knowledge_agent.rs`（3048 行）—— 渐进披露知识 agent 主体

#### 常量与协议

| 常量 | 值 | 位置 | 语义 |
|---|---|---|---|
| `MAX_ROUNDS` | 4 | :45 | LLM 决策轮硬上限；第 5 轮强制兜底。与 `RunBudget` 互不替代（budget 用尽更早跳出） |
| `CATALOG_PAGE_SIZE` | 30 | :48 | 单次 catalog 返回摘要上限 |
| `OPEN_CHUNK_BATCH` | 8 | :51 | 一次 `open_chunk` 最多展开条数 |
| `FOLLOW_RELATIONS_LIMIT` | 16 | :56 | follow 单/双跳最大收集数（= PAGE_SIZE/2） |
| `FOLLOW_PREFETCH_BODIES` | 3 | :63 | follow 直接把前 3 条关联目标【完整正文】载入 opened（当轮可 cite） |
| `CATALOG_SUMMARY_CHARS` | 120 | :66 | catalog 摘要按 char 截断（CJK 友好） |
| `CATALOG_CANDIDATE_CAP` | 400 | :81 | query 非空时 DB 候选窗；修 #619「高相关但静态置信度排 31+ 被截掉」的漏召。>400 条 verified 的尾部漏召是已知边界（注释 :79-80） |
| `MAX_REDIRECT_HOPS` | 8 | :1176 | superseded_by 版本链最大跳数 |
| `LOW_YIELD_OPENED_MIN` / `LOW_YIELD_CITED_MAX` | 3 / 1 | :1868-1869 | recall_low_yield 判定阈值 |

数据结构：`AnswerRequest`（:84-95，`max_rounds` clamp 到 `[1,4]`）；`CatalogFilter`（:97-112，`include_unverified` 默认 false=verified-only，router 路径永远 false）；`CatalogEntry`（:114-127）；`DocEntry`（:133-145，文档级导航卡片——**仅定义，全文件无构造点，见 §5 疑点 1**）；`ChunkFull`（:147-166，含 D3(a) `relation_role`：`Some("contradiction")` 表示经 contradicts 关系拉入）；`SourceQuoteCitation`（:176-183）；`AnswerResult`（:185-198，`truncated`/`cancelled` 不互斥）；`TraceEvent`（:206-222，SSE 事件 4 态：Step/Token/Failed/Final；转 relaxed extjson 避免 `{"$numberInt":"3"}`）。

`AgentAction`（:225-257）LLM 回包协议闭集 5 种：`list_catalog{filter}` / `open_chunk{ids}` / `open_document{document_id}`（camelCase alias） / `follow_relations{chunk_id,depth}` / `answer{cited_chunk_ids,source_quotes,answer}`。`SYSTEM_PROMPT`（:270-275）：wiki 研究员角色 + 渐进披露 + 「answer 是给内部回复 Agent 的知识研判，不是对客话术」+ 全自治定位（内部流程分工只转述事实边界）。

#### `answer`（:285-360）—— 生产入口 wrapper

流程：捕获 `workspace_id`/`query` → `answer_inner(state, req, None, None)` → 对结果跑 `classify_recall_outcome`，得到候选信号则 **fire-and-forget `tokio::spawn`**（:317）：
1. 先确定性 `persist_recall_signal` 落库（:322，恒在、零 LLM 依赖；`recall_miss` 时 `search_queries` 已在 spawn 前同步置为原始 query :310-311）；
2. `recall_miss` 再 fire-and-forget 生成一句 LLM 追问（`generate_gap_followup_question` :328-339），成功且非空且 ≠ 原 query 时以二次 merge-update 并入同一信号（dedup_key 同 → `$addToSet` 并集）；
3. `recall_low_yield` → `propose_structural_change(Split, affected_chunk_ids, source="recall_trace")`（:340-356，只入队、绝不应用）。

错误路径：spawn 内所有失败只 `tracing::warn!`，不影响已返回的 result。`answer_read_only`（:366-371）绕过该 wrapper（shadow/simulation 用，LLM 成本日志仍在）。

`generate_gap_followup_question`（:376-401）：字面量 system/user 走 `generate_agent_json`，prompt_key=`knowledge.gap.followup`；任何错误返回 None。

#### 流式管线（:413-664）

`answer_streaming`（:413-428）：注入 `tx`/`cancel` 调 `answer_inner`，跑完补发一帧 `TraceEvent::Final`（cache-hit/truncated/cancelled 等无 token 路径靠这帧兜底渲染）。

`AnswerStreamer`（:440-490）：增量抽取模型**原始 JSON 文本流**中顶层 `answer` 字符串值。`push`（:452-489）：未定位时在 buf 找锚点（`locate_answer_value_start`），找不到则保留尾部 64 字节窗口（char 边界对齐 :472-475）防无界增长；定位后逐字节解码直到未转义闭合引号（`done=true` 后忽略一切输入）。
- `locate_answer_value_start`（:497-529）：朴素子串定位 `"answer"` → 跳空白 → 冒号 → 起始引号。**注意：doc 注释 :437 声称"用 depth 计大括号层级忽略嵌套同名键"，实现里没有任何 depth 计数**（见 §5 疑点 2）。
- `decode_json_string_body`（:539-635）：处理 `\" \\ \/ \n \t \r \b \f \uXXXX`；`\uXXXX` 不足 4 hex / 半个转义 / 半个多字节 char → 不消费留待下次；非法 codepoint → U+FFFD；不处理代理对拼接（注释 :598-599 声明罕见于中文正文）；非法转义原样保留反斜杠。`utf8_char_len`（:638-650）按首字节推断 1-4，非法首字节按 1 推进防卡死。

`push_trace`（:654-664）：一条 trace doc 同时进 `tool_trace` 与 `tx`（Step 事件，relaxed extjson）。

#### `answer_inner`（:666-1096）—— 主循环（每轮细节）

1. **round 0 预取**：`list_catalog(filter, Some(query))`（:677-684），trace `{tool:"list_catalog", filter, returned}`。catalog 空 → 立即返回 `"知识库无相关内容。"`，`rounds_used=0, truncated=false`（:695-705）。
2. **cache 查询**（:712-747）：条件 `current_run_mode() != "shadow" && !is_cancelled`。CacheKey = workspace + account + provider(id/model/generation，经 `current_provider_cache_identity` :2084-2105 取 llm_registry snapshot，无 registry 时 `injected`/config.model/gen 0) + `prompt_pack_version`（AppState 原子量）+ `normalized_filter_key` + `normalize_query` + `visible_corpus_signature` + `max_rounds`。命中 → push `cache_hit` trace、替换 tool_trace 返回。
3. **循环 `for round in 1..=max_rounds`**（:754）：
   - loop_top cancel 检查（:755-767）→ trace `{tool:"cancelled", phase:"loop_top"}`，break（软取消：正在跑的 LLM call 不 abort，注释 :409-410）。
   - budget 检查（:768-786）：`current_run_budget().should_stop_optional_llm_calls()` → 区分 `budget_exceeded`（`is_exceeded`）与 `usage_unknown`（后者 `mark_degraded("knowledge_agent_stopped_usage_unknown")`），trace 后 break。budget.rs:245-247 亲验：`should_stop = is_exceeded() || unknown_usage_calls > 0`。
   - `last_completed_round = round`（:787，供兜底上报真实轮数）。
   - **LLM 调用**（:795-835）：tx=Some 时走 `generate_agent_json_streaming`，spawn 一个 forwarder task 把原始片段喂 `AnswerStreamer`、解码出的 delta 发 `TraceEvent::Token`；返回后 `forwarder.await` 确保末尾 token 发完（:819-821）。tx=None 走 `generate_agent_json`。两者 prompt_key 均 `knowledge.agent`。
   - post_llm cancel 检查（:840-852）：本轮 mongo 副作用（日志/usage）不回滚。
   - **action 解析失败**（:854-869）：trace `{tool:"error", reason:"invalid_action:...", raw}` + `continue`（**消耗轮次**）。
   - **ListCatalog**（:872-900）：`intersect_catalog_filters(req.filter, llm_filter)` 求交（LLM 不能放宽初始 scope）；交集不可满足（None）→ catalog 置空 + trace filter=`{impossible:true}`。
   - **OpenDocument**（:901-923）：`open_document` 取该文档下原子摘要 → `merge_catalog` 合并（去重、总量截 2×30=60 :1543-1553）。
   - **OpenChunk**（:924-958）：`ids.take(8)`；`opened_seen` 去重；`open_chunk` 可能 redirect 到现行版（D3(b)），**记 `full.chunk_id`（现行版 id）而非请求 id**（:934-943，cite⊆opened 不变量靠此对齐）；不存在 → `notFound` 列表进 trace。
   - **FollowRelations**（:960-992）：depth clamp [1,2]；返回 (摘要 entries, 预取正文 prefetched)；prefetched 直接推进 `opened`（trace 记 `openedBodies`）。
   - **Answer**（:994-1060）：
     - 空白正文 → trace `error/empty_answer` + `continue`（末轮则自然落兜底）（:1003-1014）。
     - 先记 `attempted_cited/attempted_quotes` 原始条数（B5，:1019-1020），trace 写 `attemptedCitedCount/attemptedQuoteCount`。
     - `filter_answer_against_opened_chunks` 强过滤（见下）。
     - cache put（:1046-1058）：**put 前重取 provider identity 与 lookup 时比对**，防 lookup 后热切换 provider 把新代答案存旧 generation 下。
4. **兜底**（:1064-1096）：cited 恒空集（:1070，注释：打开过 ≠ 引用，绝不制造证据 id）；trace `{tool:"answer", rounds, truncated:true, cancelled}`；answer 文案区分 cancelled / 未收敛两种；`truncated=true`。

#### 检索原语

- `list_catalog`（:1109-1173）：Mongo filter = workspace + `domain:"user_operations"` + `status`（`filter.status` 或默认 `"active"`）+（`include_unverified=false` 时）`integrity_status:"verified"` + account `$or`（None→仅共享；Some→共享+私有）+ 可选 wiki_types/business_topics `$in`。DB 按 `dynamic_confidence:-1, priority:-1, updated_at:-1` 取 400 条 → 进程内 `rank_key` 全量重排 → 截 30。
- `resolve_superseded`（:1188-1242）：沿 `superseded_by`（hex 字符串）链跳；每跳要求目标存在 + 同 workspace（`chunk_scope_filter`，**不带 verified 门**，注释 :2017-2020：archived 前驱必须可读以解析指针）且**新版必须 `visible_chunk_filter`+verified**（:1223-1238）才 redirect；空白指针/非法 id/环（visited set）/8 跳 → 停在当前。
- `open_chunk`（:1252-1277）：id 非法 → None；resolve 后 `visible_chunk_filter + verified` 查现行版；非 verified（draft/needs_review）**静默 None**。
- `open_document`（:1284-1335）：document_id 非法 → 空集；过滤条件同 list_catalog + `document_id`；rank_key 排序截 30。
- `follow_relations`（:1339-1446）：BFS ≤2 层。每个关系：`classify_relation_role`（:1534-1541：`contradicts`→Contradiction、`superseded_by`→Version、references/requires/clarifies/refines/未知→Support）；**Version 直接跳过**（版本指针不当关系材料 :1376-1380）；先查 `visited.contains(原始 id)`（**不预插**，:1381-1390 注释详述历史 bug：预插会让未被取代的目标在 resolved_hex 二次 insert 返 false 被误丢）→ resolve redirect → `opened_seen`/`visited` 以 resolved_hex 去重 → verified-only 取目标 → 收集 (chunk, role)，达 16 停止。`split_prefetch`（:1566-1570，纯函数：前 cap 个/其余）切前 3 个转 `ChunkFull`（Contradiction 标 `relation_role`），其余转 catalog 摘要。
- `merge_catalog`（:1543-1553）：按 chunk_id 去重 append，超 60 截断。

#### 证据强校验（生产 answer 过滤）

`filter_answer_against_opened_chunks`（:1608-1646）：
1. `eligible_opened_chunks`（:1648-1654）= opened 中 `verified ∧ chunk_id 非空 ∧ relation_role != "contradiction"`；
2. requested = cited ∩ eligible（保序去重）；
3. quote 保留条件（:1624-1639）：chunk ∈ eligible ∧ chunk_id ∈ requested ∧ `quote_is_chunk_evidence`；
4. **cited 最终只保留有 evidence quote 支撑的 id**（:1640-1645）——`每个被接受的 cited 必有至少一条通过锚点校验的 quote`。

`quote_is_chunk_evidence`（:1656-1688）：quote 归一化（空白折叠 :1690-1692）非空 → 必须是 `source_quote` 或 `body` 归一化文本的子串 → **anchor index 必填**（无 anchor 的 quote 仅是上下文，不能成为 accepted 证据 :1672-1676）→ index 合法且 `source_anchors[index].sourceQuote` 存在 → anchor_quote 与 quote **互相包含之一**成立。

旧版 `filter_answer_against_opened`（:1581-1600，公开供 PBT）：仅要求 cited/quote.chunk_id ∈ opened_seen，无 anchor 校验——**生产循环用的是严格版**（:1021-1022 亲验）。

#### 排序与相关度

- `wiki_type_priority`（:1781-1794）：thesis 90 > synthesis 80 > methodology 70 > finding 60 > comparison 50 > concept 40 > entity 30 > source 20 > query 10 > 未知 0；None 按 entity=30。
- `RankKey`（:1814-1821）全序五元组，字典比较：`effective_relevance_micros`（主键）→ `live`（bool，live 排前）→ `wiki_priority` → `confidence_micros` → `priority`。
- `rank_key`（:1825-1845）：`base = relevance_score(query, chunk_haystack)`；`superseded`（superseded_by 非空白）→ trust ×0.1；`expired`（valid_to < now）→ ×0.5（可叠乘 0.05）；`live = !superseded && !expired`。**降格不剔除**（:1806-1809）：仍留候选，verified-only 硬门在 open_chunk 兜底。query 空 → relevance 恒 0，退化为 live 优先 + 静态序。
- `relevance_score`（:2192-2203）：`|query 信号 ∩ haystack 信号| / |query 信号|` ∈ [0,1]，任一信号集空 → 0。
- `text_signals`（:2207-2234）：ASCII 连续 alnum 串小写整体作 token；CJK 连续 run 拆相邻 bigram（单字 run 回退 unigram :2238-2250）；其它字符为分隔。`is_cjk`（:2254-2262）覆盖 CJK 统一表意 + 扩展 A + 平/片假名 + 兼容表意（不含韩文/扩展 B+）。
- `chunk_haystack`（:2267-2291）：title + summary + body + business_topics + **product_tags**（品牌别名召回，测试 :2741-2759 锁）+ wiki_type。

#### 在线召回-trace 闭环分类器

`attempted_citation_count`（:1880-1891）：从 answer trace 读 `max(attemptedCitedCount, attemptedQuoteCount)` 的最大值，缺字段回退 0。

`classify_recall_outcome`（:1893-1997）纯函数，按序短路：
- `cancelled` → None（用户主动取消非质量问题）；
- opened 统计 = trace 中 `open_chunk.opened ∪ follow_relations.openedBodies`（**摘要级 open_document/list_catalog 不算** :1899-1901）；
- **1a `citation_format_rejected`**（high）：`cited==0 ∧ attempted>0`——模型试过引用但全被锚点校验拒；affected=全部 opened；修复方向是**重锚定**而非补录（:1926-1943）；
- **1b `recall_miss`**（high）：`cited==0`；truncated 与否 title/description 不同 → dedup_key 不同 → 收件箱分列两行（:1945-1973）；
- **2 `recall_low_yield`**（medium）：`opened ≥ 3 ∧ cited ≤ 1`；affected = opened − cited（诊断价值最高）（:1975-1994）；
- 其余 → None。

#### 辅助

`account_or`（:2010-2015）；`chunk_scope_filter`（:2021-2029，能力域，无 verified 门）；`visible_chunk_filter`（:2031-2042，+`status:"active"`+`integrity_status:"verified"`）；`visible_corpus_signature`（:2044-2075）：对 `chunk_scope_filter`（**含 draft/needs_review/archived 全部**）投影 `_id+updated_at` 按 `_id` 升序算签名——任何 chunk 变动（含关系目标、未展示的 31+ 名）都使 cache 失效（E4 注释 :707-710）；`normalized_filter_key`（:2107-2126，值 trim/sort/dedup、`\u{1f}` join）；`intersect_catalog_filters`（:2128-2180）：两侧均 scoped 且交集空 → None；status 显式冲突 → None（requested 省略 → 继承 base）；`include_unverified = base && requested`（**AND 语义，agent 无法越权**）。

`truncate_chars`（:1999-2005，按 char + `…`）。PBT 导出常量（:2294-2298）。test_helpers + 44 个单测（:2302-3048）：证据校验矩阵（anchor None/-1/越界/不匹配全拒 :2482-2503）、contradiction 拒引（:2387-2397）、filter 交集、rank_key 五性质、streamer 8 例（CJK 切割/转义切割/\u 解码/工具轮零 token）、classify 分类矩阵等。

### 2.2 `src/agent/knowledge_agent/cache.rs`（377 行）—— 答案缓存

- `TTL=300s`（:26）、`MAX_ENTRIES=256`（:29）。进程级 `OnceLock<RwLock<CacheState{map,hits,misses}>>`（:57-66）。
- `CacheKey`（:31-43）10 字段：workspace_id / account_id / provider_id / provider_model / provider_generation / prompt_pack_version / filter_norm / query_norm / corpus_sig / max_rounds。
- `normalize_query`（:69-87）：trim + 折叠连续空白 + ASCII lowercase（CJK 不变）。
- `corpus_signature`（:91-99）：`DefaultHasher` 依次 hash len、(id, ts)。
- `get`（:102-124）：命中未过期 → 升写锁 hits+1 返回克隆；不存在 → misses+1 None；过期 → 懒删除 + misses+1 None。
- `put`（:127-150）：**`cancelled || truncated` 不缓存**（结果不稳定）；满 → O(N) 扫最旧 `inserted_at` 驱逐（N≤256 可接受 :133）。
- `cache_stats`（:162-182）；测试用进程级 `Mutex` 串行守门（:201）+ 12 个单测锁 TTL/驱逐/provider 失效/语义失效。

### 2.3 `src/agent/knowledge_router.rs`（1741 行）—— 生产召回路由

#### verified-only 加载

`load_operation_knowledge`（:36-86）：documents = workspace + `domain:"user_operations"` + `status:"active"` + account `$or`，`updated_at:-1` limit **80**；chunks 同上 + **`integrity_status:"verified"`**，`priority:-1, updated_at:-1` limit **200**。产出 `KnowledgeRuntime{documents, chunks}`（types.rs:1707-1711 亲验）。

#### 未验证告警（KNOW-2）

`unverified_warning_total_filter`（:91-101，active、**不限 integrity**）与 `unverified_warning_verified_filter`（:107-118，active+verified，与注入口径逐字对齐，否则归档已核验切片会抑制告警）。`maybe_emit_unverified_warning`（:125-190）：total>0 ∧ verified==0 → 当日（`today_start_millis` :192-196，UTC 日起点 `rem_euclid`）按 contact 去重写 `knowledge_unverified_warning` event（warn 级，count 失败静默按 0）。

#### prompt 渲染

`format_operation_knowledge_for_prompt`（:202-205）= DEFAULT 销售四态 wrapper；`..._with_roles`（:212-298，H16-b）：按 active DomainProfile 的 `chunk_roles` 分桶——chunk_type 命中 role.key → 该桶，未命中 → `is_fallback=true` 桶（都没有则第一个 role）；roles 空回落内置四态；`role.order` 升序输出、空桶无 header。`render_chunk`（:253-280）每条渲染 chunkId/type/chunkType/context/title/integrityStatus/confidence/summary/body/sourceAnchors(JSON)/sourceQuote，非空才追加 productTags/businessTopics（缺口7）。

#### 测试路由入口

`test_knowledge_route_for_contact`（:300-446）：contact 缺省时合成 preview contact（H13：初始 `operation_state` 从 active 状态机取 `initial_operation_state_key`，不写死 new_contact :309-316）；合成 inbound（`raw.runMode="knowledge_test"`）；memory 有持久 contact 才加载否则空白（typed `default_memory_card`）；调 `route_operation_knowledge_preview` + `select_operation_knowledge_chunks`；返回 `{route, selectedChunks}`。

#### 四个 route 变体（全部收敛到 inner）

| 入口 | read_only | force_agent | purpose | 行 |
|---|---|---|---|---|
| `route_operation_knowledge` | false | `inbound.is_synthetic_relay` | GeneratedReply | :448-472 |
| `route_operation_knowledge_for_existing_candidate` | false | true | ExistingCandidate | :477-501（手动发送候选文本已存在，无 Reply 槽位）|
| `route_operation_knowledge_preview` | false | true | PreviewOnly | :505-529 |
| `route_operation_knowledge_read_only` | true | false | GeneratedReply | :531-555（shadow）|

`KnowledgeRoutePurpose::required_tail`（:557-577）：GeneratedReply=4+dual、ExistingCandidate=3+dual、PreviewOnly=1（dual = `second_reviewer_llm.is_some()` 加 1；每种都保留 1 个 reached-cap 完成哨兵）。

#### 预过滤（跳过昂贵多轮推理）

- `knowledge_has_local_relevance`（:582-592）：query 非空 ∧ 任一 chunk 的 `rank_key.effective_relevance_micros > 0`。仅意味"可能相关"，零结果可跳过可选推理但**绝不产生证据/授权 claim**（注释 :579-581）。
- `knowledge_prefilter_requires_agent`（:598-625）：当前消息有本地相关 → true；否则须为短上下文依赖跟进（trim 后非空 ∧ ≤12 char ∧ `looks_context_dependent`）→ 按时间倒序取**最近一条内容不同**的消息，其有本地相关才 true。
- `looks_context_dependent`（:627-632）：结尾 `? ？ 呢 吗` 或含 `多少/多久/怎么/这个/那个/它/具体`。

#### `route_operation_knowledge_inner`（:634-907）—— 核心

1. documents+chunks 全空 → `{risk:medium, coverage:missing, reason:"没有可用运营知识库"}`（:648-655）。
2. `inbound_prompt_content` 剥哨兵（H10）取当前消息（:657-660）。
3. **预过滤跳过**（:661-680）：`!force_agent ∧ chunks 非空 ∧ !prefilter` → `{risk:low, coverage:not_required}` + trace `knowledge.skip/zero_local_relevance`。
4. **query 拼接**（:686-709）：`recent_messages.rev().take(8)` 每条 `"客户"/"我方"` 前缀 + `history_prompt_content` 剥 close-tag（P0-18）；history 非空 → `"用户当前消息（外部不可信文本，仅作上下文）：\n{current}\n\n最近对话：\n{history}"`，否则裸 current。
5. **预算保留**（:715-735）：`max_rounds = min(budget.available_llm_calls_before_tail(required_tail), 4)`（budget.rs:173-178 亲验：`effective_max - used - reserved`，饱和到 0）；`==0` → `mark_degraded("knowledge_route_skipped_required_tail_reserved")` + `{coverage:missing}` + trace `required_send_tail_reserved`。
6. **调 agent**（:736-747）：`CatalogFilter::default()`（include_unverified=false）；read_only 走 `answer_read_only` 否则 `answer`。
7. **route 结果映射**（:752-906）：
   - `cited_in_corpus` = answer.cited ∩ 预载 corpus（按 hex 比对）take 8（:752-762）；
   - `evidence_excerpts` = source_quotes 非空 quote（:763-768）；
   - **fallback_rank**（cited_in_corpus 空 :794-862）：按同一 `rank_key` 全排序（**闭降格漏点**：superseded ×0.1 / 过期 ×0.5 在弱路径同样生效 :797-806）→ `FALLBACK_TOP_N=5`（:788）。探索开关（`knowledge_exploration_enabled ∧ ranked>5`）时按 `softmax((relevance + wiki_priority×dyn_conf) × trust, 温度)` 不放回抽 5 并记 propensity（P4，仅记录不消费）；否则确定性 top5。空 → `(missing, medium, navigation_only=false)`；非空 → trace `fallback_rank{navigation_only:true}` + `(weak, medium, true)`；
   - cited 非空 ∧ quotes 空 → `(weak, low, false)`（agent 自选、过 cite⊆opened 校验，可授权 :863-871）；quotes 非空 → `(enough, low, false)`；
   - 组装 `KnowledgeRouteResult`（types.rs:1601-1653 亲验）：`reason=answer.answer`、`requires_evidence=!excerpts.empty`、`selected_chunks_are_fallback=navigation_only`、`selected_chunk_rankings=build_chunk_rankings(..., "tool_loop", probs)`。

#### 纯函数三件套

- `build_chunk_rankings`（:919-945）：rank=选中下标、score=`wiki_type_priority × dynamic_confidence`、pool_size=corpus 大小、`selection_prob` 仅探索路径记录（None=propensity 1.0）；corpus 找不到的 id 跳过（不杜撰）。
- `softmax_probs`（:952-974）：减 max 数值稳定；`temp<=0` 夹 1e-6；exp 溢出/全 -inf → 均匀分布（绝不 NaN/全 0）。
- `sample_k_without_replacement`（:981-1011）：轮盘赌不放回；剩余权重全 0 → 顺序取；恒返回 min(k,n) 个不重复下标。

#### 授权红线与审计

- `empty_knowledge_route`（:1013-1024）：Reply Agent planner 判无需知识 → `not_required` + trace `knowledge.skip`。
- **`route_used_knowledge_ids`（:1037-1049，B2 红线）**：`selected_chunks_are_fallback=true` 时**只返回 `selected_knowledge_ids`（不含 chunk id）**——fallback 回填无相关度下限、未过 citation/anchor 校验，而下游 `compute_verified_chunks` 只做 `used ∩ verified ∩ 未过期` 交集，若透传则一批无关 verified chunk 能结构性放行 `blocked_unverified_product_claim`（测试 :1687-1705 锁死；legacy 缺字段默认 false=可授权 :1728-1739）。
- `select_operation_knowledge_chunks`（:1051-1065）：按 selected_chunk_ids 从 corpus 捞全量 chunk。
- `write_knowledge_usage_log`（:1086-1149）：insert `KnowledgeUsageLog`（ids=knowledge∪chunk 的合法 ObjectId、route_result 全量、reply_text、review_approved、blocked_reason=未过审的 review_summary、tool_trace）；随后对每个 id 顺序 `record_chunk_hit(blocked=!approved)`（`let _=` 吞错；注释称 fire-and-forget，实为顺序 await 仅吞错——见 §5 疑点 6）。

测试（:1151-1740）：purpose tail 表、预过滤 4 例、B3 渲染 10 例、H16-b 自定义 roles、P4 softmax/抽样 7 例、KNOW-2 filter 对齐 3 例、B2 红线 3 例。

### 2.4 `src/agent/knowledge_tools.rs`（1784 行）—— chat 工具派发全集

#### 工具名闭集与常量

`ALLOWED_CHAT_TOOL_NAMES`（:91-101）共 **9 个**：user-ops 三件套 `knowledge.list_catalog` / `knowledge.search` / `knowledge.open_slice` + chat-only 六件 `knowledge.audit_completeness` / `knowledge.search_chunks` / `knowledge.propose_repair` / `knowledge.analyze_logs` / `knowledge.open_document` / `knowledge.verify_anchor`。

常量：`TOOL_DISPATCH_TIMEOUT=5s`（:51）、`LIST_CATALOG_PER_KIND_LIMIT=2`（:54）、list_catalog limit 默认 50 / 上限 200（:57-58）、`SEARCH_QUERY_MAX_CHARS=200`（:61）、`SEARCH_SNIPPET_MAX_CHARS=200`（:64）、`REDACTED_UNVERIFIED_BODY="<redacted_unverified_chunk>"`（:68）、`CHAT_TOOL_CALLS_PER_TURN_CAP=6`（:104）、`CHAT_ANALYZE_LOGS_WINDOW_HOURS=24`（:107）、`CHAT_ANALYZE_LOGS_MAX_CHUNKS=32`（:109）。

`ToolDispatchState`（:116-127）：跨轮共享的 `list_catalog_calls_per_kind` map。

#### dispatch 入口

`dispatch_chat_tool_call`（:597-667）：①白名单校验（`unknown_tool`）→ ②`budget.record_tool_call(0)` 占 1 槽（budget.rs:202-210 亲验：tool_calls 达 budget → `ToolCallsExceeded`）→ 超额 `{error:"budget_exceeded"}` → ③整个 exec 包 5s `tokio::time::timeout` → 超时 `{error:"tool_timeout"}`。错误全部以 Value 返回（fail-as-Value，让 LLM 下轮自我修正 :513）。永不写 outbox/mcp/gateway（:514）。

#### user-ops 三件套（同步、只读预载 KnowledgeRuntime）

- `exec_list_catalog`（:172-270）：kind ∈ documents|items|chunks（缺省 chunks，非法 `invalid_input` **不消耗配额** :258）；同 kind 第 3 次 → `tool_call_repeated`（R4.4）；limit clamp；documents → `{id,title,category=source_type,integrity_status:null,updated_at}`；**items 恒空**（operation_knowledge_items 已删除 :231-234）；chunks → `{id,title,category=knowledge_type,integrity_status,updated_at}`；`truncated = total > len`；**不返回正文**。
- `exec_search`（:274-331）：query trim 非空且 ≤200 char（`invalid_query`）；top_k clamp [1,32] 缺省 `runtime.knowledge_search_top_k`（runtime.rs:430/584/955 亲验默认 8，loader clamp [1,32]）；`score_chunk_for_query > 0` 者降序取 top_k。
- `score_chunk_for_query`（:357-385）：`relevance_score(title)×3 + (summary)×2 + (body)×1`，基础分 >0 且 verified 再 +0.5（迁移自旧 contains 整串匹配，中文召回改善，测试 :1707-1726）。
- `build_search_hit`（:333-355）：verified → snippet = summary‖body 截 200；**非 verified → snippet 空串 + `redacted:true`**（integrity_status 保留原值）。
- `resolve_superseded_in_memory`（:409-436）：D3(b) 内存版 redirect——预载集合已 active+verified，新版必须**也在集合内**才跳；防环 visited；≤8 跳；找不到停在当前（**绝不丢内容** :406-408）。
- `exec_open_slice`（:438-506）：chunk_ids 非空；`cap = runtime.knowledge_open_slice_max_k.max(1)`（默认 4，loader clamp [1,16]，runtime.rs:429/583/954 亲验）超 → `over_limit`；**任一未知 id → `unknown_chunk_id` 整体 fail 不返回部分结果**（R4.6 :461-481）；命中先 redirect；verified → body 原文；非 verified → body=`<redacted_unverified_chunk>`、integrity_status 保留、redacted=true。

#### chat-only 六件（异步、直查 MongoDB，args 均 `rename_all="camelCase"`）

公共 scope：`scoped_knowledge_filter`（:1233-1244）= workspace + `domain:"user_operations"` + account `$or`——**无 status/verified 门**（chat 工作站可见 draft/needs_review/archived 元数据，正文靠 redact 与只读约束兜底）。

- `exec_audit_completeness`（:675-754）：missing_fields 检查 title/summary/sourceQuote/applicableScenes 4 项；`completeness_score = (4-缺失)/4` 保留 3 位；错误 `invalid_input`/`unknown_chunk_id`/`db_error`。
- `exec_search_chunks`（:763-836）：query 校验同 search；top_k 缺省 8；`only_verified` 可选（默认 false）；DB 拉 `updated_at:-1` limit 200 in-memory 评分（避免 `$text` 索引依赖 :787）；hit 结构同 search（redact 同规则）。
- `exec_propose_repair`（:843-922）：产 suggestions（缺 sourceQuote→high、缺 applicableScenes→medium、integrity 非 verified/needs_review→low）；**`ai_will_not_auto_apply:true`（:920），不写库**——与"AI 永不自动 verify"一致。
- `exec_analyze_logs`（:930-1033）：hours clamp (0,72] 缺省 24；`only_blocked_or_held` 缺省 true（filter `$or: review_approved=false | blocked_reason 存在`）；**入参 accountId ≠ 当前 chat 账号 → `account_scope_violation`（在任何 DB 访问前拒 :949-960）**；`knowledge_usage_logs` desc limit 32；输出 total/blocked 计数、`top_chunks` 频次 top8、items 明细。
- `exec_open_document`（:1041-1099）：max_chars 缺省 4000 上限 8000；返回 title/source_type/source_name/summary/catalog_summary/status/`raw_content_excerpt`（char 截断）/truncated/total_chars。
- `exec_verify_anchor`（:1107-1229）：chunk 无 document_id → `{anchor_hit:false, method:"none"}`；父文档缺 → `missing_parent_document`；candidate = 入参 sourceQuote ?? chunk.source_quote，空 → none；**exact**：`raw_content.find(candidate)` → `{method:"exact", anchor:{startOffset,endOffset}}`（字节偏移）；miss → 注入的 `AnchorMatchFn`（:585，chat route 注入 `source_anchor_for_quote` 同款模糊算法，routes/knowledge/chat.rs:1932/2001-2009 亲验）→ `{method:"fuzzy"}`；都 miss → none。**只校验不写库**——「主动自检」不破 AI 永不 verify 红线（:558-560）。

`parse_arguments`（:1246-1254）：空 Document → default；否则 BSON→relaxed extjson→serde。注意 user-ops 三件套 args（:132-157）**无 camelCase rename**（LLM 传 `chunk_ids`/`top_k` snake_case），chat-only 六件为 camelCase（`chunkId`/`topK`），P2-11 测试锁死两套约定（:1590-1702，含反向断言 snake_case 不识别 chat-only args）。

### 2.5 `src/agent/chat_tool_loop.rs`（719 行）—— chat 多轮工具循环

常量（:36-44）：总超时 30s、失败连击阈值 3、结果上下文 8000 chars、trace 上限 32、**最大轮数 4**（`requested_max_loops` clamp [1,4] :127）。

`chat_reply_with_tools_loop_with_timeout`（:113-254）主循环：
1. 每轮先查绝对 deadline（:139-145，到点 → `ChatToolLoopError::Timeout{elapsed, risks, tool_trace}`）；
2. `truncate_tool_results`（:400-413）：累计 `[system tool result]` 段超 8000 char → **keep-tail** 截断 + 一次性 risk `chat_tool_result_context_truncated`；
3. `timeout_at(deadline, reply_fn(truncated, loop_count))`——reply_fn 是注入闭包（chat.rs:1868 组 prompt+调 LLM），超时 → push `chat_tool_loop_timeout` + Timeout 错误；
4. `decision_phase=="tool_calling"`：带 reply_text 或 should_reply → risk `chat_tool_calling_phase_with_reply_text`（:165-167）；`dispatch_chat_turn`；timed_out → Timeout；force_stop → break；
5. 其它 phase（final 等）→ `strip_extra_tool_calls`（:415-421：final 残留 toolCalls 清空 + risk `chat_final_phase_extra_tool_calls_dropped`）→ break。

收尾（:207-246）：无任何 decision → `Reply(External)`；轮耗尽仍 tool_calling → risk `chat_tool_loop_exhausted` + 清 tool_calls + 强制 `final`（任何 tool_calling 残留同样强制 final :221-224）；trace >32 → 截断 + risk `chat_tool_trace_overflow`；**final 一致性检查**（:231-243）：`knowledge_need_reason` 非空非 `"unchanged"` 但 trace 中无一次成功的 `search`/`open_slice`/`search_chunks` → risk `chat_knowledge_need_declared_but_not_consulted`；最后并入 `last_promote_risks`。

`dispatch_chat_turn`（:262-344）：单轮 calls >6 → 截断 + risk `chat_tool_calls_per_turn_truncated`；逐个 `timeout_at(deadline, dispatch_chat_tool_call)`（**受总 deadline 而非单独 5s 限制的外层再包一层**；单工具 5s 在 dispatch 内部）；超时 → `{force_stop, timed_out}`；每个结果 `build_tool_trace_entry`（:346-383：error/detail 或 hit_count/completeness_score/blocked_or_held_runs/result_summary="ok"，含 latency_ms/started_at）；**失败连击**：`error` 存在 → streak+1，否则清零；streak≥3 → risk `chat_tool_call_failure_streak` + force_stop；`budget_exceeded` → risk `chat_tool_budget_exhausted` + force_stop。

`append_tool_result_to_context`（:385-398）：`\n[system tool result]\ntool: ...\narguments: {...}\nresult: {...}\n` 追加进累计串。

测试（:423-719）：final 单轮即退（不触 db、零配额消耗）、绝对 deadline 打断慢 reply、4 轮耗尽归一 final、requested limit=2 生效、常量金标、strip/truncate/trace entry 各 1 例。

### 2.6 `src/knowledge_wiki/mod.rs`（29 行）

模块清单 + 四件事定位（质量/可检索/可修改/可优化 :3-7）+ 召回算法零改动声明（:9-10）+ 隔离红线（:10-11）+ 子模块分工注释（page_merge 纯函数预校验 / chunk_revisions 状态机 :13-18）。

### 2.7 `src/knowledge_wiki/block_parser.rs`（468 行）—— fence 分块解析

输入格式 `---CHUNK: <id>---\n{json}\n---END CHUNK---`（:9-17）。`parse_chunk_blocks`（:77-191）**永不 Err**（:75-76），状态机 Outside{stray_buffer}/Inside{id,body}：
- fence 起始 = `trim_start` 后以 `---CHUNK:` 开头且以 `---` 结尾（`parse_fence_start` :194-202，id trim 非空）；Inside 中遇新起始 → 上一块 `UnterminatedFence`；
- **unsafe id**（`is_safe_block_id` :205-219：非空、≤128、无 `..`、无 `/ \ < > | ? * " :` 及控制字符）→ `UnsafeBlockId` warning + 进入 `__unsafe__` 忽略态直到 END（:109-119）；
- fence 结束 = trim_start 后**整行恰等于** `---END CHUNK---`（:128）；Outside 遇孤立 END → 计入 stray；
- `finalize_block`（:222-260）：JSON 解析失败 / 非 Object / body+summary+answer 全空（`is_payload_empty` :263-275）→ `InvalidJson` 丢块；
- 收尾仍 Inside → `UnterminatedFence`（已闭合块照常保留 :26-27）；
- `dedup_keep_last`（:278-308）：同 id 保留最后一个 + `DuplicateBlockId`；
- `flush_stray`（:311-318）：fence 外非空白文本 → `StrayText`（80 char excerpt）。

11 个单测覆盖 traversal id、截断、CRLF、行内 fence token 缩进不终止（:439-446——注意该测试的 token 在 JSON 行**内部**，见 §5 疑点 7）。

### 2.8 `src/knowledge_wiki/catalog_rebuild.rs`（867 行）—— catalog 落库 worker

常量：`BATCH_SIZE=16`、`LEASE_SECONDS=60`、`MAX_ATTEMPTS=5`、`MAX_RETRY_DELAY_SECONDS=300`（:37-40）。

- `catalog_rebuild_worker_loop`（:50-69）：`interval_secs==0` 关停；循环 `drain_pending_jobs` + sleep。
- `drain_pending_jobs`（:73-125）：先 `fail_exhausted_recoverable_jobs`（:327-362：attempts≥5 且 queued/过期 processing → failed + 清 claim 字段）；≤16 次 `claim_one_job`；legacy（`target_generation<=0`）先 `upgrade_legacy_claim`（:174-290，≤3 重试；事务内：验 claim → 读父文档（缺 → target=None 直接提交）→ `target=max(desired,applied)+1` → CAS 推进 desired（0 值兼容 exists/null 三态 :231-241）→ job 写 target）；spawn `spawn_claim_heartbeat`（:292-325，每 20s 以 owner+token+generation+`locked_until>=now` CAS 续约，丢锁即退出）；`render_one_document` → `finalize_rendered_catalog`；任何失败 → `requeue_or_fail_owned_job`。
- `claim_one_job`（:366-427）：可领取 = `attempts<5` ∧（queued 且 next_retry_at 到期 ∨ processing 且 lease 过期 ∨ **failed 且 legacy 无 target_generation**）；原子 `find_one_and_update` 置 processing + worker_id + claim_token(uuid) + lease + `$inc attempts/claim_generation`；sort `target_generation,queued_at,_id`。
- `claim_identity_filter`（:149-172）/`active_claim_filter`（:525-557，+`locked_until>=now`）：owner/token/claim_generation/target_generation 全绑定 fencing。
- `render_one_document`（:436-462）：该 document 下 `status:"active"` chunk（**无 verified 门**——catalog 摘要含未核验条目的元信息）`priority:-1,updated_at:-1` limit 500 → `render_persisted_catalog`（:476-523：markdown，每 chunk `### title` + id/类型(wiki_type??knowledge_type??未分类)/路由(恒 "—" :490)/integrity|confidence/dynamic|hits30d/`> excerpt≤240`；空 → "（该文档暂无 active chunk）"）。
- `finalize_rendered_catalog`（:559-585）：≤3 重试（`retryable_finalize_error` :579-585 = TransientTransactionError ∨ `catalog_generation_conflict`）。`_once`（:587-732）事务内：验 claim → 父文档缺 → job **discarded**；`applied>=target ∨ desired>target` → job **superseded**（更新的 intent 已排队）；否则 CAS（desired==target ∧ applied==读取值）写 `catalog_summary_persisted` + `catalog_applied_generation=target` + `$inc catalog_version` → job **done**。任何 matched≠1 → `catalog_claim_lost`/`catalog_generation_conflict` abort。
- `requeue_or_fail_owned_job`（:734-780）：attempts≥5 → failed+finished_at；否则 queued + `next_retry_at = now + 2^(attempts-1)s`（clamp ≤300s，`retry_delay_seconds` :144-147）。

job 状态机（字符串）：queued → processing → done | superseded | discarded | failed（→ 仅 legacy 可再领取）。

### 2.9 `src/knowledge_wiki/chunk_revisions.rs`（1354 行）—— 写入状态机

#### 闭集

- `RevisionOp`（:65-94）**10 个**：create / patch / split / merge / rollback / archive / restore / verify / unverify / reject。
- `ProvenanceSource`（:97-139）**5 个**：ai / human / rule / imported / **principal_authorized**（:107-110：领导真人裁决，视同 Human 权威家族，不落 AI draft 强制降级分支，可直接带 verified）。`FromStr` 白名单，未知 → BadRequest。

#### op × source 生命周期矩阵（`apply_server_owned_lifecycle` :201-245）

判定 `requires_review = (source==Ai) ∨ chunk_patch_requires_review(op, patch)`（:207-208）。`chunk_patch_requires_review`（:189-196）= op ∈ {Patch, Split, Merge, Rollback} ∧ patch 顶层 key 命中 `REVIEW_SENSITIVE_PATCH_FIELDS`（:174-187，12 字段：title/summary/body/knowledge_type/business_context/applicable_scenes/not_applicable_scenes/product_tags/business_topics/source_quote/source_anchors/domain_attributes；**结构性 relation-only patch 刻意排除** :173）。

| 条件 | 生命周期写入 |
|---|---|
| requires_review（AI 任意 op；或 Patch/Split/Merge/Rollback 碰敏感字段） | **强制 `status=draft` + `integrity_status=needs_review` + `confidence_score=0`**，直接 return（:209-214）——即使 op=Verify 且 source=Ai 也被压回（测试 `ai_verify_operation_cannot_promote_knowledge` :1205-1220）——**「AI 永不自动 verify」的代码级落点** |
| Archive | `status=archived`（:217-219） |
| Restore | `status=active`（:220-222） |
| Verify / Unverify / Reject | 从 patch 透传 status/integrity_status/confidence_score（:223-229，人/规则签字路径） |
| Split / Merge（不碰敏感字段时） | 额外透传 superseded_by / previous_version_id（:230-242） |
| Create / Patch / Rollback（不碰敏感字段时） | 无额外生命周期写入（:243） |

**server-owned 强制**：该函数在 `enforce_locked_fields` **之后**执行（:338→:347）——运营 per-chunk 锁不能把内容编辑后的 verified 状态锁住不降级（:198-200 注释 + 测试 :1163-1178）。

#### `prepare_chunk_revision`（:300-445）—— 三层保护流水线

`before_hash = compute_chunk_hash(existing)` → ① `apply_field_patch(existing, patch, DEFAULT_LOCKED_FIELDS)`（锁定字段**硬拒** 4xx）→ ② `union_array_fields(after_patch, existing原始, patch, DEFAULT_UNION_ARRAY_KEYS)`（KB-09：数组既有源必须是原始 chunk，防 patch clobber）→ ③ body/summary/answer **逐字段独立** 70% 门（:321-333，`is_body_truncated(old, incoming, merged, 0.7)`，长 summary 不能掩护被截断的 body）→ ④ `effective_locked_fields(existing)`（DEFAULT ∪ 运营 locked_fields）→ `enforce_locked_fields` 末次静默覆盖 → ⑤ domain schema `enforce_domain_attributes`（有 active schema 且 merged 带 domain_attributes 时）→ ⑥ `apply_server_owned_lifecycle` → ⑦ `build_chunk_provenance`（:252-272：**保留原始 source 等溯源字段**，只刷 edited_at/edited_by；无 actor 移除 edited_by；legacy 无 provenance 用当前 source 初始化）→ ⑧ `monotonic_chunk_updated_at`（:51-60：`max(now, expected+1ms)` 严格递增 CAS token）。

`after_hash` → `unchanged = before==after`；**Create 即使 unchanged 也强制写**（:364-367 `force_create_write`）。replacement 先反序列化成 typed 校验但**保留原 BSON**（:393-397，前向兼容 review 字段如 verified_claims 不丢，测试 :1078-1125）。unchanged（非 create）→ replacement=None、catalog_job=None（跳过 rebuild enqueue）。catalog_job 的 `target_generation=0` 占位（真值在 enqueue 时事务内 CAS 分配）。

#### 持久化与事务

- `persist_prepared_chunk_revision_with_session`（:447-476）：**先写 revision，再 CAS replace chunk**（filter = `_id+workspace+updated_at`，matched≠1 → `chunk_revision_conflict`）——顺序刻意（模块 doc :13-15：replace 失败留下"试图未成功"revision 痕迹，事务下实际同 commit/同滚），最后 enqueue catalog job。
- `enqueue_catalog_job_with_session`（:482-540）：事务内父文档 `catalog_desired_generation+1` CAS（0 值兼容三态 :503-513）；父缺 → `catalog_parent_missing` **失败整个知识变更**（:498，不静默丢投影）；job.target_generation=next。
- `commit_chunk_transaction`（:564-575）：`UnknownTransactionCommitResult` label 无限重试 commit；其它错 abort。`map_chunk_transaction_error`（:577-584）：TransientTransactionError → `chunk_revision_conflict`（Conflict/409）。
- `apply_chunk_revision`（:825-848，主入口）：开事务 → `apply_chunk_revision_with_session`（:588-619：raw find_one（保留未建模字段）→ typed 校验 → `unique_active_schema_with_session`（:542-562，>1 条 active schema → External 错）→ prepare → persist）→ commit。
- `rollback_chunk_revision_with_session`（:705-810）：找 target revision（workspace+chunk_id+revision_id）→ 取 **before_snapshot**（缺 → `chunk_revision_snapshot_unavailable`）→ `build_snapshot_rollback`（:637-680）：从快照恢复，但 `ROLLBACK_PRESERVED_FIELDS`（:621-635，13 个：身份 8 项 + locked_fields/usage_stats/dynamic_confidence/integrity_score）从**当前行**保留；再按当前锁 `enforce_locked_fields`（历史快照不能绕过其后配置的锁，测试 :1268-1272 锁 body）；**强制 draft+needs_review+confidence 0**（回滚永不自动回到 verified）；provenance=Human+actor；monotonic updated_at → schema 强制（`enforce_snapshot_domain_attributes` :682-703，快照缺必填域属性 → fail closed，测试 :1325-1353）→ typed 校验 → 写 rollback revision（patch=`{rollback_to_revision}`）+ replacement + catalog job。

#### 删除级联

`normalize_ref_key`（:869-875）：去 `.md`、取 `/` 末段、小写、去空格/短横/下划线（"openai"≠"ai" 防 substring 误伤，测试 :1002-1005）。`cleanup_dangling_refs`（:886-951）：archive 后扫同 workspace 所有带 related_chunks 的 chunk，移除 `chunk_id == archived_id ∨ normalize 等价` 的条目；每条受影响 chunk 走 `apply_chunk_revision(op=Patch, source=Rule, actor="system:cleanup_worker")` 留痕；related_chunks **整数组 patch**（不在 union keys——结构数组需按 chunk_id 去重 :928-929）；单条失败仅 warn 不冒泡（archive 主动作已成，cleanup best-effort :885）。注意：related_chunks 不在 REVIEW_SENSITIVE_PATCH_FIELDS → 清理不会把 verified chunk 打回 needs_review（与 :173 排除声明自洽）。

### 2.10 `src/knowledge_wiki/feedback_worker.rs`（335 行）—— 反馈闭环 worker

- lease：`FEEDBACK_LEASE_SECONDS=300`、心跳 60s（:27-28）；`FeedbackLease.id()="knowledge_feedback::{ws}"`，owner_filter=_id+token（:31-44）。
- `feedback_worker_loop`（:50-65）：interval=0 关停；每轮 `run_one_round` + sleep。
- `run_one_round`（:69-79）：`list_workspaces`（:291-307，distinct chunks.workspace_id；**空回退 default workspace**）→ 每 ws `try_acquire_feedback_lease`（:81-128：`find_one_and_update` upsert，`locked_until` 过期/null/缺失才可抢，返回行 token==自己才算拿到；并发 upsert 撞唯一键 → None）→ `run_workspace_with_lease`。
- `spawn_feedback_lease_heartbeat`（:145-177）：60s 续约；matched≠1 或 Err → `cancelled=true` 退出（fencing）。
- `run_workspace_with_lease`（:179-252）**五阶段串行**，每阶段前查 cancelled：
  1. `gap_signals::refresh_usage_stats_and_confidence_controlled`（带 cancelled 引用，内部逐 chunk 检查 → 丢锁抛 `knowledge_feedback_lease_lost`）；成功后 `upsert_deal_attribution_stats`（:257-288，stat_id=`{ws}::deal_attribution`，$set 瞬时值，0 也写锚点）；
  2. `run_structural_lint`；3. `sweep_stale_signals`；4. `lessons_learned::aggregate（14d）`；5. `reviewer_stats::aggregate（14d）`。
  每阶段失败只 warn 继续（阶段间无依赖回滚）。结束 abort 心跳 + 把 `locked_until=now` 主动释放。

### 2.11 `src/knowledge_wiki/gap_signals.rs`（2519 行）—— 信号 + dynamic_confidence

#### 信号类型全集

结构 lint 侧（`compute_structural_candidates` :200-396 纯函数产出）**9 类**：

| kind | severity | 触发条件 | 行 |
|---|---|---|---|
| `orphan` | info | 无入链 ∧ 30d hit==0 | :227-242 |
| `broken_link` | warning | related 目标既不在 active 也不在 archived | :260-268 |
| `missing_chunk` | error | related 目标在 archived 集合（依赖被回收） | :252-259 |
| `no_outlinks` | info | wiki_type ∈ {synthesis, comparison, methodology}（:45）∧ related_chunks 空 | :272-291 |
| `low_confidence` | warning | dynamic_confidence < 0.3（:48）∧ 30d hit > 0 | :293-304 |
| `stale` | warning | valid_to < now | :306-317 |
| `suggestion` | info | 非 verified ∧ 30d blocked > 3 | :319-336 |
| `dangling_anchor` | warning | 文档有 raw_content ∧ source_quote 悬空（查无原文跳过不误报） | :338-354 |
| `contradiction` | error | 同 normalize_title ≥2 chunk ∧ body 首段 sha256 ≥2 种 | :357-393 |

在线召回侧（knowledge_agent 产出，`source="recall_trace"`）3 类 kind：`recall_miss` / `recall_low_yield` / `citation_format_rejected`（见 §2.1）；另有 `recall_miss_from_product_block`（:437-457）：产品宣称被 `blocked_unverified_product_claim` 拦截时构造，severity=high，客户问句进 `search_queries`。

`anchor_is_dangling`（:177-192）契约：quote None/空白 → **false**（不打扰）；strip 全 Unicode 空白后 quote 是 raw 子串 → false（容忍 PDF 换行差异）；否则 true（含 raw=None ∧ quote 非空）。`dangling_anchor_penalty`（:59-65）：悬空 → 0.3，形参签名兜住「查无原文不罚」（调用方仅 raw 命中才传 :56-58）。

#### 去重与落库

- `dedup_key` / `signal_dedup_key`（:459-477）：broken_link/missing_chunk（affected≥2）→ **`link::{from}::{to}`（两 kind 共享前缀**，archive↔active 切换不产生孪生重复，测试 :2112-2129）；其余 → `{kind}::{normalize_title(title)}`。
- `persistent_signal_dedup_key`（:479-481）= sha256(logical_key)，落 `dedup_key` 字段；**唯一 partial index**（db/indexes.rs:180-194 亲验：`{workspace_id, dedup_key}` unique，partial `status=="pending" ∧ dedup_key 为 string`）关并发窗口。
- `pending_signal_merge_update`（:497-518）：$setOnInsert（剔除 _id/affected/search_queries）+ `$addToSet $each` 并集两个数组。
- `upsert_pending_signal`（:540-576）：upsert；dup key 输家 → 无 upsert 重试同 merge；仍 matched==0 → 冒泡原错。
- `persist_signals`（:585-677，**离线全量**）：载全部 pending 建 key map → 候选命中 → 原子 merge（sweep 竞态时 fallthrough 新建）；未命中 → 新建；**stage1 auto_resolve**：pending 有但本轮候选无 ∧ `source=="rule"` → `auto_resolved/rule:no_longer_matches`（LLM/recall_trace 信号不被误消解 :651-656）。
- `persist_recall_signal`（:690-752，**在线只增不消解**）：绝不触发 sweep（否则冲掉离线 pending :682-688）；KB-07：按 `{workspace, pending, kind}` **全量拉回**再按 dedup_key 精确匹配（旧 find_one 任意单行漏合并的病根 :696-703）；命中 → merge；否则 insert（source="recall_trace"）。

#### outcome 三态与极性

- `classify_outcome_label`（:769-775）→ `classify_outcome_label_with_polarity`（:796-811）：正极 → Hit、负极 → Block、**其余一切（沉默/pending/空/未知）→ Censored 删失**（Iron Law ②不可配 :793-795）；正极优先于负极（误配重叠取 Hit :804-807）。
- 默认极性（:778-787）：正=`user_replied_buying_signal`；负=objection/stop_requested/unsubscribed/negative/complaint 5 词。
- `resolve_effective_polarity`（:929-949）：从 DomainProfile.outcome_polarity 逐极独立回落默认常量（某极空才回落；seed 与回落同源 → 销售域字节等价）。

#### dynamic_confidence 与 usage 刷新

- **公式**（`compute_dynamic_confidence` :1314-1333）：`total = h + b`；`total < min_samples` → `clamp(base − stale_penalty, 0, 1)`（S7 止血：小样本不信 hit_rate）；否则 `clamp(base×0.6 + hit_rate×0.4 − stale_penalty, 0, 1)`，`hit_rate = h/total`（total==0 仅 min_samples==0 可达，取 0 防 NaN）。`base = integrity_score ?? 0.5`（:1162）；`stale_penalty = valid_to<now ? 0.3 : 0`（:1163-1166）；**dangling_penalty 0.3 叠加进 stale 位**（:1169-1180）。
- `refresh_usage_stats_and_confidence_controlled`（:993-1216）：30d 窗口拉 `knowledge_usage_logs` → `real_outcome_enabled`（默认，config）时按 run_id 批量 join `decision_reviews.outcome_status`（filter 带 workspace_id，IDOR 纪律 :1044-1046）三态判定；**H11 成交追认**：`real_outcome ∧ 正极非空` 时 `compute_deal_attributed_log_indices`（:1227-1297：log 按 `(account_id, wxid)` 分组防跨账号误归因 :1232-1235；contact 的 `confirmed_deal_timestamps`（staff_confirmed/payment_verified 闭集）→ `attributed_log_indices`（:894-920 纯函数：每笔成交前最近 N=3 条 log，多笔并集 :882 `DEAL_ATTRIBUTION_WINDOW_TURNS=3`））→ 被追认 log 翻 Hit（原非 Hit 计 `deal_attributed_hits`）；回滚路径（false）逐字节退回 `review_approved` 自评。每 log 的每个 knowledge_id 计 hit/block（Censored 只记 last_used）→ 每 active chunk 算 `dyn_conf` → `usage_refresh_pipeline`（:828-876 聚合管道：**保留快照后热路径增量** `max(0, 当前-observed)` 叠加回写；last_used_at 取 max；last_blocked_reason 热增量优先保留现值）单条 update。cancelled 检查每 chunk 一次（丢 lease → `knowledge_feedback_lease_lost`）。
- `record_chunk_hit`（:1351-1383）：热路径实时回写——仅 `$inc hit/blocked_count_30d` + `$set last_used_at`（blocked 带 reason）；非法 id 静默 Ok；不算浮点（worker 周期算 :1349-1350）。

#### sweep（stage 1 规则消解）

`sweep_stale_signals`（:1393-1653）：预建 active 视图（known_ids、title→首段哈希组、integrity map、incoming 入链、chunk_view 四元组）→ 扫全部 pending，8 个 arm 判自愈（`resolution_note` 区分）：
- broken_link：target 恢复 active → `rule:link_recovered`；源自身 archived → `rule:source_archived`（:1480-1501）；
- missing_chunk：dep 回 active → `rule:dep_restored`；源 archived → `rule:dep_unrelated`（:1502-1522）；
- stale：valid_to 已推未来 → `rule:valid_to_extended`（:1523-1540）；
- suggestion：chunk 已 verified → `rule:chunk_verified`（:1541-1554）；
- contradiction：strip "同题异说：" 前缀后同题组只剩 <2 或哈希收敛 → `rule:contradiction_resolved`（:1555-1571）；
- orphan：archived / 入链恢复 / 命中恢复（:1572-1587）；
- no_outlinks：archived / wiki_type 不再要求 / 已补出链（:1588-1611）；
- low_confidence：archived / 回阈值 / 30d 命中归零（:1612-1632）；
- 其它 kind（含 recall_*）→ 不动（:1633）。
stage 2 LLM batch 仅预留（`SweepReport.stage2_llm_resolved` 恒 0，:24-31 模块注释 + :1655-1660）。

工具函数：`normalize_title`（:1664-1670，trim+小写+去空白短横下划线）、`first_paragraph`（:1674-1678，双换行分段）、`sha256_hex`（:1681-1689）。

测试（:1695-2519）：pipeline 保留增量/不负 delta、dyn_conf 7 边界、9 类候选各 1-2 例、anchor 契约 6 断言、dedup、merge update 结构、三态判定矩阵、极性参数化 5 例、成交追认窗口 6 例。

### 2.12 `src/knowledge_wiki/ingest_worker.rs`（857 行）—— 外部源自动 ingest

常量：`FAILURE_STREAK_TO_FAILING=3`、`UNREACHABLE_DISABLE_HOURS=168`（7 天）、`LEASE_SECONDS=120`（:31-33）。

- `ingest_worker_loop`（:36-48）：interval=0 关停（另有 main.rs 侧 `INGEST_WORKER_ENABLED` 门，模块注释 :13-14）。
- `run_one_round`（:53-128）：`list_workspaces`（:579-590，distinct ingest_sources.workspace_id，**空不回退 default**）→ `list_active_sources`（:592-609，status ∈ {active, failing}——failing 继续重试可复活，disabled 停扫需 admin 手动 :593-595）→ `is_due`（:477-485：从未拉过 → true；`elapsed_min >= schedule_minutes.max(1)`）→ `claim_source`（:190-230：CAS = source_id+workspace+**updated_at**+status ∈ active/failing+lease 过期+source_generation（0 值三态兼容 :178-188）；$set worker/token/lease + $inc claim_generation；顺带物化 legacy generation :215-217）→ 心跳（:262-293，40s 续约）→ `fetch_source` → abort 心跳 → **`renew_claim`（:295-312：fetch 后先证明仍持有 claim 才开始写知识** :70-73）→ 三分支：
  - `NotModified`（304）→ `finalize_without_content`（:322-358：刷 last_fetched_at、清 error/streak、status=active、可选 etag/hash，claim CAS 释放）；
  - `Fetched` → `should_ingest_content`（:493-495，hash 与 last_content_hash 不等才 ingest）→ 是 → `finalize_ingested_content`；否 → finalize_without_content（含新 etag/hash）；
  - `Err` → `mark_failure`。
  finalize 自身出错（:110-124）：fetch 未失败时补一次 `mark_failure`（fetch 失败已走过唯一的 claim-owned 失败终态，不重复）。
- `fetch_source`（:139-165）：`outbound_fetch::fetch_ingest_url(url, last_etag, kind)`（**条件 GET `If-None-Match`**）；304 → NotModified；非 2xx → bail；kind=`rss`→`render_rss_to_markdown` / `html`→`render_html_to_markdown` / 其它 → bail；解析产物空白 → bail；`content_sha256`。
- `claim_identity_filter`（:232-260）：绑定 source_id/workspace/source_generation/**updated_at/url/kind/schedule_minutes/label/status**/claim_generation/worker_id/claim_token——连"同毫秒 legacy 配置更新"都不能让旧 URL 的抓取结果落库（:237-244）。
- `finalize_ingested_content`（:360-379）：≤3 重试（Transient ∨ dup key）。`_once`（:403-475）**事务**：验 claim（live lease）→ `routes::knowledge::ingest_chunked_text_with_session`（source_name = label ?? "{kind} · {url}"；**该函数内落 draft+needs_review 红线**，模块注释 :8-9）→ 更新 source（etag/hash/streak=0/active）+ `$inc ingest_count += chunk 数` + 清 claim → commit（`commit_chunk_transaction` 复用）。
- `mark_failure`（:611-645）：streak+1；`streak>=3 ∧ status=="active"` → **failing**；`last_fetched_at 距今 > 168h` → **disabled**（覆盖 failing）；last_error 截 500 char；claim CAS 释放（matched≠1 → `ingest_claim_lost`）。
- 解析器：`render_rss_to_markdown`（:497-538）：feed-rs，take 50 条；空标题∧空正文跳过；**id 用 `rss-{idx}`**（entry.id 常为 URL 含 `:`/`/` 不安全 :517-519）；payload JSON {title, summary, body(空则用 title 保证非空 :520-525), businessContext="source: {link}"}；fence 包裹。`render_html_to_markdown`（:540-577）：scraper；title 选择器 + `article, main, [role=main], .content, body` 首命中文本空白归一；空 → bail；**整页单 fence** `html-page`。两者产物均有测试回归「必须能被 block_parser 解析成离散 chunk 且零 warning」（:792-805, :837-849）。
- redline 测试入口（:652-684）：`claim_due_source_for_redline` / `finalize_claimed_content_for_redline`（走真实 claim/finalize 协议，hash 服务端算防绕过）。

### 2.13 `src/knowledge_wiki/lessons_learned.rs`（243 行）—— 模式层聚合

三类 pattern（14d 窗口，`aggregate_lessons_for_workspace` :56-101，极性从 active DomainProfile 解析）：
1. `success`（:25-36 filter）：`approved:true ∧ outcome_status ∈ 正极`（agent_decision_reviews）；
2. `reviewer_misjudge_negative`（:38-45）：`approved:true ∧ reviewer_misjudge_signal=="approved_but_user_negative"`；
3. `blocked_by_safety_guard`（:47-53）：`agent_run_logs.final_review_status=="blocked_by_safety_guard"`（不叠加 lifecycle 谓词，测试 :229-242）。

`upsert_pattern`（:110-175）：count==0 → 跳过；抽样最近 5 条 run_id；`lesson_id={ws}::{kind}` upsert，$set count/sample/updated_at，$setOnInsert `promoted_chunk_id:null` + `review_status:"pending_review"`——lessons 是 peer_case chunk 的**上游候选池**，晋升为 chunk 必经 admin review（:8-10，不绕 review queue）。

### 2.14 `src/knowledge_wiki/page_merge.rs`（518 行）—— 纯函数预校验层

- **`DEFAULT_LOCKED_FIELDS`（:35-44）8 个**：`_id / workspace_id / account_id / document_id / item_id / wiki_type / chunk_type / created_at`。`source_anchors` **明确不在默认锁**（:33-34：由可信正文/quote 变更路径重算、verify gate 校验；历史单数 `source_anchor` 非模型字段不能当安全策略）。
- **`DEFAULT_UNION_ARRAY_KEYS`（:51-59）7 个**：`tags / search_terms / sources / applicable_scenes / not_applicable_scenes / business_topics / product_tags`。`related_chunks` 不在（结构数组按 chunk_id 去重，apply 侧自行处理 :49-50）。
- `BODY_TRUNCATION_THRESHOLD=0.7`（:62）。`RevisionError` 闭集 2 种（:68-82）：LockedFieldInPatch / BodyTruncated。
- `union_array_fields`（:103-131）：**三方入参**（base=标量底 / existing_arr_source=数组既有值来源（必须原始 chunk，KB-09）/ incoming=patch）；BTreeSet 去重、existing 保序在前 + incoming 新元素按序追加；两侧都无该数组 → 不动；非字符串元素跳过。
- `effective_locked_fields`（:157-173）：DEFAULT ∪ existing.locked_fields（owned 去重）；**单一真相源**：apply_chunk_revision 与 admin PUT 两条写入路径都用它（:150-153）；只并入 enforce 的**静默覆盖集**，不并入 apply_field_patch 的硬拒集（否则连坐毙掉同 patch 合法字段 :154-156）。
- `enforce_locked_fields`（:182-195）：merged 中锁定字段强制覆盖回 existing 值（existing 无该字段则移除）——末次防线。
- `is_body_truncated`（:204-216）：`merged_len < existing_len × threshold`；existing==0 → false（新增不算截断）；**边界相等不算截断**（70/100 @0.7 → false，测试 :414-415）；incoming_len 保留未用（:210）。
- `apply_field_patch`（:227-244）：patch 顶层 key 命中 locked → **立即 Err（硬拒整个 patch）**；否则 existing 浅拷贝 + 逐字段覆盖。**不做 union**（调用方先 union :225-226）。
- `compute_chunk_hash`（:272-283）：剔除 `VOLATILE_FIELDS`（:252-260 **7 个**：`_id / updated_at / provenance / usage_stats / dynamic_confidence / integrity_score / id`）→ `bson_to_canonical_json`（:299-327：Document key 递归字典序；ObjectId→hex、DateTime→to_string、Binary→`__bin:hex`、Double 非有限→Null、其它类型 debug 串）→ serde_json 字节 → sha256 hex。字段序无关（测试 :446-450）、volatile 无关（:460-476）。

### 2.15 `src/knowledge_wiki/reviewer_stats.rs`（188 行）—— reviewer 度量

`ReviewerStatsReport`（:24-45）：considered（outcome_status 已回填的 review 数）/ approved / approved_but_user_negative；`pass_rate = approved/considered`、`misjudge_rate = neg/approved`（分母 0 → 0.0，`ratio` :141-147 防 NaN）。

`aggregate_reviewer_stats_for_workspace`（:55-138）：14d（caller 传）窗口 3 个 count（considered / +approved:true / +misjudge_signal）→ upsert `reviewer_stats` 一行，`stat_id={ws}::reviewer`；considered==0 也写锚点。**刻意 workspace 级**（KB-12 :50-54）：reviewer prompt/model 是 workspace 属性，不带 account 维度（与 outcome_metrics 双维不同）。misjudge 是 C2 negative_example 候选的上游信号源，候选入队在 reaction.rs 即时完成，本模块只度量（:15-18）。

### 2.16 `src/knowledge_wiki/structural_proposals.rs`（215 行）—— 结构化写意图提案

**顶部即声明 KB-06 就绪债（:1-3）：生产未接线**——只产 `pending_review` 提案，全仓无 apply worker / 人审 UI 消费方（红线正确但功能未闭环）。

- `StructuralKind`（:28-52）闭集 5 类：split / merge / reclassify / mark_superseded / rewrite_directory_intent。
- `STATUS_PENDING_REVIEW`（:56）本轮唯一合法状态。
- `StructuralProposal`（:63-110）：**序列化层无 apply/commit/delete 字段**（物理上无法表达"已应用"，测试 :177-201）；`new()` 唯一构造口强制 pending_review（:87-89）。字段：proposal_id（ObjectId hex）/ workspace_id / kind / target_chunk_ids / payload（自由 BSON 本轮只存不解释）/ status / rationale / source（`recall_trace`/`rule`/`human`）/ created_at。
- `structural_proposals()`（:114-116）：`db.raw()` 懒创建集合（不动 db/mod.rs，隔离红线）。`propose_structural_change`（:122-145）：只 insert，不改 chunk、不删数据、不重算 catalog。

---

## 3. 跨文件机制

### 3.1 生产召回数据流 vs 管理台 chat 数据流（完整对比）

| 维度 | 生产召回（user-ops） | 管理台 chat（知识工作站） |
|---|---|---|
| 入口 | `gateway` → `route_operation_knowledge*`（knowledge_router.rs:448-555） | `routes/knowledge/chat.rs:1925` → `chat_reply_with_tools_loop`（chat_tool_loop.rs:87） |
| 语料可见域 | **verified-only**：预载 `load_operation_knowledge`（active+verified ≤200）+ agent 侧 `CatalogFilter::default()`（include_unverified=false，router.rs:740）+ `open_chunk` verified 硬门（knowledge_agent.rs:1268-1275） | 三件套读预载 verified 集；六件 chat-only 直查 DB **无 status/verified 门**（knowledge_tools.rs:1233-1244）——可审 draft/needs_review/archived，但 search/open_slice 的正文对非 verified **redact**（:342-346, :488-492） |
| 循环形态 | LLM 自主 action 循环 ≤4 轮（knowledge_agent），单 action/轮 | decision_phase 驱动 ≤4 轮（clamp :127），**每轮 ≤6 个 toolCalls**（:280-283） |
| 预算 | `RunBudget` LLM call 计数：`available_llm_calls_before_tail(4/3/1+dual)` 压缩 max_rounds（router.rs:715-735）；`should_stop_optional_llm_calls` 每轮检查（agent.rs:768-786） | `RunBudget.record_tool_call` 每工具占 1 槽（tools.rs:615-617）；budget_exceeded → force_stop（loop.rs:329-338） |
| 超时 | 无显式墙钟（靠轮数+budget） | 总 30s 硬超时 + 单工具 5s（loop.rs:36 / tools.rs:51） |
| 失败处理 | invalid_action/empty_answer → continue 耗轮次；兜底 truncated | 失败连击 ≥3 强制结束（loop.rs:322-328）；trace ≤32 |
| 结果强校验 | `filter_answer_against_opened_chunks`：cite⊆opened ∧ verified ∧ 非 contradiction ∧ quote 是原文子串 ∧ anchor 必填且互含（agent.rs:1608-1688） | 无 cite 校验（chat 是运营助手不是对客证据链）；final 声明用知识但未咨询 → risk（loop.rs:231-243） |
| 降级路径 | fallback_rank 静态回填（navigation_only，**不可授权产品背书**，router.rs:1037-1049） | 无回填；耗尽即强制 final |
| 副作用 | 只读语料；fire-and-forget gap 信号 + StructuralProposal + record_chunk_hit + usage log | **永不写库/outbox/mcp**（tools.rs:514）；propose_repair 只出建议 |
| 流式 | SSE：Step/Token（AnswerStreamer 解码顶层 answer）/Failed/Final | 无 token 流（结果整体返回） |

### 3.2 一个 chunk 的全生命周期（导入 → verified → 被引用 → 反馈）

1. **导入**：三条入口殊途同归到 fence 分块 + `ingest_chunked_text*`：(a) 管理台手动导入（routes/knowledge/import.rs，超出本次范围）；(b) `ingest_worker` 自动拉取——条件 GET → RSS/HTML 渲染成 fence 文本（ingest_worker.rs:497-577）→ `block_parser::parse_chunk_blocks` 逐块独立校验（unsafe id/截断/重复/空块全进 warning 不污染整批）→ 事务内落库（:403-475）。**所有 AI/导入产物恒 `status=draft + integrity_status=needs_review`**（chunk_revisions.rs:17-18 + apply_server_owned_lifecycle :207-214；PrincipalAuthorized 是唯一例外 :107-110）。
2. **编辑**：任何变更走 `apply_chunk_revision` 七步流水线（§2.9）；触碰 12 个 review-sensitive 字段的 Patch/Split/Merge/Rollback **把已 verified 的 chunk 打回 draft+needs_review**（:189-214）；同一事务写不可变 `chunk_revisions` 行 + updated_at CAS 替换 + 父文档 catalog generation 推进 + rebuild job 入队。
3. **verify**：只有 Human/Rule/PrincipalAuthorized 源的 `RevisionOp::Verify` 能透传 `integrity_status="verified"`（:223-229）；AI 源即使 op=Verify 也被强制压回（测试 :1205-1220）。verify 时的锚点校验（`source_anchor_for_quote` 模糊匹配）在 routes 侧 verify gate（本次范围外），chat 的 `verify_anchor` 工具是同算法的只读预检（tools.rs:1204-1219）。
4. **被召回**：verified 后进入 `load_operation_knowledge` 的 200 条窗口与 `list_catalog` 的 400 条窗口；`rank_key` 按 query 相关度 × trust/recency 排序（superseded ×0.1、过期 ×0.5）；agent open→cite 后经严格锚点过滤成为 `route.selected_chunk_ids` + `evidence_excerpts`，供 Reply Agent 注入 prompt（`format_operation_knowledge_for_prompt_with_roles`）与产品背书硬闸（`route_used_knowledge_ids`）。
5. **反馈回写**：每次 run `write_knowledge_usage_log`（usage log 行 + 每 id `record_chunk_hit` `$inc` 实时计数）→ `feedback_worker` 每 10 分钟（默认）per-ws lease 跑：`refresh_usage_stats_and_confidence`（30d 窗口按真实用户反应三态 + 成交追认重算 `hit/blocked_count_30d` 与 `dynamic_confidence`——该值直接回灌 `rank_key.confidence_micros` 与 catalog 排序）→ structural lint 产 9 类信号 → sweep 自愈消解 → lessons/reviewer_stats 聚合。
6. **在线缺口闭环**：召回失败签名（recall_miss / low_yield / citation_format_rejected）由 `answer()` fire-and-forget 落 `knowledge_gap_signals`（source=recall_trace，只增不消解）；recall_miss 携原始 query（+LLM 追问）供运营对话式补录；low_yield 顺带 emit Split 类 `StructuralProposal`（恒 pending_review，无消费方=KB-06）。
7. **版本替代与退场**：新版通过 Split/Merge 透传 `superseded_by`/`previous_version_id`；读路径三处 redirect（open_chunk/follow_relations 查 DB 版 `resolve_superseded`、open_slice 内存版）保证 agent 永远 cite 现行 verified 版；archive 后 `cleanup_dangling_refs` 清悬链（source=Rule，不降级他链 verified 状态），sweep 把关联信号自愈消解。

### 3.3 cache 一致性链

答案缓存 key 绑定：语料签名（**全 scope 含 draft/archived** 的 _id+updated_at，agent.rs:2044-2075）+ provider 三元组 + prompt_pack_version + filter/query 规范化 + max_rounds；写入前二次比对 provider（agent.rs:1046-1058）防热切换污染；`truncated/cancelled` 不缓存（cache.rs:127-130）；shadow run 不读写 cache（agent.rs:712）。任何 chunk 编辑（哪怕未展示的）→ updated_at 变 → 签名变 → 自然失效。

---

## 4. 事实卡速查

**工具名闭集（chat，9 个）**：`knowledge.list_catalog` / `knowledge.search` / `knowledge.open_slice` / `knowledge.audit_completeness` / `knowledge.search_chunks` / `knowledge.propose_repair` / `knowledge.analyze_logs` / `knowledge.open_document` / `knowledge.verify_anchor`（knowledge_tools.rs:91-101）。
**knowledge_agent action 闭集（5 个）**：`list_catalog` / `open_document` / `open_chunk` / `follow_relations` / `answer`（knowledge_agent.rs:225-257）。

**轮次/预算/超时上限**：
- knowledge_agent：`MAX_ROUNDS=4`（agent.rs:45）；catalog 页 30 / 候选窗 400 / 摘要 120 char / open 批 8 / follow 上限 16 / 预取正文 3 / redirect 8 跳 / catalog 合并上限 60。
- router 预算保留：GeneratedReply=4（+1 dual）、ExistingCandidate=3（+1 dual）、PreviewOnly=1（router.rs:568-576）；fallback top N=5（:788）。
- chat loop：max_loops=4、每轮 toolCalls≤6、总 30s、单工具 5s、失败连击 3、context 8000 chars、trace 32（chat_tool_loop.rs:36-44 + knowledge_tools.rs:51,104）。
- chat 工具：list_catalog 同 kind ≤2 次、limit 默认 50 上限 200；search query ≤200 char、top_k 默认 8 clamp[1,32]、snippet ≤200；open_slice K 默认 4 clamp[1,16]；open_document 默认 4000 上限 8000 chars；analyze_logs 窗口默认 24h 上限 72h、返回 ≤32 行 / top 8 chunk。
- cache：TTL 300s、容量 256（cache.rs:26-29）。
- worker：catalog_rebuild lease 60s/batch 16/attempts 5/退避 ≤300s；feedback lease 300s/心跳 60s；ingest lease 120s/failing 阈值 3/disabled 168h。

**wiki_type 权重序（9+默认）**：thesis 90 > synthesis 80 > methodology 70 > finding 60 > comparison 50 > concept 40 > entity 30（None 同）> source 20 > query 10 > 未知 0（agent.rs:1781-1794）。

**chunk_type（DEFAULT 销售四态）**：product_fact（fallback）→ style_template → peer_case → negative_example（router.rs 测试 :1286-1293 顺序锁；可被 DomainProfile.chunk_roles 替换）。

**integrity_status 观察到的取值**：`verified` / `needs_review` / `draft`（agent.rs:1246-1247 注释）；另 Rule-verify 可写任意如 `needs_human_audit`（chunk_revisions.rs 测试 :1181-1202）。status：`active` / `draft` / `archived`（+ ingest source 自身的 active/failing/disabled）。

**RevisionOp 闭集（10）**：create/patch/split/merge/rollback/archive/restore/verify/unverify/reject（chunk_revisions.rs:65-94）。**ProvenanceSource 闭集（5）**：ai/human/rule/imported/principal_authorized（:97-139）。**StructuralKind 闭集（5）**：split/merge/reclassify/mark_superseded/rewrite_directory_intent（structural_proposals.rs:28-52）。

**锁定字段清单（DEFAULT_LOCKED_FIELDS，8）**：`_id, workspace_id, account_id, document_id, item_id, wiki_type, chunk_type, created_at`（page_merge.rs:35-44）；生效锁 = DEFAULT ∪ chunk.locked_fields（:157-173）。patch 携带 DEFAULT 锁 → 硬拒；运营锁 → 静默覆盖回。

**数组 union 字段清单（DEFAULT_UNION_ARRAY_KEYS，7）**：`tags, search_terms, sources, applicable_scenes, not_applicable_scenes, business_topics, product_tags`（page_merge.rs:51-59）；`related_chunks` 例外（整数组 patch，chunk_revisions.rs:928-929）。

**REVIEW_SENSITIVE_PATCH_FIELDS（12）**：title, summary, body, knowledge_type, business_context, applicable_scenes, not_applicable_scenes, product_tags, business_topics, source_quote, source_anchors, domain_attributes（chunk_revisions.rs:174-187）。

**hash volatile 剔除（7）**：`_id, updated_at, provenance, usage_stats, dynamic_confidence, integrity_score, id`（page_merge.rs:252-260）。

**70% 阈值字段**：`body` / `summary` / `answer` 三个 text 字段逐一独立判（chunk_revisions.rs:321-333；`BODY_TRUNCATION_THRESHOLD=0.7` page_merge.rs:62；merged < existing×0.7 拒收，边界相等放行，existing=0 放行）。

**dynamic_confidence 公式**（gap_signals.rs:1314-1333）：
```
base = integrity_score ?? 0.5
penalty = (valid_to < now ? 0.3 : 0) + (anchor 悬空且有原文 ? 0.3 : 0)
h+b < min_samples          → clamp(base − penalty, 0, 1)
否则 hit_rate = h/(h+b)     → clamp(base×0.6 + hit_rate×0.4 − penalty, 0, 1)
```
hit/block 来源（real_outcome 默认开）：run_id join decision_reviews.outcome_status 三态（Hit=正极 / Block=负极 / **Censored=其余一切删失**）；成交追认：已核实成交前最近 3 条 usage log 翻 Hit（DEAL_ATTRIBUTION_WINDOW_TURNS=3 :882）。rank_key 侧另有独立降格：superseded ×0.1、过期 ×0.5（agent.rs:1836）。

**gap signal kind 全集（12）**：结构 9 类 `orphan / broken_link / missing_chunk / no_outlinks / low_confidence / stale / suggestion / dangling_anchor / contradiction`（gap_signals.rs:200-396）+ 在线 3 类 `recall_miss / recall_low_yield / citation_format_rejected`（agent.rs:1893-1997）。severity：error=missing_chunk, contradiction；high=recall_miss, citation_format_rejected；warning=broken_link, low_confidence, stale, dangling_anchor；medium=recall_low_yield；info=orphan, no_outlinks, suggestion。status 流转：pending → auto_resolved（stage1 rule）｜llm_resolved（预留）｜applied/dismissed（admin，db/mod.rs:386 注释）。dedup：link 类 `link::{from}::{to}` 跨 kind 共享；其余 `{kind}::{normalize_title}`；持久键 = sha256，唯一 partial index `{workspace_id, dedup_key} where status=="pending"`（db/indexes.rs:180-194）。

**关系 kind（6）→ 角色**：references/requires/clarifies/refines/未知 → Support；contradicts → Contradiction（可看不可 cite）；superseded_by → Version（不扩散，走 redirect）（agent.rs:1534-1541）。

**coverage/risk 映射**（router.rs:794-906）：cited+quotes → enough/low；cited 无 quotes → weak/low；fallback 回填 → weak/medium（navigation_only=true，不授权）；空 → missing/medium；预过滤跳过 → not_required/low；无语料 → missing/medium；预算保留跳过 → missing/medium。

---

## 5. 偏差与疑点

1. **`DocEntry` 死代码 / #619 文档级目录未落地**：`DocEntry`（agent.rs:133-145）与模块 doc（:6-8「round 1 额外注入文档级目录（catalogSummary/routingMap 导航卡片）」）、`MAX_ROUNDS` 注释（:41-42「round1 看文档目录…正好 4 轮」）均描述 round 1 注入文档导航卡；但全文件（grep 亲验）无任何 `DocEntry` 构造点，`answer_inner` round 1 只有 chunk 级 catalog。`open_document` action 本身有效，但 agent 只能从 catalog 条目无从得知 documentId（prompt :1753 也说"没有 documentId 时无需 open_document"）——分层召回的"先文档后原子"链路实际不可达，OpenDocument 近乎不可用。文档注释与实现不符。
2. **`AnswerStreamer` 注释与实现不符**：doc 注释（agent.rs:437）称"只认顶层 answer 键，忽略嵌套对象里的同名键（用 depth 计大括号层级）"，但 `locate_answer_value_start`（:497-529）是朴素子串定位，无任何 depth 计数。若某轮 JSON 在顶层 answer 之前出现嵌套 `"answer":"..."`（如 sourceQuotes 内），token 流会提前把嵌套值当正文下发。当前 answer 轮 schema（citedChunkIds→sourceQuotes→answer 或直接 answer）下风险低，但契约描述失真。
3. **corpus 窗口错位可把合法引用降格为 fallback**：`cited_in_corpus` 求交的 corpus 是 `load_operation_knowledge` 的 **200 条 priority 序**窗口（router.rs:64-84），而 agent 的 catalog 来自 **400 条 confidence 序+相关度重排**窗口（agent.rs:1141-1170）。agent 完全可能 cite 一条排在 corpus 窗口外（201+）的 verified chunk——此时 `cited_in_corpus` 为空、走 fallback_rank 分支（router.rs:794），真实证据链被降格为 navigation_only（且 `evidence_excerpts` 非空 + `requires_evidence=true` 与 `coverage=weak/fallback` 并存，语义拧巴）。>200 条 verified 语料的 workspace 才会踩到；与 #619 修复的初衷（消除窗口截断漏召）方向相反。
4. **user-ops 三件套与 chat-only 六件的入参命名约定不一致**：前者 args 无 serde rename（snake_case：`chunk_ids`/`top_k`，tools.rs:132-157），后者 `rename_all="camelCase"`（`chunkId`/`topK`，:516-580）。同一个 chat 循环内 LLM 需混用两套命名；有测试锁死现状（:1590-1702）但对 LLM 出错率不友好——`open_slice` 传 `chunkIds` 会被静默当空数组 → `invalid_input`。
5. **`open_slice` 的 redact 分支在生产预载集合内不可达**：`KnowledgeRuntime.chunks` 恒来自 verified-only 预载（router.rs:64-84），`exec_open_slice` 的非 verified redact（tools.rs:486-492）只在测试构造下触达。防御性代码，无害，但阅读时易误以为 chat open_slice 能看 needs_review 正文（实际它根本看不到非 verified chunk——查不到 → `unknown_chunk_id`）。与之相对 `search_chunks`（直查 DB）确实能召回 draft（snippet redact）。
6. **`write_knowledge_usage_log` 的"fire-and-forget"描述不精确**：注释（router.rs:1127-1128）称 record_chunk_hit 「fire-and-forget——不阻塞 gateway 决策」，实现是循环内**顺序 await** 每次 update、仅 `let _=` 吞错（:1134-1147）；对比 agent.rs:317 的真 `tokio::spawn`。N 个 chunk id = N 次串行 DB 往返，仍在 gateway 请求路径上。
7. **block_parser 行首缩进的 fence 终止符会截断 body**：`---END CHUNK---` 判定是 `trim_start` 后整行相等（block_parser.rs:93,128），即**左侧缩进的**终止符也生效；doc 注释（:21-23）说"行内出现…不在行首 → 当作普通正文"，测试 :439-446 只覆盖了"token 后还有别的字符"的场景。若 JSON body 被 pretty-print 且某行恰为缩进的 `---END CHUNK---`（概率极低，单行 JSON 产物不受影响），块会被误终止。
8. **gap_signals 模块 doc 计数过期**：注释说"8 类 signal kind"（gap_signals.rs:11），实际结构 lint 产 9 类（dangling_anchor 后加，:338-354），在线另有 3 类 recall kind。仅文档滞后。
9. **`render_one_document` 的 catalog 无 verified 门**：catalog_rebuild.rs:441-453 只过滤 `status:"active"`，needs_review/draft chunk 的标题摘要会进 `catalog_summary_persisted`（integrity 字段有标注）。与召回面的 verified-only 是两个层（persisted catalog 是管理导航非注入 prompt），但若未来有人把它接进 agent prompt 需先加门。
10. **`knowledge_router.rs:775-780` 注释重复**：fallback_rank 的六行说明原样出现两遍（复制粘贴残留），无行为影响。
11. **性能观察**：`resolve_superseded` 每跳 2 次 find_one、`follow_relations` 对每个关系目标都调它（agent.rs:1396-1397）——高关系密度 + 长版本链的最坏 DB 放大 ~16×16 次查询/轮；`refresh_usage_stats` 逐 chunk 单条 update（gap_signals.rs:961 声明假设 <5000 条）；`cleanup_dangling_refs` 全表扫 + 每受影响 chunk 一个独立事务。均有注释声明规模假设，非缺陷。
12. **`structural_proposals` 无消费方（KB-06，模块自认）**：提案只进不出（structural_proposals.rs:1-3），`recall_low_yield → Split` 的闭环末端悬空；同样 `sweep_stale_signals` 的 stage 2 LLM 批只预留未接（gap_signals.rs:28-31）。属"红线正确但功能未闭环"的显式就绪债。
13. **`list_catalog` 的 `filter.status` 可被上层传任意值**（agent.rs:1119），且 `include_unverified=true` 时可见 needs_review——越权面收敛于调用方：`/api/knowledge/ask`、`/ask/stream` 均硬编码 `include_unverified:false`（routes/knowledge/sources_meta.rs:549,649 亲验），router 恒用 default filter；但 status 字段本身经 ask 接口透传（:546），管理员可查 archived 语料（verified 门仍在）——设计上说得通（admin 面），记录备查。

---

## 6. 覆盖自证

以下每个文件均以 Read 工具分段读取全文（无跳读），行数与 `wc -l` 输出一致：

| # | 文件 | 总行数 | 读取方式 |
|---|---|---|---|
| 1 | `src/agent/knowledge_agent.rs` | 3048 | 4 段：1-800 / 801-1600 / 1601-2400 / 2401-3048 |
| 2 | `src/agent/knowledge_agent/cache.rs` | 377 | 整读（目录 `ls` 确认仅此 1 文件） |
| 3 | `src/agent/knowledge_router.rs` | 1741 | 2 段：1-900 / 901-1741 |
| 4 | `src/agent/knowledge_tools.rs` | 1784 | 2 段：1-920 / 921-1784 |
| 5 | `src/agent/chat_tool_loop.rs` | 719 | 整读 |
| 6 | `src/knowledge_wiki/mod.rs` | 29 | 整读 |
| 7 | `src/knowledge_wiki/block_parser.rs` | 468 | 整读 |
| 8 | `src/knowledge_wiki/catalog_rebuild.rs` | 867 | 整读 |
| 9 | `src/knowledge_wiki/chunk_revisions.rs` | 1354 | 2 段：1-700 / 701-1354 |
| 10 | `src/knowledge_wiki/feedback_worker.rs` | 335 | 整读 |
| 11 | `src/knowledge_wiki/gap_signals.rs` | 2519 | 3 段：1-850 / 851-1700 / 1701-2519 |
| 12 | `src/knowledge_wiki/ingest_worker.rs` | 857 | 整读 |
| 13 | `src/knowledge_wiki/lessons_learned.rs` | 243 | 整读 |
| 14 | `src/knowledge_wiki/page_merge.rs` | 518 | 整读 |
| 15 | `src/knowledge_wiki/reviewer_stats.rs` | 188 | 整读 |
| 16 | `src/knowledge_wiki/structural_proposals.rs` | 215 | 整读 |

合计 **15,262 行**（wc -l 报 14,883 + cache.rs 377 + 各文件末尾无换行差异 ±2）。另为核验交叉断言补充 Grep 亲验：`src/agent/budget.rs:173-178,202-210,237-247`（预算原语）、`src/agent/types.rs:83-90,1601-1653,1660-1668,1707-1711`（ToolCallRequest/KnowledgeRouteResult/SelectedChunkRanking/KnowledgeRuntime）、`src/agent/runtime.rs:313-318,429-430,954-955`（top_k/max_k 默认与 clamp）、`src/db/indexes.rs:180-194,2967-3021`（gap signal 索引）、`src/db/mod.rs:370-394`（集合 accessor）、`src/routes/knowledge/sources_meta.rs:518-553,613-674`（/ask 与 /ask/stream 强制 include_unverified=false）、`src/routes/knowledge/chat.rs:1777-1932,1997-2009`（chat loop 接线与 anchor_match 注入）、全仓 `DocEntry` 使用点检索（仅定义，零使用）。

---

## 追记：25 号交叉验证回写（2026-08-13，主会话执行）

- 归因精化（25 号 §4）：本记录全部 5 个抽验疑点成立；红线合并总表（19 处）见 25 号 §3。**关键补充：`Imported+Create` 组合在 harness 层（apply_server_owned_lifecycle）不触发强制降级**（op=Create 非敏感 op、source=Imported 非 Ai）——import 路径的 draft+needs_review 红线完全依赖 `import.rs:1549-1556` 与 `:2277-2285` 两处 routes 显式赋值，删掉即破洞（已在 25 号总表加 ★ 标记，改 import 路径时必查）。
