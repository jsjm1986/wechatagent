import { create } from "zustand";
import type { Channel } from "../types";
// 仅类型导入：编译期擦除，不会在运行时把 channels.ts（及其 lucide 图标）拖进本 store。
import type { ChannelGroup } from "../app/channels";

/** 当前选中分组的持久化 key。
 *
 *  语义变更（图标轨改造）：此前存的是「手风琴唯一展开的组」，可以是 null（全部收起）。
 *  现在侧栏是「图标轨 + 二级面板」——轨上永远有一个组被选中、面板永远显示它的频道，
 *  不存在「全部收起」这个态，故类型从 `ChannelGroup | null` 收窄为 `ChannelGroup`。
 *
 *  为什么放弃手风琴：它不是设计选择，是高度妥协。侧栏 nav 可用高约 550px（757px 屏），
 *  20 个频道全展开需 1042px，两组同时展开也要 612px，所以被迫锁成「同时只开一组」。
 *  代价是跨组切频道要两步（先折叠再展开）、内容还会跳动。图标轨一次只渲染一组，
 *  最坏 5 行约 220px，任何视口都放得下——高度问题从结构上消失，那 6 档
 *  紧凑响应（行高一路压到 21px）连带可以全删。
 *
 *  两个历史 key 都清掉：wa.nav.expandedGroup 允许空串表示「全部收起」，
 *  在新语义下是非法值；wa.nav.collapsedGroups 更早，存的是数组。都不兼容，
 *  留着会被当成新格式解析出错。 */
const STORAGE_KEY = "wa.nav.activeGroup";
const LEGACY_KEYS = ["wa.nav.expandedGroup", "wa.nav.collapsedGroups"];

/** 合法分组白名单：localStorage 里可能是旧格式、空串或用户手改的脏值，
 *  不校验就会让轨上选中一个不存在的组、面板渲染成空白。 */
const VALID_GROUPS: ReadonlyArray<ChannelGroup> = [
  "日常",
  "运营",
  "知识与内容",
  "成效",
  "设置",
];

/** 首次进入时选中的组。「日常」含 AI 总控/工作台/统一收件箱，是最高频入口。
 *  导出供测试重置初始态（store 是模块级单例，跨用例会串状态）。 */
export const DEFAULT_GROUP: ChannelGroup = "日常";

/** localStorage 可能不可用（隐私模式抛异常）或存着历史脏数据，故读写都兜住。
 *  与手风琴时代不同：**没有「全部收起」态**。面板恒显示某一组，所以空值/脏值
 *  都回落到 DEFAULT_GROUP，而不是 null。 */
function loadGroup(): ChannelGroup {
  try {
    for (const k of LEGACY_KEYS) localStorage.removeItem(k);
    const raw = localStorage.getItem(STORAGE_KEY);
    // 必须过白名单：旧 key 的空串、数组 JSON、用户手改的脏值若直接 as ChannelGroup
    // 放行，轨上会选中一个不存在的组、面板渲染成空白（且没有任何 UI 能救回来）。
    return raw && VALID_GROUPS.includes(raw as ChannelGroup)
      ? (raw as ChannelGroup)
      : DEFAULT_GROUP;
  } catch {
    return DEFAULT_GROUP;
  }
}

function saveGroup(group: ChannelGroup): void {
  try {
    localStorage.setItem(STORAGE_KEY, group);
  } catch {
    /* 存不进就算了，选中组退化为「本次会话内有效」，不影响导航本身。 */
  }
}

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
  /** 二级面板当前显示的分组。恒非空——面板总在显示某一组，没有「全收起」态。 */
  activeGroup: ChannelGroup;
  /** 点图标轨切换分组。幂等：点已选中的组不做任何事（不像手风琴会收起它）。 */
  selectGroup: (group: ChannelGroup) => void;
}

export const useNavigationStore = create<NavigationState>((set) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
  activeGroup: loadGroup(),
  selectGroup: (group) => {
    saveGroup(group);
    set({ activeGroup: group });
  },
}));
