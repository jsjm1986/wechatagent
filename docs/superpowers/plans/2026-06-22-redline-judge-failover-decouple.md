# redline 红线门 judge1-key 与 agent-failover 解耦 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 解耦 `REAL_LLM_JUDGE_API_KEY` 的两用（judge1-key + agent-failover 备胎），让 redline 6 分片的 autonomy 红线门跨家族双裁判成立，消除单裁判噪声假红。

**Architecture:** 三个 redline 硬门测试文件断开 `strongest_model_client()` → agent 备胎链的接线（保留它作裁判用途，principal_relay 例外需连带删函数防 dead-code）；CI redline job 补 judge1 的 key。纯 tests/ + CI 改动，零 src/。

**Tech Stack:** Rust 2021 集成测试（`tests/`）、GitHub Actions YAML（`.github/workflows/ci.yml`）。

## Global Constraints

- **零 src/ 改动**：只改 `tests/*.rs` 与 `.github/workflows/ci.yml`。
- **反过拟合（红线）**：不碰 `AUTONOMY_HARD_THRESHOLD=7`、`aggregate_autonomy_medians` 取 max 聚合、`classify_autonomy` 判定逻辑。修的是"让多裁判成立"，不是"让这条变绿"。双裁判后 agent 若真违规仍须正确红。
- **不动软诊断 job**：`real_llm_adversarial` / `real_llm_ops_smoke` 也有 strongest→agent 备胎耦合，但是 `continue-on-error` 软诊断的有意设计——不碰。
- **本地磁盘满**：只能 `cargo check --tests`（名称解析 + dead-code 检测，`-D warnings` 下 dead code 即编译失败）+ 纯文本核对。全量编译/真跑靠 CI。
- **真信号验证**：靠 CI 单跑 `dispatch_target=redline_single` + `redline_file=<分片>`，看 judge1+judge2 双裁判都出分。

---

## Task 1: cross_domain_arc 断 strongest→agent 备胎

**Files:**
- Modify: `tests/real_llm_cross_domain_arc.rs:210-214`（`failover_backups()` 函数体）

**Interfaces:**
- Consumes: 无（独立改动）
- Produces: 无新接口。`failover_backups()` 签名不变（仍 `-> Vec<Arc<LlmClient>>`），仅不再把 `strongest_model_client()` 结果纳入 agent 备胎。`strongest_model_client()` 函数**保留**（`judge_provider():242` 仍引用它作裁判 provider）。

**背景**：`failover_backups()` 当前把 `strongest_model_client()`（读 `REAL_LLM_JUDGE_API_KEY`）塞进被测 agent 的 failover 备胎链。这导致"配 judge1 key"会连带激活 agent failover、污染被测纯度。删掉这一接线后，`REAL_LLM_JUDGE_API_KEY` 回归纯裁判用途。agent 的 failover 第二层（`REAL_LLM_FAILOVER_API_KEY` 独立变量）保留不动。

- [ ] **Step 1: 删除 strongest→agent 备胎接线**

当前 `failover_backups()`（:210-224）开头为：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if let Some(c) = strongest_model_client() {
        backups.push(c);
    }
    if failover_key_present() {
```
删除中间那 3 行（`if let Some(c) = strongest_model_client() { backups.push(c); }`），改为：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if failover_key_present() {
```
其余函数体（`failover_key_present()` 那段及之后）**逐字不动**。

- [ ] **Step 2: 确认 strongest_model_client 未变 dead-code**

Run: `grep -nE "strongest_model_client" tests/real_llm_cross_domain_arc.rs`
Expected: 仍有 3 处——定义 `fn strongest_model_client`、`judge_provider()` 内的 `match strongest_model_client()`（约 :242）；删掉的是 `failover_backups` 内那处。`judge_provider` 的引用保证它不是 dead code。

- [ ] **Step 3: cargo check --tests 验证编译（名称解析 + dead-code）**

Run: `cargo check --tests --test real_llm_cross_domain_arc 2>&1 | tail -20`
Expected: 编译通过，无 `warning: function is never used`（`strongest_model_client` 仍被 judge_provider 用）。若磁盘满 link 失败，只要前面没有 dead-code/name-resolution error 即视为通过（与 CLAUDE.md 本地/CI 分工一致）。

- [ ] **Step 4: Commit**

```bash
git add tests/real_llm_cross_domain_arc.rs
git commit -m "test(redline): cross_domain_arc断strongest→agent备胎(解耦judge1 key)

failover_backups不再把strongest_model_client(读REAL_LLM_JUDGE_API_KEY)塞进被测
agent备胎链,使REAL_LLM_JUDGE_API_KEY回归纯judge1裁判用途。strongest_model_client
保留(judge_provider仍用)。agent failover第二层REAL_LLM_FAILOVER_API_KEY不动。"
```

---

## Task 2: principal_channel 断 strongest→agent 备胎

**Files:**
- Modify: `tests/real_llm_principal_channel.rs:185-189`（`failover_backups()` 函数体）

**Interfaces:**
- Consumes: 无
- Produces: 无新接口。与 Task 1 同形：`failover_backups()` 签名不变，`strongest_model_client()` 保留（`judge_provider():217` 仍引用）。

**背景**：与 Task 1 完全同构——principal_channel 也有 `strongest_model_client()` 经 `failover_backups()` 塞进 agent 备胎的耦合，且其 `judge_provider()`（:217）也仍用 strongest 当裁判，故同样只删 push 3 行、保留函数。

- [ ] **Step 1: 删除 strongest→agent 备胎接线**

当前 `failover_backups()`（:185-189 起）：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if let Some(c) = strongest_model_client() {
        backups.push(c);
    }
    if failover_key_present() {
```
删除 `if let Some(c) = strongest_model_client() { backups.push(c); }` 这 3 行，改为：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if failover_key_present() {
```
其余逐字不动。

- [ ] **Step 2: 确认 strongest_model_client 未变 dead-code**

Run: `grep -nE "strongest_model_client" tests/real_llm_principal_channel.rs`
Expected: 仍有 3 处（定义 :165、`judge_provider()` 内 :217、删前的 :187）；删后剩定义 + judge_provider 2 处引用，非 dead。

- [ ] **Step 3: cargo check --tests 验证**

Run: `cargo check --tests --test real_llm_principal_channel 2>&1 | tail -20`
Expected: 无 dead-code / name-resolution error。

- [ ] **Step 4: Commit**

```bash
git add tests/real_llm_principal_channel.rs
git commit -m "test(redline): principal_channel断strongest→agent备胎(解耦judge1 key)

同cross_domain_arc:failover_backups删strongest_model_client塞agent备胎那3行,
strongest保留(judge_provider仍用)。"
```

---

## Task 3: principal_relay 断 strongest→agent 备胎 + 连带删 strongest 函数

**Files:**
- Modify: `tests/real_llm_principal_relay.rs:195-199`（`failover_backups()` 函数体）+ `:173-182`（`strongest_model_client` 函数定义连注释）

**Interfaces:**
- Consumes: 无
- Produces: 无新接口。`failover_backups()` 签名不变。**与 Task 1/2 的关键差异**：principal_relay 的 judge 走 `common::autonomy_gate::judges_from_env()`（:581）/ `conversation_gate::judges_from_env()`（:599），**不**使用本地 `strongest_model_client`（无 `judge_provider`）。strongest 唯一用途就是 agent 备胎，删 push 后变 dead-code，`-D warnings` 会编译失败——故须连带删 `strongest_model_client` 函数定义本身。

**已核实的级联安全性**：删 `strongest_model_client` 后，其内部调用的 `build_real_client`（:92 定义）仍被 agent 主 client 构造（:87）引用，不连带 dead；`failover_key_present`/`failover_model_list` 删的是 push 行、不碰它们，仍被 `failover_backups` 余下逻辑引用，不连带 dead。所以 Task 3 只删两段，无进一步级联。

- [ ] **Step 1: 删除 failover_backups 里的 strongest→agent 备胎接线**

当前 `failover_backups()`（:195-199 起）：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if let Some(c) = strongest_model_client() {
        backups.push(c);
    }
    if failover_key_present() {
```
删除 `if let Some(c) = strongest_model_client() { backups.push(c); }` 这 3 行，改为：
```rust
fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if failover_key_present() {
```

- [ ] **Step 2: 连带删除 strongest_model_client 函数定义（含注释）**

删除 :173-182 这整段（2 行注释 + 函数体）：
```rust
/// 构造最强模型 client（默认 llama-3.3-70b @ NVIDIA integrate）。缺 `REAL_LLM_JUDGE_API_KEY` → None。
/// 本套件不打分，仅借它作 agent 备胎链首选。
fn strongest_model_client() -> Option<Arc<LlmClient>> {
    let key = std::env::var("REAL_LLM_JUDGE_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base = std::env::var("REAL_LLM_JUDGE_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
    let model = std::env::var("REAL_LLM_JUDGE_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
    Some(Arc::new(build_real_client(base, key, model, "REAL_LLM_JUDGE_FORMAT", 5)))
}
```

- [ ] **Step 3: 确认 strongest_model_client 已无引用、且无新 dead-code**

Run: `grep -nE "strongest_model_client|build_real_client|failover_key_present|failover_model_list" tests/real_llm_principal_relay.rs`
Expected: `strongest_model_client` **零命中**（已删干净）；`build_real_client` 仍 2 处（:92 定义 + :87 agent 主 client 引用）；`failover_key_present`/`failover_model_list` 仍各 2 处（定义 + failover_backups 内引用）——均非 dead。

- [ ] **Step 4: cargo check --tests 验证（dead-code 是本 Task 的核心风险）**

Run: `cargo check --tests --test real_llm_principal_relay 2>&1 | tail -20`
Expected: 编译通过，**无 `warning: function is never used`**。若仍报某函数 dead，按报告连带处理并在 commit 说明。

- [ ] **Step 5: Commit**

```bash
git add tests/real_llm_principal_relay.rs
git commit -m "test(redline): principal_relay断strongest→agent备胎+连带删strongest函数

本套件judge走judges_from_env不用本地strongest,删failover_backups的push后strongest
成dead-code(-D warnings编译失败),故连带删strongest_model_client函数定义。
build_real_client仍被agent主client用、failover_key_present/model_list仍被
failover_backups用,均不连带dead。"
```

---

## Task 4: CI redline job 补 judge1 key

**Files:**
- Modify: `.github/workflows/ci.yml`（`real-llm-redline` job 的 `cargo test` step 的 env，judge1 env 组，约 :1555-1560）

**Interfaces:**
- Consumes: Task 1-3 已断开 strongest→agent 备胎（故配此 key 不再激活 agent failover）
- Produces: redline 6 分片运行时 judge1（gpt-5.4 @ rsxermu）出席裁判团。

**背景**：当前 redline cargo step 的 judge1 env 组配了 `REAL_LLM_JUDGE1_MODEL`/`REAL_LLM_JUDGE_BASE_URL`/`REAL_LLM_JUDGE_MODEL`/`REAL_LLM_JUDGE_FORMAT`/`REAL_LLM_JUDGE=1`，**独缺 `REAL_LLM_JUDGE_API_KEY`**——这正是 judge1 缺席、退化单裁判 qwen 假红的直接原因。`autonomy_gate.rs:113` 要求 `REAL_LLM_JUDGE_BASE_URL` 和 `REAL_LLM_JUDGE_API_KEY` 同时为 `Ok` 才装配 judge1。Task 1-3 解耦后，补这个 key 只补裁判、不激活 agent failover。judge1 = gpt-5.4 与 `REAL_LLM_JUDGE_BASE_URL: https://rsxermu666.cn/v1` 同源，故 key 用 `RSXERMU_KEY`。

- [ ] **Step 1: 在 judge1 env 组补 REAL_LLM_JUDGE_API_KEY**

在 `real-llm-redline` job 的 cargo step env 里，`REAL_LLM_JUDGE_FORMAT: openai`（约 :1558）这一行之后，新增一行（与同组保持 10 空格缩进）：
```yaml
          REAL_LLM_JUDGE_API_KEY: ${{ secrets.RSXERMU_KEY }}
```
其余 env（JUDGE2 组、ROLEPLAYER 组、LEDGER、run 行）逐字不动。仅此 job 改动——adversarial / ops_smoke / 其它 job 的 env 一律不碰。

- [ ] **Step 2: 校验 YAML 合法 + key 已就位**

Run:
```
python -c "import io,yaml; d=yaml.safe_load(io.open('.github/workflows/ci.yml',encoding='utf-8')); s=[x for x in d['jobs']['real-llm-redline']['steps'] if 'cargo test' in x.get('name','')][0]; print('JUDGE_API_KEY:', s['env'].get('REAL_LLM_JUDGE_API_KEY')); print('JUDGE2_API_KEY:', s['env'].get('REAL_LLM_JUDGE2_API_KEY'))"
```
Expected:
```
JUDGE_API_KEY: ${{ secrets.RSXERMU_KEY }}
JUDGE2_API_KEY: ${{ secrets.NVIDIA_KEY }}
```
（judge1 用 RSXERMU、judge2 用 NVIDIA，跨家族双裁判齐备。）

- [ ] **Step 3: 确认未误改其它 job 的 judge env**

Run: `git diff .github/workflows/ci.yml | grep -E "^\+" | grep -v "^\+\+\+"`
Expected: 仅 1 行新增 `REAL_LLM_JUDGE_API_KEY: ${{ secrets.RSXERMU_KEY }}`，无其它改动。

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(redline): 补judge1 key让redline门跨家族双裁判成立

redline cargo step的judge1 env组独缺REAL_LLM_JUDGE_API_KEY→autonomy_gate.rs:113
判定judge1缺席→单judge2-qwen噪声假红。Task1-3已断strongest→agent备胎,故补此key
(=RSXERMU,与JUDGE_BASE_URL rsxermu同源)只补judge1裁判、不激活agent failover。
judge1 gpt+judge2 qwen跨家族中位数抵噪。仅redline job,adversarial/ops不动。"
```

---

## Task 5: 全量 check + 收尾

**Files:**
- 仅校验，不改码。

- [ ] **Step 1: 三个改动文件名称解析 + dead-code 全量 check**

Run: `cargo check --tests --test real_llm_cross_domain_arc --test real_llm_principal_channel --test real_llm_principal_relay 2>&1 | grep -iE "error|never used|warning" | head`
Expected: 无 `error`、无 `function is never used`。若仅出现磁盘满导致的 link 阶段失败（`os error 112` / `LNK`），而无 name-resolution / dead-code error，视为本地通过（CLAUDE.md 本地/CI 分工）。

- [ ] **Step 2: 确认零 src/ 改动 + 反过拟合（阈值/判定未动）**

Run:
```
git diff origin/main...HEAD --name-only -- src/ | head
grep -rnE "AUTONOMY_HARD_THRESHOLD|aggregate_autonomy_medians" tests/common/autonomy_gate.rs | head
```
Expected: 第一条无输出（零 src/ 改动）；第二条显示 `AUTONOMY_HARD_THRESHOLD` 仍 = 7、`aggregate_autonomy_medians` 仍取 `.max()`——本批未碰。

- [ ] **Step 3: 推送 + CI 单跑真信号验证**

```bash
git push origin worktree-eval-overhaul-phase1
gh workflow run CI -f dispatch_target=redline_single -f redline_file=real_llm_cross_domain_arc
```
推送后 PR #28 的 redline 门（PR 事件触发的常规门）会重跑；同时上面的 dispatch 单跑独立验证。

- [ ] **Step 4: 核验 CI 真信号（单跑 run 跑完后）**

拉单跑 job 日志，确认：
- judge1 与 judge2 **都出现**（grep `judge1` 和 `judge2-qwen` 各 ≥1 次），不再是单裁判。
- identity_probe / 销售域 turn-1 的 autonomyRisk 跨裁判**中位数**不再因单 qwen 噪声达 10（judge1 gpt 给出对照分，median 抵噪）。
- redline 门对身份探针弧不再假红 panic（agent 回复守线时判 Clean）。
- 若双裁判后某弧仍判 Breach，**拉 transcript 核实 agent 是否真违规**——真违规则是正确红（不可改测试掩盖，反过拟合红线）。

抽验 `redline_file=real_llm_principal_channel` / `real_llm_principal_relay` 同理。

---

## Self-Review

**1. Spec coverage**（对照 spec docs/superpowers/specs/2026-06-22-redline-judge-failover-decouple-design.md 四节落地清单）：
- spec §3.1 cross_domain_arc:212-214 删 push、strongest 保留 → Task 1 ✓
- spec §3.1 principal_channel:187-189 删 push、strongest 保留 → Task 2 ✓
- spec §3.1 principal_relay:197-199 删 push + 连带删 strongest 函数(:173-182) → Task 3 ✓
- spec §3.2 CI redline 补 `REAL_LLM_JUDGE_API_KEY: RSXERMU_KEY` → Task 4 ✓
- spec §五 验证（cargo check + CI 单跑双裁判出分） → Task 5 ✓
- spec §六 边界（兜底留后续、软诊断不动）→ Global Constraints + Task 4 Step 1 显式约束 ✓
- 全覆盖。

**2. Placeholder scan**：无 TBD/TODO；每个删除/新增步骤都给了精确行号 + 前后完整代码块 + 期望 grep/编译输出。无占位。

**3. Type consistency**：`failover_backups() -> Vec<Arc<LlmClient>>` 签名在 Task 1/2/3 三处一致、均不变；`strongest_model_client()` 在 Task 1/2 保留、Task 3 删除（差异已在 Interfaces 块显式说明原因——judge_provider 有无引用）；CI key 变量名 `REAL_LLM_JUDGE_API_KEY` 与 spec/autonomy_gate.rs:113 读取的变量名逐字一致。无不一致。

无 issue。

