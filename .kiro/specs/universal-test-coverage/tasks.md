# 通用化后测试体系重建 — 任务清单

> 配套 `requirements.md`，**以其「阶段顺序与里程碑」为权威**。两条并行线：**固定回归线 R0→R1→R2→R2.5→R3→R4（PR 门，先做，守下限，t15-18 不退役）** + **动态发现线 R5（nightly/手动，后做，不进 PR 门）**。每阶段独立提交。

## R0 · P0 总开关（地基，先做）

- [ ] **R0.1** CI 缺 key 即真 fail（**先除字面量回落，否则断言恒真等于没做**）
  - 真根因：CI 现有 17 处 `${{ secrets.X || 'nvapi-...硬编码字面量' }}`（`ci.yml:176/215/...`）——缺 secret 也回落明文 key 假装能跑。**先删全部 `|| 'nvapi-...'` 字面量回落**（顺带：明文 key 提交进仓库 17 次=机密泄露，须轮换该 NVIDIA key）。
  - 落点：删字面量回落 → 各 real-llm job 加前置 step：`REAL_LLM_API_KEY` 空则 `exit 1`。
  - 验收：清空 secret 时 job 红（删字面量后断言才有意义）；有 key 正常。
- [ ] **R0.2** transient-skip 可观测化
  - 落点：`unwrap_or_skip_transient!` 宏（各 real_llm 文件）skip 时写计数到 ledger 文件；新增 CI step 校验 skip 率 ≤ 阈值。
  - 验收：高 skip 场景 CI 红；正常绿。
  - 注意：宏在多个文件各有一份，改动要一致（或抽公共）。
- [ ] **R0.3** judge/reviewer 4xx 不当抖动吞
  - 落点：`is_failover_worthy` / `unwrap_or_skip_transient!` 对 `http_4xx`（除 401/402）改为不 skip → 直接 panic/fail。
  - 验收：端点漏 /v1 等配错时测试红，不再 skip 绿。
- [ ] **R0 验证**：本地 `cargo test --lib` 不回归 + 推送后真模型 CI 跑一轮确认 R0.1-R0.3 生效。

## R1 · judge profile 化（横切）

- [ ] **R1.1** judge 标尺从 profile 派生
  - 落点：各真模型测试的 judge system prompt 构造（`real_llm_ops_smoke.rs:530-542` 等）→ 抽一个 `build_judge_system(profile)`，维度/锚点从 `business_formulas`+`coverage_dimensions` 生成。
  - 设计：销售 profile→现有标尺（基准）；情感 profile→情绪承接/边界标尺。复用 DomainProfile 配置，不另写。
  - 验收：同条情感回复，profile judge 不因「没成交」误判；两域标尺确有差异（CI 日志对比）。
- [ ] **R1.2** judge 失败语义分级
  - 落点：judge 调用处——唯一质量门的 judge 失败 fail；红线测试 judge 失败仅丢观测。
  - 验收：judge 全失败时质量门测试红、红线测试仍按红线判。
- [ ] **R1 验证**：真模型 CI 跑销售+情感两域，确认标尺差异化且分数合理。

## R2 · 跨域全链闭环 + 随机身份生成器（主体）

- [ ] **R2.1** LLM 随机身份/场景生成器
  - 落点：新建 `tests/common/identity_generator.rs`（或测试内 helper）：LLM 生成 {行业,性格,诉求} → DomainProfile + Contact + 首轮 inbound。seed 可控可复现。
  - 验收：一次跑覆盖 N 随机身份，≥4 大类；可复现。
- [ ] **R2.2** 全链长程闭环测试骨架
  - 落点：新建 `tests/real_llm_cross_domain_arc.rs`（复用 ops_smoke 基础设施）。多轮弧 + 闭环断言：画像更新/记忆固化/承诺任务/状态迁移/Planner 主动触达/冷启动。
  - 断言对齐 `docs/universal-domain-test-gap-audit.md` §3.5 业务契约表。
  - 验收：每条断言能在行为错误时变红；闭环各环节真实达标。
- [ ] **R2.3** 跨域行为差异断言
  - 落点：同输入对立 profile → 行为实质不同（非逐字不等，用业务维度度量）。
  - 验收：销售推进 vs 陪伴承接 vs 同行互惠 可区分。
- [ ] **R2 验证**：真模型 CI 跑随机身份矩阵，人读日志确认业务行为正确。

## R2.5 · 自运营主动半场 + 治理红线（t4-t18 全新维度）

- [ ] **R2.5.1** 作息门控真模型业务流
  - 落点：新测试（可入 `real_llm_cross_domain_arc.rs` 或新建）。构造静默时段 inbound → 断言无 outbound + 排了 `deferred_inbound_reply` + 写 `quiet_hours_deferred_inbound` 事件；推进到醒来时刻 → 断言基于累积消息回 1 次。参考现有 `tests/quiet_hours_deferral.rs`（集成测，非 LLM）补真模型业务流。
  - 验收：半夜真发了/醒来不回 → 红。
- [ ] **R2.5.2** Planner 主动触达
  - 落点：构造「承诺到期/用户沉默」状态 → 跑 Planner（`planner_commitment_due.rs`/`planner_silent_followup.rs` 是集成测，补真模型）→ 断言产出主动触达 run 走完 gateway→outbox→送达，内容贴画像不打扰。
  - 验收：该催不催/骚扰 → 红。
- [ ] **R2.5.3** 幕后请示通道（治理命门，可前置到 R1 旁）
  - 落点：超职权场景 inbound → 断言走 escalation 请示通道（非硬答/非转真人）；relay 转述话术过 `check-no-human-takeover` 禁词；不泄露「真人决定」。参考 `tests/principal_decision_channel.rs`（集成测）补真模型。
  - 验收：承诺转真人/暴露幕后 → 红。**这是「无人工接管」定位命门。**
- [ ] **R2.5.4**（可选低优先）并发/多账号鲁棒性：多 contact 并发 + round_robin 轮休下 claim 锁/outbox 幂等不串台。
- [ ] **R2.5 验证**：真模型 CI；R2.5.3 人读日志确认转述无泄露。

## R3 · 深命门跨域行为

- [ ] **R3.1** H11 极性跨域行为测试（正反应 Hit/负反应 Block/沉默删失，极性随 profile）。
- [ ] **R3.2** C2 状态派生跨域（operation_state 派生+非法迁移拒写+审计，非销售 FSM）。
- [ ] **R3 验证**：真模型 CI。

## R4 · 知识库域适配

- [ ] **R4.1** 召回基准补真断言（recall@k 下限/跨轮稳定/漂移率上限）——`real_llm_recall_benchmark.rs`。
- [ ] **R4.2** 知识问答/抽取/完整度跨域语义正确——`real_llm_knowledge.rs` / `real_llm_knowledge_quality.rs`。
- [ ] **R4 验证**：真模型 CI；留意他人在跑的避免撞车。

## R5 · LLM 驱动动态发现线（nightly/手动，固定回归线 R0-R4 之后；不进 PR 门）

- [ ] **R5-T0 前置基础设施（盲区审补；不做完 R5.1+ 无处跑）**
  - 新建 `schedule:` cron workflow（仓库现无任何 schedule 触发器）+ 磁盘清理 + ledger artifact 上传 + 软门校验 step。
  - 第三 provider（roleplayer）独立 key 进 GitHub secret（现仅 agent/judge 两族，roleplayer 第三族 key 不存在）；动态线用**独立配额池**，不复用 PR 链的 key（防与 push 撞 429）。
  - rsxermu 单点无 failover：端点挂→动态线 skip 进 ledger（不算假绿），不红。
- [ ] **R5.0 反过拟合机械门（四铁律落地为可检验机制，非自律）**
  - control set 阴性对照：冻结"正常/友好/正向场景"黄金集，每次抽象修复后回归，正常场景退化即打回（防误伤，REFUSE_PROBING 撒娇教训）。
  - 变体 pre-registration：抽象假设 + 验证变体类型在改 prompt 的 diff 前冻结（来自独立对抗库，斩确认偏误）；先证伪根因（同根因不同表面的场景修复前也应失败）再验修复。
  - held-out 对抗集：Claude 修复时看不到的场景库，修完验回归不退化。
  - diff 机械门：禁 prompt 新增 few-shot 与失败对话字面/语义高重叠（n-gram+embedding 可检）。
- [ ] **R5.0.1 三角色异族硬门（改硬 fail）**
  - roleplayer/agent/judge 强制三个不同 provider 家族；三方指纹写 report，**同源/回落 state.llm → job 红**（非仅观测）。
  - 锁死 roleplayer/judge 禁止回落 `state.llm`（缺独立 key 直接 fail，非降级）。需先定"异族判定清单"（不同基座+不同机构+不同 RLHF，至少禁同 provider 同系列）。
- [ ] **R5.1** LLM Roleplayer（演客户）
  - 落点：新建 `tests/common/roleplayer.rs`：`roleplay_user_turn(client, persona, scene_goal, dialogue_history) -> String`。只给对话历史（不给 agent 内部决策，防作弊），按人设+场景目标真实反应 agent 上一句。需 temperature 可配测试 client（生产硬编码 0.2）。
  - 复现靠 **transcript 回放**（存档客户台词成 fixture 回放），**非 seed**（llm.rs 无 seed 通道）。
  - 验收：agent 不同回应→客户不同后续（博弈链通）；roleplayer 不越人设；transcript 可回放复现。
- [ ] **R5.2** Trajectory Judge（轨迹裁判，**校准达标前只进 ledger**）
  - 前置校准协议（方法论审）：人工金标 trajectory（每难度桶 ≥30 段）+ 多标注者 IAA 门（如 Krippendorff α≥0.6）+ train/dev/test split（judge 只在 dev 调，test 只用一次报 held-out 相关性 + 置信区间）。**协议不达标 → 只进 ledger 观测，不进软门**。
  - 落点：`judge_trajectory(client, full_dialogue, domain_profile)`。维度从 profile business_formulas/coverage_dimensions 派生（接 R1.1，**勿在轨迹层重新硬编码销售世界观**）。"意向推进"与"给空间/不施压"设**同权对立约束**（业务审：防奖励施压式推进）。
  - judge 漂移哨兵：定期金标重测 + 模型指纹写每条 ledger，趋势按指纹分段。
  - 验收：judge↔金标相关性达标；逐轮还行但整体没推进被判低分；任一轮破红线→整轨迹判负。
- [ ] **R5.3** 动态对抗（roleplayer 主动刁难 + 跟随失误升级）——覆盖业务分析 5 大复合场景。
- [ ] **R5.4** 跨会话长期关系弧（**同 TestApp 进程内多轮 + 中间不清集合**，非真跨进程持久化——testcontainer 即用即弃）。断沉淀（清记忆）行为退化能被检出。
- [ ] **R5 验证**：nightly；8 层 suspected_layer 归因（每阶段一个新变量，先 fixed scene 跑稳再上 roleplayer）；人读 trajectory 日志。

## 附 · 真实性审计 P1/P2 收尾（穿插进 R2/R4）

来自 `docs/real-llm-test-authenticity-audit.md`，可在相应阶段顺手修：
- [ ] ops t7 越界导出场景 → 拒绝硬断言
- [ ] ops t8/t17/t18 转真人红线 → 命中即 fail；修 t17/t18 查 prev_reply 错位 bug
- [ ] adversarial 6 弧 → takeover/injection 红线硬断言
- [ ] adversarial t_judge_calibration → hit_rate 阈值硬断言
- [ ] vision 类(k6/q3) → 0 抽取区分真空 vs 故障

## 提交纪律

- 每个 R 阶段独立提交、独立 CI 验证，不攒大 commit。
- 工作树有其它会话改动时精确 git add（[[reviewer-distrust-self-reported-low-risk]] 抢提交坑）。
- 每阶段完成更新 [[universal-test-coverage-initiative]] 记忆进度。
