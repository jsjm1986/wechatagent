# KE 家族① 修复设计：evolution auto_release 方向一致性 + threshold 重判口径对齐

> 批 E 家族①（P2 最后一项）。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KE-01（:1038-1048）+ KE-02（:1050-1059）。

## 背景与根因（全部主控当场 Read/Grep 亲验，行号基于 origin/main 2ae5ac2）

evolution 自优化在 tick 末尾对 `proposal_kind="threshold" AND status="eligible_for_release"` 的候选做**自动放量**（`auto_release_eligible_thresholds`，auto_release.rs:48）。整条通道受**双闸**保护：env 总闸 `evolution_auto_release_enabled` AND per-workspace 子闸 `threshold_auto_release_enabled`（`auto_release_gate_open` :39-41），**默认双关**——两条 finding 都只在运维显式开启后才影响放量。

本批修两条"设计意图/doc 声称与实现漂移"（元家族典型），均为 `src/evolution/` 隔离模块内的纯逻辑修复，无跨模块张力。

### KE-01（CONFIRMED · Medium）：decide_auto_release 缺方向一致性校验

- 模块 doc（auto_release.rs:11-13）声称放行条件是"命中率仍在 band 之外**（方向与候选方向一致，意味着信号没有自然回正）**"。
- 实现 `decide_auto_release`（:207-211）签名 `(observed, target_lower, target_upper)` **无方向参数**，:210 只判 `rate < target_lower || rate > target_upper`（band 外**任意一侧**）。
- 调用点（:154）手握整条 `proposal`（含 `current_value` :275 / `proposed_value` :250，均 `Option<f64>`）却从未传方向。
- **失败场景（亲验推演）**：gate=fact_risk_block band=[0.05,0.15]，候选因 hit_rate=0.30>upper 生成"升阈"候选（升阈→拦截更少→命中率降）；auto_release tick 用**不同时间窗**重看命中率，若此时 observed 已翻转到 0.02<lower → `decide_auto_release(0.02,0.05,0.15)`→`0.02<0.05`=true→放行升阈候选。但命中率此刻已过低，继续升阈只把命中率推更低——朝**错误方向**放量，与"信号没自然回正才放行"设计相悖。

### KE-02（PLAUSIBLE · Medium）：threshold 重判 send_success 口径不对称

- `evaluate_threshold`（replay.rs:245-308）里两侧终态口径不对称：
  - `original_final_review_status`（:296）= `original.final_review_status.clone()` —— 源 run **真实终态**，可能是 `blocked_by_budget` / `ai_waiting_for_more_context` 等**非-5gate**因素决定的合法终态。
  - `new_final_review_status`（:299）= `final_status_from_5gate(&new_5gate_hit)`（:406-425）—— **纯 5 闸重判**，只产 5 闸决定的 5 种状态。
- **失败场景**：cohort 若含"非-5gate 因素 block 但带 `review.scores`"的 run：original 侧按真实终态计为**发送失败**、new 侧 5 闸重判不命中算 **approved=成功** → 凭空制造一次 send_success 提升（significance.rs:147-149 `send_delta = new_send - original_send`），与被改阈值无关，累积可虚假翻越 `min_send_success_delta`(0.05) 门 → 误放行伪改进。
- **关键亲验**：**prompt 路径**（`prompt_sample_to_outcome` :451-454）已对两侧都用 `final_status_from_5gate`（对称）；**只有 threshold 路径 :296 不对称**。且 `evaluate_threshold` 在 :265/279 **已算好 `original_5gate_hit`**（被改 gate 用 `current_value` 阈值、其余 4 gate 用 default），修复所需的重推向量现成可用。

## 目标

auto_release 通道（仅显式开启时）：① 只在 observed 偏离方向与候选修正方向一致时放量（KE-01）；② threshold 重判 original/new 两侧 send_success 同口径，非-5gate 因素不污染 send_delta（KE-02）。两条独立、都在 evolution 隔离模块。

## 架构：两条独立纯逻辑修复

### KE-01 —— decide_auto_release 加候选方向门

`decide_auto_release` 加 `current_value` / `proposed_value` 两参，方向由 `proposed_value - current_value` 符号表达：

```rust
pub fn decide_auto_release(
    observed: Option<f64>,
    target_lower: f64,
    target_upper: f64,
    current_value: Option<f64>,
    proposed_value: Option<f64>,
) -> bool {
    let Some(rate) = observed else { return false };          // 无信号保守 SKIP（不变）
    let (Some(cur), Some(prop)) = (current_value, proposed_value) else {
        return false;                                          // 缺方向：保守 SKIP
    };
    if prop > cur {
        rate > target_upper        // 升阈候选（阈值调高→命中率将降）：仅命中率仍过高才放行
    } else if prop < cur {
        rate < target_lower        // 降阈候选（阈值调低→命中率将升）：仅命中率仍过低才放行
    } else {
        false                      // proposed==current：无方向变化，SKIP
    }
}
```

调用点（:154）改为传 `proposal.current_value, proposal.proposed_value`。

**安全性质**：新逻辑是旧逻辑的**收窄子集**。旧的 `rate<lower || rate>upper` 中，升阈候选原本 `rate<lower` 也放行（错误方向），现被 SKIP；`rate>upper` 仍放行（正确）。降阈候选对称。故本修复**只减少误放行、不新增任何放行**，安全方向单调。

### KE-02 —— threshold 路径 original 终态改用 5 闸重推

`evaluate_threshold`（replay.rs:296）改为复用已算好的 `original_5gate_hit`：

```rust
// 旧：original_final_review_status: Some(original.final_review_status.clone()),
original_final_review_status: Some(final_status_from_5gate(&original_5gate_hit).to_string()),
```

两侧从此同口径——唯一变量是被改的那个 gate（original 用 `current_value` 阈值、new 用 `proposed_value` 阈值），其余 4 gate 两侧都 default、delta 恒 0。非-5gate 因素不再污染 send_delta。这与 prompt 路径（`prompt_sample_to_outcome` :451-454 两侧都用 `final_status_from_5gate`）对齐。

**不破 #152 反向安全门（已亲验）**：`grade_safety_regression`（significance.rs:98-125）依赖 `original_final_review_status == block_status`（`held_by_ai_policy` / `blocked_by_safety_guard` / `blocked_unverified_product_claim`）。`final_status_from_5gate` 正好产出这些 block 态——当 `original_5gate_hit` 命中安全闸时照样返回该 block_status，安全回归门照常工作，且比之前**更一致**（两侧同源于 5 闸重推）。

## 回归风险

1. **KE-01 方向门是旧逻辑安全收窄**（上证单调），只减误放行；默认双关，生产零影响。
2. **KE-02 口径对齐后 #152 门更一致**（已亲验不破）；send_delta 从此只反映被改 gate 的真实效应。
3. **既有单测须随签名/口径更新**（反过拟合边界）：
   - auto_release.rs:384-404 的 3 个 `decide_auto_release_*` 单测现调 3 参，签名变更后 E0061 编译错，须补方向参——这是签名变更被迫更新、非为过测试改逻辑，且新增方向门专项断言。
   - replay.rs:683-856 的 `evaluate_threshold_*` 若有断言 `original_final_review_status`=源真实终态者，属被本修复有意改变的口径，按反过拟合只改该断言。
4. **baseline**：两文件都在 evolution 模块，不触 baseline 门 4 个 PBT（state_transition / memory_card / wiki_chunk_revision / llm_retry_jitter），lib≥350 不回退。

## 改动面

- **Modify** `src/evolution/auto_release.rs`：`decide_auto_release` 加 current/proposed 两参 + 方向门（KE-01）；调用点 :154 传 proposal.current_value/proposed_value；更新 :384-404 的 3 个既有单测调用 + 新增方向门专项单测。
- **Modify** `src/evolution/replay.rs`：`evaluate_threshold` :296 改用 `final_status_from_5gate(&original_5gate_hit)`（KE-02）；检查/更新 :683-856 断言 original_final 的既有测试 + 新增"非-5gate 源终态"专项单测。

## 测试计划

- **KE-01 方向门单测（lib）**：
  - 升阈候选(proposed>current) + observed>upper → **release**；
  - 升阈候选 + observed<lower → **SKIP**（旧逻辑会误放行，本测锁死修复，回退即红）；
  - 降阈候选(proposed<current) + observed<lower → **release**；
  - 降阈候选 + observed>upper → **SKIP**；
  - observed=None → SKIP；current/proposed 任一缺失 → SKIP；proposed==current → SKIP。
- **KE-02 口径对齐单测（lib）**：构造源 run 真实终态=非-5gate（如 `blocked_by_budget`）但 `review.scores` 5 闸全过的 original → `evaluate_threshold` 后断言 `original_final_review_status`=5 闸重推值（approved），**不再**是源真实终态（回退即红）。

## 非目标（YAGNI）

- 不动 auto_release 双闸 / 负反应强制门 / `compute_window_gate_hit_rates`（口径正确）。
- 不动 prompt 路径（`prompt_sample_to_outcome` 两侧已对称）。
- 不做 KE-03/04/05/06（referral，Low），独立不含本 PR。
