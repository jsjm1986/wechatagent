# CI 真模型 job 独立单跑入口 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 ci.yml 的 `workflow_dispatch` 加 8 个 `-single` 孪生 job，让每个 LLM 驱动的真模型 job 都能独立手动触发，绕开导致超时的串行 needs 长链。

**Architecture:** 镜像仓内现成范例 `real-llm-ops-single`（ci.yml:492，dispatch-only、无 needs、env+steps 自带、matrix 收敛到 `${{ inputs.* }}` 单值）。为 8 个无单跑入口的真模型 job 各加一个 `-single` 孪生 job；原 8 个 job 的 push/PR 串行链逐字不动。dispatch_target choice 加 8 个值，4 个 matrix job 加分片 input。

**Tech Stack:** GitHub Actions YAML（`.github/workflows/ci.yml`）。

## Global Constraints

- **只改 `.github/workflows/ci.yml`**：不碰测试代码、src/、scripts/、其它文件。
- **push/PR 串行链零改动**：现有 8 个原 job（`real-llm` / `real-llm-recall` / `real-llm-quality` / `real-llm-adversarial` / `real-llm-redline` / `real-llm-autonomy-redline` / `real-llm-conversation-judge` / `real-llm-roleplayer-calibration`）的 `if: != workflow_dispatch`、`needs:`、`strategy.matrix`、所有 steps **逐字保留**。只新增 -single job 段 + dispatch input，绝不删改原 job 任何行。
- **每个 -single job 无 needs**：dispatch 单跑时上游 skipped 会连带 needs 它的下游 skipped，故 -single job 不得声明 needs。
- **端点并发=1**：每个 -single job 单跑只起 1 runner（matrix 收敛单值），守 rsxermu 端点并发上限 2。
- **同配置**：-single job 的 env/steps 复制自对应原 job（或参照 ops-single 切 rsxermu 不限流端点的成熟做法），保证单跑与串行跑测同一套配置。
- **反过拟合**：不为单跑改任何测试逻辑、阈值、rubric。
- **YAML 合法**：每个 Task 末尾 `python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK` 必须 OK。
- **验证方式**：本地只能验 YAML 合法 + git diff 确认原 job 未动；真信号靠推送后在 Actions 页 Run workflow 逐个手动触发（CI 上验证）。

## 现成范例（实现者必读）

`real-llm-ops-single`（ci.yml:492-541）是仓内已验证的 single 范例，**所有 -single job 照它的骨架写**：
```yaml
  real-llm-ops-single:
    name: Real-LLM ops single (${{ github.event.inputs.ops_test }} / 手动单跑)
    runs-on: ubuntu-latest
    if: ${{ github.event_name == 'workflow_dispatch' && github.event.inputs.dispatch_target == 'ops' }}
    timeout-minutes: 90
    env:
      REAL_LLM_API_KEY: ${{ secrets.RSXERMU_KEY }}
    steps:
      - name: Require REAL_LLM_API_KEY (R0.1 缺 key 真 fail，不假绿)
        if: ${{ env.REAL_LLM_API_KEY == '' }}
        run: |
          echo "::error::..."
          exit 1
      - name: Checkout
        uses: actions/checkout@v4
      - name: Free disk space
        if: ${{ env.REAL_LLM_API_KEY != '' }}
        run: | ...
      - name: Install Rust toolchain
        if: ${{ env.REAL_LLM_API_KEY != '' }}
        uses: dtolnay/rust-toolchain@stable
      - name: Cache cargo registry / target
        if: ${{ env.REAL_LLM_API_KEY != '' }}
        uses: Swatinem/rust-cache@v2
      - name: cargo test ...
        if: ${{ env.REAL_LLM_API_KEY != '' }}
        env: { ... }
        run: cargo test --no-fail-fast --test <FILE> <FILTER> -- --ignored --nocapture
```
关键：每个 step 带 `if: env.REAL_LLM_API_KEY != ''`（缺 key 时只跑 Require 那条 fail，不空编译）；多族 key 的 job（autonomy-redline/conversation-judge/roleplayer-calibration/redline/adversarial 需 judge/roleplayer key）照原 job 的 env 复制对应 key + Require 守卫。

---

## Task 1: dispatch_target 扩充 + 4 个 matrix 分片 input

**Files:**
- Modify: `.github/workflows/ci.yml`（workflow_dispatch.inputs，约 :32-48）

**Interfaces:**
- Produces: dispatch_target 新增 8 个 choice 值（`smoke_single` / `recall_single` / `quality_single` / `adversarial_single` / `redline_single` / `autonomy_redline_single` / `conversation_judge_single` / `roleplayer_calibration_single`）+ 4 个 input（`recall_test` / `quality_test` / `adv_arc` / `redline_file`），供 Task 2-9 的 -single job 消费。

- [ ] **Step 1: 在 dispatch_target.options 末尾追加 8 个值**

`workflow_dispatch.inputs.dispatch_target.options`（现有 5 个：ops/roleplay_docker/roleplay_p2/reviewer_calibration/roleplay_arc）末尾追加：
```yaml
          - smoke_single
          - recall_single
          - quality_single
          - adversarial_single
          - redline_single
          - autonomy_redline_single
          - conversation_judge_single
          - roleplayer_calibration_single
```
同步把 dispatch_target 的 `description` 补一句这些新值的含义（如「*_single=单跑对应真模型 job，绕开串行链；matrix 类配套 recall_test/quality_test/adv_arc/redline_file 指定分片」）。

- [ ] **Step 2: 在 ops_test input 之后追加 4 个 matrix 分片 input**

`workflow_dispatch.inputs`（ops_test 之后）追加：
```yaml
      recall_test:
        description: '当 target=recall_single 时单跑的 recall benchmark 测试名'
        required: false
        default: 'recall_benchmark_smoke'
        type: choice
        options:
          - recall_benchmark_smoke
          - recall_benchmark_cross_industry
          - recall_benchmark_maintenance_stability
          - recall_benchmark_gap_closed_loop_trajectory
      quality_test:
        description: '当 target=quality_single 时单跑的 knowledge quality 测试名'
        required: false
        default: 'q1_retrieval_price_objection_quality'
        type: choice
        options:
          - q1_retrieval_price_objection_quality
          - q2_article_extraction_quality
          - q3_vision_extraction_quality
          - q4_chat_workstation_quality
          - q5_completeness_audit_quality
          - q6_repair_patch_quality
          - q7_tag_extraction_quality
          - q8_honest_abstention_quality
      adv_arc:
        description: '当 target=adversarial_single 时单跑的 adversarial 弧（t_judge_calibration 最慢 70-120min）'
        required: false
        default: 't_adv_human_takeover_bait'
        type: choice
        options:
          - t_adv_human_takeover_bait
          - t_judge_calibration
          - t_adv_price_objection
          - t_adv_contradiction_trap
          - t_adv_fake_emotion_bait
          - t_adv_knowledge_fabrication_bait
          - t_adv_prompt_injection
          - t_longrun_capability
      redline_file:
        description: '当 target=redline_single 时单跑的红线门测试文件（cross_domain_arc=G8 身份探针）'
        required: false
        default: 'real_llm_cross_domain_arc'
        type: choice
        options:
          - real_llm_cross_domain_arc
          - real_llm_principal_channel
          - real_llm_proactive_outreach
          - real_llm_dynamic_adversarial
          - real_llm_digital_twin_arc
          - real_llm_principal_relay
```

- [ ] **Step 3: 校验 YAML + 提交**

Run: `python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK`
Expected: YAML-OK

```bash
git add .github/workflows/ci.yml
git commit -m "ci(dispatch): workflow_dispatch加8个*_single target+4个matrix分片input

为真模型job独立单跑入口铺路:dispatch_target choice加smoke/recall/quality/
adversarial/redline/autonomy_redline/conversation_judge/roleplayer_calibration
共8个_single值;加recall_test/quality_test/adv_arc/redline_file四个分片input
(默认值取各matrix最相关/最轻片)。供后续-single job消费。"
```

---

## Task 2: 4 个非-matrix -single job（smoke / autonomy-redline / conversation-judge / roleplayer-calibration）

**Files:**
- Modify: `.github/workflows/ci.yml`（在各原 job 之后新增对应 -single job）

**Interfaces:**
- Consumes: Task 1 的 dispatch_target 值（smoke_single / autonomy_redline_single / conversation_judge_single / roleplayer_calibration_single）。
- Produces: 4 个新 job：`real-llm-smoke-single` / `real-llm-autonomy-redline-single` / `real-llm-conversation-judge-single` / `real-llm-roleplayer-calibration-single`。

**复制配方（关键，避免逐字重抄漂移）**：每个 -single job = 复制对应原 job 的**整段 YAML**（job 名行到最后一个 step），然后只改下面列出的几处。原 job 的 env、所有 step（Require/Checkout/Free disk/Rust/Cache/cargo test/Upload）、各 step 的 `if: env.* != ''` 守卫、`timeout-minutes` **逐字保留**。原 job 本身不动（push 串行链）。

### 2a. real-llm-smoke-single

原 job：`real-llm`（ci.yml:160 起）。复制整段，改 4 处：
1. job key：`real-llm` → `real-llm-smoke-single`
2. `name:` 末尾加「 (手动单跑)」
3. `if:` → `${{ github.event_name == 'workflow_dispatch' && github.event.inputs.dispatch_target == 'smoke_single' }}`（原为 `!= 'workflow_dispatch'`）
4. 无 `needs:`（原 job 本就无 needs，确认即可）

原 smoke job 有 3 个 cargo test step（real_llm_smoke / domain_profile_e2e / real_llm_knowledge）——**全部保留**（单跑即完整复现 smoke 套件）。env 含 `REAL_LLM_LEDGER: target/real_llm_ledger/smoke` 保留。

- [ ] **Step 1: 复制 real-llm job 整段 → real-llm-smoke-single，改上述 4 处**（放在 real-llm-ops-single 之后或 real-llm job 之后，位置不影响语义）

- [ ] **Step 2: 校验 YAML**

Run: `python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK`
Expected: YAML-OK

### 2b. real-llm-autonomy-redline-single

原 job：`real-llm-autonomy-redline`（ci.yml:1136 起）。复制整段，改 4 处：
1. job key → `real-llm-autonomy-redline-single`
2. `name:` 末尾加「 (手动单跑)」
3. `if:` → `${{ github.event_name == 'workflow_dispatch' && github.event.inputs.dispatch_target == 'autonomy_redline_single' }}`
4. **删 `needs: real-llm-redline` 行**

env 双族（REAL_LLM_* + REAL_LLM_JUDGE_*）+ `REAL_LLM_LEDGER: target/real_llm_ledger/autonomy-redline` 逐字保留。test step `cargo test --no-fail-fast --test real_llm_autonomy_redline -- --ignored --nocapture` 保留。

- [ ] **Step 3: 复制 real-llm-autonomy-redline 整段 → -single，改上述 4 处（删 needs）**

### 2c. real-llm-conversation-judge-single

原 job：`real-llm-conversation-judge`（ci.yml:1206 起）。复制整段，改 4 处：
1. job key → `real-llm-conversation-judge-single`
2. `name:` 末尾加「 (手动单跑)」
3. `if:` → `... dispatch_target == 'conversation_judge_single' }}`
4. **删 `needs: real-llm-autonomy-redline` 行**

env 双族 + `REAL_LLM_LEDGER: target/real_llm_ledger/conversation-judge` 逐字保留。test step `cargo test --test real_llm_conversation_judge` 保留。

- [ ] **Step 4: 复制 real-llm-conversation-judge 整段 → -single，改上述 4 处（删 needs）**

### 2d. real-llm-roleplayer-calibration-single

原 job：`real-llm-roleplayer-calibration`（ci.yml:1277 起；含 Task C 刚补的 job 级 `ROLEPLAYER_API_KEY` + `Require ROLEPLAYER_API_KEY` step）。复制整段，改 4 处：
1. job key → `real-llm-roleplayer-calibration-single`
2. `name:` 末尾加「 (手动单跑)」
3. `if:` → `... dispatch_target == 'roleplayer_calibration_single' }}`
4. **删 `needs: real-llm-conversation-judge` 行**

env 三族（job 级 REAL_LLM_API_KEY + ROLEPLAYER_API_KEY；test step 的 REAL_LLM_JUDGE_* + ROLEPLAYER_BASE_URL/MODEL）+ 两个 Require 守卫 + 各 step 的 `&& env.ROLEPLAYER_API_KEY != ''` 复合守卫**逐字保留**（这些是 Task C 刚修的，照搬即可）。`REAL_LLM_LEDGER: target/real_llm_ledger/roleplayer-calibration` 保留。

- [ ] **Step 5: 复制 real-llm-roleplayer-calibration 整段 → -single，改上述 4 处（删 needs）**

- [ ] **Step 6: 校验 YAML + 确认原 4 job 未动 + 提交**

Run:
```
python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK
git diff --unified=0 | grep -E "^-" | grep -vE "^---" | head   # 应只有极少删除行(若有),原job行不该出现在删除里
```
Expected: YAML-OK；新增 4 个 -single job，原 real-llm / autonomy-redline / conversation-judge / roleplayer-calibration 的行未被删改（diff 只增不删原 job 内容）。

```bash
git add .github/workflows/ci.yml
git commit -m "ci(dispatch): 加4个非matrix真模型job的-single单跑入口

smoke/autonomy-redline/conversation-judge/roleplayer-calibration各加-single孪生job
(复制原job env/steps,if=dispatch&&target==xxx_single,删needs)。原job push串行链
逐字不动。autonomy/conversation双族key、roleplayer三族key+Require守卫照搬。"
```

---

## Task 3: 4 个 matrix -single job（recall / quality / adversarial / redline）

**Files:**
- Modify: `.github/workflows/ci.yml`（在各原 matrix job 之后新增对应 -single job）

**Interfaces:**
- Consumes: Task 1 的 dispatch_target 值（recall_single / quality_single / adversarial_single / redline_single）+ 分片 input（recall_test / quality_test / adv_arc / redline_file）。
- Produces: 4 个新 job：`real-llm-recall-single` / `real-llm-quality-single` / `real-llm-adversarial-single` / `real-llm-redline-single`。

**matrix 收敛配方**：复制对应原 matrix job 整段，除 Task 2 那 4 处通用改动（job key 加 -single / name 加「 (手动单跑)」/ if 改 dispatch+target / 删 needs）外，**额外把 `strategy.matrix.<key>` 从全分片列表改成读 input 的单值数组**：

| -single job | 原 job | matrix 改法 | test step run 命令（不变，靠 matrix 注入） |
|---|---|---|---|
| recall-single | real-llm-recall | `matrix.t: ["${{ github.event.inputs.recall_test }}"]` | `cargo test ... --test real_llm_recall_benchmark ${{ matrix.t }} ...` |
| quality-single | real-llm-quality | `matrix.q: ["${{ github.event.inputs.quality_test }}"]` | `cargo test ... --test real_llm_knowledge_quality ${{ matrix.q }} ...` |
| adversarial-single | real-llm-adversarial | `matrix.arc: ["${{ github.event.inputs.adv_arc }}"]` | `cargo test ... --test real_llm_adversarial ${{ matrix.arc }} ...` |
| redline-single | real-llm-redline | `matrix.file: ["${{ github.event.inputs.redline_file }}"]` | `cargo test ... --test ${{ matrix.file }} ...` |

`strategy.fail-fast` / `max-parallel` 保留（单值时无实际影响，留着不破坏结构）。各原 job 的 env（含 adversarial 的 judge2 NVIDIA key + 「不配 JUDGE_API_KEY 防 failover」注释、redline 的 judge env、各自 `REAL_LLM_LEDGER` 子目录）、Require 守卫、所有 step 逐字保留。

> 注意 adversarial 原 job 的 `timeout-minutes: ${{ matrix.arc == 't_judge_calibration' && 120 || 90 }}` 表达式——复制时保留原样（单跑 t_judge_calibration 时仍给 120min）。

- [ ] **Step 1: 复制 real-llm-recall 整段 → real-llm-recall-single**，改通用 4 处 + `matrix.t: ["${{ github.event.inputs.recall_test }}"]`

- [ ] **Step 2: 复制 real-llm-quality 整段 → real-llm-quality-single**，改通用 4 处 + `matrix.q: ["${{ github.event.inputs.quality_test }}"]`

- [ ] **Step 3: 复制 real-llm-adversarial 整段 → real-llm-adversarial-single**，改通用 4 处 + `matrix.arc: ["${{ github.event.inputs.adv_arc }}"]`（保留 timeout 表达式）

- [ ] **Step 4: 复制 real-llm-redline 整段 → real-llm-redline-single**，改通用 4 处 + `matrix.file: ["${{ github.event.inputs.redline_file }}"]`

- [ ] **Step 5: 校验 YAML + 确认原 4 matrix job 未动 + 提交**

Run: `python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo YAML-OK`
Expected: YAML-OK；新增 4 个 -single job，原 recall/quality/adversarial/redline 行未删改。

```bash
git add .github/workflows/ci.yml
git commit -m "ci(dispatch): 加4个matrix真模型job的-single单跑入口

recall/quality/adversarial/redline各加-single孪生job(matrix收敛到input单值:
recall_test/quality_test/adv_arc/redline_file),if=dispatch&&target==xxx_single,删
needs。原job env(含adversarial judge2/防failover、各自LEDGER子目录)、Require守卫、
timeout表达式逐字照搬。原push串行链不动。"
```

---

## Task 4: 全量验证 + 文档同步

**Files:**
- Modify: `.github/workflows/ci.yml`（仅校验，不改）

- [ ] **Step 1: YAML 合法 + job 总数核对**

Run:
```
python -c "import yaml,collections; d=yaml.safe_load(open('.github/workflows/ci.yml')); j=list(d['jobs']); print('jobs:',len(j)); print('single:',[x for x in j if x.endswith('-single')])"
```
Expected: jobs 总数 = 原数 + 8；`-single` job 列表含全部 8 个新 job（real-llm-smoke-single / real-llm-recall-single / real-llm-quality-single / real-llm-adversarial-single / real-llm-redline-single / real-llm-autonomy-redline-single / real-llm-conversation-judge-single / real-llm-roleplayer-calibration-single）+ 原有 real-llm-ops-single。

- [ ] **Step 2: 确认 8 个原 job 的 if/needs 未被改动**

Run:
```
grep -nE "^  real-llm(-recall|-quality|-adversarial|-redline|-autonomy-redline|-conversation-judge|-roleplayer-calibration)?:" .github/workflows/ci.yml
git diff <Task1之前的commit>..HEAD -- .github/workflows/ci.yml | grep -E "^-" | grep -vE "^---|^-          - (ops|roleplay)" | head
```
Expected: 8 个原 job 仍在、`if: != workflow_dispatch` 未变；diff 的删除行里没有原 job 的 if/needs/matrix/steps（只增不删原 job 内容）。

- [ ] **Step 3: 确认每个 -single job 无 needs**

Run: `python -c "import yaml; d=yaml.safe_load(open('.github/workflows/ci.yml')); [print(k,'needs:',v.get('needs')) for k,v in d['jobs'].items() if k.endswith('-single')]"`
Expected: 全部 -single job 的 needs 为 None。

- [ ] **Step 4: 提交（若 Step 1-3 有微调）**

若前面 Task 已全部提交且本 Task 仅校验无改动，跳过提交。否则：
```bash
git add .github/workflows/ci.yml
git commit -m "ci(dispatch): 真模型job单跑入口全量校验(YAML/job数/原job未动/无needs)"
```

---

## Self-Review

**1. Spec coverage**（对照 spec docs/superpowers/specs/2026-06-21-ci-single-dispatch-real-llm-jobs-design.md）：
- spec §3.2 dispatch_target 加 8 值 → Task 1 Step 1 ✓
- spec §3.3 4 个 matrix 分片 input → Task 1 Step 2 ✓（默认值与 spec 表一致）
- spec §3.4 非-matrix 4 job 复制规则 → Task 2 ✓
- spec §3.4 matrix 4 job 收敛单值 → Task 3 ✓
- spec §五 验证（YAML/job 数/原 job 未动/无 needs） → Task 4 ✓
- 全覆盖。

**2. Placeholder scan**：Task 2/3 用「复制配方 + 改 N 处」而非逐字重抄 8 段 YAML——这是有意 DRY（原 job env 是唯一真相源，重抄易漂移），每处改动都给了精确的「改哪几处、改成什么」，非占位。无 TBD/TODO。

**3. Type consistency**：dispatch_target 的 8 个值（Task 1）与 Task 2/3 各 job 的 `if ... == 'xxx_single'` 逐一对应；4 个 input 名（recall_test/quality_test/adv_arc/redline_file，Task 1）与 Task 3 matrix 收敛引用一致；job key 命名 `real-llm-<x>-single` 全表统一。

无 issue。
