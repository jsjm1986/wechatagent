import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DomainSchemaEditor } from "../../../features/knowledge/DomainSchemaEditor";

describe("DomainSchemaEditor", () => {
  it("create 提交 body 为 camelCase 键（schemaId/fields[allowedValues]/aliasDict）", () => {
    const onSubmit = vi.fn();
    render(<DomainSchemaEditor mode="create" onSubmit={onSubmit} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText(/schemaId/i), { target: { value: "real_estate" } });
    fireEvent.change(screen.getByPlaceholderText(/字段表名称/), { target: { value: "房产销售" } });
    // 加一个字段
    fireEvent.click(screen.getByText(/添加字段/));
    fireEvent.change(screen.getByPlaceholderText(/字段名.*name/i), { target: { value: "stage" } });
    fireEvent.change(screen.getByPlaceholderText(/中文标签.*label/i), { target: { value: "阶段" } });
    fireEvent.click(screen.getByText(/保存/));
    const body = onSubmit.mock.calls[0][0];
    expect(body).toHaveProperty("schemaId", "real_estate");
    expect(body).toHaveProperty("name", "房产销售");
    expect(Array.isArray(body.fields)).toBe(true);
    expect(body.fields[0]).toHaveProperty("name", "stage");
    // 关键：wire 键必须 camelCase，不是 allowed_values / alias_of
    expect(body.fields[0]).not.toHaveProperty("allowed_values");
    expect(body).toHaveProperty("aliasDict");
  });

  it("schemaId/name 为空时不提交（必填校验）", () => {
    const onSubmit = vi.fn();
    render(<DomainSchemaEditor mode="create" onSubmit={onSubmit} onCancel={vi.fn()} />);
    fireEvent.click(screen.getByText(/保存/));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("edit 模式回填已有 schema 且 schemaId 只读", () => {
    const existing = {
      schemaId: "x",
      version: 7,
      name: "旧名",
      fields: [{ name: "f1", label: "字段1", kind: "string", required: false }],
      aliasDict: {},
      guardDsl: null,
    };
    const onSubmit = vi.fn();
    render(<DomainSchemaEditor mode="edit" initial={existing as never} onSubmit={onSubmit} onCancel={vi.fn()} />);
    expect(screen.getByDisplayValue("旧名")).toBeInTheDocument();
    expect(screen.getByDisplayValue("字段1")).toBeInTheDocument();
    // edit 模式 schemaId 输入框只读
    const schemaIdInput = screen.getByDisplayValue("x") as HTMLInputElement;
    expect(schemaIdInput.disabled).toBe(true);
    fireEvent.click(screen.getByText("保存"));
    expect(onSubmit.mock.calls[0][0]).toHaveProperty("expectedVersion", 7);
  });
});
