# F-016 报告 — gap_signal description 中文化，收件箱不再泄漏技术串

## 背景
统一收件箱条目预览直接渲染后端 `gap_signal.description`，运营看到 `cited=0`、
`opened_bodies=`、`blocked_unverified_product_claim`、`split`/`reclassify`/`verify`
等英文技术串，看不懂。定位到 3 处硬编码 description 文案。

## Read/grep 亲验证据
- `src/agent/knowledge_agent.rs:1751-1756`（recall_miss 分支）：`format!("本次召回 cited=0（truncated={}，opened_bodies={}）…", result.truncated, result.rounds_used)`
  - **确认 opened_bodies bug**：文案写 `opened_bodies=` 但第二个填充实参是 `result.rounds_used`（轮次数），名不符实。
- `src/agent/knowledge_agent.rs:1776-1781`（recall_low_yield 分支）：`format!("本次 open 正文 {} 个但仅 cite {} 个…该 split / reclassify。", opened.len(), cited_count)`
- `src/knowledge_wiki/gap_signals.rs:446-449`（recall_miss_from_product_block）：`"产品宣称被 blocked_unverified_product_claim 拦截：… verified chunk … verify … search_queries …"`
- 复用候选 label 函数：
  - `src/agent/escalation/labels.rs:7` `blocked_status_zh()` — `pub(crate)`，`blocked_unverified_product_claim` → "产品说法未经核实"
  - `src/knowledge_digest/labels.rs:5` `block_reason_zh()` — `pub(crate)`，同枚举 → "产品说法未经核实"
  - **两者均在私有 `mod labels;` 下（escalation/mod.rs:8、knowledge_digest/mod.rs:31）**，从 `knowledge_wiki` 不可达，跨模块复用需放宽模块可见性（超出「只改 3 处文案」的范围）。且此处 reason 是编译期常量（该分支就是 product_block），无需运行时映射函数。故**内联采用与 `blocked_status_zh` 完全一致的中文措辞"产品说法未经核实"**，口径统一、零可见性改动。
- description 无机器消费：`signal_dedup_key`（gap_signals.rs:470）去重键只读 `kind` / `title` / `affected_chunk_ids`，不读 description；grep 全仓无测试断言这些 description 文案。改文案无副作用。
- `tests/knowledge_agent_pbt.rs:490-519` 只断言 `affected_chunk_ids ⊆ opened` 与 `kind`，不碰 description。

## 3 处改动前后文案
1. **recall_miss**（knowledge_agent.rs）
   - 前：`本次召回 cited=0（truncated={}，opened_bodies={}）。疑似目标知识缺失、粒度过粗或放置错位，待运营质检定位补/拆/重分类。`
   - 后：`本次没检索到可引用的知识。疑似目标知识缺失、粒度过粗或放置错位，待运营质检定位后补录、拆分或重新归类。`
   - opened_bodies bug 处理：直接**删除**这个名不符实的内部计数（原本填的是 rounds_used，翻成中文只会把错误语义固化给运营），不保留任何内部诊断量。

2. **recall_low_yield**（knowledge_agent.rs）
   - 前：`本次 open 正文 {} 个但仅 cite {} 个。原子可能过粗或放错文档，待运营质检判断该 split / reclassify。`
   - 后：`本次翻阅了多条知识正文，可真正引用的却很少。知识条目可能粒度过粗或放错文档，待运营质检判断是否拆分或重新归类。`
   - 折叠掉内部计数（open N / cite M），`split`/`reclassify` → 拆分/重新归类。

3. **product_block**（gap_signals.rs recall_miss_from_product_block）
   - 前：`产品宣称被 blocked_unverified_product_claim 拦截：本 run 引用的知识切片里没有任何 verified chunk 背书该产品声明。待运营据 search_queries 里的客户问句对话式补录 / verify 相关知识，使该缺口可被闭环修复。`
   - 后：`产品说法未经核实被拦截：本次回复引用的知识里，没有任何已核实的知识背书该产品说法。待运营根据下方客户问句对话式补录、核实相关知识，使该缺口可被闭环修复。`
   - 裸枚举 `blocked_unverified_product_claim` → "产品说法未经核实"（与 blocked_status_zh 口径一致）；`run`/`verified chunk`/`verify`/`search_queries` 全部中文化。

## 硬约束遵守
- 只改这 3 处 description 文案，未动 label import（内联措辞，无新增引用）、未改机器逻辑（去重键/落库/消费链）、未改数据结构。

## 验证结果
- `cargo check`：Finished，0 error。
- `cargo test --lib`：**1913 passed; 0 failed**（基线 ≥350，未回退）。
- 无测试断言旧英文文案（grep 已确认），无需裁决测试断言。

## commit
见 git log（提交信息：`fix(knowledge): gap_signal description 中文化,收件箱不再泄漏技术串(F-016)`）。
