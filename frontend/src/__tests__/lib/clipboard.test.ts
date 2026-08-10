import { afterEach, describe, expect, it, vi } from "vitest";

import { copyText } from "../../lib/clipboard";

/// 非安全上下文下 `navigator.clipboard` 整个对象不存在（生产是纯 HTTP + IP）。
/// jsdom 默认也不实现它，正好等价于生产形态；需要测「标准路径」时再显式装上。
function installClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
}

function removeClipboard() {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: undefined,
  });
}

describe("copyText", () => {
  afterEach(() => {
    removeClipboard();
    vi.restoreAllMocks();
    // execCommand 是 vi.stubGlobal 装上的，restoreAllMocks 不管属性删除。
    Reflect.deleteProperty(document, "execCommand");
  });

  it("安全上下文：优先走 navigator.clipboard.writeText", async () => {
    const writeText = vi.fn(async () => {});
    installClipboard(writeText);

    await expect(copyText("hello")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello");
  });

  it("clipboard 不存在（生产 HTTP 形态）时退化到 execCommand，不抛异常", async () => {
    removeClipboard();
    const execCommand = vi.fn(() => true);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    await expect(copyText("payload")).resolves.toBe(true);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("clipboard.writeText 抛错（权限拒绝/文档失焦）时也退化到 execCommand", async () => {
    installClipboard(async () => {
      throw new Error("NotAllowedError");
    });
    const execCommand = vi.fn(() => true);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    await expect(copyText("payload")).resolves.toBe(true);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("两级都不可用时返回 false 而不是抛异常", async () => {
    removeClipboard();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn(() => false),
    });

    await expect(copyText("payload")).resolves.toBe(false);
  });

  it("execCommand 兜底后清理临时 textarea，不在 DOM 留残渣", async () => {
    removeClipboard();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn(() => true),
    });

    await copyText("payload");
    expect(document.querySelectorAll("textarea")).toHaveLength(0);
  });
});
