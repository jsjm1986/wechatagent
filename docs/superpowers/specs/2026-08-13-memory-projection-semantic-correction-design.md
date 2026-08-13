# 记忆投影中的事实更正语义判断设计

## 背景

隔离服务器真实模型验收发现：客户明确说“孩子其实 10 岁，不是 8 岁”时，Reply Agent
正确理解并回复“孩子 10 岁”，但发送后的 `user.projection.task` 返回
`memoryCandidates=[]`、`memoryWriteScore=0`、`consolidationNeeded=false`。后续自动
`memory_consolidation` 因没有候选而以 `no_candidates` 正常结束，旧的 8 岁事实继续生效。

根因不是任务竞态或固化器错误。现行投影 Prompt 只声明 `memoryCandidates: []`，没有声明
数组元素的字段协议，也没有说明如何区分真实事实更正与玩笑、反讽、假设或转述。

## 目标与非目标

目标：

- 仍由 AI 基于完整语境判断一段话是否是可长期使用的真实信息。
- 对高置信、客户本人、认真且明确的事实更正，产出可验证的记忆候选并触发固化。
- 对玩笑、反讽、假设、转述、犹豫和无法确认的信息，不写入长期记忆。
- 保持现有 Projection → candidate → durable consolidation → conflict resolution 链路。

非目标：

- 不在 Rust 中新增“其实”“不是”等关键词规则或正则抽取。
- 不绕过候选层直接写 `memoryCard`。
- 不放宽无证据候选、低分候选或冲突固化的现有安全门。
- 不覆盖运营手编或演化发布的 Prompt 版本。

## 设计

### 投影 Prompt 协议

在内置 `user.projection.task` 的 JSON 示例中补齐候选单项：

```json
{
  "type": "fact",
  "content": "一条原子化长期信息",
  "evidence": "客户原话或有上下文的行为证据",
  "importance": 1,
  "confidence": 1
}
```

`type` 使用现有合法类型；默认销售域为
`fact / preference / doNotDo / commitment / objection / openLoop / conflict`，非默认行业
继续由既有动态维度指引覆盖。`importance`、`confidence` 使用 1–10。

Prompt 要求 AI 先判断语义：

1. 信息是否来自客户本人，而非引用他人；
2. 是否是认真、明确、当前有效的陈述，而非玩笑、反讽、假设或试探；
3. 是否对未来运营有持续价值；
4. 是否修正了当前记忆中的同一属性。

只有前三项成立才生成候选。若第四项成立，则生成 `conflict` 候选，证据中保留客户原话，
将 `memoryWriteScore`、候选 `importance` 和 `confidence` 均设为 8–10，并令
`consolidationNeeded=true`。无法确认时保持空候选，不把猜测写成事实。

### 版本发布

不引入新的发布通道。`ensure_prompt_pack_v2` 已按内容对齐：

- 当前版本为 `seeded_by=system` 且内容漂移时，追加并发布一个新版本；
- 当前版本来自运营手编或演化发布时保持不动；
- 历史版本不可变。

因此候选二进制在隔离克隆库启动时可验证系统 Prompt 升版；正式部署也遵循同一边界。

### 验收数据流

域⑨使用无歧义话术：

> 我刚核对过信息，认真更正：孩子今年 10 岁，之前说 8 岁是我记错了。这不是玩笑，请按
> 10 岁更新长期记录。

验收按精确身份逐步证明：

1. webhook `msgId` 对应的 run envelope 到达终态；
2. `run_id:projection` 有成功的 `user.projection.task` 审计；
3. 该 parent `run_id` 产生包含 10 岁更正的 `memory_candidates`；读取时允许它已由同一
   固化任务从 `pending` 推进为 `consolidated`，但不接受 `ignored_low_score`；
4. 自动固化任务到达 `consolidated`；
5. `memory_card_version` 推进，10 岁进入生效事实层；
6. 8 岁必须退出 core/recent 生效层，并进入 `deprecatedFacts`，或由带审计的冲突裁决明确
   替换；
7. 同一 task claim generation 恰有一条完成事件，冲突事件不存在重复。

真实模型若返回错误 JSON 形态，验收可按既有策略做有限次新 run 重试；若 JSON 合法但对上述
无歧义更正仍不产候选，则继续判失败，不通过直接写库或放宽断言掩盖。

## 测试

- Rust 单测锁定投影 Prompt 包含候选字段协议、语境排除项和显式更正规则。
- Python 契约测试锁定 run/source identity、投影失败分类、BSON Long claim generation 和
  task/event 精确绑定。
- 隔离服务器仅使用随机 MongoDB、独立端口和 loopback MCP stub；正式数据库、正式 MCP 与
  正式进程均不得被测试修改。
- 域⑨通过后再运行完整隔离业务矩阵和零残留审计。

## 风险与回滚

- 风险：Prompt 更具体后候选数量可能增加。现有最多 6 条、候选校验、分数门、pending 阈值及
  memoryCard cap 继续限制放大。
- 风险：玩笑被误判。Prompt 明确要求先判语境；无置信证据时保持空候选。
- 回滚：Prompt 版本历史不可变，可通过现有版本发布/回滚机制恢复前一 current；无需迁移数据。
