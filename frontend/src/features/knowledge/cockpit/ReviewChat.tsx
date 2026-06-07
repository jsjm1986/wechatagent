import { useState } from "react";
import { ShieldAlert } from "lucide-react";
import { canGoLive } from "../trustTypes";
import { useGoLive } from "./useGoLive";
import { StatusBadge } from "../../../components/ui/StatusBadge";
import styles from "./ReviewChat.module.css";

// 本组件 props 用本地最小接口(避免与 index.tsx 的 ReviewChunkItem 循环依赖)。
export interface ReviewChatChunk {
  id: string;
  title: string;
  summary?: string | null;
  body?: string | null;
  sourceQuote?: string | null;
  sourceAnchors?: unknown[] | null;
  integrityStatus?: string | null;
  status?: string | null;
  chunkType?: string | null;
  distortionRisks?: string[] | null;
  lockedFields?: string[] | null;
  usageStats?: { hitCount30d?: number; blockedCount30d?: number } | null;
  validFrom?: string | null;
  validTo?: string | null;
}

// 字段锁的英文键 → 大白话中文名;映射不到就显示原名。
const LOCKED_FIELD_LABELS: Record<string, string> = {
  sourceQuote: "原话出处",
  title: "标题",
  summary: "摘要",
  body: "正文",
  sourceAnchors: "来源锚点",
};

interface ReviewChatProps {
  chunk: ReviewChatChunk;
  onResolved: () => void;
}

interface ChatTurn {
  role: "operator" | "ai";
  text: string;
  // AI 改动提示:对话能让 AI 用 update_chunk 改这条草稿,但改完仍由运营放行,对话本身不 verify。
  touchedChunk?: boolean;
}

function clip(text: string | null | undefined, max = 180): string {
  if (!text) return "";
  const t = text.trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
}

export function ReviewChat({ chunk, onResolved }: ReviewChatProps) {
  const { goLive, pending } = useGoLive();
  const [sessionId, setSessionId] = useState<string | undefined>(undefined);
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [goLiveError, setGoLiveError] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const [rejecting, setRejecting] = useState(false);

  const usage = chunk.usageStats;
  const hasUsage =
    !!usage && (usage.hitCount30d != null || usage.blockedCount30d != null);
  const risks = (chunk.distortionRisks ?? []).filter((r) => !!r?.trim());
  const locks = (chunk.lockedFields ?? []).filter((f) => !!f?.trim());
  const hasValidity = !!chunk.validFrom || !!chunk.validTo;
  const hasMore = hasUsage || risks.length > 0 || locks.length > 0 || hasValidity;

  const hasQuote = !!chunk.sourceQuote?.trim();
  const hasAnchor = (chunk.sourceAnchors?.length ?? 0) > 0;
  const check = canGoLive({ hasQuote, hasAnchor });
  const isProductFact = chunk.chunkType === "product_fact";

  const handleGoLive = async () => {
    setGoLiveError(null);
    const r = await goLive({ sessionId, chunkId: chunk.id });
    if (r.ok) {
      onResolved();
      return;
    }
    setGoLiveError(
      r.reason === "gate_blocked"
        ? "还差来源信息,AI 暂时不能用这条。先把原话出处补齐。"
        : r.reason === "apply_failed"
          ? "对话里的改动没保存成功,再试一次。"
          : "出了点问题,稍后再试。"
    );
  };

  const handleReject = async () => {
    if (rejecting) return;
    setGoLiveError(null);
    setRejecting(true);
    try {
      const resp = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(chunk.id)}/reject`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
      );
      if (!resp.ok) {
        setGoLiveError("退回没成功，稍后再试。");
        return;
      }
      onResolved();
    } catch {
      setGoLiveError("退回没成功，稍后再试。");
    } finally {
      setRejecting(false);
    }
  };

  const handleSend = async () => {
    const content = draft.trim();
    if (!content || sending) return;
    setDraft("");
    setTurns((prev) => [...prev, { role: "operator", text: content }]);
    setSending(true);
    try {
      const resp = await fetch("/api/operation-knowledge/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          content,
          sessionId,
          attachments: [{ chunk_id: chunk.id }],
        }),
      });
      if (!resp.ok) {
        setTurns((prev) => [...prev, { role: "ai", text: "没接上,稍后再发一次。" }]);
        return;
      }
      const data = (await resp.json()) as Record<string, unknown>;
      if (typeof data.sessionId === "string") setSessionId(data.sessionId);

      const turn = (data.turn ?? data) as Record<string, unknown>;
      const naturalReply =
        typeof turn.naturalReply === "string"
          ? turn.naturalReply
          : typeof data.naturalReply === "string"
            ? data.naturalReply
            : typeof data.reply === "string"
              ? data.reply
              : "好的。";
      const touched = !!(turn.patch ?? data.patch);
      setTurns((prev) => [
        ...prev,
        { role: "ai", text: naturalReply, touchedChunk: touched },
      ]);
    } catch {
      setTurns((prev) => [...prev, { role: "ai", text: "没接上,稍后再发一次。" }]);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={styles.wrap}>
      {/* 左栏:裁决 + 放行 */}
      <section className={styles.verdictCol}>
        <header className={styles.verdictHead}>
          <StatusBadge tone={check.ok ? "running" : "held"}>
            {check.ok ? "材料齐了" : "还差一样"}
          </StatusBadge>
          <h3 className={styles.chunkTitle}>{chunk.title}</h3>
        </header>

        <div className={styles.block}>
          <span className={styles.blockLabel}>这条说的是</span>
          <p className={styles.blockBody}>
            {clip(chunk.summary || chunk.body) || "(还没有内容)"}
          </p>
        </div>

        <div className={styles.block}>
          <span className={styles.blockLabel}>这话的原话</span>
          {hasQuote ? (
            <blockquote className={styles.quote}>{chunk.sourceQuote}</blockquote>
          ) : (
            <p className={styles.muted}>还没填这话的原话</p>
          )}
        </div>

        {!check.ok && (
          <div className={styles.gapNote}>
            {check.missing.includes("anchor") && (
              <p>
                还不知道这话从哪来的,没法核对是否被改过,先弄清来源 AI 才能用。
              </p>
            )}
            {check.missing.includes("quote") && (
              <p>
                还没填原话出处,不知道这话最初是谁说的,补上 AI 才能用。
              </p>
            )}
          </div>
        )}

        {isProductFact && (
          <div className={styles.endorse}>
            <ShieldAlert size={15} />
            <span>放行后会成为 AI 对客的依据,请确认无误。</span>
          </div>
        )}

        {goLiveError && <p className={styles.goLiveError}>{goLiveError}</p>}

        <div className={styles.more}>
          <button
            type="button"
            className={styles.moreToggle}
            aria-expanded={moreOpen}
            onClick={() => setMoreOpen((v) => !v)}
          >
            {moreOpen ? "▾" : "▸"} 这条的更多信息(用量 / 痕迹)
          </button>
          <div className={moreOpen ? styles.moreBody : styles.moreBodyHidden}>
            {!hasMore && <p className={styles.muted}>暂无更多信息</p>}

            {hasUsage && (
              <div className={styles.moreItem}>
                <span className={styles.blockLabel}>最近 30 天</span>
                <p className={styles.moreText}>
                  AI 用了 {usage?.hitCount30d ?? 0} 次,被安全闸拦下{" "}
                  {usage?.blockedCount30d ?? 0} 次
                </p>
              </div>
            )}

            {risks.length > 0 && (
              <div className={styles.moreItem}>
                <span className={styles.blockLabel}>这条经历过什么</span>
                <ul className={styles.moreList}>
                  {risks.map((r, i) => (
                    <li key={i}>{r}</li>
                  ))}
                </ul>
              </div>
            )}

            {locks.length > 0 && (
              <div className={styles.moreItem}>
                <p className={styles.moreText}>
                  🔒 这些项被锁定改不了:
                  {locks.map((f) => LOCKED_FIELD_LABELS[f] ?? f).join("、")}
                </p>
              </div>
            )}

            {hasValidity && (
              <div className={styles.moreItem}>
                <p className={styles.moreText}>
                  有效期:{chunk.validFrom || "—"} ~ {chunk.validTo || "长期"}
                </p>
              </div>
            )}
          </div>
        </div>

        <div className={styles.actions}>
          <button
            type="button"
            className={styles.goLiveBtn}
            disabled={!check.ok || pending || rejecting}
            onClick={handleGoLive}
          >
            {pending ? "正在放行…" : "让 AI 可以用这条"}
          </button>
          <button
            type="button"
            className={styles.rejectBtn}
            disabled={rejecting || pending}
            onClick={handleReject}
          >
            {rejecting ? "退回中…" : "退回"}
          </button>
        </div>
      </section>

      {/* 右栏:对话工坊 */}
      <section className={styles.chatCol}>
        <header className={styles.chatHead}>
          <h4 className={styles.chatTitle}>问 AI 改这条</h4>
          <span className={styles.chatSub}>只动这条 · 改完仍由你放行</span>
        </header>

        <div className={styles.msgList}>
          {turns.length === 0 ? (
            <p className={styles.msgEmpty}>
              用大白话告诉 AI 怎么改,比如「把年费改成 12800,加一句含 5 个坐席」。
            </p>
          ) : (
            turns.map((t, i) => (
              <div
                key={i}
                className={t.role === "operator" ? styles.msgOp : styles.msgAi}
              >
                <p className={styles.msgText}>{t.text}</p>
                {t.touchedChunk && (
                  <span className={styles.msgHint}>← 改动可在左边确认</span>
                )}
              </div>
            ))
          )}
        </div>

        <div className={styles.composer}>
          <textarea
            className={styles.input}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="让 AI 改这条…"
            rows={2}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              }
            }}
          />
          <button
            type="button"
            className={styles.sendBtn}
            disabled={!draft.trim() || sending}
            onClick={() => void handleSend()}
          >
            {sending ? "发送中…" : "发送"}
          </button>
        </div>
      </section>
    </div>
  );
}
