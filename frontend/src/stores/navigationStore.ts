import { create } from "zustand";
import type { Channel } from "../types";

/** 历史遗留的导航持久化 key，现已全部无用，进程启动时清一次。
 *
 *  这三个 key 是导航形态两次反复的化石：
 *  - `wa.nav.collapsedGroups`（手风琴早期）：存「被折叠的组」数组。
 *  - `wa.nav.expandedGroup`（手风琴后期）：存「唯一展开的组」，空串表示全部收起。
 *  - `wa.nav.activeGroup`（图标轨）：存「轨上选中、面板正在显示的组」。
 *
 *  单列形态下**所有频道恒可见**，不存在「哪一组被展开 / 被选中」这个状态，
 *  所以三者都没有对应概念了。留在 localStorage 里只是垃圾，且下次有人想加
 *  分组状态时容易误读旧值，故显式清掉。 */
const LEGACY_KEYS = [
  "wa.nav.activeGroup",
  "wa.nav.expandedGroup",
  "wa.nav.collapsedGroups",
];

/** localStorage 在隐私模式下会抛异常，清理失败无所谓（本来就是清垃圾），故整体兜住。 */
function clearLegacyKeys(): void {
  try {
    for (const key of LEGACY_KEYS) localStorage.removeItem(key);
  } catch {
    /* 清不掉就算了，旧 key 只是占空间，不参与任何逻辑。 */
  }
}

clearLegacyKeys();

/** 导航状态：只有「当前频道」一件事。
 *
 *  为什么不再有分组状态：导航经历了两轮形态反复，两轮都错在把「侧栏不能出滚动条」
 *  当成硬约束，于是引入了本不该存在的状态：
 *    1) 手风琴 → 需要记「哪一组展开」，跨组切频道要两步、内容跳动；
 *    2) 图标轨 + 二级面板 → 需要记「轨上选中哪组」，且拿宽度换高度，把中文频道名
 *       挤到换行（侧栏 252 减内边距只剩 228，轨吃 56，层层减到文字列仅 104px，
 *       带「未上线」角标的行只剩 48px，而「微信群运营」需要 65px）。
 *  单列 + 常显分组标签把全部频道一次画出，分组只是视觉分隔，没有任何可变状态——
 *  导航状态因此回落到最小：activeChannel。 */
interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
}

export const useNavigationStore = create<NavigationState>((set) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
}));
