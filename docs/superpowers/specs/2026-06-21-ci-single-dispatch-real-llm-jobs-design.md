# CI 真模型 job 独立单跑入口 设计

> 让每一个 LLM 驱动的真模型测试 job 都能通过 `workflow_dispatch` 独立手动触发，绕开导致 GitHub Actions 总时长超时被 cancelled 的串行 needs 长链。只改 `.github/workflows/ci.yml`，push/PR 串行链零改动。

## 一、问题（要根治什么）

PR #28 的 CI run 连续两次 `cancelled`：8 个真模型 job 串成一条长 needs 链（`real-llm`(smoke) → `real-llm-recall` → `real-llm-ops` → `real-llm-quality` → `real-llm-adversarial` → `real-llm-redline` → `real-llm-autonomy-redline` → `real-llm-conversation-judge` → `real-llm-roleplayer-calibration`，且多个是 matrix），`max-parallel:1` 串行守 rsxermu 端点并发上限 2。整条链跑近 4 小时，撞 GitHub Actions 总时长上限被平台取消（cancelled 分片日志停在 rustup 安装/排队阶段，未进编译）。

后果：想验证某一个 job（如本轮 5-Blocking 修复涉及的 autonomy-redline / conversation-judge / roleplayer-calibration / redline 的 cross_domain_arc 分片）必须等整条链，而链常在末环超时——**末环 job 根本没机会跑**。需要「逐个单独触发某个真模型 job」的能力。

## 二、目标与边界

**目标**：让**每一个** LLM 驱动的真模型 job 都能独立 `workflow_dispatch` 单跑，互不依赖、不排长队、端点并发天然=1。

**现状盘点（11 个真模型 job）**：
- 已有 dispatch 单跑入口（5 个，不动）：`real-llm-ops-single`（target=ops + `ops_test`）、`roleplay-docker`、`roleplay-p2`、`reviewer-calibration`、`roleplay-arc`。
- **无单跑入口（8 个，本设计补）**：`real-llm`(smoke)、`real-llm-recall`(matrix.t)、`real-llm-quality`(matrix.q)、`real-llm-adversarial`(matrix.arc)、`real-llm-redline`(matrix.file)、`real-llm-autonomy-redline`、`real-llm-conversation-judge`、`real-llm-roleplayer-calibration`。

**边界**：
- **只改 ci.yml**：不碰测试代码、不碰 src/、不碰 scripts/。
- **push/PR 串行链零改动**：现有 8 个 job 的 `if: != workflow_dispatch` + needs 链 + max-parallel:1 完全保留。push/PR 行为与现在逐字一致（零风险）。
- **不破坏端点并发语义**：每个 -single job 单跑只起 1 个 runner（matrix 收敛到单值），端点并发天然=1，守 rsxermu 上限 2 无虞。
- **反过拟合/不点对点**：复用各原 job 的真实 env/steps（同一套配置），不为单跑改测试逻辑或阈值。

## 三、设计

### 3.1 架构（镜像现成 roleplay-arc 范式，加独立 -single job）

GitHub Actions 机制约束（已确认）：`needs` 是静态声明，dispatch 单跑时上游 job 因 `if` 不满足而 skipped，则下游 `needs` 它的 job **连带 skipped**。故单跑 job 必须**不依赖会被跳过的上游**——即无 needs、自带完整 steps。现成范式 `roleplay-arc`（ci.yml:955）正是：`if: workflow_dispatch && dispatch_target=='roleplay_arc'`、无 needs、自带 env + steps。

为 8 个无入口 job 各加一个 `-single` 孪生 job：
- 原 job **完全不动**（push/PR 串行链保留）。
- 新 `<job>-single` job：`if: github.event_name=='workflow_dispatch' && inputs.dispatch_target=='<x>_single'`、**无 needs**、env+steps 复制自原 job。
- matrix job 的 strategy.matrix 从「全分片列表」改为「读 dispatch input 的单值数组」，单跑只起 1 个分片。

### 3.2 dispatch_target choice 扩充

`workflow_dispatch.inputs.dispatch_target` 的 `options` 在现有 5 个（ops/roleplay_docker/roleplay_p2/reviewer_calibration/roleplay_arc）基础上加 8 个：

```
- smoke_single
- recall_single
- quality_single
- adversarial_single
- redline_single
- autonomy_redline_single
- conversation_judge_single
- roleplayer_calibration_single
```

### 3.3 matrix 分片 input（4 个 matrix job）

仿现有 `ops_test` input，为 4 个 matrix job 各加一个可选 input，默认值取「本轮最相关 / 最轻」的分片：

| input 名 | 用于 | 默认值 | 可选值（原 matrix 全集） |
|---|---|---|---|
| `recall_test` | recall_single | `recall_benchmark_smoke` | recall_benchmark_smoke / recall_benchmark_cross_industry / recall_benchmark_maintenance_stability / recall_benchmark_gap_closed_loop_trajectory |
| `quality_test` | quality_single | `q1_retrieval_price_objection_quality` | q1_retrieval_price_objection_quality / q2_article_extraction_quality / q3_vision_extraction_quality / q4_chat_workstation_quality / q5_completeness_audit_quality / q6_repair_patch_quality / q7_tag_extraction_quality / q8_honest_abstention_quality |
| `adv_arc` | adversarial_single | `t_adv_human_takeover_bait` | t_judge_calibration / t_adv_price_objection / t_adv_human_takeover_bait / t_adv_contradiction_trap / t_adv_fake_emotion_bait / t_adv_knowledge_fabrication_bait / t_adv_prompt_injection / t_longrun_capability（t_judge_calibration 最慢 70-120min，非默认） |
| `redline_file` | redline_single | `real_llm_cross_domain_arc` | real_llm_cross_domain_arc / real_llm_principal_channel / real_llm_proactive_outreach / real_llm_dynamic_adversarial / real_llm_digital_twin_arc / real_llm_principal_relay（cross_domain_arc=G8 身份探针） |

全部默认值与可选值均为 ci.yml 现有 matrix 列表里的真实合法值（已对照 :315 recall.t / :572 quality.q / :689 adversarial.arc / :1057 redline.file）。

### 3.4 每个 -single job 的构造规则

**4 个非-matrix job**（smoke / autonomy-redline / conversation-judge / roleplayer-calibration）：复制原 job 整体，改 3 处：
1. job 名加 `-single`（如 `real-llm-autonomy-redline-single`）。
2. `if:` → `${{ github.event_name == 'workflow_dispatch' && github.event.inputs.dispatch_target == '<x>_single' }}`。
3. 删 `needs:` 行。
env、steps（Require 守卫 / Free disk / Rust toolchain / Cache / cargo test / Upload ledger）逐字复制原 job，保证单跑与串行跑同一配置。各 step 内 `if: env.REAL_LLM_API_KEY != ''` 等守卫照搬。

**4 个 matrix job**（recall / quality / adversarial / redline）：同上 3 改，外加：
4. `strategy.matrix.<key>` 从全分片列表改为 `["${{ github.event.inputs.<input> }}"]` 单值数组（如 redline-single 的 `matrix.file: ["${{ github.event.inputs.redline_file }}"]`）。`max-parallel` 保留 1（单值无所谓，留着无害）。

> Upload-artifact 的 `name` 在 -single job 里可与原 job 同名或加 `-single` 后缀——单跑场景不进 skip-gate 汇总（skip-gate `if: != workflow_dispatch` 不在 dispatch 时跑），不影响 G1 的跨 job 求和逻辑。实现期保持与原 job 一致的 artifact 名即可（dispatch 单跑各 run 独立，无覆盖问题）。

### 3.5 ledger / REAL_LLM_LEDGER 子目录

原 job 经 Task A（G1 修复）已设带子目录的 `REAL_LLM_LEDGER`（如 `target/real_llm_ledger/redline-${{ matrix.file }}`）。-single job 复制时，matrix 收敛到单值后该路径仍有效（如 `redline-real_llm_cross_domain_arc`）。dispatch 单跑不跑 skip-gate（它 `if: != workflow_dispatch`），ledger 仅作 artifact 供人工查看 `--nocapture` 输出。保持原 env 不变即可。

## 四、落地（ci.yml 改动清单）

| 改动 | 位置 |
|---|---|
| `dispatch_target.options` 加 8 个 `*_single` 值 | workflow_dispatch.inputs（约 :39-44） |
| 加 4 个 matrix 分片 input（recall_test/quality_test/adv_arc/redline_file），各带 description+default | workflow_dispatch.inputs（ops_test 旁，约 :45-48） |
| 加 `real-llm-smoke-single` job（复制 real-llm:160，无 needs，if=smoke_single） | real-llm job 后 |
| 加 `real-llm-recall-single`（matrix.t→recall_test 单值） | real-llm-recall 后 |
| 加 `real-llm-quality-single`（matrix.q→quality_test 单值） | real-llm-quality 后 |
| 加 `real-llm-adversarial-single`（matrix.arc→adv_arc 单值） | real-llm-adversarial 后 |
| 加 `real-llm-redline-single`（matrix.file→redline_file 单值） | real-llm-redline 后 |
| 加 `real-llm-autonomy-redline-single`（无 needs，if=autonomy_redline_single） | real-llm-autonomy-redline 后 |
| 加 `real-llm-conversation-judge-single` | real-llm-conversation-judge 后 |
| 加 `real-llm-roleplayer-calibration-single` | real-llm-roleplayer-calibration 后 |

## 五、验证

- **YAML 合法**：`python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` 通过；job 总数 = 原 + 8。
- **push/PR 零改动**：现有 8 个原 job 的 if/needs/matrix/steps 逐字未变（diff 只新增 -single job 段 + dispatch input，不删改原 job 行）。可用 `git diff` 确认原 job 区段无改动。
- **dispatch 单跑真信号（推送后逐个手动触发验证）**：在 Actions 页 Run workflow → 选 `autonomy_redline_single` → 该 job 独立起跑、无 needs 阻塞、跑完出 Breach/Clean/Skipped；同理逐个验 conversation_judge_single（G3）、roleplayer_calibration_single（G7）、redline_single + redline_file=real_llm_cross_domain_arc（G8）。
- **本轮 5-Blocking 验证闭环**：通过上述单跑，逐个确认 G2/G3（autonomy-redline + conversation-judge 的 redlineHeld/overall_progress 门）、G7（roleplayer 缺 key 守卫）、G8（身份探针弧末门）的真信号——不再受串行链超时阻塞。

## 六、与其它工作的关系

- 不改任何测试代码或阈值——纯 CI 触发设施扩充，与 5-Blocking 修复（已 push、Baseline/Integration/smoke 已绿）正交。
- 解除「真模型 job 只能整链跑、链超时即全 skipped」的运维约束，让任何单个真模型能力今后都能独立快速验证。
