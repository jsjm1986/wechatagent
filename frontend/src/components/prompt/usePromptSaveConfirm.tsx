import type { ReactNode } from "react";
import { useConfirm } from "../ui/ConfirmDialog";
import { useStrategyStore, type SavePromptResult } from "../../stores/strategyStore";

// Task 8（路径B）：prompt 编辑保存的二次确认流（组件层）。
// store.savePromptTemplate 返回三态结果，本 hook 负责：
//   - needsConfirm（后端 200 needs_human_confirm）：弹框逐字核对 diff → 勾选后带 force 重提
//   - rejected（后端 4xx 红线语义审查拒绝）：弹框显拒绝理由 + 强制保存入口 → 带 force 重提
//   - ok / error：直接结束（error 已在 store 走 setError）
// 必须在 <ConfirmProvider> 子树内调用。

function diffBody(reason: string, diff: string): ReactNode {
  return promptDiffBody(reason, diff);
}

// 共享的 diff 逐字核对展示块（路径B 各保存入口复用，含质量频道独立 save()）。
export function promptDiffBody(reason: string, diff: string): ReactNode {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <p style={{ margin: 0 }}>
        语义审查提示：{reason || "需人工逐字核对本次改动"}
      </p>
      <p style={{ margin: 0, fontSize: 13, color: "#6b7280" }}>
        请逐字核对以下改动是否变相引入真人转介，触碰自治边界红线：
      </p>
      <pre
        style={{
          margin: 0,
          padding: "8px 10px",
          background: "#0f172a",
          color: "#e2e8f0",
          borderRadius: 6,
          fontSize: 12,
          whiteSpace: "pre-wrap",
          maxHeight: 220,
          overflow: "auto",
        }}
      >
        {diff || "（后端未返回 diff 详情）"}
      </pre>
    </div>
  );
}

export function usePromptSaveConfirm() {
  const confirm = useConfirm();
  const savePromptTemplate = useStrategyStore((s) => s.savePromptTemplate);

  // 返回一个保存触发器：执行保存并按三态走二次确认。
  return async function runSave(): Promise<void> {
    let result: SavePromptResult = await savePromptTemplate();
    if ("ok" in result || "error" in result) return;

    if ("needsConfirm" in result) {
      const ok = await confirm({
        title: "改动需逐字核对后确认",
        body: diffBody(result.reason, result.diff),
        tone: "danger",
        confirmText: "已核对，强制保存",
        requireText: "已核对",
      });
      if (!ok) return;
      result = await savePromptTemplate(true);
      // force 后若仍非 ok（error 已 setError），结束。
      return;
    }

    if ("rejected" in result) {
      const ok = await confirm({
        title: "触碰自治边界红线，已被语义审查拦截",
        body: diffBody(result.reason, ""),
        tone: "danger",
        confirmText: "已核对无误，强制保存",
        requireText: "已核对",
      });
      if (!ok) return;
      await savePromptTemplate(true);
    }
  };
}
