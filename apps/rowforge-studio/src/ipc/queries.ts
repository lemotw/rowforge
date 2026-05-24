import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { Settings } from "./types";

export const useSettings = () =>
  useQuery({
    queryKey: ["settings"],
    queryFn: ipc.workspace_settings_load,
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
