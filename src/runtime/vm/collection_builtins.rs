use std::cell::RefCell;
use std::rc::Rc;

use crate::bytecode::Value;

use super::{optional_slice_bound, Vm};

impl Vm {
    pub(super) fn call_collection_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "__slice_range" => {
                if args.len() != 3 {
                    return Some(Err(format!(
                        "slice_range expects 3 args, got {}",
                        args.len()
                    )));
                }
                let start = match optional_slice_bound(&args[1]) {
                    Ok(value) => value,
                    Err(err) => return Some(Err(err)),
                };
                let end = match optional_slice_bound(&args[2]) {
                    Ok(value) => value,
                    Err(err) => return Some(Err(err)),
                };
                match &args[0] {
                    Value::String(s) => {
                        let start = start.unwrap_or(0);
                        let end = end.unwrap_or(s.len());
                        if start <= end && end <= s.len() {
                            Some(Ok(Value::String(s[start..end].to_string())))
                        } else {
                            Some(Ok(Value::Null))
                        }
                    }
                    Value::List(items) => {
                        let items = items.borrow();
                        let start = start.unwrap_or(0);
                        let end = end.unwrap_or(items.len());
                        if start <= end && end <= items.len() {
                            Some(Ok(Value::List(Rc::new(RefCell::new(
                                items[start..end].to_vec(),
                            )))))
                        } else {
                            Some(Ok(Value::Null))
                        }
                    }
                    _ => Some(Ok(Value::Null)),
                }
            }
            _ => None,
        }
    }
}
