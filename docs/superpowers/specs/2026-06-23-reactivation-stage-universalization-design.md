# reactivation 目标 stage 通用化 设计

> 2026-06-23 全仓审查 #3「scan_reactivation 销售 stage 焊死」修复设计。
> 关联：[[project_codebase_audit_2026_06_23]]、[[project_universalization_residuals]]、[[project_agent_first_no_keyword_filters]]。

## 问题（实证）

`reactivation_candidate_filter`（`src/planner/mod.rs:1935`）把销售行业专有的 stage 字面量
`"dormant_reactivation"` 硬编码进 MongoDB 端预筛：

```rust
pub(crate) fn reactivation_candidate_filter(workspace_id: &str, account_id: &str) -> Document {
    doc! {
        // ...managed + 非冷却 $or...
        "domain_attributes.customer_stage": "dormant_reactivation",
    }
}
```

非销售域的 profile 状态机里**没有**这个 stage（情感陪伴/同行运营各有自己的"流失/沉默"语义
stage），于是 DB 预筛 `customer_stage == "dormant_reactivation"` 恒选 0 条 → `scan_reactivation`
游标空转 → 再激活扫描器**静默失效**，没有任何报错。

对照：隔壁 `stage_stagnation_candidate_filter`（`:995`）早已接 `PlannerStageConfig` + 用
`effective_terminal_stages()`（`:875`）做字典派生的 `$nin` 预筛，是通用化的。reactivation
是**漏网的那一个**。

### agent-first 守住

`dormant_reactivation` 是 **Reply Agent 语义判定**客户已流失/沉默后输出到
`decision.customer_stage` 的（`src/agent/domain_signals.rs` 写入点，经 C2 同步到
`domain_attributes.customer_stage`）——**没有规则写入点、没有关键词匹配**。本设计不动这个语义
判定，只把"哪个 stage 算 reactivation 目标"从硬编码字面量升级为**字典可声明**，与
[[project_agent_first_no_keyword_filters]] 立场一致。

## 方案：字典 `is_reactivation_target` 标记（与 `is_terminal` 完全对称）

加一个与现有 `is_terminal` 链路逐处对称的 `is_reactivation_target` 维度标记，让 profile 通过
`system_taxonomies` 字典声明"哪些 customer_stage 取值是再激活目标"。`PlannerStageConfig` 派生出
`reactivation_stages` 集合，`reactivation_candidate_filter` 接 config 用 `$in` 预筛。

为什么不复用 `is_terminal` / `effective_terminal_stages`：`dormant_reactivation` 在销售域
**既是终态**（`TERMINAL_STAGES` 含它，扫描漏斗不再主动推进它）**又是再激活目标**（低频唤醒）。
这是两个正交语义——终态的其它两个成员 `customer_success` / `cooldown` 不是再激活目标。所以必须
新增独立标记，不能借 `is_terminal` 表达。

### DEFAULT 销售域字节等价

字典里仅 `dormant_reactivation` 标 `is_reactivation_target=true` → `reactivation_stages =
{"dormant_reactivation"}` → 预筛 `customer_stage: { "$in": ["dormant_reactivation"] }`，与原
`customer_stage: "dormant_reactivation"` 查询**等价**（单元素 `$in` 与 `==` 同义）。空字典
（未种 / 旧库）回落写死 `["dormant_reactivation"]`，同样等价。销售域行为零变化。

## 5 处对称改动

改动面与 `is_terminal` 的现有链路逐处对应：

| # | 文件:行 | `is_terminal` 现状（范本） | 新增 `is_reactivation_target` |
| - | - | - | - |
| 1 | `src/models.rs:2553` `TaxonomyValue` | `pub is_terminal: bool`（:2571） | 加 `#[serde(default)] pub is_reactivation_target: bool` |
| 2 | `src/agent/taxonomy.rs` | cache struct `is_terminal`（:89/:146）+ `dimension_value_weights` 返回三元组带 `is_terminal`（:271） | cache 带 `is_reactivation_target`；`dimension_value_weights` 扩为四元组 |
| 3 | `src/planner/mod.rs:823` `PlannerStageConfig` | `terminal_stages: HashSet` + `effective_terminal_stages()`（:875）+ build 填充（:909-918） | 加 `reactivation_stages: HashSet` + `effective_reactivation_stages()` + build 填充 |
| 4 | `src/planner/mod.rs:1935` `reactivation_candidate_filter` | （未通用化） | 签名加 `stage_config: &PlannerStageConfig`，`==` 改 `$in effective_reactivation_stages()` |
| 5 | `src/db/migrations/m006_taxonomy_seed.rs:82` 7→8 元组 | 末列 `is_terminal` | 加一列 `is_reactivation_target`，仅 `dormant_reactivation` 行 =true |

### 1. `TaxonomyValue` 加字段（models.rs:2553 区）

```rust
    #[serde(default)]
    pub is_terminal: bool,
    /// universal-domain-adaptation #3：该取值是否为「再激活目标」stage（profile 可声明）。
    /// 与 is_terminal 正交：dormant_reactivation 既终态又再激活目标，customer_success/cooldown
    /// 是终态但非再激活目标。serde default=false → 旧 BSON 文档/未声明的维度向后兼容。
    #[serde(default)]
    pub is_reactivation_target: bool,
```

`models.rs:4878-4879` 的内联构造（测试 fixture）需补 `is_reactivation_target: false`。

### 2. taxonomy.rs 派生（:86/:145/:258/:271 等）

cache struct（`:86` 区）加 `is_reactivation_target: bool` 字段，各构造点
（`:145`/`:457`/`:583`/`:612`/`:641`）补一行 `is_reactivation_target: ...`（来自 entry / false）。

`dimension_value_weights`（`:258`）返回值从三元组 `(String, Option<i32>, bool)` 扩为四元组
`(String, Option<i32>, bool, bool)`，第四位 = `is_reactivation_target`（`:271` push 处）。
**调用点同步**：`build_planner_stage_config`（planner:909/919）的两处 `for (id, weight,
is_terminal)` 解构改四元组。intent_level 那次循环第四位用 `_` 忽略。

> 备选：另开并行函数 `dimension_reactivation_targets()` 不动 `dimension_value_weights` 签名。
> 但四元组改动面更小（单一遍历、单一缓存读），且与 is_terminal 同源一次取出，避免重复
> 遍历。**采用四元组**。

### 3. PlannerStageConfig（planner:823 区）

```rust
pub(crate) struct PlannerStageConfig {
    stage_weights: HashMap<String, i32>,
    intent_weights: HashMap<String, i32>,
    terminal_stages: HashSet<String>,
    /// 再激活目标 stage canonical id 集合（is_reactivation_target=true）。
    reactivation_stages: HashSet<String>,
    stagnation_dimension: String,
}
```

`Default`（:834）+ build 内构造（:903）补 `reactivation_stages: HashSet::new()`。新增方法，与
`effective_terminal_stages`（:875）逐字对称：

```rust
    /// 有效再激活目标集合（供 MongoDB 端 `$in` 预筛）：字典非空用字典，否则回落写死
    /// ["dormant_reactivation"]。与 reactivation_candidate_filter 同源。
    fn effective_reactivation_stages(&self) -> Vec<String> {
        if self.reactivation_stages.is_empty() {
            vec!["dormant_reactivation".to_string()]
        } else {
            self.reactivation_stages.iter().cloned().collect()
        }
    }
```

build 的 customer_stage 循环（:909-918）内补：

```rust
        if is_reactivation_target {
            config.reactivation_stages.insert(id.clone());
        }
```

（注意 `id` 此前在 `if is_terminal { config.terminal_stages.insert(id); }` 被 move，需改
`insert(id.clone())` 或调整顺序——实现时择一，保证两个 insert 都能用 id。）

### 4. reactivation_candidate_filter 接 config（planner:1935）

```rust
pub(crate) fn reactivation_candidate_filter(
    workspace_id: &str,
    account_id: &str,
    stage_config: &PlannerStageConfig,
) -> Document {
    doc! {
        // ...managed + 非冷却 $or 原样不动...
        "domain_attributes.customer_stage": {
            "$in": stage_config.effective_reactivation_stages()
        },
    }
}
```

调用点 `scan_reactivation`（:1974）：profile 已在 :1961 加载，补一行
`let stage_config = build_planner_stage_config(state, &account_id, &profile).await;`，filter 调用
改 `reactivation_candidate_filter(&workspace_id, &account_id, &stage_config)`。

> 范本一致性：`scan_stage_stagnation` 同样先 `build_planner_stage_config` 再传给
> `stage_stagnation_candidate_filter`。reactivation 照此对齐。

### 5. m006 seed 标记（m006_taxonomy_seed.rs:82）

元组类型 `&[(&str, &str, &str, &[&str], i32, bool)]` → 末尾加一列 `bool`（8 元组）：
`(&str, &str, &str, &[&str], i32, bool, bool)`，第 7 位 = `is_reactivation_target`。9 行中**仅**
`dormant_reactivation`（:148）该列 =true，其余 8 行 =false。解构
`for (id, display, desc, aliases, weight, terminal) in` → 加 `reactivation_target`，
`TaxonomyValue { ... is_reactivation_target: *reactivation_target }`。

## 测试影响（须同步，不可破基线）

1. **planner:3410** `reactivation_candidate_filter_includes_dormant_stage`：原断言 filter 的
   `customer_stage == "dormant_reactivation"`。改 `$in` 后调用需传 `&PlannerStageConfig::default()`，
   断言改为 filter 含 `customer_stage: { $in: ["dormant_reactivation"] }`（DEFAULT 回落值）。
2. **m006:420-433** H6 护栏 `customer_stage_*_matches_planner`：现断言 is_terminal 与
   TERMINAL_STAGES 一致——不动。**新增**对称护栏：断言**仅** `dormant_reactivation` 的
   `is_reactivation_target=true`（与 `effective_reactivation_stages` DEFAULT 回落值一致），防止
   未来字典与回落漂移。
3. **新增** `effective_reactivation_stages` 单测：空集合回落 `["dormant_reactivation"]`；非空集合
   返字典值（与 `effective_terminal_stages` 现有测试对称，若有）。
4. 全 workspace 编译同步：`dimension_value_weights` 四元组改动会让所有解构点编译失败——grep
   确认仅 `build_planner_stage_config` 两处 + 测试调用，逐处补第四位。

## 约束

- `cargo test --lib` ≥350 passed / 0 failed；四 PBT 累计 ≥33 / 0 不回归。
- 后端编译用 `RUSTFLAGS="-D warnings" cargo check --tests`（磁盘受限，lib 测试断言留 CI 基线门）。
- 向后兼容：`#[serde(default)]` 保证旧 BSON / 旧 LLM 输出反序列化不破。
- DEFAULT 销售域**字节等价**：单元素 `$in` ≡ `==`，空字典回落 = `["dormant_reactivation"]`。
- 不引入关键词匹配、不动 customer_stage 的 LLM 语义判定（agent-first）。
- 提交需用户显式批准；精确 `git add` 排除并行产物。

## 风险与回滚

- 风险：`dimension_value_weights` 签名改四元组是**编译期强约束**——漏改任一解构点直接编译失败，
  不会静默。这是好事（不会漏）。
- 回滚：纯加法 + serde default，`git revert` 即恢复；字典字段缺省 false 不影响旧逻辑。
- 非销售域启用：在自己的 customer_stage 字典里给对应"沉默/流失"语义 stage 标
  `is_reactivation_target=true` 即可，无需改代码。
