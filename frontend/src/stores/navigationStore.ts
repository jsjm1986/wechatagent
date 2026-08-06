import { create } from "zustand";
import type { Channel } from "../types";
// 仅类型导入：编译期擦除，不会在运行时把 channels.ts（及其 lucide 图标）拖进本 store。
import type { ChannelGroup } from "../app/channels";

/** 手风琴态持久化 key。
 *
 *  为什么存「唯一展开的组」而不是此前的「被折叠的组数组」：
 *  侧栏高度是固定的 100vh，减去品牌区/页脚/内边距后 nav 实际只有约 550px（757px 屏），
 *  而 20 个频道全展开需要 1042px。只要允许多组同时展开，行数就没有上限，滚动条必然
 *  出现——把行高从 43px 压到 36px 也只降到 874px，压缩解决不了。改成同时只展开一组后，
 *  最坏情况（最大的组 5 行）是 397px，任何视口都放得下，滚动条从结构上消失。
 *
 *  旧 key wa.nav.collapsedGroups 存的是数组，语义与此不兼容，故换新 key 并顺手清掉旧的，
 *  避免老用户的历史数组被当成新格式解析。 */
const STORAGE_KEY = "wa.nav.expandedGroup";
const LEGACY_KEY = "wa.nav.collapsedGroups";

/** 首次进入时展开的组。「日常」含 AI 总控/工作台/统一收件箱，是最高频入口。
 *  导出供测试重置初始态（store 是模块级单例，跨用例会串状态）。 */
export const DEFAULT_EXPANDED: ChannelGroup = "日常";

/** localStorage 可能不可用（隐私模式抛异常）或存着历史脏数据，故读写都兜住。
 *  空字符串是「全部收起」的合法持久化表示，与 null（没存过）区分。 */
function loadExpanded(): ChannelGroup | null {
  try {
    localStorage.removeItem(LEGACY_KEY);
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return DEFAULT_EXPANDED;
    if (raw === "") return null;
    return raw as ChannelGroup;
  } catch {
    return DEFAULT_EXPANDED;
  }
}

function saveExpanded(group: ChannelGroup | null): void {
  try {
    localStorage.setItem(STORAGE_KEY, group ?? "");
  } catch {
    /* 存不进就算了，展开态退化为「本次会话内有效」，不影响导航本身。 */
  }
}

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
  /** 当前唯一展开的组；null 表示全部收起。渲染侧据此决定是否画出组内频道。 */
  expandedGroup: ChannelGroup | null;
  /** 手风琴切换：点已展开的组则收起它，点别的组则改为只展开它。 */
  toggleGroup: (group: ChannelGroup) => void;
}

export const useNavigationStore = create<NavigationState>((set, get) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
  expandedGroup: loadExpanded(),
  toggleGroup: (group) => {
    const next = get().expandedGroup === group ? null : group;
    saveExpanded(next);
    set({ expandedGroup: next });
  },
}));
