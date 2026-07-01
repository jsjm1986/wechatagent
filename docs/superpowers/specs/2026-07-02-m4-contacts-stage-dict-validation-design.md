# M4 contacts.rs 三处建档端点 AI 生成 stage 缺字典校验修复设计

> 日期：2026-07-02
> 分支：`fix/m4-contacts-stage-dict-validation`（从 origin/main 5be5f20 切，含 M1 #90）
> 来源：终极审判审计 M4（UPHELD Medium）

## 1. 漏洞描述（对最新代码 100% 亲验）

`contacts.rs` 三处建档/重建档端点调 `agent::build_initial_operation_profile`（LLM 生成初始画像），拿到 `generated.customer_stage` / `generated.intent_level` 后，**未经任何字典校验直接经 `insert_domain_stage_fields` 写库**：

- `enable_agent`（contacts.rs:428）
- `update_profile_note`（contacts.rs:519）
- `analyze_contact_profile`（contacts.rs:940）

三处均是 `insert_domain_stage_fields(&mut set_doc, generated.customer_stage.as_deref(), generated.intent_level.as_deref(), true)` —— `insert_domain_stage_fields`（shared.rs:93-108）只把值塞进 `domain_attributes` 容器，**无任何 `system_taxonomies` 字典校验**。

### 对照：正确样板已存在

后台管理 Agent 的等价建档路径（`management.rs:1402-1435`）对**同一批** `generated.customer_stage` / `generated.intent_level` 走了字典校验：

```rust
let gen_stage = match generated.customer_stage.as_deref() {
    Some(v) => apply_admin_dim_validation(
        agent::dimension_registry::validate_dimension_value(
            &state.db, "customer_stage", v, &contact.account_id,
            agent::dimension_registry::WriteIntent::MachineWrite,
        ).await,
    )?,
    None => None,
};
// intent_level 同理
insert_domain_stage_fields(&mut set_doc, gen_stage.as_deref(), gen_intent.as_deref(), true);
```

AI 主决策路径也校验（`decision.rs:1015` → `validate_and_normalize_decision`：Active 通过 / AliasActive 改写 canonical / Deprecated 加 risk / CandidateNew 加 risk + 异步 upsert candidate）。

### 违反的红线

CLAUDE.md「双层标签」硬规则：`customer_stage` / `intent_level` / `objection_type` **必须来自 `system_taxonomies`**；自由生成的取值走 `taxonomy_candidates` 待审，**不得直接落库权威字段**。三处 contacts.rs 端点让 LLM 臆造的 stage（如「价格异议中段」这类未登记值）绕过字典直接写 `domain_attributes.customer_stage`，污染权威维度、且 alias 值不归一（同义异名不收敛）。

### 后果

同一个 `build_initial_operation_profile` 输出，走 management 建档被校验/归一/收候选，走 contacts 三端点（前端「启用 Agent」「重新分析画像」「改备注重生成」按钮）则原样落库。安全门在两条 admin 路径间**不对称**，与 M1 的发送门不对称同源。

## 2. 根因

`management.rs` 的建档路径在标签可信度改造（I1）时补了 `validate_dimension_value(MachineWrite)`，但 `contacts.rs` 的三处兄弟端点未同步接入——它们仍直传 `generated.*` 给 `insert_domain_stage_fields`。这是改造遗漏的一致性缺口，不是有意取舍（无任何注释说明为何 contacts 路径豁免校验；management 路径的注释「AI 产出 → WriteIntent::MachineWrite：越界值 drop」恰恰说明这才是设计意图）。

## 3. 方案

### 方案 A（选定）：三处 contacts.rs 端点镜像 management.rs 的 MachineWrite 校验

在三处 `insert_domain_stage_fields` 调用**之前**，对 `generated.customer_stage` / `generated.intent_level` 各跑一次 `apply_admin_dim_validation(validate_dimension_value(..., MachineWrite))`，用校验/归一后的值替代原始 `generated.*` 传入 `insert_domain_stage_fields`。与 management.rs:1402-1435 逐字节等价。

**为什么用 `MachineWrite` 而非 `AdminWrite`**（已核实 dimension_registry.rs:108-143）：这些值是 **LLM 生成**（`build_initial_operation_profile` 的 `generate_agent_json` 产出），不是 admin 手输。`MachineWrite` 语义 = 越界值 `DropSilently`（不阻断建档，只是不写脏 stage）；`AdminWrite` 语义 = 越界 `Reject`（返 400）。用 AdminWrite 会让 LLM 偶发臆造直接把「启用 Agent」按钮打成 400，破坏建档流程。management.rs 正是用 MachineWrite，镜像它。

**处置语义（classify_validation，已核实）：**
- `Active` / `Deprecated`（字典登记值）→ `Accept(原值)` 写入；
- `AliasActive` → `Accept(canonical)` 归一后写入；
- `KindUnconfigured`（该 kind 字典整个未配置）→ `Accept(原值)`，**不约束**（对齐既有「未配置≠越界」语义，避免 stage 永不落库回归）；
- `Miss`（字典有其它条目、仅此值越界）+ MachineWrite → `DropSilently` → 该维度不写（`insert_domain_stage_fields` 收到 `None` 即跳过该键，内核守卫兜住不刷时间戳）；
- 空串 → `DropSilently`。

零扰动保证：DEFAULT 销售域字典已 seed customer_stage/intent_level 的标准值，LLM 正常输出标准值 → `Active` → `Accept(原值)` → 行为字节等价。只有 LLM 臆造越界值时才 drop（这正是修复目标）。

### 否决方案 B（把校验塞进 `insert_domain_stage_fields` 内部）

`insert_domain_stage_fields` 是同步纯函数（无 `&db`、无 `async`），而 `validate_dimension_value` 需要 `&state.db` + `async`（查 taxonomy cache）。把校验塞进去要改签名为 async + 传 db，波及 shared.rs 的 12 处调用（含 guide-preview apply 等已自带 AdminWrite 校验的路径，会双重校验）。且 shared.rs 里已有 `apply_admin_dim_validation` 供调用方在**外层**用——既有架构就是「校验在调用方、写入用纯函数」。方案 A 顺架构，B 逆架构。否决。

## 4. 核心改动

落点：`src/routes/contacts.rs` 三处（enable_agent / update_profile_note / analyze_contact_profile）。

每处把：
```rust
insert_domain_stage_fields(
    &mut set_doc,
    generated.customer_stage.as_deref(),
    generated.intent_level.as_deref(),
    true,
);
```
改为（镜像 management.rs:1400-1435）：
```rust
// M4：AI 生成的初始画像 stage/intent 经 dimension_registry 校验后再落库
// （对齐 management.rs 建档路径 + AI 主决策 validate_and_normalize_decision）。
// AI 产出 → WriteIntent::MachineWrite：越界值 drop（不阻断建档），不像 admin 那样 reject。
let gen_stage = match generated.customer_stage.as_deref() {
    Some(v) => apply_admin_dim_validation(
        crate::agent::dimension_registry::validate_dimension_value(
            &state.db, "customer_stage", v, &contact.account_id,
            crate::agent::dimension_registry::WriteIntent::MachineWrite,
        ).await,
    )?,
    None => None,
};
let gen_intent = match generated.intent_level.as_deref() {
    Some(v) => apply_admin_dim_validation(
        crate::agent::dimension_registry::validate_dimension_value(
            &state.db, "intent_level", v, &contact.account_id,
            crate::agent::dimension_registry::WriteIntent::MachineWrite,
        ).await,
    )?,
    None => None,
};
insert_domain_stage_fields(&mut set_doc, gen_stage.as_deref(), gen_intent.as_deref(), true);
```

`apply_admin_dim_validation` + `insert_domain_stage_fields` 均已由 `use super::shared::*`（contacts.rs:26）导入，无需新增 import。`contact.account_id` 三处上下文均可达（enable_agent/update_profile_note 有 `contact`，analyze_contact_profile 有 `contact`）。

**不动**：`insert_domain_stage_fields` 本身、shared.rs、management.rs、decision.rs、`operation_state` 写入（那是状态机 initial 态，非 taxonomy 维度）、`profile_attributes`（自由 KV 非字典维度）。

## 5. 测试设计

三处端点的接线依赖 `build_initial_operation_profile` 真调 LLM + taxonomy cache，需 Docker，无法 hermetic 本地跑。校验内核 `classify_validation` 的四路分支已有密集单测（dimension_registry.rs:280+）、management.rs 的等价路径已有覆盖。

新增一个 **hermetic 纯函数守卫测试**（无需 DB/LLM），锁 M4 的核心不变量——「MachineWrite 下越界 stage 被 drop、合法/alias 被接受/归一」，直接调 `classify_validation`（已 pub(crate)，dimension_registry.rs 测试模块内可用），断言：
- 越界值 + MachineWrite → `DropSilently`（证明三处端点接入后，LLM 臆造 stage 不会落库）；
- alias 值 → `Accept(canonical)`（证明归一生效）；
- 已登记值 → `Accept(原值)`（证明零扰动）。

这些断言若 dimension_registry 已有等价测试则不重复；实现时先 grep 现有测试，仅补缺失的分支。**诚实声明**：三端点真实接线（HTTP → 校验 → 写库）由集成测试覆盖更佳，但需 Docker（`#[ignore]`，留 CI）。本次新增一个 `tests/contact_stage_dict_validation_integration.rs`（`#[ignore]`），构造一个 taxonomy 字典 + mock 一个越界 stage 场景验证不落库——**但** `build_initial_operation_profile` 真调 LLM 无法在集成测试确定性构造 stage 输出。故集成测试改为**直接调三个 handler**（state-only TestApp）验证：当 contact 已存在字典外 stage 时，端点行为正确。若 handler 内部 LLM 调用无法在无 key 环境跑通，则该集成测试标注依赖真 LLM、留 CI real-llm，本地仅靠纯函数守卫 + 代码审查（镜像 management 已验证路径）。

### 验证
- `cargo build --lib` 无 error。
- `cargo test --lib` ≥ 350 passed / 0 failed（基线守住）。
- 禁词 lint 通过（改动用中性词，不涉禁词）。

## 6. 范围边界

- **只增不减**：三处补校验，不改任何其它行为；DEFAULT 销售域标准值行为字节等价。
- **过拟合红线**：不为过测试改字典/阈值；修的是让 contacts 建档路径复用已有的 MachineWrite 校验，与 management + 主决策对齐。
- **多租户**：`validate_dimension_value` 传 `contact.account_id` 作 scope，字典按 account scope 覆盖 global，不跨租户。
- **YAGNI**：不接 objection_type（那是 reaction 派生、非建档路径产物）、不改 relationship_type（AdminDirect 通道，另有 update_operation_profile 端点已校验）。
