# 知识库前端适配:可信度治理驾驶舱 — 设计文档

> 日期:2026-06-07 · 状态:设计已定稿(mockup 经用户逐屏确认)· 下一步:writing-plans

## 1. 背景与问题

后端知识库子系统在过去一段时间落了一批以「**可信度治理**」为核心的新业务逻辑(D2 闸、grounding 承诺背书、completeness 5 维认知矩阵 + answeringMode、auto-verify、对话补库、字段锁、usage_stats 等),但前端知识库 UI(`frontend/src/features/knowledge/index.tsx`,6526 行)是某个时点的「行为快照」,自下沉后只有 2 条提交(下沉 + CSS 收口),从未跟随后端业务变更更新。

**前提澄清(代码核实):** 前端架构本身已重构完毕并合并进 main(App.tsx 仅 151 行,知识库 UI 已物理下沉到 features/knowledge)。本次**不是**架构迁移,而是**业务语义适配**——让前端把后端这批新业务逻辑「显形」出来。

**三条对齐链审查结论(代码为准):**
- **端点层**:80 个端点前端调了 72 个(90%),8 个孤儿端点(auto-verify、repair、open-evidence、usage、relate DELETE 等后端有能力前端无入口)。
- **数据层**:大面积 schema 漂移。最严重 `CompletenessView`(index.tsx:4264-4267)只认旧的 `perWikiType/overall`,而后端 completeness 返回 `answeringMode` + 5 维认知矩阵(knowledge.rs:3286-3293,3376)——整个矩阵在 UI 上被静默吃掉。chunk 富字段(chunk_type/provenance/valid_from-to/usage_stats/dynamic_confidence/locked_fields)前端从不读。
- **业务逻辑层**:红线在前端无交互。D2 闸、distortion_risks 降级痕迹、承诺背书、字段锁、answeringMode 认知闸在前端要么是数据黑洞,要么是交互真空。

## 2. 设计目标与原则

**目标:** 让前端知识库 UI 把后端「可信度治理」的新业务逻辑完整显形,且**普通小白运营也能理解使用**。

**核心设计原则(用户确立,贯穿全设计):**
1. **不言自明,最大幅度降低认知难度** — 把后端的「数据态」(verifiedFact/methodologyOnly/pendingDraft 等)翻译成运营的「后果」(能讲/得审/会被拦),不让运营解码字段。
2. **大白话,零行话** — 后端术语全部翻译:`source_quote`→「这话哪来的」、`verify`→「让 AI 可以用这条」、`pricing`→直接不显示英文、`integrity_status`→表情 + 大白话状态。整页只回答一个小白能懂的问题:「这条知识,AI 现在能不能拿去跟客户说?」
3. **视觉严格复用现有设计系统** — `frontend/src/components/ui/tokens.css` 的 6 语义色 token + StatusBadge/MetricCard 等原子组件;实色白卡 + inset 高光 + hover 上浮;呼吸只在「进行中(绿)/高风险(红)/当前档位(蓝)」克制出现;**不用左缘竖色杠**(违反「玻璃只做点缀」原则)。
4. **守住红线,但在交互上收敛** — 「AI 永不自动 verify」「D2 闸」「对话只起草不放行」等红线不破,但把后端的多步机制(chat_apply + verify)在 UI 上收成运营的单一意图。

## 3. 信息架构落位

将现有 **Steward「治理」模式升级**为本设计的「治理驾驶舱」(它本就承载 verify/质量信号,语义最贴)。Today/Explore/Atlas 三模式不动,整体导航不重构。改动收敛在一个模式内,增量迁移。

## 4. 已定稿的屏(mockup 在 `.superpowers/brainstorm/1455-1780766245/content/`)

### 4.1 治理驾驶舱 · 主屏(`cockpit-final.html`)

三层结构:

- **顶部 · answeringMode 极简仪表**:一颗呼吸蓝点 + 一句话状态(「可安全讲产品」)+ 细进度条 + answeringMode code + 一句解读(「距完全支撑差一步:定价有 2 条草稿待审,有待审草稿就绝不宣称完全支撑」)。把后端隐藏的 `clamp_answering_mode` 认知闸翻译成运营看得懂的话。
- **中部 · 5 维大白话裁决**:capability/pricing/caseEvidence/effectClaims/deliveryBoundary 五维,每维一行 = 维度名 + StatusBadge 状态徽章(可放心讲/N 条等你审/空白·高风险/只能讲思路)+ 大白话后果 + 直达动作按钮。状态只靠 StatusBadge 的语义圆点承载(绿点呼吸=可放心讲、红点轻吸=高风险、琥珀=待审、靛=仅方法论);遮住颜色光读文字也懂。效果数据维度缺失时直接点破「AI 一旦对客讲成功率/见效/回款会被安全闸当场拦下」。
- **下部 · 治理待办**:待审草稿数 / D2 降级数 / 知识缺口数三个 MetricCard 计数 + auto-verify 批量入口(白卡描边,非渐变)。把原本散在 Steward 各子 tab 的待办收拢成一条直达队列。

### 4.2 审核 + 对话双栏(`review-chat-v2.html`)

从主屏裁决行/待办下钻后的单条知识处理屏,左右双栏:

- **左栏 · 裁决 + 放行**:顶部一句话裁决(绿环「改完即可放行」/ 琥珀环「还差一步」,结论先行)+ chunk 内容(大白话「这条说的是」)+ 「原话出处」引用块 + 放行前检查(折叠,默认不打扰)+ 承诺背书提示(🛡️「放行后成为 AI 对客报价的依据」)+ 单一生效动作。
- **右栏 · 对话工坊**:标题写明边界「只动这条 · 改完仍由你放行」。运营自然语言说 → AI 走 `update_chunk` 改这条草稿(白名单字段)/ 补 source_quote / 调 `verify_anchor` 工具核对来源。
- **对话与左栏联动**:对话改完,左栏**实时预览**改后的样子(变化处高亮闪现)+ 出现「AI 刚帮你改了 X 处,应用?」预览条。运营看着左栏改后预览决定,不在右栏读 diff 脑补。

**业务逻辑依据(代码核实):** `ChatTurnRequest.attachments` 可携带 chunk_id(knowledge.rs:4222-4232),`update_chunk` intent 针对指定 chunk 改白名单字段(5915-5927);chat_turn 产 pending patch、chat_apply 才落库(4576);对话**不能** verify(无任何 intent 走 verify 路径,chat_apply 强制 draft+needs_review,5860-5862)。

### 4.3 单一「让 AI 可以用这条」动作(`single-action.html` / `plain-language.html`)

合并后端的 apply + verify 为运营心智中的**单一动作**:

- 对话阶段的修改只是**预览**(不落库);运营看左栏预览 OK → 点一次「让 AI 可以用这条」,前端**顺序调** chat_apply → verify 两端点。
- **红线不破**:verify 仍过 D2 闸。若 source_quote/source_anchors 未补齐,生效键**自动禁用**并用大白话说明还差什么(「不知道这话从哪来的,怕记错或被人改过,得先弄清谁定的、写在哪」)+ 给出路(「让 AI 帮我查 / 标记为口述无原文」)。补齐后生效键自动亮起。
- **实现注意**:生效键 = 两次顺序端点调用,任一失败需回滚提示;verify 被 D2 闸拒(4xx)时不报错,而是转成「还差 X 才能生效」的禁用态。

**代码依据:** `apply_update_chunk` 改完强制打回 draft+needs_review(knowledge.rs:5954-5955);verify 独立端点 + D2 闸(569、chunk_verify_gate_reason)。

### 4.4 auto-verify 批量屏(`auto-verify.html`)

主屏 auto-verify 入口下钻。**定性:运营主动发起、AI 帮筛,不是系统自动放行。**

- **筛之前定两件事**(对应后端三参数,全翻译):把关松紧(`confidence_threshold` 默认 7,宽松/适中/严格)+ 留一批我复查(`human_audit_sample_rate` 默认 0.1,翻译成「即使 AI 标通过也随机留 10% 让我看」)+ 筛多少条(`limit` 默认 50,50/100/全部)。
- **结果分三堆**(对应后端五计数,合并成三类):AI 觉得没问题(verified)/ 留给你复查(needs_human_audit)/ AI 没把握没动(needs_review + rejected)。

**红线说明(代码核实):** auto-verify 是**唯一一条 AI 判定可直达 verified 的路径**(knowledge.rs:909),但用 D2 闸(必须 quote+anchor,806-812)+ confidence 阈值 + 人工抽样三重约束兜底。它与「AI 永不自动 verify」红线的调和:红线精确含义是「ingest/import 入口永不自动 verify」(机器抽取不可信);auto-verify 是运营授权的批处理,**主体是运营**。UI 必须传达「是我让 AI 帮我筛,不是 AI 替我做主」。

## 5. 并入已有屏 · 不单独画 mockup

- **chunk 富字段显形**:usage_stats(用了多少次/被拦多少次)、valid_from-to(时效)、dynamic_confidence、locked_fields(字段锁,编辑时禁用 + 提示)——并进 4.2 审核详情卡的折叠区。
- **导入向导「全是草稿」告知**:沿用现有导入向导,导入完成后明确告知「都需你逐条放行」并一键跳进驾驶舱待办。对应红线:所有 ingest/import 入口强制 draft+needs_review。
- **知识缺口 gap signals**:现有 LintView 已实现 8 类信号树,只需接进驾驶舱待办、措辞大白话化。

## 6. 数据契约修复(实现必做)

适配落地时必须修复的 schema 漂移(前端类型 → 后端真实响应):

| 前端类型 | 现状(旧) | 必须改成(后端真实) | 锚点 |
| --- | --- | --- | --- |
| `CompletenessView` | `perWikiType[]/overall` | `answeringMode` + `coverage{5维×{verifiedFact,methodologyOnly,pendingDraft,state}}` + `gaps[]` | knowledge.rs:3286-3293,3376 |
| `IntegrityReportView` | `contested/sourceOrphan`(后端无) | `verified/rejected/items[]` | knowledge.rs:1152-1154 |
| `ReviewChunkItem`/Inspector | 缺富字段 | 补 `chunkType/provenance/validFrom/validTo/usageStats/dynamicConfidence/lockedFields/confidenceScore` | models.rs OperationKnowledgeChunk |

## 7. 红线清单(本设计必须守住)

1. **AI 永不自动 verify**:ingest/import/chat 入口一律 draft+needs_review;UI 不得让运营误以为「导入/对话即生效」。
2. **D2 闸**:verified 必须 source_quote + source_anchors 双非空;生效键在缺失时禁用,不让运营点了被后端 4xx。
3. **对话只起草不放行**:对话工坊能改/补 chunk,但放行始终是运营的独立确认动作。
4. **承诺背书**:效果类产品声明需 verified 背书;审核时提示运营「放行后 AI 会拿这条对客」。
5. **auto-verify 主体是运营**:UI 表达「AI 帮筛」而非「自动放行」,保留人工抽样复核。

## 8. 非目标(YAGNI)

- 不重构整体导航(只升级 Steward 一个模式)。
- 不改后端业务逻辑/端点(纯前端适配 + schema 对齐;若发现后端 bug 另案)。
- 不动 Today/Explore/Atlas 三模式。
- 不引入新的设计 token 或组件库(复用现有 components/ui)。
