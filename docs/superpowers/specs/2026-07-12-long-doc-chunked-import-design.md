# 长文档分块导入设计（真实修复 import-preview 超时/截断）

日期：2026-07-12
状态：已获用户口头批准，待实现

## 问题

`POST /api/operation-knowledge/import-preview` 把整篇长文档一次性喂给 LLM 抽取结构化 JSON。29KB 完整文档会失败：HTTP 200 但 `chunks=0`。

**根因（三方确认：亲读代码 + 生产 llm_call_logs + 独立 subagent 交叉验证）**：瓶颈是**输出生成**，不是 prompt 体积。
- system prompt 是一句话（`import.rs:134`）；user 模板 ~2KB（`import.rs:58-125`）；发送体只有 system+user，无隐藏注入（`agent/mod.rs:269` → `llm.rs:442-449`）；OpenAI 路径**无 max_tokens**。
- 生产实测：`prompt_tokens≈28-30k / completion_tokens≈6.3-7.1k / 耗时 166-311s`。28k 输入=那 29KB 正文本身（prefill 并行、快）；真正慢的是 6-7k tokens 抽取结果的**自回归串行生成**（受限端点 ~2 线程 ≈25-30 tok/s）。输出到一半 JSON 截断 → parse 后 chunks 为空。
- 附带隐患：import 超时被当可重试错误，最多重试 5 次（`config.rs:443`），单篇超大文档理论上拖到 ~25 分钟才最终失败。

## 目标

真实用户导入**任意大小**文档都能成功。不绕路（不靠人工分段、不靠单纯调大 timeout）。

## 方案：后端自动分块

只改 `import_operation_knowledge_preview`（`import.rs:127`）。前端 `runPreview` 的 `{document, items, chunks}` 响应契约**一字不变**，分块完全内部化，step2 逐条审阅体验不变。

**为什么不做"整篇 LLM 预加工再切"**：那一步要 LLM 输出一份和原文差不多大的新文档（≈8-10k tokens），比抽取的 6-7k 输出**更长、更慢、更易截断**——把刚修的输出瓶颈请回来还放大了。抽取本身就是"把杂乱原文重整成干净切片"，清洗天然发生在每个小抽取调用里，无需额外的大输出调用。正确顺序是**先切分 → 每段抽取（自带清洗）**。

### 1. 切分（标题优先 + 字符回退，确定性、纯函数、可单测）

常量（按 `chars().count()` 计，适配中文）：
- `IMPORT_SINGLE_CALL_MAX_CHARS = 3000`：≤ 此值单次调用，**与今天完全一致（零回归）**
- `IMPORT_SEGMENT_TARGET_CHARS = 3000`：贪心打包目标
- `IMPORT_SEGMENT_HARD_MAX_CHARS = 5000`：超此值的单块按段落再切

算法 `split_import_content(content) -> Vec<String>`：
1. 总字符 ≤ SINGLE_MAX → `vec![content]`（零回归路径）。
2. 否则按 markdown 标题行（`line.trim().starts_with('#')`，对齐 `build_section_index`）切成原子块。
3. 贪心打包：连续块累加到 TARGET 就断开；单块超 HARD_MAX → 先 flush，再按段落（`\n\n`）窗口切（在段落断点断，不切句子中间）。
4. 无任何标题的纯长文 → 步骤 2 得到单块 → 步骤 3 的 HARD_MAX 分支按段落窗口兜底。
5. 空结果兜底 `vec![content]`。

### 2. 每段抽取（并行度 2，保序）

`futures::stream::iter(segments).map(|seg| generate_agent_json(template.replace(CONTENT, seg))).buffered(2).collect()`。
- 每段用同一 `LONG_IMPORT_PROMPT_TEMPLATE`，`{CONTENT}` 只放该段 → 每次输出小、不截断。
- 并发 2 匹配端点真实 ~2 线程，避免 tool_use 争用；`buffered` 保序。
- 无 RunBudget 约束（admin 路由非 agent run，`current_run_budget()` 返回 None，跳过）。
- 各段 user 内容不同 → LRU cache key 不同，无误命中。

### 3. 合并

- **chunks**：各段 chunks 数组按序拼接，逐条过 `normalize_operation_knowledge_preview_chunk`。
- **items**：各段 items 按序拼接，逐条过 `normalize_operation_knowledge_preview_item`。
- **document**：确定性合并各段 document 原始值（scalar 取首个非空；routingMap/riskNotes/productTags/businessTopics 取并集），再过 `normalize_operation_knowledge_preview_document`（rawContent/lineIndex/sectionIndex 仍从完整 `payload.content` 生成）。**不额外调 LLM 合并**，避免重蹈输出瓶颈。
- 单段 doc 时合并=恒等 → 小文档路径与今天字节等价。

### 4. 容错

- 单段失败（内部已含 5 次重试）→ 记 warning、跳过该段、继续其它段，返回部分 chunks。
- 全部段失败 → 返回 `AppError`（不吞成空 200）。
- 响应新增 `importReport: { totalSegments, succeeded, failed }`（前端忽略未知字段，服务端 `tracing::warn` 落日志）。超时→5 次重试放大问题因每段小、不超时而自然消除。

### 5. 红线不动

- D2 锚定：`integrity_report_for_preview` 仍对**完整原文**跑一次，每 chunk 的 sourceQuote 在全文锚定。
- "AI 永不自动 verify"：preview 只产 draft 预览，apply 路径的 `status=draft + integrity_status=needs_review` 强制不变。

## 测试

1. **单元测试**（纯函数 `split_import_content`）：小文档单段；带标题多段打包；超大块段落回退；无标题长文回退；空/纯空白兜底。
2. **端到端真实验证**（生产 117）：用完整 29KB 星零感 MD 走前端 Playwright → step1 粘贴 → runPreview → 断言 step2 出现多条 chunks（非 0）→ 应用 → DB 落库 draft/needs_review。反复跑确保稳定。

## 影响面

- 改一个文件 `src/routes/knowledge/import.rs`（新增切分/合并纯函数 + 重写 preview handler + 单测）。
- 无 DB schema 变化，无迁移，无前端改动（契约不变）。
