# 批D家族② 修复设计：决策请示通道（KD-05 骚扰门口径 / KD-06 孤儿 pending / KD-02 领导泄漏词表裁决）

> 批 D 家族②（P2）。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KD-02(:844)/KD-05(:895)/KD-06(:905)。全部行号亲验于 origin/main 36dfda8。

## 概述

三条 Medium 同处决策请示通道（AI 遇超职权事项向幕后领导请示、拿回结论转述给客户）。实际落地**两条代码修复 + 一条经裁决不修**：

- **KD-02（经裁决不修）**：不加字符级"领导泄漏"词表。
- **KD-05（修）**：骚扰门统计口径漂移——加真实"最近推送时刻"字段。
- **KD-06（修）**：孤儿 pending——position 未命中回落链首。

## KD-02：经裁决不加字符级词表（交 LLM + review，与 PR #185 一致）

**台账诉求**：给 relay reply_text + holding_reply 加"领导泄漏"词表（领导/上级/老板/请示了/上面批/汇报了），命中 fail-closed。理由是"客户永不知道有领导"红线与"无人工接管"红线平级，后者有字符级 lint（check-no-human-takeover + evolution::lint），前者无。

**亲验的结构事实（CONFIRMED）**：
- `relay_output_leaks_internal_payload`（escalation/logic.rs:211-216）只检 4 个载荷标记（`__PRINCIPAL_RELAY__` / `verdict=` / `substance=` / `constraints=`），不含领导类词。
- `FORBIDDEN_LITERALS_LOWER`（evolution/lint.rs:13-28）只含 人工接管/接管/人工/takeover 家族，**无** 领导/上级/老板/请示。
- `holding_reply_text_is_safe`（escalation/holding_reply.rs:11-33）委托 `passes_forbidden_words`，同词表、同样不含领导类词。

**裁决（用户拍板）：不加词表。** 理由与 PR #185 删除 relay 数字护栏一脉相承——字符匹配是威胁模型错误的 backstop：
- "领导/老板"语义高度依赖上下文：客户自称"李老板您好"会被误杀；"上面点头了 / 帮你争取到了"这类泄漏不含词表词会漏。既误杀又必漏，正是 #185 判定"字符 backstop 做不了语义判断"的同一类问题。
- "客户永不知道有领导"的忠实度由**已在链路的正防线**保障：relay/holding prompt（明令 AI 用自己口吻转述、绝不透传内部概念，substance 是唯一事实源）+ 独立 Review Agent（经 `inbound_prompt_content(is_synthetic_relay=true)` 同时看到授权 substance + 拟发 reply_text，具备语义判断全部上下文）。
- 与 #185 的方向一致性：语义判断交还 LLM，不用字符匹配假装能做语义。加词表会重新引入 #185 刚清除的那类误杀/黑洞风险。

**不改任何代码。** 本设计文档 + 台账（KD-02 标注"经裁决交 LLM/review，字符级词表是威胁模型错误 backstop，不实施；同 #185 数字护栏"）即裁决记录。

## KD-05：骚扰门统计口径漂移（加真实"最近推送时刻"字段）

**根因（亲验）**：
- `reassign_escalation`（ledger.rs:304-319）改派只 `$set { principal_wxid, updated_at }`，**不动 created_at**。
- `count_pushes_today`（ledger.rs:347-366）filter `created_at >= since_ms`；`latest_push_ms`（ledger.rs:371-387）`sort created_at:-1` 取 created_at 当推送时刻。
- 改派后行的 `principal_wxid=next` 但 `created_at`=原始创建时刻 → 对 next 算骚扰门：`latest_push_ms(next)` 返回陈旧时刻低估最近打扰（dedupe 窗内可能再推）；改派跨天时今天推给 next 的卡不计入 next 当日 cap。

**存储键约定（亲验）**：`AgentPrincipalEscalation`（models.rs:3701-3743）**无 `#[serde(rename_all)]`** → 全字段 snake_case 存储（`principal_wxid`/`created_at`/`last_holding_reply_ms`，与现有 `doc!` 查询逐字一致）。新字段须 snake_case。

**修复**：加 `last_pushed_at_ms: Option<i64>`，镜像现有 `last_holding_reply_ms`（models.rs:3734）范式：
```rust
/// KD-05：本条台账最近一次被推卡给【当前 principal】的时刻（epoch ms）。骚扰门
/// count_pushes_today / latest_push_ms 用它而非 created_at（改派换 principal 时 created_at
/// 不刷新会低估对 next 的打扰）。首推创建时=created_at；每次 reassign 刷新为改派时刻。
/// #[serde(default)] 兼容旧文档（缺字段→None，由 m031 backfill 补成 created_at）。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub last_pushed_at_ms: Option<i64>,
```

三处写/读点：
1. **首推创建**（`insert_pending_escalation` ledger.rs:56 附近）：`last_pushed_at_ms: Some(now.timestamp_millis())`（创建即首推）。
2. **改派**（`reassign_escalation` ledger.rs:313）：`$set` 增 `"last_pushed_at_ms": DateTime::now().timestamp_millis()`（与 updated_at 同步刷新为改派时刻）。
3. **骚扰门读**：
   - `count_pushes_today`（ledger.rs:360）filter `created_at` → `last_pushed_at_ms`（键 `{ "last_pushed_at_ms": { "$gte": since_ms } }`，注意值是 i64 非 DateTime）。
   - `latest_push_ms`（ledger.rs:382）`sort created_at:-1` → `sort last_pushed_at_ms:-1`，返回值取 `e.last_pushed_at_ms`（已是 i64，无需 timestamp_millis()）。

**历史行兼容 + 治本 backfill**：旧 pending 行无 last_pushed_at_ms（serde default→None）。新增迁移 `m031_backfill_escalation_last_pushed_at`：把现有 pending 行的 `last_pushed_at_ms` 补成其 `created_at`（`$set` from `$created_at`，仅 `last_pushed_at_ms:$exists:false` 命中）。语义保持——历史行的"最近推送时刻"就近似取创建时刻，与旧口径字节等价。语义保持型回填，**不加 APP_ENV 守卫**（同 m018/m022/m025/m030，见批C家族②裁决）。id `2026_07_031_backfill_escalation_last_pushed_at`（排 m030 后）。

## KD-06：孤儿 pending（position 未命中回落链首）

**根因（亲验）**：`next_decider_on_timeout`（policy.rs:105-116）：
```rust
let timeout = policy.timeout_hours?;              // 未超时/无超时 → None
if age_hours < timeout { return None; }           // 未超时 → None
let idx = policy.decider_chain.iter().position(|d| d.wxid == current_wxid)?;  // ← 未命中即 None
policy.decider_chain.get(idx + 1)                 // 链尾越界 → None
```
`?` 在 position 未命中（当前 principal 不在链中）时返 None。scan（mod.rs:378-428）消费 None 时只能区分"超时与否"，把**真链尾**（合法：继续等）和**改链孤儿**（bug：current 已不在链）都当链尾——只发客户安抚、pending 永不改派。admin 改 decider_chain（删/换人）后旧 pending 的 principal_wxid 可能已不在新链 → 永久卡在失效决策人名下。

**关键：三种 None 语义必须分开**。当前 `next_decider_on_timeout` 返 None 混了①未超时②真链尾③孤儿。①在 :111 提前 return（scan 靠 timed_out 判据能识别"未超时"），②③都在超时后返 None 且 scan 无法区分。修复只需把 position 未命中（③）从 None 里剥离：

```rust
let timeout = policy.timeout_hours?;
if age_hours < timeout {
    return None;
}
match policy.decider_chain.iter().position(|d| d.wxid == current_wxid) {
    Some(idx) => policy.decider_chain.get(idx + 1), // 在链中：下一位（链尾→None，合法继续等，行为不变）
    None => policy.decider_chain.first(),           // 不在链中（改链孤儿）：回落链首，重新入链
}
```

**四象限行为**（超时前提下）：
- current 在链中、非链尾 → `get(idx+1)`=下一位 —— **不变**。
- current 在链中、链尾 → `get(idx+1)`=None → scan 发安抚保持 pending 继续等 —— **不变**（合法链尾语义保住，这是本修复必须不破坏的）。
- current 不在链中（孤儿）→ `first()`=链首 → scan 走改派路径推给链首 → 重新入链 —— **修复**。
- 空链 → `first()`=None → scan 发安抚（空链无人可推，安抚正确）—— 合理。

**语义确认**：position 未命中的前提是 current_wxid 不等于链中任何人（含链首），故回落链首必是换了个在链的人，不会回落到失效 principal 自己。

## 改动面

- **Modify** `src/models.rs`：`AgentPrincipalEscalation` 加 `last_pushed_at_ms: Option<i64>`。
- **Modify** `src/agent/escalation/ledger.rs`：`insert_pending_escalation`（:40-60 结构体字面量）初始化；`reassign_escalation`（:313 `$set`）刷新；`count_pushes_today`（:360）/`latest_push_ms`（:379-386）查询键+返回值换 last_pushed_at_ms。
- **Modify** `src/agent/escalation/policy.rs`：`next_decider_on_timeout`（:114-115）position 未命中回落链首。
- **Create** `src/db/migrations/m031_backfill_escalation_last_pushed_at.rs` + `src/db/migrations/mod.rs` 注册（mod 声明 + MIGRATIONS 追加，id `2026_07_031_backfill_escalation_last_pushed_at`）。
- **Modify/Create** 测试：policy.rs 单测扩展（KD-06 四象限）；m031 纯函数单测；集成测 `tests/escalation_push_time_reassign.rs`（KD-05 改派刷新 + 骚扰门口径）。
- 注意 `logic.rs:487` / `mod.rs` 等处 `AgentPrincipalEscalation` 字面量构造点须补 `last_pushed_at_ms` 字段（否则 E0063），全仓 grep 补齐。

## 测试计划

- **单测（lib，本地可跑）**：
  - KD-06 `next_decider_on_timeout`：①current 不在链→回落链首（新，退回 `?` 即 None 变红）②真链尾→仍 None（锁死合法行为不被误伤）③在链中间→下一位④空链→None⑤未超时→None（保留既有断言）。
  - m031 `backfill_filter`（pending 且 last_pushed_at_ms 缺失）+ pipeline（`$set last_pushed_at_ms=$created_at`）纯函数断言。
- **集成测（#[ignore] CI Docker）**：
  - KD-05 `reassign_refreshes_last_pushed_at_and_gate_uses_it`：seed pending（principal=A，created_at=旧时刻，last_pushed_at_ms=旧）→ reassign 到 B → 断言行 last_pushed_at_ms 刷新为改派时刻（≠ created_at）+ `latest_push_ms(B)` 返回改派时刻。
  - m031 `backfills_last_pushed_at_from_created_at`：raw insert 缺 last_pushed_at_ms 的 pending 行 → 跑 m031 → 断言补成 created_at；含 last_pushed_at_ms 的行不被覆盖（幂等）。

## 回归风险

1. **KD-06 误伤真链尾**：测试②专门锁死"链尾→None"不变。回落链首只在 position 完全未命中时触发（改链孤儿），正常链路 current 必在链中。
2. **KD-05 字段兼容**：`#[serde(default)]` 旧行→None；查询换键 + backfill 双保险，历史口径不漂。骚扰门用真实推送时刻只会让 dedupe/cap 判断更严格正确，不会误放行。
3. **字面量构造点遗漏**：加 struct 字段须 grep 全仓 `AgentPrincipalEscalation {` 补齐（logic.rs:487 测试构造点等），否则 E0063 编译失败——同 config_field_add 教训。

## 非目标（YAGNI）

- KD-02 不加代码（已裁决）。
- 不动 `push_allowed`/`in_quiet_hours` 逻辑（口径正确）。
- 不改 scan 主流程（靠 next_decider_on_timeout 返值语义修正自然生效）。
- 不含 KD-07（改派缺 next==客户守卫）/ KD-08 / KD-09 / KD-10（其它 Low）。
