import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ToastProvider } from "../../../components/ui/Toast";
import { DeciderChainEditor } from "../../../features/ask-human-config/DeciderChainEditor";
import { api } from "../../../lib/api";
import { useUserOpsStore } from "../../../stores/userOpsStore";
import { useAccountStore } from "../../../stores/accountStore";
import type { DeciderRef } from "../../../types";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn() } }));

const post = api.post as unknown as ReturnType<typeof vi.fn>;

/** 通讯录条目。agentStatus 决定选中后是否需要先导入。 */
function entry(
  wxid: string,
  nickname: string,
  agentStatus: "managed" | "normal" | "not_imported" = "normal",
  extra: Record<string, unknown> = {},
) {
  return { wxid, nickname, remark: null, avatarUrl: null, sex: 1, isNonHuman: false, agentStatus, ...extra };
}

/** 铺好 store：账号 + roster 缓存（syncing:false 表示快照已就绪）。 */
function seedRoster(items: ReturnType<typeof entry>[], syncing = false) {
  useAccountStore.setState({ accounts: [{ accountId: "acc1", wxid: "self_wx", online: true } as never], selectedAccountId: "acc1" });
  useUserOpsStore.setState({
    rosterCache: { acc1: { items: items as never, syncing, fetchedAt: Date.now(), serverFetchedAt: null } },
    loadRoster: vi.fn().mockResolvedValue({ items: items as never, syncing }),
  });
}

/**
 * 组件把导入失败走 toast（设计 §4.5）——因为 FriendPickerModal 的 scrim 是
 * portal 到 body 的全屏遮罩（z-overlay:1000），编辑器内联的错误会被它盖住；
 * toast 的 z-toast:1100 高于 scrim 才看得见。故渲染必须带 ToastProvider。
 */
function renderEditor(chain: DeciderRef[], onChange: (next: DeciderRef[]) => void) {
  return render(
    <ToastProvider>
      <DeciderChainEditor chain={chain} onChange={onChange} />
    </ToastProvider>,
  );
}

beforeEach(() => {
  post.mockReset();
});

describe("DeciderChainEditor 通讯录选择器", () => {
  it("按钮文案是「从通讯录添加」而非「从联系人添加」", () => {
    seedRoster([entry("wxid_a", "阿伟")]);
    renderEditor([], vi.fn());
    expect(screen.getByText(/从通讯录添加/)).toBeTruthy();
    expect(screen.queryByText(/从联系人添加/)).toBeNull();
  });

  it("已入库好友：选中直接入链，不调 import", async () => {
    seedRoster([entry("wxid_a", "阿伟", "normal")]);
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("阿伟"));

    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith([
        { wxid: "wxid_a", displayName: "阿伟", accountId: "acc1" },
      ]),
    );
    // 已入库 → 不该有 import 写操作。
    expect(post).not.toHaveBeenCalled();
  });

  it("未入库好友：先 import 落库再入链", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockResolvedValue({ items: [{ wxid: "wxid_new" }] });
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    await waitFor(() => expect(post).toHaveBeenCalled());
    // 端点与载荷：/api/contacts/import，candidates 带 wxid/nickname/remark。
    expect(post.mock.calls[0][0]).toBe("/api/contacts/import");
    const body = post.mock.calls[0][1] as { accountId: string; candidates: { wxid: string }[] };
    expect(body.accountId).toBe("acc1");
    expect(body.candidates[0].wxid).toBe("wxid_new");
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith([
        { wxid: "wxid_new", displayName: "新朋友", accountId: "acc1" },
      ]),
    );
  });

  it("import 成功后不强制重拉通讯录（设计 §4.4：避免打断连续添加多人）", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockResolvedValue({ items: [{ wxid: "wxid_new" }] });
    const loadRoster = useUserOpsStore.getState().loadRoster as unknown as ReturnType<typeof vi.fn>;
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    await waitFor(() => expect(post).toHaveBeenCalled());
    // force 会让后端 spawn_roster_refresh 走一次全量 MCP 重拉。
    expect(loadRoster.mock.calls.every((c) => !c[1]?.force)).toBe(true);
  });

  it("import 返回空 items（后端静默拒绝）→ 不入链并报错", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    // 坑 1：接口回 200 但 items 为空 = upsert 返回 None，导入没成功。
    post.mockResolvedValue({ items: [] });
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    expect(await screen.findByText(/未能导入通讯录/)).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("import 失败提示走 toast，能浮在弹窗遮罩之上（inline 会被 scrim 盖住）", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockResolvedValue({ items: [] });
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    // toast 挂在 ToastProvider 的 role=status 栈里（portal 到 body，z-toast 1100
    // > scrim 1000）。断言它在该栈内，而非编辑器 DOM 子树内——后者会被遮罩盖住。
    // 注：jsdom 无层叠上下文，真实遮挡关系仍需目视核验。
    const msg = await screen.findByText(/未能导入通讯录/);
    expect(msg.closest('[role="status"]')).not.toBeNull();
    // 弹窗保持打开，用户可以直接换一位选。
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("import 抛错 → 不入链并显示错误", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockRejectedValue(new Error("network down"));
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    expect(await screen.findByText(/network down/)).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("非真人不出现在候选：公众号 gh_ / 群 @chatroom / isNonHuman", async () => {
    seedRoster([
      entry("wxid_ok", "真人甲"),
      entry("gh_416c280c4978", "某公众号"),
      entry("7842243308@chatroom", "某群"),
      entry("weixin", "微信团队", "normal", { isNonHuman: true }),
    ]);
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("真人甲");
    expect(screen.queryByText("某公众号")).toBeNull();
    expect(screen.queryByText("某群")).toBeNull();
    expect(screen.queryByText("微信团队")).toBeNull();
  });

  it("已在链中的 wxid 从候选排除", async () => {
    seedRoster([entry("wxid_a", "阿伟"), entry("wxid_b", "李总")]);
    renderEditor([{ wxid: "wxid_a", displayName: "链中甲" }], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("李总");
    expect(screen.queryByText("阿伟")).toBeNull();
  });

  it("不提供手动输入 wxid 入口（后端 fail-closed 会拒绝不在通讯录的 wxid）", async () => {
    seedRoster([entry("wxid_a", "阿伟")]);
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("阿伟");
    expect(screen.queryByText(/手动输入 wxid/)).toBeNull();
  });

  it("删除 → onChange 收到去掉该项的链", () => {
    seedRoster([]);
    const onChange = vi.fn();
    renderEditor([{ wxid: "wxid_a" }, { wxid: "wxid_b" }], onChange);
    fireEvent.click(screen.getAllByLabelText("删除")[0]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }]);
  });

  it("上移第二项 → onChange 收到顺序交换的链", () => {
    seedRoster([]);
    const onChange = vi.fn();
    renderEditor([{ wxid: "wxid_a" }, { wxid: "wxid_b" }], onChange);
    fireEvent.click(screen.getAllByLabelText("上移")[1]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }, { wxid: "wxid_a" }]);
  });

  it("通讯录同步中显示同步态，而非空列表", async () => {
    // 坑 2：首次无快照时后端返回 items:[] + syncing:true。
    seedRoster([], true);
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    expect(await screen.findByText(/同步中/)).toBeTruthy();
  });

  it("通讯录加载失败在弹窗内显示错误态，而非静默空列表（E16）", async () => {
    seedRoster([]);
    useUserOpsStore.setState({ loadRoster: vi.fn().mockRejectedValue(new Error("boom")) as never });
    renderEditor([], vi.fn());
    fireEvent.click(screen.getByText(/从通讯录添加/));
    expect(await screen.findByText(/boom/)).toBeTruthy();
  });
  it("导入进行中重复点击同一好友，只发一次 import 请求", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    // 可控 promise：在连点期间保持 pending，模拟慢速网络往返。
    let release: (v: unknown) => void = () => {};
    post.mockReturnValue(new Promise((res) => { release = res; }));
    const onChange = vi.fn();
    renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));

    const card = await screen.findByText("新朋友");
    fireEvent.click(card);
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    // import 仍 pending 时再点两次——重入守卫必须挡住。
    fireEvent.click(card);
    fireEvent.click(card);
    expect(post).toHaveBeenCalledTimes(1);

    release({ items: [{ wxid: "wxid_new" }] });
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
  });

  it("import 期间父级改了 chain，入链基于最新值而非 stale 闭包快照", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    let release: (v: unknown) => void = () => {};
    post.mockReturnValue(new Promise((res) => { release = res; }));
    const onChange = vi.fn();
    // 初始空链渲染 → pick 的闭包捕获 []。
    const { rerender } = renderEditor([], onChange);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));

    // import 未完成时父级塞进一位（模拟并发/外部更新）。
    rerender(
      <ToastProvider>
        <DeciderChainEditor chain={[{ wxid: "wxid_prior", displayName: "先前一位" }]} onChange={onChange} />
      </ToastProvider>,
    );
    release({ items: [{ wxid: "wxid_new" }] });

    // 若读 stale 闭包的 []，结果只有 wxid_new，先前一位被静默丢弃。
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const last = onChange.mock.calls[onChange.mock.calls.length - 1][0] as DeciderRef[];
    expect(last.map((d) => d.wxid)).toEqual(["wxid_prior", "wxid_new"]);
  });
});
