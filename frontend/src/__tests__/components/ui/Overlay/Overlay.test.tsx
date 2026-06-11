import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Overlay } from "../../../../components/ui/Overlay";

describe("Overlay", () => {
  it("open=false 时不渲染", () => {
    render(
      <Overlay open={false} onClose={() => {}}>
        <button>内容</button>
      </Overlay>
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("open=true 渲染 role=dialog 且 aria-modal", () => {
    render(
      <Overlay open onClose={() => {}}>
        <button>内容</button>
      </Overlay>
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveAttribute("aria-modal", "true");
  });

  it("按 Esc 触发 onClose", () => {
    const onClose = vi.fn();
    render(
      <Overlay open onClose={onClose}>
        <button>内容</button>
      </Overlay>
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("点击 scrim 触发 onClose", () => {
    const onClose = vi.fn();
    render(
      <Overlay open onClose={onClose}>
        <button>内容</button>
      </Overlay>
    );
    // scrim 是 dialog 的父节点
    const scrim = screen.getByRole("dialog").parentElement as HTMLElement;
    fireEvent.mouseDown(scrim);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closeOnScrim=false 时点击 scrim 不关闭", () => {
    const onClose = vi.fn();
    render(
      <Overlay open onClose={onClose} closeOnScrim={false}>
        <button>内容</button>
      </Overlay>
    );
    const scrim = screen.getByRole("dialog").parentElement as HTMLElement;
    fireEvent.mouseDown(scrim);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("进场把焦点移到首个可聚焦元素", () => {
    render(
      <Overlay open onClose={() => {}}>
        <button>第一个</button>
        <button>第二个</button>
      </Overlay>
    );
    expect(document.activeElement).toBe(screen.getByText("第一个"));
  });
});
