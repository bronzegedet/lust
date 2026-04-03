use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{poll, read, Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

use crate::bytecode::Value;

use super::Vm;

impl Vm {
    pub(super) fn call_core_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "println" => {
                let line = args
                    .iter()
                    .map(Value::as_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("{}", line);
                self.output.push(line);
                Some(Ok(Value::Null))
            }
            "to_string" => {
                if args.len() != 1 {
                    return Some(Err(format!("to_string expects 1 arg, got {}", args.len())));
                }
                Some(Ok(Value::String(args[0].as_string())))
            }
            "to_number" => {
                if args.len() != 1 {
                    return Some(Err(format!("to_number expects 1 arg, got {}", args.len())));
                }
                Some(Ok(Value::Number(
                    args[0].as_string().parse::<f64>().unwrap_or(0.0),
                )))
            }
            "type_of" => {
                if args.len() != 1 {
                    return Some(Err(format!("type_of expects 1 arg, got {}", args.len())));
                }
                Some(Ok(Value::String(args[0].type_name())))
            }
            "debug" => {
                if args.len() != 2 {
                    return Some(Err(format!("debug expects 2 args, got {}", args.len())));
                }
                let line = format!("DEBUG {} {}", args[0], args[1]);
                println!("{}", line);
                self.output.push(line);
                Some(Ok(args[1].clone()))
            }
            "panic" => {
                if args.len() != 1 {
                    return Some(Err(format!("panic expects 1 arg, got {}", args.len())));
                }
                Some(Err(format!("lust panic: {}", args[0])))
            }
            "assert" => {
                if args.len() != 2 {
                    return Some(Err(format!("assert expects 2 args, got {}", args.len())));
                }
                if args[0].truthy() {
                    Some(Ok(Value::Null))
                } else {
                    Some(Err(format!("lust assert failed: {}", args[1])))
                }
            }
            "input" => {
                let raw = if !self.input_lines.is_empty() {
                    self.input_lines.remove(0)
                } else {
                    use std::io::{self, Write};
                    let mut s = String::new();
                    let _ = io::stdout().flush();
                    if let Err(e) = io::stdin().read_line(&mut s) {
                        return Some(Err(format!("input failed: {}", e)));
                    }
                    s
                };
                Some(Ok(Value::String(raw.trim().to_string())))
            }
            "clr" => {
                if !args.is_empty() {
                    return Some(Err(format!("clr expects 0 args, got {}", args.len())));
                }
                use std::io::{self, Write};
                print!("\x1B[2J\x1B[H");
                if let Err(e) = io::stdout().flush() {
                    return Some(Err(format!("clr failed: {}", e)));
                }
                Some(Ok(Value::Null))
            }
            "prompt" => {
                if args.len() != 1 {
                    return Some(Err(format!("prompt expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(message) => {
                        use std::io::{self, Write};
                        print!("{}", message);
                        if let Err(e) = io::stdout().flush() {
                            return Some(Err(format!("prompt failed: {}", e)));
                        }
                        let raw = if !self.input_lines.is_empty() {
                            self.input_lines.remove(0)
                        } else {
                            let mut s = String::new();
                            if let Err(e) = io::stdin().read_line(&mut s) {
                                return Some(Err(format!("prompt failed: {}", e)));
                            }
                            s
                        };
                        Some(Ok(Value::String(raw.trim().to_string())))
                    }
                    _ => Some(Err("prompt expects a string message".to_string())),
                }
            }
            "get_env" => {
                if args.len() != 1 {
                    return Some(Err(format!("get_env expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(key) => Some(Ok(Value::String(std::env::var(key).unwrap_or_default()))),
                    _ => Some(Err("get_env expects a string key".to_string())),
                }
            }
            "get_key" => {
                if let Some(token) = self.consume_queued_key() {
                    return Some(Ok(Self::key_value(token.variant.as_str(), token.payload)));
                }

                let had_raw_mode = is_raw_mode_enabled().unwrap_or(false);
                if !had_raw_mode {
                    if let Err(e) = enable_raw_mode() {
                        return Some(Err(format!("failed to enable raw mode: {}", e)));
                    }
                }
                let key = loop {
                    match read() {
                        Ok(Event::Key(event)) if event.kind == KeyEventKind::Press => {
                            if let Some(token) = Self::decode_key_event(event.code) {
                                break token;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => return Some(Err(format!("failed to read event: {}", e))),
                    }
                };
                if !had_raw_mode {
                    if let Err(e) = disable_raw_mode() {
                        return Some(Err(format!("failed to disable raw mode: {}", e)));
                    }
                }
                Some(Ok(Self::key_value(key.variant.as_str(), key.payload)))
            }
            "poll_key" => {
                if !args.is_empty() {
                    return Some(Err(format!("poll_key expects 0 args, got {}", args.len())));
                }
                if let Some(token) = self.consume_queued_key() {
                    return Some(Ok(Self::key_value(token.variant.as_str(), token.payload)));
                }

                let had_raw_mode = is_raw_mode_enabled().unwrap_or(false);
                if !had_raw_mode {
                    if let Err(e) = enable_raw_mode() {
                        return Some(Err(format!("failed to enable raw mode: {}", e)));
                    }
                }
                let polled = match poll(std::time::Duration::from_millis(0)) {
                    Ok(value) => value,
                    Err(e) => return Some(Err(format!("failed to poll key input: {}", e))),
                };
                let key = if polled {
                    match read() {
                        Ok(Event::Key(event)) if event.kind == KeyEventKind::Press => {
                            Self::decode_key_event(event.code)
                        }
                        Ok(_) => None,
                        Err(e) => return Some(Err(format!("failed to read key input: {}", e))),
                    }
                } else {
                    None
                };
                if !had_raw_mode {
                    if let Err(e) = disable_raw_mode() {
                        return Some(Err(format!("failed to disable raw mode: {}", e)));
                    }
                }

                if let Some(token) = key {
                    Some(Ok(Self::key_value(token.variant.as_str(), token.payload)))
                } else {
                    Some(Ok(Self::key_value("None", Vec::new())))
                }
            }
            "now" => {
                if !args.is_empty() {
                    return Some(Err(format!("now expects 0 args, got {}", args.len())));
                }
                let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(value) => value.as_secs_f64(),
                    Err(e) => return Some(Err(e.to_string())),
                };
                Some(Ok(Value::Number(seconds)))
            }
            "random_int" => {
                if args.len() != 2 {
                    return Some(Err(format!("random_int expects 2 args, got {}", args.len())));
                }
                let min = args[0].as_number();
                let max = args[1].as_number();
                let start = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let range = (max - min).abs();
                if range == 0.0 {
                    return Some(Ok(Value::Number(min)));
                }
                let rand = (start % 1_000_000) as f64 / 1_000_000.0 * range;
                Some(Ok(Value::Number((min + rand).floor())))
            }
            "random_float" => {
                if args.len() != 2 {
                    return Some(Err(format!("random_float expects 2 args, got {}", args.len())));
                }
                let min = args[0].as_number();
                let max = args[1].as_number();
                let start = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let range = max - min;
                if range <= 0.0 {
                    return Some(Ok(Value::Number(min)));
                }
                let frac = (start % 1_000_000) as f64 / 1_000_000.0;
                Some(Ok(Value::Number(min + frac * range)))
            }
            "sleep" => {
                if args.len() != 1 {
                    return Some(Err(format!("sleep expects 1 arg, got {}", args.len())));
                }
                match args[0] {
                    Value::Number(seconds) => {
                        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
                        Some(Ok(Value::Null))
                    }
                    _ => Some(Err("sleep expects a number argument".to_string())),
                }
            }
            _ => None,
        }
    }
}
