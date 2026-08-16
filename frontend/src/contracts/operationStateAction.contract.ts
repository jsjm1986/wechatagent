// Canonical runtime vocabulary emitted by GET /api/admin/operation-state-policies.
export const OPERATION_STATE_ACTION_VALUES = [
  "reply",
  "acknowledgement",
  "silent",
  "follow_up",
  "cooldown",
  "appointment_request",
] as const;

export type OperationStateAction = (typeof OPERATION_STATE_ACTION_VALUES)[number];

export const OPERATION_STATE_ACTION_LABELS: Record<OperationStateAction, string> = {
  reply: "业务回复",
  acknowledgement: "中性确认",
  silent: "静默",
  follow_up: "后续跟进",
  cooldown: "进入冷却",
  appointment_request: "记录预约请求",
};
