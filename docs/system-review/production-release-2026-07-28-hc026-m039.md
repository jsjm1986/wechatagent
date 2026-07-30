# HC-026 / m039 生产发布闭环（2026-07-28）

## 范围与结论

本次只结算 HC-026 的 SR-098/SR-099，以及 HC-004 中由 m039 承载的 SR-110/SR-137；SR-116、SR-119、SR-152 与 HC-004 其它租户项继续开放。正式后端从 `11d9b6fd…6954` 切换为 `f0ead4f7…cde9b`。前端仍使用原 69 项静态资源，Planner、Cold Contact、Silence Signal 与 Evolution 开关均未改变。

## 验证与切换

- HC-026 的正式 Cookie Router + MongoDB `rs0` 红线 3/3：缺完整金标首写前拒绝；生产 run 不污染评测私有预算；未知 usage 明确降级并停止后续场景。确定性 mock LLM 只证明预算归属与失败计费，不代表模型质量。
- m039 专项 ELF `8cb62b24…fdf60` 在真实服务器 `rs0` 运行 3/3：精确回填/歧义零写、历史名称等价 scoped 索引复用、同 key 非等价选项 fail-closed。测试库清单前后 SHA-256 均为 `dd40781e…b924`。
- release 以 HC-029 最终源码为基线；226 项清单无漂移，恰好 8 个运行文件变化。候选 SHA-256 为 `f0ead4f7…cde9b`，大小 101,339,328 bytes。
- 生产等价随机库保留“m039 已 applied、父 Chunk 已退役、孤儿 Revision、scoped key +历史索引名”拓扑；候选仅回环启动并逐字服务 69 项静态资源，随后精确清理随机库。
- 原子切换前后活动任务均为 0。脚本先保存旧 ELF 与全库 gzip archive，再切换候选；健康、数据库、索引、静态或日志门失败会自动恢复。回滚未触发。

## 部署后状态

正式 PID `2196929`，`NRestarts=0`，运行中/磁盘/候选 ELF 均为 `f0ead4f7…cde9b`。m039 仍为 `applied`；94 条 Revision 与 3,206 条 BehaviorSignal scope 完整，Signal scoped dedupe 重复组为 0。启动日志明确记录两条历史名称 Revision 索引被语义复用，没有重跑 backfill 或创建 canonical 新名称。内外健康、69 项静态资源、零烟测库残留均通过。切换目录保留旧 ELF、8 文件源码快照及 39,606,961 bytes 全库备份。

## 冻结证据

- [m039 专项回归](../../audit/reconciliation/m039-index-compat-v2-evidence-20260728.tar.gz)：`b27eafe8…50b9b`。
- [候选与生产等价烟测](../../audit/reconciliation/hc026-m039-v2-release-evidence-20260728.tar.gz)：`c92d1a09…43920`。
- [部署后独立复验](../../audit/reconciliation/hc026-m039-v2-postdeploy-evidence-20260728.tar.gz)：`5f5f53e9…9fd9`。
