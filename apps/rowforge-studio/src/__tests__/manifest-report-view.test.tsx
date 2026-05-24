import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ManifestReportView } from "@/components/ManifestReportView";

describe("ManifestReportView", () => {
  it("renders missing-manifest error", () => {
    render(<ManifestReportView report={{
      manifest: null,
      errors: [{ kind: "manifest_missing", path: "/x/manifest.toml" }],
      warnings: [],
    }} />);
    expect(screen.getByText(/manifest\.toml not found/i)).toBeTruthy();
  });

  it("renders parse-failed error", () => {
    render(<ManifestReportView report={{
      manifest: null,
      errors: [{ kind: "parse_failed", message: "expected = at line 3" }],
      warnings: [],
    }} />);
    expect(screen.getByText(/expected = at line 3/)).toBeTruthy();
  });

  it("renders path-lookup warning and shows manifest version", () => {
    render(<ManifestReportView report={{
      manifest: { name: "h", version: "1.0", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [{ kind: "path_lookup_failed", field: "run", token: "missing-bin" }],
    }} />);
    expect(screen.getByText(/missing-bin/)).toBeTruthy();
    expect(screen.getByText(/v1\.0/)).toBeTruthy();
  });

  it("renders success state when no errors and no warnings", () => {
    render(<ManifestReportView report={{
      manifest: { name: "h", version: "2.1", language: "go", build: null, run: "bin/handler" },
      errors: [],
      warnings: [],
    }} />);
    expect(screen.getByText(/valid/i)).toBeTruthy();
  });
});
