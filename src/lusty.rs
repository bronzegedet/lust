use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use rfd::FileDialog;
use lust::ast::Decl;
use lust::bytecode::Value;
use lust::bytecode_compiler::BytecodeCompiler;
use lust::lexer::Lexer;
use lust::parser::Parser;
use lust::typecheck::TypeChecker;
use lust::vm::{Vm, VmMemorySnapshot};

fn parse_program(lust_code: &str) -> Result<Vec<Decl>, String> {
    let mut lexer = Lexer::new(lust_code);
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }
    let mut parser = Parser::new(tokens);
    let decls = parser.parse();
    if !parser.errors.is_empty() {
        return Err(parser.errors.join("\n"));
    }
    Ok(decls)
}

fn std_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = std::env::var_os("LUST_HOME") {
        roots.push(PathBuf::from(home));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
            if let Some(grandparent) = parent.parent() {
                roots.push(grandparent.to_path_buf());
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    let mut deduped = Vec::new();
    for root in roots {
        if !deduped.iter().any(|existing: &PathBuf| existing == &root) {
            deduped.push(root);
        }
    }
    deduped
}

fn std_module_path(name: &str) -> Option<PathBuf> {
    let rest = name.strip_prefix("std/")?;
    let module_suffix = PathBuf::from("lust_src")
        .join("std")
        .join(format!("{}.lust", rest));

    for root in std_search_roots() {
        let candidate = root.join(&module_suffix);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(module_suffix))
}

fn parse_program_with_imports(
    lust_code: &str,
    base_dir: Option<&Path>,
    visited_modules: &mut HashSet<PathBuf>,
) -> Result<Vec<Decl>, String> {
    let decls = parse_program(lust_code)?;
    expand_imports(&decls, base_dir, visited_modules)
}

fn expand_imports(
    decls: &[Decl],
    base_dir: Option<&Path>,
    visited_modules: &mut HashSet<PathBuf>,
) -> Result<Vec<Decl>, String> {
    let mut merged = Vec::new();

    for decl in decls {
        if let Decl::Import(name) = decl {
            let module_path = if let Some(std_path) = std_module_path(name) {
                Some(std_path)
            } else if let Some(dir) = base_dir {
                let local_path = dir.join(format!("{}.lust", name));
                if local_path.exists() {
                    Some(local_path)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(module_path) = module_path {
                let canonical = module_path.canonicalize().map_err(|_| {
                    format!(
                        "Lust Error: module '{}' not found at {}",
                        name,
                        module_path.display()
                    )
                })?;

                if visited_modules.contains(&canonical) {
                    continue;
                }
                visited_modules.insert(canonical.clone());

                let source = fs::read_to_string(&canonical).map_err(|err| {
                    format!(
                        "Lust Error: failed to read module '{}': {}",
                        canonical.display(),
                        err
                    )
                })?;

                let mut module_decls =
                    parse_program_with_imports(&source, canonical.parent(), visited_modules)?;
                merged.append(&mut module_decls);
                continue;
            }
        }

        merged.push(decl.clone());
    }

    Ok(merged)
}

fn run_vm(
    script_path: &Path,
    source: &str,
    args: Vec<String>,
    ui_state: Option<HashMap<String, Value>>,
) -> Result<(HashMap<String, Value>, Vec<String>, VmMemorySnapshot), String> {
    let mut visited_modules = HashSet::new();
    let decls = parse_program_with_imports(source, script_path.parent(), &mut visited_modules)?;
    let type_info = TypeChecker::new()
        .check(&decls)
        .map_err(|errs| errs.join("\n"))?;
    let chunk = BytecodeCompiler::new(type_info)
        .compile(&decls)
        .map_err(|errs| {
            errs.into_iter()
                .map(|err| err.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })?;

    let mut vm = Vm::new_with_args_keys_and_input(chunk, args, Vec::new(), Vec::new());
    if let Some(state) = ui_state {
        vm.restore_ui_state(state)?;
    }
    vm.run()?;
    Ok((vm.ui_state_snapshot(), vm.output().to_vec(), vm.memory_snapshot()))
}

struct EditorTab {
    path: PathBuf,
    path_input: String,
    source: String,
    ui_state: HashMap<String, Value>,
    output_lines: Vec<String>,
    diagnostics: Vec<String>,
    first_error_line: Option<usize>,
    memory_line: String,
    dirty: bool,
    last_edit_at: Instant,
}

impl EditorTab {
    fn from_path(path: PathBuf) -> Self {
        let source = fs::read_to_string(&path).unwrap_or_else(|_| {
            "fn main()\n    print(\"hello from lusty\")\nend\n\nmain()\n".to_string()
        });
        Self {
            path: path.clone(),
            path_input: path.display().to_string(),
            source,
            ui_state: HashMap::new(),
            output_lines: Vec::new(),
            diagnostics: Vec::new(),
            first_error_line: None,
            memory_line: String::new(),
            dirty: false,
            last_edit_at: Instant::now(),
        }
    }

    fn title(&self) -> String {
        let base = self
            .path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("untitled");
        if self.dirty {
            format!("{}*", base)
        } else {
            base.to_string()
        }
    }
}

struct LustyApp {
    tabs: Vec<EditorTab>,
    active_tab: usize,
    args: Vec<String>,
    auto_run: bool,
    debounce: Duration,
    theme_applied: bool,
    preview_mode: PreviewMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewMode {
    Auto,
    Ui,
    Terminal,
}

impl LustyApp {
    fn new(path: PathBuf, args: Vec<String>) -> Self {
        let mut app = Self {
            tabs: vec![EditorTab::from_path(path)],
            active_tab: 0,
            args,
            auto_run: true,
            debounce: Duration::from_millis(250),
            theme_applied: false,
            preview_mode: PreviewMode::Auto,
        };
        app.run_tab(0);
        app
    }

    fn active_tab(&self) -> Option<&EditorTab> {
        self.tabs.get(self.active_tab)
    }

    fn active_tab_mut(&mut self) -> Option<&mut EditorTab> {
        self.tabs.get_mut(self.active_tab)
    }

    fn run_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match run_vm(
            &tab.path,
            &tab.source,
            self.args.clone(),
            Some(tab.ui_state.clone()),
        ) {
            Ok((state, output, memory)) => {
                tab.ui_state = state;
                tab.output_lines = output;
                tab.memory_line = format!(
                    "mem stack={}/{} globals={}/{} ui={}/{} list_alloc={} map_alloc={}",
                    memory.stack_len,
                    memory.stack_peak,
                    memory.globals_len,
                    memory.globals_peak,
                    memory.ui_state_len,
                    memory.ui_state_peak,
                    memory.list_allocations,
                    memory.map_allocations
                );
                tab.diagnostics.clear();
                tab.diagnostics.push("run ok".to_string());
                tab.first_error_line = None;
            }
            Err(err) => {
                tab.diagnostics = err.lines().map(|line| line.to_string()).collect();
                tab.first_error_line = extract_first_line_number(&err);
            }
        }
    }

    fn save_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match fs::write(&tab.path, &tab.source) {
            Ok(()) => {
                tab.diagnostics
                    .push(format!("saved {}", tab.path.display()));
                tab.dirty = false;
            }
            Err(err) => tab.diagnostics.push(format!("save failed: {}", err)),
        }
    }

    fn reload_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match fs::read_to_string(&tab.path) {
            Ok(text) => {
                tab.source = text;
                tab.dirty = false;
                tab.diagnostics
                    .push(format!("reloaded {}", tab.path.display()));
            }
            Err(err) => tab.diagnostics.push(format!("reload failed: {}", err)),
        }
        self.run_tab(index);
    }

    fn open_from_active_input(&mut self) {
        let Some(active) = self.active_tab() else {
            return;
        };
        let trimmed = active.path_input.trim();
        if trimmed.is_empty() {
            if let Some(tab) = self.active_tab_mut() {
                tab.diagnostics.push("open failed: empty path".to_string());
            }
            return;
        }
        let candidate = PathBuf::from(trimmed);
        self.open_path(candidate);
    }

    fn open_via_dialog(&mut self) {
        let start_dir = self.active_tab().and_then(|tab| {
            let base = if tab.path.is_absolute() {
                tab.path.clone()
            } else {
                std::env::current_dir().ok()?.join(&tab.path)
            };
            base.parent()
                .filter(|dir| dir.is_dir())
                .map(|dir| dir.to_path_buf())
        });
        let mut dialog = FileDialog::new().add_filter("Lust", &["lust"]);
        if let Some(dir) = start_dir.or_else(|| std::env::current_dir().ok()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_path(path);
        }
    }

    fn open_path(&mut self, candidate: PathBuf) {
        let normalized = if candidate.is_absolute() {
            candidate.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&candidate))
                .unwrap_or(candidate.clone())
        };

        if !normalized.is_file() {
            if let Some(tab) = self.active_tab_mut() {
                tab.diagnostics
                    .push(format!("open failed: file not found {}", normalized.display()));
            }
            return;
        }

        if let Some((idx, _)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, tab)| tab.path == normalized)
        {
            self.active_tab = idx;
            return;
        }
        let mut tab = EditorTab::from_path(normalized.clone());
        tab.diagnostics
            .push(format!("opened {}", normalized.display()));
        self.tabs.push(tab);
        self.active_tab = self.tabs.len().saturating_sub(1);
        self.run_tab(self.active_tab);
    }

    fn new_tab(&mut self) {
        let idx = self.tabs.len() + 1;
        let path = PathBuf::from(format!("untitled_{}.lust", idx));
        self.tabs.push(EditorTab::from_path(path));
        self.active_tab = self.tabs.len().saturating_sub(1);
    }
}

impl eframe::App for LustyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            apply_pixel_theme(ctx);
            self.theme_applied = true;
        }

        let mut action_run = false;
        let mut action_save = false;
        let mut action_reload = false;
        let mut action_open_dialog = false;
        let mut action_open_path = false;
        let mut action_new_tab = false;
        let active_idx = self.active_tab;

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    action_run = true;
                }
                if ui.button("New Tab").clicked() {
                    action_new_tab = true;
                }
                let open_pressed = ui.button("Open").clicked();
                if ui.button("Save").clicked() {
                    action_save = true;
                }
                if ui.button("Reload").clicked() {
                    action_reload = true;
                }
                ui.checkbox(&mut self.auto_run, "Auto-run");
                ui.separator();
                ui.label("Preview:");
                egui::ComboBox::from_id_salt("preview_mode")
                    .selected_text(match self.preview_mode {
                        PreviewMode::Auto => "Auto",
                        PreviewMode::Ui => "UI",
                        PreviewMode::Terminal => "Terminal",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.preview_mode, PreviewMode::Auto, "Auto");
                        ui.selectable_value(&mut self.preview_mode, PreviewMode::Ui, "UI");
                        ui.selectable_value(
                            &mut self.preview_mode,
                            PreviewMode::Terminal,
                            "Terminal",
                        );
                    });
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    let open_input = ui.add(
                        egui::TextEdit::singleline(&mut tab.path_input)
                            .desired_width(360.0),
                    );
                    if open_pressed
                        || (open_input.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    {
                        if open_pressed {
                            action_open_dialog = true;
                        } else {
                            action_open_path = true;
                        }
                    }
                    ui.label(format!("file: {}", tab.path.display()));
                    if let Some(line) = tab.first_error_line {
                        ui.separator();
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 110, 110),
                            format!("error line {}", line),
                        );
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                for (idx, tab) in self.tabs.iter().enumerate() {
                    if ui
                        .selectable_label(idx == self.active_tab, tab.title())
                        .clicked()
                    {
                        self.active_tab = idx;
                    }
                }
            });
        });

        egui::SidePanel::right("live_preview")
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                let preview_mode = self.preview_mode;
                ui.heading("Preview");
                ui.separator();
                if let Some(tab) = self.active_tab_mut() {
                    let has_widgets = has_widget_controls(&tab.ui_state);
                    let show_ui_preview = match preview_mode {
                        PreviewMode::Ui => true,
                        PreviewMode::Terminal => false,
                        PreviewMode::Auto => has_widgets,
                    };

                    if show_ui_preview {
                        let interacted = render_widget_preview(ui, &mut tab.ui_state);
                        if interacted {
                            action_run = true;
                        }
                    } else {
                        ui.label(egui::RichText::new("Terminal Output").strong());
                        ui.separator();
                        if tab.output_lines.is_empty() {
                            ui.monospace("<no output>");
                        } else {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for line in &tab.output_lines {
                                    ui.monospace(line);
                                }
                            });
                        }
                    }

                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            draw_pixel_background(ui, rect);
            ui.heading("Editor");
            ui.separator();
            let active_tab_id = self.active_tab;
            if let Some(tab) = self.active_tab_mut() {
                let line_count = tab.source.lines().count().max(1);
                let editor_font = egui::FontId::monospace(14.0);
                let editor_line_height = ui.fonts(|fonts| fonts.row_height(&editor_font));
                egui::ScrollArea::both()
                    .id_salt(format!("lusty_editor_scroll_{}", active_tab_id))
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for line in 1..=line_count {
                                    let text = format!("{:>4}", line);
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(38.0, editor_line_height),
                                        egui::Sense::hover(),
                                    );
                                    let color = if tab.first_error_line == Some(line) {
                                        egui::Color32::from_rgb(220, 110, 110)
                                    } else {
                                        egui::Color32::from_rgb(40, 40, 40)
                                    };
                                    ui.painter().text(
                                        rect.right_center() + egui::vec2(-2.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        text,
                                        egui::FontId::monospace(14.0),
                                        color,
                                    );
                                }
                            });
                            ui.separator();
                            let mut layouter = |ui: &egui::Ui, string: &str, wrap_width: f32| {
                                layout_lust_code(ui, string, wrap_width, editor_line_height)
                            };
                            let editor = egui::TextEdit::multiline(&mut tab.source)
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .layouter(&mut layouter)
                                .desired_width(f32::INFINITY)
                                .desired_rows(line_count + 2);
                            if ui.add(editor).changed() {
                                tab.dirty = true;
                                tab.last_edit_at = Instant::now();
                            }
                        });
                    });
            }
        });

        if action_new_tab {
            self.new_tab();
        }
        if action_open_dialog {
            self.open_via_dialog();
        }
        if action_open_path {
            self.open_from_active_input();
        }
        if action_save {
            self.save_tab(self.active_tab);
        }
        if action_reload {
            self.reload_tab(self.active_tab);
        }
        if action_run {
            self.run_tab(self.active_tab);
            if let Some(tab) = self.active_tab_mut() {
                tab.dirty = false;
            }
        } else if self.auto_run {
            let should_run = self
                .tabs
                .get(self.active_tab)
                .map(|tab| tab.dirty && tab.last_edit_at.elapsed() >= self.debounce)
                .unwrap_or(false);
            if should_run {
                self.run_tab(self.active_tab);
                if let Some(tab) = self.active_tab_mut() {
                    tab.dirty = false;
                }
            }
        }

        if self.tabs.is_empty() {
            self.new_tab();
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        } else if self.active_tab != active_idx && self.active_tab < self.tabs.len() {
            // ensure newly selected tab has a recent run state
            let needs_bootstrap = self.tabs[self.active_tab].memory_line.is_empty();
            if needs_bootstrap {
                self.run_tab(self.active_tab);
            }
        }
    }
}

fn is_internal_widget_id(id: &str) -> bool {
    id.starts_with("editor.")
        || id.starts_with("document.")
        || id.starts_with("selection.")
        || id.starts_with("commands.")
        || id.starts_with("layout.")
        || id.starts_with("host.")
        || id.starts_with("cursor.")
        || id.starts_with("scroll.")
        || id.starts_with("preview.")
        || id.starts_with("__")
}

fn has_widget_controls(state: &HashMap<String, Value>) -> bool {
    state.keys().any(|key| {
        (key.strip_prefix("field.")
            .or_else(|| key.strip_prefix("toggle."))
            .or_else(|| key.strip_prefix("slider."))
            .or_else(|| key.strip_prefix("knob."))
            .or_else(|| key.strip_prefix("button."))
            .is_some_and(|id| !is_internal_widget_id(id)))
            || key.starts_with("widget.section.")
            || key.starts_with("widget.label.")
            || key.starts_with("widget.button.")
    })
}

fn layout_lust_code(ui: &egui::Ui, text: &str, wrap_width: f32, line_height: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let _ = wrap_width;
    // Keep code editor on hard lines; soft-wrap introduces visual rows that break gutter alignment.
    job.wrap.max_width = f32::INFINITY;

    let default = egui::TextFormat {
        font_id: egui::FontId::monospace(14.0),
        color: ui.visuals().text_color(),
        line_height: Some(line_height),
        ..Default::default()
    };
    let keyword = egui::TextFormat {
        color: egui::Color32::from_rgb(46, 66, 130),
        ..default.clone()
    };
    let string = egui::TextFormat {
        color: egui::Color32::from_rgb(145, 66, 30),
        ..default.clone()
    };
    let comment = egui::TextFormat {
        color: egui::Color32::from_rgb(82, 110, 82),
        ..default.clone()
    };

    for chunk in text.split_inclusive('\n') {
        append_highlighted_line(&mut job, chunk, &default, &keyword, &string, &comment);
    }

    if !text.ends_with('\n') && !text.is_empty() && !text.contains('\n') {
        // already handled by split_inclusive; keep for single-line compatibility
    }
    ui.fonts(|fonts| fonts.layout_job(job))
}

fn append_highlighted_line(
    job: &mut egui::text::LayoutJob,
    line: &str,
    default: &egui::TextFormat,
    keyword: &egui::TextFormat,
    string: &egui::TextFormat,
    comment: &egui::TextFormat,
) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            job.append(&line[i..], 0.0, comment.clone());
            break;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            job.append(&line[i..], 0.0, comment.clone());
            break;
        }
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            job.append(&line[start..i], 0.0, string.clone());
            continue;
        }
        let ch = bytes[i] as char;
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let token = &line[start..i];
            let is_keyword = matches!(
                token,
                "fn"
                    | "let"
                    | "if"
                    | "else"
                    | "while"
                    | "for"
                    | "in"
                    | "do"
                    | "end"
                    | "return"
                    | "match"
                    | "enum"
                    | "type"
                    | "import"
                    | "true"
                    | "false"
                    | "null"
                    | "and"
                    | "or"
                    | "not"
                    | "break"
                    | "continue"
                    | "pass"
                    | "spawn"
            );
            job.append(token, 0.0, if is_keyword { keyword.clone() } else { default.clone() });
            continue;
        }
        let start = i;
        i += 1;
        job.append(&line[start..i], 0.0, default.clone());
    }
}

fn apply_pixel_theme(ctx: &egui::Context) {
    ctx.set_pixels_per_point(1.0);
    let mut style = (*ctx.style()).clone();

    style.visuals = egui::Visuals::light();
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(34, 34, 34));
    style.visuals.window_fill = egui::Color32::from_rgb(214, 214, 214);
    style.visuals.panel_fill = egui::Color32::from_rgb(196, 196, 196);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(176, 176, 176);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(232, 232, 232);
    style.visuals.code_bg_color = egui::Color32::from_rgb(242, 242, 242);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(110, 130, 170);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(245, 245, 245));

    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(188, 188, 188);
    style.visuals.widgets.noninteractive.weak_bg_fill = egui::Color32::from_rgb(188, 188, 188);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(62, 62, 62));
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(32, 32, 32));

    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(226, 226, 226);
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(226, 226, 226);
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 58, 58));
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(22, 22, 22));

    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(238, 238, 238);
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(238, 238, 238);
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 42, 42));
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(18, 18, 18));

    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(162, 178, 202);
    style.visuals.widgets.active.weak_bg_fill = egui::Color32::from_rgb(162, 178, 202);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(38, 38, 38));
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(10, 10, 10));

    style.visuals.window_corner_radius = 0.0.into();
    style.visuals.menu_corner_radius = 0.0.into();
    style.visuals.widgets.noninteractive.corner_radius = 0.0.into();
    style.visuals.widgets.inactive.corner_radius = 0.0.into();
    style.visuals.widgets.hovered.corner_radius = 0.0.into();
    style.visuals.widgets.active.corner_radius = 0.0.into();
    style.visuals.widgets.open.corner_radius = 0.0.into();

    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.window_margin = egui::Margin::same(8);

    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::monospace(18.0)),
        (egui::TextStyle::Body, egui::FontId::monospace(14.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(14.0)),
        (egui::TextStyle::Button, egui::FontId::monospace(14.0)),
        (egui::TextStyle::Small, egui::FontId::monospace(12.0)),
    ]
    .into();

    ctx.set_style(style);
}

fn draw_pixel_background(ui: &mut egui::Ui, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(186, 186, 186));
    let step = 8.0;
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(120, 120, 120, 24)),
        );
        y += step;
    }
    let mut x = rect.left();
    while x < rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(120, 120, 120, 24)),
        );
        x += step;
    }
}

fn extract_first_line_number(message: &str) -> Option<usize> {
    for line in message.lines() {
        if let Some(pos) = line.find("line ") {
            let suffix = &line[pos + 5..];
            let digits = suffix
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(number) = digits.parse::<usize>() {
                if number > 0 {
                    return Some(number);
                }
            }
        }
    }
    None
}

fn render_widget_preview(ui: &mut egui::Ui, state: &mut HashMap<String, Value>) -> bool {
    let mut changed = false;

    if let Some(title) = state.get("app.title") {
        ui.heading(title.as_string());
    }

    let mut sections = state
        .iter()
        .filter_map(|(key, value)| key.strip_prefix("widget.section.").map(|id| (id.to_string(), value.as_string())))
        .collect::<Vec<_>>();
    sections.sort_by(|a, b| a.0.cmp(&b.0));
    for (_id, title) in sections {
        ui.separator();
        ui.label(egui::RichText::new(title).strong());
    }

    let mut fields = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("field.")
                .filter(|id| !is_internal_widget_id(id))
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, mut text) in fields {
        let label = state
            .get(&format!("widget.label.{}", id))
            .map(Value::as_string)
            .unwrap_or_else(|| id.clone());
        ui.horizontal(|ui| {
            ui.label(label);
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(180.0))
                .changed()
            {
                state.insert(format!("field.{}", id), Value::String(text.clone()));
                changed = true;
            }
        });
    }

    let mut toggles = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("toggle.")
                .filter(|id| !is_internal_widget_id(id))
                .map(|id| (id.to_string(), value.truthy()))
        })
        .collect::<Vec<_>>();
    toggles.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, mut on) in toggles {
        let label = state
            .get(&format!("widget.label.{}", id))
            .map(Value::as_string)
            .unwrap_or_else(|| id.clone());
        if ui.checkbox(&mut on, label).changed() {
            state.insert(format!("toggle.{}", id), Value::Bool(on));
            changed = true;
        }
    }

    let mut sliders = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("slider.")
                .filter(|id| !is_internal_widget_id(id))
                .map(|id| (id.to_string(), value.as_number()))
        })
        .collect::<Vec<_>>();
    sliders.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, value) in sliders {
        let label = state
            .get(&format!("widget.label.{}", id))
            .map(Value::as_string)
            .unwrap_or_else(|| id.clone());
        let mut preview = value as f32;
        if ui
            .add(egui::Slider::new(&mut preview, 0.0..=1.0).text(label))
            .changed()
        {
            state.insert(format!("slider.{}", id), Value::Number(preview as f64));
            changed = true;
        }
    }

    let mut knobs = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("knob.")
                .filter(|id| !is_internal_widget_id(id))
                .map(|id| (id.to_string(), value.as_number()))
        })
        .collect::<Vec<_>>();
    knobs.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, value) in knobs {
        let label = state
            .get(&format!("widget.label.{}", id))
            .map(Value::as_string)
            .unwrap_or_else(|| id.clone());
        let mut preview = value as f32;
        if ui
            .add(egui::Slider::new(&mut preview, 0.0..=1.0).text(format!("{} (knob)", label)))
            .changed()
        {
            state.insert(format!("knob.{}", id), Value::Number(preview as f64));
            changed = true;
        }
    }

    let mut buttons = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("button.")
                .filter(|id| !is_internal_widget_id(id))
                .map(|id| (id.to_string(), value.truthy()))
        })
        .collect::<Vec<_>>();
    buttons.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, pressed) in buttons {
        let label = state
            .get(&format!("widget.button.{}.label", id))
            .map(Value::as_string)
            .unwrap_or_else(|| id.clone());
        ui.horizontal(|ui| {
            let btn = egui::Button::new(label);
            if ui.add(btn).clicked() {
                state.insert(format!("button.{}", id), Value::Bool(true));
                changed = true;
            }
            let _ = pressed;
        });
    }
    changed
}

fn main() -> Result<(), eframe::Error> {
    let mut cli = std::env::args().skip(1).collect::<Vec<_>>();
    let path = if let Some(first) = cli.first() {
        PathBuf::from(first)
    } else {
        PathBuf::from("examples/ide/lusty.lust")
    };
    if !cli.is_empty() {
        cli.remove(0);
    }
    let app = LustyApp::new(path, cli);
    let native_options = eframe::NativeOptions::default();
    eframe::run_native("Lusty IDE", native_options, Box::new(|_cc| Ok(Box::new(app))))
}
