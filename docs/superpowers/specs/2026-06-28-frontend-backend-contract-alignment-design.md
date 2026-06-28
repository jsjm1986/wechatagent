# 前后端契约彻底对齐 + 可复用检验方法论 设计

> 状态：设计已定（经四轮证伪 + 端到端 POC 硬验）。本文是实现计划（writing-plans）的依据。
> 日期：2026-06-28

## 1. 问题与根因

### 1.1 现象
前端反复出现"后端能力远超前端兑现"：字段下发了前端不读、后端改了形状前端静默错位、错误态被吞成空态。最近一次对齐审查确认 76 个缺口、0 误报。这不是某个页面的 bug，是**契约维护机制缺失**。

### 1.2 根因：线上契约被维护三遍，零强制联动
一条线上字段，今天要在三处各写一遍、且互不校验：

1. **Rust model**（`src/models.rs`，BSON-serde 结构体，snake_case）
2. **手写投影函数**（`src/routes/**` 里 ~31 个实体级 `xxx_json()`，把 model `json!({...})` 成 camelCase 线上形状，含 `ObjectId.to_hex()` / `dt_to_string` / BSON `into_relaxed_extjson()` / 跨源聚合等变换）
3. **手写前端类型**（`frontend/src/types/index.ts`，753 行，inline interface / 继承 / `Record<string,unknown>` 兜底混用）

三者之间**没有任何自动校验**。改了 model 不强制改投影，改了投影不强制改前端类型。漂移是默认结果，不是意外。

### 1.3 为什么前端类型本身也是散的
POC 与审查都确认：前端没有"canonical 契约类型"这一层。`Contact`（types/index.ts:84）算接近 canonical 的好例子，但 chunk 这类散落 7 处、且列表/详情形状硬冲突（见 1.4）。所以"和前端类型对账"这个朴素想法信噪比不足——前端类型本身不可信为基准。

### 1.4 投影是契约锚点，不是 endpoint，更不是 resource
同一个 chunk 资源，列表端点 `operation_knowledge_chunk_json`（camelCase，逐字段手工映射）与详情端点 `crud.rs:357`（`json!({"item": item})` 裸 struct，snake_case + `{$oid}`）**形状不同**。所以契约锚定的最小单位只能是**投影函数**，不能是资源、不能是 endpoint。

## 2. 已证伪的方案（不要重走）

| 方案 | 为什么不行 |
|---|---|
| codegen / ts-rs / typeshare / OpenAPI 从 Rust struct 生成 TS | 投影**不是** struct 的纯 serde 序列化。每个投影都有 `ObjectId.to_hex()` / `DateTime→rfc3339` 变换、BSON Document 桥接、跨源聚合（如 `operation_health_json` 吃 3 个 model）。codegen 反射 struct 得到的形状与线上形状系统性不符。已三路代码验证。 |
| 朴素键集 diff（投影 keys vs 前端类型 keys） | 前端无 canonical 类型（1.3），信噪比不足，且无法表达"列表/详情形状不同"。 |
| TS 编译期 excess-property 检查 | `import fixture from "*.json"` 不触发多余属性检查；tsc 对 JSON import 推断为宽类型，漏报。 |

## 3. 选定机制（四轮证伪 + POC 验证）

### 3.1 一句话
**后端纯函数测试构造 model → 调投影 → canonicalize → 写一份 fixture JSON；前端 vitest 导入同一份 fixture 做键集对账。fixture 是前后端唯一真相源，双门互补。**

### 3.2 锚点 = 投影函数
每个实体级投影函数（`-> Value`、`pub(super)`/`pub` 可见、吃一个 model）配一个契约测试。列表与详情是两个投影，各自一份 fixture。

### 3.3 后端门：纯 lib 快照测试（进 `cargo test --lib` 硬门）
- `#[cfg(test)]` 测试构造**全量赋值**的 model（每个 Option 都给 Some，每个 Vec 非空），调投影得 `Value`。
- `canonicalize`：递归排序对象键（消除嵌套 BSON Document 的键序抖动），pretty-print。
- 默认**只读对账** `frontend/src/contracts/<name>.fixture.json`；不一致 panic，报错带 re-bless 指令。
- `UPDATE_SNAPSHOTS=1` 时**写** fixture（bless）。
- fixture 路径用 `env!("CARGO_MANIFEST_DIR")` 定位（仓库非 cargo workspace，frontend/ 是子目录，POC 已验证此路径跨目录可写）。
- 零 Docker、零可见性改动、纯函数，复用现有 `chunk_json_emits_product_tags`（mod.rs:1385）同款模式。

### 3.4 前端门：vitest 运行时键集对账（非 tsc 编译期）
- `frontend/src/contracts/<projection>.contract.ts` 导出 `CANONICAL_KEYS = [...] as const`：前端显式声明"我知道后端下发这些键"。
- `src/__tests__/contracts/<projection>.contract.test.ts` 导入**同一份** fixture JSON + `CANONICAL_KEYS`，`Object.keys(fixture)` 与声明做**双向集合比对**：
  - `missingInFrontend`（后端发了、前端没声明）→ 红：强制前端登记并处理新字段。
  - `deadInFrontend`（前端声明了、后端没发）→ 红：强制前端清理死键。
- 这是"后端更新后前端必须对应处理"的**强制点**。

### 3.5 双门为何互补、不冗余（POC 模式 A 续已证）
- 后端门挡"投影变了但没 bless fixture"。
- 前端门挡"bless 了 fixture 但前端 CANONICAL_KEYS 没同步处理"。
- 少任一门都有漏网路径：一个粗心 dev 可能 re-bless 过了后端门却没碰前端——前端门此时报 `missingInFrontend`。

### 3.6 canonical 类型只供对账，不强塞既有视图（三条边界）
- `CANONICAL_KEYS` 仅服务对账测试，**不**改 `types/index.ts` 的业务类型、**不**强塞进既有组件 props。
- 对账只断言**键集合**，不断言语义/值/可选性冲突（避免把 outcomeStatus 裸英文这类"有意设计"误判为缺陷）。
- 已 canonical 的类型（如 Contact）直接复用，不另立 `ContractContact` 重复。

### 3.7 稳定性
- `canonicalize` 递归排序键 → 嵌套 BSON Document 键序不再抖动（POC 重跑零 flake）。
- 投影内无 `now()`/`uuid()`，model 字段全部测试显式赋固定值（固定 `DateTime::from_millis`），故无需时间/随机洗刷器。

## 4. 三类响应全覆盖

### 4.1 实体级投影（主体，~31 个）
按 3.3/3.4 处理。这是覆盖主力。

### 4.2 聚合响应（~8 个，observability 类 inline 拼装）
如 `operation_health_json`（吃 3 个 model）、observability.rs 里 inline 的聚合。这些不是单 model → Value，构造成本高。策略：
- 能纯函数构造的（如 `operation_health_json` 接收 model 参数）纳入 3.3 同款快照。
- 真正 inline 在 handler 里、需起 `TestApp` 才能产出的，用 `tests/` 集成测试 + golden 快照覆盖（接受 Docker 成本，进 CI integration job）。

### 4.3 raw Document 字段（三档容差）
投影里直接 `into_relaxed_extjson()` 下发的 BSON Document，按前端依赖度分三档：
- **固定结构**（usageStats / provenance / runtimeParametersSnapshot）→ 纳入快照，键集严格对账。
- **前端消费但结构松**（scores / sourceAnchors / domainAttributes / nextBestAction / formulaBreakdown）→ 快照纳入，但对账只断言**已知键存在**，不禁止新增键（结构容差）。
- **纯审计、前端不读**（promptVersions / contextPackSnapshot / domainConfigSnapshot / reactionAnalysis）→ 整体从快照**剔除**（用 canonicalize 前的字段过滤），不浪费维护成本。

> POC 顺带暴露的真 wart：`relatedChunks[].chunk_id` 是 snake_case（裸 `RelatedRef` serde），而顶层全 camelCase。这类嵌套不一致正是本机制要暴露的——记入 4.3 第二档，铺开时逐个定性（修 or 容差）。

## 5. 五域分批铺开

按 `src/routes/` 文件归属分五批，每批独立可测、独立 PR：

1. **知识域**：`operation_knowledge_{document,chunk}_json`、`knowledge_usage_json`、`revision_applied_to_json`、crud.rs 详情投影。（POC 已落 chunk 列表，作为模板）
2. **运营/Agent 域**（shared.rs）：`decision_review_json`、`agent_run_json`、`operation_health_json`、`outcome_metric_json`、`llm_call_log_json`、`operating_memory_json`、`memory_candidate_json`、`guide_preview_json`、`behavior_signal_metric_json`。
3. **字典/分类域**：`taxonomy_entry_json`、`taxonomy_candidate_json`、`operation_domain_json`、`operation_state_policy_json`、`relationship_suggestion_json`。
4. **进化/实验域**（evolution.rs）：`experiment_summary_json`、`proposal_{summary,detail}_json`、`shadow_replay_json`、`threshold_override_json`、`threshold_override_audit_json`、`runtime_flag_json`。
5. **配置/playbook 域**：`playbook_json`、`prompt_template_json`、`evaluation_scenario_json`、`suspected_deal_json`、`outbox_entry_json`。

每批落地后 `CONTRACT_COVERAGE.md`（或测试内清单）登记已覆盖投影，供防腐烂 lint 比对。

## 6. 防腐烂 lint（强制每个投影都有契约测试）

### 6.1 为什么不能手维护清单
现有 `no_orphan_pub_async_route_handlers`（mod.rs:1052）用手写 `include_str!` 清单，已确认腐烂（10 文件/20 handler 逃逸）。手维护必腐烂。

### 6.2 机制：运行时 glob 扫描
一个 `#[test]`（进 `cargo test --lib` 硬门，与契约测试同区）运行时 glob 扫 `src/routes/**`，用正则识别投影函数（`fn \w+_json` 且 `-> Value` 返回 + 非 helper 黑名单），断言每个都有：
- 对应的 `<name>.fixture.json` 存在，且
- 对应的契约测试存在（grep 测试源文件里的测试名）。

新增投影函数但忘了配契约测试 → 此 lint 红。这是"未来加投影自动被纳管"的护栏。选 `#[test]` 而非 build 脚本：build 脚本不进测试结果、CI 不拦，违背"机器强制"初衷。

### 6.3 helper 黑名单
`bson_from_json` / `bson_doc_to_json` / `parse_warning_to_json` / `vision_generate_json` / `lesson_doc_to_json` 等非实体投影显式列入豁免，注释说明理由。

## 7. CI 接线

### 7.1 后端门
进现有 `cargo test --lib` baseline 硬门（≥350/0）。契约测试是纯 lib 测试，自动被 baseline gate 执行。**不**进 continue-on-error 的 integration job（那是假绿区）。

### 7.2 前端门：新建独立 CI job
- 现状：`ci.yml` 的 `paths-ignore` 含 `frontend/**`（workflow 级），前端改动根本不触发任何 job。
- 改造：用 `dorny/paths-filter` 在 job 内判定，或从 `paths-ignore` 移除 `frontend/**` 并新增 `frontend-contract` job：`npm ci` → `tsc --noEmit` → `vitest run`。
- 保持后端 job 仍对纯前端改动跳过（避免烧真模型配额）——用 paths-filter 而非全局 paths-ignore 实现两者并存。

## 8. 三层审查（机制只是地基，审查抓语义）

1. **结构层（机器）**：第 3-6 节的双门 + lint，抓键集漂移。自动、零判断。
2. **语义层（agent 读全链）**：agent 真读 model → 投影 → store → component → render 全链（POC 外的审查已实活，抓到 outcomeStatus 裸英文并定性为有意设计）。天然可并行，每个投影一个 agent。这层抓"键对了但语义错位/前端不消费/错误态吞空态"。
3. **行为层（真跑 UI 不现实）**：降级为读代码 + 组件测（vitest + testing-library，如现有 operations.test.tsx）。

## 9. 覆盖盲区（诚实声明，不开空头支票）

- **聚合响应**：inline 在 handler 里、必须起 TestApp 的那几个，依赖 integration job（Docker），本地不跑。
- **raw Document 第二档**：只断言已知键，新增子键不报警——这是刻意的结构容差，不是覆盖。
- **值级正确性**：对账只管键集，不管值对不对（值的正确性靠语义层审查 + 既有业务测试）。
- **详情端点裸 struct**（如 crud.rs:357 `json!({"item": item})`）：形状 = model 的 serde 默认（snake_case + `{$oid}`），与列表投影冲突。本机制能快照它、暴露冲突，但"统一列表/详情形状"是产品决策，不在本工程范围（只暴露、不强制统一）。

## 10. 错误处理

- fixture 缺失（首次/没 bless）→ 后端测试 panic，提示 `UPDATE_SNAPSHOTS=1` 生成。
- fixture 非法 JSON → panic 提示。
- bless 写空对象 → 前端门第二个断言（`actualKeys.length > 0`）兜底报红。

## 11. 测试策略

本工程的"测试"就是契约测试本身。验证它有效的标准（POC 已全部实证）：
- **真绿**：对齐稳态下后端只读对账 + 前端 vitest 双绿。
- **真红 模式 A**：后端投影加字段不 re-bless → 后端门红（diff 指出多键）。
- **真红 模式 A 续**：re-bless 了但前端没登记 → 前端门红（`missingInFrontend`）。
- **真红 模式 C**：前端 CANONICAL_KEYS 多声明死键 → 前端门红（`deadInFrontend`）。

铺开每批时，每个投影至少补一个"故意漂移测红再还原"的自验证记录（不留在代码里，作为 PR 描述证据）。

## 12. POC 实证结论（已完成，代码在工作树未提交）

- 后端 `assert_contract_fixture` helper + `operation_knowledge_chunk_json_matches_contract_fixture` 测试：mod.rs，已跑通真绿。
- `frontend/src/contracts/operation_knowledge_chunk.fixture.json`：33 键，bless 生成。
- `frontend/src/contracts/operationKnowledgeChunk.contract.ts`：`CANONICAL_KEYS`（33 键 as const）。
- `frontend/src/__tests__/contracts/operationKnowledgeChunk.contract.test.ts`：键集双向对账，真绿。
- 真红三模式全部用可运行代码证明（第 11 节）。

POC 这套代码即为 spec 的可执行参照，writing-plans 的 Task 1 直接以它为模板固化，其余四域照搬。

## 13. 全局约束（实现时逐字遵守）

- 仅在用户明确要求时 commit。
- 测试 only，绝不为过测试改业务逻辑/prompt/guards/阈值（过拟合红线）。
- 新增测试只增量叠加，不删改旧维度。
- `cargo test --lib` baseline ≥350/0 + 4 PBT ≥33/0 不退。
- 跑 cargo 前 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`。
- 子 agent 一律 `model: "opus"`。
- 回复用中文。
