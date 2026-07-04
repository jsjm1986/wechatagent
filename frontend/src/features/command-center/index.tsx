import { useEffect } from "react";
import { ShieldCheck, BrainCircuit, Workflow } from "lucide-react";
import { StatusLine, type StatusLineTone } from "../../components/ui/StatusLine";
import { PlanStep, type PlanStepStatus } from "../../components/ui/PlanStep";
import { useAccountStore } from "../../stores/accountStore";
import { useContactStore } from "../../stores/contactStore";
import { useCommandStore } from "../../stores/commandStore";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CommandToolCall, CommandResult } from "../../types";
import { McpKeyForm } from "./McpKeyForm";
import { GATEWAY_STATUS_LABELS, labelOf } from "../../lib/reviewLabels";
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

// tool call 终态标签：闭集见 src/routes/management.rs CommandToolCall.status。
// executed_unverified = 工具 Ok 但业务结果未核实，必须显「待核实」不当成功展示（诚实优于好看）。
function callStatusLabel(status: string): string {
  switch (status) {
    case "succeeded":
      return "✅ 成功";
    case "failed":
      return "❌ 失败";
    case "executed_unverified":
      return "⚠️ 待核实";
    case "dry_run":
      return "演练";
    case "running":
      return "进行中";
    default:
      return status;
  }
}

// gateway 终态闭集（src/agent/run_envelope.rs GATEWAY_STATUS_VALUES，32 值）→ 中文业务语义标签。
// 单一真相源统一到 lib/reviewLabels.ts 的 GATEWAY_STATUS_LABELS（与 finalReviewStatus 交集键复用同措辞）。
function gatewayStatusLabel(status: string): string {
  return labelOf(GATEWAY_STATUS_LABELS, status);
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
      gatewayStatus ? `网关：${gatewayStatusLabel(String(gatewayStatus))}` : "",
      reviewApproved !== undefined ? `复核：${reviewApproved ? "通过" : "未通过"}` : "",
      messageId ? `消息编号：${String(messageId)}` : "",
      gatewayReason ? `原因：${String(gatewayReason)}` : ""
    ].filter(Boolean).join(" · ");
  }
  return call.status;
}

// 活动推送结果跳转守卫：仅当 dispatch_campaign 真实执行成功且带 campaignId 才给跳转 id，
// 否则返回 null（dry-run / 待确认 / 非该工具 / 无 id 一律不渲，防死链）。
export function dispatchCampaignId(call: CommandToolCall): string | null {
  if (call.toolName !== "wechatagent.dispatch_campaign") return null;
  if (call.status !== "succeeded" && call.status !== "executed_unverified") return null;
  const id = call.response?.campaignId;
  return typeof id === "string" ? id : null;
}

// 命令运行整体状态(src/routes/management.rs final_status 闭集:running/pending_confirmation/
// failed/dry_run/succeeded);未知值回落原值。
const COMMAND_STATUS_LABELS: Record<string, string> = {
  running: "执行中",
  pending_confirmation: "待确认",
  failed: "已失败",
  dry_run: "演练（未真实执行）",
  succeeded: "已完成",
};

function resultTitle(result: CommandResult): string {
  return COMMAND_STATUS_LABELS[result.status] ?? result.status;
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
    runCommand,
    confirmCommand,
    rejectCommand
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
            <StatusLine label="运营好友" value={`${managedCount} 位运营中`} tone="ai" />
            <StatusLine label="待执行任务" value={`${pendingTasks} 个待执行`} tone={pendingTasks ? "warn" : "neutral"} />
            <StatusLine label="内容资产" value={`${assets.length} 个素材`} tone="neutral" />
            <StatusLine label="AI 人格" value={`${souls.length} 个版本`} tone="neutral" />
          </div>
          <div className={styles.boundaryBox}>
            <strong>执行边界</strong>
            <p>当前版本开放完整 MCP 工具目录给 Management Agent，所有调用通过后端账号凭证代理并写入审计日志。</p>
          </div>
          {currentAccount?.id && (
            <McpKeyForm accountId={currentAccount.id} configured={!!currentAccount.mcpKeyConfigured} />
          )}
        </aside>

        {/* —— 指令面板 —— */}
        <section className={styles.commandPanel}>
          <div className={styles.commandHeader}>
            <span className={styles.commandHeaderIcon}><BrainCircuit size={20} /></span>
            <div className={styles.commandHeaderTxt}>
              <strong>管理助手<span className={styles.liveDot} /></strong>
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
              <span>演练模式（只预演，不写业务库）</span>
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

          {commandResult?.status === "pending_confirmation" && (
            <div className={styles.confirmBar}>
              <span className={styles.confirmHint}>该计划包含高风险操作，确认前不会真实执行。</span>
              <div className={styles.confirmActions}>
                <button
                  className={styles.confirmBtn}
                  onClick={() => confirmCommand(commandResult.id)}
                  disabled={commandBusy}
                >
                  确认执行
                </button>
                <button
                  className={styles.rejectBtn}
                  onClick={() => rejectCommand(commandResult.id)}
                  disabled={commandBusy}
                >
                  否决
                </button>
              </div>
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
              {commandResult.toolCalls.map((call) => {
                const campaignId = dispatchCampaignId(call);
                return (
                  <div key={call.id || call.toolName}>
                    <PlanStep
                      status={planStepStatus(call)}
                      title={`${call.toolName} · ${callStatusLabel(call.status)}`}
                      detail={commandCallDetail(call)}
                    />
                    {campaignId && (
                      <button
                        type="button"
                        className={styles.campaignJump}
                        onClick={() => useCampaignStore.getState().openReport(campaignId)}
                      >
                        查看推送结果 →
                      </button>
                    )}
                  </div>
                );
              })}
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
