import { describe, expect, it, beforeEach } from "vitest";
import { useContactStore } from "../../stores/contactStore";
import type { Contact } from "../../types";

const c = (id: string, managed: boolean): Contact =>
  ({ id, agentStatus: managed ? "managed" : "normal" } as Contact);

describe("contactStore", () => {
  beforeEach(() => useContactStore.setState({ contacts: [], selected: null, contactTab: "all" }));
  it("managedCount / normalCount 派生正确", () => {
    useContactStore.getState().setContacts([c("a", true), c("b", false), c("d", true)]);
    expect(useContactStore.getState().managedCount()).toBe(2);
    expect(useContactStore.getState().normalCount()).toBe(1);
  });
});
