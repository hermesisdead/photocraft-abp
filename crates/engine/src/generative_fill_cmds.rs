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

    /// Streams one file into `<dir>/<name>.part`, hashing as it goes, then renames it into place
    /// only if the size and SHA-256 match what the hub lists.
    pub fn fetch_file(repo: &str, file: &ModelFile, dir: &Path) -> Result<()> {
        let rel = safe_relative_path(&file.name)?;
        let dest = dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", parent.display())))?;
        }
        let part = sibling(&dest, "part");
        let url = format!("{HUB}/{repo}/resolve/main/{}", file.name);
        let response = agent().get(&url).call().map_err(|e| EngineError::Other(format!("download of `{}` failed ({e})", file.name)))?;
        let mut reader = response.into_reader().take(file.bytes.saturating_add(1));
        let mut out = std::fs::File::create(&part).map_err(|e| EngineError::Other(format!("cannot write {}: {e}", part.display())))?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf).map_err(|e| EngineError::Other(format!("download of `{}` was interrupted: {e}", file.name)))?;
            if n == 0 {
                break;
            }
            written = written.saturating_add(n as u64);
            if written > file.bytes {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return Err(EngineError::Other(format!("`{}` is larger than the hub says; not installed", file.name)));
            }
            let Some(chunk) = buf.get(..n) else { break };
            hasher.update(chunk);
            out.write_all(chunk).map_err(|e| EngineError::Other(format!("cannot write {}: {e}", part.display())))?;
        }
        out.sync_all().ok();
        drop(out);
        if written != file.bytes || hex(&hasher.finalize()) != file.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(EngineError::Other(format!("`{}` failed its checksum; not installed", file.name)));
        }
        std::fs::rename(&part, &dest).map_err(|e| EngineError::Other(format!("cannot install {}: {e}", dest.display())))?;
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
    pub fn fetch_file(_: &str, _: &ModelFile, _: &Path) -> Result<()> {
        Err(EngineError::Other(OFFLINE.into()))
    }
}

/// Downloads and verifies `repo`, replacing any earlier copy only once every file has verified.
fn download(root: &Path, repo: &str) -> Result<Installed> {
    let dir = model_dir(root, repo)?;
    let files = net::repo_files(repo)?;
    let staging = sibling(&dir, "partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", staging.display())))?;
    for f in &files {
        if let Err(e) = net::fetch_file(repo, f, &staging) {
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

/// Downloads and verifies `repo` into the cache. Blocks on the network; call it off the UI thread.
pub fn download_model(repo: &str) -> Result<Installed> {
    let root = models_root()?;
    download(&root, safe_repo(repo)?)
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

/// The model the fill uses: `model` if given (it must be installed), else the first installed one.
fn pick_model(cmd: &str, root: &Path, requested: &str) -> Result<Installed> {
    let installed = list_installed(root);
    if !requested.is_empty() {
        return installed.into_iter().find(|m| m.repo == requested).ok_or_else(|| bad(cmd, format!("`{requested}` is not installed")));
    }
    installed.into_iter().next().ok_or_else(|| EngineError::Other("no Generative Fill model is installed yet: choose one in Generative Fill › Models…".into()))
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
    serde_json::to_value(download_model(&repo)?).map_err(|e| EngineError::Other(e.to_string()))
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
    let model = pick_model(FILL, &models_root()?, requested.trim())?;
    let _ = (prompt, negative, steps, guidance);
    Err(EngineError::Other(format!(
        "Generative Fill is not available in this build: `{}` is installed, but the inpainting backend that runs it is not part of PhotoCraft yet",
        model.repo
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
        quiet(DOWNLOAD, "Download Model", r#"{"repo":"owner/name"} → the installed model (every file checksum-verified)"#, always, run_download),
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
}
