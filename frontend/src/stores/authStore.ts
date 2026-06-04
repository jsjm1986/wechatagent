import { create } from "zustand";

export interface AuthUser {
  username: string;
  userId: string;
  workspaces?: string[];
  currentWorkspace?: string;
}

interface AuthState {
  user: AuthUser | null;
  onLogout: (() => void) | null;
  onSwitchWorkspace: ((workspaceId: string) => void) | null;
  setUser: (user: AuthUser | null) => void;
  setHandlers: (handlers: { onLogout: () => void; onSwitchWorkspace: (workspaceId: string) => void }) => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  user: null,
  onLogout: null,
  onSwitchWorkspace: null,
  setUser: (user) => set({ user }),
  setHandlers: ({ onLogout, onSwitchWorkspace }) => set({ onLogout, onSwitchWorkspace }),
}));
