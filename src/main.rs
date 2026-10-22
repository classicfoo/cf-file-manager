use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[cfg(windows)]
mod windows_file_drag;

#[cfg(windows)]
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Rect, ResizeDirection, Sense, Vec2,
};
use egui_classic95::{
    Classic95, ClassicButton, ClassicListView, ClassicListViewColumn, ClassicMenuBar,
    ClassicMenuItem, ClassicMenuPopup, ClassicMenuSeparator, ClassicStatusBar, ClassicStatusPane,
    ClassicTextEdit, ClassicToolbar, ClassicToolbarButton, ClassicTooltip, ClassicTreeNode,
    ClassicTreeView, ClassicWindow, Palette, horizontal_separator, vertical_separator,
};

const PALETTE: Palette = Palette::windows_95();
const MUTED: Color32 = PALETTE.shadow;
const PATH_INPUT_ID: &str = "location_input";

fn main() -> eframe::Result {
    let app_icon = app_icon_data();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CF File Manager")
            .with_app_id("cf-file-manager")
            .with_icon(app_icon)
            .with_decorations(false)
            .with_resizable(true)
            .with_drag_and_drop(true)
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

fn app_icon_data() -> egui::IconData {
    #[cfg(target_os = "linux")]
    if let Some(image) = load_breeze_icon_image("system-file-manager", 64) {
        return egui::IconData {
            width: image.size[0] as u32,
            height: image.size[1] as u32,
            rgba: image
                .pixels
                .into_iter()
                .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                .collect(),
        };
    }

    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("the embedded application icon must be a valid PNG")
}

#[cfg(target_os = "linux")]
fn load_breeze_icon_image(name: &str, size: u16) -> Option<egui::ColorImage> {
    let path = freedesktop_icons::lookup(name)
        .with_size(size)
        .with_theme("breeze")
        .force_svg()
        .with_cache()
        .find()?;
    let svg = fs::read(path).ok()?;
    egui_extras::image::load_svg_bytes(&svg, &Default::default()).ok()
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
    tree_computer_expanded: bool,
    tree_home_expanded: bool,
    tree_selected: Option<Vec<usize>>,
    tree_selection_dir: PathBuf,
    file_drop_target: Option<PathBuf>,
    #[cfg(windows)]
    pending_native_drag: Option<(PathBuf, Instant)>,
}

impl FileManagerApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        Classic95::new(PALETTE).install(&creation_context.egui_ctx);
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
            history: vec![current_dir.clone()],
            history_index: 0,
            status: None,
            dialog: None,
            dialog_input: String::new(),
            focus_location: false,
            tree_computer_expanded: true,
            tree_home_expanded: true,
            tree_selected: None,
            tree_selection_dir: current_dir.clone(),
            file_drop_target: None,
            #[cfg(windows)]
            pending_native_drag: None,
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

        if is_downloads_folder(&self.current_dir) {
            entries.sort_by(|left, right| {
                right
                    .modified
                    .cmp(&left.modified)
                    .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            });
        } else {
            entries.sort_by(|left, right| {
                right
                    .is_dir
                    .cmp(&left.is_dir)
                    .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            });
        }
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
        match move_path(source, &target) {
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

    fn copy_dropped_items(&mut self, sources: &[PathBuf], destination_dir: &Path) {
        let mut copied = 0;
        let mut problems = Vec::new();

        for source in sources {
            let Some(name) = source.file_name() else {
                problems.push(format!("{} has no file name", source.display()));
                continue;
            };
            let target = destination_dir.join(name);
            if source == &target {
                continue;
            }
            if fs::symlink_metadata(&target).is_ok() {
                problems.push(format!("{} already exists", name.to_string_lossy()));
                continue;
            }

            let canonical_source = fs::canonicalize(source).unwrap_or_else(|_| source.clone());
            if canonical_source.is_dir() && destination_dir.starts_with(&canonical_source) {
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
                "Copied 1 item into the drop folder.".to_owned()
            } else {
                format!("Copied {copied} items into the drop folder.")
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

    fn show_menubar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("classic_menubar")
            .exact_size(26.0)
            .frame(egui::Frame::default().inner_margin(egui::Margin::ZERO))
            .show(ui, |ui| {
                ClassicMenuBar::new().show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.spacing_mut().button_padding.x = 4.0;
                    ui.add_space(4.0);
                    ui.menu_button("File", |ui| {
                        ClassicMenuPopup::new().show(ui, |ui| {
                            if ClassicMenuItem::button("New Folder...").show(ui).clicked() {
                                self.open_new_folder_dialog();
                                ui.close();
                            }
                            if ClassicMenuItem::button("Refresh").show(ui).clicked() {
                                self.refresh();
                                ui.close();
                            }
                            ClassicMenuSeparator::new().show(ui);
                            if ClassicMenuItem::button("Exit").show(ui).clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                    ui.menu_button("Edit", |ui| {
                        ClassicMenuPopup::new().show(ui, |ui| {
                            if ClassicMenuItem::button("Open").show(ui).clicked() {
                                self.open_selected();
                                ui.close();
                            }
                            if ClassicMenuItem::button("Rename...").show(ui).clicked() {
                                self.open_rename_dialog();
                                ui.close();
                            }
                            if ClassicMenuItem::button("Delete").show(ui).clicked() {
                                self.open_delete_dialog();
                                ui.close();
                            }
                        });
                    });
                    ui.menu_button("View", |ui| {
                        ClassicMenuPopup::new().show(ui, |ui| {
                            if ClassicMenuItem::check("Show hidden files", &mut self.show_hidden)
                                .show(ui)
                                .changed()
                            {
                                self.refresh();
                            }
                            ClassicMenuSeparator::new().show(ui);
                            if ClassicMenuItem::button("Refresh").show(ui).clicked() {
                                self.refresh();
                                ui.close();
                            }
                        });
                    });
                    ui.menu_button("Tools", |ui| {
                        ClassicMenuPopup::new().show(ui, |ui| {
                            ui.label("CF File Manager");
                        });
                    });
                    ui.menu_button("Help", |ui| {
                        ClassicMenuPopup::new().show(ui, |ui| {
                            ui.label("CF File Manager");
                            ui.label("Windows 95 classic theme");
                        });
                    });
                });
                let _ = horizontal_separator(ui);
            });
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        let mut new_folder = false;
        let mut rename = false;
        let mut delete = false;
        let mut open = false;
        let can_go_back = self.history_index > 0;
        let can_go_forward = self.history_index + 1 < self.history.len();
        let has_selection = self.selected.is_some();

        egui::Panel::top("toolbar")
            .exact_size(34.0)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ClassicToolbar::new().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if toolbar_button(ui, Icon::Back, "Back", can_go_back).clicked() {
                            self.go_back();
                        }
                        if toolbar_button(ui, Icon::Forward, "Forward", can_go_forward).clicked() {
                            self.go_forward();
                        }
                        if toolbar_button(ui, Icon::Up, "Up one level", true).clicked() {
                            self.go_up();
                        }
                        if toolbar_button(ui, Icon::Refresh, "Refresh", true).clicked() {
                            refresh = true;
                        }
                        let _ = vertical_separator(ui);
                        if toolbar_button(ui, Icon::FolderPlus, "New folder", true).clicked() {
                            new_folder = true;
                        }
                        if toolbar_button(ui, Icon::Rename, "Rename selected item", has_selection)
                            .clicked()
                        {
                            rename = true;
                        }
                        if toolbar_button(ui, Icon::Delete, "Delete selected item", has_selection)
                            .clicked()
                        {
                            delete = true;
                        }
                        if toolbar_button(ui, Icon::Open, "Open selected item", has_selection)
                            .clicked()
                        {
                            open = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            draw_icon(ui, Icon::Search, Color32::BLACK, 15.0);
                            ClassicTextEdit::new(
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text("Find files"),
                            )
                            .size(Vec2::new(180.0, 22.0))
                            .show(ui);
                        });
                    });
                });
            });

        let mut requested_path = None;
        egui::Panel::top("addressbar")
            .exact_size(29.0)
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(7, 3)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Address");
                    let response = ClassicTextEdit::new(
                        egui::TextEdit::singleline(&mut self.path_input)
                            .id(egui::Id::unique(PATH_INPUT_ID))
                            .hint_text("Enter a folder path"),
                    )
                    .size(Vec2::new(ui.available_width() - 47.0, 22.0))
                    .show(ui);
                    let response = ClassicTooltip::new("Location (Ctrl+L)").show_for(response);
                    if self.focus_location {
                        response.request_focus();
                        self.focus_location = false;
                    }
                    let enter_pressed = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if ClassicButton::new("Go").show(ui).clicked() || enter_pressed {
                        let entered_path = PathBuf::from(self.path_input.clone());
                        requested_path = Some(if entered_path.is_absolute() {
                            entered_path
                        } else {
                            self.current_dir.join(entered_path)
                        });
                    }
                });
            });

        if refresh {
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
        let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let desktop = home.join("Desktop");
        let roots = filesystem_roots();
        let places = standard_places(&home)
            .into_iter()
            .filter(|(label, _)| *label != "Home")
            .collect::<Vec<_>>();

        if self.tree_selection_dir != self.current_dir || self.tree_selected.is_none() {
            self.tree_selected =
                tree_selection_for_dir(&self.current_dir, &desktop, &home, &roots, &places);
            self.tree_selection_dir = self.current_dir.clone();
        }

        let mut selected = self.tree_selected.clone();
        let mut visible_icons = vec![Icon::Desktop, Icon::Drive];
        let mut visible_icon_depths = vec![0, 0];
        if self.tree_computer_expanded {
            visible_icons.extend(std::iter::repeat_n(Icon::Drive, roots.len()));
            visible_icon_depths.extend(std::iter::repeat_n(1, roots.len()));
            visible_icons.push(Icon::Home);
            visible_icon_depths.push(1);
            if self.tree_home_expanded {
                visible_icons.extend(places.iter().map(|(label, _)| place_icon(label)));
                visible_icon_depths.extend(std::iter::repeat_n(2, places.len()));
            }
        }

        let mut computer_children = roots
            .iter()
            .map(|root| ClassicTreeNode::new(format!("     {}", root_label(root))))
            .collect::<Vec<_>>();
        computer_children.push(
            ClassicTreeNode::new("     Home")
                .expanded(self.tree_home_expanded)
                .children(
                    places
                        .iter()
                        .map(|(label, _)| ClassicTreeNode::new(format!("     {label}"))),
                ),
        );
        let nodes = [
            ClassicTreeNode::new("     Desktop"),
            ClassicTreeNode::new("     My Computer")
                .expanded(self.tree_computer_expanded)
                .children(computer_children),
        ];

        egui::Panel::left("places")
            .resizable(true)
            .default_size(215.0)
            .min_size(170.0)
            .max_size(300.0)
            .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(5, 4)))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Pinned").strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let already_pinned = self.is_pinned(&self.current_dir);
                                    if ui
                                        .add_enabled(
                                            !already_pinned,
                                            ClassicButton::new("+").min_size(Vec2::new(24.0, 21.0)),
                                        )
                                        .on_hover_text("Pin the current folder")
                                        .clicked()
                                    {
                                        pin_current = true;
                                    }
                                },
                            );
                        });
                        for path in &pinned_folders {
                            let label = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned());
                            ui.horizontal(|ui| {
                                let button_width = (ui.available_width() - 27.0).max(80.0);
                                if ui
                                    .add_sized(
                                        [button_width, 22.0],
                                        ClassicButton::new(label.as_str()),
                                    )
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    requested_path = Some(path.clone());
                                }
                                if ui
                                    .add_sized(
                                        [24.0, 22.0],
                                        ClassicButton::new("×").min_size(Vec2::new(24.0, 22.0)),
                                    )
                                    .on_hover_text("Remove from Pinned")
                                    .clicked()
                                {
                                    remove_pinned = Some(path.clone());
                                }
                            });
                        }
                        let _ = horizontal_separator(ui);
                        ui.label(egui::RichText::new("All Folders").strong());
                        let _ = horizontal_separator(ui);
                        let tree = ClassicTreeView::new(nodes, &mut selected)
                            .id_salt("file-manager-folders")
                            .row_height(20.0)
                            .max_height((ui.available_height() - 4.0).max(0.0))
                            .show(ui);

                        for ((row, icon), depth) in tree
                            .rows
                            .iter()
                            .zip(visible_icons.iter().copied())
                            .zip(visible_icon_depths.iter().copied())
                        {
                            let icon_rect = Rect::from_center_size(
                                Pos2::new(
                                    row.rect.left() + 24.0 + depth as f32 * 16.0,
                                    row.rect.center().y,
                                ),
                                Vec2::splat(14.0),
                            );
                            if !paint_breeze_icon(
                                ui.painter(),
                                icon_theme_name(icon),
                                icon_rect,
                                Color32::WHITE,
                            ) {
                                paint_explorer_icon(ui.painter(), icon, icon_rect);
                            }
                        }

                        if tree.expanded_changed {
                            if let Some(path) = tree.clicked.as_deref() {
                                if path == [1] {
                                    self.tree_computer_expanded = !self.tree_computer_expanded;
                                } else if path == [1, roots.len()] {
                                    self.tree_home_expanded = !self.tree_home_expanded;
                                }
                            } else if ui.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
                                if tree.selected.as_deref() == Some([1].as_slice()) {
                                    self.tree_computer_expanded = true;
                                } else if tree.selected.as_deref()
                                    == Some([1, roots.len()].as_slice())
                                {
                                    self.tree_home_expanded = true;
                                }
                            } else if ui.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
                                if tree.selected.as_deref() == Some([1].as_slice()) {
                                    self.tree_computer_expanded = false;
                                } else if tree.selected.as_deref()
                                    == Some([1, roots.len()].as_slice())
                                {
                                    self.tree_home_expanded = false;
                                }
                            }
                        }

                        if !tree.expanded_changed {
                            if let Some(path) = tree.clicked.as_deref() {
                                requested_path =
                                    tree_path_to_dir(path, &desktop, &home, &roots, &places);
                            }
                        }

                        self.tree_selected = tree.selected;
                        if let Some(path) = &requested_path {
                            self.tree_selection_dir = path.clone();
                        }
                    },
                );
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
        #[cfg(windows)]
        if self
            .pending_native_drag
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() >= Duration::from_secs(5))
        {
            self.pending_native_drag = None;
        }

        let entries = self.filtered_entries();
        let mut selected_index = self
            .selected
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| &entry.path == path));
        let mut pin_action = None;
        #[cfg(not(windows))]
        let mut move_action: Option<(PathBuf, PathBuf)> = None;
        #[cfg(windows)]
        let mut native_drag_result = None;
        let (hovering_dropped_files, dropped_files, pointer_pos) = ui.ctx().input(|input| {
            (
                !input.raw.hovered_files.is_empty(),
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>(),
                input.pointer.hover_pos(),
            )
        });
        #[cfg(windows)]
        let native_drag_active = self.pending_native_drag.is_some();
        #[cfg(not(windows))]
        let native_drag_active = false;
        let file_drop_active =
            hovering_dropped_files || !dropped_files.is_empty() || native_drag_active;
        let current_dir = self.current_dir.clone();
        let mut file_drop_hover_target = None;

        egui::CentralPanel::default().show(ui, |ui| {
            let current_folder_drop_target = ui.interact(
                ui.max_rect(),
                ui.make_persistent_id("current_folder_drop_target"),
                Sense::hover(),
            );
            if file_drop_active {
                ui.label("Drop onto a folder row or here to place items");
            }
            let available_width = ui.available_width();
            let list = ClassicListView::new(
                [
                    ClassicListViewColumn::new("Name").width(available_width * 0.50),
                    ClassicListViewColumn::new("Size").width(available_width * 0.13),
                    ClassicListViewColumn::new("Type").width(available_width * 0.16),
                    ClassicListViewColumn::new("Modified"),
                ],
                entries.iter().map(|entry| {
                    let file_type = if entry.is_dir {
                        "Folder".to_owned()
                    } else {
                        entry
                            .path
                            .extension()
                            .map(|extension| {
                                format!("{} file", extension.to_string_lossy().to_ascii_uppercase())
                            })
                            .unwrap_or_else(|| "File".to_owned())
                    };
                    let modified = entry
                        .modified
                        .map(|time| {
                            DateTime::<Local>::from(time)
                                .format("%d/%m/%Y %H:%M")
                                .to_string()
                        })
                        .unwrap_or_else(|| "—".to_owned());
                    vec![
                        format!("     {}", entry.name),
                        if entry.is_dir {
                            "—".to_owned()
                        } else {
                            format_size(entry.size)
                        },
                        file_type,
                        modified,
                    ]
                }),
                &mut selected_index,
            )
            .id_salt("file-manager-report")
            .row_height(23.0)
            .max_height((ui.available_height() - 26.0).max(0.0))
            .show(ui);

            let row_painter = ui.painter().with_clip_rect(list.response.rect);
            for (index, (entry, base_response)) in entries.iter().zip(list.rows.iter()).enumerate()
            {
                let response = base_response.interact(Sense::click_and_drag());
                #[cfg(windows)]
                if response.drag_started_by(PointerButton::Primary) {
                    native_drag_result = Some((
                        entry.path.clone(),
                        windows_file_drag::drag_path(&entry.path),
                    ));
                }
                #[cfg(not(windows))]
                response.dnd_set_drag_payload(entry.path.clone());

                let icon_rect = Rect::from_center_size(
                    Pos2::new(response.rect.left() + 12.0, response.rect.center().y),
                    Vec2::splat(16.0),
                );
                if !paint_breeze_icon(
                    &row_painter,
                    file_icon_theme_name(entry),
                    icon_rect,
                    Color32::WHITE,
                ) {
                    paint_explorer_icon(
                        &row_painter,
                        if entry.is_dir {
                            Icon::Folder
                        } else {
                            Icon::File
                        },
                        icon_rect,
                    );
                }
                let tooltip = if entry.is_symlink {
                    "Symbolic link"
                } else {
                    "Double-click to open"
                };
                let _ = ClassicTooltip::new(tooltip).show_for(response.clone());
                if response.double_clicked() {
                    selected_index = Some(index);
                }

                if entry.is_dir {
                    let pointer_is_over = response.hovered()
                        || pointer_pos.is_some_and(|position| response.rect.contains(position));
                    if file_drop_active && pointer_is_over {
                        file_drop_hover_target = Some(entry.path.clone());
                        ui.painter().rect_stroke(
                            response.rect,
                            egui::CornerRadius::same(2),
                            egui::Stroke::new(2.0, PALETTE.highlight),
                            egui::StrokeKind::Inside,
                        );
                    }
                    #[cfg(not(windows))]
                    if response.dnd_hover_payload::<PathBuf>().is_some() {
                        ui.painter().rect_stroke(
                            response.rect,
                            egui::CornerRadius::same(2),
                            egui::Stroke::new(2.0, PALETTE.highlight),
                            egui::StrokeKind::Inside,
                        );
                    }
                    #[cfg(not(windows))]
                    if let Some(source) = response.dnd_release_payload::<PathBuf>() {
                        move_action = Some((source.as_ref().clone(), entry.path.clone()));
                    }

                    let path = entry.path.clone();
                    response.context_menu(|ui| {
                        let pinned = self.is_pinned(&path);
                        let label = if pinned {
                            "Remove from Pinned"
                        } else {
                            "Pin folder"
                        };
                        if ClassicMenuItem::button(label).show(ui).clicked() {
                            pin_action = Some((path.clone(), pinned));
                            ui.close();
                        }
                    });
                }
            }

            if entries.is_empty() {
                let message = if self.status.is_some() {
                    "This folder could not be listed."
                } else if self.search.trim().is_empty() {
                    "This folder is empty."
                } else {
                    "No files match your search."
                };
                ui.painter().text(
                    Pos2::new(
                        list.response.rect.center().x,
                        list.response.rect.top() + 2.0 + 22.0 + 34.5,
                    ),
                    Align2::CENTER_CENTER,
                    message,
                    FontId::proportional(13.0),
                    MUTED,
                );
            }

            self.selected =
                selected_index.and_then(|index| entries.get(index).map(|entry| entry.path.clone()));

            if let Some(index) = selected_index {
                if list
                    .rows
                    .get(index)
                    .is_some_and(egui::Response::double_clicked)
                {
                    if let Some(entry) = entries.get(index) {
                        let path = entry.path.clone();
                        if path.is_dir() {
                            self.navigate_to(&path, true);
                        } else if let Err(error) = open::that(&path) {
                            self.status = Some(format!("Could not open file: {error}"));
                        } else {
                            self.status = Some("Opened with the system's default app.".to_owned());
                        }
                    }
                }
            }

            if file_drop_active
                && file_drop_hover_target.is_none()
                && current_folder_drop_target.hovered()
            {
                file_drop_hover_target = Some(current_dir.clone());
            }
            #[cfg(not(windows))]
            if let Some(source) = current_folder_drop_target.dnd_release_payload::<PathBuf>() {
                move_action = Some((source.as_ref().clone(), current_dir.clone()));
            }
        });

        if let Some((path, was_pinned)) = pin_action {
            if was_pinned {
                self.unpin_folder(&path);
            } else {
                self.pin_folder(&path);
            }
        }
        #[cfg(not(windows))]
        if let Some((source, destination)) = move_action {
            self.move_item_to(&source, &destination);
        }
        #[cfg(windows)]
        if let Some((source, result)) = native_drag_result {
            match result {
                Ok(true) => self.pending_native_drag = Some((source, Instant::now())),
                Ok(false) => self.status = Some("Drag canceled.".to_owned()),
                Err(error) => self.status = Some(format!("Could not start drag: {error}")),
            }
        }

        if !dropped_files.is_empty() {
            let destination = file_drop_hover_target
                .or_else(|| self.file_drop_target.clone())
                .unwrap_or_else(|| self.current_dir.clone());
            self.file_drop_target = None;

            #[cfg(windows)]
            let pending_source = self
                .pending_native_drag
                .take()
                .filter(|(_, started)| started.elapsed() < Duration::from_secs(5))
                .map(|(path, _)| path);

            let mut files_to_copy = Vec::new();
            for source in dropped_files {
                #[cfg(windows)]
                if pending_source
                    .as_ref()
                    .is_some_and(|pending| same_filesystem_path(pending, &source))
                {
                    self.move_item_to(&source, &destination);
                    continue;
                }
                files_to_copy.push(source);
            }
            if !files_to_copy.is_empty() {
                self.copy_dropped_items(&files_to_copy, &destination);
            }
        } else if hovering_dropped_files {
            self.file_drop_target = file_drop_hover_target;
        } else {
            self.file_drop_target = None;
        }
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        let query = self.search.trim().to_lowercase();
        let visible_count = self
            .entries
            .iter()
            .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .count();
        let left_text = if let Some(path) = &self.selected {
            if let Some(entry) = self.entries.iter().find(|entry| &entry.path == path) {
                format!("{}  {}", entry.name, format_size(entry.size))
            } else {
                format!("{visible_count} object(s)")
            }
        } else {
            format!("{visible_count} object(s)")
        };
        let status_text = self.status.as_deref().unwrap_or("Ready");

        egui::Panel::bottom("status")
            .exact_size(25.0)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ClassicStatusBar::new().show(ui, |ui, bounds| {
                    let panes_rect = Rect::from_min_max(
                        bounds.min + Vec2::splat(1.0),
                        bounds.max + Vec2::new(-16.0, -1.0),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(panes_rect), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ClassicStatusPane::new()
                                .width(250.0)
                                .height(panes_rect.height())
                                .show(ui, |ui, pane| {
                                    ui.painter().text(
                                        pane.left_center() + Vec2::new(6.0, 0.0),
                                        Align2::LEFT_CENTER,
                                        &left_text,
                                        FontId::proportional(11.0),
                                        PALETTE.text,
                                    );
                                });
                            ClassicStatusPane::new()
                                .width(ui.available_width())
                                .height(panes_rect.height())
                                .show(ui, |ui, pane| {
                                    ui.painter().text(
                                        pane.left_center() + Vec2::new(6.0, 0.0),
                                        Align2::LEFT_CENTER,
                                        status_text,
                                        FontId::proportional(11.0),
                                        PALETTE.text,
                                    );
                                });
                        });
                    });
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
            .id(egui::Id::unique("file_manager_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut is_open)
            .show(ctx, |ui| match &kind {
                DialogKind::NewFolder(parent) => {
                    ui.label(format!("Create a folder in {}", parent.display()));
                    let response = ClassicTextEdit::new(
                        egui::TextEdit::singleline(&mut self.dialog_input).hint_text("Folder name"),
                    )
                    .size(Vec2::new(330.0, 30.0))
                    .show(ui);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let create = ClassicButton::new("Create").show(ui).clicked()
                            || (response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                        if create {
                            action = Some(DialogAction::CreateFolder {
                                parent: parent.clone(),
                                name: self.dialog_input.clone(),
                            });
                        }
                        if ClassicButton::new("Cancel").show(ui).clicked() {
                            cancel_requested = true;
                        }
                    });
                }
                DialogKind::Rename(path) => {
                    ui.label(format!("Rename {}", path.display()));
                    let response =
                        ClassicTextEdit::new(egui::TextEdit::singleline(&mut self.dialog_input))
                            .size(Vec2::new(330.0, 30.0))
                            .show(ui);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let save = ClassicButton::new("Save").show(ui).clicked()
                            || (response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                        if save {
                            action = Some(DialogAction::Rename {
                                old_path: path.clone(),
                                name: self.dialog_input.clone(),
                            });
                        }
                        if ClassicButton::new("Cancel").show(ui).clicked() {
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
                        if ClassicButton::new("Delete").show(ui).clicked() {
                            action = Some(DialogAction::Delete(path.clone()));
                        }
                        if ClassicButton::new("Cancel").show(ui).clicked() {
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
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        PALETTE.face.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let full_rect = ui.max_rect();
        ClassicWindow::new("CF File Manager").palette(PALETTE).show(
            ui,
            |painter, rect| {
                if !paint_breeze_icon(painter, "system-file-manager", rect, Color32::WHITE) {
                    paint_explorer_icon(painter, Icon::Drive, rect);
                }
            },
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.show_menubar(ui);
                self.show_toolbar(ui);
                if let Some(path) = self.show_sidebar(ui) {
                    self.navigate_to(&path, true);
                }
                self.show_status(ui);
                self.show_file_list(ui);
                self.show_dialog(&ctx);
            },
        );
        resize_handles(ui, full_rect);
    }
}

fn resize_handles(ui: &mut egui::Ui, rect: Rect) {
    let edge = 5.0;
    let corner = 9.0;
    let southeast_corner = 17.0;
    let handles = [
        (
            "resize-n",
            Rect::from_min_max(
                Pos2::new(rect.left() + corner, rect.top() - edge),
                Pos2::new(rect.right() - corner, rect.top() + edge),
            ),
            ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            "resize-s",
            Rect::from_min_max(
                Pos2::new(rect.left() + corner, rect.bottom() - edge),
                Pos2::new(rect.right() - southeast_corner, rect.bottom() + edge),
            ),
            ResizeDirection::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            "resize-e",
            Rect::from_min_max(
                Pos2::new(rect.right() - edge, rect.top() + corner),
                Pos2::new(rect.right() + edge, rect.bottom() - southeast_corner),
            ),
            ResizeDirection::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            "resize-w",
            Rect::from_min_max(
                Pos2::new(rect.left() - edge, rect.top() + corner),
                Pos2::new(rect.left() + edge, rect.bottom() - corner),
            ),
            ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            "resize-ne",
            Rect::from_min_max(
                Pos2::new(rect.right() - corner, rect.top() - corner),
                Pos2::new(rect.right() + edge, rect.top() + edge),
            ),
            ResizeDirection::NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            "resize-nw",
            Rect::from_min_max(
                Pos2::new(rect.left() - edge, rect.top() - corner),
                Pos2::new(rect.left() + corner, rect.top() + edge),
            ),
            ResizeDirection::NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            "resize-se",
            Rect::from_min_max(
                Pos2::new(
                    rect.right() - southeast_corner,
                    rect.bottom() - southeast_corner,
                ),
                Pos2::new(rect.right() + edge, rect.bottom() + edge),
            ),
            ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            "resize-sw",
            Rect::from_min_max(
                Pos2::new(rect.left() - edge, rect.bottom() - corner),
                Pos2::new(rect.left() + corner, rect.bottom() + edge),
            ),
            ResizeDirection::SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
    ];

    for (id, handle_rect, direction, cursor) in handles {
        let response = ui
            .interact(handle_rect, egui::Id::unique(id), Sense::drag())
            .on_hover_cursor(cursor);
        if response.drag_started_by(PointerButton::Primary) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");
    home.map(PathBuf::from)
}

fn is_downloads_folder(path: &Path) -> bool {
    home_dir()
        .and_then(|home| fs::canonicalize(home.join("Downloads")).ok())
        .is_some_and(|downloads| downloads == path)
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

fn move_path(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if is_cross_device_error(&error) => {
            if let Err(copy_error) = copy_item(source, destination) {
                remove_path_if_present(destination);
                return Err(copy_error);
            }
            if let Err(remove_error) = remove_path(source) {
                remove_path_if_present(destination);
                return Err(remove_error);
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn remove_path_if_present(path: &Path) {
    if fs::symlink_metadata(path).is_ok() {
        let _ = remove_path(path);
    }
}

fn is_cross_device_error(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17) // ERROR_NOT_SAME_DEVICE
    }
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(18) // EXDEV
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = error;
        false
    }
}

#[cfg(windows)]
fn same_filesystem_path(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
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

fn tree_selection_for_dir(
    current_dir: &Path,
    desktop: &Path,
    home: &Path,
    roots: &[PathBuf],
    places: &[(&'static str, PathBuf)],
) -> Option<Vec<usize>> {
    if current_dir == desktop && desktop.is_dir() {
        return Some(vec![0]);
    }
    if current_dir == home {
        return Some(vec![1, roots.len()]);
    }
    if let Some(index) = roots.iter().position(|root| root == current_dir) {
        return Some(vec![1, index]);
    }
    places
        .iter()
        .position(|(_, path)| path == current_dir)
        .map(|index| vec![1, roots.len(), index])
}

fn tree_path_to_dir(
    path: &[usize],
    desktop: &Path,
    home: &Path,
    roots: &[PathBuf],
    places: &[(&'static str, PathBuf)],
) -> Option<PathBuf> {
    match path {
        [0] => Some(if desktop.is_dir() {
            desktop.to_path_buf()
        } else {
            home.to_path_buf()
        }),
        [1, index] if *index < roots.len() => roots.get(*index).cloned(),
        [1, index] if *index == roots.len() => Some(home.to_path_buf()),
        [1, root_index, place_index]
            if *root_index == roots.len() && *place_index < places.len() =>
        {
            places.get(*place_index).map(|(_, path)| path.clone())
        }
        _ => None,
    }
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

fn icon_theme_name(icon: Icon) -> &'static str {
    match icon {
        Icon::Back => "go-previous",
        Icon::Forward => "go-next",
        Icon::Up => "go-up",
        Icon::Refresh => "view-refresh",
        Icon::Search => "system-search",
        Icon::FolderPlus => "folder-new",
        Icon::Rename => "document-edit",
        Icon::Delete => "edit-delete",
        Icon::Open => "document-open",
        Icon::Folder => "folder",
        Icon::File => "text-x-generic",
        Icon::Home => "user-home",
        Icon::Desktop => "user-desktop",
        Icon::Documents => "folder-documents",
        Icon::Downloads => "folder-download",
        Icon::Pictures => "folder-pictures",
        Icon::Music => "audio-x-generic",
        Icon::Videos => "folder-videos",
        Icon::Drive => "drive-harddisk",
    }
}

#[cfg(target_os = "linux")]
fn paint_breeze_icon(painter: &egui::Painter, name: &str, rect: Rect, tint: Color32) -> bool {
    let ctx = painter.ctx();
    let id = egui::Id::NULL.with(("cf-file-manager-breeze-icon", name));
    let texture = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = texture.or_else(|| {
        let image = load_breeze_icon_image(name, 32)?;
        let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
        Some(texture)
    });
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            tint,
        );
        true
    } else {
        false
    }
}

#[cfg(not(target_os = "linux"))]
fn paint_breeze_icon(_painter: &egui::Painter, _name: &str, _rect: Rect, _tint: Color32) -> bool {
    false
}

fn file_icon_theme_name(entry: &FileEntry) -> &'static str {
    if entry.is_dir {
        return "folder";
    }

    match entry
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application-pdf",
        "exe" | "appimage" | "bin" | "deb" | "rpm" => "application-x-executable",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "tif" | "tiff" => {
            "image-x-generic"
        }
        "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" => "audio-x-generic",
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "mpeg" | "mpg" => "video-x-generic",
        _ => "text-x-generic",
    }
}

fn toolbar_button(ui: &mut egui::Ui, icon: Icon, hint: &str, enabled: bool) -> egui::Response {
    let tint = if enabled { Color32::WHITE } else { MUTED };
    ClassicToolbarButton::new(hint)
        .enabled(enabled)
        .size(Vec2::new(27.0, 25.0))
        .show(ui, |painter, rect| {
            paint_toolbar_icon(painter, icon, rect, tint)
        })
}

fn paint_toolbar_icon(painter: &egui::Painter, icon: Icon, rect: Rect, tint: Color32) {
    if paint_breeze_icon(painter, icon_theme_name(icon), rect, tint) {
        return;
    }
    let px = rect.width() / 16.0;
    let py = rect.height() / 16.0;
    let p = |x: f32, y: f32| Pos2::new(rect.left() + x * px, rect.top() + y * py);
    let block = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_max(p(x, y), p(x + w, y + h));
    match icon {
        Icon::Back | Icon::Forward => {
            let color = if tint == MUTED {
                MUTED
            } else {
                Color32::from_rgb(0, 64, 160)
            };
            let left = matches!(icon, Icon::Back);
            let (shaft, head) = if left {
                (
                    block(6.0, 7.0, 8.0, 3.0),
                    [p(7.0, 3.0), p(2.0, 8.5), p(7.0, 14.0)],
                )
            } else {
                (
                    block(2.0, 7.0, 8.0, 3.0),
                    [p(9.0, 3.0), p(14.0, 8.5), p(9.0, 14.0)],
                )
            };
            painter.rect_filled(shaft, egui::CornerRadius::ZERO, color);
            painter.add(egui::Shape::convex_polygon(
                head.to_vec(),
                color,
                egui::Stroke::NONE,
            ));
            painter.line_segment(
                [p(3.0, 13.0), p(12.0, 13.0)],
                egui::Stroke::new(1.0, Color32::from_rgb(96, 96, 96)),
            );
        }
        Icon::Up => {
            paint_explorer_icon(painter, Icon::Folder, rect.shrink(1.0));
            painter.rect_filled(
                block(7.0, 3.0, 2.0, 7.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(0, 96, 192),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![p(4.0, 5.0), p(8.0, 1.0), p(12.0, 5.0)],
                Color32::from_rgb(0, 96, 192),
                egui::Stroke::NONE,
            ));
        }
        Icon::Refresh => {
            painter.circle_stroke(
                rect.center(),
                rect.width() * 0.31,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 112, 40)),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![p(10.0, 1.0), p(15.0, 2.0), p(12.0, 6.0)],
                Color32::from_rgb(0, 112, 40),
                egui::Stroke::NONE,
            ));
            painter.line_segment(
                [p(5.0, 12.0), p(11.0, 12.0)],
                egui::Stroke::new(1.0, Color32::from_rgb(0, 112, 40)),
            );
        }
        Icon::FolderPlus | Icon::Folder | Icon::Drive | Icon::Desktop | Icon::Home | Icon::File => {
            paint_explorer_icon(painter, icon, rect);
        }
        Icon::Rename => {
            painter.line_segment(
                [p(3.0, 12.0), p(11.0, 4.0)],
                egui::Stroke::new(4.0, Color32::from_rgb(228, 176, 40)),
            );
            painter.line_segment(
                [p(3.0, 12.0), p(11.0, 4.0)],
                egui::Stroke::new(1.0, Color32::from_rgb(96, 64, 16)),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![p(2.0, 12.0), p(4.0, 14.0), p(1.0, 15.0)],
                Color32::from_rgb(160, 112, 48),
                egui::Stroke::NONE,
            ));
        }
        Icon::Delete => {
            painter.rect_filled(
                block(4.0, 5.0, 8.0, 9.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(176, 184, 184),
            );
            painter.rect_stroke(
                block(4.0, 5.0, 8.0, 9.0),
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, Color32::from_rgb(64, 64, 64)),
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(
                block(3.0, 3.0, 10.0, 2.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(112, 112, 112),
            );
            painter.rect_filled(
                block(6.0, 1.0, 4.0, 1.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(64, 64, 64),
            );
            for x in [6.0, 8.0, 10.0] {
                painter.line_segment(
                    [p(x, 7.0), p(x, 12.0)],
                    egui::Stroke::new(1.0, Color32::from_rgb(96, 96, 96)),
                );
            }
        }
        Icon::Open => {
            paint_explorer_icon(painter, Icon::Folder, rect);
            painter.line_segment(
                [p(8.0, 11.0), p(13.0, 6.0)],
                egui::Stroke::new(2.0, Color32::from_rgb(0, 64, 160)),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![p(10.0, 5.0), p(14.0, 5.0), p(14.0, 9.0)],
                Color32::from_rgb(0, 64, 160),
                egui::Stroke::NONE,
            ));
        }
        Icon::Search => {
            painter.circle_filled(p(6.0, 6.0), 4.0 * px, Color32::from_rgb(96, 160, 216));
            painter.circle_stroke(
                p(6.0, 6.0),
                4.0 * px,
                egui::Stroke::new(1.0, Color32::from_rgb(32, 32, 32)),
            );
            painter.line_segment(
                [p(9.0, 9.0), p(14.0, 14.0)],
                egui::Stroke::new(2.0, Color32::from_rgb(48, 48, 48)),
            );
        }
        Icon::Documents | Icon::Downloads | Icon::Pictures | Icon::Music | Icon::Videos => {
            paint_explorer_icon(painter, icon, rect);
        }
    }
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

fn paint_explorer_icon(painter: &egui::Painter, icon: Icon, rect: Rect) {
    if paint_breeze_icon(painter, icon_theme_name(icon), rect, Color32::WHITE) {
        return;
    }
    let px = rect.width() / 16.0;
    let py = rect.height() / 14.0;
    let cell = |x: f32, y: f32, w: f32, h: f32| {
        Rect::from_min_max(
            Pos2::new(rect.left() + x * px, rect.top() + y * py),
            Pos2::new(rect.left() + (x + w) * px, rect.top() + (y + h) * py),
        )
    };
    let outline = Color32::from_rgb(96, 69, 24);
    match icon {
        Icon::Folder | Icon::FolderPlus | Icon::Downloads => {
            painter.rect_filled(
                cell(2.0, 1.0, 7.0, 4.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(255, 220, 88),
            );
            painter.rect_filled(
                cell(1.0, 4.0, 14.0, 8.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(239, 175, 46),
            );
            painter.rect_stroke(
                cell(1.0, 4.0, 14.0, 8.0),
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, outline),
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [cell(2.0, 5.0, 1.0, 0.0).min, cell(13.0, 5.0, 0.0, 0.0).min],
                egui::Stroke::new(1.0, Color32::from_rgb(255, 239, 146)),
            );
            if matches!(icon, Icon::FolderPlus) {
                painter.rect_filled(
                    cell(10.0, 7.0, 1.0, 4.0),
                    egui::CornerRadius::ZERO,
                    Color32::WHITE,
                );
                painter.rect_filled(
                    cell(8.5, 8.5, 4.0, 1.0),
                    egui::CornerRadius::ZERO,
                    Color32::WHITE,
                );
            }
            if matches!(icon, Icon::Downloads) {
                painter.line_segment(
                    [cell(8.0, 5.0, 0.0, 0.0).min, cell(8.0, 9.0, 0.0, 0.0).min],
                    egui::Stroke::new(1.5, Color32::from_rgb(0, 80, 0)),
                );
                painter.line_segment(
                    [cell(6.0, 7.0, 0.0, 0.0).min, cell(8.0, 9.0, 0.0, 0.0).min],
                    egui::Stroke::new(1.5, Color32::from_rgb(0, 80, 0)),
                );
                painter.line_segment(
                    [cell(8.0, 9.0, 0.0, 0.0).min, cell(10.0, 7.0, 0.0, 0.0).min],
                    egui::Stroke::new(1.5, Color32::from_rgb(0, 80, 0)),
                );
            }
        }
        Icon::File | Icon::Documents => {
            painter.rect_filled(
                cell(3.0, 0.5, 9.0, 13.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(250, 250, 250),
            );
            painter.rect_stroke(
                cell(3.0, 0.5, 9.0, 13.0),
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, Color32::from_rgb(80, 80, 80)),
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    cell(9.0, 1.0, 0.0, 0.0).min,
                    cell(12.0, 4.0, 0.0, 0.0).min,
                    cell(9.0, 4.0, 0.0, 0.0).min,
                ],
                Color32::from_rgb(210, 210, 210),
                egui::Stroke::new(1.0, Color32::from_rgb(120, 120, 120)),
            ));
            for y in [6.0, 8.0, 10.0] {
                painter.line_segment(
                    [cell(5.0, y, 0.0, 0.0).min, cell(10.0, y, 0.0, 0.0).min],
                    egui::Stroke::new(1.0, Color32::from_rgb(110, 110, 110)),
                );
            }
        }
        Icon::Drive => {
            painter.rect_filled(
                cell(1.0, 3.0, 14.0, 9.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(178, 178, 178),
            );
            painter.rect_stroke(
                cell(1.0, 3.0, 14.0, 9.0),
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, Color32::from_rgb(64, 64, 64)),
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(
                cell(3.0, 5.0, 10.0, 2.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(224, 224, 224),
            );
            painter.rect_filled(
                cell(4.0, 9.0, 7.0, 1.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(90, 90, 90),
            );
            painter.rect_filled(
                cell(12.0, 9.0, 1.0, 1.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(0, 128, 0),
            );
        }
        Icon::Desktop => {
            painter.rect_filled(
                cell(1.0, 0.5, 14.0, 9.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(80, 80, 80),
            );
            painter.rect_filled(
                cell(2.0, 1.5, 12.0, 7.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(68, 123, 185),
            );
            painter.rect_filled(
                cell(7.0, 9.0, 2.0, 2.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(128, 128, 128),
            );
            painter.rect_filled(
                cell(4.0, 11.0, 8.0, 1.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(80, 80, 80),
            );
            painter.rect_filled(
                cell(3.0, 12.0, 10.0, 1.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(160, 160, 160),
            );
        }
        Icon::Home => {
            painter.rect_filled(
                cell(3.0, 6.0, 10.0, 7.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(224, 196, 142),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    cell(1.0, 6.0, 0.0, 0.0).min,
                    cell(8.0, 0.5, 0.0, 0.0).min,
                    cell(15.0, 6.0, 0.0, 0.0).min,
                ],
                Color32::from_rgb(174, 42, 34),
                egui::Stroke::new(1.0, Color32::from_rgb(88, 40, 26)),
            ));
            painter.rect_filled(
                cell(7.0, 8.0, 3.0, 5.0),
                egui::CornerRadius::ZERO,
                Color32::from_rgb(122, 78, 42),
            );
        }
        _ => paint_icon(painter, icon, rect, Color32::BLACK),
    }
}

fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    if paint_breeze_icon(painter, icon_theme_name(icon), rect, Color32::WHITE) {
        return;
    }
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
