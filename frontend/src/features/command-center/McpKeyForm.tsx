import { useState } from "react";
import { KeyRound } from "lucide-react";
import { api } from "../../lib/api";
import styles from "./McpKeyForm.module.css";

/// 账号 MCP 密钥配置表单。密钥是敏感值：输入框用 password 型、不回显已存值，
/// 仅以「已配置」布尔提示状态；提交后立即清空输入，不在前端残留明文。
/// body 键为 snake_case（后端 UpdateAccountMcpKeyRequest：mcp_api_key / mcp_base_url）。
export function McpKeyForm({ accountId, configured }: { accountId: string; configured: boolean }) {
  const [key, setKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    if (!key.trim()) {
      setError("请先填写 MCP 密钥");
      return;
    }
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      await api.put(`/api/accounts/${accountId}/mcp-key`, {
        mcp_api_key: key,
        ...(baseUrl.trim() ? { mcp_base_url: baseUrl.trim() } : {}),
      });
      setKey("");
      setBaseUrl("");
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "保存失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className={styles.form}>
      <div className={styles.head}>
        <span className={styles.headIcon}><KeyRound size={16} /></span>
        <div className={styles.headTxt}>
          <strong>MCP 工具密钥</strong>
          <span className={configured ? styles.statusOk : styles.statusWarn}>
            {configured ? "已配置" : "未配置"}
          </span>
        </div>
      </div>

      <label className={styles.field} htmlFor="mcpKey">
        <span className={styles.label}>MCP 密钥{configured ? "（已配置，留空不变）" : ""}</span>
        <input
          id="mcpKey"
          className={styles.input}
          type="password"
          value={key}
          autoComplete="off"
          placeholder="粘贴账号 MCP API Key"
          onChange={(e) => { setKey(e.target.value); setSaved(false); }}
        />
      </label>

      <label className={styles.field} htmlFor="mcpBase">
        <span className={styles.label}>MCP Base URL（可选）</span>
        <input
          id="mcpBase"
          className={styles.input}
          type="text"
          value={baseUrl}
          placeholder="留空使用默认端点"
          onChange={(e) => { setBaseUrl(e.target.value); setSaved(false); }}
        />
      </label>

      <div className={styles.actions}>
        <button type="button" className={styles.saveBtn} onClick={save} disabled={saving}>
          {saving ? "保存中" : "保存密钥"}
        </button>
        {saved && <span className={styles.ok}>已保存</span>}
        {error && <span className={styles.err}>{error}</span>}
      </div>
    </div>
  );
}
