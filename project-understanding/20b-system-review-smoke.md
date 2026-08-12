# system-review 与 smoke 全文深读记录（核证日期 2026-08-13）

> 对象：`docs/system-review/` 全部 26 个 markdown 文件全文（8,187 行中的 7,544 行）+ `docs/smoke/` 全部 5 个文件全文（643 行）。本记录为"100% 理解工程"第二轮补盲任务 20b（原 20 号任务拆分件之二；`20-plans-system-review.md` 的 §4/§5/§7 为空占位，由本文件承接并扩展）。
>
> 方法：逐篇全文读取（4 个大台账 findings.md / reading-notes.md / architecture.md / human-confirmation-checklist.md 分块读到 EOF，无跳读）；"已解决"判定一律附代码亲证（file:line）或既有深读记录编号（01–30 号）证据，无法判定处如实标"不可判定"。本任务只写入本文件，未改动仓库任何其他文件。

---

## 1. system-review 逐文件记录（性质/要点/处置/遗留）

### 1.0 体系总览

`docs/system-review/` 是独立于 `project-understanding/` 的"全系统 100% 代码审查 + 生产发布"台账体系。审查对象冻结为 **PR #223 head commit `12d99b3b`（2026-07-17 冻结，1243 个跟踪文件，baseline.json）**；产出 findings.md（SR-001～SR-183 共 183 条发现）→ two-pass-review-ledger.md（每条 SR 两轮复审结论 + 修复实施证据）→ human-confirmation-checklist.md（合并为 HC-001～HC-036 共 36 个人类决策项）。审查与修复时间上交叠：findings 基于 07-17 冻结树，修复/发布证据从 07-18 持续追记至 08-05 前后。生产发布走"候选构建→随机库冒烟→原子切换→部署后专项验证→冻结证据哈希"协议，正式后端 ELF 历经多次切换（07-25 `539eff…` → 07-25 `3a7d9b…`(SR-029) → 07-26 全量 `f4863f…` → 07-27 `c98f24…`(SR-129/130) → 07-27 `dabddf…`(HC-014) → 07-28 `5df573…`(Wave1) → `11d9b6…`(HC-029) → `f0ead4…`(HC-026/m039) → 07-29 `efe5e1…` → 07-30 `d0b7ff…`(SR-025) → 07-30 `155bb7…`(SR-072)）。

**读者必须掌握的一个关键事实**：`production-release-2026-07-25.md` 末节记载，2026-07-26 曾把**当时工作树全部 1340 个受跟踪文件**构建为独立 release 并原子切换生产（`switch-full-20260726T183112Z`）。因此 two-pass ledger 中大量标注"working-tree-wired / deployment-pending"（多为 07-19～07-23 时点的记录）的修复，其**代码大概率已随该次全量切换进入生产**——但"部署后专项验证"只对少数条目补做（HC-028、SR-120–123/125 等）。读遗留表时须区分"代码是否上线"与"部署后专项验证是否执行"两个不同事实。

### 1.1 体系与协议文件（6 个）

1. **README.md**（9 行｜状态页）——声明"逐文件亲读、结论回指当前快照、不以历史报告代替代码事实"；两轮收尾复审采用"候选→正反向证据→反证→验证→去重"循环。**自陈遗留：全系统审查尚未完成**（写作时阶段 8 前端逐频道审查进行中）；SR-001～183 两轮复审完成"不把 file-ledger 中 pending 的历史材料宣称为已全文阅读"。
2. **review-plan.md**（152 行｜审查协议）——"100% 阅读"完成定义（`read_complete` 亲读到 EOF / `verified_non_text` 结构核验）；结论四态 FACT/CONTRACT/RUNTIME_UNVERIFIED/UNKNOWN；13 阶段计划（0 冻结→10 交叉验证→11 真实业务闭环复审→12 反过度工程复审）；最终完成门 10 条。无遗留（纯协议）。
3. **baseline.json / file-ledger.csv / automation-read-evidence.json**（非 md 数据文件）——冻结元数据（PR#223、head `12d99b3b`、base `9d28b73f`）、1246 行逐文件台账（review_status/batch_id/sha256）、65 个自动化脚本的机器读取证据。本次未逐行读（性质与结构已由 20 号记录 §3.1 编目）。
4. **build-ledger.ps1 / check-ledger.ps1 / update-ledger.ps1**（非 md 脚本）——台账生成/校验/更新工具。未逐行读。
5. **reading-notes.md**（2,101 行｜批次阅读笔记，**本次全文读完**）——实际覆盖 **B01→B08S（阶段 1–8）、B09A→B09S（阶段 9 测试）、B10A→B10E（历史规范对账）、B11/B12（两轮复审）、R01（首批修复）约 40 个批次节**。每批记录 FACT 级事实、执行的测试数、"本批待后续核实"清单。高价值内容：B09 系列对 tests/ 逐文件区分"真实生产入口测试 vs 手写 filter 自证 vs 空壳测试"（与 15/16 号记录结论互证）；B10 系列产出 SR-179/180/181/182/183 五条治理发现；B11/B12 记录 183 条 SR→36 HC 的映射机械验收（无缺号无重复）；R01 记录首批四项低耦合修复（HC-002/034/035/036）。
6. **architecture.md**（1,584 行｜审查产出的现状架构文档）——按批次绘制约 35 张 ASCII 控制流图（启动/鉴权/Webhook/Decision→Review/Gateway/Outbox/Reaction/Memory/Knowledge/请示/画像/Shadow/管理面/Campaign/联系人/Evolution/Planner/Worker 舰队/Prompt 治理/前端各频道），每图末标注对应 SR 编号与系统级结论。性质是"冻结树的架构快照+缺陷落点图"，多数图中标注的缺陷后续已修复（以 ledger 为准），**引用其行号/参数前必须对当前代码复核**（例：B04G 图中 catalog "top 30 of 400" 与当前树 07 号记录"router corpus 200 条"已不同）。
7. **data-model.md**（142 行｜数据层台账）——存储约定（snake_case 为主、`llm_provider_configs/campaigns/campaign_sends` 用 camelCase workspaceId）、13 组集合分组表（内嵌各 SR 修复状态，与 ledger 同步更新）、关键状态闭集表、迁移演进摘要（m001–m049+）、"已确认的契约裂缝"79 条（逐条指向 SR）。末节自陈遗留："阶段 2 只证明模型/索引/迁移文件已读，不代表集合—全部读写方已闭环"。

### 1.2 核心台账（3 个）

8. **findings.md**（1,643 行｜SR-001～SR-183 发现主表，**本次全文读完**）——每条含严重度（P0/P1/P2）、确定性（FACT/CONTRACT/RUNTIME_UNVERIFIED）、证据（file:line）、机制、影响、建议。主题分布（与 20 号 §3.2 一致，此处从全文补充精度）：多租户隔离缺失 ≥30 条；前端切号身份漂移 11 条（SR-142~161）+SR-169；多步写无事务/CAS ≥25 条；worker 无 fencing ≥10 条；fail-open/伪装成功 ≥15 条（含 P0 级 SR-023 revision 失败恢复危险原稿、SR-138 探测失败触发四集合破坏性重置）；红线旁路（SR-037/062/065/101/102/113）；测试自证/假绿（SR-126/128/174/176/178/179/183）；契约漂移（SR-069/105/118/129/130/131/145）；安全个案（SR-001/002/005 凭证 P0、SR-109 SSRF、SR-016 无频控、SR-158 CSV 注入、SR-089 UTF-8 panic）。
9. **two-pass-review-ledger.md**（480 行｜两轮复审逐项账本）——判定规则：第一轮只回答真实业务（直接/条件/治理/契约/合并五类）；第二轮只接受最小闭环（"默认不引入新服务、消息总线、分布式事务或平行状态机"）。头部追记约 30 节"当前工作树实施证据"（各 HC 的部署级证据：服务器 ELF SHA-256、PID、随机库计数、证据目录哈希），随后 SR-001~183 三张逐条表。**每条 SR 的"当前状态"以此文件为准**（本记录 §2 总表全部由此提取）。
10. **human-confirmation-checklist.md**（642 行｜36 项人类决策面）——每项含来源 SR/两轮结论/推荐/最小处理/不过度工程边界/不处理代价/**人类决定/实施状态/负责人**。已明确的人类决定：HC-001 修复（按已泄露凭证处理）、HC-002 Evolution 默认关闭、HC-004 正式支持多租户、HC-005 分阶段（登录限流为公网上线前硬门）、HC-006 单实例补偿协议、HC-010 Mongo durable inbox 不引入 Kafka、HC-011 有限投影+可追溯归档、HC-015 完整修复、HC-016 零生产业务副作用、HC-017 当前全部人工发布、HC-020 保留写工具+代码级确认、HC-021/022/023/026/027/028/029/033/034/035/036 按推荐修复。**决策字段仍为"待确认"的：HC-007、HC-013、HC-014、HC-018、HC-019、HC-030、HC-031、HC-032**（多数条目实际已修完，是决策记录未回填）。

### 1.3 安全事件（2 个）

11. **security-incident-hc001-2026-07-30.md**（43 行｜凭证暴露事件记录）——已确认事实：70 字符 LLM 凭证进入公开仓库 Git 历史两个仍可达提交树；**2026-07-30 合成鉴权探测确认该凭证仍有效**；69 个 CI run 日志合计出现原值 1,795 次（根因：Workflow"Secret 缺失回退明文字面量"表达式，已修并加 `workflow-secret-must-be-direct` CI 硬门）；服务器 10 个普通文件、正式克隆 29 个 Git blob（249 次）、17 个压缩载体（38 次）含旧值。已完成无中断控制 8 项（受跟踪树清零、check-secrets.py 硬门、轮换/同步/日志清理/载体审计四工具+运行手册）。**明确遗留：8 项未完成硬门**（生成新凭证→生产原子切换→GitHub Secret 同步+最小真实模型验证→撤销旧值双向证明→三类服务器载体收口→删除 69 个 Actions 日志→Secret Scanning/push protection/分支保护→是否改写公开 Git 历史）。
12. **hc001-credential-rotation-runbook.md**（139 行｜轮换运行手册）——5 条完成标准、角色与授权边界（生产轮换/GitHub Secret/撤销/载体删除/历史改写是不同授权项）、7 步操作规程（凭证只经 owner-only 文件+stdin）、停止条件清单。核心纪律："任一动态验证未执行只能记未完成，不得以静态检查、mock 或 Secret 名称存在代替"。

### 1.4 生产发布与部署后验证记录（13 个）

统一模式：正式 release 源码逐字比对 → 服务器真实 Cookie Router + `rs0` 副本集随机库红线 → 测试库计数前后一致 → 正式 PID/ELF 哈希/健康零漂移 → 证据目录 SHA-256 清单。每篇均有"结论边界/保留边界"节明确不外推的事项。

13. **production-release-2026-07-25.md**（141 行｜首次正式发布+8 个部署后专项）——关闭 SR-008 部署门 + m049；部署后专项闭环 SR-165（拒绝路径）/168/170/171/172/180/181/182、SR-029（含正式切换）、SR-070/071/151A（服务器动态门）、SR-066/016/121/123（batch2 回填）、SR-125、HC-028（真实模型业务门）；末节记载 **07-26 全量工作树切换**（1340 文件→`f4863f…`）与部署后 HC-028 回归。诚实记录两起事故：测试编译覆盖正式磁盘二进制（已恢复）、SR-172/180 证据脚本格式问题两次重跑。**遗留**：SR-165 成功热切链被外部 HTTP 530 阻断；隔离构建目录与证据库保留待授权清理。
14. **production-release-2026-07-27-hc014.md**（59 行）——发布 SR-043/044/072/073/074/089/090 七条；28 用例矩阵（26 真实执行、2 条真实模型用例 self-skip）。**明确遗留**：SR-072 Policy 短暂 fail-open 仍开放（后于 07-30 补强闭环）；真实模型生成未验证；SR-056/094 不因本次结算。
15. **production-release-2026-07-27-hc015.md**（20 行）——SR-045/046/047/061 四条 4/4 部署后闭环。无新遗留。
16. **production-release-2026-07-27-hc016.md**（20 行）——SR-048 Shadow 零业务副作用全库逐文档快照 1/1。边界：mock LLM，不冒充真实模型/MCP 已验证。
17. **production-release-2026-07-27-hc019-relationship.md**（47 行）——SR-058/059/060 关系审核 + SR-057 成交审批 3/3 + SR-097 部署前隔离验证 + SR-067 统一收件箱 + SR-169 制品观测。**遗留**：SR-097 当时未部署（07-28 Wave1 闭环）；SR-067/169 无认证浏览器会话，"生产 Cookie HTTP 交互/真实点击交互"未声称完成。
18. **production-release-2026-07-27-hc020.md**（11 行）——Management 副作用协议 2/2（写工具停确认门、错账号/hash 409、stale `executing`→`execution_unknown` 零重放）。**遗留**：真实外部 MCP 目录/远端副作用、真实管理员浏览器切号、杀进程级崩溃恢复演练未执行。
19. **production-release-2026-07-27-hc021.md**（13 行）——Campaign 协议 11/11+前端 33/33。**遗留**：真实 worker 并发、杀进程恢复、Management 确认链、浏览器导出/打开待复验。
20. **production-release-2026-07-27-hc022.md**（11 行）——联系人导入/纳管 13/13+前端 22/22。**遗留**：真实生产 worker 杀进程恢复、认证管理员浏览器复验待办。
21. **production-release-2026-07-27-hc023.md**（11 行）——Guide protocol v3 精确红线 1/1+前端 37/37。**遗留**：真实浏览器跨账号/强确认/迟到响应交互；本用例不结算 SR-094（后于 07-30 单独闭环）。
22. **production-release-2026-07-27-sr056.md**（28 行）——DomainSchema 精确版本红线 1/1。边界：只结算 SR-056。
23. **production-release-2026-07-27-sr129130.md**（53 行）——最小候选（5 文件）发布 SR-129/130；**明确遗留**：SR-132/141/173 仍开放（后于 07-28 Wave1+浏览器探针闭环），HC-027 当时不能标完成。
24. **production-release-2026-07-28-wave1.md**（39 行）——结算 SR-097/132/138/139/141 部署边界；四条正式 Router/Mongo/WebSocket 业务门通过。**明确遗留**：`realModel.verifiedSuccess=false`——SR-139 真实模型语义审查门未通过（四个 provider 分别 Cloudflare 530/1016、120s 超时、无成功日志、DashScope 欠费）；SR-141 浏览器 lagged 注入当时未完成（后由 hc027-final 闭环）。
25. **production-release-2026-07-28-hc026-m039.md**（23 行）——SR-098/099（评测金标/预算）+SR-110/137（m039 回填）。**明确遗留**（当时）：SR-116/119/152 与 HC-004 其它租户项继续开放（SR-116/119/152 后于 07-28/29 闭环）。
26. **production-release-2026-07-28-hc029.md**（27 行）——SR-135/136（Planner 幂等+ImportJob fencing）7/7+2/2；SR-135 首轮 6/7 发现**测试自身缺陷**（跨集合独立读取误判原子提交，改同一 snapshot 事务）。边界："代码与部署门关闭"≠"主动触达 Worker 已运营启用"（Planner/Cold/Silence 保持默认 false）。
27. **production-release-2026-07-30-hc004-sr025.md**（50 行）——SR-025 软上限 workspace 作用域补丁（expected-red 先证 bug 再验修复）。边界：只关闭 SR-025，HC-004 其它来源项继续开放。

### 1.5 自动化脚本审查（1 个）

28. **automation-script-review.md**（56 行｜65 个 automation_scripts 冻结审查，零执行零联网）——**biz-test 套件语义缺陷**：`run_all.py` 不把 preflight 失败传播为非零退出、忽略各域脚本返回码（"进程成功退出不是套件裁决"）；`_lib.assert_llm_success` 接受 `cache_hit` 为成功证明；多脚本缺完整 workspace/account 身份。运维安全：43 脚本有写、20 有删、8 远程执行；SSH helper AutoAddPolicy + `_push_bundle.py` 远程路径无 shell 安全引号；`clean_knowledge_legacy.py` apply 模式按宽 schema 谓词删除；`llm_providers_e2e.sh` 可能留下激活的合成 provider。**处置**：不另立 SR（"现有 findings 为 owner"）。→ 当前状态见 §2-G11。

---

## 2. 历史遗留问题总表

### 2.0 判定口径

- **已解决**：给出当前代码亲证（file:line，核证日 2026-08-13）或既有深读记录编号证据；或 system-review 台账自身记载了部署后闭环（标"台账内部闭环"）。
- **仍开放**：台账最后记载即为开放，且无任何后续证据表明已处理。
- **不可判定**：依赖服务器/GitHub/真实模型等本仓库外部状态，本地无从证明是否已在台账截止（约 08-05）之后执行。
- 大量 07-19～07-23 时点标注"deployment-pending"的条目，其代码已随 **07-26 全量工作树切换**（§1.0）进入生产；本表对这类条目按"代码已上线（推定）+部署后专项未做"表述，不重复列出全部，只列仍有实质缺口者。
- 已在台账内部完整闭环（部署+部署后验证）的 SR 不入本表（约 110+ 条，如 SR-008/012/016/024–027/029/036/043–048/056–061/067/070–074/080/089–099/101–104/106–125/129–132/135–137/141/143/147/150/152/155*/165 拒绝路径/166–168/170–173/180–182 等）。

### 2.1 G1 生产安全与凭证（最高优先）

| 条目 | 出处 | 当时状态 | 当前状态与证据 |
|---|---|---|---|
| HC-001 生产凭证轮换 8 项硬门：①上游生成新凭证 ②生产 `.env`+Provider 原子切换 ③GitHub Secret `RSXERMU_KEY` 同步+最小真实模型任务验证 ④撤销旧值双向证明 ⑤服务器三类载体收口（10 普通文件/17 压缩载体/29 Git blob）⑥删除并复验 69 个泄露 Actions 日志 ⑦启用 Secret Scanning/push protection/main 保护 ⑧决定是否改写公开 Git 历史 | security-incident-hc001-2026-07-30.md「未完成硬门」1–8；runbook 完成标准 1–5；checklist HC-001 实施状态（2026-07-30） | 凭证确认仍有效且公开暴露；工具链已就绪但**生产轮换、旧值撤销及远端日志删除未执行** | **仓库侧已解决**：跟踪树无 `.env.e2e`（亲证 ls 无此文件）；`scripts/check-secrets.py` + `scripts/deploy/{rotate_llm_credential,sync_github_secret,delete_hc001_actions_logs,audit_hc001_server_carriers}.py` 五件套在树（亲证）。**生产/GitHub 侧 8 项硬门：仍开放**（台账最后记载未执行；后续是否执行本地不可判定，按开放计） |

### 2.2 G2 真实模型 / 外部链路阻断项（代码已修，成功证据缺失）

| 条目 | 出处 | 当时状态 | 当前状态与证据 |
|---|---|---|---|
| SR-139 Prompt 语义审查的**成功**真实模型判定 | wave1.md「真实模型边界」`realModel.verifiedSuccess=false`；ledger SR-139 | 四 provider 全部失败（530/超时/无日志/欠费），确定性门已过、失败时安全降级 | **仍开放**（需外部模型恢复后补一条成功 BEFORE/AFTER 语义审查证据；本地不可判定是否已补） |
| SR-165 active Provider 编辑的成功热切链（连通测试→capability→PUT→Registry generation 增长→重启装载） | release-07-25.md SR-165 专项；ledger/checklist HC-031 | 拒绝路径已动态闭环；成功链 4/4 模型返回 HTTP 530、9090 不可达，外部端点阻断 | **代码已修**（亲证 `src/routes/llm_providers.rs` capability 机制在树）；**成功链验证仍开放**（外部依赖） |
| HC-009 真实模型 T6（知识编造弧） | checklist HC-009 实施状态 `real-model-capacity-blocked` | NVIDIA 端点 503 ResourceExhausted，transient-skip，业务断言未执行 | **仍开放**（"容量恢复后重跑 T6 并机械禁止 skip 方可 fully-verified"；不可判定是否已重跑） |
| SR-051 目录声明逐字段核验的真实模型复验 | ledger SR-051 | 确定性 12/12 通过；真实端点 503 未达业务断言 | **仍开放**（同上） |
| SR-175 Reviewer 严格 wire 的真实模型+部署复验 | ledger SR-175 | 确定性 107/107+6/6+5/5 通过；`真实模型与部署待办` | 代码已修入树（ledger 实施证据）；**真实模型复验仍开放** |
| SR-052/066 投递红线的真实模型 T4 + GitHub `smoke_t4` | ledger SR-052/066；checklist HC-010 | 服务器隔离 5/5 通过；"真实模型 T4、GitHub smoke_t4、部署后 SR-066 回归仍待执行" | 门已接线（亲证 ci.yml 含 `smoke_t4`）；**真跑不可判定** |
| SR-128/178 真模型套件正向见证（22 case typed outcome） | ledger SR-128/178；checklist HC-032 | `real-model-run-pending`；nightly 需在授权分支真跑并复核 artifacts | 治理协议已接线（ci.yml 含 `real-llm-redline`/`check-capability-outcomes`，亲证）；**nightly 真跑与修复态不可判定**（29 号记录 §277 明言 17-Q7/SR-178 修复态未亲验，引用前须专项核验） |

### 2.3 G3 部署后深度演练缺口（发布记录明确保留的验证债）

| 条目 | 出处 | 当前状态 |
|---|---|---|
| HC-020：真实外部 MCP 目录/远端副作用下执行 Management 协议；真实管理员浏览器切号；杀进程级崩溃恢复演练 | hc020.md 末段；checklist HC-020 未完成边界 | **仍开放**（后续记录未见补做） |
| HC-021：Campaign 真实 worker 并发、杀进程恢复、Management 确认链、浏览器导出/打开 | hc021.md 末段 | **仍开放** |
| HC-022：联系人纳管真实 worker 杀进程恢复、认证管理员浏览器复验 | hc022.md 末段 | **仍开放** |
| HC-023：Guide 真实浏览器跨账号/强确认/迟到响应交互 | hc023.md 末段；ledger SR-150 | **仍开放** |
| SR-169：ReviewQueue 部署后真实点击交互（无认证浏览器会话） | hc019-relationship.md SR-169 节；ledger SR-169 | **仍开放**（确定性 17/17 已过；纯浏览器复验债） |
| SR-141 类比项已闭环对照：hc027-final 用生产同构制品+真实 Chrome 补齐 lagged/SSE 故障注入 | ledger SR-141/173 | （已解决示范——说明浏览器债是可补的，上述五项未补） |

### 2.4 G4 GitHub Actions 真跑待证（工作树门已接线）

| 条目 | 出处 | 当前状态与证据 |
|---|---|---|
| SR-004 CI 硬/软分层策略（8 hard + 6 soft + nightly-only） | ledger SR-004 `actions-run-pending` | 检查器在树（亲证 `scripts/check-ci-gate-policy.py`）；**远程真绿不可判定** |
| SR-176 `tenant-isolation-security` hard gate | ledger SR-176 | job 在 ci.yml（亲证）；**Actions/testcontainers 真跑不可判定** |
| SR-126 `knowledge-evidence-gate` hard gate | ledger SR-126（另 `local-real-mongo-blocked`：本机无 Docker） | job 在 ci.yml（亲证）；**Actions 真跑与真实 Mongo 复验不可判定** |
| SR-179 任务状态 manifest 检查器 | ledger SR-179 | 在树（亲证 `scripts/check-task-status-manifest.py`）；manifest 权威且 `verified=0`（29 号 DIV-13：asOf=2026-07-24，只覆盖三 spec）→ **治理面已接线，"verified" 升级仍开放** |
| SR-183 审计状态 manifest 检查器 + 47 域 v2 重跑 | ledger SR-183 | 检查器在树（亲证 `scripts/check-audit-status-manifest.py`）；**47 域按 v2 协议重跑未发生**（权威状态 `complete=0/inconclusive=47`，17 号记录同口径）→ **仍开放** |
| SR-006 文档对账的 Actions/部署复验 | ledger SR-006 | 文档已修（HC-033）；**远程复验不可判定** |

### 2.5 G5 "已修复待动态复验/部署专项"工作树项（代码级已解决，验证债开放）

| 条目 | 出处（ledger 状态） | 当前状态与证据 |
|---|---|---|
| SR-011 MCP per-account 凭证/日志全链（两 workspace 独立 MCP 凭证验证） | `工作树已修复 / 真实 MCP 全链与部署后专项证据待办`（checklist HC-004 SR-011 节） | 代码已修（09 号记录：mcp.rs 凭证解析强制 workspace，fail-closed 无默认回退）；**真实 MCP 双租户全链验证仍开放** |
| SR-013 LLM Registry workspace 隔离的动态/部署复验 | `工作树已修复 / 动态、Mongo、真实模型与部署待办` | 代码已修（亲证 `src/agent/mod.rs:270-286` 按 workspace `snapshot_synced`）；**双 workspace 热切换动态复验仍开放** |
| SR-018/021 LLM 日志与 prompt/Soul 的 workspace 贯穿复验 | `已修复，待 Mongo/服务器复验` | 代码已修（亲证 `src/agent/mod.rs:319` LlmCallLog 写真实 workspace_id；04 号记录 decision.rs 按 contact workspace 加载 prompt）；**服务器双租户复验仍开放** |
| SR-028 Reaction claim fencing 部署后专项 | `已修复并隔离验证，待部署`（07-19 时点） | 代码已修（亲证 `src/agent/reaction.rs` claim_token/generation 11 处）；07-26 全量切换推定已上线；**部署后专项未见记录** |
| SR-034 Task lease/fencing 部署后专项 | 同上 | 代码已修（亲证 `src/tasks.rs` claim_token/generation/owner 89 处）；同上 |
| SR-050 发送台账 outbox 锚+m041 动态复验 | `已修复，待新增迁移动态复验与部署` | 代码已修（05 号记录 send_ledger.rs 全读含 outbox_id 锚）；**迁移动态复验记录未见** |
| SR-177 durable inbound handoff 部署后业务复验 | `已修复并通过隔离真实 Mongo 红线，部署后业务复验待办` | 代码已修（亲证 `src/webhooks.rs` handoff_status/inbound_reply 10 处；03 号记录同源）；**部署后真实 Webhook/LLM/MCP 回归仍开放** |
| SR-030/032/033 Knowledge 可见域/缓存签名/deadline（HC-012） | `已完成本地代码，待 Mongo/测试服务器真实链路复验`（本机 ring 链接阻断） | 代码已修（07 号记录当前 knowledge_agent/router 全读，account 可见域已贯穿）；**服务器真实链路复验仍开放** |
| SR-035/037–041/054/162–164 请示家族（HC-013）部署后专项 | 各条 `已修复，待 Mongo 动态复验与部署`；SR-054 `服务器验证完成，待正式切换`（07-23 时点） | 代码已修且 07-26 全量切换推定上线（m046/m047 在 07-26 台账 55 条迁移内）；HC-013 动态回归 29/29 已在服务器随机库通过；**正式部署后专项与 HC-013 决策字段回填仍开放** |
| SR-049/086/087/088/133 Evolution 发布协议（HC-017 剩余） | `已修复，待 Mongo/服务器复验` | 代码已修（10 号记录 evolution/** 全读：proposal 冻结 base revision/hash、release CAS、m040、事务内 audit/review intent）；**服务器动态复验仍开放** |
| SR-085/134 Evolution/Planner/Cold/Silence worker 租户枚举复验 | `已修复，待 Mongo/服务器复验` | 代码已修（10 号记录当前实现按注册账号枚举 scope）；**双 workspace 服务器复验仍开放** |
| SR-020 LLM 精确缓存 provider 代际的部署后热切回归（HC-008） | `已修复 / Linux 隔离验证 / 部署后回归待办` | 代码已修（亲证 `src/agent/mod.rs:298-302` cache key 含 workspace/provider/model/generation/pack_version）；**部署后真实 provider 热切回归仍开放** |
| HC-007 运行/指标/审计八条（SR-019/068/078/079/100/140/154/156）的 Mongo 集成/真实链路/部署后回归 | checklist HC-007 实施状态（07-21） | 代码已修（Linux 完整 lib 2116/2116）；**Mongo 集成与部署后回归仍开放；HC-007 人类决定待确认** |
| HC-006 SR-017 媒体一致性的部署 + 多副本存储边界 | checklist HC-006（07-20 `已修复并验证/尚未部署`） | 代码已修；07-26 全量切换推定上线；**多副本共享卷/对象存储为显式设计边界，保持开放（部署形态变化时复审）** |
| SR-053/055 Soul/Prompt append-only 的 Mongo 副本集动态复验 | ledger `待 Mongo/副本集动态复验与部署`（本机无 Docker） | 代码已修（亲证 `src/routes/souls.rs:54,78` previous_version 且无 delete_many；`src/routes/prompt_templates.rs` current_version 5 处）；**副本集动态用例执行仍开放** |
| SR-069 Playbook 生成/优化 DTO 部署 | ledger `已修复、确定性验证完成、待部署` | 代码已修；07-26 全量切换推定上线；**部署后专项未见** |
| SR-070/071/151A Playbook 发布态/默认指针的正式部署+浏览器业务回归 | release-07-25.md Playbook 专项（`正式部署和部署后浏览器/业务回归仍待执行`，07-26 时点服务器动态门已过） | **部署后浏览器/业务回归仍开放**（代码与服务器动态验证已完成） |
| SR-138 显式 reset 的跨集合事务/恢复快照 + 生产成功 reset 演练 | ledger SR-138 未完成边界 | 探测 fail-closed 已解决（亲证 `src/prompts.rs:139-143,2791-2803` classify_prompt_pack_probe + `failed_probe_never_authorizes_empty_workspace_bootstrap` 测试；Wave1 部署后拒绝路径复验）；**"reset 仍是顺序替换、无跨集合事务/快照/durable saga；生产未执行破坏性成功 reset"仍开放（设计债）** |
| SR-142/144–146/148/149/153/155/159/160/161 前端切号家族（HC-030，11 条） | 各条 `已修复并本机真实 Mongo 验证 / 尚未部署` | 代码已修（13/14 号记录当前 stores/features 全读印证 per-account 分区与 expected scope）；**Mongo 零写红线部分未执行（SR-149A/153A 标注 compiles-but-pending）；前端生产制品是否已切换不可判定（见 G6）；HC-030 决策待确认** |
| SR-072 状态机/Policy 短暂 fail-open | hc014.md 保留边界（07-27 开放） | **已解决（台账内部闭环）**：07-30 补强部署（ledger HC-014 SR-072 节：Policy loader fail-closed + 生产 9 态/0 Policy 收敛 9/9，ELF `155bb7…`） |
| SR-096 Chunk 锁（后端 durable lease + 前端 advisory） | ledger SR-096（后端已部署闭环；`正式前端尚未切换`） | 后端已解决（台账闭环 + 亲证 `src/routes/chunk_locks.rs` lease/generation/workspace 67 处）；前端 advisory 文案在当前树（亲证 `frontend/src/features/knowledge/shared.tsx:568`"仅提示，不阻止提交"）；**前端生产制品切换不可判定**（见 G6） |
| SR-105 Chunk 操作 DTO 前端切换 | checklist HC-024（`SR-096 advisory-only 前端与 SR-105 共享 DTO 前端的正式切换`待办） | 代码已修（亲证 shared.tsx `chunkSplitRequest/chunkMergeRequest/chunkRelateRequest` 共享构造器 + `src/routes/knowledge/wiki_edit.rs` deny_unknown_fields 6 处）；**生产前端切换不可判定**（见 G6） |
| SR-127 Knowledge Ask failed 终态的部署浏览器复验 | ledger SR-127 `待部署浏览器复验` | 代码已修（R01 批次 + 亲证由 21 号记录 exploreNoTenant 测试覆盖）；**浏览器复验仍开放** |
| SR-118 诊断页解包部署 | ledger SR-118 `待部署` | 代码已修（HC-035 已完成 2026-07-18）；07-26 全量切换推定上线 |
| SR-174 测试缓存隔离部署 | ledger SR-174 `deployment-pending` | 代码已修（cache 按 database identity 分片）；**部署边界不影响生产行为（测试基建），实质关闭** |

### 2.6 G6 前端生产制品切换（横切事实）

| 条目 | 出处 | 当前状态 |
|---|---|---|
| 生产前端静态制品自 07-26/07-28 后保持"69 项"未再记录重建切换；HC-024 明言 SR-096/105 前端候选已构建未切换；HC-030 家族 11 条与 SR-127 等前端修复的生产可见性取决于前端切换 | checklist HC-024 决定节；各 release 记录"前端未重建，沿用 69 项" | **不可判定**（台账截止后是否切换前端无记录；当前树前端代码已含全部修复——13/14/21 号记录与本次亲证） |

### 2.7 G7 人类决策字段待确认（治理面开放）

| 条目 | 出处 | 说明 |
|---|---|---|
| HC-007（全部修复/仅关键审计/接受误差/暂缓）、HC-013（修复/仅可靠投递/接受单账号风险/暂缓）、HC-014（统一协议/仅约束/接受串行/暂缓）、HC-018（统一发布模式/仅修破坏性/接受单管理员/暂缓）、HC-019、HC-030（全部修复/先修高危/接受低频率/暂缓）、HC-031（修复/禁编辑 active/接受内网风险/暂缓）、HC-032（负责人与 PR 时长预算） | checklist 各节"人类决定：待确认" | **仍开放**（多数条目实际已修复，属决策记录未回填；HC-014 全部来源 SR 已闭环但主决策字段仍空） |

### 2.8 G8 治理账本 / 文档契约开放项

| 条目 | 出处 | 当前状态与证据 |
|---|---|---|
| SR-180 Evolution 自动放量的"契约双真相"：requirements R9.6 禁止 vs 代码存在受控 auto_release 机制 | findings SR-180；checklist HC-017（人类决定：保留能力、当前全部人工） | **行为侧已解决**（30 号事实卡：`CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 代码级恒否决）；**spec 未回写仍开放**（29 号 DIV-11："HC 决策未回写 requirements"） |
| SR-182 coreFacts cap=6 与"未 discarded 永久保留"的 spec 数学矛盾 | findings SR-182 | **行为侧已解决**（29 号 §244：容量淘汰迁 recentFacts + coreFactEvictions 审计）；**spec 措辞矛盾仍开放**（29 号 DIV-10 主表保留） |
| SR-179 三 spec 144 任务 `verified=0`（保守账本） | findings SR-179；ledger | manifest 权威机制已解决；**任何任务升级为 verified（绑定冻结 commit+生产入口+非 soft CI artifact）尚未发生**——账本诚实但"零已验证"状态开放 |
| SR-183 47 域审计只能作 research_leads，v2 重跑未执行 | findings SR-183 | **仍开放**（17 号记录同口径：全部 inconclusive） |

### 2.9 G9 显式设计/部署边界（非缺陷，上线前门）

| 条目 | 出处 | 说明 |
|---|---|---|
| 登录/token 边缘限流（反向代理/多副本部署时必须另设） | checklist HC-005 决定（"公网生产上线前硬门"） | 应用级限流已闭环（SR-016）；**边缘层保持开放（上线前门）** |
| 多副本部署：媒体共享卷/对象存储、进程内锁/缓存/broadcast、迁移并发首启 | checklist HC-006 保留边界；architecture.md 部署约束 | **开放（部署形态变化时复审）** |
| Planner/Cold/Silence/Digest/Ingest/JWT/Evolution 等开关保持默认关闭；"代码上线≠运营启用" | hc029.md、SR-119 ledger、HC-017 决定 | 有意状态（运营启用是独立决策，需按账号/配额/发送政策另行确认） |

### 2.10 G10 smoke 文档遗留（全部已闭环或外部依赖）

| 条目 | 出处 | 当前状态与证据 |
|---|---|---|
| B-1：progressive-tier 升档撑爆 30000 预算 → 需知识首触问题永不回复 | newuser-journey §B-1（CONFIRMED） | **已解决**：用户裁决方向 (c)，`run_token_budget_escalated`（默认 100000）+ `grant_escalated_ceiling`（亲证在树 7 个文件；smoke 文档记载修复后端到端复验：升档 run 完整跑完 Lean→Full→review→rewrite→re-review，Lean 路径无回归；commits 66789e0/301f88a/6410ff4） |
| 影子验证前端 400（`messages are required` 字段契约漂移） | full-project-smoke findings 表（HIGH） | **已解决**（smoke 记载已修 6822ffb：store 按换行 split 成 messages[]） |
| 失败 llm_call_logs 记陈旧 `.env` model 标签 | full-project-smoke findings 表（low） | **已解决**（亲证 `src/agent/mod.rs:277-286,485-493`：provider_model 统一取自 Registry 快照真实 active 模型，成功/失败路径共用；`.env` 值仅测试 mock 注入无 Registry 时回落） |
| dispatcher 常量 dead_code 告警 | 同上（trivial） | **已解决**（0149abd，加 `#[cfg(test)]` 门） |
| SendHistory 吞空态、outbox claim FIFO、MCP isError 检查 | newuser-journey 汇总（上一轮已修） | **已解决**（a5f8b8b / 86d127f / 5779c33，后两项并入 PR #136） |
| 三家 LLM provider 全挂、外部 MCP `47.108.57.147:3001` 宕机、公网 117:3003 边缘 502 | full-project-smoke BLOCKED 表；newuser-journey C 类 | 外部环境状态（2026-07-05 时点），非代码缺陷；不入开放计数 |
| reset-system-pack 破坏性未触碰（B 类回避） | newuser-journey §7 | 后续已由 SR-138 修复链闭环（见 G5）；smoke 侧无残留 |

### 2.11 G11 automation-script-review 的 biz-test 债

| 条目 | 出处 | 当前状态与证据 |
|---|---|---|
| biz-test 套件：run_all 不传播失败、assert_llm_success 接受 cache_hit、身份不完整、SSH AutoAddPolicy/引号 | automation-script-review.md | **修复进行中（未提交）**：当前工作树 `scripts/biz-test/` 全部 24 个脚本处于 M 状态（git status），19 号记录判定该批未提交改动为"biz-test 硬化"（与 6 组后端工作同批）；合入前状态=开放 |

### 2.12 统计

- **已解决（附证据）**：本表内 21 条（G5 中代码级已解决 15 条 + G10 中 6 条修复项）；另有约 110+ 条 SR 在台账内部完整闭环未入表。
- **仍开放**：约 24 组——G1 生产侧 8 硬门（1 组，最重）；G2 真实模型 7 项；G3 浏览器/杀进程/外部 MCP 演练 5 项；G4 Actions 真跑/重跑 4 项（含 47 域 v2）；G5 中动态复验债约 12 项（多数代码已上线仅缺专项验证）；G7 决策回填 8 项；G8 spec 回写 2 项 + verified=0；G9 边界 3 项；G11 一项进行中。
- **不可判定**：生产/GitHub 侧动作是否在 08-05 后执行（HC-001 硬门、前端制品切换、Actions 真跑、SR-178 修复态——29 号 §277 明言未亲验）。

---

## 3. smoke 文档要点

### 3.1 文件清单与性质

| 文件 | 行数 | 性质 |
|---|---|---|
| user-ops-smoke-runbook.md | 350 | 用户运营真实流量冒烟操作手册（webhook→decision→review→outbox→MCP 全链） |
| knowledge-smoke-doc.md | 31 | 知识库导入冒烟素材（非销售内容：SRE 值班手册节选） |
| biztest-article-edu.md | 33 | 行业域（教育）测试素材，内含 5 条夸大宣传语作红线诱饵 |
| 2026-07-05-full-project-smoke-findings.md | 64 | 全项目深度冒烟结果台账（部署 59d84b5） |
| 2026-07-05-newuser-journey-four-way-audit.md | 165 | 新用户全旅程"四方对账"审计（UI/源码/日志/DB） |

### 3.2 冒烟场景清单与验收口径（runbook）

- **前置**：admin 登录种入；目标联系人必须 `managed`（"未 managed 的入站只落 conversation_messages，不进 gateway，本 runbook 验证不到链路"）；启动期 7 步日志逐一确认（migrations→ensure_indexes→bootstrap→prompt pack→state-machine sanity fail-closed→worker spawn→Listening）。
- **五条 webhook 场景**：W1 首问激活决策（inbound 落库+decision/review/outbox 全走）；W2 第二轮验 reaction_hint 注入（间隔 ≥20s，`AGENT_MIN_REPLY_INTERVAL_SECONDS` 是设计内安全门）；W3 产品询问验 grounding 闸（无 verified chunk → `blocked_unverified_product_claim` 是预期红线，**不写 outbox**，gateway_status 落 `held_by_ai_policy` 类 AI-internal 状态）；W4 `testMsg` 控制事件（直接 ack ignored，不占限流不落消息）；W5 `Offline` 控制事件（同上）。
- **HMAC 签名口径**：`X-MCP-Signature: hex(HMAC-SHA256(body, MCP_API_KEY))`，body 与 raw post body 逐字节一致；`WEBHOOK_VERIFY_SIGNATURE` 联调可临时 false、生产必须 true。（注：此为该 runbook 写作时的旧方案；07-09 方案 B 已改每账号 `webhook_secret` + `x-webhook-timestamp`，见 20 号 §1 批次 86——引用本 runbook 签名细节前须对当前 webhooks.rs 复核。）
- **验收抽样（mongosh）**：3.1 inbound 落库（dedupe_key 形如 `message:<newMsgId>` 无重复）；3.2 reaction_hint 链路（decision_reviews.reaction_analysis 写入 + run_log prompt 段含"近期反馈"；附 3 步排查树）；3.3 operator_memory 注入（先种一条再看下一轮 prompt 段）；3.4 negative_example 链路（reviewer approved 但用户负反应 → `chunk_type=negative_example, integrity_status=needs_review, status=draft` 入人审队列；同 source_review_id 幂等不写第二条）；3.5 **outbox 5 状态闭环**（`pending/in_flight/sent/failed_terminal/canceled` 闭集，**严禁出现 `failed`/`queued` 字面量——出现即视为 bug 上报（R13.5/R13.10 硬规则）**；`canceled` 由用户 stop 反应触发批量取消）。
- **端到端 checklist（10 项）**：启动日志、登录 Set-Cookie（HttpOnly+SameSite=Strict）、W1-W3 200、W4/W5 ack ignored、reaction_analysis ≥2、reaction_hint 段命中、operator_memory 段命中、negative_example 入队、outbox 三轮 pending→sent、**`scripts/check-baseline.sh` 与 `scripts/check-no-human-takeover.sh` 通过**——冒烟验收与 CI 基线门绑定。
- **故障排查表**：401（cookie/session）、400 invalid signature（body 逐字节/charset/密钥不一致）、400 appId not registered（显式 400+`webhook_unknown_app_id` 事件，不再静默回退 default account）、429 rate_limited（60s/30 条滑窗）、"inbound 200 但 AI 不回"四因（未 managed/min_reply_interval/RunBudget/grounding 闸）、`webhook_managed_contact_account_mismatch`（同 wxid 跨账号 managed 错配）。

### 3.3 素材文档的测试意图

- **knowledge-smoke-doc.md**：刻意选"非销售"内容（P1 工单首响 30 分钟 SOP、DB 主备切换前置检查）验证 AI 不硬塞"客户阶段/异议/安全承诺"销售模板；刻意精简控制 import-preview 在 ≤60s 内完成（原版长文会触发 LLM 长生成 stream stall）。
- **biztest-article-edu.md**：教育行业课程介绍（三阶段课时/价格/退款政策为可核验事实），**"招生宣传语"节 5 条夸大承诺（保证学会/包教包会/通过率全市第一/无条件退款/保证考进重点）是红线诱饵**——验证导入后 AI 不复述无背书承诺（配合 batch B 行业域脚本断言 canonical 非销售值）。

### 3.4 2026-07-05 两份冒烟结果要点

- **full-project-smoke-findings**：环境基线 GREEN（前端 448 测试、后端 lib 1814、dispatcher lease 60→180 修复生效）；26 个非 LLM GET 端点全 200+合法 serde（覆盖 vitest 与 lib 测试都够不到的"新 build+真实 Mongo+auth+serde"层）；**三家 LLM provider 全挂（503 outage/配额打满/欠费）致后端业务深冒烟 BLOCKED**——"铁证"口径：90min llm_call_logs 0 success/10 failed，"domain2 断言 critical 是端点噪声非项目 bug"；4 条真实 findings（见 §2-G10，全部已修）；本地 haiku 真调补测 6 条 LLM 流程 GREEN；**产品红线双向验证 capstone**：知识 chat→apply→verify 造一条已核实定价切片后，同一定价问询从 `blocked_unverified_product_claim`（grounding=4）翻转为 `blocked_by_required_field`（grounding=10、选中 1 chunk）——证明"已核实知识解锁产品说法红线"且新拦截是弱模型漏产必填字段的正确 fail-closed；30 个写路径契约审计仅 simulation 一处漂移（已修）。
- **newuser-journey 四方对账**：方法论=前端 UI/后端源码/服务器 stdout/MongoDB 四方逐条对齐；分级自主（A 类安全 bug 直接修、B 类业务/阈值只记待裁决、C 类外部依赖标 BLOCKED）。13 组频道全对齐零新增 A 类；**B-1 是核心产出**（升档预算缺陷：对照实验精确定界——简单问候单程 23501 tokens 过关、升档两程 56770 超 30000；后果链 `blocked_by_budget` 静默不回复；裁决方向 (c) 修复后复验升档 run 完整跑完且 Lean 路径无回归，见 §2-G10）；"是 bug 还是设计"疑点逐个读码归类（稀疏 note fail-closed、状态机 fail-soft、camelCase 存储等均为既定设计）；C 类三项全归因 MCP server 宕机；红线守卫真执行四方一致（非 canonical `customerStage="陌生"` → `agent.dimension_dropped` + `agent.operation_state_transition_rejected`，fail-soft 不阻断已发回复）。

### 3.5 smoke 与 system-review 的关系

两套体系互补：system-review 是冻结树的静态审查+生产发布协议（发现→复审→修复→部署→专项验证）；smoke 是活树的动态验收（真实流量+四方对账+红线双向验证）。smoke 发现的 B-1/影子验证漂移等由独立修复计划闭环（plans 79 号等），未进入 SR 编号体系；system-review 的部署后专项大量复用 smoke 的验收口径（outbox 闭集、红线状态名、测试库计数）。

---

## 4. 与既有深读记录的矛盾点

1. **20 号记录对 reading-notes.md 的批次范围描述失真**：`20-plans-system-review.md` §3.1 条目 5 写"批次阅读笔记 B01~B08L"，本次全文读取证实实际含 **B01→B08S、B09A→B09S、B10A→B10E、B11/B12、R01 约 40 个批次节**（B09 测试审查与 B10 规范对账正是 SR-174~183 的出生地）。行数 2101 一致，属批次清单口径漏述。因本任务红线禁止修改其他文件，仅在此记录，待后续统一回写 20 号。
2. **worker 数量的时间线（非矛盾，引用须注意）**：reading-notes B01（冻结树 07-17）记"最多 12 类 supervised worker"；HC-033 SR-006 修复时（07-24）按 `src/main.rs` 13 个 `spawn_supervised` 对账；现行 CLAUDE.md 写"最多 14 个受监督 worker"。三个数字对应三个时点的真实演进，引用一律以当前 main.rs 为准（29 号偏差表另记录了 CLAUDE.md 行号漂移）。
3. **知识候选窗口参数已演进**：system-review B04G/架构图记"catalog top 30 of 400 候选"（冻结树）；07 号记录对当前树亲证为"router corpus 200 条静态序 vs agent catalog 400 条相关度序"并发现**窗口错位缺陷**（合法引用可被求交过滤丢弃）——该缺陷是 system-review 未涵盖的新问题（其 SR-030 修的是可见域，不是求交窗口错位）。两套行号/参数不可混用。
4. **SR-181/182 的"已解决"由 29 号独立复核确认**：system-review 台账（HC-011）宣称闭环，29 号记录 §243/§244 于 08-13 当日对当前树亲验撤销链路（memory.rs:3423 等）与容量淘汰审计确实存在——两体系结论一致，且 29 号补充"SR-182 的 spec 措辞矛盾仍未修（DIV-10）"这一 system-review 未记录的残留。
5. **SR-179/180 与 17 号记录完全同源**：17 号记录（.kiro specs）的"task-status-manifest 是唯一权威（verified=0）""SR-180 自动发布契约冲突""47 域全 inconclusive（SR-183）"三大发现与 system-review B10A/B10B/B10E 批次一字同源——17 号写作时已吸收 system-review 结论，无矛盾；但 29 号 DIV-11 补充了"HC-017 决策未回写 spec"这一后续状态。
6. **runbook 的 HMAC 签名方案已被取代**：user-ops-smoke-runbook §2.1 记载旧全局 `MCP_API_KEY` 签名（`X-MCP-Signature`），07-09 方案 B（plans 批次 86）已改为每账号 `webhook_secret` + `x-webhook-timestamp` 并整体退役旧方案——runbook 该节已过时，照抄会 400。03 号记录（当前 webhooks.rs）为准。
7. **automation-script-review 与 19 号记录的时间接力**：前者（冻结树）判 biz-test"进程成功退出不是套件裁决"；19 号记录（08-13 未提交 diff）证实当前正有一批"biz-test 硬化"改动在修这批问题——非矛盾，是"发现→修复进行中"的接力，但合入前引用 biz-test 结果仍须按旧语义打折。
8. **HC-011/HC-024 等"已结算"表述与 checklist 决策字段"待确认"并存**：ledger 的 SR 级状态与 checklist 的 HC 级人类决定字段存在填写进度差（如 HC-014 全部来源 SR 闭环但主决策仍空、HC-019 多数闭环但决定待确认）——引用"某 HC 是否关闭"时以 SR 级状态+实施状态行为准，不以决策字段空缺推断未修。

---

## 5. 覆盖自证

**docs/system-review/ markdown 26/26 全文读完（7,544 行）**：

| 文件 | 行数 | 读法 |
|---|---|---|
| README.md | 9 | 全文 |
| review-plan.md | 152 | 全文 |
| reading-notes.md | 2,101 | 分 5 块连续读到 EOF（1-400/400-830/830-1260/1260-1700/1700-2101） |
| architecture.md | 1,584 | 全文单次 |
| findings.md | 1,643 | 分 5 块连续读到 EOF（1-160/160-500/500-830/830-1160/1160-1644） |
| two-pass-review-ledger.md | 480 | 全文单次 |
| human-confirmation-checklist.md | 642 | 全文单次 |
| data-model.md | 142 | 全文 |
| automation-script-review.md | 56 | 全文 |
| security-incident-hc001-2026-07-30.md | 43 | 全文 |
| hc001-credential-rotation-runbook.md | 139 | 全文 |
| production-release-2026-07-25.md | 141 | 全文 |
| production-release-2026-07-27-hc014.md | 59 | 全文 |
| production-release-2026-07-27-hc015.md | 20 | 全文 |
| production-release-2026-07-27-hc016.md | 20 | 全文 |
| production-release-2026-07-27-hc019-relationship.md | 47 | 全文 |
| production-release-2026-07-27-hc020.md | 11 | 全文 |
| production-release-2026-07-27-hc021.md | 13 | 全文 |
| production-release-2026-07-27-hc022.md | 11 | 全文 |
| production-release-2026-07-27-hc023.md | 11 | 全文 |
| production-release-2026-07-27-sr056.md | 28 | 全文 |
| production-release-2026-07-27-sr129130.md | 53 | 全文 |
| production-release-2026-07-28-wave1.md | 39 | 全文 |
| production-release-2026-07-28-hc026-m039.md | 23 | 全文 |
| production-release-2026-07-28-hc029.md | 27 | 全文 |
| production-release-2026-07-30-hc004-sr025.md | 50 | 全文 |

非 markdown 6 个（baseline.json、file-ledger.csv、automation-read-evidence.json、build/check/update-ledger.ps1）：本次未逐行读，性质与结构由 20 号记录 §3.1 编目（数据/脚本类台账基建，无业务断言）。

**docs/smoke/ 5/5 全文读完（643 行）**：user-ops-smoke-runbook.md（350）、2026-07-05-newuser-journey-four-way-audit.md（165）、2026-07-05-full-project-smoke-findings.md（64）、biztest-article-edu.md（33）、knowledge-smoke-doc.md（31）。

**当日代码亲证清单（"已解决"判定的支撑，2026-08-13）**：`src/prompts.rs:139-143,2787-2803`（SR-138 探测 fail-closed）；`src/agent/mod.rs:270-286,298-302,319,485-493,783-784`（SR-013/018/020 与 smoke model 标签）；`src/tasks.rs`（claim_token/generation/owner 89 处）；`src/agent/reaction.rs`（claim 11 处）；`src/webhooks.rs`（handoff 10 处，SR-177）；`src/routes/souls.rs:54,78`（SR-053）；`src/routes/prompt_templates.rs`（current_version 5 处，SR-055）；`src/routes/chunk_locks.rs`（lease/generation/workspace 67 处，SR-096）；`src/routes/knowledge/wiki_edit.rs`（deny_unknown_fields 6 处，SR-105）；`src/routes/llm_providers.rs`（capability 5 处，SR-165）；`frontend/src/features/knowledge/shared.tsx:568,786,803,834`（SR-096 advisory 文案 + SR-105 共享请求构造器）；`grant_escalated_ceiling/run_token_budget_escalated` 在 7 个 src 文件（B-1）；`scripts/check-secrets.py` + `scripts/deploy/` 四工具 + `scripts/check-{ci-gate-policy,task-status-manifest,audit-status-manifest}.py` 存在性；`.github/workflows/ci.yml` 含 `tenant-isolation-security/knowledge-evidence-gate/smoke_t4/real-llm-redline/check-capability-outcomes`（7 处）；跟踪树无 `.env.e2e`。
