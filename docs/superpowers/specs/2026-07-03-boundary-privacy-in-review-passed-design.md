# M-boundary review_passed 漏检 boundary_privacy_safety 软闸修复设计

> 日期：2026-07-03
> 分支：`fix/m-boundary-privacy-in-review-passed`（从 origin/main d86d11b 切）
> 来源：本会话重跑审计（decision+safety-gate agent）发现

## 1. 漏洞描述（对最新代码 100% 亲验）

`review_passed`（gates.rs:20-40）是两条发送路径的最终放行判定：
- 客户主链路 revision 后的 `second_passed`（gateway.rs:1858）：`matches!(Approved) && review_passed(...)`；
- 管理发送路径（gateway.rs:313）：`matches!(Approved) && review_passed(...)`（**无 revision 循环**）。

`review_passed` 检查了 4 个软闸里的 3 个——`human_like` / `emotional_value` / `pressure_risk`——但**漏了 `boundary_privacy_safety`**。而 `classify_dual_gate`（gates.rs:183）四个软闸都检查：

```rust
// classify_dual_gate:183 —— boundary 低分(1-3)触发 SoftGateFailure
if review.scores.boundary_privacy_safety != 0 && review.scores.boundary_privacy_safety <= 3 { ... }
```

`review_passed` 没有对偶的这一行。

### boundary_privacy_safety 是什么（gates.rs:180-192）
2026-06-23 加入的隐私/边界软闸：候选回复**泄露内部画像/评判、暴露 AI 身份、暴露幕后领导信息**时给 1-3 低分。revision_direction 明写「移除对客户的内部评判表述，不暴露AI身份与幕后决策来源」——这是 CLAUDE.md「无人工接管/客户永不面对真人、AI 身份不外露」红线的量化闸。

### 后果（两条路径，机制不同，均已逐行核实）

**管理发送路径**：无 revision 循环。boundary 低、其它全过时 → `review_passed=true`（漏检）→ finalize 首分支 `approved && should_reply` 返回 Approved → `passed=true` → **直接发出泄露内部画像/AI 身份的内容**。

**客户主链路**：`needs_revision`（由 route_dual_gate 对 SoftGateFailure 置真）驱动一次 revision，所以首轮会改写。但改写后 `second_passed = Approved && review_passed`——若 revision **没修好** boundary（仍 1-3），`review_passed` 漏检 → second_passed=true → **改写失败的泄露内容仍发出**。对照 human_like/pressure：revision 没修好时 review_passed=false → revision_failed → held。boundary 独缺这层兜底。

## 2. 根因

boundary_privacy_safety 软闸 2026-06-23 加入 `classify_dual_gate` 时，未同步加入 `review_passed`。其它 3 个软闸都在 `review_passed` 里、且 `review_passed` 无任何注释说明为何豁免 boundary → 是遗漏，非有意取舍。

## 3. 方案（选定）：review_passed 补 boundary_privacy_safety 对偶判定

在 `review_passed` 末尾加一行，与 `classify_dual_gate:183` 严格对偶、镜像 `pressure_risk` 的老数据豁免形态：

```rust
// boundary/privacy 软闸对偶（与 classify_dual_gate:183 同源）：1-3 低分拦截，
// 0 = reviewer 未填/老数据豁免（同 pressure_risk）。boundary>=4 放行。
&& (review.scores.boundary_privacy_safety == 0
    || review.scores.boundary_privacy_safety > 3)
```

boundary 无 runtime 阈值字段（已核实 runtime.rs 无 boundary 项，classify_dual_gate 也是硬编码 `<= 3`），故 review_passed 也硬编码 `> 3`，与 classify 一致。

### 逐场景验证（每条已核实）

| boundary 分 | classify_dual_gate | 加固后 review_passed | 客户路径 | 管理路径 |
|---|---|---|---|---|
| 0（未填/老数据） | 不触发 | 通过（`==0`） | 不变 ✓ | 不变 ✓ |
| >=4 | 不触发 | 通过（`>3`） | 不变 ✓ | 不变 ✓ |
| 1-3 + 其它全过 | SoftGateFailure→needs_revision | **false** | 首轮 finalize 靠 needs_revision 翻 Approved 走 revision（不变）；revision 没修好 → second_passed=false → revision_failed/held ✓ **新增兜底** | passed=false → held_by_ai_policy ✓ **新增保护** |

关键不回归点：boundary 1-3 时 `review_passed=false`，但 finalize 的 needs_revision 翻回 Approved 分支（gates.rs:801-820）仍让客户路径走一次 revision——与 human_like/pressure 行为**完全一致**（route_dual_gate 对所有 SoftGateFailure 都置 needs_revision + revision_direction）。加固只影响「revision 后仍未修好」与「管理路径无 revision」两个此前漏网的终局判定。

## 4. 核心改动
`src/agent/review/gates.rs` `review_passed`：加上述一行。不动 classify_dual_gate、route_dual_gate、finalize、gateway 调用点。

## 5. 测试设计（纯函数单测，镜像现有 pressure_risk 测试 gates.rs:1063-1105）
新增到 gates.rs `#[cfg(test)]`：
- `review_passed_blocks_when_boundary_privacy_low`：full_pass_review + boundary=2 → `!review_passed`。
- `review_passed_blocks_when_boundary_privacy_at_3`：boundary=3 → `!review_passed`（边界值）。
- `review_passed_passes_when_boundary_privacy_at_4`：boundary=4 → `review_passed`（阈值上沿放行）。
- `review_passed_ignores_boundary_privacy_zero_for_legacy_data`：boundary=0 → `review_passed`（老数据豁免）。

### 验证
- `cargo build --lib` 无 error；`cargo test --lib` ≥ 350 / 0 + 4 新单测。
- 禁词 lint 通过（改动纯技术，AI-internal 措辞）。

## 6. 范围边界
- 只增不减：boundary 0/>=4 字节等价；仅 1-3 时新增拦截（本就该拦）。
- 过拟合红线：不改任何阈值/门逻辑，只补 review_passed 与 classify_dual_gate 的对偶缺口，让两条发送路径的 boundary 软闸与其它 3 软闸对齐。
