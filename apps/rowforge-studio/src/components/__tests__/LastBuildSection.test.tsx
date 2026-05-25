import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LastBuildSection } from "@/components/LastBuildSection";
import type { BuildOutcome } from "@/ipc/types";

const ok: BuildOutcome = {
  started_at: "2026-05-25T10:00:00Z",
  finished_at: "2026-05-25T10:00:01Z",
  exit_code: 0,
  command: ["sh", "-c", "echo hi"],
  stdout: "hi\n",
  stderr: "",
};
const fail: BuildOutcome = { ...ok, exit_code: 7, stderr: "oops\n", stdout: "" };

describe("LastBuildSection", () => {
  it("renders nothing when no build and not pending", () => {
    const { container } = render(<LastBuildSection last_build={null} pending={false} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders 'Building…' when pending", () => {
    render(<LastBuildSection last_build={null} pending={true} />);
    expect(screen.getByText(/Building…/)).toBeInTheDocument();
  });

  it("renders success badge for exit_code 0", () => {
    render(<LastBuildSection last_build={ok} pending={false} />);
    expect(screen.getByText("success")).toBeInTheDocument();
    expect(screen.getByText(/exit 0/)).toBeInTheDocument();
  });

  it("renders failed badge for non-zero exit", () => {
    render(<LastBuildSection last_build={fail} pending={false} />);
    expect(screen.getByText("failed")).toBeInTheDocument();
    expect(screen.getByText(/exit 7/)).toBeInTheDocument();
  });

  it("expands log on Show output click", () => {
    render(<LastBuildSection last_build={fail} pending={false} />);
    expect(screen.queryByText(/oops/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByText(/Show output ▾/));
    expect(screen.getByText((c) => c.includes("oops"))).toBeInTheDocument();
  });
});
