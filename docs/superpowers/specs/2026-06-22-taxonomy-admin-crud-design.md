# 字典条目运营编辑（taxonomy admin CRUD 前端）设计

> 为系统策略频道的「双层标签字典灰度」面板（`TaxonomiesAdmin`）补「运营主动新增 / 编辑 / 废弃·恢复字典条目」的前端表单，接已就绪的后端端点。

**Date:** 2026-06-22
**Status:** 设计待审批
**Scope:** 纯前端（后端 `POST/PATCH/DELETE /api/admin/taxonomies` 已就绪）。改动内聚在 `frontend/src/features/system-strategy/index.tsx` 的 `TaxonomiesAdmin` 函数 + 同频道 vitest 测试 append。

## 1. 背景与动机

通用化改造后，画像维度（`customer_stage` / `intent_level` / `objection_type` / `relationship_type` / `purchase_lifecycle` / `churn_reason` / `value_tier`）的**具体合法取值**不再硬编码，而是存在 DB `system_taxonomies` 集合、按 `(scope, kind)` 组织、运营可按行业增删（`dimension_registry.rs` 只声明维度的结构契约与「取值要不要查字典校验」，取值在 DB）。

后端字典 CRUD 能力**已完整就绪**（`src/routes/admin_taxonomies.rs`）：
- `GET /api/admin/taxonomies?scope=&kind=&includeDeprecated=&includeAllVersions=`
- `POST /api/admin/taxonomies` — 新增（`version=1, current_version=true, seeded_by="manual"`，靠 `(scope,kind,value.id)` 唯一索引防重，重复→409）
- `PATCH /api/admin/taxonomies/:id` — 原地局部更新 label / aliases / description / deprecated（**不产生新版本**）
- `DELETE /api/admin/taxonomies/:id` — 软删（`value.status="deprecated"`，保留历史 run / 审核留档可读）

但**前端只接了「读 + 版本灰度 + 候选采纳」**：`TaxonomiesAdmin` 面板只读列表 + 复用 `ActiveVersionsBar` 做 publish/rollout/rollback；`TaxonomyCandidatesAdmin` 审核 AI 发现的新词候选。缺「运营主动建一套新行业词表 / 改别名 / 废弃旧阶段」的表单。运营想主动配阶段词表，目前只能等 AI 冒出新词进候选（被动），或绕过前端直接调 API / 改 DB。

本设计补齐这个「显形层」缺口——纯前端接已有 CRUD 端点。

## 2. 已锁定的关键决策（brainstorming 产出）

| # | 决策点 | 选择 | 理由 |
|---|---|---|---|
| D1 | 核心场景 | **通用字典编辑（所有 7 维度）** | 后端字典是通用的，`customer_stage` 只是维度之一；通用表单支持任意 `(scope,kind)`，不写死单维度 |
| D2 | 状态机联动 | **软提示（不阻断）** | 新增 `customer_stage` 时提示「需到状态机面板同步配置对应 state」并指向该面板；不做自动联动（跨两个后端资源事务，超纯前端范围；与设计「不阻断已发送/已写」一致） |
| D3 | 删除语义 | **废弃 + 恢复（贴合后端软删）** | 后端 DELETE 是软删 `status→deprecated`、PATCH `deprecated:false` 可恢复；UI 据实叫「废弃」「恢复」，不叫「删除」（避免误导运营以为物理删） |
| D4 | 实现组织 | **方案 A：扩展 `TaxonomiesAdmin` inline 表单** | 复用同文件 `TaxonomyCandidatesAdmin` approve 表单已验证的 inline 展开 + draft state + 逗号别名解析模式；零新组件 / 零新样式 / 零新交互范式 |
| D5 | 前端校验严格度 | **不超过后端** | id 不前端硬校验格式、`priority_weight`/`is_terminal` 不暴露；避免「前端比后端严」把合法写入拦死（项目踩过 customer_stage 越界校验的坑） |

## 3. 架构

全部改动内聚在 `system-strategy/index.tsx` 的 `TaxonomiesAdmin` 函数（现 590-688），**不新建文件、不抽子组件、不改 `types/index.ts`**（draft 类型本地定义，同 approve 的 `ApproveDraft` 先例）。复用现有 `.module.css` 类名（`styles.panel/panelHead/buttonRow/btnGhost/versionedListItem/field/input/inlineError/empty` 等）——零新样式、零硬编码颜色。

**版本化关系澄清**（设计要点）：字典的 CRUD 是**原地操作**（create 建 `version=1`、patch `$set` 原地改、delete 改 status），与 `ActiveVersionsBar` 的 publish/rollout/rollback **版本灰度是另一套叠加机制**。本表单只接 CRUD 三端点，**不碰版本化**——运营改个别名走 PATCH 原地改，不必走发版流程。

## 4. 组件结构与状态

### 4.1 新增 state（复用 approve 表单模式）

```ts
// 新增表单（面板顶部，可展开）
const [showCreate, setShowCreate] = useState(false);
const [createDraft, setCreateDraft] = useState<TaxonomyDraft>({
  scope: "global", kind: "customer_stage", id: "", label: "", aliases: "", description: "",
});
// 行内编辑（一次只展开一行，复用 expandedId 单选模式）
const [editingId, setEditingId] = useState<string | null>(null);
const [editDraft, setEditDraft] = useState<EditDraft>({ label: "", aliases: "", description: "" });
// 操作态 / 反馈（同 approve 表单）
const [acting, setActing] = useState(false);
const [info, setInfo] = useState<string | null>(null);
// error / loading / items / includeAll / includeDeprecated 已有
```

### 4.2 本地 draft 类型（不进 types/index.ts）

```ts
type TaxonomyDraft = { scope: string; kind: string; id: string; label: string; aliases: string; description: string };
type EditDraft = { label: string; aliases: string; description: string };
```

create 与 edit 分两个 draft：create 要 scope/kind/id（建条目主键，后端按 `(scope,kind,value.id)` 唯一），edit 只改 label/aliases/description/status（id/scope/kind 是主键不可改，对应后端 PATCH 只接这几个字段）。分开避免「编辑时似乎能改 id」的误导。

别名输入复用 approve 的 `aliases: string`（逗号分隔）+ `split(/[,，]/).map(trim).filter(Boolean)` 解析（中英文逗号都吃）。

## 5. UI 布局与数据流

在现有 `<section>`（panel）里加三块：

### 5.1 面板头「新增条目」按钮 + 展开表单
panelHead 的 buttonRow（刷新按钮旁）加「新增条目」按钮，toggle `showCreate`。展开后面板头下方显示表单：`scope`（默认 `global`）、`kind`（默认 `customer_stage`）、`id`（canonical id）、`label`（显示名）、`aliases`（逗号分隔）、`description`，底部「保存 / 取消」。

**状态机软提示（D2）**：`createDraft.kind === "customer_stage"` 时，表单内显示一行提示 + 指向状态机面板的引导：「新增客户阶段后，需到上方『状态机灰度』面板同步配置对应 state，否则该阶段的 operation_state 流转校验会被跳过」。纯提示，不阻断保存。

### 5.2 条目行操作按钮
每个 `versionedListItem` 的 head 区（`ActiveVersionsBar` 旁）加：
- 「编辑」→ `editingId=item.id` + 填充 `editDraft`（label / aliases.join("，") / description），inline 展开编辑表单（**仅 label / aliases / description，不含 id/scope/kind**）。一次只开一行。
- 「废弃」（`status==="active"`）/「恢复」（`status==="deprecated"`）→ 按钮文案随 `item.value.status` 切换。

### 5.3 数据流（复用 approve 的请求模式）

```
新增: api.postRaw("/api/admin/taxonomies",
        { scope, kind, value: { id, label, aliases:[...], description } })
      → 409 → setInfo("该字典条目已存在")（不当错误） → reload()
      → ok → setInfo(`已新增：${id}`) + 收起表单 → reload()
编辑: api.patch(`/api/admin/taxonomies/${id}`, { label, aliases:[...], description }) → reload()
废弃: api.delete(`/api/admin/taxonomies/${id}`) → reload()
恢复: api.patch(`/api/admin/taxonomies/${id}`, { deprecated: false }) → reload()
```

每次写后调现有 `reload()`（后端写操作已自动 `invalidate_global_taxonomy_cache`，AI 下轮即用新字典）。

**废弃可见性提示**：废弃成功后若当前未勾「显示已废弃」（`includeDeprecated=false`），给 `setInfo("已废弃，勾选『显示已废弃』可查看")`，避免运营以为条目消失了。

## 6. 错误处理与边界

- **前端校验（提交前拦，省往返）**：新增 `scope/kind/id/label` 任一为空 → `setError` 不发请求（`description` 可空）；编辑 `label` 为空 → 拦下（后端 PATCH 对空 label 返 400）。
- **409 重复**：复用 approve 先例当 `info` 不当 `error`（用 `api.postRaw` 拿 status 判断）。
- **400 / 404 / 其它**：`setError(res.data?.message ?? ...)`，显示在现有 `styles.inlineError`；404（并行删）→ reload 后自然消失。
- **网络异常**：`catch` 里 `setError((e as Error).message)`。
- **id 格式**：后端只 trim 不校验格式；前端给 placeholder（如 `need_discovery`）+ 说明「建议英文 snake_case」，**不硬校验格式**（与后端一致，避免前端更严拦死合法值——D5）。
- **scope**：默认 `global`；输入框 placeholder 说明「global = 全局，填 accountId = 仅该账号」。
- **不暴露 `priority_weight` / `is_terminal`**：create 端点对这俩硬编码（`None`/`false`），无 create 入参，前端暴露会是死字段；阶段权重/终态属状态机面板职责（呼应 D2 软提示）。
- **并发**：`editingId` 单值（一次一个编辑行）；新增表单与编辑互斥（开编辑收起新增，反之亦然）。

## 7. 测试策略

纯前端改动，vitest + 现有 `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx` **append（不删旧用例）**：

| # | 测什么 | 断言 |
|---|---|---|
| 1 | 新增提交形态 | `api.postRaw` URL=`/api/admin/taxonomies`、body=`{scope,kind,value:{id,label,aliases,description}}`、`"a，b, c"` split 成 `["a","b","c"]` |
| 2 | 编辑提交形态 | `api.patch` URL=`/api/admin/taxonomies/:id`、body 仅 `{label,aliases,description}`（无 id/scope/kind） |
| 3 | 废弃/恢复 | active 显示「废弃」点击调 `api.delete`；deprecated 显示「恢复」点击调 `api.patch {deprecated:false}` |
| 4 | 409 当 info | mock postRaw 返 409 → 显示 info、不显示 inlineError |
| 5 | 前端校验 | id 为空提交 → 不发请求 + 显示校验错误 |
| 6 | 状态机软提示条件渲染 | kind=customer_stage 显示提示；kind=intent_level 不显示 |

**构建/质量门**：`cd frontend && npm run build`（TS 零错误）+ `npm run test`（vitest 全绿含新用例）。

**no-human-takeover 文案扫描**：新增文案走「标签 / 字典 / 阶段 / 废弃 / 恢复 / 业务主题」中性词，无 `人工/接管/takeover/hand-off/人工介入` 等禁词。

**不做**：不写后端测试（CRUD 端点已有 `admin_taxonomies.rs` lib 单测覆盖请求形态/409/别名兼容，本次零后端改动）；不做 E2E（无此前端 E2E 设施）。

## 8. 边界 / 不做（YAGNI）

- **不做**字典管理大平台（标签云 / 批量导入导出 / 词表模板市场）。
- **不碰**版本灰度机制（publish/rollout/rollback 仍由现有 `ActiveVersionsBar` 管；CRUD 是原地操作，正交）。
- **不做**自动联动写状态机（D2 只软提示；跨资源事务超纯前端范围）。
- **不暴露** `priority_weight` / `is_terminal`（后端无 create 入参，属状态机职责）。
- **不前端硬校验** id 的 snake_case 格式（与后端一致，仅 placeholder 引导）。
- **不抽** TaxonomyEditor 子组件 / 不引入模态弹窗（无此频道先例，inline 展开是现有范式）。
- **不改**后端任何代码（CRUD 端点已就绪）。

## 9. 红线守卫

- 纯前端接已有端点；零后端改动。
- 遵 `docs/frontend-design-system.md`：复用既有 `.module.css` 类名，无新样式 / 无硬编码颜色 / 无新交互范式（inline 展开同 approve 表单）。
- 前端校验不超过后端（D5），避免拦死合法写入。
- 新增文案无 no-human-takeover 禁词。
- 新增测试只 append，不删既有 system-strategy 用例。
- baseline 不回归（纯前端，不影响 `cargo test --lib`）。
