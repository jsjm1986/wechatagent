import { useState } from "react";
import { api } from "../../../lib/api";

interface SuspectedDealReviewProps {
  signalId: string;
  contactId: string;
  evidence?: string;
  confidence?: number;
  occurrences?: number;
  onDone: () => void;
}

export function yuanToCents(input: string): number | null {
  const text = input.trim();
  if (!text) return null;
  const yuan = Number(text);
  if (!Number.isFinite(yuan) || yuan < 0) return null;
  const cents = Math.round(yuan * 100);
  return Number.isSafeInteger(cents) ? cents : null;
}

export function SuspectedDealReviewCard({
  signalId,
  contactId,
  evidence,
  confidence,
  occurrences,
  onDone,
}: SuspectedDealReviewProps) {
  const [amount, setAmount] = useState("");
  const [currency, setCurrency] = useState("CNY");
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const approve = async () => {
    const cents = yuanToCents(amount);
    if (amount.trim() && cents == null) {
      setError("金额必须是大于等于 0 的有效数字");
      return;
    }
    const normalizedCurrency = currency.trim().toUpperCase();
    if (normalizedCurrency && !/^[A-Z]{3}$/.test(normalizedCurrency)) {
      setError("币种必须是三位大写代码，例如 CNY");
      return;
    }
    const body: Record<string, unknown> = {};
    if (cents != null) body.amount = cents;
    if (normalizedCurrency) body.currency = normalizedCurrency;
    setBusy(true);
    setError(null);
    try {
      await api.post(`/api/admin/suspected-deals/${encodeURIComponent(signalId)}/approve`, body);
      onDone();
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  };

  const reject = async () => {
    const normalizedReason = reason.trim();
    if (!normalizedReason) {
      setError("驳回原因不能为空");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.post(`/api/admin/suspected-deals/${encodeURIComponent(signalId)}/reject`, {
        reason: normalizedReason,
      });
      onDone();
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="suspectedDealReviewCard">
      <div className="simpleActionEvidence">
        {evidence && <div>判断依据：{evidence}</div>}
        {confidence !== undefined && <div>置信度：{confidence}</div>}
        {occurrences !== undefined && <div>出现次数：{occurrences}</div>}
        <div>客户标识：{contactId}</div>
      </div>
      <div className="suspectedDealFields">
        <label>
          成交金额（元，可选）
          <input
            aria-label="成交金额（元，可选）"
            inputMode="decimal"
            value={amount}
            onChange={(event) => setAmount(event.target.value)}
            disabled={busy}
          />
        </label>
        <label>
          币种
          <input
            aria-label="币种"
            value={currency}
            maxLength={3}
            onChange={(event) => setCurrency(event.target.value)}
            disabled={busy}
          />
        </label>
      </div>
      {rejecting && (
        <label className="suspectedDealRejectReason">
          驳回原因
          <textarea
            aria-label="驳回原因"
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            disabled={busy}
          />
        </label>
      )}
      {error && <div className="suspectedDealReviewError">{error}</div>}
      <div className="simpleActionButtons">
        <button type="button" disabled={busy} onClick={() => void approve()}>
          确认成交
        </button>
        {rejecting ? (
          <button type="button" disabled={busy} onClick={() => void reject()}>
            提交驳回
          </button>
        ) : (
          <button type="button" disabled={busy} onClick={() => setRejecting(true)}>
            驳回线索
          </button>
        )}
      </div>
    </div>
  );
}
