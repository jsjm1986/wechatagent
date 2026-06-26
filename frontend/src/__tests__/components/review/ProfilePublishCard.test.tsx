import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ToastProvider } from "../../../components/ui/Toast";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ProfilePublishCard } from "../../../components/review/ProfilePublishCard";

// ProfilePublishCard 自身拉列表（GET /api/admin/domain-profiles → { items }）按 profileId 过滤，
// 不接受 profile prop（组件真实 props = { profileId, onDone }）。故测试 mock api.get 返回 items。
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

vi.mock("../../../lib/api", () => ({
  api: {
    get: (...args: unknown[]) => getMock(...args),
    post: vi.fn().mockResolvedValue({}),
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
  beforeEach(() => vi.clearAllMocks());

  it("展示 generated_state_machine 的 states/goal/advanceSignals/riskRules", async () => {
    getMock.mockResolvedValue({
      items: [
        {
          id: "P1",
          display_name: "母婴顾问",
          is_active: false,
          current_version: false,
          generated_state_machine: STATE_MACHINE,
        },
      ],
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
      items: [{ id: "P1", display_name: "无状态机", is_active: false, current_version: false }],
    });
    renderCard();
    expect(await screen.findByText("无状态机")).toBeInTheDocument();
    await waitFor(() => expect(getMock).toHaveBeenCalled());
    // 状态机审阅区标题不应出现
    expect(screen.queryByText("状态机（激活前审阅）")).not.toBeInTheDocument();
  });
});
