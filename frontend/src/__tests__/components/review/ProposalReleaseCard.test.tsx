import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ProposalReleaseCard } from "../../../components/review/ProposalReleaseCard";
import type { ShadowReplaySample } from "../../../components/review/proposalTypes";
import { GATE_LABELS } from "../../../components/review/evidenceMetrics";

// ProposalReleaseCard 自身拉详情（GET /api/evolution/proposals/:id → ProposalDetailResponse）。
// 取值路径有别（proposalTypes.ts 实证）：diffSummary/riskNote/previousPromptVersion/evalMetrics
// 在 data.proposal.*；cohortRunIds 在 data 顶层（ProposalDetailResponse:117）。
// 组件 load 异步，断言用 findBy*。本文件只覆盖新增的 5 字段元数据渲染与空值处理。

const getMock = vi.fn();

vi.mock("../../../lib/api", () => ({
  api: {
    get: (...args: unknown[]) => getMock(...args),
    post: vi.fn().mockResolvedValue({}),
  },
}));

function baseDetail(overrides: Record<string, unknown> = {}) {
  return {
    proposal: {
      id: "PR1",
      experimentId: "EXP1",
      workspaceId: "W1",
      accountId: "A1",
      kind: "prompt",
      status: "eligible_for_release",
      gateKey: null,
      currentValue: null,
      proposedValue: null,
      cohortNotes: {},
      proposedTemplateKey: "soul",
      proposedSection: "tone",
      diffSummary: "收紧追问节奏，避免连续提问。",
      diffSnippet: "新内容片段",
      criticReasoning: null,
      expectedImprovementOn: null,
      riskNote: "存在轻微语气偏冷风险。",
      previousPromptVersion: "v2.3.1",
      evalMetrics: { win_rate: 0.62, sample_size: 48 },
      evalReplaysCompleted: 10,
      evalReplaysFailed: 0,
      significancePassed: true,
      failureReason: null,
      releasedAt: null,
      releasedBy: null,
      rolledBackAt: null,
      rolledBackBy: null,
      createdAt: "2026-06-01T00:00:00Z",
      updatedAt: "2026-06-02T00:00:00Z",
      ...overrides,
    },
    experiment: null,
    cohortRunIds: ["run-aaa", "run-bbb"],
    shadowReplays: { totalCompleted: 10, totalFailed: 0, samples: [] as ShadowReplaySample[] },
    currentState: {},
  };
}

function renderCard() {
  return render(<ProposalReleaseCard proposalId="PR1" />);
}

describe("ProposalReleaseCard 元数据 5 字段渲染（E12）", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders riskNote/diffSummary/evalMetrics/cohortRunIds/previousPromptVersion", async () => {
    getMock.mockResolvedValue(baseDetail());
    renderCard();

    // diffSummary（data.proposal.diffSummary）
    const diffSummary = await screen.findByTestId("proposal-diff-summary");
    expect(diffSummary).toHaveTextContent("收紧追问节奏，避免连续提问。");
    // riskNote（data.proposal.riskNote）
    expect(screen.getByTestId("proposal-risk-note")).toHaveTextContent(
      "存在轻微语气偏冷风险。",
    );
    // previousPromptVersion（data.proposal.previousPromptVersion）
    expect(screen.getByTestId("proposal-prev-version")).toHaveTextContent("v2.3.1");
    // evalMetrics（data.proposal.evalMetrics，Record key-value）
    const metrics = screen.getByTestId("proposal-eval-metrics");
    expect(metrics).toHaveTextContent("win_rate");
    expect(metrics).toHaveTextContent("0.62");
    expect(metrics).toHaveTextContent("sample_size");
    // cohortRunIds（data 顶层，数组）
    const cohort = screen.getByTestId("proposal-cohort-runs");
    expect(cohort).toHaveTextContent("run-aaa");
    expect(cohort).toHaveTextContent("run-bbb");
  });

  it("5 字段全空（null/{}/[]）时对应区块不渲染", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        diffSummary: null,
        riskNote: null,
        previousPromptVersion: null,
        evalMetrics: {},
      }),
    );
    // cohortRunIds 顶层置空
    getMock.mockResolvedValue({
      ...baseDetail({
        diffSummary: null,
        riskNote: null,
        previousPromptVersion: null,
        evalMetrics: {},
      }),
      cohortRunIds: [],
    });
    renderCard();

    // 等组件加载完成（详情容器出现）
    await screen.findByTestId("proposal-detail");
    await waitFor(() => expect(getMock).toHaveBeenCalled());

    expect(screen.queryByTestId("proposal-diff-summary")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-risk-note")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-prev-version")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-eval-metrics")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-cohort-runs")).not.toBeInTheDocument();
  });
});

describe("ProposalReleaseCard prompt 新旧对照表（阶段三）", () => {
  beforeEach(() => vi.clearAllMocks());

  function promptDetailWithEvidence() {
    const d = baseDetail({
      evalMetrics: {
        kind: "prompt",
        completed_replay_count: 2,
        failed_replay_count: 0,
        five_gate_hit_delta_per_gate: {
          fact_risk_block: -0.5,
          pressure_risk_block: 0,
          human_like_score_rewrite: 0,
          emotional_value_rewrite: 0,
          product_accuracy_score_block: -0.5,
        },
        original_self_critique_addressed_rate: 0.5,
        new_self_critique_addressed_rate: 1.0,
        self_critique_addressed_delta_observed: 0.5,
        token_cost_delta_mean_observed: 88,
      },
    });
    d.shadowReplays = {
      totalCompleted: 2,
      totalFailed: 0,
      samples: [
        {
          id: "s1",
          sourceRunId: "run-001",
          status: "completed",
          failureReason: null,
          originalFinalReviewStatus: "held_by_ai_policy",
          original5gateHit: { fact_risk_block: true, pressure_risk_block: false, human_like_score_rewrite: false, emotional_value_rewrite: false, product_accuracy_score_block: false },
          originalSelfCritiqueAddressed: false,
          newFinalReviewStatus: "approved",
          newReviewRisks: [],
          newTokenCost: 200,
          new5gateHit: { fact_risk_block: false, pressure_risk_block: false, human_like_score_rewrite: false, emotional_value_rewrite: false, product_accuracy_score_block: false },
          newSelfCritiqueAddressed: true,
          similarityToOriginalText: 0,
          startedAt: "2026-06-01T00:00:00Z",
          finishedAt: "2026-06-01T00:00:01Z",
        },
      ] as ShadowReplaySample[],
    };
    return d;
  }

  it("prompt 类有对照数据时渲染聚合表与样本对照表", async () => {
    getMock.mockResolvedValue(promptDetailWithEvidence());
    renderCard();

    const agg = await screen.findByTestId("evidence-aggregate");
    // 5 行 gate 标签
    expect(agg).toHaveTextContent("事实风险");
    expect(agg).toHaveTextContent("产品准确度");
    // 自评率对照
    expect(agg).toHaveTextContent("自评解决率");

    const samples = screen.getByTestId("evidence-samples");
    expect(samples).toHaveTextContent("run-001");
    expect(samples).toHaveTextContent("held_by_ai_policy");
    expect(samples).toHaveTextContent("approved");

    // 补强 1：样本点阵字形回归保护（按 FIVE_GATE_KEYS 序）。
    // run-001 original5gateHit={fact_risk_block:true,其余false} → 原五闸点阵 ●○○○○；
    // new5gateHit={全false} → 新五闸点阵 ○○○○○。● 命中 / ○ 未中 / · 缺失。
    const samplesText = samples.textContent ?? "";
    expect(samplesText).toContain("●○○○○");
    expect(samplesText).toContain("○○○○○");
    // 自评列：original=false→「未解决」、new=true→「已解决」
    expect(samples).toHaveTextContent("未解决");
    expect(samples).toHaveTextContent("已解决");

    // 补强 2：聚合表恰好 5 行五闸——GATE_LABELS 的 5 个中文标签全部出现。
    expect(agg).toHaveTextContent(GATE_LABELS.fact_risk_block);
    expect(agg).toHaveTextContent(GATE_LABELS.pressure_risk_block);
    expect(agg).toHaveTextContent(GATE_LABELS.human_like_score_rewrite);
    expect(agg).toHaveTextContent(GATE_LABELS.emotional_value_rewrite);
    expect(agg).toHaveTextContent(GATE_LABELS.product_accuracy_score_block);
  });

  it("空 original 数据时样本单元格显示占位不 crash", async () => {
    const d = promptDetailWithEvidence();
    d.shadowReplays.samples[0].original5gateHit = {};
    d.shadowReplays.samples[0].originalSelfCritiqueAddressed = null;
    getMock.mockResolvedValue(d);
    renderCard();

    const samples = await screen.findByTestId("evidence-samples");
    expect(samples).toHaveTextContent("run-001"); // 仍渲染，不 crash
  });
});

describe("MetadataSection 聚合证据移出（阶段三）", () => {
  beforeEach(() => vi.clearAllMocks());

  it("prompt 类通用 evalMetrics 表不再平铺已结构化的聚合 key", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        evalMetrics: {
          kind: "prompt",
          five_gate_hit_delta_per_gate: { fact_risk_block: -0.1 },
          self_critique_addressed_delta_observed: 0.2,
          custom_extra_note: "保留这条",
        },
      }),
    );
    renderCard();
    await screen.findByTestId("proposal-detail");
    const metrics = screen.queryByTestId("proposal-eval-metrics");
    // 非白名单 key 仍展示
    expect(metrics).toHaveTextContent("custom_extra_note");
    // 白名单 key 不在通用表里（已被对照表结构化展示）
    expect(metrics).not.toHaveTextContent("five_gate_hit_delta_per_gate");
    expect(metrics).not.toHaveTextContent("self_critique_addressed_delta_observed");
  });

  it("threshold 类通用 evalMetrics 表保留全部 key（含共有的 five_gate_hit_delta_per_gate）", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        kind: "threshold",
        gateKey: "fact_risk_block",
        evalMetrics: {
          kind: "threshold",
          original_send_success_rate: 0.8,
          new_send_success_rate: 0.85,
          five_gate_hit_delta_per_gate: { fact_risk_block: -0.05 },
          safety_regression_rate: 0.0,
        },
      }),
    );
    renderCard();
    await screen.findByTestId("proposal-detail");
    const metrics = screen.getByTestId("proposal-eval-metrics");
    // threshold 类完全不移出：独有 + 共有 key 都在
    expect(metrics).toHaveTextContent("original_send_success_rate");
    expect(metrics).toHaveTextContent("five_gate_hit_delta_per_gate");
    expect(metrics).toHaveTextContent("safety_regression_rate");
  });
});
