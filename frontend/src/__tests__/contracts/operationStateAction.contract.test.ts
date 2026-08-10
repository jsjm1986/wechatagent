import { describe, expect, it } from "vitest";
import fixture from "../../contracts/operation_state_action_values.fixture.json";
import {
  OPERATION_STATE_ACTION_LABELS,
  OPERATION_STATE_ACTION_VALUES,
} from "../../contracts/operationStateAction.contract";

describe("契约: 状态策略动作闭集", () => {
  it("与后端 fixture 完全一致且每个动作有中文标签", () => {
    expect(OPERATION_STATE_ACTION_VALUES).toEqual(fixture);
    expect(Object.keys(OPERATION_STATE_ACTION_LABELS).sort()).toEqual([...fixture].sort());
    expect(OPERATION_STATE_ACTION_VALUES).toContain("acknowledgement");
  });
});
