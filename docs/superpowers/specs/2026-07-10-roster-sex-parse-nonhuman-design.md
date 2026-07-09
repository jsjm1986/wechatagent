# 通讯录性别解析修复 + 非真人账号标记折叠 + roster 缓存

> 日期：2026-07-10 · 分支：fix/roster-sex-parse-nonhuman · 状态：设计（待写计划）

## 背景

通讯录切 `contacts_fetch_full`（#158）上线后，服务器真实数据（账号 t-1，4832 条好友）暴露三个问题（第 3 个为用户补充诉求）：

1. **性别全显「未知」**：`parse_roster_items`（src/mcp.rs:559）用 `obj.get("sex").and_then(|v| v.as_i64())` 解析，但 MCP 返回的 `sex` 真实形态是 **int64 序列化对象** `{"high":0,"low":0,"unsigned":false}`，`as_i64()` 对对象取不到值 → 全部 `None` → 前端全显「未知」。真实数据其实完整：`sex.low` 分布 = 男 1427 / 女 2857 / 未知 548。

2. **系统账号混入列表**：通讯录含微信系统账号（fmessage 朋友推荐消息、qqmail QQ邮箱提醒、weixin 微信团队、mphelper 公众平台安全助手、medianote 语音记事本、qmessage QQ离线消息、floatbottle 漂流瓶 等），运营看着困惑。

3. **每次进频道都重新加载（用户补充诉求）**：切进通讯录频道就重拉 4831 条（慢），应首次拉后缓存，仅点「刷新」才重拉。

## 已亲验事实（服务器 mcp_call_logs 真实数据，2026-07-10）

- `sex` 形态：`{"high":0,"low":0,"unsigned":false}`，真实值在 `.low`（0未知/1男/2女）。
- `type` 字段：只有两值 —— `friend`(4830) / `system`(2, 仅 weixin 等)。
- 系统账号：多数 `type=="friend"`（fmessage/qqmail/mphelper/medianote/qmessage/floatbottle），**仅靠 type 筛不掉**，但它们是**微信固定保留 wxid**（业界通用白名单）。
- **公众号无法可靠识别**：福州晚报（userName=`wxid_8874178741811`, alias=`fuzhouwb`）与真人字段**完全同构**，gewe 数据**无 verifyFlag/bizFlag/contactType 等公众号标识字段**（已核 item 全字段集）。故公众号不做自动识别（硬猜必误伤真人）。
- 真人 userName 前缀混杂：`wxid_` 3737 个 + 老号自定义短 id 1095 个（含真人如 `songboyu1993 小宇`）——**不能按「非 wxid_」过滤**。

## 设计决策（已与用户对齐）

1. **性别解析取 `.low`**（兼容裸整数，防未来形态变化）。
2. **非真人判定放后端**：`type=="system"` 或 `userName` ∈ 微信保留账号白名单 → `is_non_human=true`。
3. **公众号不自动识别**（无可靠字段），当普通好友保留。
4. **前端默认折叠**非真人账号（不删、不后端过滤），运营可展开。

## 实现范围

### 后端 `src/mcp.rs`
- **sex 解析修复**（parse_roster_items 对象分支，约 559 行）：
  ```rust
  sex: obj.get("sex").and_then(|v| {
      v.as_i64().or_else(|| v.get("low").and_then(|l| l.as_i64()))
  }).map(|n| n as i32),
  ```
  兼容裸整数（`as_i64` 直取）与 `{high,low}` 对象（回退取 `.low`）。
- **非真人白名单常量**（模块级）：
  ```rust
  const WECHAT_SYSTEM_ACCOUNTS: &[&str] = &[
      "fmessage", "qqmail", "weixin", "mphelper", "medianote",
      "qmessage", "floatbottle", "tmessage", "qqsync", "newsapp",
      "filehelper", "weibo", "brandsessionholder",
  ];
  ```
- **判定纯函数**：`fn is_non_human_account(user_name: &str, item_type: Option<&str>) -> bool`
  = `item_type == Some("system")` 或 `WECHAT_SYSTEM_ACCOUNTS.contains(&user_name)`。
- **`RosterFriend` 加 `is_non_human: bool`**（无 Option，默认 false）。parse_roster_items **两处构造点**都补：
  - 字符串分支（wxid 字符串数组，约 550 行）：`is_non_human: is_non_human_account(s, None)`（字符串形态无 type）。
  - 对象分支（约 549 行）：`is_non_human: is_non_human_account(&wxid, obj.get("type").and_then(|v| v.as_str()))`。

### API 层 `src/routes/contacts.rs`
- `roster_endpoint` items json（约 409 行）加 `"isNonHuman": f.is_non_human`。

### 前端
- `RosterEntry` 类型（types/index.ts）加 `isNonHuman?: boolean`。
- `RosterView`：
  - 拆分 `filtered` 为 `humanRows`（真人）+ `nonHumanRows`（非真人）。
  - 真人正常分页显示（现有逻辑）。
  - 非真人默认折叠：列表末尾一个可点击区「含 N 个系统账号 · 展开/收起」，展开后网格显示，标「系统账号」灰标签。
  - 过滤（filter）时对两组都生效。
  - 勾选批量托管仍可选（不强制禁用非真人——运营自主判断），但折叠区默认不显、需展开才能勾。

### 前端 roster 缓存（用户新增诉求：进频道用缓存，不每次实时拉）

**问题**：roster 数据当前存在 `RosterView` 组件 local state（`useState`），组件卸载即丢。每次切进通讯录频道 → 重新挂载 → state 空 → `useEffect` 触发 `refresh` → 后端实时打 MCP 拉 4831 条 + 头像补全（慢）。用户诉求：**首次拉一次后缓存，切进频道直接用缓存，只有点「刷新」才重拉**。

**修法**（纯前端，后端不动）：roster 数据从组件 local state 提到 **store**，按 `accountId` 键控缓存。
- `userOpsStore` 加状态：`rosterCache: Record<accountId, { items: RosterEntry[]; syncing: boolean; fetchedAt: number }>`。
- `loadRoster(accountId, opts?: { force?: boolean })`：
  - `force !== true` 且 `rosterCache[accountId]` 存在 → 直接返回缓存，**不打 API**。
  - 否则打 API，结果写入 `rosterCache[accountId]` 再返回。
  - 注：`syncing:true`（同步中）的结果**不缓存**（或缓存但允许自动重拉覆盖），否则会卡在同步中态。仅 `syncing:false`（就绪）才落缓存。
- `RosterView`：
  - 挂载时的 `refresh`：走缓存（`force:false`），有缓存不重拉。
  - 「刷新」按钮：`force:true` 强制重拉。
  - 切账号：目标账号有缓存则用缓存，无则拉（请求序号守卫保留）。
  - `roster` 数据源：删掉组件里的 `const [roster, setRoster] = useState`，直接从 store 读 `rosterCache[effectiveAccountId]?.items ?? []`（store 是唯一真相源，保证跨挂载存活）。`syncing` 同理从缓存条目读。组件不再持有 roster 的 local state。
- **缓存生命周期**：session 内存活（zustand store，刷新浏览器即清）——不做 localStorage 持久化（YAGNI，好友列表时效性要求高，浏览器刷新重拉可接受）。

## YAGNI（明确不做）
- 公众号自动识别（无可靠字段）。
- 首屏头像加载性能优化（`loading="lazy"` 已加，进一步优化属额外范围，用户暂不要）。
- 后端物理过滤 / 删除非真人（用户明确要「标记不删」）。

## 测试计划

- `parse_roster_items` 新增：`sex:{high,low}` 对象形态解析出正确性别（复用已有 `contacts_fetch_full` 测试扩断言）；裸整数形态仍兼容。
- `is_non_human_account` 纯函数单测：system type→true、fmessage/qqmail 白名单→true、真人 wxid_/短id→false、福州晚报 wxid_→false（公众号不误判）。
- 前端 roster.test：非真人默认折叠、展开后可见、真人不受影响；性别文字正确。
- 基线门：`cargo test --lib` ≥350、前端 tsc 0 + vitest 全绿。

## 部署提醒
带前端改动，部署须 `cargo build` + `npm run build`（见 [[deploy-server-117]] 部署完整性三查）。
