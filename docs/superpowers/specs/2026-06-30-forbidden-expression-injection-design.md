# 禁用表达独立段注入 设计 spec

> 日期：2026-06-30　分支基线：origin/main（含 PR#66 文本资产可用性补全）

## 问题

`forbidden_expression`（运营在内容资产库按条建的「禁用表达/禁语」素材）当前被混在决策提示词的「可引用内容资产:」段里注入，靠英文 kind 标签 `[forbidden_expression]` 自辨语义，**没有任何显式的「禁止」框定**。

- 语义反转：禁语的业务语义是「绝不能说这些」，却被罩在「可引用内容资产」标题下，弱模型可能把红线词当可引用素材直接说给客户。
- 这是 PR#63（文本资产分档注入）之前就存在的既有缺陷，但本系列改造鼓励运营多建文本资产、按档注入，禁语被建得越多，暴露面越大。

## 核查确认的事实（动手前已全量核查）

1. `load_context_assets`（`src/agent/decision.rs:1483`，唯一调用点 `:334`）是 `forbidden_expression` 注入提示词的**唯一消费点**。无别的 prompt 构造、review agent、knowledge router 读它。并列加载器 `load_sendable_assets`(:1368) / `load_referral_cards`(:1422) 的 query 都不含 forbidden_expression。
2. **当前 forbidden_expression 没有任何代码层强制拦截**——它纯粹被当「可引用内容」文本注入（渲染 `- [kind] 标题: 正文`），靠 LLM 自觉不说。`src/agent/guards/`、`review`、`gateway` 均无引用该字面量。
3. `ContentAsset.kind` 是自由 `String`（`src/models.rs:939`），无 enum 约束、无种子、无针对 forbidden_expression 的特殊 serde。
4. 前端 `KIND_OPTIONS`（`frontend/src/features/content-assets/index.tsx:15`）已有 `{value:"forbidden_expression", label:"禁用表达"}`，建/编辑能力与其他文本 kind 同构、完整。
5. 提示词模板「可引用内容资产:」标题（`decision.rs:901`）下是**三个连续 `{}`**（:902/903/904），分别由 `assets`（:950 文本资产）、`format!("{sendable_candidates_text}{sendable_overview_text}")`（:954 可发送素材）、`format!("{referral_block}{assist_escalation_hint}{assist_redline_yield}")`（:960 名片引荐）填充——三块共享同一段落标题。

## 设计决策（已与用户对齐）

| # | 决策 | 取值 |
| --- | --- | --- |
| D1 | 禁语是否受 min_inject_tier 过滤 | **否，恒注入**。禁语是安全红线，Lean/Relational/Full 任何档都注入，忽略该条的 min_inject_tier（运营建错档也不会让红线在轻档消失）。其余 text/faq/script/brand_voice 仍按各自 tier 过滤。 |
| D2 | 禁语段位置 | **独立段**，贴在「可引用内容资产」三块整体之后、「可引荐的专属顾问:」之前。 |
| D3 | 本次剖出范围 | **只剖 forbidden_expression**。brand_voice/text/faq/script 不动（brand_voice 是「按这个口吻说」，属可引用风格指引，无语义反转，YAGNI）。 |
| D4 | 查询实现路径 | **方案 A**：一次查询拉全 + Rust 侧纯函数分流。禁语豁免直接进 query 的 `$or`，保留 PR#63 的 tier 下推（可引用 4 类仍享 `$in` pushdown）。 |
| D5 | 是否加出站硬拦截门 | **否**。本次只改提示词注入语义，不在 review/gateway 加「出站文本命中禁语即拦截」的门（那是更大专题，涉 guard/语义匹配/误伤风险，超范围）。 |

## 改动单元（3 处，全在 `src/agent/decision.rs`）

### 1. `load_context_assets` 查询条件 + 返回值（约 :1483-1530）

**查询 filter**：tier 条件从「对全 5 类生效」改为「只对可引用 4 类生效，禁语豁免」。结构：

```rust
// kind 维度拆成两支：可引用 4 类受 tier_cond 约束；forbidden_expression 恒拉（无 tier）
"$or": [
    {
        "kind": { "$in": ["text", "faq", "script", "brand_voice"] },
        "$and": [ tier_cond ]      // tier_cond 同现状（Full 含 $exists:false 老数据兜底）
    },
    { "kind": "forbidden_expression" }
]
```

`workspace_id` / `account_id($or null)` / sort(updated_at desc) 不变。`limit` 12 → **16**（禁语不挤占可引用名额；禁语通常少，16 给两组留余量）。

**返回值**：`AppResult<String>` → `AppResult<(String, String)>`，即 `(referable, forbidden)`。游标遍历改为先收集成 `Vec<ContentAsset>`，再交给纯函数 `split_context_assets` 分流渲染。

### 2. 新增纯函数 `split_context_assets`（同文件，紧邻 load_context_assets）

```rust
/// 把查回的内容资产按 kind 分流渲染成两段提示词文本：
/// - 可引用组（text/faq/script/brand_voice）：保留 `- [kind] 标题: 正文`，语义＝可引用。
/// - 禁语组（forbidden_expression）：渲染 `- 标题: 正文`（不带 kind 标签，段落标题已框定语义）。
/// 返回 (referable, forbidden)，各自 `\n` 连接；某组空 → 空串。
pub(crate) fn split_context_assets(assets: Vec<crate::models::ContentAsset>) -> (String, String) {
    let mut referable = Vec::new();
    let mut forbidden = Vec::new();
    for asset in assets {
        let body = asset.body.unwrap_or_default();
        if asset.kind == "forbidden_expression" {
            forbidden.push(format!("- {}: {}", asset.title, body));
        } else {
            referable.push(format!("- [{}] {}: {}", asset.kind, asset.title, body));
        }
    }
    (referable.join("\n"), forbidden.join("\n"))
}
```

`load_context_assets` 内部：`let (referable, forbidden) = split_context_assets(collected); Ok((referable, forbidden))`。

### 3. 提示词模板 + 拼接参数（约 :901-904 模板、:950 参数）

**模板**：在「可引用内容资产:」三块（三个 `{}`，:902/903/904）整体之后、「可引荐的专属顾问:」（:905）之前，插入新段：

```
可引荐的专属顾问:    ← 现有，前面插入下面两行
```

改为：

```
以下表达禁止使用（运营标注的禁语，不得直接说，也不得改写后变相说）:
{}

可引荐的专属顾问:
```

**调用点**：
- `:334` 处 `let assets = load_context_assets(...).await.unwrap_or_default();` 改为 `let (referable_assets, forbidden_assets) = load_context_assets(...).await.unwrap_or_default();`（`unwrap_or_default()` 对 tuple 返回 `(String::new(), String::new())`，best-effort 语义不变）。
- 参数列表 `:950` 的 `assets` 改为 `referable_assets`；在三块（assets/sendable/referral，:950/954/960）之后、对应新 `{}` 的位置插入 `forbidden_assets`。

**占位符对位铁律**：新加的 `{}` 在模板里位于第 N 个，参数列表里 `forbidden_assets` 必须插在第 N 个位置（紧跟 `format!("{referral_block}...")` 之后、`task_text` 之前）。错位会让后续所有占位符整体移位——实现时逐个数对。

## 数据流

```
content_assets(forbidden_expression, 任意 tier)
  ├─ load_context_assets 一次查询（禁语豁免 tier，可引用4类享tier下推）
  ├─ 收集 Vec<ContentAsset>
  ├─ split_context_assets 纯函数分流渲染
  ├─ (referable, forbidden) 两串
  └─ referable→「可引用内容资产」原槽；forbidden→新「禁止使用」段
       → LLM 在独立段看到禁语，与「可引用」物理隔离
```

## 错误处理

不变。DB 故障 → `load_context_assets` 返 `Err` → 调用点 `.unwrap_or_default()` → `(String::new(), String::new())`，两段皆空，不阻塞决策（同现有 reaction_hint / sendable best-effort 路径）。

## 测试

- **新增 lib 单测**覆盖 `split_context_assets` 纯函数：
  - 禁语条进 forbidden 组、其余 4 类进 referable 组；
  - 渲染格式（referable 带 `[kind]`、forbidden 不带）；
  - 某组空 → 空串；混合输入两组都非空；
  - body 为 None → 渲染空正文不 panic。
- 现有 `asset_visible_at_tier` / `visible_min_tiers_for` 测试**不动**（tier 逻辑只服务可引用 4 类，禁语不走 tier）。
- 注入侧 DB 端到端留给现有 `#[ignore]` + testcontainers，不新增重型集成测试。
- CI 基线门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0（新函数 `pub(crate)` + 被 load_context_assets 生产调用，无 dead-code）。

## 不做（YAGNI 边界）

- 不加出站硬拦截门（D5）。
- 不动 brand_voice/text/faq/script 的注入逻辑（D3）。
- 不动 playbook 级「禁用规则:」段（`decision.rs:1232`，来源是 `playbook.forbidden_rules` 域级方法论禁语，与 content_assets 的 forbidden_expression 是两个来源，本次不合并、不去重）。
- 不动前端（建/编辑禁语资产能力已完整，KIND_OPTIONS 已有选项）。
- 不碰知识库 wiki 注入、顾问名片发送链、文件素材发送编排。

## 诚实边界

本改动是**提示词层语义修正**：把禁语从「可引用」反转为「禁止」框定，降低弱模型误说概率；段落措辞写成强指令但不暗示存在代码硬门。禁语最终仍依赖 LLM 遵从——**无代码层强制拦截**（出站硬门是后续可选专题，本次不做）。

## 命名红线

新增行（段落文案「以下表达禁止使用…」、函数注释、测试文案）不得含 CI 禁词 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。本设计用词「禁止使用 / 禁语 / 红线」均合规。
