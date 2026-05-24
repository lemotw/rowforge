import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { ReplayToggle } from "@/components/ReplayToggle";

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}

function wrap(node: React.ReactNode, initial = "/exec/e1/attempt/a1") {
  return (
    <MemoryRouter initialEntries={[initial]}>
      <Routes>
        <Route path="/exec/:id/attempt/:aid" element={<>{node}<LocationProbe /></>} />
      </Routes>
    </MemoryRouter>
  );
}

describe("ReplayToggle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("default speed is 1x", () => {
    render(wrap(<ReplayToggle executionId="e1" attemptId="a1" />));
    // 1× button has the active style
    const oneX = screen.getByRole("button", { name: /^1×$/ });
    expect(oneX.className).toContain("bg-primary/20");
  });

  it("clicking Replay calls attempt_replay_start and navigates with ?run", async () => {
    (invoke as any).mockResolvedValue("run-replay-1");
    render(wrap(<ReplayToggle executionId="e1" attemptId="a1" />));
    fireEvent.click(screen.getByRole("button", { name: /^Replay$/i }));
    await new Promise((r) => setTimeout(r, 10));
    expect(invoke).toHaveBeenCalledWith("attempt_replay_start", {
      executionId: "e1",
      attemptId: "a1",
      speed: 1,
    });
    // URL should now contain ?run=run-replay-1
    expect(screen.getByTestId("location").textContent).toBe(
      "/exec/e1/attempt/a1?run=run-replay-1"
    );
  });

  it("clicking 10× then Replay sends speed=10", async () => {
    (invoke as any).mockResolvedValue("run-replay-2");
    render(wrap(<ReplayToggle executionId="e1" attemptId="a1" />));
    fireEvent.click(screen.getByRole("button", { name: /^10×$/ }));
    fireEvent.click(screen.getByRole("button", { name: /^Replay$/i }));
    await new Promise((r) => setTimeout(r, 10));
    expect(invoke).toHaveBeenCalledWith("attempt_replay_start", {
      executionId: "e1",
      attemptId: "a1",
      speed: 10,
    });
  });
});
