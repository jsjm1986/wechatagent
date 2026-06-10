import type { OperationDomainConfig, OperationDomainDraft } from "../types";

// USER_RUNTIME_PARAMETER_FIELDS 常量定义
const USER_RUNTIME_PARAMETER_FIELDS: Array<{
  key: string;
  label: string;
  detail: string;
  kind: "number" | "boolean";
  defaultValue: number | boolean;
}> = [
  { key: "recentMessageLimit", label: "上下文消息数", detail: "每次决策读取的最近消息数量", kind: "number", defaultValue: 12 },
  { key: "minReplyIntervalSeconds", label: "最小回复间隔", detail: "避免短时间连续自动回复，单位秒", kind: "number", defaultValue: 20 },
  { key: "maxDailyTouches", label: "每日触达上限", detail: "单个好友每天最多主动触达次数", kind: "number", defaultValue: 3 },
  { key: "maxPendingFollowUps", label: "待跟进上限", detail: "同一好友最多保留的未执行跟进任务", kind: "number", defaultValue: 3 },
  { key: "followUpExpiresHours", label: "跟进过期时间", detail: "超过时间未执行则自动失效，单位小时", kind: "number", defaultValue: 48 },
  { key: "cooldownAfterNoReplyHours", label: "未回复冷却", detail: "用户无回应后的默认冷却时间，单位小时", kind: "number", defaultValue: 24 },
  { key: "hallucinationBlockAt", label: "幻觉风险拦截线", detail: "幻觉风险达到该分值则禁止发送", kind: "number", defaultValue: 6 },
  { key: "knowledgeGroundingBlockBelow", label: "知识落地拦截线", detail: "低于该分值则禁止发送涉及产品/价格/政策的内容", kind: "number", defaultValue: 7 },
  { key: "humanLikeRewriteBelow", label: "真人感重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 6 },
  { key: "emotionalValueRewriteBelow", label: "情绪价值重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 5 },
  { key: "operationStateConfidenceFullReviewBelow", label: "状态置信 Review 线", detail: "低于该分值强制完整 Review", kind: "number", defaultValue: 4 },
  { key: "runTokenBudget", label: "单次 Token 预算", detail: "单次用户运营运行的最大 token", kind: "number", defaultValue: 30000 },
  { key: "runMaxLlmCalls", label: "单次模型调用上限", detail: "单次用户运营最多 LLM 调用次数", kind: "number", defaultValue: 6 },
  { key: "simulationTokenBudget", label: "模拟评测预算", detail: "单次模拟/评测可用 token", kind: "number", defaultValue: 60000 },
  { key: "reactionTokenBudget", label: "反应分析预算", detail: "用户回应分析单次最多 token", kind: "number", defaultValue: 8000 },
  { key: "reactionMaxLlmCalls", label: "反应分析调用上限", detail: "用户回应分析最多 LLM 调用次数", kind: "number", defaultValue: 2 },
  { key: "quietHoursEnabled", label: "作息门控", detail: "开启后客户在休息时段来的消息不立即回，等醒来时段一次性回复；主动跟进也顺延到醒来", kind: "boolean", defaultValue: true },
  { key: "quietHoursStart", label: "休息起点(时)", detail: "进入静默的整点小时，运营方本地时区，0-23，含。默认 22", kind: "number", defaultValue: 22 },
  { key: "quietHoursEnd", label: "醒来时间(时)", detail: "结束静默/醒来回复的整点小时，0-23，不含。默认 8；起点>终点表示跨午夜", kind: "number", defaultValue: 8 }
];

function parseParameterValue(value: string) {
  if (value === "true") return true;
  if (value === "false") return false;
  if (value && !Number.isNaN(Number(value))) return Number(value);
  return value;
}

function jsonText(value: Record<string, unknown>) {
  if (!value || !Object.keys(value).length) return "";
  return JSON.stringify(value, null, 2);
}

function jsonFromText(text: string) {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text);
  } catch {
    return {};
  }
}

function orderedRuntimeParameters(value: Record<string, unknown>) {
  const knownKeys = new Set(USER_RUNTIME_PARAMETER_FIELDS.map((field) => field.key));
  const known = USER_RUNTIME_PARAMETER_FIELDS
    .filter((field) => value[field.key] !== undefined)
    .map((field) => [field.key, value[field.key]] as [string, unknown]);
  const rest = Object.entries(value).filter(([key]) => !knownKeys.has(key));
  return [...known, ...rest];
}

function runtimeParametersText(value: Record<string, unknown>) {
  return orderedRuntimeParameters(value || {})
    .map(([key, item]) => `${key} = ${String(item)}`)
    .join("\n");
}

function runtimeParametersFromText(text: string) {
  return Object.fromEntries(
    text
      .split(/\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const [rawKey, ...rest] = line.split("=");
        const rawValue = rest.join("=").trim();
        return [rawKey.trim(), parseParameterValue(rawValue)];
      })
      .filter(([key]) => key)
  );
}

export function domainPayload(draft: OperationDomainDraft) {
  return {
    name: draft.name,
    goal: draft.goal,
    methodology: draft.methodology,
    workflow: draft.workflow,
    toolPolicy: draft.toolPolicy,
    automationPolicy: draft.automationPolicy,
    reviewPolicy: draft.reviewPolicy,
    runtimeParameters: runtimeParametersFromText(draft.runtimeParameters),
    stateMachine: jsonFromText(draft.stateMachine)
  };
}

export function domainDraftsFromConfigs(configs: OperationDomainConfig[]) {
  return Object.fromEntries(configs.map((config) => [config.domain, domainDraftFromConfig(config)]));
}

export function domainDraftFromConfig(config: OperationDomainConfig): OperationDomainDraft {
  return {
    name: config.name,
    goal: config.goal,
    methodology: config.methodology,
    workflow: config.workflow,
    toolPolicy: config.toolPolicy,
    automationPolicy: config.automationPolicy,
    reviewPolicy: config.reviewPolicy,
    runtimeParameters: runtimeParametersText(config.runtimeParameters),
    stateMachine: jsonText(config.stateMachine)
  };
}