import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";

const items: FriendPickerItem[] = [
  { wxid: "wxid_a", nickname: "张三", remark: "老张" },
  { wxid: "wxid_b", nickname: "李四", avatarUrl: "http://x/y.png" },
  { wxid: "wxid_zhang_media", nickname: "某广播" },
];

const baseProps = {
  open: true,
  items,
  onSelect: vi.fn(),
  onClose: vi.fn(),
};

describe("FriendPickerModal", () => {
  it("open=false 不渲染内容", () => {
    render(<FriendPickerModal {...baseProps} open={false} />);
    expect(screen.queryByText("张三")).toBeNull();
  });

  it("渲染所有好友卡片(名字取 remark||nickname||wxid)", () => {
    render(<FriendPickerModal {...baseProps} />);
    expect(screen.getByText("老张")).toBeInTheDocument(); // remark 优先
    expect(screen.getByText("李四")).toBeInTheDocument();
  });

  it("搜索框按 nickname/remark/wxid 过滤", () => {
    render(<FriendPickerModal {...baseProps} />);
    fireEvent.change(screen.getByPlaceholderText(/搜索/), { target: { value: "李四" } });
    expect(screen.getByText("李四")).toBeInTheDocument();
    expect(screen.queryByText("老张")).toBeNull();
  });

  it("点选卡片触发 onSelect 一次(对应 item)", () => {
    const onSelect = vi.fn();
    render(<FriendPickerModal {...baseProps} onSelect={onSelect} />);
    fireEvent.click(screen.getByText("李四"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(items[1]);
  });

  it("loading 显加载态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} loading />);
    expect(screen.getByText(/加载中/)).toBeInTheDocument();
  });

  it("error 显错误态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} error="拉取失败" />);
    expect(screen.getByText(/拉取失败/)).toBeInTheDocument();
  });

  it("空 items 显空态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} />);
    expect(screen.getByText(/暂无好友|没有匹配/)).toBeInTheDocument();
  });

  it("emptyText 自定义空态文案", () => {
    render(<FriendPickerModal {...baseProps} items={[]} emptyText="请先到账号管理同步通讯录" />);
    expect(screen.getByText("请先到账号管理同步通讯录")).toBeInTheDocument();
  });

  it("allowManualWxid=true 时有手动输入入口,提交调 onManualWxid", () => {
    const onManualWxid = vi.fn();
    render(<FriendPickerModal {...baseProps} allowManualWxid onManualWxid={onManualWxid} />);
    fireEvent.click(screen.getByText(/手动输入/));
    fireEvent.change(screen.getByPlaceholderText(/输入.*wxid|微信/i), { target: { value: "wxid_manual" } });
    fireEvent.click(screen.getByText(/确认/));
    expect(onManualWxid).toHaveBeenCalledWith("wxid_manual");
  });

  it("allowManualWxid 默认 false 时无手动输入入口", () => {
    render(<FriendPickerModal {...baseProps} />);
    expect(screen.queryByText(/手动输入/)).toBeNull();
  });

  it("面板加宽到 720px：双列网格放得下完整昵称", () => {
    render(<FriendPickerModal {...baseProps} />);
    // Overlay 的 maxWidth 走 inline style（CSS 默认 480 只够一列半，昵称会大面积截断）。
    expect(screen.getByRole("dialog").style.maxWidth).toBe("720px");
  });

  it("常驻显示总数——4800 人通讯录里这是判断搜索是否生效的关键反馈", () => {
    render(<FriendPickerModal {...baseProps} />);
    expect(screen.getByText("共 3 位")).toBeInTheDocument();
  });

  it("搜索后计数改说「匹配 N 位」，与未搜索态区分", () => {
    render(<FriendPickerModal {...baseProps} />);
    fireEvent.change(screen.getByPlaceholderText(/搜索/), { target: { value: "李四" } });
    expect(screen.getByText("匹配 1 位")).toBeInTheDocument();
    expect(screen.queryByText("共 3 位")).toBeNull();
  });

  it("列表为空时不显示计数（空态已自带说明，再报「共 0 位」是噪音）", () => {
    render(<FriendPickerModal {...baseProps} items={[]} />);
    expect(screen.queryByText(/共 0 位|匹配 0 位/)).toBeNull();
  });
});
