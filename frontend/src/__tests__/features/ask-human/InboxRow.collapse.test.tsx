import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { InboxRow } from "../../../features/ask-human/index";

describe("InboxRow 折叠壳", () => {
  it("默认折叠:children 不渲染,点击展开后渲染", () => {
    render(
      <InboxRow badge={{ label: "请示裁决", tone: "brand" }} title="#EQERR" preview="候选回复人味较好">
        <div>展开详情内容</div>
      </InboxRow>,
    );
    expect(screen.getByText("请示裁决")).toBeInTheDocument();
    expect(screen.getByText("#EQERR")).toBeInTheDocument();
    expect(screen.queryByText("展开详情内容")).toBeNull();
    fireEvent.click(screen.getByText("#EQERR").closest("button")!);
    expect(screen.getByText("展开详情内容")).toBeInTheDocument();
  });
});
