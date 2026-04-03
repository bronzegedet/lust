use std::cell::RefCell;
use std::rc::Rc;

use crate::bytecode::Value;

use super::Vm;

impl Vm {
    pub(super) fn call_text_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "__str_trim" => {
                if args.len() != 1 {
                    return Some(Err(format!("trim expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(s) => Some(Ok(Value::String(s.trim().to_string()))),
                    _ => Some(Err("trim() is only supported on strings".to_string())),
                }
            }
            "__str_at" => {
                if args.len() != 2 {
                    return Some(Err(format!("at expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::Number(idx)) => {
                        let idx = *idx as usize;
                        if let Some(c) = s.chars().nth(idx) {
                            Some(Ok(Value::String(c.to_string())))
                        } else {
                            Some(Ok(Value::Null))
                        }
                    }
                    _ => Some(Err("at() expects a string target and numeric index".to_string())),
                }
            }
            "__str_slice" => {
                if args.len() != 3 {
                    return Some(Err(format!("slice expects 3 args, got {}", args.len())));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(s), Value::Number(start), Value::Number(end)) => {
                        let start = *start as usize;
                        let end = *end as usize;
                        if start <= end && end <= s.len() {
                            Some(Ok(Value::String(s[start..end].to_string())))
                        } else {
                            Some(Ok(Value::Null))
                        }
                    }
                    _ => Some(Err("slice() expects a string target and numeric bounds".to_string())),
                }
            }
            "__str_contains" => {
                if args.len() != 2 {
                    return Some(Err(format!("contains expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(sub)) => Some(Ok(Value::Bool(s.contains(sub)))),
                    _ => Some(Err(
                        "contains() expects string target and string argument".to_string(),
                    )),
                }
            }
            "__str_starts_with" => {
                if args.len() != 2 {
                    return Some(Err(format!(
                        "starts_with expects 2 args, got {}",
                        args.len()
                    )));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(prefix)) => {
                        Some(Ok(Value::Bool(s.starts_with(prefix))))
                    }
                    _ => Some(Err(
                        "starts_with() expects string target and string argument".to_string(),
                    )),
                }
            }
            "__str_ends_with" => {
                if args.len() != 2 {
                    return Some(Err(format!("ends_with expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(suffix)) => {
                        Some(Ok(Value::Bool(s.ends_with(suffix))))
                    }
                    _ => Some(Err(
                        "ends_with() expects string target and string argument".to_string(),
                    )),
                }
            }
            "__str_split" => {
                if args.len() != 2 {
                    return Some(Err(format!("split expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(sep)) => {
                        let parts = s
                            .split(sep)
                            .map(|part| Value::String(part.to_string()))
                            .collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(parts)))))
                    }
                    _ => Some(Err(
                        "split() expects string target and string separator".to_string(),
                    )),
                }
            }
            "__str_lines" => {
                if args.len() != 1 {
                    return Some(Err(format!("lines expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(s) => {
                        let parts = s
                            .lines()
                            .map(|part| Value::String(part.to_string()))
                            .collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(parts)))))
                    }
                    _ => Some(Err("lines() is only supported on strings".to_string())),
                }
            }
            "__str_to_list" => {
                if args.len() != 1 {
                    return Some(Err(format!("to_list expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(s) => {
                        let parts = s
                            .chars()
                            .map(|c| Value::String(c.to_string()))
                            .collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(parts)))))
                    }
                    _ => Some(Err("to_list() is only supported on strings".to_string())),
                }
            }
            "__str_replace" => {
                if args.len() != 3 {
                    return Some(Err(format!("replace expects 3 args, got {}", args.len())));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(s), Value::String(from), Value::String(to)) => {
                        Some(Ok(Value::String(s.replace(from, to))))
                    }
                    _ => Some(Err(
                        "replace() expects string target and string arguments".to_string(),
                    )),
                }
            }
            "__str_insert" => {
                if args.len() != 3 {
                    return Some(Err(format!("insert expects 3 args, got {}", args.len())));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(base), Value::Number(index), Value::String(part)) => {
                        let index = (*index).max(0.0) as usize;
                        let byte = byte_index_for_char_pos(base, index).unwrap_or(base.len());
                        let mut out = String::with_capacity(base.len() + part.len());
                        out.push_str(&base[..byte]);
                        out.push_str(part);
                        out.push_str(&base[byte..]);
                        Some(Ok(Value::String(out)))
                    }
                    _ => Some(Err("insert() expects string, number index, string part".to_string())),
                }
            }
            "__str_delete_range" => {
                if args.len() != 3 {
                    return Some(Err(format!(
                        "delete_range expects 3 args, got {}",
                        args.len()
                    )));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(base), Value::Number(start), Value::Number(end)) => {
                        let start = (*start).max(0.0) as usize;
                        let end = (*end).max(0.0) as usize;
                        let (low, high) = if start <= end { (start, end) } else { (end, start) };
                        let start_byte = byte_index_for_char_pos(base, low).unwrap_or(base.len());
                        let end_byte = byte_index_for_char_pos(base, high).unwrap_or(base.len());
                        if start_byte > end_byte || end_byte > base.len() {
                            return Some(Ok(Value::String(base.clone())));
                        }
                        let mut out = String::with_capacity(base.len().saturating_sub(end_byte - start_byte));
                        out.push_str(&base[..start_byte]);
                        out.push_str(&base[end_byte..]);
                        Some(Ok(Value::String(out)))
                    }
                    _ => Some(Err(
                        "delete_range() expects string and numeric bounds".to_string(),
                    )),
                }
            }
            "__str_find" => {
                if args.len() != 2 {
                    return Some(Err(format!("find expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(base), Value::String(needle)) => {
                        if needle.is_empty() {
                            return Some(Ok(Value::Number(0.0)));
                        }
                        if let Some(byte_index) = base.find(needle) {
                            let char_index = base[..byte_index].chars().count();
                            Some(Ok(Value::Number(char_index as f64)))
                        } else {
                            Some(Ok(Value::Number(-1.0)))
                        }
                    }
                    _ => Some(Err("find() expects string target and string needle".to_string())),
                }
            }
            _ => None,
        }
    }
}

fn byte_index_for_char_pos(text: &str, char_pos: usize) -> Option<usize> {
    if char_pos == 0 {
        return Some(0);
    }
    let mut count = 0usize;
    for (byte_idx, _) in text.char_indices() {
        if count == char_pos {
            return Some(byte_idx);
        }
        count = count.saturating_add(1);
    }
    if count == char_pos {
        Some(text.len())
    } else {
        None
    }
}
