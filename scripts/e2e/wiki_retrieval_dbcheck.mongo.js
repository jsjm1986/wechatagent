// #89 检索前置(mongosh 版):查 星零感 相关切片核验状态分布
// 只有 status=active AND integrity_status=verified 的切片进检索(knowledge_router.rs:63-79)
// 运行: mongosh mongodb://localhost:27017/<db> --quiet --file /tmp/wiki_retrieval_dbcheck.mongo.js
const chunks = db.getCollection("operation_knowledge_chunks");
const kw = { $regex: "星零感|去眼袋|微孔|眼袋", $options: "i" };
const q = { $or: [{ title: kw }, { body: kw }, { summary: kw }] };

const out = {};
out.db = db.getName();
out.matched_total = chunks.countDocuments(q);

out.by_status_integrity = chunks.aggregate([
  { $match: q },
  { $group: { _id: { status: "$status", integrity: "$integrity_status" }, n: { $sum: 1 } } },
]).toArray().map((g) => ({ status: g._id.status, integrity: g._id.integrity, n: g.n }));

out.retrievable = chunks.find(
  { $and: [q, { status: "active", integrity_status: "verified" }] },
  { title: 1, workspace_id: 1, account_id: 1, chunk_type: 1, wiki_type: 1, source_anchors: 1 }
).limit(40).toArray().map((c) => ({
  id: c._id.toString(),
  title: c.title,
  ws: c.workspace_id,
  acct: c.account_id,
  chunk_type: c.chunk_type,
  wiki_type: c.wiki_type,
  anchors: (c.source_anchors || []).length,
}));
out.retrievable_count = out.retrievable.length;

if (out.retrievable_count === 0) {
  out.needs_review_sample = chunks.find(
    { $and: [q, { integrity_status: "needs_review" }] },
    { title: 1, workspace_id: 1, account_id: 1, status: 1 }
  ).limit(20).toArray().map((c) => ({
    id: c._id.toString(), title: c.title, ws: c.workspace_id, acct: c.account_id, status: c.status,
  }));
}

print(JSON.stringify(out, null, 2));
