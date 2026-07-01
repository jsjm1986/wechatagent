# 内容资产 / 知识库 前端概念混淆消除（导航文案对齐）设计

> 日期：2026-06-30
> 类型：纯前端文案 / 信息架构微调（不碰任何后端业务逻辑、不碰前端组件逻辑）

## 1. 背景与问题

后端层已确认（多波 opus 逐行核 + file:line 证据）：「内容资产（content_assets）」与「wiki 知识库（operation_knowledge_chunks）」是**两套独立集合、独立注入路径、职责正交不冲突**——素材库 = "AI 可原样引用/发送的话术·素材 + 禁语"，知识库 = "AI 产品声明的已验证事实依据（grounding 来源）"。grounding 红线只校验知识 chunk、不看素材，两者互补。

但**前端导航呈现层存在真实的概念混淆**，三处根因（均带 channels.ts 行号证据）：

1. **content 频道被贴"知识"标签**：caption=`素材知识`、eyebrow=`Knowledge Assets`（channels.ts:122/124），而其页面内标题实为「内容资产库」——频道层与页面层自相矛盾，且与知识库撞名。
2. **同组并列**：`Shell.tsx` 按 `group` 渲染导航，「知识」组并列 `内容资产`/`专属顾问`/`Wiki 管理`（channels.ts group 字段），素材库与知识库肩并肩、都顶"知识"。
3. **「Wiki 管理」名实不符（最严重）**：subtitle 写「管理知识库领域 schema、缺口信号与切片修订历史」（channels.ts:192），听起来是运维页；但探索证实它实际是**知识内容录入（导入向导 ImportWizard）+ 审核（待评审 ReviewView）+ 问答 + schema** 的全功能工作站（knowledge/index.tsx:43-64、320-323、608）。管理员想录知识内容，会因 subtitle 判定"此处与我无关"而**扑空**——录入口恰恰藏在这里。

> 既有缓解（保留）：内容资产上传表单 hint（content-assets/index.tsx:296）已写「若知识库已有同内容文本，请确认两边口径一致」——页面自身已意识到与知识库的关系，新 subtitle 与之呼应。

## 2. 目标与范围

**目标**：用纯导航文案对齐消除上述混淆——让管理员扫一眼菜单即可分清"配可发话术/素材进内容资产、录入/审核已验证知识进知识库 Wiki"，且知识录入口不再被 subtitle 藏住。

**范围边界（YAGNI）**：
- 只改 `frontend/src/app/channels.ts` 中两个频道的**显示文案字段**（label/caption/eyebrow/title/subtitle）。
- **不动**：频道 `id`（已核实 `knowledgeWiki`/`content` 作为 id 仅在 channels.ts 出现一次，别处同名是 CSS className，与 id 无关 → 改文案零逻辑风险）；不动导航**分组结构**（"知识"组成员不重排——重排 IA 超范围，且专属顾问归组问题与本任务无关）；不动任何页面组件、store、后端。

## 3. 具体改动（channels.ts，逐字）

### 3.A content 频道（:119-128）
| 字段 | 现状 | 改为 |
| --- | --- | --- |
| label | 内容资产 | 内容资产（不变）|
| caption | `素材知识` | `话术 / 素材` |
| eyebrow | `Knowledge Assets` | `Content Assets` |
| title | 内容资产 | 内容资产（不变）|
| subtitle | `维护产品资料、FAQ、话术、禁用表达、品牌语气和朋友圈素材。` | `维护 AI 可直接引用发送的话术、FAQ、品牌口吻、禁用表达与文件素材。事实依据与产品口径以知识库为准。` |

### 3.B knowledgeWiki 频道（:185-194）
| 字段 | 现状 | 改为 |
| --- | --- | --- |
| label | `Wiki 管理` | `知识库 Wiki` |
| caption | `schema / 信号 / 历史` | `录入 / 审核 / 问答` |
| eyebrow | `Knowledge Wiki` | `Knowledge Wiki`（不变）|
| title | `Wiki 管理` | `知识库 Wiki`（与 label 对齐）|
| subtitle | `管理知识库领域 schema、缺口信号与切片修订历史。` | `录入与审核 AI 的已验证知识内容（导入、问答、待评审），并管理领域 schema、缺口信号与修订历史。` |

改后两频道文案形成清晰对照：内容资产 caption「话术 / 素材」、知识库 Wiki caption「录入 / 审核 / 问答」；两条 subtitle 各自点明职责且互相指认（内容资产 subtitle 末句指向知识库为事实口径来源）。

## 4. 设计系统合规（docs/frontend-design-system.md）

- 纯文本字段改动，**不涉颜色**（蓝仅主操作 / teal 仅 AI，本次不碰）。
- caption/eyebrow 维持短词组，与现有同类频道字数量级一致（导航文案模型 design-system.md:70-83）。
- 不新增组件、不动 CSS、不动 .module.css 绑定。

## 5. 验证

1. `cd frontend && npx tsc --noEmit` → 0 error（channels.ts 是 TS，字段类型不变，纯字符串值改动）。
2. `cd frontend && npm run build` → 成功（CSS 存活；无组件改动）。
3. `bash scripts/check-no-human-takeover.sh` → 0 violations（新文案无禁用词；核对"真人/转人工/接管"等不出现——本次文案均为"知识/素材/话术/录入/审核"中性词）。
4. 人工目视（若起得了 dev server）：左侧导航「知识」组下，内容资产 caption 显「话术 / 素材」、知识库 Wiki 显「录入 / 审核 / 问答」；点进各频道顶部 title/subtitle 与新文案一致。
5. 既有前端测试不回归（channels 文案若被某快照测试断言，更新该快照——探索未发现 channels label 的快照断言，实现时 grep 确认 `素材知识`/`Wiki 管理` 无测试硬编码引用）。

## 6. 不做（YAGNI 边界）

- 不重排导航分组、不新建分组、不移动"专属顾问"。
- 不改频道 id、路由、组件、store、后端、任何业务逻辑。
- 不改知识审核的双入口现状（stat;统一收件箱 + 知识库 Wiki 内 ReviewView 并存是既有设计，超范围）。
- 不动内容资产页 / 知识页内部任何文案（页面内已有"口径一致"提示，足够）。

## 7. 执行

改动极小（单文件 channels.ts、约 6 个字符串字段）。不必 Subagent-Driven，主会话直接改 + 三步验证（tsc / build / lint）即可。基于最新 origin/main 开分支（本 worktree 落后于 main），推送开 PR → CI 绿后合并。
