import { describe, it, expect } from "vitest";
import { isPickableDecider } from "../../../features/ask-human-config/deciderCandidates";

describe("isPickableDecider", () => {
  it("真人 wxid 可选", () => {
    expect(isPickableDecider({ wxid: "wxid_ydzaomn4scsb12" })).toBe(true);
    expect(isPickableDecider({ wxid: "wxid_8874178741811" })).toBe(true);
  });

  it("roster 已标 isNonHuman 的不可选（覆盖系统号那一半判据）", () => {
    expect(isPickableDecider({ wxid: "weixin", isNonHuman: true })).toBe(false);
    expect(isPickableDecider({ wxid: "fmessage", isNonHuman: true })).toBe(false);
  });

  it("公众号 gh_ 前缀不可选（roster 可能漏标，后端会静默拒绝）", () => {
    expect(isPickableDecider({ wxid: "gh_416c280c4978" })).toBe(false);
    expect(isPickableDecider({ wxid: "gh_416c280c4978", isNonHuman: false })).toBe(false);
  });

  it("群 @chatroom 不可选", () => {
    expect(isPickableDecider({ wxid: "7842243308@chatroom" })).toBe(false);
  });

  it("企业微信/开放 IM @openim 不可选", () => {
    expect(isPickableDecider({ wxid: "25984984932102183@openim" })).toBe(false);
  });

  it("gh 出现在中间不算公众号（只认前缀，与后端 starts_with 对齐）", () => {
    expect(isPickableDecider({ wxid: "wxid_gh_not_prefix" })).toBe(true);
  });
});
