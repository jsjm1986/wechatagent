# main 健康度审查 batch1 修复设计

> **来源**:`.git/sdd/main-health-cross-audit-2026-06-26.md` 净清单(7 维度 workflow 审 + 独立 verifier 证伪 + 主线程深核三轮交叉验证)。
> **范围**:修复 4 条 finding——SEC-1 + EVO-2(同源)、KNOW-1、FE-1。
> **分支**:`fix/main-health-audit-batch1`,不直接动 main。
> **日期**:2026-06-26。

## 目标

闭合两轮交叉验证确认的 4 条真问题(3 安全/access-control + 1 诚实置信),不引入新行为,补回归测试,过 baseline(lib ≥350/0,4 PBT ≥33/0)。

两个系统性根因:① evolution 端点信任记录自带 `proposal.workspace_id` 而非鉴权身份 `admin.current_workspace`(SEC-1/EVO-2);② guide preview 健康度前端重建逻辑与后端契约脱节(FE-1)。

## 设计决策(用户已确认)

- 跨 workspace 越权访问被拦 → **404 NotFound**(不暴露资源存在性,与 handler 既有 proposal-not-found 同码)。
- 本 PR 修 **4 条全部**(SEC-1/EVO-2/KNOW-1/FE-1)。
- FE-1 修法:**后端返回构建好的 items**(复用后端已有 `health_item` 逻辑),而非前端重写——深核发现后端已有正确的量纲/风险方向处理,前端重写会造成两侧漂移。

---

## 第 1 节:SEC-1 + EVO-2 — evolution 三端点加 workspace scope + 真实 actor

**改动文件**:仅 `src/routes/evolution.rs`(handler 层),`src/evolution/release.rs` 内部函数零改动。

三个 handler 各加 `Extension(admin): Extension<AuthenticatedAdmin>`:
- `get_evolution_proposal_detail`(:106)
- `release_evolution_proposal`(:138)
- `rollback_evolution_proposal`(:180)

**SEC-1(workspace scope)**:三处 `find_one(doc!{"_id": proposal_id})` 改为 `find_one(doc!{"_id": proposal_id, "workspace_id": &admin.current_workspace})`。跨 workspace → find_one 返 None → 走已有 `ok_or_else(NotFound)` → 404。

**EVO-2(真实 actor)**:release/rollback 内 4 处 `DEFAULT_RELEASE_ADMIN` 传参改为 `&admin.username`。

**核实结论(已亲核)**:
- 三路由(routes/mod.rs:957-968)挂在 `require_session` middleware(:982)之内,且同文件 `list_evolution_experiments:67` 已用同款 `Extension<AuthenticatedAdmin>`——extension 一定被注入,加参数不会 500。
- `DEFAULT_RELEASE_ADMIN` 常量**保留**(evolution.rs:581 `put_evolution_runtime_flag` 仍用作 updated_by 回落默认),只改 4 处传参。
- 内部 `release_threshold/release_prompt/rollback_threshold/rollback_prompt`(release.rs:36/195/393/520)签名 `(state, proposal_id, admin: &str)` 不变;内部自行 reload proposal 用 `proposal.workspace_id`——因 handler 已校验 `proposal.workspace_id == admin.current_workspace`,内部用值已等价,无需改 workspace 逻辑。
- `AuthenticatedAdmin`(auth/mod.rs:59-65)有 `user_id`/`username`/`current_workspace`。actor 用 `admin.username`(与 put_evolution_runtime_flag updated_by 同源,可读)。

---

## 第 2 节:KNOW-1 — 知识预览端点透传 workspace

**改动文件**:`src/agent/knowledge_router.rs` + `src/routes/knowledge/catalog.rs` + 测试文件。

`test_knowledge_route_for_contact`(knowledge_router.rs:276)加参数 `workspace_id: &str`(置于 `account_id` 后)。内部 contact=None 分支两处 `state.config.default_workspace_id` 改用该参数:
- :287 — `load_user_operation_domain_config` 的 workspace
- :297 — 合成 contact 的 `workspace_id`

下游 inbound(:343)、load_operation_knowledge 经 `contact.workspace_id` 继承,自动隔离。

**调用点改动(已亲核全部调用方)**:
- 生产:`catalog.rs:205`(search_operation_knowledge_tool)、`:259`(test_operation_knowledge_match)——两者已有 `Extension(admin)`,传 `&admin.current_workspace`。
- 测试:`tests/knowledge_router_fallback_e2e.rs:98/194/230`——3 处 `(&app.state, None, ACCOUNT, "...")` 补传 `&app.state.config.default_workspace_id`(语义不变)。
- mod.rs:91 是 re-export,无需改。

---

## 第 3 节:FE-1 — guide preview 后端返回构建好的 health items

**问题真相(深核确认)**:后端已有正确的 `health_item` 函数(shared.rs:468-491):风险类(`key.ends_with("Risk")`)自动反转 tone(score≥70 danger/≥40 warn/else good)、非风险类正常方向、量纲 0-100、label+detail 齐全。正常加载路径(userOpsStore.ts:342)和 :624 都直接用后端构建好的 items。**唯独 guide preview**(guides.rs:78-79 只返回裸 `healthScores` document,不返回 items)导致前端 :595 用坏函数 `healthFromScores` 自行重建——该函数 4 个 key(trust_level 等)与后端 7 个 camelCase key 零交集、阈值按 0-10(实际 0-100)、风险类未反转,三重错,展示伪造健康分。

**后端改动**(`src/routes/guides.rs` + 必要时 `shared.rs`):
guide preview 路径把 health 按 `{scores, items}` 完整形态返回(复用 shared.rs:448-466 已有的组装逻辑)。无论 scores 来自 LLM 生成(guides.rs:78 json_document_any)还是兜底 `health_scores_document`,都过 `health_item` 组装成 items,保证 items 与 scores 同源一致。`guide_preview_json`(shared.rs:926-943)输出 `health`(含 items)而非仅 `healthScores`。

**前端改动**(`frontend/src/stores/userOpsStore.ts`):
- `:595` 从 `healthFromScores(data.item.healthScores)` 改为直接用后端 `data.item.health`(与 :342/:624 同形态)。
- **删除**坏函数 `healthFromScores`(:198-225)与 `defaultHealthItems` 的 4 个错 key(:189-196)。若 legacy.tsx:313 仍需 `defaultHealthItems()` 兜底(health 为 null 时),改为返回空数组或 7-key 中性占位(中性占位不展示伪造分)。
- 顺带清理:`shared.rs:539-540` 注释非 stale 但描述与前端实现脱节,保留即可(后端契约正确)。

---

## 第 4 节:测试策略

| finding | 测试 | 位置/运行 |
| --- | --- | --- |
| SEC-1 | admin A 建 proposal,admin B(不同 workspace)GET/release/rollback → 断言 404;同 workspace → 正常 | `#[ignore]` 集成,Docker,CI 跑 |
| EVO-2 | release 后断言 `proposals.released_by == admin.username`(非 "admin") | 同上集成 |
| KNOW-1 | 非 default workspace admin 无 contact 预览 → 读到自己 workspace 知识(或空)非 DEFAULT;纯函数层断言 test_knowledge_route_for_contact 传不同 workspace 的隔离 | `#[ignore]` 集成,Docker,CI |
| FE-1 | guide preview 后 operationHealth.items 为 7 项、风险类高分=danger tone、值取自后端非占位 | 前端 vitest,本地可跑 |

**约束**:后端集成测试本地无 Docker → 写 `#[ignore]`,本地 `cargo check --tests` 验证编译,逻辑断言留 CI;前端 vitest 本地三连。baseline lib ≥350/0 不回退,no-human-takeover lint 无禁词(本 PR 新增行均技术词)。精确 `git add` 涉及文件,不 `-A`。

---

## 改动文件清单

- `src/routes/evolution.rs`(SEC-1+EVO-2)
- `src/agent/knowledge_router.rs`、`src/routes/knowledge/catalog.rs`(KNOW-1)
- `src/routes/guides.rs`、`src/routes/shared.rs`(FE-1 后端)
- `frontend/src/stores/userOpsStore.ts`(FE-1 前端)
- 测试:evolution 集成测试(新建或扩)、`tests/knowledge_router_fallback_e2e.rs`(补参)、knowledge 隔离集成测试、前端 health vitest

## 非目标(本 PR 不做)

- CONC-1/CONC-2/CONC-3(并发/OCC)、GATE-1、KNOW-2、EVO-3——留后续批次。
- prompt-pack 启动对齐线(另一工作线,不碰)。
- 不重构大文件、不动无关代码。
