# 通用化能力审查修复批次 — 设计 spec

日期：2026-06-20
基线 HEAD：`ca18ba4`（main）
来源：通用化能力全面交叉审查终审报告（`.git/sdd/universal-audit/FINAL-REPORT.md`）
流程：13 域深审 → 去重 35 组 → 36 agent 主动证伪 → 综合终审（无 Critical/High，13 初判 High 全降级）

## 范围

用户拍板：**4 必修 Medium + 全部可缓 Low + 文档订正**。数字分身「口吻分化」大工程不在本批（单独立项，需先 brainstorming）。

测试策略（用户拍板）：纯函数/lib 单测进 baseline 硬门（不依赖 Docker，符合磁盘纪律）；需 DB 的场景写 `#[ignore]` 留 CI；**顺手修 G09**——把 c2/domain_profile_e2e 的 DB 层断言拆纯函数版进 baseline。

红线全程守：DEFAULT 销售域字节等价、serde 向后兼容、AI 永不自动 verify、不造双真相源、无人工接管、反过拟合、boundary_protection 不被 profile 放宽。磁盘纪律：`rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`，本地只跑 `cargo test --lib` + 单 PBT，绝不 `cargo build --tests`，集成留 CI。

## 已核实的代码事实（修正终审报告的两处偏差）

- **G21 是 4 个调用点**（终审说 2 个）：`contacts.rs:392`（create，手握 `admin.current_workspace`+contact）、`contacts.rs:485`（profile-note，同）、`contacts.rs:754`（手握 contact）、`management.rs:744`（MCP enable_contact_agent，手握 `workspace_id` 变量）。`build_initial_operation_profile`（decision.rs:32）内 4 处用 `state.config.default_workspace_id`（:41 domain_config / :54 active_profile / :63 system prompt / :69 task prompt）。
- **G07 参数化版本已就绪**：`classify_outcome_label_with_polarity`（gap_signals.rs:686）+ 测试已存在。`compute_negative_reaction_rate`（post_release.rs:351）签名已带 `workspace_id`，但函数体 :358 import 裸 `classify_outcome_label`、:382 调它（写死销售极性），doc :346-347 却假声称「自动跟随 profile」。
- **G06 两个直编路由**都 `$set` state_machine 不派 policy：`update_operation_domain`（domains.rs:87-128，:119）、`update_operation_domain_state_machine`（domains.rs:140-168，:160）。对比 activate 路径已走 `publish_state_machine_version` 联动派生。
- **G13 单一收口点**：`apply_profile_threshold_overrides`（runtime.rs:225-245）五字段直接赋值无 clamp，所有写路径都过它。
- **G01**：`review_passed`（gates.rs:20-33）:28 grounding 项无条件、无 bypass 分支；对比 `classify_dual_gate`:120-124 有 `!bypass || claim_requires_product_knowledge` 守卫。

## 修复分组与方案

### A 组：逻辑修复（改运行时行为，每条配测试）

**G21（Medium）多租户首屏画像取错 workspace**
- `build_initial_operation_profile` 增 `workspace_id: &str` 参数（放在 `state` 后）；4 处 `default_workspace_id` 改用之。
- 4 调用点传真实 workspace：contacts.rs 三处传 `&admin.current_workspace`，management.rs:744 传 `workspace_id`。
- 测试：调用点签名编译强制；real_llm_ops_smoke.rs 三处调用（:1688/:2523 + 1 处）需补 workspace 参数（编译强制，传 `default_workspace_id` 保持单测语义）。
- 红线：单租户下 `current_workspace`==`default_workspace_id`，行为不变。

**G13（Medium）五闸阈值无 clamp**
- `apply_profile_threshold_overrides`（runtime.rs:230-244）五字段赋值前 clamp 到 `1..=10`。优先复用已有五闸 min/max 常量（如有 `FIVE_GATE_HARD_MIN/MAX`，否则定义 `1..=10`）。
- 测试：纯函数单测——override=100→clamp 10，override=0→1，override=None→不动（DEFAULT 字节等价）。

**G01（Medium）grounding bypass 漏加在 review_passed**
- `review_passed`（gates.rs:28）grounding 项改为与 classify_dual_gate:120-124 对齐：`(runtime.grounding_gate_bypass_without_claim && !claim_requires_product_knowledge(&review.claim_analysis)) || review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below`。
- 注意 review_passed 当前签名只收 `(review, runtime)`，claim_analysis 在 review 里——确认 `claim_requires_product_knowledge` 可从 review.claim_analysis 取（classify_dual_gate 就这么用）。
- 测试：纯函数单测——bypass=true+无产品声明+grounding<阈值→review_passed=true；bypass=false（DEFAULT）→不变（字节等价）；bypass=true 但有产品声明+grounding<阈值→仍 false。

**G06（Medium）直编路由不派生 policy**
- 抽 forbidsProactive→`derive_state_policy_lists`→`operation_state_policies` 的派生逻辑为共享 helper（admin_ops_versions.rs 里 publish_state_machine_version 已有此逻辑，提取复用）。
- `update_operation_domain`（domains.rs:119 $set 后）和 `update_operation_domain_state_machine`（:160 $set 后）调共享 helper 重派 policy。
- best-effort：派生失败 warn 不阻断（与 publish 一致）。
- 测试：#[ignore] 集成（testcontainers）——直编路由 PUT 带 forbidsProactive:true 的 state → 断言 operation_state_policies 有对应 active 行。

**G07（Medium）负反应率未 profile 化**
- `compute_negative_reaction_rate`（post_release.rs:358/382）：加载该 workspace active profile → resolve 极性 → 改调 `classify_outcome_label_with_polarity`。复用回路① 同源加载方式。
- 订正 :346-347 假声明注释。
- 测试：纯函数/lib 单测——自定义负极（如 user_went_cold）在 profile 极性下被识别为 Block。
- 注意：auto_release.rs:88 也调此函数（同极性源），确认两处一致。

### B 组：一致性/健壮性 Low

**G31** RISKY_FIELD_NAMES（domain_profiles.rs:685）增 `reviewer_orientation`/`mode_gate_policy_override`；加测试断言真 prompt（prompts.rs:1325 / review/mod.rs:367）含锚常量（domain_profile.rs:487-494）。

**G03** renewal/reactivation 短路（planner/mod.rs:1763-1765/1953-1955）条件改为「profile 默认开 OR per_relationship 任一开 OR contact override 开」再放行。

**G04** `effective_quiet_hours_enabled`（quiet_hours.rs:94-103）改经 resolve_operation_mode 读 profile 级 quiet_hours；gateway/webhooks 两消费点传 profile。

**G08+G32** camelCase 归一 data-loss：仿 stateMachine 在 normalize 前 remove `businessFormulas` 单独处理（保 camelCase）或加 serde alias；`coerce_scalar_string_fields` 扩到嵌套 `profileDimensions[].description`。

**G11/G12** publish no-op 短路前加 policy 行存在性 reconcile；rollback/rollout（admin_ops_versions.rs:445-498）切机器后复用 G06 共享 helper 重派 policy。

**G16** 去掉 gateway.rs:1043 双 upsert（decision.rs:711 路径已覆盖）。

**G24** review finalize 出站正文复用 `passes_forbidden_words` 做运行期 fail-closed 校验（纵深加固红线⑤，profile override/正文逃逸静态 lint 的缺口）。

### C 组：CI/测试硬门

**G09** 把 c2_state_transition_cross_domain / domain_profile_e2e 的 DB 层断言拆出**纯函数版本**进 baseline 硬门（不依赖 Docker）。原 #[ignore] 集成测试保留留 CI。

**G10** redline job（ci.yml:1026-1095）补 `Require ROLEPLAYER_API_KEY`，与 roleplay-arc（ci.yml:963-967）对齐。

### D 组：纯文档/注释订正（零代码风险）

**G18** decision.rs:428 「字节等价」改「内容/语义等价（段从中部移末尾）」。

**G05** domain_profile.rs:910/921 删/订正「口吻最像本人/各异」不可达注释，指明数字分身口吻分化是独立专题。

## 实施方式

- subagent-driven（每条/每组 implementer + 独立 reviewer，model:opus）。
- 推进顺序：A 组（功能正确性，优先）→ B 组 → C 组 → D 组。
- 组内独立项并行，碰同文件的串行（如 G06/G11/G12 都碰 admin_ops_versions.rs policy 派生，合并为「policy 派生一致性」子任务串行做）。
- 每组完成跑 `cargo test --lib` baseline；全批完成跑完整 merge gate（lib≥350/0 + 四 PBT≥33/0 + check-no-human-takeover clean + -Dwarnings）。
- 提交边界：精确 `git add` 命名文件，排除并行产物（tests/real_llm_*、tests/roleplay_*、.kiro/、AGENTS.md、agent_t*.txt、t15_single.txt 等）。
- 推送/合并需用户显式授权。

## 模块边界（隔离与清晰）

- G21 改 `build_initial_operation_profile` 签名 → 影响 3 个生产调用点 + 3 个测试调用点，编译强制全改。
- G13 收口在 `apply_profile_threshold_overrides` 单点。
- G01 收口在 `review_passed` 单函数。
- G06/G11/G12 共享一个 policy 派生 helper（抽到 admin_ops_versions.rs），三处调用点（2 直编路由 + rollback）+ no-op reconcile。
- G07 收口在 `compute_negative_reaction_rate`，与回路① 同源。
- 各组互不耦合，可独立实现+测试+审查。

## 不做（明确排除）

- 数字分身口吻分化（per_relationship soul/tone/voice + decision.rs relationship_type 路由 + 引导 schema + 前端 UI）——独立大工程专题。
- 已 REFUTED 项（G17/G25/G26）、INTENDED 项（G30/G33/G35）——无需动。
- G14（perRelationship 前端/引导显形）——B 阶段预期状态，属能力缺口非缺陷。
- G34（APP_ENV 守卫生产隐患）——已知 MEMORY 项，留生产部署测试验证。
