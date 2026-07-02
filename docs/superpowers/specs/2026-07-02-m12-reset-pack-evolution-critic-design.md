# M12 reset-system-pack 遗漏 evolution critic prompt 重种修复设计

> 日期：2026-07-02
> 分支：`fix/m12-reset-pack-evolution-critic`（从 origin/main 11acc3a 切，含 M1 #90 / M4 #91 / M10 #94）
> 来源：终极审判审计 M12（UPHELD Medium）

## 1. 漏洞描述（对最新代码 100% 亲验）

管理员显式维护动作 `POST /prompt-templates/reset-system-pack`（`reset_system_prompt_pack`，prompt_templates.rs:340）调 `prompts::reset_prompt_pack_v2`（prompts.rs:324）。该函数：

1. **无条件 `delete_many`** 本 workspace 的**全部** `prompt_templates`（prompts.rs:332-334，无 status/agent_kind 过滤 → 连 `evolution_critic_v1` 一起删）；
2. 只从 `prompt_specs()`（prompts.rs:1047，业务 Reply Agent pack）逐条重种。

而演化器 Critic Agent 的 prompt `evolution_critic_v1` 由**另一个独立函数** `ensure_evolution_prompt_pack_v1`（prompts.rs:2271）种，来自 `evolution_prompt_specs()`（prompts.rs:2326），且该函数**只在进程启动时调一次**（main.rs:191）。

### 后果链（已逐行核实）

- reset 后 `evolution_critic_v1` 被删、不重种 → DB 里该 key 消失；
- 演化器 Critic 消费方 `prompt_critic.rs:118` 调 `load_prompt(db, ws, "evolution_critic_v1")`；
- `load_prompt`（prompts.rs:447）DB 未命中 → 回落 `default_prompt_content`（prompts.rs:2255），后者**只查 `prompt_specs()`**（不含 `evolution_critic_v1`）→ 返 `None` → `load_prompt` 抛 `AppError::NotFound`；
- `prompt_critic.rs:119` 把它 `map_err` 成 `EvolutionError::Internal("load_prompt(evolution_critic_v1) failed: ...")`；
- 结果：**管理员点一次"重置系统提示词包"后，整个 prompt 演化循环（W2 Critic 产候选）持续报错、直到进程重启**才由 main.rs:191 重种恢复。

### 为什么这是真 bug 而非取舍

`reset_prompt_pack_v2` 的注释与设计意图是"物理销毁并重种业务 pack"（显式维护动作），**从未**声明要连带废掉演化器 Critic。`evolution_critic_v1` 明确是**不变量**（`PROMPT_EVOLUTION_FORBIDDEN_KEYS`，禁止被演化循环重写），启动幂等种入——它本该在 reset 后继续存在。reset 把它误删且不补种，是"全量 delete + 部分 reseed"的覆盖面不匹配缺陷，不是有意设计。

## 2. 根因

`reset_prompt_pack_v2` 的 `delete_many` 覆盖面（全部 prompt_templates）> 其 reseed 覆盖面（仅业务 `prompt_specs()`）。演化 Critic pack 是后于 reset 逻辑加入的独立 pack（`ensure_evolution_prompt_pack_v1`），reset 落地时未同步补种它。

## 3. 方案

### 方案 A（选定）：reset 末尾补调 `ensure_evolution_prompt_pack_v1`

在 `reset_prompt_pack_v2` 重种业务 pack、domain configs 之后、`Ok(())` 之前，补一行：

```rust
// M12：reset 无条件删了全部 prompt_templates（含 evolution_critic_v1），业务 pack
// 由上面 prompt_specs() 重种，但演化器 Critic pack 是独立 pack、只在启动时种。
// 这里同 workspace 补种回来，避免 reset 后演化循环因 critic prompt 缺失持续报错到重启。
// ensure_evolution_prompt_pack_v1 幂等（已存在则跳过），此处 critic 已被删故会重插。
ensure_evolution_prompt_pack_v1(db, workspace_id).await?;
```

`ensure_evolution_prompt_pack_v1` 本就幂等（prompt_templates.rs:2273 先 find_one 存在则 continue）。reset 刚删完 critic，故此调用会重插一条 `current_version=true` 的 `evolution_critic_v1`——与启动时的种子字节等价。

### 为什么不改 `delete_many` 加过滤（否决方案 B）

也可以让 reset 的 `delete_many` 加 `agent_kind != "evolution"` 过滤（不删 critic）。否决原因：
1. reset 的语义是"把业务 pack 彻底重置到出厂"，其中"删干净再重种"对业务 pack 是刻意的（清运营手改版本）；给 delete 加过滤会让"reset 后 critic 保留旧内容"，而**重种**语义（拿到最新 `evolution_prompt_specs()` 版本）更符合"重置到出厂"的一致预期；
2. 方案 A 复用现成幂等函数、零新逻辑，且与 main.rs 启动路径同源（唯一真相源 `ensure_evolution_prompt_pack_v1`），不引入第二处"哪些 key 属于 evolution"的判断（易漂移）。

## 4. 核心改动

落点：`src/prompts.rs` `reset_prompt_pack_v2`，在 `default_domain_configs` 循环之后、`Ok(())` 之前补一行 `ensure_evolution_prompt_pack_v1(db, workspace_id).await?;`。`ensure_evolution_prompt_pack_v1` 已是同模块 `pub async fn`，无需 import。

**不动**：`delete_many` 逻辑、业务 pack reseed、`ensure_evolution_prompt_pack_v1` 本身、启动路径、route handler、LRU version bump（route handler 已 bump `prompt_pack_version`，critic prompt 不在业务 LRU 白名单内、不受影响）。

## 5. 测试设计

`reset_prompt_pack_v2` 需真实 Mongo（delete_many + insert），是 `#[ignore]` 集成测试范畴（Docker）。新增一个集成测试到 `tests/`：
- seed：启动式先 `ensure_prompt_pack_v2` + `ensure_evolution_prompt_pack_v1`，断言 `evolution_critic_v1` 存在；
- act：调 `reset_prompt_pack_v2`；
- assert：`evolution_critic_v1` **仍存在**（`current_version=true`、`status=active`）——钉死 reset 不再孤儿化 critic。

该测试 `#[ignore]`（需 Docker/testcontainers），本地不跑、留 CI Integration job。**诚实声明**：本地只能 `cargo build --lib` + `cargo test --lib`（基线守住 + 编译通过）验证不回归；reset 的真实 DB 行为由该集成测试在 CI 覆盖。

### 验证
- `cargo build --lib` 无 error。
- `cargo test --lib` ≥ 350 passed / 0 failed（基线守住）。
- 新增 `tests/reset_pack_preserves_evolution_critic_integration.rs`（`#[ignore]`）编译通过（`cargo test --no-run` 或 CI）。
- 禁词 lint 通过。

## 6. 范围边界

- **只增不减**：reset 多补种一次 critic，其它行为不变；未删 critic 的场景下（正常运行期）无影响。
- **过拟合红线**：不改 reset 的 delete/业务 reseed 语义，不改 critic pack 内容；修的是让 reset 的 reseed 覆盖面与 delete 覆盖面对齐（复用现成幂等 `ensure_evolution_prompt_pack_v1`）。
- **多租户**：`ensure_evolution_prompt_pack_v1(db, workspace_id)` 用 reset 传入的同一 `workspace_id`，不跨租户。
