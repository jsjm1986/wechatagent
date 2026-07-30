// 记忆全景下钻（Task 5）。把 MemoryCardSummary 里被压成纯文本徽标的溯源结构
// 完整铺开：事实分区（核心 / 近期 / 已弃用）每条显 confidence/importance 徽标、
// 易失效标记、证据展开、弃用则显 deprecatedAt + deprecationReason；
// 加偏好/异议/承诺/禁忌/待办/记忆冲突纯文本分区 + 核心画像。
// 复用 legacy 的 canonical helper（memoryFactList / MemoryFactRow / contextPackList / stringField），
// 不重写数据抽取逻辑，保证与 MemoryCardSummary 的兼容语义一致（含 coreFacts 字符串旧形态）。
import { ArrowLeft } from "lucide-react";
import {
  EmptyInline,
  MemoryFactRow,
  contextPackList,
  memoryFactList,
  stringField
} from "../../legacy";
import { useProfileStore, labelFor } from "../../../../stores/profileStore";
import styles from "../cockpit.module.css";

const FACT_SECTIONS = [
  { key: "coreFacts", label: "核心事实" },
  { key: "recentFacts", label: "近期事实" },
  { key: "deprecatedFacts", label: "已过期事实" }
] as const;

const PLAIN_SECTIONS = [
  { key: "preferences", label: "偏好" },
  { key: "objections", label: "异议" },
  { key: "commitments", label: "承诺" },
  { key: "doNotDo", label: "禁忌" },
  { key: "openLoops", label: "待办" },
  { key: "conflicts", label: "记忆冲突" }
] as const;

type CoreFactEvictionView = {
  text: string;
  reason: string;
  evictedAt?: string;
  coreFactRank?: number;
};

function coreFactEvictions(memoryCard?: Record<string, unknown>): CoreFactEvictionView[] {
  const value = memoryCard?.coreFactEvictions;
  if (!Array.isArray(value)) return [];
  return value
    .map((item): CoreFactEvictionView | null => {
      if (!item || typeof item !== "object") return null;
      const record = item as Record<string, unknown>;
      const text = stringField(record, "text").trim();
      if (!text) return null;
      return {
        text,
        reason: stringField(record, "reason").trim(),
        evictedAt: stringField(record, "evictedAt").trim() || undefined,
        coreFactRank: typeof record.coreFactRank === "number" ? record.coreFactRank : undefined
      };
    })
    .filter((item): item is CoreFactEvictionView => item !== null);
}

export function MemoryDetailView({
  memoryCard,
  onBack
}: {
  memoryCard?: Record<string, unknown>;
  onBack: () => void;
}) {
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const profile = memoryCard?.coreProfile as Record<string, unknown> | undefined;
  const relation = memoryCard?.relationshipState as Record<string, unknown> | undefined;
  const stageRaw = stringField(relation || {}, "stage");
  const stageLabel = stageRaw ? labelFor(taxonomies, "customer_stage", stageRaw).text : "";

  const factItems = FACT_SECTIONS
    .map((section) => ({ ...section, facts: memoryFactList(memoryCard, section.key) }))
    .filter((section) => section.facts.length > 0);
  const plainItems = PLAIN_SECTIONS
    .map((section) => ({ ...section, values: contextPackList(memoryCard, section.key) }))
    .filter((section) => section.values.length > 0);
  const evictions = coreFactEvictions(memoryCard);

  const empty = !factItems.length && !plainItems.length && !evictions.length && !profile && !relation;

  return (
    <section className="smartTabPanel">
      <div className={styles.drilldownHead}>
        <button className={styles.backButton} type="button" onClick={onBack}>
          <ArrowLeft size={15} />
          返回
        </button>
        <strong>长期记忆</strong>
      </div>

      {empty ? (
        <section className="cockpitSection">
          <EmptyInline text="还没有形成长期记忆。下一次真实对话或模拟验证后会生成。" />
        </section>
      ) : (
        <div className="contextPackGrid">
          {(profile || relation) && (
            <div>
              <span>核心画像</span>
              <p>
                {[
                  stringField(profile || {}, "identity"),
                  stringField(profile || {}, "businessContext"),
                  stringField(profile || {}, "communicationStyle"),
                  stageLabel
                ]
                  .filter(Boolean)
                  .join(" / ") || "待确认"}
              </p>
            </div>
          )}
          {factItems.map((section) => (
            <div key={section.key}>
              <span>{section.label}</span>
              {section.facts.map((fact, index) => (
                <div key={`${section.key}-${index}`}>
                  <MemoryFactRow fact={fact} />
                  {fact.deprecatedAt && (
                    <p className={styles.factDeprecatedNote}>
                      已弃用 · {fact.deprecatedAt}
                      {fact.deprecationReason ? `（${fact.deprecationReason}）` : ""}
                    </p>
                  )}
                </div>
              ))}
            </div>
          ))}
          {plainItems.map((section) => (
            <div key={section.key}>
              <span>{section.label}</span>
              {section.values.map((value, index) => (
                <p key={`${section.key}-${index}`}>{value}</p>
              ))}
            </div>
          ))}
          {evictions.length > 0 && (
            <div>
              <span>核心事实归档</span>
              {evictions.map((eviction, index) => (
                <div className={styles.factEviction} key={`${eviction.text}-${index}`}>
                  <p>{eviction.text}</p>
                  <p className={styles.factEvictionNote}>
                    {eviction.reason === "core_fact_capacity"
                      ? "因核心事实窗口上限归档"
                      : "已从核心事实窗口归档"}
                    {typeof eviction.coreFactRank === "number" ? ` · 原排名 ${eviction.coreFactRank}` : ""}
                    {eviction.evictedAt ? ` · ${eviction.evictedAt}` : ""}
                  </p>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
