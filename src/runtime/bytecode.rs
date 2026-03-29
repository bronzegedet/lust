use std::collections::{HashMap, HashSet};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct RegexCaptureValue {
    pub field_slots: Rc<HashMap<String, usize>>,
    pub fields: Vec<Option<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<HashMap<String, Value>>>),
    Struct(String, Rc<RefCell<HashMap<String, Value>>>),
    RegexCapture(Rc<RegexCaptureValue>),
    Enum(String, String, Vec<Value>),
    Function(String),
    Null,
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(v) => *v,
            Value::Null => false,
            Value::Number(v) => *v != 0.0,
            Value::String(v) => !v.is_empty(),
            Value::List(v) => !v.borrow().is_empty(),
            Value::Map(v) => !v.borrow().is_empty(),
            Value::Struct(_, _) | Value::RegexCapture(_) => true,
            Value::Enum(_, _, _) => true,
            Value::Function(_) => true,
        }
    }

    pub fn as_number(&self) -> f64 {
        match self {
            Value::Number(v) => *v,
            Value::Bool(v) => if *v { 1.0 } else { 0.0 },
            Value::String(s) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Value::Number(v) => {
                if v.fract() == 0.0 {
                    format!("{:.0}", v)
                } else {
                    v.to_string()
                }
            }
            Value::String(v) => v.clone(),
            Value::Bool(v) => v.to_string(),
            Value::Null => "null".to_string(),
            Value::List(_) | Value::Map(_) | Value::Struct(_, _) | Value::RegexCapture(_) | Value::Enum(_, _, _) | Value::Function(_) => {
                self.to_string()
            }
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            Value::Number(_) => "Number".to_string(),
            Value::String(_) => "String".to_string(),
            Value::Bool(_) => "Boolean".to_string(),
            Value::List(_) => "List".to_string(),
            Value::Map(_) => "Map".to_string(),
            Value::Struct(name, _) => format!("Struct:{}", name),
            Value::RegexCapture(_) => "Struct:RegexCapture".to_string(),
            Value::Enum(owner, variant, _) => format!("Enum:{}.{}", owner, variant),
            Value::Function(name) => format!("Function:{}", name),
            Value::Null => "Null".to_string(),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(v) => {
                if v.fract() == 0.0 {
                    write!(f, "{:.0}", v)
                } else {
                    write!(f, "{}", v)
                }
            }
            Value::String(v) => write!(f, "{}", v),
            Value::Bool(v) => write!(f, "{}", v),
            Value::List(v) => {
                let parts: Vec<String> = v.borrow().iter().map(|item| item.to_string()).collect();
                write!(f, "[{}]", parts.join(", "))
            }
            Value::Map(items) => {
                let items = items.borrow();
                let mut keys = items.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                let parts = keys
                    .into_iter()
                    .map(|key| format!("\"{}\": {}", key, items.get(&key).unwrap()))
                    .collect::<Vec<_>>();
                write!(f, "{{{}}}", parts.join(", "))
            }
            Value::Struct(name, fields) => {
                let fields = fields.borrow();
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(field, value)| format!("{}: {}", field, value))
                    .collect();
                write!(f, "{} {{ {} }}", name, parts.join(", "))
            }
            Value::RegexCapture(capture) => {
                let mut entries = capture
                    .field_slots
                    .iter()
                    .map(|(name, slot)| (name.as_str(), *slot))
                    .collect::<Vec<_>>();
                entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
                let parts = entries
                    .into_iter()
                    .map(|(name, slot)| match capture.fields.get(slot).and_then(|value| value.as_ref()) {
                        Some(value) => format!("{}: {}", name, value),
                        None => format!("{}: null", name),
                    })
                    .collect::<Vec<_>>();
                write!(f, "RegexCapture {{ {} }}", parts.join(", "))
            }
            Value::Enum(_, variant, values) => {
                if values.is_empty() {
                    write!(f, "{}", variant)
                } else {
                    let parts: Vec<String> = values.iter().map(|value| value.to_string()).collect();
                    write!(f, "{}({})", variant, parts.join(", "))
                }
            }
            Value::Function(name) => write!(f, "<function {}>", name),
            Value::Null => write!(f, "null"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Constant(usize),
    LoadGlobal(String),
    StoreGlobal(String),
    LoadLocal(usize),
    StoreLocal(usize),
    BuildList(usize),
    BuildStruct(String, Vec<String>),
    BuildEnum(String, String, usize),
    IndexGet,
    IndexSet,
    MemberGet(String),
    MemberSet(String),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    And,
    Or,
    Not,
    Call(String, usize),
    CallMethod(String, usize),
    CallBuiltin(String, usize),
    CallDynamic(usize),
    LoadFunction(String),
    ListPush,
    ListLen,
    IsList,
    StructIsType(String),
    StructGetField(String),
    EnumIsVariant(String, String),
    EnumGetField(usize),
    Return,
    Print(usize),
    Pop,
    JumpIfFalse(usize),
    Jump(usize),
    Halt,
}

#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn add_constant(&mut self, value: Value) -> usize {
        self.constants.push(value);
        self.constants.len() - 1
    }

    pub fn emit(&mut self, instruction: Instruction) -> usize {
        self.instructions.push(instruction);
        self.instructions.len() - 1
    }

    pub fn patch_jump(&mut self, at: usize, target: usize) {
        match self.instructions.get_mut(at) {
            Some(Instruction::JumpIfFalse(slot)) | Some(Instruction::Jump(slot)) => {
                *slot = target;
            }
            _ => panic!("attempted to patch non-jump instruction"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    pub arity: usize,
    pub chunk: Chunk,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub entry: String,
    pub functions: HashMap<String, Function>,
    pub struct_defs: HashMap<String, Vec<String>>,
    pub imports: HashSet<String>,
}
