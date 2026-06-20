# 评判体系重构 — 5 个 Blocking 修复设计

> 承 5 阶段评判体系重构的交叉验证终审（`docs/superpowers/findings/2026-06-21-eval-overhaul-5phase-cross-verify.md`）。终审 CONFIRMED 14 / PARTIAL_REFUTED 7 / REFUTED 0，主控独立读代码核实了 5 个「合并前必修」Blocking。本设计只修这 5 项；可后续项（G13/G14/G11 文档 + 6 Observation + 7 PARTIAL 收窄小修）不在本次范围。

## 一、问题（要根治什么）

评判设施的骨架（对话级 LLM 裁判迁移、跨家族 median、min/max 极性纪律、ledger+skip-gate 三态防假绿）方向正确，但**假绿防线（铁律4）在多处失效** + **一处聚合方向错（铁律③）** + **一处接口断裂**。在修复前，端点崩溃时整套真模型评判可能静默全绿，直接架空本次重构「让 agent 做错 → 测试变红」的目标。

5 个 Blocking（均经主控独立读代码核实，非仅报告转述）：

| ID | 级别 | 根因 | 铁律 |
|---|---|---|---|
| G1 | Critical | `skip_ledger.jsonl` 跨 9 job 同名 → `merge-multiple` last-wins 覆盖 + `check-skip-ledger.sh` `wc -l` 单文件 → 跨 job 几十条 skip 只数到 1 分片 → 端点全崩静默假绿 | 4（假绿防线地基） |
| G2 | Important | 校准弧/内联门 Skipped/None 分支只 `eprintln` 不写 `record_judge_skip` → 端点掉线静默判绿、skip-gate 数不到 | 4 |
| G3 | Important | t15 `overall_progress`（越高越好+抓低端）地板门走 `report_dim`=跨裁判 max（应 min）→ 漏判方向 | ③（聚合方向） |
| G7 | Important | `real-llm-roleplayer-calibration` job 缺 `Require ROLEPLAYER_API_KEY` 守卫 + key 只在 step env（job 级 if 看不到）→ NVIDIA key 空时 J3 静默假绿 | 4 / R0.1 |
| G8 | Important | 身份探针 `r2_2_identity_probe_no_leak_no_freeze` 无弧末 redlineHeld 门 → 丢失「自曝 AI/系统身份」检测（迁移后 IDENTITY_LEAK_MARKERS 无门接替）→ 专测 no_leak 的弧漏判其唯一目标 | 接口断裂 |

## 二、目标与边界

**目标**：修复上述 5 个 Blocking，恢复铁律4 假绿防线 + 铁律③ 聚合方向 + G8 接口完整。

**边界**：
- **零 src/ 改动**：全部落 `tests/` + `.github/workflows/ci.yml` + `scripts/` + `docs/`。不碰被测 agent prompt、生产护栏（`src/evolution/lint.rs` / `src/agent/guards.rs`）、`check-no-human-takeover` lint。
- **反过拟合（铁律③）**：阈值/锚点一次定，全部不动（`AUTONOMY_HARD_THRESHOLD=7` / `REDLINE_HELD_MIN=5` / `T15_MIN_PROGRESS=3`）。只改接线、聚合算子方向、注释。`REAL_LLM_MAX_SKIP` 的重估是按真实 job 规模校正旧错值，非朝单次结果点调。
- **agent-first**：G8 的修法（补 redlineHeld LLM 门）正强化 agent-first（语义判断交 LLM），不引入新词表。
- **本地磁盘满不编译**：本地 100% 满，不跑 `cargo build/test`。靠 `cargo check`（名称解析，若磁盘允许）+ CI 验证。
- **不扩范围**：可后续项（文档陈旧 G13/G14、rubric 维 G11、Observation、PARTIAL 收窄）本次不动。G7 只补缺 key 守卫，expired-key fallback 缝留后续。

**执行粒度**：4 个 Task。**Task A = G1 + G2**（同一假绿防线，强耦合：G2 写了 ledger 若 G1 没修则被覆盖，必须一起验）；**Task B = G3**、**Task C = G7**、**Task D = G8**（相对独立）。

## 三、设计

### Task A · G1（写侧子目录 + 读侧递归求和 + 重定阈值 + 回归 fixture）

**现状**（已亲验）：
- 写侧：`tests/common/judge.rs:677` `record_judge_skip` 与 `tests/real_llm_ops_smoke.rs:327` `unwrap_or_skip_transient!` 宏都 `open(format!("{dir}/skip_ledger.jsonl"))`——同名，无 job/matrix 后缀。
- CI 收集：9 个 PR 门 job 各 `upload-artifact`，artifact 名不同（`real-llm-ledger-autonomy-redline` / `real-llm-ledger-conversation-judge` / `real-llm-ledger-redline-${{ matrix.file }}` / `real-llm-ledger-${{ matrix.q }}` / `real-llm-skip-ledger-*` 等），但每个 artifact **内部文件都叫 `skip_ledger.jsonl`**。
- skip-gate（`ci.yml:1356-1360`）：`download-artifact ... pattern:"*ledger*" merge-multiple:true path:target/real_llm_ledger` → 多 artifact 同名文件合进同目录 last-wins **覆盖**。
- 读侧（`scripts/check-skip-ledger.sh:23-25`）：`LEDGER="$LEDGER_DIR/skip_ledger.jsonl"` + `wc -l < "$LEDGER"` 只数单一固定路径。

**修法**：
1. **写侧给每 job 独立子目录**（`ci.yml` 各 real-llm job 的 test step `env`）：把 `REAL_LLM_LEDGER` 设为带 job/matrix 后缀的子目录。**注意现状不一致**（已亲验）：
   - 已显式设 `REAL_LLM_LEDGER: target/real_llm_ledger` 的 6 处：quality（:640）、adversarial（:751）、redline（:1105）、autonomy-redline（:1184）、conversation-judge（:1254）、roleplayer-calibration（:1325）→ 改为带后缀子目录。
   - **当前未显式设、走脚本默认值的 3 个 job**：smoke（real-llm，test step :255-264 无 `REAL_LLM_LEDGER`）、recall（:359-365）、ops（:457-466）→ **需新增** `REAL_LLM_LEDGER` env 指向各自子目录（否则它们仍写默认根目录、互相覆盖）。
   - 子目录命名：单 job 用 job 名（如 `target/real_llm_ledger/autonomy-redline`）；matrix job 用 `<job>-${{ matrix.x }}`（redline=`redline-${{ matrix.file }}`、ops=`ops-${{ matrix.t }}`、recall=`recall-${{ matrix.t }}`、quality=`quality-${{ matrix.q }}`、adversarial=`adversarial-${{ matrix.arc }}`）。各 job `upload-artifact` 的 `path` 仍传 `target/real_llm_ledger/`（含子目录），artifact 名不变。同一 matrix 的多分片各自子目录互不覆盖。
2. **读侧跨所有分片求和**（`check-skip-ledger.sh`）：
   ```sh
   SKIP_COUNT=$(find "$LEDGER_DIR" -name 'skip_ledger*.jsonl' -exec cat {} + 2>/dev/null | wc -l | tr -d ' ')
   ```
   文件不存在时 `find` 输出空 → `wc -l` 得 0，保留「0 skip OK」分支（改为：先 `find` 求和，`SKIP_COUNT` 为空或 0 → 视作 0 skip）。按 kind/test 分布的 `grep -oE` 也改为对 `find ... -exec cat {} +` 的输出统计。
3. **重定阈值**：`ci.yml:1364` 的 `REAL_LLM_MAX_SKIP`（现 12，按「5 job 只数到 1 分片」旧估）按真实 9 个 PR 门 job 规模重估。每 job 套件 ~3-16 测试，健康端点 0 skip；阈值取「跨 9 job 偶发抖动可容忍、持续大面积掉线即红」的经验值（建议 ~20，实施时按 job 数 × 单 job 容忍量核定，写进注释依据）。同步更新 :1362-1363 过时注释（「5 个 job」→ 9 个 PR 门 job）。
4. **回归 fixture**（防 G1 再发）：在 `check-skip-ledger.sh` 加一段可选自检（或独立小脚本 `scripts/test-skip-ledger.sh`），由一个轻量 CI step 或脚本 `--self-test` 触发：临时 `LEDGER_DIR` 下造两个子目录 `job-a/skip_ledger.jsonl`、`job-b/skip_ledger.jsonl` 各写 N 行，断言 `SKIP_COUNT==2N`（而非 N）。证明递归求和真跨分片，防回归退化回 last-wins。

**反过拟合**：阈值重估是校正「按错误的单分片计数估的旧值」，非针对某次 CI 结果点调；fixture 锁的是「跨分片求和」这个抽象行为。

### Task A · G2（抽统一 helper，仅 CI 掉线写 ledger）

**现状**（已亲验）：多处「裁判即唯一信号」的弧在裁判全掉线分支只 `eprintln`：
- `tests/real_llm_autonomy_redline.rs:12-22`：`gate()` 在 `judges.is_empty()` 早返 Skipped（本地无 key）；CI 有 key 但端点掉线时 `run_autonomy_redline_gate` 返 Skipped，金标用 `!matches!(Clean)` / `!matches!(Breach)` 守卫——**Skipped 两者都满足 → 静默通过，且全程不经 `assert_autonomy_verdict`（唯一写 ledger 处）**。
- `tests/real_llm_ops_smoke.rs:2340`：t15 弧末 `report_dim → None` 分支只 eprintln。
- `tests/real_llm_conversation_judge.rs:55-57,76,112-114`：校准弧三处 else 只打印。
- `tests/real_llm_roleplayer_calibration.rs:41,73,107`：三出口只打印。

**关键区分**（避免误修，主控核出的设计要点）：
- **本地无 key**（裁判工厂返空，零成本设计跳过）→ **不写 ledger**。本地跑测试不该污染 `target/real_llm_ledger`。
- **CI 有 key 但端点掉线**（裁判真跑了但全掉线/没出分）→ **真缝，必写 ledger**。

**难点（已亲验，spec 第一版假设有误，此处修正）**：各文件「全掉线」分支结构不同，且**多数分支拿不到 `judges`**：
- `autonomy_redline.rs`：判定在 `gate()` **内部**（:13 取 `judges`），但 `judges.is_empty()` :14-16 早返 Skipped（本地）、CI 掉线 `run_autonomy_redline_gate` 也返 Skipped——**调用方无法区分两种 Skipped**。
- `conversation_judge.rs`：`judge()`（:13-22）本地无 key 返 `ConversationReport{any_scored:false}`（空 report，**非 Skipped 枚举**）；三处 else（:55,:75,:112）触发于 `report_dim` 返 None，None **混合**「本地空 report」与「CI 裁判掉线」两源，且 `judges` 是 `judge()` 局部变量、外层 else 拿不到。
- `roleplayer_calibration.rs`：`judges`/`rp` 在测试函数作用域（:37-38）；:39-42 早返（本地无 key/缺第三族）、:71-74 `all_fallback`（CI 有 key 但 roleplayer 端点全挂）、:96 `_=>`（CI 裁判掉线）——后两处 `judges` 在作用域内可直接判。

**修法（按真实结构分治，记账下沉到能区分本地 vs CI 的那一层）**：
1. **统一 helper**（放 `tests/common/judge.rs`，与 `record_judge_skip` 同文件）。签名用 `bool` 而非 `&[...]`——调用方在能取到 judges 处传 `!judges.is_empty()`，封装了 judges 的函数自己回传「真跑了裁判」：
   ```rust
   /// 仅当 judged==true(有 key、真跑了裁判)但全掉线时写 skip ledger;
   /// 本地无 key 不写(否则本地跑测试污染 ledger + 误报)。
   pub fn record_arc_skip_if_judged(judged: bool, label: &str) {
       if judged {
           record_judge_skip(label, "judge_offline");
       }
   }
   ```
2. **conversation_judge.rs**（关键修正）：三处 else 拿不到 judges，故记账下沉进 `judge()`：本地无 key 早返（:15-17）保持不写；CI 有 key 但 `run_conversation_judge` 回来 `any_scored==false` 时，在 `judge()` 内部（`judges` 在作用域）调 `record_arc_skip_if_judged(true, label)`。三处 else 无需改、eprintln 保留（人类可读）。
3. **autonomy_redline.rs**：`gate()` 内 :14 本地早返保持不写；CI 路径 `run_autonomy_redline_gate` 返 Skipped 时，在 `gate()` 内（judges 非空已知）调 `record_arc_skip_if_judged(true, label)` 再返回。调用方金标 `!matches!` 守卫不变。
4. **roleplayer_calibration.rs**：:39-42 早返不写；:71-74 `all_fallback`、:96 `_=>` 两处补 `record_arc_skip_if_judged(true, label)`（此处已知 judges 非空、真跑了）。
5. **ops_smoke.rs t15**（:2340 None 分支）：`judges` 在作用域（:2322），补 `record_arc_skip_if_judged(!judges.is_empty(), "t15-成交弧")`。顺手修 :2317 注释（QualityGate→ObserveOnly，与 :2328 实参一致）。

**与 G1 同 Task**：G2 写了 ledger，若 G1 未修则被同名覆盖、skip-gate 仍数不到——必须同 Task 一起验证才有意义。

**正确范式对照**（已存在、本次复用）：`assert_autonomy_verdict`（autonomy_gate.rs:97）/ `assert_arc_redline_held`（redline_arc.rs:71）在 Skipped/None 分支已调 `record_judge_skip`。

### Task B · G3（conversation_gate 加 report_dim_min，t15 改读 min）

**现状**（已亲验）：`tests/real_llm_ops_smoke.rs:2330` t15 弧末 `report_dim(&report, "overall_progress")` → `aggregate_dim_medians`（conversation_gate.rs:32 `.max()`）做下限门 `prog >= 3`。overall_progress「越高越好」、要抓 LOW 端兜圈退化 → 取 max（最宽松裁判）漏判。t17 pressure_arc（越高越坏+抓高端）走 max 方向正确，非对称坐实 t15 用错。对照 `redline_arc.rs:17-22` 对同为「越高越好」的 redlineHeld 特意走 `aggregate_redline_held_min`（min），且注释明文禁用 `aggregate_dim_medians`。

**修法**：
1. `tests/common/conversation_gate.rs` 加纯函数 `report_dim_min(report, dim) -> Option<i64>`：从该维 `ConversationVerdict.judge_medians` 取 `.min()`（镜像 `aggregate_redline_held_min`；空 → None）。
2. t15 的 overall_progress 下限门改读 `report_dim_min`；**t17 pressure_arc 上限门保持 `report_dim`（max）**。
3. 阈值 `T15_MIN_PROGRESS=3` **不动**；t15:2332 的「跨裁判 max median」eprintln 文案改「min」。
4. 给 `aggregate_dim_medians`（conversation_gate.rs:31-34）补注释「仅用于越高越坏/抓高端维；越高越好/抓低端维须用 report_dim_min」，与 redline_arc.rs:17-19 双向交叉引用，防后人再踩。
5. 纯函数单测：`report_dim_min` 取 min（`judge_medians=[8,3,6] → 3`、空 → None），与现有 `aggregate_redline_held_min` 单测同款。

**当前不显形但必修**：ops job 单裁判时 max==min，但补 judge2 即漏判——属设计方向缺陷，廉价可改。

### Task C · G7（roleplayer-calibration 补 ROLEPLAYER_API_KEY 守卫）

**现状**（已亲验）：`ci.yml:1274-1335` `real-llm-roleplayer-calibration` job：job 级 env（:1281-1282）只有 `REAL_LLM_API_KEY`；`Require` step 只有 `REAL_LLM_API_KEY`（:1284-1288）；`ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}` 只在 test step env（:1322）。NVIDIA_KEY 空 → `roleplayer_client()` 返 None → `real_llm_roleplayer_calibration.rs:39-42` 静默早返 → continue-on-error success → 本 job 唯一目的零信号。对照 roleplay-arc（:957-959 job 级 env 有 ROLEPLAYER_API_KEY + :966-970 `Require ROLEPLAYER_API_KEY` step）。

**修法**（镜像 roleplay-arc）：
1. 把 `ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}` 提到 **job 级 env**（:1281-1282），使后续 `if: ${{ env.ROLEPLAYER_API_KEY == '' }}` 可见。
2. 在 Checkout 前补 `Require ROLEPLAYER_API_KEY` step（`if: ${{ env.ROLEPLAYER_API_KEY == '' }}` → `exit 1`），文案照 :966-970（R0.1 缺第三族 key 真 fail）。
3. test step 内重复的 `ROLEPLAYER_API_KEY`（:1322）删除（已 job 级继承）；其它 `if: ${{ env.REAL_LLM_API_KEY != '' }}` 守卫的 step 可顺带加 `&& env.ROLEPLAYER_API_KEY != ''`（与 roleplay-arc:976 一致），保证缺第三族 key 时跳过编译跑测试。

**范围**：只补缺 key 守卫。expired（非空但失效）key 仍 fallback→Skipped 无 ledger 是另一条缝——Task A 的 helper 落地后，让 roleplayer fallback 路径也走 `record_arc_skip_if_judged` 可顺带兜底，但不强制；G7 本身不扩到 expired。

### Task D · G8（身份探针补弧末 redlineHeld 门）

**现状**（已亲验）：`tests/real_llm_cross_domain_arc.rs:1197` `r2_2_identity_probe_no_leak_no_freeze` 用销售 DEFAULT 域，逐轮只 `assert_turn_redline`（:1268，读 autonomyRisk——不衡量身份自曝）；无弧末 `assert_arc_redline_held`。唯一覆盖「暴露系统身份」的 redlineHeld 维（judge.rs:225 锚点已覆盖）只在弧末读，本测试没接。被删 redline.rs `IDENTITY_LEAK_MARKERS`（注释「注入弧+身份探针用」）迁移后无门接替。结果：agent 答「是的我是 AI 机器人」因 autonomyRisk 低分 Clean 放行，专测 no_leak 的弧漏判唯一目标。:1259 注释「②不转人工/不自曝系统身份」误导。

**与已裁决缺口区别**：非 LEAK_FINGERPRINTS（逐字内部字段名 dump，词表才精确抓、已裁决留生产出站守卫）。身份自曝（「我是 AI/机器人」）是**语义红线**，redlineHeld 锚点已覆盖，属 LLM 裁判优势场景——只是没接线。真接口断裂。

**修法**（只补接线，阈值不动）：
1. 函数体末尾（:1290 前）补弧末门：循环内已收集 `replies`，补一个有序 transcript 串（「客户: {probe}\n助理: {reply}」按轮拼接）；调 `assert_arc_redline_held(&refs, &arc_profile, "identity_probe-弧末", &transcript)`（`arc_profile` 用 DEFAULT 域 profile，`refs` 复用逐轮已构造的裁判切片）。`assert_arc_redline_held` 内部读 redlineHeld 取 min < 5 panic、裁判全掉线 → 已写 ledger（redline_arc.rs:71 现成）。
2. 前置守卫照其它弧：`if !judges.is_empty() && transcript 非空`。
3. 修正 :1259 注释：逐轮 autonomy 门只覆盖「不转人工」，身份自曝由弧末 redlineHeld 门接。
4. 反过拟合：只改接线/复用已有抽象锚点，阈值沿用 `REDLINE_HELD_MIN=5`。

## 四、测试落地

| 文件 | 改动 |
|---|---|
| `scripts/check-skip-ledger.sh`（改） | 读侧 `find ... -exec cat {} + | wc -l` 跨分片求和 + kind/test 分布改 find；加 `--self-test` 回归 fixture（两子目录各 N 行断言 2N） |
| `.github/workflows/ci.yml`（改） | 各 real-llm job test step `REAL_LLM_LEDGER` 加 job/matrix 子目录；`REAL_LLM_MAX_SKIP` 重估 + 注释更新；roleplayer-calibration job 提 `ROLEPLAYER_API_KEY` 到 job 级 env + 补 `Require` step + step 守卫加 `&& ROLEPLAYER_API_KEY != ''` |
| `tests/common/judge.rs`（改） | 加 `record_arc_skip_if_judged(judged: bool, label)` helper（judged=真跑了裁判才写 ledger，与 `record_judge_skip` 同文件）+ 纯函数/语义单测 |
| `tests/common/conversation_gate.rs`（改） | 加 `report_dim_min(report, dim)`（取 min）+ 单测；`aggregate_dim_medians` 补「仅越高越坏维」注释 |
| `tests/real_llm_autonomy_redline.rs`（改） | `gate()` 内 CI 掉线（judges 非空且返 Skipped）补 `record_arc_skip_if_judged(true,..)`；:14 本地早返不写 |
| `tests/real_llm_ops_smoke.rs`（改） | t15 overall_progress 改读 `report_dim_min` + None 分支补 `record_arc_skip_if_judged(!judges.is_empty(),..)`；:2317 注释修正 |
| `tests/real_llm_conversation_judge.rs`（改） | 记账下沉进 `judge()`（CI 有 key 但 `any_scored==false` 写 ledger，本地无 key 早返不写）；三处 else 不改、eprintln 保留 |
| `tests/real_llm_roleplayer_calibration.rs`（改） | :71 `all_fallback` 与 :96 裁判全掉线两处补 `record_arc_skip_if_judged(true,..)`；:39 本地早返不写 |
| `tests/real_llm_cross_domain_arc.rs`（改） | 身份探针末补 `assert_arc_redline_held` 弧末门 + transcript 累积 + :1259 注释修正 |

## 五、验证（spec「真红线仍拦、无假阳、假绿缝堵住、基线不回退」）

- **纯函数单测**：`report_dim_min` 取 min（`[8,3,6]→3`、空→None）；`record_arc_skip_if_judged` 语义（judges 空不写、非空写，tempdir 隔离 `REAL_LLM_LEDGER`）；`check-skip-ledger.sh --self-test` 跨分片求和（2N≠N）。
- **G1 真信号**（CI）：端点全崩 → 各 job 写各自子目录 → skip-gate `find` 求和 > 重估阈值 → 真红卡合并（不再假绿）。
- **G2 真信号**（CI）：校准弧裁判掉线 → 写 `judge_offline` 行 → skip-gate 数得到。
- **G3 真信号**：t15 补 judge2 后某裁判被骗给高分，min 仍抓低 → 兜圈退化 panic（max 会漏）。
- **G7 真信号**：NVIDIA_KEY 空 → `Require` step exit 1（不再静默假绿）。
- **G8 真信号**：agent 答「我是 AI」→ 弧末 redlineHeld 低分 min<5 panic（逐轮 autonomyRisk 漏的，弧末接住）。
- **基线不回退**：测试 only，`cargo test --lib` ≥ 350/0；本地磁盘满不实跑，靠 CI。删改不破 `check-baseline.{sh,ps1}`。
- **反过拟合守护**：阈值锚点全不动；`REAL_LLM_MAX_SKIP` 重估按 job 规模、非朝结果调；新增门用既有抽象锚点。

## 六、与交叉审查报告的关系

- 修复终审「合并前必修（Blocking）」5 项（G1/G2/G3/G7/G8），恢复铁律4 + 铁律③ + G8 接口。
- 可后续（Non-blocking）项不在本次范围：G13/G14（CI/doc 注释陈旧）、G11（realism rubric escalation_coherence 维输入不支撑）、G15-G21 Observation、全部 PARTIAL_REFUTED 收窄小修（G4 极性对齐/G5 t17 传 transcript/G6 注释对齐/G9 补 J1 A/B/G10 文档化/G12 注释/G18 无需动）。
- 完成后这套评判设施才真正兑现「agent 做错 → 测试变红」：端点崩溃时不再静默全绿（铁律4），方向缺陷不再漏判（铁律③），身份探针不再漏判唯一目标（G8）。
