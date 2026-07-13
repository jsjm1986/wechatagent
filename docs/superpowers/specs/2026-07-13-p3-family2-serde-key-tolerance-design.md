# P3 家族② serde 键容错 / verified 口径对齐 设计

> 批 E 后续 P3 家族②。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` D-01（:215-225）+ KB-03（:437-445）。两条 Low，均低风险一致性加固。

## 背景与根因（全部主控当场 Read/Grep 亲验，行号基于 origin/main 4ccaf5c 含 #193）

两条 finding 同属"同一语义在不同代码路径口径不对称"元家族，各自独立、都是隔离的纯逻辑/serde 加固，无跨模块张力。

### D-01（PLAUSIBLE · Low）：reply 主路径顶层标签键只认 camelCase，无 snake 双形容错

- `RawAgentDecision`（`types.rs:405`）有 `#[serde(rename_all = "camelCase", default)]`，顶层 `customer_stage`（:461）/ `intent_level`（:462）序列化只认 camelCase 键 `customerStage`/`intentLevel`，**无** `#[serde(alias)]`。
- LLM 若顶层输出 snake_case `customer_stage`/`intent_level` → serde 静默 miss → `None`（`default` 不报错、不报 risk）→ promote（`types.rs:1008-1012` 靠 `raw.customer_stage.is_some()`）不透传 → 标签丢失、画像不精准。
- **不对称**：初始画像路径 `decision.rs:150-153` 有手动双形兜底 `optional_string("customerStage").or_else(|| optional_string("customer_stage"))`；reply 主路径只容 camel。历史 PR#151 修的是 `Document` 内层键双形，未覆盖此顶层 serde 键。
- **关键亲验（消除误吸风险）**：prompt 模板要求 LLM 顶层输出 camelCase `customerStage`/`intentLevel`（`prompts.rs:1085-1086`/`:1262-1263`），而 `dimensionDisplayNames` 嵌套对象内用 snake_case `customer_stage`/`intent_level` 做**中文显示名**（`:1266-1267`，语义完全不同，值如「焦虑观望」）。serde 顶层 alias **只作用于顶层键空间**，不会去嵌套 `Document`（`types.rs:132` `dimension_display_names: Document`）内抓同名键，故加 `alias="customer_stage"` 不会把中文显示名误吸为权威 stage 值。

### KB-03（PLAUSIBLE · Low · latent 当前不可触发）：verified 判定口径大小写不一致

- 硬闸 `guards.rs:314` `is_verified` 用 `eq_ignore_ascii_case("verified")`（大小写不敏感）；而**所有召回过滤**（`knowledge_router.rs:71` / `knowledge_agent.rs` 各 tool / `chat.rs:1076` / catalog）均 `== "verified"` 精确匹配。
- 写入侧 `verify.rs:112` 恒写小写 `"verified"`，当前不可能产生 `"Verified"`。若真出现，召回侧精确匹配失败 → 切片根本不进语料 → 不注入、不进 finalize 的 knowledge_chunks → `is_verified` 的宽松判定永远碰不到它 → 既不误放也不误堵。纯 latent 口径漂移。

## 目标

① reply 主路径顶层 `customer_stage`/`intent_level` 容 camel+snake 双形，与初始画像路径对齐（D-01）；② `is_verified` 判定口径收窄为精确小写 `== "verified"`，与写入侧、召回侧三方统一（KB-03）。两条独立、都是低风险加固。

## 架构：两条独立加固

### D-01 —— RawAgentDecision 顶层标签键加 serde alias

`types.rs:461-462` 给两字段各加 alias：

```rust
#[serde(alias = "customer_stage")]
pub customer_stage: Option<String>,
#[serde(alias = "intent_level")]
pub intent_level: Option<String>,
```

`rename_all="camelCase"` 让主名是 `customerStage`/`intentLevel`；`alias` 让 serde **额外**接受 snake_case 键。两形态都命中同一字段，不再静默 miss。

**安全性质**：alias 是**纯增量放宽**——原本能反序列化的 camelCase 键行为不变，只新增接受 snake_case 键。不改任何既有成功路径。已亲验嵌套 `dimensionDisplayNames` 的同名 snake_case 中文显示名键不在顶层反序列化范围（serde 顶层 alias 不下探嵌套 Document），无误吸风险。

### KB-03 —— is_verified 收窄为精确匹配

`guards.rs:314`：

```rust
// 旧：chunk.integrity_status.eq_ignore_ascii_case("verified")
chunk.integrity_status == "verified"
```

与写入侧（`verify.rs:112` 恒写小写）、所有召回过滤（`== "verified"`）三方口径统一到"精确小写"。

**安全性质**：写入侧恒写小写，当前不存在 `"Verified"`，此改动对现有数据行为**完全等价**（纯 latent 口径收敛）。方向选择"向数据实况收敛"而非"放宽召回侧"——后者需改 5+ 处 Mongo 查询（`$regex`/collation 才能不敏感，复杂度高），且是为不可能出现的大写数据留后门，违 YAGNI。

## 回归风险

1. **D-01 alias 是纯增量放宽**：camelCase 既有路径零变化，只新增 snake 容错；无误吸嵌套中文显示名（已亲验）。
2. **KB-03 对现有数据完全等价**：写入侧恒小写，收窄判定只是消除 latent 漂移，不改任何现有 chunk 的 verified 判定结果。
3. **baseline**：两处改动都在 `src/agent/`，不触 baseline 门 4 PBT（state_transition/memory_card/wiki_chunk_revision/llm_retry_jitter），lib≥350 不回退。新增确定性单测各 1。

## 改动面

- **Modify** `src/agent/types.rs:461-462`：`customer_stage`/`intent_level` 各加 `#[serde(alias)]`（D-01）+ 新增 serde 双形单测。
- **Modify** `src/agent/guards.rs:314`：`is_verified` 的 `eq_ignore_ascii_case("verified")` 改精确 `== "verified"`（KB-03）+ 新增精确口径单测。

## 测试计划

- **D-01 serde 双形单测（lib）**：构造顶层 snake_case JSON（`{"customer_stage":"...","intent_level":"..."}`）→ `serde_json::from_str::<RawAgentDecision>` → 断言两字段 `Some(...)`；构造顶层 camelCase JSON → 同样 `Some(...)`。回退到无 alias 时 snake 变体断言变红（真回归哨兵）。
- **KB-03 精确口径单测（lib）**：`is_verified` 对 `"verified"` 返 true、对 `"Verified"`/`"VERIFIED"` 返 false。回退到 `eq_ignore_ascii_case` 时大写变体断言变红。

## 非目标（YAGNI）

- 不给 RawAgentDecision 其它 snake_case 字段无差别加 alias（仅修台账点名的两个权威标签字段；其它字段无此 finding、无证据漂移）。
- 不放宽召回侧大小写（KB-03 选收窄硬闸方向，不动 5+ 处 Mongo 查询）。
- 不动 promote 逻辑、prompt 模板、写入侧 verify.rs。
