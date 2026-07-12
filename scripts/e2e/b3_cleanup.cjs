// 批3 清理：删 b3test 产品 + pull 掉 probe 联系人上我登记的 b3test 成交事件
const { MongoClient } = require("mongodb");
(async () => {
  const cli = new MongoClient(process.env.MONGODB_URI);
  await cli.connect();
  const db = cli.db(process.env.MONGODB_DATABASE);
  const delProd = await db.collection("products").deleteMany({ product_id: { $regex: "^b3test" } });
  const pull = await db.collection("contacts").updateOne(
    { _id: require("mongodb").ObjectId.createFromHexString("6a4f5b189d28a161324c2dd0") },
    { $pull: { outcome_events: { "product_ref.product_id": { $regex: "^b3test" } } } }
  );
  // 审计事件一并清（outcome_event_marked 由本次登记产生）
  const evDel = await db.collection("agent_events").deleteMany({ note: "b3test 走查登记" }).catch(() => ({ deletedCount: -1 }));
  console.log(JSON.stringify({ products_deleted: delProd.deletedCount, contact_modified: pull.modifiedCount, audit_maybe: evDel.deletedCount }, null, 2));
  await cli.close();
})().catch((e) => { console.error("ERR", String(e)); process.exit(1); });
