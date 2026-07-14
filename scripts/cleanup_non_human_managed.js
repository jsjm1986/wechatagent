// 一次性存量清理：把 managed+hidden 矛盾的非真人号 agent_status 改回 normal。
// 用法: mongosh <db> scripts/cleanup_non_human_managed.js
// 非 migration —— 清历史脏数据，不入启动流程。
const TARGETS = [
  "wxid_8874178741811",       // 福州晚报(新闻号)
  "wxid_2540165401612",       // 福建经济广播(电台号)
  "wxid_czpvyjvhzizj22",      // AI应用开发(营销号)
  "wxid_3yeirsb75afd22",      // Demi = 账号102自己(自反身)
  "25984984932102183@openim", // 企业微信号
];

print("=== 更新前备份 ===");
db.contacts.find({ wxid: { $in: TARGETS } }, { wxid: 1, nickname: 1, agent_status: 1, hidden_from_pool: 1 })
  .forEach(c => print(`  ${c.wxid} | ${c.nickname} | agent_status=${c.agent_status} | hidden=${c.hidden_from_pool}`));

const r = db.contacts.updateMany(
  { wxid: { $in: TARGETS }, agent_status: "managed" },
  { $set: { agent_status: "normal", updated_at: new Date() } }
);
print(`=== 更新 matched=${r.matchedCount} modified=${r.modifiedCount} ===`);

print("=== 更新后回读 ===");
db.contacts.find({ wxid: { $in: TARGETS } }, { wxid: 1, nickname: 1, agent_status: 1, hidden_from_pool: 1 })
  .forEach(c => print(`  ${c.wxid} | ${c.nickname} | agent_status=${c.agent_status} | hidden=${c.hidden_from_pool}`));
