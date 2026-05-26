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

  it("renders toolchain_missing with the tool name", () => {
    const e: UiError = {
      kind: "toolchain_missing",
      message: { name: "my-handler", tool: "ghc" },
    };
    expect(uiErrorMessage(e)).toContain("ghc");
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

  it("renders build_failed copy", () => {
    expect(
      uiErrorMessage({ kind: "build_failed", message: { name: "alpha", exit_code: 3 } })
    ).toContain("Build failed");
  });

  it("renders toolchain_missing copy", () => {
    expect(
      uiErrorMessage({ kind: "toolchain_missing", message: { name: "alpha", tool: "go" } })
    ).toContain("go");
  });

  it("renders no_build_command copy", () => {
    expect(
      uiErrorMessage({ kind: "no_build_command", message: { name: "alpha" } })
    ).toContain("entry.build");
  });

  it("renders execution_in_use copy", () => {
    expect(uiErrorMessage({ kind: "execution_in_use", message: { exec_id: "e_test" } })).toContain("active run");
  });

  it("falls back to String() for non-UiError inputs", () => {
    expect(uiErrorMessage("plain string")).toBe("plain string");
    expect(uiErrorMessage(null)).toBe("null");
    expect(uiErrorMessage(undefined)).toBe("undefined");
  });
});
