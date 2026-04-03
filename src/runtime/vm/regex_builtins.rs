use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use regex::Regex;

use crate::bytecode::{RegexCaptureValue, Value};

use super::Vm;

impl Vm {
    pub(super) fn call_regex_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "compile_lustgex" => {
                if args.len() != 1 {
                    return Some(Err(format!(
                        "compile_lustgex expects 1 arg, got {}",
                        args.len()
                    )));
                }
                match &args[0] {
                    Value::String(pattern) => {
                        let cached = match self.get_or_compile_lustgex_pattern(pattern) {
                            Ok(cached) => cached,
                            Err(err) => return Some(Err(err)),
                        };
                        Some(Ok(Value::String(cached.compiled.clone())))
                    }
                    _ => Some(Err("compile_lustgex expects a string pattern".to_string())),
                }
            }
            "lustgex_match" => {
                if args.len() != 2 {
                    return Some(Err(format!(
                        "lustgex_match expects 2 args, got {}",
                        args.len()
                    )));
                }
                match (&args[0], &args[1]) {
                    (Value::String(text), Value::String(pattern)) => {
                        let cached = match self.get_or_compile_lustgex_pattern(pattern) {
                            Ok(cached) => cached,
                            Err(err) => return Some(Err(err)),
                        };
                        Some(Ok(Value::Bool(cached.regex.is_match(text))))
                    }
                    _ => Some(Err(
                        "lustgex_match expects string text and string pattern".to_string(),
                    )),
                }
            }
            "lustgex_capture_builtin" => {
                if args.len() != 2 {
                    return Some(Err(format!(
                        "lustgex_capture_builtin expects 2 args, got {}",
                        args.len()
                    )));
                }
                match (&args[0], &args[1]) {
                    (Value::String(text), Value::String(pattern)) => {
                        let cached = match self.get_or_compile_lustgex_pattern(pattern) {
                            Ok(cached) => cached,
                            Err(err) => return Some(Err(err)),
                        };
                        let Some(captures) = cached.regex.captures(text) else {
                            return Some(Ok(Value::Null));
                        };

                        let mut fields = Vec::with_capacity(cached.capture_names.len());
                        for name in &cached.capture_names {
                            fields.push(captures.name(name).map(|m| m.as_str().to_string()));
                        }

                        Some(Ok(Value::RegexCapture(Rc::new(RegexCaptureValue {
                            field_slots: cached.capture_slots.clone(),
                            fields,
                        }))))
                    }
                    _ => Some(Err(
                        "lustgex_capture_builtin expects string text and string pattern"
                            .to_string(),
                    )),
                }
            }
            "regex_capture" => {
                if args.len() != 3 {
                    return Some(Err(format!(
                        "regex_capture expects 3 args, got {}",
                        args.len()
                    )));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(text), Value::String(pattern), Value::List(names)) => {
                        let regex = match Regex::new(pattern) {
                            Ok(regex) => regex,
                            Err(e) => {
                                return Some(Err(format!(
                                    "regex_capture invalid regex '{}': {}",
                                    pattern, e
                                )));
                            }
                        };
                        let Some(captures) = regex.captures(text) else {
                            return Some(Ok(Value::Null));
                        };

                        let mut fields = HashMap::new();
                        for name_value in names.borrow().iter() {
                            let Value::String(name) = name_value else {
                                return Some(Err(
                                    "regex_capture expects a list of string capture names"
                                        .to_string(),
                                ));
                            };
                            let value = captures
                                .name(name)
                                .map(|m| Value::String(m.as_str().to_string()))
                                .unwrap_or(Value::Null);
                            fields.insert(name.clone(), value);
                        }

                        Some(Ok(Value::Struct(
                            "RegexCapture".to_string(),
                            Rc::new(RefCell::new(fields)),
                        )))
                    }
                    _ => Some(Err(
                        "regex_capture expects string text, string regex, and list capture names"
                            .to_string(),
                    )),
                }
            }
            _ => None,
        }
    }
}
