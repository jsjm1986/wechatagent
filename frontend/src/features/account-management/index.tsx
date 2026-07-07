import { useState, useEffect } from 'react';
import { RefreshCw, LogIn, Wifi, WifiOff, Clock } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { AccountLogin } from './AccountLogin';
import { api } from '@/lib/api';
import type { Account } from '@/types';
import styles from './AccountManagement.module.css';

export default function AccountManagementFeature() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showLogin, setShowLogin] = useState(false);

  const loadAccounts = async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.get<{ items: Account[] }>('/api/accounts');
      setAccounts(data.items);
    } catch (err: any) {
      setError(err.message || 'Failed to load accounts');
    } finally {
      setLoading(false);
    }
  };

  const syncAccounts = async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.post('/api/accounts/sync');
      await loadAccounts();
    } catch (err: any) {
      setError(err.message || 'Sync failed');
    } finally {
      setSyncing(false);
    }
  };

  useEffect(() => {
    loadAccounts();
  }, []);

  const onlineCount = accounts.filter((a) => a.online).length;

  if (showLogin) {
    return (
      <div className={styles.page}>
        <div className={styles.backBar}>
          <Button variant="outline" onClick={() => setShowLogin(false)}>
            ← 返回账号列表
          </Button>
        </div>
        <AccountLogin />
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div>
          <h1 className={styles.title}>账号管理</h1>
          <p className={styles.subtitle}>
            管理微信账号、配置 MCP 凭证、监控在线状态
          </p>
        </div>
        <div className={styles.headerActions}>
          <Button
            variant="outline"
            onClick={syncAccounts}
            disabled={syncing}
          >
            <RefreshCw className={syncing ? 'animate-spin' : ''} size={16} />
            同步账号
          </Button>
          <Button onClick={() => setShowLogin(true)}>
            <LogIn size={16} />
            登录微信账号
          </Button>
        </div>
      </div>

      {error && (
        <Alert variant="destructive" className={styles.alert}>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <div className={styles.stats}>
        <Card>
          <CardHeader>
            <CardDescription>在线账号</CardDescription>
            <CardTitle className="text-3xl">{onlineCount}</CardTitle>
          </CardHeader>
        </Card>
        <Card>
          <CardHeader>
            <CardDescription>总账号数</CardDescription>
            <CardTitle className="text-3xl">{accounts.length}</CardTitle>
          </CardHeader>
        </Card>
        <Card>
          <CardHeader>
            <CardDescription>离线账号</CardDescription>
            <CardTitle className="text-3xl">{accounts.length - onlineCount}</CardTitle>
          </CardHeader>
        </Card>
      </div>

      {loading ? (
        <div className={styles.loading}>加载中...</div>
      ) : accounts.length === 0 ? (
        <Card className={styles.empty}>
          <CardHeader>
            <CardTitle>暂无账号</CardTitle>
            <CardDescription>
              点击"登录微信账号"添加第一个账号，或点击"同步账号"从 MCP Server 拉取已登录的账号。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className={styles.emptyActions}>
              <Button onClick={() => setShowLogin(true)}>
                <LogIn size={16} />
                登录微信账号
              </Button>
              <Button variant="outline" onClick={syncAccounts} disabled={syncing}>
                <RefreshCw className={syncing ? 'animate-spin' : ''} size={16} />
                同步账号
              </Button>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className={styles.accountGrid}>
          {accounts.map((account) => (
            <Card key={account.id || account.accountId} className={styles.accountCard}>
              <CardHeader>
                <div className={styles.accountHeader}>
                  <div className={styles.accountInfo}>
                    <CardTitle className={styles.accountName}>
                      {account.alias || account.displayName || account.accountId}
                    </CardTitle>
                    {account.nickName && (
                      <CardDescription>{account.nickName}</CardDescription>
                    )}
                  </div>
                  <div className={styles.statusBadge}>
                    {account.online ? (
                      <span className={styles.statusOnline}>
                        <Wifi size={14} />
                        在线
                      </span>
                    ) : (
                      <span className={styles.statusOffline}>
                        <WifiOff size={14} />
                        离线
                      </span>
                    )}
                  </div>
                </div>
              </CardHeader>
              <CardContent className={styles.accountMeta}>
                {account.wxid && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>微信 ID:</span>
                    <span className={styles.metaValue}>{account.wxid}</span>
                  </div>
                )}
                {account.appId && (
                  <div className={styles.metaRow}>
                    <span className={styles.metaLabel}>App ID:</span>
                    <span className={styles.metaValue}>{account.appId}</span>
                  </div>
                )}
                <div className={styles.metaRow}>
                  <span className={styles.metaLabel}>MCP 配置:</span>
                  <span className={styles.metaValue}>
                    {account.mcpKeyConfigured ? '✓ 已配置' : '✗ 未配置'}
                  </span>
                </div>
                {account.lastSyncAt && (
                  <div className={styles.metaRow}>
                    <Clock size={12} />
                    <span className={styles.metaValue}>
                      {new Date(account.lastSyncAt).toLocaleString('zh-CN')}
                    </span>
                  </div>
                )}
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
