# 好友选择器弹窗（FriendPickerModal）设计

**日期**：2026-07-11
**状态**：设计讨论中，待用户复审
**范围**：把"手填微信 wxid"的痛点换成"点按钮 → 弹窗头像网格 + 搜索 → 点选回填"。抽一个共享单选弹窗组件 `FriendPickerModal`，专属顾问名片（referral-cards，roster 源）+ products-deals 成交/持仓选择器（ContactPicker，运营池源）两处接入。纯前端，后端零改动。

## 背景（现状 100% 亲验 file:line）

普通用户不知道自己/好友的微信 wxid，但当前多处让用户手填：
- **专属顾问名片**（`frontend/src/features/referral-cards/index.tsx:106-111`）：`targetWxid` 是裸文本框（placeholder "用于发送名片的微信 wxid"），手填。这是主痛点。
- **products-deals**（`frontend/src/features/products-deals/index.tsx:351-414` `ContactPicker`）：已有"搜索框 + 内联下拉列表"选择器，读 `/api/contacts`（运营池），返回 `Contact` 类型。功能可用但无头像、交互与通讯录不一致。
- **决策链**（`frontend/src/features/ask-human-config/DeciderChainEditor.tsx:78-94`）：已有内联选择器，读运营池，**多选累加**交互。

已有的全量好友数据源：
- 通讯录 `RosterView`（`frontend/src/features/user-ops/RosterView.tsx`）通过 `userOpsStore.loadRoster(accountId)` 拉全部微信好友（带头像），缓存在 `rosterCache`（`userOpsStore.ts:77,117,475-489`）。数据经 GET `/api/contacts/roster?accountId=`。
- `RosterEntry` 类型（`frontend/src/types/index.ts:136`）：`{ wxid, nickname?, remark?, avatarUrl?, sex?, isNonHuman?, agentStatus }`。

## 已定决策（用户拍板）

1. **数据源**：专属顾问从 **roster（全部好友）** 选——顾问是自己人、大概率不在运营池，只有全好友列表才搜得到。
2. **交互形态**：**弹窗头像网格 + 搜索**（与通讯录 RosterView 观感一致，头像最直观）。
3. **统一策略**：**统一 UI、数据源可配**——抽 `FriendPickerModal` 只管弹窗/网格/搜索，数据由调用方传入（专属顾问传 roster，products-deals 传 contacts）。不动各调用方的业务逻辑（成交按 contact.id 查、决策链存 wxid+displayName）。
4. **范围收敛**：**决策链不动**（它是多选累加，与单选弹窗交互不同，为统一而统一引入风险不值得）。实际接入 = 专属顾问 + products-deals ContactPicker 两处。
5. **顾问不在好友列表的兜底**：弹窗底部留一个折叠的"手动输入 wxid"小入口——默认引导选好友，选不到（未加好友）才手填。

## 架构与组件

### 组件 1：`FriendPickerModal`（新建，`frontend/src/components/ui/FriendPickerModal.tsx`）

放公共 ui 层，供多频道引用。**纯受控、UI-only、不自己拉数据**。

**统一 item 形态**（调用方把 RosterEntry / Contact 各自 map 成此形态）：
```ts
export type FriendPickerItem = {
  wxid: string;
  nickname?: string | null;
  remark?: string | null;
  avatarUrl?: string | null;
  sex?: number | null;
  badge?: string;        // 可选状态标签（如"已托管"），不传则不显示
};
```

**Props**：
```ts
{
  open: boolean;
  items: FriendPickerItem[];
  onSelect: (item: FriendPickerItem) => void;  // 点选一个 → 回调 + 由调用方关闭
  onClose: () => void;
  title?: string;              // 默认"选择好友"
  loading?: boolean;
  error?: string | null;
  allowManualWxid?: boolean;   // 默认 false；true 时底部展开"手动输入 wxid"入口
  onManualWxid?: (wxid: string) => void;  // 手填提交回调（allowManualWxid 时必传）
}
```

**内部结构**（复用 RosterView 卡片观感 + 分页 hook 思路，但独立实现避免耦合 RosterView）：
- 遮罩层 + 居中弹窗容器（遵守 `docs/frontend-design-system.md`，不引入新颜色）。
- 顶部：标题 + 关闭按钮。
- 搜索框：按 `remark / nickname / wxid` 过滤（与 RosterView:100-106 同款过滤）。
- 头像卡片网格：每卡片头像（缺失回落首字母）+ 名字（remark || nickname || wxid）+ wxid 小字 + 可选 badge。点卡片 → `onSelect(item)`。
- 分页（每页 60，roster 可能几百人；复用 RosterView 的 usePagedList 思路，组件内本地实现）。
- 空态（无 items）/ 加载态（loading）/ 错误态（error）。
- 底部（allowManualWxid=true 时）：折叠的"手动输入 wxid"——展开后一个输入框 + 确认，点确认调 `onManualWxid(wxid)`。默认收起、不显眼。
- 单选：点一个卡片即 `onSelect` + 调用方关闭。无多选态。

**CSS**：`FriendPickerModal.module.css`，卡片网格样式参照 RosterView.module.css 的 `.grid/.card/.avatar` 观感（不 import 它的 css module，独立写，避免跨组件耦合）。

### 组件 2：专属顾问接入（`frontend/src/features/referral-cards/index.tsx`）

- "顾问微信号"裸文本框（:104-112）改为 **只读展示 + 按钮**：
  - 未选：按钮"从好友选择" → 打开弹窗（`allowManualWxid=true` 兜底）。
  - 已选：显示头像 + 名字 + wxid + "重选"按钮。
- 组件内加载 roster：`useEffect` 调 `useUserOpsStore(s => s.loadRoster)(currentAccountId)`，读 `rosterCache[currentAccountId].items`，map 成 `FriendPickerItem[]`（badge 用 agentStatus 中文化或不传）传入弹窗。
- `onSelect`：回填 `cardDraft.targetWxid = item.wxid`；若"顾问名称"（displayName）当前为空，顺带回填 `item.remark || item.nickname`（省一步手填；已填则不覆盖）。
- `onManualWxid`：回填 `targetWxid`（兜底，displayName 不动）。
- `cardDraft.targetWxid` 仍是 wxid 字符串——**后端零改动**（`referral_cards.rs` `target_wxid` 只校验非空，:56）。
- 保存按钮 disabled 逻辑不变（`!targetWxid.trim()`）。

### 组件 3：products-deals ContactPicker 换壳（`frontend/src/features/products-deals/index.tsx:351-414`）

- 保持现有 `/api/contacts` 拉取（:362-374）与 `Contact` 类型不变——**不动业务逻辑**（成交/持仓按 contact.id 查）。
- 把内联搜索框 + 列表（:385-412）换成"按钮 → `FriendPickerModal`"：
  - map `Contact[]` → `FriendPickerItem[]`（badge 可不传或用 operationState 中文化）。
  - `onSelect`：从 wxid 找回原 `Contact` 对象（`contacts.find(c => c.wxid === item.wxid)`）调用现有 `onSelect(contact)`——保持外部契约 `onSelect(c: Contact | null)` 不变。
  - `allowManualWxid=false`（运营池选择不需要手填兜底）。
- 空态文案沿用现有（"当前账号还没有联系人…"）。

## 数据流

```
专属顾问:
  referral-cards useEffect → loadRoster(accountId) → rosterCache[acct].items (RosterEntry[])
    → map → FriendPickerItem[] → <FriendPickerModal open items onSelect>
    → onSelect → targetWxid=item.wxid (+displayName 联动) → 关闭

products-deals:
  ContactPicker 已有 /api/contacts 拉取 (Contact[]) 不变
    → map → FriendPickerItem[] → <FriendPickerModal>
    → onSelect(item) → contacts.find(wxid) → 现有 onSelect(Contact) → 关闭
```

调用方各自负责 map 与 open/close state；`FriendPickerModal` 无数据获取、无业务耦合。

## 测试

### 前端契约（vitest）
- **FriendPickerModal**（`frontend/src/__tests__/components/FriendPickerModal.test.tsx` 新建）：
  - 搜索过滤：输入昵称/备注/wxid 片段，列表只剩匹配项。
  - 点选卡片 → `onSelect` 以对应 item 被调用一次。
  - `open=false` 不渲染内容；`loading` 显加载态；`error` 显错误态；空 items 显空态。
  - `allowManualWxid=true` 底部有手动输入入口；提交调 `onManualWxid`；`allowManualWxid=false` 无该入口。
- **referral-cards**（扩现有或新建测试）：选好友后 targetWxid 回填 + displayName 为空时联动回填、已填时不覆盖；"重选"重开弹窗。
- **products-deals ContactPicker**：换壳后点选仍以正确 `Contact` 调 `onSelect`（现有测试若存在先跑不回归）。

### 验证门（硬性）
1. `cd frontend && npm run build` 成功（tsc 无错）。
2. `cd frontend && npx vitest run` 全绿（含新增契约测试）。
3. `scripts/check-no-human-takeover.sh` 0 violations（新增文案"从好友选择/重选/手动输入 wxid"等不含 `人工/接管/takeover/hand-off`）。
4. 后端零改动，lib 基线不涉及；如误触后端则 `cargo test --lib` ≥ 350/0 + `cargo check --tests`。

### Playwright（收尾验证）
- 专属顾问表单点"从好友选择"→ 弹窗出好友头像网格 → 搜索 → 点选 → wxid 回填。before/after 截图。

## 不做的事（YAGNI）
- 决策链（DeciderChainEditor）不改（多选累加交互不同）。
- 不加 multi 多选模式（当前两个接入点都是单选）。
- 不改后端 `referral_cards.rs` / roster 端点 / `target_wxid` 校验。
- 不新增 roster 端点（复用 `loadRoster`）。
- 不动 products-deals 的成交/持仓业务逻辑（按 contact.id 查不变）。

## 关联
- roster 数据源：`userOpsStore.loadRoster` + `roster_snapshots`（见 memory `project-roster-backend-snapshot-deployed`）。
- 专属顾问业务背景：`docs/superpowers/specs/2026-06-21-referral-card-push-design.md`（辅助模式名片引荐）。
- 设计系统：`docs/frontend-design-system.md`（颜色纪律：蓝仅主操作/紫仅 AI 身份；卡片观感参照 RosterView）。
