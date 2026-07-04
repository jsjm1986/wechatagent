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
    expect(screen.getByText("管理助手")).toBeInTheDocument();
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
    expect(screen.getByText("3 个待执行")).toBeInTheDocument();
  });

  it("displays execution plan section", () => {
    render(<CommandCenterFeature />);
    expect(screen.getByText("执行计划")).toBeInTheDocument();
  });

  it("renders confirm/reject buttons when command is pending_confirmation", () => {
    useCommandStore.setState({
      commandResult: {
        id: "run-hex-1",
        status: "pending_confirmation",
        summary: "该计划包含高风险操作，等待确认。",
        toolCalls: [
          { id: "tc1", toolName: "message_send_text", status: "pending" },
        ],
      },
    });
    render(<CommandCenterFeature />);
    expect(screen.getByRole("button", { name: "确认执行" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "否决" })).toBeInTheDocument();
  });

  it("shows 待核实 marker for executed_unverified tool calls", () => {
    useCommandStore.setState({
      commandResult: {
        id: "run-hex-2",
        status: "succeeded",
        summary: "已执行。",
        toolCalls: [
          { id: "tc2", toolName: "message_send_text", status: "executed_unverified" },
        ],
      },
    });
    render(<CommandCenterFeature />);
    expect(screen.getByText(/待核实/)).toBeInTheDocument();
  });

  it("does not render confirm button for already succeeded commands", () => {
    useCommandStore.setState({
      commandResult: {
        id: "run-hex-3",
        status: "succeeded",
        summary: "已执行。",
        toolCalls: [
          { id: "tc3", toolName: "message_send_text", status: "succeeded" },
        ],
      },
    });
    render(<CommandCenterFeature />);
    expect(screen.queryByRole("button", { name: "确认执行" })).not.toBeInTheDocument();
  });

  it("F13: gatewayStatus 显示中文标签", () => {
    useCommandStore.setState({
      commandResult: {
        id: "run-hex-4",
        status: "succeeded",
        summary: "已执行。",
        toolCalls: [
          {
            id: "tc4",
            toolName: "message_send_text",
            status: "executed_unverified",
            response: { sentContent: "您好，已收到您的咨询", gatewayStatus: "held_by_ai_policy" },
          },
        ],
      },
    });
    render(<CommandCenterFeature />);
    expect(screen.getByText(/AI 策略主动暂缓/)).toBeInTheDocument();
    expect(screen.queryByText(/held_by_ai_policy/)).not.toBeInTheDocument();
  });

  it("F13: 未知 gatewayStatus 回落原值不崩", () => {
    useCommandStore.setState({
      commandResult: {
        id: "run-hex-5",
        status: "succeeded",
        summary: "已执行。",
        toolCalls: [
          {
            id: "tc5",
            toolName: "message_send_text",
            status: "executed_unverified",
            response: { sentContent: "测试发送内容", gatewayStatus: "some_future_status" },
          },
        ],
      },
    });
    render(<CommandCenterFeature />);
    expect(screen.getByText(/some_future_status/)).toBeInTheDocument();
  });
});