# 决策路径标签候选补 AI 中文建议名设计

- 日期：2026-07-08
- 状态：设计待批
- 范围：后端 `src/agent/types.rs`（数据结构 + carry-through + 手写 Default）、`src/prompts.rs`（决策 prompt + 版本 bump）、`src/agent/gateway.rs`（取名传参 + 纯函数）、`src/agent/decision_taxonomy.rs`（同源竞争路径，须同改——写计划时已亲验）

## 一、问题

统一收件箱的「标签候选」命名卡（上一改造 PR #146 交付）在采纳表单里预填「显示名」，逻辑是 `suggestedDisplayName || rawValue`。但收件箱里的候选**绝大多数来自 gateway 决策路径**，而该路径写候选时 `suggested_display_name` 恒 `None`：

- `gateway.rs:1598-1612` 遍历 `outcome.candidate_writes`（`Vec<(kind, raw)>`），调 `taxonomy_upsert_candidate(..., None)` —— 第 7 参 `suggested_display_name` 写死 `None`。
- `decision_taxonomy.rs:112` 同样传 `None`。
- 唯一传中文名的是 `guide_profile.rs:470`（行业配置向导，管理员生成 profile 时 `label` 现成）。

**后果**：小白运营在收件箱打开决策路径产生的候选卡时，「显示名」框预填的是**英文/拼音裸值**（如 `anxious`），需自己改成中文。「AI 建议中文名、运营一键采纳」这个最顺的体验对收件箱主路径兜不住。

**根因（数据流已 file:line 亲验）**：候选值来自 `compute_taxonomy_guard_outcome`（`gateway.rs:5052-5083`）——它对每个维度 `kind` 用 `get_dimension(decision, kind)`（`domain_signals.rs:91`）读出 LLM 填的裸字符串，判为 `CandidateNew`（字典没有）时推入 `candidate_writes`。`AgentDecision` 上维度值就是一个裸 `String`（`customer_stage`/`intent_level` typed 字段 + `domain_signals` 容器），**没有任何中文名伴生字段**。LLM 当前只输出「值」，不输出「值的中文显示名」。

## 二、目标

让 gateway/decision 决策路径产生的标签候选带上 **LLM 生成的中文建议名**，使收件箱命名卡「显示名」预填中文而非英文裸值。名字由决策 LLM 顺带产出（它有完整对话上下文，知道 `anxious` 在此语境该叫「焦虑」还是「担忧」，符合项目 agent-first 立场）。

## 三、方案：决策 LLM 顺带产中文名 + gateway 取名落库

### 3.1 数据结构（`src/agent/types.rs`）

`AgentDecision` 新增维度中文名映射容器：

```rust
/// 维度值 → 中文显示名。LLM 仅在为某维度填了「字典外自造新值」时，
/// 在此为该维度配一个简洁中文名（如 {"customer_stage": "焦虑观望"}）。
/// 字典已有的标准值不必填（已有 canonical label）。gateway 产 taxonomy
/// 候选时按 kind 查此表取中文名作为 suggested_display_name。
#[serde(default)]
pub dimension_display_names: Document,   // camelCase: dimensionDisplayNames
```

Raw 镜像结构（`types.rs` 那组 `Option<Document>` 字段，如 `domain_signals: Option<Document>` at `:460`）加对应可选字段：

```rust
#[serde(default)]
pub dimension_display_names: Option<Document>,
```

**字段 optional 的理由是 LLM 输出容错，不是向后兼容**：按方案该字段「只在 LLM 自造新值时才产出」，即绝大多数轮次它本就不出现（没有新造值就没有名字要配）。若改必填，LLM 任一轮没产出 → decision JSON 反序列化失败 → 决策链路崩。字段可选是其业务本质。

### 3.2 carry-through（`src/agent/types.rs` Raw→decision 透传段，`:~1001`）

在把非-9-自治协议字段从 Raw 透传到 decision 的逻辑里加一行：

```rust
if let Some(v) = raw.dimension_display_names {
    decision.dimension_display_names = v;
}
```

**这是最关键的一处**：漏了它，LLM 输出的字段会在 Raw→decision 转换时被静默丢弃（上一改造在结构化字段上踩过同类坑：新字段必须接 Raw + carry-through，否则 LLM 输出丢失）。实现时须对照 `domain_signals` 等既有字段的透传写法逐字比对。

### 3.3 决策 Prompt（`src/prompts.rs`）

在 reply 决策 prompt 里加一段指令（措辞实现时定稿，语义如下）：

> 当你为 `customer_stage` / `intent_level` / 其它维度填的值可能不在标准字典里（你自造的新值）时，在 `dimensionDisplayNames` 对象里为该维度配一个简洁中文名，例如 `{"customer_stage": "焦虑观望"}`。字典已有的标准值不必填。

**不 bump `PROMPT_PACK_VERSION`**（写计划时已亲验当前机制，纠正早期设想）：`ensure_prompt_pack_v2` → `align_prompt_specs`（`prompts.rs:104,181,240-265`）在启动时按 `normalize_prompt_content(row.content) != normalize_prompt_content(spec.content)`（`:259`）逐 key 内容比对决定是否重种——**内容变了重启必生效，不靠版本号**（`:113` 注释 + `domain_profile.rs:262`「不 bump PROMPT_PACK_VERSION」是既定方案核心）。`PROMPT_PACK_VERSION`（`:15`）只是 stamp 到行上（`:304`）的 provenance 字段，非生效闸。运行时 `state.prompt_pack_version` 是另一个独立的 `AtomicU64` LRU 失效计数器（evolution release / 启动各 fetch_add），与本字符串常量无关。若 bump，只有内容变了的 `user.reply.task` 一行被重种为 v17、其余未变行留 v16 → 行间版本混乱，反而更差。故本改动**只改 prompt 文本，不动版本常量**。

### 3.4 gateway 取名并传入（`src/agent/gateway.rs:1598-1612`）

**取名逻辑抽纯函数便于单测**（镜像上一改造抽 `taxonomy_candidate_to_inbox_item` 纯函数的做法）：

```rust
/// 从维度中文名映射里按 kind 取名；缺键/非串/空串/纯空格 → None（回落英文裸值）。
fn pick_dimension_display_name<'a>(names: &'a Document, kind: &str) -> Option<&'a str> {
    names
        .get_str(kind)
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
```

`candidate_writes` 循环里调它取第 7 参（返回 `Option<&str>`，借用自 `final_decision`，其生命周期覆盖整个循环）：

```rust
for (kind, raw) in &outcome.candidate_writes {
    let display_name = pick_dimension_display_name(&final_decision.dimension_display_names, kind);
    if let Err(error) = taxonomy_upsert_candidate(
        &state.db, &contact.account_id, kind, raw,
        Some("user-ops decision path"), 50,
        display_name,   // 原写死 None
    ).await {
        tracing::warn!(?error, kind = kind.as_str(), raw = %raw, "taxonomy upsert_candidate failed");
    }
}
```

返回类型 `Option<&str>` 与 `upsert_candidate` 第 7 参签名（`taxonomy.rs:354` `suggested_display_name: Option<&str>`）逐字匹配，无需 `.as_deref()`。取不到/空 → `None` → 回落原英文，无回归。

### 3.5 decision_taxonomy 路径（`src/agent/decision_taxonomy.rs:112`）——已亲验：必须同改

写计划阶段已亲验（`decision.rs:1015` + `decision_taxonomy.rs:84-123` + `taxonomy.rs:371-430`）：

- **它是活路径，不是死桩**：`decide_reply_with_promote`（`decision.rs:1015`）每轮调 `validate_and_normalize_decision`，对 `customer_stage`/`intent_level` 同样判 `CandidateNew` 并 `tokio::spawn` fire-and-forget `upsert_candidate(..., None)`（`:112`）。
- **它与 gateway ③ 写同一幂等键**：两条路径的候选键都是 `(scope, kind, raw_value)`，而 `upsert_candidate` 对**已存在**候选**不更新** `suggested_display_name`（`:371-413` 命中即 return，只有 `:416-430` 首次 insert 才写名）——**先写者赢**。
- **结论：④ 必须同改，否则 ③ 的中文名可能被 ④ 的 `None` 抢先写掉**。二者读的是同一个 `final_decision.dimension_display_names`（decision 在 `decide_reply_with_promote` 内产出，gateway 收到的就是它），取到的中文名一致，重复写幂等无害。
- **它能取到名**：`validate_and_normalize_decision(db, &mut decision, ...)` 持有 `&mut decision`，可在 `classify_decision_tags` 返回后、`spawn_candidate_upserts` 之前读 `decision.dimension_display_names`，把 `candidates: Vec<(kind, raw)>` 升级为带名传参。**纯函数 `classify_decision_tags` 不动**（PBT 覆盖），只在生产入口 `validate_and_normalize_decision` / `spawn_candidate_upserts` 侧取名——复用 3.4 的 `pick_dimension_display_name`。

## 四、错误处理

- 全链 best-effort：`dimension_display_names` 缺失/空/取不到 → `None` → 回落英文 → 与现状等价，不阻塞 run（守 CLAUDE.md「unreviewed candidates must not block runs」）。
- LLM 未产该字段（多数轮次的正常情况）→ 空 doc → 全走 `None`，无崩溃。
- 取名纯函数对空 doc / 缺键 / 空串 / 纯空格均返回 `None`。

## 五、测试

- `compute_taxonomy_guard_outcome` 既有单测不动（测试只增量叠加）。
- **新增** `pick_dimension_display_name` 纯函数单测：给带 `dimensionDisplayNames` 的 Document + kind，断言取出正确中文名；缺键/空串/空格/空 doc 断言 `None`。
- **新增** carry-through 单测：构造带 `dimensionDisplayNames` 的 Raw，断言转 decision 后字段保留（守 3.2 头号坑）。
- 三闸：`cargo test --lib`（≥350 passed / 0 failed）、`bash scripts/check-no-human-takeover.sh`（0 违规）。本次不碰前端。

## 六、红线合规

- **AI 全自治**：新增 prompt 指令与代码无「人工接管/介入/托管/接管/takeover/hand-off」禁词；check-no-human-takeover lint 拦截。
- **AI 不自动核验**：候选进字典仍需运营在收件箱点采纳；本改动只是让预填名从英文变中文，不改「AI 提议 + 人工确认」闭环。
- **unreviewed candidates must not block runs**：取名全链 best-effort，任何缺失回落 None，绝不阻塞决策。
- **agent-first**：中文名由 LLM 语义产出（有对话上下文），非词表映射。

## 七、验证局限（诚实标注）

「LLM 真能按指令产出合理中文名」需**真模型验证**。本地只能验证结构链路（carry-through 保留字段 + gateway 取名 + 落库传参）。真模型泛化验证依赖 server 117 部署最新 main，实施计划里标为待办，不假绿。

## 八、不做（YAGNI）

- 不给字典已有值产中文名（已有 canonical label，纯浪费 token）。
- 不新增本地英文→中文词表兜底（agent-first：交给 LLM 语义）。
- 不改 `upsert_candidate` 函数签名（第 7 参 `Option<&str>` 已存在，本就为此设计）。
- 不碰前端（命名卡 `suggestedDisplayName || rawValue` 预填逻辑已就绪，后端喂中文名即自动生效）。
- 不做存量数据迁移/兼容（全新项目，无存量 taxonomy_candidates 需迁就）。
- 不 bump `PROMPT_PACK_VERSION`（见 §3.3：启动对齐按内容 diff 生效，bump 反致行间版本混乱）。
- 不改 `upsert_candidate` 对已存在候选「不更新 suggested_display_name」的语义（③④同源同名，先写者赢已足够；改成"后写覆盖"会引入并发写放大且无收益）。
