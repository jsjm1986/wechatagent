import { create } from "zustand";
import {
  fetchInbox,
  fetchSummary,
  sortItems,
  type InboxItem,
  type SourceError,
  type InboxSummary,
} from "../lib/inboxApi";

interface InboxState {
  items: InboxItem[];
  errors: SourceError[];
  summary: InboxSummary | null;
  loading: boolean;
  fatalError: string | null;
  activeSource: string | null; // null = 全部源
  setActiveSource: (s: string | null) => void;
  load: (source?: string) => Promise<void>;
}

export const useInboxStore = create<InboxState>((set, get) => ({
  items: [],
  errors: [],
  summary: null,
  loading: false,
  fatalError: null,
  activeSource: null,
  setActiveSource: (s) => set({ activeSource: s }),
  load: async (source) => {
    const src = source ?? get().activeSource ?? undefined;
    set({ loading: true, fatalError: null });
    try {
      // inbox 失败是 fatal（无数据可显，走 catch 保留旧 items）；summary 失败软化为保留上次。
      const [inbox, summary] = await Promise.all([
        fetchInbox(src),
        fetchSummary().catch(() => get().summary),
      ]);
      set({
        items: sortItems(inbox.items),
        errors: inbox.errors,
        summary: summary ?? get().summary,
        loading: false,
      });
    } catch (e) {
      // 请求级失败（网络/401）：保留上次 items，绝不清空；只置 fatalError。
      set({
        loading: false,
        fatalError: e instanceof Error ? e.message : String(e),
      });
    }
  },
}));
