/**
 * 决策人候选过滤：与后端 import 守卫 `webhooks::is_operatable_person` 等价。
 *
 * 为何不是只看 isNonHuman：roster 的 isNonHuman 判据是
 * `item_type=="system" || is_system_account(wxid)`（src/mcp.rs 的 is_non_human_account），
 * 而后端 import 拒绝的是
 * `gh_ 前缀 || @chatroom || @openim || is_system_account`（src/webhooks.rs 的 is_operatable_person）。
 * 公众号/群/企业号只在 item_type=="system" 时才被 roster 标记，否则漏网——
 * 用户能选中，但 import 会静默拒绝（返回 200 且 items 为空），表现为「点了没反应」。
 *
 * 为何不复制后端的系统号白名单：isNonHuman 已覆盖 is_system_account 那一半，
 * 此处只补三条结构性规则即与后端等价。复制 WECHAT_SYSTEM_ACCOUNTS（src/mcp.rs，13 条）
 * 会产生两份清单，后端增删时前端必然漂移。
 */
export function isPickableDecider(entry: { wxid: string; isNonHuman?: boolean }): boolean {
  if (entry.isNonHuman) return false;
  const wxid = entry.wxid;
  if (wxid.startsWith("gh_")) return false;
  if (wxid.includes("@chatroom")) return false;
  if (wxid.includes("@openim")) return false;
  return true;
}
