# 业务效果审查报告（对客体感 / AI 决策效果面）

> 审查基线：`0e15181f4ced064d21d97f11bfe1af77fbe01aff`（main，2026-08-05）
> 审查日期：2026-08-05
> 交叉复核：2026-08-05
> 审查重点：**业务效果**——AI 决策链路的实际产出质量、知识召回是否可靠、
> 主动触达是否合理、客户侧真实体感。**不重复**安全面、幂等、状态机、租户边界
> （见 `CODE_REVIEW_FINDINGS.md`）与授权有效期 / 审核版本错配 / Admin 输入校验
> （见 `SWARM_BUSINESS_LOGIC_REVIEW.md`）。
> 方法：源码逐段阅读 + `rg` 双向引用追踪 + 生产调用链反证 + 针对性单元/集成测试。
> 未完成真实 LLM、MCP、微信链路验证的部分在文末「未覆盖」单列，不据此扩张结论。

## 结论标记

| 标记 | 含义 |
| --- | --- |
| ✅ **确认缺陷** | 源码与完整调用链已交叉确认，行为偏离自身契约或产生错误业务结果 |
| 🟡 **确认风险** | 代码事实成立，但危害受补偿控制约束，或需特定配置/规模才触发 |
| 🧭 **设计选择** | 行为是刻意取舍，需产品决策而非直接按缺陷修复 |

严重度：**S1** 红线 / **S2** 功能正确性与业务有效性 / **S3** 精度、可维护性、较窄风险。

## 结论摘要

本轮确认 **5 条需处理**：2 个 S2 确认缺陷（B1、B2）、1 个 S2 确认风险（B3）、
2 个 S3 确认风险（B4、B5）；另有 1 条待产品确认的设计选择（B6）。

最高优先级是 `B2`：知识 Agent 已明确弃答或 citation 校验失败后，路由仍会从整个
verified corpus 无相关度下限地回填 top-5。进入 Full 档或 Full 改写后，这些回填 ID
会成为 `used_knowledge_ids`；确定性产品背书闸只检查它们是否 verified/未过期，
不检查与 query 或回复 claim 的相关性。因此无关知识可能满足原本用于防止无依据产品声明的硬闸。

`B1` 同样应优先处理：默认配额为 3 时，一次成功发送 3 段的普通被动回复，或较少分段叠加
此前出站记录，就会占满滚动 24 小时计数，使后续主动 FollowUp 被 `daily_limit` 拦截。

`B3` 与 `B5` 会共同造成「知识存在但引用失败，运营却被引导去补知识」；`B4` 是缺少
按内容长度和会话语境调节发送节奏的业务体验风险。`B6` 是明确设计选择，需结合目标客群活跃时段决定。

负面结论经复核后也已收窄：N1 仅证明所检查的三条 Reviewer 失败路径 fail-closed；
多段发送会诚实暴露 `partially_sent`，但并不保证客户永远收不到半截；Planner 各段有
segment cap，但同时共享一个总 daily cap。

---

## B1 · 被动分段回复消耗主动触达配额，后续跟进被拦 ✅ 确认缺陷（S2）

**位置**：`src/agent/gateway.rs:4136-4152,4294-4300,4623-4647`、
`src/models.rs:4729-4731`、`src/config.rs:459-464`

**已确认事实**：

1. `max_daily_touches` 默认是 **3**。
2. `daily_limit_applies_to` 只让 `FollowUp` 受闸，注释明确声明 Inbound 被动回复不应受主动触达上限限制。
3. 但 `daily_touch_count` 统计该客户最近 **滚动 24 小时**内所有
   `conversation_messages.direction="outbound"` 的文档数，不区分主动/被动来源。
4. 每个成功送达的文本分段都会独立写一条 outbound conversation message。
5. 默认每次回复最多拆成 4 段。

**触发时序**：客户发起普通咨询 → AI 被动回复成功发送 3 段 → outbound 计数达到默认阈值 3 →
后续承诺跟进、续费提醒、纪念日关怀、停滞催进等 FollowUp 在 precheck 被
`daily_limit` 拦截。2 段本身不会立即耗尽默认配额，但叠加此前任何 outbound 记录也可触发。

**为何是缺陷**：闸门入口豁免了 Inbound，却在计数侧重新把 Inbound 的每个发送分段算入配额，
实现与自身契约相反。分段越多，主动运营能力越容易被被动服务消息挤掉。

**影响边界**：这是滚动 24 小时，不是自然日；只有后续 FollowUp 受该闸影响，Inbound 和
领导裁决 relay 不会被它阻断。

**建议修复**：建立明确的“主动业务触达”计数事实源。优先按 Outbox/Run 的 FollowUp 来源统计
一次逻辑触达，而不是按 conversation message 分段数统计。仅按 `run_id` 去重仍会让被动回复消耗
主动配额，不能完整恢复契约；当前 conversation message 也没有可直接依赖的顶层 `source_kind`，
如继续从该集合统计，需要先持久化可信来源字段。

**建议测试**：

- 一次 3/4 段 Inbound 被动回复后，首个主动 FollowUp 仍放行；
- 一次主动 FollowUp 即使拆成多段，也只消耗一次逻辑触达；
- 多次主动 FollowUp 达到上限后正确拦截。

---

## B2 · 无关 fallback 知识可在 Full 档满足产品背书硬闸 ✅ 确认缺陷（S2，最高优先）

**位置**：`src/agent/knowledge_router.rs:568-668,813-819`、
`src/agent/knowledge_agent.rs:1054-1080`、`src/agent/gateway.rs:1800-1834,1907-1960,2146-2153,2285,2571-2572`、
`src/agent/guards.rs:338-400`、`src/agent/review/gates.rs:722-776`

**已确认事实**：

1. Knowledge Agent 未能产出可验证 answer/citation 时，刻意返回空 `cited_chunk_ids`，不制造证据 ID。
2. Router 发现 citation 为空后，会对已加载的 verified corpus 排序并无条件取 top-5；
   `fallback_ids` 只在 corpus 本身为空时为空。
3. `rank_key` 虽包含相关度，但 fallback 没有最低相关度门槛。即使所有候选与 query 的
   `effective_relevance_micros` 都为 0，仍会选出 top-5，并把 coverage 从 `missing` 改成 `weak`。
4. 非 Full 档会清空 `used_knowledge_ids`，这是有效补偿控制。
5. 但只要第一程强升/自评升到 Full，或后续进入 Full rewrite，route 中的 fallback ID 会写进
   `used_knowledge_ids`。
6. 产品硬闸的 `compute_verified_chunks` 只求 `used IDs ∩ verified、未过期 chunks`，
   不验证 query 相关性、claim 对应关系，也不要求这些 ID 通过 citation + source quote 校验。

**业务后果**：

- corpus 非空时，真正的“零相关知识”通常被标成 `weak`，`should_force_full_on_missing` 无法因
  `missing` 触发强升，运营也看不到准确的缺失状态；
- 一旦流程因其他原因进入 Full，完全无关但 verified 的 fallback chunk 可成为
  `used_knowledge_ids`，从结构上满足产品背书硬闸；
- 独立 Reviewer 仍可能从语义上识别并拦截，这是补偿控制，但原本确定性的结构化背书闸已被架空。

**为何是缺陷**：fallback 的设计目标是弱导航/降级候选，却被复用成产品事实授权证据；
“已验证”只证明 chunk 自身经过审核，不证明它与当前 query 或候选回复中的产品 claim 有关。

**建议修复**：

1. 将“导航候选”与“可授权证据”拆成不同字段；fallback ID 不得直接进入 `used_knowledge_ids`。
2. 只有通过 `filter_answer_against_opened_chunks` 的 citation + quote + anchor 校验，且与当前 claim
   建立绑定的 chunk，才能满足产品背书硬闸。
3. fallback 增加相关度下限：零相关维持 `missing`，弱相关才标 `weak`。
4. Full rewrite 不得无条件重置为全部 route IDs，应保留或重新计算已验证 citation。

**建议测试**：构造一个与 query 完全无关的 verified corpus，强制进入 Full，断言 fallback IDs
不能使产品声明通过 `blocked_unverified_product_claim`；再用真实相关且 citation/anchor 有效的知识作正例。

---

## B3 · 畸形 anchor 可通过 verify，并使 verified 知识持续无法引用 🟡 确认风险（S2）

**位置**：`src/routes/knowledge/mod.rs:669-717,963-1000,1114-1138,1228-1245`、
`src/routes/knowledge/verify.rs:66-123`、`src/agent/knowledge_agent.rs:1592-1682`

**已确认事实**：

- 服务端正常构造的 anchor 恒含 `sourceQuote`，因此“所有生产 anchor 都缺该字段”不成立。
- 但请求体携带的 `source_anchors` 会原样进入 chunk；只要数组非空，`apply_chunk_integrity`
  就不会重算。
- Verify 闸只检查 `source_quote` 非空和 anchor 数组非空，不验证 anchor 元素结构或与父文档的一致性。
- 读取侧要求模型选择的 anchor 下标存在，且该 anchor 有可读取的 `sourceQuote`；否则 quote 被拒，
  对应 chunk 最终不能进入 cited 集。
- 现有结构测试把仅含 `startOffset` 的非空 anchor 视为“anchor 存在”，未覆盖引用侧契约。

**影响边界**：当模型选中的 anchor 畸形，或该 chunk 的全部 anchor 都不满足读取契约时，
该知识在重新锚定/修复前无法形成有效 citation。不是不可恢复的“永久失效”；存在其他合法 anchor 时，
模型选择合法下标仍可能成功。

**CJK 匹配风险**：`normalize_evidence_text` 只折叠空白。中文引用改动标点、虚词或字符后可能无法
通过严格子串校验；该风险成立，但真实误拒率需要实际 LLM 样本衡量。

**建议修复**：Verify 事务内验证 anchor schema 和父文档一致性，至少包括非空 `sourceQuote`、
合法 offset、quote hash、document identity，并为历史 `quote`/`sourceQuote` 形态提供迁移或兼容。
可复用写入侧 `fuzzy_locate_quote` 的归一化能力，但不能把模糊匹配放宽到无法证明原文出处。

---

## B4 · 发送节奏不随文本长度和会话语境变化 🟡 确认风险（S3）

**位置**：`src/agent/outbox.rs:277-310`、`src/agent/pacing.rs:1-19`、
`src/agent/outbox_dispatcher.rs:2748-2764,3004-3052`、`src/config.rs:457-466`

**确认成立的范围**：系统没有独立的阅读、思考、打字时长模型；账号发送间隔只在默认 1–4 秒范围
随机，且不读取文本长度。同一账号跨客户和同一回复的不同分段都受该账号级节奏闸控制。

**需要避免的过度结论**：

- 不是“全链路零延迟”：默认还有 4 秒入站去抖、Knowledge/Reply/Reviewer LLM、可能的 Rewrite、
  Outbox worker 轮询及账号节奏闸。
- 源码不能证明客户固定在 4–5 秒收到完整回复，正常链路耗时会受模型与 worker 调度影响。
- “会被平台风控识别”需要真实平台数据，不能仅凭静态代码确证。

**业务风险**：短句和长段落使用同一发送间隔分布，可能削弱分段消息的自然感；账号级全局节奏还会让
其他客户流量影响当前会话节奏。

**建议**：先采集真实端到端送达分布，再决定是否在 Outbox 为文本段写入基于字符数、段序和会话上下文的
计划发送时间。必须设置最大延迟，并确保紧急回应、领导裁决和客户等待场景不会被拟人延迟反向伤害。

---

## B5 · `recall_miss` 混合知识缺失与引用格式失败 🟡 确认风险（S3）

**位置**：`src/agent/knowledge_agent.rs:285-350,1837-1924`、
`src/knowledge_wiki/gap_signals.rs:400-477,650-752,1385-1571`

**已确认事实**：`classify_recall_outcome` 对所有非取消且 `cited_count == 0` 的结果统一生成
`recall_miss`。该状态至少可能表示：

- corpus 里真的没有相关知识；
- 找到了 chunk，但 citation/anchor/quote 校验失败；
- Agent 探索未收敛或主动诚实弃答。

这些原因使用同一 kind、severity 和修复描述。`recall_miss` 也没有 `sweep_stale_signals` 的自动消解分支；
`persist_recall_signal` 明确采用只增不消解语义。

**影响边界**：普通 Knowledge Agent 的 recall miss 标题固定，dedup key 为 `kind + normalized title`，
同 workspace 的多次 miss 通常合并到同一 pending 行并累积 query，不一定形成大量独立记录。
产品声明拦截路径的标题包含 query 摘要，更可能形成多条信号。核心问题是原因混淆和长期 pending，
而不是所有场景都产生大量独立行。

**建议修复**：在过滤 citation 时保留结构化拒绝原因，至少区分 `knowledge_missing`、
`citation_format_rejected`、`exploration_exhausted`。只有第一类指导补录知识；格式类应引导修复 anchor，
并在对应 chunk 重新锚定后具备自动消解条件。

---

## B6 · 静默时段客户消息零回应，默认最长约 10 小时 15 分钟 🧭 设计选择（待产品确认）

**位置**：`src/webhooks.rs:275-310,920-985,1036-1087`、`src/agent/quiet_hours.rs`、
`src/agent/runtime.rs:958-960`、`src/agent/gateway.rs:4549-4576`

默认静默窗口是 22:00–08:00，共 10 小时。客户恰在 22:00 发消息时，wake task 会排到 08:00，
再叠加最多 15 分钟 per-contact jitter，以及 worker 调度和决策生成耗时。静默期间不进回复流水线，
`quiet_hours_deferred` 也明确排除在客户占位回应保障之外。

该机制是有意设计而非实现错误：支持 contact/profile/per-relationship 级关闭，wake task 有确定性 jitter，
主动 FollowUp 命中静默期会重排而非丢弃，过期检查也会防止过时跟进次日发送。

**待产品确认**：若目标客群夜间咨询频繁，默认最长约 10 小时 15 分钟零回应可能不符合业务目标；
可考虑调整默认窗口、按关系类型关闭，或发送不承诺业务事实的极简占位。若产品目标就是模拟真人休息，
现状可保留。

---

## 负面结论与边界留档

### N1 · 所核查的三条 Reviewer 失败路径 fail-closed ✅

本轮核查的三条路径均未发现 fail-open：

- Reviewer 输出畸形时走 schema failure hold，给最坏风险分并阻断；
- Reviewer 未执行、预算耗尽时走本地保守 review，不直接放行；
- Rewrite 失败只允许纯风格问题回退原稿，硬闸、压迫感、隐私边界和 reviewer 分歧不能回退。

该结论仅覆盖上述三条路径，不泛化为整个复核系统所有异常分支均已穷举验证。

### N2 · 分段构造和状态观测正确，但允许部分送达 ⚠️ 非“绝不发半截”

已确认的正向机制：分段函数不会因段数上限截断正文；相同内容的不同段使用带段序的幂等 key；
Outbox 按 `created_at, _id` FIFO 领取；入队循环遇到单段失败会继续尝试后续段并记录失败索引。

但系统不具备多段发送的原子交付：已经成功入队或送达的段不会因其他段失败而回滚。生产事件明确记录
“已入队段照常发出，失败段缺失”，Outbox 聚合状态也支持 `partially_sent`。正确结论是系统能诚实观测
部分送达并尽量减少缺失，而不是保证客户永远收不到半截回复。

### N3 · 六个 Planner 扫描段均接线；segment cap 与共享总 cap 同时生效 ✅

`scan_silent`、`scan_commitments`、`scan_stage_stagnation`、`scan_calendar`、`scan_renewal`、
`scan_reactivation` 均已接入生产 Planner loop，并按段隔离错误。

配额并非彼此完全独立：六段使用同一 `strategic_planner` namespace 和 account-scoped 总 daily cap，
`EMIT_EVENT_KINDS` 也跨段汇总。Calendar、Renewal、Reactivation 等可以另有 segment cap，但仍同时受
共享总 cap 限制。Cold Contact 等使用其他 namespace 的流程才拥有独立配额桶。

Campaign 群发按联系人生成确定性 task 并走完整网关；本轮未发现直接批量绕过发送门的路径。

---

## 未覆盖（不含断言，不得据本文推断）

1. `cited_in_corpus` 双窗口：检索 catalog 候选上限与已加载 corpus 上限不同；corpus 超过 200 条时，
   合法 citation 是否可能因成员校验窗口较小而被过滤，尚未完成数据规模与排序条件验证。
2. 未运行真实 LLM、MCP、微信链路；B3 的中文引用误拒率、B4 的端到端节奏体感、B6 的客户流失影响
   都需要生产或仿真样本，静态代码不能给出概率结论。
3. 本轮不含安全面、幂等、竞态、多租户隔离；相关结论见其他审查文档。
4. 未把“缺测试”单独立项，仅在具体问题中记录应补的不变量。

## 针对性验证

交叉复核实际执行（下方通过数已由第二方独立重跑逐条核对，六组数字精确吻合）：

| 测试 | 通过数 | 佐证的断言 |
| --- | --- | --- |
| `knowledge_router_fallback_e2e` | 3 | 非空 corpus 下零 citation 会回填 top-N 并标 weak（B2 事实 2/3） |
| `agent::sufficiency` | 21 | Full 档保留 route IDs、非 Full 档清空（B2 事实 4/5） |
| `outbox_integration::mixed_run_status_is_order_independent` | 1 | 混合终态稳定聚合为 `partially_sent`（N2） |
| `sr135_proactive_outreach::segment_cap_and_shared_total_cap_are_both_persistent` | 1 | 段 cap 与共享总 cap 同时生效（N3） |
| 回复分段纯函数（`split_reply_*`） | 6 | 分段构造正确性（N2 正向机制） |
| `daily_limit_applies_only_to_follow_up` | 1 | daily cap 只约束 FollowUp（B1 前提） |

**复现命令**（前三项是 `#[ignore]` 且需 Docker/testcontainers，缺少 `DOCKER_AVAILABLE=1 -- --ignored`
会静默 filtered out、看不到任何失败，故必须照写）：

```bash
# 需 Docker 的集成测试（各自 --ignored）
DOCKER_AVAILABLE=1 cargo test --test knowledge_router_fallback_e2e -- --ignored
DOCKER_AVAILABLE=1 cargo test --test outbox_integration mixed_run_status_is_order_independent -- --ignored
DOCKER_AVAILABLE=1 cargo test --test sr135_proactive_outreach segment_cap_and_shared_total_cap_are_both_persistent -- --ignored

# lib 内纯函数 / 谓词测试（无需 Docker）
cargo test --lib agent::sufficiency
cargo test --lib split_reply_
cargo test --lib daily_limit_applies_only_to_follow_up
```

这些测试证明当前机制与本文描述一致，**不代表缺陷已修复**——它们锁定的正是本文认定为问题的现行行为。
其中 `router_returns_missing_when_corpus_completely_empty` 尤其关键：它用测试锁定了「**只有** corpus
完全为空才返回 `missing`」，即 B2 事实 3（相关度不参与 `missing`/`weak` 门槛）不是推测，而是仓库自身
测试正在断言的语义。修 B2 时这条测试的预期需同步调整。

## 建议处理顺序

1. **B2**：拆分导航候选和授权证据，阻止无关 fallback ID 满足产品硬闸，并恢复 missing/weak 真实性。
2. **B1**：改为统计主动逻辑触达，不让 Inbound 分段占用 FollowUp 配额。
3. **B3**：统一写入/Verify/读取三侧 anchor 契约，并迁移历史畸形数据。
4. **B5**：按 citation 拒绝原因分裂 gap signal，恢复运营队列信噪比和自动消解能力。
5. **B4**：先测量真实时延，再由产品确定长度相关节奏参数。
6. **B6**：结合目标客群夜间活跃度做产品决策。

## 维护约定

- 新发现先标 `⚠️ 待复核`，补齐源码位置、完整调用链、补偿控制、反证测试和影响边界后再升级。
- 每条必须记录已排除的更强指控，避免把“存在风险”扩大成未经证实的必然结果。
- 修复后不删除条目，改标 `🧹 已修复`，记录提交和验证命令，保留原因与边界。
