import { render, screen } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import CommandCenterFeature from "../../../features/command-center";
import { useCommandStore } from "../../../stores/commandStore";
import { useAccountStore } from "../../../stores/accountStore";
import { useContactStore } from "../../../stores/contactStore";
import type { Account, Contact, AgentSoul, ContentAsset } from "../../../types";

describe("CommandCenterFeature", () => {
  beforeEach(() => {
    // Mock loadCommandData to avoid API calls
    const mockLoadCommandData = vi.fn();

    // Reset stores
    useCommandStore.setState({
      commandDraft: "把 AI应用开发 加入 Agent 运营列表，并生成一份克制、专业的运营备注",
      commandResult: null,
      commandDryRun: true,
      commandBusy: false,
      souls: [
        { id: "soul1", agentKind: "reply", name: "测试Soul", content: "测试内容", status: "active", version: 1 } as AgentSoul
      ],
      assets: [
        { id: "asset1", kind: "faq", title: "测试资产", body: "测试内容" } as ContentAsset
      ],
      pendingTasks: 3,
      setCommandDraft: vi.fn(),
      setCommandDryRun: vi.fn(),
      loadCommandData: mockLoadCommandData,
      runCommand: vi.fn(),
    });

    useAccountStore.setState({
      accounts: [
        { id: "acc1", accountId: "test123", alias: "测试账号", displayName: "Test Account", online: true, mcpKeyConfigured: true } as Account
      ],
      selectedAccountId: "acc1",
    });

    useContactStore.setState({
      contacts: [
        { id: "c1", agentStatus: "managed" } as Contact,
        { id: "c2", agentStatus: "normal" } as Contact,
      ],
      selected: null,
      contactTab: "all",
    });
  });

  it("renders Management Agent title", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("Management Agent")).toBeInTheDocument();
  });

  it("renders operation scope section", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("操作范围")).toBeInTheDocument();
  });

  it("displays current account status", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("当前账号")).toBeInTheDocument();
    expect(screen.getByText("测试账号")).toBeInTheDocument();
  });

  it("displays pending tasks count", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("待执行任务")).toBeInTheDocument();
    expect(screen.getByText("3 pending")).toBeInTheDocument();
  });

  it("displays execution plan section", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("执行计划")).toBeInTheDocument();
  });
});