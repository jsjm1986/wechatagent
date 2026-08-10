import { describe, expect, it } from "vitest";

import {
  DIGEST_CARD_KIND_LABELS,
  DIGEST_SUGGESTED_ACTION_LABELS,
  DIGEST_TARGET_REF_KIND_LABELS,
  digestCardKindLabel,
  digestMetricNameLabel,
  digestSuggestedActionLabel,
  digestTargetRefKindLabel,
} from "../../../features/knowledge/labels";

/// 后端 `knowledge_digest/mod.rs::parse_cards_from_llm_array` 的 allowed_kinds
/// 白名单。任何不在其中的 kind 会被整张卡片丢弃，故前端字典必须逐字覆盖它。
const BACKEND_ALLOWED_KINDS = [
  "chunk_missing_field",
  "chunk_low_hit_rate",
  "chunk_caused_block",
  "pack_outdated",
  "evolution_pending",
  "evolution_released",
  "freeform",
];

/// 同上，allowed_actions 白名单。
const BACKEND_ALLOWED_ACTIONS = [
  "fix_chunk",
  "add_chunk",
  "retag",
  "review_evolution",
  "dismiss",
  "freeform",
];

describe("digest 卡片标签与后端闭集对账", () => {
  it("kind 字典逐字覆盖后端 allowed_kinds", () => {
    for (const kind of BACKEND_ALLOWED_KINDS) {
      expect(DIGEST_CARD_KIND_LABELS[kind], `缺 ${kind}`).toBeTruthy();
      expect(digestCardKindLabel(kind), `${kind} 未翻译`).not.toBe(kind);
    }
  });

  it("kind 字典不含后端会丢弃的键（多写即死键）", () => {
    for (const key of Object.keys(DIGEST_CARD_KIND_LABELS)) {
      expect(BACKEND_ALLOWED_KINDS, `${key} 不在后端白名单内`).toContain(key);
    }
  });

  it("suggestedAction 字典逐字覆盖后端 allowed_actions", () => {
    for (const action of BACKEND_ALLOWED_ACTIONS) {
      expect(DIGEST_SUGGESTED_ACTION_LABELS[action], `缺 ${action}`).toBeTruthy();
      expect(digestSuggestedActionLabel(action), `${action} 未翻译`).not.toBe(action);
    }
  });

  it("未知枚举值回落原文而不是崩或吞", () => {
    expect(digestCardKindLabel("brand_new_kind")).toBe("brand_new_kind");
    expect(digestSuggestedActionLabel("brand_new_action")).toBe("brand_new_action");
    expect(digestCardKindLabel(null)).toBe("—");
    expect(digestCardKindLabel(undefined)).toBe("—");
  });
});

describe("digestMetricNameLabel", () => {
  it("翻译已知指标名", () => {
    expect(digestMetricNameLabel("missing_fields")).toBe("缺失字段数");
    expect(digestMetricNameLabel("hit_rate")).toBe("检索命中率");
    expect(digestMetricNameLabel("block_count")).toBe("拦截次数");
  });

  it("camelCase 与 snake_case 命中同一条中文（metric.name 是 LLM 自由填写）", () => {
    expect(digestMetricNameLabel("missingFields")).toBe("缺失字段数");
    expect(digestMetricNameLabel("hitRate")).toBe("检索命中率");
    expect(digestMetricNameLabel("blockCount")).toBe("拦截次数");
  });

  it("空格 / 连字符 / 大小写混杂也能归一", () => {
    expect(digestMetricNameLabel("missing fields")).toBe("缺失字段数");
    expect(digestMetricNameLabel("missing-fields")).toBe("缺失字段数");
    expect(digestMetricNameLabel("  Hit_Rate  ")).toBe("检索命中率");
  });

  it("未知指标名回落原文（这张表是尽力而为，非闭集）", () => {
    expect(digestMetricNameLabel("some_new_metric")).toBe("some_new_metric");
    expect(digestMetricNameLabel(null)).toBe("—");
  });
});

describe("digestTargetRefKindLabel", () => {
  it("覆盖 prompt 与 models 注释两侧口径的并集", () => {
    // prompts.rs 给 LLM 的枚举
    for (const kind of ["chunk", "pack", "proposal"]) {
      expect(DIGEST_TARGET_REF_KIND_LABELS[kind], `缺 ${kind}`).toBeTruthy();
    }
    // models.rs 历史注释里额外列过的
    for (const kind of ["item", "run", "evolution_proposal"]) {
      expect(DIGEST_TARGET_REF_KIND_LABELS[kind], `缺 ${kind}`).toBeTruthy();
    }
  });

  it("proposal 与 evolution_proposal 同义", () => {
    expect(digestTargetRefKindLabel("proposal")).toBe(digestTargetRefKindLabel("evolution_proposal"));
  });

  it("未知 kind 回落原文", () => {
    expect(digestTargetRefKindLabel("brand_new")).toBe("brand_new");
    expect(digestTargetRefKindLabel(null)).toBe("—");
  });
});
