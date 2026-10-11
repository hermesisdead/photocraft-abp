//! Generative Fill's model browser: a window to search Hugging Face for inpainting models,
//! download one, and remove installed ones. Opened from the Generative Fill tool's options bar
//! (Models…). Searches and downloads run on a background thread, so the canvas keeps responding
//! during a multi-gigabyte download; the thread reports back through a channel polled each frame.
//!
//! Everything here calls `photocraft_engine::generative_fill_cmds`, the same functions the
//! `ai.generativeFill.*` commands use, so the window and an agent see identical results.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use egui::{Context, RichText};
use photocraft_engine::generative_fill_cmds::{self as gf, HubModel, Installed};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Results shown per search.
const SEARCH_LIMIT: usize = 25;

/// The browser's state. Lives on the app (not in the saved UI state): it holds live channels.
#[derive(Default)]
pub struct Tasks {
    pub open: bool,
    pub query: String,
    pub results: Vec<HubModel>,
    pub searched: bool,
    pub installed: Vec<Installed>,
    pub status: String,
    search_rx: Option<Receiver<Result<Vec<HubModel>, String>>>,
    download_rx: Option<(String, Receiver<Result<Installed, String>>)>,
}

impl Tasks {
    fn downloading(&self) -> Option<&str> {
        self.download_rx.as_ref().map(|(repo, _)| repo.as_str())
    }
}

/// Opens the browser and refreshes the installed list.
pub fn open(t: &mut Tasks) {
    t.open = true;
    refresh_installed(t);
}

fn refresh_installed(t: &mut Tasks) {
    match gf::installed_models() {
        Ok(list) => t.installed = list,
        Err(e) => t.status = e.to_string(),
    }
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

fn start_download(t: &mut Tasks, repo: &str, ctx: &Context) {
    if t.download_rx.is_some() {
        t.status = "Another model is downloading; wait for it to finish".into();
        return;
    }
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    let owned = repo.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(gf::download_model(&owned).map_err(|e| e.to_string()));
        ctx.request_repaint();
    });
    t.download_rx = Some((repo.to_string(), rx));
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
            match result {
                Ok(_) => t.status = format!("Installed {repo}. Generate… uses the first installed model."),
                Err(e) => t.status = e,
            }
            refresh_installed(t);
        }
        Some((_, Err(TryRecvError::Disconnected))) => {
            t.download_rx = None;
            t.status = "The download stopped unexpectedly".into();
        }
        None => {}
    }
}

/// The browser window, drawn each frame while it is open.
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
        .default_width(480.0)
        .open(&mut open)
        .show(ctx, |ui| body(&mut app.generative_fill, ui, ctx));
    if !open {
        app.generative_fill.open = false;
    }
}

enum Action {
    Download(String),
    Remove(String),
}

fn body(t: &mut Tasks, ui: &mut egui::Ui, ctx: &Context) {
    let dim = Tokens::get(ctx).text_dim;
    ui.label(RichText::new(tl!("Inpainting models from Hugging Face. Each file is checked against the hub's SHA-256 before a model is installed.")).size(12.0).color(dim));
    ui.add_space(4.0);
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
    ui.separator();

    let mut action: Option<Action> = None;
    let downloading = t.downloading().map(str::to_string);
    egui::ScrollArea::vertical().id_salt("generative-fill-results").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
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
                        ui.label(RichText::new(tl!("Installed")).color(dim));
                    } else if downloading.as_deref() == Some(m.id.as_str()) {
                        ui.add(egui::Spinner::new());
                    } else if crate::widgets::secondary_button(ui, tl!("Download"), 90.0).clicked() {
                        action = Some(Action::Download(m.id.clone()));
                    }
                });
            });
            ui.separator();
        }
    });

    ui.add_space(4.0);
    ui.label(RichText::new(tl!("Installed")).strong());
    if t.installed.is_empty() {
        ui.label(RichText::new(tl!("No models installed yet.")).color(dim));
    }
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

    if !t.status.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(&t.status).size(12.0).color(dim));
    }

    match action {
        Some(Action::Download(repo)) => start_download(t, &repo, ctx),
        Some(Action::Remove(repo)) => match gf::remove_model(&repo) {
            Ok(_) => {
                t.status = format!("Removed {repo}");
                refresh_installed(t);
            }
            Err(e) => t.status = e.to_string(),
        },
        None => {}
    }
}

/// The Generative Fill tool's options bar: Generate… opens the pop-up, Models… the browser.
pub fn tool_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if crate::widgets::secondary_button(ui, tl!("Generate…"), 110.0).clicked()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "ai.generativeFill", json!({}))
    {
        app.generative_fill.status = e;
    }
    if crate::widgets::secondary_button(ui, tl!("Models…"), 110.0).clicked() {
        open(&mut app.generative_fill);
    }
    if !app.generative_fill.status.is_empty() {
        ui.label(RichText::new(&app.generative_fill.status).size(12.0).color(Tokens::get(ui.ctx()).text_dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_failed_download_reports_its_error_and_frees_the_slot() {
        let ctx = Context::default();
        let (tx, rx) = channel();
        let mut t = Tasks { download_rx: Some(("owner/model".into(), rx)), ..Default::default() };
        tx.send(Err("checksum failed".into())).expect("send");
        poll(&mut t, &ctx);
        assert!(t.downloading().is_none(), "a second download can start");
        assert_eq!(t.status, "checksum failed");
    }

    #[test]
    fn a_second_download_waits_while_one_is_running() {
        let ctx = Context::default();
        let (_tx, rx) = channel();
        let mut t = Tasks { download_rx: Some(("owner/first".into(), rx)), ..Default::default() };
        start_download(&mut t, "owner/second", &ctx);
        assert_eq!(t.downloading(), Some("owner/first"));
        assert!(t.status.contains("wait"));
    }
}
