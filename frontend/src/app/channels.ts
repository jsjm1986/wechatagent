import { lazy } from "react";
import type { ComponentType, LazyExoticComponent } from "react";
import {
  BrainCircuit,
  LayoutDashboard,
  UserRoundCheck,
  UsersRound,
  Sparkles,
  FileText,
  Settings2,
  Bot,
  Activity,
  ShieldCheck,
  Workflow,
  FileBox,
  type LucideIcon,
} from "lucide-react";
import type { Channel } from "../types";

const OverviewFeature = lazy(() => import("../features/overview"));
const CommandCenterFeature = lazy(() => import("../features/command-center"));
const ContentAssetsFeature = lazy(() => import("../features/content-assets"));
const SystemStrategyFeature = lazy(() => import("../features/system-strategy"));
const OperationsFeature = lazy(() => import("../features/operations"));
const AutonomyFeature = lazy(() => import("../features/autonomy"));
const EvolutionFeature = lazy(() => import("../features/evolution"));
const QualityFeature = lazy(() => import("../features/quality"));
const LlmProvidersFeature = lazy(() => import("../features/llm-providers"));
const KnowledgeFeature = lazy(() => import("../features/knowledge"));

export interface ChannelDef {
  id: Channel;
  group: "运营" | "知识" | "系统";
  label: string;
  caption: string;
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  subtitle: string;
  Component: LazyExoticComponent<ComponentType>;
}

// 单一事实来源：合并原 App.tsx 的 channels 数组 + channelTitle/Eyebrow/Subtitle。
// 迁移期间除 overview 外，Component 暂统一指向 OverviewFeature 占位，
// 随阶段 3 各 feature 落地后逐个替换为真实入口。
export const CHANNELS: ChannelDef[] = [
  {
    id: "command",
    group: "运营",
    label: "AI 总控",
    caption: "Command Center",
    icon: BrainCircuit,
    eyebrow: "Management Agent",
    title: "AI Command Center",
    subtitle: "用一个后台管理 Agent 统筹好友、微信群、朋友圈与系统任务。",
    Component: CommandCenterFeature,
  },
  {
    id: "overview",
    group: "运营",
    label: "工作台",
    caption: "运行态势",
    icon: LayoutDashboard,
    eyebrow: "System Overview",
    title: "运营工作台",
    subtitle: "查看微信账号、运营对象、任务和最近事件的整体状态。",
    Component: OverviewFeature,
  },
  {
    id: "userOps",
    group: "运营",
    label: "用户运营",
    caption: "私聊关系运营",
    icon: UserRoundCheck,
    eyebrow: "User Operations",
    title: "用户运营",
    subtitle: "围绕单个好友长期运营，维护用户画像、运营记忆、方法论、提示词和执行边界。",
    Component: OverviewFeature,
  },
  {
    id: "groupOps",
    group: "运营",
    label: "微信群运营",
    caption: "群分析与线索",
    icon: UsersRound,
    eyebrow: "Group Operations",
    title: "微信群运营",
    subtitle: "下一阶段独立建设群画像、群节奏和群工具工作流。",
    Component: OverviewFeature,
  },
  {
    id: "momentOps",
    group: "运营",
    label: "朋友圈运营",
    caption: "内容计划",
    icon: Sparkles,
    eyebrow: "Moment Operations",
    title: "朋友圈运营",
    subtitle: "下一阶段独立建设朋友圈内容计划、发布队列和互动复盘。",
    Component: OverviewFeature,
  },
  {
    id: "content",
    group: "知识",
    label: "内容资产",
    caption: "素材知识",
    icon: FileText,
    eyebrow: "Knowledge Assets",
    title: "内容资产",
    subtitle: "维护产品资料、FAQ、话术、禁用表达、品牌语气和朋友圈素材。",
    Component: ContentAssetsFeature,
  },
  {
    id: "knowledgeWiki",
    group: "知识",
    label: "Wiki 管理",
    caption: "schema / 信号 / 历史",
    icon: FileBox,
    eyebrow: "Knowledge Wiki",
    title: "Wiki 管理",
    subtitle: "管理知识库领域 schema、缺口信号与切片修订历史。",
    Component: KnowledgeFeature,
  },
  {
    id: "systemStrategy",
    group: "系统",
    label: "系统策略",
    caption: "全局与总控",
    icon: Settings2,
    eyebrow: "Global Prompt Policy",
    title: "系统策略",
    subtitle: "管理后台总控 Agent、方法论生成 Agent 和跨模块 Prompt Pack。",
    Component: SystemStrategyFeature,
  },
  {
    id: "llmProviders",
    group: "系统",
    label: "AI 模型配置",
    caption: "LLM Providers",
    icon: Bot,
    eyebrow: "LLM Providers",
    title: "AI 模型配置",
    subtitle: "管理 LLM 服务商：base_url / api_key / model / 协议格式（OpenAI 兼容、Anthropic 兼容）；支持测试连通性与一键热切换激活配置。",
    Component: LlmProvidersFeature,
  },
  {
    id: "operations",
    group: "系统",
    label: "任务日志",
    caption: "执行审计",
    icon: Activity,
    eyebrow: "Execution Audit",
    title: "任务与日志",
    subtitle: "追踪跟进任务、Agent 决策事件和系统执行结果。",
    Component: OperationsFeature,
  },
  {
    id: "autonomy",
    group: "系统",
    label: "自治回路监控",
    caption: "Autonomy Loop",
    icon: ShieldCheck,
    eyebrow: "Autonomy Loop",
    title: "自治回路监控",
    subtitle: "实时监控自治回路：修订触发率、AI 暂缓三类细分、未验证产品声明拦截、发送链路状态与最近修订记录。",
    Component: AutonomyFeature,
  },
  {
    id: "evolution",
    group: "系统",
    label: "演化中心",
    caption: "Self Evolution",
    icon: ShieldCheck,
    eyebrow: "Self Evolution",
    title: "演化中心",
    subtitle: "查看自演化器产出的 experiments、阈值与 Prompt 候选、Shadow 评测与显著性结论；管理员二次确认后发布或回滚。",
    Component: EvolutionFeature,
  },
  {
    id: "quality",
    group: "系统",
    label: "运营成效",
    caption: "指标与质量",
    icon: Workflow,
    eyebrow: "Outcome & Quality",
    title: "运营成效",
    subtitle: "用户回复率、对话深度等长期指标，知识切片自动校验，公式遵守度评测，产品声明兜底标记词管理。",
    Component: QualityFeature,
  },
];
