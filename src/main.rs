use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Local};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

const ACCENT: Color32 = Color32::from_rgb(88, 155, 255);
const PANEL: Color32 = Color32::from_rgb(24, 29, 38);
const SURFACE: Color32 = Color32::from_rgb(31, 37, 48);
const MUTED: Color32 = Color32::from_rgb(151, 162, 180);
const PATH_INPUT_ID: &str = "location_input";

fn main() -> eframe::Result {
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("the embedded application icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CF File Manager")
            .with_app_id("cf-file-manager")
            .with_icon(app_icon)
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([780.0, 520.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "CF File Manager",
        options,
        Box::new(|creation_context| Ok(Box::new(FileManagerApp::new(creation_context)))),
    )
}

#[derive(Clone)]
struct FileEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_symlink: bool,
    size: u64,
    modified: Option<SystemTime>,
}

#[derive(Clone)]
enum DialogKind {
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
}

enum DialogAction {
    CreateFolder { parent: PathBuf, name: String },
    Rename { old_path: PathBuf, name: String },
    Delete(PathBuf),
}

struct FileManagerApp {
    current_dir: PathBuf,
    pinned_folders: Vec<PathBuf>,
    path_input: String,
    entries: Vec<FileEntry>,
    selected: Option<PathBuf>,
    search: String,
    show_hidden: bool,
    history: Vec<PathBuf>,
    history_index: usize,
    status: Option<String>,
    dialog: Option<DialogKind>,
    dialog_input: String,
    focus_location: bool,
}

impl FileManagerApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = SURFACE;
        visuals.widgets.noninteractive.bg_fill = PANEL;
        visuals.widgets.inactive.bg_fill = SURFACE;
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 58, 74);
        visuals.widgets.active.bg_fill = Color32::from_rgb(59, 91, 137);
        visuals.selection.bg_fill = ACCENT;
        creation_context.egui_ctx.set_visuals(visuals);

        let home =
            home_dir().unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let current_dir = fs::canonicalize(&home).unwrap_or(home);
        let mut app = Self {
            current_dir: current_dir.clone(),
            pinned_folders: load_pinned_folders(),
            path_input: current_dir.to_string_lossy().into_owned(),
            entries: Vec::new(),
            selected: None,
            search: String::new(),
            show_hidden: false,
            history: vec![current_dir],
            history_index: 0,
            status: None,
            dialog: None,
            dialog_input: String::new(),
            focus_location: false,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.status = None;
        let mut entries = Vec::new();

        match fs::read_dir(&self.current_dir) {
            Ok(directory) => {
                for item in directory {
                    let item = match item {
                        Ok(item) => item,
                        Err(error) => {
                            self.status = Some(format!("Could not read an entry: {error}"));
                            continue;
                        }
                    };

                    let name = item.file_name().to_string_lossy().into_owned();
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }

                    let file_type = match item.file_type() {
                        Ok(file_type) => file_type,
                        Err(_) => continue,
                    };
                    let metadata = item.metadata().ok();
                    entries.push(FileEntry {
                        path: item.path(),
                        name,
                        is_dir: metadata.as_ref().is_some_and(|metadata| metadata.is_dir()),
                        is_symlink: file_type.is_symlink(),
                        size: metadata.as_ref().map_or(0, |metadata| metadata.len()),
                        modified: metadata.and_then(|metadata| metadata.modified().ok()),
                    });
                }
            }
            Err(error) => self.status = Some(format!("Could not open this folder: {error}")),
        }

        entries.sort_by(|left, right| {
            right
                .is_dir
                .cmp(&left.is_dir)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !entries.iter().any(|entry| &entry.path == selected))
        {
            self.selected = None;
        }
        self.entries = entries;
    }

    fn navigate_to(&mut self, path: &Path, add_to_history: bool) {
        match fs::canonicalize(path) {
            Ok(path) if path.is_dir() => {
                if add_to_history && path != self.current_dir {
                    self.history.truncate(self.history_index.saturating_add(1));
                    self.history.push(path.clone());
                    self.history_index = self.history.len() - 1;
                }
                self.current_dir = path;
                self.path_input = self.current_dir.to_string_lossy().into_owned();
                self.selected = None;
                self.refresh();
            }
            Ok(_) => self.status = Some("That path is a file, not a folder.".to_owned()),
            Err(error) => self.status = Some(format!("Could not open that folder: {error}")),
        }
    }

    fn navigate_history(&mut self, index: usize) {
        if let Some(path) = self.history.get(index).cloned() {
            self.history_index = index;
            self.current_dir = path;
            self.path_input = self.current_dir.to_string_lossy().into_owned();
            self.selected = None;
            self.refresh();
        }
    }

    fn go_back(&mut self) {
        if self.history_index > 0 {
            self.navigate_history(self.history_index - 1);
        }
    }

    fn go_forward(&mut self) {
        if self.history_index + 1 < self.history.len() {
            self.navigate_history(self.history_index + 1);
        }
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) {
            self.navigate_to(&parent, true);
        }
    }

    fn is_pinned(&self, path: &Path) -> bool {
        self.pinned_folders.iter().any(|pinned| pinned == path)
    }

    fn pin_folder(&mut self, path: &Path) {
        let canonical_path = match fs::canonicalize(path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                self.status = Some("Only folders can be pinned.".to_owned());
                return;
            }
            Err(error) => {
                self.status = Some(format!("Could not pin this folder: {error}"));
                return;
            }
        };

        if self.is_pinned(&canonical_path) {
            self.status = Some("This folder is already pinned.".to_owned());
            return;
        }

        self.pinned_folders.push(canonical_path);
        self.status = Some(match self.persist_pinned_folders() {
            Ok(()) => "Folder added to Pinned.".to_owned(),
            Err(error) => format!("Folder pinned for this session, but {error}"),
        });
    }

    fn unpin_folder(&mut self, path: &Path) {
        let previous_len = self.pinned_folders.len();
        self.pinned_folders.retain(|pinned| pinned != path);
        if self.pinned_folders.len() != previous_len {
            self.status = Some(match self.persist_pinned_folders() {
                Ok(()) => "Folder removed from Pinned.".to_owned(),
                Err(error) => format!("Folder unpinned for this session, but {error}"),
            });
        }
    }

    fn persist_pinned_folders(&self) -> Result<(), String> {
        pinned_folders_file().and_then(|file| {
            let parent = file
                .parent()
                .ok_or_else(|| "Could not find the settings folder.".to_owned())?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create the settings folder: {error}"))?;
            let contents = self
                .pinned_folders
                .iter()
                .map(|path| escape_config_path(&path.to_string_lossy()))
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(file, contents)
                .map_err(|error| format!("Could not save pinned folders: {error}"))
        })
    }

    fn move_item_to(&mut self, source: &Path, destination_dir: &Path) {
        if !destination_dir.is_dir() {
            self.status = Some("Drop items onto a folder.".to_owned());
            return;
        }
        if source.parent() == Some(destination_dir) {
            self.status = Some("This item is already in that folder.".to_owned());
            return;
        }
        if source.is_dir() && destination_dir.starts_with(source) {
            self.status = Some("A folder cannot be moved inside itself.".to_owned());
            return;
        }

        let Some(name) = source.file_name() else {
            self.status = Some("This item cannot be moved.".to_owned());
            return;
        };
        let target = destination_dir.join(name);
        if fs::symlink_metadata(&target).is_ok() {
            self.status = Some(format!("{} already exists there.", name.to_string_lossy()));
            return;
        }

        let moving_directory = source.is_dir();
        match fs::rename(source, &target) {
            Ok(()) => {
                let mut pin_save_result = None;
                if moving_directory {
                    let mut changed_pins = false;
                    for pinned in &mut self.pinned_folders {
                        if let Ok(relative) = pinned.strip_prefix(source) {
                            *pinned = target.join(relative);
                            changed_pins = true;
                        }
                    }
                    if changed_pins {
                        pin_save_result = Some(self.persist_pinned_folders());
                    }
                }
                if self.selected.as_deref() == Some(source) {
                    self.selected = Some(target);
                }
                self.refresh();
                self.status = Some(match pin_save_result {
                    Some(Ok(())) => "Item moved and pinned paths updated.".to_owned(),
                    Some(Err(error)) => {
                        format!("Item moved, but pinned paths could not be saved: {error}")
                    }
                    None => "Item moved.".to_owned(),
                });
            }
            Err(error) => self.status = Some(format!("Could not move item: {error}")),
        }
    }

    fn copy_dropped_items(&mut self, sources: &[PathBuf]) {
        let mut copied = 0;
        let mut problems = Vec::new();

        for source in sources {
            let Some(name) = source.file_name() else {
                problems.push(format!("{} has no file name", source.display()));
                continue;
            };
            let target = self.current_dir.join(name);
            if source == &target {
                continue;
            }
            if fs::symlink_metadata(&target).is_ok() {
                problems.push(format!("{} already exists", name.to_string_lossy()));
                continue;
            }

            let canonical_source = fs::canonicalize(source).unwrap_or_else(|_| source.clone());
            if canonical_source.is_dir() && self.current_dir.starts_with(&canonical_source) {
                problems.push(format!(
                    "{} cannot be copied into itself",
                    name.to_string_lossy()
                ));
                continue;
            }

            match copy_item(source, &target) {
                Ok(()) => copied += 1,
                Err(error) => {
                    if fs::symlink_metadata(&target).is_ok() {
                        let _ = if target.is_dir() {
                            fs::remove_dir_all(&target)
                        } else {
                            fs::remove_file(&target)
                        };
                    }
                    problems.push(format!("{}: {error}", name.to_string_lossy()));
                }
            }
        }

        self.refresh();
        self.status = Some(if problems.is_empty() {
            if copied == 1 {
                "Copied 1 item into this folder.".to_owned()
            } else {
                format!("Copied {copied} items into this folder.")
            }
        } else if copied == 0 {
            format!("Nothing copied: {}", problems.join("; "))
        } else {
            format!("Copied {copied} items. Skipped: {}", problems.join("; "))
        });
    }

    fn open_selected(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        if path.is_dir() {
            self.navigate_to(&path, true);
        } else if let Err(error) = open::that(&path) {
            self.status = Some(format!("Could not open file: {error}"));
        } else {
            self.status = Some("Opened with the system's default app.".to_owned());
        }
    }

    fn open_new_folder_dialog(&mut self) {
        self.dialog_input.clear();
        self.dialog = Some(DialogKind::NewFolder(self.current_dir.clone()));
    }

    fn open_rename_dialog(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        self.dialog_input = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.dialog = Some(DialogKind::Rename(path));
    }

    fn open_delete_dialog(&mut self) {
        if let Some(path) = self.selected.clone() {
            self.dialog = Some(DialogKind::Delete(path));
        }
    }

    fn apply_dialog_action(&mut self, action: DialogAction) {
        match action {
            DialogAction::CreateFolder { parent, name } => {
                let target = match child_path(&parent, &name) {
                    Ok(target) => target,
                    Err(message) => {
                        self.status = Some(message);
                        return;
                    }
                };
                match fs::create_dir(&target) {
                    Ok(()) => {
                        self.selected = Some(target);
                        self.refresh();
                        self.status = Some("Folder created.".to_owned());
                    }
                    Err(error) => self.status = Some(format!("Could not create folder: {error}")),
                }
            }
            DialogAction::Rename { old_path, name } => {
                let renamed_directory = old_path.is_dir();
                let parent = old_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.current_dir.clone());
                let target = match child_path(&parent, &name) {
                    Ok(target) => target,
                    Err(message) => {
                        self.status = Some(message);
                        return;
                    }
                };
                if target == old_path {
                    self.status = None;
                    return;
                }
                match fs::rename(&old_path, &target) {
                    Ok(()) => {
                        let mut pin_save_result = None;
                        if renamed_directory {
                            let mut changed_pins = false;
                            for pinned in &mut self.pinned_folders {
                                if let Ok(relative) = pinned.strip_prefix(&old_path) {
                                    *pinned = target.join(relative);
                                    changed_pins = true;
                                }
                            }
                            if changed_pins {
                                pin_save_result = Some(self.persist_pinned_folders());
                            }
                        }
                        self.selected = Some(target);
                        self.refresh();
                        self.status = Some(match pin_save_result {
                            Some(Ok(())) => "Item renamed and pinned paths updated.".to_owned(),
                            Some(Err(error)) => format!(
                                "Item renamed, but pinned paths could not be saved: {error}"
                            ),
                            None => "Item renamed.".to_owned(),
                        });
                    }
                    Err(error) => self.status = Some(format!("Could not rename item: {error}")),
                }
            }
            DialogAction::Delete(path) => {
                let result = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                match result {
                    Ok(()) => {
                        let previous_len = self.pinned_folders.len();
                        self.pinned_folders
                            .retain(|pinned| !pinned.starts_with(&path));
                        let pin_status = if self.pinned_folders.len() != previous_len {
                            self.persist_pinned_folders().err().map(|error| {
                                format!(
                                    "Item deleted, but pinned folders could not be saved: {error}"
                                )
                            })
                        } else {
                            None
                        };
                        self.selected = None;
                        self.refresh();
                        self.status =
                            Some(pin_status.unwrap_or_else(|| "Item deleted.".to_owned()));
                    }
                    Err(error) => self.status = Some(format!("Could not delete item: {error}")),
                }
            }
        }
    }

    fn filtered_entries(&self) -> Vec<FileEntry> {
        let query = self.search.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut requested_path = None;
        let mut refresh = false;
        let mut new_folder = false;
        let mut rename = false;
        let mut delete = false;
        let mut open = false;
        let mut hidden_changed = false;
        let can_go_back = self.history_index > 0;
        let can_go_forward = self.history_index + 1 < self.history.len();
        let has_selection = self.selected.is_some();

        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::default()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("CF File Manager").strong());
                    ui.add_space(12.0);
                    if icon_button(ui, Icon::Back, "Back", "Alt+Left", 64.0, can_go_back).clicked()
                    {
                        self.go_back();
                    }
                    if icon_button(
                        ui,
                        Icon::Forward,
                        "Forward",
                        "Alt+Right",
                        78.0,
                        can_go_forward,
                    )
                    .clicked()
                    {
                        self.go_forward();
                    }
                    if icon_button(ui, Icon::Up, "Up", "Open parent folder", 54.0, true).clicked() {
                        self.go_up();
                    }
                    if icon_button(ui, Icon::Refresh, "Refresh", "Ctrl+R", 78.0, true).clicked() {
                        refresh = true;
                    }

                    let path_width = (ui.available_width() - 48.0).max(160.0);
                    let response = ui.add_sized(
                        [path_width, 30.0],
                        egui::TextEdit::singleline(&mut self.path_input)
                            .id(egui::Id::new(PATH_INPUT_ID))
                            .hint_text("Enter a folder path"),
                    );
                    response.clone().on_hover_text("Location (Ctrl+L)");
                    if self.focus_location {
                        response.request_focus();
                        self.focus_location = false;
                    }
                    let enter_pressed = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if icon_button(ui, Icon::Open, "Go", "Open this location", 54.0, true).clicked()
                        || enter_pressed
                    {
                        let entered_path = PathBuf::from(self.path_input.clone());
                        requested_path = Some(if entered_path.is_absolute() {
                            entered_path
                        } else {
                            self.current_dir.join(entered_path)
                        });
                    }
                });

                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        draw_icon(ui, Icon::Search, MUTED, 16.0);
                        ui.add_sized(
                            [250.0, 29.0],
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search this folder"),
                        );
                    });
                    let hidden_toggle = ui
                        .checkbox(&mut self.show_hidden, "Show hidden dotfiles")
                        .on_hover_text("Include names that start with a dot");
                    hidden_changed = hidden_toggle.changed();
                    if icon_button(
                        ui,
                        Icon::FolderPlus,
                        "New folder",
                        "Ctrl+Shift+N",
                        112.0,
                        true,
                    )
                    .clicked()
                    {
                        new_folder = true;
                    }
                    if icon_button(
                        ui,
                        Icon::Rename,
                        "Rename",
                        "Rename selected item",
                        82.0,
                        has_selection,
                    )
                    .clicked()
                    {
                        rename = true;
                    }
                    if icon_button(
                        ui,
                        Icon::Delete,
                        "Delete",
                        "Delete (confirm)",
                        76.0,
                        has_selection,
                    )
                    .clicked()
                    {
                        delete = true;
                    }
                    if icon_button(
                        ui,
                        Icon::Open,
                        "Open",
                        "Open selected item",
                        68.0,
                        has_selection,
                    )
                    .clicked()
                    {
                        open = true;
                    }
                });
            });

        if refresh || hidden_changed {
            self.refresh();
        }
        if new_folder {
            self.open_new_folder_dialog();
        }
        if rename {
            self.open_rename_dialog();
        }
        if delete {
            self.open_delete_dialog();
        }
        if open {
            self.open_selected();
        }
        if let Some(path) = requested_path {
            self.navigate_to(&path, true);
        }
    }

    fn show_sidebar(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut requested_path = None;
        let mut pin_current = false;
        let mut remove_pinned = None;
        let pinned_folders = self.pinned_folders.clone();
        egui::Panel::left("places")
            .resizable(true)
            .default_size(205.0)
            .min_size(165.0)
            .max_size(280.0)
            .frame(
                egui::Frame::default()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 14)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_scroll")
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("PLACES").small().strong().color(MUTED));
                        ui.add_space(8.0);
                        if let Some(home) = home_dir() {
                            for (label, path) in standard_places(&home) {
                                let active = self.current_dir == path;
                                if place_button(ui, place_icon(label), label, active).clicked() {
                                    requested_path = Some(path);
                                }
                            }
                        }

                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("PINNED").small().strong().color(MUTED));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let already_pinned = self.is_pinned(&self.current_dir);
                                    if ui
                                        .add_enabled(!already_pinned, egui::Button::new("+"))
                                        .on_hover_text("Pin the current folder")
                                        .clicked()
                                    {
                                        pin_current = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(8.0);
                        for path in &pinned_folders {
                            let label = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned());
                            let active = self.current_dir == *path;
                            ui.horizontal(|ui| {
                                let button_width = (ui.available_width() - 26.0).max(80.0);
                                if place_button_sized(
                                    ui,
                                    Icon::Folder,
                                    &label,
                                    active,
                                    button_width,
                                )
                                .on_hover_text(path.display().to_string())
                                .clicked()
                                {
                                    requested_path = Some(path.clone());
                                }
                                if ui
                                    .add_sized([22.0, 24.0], egui::Button::new("×"))
                                    .on_hover_text("Remove from Pinned")
                                    .clicked()
                                {
                                    remove_pinned = Some(path.clone());
                                }
                            });
                        }

                        ui.add_space(18.0);
                        ui.label(
                            egui::RichText::new("FILESYSTEM")
                                .small()
                                .strong()
                                .color(MUTED),
                        );
                        ui.add_space(8.0);
                        for root in filesystem_roots() {
                            let active = self.current_dir == root;
                            let label = root_label(&root);
                            if place_button(ui, Icon::Drive, &label, active).clicked() {
                                requested_path = Some(root);
                            }
                        }
                    });
            });

        if pin_current {
            let current_dir = self.current_dir.clone();
            self.pin_folder(&current_dir);
        }
        if let Some(path) = remove_pinned {
            self.unpin_folder(&path);
        }
        requested_path
    }

    fn show_file_list(&mut self, ui: &mut egui::Ui) {
        let mut clicked_path = None;
        let mut opened_path = None;
        let mut pin_action = None;
        let mut drop_action = None;
        let entries = self.filtered_entries();
        let selected = self.selected.clone();

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(Color32::from_rgb(19, 23, 30))
                    .inner_margin(egui::Margin::symmetric(18, 14)),
            )
            .show(ui, |ui| {
                let current_folder_drop_target = ui.interact(
                    ui.max_rect(),
                    egui::Id::new("current_folder_drop_target"),
                    Sense::hover(),
                );
                let folder_name = self
                    .current_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.current_dir.to_string_lossy().into_owned());
                ui.heading(folder_name);
                ui.label(
                    egui::RichText::new(self.current_dir.to_string_lossy())
                        .small()
                        .color(MUTED),
                );
                if ui.input(|input| !input.raw.hovered_files.is_empty()) {
                    ui.colored_label(ACCENT, "Drop files to copy them into this folder");
                }
                ui.add_space(12.0);

                let widths = column_widths(ui.available_width());
                draw_table_header(ui, widths);
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if entries.is_empty() {
                        let message = if self.status.is_some() {
                            "This folder could not be listed."
                        } else if self.search.trim().is_empty() {
                            "This folder is empty."
                        } else {
                            "No files match your search."
                        };
                        ui.add_space(24.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new(message).color(MUTED));
                        });
                    }

                    for entry in &entries {
                        let is_selected = selected.as_ref() == Some(&entry.path);
                        let response = draw_file_row(ui, entry, is_selected, widths);
                        response.dnd_set_drag_payload(entry.path.clone());
                        if response.clicked() {
                            clicked_path = Some(entry.path.clone());
                        }
                        if response.double_clicked() {
                            opened_path = Some(entry.path.clone());
                        }
                        if entry.is_dir {
                            if response.dnd_hover_payload::<PathBuf>().is_some() {
                                ui.painter().rect_stroke(
                                    response.rect,
                                    egui::CornerRadius::same(5),
                                    egui::Stroke::new(2.0, ACCENT),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if let Some(source) = response.dnd_release_payload::<PathBuf>() {
                                drop_action = Some((source.as_ref().clone(), entry.path.clone()));
                            }
                        }
                        if entry.is_dir {
                            response.context_menu(|ui| {
                                let pinned = self.is_pinned(&entry.path);
                                let label = if pinned {
                                    "Remove from Pinned"
                                } else {
                                    "Pin folder"
                                };
                                if ui.button(label).clicked() {
                                    pin_action = Some((entry.path.clone(), pinned));
                                    ui.close();
                                }
                            });
                        }
                    }
                });
                if let Some(source) = current_folder_drop_target.dnd_release_payload::<PathBuf>() {
                    drop_action = Some((source.as_ref().clone(), self.current_dir.clone()));
                }
            });

        if let Some(path) = clicked_path {
            self.selected = Some(path);
        }
        if let Some(path) = opened_path {
            if path.is_dir() {
                self.navigate_to(&path, true);
            } else if let Err(error) = open::that(&path) {
                self.status = Some(format!("Could not open file: {error}"));
            } else {
                self.status = Some("Opened with the system's default app.".to_owned());
            }
        }
        if let Some((path, was_pinned)) = pin_action {
            if was_pinned {
                self.unpin_folder(&path);
            } else {
                self.pin_folder(&path);
            }
        }
        if let Some((source, destination)) = drop_action {
            self.move_item_to(&source, &destination);
        }
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        let query = self.search.trim().to_lowercase();
        let visible_count = self
            .entries
            .iter()
            .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .count();
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::default()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(status) = &self.status {
                        ui.colored_label(Color32::from_rgb(255, 174, 120), status);
                    } else {
                        ui.label(format!("{visible_count} items"));
                    }
                    if let Some(path) = &self.selected {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        });
                    }
                });
            });
    }

    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.dialog.clone() else {
            return;
        };

        let title = match &kind {
            DialogKind::NewFolder(_) => "Create folder",
            DialogKind::Rename(_) => "Rename item",
            DialogKind::Delete(_) => "Delete item?",
        };
        let mut is_open = true;
        let mut cancel_requested = false;
        let mut action = None;
        egui::Window::new(title)
            .id(egui::Id::new("file_manager_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut is_open)
            .show(ctx, |ui| match &kind {
                DialogKind::NewFolder(parent) => {
                    ui.label(format!("Create a folder in {}", parent.display()));
                    let response = ui.add_sized(
                        [330.0, 30.0],
                        egui::TextEdit::singleline(&mut self.dialog_input).hint_text("Folder name"),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let create = ui.button("Create").clicked()
                            || (response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                        if create {
                            action = Some(DialogAction::CreateFolder {
                                parent: parent.clone(),
                                name: self.dialog_input.clone(),
                            });
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_requested = true;
                        }
                    });
                }
                DialogKind::Rename(path) => {
                    ui.label(format!("Rename {}", path.display()));
                    let response = ui.add_sized(
                        [330.0, 30.0],
                        egui::TextEdit::singleline(&mut self.dialog_input),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let save = ui.button("Save").clicked()
                            || (response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                        if save {
                            action = Some(DialogAction::Rename {
                                old_path: path.clone(),
                                name: self.dialog_input.clone(),
                            });
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_requested = true;
                        }
                    });
                }
                DialogKind::Delete(path) => {
                    ui.label("This will permanently remove the selected item and its contents.");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(path.display().to_string()).color(MUTED));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(egui::RichText::new("Delete").color(Color32::WHITE))
                            .clicked()
                        {
                            action = Some(DialogAction::Delete(path.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_requested = true;
                        }
                    });
                }
            });

        if cancel_requested {
            is_open = false;
        }
        if !is_open || action.is_some() {
            self.dialog = None;
        }
        if let Some(action) = action {
            self.apply_dialog_action(action);
        }
    }
}

impl eframe::App for FileManagerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped_files = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !dropped_files.is_empty() {
            self.copy_dropped_items(&dropped_files);
        }
        let shortcuts_enabled = !ctx.egui_wants_keyboard_input();
        if shortcuts_enabled {
            let (back, forward, refresh, focus_location, new_folder, delete) = ctx.input(|input| {
                (
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft),
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight),
                    input.modifiers.ctrl && input.key_pressed(egui::Key::R),
                    input.modifiers.ctrl && input.key_pressed(egui::Key::L),
                    input.modifiers.ctrl
                        && input.modifiers.shift
                        && input.key_pressed(egui::Key::N),
                    input.key_pressed(egui::Key::Delete),
                )
            });
            if back {
                self.go_back();
            }
            if forward {
                self.go_forward();
            }
            if refresh {
                self.refresh();
            }
            if focus_location {
                self.focus_location = true;
            }
            if new_folder {
                self.open_new_folder_dialog();
            }
            if delete {
                self.open_delete_dialog();
            }
        }

        self.show_toolbar(ui);
        if let Some(path) = self.show_sidebar(ui) {
            self.navigate_to(&path, true);
        }
        self.show_status(ui);
        self.show_file_list(ui);
        self.show_dialog(&ctx);
    }
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");
    home.map(PathBuf::from)
}

fn pinned_folders_file() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let config_dir = env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("CF File Manager"))
        .ok_or_else(|| "Could not find the application data folder.".to_owned())?;

    #[cfg(target_os = "macos")]
    let config_dir = home_dir()
        .map(|path| {
            path.join("Library")
                .join("Application Support")
                .join("CF File Manager")
        })
        .ok_or_else(|| "Could not find the application data folder.".to_owned())?;

    #[cfg(all(unix, not(target_os = "macos")))]
    let config_dir = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|path| path.join(".config")))
        .map(|path| path.join("cf-file-manager"))
        .ok_or_else(|| "Could not find the application data folder.".to_owned())?;

    Ok(config_dir.join("pinned-folders.txt"))
}

fn load_pinned_folders() -> Vec<PathBuf> {
    let Ok(file) = pinned_folders_file() else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(file) else {
        return Vec::new();
    };

    let mut folders = Vec::new();
    for line in contents.lines() {
        let path = PathBuf::from(unescape_config_path(line));
        let Ok(path) = fs::canonicalize(path) else {
            continue;
        };
        if path.is_dir() && !folders.contains(&path) {
            folders.push(path);
        }
    }
    folders
}

fn escape_config_path(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn unescape_config_path(path: &str) -> String {
    let mut unescaped = String::with_capacity(path.len());
    let mut characters = path.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => unescaped.push('\\'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            Some(other) => {
                unescaped.push('\\');
                unescaped.push(other);
            }
            None => unescaped.push('\\'),
        }
    }
    unescaped
}

fn copy_item(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "symbolic links cannot be copied by dropping them",
        ));
    }

    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_item(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn standard_places(home: &Path) -> Vec<(&'static str, PathBuf)> {
    [
        ("Home", home.to_path_buf()),
        ("Desktop", home.join("Desktop")),
        ("Documents", home.join("Documents")),
        ("Downloads", home.join("Downloads")),
        ("Pictures", home.join("Pictures")),
        ("Music", home.join("Music")),
        ("Videos", home.join("Videos")),
    ]
    .into_iter()
    .filter(|(_, path)| path.is_dir())
    .collect()
}

fn filesystem_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        (b'C'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", char::from(letter))))
            .filter(|path| path.is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

fn root_label(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().into_owned()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        "File system".to_owned()
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Back,
    Forward,
    Up,
    Refresh,
    Search,
    FolderPlus,
    Rename,
    Delete,
    Open,
    Folder,
    File,
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    Drive,
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    width: f32,
    enabled: bool,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(Vec2::new(width, 29.0)),
    );
    let rect = response.rect;
    let tint = if enabled {
        ui.visuals().text_color()
    } else {
        MUTED
    };
    paint_icon(
        ui.painter(),
        icon,
        Rect::from_min_size(rect.min + Vec2::new(7.0, 7.0), Vec2::splat(16.0)),
        tint,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        tint,
    );
    response.on_hover_text(hint)
}

fn place_button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    place_button_sized(ui, icon, label, selected, width)
}

fn place_button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 27.0), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(4),
            if selected {
                Color32::from_rgb(41, 73, 117)
            } else {
                Color32::from_rgb(33, 41, 54)
            },
        );
    }
    paint_icon(
        ui.painter(),
        icon,
        Rect::from_min_size(rect.min + Vec2::new(9.0, 6.0), Vec2::splat(15.0)),
        if selected { Color32::WHITE } else { MUTED },
    );
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        if selected {
            Color32::WHITE
        } else {
            Color32::from_rgb(220, 225, 234)
        },
    );
    response
}

fn place_icon(label: &str) -> Icon {
    match label {
        "Home" => Icon::Home,
        "Desktop" => Icon::Desktop,
        "Documents" => Icon::Documents,
        "Downloads" => Icon::Downloads,
        "Pictures" => Icon::Pictures,
        "Music" => Icon::Music,
        "Videos" => Icon::Videos,
        _ => Icon::Folder,
    }
}

fn draw_icon(ui: &mut egui::Ui, icon: Icon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), icon, rect, color);
}

fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let s = egui::Stroke::new(1.6, color);
    let x = rect.left();
    let y = rect.top();
    let w = rect.width();
    let h = rect.height();
    let line = |a, b| painter.line_segment([a, b], s);
    match icon {
        Icon::Back => {
            line(
                Pos2::new(x + w * 0.82, y + h * 0.5),
                Pos2::new(x + w * 0.18, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.43, y + h * 0.23),
                Pos2::new(x + w * 0.18, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.18, y + h * 0.5),
                Pos2::new(x + w * 0.43, y + h * 0.77),
            );
        }
        Icon::Forward => {
            line(
                Pos2::new(x + w * 0.18, y + h * 0.5),
                Pos2::new(x + w * 0.82, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.57, y + h * 0.23),
                Pos2::new(x + w * 0.82, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.82, y + h * 0.5),
                Pos2::new(x + w * 0.57, y + h * 0.77),
            );
        }
        Icon::Up => {
            line(
                Pos2::new(x + w * 0.5, y + h * 0.82),
                Pos2::new(x + w * 0.5, y + h * 0.18),
            );
            line(
                Pos2::new(x + w * 0.23, y + h * 0.43),
                Pos2::new(x + w * 0.5, y + h * 0.18),
            );
            line(
                Pos2::new(x + w * 0.5, y + h * 0.18),
                Pos2::new(x + w * 0.77, y + h * 0.43),
            );
        }
        Icon::Refresh => {
            painter.circle_stroke(rect.center(), w * 0.34, s);
            line(
                Pos2::new(x + w * 0.67, y + h * 0.1),
                Pos2::new(x + w * 0.9, y + h * 0.12),
            );
            line(
                Pos2::new(x + w * 0.9, y + h * 0.12),
                Pos2::new(x + w * 0.85, y + h * 0.35),
            );
        }
        Icon::Search => {
            painter.circle_stroke(Pos2::new(x + w * 0.42, y + h * 0.42), w * 0.29, s);
            line(
                Pos2::new(x + w * 0.63, y + h * 0.63),
                Pos2::new(x + w * 0.92, y + h * 0.92),
            );
        }
        Icon::Folder | Icon::FolderPlus | Icon::Downloads => {
            let points = [
                Pos2::new(x + w * 0.10, y + h * 0.32),
                Pos2::new(x + w * 0.10, y + h * 0.18),
                Pos2::new(x + w * 0.42, y + h * 0.18),
                Pos2::new(x + w * 0.42, y + h * 0.32),
                Pos2::new(x + w * 0.90, y + h * 0.32),
                Pos2::new(x + w * 0.90, y + h * 0.84),
                Pos2::new(x + w * 0.10, y + h * 0.84),
                Pos2::new(x + w * 0.10, y + h * 0.32),
            ];
            painter.add(egui::Shape::line(points.to_vec(), s));
            if matches!(icon, Icon::FolderPlus) {
                line(
                    Pos2::new(x + w * 0.5, y + h * 0.5),
                    Pos2::new(x + w * 0.5, y + h * 0.76),
                );
                line(
                    Pos2::new(x + w * 0.37, y + h * 0.63),
                    Pos2::new(x + w * 0.63, y + h * 0.63),
                );
            }
            if matches!(icon, Icon::Downloads) {
                line(
                    Pos2::new(x + w * 0.5, y + h * 0.44),
                    Pos2::new(x + w * 0.5, y + h * 0.68),
                );
                line(
                    Pos2::new(x + w * 0.38, y + h * 0.57),
                    Pos2::new(x + w * 0.5, y + h * 0.69),
                );
                line(
                    Pos2::new(x + w * 0.62, y + h * 0.57),
                    Pos2::new(x + w * 0.5, y + h * 0.69),
                );
            }
        }
        Icon::File | Icon::Documents => {
            let page = [
                Pos2::new(x + w * 0.22, y + h * 0.08),
                Pos2::new(x + w * 0.60, y + h * 0.08),
                Pos2::new(x + w * 0.80, y + h * 0.28),
                Pos2::new(x + w * 0.80, y + h * 0.92),
                Pos2::new(x + w * 0.22, y + h * 0.92),
                Pos2::new(x + w * 0.22, y + h * 0.08),
            ];
            painter.add(egui::Shape::line(page.to_vec(), s));
            line(
                Pos2::new(x + w * 0.60, y + h * 0.08),
                Pos2::new(x + w * 0.60, y + h * 0.28),
            );
            line(
                Pos2::new(x + w * 0.60, y + h * 0.28),
                Pos2::new(x + w * 0.80, y + h * 0.28),
            );
            line(
                Pos2::new(x + w * 0.32, y + h * 0.48),
                Pos2::new(x + w * 0.68, y + h * 0.48),
            );
            line(
                Pos2::new(x + w * 0.32, y + h * 0.64),
                Pos2::new(x + w * 0.68, y + h * 0.64),
            );
            if matches!(icon, Icon::Documents) {
                line(
                    Pos2::new(x + w * 0.32, y + h * 0.78),
                    Pos2::new(x + w * 0.58, y + h * 0.78),
                );
            }
        }
        Icon::Rename => {
            let pencil = [
                Pos2::new(x + w * 0.12, y + h * 0.88),
                Pos2::new(x + w * 0.25, y + h * 0.84),
                Pos2::new(x + w * 0.78, y + h * 0.30),
                Pos2::new(x + w * 0.65, y + h * 0.17),
                Pos2::new(x + w * 0.16, y + h * 0.72),
                Pos2::new(x + w * 0.12, y + h * 0.88),
            ];
            painter.add(egui::Shape::line(pencil.to_vec(), s));
        }
        Icon::Delete => {
            line(
                Pos2::new(x + w * 0.18, y + h * 0.30),
                Pos2::new(x + w * 0.82, y + h * 0.30),
            );
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.27, y + h * 0.36),
                    Pos2::new(x + w * 0.73, y + h * 0.88),
                ),
                egui::CornerRadius::ZERO,
                s,
                egui::StrokeKind::Inside,
            );
            line(
                Pos2::new(x + w * 0.40, y + h * 0.16),
                Pos2::new(x + w * 0.60, y + h * 0.16),
            );
            line(
                Pos2::new(x + w * 0.45, y + h * 0.16),
                Pos2::new(x + w * 0.45, y + h * 0.27),
            );
            line(
                Pos2::new(x + w * 0.55, y + h * 0.16),
                Pos2::new(x + w * 0.55, y + h * 0.27),
            );
            line(
                Pos2::new(x + w * 0.43, y + h * 0.48),
                Pos2::new(x + w * 0.43, y + h * 0.76),
            );
            line(
                Pos2::new(x + w * 0.57, y + h * 0.48),
                Pos2::new(x + w * 0.57, y + h * 0.76),
            );
        }
        Icon::Open => {
            line(
                Pos2::new(x + w * 0.16, y + h * 0.5),
                Pos2::new(x + w * 0.84, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.58, y + h * 0.24),
                Pos2::new(x + w * 0.84, y + h * 0.5),
            );
            line(
                Pos2::new(x + w * 0.58, y + h * 0.76),
                Pos2::new(x + w * 0.84, y + h * 0.5),
            );
        }
        Icon::Home => {
            let house = [
                Pos2::new(x + w * 0.10, y + h * 0.48),
                Pos2::new(x + w * 0.50, y + h * 0.14),
                Pos2::new(x + w * 0.90, y + h * 0.48),
                Pos2::new(x + w * 0.76, y + h * 0.48),
                Pos2::new(x + w * 0.76, y + h * 0.88),
                Pos2::new(x + w * 0.24, y + h * 0.88),
                Pos2::new(x + w * 0.24, y + h * 0.48),
                Pos2::new(x + w * 0.10, y + h * 0.48),
            ];
            painter.add(egui::Shape::line(house.to_vec(), s));
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.43, y + h * 0.64),
                    Pos2::new(x + w * 0.57, y + h * 0.88),
                ),
                egui::CornerRadius::ZERO,
                s,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Desktop => {
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.08, y + h * 0.12),
                    Pos2::new(x + w * 0.92, y + h * 0.68),
                ),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
            line(
                Pos2::new(x + w * 0.5, y + h * 0.68),
                Pos2::new(x + w * 0.5, y + h * 0.86),
            );
            line(
                Pos2::new(x + w * 0.3, y + h * 0.88),
                Pos2::new(x + w * 0.7, y + h * 0.88),
            );
        }
        Icon::Pictures => {
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.1, y + h * 0.12),
                    Pos2::new(x + w * 0.9, y + h * 0.88),
                ),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
            painter.circle_filled(Pos2::new(x + w * 0.68, y + h * 0.34), w * 0.08, color);
            line(
                Pos2::new(x + w * 0.18, y + h * 0.75),
                Pos2::new(x + w * 0.43, y + h * 0.48),
            );
            line(
                Pos2::new(x + w * 0.43, y + h * 0.48),
                Pos2::new(x + w * 0.82, y + h * 0.75),
            );
        }
        Icon::Music => {
            line(
                Pos2::new(x + w * 0.40, y + h * 0.22),
                Pos2::new(x + w * 0.40, y + h * 0.70),
            );
            line(
                Pos2::new(x + w * 0.40, y + h * 0.22),
                Pos2::new(x + w * 0.78, y + h * 0.12),
            );
            line(
                Pos2::new(x + w * 0.78, y + h * 0.12),
                Pos2::new(x + w * 0.78, y + h * 0.58),
            );
            painter.circle_filled(Pos2::new(x + w * 0.26, y + h * 0.74), w * 0.12, color);
            painter.circle_filled(Pos2::new(x + w * 0.64, y + h * 0.62), w * 0.12, color);
        }
        Icon::Videos => {
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.08, y + h * 0.18),
                    Pos2::new(x + w * 0.92, y + h * 0.82),
                ),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x + w * 0.42, y + h * 0.34),
                    Pos2::new(x + w * 0.42, y + h * 0.66),
                    Pos2::new(x + w * 0.68, y + h * 0.5),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Drive => {
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(x + w * 0.08, y + h * 0.2),
                    Pos2::new(x + w * 0.92, y + h * 0.8),
                ),
                egui::CornerRadius::same(3),
                s,
                egui::StrokeKind::Inside,
            );
            line(
                Pos2::new(x + w * 0.2, y + h * 0.58),
                Pos2::new(x + w * 0.8, y + h * 0.58),
            );
            painter.circle_filled(Pos2::new(x + w * 0.72, y + h * 0.7), w * 0.045, color);
        }
    }
}

fn child_path(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if name.trim().is_empty() {
        return Err("Enter a name first.".to_owned());
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("Use a single file or folder name.".to_owned());
    }
    Ok(parent.join(name))
}

fn column_widths(total: f32) -> [f32; 4] {
    let name = total * 0.50;
    let file_type = total * 0.18;
    let size = total * 0.12;
    let modified = total - name - file_type - size;
    [name, file_type, size, modified]
}

fn draw_table_header(ui: &mut egui::Ui, widths: [f32; 4]) {
    let width = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
    let painter = ui.painter();
    let mut left = rect.left() + 10.0;
    for (label, column_width) in ["Name", "Type", "Size", "Modified"].into_iter().zip(widths) {
        let cell = Rect::from_min_max(
            Pos2::new(left, rect.top()),
            Pos2::new(left + column_width - 12.0, rect.bottom()),
        );
        painter.with_clip_rect(cell).text(
            Pos2::new(cell.left(), cell.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            MUTED,
        );
        left += column_width;
    }
}

fn draw_file_row(
    ui: &mut egui::Ui,
    entry: &FileEntry,
    selected: bool,
    widths: [f32; 4],
) -> egui::Response {
    let total_width = widths.iter().sum();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(total_width, 34.0), Sense::click_and_drag());
    let fill = if selected {
        Color32::from_rgb(41, 73, 117)
    } else if response.hovered() {
        Color32::from_rgb(33, 41, 54)
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(5), fill);
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, Color32::from_rgb(39, 45, 56)),
    );

    let name = entry.name.clone();
    let file_type = if entry.is_dir {
        "Folder".to_owned()
    } else {
        entry
            .path
            .extension()
            .map(|extension| format!("{} file", extension.to_string_lossy().to_ascii_uppercase()))
            .unwrap_or_else(|| "File".to_owned())
    };
    let modified = entry
        .modified
        .clone()
        .map(|time| {
            DateTime::<Local>::from(time)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_owned());
    let text_color = if selected {
        Color32::WHITE
    } else {
        Color32::from_rgb(220, 225, 234)
    };
    let mut left = rect.left() + 10.0;
    for (index, (text, column_width)) in [
        name,
        file_type,
        if entry.is_dir {
            "—".to_owned()
        } else {
            format_size(entry.size)
        },
        modified,
    ]
    .into_iter()
    .zip(widths)
    .enumerate()
    {
        let cell = Rect::from_min_max(
            Pos2::new(left, rect.top()),
            Pos2::new(left + column_width - 12.0, rect.bottom()),
        );
        if index == 0 {
            paint_icon(
                ui.painter(),
                if entry.is_dir {
                    Icon::Folder
                } else {
                    Icon::File
                },
                Rect::from_min_size(
                    Pos2::new(cell.left(), cell.center().y - 8.0),
                    Vec2::splat(16.0),
                ),
                if selected {
                    Color32::WHITE
                } else if entry.is_dir {
                    ACCENT
                } else {
                    MUTED
                },
            );
        }
        let color = if index == 1 || index == 2 || index == 3 {
            if selected { text_color } else { MUTED }
        } else {
            text_color
        };
        ui.painter().with_clip_rect(cell).text(
            Pos2::new(
                cell.left() + if index == 0 { 21.0 } else { 0.0 },
                cell.center().y,
            ),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            color,
        );
        left += column_width;
    }
    response.on_hover_text(if entry.is_symlink {
        "Symbolic link"
    } else {
        "Double-click to open"
    })
}

fn format_size(mut size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut unit = 0;
    let mut value = size as f64;
    while size >= 1024 && unit < UNITS.len() - 1 {
        size /= 1024;
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
