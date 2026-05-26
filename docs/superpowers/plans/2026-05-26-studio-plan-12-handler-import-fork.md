# Plan 12 — Handler import + fork Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.

**Goal:** Two new ways to create a handler: (a) import an existing folder from disk (must contain rowforge.yaml), (b) fork an existing workspace handler (auto-rewrites manifest.name).

**Architecture:** Shared `copy_dir_recursive` helper. Two new StudioCore methods + Tauri commands. ScaffoldDialog gains a 4th radio for "Import from folder"; HandlerDetailPage gains a Fork button + new ForkHandlerDialog.

**Design spec:** `docs/superpowers/specs/2026-05-26-studio-plan-12-handler-import-fork-design.md`

---

## Task 1: studio-core — copy_dir_recursive + handler_import_from_folder + handler_fork

**Files:**
- Modify: `crates/rowforge-studio-core/src/handler.rs` (or co-locate in lib.rs)
- Modify: `crates/rowforge-studio-core/src/lib.rs` (StudioCore methods)
- Test: `crates/rowforge-studio-core/tests/foundation.rs`

- [ ] **Step 1: Add `copy_dir_recursive` helper**

In `crates/rowforge-studio-core/src/handler.rs` (or wherever Plan 7's scaffold logic lives):

```rust
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).expect("walkdir invariant");
        let target = dst.join(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if ft.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &target)?;
        } else {
            tracing::warn!(path = ?path, "copy_dir_recursive: skipping non-regular entry");
        }
    }
    Ok(())
}
```

walkdir is already a workspace dep (Plan 10).

- [ ] **Step 2: Add `handler_import_from_folder`**

In `crates/rowforge-studio-core/src/lib.rs` (StudioCore impl):

```rust
pub fn handler_import_from_folder(
    &self,
    source_path: &Path,
    name: &str,
) -> Result<(), UiError> {
    // Validate name format (Plan 7 helper)
    if !crate::handler::validate_name(name) {
        return Err(UiError::InvalidHandlerName { name: name.to_string() });
    }

    // Validate source: must exist + be a directory + contain rowforge.yaml
    if !source_path.is_dir() {
        return Err(UiError::Io(format!("source path is not a directory: {}", source_path.display())));
    }
    if !source_path.join("rowforge.yaml").exists() {
        return Err(UiError::InvalidArg(
            "source folder must contain rowforge.yaml".to_string(),
        ));
    }

    // Validate target doesn't exist
    let target = self.workspace.root.as_path().join("handlers").join(name);
    if target.exists() {
        return Err(UiError::HandlerExists { name: name.to_string() });
    }

    // Copy
    crate::handler::copy_dir_recursive(source_path, &target)
        .map_err(|e| UiError::Io(format!("copy: {}", e)))?;

    Ok(())
}
```

> Adjust the module path / `pub(crate)` visibility of `copy_dir_recursive` as needed.
> If `UiError::InvalidArg(String)` doesn't exist, use the closest existing variant or add it (Plan 10 might have added it).

- [ ] **Step 3: Add `handler_fork`**

```rust
pub fn handler_fork(
    &self,
    source_name: &str,
    new_name: &str,
) -> Result<(), UiError> {
    if !crate::handler::validate_name(source_name) {
        return Err(UiError::InvalidHandlerName { name: source_name.to_string() });
    }
    if !crate::handler::validate_name(new_name) {
        return Err(UiError::InvalidHandlerName { name: new_name.to_string() });
    }
    if source_name == new_name {
        return Err(UiError::InvalidArg("new_name must differ from source".to_string()));
    }

    let handlers_dir = self.workspace.root.as_path().join("handlers");
    let source = handlers_dir.join(source_name);
    let target = handlers_dir.join(new_name);
    if !source.is_dir() {
        return Err(UiError::HandlerNotFound { name: source_name.to_string() });
    }
    if target.exists() {
        return Err(UiError::HandlerExists { name: new_name.to_string() });
    }

    crate::handler::copy_dir_recursive(&source, &target)
        .map_err(|e| UiError::Io(format!("copy: {}", e)))?;

    // Rewrite manifest.name in the new handler. Serde round-trip loses
    // comments + may reorder keys; documented in Plan 12 spec / HUMAN_SMOKE.
    let manifest_path = target.join("rowforge.yaml");
    if let Ok((mut manifest, _)) = rowforge_core::manifest::Manifest::load_from_dir(&target) {
        manifest.name = new_name.to_string();
        let yaml = serde_yaml::to_string(&manifest)
            .map_err(|e| UiError::Io(format!("serialize manifest: {}", e)))?;
        std::fs::write(&manifest_path, yaml)
            .map_err(|e| UiError::Io(format!("write manifest: {}", e)))?;
    }
    // If manifest load failed, leave as-is (user can fix later)

    Ok(())
}
```

> Verify the `Manifest::load_from_dir` signature (Plan 8 confirmed returns `(Manifest, PathBuf)`).
> `serde_yaml` should already be a workspace dep — verify.

- [ ] **Step 4: Tests**

Append to `crates/rowforge-studio-core/tests/foundation.rs`:

```rust
#[test]
fn copy_dir_recursive_copies_nested_structure() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    std::fs::write(src.path().join("a.txt"), b"a").unwrap();
    std::fs::create_dir(src.path().join("sub")).unwrap();
    std::fs::write(src.path().join("sub/b.txt"), b"b").unwrap();

    crate::handler::copy_dir_recursive(src.path(), &dst.path().join("out")).unwrap();

    assert_eq!(std::fs::read(dst.path().join("out/a.txt")).unwrap(), b"a");
    assert_eq!(std::fs::read(dst.path().join("out/sub/b.txt")).unwrap(), b"b");
}
// (Adjust visibility of copy_dir_recursive for the test to access it,
//  or test indirectly through handler_import_from_folder.)

#[test]
fn handler_import_from_folder_happy_path() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());

    // Build a source dir somewhere
    let source = TempDir::new().unwrap();
    std::fs::write(
        source.path().join("rowforge.yaml"),
        "name: original\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n",
    ).unwrap();
    std::fs::write(source.path().join("handler.go"), "package main\n").unwrap();

    core.handler_import_from_folder(source.path(), "imported").expect("ok");

    let target = tmp.path().join("handlers/imported");
    assert!(target.exists());
    assert!(target.join("rowforge.yaml").exists());
    assert!(target.join("handler.go").exists());
}

#[test]
fn handler_import_rejects_missing_rowforge_yaml() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let source = TempDir::new().unwrap();
    std::fs::write(source.path().join("just_code.go"), "package main").unwrap();

    let err = core.handler_import_from_folder(source.path(), "imported").unwrap_err();
    assert!(matches!(err, UiError::InvalidArg(_)));
}

#[test]
fn handler_import_rejects_existing_target() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    std::fs::create_dir_all(tmp.path().join("handlers/taken")).unwrap();
    let source = TempDir::new().unwrap();
    std::fs::write(source.path().join("rowforge.yaml"), "name: x\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n").unwrap();

    let err = core.handler_import_from_folder(source.path(), "taken").unwrap_err();
    assert!(matches!(err, UiError::HandlerExists { .. }));
}

#[test]
fn handler_fork_happy_path_rewrites_manifest_name() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    // Seed source handler
    let src_dir = tmp.path().join("handlers/source");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("rowforge.yaml"),
        "name: source\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n",
    ).unwrap();
    std::fs::write(src_dir.join("handler.go"), "package main\n").unwrap();

    core.handler_fork("source", "source-fork").expect("ok");

    let new_dir = tmp.path().join("handlers/source-fork");
    assert!(new_dir.exists());
    let manifest_text = std::fs::read_to_string(new_dir.join("rowforge.yaml")).unwrap();
    assert!(manifest_text.contains("source-fork"), "manifest should mention new name");
    assert!(!manifest_text.contains("name: source\n"), "old name should be gone");
}

#[test]
fn handler_fork_rejects_existing_target() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    std::fs::create_dir_all(tmp.path().join("handlers/source")).unwrap();
    std::fs::write(
        tmp.path().join("handlers/source/rowforge.yaml"),
        "name: source\nkind: row\nprimary_field: id\nentry:\n  cmd: [\"./bin\"]\n",
    ).unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers/taken")).unwrap();

    let err = core.handler_fork("source", "taken").unwrap_err();
    assert!(matches!(err, UiError::HandlerExists { .. }));
}

#[test]
fn handler_fork_rejects_missing_source() {
    let tmp = TempDir::new().unwrap();
    let core = open_test_workspace(tmp.path());
    let err = core.handler_fork("nonexistent", "new").unwrap_err();
    assert!(matches!(err, UiError::HandlerNotFound { .. }));
}
```

- [ ] **Step 5: Verify**

```bash
cargo build
cargo test -p rowforge-studio-core
```

Expected: +7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-studio-core/src/handler.rs crates/rowforge-studio-core/src/lib.rs crates/rowforge-studio-core/tests/foundation.rs
git commit -m "studio-core: handler_import_from_folder + handler_fork

Two new ways to create a handler:

- handler_import_from_folder(source_path, name) — copies a folder
  from anywhere on disk into <workspace>/handlers/<name>/. Requires
  source to contain rowforge.yaml (Plan 12 design: 'pure source
  folders should go through Scaffold + paste' workflow).

- handler_fork(source_name, new_name) — copies an existing workspace
  handler dir under a new name; rewrites the manifest's name field
  via serde round-trip (loses YAML comments — documented).

Both share a copy_dir_recursive walkdir helper. No filter — copies
everything including .git, node_modules, build outputs per Plan 12
spec §2 'user cleans up if unwanted'. Symlinks skipped with
tracing::warn.

+7 integration tests: copy nested; import happy + missing manifest +
target exists; fork happy with manifest rewrite + target exists +
missing source."
```

---

## Task 2: Tauri commands + ipc_contract

**Files:**
- Modify: `apps/rowforge-studio/src-tauri/src/commands.rs`
- Modify: `apps/rowforge-studio/src-tauri/src/lib.rs` (register)
- Modify: `apps/rowforge-studio/src-tauri/tests/ipc_contract.rs`

- [ ] **Step 1: Two commands**

```rust
#[tauri::command]
pub fn handler_import_from_folder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_path: String,
    name: String,
) -> Result<(), UiError> {
    let result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_import_from_folder(std::path::Path::new(&source_path), &name)
    };
    if result.is_ok() {
        use tauri::Emitter;
        let _ = app.emit("handlers:list", ());
    }
    result
}

#[tauri::command]
pub fn handler_fork(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_name: String,
    new_name: String,
) -> Result<(), UiError> {
    let result = {
        let guard = state.core.lock().unwrap_or_else(|p| p.into_inner());
        let core = guard.as_ref()
            .ok_or_else(|| UiError::WorkspaceLocked("no workspace open".into()))?;
        core.handler_fork(&source_name, &new_name)
    };
    if result.is_ok() {
        use tauri::Emitter;
        let _ = app.emit("handlers:list", ());
    }
    result
}
```

- [ ] **Step 2: Register**

Add to `generate_handler![...]`.

- [ ] **Step 3: ipc_contract tests**

```rust
#[test]
fn plan12_handler_import_from_folder_command_registered() {
    let _ = crate::commands::handler_import_from_folder;
}

#[test]
fn plan12_handler_fork_command_registered() {
    let _ = crate::commands::handler_fork;
}
```

- [ ] **Step 4: Verify + commit**

```bash
cargo build && cargo test -p rowforge-studio --test ipc_contract
```

```bash
git add apps/rowforge-studio/src-tauri/src/ apps/rowforge-studio/src-tauri/tests/ipc_contract.rs
git commit -m "studio-shell: handler_import_from_folder + handler_fork Tauri commands

Both sync (file operations are quick). Both emit handlers:list
event on success so HandlerList in any window invalidates.

ipc_contract +2."
```

---

## Task 3: TS mirrors + hooks

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/client.ts`
- Modify: `apps/rowforge-studio/src/ipc/use-handlers.ts` (Plan 7's hook file)

- [ ] **Step 1: ipc client**

```ts
handler_import_from_folder: (args: { sourcePath: string; name: string }) =>
  invoke<void>("handler_import_from_folder", args),
handler_fork: (args: { sourceName: string; newName: string }) =>
  invoke<void>("handler_fork", args),
```

camelCase per Plan 9 convention.

- [ ] **Step 2: Hooks**

```ts
export const useHandlerImportFromFolder = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { sourcePath: string; name: string }) =>
      ipc.handler_import_from_folder(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    },
  });
};

export const useHandlerFork = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { sourceName: string; newName: string }) =>
      ipc.handler_fork(args),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["handler_list"] });
    },
  });
};
```

- [ ] **Step 3: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
```

```bash
git add apps/rowforge-studio/src/ipc/
git commit -m "studio-shell: ipc wrappers + hooks for handler import + fork

ipc.handler_import_from_folder + ipc.handler_fork (camelCase args
per Plan 9 fix).

useHandlerImportFromFolder + useHandlerFork mutations; both
invalidate handler_list on success."
```

---

## Task 4: ScaffoldDialog — 4th radio "Import from folder"

**Files:**
- Modify: `apps/rowforge-studio/src/components/ScaffoldDialog.tsx`
- Modify or extend: `apps/rowforge-studio/src/components/__tests__/ScaffoldDialog.test.tsx`

- [ ] **Step 1: State additions**

In ScaffoldDialog:

```tsx
type Source = "template" | "folder";
const [source, setSource] = useState<Source>("template");
const [sourceFolder, setSourceFolder] = useState<string | null>(null);

const importMut = useHandlerImportFromFolder();
// existing scaffold mutation hook stays

const pickFolder = async () => {
  const path = await dialogOpen({ directory: true, multiple: false });
  if (typeof path === "string") setSourceFolder(path);
};
```

Import the folder picker from `@tauri-apps/plugin-dialog`:

```tsx
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
```

- [ ] **Step 2: Render the 4th radio**

Add to the existing template radio group:

```tsx
<label className="flex items-center gap-2 cursor-pointer">
  <input
    type="radio"
    checked={source === "folder"}
    onChange={() => setSource("folder")}
  />
  <div>
    <div className="font-medium">Import from folder…</div>
    <div className="text-xs text-muted-foreground">
      Copy an existing handler folder (must contain rowforge.yaml).
    </div>
  </div>
</label>
```

(The 3 existing template radios remain; this is a 4th option in the same group. When `source === "folder"` is true, none of `template` values are active — the existing template state is irrelevant.)

- [ ] **Step 3: Conditional UI for folder mode**

When `source === "folder"`:
- Hide the `primary_field` input (it's not used)
- Show the folder picker:

```tsx
{source === "folder" && (
  <Field label="Source folder" htmlFor="">
    <div className="flex items-center gap-2">
      <Button onClick={pickFolder} variant="outline" size="sm">
        {sourceFolder ? "Change…" : "Pick folder…"}
      </Button>
      {sourceFolder && (
        <code className="text-xs text-muted-foreground truncate flex-1">
          {sourceFolder}
        </code>
      )}
    </div>
    <div className="mt-1 text-xs text-muted-foreground">
      Must contain rowforge.yaml. Everything in the folder copies
      verbatim — including .git / node_modules if present.
    </div>
  </Field>
)}
```

Wrap the existing template + primary_field block in `{source === "template" && (...)}`.

- [ ] **Step 4: Submit handler**

```tsx
const handleSubmit = () => {
  if (source === "template") {
    scaffoldMut.mutate(
      { name, template, primaryField },
      { onSuccess: (createdName) => {
          toast.success(`Handler "${createdName}" created`);
          handleOpenChange(false);
          navigate(`/handlers/${createdName}`);
        } }
    );
  } else {
    if (!sourceFolder) return;
    importMut.mutate(
      { sourcePath: sourceFolder, name },
      { onSuccess: () => {
          toast.success(`Handler "${name}" imported`);
          handleOpenChange(false);
          navigate(`/handlers/${name}`);
        } }
    );
  }
};
```

`canSubmit` logic:
```tsx
const canSubmit = source === "template"
  ? name.length > 0 && nameValid && primaryField.length > 0 && !scaffoldMut.isPending
  : name.length > 0 && nameValid && !!sourceFolder && !importMut.isPending;
```

The error display (`scaffoldMut.error || importMut.error`) needs both — `uiErrorMessage(scaffoldMut.error ?? importMut.error)`.

- [ ] **Step 5: Tests**

Extend `ScaffoldDialog.test.tsx`. Mirror existing Plan 7 tests. 3+ new:

```tsx
it("Import from folder radio reveals folder picker + hides primary_field", () => { ... });

it("Create button disabled when folder source has no folder selected", () => { ... });

it("submitting Import from folder calls handler_import_from_folder ipc", async () => {
  // mock invoke + dialog open; render; click Import radio; click folder picker; assert path appears; click Create; assert invoke("handler_import_from_folder", { sourcePath, name }) called.
});
```

Mock `@tauri-apps/plugin-dialog`:
```ts
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
```

- [ ] **Step 6: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
```

```bash
git add apps/rowforge-studio/src/components/ScaffoldDialog.tsx apps/rowforge-studio/src/components/__tests__/ScaffoldDialog.test.tsx
git commit -m "studio-shell: ScaffoldDialog gains 'Import from folder' source

Plan 12 import flow lives in the same dialog as scaffold templates.
4th radio toggles UI:
- 'template' mode: existing 3-template chooser + primary_field input
- 'folder' mode: folder picker (Tauri dialog), primary_field hidden

Create button dispatches to handler_scaffold OR handler_import_from_folder
based on the selected source. Source folder required when in folder
mode; backend additionally validates the folder contains rowforge.yaml.

+3 vitest covering folder-mode UI + submit dispatch."
```

---

## Task 5: ForkHandlerDialog + HandlerDetailPage Fork button

**Files:**
- Create: `apps/rowforge-studio/src/components/ForkHandlerDialog.tsx`
- Create: `apps/rowforge-studio/src/components/__tests__/ForkHandlerDialog.test.tsx`
- Modify: `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`

- [ ] **Step 1: Build ForkHandlerDialog**

Mirror RenameHandlerDialog from Plan 7 — same regex validation pattern.

```tsx
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useHandlerFork } from "@/ipc/use-handlers";
import { uiErrorMessage } from "@/ipc/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sourceName: string;
}

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

export function ForkHandlerDialog({ open, onOpenChange, sourceName }: Props) {
  const navigate = useNavigate();
  const fork = useHandlerFork();
  const [newName, setNewName] = useState(`${sourceName}-fork`);

  useEffect(() => {
    if (open) {
      setNewName(`${sourceName}-fork`);
      fork.reset();
    }
  }, [open, sourceName]);

  const nameError =
    newName === "" ? "Name is required" :
    !NAME_RE.test(newName) ? "Lowercase letters, numbers, and hyphens; must start with a letter or number" :
    newName === sourceName ? "Name must differ from source" :
    null;
  const canSubmit = nameError === null && !fork.isPending;

  const handleSubmit = () => {
    if (!canSubmit) return;
    fork.mutate(
      { sourceName, newName },
      {
        onSuccess: () => {
          toast.success(`Handler forked to "${newName}"`);
          onOpenChange(false);
          navigate(`/handlers/${newName}`);
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Fork handler "{sourceName}"</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <label htmlFor="fork-name" className="mb-1 block text-sm font-medium">
              Name for the new handler
            </label>
            <Input
              id="fork-name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              autoFocus
            />
            {nameError && <div className="mt-1 text-xs text-red-300">{nameError}</div>}
          </div>

          <p className="text-xs text-muted-foreground">
            Copies all files from "{sourceName}" into a new handler dir,
            updating the manifest's name field to match.
          </p>
          <p className="text-xs text-yellow-300">
            ⚠ Comments in rowforge.yaml will not survive the fork (serde
            round-trip).
          </p>

          {fork.isError && (
            <div className="rounded border border-red-500/40 bg-red-500/10 p-2 text-sm text-red-200">
              {uiErrorMessage(fork.error)}
            </div>
          )}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button onClick={handleSubmit} disabled={!canSubmit}>
            {fork.isPending ? "Forking…" : "Fork"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 2: Tests for ForkHandlerDialog**

```tsx
describe("ForkHandlerDialog", () => {
  it("pre-fills name with <source>-fork", () => { ... });
  it("disables Fork when name unchanged from source", () => { ... });
  it("disables Fork for invalid name regex", () => { ... });
  it("happy path: clicks Fork → calls handler_fork ipc → toast + navigate", async () => { ... });
  it("HandlerExists error renders inline, dialog stays open", async () => { ... });
});
```

- [ ] **Step 3: HandlerDetailPage Fork button**

In `apps/rowforge-studio/src/pages/HandlerDetailPage.tsx`. Find the header action row (Open in editor / Reveal / Rename / Delete buttons from Plan 7).

Add:
```tsx
import { ForkHandlerDialog } from "@/components/ForkHandlerDialog";

// inside component
const [forkOpen, setForkOpen] = useState(false);

// in action row (between Rename and Delete):
<Button variant="outline" onClick={() => setForkOpen(true)}>
  Fork…
</Button>

// at the bottom of the JSX tree (with other dialogs):
<ForkHandlerDialog
  open={forkOpen}
  onOpenChange={setForkOpen}
  sourceName={data.summary.name}
/>
```

- [ ] **Step 4: Verify + commit**

```bash
cd apps/rowforge-studio
pnpm tsc -b
pnpm test
pnpm build
```

```bash
git add apps/rowforge-studio/src/components/ForkHandlerDialog.tsx apps/rowforge-studio/src/components/__tests__/ForkHandlerDialog.test.tsx apps/rowforge-studio/src/pages/HandlerDetailPage.tsx
git commit -m "studio-shell: ForkHandlerDialog + HandlerDetailPage Fork button

ForkHandlerDialog mirrors Plan 7's RenameHandlerDialog UX:
- Pre-fills new name with <source>-fork
- Same regex validation as scaffold/rename
- Disabled when unchanged from source / invalid / pending
- onSuccess: toast + navigate to /handlers/<new_name>

HandlerDetailPage header gets a Fork… button between Rename… and
Delete… buttons.

+5 vitest."
```

---

## Task 6: Spec docs + HUMAN_SMOKE Plan 12

**Files:**
- Modify: `docs/spec/studio/part-5-api.md` (en + zh-Hant) — 2 new commands
- Modify: `docs/spec/studio/part-8-handler-authoring.md` (en + zh-Hant) — §8.4.6 → "Scaffold sources" + new "Handler fork"
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md` — Plan 12 section

- [ ] **Step 1: part-5**

Add 2 new commands: `handler_import_from_folder(source_path, name)`, `handler_fork(source_name, new_name)`. Both emit `handlers:list` event.

- [ ] **Step 2: part-8**

§8.4.6 (Scaffold templates): rename to "Scaffold sources". Document:
- Templates (existing 3)
- Import from folder (new): copy verbatim from any local path; must contain rowforge.yaml; no filter; symlinks skipped

New §8.4.x "Handler fork":
- Same-name in manifest auto-rewritten via serde round-trip
- Loses YAML comments / may reorder keys (documented limitation)
- Source unchanged

- [ ] **Step 3: HUMAN_SMOKE Plan 12**

Append after Plan 11 section. ~15 steps:

#### Import from folder (1-6)
1. Have a folder on disk with rowforge.yaml + some source files (e.g. examples/handlers/golang-stats-refund-records/)
2. Studio → /handlers → New Handler
3. Select "Import from folder" radio
4. Click Pick folder → OS dialog → select the folder
5. Enter name, click Create
6. Imported handler appears in list; navigate to it; rowforge.yaml + source files all present

#### Import edge cases (7-9)
7. Folder without rowforge.yaml → backend rejects with friendly error
8. Name collision → HandlerExists error rendered
9. .git folder in source → copies through verbatim (verify `<workspace>/handlers/<name>/.git/` exists)

#### Fork (10-13)
10. Navigate to an existing handler's detail page
11. Click Fork… → dialog pre-fills `<source>-fork`
12. Confirm → navigate to new handler's detail
13. Verify new handler's rowforge.yaml has updated `name:` field

#### Fork edge cases (14-15)
14. Fork to existing name → HandlerExists rendered
15. YAML comments in source manifest do NOT survive the fork (documented limitation; verify by adding a `# comment` to source's rowforge.yaml and confirming it's gone in fork's)

#### Known limitations
- Copy filter is none — .git / node_modules / build outputs come along
- Fork loses YAML comments + may reorder keys
- No cross-workspace import (only single-process / OS-dialog source)

- [ ] **Step 4: zh-Hant mirror**

Translation conventions:
- "import from folder" → 「從資料夾匯入」
- "fork" → 「Fork」(keep English; technical term)
- "manifest name field" → 「manifest 的 name 欄位」
- "comments" → 「註解」

- [ ] **Step 5: Verify diff + commit**

```bash
git diff --stat docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
```

Expected: 5 files modified (2 en + 2 zh-Hant + 1 HUMAN_SMOKE).

```bash
git add docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "docs: Plan 12 spec sync (en + zh-Hant) + HUMAN_SMOKE Plan 12

part-5: 2 new commands (handler_import_from_folder, handler_fork)
in §5.5; both emit handlers:list event.

part-8: §8.4.6 renamed 'Scaffold sources' — templates + import
from folder; new 'Handler fork' subsection documents serde
round-trip comment loss.

zh-Hant mirrored.

HUMAN_SMOKE Plan 12: 15 numbered steps covering import happy +
edge cases (missing manifest / target exists / .git carried);
fork happy + edge cases (existing name / comment loss)."
```

---

## Final verification + PR

```bash
cargo build && cargo test
cd apps/rowforge-studio && pnpm tsc -b && pnpm test && pnpm build
```

Expected:
- cargo: 398 → ~405 (+7)
- vitest: 158 → ~166 (+8: 3 ScaffoldDialog + 5 ForkHandlerDialog)

PR body:
```
## Summary

Two new ways to create a handler:
- Import from folder — pick any local folder containing rowforge.yaml,
  Studio snapshot-copies it into the workspace
- Fork — duplicate an existing handler under a new name; manifest name
  rewritten via serde round-trip

ScaffoldDialog gets a 4th 'Import from folder' radio. HandlerDetailPage
gets a Fork… button next to Rename / Delete.

## Test plan

- [x] cargo + vitest suites green
- [ ] Manual smoke per HUMAN_SMOKE Plan 12 (15 steps)
```

---

## Order dependency

T1 → T2 (Tauri needs T1) → T3 (TS needs T2) → T4 (ScaffoldDialog needs T3) → T5 (ForkDialog needs T3) → T6 (docs).

T4 and T5 are parallelizable (independent UI components), but use the same hooks file (T3). Run sequentially for simplicity.
