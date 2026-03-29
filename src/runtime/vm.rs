use std::collections::HashMap;
use std::cell::RefCell;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use crossterm::event::{read, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use regex::Regex;

use crate::bytecode::{Function, Instruction, Program, RegexCaptureValue, Value};
use crate::modules::audio;

struct CallFrame {
    function: String,
    ip: usize,
    locals: Vec<Value>,
}

struct CachedLustgexPattern {
    compiled: String,
    regex: Regex,
    capture_names: Vec<String>,
    capture_slots: Rc<HashMap<String, usize>>,
}

pub struct Vm {
    program: Program,
    frames: Vec<CallFrame>,
    stack: Vec<Value>,
    globals: HashMap<String, Value>,
    output: Vec<String>,
    args: Vec<String>,
    key_inputs: Vec<String>,
    input_lines: Vec<String>,
    open_files: HashMap<u64, BufWriter<File>>,
    next_file_handle_id: u64,
    profile_ops: bool,
    instruction_count: usize,
    opcode_counts: HashMap<&'static str, usize>,
    lustgex_cache: HashMap<String, Rc<CachedLustgexPattern>>,
}

impl Vm {
    pub fn new(program: Program) -> Self {
        Self::new_with_args_keys_and_input(program, Vec::new(), Vec::new(), Vec::new())
    }

    pub fn new_with_args(program: Program, args: Vec<String>) -> Self {
        Self::new_with_args_keys_and_input(program, args, Vec::new(), Vec::new())
    }

    pub fn new_with_args_and_keys(program: Program, args: Vec<String>, key_inputs: Vec<String>) -> Self {
        Self::new_with_args_keys_and_input(program, args, key_inputs, Vec::new())
    }

    pub fn new_with_args_keys_and_input(
        program: Program,
        args: Vec<String>,
        key_inputs: Vec<String>,
        input_lines: Vec<String>,
    ) -> Self {
        Self {
            program,
            frames: Vec::new(),
            stack: Vec::new(),
            globals: HashMap::new(),
            output: Vec::new(),
            args,
            key_inputs,
            input_lines,
            open_files: HashMap::new(),
            next_file_handle_id: 1,
            profile_ops: std::env::var("LUST_PROFILE_OPS")
                .map(|value| {
                    let trimmed = value.trim();
                    !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
                })
                .unwrap_or(false),
            instruction_count: 0,
            opcode_counts: HashMap::new(),
            lustgex_cache: HashMap::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.install_import_globals();
        let entry = self.program.entry.clone();
        self.call_function(&entry, Vec::new())?;

        loop {
            if self.frames.is_empty() {
                self.finish_opcode_profile();
                return Ok(());
            }

            let instruction = self.current_instruction()?;
            self.current_frame_mut()?.ip += 1;
            self.execute_instruction(instruction)?;
        }
    }

    fn execute_instruction(&mut self, instruction: Instruction) -> Result<(), String> {
        self.record_instruction(&instruction);
        match instruction {
            Instruction::Constant(idx) => {
                let function = self.current_function()?;
                let value = function
                    .chunk
                    .constants
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| format!("constant index {} out of bounds", idx))?;
                self.stack.push(value);
            }
            Instruction::LoadGlobal(name) => {
                let value = self
                    .globals
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("undefined variable '{}'", name))?;
                self.stack.push(value);
            }
            Instruction::StoreGlobal(name) => {
                let value = self.pop()?;
                self.globals.insert(name, value);
            }
            Instruction::LoadLocal(slot) => {
                let value = self
                    .current_frame()?
                    .locals
                    .get(slot)
                    .cloned()
                    .ok_or_else(|| format!("local slot {} out of bounds", slot))?;
                self.stack.push(value);
            }
            Instruction::StoreLocal(slot) => {
                let value = self.pop()?;
                let frame = self.current_frame_mut()?;
                if slot >= frame.locals.len() {
                    frame.locals.resize(slot + 1, Value::Null);
                }
                frame.locals[slot] = value;
            }
            Instruction::BuildList(count) => {
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.pop()?);
                }
                items.reverse();
                self.stack.push(Value::List(Rc::new(RefCell::new(items))));
            }
            Instruction::BuildStruct(name, fields) => {
                let expected_fields = self
                    .program
                    .struct_defs
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("unknown struct '{}'", name))?;
                let mut items = HashMap::new();
                for field in fields.iter().rev() {
                    let value = self.pop()?;
                    if !expected_fields.contains(field) {
                        return Err(format!("unknown field '{}' for struct '{}'", field, name));
                    }
                    items.insert(field.clone(), value);
                }
                self.stack
                    .push(Value::Struct(name, Rc::new(RefCell::new(items))));
            }
            Instruction::BuildEnum(enum_name, variant, arity) => {
                let mut values = Vec::with_capacity(arity);
                for _ in 0..arity {
                    values.push(self.pop()?);
                }
                values.reverse();
                self.stack.push(Value::Enum(enum_name, variant, values));
            }
            Instruction::IndexGet => {
                let index = self.pop()?;
                let target = self.pop()?;
                match (target, index) {
                    (Value::List(items), Value::Number(i)) => {
                        let idx = i as usize;
                        let value = items
                            .borrow()
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| {
                                let frame = self.current_frame().ok();
                                if let Some(frame) = frame {
                                    format!(
                                        "index out of bounds in {}@ip{}: {} on list of len {}",
                                        frame.function,
                                        frame.ip.saturating_sub(1),
                                        idx,
                                        items.borrow().len()
                                    )
                                } else {
                                    format!("index out of bounds: {}", idx)
                                }
                            })?;
                        self.stack.push(value);
                    }
                    (Value::String(s), Value::Number(i)) => {
                        let idx = i as usize;
                        let value = s
                            .chars()
                            .nth(idx)
                            .map(|c| Value::String(c.to_string()))
                            .ok_or_else(|| {
                                let frame = self.current_frame().ok();
                                if let Some(frame) = frame {
                                    format!(
                                        "index out of bounds in {}@ip{}: {} on string of len {}",
                                        frame.function,
                                        frame.ip.saturating_sub(1),
                                        idx,
                                        s.chars().count()
                                    )
                                } else {
                                    format!("index out of bounds: {}", idx)
                                }
                            })?;
                        self.stack.push(value);
                    }
                    (Value::Map(items), Value::String(key)) => {
                        let value = items.borrow().get(&key).cloned().unwrap_or(Value::Null);
                        self.stack.push(value);
                    }
                    _ => return Err("indexing is only supported on lists, maps, and strings".to_string()),
                }
            }
            Instruction::IndexSet => {
                let value = self.pop()?;
                let index = self.pop()?;
                let target = self.pop()?;
                match (target, index) {
                    (Value::List(items), Value::Number(i)) => {
                        let idx = i as usize;
                        let mut items = items.borrow_mut();
                        if idx >= items.len() {
                            return Err(format!("index out of bounds: {}", idx));
                        }
                        items[idx] = value;
                    }
                    (Value::Map(items), Value::String(key)) => {
                        items.borrow_mut().insert(key, value);
                    }
                    _ => {
                        return Err("index assignment is only supported on lists and maps".to_string())
                    }
                }
            }
            Instruction::MemberGet(field) => {
                let target = self.pop()?;
                match target {
                    Value::Struct(_, fields) => {
                        let value = fields
                            .borrow()
                            .get(&field)
                            .cloned()
                            .ok_or_else(|| format!("field '{}' not found in struct", field))?;
                        self.stack.push(value);
                    }
                    Value::RegexCapture(capture) => {
                        let value = capture
                            .field_slots
                            .get(&field)
                            .and_then(|slot| capture.fields.get(*slot))
                            .and_then(|value| value.as_ref())
                            .map(|value| Value::String(value.clone()))
                            .unwrap_or(Value::Null);
                        self.stack.push(value);
                    }
                    _ => return Err(format!("member '{}' not found on target value", field)),
                }
            }
            Instruction::MemberSet(field) => {
                let value = self.pop()?;
                let target = self.pop()?;
                match target {
                    Value::Struct(name, fields) => {
                        let expected_fields = self
                            .program
                            .struct_defs
                            .get(&name)
                            .cloned()
                            .ok_or_else(|| format!("unknown struct '{}'", name))?;
                        if !expected_fields.contains(&field) {
                            return Err(format!("field '{}' not found in struct '{}'", field, name));
                        }
                        fields.borrow_mut().insert(field, value);
                    }
                    _ => return Err("member assignment is only supported on structs".to_string()),
                }
            }
            Instruction::Add => self.binary_add()?,
            Instruction::Sub => self.binary_number(|a, b| a - b)?,
            Instruction::Mul => self.binary_number(|a, b| a * b)?,
            Instruction::Div => self.binary_number(|a, b| a / b)?,
            Instruction::Mod => self.binary_number(|a, b| a % b)?,
            Instruction::Eq => self.binary_compare(|a, b| a == b)?,
            Instruction::Ne => self.binary_compare(|a, b| a != b)?,
            Instruction::Gt => self.binary_cmp_number(|a, b| a > b)?,
            Instruction::Ge => self.binary_cmp_number(|a, b| a >= b)?,
            Instruction::Lt => self.binary_cmp_number(|a, b| a < b)?,
            Instruction::Le => self.binary_cmp_number(|a, b| a <= b)?,
            Instruction::And => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.stack.push(Value::Bool(left.truthy() && right.truthy()));
            }
            Instruction::Or => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.stack.push(Value::Bool(left.truthy() || right.truthy()));
            }
            Instruction::Not => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(!value.truthy()));
            }
            Instruction::Call(name, argc) => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.pop()?);
                }
                args.reverse();
                self.call_function(&name, args)?;
            }
            Instruction::CallMethod(name, argc) => {
                let mut args = Vec::with_capacity(argc + 1);
                for _ in 0..argc {
                    args.push(self.pop()?);
                }
                args.reverse();
                let target = self.pop()?;
                if let Some(value) = self.call_value_method(&target, &name, &args)? {
                    self.stack.push(value);
                    return Ok(());
                }
                let function_name = match &target {
                    Value::Struct(type_name, _) => format!("{}.{}", type_name, name),
                    _ => return Err(format!("method '{}' not found on target value", name)),
                };
                let mut full_args = Vec::with_capacity(argc + 1);
                full_args.push(target);
                full_args.extend(args);
                self.call_function(&function_name, full_args)?;
            }
            Instruction::CallBuiltin(name, argc) => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.pop()?);
                }
                args.reverse();

                if name == "map" || name == "filter" {
                    if args.len() != 2 {
                        return Err(format!("{} expects 2 args, got {}", name, args.len()));
                    }
                    let items = match &args[0] {
                        Value::List(l) => l.borrow().clone(),
                        _ => return Err(format!("{} expects list as first argument", name)),
                    };
                    let func_name = match &args[1] {
                        Value::Function(n) => n.clone(),
                        _ => return Err(format!("{} expects function as second argument", name)),
                    };

                    let mut results = Vec::new();
                    for item in items {
                        self.call_function(&func_name, vec![item.clone()])?;
                        
                        // Execute until the function returns
                        let initial_frame_count = self.frames.len();
                        while self.frames.len() >= initial_frame_count {
                            let inst = self.current_instruction()?;
                            self.current_frame_mut()?.ip += 1;
                            self.execute_instruction(inst)?;
                        }
                        
                        let res = self.pop()?;
                        if name == "map" {
                            results.push(res);
                        } else {
                            if res.truthy() {
                                results.push(item);
                            }
                        }
                    }

                    self.stack.push(Value::List(Rc::new(RefCell::new(results))));
                    return Ok(());
                }

                if name == "map_values" || name == "filter_values" || name == "map_entries" || name == "filter_entries" {
                    if args.len() != 2 {
                        return Err(format!("{} expects 2 args, got {}", name, args.len()));
                    }
                    let items = match &args[0] {
                        Value::Map(m) => m.borrow().clone(),
                        _ => return Err(format!("{} expects map as first argument", name)),
                    };
                    let func_name = match &args[1] {
                        Value::Function(n) => n.clone(),
                        _ => return Err(format!("{} expects function as second argument", name)),
                    };

                    let mut keys = items.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    let mut results = HashMap::new();
                    for key in keys {
                        let item = items.get(&key).cloned().unwrap_or(Value::Null);
                        let input = if name == "map_entries" || name == "filter_entries" {
                            Value::List(Rc::new(RefCell::new(vec![
                                Value::String(key.clone()),
                                item.clone(),
                            ])))
                        } else {
                            item.clone()
                        };
                        self.call_function(&func_name, vec![input])?;

                        let initial_frame_count = self.frames.len();
                        while self.frames.len() >= initial_frame_count {
                            let inst = self.current_instruction()?;
                            self.current_frame_mut()?.ip += 1;
                            self.execute_instruction(inst)?;
                        }

                        let res = self.pop()?;
                        if name == "map_values" {
                            results.insert(key, res);
                        } else if name == "map_entries" {
                            match res {
                                Value::List(pair) => {
                                    let pair = pair.borrow();
                                    if pair.len() != 2 {
                                        return Err("map_entries expects lambda to return [key, value]".to_string());
                                    }
                                    let new_key = match &pair[0] {
                                        Value::String(text) => text.clone(),
                                        _ => return Err("map_entries expects string keys".to_string()),
                                    };
                                    results.insert(new_key, pair[1].clone());
                                }
                                _ => return Err("map_entries expects lambda to return [key, value]".to_string()),
                            }
                        } else if res.truthy() {
                            results.insert(key, item);
                        }
                    }

                    self.stack.push(Value::Map(Rc::new(RefCell::new(results))));
                    return Ok(());
                }

                let value = self.call_builtin(&name, args)?;
                self.stack.push(value);
            }
            Instruction::CallDynamic(argc) => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.pop()?);
                }
                args.reverse();
                let target = self.pop()?;
                match target {
                    Value::Function(name) => {
                        self.call_function(&name, args)?;
                    }
                    _ => return Err(format!("expected function, found {}", target.type_name())),
                }
            }
            Instruction::LoadFunction(name) => {
                self.stack.push(Value::Function(name));
            }
            Instruction::ListPush => {
                let value = self.pop()?;
                let target = self.pop()?;
                match target {
                    Value::List(items) => {
                        items.borrow_mut().push(value);
                        self.stack.push(Value::Null);
                    }
                    other => {
                        let frame = self.current_frame()?;
                        return Err(format!(
                            "push() is only supported on lists in {}@ip{}, found {} ({})",
                            frame.function,
                            frame.ip.saturating_sub(1),
                            other.type_name(),
                            other
                        ))
                    }
                }
            }
            Instruction::ListLen => {
                let target = self.pop()?;
                match target {
                    Value::List(items) => self.stack.push(Value::Number(items.borrow().len() as f64)),
                    Value::Map(items) => self.stack.push(Value::Number(items.borrow().len() as f64)),
                    Value::String(s) => self.stack.push(Value::Number(s.len() as f64)),
                    _ => return Err("length() is only supported on lists, maps, and strings".to_string()),
                }
            }
            Instruction::IsList => {
                let target = self.pop()?;
                self.stack.push(Value::Bool(matches!(target, Value::List(_))));
            }
            Instruction::StructIsType(name) => {
                let target = self.pop()?;
                match target {
                    Value::Struct(struct_name, _) => {
                        self.stack.push(Value::Bool(struct_name == name));
                    }
                    Value::RegexCapture(_) => {
                        self.stack.push(Value::Bool(name == "RegexCapture"));
                    }
                    _ => self.stack.push(Value::Bool(false)),
                }
            }
            Instruction::StructGetField(field) => {
                let target = self.pop()?;
                match target {
                    Value::Struct(_, fields) => {
                        let value = fields.borrow().get(&field).cloned().unwrap_or(Value::Null);
                        self.stack.push(value);
                    }
                    Value::RegexCapture(capture) => {
                        let value = capture
                            .field_slots
                            .get(&field)
                            .and_then(|slot| capture.fields.get(*slot))
                            .and_then(|value| value.as_ref())
                            .map(|value| Value::String(value.clone()))
                            .unwrap_or(Value::Null);
                        self.stack.push(value);
                    }
                    _ => self.stack.push(Value::Null),
                }
            }
            Instruction::EnumIsVariant(enum_name, variant) => {
                let target = self.pop()?;
                match target {
                    Value::Enum(target_enum, target_variant, _) => {
                        self.stack.push(Value::Bool(
                            target_enum == enum_name && target_variant == variant,
                        ));
                    }
                    _ => self.stack.push(Value::Bool(false)),
                }
            }
            Instruction::EnumGetField(index) => {
                let target = self.pop()?;
                match target {
                    Value::Enum(_, _, values) => {
                        let value = values
                            .get(index)
                            .cloned()
                            .ok_or_else(|| format!("enum payload index out of bounds: {}", index))?;
                        self.stack.push(value);
                    }
                    _ => return Err("enum field access is only supported on enum values".to_string()),
                }
            }
            Instruction::Return => {
                let ret = self.pop()?;
                self.frames.pop();
                if !self.frames.is_empty() {
                    self.stack.push(ret);
                }
            }
            Instruction::Print(count) => {
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.pop()?.as_string());
                }
                parts.reverse();
                let line = parts.join(" ");
                println!("{}", line);
                self.output.push(line);
            }
            Instruction::Pop => {
                self.pop()?;
            }
            Instruction::JumpIfFalse(target) => {
                let cond = self.peek()?;
                if !cond.truthy() {
                    self.current_frame_mut()?.ip = target;
                }
            }
            Instruction::Jump(target) => {
                self.current_frame_mut()?.ip = target;
            }
            Instruction::Halt => {
                self.frames.clear();
            }
        }
        Ok(())
    }

    pub fn output(&self) -> &[String] {
        &self.output
    }

    fn record_instruction(&mut self, instruction: &Instruction) {
        if !self.profile_ops {
            return;
        }

        self.instruction_count += 1;
        *self.opcode_counts.entry(instruction_name(instruction)).or_default() += 1;
    }

    fn finish_opcode_profile(&self) {
        if !self.profile_ops {
            return;
        }

        let mut counts = self
            .opcode_counts
            .iter()
            .map(|(name, count)| (*name, *count))
            .collect::<Vec<_>>();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        eprintln!("lust vm op profile:");
        eprintln!("  total instructions {:>12}", self.instruction_count);
        for (name, count) in counts.into_iter().take(10) {
            let pct = if self.instruction_count == 0 {
                0.0
            } else {
                (count as f64 / self.instruction_count as f64) * 100.0
            };
            eprintln!("  {:<16} {:>12} {:>7.2}%", name, count, pct);
        }
    }

    fn get_or_compile_lustgex_pattern(
        &mut self,
        pattern: &str,
    ) -> Result<Rc<CachedLustgexPattern>, String> {
        if let Some(cached) = self.lustgex_cache.get(pattern) {
            return Ok(cached.clone());
        }

        let compiled = compile_lustgex(pattern)?;
        let regex = Regex::new(&compiled)
            .map_err(|e| format!("lustgex invalid regex '{}': {}", compiled, e))?;
        let capture_names = extract_lustgex_capture_names(pattern);
        let mut capture_slots = HashMap::new();
        for (index, name) in capture_names.iter().enumerate() {
            capture_slots.insert(name.clone(), index);
        }
        let cached = Rc::new(CachedLustgexPattern {
            compiled,
            regex,
            capture_names,
            capture_slots: Rc::new(capture_slots),
        });
        self.lustgex_cache
            .insert(pattern.to_string(), cached.clone());
        Ok(cached)
    }

    fn install_import_globals(&mut self) {
        if self.program.imports.contains("draw") {
            for color in ["red", "green", "blue", "white", "black", "dark_gray", "neon_pink"] {
                self.globals
                    .entry(color.to_string())
                    .or_insert_with(|| Value::String(color.to_string()));
            }
        }
    }

    fn call_builtin(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        match name {
            "println" => {
                let line = args
                    .iter()
                    .map(Value::as_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("{}", line);
                self.output.push(line);
                Ok(Value::Null)
            }
            "to_string" => {
                if args.len() != 1 {
                    return Err(format!("to_string expects 1 arg, got {}", args.len()));
                }
                Ok(Value::String(args[0].as_string()))
            }
            "to_number" => {
                if args.len() != 1 {
                    return Err(format!("to_number expects 1 arg, got {}", args.len()));
                }
                Ok(Value::Number(args[0].as_string().parse::<f64>().unwrap_or(0.0)))
            }
            "type_of" => {
                if args.len() != 1 {
                    return Err(format!("type_of expects 1 arg, got {}", args.len()));
                }
                Ok(Value::String(args[0].type_name()))
            }
            "debug" => {
                if args.len() != 2 {
                    return Err(format!("debug expects 2 args, got {}", args.len()));
                }
                let line = format!("DEBUG {} {}", args[0], args[1]);
                println!("{}", line);
                self.output.push(line);
                Ok(args[1].clone())
            }
            "panic" => {
                if args.len() != 1 {
                    return Err(format!("panic expects 1 arg, got {}", args.len()));
                }
                Err(format!("lust panic: {}", args[0]))
            }
            "assert" => {
                if args.len() != 2 {
                    return Err(format!("assert expects 2 args, got {}", args.len()));
                }
                if args[0].truthy() {
                    Ok(Value::Null)
                } else {
                    Err(format!("lust assert failed: {}", args[1]))
                }
            }
            "__str_trim" => {
                if args.len() != 1 {
                    return Err(format!("trim expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(s) => Ok(Value::String(s.trim().to_string())),
                    _ => Err("trim() is only supported on strings".to_string()),
                }
            }
            "__str_at" => {
                if args.len() != 2 {
                    return Err(format!("at expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::Number(idx)) => {
                        let idx = *idx as usize;
                        if let Some(c) = s.chars().nth(idx) {
                            Ok(Value::String(c.to_string()))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    _ => Err("at() expects a string target and numeric index".to_string()),
                }
            }
            "__str_slice" => {
                if args.len() != 3 {
                    return Err(format!("slice expects 3 args, got {}", args.len()));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(s), Value::Number(start), Value::Number(end)) => {
                        let start = *start as usize;
                        let end = *end as usize;
                        if start <= end && end <= s.len() {
                            Ok(Value::String(s[start..end].to_string()))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    _ => Err("slice() expects a string target and numeric bounds".to_string()),
                }
            }
            "__slice_range" => {
                if args.len() != 3 {
                    return Err(format!("slice_range expects 3 args, got {}", args.len()));
                }
                let start = optional_slice_bound(&args[1])?;
                let end = optional_slice_bound(&args[2])?;
                match &args[0] {
                    Value::String(s) => {
                        let start = start.unwrap_or(0);
                        let end = end.unwrap_or(s.len());
                        if start <= end && end <= s.len() {
                            Ok(Value::String(s[start..end].to_string()))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    Value::List(items) => {
                        let items = items.borrow();
                        let start = start.unwrap_or(0);
                        let end = end.unwrap_or(items.len());
                        if start <= end && end <= items.len() {
                            Ok(Value::List(Rc::new(RefCell::new(items[start..end].to_vec()))))
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    _ => Ok(Value::Null),
                }
            }
            "__str_contains" => {
                if args.len() != 2 {
                    return Err(format!("contains expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(sub)) => Ok(Value::Bool(s.contains(sub))),
                    _ => Err("contains() expects string target and string argument".to_string()),
                }
            }
            "__str_starts_with" => {
                if args.len() != 2 {
                    return Err(format!("starts_with expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(prefix)) => Ok(Value::Bool(s.starts_with(prefix))),
                    _ => Err("starts_with() expects string target and string argument".to_string()),
                }
            }
            "__str_ends_with" => {
                if args.len() != 2 {
                    return Err(format!("ends_with expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(suffix)) => Ok(Value::Bool(s.ends_with(suffix))),
                    _ => Err("ends_with() expects string target and string argument".to_string()),
                }
            }
            "__str_split" => {
                if args.len() != 2 {
                    return Err(format!("split expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(sep)) => {
                        let parts = s
                            .split(sep)
                            .map(|part| Value::String(part.to_string()))
                            .collect::<Vec<_>>();
                        Ok(Value::List(Rc::new(RefCell::new(parts))))
                    }
                    _ => Err("split() expects string target and string separator".to_string()),
                }
            }
            "__str_lines" => {
                if args.len() != 1 {
                    return Err(format!("lines expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(s) => {
                        let parts = s
                            .lines()
                            .map(|part| Value::String(part.to_string()))
                            .collect::<Vec<_>>();
                        Ok(Value::List(Rc::new(RefCell::new(parts))))
                    }
                    _ => Err("lines() is only supported on strings".to_string()),
                }
            }
            "__str_to_list" => {
                if args.len() != 1 {
                    return Err(format!("to_list expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(s) => {
                        let parts = s
                            .chars()
                            .map(|c| Value::String(c.to_string()))
                            .collect::<Vec<_>>();
                        Ok(Value::List(Rc::new(RefCell::new(parts))))
                    }
                    _ => Err("to_list() is only supported on strings".to_string()),
                }
            }
            "__str_replace" => {
                if args.len() != 3 {
                    return Err(format!("replace expects 3 args, got {}", args.len()));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(s), Value::String(from), Value::String(to)) => {
                        Ok(Value::String(s.replace(from, to)))
                    }
                    _ => Err("replace() expects string target and string arguments".to_string()),
                }
            }
            "read_file" => {
                if args.len() != 1 {
                    return Err(format!("read_file expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(path) => {
                        let content = std::fs::read_to_string(path)
                            .map_err(|e| format!("read_file failed for '{}': {}", path, e))?;
                        Ok(Value::String(content))
                    }
                    _ => Err("read_file expects a string path".to_string()),
                }
            }
            "read_file_result" => {
                if args.len() != 1 {
                    return Err(format!("read_file_result expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(path) => match fs::read_to_string(path) {
                        Ok(content) => Ok(Value::Enum(
                            "FileResult".to_string(),
                            "FileOk".to_string(),
                            vec![Value::String(content)],
                        )),
                        Err(err) => Ok(Value::Enum(
                            "FileResult".to_string(),
                            "FileErr".to_string(),
                            vec![Value::String(err.to_string())],
                        )),
                    },
                    _ => Err("read_file_result expects a string path".to_string()),
                }
            }
            "try_read_file" => {
                if args.len() != 1 {
                    return Err(format!("try_read_file expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(path) => {
                        let content = std::fs::read_to_string(path).unwrap_or_default();
                        Ok(Value::String(content))
                    }
                    _ => Err("try_read_file expects a string path".to_string()),
                }
            }
            "write_file" => {
                if args.len() != 2 {
                    return Err(format!("write_file expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(path), Value::String(content)) => {
                        std::fs::write(path, content)
                            .map_err(|e| format!("write_file failed for '{}': {}", path, e))?;
                        Ok(Value::Null)
                    }
                    _ => Err("write_file expects string path and string content".to_string()),
                }
            }
            "open_file" => {
                if args.len() != 2 {
                    return Err(format!("open_file expects 2 args, got {}", args.len()));
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
                            _ => return Err(format!("open_file unsupported mode '{}'", mode)),
                        }
                        let file = options
                            .open(path)
                            .map_err(|e| format!("open_file failed for '{}': {}", path, e))?;
                        let handle_id = self.next_file_handle_id;
                        self.next_file_handle_id += 1;
                        self.open_files.insert(handle_id, BufWriter::new(file));
                        let mut fields = HashMap::new();
                        fields.insert("id".to_string(), Value::Number(handle_id as f64));
                        Ok(Value::Struct(
                            "FileHandle".to_string(),
                            Rc::new(RefCell::new(fields)),
                        ))
                    }
                    _ => Err("open_file expects string path and string mode".to_string()),
                }
            }
            "list_dir" => {
                if args.len() != 1 {
                    return Err(format!("list_dir expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(path) => {
                        let mut entries = Vec::new();
                        for entry_result in fs::read_dir(path)
                            .map_err(|e| format!("list_dir failed for '{}': {}", path, e))?
                        {
                            let Ok(entry) = entry_result else {
                                continue;
                            };
                            let Ok(name) = entry.file_name().into_string() else {
                                continue;
                            };
                            entries.push(name);
                        }
                        entries.sort();
                        Ok(Value::List(Rc::new(RefCell::new(
                            entries.into_iter().map(Value::String).collect(),
                        ))))
                    }
                    _ => Err("list_dir expects a string path".to_string()),
                }
            }
            "compile_lustgex" => {
                if args.len() != 1 {
                    return Err(format!("compile_lustgex expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(pattern) => Ok(Value::String(
                        self.get_or_compile_lustgex_pattern(pattern)?.compiled.clone(),
                    )),
                    _ => Err("compile_lustgex expects a string pattern".to_string()),
                }
            }
            "lustgex_match" => {
                if args.len() != 2 {
                    return Err(format!("lustgex_match expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(text), Value::String(pattern)) => {
                        let cached = self.get_or_compile_lustgex_pattern(pattern)?;
                        Ok(Value::Bool(cached.regex.is_match(text)))
                    }
                    _ => Err("lustgex_match expects string text and string pattern".to_string()),
                }
            }
            "lustgex_capture_builtin" => {
                if args.len() != 2 {
                    return Err(format!("lustgex_capture_builtin expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(text), Value::String(pattern)) => {
                        let cached = self.get_or_compile_lustgex_pattern(pattern)?;
                        let Some(captures) = cached.regex.captures(text) else {
                            return Ok(Value::Null);
                        };

                        let mut fields = Vec::with_capacity(cached.capture_names.len());
                        for name in &cached.capture_names {
                            fields.push(captures.name(name).map(|m| m.as_str().to_string()));
                        }

                        Ok(Value::RegexCapture(Rc::new(RegexCaptureValue {
                            field_slots: cached.capture_slots.clone(),
                            fields,
                        })))
                    }
                    _ => Err("lustgex_capture_builtin expects string text and string pattern".to_string()),
                }
            }
            "regex_capture" => {
                if args.len() != 3 {
                    return Err(format!("regex_capture expects 3 args, got {}", args.len()));
                }
                match (&args[0], &args[1], &args[2]) {
                    (Value::String(text), Value::String(pattern), Value::List(names)) => {
                        let regex = Regex::new(pattern)
                            .map_err(|e| format!("regex_capture invalid regex '{}': {}", pattern, e))?;
                        let Some(captures) = regex.captures(text) else {
                            return Ok(Value::Null);
                        };

                        let mut fields = HashMap::new();
                        for name_value in names.borrow().iter() {
                            let Value::String(name) = name_value else {
                                return Err("regex_capture expects a list of string capture names".to_string());
                            };
                            let value = captures
                                .name(name)
                                .map(|m| Value::String(m.as_str().to_string()))
                                .unwrap_or(Value::Null);
                            fields.insert(name.clone(), value);
                        }

                        Ok(Value::Struct(
                            "RegexCapture".to_string(),
                            Rc::new(RefCell::new(fields)),
                        ))
                    }
                    _ => Err("regex_capture expects string text, string regex, and list capture names".to_string()),
                }
            }
            "get_args" => {
                let values = self
                    .args
                    .iter()
                    .map(|arg| Value::String(arg.clone()))
                    .collect::<Vec<_>>();
                Ok(Value::List(Rc::new(RefCell::new(values))))
            }
            "launch_lust" => {
                if args.len() != 2 {
                    return Err(format!("launch_lust expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::String(mode), Value::String(path)) => {
                        let exe = std::env::current_exe()
                            .map_err(|e| format!("launch_lust failed to resolve current executable: {}", e))?;
                        let status = std::process::Command::new(exe)
                            .arg(mode)
                            .arg(path)
                            .status()
                            .map_err(|e| format!("launch_lust failed: {}", e))?;
                        Ok(Value::Number(status.code().unwrap_or(1) as f64))
                    }
                    _ => Err("launch_lust expects string mode and string path".to_string()),
                }
            }
            "input" => {
                let raw = if !self.input_lines.is_empty() {
                    self.input_lines.remove(0)
                } else {
                    use std::io::{self, Write};
                    let mut s = String::new();
                    let _ = io::stdout().flush();
                    io::stdin()
                        .read_line(&mut s)
                        .map_err(|e| format!("input failed: {}", e))?;
                    s
                };
                Ok(Value::String(raw.trim().to_string()))
            }
            "clr" => {
                if !args.is_empty() {
                    return Err(format!("clr expects 0 args, got {}", args.len()));
                }
                use std::io::{self, Write};
                print!("\x1B[2J\x1B[H");
                io::stdout()
                    .flush()
                    .map_err(|e| format!("clr failed: {}", e))?;
                Ok(Value::Null)
            }
            "prompt" => {
                if args.len() != 1 {
                    return Err(format!("prompt expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(message) => {
                        use std::io::{self, Write};
                        print!("{}", message);
                        io::stdout()
                            .flush()
                            .map_err(|e| format!("prompt failed: {}", e))?;
                        let raw = if !self.input_lines.is_empty() {
                            self.input_lines.remove(0)
                        } else {
                            let mut s = String::new();
                            io::stdin()
                                .read_line(&mut s)
                                .map_err(|e| format!("prompt failed: {}", e))?;
                            s
                        };
                        Ok(Value::String(raw.trim().to_string()))
                    }
                    _ => Err("prompt expects a string message".to_string()),
                }
            }
            "get_env" => {
                if args.len() != 1 {
                    return Err(format!("get_env expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(key) => Ok(Value::String(std::env::var(key).unwrap_or_default())),
                    _ => Err("get_env expects a string key".to_string()),
                }
            }
            "get_key" => {
                if !self.key_inputs.is_empty() {
                    let raw = self.key_inputs.remove(0);
                    let trimmed = raw.trim().to_lowercase();
                    let (variant, payload) = match trimmed.as_str() {
                        "up" => ("Up", vec![]),
                        "down" => ("Down", vec![]),
                        "left" => ("Left", vec![]),
                        "right" => ("Right", vec![]),
                        "enter" => ("Enter", vec![]),
                        "esc" => ("Esc", vec![]),
                        _ => {
                            let mut chars = trimmed.chars();
                            match (chars.next(), chars.next()) {
                                (Some(ch), None) => ("Char", vec![Value::String(ch.to_string())]),
                                _ => ("None", vec![]),
                            }
                        }
                    };
                    return Ok(Value::Enum("Key".to_string(), variant.to_string(), payload));
                }
                
                enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {}", e))?;
                let key = loop {
                    if let Event::Key(event) = read().map_err(|e| format!("failed to read event: {}", e))? {
                        if event.kind == KeyEventKind::Press {
                            break match event.code {
                                KeyCode::Up => ("Up", vec![]),
                                KeyCode::Down => ("Down", vec![]),
                                KeyCode::Left => ("Left", vec![]),
                                KeyCode::Right => ("Right", vec![]),
                                KeyCode::Enter => ("Enter", vec![]),
                                KeyCode::Esc => ("Esc", vec![]),
                                KeyCode::Char(c) => ("Char", vec![Value::String(c.to_string())]),
                                _ => continue,
                            };
                        }
                    }
                };
                disable_raw_mode().map_err(|e| format!("failed to disable raw mode: {}", e))?;
                Ok(Value::Enum("Key".to_string(), key.0.to_string(), key.1))
            }
            "poll_key" => {
                if !args.is_empty() {
                    return Err(format!("poll_key expects 0 args, got {}", args.len()));
                }
                Ok(Value::Enum("Key".to_string(), "None".to_string(), vec![]))
            }
            "now" => {
                if !args.is_empty() {
                    return Err(format!("now expects 0 args, got {}", args.len()));
                }
                let seconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs_f64();
                Ok(Value::Number(seconds))
            }
            "random_int" => {
                if args.len() != 2 {
                    return Err(format!("random_int expects 2 args, got {}", args.len()));
                }
                let min = args[0].as_number();
                let max = args[1].as_number();
                let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
                let range = (max - min).abs();
                if range == 0.0 { return Ok(Value::Number(min)); }
                let rand = (start % 1_000_000) as f64 / 1_000_000.0 * range;
                Ok(Value::Number((min + rand).floor()))
            }
            "random_float" => {
                if args.len() != 2 {
                    return Err(format!("random_float expects 2 args, got {}", args.len()));
                }
                let min = args[0].as_number();
                let max = args[1].as_number();
                let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
                let range = max - min;
                if range <= 0.0 { return Ok(Value::Number(min)); }
                let frac = (start % 1_000_000) as f64 / 1_000_000.0;
                Ok(Value::Number(min + frac * range))
            }
            "__range" | "__range_inclusive" => {
                if args.len() != 2 {
                    return Err(format!("{} expects 2 args, got {}", name, args.len()));
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
                        Ok(Value::List(Rc::new(RefCell::new(values))))
                    }
                    _ => Err(format!("{} expects numeric start and end", name)),
                }
            }
            "__map_keys" => {
                if args.len() != 1 {
                    return Err(format!("keys expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::Map(items) => {
                        let mut keys = items.borrow().keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        let values = keys.into_iter().map(Value::String).collect::<Vec<_>>();
                        Ok(Value::List(Rc::new(RefCell::new(values))))
                    }
                    _ => Err("keys() is only supported on maps".to_string()),
                }
            }
            "__map_values" => {
                if args.len() != 1 {
                    return Err(format!("values expects 1 arg, got {}", args.len()));
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
                        Ok(Value::List(Rc::new(RefCell::new(values))))
                    }
                    _ => Err("values() is only supported on maps".to_string()),
                }
            }
            "__map_entries" => {
                if args.len() != 1 {
                    return Err(format!("entries expects 1 arg, got {}", args.len()));
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
                        Ok(Value::List(Rc::new(RefCell::new(entries))))
                    }
                    _ => Err("entries() is only supported on maps".to_string()),
                }
            }
            "__map_has" => {
                if args.len() != 2 {
                    return Err(format!("has expects 2 args, got {}", args.len()));
                }
                match (&args[0], &args[1]) {
                    (Value::Map(items), Value::String(key)) => Ok(Value::Bool(items.borrow().contains_key(key))),
                    _ => Err("has() expects a map receiver and string key".to_string()),
                }
            }
            "dict" => {
                if args.len() % 2 != 0 {
                    return Err(format!("dict expects an even number of args, got {}", args.len()));
                }
                let mut items = HashMap::new();
                let mut idx = 0usize;
                while idx < args.len() {
                    let key = match &args[idx] {
                        Value::String(key) => key.clone(),
                        _ => return Err("dict expects string keys".to_string()),
                    };
                    items.insert(key, args[idx + 1].clone());
                    idx += 2;
                }
                Ok(Value::Map(Rc::new(RefCell::new(items))))
            }
            "json_encode" => {
                if args.len() != 1 {
                    return Err(format!("json_encode expects 1 arg, got {}", args.len()));
                }
                let encoded = lust_to_json_value(&args[0])?;
                let encoded = serde_json::to_string(&encoded)
                    .map_err(|e| format!("json_encode failed: {}", e))?;
                Ok(Value::String(encoded))
            }
            "json_decode" => {
                if args.len() != 1 {
                    return Err(format!("json_decode expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(text) => {
                        let decoded: serde_json::Value =
                            serde_json::from_str(text).map_err(|e| format!("json_decode failed: {}", e))?;
                        let pretty = serde_json::to_string_pretty(&decoded)
                            .map_err(|e| format!("json_decode formatting failed: {}", e))?;
                        Ok(Value::String(pretty))
                    }
                    _ => Err("json_decode expects a string argument".to_string()),
                }
            }
            "json_parse" => {
                if args.len() != 1 {
                    return Err(format!("json_parse expects 1 arg, got {}", args.len()));
                }
                match &args[0] {
                    Value::String(text) => {
                        let decoded: serde_json::Value =
                            serde_json::from_str(text).map_err(|e| format!("json_parse failed: {}", e))?;
                        Ok(json_to_lust_value(decoded))
                    }
                    _ => Err("json_parse expects a string argument".to_string()),
                }
            }
            "sleep" => {
                if args.len() != 1 {
                    return Err(format!("sleep expects 1 arg, got {}", args.len()));
                }
                match args[0] {
                    Value::Number(seconds) => {
                        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
                        Ok(Value::Null)
                    }
                    _ => Err("sleep expects a number argument".to_string()),
                }
            }
            "audio_init" => {
                self.require_import("audio", name)?;
                audio::audio_init_native()?;
                Ok(Value::Null)
            }
            "audio_set_freq" => {
                self.require_import("audio", name)?;
                if args.len() != 1 {
                    return Err(format!("audio_set_freq expects 1 arg, got {}", args.len()));
                }
                match args[0] {
                    Value::Number(freq) => {
                        audio::audio_set_freq_native(freq)?;
                        Ok(Value::Null)
                    }
                    _ => Err("audio_set_freq expects a number".to_string()),
                }
            }
            "audio_set_gain" => {
                self.require_import("audio", name)?;
                if args.len() != 1 {
                    return Err(format!("audio_set_gain expects 1 arg, got {}", args.len()));
                }
                match args[0] {
                    Value::Number(gain) => {
                        audio::audio_set_gain_native(gain)?;
                        Ok(Value::Null)
                    }
                    _ => Err("audio_set_gain expects a number".to_string()),
                }
            }
            "audio_note_on" => {
                self.require_import("audio", name)?;
                audio::audio_note_on_native()?;
                Ok(Value::Null)
            }
            "audio_note_off" => {
                self.require_import("audio", name)?;
                audio::audio_note_off_native()?;
                Ok(Value::Null)
            }
            "window" => {
                self.require_import("draw", name)?;
                let _ = args;
                Ok(Value::Null)
            }
            "live" => {
                self.require_import("draw", name)?;
                if !args.is_empty() {
                    return Err(format!("live expects 0 args, got {}", args.len()));
                }
                Ok(Value::Bool(false))
            }
            "clear_screen" | "circle" | "rect" | "line" | "triangle" | "text" => {
                self.require_import("draw", name)?;
                let _ = args;
                Ok(Value::Null)
            }
            _ => Err(format!("unknown builtin '{}'", name)),
        }
    }

    fn require_import(&self, module: &str, builtin: &str) -> Result<(), String> {
        if self.program.imports.contains(module) {
            Ok(())
        } else {
            Err(format!(
                "builtin '{}' requires `import \"{}\"` on the VM backend",
                builtin, module
            ))
        }
    }

    fn call_value_method(
        &mut self,
        target: &Value,
        name: &str,
        args: &[Value],
    ) -> Result<Option<Value>, String> {
        match target {
            Value::Struct(type_name, fields) if type_name == "FileHandle" => {
                let handle_id = match fields.borrow().get("id") {
                    Some(Value::Number(id)) => *id as u64,
                    _ => return Err("invalid FileHandle value".to_string()),
                };
                match name {
                    "write" => {
                        if args.len() != 1 {
                            return Err(format!("write expects 1 arg, got {}", args.len()));
                        }
                        let writer = self
                            .open_files
                            .get_mut(&handle_id)
                            .ok_or_else(|| format!("file handle {} is closed", handle_id))?;
                        match &args[0] {
                            Value::String(text) => writer
                                .write_all(text.as_bytes())
                                .map_err(|e| format!("file write failed: {}", e))?,
                            value => writer
                                .write_all(value.as_string().as_bytes())
                                .map_err(|e| format!("file write failed: {}", e))?,
                        }
                        Ok(Some(Value::Null))
                    }
                    "write_line" => {
                        if args.len() != 1 {
                            return Err(format!("write_line expects 1 arg, got {}", args.len()));
                        }
                        let writer = self
                            .open_files
                            .get_mut(&handle_id)
                            .ok_or_else(|| format!("file handle {} is closed", handle_id))?;
                        match &args[0] {
                            Value::String(text) => {
                                writer
                                    .write_all(text.as_bytes())
                                    .map_err(|e| format!("file write_line failed: {}", e))?;
                                writer
                                    .write_all(b"\n")
                                    .map_err(|e| format!("file write_line failed: {}", e))?;
                            }
                            value => {
                                writer
                                    .write_all(value.as_string().as_bytes())
                                    .map_err(|e| format!("file write_line failed: {}", e))?;
                                writer
                                    .write_all(b"\n")
                                    .map_err(|e| format!("file write_line failed: {}", e))?;
                            }
                        }
                        Ok(Some(Value::Null))
                    }
                    "close" => {
                        if !args.is_empty() {
                            return Err(format!("close expects 0 args, got {}", args.len()));
                        }
                        let mut writer = self
                            .open_files
                            .remove(&handle_id)
                            .ok_or_else(|| format!("file handle {} is closed", handle_id))?;
                        writer.flush().map_err(|e| format!("file close failed: {}", e))?;
                        Ok(Some(Value::Null))
                    }
                    _ => Ok(None),
                }
            }
            Value::Map(items) => match name {
                "set" => {
                    if args.len() != 2 {
                        return Err(format!("set expects 2 args, got {}", args.len()));
                    }
                    let key = match &args[0] {
                        Value::String(key) => key.clone(),
                        _ => return Err("set() expects a string key".to_string()),
                    };
                    items.borrow_mut().insert(key, args[1].clone());
                    Ok(Some(Value::Null))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn call_function(&mut self, name: &str, args: Vec<Value>) -> Result<(), String> {
        let function = self
            .program
            .functions
            .get(name)
            .ok_or_else(|| format!("unknown function '{}'", name))?;

        if function.arity != args.len() {
            return Err(format!(
                "function '{}' expects {} args but got {}",
                name,
                function.arity,
                args.len()
            ));
        }

        self.frames.push(CallFrame {
            function: name.to_string(),
            ip: 0,
            locals: args,
        });
        Ok(())
    }

    fn current_frame(&self) -> Result<&CallFrame, String> {
        self.frames.last().ok_or_else(|| "no active call frame".to_string())
    }

    fn current_frame_mut(&mut self) -> Result<&mut CallFrame, String> {
        self.frames.last_mut().ok_or_else(|| "no active call frame".to_string())
    }

    fn current_function(&self) -> Result<&Function, String> {
        let name = &self.current_frame()?.function;
        self.program
            .functions
            .get(name)
            .ok_or_else(|| format!("unknown function '{}'", name))
    }

    fn current_instruction(&self) -> Result<Instruction, String> {
        let frame = self.current_frame()?;
        let function = self.current_function()?;
        function
            .chunk
            .instructions
            .get(frame.ip)
            .cloned()
            .ok_or_else(|| format!("instruction pointer out of bounds in '{}'", frame.function))
    }

    fn pop(&mut self) -> Result<Value, String> {
        self.stack.pop().ok_or_else(|| "stack underflow".to_string())
    }

    fn peek(&self) -> Result<&Value, String> {
        self.stack.last().ok_or_else(|| "stack underflow".to_string())
    }

    fn binary_add(&mut self) -> Result<(), String> {
        let right = self.pop()?;
        let left = self.pop()?;
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => self.stack.push(Value::Number(a + b)),
            (Value::String(a), Value::String(b)) => {
                let mut combined = String::with_capacity(a.len() + b.len());
                combined.push_str(&a);
                combined.push_str(&b);
                self.stack.push(Value::String(combined));
            }
            (Value::String(a), b) => {
                let b = b.as_string();
                let mut combined = String::with_capacity(a.len() + b.len());
                combined.push_str(&a);
                combined.push_str(&b);
                self.stack.push(Value::String(combined));
            }
            (a, Value::String(b)) => {
                let a = a.as_string();
                let mut combined = String::with_capacity(a.len() + b.len());
                combined.push_str(&a);
                combined.push_str(&b);
                self.stack.push(Value::String(combined));
            }
            _ => return Err("invalid operands for '+'".to_string()),
        }
        Ok(())
    }

    fn binary_number<F>(&mut self, f: F) -> Result<(), String>
    where
        F: FnOnce(f64, f64) -> f64,
    {
        let right = self.pop()?;
        let left = self.pop()?;
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => {
                self.stack.push(Value::Number(f(a, b)));
                Ok(())
            }
            _ => Err("expected numeric operands".to_string()),
        }
    }

    fn binary_cmp_number<F>(&mut self, f: F) -> Result<(), String>
    where
        F: FnOnce(f64, f64) -> bool,
    {
        let right = self.pop()?;
        let left = self.pop()?;
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => {
                self.stack.push(Value::Bool(f(a, b)));
                Ok(())
            }
            _ => Err("expected numeric operands".to_string()),
        }
    }

    fn binary_compare<F>(&mut self, f: F) -> Result<(), String>
    where
        F: FnOnce(Value, Value) -> bool,
    {
        let right = self.pop()?;
        let left = self.pop()?;
        self.stack.push(Value::Bool(f(left, right)));
        Ok(())
    }
}

fn optional_slice_bound(value: &Value) -> Result<Option<usize>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) if *n >= 0.0 => Ok(Some(*n as usize)),
        Value::Number(_) => Err("slice bounds must be non-negative numbers".to_string()),
        _ => Err("slice bounds must be numbers or null".to_string()),
    }
}

fn compile_lustgex(pattern: &str) -> Result<String, String> {
    let parts = split_lustgex_whitespace(pattern);
    let mut result = String::new();
    let mut i = 0usize;

    while i < parts.len() {
        let part = &parts[i];
        if part == "then" {
            i += 1;
            continue;
        }

        let mut non_greedy = false;
        if part == "fewest" {
            non_greedy = true;
            i += 1;
            if i >= parts.len() {
                break;
            }
        }

        let (fragment, consumed) = lustgex_token_fragment(&parts, i, non_greedy)?;
        let mut consumed_total = consumed;

        if i + consumed_total < parts.len() && parts[i + consumed_total] == "as" {
            if i + consumed_total + 1 >= parts.len() {
                return Err("lustgex 'as' must be followed by a capture name".to_string());
            }
            let capture_name = &parts[i + consumed_total + 1];
            result.push_str("(?P<");
            result.push_str(capture_name);
            result.push('>');
            result.push_str(&fragment);
            result.push(')');
            consumed_total += 2;
        } else {
            result.push_str(&fragment);
        }

        i += consumed_total;
    }

    Ok(result)
}

fn extract_lustgex_capture_names(pattern: &str) -> Vec<String> {
    let parts = split_lustgex_whitespace(pattern);
    let mut names = Vec::new();
    let mut i = 0usize;

    while i < parts.len() {
        let part = &parts[i];
        if part == "then" {
            i += 1;
            continue;
        }

        if part == "fewest" {
            i += 1;
            if i >= parts.len() {
                break;
            }
        }

        let consumed = lustgex_token_fragment(&parts, i, false)
            .map(|(_, consumed)| consumed)
            .unwrap_or(1);
        if i + consumed < parts.len() && parts[i + consumed] == "as" {
            if let Some(name) = parts.get(i + consumed + 1) {
                names.push(name.clone());
                i += consumed + 2;
                continue;
            }
        }

        i += consumed;
    }

    names
}

fn split_lustgex_whitespace(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn parse_lustgex_usize(text: &str) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0usize;
    for ch in text.chars() {
        let digit = ch.to_digit(10)? as usize;
        value = value.saturating_mul(10).saturating_add(digit);
    }
    Some(value)
}

fn lustgex_escape_regex(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn lustgex_simple_fragment(part: &str, non_greedy: bool) -> String {
    let fragment = match part {
        "letters" => "[A-Za-z]+",
        "letter" => "[A-Za-z]",
        "digits" | "integer" => "\\d+",
        "digit" => "\\d",
        "spaces" | "blanks" => "\\s+",
        "space" | "blank" => "\\s",
        "anything" => ".*",
        "maybe" => "?",
        "start" => "^",
        "end" => "$",
        "" => "",
        other => return lustgex_escape_regex(other),
    };

    if non_greedy && matches!(part, "letters" | "digits" | "integer" | "spaces" | "blanks" | "anything") {
        format!("{}?", fragment)
    } else {
        fragment.to_string()
    }
}

fn lustgex_token_fragment(parts: &[String], index: usize, non_greedy: bool) -> Result<(String, usize), String> {
    let current = &parts[index];

    if current.len() >= 2 && current.starts_with('"') && current.ends_with('"') {
        let lit = &current[1..current.len() - 1];
        let fragment = lustgex_escape_regex(lit);
        if non_greedy {
            return Ok((format!(".*?{}", fragment), 1));
        }
        return Ok((fragment, 1));
    }

    if let Some(num) = parse_lustgex_usize(current) {
        if let Some(next) = parts.get(index + 1) {
            let fragment = match next.as_str() {
                "digits" | "digit" => Some(format!("\\d{{{}}}", num)),
                "letters" | "letter" => Some(format!("[A-Za-z]{{{}}}", num)),
                "spaces" | "space" | "blanks" | "blank" => Some(format!("\\s{{{}}}", num)),
                _ => None,
            };
            if let Some(fragment) = fragment {
                return if non_greedy {
                    Ok((format!("{}?", fragment), 2))
                } else {
                    Ok((fragment, 2))
                };
            }
        }
    }

    Ok((lustgex_simple_fragment(current, non_greedy), 1))
}

fn json_to_lust_value(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(v) => Value::Bool(v),
        serde_json::Value::Number(v) => Value::Number(v.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(v) => Value::String(v),
        serde_json::Value::Array(items) => Value::List(Rc::new(RefCell::new(
            items.into_iter().map(json_to_lust_value).collect(),
        ))),
        serde_json::Value::Object(items) => Value::Map(Rc::new(RefCell::new(
            items
                .into_iter()
                .map(|(key, value)| (key, json_to_lust_value(value)))
                .collect(),
        ))),
    }
}

fn lust_to_json_value(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Bool(v) => Ok(serde_json::Value::Bool(*v)),
        Value::Number(v) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!("json_encode cannot encode non-finite number {}", v)),
        Value::String(v) => Ok(serde_json::Value::String(v.clone())),
        Value::List(items) => {
            let mut values = Vec::new();
            for item in items.borrow().iter() {
                values.push(lust_to_json_value(item)?);
            }
            Ok(serde_json::Value::Array(values))
        }
        Value::Map(items) => {
            let items = items.borrow();
            let mut keys = items.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut object = serde_json::Map::new();
            for key in keys {
                let value = items.get(&key).expect("key should exist");
                object.insert(key, lust_to_json_value(value)?);
            }
            Ok(serde_json::Value::Object(object))
        }
        Value::Struct(_, fields) => {
            let fields = fields.borrow();
            let mut keys = fields.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut object = serde_json::Map::new();
            for key in keys {
                let value = fields.get(&key).expect("field should exist");
                object.insert(key, lust_to_json_value(value)?);
            }
            Ok(serde_json::Value::Object(object))
        }
        Value::RegexCapture(capture) => {
            let mut entries = capture
                .field_slots
                .iter()
                .map(|(name, slot)| (name.clone(), *slot))
                .collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut object = serde_json::Map::new();
            for (key, slot) in entries {
                let value = capture
                    .fields
                    .get(slot)
                    .and_then(|value| value.as_ref())
                    .map(|value| serde_json::Value::String(value.clone()))
                    .unwrap_or(serde_json::Value::Null);
                object.insert(key, value);
            }
            Ok(serde_json::Value::Object(object))
        }
        Value::Enum(owner, variant, values) => {
            if values.is_empty() {
                Ok(serde_json::Value::String(format!("{}.{}", owner, variant)))
            } else {
                let mut object = serde_json::Map::new();
                object.insert(
                    "enum".to_string(),
                    serde_json::Value::String(owner.clone()),
                );
                object.insert(
                    "variant".to_string(),
                    serde_json::Value::String(variant.clone()),
                );
                let mut payload = Vec::new();
                for value in values {
                    payload.push(lust_to_json_value(value)?);
                }
                object.insert("values".to_string(), serde_json::Value::Array(payload));
                Ok(serde_json::Value::Object(object))
            }
        }
        Value::Function(name) => Err(format!("json_encode cannot encode function {}", name)),
    }
}

fn instruction_name(instruction: &Instruction) -> &'static str {
    match instruction {
        Instruction::Constant(_) => "Constant",
        Instruction::LoadGlobal(_) => "LoadGlobal",
        Instruction::StoreGlobal(_) => "StoreGlobal",
        Instruction::LoadLocal(_) => "LoadLocal",
        Instruction::StoreLocal(_) => "StoreLocal",
        Instruction::BuildList(_) => "BuildList",
        Instruction::BuildStruct(_, _) => "BuildStruct",
        Instruction::BuildEnum(_, _, _) => "BuildEnum",
        Instruction::IndexGet => "IndexGet",
        Instruction::IndexSet => "IndexSet",
        Instruction::MemberGet(_) => "MemberGet",
        Instruction::MemberSet(_) => "MemberSet",
        Instruction::Add => "Add",
        Instruction::Sub => "Sub",
        Instruction::Mul => "Mul",
        Instruction::Div => "Div",
        Instruction::Mod => "Mod",
        Instruction::Eq => "Eq",
        Instruction::Ne => "Ne",
        Instruction::Gt => "Gt",
        Instruction::Ge => "Ge",
        Instruction::Lt => "Lt",
        Instruction::Le => "Le",
        Instruction::And => "And",
        Instruction::Or => "Or",
        Instruction::Not => "Not",
        Instruction::Call(_, _) => "Call",
        Instruction::CallMethod(_, _) => "CallMethod",
        Instruction::CallBuiltin(_, _) => "CallBuiltin",
        Instruction::CallDynamic(_) => "CallDynamic",
        Instruction::LoadFunction(_) => "LoadFunction",
        Instruction::ListPush => "ListPush",
        Instruction::ListLen => "ListLen",
        Instruction::IsList => "IsList",
        Instruction::StructIsType(_) => "StructIsType",
        Instruction::StructGetField(_) => "StructGetField",
        Instruction::EnumIsVariant(_, _) => "EnumIsVariant",
        Instruction::EnumGetField(_) => "EnumGetField",
        Instruction::Return => "Return",
        Instruction::Print(_) => "Print",
        Instruction::Pop => "Pop",
        Instruction::JumpIfFalse(_) => "JumpIfFalse",
        Instruction::Jump(_) => "Jump",
        Instruction::Halt => "Halt",
    }
}
