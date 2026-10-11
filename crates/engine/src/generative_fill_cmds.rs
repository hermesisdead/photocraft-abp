//! AI Generative Fill: a browser for inpainting models on the Hugging Face Hub, on-demand
//! downloads into the user's cache, and the `ai.generativeFill` command that the Generative Fill
//! tool's pop-up runs.
//!
//! * `ai.generativeFill.search` `{"query": text, "limit": n}`: models matching `query`.
//! * `ai.generativeFill.installed` `{}`: models already downloaded.
//! * `ai.generativeFill.download` `{"repo": "owner/name"}`: fetches the model's `.onnx` files, checks
//!   each against the SHA-256 the Hub lists for it, and only then installs it.
//! * `ai.generativeFill.remove` `{"repo": "owner/name"}`: deletes an installed model.
//! * `ai.generativeFill` (the pop-up): validates prompt and sampling parameters and needs a
//!   selection on a pixel layer. The inpainting backend is not part of this build yet, so a valid
//!   request ends with an explicit error rather than a fake result.
//!
//! Downloads never touch the document, so they are not journaled. Every input is validated; a bad
//! value is an error, never a panic.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const SEARCH: &str = "ai.generativeFill.search";
const INSTALLED: &str = "ai.generativeFill.installed";
const DOWNLOAD: &str = "ai.generativeFill.download";
const REMOVE: &str = "ai.generativeFill.remove";
const FILL: &str = "ai.generativeFill";

/// Largest total download accepted for one model.
pub const MAX_MODEL_BYTES: u64 = 4 << 30;
const MAX_PROMPT_CHARS: usize = 1000;
const HUB: &str = "https://huggingface.co";
const MANIFEST: &str = "manifest.json";
const SELECTED: &str = "selected.txt";
const SELECT: &str = "ai.generativeFill.select";
const LOCAL: &str = "ai.generativeFill.localModels";
/// How deep under the models folder `.onnx` files are looked for, and how many are listed.
const SCAN_DEPTH: usize = 3;
const SCAN_LIMIT: usize = 500;

/// The model offered as the one-click default: LaMa, a small (about 208 MB) Apache-2.0 inpainting
/// network. Its size and SHA-256 come from the hub's own file listing when it is downloaded.
pub const DEFAULT_MODEL_REPO: &str = "Carve/LaMa-ONNX";
pub const DEFAULT_MODEL_FILE: &str = "lama_fp32.onnx";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

// ----------------------------------------------------------------------------- Hub data

/// One search hit, as the model browser lists it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HubModel {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    pub pipeline_tag: Option<String>,
}

#[derive(Deserialize)]
struct SearchEntry {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

/// Parses the Hub's model-search response (a JSON array). Entries without a usable id are skipped.
pub fn parse_search(body: &str) -> Result<Vec<HubModel>> {
    let entries: Vec<SearchEntry> = serde_json::from_str(body).map_err(|e| EngineError::Other(format!("unexpected search response from the model hub: {e}")))?;
    Ok(entries
        .into_iter()
        .filter(|e| safe_repo(&e.id).is_ok())
        .map(|e| HubModel { id: e.id, downloads: e.downloads, likes: e.likes, pipeline_tag: e.pipeline_tag })
        .collect())
}

#[derive(Deserialize)]
struct RepoEntry {
    #[serde(default)]
    siblings: Vec<Sibling>,
}

#[derive(Deserialize)]
struct Sibling {
    rfilename: String,
    #[serde(default)]
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    sha256: String,
    size: u64,
}

/// One file of a model to download, with the checksum the Hub lists for it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The `.onnx` files of a model repository, from its `?blobs=true` metadata. A file without a
/// SHA-256 on record is not accepted, since it could not be verified.
pub fn parse_repo_files(body: &str) -> Result<Vec<ModelFile>> {
    let repo: RepoEntry = serde_json::from_str(body).map_err(|e| EngineError::Other(format!("unexpected model metadata from the hub: {e}")))?;
    let mut files = Vec::new();
    for s in repo.siblings {
        if !s.rfilename.to_ascii_lowercase().ends_with(".onnx") {
            continue;
        }
        safe_relative_path(&s.rfilename)?;
        let lfs = s.lfs.ok_or_else(|| EngineError::Other(format!("`{}` has no checksum on the hub; it cannot be verified", s.rfilename)))?;
        if lfs.sha256.len() != 64 || !lfs.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(EngineError::Other(format!("`{}` has a malformed checksum on the hub", s.rfilename)));
        }
        files.push(ModelFile { name: s.rfilename, bytes: lfs.size, sha256: lfs.sha256.to_ascii_lowercase() });
    }
    if files.is_empty() {
        return Err(EngineError::Other("this model has no .onnx weights to download".into()));
    }
    let total: u64 = files.iter().map(|f| f.bytes).sum();
    if total > MAX_MODEL_BYTES {
        return Err(EngineError::Other(format!("this model is {total} bytes, over the {MAX_MODEL_BYTES}-byte limit")));
    }
    Ok(files)
}

// ----------------------------------------------------------------------------- validation

/// `owner/name` with safe characters only: it becomes a folder name and a URL.
pub fn safe_repo(repo: &str) -> Result<&str> {
    let ok_part = |p: &str| !p.is_empty() && p.len() <= 96 && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), None) if ok_part(a) && ok_part(b) => Ok(repo),
        _ => Err(EngineError::Other(format!("`{repo}` is not a model id of the form owner/name"))),
    }
}

/// A relative path inside a model folder: no absolute prefix, no `..`, no backslashes.
pub fn safe_relative_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('\\') || name.len() > 256 {
        return Err(EngineError::Other(format!("unsafe file name `{name}`")));
    }
    let mut out = PathBuf::new();
    for c in Path::new(name).components() {
        match c {
            Component::Normal(part) => out.push(part),
            _ => return Err(EngineError::Other(format!("unsafe file name `{name}`"))),
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------------------- storage

/// Where downloaded models live: `<cache>/photocraft/generative-fill`, one folder per model.
pub fn models_root() -> Result<PathBuf> {
    let base = if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(x)
    } else if cfg!(target_os = "macos") {
        home()?.join("Library").join("Caches")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).ok_or_else(|| EngineError::Other("LOCALAPPDATA is not set".into()))?
    } else {
        home()?.join(".cache")
    };
    Ok(base.join("photocraft").join("generative-fill"))
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from).ok_or_else(|| EngineError::Other("HOME is not set".into()))
}

/// `path` with `suffix` appended to its file name (not replacing an extension: model folders can
/// contain dots, as in `owner__name.v1`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

fn model_dir(root: &Path, repo: &str) -> Result<PathBuf> {
    Ok(root.join(safe_repo(repo)?.replace('/', "__")))
}

/// A model that finished downloading and verified.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub repo: String,
    pub files: Vec<ModelFile>,
    /// Seconds since the Unix epoch.
    pub installed_at: u64,
}

fn read_installed(dir: &Path) -> Option<Installed> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Every installed model under `root`, sorted by id. A folder without a valid manifest is ignored.
pub fn list_installed(root: &Path) -> Vec<Installed> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<Installed> = entries.filter_map(|e| e.ok()).filter_map(|e| read_installed(&e.path())).collect();
    out.sort_by(|a, b| a.repo.cmp(&b.repo));
    out
}

// ----------------------------------------------------------------------------- hashing

/// SHA-256 of a file, hex lowercase. Native only: `sha2` is a native dependency.
#[cfg(not(target_arch = "wasm32"))]
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let read_err = |e: std::io::Error| EngineError::Other(format!("cannot read {}: {e}", path.display()));
    let mut file = std::fs::File::open(path).map_err(read_err)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(read_err)?;
        if n == 0 {
            break;
        }
        if let Some(chunk) = buf.get(..n) {
            hasher.update(chunk);
        }
    }
    Ok(hex(&hasher.finalize()))
}

#[cfg(not(target_arch = "wasm32"))]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ----------------------------------------------------------------------------- progress

/// Live progress of a download, shared between the thread that downloads and the UI that draws it.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }
    /// Bytes received so far, across every file of the download.
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    /// Bytes expected in all (0 until the hub has answered).
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
    /// Done over total, in 0..=1 (0 while the total is not known).
    pub fn fraction(&self) -> f32 {
        let total = self.total();
        if total == 0 { 0.0 } else { (self.done() as f64 / total as f64).clamp(0.0, 1.0) as f32 }
    }
    /// Ask the download to stop. It deletes its partial files and returns `Cancelled`.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn start(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn add(&self, bytes: u64) {
        let _ = self.done.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |d| Some(d.saturating_add(bytes)));
    }
}

// ----------------------------------------------------------------------------- local models

/// A `.onnx` file in the models folder, whether PhotoCraft downloaded it or the user dropped it in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalModel {
    /// Path under the models folder, `/`-separated: the name used to select it.
    pub name: String,
    pub bytes: u64,
    /// True when PhotoCraft downloaded it and checked its SHA-256; false for a file added by hand.
    pub verified: bool,
}

/// The name [`DEFAULT_MODEL_FILE`] has once downloaded.
pub fn default_model_name() -> String {
    format!("{}/{}", DEFAULT_MODEL_REPO.replace('/', "__"), DEFAULT_MODEL_FILE)
}

fn collect_onnx(dir: &Path, rel: &str, depth: usize, out: &mut Vec<LocalModel>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        if out.len() >= SCAN_LIMIT {
            return;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else { continue };
        let path = entry.path();
        let name = if rel.is_empty() { file_name.clone() } else { format!("{rel}/{file_name}") };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if depth < SCAN_DEPTH {
                collect_onnx(&path, &name, depth + 1, out);
            }
            continue;
        }
        if !file_name.to_ascii_lowercase().ends_with(".onnx") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() {
            continue;
        }
        out.push(LocalModel { verified: manifest_lists(root_of(&path, &name), &name, meta.len()), name, bytes: meta.len() });
    }
}

/// The models folder a file named `name` (relative to it) sits in.
fn root_of(path: &Path, name: &str) -> PathBuf {
    let mut root = path.to_path_buf();
    for _ in name.split('/') {
        root.pop();
    }
    root
}

/// Whether `name` (relative to `root`) is a file PhotoCraft downloaded: its model folder's manifest
/// lists it at this size. A file at the top level, or in a folder without a manifest, is not.
fn manifest_lists(root: PathBuf, name: &str, bytes: u64) -> bool {
    let Some((top, inside)) = name.split_once('/') else { return false };
    read_installed(&root.join(top)).is_some_and(|m| m.files.iter().any(|f| f.name == inside && f.bytes == bytes))
}

/// Every `.onnx` file under `root`, sorted by name.
pub fn list_local(root: &Path) -> Vec<LocalModel> {
    let mut out = Vec::new();
    collect_onnx(root, "", 1, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The model the user picked, if it is still in the folder.
pub fn selected_model(root: &Path) -> Option<String> {
    let name = std::fs::read_to_string(root.join(SELECTED)).ok()?.trim().to_string();
    list_local(root).into_iter().any(|m| m.name == name).then_some(name)
}

/// Remember `name` as the model to use. It must be a file in the folder.
pub fn select_model(root: &Path, name: &str) -> Result<()> {
    safe_relative_path(name)?;
    if !list_local(root).iter().any(|m| m.name == name) {
        return Err(EngineError::Other(format!("`{name}` is not in the models folder")));
    }
    std::fs::create_dir_all(root).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", root.display())))?;
    std::fs::write(root.join(SELECTED), name).map_err(|e| EngineError::Other(format!("cannot save the choice: {e}")))
}

/// The model a fill uses: `requested` if given, else the user's pick, else the first one found.
pub fn resolve_model(root: &Path, requested: &str) -> Result<LocalModel> {
    let all = list_local(root);
    if !requested.is_empty() {
        return all.into_iter().find(|m| m.name == requested).ok_or_else(|| EngineError::Other(format!("`{requested}` is not in the models folder")));
    }
    let picked = selected_model(root);
    let mut all = all;
    let index = picked.and_then(|n| all.iter().position(|m| m.name == n)).or(if all.is_empty() { None } else { Some(0) });
    match index {
        Some(i) => Ok(all.swap_remove(i)),
        None => Err(EngineError::Other("no Generative Fill model yet: open the tool's Models… window and download the default one, or put a .onnx file in the models folder".into())),
    }
}

// ----------------------------------------------------------------------------- network (native)

#[cfg(not(target_arch = "wasm32"))]
mod net {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    fn agent() -> ureq::Agent {
        // Timeouts bound each stall, not the whole transfer: a multi-gigabyte download is fine as long
        // as bytes keep arriving, but a hub that goes silent cannot hang the browser or a test.
        ureq::AgentBuilder::new()
            .user_agent("photocraft")
            .timeout_connect(std::time::Duration::from_secs(20))
            .timeout_read(std::time::Duration::from_secs(30))
            .build()
    }

    fn get_text(url: &str) -> Result<String> {
        agent().get(url).call().map_err(|e| EngineError::Other(format!("could not reach the model hub ({e})")))?.into_string().map_err(|e| EngineError::Other(format!("hub response was unreadable: {e}")))
    }

    pub fn search(query: &str, limit: usize) -> Result<Vec<HubModel>> {
        let response = agent()
            .get(&format!("{HUB}/api/models"))
            .query("search", query)
            .query("limit", &limit.to_string())
            .query("sort", "downloads")
            .call()
            .map_err(|e| EngineError::Other(format!("could not reach the model hub ({e})")))?;
        let body = response.into_string().map_err(|e| EngineError::Other(format!("hub response was unreadable: {e}")))?;
        parse_search(&body)
    }

    pub fn repo_files(repo: &str) -> Result<Vec<ModelFile>> {
        safe_repo(repo)?;
        parse_repo_files(&get_text(&format!("{HUB}/api/models/{repo}?blobs=true"))?)
    }

    /// Downloads `file` from the hub's copy of `repo`. See [`fetch_url`].
    pub fn fetch_file(repo: &str, file: &ModelFile, dir: &Path, progress: &Progress) -> Result<()> {
        fetch_url(&format!("{HUB}/{repo}/resolve/main/{}", file.name), file, dir, progress)
    }

    /// Streams `url` into `<dir>/<name>.part`, hashing as it goes and counting bytes in `progress`,
    /// then renames it into place only if the size and SHA-256 match what the hub lists. Whatever
    /// goes wrong (network, cancel, size, checksum) the partial file is deleted.
    pub fn fetch_url(url: &str, file: &ModelFile, dir: &Path, progress: &Progress) -> Result<()> {
        let rel = safe_relative_path(&file.name)?;
        let dest = dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", parent.display())))?;
        }
        let part = sibling(&dest, "part");
        let result = stream(url, file, &part, progress);
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
            return result;
        }
        std::fs::rename(&part, &dest).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            EngineError::Other(format!("cannot install {}: {e}", dest.display()))
        })
    }

    fn stream(url: &str, file: &ModelFile, part: &Path, progress: &Progress) -> Result<()> {
        if progress.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        let response = agent().get(url).call().map_err(|e| EngineError::Other(format!("download of `{}` failed ({e})", file.name)))?;
        let mut reader = response.into_reader().take(file.bytes.saturating_add(1));
        let mut out = std::fs::File::create(part).map_err(|e| EngineError::Other(format!("cannot write {}: {e}", part.display())))?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            if progress.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| EngineError::Other(format!("download of `{}` was interrupted: {e}", file.name)))?;
            if n == 0 {
                break;
            }
            written = written.saturating_add(n as u64);
            if written > file.bytes {
                return Err(EngineError::Other(format!("`{}` is larger than the hub says; not installed", file.name)));
            }
            let Some(chunk) = buf.get(..n) else { break };
            hasher.update(chunk);
            out.write_all(chunk).map_err(|e| EngineError::Other(format!("cannot write {}: {e}", part.display())))?;
            progress.add(n as u64);
        }
        out.sync_all().ok();
        if written != file.bytes {
            return Err(EngineError::Other(format!("`{}` ended early ({written} of {} bytes); not installed", file.name, file.bytes)));
        }
        if hex(&hasher.finalize()) != file.sha256 {
            return Err(EngineError::Other(format!("`{}` failed its checksum; not installed", file.name)));
        }
        Ok(())
    }

    /// Opens `dir` in the system file manager.
    pub fn open_folder(dir: &Path) -> Result<()> {
        let program = if cfg!(target_os = "windows") {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let mut child = std::process::Command::new(program)
            .arg(dir)
            .spawn()
            .map_err(|e| EngineError::Other(format!("could not open the folder ({e}); it is at {}", dir.display())))?;
        // Reap the helper so it does not linger as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod net {
    use super::*;
    const OFFLINE: &str = "downloading models is not available in the web build";
    pub fn search(_: &str, _: usize) -> Result<Vec<HubModel>> {
        Err(EngineError::Other(OFFLINE.into()))
    }
    pub fn repo_files(_: &str) -> Result<Vec<ModelFile>> {
        Err(EngineError::Other(OFFLINE.into()))
    }
    pub fn fetch_file(_: &str, _: &ModelFile, _: &Path, _: &Progress) -> Result<()> {
        Err(EngineError::Other(OFFLINE.into()))
    }
    pub fn open_folder(_: &Path) -> Result<()> {
        Err(EngineError::Other("opening folders is not available in the web build".into()))
    }
}

/// Downloads and verifies `repo` (just `only_file` if given), replacing any earlier copy only once
/// every file has verified.
fn download(root: &Path, repo: &str, only_file: Option<&str>, progress: &Progress) -> Result<Installed> {
    let dir = model_dir(root, repo)?;
    let mut files = net::repo_files(repo)?;
    if let Some(wanted) = only_file {
        safe_relative_path(wanted)?;
        files.retain(|f| f.name == wanted);
        if files.is_empty() {
            return Err(EngineError::Other(format!("`{wanted}` is not a model file in {repo}")));
        }
    }
    progress.start(files.iter().map(|f| f.bytes).fold(0u64, u64::saturating_add));
    let staging = sibling(&dir, "partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", staging.display())))?;
    for f in &files {
        if let Err(e) = net::fetch_file(repo, f, &staging, progress) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    }
    let installed = Installed {
        repo: repo.to_string(),
        files,
        installed_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    let manifest = serde_json::to_string_pretty(&installed).map_err(|e| EngineError::Other(e.to_string()))?;
    std::fs::write(staging.join(MANIFEST), manifest).map_err(|e| EngineError::Other(format!("cannot write manifest: {e}")))?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| EngineError::Other(format!("cannot install {}: {e}", dir.display())))?;
    Ok(installed)
}

// ----------------------------------------------------------------------------- public API

/// Hub models matching `query`, most downloaded first. Blocks on the network; call it off the UI thread.
pub fn search_hub(query: &str, limit: usize) -> Result<Vec<HubModel>> {
    net::search(query, limit)
}

/// Downloads and verifies `repo` (just `file`, if given) into the models folder, reporting bytes in
/// `progress`. Blocks on the network; call it off the UI thread.
pub fn download_model(repo: &str, file: Option<&str>, progress: &Progress) -> Result<Installed> {
    let root = models_root()?;
    download(&root, safe_repo(repo)?, file, progress)
}

/// Downloads the one-click default model ([`DEFAULT_MODEL_REPO`]).
pub fn download_default_model(progress: &Progress) -> Result<Installed> {
    download_model(DEFAULT_MODEL_REPO, Some(DEFAULT_MODEL_FILE), progress)
}

/// Every `.onnx` file in the models folder.
pub fn local_models() -> Result<Vec<LocalModel>> {
    Ok(list_local(&models_root()?))
}

/// The user's chosen model, if it is still there.
pub fn selected_model_name() -> Result<Option<String>> {
    Ok(selected_model(&models_root()?))
}

/// Choose the model Generative Fill uses.
pub fn select_local_model(name: &str) -> Result<()> {
    select_model(&models_root()?, name)
}

/// Creates the models folder if needed and opens it in the file manager. Returns its path.
pub fn open_models_folder() -> Result<PathBuf> {
    let root = models_root()?;
    std::fs::create_dir_all(&root).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", root.display())))?;
    net::open_folder(&root)?;
    Ok(root)
}

/// Models installed in the cache, sorted by id.
pub fn installed_models() -> Result<Vec<Installed>> {
    Ok(list_installed(&models_root()?))
}

/// Deletes an installed model. Returns whether it was there.
pub fn remove_model(repo: &str) -> Result<bool> {
    let dir = model_dir(&models_root()?, repo)?;
    if !dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| EngineError::Other(format!("cannot remove {}: {e}", dir.display())))?;
    Ok(true)
}

// ----------------------------------------------------------------------------- params

fn text_param(cmd: &str, p: &Value, key: &str, max_chars: usize) -> Result<String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) if s.chars().count() <= max_chars => Ok(s.clone()),
        Some(Value::String(_)) => Err(bad(cmd, format!("`{key}` is longer than {max_chars} characters"))),
        Some(v) => Err(bad(cmd, format!("`{key}` must be text, not {v}"))),
    }
}

fn number_param(cmd: &str, p: &Value, key: &str, min: f64, max: f64, default: f64) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number, not {v}")))?;
            if n < min || n > max {
                return Err(bad(cmd, format!("`{key}` must be between {min} and {max}")));
            }
            Ok(n)
        }
    }
}

fn repo_param(cmd: &str, p: &Value) -> Result<String> {
    let repo = p.get("repo").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`repo` (owner/name) is required"))?;
    safe_repo(repo).map(str::to_string).map_err(|e| bad(cmd, e.to_string()))
}

fn has_selection_on_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_none() {
        return Err("Generative Fill needs a selection".into());
    }
    d.active_layer.and_then(|l| d.doc.layer(l)).filter(|l| l.surface().is_some()).map(|_| ()).ok_or_else(|| "Generative Fill needs an active pixel layer".into())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

// ----------------------------------------------------------------------------- commands

fn run_search(_: &mut Session, p: &Value) -> Result<Value> {
    let query = text_param(SEARCH, p, "query", 200)?;
    let query = if query.trim().is_empty() { "inpaint".to_string() } else { query };
    let limit = number_param(SEARCH, p, "limit", 1.0, 100.0, 20.0)? as usize;
    let models = search_hub(query.trim(), limit)?;
    Ok(json!({ "query": query.trim(), "models": models }))
}

fn run_installed(_: &mut Session, _: &Value) -> Result<Value> {
    let root = models_root()?;
    Ok(json!({ "root": root.display().to_string(), "models": list_installed(&root) }))
}

fn run_download(_: &mut Session, p: &Value) -> Result<Value> {
    let repo = repo_param(DOWNLOAD, p)?;
    let file = text_param(DOWNLOAD, p, "file", 256)?;
    let file = (!file.trim().is_empty()).then(|| file.trim().to_string());
    serde_json::to_value(download_model(&repo, file.as_deref(), &Progress::new())?).map_err(|e| EngineError::Other(e.to_string()))
}

fn run_local(_: &mut Session, _: &Value) -> Result<Value> {
    let root = models_root()?;
    Ok(json!({ "root": root.display().to_string(), "selected": selected_model(&root), "default": default_model_name(), "models": list_local(&root) }))
}

fn run_select(_: &mut Session, p: &Value) -> Result<Value> {
    let name = text_param(SELECT, p, "model", 300)?;
    if name.trim().is_empty() {
        return Err(bad(SELECT, "`model` (a name from ai.generativeFill.localModels) is required"));
    }
    select_model(&models_root()?, name.trim())?;
    Ok(json!({ "selected": name.trim() }))
}

fn run_remove(_: &mut Session, p: &Value) -> Result<Value> {
    let repo = repo_param(REMOVE, p)?;
    let removed = remove_model(&repo)?;
    Ok(json!({ "repo": repo, "removed": removed }))
}

fn run_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = text_param(FILL, p, "prompt", MAX_PROMPT_CHARS)?;
    let negative = text_param(FILL, p, "negativePrompt", MAX_PROMPT_CHARS)?;
    let steps = number_param(FILL, p, "steps", 1.0, 50.0, 20.0)?.round() as u32;
    let guidance = number_param(FILL, p, "guidance", 1.0, 20.0, 7.5)?;
    let requested = text_param(FILL, p, "model", 200)?;
    has_selection_on_pixels(s).map_err(EngineError::Other)?;
    let model = resolve_model(&models_root()?, requested.trim())?;
    let _ = (prompt, negative, steps, guidance);
    Err(EngineError::Other(format!(
        "Generative Fill cannot paint yet: the model `{}` is ready, but the part of PhotoCraft that runs it is not built. Nothing was changed.",
        model.name
    )))
}

/// The commands this module adds to the registry.
pub fn specs() -> Vec<CommandSpec> {
    fn quiet(
        id: &'static str,
        label: &'static str,
        params: &'static str,
        enabled: fn(&Session) -> std::result::Result<(), String>,
        run: fn(&mut Session, &Value) -> Result<Value>,
    ) -> CommandSpec {
        CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: false }
    }
    vec![
        quiet(SEARCH, "Search Models…", r#"{"query":text="inpaint","limit":1..100=20} → {query, models:[{id, downloads, likes, pipeline_tag}]}"#, always, run_search),
        quiet(INSTALLED, "Installed Models", "{} → {root, models:[{repo, files, installed_at}]}", always, run_installed),
        quiet(DOWNLOAD, "Download Model", r#"{"repo":"owner/name","file":text} → the installed model (every file checksum-verified; `file` limits it to one weight file)"#, always, run_download),
        quiet(LOCAL, "Local Models", "{} → {root, selected, default, models:[{name, bytes, verified}]} (every .onnx file in the models folder)", always, run_local),
        quiet(SELECT, "Choose Model", r#"{"model":text} → {selected} (a name from ai.generativeFill.localModels)"#, always, run_select),
        quiet(REMOVE, "Remove Model", r#"{"repo":"owner/name"} → {repo, removed}"#, always, run_remove),
        CommandSpec {
            id: FILL,
            label: "Generative Fill…",
            menu: &[],
            shortcut: None,
            params: r#"{"prompt":text,"negativePrompt":text,"steps":1..50=20,"guidance":1..20=7.5,"model":text}"#,
            enabled: has_selection_on_pixels,
            run: run_fill,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO_JSON: &str = r#"{"id":"owner/inpaint","siblings":[
        {"rfilename":"README.md"},
        {"rfilename":"model.onnx","size":12,"lfs":{"sha256":"AB12CD34EF56AB12CD34EF56AB12CD34EF56AB12CD34EF56AB12CD34EF56AB12","size":12}},
        {"rfilename":"nested/extra.ONNX","lfs":{"sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":3}}
    ]}"#;

    #[test]
    fn repo_ids_are_checked_before_they_become_paths_or_urls() {
        assert_eq!(safe_repo("owner/name.v1").ok(), Some("owner/name.v1"));
        for bad in ["", "owner", "a/b/c", "../etc", "owner/..", "own er/x", "owner/x?y=1", "owner/a\\b"] {
            assert!(safe_repo(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn file_names_cannot_escape_the_model_folder() {
        assert!(safe_relative_path("model.onnx").is_ok());
        assert!(safe_relative_path("nested/extra.onnx").is_ok());
        for bad in ["", "/etc/passwd", "../x.onnx", "a/../../x.onnx", "..\\x.onnx", "C:\\x.onnx"] {
            assert!(safe_relative_path(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn repo_metadata_yields_only_verified_onnx_files() {
        let files = parse_repo_files(REPO_JSON).expect("parses");
        assert_eq!(files.len(), 2, "README.md is not a weight file");
        assert_eq!(files[0].name, "model.onnx");
        assert_eq!(files[0].sha256, "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12", "checksums are normalised to lowercase");
    }

    #[test]
    fn a_weight_file_without_a_checksum_is_refused() {
        let body = r#"{"siblings":[{"rfilename":"model.onnx","size":3}]}"#;
        assert!(parse_repo_files(body).is_err());
    }

    #[test]
    fn oversized_or_empty_models_are_refused() {
        let big = format!(r#"{{"siblings":[{{"rfilename":"m.onnx","lfs":{{"sha256":"{}","size":{}}}}}]}}"#, "a".repeat(64), MAX_MODEL_BYTES + 1);
        assert!(parse_repo_files(&big).is_err());
        assert!(parse_repo_files(r#"{"siblings":[]}"#).is_err());
    }

    #[test]
    fn search_results_parse_and_drop_bad_ids() {
        let body = r#"[{"id":"a/b","downloads":5,"likes":1,"pipeline_tag":"image-to-image"},{"id":"../x"},{"id":"c/d"}]"#;
        let models = parse_search(body).expect("parses");
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/b", "c/d"]);
        assert_eq!(models[0].downloads, 5);
        assert!(parse_search("not json").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn checksums_are_computed_over_the_file_bytes() {
        let dir = std::env::temp_dir().join(format!("pc-genfill-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let f = dir.join("abc.bin");
        std::fs::write(&f, b"abc").expect("write");
        assert_eq!(sha256_file(&f).expect("hash"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_models_are_listed_only_with_a_manifest() {
        let root = std::env::temp_dir().join(format!("pc-genfill-list-{}", std::process::id()));
        let good = root.join("owner__good");
        std::fs::create_dir_all(&good).expect("dir");
        let m = Installed { repo: "owner/good".into(), files: vec![], installed_at: 1 };
        std::fs::write(good.join(MANIFEST), serde_json::to_string(&m).expect("json")).expect("write");
        std::fs::create_dir_all(root.join("owner__partial")).expect("dir");
        let listed = list_installed(&root);
        assert_eq!(listed.iter().map(|i| i.repo.as_str()).collect::<Vec<_>>(), ["owner/good"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn generative_fill_validates_before_touching_anything() {
        assert!(number_param(FILL, &json!({"steps": 0}), "steps", 1.0, 50.0, 20.0).is_err());
        assert!(number_param(FILL, &json!({"steps": "many"}), "steps", 1.0, 50.0, 20.0).is_err());
        assert_eq!(number_param(FILL, &json!({}), "steps", 1.0, 50.0, 20.0).ok(), Some(20.0));
        assert!(text_param(FILL, &json!({"prompt": "x".repeat(MAX_PROMPT_CHARS + 1)}), "prompt", MAX_PROMPT_CHARS).is_err());
        assert!(repo_param(DOWNLOAD, &json!({})).is_err());
        assert!(repo_param(DOWNLOAD, &json!({"repo": "../../etc"})).is_err());
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-gf-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("dir");
        d
    }

    #[test]
    fn progress_reports_a_fraction_and_cancel() {
        let p = Progress::new();
        assert_eq!(p.fraction(), 0.0);
        p.start(200);
        p.add(50);
        assert!((p.fraction() - 0.25).abs() < 1e-6);
        assert!(!p.is_cancelled());
        p.cancel();
        assert!(p.is_cancelled());
    }

    #[test]
    fn local_models_show_hand_added_files_and_mark_downloaded_ones() {
        let root = temp("local");
        let dl = root.join("owner__good");
        std::fs::create_dir_all(dl.join("sub")).expect("dir");
        std::fs::write(dl.join("m.onnx"), b"abcd").expect("w");
        let m = Installed {
            repo: "owner/good".into(),
            files: vec![ModelFile { name: "m.onnx".into(), bytes: 4, sha256: "0".repeat(64) }],
            installed_at: 1,
        };
        std::fs::write(dl.join(MANIFEST), serde_json::to_string(&m).expect("json")).expect("w");
        std::fs::write(dl.join("sub").join("deep.onnx"), b"xy").expect("w");
        std::fs::write(root.join("mine.ONNX"), b"xyz").expect("w");
        std::fs::write(root.join("notes.txt"), b"x").expect("w");
        std::fs::write(dl.join("m.onnx.part"), b"x").expect("w");
        let got: Vec<_> = list_local(&root).into_iter().map(|m| (m.name, m.verified)).collect();
        assert_eq!(
            got,
            [
                ("mine.ONNX".to_string(), false),
                ("owner__good/m.onnx".to_string(), true),
                ("owner__good/sub/deep.onnx".to_string(), false),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn choosing_a_model_is_remembered_and_unknown_names_are_refused() {
        let root = temp("select");
        std::fs::write(root.join("a.onnx"), b"1").expect("w");
        std::fs::write(root.join("b.onnx"), b"2").expect("w");
        assert_eq!(resolve_model(&root, "").expect("first").name, "a.onnx");
        select_model(&root, "b.onnx").expect("select");
        assert_eq!(selected_model(&root).as_deref(), Some("b.onnx"));
        assert_eq!(resolve_model(&root, "").expect("picked").name, "b.onnx");
        assert!(select_model(&root, "nope.onnx").is_err());
        assert!(select_model(&root, "../x.onnx").is_err());
        assert!(resolve_model(&root, "nope.onnx").is_err());
        std::fs::remove_file(root.join("b.onnx")).expect("rm");
        assert_eq!(selected_model(&root), None);
        assert_eq!(resolve_model(&root, "").expect("falls back").name, "a.onnx");
        let empty = temp("select-empty");
        assert!(resolve_model(&empty, "").is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// Serves `body` (declaring `declared` bytes) once on localhost; returns the URL.
    #[cfg(not(target_arch = "wasm32"))]
    fn serve(body: Vec<u8>, declared: usize) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut req = [0u8; 2048];
                let _ = conn.read(&mut req);
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n");
                let _ = conn.write_all(head.as_bytes());
                let _ = conn.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}/m.onnx")
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn abc() -> ModelFile {
        ModelFile { name: "m.onnx".into(), bytes: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into() }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_good_download_is_installed_and_counted() {
        let dir = temp("fetch-ok");
        let p = Progress::new();
        p.start(3);
        net::fetch_url(&serve(b"abc".to_vec(), 3), &abc(), &dir, &p).expect("download");
        assert_eq!(std::fs::read(dir.join("m.onnx")).expect("file"), b"abc");
        assert_eq!(p.done(), 3);
        assert!(!dir.join("m.onnx.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bad_downloads_leave_nothing_behind() {
        let cases: [(&str, Vec<u8>, usize); 3] = [
            ("wrong bytes", b"xyz".to_vec(), 3),
            ("too big", b"abcdef".to_vec(), 6),
            ("cut short", b"ab".to_vec(), 3),
        ];
        for (label, body, declared) in cases {
            let dir = temp("fetch-bad");
            let p = Progress::new();
            let r = net::fetch_url(&serve(body, declared), &abc(), &dir, &p);
            assert!(r.is_err(), "{label} should fail");
            assert!(!dir.join("m.onnx").exists(), "{label}: model installed");
            assert!(!dir.join("m.onnx.part").exists(), "{label}: part file left");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cancelling_stops_the_download_and_cleans_up() {
        let dir = temp("fetch-cancel");
        let p = Progress::new();
        p.cancel();
        let r = net::fetch_url(&serve(b"abc".to_vec(), 3), &abc(), &dir, &p);
        assert!(matches!(r, Err(EngineError::Cancelled)));
        assert!(!dir.join("m.onnx.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
