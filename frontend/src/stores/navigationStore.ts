import { create } from "zustand";
import type { Channel } from "../types";
// 仅类型导入：编译期擦除，不会在运行时把 channels.ts（及其 lucide 图标）拖进本 store。
import type { ChannelGroup } from "../app/channels";

/** 折叠态持久化 key。存的是「被折叠的组」而非「展开的组」：
 *  这样以后新增分组时，老用户的 localStorage 里没有它 → 默认展开，
 *  而不是被历史快照静默隐藏。 */
const STORAGE_KEY = "wa.nav.collapsedGroups";

/** 首次进入时折叠的组。日常/运营常用，默认展开；其余三组按需展开。
 *  导出供测试重置初始态（store 是模块级单例，跨用例会串状态）。 */
export const DEFAULT_COLLAPSED: ChannelGroup[] = ["知识与内容", "成效", "设置"];

/** localStorage 可能不可用（隐私模式抛异常）或存着历史脏数据，故读写都兜住。 */
function loadCollapsed(): ChannelGroup[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return DEFAULT_COLLAPSED;
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return DEFAULT_COLLAPSED;
    return parsed.filter((x): x is ChannelGroup => typeof x === "string");
  } catch {
    return DEFAULT_COLLAPSED;
  }
}

function saveCollapsed(groups: ChannelGroup[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(groups));
  } catch {
    /* 存不进就算了，折叠态退化为「本次会话内有效」，不影响导航本身。 */
  }
}

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
  /** 当前被折叠的组。渲染侧据此决定是否画出组内频道。 */
  collapsedGroups: ChannelGroup[];
  toggleGroup: (group: ChannelGroup) => void;
}

export const useNavigationStore = create<NavigationState>((set, get) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
  collapsedGroups: loadCollapsed(),
  toggleGroup: (group) => {
    const current = get().collapsedGroups;
    const next = current.includes(group)
      ? current.filter((g) => g !== group)
      : [...current, group];
    saveCollapsed(next);
    set({ collapsedGroups: next });
  },
}));
