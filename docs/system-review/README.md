# 全系统 100% 代码审查

本目录保存 WechatAgent 全系统审查的持久化证据。审查以逐文件亲读为准；任何结论必须能回指当前快照中的代码、测试、配置或规范，不以历史报告代替代码事实。

100% 文件覆盖不是最终交付点。覆盖与交叉验证完成后，还必须依次执行两轮收尾复审：第一轮从真实业务闭环和生产副作用出发确认问题，第二轮从业务简化与反过度工程出发确认最小方案。两轮均采用“候选→正反向证据→反证→验证→去重”的循环；最终成立的问题统一进入 `human-confirmation-checklist.md`，留给人类逐项确认。

当前状态：阶段 1（规则、启动骨架与 CI）、阶段 2（数据模型、索引与迁移）、阶段 3（鉴权、Webhook、MCP、LLM、账号与媒体外部边界）和阶段 4（私聊 Agent 主链，B04A–B04L）已完成全文阅读；阶段 5 的管理面与业务路由批次 B05A–B05L 已完成，冻结树中的全部 `src/routes/**/*.rs` 已连续读到 EOF。阶段 6 的知识路由子集 B05K/B05L、`src/knowledge_wiki/*.rs` 内核 B06A、Knowledge Digest/Task B06B，以及 Digest/Task、Knowledge Agent、Ask/核验、真模型全能力/质量和 Knowledge 前端 B06C–B06J 均已完成；Knowledge Agent 主文件已在 B04G/B04H 全文结算。阶段 6 已完成全文阅读与端到端证据映射；阶段 7 的 Evolution、Planner、Worker 舰队、Behavior Signal/Supervisor 与提示词治理批次 B07A–B07E 已完成。阶段 8 已完成前端运行骨架 B08A、工作台/账号管理/AI 总控 B08B、用户运营数据层/联系人池/通讯录 B08C、智能驾驶舱/联系人观测与配置 B08D、传统用户运营视图/运营池 B08E、Operations 运行观测/任务动作/运营域契约 B08F、Campaign 活动列表/圈人预览/结果看板 B08G、自治回路/运营成效/发送成效 B08H、内容资产/产品成交/专属顾问 B08I、统一收件箱/请示通道配置 B08J、模型供应商配置 B08K，以及系统策略整体 B08L，正在继续逐频道前端全文审查。全系统审查尚未完成。证据见 `reading-notes.md`、`architecture.md`、`data-model.md` 与 `findings.md`。审查对象固定为 PR #223 的 head commit `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8`，不是浮动分支、当前工作树或未来 merge commit。PR 冻结元数据见 `baseline.json`；详细协议、范围和阶段计划见 `review-plan.md`；逐文件状态见 `file-ledger.csv`。

现有 `findings.md` 中 SR-001～SR-183 的阶段 11/12 两轮决策复审已完成：逐条结论见 `two-pass-review-ledger.md`，合并后的 36 项人类决策面见 `human-confirmation-checklist.md`。该完成状态仅针对现有 findings，不把 `file-ledger.csv` 中仍为 pending 的历史材料宣称为已全文阅读，也不替代人类最终选择。
