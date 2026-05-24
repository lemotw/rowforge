import { useMutation } from "@tanstack/react-query";
import { ipc } from "./client";
import type { ManifestSource } from "./types";

/**
 * Plan 5 T11: mutation hook for manifest_validate.
 * Used by NewExecutionWizard step 2 to gate the Submit button.
 */
export const useManifestValidate = () =>
  useMutation({
    mutationFn: (source: ManifestSource) => ipc.manifest_validate(source),
  });
