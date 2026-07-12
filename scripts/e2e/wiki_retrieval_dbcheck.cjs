// #89 检索前置:查生产 DB 里 星零感 相关切片的核验状态分布
// 只有 status=active AND integrity_status=verified 的切片会进检索(knowledge_router.rs:63-79)
const { MongoClient } = require("mongodb");
const fs = require("fs");

function readEnv(k) {
  const txt = fs.readFileSync("/opt/wechatagent/.env", "utf8");
  const m = txt.match(new RegExp("^" + k + "=(.*)$", "m"));
  return m ? m[1].trim().replace(/^["']|["']$/g, "") : "";
}

(async () => {
  const uri = readEnv("MONGODB_URI") || process.env.MONGODB_URI;
  const dbName = readEnv("MONGODB_DATABASE") || process.env.MONGODB_DATABASE || "wechatagent";
  const cli = new MongoClient(uri);
  await cli.connect();
  const db = cli.db(dbName);
  const chunks = db.collection("operation_knowledge_chunks");

  const out = {};
  // 星零感/去眼袋/微孔 关键词命中的切片总览
  const kw = { $regex: "星零感|去眼袋|微孔|眼袋", $options: "i" };
  const q = { $or: [{ title: kw }, { body: kw }, { summary: kw }] };
  out.matched_total = await chunks.countDocuments(q);

  // 按 status × integrity_status 分组
  const grouped = await chunks.aggregate([
    { $match: q },
    { $group: { _id: { status: "$status", integrity: "$integrity_status" }, n: { $sum: 1 } } },
  ]).toArray();
  out.by_status_integrity = grouped.map((g) => ({ ...g._id, n: g.n }));

  // 可进检索的切片(active + verified)的 workspace/account/标题清单
  out.retrievable = await chunks.find(
    { ...q, status: "active", integrity_status: "verified" },
    { projection: { _id: 1, title: 1, workspace_id: 1, account_id: 1, chunk_type: 1, wiki_type: 1, source_anchors: 1 } }
  ).limit(40).toArray();
  out.retrievable = out.retrievable.map((c) => ({
    id: c._id.toString(),
    title: c.title,
    ws: c.workspace_id,
    acct: c.account_id,
    chunk_type: c.chunk_type,
    wiki_type: c.wiki_type,
    anchors: (c.source_anchors || []).length,
  }));
  out.retrievable_count = out.retrievable.length;

  // 若 verified=0,列出 needs_review 的标题(说明 #88 verify 是否真落库)
  if (out.retrievable_count === 0) {
    out.needs_review_sample = await chunks.find(
      { ...q, integrity_status: "needs_review" },
      { projection: { _id: 1, title: 1, workspace_id: 1, account_id: 1, status: 1 } }
    ).limit(20).toArray();
    out.needs_review_sample = out.needs_review_sample.map((c) => ({
      id: c._id.toString(), title: c.title, ws: c.workspace_id, acct: c.account_id, status: c.status,
    }));
  }

  console.log(JSON.stringify(out, null, 2));
  await cli.close();
})().catch((e) => { console.error("ERR", String(e)); process.exit(1); });
