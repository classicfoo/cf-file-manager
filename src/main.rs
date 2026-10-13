use std::{
    env, fs,
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CF File Manager")
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([780.0, 520.0]),
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

        let home = home_dir().unwrap_or_else(|| {
            env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        });
        let current_dir = fs::canonicalize(&home).unwrap_or(home);
        let mut app = Self {
            current_dir: current_dir.clone(),
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
                        self.selected = Some(target);
                        self.refresh();
                        self.status = Some("Item renamed.".to_owned());
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
                        self.selected = None;
                        self.refresh();
                        self.status = Some("Item deleted.".to_owned());
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

    fn show_toolbar(&mut self, ctx: &egui::Context) {
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

        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::default().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("CF File Manager").strong());
                    ui.add_space(12.0);
                    if ui.add_enabled(can_go_back, egui::Button::new("‹")).on_hover_text("Back").clicked() {
                        self.go_back();
                    }
                    if ui.add_enabled(can_go_forward, egui::Button::new("›")).on_hover_text("Forward").clicked() {
                        self.go_forward();
                    }
                    if ui.button("Up").on_hover_text("Open parent folder").clicked() {
                        self.go_up();
                    }
                    if ui.button("Refresh").clicked() {
                        refresh = true;
                    }

                    let path_width = (ui.available_width() - 48.0).max(160.0);
                    let response = ui.add_sized(
                        [path_width, 30.0],
                        egui::TextEdit::singleline(&mut self.path_input)
                            .id(egui::Id::new(PATH_INPUT_ID))
                            .hint_text("Enter a folder path"),
                    );
                    if self.focus_location {
                        response.request_focus();
                        self.focus_location = false;
                    }
                    let enter_pressed = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if ui.button("Go").clicked() || enter_pressed {
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
                    ui.add_sized(
                        [250.0, 29.0],
                        egui::TextEdit::singleline(&mut self.search).hint_text("Search this folder"),
                    );
                    let hidden_toggle = ui
                        .checkbox(&mut self.show_hidden, "Show hidden dotfiles")
                        .on_hover_text("Include names that start with a dot");
                    hidden_changed = hidden_toggle.changed();
                    if ui.button("New folder").clicked() {
                        new_folder = true;
                    }
                    if ui.add_enabled(has_selection, egui::Button::new("Rename")).clicked() {
                        rename = true;
                    }
                    if ui.add_enabled(has_selection, egui::Button::new("Delete")).clicked() {
                        delete = true;
                    }
                    if ui.add_enabled(has_selection, egui::Button::new("Open")).clicked() {
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

    fn show_sidebar(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let mut requested_path = None;
        egui::SidePanel::left("places")
            .resizable(true)
            .default_width(205.0)
            .min_width(165.0)
            .max_width(280.0)
            .frame(egui::Frame::default().fill(PANEL).inner_margin(egui::Margin::symmetric(12, 14)))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("PLACES").small().strong().color(MUTED));
                ui.add_space(8.0);
                if let Some(home) = home_dir() {
                    for (label, path) in standard_places(&home) {
                        let active = self.current_dir == path;
                        if ui.selectable_label(active, label).clicked() {
                            requested_path = Some(path);
                        }
                    }
                }

                ui.add_space(18.0);
                ui.label(egui::RichText::new("FILESYSTEM").small().strong().color(MUTED));
                ui.add_space(8.0);
                for root in filesystem_roots() {
                    let active = self.current_dir == root;
                    let label = root_label(&root);
                    if ui.selectable_label(active, label).clicked() {
                        requested_path = Some(root);
                    }
                }
            });
        requested_path
    }

    fn show_file_list(&mut self, ctx: &egui::Context) {
        let mut clicked_path = None;
        let mut opened_path = None;
        let entries = self.filtered_entries();
        let selected = self.selected.clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Color32::from_rgb(19, 23, 30)).inner_margin(egui::Margin::symmetric(18, 14)))
            .show(ctx, |ui| {
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
                        if response.clicked() {
                            clicked_path = Some(entry.path.clone());
                        }
                        if response.double_clicked() {
                            opened_path = Some(entry.path.clone());
                        }
                    }
                });
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
    }

    fn show_status(&self, ctx: &egui::Context) {
        let query = self.search.trim().to_lowercase();
        let visible_count = self
            .entries
            .iter()
            .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .count();
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::default().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 6)))
            .show(ctx, |ui| {
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
                        if ui.button(egui::RichText::new("Delete").color(Color32::WHITE)).clicked() {
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
        let shortcuts_enabled = !ctx.wants_keyboard_input();
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

        self.show_toolbar(&ctx);
        if let Some(path) = self.show_sidebar(&ctx) {
            self.navigate_to(&path, true);
        }
        self.show_status(&ctx);
        self.show_file_list(&ctx);
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
    let (rect, response) = ui.allocate_exact_size(Vec2::new(total_width, 34.0), Sense::click());
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

    let name = if entry.is_dir {
        format!("[Folder]  {}", entry.name)
    } else {
        format!("[File]  {}", entry.name)
    };
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
        .map(|time| DateTime::<Local>::from(time).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_owned());
    let text_color = if selected { Color32::WHITE } else { Color32::from_rgb(220, 225, 234) };
    let mut left = rect.left() + 10.0;
    for (index, (text, column_width)) in [
        name,
        file_type,
        if entry.is_dir { "—".to_owned() } else { format_size(entry.size) },
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
        let color = if index == 1 || index == 2 || index == 3 {
            if selected { text_color } else { MUTED }
        } else {
            text_color
        };
        ui.painter().with_clip_rect(cell).text(
            Pos2::new(cell.left(), cell.center().y),
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
