import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { QuietHoursSettings } from "../../../features/user-ops/QuietHoursSettings";
import {
  domainPayload,
  quietHoursSettingsFromDraft
} from "../../../stores/userOpsDomainHelpers";
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

describe("QuietHoursSettings", () => {
  it("缺少作息字段时展示兼容默认值", () => {
    render(
      <QuietHoursSettings busy={false} draft={draft()} onChange={vi.fn()} onReload={vi.fn()} onSave={vi.fn()} />
    );

    expect(screen.getByRole("checkbox", { name: "已启用" })).toBeChecked();
    expect(screen.getByLabelText("休息开始")).toHaveValue("22");
    expect(screen.getByLabelText("醒来时间")).toHaveValue("8");
    expect(screen.getByLabelText("时区")).toHaveValue("8");
    expect(screen.getByText(/每天 22:00 至次日 08:00/)).toBeInTheDocument();
  });

  it("编辑任一字段时显式写入全部作息字段并保留其他运行参数", () => {
    const onChange = vi.fn();
    render(
      <QuietHoursSettings busy={false} draft={draft()} onChange={onChange} onReload={vi.fn()} onSave={vi.fn()} />
    );

    fireEvent.change(screen.getByLabelText("醒来时间"), { target: { value: "9" } });

    const nextDraft = onChange.mock.calls[0][0] as OperationDomainDraft;
    expect(domainPayload(nextDraft).runtimeParameters).toEqual({
      maxDailyTouches: 3,
      quietHoursEnabled: true,
      quietHoursStart: 22,
      quietHoursEnd: 9,
      quietHoursTzOffsetHours: 8
    });
  });

  it("直接保存时也物化全部作息字段并保留其他运行参数", () => {
    const onSave = vi.fn();
    render(
      <QuietHoursSettings busy={false} draft={draft()} onChange={vi.fn()} onReload={vi.fn()} onSave={onSave} />
    );

    fireEvent.click(screen.getByRole("button", { name: "保存作息" }));

    const savedDraft = onSave.mock.calls[0][0] as OperationDomainDraft;
    expect(domainPayload(savedDraft).runtimeParameters).toEqual({
      maxDailyTouches: 3,
      quietHoursEnabled: true,
      quietHoursStart: 22,
      quietHoursEnd: 8,
      quietHoursTzOffsetHours: 8
    });
  });

  it("允许关闭门控并保存", () => {
    const onChange = vi.fn();
    const onSave = vi.fn();
    const current = draft([
      "quietHoursEnabled = true",
      "quietHoursStart = 22",
      "quietHoursEnd = 8",
      "quietHoursTzOffsetHours = 8"
    ].join("\n"));
    const { rerender } = render(
      <QuietHoursSettings busy={false} draft={current} onChange={onChange} onReload={vi.fn()} onSave={onSave} />
    );

    fireEvent.click(screen.getByRole("checkbox"));
    const disabledDraft = onChange.mock.calls[0][0] as OperationDomainDraft;
    rerender(
      <QuietHoursSettings busy={false} draft={disabledDraft} onChange={onChange} onReload={vi.fn()} onSave={onSave} />
    );
    fireEvent.click(screen.getByRole("button", { name: "保存作息" }));

    expect(quietHoursSettingsFromDraft(disabledDraft).enabled).toBe(false);
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it("启用时起止小时相同会显示错误并禁止保存", () => {
    render(
      <QuietHoursSettings
        busy={false}
        draft={draft([
          "quietHoursEnabled = true",
          "quietHoursStart = 8",
          "quietHoursEnd = 8",
          "quietHoursTzOffsetHours = 8"
        ].join("\n"))}
        onChange={vi.fn()}
        onReload={vi.fn()}
        onSave={vi.fn()}
      />
    );

    expect(screen.getByRole("alert")).toHaveTextContent("休息开始时间不能与醒来时间相同");
    expect(screen.getByRole("button", { name: "保存作息" })).toBeDisabled();
  });

  it("未加载领域配置时不渲染可保存的空表单", () => {
    const onReload = vi.fn();
    render(
      <QuietHoursSettings busy={false} draft={undefined} onChange={vi.fn()} onReload={onReload} onSave={vi.fn()} />
    );

    expect(screen.queryByRole("button", { name: "保存作息" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "重新加载" }));
    expect(onReload).toHaveBeenCalledTimes(1);
  });
});
