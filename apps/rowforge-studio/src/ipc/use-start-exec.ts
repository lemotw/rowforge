import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { StartExecArgs } from "./types";

/**
 * Plan 5 T11: mutation hook for exec_start.
 * Invalidates the exec_list query on success so Workspace Home
 * re-fetches the new exec into the list.
 */
export const useStartExec = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: StartExecArgs) => ipc.exec_start(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
    },
  });
};
