# user-ops 三件套工具循环死代码下线 设计

> 2026-06-23 全仓审查 #1「user-ops 三件套工具循环真死」修复设计。
> 关联：[[project_codebase_audit_2026_06_23]]、`docs/sunset-plan.md`(D+21 里程碑)、[[project_agent_first_no_keyword_filters]]。

## 问题（实证）

user-ops 知识检索的**生产链路**是 single-pass：`route_operation_knowledge`(knowledge_router.rs)→ `select_operation_knowledge_chunks` → `decide_reply_with_promote`(gateway.rs:910-938)。Reply Agent 一次拿着真实知识做决策。

W3-W5 灰度期为"双轨兼容"引入的另一条路径——`reply_with_tools_loop`(tool_loop.rs，多轮 LLM 工具调用 `list_catalog → search → open_slice`)+ `knowledgeRoutingMode` 灰度开关——**从未在生产接线**：

- `tool_loop.rs` 顶部 `#![allow(dead_code)]`，作者自承未接生产。
- `reply_with_tools_loop`(:126)全部 8 个调用点都在 `#[cfg(test)]` 内（tests + pbt_tests），无任何生产入口（webhook / tasks / routes / spawn worker）触达。
- `knowledge_routing_mode` 被 clamp、序列化进 envelope、存储，但**没有任何 `if/match` 读它的值来选执行路径**——gateway 根本不读它。是"挂着但不接线"的功能性死参数。

`docs/sunset-plan.md` 第 18/49 行的 **D+21 里程碑明文规定删除 `knowledgeRoutingMode` 灰度开关**（"配置默认值完全无开关"）。第 103 行把"长期保留灰度开关"列为 PR 违规项。本设计是**兑现项目自己的 sunset 计划**，非新决定。

### agent-first 守住

删除只移除从未启用的备选路径，生产的 single-pass 语义检索（`route_operation_knowledge` + `knowledge_agent`）一字不动。不引入关键词匹配（[[project_agent_first_no_keyword_filters]]）。

## 删除范围（4 组，牵连递增）

### A. tool_loop.rs 整文件下线

- 删 `src/agent/tool_loop.rs`（802 行）+ `src/agent/mod.rs:46` 的 `mod tool_loop;`。
- 随文件删除的：`reply_with_tools_loop`、`ToolLoopOutcome`、`ToolLoopError`、`ToolLoopReplyResult/Fn`、常量 `TOOL_CALLS_PER_TURN_CAP`/`TOOL_LOOP_TOTAL_TIMEOUT`/`TOOL_FAILURE_STREAK_LIMIT`/`TOOL_RESULT_CONTEXT_MAX_CHARS`/`TOOL_TRACE_MAX_LEN`、~9 个测试（8 个 `#[test]/#[tokio::test]` + pbt_tests 的 proptest `p7_loop_terminates_and_budget_never_bypassed`）。
- **改 `chat_tool_loop.rs:486-502`** 的对账测试 `chat_tool_loop_constants_are_aligned_with_design`：原断言 `CHAT_TOOL_* == crate::agent::tool_loop::TOOL_*`（跨模块引用 3 处），改为断言 `CHAT_TOOL_* == 内联字面量`。chat 侧自有常量真实值（已实证）：`CHAT_TOOL_FAILURE_STREAK_LIMIT=3`、`CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS=8000`、`CHAT_TOOL_TRACE_MAX_LEN=32`。测试保留（验证 chat 侧金标），仅比较对象从跨模块引用换成内联金标，语义不变、仍真咬住。
  - **精确范围**：该测试里另有 `assert_eq!(CHAT_TOOL_LOOP_MAX_LOOPS, 4)` 和 `assert_eq!(CHAT_TOOL_CALLS_PER_TURN_CAP, 6)` 两行是 chat 侧自有字面量断言（不引用 tool_loop），**保持不动**。只改引用 `tool_loop::TOOL_*` 的那 3 行。

### B. knowledge_tools.rs 的 user-ops dispatch 半边连带删

- 删 `dispatch_tool_call`（knowledge_tools.rs:181）+ 它的 3 个测试（:1598/:1610/:1626，`dispatch_unknown_tool_returns_error_value` / `dispatch_returns_budget_exceeded_when_quota_zero` / `dispatch_list_catalog_happy_path_consumes_budget`）。
  - **命名澄清**：这是 `async fn`（项目历史注释称其为"sync 版"是相对 chat 多轮循环的语义对比，非 Rust sync）。判定依据是**调用关系**：唯一生产调用点是 `tool_loop.rs:288`，删 tool_loop 后它连带变死。
- **不碰**（chat 活路径 chat_tool_loop.rs:234 依赖）：`dispatch_chat_tool_call`（:655）、`ToolDispatchState`、`AnchorMatchFn`、`TOOL_OPEN_SLICE/TOOL_SEARCH/TOOL_SEARCH_CHUNKS`、`ALLOWED_CHAT_TOOL_NAMES`，及 mod.rs:135 的相关导出。
- 删 `dispatch_tool_call` 后检查 knowledge_tools.rs 内是否有**仅被它使用**的私有 helper / import 变死，一并清理（实现时按 `cargo check` warning 定位）。

### C. 两个 runtime 参数全下线（兑现 sunset-plan D+21）

删 `knowledge_routing_mode` + `knowledge_max_tool_loops`，及其全部存在点：

| 文件:行 | 内容 |
| --- | --- |
| `runtime.rs:52,55` | UserRuntimeParameters 字段声明 |
| `runtime.rs:162-163,318-319` | 2 处 from-typed 构造（`clamp_knowledge_routing_mode(...)` + `clamp_i32(typed.knowledge_max_tool_loops, 1, 5, 3)`） |
| `runtime.rs:205-206` | envelope JSON 序列化 |
| `runtime.rs:283-288` | `clamp_knowledge_routing_mode` 函数整体 |
| `runtime.rs:592-593` | Default impl |
| `models.rs:3189-3194` | RuntimeParametersTyped 字段 + serde default 属性 |
| `models.rs:3255-3256` | RuntimeParametersTyped Default impl |
| `models.rs:3332-3335` | `defaults::knowledge_routing_mode` / `knowledge_max_tool_loops` 函数 |
| `mod.rs:541-542` | 构造默认值 |
| `types.rs:1630-1631` | 构造默认值 |
| `run_envelope.rs:1559-1560` | 构造默认值 |

- **⚠️ 关键护栏（最高优先级，防误删）**：`knowledge_max_tool_loops`（**删**，带 `_loops` 后缀）≠ `knowledge_max_tool_calls`（**留，是活路径**——budget.rs 注入 tool_call_budget，gateway.rs:581 / memory.rs:890 / reaction.rs:44 都在用）。两个名字极像。所有 grep / 替换必须用**全词 `knowledge_max_tool_loops`**，逐处肉眼确认是 `_loops` 而非 `_calls`。
- `clamp_i32` 是通用 helper（别处 5 个参数在用），**只删对它的 2 次调用（:163/:319），不删函数**。
- **向后兼容（命门，已实证）**：`RuntimeParametersTyped` 只标 `#[serde(rename_all = "camelCase")]`，**无 `deny_unknown_fields`**。删字段后，旧 BSON / run envelope 文档里残留的 `knowledgeRoutingMode` / `knowledgeMaxToolLoops` 被 serde **静默忽略**，反序列化不报错。sunset-plan 设想的 `legacy_runtime_parameter_dropped` 启动日志机制**从未实装**（grep 空），无需为本删除补建——serde 默认忽略已足够。
- 前端**零使用**这两个参数（已 grep 确认 frontend/src 全空），无前端改动。

### D. 文档同步

- `docs/sunset-plan.md`：在 knowledgeRoutingMode 段（第 38-49 行）标注"D+21 已兑现：字段与 reply_with_tools_loop 已删除（commit 待填）"。
- `autonomyProtocolEnabled`（sunset-plan 同 D+21 删除项）**不在本次范围**——它是独立开关，需单独核实是否仍有读点，避免范围蔓延。
- 其它 `.kiro/specs/*` 里引用 tool_loop / classic_router 的历史规范文档**不动**（是历史规范快照，非代码）。

## 测试影响

1. **tool_loop.rs ~9 个测试随文件删**：死代码的自测随死代码走，不违反"测试只增量"铁律（铁律针对活测试维度/弧/金标）。
2. **knowledge_tools.rs 3 个 dispatch_tool_call 测试删**：测的是即将删的函数。
3. **chat_tool_loop.rs 对账测试改写**（非删）：比较对象换内联金标，测试本身保留。
4. **新增 1 条向后兼容回归测试**（增量）：在 models.rs 的 RuntimeParametersTyped 测试区，加一条反序列化测试——喂含老 `knowledgeRoutingMode` + `knowledgeMaxToolLoops` 字段的 BSON 文档，断言 `from_document` 成功（serde 忽略未知字段）。这是删字段唯一真实风险点，必须钉住。
5. **lib 基线**：删 ~12 个测试后 lib 数下降，但当前 1500 ≫ 基线门 350，删后仍 ≫350。`check-baseline.{sh,ps1}` 的 `LIB_BASELINE=350` 不动。

## 约束

- `cargo check --tests` 0 error / 0 warning（删代码易留 unused import/warning，必须清干净）。
- `cargo test --lib` ≫350 / 0；四 PBT ≥33 / 0 不回归。
- 磁盘受限：本地 `cargo check` + 点跑小单测；全量留 CI 基线门。
- 红线双门 `check-no-human-takeover` / `check-no-model-hint` clean（本删除不涉红线措辞）。
- 纯减法：不引入新行为、新抽象、新开关（sunset-plan 第 103 行禁止新开关）。
- 精确 `git add`，排除并行产物（.kiro/* / agent_t*.txt / 其它计划文件）。
- 提交需用户显式批准。

## 风险与回滚

- **最大风险 = 误删 `knowledge_max_tool_calls`**（活路径）。缓解：C 组护栏 + 删后必跑 `cargo check --tests`，若 budget/gateway/memory/reaction 报错说明误删，立即回退。
- **向后兼容风险**：旧 envelope 反序列化。缓解：测试影响 #4 的回归测试 + 无 deny_unknown_fields 已实证。
- 删 tool_loop 可能暴露 knowledge_tools.rs 内仅被它用的私有项变死。缓解：`cargo check` warning 逐个清。
- 回滚：纯删除，`git revert` 即恢复。死代码无生产行为，回滚无副作用。
