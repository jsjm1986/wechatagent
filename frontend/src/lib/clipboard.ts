// 复制到剪贴板（单一真相源）。
//
// 与 lib/uuid.ts 同一个坑：`navigator.clipboard` 只在**安全上下文**暴露。生产以
// 纯 HTTP + IP 提供服务，于是异步 Clipboard API 整个不存在，`navigator.clipboard
// .writeText(...)` 会抛 "Cannot read properties of undefined"。开发（localhost）
// 与 jsdom 测试都照不出来。
//
// 两级降级：
//   1. `navigator.clipboard.writeText()` —— 安全上下文下的标准实现
//   2. 隐藏 `<textarea>` + `document.execCommand("copy")` —— 已废弃但所有主流
//      浏览器仍实现，且**不要求**安全上下文，只要求「用户手势中同步调用」。
//      本函数总是从点击回调里调用，满足该前提。
//
// 返回 boolean 而非抛错：调用方要显示的是「已复制 / 复制失败」，不是异常栈。

async function viaClipboardApi(text: string): Promise<boolean> {
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  if (!nav?.clipboard || typeof nav.clipboard.writeText !== "function") return false;
  try {
    await nav.clipboard.writeText(text);
    return true;
  } catch {
    // 权限被拒 / 文档失焦。落到 execCommand。
    return false;
  }
}

function viaExecCommand(text: string): boolean {
  if (typeof document === "undefined" || typeof document.body === "undefined") return false;
  const ta = document.createElement("textarea");
  ta.value = text;
  // 必须留在渲染树内且可聚焦，否则 execCommand 复制不到内容；
  // 同时不能引起滚动跳动或被读屏软件念出来。
  ta.setAttribute("readonly", "");
  ta.setAttribute("aria-hidden", "true");
  ta.style.position = "fixed";
  ta.style.top = "0";
  ta.style.left = "0";
  ta.style.width = "1px";
  ta.style.height = "1px";
  ta.style.padding = "0";
  ta.style.border = "none";
  ta.style.outline = "none";
  ta.style.boxShadow = "none";
  ta.style.background = "transparent";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  const previous = document.activeElement;
  try {
    ta.focus();
    ta.select();
    ta.setSelectionRange(0, text.length);
    return typeof document.execCommand === "function" && document.execCommand("copy");
  } catch {
    return false;
  } finally {
    document.body.removeChild(ta);
    // 焦点还给原元素，避免复制后键盘焦点丢失。
    if (previous instanceof HTMLElement) previous.focus();
  }
}

/**
 * 把文本写入剪贴板，成功返回 true。任何宿主上都不抛异常。
 *
 * 必须在用户手势（click 等）的同步调用链里发起：execCommand 兜底依赖这一点。
 */
export async function copyText(text: string): Promise<boolean> {
  if (await viaClipboardApi(text)) return true;
  return viaExecCommand(text);
}
