import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FormDialogProvider, useFormDialog, type FormField } from "../../../../components/ui/FormDialog";

function Harness({ fields, onResult }: { fields: FormField[]; onResult: (v: Record<string, string> | null) => void }) {
  const form = useFormDialog();
  return (
    <button
      onClick={async () => {
        const v = await form({ title: "拆分知识条目", fields });
        onResult(v);
      }}
    >
      触发
    </button>
  );
}

function setup(fields: FormField[]) {
  const results: (Record<string, string> | null)[] = [];
  render(
    <FormDialogProvider>
      <Harness fields={fields} onResult={(v) => results.push(v)} />
    </FormDialogProvider>
  );
  fireEvent.click(screen.getByText("触发"));
  return results;
}

describe("FormDialog", () => {
  it("渲染字段，提交返回各字段值", async () => {
    const results = setup([
      { kind: "text", name: "reason", label: "原因" },
      { kind: "select", name: "kind", label: "关系类型", options: [
        { value: "supports", label: "支持" },
        { value: "contradicts", label: "矛盾" },
      ] },
    ]);
    await screen.findByText("拆分知识条目");
    fireEvent.change(screen.getByLabelText(/原因/), { target: { value: "内容过期" } });
    fireEvent.change(screen.getByLabelText(/关系类型/), { target: { value: "contradicts" } });
    fireEvent.click(screen.getByRole("button", { name: "确定" }));
    await waitFor(() => expect(results).toEqual([{ reason: "内容过期", kind: "contradicts" }]));
  });

  it("required 字段为空时提交按钮禁用", async () => {
    setup([{ kind: "text", name: "reason", label: "退回原因", required: true }]);
    await screen.findByText("拆分知识条目");
    expect(screen.getByRole("button", { name: "确定" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/退回原因/), { target: { value: "来源失效" } });
    expect(screen.getByRole("button", { name: "确定" })).toBeEnabled();
  });

  it("取消返回 null", async () => {
    const results = setup([{ kind: "text", name: "x", label: "随便" }]);
    await screen.findByText("拆分知识条目");
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(results).toEqual([null]));
  });
});
