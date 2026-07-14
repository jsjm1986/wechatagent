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

## 环节汇总

- **总 findings 数：20**（簇A 7 + 簇B 3 + 簇C 5 + 簇D 5）
- **严重度分布：0 High / 5 Medium / 15 Low**
  - Medium（5）：A-01 core_facts 缺证据门 / A-02 confirmed_tags 截断 replace 丢标签(偏 High) / A-03 consolidation 跨集合写非原子重放 / C-01 stagnation 读写不对称 / C-02 初始画像半接线残留销售 schema
  - Low（15）：A-04/A-05/A-06/A-07、B-01(Low-Med)/B-02/B-03、C-03/C-04/C-05、D-01/D-07 + 3 条 WontFix 性质（D-02 by-design 观测 / D-05 既定产品范围 / D-08 已文档化接受取舍）
- **无 High、无红线破坏**：核心红线（bayesian/personality 只写不进决策、影子发送侧零副作用、entitlements fail-closed）逐条亲验全部 HOLDS。
- **元家族归纳（本批主线）**：**证据门/合并保护的层间不对称** —— 记忆/标签子系统里"某一层有 fail-closed 证据门或保留合并、对称的另一层却没有"。A-01（core_facts 无证据门 vs tags/personality 有）与 A-02（confirmed_tags 无保留合并 vs core_facts 有）是同一元家族的一对镜像；C-01（读侧动态 vs 写侧写死）、C-02（reply 路径接线 vs 初始画像半接线）是"新旧/读写路径不对称"的延续（与上轮 53findings 元家族同型）。其余为死代码/文档漂移/TOCTOU/边缘就绪债。
- **后续 P0-P3 修复路线建议**：
  - **P0**：无（0 High）。
  - **P1（Medium，优先）**：A-02（最值得修，有现成未消费的 discardedTags 通道可直接实现"显式弃用才移除"）+ A-01（证据门对齐）宜同族处理（都是记忆层证据/保留不对称）；C-01（stagnation 写侧补维度时间戳）+ C-03（配套订正陈旧注释）同族；C-02（初始画像补维度指引）；A-03（consolidation 原子性/fail-soft 对齐）。
  - **P2/P3（Low）**：A-04(validate 接线或改文档)/A-05(部分唯一索引)/A-06/A-07、B-01(收敛双写单点)/B-02(删死代码)/B-03、C-04(移除过期 allow)/C-05(注释)、D-01(影子只读)/D-07(吞错补 warn)。
  - **WontFix 留痕**：D-02/D-05/D-08（主控亲验确认非缺陷或已裁决接受取舍）。
- **交叉去重**：无跨簇重复 finding。A-02（confirmed_tags 记忆丢失）与 B-01（taxonomy 候选双写）虽都涉标签，但前者是 memory consolidation 的 confirmed_tags replace、后者是 taxonomy_candidates 的 upsert 双计，属不同集合不同写路径，非重复。

---

## 簇A 记忆固化 findings

> 主控亲验结论：历史 lead"memory_summary 无界 append"**已修**（merge_memory_summary_dedup_capped 去重+12行/1200字节双封顶）；memoryCard 走 replace+OCC乐观锁+cap（core≤6/recent≤10/deprecated≤20）基本健壮。本簇真正的结构性缺口是**两个记忆层的证据门/合并保护不对称**（元家族）：core_facts 事实层缺 tags/personality 那样的 fail-closed 证据锚定（A-01），confirmed_tags 反过来缺 core_facts 那样的"未显式弃用即保留"合并保护、在截断窗口上整体 replace 丢历史标签（A-02，最值得修）。

### [A-01] memoryCard 事实层 core_facts 缺证据锚定门，与 tags/personality 的 fail-closed 门不对称
- 入口频道: userOps（每轮 reply prompt 注入）
- 所属簇: A
- 类型: 一致性（置信门缺失·元家族不对称）
- 严重度: Medium（主控亲验裁定：core_facts 被注入每一轮 reply prompt 是权威长期记忆，其建立仅凭 LLM 自评分无对话证据锚定；不构成确定性红线破坏（需 LLM 过度采信一句话），但门的结构性缺失是确定的）
- 现象/风险: 客户一句未经佐证的话（玩笑/试探"我预算 500 万"）若被 Reply Agent 判高 importance，即可无阻碍沉淀为长期 core_fact 污染后续所有回复——历史 lead"不因一句话盲目更新记忆"在事实层仍成立。
- 根因（亲验 file:line）: 候选入池仅凭 LLM 自评——`memory.rs:1897-1913` validated_memory_candidate 只要求 importance>0 && confidence>0（均 LLM 自报）；`memory.rs:1887-1895` decide_candidate_status write_score>=6||max_importance>=8→pending 无证据门；consolidator 转 core_facts 落库路径（memory.rs:1469-1547）**无任何 resolve_evidence 调用**。对比 confirmed_tags（parse_reconfirmed_tags memory.rs:1086-1090 亲验）与 personality（parse_facet memory.rs:1123-1129）都强制 resolve_evidence、证据空即 fail-closed。事实层独缺此门。
- 复现设想: 单条 inbound 高 importance 候选→无 evidenceTurns 校验入 pending→consolidation→core_fact 落库→之后每轮 reply prompt 注入。
- 验证状态: PLAUSIBLE（门缺失与不对称代码确证；"坏事实落库"最终发生依赖 LLM 采信度未跑真实 LLM）
- 修复建议: 给 memory_candidates→core_facts 通道加与 tags/personality 同源的证据锚定（candidate 带 evidenceTurns，consolidation 对当前窗口 resolve_evidence，无锚高分候选降级/不进 core_facts），或对"单轮弱证据产生"的候选设 importance 天花板。
- 状态: Open

### [A-02] confirmed_tags 在截断窗口上整体 replace，证据滚出窗口的持久标签被静默清除
- 入口频道: userOps（consolidation 整理）
- 所属簇: A
- 类型: 逻辑正确性（记忆丢失·元家族不对称）
- 严重度: Medium（偏 High）（主控亲验裁定：推荐配置 60 条/6000 字窗口下，任何历史 >60 条消息的联系人其早期 confirmed_tag 一旦支撑证据滚出窗口就可能在下次 consolidation 被清除——即便无任何对话推翻它；是否真丢取决于 LLM 是否重挂窗口内某条消息，故非 100% 确定，定 Medium）
- 现象/风险: 长期确信标签（100 条消息前确立的"预算充足"）在一次例行整理后凭空消失，或被 LLM 强行锚到近期不相关消息（锚点漂移/污染）。与 core_facts"previous 未 discarded 即保留"保护形成鲜明不对称。
- 根因（亲验 file:line）: replace 语义——`memory.rs:1552-1553` 注释"replace 语义：整体覆盖 confirmed_tags，不与旧值合并"（亲验）+ :1612-1624 `$set confirmed_tags`；fail-closed 丢无窗口内证据的标签——`parse_reconfirmed_tags memory.rs:1086-1090` evidences.is_empty()→None（亲验）；窗口被截断——memory.rs:1288-1292 take_window_by_budget(recent_asc,6000,60)。旧标签虽注入 prompt(memory.rs:1295 current_tags)，但 LLM 要保住必须重列且给窗口内可解析 evidenceTurn，证据在窗外时只能丢弃或伪造近窗锚点。
- 复现设想: contact 有 200 条消息，早期确立 tag"价格敏感"（证据在第 3 条），后续不再提及；下次 consolidation 窗口只含最近 60 条→LLM 无法给有效 evidenceTurn→标签被 replace 掉。
- 验证状态: PLAUSIBLE（replace+fail-closed+截断窗口三者组合代码确证会丢无近窗证据标签；最终丢/漂移取决于 LLM 输出未跑真实 LLM）
- 修复建议: 给 confirmed_tags 加与 core_facts 同款"previous 未被显式 discardedTags 推翻则保留"合并（LLM 已输出 discardedTags 通道，memory.rs prompt:1443/1455 有该字段但**当前未被消费**——可据此实现"仅显式推翻才移除，否则保留旧确信标签"），把 replace 改为"合并+显式弃用"。
- 状态: Open

### [A-03] consolidation 跨集合写非原子：memory_card 写成功后中途失败会重放候选记忆
- 入口频道: userOps（consolidation task）
- 所属簇: A
- 类型: 竞态 / 错误处理 / 一致性
- 严重度: Medium（主控亲验裁定：需在两次写之间发生失败（网络/进程）非常驻发生；但一旦发生候选会被重新并入已推进的卡，且路径本身把这类失败当可 retry 放大重放概率）
- 现象/风险: memory_card 已升 v2（operating_memories OCC 写）但随后写 contacts.confirmed_tags 失败→整函数返 Err→候选未被标 consolidated→task retry→重新加载 v2 卡+同批仍 pending 候选→再次并入卡（v3）。同批候选内容被并入两次。
- 根因（亲验 file:line）: 写顺序与 ? 传播——memory_card OCC 写(memory.rs:1560-1580 成功后 v→next_version 已落库)；随后 confirmed_tags 写用 ?(memory.rs:1621 to_bson? + :1612-1624 .await?)；personality 写 fail-soft(memory.rs:1657-1688 仅 warn)；候选标 consolidated 在最末尾且用 ?(memory.rs:1690-1700 update_many...await?)。memory_card 写与候选标记之间任一 ? 失败→"卡已推进但候选仍 pending"撕裂态，task 走 tasks.rs:254-260 retry。
- 复现设想: mock contacts.update_one 在 memory_card OCC 写成功后抛错→候选仍 pending、memory_card_version 已+1；触发 retry 后候选二次并入。
- 验证状态: PLAUSIBLE（需故障注入；写顺序与 ? 传播已确证）
- 修复建议: 把候选标 consolidated 与 memory_card 写归入同一原子边界（先标候选再写卡，或同一事务/幂等键），或把 memory_card 写之后所有 contacts 写改 fail-soft（warn，与 personality 一致）避免"卡已进但被判失败重放"。
- 状态: Open

### [A-04] `MemoryFact::validate()` 在固化写路径从未被调用（文档-代码漂移，bounds 未强制）
- 入口频道: —
- 所属簇: A
- 类型: 文档-代码漂移 / 就绪债
- 严重度: Low（主控亲验裁定：core_fact 的 confidence/importance 未被决策逻辑消费（grep 确认 memory.rs 内仅 personality 读 confidence，事实层不读）；超长 text 部分被非原子门>80 字间接拦；故未确认决策路径实害，主要是文档失真+潜在脏数据）
- 现象/风险: consolidator 产出的 MemoryFact 若 confidence/importance 越界、text>500 字、evidence>1000 字、deprecation_reason>200 字，均不被校验也不被丢弃，原样落库。
- 根因（亲验 file:line）: 文档断言会校验——models.rs:4151-4153"MemoryFact::validate 提供运行时长度/范围检查；apply_consolidator_deprecations 与 W2 校验链调用前会执行，违规 fact 将被 drop+warning"；实际——`.validate()` 在 **src/agent 零命中**（主控 grep 亲验确认），apply_consolidator_deprecations(memory.rs:631-750)全程无 validate 调用，validate 仅出现在 models.rs 单测。部分兜底 fact_is_non_atomic(memory.rs:499-512)>80 字触发重试但越界 confidence/importance 无检查。
- 复现设想: consolidator 返回 confidence:99,text:<600字> 的 fact→落库后 confidence=99 原样存在。
- 验证状态: CONFIRMED（.validate() 在 src/agent 零调用，文档断言可证伪）
- 修复建议: 在 apply_consolidator_deprecations 或 compact 落库前对每条 MemoryFact 调 validate()，违规按文档承诺 drop+warning；或修正 models.rs 文档去掉不实断言。
- 状态: Open

### [A-05] `schedule_memory_consolidation_task` find-then-insert TOCTOU，可产生重复整理任务
- 入口频道: userOps
- 所属簇: A
- 类型: 竞态 / 就绪债
- 严重度: Low（主控亲验裁定：OCC 保护 memory_card 不被双写污染；重复任务多为空耗一次 LLM 调用后自愈——候选已被赢家标记→输家 no_candidates）
- 现象/风险: 同一 contact 两次 reply run 近乎同时调度整理，二者都 find_one 到无 pending 任务→都 insert→生成两条 memory_consolidation 任务。
- 根因（亲验 file:line）: memory.rs:1920-1966 先查后插无唯一索引兜底（find_one pending 任务→is_some 则 return→否则 insert_one，无 (workspace,account,contact,kind,status) 唯一索引）；db/indexes.rs 无对应 unique index。两任务跑起后 memory_card OCC(memory.rs:1560/1581)保证只一 winner 落库，输家 retry。
- 复现设想: 并发两次 webhook run 对同一 contact 命中 consolidation_needed→两条任务。
- 验证状态: PLAUSIBLE（TOCTOU 窗口与缺唯一索引已确证；实际重复概率取决于并发时序）
- 修复建议: 给 tasks 加 (workspace_id,account_id,contact_wxid,kind) 在 status∈active 上的部分唯一索引，或 insert 用 upsert 幂等。
- 状态: Open

### [A-06] 同轮既新增又弃用的 fact 会整条消失（deprecated 集合查不到原件）
- 入口频道: —
- 所属簇: A
- 类型: 逻辑正确性（边缘）
- 严重度: Low（主控亲验裁定：需 LLM 在同一轮把某 id 同时放进 active coreFacts 和 deprecatedFacts，属畸形输出边缘）
- 现象/风险: 该 fact 既不在 active 也不在 deprecated，凭空丢失（无 deprecated 审计痕迹）。
- 根因（亲验 file:line）: apply_consolidator_deprecations 原件仅在 previous 里查(memory.rs:690-702)，新增于本轮 previous 无此 id→warning deprecated_fact_id_not_found+continue 不写 deprecated；而同 id 命中 active 时又从 active 移除(memory.rs:712-723)→active 移除+deprecated 未写=整条消失。
- 复现设想: consolidator 输出 coreFacts 含 {id:"new1"} 且 deprecatedFacts 也含 {id:"new1"}。
- 验证状态: PLAUSIBLE（读码推断；依赖畸形 LLM 输出）
- 修复建议: original 查不到时回落到 incoming card active 集合取原件，或对该情形记更明确 warning 并保留 active。
- 状态: Open

### [A-07] `extra` 越界键与 `recentEpisodeSummary` 长度无 cap（replace 语义故非累积）
- 入口频道: —
- 所属簇: A
- 类型: 就绪债 / 边缘
- 严重度: Low（主控亲验裁定：consolidator 是 replace 故不累积；未知键被所有消费方忽略；recentEpisodeSummary 每轮由单次 LLM 输出决定非 append）
- 现象/风险: LLM 若产出 dimension 表以外的 extra 数组键，或超长 recentEpisodeSummary，本轮不被截断，膨胀单张卡文档体积。
- 根因（亲验 file:line）: compact_memory_card_with_dimensions(memory.rs:473-478)只 cap 已知键(coreFacts 6/recentFacts 10/deprecatedFacts 6 + dimensions 各自 cap)；recentEpisodeSummary(string 非 array)无长度封顶；未在 dimensions 且非上述三键的 array 不被 cap。DEFAULT 8 维(domain_profile.rs:86-112)覆盖 prompt 已知槽，仅越界键漏网。
- 复现设想: LLM 吐 extra 越界数组键或超长 recentEpisodeSummary。
- 验证状态: PLAUSIBLE（cap 覆盖范围已确证；实际膨胀依赖 LLM 是否吐越界键）
- 修复建议: 对 recentEpisodeSummary 加字符封顶；或对 extra 内所有 array 键统一兜底 cap（未知键给保守默认上限）。
- 状态: Open

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

> 主控亲验结论：**引擎层通用化远比预期健康，无写死销售死字段**——domain_signals/dimension_registry/domain_profile 里的销售字面量全是 default_* seed（有 *_matches_hardcoded_verbatim / *_default_is_byte_identical 字节等价护栏），非残留硬编码；历史"C3 apply_active_profile 仅 3 标量"已被设计化解（models.rs:1970-1995 文档化三类接线约定）。真正缺口只有 2 Medium 且都 fail-soft/自愈：F1 stagnation 写侧半接线 + F2 初始画像残留销售 schema。印证喂入线索"引擎/契约/知识三层已闭环"，未发现前端 labelFor 之外的后端残留硬编码销售假设。

### [C-01] `stagnation_dimension` 配置读写不对称：读侧全动态、写侧写死 customer_stage_updated_at
- 入口频道: userOps（planner 停滞催进）
- 所属簇: C
- 类型: 一致性（读写不对称·元家族典型）
- 严重度: Medium（主控亲验裁定：CONFIRMED 读写不对称确凿；但读侧 fail-soft 回落掩盖缺口、funnel.enabled=false 兜底不炸，故非确定性崩溃而是"配了非 customer_stage 停滞维度的行业催进时机语义错误"）
- 现象/风险: DomainProfile.stagnation_dimension 声称让任意行业指定"哪个维度驱动 planner 停滞计时"。读侧已完全动态化，但写侧从不写非 customer_stage 维度的时间戳→配非 customer_stage 停滞维度的行业计时实际永远跟踪 customer_stage 时间戳。
- 根因（亲验 file:line）: 读侧动态——`planner/mod.rs:1022-1023` DB filter `format!("domain_attributes.{dim}_updated_at")`（亲验）+ 内存判定按同 key。写侧写死——`domain_signals.rs:148-149` `if stage_changed && signals.get_str("customer_stage").is_ok() { set_doc.insert("domain_attributes.customer_stage_updated_at", ...) }`（亲验）；gateway.rs:4043-4048 stage_changed 也只按 customer_stage 新旧比对。读侧 planner/mod.rs:1029-1032 第二支 $or（`<dim>_updated_at $exists:false` AND customer_stage_updated_at<before）回落恰好掩盖写侧缺口。
- 复现设想: profile 设 stagnation_dimension="relationship_closeness"+funnel.enabled=true，期望"关系亲密度 N 天未变→催进"；实际 relationship_closeness_updated_at 永不被写→计时恒回落 customer_stage_updated_at→催进时机语义错误（且若该域根本不写 customer_stage，stage_stagnation_candidate_filter 的 customer_stage:{$exists} 会整体排除这些 contact，仅靠 funnel.enabled=false 兜底）。
- 验证状态: CONFIRMED
- 修复建议: insert_domain_signal_values 增参 stagnation_dimension，stage_changed 时写 `domain_attributes.{stagnation_dimension}_updated_at`（DEFAULT=customer_stage 时字节等价）；或 gateway 写点按 active profile 的 stagnation_dimension 补写对应时间戳。
- 状态: Open

### [C-02] 初始画像生成路径通用化只做了一半 + 残留销售 schema
- 入口频道: userOps（admin 建档时的初始画像 seed）
- 所属簇: C
- 类型: 一致性（半接线·新旧路径不对称）
- 严重度: Medium（主控亲验裁定：CONFIRMED 半接线确凿；但初始画像是一次性 seed，live reply 每轮重新派生维度→首条入站消息即自愈，故非确定性失效；非销售域首屏画像被销售字段主动框住是真实瑕疵）
- 现象/风险: build_initial_operation_profile 注释自称已修复"唯一漏接 active DomainProfile 的 prompt 构造点"，但只接了 prompt_fragment，未接 live reply 路径已有的 render_decision_dimensions_guidance/render_memory_candidate_types_guidance 等；抽取仅读 customerStage/intentLevel。非销售域声明的 profile_dimensions（relationship_closeness/emotion_state）在建档时既不被告知 LLM 也不被采集。
- 根因（亲验 file:line）: `decision.rs:69-77` 初始画像只接 active_profile.prompt_fragment 作 business_context（亲验），无 render_decision_dimensions_guidance append；对照 live reply 路径 decision.rs:679-686 明确 append 了该 guidance。默认 prompt user.initial_profile.task（prompts.rs:1075-1102）schema 写死销售字段 painPoints/budget/decisionRole/customerStage/intentLevel。
- 复现设想: 情感陪伴域 workspace（profile_dimensions 含 emotion_state 而非 customer_stage）建档，首屏画像 prompt 仍问 budget/decisionRole，不采集 emotion_state；首条真实 inbound 后 live reply 才自愈。
- 验证状态: CONFIRMED（半接线亲验；自愈缓解亦亲验）
- 修复建议: 在 build_initial_operation_profile 的 user prompt 组装处比照 reply 路径追加 render_decision_dimensions_guidance + render_memory_candidate_types_guidance；或将 user.initial_profile.task 的销售 schema 段纳入 profile 驱动的注入点。
- 状态: Open

### [C-03] `planner/mod.rs:819-821` 注释陈旧：称 DB dotted-key 动态化"待后续 milestone"实际已实现
- 入口频道: —
- 所属簇: C
- 类型: 文档-代码漂移
- 严重度: Low（主控亲验裁定：CONFIRMED 注释与实现矛盾，易误导并掩盖 F1 真缺口在写侧）
- 现象/风险: planner/mod.rs:819-821 注释称 stagnation_dimension"当前仅承载该值供内存判定；MongoDB 端 filter 的 dotted-key 动态化随后续 milestone 跟进"，但 stage_stagnation_candidate_filter(:1022-1033) 已实现 DB 端动态拼 `<dim>_updated_at`。
- 根因（亲验 file:line）: planner/mod.rs:1022-1033 已动态化（C-01 亲验时确认），注释未更新。
- 复现设想: 无（读码发现）。
- 验证状态: CONFIRMED
- 修复建议: 更新注释为"DB filter 已动态化，真正缺口在写侧只写 customer_stage_updated_at（见 C-01）"。
- 状态: Open

### [C-04] `domain_profile.rs:12` 模块级 `#![allow(dead_code)]` 承诺"Phase 1 后移除"却仍在
- 入口频道: —
- 所属簇: C
- 类型: 就绪债
- 严重度: Low（主控亲验裁定：CONFIRMED；subagent 逐一亲验全部 ~30 个 DomainProfile 字段均有真实消费方无死字段，故当前未掩盖实际死代码，但该 allow 关闭全模块死代码检测、未兑现自身注释承诺）
- 现象/风险: domain_profile.rs:10-12 注释明写"Phase 1 接线后移除本 allow，由编译器确保每个导出项都被真实消费"，但 `#![allow(dead_code)]` 仍在，关闭整模块死代码检测。
- 根因（亲验 file:line）: domain_profile.rs:10-12 注释 + allow 并存。
- 复现设想: 无。
- 验证状态: CONFIRMED
- 修复建议: 移除模块级 allow 恢复编译器保护（subagent 已验全字段有消费方，移除应不触发新 warning）。
- 状态: Open

### [C-05] `DimensionChannel` 四变体仅 AdminDirect 驱动逻辑，其余三个纯描述性
- 入口频道: —
- 所属簇: C
- 类型: 文档-代码漂移（易误导）
- 严重度: Low（主控亲验裁定：CONFIRMED；注释已把 channel 定位为"结构属性"元数据，但维护者可能误以为 channel 决定写入路由）
- 现象/风险: dimension_registry.rs:14-24 定义四通道但逻辑仅 AdminDirect 被 match(:133)，LlmSignals/GatewayDerived/ReactionDerived 从不驱动路由——真实"机器 vs admin"区分由正交的 WriteIntent 承担。
- 根因（亲验 file:line）: dimension_registry.rs:133 `matches!(spec.channel, DimensionChannel::AdminDirect)` 是唯一 channel 判定点。
- 复现设想: 无。
- 验证状态: CONFIRMED
- 修复建议: 注释显式说明"仅 AdminDirect 载逻辑、余者为文档性元数据，写入路由由 WriteIntent 承担"。
- 状态: Open

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
