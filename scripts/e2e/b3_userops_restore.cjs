// 批3 userOps 走查恢复：还原走查联系人画像/记忆/note/tags + 删测试 playbook。
// 敏感标识从 env 读，不硬编码真实 PII：
//   RESTORE_CONTACT_OID   走查联系人 _id(hex)
//   RESTORE_CONTACT_WXID  走查联系人 wxid
//   RESTORE_PLAYBOOK_OID  待删测试 playbook _id(hex)
const { MongoClient, ObjectId } = require("mongodb");
(async () => {
  const contactOidHex = process.env.RESTORE_CONTACT_OID;
  const contactWxid = process.env.RESTORE_CONTACT_WXID;
  const playbookOidHex = process.env.RESTORE_PLAYBOOK_OID;
  if (!contactOidHex || !contactWxid) {
    console.error("ERR: 需设置 RESTORE_CONTACT_OID 与 RESTORE_CONTACT_WXID");
    process.exit(1);
  }
  const c = new MongoClient(process.env.MONGODB_URI);
  await c.connect();
  const db = c.db(process.env.MONGODB_DATABASE);
  const oid = ObjectId.createFromHexString(contactOidHex);
  const origProfile = {
    summary: "初次接触的潜在客户，对我方产品/服务有初步了解意愿，通过链接和直接提问表现主动",
    interests: ["产品服务流程"],
    communicationStyle: "直接干脆，问题导向",
    operationGoal: "建立信任，引导进入下一步了解",
  };
  const r = await db.collection("contacts").updateOne(
    { _id: oid },
    {
      $set: { agent_profile: origProfile, operation_state: "new_contact" },
      $unset: {
        human_profile_note: "",
        tags: "",
        intent_level: "",
        customer_stage: "",
        follow_up_policy: "",
        operation_state_reason: "",
      },
    }
  );
  console.log("contact_restored", r.modifiedCount);
  const dm = await db.collection("operating_memory").deleteOne({ contact_wxid: contactWxid }).catch(() => ({ deletedCount: -1 }));
  console.log("operating_memory_deleted", dm.deletedCount);
  if (playbookOidHex) {
    const dp = await db.collection("operation_playbooks").deleteOne({ _id: ObjectId.createFromHexString(playbookOidHex) });
    console.log("playbook_deleted", dp.deletedCount);
  }
  // 核对
  const after = await db.collection("contacts").findOne({ _id: oid }, { projection: { agent_profile: 1, human_profile_note: 1, tags: 1, follow_up_policy: 1, operation_state: 1, agent_status: 1 } });
  console.log("AFTER", JSON.stringify(after));
  await c.close();
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
