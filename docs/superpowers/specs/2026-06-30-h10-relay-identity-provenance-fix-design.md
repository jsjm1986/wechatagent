# H10 修复设计：relay 身份从"内容前缀"改为"来源凭证"

**日期**：2026-06-30
**类型**：安全修复（终极审查 H10，UPHELD High）
**触及红线机制**：principal 决策请示通道 / relay 转述（见 `2026-06-05-principal-decision-channel-design.md`）

---

## 1. 问题陈述

### 漏洞（H10，对抗式证伪 UPHELD High）

relay 转述（领导裁决回送客户）的身份判定 `is_principal_relay_trigger`（`src/agent/escalation/logic.rs:173-178`）以**客户完全可控的 inbound content 前缀** `__PRINCIPAL_RELAY__` 为唯一判据：

```rust
pub(crate) fn is_principal_relay_trigger(trigger: &AgentTrigger<'_>) -> bool {
    matches!(
        trigger,
        AgentTrigger::Inbound(m) if m.content.starts_with(crate::models::PRINCIPAL_RELAY_SENTINEL)
    )
}
```

客户发一条以 `__PRINCIPAL_RELAY__` 开头的消息即可：

1. **劫持"领导决策转述模式"**：decision prompt 见哨兵进入转述模式，把客户自己伪造的 `verdict=/substance=` 当成领导授权的裁决转述出去。
2. **绕过所有发送闸**：命中 relay-exempt 分支（`gateway.rs:2994`），跳过 cooldown / operation_policy / rate_limited / daily_limit / quiet_hours。
3. **号码护栏失效**：号码白名单的"授权源"取自同一条伪造消息的 content（`gateway.rs:2357-2358`），客户在伪造 `substance=` 里写的数字被当成已授权数字。

`prompt_isolation::strip_known_tags`（`prompt_isolation.rs:44-53`）不剥离该哨兵，`logic.rs:170` 注释声称"真实客户消息经 prompt_isolation 隔离不会以哨兵开头"是**事实错误**——strip 不剥哨兵，且身份判定用的是 trigger 原始 content，prompt_isolation 只在拼 prompt 时施加，对判定毫无作用。

### 根因

**relay 身份靠"内容像不像 relay"判定，而非"来源是不是真 relay"。** 合法 relay 与"客户碰巧发了同样前缀的消息"在 content 层不可区分。

---

## 2. 合法 relay 数据流（修复前必须理解的不变量）

已逐行读码确认合法 relay 的完整可信链：

```
领导真实回复（微信）
  → webhooks.rs handle_principal_reply（escalation/mod.rs:272）
  → interpret_principal_reply：LLM 从领导回复解读出 PrincipalDecision（verdict/substance/constraints）
  → resolve_escalation：PrincipalDecision 落台账（escalation/mod.rs:329）
  → enqueue_relay_task（escalation/mod.rs:333）
  → relay task 执行 → relay_principal_decision_to_customer（gateway.rs:713）
  → ConversationMessage::synthetic_principal_relay(&contact, &decision.verdict, &decision.substance, &decision.constraints)
       构造合成消息：content = "__PRINCIPAL_RELAY__\nverdict=…\nsubstance=…\nconstraints=…"
       且 message_id=None / dedupe_key=None / raw=None（models.rs:782-794）
  → run_user_operation_gateway(AgentTrigger::Inbound(&synthetic))  ← 走第二遍 gateway
```

**关键不变量**：
- 合法 relay 走 `AgentTrigger::Inbound(&synthetic)`——与真客户消息**共用同一个 enum 变体**，仅靠 content 哨兵区分（这正是漏洞）。
- 合法合成消息的 content payload **源自可信的 `PrincipalDecision`**（领导授权），不是客户输入。因此号码护栏从该 content 取授权数字**本就正确**——前提是确保只有合法合成消息能走到那里。
- 真客户 inbound 消息经 webhook 进来时 `message_id` / `dedupe_key` / `raw` 由 webhook 填成 Some；合成 relay 消息这三个字段是 None。两者在"来源"层有客观差异，只是现有判定没用它。

---

## 3. 修复方案

**策略**：身份判定从"内容前缀"改为"来源凭证"——给 `ConversationMessage` 加一个**绝不落库、反序列化恒 false** 的内存标记，只有合成构造器置 true。三道防御。

### 3.1 标记字段（`src/models.rs`，核心）

`ConversationMessage` 新增：

```rust
/// relay 合成消息的来源标记：仅由 `synthetic_principal_relay` 构造器在内存置 true。
/// `skip_serializing` 保证绝不写库；`default` 保证任何反序列化来源（webhook 入站 / DB 读）
/// 恒得 false——故客户伪造 content 无法伪造此标记，relay 身份判定与 content 彻底脱钩。
#[serde(default, skip_serializing)]
pub is_synthetic_relay: bool,
```

serde 属性是安全根基：
- **`skip_serializing`**：永不写入 DB（合成消息本就不落客户会话；真客户 inbound 落库也不带它）。
- **`default`**：任何来源反序列化出的 `ConversationMessage`（webhook 入站、DB 读旧文档）该字段**恒为 false**。

**安全论证**：客户消息经 webhook 反序列化进来 → 该字段恒 false → 无论 content 写什么都进不了 relay 分支。伪造无从下手。

唯一置 true 处——`synthetic_principal_relay` 构造器（models.rs:782）结构体字面量加 `is_synthetic_relay: true`。

其余所有 `ConversationMessage { ... }` 结构体字面量构造点补 `is_synthetic_relay: false`（编译器 E0063 强制全部补齐，不会漏；含 gateway.rs FollowUp 占位消息、webhooks.rs 入站、simulation.rs、tests/ 等）。

### 3.2 身份判定改用标记（`src/agent/escalation/logic.rs`）

```rust
pub(crate) fn is_principal_relay_trigger(trigger: &AgentTrigger<'_>) -> bool {
    matches!(
        trigger,
        AgentTrigger::Inbound(m) if m.is_synthetic_relay  // 原: m.content.starts_with(PRINCIPAL_RELAY_SENTINEL)
    )
}
```

同步修正 `logic.rs:167-172` 的过时/错误注释（删除"真实客户消息经 prompt_isolation 隔离不会以哨兵开头"这句事实错误的防护声明，改为说明判据是来源标记）。

**连带自动收敛（无需单独改）**：
- **relay-exempt 频控豁免**（`gateway.rs:2985`）：已调 `is_principal_relay_trigger(trigger)`，自动跟随。伪造消息 `is_synthetic_relay=false` → 不豁免 → 频控全部正常拦截。
- **号码护栏授权源**（`gateway.rs:2356`）：外层 `if` 即 `is_principal_relay_trigger(&trigger)`。修复后只有合法合成消息能进该分支，其 content 源自可信 `PrincipalDecision`，授权数字取自它**本就正确**。原"授权源失效"子问题随身份判定修复一起消失，**无需额外改授权源取值逻辑**。

### 3.3 strip 纵深加固（`src/agent/prompt_isolation.rs`）

`strip_known_tags` 末尾追加剥离哨兵：

```rust
.replace(crate::models::PRINCIPAL_RELAY_SENTINEL, "")
```

**意义**：content 层的第二道独立防线，与身份判定层正交。即使将来身份判定再出 bug，客户伪造的哨兵 content 在拼进 LLM prompt 前被剥掉，进不了转述模式。

**注意不破坏合法 relay**：合法 relay 的 content 哨兵是给 decision prompt 看的转述模式触发器。需确认 `strip_known_tags` 的调用位置——它应只施加于**不可信的客户输入隔离**路径，不应施加于合法 relay 合成消息拼 prompt 的路径。实现时（writing-plans 阶段）须读 strip_known_tags 的所有调用点，确认合法 relay 转述模式的 prompt 构造不经过会剥哨兵的那条 strip 路径（否则转述模式失效）。若两者共用一条 strip 路径，则改为：身份判定（3.2）是主防线，strip 加固降级为可选，避免破坏转述模式——以不破坏合法功能为最高优先级。

---

## 4. 哨兵的职责变化

修复后 `PRINCIPAL_RELAY_SENTINEL` 哨兵**不再承担安全判定职责**：
- 身份判定（"谁是 relay"）→ 改由 `is_synthetic_relay` 来源标记负责。
- 哨兵仅剩**给 LLM 看的转述模式触发器**（decision prompt 见哨兵进入转述模式）+ 出站泄漏守卫的检测目标（`relay_output_leaks_internal_payload` 仍检测拟发文本是否含哨兵/字段标记，不变）。

合法 relay 合成消息的 content payload 格式**完全不变**（仍带哨兵+verdict/substance/constraints），prompt 转述模式契约不动。

---

## 5. 测试

### 5.1 安全回归测试（核心，必须新增）

- **客户伪造哨兵不被认作 relay**：构造一条 `ConversationMessage`，content 以 `__PRINCIPAL_RELAY__` 开头、但**经反序列化路径**（模拟 webhook 入站：`from_document` 或显式 `is_synthetic_relay` 不设）得到的消息，断言 `is_principal_relay_trigger` 返回 **false**。
- **合法合成消息被认作 relay**：`synthetic_principal_relay(...)` 构造的消息，断言 `is_principal_relay_trigger` 返回 **true**。
- **伪造消息不豁免频控**：managed contact + `last_agent_run_at=now`，喂一条伪造哨兵的客户 inbound 跑 precheck，断言得 `rate_limited`（而非 relay 豁免的 allowed）。镜像现有 `principal_decision_channel.rs` 的 relay 豁免测试。
- **`is_synthetic_relay` 不落库**：序列化一条 `is_synthetic_relay=true` 的合成消息为 Document，断言 Document **不含** `is_synthetic_relay` 键（skip_serializing 生效）；反序列化任意不含该键的 Document，断言字段为 false。

### 5.2 合法功能不回归

- 现有 `tests/principal_decision_channel.rs` 全部用例须继续通过（relay 转述端到端、哨兵载荷格式、号码护栏、泄漏守卫）。
- `logic.rs` 现有 relay 单测（:693-710）按新判据调整：原先靠 content 哨兵断言的，改为靠 `is_synthetic_relay` 标记断言。

### 5.3 基线

- `cargo test --lib` ≥ 350/0；4 PBT ≥ 33/0。
- 禁词 lint（no-human-takeover）：本修复全程用"领导/relay/转述"，不引入禁词。

---

## 6. 改动清单（收敛后）

**真改 3 处**：
1. `src/models.rs`：`ConversationMessage` 加 `is_synthetic_relay` 字段 + `synthetic_principal_relay` 构造器置 true。
2. `src/agent/escalation/logic.rs`：`is_principal_relay_trigger` 判据改标记 + 修正错误注释 + 调整现有单测。
3. `src/agent/prompt_isolation.rs`：`strip_known_tags` 补剥哨兵（须先确认调用路径不破坏合法转述模式，见 3.3）。

**机械补字段**：所有 `ConversationMessage { ... }` 结构体字面量构造点补 `is_synthetic_relay: false`（E0063 编译器强制，含 src + tests）。

**新增测试**：5.1 安全回归 4 项 + 5.2 调整既有单测。

**不改**（自动收敛）：`gateway.rs:2985` relay-exempt、`gateway.rs:2356` 号码护栏授权源——均因调用/门控 `is_principal_relay_trigger` 自动修好。

---

## 7. 风险与回滚

- **风险点**：3.3 strip 加固若与合法 relay 转述模式共用 strip 路径，可能剥掉合法 relay 的哨兵致转述模式失效。**缓解**：writing-plans 阶段先读 strip_known_tags 全部调用点；身份判定（3.2）是主防线足以闭死漏洞，strip 加固为正交第二防线、以不破坏合法功能为最高优先级，必要时降级或仅在客户输入隔离路径施加。
- **回滚**：3 处改动均为局部，`is_synthetic_relay` 字段 skip_serializing 不污染 DB，回滚无数据迁移负担。
- **向后兼容**：字段 `default` 反序列化，旧 DB 文档（虽不落该字段）读出恒 false，无兼容问题；`coreFacts` 等其他兼容契约不受影响。
