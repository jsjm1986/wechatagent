# 批E家族① auto_release 方向一致性 + threshold 重判口径对齐 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 evolution auto_release 两条方向/口径漏洞：KE-01 放量方向一致性（避免命中率跨 band 翻转时反向放量）+ KE-02 threshold 重判 original/new 口径对齐（避免非-5gate 终态虚假抬高 send_delta）。

**Architecture:** 两条独立纯逻辑修复，全在 `src/evolution/` 隔离模块内。KE-01 给 `decide_auto_release` 加候选方向参数（升阈候选仅命中率仍过高时放行、降阈候选仅仍过低时放行），是旧逻辑的安全收窄。KE-02 把 threshold 重判路径的 `original_final_review_status` 从"源 run 真实终态"改为"基于已算好的 original_5gate_hit 的 5闸重推"，与 prompt 路径对齐。

**Tech Stack:** Rust 2021，纯函数单测（lib，本地可跑），无 Docker、无新依赖。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-12-ke-family1-auto-release-direction-consistency-design.md`（已获批 commit 444f5e4）。所有行号亲验于 origin/main 2ae5ac2。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为 / 签名变更被迫更新"，绝不为过测试改业务逻辑。
- 不动 auto_release 双闸 `auto_release_gate_open` / 负反应门 `decide_negative_reaction_block` / `compute_window_gate_hit_rates`（口径正确）。不动 prompt 路径 `prompt_sample_to_outcome`（已对称）。不动 `final_status_from_5gate` 本身。
- 不破 #152 反向安全门 `grade_safety_regression`（significance.rs:98-125，依赖 `original_final_review_status==block_status`；改用 5闸重推后 `final_status_from_5gate` 正好产出那些 block 态、门更一致）。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不回退。改动不触 baseline 门 4 PBT（state_transition/memory_card/wiki_chunk_revision/llm_retry_jitter）。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。绝不动任何 sibling worktree 的 target/。

---

## File Structure

- `src/evolution/auto_release.rs`：**Modify** `decide_auto_release`（:207-212）加方向门 + 调用点（:154）传方向 + 更新 3 处既有单测（:381-405）+ 新增方向门专项单测。KE-01 全在此文件。
- `src/evolution/replay.rs`：**Modify** `evaluate_threshold`（:296）original 终态改用 5闸重推 + 新增 1 个"非-5gate 源终态"专项单测。KE-02 全在此文件。

两文件互不依赖，但 KE-01 与 KE-02 是独立关注点，各自一个 task 便于独立 review。

---

## Task 1: KE-01 —— decide_auto_release 加候选方向门（auto_release.rs）

**Files:**
- Modify: `src/evolution/auto_release.rs:207-212`（`decide_auto_release` 签名 + 方向门）
- Modify: `src/evolution/auto_release.rs:154`（调用点传 proposal.current_value/proposed_value）
- Modify: `src/evolution/auto_release.rs:381-405`（既有 3 单测补方向参）

**Interfaces:**
- Consumes: `crate::models::Proposal`（字段 `current_value: Option<f64>` / `proposed_value: Option<f64>`，已亲验 replay.rs:250/275 使用）。
- Produces: `pub fn decide_auto_release(observed: Option<f64>, target_lower: f64, target_upper: f64, current_value: Option<f64>, proposed_value: Option<f64>) -> bool`。

- [ ] **Step 1: 先改既有 3 单测到新签名 + 新增方向门专项单测（先写，验证会编译失败）**

把 `src/evolution/auto_release.rs` 的 `decide_auto_release_inside_band_skips` / `decide_auto_release_below_lower_releases` / `decide_auto_release_above_upper_releases` / `decide_auto_release_no_signal_skips`（:381-405）整体替换为下面这组（既有 4 测补方向参使语义明确 + 新增 7 个方向门断言）：

```rust
    #[test]
    fn decide_auto_release_inside_band_skips() {
        // 命中率回到正常区间 → 留给 admin，不自动 release（无论方向）。
        // 升阈候选(6→7)：band 内一律 SKIP。
        assert!(!decide_auto_release(Some(0.10), 0.05, 0.15, Some(6.0), Some(7.0)));
        assert!(!decide_auto_release(Some(0.05), 0.05, 0.15, Some(6.0), Some(7.0)));
        assert!(!decide_auto_release(Some(0.15), 0.05, 0.15, Some(6.0), Some(7.0)));
    }

    #[test]
    fn decide_auto_release_no_signal_skips() {
        // 窗口内无样本：保守拒释放（方向齐备也不放行）。
        assert!(!decide_auto_release(None, 0.05, 0.15, Some(6.0), Some(7.0)));
    }

    // ── KE-01 方向门：升阈候选(proposed>current)仅命中率仍过高(>upper)才放行 ──

    #[test]
    fn decide_auto_release_raise_threshold_releases_only_when_still_above_upper() {
        // 升阈候选(6→7)：命中率仍 > upper（仍过高、需继续降）→ RELEASE。
        assert!(decide_auto_release(Some(0.50), 0.05, 0.15, Some(6.0), Some(7.0)));
    }

    #[test]
    fn decide_auto_release_raise_threshold_skips_when_flipped_below_lower() {
        // KE-01 核心修复：升阈候选(6→7)，但命中率已翻转到 < lower（已过低）→ SKIP。
        // 旧逻辑 rate<lower 也放行 = 反向放量把命中率推更低；本测锁死修复（回退即红）。
        assert!(!decide_auto_release(Some(0.02), 0.05, 0.15, Some(6.0), Some(7.0)));
    }

    // ── KE-01 方向门：降阈候选(proposed<current)仅命中率仍过低(<lower)才放行 ──

    #[test]
    fn decide_auto_release_lower_threshold_releases_only_when_still_below_lower() {
        // 降阈候选(6→5)：命中率仍 < lower（仍过低、需继续升）→ RELEASE。
        assert!(decide_auto_release(Some(0.02), 0.05, 0.15, Some(6.0), Some(5.0)));
    }

    #[test]
    fn decide_auto_release_lower_threshold_skips_when_flipped_above_upper() {
        // 降阈候选(6→5)，但命中率已翻转到 > upper（已过高）→ SKIP（反向放量防护）。
        assert!(!decide_auto_release(Some(0.50), 0.05, 0.15, Some(6.0), Some(5.0)));
    }

    #[test]
    fn decide_auto_release_no_direction_skips() {
        // proposed==current（无方向变化）→ SKIP。
        assert!(!decide_auto_release(Some(0.50), 0.05, 0.15, Some(6.0), Some(6.0)));
    }

    #[test]
    fn decide_auto_release_missing_value_skips() {
        // current/proposed 任一缺失（无法定方向）→ 保守 SKIP。
        assert!(!decide_auto_release(Some(0.50), 0.05, 0.15, None, Some(7.0)));
        assert!(!decide_auto_release(Some(0.50), 0.05, 0.15, Some(6.0), None));
    }
```

（注：删掉旧的 `decide_auto_release_below_lower_releases` / `decide_auto_release_above_upper_releases`——它们断言"band 外任意一侧都放行"，正是 KE-01 有意废除的旧行为；新测按方向拆分覆盖。）

- [ ] **Step 2: 运行确认编译失败**

Run: `cargo test --lib decide_auto_release 2>&1 | tail -20`
Expected: 编译错误 E0061（`decide_auto_release` 旧签名只收 3 参，新测传 5 参）。

- [ ] **Step 3: 改 decide_auto_release 签名 + 方向门**

把 `src/evolution/auto_release.rs:204-212` 的函数：

```rust
/// 纯函数版本：观察到的窗口命中率落在区间外 → 释放（true），落在区间内 → 跳过
/// 留给 admin（false）。`observed=None`（窗口内无样本）也保守返回 false ——
/// 没有信号不能盲目释放。
pub fn decide_auto_release(observed: Option<f64>, target_lower: f64, target_upper: f64) -> bool {
    match observed {
        None => false,
        Some(rate) => rate < target_lower || rate > target_upper,
    }
}
```

替换为：

```rust
/// 纯函数版本：命中率仍在 band 外**且偏离方向与候选修正方向一致**时释放（true）。
///
/// KE-01：旧实现只判 `rate<lower || rate>upper`（band 外任意一侧），不看候选方向，
/// 与模块 doc「方向与候选方向一致才放行」相悖。命中率跨 band 翻转到相反外侧时会
/// 反向放量（升阈候选在命中率已过低时仍放行、继续把命中率推更低）。
///
/// 方向由 `proposed_value - current_value` 符号表达：
/// - **升阈候选**（proposed>current，阈值调高→命中率将下降）：仅 `rate>upper`（仍过高）放行；
/// - **降阈候选**（proposed<current，阈值调低→命中率将上升）：仅 `rate<lower`（仍过低）放行；
/// - proposed==current（无方向）/ current 或 proposed 缺失 / `observed=None`：保守 SKIP。
///
/// 这是旧逻辑的**安全收窄**：只减少误放行、绝不新增放行。
pub fn decide_auto_release(
    observed: Option<f64>,
    target_lower: f64,
    target_upper: f64,
    current_value: Option<f64>,
    proposed_value: Option<f64>,
) -> bool {
    let Some(rate) = observed else {
        return false; // 无信号不盲动
    };
    let (Some(cur), Some(prop)) = (current_value, proposed_value) else {
        return false; // 缺方向不盲动
    };
    if prop > cur {
        rate > target_upper // 升阈候选：仅命中率仍过高才放行
    } else if prop < cur {
        rate < target_lower // 降阈候选：仅命中率仍过低才放行
    } else {
        false // 无方向变化
    }
}
```

- [ ] **Step 4: 改调用点传方向**

把 `src/evolution/auto_release.rs:154`：

```rust
        let decision = decide_auto_release(observed, lower, upper);
```

替换为：

```rust
        let decision = decide_auto_release(
            observed,
            lower,
            upper,
            proposal.current_value,
            proposal.proposed_value,
        );
```

（`proposal` 是循环变量，:121 `for proposal in proposals`；`current_value`/`proposed_value` 是 `Option<f64>` 字段，直接传。）

- [ ] **Step 5: 运行确认单测通过**

Run: `cargo test --lib decide_auto_release 2>&1 | tail -30`
Expected: 全部 PASS（含新增 7 个方向门断言 + inside_band/no_signal）。

- [ ] **Step 6: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 7: Commit**

```bash
git add src/evolution/auto_release.rs
git commit -m "fix(evolution): auto_release 加候选方向门,防命中率翻转时反向放量 (KE-01)"
```

---

## Task 2: KE-02 —— threshold 重判 original 终态改用 5闸重推（replay.rs）

**Files:**
- Modify: `src/evolution/replay.rs:296`（`evaluate_threshold` 的 `original_final_review_status`）
- Test: `src/evolution/replay.rs`（新增 1 个"非-5gate 源终态"专项单测）

**Interfaces:**
- Consumes: `final_status_from_5gate(&Document) -> &'static str`（replay.rs:406，已存在）；`original_5gate_hit`（evaluate_threshold 内 :265/279 已算好的本地变量）。
- Produces: 无对外接口变化（只改 `ReplayOutcome.original_final_review_status` 的取值口径）。

- [ ] **Step 1: 新增"非-5gate 源终态"专项单测（先写，验证会失败）**

在 `src/evolution/replay.rs` 的 `mod tests` 内（紧跟 `evaluate_threshold_relaxes_fact_risk_block` 之后）新增：

```rust
    /// KE-02：源 run 真实终态是**非-5gate**因素（如 blocked_by_budget）但 review.scores
    /// 5 闸全过。修复后 original_final_review_status 必须用 5闸重推值（approved），
    /// 不再是源真实终态——否则 original 侧算"发送失败"、new 侧 5闸算"成功"，凭空 +send_delta。
    /// 回退到 `original.final_review_status.clone()` 即变红。
    #[test]
    fn evaluate_threshold_original_uses_5gate_not_real_terminal() {
        let scores = doc! {
            "factRisk": 1_i32,       // 远低于阈值 → 不 block
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        // 源 run 真实终态 = 非-5gate 因素（预算耗尽），但 scores 5 闸全过。
        let run = mk_run_log(scores, "blocked_by_budget");
        // 放松 fact_risk_block 6→7（升阈候选）；两侧 fact_risk 都不命中（scores 极低）。
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        // KE-02 核心：original 终态 = 5闸重推(approved)，不再是源真实终态 blocked_by_budget。
        assert_eq!(
            outcome.original_final_review_status.as_deref(),
            Some("approved"),
            "original 须用 5闸重推(与 new 侧同口径),不得用源真实非-5gate 终态"
        );
        // new 侧同样 approved（升阈后仍不命中）→ send_delta 对该 run 贡献 0，不再凭空 +。
        assert_eq!(outcome.new_final_review_status.as_deref(), Some("approved"));
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib evaluate_threshold_original_uses_5gate_not_real_terminal 2>&1 | tail -20`
Expected: FAIL —— `original_final_review_status` 实得 `Some("blocked_by_budget")`（旧代码用源真实终态），断言期望 `Some("approved")`。

- [ ] **Step 3: 改 evaluate_threshold 的 original 终态口径**

把 `src/evolution/replay.rs:296`：

```rust
        original_final_review_status: Some(original.final_review_status.clone()),
```

替换为：

```rust
        // KE-02：original 终态用 5闸重推(基于已在上方算好的 original_5gate_hit)，
        // 与 prompt 路径 prompt_sample_to_outcome 及 new 侧同口径。旧代码用源 run 真实
        // 终态 original.final_review_status，若终态是非-5gate 因素(blocked_by_budget/
        // ai_waiting_for_more_context 等)会让 original 侧算"发送失败"、new 侧 5闸算"成功"，
        // 凭空 +send_delta 虚假翻越 min_send_success_delta 门。两侧同口径后唯一变量是被改 gate。
        original_final_review_status: Some(final_status_from_5gate(&original_5gate_hit).to_string()),
```

- [ ] **Step 4: 运行确认单测通过**

Run: `cargo test --lib evaluate_threshold 2>&1 | tail -30`
Expected: 全部 PASS —— 新测通过；既有 `evaluate_threshold_*`（:683-859）全绿（它们断言的是 `new_final_review_status` / `new_5gate_hit` / `original_5gate_hit`，无一断言 `original_final_review_status`，已亲验，故不受影响）。

- [ ] **Step 5: significance 相关测确认 #152 门不破**

Run: `cargo test --lib grade_threshold 2>&1 | tail -20`
Expected: 全部 PASS（`grade_safety_regression` 依赖 `original_final_review_status==block_status`；改用 5闸重推后当 original_5gate_hit 命中安全闸仍产出该 block 态，门照常工作）。

- [ ] **Step 6: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 7: Commit**

```bash
git add src/evolution/replay.rs
git commit -m "fix(evolution): threshold 重判 original 终态改用 5闸重推,与 new 侧对齐口径 (KE-02)"
```

---

## Self-Review 结论

- **Spec coverage**：KE-01（方向门）→ Task 1；KE-02（口径对齐）→ Task 2。两条 finding 全覆盖。
- **Placeholder scan**：无 TBD/TODO，每步含完整可编译代码 + 精确命令 + 期望输出。
- **Type consistency**：`decide_auto_release` 新签名 5 参在函数定义（Task1 Step3）、调用点（Step4）、7 个单测（Step1）三处一致；`final_status_from_5gate(&original_5gate_hit)` 复用已存在函数 + 已算好的本地变量，类型 `&Document → &'static str` 已亲验。
- **既有测试冲击**：Task1 删除 2 个断言旧"任意一侧放行"的测（KE-01 有意废除的行为，反过拟合合规）；Task2 亲验既有 threshold 测无一断言 `original_final_review_status`，零冲击。
