import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { ESCALATION_VERDICT_LABELS, ESCALATION_RESOLVED_VIA_LABELS, labelOf } from "../../lib/reviewLabels";

// 已裁决（resolved）请示历史：只读回顾，不是待办，故与 pending 统一收件箱（inboxStore）正交，
// 自取数走专门端点 GET /api/admin/principal-escalations?status=resolved。
// wire 键核实：list handler 用 json!{} 手工拼 camelCase 外层键（principal_escalations.rs:42-55）；
// decision 是 PrincipalDecision，其 struct 无 rename_all（models.rs:3308 注释明示保 snake_case），
// 故内层键为 verdict / substance / constraints / authorization_window_hours（verdict/substance 单词无大小写差异）。

interface ResolvedDecision {
  verdict?: string;
  substance?: string;
  constraints?: string[];
  authorization_window_hours?: number | null;
  exemption_type?: string | null;
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

// 后端契约是 RFC3339 字符串（principal_escalations::escalation_list_item_json 经
// dt_to_string 统一）；这里防御性兼容毫秒数与历史 bson 扩展 JSON 对象
// {$date:{$numberLong:"…"}} / {$date:"…"}（旧部署/缓存残留曾把对象原样交给
// React 渲染导致整页崩溃）。任何形态都绝不把非字符串值直接返回。
function expiryToDate(value: unknown): Date | null {
  if (typeof value === "number" && Number.isFinite(value)) return new Date(value);
  if (typeof value === "string") {
    const t = new Date(value);
    return Number.isNaN(t.getTime()) ? null : t;
  }
  if (typeof value === "object" && value !== null && "$date" in value) {
    const inner = (value as { $date: unknown }).$date;
    if (typeof inner === "string" || typeof inner === "number") return expiryToDate(inner);
    if (typeof inner === "object" && inner !== null && "$numberLong" in inner) {
      const ms = Number((inner as { $numberLong: unknown }).$numberLong);
      return Number.isFinite(ms) ? new Date(ms) : null;
    }
  }
  return null;
}

export function formatExpiry(value: unknown): string {
  if (!value) return "本次转述不设期限";
  const t = expiryToDate(value);
  if (!t) return typeof value === "string" ? value : "时间格式无法识别";
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
        const verdictLabel = labelOf(ESCALATION_VERDICT_LABELS, v);
        const viaLabel = labelOf(ESCALATION_RESOLVED_VIA_LABELS, it.resolvedVia);
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
              <dt>本次转述到期</dt>
              <dd>{formatExpiry(it.authorizationExpiresAt)}</dd>
              {it.decision?.exemption_type && it.decision.exemption_type !== "none" && (
                <>
                  <dt>长期豁免</dt>
                  <dd>{it.decision.exemption_type === "knowledge" ? "该客户 + 通用知识" : "仅该客户"}</dd>
                </>
              )}
              <dt>裁决渠道</dt>
              <dd>{viaLabel}</dd>
            </dl>
          </div>
        );
      })}
    </div>
  );
}
