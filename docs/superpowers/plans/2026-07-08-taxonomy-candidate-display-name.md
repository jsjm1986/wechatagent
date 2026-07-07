# 决策路径标签候选补 AI 中文建议名 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 gateway/decision 决策路径写入的 `taxonomy_candidates` 带上 LLM 生成的中文建议名（`suggested_display_name`），使收件箱命名卡「显示名」预填中文而非英文裸值（`anxious`）。

**Architecture:** 决策 LLM 在 `dimensionDisplayNames` 映射里为「自造新值」顺带产中文名 → `RawAgentDecision` 反序列化 + carry-through 到 `AgentDecision.dimension_display_names` → 两条同源候选写入路径（gateway 主循环 + decision_taxonomy fire-and-forget）各按 `kind` 取名作 `upsert_candidate` 第 7 参。全链 best-effort：缺失→None→回落英文，绝不阻塞 run。

**Tech Stack:** Rust 2021 (Axum + MongoDB `bson::Document`)；`serde` 反序列化；`cargo test --lib` 单测。纯后端，不碰前端（命名卡预填 `suggestedDisplayName || rawValue` 已就绪）。

## Global Constraints

以下为项目级硬约束，每个 task 隐含遵守（值逐字取自 spec 与 CLAUDE.md）：

- **红线**：任何改动落地前 100% 读懂受影响代码路径，引用 `file:line` 须亲验；不猜。
- **AI 全自治**：新增代码/文案禁含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`；`bash scripts/check-no-human-takeover.sh` 必须 0 违规。
- **候选不阻塞 run**：`taxonomy_candidates` 写入全链 best-effort，任何缺失/IO 故障回落 `None` 并继续（CLAUDE.md「unreviewed candidates must not block runs」）。
- **测试基线不回归**：`cargo test --lib` **≥ 350 passed / 0 failed**；测试只增量叠加，不删改旧用例。
- **不 bump `PROMPT_PACK_VERSION`**：`prompts.rs:15` 常量不动；prompt 文本改动由启动 `align_prompt_specs` 内容 diff（`prompts.rs:259`）生效。
- **字段 optional 是 LLM 输出容错**：`dimensionDisplayNames` 绝大多数轮次不出现（无自造值就无名字），改必填会使 LLM 漏填的轮次 decision 反序列化失败 → 决策链路崩。
- **reply（中文）**：与用户交流用中文；代码/标识符/commit 遵循既有约定。
- **提交纪律**：未经用户明确许可不 push / 开 PR / 合并。

---

### Task 1: `AgentDecision` 承载 `dimension_display_names`（字段 + 手写 Default + Raw 镜像 + carry-through）

`dimensionDisplayNames` 是 LLM 输出的新字段。`AgentDecision` 有**手写** `impl Default`（`types.rs:305-380`，非 derive），且主决策路径经 `RawAgentDecision` → `validate_and_promote` → `carry_through_fields` 落地——四处都要动，漏一处则字段被静默丢弃。

**Files:**
- Modify: `src/agent/types.rs`
  - `AgentDecision` 结构体加字段（在 `:124` `domain_signals` 之后）
  - `impl Default for AgentDecision`（`:322` `domain_signals: Document::new(),` 之后加一行）
  - `RawAgentDecision` 结构体加字段（`:460` `domain_signals` 之后）
  - `carry_through_fields`（`:1001-1007` `domain_signals` 透传块之后加一行）
- Test: `src/agent/types.rs`（`#[cfg(test)] mod carry_through_*` append 新用例）

**Interfaces:**
- Produces:
  - `AgentDecision.dimension_display_names: Document`（serde `dimensionDisplayNames`，`#[serde(default)]`）——Task 2/3 从这里按 kind 取中文名。
  - `RawAgentDecision.dimension_display_names: Option<Document>`（serde `dimensionDisplayNames`，`#[serde(default)]`）。

- [ ] **Step 1: 写失败测试（carry-through 保留 + 空缺省）**

在 `types.rs` 末尾的 `#[cfg(test)] mod namecard_carry_through_tests`（`:2195` 一带，复用其 `make_valid_low_routine_raw` / `runtime_default` 导入）**之后**追加一个新测试模块（不改旧模块）：

```rust
#[cfg(test)]
mod dimension_display_names_tests {
    //! dimensionDisplayNames carry-through 回归：LLM 输出的维度中文名映射
    //! 必须经 RawAgentDecision → validate_and_promote 透传到 AgentDecision，
    //! 不能被静默丢弃（防丢字段硬伤，同 namecard/assets 老坑）。
    use super::validate_and_promote_tests::{make_valid_low_routine_raw, runtime_default};
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn decision_without_display_names_defaults_empty() {
        // 旧/常规 LLM 输出（无 dimensionDisplayNames）仍能反序列化，字段默认空 doc。
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.dimension_display_names.is_empty());
    }

    #[test]
    fn raw_decision_carries_display_names_through_promote() {
        let mut raw = make_valid_low_routine_raw();
        raw.dimension_display_names = Some(doc! { "customer_stage": "焦虑观望" });
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert_eq!(
            decision.dimension_display_names.get_str("customer_stage").ok(),
            Some("焦虑观望"),
            "dimensionDisplayNames 必须 carry-through，实际 {:?}",
            decision.dimension_display_names
        );
    }

    #[test]
    fn raw_decision_none_display_names_stays_empty_after_promote() {
        // Raw 未给该字段 → promote 后保持空 doc（不 panic、不误填）。
        let raw = make_valid_low_routine_raw();
        let runtime = runtime_default(true);
        let (decision, _risks) = raw.validate_and_promote(&runtime);
        assert!(decision.dimension_display_names.is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `touch src/lib.rs && cargo test --lib dimension_display_names_tests 2>&1 | tail -20`
Expected: 编译失败——`AgentDecision` / `RawAgentDecision` 无 `dimension_display_names` 字段（`error[E0560]` 或 `no field`）。

- [ ] **Step 3: `AgentDecision` 加字段**

在 `types.rs:124`（`pub domain_signals: Document,` 那一行）**之后**插入：

```rust
    /// 维度值 → 中文显示名。LLM 仅在为某维度填了「字典外自造新值」时，在此为该
    /// 维度配一个简洁中文名（如 `{"customer_stage": "焦虑观望"}`）。字典已有的标准
    /// 值不必填（已有 canonical label）。gateway / decision_taxonomy 产 taxonomy
    /// 候选时按 kind 查此表取中文名作 `suggested_display_name`（收件箱命名卡预填）。
    /// 绝大多数轮次不出现（无自造值即无名字）——故 `#[serde(default)]` 是输出容错，
    /// 非兼容 shim；改必填会使 LLM 漏填的轮次 decision 反序列化失败、决策链路崩。
    #[serde(default)]
    pub dimension_display_names: Document,
```

- [ ] **Step 4: 手写 `Default` 加字段**

在 `types.rs:322`（`domain_signals: Document::new(),`）**之后**插入：

```rust
            dimension_display_names: Document::new(),
```

- [ ] **Step 5: `RawAgentDecision` 加字段**

在 `types.rs:460`（`pub domain_signals: Option<Document>,`）**之后**插入：

```rust
    /// 维度值→中文名映射（promote 后经 carry_through 透传到
    /// `AgentDecision.dimension_display_names`）。LLM 缺省 → None → 容器空。
    #[serde(default)]
    pub dimension_display_names: Option<Document>,
```

- [ ] **Step 6: `carry_through_fields` 加透传**

在 `types.rs:1007`（`domain_signals` 透传块的结尾 `}`）**之后**插入。注意镜像 `domain_signals` 的「非空才覆盖」写法（空 doc 不覆盖默认空 doc，行为等价、省一次赋值）：

```rust
    if let Some(v) = raw.dimension_display_names {
        // 维度中文名 carry-through（同 namecard/assets 老坑：不透传则 promote 后
        // 永远空、LLM 产的中文名被静默丢弃，收件箱又回落英文）。仅非空覆盖。
        if !v.is_empty() {
            decision.dimension_display_names = v;
        }
    }
```

- [ ] **Step 7: 跑测试确认通过 + 基线不回归**

Run: `cargo test --lib dimension_display_names_tests 2>&1 | tail -20`
Expected: 3 个新测试全 PASS。

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. NNN passed; 0 failed`，NNN ≥ 350。

- [ ] **Step 8: 提交**

```bash
git add src/agent/types.rs
git commit -m "feat(types): AgentDecision 承载 dimensionDisplayNames + carry-through

LLM 为自造新值产的维度中文名映射,经 RawAgentDecision → promote →
carry_through_fields 透传到 AgentDecision.dimension_display_names,
供 gateway/decision_taxonomy 取作候选 suggested_display_name。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: gateway 主循环取名传参（纯函数 `pick_dimension_display_name` + 单测）

gateway 的 `candidate_writes` 循环（`gateway.rs:1702-1716`）现在把第 7 参写死 `None`。抽一个纯函数按 kind 从 `final_decision.dimension_display_names` 取中文名，循环调它传参。纯函数便于单测（不必起 gateway/db）。

**Files:**
- Modify: `src/agent/gateway.rs`
  - 新增纯函数 `pick_dimension_display_name`（放在 `compute_taxonomy_guard_outcome` 附近，`:5167` 之前）
  - 改 `candidate_writes` 循环第 7 参（`:1702-1716`）
- Test: `src/agent/gateway.rs`（既有 `#[cfg(test)]` 模块内 append 纯函数单测——该文件测试模块含 `taxonomy_outcome_*` 用例，见 `:5777` 一带）

**Interfaces:**
- Consumes: `AgentDecision.dimension_display_names: Document`（Task 1）；`taxonomy_upsert_candidate(db, scope, kind, raw, evidence, confidence, suggested_display_name: Option<&str>)`（`taxonomy.rs:347-355`，第 7 参已存在）。
- Produces: `fn pick_dimension_display_name(names: &Document, kind: &str) -> Option<&str>`（供 Task 3 复用）。

- [ ] **Step 1: 写失败测试（纯函数四类输入）**

在 `gateway.rs` 的测试模块里（`taxonomy_outcome_handles_both_kinds_in_single_pass` 之后，约 `:5930`；与既有 taxonomy 单测同模块）append：

```rust
    #[test]
    fn pick_display_name_hits_trims_and_misses() {
        use mongodb::bson::doc;
        let names = doc! {
            "customer_stage": "焦虑观望",
            "intent_level": "  高意向  ",
            "blank": "   ",
            "nonstr": 42_i32,
        };
        // 命中 → 取出
        assert_eq!(pick_dimension_display_name(&names, "customer_stage"), Some("焦虑观望"));
        // 命中但含首尾空格 → trim
        assert_eq!(pick_dimension_display_name(&names, "intent_level"), Some("高意向"));
        // 纯空格 → None
        assert_eq!(pick_dimension_display_name(&names, "blank"), None);
        // 非字符串值 → None（get_str 失败）
        assert_eq!(pick_dimension_display_name(&names, "nonstr"), None);
        // 缺键 → None
        assert_eq!(pick_dimension_display_name(&names, "absent"), None);
        // 空 doc → None
        assert_eq!(pick_dimension_display_name(&Document::new(), "customer_stage"), None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `touch src/lib.rs && cargo test --lib pick_display_name 2>&1 | tail -15`
Expected: 编译失败——`pick_dimension_display_name` 未定义（`cannot find function`）。

- [ ] **Step 3: 实现纯函数**

在 `gateway.rs:5167`（`pub(crate) fn compute_taxonomy_guard_outcome` 定义）**之前**插入：

```rust
/// 从维度中文名映射（`AgentDecision.dimension_display_names`）里按 `kind` 取中文名。
/// 缺键 / 非字符串 / 空串 / 纯空格 → `None`（候选回落英文裸值）。纯函数，便于单测。
pub(crate) fn pick_dimension_display_name<'a>(names: &'a Document, kind: &str) -> Option<&'a str> {
    names
        .get_str(kind)
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
```

（`Document` 已在 `gateway.rs` 顶部 use；若单测报 `Document` 未导入，测试块内 `use mongodb::bson::Document;`。实现处若缺再加顶层 use。）

- [ ] **Step 4: 改循环第 7 参**

把 `gateway.rs:1702-1716` 的循环改为（`final_decision` 在此处在作用域内，见 `:1684` 传参；`kind` 是 `&String`，`pick_*` 收 `&str` 自动 deref）：

```rust
        for (kind, raw) in &outcome.candidate_writes {
            let display_name =
                pick_dimension_display_name(&final_decision.dimension_display_names, kind);
            if let Err(error) = taxonomy_upsert_candidate(
                &state.db,
                &contact.account_id,
                kind,
                raw,
                Some("user-ops decision path"),
                50,
                display_name,
            )
            .await
            {
                tracing::warn!(?error, kind = kind.as_str(), raw = %raw, "taxonomy upsert_candidate failed");
            }
        }
```

- [ ] **Step 5: 跑测试确认通过 + 编译 + 基线**

Run: `cargo test --lib pick_display_name 2>&1 | tail -15`
Expected: PASS。

Run: `RUSTFLAGS="-D warnings" cargo check --lib 2>&1 | tail -5`
Expected: EXIT 0，无 warning（纯函数被循环 + 单测调用，无 dead-code；无未用导入）。

Run: `cargo test --lib 2>&1 | tail -5`
Expected: NNN ≥ 350 passed / 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(gateway): 决策候选写入携带 AI 中文建议名

candidate_writes 循环按 kind 从 dimensionDisplayNames 取中文名作
upsert_candidate 第7参(原写死 None),收件箱命名卡预填中文而非英文。
取名逻辑抽纯函数 pick_dimension_display_name + 单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: decision_taxonomy 同源竞争路径取名（避免 None 抢先写掉中文名）

`decision_taxonomy::validate_and_normalize_decision`（`decision.rs:1015` 每轮调用）与 Task 2 的 gateway 循环写**同一幂等键** `(scope, kind, raw_value)`，且 `upsert_candidate` 对已存在候选**不更新** `suggested_display_name`（`taxonomy.rs:371-413` 命中即 return）——先写者赢。此路径 fire-and-forget 传 `None`（`:112`）若先落库，会把 gateway 的中文名挡在门外。故必须同改。

关键：**纯函数 `classify_decision_tags` 不动**（`:46-76`，PBT 覆盖）。只在生产入口 `validate_and_normalize_decision`（`:84-95`，持 `&mut decision`）取名后升级 candidates 元组，并改 `spawn_candidate_upserts`（`:99-123`，唯一调用点在 `:93`；测试走 `classify_with_cache_for_tests` 不碰它）。复用 Task 2 的 `pick_dimension_display_name`。

**Files:**
- Modify: `src/agent/decision_taxonomy.rs`
  - `validate_and_normalize_decision`（`:84-95`）：`classify_decision_tags` 返回后按 kind 取名，把 `Vec<(String,String)>` 升级为 `Vec<(String,String,Option<String>)>`
  - `spawn_candidate_upserts`（`:99-123`）：入参与 upsert 第 7 参改带名
- Test: `src/agent/decision_taxonomy.rs`（`#[cfg(test)] mod tests` 内 append 一个「取名映射」纯逻辑单测）

**Interfaces:**
- Consumes: `super::super::agent::gateway::pick_dimension_display_name`（Task 2，`pub(crate)`）；`AgentDecision.dimension_display_names`（Task 1）。

- [ ] **Step 1: 写失败测试（生产入口取名——用可注入 cache 变体验证 candidates 带名）**

现有 `classify_with_cache_for_tests` 只透传 `classify_decision_tags`（不带名，保持不动）。新增一个「带名收集」的测试辅助 + 断言：在 `decision_taxonomy.rs` 的 `#[cfg(test)] mod tests` 内 append：

```rust
    #[test]
    fn candidates_pick_up_display_name_from_decision() {
        // decision 带 dimensionDisplayNames 时，收集到的候选应带上对应中文名；
        // 缺名的维度回落 None。这段逻辑是生产入口 validate_and_normalize_decision
        // 在 classify_decision_tags 之后做的取名映射（纯逻辑，用 pick_* + 手工映射验证）。
        use crate::agent::gateway::pick_dimension_display_name;
        use mongodb::bson::doc;
        let cache = mk_cache(vec![]); // 空字典 → 两维都 CandidateNew
        let mut d = AgentDecision::default();
        d.customer_stage = Some("焦虑观望期".to_string());
        d.intent_level = Some("试探型".to_string());
        d.dimension_display_names = doc! { "customer_stage": "焦虑观望期" }; // 只给 stage 配名
        let (_risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        // 模拟生产入口的取名映射：
        let named: Vec<(String, String, Option<String>)> = cands
            .into_iter()
            .map(|(kind, raw)| {
                let name = pick_dimension_display_name(&d.dimension_display_names, &kind)
                    .map(str::to_string);
                (kind, raw, name)
            })
            .collect();
        let stage = named.iter().find(|(k, _, _)| k == "customer_stage").expect("stage 候选");
        assert_eq!(stage.2.as_deref(), Some("焦虑观望期"));
        let intent = named.iter().find(|(k, _, _)| k == "intent_level").expect("intent 候选");
        assert_eq!(intent.2, None, "未配名的维度回落 None");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `touch src/lib.rs && cargo test --lib candidates_pick_up_display_name 2>&1 | tail -15`
Expected: 编译失败——`pick_dimension_display_name` 未 `pub(crate)` 可见 / 或（若 Task 2 已合）测试引用路径未就绪。若 Task 2 已完成则应因断言逻辑就绪而直接指向缺少生产接线——本步核心是先让测试红。

> 注：本 Task 依赖 Task 1（字段）+ Task 2（纯函数 `pub(crate)`）。按顺序执行。

- [ ] **Step 3: 改 `spawn_candidate_upserts` 带名**

把 `decision_taxonomy.rs:99-123` 整个函数替换为：

```rust
/// 把 `candidates`（含中文建议名）列表 fire-and-forget 写盘。抽到独立函数便于未来
/// 加入熔断 / 限流策略。第三元 `Option<String>` = LLM 为自造新值产的中文名，作
/// `upsert_candidate` 的 `suggested_display_name`（收件箱命名卡预填）；None 回落英文。
fn spawn_candidate_upserts(
    db: &Database,
    scope_account_id: &str,
    candidates: Vec<(String, String, Option<String>)>,
) {
    if candidates.is_empty() {
        return;
    }
    let db = db.clone();
    let scope = scope_account_id.to_string();
    tokio::spawn(async move {
        for (kind, raw, display_name) in candidates {
            if let Err(err) =
                upsert_candidate(&db, &scope, &kind, &raw, None, 0, display_name.as_deref()).await
            {
                tracing::warn!(
                    kind = %kind,
                    raw_value = %raw,
                    ?err,
                    "taxonomy candidate upsert failed (best-effort)"
                );
            }
        }
    });
}
```

- [ ] **Step 4: 改 `validate_and_normalize_decision` 取名映射**

把 `decision_taxonomy.rs:84-95` 的函数体改为（在 `classify_decision_tags` 之后、`spawn_candidate_upserts` 之前插入取名映射）：

```rust
pub(crate) fn validate_and_normalize_decision(
    db: &Database,
    decision: &mut AgentDecision,
    dimension_kinds: &[String],
    scope_account_id: &str,
) -> Vec<String> {
    let cache = global_taxonomy_cache();
    let (risks, candidates) =
        classify_decision_tags(decision, dimension_kinds, scope_account_id, &cache);
    // 与 gateway 主循环同源：按 kind 从 decision.dimension_display_names 取 LLM 产的
    // 中文名，随候选一起落库。二者写同一幂等键 (scope,kind,raw)，upsert 对已存在候选
    // 不更新 display_name（先写者赢）——此处带名，避免本 fire-and-forget 路径的 None
    // 抢先把 gateway 的中文名挡在门外。取的是同一个 decision，名字一致、幂等无害。
    let named: Vec<(String, String, Option<String>)> = candidates
        .into_iter()
        .map(|(kind, raw)| {
            let name = crate::agent::gateway::pick_dimension_display_name(
                &decision.dimension_display_names,
                &kind,
            )
            .map(str::to_string);
            (kind, raw, name)
        })
        .collect();
    spawn_candidate_upserts(db, scope_account_id, named);
    risks
}
```

> 借用说明：`classify_decision_tags` 借 `&mut decision` 在该语句结束后即释放；随后 `pick_dimension_display_name(&decision.dimension_display_names, ...)` 取共享借用，二者不重叠，借用检查通过。

- [ ] **Step 5: 跑测试 + 编译 + 基线**

Run: `cargo test --lib candidates_pick_up_display_name 2>&1 | tail -15`
Expected: PASS。

Run: `RUSTFLAGS="-D warnings" cargo check --lib 2>&1 | tail -5`
Expected: EXIT 0，无 warning。

Run: `cargo test --lib 2>&1 | tail -5`
Expected: NNN ≥ 350 passed / 0 failed（含 decision_taxonomy 既有 PBT 全绿——纯函数未动）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/decision_taxonomy.rs
git commit -m "feat(decision_taxonomy): 同源候选写入路径携带 AI 中文建议名

validate_and_normalize_decision 与 gateway 主循环写同一候选幂等键且
upsert 先写者赢;本 fire-and-forget 路径原传 None 会抢先挡掉 gateway
的中文名。改为同样按 kind 取 dimensionDisplayNames 落 suggested_display_name。
纯函数 classify_decision_tags 不动(PBT 覆盖)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: reply 决策 prompt 加 `dimensionDisplayNames` 指令（不 bump 版本）

让 LLM 在为维度填「字典外自造新值」时，在 `dimensionDisplayNames` 里配简洁中文名。指令加进 `user.reply.task` prompt 的 final 形态 schema 块（紧邻 `customerStage`/`intentLevel` 定义处，`prompts.rs:1262-1263`）。**不 bump `PROMPT_PACK_VERSION`**（Global Constraints + spec §3.3：启动 align 按内容 diff 生效）。

**Files:**
- Modify: `src/prompts.rs`（`user.reply.task` content 的 schema 块，`:1263` `"intentLevel"` 行之后）

**Interfaces:** 无代码接口；纯 prompt 文本。产出的 `dimensionDisplayNames` 由 Task 1 的 `RawAgentDecision` 反序列化承接。

- [ ] **Step 1: 加 schema 指令行**

在 `prompts.rs:1263`（`  "intentLevel": "自由生成的意向等级",`）**之后**逐字插入下面这一段（**内层键必须 snake_case**，见下方硬要求）：

```
  // ── 维度中文显示名（仅在你为上面 customerStage / intentLevel 等维度填了"字典里可能没有的自造新值"时才填） ──
  "dimensionDisplayNames": {
    "customer_stage": "为你上面填的 customerStage 值配一个 4-8 字简洁中文名（如 焦虑观望）；若该值是常见标准阶段、或你没把握，就不要填这一项"
  },
```

> **键名规则（硬要求，别写错）**：外层字段名 `dimensionDisplayNames` 用 camelCase（`RawAgentDecision` 走 `#[serde(rename_all=camelCase)]`）；**内层键用 snake_case**（`customer_stage` / `intent_level`）。原因：后端 `pick_dimension_display_name(&names, kind)` 的 `kind` 来自 `compute_taxonomy_guard_outcome` → `get_dimension`，取值恒为 snake_case `customer_stage` / `intent_level`（`domain_signals.rs:92-95`）。内层键若写成 camelCase `customerStage`，`get_str("customer_stage")` 会落空、中文名取不到。上面代码块已是正确形态，照抄即可。

- [ ] **Step 2: 编译确认 prompt 常量无语法问题**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: EXIT 0（`r#"..."#` 原始串内新增内容无需转义；确认没破坏字符串闭合）。

- [ ] **Step 3: 确认未误改版本常量**

Run: `git diff src/prompts.rs | grep -n "PROMPT_PACK_VERSION" || echo "版本常量未改(符合预期)"`
Expected: 输出「版本常量未改(符合预期)」——本 Task 绝不动 `:15` 常量。

- [ ] **Step 4: 提交**

```bash
git add src/prompts.rs
git commit -m "feat(prompts): reply 决策 schema 加 dimensionDisplayNames 指令

LLM 为自造新值(字典外)的维度配简洁中文名(内层键 snake_case 对齐后端
kind 空间),供候选 suggested_display_name 落库。不 bump PROMPT_PACK_VERSION
(启动 align 按内容 diff 生效)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 全链验证 + 三闸 + 诚实标注真模型局限

结构链路（Task 1-3）已各自单测。本 Task 做端到端三闸 + 记录无法本地验证的部分（真模型是否产出合理中文名）。

**Files:** 无改动（纯验证）。

- [ ] **Step 1: lib 基线闸**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: `test result: ok. NNN passed; 0 failed`，NNN ≥ 350。记录实际 NNN。

- [ ] **Step 2: 相关 PBT 编译闸（决策路径 taxonomy 无回归）**

Run: `cargo test --lib decision_taxonomy 2>&1 | tail -10`
Expected: decision_taxonomy 全部单测 PASS（含既有 4 路分支 PBT + 新增取名测试）。

Run: `cargo test --lib -- taxonomy 2>&1 | tail -10`
Expected: gateway `taxonomy_outcome_*` + `pick_display_name` + taxonomy.rs 单测全 PASS。

- [ ] **Step 3: no-human-takeover lint 闸**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 0 violations。新增代码/prompt 无禁用词（「中文名 / 显示名 / 建议名」均不在禁词集）。

- [ ] **Step 4: 端到端逻辑核对（人工读，非自动）**

确认数据流闭环（引用亲验）：
- LLM 产 `dimensionDisplayNames`（Task 4 prompt）
- → `RawAgentDecision.dimension_display_names`（Task 1 反序列化）
- → `AgentDecision.dimension_display_names`（Task 1 carry-through）
- → gateway `:1702` 循环 + decision_taxonomy `:93` 各取名（Task 2/3）
- → `upsert_candidate` 第 7 参 → `TaxonomyCandidate.suggested_display_name`（`taxonomy.rs:429`）
- → 收件箱命名卡 `suggestedDisplayName || rawValue` 预填（前端已就绪，PR #146）

- [ ] **Step 5: 诚实标注真模型局限（不假绿）**

在最终交付说明里记录：**「LLM 是否真按指令产出合理中文名」需真模型验证（server 117 部署最新 main 后跑一轮 webhook 自动回复，检查 `taxonomy_candidates.suggested_display_name` 是否为合理中文）。本地只验证了结构链路（字段承接 + 取名 + 落库传参），未验证 LLM 行为。** 这是待办，不得声称已验证。

- [ ] **Step 6: 汇总提交（若前几步有未提交的验证脚本/笔记则提交；否则跳过）**

本 Task 通常无代码改动，不产生提交。若验证中发现前序 Task 的遗漏，回到对应 Task 修复并重跑三闸。

---

## 自查（Self-Review）

- **Spec 覆盖**：spec §3.1（types 字段+Default+Raw+carry-through）→ Task 1；§3.3（prompt 不 bump）→ Task 4；§3.4（gateway 纯函数+取名）→ Task 2；§3.5（decision_taxonomy 同源必改）→ Task 3；§五（测试）→ 各 Task Step 1 + Task 5；§六（红线）→ Global Constraints + Task 5 Step 3；§七（真模型局限）→ Task 5 Step 5。无遗漏。
- **类型一致**：`pick_dimension_display_name(&Document, &str) -> Option<&str>`（Task 2 定义，Task 3 复用，Task 5 核对）全程一致；`spawn_candidate_upserts` 入参 `Vec<(String,String,Option<String>)>`（Task 3 定义与调用一致）；`dimension_display_names: Document`（AgentDecision）/ `Option<Document>`（Raw）（Task 1 定义，Task 2/3 消费）一致。
- **无占位符**：每个代码 step 均给完整可编译代码 + 精确行号 + 预期输出。
- **依赖顺序**：Task 1 → 2 → 3（Task 3 依赖 1 的字段 + 2 的 `pub(crate)` 纯函数）；Task 4 独立（可任意序，但建议 1-3 后做便于 Step 4 端到端核对）；Task 5 最后。
