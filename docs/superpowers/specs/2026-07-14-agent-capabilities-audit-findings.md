# agent 旁挂能力深度逻辑审查 findings 台账（第一批）

> 接续 2026-07-11 主链路深度逻辑审查（53 findings，P0-P3 + F-01 全闭环）之后的新一轮。本批范围 = agent 旁挂能力子系统。**只审不修**——先出完整台账，再按 P0-P3 分批修（各走独立 brainstorming→SDD→PR）。
>
> 设计：`docs/superpowers/specs/2026-07-14-agent-capabilities-audit-design.md`
> 计划：`docs/superpowers/plans/2026-07-14-agent-capabilities-audit.md`

## 审查范围（4 簇 / ~10.6k 行）

- **簇A 记忆固化**：`src/agent/memory.rs`(3291) + `src/agent/consolidation_window.rs`(77)
- **簇B 标签体系**：`src/agent/taxonomy.rs`(1036) + `src/agent/decision_taxonomy.rs`(427) + `src/agent/tag_evidence.rs`(101) + `src/agent/bayesian_slots.rs`(202)
- **簇C 通用化底座**：`src/agent/domain_profile.rs`(2454) + `src/agent/domain.rs`(107) + `src/agent/domain_signals.rs`(456) + `src/agent/dimension_registry.rs`(449)
- **簇D 节流准入**：`src/agent/simulation.rs`(265) + `src/agent/pacing.rs`(51) + `src/agent/quiet_hours.rs`(357) + `src/agent/entitlements.rs`(1311)

## 方法论

- 4 个只读审查 subagent 分簇并行审（继承 Opus）+ 主控逐条亲验 file:line（复核属实性 + 因果链，驳回夸大）。
- 两态：**PLAUSIBLE**（纯读码推断）/ **CONFIRMED**（可构造推荐配置下真实触发）。
- 元家族：设计声称的不变量/闭环/口径，实现层有旁路 / 缺口 / 非原子窗口 / 新旧不对称。

## 严重度校准（防夸大）

- **High**：推荐配置下**确定性发生**的核心交互失效 / 红线破坏。
- **Medium**：需多条件叠加、或依赖 DB/LLM 瞬时故障注入才触发。
- **Low**：观测项 / 边缘 / 就绪债 / 死代码 / 文档-代码漂移。

## Finding 字段模板

```
### [X-NN] 一句话标题
- 入口频道: —
- 所属簇: A|B|C|D
- 类型: 幂等|竞态|错误处理|一致性|逻辑正确性|红线|文档-代码漂移|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 环节汇总（收尾时填）

- 总 findings 数：（待填）
- 严重度分布：H / M / L（待填）
- 元家族归纳：（待填）
- 后续 P0-P3 修复路线建议：（待填）

---

## 簇A 记忆固化 findings

（主控亲验后填入）

## 簇B 标签体系 findings

> 主控亲验结论：**8 铁律逐条复核全部 HOLDS**（三层物理隔离 / 证据 fail-closed / bayesian·personality 只写不进决策 / 候选不阻断运行）。最关键红线③在实现层零旁路——`.personality_profile`/`.bayesian_signals` 全部读点仅前端投影+测试+写回自身 clone，决策/planner/prompt/状态机无读路径，planner 有契约测试钉死。3 条 finding 均属质量瑕疵，非红线破坏。

### [B-01] 双活 taxonomy 分类路径对同一未知值重复 upsert → occurrences 翻倍 + confidence/evidence 落库竞态
- 入口频道: userOps（approved 发送）
- 所属簇: B
- 类型: 一致性 / 写侧非确定性（重复逻辑）
- 严重度: Low-Medium（主控亲验裁定：不破坏任何铁律、候选不阻断运行仍成立；但 admin 审核队列 occurrences「出现频次」被系统性放大约 2×，confidence/evidence 元数据取决于两路径落库竞态，影响运营对候选优先级判断——可观测性/数据质量瑕疵，非功能错误）
- 现象/风险: 同一 approved 回复的同批维度被分类两次并写同一幂等键：①决策路径 `decision.rs:998→classify_decision_tags` 产 risk `taxonomy_candidate:{kind}:{raw}` 并 `tokio::spawn` fire-and-forget upsert(confidence=0/evidence=None)；②网关路径 `gateway.rs:1693→compute_taxonomy_guard_outcome` 产 risk `taxonomy_candidate_new:{kind}`（不带值、词表不同）并 await upsert(confidence=50/evidence=Some)。
- 根因（亲验 file:line）: `taxonomy.rs:401-413` upsert_candidate 命中 pending 时 `$inc occurrences:1`（亲验）；`gateway.rs:1714-1722` confidence=50/evidence="user-ops decision path"（亲验）；`decision_taxonomy.rs:137-138` spawn_candidate_upserts fire-and-forget confidence=0/evidence=None（亲验）。两路径写同键→单轮 occurrences+2；spawn(时机不定) vs await(inline) 谁先落库不定→confidence(0 vs 50)/evidence 非确定。`decision_taxonomy.rs:93-96` 注释知晓双写但只论证 display_name 幂等无害，未覆盖 occurrences 双计与 confidence 竞态。两 risk 词表不一致致 review.risks 堆两种格式（gateway.rs:1706-1710 去重仅按字面量不去重异格式，冗余无害均不进硬门）。
- 复现设想: managed 联系人触发 approved 回复，LLM 输出字典外 customer_stage；审计 taxonomy_candidates 该文档单轮后 occurrences=2，多轮观察 confidence 在 0/50 跳变。
- 验证状态: PLAUSIBLE（两路径均亲验会在 approved 发送时先后写同键；竞态具体落库值未生产实测）
- 修复建议: 收敛为单一分类点（删决策路径 upsert 只保留 risk 收集，落库统一交 gateway；或反之）+ 统一 risk 词表；若为覆盖 revision 改值保留双路径，则命中幂等键时不再 `$inc occurrences`（改 `$max last_seen_at` 语义）消双计。
- 状态: Open

### [B-02] `taxonomy::approve`/`reject` 为死代码，与 routes 层活实现并行（别名处理已不等价）
- 入口频道: —（admin 审核）
- 所属簇: B
- 类型: 死代码 / 同一不变量的平行实现（漂移隐患）
- 严重度: Low（主控亲验裁定：生产只走 routes 实现、死函数不可达；但候选→字典晋升关键不变量存在两份实现，未来改一份忘改另一份会静默漂移）
- 现象/风险: `taxonomy.rs:468` approve、`:557` reject 均带 `#[allow(dead_code)]`；生产审核入口 `routes/admin_taxonomy_candidates.rs`（经 management.rs:2328 调 approve_taxonomy_candidate_inner）重新实现了一份 approve。
- 根因（亲验 file:line）: 死实现 `taxonomy.rs:468` `#[allow(dead_code)] pub(crate) async fn approve`（亲验），写字典 entry 时 aliases 不含 raw_value；活实现 `admin_taxonomy_candidates.rs:170-178` 自动把 `candidate.raw_value` 加进 aliases（亲验，便于历史 run 命中）+ 支持自定义 canonical id/label/description。两份逻辑别名处理不等价，死实现是历史残留。
- 复现设想: 无运行时可复现（死码不可达），靠 code review 发现。
- 验证状态: CONFIRMED（死代码标注 + 活实现均亲验）
- 修复建议: 删除 `taxonomy::approve`/`reject`（及 `#[allow(dead_code)]`），或把 routes 实现重构为调用它并把「raw_value 自动入 alias」下沉进 `taxonomy::approve` 消除平行实现。
- 状态: Open

### [B-03] `evidence_strength` 二次按 `e.turn` 索引 window，依赖「与 resolve 同一 window」的隐式契约（当前安全，潜在耦合）
- 入口频道: —
- 所属簇: B
- 类型: 潜在耦合 / 未由类型强制的前置条件
- 严重度: Low（主控亲验裁定：全部现有调用点用同一 window，当前无 bug；仅为将来异窗调用埋隐患）
- 现象/风险: `tag_evidence.rs:41-46` evidence_strength 判强证据时再次 `window.get(e.turn as usize)` 读消息方向；e.turn 来自 resolve_evidence（已校验 idx≥0 且 window 内有值），对同一 window 二次查表安全，但函数签名把 evidences 与 window 拆两参未由类型保证同源——将来用「旧窗口 resolve 出的 evidences」配「新窗口」会索引错位，弱证据可能被误判强→实时写 stage。
- 根因（亲验 file:line）: `tag_evidence.rs:41-46` 二次索引（亲验）；现有两调用点均传同一 window（gateway.rs:4001-4010 resolve_evidence 与 evidence_strength 同 window；build_observed_dimensions gateway.rs:3784-3792 内联算强证据逻辑一致），故当前无触发路径。
- 复现设想: 新增消费持久化历史 evidences 的调用点、用当前轮 window 传入 evidence_strength 即触发错位。
- 验证状态: PLAUSIBLE（当前所有调用点亲验安全；隐患为推演）
- 修复建议: 非必须。加固可让 Evidence 携带自证方向信息（缓存 direction），或把 evidence_strength 收进只接受「resolve 当场产出的 (evidences,window) 对」的封装，使异窗调用类型层不可表达。
- 状态: Open

## 簇C 通用化底座 findings

（主控亲验后填入）

## 簇D 节流准入 findings

> 主控亲验结论：**影子隔离性（发送侧红线）HOLDS**——simulate_user_dialogue 绝不触发真实 MCP 发送/outbox/outbound 写（有回归门 tests/simulation_no_sideeffect_integration.rs）。负向结论（亲验干净、不入 finding）：D-3 pacing 边界正确（`<` 恰好到点放行 + fail-soft 有意取舍）、D-4 quiet_hours 时区/跨午夜/边界/醒来时刻全对且单测锁死、D-6 entitlements.rs 全程 fail-safe/fail-closed 主文件内零 fail-open。

### [D-01] 影子模拟真写 `operating_memories`（create+seed）——孤儿写入，非发送侧泄漏
- 入口频道: userOps（影子演练 POST /simulations/dialogue）
- 所属簇: D
- 类型: 逻辑正确性（副作用越界）
- 严重度: Low（主控亲验裁定：写的是 operating_memories 非发送侧集合；seed 仅派生自 contact 自身档案、**不含模拟对话内容**；与真实首触达同函数近似幂等；不构成客户副作用、不违反"发送侧零副作用"红线）
- 现象/风险: 对从未真实运营的 contact 跑影子演练会在生产库静默落一条 operating_memories 文档并可能 seed memory card，与"影子=只读演练"心智模型 + 路由自身 `apply_memory=false` 门矛盾。
- 根因（亲验 file:line）: `simulation.rs:86` 影子内联无条件 `load_or_create_operating_memory`（亲验）；`memory.rs:943-946` create 分支真 insert_one；`memory.rs:832-852` 已存在无信号时 seed 走 OCC update_one 真写；对照 `routes/simulations.rs:41-44` 影子显式拒 apply_memory=true，说明设计期望不落 memory，但 load_or_create 的 create/seed 侧写绕过该意图。
- 复现设想: 对无 operating_memories 行的 managed contact 调影子 dialogue，跑完查该集合多一行（memory_card 由 contact 档案 seed）。
- 验证状态: CONFIRMED（写路径+调用点亲验；"不含对话内容"亦亲验——seed 在消息循环前、用未经模拟的 contact）
- 修复建议: 若要影子严格只读，simulate_user_dialogue_inner 改用"只读加载、缺失则构造内存态默认 memory 不落库"分支（新增 load_operating_memory_readonly 或给 load_or_create 传 persist=false）。属产品意图裁决项，非红线，可暂缓。
- 状态: Open

### [D-02] 影子模拟真写 `llm_call_logs`（每次 LLM 调用一行）——by-design 观测，非泄漏
- 入口频道: userOps（影子演练）
- 所属簇: D
- 类型: 逻辑正确性（副作用越界·设计内）
- 严重度: Low（主控亲验裁定：影子复用真实 decide/review LLM，记录 token/延迟/状态属正当观测；日志带影子 run_id 不污染发送侧/客户态；属设计内行为）
- 现象/风险: 影子演练在 llm_call_logs 累积行、消耗真实 token 预算（simulation_token_budget 独立计费默认 300000），对成本/配额有真实影响但无功能副作用。
- 根因（亲验 file:line）: `mod.rs:215` generate_agent_json（唯一 LLM JSON 入口，影子 decide/review/knowledge 全经它）→ `mod.rs:239-306` cache_hit/success/failure 三分支均 llm_call_logs 写。
- 复现设想: 跑影子 dialogue 后查 llm_call_logs 有对应 run_id 的行。
- 验证状态: CONFIRMED
- 修复建议: 无需修（by-design）。若追求"影子零写入"可给 generate_agent_json 传 shadow 标记跳过日志，但牺牲 LLM 成本可观测性，不建议。
- 状态: WontFix（by-design 观测，主控亲验确认非缺陷，留痕）

### [D-05] quiet_hours 仅单时段+小时粒度（无多时段/分钟精度）——设计范围说明
- 入口频道: —
- 所属簇: D
- 类型: 就绪债（表达力）
- 严重度: Low（主控亲验裁定：非逻辑缺陷，是既定产品设计=运营方作息一个连续睡眠窗）
- 现象/风险: 无法配"午间 12-14 + 夜间 22-08"多窗，也无法配 22:30 半点边界（in_quiet_hours 入参 u32 小时）。若某行业需多窗/分钟级作息当前引擎不支持。
- 根因（亲验 file:line）: `quiet_hours.rs:29` in_quiet_hours(now_hour:u32,start:u32,end:u32) 单区间小时入参；`runtime.rs:70-72` quiet_hours_start/end:u32 单对。
- 复现设想: 现状属实，无需运行时复现。
- 验证状态: CONFIRMED（现状属实）
- 修复建议: 无需修（超出当前产品范围）。未来若需多窗把 (start,end) 升级 Vec<(u32,u32)> 或引入分钟粒度，in_quiet_hours 改 any-match。
- 状态: WontFix（既定产品范围，留痕）

### [D-07] `load_active_products` best-effort 吞错 → 静默 fail-closed 降级 + 无法区分"无产品"与"DB 错误"
- 入口频道: userOps / command（产品报价决策）
- 所属簇: D
- 类型: 错误处理（可用性降级·观测缺口）
- 严重度: Low（主控亲验裁定：方向正确 fail-closed 不误放行，但错误被完全吞掉致"合法报价被误 block"且无从诊断——观测缺口非安全漏洞）
- 现象/风险: products 集合瞬时错误时 load_active_products 返空 Vec，与"确实没配产品"不可区分。后果：①决策 prompt 产品目录/持有投影段变空；②priced_from_active_catalog 恒 false→若该 run 又无 verified chunk，`review/gates.rs:665` 触发 blocked_unverified_product_claim 误 block 本可正确报价的回复。
- 根因（亲验 file:line）: `entitlements.rs:243-246` `match cursor { Ok(c)=>try_collect().unwrap_or_default(), Err(_)=>Vec::new() }`——Err 直接丢无日志/事件；调用点 gateway/campaigns.rs:208/contacts.rs:480 均无法感知是错误还是空表。
- 复现设想: products 查询临时失败（索引重建/连接抖动）时，引用真实 active 产品的报价决策被 blocked_unverified_product_claim 拦下，日志只见"无 verified 背书"看不到根因是 DB 错。
- 验证状态: CONFIRMED（吞错代码亲验；误 block 链路经 gates.rs:665 亲验）
- 修复建议: Err 分支至少 tracing::warn! + 可选 best-effort 事件让"DB 错误导致的空产品"可诊断；或让 gateway 在 products 加载失败时对 priced_from_catalog 采取与"确认无产品"不同处置。属稳健性增强，非红线。
- 状态: Open

### [D-08] `claim_analysis` 缺失 → 产品声明硬门被跳过 = fail-open（**已文档化的既定接受取舍，非新缺陷**）
- 入口频道: userOps（产品声明回复）
- 所属簇: D（相邻 review/gates.rs，entitlements 消费侧）
- 类型: 红线（准入门 fail-open）—— **主控裁定=已知取舍**
- 严重度: Low（**主控亲验校准：subagent 初判 Medium，降级为 Low/已知取舍**——`gates.rs:654-657` 注释逐字写明"2026-05-25 知识库清理删除 chunk.safe_claims/ProductClaimMarkers，R5.3 claim_analysis 缺失 fail-closed 推断不在本次恢复范围；claim_analysis 缺失时按'非产品声明'放行"，是**显式文档化的既定接受取舍**，非本轮新发现的缺陷。memory 亦记载"auto-verify/fail-closed 推断删除是产品意图裁决非 bug"。有两道兜底：reviewer 软闸 + knowledge_router verified-only 语料。**不重开已裁决事项**。）
- 现象/风险: Review Agent 未产出 claim_analysis（LLM 漏填/响应降级）时 claim_requires_product_knowledge 返 false→blocked_unverified_product_claim 硬门被跳过→产品声明无 verified chunk 也无目录背书的回复被放行。
- 根因（亲验 file:line）: `gates.rs:658` 仅当 claim_requires_product_knowledge 为真才进 block 块；`gates.rs:654-657` 注释明确记录这是 2026-05-25 显式接受的取舍（亲验逐字属实）；硬门本体 `gates.rs:665` `if verified_chunks.is_empty() && !priced_from_catalog { block }`。
- 复现设想: 构造 review 缺失 claim_analysis + decision 含产品声明但 used_knowledge_ids 无 verified chunk + quoted id 不在 active 目录→预期放行（本应 block）。
- 验证状态: PLAUSIBLE（fail-open 路径代码层确凿可达；但"是否算 bug"取决于是否接受 2026-05-25 设计取舍——注释 + memory 均指向有意接受）
- 修复建议: 若未来产品决定收紧回 fail-closed：claim_analysis 缺失时保守推断"可能是产品声明"并要求背书（恢复被删的 R5.3 语义）。**这是产品意图裁决项，改前须与用户确认**（会提高误 block 率）。当前保持接受取舍。
- 状态: WontFix（2026-05-25 显式接受的既定取舍，主控亲验确认非新缺陷，留痕待未来产品裁决）
