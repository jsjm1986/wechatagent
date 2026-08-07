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
  Inbox,
  PackageSearch,
  Contact,
  SlidersHorizontal,
  BarChart3,
  Megaphone,
  type LucideIcon,
} from "lucide-react";
import type { Channel, DomainProfile } from "../types";

const OverviewFeature = lazy(() => import("../features/overview"));
const CommandCenterFeature = lazy(() => import("../features/command-center"));
const AccountManagementFeature = lazy(() => import("../features/account-management"));
const UserOpsFeature = lazy(() => import("../features/user-ops"));
const ContentAssetsFeature = lazy(() => import("../features/content-assets"));
const SystemStrategyFeature = lazy(() => import("../features/system-strategy"));
const OperationsFeature = lazy(() => import("../features/operations"));
const AutonomyFeature = lazy(() => import("../features/autonomy"));
const EvolutionFeature = lazy(() => import("../features/evolution"));
const QualityFeature = lazy(() => import("../features/quality"));
const LlmProvidersFeature = lazy(() => import("../features/llm-providers"));
const KnowledgeFeature = lazy(() => import("../features/knowledge"));
const ProductsDealsFeature = lazy(() => import("../features/products-deals"));
const ReferralCardsFeature = lazy(() => import("../features/referral-cards"));
const AskHumanFeature = lazy(() => import("../features/ask-human"));
const AskHumanConfigFeature = lazy(() => import("../features/ask-human-config"));
const SendAnalyticsFeature = lazy(() => import("../features/send-analytics"));
const CampaignFeature = lazy(() => import("../features/campaign"));

/** 侧栏一级分组。二级即频道本身，不再往下分层（最多两级）。
 *
 *  分组按「使用频次 + 动作性质」切，不按实现模块切：
 *  - 日常：每天都要开的（总控、工作台、收件箱）
 *  - 运营：对客户做事的业务频道
 *  - 知识与内容：喂给 AI 的素材与知识
 *  - 成效：看结果与审计，不做操作
 *  - 设置：配好就不常动的
 *
 *  历史分组混了两个维度（`账号管理`/`请示通道配置` 是配置却在「运营」，
 *  `运营成效`/`发送成效` 是业务指标却在「系统」），导致既不好按业务找、
 *  也不好按系统层找。 */
export type ChannelGroup = "日常" | "运营" | "知识与内容" | "成效" | "设置";

/** 侧栏一级分组的显示顺序。单一事实来源——Shell.tsx 不再维护第二份顺序数组
 *  （两份必然漂移）。
 *
 *  曾短暂带过 `icon` 与 `hint` 两个字段，那是「图标轨 + 二级面板」形态的需要：
 *  轨上只有图标，分组必须能被一枚图标代表、且需要 tooltip/aria-label 才可读。
 *  该形态已撤销——它拿宽度换高度，而中文标签更缺宽度（侧栏 252 减内边距只剩 228，
 *  轨吃 56 后文字列只有 104px，带「未上线」角标的行仅 48px，「微信群运营」需 65px
 *  故换行）；且纯图标分组（「日常」用仪表盘、「成效」用上升箭头）语义有歧义，
 *  必须 hover 才知道是什么。单列形态下分组标签是文字，两个字段都没有消费者，
 *  故连同那 5 个 lucide 图标 import 一起删掉，不留死代码。 */
export const GROUP_ORDER: ReadonlyArray<ChannelGroup> = [
  "日常",
  "运营",
  "知识与内容",
  "成效",
  "设置",
];

export interface ChannelDef {
  id: Channel;
  group: ChannelGroup;
  label: string;
  caption: string;
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  subtitle: string;
  Component: LazyExoticComponent<ComponentType>;
  /** 频道可见性谓词：未定义→默认显示（白名单退出）；定义了→按返回值。
   *  读 active profile 决定该频道是否对当前行业显示。本期无频道使用，留作扩展点。 */
  visibleWhen?: (profile: DomainProfile | null) => boolean;
  /** 未上线占位频道：`Component` 仍指向 `OverviewFeature`，功能未独立建设。
   *  侧栏渲染成灰显 + 「未上线」角标且不可点，避免点进去看到的是工作台内容。 */
  comingSoon?: boolean;
}

// 单一事实来源：合并原 App.tsx 的 channels 数组 + channelTitle/Eyebrow/Subtitle。
// 迁移期间除 overview 外，Component 暂统一指向 OverviewFeature 占位，
// 随阶段 3 各 feature 落地后逐个替换为真实入口。
export const CHANNELS: ChannelDef[] = [
  {
    id: "command",
    group: "日常",
    label: "AI 总控",
    caption: "Command Center",
    icon: BrainCircuit,
    eyebrow: "Management Agent",
    title: "AI Command Center",
    subtitle: "用一个后台管理 Agent 统筹好友、微信群、朋友圈与系统任务。",
    Component: CommandCenterFeature,
  },
  {
    id: "accountManagement",
    group: "设置",
    label: "账号管理",
    caption: "微信账号",
    icon: Contact,
    eyebrow: "Account Management",
    title: "账号管理",
    subtitle: "管理微信账号、配置 MCP 凭证、监控在线状态。",
    Component: AccountManagementFeature,
  },
  {
    id: "overview",
    group: "日常",
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
    Component: UserOpsFeature,
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
    comingSoon: true,
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
    comingSoon: true,
  },
  {
    id: "content",
    group: "知识与内容",
    label: "内容资产",
    caption: "话术 / 素材",
    icon: FileText,
    eyebrow: "Content Assets",
    title: "内容资产",
    subtitle: "维护 AI 可直接引用发送的话术、FAQ、品牌口吻、禁用表达与文件素材。事实依据与产品口径以知识库为准。",
    Component: ContentAssetsFeature,
  },
  {
    id: "referralCards",
    group: "知识与内容",
    label: "专属顾问",
    caption: "名片引荐库",
    icon: Contact,
    eyebrow: "Referral Cards",
    title: "专属顾问名片库",
    subtitle: "维护可由 AI 主动引荐给客户的真人专属顾问名片，录入引荐条件、审核与启停（辅助模式）。",
    Component: ReferralCardsFeature,
  },
  {
    id: "askHuman",
    group: "日常",
    label: "统一收件箱",
    caption: "Ask-Human Inbox",
    icon: Inbox,
    eyebrow: "Ask-Human",
    title: "统一收件箱",
    subtitle: "所有需要决策/审核的事项收口在此：请示裁决、知识核验、画像发布、经验晋升。",
    Component: AskHumanFeature,
  },
  {
    id: "askHumanConfig",
    group: "设置",
    label: "请示通道配置",
    caption: "Ask-Human Policy",
    icon: SlidersHorizontal,
    eyebrow: "Ask-Human Policy",
    title: "请示通道配置",
    subtitle: "配置决策人链、触发请示的情形、超时转备选与推送频控；保存后即时生效于私聊运营域。",
    Component: AskHumanConfigFeature,
  },
  {
    id: "campaign",
    group: "运营",
    label: "活动",
    caption: "Campaign",
    icon: Megaphone,
    eyebrow: "Campaign",
    title: "活动推送",
    subtitle: "建活动、按购买产品/价值分层圈人预览，查看真实触达分布（已送达/在途/被拦/已请示）。确认推送在 AI 总控对话中完成。",
    Component: CampaignFeature,
  },
  {
    id: "productsDeals",
    group: "运营",
    label: "产品与成交",
    caption: "Products & Deals",
    icon: PackageSearch,
    eyebrow: "Products & Deals",
    title: "产品与成交",
    subtitle: "维护产品目录与价格，登记核实成交，查看客户当前持有与售后状态。",
    Component: ProductsDealsFeature,
  },
  {
    id: "knowledgeWiki",
    group: "知识与内容",
    label: "知识库 Wiki",
    caption: "录入 / 审核 / 问答",
    icon: FileBox,
    eyebrow: "Knowledge Wiki",
    title: "知识库 Wiki",
    subtitle: "录入与审核 AI 的已验证知识内容（导入、问答、待评审），并管理领域 schema、缺口信号与修订历史。",
    Component: KnowledgeFeature,
  },
  {
    id: "systemStrategy",
    group: "设置",
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
    group: "设置",
    label: "AI 模型配置",
    caption: "LLM Providers",
    icon: Bot,
    eyebrow: "LLM Providers",
    title: "AI 模型配置",
    subtitle: "管理 LLM 服务商：base_url / api_key / model / 协议格式（兼容主流 Chat Completions 与 Messages 协议）；支持测试连通性与一键热切换激活配置。",
    Component: LlmProvidersFeature,
  },
  {
    id: "operations",
    group: "成效",
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
    group: "成效",
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
    group: "成效",
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
    group: "成效",
    label: "运营成效",
    caption: "指标与质量",
    icon: Workflow,
    eyebrow: "Outcome & Quality",
    title: "运营成效",
    subtitle: "用户回复率、对话深度等长期指标，知识切片自动校验，公式遵守度评测，产品声明兜底标记词管理。",
    Component: QualityFeature,
  },
  {
    id: "sendAnalytics",
    group: "成效",
    label: "发送成效",
    caption: "Send Analytics",
    icon: BarChart3,
    eyebrow: "Send Analytics",
    title: "发送成效",
    subtitle: "查看 AI 主动发送的素材与专属顾问名片的使用次数、覆盖客户数、响应率与阶段推进率。",
    Component: SendAnalyticsFeature,
  },
];
