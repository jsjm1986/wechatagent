import { describe, expect, it, beforeEach } from "vitest";
import { useNavigationStore } from "../../stores/navigationStore";

describe("navigationStore", () => {
  beforeEach(() => useNavigationStore.setState({ activeChannel: "command" }));
  it("默认 channel 为 command", () => {
    expect(useNavigationStore.getState().activeChannel).toBe("command");
  });
  it("setChannel 切换 activeChannel", () => {
    useNavigationStore.getState().setChannel("userOps");
    expect(useNavigationStore.getState().activeChannel).toBe("userOps");
  });
});
