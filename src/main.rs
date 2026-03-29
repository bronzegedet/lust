use lust::lexer::Lexer;
use lust::parser::Parser;
#[cfg(test)]
use lust::token::Token;
use lust::typecheck::TypeChecker;
use lust::ast::Decl;
use lust::bytecode_compiler::BytecodeCompiler;
use lust::vm::Vm;
use std::env;
use std::fs;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use clap::{Parser as ClapParser, Subcommand};
use colored::*;

#[derive(ClapParser)]
#[command(name = "lust")]
#[command(about = "The Lust Programming Language CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(alias = "vm")]
    /// Run a .lust file on the VM backend
    Run {
        /// Path to the .lust file
        path: String,
        /// Additional arguments for the script
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run all tests in the /tests folder
    Test,
    /// Stress test with a fuzzed directory
    Fuzz {
        /// Path to the fuzzed directory
        path: String,
    },
}

fn parse_program(lust_code: &str) -> Result<Vec<Decl>, Vec<String>> {
    let mut lexer = Lexer::new(lust_code);
    let mut tokens = Vec::new();
    while let Some(t) = lexer.next_token() {
        tokens.push(t);
    }

    let mut parser = Parser::new(tokens);
    let decls = parser.parse();

    if !parser.errors.is_empty() {
        return Err(parser.errors);
    }

    Ok(decls)
}

fn parse_program_with_std_imports(
    lust_code: &str,
    base_dir: Option<&Path>,
    visited_std_modules: &mut HashSet<PathBuf>,
) -> Result<Vec<Decl>, Vec<String>> {
    let decls = parse_program(lust_code)?;
    expand_imports(&decls, base_dir, visited_std_modules)
}

fn std_module_path(name: &str) -> Option<PathBuf> {
    let rest = name.strip_prefix("std/")?;
    let module_suffix = PathBuf::from("lust_src")
        .join("std")
        .join(format!("{}.lust", rest));

    for root in std_search_roots() {
        let candidate = root.join(&module_suffix);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(module_suffix))
}

fn std_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = env::var_os("LUST_HOME") {
        roots.push(PathBuf::from(home));
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
            if let Some(grandparent) = parent.parent() {
                roots.push(grandparent.to_path_buf());
            }
        }
    }

    if let Ok(cwd) = env::current_dir() {
        roots.push(cwd);
    }

    let mut deduped = Vec::new();
    for root in roots {
        if !deduped.iter().any(|existing: &PathBuf| existing == &root) {
            deduped.push(root);
        }
    }
    deduped
}

fn expand_imports(
    decls: &[Decl],
    base_dir: Option<&Path>,
    visited_std_modules: &mut HashSet<PathBuf>,
) -> Result<Vec<Decl>, Vec<String>> {
    let mut merged = Vec::new();
    let mut errors = Vec::new();

    for decl in decls {
        if let Decl::Import(name) = decl {
            let module_path = if let Some(std_path) = std_module_path(name) {
                Some(std_path)
            } else if let Some(dir) = base_dir {
                let local_path = dir.join(format!("{}.lust", name));
                if local_path.exists() {
                    Some(local_path)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(module_path) = module_path {
                let canonical = match module_path.canonicalize() {
                    Ok(path) => path,
                    Err(_) => {
                        errors.push(format!(
                            "Lust Error: module '{}' not found at {}",
                            name,
                            module_path.display()
                        ));
                        continue;
                    }
                };

                if visited_std_modules.contains(&canonical) {
                    continue;
                }
                visited_std_modules.insert(canonical.clone());

                let source = match fs::read_to_string(&canonical) {
                    Ok(source) => source,
                    Err(err) => {
                        errors.push(format!(
                            "Lust Error: failed to read module '{}': {}",
                            canonical.display(),
                            err
                        ));
                        continue;
                    }
                };

                match parse_program_with_std_imports(&source, canonical.parent(), visited_std_modules) {
                    Ok(mut module_decls) => merged.append(&mut module_decls),
                    Err(mut module_errors) => errors.append(&mut module_errors),
                }
                continue;
            }
        }

        merged.push(decl.clone());
    }

    if errors.is_empty() {
        Ok(merged)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
fn lex_tokens(lust_code: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(lust_code);
    let mut tokens = Vec::new();
    while let Some(t) = lexer.next_token() {
        tokens.push(t.kind);
    }
    tokens
}

#[cfg(test)]
fn repl_needs_more_input(lust_code: &str) -> bool {
    let tokens = lex_tokens(lust_code);
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut block_depth = 0usize;

    for (idx, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => paren_depth += 1,
            Token::RParen => paren_depth = paren_depth.saturating_sub(1),
            Token::LBrace => brace_depth += 1,
            Token::RBrace => brace_depth = brace_depth.saturating_sub(1),
            Token::LBracket => bracket_depth += 1,
            Token::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
            Token::Fn | Token::Match | Token::While => block_depth += 1,
            Token::If => {
                let preceded_by_else = idx > 0 && matches!(tokens[idx - 1], Token::Else);
                if !preceded_by_else {
                    block_depth += 1;
                }
            }
            Token::End => block_depth = block_depth.saturating_sub(1),
            _ => {}
        }
    }

    paren_depth > 0 || brace_depth > 0 || bracket_depth > 0 || block_depth > 0
}

struct CwdGuard {
    previous: PathBuf,
}

impl CwdGuard {
    fn change_to(path: &Path) -> Result<Self, String> {
        let previous = std::env::current_dir()
            .map_err(|e| format!("Failed to read current directory: {}", e))?;
        std::env::set_current_dir(path)
            .map_err(|e| format!("Failed to change directory to {}: {}", path.display(), e))?;
        Ok(Self { previous })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.previous);
    }
}

fn resolve_script_path(path: &str) -> Result<PathBuf, String> {
    let input = Path::new(path);
    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Failed to read current directory: {}", e))?
            .join(input)
    };
    Ok(absolute)
}

fn script_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

struct RunProfiler {
    enabled: bool,
    phases: Vec<(&'static str, Duration)>,
}

impl RunProfiler {
    fn maybe_from_env() -> Self {
        let enabled = std::env::var("LUST_PROFILE")
            .map(|value| {
                let trimmed = value.trim();
                !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
            })
            .unwrap_or(false);

        Self {
            enabled,
            phases: Vec::new(),
        }
    }

    fn record(&mut self, phase: &'static str, duration: Duration) {
        if self.enabled {
            self.phases.push((phase, duration));
        }
    }

    fn finish(&self) {
        if !self.enabled {
            return;
        }

        eprintln!("lust profile:");
        for (phase, duration) in &self.phases {
            eprintln!("  {:<16} {:>8.3} ms", phase, duration.as_secs_f64() * 1000.0);
        }
    }
}

fn run_vm_file(path: &str, args: Vec<String>) -> Result<(), String> {
    let mut profiler = RunProfiler::maybe_from_env();
    let total_start = Instant::now();

    let resolve_start = Instant::now();
    let script_path = resolve_script_path(path)?;
    profiler.record("resolve_path", resolve_start.elapsed());

    let cwd_start = Instant::now();
    let _cwd = CwdGuard::change_to(&script_dir(&script_path))?;
    profiler.record("set_cwd", cwd_start.elapsed());

    let read_start = Instant::now();
    let lust_code = fs::read_to_string(&script_path).map_err(|e| format!("Failed to read Lust file: {}", e))?;
    profiler.record("read_file", read_start.elapsed());

    let mut visited_std_modules = HashSet::new();
    let parse_start = Instant::now();
    let decls = parse_program_with_std_imports(&lust_code, script_path.parent(), &mut visited_std_modules)
        .map_err(|errs| errs.join("\n"))?;
    profiler.record("parse_imports", parse_start.elapsed());

    let typecheck_start = Instant::now();
    let type_info = TypeChecker::new()
        .check(&decls)
        .map_err(|errs| errs.join("\n"))?;
    profiler.record("typecheck", typecheck_start.elapsed());

    let compile_start = Instant::now();
    let chunk = BytecodeCompiler::new(type_info)
        .compile(&decls)
        .map_err(|errs| errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))?;
    profiler.record("bytecode", compile_start.elapsed());

    let mut vm = Vm::new_with_args(chunk, args);
    let vm_start = Instant::now();
    let result = vm.run();
    profiler.record("vm_run", vm_start.elapsed());
    profiler.record("total", total_start.elapsed());
    profiler.finish();
    result
}

fn run_vm_capture(path: &str, args: Vec<String>) -> String {
    let script_path = match resolve_script_path(path) {
        Ok(path) => path,
        Err(err) => return err,
    };
    let _cwd = match CwdGuard::change_to(&script_dir(&script_path)) {
        Ok(guard) => guard,
        Err(err) => return err,
    };
    let lust_code = match fs::read_to_string(&script_path) {
        Ok(code) => code,
        Err(err) => return format!("Failed to read Lust file: {}", err),
    };
    let mut visited_std_modules = HashSet::new();
    let decls = match parse_program_with_std_imports(&lust_code, script_path.parent(), &mut visited_std_modules) {
        Ok(decls) => decls,
        Err(errs) => return errs.join("\n"),
    };
    let type_info = match TypeChecker::new().check(&decls) {
        Ok(type_info) => type_info,
        Err(errs) => return errs.join("\n"),
    };
    let program = match BytecodeCompiler::new(type_info).compile(&decls) {
        Ok(program) => program,
        Err(errs) => return errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"),
    };
    let mut vm = Vm::new_with_args(program, args);
    match vm.run() {
        Ok(()) => vm.output().join("\n"),
        Err(err) => err,
    }
}

fn run_file_capture(path: &str, args: Vec<String>) -> String {
    run_vm_capture(path, args)
}

fn handle_test() {
    let test_dir = "tests";
    if !Path::new(test_dir).exists() {
        println!("{}", "No tests folder found.".yellow());
        return;
    }

    let entries = fs::read_dir(test_dir).expect("Failed to read tests directory");
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("lust") {
            let content = fs::read_to_string(&path).unwrap();
            let first_line = content.lines().next().unwrap_or("");
            
            // Expected output: // Expected: "Count is: 1"
            let expected_prefix = "// Expected: ";
            if first_line.starts_with(expected_prefix) {
                let expected = first_line.trim_start_matches(expected_prefix).trim_matches('"');
                let expected_norm = expected.split_whitespace().collect::<Vec<_>>().join(" ");
                let actual = run_file_capture(path.to_str().unwrap(), vec![]).trim().to_string();
                let actual_norm = actual.split_whitespace().collect::<Vec<_>>().join(" ");
                
                if actual_norm == expected_norm {
                   println!("{}: {} {}", "SUCCESS".green().bold(), path.file_name().unwrap().to_str().unwrap(), "passed".green());
                } else {
                    println!("{}: {} {}", "ERROR".red().bold(), path.file_name().unwrap().to_str().unwrap(), "failed".red());
                    println!("  Expected: {}", expected.blue());
                    println!("  Actual:   {}", actual.red());
                }
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { path, args } => {
            if let Err(err) = run_vm_file(path, args.clone()) {
                println!("{}", err);
            }
        }
        Commands::Test => {
            handle_test();
        }
        Commands::Fuzz { path } => {
            handle_fuzz(path);
        }
    }
}

#[cfg(test)]
mod repl_tests {
    use super::repl_needs_more_input;

    #[test]
    fn repl_multiline_helper_detects_open_function_block() {
        assert!(repl_needs_more_input(
            r#"
fn add_one(n)
    return n + 1
"#
        ));
    }

    #[test]
    fn repl_multiline_helper_accepts_closed_function_block() {
        assert!(!repl_needs_more_input(
            r#"
fn add_one(n)
    return n + 1
end
"#
        ));
    }

    #[test]
    fn repl_multiline_helper_detects_unclosed_delimiters() {
        assert!(repl_needs_more_input("print(add_one(1"));
        assert!(!repl_needs_more_input("print(add_one(1))"));
    }
}

fn handle_fuzz(dir_path: &str) {
    if !Path::new(dir_path).exists() {
        println!("Fuzz directory not found.");
        return;
    }

    let entries = fs::read_dir(dir_path).expect("Failed to read fuzz directory");
    let mut total = 0;
    let mut survived = 0;

    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("lust") {
            total += 1;
            let output = run_file_capture(path.to_str().unwrap(), vec![]);
            if !output.contains("panic") && !output.contains("process::exit(1)") {
                survived += 1;
            }
        }
    }

    println!("Fuzzing complete. Compiler survived {}/{} garbage files without crashing.", survived, total);
}
