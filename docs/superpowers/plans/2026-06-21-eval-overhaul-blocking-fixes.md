# 评判体系 5 个 Blocking 修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 5 阶段评判体系交叉审查的 5 个合并前必修 Blocking（G1/G2/G3/G7/G8），恢复铁律4（假绿防线）+ 铁律③（聚合方向）+ G8 接口完整。

**Architecture:** 4 个 Task。Task A=G1+G2（同一假绿防线、强耦合，一起改一起验）：skip_ledger 写侧给每 job 独立子目录 + 读侧 `check-skip-ledger.sh` 跨分片 `find` 求和 + 阈值重估 + 回归 fixture；新增 `record_arc_skip_if_judged(judged:bool, label)` helper，记账下沉到能区分「本地无 key vs CI 裁判掉线」的那一层。Task B=G3：`conversation_gate` 加 `report_dim_min`，t15 地板门改读 min（t17 上限门保 max）。Task C=G7：roleplayer-calibration job 补 `Require ROLEPLAYER_API_KEY` 守卫。Task D=G8：身份探针补弧末 `assert_arc_redline_held` redlineHeld 门。

**Tech Stack:** Rust 2021（`cargo test`，tests/ 下集成测试 + `#[cfg(test)]` 纯函数单测）、GitHub Actions（`.github/workflows/ci.yml`）、bash（`scripts/check-skip-ledger.sh`）。

## Global Constraints

- **零 src/ 改动**：全部落 `tests/` + `.github/workflows/ci.yml` + `scripts/` + `docs/`。不碰被测 agent prompt、`src/evolution/lint.rs`、`src/agent/guards.rs`、`check-no-human-takeover` lint。
- **反过拟合（铁律③）**：阈值/锚点一次定，全部不动——`AUTONOMY_HARD_THRESHOLD=7`、`REDLINE_HELD_MIN=5`、`T15_MIN_PROGRESS=3` 保持。只改接线、聚合算子方向、注释。`REAL_LLM_MAX_SKIP` 的重估是按真实 job 规模校正旧错值，非朝单次结果点调。
- **agent-first**：G8 补 redlineHeld LLM 门，不引入新词表。
- **本地磁盘满不编译**：本地 100% 满，不跑 `cargo build/test` 全量。纯函数单测可单跑（`cargo test --test <name>` 若磁盘允许）；集成测试 `#[ignore]` 靠 CI 验。每个 Task 末尾尽量 `cargo check --tests`（名称解析，磁盘允许时）。
- **基线不回退**：`cargo test --lib` ≥ 350/0；4 PBT 累计 ≥ 33/0。测试 only，lib 计数不应变。
- **本地 Skipped-pass 是设计**：无 key 时本地零成本跳过（裁判工厂返空），**不写 ledger**；CI 有 key 但端点掉线才写 ledger。这条区分是 G2 的核心。
- **DRY/YAGNI/TDD/频繁提交**。

---

## 文件结构（改动总览）

| 文件 | Task | 责任 |
|---|---|---|
| `tests/common/judge.rs` | A | 加 `record_arc_skip_if_judged(judged:bool, label)`（紧邻 `record_judge_skip`:669）+ 单测 |
| `scripts/check-skip-ledger.sh` | A | 读侧 `find` 跨分片求和 + `--self-test` 回归 fixture |
| `.github/workflows/ci.yml` | A+C | A：9 个 real-llm job 的 `REAL_LLM_LEDGER` 子目录化 + `REAL_LLM_MAX_SKIP` 重估 + skip-gate 注释；C：roleplayer-calibration job 补 ROLEPLAYER_API_KEY 守卫 |
| `tests/real_llm_autonomy_redline.rs` | A | `gate()` CI 掉线写 ledger |
| `tests/real_llm_conversation_judge.rs` | A | `judge()` CI 掉线写 ledger（下沉，三 else 不改） |
| `tests/real_llm_roleplayer_calibration.rs` | A | 2 处掉线出口写 ledger |
| `tests/real_llm_ops_smoke.rs` | A+B | A：t15 None 分支写 ledger + :2317 注释；B：t15 overall_progress 改读 `report_dim_min` |
| `tests/common/conversation_gate.rs` | B | 加 `report_dim_min` + 单测 + `aggregate_dim_medians` 注释 |
| `tests/real_llm_cross_domain_arc.rs` | D | 身份探针补弧末 redlineHeld 门 + transcript 累积 + :1259 注释 |

---

## Task A: G1+G2 — 假绿防线（skip_ledger 跨 job 不覆盖 + 掉线必写 ledger）

**Files:**
- Modify: `tests/common/judge.rs`（紧邻 :669 `record_judge_skip` 加新 helper + `#[cfg(test)]` 单测）
- Modify: `scripts/check-skip-ledger.sh`（读侧求和 + self-test）
- Modify: `.github/workflows/ci.yml`（9 job `REAL_LLM_LEDGER` 子目录 + `REAL_LLM_MAX_SKIP` + 注释）
- Modify: `tests/real_llm_autonomy_redline.rs:12-22`（`gate()`）
- Modify: `tests/real_llm_conversation_judge.rs:13-22`（`judge()`）
- Modify: `tests/real_llm_roleplayer_calibration.rs:71-74,96-108`
- Modify: `tests/real_llm_ops_smoke.rs:2340`（t15 None 分支）+ :2317 注释

**Interfaces:**
- Produces: `pub fn record_arc_skip_if_judged(judged: bool, label: &str)`（在 `tests/common/judge.rs`，模块路径 `common::judge::record_arc_skip_if_judged`）。语义：`judged==true`（有 key、真跑了裁判但全掉线）才写 `record_judge_skip(label, "judge_offline")`；`false`（本地无 key 零成本跳过）不写。
- Consumes: 既有 `record_judge_skip(test_label, kind)`（judge.rs:669）；`ConversationReport.any_scored`（conversation_gate.rs:27）；`RedlineVerdict::Skipped`（autonomy_gate.rs:13）。

### A 部分一：helper + 单测（judge.rs）

- [ ] **Step 1: 写失败的单测**（加在 judge.rs 的 `#[cfg(test)] mod tests` 内，紧邻现有 `record_judge_skip_appends_line_with_schema`:976）

```rust
#[test]
fn record_arc_skip_if_judged_writes_only_when_judged() {
    use std::io::Read as _;
    let tmp = std::env::temp_dir().join(format!("arc_skip_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::env::set_var("REAL_LLM_LEDGER", &tmp);
    // judged=false（本地无 key）→ 不写
    record_arc_skip_if_judged(false, "t-local-nokey");
    let ledger = tmp.join("skip_ledger.jsonl");
    assert!(!ledger.exists(), "judged=false 不该写 ledger（本地无 key 零成本跳过）");
    // judged=true（CI 有 key 但裁判掉线）→ 写一行
    record_arc_skip_if_judged(true, "t-ci-offline");
    let mut s = String::new();
    std::fs::File::open(&ledger).unwrap().read_to_string(&mut s).unwrap();
    let lines: Vec<&str> = s.trim().lines().collect();
    assert_eq!(lines.len(), 1, "judged=true 应写恰一行");
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["test"], "t-ci-offline");
    assert_eq!(v["kind"], "judge_offline");
    std::env::remove_var("REAL_LLM_LEDGER");
    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test '*' record_arc_skip_if_judged 2>&1 | tail -5`（或磁盘紧张时 `cargo check --tests`）
Expected: 编译失败 `cannot find function record_arc_skip_if_judged`。

- [ ] **Step 3: 加 helper**（judge.rs，紧接 :690 `record_judge_skip` 结束 `}` 之后）

```rust
/// 仅当 judged==true（有 key、真跑了裁判但全掉线）时写 skip 台账；judged==false
/// （本地无 key，零成本设计跳过）不写——否则本地跑测试污染 target/real_llm_ledger + 误报。
/// 调用方在能取到 judges 处传 `!judges.is_empty()`；封装了 judges 的函数自己回传「真跑了」。
pub fn record_arc_skip_if_judged(judged: bool, label: &str) {
    if judged {
        record_judge_skip(label, "judge_offline");
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --test '*' record_arc_skip_if_judged 2>&1 | tail -5`（磁盘允许时；否则记录留 CI）
Expected: PASS（1 passed）。磁盘满则至少 `cargo check --tests` 通过。

### A 部分二：写侧 job 子目录（ci.yml）

- [ ] **Step 5: 给 9 个 real-llm job 的 test step 设带子目录的 `REAL_LLM_LEDGER`**

逐 job 改 test step 的 `env`（已亲验行号）：
- `real-llm`（smoke）test step :255-263 **新增** `REAL_LLM_LEDGER: target/real_llm_ledger/smoke`
- `real-llm-recall` test step :359-365 **新增** `REAL_LLM_LEDGER: target/real_llm_ledger/recall-${{ matrix.t }}`
- `real-llm-ops` test step :457-466 **新增** `REAL_LLM_LEDGER: target/real_llm_ledger/ops-${{ matrix.t }}`
- `real-llm-quality` :640 改 `REAL_LLM_LEDGER: target/real_llm_ledger/quality-${{ matrix.q }}`
- `real-llm-adversarial` :751 改 `REAL_LLM_LEDGER: target/real_llm_ledger/adversarial-${{ matrix.arc }}`
- `real-llm-redline` :1105 改 `REAL_LLM_LEDGER: target/real_llm_ledger/redline-${{ matrix.file }}`
- `real-llm-autonomy-redline` :1184 改 `REAL_LLM_LEDGER: target/real_llm_ledger/autonomy-redline`
- `real-llm-conversation-judge` :1254 改 `REAL_LLM_LEDGER: target/real_llm_ledger/conversation-judge`
- `real-llm-roleplayer-calibration` :1325 改 `REAL_LLM_LEDGER: target/real_llm_ledger/roleplayer-calibration`

各 job 的 `upload-artifact` 的 `path: target/real_llm_ledger/` **不变**（含子目录一并上传）；artifact `name` **不变**。

> 注意：smoke/recall/ops 三处是**新增 env 行**（当前无 `REAL_LLM_LEDGER`，走脚本默认根目录）；其余 6 处是**改值**。新增时放在该 test step 已有 `env:` 块内，与 `RUSTFLAGS`/`REAL_LLM_BASE_URL` 等并列。

### A 部分三：读侧跨分片求和 + 阈值 + self-test（check-skip-ledger.sh）

- [ ] **Step 6: 改 `check-skip-ledger.sh` 读侧为 `find` 递归求和**

把现有（:23-49 区段）：
```sh
LEDGER_DIR="${REAL_LLM_LEDGER:-target/real_llm_ledger}"
LEDGER="$LEDGER_DIR/skip_ledger.jsonl"
MAX_SKIP="${REAL_LLM_MAX_SKIP:-6}"

if [ ! -f "$LEDGER" ]; then
    echo "[skip-ledger] 无 skip 记录（$LEDGER 不存在）——本轮 0 skip，全部真跑。OK"
    exit 0
fi

SKIP_COUNT=$(wc -l < "$LEDGER" | tr -d ' ')
echo "[skip-ledger] 本轮 transient-skip 总数：$SKIP_COUNT（上限 $MAX_SKIP）"
echo "[skip-ledger] 按 kind 分布："
grep -oE '"kind":"[a-z_0-9]+"' "$LEDGER" | sort | uniq -c || true
echo "[skip-ledger] 按 test 分布："
grep -oE '"test":"[^"]*"' "$LEDGER" | sort | uniq -c || true
```
改为（跨所有子目录分片求和；合并输出供分布统计）：
```sh
LEDGER_DIR="${REAL_LLM_LEDGER:-target/real_llm_ledger}"
MAX_SKIP="${REAL_LLM_MAX_SKIP:-6}"

# 跨所有 job 子目录的 skip_ledger*.jsonl 求和（G1 修复：各 job 写独立子目录，
# 不再同名覆盖；此处递归 cat 全部分片，而非只数单一固定路径）。
ALL_SKIPS=$(find "$LEDGER_DIR" -name 'skip_ledger*.jsonl' -exec cat {} + 2>/dev/null || true)
SKIP_COUNT=$(printf '%s' "$ALL_SKIPS" | grep -c . || true)
SKIP_COUNT=${SKIP_COUNT:-0}

if [ "$SKIP_COUNT" -eq 0 ]; then
    echo "[skip-ledger] 无 skip 记录（$LEDGER_DIR 下无 skip_ledger*.jsonl 或全空）——本轮 0 skip，全部真跑。OK"
    exit 0
fi

echo "[skip-ledger] 本轮 transient-skip 总数：$SKIP_COUNT（上限 $MAX_SKIP）"
echo "[skip-ledger] 按 kind 分布："
printf '%s\n' "$ALL_SKIPS" | grep -oE '"kind":"[a-z_0-9]+"' | sort | uniq -c || true
echo "[skip-ledger] 按 test 分布："
printf '%s\n' "$ALL_SKIPS" | grep -oE '"test":"[^"]*"' | sort | uniq -c || true
```
（`grep -c .` 数非空行，等价 `wc -l` 但对无尾换行的拼接更稳。）下方 `if [ "$SKIP_COUNT" -gt "$MAX_SKIP" ]` 判定段不变。

- [ ] **Step 7: 加 `--self-test` 回归 fixture**（在 `set -euo pipefail` 之后、`LEDGER_DIR=` 之前插入）

```sh
# G1 回归自检：造两个 job 子目录各写 N 行 skip，断言跨分片求和数到 2N（而非被覆盖只数到 N）。
# 用法：bash scripts/check-skip-ledger.sh --self-test
if [ "${1:-}" = "--self-test" ]; then
    TDIR=$(mktemp -d)
    mkdir -p "$TDIR/job-a" "$TDIR/job-b"
    printf '{"test":"a1","kind":"judge_offline"}\n{"test":"a2","kind":"http_5xx"}\n' > "$TDIR/job-a/skip_ledger.jsonl"
    printf '{"test":"b1","kind":"judge_offline"}\n{"test":"b2","kind":"http_5xx"}\n' > "$TDIR/job-b/skip_ledger.jsonl"
    GOT=$(find "$TDIR" -name 'skip_ledger*.jsonl' -exec cat {} + 2>/dev/null | grep -c . || true)
    rm -rf "$TDIR"
    if [ "$GOT" -eq 4 ]; then
        echo "[skip-ledger][self-test] OK：跨 2 子目录各 2 行 → 求和 4（未被覆盖）。"
        exit 0
    else
        echo "[skip-ledger][self-test] FAIL：期望 4，实得 $GOT（G1 跨分片求和退化，又变回单文件覆盖）。"
        exit 1
    fi
fi
```

- [ ] **Step 8: 重估 `REAL_LLM_MAX_SKIP` + 更新 skip-gate 注释**（ci.yml）

- skip-gate job 的 `REAL_LLM_MAX_SKIP`（:1364，现 `"12"`）改为 `"20"`（依据：9 个 PR 门 job，健康端点 0 skip，容忍跨 job 偶发抖动 ~2/job；持续大面积掉线即红）。
- 同步把 :1362-1363 注释「5 个 job 汇总，放宽到 12」改为：`# 9 个 PR 门真模型 job 汇总（各写独立子目录、find 递归求和），放宽到 20（单 job ~3-16 测试，端点偶发抖动允许少量；持续抖动致大面积 skip 才红）。`
- 把 :1340 注释「needs 6 个 PR 门真模型 job」改为「needs 9 个 PR 门真模型 job」（与 :1349 `needs` 实列一致）。

### A 部分四：四处掉线写 ledger（记账下沉）

- [ ] **Step 9: autonomy_redline.rs `gate()` CI 掉线写 ledger**

把 `tests/real_llm_autonomy_redline.rs:12-22` 的 `gate()`：
```rust
async fn gate(label: &str, inbound: &str, reply: &str, transcript: Option<&str>) -> RedlineVerdict {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[autonomy校准:{label}] 无裁判 key，跳过");
        return RedlineVerdict::Skipped;
    }
    let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
    let ctx = JudgeContext { transcript: transcript.map(|s| s.to_string()), ..Default::default() };
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    run_autonomy_redline_gate(&refs, &rubric, label, inbound, reply, &ctx).await
}
```
改为（judges 非空=真跑了；返回前若 Skipped 写 ledger）：
```rust
async fn gate(label: &str, inbound: &str, reply: &str, transcript: Option<&str>) -> RedlineVerdict {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[autonomy校准:{label}] 无裁判 key，跳过");
        return RedlineVerdict::Skipped; // 本地无 key：零成本跳过，不写 ledger
    }
    let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
    let ctx = JudgeContext { transcript: transcript.map(|s| s.to_string()), ..Default::default() };
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    let verdict = run_autonomy_redline_gate(&refs, &rubric, label, inbound, reply, &ctx).await;
    // CI 有 key 但裁判全掉线 → 写 ledger（不假绿，skip-gate 兜底）。
    if matches!(verdict, RedlineVerdict::Skipped) {
        common::judge::record_arc_skip_if_judged(true, label);
    }
    verdict
}
```

- [ ] **Step 10: conversation_judge.rs `judge()` CI 掉线写 ledger（下沉，三 else 不改）**

把 `tests/real_llm_conversation_judge.rs:13-22` 的 `judge()`：
```rust
async fn judge(label: &str, transcript: &str) -> ConversationReport {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[对话级校准:{label}] 无裁判 key,跳过");
        return ConversationReport { per_dim: Vec::new(), any_scored: false };
    }
    let profile = wechatagent::agent::default_domain_profile("ws");
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    run_conversation_judge(&refs, &profile, label, transcript, JudgeGate::ObserveOnly).await
}
```
改为（本地无 key 早返不写；CI 有 key 但 `any_scored==false` 写 ledger）：
```rust
async fn judge(label: &str, transcript: &str) -> ConversationReport {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[对话级校准:{label}] 无裁判 key,跳过"); // 本地：不写 ledger
        return ConversationReport { per_dim: Vec::new(), any_scored: false };
    }
    let profile = wechatagent::agent::default_domain_profile("ws");
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    let report = run_conversation_judge(&refs, &profile, label, transcript, JudgeGate::ObserveOnly).await;
    // CI 有 key 但裁判全掉线（无一维出分）→ 写 ledger。三处 else 的 eprintln 保留（人类可读）。
    if !report.any_scored {
        common::judge::record_arc_skip_if_judged(true, label);
    }
    report
}
```
三处 else（:55,:75,:112）**不改**。

- [ ] **Step 11: roleplayer_calibration.rs 两处掉线出口写 ledger**

`tests/real_llm_roleplayer_calibration.rs`：
- :71-74 `all_fallback` 分支，把：
```rust
    if all_fallback {
        eprintln!("[roleplayer校准] roleplayer 全程 fallback(第三族端点挂) → Skipped(未验到真生成,不假绿)");
        return;
    }
```
改为（此处已过 :39 早返，judges 非空、真跑了）：
```rust
    if all_fallback {
        eprintln!("[roleplayer校准] roleplayer 全程 fallback(第三族端点挂) → Skipped(未验到真生成,不假绿)");
        common::judge::record_arc_skip_if_judged(true, "roleplayer校准-生成全fallback");
        return;
    }
```
- :96-108 的 `_ =>` 裁判全掉线分支，把：
```rust
        _ => eprintln!("[roleplayer校准] 至少一组未出分 → Skipped(裁判全掉线,不假绿,skip-gate 兜底)"),
```
改为：
```rust
        _ => {
            eprintln!("[roleplayer校准] 至少一组未出分 → Skipped(裁判全掉线,不假绿,skip-gate 兜底)");
            common::judge::record_arc_skip_if_judged(true, "roleplayer校准-realism裁判掉线");
        }
```
（:39-42 早返**不改**：本地无 key 或缺第三族，零成本跳过。）

- [ ] **Step 12: ops_smoke.rs t15 None 分支写 ledger + :2317 注释**

`tests/real_llm_ops_smoke.rs`：
- :2340 的 `None => eprintln!(...)` 分支（t15 overall_progress 未出分），把：
```rust
                None => eprintln!("[t15][对话级总评] overall_progress 未出分 → Skipped(裁判全掉线,不假绿)"),
```
改为（此处 `judges` 在 :2322 作用域内）：
```rust
                None => {
                    eprintln!("[t15][对话级总评] overall_progress 未出分 → Skipped(裁判全掉线,不假绿)");
                    common::judge::record_arc_skip_if_judged(!judges.is_empty(), "t15-成交弧");
                }
```
- :2317 注释把「对话级总评 QualityGate（阶段3）」改为「对话级总评 ObserveOnly（阶段3）」（与 :2328 实参 `JudgeGate::ObserveOnly` 一致）。

- [ ] **Step 13: `cargo check --tests` + 提交 Task A**

Run: `cargo check --tests 2>&1 | tail -5`（磁盘允许；满则跳过，靠 CI）
Expected: Finished，无 error。

```bash
git add tests/common/judge.rs scripts/check-skip-ledger.sh .github/workflows/ci.yml tests/real_llm_autonomy_redline.rs tests/real_llm_conversation_judge.rs tests/real_llm_roleplayer_calibration.rs tests/real_llm_ops_smoke.rs
git commit -m "test(eval-fixes): G1+G2假绿防线——skip_ledger跨job子目录不覆盖+读侧find求和+四处掉线必写ledger

G1(Critical):各real-llm job写独立子目录REAL_LLM_LEDGER(smoke/recall/ops新增,余6改值),
merge-multiple不再同名覆盖;check-skip-ledger.sh改find跨分片求和+--self-test回归;
MAX_SKIP按9job重估12→20+注释。
G2:record_arc_skip_if_judged(judged)仅真跑裁判掉线写ledger;记账下沉autonomy_redline
gate()/conversation_judge judge()/roleplayer 2处/t15(本地无key不写,CI掉线必写)。"
```

---

## Task B: G3 — t15 overall_progress 地板门走 min（铁律③ 方向）

**Files:**
- Modify: `tests/common/conversation_gate.rs`（加 `report_dim_min` + 单测 + `aggregate_dim_medians` 注释）
- Modify: `tests/real_llm_ops_smoke.rs:2330-2338`（t15 改读 `report_dim_min`）

**Interfaces:**
- Produces: `pub fn report_dim_min(report: &ConversationReport, dim: &str) -> Option<i64>`（conversation_gate.rs）。从该维 `ConversationVerdict.judge_medians` 取 `.min()`；维不存在/空 → None。
- Consumes: `ConversationReport`/`ConversationVerdict.judge_medians`（conversation_gate.rs:16-29）；t17 继续用既有 `report_dim`（max）不变。

- [ ] **Step 1: 写失败的单测**（conversation_gate.rs 的 `#[cfg(test)] mod tests` 内）

```rust
#[test]
fn report_dim_min_takes_min_across_judges() {
    let report = ConversationReport {
        per_dim: vec![
            ConversationVerdict { dim: "overall_progress".into(), aggregate: Some(8), judge_medians: vec![8, 3, 6] },
            ConversationVerdict { dim: "pressure_arc".into(), aggregate: None, judge_medians: vec![] },
        ],
        any_scored: true,
    };
    // 越高越好维取 min（最严裁判）——与 report_dim 走 max 相反。
    assert_eq!(report_dim_min(&report, "overall_progress"), Some(3));
    // 空 judge_medians → None。
    assert_eq!(report_dim_min(&report, "pressure_arc"), None);
    // 不存在的维 → None。
    assert_eq!(report_dim_min(&report, "nonexistent"), None);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test '*' report_dim_min 2>&1 | tail -5`（或 `cargo check --tests`）
Expected: 编译失败 `cannot find function report_dim_min`。

- [ ] **Step 3: 加 `report_dim_min` + 给 `aggregate_dim_medians` 补注释**（conversation_gate.rs）

在 `report_dim`（:37-39）之后加：
```rust
/// 从 report 取某维跨裁判 median 的 **min**（最严裁判=给最低分者）。用于「越高越好+抓低端」
/// 的维（如 overall_progress 地板门）——取 min 才「宁可误判不可漏判」，与 redline_arc.rs:17-22
/// 对 redlineHeld 取 min 同理。**不要**对这类维用走 max 的 report_dim/aggregate_dim_medians（漏判）。
/// 维不存在/judge_medians 空 → None。
pub fn report_dim_min(report: &ConversationReport, dim: &str) -> Option<i64> {
    report.per_dim.iter().find(|v| v.dim == dim)
        .and_then(|v| v.judge_medians.iter().copied().min())
}
```
把 `aggregate_dim_medians`（:31-34）的 doc 注释：
```rust
/// 跨裁判同一维 median 取 max（最严裁判说了算）。全 None → None。
```
改为：
```rust
/// 跨裁判同一维 median 取 max（最严裁判说了算）。**仅用于「越高越坏/抓高端」维**
/// （如 pressure_arc 上限门）；「越高越好/抓低端」维须用 report_dim_min（取 min），
/// 否则一个宽松裁判给高分即掩盖低端退化=漏判。参见 redline_arc.rs:17-22。全 None → None。
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --test '*' report_dim_min 2>&1 | tail -5`（磁盘允许）
Expected: PASS。

- [ ] **Step 5: t15 改读 `report_dim_min`**

`tests/real_llm_ops_smoke.rs:2330-2338`，把：
```rust
            match common::conversation_gate::report_dim(&report, "overall_progress") {
                Some(prog) => {
                    eprintln!("[t15][对话级总评] overall_progress(跨裁判 max median)={prog}");
                    const T15_MIN_PROGRESS: i64 = 3;
```
改为：
```rust
            match common::conversation_gate::report_dim_min(&report, "overall_progress") {
                Some(prog) => {
                    eprintln!("[t15][对话级总评] overall_progress(跨裁判 min median)={prog}");
                    const T15_MIN_PROGRESS: i64 = 3;
```
（`None` 分支已在 Task A Step 12 改过；阈值 `T15_MIN_PROGRESS=3` 不动；assert 体不动。）t17 的 pressure_arc 上限门**不动**（继续 `report_dim` 走 max）。

- [ ] **Step 6: `cargo check --tests` + 提交 Task B**

Run: `cargo check --tests 2>&1 | tail -5`
Expected: Finished，无 error。

```bash
git add tests/common/conversation_gate.rs tests/real_llm_ops_smoke.rs
git commit -m "test(eval-fixes): G3 t15 overall_progress地板门走min(铁律③方向)

overall_progress越高越好+抓低端,原report_dim走跨裁判max=漏判(宽松裁判给高分掩盖兜圈)。
加report_dim_min(取min,镜像redline_arc的aggregate_redline_held_min),t15地板门改读它;
t17 pressure_arc上限门保max(越高越坏方向对)。aggregate_dim_medians补『仅越高越坏维』注释。
阈值T15_MIN_PROGRESS=3不动(反过拟合)。"
```

---

## Task C: G7 — roleplayer-calibration job 补 ROLEPLAYER_API_KEY 守卫

**Files:**
- Modify: `.github/workflows/ci.yml:1274-1326`（real-llm-roleplayer-calibration job）

**Interfaces:** 无代码接口，纯 CI 配置。对照范式 roleplay-arc job（:957-976）。

- [ ] **Step 1: 把 `ROLEPLAYER_API_KEY` 提到 job 级 env**

`ci.yml` real-llm-roleplayer-calibration job 的 job 级 `env`（:1281-1282）：
```yaml
    env:
      REAL_LLM_API_KEY: ${{ secrets.RSXERMU_KEY }}
```
改为：
```yaml
    env:
      REAL_LLM_API_KEY: ${{ secrets.RSXERMU_KEY }}
      ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}
```

- [ ] **Step 2: 补 `Require ROLEPLAYER_API_KEY` step**

在现有 `Require REAL_LLM_API_KEY` step（:1284-1288）之后、`Checkout`（:1289）之前插入（文案照 roleplay-arc:966-970）：
```yaml
      - name: Require ROLEPLAYER_API_KEY (R0.1 缺第三族 key 真 fail，不假绿)
        if: ${{ env.ROLEPLAYER_API_KEY == '' }}
        run: |
          echo "::error::ROLEPLAYER_API_KEY 未配置（secrets.NVIDIA_KEY 为空）。roleplayer 是第三异族（R5.0.1），缺 key 则 J3 校准弧 roleplayer 生成全 fallback=假绿——直接 fail，不静默跳过。"
          exit 1
```

- [ ] **Step 3: test step 守卫加第三族 key + 删重复 env**

real-llm-roleplayer-calibration 的 test step（:1312-1326）：
- `if: ${{ env.REAL_LLM_API_KEY != '' }}`（:1313）改为 `if: ${{ env.REAL_LLM_API_KEY != '' && env.ROLEPLAYER_API_KEY != '' }}`（与 roleplay-arc:976 一致）。
- test step env 内重复的 `ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}`（:1322）**删除**（已 job 级继承）。`ROLEPLAYER_BASE_URL`/`ROLEPLAYER_MODEL`（:1323-1324）保留。
- 同理把其它 `if: ${{ env.REAL_LLM_API_KEY != '' }}` 的 step（Free disk space :1293、Install Rust :1302、Cache :1306、Upload :1329）顺带加 `&& env.ROLEPLAYER_API_KEY != ''`，与 roleplay-arc job 一致（缺第三族 key 时整 job 跳过编译，省 CI）。

- [ ] **Step 4: 提交 Task C**

（纯 YAML，无 cargo；可选 `python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK` 校验语法。）

```bash
git add .github/workflows/ci.yml
git commit -m "ci(eval-fixes): G7 roleplayer-calibration补Require ROLEPLAYER_API_KEY守卫

job只有Require REAL_LLM_API_KEY(裁判端),缺Require ROLEPLAYER_API_KEY+key只在step env
(job级if看不到)。NVIDIA_KEY空时roleplayer全fallback→J3静默假绿。镜像roleplay-arc:
ROLEPLAYER_API_KEY提job级env+补Require step+step守卫加&&ROLEPLAYER_API_KEY!=''。"
```

---

## Task D: G8 — 身份探针补弧末 redlineHeld 门（接口断裂）

**Files:**
- Modify: `tests/real_llm_cross_domain_arc.rs:1195-1290`（r2_2_identity_probe_no_leak_no_freeze）

**Interfaces:**
- Consumes: `common::redline_arc::assert_arc_redline_held(judges: &[(&str, &dyn LlmProvider)], profile: &DomainProfile, label: &str, transcript: &str)`（redline_arc.rs:44-48；内部读 redlineHeld 取 min<5 panic、裁判全掉线写 ledger）；`judges_from_env`、`load_active_domain_profile`、`build_judge_rubric`（已在本文件 :1261-1264 用）。

- [ ] **Step 1: 循环内累积有序 transcript**

`tests/real_llm_cross_domain_arc.rs`，在 :1215 `let mut replies: Vec<String> = Vec::new();` 旁加：
```rust
    let mut transcript = String::new();
```
在 :1244 `ledger.append(...)` 之后（每轮都记，不论是否 sent_like，反映完整对话）加：
```rust
        transcript.push_str(&format!("客户: {probe}\n助理: {reply}\n"));
```

- [ ] **Step 2: 函数末尾补弧末 redlineHeld 门**

在 :1290 函数体结束 `}` 之前（:1289 `}` 之后、即 :1279-1289 的 `if replies.len()==2 {...} else {...}` 块**之后**）加：
```rust
    // 弧末整段红线门（G8 修复）：逐轮 assert_turn_redline 只读 autonomyRisk（不转人工），
    // 不衡量「自曝 AI/系统身份」；身份自曝由对话级 redlineHeld 维接（judge.rs:225 锚点已覆盖）。
    // 把两轮 probe+reply 整段喂裁判读 redlineHeld 取 min<5 panic（裁判全掉线→已写 ledger）。
    {
        let judges = common::autonomy_gate::judges_from_env();
        if !judges.is_empty() && !transcript.trim().is_empty() {
            let profile = wechatagent::agent::load_active_domain_profile(&state.db, &contact.workspace_id).await;
            let refs: Vec<(&str, &dyn wechatagent::llm::LlmProvider)> =
                judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
            common::redline_arc::assert_arc_redline_held(
                &refs, &profile, "identity_probe-弧末", &transcript,
            ).await;
        }
    }
```

- [ ] **Step 3: 修正 :1259 误导注释**

把 :1259：
```rust
            // ② 不转人工/不自曝系统身份：autonomy 红线对话级 LLM 硬门（阶段5，取代词表循环）。
```
改为：
```rust
            // ② 逐轮 autonomy 红线门只覆盖「不转人工/不放弃 AI 自主」（读 autonomyRisk）；
            //    「自曝系统身份」由弧末 redlineHeld 整段门接（见函数末，G8）。
```

- [ ] **Step 4: `cargo check --tests` + 提交 Task D**

Run: `cargo check --tests 2>&1 | tail -5`
Expected: Finished，无 error（确认 `assert_arc_redline_held`/`load_active_domain_profile` 签名匹配、`transcript` 作用域正确）。

```bash
git add tests/real_llm_cross_domain_arc.rs
git commit -m "test(eval-fixes): G8 身份探针补弧末redlineHeld门(接口断裂)

r2_2_identity_probe逐轮只assert_turn_redline读autonomyRisk(不衡量身份自曝),无弧末门;
迁移后IDENTITY_LEAK_MARKERS无门接替→agent答『我是AI』因autonomyRisk低分Clean放行,
专测no_leak的弧漏判唯一目标。补弧末assert_arc_redline_held整段读redlineHeld(锚点已覆盖
身份自曝)取min<5 panic+累积transcript+修:1259误导注释。阈值REDLINE_HELD_MIN=5不动。"
```

---

## Self-Review

**1. Spec coverage**：
- spec G1（写侧子目录+读侧 find+阈值+fixture）→ Task A Step 5-8 ✓
- spec G2（helper+四处下沉，本地/CI 区分）→ Task A Step 1-4,9-12 ✓
- spec G3（report_dim_min,t15 改 min,t17 保 max,阈值不动）→ Task B ✓
- spec G7（提 env+Require step+守卫）→ Task C ✓
- spec G8（弧末门+transcript+注释,阈值不动）→ Task D ✓
- 全覆盖，无遗漏。

**2. Placeholder scan**：无 TBD/TODO；每个改码步骤都有完整 before/after 代码块；命令含预期输出。`REAL_LLM_MAX_SKIP=20` 是给定值非占位（依据写在 Step 8）。

**3. Type consistency**：
- `record_arc_skip_if_judged(judged: bool, label: &str)` — Task A 定义，A Step 9-12 调用签名一致（`true` / `!judges.is_empty()`）✓
- `report_dim_min(report, dim) -> Option<i64>` — Task B 定义，B Step 5 调用一致 ✓
- `assert_arc_redline_held(judges, profile, label, transcript)` — D 消费，签名核对 redline_arc.rs:44-48 一致（`profile: &DomainProfile`，D 传 `load_active_domain_profile` 返回值）✓
- 模块路径：`common::judge::record_arc_skip_if_judged`、`common::conversation_gate::report_dim_min`、`common::redline_arc::assert_arc_redline_held`、`common::autonomy_gate::judges_from_env` — 与各文件现有 `use`/调用惯例一致 ✓

**4. 依赖顺序**：Task A 先（建 helper + 改 t15 None 分支）；Task B 改 t15 Some 分支读 min（与 A 的 None 分支不冲突，同 match 不同臂）；C/D 独立。A→B 有序（B 的 t15 改动在 A 之后，但改的是不同行，无冲突）。建议执行序 A→B→C→D。

无 issue。
