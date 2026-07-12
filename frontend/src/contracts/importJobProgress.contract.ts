// 契约对账声明 —— 异步导入 job 进度投影 import_job_progress_json 的线上键集。
// 仅服务契约对账测试，非业务类型。get/list import-preview-job 端点前端轮询按此读进度。
// 后端改投影→re-bless fixture→此处对账测红，强制前端同步。
export const CANONICAL_KEYS = [
  "jobId",
  "status",
  "progress",
  "result",
  "error",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
