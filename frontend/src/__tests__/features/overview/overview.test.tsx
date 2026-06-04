import { render, screen } from "@testing-library/react";
import { describe, expect, it, beforeEach } from "vitest";
import OverviewFeature from "../../../features/overview";
import { useContactStore } from "../../../stores/contactStore";
import { useAccountStore } from "../../../stores/accountStore";
import type { Contact } from "../../../types";

describe("OverviewFeature", () => {
  beforeEach(() => {
    useContactStore.setState({
      contacts: [
        { id: "a", agentStatus: "managed" } as Contact,
        { id: "b", agentStatus: "normal" } as Contact,
      ],
      selected: null,
      contactTab: "all",
    });
    useAccountStore.setState({ accounts: [], selectedAccountId: "" });
  });

  it("显示 managedCount 指标卡数值", () => {
    render(<OverviewFeature />);
    expect(screen.getByText("Managed Users")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("无最近事件时显示空态", () => {
    render(<OverviewFeature />);
    expect(screen.getByText("暂无运营事件")).toBeInTheDocument();
  });
});
