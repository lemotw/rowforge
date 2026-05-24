import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { PhaseChipBar } from "@/components/PhaseChipBar";

describe("PhaseChipBar", () => {
  it("shows ✓ for past phases and ● for current", () => {
    const { container } = render(<PhaseChipBar current="starting" />);
    const text = container.textContent ?? "";
    // initializing + snapshotting are past (✓), starting is current (●)
    expect(text).toMatch(/✓\s*Init/);
    expect(text).toMatch(/✓\s*Snap/);
    expect(text).toMatch(/●\s*Start/);
    // running/cancelling/persisting are future — no markers
    expect(text).toContain("Run");
  });

  it("no phase set renders all as future (no ● or ✓)", () => {
    const { container } = render(<PhaseChipBar current={null} />);
    expect(container.textContent ?? "").not.toMatch(/●/);
    expect(container.textContent ?? "").not.toMatch(/✓/);
  });
});
