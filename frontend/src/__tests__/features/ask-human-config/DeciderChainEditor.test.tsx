import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DeciderChainEditor } from "../../../features/ask-human-config/DeciderChainEditor";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn() } }));

const CONTACTS = {
  items: [
    { id: "1", accountId: "acc1", wxid: "wxid_a", nickname: "阿伟", agentStatus: "managed", tags: [] },
    { id: "2", accountId: "acc1", wxid: "wxid_b", remark: "李总", agentStatus: "normal", tags: [] },
  ],
};

beforeEach(() => {
  (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(CONTACTS);
});

describe("DeciderChainEditor", () => {
  it("从联系人添加 → onChange 收到含 wxid+displayName 的新链", async () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    await waitFor(() => screen.getByText("阿伟"));
    fireEvent.click(screen.getByText("阿伟"));
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_a", displayName: "阿伟", accountId: "acc1" }]);
  });

  it("已在链中的 wxid 从候选排除", async () => {
    // 链中已有 wxid_a（显示名故意取一个不与任何候选 label 撞名的串，
    // 这样「候选里搜不到 wxid_a 对应联系人」可直接断言其 label "阿伟" 不出现）。
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a", displayName: "链中甲" }]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    await waitFor(() => screen.getByText("李总"));      // 候选 wxid_b 出现，确认面板已加载
    expect(screen.queryByText("阿伟")).toBeNull();        // wxid_a 已在链中 → 候选里不出现其 label "阿伟"
  });

  it("删除 → onChange 收到去掉该项的链", () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("删除")[0]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }]);
  });

  it("上移第二项 → onChange 收到顺序交换的链", () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("上移")[1]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }, { wxid: "wxid_a" }]);
  });

  it("加载失败显示错误态而非静默空列表（E16）", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("boom"));
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    // 错误信息出现，且与「无可选联系人」空态区分开。
    expect(await screen.findByText(/boom/)).toBeInTheDocument();
    expect(screen.queryByText("无可选联系人")).toBeNull();
  });

  it("加载成功后不显示错误态", async () => {
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    await waitFor(() => screen.getByText("阿伟"));
    expect(screen.queryByText(/boom/)).toBeNull();
  });
});
