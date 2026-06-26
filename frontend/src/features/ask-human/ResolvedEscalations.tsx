import { useEffect, useState } from "react";
import { api } from "../../lib/api";

// 已裁决（resolved）请示历史：只读回顾，不是待办，故与 pending 统一收件箱（inboxStore）正交，
// 自取数走专门端点 GET /api/admin/principal-escalations?status=resolved。
// wire 键核实：list handler 用 json!{} 手工拼 camelCase 外层键（principal_escalations.rs:42-55）；
// decision 是 PrincipalDecision，其 struct 无 rename_all（models.rs:3308 注释明示保 snake_case），
// 故内层键为 verdict / substance / constraints / authorization_window_hours（verdict/substance 单词无大小写差异）。

// 裁决口径闭集 → 中文标签（与 EscalationInline 的 VERDICT_OPTIONS 同源）。
const VERDICT_LABEL: Record<string, string> = {
  approved: "批准",
  rejected: "驳回",
  conditional: "有条件批准",
  deferred: "暂缓",
  delegated_back: "退回再议",
};

// 裁决渠道 → 中文标签。未知值原样回显（不吞）。
const RESOLVED_VIA_LABEL: Record<string, string> = {
  wechat: "决策人微信回复",
  admin: "后台直接裁决",
  principal_chat: "决策人对话",
  admin_direct: "后台直接裁决",
};

interface ResolvedDecision {
  verdict?: string;
  substance?: string;
  constraints?: string[];
  authorization_window_hours?: number | null;
}
interface ResolvedItem {
  shortCode: string;
  contactWxid?: string;
  category?: string;
  reason?: string;
  decision?: ResolvedDecision | null;
  authorizationExpiresAt?: string | null;
  resolvedVia?: string | null;
  createdAt?: string | null;
}

function formatExpiry(value: string | null | undefined): string {
  if (!value) return "长期有效";
  const t = new Date(value);
  if (Number.isNaN(t.getTime())) return value;
  // 本地化到分钟即可，秒级精度对回顾历史无意义。
  return t.toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ResolvedEscalations() {
  const [items, setItems] = useState<ResolvedItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .get<{ items?: ResolvedItem[] }>("/api/admin/principal-escalations?status=resolved")
      .then((res) => {
        if (cancelled) return;
        setItems(res.items ?? []);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : "加载失败");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (loading) {
    return <div className="resolvedEscEmpty">加载中…</div>;
  }
  if (error) {
    return <div className="askHumanFatal">加载失败：{error}</div>;
  }
  if (items.length === 0) {
    return <div className="resolvedEscEmpty">暂无已裁决记录</div>;
  }

  return (
    <div className="resolvedEscList">
      {items.map((it) => {
        const v = it.decision?.verdict;
        const verdictLabel = v ? (VERDICT_LABEL[v] ?? v) : "—";
        const viaLabel = it.resolvedVia
          ? (RESOLVED_VIA_LABEL[it.resolvedVia] ?? it.resolvedVia)
          : "—";
        return (
          <div className="resolvedEscRow" key={it.shortCode}>
            <div className="resolvedEscHead">
              <span className="resolvedEscCode">{it.shortCode}</span>
              <span className="resolvedEscVerdict">{verdictLabel}</span>
            </div>
            {it.decision?.substance && (
              <div className="resolvedEscSubstance">{it.decision.substance}</div>
            )}
            <dl className="resolvedEscMeta">
              {it.contactWxid && (
                <>
                  <dt>客户</dt>
                  <dd>{it.contactWxid}</dd>
                </>
              )}
              {it.decision?.constraints && it.decision.constraints.length > 0 && (
                <>
                  <dt>约束</dt>
                  <dd>{it.decision.constraints.join("；")}</dd>
                </>
              )}
              <dt>授权到期</dt>
              <dd>{formatExpiry(it.authorizationExpiresAt)}</dd>
              <dt>裁决渠道</dt>
              <dd>{viaLabel}</dd>
            </dl>
          </div>
        );
      })}
    </div>
  );
}
