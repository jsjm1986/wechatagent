import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ToastProvider } from "../../../components/ui/Toast";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ProfilePublishCard } from "../../../components/review/ProfilePublishCard";

// ProfilePublishCard 通过按 ID 的 GET /api/admin/domain-profiles/:id 加载 `{ item }`，
// 草稿是否可审阅不再依赖列表过滤语义。
// generated_state_machine 内层 key 为 camelCase（guide_profile.rs:368-413 绕过 normalize_json_keys），
// goal/advanceSignals/riskRules 逐 state（prompts.rs default_user_operation_state_machine 实证）。

const STATE_MACHINE = {
  states: [
    {
      key: "new_contact",
      name: "初始了解",
      goal: "建立基本上下文，避免过早推销。",
      initial: true,
      advanceSignals: ["明确身份", "表达业务背景"],
      riskRules: ["禁止直接销售"],
    },
    {
      key: "relationship_building",
      name: "关系建立",
      goal: "通过具体帮助建立信任。",
      advanceSignals: ["愿意继续交流"],
      riskRules: ["不要连续追问"],
    },
  ],
};

const getMock = vi.fn();
const postMock = vi.fn();

vi.mock("../../../lib/api", () => ({
  api: {
    get: (...args: unknown[]) => getMock(...args),
    post: (...args: unknown[]) => postMock(...args),
  },
}));

function renderCard() {
  return render(
    <ToastProvider>
      <ConfirmProvider>
        <ProfilePublishCard profileId="P1" />
      </ConfirmProvider>
    </ToastProvider>,
  );
}

describe("ProfilePublishCard 状态机激活前审阅", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    postMock.mockResolvedValue({ status: "completed" });
  });

  it("展示 generated_state_machine 的 states/goal/advanceSignals/riskRules", async () => {
    getMock.mockResolvedValue({
      item: {
        id: "P1",
        display_name: "母婴顾问",
        is_active: false,
        current_version: false,
        release_status: "draft",
        generated_state_machine: STATE_MACHINE,
      },
    });
    renderCard();
    // state name + key
    expect(await screen.findByText("初始了解")).toBeInTheDocument();
    expect(screen.getByText("new_contact")).toBeInTheDocument();
    expect(screen.getByText("关系建立")).toBeInTheDocument();
    expect(screen.getByText("relationship_building")).toBeInTheDocument();
    // goal
    expect(screen.getByText("建立基本上下文，避免过早推销。")).toBeInTheDocument();
    // advanceSignals
    expect(screen.getByText("明确身份")).toBeInTheDocument();
    expect(screen.getByText("表达业务背景")).toBeInTheDocument();
    // riskRules
    expect(screen.getByText("禁止直接销售")).toBeInTheDocument();
  });

  it("无 generated_state_machine 时不渲染状态机区且不崩", async () => {
    getMock.mockResolvedValue({
      item: { id: "P1", display_name: "无状态机", release_status: "draft", is_active: false, current_version: false },
    });
    renderCard();
    expect(await screen.findByText("无状态机")).toBeInTheDocument();
    await waitFor(() => expect(getMock).toHaveBeenCalled());
    // 状态机审阅区标题不应出现
    expect(screen.queryByText("状态机（激活前审阅）")).not.toBeInTheDocument();
  });

  it("发布草稿只调用 publish，不调用 rollout 或 activate", async () => {
    const user = userEvent.setup();
    getMock.mockResolvedValue({
      item: { id: "P1", display_name: "待发布", release_status: "draft", current_version: false, is_active: false },
    });
    renderCard();
    await user.click(await screen.findByRole("button", { name: "发布" }));
    await user.click(screen.getByRole("button", { name: "确认发布" }));
    await waitFor(() => {
      expect(postMock).toHaveBeenCalledWith("/api/admin/domain-profiles/P1/publish", {});
    });
    expect(postMock).toHaveBeenCalledTimes(1);
    expect(postMock.mock.calls.some(([url]) => String(url).includes("/rollout"))).toBe(false);
    expect(postMock.mock.calls.some(([url]) => String(url).includes("/activate"))).toBe(false);
  });

  it("partial 激活明确提示并保留附属同步重试入口", async () => {
    const user = userEvent.setup();
    const pending = {
      id: "P1",
      display_name: "待激活",
      release_status: "published",
      current_version: true,
      is_active: false,
    };
    const active = { ...pending, is_active: true };
    getMock.mockResolvedValueOnce({ item: pending }).mockResolvedValue({ item: active });
    postMock.mockResolvedValueOnce({
      status: "partial",
      retryable: true,
      errors: [{ step: "contacts", message: "temporary failure" }],
    });
    renderCard();
    await user.click(await screen.findByRole("button", { name: "激活生效" }));
    await user.click(screen.getByRole("button", { name: "确认激活" }));
    expect(await screen.findByText(/核心已激活.*contacts.*重试附属同步/)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "重试附属同步" })).toBeInTheDocument();
    expect(postMock).toHaveBeenCalledWith("/api/admin/domain-profiles/P1/activate", {});
  });

  it("历史 published 版本不会误显示为草稿或提供发布按钮", async () => {
    getMock.mockResolvedValue({
      item: { id: "P1", display_name: "历史版", release_status: "published", current_version: false, is_active: false },
    });
    renderCard();
    expect(await screen.findByText("已发布历史")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "发布" })).not.toBeInTheDocument();
  });
});
