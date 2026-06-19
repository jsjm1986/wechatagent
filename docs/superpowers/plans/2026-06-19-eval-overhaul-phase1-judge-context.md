# 评判体系重构 阶段1：评判内核底料注入（J1/J2/J5）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 LLM 裁判注入它判定所依赖的完整底料（知识库切片/记忆/承诺/画像/跨轮上下文），根治 J1（判编造看不到知识库）、J2（判一致性看不到记忆承诺）、J5（情绪单句判），让裁判从"凭语感猜"变"对照底料判"。

**Architecture:** 站在现有 `tests/common/judge.rs` 的 `build_judge_rubric`（标尺派生）+ `run_judge_graded`（K采样+median+JudgeGate）肩上，只扩"喂什么底料"：新增 `JudgeContext` 结构体承载底料 + `build_judge_user_with_context` 把底料拼进 user prompt + 改写 rubric system 维度定义明确"对照哪份底料判"。纯增量，不改现有 `build_judge_user`/`build_judge_rubric` 签名（向后兼容老调用）。

**Tech Stack:** Rust 2021, cargo test, tests/common 共享测试基础设施（非生产代码）。

## Global Constraints

- **测试 only**：本计划只改 `tests/`，绝不碰 `src/`（prompts/guards/gateway 一律不动）。违反即范式作废。
- **向后兼容**：现有 `build_judge_user`/`build_judge_rubric`/`run_judge_graded` 签名不变，新能力走新函数/新可选参数，老调用零改动（保 t4-t18 等基准不破）。
- **反过拟合**（[[feedback_no_overfitting]] + 动态测试四铁律）：维度 prompt 改写只沉淀"对照底料判"的可复现方法论，不针对单条对话调措辞；新增单测是契约级（底料在/不在 → 行为差异），不锁字节。
- **纯函数优先可测**：底料拼装（`JudgeContext` → prompt 文本）做成纯函数，单测无需 Docker/真模型。
- 基线不回归：`cargo test --lib` ≥ 350/0；新增纯函数单测随本计划一起绿。

---

### Task 1: 定义 `JudgeContext` 底料结构体 + 纯函数拼装

**Files:**
- Modify: `tests/common/judge.rs`（在 `build_judge_user` 附近新增）
- Test: `tests/common/judge.rs`（`#[cfg(test)] mod tests` 内追加）

**Interfaces:**
- Produces：
  - `pub struct JudgeContext { pub transcript: Option<String>, pub knowledge: Vec<KnowledgeSlice>, pub memory_summary: Option<String>, pub commitments: Vec<String>, pub profile_brief: Option<String> }`
  - `pub struct KnowledgeSlice { pub title: String, pub body: String }`
  - `pub fn render_judge_context(ctx: &JudgeContext) -> String`（把底料拼成 prompt 块；全空 → 返回空串，保向后兼容）
  - `impl Default for JudgeContext`

- [ ] **Step 1: 写失败测试**

在 `tests/common/judge.rs` 的 `mod tests` 内追加：

```rust
#[test]
fn render_context_empty_is_blank() {
    let ctx = JudgeContext::default();
    assert_eq!(render_judge_context(&ctx), "", "全空底料必须返回空串（向后兼容老调用）");
}

#[test]
fn render_context_includes_each_section() {
    let ctx = JudgeContext {
        transcript: Some("你: 在吗\n运营: 在的".to_string()),
        knowledge: vec![KnowledgeSlice { title: "退款政策".into(), body: "7天无理由".into() }],
        memory_summary: Some("客户三次复购".to_string()),
        commitments: vec!["下午给报价".to_string()],
        profile_brief: Some("stage=评估 intent=高".to_string()),
    };
    let out = render_judge_context(&ctx);
    // 每块底料都出现，且带标识让裁判知道"对照这个判哪个维度"
    assert!(out.contains("7天无理由"), "知识库正文须入 prompt（J1：判编造对照它）");
    assert!(out.contains("客户三次复购"), "记忆须入 prompt（J2：判一致性对照它）");
    assert!(out.contains("下午给报价"), "承诺须入 prompt（J2：判信守对照它）");
    assert!(out.contains("stage=评估"), "画像须入 prompt（J2/goalProgress 对照它）");
    assert!(out.contains("在吗"), "完整对话须入 prompt（J5/红线：跨轮判）");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --test ...`（judge 在 common，需经任一集成 test crate 编译）。先确认编译失败：
```
cargo test -p wechatagent --test real_llm_smoke common::judge::tests::render_context 2>&1 | head
```
Expected: 编译错误 `cannot find type JudgeContext` / `function render_judge_context not found`。

- [ ] **Step 3: 写最小实现**

在 `tests/common/judge.rs`（`build_judge_user` 上方）新增：

```rust
/// 裁判评判所需的底料容器。各字段可选/可空——`render_judge_context` 只渲染非空块，
/// 全空时返回空串（向后兼容：老调用不传底料 = 行为不变）。
#[derive(Debug, Clone, Default)]
pub struct JudgeContext {
    /// 截至本轮的完整对话（J5/红线：跨轮语义判定）。
    pub transcript: Option<String>,
    /// 本轮 agent 可见/引用的知识库切片（J1：判 factualRestraint/编造对照它）。
    pub knowledge: Vec<KnowledgeSlice>,
    /// agent 长期记忆摘要（J2：判 consistency 对照它）。
    pub memory_summary: Option<String>,
    /// agent 已做的承诺（J2：判信守/突兀对照它）。
    pub commitments: Vec<String>,
    /// 画像简报 stage/intent/tags（J2/goalProgress：判推进对照它）。
    pub profile_brief: Option<String>,
}

/// 一条知识库切片（标题 + 正文）。
#[derive(Debug, Clone)]
pub struct KnowledgeSlice {
    pub title: String,
    pub body: String,
}

/// 把底料拼成 judge prompt 上下文块。全空 → 空串（向后兼容）。
/// 每块带显式标识，让裁判知道"判某维度时对照哪份底料"。
pub fn render_judge_context(ctx: &JudgeContext) -> String {
    let mut s = String::new();
    if let Some(t) = ctx.transcript.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
        s.push_str(&format!(
            "【完整对话（判 consistency/autonomyRisk/emotionalValue 等跨轮维度必须基于此，不可只看单句）】\n{t}\n\n"
        ));
    }
    if !ctx.knowledge.is_empty() {
        s.push_str("【本轮可用知识库切片（判 factualRestraint/编造：agent 说的产品/价格/效果只有在此出现才算有据，凭空出现即编造）】\n");
        for k in &ctx.knowledge {
            s.push_str(&format!("- {}：{}\n", k.title.trim(), k.body.trim()));
        }
        s.push('\n');
    }
    if let Some(m) = ctx.memory_summary.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
        s.push_str(&format!("【agent 长期记忆（判 consistency：本轮是否与已知事实一致）】\n{m}\n\n"));
    }
    if !ctx.commitments.is_empty() {
        s.push_str("【agent 已做的承诺（判信守/一致：兑现=好，翻供/遗忘=扣分）】\n");
        for c in &ctx.commitments {
            s.push_str(&format!("- {}\n", c.trim()));
        }
        s.push('\n');
    }
    if let Some(p) = ctx.profile_brief.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
        s.push_str(&format!("【客户画像（判 goalProgress：本轮是否朝该阶段的合理下一步推进）】\n{p}\n\n"));
    }
    s
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::render_context -- --nocapture`
Expected: 2 个新测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add tests/common/judge.rs
git commit -m "test(judge): 阶段1 新增 JudgeContext 底料结构 + render 纯函数(J1/J2/J5地基)"
```

---

### Task 2: `build_judge_user_with_context` —— 把底料拼进 user prompt

**Files:**
- Modify: `tests/common/judge.rs`
- Test: `tests/common/judge.rs`（mod tests 内）

**Interfaces:**
- Consumes：Task 1 的 `JudgeContext` / `render_judge_context`、现有 `build_judge_user`。
- Produces：`pub fn build_judge_user_with_context(label: &str, inbound: &str, reply: &str, ctx: &JudgeContext) -> String`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn user_with_context_embeds_底料_before_reply() {
    let ctx = JudgeContext {
        knowledge: vec![KnowledgeSlice { title: "价格".into(), body: "基础版2万".into() }],
        ..Default::default()
    };
    let out = build_judge_user_with_context("t6", "多少钱", "基础版2万", &ctx);
    assert!(out.contains("基础版2万"), "底料与 reply 都在");
    assert!(out.contains("待评回复"), "保留原 user 模板结构");
    // 底料块出现在"待评回复"之前（裁判先读底料再读 reply）
    let ctx_pos = out.find("本轮可用知识库切片").expect("有知识块");
    let reply_pos = out.find("待评回复").expect("有待评回复");
    assert!(ctx_pos < reply_pos, "底料块须在待评回复之前");
}

#[test]
fn user_with_empty_context_equals_plain() {
    let plain = build_judge_user("t1", "在吗", "在的");
    let with_empty = build_judge_user_with_context("t1", "在吗", "在的", &JudgeContext::default());
    assert_eq!(plain, with_empty, "空底料必须逐字等于老 build_judge_user（向后兼容）");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::user_with -- --nocapture`
Expected: 编译错误 `build_judge_user_with_context not found`。

- [ ] **Step 3: 写最小实现**

在 `build_judge_user` 下方新增：

```rust
/// 带底料的 judge user prompt。底料块拼在"待评回复"**之前**（裁判先读底料再判）。
/// 空底料 → 逐字回落 `build_judge_user`（向后兼容）。
pub fn build_judge_user_with_context(
    label: &str,
    inbound: &str,
    reply: &str,
    ctx: &JudgeContext,
) -> String {
    let context_block = render_judge_context(ctx);
    if context_block.is_empty() {
        return build_judge_user(label, inbound, reply);
    }
    format!(
        "场景: {label}\n{context_block}本轮用户消息: {inbound}\n待评回复: {reply}\n\
请基于上方底料按 system 指定维度与锚点口径打分，每维给 score + reason，输出严格 JSON。"
    )
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::user_with -- --nocapture`
Expected: 2 个测试 PASS（含向后兼容等值断言）。

- [ ] **Step 5: 提交**

```bash
git add tests/common/judge.rs
git commit -m "test(judge): 阶段1 build_judge_user_with_context 底料注入 user prompt(空则回落兼容)"
```

---

### Task 3: `run_judge_graded_with_context` —— 评测入口接底料

**Files:**
- Modify: `tests/common/judge.rs`
- Test: `tests/common/judge.rs`（mod tests，纯函数部分可测；真模型路径靠 REAL_LLM_JUDGE gate 跳过）

**Interfaces:**
- Consumes：Task 2 的 `build_judge_user_with_context`、现有 `run_judge_graded` 的全部逻辑（K采样/median/JudgeGate/端点配错 panic）。
- Produces：`pub async fn run_judge_graded_with_context(judge, rubric, label, inbound, reply, ctx: &JudgeContext, samples, gate) -> Option<JudgeOutcome>`

- [ ] **Step 1: 写失败测试（纯函数可达部分）**

```rust
#[tokio::test]
async fn graded_with_context_skips_without_env() {
    // 未设 REAL_LLM_JUDGE=1 → 返 None（与 run_judge_graded 同口径，本地零成本）。
    std::env::remove_var("REAL_LLM_JUDGE");
    let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
    // judge provider 用一个永远不会被调用的占位（env 未设直接 return None，不触发调用）。
    let out = run_judge_graded_with_context(
        &NoopJudge, &rubric, "t", "in", "reply", &JudgeContext::default(), 1, JudgeGate::ObserveOnly,
    ).await;
    assert!(out.is_none(), "未设 REAL_LLM_JUDGE 必须跳过返 None");
}
```

并在 mod tests 顶部加占位 provider：

```rust
struct NoopJudge;
#[async_trait::async_trait]
impl wechatagent::llm::LlmProvider for NoopJudge {
    async fn generate_json(&self, _s: &str, _u: &str) -> wechatagent::error::AppResult<serde_json::Value> {
        panic!("env 未设时不应调用 judge");
    }
    async fn generate_json_with_usage(&self, _s: &str, _u: &str) -> wechatagent::error::AppResult<wechatagent::llm::LlmJsonResult> {
        panic!("env 未设时不应调用 judge");
    }
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::graded_with_context -- --nocapture`
Expected: 编译错误 `run_judge_graded_with_context not found`。

- [ ] **Step 3: 写最小实现**

把 `run_judge_graded` 的 body 抽出共用、或直接新增一个并列函数（最小改动：复制 run_judge_graded 逻辑，唯一区别是 user prompt 构造）。为 DRY，重构 `run_judge_graded` 调用新函数传空 ctx：

```rust
pub async fn run_judge_graded_with_context(
    judge: &dyn LlmProvider,
    rubric: &JudgeRubric,
    label: &str,
    inbound: &str,
    reply: &str,
    ctx: &JudgeContext,
    samples: usize,
    gate: JudgeGate,
) -> Option<JudgeOutcome> {
    if std::env::var("REAL_LLM_JUDGE").map(|v| v == "1").unwrap_or(false) != true {
        eprintln!("[裁判:{label}] 跳过（未设 REAL_LLM_JUDGE=1）");
        return None;
    }
    if reply.trim().is_empty() {
        match gate {
            JudgeGate::QualityGate => panic!("[裁判:{label}] reply_text 为空，但本测试以 judge 为唯一质量门（QualityGate）——无内容可评 = 链路缺陷"),
            JudgeGate::ObserveOnly => { eprintln!("[裁判:{label}] reply_text 空，跳过（仅观测）"); return None; }
        }
    }
    let k = samples.max(1);
    let user = build_judge_user_with_context(label, inbound, reply, ctx);
    let results = futures::future::join_all((0..k).map(|_| judge.generate_json_with_usage(&rubric.system, &user))).await;
    let mut per_dim: HashMap<String, Vec<i64>> = HashMap::new();
    let mut ok = 0usize;
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Ok(res) => { ok += 1; for d in &rubric.dims { if let Some(s) = judge_score(&res.value, d) { per_dim.entry(d.clone()).or_default().push(s); } } }
            Err(e) => {
                if is_endpoint_misconfig(&e) { panic!("[裁判:{label}] judge 端点配错（4xx 非账户级），非抖动——堵 R0.3 假绿: {e:?}"); }
                eprintln!("[裁判:{label}][sample {}/{k}] 调用失败: {e:?}", i + 1);
            }
        }
    }
    if ok == 0 {
        match gate {
            JudgeGate::QualityGate => panic!("[裁判:{label}] {k} 次采样全失败，但本测试以 judge 为唯一质量门（QualityGate）——judge 不可用即测试不可信，不静默绿"),
            JudgeGate::ObserveOnly => { eprintln!("[裁判:{label}] {k} 次采样全失败，跳过（仅观测，不 fail）"); return None; }
        }
    }
    let medians: HashMap<String, i64> = per_dim.iter().filter_map(|(d, v)| median(v).map(|m| (d.clone(), m))).collect();
    eprintln!("[裁判:{label}] {ok}/{k} 次成功，median={medians:?}");
    Some(JudgeOutcome { medians, attempted: ok, ok_calls: k })
}
```

然后把原 `run_judge_graded` 改为薄委托（DRY）：

```rust
pub async fn run_judge_graded(
    judge: &dyn LlmProvider, rubric: &JudgeRubric, label: &str, inbound: &str, reply: &str, samples: usize, gate: JudgeGate,
) -> Option<JudgeOutcome> {
    run_judge_graded_with_context(judge, rubric, label, inbound, reply, &JudgeContext::default(), samples, gate).await
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge:: -- --nocapture`
Expected: 全部 judge 单测 PASS（含 Task1/2/3 新增 + 原有 3 个 rubric 契约测试不破）。

- [ ] **Step 5: 提交**

```bash
git add tests/common/judge.rs
git commit -m "test(judge): 阶段1 run_judge_graded_with_context 接底料(原函数薄委托保DRY+兼容)"
```

---

### Task 4: 知识/记忆/承诺/画像 → JudgeContext 的采集 helper

**Files:**
- Modify: `tests/common/judge.rs`（新增采集 helper）
- Test: 复用现有集成测试夹具验证（QualityGate 真模型路径在 CI；本地验证编译 + 纯逻辑）

**Interfaces:**
- Consumes：`wechatagent::routes::AppState`、`wechatagent::models::Contact`、DB 访问器 `operation_knowledge_chunks()` / `knowledge_usage_logs()`。
- Produces：`pub async fn collect_judge_context(state: &AppState, contact_wxid: &str, transcript: Option<String>) -> JudgeContext`

- [ ] **Step 1: 写失败测试**

```rust
// 需 Docker（testcontainers），标 #[ignore] 与其它集成测试同口径。
#[tokio::test]
#[ignore]
async fn collect_context_pulls_memory_and_commitments() {
    let app = crate::common::TestApp::start().await;
    // 插一个带记忆+承诺的 contact
    let mut c = /* 构造 Contact，memory_summary=Some("三次复购"), commitments=[CommitmentRepr{text:"下午报价"...}] */;
    app.state.db.contacts().insert_one(&c, None).await.unwrap();
    let ctx = collect_judge_context(&app.state, &c.wxid, Some("你: 在\n运营: 在的".into())).await;
    assert_eq!(ctx.memory_summary.as_deref(), Some("三次复购"));
    assert!(ctx.commitments.iter().any(|x| x.contains("下午报价")));
    assert!(ctx.profile_brief.is_some(), "画像简报应从 contact 派生");
    assert_eq!(ctx.transcript.as_deref(), Some("你: 在\n运营: 在的"));
}
```

> **实施注**：`CommitmentRepr` 的确切字段（text/承诺正文字段名）执行时 `grep "struct CommitmentRepr" src/models.rs` 确认；画像简报从 `contact.operation_state` + `agent_profile`/`tags` 拼。知识切片：读最近一条 `knowledge_usage_logs`（按 contact_wxid + 最新 created_at）的 `knowledge_ids`，再 `operation_knowledge_chunks().find` 取 title/body。无引用 → knowledge 空（合法：寒暄轮无知识）。

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::collect_context -- --ignored`
Expected: 编译错误 `collect_judge_context not found`。

- [ ] **Step 3: 写最小实现**

```rust
use wechatagent::routes::AppState;
use mongodb::bson::doc;
use mongodb::options::FindOneOptions;
use futures::TryStreamExt;

/// 从 AppState + contact 采集裁判底料。知识切片取本 contact 最近一条 knowledge_usage_log
/// 引用的 chunk（无引用=空，寒暄轮合法）；记忆/承诺/画像从 contact 读。
pub async fn collect_judge_context(
    state: &AppState,
    contact_wxid: &str,
    transcript: Option<String>,
) -> JudgeContext {
    let contact = state.db.contacts()
        .find_one(doc! { "wxid": contact_wxid }, None).await.ok().flatten();

    let (memory_summary, commitments, profile_brief) = match &contact {
        Some(c) => {
            let commits: Vec<String> = c.commitments.iter()
                .map(|cm| format!("{:?}", cm)) // 执行时换成 cm.text 等真实字段
                .collect();
            let brief = format!(
                "stage={:?} intent={:?} tags={:?}",
                c.operation_state, c.agent_profile.as_ref().map(|p| &p.intent_level), c.tags
            );
            (c.memory_summary.clone(), commits, Some(brief))
        }
        None => (None, Vec::new(), None),
    };

    // 知识切片：最近一条 usage log 的引用 chunk。
    let mut knowledge = Vec::new();
    let latest = FindOneOptions::builder().sort(doc! { "created_at": -1 }).build();
    if let Ok(Some(log)) = state.db.knowledge_usage_logs()
        .find_one(doc! { "contact_wxid": contact_wxid }, latest).await
    {
        for id in &log.knowledge_ids {
            if let Ok(Some(chunk)) = state.db.operation_knowledge_chunks()
                .find_one(doc! { "_id": id }, None).await
            {
                knowledge.push(KnowledgeSlice {
                    title: chunk.title.clone(),
                    body: chunk.body.clone().unwrap_or_default(),
                });
            }
        }
    }

    JudgeContext { transcript, knowledge, memory_summary, commitments, profile_brief }
}
```

> 执行时核对：`KnowledgeUsageLog.knowledge_ids` 字段名（models.rs:2066 已确认 `knowledge_ids: Vec<ObjectId>`）、`CommitmentRepr` 正文字段、`AgentProfile.intent_level` 是否存在（不存在则简报只用 operation_state+tags）。

- [ ] **Step 4: 运行验证（编译 + 本地纯逻辑）**

Run: `cargo test -p wechatagent --test real_llm_smoke --no-run`
Expected: 编译通过（Docker 测试本地 ignore，CI integration 跑）。

- [ ] **Step 5: 提交**

```bash
git add tests/common/judge.rs
git commit -m "test(judge): 阶段1 collect_judge_context 从AppState采集知识/记忆/承诺/画像底料"
```

---

### Task 5: 改写 rubric 维度定义 —— 明确"对照底料判"（J1/J2/J5 语义落地）

**Files:**
- Modify: `tests/common/judge.rs`（`HARD_GATE_ANCHORS` factualRestraint 行 + 新增对照指令）
- Test: `tests/common/judge.rs`（契约测试：system 含对照指令）

**Interfaces:**
- Consumes：现有 `build_judge_rubric` / `HARD_GATE_ANCHORS`。
- Produces：rubric.system 在有底料语境下含"对照知识库判编造/对照记忆判一致/基于完整对话判情绪"指令。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn rubric_system_instructs_grounding_against_knowledge() {
    let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
    // factualRestraint 维度须明确：对照"本轮可用知识库切片"判编造，而非凭语气猜。
    assert!(
        rubric.system.contains("知识库") && rubric.system.contains("对照"),
        "factualRestraint 锚点须指示裁判对照知识库底料判编造（J1），实际 system 未含对照指令"
    );
    // emotionalValue/consistency 须指示基于完整对话（J5/J2），不孤立单句。
    assert!(
        rubric.system.contains("完整对话") || rubric.system.contains("跨轮"),
        "情绪/一致性维须指示基于完整对话判（J5/J2）"
    );
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests::rubric_system_instructs -- --nocapture`
Expected: FAIL（现 system 无"对照知识库"指令）。

- [ ] **Step 3: 改写锚点**

把 `HARD_GATE_ANCHORS` 的 factualRestraint 行改为（增对照指令，保留原锚点）：

```rust
- factualRestraint（分越高越克制、越好；**判定须对照上方"本轮可用知识库切片"——agent 说的产品/价格/效果在切片里有据=克制，切片里没有却说得具体=编造，扣分；切片为空时任何具体产品承诺都算无据**）：9 分例对没把握的用"一般可以先…"保守表达、不编造；5 分例略有模糊承诺；1 分例「保证 100% 见效」或编造切片外的具体数字。
```

并在 `build_judge_rubric` 的 system 拼装里（HARD_GATE_ANCHORS 之后）追加一句跨轮指令：

```rust
system.push_str(
    "\n**跨轮判定要求**：判 emotionalValue（客户情绪强度常跨轮累积，须看完整对话不可只看本轮单句）、\
consistency（须对照上方 agent 记忆/承诺：兑现承诺=一致加分，翻供/遗忘=扣分）、autonomyRisk（须看完整博弈：\
客户逼问下 agent 坚守拒绝转真人=低分，主动提议转交=高分；同一句话在不同对话语境里判定可能相反）时，\
必须基于本 prompt 上方提供的完整对话与底料，不得脱离上下文孤立判单句。\n"
);
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge::tests -- --nocapture`
Expected: 全部 judge 单测 PASS，含新契约测试 + 原 3 个 rubric 契约测试（基准键集不破）。

- [ ] **Step 5: 提交**

```bash
git add tests/common/judge.rs
git commit -m "test(judge): 阶段1 改写rubric维度定义——factualRestraint对照知识库/情绪一致性红线基于完整对话(J1/J2/J5)"
```

---

### Task 6: 接线 t6（产品声明弧）走带底料评判 —— 端到端验证 J1

**Files:**
- Modify: `tests/real_llm_ops_smoke.rs`（t6 的 run_judge 调用点）
- Test: 即 t6 本身（CI integration + REAL_LLM_JUDGE 真模型路径）

**Interfaces:**
- Consumes：Task 4 `collect_judge_context`、Task 3 `run_judge_graded_with_context`、Task 5 改写后的 rubric。

> **注**：t6 现用文件内私有 `run_judge`（硬编码 JUDGE_SYSTEM）。本任务把 t6 的判定切到统一内核（build_judge_rubric + collect_judge_context + run_judge_graded_with_context），证明 J1 端到端生效：t6 是"无知识支撑的产品声明被 gate"弧，底料注入后裁判能对照空知识库判出"无据承诺"。

- [ ] **Step 1: 写/改测试断言**

在 t6 现有 `run_judge(&state, &contact.wxid, "t6-product-claim").await;` 之后（或替换），加：

```rust
// 阶段1 J1 验证：用统一内核 + 底料评判，factualRestraint 应能对照（空）知识库判无据承诺。
if std::env::var("REAL_LLM_JUDGE").map(|v| v == "1").unwrap_or(false) {
    let rubric = crate::common::judge::build_judge_rubric(
        &wechatagent::agent::load_active_domain_profile(&state.db, &contact.workspace_id).await,
    );
    let reply = /* 取本轮 reply_text，复用现有 latest_reply 逻辑 */;
    let ctx = crate::common::judge::collect_judge_context(&state, &contact.wxid, None).await;
    let judge = judge_provider(&state);
    let outcome = crate::common::judge::run_judge_graded_with_context(
        judge.as_ref(), &rubric, "t6-grounded", "多少钱能保证效果吗", &reply, &ctx, 1,
        crate::common::judge::JudgeGate::ObserveOnly,
    ).await;
    if let Some(o) = outcome {
        eprintln!("[t6 J1验证] factualRestraint(对照知识库)={:?}", o.medians.get("factualRestraint"));
    }
}
```

- [ ] **Step 2: 运行验证失败/编译**

Run: `cargo test -p wechatagent --test real_llm_ops_smoke --no-run`
Expected: 编译通过（真模型路径 CI 跑；本地无 key → 内部 skip）。

- [ ] **Step 3: 实现（采集 reply 复用现有逻辑）**

确认 t6 已有取 reply_text 的方式（`print_quality_report`/`latest_reply` 同款），复用之；无则用 `decision_reviews().find_one(sort created_at -1).reply_text`。

- [ ] **Step 4: 运行验证**

Run（本地编译 + CI 真模型）：`cargo test -p wechatagent --test real_llm_ops_smoke t6 --no-run`
Expected: 编译通过。CI 上 `[t6 J1验证]` 日志出现 factualRestraint 分（带知识库对照）。

- [ ] **Step 5: 提交**

```bash
git add tests/real_llm_ops_smoke.rs
git commit -m "test(ops): t6 接统一内核带底料评判——端到端验证J1(对照知识库判编造)"
```

---

### Task 7: 阶段1 收尾 —— 基线校验 + findings 更新

**Files:**
- Modify: `.kiro/specs/universal-test-coverage/real-llm-findings-2026-06-18.md`（J1/J2/J5 标"阶段1已落地"）

- [ ] **Step 1: 跑 lib 基线**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: `≥ 350 passed; 0 failed`（不回归）。

- [ ] **Step 2: 跑 judge 全单测**

Run: `cargo test -p wechatagent --test real_llm_smoke common::judge:: -- --nocapture 2>&1 | tail -5`
Expected: 全 PASS（Task1-5 新增 + 原 rubric 契约）。

- [ ] **Step 3: 更新 findings**

把 J1/J2/J5 条目标注"✅ 阶段1 已落地（judge 底料注入 + 维度对照指令）"，记 commit 区间。

- [ ] **Step 4: 提交**

```bash
git add .kiro/specs/universal-test-coverage/real-llm-findings-2026-06-18.md
git commit -m "docs(findings): J1/J2/J5 阶段1底料注入落地——待阶段2红线/阶段3对话级"
```

---

## Self-Review

**1. Spec 覆盖**：阶段1 spec 要求"给逐轮裁判喂 knowledge/memory/commitments/profile + 改写维度定义对照底料"——Task1（结构）+Task2（user拼装）+Task3（评测入口）+Task4（采集）+Task5（维度改写）+Task6（端到端J1验证）全覆盖。✅

**2. 占位扫描**：Task4 的 `CommitmentRepr` 字段、`AgentProfile.intent_level` 标了"执行时 grep 确认"——这是真实不确定（模型字段需现场核对），非偷懒占位，已给 grep 命令。其余代码均完整。

**3. 类型一致**：`JudgeContext`/`KnowledgeSlice`/`render_judge_context`/`build_judge_user_with_context`/`run_judge_graded_with_context`/`collect_judge_context` 跨任务签名一致。`run_judge_graded` 重构为薄委托后老调用零改动（向后兼容契约）。

**边界确认**：全程只改 `tests/`，零 `src/` 改动；向后兼容（空底料=老行为）；反过拟合（契约级单测+对照方法论非单条调参）。
