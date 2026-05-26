import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { AttemptId, CancelMode, ExecDeleteBulkResult, ExecutionId, FailedPageQuery, RunHandle, RunStartedHandle, Settings } from "./types";

export const useSettings = () =>
  useQuery({
    queryKey: ["settings"],
    queryFn: ipc.workspace_settings_load,
  });

export const useWorkspace = () =>
  useQuery({
    queryKey: ["workspace"],
    queryFn: ipc.workspace_current,
  });

export const useExecList = (enabled: boolean) =>
  useQuery({
    queryKey: ["exec_list"],
    queryFn: ipc.exec_list,
    enabled,
  });

export const useOpenWorkspace = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (path: string | null) => ipc.workspace_open({ path }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
      qc.invalidateQueries({ queryKey: ["settings"] });
      qc.invalidateQueries({ queryKey: ["workspace"] });
    },
  });
};

export const useSaveSettings = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (settings: Settings) =>
      ipc.workspace_settings_save({ settings }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["settings"] }),
  });
};

export const useExecDetail = (id: ExecutionId | null) =>
  useQuery({
    queryKey: ["exec_show", id],
    queryFn: () => ipc.exec_show({ id: id! }),
    enabled: !!id,
  });

export const useAttemptDetail = (e: ExecutionId | null, r: AttemptId | null) =>
  useQuery({
    queryKey: ["attempt_show", e, r],
    queryFn: () => ipc.attempt_show({ executionId: e!, attemptId: r! }),
    enabled: !!e && !!r,
  });

export const useExecRollup = (id: ExecutionId | null, enabled: boolean) =>
  useQuery({
    queryKey: ["exec_rollup", id],
    queryFn: () => ipc.exec_rollup({ id: id! }),
    enabled: enabled && !!id,
    staleTime: 60_000,
  });

// no current consumer; FailedRowsTable uses useInfiniteQuery directly.
// Kept for future single-page callers (e.g. export flows).
export const useFailedPage = (query: FailedPageQuery | null) =>
  useQuery({
    queryKey: ["attempt_failed_page", query?.execution_id, query?.attempt_id, query?.offset, query?.error_code_filter],
    queryFn: () => ipc.attempt_failed_page({ query: query! }),
    enabled: !!query,
  });

export const useRowHistory = (e: ExecutionId | null, seq: number | null) =>
  useQuery({
    queryKey: ["attempt_row_history", e, seq],
    queryFn: () => ipc.attempt_row_history({ executionId: e!, seq: seq! }),
    enabled: !!e && seq !== null,
  });

export const useAttemptFailedRowIds = (
  execId: string,
  attemptId: string | null,
) =>
  useQuery({
    queryKey: ["attempt_failed_row_ids", execId, attemptId],
    queryFn: () =>
      ipc.attempt_failed_row_ids({ execId, attemptId: attemptId! }),
    enabled: !!attemptId,
  });

export const useRunStart = () => {
  const qc = useQueryClient();
  return useMutation<
    RunStartedHandle,
    Error,
    {
      executionId: ExecutionId;
      handlerDir: string;
      rowLimit?: number | null;
      workers?: number | null;
      dryRun?: boolean | null;
      skipAttempted?: boolean | null;
      onlyRowIds?: number[] | null;
    }
  >({
    mutationFn: (args) => ipc.run_start(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
      qc.invalidateQueries({ queryKey: ["exec_show"] });
    },
  });
};

export const useRunCancel = () =>
  useMutation({
    mutationFn: (args: { handle: RunHandle; mode: CancelMode }) =>
      ipc.run_cancel(args),
  });

export const useActiveRuns = () =>
  useQuery({
    queryKey: ["run_active"],
    queryFn: ipc.run_active,
    refetchInterval: 2000, // 2s poll fallback if runs:active event missed
  });

// ===== Plan 10 exec delete =====

/**
 * Delete a single execution by id.
 * Invalidates exec_list on success.
 * Tauri also emits exec_list:refresh after deletion; T7 wires the event
 * listener at the ExecList page level for cross-window invalidation.
 */
export const useExecutionDelete = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { execId: string }) => ipc.execution_delete(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["exec_list"] });
    },
  });
};

/**
 * Delete multiple executions in a single command.
 * Invalidates exec_list only when at least one deletion succeeded.
 * Callers should inspect result.failed for partial-failure reporting.
 */
export const useExecutionDeleteBulk = () => {
  const qc = useQueryClient();
  return useMutation<ExecDeleteBulkResult, Error, { execIds: string[] }>({
    mutationFn: (args) => ipc.execution_delete_bulk(args),
    onSuccess: (result) => {
      if (result.deleted.length > 0) {
        qc.invalidateQueries({ queryKey: ["exec_list"] });
      }
    },
  });
};
