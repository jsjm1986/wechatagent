// 批3 DB 核对：contacts / products(b3test) / suspected_deal_signals / outcome_events
const { MongoClient } = require("mongodb");
(async () => {
  const uri = process.env.MONGODB_URI;
  const dbName = process.env.MONGODB_DATABASE;
  const cli = new MongoClient(uri);
  await cli.connect();
  const db = cli.db(dbName);
  const out = {};
  out.contacts_total = await db.collection("contacts").countDocuments({});
  out.contacts_managed = await db.collection("contacts").countDocuments({ agent_status: "managed" });
  out.contacts_sample = await db.collection("contacts").find({}, { projection: { _id: 1, wxid: 1, agent_status: 1, workspace_id: 1, account_id: 1 } }).limit(3).toArray();
  out.products_b3test = await db.collection("products").find({ product_id: { $regex: "^b3test" } }, { projection: { product_id: 1, name: 1, price: 1, status: 1, workspace_id: 1 } }).toArray();
  out.products_total = await db.collection("products").countDocuments({});
  out.suspected_pending = await db.collection("suspected_deal_signals").countDocuments({ status: "pending" });
  out.suspected_total = await db.collection("suspected_deal_signals").countDocuments({});
  // askHuman sources
  out.taxonomy_candidates_pending = await db.collection("taxonomy_candidates").countDocuments({ status: "pending" }).catch(() => -1);
  out.knowledge_chunks_needs_review = await db.collection("operation_knowledge_chunks").countDocuments({ integrity_status: "needs_review" }).catch(() => -1);
  console.log(JSON.stringify(out, null, 2));
  await cli.close();
})().catch((e) => { console.error("ERR", String(e)); process.exit(1); });
