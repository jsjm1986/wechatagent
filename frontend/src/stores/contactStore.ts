import { create } from "zustand";
import type { Contact, ContactTab } from "../types";

interface ContactState {
  contacts: Contact[];
  selected: Contact | null;
  contactTab: ContactTab;
  setContacts: (contacts: Contact[]) => void;
  setSelected: (c: Contact | null) => void;
  setContactTab: (t: ContactTab) => void;
  managedCount: () => number;
  normalCount: () => number;
}

export const useContactStore = create<ContactState>((set, get) => ({
  contacts: [],
  selected: null,
  contactTab: "all",
  setContacts: (contacts) => set({ contacts }),
  setSelected: (selected) => set({ selected }),
  setContactTab: (contactTab) => set({ contactTab }),
  managedCount: () => get().contacts.filter((c) => c.agentStatus === "managed").length,
  normalCount: () => get().contacts.length - get().managedCount(),
}));
