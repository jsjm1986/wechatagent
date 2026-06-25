# Prompt Pack 启动对齐设计（spec 为真相 + 修复 evolution 回滚链）

- 日期：2026-06-26
- 来源：ptier 交叉验证审查 F3（CONFIRMED medium）→ brainstorming 收敛
- 状态：设计待复核 → writing-plans
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

### 改动 3：四集合范围界定

| 集合 | 标记现状 | 本期处理 |
|---|---|---|
| prompt_templates | 有 seeded_by(system/manual)+created_by | **完整启动对齐**（改动 2） |
| operation_domain_configs | **已有 seeded_by**（Phase E 灰度四元组）+ **已有 `is_refreshable_policy_seeded_by` 安全刷新逻辑**（admin_ops_versions.rs:176） | 本期沿用其既有"可刷新判别"模式做对齐；manual/statemachine_publish 等手工行天然被跳过，无需另确认灰度脉络 |
| operation_playbooks | 有 created_by(system/manual)+is_default | 按 created_by 圈定系统脉络对齐 |
| agent_souls | **无 seeded_by、无 version 四元组、无 archive**，publish 是 delete_many 物理删 | **本期不纳入**（无承载结构）。仅加 `seeded_by` 字段备用，但启动对齐逻辑暂不覆盖 souls，显式标注待后续给 AgentSoul 补版本化机制后再做 |

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

## 范围与 YAGNI

- 本期只做 prompt_templates 完整对齐 + playbooks/domain_configs 对齐 + rollback 修复 + AgentSoul 加字段。
- souls 完整版本化（status/version/archive + 对齐）**显式不纳入本期**，待后续专项。
- 不做归档恢复 UI（研发期无生产数据，伪需求）。
