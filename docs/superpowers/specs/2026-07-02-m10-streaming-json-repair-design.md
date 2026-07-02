# M10 流式 JSON 解析缺 LLM-repair 兜底修复设计

> 日期：2026-07-02
> 分支：`fix/m10-streaming-json-repair`（从 origin/main 3f59771 切，含 M1 #90 / M4 #91）
> 来源：终极审判审计 M10（UPHELD Medium）

## 1. 漏洞描述（对最新代码 100% 亲验）

`LlmClient::generate_json_streaming_openai`（llm.rs:711-813）消费上游 SSE、累积 `delta.content` 到 `accumulated`，最后用 **`parse_json_content(&accumulated)?`**（llm.rs:805）解析出返回 `value`。

`parse_json_content`（llm.rs:1195-1227）是**三层确定性解析**：
1. 快路径 `serde_json::from_str`；
2. `repair_loose_json`（trailing comma / 未闭合）；
3. `extract_embedded_json`（从"推理 + JSON"混合体截首个可解析块）。

而非流式路径 `generate_json_with_usage`（→ llm.rs:824+）用的是 **`parse_or_repair`**（llm.rs:318-339），它在上述三层**之上**再加**第四层**：三层全失败时把整段脏文本回喂 LLM 修成合法 JSON（`repair_via_llm`，最多 `REPAIR_MAX_ATTEMPTS` 次）。

**同一个 `impl LlmClient`、同一个 `parse_or_repair` 已存在**，流式路径却没用它——两条路径的 JSON 鲁棒性不对称。

### 消费方不降级（原设计假设不成立）

流式路径注释（llm.rs:708-710）写：

> 单次尝试、不走 `generate_json_with_usage` 的重试循环 —— 流式一旦开始推 token，重试会导致前端重复/错乱；HTTP/1.1 + keepalive 已稳定链路，**失败直接上抛由调用方降级**。

但唯一调用链（`knowledge_agent::answer_streaming` → `generate_agent_json_streaming` → 本函数）的 HTTP 消费方 `sources_meta.rs:662-676` **并不降级到非流式**——它只把 `Err` 转成一条 SSE `error` 事件发给前端。所以「调用方降级」这个注释假设在流式路径上**从未实装**：脏 JSON 撞穿三层 → 整轮知识对话直接失败报错。

### 关键：加第四层 ≠ 重启流

注释真正要规避的是**重试 HTTP 流**（re-stream 会让前端 token 重复/错乱）。而 `parse_or_repair` 的第四层 `repair_via_llm` → `fetch_raw_text`（llm.rs:351）是对**已累积完的文本**发一次**独立**修复请求，**不向 `token_tx` 推任何 token**。即：token 已全部流完（前端预览已定），修复只在幕后重算返回值。注释的顾虑（re-stream 乱序）对第四层不适用——第四层安全且正是注释期望的"失败兜底"。

## 2. 根因

流式实现落地时内联了 `parse_json_content`（确定性三层），漏接了非流式路径共用的 async 第四层 `parse_or_repair`。是遗漏，非有意取舍——作者注释本身期望"失败降级"，只是把降级误寄望于调用方（而调用方未实装）。

## 3. 方案

### 方案 A（选定）：流式返回前改用 `parse_or_repair`

llm.rs:805：
```rust
let value = parse_json_content(&accumulated)?;
```
→
```rust
// M10：与非流式路径对齐——三层确定性解析全失败时再走第四层 LLM-repair
// （对已累积完的文本发独立修复请求，不向 token_tx 再推 token，不 re-stream）。
let value = self.parse_or_repair(&accumulated).await?;
```

`parse_or_repair` 第一步就是 `parse_json_content`（llm.rs:320）——**happy path 字节等价**（干净 JSON 直接返回，零额外 LLM 调用）。只有三层全失败时才多一次幕后修复请求。严格只增不减：可恢复的脏 JSON 从"整轮报错"变"修复后成功"，绝不劣化任何现有成功场景。

### 逐场景验证（已核实）

| 场景 | 当前（parse_json_content） | 方案 A（parse_or_repair） |
|---|---|---|
| 干净 JSON | 快路径返回 | 快路径返回 ✓ 字节等价、零额外调用 |
| trailing comma / 未闭合 | repair_loose_json 修复 | 同 ✓ 第二层，不触发第四层 |
| 推理+JSON 混合 | extract_embedded_json 截块 | 同 ✓ 第三层，不触发第四层 |
| **三层全失败的脏文本** | **整轮报错** | 第四层回喂 LLM 修复 → 成功 ✓ **新增兜底** |
| 第四层也失败 | —（撞不到） | 抛严格 json_decode 错（不吞噪声）✓ 与非流式同口径 |

### 否决方案 B（在流式循环里重试 HTTP）
正是注释所拒——re-stream 会让前端 token 乱序/重复。方案 A 不 re-stream，规避此坑。否决。

## 4. 核心改动

落点：`src/llm.rs` `generate_json_streaming_openai` 一行（llm.rs:805）。`parse_or_repair` / `repair_via_llm` 均已存在于同一 `impl LlmClient`，无新增函数、无签名变更（函数本就 `async`）。

**不动**：SSE 累积逻辑、`token_tx` 推送、usage 聚合、Anthropic 分支（llm.rs:870 已走非流式 `generate_json_with_usage`，本就含 `parse_or_repair`）、非流式路径、调用方。

## 5. 测试设计

`parse_or_repair` 的**第一层** `parse_json_content` 的三层确定性分支已有密集单测覆盖（快路径 / trailing comma / embedded），本次 happy-path 字节等价即由这些既有测试守住。

**第四层** `repair_via_llm` 需要真实 LLM 端点（对脏文本发修复请求），无法本地 hermetic 驱动——与非流式 `parse_or_repair` 的第四层同样只能由 real-LLM CI 覆盖。**诚实声明**：不新增需要 LLM 端点的假测试；本次是"让流式复用已被非流式路径验证过的 `parse_or_repair`"，正确性由（a）happy-path 字节等价、（b）`parse_or_repair` 是现成受信路径、（c）编译验证 保障。

### 验证
- `cargo build --lib` 无 error。
- `cargo test --lib` ≥ 350 passed / 0 failed（基线守住）。
- 禁词 lint 通过（改动纯技术、不涉禁词）。

## 6. 范围边界

- **只增不减**：happy path 字节等价，仅在三层全败时新增第四层兜底。
- **过拟合红线**：不改任何解析阈值/层逻辑；修的是让流式复用非流式已有的 `parse_or_repair`，让两路 JSON 鲁棒性对齐。
- **YAGNI**：不碰 SSE 累积 / token 推送 / retry 循环 / Anthropic 分支。
