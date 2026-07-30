# HC-014 DomainProfile 协议生产发布记录（2026-07-27）

## 发布范围

本批发布 HC-014 中的 SR-043、SR-044、SR-072、SR-073、SR-074、SR-089、SR-090，不自动结算 SR-056、SR-094 或 SR-168。前端继续复用已部署的 69 文件构建；本批只切换后端，并保持 Evolution 关闭。

已部署协议包括：DomainProfile 版本、lineage current 与 workspace active 的唯一约束；draft append、publish/current 与 activate/runtime-active 的分离事务；永久非法生成状态机首写前拒绝；动态画像 kind 的单路径与保留 namespace 校验；Operation Domain reset 的 append-only 版本切换；Unicode key 的字符索引归一化；完整版本列表与统一待审入口。

SR-072 仍保留明确边界：核心 active 指针事务提交后的状态机发布、Policy 对账和联系人迁移是可见、可重试的附属步骤。状态机已切换而 Policy 对账瞬时失败时，缺失 Policy 的兼容路径可能短暂 fail-open；当前实现不是跨四集合原子提交，不能标为风险完全关闭。

## 候选与隔离烟测

- 候选后端 SHA-256：`dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`
- 旧后端 SHA-256：`32f496e6cbebcf9ed1fb284d4e9a1433b4c985da20533267f3842d7e27272ec8`
- 前端首页 SHA-256：`597ffecf9fdfacc9fda4de3ba5ff563d7f6fdc2ed71ee4ae005963c3992941b0`
- 候选大小：98,241,256 字节
- 候选在生产库当前快照恢复出的随机隔离库中启动；72 个集合、55 条迁移，m050–m054 均为 `applied`。
- 候选库账号与全部 worker/外链入口清零；候选仅监听回环地址，健康连续通过，69 个静态文件逐字一致。
- 正式四个 HC-014 集合的探针计数均为 0；候选未连接或改写生产业务库。
- 候选烟测证据目录：`audit/hc014-candidate-smoke-20260727T083000Z`。

## 切换与回滚

切换脚本先以只读 dry-run 核验旧/新哈希、前端不变、55 条迁移、m051 `applied`、MongoDB `rs0` 可写主节点和六类活动队列全空。apply 阶段停服后再次确认队列为空，创建生产库归档并保存旧二进制，再同盘原子替换；任一哈希、健康、迁移或队列门失败都会恢复旧二进制。

- 切换前 PID：`1967547`
- 切换后 PID：`2021387`
- `NRestarts=0`
- 生产库备份 SHA-256：`75099be7e83ca93797e08fd63ff6c5783c7890cb2b867f6897a4f1787729578b`
- 切换目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/switch-hc014-20260727T083954Z`
- 切换冻结清单 SHA-256：`c6712bf8a4070074e67d9ee85699129675312530c3892647ffe2143d0587fbcb`
- 部署后证据清单 SHA-256：`2d77e73c59d5d601334bfff7d3414108ab20471383ec15bc906a1c9e0f18f2b2`

自动回滚未触发。旧二进制、生产库归档、切换前后状态、健康响应、迁移快照、队列快照和校验清单均保留。

## 服务器验证

- 纯函数永久非法状态机回归：1/1。
- 授权服务器真实 `rs0` 四集合完整 BSON 零写红线：1/1；部署后再次运行：1/1。
- 部署后 DomainProfile 矩阵：28 个测试函数返回成功，其中 26 条真实 Mongo/Handler 路径实际执行；2 条真实模型生成用例因未配置 `REAL_LLM_*` 明确 self-skip，不计真实模型通过。
- SR-074 真实 Cookie Router + `rs0` reset：1/1，证明只追加一个新版本、旧历史全部保留、旧 current 降级且最终恰一 current。
- SR-044 生产 validator：1/1，覆盖 dotted path、美元前缀、非 canonical、系统保留名、`_updated_at` 后缀和重复 kind 拒绝。
- SR-089 真实生成 Handler：1/1，mock 模型返回含 `客户Stage`、`éValue` 的 JSON，未 panic，且只落 `draft/current=false/active=false` 候选。
- 部署后 6/6 稳定观察均保持 PID `2021387`、零重启、运行中与磁盘哈希一致、健康 `ok=true`、Evolution 关闭。
- 55 条迁移、m051 `applied`、六类活动队列为 0；正式四集合 HC-014 marker 为 0；部署日志错误门为空。

新增冻结证据：

- DomainProfile 矩阵：`audit/hc014-deployed-matrix-20260727T090000Z`，清单 SHA-256 `192c0994913623f9f1fee88a9eb9cf56ee5a1dddc71402e018afb26ce865abde`。
- SR-074 reset：`audit/hc014-sr074-reset-20260727T093000Z`，清单 SHA-256 `4bb3ff8aed60ca93b957c4fc57401fcd0869f2e2dcdef77093d4e5456f275901`。
- SR-044/SR-089：`audit/hc014-sr044-sr089-20260727T095000Z`，清单 SHA-256 `eee64d16811d141d1961bcf14cb4cf4482515b6d3c00a229e0bb94b1f716d634`。

矩阵本轮保留 25 座随机证据库用于审计，没有擅自删除；SR-074 和 SR-089 测试调用显式 cleanup。测试包装器曾因只读 HOME 的 mongosh 日志警告和“测试库数量必须不变”断言返回非零，但业务测试日志分别明确为 1/1 与 28/28；最终证据将包装器噪声与业务结果分开固化。

## 保留边界

- SR-072 的 Policy 短暂 fail-open 风险仍开放；若产品要求新状态机与全部 Policy 同时生效，需要另行将 Policy 预物化、完整校验并纳入同一切换协议。
- 两条真实模型生成用例没有实际调用模型，不能将本批记录为真实模型验证。
- SR-056 DomainSchema 与 SR-094 typed runtime 维持各自原有状态，不因本次 DomainProfile 发布自动结算。
