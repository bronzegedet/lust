use crate::bytecode::Value;
use crate::modules::draw;

use super::Vm;

impl Vm {
    pub(super) fn call_draw_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "window" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 3 {
                    return Some(Err(format!("window expects 3 args, got {}", args.len())));
                }
                let width = args[0].as_number().max(1.0) as u16;
                let height = args[1].as_number().max(1.0) as u16;
                let title = args[2].as_string();
                Some(draw::DrawRuntime::new(width, height, title).map(|runtime| {
                    self.draw_runtime = Some(runtime);
                    Value::Null
                }))
            }
            "live" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if !args.is_empty() {
                    return Some(Err(format!("live expects 0 args, got {}", args.len())));
                }
                let is_live = if let Some(runtime) = self.draw_runtime.as_mut() {
                    match runtime.live() {
                        Ok(value) => value,
                        Err(err) => return Some(Err(err)),
                    }
                } else {
                    false
                };
                Some(Ok(Value::Bool(is_live)))
            }
            "clear_screen" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 1 {
                    return Some(Err(format!(
                        "clear_screen expects 1 arg, got {}",
                        args.len()
                    )));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.clear_screen(&args[0].as_string());
                }
                Some(Ok(Value::Null))
            }
            "circle" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 4 {
                    return Some(Err(format!("circle expects 4 args, got {}", args.len())));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.circle(
                        args[0].as_number(),
                        args[1].as_number(),
                        args[2].as_number(),
                        &args[3].as_string(),
                    );
                }
                Some(Ok(Value::Null))
            }
            "rect" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 5 {
                    return Some(Err(format!("rect expects 5 args, got {}", args.len())));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.rect(
                        args[0].as_number(),
                        args[1].as_number(),
                        args[2].as_number(),
                        args[3].as_number(),
                        &args[4].as_string(),
                    );
                }
                Some(Ok(Value::Null))
            }
            "line" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 5 {
                    return Some(Err(format!("line expects 5 args, got {}", args.len())));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.line(
                        args[0].as_number(),
                        args[1].as_number(),
                        args[2].as_number(),
                        args[3].as_number(),
                        &args[4].as_string(),
                    );
                }
                Some(Ok(Value::Null))
            }
            "triangle" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 7 {
                    return Some(Err(format!("triangle expects 7 args, got {}", args.len())));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.triangle(
                        args[0].as_number(),
                        args[1].as_number(),
                        args[2].as_number(),
                        args[3].as_number(),
                        args[4].as_number(),
                        args[5].as_number(),
                        &args[6].as_string(),
                    );
                }
                Some(Ok(Value::Null))
            }
            "text" => {
                if let Err(err) = self.require_import("draw", name) {
                    return Some(Err(err));
                }
                if args.len() != 5 {
                    return Some(Err(format!("text expects 5 args, got {}", args.len())));
                }
                if let Some(runtime) = self.draw_runtime.as_mut() {
                    runtime.text(
                        &args[0].as_string(),
                        args[1].as_number(),
                        args[2].as_number(),
                        args[3].as_number(),
                        &args[4].as_string(),
                    );
                }
                Some(Ok(Value::Null))
            }
            _ => None,
        }
    }
}
