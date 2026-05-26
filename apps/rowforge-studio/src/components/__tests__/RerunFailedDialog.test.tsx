import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { RerunFailedDialog } from "@/components/RerunFailedDialog";

describe("RerunFailedDialog", () => {
  it("renders title with row count (plural)", () => {
    render(
      <RerunFailedDialog
        open={true}
        onOpenChange={() => {}}
        rowCount={5}
        handlerDir="/path"
        sourceAttemptId="r_test"
        onConfirm={() => {}}
        isPending={false}
      />,
    );
    expect(screen.getByText(/Re-run 5 failed rows\?/)).toBeInTheDocument();
  });

  it("renders singular for 1 row", () => {
    render(
      <RerunFailedDialog
        open={true}
        onOpenChange={() => {}}
        rowCount={1}
        handlerDir="/path"
        sourceAttemptId="r_test"
        onConfirm={() => {}}
        isPending={false}
      />,
    );
    expect(screen.getByText(/Re-run 1 failed row\?/)).toBeInTheDocument();
  });

  it("shows handler and source attempt info", () => {
    render(
      <RerunFailedDialog
        open={true}
        onOpenChange={() => {}}
        rowCount={3}
        handlerDir="/handlers/alpha"
        sourceAttemptId="r_01ABC"
        onConfirm={() => {}}
        isPending={false}
      />,
    );
    expect(screen.getByText(/\/handlers\/alpha/)).toBeInTheDocument();
    expect(screen.getByText(/r_01ABC/)).toBeInTheDocument();
  });

  it("disables Re-run when isPending", () => {
    render(
      <RerunFailedDialog
        open={true}
        onOpenChange={() => {}}
        rowCount={3}
        handlerDir="/path"
        sourceAttemptId="r_test"
        onConfirm={() => {}}
        isPending={true}
      />,
    );
    const btn = screen.getByRole("button", { name: /Starting…/ });
    expect(btn).toBeDisabled();
  });

  it("calls onConfirm on Re-run click", () => {
    const onConfirm = vi.fn();
    render(
      <RerunFailedDialog
        open={true}
        onOpenChange={() => {}}
        rowCount={3}
        handlerDir="/path"
        sourceAttemptId="r_test"
        onConfirm={onConfirm}
        isPending={false}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^Re-run 3 rows$/ }));
    expect(onConfirm).toHaveBeenCalled();
  });
});
