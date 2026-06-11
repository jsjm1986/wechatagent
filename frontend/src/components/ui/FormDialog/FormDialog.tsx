import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { Overlay } from "../Overlay";
import { ChunkPicker, type PickerChunk } from "../ChunkRef";
import styles from "./FormDialog.module.css";

export type FormField =
  | { kind: "text" | "textarea"; name: string; label: string; required?: boolean; placeholder?: string; defaultValue?: string; hint?: string }
  | { kind: "select"; name: string; label: string; options: { value: string; label: string }[]; defaultValue?: string; hint?: string }
  | { kind: "chunkRef"; name: string; label: string; required?: boolean; hint?: string };

export interface FormDialogOptions {
  title: string;
  fields: FormField[];
  submitText?: string;
  cancelText?: string;
  /// chunkRef 字段需要的列表加载器（注入避免 ui 层耦合接口）
  loadChunks?: () => Promise<PickerChunk[]>;
}

type Resolver = (values: Record<string, string> | null) => void;

const FormDialogContext = createContext<((opts: FormDialogOptions) => Promise<Record<string, string> | null>) | null>(null);

function initialValues(fields: FormField[]): Record<string, string> {
  const v: Record<string, string> = {};
  for (const f of fields) {
    v[f.name] = "defaultValue" in f && f.defaultValue ? f.defaultValue : "";
  }
  return v;
}

/// 全站表单弹窗，替代 window.prompt（split/merge/relate/patch/reject）。
/// 用法：const v = await form({ title, fields }); if (!v) return; // 取消
export function FormDialogProvider({ children }: { children: ReactNode }) {
  const [opts, setOpts] = useState<FormDialogOptions | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const resolverRef = useRef<Resolver | null>(null);

  const form = useCallback((o: FormDialogOptions) => {
    setOpts(o);
    setValues(initialValues(o.fields));
    return new Promise<Record<string, string> | null>((resolve) => {
      resolverRef.current = resolve;
    });
  }, []);

  const settle = useCallback((result: Record<string, string> | null) => {
    resolverRef.current?.(result);
    resolverRef.current = null;
    setOpts(null);
    setValues({});
  }, []);

  const missingRequired =
    !!opts &&
    opts.fields.some(
      (f) => "required" in f && f.required && !(values[f.name] ?? "").trim()
    );

  const setField = (name: string, val: string) => setValues((p) => ({ ...p, [name]: val }));

  return (
    <FormDialogContext.Provider value={form}>
      {children}
      <Overlay open={!!opts} onClose={() => settle(null)} labelledBy="formDialogTitle">
        {opts && (
          <form
            className={styles.box}
            onSubmit={(e) => {
              e.preventDefault();
              if (!missingRequired) settle(values);
            }}
          >
            <h3 id="formDialogTitle" className={styles.title}>
              {opts.title}
            </h3>
            <div className={styles.fields}>
              {opts.fields.map((f) => (
                <label key={f.name} className={styles.field}>
                  <span className={styles.label}>
                    {f.label}
                    {"required" in f && f.required ? <em className={styles.req}>*</em> : null}
                  </span>
                  {f.kind === "textarea" ? (
                    <textarea
                      className={styles.textarea}
                      value={values[f.name] ?? ""}
                      onChange={(e) => setField(f.name, e.target.value)}
                      placeholder={f.placeholder}
                      rows={4}
                    />
                  ) : f.kind === "select" ? (
                    <select
                      className={styles.select}
                      value={values[f.name] ?? ""}
                      onChange={(e) => setField(f.name, e.target.value)}
                    >
                      {f.options.map((o) => (
                        <option key={o.value} value={o.value}>
                          {o.label}
                        </option>
                      ))}
                    </select>
                  ) : f.kind === "chunkRef" ? (
                    <ChunkPicker
                      value={values[f.name] ?? ""}
                      onChange={(id) => setField(f.name, id)}
                      loadChunks={opts.loadChunks ?? (async () => [])}
                    />
                  ) : (
                    <input
                      type="text"
                      className={styles.input}
                      value={values[f.name] ?? ""}
                      onChange={(e) => setField(f.name, e.target.value)}
                      placeholder={f.placeholder}
                    />
                  )}
                  {f.hint && <span className={styles.hint}>{f.hint}</span>}
                </label>
              ))}
            </div>
            <div className={styles.actions}>
              <button type="button" className={styles.cancel} onClick={() => settle(null)}>
                {opts.cancelText ?? "取消"}
              </button>
              <button type="submit" className={styles.submit} disabled={missingRequired}>
                {opts.submitText ?? "确定"}
              </button>
            </div>
          </form>
        )}
      </Overlay>
    </FormDialogContext.Provider>
  );
}

export function useFormDialog(): (opts: FormDialogOptions) => Promise<Record<string, string> | null> {
  const ctx = useContext(FormDialogContext);
  if (!ctx) throw new Error("useFormDialog 必须在 <FormDialogProvider> 内使用");
  return ctx;
}
