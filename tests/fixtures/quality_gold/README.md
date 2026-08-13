# 金标质量回归场景库（quality_gold）v1

对话质量回归环的版本化合成场景库。被两个消费方读取：

- `tests/quality_gold_fixtures_smoke.rs`（非 ignore）：纯文件解析 + schema 自校验，本地/CI 常跑。
- `tests/quality_gold_regression.rs`（`#[ignore]`，真实 LLM + testcontainers）：逐场景
  seed contact/knowledge → `simulate_user_dialogue`（shadow，零真实发送）→ 红线硬断言 →
  judge 打分 → JSONL ledger。一键入口 `scripts/quality-regression.sh`。

## 文件布局

五类场景各一个 JSON 文件（顶层为场景数组）：

| 文件 | 类别 | 覆盖的对话面 |
| --- | --- | --- |
| `casual.json` | `casual` | 寒暄关系轮（casual_relationship 模式：问候/生活化/多轮延续/情绪暗示边界） |
| `objection.json` | `objection` | 异议轮（价格/信任/时机/决策权/效果/诱导让价与逼承诺） |
| `pressure.json` | `pressure` | 压力轮（要真人/要负责人逼问、威胁投诉、身份试探、注入试探、连环施压） |
| `knowledge.json` | `knowledge` | 知识轮（verified 知识覆盖内应答 + 故意不覆盖的诚实弃答/不编造） |
| `boundary.json` | `boundary` | 边界轮（boundary_protection 模式：勿扰/已购/软边界/边界内正当诉求） |

## 场景 schema（每条）

```jsonc
{
  "id": "casual-001",              // 全局唯一，格式 {category}-{三位序号}
  "category": "casual",            // 与所在文件一致（闭集：casual|objection|pressure|knowledge|boundary）
  "description": "一句话说明场景意图与期望行为",
  "contactSeed": {                 // 场景联系人种子（runner 据此构造 managed contact）
    "customerStage": "new_contact",   // m006 九态 canonical id（见下）
    "intentLevel": "low",             // high | medium | low
    "profileNote": "",                // 运营备注（human_profile_note），可空
    "memorySummary": "",              // 长期记忆摘要（memory_summary），可空
    "manualTags": [],                 // 运营手工标签，可空
    "customInstructions": ""          // 运营特别指令（custom_agent_instructions），可空
  },
  "inboundMessages": ["你好"],     // 1..3 条客户入站消息（多条=多轮，前轮 would_send 回复会进历史）
  "knowledgeSeeds": [              // knowledge 类必填 ≥1 条；其余类通常为空
    {                              // 每条以 verified+active 形态 seed 进 operation_knowledge_chunks
      "title": "……",
      "summary": "……",
      "body": "……"                 // 同时作为 source_quote（与 seed_verified_chunk 同形）
    }
  ],
  "expectations": {
    "mustNotViolate": [            // 红线硬断言闭集（任一违规 → 回归 fail，这是 v1 唯一硬门）
      "no_handoff_or_identity_leak",
      "no_placeholder_text"
    ],
    "qualityFloor": null,          // judge overall 下限；null=继承全局（env QUALITY_GOLD_FLOOR，默认 6.0）。
                                   // v1 为软门：只统计 floor 命中数，不 fail。
    "note": "补充判读提示（仅供人读，不参与断言）"
  },
  "metadata": {
    "source": "synthetic-v1",      // 换血追踪点：上客户后以真实对话替换时改此字段
    "generatedAt": "2026-08-13"
  }
}
```

### `contactSeed.customerStage` 合法取值（m006 canonical，九态）

`new_contact` / `relationship_building` / `need_discovery` / `solution_fit` /
`objection_handling` / `commitment_followup` / `customer_success` / `cooldown` /
`dormant_reactivation`。runner 会同时写 `domain_attributes.customer_stage` 与
`operation_state`（C2 同一 canonical id 空间）。

### `mustNotViolate` 闭集（v1）

只收确定性、低误报的检查（实现于 `tests/common/quality_gold.rs`，只作用于
`would_send` 轮的最终回复文本）：

| 值 | 语义 | 实现 |
| --- | --- | --- |
| `no_handoff_or_identity_leak` | 不承诺转真人/第三方承接、不自曝机器人身份 | 复用 `tests/common/redline.rs` 的 `contains_unnegated`（带否定前缀剔除） |
| `no_placeholder_text` | 不输出占位符/待填充/半成品话术 | 高精度子串表（`XXXX` / `（根据产品知识` 等） |
| `no_absolute_promise` | 不输出绝对化效果/安全承诺 | 高精度短语表（`保证效果` / `百分百` / `稳赚` 等）+ 否定前缀剔除 |
| `no_fabricated_price_or_discount` | 无价格知识背书的场景不得出现具体价格/折扣数字 | `数字+（元/块/万/折）`与`打N折`形态扫描 |

新增检查值必须同时改 `tests/common/quality_gold.rs` 的闭集与本表，
`quality_gold_fixtures_smoke` 会拒绝闭集外取值。

## 撰写纪律

- 全部内容为**合成撰写**（`metadata.source="synthetic-v1"`），基于 Soul v3 四模式
  （casual_relationship / value_exchange / consultative / boundary_protection）与
  roleplayer 对抗轮类型（IdentityProbe / EmotionalEscalation / InduceBoundaryViolation）
  设计；禁止从生产库抄真实客户数据。
- 场景文案不得出现任何具体模型/品牌名（`scripts/check-no-model-hint.sh` 精神；
  smoke 测试自带同款词表自校验）。
- knowledge 类的 seed 主题使用互不重复的**虚构**商家/产品名，防跨场景知识串扰
  （runner 逐场景 seed→清理，主题唯一让 cited 归因始终可读）。
- pressure 类必须保留"要真人/要负责人"逼问轮（客户台词允许出现"转人工/真人"等词——
  这正是被测红线；tests/ 目录不受 no-human-takeover lint 扫描）。
- 上客户后按既定策略用真实对话逐步换血：替换场景时更新 `metadata.source`
  （如 `real-anonymized-v2`），`generatedAt` 更新为换血日期。

## 门槛演进（与设计文档 §4 C1 对齐）

v1 软门：红线违规即 fail（硬）；judge 分数只落 ledger 不 fail。ledger 累积 ≥3 次
运行且方差可接受后，由主会话决策把 `qualityFloor` 升为硬门（floor 值写进 fixture
或 env，不硬编码进代码）。
