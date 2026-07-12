// 清理验证期间造的 draft 测试活动(title 含 "验证活动-fix-")。在117本机跑。
const { MongoClient } = require("mongodb");
const fs = require("fs");

function readEnv() {
  const txt = fs.readFileSync("/opt/wechatagent/.env", "utf8");
  const get = (k) => { const m = txt.match(new RegExp("^" + k + "=(.*)$", "m")); return m ? m[1].trim().replace(/^["']|["']$/g, "") : ""; };
  return { uri: get("MONGODB_URI"), db: get("MONGODB_DATABASE") };
}

(async () => {
  const { uri, db } = readEnv();
  const c = await MongoClient.connect(uri);
  const database = c.db(db);
  const coll = database.collection("campaigns");
  const q = { title: { $regex: "^验证活动-fix-" } };
  const found = await coll.find(q).project({ title: 1, account_id: 1, status: 1 }).toArray();
  console.log("匹配到 " + found.length + " 个验证活动:");
  for (const f of found) console.log("  - " + f.title + " (account=" + f.account_id + " status=" + f.status + ")");
  if (found.length > 0) {
    const res = await coll.deleteMany(q);
    console.log("已删除 " + res.deletedCount + " 个");
  }
  await c.close();
})().catch((e) => { console.error("ERR", e.message); process.exit(1); });
