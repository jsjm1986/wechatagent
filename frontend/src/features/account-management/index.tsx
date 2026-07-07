import { useState, useEffect, useCallback } from "react";
import { RefreshCw, LogIn, Wifi, WifiOff, ArrowLeft } from "lucide-react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import type { Account } from "../../types";
import { AccountLogin } from "./AccountLogin";
import styles from "./AccountManagement.module.css";

export default function AccountManagementFeature() {
  const setAccounts = useAccountStore((s) => s.setAccounts);
  const accounts = useAccountStore((s) => s.accounts);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showLogin, setShowLogin] = useState(false);

  const loadAccounts = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.get<{ items: Account[] }>("/api/accounts");
      setAccounts(data.items);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载账号失败");
    } finally {
      setLoading(false);
    }
  }, [setAccounts]);

  const syncAccounts = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.post("/api/accounts/sync");
      await loadAccounts();
    } catch (e) {
      setError(e instanceof Error ? e.message : "同步失败");
    } finally {
      setSyncing(false);
    }
  }, [loadAccounts]);

  useEffect(() => {
    void loadAccounts();
  }, [loadAccounts]);

  const onlineCount = accounts.filter((a) => a.online).length;

  if (showLogin) {
    return (
      <div className={styles.page}>
        <button type="button" className={styles.backBtn} onClick={() => { setShowLogin(false); void loadAccounts(); }}>
          <ArrowLeft size={14} />
          返回账号列表
        </button>
        <AccountLogin onLoggedIn={() => { void loadAccounts(); }} />
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div className={styles.headerText}>
          <h1 className={styles.title}>账号管理</h1>
          <p className={styles.subtitle}>管理微信账号、配置 MCP 凭证、监控在线状态。</p>
        </div>
        <div className={styles.headerActions}>
          <button type="button" className={styles.ghostBtn} onClick={syncAccounts} disabled={syncing}>
            <RefreshCw size={14} className={syncing ? styles.spin : ""} />
            {syncing ? "同步中…" : "同步账号"}
          </button>
          <button type="button" className={styles.primaryBtn} onClick={() => setShowLogin(true)}>
            <LogIn size={14} />
            登录微信账号
          </button>
        </div>
      </div>

      {error && <div className={styles.err}>{error}</div>}

      <div className={styles.stats}>
        <div className={styles.statCard}>
          <span className={styles.statLabel}>在线账号</span>
          <strong className={styles.statValue}>{onlineCount}</strong>
        </div>
        <div className={styles.statCard}>
          <span className={styles.statLabel}>总账号数</span>
          <strong className={styles.statValue}>{accounts.length}</strong>
        </div>
        <div className={styles.statCard}>
          <span className={styles.statLabel}>离线账号</span>
          <strong className={styles.statValue}>{accounts.length - onlineCount}</strong>
        </div>
      </div>

      {loading ? (
        <div className={styles.loading}>加载中…</div>
      ) : accounts.length === 0 ? (
        <div className={styles.empty}>
          <strong>暂无账号</strong>
          <p>点击「登录微信账号」添加第一个账号，或点击「同步账号」从 MCP Server 拉取已登录的账号。</p>
          <div className={styles.emptyActions}>
            <button type="button" className={styles.primaryBtn} onClick={() => setShowLogin(true)}>
              <LogIn size={14} />
              登录微信账号
            </button>
            <button type="button" className={styles.ghostBtn} onClick={syncAccounts} disabled={syncing}>
              <RefreshCw size={14} className={syncing ? styles.spin : ""} />
              同步账号
            </button>
          </div>
        </div>
      ) : (
        <div className={styles.grid}>
          {accounts.map((account) => (
            <div key={account.id || account.accountId} className={styles.accountCard}>
              <div className={styles.accountHead}>
                <div className={styles.accountName}>
                  <strong>{account.alias || account.displayName || account.accountId}</strong>
                  {account.nickName && <span className={styles.accountNick}>{account.nickName}</span>}
                </div>
                {account.online ? (
                  <span className={styles.online}><Wifi size={13} />在线</span>
                ) : (
                  <span className={styles.offline}><WifiOff size={13} />离线</span>
                )}
              </div>
              <div className={styles.accountMeta}>
                {account.wxid && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>微信 ID</span>
                    <span className={styles.metaValue}>{account.wxid}</span>
                  </div>
                )}
                {account.appId && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>App ID</span>
                    <span className={styles.metaValue}>{account.appId}</span>
                  </div>
                )}
                {account.status && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>账号状态</span>
                    <span className={styles.metaValue}>{account.status}</span>
                  </div>
                )}
                <div className={styles.metaRow}>
                  <span className={styles.metaLabel}>MCP 配置</span>
                  <span className={styles.metaValue}>{account.mcpKeyConfigured ? "已配置" : "未配置"}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
