# Agent Policy

Agent 策略定义哪些对象可以自动化、自动化到什么程度、何时停止、如何记录。

## Default Automation Boundary

当前默认：

```text
普通好友 normal：不自动回复
纳管好友 managed：允许私聊自动回复
微信群：暂不自动发言
朋友圈：暂不自动发布
```

## User Operations Policy

managed 好友允许 Agent 执行：

- 读取运营备注
- 读取历史消息
- 读取长期记忆
- 生成回复
- 调用 `message_send_text`
- 更新画像
- 更新记忆
- 创建跟进任务

Agent 必须遵守：

- 不编造成交、价格、承诺、身份等事实。
- 回复要像真人微信，不暴露系统或 AI。
- 如果最新消息无需回复，可以 `shouldReply=false`。
- 输出为空或 JSON 解析失败时不发送。
- 短时间内已回复时跳过，避免重复触达。

## Group Operations Policy

群运营第一阶段只允许：

- 群消息分析
- 话题总结
- 线索识别
- 回复建议

默认禁止：

- 自动群内发言
- 自动邀请成员
- 自动移除成员
- 自动修改公告
- 自动解散/退出群

未来开放自动群发言时，必须同时具备：

- 群白名单
- 触发条件
- 频控规则
- 禁止词规则
- 日志记录

## Moment Operations Policy

朋友圈第一阶段建议：

- AI 生成草稿
- AI 生成内容计划
- 选择内容资产
- 创建待发布任务

默认禁止：

- 无规则自动发布
- 无来源素材发布
- 高频连续发布

未来允许自动发布时，必须具备：

- 发布频率限制
- 内容来源记录
- 发布窗口配置
- 失败回滚/取消机制
- 发布历史审计

## Tool Risk Levels

低风险，可自动：

```text
auth_whoami
account_list
account_get_status
contacts_search
contact_get_detail
schedule_list
```

中风险，可按策略自动：

```text
message_send_text
media_get
schedule_create
schedule_cancel
```

高风险，默认不自动：

```text
moment_post_*
group_* 修改类工具
friend_delete
account_logout
personal_update_*
gewe_execute_raw
```

## Decision Logging

每次 Agent 行为都应记录：

- 输入对象
- 当前策略
- 是否回复
- 回复内容
- MCP 工具
- 成功/失败
- 失败原因
- 画像/记忆是否更新

日志是长期运营系统的安全边界，不是可选功能。

