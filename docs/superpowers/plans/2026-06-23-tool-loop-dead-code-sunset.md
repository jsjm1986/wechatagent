# user-ops 工具循环死代码下线 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 下线 W3-W5 灰度期引入但从未接生产的 user-ops 多轮工具循环（`reply_with_tools_loop` + `knowledgeRoutingMode` 灰度开关），兑现 `docs/sunset-plan.md` D+21 里程碑。

**Architecture:** 纯减法删除，分 4 组牵连递增：tool_loop.rs 整文件 → knowledge_tools.rs 的 user-ops dispatch 半边（精确保留 chat 共用项）→ 两个 runtime 灰度参数全下线 → 文档同步。生产 single-pass 检索链路（`route_operation_knowledge` + `knowledge_agent`，gateway.rs）一字不动。

**Tech Stack:** Rust 2021 / Axum / MongoDB（bson serde）。

## Global Constraints

- 子代理 ALWAYS `model: "opus"`；回复中文。
- `cargo test --lib` ≫350 passed / 0 failed（当前 1500，删 ~12 测试后仍 ≫350，`LIB_BASELINE=350` 不动）；四 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33 / 0。
- 后端编译验证用 `RUSTFLAGS="-D warnings" cargo check --tests`（删代码易留 unused import/warning，必须 0 warning）。磁盘受限：本地只 `cargo check` + 点跑小单测，全量留 CI。
- **纯减法**：不引入新行为、新抽象、新开关（sunset-plan 第 103 行禁止新灰度开关）。
- **agent-first**：不动 customer_stage 的 LLM 语义判定、不引关键词匹配。生产检索链路不碰。
- 精确 `git add` 指定文件，排除并行产物（`.kiro/*` `AGENTS.md` `agent_t*.txt` `t15_single.txt` `dead-code-analysis.md` 及其它 `??` 计划文件）。
- 红线双门 `check-no-human-takeover` / `check-no-model-hint` clean。
- 提交需用户显式批准。

## ⚠️ 最高优先级护栏（防误删，已实证）

1. **`knowledge_max_tool_loops`（删，带 `_loops`）≠ `knowledge_max_tool_calls`（留，活路径）**。后者被 budget.rs 注入 tool_call_budget，gateway.rs:581 / memory.rs:890 / reaction.rs:44 都在用。两名极像。所有 grep/替换用**全词 `knowledge_max_tool_loops`**，逐处肉眼确认 `_loops` 而非 `_calls`。
2. **`dispatch_tool_call`（删）≠ `dispatch_chat_tool_call`（留，chat 活路径）**。后者 chat_tool_loop.rs:234 调。
3. **`exec_list_catalog` / `exec_search` / `exec_open_slice` / `TOOL_LIST_CATALOG` / `TOOL_SEARCH` / `TOOL_OPEN_SLICE`（全留）**：已实证 chat 半边 `dispatch_chat_tool_call`（:680-682）共用，删 dispatch_tool_call 后**不变死**，不删。
4. **`clamp_i32`（留）**：通用 helper，别处 5 个参数在用。只删对它的 2 次调用，不删函数。

## File Structure

| 文件 | 本计划改动 |
| --- | --- |
| `src/agent/tool_loop.rs` | 删整文件（802 行） |
| `src/agent/mod.rs` | 删 `mod tool_loop;`（:46） |
| `src/agent/chat_tool_loop.rs` | 改对账测试 3 行跨模块引用 → 内联金标 |
| `src/agent/knowledge_tools.rs` | 删 `dispatch_tool_call`（:181）+ `ALLOWED_TOOL_NAMES`（:93）+ 3 测试 |
| `src/agent/runtime.rs` | 删 2 参数字段/构造/序列化/clamp 函数/Default |
| `src/models.rs` | 删 RuntimeParametersTyped 2 字段/默认/defaults 函数 + 新增 1 兼容测试 |
| `src/agent/mod.rs` `types.rs` `run_envelope.rs` | 删 3 处构造默认值 |
| `docs/sunset-plan.md` | 标注 D+21 已兑现 |

**任务顺序**：T1（tool_loop 文件 + 对账测试，自包含）→ T2（knowledge_tools dispatch 半边）→ T3（runtime 参数全下线，跨 runtime/models/mod/types/run_envelope）→ T4（文档）。T1/T2 删代码会让 T3 的部分默认值点更易识别，顺序执行最稳。

---

## Task 1：删 tool_loop.rs 整文件 + 改 chat 对账测试

**Files:**
- Delete: `src/agent/tool_loop.rs`
- Modify: `src/agent/mod.rs:46`（删 `mod tool_loop;`）
- Modify: `src/agent/chat_tool_loop.rs:488-500`（对账测试 3 处跨模块引用改内联字面量）

**Interfaces:**
- Consumes: 无
- Produces: 无（纯删除 + 测试改写）

**安全边界（已实证）**：`reply_with_tools_loop` 全部 8 调用点在 `#[cfg(test)]`，无生产入口；`#![allow(dead_code)]` 自承。chat_tool_loop.rs 生产逻辑用自己的 `CHAT_TOOL_*` 常量，只在对账测试里引用 tool_loop 的 3 个常量。

- [ ] **Step 1: 确认 reply_with_tools_loop 无生产调用**

Run: `grep -rn "reply_with_tools_loop\|tool_loop::" src/ | grep -v "src/agent/tool_loop.rs" | grep -v "//"`
Expected: 只剩 chat_tool_loop.rs:491/495/499（对账测试引用 3 常量）。若出现任何非测试、非注释的生产调用，**停止报告**（说明不是死代码）。

- [ ] **Step 2: 删文件 + mod 声明**

```bash
rm src/agent/tool_loop.rs
```
删 `src/agent/mod.rs:46` 的 `mod tool_loop;` 整行。

- [ ] **Step 3: 改 chat 对账测试为内联金标**

`src/agent/chat_tool_loop.rs:487-502` 的 `chat_tool_loop_constants_are_aligned_with_design`，把 3 处引用 `crate::agent::tool_loop::TOOL_*` 的 `assert_eq!` 改成内联字面量（值已实证：3 / 8000 / 32）。**保持 `CHAT_TOOL_LOOP_MAX_LOOPS==4` 和 `CHAT_TOOL_CALLS_PER_TURN_CAP==6` 两行不动**（它们是 chat 侧自有字面量断言，不涉 tool_loop）：

```rust
    /// 验证常量与设计金标一致：CHAT_TOOL_CALLS_PER_TURN_CAP=6,
    /// CHAT_TOOL_LOOP_MAX_LOOPS=4。failure_streak/context/trace 金标与
    /// user-ops 工具循环下线前同源（3 / 8000 / 32）。
    #[test]
    fn chat_tool_loop_constants_are_aligned_with_design() {
        assert_eq!(super::CHAT_TOOL_LOOP_MAX_LOOPS, 4);
        assert_eq!(super::CHAT_TOOL_FAILURE_STREAK_LIMIT, 3);
        assert_eq!(super::CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS, 8000);
        assert_eq!(super::CHAT_TOOL_TRACE_MAX_LEN, 32);
        assert_eq!(super::CHAT_TOOL_CALLS_PER_TURN_CAP, 6);
    }
```

- [ ] **Step 4: 编译验证**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 报错只应是 knowledge_tools.rs 的 `dispatch_tool_call` 等因 tool_loop.rs 删除而"未使用"或别处对 tool_loop 的引用——**预期**：此刻 `dispatch_tool_call`（knowledge_tools.rs:181）失去唯一生产调用者 tool_loop.rs:288，可能触发 dead_code warning（`-D warnings` 下变 error）。这是 T2 要删的，预期残留。若出现**其它**意外错误（chat 活路径断裂、非 dispatch_tool_call 相关），停止报告。

> 说明：T1 单独可能不全绿（dispatch_tool_call 变 unused）。这是预期，T2 紧接删它。chat_tool_loop.rs 测试改写后自身应无错。

- [ ] **Step 5: Commit**

```bash
git add src/agent/tool_loop.rs src/agent/mod.rs src/agent/chat_tool_loop.rs
git commit -m "chore(agent): 删 tool_loop.rs 死代码整文件+chat对账测试改内联金标(#1,#![allow(dead_code)]自承未接生产)"
```

---

## Task 2：删 knowledge_tools.rs 的 user-ops dispatch 半边

**Files:**
- Modify: `src/agent/knowledge_tools.rs`（删 `dispatch_tool_call`:181 函数整体、`ALLOWED_TOOL_NAMES`:93-94 常量、3 个测试 :1598/:1610/:1626；相关注释 :5/:79/:120 清理）

**Interfaces:**
- Consumes: 无
- Produces: 无（删 user-ops 半边）

**安全边界（已实证，精确清单）**：
- **删**：`dispatch_tool_call`（:181，唯一生产调用者 tool_loop.rs:288 已随 T1 删）、`ALLOWED_TOOL_NAMES`（:93，仅 dispatch_tool_call:189 用）、3 个测试（`dispatch_unknown_tool_returns_error_value` / `dispatch_returns_budget_exceeded_when_quota_zero` / `dispatch_list_catalog_happy_path_consumes_budget`）。
- **保留（chat 半边 dispatch_chat_tool_call:680-682 共用，删 dispatch_tool_call 后不变死）**：`dispatch_chat_tool_call`、`exec_list_catalog`、`exec_search`、`exec_open_slice`、`exec_search_chunks`、`TOOL_LIST_CATALOG`、`TOOL_SEARCH`、`TOOL_OPEN_SLICE`、`TOOL_SEARCH_CHUNKS`、`ALLOWED_CHAT_TOOL_NAMES`、`ToolDispatchState`、`AnchorMatchFn`。

- [ ] **Step 1: 确认 ALLOWED_TOOL_NAMES 仅 dispatch_tool_call 用**

Run: `grep -n "ALLOWED_TOOL_NAMES" src/agent/knowledge_tools.rs`
Expected: 只有定义（:93-94）+ dispatch_tool_call 内引用（:189）。若别处也用，保留该常量、只删 dispatch_tool_call。

- [ ] **Step 2: 删 dispatch_tool_call 函数 + ALLOWED_TOOL_NAMES + 3 测试**

删 `dispatch_tool_call`（:181 到其函数体结束）；删 `ALLOWED_TOOL_NAMES`（:93-94 常量定义）；删 3 个测试函数（:1598/:1610/:1626 各自整体）。清理顶部注释里专指 user-ops dispatch 的描述（:5/:79/:120，措辞改为只描述 chat dispatch，或删 user-ops 特定句）。**不碰** Step「保留」清单里的任何符号。

- [ ] **Step 3: 编译验证 0 warning**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 0 error / 0 warning（T1 的 dispatch_tool_call unused 残留此刻消除；exec_*/TOOL_* 因 chat 半边仍用故无 unused）。若报某 exec_*/常量 unused，说明它其实不被 chat 共用——**停止核实**（可能判断有误），勿盲目删。

- [ ] **Step 4: 点跑 chat 活路径测试确认未误伤**

Run: `cargo test --lib chat_tool_loop 2>&1 | tail -10`
Expected: PASS（chat 半边及其常量未受影响）。
Run: `cargo test --lib knowledge_tools 2>&1 | tail -10`
Expected: PASS（保留的 exec_* 测试仍在且过）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/knowledge_tools.rs
git commit -m "chore(agent): 删 user-ops dispatch_tool_call 半边+ALLOWED_TOOL_NAMES(#1,唯一调用者tool_loop已删;chat dispatch_chat_tool_call及exec_*共用项保留)"
```

---

## Task 3：两个 runtime 灰度参数全下线

**Files:**
- Modify: `src/agent/runtime.rs`（删字段 :52,:55；2 处构造 :162-163,:318-319；序列化 :205-206；`clamp_knowledge_routing_mode` 函数 :283-288；Default :592-593）
- Modify: `src/models.rs`（删 RuntimeParametersTyped 字段 :3189-3194；Default :3255-3256；`defaults::knowledge_routing_mode`/`knowledge_max_tool_loops` 函数 :3332-3335；新增 1 兼容测试）
- Modify: `src/agent/mod.rs:541-542`、`src/agent/types.rs:1630-1631`、`src/agent/run_envelope.rs:1559-1560`（删 3 处构造默认值）

**Interfaces:**
- Consumes: 无
- Produces: `UserRuntimeParameters` 和 `RuntimeParametersTyped` 不再有 `knowledge_routing_mode` / `knowledge_max_tool_loops` 字段。

**安全边界（已实证）**：
- `clamp_i32`（runtime.rs:273）通用 helper，别处 5 参数用——**只删对它涉及 `knowledge_max_tool_loops` 的 2 次调用，不删函数**。
- `knowledge_max_tool_calls`（带 `_calls`，活路径）**全程不碰**。
- `RuntimeParametersTyped` 无 `deny_unknown_fields`（已实证 models.rs:3141 只有 `rename_all`），删字段后旧 BSON 残留字段 serde 静默忽略。

- [ ] **Step 1: 先写向后兼容回归测试（TDD）**

`src/models.rs` 的 RuntimeParametersTyped 测试区（紧邻 `runtime_parameters_typed_reads_existing_values`，约 :4343 后）加：

```rust
    /// #1 sunset：删除 knowledgeRoutingMode/knowledgeMaxToolLoops 字段后，旧 run envelope /
    /// 配置文档里残留这两个字段时，反序列化必须静默忽略（RuntimeParametersTyped 无
    /// deny_unknown_fields），不破历史数据。
    #[test]
    fn runtime_parameters_typed_ignores_dropped_legacy_routing_fields() {
        let doc = doc! {
            "recentMessageLimit": 18,
            "knowledgeRoutingMode": "auto_tool_loop",
            "knowledgeMaxToolLoops": 3,
            "knowledgeMaxToolCalls": 6
        };
        let p: RuntimeParametersTyped =
            mongodb::bson::from_document(doc).expect("含已删字段的旧文档仍应反序列化成功");
        assert_eq!(p.recent_message_limit, 18);
        // 未删的 _calls 仍正常读取（防误删护栏的反向证明）。
        assert_eq!(p.knowledge_max_tool_calls, 6);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib runtime_parameters_typed_ignores_dropped_legacy_routing_fields 2>&1 | tail -10`
Expected: 编译失败（此刻 `knowledge_routing_mode`/`knowledge_max_tool_loops` 字段仍在，但测试 doc 里也含 `knowledgeMaxToolLoops` —— 实际此刻会因字段仍存在而反序列化进字段，测试可能直接编译过但语义未验证）。

> 修正预期：此刻字段仍在，测试会编译通过（doc 里的 knowledgeMaxToolLoops 被读进字段）。这个测试的真正价值在 Step 4 之后——删字段后它验证"残留字段被忽略"。Step 2 只需确认测试**编译通过、当前能跑**（绿），作为删除前的基线锚点。删字段后（Step 4）它仍须绿（语义变为"忽略未知字段"）。

Run: `cargo test --lib runtime_parameters_typed_ignores_dropped_legacy_routing_fields 2>&1 | tail -5`
Expected: PASS（删除前基线）。

- [ ] **Step 3: 删 runtime.rs 的 2 参数**

删 `knowledge_routing_mode`（:52）、`knowledge_max_tool_loops`（:55）字段；删 2 处 from-typed 构造里这两行（:162-163 区、:318-319 区——`clamp_knowledge_routing_mode(...)` 那行 + `clamp_i32(typed.knowledge_max_tool_loops, 1, 5, 3)` 那行，**保留同区其它 clamp_i32 调用**）；删序列化 :205（`"knowledgeRoutingMode"`）、:206（`"knowledgeMaxToolLoops"`）；删 `clamp_knowledge_routing_mode` 函数整体（:280-289 含 doc）；删 Default 的两行（:592-593）。

- [ ] **Step 4: 删 models.rs 的 2 参数**

删 RuntimeParametersTyped 字段（:3189-3190 含 serde default 属性、:3193-3194）；删 Default impl 两行（:3255-3256）；删 `defaults::knowledge_routing_mode`（:3332-3334）和 `defaults::knowledge_max_tool_loops`（:3335 区）函数。

- [ ] **Step 5: 删 3 处构造默认值**

`src/agent/mod.rs:541-542`、`src/agent/types.rs:1630-1631`、`src/agent/run_envelope.rs:1559-1560` 各删这两行（构造 UserRuntimeParameters 时的 `knowledge_routing_mode: ...` / `knowledge_max_tool_loops: ...`）。

- [ ] **Step 6: 编译 + 兼容测试 + 误删护栏验证**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -20`
Expected: 0 error / 0 warning。若报 `knowledge_max_tool_calls` 相关错误，说明误删了活字段——立即检查 Step3-5 是否手滑删了 `_calls`。
Run: `cargo test --lib runtime_parameters_typed_ignores_dropped_legacy_routing_fields 2>&1 | tail -5`
Expected: PASS（删字段后，doc 里 `knowledgeMaxToolLoops` 被 serde 忽略，`knowledgeMaxToolCalls` 仍读进 `knowledge_max_tool_calls`）。
Run: `grep -rn "knowledge_max_tool_calls\|knowledgeMaxToolCalls" src/agent/runtime.rs src/models.rs | head`
Expected: 仍存在（护栏：活字段未被误删）。

- [ ] **Step 7: Commit**

```bash
git add src/agent/runtime.rs src/models.rs src/agent/mod.rs src/agent/types.rs src/agent/run_envelope.rs
git commit -m "chore(agent): 下线 knowledgeRoutingMode/knowledgeMaxToolLoops 灰度参数(#1兑现sunset-plan D+21;_calls活字段保留;serde忽略旧值兼容)"
```

---

## Task 4：文档同步

**Files:**
- Modify: `docs/sunset-plan.md`（knowledgeRoutingMode 段标注已兑现）

**Interfaces:** Consumes 无；Produces 无。

- [ ] **Step 1: 标注 sunset-plan D+21 已兑现**

`docs/sunset-plan.md` 的 `### 2. runtime_parameters.knowledgeRoutingMode` 段（:38-49），在表格后加一行说明：

```markdown
> **D+21 已兑现（2026-06-23）**：`knowledgeRoutingMode` / `knowledgeMaxToolLoops` 字段及
> `reply_with_tools_loop`（tool_loop.rs）已删除；生产链路统一走 single-pass
> `route_operation_knowledge` + `knowledge_agent`。`RuntimeParametersTyped` 无
> `deny_unknown_fields`，旧文档残留字段反序列化时静默忽略。
```

> `autonomyProtocolEnabled`（同 D+21 删除项）不在本次范围——独立开关，需单独核实读点后另行下线。

- [ ] **Step 2: Commit**

```bash
git add docs/sunset-plan.md
git commit -m "docs(sunset-plan): 标注 knowledgeRoutingMode D+21 已兑现(#1)"
```

---

## Task 5：全量基线 + lint 收口

**Files:** 无（验证任务）

- [ ] **Step 1: 全量编译**

Run: `RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -10`
Expected: 0 error / 0 warning。

- [ ] **Step 2: 受影响 lib 单测**

Run: `cargo test --lib knowledge_tools 2>&1 | tail -8`（保留的 exec_* 测试）
Run: `cargo test --lib chat_tool_loop 2>&1 | tail -8`（chat 活路径 + 改写的对账测试）
Run: `cargo test --lib runtime_parameters_typed 2>&1 | tail -8`（含新增兼容测试）
Expected: 全 PASS。

- [ ] **Step 3: 红线 lint**

Run: `bash scripts/check-no-human-takeover.sh origin/main HEAD 2>&1 | tail -5`
Run: `bash scripts/check-no-model-hint.sh origin/main HEAD 2>&1 | tail -5`
Expected: clean。

- [ ] **Step 4: 推送 + 开 PR（用户批准后）**

```bash
git push -u origin chore/tool-loop-dead-code-sunset
gh pr create --title "chore(agent): 下线 user-ops 工具循环死代码(审查#1 兑现 sunset-plan D+21)" --body "$(cat <<'EOF'
## Summary
- 审查 #1：W3-W5 灰度期引入的 user-ops 多轮工具循环 `reply_with_tools_loop`（tool_loop.rs，`#![allow(dead_code)]` 自承）+ `knowledgeRoutingMode` 灰度开关从未接生产。`docs/sunset-plan.md` D+21 明文规定删除。
- 生产链路统一 single-pass `route_operation_knowledge` + `knowledge_agent`（gateway.rs），一字不动。
- 删：tool_loop.rs 整文件、knowledge_tools.rs 的 `dispatch_tool_call` + `ALLOWED_TOOL_NAMES`、两个 runtime 参数（字段/clamp/序列化/默认）。
- 保留（chat 活路径共用）：`dispatch_chat_tool_call`、`exec_list_catalog/search/open_slice`、`TOOL_*` 常量；`knowledge_max_tool_calls`（活，≠ 删除的 `_loops`）；`clamp_i32`（通用 helper）。
- 向后兼容：`RuntimeParametersTyped` 无 `deny_unknown_fields`，旧 envelope 残留字段 serde 静默忽略。

## Test plan
- [x] `cargo check --tests` 0 error / 0 warning
- [x] `cargo test --lib` ≫350/0（删 ~12 死代码测试，远超基线门 350）
- [x] 新增 `runtime_parameters_typed_ignores_dropped_legacy_routing_fields`（向后兼容 + 误删护栏）
- [x] chat 活路径 + 对账测试 + knowledge_tools exec_* 测试全过
- [x] 红线双门 clean
- [ ] CI 基线门 + Integration job

设计：`docs/superpowers/specs/2026-06-23-tool-loop-dead-code-sunset-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## 自审

**1. Spec 覆盖**：设计 4 组删除 → T1（A 组 tool_loop + 对账）/ T2（B 组 dispatch 半边）/ T3（C 组 runtime 参数）/ T4（D 组文档）。测试影响 5 条 → T1（删 tool_loop 测试 + 改对账）/ T2（删 3 dispatch 测试）/ T3 Step1（新增兼容测试）/ T5（基线）。全覆盖。

**2. Placeholder 扫描**：无 TBD/TODO；每个删改 Step 给了精确坐标 + 完整代码（对账测试、兼容测试全文）+ 确切命令与预期。T1 Step4「单独不全绿」已显式说明是预期（dispatch_tool_call 变 unused，T2 紧接删）。

**3. 边界一致性**：删除清单（dispatch_tool_call/ALLOWED_TOOL_NAMES/2 参数）与保留清单（dispatch_chat_tool_call/exec_*/TOOL_*/knowledge_max_tool_calls/clamp_i32）在护栏节、T2 安全边界、T3 安全边界三处一致。`_loops` vs `_calls` 护栏贯穿 T3 全程。

**4. 风险点**：T2 删 dispatch_tool_call 后若 exec_* 报 unused → 说明"chat 共用"判断有误，Step3 已要求"停止核实勿盲删"。T3 误删 `_calls` → Step6 grep 护栏 + check 报错双重兜底。
