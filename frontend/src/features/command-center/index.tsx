import { useEffect } from "react";
import { ShieldCheck, BrainCircuit, Workflow } from "lucide-react";
import { StatusLine, type StatusLineTone } from "../../components/ui/StatusLine";
import { PlanStep, type PlanStepStatus } from "../../components/ui/PlanStep";
import { useAccountStore } from "../../stores/accountStore";
import { useContactStore } from "../../stores/contactStore";
import { useCommandStore } from "../../stores/commandStore";
import type { CommandToolCall, CommandResult } from "../../types";
import styles from "./CommandCenter.module.css";

const EXAMPLES = ["把 xx 加入 Agent 运营", "发送 xx 给好友 xx", "查看今天失败任务"];

type ResultTone = "good" | "error" | "warn" | "neutral";

function resultTone(status?: string): ResultTone {
  const s = (status || "").toLowerCase();
  if (s.includes("succeeded") || s.includes("success") || s === "ok") return "good";
  if (s.includes("fail") || s.includes("error") || s.includes("blocked")) return "error";
  if (s.includes("dry") || s.includes("warn") || s.includes("pending")) return "warn";
  return "neutral";
}

function planStepStatus(call: CommandToolCall): PlanStepStatus {
  return call.status === "succeeded" || call.status === "dry_run" ? "ready" : "pending";
}

// 工具调用 detail 摘要：dry-run 时摊开 would_execute；真实执行时打出网关/发送结果。
function commandCallDetail(call: CommandToolCall): string {
  if (call.error) return call.error;
  const response = call.response || {};
  if (response.dry_run === true || call.status === "dry_run") {
    const would = response.would_execute as Record<string, unknown> | undefined;
    if (would) {
      const args = would.arguments as Record<string, unknown> | undefined;
      const errorField = would.error as string | undefined;
      const content = args && typeof args.content === "string" ? args.content : undefined;
      const tool = (would.toolName as string | undefined) || call.toolName;
      const summary = [
        `演练：${tool}`,
        content ? `content="${content.slice(0, 60)}"` : "",
        errorField ? `error=${errorField}` : ""
      ].filter(Boolean).join(" · ");
      return summary || `演练：${tool}（不真实执行）`;
    }
    return "演练模式：未实际调用工具";
  }
  const sentContent = response.sentContent;
  const messageId = response.messageId;
  const reviewApproved = response.reviewApproved;
  const gatewayStatus = response.gatewayStatus;
  const gatewayReason = response.gatewayReason;
  if (typeof sentContent === "string" && sentContent.trim()) {
    return [
      `实际发送：${sentContent}`,
      gatewayStatus ? `网关：${String(gatewayStatus)}` : "",
      reviewApproved !== undefined ? `Review：${reviewApproved ? "通过" : "未通过"}` : "",
      messageId ? `messageId：${String(messageId)}` : "",
      gatewayReason ? `原因：${String(gatewayReason)}` : ""
    ].filter(Boolean).join(" · ");
  }
  return call.status;
}

function resultTitle(result: CommandResult): string {
  return result.status === "dry_run" ? "DRY-RUN 演练" : result.status;
}

export default function CommandCenterFeature() {
  const accounts = useAccountStore((s) => s.accounts);
  const onlineCount = useAccountStore((s) => s.onlineCount());
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const currentAccount = useAccountStore((s) => s.currentAccount());

  const managedCount = useContactStore((s) => s.managedCount());

  const {
    commandDraft,
    commandResult,
    commandDryRun,
    commandBusy,
    souls,
    assets,
    pendingTasks,
    setCommandDraft,
    setCommandDryRun,
    loadCommandData,
    runCommand
  } = useCommandStore();

  useEffect(() => {
    loadCommandData(currentAccountId);
  }, [currentAccountId, loadCommandData]);

  const handleRunCommand = () => {
    if (currentAccountId) {
      runCommand(currentAccountId);
    }
  };

  const accountTone: StatusLineTone = currentAccount?.mcpKeyConfigured ? "ai" : "warn";

  return (
    <div className={styles.page}>
      <section className={styles.layout}>
        {/* —— 操作范围 —— */}
        <aside className={`${styles.panel} ${styles.scopePanel}`}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Scope</span>
              <span className={styles.title}>操作范围</span>
            </div>
            <span className={styles.headIcon}><ShieldCheck size={18} /></span>
          </div>
          <div className={styles.scopeStack}>
            <StatusLine label="微信账号" value={`${onlineCount}/${accounts.length} 在线`} tone="good" />
            <StatusLine
              label="当前账号"
              value={currentAccount?.alias || currentAccount?.displayName || currentAccount?.accountId || "-"}
              tone={accountTone}
            />
            <StatusLine label="运营好友" value={`${managedCount} managed`} tone="ai" />
            <StatusLine label="待执行任务" value={`${pendingTasks} pending`} tone={pendingTasks ? "warn" : "neutral"} />
            <StatusLine label="内容资产" value={`${assets.length} assets`} tone="neutral" />
            <StatusLine label="Agent Soul" value={`${souls.length} versions`} tone="neutral" />
          </div>
          <div className={styles.boundaryBox}>
            <strong>执行边界</strong>
            <p>当前版本开放完整 MCP 工具目录给 Management Agent，所有调用通过后端账号凭证代理并写入审计日志。</p>
          </div>
        </aside>

        {/* —— 指令面板 —— */}
        <section className={styles.commandPanel}>
          <div className={styles.commandHeader}>
            <span className={styles.commandHeaderIcon}><BrainCircuit size={20} /></span>
            <div className={styles.commandHeaderTxt}>
              <strong>Management Agent<span className={styles.liveDot} /></strong>
              <span>用自然语言管理好友、群、朋友圈和任务。</span>
            </div>
          </div>

          <label className={styles.commandInput}>
            <textarea value={commandDraft} onChange={(event) => setCommandDraft(event.target.value)} />
          </label>

          <div className={styles.suggestionRow}>
            {EXAMPLES.map((item) => (
              <button key={item} className={styles.chip} onClick={() => setCommandDraft(item)}>
                {item}
              </button>
            ))}
          </div>

          <div className={styles.actions}>
            <button
              className={`${styles.runBtn} ${commandBusy ? styles.busy : ""}`}
              onClick={handleRunCommand}
              disabled={commandBusy || !commandDraft.trim()}
            >
              <Workflow size={16} />
              {commandBusy ? "执行中" : "执行指令"}
            </button>
            {/* dry-run toggle：打开后写库/发消息工具只回放 would_execute，不实际触达 MCP。 */}
            <label className={styles.dryRunToggle}>
              <input
                type="checkbox"
                checked={commandDryRun}
                onChange={(event) => setCommandDryRun(event.target.checked)}
              />
              <span>Dry-run（不写业务库）</span>
            </label>
            <span className={`${styles.modeBadge} ${commandDryRun ? styles.dryRun : styles.live}`}>
              {commandDryRun ? "演练模式" : "真实执行"}
            </span>
            <span className={styles.hint}>LLM 生成工具计划，后端逐步调用 MCP 并记录结果</span>
          </div>

          {commandResult && (
            <div className={`${styles.result} ${styles[resultTone(commandResult.status)]}`}>
              <strong>{resultTitle(commandResult)}</strong>
              <p>{commandResult.summary}</p>
            </div>
          )}
        </section>

        {/* —— 执行计划 —— */}
        <aside className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Plan Preview</span>
              <span className={styles.title}>执行计划</span>
            </div>
            <span className={styles.headIcon}><Workflow size={18} /></span>
          </div>
          {commandResult?.toolCalls.length ? (
            <div className={styles.planSteps}>
              {commandResult.toolCalls.map((call) => (
                <PlanStep
                  key={call.id || call.toolName}
                  status={planStepStatus(call)}
                  title={call.toolName}
                  detail={commandCallDetail(call)}
                />
              ))}
            </div>
          ) : (
            <div className={styles.planSteps}>
              <PlanStep status="ready" title="加载工具目录" detail="从当前账号 MCP Server 获取完整工具列表" />
              <PlanStep status="pending" title="生成执行计划" detail="LLM 选择工具并输出结构化 JSON" />
              <PlanStep status="pending" title="调用 MCP 工具" detail="后端代理执行并记录日志" />
            </div>
          )}
        </aside>
      </section>
    </div>
  );
}
