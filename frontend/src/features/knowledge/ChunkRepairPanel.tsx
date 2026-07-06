import { useState } from "react";
import type { ChunkRepairProposal } from "./trustTypes";
import { chunkFieldLabel } from "./labels";
import { applyAiRepairPatch } from "../../lib/applyAiRepairPatch";

type RepairStatus = "idle" | "proposing" | "reviewing" | "answering" | "applying" | "done" | "error";

export function ChunkRepairPanel({
  chunkId,
  originalChunk,
  onApplied,
}: {
  chunkId: string;
  originalChunk: Record<string, unknown>;
  onApplied: () => void;
}) {
  const [status, setStatus] = useState<RepairStatus>("idle");
  const [proposal, setProposal] = useState<ChunkRepairProposal | null>(null);
  const [accepted, setAccepted] = useState<Set<string>>(new Set());
  const [answerDrafts, setAnswerDrafts] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  async function propose() {
    setStatus("proposing");
    setError(null);
    try {
      const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/repair`, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: "{}",
      });
      if (!r.ok) {
        setError("AI 修复建议生成失败（可能预算用尽），请稍后重试");
        setStatus("error");
        return;
      }
      const data = (await r.json()) as ChunkRepairProposal;
      setProposal(data);
      setAccepted(new Set(Object.keys(data.patch ?? {}).filter((k) => k !== "extras"))); // 默认全勾，运营可取消
      setStatus("reviewing");
    } catch {
      setError("AI 修复建议生成失败，请稍后重试");
      setStatus("error");
    }
  }

  function toggleField(name: string) {
    setAccepted((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name); else next.add(name);
      return next;
    });
  }

  async function answer() {
    if (!proposal) return;
    setStatus("answering");
    setError(null);
    try {
      const answers = proposal.followupQuestions.map((q) => ({
        id: q.id, field: q.field ?? null, text: answerDrafts[q.id] ?? "",
      }));
      const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/repair/answer`, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sessionId: proposal.sessionId, previousPatch: proposal.patch, answers, turn: proposal.turn,
        }),
      });
      if (!r.ok) { setError("追问应答失败，请稍后重试"); setStatus("error"); return; }
      const data = (await r.json()) as ChunkRepairProposal;
      setProposal(data);
      setAccepted(new Set(Object.keys(data.patch ?? {}).filter((k) => k !== "extras")));
      setAnswerDrafts({});
      setStatus("reviewing");
    } catch { setError("追问应答失败，请稍后重试"); setStatus("error"); }
  }

  async function apply() {
    if (!proposal) return;
    setStatus("applying");
    setError(null);
    const r = await applyAiRepairPatch({
      chunkId,
      originalChunk,
      patch: proposal.patch,
      acceptedFieldNames: [...accepted],
      sessionId: proposal.sessionId,
      turn: proposal.turn,
      confidenceHint: proposal.confidenceHint,
      extras: (proposal.patch as Record<string, unknown>).extras,
    });
    if (r.ok) {
      setStatus("done");
      onApplied();
    } else {
      setError(r.message ?? (r.reason === "apply_failed" ? "落库失败，请重试" : "操作失败，请重试"));
      setStatus("error");
    }
  }

  if (status === "idle") {
    return (
      <button type="button" className="wikiBtn" onClick={() => void propose()}>
        AI 修复建议
      </button>
    );
  }
  if (status === "proposing") return <div className="wikiHint">AI 正在分析这条切片…</div>;
  if (status === "applying") return <div className="wikiHint">正在落库…</div>;
  if (status === "done") return <div className="wikiAlert ok">已落库为草稿，可在上方「确认放行」按钮去核验。</div>;
  if (status === "error") return (
    <div className="wikiAlert error">
      {error}
      <button type="button" className="wikiBtn" onClick={() => void propose()}>重试</button>
    </div>
  );

  // reviewing（answer 区 Task3 补、落库按钮 Task4 补）
  const patchEntries = proposal ? Object.entries(proposal.patch ?? {}).filter(([k]) => k !== "extras") : [];
  return (
    <div className="wikiRepairPanel">
      {proposal?.interpretation ? (
        <div className="wikiRepairInterp">
          {Object.entries(proposal.interpretation).map(([k, v]) => (
            <span key={k} className="wikiArchiveTag">{k}: {String(v)}</span>
          ))}
        </div>
      ) : null}
      <div className="wikiRepairConfidence">AI 自评可信度：{proposal?.confidenceHint ?? 0}</div>
      <div className="wikiRepairFields">
        {patchEntries.map(([field, value]) => (
          <label key={field} className="wikiRepairField">
            <input type="checkbox" checked={accepted.has(field)} onChange={() => toggleField(field)} />
            <span className="wikiRepairFieldName">{chunkFieldLabel(field)}</span>
            <span className="wikiRepairFieldValue">{typeof value === "string" ? value : JSON.stringify(value)}</span>
          </label>
        ))}
      </div>
      {proposal && proposal.missingFields.length > 0 ? (
        <div className="wikiRepairMissing">
          仍缺：{proposal.missingFields.map((m) => chunkFieldLabel(m.field)).join("、")}
        </div>
      ) : null}
      {proposal && proposal.followupQuestions.length > 0 ? (
        <div className="wikiRepairFollowup">
          {proposal.followupQuestions.map((q) => (
            <label key={q.id} className="wikiRepairFollowupItem">
              <span>{q.question}</span>
              <input
                type="text"
                placeholder="回答 AI 的追问（可留空）"
                value={answerDrafts[q.id] ?? ""}
                onChange={(e) => setAnswerDrafts((p) => ({ ...p, [q.id]: e.target.value }))}
              />
            </label>
          ))}
          <button type="button" className="wikiBtn" onClick={() => void answer()}>提交回答</button>
        </div>
      ) : null}
      <button type="button" className="primary" disabled={accepted.size === 0} onClick={() => void apply()}>
        落库勾选字段（{accepted.size}）
      </button>
    </div>
  );
}
