# 优化线 E · 请示评审域 实施计划（S5 批复项 5+6）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 请示通道链尾超时后可执行运营预授权底线（客户不再无限等待）；寒暄低风险轮跳过 ClaimGate 调用（每轮省一次 LLM）。

**Architecture:** 任务 5 是"人类预授权的延迟执行"——语义等同领导提前给出的 conditional 裁决，AI 只是执行方，不是代决（命名与文案严守禁词红线）；任务 6 是保守条件下的成本刀，审查（Review）本体照跑，只跳过语义声明抽取器。均为默认行为不加 flag。

## Global Constraints

- 模型：仅 Fable 5 1M max（不派生 subagent）。
- 红线：动手前全文读懂受影响函数并重验锚点；R5.4 判定输入契约零破坏（它消费 reviewer 的 claim_analysis——动 ClaimGate 前必须亲验这一独立性）；禁词 lint（standing order 相关全部命名/文案/事件 kind 过 `check-no-human-takeover.sh`）；磁盘紧张先清本 worktree incremental。
- 文件边界：只许修改 `src/models.rs`（仅 AskHumanPolicy 增字段）、`src/agent/escalation/**`、`src/agent/review/**`、`src/agent/gateway.rs`（仅 ClaimGate 调用分流点）、`src/routes/domains.rs`（policy 校验）、`frontend/src/features/ask-human-config/**`、`tests/**`。
- 每任务独立 commit；收尾 `cargo test --lib` 全绿（≥2534）+ 四 PBT + `-D warnings` + 禁词 lint + `cd frontend && npm test` 相关文件。

---

### Task E1: 请示预授权底线（standing order，S5-5）

**Files:**
- Modify: `src/models.rs`（`AskHumanPolicy` 增两字段，serde default 向后兼容）
- Modify: `src/agent/escalation/policy.rs`（`ResolvedAskHumanPolicy` 同步）
- Modify: `src/agent/escalation/mod.rs`（超时扫描链尾分支）
- Modify: `src/routes/domains.rs`（ask-human-policy PUT 校验）
- Modify: `frontend/src/features/ask-human-config/`（policyForm/表单两字段）
- Test: escalation 集成测试（`#[ignore]`）+ policy 解析单测 + 前端表单测试

**行为契约:**
- 模型字段：`standing_order: Option<String>`（运营预写的底线口径文本，如"最多 95 折，赠品可送，超出请客户稍等"）+ `standing_order_after_hours: Option<f64>`（链尾无人应答持续多久后启用；None 或未配 = 维持现状无限等待+安抚）。两字段 `#[serde(default, skip_serializing_if = "Option::is_none")]`。
- 触发条件（超时扫描内，替换/前置于 ChainTail 安抚分支）：台账 pending ∧ 链上无下一位决策人 ∧ `standing_order` 已配置且非空白 ∧ `now - created_at > standing_order_after_hours`。
- 执行：构造与领导裁决同形的 `PrincipalDecision { verdict: conditional, substance: standing_order 文本, constraints: 空, ... }`，`decided_by` 记 `"standing_order_policy"`（重验 PrincipalDecision/resolve 内核的实际字段与物化 relay 的函数签名——复用 resolve→relay 既有链路，零新发送路径）；台账 `resolved` + 事件 `escalation_standing_order_applied`（details 含 short_code 与生效时长）。relay 之后客户收到的是 AI 以自己口吻转述的底线方案（既有 relay prompt 行为，零改动）。
- 幂等：standing order 对同一台账只应用一次（resolved 终态天然幂等；扫描 filter 排除已 resolved）。
- 校验（routes PUT）：`standing_order_after_hours` 若给出必须 > 0 且 ≤ 8760；`standing_order` 若给出必须非空白且 ≤ 2000 字符；两者必须同时给出或同时缺省（只配一半 → 400，防"配了文本永不生效"的静默误配）。
- 前端：policyForm 增 textarea（底线口径）+ number（小时数），空态提示"不配置=链尾无人应答时仅周期安抚"；文案避开禁词（用"预授权底线/兜底口径"表述）。
- 红线自检：这是执行**人类预先写好的授权**，非 AI 代决；所有新增 kind/字段/文案跑禁词 lint。

**Steps:**
- [ ] 读 `scan_escalation_timeouts`→`converge_timed_out_escalation`（线 A 刚重构过）→ChainTail 分支→resolve 内核→relay 物化链全文；重验 AskHumanPolicy/ResolvedAskHumanPolicy 现字段。
- [ ] 失败集成测试：链尾+配置+超时 → 台账 resolved（decided_by=standing_order_policy）+ relay 任务物化 + 事件在场；对照：未配置→安抚不变；只过一半时限→不触发；重复扫描→不重复应用。
- [ ] 实现（模型→policy 解析→扫描分支→routes 校验→前端表单）；后端测试绿；前端测试绿。
- [ ] Commit：`feat(escalation): apply operator standing-order floor when decider chain stays silent past deadline`

### Task E2: 寒暄低风险轮跳过 ClaimGate（S5-6）

**Files:**
- Modify: `src/agent/gateway.rs`（ClaimGate 并行调用分流点——先重验 `review_and_evaluate_claim_gate` / `evaluate_independent_claim_gate` 的调用结构）
- Modify: `src/agent/review/mod.rs`（如需暴露"空 verdict"构造或跳过原因常量；`apply_independent_claim_gate` 本体不动）
- Test: 跳过条件纯函数单测 + 集成测试（寒暄轮 llm 调用数少一次、产品声明轮照跑）

**行为契约:**
- 跳过条件（全部满足才跳，写成纯函数 `should_skip_claim_gate(decision, planner) -> bool` 便于单测）：`conversation_mode == "casual_relationship"` ∧ `planner.risk_level == "low"` ∧ `knowledge_need == "not_required"` ∧ `used_knowledge_ids` 与 `matched_knowledge_ids` 均空 ∧ `assets_to_send` 空 ∧ `namecard_to_send` 为 None ∧ escalation_request 未请求。
- 跳过时的下游契约（实现前必须亲验）：R5.4 的 `claim_requires_product_knowledge` 消费的是 **reviewer 的 claim_analysis**（独立于 ClaimGate）——亲验 `finalize_review_for_send` 与 `apply_independent_claim_gate` 各自的输入来源后，选择正确的跳过形态：要么不调 evaluate 并跳过 apply（若 apply 对缺失 verdict 是 hold 语义则不可行），要么传入"无声明"空 verdict（requiresEvidence=false, claims=[], claimsComplete=true 的等价构造）。**任何情况下 hold_for_claim_gate_failure 不得被误触发。**
- 审计：跳过时在 review.risks 追加 `claim_gate_skipped_casual_low_risk`（或 run log details，取更贴近现有审计惯例者），保证跳过率可观测、误跳可排查。
- 语义安全论证（写进注释）：满足条件的轮次无产品声明、无知识引用、无资产动作——ClaimGate 在这类轮次的历史产出恒为空声明集，跳过消除的是纯空转调用。

**Steps:**
- [ ] 读 gateway ClaimGate 调用段 + `evaluate_independent_claim_gate`/`apply_independent_claim_gate`/`hold_for_claim_gate_failure` 全文 + finalize R5.4 输入链；确认跳过形态。
- [ ] 纯函数失败单测（七条件矩阵：逐条不满足→不跳）→ 实现 → 集成测试（mock LLM：寒暄轮 claim_gate prompt key 零调用且终态 approved；引用知识的轮照调）。
- [ ] Commit：`perf(review): skip independent claim gate on knowledge-free casual low-risk turns`

### 收尾

- [ ] `cargo test --lib` ≥2534/0、四 PBT、`-D warnings`、禁词 lint、前端相关测试。
- [ ] 交付报告（≤15 行）：锚点重验结论、E1 的 resolve/relay 复用点与幂等说明、E2 的跳过形态选定依据（含 R5.4 独立性亲验结论）、测试结果、commit hashes。
