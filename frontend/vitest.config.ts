// W6 / Task 7.3：自治回路监控 Tab 端到端测试 — Vitest + jsdom 配置。
//
// 单独一份 vitest.config.ts（不与 vite.config.ts 合并）：
// - 测试 jsdom 环境无需 dev-server 的 proxy 配置；
// - setup 文件挂 @testing-library/jest-dom 的扩展断言；
// - 仅扫描 src/__tests__/**/*.test.{ts,tsx}，避免误吞 App.tsx 里的字符串。

import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/__tests__/setup.ts"],
    include: ["src/__tests__/**/*.test.{ts,tsx}"],
  },
});
