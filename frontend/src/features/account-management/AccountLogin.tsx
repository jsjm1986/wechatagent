import { useState, useRef, useEffect } from "react";
import { QrCode, Loader2, CheckCircle2, ExternalLink } from "lucide-react";
import { api } from "../../lib/api";
import styles from "./AccountLogin.module.css";

interface LoginBeginResponse {
  login_session_id: string;
  qr_code_base64?: string;
  login_page_url?: string;
  status?: string;
}

interface LoginPollResponse {
  status: string;
  wxid?: string;
  nick_name?: string;
}

export function AccountLogin({ onLoggedIn }: { onLoggedIn?: () => void }) {
  const [accountAlias, setAccountAlias] = useState("");
  const [loginType, setLoginType] = useState<"mac" | "ipad">("mac");
  const [loginFlow, setLoginFlow] = useState<"auto" | "manual">("auto");
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [loginPageUrl, setLoginPageUrl] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<{ wxid?: string; nickName?: string } | null>(null);
  const pollTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (pollTimer.current) window.clearTimeout(pollTimer.current);
    };
  }, []);

  const beginLogin = async () => {
    setError(null);
    setSuccess(null);
    setQrDataUrl(null);
    setLoginPageUrl(null);
    setSessionId(null);
    setBusy(true);
    try {
      const data = await api.post<LoginBeginResponse>("/api/accounts/login/begin", {
        accountAlias: accountAlias.trim() || undefined,
        loginType,
        loginFlow,
      });
      setQrDataUrl(data.qr_code_base64 || null);
      setLoginPageUrl(data.login_page_url || null);
      setSessionId(data.login_session_id);
      if (data.login_session_id) startPolling(data.login_session_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : "发起登录失败");
    } finally {
      setBusy(false);
    }
  };

  const startPolling = (sid: string) => {
    setPolling(true);
    const poll = async () => {
      try {
        const params = new URLSearchParams({ loginSessionId: sid });
        if (accountAlias.trim()) params.append("accountAlias", accountAlias.trim());
        const data = await api.get<LoginPollResponse>(`/api/accounts/login/poll?${params}`);
        if (data.status === "success") {
          setSuccess({ wxid: data.wxid, nickName: data.nick_name });
          setPolling(false);
          setQrDataUrl(null);
          try {
            await api.post("/api/accounts/sync");
          } catch {
            /* 同步失败不阻断登录成功提示，用户可手动点同步 */
          }
          onLoggedIn?.();
        } else if (data.status === "pending") {
          pollTimer.current = window.setTimeout(poll, 2500);
        } else {
          setError(`登录${data.status === "expired" ? "已过期" : "未完成"}（${data.status}）`);
          setPolling(false);
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : "轮询失败");
        setPolling(false);
      }
    };
    poll();
  };

  const reset = () => {
    if (pollTimer.current) window.clearTimeout(pollTimer.current);
    setQrDataUrl(null);
    setLoginPageUrl(null);
    setSessionId(null);
    setPolling(false);
    setError(null);
    setSuccess(null);
  };

  return (
    <div className={styles.card}>
      <div className={styles.head}>
        <span className={styles.headIcon}><QrCode size={18} /></span>
        <div className={styles.headTxt}>
          <strong>微信账号登录</strong>
          <span>扫码登录微信账号到 MCP Server，成功后自动同步到系统。</span>
        </div>
      </div>

      {!sessionId && !success && (
        <div className={styles.form}>
          <label className={styles.field}>
            <span className={styles.label}>
              账号别名 account_alias
              <em className={styles.hint}>Workspace Key 必填（如 t-1）；Account Key 可留空</em>
            </span>
            <input
              className={styles.input}
              type="text"
              value={accountAlias}
              placeholder="例如 t-1"
              onChange={(e) => setAccountAlias(e.target.value)}
            />
          </label>

          <div className={styles.row}>
            <label className={styles.field}>
              <span className={styles.label}>登录平台</span>
              <select
                className={styles.input}
                value={loginType}
                onChange={(e) => setLoginType(e.target.value as "mac" | "ipad")}
              >
                <option value="mac">Mac</option>
                <option value="ipad">iPad</option>
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.label}>登录流程</span>
              <select
                className={styles.input}
                value={loginFlow}
                onChange={(e) => setLoginFlow(e.target.value as "auto" | "manual")}
              >
                <option value="auto">Auto（推荐）</option>
                <option value="manual">Manual</option>
              </select>
            </label>
          </div>

          <button type="button" className={styles.primaryBtn} onClick={beginLogin} disabled={busy}>
            {busy ? "发起中…" : "开始登录"}
          </button>
        </div>
      )}

      {sessionId && !success && (
        <div className={styles.qrArea}>
          {loginPageUrl && (
            <a className={styles.pageLink} href={loginPageUrl} target="_blank" rel="noopener noreferrer">
              <ExternalLink size={14} />
              打开 MCP 登录页面（支持二维码刷新与二次验证）
            </a>
          )}
          {qrDataUrl && (
            <div className={styles.qrBox}>
              <img src={qrDataUrl} alt="登录二维码" className={styles.qrImg} />
              <p className={styles.qrTip}>请使用微信扫描二维码登录</p>
            </div>
          )}
          {polling && (
            <div className={styles.polling}>
              <Loader2 size={16} className={styles.spin} />
              <span>等待扫码确认…</span>
            </div>
          )}
          <button type="button" className={styles.ghostBtn} onClick={reset}>取消</button>
        </div>
      )}

      {success && (
        <div className={styles.successBox}>
          <div className={styles.successHead}>
            <CheckCircle2 size={18} />
            <strong>登录成功</strong>
          </div>
          {success.wxid && (
            <div className={styles.metaRow}>
              <span className={styles.metaLabel}>微信 ID</span>
              <span className={styles.metaValue}>{success.wxid}</span>
            </div>
          )}
          {success.nickName && (
            <div className={styles.metaRow}>
              <span className={styles.metaLabel}>昵称</span>
              <span className={styles.metaValue}>{success.nickName}</span>
            </div>
          )}
          <p className={styles.successTip}>账号已自动同步，可在账号列表查看。</p>
          <button type="button" className={styles.ghostBtn} onClick={reset}>登录另一个账号</button>
        </div>
      )}

      {error && <div className={styles.err}>{error}</div>}
    </div>
  );
}
