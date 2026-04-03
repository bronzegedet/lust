use std::collections::VecDeque;
use std::io::{stdout, IsTerminal, Write};
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{ExecutableCommand, QueueableCommand};

#[derive(Clone, Debug)]
pub struct KeyToken {
    pub variant: String,
    pub payload: Vec<String>,
}

pub struct DrawRuntime {
    logical_width: u16,
    logical_height: u16,
    _title: String,
    headless: bool,
    frame_limit: u64,
    presented_frames: u64,
    started: bool,
    buffer_width: usize,
    buffer_height: usize,
    buffer: Vec<char>,
    terminal_active: bool,
    pending_keys: VecDeque<KeyToken>,
}

impl DrawRuntime {
    pub fn new(width: u16, height: u16, title: String) -> Result<Self, String> {
        let headless = env_truthy("LUST_DRAW_HEADLESS") || !stdout().is_terminal();
        let frame_limit = std::env::var("LUST_DRAW_FRAME_LIMIT")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(if headless { 1 } else { 600 });

        let (buffer_width, buffer_height) = if headless {
            let w = std::env::var("LUST_DRAW_HEADLESS_WIDTH")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(120);
            let h = std::env::var("LUST_DRAW_HEADLESS_HEIGHT")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(40);
            (w.max(16), h.max(8))
        } else {
            let w = std::env::var("LUST_DRAW_TERM_WIDTH")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(120);
            let h = std::env::var("LUST_DRAW_TERM_HEIGHT")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(40);
            (w.max(16), h.max(8))
        };

        let mut runtime = Self {
            logical_width: width.max(1),
            logical_height: height.max(1),
            _title: title,
            headless,
            frame_limit,
            presented_frames: 0,
            started: false,
            buffer_width,
            buffer_height,
            buffer: vec![' '; buffer_width * buffer_height],
            terminal_active: false,
            pending_keys: VecDeque::new(),
        };

        if !runtime.headless {
            let mut out = stdout();
            out.execute(EnterAlternateScreen)
                .map_err(|e| format!("draw: failed to enter alternate screen: {}", e))?;
            out.execute(Hide)
                .map_err(|e| format!("draw: failed to hide cursor: {}", e))?;
            out.execute(EnableMouseCapture)
                .map_err(|e| format!("draw: failed to enable mouse capture: {}", e))?;
            enable_raw_mode().map_err(|e| format!("draw: failed to enable raw mode: {}", e))?;
            runtime.terminal_active = true;
        }

        Ok(runtime)
    }

    pub fn live(&mut self) -> Result<bool, String> {
        if self.started {
            self.presented_frames += 1;
            if !self.headless {
                self.flush_frame()?;
            }
            if self.frame_limit > 0 && self.presented_frames >= self.frame_limit {
                return Ok(false);
            }
        }
        self.started = true;
        if !self.headless && self.terminal_active {
            self.capture_pending_keys()?;
        }
        Ok(true)
    }

    pub fn clear_screen(&mut self, color: &str) {
        let fill = color_to_glyph(color);
        self.buffer.fill(fill);
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: &str) {
        let glyph = color_to_glyph(color);
        let start_x = x.floor() as i32;
        let start_y = y.floor() as i32;
        let end_x = (x + w).ceil() as i32;
        let end_y = (y + h).ceil() as i32;

        for yy in start_y..end_y {
            for xx in start_x..end_x {
                self.plot_logical(xx, yy, glyph);
            }
        }
    }

    pub fn circle(&mut self, cx: f64, cy: f64, radius: f64, color: &str) {
        let glyph = color_to_glyph(color);
        let r = radius.max(0.0).round() as i32;
        let center_x = cx.round() as i32;
        let center_y = cy.round() as i32;
        let rr = r * r;

        for yy in -r..=r {
            for xx in -r..=r {
                if (xx * xx + yy * yy) <= rr {
                    self.plot_logical(center_x + xx, center_y + yy, glyph);
                }
            }
        }
    }

    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: &str) {
        let glyph = color_to_glyph(color);
        let mut x = x1.round() as i32;
        let mut y = y1.round() as i32;
        let end_x = x2.round() as i32;
        let end_y = y2.round() as i32;

        let dx = (end_x - x).abs();
        let sx = if x < end_x { 1 } else { -1 };
        let dy = -(end_y - y).abs();
        let sy = if y < end_y { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.plot_logical(x, y, glyph);
            if x == end_x && y == end_y {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    pub fn triangle(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64, color: &str) {
        self.line(x1, y1, x2, y2, color);
        self.line(x2, y2, x3, y3, color);
        self.line(x3, y3, x1, y1, color);
    }

    pub fn text(&mut self, text: &str, x: f64, y: f64, _size: f64, _color: &str) {
        let base_x = x.round() as i32;
        let base_y = y.round() as i32;
        for (idx, ch) in text.chars().enumerate() {
            self.plot_logical(base_x + idx as i32, base_y, ch);
        }
    }

    pub fn take_pending_key(&mut self) -> Result<Option<KeyToken>, String> {
        if !self.headless && self.terminal_active {
            self.capture_pending_keys()?;
        }
        Ok(self.pending_keys.pop_front())
    }

    fn capture_pending_keys(&mut self) -> Result<(), String> {
        loop {
            let has_event = poll(Duration::from_millis(0))
                .map_err(|e| format!("draw: failed to poll key events: {}", e))?;
            if !has_event {
                break;
            }

            match read().map_err(|e| format!("draw: failed to read input event: {}", e))? {
                Event::Key(event) => {
                    if event.kind == KeyEventKind::Press {
                        if let Some(token) = key_event_to_token(event.code) {
                            self.pending_keys.push_back(token);
                        }
                    }
                }
                Event::Mouse(event) => {
                    if let Some(token) = self.mouse_event_to_token(event) {
                        self.pending_keys.push_back(token);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn logical_to_buffer(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        if x < 0 || y < 0 {
            return None;
        }
        let lx = x as f64 / self.logical_width as f64;
        let ly = y as f64 / self.logical_height as f64;
        let bx = (lx * self.buffer_width as f64).floor() as isize;
        let by = (ly * self.buffer_height as f64).floor() as isize;
        if bx < 0 || by < 0 || bx >= self.buffer_width as isize || by >= self.buffer_height as isize {
            return None;
        }
        Some((bx as usize, by as usize))
    }

    fn plot_logical(&mut self, x: i32, y: i32, glyph: char) {
        if let Some((bx, by)) = self.logical_to_buffer(x, y) {
            let idx = by * self.buffer_width + bx;
            if idx < self.buffer.len() {
                self.buffer[idx] = glyph;
            }
        }
    }

    fn flush_frame(&mut self) -> Result<(), String> {
        let mut out = stdout();
        out.queue(MoveTo(0, 0))
            .map_err(|e| format!("draw: failed to move cursor: {}", e))?;
        out.queue(Clear(ClearType::All))
            .map_err(|e| format!("draw: failed to clear frame: {}", e))?;

        for y in 0..self.buffer_height {
            let row_start = y * self.buffer_width;
            let row_end = row_start + self.buffer_width;
            let row = self.buffer[row_start..row_end]
                .iter()
                .collect::<String>();
            out.queue(Print(row))
                .map_err(|e| format!("draw: failed to print row: {}", e))?;
            out.queue(Print("\r\n"))
                .map_err(|e| format!("draw: failed to print newline: {}", e))?;
        }

        out.flush()
            .map_err(|e| format!("draw: failed to flush frame: {}", e))?;
        Ok(())
    }

    fn mouse_event_to_token(&self, event: MouseEvent) -> Option<KeyToken> {
        let (x, y) = self.buffer_to_logical(event.column, event.row);
        let payload_xy = vec![to_coord_token(x), to_coord_token(y)];
        match event.kind {
            MouseEventKind::Moved => Some(KeyToken {
                variant: "MouseMove".to_string(),
                payload: payload_xy,
            }),
            MouseEventKind::Down(button) => {
                let mut payload = payload_xy;
                payload.push(mouse_button_name(button).to_string());
                Some(KeyToken {
                    variant: "MouseDown".to_string(),
                    payload,
                })
            }
            MouseEventKind::Up(button) => {
                let mut payload = payload_xy;
                payload.push(mouse_button_name(button).to_string());
                Some(KeyToken {
                    variant: "MouseUp".to_string(),
                    payload,
                })
            }
            MouseEventKind::Drag(button) => {
                let mut payload = payload_xy;
                payload.push(mouse_button_name(button).to_string());
                Some(KeyToken {
                    variant: "MouseDrag".to_string(),
                    payload,
                })
            }
            MouseEventKind::ScrollUp => Some(KeyToken {
                variant: "MouseScrollUp".to_string(),
                payload: payload_xy,
            }),
            MouseEventKind::ScrollDown => Some(KeyToken {
                variant: "MouseScrollDown".to_string(),
                payload: payload_xy,
            }),
            MouseEventKind::ScrollLeft => Some(KeyToken {
                variant: "MouseScrollLeft".to_string(),
                payload: payload_xy,
            }),
            MouseEventKind::ScrollRight => Some(KeyToken {
                variant: "MouseScrollRight".to_string(),
                payload: payload_xy,
            }),
        }
    }

    fn buffer_to_logical(&self, column: u16, row: u16) -> (f64, f64) {
        let max_bx = self.buffer_width.saturating_sub(1) as f64;
        let max_by = self.buffer_height.saturating_sub(1) as f64;
        let bx = (column as f64).clamp(0.0, max_bx);
        let by = (row as f64).clamp(0.0, max_by);
        let x = if max_bx <= 0.0 {
            0.0
        } else {
            (bx / max_bx) * (self.logical_width.saturating_sub(1) as f64)
        };
        let y = if max_by <= 0.0 {
            0.0
        } else {
            (by / max_by) * (self.logical_height.saturating_sub(1) as f64)
        };
        (x.floor(), y.floor())
    }
}

impl Drop for DrawRuntime {
    fn drop(&mut self) {
        if self.terminal_active {
            let _ = disable_raw_mode();
            let mut out = stdout();
            let _ = out.execute(Show);
            let _ = out.execute(DisableMouseCapture);
            let _ = out.execute(LeaveAlternateScreen);
            let _ = out.flush();
            self.terminal_active = false;
        }
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

fn color_to_glyph(color: &str) -> char {
    match color {
        "black" => ' ',
        "white" => '.',
        "red" => 'R',
        "green" => 'G',
        "blue" => 'B',
        "dark_gray" => ':',
        "neon_pink" => 'P',
        _ => '#',
    }
}

fn key_event_to_token(code: KeyCode) -> Option<KeyToken> {
    let token = match code {
        KeyCode::Up => KeyToken {
            variant: "Up".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Down => KeyToken {
            variant: "Down".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Left => KeyToken {
            variant: "Left".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Right => KeyToken {
            variant: "Right".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Enter => KeyToken {
            variant: "Enter".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Backspace => KeyToken {
            variant: "Backspace".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Delete => KeyToken {
            variant: "Delete".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Esc => KeyToken {
            variant: "Esc".to_string(),
            payload: Vec::new(),
        },
        KeyCode::Char(c) => KeyToken {
            variant: "Char".to_string(),
            payload: vec![c.to_string()],
        },
        _ => return None,
    };
    Some(token)
}

fn mouse_button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "Left",
        MouseButton::Right => "Right",
        MouseButton::Middle => "Middle",
    }
}

fn to_coord_token(value: f64) -> String {
    format!("{}", value.floor())
}
