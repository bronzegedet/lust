use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::Value;

use super::{json_to_lust_value, lust_to_json_value, Vm};

impl Vm {
    pub(super) fn call_data_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "__range" | "__range_inclusive" => {
                if args.len() != 2 {
                    return Some(Err(format!("{} expects 2 args, got {}", name, args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::Number(start), Value::Number(end)) => {
                        let start = *start as i64;
                        let end = *end as i64;
                        let mut values = Vec::new();
                        let inclusive = name == "__range_inclusive";
                        if start <= end {
                            let upper = if inclusive { end + 1 } else { end };
                            for n in start..upper {
                                values.push(Value::Number(n as f64));
                            }
                        } else {
                            let lower = if inclusive { end } else { end + 1 };
                            for n in (lower..=start).rev() {
                                values.push(Value::Number(n as f64));
                            }
                        }
                        Some(Ok(Value::List(Rc::new(RefCell::new(values)))))
                    }
                    _ => Some(Err(format!("{} expects numeric start and end", name))),
                }
            }
            "__map_keys" => {
                if args.len() != 1 {
                    return Some(Err(format!("keys expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::Map(items) => {
                        let mut keys = items.borrow().keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        let values = keys.into_iter().map(Value::String).collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(values)))))
                    }
                    _ => Some(Err("keys() is only supported on maps".to_string())),
                }
            }
            "__map_values" => {
                if args.len() != 1 {
                    return Some(Err(format!("values expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::Map(items) => {
                        let items = items.borrow();
                        let mut keys = items.keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        let values = keys
                            .into_iter()
                            .map(|key| items.get(&key).cloned().unwrap_or(Value::Null))
                            .collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(values)))))
                    }
                    _ => Some(Err("values() is only supported on maps".to_string())),
                }
            }
            "__map_entries" => {
                if args.len() != 1 {
                    return Some(Err(format!("entries expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::Map(items) => {
                        let items = items.borrow();
                        let mut keys = items.keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        let entries = keys
                            .into_iter()
                            .map(|key| {
                                Value::List(Rc::new(RefCell::new(vec![
                                    Value::String(key.clone()),
                                    items.get(&key).cloned().unwrap_or(Value::Null),
                                ])))
                            })
                            .collect::<Vec<_>>();
                        Some(Ok(Value::List(Rc::new(RefCell::new(entries)))))
                    }
                    _ => Some(Err("entries() is only supported on maps".to_string())),
                }
            }
            "__map_has" => {
                if args.len() != 2 {
                    return Some(Err(format!("has expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::Map(items), Value::String(key)) => {
                        Some(Ok(Value::Bool(items.borrow().contains_key(key))))
                    }
                    _ => Some(Err("has() expects a map receiver and string key".to_string())),
                }
            }
            "dict" => {
                if !args.len().is_multiple_of(2) {
                    return Some(Err(format!(
                        "dict expects an even number of args, got {}",
                        args.len()
                    )));
                }
                let mut items = HashMap::new();
                let mut idx = 0usize;
                while idx < args.len() {
                    let key = match &args[idx] {
                        Value::String(key) => key.clone(),
                        _ => return Some(Err("dict expects string keys".to_string())),
                    };
                    items.insert(key, args[idx + 1].clone());
                    idx += 2;
                }
                Some(Ok(Value::Map(Rc::new(RefCell::new(items)))))
            }
            "json_encode" => {
                if args.len() != 1 {
                    return Some(Err(format!("json_encode expects 1 arg, got {}", args.len())));
                }
                let encoded = match lust_to_json_value(&args[0]) {
                    Ok(value) => value,
                    Err(err) => return Some(Err(err)),
                };
                let encoded = match serde_json::to_string(&encoded) {
                    Ok(value) => value,
                    Err(e) => return Some(Err(format!("json_encode failed: {}", e))),
                };
                Some(Ok(Value::String(encoded)))
            }
            "json_decode" => {
                if args.len() != 1 {
                    return Some(Err(format!("json_decode expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(text) => {
                        let decoded: serde_json::Value = match serde_json::from_str(text) {
                            Ok(value) => value,
                            Err(e) => return Some(Err(format!("json_decode failed: {}", e))),
                        };
                        let pretty = match serde_json::to_string_pretty(&decoded) {
                            Ok(value) => value,
                            Err(e) => {
                                return Some(Err(format!(
                                    "json_decode formatting failed: {}",
                                    e
                                )));
                            }
                        };
                        Some(Ok(Value::String(pretty)))
                    }
                    _ => Some(Err("json_decode expects a string argument".to_string())),
                }
            }
            "json_parse" => {
                if args.len() != 1 {
                    return Some(Err(format!("json_parse expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(text) => {
                        let decoded: serde_json::Value = match serde_json::from_str(text) {
                            Ok(value) => value,
                            Err(e) => return Some(Err(format!("json_parse failed: {}", e))),
                        };
                        Some(Ok(json_to_lust_value(decoded)))
                    }
                    _ => Some(Err("json_parse expects a string argument".to_string())),
                }
            }
            _ => None,
        }
    }
}
