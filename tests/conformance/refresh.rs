//! The refresh (`mise run corpus:refresh`): moves the copy to a statifier-ex
//! tag by the sequence under "Vendoring the corpus" in the copy's
//! `RATCHET.md`, with `DEST` set to `conformance/statifier`. Every file under
//! `conformance/` at the tag is written through `git show` as git stores it,
//! with no edit of any kind; the copy's corpus hash is recomputed and
//! required to equal its manifest's; and the provenance record
//! `{repo, tag, sha, corpus_hash}` is written as one line beside the copy.
//! Nothing is written until the copy has hashed to its manifest's hash.
//!
//! It reads the tag from a clone of the reference (any git directory that
//! holds the tag, bare or not); the record names the public repository,
//! never the clone's path. A refresh is a reviewed change, never a silent
//! update: it refuses while the copy or the provenance record has
//! uncommitted changes, so it never overwrites work that is not in git. In
//! its check mode it stages the copy the same way and compares it, and the
//! record it would write, with what is on disk, byte for byte, and writes
//! nothing. No gate stage runs it.

use crate::corpus::{PROVENANCE_FILE, REFERENCE_REPO, VENDORED_DIR, corpus_hash, read_json};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a refresh does once the copy is staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Replace the copy and the record.
    Write,
    /// Compare them with the staged copy and write nothing.
    Check,
}

/// The provenance record's line, as the sequence's `printf` writes it.
pub fn record(tag: &str, sha: &str, corpus_hash: &str) -> String {
    format!(
        "{{\"repo\":{},\"tag\":{},\"sha\":{},\"corpus_hash\":{}}}\n",
        Value::from(REFERENCE_REPO),
        Value::from(tag),
        Value::from(sha),
        Value::from(corpus_hash)
    )
}

/// The output of git run in `dir`, as bytes; a sentence when git fails.
fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| format!("git cannot be run: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {} failed in {}: {}",
            args.first().copied().unwrap_or_default(),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The uncommitted changes git reports under `repo_root` to the copy and the
/// record, untracked files included, one porcelain line each.
pub fn dirty_paths(repo_root: &Path) -> Result<Vec<String>, String> {
    let copy = format!("conformance/{VENDORED_DIR}");
    let record = format!("conformance/{PROVENANCE_FILE}");
    let out = git(
        repo_root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            &copy,
            &record,
        ],
    )?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .map(str::to_owned)
        .collect())
}

/// A directory under the build's own temporary directory, removed when
/// dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    /// A fresh, empty directory named for `name`, this process and a count
    /// of the directories this process has made, so two tests running at
    /// once never share one. It is created with `create_dir`, which fails
    /// rather than reuse a directory that already exists.
    pub fn new(name: &str) -> Result<Scratch, String> {
        static MADE: AtomicU64 = AtomicU64::new(0);
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("{} cannot be made: {error}", base.display()))?;
        let count = MADE.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("{name}-{}-{count}", std::process::id()));
        std::fs::create_dir(&dir)
            .map_err(|error| format!("{} cannot be made: {error}", dir.display()))?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Moves (or, in [`Mode::Check`], compares) the copy under `repo_root`'s
/// `conformance/` to `tag`, read from the git directory `from`. Answers the
/// line the task prints; answers a refusal, having written nothing, when it
/// may not or cannot, and in check mode when the copy or the record differs.
pub fn run(tag: &str, from: &Path, repo_root: &Path, mode: Mode) -> Result<String, String> {
    let dirty = dirty_paths(repo_root)?;
    if !dirty.is_empty() {
        return Err(format!(
            "the copy or its record has uncommitted changes; commit or discard them first:\n{}",
            dirty.join("\n")
        ));
    }
    let sha = resolve_tag(from, tag)?;
    let scratch = Scratch::new("corpus-refresh")?;
    let (count, hash) = stage(from, &sha, tag, &scratch.0)?;
    let line = record(tag, &sha, &hash);
    let conformance = repo_root.join("conformance");
    match mode {
        Mode::Write => {
            install(&scratch.0, &conformance, &line)?;
            Ok(format!(
                "corpus refresh: {count} files from {tag} ({sha}) into conformance/{VENDORED_DIR}, {hash}"
            ))
        }
        Mode::Check => {
            let differences = compare(&scratch.0, &conformance, &line)?;
            if differences.is_empty() {
                Ok(format!(
                    "corpus refresh check: conformance/{VENDORED_DIR} is {tag} ({sha}) byte for byte, {count} files, and the record agrees"
                ))
            } else {
                Err(format!(
                    "the copy is not {tag} ({sha}):\n{}",
                    differences.join("\n")
                ))
            }
        }
    }
}

fn resolve_tag(from: &Path, tag: &str) -> Result<String, String> {
    let reference = format!("refs/tags/{tag}");
    if git(from, &["rev-parse", "-q", "--verify", &reference]).is_err() {
        return Err(format!(
            "{tag} is not a tag in {}; a tag is what is recorded, never a branch",
            from.display()
        ));
    }
    let sha = git(from, &["rev-parse", &format!("{reference}^{{commit}}")])?;
    Ok(String::from_utf8_lossy(&sha).trim().to_owned())
}

/// Writes the copy at `sha` into `scratch/statifier` and answers its file
/// count and corpus hash, refusing a copy that does not hash to its
/// manifest's.
fn stage(from: &Path, sha: &str, tag: &str, scratch: &Path) -> Result<(usize, String), String> {
    let listing = git(
        from,
        &[
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            sha,
            "--",
            "conformance/",
        ],
    )?;
    let paths: Vec<String> = listing
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    if paths.is_empty() {
        return Err(format!("there is no conformance/ at {tag} ({sha})"));
    }
    let copy = scratch.join(VENDORED_DIR);
    for path in &paths {
        let relative = path.strip_prefix("conformance/").unwrap_or(path);
        let out = copy.join(relative);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let bytes = git(from, &["show", &format!("{sha}:{path}")])?;
        std::fs::write(&out, bytes).map_err(|error| error.to_string())?;
    }
    let hash = corpus_hash(scratch);
    let wanted = read_json(&copy, "manifest.json")?
        .get("corpus_hash")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (hash, wanted) {
        (Some(hash), Some(wanted)) if hash == wanted => Ok((paths.len(), hash)),
        (hash, wanted) => Err(format!(
            "the copy at {tag} hashes to {}; its manifest says {}",
            hash.unwrap_or_else(|| "nothing (no corpus file)".to_owned()),
            wanted.unwrap_or_else(|| "no corpus_hash".to_owned())
        )),
    }
}

/// Every file under `dir`, by its path relative to `dir` with `/`
/// separators, with its bytes.
pub fn files(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut found = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let listing = std::fs::read_dir(&next)
            .map_err(|error| format!("{} cannot be read: {error}", next.display()))?;
        for entry in listing {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(dir)
                    .map_err(|error| error.to_string())?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
                found.insert(relative, bytes);
            }
        }
    }
    Ok(found)
}

/// Copies every file under `from` to the same path under `to`.
pub fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    for (relative, bytes) in files(from)? {
        let out = to.join(&relative);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&out, bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn install(scratch: &Path, conformance: &Path, line: &str) -> Result<(), String> {
    let dest = conformance.join(VENDORED_DIR);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|error| error.to_string())?;
    }
    copy_dir(&scratch.join(VENDORED_DIR), &dest)?;
    std::fs::write(conformance.join(PROVENANCE_FILE), line).map_err(|error| error.to_string())
}

/// Each way the copy and the record on disk differ from the staged copy and
/// the record a refresh would write, one sentence each.
fn compare(scratch: &Path, conformance: &Path, line: &str) -> Result<Vec<String>, String> {
    let staged = files(&scratch.join(VENDORED_DIR))?;
    let dest = conformance.join(VENDORED_DIR);
    let present = if dest.is_dir() {
        files(&dest)?
    } else {
        BTreeMap::new()
    };
    let mut differences = Vec::new();
    for (path, bytes) in &staged {
        match present.get(path) {
            None => differences.push(format!("missing: {VENDORED_DIR}/{path}")),
            Some(found) if found != bytes => {
                differences.push(format!("changed: {VENDORED_DIR}/{path}"));
            }
            Some(_) => {}
        }
    }
    for path in present.keys() {
        if !staged.contains_key(path) {
            differences.push(format!("not at the tag: {VENDORED_DIR}/{path}"));
        }
    }
    let on_disk = std::fs::read(conformance.join(PROVENANCE_FILE)).unwrap_or_default();
    if on_disk != line.as_bytes() {
        differences.push(format!(
            "changed: {PROVENANCE_FILE} is not the record the refresh writes"
        ));
    }
    Ok(differences)
}
