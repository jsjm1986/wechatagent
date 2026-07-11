import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContactsView } from "../../../features/user-ops/legacy";

describe("ContactsView 运营池", () => {
  // baseProps 对齐当前 ContactsView 实际 props 类型（legacy.tsx:440-464，无 busy）。
  const baseProps = {
    contactTab: "all" as const,
    contacts: [],
    managedCount: 2,
    normalCount: 61,
    query: "",
    selected: null,
    totalCount: 63,
    onContactTab: vi.fn(),
    onLoadAll: vi.fn(),
    onOpenContact: vi.fn(),
    onQuery: vi.fn()
  };

  it("三 tab 文案为 待启用 / Agent / 全部，计数来自 props", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.getByText("待启用 61")).toBeInTheDocument();
    expect(screen.getByText("Agent 2")).toBeInTheDocument();
    expect(screen.getByText("全部 63")).toBeInTheDocument();
    // 旧文案不得残留。
    expect(screen.queryByText(/^已互动 /)).toBeNull();
    expect(screen.queryByText(/^普通 /)).toBeNull();
  });

  it("导入框已移除，只保留过滤框", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.queryByPlaceholderText("搜索并导入好友，例如 AI应用开发")).toBeNull();
    expect(screen.getByPlaceholderText("过滤联系人")).toBeInTheDocument();
  });

  // 漏斗工作台契约（Task 8 行为锁定）：定位说明 / 待启用档差异化 / Agent 档差异化。
  describe("漏斗工作台", () => {
    it("顶部有定位说明 + 区别通讯录小字", () => {
      render(<ContactsView {...baseProps} />);
      // legacy.tsx:558-559 —— 运营池定位 + 与通讯录的区别。
      expect(screen.getByText(/主动来找过你的人/)).toBeInTheDocument();
      expect(screen.getByText(/区别于通讯录/)).toBeInTheDocument();
    });

    it("待启用档行显示消息摘要 + 启用按钮（传了 onBatchEnable）", () => {
      const contacts = [
        {
          id: "1",
          wxid: "wxid_a",
          nickname: "小明",
          agentStatus: "normal",
          lastInboundPreview: "想问下课程怎么收费",
          tags: [],
          operationPolicy: {},
          profileAttributes: {},
          updatedAt: "2026-07-11T00:00:00Z"
        }
      ] as any;
      render(
        <ContactsView
          {...baseProps}
          contactTab="normal"
          contacts={contacts}
          onBatchEnable={vi.fn().mockResolvedValue(undefined)}
        />
      );
      // 消息摘要仅待启用档渲染（legacy.tsx:659-661）。
      expect(screen.getByText(/想问下课程怎么收费/)).toBeInTheDocument();
      // 单人启用按钮仅 selectable（normal + onBatchEnable）时渲染（legacy.tsx:672-683）。
      expect(screen.getByText("启用 Agent")).toBeInTheDocument();
    });

    it("不传 onBatchEnable 时降级为只读列表：无启用按钮", () => {
      const contacts = [
        {
          id: "1",
          wxid: "wxid_a",
          nickname: "小明",
          agentStatus: "normal",
          lastInboundPreview: "想问下课程怎么收费",
          tags: [],
          operationPolicy: {},
          profileAttributes: {},
          updatedAt: "2026-07-11T00:00:00Z"
        }
      ] as any;
      render(<ContactsView {...baseProps} contactTab="normal" contacts={contacts} />);
      // 摘要照常渲染，但无勾选/启用按钮（selectable=false）。
      expect(screen.getByText(/想问下课程怎么收费/)).toBeInTheDocument();
      expect(screen.queryByText("启用 Agent")).toBeNull();
    });

    it("传 onHideFromPool 时行尾有「从池移除」按钮，点击弹确认后调回调", () => {
      const contacts = [
        {
          id: "c1", wxid: "wxid_media", nickname: "福州晚报", agentStatus: "normal",
          lastInboundPreview: "[链接]", tags: [], operationPolicy: {}, profileAttributes: {},
          updatedAt: "2026-07-11T00:00:00Z"
        }
      ] as any;
      const onHideFromPool = vi.fn();
      const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
      render(
        <ContactsView
          {...baseProps}
          contactTab="normal"
          contacts={contacts}
          onBatchEnable={vi.fn().mockResolvedValue(undefined)}
          onHideFromPool={onHideFromPool}
        />
      );
      screen.getByText("从池移除").click();
      expect(confirmSpy).toHaveBeenCalled();
      expect(onHideFromPool).toHaveBeenCalledWith(contacts[0]);
      confirmSpy.mockRestore();
    });

    it("预览为标签时行内渲染标签而非 XML", () => {
      const contacts = [
        {
          id: "c2", wxid: "wxid_x", nickname: "某公众号内容", agentStatus: "normal",
          lastInboundPreview: "[链接]", tags: [], operationPolicy: {}, profileAttributes: {},
          updatedAt: "2026-07-11T00:00:00Z"
        }
      ] as any;
      render(<ContactsView {...baseProps} contactTab="normal" contacts={contacts} onBatchEnable={vi.fn()} />);
      expect(screen.getByText("[链接]")).toBeInTheDocument();
      expect(screen.queryByText(/<msg>|<appmsg|<sysmsg/)).toBeNull();
    });

    it("Agent 档行显示运营阶段徽章", () => {
      const contacts = [
        {
          id: "2",
          wxid: "wxid_b",
          nickname: "张总",
          agentStatus: "managed",
          operationState: "new_contact",
          tags: [],
          operationPolicy: {},
          profileAttributes: {},
          updatedAt: "2026-07-11T00:00:00Z"
        }
      ] as any;
      render(<ContactsView {...baseProps} contactTab="managed" contacts={contacts} />);
      // 阶段值经 labelFor 转中文；无字典（taxonomies 默认 {}）回落原值 new_contact（legacy.tsx:617-620,657）。
      expect(screen.getByText(/new_contact|初次接触|新联系人/)).toBeInTheDocument();
    });
  });
});
