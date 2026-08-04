// 全局测试 setup — 引入 jest-dom 自定义匹配器（toBeInTheDocument 等）。
import "@testing-library/jest-dom/vitest";

// Node 25 exposes a process-global `localStorage` accessor when launched with
// `--localstorage-file`. Some managed runners inject that flag without a path,
// yielding `{}` and shadowing jsdom's standards-compliant Storage object.
// Repair only that invalid test-runtime shape; browsers and normal jsdom keep
// their native implementation.
if (typeof globalThis.localStorage?.getItem !== "function") {
  const values = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key: string) {
      return values.get(String(key)) ?? null;
    },
    key(index: number) {
      return Array.from(values.keys())[index] ?? null;
    },
    removeItem(key: string) {
      values.delete(String(key));
    },
    setItem(key: string, value: string) {
      values.set(String(key), String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
}
