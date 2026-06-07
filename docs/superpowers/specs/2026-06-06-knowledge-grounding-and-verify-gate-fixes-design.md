# 知识库 grounding 漏判兜底 + 人工后门 D2 收口 设计

> 状态：设计待批准（2026-06-06）。
> 定位：两处**独立**的知识库红线收口，合并到一份 spec 一并实现。A=发送侧 grounding 漏判（review.rs），B=写入侧人工后门绕过 D2 门（routes/knowledge.rs）。两者无耦合，可分别 TDD、分别提交。

## 0. 来源：代码审计结论（以代码为准）

本设计基于一次全链路逐行亲验（生产代码，非测试、非文档），结论：

- **「AI 永不自动 verify」红线在代码层全链路成立**：写入侧 `apply_chunk_revision`（`src/knowledge_wiki/chunk_revisions.rs:207-210`）对 `source==Ai` 强制 `draft+needs_review`；import 三分支（`src/routes/knowledge.rs:1501-1502 / 1548-1551 / 2038-2044`）调 `apply_chunk_integrity` 后无条件回压 needs_review；chat 落库（`5861-5862 / 5954-5955`）强制 needs_review；AI 自主修复只产 patch 不写库（`3499-3505`）；后台 worker 无一写 `integrity_status`。消费侧 `load_operation_knowledge`（`src/agent/knowledge_router.rs:71`）DB 层硬过滤 `integrity_status="verified"`，draft 根本查不出。
- **两处张力都在人工 / reviewer 环节，不在 AI 自动化环节**，即下面的 A 和 B。

## A — grounding 漏判：观测探针抬为「词类型切分」硬闸 + prompt 根因

### A.1 现状（亲验）

- **R5.4 硬闸**（`src/agent/review.rs:932-961`）：仅当 reviewer 的 `claim_analysis.requiresProductKnowledge==true` 且 `compute_verified_chunks`（`src/agent/guards.rs:286`）交集为空时，强制 `blocked_unverified_product_claim`。**这条不看承诺词，只看 verified 交集，是干净的结构化闸。**
- **观测探针**（`src/agent/review.rs:975-998`）：reviewer **漏判**（未自报 requiresProductKnowledge）∧ 回复含承诺词（`guards::reply_contains_commitment_claim`，`src/agent/guards.rs:324-333`，8 个裸词）∧ 无 verified 背书 → 只落 `grounding_probe_reviewer_missed`（status=observe）telemetry，**不拦截**。
- 这是 commit `5a78e94`「内容质量第一轮迭代」刻意引入的「先观测后判罚」，commit message 明言「有证据再决定是否抬硬闸（避免重新引入 2026-05-25 刻意删除的脆弱 string-marker 判罚）」。

### A.2 问题

reviewer 漏判时，含绝对化产品承诺、又无 verified 背书的回复会直接放行——质量红线上的真实缺口。但直接把 8 个承诺词全部抬成硬闸 = 恢复 2026-05-25 删掉的脆弱 string-marker 判罚，会误杀情感承诺（「我**保证**认真对待您的问题」「这事**绝对**不怪你」）。

### A.3 方案：双层（prompt 根因为主，词类型切分硬闸为兜底）

**第一层 — prompt 根因（治本，降低漏判率本身）**

强化 `user.review.system`（`src/prompts.rs:1259-1300`）中 `requiresProductKnowledge` 的判定指引，让 reviewer 更可靠地自报。一旦自报准确，走的是 R5.4 既有结构化硬闸（基于 verified 交集，**完全不碰承诺词**），这是消除漏判的正路。

- 现有 prompt 在 `:578-599` 一带已有 `requiresProductKnowledge` 的说明（「只是承接顾虑/表达理解/轻量澄清 → false」）。本层在 reviewer system 增补「**反向**」锚点：候选回复含可验证的产品效果/数据/案例/价格断言（成功率、见效时间、回款、百分比、具体价格、客户案例）时，无论语气软硬，`requiresProductKnowledge=true`。
- 纯文案改动，seed「存在即跳过」，老库不变、CI 全新库自动生效（与 `5a78e94` 第③件套同机制）。**无法在纯函数单测断言**，靠 real-LLM 套件（`tests/real_llm_ops_smoke.rs` 的 grounding 探针读取）观测漏判率变化。

**第二层 — 词类型切分硬闸（兜底，治标，确定性可单测）**

把 `guards::reply_contains_commitment_claim` 的 8 词拆成两类，新增一个分类函数（不删旧函数，旧函数继续供观测探针用）：

| 类别 | 词 | 处置 |
|---|---|---|
| **效果/数据类**（product_effect） | `成功率` `见效` `回款` `百分之` `百分百` | 漏判 ∧ 命中 ∧ 无 verified → **硬闸拦截** |
| **语气类**（tone_only） | `保证` `一定能` `绝对` | 仅观测，不拦（最易误杀情感承诺） |

- 切分依据：效果/数据类词几乎只出现在「可验证的产品断言」语境（成功率、回款是业务效果数据），情感承诺极少用；语气类词大量出现在情感/口语承诺，误杀风险高。
- 新增纯函数 `src/agent/guards.rs`：`fn commitment_claim_class(reply_text: &str) -> CommitmentClass`，返回 `ProductEffect | ToneOnly | None`（`ProductEffect` 优先：同时含两类时按更危险的 ProductEffect 处置）。
- `reply_contains_commitment_claim` 保持不变（观测探针仍用它，覆盖全 8 词，telemetry 不缩窄）。

### A.4 review.rs 改动（`src/agent/review.rs:963-999`）

把现有「先观测」分支改为「先判硬闸、再观测」：

```text
若 reviewer 漏判（!claim_requires_product_knowledge）:
  verified = compute_verified_chunks(used_knowledge_ids, knowledge_chunks)
  若 verified.is_empty():
    class = commitment_claim_class(reply_text)
    若 class == ProductEffect:
      # 硬闸兜底：与 R5.4 同样的 block 形态
      review.approved = false
      review.scores.hallucination_score = max(6)
      decision.should_reply = false
      decision.autonomy_mode = "blocked"
      review.final_review_status = "blocked_unverified_product_claim"   # 复用既有闭集枚举
      push event kind="product_claim_blocked_by_probe_fallback" status="blocked"
      return BlockedUnverifiedProductClaim
    若 class == ToneOnly:
      # 维持现状：仅观测
      push event kind="grounding_probe_reviewer_missed" status="observe"
```

要点：
- **复用既有 `final_review_status="blocked_unverified_product_claim"` 与 `GatewayStatusFinal::BlockedUnverifiedProductClaim`**，不新增闭集枚举（R9.10.e 闭集校验零改动）。
- event kind 用新值 `product_claim_blocked_by_probe_fallback` 以便和 reviewer 自报触发的 `product_claim_blocked` 在 telemetry 上区分（一个是 reviewer 正报、一个是兜底网捞回），但 status 都是 `blocked`（自由字符串字段，不触发闭集校验）。
- ToneOnly 与「无承诺词」路径都不拦，行为同今天。

### A.5 A 的测试（`src/agent/review.rs` tests + `guards.rs` tests，纯函数 / 本地可跑）

新增（append，不改三个既有探针单测的语义；其中 `finalize_emits_grounding_probe_on_reviewer_missed_commitment` 用例 reply_text=「这个方案**一定能**帮您解决问题」属 ToneOnly，断言不变——仍只观测，正好守住「语气类不误杀」）：

1. `commitment_claim_class_*`（guards.rs 纯函数）：`成功率95%`→ProductEffect；`我保证认真对待`→ToneOnly；`一定能 + 成功率`→ProductEffect（更危险优先）；`好的我先了解下`→None。
2. `finalize_blocks_on_product_effect_claim_when_reviewer_missed`：reviewer 漏判 + reply 含 `回款` + 无 verified → `BlockedUnverifiedProductClaim` + event `product_claim_blocked_by_probe_fallback`。
3. `finalize_only_observes_on_tone_only_claim_when_reviewer_missed`：reviewer 漏判 + reply 含 `保证`（且不含效果词）+ 无 verified → Approved + observe 事件（不拦）。
4. `finalize_probe_fallback_skipped_when_verified_present`：reviewer 漏判 + reply 含 `成功率` + **有** verified 交集 → Approved（兜底不误伤有背书的）。

## B — 人工后门 create/PUT 不过 D2 门：收口为同一道闸

### B.1 现状（亲验）

- `create_operation_knowledge_chunk`（`src/routes/knowledge.rs:443-459`）：`validate_operation_knowledge_chunk`（`:2371-2378`）只查 title 非空；builder `operation_knowledge_chunk_from_request`（`:2482`）原样写入调用方自带的 `integrity_status`。前端误传 `integrityStatus:"verified"` → 直接落 verified，零 gate。
- `update_operation_knowledge_chunk`（`:462-507`）：调 `apply_chunk_integrity`（`:491`）——锚点 fuzzy 命中即置 `verified`（`:2912`）——之后**不回压 needs_review**，与 import 三分支不对称。
- D2 门 `chunk_verify_gate_reason`（`:3455-3470`，要求 sourceQuote+source_anchors 双非空）**只在 `/verify`、`/batch-verify` 生效**。

### B.2 方案 B1：create/PUT 落库前，verified 必须过 D2 门，否则降级 needs_review

新增一个 super 内部纯函数（落 `src/routes/knowledge.rs`），在两个 handler 的落库**之前**调用：

```text
fn coerce_integrity_against_d2_gate(payload: &mut OperationKnowledgeChunkRequest):
    若 payload.integrity_status.as_deref() == Some("verified"):
        has_quote = payload.source_quote 非空白
        has_anchor = payload.source_anchors 非空
        若 chunk_verify_gate_reason(has_quote, has_anchor).is_some():   # 复用既有 D2 纯函数
            payload.integrity_status = Some("needs_review")            # 降级，不 400
            payload.distortion_risks.push("提交为 verified 但缺 sourceQuote/source_anchors，未过 D2 闸，已降级 needs_review")  # 必留审计痕迹，与 import 路径 :2922-2929 一致
```

接入点：
- `create_operation_knowledge_chunk`：`validate_*` 之后、builder 之前插一行 `coerce_integrity_against_d2_gate(&mut payload)`。
- `update_operation_knowledge_chunk`：在现有 `apply_chunk_integrity(&mut payload, ...)`（`:491`）**之后**插同一行——这样 fuzzy-anchor 置的 verified 也要再过 D2 门（quote+anchor 双非空）才保留。

### B.3 为什么是「降级」不是「400 拒绝」

- 降级 needs_review 与 import/chat/AI 全部写入口的既有行为一致（它们也是「锚点是审核线索、最终落 needs_review」），语义统一。
- 400 会打断前端可能存在的「直接建 verified」工作流，且需要先排查前端用法；降级是非破坏性、零工作流中断的收口。
- 真正要 verified 的正路仍在：走 `/verify`（过 D2 + 写 verified_at/verified_by + 留 verify revision）。

### B.4 不动的部分（避免过度修复）

- **不改 `apply_chunk_integrity` 本身**：它在 import 路径里被「调用后回压」消费，逻辑正确，动它会波及 import。只在 create/PUT 的**调用点之后**加 D2 收口。
- **不给 chunk status/integrity_status 加集中 ALLOWED_* DB 断言**：那是更大的重构，超出本次范围（YAGNI）。本次只收口「verified 必须过 D2」这一条不变量。

### B.5 B 的测试（纯函数 / 本地可跑）

新增纯函数单测（不需 Mongo）：
1. `coerce_*_downgrades_verified_without_quote`：payload verified + 空 quote → needs_review。
2. `coerce_*_downgrades_verified_without_anchor`：payload verified + 空 anchors → needs_review。
3. `coerce_*_keeps_verified_with_quote_and_anchor`：payload verified + quote + anchor → 保持 verified。
4. `coerce_*_ignores_non_verified`：payload needs_review/draft → 原样不动。

（handler 级集成测试涉及 Mongo，按项目「本地只跑 lib + PBT，集成留 CI」纪律，集成断言进 `#[ignore]`，本地不强求跑。）

## 红线护栏（全程）

- 不削弱 D2：B 是**加强** D2 覆盖面（人工后门也纳入），不是放水。
- 不恢复脆弱全词 string-marker：A 的硬闸只收窄到效果/数据类 5 词，语气类 3 词仍仅观测；治本靠 prompt 根因。
- 复用既有闭集枚举（`blocked_unverified_product_claim` / `BlockedUnverifiedProductClaim`），R9.10.e 零改动。
- `scripts/check-no-human-takeover.{sh,ps1}` 禁词全程守住：新增 event kind / 注释 / prompt 文案不得含「人工接管/takeover/hand-off/人工」等。本设计 A/B 文案均用 AI-internal 措辞。
- baseline 门（lib ≥ 350/0 + 4 PBT ≥ 33/0）每提交后零回归；A/B 各自只 append 测试，不删改旧维度。
- 反过拟合：A 的词切分是**可复现的抽象规则**（按词义分类，非对单条 CI 样本点对点修补）；prompt 锚点是通用判定指引，非贴合单样本。

## 实施顺序（两条独立链，可分别提交）

- **B 先行**（更小、零 LLM、纯一致性收口）：B1 函数 + 4 纯函数单测 + 两 handler 接入 → `cargo test --lib` 绿 → 提交。
- **A 随后**（两层）：
  1. `guards.rs` 新增 `commitment_claim_class` + 单测；
  2. `review.rs` 探针分支改造 + 4 单测（含守住 ToneOnly 不误杀的回归）；
  3. `prompts.rs` reviewer system 增补 requiresProductKnowledge 反向锚点（纯文案）；
  4. `cargo test --lib` 绿 → 提交。real-LLM 漏判率变化留 `tests/real_llm_ops_smoke.rs` / CI 观测。

## 范围外（Out of scope）

- chunk status/integrity_status 的集中 DB 闭集断言（更大重构）。
- 把语气类 3 词也抬硬闸（误杀风险高，留给 prompt 根因 + 探针数据驱动后再议）。
- kb-probe spec（`2026-06-06-kb-business-probe-design.md`）所属的端到端仿真探针——本设计是点状收口，与之并行不冲突。
