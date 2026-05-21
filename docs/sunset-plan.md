# Sunset Plan：自治协议灰度开关下线时间表

> 关联规范：`.kiro/specs/agent-autonomy-loop/requirements.md` R11.5、R11.9。
>
> 目的：W1–W6 升级期为了灰度兼容引入了若干"双轨开关"与"老格式兜底"，
> 它们在升级稳定后必须按计划下线，避免长期双轨增加心智负担、隐藏退化。

## 时间锚点

记 W6 全部 task（含 7.9 最终检查点）合入 main 的日期为 **D**。
后续节点都是相对 D 的天数。每个里程碑独立 PR 合入，**禁止**捆绑。

| 节点 | 日期 | 主要动作 | 验收 |
|---|---|---|---|
| **D**     | 升级合入 | W6 合入 main；新指标全量启用；灰度开关默认走"新协议路径" | `scripts/check-baseline.sh` 通过；`/api/outcomes/autonomy` 上线 |
| **D+7**   | +7 天   | 升级度量脚本：`legacy_mode_unchecked` 计数应当 → 0；如非 0 → 暂缓后续步骤 | 仪表盘连续 7 天 `legacy_mode_unchecked == 0` |
| **D+14**  | +14 天  | 移除 `MemoryFactRepr::Plain` 反序列化兼容；老 `Vec<String>` 输入返回 400；删除 `auto_upgrade_plain_facts` 相关警告 | `cargo test --lib` 不再覆盖 Plain 分支；新 PBT 覆盖 Structured-only round-trip |
| **D+21**  | +21 天  | 删除 `autonomyProtocolEnabled` / `knowledgeRoutingMode` 灰度开关、删除 `2026_05_005_memory_facts_to_structured` 等 W5 一次性迁移脚本、删除 `held_for_human` 历史值的兼容兜底 | 配置默认值完全无开关；迁移脚本删除后 `cargo test` 仍通过 |

> **回滚原则**：任意里程碑发现仪表盘 7 指标显著退化（`revision_pass_rate < 0.6`、`unverified_claim_block_rate` 异常飙升、`outbox_send_success_rate < 0.95`）→ 回滚到上一锚点；不要"在退化基础上继续推下一锚点"。

## 灰度开关行为对照表

### 1. `runtime_parameters.autonomyProtocolEnabled`

`OperationDomainConfig.runtime_parameters.autonomyProtocolEnabled: bool`，
默认 `true`。控制 Reply Agent 是否走 9 字段自治协议（vs. 老
"reply_text + should_reply" 短形式）。

| 阶段 | 行为 |
|---|---|
| 升级前（W0 之前）| 不存在该字段；Reply Agent 走旧短形式 |
| W1–W5 灰度期    | `true` → 走 `RawAgentDecision::validate_and_promote`；`false` → 走旧路径 + risks `legacy_mode_unchecked` |
| **D 起**         | 默认 `true`；运维显式置 `false` 时仅记录 `legacy_mode_unchecked`，不参与 7 指标分子分母 |
| **D+7 起**       | `false` 触发结构化告警（`agent_events kind="legacy_mode_unexpectedly_active"`） |
| **D+21 起**      | 字段被删除；`config` 反序列化时忽略，运维侧无任何兜底，老配置启动报警一次后等同 `true` |

### 2. `runtime_parameters.knowledgeRoutingMode`

字符串枚举：`"tool_calling"`（新）、`"prompt_inline"`（老）。默认
`"tool_calling"`。

| 阶段 | 行为 |
|---|---|
| 升级前 | 知识库整段塞 prompt（`prompt_inline` 隐含路径） |
| W3–W5 灰度期 | `"tool_calling"` 走 `reply_with_tools_loop` (`list_catalog → search → open_slice`)；`"prompt_inline"` 走旧路径 |
| **D 起**     | 默认 `"tool_calling"`；老 `"prompt_inline"` 仅老灰度场景使用，写 risks `legacy_mode_unchecked` |
| **D+7 起**   | `"prompt_inline"` 触发告警 |
| **D+21 起**  | 字段删除；任何非空老值在启动期记一次 `legacy_runtime_parameter_dropped` 后等同 `"tool_calling"` |

### 3. `MemoryFactRepr::Plain` 兼容反序列化

`#[serde(untagged)] enum MemoryFactRepr { Plain(String), Structured(MemoryFact) }`，允许 `coreFacts: ["纯字符串"]` 落库。

| 阶段 | 行为 |
|---|---|
| 升级前 | `coreFacts: Vec<String>` 是唯一形态 |
| W5 灰度期（D 起） | `Plain` 在反序列化时透传；consolidator 路径 `MemoryCardTyped::auto_upgrade_plain_facts` 在写库前升级为 `Structured`，并写 `agent_run_logs.memory_consolidator_warnings` 含 `memory_facts_auto_upgraded:<count>` |
| **D+14 起**       | 删除 `MemoryFactRepr::Plain` 分支；老 `Vec<String>` 输入直接返回 400 / 反序列化错误 |
| **D+21 起**       | 删除 `auto_upgrade_plain_facts` 与对应单元测试；只保留 Structured-only PBT |

### 4. `OutboxStatus::FailedTerminal` 与历史 `failed`

W4 之前历史 outbox 行可能写过老枚举值 `"failed"`（无 `_terminal` 后缀）。
W4 新代码统一写 `failed_terminal`，但读路径仍兼容。

| 阶段 | 行为 |
|---|---|
| W4 升级 | 写路径只写 `failed_terminal`；读路径 `OutboxStatus::from_str("failed")` 返回 `FailedTerminal` |
| **D+21 起** | 删除老值兼容；读到 `"failed"` 直接 panic（迁移脚本已把所有老行升级） |

### 5. `finalReviewStatus` 中的 `held_for_human`

老枚举值；产品定位转向"全自治、无人工接管"后该值不再合法。

| 阶段 | 行为 |
|---|---|
| W2 起 | 写库前 `assert_final_review_status_valid` 阻断 `held_for_human`，老行视为脏数据；7 指标统计时跳过 |
| **D+21 起** | `assert_final_review_status_valid` 改为 panic；统计层删除"跳过 `held_for_human`"分支 |

### 6. 一次性迁移脚本

`db/migrations/` 下 W5 / W4 / W3 升级期写的迁移脚本：

| 脚本 | 作用 |
|---|---|
| `2026_05_001_*` | seed `system_taxonomies` 第一版字典 |
| `2026_05_002_*` | 给 `agent_run_logs` 补 `(account_id, finalReviewStatus, started_at)` 复合索引 |
| `2026_05_003_*` | 给 `agent_send_outbox` 加 `(idempotency_key) unique` |
| `2026_05_004_*` | 老 `agent_run_logs.finalReviewStatus="held_for_human"` → 标记为 `legacy_held` |
| `2026_05_005_memory_facts_to_structured` | `Vec<String>` coreFacts/recentFacts → 结构化升级 |

| 阶段 | 行为 |
|---|---|
| **D 起** | 迁移脚本仍在 `migrations::run` 链路里，幂等 |
| **D+21 起** | 移除整个 `2026_05_*` 迁移文件（保留一份归档目录 `db/migrations/_archive/`，方便审计） |

## "双轨长期维护"协议违规检测项（PR review checklist）

任意 PR 评审时若命中下述任一项，**必须打 `regression-risk` 标签并由
spec maintainer 复核**；不得以"小改动"为由直接合入：

1. **新增灰度开关**：`runtime_parameters` 新增 `*_enabled / *_mode / *_legacy_*` 字段。
   理由：W6 之后不应再引入"老 + 新两条路径"；任何新能力直接走单条路径，
   出问题就回滚 commit，而不是加开关藏起来。

2. **保留 `MemoryFactRepr::Plain` 分支**：写路径里出现新的
   `MemoryFactRepr::Plain(...)`（除迁移脚本外）→ 拒收。
   理由：D+14 后该分支应当只在反序列化兜底里见到一次。

3. **新增老 `finalReviewStatus` / `gateway_status` 枚举值**：
   `held_for_human / failed`（无 `_terminal`）/ `human_takeover` /
   `manual_*` 等老词。
   理由：违反 R2.7"全自治、无人工接管"定位；CI lint
   `scripts/check-no-human-takeover.sh` 是第一道关，PR review 是兜底。

4. **绕过统一发送网关**：webhook 自动回复 / follow-up task / 手动重试 / 测试
   helper 直接调用 `mcp::send_text_message`，未经 `agent_send_outbox` +
   `process_entry` 闭环。
   理由：R13.10 要求所有 send 必走 outbox；绕过就丢幂等。

5. **绕过 `assert_final_review_status_valid`**：写 `agent_run_logs` /
   `agent_decision_reviews` 时直接拼字符串 status。
   理由：R9.10 闭枚举不允许在写入端做"宽进"。

6. **新增"老 vec<String> coreFacts"输入接口**：在 routes/ 下出现新 endpoint
   接受 `coreFacts: Vec<String>` 字段（即便加了 auto-upgrade 警告）。
   理由：D+14 该输入路径应当返回 400；新接口不得复活老形态。

7. **绕过 `RunBudget`**：在 LLM 调用栈外新增直连 `llm::generate_*`，未传
   budget 句柄。
   理由：MP-5 budget 是单 run 唯一的成本闸口；绕过就让单 run 不收口。

8. **删除已通过的 PBT 用例数**：缩小 `tests/*_pbt.rs` 用例数到 < 64 / 单条
   PBT 执行 < 60s 限制。
   理由：R11.6 baseline 阈值 + design.md "性质测试统一标准"；PBT 不是可选项。

9. **frontend 文案使用"人工接管 / 人工介入"等词**：`frontend/src/` 下新增
   `Tab` / `Toast` / `Tooltip` 文案命中
   `scripts/check-no-human-takeover.sh` 严禁词。
   理由：用户可见层是产品定位的最后一公里；CI lint 已强制，但 reviewer
   要二次确认中文表述合规。

> Reviewer 操作流程：发现命中项 → 直接 request changes 并贴本节锚点链接 →
> 由作者主动撤销违规改动；不要把"反正有 lint"作为放行理由。
