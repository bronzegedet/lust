use crate::bytecode::Value;

use super::Vm;

impl Vm {
    pub(super) fn call_ui_builtin(&mut self, name: &str, args: &[Value]) -> Option<Result<Value, String>> {
        match name {
            "ui_knob" | "ui_slider" => {
                if args.len() != 4 {
                    return Some(Err(format!("{} expects 4 args, got {}", name, args.len())));
                }
                let id = args[0].as_string();
                let min = args[1].as_number();
                let max = args[2].as_number();
                let default_value = args[3].as_number();
                let value = self.ui_number_control(&id, min, max, default_value);
                let result = Value::Number(value);
                self.trace_ui_call(name, Some(&id), &result);
                Some(Ok(result))
            }
            "ui_toggle" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_toggle expects 2 args, got {}", args.len())));
                }
                let id = args[0].as_string();
                let default_value = args[1].truthy();
                let value = self.ui_bool_control(&id, default_value);
                let result = Value::Bool(value);
                self.trace_ui_call("ui_toggle", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_textbox" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_textbox expects 2 args, got {}", args.len())));
                }
                let id = args[0].as_string();
                let default_value = args[1].as_string();
                let value = self.ui_string_control(&id, &default_value);
                let result = Value::String(value);
                self.trace_ui_call("ui_textbox", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_button" => {
                if args.len() != 1 {
                    return Some(Err(format!("ui_button expects 1 arg, got {}", args.len())));
                }
                let id = args[0].as_string();
                let pressed = self.ui_button_latches.remove(&id).unwrap_or(false);
                if !self.ui_state.contains_key(&id) {
                    if let Err(err) = self.ensure_ui_state_len(self.ui_state.len().saturating_add(1)) {
                        return Some(Err(err));
                    }
                }
                self.ui_state.insert(id.clone(), Value::Bool(false));
                let result = Value::Bool(pressed);
                self.trace_ui_call("ui_button", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_theme" => {
                if args.len() != 1 {
                    return Some(Err(format!("ui_theme expects 1 arg, got {}", args.len())));
                }
                let theme_name = args[0].as_string();
                let result = Value::String(theme_name.clone());
                if !self.ui_state.contains_key("__theme") {
                    if let Err(err) = self.ensure_ui_state_len(self.ui_state.len().saturating_add(1)) {
                        return Some(Err(err));
                    }
                }
                self.ui_state
                    .insert("__theme".to_string(), Value::String(theme_name.clone()));
                self.trace_ui_call("ui_theme", Some(&theme_name), &result);
                Some(Ok(result))
            }
            "ui_set" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_set expects 2 args, got {}", args.len())));
                }
                let id = args[0].as_string();
                let value = args[1].clone();
                if !self.ui_state.contains_key(&id) {
                    if let Err(err) = self.ensure_ui_state_len(self.ui_state.len().saturating_add(1)) {
                        return Some(Err(err));
                    }
                }
                if let Value::Bool(flag) = value {
                    self.ui_button_latches.insert(id.clone(), flag);
                } else if id.starts_with("button.") {
                    self.ui_button_latches.insert(id.clone(), value.truthy());
                }
                self.ui_state.insert(id.clone(), value.clone());
                self.trace_ui_call("ui_set", Some(&id), &value);
                Some(Ok(value))
            }
            "ui_get" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_get expects 2 args, got {}", args.len())));
                }
                let id = args[0].as_string();
                let default_value = args[1].clone();
                if !self.ui_state.contains_key(&id) {
                    if let Err(err) = self.ensure_ui_state_len(self.ui_state.len().saturating_add(1)) {
                        return Some(Err(err));
                    }
                }
                let value = self
                    .ui_state
                    .entry(id.clone())
                    .or_insert_with(|| default_value.clone())
                    .clone();
                self.trace_ui_call("ui_get", Some(&id), &value);
                Some(Ok(value))
            }
            "ui_caret" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_caret expects 2 args, got {}", args.len())));
                }
                let id = format!("caret.{}", args[0].as_string());
                let default_value = args[1].as_number().max(0.0).floor();
                let value = self.ui_index_control(&id, default_value);
                let result = Value::Number(value);
                self.trace_ui_call("ui_caret", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_selection_start" => {
                if args.len() != 2 {
                    return Some(Err(format!(
                        "ui_selection_start expects 2 args, got {}",
                        args.len()
                    )));
                }
                let id = format!("selection.start.{}", args[0].as_string());
                let default_value = args[1].as_number().max(0.0).floor();
                let value = self.ui_index_control(&id, default_value);
                let result = Value::Number(value);
                self.trace_ui_call("ui_selection_start", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_selection_end" => {
                if args.len() != 2 {
                    return Some(Err(format!(
                        "ui_selection_end expects 2 args, got {}",
                        args.len()
                    )));
                }
                let id = format!("selection.end.{}", args[0].as_string());
                let default_value = args[1].as_number().max(0.0).floor();
                let value = self.ui_index_control(&id, default_value);
                let result = Value::Number(value);
                self.trace_ui_call("ui_selection_end", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_scroll_y" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_scroll_y expects 2 args, got {}", args.len())));
                }
                let id = format!("scroll.y.{}", args[0].as_string());
                let default_value = args[1].as_number().max(0.0).floor();
                let value = self.ui_index_control(&id, default_value);
                let result = Value::Number(value);
                self.trace_ui_call("ui_scroll_y", Some(&id), &result);
                Some(Ok(result))
            }
            "ui_text_input" => {
                if !args.is_empty() {
                    return Some(Err(format!(
                        "ui_text_input expects 0 args, got {}",
                        args.len()
                    )));
                }
                let text = if let Some(token) = self.consume_queued_key() {
                    if token.variant == "Char" {
                        token.payload.first().cloned().unwrap_or_default()
                    } else if token.variant == "Enter" {
                        "\n".to_string()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let result = Value::String(text);
                self.trace_ui_call("ui_text_input", None, &result);
                Some(Ok(result))
            }
            "ui_key_left" => Some(Ok(self.ui_key_pressed("Left", "ui_key_left"))),
            "ui_key_right" => Some(Ok(self.ui_key_pressed("Right", "ui_key_right"))),
            "ui_key_up" => Some(Ok(self.ui_key_pressed("Up", "ui_key_up"))),
            "ui_key_down" => Some(Ok(self.ui_key_pressed("Down", "ui_key_down"))),
            "ui_key_enter" => Some(Ok(self.ui_key_pressed("Enter", "ui_key_enter"))),
            "ui_key_esc" => Some(Ok(self.ui_key_pressed("Esc", "ui_key_esc"))),
            "ui_key_backspace" => Some(Ok(self.ui_key_pressed("Backspace", "ui_key_backspace"))),
            "ui_key_delete" => Some(Ok(self.ui_key_pressed("Delete", "ui_key_delete"))),
            "ui_mouse_x" => {
                if !args.is_empty() {
                    return Some(Err(format!("ui_mouse_x expects 0 args, got {}", args.len())));
                }
                self.poll_pointer_events();
                let result = Value::Number(self.ui_mouse_x);
                self.trace_ui_call("ui_mouse_x", None, &result);
                Some(Ok(result))
            }
            "ui_mouse_y" => {
                if !args.is_empty() {
                    return Some(Err(format!("ui_mouse_y expects 0 args, got {}", args.len())));
                }
                self.poll_pointer_events();
                let result = Value::Number(self.ui_mouse_y);
                self.trace_ui_call("ui_mouse_y", None, &result);
                Some(Ok(result))
            }
            "ui_mouse_down" => {
                if !args.is_empty() {
                    return Some(Err(format!(
                        "ui_mouse_down expects 0 args, got {}",
                        args.len()
                    )));
                }
                self.poll_pointer_events();
                let result = Value::Bool(self.ui_mouse_down);
                self.trace_ui_call("ui_mouse_down", None, &result);
                Some(Ok(result))
            }
            "ui_mouse_clicked" => {
                if !args.is_empty() {
                    return Some(Err(format!(
                        "ui_mouse_clicked expects 0 args, got {}",
                        args.len()
                    )));
                }
                self.poll_pointer_events();
                let clicked = self.ui_mouse_clicked;
                self.ui_mouse_clicked = false;
                let result = Value::Bool(clicked);
                self.trace_ui_call("ui_mouse_clicked", None, &result);
                Some(Ok(result))
            }
            "ui_mouse_click_x" => {
                if !args.is_empty() {
                    return Some(Err(format!(
                        "ui_mouse_click_x expects 0 args, got {}",
                        args.len()
                    )));
                }
                self.poll_pointer_events();
                let result = Value::Number(self.ui_mouse_click_x);
                self.trace_ui_call("ui_mouse_click_x", None, &result);
                Some(Ok(result))
            }
            "ui_mouse_click_y" => {
                if !args.is_empty() {
                    return Some(Err(format!(
                        "ui_mouse_click_y expects 0 args, got {}",
                        args.len()
                    )));
                }
                self.poll_pointer_events();
                let result = Value::Number(self.ui_mouse_click_y);
                self.trace_ui_call("ui_mouse_click_y", None, &result);
                Some(Ok(result))
            }
            "ui_command" => {
                if args.len() != 2 {
                    return Some(Err(format!("ui_command expects 2 args, got {}", args.len())));
                }
                let id = format!("command.{}", args[0].as_string());
                let default_value = args[1].as_string();
                if !self.ui_state.contains_key(&id) {
                    if let Err(err) = self.ensure_ui_state_len(self.ui_state.len().saturating_add(1)) {
                        return Some(Err(err));
                    }
                    self.ui_state
                        .insert(id.clone(), Value::String(default_value.clone()));
                }
                let value = self
                    .ui_state
                    .get(&id)
                    .map(Value::as_string)
                    .unwrap_or(default_value);
                self.ui_state.insert(id.clone(), Value::String(String::new()));
                let result = Value::String(value);
                self.trace_ui_call("ui_command", Some(&id), &result);
                Some(Ok(result))
            }
            _ => None,
        }
    }

    fn trace_ui_call(&mut self, name: &str, id: Option<&str>, result: &Value) {
        if !self.trace_enabled {
            return;
        }
        let id_text = id.unwrap_or("-");
        self.trace_events.push(format!(
            "builtin={} id={} result={}",
            name,
            id_text,
            result.as_string()
        ));
        if self.trace_events.len() > self.memory_budget.max_trace_events {
            let drop_count = self
                .trace_events
                .len()
                .saturating_sub(self.memory_budget.max_trace_events);
            self.trace_events.drain(0..drop_count);
        }
    }

    fn ui_number_control(&mut self, id: &str, min: f64, max: f64, default_value: f64) -> f64 {
        let low = min.min(max);
        let high = min.max(max);
        let default_clamped = default_value.clamp(low, high);
        let entry = self
            .ui_state
            .entry(id.to_string())
            .or_insert_with(|| Value::Number(default_clamped));
        let raw = match entry {
            Value::Number(value) => *value,
            _ => default_clamped,
        };
        let clamped = raw.clamp(low, high);
        *entry = Value::Number(clamped);
        clamped
    }

    fn ui_bool_control(&mut self, id: &str, default_value: bool) -> bool {
        let entry = self
            .ui_state
            .entry(id.to_string())
            .or_insert_with(|| Value::Bool(default_value));
        let value = match &*entry {
            Value::Bool(flag) => *flag,
            other => other.truthy(),
        };
        *entry = Value::Bool(value);
        value
    }

    fn ui_string_control(&mut self, id: &str, default_value: &str) -> String {
        let entry = self
            .ui_state
            .entry(id.to_string())
            .or_insert_with(|| Value::String(default_value.to_string()));
        let value = match &*entry {
            Value::String(text) => text.clone(),
            other => other.as_string(),
        };
        *entry = Value::String(value.clone());
        value
    }

    fn ui_index_control(&mut self, id: &str, default_value: f64) -> f64 {
        let safe_default = default_value.max(0.0).floor();
        let entry = self
            .ui_state
            .entry(id.to_string())
            .or_insert_with(|| Value::Number(safe_default));
        let raw = match &*entry {
            Value::Number(value) => *value,
            Value::Bool(flag) => {
                if *flag {
                    1.0
                } else {
                    0.0
                }
            }
            Value::String(text) => text.parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        };
        let clamped = raw.max(0.0).floor();
        *entry = Value::Number(clamped);
        clamped
    }

    fn ui_key_pressed(&mut self, expected_variant: &str, builtin_name: &str) -> Value {
        let token = if let Some(buffered) = self.ui_key_buffer.take() {
            Some(buffered)
        } else {
            self.consume_queued_key()
        };
        let pressed = if let Some(token) = token {
            if token.variant == expected_variant {
                true
            } else {
                self.ui_key_buffer = Some(token);
                false
            }
        } else {
            false
        };
        let result = Value::Bool(pressed);
        self.trace_ui_call(builtin_name, Some(expected_variant), &result);
        result
    }
}
