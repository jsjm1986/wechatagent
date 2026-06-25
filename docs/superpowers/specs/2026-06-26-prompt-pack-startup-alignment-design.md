# Prompt Pack 启动对齐设计（spec 为真相 + 修复 evolution 回滚链）

- 日期：2026-06-26
- 来源：ptier 交叉验证审查 F3（CONFIRMED medium）→ brainstorming 收敛
- 状态：首版已实现（commits 62f2a5f..cae6393）→ **2026-06-26 whole-branch 终审追加改动 2bis + 修正改动 3 范围**（兑现 #1 spec 承诺、收窄 domain_config/playbook 范围）→ writing-plans 扩计划
- 关联：`.kiro/specs/universal-test-coverage/ptier-cross-audit-2026-06-26.md`（F3）、`project_config_seed_in_prompts_not_migrations` memory

## 背景与问题

WechatAgent 正研发期，无生产数据。`ensure_prompt_pack_v2`（src/prompts.rs:85）在每次启动时种 prompt pack：
- lookup 库里有无 `prompt_pack_version == PROMPT_PACK_VERSION`（常量，当前 v12）的模板
- `Ok(Some)` → `delete_redundant`（只删 archived）+ `ensure_missing`（只补缺失 key，**不更新已存在模板**）
- `Ok(None)`（旧版本库 或 全新空库）→ `reset_prompt_pack_v2`（对 4 集合无过滤 `delete_many` 全删重种）
- `Err` → 兜底也走 reset

### 真实设计瑕疵（研发期视角，已交叉验证）

1. **改 spec 不 bump 版本号不生效**（核心痛点）：`ensure_missing` 对已存在 key 直接 `continue`、绝不比对内容（prompts.rs:158-160）。要让改动的 prompt 生效，唯一路径是 bump 版本号 → 走 `Ok(None)` → 破坏性全量 reset。
2. **`Ok(None)` 混淆空库与旧版本库**：两者都没有当前版本模板，落同一 reset 分支。
3. **生效判断基于版本号字符串，与内容真实状态脱节**：lookup 只问"有没有 v12"，不问"内容是否和代码 spec 一致"。

### 方法论定调（用户决策）

- 研发期无生产数据，"保护运营在线编辑"是伪需求，不做归档恢复 UI。
- **spec 为真相**：代码里的 spec 是唯一真相，启动时逐 key 比对、不一致就用 spec 覆盖。改 spec 重启必生效，不靠版本号。
- **治本**：一并修复 evolution 回滚链的 status 缺陷（见下），不留"回滚静默失效"隐患。

## 关键约束：与 evolution 灰度机制的耦合（最后核验抓出，必须遵守）

系统有 evolution 自动演化 + A/B 灰度机制，启动对齐**绝不能破坏**它：

- `load_prompt`（prompts.rs:313）取 `status="active"`；`load_prompt_for_contact`（prompts.rs:353）按 contact hash 分流**多条 active** 做 A/B。
- `release_prompt`（release.rs:234-312）：取 `current_version=true` 的行作锚点（首次即 system 行），旧行留 `active + current_version=false`，新行 `active + current_version=true + seeded_by="evolution_release" + previous_version=旧version`。**两条同时 active = A/B 分桶基础**。
- `rollback_prompt`（release.rs:598-619）：把 `previous_version` 那条重新 `$set current_version=true`，**但不恢复 status**（已亲核确认）。
- `ensure_evolution_prompt_pack_v1` 另种 `evolution_critic_v1`，`seeded_by="system_evolution_v1"`（不在 prompt_specs，前缀近似 system 但不等于）。

**耦合风险（若启动对齐粗暴归档 system 行）**：
- 破坏回滚：被归档的 previous_version 行回滚后变 `current=true + status=archived` → load 只取 active → 静默回落 default，回滚失效无报错。
- 折叠在飞 A/B：system 实验臂被 archive，A/B 单边塌缩。
- 双 current 不变量破坏：重种新 system 行带 current=true，与 evolution 版 current=true 并存，下次 release 锚点错乱。

## 设计

### 改动 1：修复 rollback 链 status 缺陷（治本，src/evolution/release.rs:598-619）

rollback 第 2 步把 `previous_version` 行置 current 时，**一并 `$set status: "active"`**：
```
"$set": { "current_version": true, "status": "active", "updated_at": now }
```
效果：无论该行此前是否被归档，回滚都能真正生效。rollback 自我修复 status，启动对齐的归档不再能破坏回滚链。这是 status 缺陷的根治，独立于启动对齐价值（现状下若有人手动 archive 过旧版，回滚也会失效）。

### 改动 2：ensure_prompt_pack_v2 改为 spec 为真相的启动对齐（src/prompts.rs）

**复用项目既有模式**：domain_configs 早已用 `is_refreshable_policy_seeded_by`（admin_ops_versions.rs:176）解决同一问题——白名单区分"机器派生可刷新"vs"运营/演化手工行保留"。prompt_templates 的启动对齐**照搬这套模式**，不另造轮子。新增 `is_refreshable_prompt_seeded_by(seeded_by) -> bool`：
- `Some("system")` → 可刷新（系统种子脉络）
- `Some("evolution_release")` / `Some("manual")` / `Some("system_evolution_v1")` / 其它任意值 → **不可刷新，保留**
- `None` → **不可刷新，保留**（保守：prompt_templates 历史种子虽都写了 system，但不照搬 domain_configs 的"None→可刷新"，避免任何未打标行被误刷）

把"版本号 lookup → 二分（reset / ensure_missing）"改为**逐 key 内容对齐**。对 `prompt_specs()` 每个 spec：

1. **守卫：若该 key 存在任何 `seeded_by="evolution_release"` 的行（在飞 A/B 或 release 链）→ 跳过该 key + 写告警事件**，把灰度链交 admin 手动收口。绝不在有 evolution 链的 key 上动手。
2. 查该 key 下"可刷新"（`is_refreshable_prompt_seeded_by` 为真）且 `current_version=true` 的行。
3. 比对内容（见下方 normalize）：
   - **一致** → 跳过。
   - **不一致 / 不存在** → 归档旧可刷新行（`status="archived"`，非 delete）+ 种入 spec 新行（`active`, `current_version=true`, `seeded_by="system"`）。
4. 不可刷新行（manual / evolution）一律不动。

**内容比对必须 normalize（核验 A3 抓出，否则炸）**：spec 是 Windows 工作树的 `r#"..."#` 多行串，git autocrlf 跨构建 LF↔CRLF 互转会让编译进二进制的字节与 DB 存的不同 → 裸 `==` 每次重启都判"不一致"→ 版本号无限膨胀 + A/B 轮换抖动。**定方案：比对前把两侧统一换行符 `\r\n`→`\n`（不额外 trim 行尾，避免吞掉 spec 有意义的尾随空格），normalize 后字符串相等即视为一致。** 不引入 hash（多一层无收益）。

**归档而非删除**：所有"替换"用 `status="archived"`（可回溯，研发期零成本留路）。`delete_redundant`（删 archived）逻辑保留不变。

**LRU 失效**：对齐若产生任何写入（归档+重种），沿用 main.rs:193 现有 `state.prompt_pack_version.fetch_add(1)` 路径失效缓存。注意 `state.prompt_pack_version`（运行时 AtomicU64 LRU 计数器）与 `PROMPT_PACK_VERSION`（种子包版本字符串常量）是两个不同的东西，勿混。

### 改动 2bis：ensure_prompt_pack_v2 接入结构——版本盲三分支 → 空库分流（2026-06-26 终审 #1 根因修复）

**问题（whole-branch 终审 #1 抓出）**：原 spec 改动 2 只描述了 `align_prompt_specs` 的内部对齐逻辑，**没说它接入 `ensure_prompt_pack_v2` 的哪个分支**。首版实现据此只把 align 挂在 `Ok(None)`（版本号不匹配=bump 后首启）分支，`Ok(Some)`（版本号匹配=**日常改 spec 最常见场景**）仍走 `ensure_missing_prompt_templates`——它对已存在 key 直接 `continue`、**绝不比对内容**。结果：日常改 `prompt_specs()` 内容、不 bump `PROMPT_PACK_VERSION`，重启仍不生效——F3 点名的核心痛点"改 spec 不 bump 不生效"**未被消除**，与本 spec「方法论定调」白纸黑字的"改 spec 重启必生效，不靠版本号"直接矛盾。首版只修好了"bump 版本时改非破坏性 align 而非破坏性 reset"（真价值，但比承诺窄）。

**修复：把"版本号三分支"重构为"空库分流"。** `PROMPT_PACK_VERSION` 常量**降级为非生效闸**——不再用它判断"内容是否生效"，只保留 stamp 进新种行做溯源/可观测。生效判定完全交给内容比对（`align_prompt_specs` 内部 normalize 后逐 key 比）。

原结构（版本盲，问题所在）：
```
lookup(prompt_pack_version == PROMPT_PACK_VERSION 的 active/draft 行)
  Ok(Some) → delete_redundant + ensure_missing   // 日常路径，内容盲，改 spec 不生效
  Ok(None) → reset_prompt_pack_v2 (全量重种)      // bump 路径
  Err      → warn 事件 + reset 兜底
```

新结构（空库分流，spec 为真相）：
```
判定 any_existing = 库里有无任何 prompt_templates 行（不限版本/状态）
  空库(!any_existing)      → reset_prompt_pack_v2  // 全新库首次种四集合
  非空库(any_existing)     → delete_redundant(GC archived) + align_prompt_specs(逐 key 内容对齐)
  查询 Err                 → 保留现有 warn 事件 + reset 兜底（不变）
```

**关键性质**：
1. **改 spec 不 bump 也生效**：日常改 `prompt_specs()` 内容→重启→非空库→align→内容不一致→归档旧行+种新行。真正兑现"不靠版本号"。
2. **GC 不停摆**：`delete_redundant`（删 archived 行）移到非空库路径**每次启动都跑**。修复终审 Minor #3（原结构里纯 version bump 无内容漂移会卡在 `Ok(None)`/align 路径、`Ok(Some)` 的 GC 永不执行→archived 行无限堆积）。
3. **空库判定取代版本 lookup**：`align_prompt_specs` 内部已有 `any_existing` 判定（首版 Task 3 引入），新结构把它提到 `ensure_prompt_pack_v2` 顶层做分流依据；`align_prompt_specs` 自身逻辑不变（evolution 守卫、归档非删、normalize、收敛、不动 manual 全部复用，零改动）。
4. **收敛**：align 种的新行 stamp 当前 `PROMPT_PACK_VERSION`；但因为生效判定已不看版本号，纯 bump 无漂移的场景也不再卡——每次启动 align 跑一遍（幂等、内容一致即 no-op），GC 顺带跑。
5. **`ensure_missing_prompt_templates` 去留**：新结构非空库路径用 align 取代 ensure_missing（align 覆盖"不存在则种入"语义，含 ensure_missing 的补缺能力）。ensure_missing 若无其它调用方则一并删除（实现时 grep 确认），避免死代码。



### 改动 3：四集合范围界定

| 集合 | 标记现状 | 本期处理 |
|---|---|---|
| prompt_templates | 有 seeded_by(system/manual)+created_by | **完整启动对齐**（改动 2 + 改动 2bis） |
| operation_domain_configs | **已有 seeded_by**（Phase E 灰度四元组）+ version 四元组 + status + A/B 灰度 | **本期不纳入**（理据见下）。结构齐备，但 admin 写入模型与"物理隔离式对齐"不兼容 |
| operation_playbooks | 有 created_by(system/manual)+is_default，**无 status / 无 version 四元组 / 无 seeded_by** | **本期不纳入**（无承载结构，同 agent_souls）。运行时 is_default 单条加载、无 A/B/灰度，无 status 字段无法归档 |
| agent_souls | **无 seeded_by、无 version 四元组、无 archive**，publish 是 delete_many 物理删 | **本期不纳入**（无承载结构）。仅加 `seeded_by` 字段备用，但启动对齐逻辑暂不覆盖 souls，显式标注待后续给 AgentSoul 补版本化机制后再做 |

**为什么 domain_config 本期不纳入（深核结论，2026-06-26 whole-branch 终审后追加）**：domain_config 看似结构齐备（有 seeded_by + 四元组 + status），但与 prompt_templates 有本质差异——
- prompt_templates 的 admin 编辑落到**独立的 manual 行**（物理隔离），系统种子行始终是干净的 `seeded_by="system"`，对齐只碰 system 行天然安全。
- domain_config 的 admin 编辑（含 `runtime_parameters` 阈值，经 `routes/domains.rs::update_operation_domain` 原地 `$set` 到 current 行）**不翻 seeded_by**——被 admin 调过的 current 行 seeded_by 仍是 `"system"`。无法用 seeded_by 区分"纯 spec 系统行"与"被 admin 脏改的系统行"，任何全字段对齐都会把 admin 调的阈值冲回 spec 默认。
- 且 domain_config 已有 `publish/rollout/rollback` 完整灰度发布路径作为"改种子内容→生效"的机制，痛点远不如 prompt（prompt 无别的生效路径）。
- 彻底纳入需先改 admin 写入语义（4 个 handler 让 admin 编辑翻 seeded_by→manual），blast radius 大、影响 publish/rollout 锚点逻辑——留未来专项。

**为什么 playbook / souls 本期不纳入**：两者都无 status 字段、无 version 四元组，无法承载"归档旧行而非删除"的对齐语义。强行做会退化为物理删（破坏可回溯），故与 agent_souls 一致显式推迟，待补版本化结构后再做。本期仅 prompt_templates 走完整对齐。

### 改动 4：AgentSoul 加 seeded_by 字段（仅备用，核验 A2 实测）

`models.rs` AgentSoul 加 `#[serde(default)] pub seeded_by: Option<String>`。**必补 2 个构造点**（否则 E0063，全仓 grep 实测仅这两处、tests 零构造）：
- src/prompts.rs:213（reset 种子写 `Some("system")`）
- src/routes/souls.rs:87（管理端 create 写 `Some("manual")`）

OperationDomainConfig **不动**（已有 seeded_by）。

## 不变量（必须守住）

- 启动对齐后，任一 key 下 `current_version=true` 的行有且仅有一条（不制造双 current）。
- evolution_release / system_evolution_v1 / manual 脉络的行永不被启动对齐归档。
- 有 evolution 灰度链的 key 被跳过 + 告警，不被单边折叠。
- spec 没变时启动对齐幂等（不产生新行、不翻版本）。

## 测试（呼应 F2 教训：确定性、无网络、进 baseline 门）

复用现成测试 DB helper，不带 `#[ignore]`：
1. spec 内容变 → 对齐后 DB 取到新内容(active) + 旧 system 行 archived。
2. spec 没变 → 对齐幂等，无新行、版本号不涨。
3. **normalize**：DB 存 CRLF、spec 是 LF（或反之）→ 视为一致，不误归档（防版本膨胀）。
4. **evolution 边界**：某 key 有 evolution_release 行 → 对齐跳过该 key，evolution 行原样保留、未被归档（守住关键边界）。
5. **rollback 修复**：先 archive 一条 previous_version 行，rollback 后该行 `current=true + status=active`，load_prompt 取到它（验证治本）。
6. manual 脉络行不被对齐归档。
7. **`is_refreshable_prompt_seeded_by` 谓词边界**（纯函数单测，进 lib 基线门）：`Some("system")`→true；`Some("manual")`/`Some("evolution_release")`/`Some("system_evolution_v1")`/`None`/其它→false。守住"只刷系统种子、保留一切其它脉络"的白名单语义（正向匹配，不用 `!=` 否定）。

**改动 2bis 新增测试（终审 #1 回归，证明不再版本盲）**：
8. **版本号匹配但内容漂移仍对齐（#1 核心回归）**：DB 已是当前 `PROMPT_PACK_VERSION`（即原结构会走 `Ok(Some)` 的场景）、但某 system 行内容被改脏 → 重跑 `ensure_prompt_pack_v2` → 脏行被对齐回 spec(新行 active) + 脏行归档。**这是与首版测试 #1 的本质区别**：首版测试在 setup 里把 `prompt_pack_version` 改旧值制造 `Ok(None)` 才触发 align；本测试**不改版本号**，直接验证"版本号匹配时也对齐"——若实现仍是版本盲（align 只挂 Ok(None)），本测试必失败。
9. **GC 在非空库路径每次跑**：预置一条 `status="archived"` 的孤立行 → 重跑 `ensure_prompt_pack_v2`（不改版本号，走非空库路径）→ 该 archived 行被 `delete_redundant` 清除。验证 GC 不再绑死在已删除的 `Ok(Some)` 分支。

## 范围与 YAGNI

- 本期只做 **prompt_templates 完整启动对齐**（改动 2 + 改动 2bis）+ rollback status 修复（改动 1）+ AgentSoul 加 seeded_by 字段（改动 4，仅备用）。
- **domain_config / playbook / souls 启动对齐显式不纳入本期**（理据见改动 3）：domain_config 因 admin 原地编辑不翻 seeded_by 与物理隔离式对齐不兼容、且已有 publish/rollout 灰度发布作为生效机制；playbook/souls 无 status/version 承载结构无法归档。三者均留未来专项。
- souls 完整版本化（status/version/archive + 对齐）待后续专项。
- 不做归档恢复 UI（研发期无生产数据，伪需求）。
