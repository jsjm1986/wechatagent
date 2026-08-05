import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { QuietHoursSettings } from "../../../features/user-ops/QuietHoursSettings";
import { domainPayload } from "../../../stores/userOpsDomainHelpers";
import type { OperationDomainDraft } from "../../../types";

function draft(runtimeParameters = "maxDailyTouches = 3"): OperationDomainDraft {
  return {
    name: "用户运营",
    goal: "goal",
    methodology: "method",
    workflow: "workflow",
    toolPolicy: "tools",
    automationPolicy: "automation",
    reviewPolicy: "review",
    runtimeParameters,
    stateMachine: "",
    assistModeEnabled: false
  };
}

function renderSettings(overrides: Partial<Parameters<typeof QuietHoursSettings>[0]> = {}) {
  const props: Parameters<typeof QuietHoursSettings>[0] = {
    busy: false,
    draft: draft(),
    onReload: vi.fn(),
    onSave: vi.fn().mockResolvedValue(true),
    ...overrides
  };
  return { ...render(<QuietHoursSettings {...props} />), props };
}

function openDialog() {
  fireEvent.click(screen.getByRole("button", { name: /作息.*22:00.*08:00/ }));
}

describe("QuietHoursSettings", () => {
  it("默认只显示紧凑按钮，点击后才显示兼容默认值", () => {
    renderSettings();

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: /作息.*22:00.*08:00/ })).toBeInTheDocument();

    openDialog();
    expect(screen.getByRole("dialog", { name: "作息时间" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /启用作息门控/ })).toBeChecked();
    expect(screen.getByLabelText("休息开始")).toHaveValue("22");
    expect(screen.getByLabelText("醒来时间")).toHaveValue("8");
    expect(screen.getByLabelText("时区")).toHaveValue("8");
    expect(screen.getByText(/保存成功后立即生效，无需重启/)).toBeInTheDocument();
    expect(screen.getByText(/已排队的醒来回复保留原执行时间/)).toBeInTheDocument();
  });

  it("取消丢弃本地编辑，不提交也不改变按钮摘要", () => {
    const onSave = vi.fn().mockResolvedValue(true);
    renderSettings({ onSave });
    openDialog();

    fireEvent.change(screen.getByLabelText("醒来时间"), { target: { value: "9" } });
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    expect(onSave).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: /作息.*22:00.*08:00/ })).toBeInTheDocument();

    openDialog();
    expect(screen.getByLabelText("醒来时间")).toHaveValue("8");
  });

  it("保存时显式写入全部作息字段并保留其他运行参数", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    renderSettings({ onSave });
    openDialog();

    fireEvent.change(screen.getByLabelText("醒来时间"), { target: { value: "9" } });
    fireEvent.click(screen.getByRole("button", { name: "保存作息" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const savedDraft = onSave.mock.calls[0][0] as OperationDomainDraft;
    expect(domainPayload(savedDraft).runtimeParameters).toEqual({
      maxDailyTouches: 3,
      quietHoursEnabled: true,
      quietHoursStart: 22,
      quietHoursEnd: 9,
      quietHoursTzOffsetHours: 8
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("保存失败时保留弹窗和用户输入", async () => {
    const onSave = vi.fn().mockResolvedValue(false);
    renderSettings({ onSave });
    openDialog();

    fireEvent.change(screen.getByLabelText("醒来时间"), { target: { value: "9" } });
    fireEvent.click(screen.getByRole("button", { name: "保存作息" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("dialog", { name: "作息时间" })).toBeInTheDocument();
    expect(screen.getByLabelText("醒来时间")).toHaveValue("9");
  });

  it("允许关闭门控并保存，时间字段随之禁用", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    renderSettings({ onSave });
    openDialog();

    fireEvent.click(screen.getByRole("checkbox", { name: /启用作息门控/ }));
    expect(screen.getByLabelText("休息开始")).toBeDisabled();
    expect(screen.getByLabelText("醒来时间")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "保存作息" }));

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(domainPayload(onSave.mock.calls[0][0]).runtimeParameters.quietHoursEnabled).toBe(false);
  });

  it("启用时起止小时相同会显示错误并禁止保存", () => {
    renderSettings({
      draft: draft([
        "quietHoursEnabled = true",
        "quietHoursStart = 8",
        "quietHoursEnd = 8",
        "quietHoursTzOffsetHours = 8"
      ].join("\n"))
    });

    fireEvent.click(screen.getByRole("button", { name: /作息.*08:00.*08:00/ }));
    expect(screen.getByRole("alert")).toHaveTextContent("休息开始时间不能与醒来时间相同");
    expect(screen.getByRole("button", { name: "保存作息" })).toBeDisabled();
  });

  it("未加载领域配置时只显示紧凑重试按钮", () => {
    const onReload = vi.fn();
    renderSettings({ draft: undefined, onReload });

    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "重新加载作息" }));
    expect(onReload).toHaveBeenCalledTimes(1);
  });
});
