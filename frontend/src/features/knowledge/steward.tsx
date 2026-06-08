import { useState, useEffect, useCallback, useMemo, Fragment, type FormEvent } from "react";
import {
  Archive,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  MessageSquareText,
  RefreshCw,
  Search,
  Undo2,
  Workflow,
  X,
} from "lucide-react";
import { parseApiError } from "../../lib/api";
import { parseCompleteness, parseIntegrityReport, type CompletenessView, type IntegrityReportView } from "./trustTypes";
import { ChunkInspectorPane, classifyChunk, focusChunk, type ReviewChunkItem, type ReviewCategory } from "./shared";
import { ReviewChat, type ReviewChatChunk } from "./cockpit/ReviewChat";

// ── G2 · DocumentsView · 知识文档目录 CRUD ─────────────────────────────
interface DocumentItem {
  id: string;
  title: string;
  summary?: string | null;
  domain?: string | null;
  sourceType?: string | null;
  sourceName?: string | null;
  status?: string | null;
  catalogSummary?: string | null;
  updatedAt?: string | null;
  routingMap?: string[] | null;
  productTags?: string[] | null;
  businessTopics?: string[] | null;
}

interface DocumentChunkRow {
  id: string;
  title?: string | null;
  wikiType?: string | null;
  status?: string | null;
  integrityStatus?: string | null;
  summary?: string | null;
  updatedAt?: string | null;
}

export function DocumentsView() {
  const [items, setItems] = useState<DocumentItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState({ title: "", summary: "", sourceName: "", sourceType: "imported_markdown" });
  // 行内展开：documentId → 子表状态。后端 GET /documents/:id/chunks
  // 已存在但前端未挂；这里按需 lazy-load，节省默认列表渲染开销。
  const [expandedDoc, setExpandedDoc] = useState<string | null>(null);
  const [docChunks, setDocChunks] = useState<Record<string, DocumentChunkRow[]>>({});
  const [docChunksLoading, setDocChunksLoading] = useState<string | null>(null);
  const [docChunksError, setDocChunksError] = useState<string | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/documents");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: DocumentItem[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void load(); }, []);

  async function handleCreate(ev: FormEvent) {
    ev.preventDefault();
    if (!draft.title.trim()) return;
    setCreating(true);
    try {
      const r = await fetch("/api/operation-knowledge/documents", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          domain: "user_operations",
          title: draft.title.trim(),
          summary: draft.summary.trim() || null,
          sourceName: draft.sourceName.trim() || null,
          sourceType: draft.sourceType,
          status: "draft"
        })
      });
      if (!r.ok) throw await parseApiError(r);
      setDraft({ title: "", summary: "", sourceName: "", sourceType: "imported_markdown" });
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  async function handleDelete(id: string) {
    if (!window.confirm(`删除文档？关联 chunks 不会被删除，但失去文档归属。`)) return;
    try {
      const r = await fetch(`/api/operation-knowledge/documents/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleDocChunks(docId: string) {
    if (expandedDoc === docId) {
      setExpandedDoc(null);
      return;
    }
    setExpandedDoc(docId);
    if (docChunks[docId]) return; // 已缓存
    setDocChunksLoading(docId);
    setDocChunksError(null);
    try {
      const r = await fetch(`/api/operation-knowledge/documents/${encodeURIComponent(docId)}/chunks`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: DocumentChunkRow[] };
      setDocChunks({ ...docChunks, [docId]: data.items ?? [] });
    } catch (e) {
      setDocChunksError(String(e));
    } finally {
      setDocChunksLoading(null);
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Documents · 文档目录</span>
        <h3 style={{ fontSize: 20 }}>知识文档</h3>
      </header>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      <form onSubmit={handleCreate} style={{ display: "grid", gridTemplateColumns: "1fr 1fr 200px auto", gap: 8, marginBottom: 16 }}>
        <input
          type="text"
          placeholder="文档标题（必填）"
          value={draft.title}
          onChange={(e) => setDraft({ ...draft, title: e.target.value })}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="摘要"
          value={draft.summary}
          onChange={(e) => setDraft({ ...draft, summary: e.target.value })}
          className="wikiInput"
        />
        <select
          value={draft.sourceType}
          onChange={(e) => setDraft({ ...draft, sourceType: e.target.value })}
          className="wikiInput"
        >
          <option value="imported_markdown">imported_markdown</option>
          <option value="manual">manual</option>
          <option value="external_url">external_url</option>
          <option value="archived">archived</option>
        </select>
        <button type="submit" className="wikiBtn" disabled={creating || !draft.title.trim()}>
          {creating ? "保存中…" : "新建"}
        </button>
      </form>
      {loading ? <div className="wikiHint">加载中…</div> : items.length === 0 ? (
        <div className="wikiHint">还没有文档。新建第一份，或使用导入向导。</div>
      ) : (
        <table className="wikiTable" style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>标题</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>来源</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>状态</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>更新</th>
              <th style={{ padding: "8px 6px", borderBottom: "1px solid var(--line)" }}></th>
            </tr>
          </thead>
          <tbody>
            {items.map((d) => (
              <Fragment key={d.id}>
              <tr style={{ borderBottom: "1px solid var(--line)" }}>
                <td style={{ padding: "10px 6px" }}>
                  <div style={{ fontFamily: "var(--font-display)", fontWeight: 600 }}>{d.title}</div>
                  {d.summary ? <div style={{ color: "var(--muted)", fontSize: 12, marginTop: 2 }}>{d.summary}</div> : null}
                  <div style={{ marginTop: 4 }}>
                    {(d.businessTopics ?? []).map((t, i) => <span key={i} className="wikiArchiveTag">{t}</span>)}
                  </div>
                </td>
                <td style={{ padding: "10px 6px", fontFamily: "var(--font-mono)", fontSize: 12 }}>
                  <div>{d.sourceType ?? "—"}</div>
                  <div style={{ color: "var(--muted)" }}>{d.sourceName ?? ""}</div>
                </td>
                <td style={{ padding: "10px 6px" }}>
                  <span className="wikiBadge">{d.status ?? "—"}</span>
                </td>
                <td style={{ padding: "10px 6px", fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)" }}>
                  {d.updatedAt ? new Date(d.updatedAt).toLocaleString() : "—"}
                </td>
                <td style={{ padding: "10px 6px", textAlign: "right", whiteSpace: "nowrap" }}>
                  <button
                    type="button"
                    className="wikiArchiveRollback"
                    style={{ marginRight: 6 }}
                    onClick={() => void toggleDocChunks(d.id)}
                  >
                    {expandedDoc === d.id ? "收起 chunks" : "查看 chunks"}
                  </button>
                  <button type="button" className="wikiArchiveRollback" onClick={() => handleDelete(d.id)}>删除</button>
                </td>
              </tr>
              {expandedDoc === d.id ? (
                <tr>
                  <td colSpan={5} style={{ background: "var(--surface-2, #f4efe5)", padding: "10px 14px" }}>
                    {docChunksLoading === d.id ? (
                      <div className="wikiHint">正在拉 chunks…</div>
                    ) : docChunksError ? (
                      <div className="wikiAlert error">{docChunksError}</div>
                    ) : (docChunks[d.id]?.length ?? 0) === 0 ? (
                      <div className="wikiHint">该文档下还没有 chunks。可走导入向导或手工新建。</div>
                    ) : (
                      <div style={{ display: "grid", gap: 4 }}>
                        <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)", marginBottom: 4 }}>
                          {docChunks[d.id].length} chunks · 点击编号跳到 ChunkInspectorPane
                        </div>
                        {docChunks[d.id].map((c) => (
                          <button
                            key={c.id}
                            type="button"
                            className="wikiSignalChunkBtn"
                            onClick={() => focusChunk(c.id)}
                            style={{ textAlign: "left", display: "grid", gridTemplateColumns: "120px 100px 1fr auto", gap: 8, alignItems: "center" }}
                          >
                            <span className="wikiArchiveTag">{c.wikiType ?? "—"}</span>
                            <span style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>
                              {c.status ?? "—"} / {c.integrityStatus ?? "—"}
                            </span>
                            <span style={{ fontFamily: "var(--font-display)", fontWeight: 600 }}>
                              {c.title ?? "（无标题）"}
                            </span>
                            <code style={{ fontSize: 10, color: "var(--muted)" }}>{c.id.slice(-6)}</code>
                          </button>
                        ))}
                      </div>
                    )}
                  </td>
                </tr>
              ) : null}
              </Fragment>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

// ── G2 · ImportWizard · 三步条粘贴 → 预览 → 应用 ───────────────────────
interface ImportPreviewChunk {
  title?: string | null;
  body?: string | null;
  summary?: string | null;
  wikiType?: string | null;
  businessTopics?: string[] | null;
  productTags?: string[] | null;
  routingCard?: string | null;
}

interface ImportPreviewResult {
  document?: { title?: string; summary?: string; catalogSummary?: string } | null;
  items?: unknown[];
  chunks?: ImportPreviewChunk[];
}

export function ImportWizard() {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [content, setContent] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreviewResult | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [edits, setEdits] = useState<Record<number, Partial<ImportPreviewChunk>>>({});
  const [created, setCreated] = useState<string[]>([]);
  // G-后续/4：AI 重抽 tags 按钮单条 loading 标记。
  const [retagging, setRetagging] = useState<number | null>(null);

  async function retagCandidate(i: number) {
    const merged = { ...(preview?.chunks ?? [])[i], ...(edits[i] ?? {}) };
    if (!merged.body || !merged.body.trim()) {
      setError("候选无正文，无法重抽 tags");
      return;
    }
    setRetagging(i);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/extract-tags", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          title: merged.title ?? null,
          body: merged.body,
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { productTags?: string[]; businessTopics?: string[] };
      setEdits({
        ...edits,
        [i]: {
          ...(edits[i] ?? {}),
          productTags: data.productTags ?? [],
          businessTopics: data.businessTopics ?? [],
        },
      });
    } catch (e) {
      setError(`重抽 tags 失败：${e}`);
    } finally {
      setRetagging(null);
    }
  }

  async function runPreview() {
    if (!content.trim()) return;
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/import-preview", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ content, sourceName: sourceName.trim() || null })
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as ImportPreviewResult;
      setPreview(data);
      const all = new Set<number>();
      (data.chunks ?? []).forEach((_, i) => all.add(i));
      setSelected(all);
      setEdits({});
      setStep(2);
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  async function runApply() {
    if (!preview) return;
    setPending(true);
    setError(null);
    try {
      const finalChunks = (preview.chunks ?? [])
        .map((c, i) => ({ ...c, ...(edits[i] ?? {}) }))
        .filter((_, i) => selected.has(i));
      const payload = {
        document: preview.document,
        items: preview.items,
        chunks: finalChunks,
        sourceName: sourceName.trim() || null
      };
      const r = await fetch("/api/operation-knowledge/import-apply", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload)
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { createdChunkIds?: string[]; created_chunk_ids?: string[] };
      const ids = data.createdChunkIds ?? data.created_chunk_ids ?? [];
      setCreated(ids);
      setStep(3);
      if (ids[0]) {
        setTimeout(() => focusChunk(ids[0]), 100);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  function reset() {
    setStep(1); setContent(""); setSourceName(""); setPreview(null);
    setSelected(new Set()); setEdits({}); setCreated([]); setError(null);
  }

  function toggle(i: number) {
    const next = new Set(selected);
    if (next.has(i)) next.delete(i); else next.add(i);
    setSelected(next);
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Import Wizard · 文档导入</span>
        <h3 style={{ fontSize: 20 }}>导入向导</h3>
      </header>
      <div className="wikiImportStepper">
        {[
          { n: 1, label: "粘贴" },
          { n: 2, label: "预览" },
          { n: 3, label: "应用" }
        ].map((s) => (
          <div key={s.n} className={`wikiImportStep${step === s.n ? " active" : ""}${step > s.n ? " done" : ""}`}>
            <span className="wikiImportStepNum">{s.n}</span> {s.label}
          </div>
        ))}
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {step === 1 ? (
        <div style={{ marginTop: 12 }}>
          <input
            type="text"
            placeholder="来源名称（可选）：例如「运营手册 v3」"
            value={sourceName}
            onChange={(e) => setSourceName(e.target.value)}
            className="wikiInput"
            style={{ width: "100%", marginBottom: 8 }}
          />
          <textarea
            placeholder="粘贴 markdown / 长文本…"
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={14}
            className="wikiInput"
            style={{ width: "100%", fontFamily: "var(--font-mono)", fontSize: 12 }}
          />
          <div style={{ marginTop: 10, display: "flex", gap: 8 }}>
            <button type="button" className="wikiBtn" onClick={runPreview} disabled={pending || !content.trim()}>
              {pending ? "解析中…" : "下一步：预览"}
            </button>
            <span style={{ color: "var(--muted)", fontSize: 12, alignSelf: "center" }}>
              将由 AI 拆为候选 chunk，所有 chunk 默认 status=draft + integrity_status=needs_review。
            </span>
          </div>
          <div style={{ marginTop: 14, padding: "10px 12px", border: "1px dashed var(--border)", borderRadius: 6 }}>
            <div style={{ fontSize: 12, color: "var(--muted)", marginBottom: 6 }}>
              · multimodal 入口（绕过 AI preview，直接 fence 解析 / vision 抽取，落 status=draft）
            </div>
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
              <label className="wikiBtn" style={{ cursor: pending ? "not-allowed" : "pointer", margin: 0 }}>
                {pending ? "上传中…" : "上传 PDF"}
                <input
                  type="file"
                  accept="application/pdf,.pdf"
                  hidden
                  disabled={pending}
                  onChange={async (ev) => {
                    const f = ev.target.files?.[0];
                    if (!f) return;
                    setPending(true);
                    setError(null);
                    try {
                      const fd = new FormData();
                      fd.append("file", f);
                      fd.append("sourceName", sourceName.trim() || f.name);
                      const r = await fetch("/api/operation-knowledge/import-apply-pdf", {
                        method: "POST",
                        body: fd,
                      });
                      if (!r.ok) throw await parseApiError(r);
                      const data = (await r.json()) as { chunkIds?: string[]; fallbackBlob?: boolean };
                      setCreated(data.chunkIds ?? []);
                      setStep(3);
                      if ((data.chunkIds ?? [])[0]) {
                        setTimeout(() => focusChunk(data.chunkIds![0]), 100);
                      }
                    } catch (e) {
                      setError(`PDF 导入失败：${e}`);
                    } finally {
                      setPending(false);
                      ev.target.value = "";
                    }
                  }}
                />
              </label>
              <label className="wikiBtn" style={{ cursor: pending ? "not-allowed" : "pointer", margin: 0 }}>
                {pending ? "上传中…" : "上传图片（vision）"}
                <input
                  type="file"
                  accept="image/*"
                  hidden
                  disabled={pending}
                  onChange={async (ev) => {
                    const f = ev.target.files?.[0];
                    if (!f) return;
                    setPending(true);
                    setError(null);
                    try {
                      const buf = await f.arrayBuffer();
                      let bin = "";
                      const u8 = new Uint8Array(buf);
                      for (let i = 0; i < u8.byteLength; i++) bin += String.fromCharCode(u8[i]);
                      const b64 = btoa(bin);
                      const r = await fetch("/api/operation-knowledge/import-apply-image", {
                        method: "POST",
                        headers: { "content-type": "application/json" },
                        body: JSON.stringify({
                          imageBase64: b64,
                          mime: f.type || "image/png",
                          sourceName: sourceName.trim() || f.name,
                        }),
                      });
                      if (!r.ok) throw await parseApiError(r);
                      const data = (await r.json()) as { chunkIds?: string[]; note?: string };
                      setCreated(data.chunkIds ?? []);
                      setStep(3);
                      if (data.note) setError(`vision 提示：${data.note}`);
                      if ((data.chunkIds ?? [])[0]) {
                        setTimeout(() => focusChunk(data.chunkIds![0]), 100);
                      }
                    } catch (e) {
                      setError(`图片导入失败：${e}（需文字主模型支持图片，或在模型设置里指派一个「视觉模型」）`);
                    } finally {
                      setPending(false);
                      ev.target.value = "";
                    }
                  }}
                />
              </label>
            </div>
          </div>
        </div>
      ) : null}
      {step === 2 && preview ? (
        <div style={{ marginTop: 12 }}>
          {preview.document ? (
            <dl className="wikiArchiveMeta">
              <dt>文档标题</dt><dd>{preview.document.title}</dd>
              {preview.document.summary ? (<><dt>摘要</dt><dd>{preview.document.summary}</dd></>) : null}
              {preview.document.catalogSummary ? (<><dt>目录摘要</dt><dd>{preview.document.catalogSummary}</dd></>) : null}
            </dl>
          ) : null}
          <hr className="wikiArchiveRule" />
          <div style={{ marginBottom: 8, fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--muted)" }}>
            候选 chunks · {selected.size}/{(preview.chunks ?? []).length} 已选
          </div>
          <div style={{ display: "grid", gap: 10 }}>
            {(preview.chunks ?? []).map((c, i) => {
              const e = edits[i] ?? {};
              const merged = { ...c, ...e };
              const isOn = selected.has(i);
              return (
                <div key={i} className={`wikiImportCandidate${isOn ? " selected" : ""}`}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <input type="checkbox" checked={isOn} onChange={() => toggle(i)} />
                    <span className="wikiArchiveTag">{merged.wikiType ?? "—"}</span>
                    <input
                      type="text"
                      value={merged.title ?? ""}
                      onChange={(ev) => setEdits({ ...edits, [i]: { ...e, title: ev.target.value } })}
                      className="wikiInput"
                      style={{ flex: 1, fontFamily: "var(--font-display)", fontWeight: 600 }}
                    />
                  </div>
                  {merged.summary ? <p style={{ color: "var(--muted)", fontSize: 12.5, margin: "6px 0 4px" }}>{merged.summary}</p> : null}
                  <div className="wikiImportChips">
                    {(merged.businessTopics ?? []).map((t, j) => <span key={`bt-${j}`} className="wikiArchiveTag">{t}</span>)}
                    {(merged.productTags ?? []).map((t, j) => <span key={`pt-${j}`} className="wikiArchiveTag" style={{ borderStyle: "dashed" }}>{t}</span>)}
                    <button
                      type="button"
                      className="wikiArchiveRollback"
                      style={{ marginLeft: "auto", fontSize: 11, padding: "2px 8px" }}
                      onClick={() => void retagCandidate(i)}
                      disabled={retagging === i || !merged.body}
                      title="调 /extract-tags 重抽 productTags / businessTopics"
                    >
                      {retagging === i ? "AI 抽取中…" : "AI 重抽 tags"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
          <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
            <button type="button" className="wikiBtn" onClick={() => setStep(1)}>← 返回粘贴</button>
            <button type="button" className="wikiBtn primary" onClick={runApply} disabled={pending || selected.size === 0}>
              {pending ? "应用中…" : `应用 ${selected.size} 条 →`}
            </button>
          </div>
        </div>
      ) : null}
      {step === 3 ? (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontFamily: "var(--font-display)", fontSize: 16, marginBottom: 8 }}>
            ✓ 已存入 {created.length} 条草稿
          </div>
          <p style={{ color: "var(--muted)", fontSize: 12.5 }}>
            这些都是草稿,AI 还<strong>不能</strong>拿去跟客户说。需要你逐条看过、确认没问题后,AI 才会用它们。
          </p>
          <div style={{ marginTop: 8, display: "grid", gap: 4 }}>
            {created.map((id) => (
              <button key={id} type="button" className="wikiSignalChunkBtn" onClick={() => focusChunk(id)}>
                <code>{id}</code>
              </button>
            ))}
          </div>
          <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
            <button
              type="button"
              className="wikiBtn"
              onClick={() => window.dispatchEvent(new CustomEvent("wikiOpenCockpit"))}
            >
              去治理总览逐条处理 →
            </button>
            <button type="button" className="wikiBtn" onClick={reset}>导入更多</button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

// ── G-后续/3 · TryRecallView · "按 catalog 试召" 诊断 ─────────────────────
// 调 POST /tools/search（输入 query → 命中 chunk_ids），再 POST /tools/open-slice
// 拉具体 chunk 详情。开发者诊断 grounding：看哪些 chunk 被检索器选上、为什么。
interface TryRecallTraceStep {
  step?: string | number | null;
  tool?: string | null;
  input?: unknown;
  output?: unknown;
  notes?: string | null;
}
interface TryRecallRouteResult {
  neededCategories?: string[] | null;
  selectedKnowledgeIds?: string[] | null;
  selectedDocumentIds?: string[] | null;
  selectedChunkIds?: string[] | null;
  selectedSliceReasons?: string[] | null;
  riskLevel?: string | null;
  requiresEvidence?: boolean | null;
  knowledgeCoverage?: string | null;
  missingKnowledge?: string[] | null;
  reason?: string | null;
  toolTrace?: TryRecallTraceStep[] | null;
  evidenceExcerpts?: string[] | null;
}
interface TryRecallSliceItem {
  id: string;
  title?: string | null;
  wikiType?: string | null;
  status?: string | null;
  integrityStatus?: string | null;
  summary?: string | null;
  body?: string | null;
}

export function TryRecallView() {
  const [accountId, setAccountId] = useState("default");
  const [contactId, setContactId] = useState("");
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [route, setRoute] = useState<TryRecallRouteResult | null>(null);
  const [slices, setSlices] = useState<TryRecallSliceItem[]>([]);
  const [openingSlices, setOpeningSlices] = useState(false);

  async function runSearch() {
    if (!query.trim()) return;
    setPending(true);
    setError(null);
    setRoute(null);
    setSlices([]);
    try {
      const r = await fetch("/api/operation-knowledge/tools/search", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          accountId: accountId.trim() || "default",
          contactId: contactId.trim() || null,
          query: query.trim(),
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { item?: TryRecallRouteResult };
      const item = data.item ?? null;
      setRoute(item);
      const ids = item?.selectedChunkIds ?? [];
      if (ids.length > 0) {
        setOpeningSlices(true);
        try {
          const r2 = await fetch("/api/operation-knowledge/tools/open-slice", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ ids }),
          });
          if (!r2.ok) throw await parseApiError(r2);
          const d2 = (await r2.json()) as { items?: TryRecallSliceItem[] };
          setSlices(d2.items ?? []);
        } catch (e2) {
          setError(`检索结果已返回，但 open-slice 失败：${e2}`);
        } finally {
          setOpeningSlices(false);
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Try Recall · 试召诊断</span>
        <h3 style={{ fontSize: 20 }}>按 catalog 试召</h3>
      </header>
      <p style={{ color: "var(--muted)", fontSize: 12.5, margin: "0 0 12px" }}>
        给定 accountId（可选 contactId）和一句话 query，调用知识路由器看哪些 chunk 被选上，
        以及 tool_trace 里的 catalog → list_chunks → open_slice 决策链。开发者调试 grounding 用。
      </p>
      <form
        onSubmit={(e) => { e.preventDefault(); void runSearch(); }}
        style={{ display: "grid", gridTemplateColumns: "200px 200px 1fr auto", gap: 8, marginBottom: 12 }}
      >
        <input
          type="text"
          placeholder="accountId（默认 default）"
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="contactId（可选）"
          value={contactId}
          onChange={(e) => setContactId(e.target.value)}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="query：例如「这个产品对接 SaaS 还是私有化」"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="wikiInput"
        />
        <button type="submit" className="wikiBtn primary" disabled={pending || !query.trim()}>
          {pending ? "试召中…" : "试召"}
        </button>
      </form>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {route ? (
        <>
          <dl className="wikiArchiveMeta">
            <dt>风险等级</dt><dd>{route.riskLevel || "—"}</dd>
            <dt>需证据</dt><dd>{route.requiresEvidence ? "是" : "否"}</dd>
            <dt>覆盖度</dt><dd>{route.knowledgeCoverage || "—"}</dd>
            {route.reason ? (<><dt>路由原因</dt><dd>{route.reason}</dd></>) : null}
            {(route.neededCategories ?? []).length > 0 ? (
              <>
                <dt>需要类别</dt>
                <dd>{(route.neededCategories ?? []).map((c, i) => <span key={i} className="wikiArchiveTag">{c}</span>)}</dd>
              </>
            ) : null}
            {(route.missingKnowledge ?? []).length > 0 ? (
              <>
                <dt>缺失知识</dt>
                <dd>{(route.missingKnowledge ?? []).map((c, i) => <span key={i} className="wikiArchiveTag">{c}</span>)}</dd>
              </>
            ) : null}
          </dl>
          <hr className="wikiArchiveRule" />
          <div style={{ marginBottom: 6, fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--muted)" }}>
            命中 chunks · {(route.selectedChunkIds ?? []).length} 条
            {openingSlices ? "（正在拉详情…）" : ""}
          </div>
          <div style={{ display: "grid", gap: 8, marginBottom: 12 }}>
            {slices.length > 0
              ? slices.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  className="wikiSignalChunkBtn"
                  onClick={() => focusChunk(s.id)}
                  style={{ textAlign: "left", display: "grid", gap: 4 }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <span className="wikiArchiveTag">{s.wikiType ?? "—"}</span>
                    <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)" }}>
                      {s.status ?? "—"} / {s.integrityStatus ?? "—"}
                    </span>
                    <span style={{ fontFamily: "var(--font-display)", fontWeight: 600, marginLeft: 4 }}>
                      {s.title ?? "（无标题）"}
                    </span>
                  </div>
                  {s.summary ? <div style={{ color: "var(--muted)", fontSize: 12 }}>{s.summary}</div> : null}
                </button>
              ))
              : (route.selectedChunkIds ?? []).length === 0
                ? <div className="wikiHint">本次试召未选中任何 chunk。</div>
                : null}
          </div>
          {(route.selectedSliceReasons ?? []).length > 0 ? (
            <details>
              <summary className="wikiInspectorSectionTitle">slice 选择原因（{(route.selectedSliceReasons ?? []).length}）</summary>
              <ul style={{ margin: "6px 0 0 18px", fontSize: 12.5 }}>
                {(route.selectedSliceReasons ?? []).map((r, i) => <li key={i}>{r}</li>)}
              </ul>
            </details>
          ) : null}
          {(route.toolTrace ?? []).length > 0 ? (
            <details style={{ marginTop: 8 }}>
              <summary className="wikiInspectorSectionTitle">tool_trace（{(route.toolTrace ?? []).length}）</summary>
              <pre style={{ fontFamily: "var(--font-mono)", fontSize: 11, background: "var(--surface-2, #f4efe5)", padding: 10, border: "1px solid var(--line)", maxHeight: 320, overflow: "auto" }}>
                {JSON.stringify(route.toolTrace, null, 2)}
              </pre>
            </details>
          ) : null}
          {(route.evidenceExcerpts ?? []).length > 0 ? (
            <details style={{ marginTop: 8 }}>
              <summary className="wikiInspectorSectionTitle">evidence_excerpts（{(route.evidenceExcerpts ?? []).length}）</summary>
              <ul style={{ margin: "6px 0 0 18px", fontSize: 12.5 }}>
                {(route.evidenceExcerpts ?? []).map((e, i) => <li key={i}><code>{e}</code></li>)}
              </ul>
            </details>
          ) : null}
        </>
      ) : null}
    </div>
  );
}


interface GapSignalItem {
  signalId: string;
  kind: string;
  title: string;
  description: string;
  severity: string;
  source: string;
  status: string;
  affectedChunkIds: string[];
  searchQueries: string[];
  resolutionNote?: string | null;
  createdAt?: string | null;
  resolvedAt?: string | null;
}

// 8 类 gap_signal kind —— 与 src/knowledge_wiki/gap_signals.rs:11-19 对齐。
const GAP_SIGNAL_KINDS: { v: string; label: string }[] = [
  { v: "orphan", label: "孤立 chunk" },
  { v: "broken_link", label: "断链" },
  { v: "no_outlinks", label: "缺出链" },
  { v: "low_confidence", label: "低分被命中" },
  { v: "stale", label: "时效已过" },
  { v: "contradiction", label: "同题异说" },
  { v: "missing_chunk", label: "依赖已归档" },
  { v: "suggestion", label: "建议补完" },
];

export function LintView() {
  const [items, setItems] = useState<GapSignalItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [sweeping, setSweeping] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [activeKind, setActiveKind] = useState<string>("");

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams({ status: "pending", limit: "300" });
      const r = await fetch(`/api/knowledge/gap-signals?${params.toString()}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { signals: GapSignalItem[] };
      setItems(data.signals ?? []);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function sweep() {
    setSweeping(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/gap-signals/sweep", { method: "POST" });
      if (!r.ok) throw await parseApiError(r);
      const data = await r.json();
      setInfo(
        `lint 新增 ${data?.structuralLint?.newSignals ?? 0}，` +
          `stage1 自动消解 ${data?.sweep?.stage1AutoResolved ?? 0}`,
      );
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweeping(false);
    }
  }

  async function resolve(signalId: string, action: "dismiss" | "apply") {
    setBusyId(signalId);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/knowledge/gap-signals/${encodeURIComponent(signalId)}/${action}`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
        },
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(action === "dismiss" ? "已忽略" : "已标记为已应用");
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of items) m.set(s.kind, (m.get(s.kind) ?? 0) + 1);
    return m;
  }, [items]);

  const visible = useMemo(() => {
    if (!activeKind) return items;
    return items.filter((s) => s.kind === activeKind);
  }, [items, activeKind]);

  return (
    <div className="wikiPanelBody wikiLintBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <button type="button" className="primary" onClick={() => void sweep()} disabled={sweeping}>
          {sweeping ? "扫描中…" : "立即扫描"}
        </button>
        <span className="wikiHint">
          仅展示 status=pending；扫描包含 structural lint + stage 1 规则消解。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      <div className="wikiLintLayout">
        <div className="wikiLintTree">
          <button
            type="button"
            className={`wikiLintTreeNode ${activeKind === "" ? "active" : ""}`}
            onClick={() => setActiveKind("")}
          >
            <span>全部</span>
            <span className="wikiLintCount">{items.length}</span>
          </button>
          {GAP_SIGNAL_KINDS.map((k) => {
            const c = counts.get(k.v) ?? 0;
            return (
              <button
                type="button"
                key={k.v}
                className={`wikiLintTreeNode ${activeKind === k.v ? "active" : ""} ${
                  c === 0 ? "empty" : ""
                }`}
                onClick={() => setActiveKind(k.v)}
              >
                <span>
                  <span className={`wikiKind ${k.v}`}>{k.v}</span> {k.label}
                </span>
                <span className="wikiLintCount">{c}</span>
              </button>
            );
          })}
        </div>
        <div className="wikiLintPanel">
          {!loading && visible.length === 0 ? (
            <div className="wikiEmpty">
              {activeKind ? `当前 kind 没有 pending 信号。` : "没有 pending 信号。库结构当前看起来很健康。"}
            </div>
          ) : null}
          <div className="wikiSignalList">
            {visible.map((s) => (
              <div className={`wikiSignalCard sev-${s.severity}`} key={s.signalId}>
                <div className="wikiSignalHead">
                  <div className="wikiSignalTitle">
                    <span className={`wikiKind ${s.kind}`}>{s.kind}</span>
                    <span className={`wikiSev ${s.severity}`}>{s.severity}</span>
                    <strong>{s.title}</strong>
                  </div>
                  <div className="wikiSignalActions">
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => void resolve(s.signalId, "dismiss")}
                      disabled={busyId === s.signalId}
                    >
                      忽略
                    </button>
                    <button
                      type="button"
                      className="primary"
                      onClick={() => void resolve(s.signalId, "apply")}
                      disabled={busyId === s.signalId}
                    >
                      标记已处理
                    </button>
                  </div>
                </div>
                <p className="wikiSignalDesc">{s.description}</p>
                {s.affectedChunkIds.length > 0 ? (
                  <div className="wikiSignalRefs">
                    <span className="wikiSignalLabel">affected：</span>
                    {s.affectedChunkIds.slice(0, 8).map((id) => (
                      <button
                        type="button"
                        key={id}
                        className="wikiSignalChunkBtn"
                        onClick={() => focusChunk(id)}
                        title="在 Inspector 中打开"
                      >
                        <code>{id}</code>
                      </button>
                    ))}
                    {s.affectedChunkIds.length > 8 ? (
                      <span className="wikiHint">+{s.affectedChunkIds.length - 8}</span>
                    ) : null}
                  </div>
                ) : null}
                {s.searchQueries.length > 0 ? (
                  <div className="wikiSignalRefs">
                    <span className="wikiSignalLabel">queries：</span>
                    {s.searchQueries.slice(0, 5).map((q, i) => (
                      <code key={`${s.signalId}-q-${i}`}>{q}</code>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

// ReviewView：把 active chunks 客户端分类成 5 类待评审视图。
//
// 5 类（互斥优先级，从严到宽）：
//   1. contested            integrityStatus=rejected — 被否的需要重新审视
//   2. needs_review         integrityStatus=needs_review — 等待运营初审
//   3. source_orphan        缺 sourceQuote 或 sourceAnchors — 无法定位回源文档
//   4. pending_verification integrityStatus=needs_review 且 已有 sourceQuote — 距离 verify 一步之遥
//   5. dependents_pending   relatedChunks 引用的 chunk 不在当前活跃集合 — 关系链残缺
//
// 处置走现有路由：
//   - Verify  → POST /api/operation-knowledge/chunks/:id/verify
//   - Reject  → POST /api/operation-knowledge/chunks/:id/reject
//   - Patch   → 切到 编辑历史 tab，用户用现有 ChunkRevisionsDrawer 修
//
// AI 永不自动 verify：所有按钮都是显式管理员维护动作，前端不做"批量自动 verify"。
const REVIEW_CATEGORIES: { v: ReviewCategory; label: string; hint: string }[] = [
  { v: "contested", label: "被否决", hint: "integrity_status=rejected — 已被管理员或 AI 否决，等待重新评估" },
  { v: "needs_review", label: "待初审", hint: "integrity_status=needs_review — 等待管理员初审或补完证据" },
  { v: "source_orphan", label: "缺源", hint: "缺 source_quote 或 source_anchors — 无法定位回原文档" },
  { v: "pending_verification", label: "待 verify", hint: "已经有 source_quote，距 verify 一步之遥" },
  { v: "dependents_pending", label: "关系残缺", hint: "related_chunks 引用了不在活跃集合中的 chunk" }
];

export function ReviewView() {
  const [items, setItems] = useState<ReviewChunkItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [activeCategory, setActiveCategory] = useState<ReviewCategory>("needs_review");
  const [openBody, setOpenBody] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  // T9：在主区打开「审核+对话」双栏(ReviewChat)；非空时替代列表。
  const [chatChunk, setChatChunk] = useState<ReviewChatChunk | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/chunks");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: ReviewChunkItem[] };
      setItems(data.items ?? []);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  // 按优先级把每个 chunk 归入第一个命中的类别。
  // 注意：active chunks 列表也包含 status=rejected 的（status 与 integrity_status 是两条轴）；
  // contested 走 integrity_status，本视图不再二次过滤 status。
  const classified = useMemo(() => {
    const byId = new Set(items.map((i) => i.id));
    const out = new Map<ReviewCategory, ReviewChunkItem[]>();
    for (const cat of REVIEW_CATEGORIES) out.set(cat.v, []);
    for (const it of items) {
      const cat = classifyChunk(it, byId);
      if (cat) out.get(cat)!.push(it);
    }
    return out;
  }, [items]);

  const counts = useMemo(() => {
    const m = new Map<ReviewCategory, number>();
    for (const cat of REVIEW_CATEGORIES) m.set(cat.v, classified.get(cat.v)?.length ?? 0);
    return m;
  }, [classified]);

  const visible = classified.get(activeCategory) ?? [];

  function toggleBody(id: string) {
    setOpenBody((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function verify(id: string) {
    setBusyId(id);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/verify`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({}) }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已 verify：${id}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function reject(id: string) {
    setBusyId(id);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/reject`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({}) }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已 reject：${id}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function batchAction(action: "verify" | "archive") {
    if (selected.size === 0) return;
    if (action === "archive" && !window.confirm(`批量 archive ${selected.size} 条 chunk？`)) return;
    setBatchBusy(true);
    setError(null);
    setInfo(null);
    const ids = [...selected];
    const path =
      action === "verify"
        ? "/api/operation-knowledge/chunks/batch-verify"
        : "/api/operation-knowledge/chunks/batch-archive";
    const body =
      action === "verify"
        ? { ids, note: "batch verify (admin)" }
        : { ids, reason: "batch archive (admin)", actor: "admin" };
    try {
      const r = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as {
        verified?: string[];
        archived?: string[];
        skipped?: { id: string; reason: string }[];
      };
      const okCount = (data.verified?.length ?? data.archived?.length ?? 0);
      const skippedCount = data.skipped?.length ?? 0;
      setInfo(
        `批量${action === "verify" ? "verify" : "archive"} 完成：成功 ${okCount}，跳过 ${skippedCount}` +
          (skippedCount > 0 && data.skipped
            ? `（${data.skipped.slice(0, 3).map((s) => `${s.id}:${s.reason}`).join("； ")}${skippedCount > 3 ? "…" : ""}）`
            : ""),
      );
      setSelected(new Set());
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBatchBusy(false);
    }
  }

  return (
    <div className="wikiArchiveShell wikiReviewBody">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">explore / review queue</div>
          <h2>评审队列</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" className="ghost wikiBtn" onClick={() => void load()} disabled={loading}>
            <RefreshCw size={14} />
            {loading ? "加载中…" : "刷新"}
          </button>
        </div>
      </header>
      <div className="wikiToolbar">
        <span className="wikiHint">
          仅展示活跃 chunks。verify / reject 直接走现有路由，AI 永不自动 verify。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      {selected.size > 0 ? (
        <div className="wikiBatchToolbar">
          <span className="wikiArchiveTag">已选 {selected.size}</span>
          <button
            type="button"
            className="wikiBtn wikiActionBtn--verify"
            disabled={batchBusy}
            onClick={() => void batchAction("verify")}
          >
            <CheckCircle2 size={13} /> 批量 verify
          </button>
          <button
            type="button"
            className="wikiBtn"
            disabled={batchBusy}
            onClick={() => void batchAction("archive")}
          >
            <Archive size={13} /> 批量 archive
          </button>
          <button
            type="button"
            className="wikiBtn"
            disabled={batchBusy}
            onClick={() => setSelected(new Set())}
          >
            清空选中
          </button>
        </div>
      ) : null}
      <div className="wikiLintLayout">
        <div className="wikiReviewFilter">
          {REVIEW_CATEGORIES.map((cat) => {
            const c = counts.get(cat.v) ?? 0;
            return (
              <button
                type="button"
                key={cat.v}
                className={`wikiLintTreeNode ${activeCategory === cat.v ? "active" : ""} ${
                  c === 0 ? "empty" : ""
                }`}
                onClick={() => setActiveCategory(cat.v)}
                title={cat.hint}
              >
                <span>{cat.label}</span>
                <span className="wikiLintCount">{c}</span>
              </button>
            );
          })}
        </div>
        <div className="wikiLintPanel">
          {chatChunk ? (
            <>
              <button
                type="button"
                className="ghost wikiBtn"
                onClick={() => setChatChunk(null)}
              >
                <Undo2 size={14} />
                返回评审列表
              </button>
              <ReviewChat
                chunk={chatChunk}
                onResolved={() => {
                  setChatChunk(null);
                  void load();
                }}
              />
            </>
          ) : (
            <>
          {!loading && visible.length === 0 ? (
            <div className="wikiEmpty">
              当前类别没有待评审 chunk。
            </div>
          ) : null}
          <div className="wikiSignalList">
            {visible.map((c) => {
              const open = openBody.has(c.id);
              const hasQuote = !!c.sourceQuote && c.sourceQuote.trim().length > 0;
              const hasAnchor = (c.sourceAnchors?.length ?? 0) > 0;
              return (
                <div className="wikiReviewChunkCard" key={c.id}>
                  <div className="wikiSignalHead">
                    <input
                      type="checkbox"
                      className="wikiBatchCheckbox"
                      checked={selected.has(c.id)}
                      onChange={() => toggleSelect(c.id)}
                      title="选中以批量 verify / archive"
                    />
                    <div className="wikiSignalTitle">
                      <span className={`wikiKind ${c.wikiType ?? "unknown"}`}>{c.wikiType ?? "—"}</span>
                      <span className={`wikiSev ${c.integrityStatus === "rejected" ? "error" : "info"}`}>
                        {c.integrityStatus ?? "—"}
                      </span>
                      <button
                        type="button"
                        className="wikiReviewTitleBtn"
                        onClick={() => focusChunk(c.id)}
                        title="在 Inspector 中打开"
                      >
                        <strong>{c.title}</strong>
                      </button>
                    </div>
                    <div className="wikiSignalActions">
                      <button
                        type="button"
                        className="wikiReviewActionBtn verify"
                        onClick={() => void verify(c.id)}
                        disabled={busyId === c.id || !hasQuote || !hasAnchor}
                        title={!hasQuote || !hasAnchor ? "verify gate：需 sourceQuote + sourceAnchors 全有" : "标记为 verified"}
                      >
                        <CheckCircle2 size={14} />
                        Verify
                      </button>
                      <button
                        type="button"
                        className="wikiReviewActionBtn reject"
                        onClick={() => void reject(c.id)}
                        disabled={busyId === c.id}
                      >
                        <X size={14} />
                        Reject
                      </button>
                      <button
                        type="button"
                        className="wikiReviewActionBtn"
                        onClick={() => setChatChunk(c)}
                        title="打开审核+对话双栏：让 AI 改这条草稿，改完一键放行"
                      >
                        <MessageSquareText size={14} />
                        审核 / 对话
                      </button>
                    </div>
                  </div>
                  {c.summary ? <p className="wikiSignalDesc">{c.summary}</p> : null}
                  {hasQuote ? (
                    <blockquote className="wikiArchiveCitation">
                      {c.sourceQuote}
                      <span className="wikiArchiveCitationSource">{c.id}</span>
                    </blockquote>
                  ) : (
                    <div className="wikiHint">未配 source_quote — verify gate 将硬挡。</div>
                  )}
                  <div className="wikiReviewMeta">
                    <span>id：<code>{c.id}</code></span>
                    {hasAnchor ? (
                      <span>anchors：{c.sourceAnchors?.length ?? 0}</span>
                    ) : (
                      <span className="wikiBadge warn">无 anchors</span>
                    )}
                    {c.relatedChunks && c.relatedChunks.length > 0 ? (
                      <span>related：{c.relatedChunks.length}</span>
                    ) : null}
                    <button
                      type="button"
                      className="wikiCitedHead"
                      onClick={() => toggleBody(c.id)}
                    >
                      {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      {open ? "收起正文" : "展开正文"}
                    </button>
                  </div>
                  {open && c.body ? <pre className="wikiReviewBodyText">{c.body}</pre> : null}
                </div>
              );
            })}
          </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// KnowledgeTreeView：3 级树（wiki_type → business_topic → chunk title），右侧
// ChunkDetail 透出 source_quote 黄边块 + source_anchors 锚点 + related_chunks 跳转。
//
// 数据全部从 /api/operation-knowledge/chunks 取，纯客户端聚合：
//   l1: 9 类 wiki_type（source / entity / concept / comparison / synthesis /
//        methodology / finding / query / thesis；缺省落 "未分类"）
//   l2: chunk.business_topics[0]（缺省 "通用"）
//   l3: chunk.title（点击进入右侧 detail）
//
// 右侧 ChunkDetail 用同一个 chunk 数据渲染：
//   - title + wikiType + integrityStatus + status badge
//   - summary
//   - source_quote 黄边引用块（与 Review / Ask 视图风格一致）
//   - source_anchors 锚点列表：[startLine-endLine] [hash 短前缀]，hover 显示
//     完整 quoteHash + offsets；点击复制 anchor JSON 到剪贴板
//   - related_chunks 跳转 chip：点击切到目标 chunk
//   - body 默认收起；展开后用 <pre> 渲染原文（white-space pre-wrap）
//
// 全部只读：本视图不修改 chunk，verify / reject 走 ReviewView。
interface CatalogPersistedView {
  total?: number;
  items?: unknown[];
}

interface LogsAnalyzeView {
  windowHours?: number;
  totalCalls?: number;
  avgTurns?: number;
  truncationRate?: number;
  samples?: unknown[];
}

interface IngestSourceItem {
  sourceId: string;
  workspaceId: string;
  kind: string;
  url: string;
  label?: string | null;
  scheduleMinutes: number;
  lastFetchedAt?: string | null;
  lastEtag?: string | null;
  status: string;
  failureStreak?: number;
  lastError?: string | null;
  ingestCount?: number;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export function IngestSourcesView() {
  const [items, setItems] = useState<IngestSourceItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState({ kind: "rss", url: "", label: "", scheduleMinutes: 60 });

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/ingest-sources");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: IngestSourceItem[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void load(); }, []);

  async function handleCreate(ev: FormEvent) {
    ev.preventDefault();
    if (!draft.url.trim()) return;
    setCreating(true);
    try {
      const r = await fetch("/api/knowledge/ingest-sources", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          kind: draft.kind,
          url: draft.url.trim(),
          label: draft.label.trim() || null,
          scheduleMinutes: draft.scheduleMinutes,
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      setDraft({ kind: "rss", url: "", label: "", scheduleMinutes: 60 });
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  async function handleReactivate(id: string) {
    try {
      const r = await fetch(`/api/knowledge/ingest-sources/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ status: "active" }),
      });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleDelete(id: string) {
    if (!window.confirm("删除外部源？已 ingest 的 chunks 不会被回收。")) return;
    try {
      const r = await fetch(`/api/knowledge/ingest-sources/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Ingest Sources · 外部源自动 ingest</span>
        <h3 style={{ fontSize: 20 }}>外部源</h3>
        <p style={{ color: "var(--wiki-muted)", fontSize: 12, marginTop: 6 }}>
          周期性拉取 RSS / HTML 源，落库 chunk 默认 draft + needs_review（AI 永不自动 verify）。
          连续 3 次失败 → status=failing；7 天不可达 → status=disabled。
        </p>
      </header>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      <form
        onSubmit={handleCreate}
        style={{
          display: "grid",
          gridTemplateColumns: "120px 1fr 1fr 140px auto",
          gap: 8,
          marginBottom: 16,
        }}
      >
        <select
          value={draft.kind}
          onChange={(e) => setDraft({ ...draft, kind: e.target.value })}
          className="wikiInput"
        >
          <option value="rss">rss</option>
          <option value="html">html</option>
        </select>
        <input
          type="text"
          placeholder="URL（必填）"
          value={draft.url}
          onChange={(e) => setDraft({ ...draft, url: e.target.value })}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="标签（可选，便于识别）"
          value={draft.label}
          onChange={(e) => setDraft({ ...draft, label: e.target.value })}
          className="wikiInput"
        />
        <input
          type="number"
          min={1}
          placeholder="间隔（分钟）"
          value={draft.scheduleMinutes}
          onChange={(e) => setDraft({ ...draft, scheduleMinutes: Number(e.target.value) || 60 })}
          className="wikiInput"
        />
        <button type="submit" className="wikiBtn primary" disabled={creating || !draft.url.trim()}>
          {creating ? "创建中…" : "新增"}
        </button>
      </form>
      {loading ? (
        <div className="wikiHint">加载中…</div>
      ) : items.length === 0 ? (
        <div className="wikiHint">暂无外部源。新增后由 worker 周期拉取。</div>
      ) : (
        <table className="wikiTable" style={{ width: "100%", fontSize: 13 }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>kind</th>
              <th style={{ textAlign: "left" }}>URL / 标签</th>
              <th style={{ textAlign: "right" }}>间隔</th>
              <th style={{ textAlign: "left" }}>最后拉取</th>
              <th style={{ textAlign: "left" }}>状态</th>
              <th style={{ textAlign: "right" }}>失败次数</th>
              <th style={{ textAlign: "right" }}>已 ingest</th>
              <th style={{ textAlign: "left" }}>错误</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {items.map((it) => (
              <tr key={it.sourceId}>
                <td>{it.kind}</td>
                <td style={{ maxWidth: 360, wordBreak: "break-all" }}>
                  <div>{it.url}</div>
                  {it.label ? (
                    <div style={{ color: "var(--wiki-muted)", fontSize: 11 }}>{it.label}</div>
                  ) : null}
                </td>
                <td style={{ textAlign: "right" }}>{it.scheduleMinutes}m</td>
                <td style={{ fontSize: 11, color: "var(--wiki-muted)" }}>
                  {it.lastFetchedAt ? new Date(it.lastFetchedAt).toLocaleString() : "—"}
                </td>
                <td>
                  <span
                    className="wikiBadge"
                    style={{
                      background:
                        it.status === "active"
                          ? "var(--wiki-ok-bg, #d1fadf)"
                          : it.status === "failing"
                          ? "var(--wiki-warn-bg, #fef0c7)"
                          : "var(--wiki-error-bg, #fee4e2)",
                    }}
                  >
                    {it.status}
                  </span>
                </td>
                <td style={{ textAlign: "right" }}>{it.failureStreak ?? 0}</td>
                <td style={{ textAlign: "right" }}>{it.ingestCount ?? 0}</td>
                <td style={{ maxWidth: 220, fontSize: 11, color: "var(--wiki-error, #b42318)" }}>
                  {it.lastError ?? ""}
                </td>
                <td style={{ whiteSpace: "nowrap" }}>
                  {it.status !== "active" ? (
                    <button
                      type="button"
                      className="wikiBtn"
                      onClick={() => handleReactivate(it.sourceId)}
                    >
                      重新激活
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="wikiBtn danger"
                    style={{ marginLeft: 6 }}
                    onClick={() => handleDelete(it.sourceId)}
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

export function ObservabilityDashboard() {
  const [catalog, setCatalog] = useState<CatalogPersistedView | null>(null);
  const [catalogLive, setCatalogLive] = useState<{ total?: number } | null>(null);
  const [completeness, setCompleteness] = useState<CompletenessView | null>(null);
  const [integrity, setIntegrity] = useState<IntegrityReportView | null>(null);
  const [logs, setLogs] = useState<LogsAnalyzeView | null>(null);
  const [cacheStats, setCacheStats] = useState<{
    entries?: number;
    hits?: number;
    misses?: number;
    maxEntries?: number;
    ttlSeconds?: number;
  } | null>(null);
  const [phaseRollup, setPhaseRollup] = useState<{
    windowHours?: number;
    lifecycle?: Array<{ lifecycle: string; count: number; outOfClosedSet?: boolean }>;
    revisionReasons?: Array<{ reason: string; count: number }>;
    reviewerMisjudge?: Array<{ kind: string; count: number }>;
    negativeExamplePending?: number;
  } | null>(null);
  const [workerHealth, setWorkerHealth] = useState<{
    chatTasks?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      errorKindsTop?: Array<{ errorKind: string; count: number }>;
    };
    gapSignals?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      pendingKindsTop?: Array<{ kind: string; count: number }>;
      total?: number;
      pending?: number;
      resolved?: number;
      sweepHitRate?: number;
    };
    lessonsLearned?: {
      windowDays?: number;
      patternTop?: Array<{ pattern: string; count: number; outOfClosedSet?: boolean }>;
      blockedTotal?: number;
    };
  } | null>(null);
  const [pending, setPending] = useState(false);
  const [sweeping, setSweeping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const [a, b, c, d, e, f, g, h] = await Promise.all([
        fetch("/api/operation-knowledge/catalog/persisted").then((r) => r.json()),
        fetch("/api/operation-knowledge/catalog").then((r) => r.json()),
        fetch("/api/operation-knowledge/completeness").then((r) => r.json()),
        fetch("/api/operation-knowledge/integrity-report").then((r) => r.json()),
        fetch("/api/operation-knowledge/logs/analyze").then((r) => r.json()),
        fetch("/api/knowledge/metrics").then((r) => r.json()),
        fetch("/api/admin/observability/phase-rollup").then((r) => r.json()),
        fetch("/api/admin/observability/worker-health").then((r) => r.json())
      ]);
      setCatalog(a as CatalogPersistedView);
      setCatalogLive(b as { total?: number });
      setCompleteness(parseCompleteness(c));
      setIntegrity(parseIntegrityReport(d));
      setLogs(e as LogsAnalyzeView);
      const metrics = f as { answerCache?: typeof cacheStats };
      setCacheStats(metrics?.answerCache ?? null);
      setPhaseRollup(g as typeof phaseRollup);
      setWorkerHealth(h as typeof workerHealth);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function sweep() {
    setSweeping(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/gap-signals/sweep", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}"
      });
      if (!r.ok) throw await parseApiError(r);
      setInfo("已触发 gap-signals sweep");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweeping(false);
    }
  }

  const persistedTotal = catalog?.total ?? catalog?.items?.length ?? 0;
  const liveTotal = catalogLive?.total ?? 0;
  const drift = liveTotal - persistedTotal;

  return (
    <div className="wikiArchiveShell wikiObservability">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">steward / diagnostics</div>
          <h2>诊断仪表</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}
      {info ? <div className="wikiBannerInfo">{info}</div> : null}

      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[catalog]</span>
            <h4>目录覆盖</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>持久化</dt>
            <dd>{persistedTotal}</dd>
            <dt>实时</dt>
            <dd>{liveTotal}</dd>
            <dt>偏差</dt>
            <dd className={drift !== 0 ? "wikiObservabilityDrift" : undefined}>
              {drift > 0 ? `+${drift}` : drift}
            </dd>
          </dl>
          <button
            type="button"
            className="wikiObservabilityCta"
            onClick={() => void sweep()}
            disabled={sweeping}
          >
            <Workflow size={12} /> {sweeping ? "扫描中…" : "触发 sweep"}
          </button>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[completeness]</span>
            <h4>类型完整度</h4>
          </header>
          {completeness ? (
            <dl className="wikiArchiveMeta">
              <dt>应答模式</dt>
              <dd>{completeness.answeringMode}</dd>
              <dt>已验证</dt>
              <dd>
                {completeness.verifiedChunks}/{completeness.totalChunks}
              </dd>
              {completeness.summary ? (
                <>
                  <dt>摘要</dt>
                  <dd>{completeness.summary}</dd>
                </>
              ) : null}
              {/* coverage 5 维裁决渲染见后续 cockpit 任务 */}
            </dl>
          ) : (
            <div className="wikiEmpty">无完整度数据</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[integrity]</span>
            <h4>完整性诊断</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>needs_review</dt>
            <dd>{integrity?.needsReview ?? 0}</dd>
            <dt>verified</dt>
            <dd>{integrity?.verified ?? 0}</dd>
            <dt>rejected</dt>
            <dd>{integrity?.rejected ?? 0}</dd>
            <dt>total</dt>
            <dd>{integrity?.total ?? 0}</dd>
          </dl>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[logs]</span>
            <h4>检索 trace（24h）</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>窗口</dt>
            <dd>{logs?.windowHours ?? 24}h</dd>
            <dt>调用</dt>
            <dd>{logs?.totalCalls ?? 0}</dd>
            <dt>平均轮数</dt>
            <dd>{logs?.avgTurns?.toFixed?.(1) ?? "—"}</dd>
            <dt>截断率</dt>
            <dd>
              {typeof logs?.truncationRate === "number"
                ? `${(logs.truncationRate * 100).toFixed(1)}%`
                : "—"}
            </dd>
          </dl>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[answer-cache]</span>
            <h4>问答缓存</h4>
          </header>
          {(() => {
            const hits = cacheStats?.hits ?? 0;
            const misses = cacheStats?.misses ?? 0;
            const total = hits + misses;
            const ratio = total > 0 ? (hits / total) * 100 : null;
            return (
              <dl className="wikiArchiveMeta">
                <dt>条目</dt>
                <dd>{cacheStats?.entries ?? 0} / {cacheStats?.maxEntries ?? "—"}</dd>
                <dt>命中</dt>
                <dd>{hits}</dd>
                <dt>未命中</dt>
                <dd>{misses}</dd>
                <dt>命中率</dt>
                <dd className={ratio !== null && ratio >= 30 ? "wikiObservabilityDrift" : undefined}>
                  {ratio === null ? "—" : `${ratio.toFixed(1)}%`}
                </dd>
                <dt>TTL</dt>
                <dd>{cacheStats?.ttlSeconds ?? "—"}s</dd>
              </dl>
            );
          })()}
        </article>
      </div>

      <PhaseRollupPanel data={phaseRollup} />

      <WorkerHealthPanel data={workerHealth} />

      <TestMatchPanel />
    </div>
  );
}

function PhaseRollupPanel({
  data
}: {
  data: {
    windowHours?: number;
    lifecycle?: Array<{ lifecycle: string; count: number; outOfClosedSet?: boolean }>;
    revisionReasons?: Array<{ reason: string; count: number }>;
    reviewerMisjudge?: Array<{ kind: string; count: number }>;
    negativeExamplePending?: number;
  } | null;
}) {
  if (!data) {
    return null;
  }
  const lifecycle = data.lifecycle ?? [];
  const lifecycleTotal = lifecycle.reduce((sum, row) => sum + (row.count ?? 0), 0);
  const revisionReasons = data.revisionReasons ?? [];
  const reviewerMisjudge = data.reviewerMisjudge ?? [];
  const negativeExamplePending = data.negativeExamplePending ?? 0;
  const windowHours = data.windowHours ?? 24;

  return (
    <section className="wikiObservabilityPhaseRollup">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[phase-rollup]</span>
        <h4>Phase 0-D 自治信号（{windowHours}h）</h4>
      </header>
      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[lifecycle]</span>
            <h4>run lifecycle 终态</h4>
          </header>
          {lifecycleTotal === 0 ? (
            <div className="wikiEmpty">窗口内无 run</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {lifecycle.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.lifecycle}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[revision]</span>
            <h4>single-shot revision top</h4>
          </header>
          {revisionReasons.length === 0 ? (
            <div className="wikiEmpty">窗口内无 revision</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {revisionReasons.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.reason}</dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[reviewer]</span>
            <h4>reviewer 误判信号</h4>
          </header>
          {reviewerMisjudge.length === 0 ? (
            <div className="wikiEmpty">窗口内无误判信号</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {reviewerMisjudge.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.kind}</dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[negative-example]</span>
            <h4>负例候选 needs_review</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>待审核</dt>
            <dd
              className={
                negativeExamplePending > 0 ? "wikiObservabilityDrift" : undefined
              }
            >
              {negativeExamplePending}
            </dd>
          </dl>
        </article>
      </div>
    </section>
  );
}

// G-后续Ⅱ/2 · ObservabilityDashboard 第二波卡片：worker 健康聚合
//   - knowledge_chat_tasks 状态分布 + 失败 error_kind top
//   - knowledge_gap_signals sweep 命中率 + pending kind top
//   - lessons_learned 14d pattern × review_status 矩阵 + blocked_total
function WorkerHealthPanel({
  data
}: {
  data: {
    chatTasks?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      errorKindsTop?: Array<{ errorKind: string; count: number }>;
    };
    gapSignals?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      pendingKindsTop?: Array<{ kind: string; count: number }>;
      total?: number;
      pending?: number;
      resolved?: number;
      sweepHitRate?: number;
    };
    lessonsLearned?: {
      windowDays?: number;
      patternTop?: Array<{ pattern: string; count: number; outOfClosedSet?: boolean }>;
      blockedTotal?: number;
    };
  } | null;
}) {
  if (!data) {
    return null;
  }
  const chatStatuses = data.chatTasks?.byStatus ?? [];
  const chatTotal = chatStatuses.reduce((sum, row) => sum + (row.count ?? 0), 0);
  const chatErrors = data.chatTasks?.errorKindsTop ?? [];
  const gapStatuses = data.gapSignals?.byStatus ?? [];
  const gapKinds = data.gapSignals?.pendingKindsTop ?? [];
  const gapTotal = data.gapSignals?.total ?? 0;
  const gapPending = data.gapSignals?.pending ?? 0;
  const gapResolved = data.gapSignals?.resolved ?? 0;
  const gapHitRate = data.gapSignals?.sweepHitRate ?? 0;
  const lessonsPatterns = data.lessonsLearned?.patternTop ?? [];
  const lessonsBlocked = data.lessonsLearned?.blockedTotal ?? 0;
  const lessonsWindow = data.lessonsLearned?.windowDays ?? 14;

  return (
    <section className="wikiObservabilityPhaseRollup">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[worker-health]</span>
        <h4>worker 健康聚合</h4>
      </header>
      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[chat-tasks]</span>
            <h4>chat task 状态</h4>
          </header>
          {chatTotal === 0 ? (
            <div className="wikiEmpty">无任务</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {chatStatuses.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.status}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd
                    className={
                      row.status === "failed" && row.count > 0
                        ? "wikiObservabilityDrift"
                        : undefined
                    }
                  >
                    {row.count}
                  </dd>
                </Fragment>
              ))}
            </dl>
          )}
          {chatErrors.length > 0 ? (
            <div className="wikiArchiveCitation">
              <strong>error_kind top</strong>
              <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                {chatErrors.map((row, i) => (
                  <li key={i}>
                    <span className="wikiArchiveTag">[{row.errorKind}]</span> {row.count}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[gap-signals]</span>
            <h4>gap signals · sweep 命中率</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>总计</dt>
            <dd>{gapTotal}</dd>
            <dt>pending</dt>
            <dd className={gapPending > 0 ? "wikiObservabilityDrift" : undefined}>
              {gapPending}
            </dd>
            <dt>resolved</dt>
            <dd>{gapResolved}</dd>
            <dt>命中率</dt>
            <dd className={gapHitRate >= 0.5 ? "wikiObservabilityDrift" : undefined}>
              {gapTotal === 0 ? "—" : `${(gapHitRate * 100).toFixed(1)}%`}
            </dd>
          </dl>
          {gapKinds.length > 0 ? (
            <div className="wikiArchiveCitation">
              <strong>pending kind top</strong>
              <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                {gapKinds.map((row, i) => (
                  <li key={i}>
                    <span className="wikiArchiveTag">[{row.kind}]</span> {row.count}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          {gapStatuses.length > 0 ? (
            <details>
              <summary>状态明细</summary>
              <dl className="wikiArchiveMeta">
                {gapStatuses.map((row, i) => (
                  <Fragment key={i}>
                    <dt>
                      {row.status}
                      {row.outOfClosedSet ? (
                        <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                      ) : null}
                    </dt>
                    <dd>{row.count}</dd>
                  </Fragment>
                ))}
              </dl>
            </details>
          ) : null}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[lessons-learned]</span>
            <h4>lessons_learned ({lessonsWindow}d)</h4>
          </header>
          {lessonsPatterns.length === 0 ? (
            <div className="wikiEmpty">窗口内无产出</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {lessonsPatterns.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.pattern}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd
                    className={
                      row.pattern === "blocked_by_safety_guard" && row.count > 0
                        ? "wikiObservabilityDrift"
                        : undefined
                    }
                  >
                    {row.count}
                  </dd>
                </Fragment>
              ))}
              <dt>blocked_total</dt>
              <dd className={lessonsBlocked > 0 ? "wikiObservabilityDrift" : undefined}>
                {lessonsBlocked}
              </dd>
            </dl>
          )}
        </article>
      </div>
    </section>
  );
}

function TestMatchPanel() {
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    const q = query.trim();
    if (!q) {
      setError("请输入查询");
      return;
    }
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/test-match", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query: q })
      });
      if (!r.ok) throw await parseApiError(r);
      setResult(await r.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="wikiObservabilityCard wikiTestMatch">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[test-match]</span>
        <h4>检索调试</h4>
      </header>
      <div className="wikiTestMatchRow">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="输入查询，看哪些 chunk 命中 + grounding score"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void run();
            }
          }}
        />
        <button type="button" className="primary" onClick={() => void run()} disabled={pending}>
          <Search size={12} /> {pending ? "查询中…" : "试算"}
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      {result ? (
        <pre className="wikiTestMatchResult">{JSON.stringify(result, null, 2)}</pre>
      ) : null}
    </section>
  );
}


interface ChunkRevisionItem {
  revisionId: string;
  chunkId: string;
  op: string;
  patch: unknown;
  beforeHash: string;
  afterHash: string;
  source: string;
  reason?: string | null;
  createdAt?: string | null;
  createdBy?: string | null;
}

export function ChunkRevisionsDrawer() {
  const [chunkId, setChunkId] = useState("");
  const [pending, setPending] = useState(false);
  const [items, setItems] = useState<ChunkRevisionItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  async function load() {
    const id = chunkId.trim();
    if (!id) {
      setError("请输入 chunkId（24 位 ObjectId 十六进制）。");
      return;
    }
    setPending(true);
    setError(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/revisions?limit=100`,
      );
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: ChunkRevisionItem[] };
      setItems(data.items ?? []);
      setExpanded(new Set());
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setPending(false);
    }
  }

  function toggle(revisionId: string) {
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(revisionId)) next.delete(revisionId);
      else next.add(revisionId);
      return next;
    });
  }

  return (
    <div className="wikiPanelBody">
      <div className="wikiToolbar">
        <input
          type="text"
          className="wikiInput"
          placeholder="输入 chunkId 查看 revision 历史"
          value={chunkId}
          onChange={(e) => setChunkId(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void load();
          }}
        />
        <button type="button" className="primary" onClick={() => void load()} disabled={pending}>
          {pending ? "加载中…" : "拉取历史"}
        </button>
        <span className="wikiHint">timeline 倒序；每行展开查看 patch JSON 与 hash。</span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {!pending && chunkId && items.length === 0 && !error ? (
        <div className="wikiEmpty">该 chunk 暂无 revision 记录。</div>
      ) : null}
      <div className="wikiRevisionList">
        {items.map((r) => {
          const isOpen = expanded.has(r.revisionId);
          return (
            <div className={`wikiRevCard op-${r.op}`} key={r.revisionId}>
              <div className="wikiRevHead" onClick={() => toggle(r.revisionId)}>
                <span className={`wikiOp ${r.op}`}>{r.op}</span>
                <span className={`wikiSource ${r.source}`}>{r.source}</span>
                <span className="wikiRevTime">{r.createdAt ?? ""}</span>
                <span className="wikiRevId">{r.revisionId}</span>
                {r.reason ? <span className="wikiRevReason">{r.reason}</span> : null}
                {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              </div>
              {isOpen ? (
                <div className="wikiRevBody">
                  <div className="wikiRevHash">
                    <span>before: {r.beforeHash || "-"}</span>
                    <span>after: {r.afterHash || "-"}</span>
                    {r.createdBy ? <span>by: {r.createdBy}</span> : null}
                  </div>
                  <pre className="wikiRevPatch">{JSON.stringify(r.patch, null, 2)}</pre>
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
