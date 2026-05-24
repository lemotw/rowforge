import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { AttemptId, ExecutionId, FailedPageQuery, Settings } from "./types";

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
