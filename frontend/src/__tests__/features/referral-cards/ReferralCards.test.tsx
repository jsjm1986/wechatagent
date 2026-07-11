import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ReferralCardsFeature from "../../../features/referral-cards";
import { useReferralCardStore } from "../../../stores/referralCardStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";

// loadCards useEffect 会打 /api/referral-cards（走 fetch）——stub 掉避免真网络。
vi.stubGlobal("fetch", vi.fn());

beforeEach(() => {
  vi.mocked(fetch).mockResolvedValue({
    ok: true,
    json: async () => ({ items: [] }),
  } as Response);

  useReferralCardStore.setState({
    cards: [],
    cardDraft: { displayName: "", targetWxid: "", sendTriggerHint: "", targetStages: "", tags: "" },
  } as any);
  useUserOpsStore.setState({
    rosterCache: {
      "": {
        items: [
          { wxid: "wxid_adv", nickname: "王顾问", remark: null, avatarUrl: null, sex: 1, agentStatus: "not_imported" },
        ],
        syncing: false,
        fetchedAt: Date.now(),
      },
    },
    loadRoster: vi.fn().mockResolvedValue({
      items: [{ wxid: "wxid_adv", nickname: "王顾问", agentStatus: "not_imported" }],
      syncing: false,
    }),
  } as any);
});

describe("ReferralCards 顾问选择器", () => {
  it("点「从好友选择」打开弹窗,选好友后回填 wxid 且名称为空时联动回填", async () => {
    render(<ReferralCardsFeature />);
    fireEvent.click(screen.getByText(/从好友选择/));
    // 弹窗出现好友
    const friend = await screen.findByText("王顾问");
    fireEvent.click(friend);
    // 回填后:已选展示 wxid + 名称联动
    await waitFor(() => {
      expect(useReferralCardStore.getState().cardDraft.targetWxid).toBe("wxid_adv");
      expect(useReferralCardStore.getState().cardDraft.displayName).toBe("王顾问");
    });
  });
});
