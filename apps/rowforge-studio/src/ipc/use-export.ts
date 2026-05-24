import { useMutation } from "@tanstack/react-query";
import { ipc } from "./client";
import type { ExecutionId, ExportOpts } from "./types";

/**
 * Plan 5 T11: mutation hook for exec_export.
 * No query invalidation — export doesn't change exec state.
 */
export const useExport = () =>
  useMutation({
    mutationFn: ({ id, opts }: { id: ExecutionId; opts: ExportOpts }) =>
      ipc.exec_export(id, opts),
  });
