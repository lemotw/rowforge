import { describe, it, expect } from "vitest";
import { uiErrorMessage, type UiError } from "@/ipc/types";

describe("uiErrorMessage", () => {
  it("renders tuple-String variants verbatim", () => {
    const e: UiError = { kind: "workspace_locked", message: "no home dir" };
    expect(uiErrorMessage(e)).toBe("[workspace_locked] no home dir");
  });

  it("renders run_busy struct payload with scope + limit", () => {
    const e: UiError = {
      kind: "run_busy",
      message: { execution_id: "e_TEST", limit: 3, scope: "per_workspace" },
    };
    expect(uiErrorMessage(e)).toBe(
      "[run_busy] per_workspace limit 3 reached",
    );
  });

  it("renders run_aborted with the AbortReason kind", () => {
    const e: UiError = {
      kind: "run_aborted",
      message: { kind: "user_cancelled" },
    };
    expect(uiErrorMessage(e)).toBe("[run_aborted] user_cancelled");
  });

  it("renders export_incomplete with the missing count", () => {
    const e: UiError = {
      kind: "export_incomplete",
      message: { missing_count: 42 },
    };
    expect(uiErrorMessage(e)).toContain("42 row(s) unresolved");
  });

  it("renders duplicate_exec_name with the name field", () => {
    const e: UiError = {
      kind: "duplicate_exec_name",
      message: { name: "smoke_test" },
    };
    expect(uiErrorMessage(e)).toContain("'smoke_test' already exists");
  });

  it("renders manifest_invalid with the error count", () => {
    const e: UiError = {
      kind: "manifest_invalid",
      message: {
        errors: [
          { kind: "manifest_missing", path: "/x/rowforge.yaml" },
          { kind: "parse_failed", message: "boom" },
        ],
      },
    };
    expect(uiErrorMessage(e)).toBe("[manifest_invalid] 2 error(s)");
  });

  it("renders toolchain_missing with the token", () => {
    const e: UiError = {
      kind: "toolchain_missing",
      message: { token: "ghc" },
    };
    expect(uiErrorMessage(e)).toContain("'ghc' not on PATH");
  });

  it("renders editor_not_found", () => {
    expect(uiErrorMessage({ kind: "editor_not_found", message: null }))
      .toContain("editor_not_found");
  });

  it("renders handler_not_found with name", () => {
    expect(uiErrorMessage({ kind: "handler_not_found", message: { name: "foo" } }))
      .toContain("foo");
  });

  it("renders handler_exists with name", () => {
    expect(uiErrorMessage({ kind: "handler_exists", message: { name: "taken" } }))
      .toContain("taken");
  });

  it("renders invalid_handler_name with name", () => {
    expect(uiErrorMessage({ kind: "invalid_handler_name", message: { name: "Bad Name" } }))
      .toContain("Bad Name");
  });

  it("falls back to String() for non-UiError inputs", () => {
    expect(uiErrorMessage("plain string")).toBe("plain string");
    expect(uiErrorMessage(null)).toBe("null");
    expect(uiErrorMessage(undefined)).toBe("undefined");
  });
});
