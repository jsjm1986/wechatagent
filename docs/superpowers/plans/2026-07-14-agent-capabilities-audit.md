# agent 旁挂能力深度逻辑审查（第一批）Implementation Plan

> **For agentic workers:** 这是**审查工程**，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 的 implementer→reviewer 双裁决流程；改由**主控直接编排**：每簇派一个只读审查 subagent → 主控逐条亲验 file:line → 填台账。步骤用 checkbox 跟踪。

**Goal:** 对 agent 旁挂能力 4 簇（~10.6k 行）做纯代码/设计层深度逻辑审查，产出一份经主控逐条亲验的 findings 台账，合并 docs PR。

**Architecture:** 4 个只读审查 subagent 分簇并行审 → 主控用 Read/Grep 逐条复核 file:line 属实性与因果链、驳回夸大 → 汇总进单一台账文件 → 合并 docs PR。只审不修。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象是 Rust（src/agent/*.rs）。

## Global Constraints

- 分支 `docs/agent-capabilities-audit`，基于含 #206 的最新 origin/main（69698eb）。
- **只审不修**：本批绝不改任何 .rs / prompt / 阈值 / 词表。产出纯 docs（台账文件）。
- **subagent 只读**：审查 subagent 不得改任何文件；每 finding 必附亲验的 `file:line` 证据（贴实际代码行）；先 100% 读懂再下结论；凭猜测的产出打回重审。
- **subagent 全部继承主会话 Opus**：省略 model 参数（`model:"opus"` 报 400 INVALID_MODEL_ID，省略即继承 opus，满足子 agent 红线）。
- **主控逐条亲验**：subagent 每个 finding 主控必用 Read/Grep 复核 file:line 属实 + 因果链成立，驳回夸大/误报（上轮铁律：subagent 首轮常夸大）。
- **两态**：PLAUSIBLE（读码推断）/ CONFIRMED（可构造推荐配置下真实触发）。
- **严重度校准**：High=推荐配置下确定性发生的核心交互失效/红线破坏；Medium=需多条件叠加或故障注入；Low=观测/边缘/就绪债/死代码/文档漂移。每条带主控裁定理由。
- **元家族聚焦**：设计声称的不变量/闭环/口径，实现层有旁路/缺口/非原子窗口/新旧不对称。
- 不碰主仓在途工作（主仓被并行会话占在 feat/principal-auth-exemption）。

---

### Task 0: 建台账骨架

**Files:**
- Create: `docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md`

- [ ] **Step 1: 写台账头部**

台账头含：标题、审查范围（4 簇 + 模块清单）、方法论（subagent 分簇审 + 主控亲验 + 两态）、严重度校准口径、元家族说明、finding 字段模板。字段模板逐字：

```
### [X-NN] 一句话标题
- 入口频道: —
- 所属簇: A|B|C|D
- 类型: 幂等|竞态|错误处理|一致性|逻辑正确性|红线|文档-代码漂移|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): agent 旁挂能力审查台账骨架(第一批)"
```

---

### Task A: 簇A 记忆固化审查

**审查对象:** `src/agent/memory.rs`(3291) + `src/agent/consolidation_window.rs`(77)

**Interfaces:**
- Produces: 簇A findings（A-01, A-02…），主控亲验后填入台账。

- [ ] **Step 1: 派审查 subagent（Explore 类，只读，继承 Opus）**

dispatch 指令要点：
- 审查对象两文件 + 相关调用点（memoryCard 固化被谁调、consolidation window 如何触发）。
- **重心**：memoryCard 长期固化的无界 append / 覆盖语义 / 并发写窗口 / 置信门缺失。
- **已知线索喂入**：memory 记载"memory_summary 无界 append 写侧严谨待修" + "画像/记忆更新须保守，不因一句话盲目画像，gateway tags/stage 覆盖无置信门"——核这两条当前是否仍成立、根因 file:line。
- **元家族**：固化声称的不变量（如"只在证据充分时更新"）实现层有无旁路。
- 硬约束：先读懂再下结论、每 finding 附 file:line 实际代码行、只读不改、PLAUSIBLE/CONFIRMED 标注、严重度初判带理由。
- 报告写到 `.superpowers/audit/cluster-A-report.md`，返回值只给 finding 数 + 一行摘要。

- [ ] **Step 2: 主控逐条亲验**

对 subagent 每个 finding：Read/Grep 复核 file:line 属实、因果链成立；驳回夸大；校准严重度。记录亲验结论。

- [ ] **Step 3: 填台账 + Commit**

主控把亲验通过的 finding 按字段模板写入台账（编号 A-NN）。

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): 簇A 记忆固化 findings(主控亲验)"
```

---

### Task B: 簇B 标签体系审查

**审查对象:** `src/agent/taxonomy.rs`(1036) + `src/agent/decision_taxonomy.rs`(427) + `src/agent/tag_evidence.rs`(101) + `src/agent/bayesian_slots.rs`(202)

**Interfaces:**
- Produces: 簇B findings（B-01…），主控亲验后填台账。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus）**

dispatch 指令要点：
- **重心**：双层标签铁律——"AI 标签不可信 → 三层物理隔离(manual 权威 / confirmed AI / tag_observation)" + "证据 fail-closed" + "bayesian/personality 只写不进决策"。查这些不变量实现层有无旁路。
- **已知线索喂入**：memory `project_tag_trust_reform` 记"8 铁律 HOLDS"——本批复核每条是否真 HOLDS，特别是 bayesian/personality 是否真的只写不进决策链、证据不足时是否真 fail-closed。
- **元家族**：物理隔离声称"AI 标签永不驱动决策"，查有无写路径让 confirmed/observation 层渗入决策。
- 硬约束同 Task A。
- 报告写到 `.superpowers/audit/cluster-B-report.md`。

- [ ] **Step 2: 主控逐条亲验**（同 Task A Step 2）

- [ ] **Step 3: 填台账 + Commit**

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): 簇B 标签体系 findings(主控亲验)"
```

---

### Task C: 簇C 通用化底座审查

**审查对象:** `src/agent/domain_profile.rs`(2454) + `src/agent/domain.rs`(107) + `src/agent/domain_signals.rs`(456) + `src/agent/dimension_registry.rs`(449)

**Interfaces:**
- Produces: 簇C findings（C-01…），主控亲验后填台账。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus）**

dispatch 指令要点：
- **重心**：行业无关引擎的扩展点是否有硬编码销售假设；profile 加载/派生/应用（apply_active_profile）的新旧字段不对称；dimension_registry 默认值口径；domain_signals 与 taxonomy 的边界。
- **已知线索喂入**：memory `project_universalization_residuals` 记"引擎/契约/知识三层已闭环，残留命门在前端 labelFor 写死销售标签"——本批查**后端引擎层**是否也有残留硬编码销售假设（前端不在本批范围）。memory `project_universal_base_extensibility_audit` 记"C3 接线点 apply_active_profile 仅 3 标量"——核扩展点是否够用。
- **元家族**：通用化声称"行业无关"，查实现层残留的行业特定假设/默认值。
- 硬约束同 Task A。
- 报告写到 `.superpowers/audit/cluster-C-report.md`。

- [ ] **Step 2: 主控逐条亲验**（同 Task A Step 2）

- [ ] **Step 3: 填台账 + Commit**

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): 簇C 通用化底座 findings(主控亲验)"
```

---

### Task D: 簇D 节流与准入审查

**审查对象:** `src/agent/simulation.rs`(265) + `src/agent/pacing.rs`(51) + `src/agent/quiet_hours.rs`(357) + `src/agent/entitlements.rs`(1311)

**Interfaces:**
- Produces: 簇D findings（D-01…），主控亲验后填台账。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus）**

dispatch 指令要点：
- **重心**：影子模拟（simulate_user_dialogue）与真实发送路径的隔离性（绝不误触真实 MCP / outbox / DB 写）；pacing 账号级间隔的边界比较符；quiet_hours 时区/跨零点/边界；entitlements 权限门的 fail-open vs fail-closed 方向（缺配置时是放行还是拒绝）。
- **元家族**：simulation 声称"影子不影响生产"，查有无写路径泄漏到真实集合；entitlements 声称"权限门"，查缺配置/异常时门是否 fail-open 误放行。
- 硬约束同 Task A。
- 报告写到 `.superpowers/audit/cluster-D-report.md`。

- [ ] **Step 2: 主控逐条亲验**（同 Task A Step 2）

- [ ] **Step 3: 填台账 + Commit**

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): 簇D 节流准入 findings(主控亲验)"
```

---

### Task E: 台账收尾 + push + PR

- [ ] **Step 1: 台账汇总头**

主控在台账头部补：4 簇总 findings 数、严重度分布（H/M/L 计数）、元家族归纳、后续 P0-P3 修复路线建议。

- [ ] **Step 2: 交叉去重**

主控扫全台账，去重跨簇重复 finding（如 memory 与 taxonomy 边界重叠），标注留痕。

- [ ] **Step 3: Commit + push（显式 refspec）+ PR**

```bash
git add docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md
git commit -m "docs(audit): agent 旁挂能力审查台账收尾(严重度分布+修复路线)"
LOCAL=$(git rev-parse HEAD)
git push origin HEAD:refs/heads/docs/agent-capabilities-audit -u
git ls-remote origin refs/heads/docs/agent-capabilities-audit   # 亲验 tip==LOCAL
gh pr create --head docs/agent-capabilities-audit --base main --title "docs(audit): agent 旁挂能力深度逻辑审查台账(第一批·只审不修)" --body "..."
gh pr view docs/agent-capabilities-audit --json number,headRefName,baseRefName,headRefOid  # 核身份
```

- [ ] **Step 4: 台账是纯 docs，CI 仅 changes/paths-filter 相关**

docs-only PR 走 paths-ignore，后端 job 大概率 skip（与上轮 PR#178 同）。核 CI 无意外 FAILURE 后 squash merge（不带 --delete-branch，worktree 铁律）。

---

## Self-Review

**1. Spec coverage:** 设计的 4 簇 → Task A/B/C/D 一一对应；只审不修 + 台账格式 → Task 0 + 各 Task Step 3；主控亲验 → 各 Task Step 2；严重度校准 + 元家族 → Global Constraints + 各簇 dispatch 要点；后续修复路径 → Task E Step 1（修复路线建议）。✓ 无遗漏。

**2. Placeholder scan:** 无 TBD/TODO。台账字段模板、各簇审查重心、已知线索、commit message 均具体。PR body 的 `"..."` 在 Task E 执行时按实际 findings 数填写（届时才知内容），非计划占位。✓

**3. Type consistency:** 台账 finding 编号 A-NN/B-NN/C-NN/D-NN 全计划一致；台账路径 `docs/superpowers/specs/2026-07-14-agent-capabilities-audit-findings.md` 各处一致；报告文件 `.superpowers/audit/cluster-{A,B,C,D}-report.md` 命名一致。✓

## 备注

- 审查 subagent 用 Explore 类（只读）或 general-purpose（只读指令约束），继承 Opus。
- Task A-D 可**并行派 subagent**（4 簇独立），但主控亲验（各 Step 2）串行做以保质量。填台账各 Step 3 独立 commit。
- 本批零代码改动、零 CI 风险（纯 docs）。
- 报告文件 `.superpowers/audit/*.md` 是 git-ignored scratch（同 SDD ledger），不进 commit。
