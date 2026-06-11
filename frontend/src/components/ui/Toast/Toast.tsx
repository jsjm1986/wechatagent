import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { CheckCircle2, AlertTriangle, Info, X } from "lucide-react";
import styles from "./Toast.module.css";

type ToastTone = "success" | "error" | "info";
interface ToastItem {
  id: number;
  tone: ToastTone;
  message: string;
}

interface ToastApi {
  success: (msg: string) => void;
  error: (msg: string) => void;
  info: (msg: string) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

const ICONS = { success: CheckCircle2, error: AlertTriangle, info: Info };
const DURATION = { success: 3000, error: 6000, info: 4000 };

/// 全站瞬时反馈通道。挂在频道根，子树用 useToast() 推送。
/// 持久错误（需重试的）不走这里，仍用 LlmErrorBanner。
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const seq = useRef(0);

  const remove = useCallback((id: number) => {
    setItems((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const push = useCallback(
    (tone: ToastTone, message: string) => {
      const id = ++seq.current;
      setItems((prev) => [...prev, { id, tone, message }]);
      setTimeout(() => remove(id), DURATION[tone]);
    },
    [remove]
  );

  const api: ToastApi = {
    success: useCallback((m: string) => push("success", m), [push]),
    error: useCallback((m: string) => push("error", m), [push]),
    info: useCallback((m: string) => push("info", m), [push]),
  };

  return (
    <ToastContext.Provider value={api}>
      {children}
      {createPortal(
        <div className={styles.stack} role="status" aria-live="polite">
          {items.map((t) => {
            const Icon = ICONS[t.tone];
            return (
              <div key={t.id} className={`${styles.toast} ${styles[t.tone]}`}>
                <Icon size={16} className={styles.icon} />
                <span className={styles.msg}>{t.message}</span>
                <button type="button" className={styles.close} onClick={() => remove(t.id)} aria-label="关闭">
                  <X size={13} />
                </button>
              </div>
            );
          })}
        </div>,
        document.body
      )}
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast 必须在 <ToastProvider> 内使用");
  return ctx;
}
