import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { LifecycleBanners } from "@/components/LifecycleBanner";

describe("LifecycleBanners", () => {
  it("renders nothing when banners list is empty", () => {
    const { container } = render(<LifecycleBanners banners={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders worker_crashed with stderr_tail details", () => {
    render(
      <LifecycleBanners
        banners={[
          {
            id: "b1",
            kind: "worker_crashed",
            message: "Worker 2 crashed (signal=11)",
            stderr_tail: "boom\noh no\n",
            worker_id: 2,
          },
        ]}
      />
    );
    expect(screen.getByText(/Worker 2 crashed/)).toBeInTheDocument();
    expect(screen.getByText(/stderr tail/)).toBeInTheDocument();
  });

  it("renders multiple banners", () => {
    render(
      <LifecycleBanners
        banners={[
          { id: "1", kind: "stall_warning", message: "No progress for 30s" },
          { id: "2", kind: "pipeline_warning", message: "[EVENT_LAG] 5 dropped" },
        ]}
      />
    );
    expect(screen.getByText(/No progress/)).toBeInTheDocument();
    expect(screen.getByText(/EVENT_LAG/)).toBeInTheDocument();
  });
});
