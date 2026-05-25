import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc } from "./client";
import type { ScaffoldArgs } from "./types";

/** Plan 7: list all handlers under <workspace>/handlers/. */
export const useHandlerList = () =>
  useQuery({
    queryKey: ["handler_list"],
    queryFn: () => ipc.handler_list(),
  });

/** Plan 7: load a single handler's detail. */
export const useHandlerShow = (name: string | null) =>
  useQuery({
    queryKey: ["handler_show", name],
    queryFn: () => ipc.handler_show({ name: name! }),
    enabled: !!name,
  });

/** Plan 7: spawn external editor at the handler dir. */
export const useHandlerOpenEditor = () =>
  useMutation({
    mutationFn: (args: { name: string }) => ipc.handler_open_editor(args),
  });

/** Plan 7: reveal handler dir in OS file manager. */
export const useHandlerReveal = () =>
  useMutation({
    mutationFn: (args: { name: string }) => ipc.handler_reveal(args),
  });

/**
 * Plan 7: scaffold a new handler. Invalidates the list query on success
 * so newly-created handlers appear immediately.
 *
 * Mutating commands also emit `handlers:list` events from the Rust side;
 * the page-level listener triggers a separate invalidate path. This
 * onSuccess invalidate covers the same-component happy path without
 * waiting for the event round-trip.
 */
export const useHandlerScaffold = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: ScaffoldArgs) => ipc.handler_scaffold(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    },
  });
};

/** Plan 7: delete a handler. Invalidates list + the specific show query. */
export const useHandlerDelete = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { name: string }) => ipc.handler_delete(args),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
      qc.removeQueries({ queryKey: ["handler_show", vars.name] });
    },
  });
};

/** Plan 7: rename a handler. Invalidates list + removes old show cache. */
export const useHandlerRename = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { old: string; new: string }) => ipc.handler_rename(args),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
      qc.removeQueries({ queryKey: ["handler_show", vars.old] });
    },
  });
};
