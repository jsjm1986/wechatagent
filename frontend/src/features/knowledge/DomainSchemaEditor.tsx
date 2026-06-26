import { useState } from "react";

export interface DomainSchemaFieldDraft {
  name: string;
  label: string;
  kind: string; // string|enum|number|date|reference
  required: boolean;
  allowedValues?: string[];
  aliasOf?: string;
}
export interface DomainSchemaUpsertBody {
  schemaId: string;
  name: string;
  fields: DomainSchemaFieldDraft[];
  aliasDict: Record<string, string>;
  guardDsl?: string;
}
interface InitialSchema {
  schemaId: string;
  name: string;
  fields: DomainSchemaFieldDraft[];
  aliasDict?: Record<string, string>;
  guardDsl?: string | null;
}

const KIND_OPTIONS = ["string", "enum", "number", "date", "reference"];

export function DomainSchemaEditor({
  mode,
  initial,
  onSubmit,
  onCancel,
}: {
  mode: "create" | "edit";
  initial?: InitialSchema;
  onSubmit: (body: DomainSchemaUpsertBody) => void;
  onCancel: () => void;
}) {
  const [schemaId, setSchemaId] = useState(initial?.schemaId ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [fields, setFields] = useState<DomainSchemaFieldDraft[]>(initial?.fields ?? []);
  const [aliasText, setAliasText] = useState(
    Object.entries(initial?.aliasDict ?? {})
      .map(([k, v]) => `${k}=${v}`)
      .join("\n"),
  );
  const [guardDsl, setGuardDsl] = useState(initial?.guardDsl ?? "");

  function updateField(i: number, patch: Partial<DomainSchemaFieldDraft>) {
    setFields((arr) => arr.map((f, idx) => (idx === i ? { ...f, ...patch } : f)));
  }

  function submit() {
    if (!schemaId.trim() || !name.trim()) return; // 基本必填，越界交后端 400
    const aliasDict: Record<string, string> = {};
    for (const line of aliasText.split("\n")) {
      const [k, v] = line.split("=");
      if (k?.trim() && v?.trim()) aliasDict[k.trim()] = v.trim();
    }
    const body: DomainSchemaUpsertBody = {
      schemaId: schemaId.trim(),
      name: name.trim(),
      fields: fields.map((f) => ({
        name: f.name.trim(),
        label: f.label.trim(),
        kind: f.kind,
        required: f.required,
        ...(f.kind === "enum" && f.allowedValues?.length ? { allowedValues: f.allowedValues } : {}),
        ...(f.aliasOf?.trim() ? { aliasOf: f.aliasOf.trim() } : {}),
      })),
      aliasDict,
      ...(guardDsl.trim() ? { guardDsl: guardDsl.trim() } : {}),
    };
    onSubmit(body);
  }

  return (
    <div className="wikiSchemaEditor">
      <label className="wikiField">
        <span>schemaId（英文 id，唯一）</span>
        <input
          className="wikiInput"
          placeholder="schemaId 如 real_estate"
          value={schemaId}
          onChange={(e) => setSchemaId(e.target.value)}
          disabled={mode === "edit"}
        />
      </label>
      <label className="wikiField">
        <span>字段表名称</span>
        <input
          className="wikiInput"
          placeholder="字段表名称（中文，如 房产销售）"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>

      <div className="wikiSchemaEditorFields">
        {fields.map((f, i) => (
          <div className="wikiSchemaEditorRow" key={i}>
            <input
              className="wikiInput"
              placeholder="字段名 name（英文）"
              value={f.name}
              onChange={(e) => updateField(i, { name: e.target.value })}
            />
            <input
              className="wikiInput"
              placeholder="中文标签 label"
              value={f.label}
              onChange={(e) => updateField(i, { label: e.target.value })}
            />
            <select className="wikiInput" value={f.kind} onChange={(e) => updateField(i, { kind: e.target.value })}>
              {KIND_OPTIONS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
            <label className="wikiInlineCheckbox">
              <input
                type="checkbox"
                checked={f.required}
                onChange={(e) => updateField(i, { required: e.target.checked })}
              />
              必填
            </label>
            {f.kind === "enum" ? (
              <input
                className="wikiInput"
                placeholder="可选值（逗号分隔）"
                value={(f.allowedValues ?? []).join(", ")}
                onChange={(e) =>
                  updateField(i, {
                    allowedValues: e.target.value
                      .split(/[,，]/)
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
              />
            ) : null}
            <input
              className="wikiInput"
              placeholder="aliasOf（可选，指向另一字段名）"
              value={f.aliasOf ?? ""}
              onChange={(e) => updateField(i, { aliasOf: e.target.value })}
            />
            <button
              type="button"
              className="ghost"
              onClick={() => setFields((arr) => arr.filter((_, idx) => idx !== i))}
            >
              删除
            </button>
          </div>
        ))}
        <button
          type="button"
          className="ghost"
          onClick={() => setFields((arr) => [...arr, { name: "", label: "", kind: "string", required: false }])}
        >
          + 添加字段
        </button>
      </div>

      <label className="wikiField">
        <span>同义词识别（每行一条：别名=字段名）</span>
        <textarea
          className="wikiInput"
          rows={3}
          placeholder="例如：预算=budget"
          value={aliasText}
          onChange={(e) => setAliasText(e.target.value)}
        />
      </label>
      <label className="wikiField">
        <span>guardDsl（可选）</span>
        <textarea className="wikiInput" rows={2} value={guardDsl} onChange={(e) => setGuardDsl(e.target.value)} />
      </label>

      <div className="wikiSchemaEditorActions">
        <button type="button" className="ghost" onClick={onCancel}>
          取消
        </button>
        <button type="button" className="primary" onClick={submit} disabled={!schemaId.trim() || !name.trim()}>
          保存
        </button>
      </div>
    </div>
  );
}
