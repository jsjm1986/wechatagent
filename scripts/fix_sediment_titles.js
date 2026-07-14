// 一次性修正：领导授权沉淀 / 待审核提案 chunk 的 title/body 误用 reviewer 质检点评。
// 根因见 docs/superpowers/specs/2026-07-15-sediment-title-from-substance-design.md。
//
// 跑法：mongosh wechatagent scripts/fix_sediment_titles.js
//   或  mongosh "<PROD_URI>" scripts/fix_sediment_titles.js
//
// 三段式：备份 → 逐条 update → 回读校验。
// - 备份写到 _sediment_title_backup_<ts> 集合便于回滚。
// - title 重算逻辑与 src/agent/escalation/ledger.rs `derive_sediment_title_fallback`
//   确定性等价：取首句（截到第一个句末标点 。！？!? 或换行之前），按 chars 限长 40，
//   超长加省略号；空则固定安全标题「领导授权沉淀」。
// - B 类（title 以「领导授权沉淀：」开头，status=active）：重算 title 无前缀。
// - draft 类（title 以「真人决策沉淀（待审核）：」开头）：重算 title 加「待审核：」前缀。
// - body 去掉「卡点：」那一行。

// —— 与 derive_sediment_title_fallback 等价的确定性纯逻辑 ——
function deriveTitleFallback(substance) {
  const trimmed = (substance || "").trim();
  if (trimmed.length === 0) {
    return "领导授权沉淀";
  }
  // 首句：截到第一个句末标点 / 换行之前。
  let first = trimmed;
  for (let i = 0; i < trimmed.length; i++) {
    const ch = trimmed[i];
    if (ch === "。" || ch === "！" || ch === "？" || ch === "!" || ch === "?" || ch === "\n") {
      first = trimmed.slice(0, i);
      break;
    }
  }
  first = first.trim();
  if (first.length === 0) {
    first = trimmed;
  }
  // 按 chars（Unicode code point）限长 40，多字节安全。
  const chars = Array.from(first);
  if (chars.length > 40) {
    return chars.slice(0, 40).join("") + "…";
  }
  return chars.join("");
}

// 从 chunk 提取 substance：优先 source_quote（B 类有），否则从 body 的「领导裁决：」段取。
function extractSubstance(chunk) {
  if (chunk.source_quote && chunk.source_quote.trim().length > 0) {
    return chunk.source_quote.trim();
  }
  const body = chunk.body || "";
  const lines = body.split("\n");
  for (const line of lines) {
    const idx = line.indexOf("领导裁决：");
    if (idx >= 0) {
      return line.slice(idx + "领导裁决：".length).trim();
    }
  }
  return "";
}

// 去掉 body 里「卡点：」段：从以「卡点：」开头的行起，删到下一个已知字段
// （「领导裁决：」/「约束：」）或结尾之前的所有行。对多行 reason（LLM 生成的
// 多行质检点评）健壮，不只删首行。
function stripBlockerLine(body) {
  if (!body) return body;
  const lines = body.split("\n");
  const out = [];
  let inBlocker = false;
  for (const line of lines) {
    if (line.startsWith("卡点：")) {
      inBlocker = true;
      continue;
    }
    if (inBlocker) {
      // 遇到下一个已知字段行 → 卡点段结束，恢复保留。
      if (line.startsWith("领导裁决：") || line.startsWith("约束：")) {
        inBlocker = false;
        out.push(line);
      }
      // 否则仍在卡点段（多行 reason 续行）→ 丢弃。
      continue;
    }
    out.push(line);
  }
  return out.join("\n");
}

const B_PREFIX = "领导授权沉淀：";
const DRAFT_PREFIX = "真人决策沉淀（待审核）：";

const now = new Date();
const tsSuffix = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, "0")}${String(now.getDate()).padStart(2, "0")}_${String(now.getHours()).padStart(2, "0")}${String(now.getMinutes()).padStart(2, "0")}`;
const backupName = `_sediment_title_backup_${tsSuffix}`;

print(`\n=== 沉淀标题修正 (${now.toISOString()}) ===`);

// 1) 找出目标：title 以两种污染前缀之一开头。
const targets = db.operation_knowledge_chunks
  .find({
    $or: [
      { title: { $regex: `^${B_PREFIX}` } },
      { title: { $regex: "^真人决策沉淀（待审核）：" } },
    ],
  })
  .toArray();

print(`命中 ${targets.length} 条待修正 chunk`);

if (targets.length === 0) {
  print("无需修正，退出。");
} else {
  // 2) 备份到独立集合（便于回滚）。
  db[backupName].insertMany(targets);
  print(`已备份到 ${backupName}`);

  // 3) 逐条重算 title + 去卡点行。
  let updated = 0;
  targets.forEach((chunk) => {
    const substance = extractSubstance(chunk);
    const raw = deriveTitleFallback(substance);
    let newTitle;
    if (chunk.title.startsWith(DRAFT_PREFIX)) {
      newTitle = `待审核：${raw}`;
    } else {
      newTitle = raw; // B 类无前缀
    }
    const newBody = stripBlockerLine(chunk.body);
    const res = db.operation_knowledge_chunks.updateOne(
      { _id: chunk._id },
      { $set: { title: newTitle, body: newBody } }
    );
    updated += res.modifiedCount;
    print(`  ${chunk._id}: title="${newTitle}"`);
  });
  print(`已更新 ${updated} 条`);

  // 4) 回读校验。
  print("\n=== 回读校验 ===");
  db.operation_knowledge_chunks
    .find({ _id: { $in: targets.map((t) => t._id) } })
    .forEach((c) => print(`  ${c._id}: title="${c.title}" | body首行="${(c.body || "").split("\n")[0]}"`));
}
