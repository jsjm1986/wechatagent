import { afterEach, describe, expect, it, vi } from "vitest";

import { randomUuid } from "../../lib/uuid";

// 生产是纯 HTTP + IP（非安全上下文），`crypto.randomUUID` 在那里是 undefined。
// 开发（localhost）与 jsdom（Node webcrypto）两边都有它，所以这组用例必须
// **主动摘掉**各级能力来复现线上宿主，否则永远测不到降级路径。

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const realCrypto = globalThis.crypto;

function setCrypto(value: unknown): void {
  Object.defineProperty(globalThis, "crypto", {
    configurable: true,
    writable: true,
    value,
  });
}

describe("randomUuid 在非安全上下文下的降级", () => {
  afterEach(() => {
    setCrypto(realCrypto);
    vi.restoreAllMocks();
  });

  it("安全上下文：直接用 crypto.randomUUID", () => {
    const spy = vi.fn(() => "11111111-2222-4333-8444-555555555555");
    setCrypto({ randomUUID: spy, getRandomValues: realCrypto.getRandomValues.bind(realCrypto) });
    expect(randomUuid()).toBe("11111111-2222-4333-8444-555555555555");
    expect(spy).toHaveBeenCalledTimes(1);
  });

  /// 这条正是线上崩溃的复现：randomUUID 不存在时不得抛错。
  it("randomUUID 缺失（线上形态）：退到 getRandomValues 且仍是合法 v4", () => {
    setCrypto({ getRandomValues: realCrypto.getRandomValues.bind(realCrypto) });
    const id = randomUuid();
    expect(id).toMatch(UUID_V4);
  });

  it("randomUUID 存在但抛错：同样退到 getRandomValues", () => {
    setCrypto({
      randomUUID: () => {
        throw new Error("SecurityError");
      },
      getRandomValues: realCrypto.getRandomValues.bind(realCrypto),
    });
    expect(randomUuid()).toMatch(UUID_V4);
  });

  it("WebCrypto 整体缺失：退到 Math.random 兜底，形态不变", () => {
    setCrypto(undefined);
    expect(randomUuid()).toMatch(UUID_V4);
  });

  it("getRandomValues 抛错：仍能产出合法 v4", () => {
    setCrypto({
      getRandomValues: () => {
        throw new Error("blocked");
      },
    });
    expect(randomUuid()).toMatch(UUID_V4);
  });

  it("连续生成不重复（各降级层都成立）", () => {
    for (const crypto of [
      { getRandomValues: realCrypto.getRandomValues.bind(realCrypto) },
      undefined,
    ]) {
      setCrypto(crypto);
      const ids = new Set(Array.from({ length: 200 }, () => randomUuid()));
      expect(ids.size).toBe(200);
    }
  });
});
