# 真实 LLM 测试「真实性」审计

> 2026-06-16。用户要求：凡涉及 LLM 的测试必须是真实任务/真实业务流程，不接受 mock/桩/skip 出来的假绿。本表审计所有名义"真实 LLM"测试，定级「真跑 / 条件 skip / 弱断言 / 假绿风险」，并给修复清单。配套 `docs/universal-domain-test-gap-audit.md`（覆盖缺口）。

## 一、贯穿全局的三个真实性漏洞（根因，最该先堵）

1. **缺 key 静默全绿**：所有 5 个 real_llm 文件的 `require_real_llm!` 宏在无 `REAL_LLM_API_KEY` 时是 `eprintln + return`（PASS，不 fail）。CI 若缺 key，200+ 个 `#[ignore]` 真模型测试**全部静默返回 ok**。定义点：`real_llm_ops_smoke.rs:282` / `adversarial:262` / `smoke:93` / `emotional:229` / `calibration:212` / `knowledge:85` / `quality:248` / `recall:54`。
   → **修复**：CI 的每个 real-llm job 显式断言 key 非空，缺失即 job fail，而非靠 test 内 skip。这是所有假绿的总开关。

2. **瞬时失败吞绿**：`unwrap_or_skip_transient!` 遇 `AppError::LlmUnavailable`（429/超时/5xx/connect，client 重试耗尽后）→ `eprintln + return`（测试 PASS，零断言执行）。它包住几乎每个测试的**核心 agent 调用**，所以 turn-1 撞 NVIDIA 429 = 绿但什么都没测。这正是已实测的 t12/ops 假绿模式。
   → **修复**：核心链路重试耗尽后应 fail（或把 skip 计数/原因落 ledger，CI 设单 job skip 率上限作硬门）。`http_4xx` 尤其危险——reviewer/judge 端点配错（漏 /v1）返 4xx 会被当 transient 吞掉（正是 405 案例）。

3. **judge 失败不影响 pass**：judge/裁判调用失败时一律 `eprintln + return None`，测试照 pass（`quality handle_verdict:1151` / `emotional:462` / `adversarial t_judge_calibration` 整测零断言）。judge 静默缺失 = 质量无人把关却绿。
   → **修复**：judge 是"仪器"——它失败应区分（红线测试不依赖 judge 的，judge 挂只丢观测，可接受；但以 judge 为唯一质量门的测试，judge 失败必须 fail 或落 CI 门）。

## 二、定级汇总（运营侧）

真实性最高（范式参照）：`smoke t2`（引用接地 cited⊆seed）、`calibration`（对称阈值契约+真 reviewer）、`ops t11`（consolidation 落库断言）。

| 测试 | 定级 | 关键问题 file:line |
|---|---|---|
| ops t4 跟进任务 | 真跑+弱断言 | 仅断 status 闭集；live 分支无行为断言 `ops_smoke.rs:1066` |
| ops t5 状态机 | 弱断言 | operation_state∈集但 `if let Some` 守卫，None 放过 `:1182` |
| ops t6 产品门 | 条件强断言 | sent 则 reply 不含价格数字（真红线）；None 分支放过 `:1267` |
| ops t7 多场景泛化 | **弱断言** | "导出所有客户微信号"越界场景**完全不校验是否拒绝** `:1296` |
| ops t8 autonomy 闭集 | **弱断言** | "承诺转真人"红线只 eprintln 不断言 `:1404` |
| ops t9 用户反应 | 弱断言+易逃逸 | 无 sent review 即 return 逃逸 `:1482` |
| ops t10 画像生成 | 真跑中等 | profile 至少一字段非空 `:1573` |
| ops t11 记忆整理 | **真跑强断言** | status==consolidated + version≥1 `:1687` |
| ops t12 可操控性 | 弱断言 | "先问预算"指令遵守只 eprintln（format 已修） `:1807` |
| ops t13 千人千面 | 真跑最小 | reply_a≠reply_b（仅逐字不等，无实质差异度量） `:1911` |
| ops t14 画像弱信号 | 弱断言 | 画像翻转/丢标签全 eprintln `:2023` |
| ops t15 跌单弧 | 真跑中等+skip高 | approved_turns≥2 红线；turn-1 抖动整测 skip `:2150/2100` |
| ops t16 人格交叉 | 真跑中等+skip | snapshot 硬断言+reply 不等 `:2254` |
| ops t17 边界压测 | **弱断言+bug** | handoff 只 eprintln 且查 `prev_reply` 非当轮（逻辑错位） `:2337` |
| ops t18 暖启动 | **弱断言+bug** | "别推销"全 eprintln 且查 prev_reply `:2457` |
| adversarial t_adv_*（6 条） | **假绿风险** | 全观测-only，gateway Err 不腰斩，autonomy/越狱/编造零行为硬断言 `adversarial.rs:1075/1165` |
| adversarial t_judge_calibration | **假绿** | 自称退出门却**零 assert**，judge 全 405 照 pass `:1551` |
| adversarial t_longrun | 弱断言 | 记忆/画像漂移全 eprintln `:1721` |
| smoke t1 决策-审查链 | 真跑中等 | outbox→Sent 真 e2e `smoke.rs:364` |
| smoke t2 知识 tool-loop | **真跑强断言** | rounds≥1+answer非空+首工具 list_catalog+cited⊆seed `:465` |
| smoke t3 vision | 真跑条件红线 | 落库必 draft+needs_review；0chunk 空过 `:578` |
| emotional p2 陪伴弧 | 真跑有红线+多观测 | 禁词(转人工/我是AI)红线+不复读真断言；approved_turns<3 只 ledger `emotional.rs:664/998` |
| calibration 高压对照 | **真跑最强** | 对称阈值契约+真 reviewer；隐患:reviewer 4xx 会被 transient 吞 `calibration.rs:627` |

## 三、定级汇总（知识库侧）

真实性最高：`q2 抽取质量`（确定性 recall floor 独立于 judge）、`k3 无幻觉`、`k7/k8/k10 红线`、`smoke t2`。

| 测试 | 定级 | 关键问题 file:line |
|---|---|---|
| knowledge k1/k2 | 真跑+条件skip | 硬断 open_chunk/关系触达；429 静默 skip `knowledge.rs:304/423` |
| knowledge k3 无幻觉 | 真跑 | cite⊆seed 红线+recall_miss 闭环 `:543` |
| knowledge k4 未验证不服务 | 真跑(弱正向) | 模型 cite 空时断言真空成立 `:618` |
| knowledge k6 vision 抽取 | **弱断言/假绿** | chunk_ids 空时 for 空转无断言直接 PASS `:806` |
| knowledge k9 标签 | 弱断言 | 仅断两字段是数组，不验标签质量 `:1065` |
| knowledge k5/k7/k8/k10/k11 | 真跑(红线) | 各有 draft/needs_review/不落库/cite 红线 |
| quality q2 抽取 | **真跑最强** | 确定性 recall floor + 泛化 gap 门独立于 judge `quality.rs:1991` |
| quality q3 vision | **弱断言/假绿** | 0 抽取 judge 前 return；质量全靠可 skip 的 vision judge `:2147` |
| quality q7 标签 | 弱断言 | 仅断两数组 shape `:2502` |
| quality q8 诚实弃答 | 真跑(弱) | answer非空+cite⊆seed；honesty 全靠可 skip judge `:2559` |
| quality q1/q4/q5/q6 | 真跑(红线+judge可skip) | 各有确定性红线恒跑 |
| recall_benchmark_smoke | **弱断言** | 跑一次 answer 仅 eprintln，**无任何召回断言** `recall.rs:677` |
| recall_benchmark_cross_industry | **假绿风险** | 自称"召回率基准"却对召回率/稳定性零硬断言 `:906` |
| recall_benchmark_maintenance | 弱断言 | 漂移全 SOFT-WARN eprintln 无阈值 `:1167` |
| recall_benchmark_gap_closed_loop | 真跑(本文件最强) | 补库 chunk verified+reach 命中主断言 `:1515` |

**不算真 LLM 测试（mock 桩，定位正确无需改）**：`knowledge_agent_eval`（push_response mock）、`knowledge_ask_e2e`（mock）、`knowledge_router_fallback_e2e`（mock）、`knowledge_closed_loop_trajectory`（纯 DB 不调 LLM）、各文件非 #[ignore] 离线纯函数单测。

## 四、修复优先级

**P0 总开关（一次性堵假绿根因）**
1. CI real-llm job 显式断言 key 非空 → 缺失即 fail（堵漏洞①）。
2. transient-skip 可观测化：skip 原因/计数落 ledger，CI 设单 job skip 率上限硬门（堵漏洞②，防 429 假绿复发）。
3. reviewer/judge 端点调用改 `.expect`/panic 而非 transient-skip（4xx 不该被当抖动吞——405 案例根治）。

**P1 红线降级为 eprintln 的必须升级为硬断言**
4. ops t7 越界导出场景、t8/t17/t18 转真人红线 → 命中即 fail；并修 t17/t18 查 `prev_reply` 而非当轮的错位 bug。
5. adversarial 6 条对抗弧 → 至少 takeover 弧（HANDOFF/AUTHORITY 命中 fail）、injection 弧（泄露系统设定 fail）加红线硬断言。
6. adversarial t_judge_calibration → effective_hit_rate 低于阈值改硬断言（否则它不是门）。

**P2 弱断言补实质度量**
7. recall_benchmark_cross_industry/smoke/maintenance → 加 recall@k 下限 + 跨轮稳定占比 + 漂移率上限。
8. vision 类（k6/q3）→ 0 抽取区分"真空能力" vs "端点故障 skip"，不默绿。
9. ops t13/t16 千人千面 → reply 不等升级为实质差异度量（术语/共情词分布）。

## 五、反过拟合边界

- 红线硬断言走"命中禁忌即 fail"（如转真人、泄露系统提示、价格数字），不锁单条措辞（[[no-overfitting-methodology]]）。
- 离线纯函数单测、mock 集成测**不动**——它们定位正确，是确定性地基。
- 修真实性 ≠ 调严断言到过拟合；目标是"让假绿暴露成真相"，不是"让测试更难过"。
