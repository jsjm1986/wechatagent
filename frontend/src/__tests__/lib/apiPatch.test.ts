import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";

describe("api.patch", () => {
  afterEach(() => vi.restoreAllMocks());

  it("发 PATCH 请求，带 JSON body 和 Content-Type，返回解析后的 JSON", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ item: { id: "x" } }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await api.patch<{ item: { id: string } }>("/api/admin/taxonomies/abc", {
      label: "新名",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/admin/taxonomies/abc", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ label: "新名" }),
    });
    expect(result).toEqual({ item: { id: "x" } });
  });

  it("非 2xx 抛错", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 400,
        headers: { get: () => "application/json" },
        text: async () => JSON.stringify({ error: "label 不能为空" }),
      })
    );
    await expect(api.patch("/api/admin/taxonomies/abc", {})).rejects.toThrow("label 不能为空");
  });
});
