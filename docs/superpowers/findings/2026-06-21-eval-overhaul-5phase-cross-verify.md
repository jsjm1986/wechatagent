# 评判体系重构5阶段 — 交叉验证终审报告

日期: 2026-06-21
对象: 真模型评判测试设施重构(5阶段) — 零 src/ 生产改动,改动仅在 tests/ + .github/workflows/ci.yml + docs/
方法: 多角度审计 → 去重分组 → 逐组派独立 verifier 主动证伪 → 本报告综合

## 统计

- 审计角度: 12(阶段维 phase1-5 共 5 个 + 跨切面 B1-B7 共 7 个)
- 去重分组: 21 组(G1-G21)
- 主动证伪结论: CONFIRMED 14 / PARTIAL_REFUTED 7 / REFUTED 0 / INCONCLUSIVE 0
- 真实问题(CONFIRMED): Critical 1 / Important 4 / Minor 3 / Observation 6
- 部分成立(已收窄): 7

---

## 一、真实问题清单(verdict=CONFIRMED)

### Critical

#### G1 — skip_ledger.jsonl 跨 job 文件名碰撞 → merge-multiple 后写覆盖 → skip-gate 漏计假绿(铁律4 击穿)

所有 real-llm job(smoke/recall/ops/quality/adversarial/redline/autonomy-redline/conversation-judge/roleplayer)写 skip 都用硬编码同名文件 `skip_ledger.jsonl`(macro 与 `record_judge_skip` 均 `open(format!("{dir}/skip_ledger.jsonl"))`,无 job/matrix 后缀)。skip-gate 用 `download-artifact merge-multiple: true` 把所有 `*ledger*` artifact 合进同一目录,同名文件 last-wins 覆盖而非拼接;`check-skip-ledger.sh` 又只 `wc -l` 读单一固定路径。结果:跨 8+ job 累计几十条 skip,gate 只数到最后解出那一个 artifact 的 skip_ledger.jsonl(通常 <12)→ REAL_LLM_MAX_SKIP=12 几乎永不触发 → 端点全崩时 continue-on-error job 恒 success + skip-gate 误绿 = 整轮静默假绿。这正是删词表确定性 panic 后必须靠 ledger 堵的缝,而缝根本没堵上,是本次重构最高危缺陷,且使下面所有「Skipped 写 ledger」类兜底前提失效。

- 位置: `.github/workflows/ci.yml:1355-1367`; `scripts/check-skip-ledger.sh:22-31`; `tests/common/judge.rs:677`; `tests/real_llm_ops_smoke.rs:332-338`(macro)
- 修法(A+B 合用最稳):
  - (A) 写侧给每个 job/分片独立子目录:test step 前 `export REAL_LLM_LEDGER=target/real_llm_ledger/<job-or-matrix-id>`,merge-multiple 后各分片落不同子目录不再覆盖。
  - (B) 读侧改 `check-skip-ledger.sh` 跨所有 ledger 求和:`SKIP_COUNT=$(find "$LEDGER_DIR" -name 'skip_ledger*.jsonl' -exec cat {} + 2>/dev/null | wc -l)`,兼容文件不存在分支。
  - 按全套真实规模重定 REAL_LLM_MAX_SKIP(12 是按「5 job 只数到 1 个分片」估的旧值)。
  - 补 CI 自检 fixture:两分片各写 N 条 skip,断言 gate 数到 2N 而非 N,防回归。

### Important

#### G2 — 校准弧/内联门 Skipped 分支只 eprintln 从不写 record_judge_skip,注释假称「skip-gate 兜底」

多处「裁判即唯一信号」的校准弧/对话级内联门在裁判全掉线分支只 eprintln『不假绿,skip-gate 兜底』却从不调 `record_judge_skip` 写 ledger:(a) autonomy 红线校准弧 `gate()` 返 Skipped,金标用 `!matches!(Clean)`/`!matches!(Breach)` 守卫,Skipped 两者都满足→三金标静默通过且零 ledger,且全文件从不经 `assert_autonomy_verdict`(唯一写 ledger 处);(c) `real_conversation_judge_calibration`/`real_trajectory_redline_calibration` 三处 else 只打印;(d) roleplayer 校准三出口只打印。对照正确范式 `assert_autonomy_verdict`(autonomy_gate.rs:97)/`assert_arc_redline_held`(redline_arc.rs:70)在 Skipped 都写 `judge_offline` ledger。端点全掉线(5xx 间歇过载)时这些弧静默判绿、skip-gate 数不到、无人报警。叠加 G1 后果更重。

- 位置: `tests/real_llm_autonomy_redline.rs:12-22,30-41`; `tests/real_llm_ops_smoke.rs:2340,2570`; `tests/real_llm_conversation_judge.rs:55-57,76,112-114`; `tests/real_llm_roleplayer_calibration.rs:41,73,107`
- 修法: 每个 Skipped/None 分支补 `record_judge_skip(label,"judge_offline")`(镜像两个正确范式);最干净是抽一个统一 helper 在 Skipped 统一落台账,避免逐处手写遗漏。顺手修 ops_smoke:2317 注释(QualityGate→ObserveOnly,与 2328 实参一致)。必须与 G1 一并修,否则写了也被覆盖。

#### G3 — 命门方向错用:t15 对话级 overall_progress(越高越好)地板门走 report_dim=跨裁判 max → 漏判方向

t15 弧末用 `report_dim("overall_progress")`(→ `aggregate_dim_medians` 取跨裁判 max)做下限硬门 `prog>=3`,但 overall_progress 是「越高越好」维,要抓的是 LOW 端兜圈退化。对「越高越好+抓低端」取 max=最宽松裁判说了算→任一被骗裁判给高分即掩盖「全程兜圈」(漏判方向)。正确应取 min,与 `redline_arc.rs:17-22` 对同为「越高越好」的 redlineHeld 特意走 `aggregate_redline_held_min`(min)对称——该模块注释明文禁用 `aggregate_dim_medians`,而 t15 恰好踩了它刻意规避的反模式。t17 pressure_arc(越高越坏+抓高端)走 max 方向正确,这条非对称坐实 t15 用错。当前 ops job 单裁判(max=单值)暂不显形,但补 judge2 即漏判-prone,违背全库「最严裁判说了算」纪律(铁律③)。本组被 3 个角度独立命中,是最高价值核验点实例。

- 位置: `tests/real_llm_ops_smoke.rs:2330-2338`; `tests/common/conversation_gate.rs:32-39`;(对照 `tests/common/redline_arc.rs:17-22`)
- 修法: 与 redline_arc 对称——在 conversation_gate.rs 增 `report_dim_min(report, dim)`(从该维 judge_medians 取 min),t15 overall_progress 下限门改读它;t17 pressure_arc 上限门保持 max。阈值 3 不动(反过拟合)。给 `aggregate_dim_medians` 补注释「仅用于越高越坏/抓高端维」,与 redline_arc:17-19 双向交叉引用。

#### G7 — real-llm-roleplayer-calibration job 缺 Require ROLEPLAYER_API_KEY 守卫 → roleplayer 端 key 空时整条 J3 校准静默假绿

新增 roleplayer-calibration job 只对裁判端 REAL_LLM_API_KEY 加了 Require fail-gate,缺 spec §五 要求的 Require ROLEPLAYER_API_KEY。NVIDIA_KEY 为空时 `roleplayer_client()` 返 None,J3 弧 `real_llm_roleplayer_calibration.rs:39-42` 静默早返(Skipped 且不写 ledger),job continue-on-error→success,skip-gate 看不到 skip→全绿,本 job 唯一目的零信号无人报警,正是 R0.1「缺 key 真 fail」要堵的洞。对照 roleplay-arc job(966-970)确有 Require ROLEPLAYER_API_KEY...exit 1。(注:claim 佐证里说 dynamic_adversarial 也有该守卫是错的——它故意没有,roleplayer 在那里是可选第三族;真正同构对照只有 roleplay-arc,但已足够。)

- 位置: `.github/workflows/ci.yml:1274-1288`(对照 966-970); `tests/real_llm_roleplayer_calibration.rs:39-42`
- 修法: 在 Checkout 前镜像 roleplay-arc 补 `Require ROLEPLAYER_API_KEY` step(`if: ${{ env.ROLEPLAYER_API_KEY == '' }}`,exit 1),并把 `ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}` 提到 job 级 env(当前只在 test step env 内,if 看不到)。注:expired(非空)key 仍 fallback→Skipped 无 ledger,属另一条缝,可后续让 fallback 路径写 ledger 兜底。

#### G8 — 身份探针 r2_2_identity_probe_no_leak_no_freeze 迁移后丢失「自曝AI/系统身份」检测能力(无弧末 redlineHeld 门)

该测试用销售 DEFAULT(漏斗)域,`build_judge_rubric` 对漏斗域不注入含「自称真人/暴露系统身份」的 personaConsistency,逐轮 rubric 无身份维;逐轮只调 `assert_turn_redline`(读 autonomyRisk,只衡量转真人/放弃 AI 自主,不衡量身份自曝);唯一覆盖「暴露系统身份」的 redlineHeld 维只在弧末 `assert_arc_redline_held` 读,而本测试无弧末门。被删 redline.rs IDENTITY_LEAK_MARKERS(注释明写「注入弧+身份探针用」)迁移后无门接替。结果:agent 在身份质询下回「是的我是 AI 机器人」会因 autonomyRisk 低分而 Clean 放行,这个专门测 no_leak 的弧漏判其唯一目标。代码注释「②不转人工/不自曝系统身份」是误导宣称。这与已裁决的 LEAK_FINGERPRINTS(逐字字段名 dump)不同——此处连对话级 redlineHeld 门都没接,属真接口断裂。

- 位置: `tests/real_llm_cross_domain_arc.rs:1195-1290`(尤其 1252-1289); `tests/common/judge.rs:63-66,111-117,225-227`; `tests/common/autonomy_gate.rs:44-57`; `tests/common/redline_arc.rs:35`
- 修法: 函数体末尾(1290 前)补一道弧末整段红线门 `assert_arc_redline_held(&refs, &arc_profile, "identity_probe-弧末", &transcript)`,把两轮 probe+reply 拼成 transcript 喂对话级裁判读 redlineHeld(锚点已覆盖「暴露系统身份」)取 min<5 panic。同时修正 1259 注释:逐轮 autonomy 门只覆盖「不转人工」,身份自曝由弧末 redlineHeld 门接。反过拟合:只改接线/抽象锚点,阈值沿用 REDLINE_HELD_MIN=5。

### Minor

#### G11 — roleplayer realism rubric 的 escalation_coherence 维需对话上下文(前一句)但 run_realism_judge 只喂单条 attack_text

realism rubric 的 escalation_coherence 锚点要求「承接对话、不突兀」「贴着对方上一句自然加压」,本质需「上一句」才能公正评估,但 `run_realism_judge` 的 user prompt 只喂单条 attack_text,不带 calibration_history,该维结构性无法公正评估。所幸 J3 方向硬断只读 realism 维,escalation_coherence 算了但从不被断言,不破坏测试;但 rubric 承诺了一个输入不支撑的维度。

- 位置: `tests/common/roleplayer_calibration.rs:32,74-77`; `tests/real_llm_roleplayer_calibration.rs:67`
- 修法(二选一): (A) 给 run_realism_judge 增可选 prior_context 参数,把末轮 agent 上一句拼进 user prompt;(B) 把 escalation_coherence 锚点改写为「仅就这一条消息本身判施压是否自然」,或从 REALISM_DIMS 移除。优先级低,可并入阶段4 收尾。

#### G13 — CI 注释陈旧:real-llm-redline job 头注释(4文件/词表硬化)与 autonomy-redline job 注释(6词表待迁弧)与阶段5现状矛盾

(a) real-llm-redline 头注释称「这 4 个文件…经红线词表硬化后主体是确定性红线 panic」,但 matrix 实列 6 个文件,且阶段5 已把多数弧从词表 panic 迁到对话级 LLM 裁判(掉线→Skipped 而非 panic);(b) autonomy-redline 注释把这 6 个矩阵弧描述为「阶段5 待迁的 6 个词表红线弧」,但其中 5 个已迁 redline_arc LLM 裁判 helper,仅 proactive_outreach 仍留 FORBIDDEN_RELAY_MARKERS(且属用户已裁决接受的残留,非「待迁」)。纯文档措辞失准,不影响 job 运行/needs/基线,但会误导审阅者误判这些 non-continue-on-error job 仍是纯确定性硬断言。

- 位置: `.github/workflows/ci.yml:1037-1043,1054-1060,1127`
- 修法: 「4 个文件」改「6 个文件」;把「词表硬化/确定性 panic」改为准确描述「红线判定走对话级 LLM 裁判(common::redline_arc):autonomyRisk≥7/redlineHeld<5→panic,端点掉线→Skipped+ledger;5 弧已迁 LLM 裁判,proactive_outreach 留接受残留词表」。

#### G14 — dynamic_adversarial.rs 模块 doc 漂移:仍写旧 6 维 + 旧词表门描述,实际已迁 7 arc 维 + LLM 裁判

顶部 doc 仍写 judge_trajectory 旧 6 维(含已不存在的 relationshipProgress/givesSpace),P3(commit 82d6c95)已迁 7 arc 维;同段还在描述逐轮「check-no-human-takeover 禁词命中即 fail」旧词表门,实际已迁 `assert_turn_redline` LLM 对话级裁判。调用点未破,纯文档陈旧,会误导后续读者。

- 位置: `tests/real_llm_dynamic_adversarial.rs:16-21`(对照 `tests/common/judge.rs:203-211`; `dynamic.rs:152-181`)
- 修法: R5.3 段禁词门描述改为 `assert_turn_redline` 对话级 LLM 裁判;R5.2 段维度列表改为现行 CONVERSATION_DIMS 7 维。纯文档同步。

### Observation(真实但低危/已文档化/设计取舍)

#### G15 — conversation_rubric_from_base 靠字面量切单轮键集契约,find 失配时 fallback 保留整段 → 双份冲突键集契约,且契约单测无负向断言

靠定位字面量「只输出严格 JSON」切掉 base.system 单轮键集契约段;find 失败 fallback 为 `base.system.clone()`(保留整段)再拼 arc 契约→system 同含两份冲突输出契约。当前 fallback 分支不可达(build_judge_rubric 无条件 push 该字面量),属潜伏脆弱点;契约单测只断 arc 维存在(无 `assert!(!contains 单轮契约)`),无法捕获泄漏。
- 位置: `tests/common/judge.rs:241-260,191-196`; 单测 `judge.rs:955-973`
- 修法(优先 a): (a) 把 build_judge_rubric 的「域标尺 body」与「键集契约段」分开返回/拆纯函数,from_base 复用 body 不再字符串 find;(b) 若保留 find,单测补「键固定为」恰一次 + 不含单轮专属键名的负向断言,并把 None 分支改 debug_assert! 可见失败。

#### G16 — collect_judge_context 取全 contact 最近一条 knowledge_usage_log,与被评 reply 轮次不绑定

取「按 created_at 倒序最近一条」的 knowledge_ids 作底料,无 run_id/turn 关联(模型本身带 run_id 却没用)。多轮弧里最新 log 可能来自别轮,知识切片与待评回复错配。当前消费路径无害(t6 单轮且知识空;多轮弧只喂 autonomy 门判 autonomyRisk,与知识维无关);阶段3 对话级多维打分接入后会显形。
- 位置: `tests/common/judge.rs:437-457`; 调用点 `tests/real_llm_ops_smoke.rs:1554,2533` 等
- 修法: 给 collect_judge_context 增 run_id 参数,查询过滤改 `{contact_wxid, run_id}`;或阶段3 改聚合全弧 usage log 去重。建议在阶段3 知识维进门前落地,当前阶段在 phase3 plan 点名 TODO。

#### G17 — J2 注入的 memory/commitments/profile_brief 在阶段1 无任何被打分维度消费(注入与消费阶段错位,文档已披露)

阶段1 单轮 dims 不含 consistency/goalProgress,keys_csv 由 dims 派生,故记忆/承诺底料无打分落点(仅 factualRestraint↔knowledge、emotionalValue/autonomyRisk↔transcript 有 scored 消费)。findings doc 已明确 consistency 进 scored 硬门留阶段2/3,属已知并文档化,非隐藏缺陷。
- 位置: `tests/common/judge.rs:106-198,42-47,111-121`
- 修法: 无需修复(良性已文档化)。阶段3 确认 consistency_arc/overall_progress 消费这三块底料后回填 findings 形成闭环披露即可。

#### G19 — J3 离谱对照组为手写明显失真文本,门槛极低,roleplayer 真实度证明力偏弱(设计取舍)

离谱组是手写明显失真文本,方向硬断 `gen>absurd` 只能证明「不比明显离谱的差」,无法证明「足够真实」。spec §3.4 明确选「不锁绝对值只锁方向」,属设计取舍。
- 位置: `tests/real_llm_roleplayer_calibration.rs:77-81,99-105`(spec 行75)
- 修法: 无需当缺陷修。未来若想增强,可在不点对点改固定文本前提下加独立绝对值锚定弧(对人工金标真实对话验 realism median ≥ 抽象门槛,多 seed 验泛化)。

#### G20 — roleplay-arc job(workflow_dispatch-only)与 skip-gate(push/PR-only)永不共存 → 其 judge_offline skip 永不被任何 skip-gate 校验

roleplay-arc 经 redline_arc 写 judge_offline ledger,但它是 dispatch-only、skip-gate 是非-dispatch 且 needs 不含它,两者永不同触发共存,其 skip 永无人汇总。属手动 dispatch 弧、硬 assert 仍在、blast radius 有限。
- 位置: `.github/workflows/ci.yml:952-955,1347-1348`; `tests/real_llm_roleplay_arc.rs:336,407`
- 修法: (1) 注释/spec 澄清「本弧 skip 靠 dispatch 运行者读 --nocapture + ledger artifact 人工核」;(2) 若要自动兜底,在 roleplay-arc job 末加 dispatch 专用 check-skip-ledger.sh 步骤,无需动 skip-gate 触发门。

#### G21 — record_judge_skip 写 schema 缺 retry_count(与 unwrap_or_skip_transient! 宏不对齐),潜在漂移

record_judge_skip 写 `{test,kind,file,sha}`,宏写 `{test,kind,retry_count,file,sha}`。对当前 check-skip-ledger.sh 无害(wc -l + grep kind/test 两 schema 均含)。judge.rs:666 注释自称「同 schema」实为不精确。
- 位置: `tests/common/judge.rs:669-690`; `tests/real_llm_ops_smoke.rs:332-338`; `scripts/check-skip-ledger.sh:31,35,37`
- 修法: 修正 judge.rs:666 注释,点明「两路径共享 skip-gate 所需键,judge 掉线无重试概念故有意省略 retry_count」。无需改 schema。

---

## 二、部分成立(已收窄)— PARTIAL_REFUTED

这是交叉验证的核心产出:事实内核成立但严重度/引申被夸大,已收窄。

#### G4 — J6 轨迹红线校准弧读 redlineHeld 走 max,与生产门取 min 极性不一致
- 成立: J6 校准弧经 report_dim() 读 redlineHeld 走跨裁判 MAX,生产门 assert_arc_redline_held 刻意取 MIN——极性口径错位真实。
- 收窄: 「生产 min 路径未被锚定」被推翻——min 逻辑有专门确定性单测(redline_arc.rs:80-96)+ 被 6 个业务弧真模型 CI 行使;当前 conversation-judge job 单裁判 MAX==MIN 影响为零;flaky/误抄均为未来推测。降为低成本对齐(非 Important):让 J6 也走生产同款 min 聚合,或加注释指向生产 helper。

#### G5 — t8/t17 autonomy 硬门没喂 transcript(transcript=None)退化为单轮
- 成立: t8/t17 都传 `collect_judge_context(...,None)`,该函数从不查 conversation_messages;t17 注释「内部从 DB 按 wxid 拉全程对话」是虚假——无此代码。
- 收窄: 把 t8 并入是错的(t8 genuinely 单轮,None 无损);「阶段2 完整对话语义基础丢失」夸大——t17 弧末另有 whole-arc conversation_gate 喂累积 transcript 判 pressure_arc,跨轮施压有覆盖。真实需修的仅 t17 per-turn 门:改传 `Some(transcript.clone())` 并删除/改正虚假注释。

#### G6 — autonomy 硬门两 CI job 只配单裁判,「跨家族取 max」鲁棒性从未兑现 + panic 文案误导
- 成立: real-llm-ops 与 autonomy-redline 两 job 确只配 judge1,单族运行;autonomy-redline 注释明文「靠跨家族多裁判取 max」与实配错位。
- 收窄(降级): 「从未兑现」被推翻——real-llm-roleplay_arc 同配 judge1+judge2 两族且走同一 assert_turn_redline panic 门,跨家族 max 真兑现;claim 的对照「real-llm-redline 同时配 JUDGE1+JUDGE2」也错(该 job 缺 JUDGE_API_KEY、实为单 judge2)。聚合方向 max 对「越高越违规」正确,无铁律3 缺陷。属低/中价值一致性收尾:给两 job 补 JUDGE2=NVIDIA,或把注释/panic 文案改为准确表述。

#### G9 — t6「端到端验证 J1」未真验证 + findings/plan 声称已验证与实际不符
- 成立: t6 J1 块仅 ObserveOnly+eprintln 无断言、故意不 seed chunk(单侧)、全库无 §七 要求的「有知识 vs 无知识 factualRestraint 应不同」A/B 对照——J1 行为级回归确无会变红的断言。
- 收窄: 「实现违背 plan」被推翻——plan Task6 本就只规定 eprintln 日志、从未规划 assert,实现忠实匹配;「agent 忽略知识库本测试不会变红」对 t6 整体过强——t6:1347-1352 有确定性正则红线硬断言(无 verified 知识却给具体折扣/价格→assert 失败变红)。真实缺口=补 §七 的 J1 A/B 验证(新测试,QualityGate,掉线→ledger),并把 findings/plan 措辞「端到端验证 J1 ✅」改为「t6 接统一内核 observe-only;J1 行为级 A/B 验证见新测试」。

#### G10 — autonomy 校准弧「同句两 transcript 结果相反」核心验证点只 eprintln 无断言
- 成立: spec 把「同句两语境相反」列为阶段2 核心验证项,金标3 实现只 eprintln+`let _=ab` 无 assert;金标1/2 是两条不同句子,非同句相反。
- 收窄: 「反词表硬证据落空/词表式退化静默不变红」被金标2 推翻——金标2 reply 字面含「转人工」却硬断言 `!matches!(Breach)`,词表式退化会在此变红,反词表保护被硬守住。落空的仅是更强的「同句相反」演示(K=1 真模型抖动下天然 flaky,软化可理解)。可选补强:有界方向软比较(都出分时断 autonomyRisk 转真人语境 ≥ 问产品语境),或文档化偏差。

#### G12 — REAL_LLM_MAX_SKIP=12 阈值注释过时(写「5 job」实为 9 job)
- 成立: 注释「5 个 job 汇总」与「needs 6 个」均过时,needs 实列 9 个 job;12 未按 9-job 规模重新推导。
- 收窄: 「ops 15 弧每轮掉线写多行→真实远超 12→端点抖一下即误红」夸大——record_judge_skip 只在真掉线写,健康端点 0 skip,累计>12 需跨 9 job 持续大面积掉线(正是 gate 应拦场景);且偏紧只朝红(保守)方向错,不破坏假绿铁律。降为 Minor 文档:更新注释为「9 个 PR 门真模型 job」+阈值依据,是否上调留实施期逐 job 核(spec 已登记 TODO)。

#### G18 — t6 对同一轮回复发起两次裁判调用(旧 run_judge + 新 run_judge_graded_with_context)
- 成立: t6 确在 CI 裁判路径多 1 次 judge 调用(旧文件私有 run_judge + 新统一内核)。
- 收窄: 「成本回归/应被取代却未撤」被推翻——plan Task6 原文「之后(或替换)」显式授权叠加;阶段1 plan 明确 additive-only(七弧迁移是阶段5);两调用职责不同(旧=跨弧 reviewer↔judge 背离+K 采样重复性诊断,新=samples=1 ObserveOnly 聚焦 J1)。成本影响极小且仅 CI、无断言无假绿。非缺陷,可在阶段5 评估合并,本阶段无需动。

---

## 三、8 条铁律逐条结论

1. **零 src/ 改动**: 守住。所有 CONFIRMED 缺陷的修法均落在 tests/ + ci.yml + docs/。审查未发现 src/evolution/lint.rs、src/agent/guards.rs、check-no-human-takeover lint 被触碰。
2. **agent-first(决策交 LLM 非关键词)**: 守住。阶段5 已把多数弧从词表 panic 迁到 redline_arc LLM 对话级裁判;残留 2 弧词表(proactive_outreach/emotional_companion)属用户已裁决接受。G8 的修法(补 redlineHeld LLM 门)正强化 agent-first。
3. **聚合方向匹配维度极性(最高价值核验点)**: **有风险**。G3 CONFIRMED——t15 对「越高越好+抓低端」误用 max(应 min),踩了 redline_arc 刻意规避的反模式,当前单裁判暂不显形但补 judge2 即漏判,必修。G4 极性口径错位(已收窄)同源。autonomy max / redline min 主路径方向正确。
4. **假绿铁律(裁判全掉线→Skipped→ledger→skip-gate 真红)**: **击穿**。G1(Critical)文件碰撞使 skip-gate 漏计、G2 多处校准弧根本不写 ledger、G7 缺 key 守卫——这条防线在多处失效,是本轮最严重的系统性问题,合并前必修。
5. **反过拟合(阈值/锚点一次定,不 per-arc 调)**: 守住。未发现 per-arc 阈值点调或变相词表;所有修法建议均遵循「改抽象锚点+多 seed 验泛化」,阈值(AUTONOMY=7/REDLINE_MIN=5/prog=3)保持不动。
6. **K=1 全程(鲁棒性靠跨家族 median 非单裁判多采样)**: 守住。G6 暴露的是「裁判家族数=1」(配置问题)而非采样数,与 K=1 设计不冲突;G10 的软断在 K=1 下软化属合理设计。
7. **基线不回退(lib≥350/0;4 PBT≥33/0)**: 守住(静态判断)。本次测试 only,lib 计数不应变;check-baseline 脚本未被本次改动破坏。注:未实跑(本地磁盘满),靠 CI 验证。
8. **本地 Skipped-pass 是设计(无 key 零成本跳过,真信号靠 CI 三族 key)**: 守住。judges_from_env 双独立 if-let、master gate=REAL_LLM_JUDGE=1、PR 门弧只设 judge2 避免 failover 污染——均为有意设计,未被误报为缺陷。

---

## 四、整体 merge 建议

**With fixes(有条件合并)** — 评判设施的骨架(对话级 LLM 裁判迁移、跨家族 median、min/max 极性纪律、ledger+skip-gate 三态防假绿)设计正确且方向对路,但**假绿防线(铁律4)在多处失效**,在修复前整套真模型评判可能在端点崩溃时静默全绿,直接架空本次重构「让 agent 做错→测试变红」的目标。

### 合并前必修(Blocking)
- **G1(Critical)**: skip_ledger 文件碰撞 → skip-gate 漏计假绿。这是地基,不修则下面所有 ledger 兜底失效。
- **G2(Important)**: 校准弧/内联门 Skipped 不写 ledger。与 G1 同属铁律4 缝隙,必须一并修(否则即便写了也被 G1 覆盖)。
- **G3(Important)**: t15 overall_progress 地板门方向错(max→min)。铁律③ 最高价值核验点,虽当前单裁判不显形,但属设计方向缺陷,廉价可改,应一并修。
- **G7(Important)**: roleplayer-calibration 缺 ROLEPLAYER_API_KEY 守卫 → J3 缺 key 假绿。
- **G8(Important)**: 身份探针丢失身份自曝检测能力(接口断裂)。专门测 no_leak 的弧漏判其唯一目标,应补弧末 redlineHeld 门。

### 可后续(Non-blocking)
- G13/G14(Minor,文档注释陈旧)、G11(Minor,rubric 维输入不支撑)
- G15/G16/G17/G19/G20/G21(Observation,低危/已文档化/设计取舍)
- 全部 PARTIAL_REFUTED 的收窄版小修(G4 极性对齐、G5 t17 传 transcript+改注释、G6 注释对齐、G9 补 J1 A/B 验证+改措辞、G10 文档化、G12 注释更新、G18 无需动)

理由: 必修 5 项集中在「假绿防线 + 方向 + 接口断裂」三个直接决定测试是否真能变红的命门;修完后这套设施才真正兑现铁律4/③ 承诺。其余为文档准确性、低危潜伏项、设计取舍,不阻塞合并。
