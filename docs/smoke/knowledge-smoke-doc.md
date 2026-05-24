# 内部工单系统（OpsDesk）值班手册节选

> 知识库冒烟用的精简版。这份内容**不是销售文案、不是 FAQ、不是产品手册**——
> 是一线 SRE 值班的处置 playbook。刻意选"非销售"内容来验证 AI 是否还硬塞
> "客户阶段 / 异议 / 安全承诺"那套销售模板。
> （原版较长会触发 LLM 长生成→stream stall，这里只保留 P1 首响 + DB 主备
> 切换两段，把 import-preview 一次拿到 ≤60s 内能完成的输出。）

## 1. 值班动作清单

### 1.1 P1 工单首响 30 分钟

P1 出现时值班机器人会 @ 当班 SRE。30 分钟内必须：

1. **签收**：工单详情页点【签收】，状态从 `triaged` → `acknowledged`。
   未签收的 P1 在 25 分钟时触发二次告警。
2. **创建作战频道**：执行 `/incident open <ticket_id>`，自动拉 reporter / owning team / oncall manager 进群。
3. **首条状态更新**：在频道里发"已收到 / 当前怀疑方向 / 下一步动作 / 下次更新时间"四段，**不能**只发"在看"。
4. **是否升级**：5 分钟内仍无法定位影响面 → 升 P0 + 通知 oncall manager。SRE **不能**自己下调 P1 严重度。

### 1.2 数据库主备切换前置

主备切换前必须确认：

- MySQL **主库 binlog 同步延迟 < 30 秒**（Grafana `mysql/replica-lag`）；
- ClickHouse 集群**未在执行 DDL**；
- 切换窗口**不在 00:30–02:00**（自动备份窗口，强切会损坏 binlog 链）。

切换：`opsctl db failover --cluster=<name> --to=<target_az>`，等返回 `failover_done` 后看 QPS / 错误率 / p99 三条曲线，10 分钟内任一指标恶化超过 30% 必须 `opsctl db failover --rollback`（**不是**重新跑 failover）。

**绝对不能做的**：binlog 延迟 ≥ 30 秒时强切 → 备库丢交易，RPO 退化到 24 小时。
