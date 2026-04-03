#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    StringLit(String),
    Ident(String),
    Self_,
    Binary(Box<Expr>, String, Box<Expr>),
    Call(String, Vec<Expr>),
    Lambda(Vec<String>, Box<Expr>),
    Pipe(Box<Expr>, String, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    Member(Box<Expr>, String),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    List(Vec<Expr>),
    MapLit(Vec<(Expr, Expr)>),
    StructInst(String, Vec<(String, Expr)>, Option<Box<Expr>>),
    EnumVariant(String, Vec<Expr>),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Bind(String),
    Number(f64),
    StringLit(String),
    Bool(bool),
    Null,
    List(Vec<Pattern>, bool),
    Struct(String, Vec<(String, Pattern)>),
    EnumVariant(String, Vec<Pattern>),
}

#[derive(Debug, Clone)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(usize, String, Option<String>, Expr),
    LetPattern(usize, Pattern, Expr),
    Assign(usize, Expr, Expr), // Add support for assignment (e.g. self.x = 1)
    Pass(usize),
    Return(usize, Expr),
    Break(usize),
    Continue(usize),
    Print(usize, Vec<Expr>),
    If(usize, Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    Match(usize, Expr, Vec<MatchCase>),
    While(usize, Expr, Vec<Stmt>),
    For(usize, Option<String>, String, Expr, Vec<Stmt>),
    ExprStmt(usize, Expr),
    Spawn(usize, String, Vec<Expr>),
}

impl Stmt {
    pub fn line(&self) -> usize {
        match self {
            Stmt::Let(line, _, _, _)
            | Stmt::LetPattern(line, _, _)
            | Stmt::Assign(line, _, _)
            | Stmt::Pass(line)
            | Stmt::Return(line, _)
            | Stmt::Break(line)
            | Stmt::Continue(line)
            | Stmt::Print(line, _)
            | Stmt::If(line, _, _, _)
            | Stmt::Match(line, _, _)
            | Stmt::While(line, _, _)
            | Stmt::For(line, _, _, _, _)
            | Stmt::ExprStmt(line, _)
            | Stmt::Spawn(line, _, _) => *line,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Decl {
    Fn(String, Option<String>, Vec<(String, Option<String>)>, Option<String>, Vec<Stmt>), // name, target_type, args, ret_type, body
    Type(String, Vec<(String, Option<String>)>),
    Enum(String, Vec<(String, Vec<String>)>),
    Import(String),
    Stmt(Stmt),
}
