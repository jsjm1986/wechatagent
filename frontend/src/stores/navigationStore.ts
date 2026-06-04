import { create } from "zustand";
import type { Channel } from "../types";

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
}

export const useNavigationStore = create<NavigationState>((set) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
}));
