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

export function MemoryDetailView({
  memoryCard,
  onBack
}: {
  memoryCard?: Record<string, unknown>;
  onBack: () => void;
}) {
  const profile = memoryCard?.coreProfile as Record<string, unknown> | undefined;
  const relation = memoryCard?.relationshipState as Record<string, unknown> | undefined;

  const factItems = FACT_SECTIONS
    .map((section) => ({ ...section, facts: memoryFactList(memoryCard, section.key) }))
    .filter((section) => section.facts.length > 0);
  const plainItems = PLAIN_SECTIONS
    .map((section) => ({ ...section, values: contextPackList(memoryCard, section.key) }))
    .filter((section) => section.values.length > 0);

  const empty = !factItems.length && !plainItems.length && !profile && !relation;

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
                  stringField(relation || {}, "stage")
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
        </div>
      )}
    </section>
  );
}
