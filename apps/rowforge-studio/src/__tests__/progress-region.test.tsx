import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProgressRegion } from "@/components/ProgressRegion";
import { initialRunState } from "@/ipc/run-state";

describe("ProgressRegion", () => {
  it("renders percent when total is known", () => {
    const state = { ...initialRunState, processed: 250, total: 1000, rate_10s: 200 };
    render(<ProgressRegion state={state} />);
    expect(screen.getByText(/250 \/ 1,000 \(25\.0%\)/)).toBeInTheDocument();
  });

  it("renders em-dash when total is unknown", () => {
    const state = { ...initialRunState, processed: 250, total: null };
    render(<ProgressRegion state={state} />);
    expect(screen.getByText(/250 \/ —/)).toBeInTheDocument();
  });

  it("formats ETA as minutes:seconds when above 60s", () => {
    const state = { ...initialRunState, eta_ms: 125_000 };
    render(<ProgressRegion state={state} />);
    expect(screen.getByText("2m 05s")).toBeInTheDocument();
  });

  it("shows em-dash for ETA when null", () => {
    render(<ProgressRegion state={initialRunState} />);
    const etaLabel = screen.getByText("ETA");
    // ETA value is the sibling div above the label
    expect(etaLabel.parentElement?.parentElement?.textContent).toContain("—");
  });
});
