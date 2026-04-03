use std::collections::HashMap;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::rc::Rc;
use crossterm::event::KeyCode;
use regex::Regex;
use super::vm_memory::{VmMemoryBudget, VmMemoryStats};
pub use super::vm_memory::VmMemorySnapshot;

mod audio_builtins;
mod collection_builtins;
mod core_builtins;
mod data_builtins;
mod draw_builtins;
mod io_builtins;
mod regex_builtins;
mod text_builtins;
mod ui;

use crate::bytecode::{Function, Instruction, Program, Value};
use crate::modules::draw;

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
    draw_runtime: Option<draw::DrawRuntime>,
    ui_state: HashMap<String, Value>,
    ui_button_latches: HashMap<String, bool>,
    ui_key_buffer: Option<draw::KeyToken>,
    ui_mouse_x: f64,
    ui_mouse_y: f64,
    ui_mouse_down: bool,
    ui_mouse_clicked: bool,
    ui_mouse_click_x: f64,
    ui_mouse_click_y: f64,
    trace_enabled: bool,
    trace_events: Vec<String>,
    memory_budget: VmMemoryBudget,
    memory_stats: VmMemoryStats,
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
            draw_runtime: None,
            ui_state: HashMap::new(),
            ui_button_latches: HashMap::new(),
            ui_key_buffer: None,
            ui_mouse_x: 0.0,
            ui_mouse_y: 0.0,
            ui_mouse_down: false,
            ui_mouse_clicked: false,
            ui_mouse_click_x: 0.0,
            ui_mouse_click_y: 0.0,
            trace_enabled: false,
            trace_events: Vec::new(),
            memory_budget: VmMemoryBudget::from_env(),
            memory_stats: VmMemoryStats::default(),
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
            self.enforce_memory_budget()?;
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
                if !self.globals.contains_key(&name) {
                    self.ensure_globals_len(self.globals.len().saturating_add(1))?;
                }
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
                self.ensure_list_len(count, "list literal")?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.pop()?);
                }
                items.reverse();
                self.memory_stats.list_allocations = self.memory_stats.list_allocations.saturating_add(1);
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
                self.ensure_map_len(fields.len(), "struct literal")?;
                for field in fields.iter().rev() {
                    let value = self.pop()?;
                    if !expected_fields.contains(field) {
                        return Err(format!("unknown field '{}' for struct '{}'", field, name));
                    }
                    items.insert(field.clone(), value);
                }
                self.memory_stats.struct_allocations =
                    self.memory_stats.struct_allocations.saturating_add(1);
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
                        {
                            let mut map = items.borrow_mut();
                            if !map.contains_key(&key) {
                                self.ensure_map_len(
                                    map.len().saturating_add(1),
                                    "map index assignment",
                                )?;
                                self.memory_stats.map_insert_ops =
                                    self.memory_stats.map_insert_ops.saturating_add(1);
                            }
                            map.insert(key, value);
                        }
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

                    self.ensure_list_len(results.len(), "map/filter result")?;
                    self.memory_stats.list_allocations =
                        self.memory_stats.list_allocations.saturating_add(1);
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

                    self.ensure_map_len(results.len(), "map/filter map result")?;
                    self.memory_stats.map_allocations =
                        self.memory_stats.map_allocations.saturating_add(1);
                    self.stack.push(Value::Map(Rc::new(RefCell::new(results))));
                    return Ok(());
                }

                let value = self.call_builtin(&name, args)?;
                self.validate_value_memory(&value)?;
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
                        {
                            let mut list = items.borrow_mut();
                            self.ensure_list_len(list.len().saturating_add(1), "push() on list")?;
                            list.push(value);
                        }
                        self.memory_stats.list_push_ops =
                            self.memory_stats.list_push_ops.saturating_add(1);
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

    pub fn ui_state_snapshot(&self) -> HashMap<String, Value> {
        self.ui_state.clone()
    }

    pub fn restore_ui_state(&mut self, state: HashMap<String, Value>) -> Result<(), String> {
        self.ensure_ui_state_len(state.len())?;
        self.ui_button_latches.clear();
        for (key, value) in &state {
            if key.starts_with("button.") {
                self.ui_button_latches.insert(key.clone(), value.truthy());
            }
        }
        self.ui_state = state;
        self.enforce_memory_budget()?;
        Ok(())
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
    }

    pub fn trace_events_snapshot(&self) -> Vec<String> {
        self.trace_events.clone()
    }

    pub fn memory_snapshot(&self) -> VmMemorySnapshot {
        VmMemorySnapshot {
            stack_len: self.stack.len(),
            stack_peak: self.memory_stats.peak_stack,
            globals_len: self.globals.len(),
            globals_peak: self.memory_stats.peak_globals,
            ui_state_len: self.ui_state.len(),
            ui_state_peak: self.memory_stats.peak_ui_state_entries,
            trace_events_len: self.trace_events.len(),
            list_allocations: self.memory_stats.list_allocations,
            map_allocations: self.memory_stats.map_allocations,
            struct_allocations: self.memory_stats.struct_allocations,
            list_push_ops: self.memory_stats.list_push_ops,
            map_insert_ops: self.memory_stats.map_insert_ops,
            max_stack: self.memory_budget.max_stack,
            max_globals: self.memory_budget.max_globals,
            max_ui_state_entries: self.memory_budget.max_ui_state_entries,
            max_trace_events: self.memory_budget.max_trace_events,
            max_list_len: self.memory_budget.max_list_len,
            max_map_len: self.memory_budget.max_map_len,
        }
    }

    fn enforce_memory_budget(&mut self) -> Result<(), String> {
        self.memory_stats.peak_stack = self.memory_stats.peak_stack.max(self.stack.len());
        self.memory_stats.peak_globals = self.memory_stats.peak_globals.max(self.globals.len());
        self.memory_stats.peak_ui_state_entries = self
            .memory_stats
            .peak_ui_state_entries
            .max(self.ui_state.len());

        if self.stack.len() > self.memory_budget.max_stack {
            return Err(format!(
                "vm memory guard: stack size {} exceeded limit {} (LUST_VM_MAX_STACK)",
                self.stack.len(),
                self.memory_budget.max_stack
            ));
        }
        if self.globals.len() > self.memory_budget.max_globals {
            return Err(format!(
                "vm memory guard: globals size {} exceeded limit {} (LUST_VM_MAX_GLOBALS)",
                self.globals.len(),
                self.memory_budget.max_globals
            ));
        }
        if self.ui_state.len() > self.memory_budget.max_ui_state_entries {
            return Err(format!(
                "vm memory guard: ui_state size {} exceeded limit {} (LUST_VM_MAX_UI_STATE)",
                self.ui_state.len(),
                self.memory_budget.max_ui_state_entries
            ));
        }
        if self.trace_events.len() > self.memory_budget.max_trace_events {
            return Err(format!(
                "vm memory guard: trace events {} exceeded limit {} (LUST_VM_MAX_TRACE_EVENTS)",
                self.trace_events.len(),
                self.memory_budget.max_trace_events
            ));
        }
        Ok(())
    }

    fn ensure_globals_len(&self, len: usize) -> Result<(), String> {
        if len > self.memory_budget.max_globals {
            return Err(format!(
                "vm memory guard: globals size {} exceeded limit {} (LUST_VM_MAX_GLOBALS)",
                len, self.memory_budget.max_globals
            ));
        }
        Ok(())
    }

    fn ensure_ui_state_len(&self, len: usize) -> Result<(), String> {
        if len > self.memory_budget.max_ui_state_entries {
            return Err(format!(
                "vm memory guard: ui_state size {} exceeded limit {} (LUST_VM_MAX_UI_STATE)",
                len, self.memory_budget.max_ui_state_entries
            ));
        }
        Ok(())
    }

    fn ensure_list_len(&self, len: usize, context: &str) -> Result<(), String> {
        if len > self.memory_budget.max_list_len {
            return Err(format!(
                "vm memory guard: list length {} exceeded limit {} in {} (LUST_VM_MAX_LIST_LEN)",
                len, self.memory_budget.max_list_len, context
            ));
        }
        Ok(())
    }

    fn ensure_map_len(&self, len: usize, context: &str) -> Result<(), String> {
        if len > self.memory_budget.max_map_len {
            return Err(format!(
                "vm memory guard: map/struct size {} exceeded limit {} in {} (LUST_VM_MAX_MAP_LEN)",
                len, self.memory_budget.max_map_len, context
            ));
        }
        Ok(())
    }

    fn validate_value_memory(&mut self, value: &Value) -> Result<(), String> {
        match value {
            Value::List(items) => {
                let len = items.borrow().len();
                self.ensure_list_len(len, "builtin return list")?;
                self.memory_stats.list_allocations =
                    self.memory_stats.list_allocations.saturating_add(1);
            }
            Value::Map(items) => {
                let len = items.borrow().len();
                self.ensure_map_len(len, "builtin return map")?;
                self.memory_stats.map_allocations =
                    self.memory_stats.map_allocations.saturating_add(1);
            }
            Value::Struct(_, fields) => {
                let len = fields.borrow().len();
                self.ensure_map_len(len, "builtin return struct")?;
                self.memory_stats.struct_allocations =
                    self.memory_stats.struct_allocations.saturating_add(1);
            }
            _ => {}
        }
        Ok(())
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
        if let Some(result) = self.call_ui_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_audio_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_draw_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_io_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_data_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_text_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_collection_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_regex_builtin(name, &args) {
            return result;
        }
        if let Some(result) = self.call_core_builtin(name, &args) {
            return result;
        }
        match name {
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

    fn consume_queued_input_event(&mut self) -> Option<draw::KeyToken> {
        if let Some(runtime) = self.draw_runtime.as_mut() {
            match runtime.take_pending_key() {
                Ok(Some(token)) => return Some(token),
                Ok(None) => {}
                Err(_) => {}
            }
        }
        if !self.key_inputs.is_empty() {
            let raw = self.key_inputs.remove(0);
            return Some(Self::decode_scripted_key(raw));
        }
        None
    }

    fn consume_queued_key(&mut self) -> Option<draw::KeyToken> {
        loop {
            let token = self.consume_queued_input_event()?;
            if self.process_pointer_token(&token) {
                continue;
            }
            return Some(token);
        }
    }

    fn poll_pointer_events(&mut self) {
        if self.ui_key_buffer.is_some() {
            return;
        }
        loop {
            let Some(token) = self.consume_queued_input_event() else {
                break;
            };
            if self.process_pointer_token(&token) {
                continue;
            }
            self.ui_key_buffer = Some(token);
            break;
        }
    }

    fn process_pointer_token(&mut self, token: &draw::KeyToken) -> bool {
        match token.variant.as_str() {
            "MouseMove" | "MouseDrag" => {
                if let Some((x, y)) = parse_mouse_xy(&token.payload) {
                    self.ui_mouse_x = x;
                    self.ui_mouse_y = y;
                }
                true
            }
            "MouseDown" => {
                if let Some((x, y)) = parse_mouse_xy(&token.payload) {
                    self.ui_mouse_x = x;
                    self.ui_mouse_y = y;
                    self.ui_mouse_click_x = x;
                    self.ui_mouse_click_y = y;
                }
                self.ui_mouse_down = true;
                self.ui_mouse_clicked = true;
                true
            }
            "MouseUp" => {
                if let Some((x, y)) = parse_mouse_xy(&token.payload) {
                    self.ui_mouse_x = x;
                    self.ui_mouse_y = y;
                }
                self.ui_mouse_down = false;
                true
            }
            "MouseScrollUp" | "MouseScrollDown" | "MouseScrollLeft" | "MouseScrollRight" => {
                if let Some((x, y)) = parse_mouse_xy(&token.payload) {
                    self.ui_mouse_x = x;
                    self.ui_mouse_y = y;
                }
                true
            }
            _ => false,
        }
    }

    fn decode_scripted_key(raw: String) -> draw::KeyToken {
        let trimmed = raw.trim().to_lowercase();
        if let Some(payload) = trimmed.strip_prefix("mouse_move:") {
            return scripted_mouse_token("MouseMove", payload);
        }
        if let Some(payload) = trimmed.strip_prefix("mouse_down:") {
            return scripted_mouse_token("MouseDown", payload);
        }
        if let Some(payload) = trimmed.strip_prefix("mouse_up:") {
            return scripted_mouse_token("MouseUp", payload);
        }
        if let Some(payload) = trimmed.strip_prefix("mouse_drag:") {
            return scripted_mouse_token("MouseDrag", payload);
        }
        match trimmed.as_str() {
            "up" => draw::KeyToken {
                variant: "Up".to_string(),
                payload: Vec::new(),
            },
            "down" => draw::KeyToken {
                variant: "Down".to_string(),
                payload: Vec::new(),
            },
            "left" => draw::KeyToken {
                variant: "Left".to_string(),
                payload: Vec::new(),
            },
            "right" => draw::KeyToken {
                variant: "Right".to_string(),
                payload: Vec::new(),
            },
            "enter" => draw::KeyToken {
                variant: "Enter".to_string(),
                payload: Vec::new(),
            },
            "backspace" => draw::KeyToken {
                variant: "Backspace".to_string(),
                payload: Vec::new(),
            },
            "delete" => draw::KeyToken {
                variant: "Delete".to_string(),
                payload: Vec::new(),
            },
            "esc" => draw::KeyToken {
                variant: "Esc".to_string(),
                payload: Vec::new(),
            },
            _ => {
                let mut chars = trimmed.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => draw::KeyToken {
                        variant: "Char".to_string(),
                        payload: vec![ch.to_string()],
                    },
                    _ => draw::KeyToken {
                        variant: "None".to_string(),
                        payload: Vec::new(),
                    },
                }
            }
        }
    }

    fn decode_key_event(code: KeyCode) -> Option<draw::KeyToken> {
        match code {
            KeyCode::Up => Some(draw::KeyToken {
                variant: "Up".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Down => Some(draw::KeyToken {
                variant: "Down".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Left => Some(draw::KeyToken {
                variant: "Left".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Right => Some(draw::KeyToken {
                variant: "Right".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Enter => Some(draw::KeyToken {
                variant: "Enter".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Backspace => Some(draw::KeyToken {
                variant: "Backspace".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Delete => Some(draw::KeyToken {
                variant: "Delete".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Esc => Some(draw::KeyToken {
                variant: "Esc".to_string(),
                payload: Vec::new(),
            }),
            KeyCode::Char(c) => Some(draw::KeyToken {
                variant: "Char".to_string(),
                payload: vec![c.to_string()],
            }),
            _ => None,
        }
    }

    fn key_value(variant: &str, payload: Vec<String>) -> Value {
        Value::Enum(
            "Key".to_string(),
            variant.to_string(),
            payload.into_iter().map(Value::String).collect(),
        )
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
                    {
                        let mut map = items.borrow_mut();
                        if !map.contains_key(&key) {
                            self.ensure_map_len(map.len().saturating_add(1), "map.set()")?;
                            self.memory_stats.map_insert_ops =
                                self.memory_stats.map_insert_ops.saturating_add(1);
                        }
                        map.insert(key, args[1].clone());
                    }
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

fn parse_mouse_xy(payload: &[String]) -> Option<(f64, f64)> {
    let x = payload.first()?.parse::<f64>().ok()?;
    let y = payload.get(1)?.parse::<f64>().ok()?;
    Some((x.max(0.0).floor(), y.max(0.0).floor()))
}

fn scripted_mouse_token(variant: &str, payload_text: &str) -> draw::KeyToken {
    let mut parts = payload_text.split(':');
    let x = parts.next().unwrap_or("0").trim();
    let y = parts.next().unwrap_or("0").trim();
    let mut payload = vec![x.to_string(), y.to_string()];
    if let Some(button) = parts.next() {
        payload.push(button.trim().to_string());
    }
    draw::KeyToken {
        variant: variant.to_string(),
        payload,
    }
}

pub(super) fn optional_slice_bound(value: &Value) -> Result<Option<usize>, String> {
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
