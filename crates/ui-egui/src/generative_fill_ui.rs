//! Generative Fill's model window: pick the model to use from the models folder, download the
//! default one with a progress bar, open the folder to add models by hand, and search Hugging Face
//! for more. Opened from the Generative Fill tool's options bar (Models…).
//!
//! Searches and downloads run on a background thread, so the canvas keeps responding during a
//! 200 MB download. The thread counts bytes in a shared [`gf::Progress`] that the window draws as a
//! bar, and reports back through a channel polled each frame. A download that fails leaves its
//! error on screen in red, with a Try again button, until the next attempt starts.
//!
//! Everything here calls `photocraft_engine::generative_fill_cmds`, the same functions the
//! `ai.generativeFill.*` commands use.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use egui::{Context, RichText};
use photocraft_engine::generative_fill_cmds::{self as gf, HubModel, Installed, LocalModel};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Results shown per search.
const SEARCH_LIMIT: usize = 25;

/// The window's state. Lives on the app (not in the saved UI state): it holds live channels.
#[derive(Default)]
pub struct Tasks {
    pub open: bool,
    pub query: String,
    pub results: Vec<HubModel>,
    pub searched: bool,
    /// Models downloaded from the hub (they have a manifest), for the Remove buttons.
    pub installed: Vec<Installed>,
    /// Every `.onnx` file in the models folder, downloaded or added by hand.
    pub local: Vec<LocalModel>,
    /// The model the user picked, if it is still in the folder.
    pub selected: Option<String>,
    pub status: String,
    /// The last download's error, shown in red with Try again until the next attempt starts.
    pub failed: Option<String>,
    /// The running download's byte counter, for the progress bar.
    pub progress: Option<Arc<gf::Progress>>,
    loaded: bool,
    last_request: Option<(String, Option<String>)>,
    search_rx: Option<Receiver<Result<Vec<HubModel>, String>>>,
    download_rx: Option<(String, Receiver<Result<Installed, String>>)>,
}

impl Tasks {
    fn downloading(&self) -> Option<&str> {
        self.download_rx.as_ref().map(|(repo, _)| repo.as_str())
    }
}

/// Opens the window and rescans the models folder.
pub fn open(t: &mut Tasks) {
    t.open = true;
    refresh(t);
}

fn refresh(t: &mut Tasks) {
    t.loaded = true;
    match gf::installed_models() {
        Ok(list) => t.installed = list,
        Err(e) => t.status = e.to_string(),
    }
    match gf::local_models() {
        Ok(list) => t.local = list,
        Err(e) => t.status = e.to_string(),
    }
    t.selected = gf::selected_model_name().ok().flatten();
}

/// `1.4 MB`-style size.
fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn local_label(m: &LocalModel) -> String {
    format!("{}  ({}{})", m.name, mb(m.bytes), if m.verified { "" } else { ", added by you" })
}

/// The name to select after `installed` downloads, when the user has not picked a model yet.
fn auto_pick(selected: &Option<String>, installed: &Installed) -> Option<String> {
    if selected.is_some() {
        return None;
    }
    let first = installed.files.first()?;
    Some(format!("{}/{}", installed.repo.replace('/', "__"), first.name))
}

fn start_search(t: &mut Tasks, ctx: &Context) {
    if t.search_rx.is_some() {
        return;
    }
    let query = t.query.trim().to_string();
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let q = if query.is_empty() { "inpaint" } else { query.as_str() };
        let _ = tx.send(gf::search_hub(q, SEARCH_LIMIT).map_err(|e| e.to_string()));
        ctx.request_repaint();
    });
    t.search_rx = Some(rx);
    t.status = "Searching…".into();
}

fn start_download(t: &mut Tasks, repo: &str, file: Option<&str>, ctx: &Context) {
    if t.download_rx.is_some() {
        t.status = "Another model is downloading; wait for it to finish".into();
        return;
    }
    let progress = Arc::new(gf::Progress::new());
    let (tx, rx) = channel();
    let (ctx2, owned, owned_file, counter) = (ctx.clone(), repo.to_string(), file.map(str::to_string), progress.clone());
    std::thread::spawn(move || {
        let _ = tx.send(gf::download_model(&owned, owned_file.as_deref(), &counter).map_err(|e| e.to_string()));
        ctx2.request_repaint();
    });
    t.download_rx = Some((repo.to_string(), rx));
    t.progress = Some(progress);
    t.failed = None;
    t.last_request = Some((repo.to_string(), file.map(str::to_string)));
    t.status = format!("Downloading {repo}…");
}

/// Applies any finished background work. Called once per frame.
pub fn poll(t: &mut Tasks, ctx: &Context) {
    if let Some(rx) = t.search_rx.as_ref() {
        match rx.try_recv() {
            Ok(result) => {
                t.search_rx = None;
                t.searched = true;
                match result {
                    Ok(models) => {
                        t.status = match models.len() {
                            1 => "1 model found".to_string(),
                            n => format!("{n} models found"),
                        };
                        t.results = models;
                    }
                    Err(e) => {
                        t.status = e;
                        t.results.clear();
                    }
                }
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                t.search_rx = None;
                t.status = "The search stopped unexpectedly".into();
            }
        }
    }
    let outcome = t.download_rx.as_ref().map(|(repo, rx)| (repo.clone(), rx.try_recv()));
    match outcome {
        Some((_, Err(TryRecvError::Empty))) => ctx.request_repaint_after(Duration::from_millis(100)),
        Some((repo, Ok(result))) => {
            t.download_rx = None;
            t.progress = None;
            match result {
                Ok(installed) => {
                    t.failed = None;
                    t.status = format!("Downloaded {repo}.");
                    refresh(t);
                    if let Some(name) = auto_pick(&t.selected, &installed)
                        && gf::select_local_model(&name).is_ok()
                    {
                        t.selected = Some(name);
                    }
                }
                Err(e) if e == "cancelled" => t.status = "Download cancelled.".into(),
                Err(e) => {
                    t.status = format!("Download failed: {e}");
                    t.failed = Some(e);
                }
            }
        }
        Some((_, Err(TryRecvError::Disconnected))) => {
            t.download_rx = None;
            t.progress = None;
            t.failed = Some("the download stopped unexpectedly".into());
            t.status = "Download failed: the download stopped unexpectedly".into();
        }
        None => {}
    }
}

/// The window, drawn each frame while it is open.
pub fn window(app: &mut PhotocraftApp, ctx: &Context) {
    poll(&mut app.generative_fill, ctx);
    if !app.generative_fill.open {
        return;
    }
    let mut open = true;
    egui::Window::new(tl!("Generative Fill Models"))
        .id(egui::Id::new("generative-fill-models"))
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .open(&mut open)
        .show(ctx, |ui| body(&mut app.generative_fill, ui, ctx));
    if !open {
        app.generative_fill.open = false;
    }
}

enum Action {
    Select(String),
    Download(String, Option<String>),
    Retry,
    Cancel,
    Remove(String),
    OpenFolder,
    Refresh,
}

fn body(t: &mut Tasks, ui: &mut egui::Ui, ctx: &Context) {
    let tokens = Tokens::get(ctx);
    let (dim, danger) = (tokens.text_dim, tokens.danger);
    let mut action: Option<Action> = None;

    // ---- the model in use
    ui.label(RichText::new(tl!("Model in use")).strong());
    if t.local.is_empty() {
        ui.label(RichText::new(tl!("No model yet. Download the default model, or put a .onnx file in the models folder.")).color(dim));
    } else {
        let current = t.selected.clone().or_else(|| t.local.first().map(|m| m.name.clone())).unwrap_or_default();
        let labels: Vec<String> = t.local.iter().map(local_label).collect();
        let options: Vec<(String, &str)> = t.local.iter().zip(&labels).map(|(m, l)| (m.name.clone(), l.as_str())).collect();
        let mut chosen = current.clone();
        if crate::widgets::dropdown(ui, "generative-fill-model", &mut chosen, &options, 440.0) && chosen != current {
            action = Some(Action::Select(chosen));
        }
    }

    // ---- the default model, its progress, and any failure
    let busy = t.progress.is_some();
    let default_name = gf::default_model_name();
    ui.horizontal(|ui| {
        if !busy && !t.local.iter().any(|m| m.name == default_name) && crate::widgets::primary_button(ui, tl!("Download default model (LaMa, about 208 MB)"), 0.0).clicked() {
            action = Some(Action::Download(gf::DEFAULT_MODEL_REPO.into(), Some(gf::DEFAULT_MODEL_FILE.into())));
        }
        if crate::widgets::secondary_button(ui, tl!("Open models folder"), 0.0).clicked() {
            action = Some(Action::OpenFolder);
        }
        if crate::widgets::secondary_button(ui, tl!("Refresh"), 0.0).clicked() {
            action = Some(Action::Refresh);
        }
    });
    if let Some(p) = &t.progress {
        let known = p.total() > 0;
        let text = if known { format!("{:.0}%  ·  {} of {}", p.fraction() * 100.0, mb(p.done()), mb(p.total())) } else { "Contacting Hugging Face…".to_string() };
        ui.horizontal(|ui| {
            ui.add(egui::ProgressBar::new(p.fraction()).text(text).desired_width(360.0).animate(!known));
            if crate::widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
                action = Some(Action::Cancel);
            }
        });
    } else if let Some(err) = &t.failed {
        ui.label(RichText::new(format!("Download failed: {err}")).color(danger));
        if t.last_request.is_some() && crate::widgets::secondary_button(ui, tl!("Try again"), 0.0).clicked() {
            action = Some(Action::Retry);
        }
    }
    ui.label(RichText::new(tl!("To use another model, put its .onnx file in the models folder, then press Refresh.")).size(12.0).color(dim));
    ui.separator();

    // ---- Hugging Face
    egui::CollapsingHeader::new(tl!("Find more models on Hugging Face")).default_open(false).show(ui, |ui| {
        let mut search_now = false;
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut t.query).hint_text(tl!("Search models (default: inpaint)")).desired_width(300.0));
            if crate::widgets::primary_button(ui, tl!("Search"), 90.0).clicked() || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                search_now = true;
            }
        });
        if search_now {
            start_search(t, ctx);
        }
        let downloading = t.downloading().map(str::to_string);
        egui::ScrollArea::vertical().id_salt("generative-fill-results").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
            if !t.searched {
                ui.label(RichText::new(tl!("Search the hub to find models.")).color(dim));
            } else if t.results.is_empty() {
                ui.label(RichText::new(tl!("No models matched.")).color(dim));
            }
            for m in &t.results {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&m.id).strong());
                        let tag = m.pipeline_tag.as_deref().unwrap_or("no task listed");
                        ui.label(RichText::new(format!("{tag} · {} downloads · {} likes", m.downloads, m.likes)).size(11.0).color(dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if t.installed.iter().any(|i| i.repo == m.id) {
                            ui.label(RichText::new(tl!("Downloaded")).color(dim));
                        } else if downloading.as_deref() == Some(m.id.as_str()) {
                            ui.add(egui::Spinner::new());
                        } else if !busy && crate::widgets::secondary_button(ui, tl!("Download"), 90.0).clicked() {
                            action = Some(Action::Download(m.id.clone(), None));
                        }
                    });
                });
                ui.separator();
            }
        });
        if !t.installed.is_empty() {
            ui.label(RichText::new(tl!("Downloaded from Hugging Face")).strong());
            for i in &t.installed {
                ui.horizontal(|ui| {
                    ui.label(&i.repo);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::secondary_button(ui, tl!("Remove"), 90.0).clicked() {
                            action = Some(Action::Remove(i.repo.clone()));
                        }
                    });
                });
            }
        }
    });

    if !t.status.is_empty() && t.failed.is_none() {
        ui.add_space(6.0);
        ui.label(RichText::new(&t.status).size(12.0).color(dim));
    }

    match action {
        Some(Action::Select(name)) => match gf::select_local_model(&name) {
            Ok(()) => t.selected = Some(name),
            Err(e) => t.status = e.to_string(),
        },
        Some(Action::Download(repo, file)) => start_download(t, &repo, file.as_deref(), ctx),
        Some(Action::Retry) => {
            if let Some((repo, file)) = t.last_request.clone() {
                start_download(t, &repo, file.as_deref(), ctx);
            }
        }
        Some(Action::Cancel) => {
            if let Some(p) = &t.progress {
                p.cancel();
                t.status = "Cancelling…".into();
            }
        }
        Some(Action::Remove(repo)) => match gf::remove_model(&repo) {
            Ok(_) => {
                t.status = format!("Removed {repo}");
                refresh(t);
            }
            Err(e) => t.status = e.to_string(),
        },
        Some(Action::OpenFolder) => match gf::open_models_folder() {
            Ok(path) => t.status = format!("Opened {}", path.display()),
            Err(e) => t.status = e.to_string(),
        },
        Some(Action::Refresh) => {
            refresh(t);
            t.status = format!("{} model file(s) in the folder", t.local.len());
        }
        None => {}
    }
}

/// The Generative Fill tool's options bar: the model in use, Generate…, and Models….
pub fn tool_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !app.generative_fill.loaded {
        refresh(&mut app.generative_fill);
    }
    let dim = Tokens::get(ui.ctx()).text_dim;
    let model = app.generative_fill.selected.clone().or_else(|| app.generative_fill.local.first().map(|m| m.name.clone()));
    let label = match &model {
        Some(name) => format!("Model: {name}"),
        None => "No model yet: open Models…".to_string(),
    };
    ui.label(RichText::new(label).size(12.0).color(dim));
    if crate::widgets::secondary_button(ui, tl!("Generate…"), 110.0).clicked()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "ai.generativeFill", json!({}))
    {
        app.generative_fill.status = e;
    }
    if crate::widgets::secondary_button(ui, tl!("Models…"), 110.0).clicked() {
        open(&mut app.generative_fill);
    }
    if !app.generative_fill.status.is_empty() {
        ui.label(RichText::new(&app.generative_fill.status).size(12.0).color(dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(repo: &str, file: &str) -> Installed {
        Installed { repo: repo.into(), files: vec![gf::ModelFile { name: file.into(), bytes: 1, sha256: "0".repeat(64) }], installed_at: 0 }
    }

    #[test]
    fn a_finished_search_replaces_the_results_and_frees_the_channel() {
        let ctx = Context::default();
        let (tx, rx) = channel();
        let mut t = Tasks { search_rx: Some(rx), ..Default::default() };
        tx.send(Ok(vec![HubModel { id: "owner/model".into(), downloads: 1, likes: 0, pipeline_tag: None }])).expect("send");
        poll(&mut t, &ctx);
        assert!(t.search_rx.is_none());
        assert!(t.searched);
        assert_eq!(t.results.len(), 1);
        assert_eq!(t.status, "1 model found");
    }

    #[test]
    fn a_failed_search_clears_old_results_and_shows_the_error() {
        let ctx = Context::default();
        let (tx, rx) = channel();
        let mut t = Tasks { search_rx: Some(rx), results: vec![HubModel { id: "old/one".into(), downloads: 0, likes: 0, pipeline_tag: None }], ..Default::default() };
        tx.send(Err("could not reach the model hub".into())).expect("send");
        poll(&mut t, &ctx);
        assert!(t.results.is_empty(), "stale results are not left on screen");
        assert_eq!(t.status, "could not reach the model hub");
    }

    #[test]
    fn a_failed_download_keeps_its_error_on_screen_and_frees_the_slot() {
        let ctx = Context::default();
        let (tx, rx) = channel();
        let mut t = Tasks { download_rx: Some(("owner/model".into(), rx)), progress: Some(Arc::new(gf::Progress::new())), ..Default::default() };
        tx.send(Err("checksum failed".into())).expect("send");
        poll(&mut t, &ctx);
        assert!(t.downloading().is_none(), "a second download can start");
        assert!(t.progress.is_none(), "the bar goes away");
        assert_eq!(t.failed.as_deref(), Some("checksum failed"), "the red message stays until the next attempt");
        assert_eq!(t.status, "Download failed: checksum failed");
    }

    #[test]
    fn a_cancelled_download_is_not_reported_as_a_failure() {
        let ctx = Context::default();
        let (tx, rx) = channel();
        let mut t = Tasks { download_rx: Some(("owner/model".into(), rx)), progress: Some(Arc::new(gf::Progress::new())), ..Default::default() };
        tx.send(Err("cancelled".into())).expect("send");
        poll(&mut t, &ctx);
        assert!(t.failed.is_none());
        assert!(t.progress.is_none());
        assert_eq!(t.status, "Download cancelled.");
    }

    #[test]
    fn a_download_thread_that_dies_is_reported_as_a_failure() {
        let ctx = Context::default();
        let (tx, rx) = channel::<Result<Installed, String>>();
        drop(tx);
        let mut t = Tasks { download_rx: Some(("owner/model".into(), rx)), progress: Some(Arc::new(gf::Progress::new())), ..Default::default() };
        poll(&mut t, &ctx);
        assert!(t.failed.is_some() && t.progress.is_none() && t.downloading().is_none());
    }

    #[test]
    fn a_second_download_waits_while_one_is_running() {
        let ctx = Context::default();
        let (_tx, rx) = channel();
        let mut t = Tasks { download_rx: Some(("owner/first".into(), rx)), ..Default::default() };
        start_download(&mut t, "owner/second", None, &ctx);
        assert_eq!(t.downloading(), Some("owner/first"));
        assert!(t.status.contains("wait"));
    }

    #[test]
    fn the_first_downloaded_model_is_picked_only_when_nothing_is_picked_yet() {
        let m = installed("Carve/LaMa-ONNX", "lama_fp32.onnx");
        assert_eq!(auto_pick(&None, &m).as_deref(), Some("Carve__LaMa-ONNX/lama_fp32.onnx"));
        assert_eq!(auto_pick(&Some("mine.onnx".into()), &m), None, "never overrides the user's choice");
    }

    #[test]
    fn the_default_model_name_matches_what_a_download_produces() {
        let m = installed(gf::DEFAULT_MODEL_REPO, gf::DEFAULT_MODEL_FILE);
        assert_eq!(auto_pick(&None, &m), Some(gf::default_model_name()));
    }

    #[test]
    fn model_labels_say_when_a_file_was_added_by_hand() {
        let mine = LocalModel { name: "mine.onnx".into(), bytes: 2_097_152, verified: false };
        let dl = LocalModel { name: "a__b/m.onnx".into(), bytes: 1_048_576, verified: true };
        assert_eq!(local_label(&mine), "mine.onnx  (2.0 MB, added by you)");
        assert_eq!(local_label(&dl), "a__b/m.onnx  (1.0 MB)");
    }
}
