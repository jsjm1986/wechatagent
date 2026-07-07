import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Loader2 } from 'lucide-react';

interface LoginBeginResponse {
  qr_data_url?: string;
  login_page_url?: string;
  session_id: string;
}

interface LoginPollResponse {
  status: 'pending' | 'success' | 'expired' | 'canceled';
  wxid?: string;
  nick_name?: string;
}

export function AccountLogin() {
  const [accountAlias, setAccountAlias] = useState('');
  const [loginType, setLoginType] = useState<'mac' | 'ipad'>('mac');
  const [loginFlow, setLoginFlow] = useState<'auto' | 'manual'>('auto');
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [loginPageUrl, setLoginPageUrl] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<{ wxid: string; nickName: string } | null>(null);

  const handleBeginLogin = async () => {
    setError(null);
    setSuccess(null);
    setQrDataUrl(null);
    setLoginPageUrl(null);
    setSessionId(null);

    try {
      const res = await fetch('/api/accounts/login/begin', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          accountAlias: accountAlias.trim() || undefined,
          loginType,
          loginFlow,
        }),
      });

      if (!res.ok) {
        const err = await res.json();
        throw new Error(err.message || `HTTP ${res.status}`);
      }

      const data: LoginBeginResponse = await res.json();
      setQrDataUrl(data.qr_data_url || null);
      setLoginPageUrl(data.login_page_url || null);
      setSessionId(data.session_id);

      // 自动开始轮询
      if (data.session_id) {
        startPolling(data.session_id);
      }
    } catch (err: any) {
      setError(err.message || 'Failed to begin login');
    }
  };

  const startPolling = async (sid: string) => {
    setPolling(true);

    const poll = async () => {
      try {
        const params = new URLSearchParams({ session_id: sid });
        if (accountAlias.trim()) {
          params.append('account_alias', accountAlias.trim());
        }

        const res = await fetch(`/api/accounts/login/poll?${params}`);
        if (!res.ok) {
          const err = await res.json();
          throw new Error(err.message || `HTTP ${res.status}`);
        }

        const data: LoginPollResponse = await res.json();

        if (data.status === 'success') {
          setSuccess({ wxid: data.wxid!, nickName: data.nick_name! });
          setPolling(false);
          setQrDataUrl(null);
          // 登录成功后自动同步账号
          await syncAccounts();
        } else if (data.status === 'pending') {
          // 继续轮询
          setTimeout(() => poll(), 2500);
        } else {
          // expired / canceled
          setError(`Login ${data.status}`);
          setPolling(false);
        }
      } catch (err: any) {
        setError(err.message || 'Polling failed');
        setPolling(false);
      }
    };

    poll();
  };

  const syncAccounts = async () => {
    try {
      const res = await fetch('/api/accounts/sync', { method: 'POST' });
      if (!res.ok) throw new Error('Sync failed');
    } catch (err) {
      console.error('Auto sync failed:', err);
    }
  };

  const handleReset = () => {
    setQrDataUrl(null);
    setLoginPageUrl(null);
    setSessionId(null);
    setPolling(false);
    setError(null);
    setSuccess(null);
  };

  return (
    <Card className="w-full max-w-2xl mx-auto">
      <CardHeader>
        <CardTitle>微信账号登录</CardTitle>
        <CardDescription>
          通过扫码登录微信账号到 MCP Server。登录成功后账号会自动同步到系统。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!sessionId && (
          <>
            <div className="space-y-2">
              <Label htmlFor="accountAlias">
                账号别名 (Account Alias)
                <span className="text-muted-foreground text-xs ml-2">
                  Workspace Key 必填；Account Key 可留空
                </span>
              </Label>
              <Input
                id="accountAlias"
                placeholder="例如: kefu-a"
                value={accountAlias}
                onChange={(e) => setAccountAlias(e.target.value)}
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="loginType">登录平台</Label>
                <Select value={loginType} onValueChange={(v: any) => setLoginType(v)}>
                  <SelectTrigger id="loginType">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="mac">Mac</SelectItem>
                    <SelectItem value="ipad">iPad</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="loginFlow">登录流程</Label>
                <Select value={loginFlow} onValueChange={(v: any) => setLoginFlow(v)}>
                  <SelectTrigger id="loginFlow">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto (推荐)</SelectItem>
                    <SelectItem value="manual">Manual</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <Button onClick={handleBeginLogin} className="w-full">
              开始登录
            </Button>
          </>
        )}

        {sessionId && !success && (
          <div className="space-y-4">
            {loginPageUrl && (
              <Alert>
                <AlertDescription>
                  推荐使用 MCP Server 提供的登录页面（支持自动刷新二维码和二次验证）：
                  <a
                    href={loginPageUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-600 hover:underline ml-2"
                  >
                    打开登录页面
                  </a>
                </AlertDescription>
              </Alert>
            )}

            {qrDataUrl && (
              <div className="flex flex-col items-center space-y-2">
                <img src={qrDataUrl} alt="Login QR Code" className="w-64 h-64 border rounded" />
                <p className="text-sm text-muted-foreground">请使用微信扫描二维码登录</p>
              </div>
            )}

            {polling && (
              <div className="flex items-center justify-center space-x-2 text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>等待扫码确认...</span>
              </div>
            )}

            <Button variant="outline" onClick={handleReset} className="w-full">
              取消
            </Button>
          </div>
        )}

        {success && (
          <Alert className="bg-green-50 border-green-200">
            <AlertDescription className="space-y-2">
              <p className="font-semibold text-green-800">✓ 登录成功</p>
              <p className="text-sm">
                <strong>微信 ID:</strong> {success.wxid}
              </p>
              <p className="text-sm">
                <strong>昵称:</strong> {success.nickName}
              </p>
              <p className="text-xs text-muted-foreground mt-2">
                账号已自动同步到系统，可在账号列表查看。
              </p>
              <Button onClick={handleReset} variant="outline" size="sm" className="mt-2">
                登录另一个账号
              </Button>
            </AlertDescription>
          </Alert>
        )}

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
      </CardContent>
    </Card>
  );
}
