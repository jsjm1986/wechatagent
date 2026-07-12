use std::env;

use crate::secret::mask_secret;

/// EVOLUTION_ENABLED 默认串。语义：是否允许在 UI 开启演化中心（默认允许）；
/// 设 "false" 为运维硬锁定（紧急熔断，无需 mongo 写权限）。
pub(crate) const EVOLUTION_ENABLED_DEFAULT: &str = "true";

#[derive(Clone)]
pub struct AppConfig {
    pub app_host: String,
    pub app_port: u16,
    pub app_base_url: String,
    pub mongodb_uri: String,
    pub mongodb_database: String,
    pub mcp_base_url: String,
    pub mcp_api_key: String,
    pub openai_base_url: String,
    pub openai_api_key: String,
    pub openai_model: String,
    pub default_workspace_id: String,
    pub default_account_id: String,
    pub agent_recent_message_limit: i64,
    pub agent_min_reply_interval_seconds: i64,
    /// 账号级最小发送间隔闸：同账号相邻两次发送的最小间隔下界（毫秒）。
    pub account_send_min_interval_ms: i64,
    /// 账号级最小发送间隔闸：上界（毫秒）。实际间隔在 [min,max] 间随机。
    pub account_send_max_interval_ms: i64,
    /// #68：单条出站消息软上限字符数。回复超过此长度时按句末标点就近切分成多条短消息,
    /// 更贴微信即时通讯习惯。默认 120。
    pub agent_reply_max_segment_chars: usize,
    /// #68：单次回复最多拆成几条短消息,超出则把尾部合并回最后一段,避免刷屏。默认 4。
    pub agent_reply_max_segments: usize,
    /// 并发多消息去抖窗口（毫秒）。用户连发多条时，调度器在收到最后一条后
    /// 等待此窗口再跑一次聚合流水线，把整串消息塌成一次回复（去抖 + 单联系人串行）。
    /// 默认 4000ms（3-5s 区间中点），clamp 到 [1000, 10000] 防退化忙等 / 误填。
    pub message_debounce_window_ms: u64,
    pub task_worker_interval_seconds: u64,
    /// F-013：completeness 缓存 TTL（秒）。默认 300。
    pub completeness_cache_ttl_seconds: i64,
    pub llm_timeout_seconds: u64,
    pub llm_max_retries: u32,
    pub llm_retry_base_ms: u64,
    /// HP-1 / Task 9：Worker 进程崩溃后，状态卡在 `running` 的任务的回收阈值。
    /// `claimed_at` 早于 `now - 该秒数` 即视为 stale，由下一次 tick 重置回 `retry`。
    pub task_claim_timeout_seconds: u64,
    /// 异步知识导入 worker 的轮询周期秒数（默认 2，求启动延迟低）。
    pub import_worker_interval_seconds: u64,
    /// 异步导入 job 的孤儿回收阈值秒数（默认 600 / 10 分钟，覆盖长文档墙钟 + 余量）。
    /// `running` 且 `claimed_at` 早于 `now - 该秒数` 视为进程崩溃遗留，重认领重跑。
    pub import_job_claim_timeout_seconds: u64,
    /// HP-3 / Task 10：reaction analysis claim 锁的超时阈值。
    /// `outcome_status="analyzing"` 但 `reaction_claimed_at` 早于 `now - 该秒数`
    /// 的 review 会被视为分析进程崩溃，允许下次 webhook 重新 claim。
    pub reaction_analysis_claim_timeout_seconds: u64,
    /// LP-14 / Task 20：webhook 限流窗口秒数（默认 60）。
    pub webhook_rate_limit_window_seconds: u32,
    /// LP-14 / Task 20：webhook 限流窗口内最大请求数（默认 30）。
    pub webhook_rate_limit_capacity: u32,
    /// M1 Strategic Planner：是否启用静默跟进扫描器。默认 false——首发关闭，
    /// 通过 `STRATEGIC_PLANNER_ENABLED=true` 显式开启；关闭时 `main.rs` 不会
    /// 启动 planner loop，等价于功能未上线，是回滚开关。
    pub strategic_planner_enabled: bool,
    /// M1 Strategic Planner：扫描循环周期秒数（默认 600 / 10 分钟）。
    pub strategic_planner_interval_seconds: u64,
    /// M1 Strategic Planner：判定 contact "静默" 的阈值小时数。
    /// `last_inbound_at < now - 该小时数` 才会被纳入候选。默认 72。
    pub strategic_planner_silent_threshold_hours: i64,
    /// M1 Strategic Planner：每个 account 当日最多 emit 多少条 follow-up 任务。
    /// 防止扫描器一次性把积压的静默联系人全部 emit 出去。默认 20。
    pub strategic_planner_daily_emit_cap: i64,
    /// M2 Strategic Planner：commitment.due_at 在 [now, now+N] 内视为 imminent，
    /// 触发 `Planner: commitment_imminent` 提前提醒。默认 8 小时。
    pub strategic_planner_commitment_imminent_window_hours: i64,
    /// M2 Strategic Planner：无显式 due_at 的承诺，用 `created_at + N 小时` 合成兜底
    /// 到期时间,使 LLM 当前产出的无 due_at 承诺也能被兜底跟进。默认 72 小时；
    /// 设 0 禁用兜底(回到"无 due_at 即跳过"的旧行为)。
    pub strategic_planner_commitment_fallback_due_hours: i64,
    /// M2 Strategic Planner：同一 commitment_id 在多少小时内不重复 emit。
    /// Planner 通过 `agent_events` 反查同 commitment_id 历史 emit 实现幂等。默认 24 小时。
    pub strategic_planner_commitment_emit_dedup_hours: i64,
    /// M2 Strategic Planner：customer_stage 多久未变视为停滞。
    /// 默认 14 天，比 silent 阈值 (72h) 长，避免与静默扫描器叠加。
    pub strategic_planner_stage_stagnation_threshold_days: i64,
    /// M2 Strategic Planner：比此值更近的 inbound 跳过 stage_stagnation emit。
    /// 默认 24 小时——避免用户刚说过话还没轮到推进就被 Planner 强催。
    pub strategic_planner_stage_stagnation_recent_inbound_hours: i64,
    /// §3.7 Strategic Planner：`scan_calendar` 主动关怀的临近窗口（天）。纪念日/生日
    /// 前几天起就允许触达。默认 1（当天 ± 提前 1 天）。`CalendarMode.lookahead_days`
    /// 可 per-profile 覆盖。
    pub strategic_planner_calendar_lookahead_days: i64,
    /// §3.7 Strategic Planner：`scan_calendar` 的独立每日 emit 上限（比常规
    /// `strategic_planner_daily_emit_cap` 更克制，防早安晚安/纪念日触达变骚扰）。
    /// 默认 3。`CalendarMode.daily_cap` 可 per-profile 覆盖。
    pub strategic_planner_calendar_daily_cap: i64,
    /// §3.7 Strategic Planner：`scan_calendar` 计算「今日」用的运营方时区固定偏移（小时）。
    /// planner 是 workspace 全局扫描、不构造 per-contact runtime，故在此取全局偏移（与
    /// `quiet_hours` 同款固定偏移哲学，不依赖部署宿主时区）。默认 +8（中国）。
    pub strategic_planner_calendar_tz_offset_hours: i32,
    /// G5 Strategic Planner：`scan_renewal` 续费推进的到期前临近窗口（天）。客户持有产品
    /// 到期前几天起就允许主动推进续费。默认 14。`RenewalMode.lookahead_days` 可 per-profile 覆盖。
    pub strategic_planner_renewal_lookahead_days: i64,
    /// G5 Strategic Planner：`scan_renewal` 已过期回看窗（天）。产品刚过期 N 天内仍主动挽留；
    /// 过期超此窗后不再触达，自然收口。默认 7。`RenewalMode.grace_days` 可 per-profile 覆盖。
    pub strategic_planner_renewal_grace_days: i64,
    /// G5 Strategic Planner：`scan_renewal` 的独立每日 emit 上限（防续费触达刷屏）。
    /// 默认 3。`RenewalMode.daily_cap` 可 per-profile 覆盖。
    pub strategic_planner_renewal_daily_cap: i64,
    /// G5 阶段2 Strategic Planner：`scan_reactivation` 进入休眠满 N 天后才开始唤醒（避免刚
    /// 流失就立刻骚扰）。默认 30。`ReactivationMode.dormant_days` 可 per-profile 覆盖。
    pub strategic_planner_reactivation_dormant_days: i64,
    /// G5 阶段2 Strategic Planner：`scan_reactivation` 每次唤醒最小间隔（天）。定期低频再激活、
    /// 绝不放任老客的节奏锚。默认 30。`ReactivationMode.cadence_days` 可 per-profile 覆盖。
    pub strategic_planner_reactivation_cadence_days: i64,
    /// G5 阶段2 Strategic Planner：`scan_reactivation` 的独立每日 emit 上限。默认 3。
    /// `ReactivationMode.daily_cap` 可 per-profile 覆盖。
    pub strategic_planner_reactivation_daily_cap: i64,
    /// G6 客户价值分层：中价值门槛（累计成交额，最小币种单位/分，单币种 CNY）。
    /// 累计成交额 ≥ 此值 → 中价值层。默认 50000（¥500）。
    pub value_tier_mid_threshold_cents: i64,
    /// G6 客户价值分层：高价值门槛（累计成交额，分，单币种 CNY）。
    /// 累计成交额 ≥ 此值 → 高价值层（优先于中价值）。默认 300000（¥3000）。
    pub value_tier_high_threshold_cents: i64,
    /// M3 Strategic Planner：反馈环回看窗口小时数。
    /// 在此窗口内反查该 contact 的 `agent_run_logs.final_review_status`，
    /// 计算 block-rate；默认 24。
    pub strategic_planner_block_rate_window_hours: i64,
    /// M3 Strategic Planner：反馈环最少 run 数门槛。
    /// 窗口内 run 数 < 此值时不参与 backoff 判定（冷启动友好）。默认 3。
    pub strategic_planner_block_rate_min_runs: i64,
    /// M3 Strategic Planner：block-rate 阈值（0.0~1.0）。
    /// `blocked / (blocked + ok) >= 此值` 时当次 emit 跳过，写 backoff 事件。默认 0.6。
    pub strategic_planner_block_rate_threshold: f64,
    /// M3 Strategic Planner：是否启用 commitment / stage_stagnation 段的跨联系人优先级排序。
    /// false 时退化为 M2 自然顺序（Mongo cursor 顺序）。默认 true。
    pub strategic_planner_priority_enabled: bool,

    // ── Phase D / D3：cold contact reactivation ──

    /// Phase D / D3：是否启用冷联系人重激活扫描器（与静默扫描器互补：
    /// 关注 `last_outbound_at` 远早于 now 的 contact，由 peer_case 钩子文案推动）。
    /// 默认 false——首发关闭，通过 `COLD_CONTACT_WORKER_ENABLED=true` 显式开启。
    pub cold_contact_worker_enabled: bool,
    /// Phase D / D3：判定 contact "冷链路" 的阈值小时数。
    /// `last_outbound_at < now - 该小时数` 才会被纳入候选；默认 168（7 天）。
    pub cold_contact_threshold_hours: i64,
    /// Phase D / D3：单 account 当日最多 emit 多少条冷重激活 follow_up；
    /// 与 strategic_planner_daily_emit_cap 解耦，避免拖累常规 follow_up。默认 5。
    pub cold_contact_daily_emit_cap: i64,
    /// 决策请示通道：领导链尾失联（无更多决策人可改派）时，给客户发 AI 延期安抚
    /// 话术的最小间隔（小时）。去重用——每 worker tick 都可能命中链尾超时，限频防刷屏。默认 6.0。
    pub holding_reply_min_interval_hours: f64,
    /// 过渡/占位回复 AI 生成的独立预算 token 上限（仅够一次短文案）。默认 3000。
    pub holding_reply_token_budget: i64,
    /// ④ 账号级发送软上限：单账号当日（UTC 日界起）`agent_send_outbox`
    /// `status=sent` 的总发送量达到该值时，发送主路径记一条 warning 审计事件
    /// （`agent.account_daily_send_soft_cap_exceeded`）告警防封号。**仅告警，
    /// 不拦截、不排队、不改变发送行为**——观测先行。默认 500（保守高值）。
    pub account_daily_send_soft_cap: i64,

    // ── 自学习采集管道（第一阶段）：行为信号 + 沉默删失 + 止血 ──
    //
    // 全部默认关停 / 保守值。本阶段只铺 append-only 采集底座，不调任何学习
    // 公式。沉默信号恒带 censored=true（删失，绝不当负例）。

    /// 是否启用沉默信号探测 worker（S6）。默认 false——首发关闭，需显式
    /// `SILENCE_SIGNAL_WORKER_ENABLED=true` 打开。只写信号，绝不发任何消息。
    pub silence_signal_worker_enabled: bool,
    /// 判定一条 outbound "沉默"（用户至今未回）的阈值秒数。默认 86400（24h）。
    pub silence_threshold_seconds: i64,
    /// 沉默探测 worker 主循环间隔秒数。默认 600（10 分钟）。
    pub silence_signal_interval_seconds: u64,
    /// 单 workspace 单 tick 最多落多少条沉默信号；防首跑信号风暴。默认 500。
    pub silence_signal_daily_cap: i64,
    /// S7 止血：dynamic_confidence 信 hit_rate 所需的最小样本数（hits+blocks）。
    /// 低于此值时只用 base（不被 1-2 个 reviewer 自评样本甩飞）。默认 5。
    pub dynamic_confidence_min_samples: u64,
    /// P1 换血（第二阶段）：dynamic_confidence 的 hit/block 信号是否取**真实用户反应**
    /// （按 run_id join `agent_decision_reviews.outcome_status`）而非 reviewer 自评
    /// （`review_approved`）。默认 **true**——立即止住"镜厅效应"（系统学的是 reviewer
    /// 喜欢什么，不是用户正反应什么）。设 false 可秒级回滚到旧的 review_approved 统计。
    /// 沉默/无反应（pending/None/unclassified）一律删失排除，不进 hit 也不进 block 分母。
    pub dynamic_confidence_real_outcome_enabled: bool,
    /// P3（第二阶段）：是否启用行为信号采集健康度计数（写入 `behavior_signal_metrics`）。
    /// 默认 false。打开后在采集点按 persisted/dedupe_skipped/errors 三态 `$inc` 累加，
    /// best-effort 不阻断主链。
    pub behavior_signal_metrics_enabled: bool,
    /// P4（第二阶段）：是否在知识召回 fallback 排序处启用受控探索（top-k 内 softmax 抽样）
    /// 并记录每个 chunk 的选中概率（selection_prob），为未来 off-policy 纠偏留 propensity。
    /// 默认 **false**（保持现状确定性 top-k）。探索只在已验证（verified）池内做，
    /// grounding/FactRisk/ProductAccuracy 硬门照常在其后执行，红线零破坏。
    pub knowledge_exploration_enabled: bool,
    /// 默认 **true**——渐进式三档机制开关（注意：与多数 *_ENABLED 默认 false 相反）。
    /// 关时第一程直接传 Full、跳过强升、退回 ptier 前单程行为，等于 kill switch
    /// （上线初期止损 / 账号灰度 / A-B 对照）。
    pub progressive_tier_enabled: bool,
    /// P4：探索 softmax 温度。越大越接近均匀抽样、越小越接近确定性 argmax。默认 1.0。
    /// 仅在 `knowledge_exploration_enabled=true` 时生效。
    pub knowledge_exploration_temperature: f64,

    // ── agent-self-evolution M4：演化器（独立 worker） ──
    //
    // 默认全部保守值。`evolution_enabled=false` 是安装态默认；运维需显式
    // 通过 env 打开。所有阈值在 design.md §5 中有明确单位与范围说明。

    /// M4：是否允许在 UI 开启演化中心（runtime flag 总开关的硬上限）。默认 true（允许）；
    /// 设 false 为运维硬锁定——worker 不进 tick、UI 总开关锁定。
    pub evolution_enabled: bool,
    /// M4：演化器主循环间隔秒数。默认 21600（6 小时）——比 strategic planner 长一档。
    pub evolution_tick_seconds: u64,
    /// M4：单次 tick 的 LLM token 预算上限。Critic LLM 触顶后整波 prompt 候选 drop。
    pub evolution_run_token_budget: i64,
    /// M4：单次 tick 的 LLM 调用次数上限。
    pub evolution_run_max_llm_calls: i32,
    /// M4：cohort 选择回看窗口小时数。默认 72。
    pub evolution_eval_window_hours: u32,
    /// M4：cohort 最少 run 数门槛。低于此值整波 cohort 视为空（不产候选）。
    pub evolution_min_replays: usize,
    /// M4：threshold 候选释放门槛——shadow eval 后 send_success_rate 提升必须 ≥ 此值。
    pub evolution_min_send_success_delta: f64,
    /// M4：prompt 候选释放门槛——self_critique_addressed_rate 提升必须 ≥ 此值。
    pub evolution_min_self_critique_delta: f64,
    /// M4：5 闸命中率任一上升不得超过此值（防止 prompt 修订引入新风险）。
    pub evolution_max_5gate_hit_increase: f64,
    /// #152：安全闸（fact_risk_block / pressure_risk_block / product_accuracy_score_block）
    /// 放松提案的「安全回归率」上限——shadow 中"原本被该安全闸拦下、新阈值却放行"
    /// 的占比超过此值即 reject，哪怕 send_success 提升达标。默认 0.0（零容忍：
    /// 任一条风险消息从 blocked 翻成 sent 即否决放松）。
    pub evolution_max_safety_regression_rate: f64,
    /// M4：shadow replay 并发上限（tokio Semaphore 容量）。
    pub evolution_replay_concurrency: usize,
    /// M4：shadow replay 失败率上限——超过此比例直接 reject 候选。
    pub evolution_replay_max_fail_rate: f64,
    /// M4：同 gate_key 上次 release 的 cooldown 小时数；窗口内同 gate 候选直接 reject。
    pub evolution_threshold_release_cooldown_hours: u32,
    /// M4：cohort 内同一 contact_wxid 的 run 上限（去重）。
    pub evolution_cohort_per_contact_cap: usize,
    /// M4：每个 finalReviewStatus 失败桶给 Critic LLM 的样本数。
    pub evolution_cohort_sample_per_failure_bucket: usize,
    /// universal-domain-adaptation 2.5-pre-3：post-release 业务结果兜底观测指标
    /// `negative_reaction_rate`（窗口内客户负反应占已分类反应的比例，按 run_id join
    /// `agent_decision_reviews.outcome_status` 经 [`crate::knowledge_wiki::gap_signals::classify_outcome_label`]
    /// 三态判定，沉默/未分类删失排除）的「升幅」上限。release 后该比率较 release 前
    /// 上升超过此值，说明放行率提升可能以更多客户负反应为代价（reviewer 过程指标与
    /// 业务结果背离）。**本期仅观测**：算出 delta 写进 post_release 事件 details 供 admin
    /// 察觉错配，**不参与任何 promote/rollback 判决**（强制门留 2.5-main-4，默认关）。默认 0.05。
    pub evolution_max_negative_reaction_increase: f64,

    // ── Phase C / C5：threshold_overrides 自动 release（hold_rate close-loop） ──
    //
    // 默认关停。`evolution_auto_release_enabled=true` 时演化器 tick 末尾会扫描
    // status="eligible_for_release" 的 threshold proposal，回看
    // `evolution_auto_release_window_hours` 小时窗口的 hold_rate / hit_rate 信号；
    // 仍在异常区间 → 自动调 release_threshold（admin id="evolution_auto_release"）；
    // 已回到正常区间 → 跳过留给 admin 显式判断。
    //
    // 自动通道仅适用于 threshold（纯统计可观测）；prompt 候选仍要 admin 二次确认。
    // rollback 永远人工——Requirements 9.7 的硬约束。

    /// Phase C / C5：是否启用 threshold proposal 自动 release。默认 false。
    pub evolution_auto_release_enabled: bool,
    /// Phase C / C5：自动 release 决策回看窗口（小时）。默认 336（14 天）。
    pub evolution_auto_release_window_hours: u32,
    /// Phase C / C5：单 tick 自动 release 的 proposal 数量上限（防止一波打开过多 gate）。
    pub evolution_auto_release_per_tick_cap: usize,
    /// universal-domain-adaptation 2.5-main-4：自动 release 的「客户负反应强制门」开关。
    /// 默认 false（字节等价：关时 auto_release 行为与 main-4 前完全一致）。开启后，
    /// auto_release 在 `decide_auto_release` 判定放行**之后、实际调 release_threshold
    /// 之前**，多过一道闸：回看窗口内当前**绝对**负反应率（按 `decision_reviews.outcome_status`
    /// 经 active `DomainProfile.outcome_polarity` 分类，复用回路① 的 `classify_outcome_label`）
    /// 高于 [`Self::evolution_auto_release_max_negative_reaction_rate`] 时，强制 SKIP 该候选、
    /// 退回 admin 显式判断，**不自动放行阈值放松**。这是「拒绝自动放行」而非「回滚」，
    /// 不触碰 Requirements 9.7（rollback 永远手动）的硬约束。
    pub evolution_auto_release_negative_reaction_gate_enabled: bool,
    /// universal-domain-adaptation 2.5-main-4：自动 release 负反应强制门的**绝对**阈值
    /// （非 pre-3 的前/后窗口升幅 delta —— auto_release 在 release 前决策，没有「后窗口」
    /// 可比，故看当前窗口的绝对负反应率）。仅当
    /// [`Self::evolution_auto_release_negative_reaction_gate_enabled`]=true 时生效。
    /// 默认 0.30。窗口内无已分类客户反应（全删失/无反应）时**不阻拦**（保守：无信号不强制 skip）。
    pub evolution_auto_release_max_negative_reaction_rate: f64,

    // ── Knowledge Digest Workstation ──
    //
    // 默认关停。设计见 `.kiro/specs/knowledge-digest-workstation/`。

    /// 是否启用知识库日报 worker。默认 false（安装态关停）。
    pub knowledge_digest_enabled: bool,
    /// 每天触发合成的小时（运营时区，0..=23）。默认 9。
    pub knowledge_digest_run_hour: u32,
    /// 单次 worker tick 的 LLM token 预算上限。
    pub knowledge_digest_run_token_budget: i64,
    /// 单次 worker tick 的 LLM 调用次数上限。
    pub knowledge_digest_run_max_llm_calls: i32,
    /// `KnowledgeTaskWorker` tick 间隔秒数；0 表示停掉。默认 30。
    pub knowledge_task_worker_interval_seconds: u64,
    /// knowledge-wiki Phase E：catalog rebuild worker tick 间隔秒数；0 表示停掉。默认 3。
    pub catalog_rebuild_worker_interval_seconds: u64,
    /// knowledge-wiki Phase F：feedback worker tick 间隔秒数；0 表示停掉。默认 600（10 分钟）。
    pub knowledge_feedback_interval_seconds: u64,
    /// Phase G P1-6：auto-ingest worker 总开关。默认 false（安装态关停，需运维显式打开）。
    /// 关闭时 `main.rs` 不 spawn worker，等价于功能未上线，是回滚开关。
    pub ingest_worker_enabled: bool,
    /// Phase G P1-6：auto-ingest worker tick 间隔秒数；0 表示停掉。默认 3600（每小时一轮）。
    /// 单 source 自身的 schedule_minutes 节流叠加在这之上（更长时间间隔以 source 为准）。
    pub ingest_worker_interval_seconds: u64,
    /// Phase E / E2：reviewer 双脑并行开关。`true` 时 review_decision 会用第二
    /// provider 并行跑一次评分；与主 reviewer 在 `approved` 或软闸命中上分歧
    /// 即触发 single-shot revision，达到 epistemic diversity。`false`（默认）
    /// 退回单 reviewer，行为与开关引入前完全一致。
    pub reviewer_dual_enabled: bool,
    /// 第二 reviewer provider 的 base_url；`reviewer_dual_enabled=true` 但本字段
    /// 为空时启动序列拒绝（避免静默退化为单 reviewer）。
    pub reviewer_second_provider_base_url: Option<String>,
    /// 第二 reviewer provider 的 api_key。
    pub reviewer_second_provider_api_key: Option<String>,
    /// 第二 reviewer provider 的 model 名（如 `deepseek-chat` / `claude-3-5-haiku`）。
    pub reviewer_second_provider_model: Option<String>,
    /// 第二 reviewer provider 的协议形态：`openai`（默认，覆盖 OpenAI / DeepSeek /
    /// 兼容路径）/ `anthropic`。与 [`crate::llm::LlmFormat`] 同集合。
    pub reviewer_second_provider_format: String,

    // ── P0 鉴权 / Session ──
    //
    // admin SPA 同 origin 走 cookie session；公网部署阻断未登录访问。

    /// session cookie TTL（小时）。默认 168（7 天）。
    pub session_ttl_hours: i64,
    /// Set-Cookie 是否带 Secure 属性。生产环境（HTTPS）必须 true；
    /// 本地开发 (HTTP) 留 false 否则浏览器会拒绝。
    pub session_cookie_secure: bool,
    /// 启动 bootstrap：当 admin_users 集合为空时，从这两个 env 创建首个 admin。
    /// 留空则不 bootstrap（首次部署后建议清空 env）。
    pub bootstrap_admin_username: Option<String>,
    pub bootstrap_admin_password: Option<String>,
    /// webhook 是否校验 HMAC-SHA256(body, MCP_API_KEY) 签名（X-MCP-Signature 头）。
    /// 生产必须 true；staging/local 测试可以临时关掉。默认 true。
    pub webhook_verify_signature: bool,
    /// 方案 B：`x-webhook-timestamp`（毫秒）与当前时间允许的最大偏差（秒），
    /// 超窗拒绝以防重放。默认 300（±5 分钟），与 gewe-agent 入站侧 skew 校验对称。
    pub webhook_timestamp_skew_seconds: i64,
    // ── P1-7：JWT RS256（公网 Bearer token 鉴权） ──
    //
    // session cookie 同 origin 路径，已经覆盖 admin SPA。公网 / 第三方调用走 JWT：
    // `Authorization: Bearer <jwt>`。`jwt_enabled=false`（默认）时 middleware 仅
    // 接 cookie 路径；置 true 必须同时配齐 PEM 双密钥，否则启动期 panic 拒起。
    /// JWT 总开关。`true` 时 `/auth/token` 路由开放、middleware 接受 Bearer 头。
    pub jwt_enabled: bool,
    /// JWT 过期窗口（分钟）。默认 60。
    pub jwt_ttl_minutes: i64,
    /// RS256 私钥 PEM。`jwt_enabled=true` 时必填。
    pub jwt_private_key_pem: Option<String>,
    /// RS256 公钥 PEM。`jwt_enabled=true` 时必填，用于 verify。
    pub jwt_public_key_pem: Option<String>,

    // ── 销售素材文件发送 ──
    /// 销售素材文件本地存储根目录。默认 "./media"；生产 117 为 "/opt/wechatagent/media"。
    pub media_storage_dir: String,
    /// 单个素材上传大小上限（MB）。默认 50。
    pub media_max_file_size_mb: u64,
    /// MCP media_id 缓存有效期（小时），过期重传。默认 24。
    pub media_id_cache_ttl_hours: i64,
    /// 唤醒任务 per-contact 确定性 jitter 上限（秒）。静默期延后的发送/入站全部重排到
    /// 次日 quiet_hours_end（如 8:00），同 workspace 多客户会整点齐发；给 next_wake_at
    /// 加按 contact.wxid 派生的 [0, 该值] 秒偏移把唤醒散开，避峰且更像真人。默认 900（15min）。
    /// 设 0 关闭 jitter（恒整点）。env `WAKE_JITTER_MAX_SECONDS`。
    pub wake_jitter_max_seconds: u32,
}

/// 手写 `Debug` 实现：把所有 secret 字段过 [`mask_secret`]，避免日志/panic
/// 输出 `{:?}` 时把完整 api_key / 密码泄漏到 stdout 与 tracing 后端。
/// 非 secret 字段保持透出，方便启动期诊断。
impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("app_host", &self.app_host)
            .field("app_port", &self.app_port)
            .field("app_base_url", &self.app_base_url)
            .field("mongodb_uri", &self.mongodb_uri)
            .field("mongodb_database", &self.mongodb_database)
            .field("mcp_base_url", &self.mcp_base_url)
            .field("mcp_api_key", &mask_secret(&self.mcp_api_key))
            .field("openai_base_url", &self.openai_base_url)
            .field("openai_api_key", &mask_secret(&self.openai_api_key))
            .field("openai_model", &self.openai_model)
            .field("default_workspace_id", &self.default_workspace_id)
            .field("default_account_id", &self.default_account_id)
            .field(
                "reviewer_second_provider_api_key",
                &self
                    .reviewer_second_provider_api_key
                    .as_deref()
                    .map(mask_secret),
            )
            .field("bootstrap_admin_username", &self.bootstrap_admin_username)
            .field(
                "bootstrap_admin_password",
                &self.bootstrap_admin_password.as_deref().map(mask_secret),
            )
            .field("webhook_verify_signature", &self.webhook_verify_signature)
            .field("session_cookie_secure", &self.session_cookie_secure)
            .field("jwt_enabled", &self.jwt_enabled)
            .field("jwt_ttl_minutes", &self.jwt_ttl_minutes)
            .field(
                "jwt_private_key_pem",
                &self.jwt_private_key_pem.as_deref().map(mask_secret),
            )
            .field(
                "jwt_public_key_pem",
                &self.jwt_public_key_pem.as_deref().map(mask_secret),
            )
            .finish_non_exhaustive()
    }
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            app_host: env_or("APP_HOST", "0.0.0.0"),
            app_port: env_or("APP_PORT", "8080").parse()?,
            app_base_url: env_or("APP_BASE_URL", "http://localhost:8080"),
            mongodb_uri: env_or("MONGODB_URI", "mongodb://localhost:27017"),
            mongodb_database: env_or("MONGODB_DATABASE", "wechatagent"),
            mcp_base_url: env_or("MCP_BASE_URL", "http://47.108.57.147:3001"),
            mcp_api_key: require_env("MCP_API_KEY")?,
            openai_base_url: env_or("OPENAI_BASE_URL", "https://api.openai.com/v1"),
            openai_api_key: require_env("OPENAI_API_KEY")?,
            openai_model: env_or("OPENAI_MODEL", "gpt-4.1-mini"),
            default_workspace_id: env_or("DEFAULT_WORKSPACE_ID", "default"),
            default_account_id: env_or("DEFAULT_ACCOUNT_ID", "default"),
            agent_recent_message_limit: env_or("AGENT_RECENT_MESSAGE_LIMIT", "12").parse()?,
            agent_min_reply_interval_seconds: env_or("AGENT_MIN_REPLY_INTERVAL_SECONDS", "20")
                .parse()?,
            account_send_min_interval_ms: env_or("ACCOUNT_SEND_MIN_INTERVAL_MS", "1000").parse()?,
            account_send_max_interval_ms: env_or("ACCOUNT_SEND_MAX_INTERVAL_MS", "4000").parse()?,
            agent_reply_max_segment_chars: env_or("AGENT_REPLY_MAX_SEGMENT_CHARS", "120")
                .parse::<usize>()?
                .max(1),
            agent_reply_max_segments: env_or("AGENT_REPLY_MAX_SEGMENTS", "4")
                .parse::<usize>()?
                .max(1),
            message_debounce_window_ms: env_or("MESSAGE_DEBOUNCE_WINDOW_MS", "4000")
                .parse::<u64>()?
                .clamp(1000, 10_000),
            task_worker_interval_seconds: env_or("TASK_WORKER_INTERVAL_SECONDS", "30").parse()?,
            completeness_cache_ttl_seconds: env_or("COMPLETENESS_CACHE_TTL_SECONDS", "300")
                .parse()?,
            llm_timeout_seconds: env_or("LLM_TIMEOUT_SECONDS", "45").parse()?,
            llm_max_retries: env_or("LLM_MAX_RETRIES", "5").parse()?,
            llm_retry_base_ms: env_or("LLM_RETRY_BASE_MS", "1500").parse()?,
            task_claim_timeout_seconds: env_or("TASK_CLAIM_TIMEOUT_SECONDS", "300").parse()?,
            import_worker_interval_seconds: env_or("IMPORT_WORKER_INTERVAL_SECONDS", "2").parse()?,
            import_job_claim_timeout_seconds: env_or("IMPORT_JOB_CLAIM_TIMEOUT_SECONDS", "600")
                .parse()?,
            reaction_analysis_claim_timeout_seconds: env_or(
                "REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS",
                "60",
            )
            .parse()?,
            webhook_rate_limit_window_seconds: env_or("WEBHOOK_RATE_LIMIT_WINDOW_SECONDS", "60")
                .parse()?,
            webhook_rate_limit_capacity: env_or("WEBHOOK_RATE_LIMIT_CAPACITY", "30").parse()?,
            strategic_planner_enabled: parse_bool(&env_or("STRATEGIC_PLANNER_ENABLED", "false")),
            strategic_planner_interval_seconds: env_or("STRATEGIC_PLANNER_INTERVAL_SECONDS", "600")
                .parse()?,
            strategic_planner_silent_threshold_hours: env_or(
                "STRATEGIC_PLANNER_SILENT_THRESHOLD_HOURS",
                "72",
            )
            .parse()?,
            strategic_planner_daily_emit_cap: env_or("STRATEGIC_PLANNER_DAILY_EMIT_CAP", "20")
                .parse()?,
            strategic_planner_commitment_imminent_window_hours: env_or(
                "STRATEGIC_PLANNER_COMMITMENT_IMMINENT_WINDOW_HOURS",
                "8",
            )
            .parse()?,
            strategic_planner_commitment_fallback_due_hours: env_or(
                "STRATEGIC_PLANNER_COMMITMENT_FALLBACK_DUE_HOURS",
                "72",
            )
            .parse()?,
            strategic_planner_commitment_emit_dedup_hours: env_or(
                "STRATEGIC_PLANNER_COMMITMENT_EMIT_DEDUP_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_stage_stagnation_threshold_days: env_or(
                "STRATEGIC_PLANNER_STAGE_STAGNATION_THRESHOLD_DAYS",
                "14",
            )
            .parse()?,
            strategic_planner_stage_stagnation_recent_inbound_hours: env_or(
                "STRATEGIC_PLANNER_STAGE_STAGNATION_RECENT_INBOUND_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_calendar_lookahead_days: env_or(
                "STRATEGIC_PLANNER_CALENDAR_LOOKAHEAD_DAYS",
                "1",
            )
            .parse()?,
            strategic_planner_calendar_daily_cap: env_or(
                "STRATEGIC_PLANNER_CALENDAR_DAILY_CAP",
                "3",
            )
            .parse()?,
            strategic_planner_calendar_tz_offset_hours: env_or(
                "STRATEGIC_PLANNER_CALENDAR_TZ_OFFSET_HOURS",
                "8",
            )
            .parse()?,
            strategic_planner_renewal_lookahead_days: env_or(
                "STRATEGIC_PLANNER_RENEWAL_LOOKAHEAD_DAYS",
                "14",
            )
            .parse()?,
            strategic_planner_renewal_grace_days: env_or(
                "STRATEGIC_PLANNER_RENEWAL_GRACE_DAYS",
                "7",
            )
            .parse()?,
            strategic_planner_renewal_daily_cap: env_or(
                "STRATEGIC_PLANNER_RENEWAL_DAILY_CAP",
                "3",
            )
            .parse()?,
            strategic_planner_reactivation_dormant_days: env_or(
                "STRATEGIC_PLANNER_REACTIVATION_DORMANT_DAYS",
                "30",
            )
            .parse()?,
            strategic_planner_reactivation_cadence_days: env_or(
                "STRATEGIC_PLANNER_REACTIVATION_CADENCE_DAYS",
                "30",
            )
            .parse()?,
            strategic_planner_reactivation_daily_cap: env_or(
                "STRATEGIC_PLANNER_REACTIVATION_DAILY_CAP",
                "3",
            )
            .parse()?,
            value_tier_mid_threshold_cents: env_or(
                "VALUE_TIER_MID_THRESHOLD_CENTS",
                "50000",
            )
            .parse()?,
            value_tier_high_threshold_cents: env_or(
                "VALUE_TIER_HIGH_THRESHOLD_CENTS",
                "300000",
            )
            .parse()?,
            strategic_planner_block_rate_window_hours: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_WINDOW_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_block_rate_min_runs: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_MIN_RUNS",
                "3",
            )
            .parse()?,
            strategic_planner_block_rate_threshold: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_THRESHOLD",
                "0.6",
            )
            .parse()?,
            strategic_planner_priority_enabled: parse_bool(&env_or(
                "STRATEGIC_PLANNER_PRIORITY_ENABLED",
                "true",
            )),
            cold_contact_worker_enabled: parse_bool(&env_or(
                "COLD_CONTACT_WORKER_ENABLED",
                "false",
            )),
            cold_contact_threshold_hours: env_or("COLD_CONTACT_THRESHOLD_HOURS", "168").parse()?,
            cold_contact_daily_emit_cap: env_or("COLD_CONTACT_DAILY_EMIT_CAP", "5").parse()?,
            holding_reply_min_interval_hours: env_or("HOLDING_REPLY_MIN_INTERVAL_HOURS", "6.0")
                .parse()?,
            holding_reply_token_budget: env_or("HOLDING_REPLY_TOKEN_BUDGET", "3000").parse()?,
            account_daily_send_soft_cap: env_or("ACCOUNT_DAILY_SEND_SOFT_CAP", "500").parse()?,
            // ── 自学习采集管道（第一阶段） ──
            silence_signal_worker_enabled: parse_bool(&env_or(
                "SILENCE_SIGNAL_WORKER_ENABLED",
                "false",
            )),
            silence_threshold_seconds: env_or("SILENCE_THRESHOLD_SECONDS", "86400").parse()?,
            silence_signal_interval_seconds: env_or("SILENCE_SIGNAL_INTERVAL_SECONDS", "600")
                .parse()?,
            silence_signal_daily_cap: env_or("SILENCE_SIGNAL_DAILY_CAP", "500").parse()?,
            dynamic_confidence_min_samples: env_or("DYNAMIC_CONFIDENCE_MIN_SAMPLES", "5").parse()?,
            dynamic_confidence_real_outcome_enabled: parse_bool(&env_or(
                "DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED",
                "true",
            )),
            behavior_signal_metrics_enabled: parse_bool(&env_or(
                "BEHAVIOR_SIGNAL_METRICS_ENABLED",
                "false",
            )),
            knowledge_exploration_enabled: parse_bool(&env_or(
                "KNOWLEDGE_EXPLORATION_ENABLED",
                "false",
            )),
            progressive_tier_enabled: parse_bool(&env_or("PROGRESSIVE_TIER_ENABLED", "true")),
            knowledge_exploration_temperature: env_or("KNOWLEDGE_EXPLORATION_TEMPERATURE", "1.0")
                .parse()?,
            // ── agent-self-evolution M4 ──
            evolution_enabled: parse_bool(&env_or("EVOLUTION_ENABLED", EVOLUTION_ENABLED_DEFAULT)),
            evolution_tick_seconds: env_or("EVOLUTION_TICK_SECONDS", "21600").parse()?,
            evolution_run_token_budget: env_or("EVOLUTION_RUN_TOKEN_BUDGET", "60000").parse()?,
            evolution_run_max_llm_calls: env_or("EVOLUTION_RUN_MAX_LLM_CALLS", "30").parse()?,
            evolution_eval_window_hours: env_or("EVOLUTION_EVAL_WINDOW_HOURS", "72").parse()?,
            evolution_min_replays: env_or("EVOLUTION_MIN_REPLAYS", "30").parse()?,
            evolution_min_send_success_delta: env_or("EVOLUTION_MIN_SEND_SUCCESS_DELTA", "0.05")
                .parse()?,
            evolution_min_self_critique_delta: env_or("EVOLUTION_MIN_SELF_CRITIQUE_DELTA", "0.10")
                .parse()?,
            evolution_max_5gate_hit_increase: env_or("EVOLUTION_MAX_5GATE_HIT_INCREASE", "0.10")
                .parse()?,
            evolution_max_safety_regression_rate: env_or(
                "EVOLUTION_MAX_SAFETY_REGRESSION_RATE",
                "0.0",
            )
            .parse()?,
            evolution_replay_concurrency: env_or("EVOLUTION_REPLAY_CONCURRENCY", "4").parse()?,
            evolution_replay_max_fail_rate: env_or("EVOLUTION_REPLAY_MAX_FAIL_RATE", "0.30")
                .parse()?,
            evolution_threshold_release_cooldown_hours: env_or(
                "EVOLUTION_THRESHOLD_RELEASE_COOLDOWN_HOURS",
                "24",
            )
            .parse()?,
            evolution_cohort_per_contact_cap: env_or("EVOLUTION_COHORT_PER_CONTACT_CAP", "3")
                .parse()?,
            evolution_cohort_sample_per_failure_bucket: env_or(
                "EVOLUTION_COHORT_SAMPLE_PER_FAILURE_BUCKET",
                "10",
            )
            .parse()?,
            evolution_max_negative_reaction_increase: env_or(
                "EVOLUTION_MAX_NEGATIVE_REACTION_INCREASE",
                "0.05",
            )
            .parse()?,
            // ── Phase C / C5：自动 release ──
            evolution_auto_release_enabled: parse_bool(&env_or(
                "EVOLUTION_AUTO_RELEASE_ENABLED",
                "false",
            )),
            evolution_auto_release_window_hours: env_or(
                "EVOLUTION_AUTO_RELEASE_WINDOW_HOURS",
                "336",
            )
            .parse()?,
            evolution_auto_release_per_tick_cap: env_or(
                "EVOLUTION_AUTO_RELEASE_PER_TICK_CAP",
                "1",
            )
            .parse()?,
            evolution_auto_release_negative_reaction_gate_enabled: parse_bool(&env_or(
                "EVOLUTION_AUTO_RELEASE_NEGATIVE_REACTION_GATE_ENABLED",
                "false",
            )),
            evolution_auto_release_max_negative_reaction_rate: env_or(
                "EVOLUTION_AUTO_RELEASE_MAX_NEGATIVE_REACTION_RATE",
                "0.30",
            )
            .parse()?,
            // ── Knowledge Digest Workstation ──
            knowledge_digest_enabled: parse_bool(&env_or("KNOWLEDGE_DIGEST_ENABLED", "false")),
            knowledge_digest_run_hour: env_or("KNOWLEDGE_DIGEST_RUN_HOUR", "9").parse()?,
            knowledge_digest_run_token_budget: env_or(
                "KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET",
                "24000",
            )
            .parse()?,
            knowledge_digest_run_max_llm_calls: env_or(
                "KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS",
                "8",
            )
            .parse()?,
            knowledge_task_worker_interval_seconds: env_or(
                "KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS",
                "30",
            )
            .parse()?,
            catalog_rebuild_worker_interval_seconds: env_or(
                "CATALOG_REBUILD_WORKER_INTERVAL_SECONDS",
                "3",
            )
            .parse()?,
            knowledge_feedback_interval_seconds: env_or(
                "KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS",
                "600",
            )
            .parse()?,
            ingest_worker_enabled: parse_bool(&env_or("INGEST_WORKER_ENABLED", "false")),
            ingest_worker_interval_seconds: env_or("INGEST_WORKER_INTERVAL_SECONDS", "3600")
                .parse()?,
            reviewer_dual_enabled: parse_bool(&env_or("REVIEWER_DUAL_ENABLED", "false")),
            reviewer_second_provider_base_url: env::var("REVIEWER_SECOND_PROVIDER_BASE_URL").ok(),
            reviewer_second_provider_api_key: env::var("REVIEWER_SECOND_PROVIDER_API_KEY").ok(),
            reviewer_second_provider_model: env::var("REVIEWER_SECOND_PROVIDER_MODEL").ok(),
            reviewer_second_provider_format: env_or("REVIEWER_SECOND_PROVIDER_FORMAT", "openai"),
            session_ttl_hours: env_or("SESSION_TTL_HOURS", "168").parse()?,
            session_cookie_secure: parse_bool(&env_or("SESSION_COOKIE_SECURE", "false")),
            bootstrap_admin_username: env::var("BOOTSTRAP_ADMIN_USERNAME")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            bootstrap_admin_password: env::var("BOOTSTRAP_ADMIN_PASSWORD")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            webhook_verify_signature: parse_bool(&env_or("WEBHOOK_VERIFY_SIGNATURE", "true")),
            webhook_timestamp_skew_seconds: env_or("WEBHOOK_TIMESTAMP_SKEW_SECONDS", "300").parse()?,
            jwt_enabled: parse_bool(&env_or("JWT_ENABLED", "false")),
            jwt_ttl_minutes: env_or("JWT_TTL_MINUTES", "60").parse()?,
            jwt_private_key_pem: env::var("JWT_PRIVATE_KEY_PEM")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            jwt_public_key_pem: env::var("JWT_PUBLIC_KEY_PEM")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            media_storage_dir: env_or("MEDIA_STORAGE_DIR", "./media"),
            media_max_file_size_mb: env_or("MEDIA_MAX_FILE_SIZE_MB", "50").parse()?,
            media_id_cache_ttl_hours: env_or("MEDIA_ID_CACHE_TTL_HOURS", "24").parse()?,
            wake_jitter_max_seconds: env_or("WAKE_JITTER_MAX_SECONDS", "900").parse()?,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn require_env(key: &str) -> anyhow::Result<String> {
    env::var(key).map_err(|_| anyhow::anyhow!("missing required environment variable {key}"))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 销售素材三配置的默认值（与 `.env.example` / brief 约定一致）。
    /// 直接断言 `from_env` 用的同款 `env_or(key, default)` 默认串解析结果，
    /// 避免改动进程全局 env（并行测试下会互相污染）。
    #[test]
    fn media_config_defaults() {
        // MEDIA_STORAGE_DIR：未设时默认 "./media"。
        assert_eq!(env_or("MEDIA_STORAGE_DIR_UNSET_XYZ", "./media"), "./media");
        // MEDIA_MAX_FILE_SIZE_MB：默认 50，解析为 u64。
        let max_mb: u64 = env_or("MEDIA_MAX_FILE_SIZE_MB_UNSET_XYZ", "50")
            .parse()
            .expect("default media max file size must parse as u64");
        assert_eq!(max_mb, 50);
        // MEDIA_ID_CACHE_TTL_HOURS：默认 24，解析为 i64。
        let ttl_hours: i64 = env_or("MEDIA_ID_CACHE_TTL_HOURS_UNSET_XYZ", "24")
            .parse()
            .expect("default media id cache ttl must parse as i64");
        assert_eq!(ttl_hours, 24);
    }

    /// EVOLUTION_ENABLED 默认＝允许（true）。锁定 EVOLUTION_ENABLED_DEFAULT 常量值，
    /// 防止有人把默认改回 "false" 而不更新部署文档（语义：默认允许 UI 开启演化中心）。
    #[test]
    fn evolution_enabled_defaults_to_true() {
        assert_eq!(EVOLUTION_ENABLED_DEFAULT, "true");
        assert!(parse_bool(EVOLUTION_ENABLED_DEFAULT));
    }
}
