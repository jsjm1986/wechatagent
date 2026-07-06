import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { VERSION_STATUS_LABELS, labelOf } from "../../lib/reviewLabels";
import styles from "./EvaluationScenariosPanel.module.css";

// 评测场景配置入口：管理员自助维护 formula-adherence 评测所依赖的 active evaluation_scenarios，
// 不再需要后端写库。列表（GET）+ 新建（POST）+ 删除（DELETE）。body 全 camelCase，端点无 /admin 前缀。

type Scenario = {
  id: string;
  scenarioId: string;
  title: string;
  description?: string;
  inboundMessages?: string[];
  tags?: string[];
  status?: string;
};

export function EvaluationScenariosPanel() {
  const [items, setItems] = useState<Scenario[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const [scenarioId, setScenarioId] = useState("");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [inboundText, setInboundText] = useState("");
  const [submitting, setSubmitting] = useState(false);

  async function load() {
    setLoading(true);
    setErr("");
    try {
      const data = await api.get<{ items: Scenario[] }>("/api/evaluation-scenarios");
      setItems(data.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function create() {
    if (!scenarioId.trim() || !title.trim()) {
      setErr("场景标识与场景标题为必填项。");
      return;
    }
    const inboundMessages = inboundText
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
    setSubmitting(true);
    setErr("");
    try {
      await api.post("/api/evaluation-scenarios", {
        scenarioId: scenarioId.trim(),
        title: title.trim(),
        description: description.trim(),
        inboundMessages,
      });
      setScenarioId("");
      setTitle("");
      setDescription("");
      setInboundText("");
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  async function remove(scenario: Scenario) {
    if (!scenario.id) return;
    setErr("");
    try {
      await api.delete(`/api/evaluation-scenarios/${scenario.id}`);
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className={styles.panel}>
      <p className={styles.desc}>
        管理「公式遵守度」评测所依赖的评测场景。只有<strong>启用中</strong>的场景会被评测跑到。
        每个场景包含输入消息序列与标准答案；这里维护基础字段，更细的初始联系人 / 标准答案仍可由后端补全。
      </p>

      <form
        className={styles.form}
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <div className={styles.formRow}>
          <input
            className={styles.input}
            placeholder="场景标识(scenarioId)"
            value={scenarioId}
            onChange={(e) => setScenarioId(e.target.value)}
          />
          <input
            className={styles.input}
            placeholder="场景标题"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
        </div>
        <input
          className={styles.inputWide}
          placeholder="场景描述（选填）"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
        <textarea
          className={styles.textarea}
          placeholder="输入消息（每行一条，模拟客户依次发来的话）"
          value={inboundText}
          onChange={(e) => setInboundText(e.target.value)}
        />
        <div className={styles.actions}>
          <button className={styles.btnPrimary} type="submit" disabled={submitting}>
            {submitting ? "提交中" : "新建场景"}
          </button>
          <button
            className={styles.btnGhost}
            type="button"
            onClick={() => void load()}
            disabled={loading}
          >
            {loading ? "加载中" : "刷新"}
          </button>
        </div>
      </form>

      {err && <div className={styles.error}>{err}</div>}

      {items.length === 0 && !loading ? (
        <p className={styles.hint}>还没有评测场景。新建一个 active 场景后，公式遵守度评测才有基准可跑。</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>场景</th>
              <th>状态</th>
              <th>输入消息数</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {items.map((s) => (
              <tr key={s.id || s.scenarioId}>
                <td>
                  <strong>{s.title}</strong>
                  <br />
                  <small>{s.scenarioId}</small>
                </td>
                <td>{labelOf(VERSION_STATUS_LABELS, s.status)}</td>
                <td>{s.inboundMessages?.length ?? 0}</td>
                <td>
                  <button
                    className={styles.btnGhost}
                    type="button"
                    onClick={() => void remove(s)}
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
