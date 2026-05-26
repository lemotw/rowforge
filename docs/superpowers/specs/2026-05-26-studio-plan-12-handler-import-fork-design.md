# Plan 12 — Handler import from folder + fork

**Date:** 2026-05-26
**Branch:** `studio-plan-12-handler-import-fork`
**Builds on:** Plans 7-11

## 1. Purpose

Two new ways to create a handler beyond the existing template scaffold (Plan 7):

1. **Import from folder** — pick any folder on disk that already contains a `rowforge.yaml`; Studio snapshot-copies it into `<workspace>/handlers/<name>/`. Use cases: handler written outside Studio, cloned from a repo, tutorial download.

2. **Fork** — duplicate an existing workspace handler under a new name. Studio copies the source dir and rewrites the manifest's `name:` field. Use cases: variant for different field, A/B experiment, safe baseline before customization.

Both operations are essentially "copy a directory tree into the workspace handlers/", differing only in source (external path vs existing handler) and post-copy mutations (none vs manifest name rewrite).

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Copy filter | None — copy everything including .git / node_modules / build outputs | User-stated. Conservative; user cleans up if unwanted. Simpler implementation. |
| Fork manifest.name | Auto-rewrite to new handler name | User-stated. Prevents two handlers reporting the same manifest name. |
| Import source validation | Require `rowforge.yaml` in source dir | User-stated. Pure source folders without manifest should go through scaffold + paste workflow. |
| Import UI | Extend ScaffoldDialog with 4th option "Import from folder" | Cohesive — same "create new handler" flow with different source |
| Fork UI | New ForkHandlerDialog on HandlerDetailPage header | Discoverable in context where source handler is visible |
| Copy implementation | `walkdir` recursive walk + `std::fs::copy` per file | Simple; preserves directory structure; no symlink magic |
| Symlinks | Skip silently with tracing::warn | Following them can pull in unrelated files; preserving them risks broken cross-workspace references |

## 3. Backend changes

### 3.1 Shared helper: `copy_dir_recursive`

In `crates/rowforge-studio-core/src/handler.rs` (or a `util` module):

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
            // symlinks, devices, sockets, etc. → skip with warning
            tracing::warn!(path = ?path, "copy_dir_recursive: skipping non-regular entry");
        }
    }
    Ok(())
}
```

Plan 10 added walkdir as a workspace dep; reuse.

### 3.2 New API: `handler_import_from_folder`

```rust
impl StudioCore {
    pub fn handler_import_from_folder(
        &self,
        source_path: &Path,
        name: &str,
    ) -> Result<(), UiError>;
}
```

Algorithm:
1. Validate `name` via existing `validate_name` (lowercase + digits + hyphens, non-leading hyphen)
2. Resolve target: `<workspace>/handlers/<name>/`
3. Refuse if target exists → `UiError::HandlerExists { name }`
4. Validate source: must be a directory, must contain `rowforge.yaml` at top level. If not → `UiError::InvalidArg("source folder must contain rowforge.yaml")`
5. Call `copy_dir_recursive(source, target)`
6. Map any io error → `UiError::Io`

Notes:
- Source path may be ANYWHERE on disk (user-picked via OS dialog). Not workspace-restricted. We trust the user; they're explicitly importing from this path.
- Source `rowforge.yaml` doesn't need to be valid YAML — we just check for its presence. Existing Plan 7 manifest validation will surface any issues when the user navigates to the imported handler's detail page.

### 3.3 New API: `handler_fork`

```rust
impl StudioCore {
    pub fn handler_fork(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), UiError>;
}
```

Algorithm:
1. Validate both names via `validate_name`
2. Refuse `source_name == new_name` → `UiError::InvalidArg("new_name must differ from source")`
3. Resolve `source_dir = <workspace>/handlers/<source_name>/`. Must exist → `UiError::HandlerNotFound { name: source_name }`
4. Resolve `target_dir = <workspace>/handlers/<new_name>/`. Must not exist → `UiError::HandlerExists { name: new_name }`
5. `copy_dir_recursive(source_dir, target_dir)`
6. Open `<target_dir>/rowforge.yaml`, parse via `rowforge_core::manifest::Manifest::load_from_dir(target_dir)`. If load fails, leave the file as-is (graceful — user may want to keep manifest as-is even if it has issues).
7. If load succeeds: mutate `manifest.name = new_name.to_string()`, serialize back via `serde_yaml::to_string(&manifest)`, write to `<target_dir>/rowforge.yaml`.
8. **Tradeoff**: serde round-trip loses YAML comments and may reorder keys. For v1, accept this. Document in HUMAN_SMOKE that comments in the manifest are lost on fork.

Alternative considered: line-by-line text replacement of the first `name:` field. More fragile (false positives if multiple name lines, indentation variations) but preserves comments. Not chosen.

## 4. Tauri shell

Two new commands:

```rust
#[tauri::command]
pub fn handler_import_from_folder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_path: String,
    name: String,
) -> Result<(), UiError>;

#[tauri::command]
pub fn handler_fork(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    source_name: String,
    new_name: String,
) -> Result<(), UiError>;
```

Both sync. Both emit `handlers:list` event on success (so ExecList / HandlerList in any window refresh).

## 5. React UI

### 5.1 ScaffoldDialog extension

Existing template radio:
- `( ) Go (row mode)`
- `( ) Go (batch mode)`
- `( ) Empty`

Add 4th:
- `( ) Import from folder…`

When "Import from folder" is selected:
- Hide `primary_field` input (irrelevant — source has its own manifest)
- Show a `[Pick folder…]` button. Click → Tauri dialog open dir. Selected path renders below the button.
- Create button enabled when: name valid + folder selected
- On Create → `handler_import_from_folder({ sourcePath, name })` instead of `handler_scaffold`

UI state additions:
```tsx
const [source, setSource] = useState<"template" | "folder">("template");
const [sourceFolder, setSourceFolder] = useState<string | null>(null);
```

The `template` and `primary_field` fields only render when `source === "template"`.

### 5.2 ForkHandlerDialog (new component)

`apps/rowforge-studio/src/components/ForkHandlerDialog.tsx`:

```
┌─ Fork handler "alpha" ─────────────────────────────┐
│ Name for the new handler:                          │
│ [ alpha-fork                              ]       │
│                                                    │
│ The new handler will be a complete copy of         │
│ "alpha" with its manifest name updated. The         │
│ original handler is unchanged.                     │
│                                                    │
│ ⚠ Comments in rowforge.yaml will not survive       │
│ the fork (serde round-trip).                       │
│                                                    │
│         [Cancel]    [Fork]                         │
└────────────────────────────────────────────────────┘
```

Pre-fill new name with `<source>-fork`. Validate via the same regex as scaffold. Disabled when invalid / unchanged / pending.

### 5.3 HandlerDetailPage Fork button

Header action row (between Rename… and Delete…):

```tsx
<Button variant="outline" onClick={() => setForkOpen(true)}>
  Fork…
</Button>
```

On fork success → toast + navigate to `/handlers/<new_name>`.

### 5.4 Hooks

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

## 6. Out of scope (explicit)

- Selective file inclusion / exclusion patterns (always all-or-nothing)
- Source git history preservation (we copy .git verbatim; that IS the history)
- Cross-workspace handler import (UI doesn't expose this; user must use OS file copy)
- Fork chain visualization (no parent-child relationship stored)
- Manifest name validation against existing handler names before fork (backend's target-exists check is the gate; the manifest name and the handler dir name can technically diverge after manual edits)
- Smart manifest merge (e.g. comments preservation via yaml-rust or libyaml). Stick with serde round-trip.

## 7. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-studio-core | ~7 | copy_dir_recursive happy + nested + symlink-skip; handler_import_from_folder happy + missing-yaml-rejected + target-exists; handler_fork happy + manifest-name-rewritten + target-exists |
| studio-shell ipc_contract | ~2 | command registration + JSON shape |
| vitest | ~5 | ScaffoldDialog "Import from folder" mode; folder pick disabled until folder selected; ForkHandlerDialog name validation; default name is `<source>-fork`; mutation calls correct ipc |

Targets:
- cargo: 398 → ~405 (+7)
- vitest: 158 → ~163 (+5)

## 8. Spec doc updates

- `docs/spec/studio/part-5-api.md`: 2 new commands (handler_import_from_folder, handler_fork)
- `docs/spec/studio/part-8-handler-authoring.md`: §8.4.6 scaffold templates section expands to "Scaffold sources" covering templates + folder import; new "Handler fork" subsection
- Mirror in zh-Hant
- HUMAN_SMOKE Plan 12: 15-20 steps covering import happy, import no-manifest rejected, import target-exists, fork happy, fork manifest name updated, fork target-exists, fork source-missing, comment-loss caveat

## 9. Acceptance criteria

1. `cargo build && cargo test` clean
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. ScaffoldDialog has 4th option "Import from folder…"
4. Selecting it hides primary_field + shows folder picker
5. Picking a folder without rowforge.yaml → backend rejects, UI shows friendly error
6. Picking a valid handler folder → import succeeds, navigates to new handler
7. HandlerDetailPage shows Fork… button next to Rename / Delete
8. Fork dialog pre-fills name with `<source>-fork`
9. Fork succeeds → navigates to new handler; rowforge.yaml in new handler has updated name field
10. Fork to existing name → UiError::HandlerExists rendered
11. HUMAN_SMOKE Plan 12 walkthrough added
12. Spec docs (part-5 + part-8 en + zh-Hant) updated

## 10. Open questions

None at design time.
