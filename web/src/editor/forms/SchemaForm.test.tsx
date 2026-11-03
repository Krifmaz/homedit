import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { FieldValue, WidgetSchema } from "../../api/types";
import { SchemaForm, defaultValues, validateWidget } from "./SchemaForm";

const schema: WidgetSchema = {
  type: "test",
  name: "Test",
  category: "Generic",
  fields: [
    { key: "url", label: "URL", kind: "url", required: true, help: "Base URL" },
    { key: "user", label: "Username", kind: "string", required: false },
    { key: "key", label: "API key", kind: "secret", required: true },
    { key: "interval", label: "Interval", kind: "number", required: false, default: 30 },
    { key: "now", label: "Now playing", kind: "bool", required: false },
    { key: "method", label: "Method", kind: "select", required: true, options: ["GET", "POST"] },
  ],
};

/** Stateful wrapper that records every change. */
function Harness({
  initial,
  spy,
}: {
  initial: Record<string, FieldValue>;
  spy: (v: Record<string, FieldValue>) => void;
}) {
  const [values, setValues] = useState(initial);
  return (
    <SchemaForm
      schema={schema}
      values={values}
      onChange={(v) => {
        spy(v);
        setValues(v);
      }}
    />
  );
}

describe("SchemaForm", () => {
  it("renders an input of the right kind for each field, with labels", () => {
    render(<SchemaForm schema={schema} values={defaultValues(schema)} onChange={() => {}} />);
    expect(screen.getByLabelText("URL")).toHaveAttribute("type", "url");
    expect(screen.getByLabelText("Username")).toHaveAttribute("type", "text");
    expect(screen.getByLabelText("API key")).toHaveAttribute("type", "password");
    expect(screen.getByLabelText("Interval")).toHaveAttribute("type", "number");
    expect(screen.getByLabelText("Interval")).toHaveValue(30);
    expect(screen.getByLabelText("Now playing")).toHaveAttribute("type", "checkbox");
    expect(screen.getByLabelText("Method").tagName).toBe("SELECT");
    expect(screen.getByLabelText("Method")).toHaveValue("GET");
    expect(screen.getByText("Base URL")).toBeInTheDocument();
    expect(screen.getByText("URL")).toHaveClass("required");
    expect(screen.getByText("Username")).not.toHaveClass("required");
  });

  it("reports typed values with the right types", async () => {
    const spy = vi.fn();
    render(<Harness initial={defaultValues(schema)} spy={spy} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Username"), "bob");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ user: "bob" }));
    await user.clear(screen.getByLabelText("Interval"));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ interval: null }));
    await user.type(screen.getByLabelText("Interval"), "45");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ interval: 45 }));
    await user.click(screen.getByLabelText("Now playing"));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ now: true }));
    await user.selectOptions(screen.getByLabelText("Method"), "POST");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ method: "POST" }));
  });

  it("masks a stored secret and lets the user replace it or keep it", async () => {
    const spy = vi.fn();
    render(<Harness initial={{ ...defaultValues(schema), key: { $secret: "set" } }} spy={spy} />);
    const user = userEvent.setup();
    const masked = screen.getByLabelText("API key");
    expect(masked).toHaveAttribute("placeholder", "••••••••");
    expect(masked).toHaveValue("");
    expect(screen.queryByRole("button", { name: "Keep" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Replace" }));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ key: "" }));
    await user.type(screen.getByLabelText("API key"), "s3cret");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ key: "s3cret" }));

    await user.click(screen.getByRole("button", { name: "Keep" }));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ key: { $secret: "set" } }));
    expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ key: null }));
    expect(screen.queryByRole("button", { name: "Replace" })).not.toBeInTheDocument();
  });

  it("shows errors linked to their inputs", () => {
    render(
      <SchemaForm
        schema={schema}
        values={defaultValues(schema)}
        onChange={() => {}}
        errors={{ url: "URL is required" }}
      />,
    );
    const input = screen.getByLabelText("URL");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription(expect.stringContaining("URL is required"));
  });
});

describe("validateWidget", () => {
  const valid = { url: "http://x:1", key: "k", method: "GET" };

  it("accepts valid values", () => {
    expect(validateWidget(schema, valid)).toEqual({});
  });

  it("flags missing required fields", () => {
    const errors = validateWidget(schema, { url: "", key: null });
    expect(Object.keys(errors).sort()).toEqual(["key", "method", "url"]);
    expect(errors.url).toMatch(/required/);
  });

  it("treats a stored secret placeholder as present", () => {
    expect(validateWidget(schema, { ...valid, key: { $secret: "set" } })).toEqual({});
  });

  it("requires http(s) URLs", () => {
    expect(validateWidget(schema, { ...valid, url: "ftp://x" }).url).toMatch(/http/);
    expect(validateWidget(schema, { ...valid, url: "sonarr:8989" }).url).toMatch(/http/);
    expect(validateWidget(schema, { ...valid, url: "https://sonarr.lan" })).toEqual({});
  });

  it("requires numeric numbers", () => {
    expect(validateWidget(schema, { ...valid, interval: "abc" }).interval).toMatch(/number/);
    expect(validateWidget(schema, { ...valid, interval: "12" })).toEqual({});
    expect(validateWidget(schema, { ...valid, interval: 12 })).toEqual({});
  });

  it("requires select values to be one of the options", () => {
    expect(validateWidget(schema, { ...valid, method: "PUT" }).method).toMatch(/one of/);
  });
});

describe("defaultValues", () => {
  it("uses defaults, false for booleans, first option for required selects", () => {
    expect(defaultValues(schema)).toEqual({
      url: null,
      user: null,
      key: null,
      interval: 30,
      now: false,
      method: "GET",
    });
  });
});
