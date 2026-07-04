// Ask-Human Phase 2 Task 9：单条 lessons-learned「晋升为 peer_case」处置卡（含 admin 填写表单）。
// 源自 system-strategy LessonsLearnedAdmin 的单条晋升内联表单。中立化到 components/review/
// 后，老页与统一收件箱频道都从这里 import。深链 lessonId 由 Task 1 的 richParams.lessonId 提供。
//
// 零跨 feature import：只依赖 react/lib/api/ui Toast。晋升需 admin 填 title/body（非一键，设计要求）。
// 晋升产出的 peer_case 是候选 chunk，仍需 admin 在知识审核队列二次 verify（已有后端行为，不改）。
import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";

// 字段子集（LessonLearnedEntry 的 camelCase，system-strategy/index.tsx:49 实证）。
// 注意 id 字段名是 lessonId（不是 id）。
interface LessonEntry {
  lessonId?: string;
  patternKind?: string;
  reviewStatus?: string;
  count?: number;
}

export function LessonPromoteCard({ lessonId, onDone }: { lessonId: string; onDone?: () => void }) {
  const toast = useToast();
  const [lesson, setLesson] = useState<LessonEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [summary, setSummary] = useState("");

  const load = useCallback(async () => {
    setError(null);
    try {
      // 无单项 GET：拉列表按 lessonId 过滤（与老页一致）。
      const { items } = await api.get<{ items: LessonEntry[] }>("/api/admin/lessons-learned");
      setLesson(items.find((l) => l.lessonId === lessonId) ?? null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [lessonId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function promote() {
    if (!title.trim() || !body.trim()) {
      toast.error("标题和案例正文都不能为空");
      return;
    }
    setBusy(true);
    try {
      const payload: Record<string, string> = { title: title.trim(), body: body.trim() };
      if (summary.trim()) payload.summary = summary.trim();
      await api.post(
        `/api/admin/lessons-learned/${encodeURIComponent(lessonId)}/promote-to-peer-case`,
        payload,
      );
      toast.success("已晋升为同行案例候选（仍需在知识审核队列核验）");
      onDone?.();
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (error) return <div className="lessonPromoteError">加载失败：{error}</div>;
  if (!lesson) return <div className="lessonPromoteLoading">加载中…</div>;

  // 已晋升的不再提供晋升表单（与老页 reviewStatus==="promoted" 隐藏按钮一致）。
  if (lesson.reviewStatus === "promoted") {
    return <div className="lessonPromoteDone">已晋升为同行案例</div>;
  }

  return (
    <div className="lessonPromoteCard">
      <div className="lessonPromoteKind">
        晋升为同行案例候选（仍需在知识审核队列核验）
      </div>
      <input
        className="lessonPromoteInput"
        type="text"
        placeholder="标题（≤ 200 字，必填）"
        value={title}
        maxLength={200}
        onChange={(e) => setTitle(e.target.value)}
      />
      <input
        className="lessonPromoteInput"
        type="text"
        placeholder="一句话摘要（可选）"
        value={summary}
        onChange={(e) => setSummary(e.target.value)}
      />
      <textarea
        className="lessonPromoteTextarea"
        placeholder="案例正文（≤ 4000 字，必填）"
        value={body}
        rows={6}
        maxLength={4000}
        onChange={(e) => setBody(e.target.value)}
      />
      <button
        type="button"
        disabled={busy || !title.trim() || !body.trim()}
        onClick={() => void promote()}
      >
        提交晋升
      </button>
    </div>
  );
}
