import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { useAuthStore } from "./stores/authStore";
import "./components/ui/tokens.css";
import "./components/ui/reset.css";
import "./styles.css";

// P0-F：全局 fetch 401 拦截器。后端 session middleware 拒登录后会返 401；
// 前端这里 monkey-patch 一次 fetch，所有 /api 调用 401 都走 forceLogout，
// 把页面拉回 LoginScreen。开关用 sessionStorage（重启 tab 也能复现）。
const SESSION_KEY = "wa.authed";
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const res = await originalFetch(input, init);
  if (res.status === 401) {
    const url = typeof input === "string" ? input : (input as Request).url;
    if (url.startsWith("/api/") && !url.startsWith("/api/auth/login")) {
      sessionStorage.removeItem(SESSION_KEY);
      window.dispatchEvent(new CustomEvent("wa-auth-expired"));
    }
  }
  return res;
};

interface MeResponse {
  username: string;
  userId: string;
  workspaces?: string[];
  currentWorkspace?: string;
}


function LoginScreen({ onLoggedIn }: { onLoggedIn: (me: MeResponse) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setErr("");
    setBusy(true);
    try {
      const res = await originalFetch("/api/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password }),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        const code = (body as { error?: string }).error || "login_failed";
        setErr(code === "invalid_credentials" ? "用户名或密码错误" : `登录失败：${code}`);
        return;
      }
      const meRes = await originalFetch("/api/auth/me");
      if (!meRes.ok) {
        setErr("登录后状态校验失败，请重试");
        return;
      }
      const me = (await meRes.json()) as MeResponse;
      sessionStorage.setItem(SESSION_KEY, "1");
      onLoggedIn(me);
    } catch (e) {
      setErr(`网络错误：${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="authLoginShell">
      <div className="authLoginCard">
        <header className="authLoginHeader">
          <div className="authLoginEyebrow">archive · administration</div>
          <h1 className="authLoginTitle">WechatAgent</h1>
          <div className="authLoginSubtitle">运营档案馆 · 管理员入口</div>
        </header>
        <form className="authLoginForm" onSubmit={handleSubmit}>
          <label className="authLoginField">
            <span className="authLoginLabel">用户名</span>
            <input
              type="text"
              autoComplete="username"
              autoFocus
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              required
              disabled={busy}
            />
          </label>
          <label className="authLoginField">
            <span className="authLoginLabel">密码</span>
            <input
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              disabled={busy}
            />
          </label>
          {err && <div className="authLoginError">{err}</div>}
          <button type="submit" className="authLoginSubmit" disabled={busy}>
            {busy ? "正在登录" : "登录"}
          </button>
        </form>
        <footer className="authLoginFooter">
          管理员账号由运维通过 <code>BOOTSTRAP_ADMIN_USERNAME</code> /{" "}
          <code>BOOTSTRAP_ADMIN_PASSWORD</code> 环境变量初始化
        </footer>
      </div>
    </div>
  );
}

function AuthGate() {
  const [me, setMe] = useState<MeResponse | null>(null);
  const [bootstrapping, setBootstrapping] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const r = await originalFetch("/api/auth/me");
        if (cancelled) return;
        if (r.ok) {
          const body = (await r.json()) as MeResponse;
          sessionStorage.setItem(SESSION_KEY, "1");
          setMe(body);
        } else {
          sessionStorage.removeItem(SESSION_KEY);
          setMe(null);
        }
      } finally {
        if (!cancelled) setBootstrapping(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    function onExpired() {
      setMe(null);
    }
    window.addEventListener("wa-auth-expired", onExpired);
    return () => window.removeEventListener("wa-auth-expired", onExpired);
  }, []);

  async function logout() {
    try {
      await originalFetch("/api/auth/logout", { method: "POST" });
    } catch {
      // 网络错失败也清本地状态。
    }
    sessionStorage.removeItem(SESSION_KEY);
    setMe(null);
  }

  if (bootstrapping) {
    return <div className="authLoginShell"><div className="authLoginBootstrap">正在校验登录状态…</div></div>;
  }
  if (!me) {
    return <LoginScreen onLoggedIn={setMe} />;
  }
  return <AuthedApp me={me} onLogout={logout} />;
}

function AuthedApp({ me, onLogout }: { me: MeResponse; onLogout: () => void }) {
  useEffect(() => {
    useAuthStore.getState().setUser(me);
    useAuthStore.getState().setHandlers({
      onLogout,
      onSwitchWorkspace: async (workspaceId: string) => {
        const current = me.currentWorkspace ?? (me.workspaces ?? [])[0] ?? "default";
        if (workspaceId === current) return;
        try {
          const r = await originalFetch("/api/auth/workspace", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ workspaceId }),
          });
          if (r.ok) window.location.reload();
        } catch {
          /* 切换失败保持原状 */
        }
      },
    });
  }, [me, onLogout]);

  return <App />;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AuthGate />
  </React.StrictMode>
);
