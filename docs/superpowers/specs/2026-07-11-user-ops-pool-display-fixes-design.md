# 用户运营池显示/内容问题修复

**日期**：2026-07-11
**状态**：设计讨论中，待用户复审
**范围**：ContactsView（智能模式运营池）三个已 Playwright 复现的显示/内容问题 —— tab 布局畸形、消息预览吐 XML、系统号/媒体号混入池。跨前端 CSS + 后端 preview + 判据扩展 + 一个新端点。

## 背景（Playwright 亲验复现，before 截图 pool_before.png）

运营池上线后用户截图暴露三个问题，本地 harness 复现确认：

1. **三档 tab 挤成畸形椭圆**："待启用 10" 文字竖排换行成一个变形的圆角框。
2. **消息预览显示原始 XML**：福建经济广播/福州晚报显示 `<msg> <appmsg appid="" sdk...`，微信团队显示 `<sysmsg type="functionmsg"> <f...`。
3. **系统号/媒体号混入池且带「启用 Agent」按钮**：微信团队(weixin)、朋友推荐消息(fmessage)、福州晚报/海峡都市报/福建经济广播 等出现在运营池，还能点「启用 Agent」——它们不是能运营的私聊真人。

## 根因（全部 100% 亲验 file:line）

1. **CSS 布局**：`.panelHead`（`frontend/src/styles.css:444`）是 `display:flex; align-items:center; justify-content:space-between`，把 `poolHeadText`（标题 + 两行长副标题「主动来找过你的人…」「区别于通讯录…」）和 `.segmented`（3 档 tab）放**同一横行**。智能模式左栏（`userCockpitGrid` 窄列约 360px）里，长副标题占满横向空间，`.segmented` 被挤到最小宽度 → tab 文字竖排换行成畸形椭圆。
2. **预览 XML**：`list_contacts`（`src/routes/contacts.rs:212-213`）对最近入站消息无条件 `truncate_preview(&msg.content, 30)`。而 appmsg（链接/文件/小程序）、sysmsg（系统消息）的 `content` 本身就是 XML 串，截断后就是 XML 垃圾。`ConversationMessage.msg_type: Option<String>`（`models.rs:775`）已存了分类类型（webhooks.rs `classify_inbound_msg_type:925` 归一为 text/image/voice/video/namecard/emoji/location/appmsg/voip/statussync/system/unknown），但 preview 没用它。
3. **判据不够**：`is_operatable_person`（`src/webhooks.rs`，真人漏斗那轮加的）只拦 `gh_` 前缀 + `@chatroom`。微信系统号（weixin/fmessage/newsapp/filehelper 等）既不带 gh_ 也不含 @chatroom → 漏网进池。代码里**已有**系统号白名单 `WECHAT_SYSTEM_ACCOUNTS`（`src/mcp.rs:496-500`）+ 判据 `is_non_human_account`（`mcp.rs:503`），但目前是 `mcp.rs` 私有、只用于 roster 标记，未被建档判据复用。

**媒体号的边界（实测定案）**：福州晚报/海峡都市报/福建经济广播 是 `wxid_*` 开头、被用户主动加为好友，微信 roster 里 `is_non_human=false`（`mcp.rs:1297` 测试也断言福州晚报非 non-human）。代码**无可靠信号**区分「媒体号好友」和「真人好友」——两者都是 wxid_*、都 is_non_human=false。故媒体号不能自动拦，只能人工移除。

## 已定决策（用户拍板）

1. **系统号自动拦**：复用 `WECHAT_SYSTEM_ACCOUNTS` + `is_non_human_account` 扩展 `is_operatable_person`。
2. **媒体号手动移除**：前端每行加「从池移除」入口 → 后端标记 `hidden_from_pool`（**不删记录**）→ list_contacts 过滤。**单向移除，不做恢复 UI**（误移极少见，真要恢复走后台）。
3. **预览按 msg_type 标签化**：非 text 类型显示友好标签，只有 text 才截原文。
4. **CSS**：tab 下移，不再与长副标题同行挤压。

## 架构与四层改动

### 改动 1：CSS 修 tab 布局（`frontend/src/styles.css`）

运营池的 `panelHead` 不该用横向 `space-between`——标题/副标题块与 tab 应**上下堆叠**。方案：给运营池的 panelHead 加一个修饰类（如 `.panelHead.poolHead`）或直接改 poolHeadText/segmented 的容器为纵向 flex/grid：
- `poolHeadText`（标题 + 2 行副标题）占一整行（纵向堆叠，已是 `display:grid`）。
- `.segmented`（3 档 tab）**换行到下方**独占一行，`width:100%` 或自然宽度，三个 button `flex:1` 均分，不再被副标题挤压。
- 保持四级层级、不新建嵌套 panel、色板纪律（`docs/frontend-design-system.md`）。
- 具体实现（改 JSX 结构 vs 纯 CSS override）留 writing-plans 定；原则：最小改动让 tab 独占一行且不畸形。**Playwright after 截图验证 tab 正常横排**。

### 改动 2：预览按 msg_type 标签化（`src/routes/contacts.rs`）

`list_contacts` 填 `last_inbound_preview` 时，先看 `msg.msg_type`：
- `text`（或 None/缺省，向后兼容旧消息）→ `truncate_preview(&msg.content, 30)`（现有逻辑，原文截断）。
- 其它类型 → 固定中文标签，**不读 content**（避免 XML）。

**映射表**（新增纯函数 `preview_label_for_type(msg_type: Option<&str>, content: &str) -> String`，可 lib 单测）：

| msg_type | 预览 |
| --- | --- |
| text / None | `truncate_preview(content, 30)` |
| image | `[图片]` |
| voice | `[语音]` |
| video | `[视频]` |
| namecard | `[名片]` |
| emoji | `[表情]` |
| location | `[位置]` |
| appmsg | `[链接]` |
| voip | `[通话]` |
| statussync | `""`（空，状态同步无展示意义） |
| system | `[系统消息]` |
| unknown / 其它 | `[消息]` |

`normal` 不调 LLM 红线不变（纯静态映射，非智能摘要）。

### 改动 3：系统号判据扩展（`src/mcp.rs` + `src/webhooks.rs`）

1. `WECHAT_SYSTEM_ACCOUNTS` 常量 + `is_non_human_account` 从 `mcp.rs` 私有改 `pub(crate)`（供 webhooks 复用；judge 逻辑单一来源，杜绝两份系统号清单漂移）。
2. `is_operatable_person`（webhooks.rs）扩展：现有 `!(gh_ || @chatroom)` 基础上，再排除 `WECHAT_SYSTEM_ACCOUNTS` 命中者。即：`!(gh_前缀 || @chatroom后缀 || 是微信系统保留号)`。
3. 判据同源生效于三处（建档 upsert_webhook_contact / m029 已跑过存量此处不重跑 / list_contacts 读时过滤）——扩展判据后，建档自动拦系统号 + list 读时双保险过滤系统号（存量里的 weixin/fmessage 读时即被过滤，无需新 migration）。

**注意**：`item_type` 参数——webhooks 判据只有 wxid（无 roster 的 type 字段），故复用时按 `is_non_human_account(wxid, None)`（只走白名单命中分支，不依赖 type=="system"）。或抽一个 `is_system_account(wxid: &str) -> bool` 只查白名单，给 is_operatable_person 用。实现时二选一，writing-plans 定。

### 改动 4：`hidden_from_pool` 手动移除（后端端点 + 前端入口）

**数据模型**：`Contact` 加 `#[serde(default)] pub hidden_from_pool: bool`（`src/models.rs` Contact struct；默认 false 向后兼容旧文档，无需 migration）。

**后端端点**：新增 `POST /api/contacts/:id/hide-from-pool`（或 PUT，REST 风格 writing-plans 定），set `hidden_from_pool: true`。挂在 `routes/mod.rs`，workspace/account 隔离校验同现有 contacts 端点。

**list_contacts 过滤**：filter 加 `hidden_from_pool` 不为 true 的条件（`{ "hidden_from_pool": { "$ne": true } }`，兼容缺字段旧文档）。count_contacts 若也要一致（终审提过 count/list 口径），本轮**同步给 count_contacts 加同款过滤**（顺带收口 count/list 漂移那个 Minor）。

**前端入口**：ContactsView 每行加一个「从池移除」小按钮/菜单（hover 显示或行尾 icon），点击调新端点 + 乐观更新（从列表移除该行）+ 刷新 count。**仅对当前行**，不影响批量选择。二次确认（window.confirm 或轻提示）避免误点。

## 测试

### 后端
- **纯函数单测**（进 lib）：`preview_label_for_type`（各 msg_type → 对应标签；text 走截断；None 向后兼容）；`is_operatable_person` 扩展后断言系统号（weixin/fmessage/newsapp）返 false、真人 wxid_* 返 true、gh_/@chatroom 仍 false。
- **hidden_from_pool**：集成测试（#[ignore] 待 CI Docker）——hide 端点标记后 list_contacts 不返回该联系人；count 同步不计；再发消息（webhook）不重置标记（保持隐藏）。
- `Contact` 加字段后所有构造点编译（`cargo check --tests`）。

### 前端
- ContactsView 契约测试：预览标签渲染（appmsg 行显示「[链接]」而非 XML）；「从池移除」按钮存在 + 点击调回调；tab 区域渲染（结构层面）。
- **Playwright before/after 截图**：after 图验证 ①tab 正常横排不畸形 ②预览显示 [链接]/[系统消息] 而非 XML ③系统号不在池中。与 pool_before.png 对比。

### 验证门（硬性）
1. `cargo test --lib` ≥ 350 passed / 0 failed。
2. `cargo check --tests`。
3. `cd frontend && npm run build` + `npx vitest run`。
4. `scripts/check-no-human-takeover.sh`（新增文案「从池移除/[系统消息]」等不含禁用词）。

## 不做的事（YAGNI）

- 不做「已移除」视图 / 恢复端点（单向移除）。
- 不自动判媒体号（无可靠信号，避免误伤真人）。
- 不给 normal 联系人调 LLM（预览是静态标签/原文截断）。
- 不改 webhook 消息落库 / gateway / principal / quiet-hours。
- 不新增 migration（hidden_from_pool 靠 serde default；系统号靠 list 读时过滤清存量）。
- 不动通讯录 RosterView。

## 部署注意

117 当前挂在并行会话的 `verify-batch2` 分支（含真人漏斗）。本轮改动合并 main 后，部署时机需与并行会话协调，**不贸然覆盖 117 当前分支**。且部署必须同时重建前端（`npm run build`）——上次漏了前端构建导致旧 dist 残留（本次问题的触发源之一）。

## 关联

- 前序：真人漏斗重设计（PR #168，本轮修它的显示/判据遗留）。
- 判据同源：`is_operatable_person`（webhooks.rs）+ `WECHAT_SYSTEM_ACCOUNTS`（mcp.rs:496）。
- 部署/前端构建坑：见 memory `bug-webhook-contact-nickname-mcp-account-self` 部署段。
