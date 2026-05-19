use anyhow::{anyhow, Context};
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct PackArgs {
    /// Target triple alias: darwin-arm64 | linux-x86_64 | linux-aarch64
    #[arg(long)]
    pub target: String,

    /// Handler directory to include. Repeatable.
    #[arg(long = "handler")]
    pub handlers: Vec<PathBuf>,

    /// Output zip path.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

pub async fn run(args: PackArgs) -> anyhow::Result<i32> {
    let target = parse_target(&args.target)?;
    if args.handlers.is_empty() {
        return Err(anyhow!("at least one --handler is required"));
    }
    eprintln!("[rowforge pack] target: {} ({})", args.target, target.triple);
    eprintln!("[rowforge pack] handlers: {} dirs", args.handlers.len());
    eprintln!("[rowforge pack] output: {}", args.output.display());

    let binary = cross_compile_binary(&target)?;
    assemble_bundle(&binary, &args.handlers, &target, &args.output)?;

    eprintln!("[rowforge pack] wrote {}", args.output.display());
    Ok(0)
}

#[derive(Debug, Clone)]
pub struct Target {
    /// Friendly name shown to users (e.g. "linux-x86_64").
    pub alias: &'static str,
    /// Rust target triple (e.g. "x86_64-unknown-linux-gnu").
    pub triple: &'static str,
    /// File suffix for the binary (".exe" on Windows; "" elsewhere).
    pub bin_suffix: &'static str,
}

pub fn parse_target(alias: &str) -> anyhow::Result<Target> {
    Ok(match alias {
        "darwin-arm64" => Target {
            alias: "darwin-arm64",
            triple: "aarch64-apple-darwin",
            bin_suffix: "",
        },
        "linux-x86_64" => Target {
            alias: "linux-x86_64",
            triple: "x86_64-unknown-linux-gnu",
            bin_suffix: "",
        },
        "linux-aarch64" => Target {
            alias: "linux-aarch64",
            triple: "aarch64-unknown-linux-gnu",
            bin_suffix: "",
        },
        other => return Err(anyhow!(
            "unknown --target '{}'. Supported: darwin-arm64, linux-x86_64, linux-aarch64",
            other
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_known() {
        assert_eq!(parse_target("linux-x86_64").unwrap().triple, "x86_64-unknown-linux-gnu");
        assert_eq!(parse_target("darwin-arm64").unwrap().triple, "aarch64-apple-darwin");
        assert_eq!(parse_target("linux-aarch64").unwrap().triple, "aarch64-unknown-linux-gnu");
    }

    #[test]
    fn parse_target_unknown() {
        let err = parse_target("freebsd-x86_64").unwrap_err();
        assert!(format!("{}", err).contains("unknown --target"));
    }
}

use rowforge_core::manifest::Manifest;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

/// Stage handlers + binary into a temp dir, generate README, write zip.
pub fn assemble_bundle(
    rowforge_binary: &Path,
    handler_dirs: &[PathBuf],
    target: &Target,
    output_zip: &Path,
) -> anyhow::Result<()> {
    let staging = tempfile::tempdir().context("create staging dir")?;
    let staging_root = staging.path();

    // 1. Copy rowforge binary at the root.
    let bin_dest = staging_root.join(format!("rowforge{}", target.bin_suffix));
    fs::copy(rowforge_binary, &bin_dest)
        .with_context(|| format!("copy {} -> {}", rowforge_binary.display(), bin_dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bin_dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin_dest, perms)?;
    }

    // 2. Copy each handler into staging/handlers/<name>/.
    let handlers_root = staging_root.join("handlers");
    fs::create_dir_all(&handlers_root)?;
    let mut runtimes: BTreeSet<String> = BTreeSet::new();
    let mut handler_summaries = Vec::with_capacity(handler_dirs.len());
    for src in handler_dirs {
        let (manifest, _) = Manifest::load_from_dir(src)
            .with_context(|| format!("read manifest in {}", src.display()))?;
        let dest = handlers_root.join(&manifest.name);
        copy_dir_recursive(src, &dest)
            .with_context(|| format!("copy handler {} -> {}", src.display(), dest.display()))?;
        if !manifest.language.is_empty() {
            runtimes.insert(manifest.language.clone());
        }
        handler_summaries.push((manifest.name.clone(), manifest.version.clone(),
                                 manifest.description.clone(), manifest.language.clone()));
    }

    // 3. Generate README.
    let readme = render_readme(target, &handler_summaries, &runtimes);
    fs::write(staging_root.join("README.md"), readme)?;

    // 4. Zip it.
    write_zip(staging_root, output_zip)?;

    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            fs::copy(&from, &to)?;
        }
        // skip symlinks for v0.2 simplicity
    }
    Ok(())
}

fn render_readme(
    target: &Target,
    handlers: &[(String, String, String, String)],
    runtimes: &BTreeSet<String>,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# rowforge bundle ({})\n\n", target.alias));
    s.push_str("Unzip and run:\n\n```\n./rowforge run --handler handlers/<name> --input data.csv --output-dir ./out\n```\n\n");
    s.push_str("## Handlers\n\n");
    for (name, version, desc, lang) in handlers {
        s.push_str(&format!("- **{} {}** ({}) — {}\n", name, version,
            if lang.is_empty() { "n/a" } else { lang.as_str() },
            if desc.is_empty() { "(no description)" } else { desc.as_str() }));
    }
    if !runtimes.is_empty() {
        s.push_str("\n## Recipient runtime requirements\n\n");
        s.push_str("Install these on the target machine before running handlers:\n\n");
        for r in runtimes {
            s.push_str(&format!("- {}\n", r));
        }
    }
    s
}

fn write_zip(staging: &Path, output: &Path) -> anyhow::Result<()> {
    use zip::write::SimpleFileOptions;
    let f = File::create(output)
        .with_context(|| format!("create {}", output.display()))?;
    let mut zip = zip::ZipWriter::new(f);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let exec_opts = opts.unix_permissions(0o755);

    walk_into_zip(staging, staging, &mut zip, &opts, &exec_opts)?;
    zip.finish()?;
    Ok(())
}

fn walk_into_zip<W: Write + std::io::Seek>(
    staging_root: &Path,
    cur: &Path,
    zip: &mut zip::ZipWriter<W>,
    opts: &zip::write::SimpleFileOptions,
    exec_opts: &zip::write::SimpleFileOptions,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(cur)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(staging_root).unwrap();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if entry.file_type()?.is_dir() {
            zip.add_directory(rel_str, *opts)?;
            walk_into_zip(staging_root, &path, zip, opts, exec_opts)?;
        } else {
            // Top-level file named "rowforge" or "rowforge.exe" gets exec perms.
            let is_root_binary = rel.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(false)
                && (rel.file_name().map(|n| n == "rowforge" || n == "rowforge.exe").unwrap_or(false));
            let chosen = if is_root_binary { *exec_opts } else { *opts };
            zip.start_file(rel_str, chosen)?;
            let mut content = Vec::new();
            File::open(&path)?.read_to_end(&mut content)?;
            zip.write_all(&content)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod assembly_tests {
    use super::*;

    fn make_handler(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let yaml = format!(
            "name: {n}\nversion: 0.0.1\ndescription: \"test\"\nlanguage: python\nentry:\n  cmd: ['python3', 'h.py']\nschema:\n  input: {{}}\n  output: {{}}\n",
            n = name);
        fs::write(dir.join("rowforge.yaml"), yaml).unwrap();
        fs::write(dir.join("h.py"), "# stub\n").unwrap();
        dir
    }

    #[test]
    fn assemble_produces_zip_with_binary_and_handlers() {
        let scratch = tempfile::tempdir().unwrap();
        // Fake "binary" — just any non-empty file.
        let fake_bin = scratch.path().join("fake-rowforge");
        fs::write(&fake_bin, b"#!/bin/sh\necho hi\n").unwrap();

        let h1 = make_handler(scratch.path(), "h1");
        let h2 = make_handler(scratch.path(), "h2");
        let out_zip = scratch.path().join("out.zip");
        let target = parse_target("linux-x86_64").unwrap();

        assemble_bundle(&fake_bin, &[h1, h2], &target, &out_zip).unwrap();

        assert!(out_zip.exists());
        // Re-open and inspect.
        let f = File::open(&out_zip).unwrap();
        let mut zr = zip::ZipArchive::new(f).unwrap();
        let names: BTreeSet<String> = (0..zr.len())
            .map(|i| zr.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains("rowforge"), "expected rowforge in zip, got {:?}", names);
        assert!(names.iter().any(|n| n.starts_with("handlers/h1/")));
        assert!(names.iter().any(|n| n.starts_with("handlers/h2/")));
        assert!(names.contains("README.md"));
    }

    #[test]
    fn readme_lists_runtimes() {
        let summaries = vec![
            ("h1".into(), "0.0.1".into(), "first handler".into(), "python".into()),
            ("h2".into(), "0.0.1".into(), "second handler".into(), "go".into()),
        ];
        let mut runtimes = BTreeSet::new();
        runtimes.insert("python".into());
        runtimes.insert("go".into());
        let target = parse_target("linux-x86_64").unwrap();
        let r = render_readme(&target, &summaries, &runtimes);
        assert!(r.contains("python"));
        assert!(r.contains("go"));
        assert!(r.contains("h1 0.0.1"));
        assert!(r.contains("linux-x86_64"));
    }
}

use std::process::Command;

/// Run `cargo zigbuild --release --target <triple>` from the workspace root and
/// return the produced binary's path.
fn cross_compile_binary(target: &Target) -> anyhow::Result<PathBuf> {
    // Probe for cargo-zigbuild.
    let probe = Command::new("cargo").args(["zigbuild", "--version"]).output();
    match probe {
        Ok(o) if o.status.success() => {}
        _ => return Err(anyhow!(
            "cargo-zigbuild not found.\n\
             Install:  brew install zig && cargo install cargo-zigbuild\n\
             More:     https://github.com/rust-cross/cargo-zigbuild"
        )),
    }

    let workspace_root = locate_workspace_root()?;
    eprintln!("[rowforge pack] cross-compiling rowforge for {} via cargo-zigbuild...", target.triple);
    let status = Command::new("cargo")
        .current_dir(&workspace_root)
        .args(["zigbuild", "--release", "--target", target.triple, "-p", "rowforge-cli"])
        .status()
        .with_context(|| "spawn cargo zigbuild")?;
    if !status.success() {
        return Err(anyhow!("cargo zigbuild --target {} failed (exit {:?})", target.triple, status.code()));
    }

    let out = workspace_root
        .join("target")
        .join(target.triple)
        .join("release")
        .join(format!("rowforge{}", target.bin_suffix));
    if !out.exists() {
        return Err(anyhow!("expected binary at {} not found after build", out.display()));
    }
    Ok(out)
}

/// Walk up from the rowforge-cli crate's CARGO_MANIFEST_DIR to find the workspace root.
fn locate_workspace_root() -> anyhow::Result<PathBuf> {
    // CARGO_MANIFEST_DIR is set at compile time; we walk relative to the binary's location at runtime.
    // Strategy: start from the running binary's dir, walk up looking for Cargo.toml with [workspace].
    let exe = std::env::current_exe().context("locate self exe")?;
    let mut cur = exe.parent().unwrap().to_path_buf();
    loop {
        let candidate = cur.join("Cargo.toml");
        if candidate.exists() {
            let s = std::fs::read_to_string(&candidate).unwrap_or_default();
            if s.contains("[workspace]") {
                return Ok(cur);
            }
        }
        if !cur.pop() {
            return Err(anyhow!("could not locate workspace root from {}", exe.display()));
        }
    }
}
