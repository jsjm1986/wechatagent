import { describe, it, expect } from "vitest";
import { safeCsvCell, toCsv } from "../../../features/campaign/csv";
import type { CampaignSendItem } from "../../../stores/campaignStore";

describe("toCsv", () => {
  it("表头 + 每行 客户名/wxid/状态中文/原因", () => {
    const items: CampaignSendItem[] = [
      { contactWxid: "wx_a", name: "张三", status: "sent" },
      { contactWxid: "wx_b", name: "李四", status: "blocked", reason: "daily_limit" },
    ];
    const csv = toCsv(items);
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe("客户名,wxid,状态,原因");
    expect(lines[1]).toBe("张三,wx_a,已送达,");
    // reason 经 SEND_OUTCOME_REASON_LABELS 翻译:daily_limit→已达每日上限。
    expect(lines[2]).toBe("李四,wx_b,被拦,已达每日上限");
  });

  it("空 items 仅表头", () => {
    expect(toCsv([])).toBe("客户名,wxid,状态,原因");
  });

  it("含逗号/引号/换行的值用双引号转义", () => {
    const items: CampaignSendItem[] = [
      { contactWxid: "wx_c", name: 'a,b"c', status: "unknown", reason: "x\ny" },
    ];
    const csv = toCsv(items);
    const line = csv.split("\r\n")[1];
    expect(line).toContain('"a,b""c"');
    expect(line).toContain('"x\ny"');
  });

  it.each(["=HYPERLINK(\"https://evil.test\")", "+SUM(1,2)", "-1+2", "@SUM(1,2)"])(
    "中和公式前缀 %s",
    (value) => {
      const cell = safeCsvCell(value);
      const decoded = cell.startsWith('"')
        ? cell.slice(1, -1).replace(/""/g, '"')
        : cell;
      expect(decoded.startsWith("'")).toBe(true);
    },
  );

  it("前导空白/制表与逗号包裹公式仍被中和，控制字符被规范化", () => {
    expect(safeCsvCell("  =1+1")).toBe("'  =1+1");
    expect(safeCsvCell("\t@SUM(1,2)")).toBe('"\' @SUM(1,2)"');
    expect(safeCsvCell("=SUM(1,2)")).toBe('"\'=SUM(1,2)"');
    expect(safeCsvCell("normal - value")).toBe("normal - value");
  });
});
