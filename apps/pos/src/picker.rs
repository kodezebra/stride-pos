//! Choosing a file to read, or a folder to write into.
//!
//! The till reads a catalog CSV and writes the books out. Asking for a path
//! assumes the operator knows what a path is. This picker walks one folder at a
//! time instead, and deliberately lives inside the app: a native dialog would
//! be a dependency the offline build does not carry, and it would look nothing
//! like the rest of the till.

use std::path::{Path, PathBuf};

use ui_kit::components;
use ui_kit::tokens::{SPACING, TYPE};
use ui_shell::egui;
use ui_shell::regions;

use crate::state::{ImportDraft, State};

/// What the chosen path is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A `.csv` to read into the catalog.
    Import,
    /// A folder to write one of the exports into.
    Export(ExportKind),
}

impl Purpose {
    /// Whether the list shows files. An export only needs a folder, and a row
    /// that cannot be clicked would be a dead end on a touchscreen.
    fn lists_files(self) -> bool {
        matches!(self, Self::Import)
    }

    fn action(self) -> &'static str {
        match self {
            Self::Import => "Import",
            Self::Export(_) => "Save here",
        }
    }
}

/// Which file an export writes. The filename carries the difference, so a
/// folder holding both catalog exports is unambiguous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Products,
    AllProducts,
    Sales,
    /// A CSV to start an import from, not a list of what is here now.
    Template,
}

impl ExportKind {
    fn heading(self) -> &'static str {
        match self {
            Self::Products => "Save the products on screen",
            Self::AllProducts => "Save the whole catalog",
            Self::Sales => "Save the sales export",
            Self::Template => "Save an import template",
        }
    }
}

/// One row in the list.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// One shortcut above the list.
#[derive(Clone, Debug)]
struct Place {
    label: &'static str,
    path: PathBuf,
}

/// The file chooser, open as a modal over the app.
pub struct Picker {
    pub purpose: Purpose,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: Option<PathBuf>,
    pub error: Option<String>,
    places: Vec<Place>,
    /// True for the frame the picker was opened in. The Enter that pressed the
    /// button that opened it is still in this frame's input; without this guard
    /// it would also confirm the default choice.
    just_opened: bool,
}

impl Picker {
    pub fn open(purpose: Purpose, start: PathBuf, till: &Path) -> Self {
        let dir = if start.is_dir() {
            start
        } else {
            till.to_path_buf()
        };
        let mut picker = Self {
            purpose,
            dir: PathBuf::new(),
            entries: Vec::new(),
            selected: None,
            error: None,
            places: places(purpose, till),
            just_opened: true,
        };
        picker.go(dir);
        picker
    }

    /// Enter `dir`: list it, and forget the previous selection, which belonged
    /// to the folder we just left.
    pub fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.selected = None;
        let (entries, error) = read_entries(&self.dir, self.purpose.lists_files());
        self.entries = entries;
        self.error = error;
    }

    /// Go to the parent folder, when there is one.
    pub fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.go(parent);
        }
    }

    /// The path a confirm would use: the selected file for an import, the open
    /// folder for an export.
    pub fn choice(&self) -> Option<&Path> {
        match self.purpose {
            Purpose::Import => self.selected.as_deref(),
            Purpose::Export(_) => Some(self.dir.as_path()),
        }
    }
}

/// Where a picker should open.
///
/// The remembered folder wins while it still exists; otherwise Documents, then
/// Home, then the till's own data folder. A folder that was renamed away is not
/// an error to show on open — it is a reason to start somewhere that works.
pub fn start_dir(last: Option<&Path>, till: &Path) -> PathBuf {
    last.filter(|path| path.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| {
            let dirs = directories::UserDirs::new()?;
            dirs.document_dir()
                .filter(|path| path.is_dir())
                .map(Path::to_path_buf)
                .or_else(|| Some(dirs.home_dir().to_path_buf()).filter(|path| path.is_dir()))
        })
        .unwrap_or_else(|| till.to_path_buf())
}

fn places(purpose: Purpose, till: &Path) -> Vec<Place> {
    let mut places = Vec::new();

    if let Some(dirs) = directories::UserDirs::new() {
        let shortcuts = [
            ("Home", Some(dirs.home_dir())),
            ("Documents", dirs.document_dir()),
            ("Downloads", dirs.download_dir()),
            ("Desktop", dirs.desktop_dir()),
        ];
        for (label, path) in shortcuts {
            push(&mut places, label, path);
        }
    }

    match purpose {
        Purpose::Import => push(&mut places, "Till files", Some(till)),
        Purpose::Export(_) => {
            let exports = till.join("exports");
            push(&mut places, "Till exports", Some(&exports));
        }
    }

    places
}

fn push(places: &mut Vec<Place>, label: &'static str, path: Option<&Path>) {
    let Some(path) = path.filter(|path| path.is_dir()) else {
        return;
    };
    if places.iter().any(|place| place.path == path) {
        return;
    }
    places.push(Place {
        label,
        path: path.to_path_buf(),
    });
}

/// List one folder: folders first, then (when asked for) CSV files, each group
/// by name. Hidden names are skipped because they are never the file an owner
/// means to pick.
fn read_entries(dir: &Path, files: bool) -> (Vec<Entry>, Option<String>) {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Could not open this folder. {error}")),
            );
        }
    };

    let mut entries = Vec::new();
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        let is_dir = path.is_dir();
        if !is_dir && (!files || !is_csv(&path)) {
            continue;
        }
        entries.push(Entry { path, name, is_dir });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    (entries, None)
}

fn is_csv(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
}

/// Apply the picker's choice.
///
/// Called after the modal has been taken down: importing and exporting both
/// read the whole state, and doing that while the picker still borrows it is
/// the kind of split borrow the compiler is right to refuse.
pub(crate) fn confirm(state: &mut State) {
    let Some(picker) = state.picker.take() else {
        return;
    };

    match picker.purpose {
        Purpose::Import => {
            let Some(file) = picker.selected else {
                state.picker = Some(picker);
                return;
            };
            state.import_dir = file.parent().map(Path::to_path_buf);
            state.import = Some(ImportDraft {
                path: file.display().to_string(),
                error: None,
            });
            state.import_products();
        }
        Purpose::Export(kind) => {
            state.export_dir = Some(picker.dir.clone());
            crate::modules::run_export(state, kind, &picker.dir);
        }
    }
}

/// The chooser, drawn with the other foreground layers in `shell.rs`.
pub fn picker_modal(ctx: &egui::Context, state: &mut State) {
    let Some(picker) = state.picker.as_ref() else {
        return;
    };

    let purpose = picker.purpose;
    let fresh = picker.just_opened;
    let mut go: Option<PathBuf> = None;
    let mut select: Option<PathBuf> = None;
    let mut up = false;
    let mut confirm_pick = false;
    let mut cancel = false;
    let mut type_path = false;
    let mut template = false;

    crate::modules::modal(ctx, egui::Id::new("pos_file_picker")).show(ctx, |ui| {
        ui.set_width(520.0);

        let (heading, caption) = match purpose {
            Purpose::Import => (
                "Import products from CSV",
                "Pick the file to read. Rows are read as name, category, price and an \
                 optional barcode; prices are read in this till's currency. No file yet? \
                 Save a template and fill it in.",
            ),
            Purpose::Export(ExportKind::Template) => (
                ExportKind::Template.heading(),
                "Pick the folder to save into. The template keeps this name, so saving \
                 it again replaces the last one instead of piling up copies.",
            ),
            Purpose::Export(kind) => (
                kind.heading(),
                "Pick the folder to write into. The file is named after this moment, \
                 so an earlier export in the same folder is never overwritten.",
            ),
        };
        ui.label(components::semibold(heading).size(TYPE.heading));
        components::caption(ui, caption);

        ui.add_space(SPACING.md);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING.xs;
            for place in &picker.places {
                if ui.button(place.label).clicked() {
                    go = Some(place.path.clone());
                }
            }
        });

        ui.add_space(SPACING.sm);
        components::field_label(ui, "Folder");
        ui.add_space(SPACING.xs);
        components::caption(ui, &picker.dir.display().to_string());
        ui.add_space(SPACING.xs);
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("file_picker")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if picker.dir.parent().is_some()
                    && regions::nav_item(ui, false, "Up one folder").clicked()
                {
                    up = true;
                }

                if picker.entries.is_empty() {
                    components::caption(
                        ui,
                        match purpose {
                            Purpose::Import => "No folders or CSV files here.",
                            Purpose::Export(_) => "No folders here.",
                        },
                    );
                }

                for entry in &picker.entries {
                    let selected = picker.selected.as_deref() == Some(entry.path.as_path());
                    let label = if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name.clone()
                    };
                    if regions::nav_item(ui, selected, &label).clicked() {
                        if entry.is_dir {
                            go = Some(entry.path.clone());
                        } else {
                            select = Some(entry.path.clone());
                        }
                    }
                }
            });

        if let Some(error) = &picker.error {
            ui.add_space(SPACING.sm);
            ui.label(
                egui::RichText::new(error)
                    .size(TYPE.small)
                    .color(ui.visuals().error_fg_color),
            );
        }

        ui.add_space(SPACING.md);
        let chosen = match purpose {
            Purpose::Import => match picker.selected.as_deref() {
                Some(path) => format!("Selected: {}", path.display()),
                None => "Pick a CSV file.".to_owned(),
            },
            Purpose::Export(_) => "The file is written into the folder above.".to_owned(),
        };
        components::caption(ui, &chosen);

        ui.add_space(SPACING.md);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(picker.choice().is_some(), |ui| {
                if components::primary_button(ui, purpose.action()).clicked() {
                    confirm_pick = true;
                }
            });
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            if purpose == Purpose::Import && ui.button("Type the path instead").clicked() {
                type_path = true;
            }
            if purpose == Purpose::Import
                && ui
                    .button("Save a template")
                    .on_hover_text("Save a CSV with these columns and two example rows")
                    .clicked()
            {
                template = true;
            }
        });
    });

    // Keyboard, matching the rest of the till. The freshly opened frame is
    // skipped so the keystroke that opened the picker cannot also confirm it.
    if !fresh {
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            confirm_pick = true;
        }
    }

    let moved = go.is_some() || select.is_some() || up;

    if up && let Some(picker) = state.picker.as_mut() {
        picker.up();
    }
    if let Some(dir) = go
        && let Some(picker) = state.picker.as_mut()
    {
        picker.go(dir);
    }
    if let Some(file) = select
        && let Some(picker) = state.picker.as_mut()
    {
        picker.selected = Some(file);
    }

    if cancel {
        state.picker = None;
    } else if type_path {
        state.picker = None;
        state.start_import();
    } else if template {
        // The picker is already a folder chooser when it is exporting; reuse it
        // rather than opening the import modal only to close it again.
        state.picker = None;
        state.start_export(ExportKind::Template);
    } else if confirm_pick && !moved {
        confirm(state);
    }

    if let Some(picker) = state.picker.as_mut() {
        picker.just_opened = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pos-picker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, "").expect("write file");
    }

    #[test]
    fn folders_come_before_files_and_hidden_names_stay_out() {
        let dir = scratch("order");
        std::fs::create_dir(dir.join("folder")).expect("folder");
        touch(&dir.join("b.csv"));
        touch(&dir.join("A.csv"));
        touch(&dir.join("notes.txt"));
        touch(&dir.join(".hidden.csv"));

        let (entries, error) = read_entries(&dir, true);

        assert!(error.is_none());
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["folder", "A.csv", "b.csv"]);
        assert!(entries[0].is_dir);
    }

    #[test]
    fn folder_mode_hides_the_files() {
        let dir = scratch("folders");
        std::fs::create_dir(dir.join("folder")).expect("folder");
        touch(&dir.join("products.csv"));

        let (entries, _) = read_entries(&dir, false);

        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_dir);
    }

    #[test]
    fn up_lands_in_the_parent_and_forgets_the_choice() {
        let root = scratch("up");
        let child = root.join("child");
        std::fs::create_dir(&child).expect("child");
        touch(&child.join("products.csv"));

        let mut picker = Picker::open(Purpose::Import, child.clone(), &root);
        picker.selected = Some(child.join("products.csv"));
        assert!(picker.choice().is_some());

        picker.up();

        assert_eq!(picker.dir, root);
        assert!(picker.selected.is_none(), "a choice from another folder");
    }

    #[test]
    fn a_folder_that_cannot_be_read_keeps_the_reason() {
        let dir = scratch("missing");
        let mut picker = Picker::open(Purpose::Import, dir.clone(), &dir);
        let gone = dir.join("gone");

        picker.go(gone.clone());

        assert_eq!(picker.dir, gone);
        assert!(picker.entries.is_empty());
        assert!(
            picker
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Could not open")),
            "the operator needs a reason, not an empty list"
        );
    }

    #[test]
    fn an_import_needs_a_file_but_an_export_does_not() {
        let dir = scratch("choice");
        touch(&dir.join("products.csv"));

        let mut import = Picker::open(Purpose::Import, dir.clone(), &dir);
        assert!(import.choice().is_none());
        import.selected = Some(dir.join("products.csv"));
        assert_eq!(import.choice(), Some(dir.join("products.csv").as_path()));

        let export = Picker::open(Purpose::Export(ExportKind::Products), dir.clone(), &dir);
        assert_eq!(export.choice(), Some(dir.as_path()));
    }

    #[test]
    fn start_dir_falls_back_when_the_remembered_folder_is_gone() {
        let till = scratch("start");
        let existing = till.join("existing");
        std::fs::create_dir(&existing).expect("folder");
        let gone = till.join("gone");

        assert_eq!(start_dir(Some(&existing), &till), existing);
        assert!(start_dir(Some(&gone), &till).is_dir());
    }
}
