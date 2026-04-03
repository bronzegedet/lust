use std::io::{stdout, IsTerminal, Stdout, Write};
use std::path::Path;
use std::time::Duration;

use crossterm::cursor::MoveTo;
use crossterm::event::{
    poll, read, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{ExecutableCommand, QueueableCommand};

pub struct IdeShell {
    stdout: Stdout,
    source_lines: Vec<String>,
    preview_lines: Vec<String>,
    diag_lines: Vec<String>,
    status_line: String,
    path_label: String,
    focus_line: Option<usize>,
    theme: ShellTheme,
    active: bool,
    symbol_filter_mode: bool,
    symbol_filter_query: String,
    symbol_palette_active: bool,
    symbol_palette_lines: Vec<String>,
    symbol_palette_cursor: usize,
}

pub enum ShellAction {
    None,
    Quit,
    Reload,
    ToggleInspect,
    InspectNext,
    InspectPrev,
    SymbolPrev,
    SymbolNext,
    SymbolBack,
    SymbolForward,
    SymbolFilterChanged {
        query: String,
    },
    SymbolFilterPrev,
    SymbolFilterNext,
    SymbolFilterPagePrev,
    SymbolFilterPageNext,
    SymbolOpenSelected,
    SymbolPaletteClick {
        line_index: usize,
    },
    SymbolFilterCancel,
    PreviewPointer {
        kind: PreviewPointerKind,
        col: u16,
        row: u16,
        shift: bool,
    },
    SourceClick {
        line: usize,
        shift: bool,
    },
}

pub enum PreviewPointerKind {
    Down,
    Drag,
    Up,
}

#[derive(Clone)]
pub struct ShellTheme {
    pub name: String,
    pub show_borders: bool,
    pub h: char,
    pub v: char,
    pub corner: char,
    pub source_label: String,
    pub preview_label: String,
    pub diag_label: String,
}

impl IdeShell {
    pub fn start(path: &Path) -> Result<Option<Self>, String> {
        let mut out = stdout();
        if !out.is_terminal() {
            return Ok(None);
        }
        out.execute(EnterAlternateScreen)
            .map_err(|e| format!("ide shell enter alt screen failed: {}", e))?;
        enable_raw_mode().map_err(|e| format!("ide shell raw mode failed: {}", e))?;

        Ok(Some(Self {
            stdout: out,
            source_lines: Vec::new(),
            preview_lines: Vec::new(),
            diag_lines: Vec::new(),
            status_line: "starting".to_string(),
            path_label: path.display().to_string(),
            focus_line: None,
            theme: ShellTheme::default(),
            active: true,
            symbol_filter_mode: false,
            symbol_filter_query: String::new(),
            symbol_palette_active: false,
            symbol_palette_lines: Vec::new(),
            symbol_palette_cursor: 0,
        }))
    }

    pub fn set_source(&mut self, text: &str) {
        self.source_lines = text.lines().map(|line| line.to_string()).collect();
    }

    pub fn set_preview_lines(&mut self, lines: Vec<String>) {
        self.preview_lines = lines;
    }

    pub fn push_diag(&mut self, line: impl Into<String>) {
        self.diag_lines.push(line.into());
        if self.diag_lines.len() > 200 {
            let drop_count = self.diag_lines.len().saturating_sub(200);
            self.diag_lines.drain(0..drop_count);
        }
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status_line = status.into();
    }

    pub fn set_path_label(&mut self, path: impl Into<String>) {
        self.path_label = path.into();
    }

    pub fn set_focus_line(&mut self, line: Option<usize>) {
        self.focus_line = line.filter(|line| *line > 0);
    }

    pub fn apply_theme(&mut self, theme: ShellTheme) {
        self.theme = theme;
    }

    pub fn set_symbol_palette(
        &mut self,
        active: bool,
        lines: Vec<String>,
        cursor: usize,
    ) {
        self.symbol_palette_active = active;
        self.symbol_palette_lines = lines;
        self.symbol_palette_cursor = cursor;
    }

    pub fn render(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let (w, h) = size().map_err(|e| format!("ide shell size failed: {}", e))?;
        let width = w.max(40);
        let height = h.max(12);

        let split_x = width / 2;
        let diag_top = height.saturating_sub(6);

        self.stdout
            .queue(MoveTo(0, 0))
            .map_err(|e| format!("ide shell move failed: {}", e))?;
        self.stdout
            .queue(Clear(ClearType::All))
            .map_err(|e| format!("ide shell clear failed: {}", e))?;

        if self.theme.show_borders {
            self.draw_borders(width, height, split_x, diag_top)?;
        }
        self.draw_title(width)?;
        self.draw_source_pane(split_x, diag_top)?;
        self.draw_preview_pane(split_x, width, diag_top)?;
        self.draw_symbol_palette(split_x, width, diag_top)?;
        self.draw_diag_pane(width, height, diag_top)?;

        self.stdout
            .flush()
            .map_err(|e| format!("ide shell flush failed: {}", e))?;
        Ok(())
    }

    fn draw_title(&mut self, width: u16) -> Result<(), String> {
        let symbol_mode = if self.symbol_filter_mode {
            format!(" | symbol-filter: {}", self.symbol_filter_query)
        } else {
            String::new()
        };
        let title = format!(
            "lust ide [{}] | {} | {}{} | q quit / r reload / i inspect / j-k move / [-] symbols / b-n history / / filter / pgup-pgdn page",
            self.theme.name, self.path_label, self.status_line, symbol_mode
        );
        self.stdout
            .queue(MoveTo(0, 0))
            .map_err(|e| format!("ide shell title move failed: {}", e))?;
        self.stdout
            .queue(Print(truncate(&title, width as usize)))
            .map_err(|e| format!("ide shell title print failed: {}", e))?;
        Ok(())
    }

    fn draw_source_pane(&mut self, split_x: u16, diag_top: u16) -> Result<(), String> {
        self.stdout
            .queue(MoveTo(1, 1))
            .map_err(|e| format!("ide shell source header move failed: {}", e))?;
        self.stdout
            .queue(Print(&self.theme.source_label))
            .map_err(|e| format!("ide shell source header print failed: {}", e))?;

        let max_lines = diag_top.saturating_sub(2) as usize;
        let start = if let Some(focus) = self.focus_line {
            let focus_idx = focus.saturating_sub(1);
            focus_idx.saturating_sub(max_lines / 2)
        } else {
            self.source_lines.len().saturating_sub(max_lines)
        };
        let render_lines = &self.source_lines[start..];

        for (idx, line) in render_lines.iter().enumerate() {
            let y = 2 + idx as u16;
            let line_no = start + idx + 1;
            let marker = if self.focus_line == Some(line_no) { ">" } else { " " };
            let label = format!("{}{:>4} {}", marker, line_no, line);
            self.stdout
                .queue(MoveTo(1, y))
                .map_err(|e| format!("ide shell source line move failed: {}", e))?;
            self.stdout
                .queue(Print(truncate(&label, split_x.saturating_sub(3) as usize)))
                .map_err(|e| format!("ide shell source line print failed: {}", e))?;
        }
        Ok(())
    }

    fn draw_preview_pane(&mut self, split_x: u16, width: u16, diag_top: u16) -> Result<(), String> {
        self.stdout
            .queue(MoveTo(split_x + 2, 1))
            .map_err(|e| format!("ide shell preview header move failed: {}", e))?;
        self.stdout
            .queue(Print(&self.theme.preview_label))
            .map_err(|e| format!("ide shell preview header print failed: {}", e))?;

        let pane_width = width.saturating_sub(split_x + 4);
        let max_lines = diag_top.saturating_sub(2) as usize;
        let start = self.preview_lines.len().saturating_sub(max_lines);
        let render_lines = &self.preview_lines[start..];
        for (idx, line) in render_lines.iter().enumerate() {
            let y = 2 + idx as u16;
            self.stdout
                .queue(MoveTo(split_x + 2, y))
                .map_err(|e| format!("ide shell preview line move failed: {}", e))?;
            self.stdout
                .queue(Print(truncate(line, pane_width as usize)))
                .map_err(|e| format!("ide shell preview line print failed: {}", e))?;
        }
        Ok(())
    }

    fn draw_diag_pane(&mut self, width: u16, height: u16, diag_top: u16) -> Result<(), String> {
        self.stdout
            .queue(MoveTo(1, diag_top))
            .map_err(|e| format!("ide shell diag header move failed: {}", e))?;
        self.stdout
            .queue(Print(&self.theme.diag_label))
            .map_err(|e| format!("ide shell diag header print failed: {}", e))?;

        let max_lines = height.saturating_sub(diag_top + 1) as usize;
        let start = self.diag_lines.len().saturating_sub(max_lines);
        let render_lines = &self.diag_lines[start..];
        for (idx, line) in render_lines.iter().enumerate() {
            let y = diag_top + 1 + idx as u16;
            self.stdout
                .queue(MoveTo(1, y))
                .map_err(|e| format!("ide shell diag line move failed: {}", e))?;
            self.stdout
                .queue(Print(truncate(line, width.saturating_sub(2) as usize)))
                .map_err(|e| format!("ide shell diag line print failed: {}", e))?;
        }
        Ok(())
    }

    fn draw_symbol_palette(&mut self, split_x: u16, width: u16, diag_top: u16) -> Result<(), String> {
        if !self.symbol_palette_active {
            return Ok(());
        }
        let left = split_x + 2;
        let pane_width = width.saturating_sub(split_x + 4) as usize;
        let max_lines = diag_top.saturating_sub(2) as usize;
        if max_lines == 0 || pane_width == 0 {
            return Ok(());
        }
        self.stdout
            .queue(MoveTo(left, 1))
            .map_err(|e| format!("ide shell symbol header move failed: {}", e))?;
        self.stdout
            .queue(Print(truncate("Symbols", pane_width)))
            .map_err(|e| format!("ide shell symbol header print failed: {}", e))?;

        let start = self
            .symbol_palette_cursor
            .saturating_sub(max_lines.saturating_sub(1) / 2);
        let render_lines = &self.symbol_palette_lines[start.min(self.symbol_palette_lines.len())..];
        for (idx, line) in render_lines.iter().take(max_lines).enumerate() {
            let y = 2 + idx as u16;
            self.stdout
                .queue(MoveTo(left, y))
                .map_err(|e| format!("ide shell symbol line move failed: {}", e))?;
            self.stdout
                .queue(Print(truncate(line, pane_width)))
                .map_err(|e| format!("ide shell symbol line print failed: {}", e))?;
        }
        Ok(())
    }

    fn draw_borders(
        &mut self,
        width: u16,
        height: u16,
        split_x: u16,
        diag_top: u16,
    ) -> Result<(), String> {
        if width < 6 || height < 6 {
            return Ok(());
        }
        let hline = self.theme.h.to_string();
        let vline = self.theme.v.to_string();
        let corner = self.theme.corner.to_string();

        for x in 0..width {
            self.stdout
                .queue(MoveTo(x, 1))
                .map_err(|e| format!("ide shell border move failed: {}", e))?;
            self.stdout
                .queue(Print(&hline))
                .map_err(|e| format!("ide shell border print failed: {}", e))?;
            self.stdout
                .queue(MoveTo(x, height.saturating_sub(1)))
                .map_err(|e| format!("ide shell border move failed: {}", e))?;
            self.stdout
                .queue(Print(&hline))
                .map_err(|e| format!("ide shell border print failed: {}", e))?;
        }

        for y in 1..height {
            self.stdout
                .queue(MoveTo(0, y))
                .map_err(|e| format!("ide shell border move failed: {}", e))?;
            self.stdout
                .queue(Print(&vline))
                .map_err(|e| format!("ide shell border print failed: {}", e))?;
            self.stdout
                .queue(MoveTo(width.saturating_sub(1), y))
                .map_err(|e| format!("ide shell border move failed: {}", e))?;
            self.stdout
                .queue(Print(&vline))
                .map_err(|e| format!("ide shell border print failed: {}", e))?;
        }

        for y in 1..diag_top {
            self.stdout
                .queue(MoveTo(split_x, y))
                .map_err(|e| format!("ide shell split move failed: {}", e))?;
            self.stdout
                .queue(Print(&vline))
                .map_err(|e| format!("ide shell split print failed: {}", e))?;
        }

        for x in 0..width {
            self.stdout
                .queue(MoveTo(x, diag_top))
                .map_err(|e| format!("ide shell diag split move failed: {}", e))?;
            self.stdout
                .queue(Print(&hline))
                .map_err(|e| format!("ide shell diag split print failed: {}", e))?;
        }

        for (x, y) in [
            (0, 1),
            (width.saturating_sub(1), 1),
            (0, height.saturating_sub(1)),
            (width.saturating_sub(1), height.saturating_sub(1)),
            (split_x, 1),
            (split_x, diag_top),
            (0, diag_top),
            (width.saturating_sub(1), diag_top),
        ] {
            self.stdout
                .queue(MoveTo(x, y))
                .map_err(|e| format!("ide shell corner move failed: {}", e))?;
            self.stdout
                .queue(Print(&corner))
                .map_err(|e| format!("ide shell corner print failed: {}", e))?;
        }
        Ok(())
    }

    pub fn poll_action(&mut self) -> Result<ShellAction, String> {
        if !self.active {
            return Ok(ShellAction::None);
        }
        let has_event = poll(Duration::from_millis(0))
            .map_err(|e| format!("ide shell poll input failed: {}", e))?;
        if !has_event {
            return Ok(ShellAction::None);
        }
        let event = read().map_err(|e| format!("ide shell read input failed: {}", e))?;
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(ShellAction::None);
                }
                if self.symbol_filter_mode {
                    if key.code == KeyCode::Esc {
                        self.symbol_filter_mode = false;
                        self.symbol_filter_query.clear();
                        return Ok(ShellAction::SymbolFilterCancel);
                    }
                    if key.code == KeyCode::Enter {
                        self.symbol_filter_mode = false;
                        return Ok(ShellAction::SymbolOpenSelected);
                    }
                    if key.code == KeyCode::Up || key.code == KeyCode::Char('k') {
                        return Ok(ShellAction::SymbolFilterPrev);
                    }
                    if key.code == KeyCode::Down || key.code == KeyCode::Char('j') {
                        return Ok(ShellAction::SymbolFilterNext);
                    }
                    if key.code == KeyCode::Backspace {
                        self.symbol_filter_query.pop();
                        return Ok(ShellAction::SymbolFilterChanged {
                            query: self.symbol_filter_query.clone(),
                        });
                    }
                    if key.code == KeyCode::PageUp {
                        return Ok(ShellAction::SymbolFilterPagePrev);
                    }
                    if key.code == KeyCode::PageDown {
                        return Ok(ShellAction::SymbolFilterPageNext);
                    }
                    if let KeyCode::Char(ch) = key.code {
                        if !ch.is_control() {
                            self.symbol_filter_query.push(ch);
                            return Ok(ShellAction::SymbolFilterChanged {
                                query: self.symbol_filter_query.clone(),
                            });
                        }
                    }
                    return Ok(ShellAction::None);
                }
                if key.code == KeyCode::Char('q') {
                    return Ok(ShellAction::Quit);
                }
                if key.code == KeyCode::Esc {
                    return Ok(ShellAction::Quit);
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(ShellAction::Quit);
                }
                if key.code == KeyCode::Char('r') {
                    return Ok(ShellAction::Reload);
                }
                if key.code == KeyCode::Char('i') {
                    return Ok(ShellAction::ToggleInspect);
                }
                if key.code == KeyCode::Char('/') {
                    self.symbol_filter_mode = true;
                    self.symbol_filter_query.clear();
                    return Ok(ShellAction::SymbolFilterChanged {
                        query: self.symbol_filter_query.clone(),
                    });
                }
                if key.code == KeyCode::Char('[') {
                    return Ok(ShellAction::SymbolPrev);
                }
                if key.code == KeyCode::Char(']') {
                    return Ok(ShellAction::SymbolNext);
                }
                if key.code == KeyCode::Char('b') {
                    return Ok(ShellAction::SymbolBack);
                }
                if key.code == KeyCode::Char('n') {
                    return Ok(ShellAction::SymbolForward);
                }
                if key.code == KeyCode::Enter {
                    return Ok(ShellAction::SymbolOpenSelected);
                }
                if key.code == KeyCode::Char('j') || key.code == KeyCode::Down {
                    return Ok(ShellAction::InspectNext);
                }
                if key.code == KeyCode::Char('k') || key.code == KeyCode::Up {
                    return Ok(ShellAction::InspectPrev);
                }
            }
            Event::Mouse(mouse) => {
                let pointer_kind = match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => Some(PreviewPointerKind::Down),
                    MouseEventKind::Drag(MouseButton::Left) => Some(PreviewPointerKind::Drag),
                    MouseEventKind::Up(MouseButton::Left) => Some(PreviewPointerKind::Up),
                    _ => None,
                };
                if let Some(kind) = pointer_kind {
                    let (w, h) = size().map_err(|e| format!("ide shell size failed: {}", e))?;
                    let width = w.max(40);
                    let height = h.max(12);
                    let split_x = width / 2;
                    let diag_top = height.saturating_sub(6);
                    let preview_left = split_x + 2;
                    let preview_top = 2;
                    let preview_right = width.saturating_sub(2);
                    let preview_bottom = diag_top.saturating_sub(1);
                    let x = mouse.column;
                    let y = mouse.row;
                    if x >= preview_left && x <= preview_right && y >= preview_top && y <= preview_bottom {
                        return Ok(ShellAction::PreviewPointer {
                            kind,
                            col: x.saturating_sub(preview_left),
                            row: y.saturating_sub(preview_top),
                            shift: mouse.modifiers.contains(KeyModifiers::SHIFT),
                        });
                    }
                }
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    let (w, h) = size().map_err(|e| format!("ide shell size failed: {}", e))?;
                    let width = w.max(40);
                    let height = h.max(12);
                    let split_x = width / 2;
                    let diag_top = height.saturating_sub(6);
                    if self.symbol_palette_active {
                        let palette_left = split_x + 2;
                        let palette_top = 2;
                        let palette_right = width.saturating_sub(2);
                        let palette_bottom = diag_top.saturating_sub(1);
                        let x = mouse.column;
                        let y = mouse.row;
                        if x >= palette_left
                            && x <= palette_right
                            && y >= palette_top
                            && y <= palette_bottom
                        {
                            let pane_width = width.saturating_sub(split_x + 4) as usize;
                            let max_lines = diag_top.saturating_sub(2) as usize;
                            if max_lines > 0 && pane_width > 0 && !self.symbol_palette_lines.is_empty() {
                                let start = self
                                    .symbol_palette_cursor
                                    .saturating_sub(max_lines.saturating_sub(1) / 2)
                                    .min(self.symbol_palette_lines.len());
                                let clicked_visible = y.saturating_sub(palette_top) as usize;
                                let line_index = start + clicked_visible;
                                if line_index < self.symbol_palette_lines.len() {
                                    return Ok(ShellAction::SymbolPaletteClick { line_index });
                                }
                            }
                        }
                    }
                    let source_left = 1;
                    let source_right = split_x.saturating_sub(2);
                    let source_top = 2;
                    let source_bottom = diag_top.saturating_sub(1);
                    let x = mouse.column;
                    let y = mouse.row;
                    if x >= source_left && x <= source_right && y >= source_top && y <= source_bottom {
                        let max_lines = diag_top.saturating_sub(2) as usize;
                        let start = if let Some(focus) = self.focus_line {
                            let focus_idx = focus.saturating_sub(1);
                            focus_idx.saturating_sub(max_lines / 2)
                        } else {
                            self.source_lines.len().saturating_sub(max_lines)
                        };
                        let row_index = y.saturating_sub(source_top) as usize;
                        let line = start + row_index + 1;
                        if line <= self.source_lines.len() {
                            return Ok(ShellAction::SourceClick {
                                line,
                                shift: mouse.modifiers.contains(KeyModifiers::SHIFT),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(ShellAction::None)
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        disable_raw_mode().map_err(|e| format!("ide shell disable raw failed: {}", e))?;
        self.stdout
            .execute(LeaveAlternateScreen)
            .map_err(|e| format!("ide shell leave alt screen failed: {}", e))?;
        self.active = false;
        Ok(())
    }
}

impl Drop for IdeShell {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            let _ = self.stdout.execute(LeaveAlternateScreen);
            self.active = false;
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

impl Default for ShellTheme {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            show_borders: true,
            h: '-',
            v: '|',
            corner: '+',
            source_label: "SOURCE".to_string(),
            preview_label: "PREVIEW / VM OUTPUT".to_string(),
            diag_label: "DIAGNOSTICS".to_string(),
        }
    }
}

impl ShellTheme {
    pub fn from_lust(theme_name: &str, settings: &std::collections::HashMap<String, String>) -> Self {
        let mut theme = if theme_name == "classic" {
            Self {
                name: "classic".to_string(),
                show_borders: true,
                h: '=',
                v: '|',
                corner: '#',
                source_label: "Source".to_string(),
                preview_label: "Preview".to_string(),
                diag_label: "Console".to_string(),
            }
        } else {
            Self::default()
        };

        if let Some(h) = settings.get("h").and_then(|s| s.chars().next()) {
            theme.h = h;
        }
        if let Some(v) = settings.get("v").and_then(|s| s.chars().next()) {
            theme.v = v;
        }
        if let Some(corner) = settings.get("corner").and_then(|s| s.chars().next()) {
            theme.corner = corner;
        }
        if let Some(show) = settings.get("borders") {
            let show_norm = show.trim().to_lowercase();
            theme.show_borders = show_norm != "0" && show_norm != "false" && show_norm != "off";
        }
        if let Some(label) = settings.get("source_label") {
            theme.source_label = label.clone();
        }
        if let Some(label) = settings.get("preview_label") {
            theme.preview_label = label.clone();
        }
        if let Some(label) = settings.get("diag_label") {
            theme.diag_label = label.clone();
        }
        theme
    }
}
