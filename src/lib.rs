pub mod frontend {
    pub mod ast;
    pub mod lexer;
    pub mod parser;
    pub mod token;
    pub mod typecheck;
}

pub mod runtime {
    pub mod bytecode;
    pub mod bytecode_compiler;
    pub mod modules;
    pub mod vm;
    pub mod vm_memory;
}

pub mod semantics {
    pub mod access;
    pub mod control_flow;
    pub mod dispatch;
    pub mod expressions;
    pub mod helpers;
    pub mod lowered;
    pub mod patterns;
    pub mod statements;
}

pub use frontend::{ast, lexer, parser, token, typecheck};
pub use runtime::{bytecode, bytecode_compiler, modules, vm, vm_memory};
pub use semantics::{access, control_flow, dispatch, expressions, helpers, lowered, patterns, statements};

#[cfg(test)]
mod compiler_tests_selfhost;
