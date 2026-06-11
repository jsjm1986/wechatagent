import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { Overlay } from "../Overlay";
import styles from "./ConfirmDialog.module.css";

export interface ConfirmOptions {
  title: string;
  body?: ReactNode;
  confirmText?: string;
  cancelText?: string;
  tone?: "default" | "danger";
  /// 危险操作守护：填写后，用户必须在输入框键入与之完全一致的文本才能解锁确认按钮。
  /// 用于全量发版 / 切换 active 状态机等不可逆操作。
  requireText?: string;
}

type Resolver = (ok: boolean) => void;

const ConfirmContext = createContext<((opts: ConfirmOptions) => Promise<boolean>) | null>(null);

/// 全站确认弹窗。挂在频道根，子树用 useConfirm() 拿到 confirm 函数。
/// 用法：const ok = await confirm({ title, tone:"danger" }); if (!ok) return;
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [opts, setOpts] = useState<ConfirmOptions | null>(null);
  const [typed, setTyped] = useState("");
  const resolverRef = useRef<Resolver | null>(null);

  const confirm = useCallback((o: ConfirmOptions) => {
    setOpts(o);
    setTyped("");
    return new Promise<boolean>((resolve) => {
      resolverRef.current = resolve;
    });
  }, []);

  const settle = useCallback((ok: boolean) => {
    resolverRef.current?.(ok);
    resolverRef.current = null;
    setOpts(null);
    setTyped("");
  }, []);

  const locked = !!opts?.requireText && typed.trim() !== opts.requireText.trim();

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      <Overlay
        open={!!opts}
        onClose={() => settle(false)}
        labelledBy="confirmDialogTitle"
        closeOnScrim={opts?.tone !== "danger"}
      >
        {opts && (
          <div className={styles.box}>
            <h3 id="confirmDialogTitle" className={styles.title}>
              {opts.title}
            </h3>
            {opts.body && <div className={styles.body}>{opts.body}</div>}
            {opts.requireText && (
              <label className={styles.requireField}>
                <span>
                  请输入 <b>{opts.requireText}</b> 以确认
                </span>
                <input
                  type="text"
                  value={typed}
                  onChange={(e) => setTyped(e.target.value)}
                  autoFocus
                  placeholder={opts.requireText}
                />
              </label>
            )}
            <div className={styles.actions}>
              <button type="button" className={styles.cancel} onClick={() => settle(false)}>
                {opts.cancelText ?? "取消"}
              </button>
              <button
                type="button"
                className={opts.tone === "danger" ? styles.confirmDanger : styles.confirm}
                onClick={() => settle(true)}
                disabled={locked}
              >
                {opts.confirmText ?? "确认"}
              </button>
            </div>
          </div>
        )}
      </Overlay>
    </ConfirmContext.Provider>
  );
}

export function useConfirm(): (opts: ConfirmOptions) => Promise<boolean> {
  const ctx = useContext(ConfirmContext);
  if (!ctx) throw new Error("useConfirm 必须在 <ConfirmProvider> 内使用");
  return ctx;
}
