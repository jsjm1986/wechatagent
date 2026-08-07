import { create } from "zustand";
import type { Channel } from "../types";
// 仅类型导入：编译期擦除，不会在运行时把 channels.ts（及其 lucide 图标）拖进本 store。
import type { ChannelGroup } from "../app/channels";

/** 折叠态持久化 key（存**被收起**的组名数组）。
 *
 *  为什么存"收起的"而不是"展开的"：新增分组时默认展开（数组里没有它），
 *  这是更安全的默认——新频道不会因为存量用户的旧持久化值而被藏起来。
 *
 *  与历史上两次尝试的区别（都失败了，别再走回去）：
 *  1) 手风琴（wa.nav.expandedGroup，存"唯一展开的组"）：强制互斥——展开一组必须
 *     收起另一组。病根是当时把"侧栏不能出滚动条"当硬约束，于是用互斥硬压高度。
 *     代价是跨组切频道要两步（先折叠再展开）、内容跳动。
 *  2) 图标轨（wa.nav.activeGroup，存"当前选中组"）：拿宽度换高度，中文标签直接
 *     换行（带角标的行文字列只剩 48px，「微信群运营」需 65px）。
 *
 *  现在滚动是被允许的（VS Code / Linear / Notion 侧栏都滚），所以**互斥这个约束
 *  根本不必要**。各组独立折叠：想全开就全开（滚动兜住），想全收就全收，
 *  跨组切频道一步到位、无跳动。折叠在这里只是"减少滚动距离"的便利，不是高度妥协。 */
/** key 带 v2 后缀：分组重命名（日常/运营/知识与内容/成效/设置 → 运营对象/AI 的资料/
 *  需要你决策/运行与结果/系统/即将上线）后，旧值里的组名在新白名单下全部非法。
 *  若沿用同一个 key，过滤后会得到**空集合** = 全部展开 = 立刻滚动，而不是回落到
 *  新的默认折叠。换 key 让老用户走 `raw === null` 分支拿到 DEFAULT_COLLAPSED。 */
const STORAGE_KEY = "wa.nav.collapsed.v2";
/** 历史 key 全部清理：语义都与现在不兼容（见上方注释与 v2 说明）。 */
const LEGACY_KEYS = [
  "wa.nav.collapsed",
  "wa.nav.activeGroup",
  "wa.nav.expandedGroup",
  "wa.nav.collapsedGroups",
];

/** 合法分组白名单：localStorage 里可能是旧格式或用户手改的脏值。
 *  不校验就会让某个不存在的组名一直留在集合里（无害但会累积）。 */
const VALID_GROUPS: ReadonlyArray<ChannelGroup> = [
  "运营对象",
  "AI 的资料",
  "需要你决策",
  "运行与结果",
  "系统",
  "即将上线",
];

/** 首次进入时默认收起的组。
 *
 *  分组从 5 组变 6 组后，标签 + 组间距的固定开销从 179px 涨到 218px（6 个标签 ×
 *  23px + 5 个间距 × 16px），可放行数上限降到 10。所以默认必须收起 3 组才不滚：
 *    - 「系统」        —— 账号/模型/策略/自治回路/演化，配好就不动
 *    - 「即将上线」     —— 两个占位频道，点不了
 *    - 「AI 的资料」    —— 知识库/话术/名片，配一次用很久，不是每天改
 *  余下「运营对象」「需要你决策」「运行与结果」展开 = 10 行 ≈ 578px，
 *  实测 1440×900 下可用 611px，余 33px 不滚动。
 *
 *  导出供测试复用，避免测试里硬编码一份漂移的默认值。 */
export const DEFAULT_COLLAPSED: ReadonlyArray<ChannelGroup> = [
  "AI 的资料",
  "系统",
  "即将上线",
];

/** localStorage 可能不可用（隐私模式抛异常）或存着脏数据，故读写都兜住。 */
function loadCollapsed(): Set<ChannelGroup> {
  try {
    for (const k of LEGACY_KEYS) localStorage.removeItem(k);
    const raw = localStorage.getItem(STORAGE_KEY);
    // 没存过 → 用默认值。存过空数组（"[]"）是"全部展开"的合法表示，
    // 必须与"没存过"区分，否则用户手动全展开后一刷新又被收回默认态。
    if (raw === null) return new Set(DEFAULT_COLLAPSED);
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set(DEFAULT_COLLAPSED);
    return new Set(
      parsed.filter((g): g is ChannelGroup =>
        VALID_GROUPS.includes(g as ChannelGroup)
      )
    );
  } catch {
    return new Set(DEFAULT_COLLAPSED);
  }
}

function saveCollapsed(groups: Set<ChannelGroup>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...groups]));
  } catch {
    /* 存不进就算了，折叠态退化为「本次会话内有效」，不影响导航本身。 */
  }
}

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
  /** 被收起的分组集合。不在集合里 = 展开。可以为空（全部展开）。 */
  collapsedGroups: Set<ChannelGroup>;
  /** 独立折叠某组，不影响其他组——这是与手风琴的根本区别。 */
  toggleGroup: (group: ChannelGroup) => void;
}

export const useNavigationStore = create<NavigationState>((set, get) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
  collapsedGroups: loadCollapsed(),
  toggleGroup: (group) => {
    // 必须造新 Set：zustand 靠引用比较决定重渲染，原地 add/delete 组件不会更新。
    const next = new Set(get().collapsedGroups);
    if (next.has(group)) next.delete(group);
    else next.add(group);
    saveCollapsed(next);
    set({ collapsedGroups: next });
  },
}));
