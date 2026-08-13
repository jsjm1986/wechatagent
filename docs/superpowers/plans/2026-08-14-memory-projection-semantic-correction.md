# Memory Projection Semantic Correction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让投影 AI 在完整语境中区分真实事实更正与玩笑/假设，并把高置信更正送入现有候选、自动固化和冲突裁决链路。

**Architecture:** 只增强系统内置 `user.projection.task` 的语义与 JSON 协议，不增加 Rust 关键词抽取或直接 memoryCard 写入。验收脚本按 webhook source、parent run、projection child run、candidate、task claim generation 和 commit event 的精确身份逐层取证。

**Tech Stack:** Rust 2021、MongoDB BSON、Python 3 biz-test、systemd transient units、loopback MCP stub。

## Global Constraints

- 客户陈述的真伪、玩笑、反讽、假设和转述必须由 AI 结合上下文判断，不得新增字符串匹配兜底。
- 只允许经 `memory_candidates` 和 durable `memory_consolidation` task 更新长期记忆。
- 无证据、低置信或语境不明确时保持空候选。
- 系统 Prompt 内容漂移只升级 `seeded_by=system` 的 current；运营手编和演化发布版本不覆盖。
- 所有服务器测试使用随机 MongoDB、独立端口和 loopback MCP；不得切换或重启正式服务。
- 未经用户单独要求不创建 Git commit。

---

### Task 1: Projection Prompt Contract

**Files:**
- Modify: `src/prompts.rs:1311-1345`
- Test: `src/prompts.rs` 内联测试模块

**Interfaces:**
- Consumes: `prompt_specs() -> Vec<PromptSpec>`、现有 `user.projection.task`
- Produces: 完整的 `memoryCandidates[]` item wire contract 和 AI 语境判断规则

- [ ] **Step 1: Write the failing Rust test**

在 `src/prompts.rs` 测试模块新增：

```rust
#[test]
fn projection_schema_defines_semantic_memory_correction_contract() {
    let task = prompt_specs()
        .into_iter()
        .find(|spec| spec.key == "user.projection.task")
        .expect("projection task exists");

    for field in [
        "\"type\": \"fact\"",
        "\"content\":",
        "\"evidence\":",
        "\"importance\":",
        "\"confidence\":",
    ] {
        assert!(task.content.contains(field), "missing candidate field {field}");
    }
    for semantic_guard in ["玩笑", "反讽", "假设", "转述", "conflict", "consolidationNeeded"] {
        assert!(
            task.content.contains(semantic_guard),
            "missing semantic memory guard {semantic_guard}"
        );
    }
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test --lib projection_schema_defines_semantic_memory_correction_contract
```

Expected: FAIL because the current schema only contains `"memoryCandidates": []`.

- [ ] **Step 3: Add the minimal Prompt protocol**

把 schema 中的空数组示例改为：

```json
"memoryCandidates": [
  {
    "type": "fact",
    "content": "一条原子化、可长期使用的信息",
    "evidence": "客户原话或有上下文的行为证据",
    "importance": 8,
    "confidence": 8
  }
]
```

并在规则中明确：

```text
- 输出前先判断信息是否来自客户本人，是否为认真、明确、当前有效且有长期价值的陈述。
- 玩笑、反讽、假设、试探、转述他人或无法确认的信息不得写入 memoryCandidates。
- 若客户高置信地修正 memoryCard 中同一属性，输出 type=conflict 的候选，保留客户原话证据，
  memoryWriteScore/importance/confidence 均设为 8–10，并令 consolidationNeeded=true。
- 没有新增长期信息时 memoryCandidates 必须返回空数组；示例对象不是强制填充项。
```

- [ ] **Step 4: Run Prompt tests and verify GREEN**

Run:

```bash
cargo test --lib projection_schema_defines_semantic_memory_correction_contract
cargo test --lib prompts::tests
```

Expected: all selected tests pass.

- [ ] **Step 5: Formatting checkpoint**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: exit 0. Do not commit.

---

### Task 2: Exact Candidate Evidence in Domain ⑨

**Files:**
- Modify: `scripts/biz-test/_lib.py`
- Modify: `scripts/biz-test/test_lib.py`
- Modify: `scripts/biz-test/batch_a_domain9.py`

**Interfaces:**
- Produces: `memory_candidate_texts(rows: list[dict]) -> list[str]`
- Consumes: `memory_candidates_for_runs(wxid, run_ids)`

- [ ] **Step 1: Write the failing Python helper test**

在 `scripts/biz-test/test_lib.py` 新增：

```python
def test_memory_candidate_texts_only_extract_semantic_fields(self) -> None:
    rows = [{
        "candidates": [{
            "content": "孩子10岁",
            "evidence": "客户认真更正：不是8岁",
            "importance": 10,
            "confidence": 9,
        }],
    }]
    self.assertEqual(
        lib.memory_candidate_texts(rows),
        ["孩子10岁", "客户认真更正：不是8岁"],
    )
```

- [ ] **Step 2: Run the helper test and verify RED**

Run:

```bash
python3 -m unittest \
  scripts.biz-test.test_lib.WebhookSigningTests.test_memory_candidate_texts_only_extract_semantic_fields -v
```

Expected: FAIL with missing `memory_candidate_texts`.

- [ ] **Step 3: Implement the evidence extractor**

在 `_lib.py` 的 run-scoped memory helper 附近实现：

```python
def memory_candidate_texts(rows: list[dict]) -> list[str]:
    """Extract only semantic candidate text/evidence fields, never numeric scores."""
    texts: list[str] = []
    for row in rows:
        candidates = row.get("candidates", []) if isinstance(row, dict) else []
        if not isinstance(candidates, list):
            continue
        for candidate in candidates:
            if not isinstance(candidate, dict):
                continue
            for key in ("content", "evidence", "text", "value"):
                value = candidate.get(key)
                if isinstance(value, str) and value.strip():
                    texts.append(value.strip())
    return texts
```

- [ ] **Step 4: Bind projected turns to required facts**

在 `batch_a_domain9.py` 增加 `from typing import Optional`，然后扩展
`_send_projected_turn`：

```python
def _send_projected_turn(
    app_id: str,
    content: str,
    tag: str,
    label: str,
    account_id: str,
    *,
    required_age: Optional[str] = None,
    max_attempts: int = 3,
) -> str:
```

每次 projection 完成后，在返回 run_id 前读取：

```python
rows = _lib.memory_candidates_for_runs(WXID, [run_id])
candidate_ages = _ages_in(_lib.memory_candidate_texts(rows))
if required_age is not None and required_age not in candidate_ages:
    print(
        f"[{DOMAIN}] {label} 合法投影未生成 {required_age} 岁候选；"
        f"run_id={run_id}，做有界新 run 重试..."
    )
    continue
```

三次都缺失时用 `_require(False, ...)` 失败，证据包含最后一个 run 及其 candidate rows。

调用约束：

```python
_send_projected_turn(..., "请记住：我孩子今年8岁，目前零基础。", ..., required_age="8")

_send_projected_turn(
    app_id,
    "我刚核对过信息，认真更正：孩子今年10岁，之前说8岁是我记错了。"
    "这不是玩笑，请按10岁更新长期记录。",
    "m9b",
    "B阶段改口轮",
    account_id,
    required_age="10",
)
```

- [ ] **Step 5: Fail fast when the memory card did not advance**

`_wait_memory_card` 返回后，先断言：

```python
_require(
    card_b.get("memory_card_version", 0) > ver_a,
    "B阶段更正触发新 memory_card 版本",
    card_b,
)
```

然后再读取 `memory_source_task_id`，禁止把上一轮 task 误当作本轮 consolidated task。

- [ ] **Step 6: Run Python contract tests**

Run:

```bash
python3 -m unittest scripts/biz-test/test_lib.py -v
python3 -m py_compile \
  scripts/biz-test/_lib.py \
  scripts/biz-test/batch_a_domain9.py
```

Expected: all tests pass and compilation exits 0.

---

### Task 3: Local Verification and Linux Candidate Rebuild

**Files:**
- Deploy source: `src/prompts.rs`
- Deploy scripts: `scripts/biz-test/_lib.py`, `test_lib.py`, `batch_a_domain9.py`
- Candidate runner: `target/wechatagent_isolated_targeted_handoff.sh`

**Interfaces:**
- Produces: Linux x86-64 candidate binary and SHA-256
- Preserves: currently running production PID, restart count and executable hash

- [ ] **Step 1: Run local targeted verification**

Run:

```bash
cargo test --lib projection_schema_defines_semantic_memory_correction_contract
python3 -m unittest scripts/biz-test/test_lib.py -v
cargo fmt --all -- --check
git diff --check
```

Expected: all exit 0.

- [ ] **Step 2: Inspect the server build tree before mutation**

Verify the candidate/build paths, active production PID/hash, architecture and available disk. Do not touch `/opt/wechatagent/current` or `wechatagent.service`.

- [ ] **Step 3: Upload only the verified source and biz-test files**

Use the password-authenticated transfer helper to update the existing isolated candidate build tree.
After upload, compare local and remote SHA-256 for every transferred file.

- [ ] **Step 4: Run server tests before release build**

Run in the Linux build tree:

```bash
cargo test --release projection_schema_defines_semantic_memory_correction_contract
python3 scripts/biz-test/test_lib.py
```

Expected: Rust target passes; all Python contracts pass.

- [ ] **Step 5: Force a fresh Linux release build**

Run:

```bash
cargo clean -p wechatagent
cargo build --release
file target/release/wechatagent
sha256sum target/release/wechatagent
```

Expected: ELF 64-bit x86-64 and a new recorded SHA-256. Copy it only into the isolated candidate app tree.

- [ ] **Step 6: Recheck production invariants**

Verify production PID, `NRestarts`, running executable hash and `/api/health` are unchanged.

---

### Task 4: Isolated Acceptance and Full Matrix

**Files:**
- Evidence only under the existing server audit/release evidence directory

**Interfaces:**
- Consumes: candidate directory + exact SHA-256
- Produces: domain ⑨ result, full matrix result, final zero-residue JSON and production invariants

- [x] **Step 1: Run isolated domain ⑨**

Start a collected `Type=oneshot` transient unit with:

```text
BIZTEST_CANDIDATE_DIR=<isolated candidate app>
BIZTEST_CANDIDATE_SHA256=<exact ELF hash>
BIZTEST_TARGET_MODULES=batch_a_domain9
```

Expected assertions:

- exact 8-year and 10-year projection candidates;
- automatic consolidation tasks reach `consolidated`;
- memory version advances twice;
- 10-year fact is live and 8-year fact is not live;
- exact completion/conflict events are non-duplicated;
- cleanup succeeds with all final open counts zero.

- [x] **Step 2: Diagnose any remaining failure from frozen evidence**

Use the run/review/LLM/event/candidate/task/outbox failure snapshot. Change code only for a proven product or assertion-contract defect; do not weaken semantic expectations.

- [ ] **Step 3: Run the complete isolated business matrix**
  - 已跑严格 `BIZTEST_SUITE_MODE=full`（full-4，候选 `b2eccd…`）。
  - 域 1–6 与 campaign 通过；其余 LLM 域被 DeepSeek 402 挡住。
  - 2026-08-14 06:12 对 active DeepSeek 1-token 探活仍 402，未再复跑。
  - 裁定：`docs/superpowers/reports/2026-08-14-isolated-biztest-acceptance-ruling.md`

Start the same runner with:

```text
BIZTEST_SUITE_MODE=full
BIZTEST_CANDIDATE_DIR=<isolated candidate app>
BIZTEST_CANDIDATE_SHA256=<exact ELF hash>
```

Expected: cleanup and preflight pass; every runnable domain passes; unavailable external capability is recorded only through the established BLOCKED ledger.

- [x] **Step 4: Verify zero residue and production health**
  - full-4 `final.json` 全部 open 计数为 0。
  - 正式 PID `1020101`、NRestarts=0、健康 200；正式 ELF 仍为 `9472129e…`。

Require final facts:

```json
{
  "bizContacts": 0,
  "bizProfiles": 0,
  "bizKnowledge": 0,
  "bizManagement": 0,
  "activeBizProfiles": 0,
  "externalMcpConfig": 0,
  "externalReferralCards": 0,
  "open": [0, 0, 0, 0, 0]
}
```

Also require unchanged production PID, restart count and executable hash, plus a successful health response.

- [x] **Step 5: Final handoff**
  - 见 `docs/superpowers/reports/2026-08-14-isolated-biztest-acceptance-ruling.md`。
  - 未切正式、未创建 commit。全矩阵全绿仍被 402 阻塞。

Report the candidate hash, exact commands/tests, domain/full-matrix outcomes, blocked capabilities, residue audit and production invariants. Do not switch production or commit unless separately requested.
