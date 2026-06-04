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
        { id: "a", agentStatus: "managed", remark: "陈先生" } as Contact,
        { id: "b", agentStatus: "normal" } as Contact,
      ],
      selected: null,
      contactTab: "all",
    });
    useAccountStore.setState({ accounts: [], selectedAccountId: "" });
  });

  it("显示托管联系人统计数值", () => {
    render(<OverviewFeature />);
    expect(screen.getByText("托管联系人")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("实时运营流渲染托管联系人 + 自主回复状态", () => {
    render(<OverviewFeature />);
    expect(screen.getByText("实时运营流")).toBeInTheDocument();
    expect(screen.getByText("陈先生")).toBeInTheDocument();
    expect(screen.getByText("自主回复")).toBeInTheDocument();
  });

  it("无托管联系人时显示空态", () => {
    useContactStore.setState({
      contacts: [{ id: "b", agentStatus: "normal" } as Contact],
      selected: null,
      contactTab: "all",
    });
    render(<OverviewFeature />);
    expect(screen.getByText("暂无托管联系人")).toBeInTheDocument();
  });
});
