# DomainProfile 版本/生效语义分析（CORRECT-2 复核笔记）

> 工作笔记，供审阅决策用。不是正式设计文档。
> 起因：审查发现 CORRECT-2「publish 后新版本即时生效」，我一度判为"realign 修过头、违背人审红线"，
> 经完整核码后**推翻了自己这个判断**。本笔记记录核码事实与最终结论。

## 1. 两个正交维度（这是理解一切的钥匙）

`DomainProfile` 有两个独立标记，回答不同问题：

| 标记 | 作用域 | 回答的问题 | 谁来设 |
| --- | --- | --- | --- |
| `current_version` | `(workspace_id, profile_id)` 血缘内 | 这个 profile 的**哪个版本是最新定稿** | publish/rollout/rollback |
| `is_active` | `workspace_id` 内（跨 profile_id） | 这个 workspace **哪个 profile 在生效** | activate |

- 同血缘至多一条 `current_version=true`。
- 同 workspace 至多一条 `is_active=true`（activate 时把同 ws 其他 profile 的 active 清掉，`domain_profiles.rs:436-443`）。

**运行时加载查询**（`domain_profile.rs:625`）：`{ is_active: true, current_version: true }`——两条件 AND，必须同一行。
含义 = "加载当前生效 profile 的最新定稿版本"。current_version 选血缘内最新稿，is_active 选生效的 profile，缺一不可。

## 2. 与 ops 三表（operation_domain_configs）的对比——为什么 domain_profiles 多一个维度

- **ops 三表**运行时加载只用 `current_version=true` 单维（`decision.rs:759`、`escalation/ledger.rs:24`、
  `principal_decision_channel.rs:147`），**没有 is_active**。publish 即生效。
- **domain_profiles** 故意多加 `is_active`。因为它有一条 ops 三表没有的红线（文件头 `domain_profiles.rs:21`）：
  **"引导层 AI 生成的 profile 必须人审才能 activate"**。所以 publish（定稿）与 activate（人审生效）两步分离。

> 结论：domain_profiles 的双维度是**有意为之**，不是冗余。is_active = "通过人审、允许生效"的闸。

## 3. AI 生成候选走的是独立草稿路径（关键事实）

`guide_profile.rs:190 generate_domain_profile_candidate` 落库时**强制**（`:248-249`）：
```
is_active = false
current_version = false
```
→ AI 生成的候选是**纯草稿**，既不 active 也不 current。要生效必须 publish（定稿）+ activate（人审）两步。

## 4. realign_active_to_current 的三场景（`domain_profiles.rs:461-499`）

逻辑：移动 current_version **之后**调用；若血缘里有 active 行 → 把 active 迁到新 current 行；血缘无 active → noop。

| 调用点 | 操作对象 | 血缘有无 active | realign 行为 | 正确性 |
| --- | --- | --- | --- | --- |
| **rollout** (`:343`) | 已生效 profile 在血缘内换 current 版本 | 有 | active 跟随 current 迁移 | ✅ 必需（否则 active 停旧 current 行→零命中→回落 DEFAULT） |
| **rollback** (`:401`) | 已生效 profile 回退到 prev 版本 | 有 | active 跟随回退 | ✅ 必需（同上） |
| **publish 已生效血缘** (`:296-299`) | 运营编辑已生效行业配置→publish v2 | 有(v1) | active 迁到 v2 | ⚠️ 见下 §5 |
| **publish 纯草稿血缘** (`:296-299`) | publish 一个从未 active 的 AI 草稿血缘 | 无 | **noop** | ✅ 红线守住：仍 is_active=false，须 activate |

## 5. 我一度的误判 & 推翻

**误判**：publish 里的 realign 让"已生效血缘 publish v2"即时生效、跳过 activate → 我说它"违背 AI 生成候选须人审红线、修过头"。

**推翻理由**（核 `guide_profile.rs` 后）：
1. 红线的精确含义是"**AI 生成的候选**须人审"。AI 候选走 §3 的独立草稿路径，落库 is_active=false、**血缘从未 active** → publish 时 realign 命中 `active_in_lineage==0` 分支（`:478`）→ **noop** → 仍须 activate。**红线由"血缘从未 active→noop"独立守住，与 publish 是否 realign 无关。**
2. publish 里 realign 真正影响的是"**运营手动编辑已生效配置**(PUT update)→publish v2"场景。这跟 ops 三表"publish 即生效"一致，且符合"运营改自己已生效的配置就该生效"的直觉。它不是 AI 自动生成，不在红线射程内。
3. 我曾担心"publish 移除 realign 会产生 publish→activate 窗口期回落 DEFAULT"——**恰恰相反**：publish 里的 realign 正是**消除**这个窗口的东西（已生效血缘 publish 后立即切 v2，无窗口）。移除它才会制造窗口。

**结论：publish/rollout/rollback 三处 realign 都是正确的，不违背红线。**

## 6. 唯一真实瑕疵：测试A 与实现的语义漂移（TEST-2 风险的实体）

`domain_profiles.rs:594` 测试 `publish_demotes_current_but_leaves_is_active_untouched`：
- 注释（`:601-602`）："publish 不动 is_active …需后续 activate 版本2 才生效"
- 断言（`:603-604`）：publish 后 v1 仍 active、v2 未 active
- 但它测的是 **sim 函数 `publish_demote_current`（不含 realign）**，不是真 handler

而真 `publish_domain_profile`（`:296-299`）**调了 realign**。所以：
- 真实行为 = realign 后 v2 生效（测试B `realign_migrates_active_to_new_current_when_lineage_was_active` `:631` 才是真行为）
- 测试A 描述的是**没有 realign 的旧设计**，与真实现矛盾
- 两个测试能同时绿，因为测试A 避开了 realign 只测 sim 函数 → **sim 测试掩盖了实现与文档的语义漂移**（正是审查 TEST-2 指出的"simulation 验证的是测试的复刻而非产线代码"的危害实体化）

文件头注释 `domain_profiles.rs:9-14` 同样描述旧两步制（"publish 不动 is_active"），与 publish 实际调 realign 矛盾。

## 7. 建议修法（小而精确，不改 realign 逻辑）

1. **改测试A**：重命名为反映真实语义（如 `publish_on_active_lineage_migrates_active_to_new_version`），断言改为"publish 已生效血缘后 realign 使 v2 active"。或保留 sim 测试但明确它只测 demote 子步、补一个测真 handler 语义的测试。
2. **修文件头注释** `:9-14`：说明 publish 对「已生效血缘」即时切版本生效、对「从未 active 血缘（含 AI 草稿）」保持非 active（须 activate）。
3. **补红线钉死测试**：`AI 草稿血缘（is_active=false）publish 后仍 is_active=false`（断言 realign 的 noop 分支守住人审红线）。
4. realign 三处调用、逻辑本体——**不动**。

## 8. 仍待你定夺的语义边界（唯一真分歧点）

红线是"**AI 生成候选**须人审"（我的理解，realign 不违背）还是"**任何版本变更**须人审"（若是，运营编辑已生效配置 publish 也该回到草稿等 activate，则 publish 应移除 realign 并接受窗口期）？

- 选前者 → §7 修法（保留 realign 改文档/测试）。
- 选后者 → publish 移除 realign，rollout/rollback 保留，接受 publish→activate 窗口期（需讨论：情感陪伴 ws 窗口期回落销售人格的减害）。
