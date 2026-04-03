use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::BufWriter;
use std::rc::Rc;

use crate::bytecode::Value;

use super::Vm;

impl Vm {
    pub(super) fn call_io_builtin(&mut self, name: &str, args: &[Value]) -> Option<Result<Value, String>> {
        match name {
            "read_file" => {
                if args.len() != 1 {
                    return Some(Err(format!("read_file expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(path) => {
                        let content = match std::fs::read_to_string(path) {
                            Ok(content) => content,
                            Err(e) => return Some(Err(format!("read_file failed for '{}': {}", path, e))),
                        };
                        Some(Ok(Value::String(content)))
                    }
                    _ => Some(Err("read_file expects a string path".to_string())),
                }
            }
            "read_file_result" => {
                if args.len() != 1 {
                    return Some(Err(format!(
                        "read_file_result expects 1 arg, got {}",
                        args.len()
                    )));
                }
                match &args[0] {
                    Value::String(path) => match fs::read_to_string(path) {
                        Ok(content) => Some(Ok(Value::Enum(
                            "FileResult".to_string(),
                            "FileOk".to_string(),
                            vec![Value::String(content)],
                        ))),
                        Err(err) => Some(Ok(Value::Enum(
                            "FileResult".to_string(),
                            "FileErr".to_string(),
                            vec![Value::String(err.to_string())],
                        ))),
                    },
                    _ => Some(Err("read_file_result expects a string path".to_string())),
                }
            }
            "try_read_file" => {
                if args.len() != 1 {
                    return Some(Err(format!("try_read_file expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(path) => {
                        let content = std::fs::read_to_string(path).unwrap_or_default();
                        Some(Ok(Value::String(content)))
                    }
                    _ => Some(Err("try_read_file expects a string path".to_string())),
                }
            }
            "write_file" => {
                if args.len() != 2 {
                    return Some(Err(format!("write_file expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(path), Value::String(content)) => {
                        if let Err(e) = std::fs::write(path, content) {
                            return Some(Err(format!("write_file failed for '{}': {}", path, e)));
                        }
                        Some(Ok(Value::Null))
                    }
                    _ => Some(Err(
                        "write_file expects string path and string content".to_string(),
                    )),
                }
            }
            "open_file" => {
                if args.len() != 2 {
                    return Some(Err(format!("open_file expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(path), Value::String(mode)) => {
                        let mut options = OpenOptions::new();
                        match mode.as_str() {
                            "write" | "w" => {
                                options.write(true).create(true).truncate(true);
                            }
                            "append" | "a" => {
                                options.append(true).create(true);
                            }
                            _ => return Some(Err(format!("open_file unsupported mode '{}'", mode))),
                        }
                        let file = match options.open(path) {
                            Ok(file) => file,
                            Err(e) => return Some(Err(format!("open_file failed for '{}': {}", path, e))),
                        };
                        let handle_id = self.next_file_handle_id;
                        self.next_file_handle_id += 1;
                        self.open_files.insert(handle_id, BufWriter::new(file));
                        let mut fields = HashMap::new();
                        fields.insert("id".to_string(), Value::Number(handle_id as f64));
                        Some(Ok(Value::Struct(
                            "FileHandle".to_string(),
                            Rc::new(RefCell::new(fields)),
                        )))
                    }
                    _ => Some(Err("open_file expects string path and string mode".to_string())),
                }
            }
            "list_dir" => {
                if args.len() != 1 {
                    return Some(Err(format!("list_dir expects 1 arg, got {}", args.len())));
                }
                match &args[0] {
                    Value::String(path) => {
                        let mut entries = Vec::new();
                        let dir_iter = match fs::read_dir(path) {
                            Ok(iter) => iter,
                            Err(e) => return Some(Err(format!("list_dir failed for '{}': {}", path, e))),
                        };
                        for entry_result in dir_iter {
                            let Ok(entry) = entry_result else {
                                continue;
                            };
                            let Ok(name) = entry.file_name().into_string() else {
                                continue;
                            };
                            entries.push(name);
                        }
                        entries.sort();
                        Some(Ok(Value::List(Rc::new(RefCell::new(
                            entries.into_iter().map(Value::String).collect(),
                        )))))
                    }
                    _ => Some(Err("list_dir expects a string path".to_string())),
                }
            }
            "get_args" => {
                let values = self
                    .args
                    .iter()
                    .map(|arg| Value::String(arg.clone()))
                    .collect::<Vec<_>>();
                Some(Ok(Value::List(Rc::new(RefCell::new(values)))))
            }
            "launch_lust" => {
                if args.len() != 2 {
                    return Some(Err(format!("launch_lust expects 2 args, got {}", args.len())));
                }
                match (&args[0], &args[1]) {
                    (Value::String(mode), Value::String(path)) => {
                        let exe = match std::env::current_exe() {
                            Ok(exe) => exe,
                            Err(e) => {
                                return Some(Err(format!(
                                    "launch_lust failed to resolve current executable: {}",
                                    e
                                )));
                            }
                        };
                        let status = match std::process::Command::new(exe).arg(mode).arg(path).status() {
                            Ok(status) => status,
                            Err(e) => return Some(Err(format!("launch_lust failed: {}", e))),
                        };
                        Some(Ok(Value::Number(status.code().unwrap_or(1) as f64)))
                    }
                    _ => Some(Err("launch_lust expects string mode and string path".to_string())),
                }
            }
            _ => None,
        }
    }
}
