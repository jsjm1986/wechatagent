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
  requestGeneration: number;
  summaryRequestGeneration: number;
  setActiveSource: (s: string | null) => void;
  refreshSummary: () => Promise<void>;
  load: (source?: string) => Promise<void>;
}

export function principalEscalationCount(summary: InboxSummary | null): number | null {
  const value = summary?.counts.principalEscalation;
  return typeof value === "number" ? value : null;
}

export const useInboxStore = create<InboxState>((set, get) => ({
  items: [],
  errors: [],
  summary: null,
  loading: false,
  fatalError: null,
  activeSource: null,
  requestGeneration: 0,
  summaryRequestGeneration: 0,
  setActiveSource: (s) => set({ activeSource: s }),
  refreshSummary: async () => {
    const generation = get().summaryRequestGeneration + 1;
    set({ summaryRequestGeneration: generation });
    try {
      const summary = await fetchSummary();
      if (get().summaryRequestGeneration !== generation) return;
      set({ summary });
    } catch {
      // Summary 是降级数据：失败时保留最后一次成功快照，null 明确表示尚不可用。
    }
  },
  load: async (source) => {
    const src = source ?? get().activeSource ?? undefined;
    const generation = get().requestGeneration + 1;
    set({ loading: true, fatalError: null, requestGeneration: generation });
    try {
      // inbox 失败是 fatal（无数据可显，走 catch 保留旧 items）；summary 独立刷新并保留上次成功快照。
      const [inbox] = await Promise.all([fetchInbox(src), get().refreshSummary()]);
      if (get().requestGeneration !== generation) return;
      const summary = get().summary;
      set({
        items: sortItems(inbox.items),
        errors: [...inbox.errors, ...(summary?.errors ?? [])].filter(
          (error, index, all) =>
            all.findIndex((candidate) => candidate.source === error.source) ===
            index,
        ),
        loading: false,
      });
    } catch (e) {
      if (get().requestGeneration !== generation) return;
      // 请求级失败（网络/401）：保留上次 items，绝不清空；只置 fatalError。
      set({
        loading: false,
        fatalError: e instanceof Error ? e.message : String(e),
      });
    }
  },
}));
