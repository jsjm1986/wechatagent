# 销售素材文件发送能力 — 设计文档

- 日期：2026-06-21
- 范围：Phase 1 用户（私聊）运营域
- 状态：设计已获用户分节确认，待最终审阅

## 1. 背景与问题

销售场景下，运营产品有海报/宣传图片、PDF、Word、报价表格等**原始资料文件**。运营希望 AI 在销售对话中能把这些文件**直接发给微信客户**（如已购买用户发使用说明书、未成交用户发报价单）。

经四层实证核查（MCP 发送层 / Agent 决策网关 / 知识库存储导入 / 前端），当前系统是**纯文本闭环**，四层都不支持发文件：

- **发送层**：所有出站唯一落点是 MCP `message_send_text`，payload 仅 `{recipient, content}`，无 media 字段（`src/agent/gateway.rs:1864-1879`）。
- **决策层**：`AgentDecision` 唯一承载发送内容的字段是 `reply_text: String`（`src/agent/types.rs:94`），无 attachments/media/files 字段；`OutboxEntry` 也只有 `content: String`（`src/models.rs:2334`）。
- **存储层**：知识库导入 PDF/图片是"抽取成文本"模式（PDF 抽文字、图片 vision 转文字描述），原始文件用完即弃（`src/routes/knowledge/import.rs:501-510, 613-703`）；不支持 Word；无任何对象存储。现有 `ContentAsset` 素材库（`src/models.rs:669-685`）只存 url/media_id 字符串、不接收文件上传，且 Agent 决策时只读 `kind/title/body`、不读 url/media_id（`src/agent/decision.rs:1039-1053`）。
- **前端**：唯一文件上传入口是知识导入向导（PDF/图片→转文本草稿，文件不留存）；对话消息类型 `Message` 只有 `content` 文本，只渲染 `<p>`。

**关键修正（来自用户）**：MCP server（GeWe，自有服务器）**端能力没有问题**。其公开工具清单（`http://117.72.54.28:3001/mcp-guide.html`）确认私聊侧支持富媒体发送：

| 工具 | 用途 |
|---|---|
| `media_upload_base64` | 上传文件/图片/视频，返回 `media_id` |
| `media_get` | 取回媒体 |
| `message_send_text` | 发文本（当前唯一在用） |
| `message_send_image` | 发图片 |
| `message_send_file` | 发文件（PDF/Word/Excel 等） |
| `message_send_video` | 发视频 |

页面 Media 段明确：**文件/图片/视频先调 `media_upload_base64` 拿 `media_id`，再发送**。

因此能力缺口**全在 WechatAgent 本仓侧**，而非 MCP server。本设计聚焦打通本仓四层。链接卡片/小程序：MCP 私聊侧未见对应工具名（清单里 `moment_post_link` 是朋友圈发布工具），故发送层做成"媒体类型→工具名"可扩展映射，确认私聊工具名后再接入。

## 2. 决策记录（澄清结论）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 发送决策权 | **AI 自主为主**；高价值/高风险素材发送前可走现有「领导决策请示通道」(escalation) 兜底 |
| D2 | 素材归属 | **改造现有 `ContentAsset`** 为真正的独立素材库（不新建集合，不挂知识切片） |
| D3 | 文件二进制存储 | **服务器本地磁盘**（DB 存路径+元数据），适配 117 单机部署 |
| D4 | AI 选材机制 | **标签 + 候选清单注入 prompt**（方案 A），复用 `system_taxonomies` 与 `load_context_assets` 模式 |
| D5 | 覆盖媒体类型 | 图片/文档/视频先接入；链接卡片/小程序留可扩展占位（媒体类型→MCP 工具名映射表） |
| D6 | 治理模型 | 素材是**人类制作+标注、内容可信、免 AI 核验**；AI 只负责**识别发送时机 + 选对文件**；发送依据来自上传时人类填的标注与触发提示词 |
| D7 | 知识库冲突 | 见第 3 节"双轨并行"；冲突三不做系统强制同步，前端上传时给小提醒标识，一致性归管理员 |

## 3. 总体架构与职责边界

把 `ContentAsset` 从"只存 URL 字符串"改造成"存原始文件 + 带发送标注"的素材资产库，打通四层：**存储 → AI 决策选材 → MCP 富媒体发送 → 前端管理**。

### 3.1 职责边界（地基）

| | 知识库（已有） | 素材库（本次改造） |
|---|---|---|
| 管什么 | AI **怎么说**（话术轨） | AI **发什么文件**（交付轨） |
| 内容载体 | 抽取后的文本 chunk | 人类制作的原始文件 |
| 内容可信性 | AI 不自我核验（grounding 红线） | 人类已把关，免 AI 核验 |
| AI 职责 | 用文字回答，受 grounding 约束 | 判断发送时机 + 选对文件 |

一句话：**知识库管 AI"怎么说"，素材库管 AI"发什么文件"**。

### 3.2 双轨并行模型（解知识库冲突）

每次决策两条轨道**并行评估**，非二选一：

- **话术轨（知识库驱动）**：AI 永远说点什么（哪怕一句），保持对话温度。
- **交付轨（素材库驱动）**：AI 判断此刻要不要附文件。

重叠时由素材的**表达偏好（expression_pref）**决定文字详略：

- `file_primary`（文件为主，如报价单/说明书/企业介绍 PDF）：发文件 + 文字只做简短引导，**不复述文件内容**。
- `file_support`（文件佐证，如医美客户案例图）：正常话术 + 文件加分。
- **无匹配素材**：自然退化为纯知识库话术。

验证用例：

| 客户问 | 命中素材 | 表达偏好 | AI 行为 |
|---|---|---|---|
| "怎么用？"（已购买） | 使用说明书.pdf | file_primary | 发 PDF + "给您说明书，按第3页装就行" |
| "多少钱？"（未成交） | 报价单.xlsx | file_primary | 发 Excel + "这是报价，三个套餐您看下" |
| "效果真好吗？"（医美意向） | 客户案例图 | file_support | 话术讲效果 + 附案例图佐证 |
| "你们靠谱吗？" | 无匹配 | — | 纯知识库话术回答 |

### 3.3 三类知识库冲突的处置

- **冲突一（同一资料双重身份）**：知识库存文本供 AI 读懂、用话术回答；素材库存原文件供 AI 发送。靠"双轨并行 + expression_pref"消解重复（file_primary 时文字不复述文件）。
- **冲突二（grounding 边界）**：素材文件本身免 grounding（人类已把关），但伴随的 `reply_text` 仍照常走五闸门——堵住"发文件绕过 grounding"的后门。
- **冲突三（内容不一致）**：不做系统强制双向同步。前端上传时给小提醒标识（"若知识库已有同内容文本，请确认两边一致"），一致性是管理员责任（符合"人类把关"定位）。

## 4. 数据模型

改造 `ContentAsset`（`src/models.rs:669-685`），**新增字段全部 `Option` + `#[serde(default)]`，向后兼容**——现有朋友圈素材行不受影响（项目兼容红线）。

```rust
pub struct ContentAsset {
    // ===== 现有字段，保留 =====
    id, workspace_id, account_id, kind, title, body, tags,
    url, media_id, usage_scene, created_at, updated_at,

    // ===== 新增：文件资产本体 =====
    media_type:        Option<String>,  // "image"|"file"|"video" → 决定走哪个 MCP 工具
    file_path:         Option<String>,  // MEDIA_STORAGE_DIR 下相对路径（二进制落盘处）
    file_name:         Option<String>,  // 原始文件名（发给客户时显示，如"产品报价单.xlsx"）
    file_size:         Option<i64>,     // 字节数
    mime_type:         Option<String>,  // "application/pdf" 等
    file_sha256:       Option<String>,  // 内容指纹，去重 + 完整性校验

    // ===== 新增：发送标注（人类上传时填，给 AI 看的发送依据）=====
    sendable:          Option<bool>,    // 是否"可发送素材"（区别于朋友圈草稿）。默认 false
    send_trigger_hint: Option<String>,  // 自然语言触发提示词
    target_stages:     Option<Vec<String>>, // 适用客户阶段（来自 system_taxonomies）
    expression_pref:   Option<String>,  // "file_primary"|"file_support"
    requires_principal_approval: Option<bool>, // 发送前是否走领导请示通道。默认 false

    // ===== 新增：审核状态（沿用知识库"AI 不自我核验"红线语义）=====
    review_status:     Option<String>,  // "draft"|"approved"。仅 approved+sendable 才允许 AI 发
    review_note:       Option<String>,
}
```

要点：

1. `sendable=true` 且有 `media_type` 是"可发送素材"的分水岭；老朋友圈素材 `sendable=None/false` 在选材时被过滤，不会误发。
2. `media_id` 字段复用为 MCP 上传后的缓存（带 TTL，失效重传）。**不依赖 media_id 必然可复用的假设**——设计成"可缓存、失效则重传"的容错形态（MCP media_id 有效期/复用性待 server 侧确认，容错形态对两种情况都成立）。
3. `file_path` 存相对路径，根目录由 `MEDIA_STORAGE_DIR` 配置。
4. 复用 `content_assets` 集合，不新建表。

## 5. 发送链路（存储 → 上传 → 发送）

### 5.1 文件上传入库

新增 `POST /api/content-assets/upload`（multipart），复用现有 PDF 导入已验证的 `axum::extract::Multipart` 模式（`src/routes/knowledge/import.rs:428` 先例）：

```
接收 multipart（file + 标注字段）
  → 校验 mime/大小（配置上限，默认 50MB）
  → 算 sha256，按指纹去重
  → 写入 MEDIA_STORAGE_DIR/{workspace}/{sha256前2位}/{sha256}.{ext}
  → 落库 ContentAsset{ file_path, media_type, sendable=true, review_status="draft", ...标注 }
```

**安全（OWASP 文件上传）**：

- 路径穿越防护：落盘文件名一律用 `sha256+扩展名`，绝不把 `file_name` 拼进磁盘路径；原始文件名只存 DB 字段用于展示。
- mime 与扩展名做白名单校验。
- 大小上限由配置约束。

### 5.2 发送网关改造

不破坏现有 `send_outbound_message`（`src/agent/gateway.rs:1864`，写死 `message_send_text`）。新增并列的媒体发送函数，用**媒体类型→MCP 工具名映射表**（不写死）：

```rust
async fn send_outbound_media(state, contact, asset: &ContentAsset) -> AppResult<Value> {
    let media_id = ensure_media_uploaded(state, asset).await?; // 缓存命中直接用，否则读盘 base64 → media_upload_base64
    let tool = match asset.media_type.as_deref() {
        Some("image") => "message_send_image",
        Some("file")  => "message_send_file",
        Some("video") => "message_send_video",
        _ => return Err(...),  // 未知类型不发
    };
    logged_call_for_account(state, &contact.account_id, tool,
        json!({ "recipient": contact.wxid, "mediaId": media_id })).await
    // 落 conversation_messages（标记 msg_type=media，供前端渲染）
}
```

链接卡片/小程序：映射表留占位，确认 MCP 私聊工具名后加一行即可，不改架构。

### 5.3 经过 outbox（保持幂等红线）

CLAUDE.md 铁律：approved 发送必须先进 `agent_send_outbox` 拿幂等键再调 MCP。素材发送同样过 outbox——给 `OutboxEntry`（`src/models.rs:2334`）新增 `Option` 字段标识"媒体发送 + asset_id"，dispatcher 据此调 `send_outbound_media` 而非 text。

**文字 + 文件一起发**：作为**两条独立 outbox 条目**先后发出（按 expression_pref 定序），各有独立幂等键。任一条失败可独立重试，不会因文件传失败回滚已发文字。

## 6. AI 决策选材（双轨并行落地）

### 6.1 候选素材注入 prompt（方案 A）

新增并列加载器（现有 `load_context_assets` 只读 `kind∈[text,faq,...]` 且不读 file/media 字段，`src/agent/decision.rs:1025`）：

```rust
async fn load_sendable_assets(state, account_id, customer_stage) -> Vec<AssetCandidate> {
    // 过滤: sendable=true AND review_status="approved"
    //       AND (target_stages 命中当前 customer_stage 或为空)
}
```

注入形态（每素材一行，给 AI 选材依据）：

```
可发送素材（按需选择，没有合适的就不发）：
- [id:a1] 产品报价单.xlsx | 阶段:意向,未成交 | 表达:文件为主
  触发提示:客户问价格/表达购买意向但未下单时发
- [id:a2] 使用说明书.pdf | 阶段:已成交 | 表达:文件为主
  触发提示:客户购买后问怎么用/怎么装时发
- [id:a3] 客户案例图 | 阶段:意向 | 表达:文件佐证
  触发提示:客户质疑效果时发图佐证
```

### 6.2 AgentDecision 扩展

`AgentDecision`（`src/agent/types.rs:80`）新增（`Option`+`default`，兼容）：

```rust
assets_to_send: Vec<AssetSendDirective>,  // 每个 = { asset_id, reason }
```

AI 在**一次决策**里同时输出 `reply_text`（说什么）+ `assets_to_send`（发什么文件），双轨并行，不多跑 LLM 轮次。prompt 指引（柔性、agent-first，不用词表硬匹配）：

- 仅从候选清单选，没合适的就空数组（不为发而发）；
- 选 `file_primary` 素材时 `reply_text` 只简短引导、不复述文件内容；
- `file_support` 素材正常话术 + 文件佐证。

### 6.3 发送时闸门（治"时机"不治"内容"）

AI 选出 `assets_to_send` 后，发送前过以下闸门（复用现有机制）：

| 闸门 | 作用 | 复用 |
|---|---|---|
| 准入校验 | 二次确认 asset `approved+sendable`，防 AI 幻觉出不存在/未审 id | 新增校验 |
| 频率/冷却/日上限 | 防刷屏猛发文件 | 现有 gateway 频控 |
| PressureRisk | 客户反感时不硬塞资料 | 现有五闸门之一 |
| 领导请示 | `requires_principal_approval=true` 走请示通道兜底 | 现有 escalation |

边界：素材内容免 grounding（人类已把关）；伴随的 `reply_text` 照常走五闸门（堵 grounding 后门）。

## 7. 前端

遵循 `docs/frontend-design-system.md` 企业白色基调，新增/改造三处：

1. **素材库管理页**（改造现有 content-assets 界面）：
   - 文件上传组件（`<input type="file">` + 拖拽，复用 `frontend/src/lib/api.ts:82` 已有但未被调用的 `postForm` multipart helper）；
   - 标注表单：媒体类型、适用阶段、触发提示词、表达偏好、是否需请示；
   - 冲突三小提醒标识（"若知识库已有同内容文本，请确认两边一致"）；
   - 图片缩略图预览、文件名/大小/类型、审核状态（draft/approved）。
2. **素材审核**：草稿态列表 + "标记为可发送(approved)"操作——人类把关入口。
3. **对话消息渲染**：`Message` 类型新增 `msg_type` 字段；媒体消息渲染成图片缩略图/文件卡片，让运营看到 AI 发了什么文件给客户（现仅渲染 `<p>{content}`）。

## 8. 配置与数据库

- **配置**（`src/config.rs` + `.env.example`，全带默认值）：
  - `MEDIA_STORAGE_DIR`（默认 `./media`；生产 117 为 `/opt/wechatagent/media`）
  - `MEDIA_MAX_FILE_SIZE_MB`（默认 50）
  - `MEDIA_ID_CACHE_TTL_HOURS`（media_id 复用 TTL）
- **数据库**：复用 `content_assets` 集合与现有 typed accessor。新增索引 `{workspace_id, sendable, review_status}`（选材查询）、`{file_sha256}`（去重），走 `ensure_indexes`。
- **存储目录**：`MEDIA_STORAGE_DIR` 启动时确保存在。

## 9. 测试策略

遵循项目铁律（纯函数确定性测试为主、不接受 skip 假绿、新增只叠加不删旧维度、不过拟合单条样本）：

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | 媒体类型→工具名映射、候选素材过滤、路径安全（防穿越）、sha256 去重 | lib 单测 |
| 准入闸门 | 未审/不可发素材被过滤、AI 选了非法 id 被拦 | lib 单测 |
| prompt 注入 | 候选清单形状、expression_pref 详略指引 | 纯函数测 |
| 发送链路 | upload→media_id→send 数据流、outbox 两条目幂等 | 集成测（CI，testcontainers） |
| 向后兼容 | 老 content_assets 行（无新字段）能正常反序列化、不被误选为可发送 | lib 单测 |
| 真实 LLM | 给定客户阶段+提问，AI 是否选对素材（报价单/说明书两例） | CI real-llm |

baseline 不回归（lib≥350、PBT≥33）；新增测试只 append。

## 10. 不做（YAGNI / 范围外）

- 不接入对象存储（S3/OSS/MinIO）——单机本地磁盘足够。
- 不做知识库↔素材库强制双向同步。
- 不在本期接入链接卡片/小程序发送（留映射占位）。
- 不改 Phase 1 之外的群/朋友圈运营域（朋友圈素材仍用 ContentAsset 旧语义，与新字段共存）。
- 不为发文件而发——AI 无匹配素材即空数组。

## 11. 风险与未决

- **MCP media_id 复用性/有效期**：文档未明确，设计用"可缓存+失效重传"容错形态规避，不阻塞实现。MCP 各发送工具的精确入参字段名（`mediaId` vs `media_id` 等）以 server 侧 `tools/list` 实际 schema 为准，实现时对齐（用户将负责 MCP 侧）。
- **文件上传安全**：按 OWASP 规范（路径穿越、mime 白名单、大小限制）。
- **媒体消息落 conversation_messages 的形态**：需与现有 `ConversationMessage` 兼容（新增 `Option` 字段，不破坏纯文本读路径）。
