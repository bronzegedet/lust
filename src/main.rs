use lust::lexer::Lexer;
use lust::parser::Parser;
#[cfg(test)]
use lust::token::Token;
use lust::typecheck::TypeChecker;
use lust::ast::{Decl, Stmt};
use lust::bytecode::Value;
use lust::bytecode_compiler::BytecodeCompiler;
use lust::vm::{Vm, VmMemorySnapshot};
use std::env;
use std::fs;
use std::collections::HashSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use clap::{Parser as ClapParser, Subcommand};
use colored::*;

mod ide_shell;

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
    /// Watch and rerun a Lust script with VM UI-state persistence (practical IDE loop)
    Ide {
        /// Path to the .lust file
        path: String,
        /// Additional arguments for the script
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// File-watch poll interval in milliseconds
        #[arg(long = "interval-ms", default_value_t = 250)]
        interval_ms: u64,
        /// Debounce delay before rerun after a change is detected
        #[arg(long = "debounce-ms", default_value_t = 300)]
        debounce_ms: u64,
        /// Show executed-code trace mapping for live UI interactions
        #[arg(long = "show-exec", default_value_t = false)]
        show_exec: bool,
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

fn run_vm_file_with_ui_state(
    path: &str,
    args: Vec<String>,
    runtime_inputs: Vec<String>,
    incoming_ui_state: Option<HashMap<String, Value>>,
    show_exec: bool,
) -> Result<(HashMap<String, Value>, Vec<String>, Vec<String>, VmMemorySnapshot), String> {
    let script_path = resolve_script_path(path)?;
    let _cwd = CwdGuard::change_to(&script_dir(&script_path))?;

    let lust_code = fs::read_to_string(&script_path)
        .map_err(|e| format!("Failed to read Lust file: {}", e))?;
    let mut visited_std_modules = HashSet::new();
    let decls = parse_program_with_std_imports(
        &lust_code,
        script_path.parent(),
        &mut visited_std_modules,
    )
    .map_err(|errs| errs.join("\n"))?;
    let type_info = TypeChecker::new()
        .check(&decls)
        .map_err(|errs| errs.join("\n"))?;
    let chunk = BytecodeCompiler::new(type_info)
        .compile(&decls)
        .map_err(|errs| errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))?;

    let mut vm = Vm::new_with_args_keys_and_input(chunk, args, runtime_inputs, Vec::new());
    vm.set_trace_enabled(show_exec);
    if let Some(state) = incoming_ui_state {
        vm.restore_ui_state(state)?;
    }
    vm.run()?;
    Ok((
        vm.ui_state_snapshot(),
        vm.output().to_vec(),
        vm.trace_events_snapshot(),
        vm.memory_snapshot(),
    ))
}

fn run_ide_watch(
    path: &str,
    args: Vec<String>,
    interval_ms: u64,
    debounce_ms: u64,
    show_exec: bool,
) -> Result<(), String> {
    let resolved = resolve_script_path(path)?;
    let project_root = resolved
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut active_path = resolved.clone();
    let mut last_observed_modified = fs::metadata(&active_path)
        .and_then(|meta| meta.modified())
        .map_err(|e| format!("Failed to read script metadata: {}", e))?;
    let debounce = Duration::from_millis(debounce_ms.max(50));

    let mut shell = ide_shell::IdeShell::start(&resolved)?;
    if shell.is_none() {
        println!("lust ide watching {}", resolved.display());
        println!("Edit + save the file to rerun. Press Ctrl+C to stop.");
        println!("When shell is active: press 'r' to force reload, 'q' to quit.");
        println!(
            "poll={}ms debounce={}ms",
            interval_ms.max(50),
            debounce_ms.max(50)
        );
    }

    let mut ui_state: HashMap<String, Value> = HashMap::new();
    let mut current_source = fs::read_to_string(&active_path).unwrap_or_default();
    let mut last_output_lines: Vec<String> = Vec::new();
    let mut last_exec_preview_lines: Vec<String> = Vec::new();
    let mut inspect_mode = false;
    let mut inspect_index = 0usize;
    let mut inspect_targets: Vec<InspectTarget> = Vec::new();
    let mut project_symbols = build_project_symbol_index(&project_root, &active_path);
    let mut symbol_filter_query = String::new();
    let mut symbol_filtered_indices = symbol_filter_indices(&project_symbols, &symbol_filter_query);
    let mut symbol_index: usize = 0;
    let mut symbol_page: usize = 0;
    let symbol_page_size: usize = 12;
    let mut symbol_filter_active = false;
    let mut symbol_nav_history = vec![symbol_nav_point(&active_path, 1)];
    let mut symbol_nav_history_index: usize = 0;
    if let Some(shell) = shell.as_mut() {
        shell.set_source(&current_source);
        shell.set_status("running initial pass");
        shell.set_symbol_palette(false, Vec::new(), 0);
    }
    match run_vm_file_with_ui_state(
        &active_path.display().to_string(),
        args.clone(),
        Vec::new(),
        None,
        show_exec,
    ) {
        Ok((state, output_lines, trace_events, memory)) => {
            ui_state = state;
            last_output_lines = output_lines.clone();
            last_exec_preview_lines = if show_exec {
                trace_to_preview_exec_lines(&current_source, &trace_events)
            } else {
                Vec::new()
            };
            inspect_targets = build_inspect_targets(&ui_state, &current_source);
            if inspect_index >= inspect_targets.len() {
                inspect_index = 0;
            }
            if let Some(shell) = shell.as_mut() {
                let (theme_name, theme_settings) = extract_theme_from_ui_state(&ui_state);
                shell.apply_theme(ide_shell::ShellTheme::from_lust(
                    &theme_name,
                    &theme_settings,
                ));
                let selected_id = selected_inspect_id(inspect_mode, &inspect_targets, inspect_index);
                shell.set_preview_lines(build_ui_preview_lines(
                    &last_output_lines,
                    &ui_state,
                    &last_exec_preview_lines,
                    selected_id,
                ));
                shell.set_focus_line(selected_inspect_line(inspect_mode, &inspect_targets, inspect_index));
                if show_exec {
                    for line in trace_to_source_hints(&current_source, &trace_events) {
                        shell.push_diag(format!("[exec] {}", line));
                    }
                }
                shell.push_diag(format!("[mem] {}", format_memory_snapshot(&memory)));
                shell.push_diag("[ide] initial run ok");
                shell.set_status("ready");
            } else {
                println!("[ide] initial run ok");
            }
        }
        Err(err) => {
            if let Some(shell) = shell.as_mut() {
                let focus_line = extract_first_line_number(&err);
                shell.push_diag(err);
                shell.set_focus_line(focus_line);
                shell.set_status("initial run failed");
            } else {
                println!("{}", err);
            }
        }
    }
    if let Some(shell) = shell.as_mut() {
        shell.render()?;
    }

    let mut pending_change_at: Option<Instant> = None;
    let mut pending_runtime_inputs: Vec<String> = Vec::new();
    loop {
        if let Some(shell) = shell.as_mut() {
            match shell.poll_action()? {
                ide_shell::ShellAction::Quit => {
                    shell.stop()?;
                    break;
                }
                ide_shell::ShellAction::Reload => {
                    pending_change_at = Some(Instant::now() - debounce);
                    pending_runtime_inputs.clear();
                    shell.push_diag("[ide] manual reload requested");
                    shell.set_status("manual reload requested");
                    shell.render()?;
                }
                ide_shell::ShellAction::SymbolPrev => {
                    if !symbol_filtered_indices.is_empty() {
                        symbol_index = if symbol_index == 0 {
                            symbol_filtered_indices.len().saturating_sub(1)
                        } else {
                            symbol_index.saturating_sub(1)
                        };
                        if let Some(&pool_idx) = symbol_filtered_indices.get(symbol_index) {
                            let symbol = &project_symbols[pool_idx];
                            let selected_key = symbol_selection_key(symbol);
                            let should_rerun = apply_symbol_navigation(
                                symbol,
                                &mut active_path,
                                &mut current_source,
                                &mut ui_state,
                                &mut last_observed_modified,
                                shell,
                            );
                            if should_rerun {
                                push_symbol_nav_history(
                                    &mut symbol_nav_history,
                                    &mut symbol_nav_history_index,
                                    symbol_nav_point(&symbol.path, symbol.line),
                                );
                                project_symbols = build_project_symbol_index(&project_root, &active_path);
                                symbol_filtered_indices =
                                    symbol_filter_indices(&project_symbols, &symbol_filter_query);
                                symbol_index = restore_symbol_selection(
                                    &project_symbols,
                                    &symbol_filtered_indices,
                                    Some(&selected_key),
                                    symbol_index,
                                );
                                symbol_page = symbol_index / symbol_page_size;
                                pending_change_at = Some(Instant::now() - debounce);
                            }
                            shell.set_status(format!(
                                "symbol {}/{}",
                                symbol_index + 1,
                                symbol_filtered_indices.len()
                            ));
                            shell.render()?;
                        }
                    }
                }
                ide_shell::ShellAction::SymbolNext => {
                    if !symbol_filtered_indices.is_empty() {
                        symbol_index = (symbol_index + 1) % symbol_filtered_indices.len();
                        if let Some(&pool_idx) = symbol_filtered_indices.get(symbol_index) {
                            let symbol = &project_symbols[pool_idx];
                            let selected_key = symbol_selection_key(symbol);
                            let should_rerun = apply_symbol_navigation(
                                symbol,
                                &mut active_path,
                                &mut current_source,
                                &mut ui_state,
                                &mut last_observed_modified,
                                shell,
                            );
                            if should_rerun {
                                push_symbol_nav_history(
                                    &mut symbol_nav_history,
                                    &mut symbol_nav_history_index,
                                    symbol_nav_point(&symbol.path, symbol.line),
                                );
                                project_symbols = build_project_symbol_index(&project_root, &active_path);
                                symbol_filtered_indices =
                                    symbol_filter_indices(&project_symbols, &symbol_filter_query);
                                symbol_index = restore_symbol_selection(
                                    &project_symbols,
                                    &symbol_filtered_indices,
                                    Some(&selected_key),
                                    symbol_index,
                                );
                                symbol_page = symbol_index / symbol_page_size;
                                pending_change_at = Some(Instant::now() - debounce);
                            }
                            shell.set_status(format!(
                                "symbol {}/{}",
                                symbol_index + 1,
                                symbol_filtered_indices.len()
                            ));
                            shell.render()?;
                        }
                    }
                }
                ide_shell::ShellAction::SymbolBack => {
                    if symbol_nav_history_index > 0 {
                        symbol_nav_history_index = symbol_nav_history_index.saturating_sub(1);
                        if let Some(point) = symbol_nav_history.get(symbol_nav_history_index).cloned() {
                            let should_rerun = apply_symbol_nav_point(
                                &point,
                                &mut active_path,
                                &mut current_source,
                                &mut ui_state,
                                &mut last_observed_modified,
                                shell,
                            );
                            if should_rerun {
                                project_symbols = build_project_symbol_index(&project_root, &active_path);
                                symbol_filtered_indices =
                                    symbol_filter_indices(&project_symbols, &symbol_filter_query);
                                symbol_index = symbol_filtered_indices
                                    .iter()
                                    .position(|pool_idx| {
                                        project_symbols.get(*pool_idx).is_some_and(|symbol| {
                                            symbol.path == point.path && symbol.line == point.line
                                        })
                                    })
                                    .unwrap_or(0);
                                symbol_page = symbol_index / symbol_page_size;
                                pending_change_at = Some(Instant::now() - debounce);
                            }
                            shell.set_status(format!(
                                "symbol history {}/{}",
                                symbol_nav_history_index + 1,
                                symbol_nav_history.len()
                            ));
                            shell.render()?;
                        }
                    } else {
                        shell.set_status("symbol history start");
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolForward => {
                    if symbol_nav_history_index + 1 < symbol_nav_history.len() {
                        symbol_nav_history_index += 1;
                        if let Some(point) = symbol_nav_history.get(symbol_nav_history_index).cloned() {
                            let should_rerun = apply_symbol_nav_point(
                                &point,
                                &mut active_path,
                                &mut current_source,
                                &mut ui_state,
                                &mut last_observed_modified,
                                shell,
                            );
                            if should_rerun {
                                project_symbols = build_project_symbol_index(&project_root, &active_path);
                                symbol_filtered_indices =
                                    symbol_filter_indices(&project_symbols, &symbol_filter_query);
                                symbol_index = symbol_filtered_indices
                                    .iter()
                                    .position(|pool_idx| {
                                        project_symbols.get(*pool_idx).is_some_and(|symbol| {
                                            symbol.path == point.path && symbol.line == point.line
                                        })
                                    })
                                    .unwrap_or(0);
                                symbol_page = symbol_index / symbol_page_size;
                                pending_change_at = Some(Instant::now() - debounce);
                            }
                            shell.set_status(format!(
                                "symbol history {}/{}",
                                symbol_nav_history_index + 1,
                                symbol_nav_history.len()
                            ));
                            shell.render()?;
                        }
                    } else {
                        shell.set_status("symbol history end");
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolFilterChanged { query } => {
                    symbol_filter_active = true;
                    symbol_filter_query = query;
                    symbol_filtered_indices =
                        symbol_filter_indices(&project_symbols, &symbol_filter_query);
                    symbol_index = 0;
                    symbol_page = 0;
                    shell.set_symbol_palette(
                        true,
                        build_symbol_palette_lines(
                            &project_symbols,
                            &symbol_filtered_indices,
                            symbol_index,
                            symbol_page,
                            symbol_page_size,
                            &symbol_filter_query,
                        ),
                        symbol_palette_cursor_line(symbol_index, symbol_page, symbol_page_size),
                    );
                    shell.push_diag(format!(
                        "[symbol] filter='{}' matches={}",
                        symbol_filter_query,
                        symbol_filtered_indices.len()
                    ));
                    shell.set_status(format!(
                        "symbol filter {}",
                        if symbol_filter_query.is_empty() {
                            "(all)".to_string()
                        } else {
                            symbol_filter_query.clone()
                        }
                    ));
                    shell.render()?;
                }
                ide_shell::ShellAction::SymbolFilterPrev => {
                    if !symbol_filtered_indices.is_empty() {
                        symbol_index = if symbol_index == 0 {
                            symbol_filtered_indices.len().saturating_sub(1)
                        } else {
                            symbol_index.saturating_sub(1)
                        };
                        symbol_page = symbol_index / symbol_page_size;
                        shell.set_symbol_palette(
                            true,
                            build_symbol_palette_lines(
                                &project_symbols,
                                &symbol_filtered_indices,
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                                &symbol_filter_query,
                            ),
                            symbol_palette_cursor_line(
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                            ),
                        );
                        shell.set_status(format!(
                            "symbol filter {}/{}",
                            symbol_index + 1,
                            symbol_filtered_indices.len()
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolFilterNext => {
                    if !symbol_filtered_indices.is_empty() {
                        symbol_index = (symbol_index + 1) % symbol_filtered_indices.len();
                        symbol_page = symbol_index / symbol_page_size;
                        shell.set_symbol_palette(
                            true,
                            build_symbol_palette_lines(
                                &project_symbols,
                                &symbol_filtered_indices,
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                                &symbol_filter_query,
                            ),
                            symbol_palette_cursor_line(
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                            ),
                        );
                        shell.set_status(format!(
                            "symbol filter {}/{}",
                            symbol_index + 1,
                            symbol_filtered_indices.len()
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolFilterPagePrev => {
                    if !symbol_filtered_indices.is_empty() {
                        let max_page =
                            (symbol_filtered_indices.len().saturating_sub(1)) / symbol_page_size;
                        symbol_page = symbol_page.saturating_sub(1);
                        if symbol_page > max_page {
                            symbol_page = max_page;
                        }
                        let page_start = symbol_page * symbol_page_size;
                        symbol_index = page_start.min(symbol_filtered_indices.len().saturating_sub(1));
                        shell.set_symbol_palette(
                            true,
                            build_symbol_palette_lines(
                                &project_symbols,
                                &symbol_filtered_indices,
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                                &symbol_filter_query,
                            ),
                            symbol_palette_cursor_line(
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                            ),
                        );
                        shell.set_status(format!(
                            "symbol page {}/{}",
                            symbol_page + 1,
                            max_page + 1
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolFilterPageNext => {
                    if !symbol_filtered_indices.is_empty() {
                        let max_page =
                            (symbol_filtered_indices.len().saturating_sub(1)) / symbol_page_size;
                        symbol_page = (symbol_page + 1).min(max_page);
                        let page_start = symbol_page * symbol_page_size;
                        symbol_index = page_start.min(symbol_filtered_indices.len().saturating_sub(1));
                        shell.set_symbol_palette(
                            true,
                            build_symbol_palette_lines(
                                &project_symbols,
                                &symbol_filtered_indices,
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                                &symbol_filter_query,
                            ),
                            symbol_palette_cursor_line(
                                symbol_index,
                                symbol_page,
                                symbol_page_size,
                            ),
                        );
                        shell.set_status(format!(
                            "symbol page {}/{}",
                            symbol_page + 1,
                            max_page + 1
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolPaletteClick { line_index } => {
                    if let Some(target_index) = symbol_index_from_palette_line(
                        symbol_filtered_indices.len(),
                        symbol_page,
                        symbol_page_size,
                        line_index,
                    ) {
                        symbol_index = target_index;
                        symbol_filter_active = false;
                        if let Some(&pool_idx) = symbol_filtered_indices.get(symbol_index) {
                            let symbol = &project_symbols[pool_idx];
                            let selected_key = symbol_selection_key(symbol);
                            let should_rerun = apply_symbol_navigation(
                                symbol,
                                &mut active_path,
                                &mut current_source,
                                &mut ui_state,
                                &mut last_observed_modified,
                                shell,
                            );
                            if should_rerun {
                                push_symbol_nav_history(
                                    &mut symbol_nav_history,
                                    &mut symbol_nav_history_index,
                                    symbol_nav_point(&symbol.path, symbol.line),
                                );
                                project_symbols = build_project_symbol_index(&project_root, &active_path);
                                symbol_filtered_indices =
                                    symbol_filter_indices(&project_symbols, &symbol_filter_query);
                                symbol_index = restore_symbol_selection(
                                    &project_symbols,
                                    &symbol_filtered_indices,
                                    Some(&selected_key),
                                    symbol_index,
                                );
                                symbol_page = symbol_index / symbol_page_size;
                                pending_change_at = Some(Instant::now() - debounce);
                            }
                            shell.set_status(format!(
                                "symbol {}/{}",
                                symbol_index + 1,
                                symbol_filtered_indices.len()
                            ));
                        } else {
                            shell.push_diag("[symbol] no match");
                            shell.set_status("symbol no match");
                        }
                        shell.set_symbol_palette(false, Vec::new(), 0);
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::SymbolOpenSelected => {
                    symbol_filter_active = false;
                    if let Some(&pool_idx) = symbol_filtered_indices.get(symbol_index) {
                        let symbol = &project_symbols[pool_idx];
                        let selected_key = symbol_selection_key(symbol);
                        let should_rerun = apply_symbol_navigation(
                            symbol,
                            &mut active_path,
                            &mut current_source,
                            &mut ui_state,
                            &mut last_observed_modified,
                            shell,
                        );
                        if should_rerun {
                            push_symbol_nav_history(
                                &mut symbol_nav_history,
                                &mut symbol_nav_history_index,
                                symbol_nav_point(&symbol.path, symbol.line),
                            );
                            project_symbols = build_project_symbol_index(&project_root, &active_path);
                            symbol_filtered_indices =
                                symbol_filter_indices(&project_symbols, &symbol_filter_query);
                            symbol_index = restore_symbol_selection(
                                &project_symbols,
                                &symbol_filtered_indices,
                                Some(&selected_key),
                                symbol_index,
                            );
                            symbol_page = symbol_index / symbol_page_size;
                            pending_change_at = Some(Instant::now() - debounce);
                        }
                        shell.set_status(format!(
                            "symbol {}/{}",
                            symbol_index + 1,
                            symbol_filtered_indices.len()
                        ));
                    } else {
                        shell.push_diag("[symbol] no match");
                        shell.set_status("symbol no match");
                    }
                    shell.set_symbol_palette(false, Vec::new(), 0);
                    shell.render()?;
                }
                ide_shell::ShellAction::SymbolFilterCancel => {
                    symbol_filter_active = false;
                    symbol_filter_query.clear();
                    symbol_filtered_indices =
                        symbol_filter_indices(&project_symbols, &symbol_filter_query);
                    symbol_index = 0;
                    symbol_page = 0;
                    shell.push_diag("[symbol] filter cancelled");
                    shell.set_status("ready");
                    shell.set_symbol_palette(false, Vec::new(), 0);
                    shell.render()?;
                }
                ide_shell::ShellAction::PreviewPointer { kind, col, row, shift } => {
                    let phase = match kind {
                        ide_shell::PreviewPointerKind::Down => "down",
                        ide_shell::PreviewPointerKind::Drag => "drag",
                        ide_shell::PreviewPointerKind::Up => "up",
                    };
                    let event_input = match kind {
                        ide_shell::PreviewPointerKind::Down => {
                            if shift {
                                ui_state.insert(
                                    "editor.pointer.extend_once".to_string(),
                                    Value::Bool(true),
                                );
                            }
                            format!("mouse_down:{}:{}:left", col, row)
                        }
                        ide_shell::PreviewPointerKind::Drag => {
                            format!("mouse_drag:{}:{}:left", col, row)
                        }
                        ide_shell::PreviewPointerKind::Up => {
                            format!("mouse_up:{}:{}:left", col, row)
                        }
                    };
                    pending_runtime_inputs = vec![event_input];
                    pending_change_at = Some(Instant::now() - debounce);
                    shell.push_diag(format!(
                        "[ide] preview pointer {} col={} row={} shift={}",
                        phase, col, row, shift
                    ));
                    shell.set_status("pointer event");
                    shell.render()?;
                }
                ide_shell::ShellAction::SourceClick { line, shift } => {
                    let (target_line, span_label, span_range) =
                        semantic_source_target_from_ast(&current_source, line);
                    ui_state.insert(
                        "editor.host.goto_line".to_string(),
                        Value::Number(target_line as f64),
                    );
                    if let Some((start_line, end_line)) = span_range {
                        ui_state.insert(
                            "editor.host.goto_span_start".to_string(),
                            Value::Number(start_line as f64),
                        );
                        ui_state.insert(
                            "editor.host.goto_span_end".to_string(),
                            Value::Number(end_line as f64),
                        );
                        ui_state.insert(
                            "editor.host.goto_span_mode".to_string(),
                            Value::String(if shift { "select" } else { "goto" }.to_string()),
                        );
                    }
                    if shift {
                        ui_state.insert("editor.pointer.extend_once".to_string(), Value::Bool(true));
                    }
                    push_symbol_nav_history(
                        &mut symbol_nav_history,
                        &mut symbol_nav_history_index,
                        symbol_nav_point(&active_path, target_line),
                    );
                    pending_change_at = Some(Instant::now() - debounce);
                    shell.set_focus_line(Some(target_line));
                    if let Some(label) = span_label {
                        shell.push_diag(format!(
                            "[source] goto {} (line {} -> {} @ line {}, shift={})",
                            target_line, line, label, target_line, shift
                        ));
                    } else {
                        shell.push_diag(format!(
                            "[source] goto line {} (clicked {}, shift={})",
                            target_line, line, shift
                        ));
                    }
                    shell.set_status(format!("source goto {}", target_line));
                    shell.render()?;
                }
                ide_shell::ShellAction::ToggleInspect => {
                    inspect_mode = !inspect_mode;
                    if inspect_mode {
                        if inspect_targets.is_empty() {
                            shell.push_diag("[inspect] no UI targets available in current run");
                            shell.set_status("inspect: no targets");
                            shell.set_focus_line(None);
                        } else {
                            if inspect_index >= inspect_targets.len() {
                                inspect_index = 0;
                            }
                            if let Some(target) = inspect_targets.get(inspect_index) {
                                shell.push_diag(format!(
                                    "[inspect] {} -> {}",
                                    target.id,
                                    target.source_label()
                                ));
                            }
                            shell.set_status(format!(
                                "inspect {}/{}",
                                inspect_index + 1,
                                inspect_targets.len()
                            ));
                            shell.set_focus_line(selected_inspect_line(
                                inspect_mode,
                                &inspect_targets,
                                inspect_index,
                            ));
                        }
                    } else {
                        shell.push_diag("[inspect] off");
                        shell.set_status("ready");
                        shell.set_focus_line(None);
                    }
                    let selected_id =
                        selected_inspect_id(inspect_mode, &inspect_targets, inspect_index);
                    shell.set_preview_lines(build_ui_preview_lines(
                        &last_output_lines,
                        &ui_state,
                        &last_exec_preview_lines,
                        selected_id,
                    ));
                    shell.render()?;
                }
                ide_shell::ShellAction::InspectNext => {
                    if inspect_mode && !inspect_targets.is_empty() {
                        inspect_index = (inspect_index + 1) % inspect_targets.len();
                        if let Some(target) = inspect_targets.get(inspect_index) {
                            shell.push_diag(format!(
                                "[inspect] {} -> {}",
                                target.id,
                                target.source_label()
                            ));
                        }
                        shell.set_status(format!(
                            "inspect {}/{}",
                            inspect_index + 1,
                            inspect_targets.len()
                        ));
                        shell.set_focus_line(selected_inspect_line(
                            inspect_mode,
                            &inspect_targets,
                            inspect_index,
                        ));
                        let selected_id =
                            selected_inspect_id(inspect_mode, &inspect_targets, inspect_index);
                        shell.set_preview_lines(build_ui_preview_lines(
                            &last_output_lines,
                            &ui_state,
                            &last_exec_preview_lines,
                            selected_id,
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::InspectPrev => {
                    if inspect_mode && !inspect_targets.is_empty() {
                        inspect_index = if inspect_index == 0 {
                            inspect_targets.len().saturating_sub(1)
                        } else {
                            inspect_index.saturating_sub(1)
                        };
                        if let Some(target) = inspect_targets.get(inspect_index) {
                            shell.push_diag(format!(
                                "[inspect] {} -> {}",
                                target.id,
                                target.source_label()
                            ));
                        }
                        shell.set_status(format!(
                            "inspect {}/{}",
                            inspect_index + 1,
                            inspect_targets.len()
                        ));
                        shell.set_focus_line(selected_inspect_line(
                            inspect_mode,
                            &inspect_targets,
                            inspect_index,
                        ));
                        let selected_id =
                            selected_inspect_id(inspect_mode, &inspect_targets, inspect_index);
                        shell.set_preview_lines(build_ui_preview_lines(
                            &last_output_lines,
                            &ui_state,
                            &last_exec_preview_lines,
                            selected_id,
                        ));
                        shell.render()?;
                    }
                }
                ide_shell::ShellAction::None => {}
            }
        }

        std::thread::sleep(Duration::from_millis(interval_ms.max(50)));
        let modified = fs::metadata(&active_path)
            .and_then(|meta| meta.modified())
            .map_err(|e| format!("Failed to read script metadata: {}", e))?;
        if modified > last_observed_modified {
            last_observed_modified = modified;
            pending_change_at = Some(Instant::now());
            pending_runtime_inputs.clear();
            current_source = fs::read_to_string(&active_path).unwrap_or_default();
            let selected_key =
                selected_symbol_key(&project_symbols, &symbol_filtered_indices, symbol_index);
            project_symbols = build_project_symbol_index(&project_root, &active_path);
            symbol_filtered_indices =
                symbol_filter_indices(&project_symbols, &symbol_filter_query);
            symbol_index = restore_symbol_selection(
                &project_symbols,
                &symbol_filtered_indices,
                selected_key.as_ref(),
                symbol_index,
            );
            symbol_page = symbol_index / symbol_page_size;
            if let Some(shell) = shell.as_mut() {
                shell.set_source(&current_source);
                shell.set_path_label(active_path.display().to_string());
                shell.push_diag(format!("[ide] change detected: {}", active_path.display()));
                shell.push_diag(format!(
                    "[symbol] indexed {} symbols",
                    project_symbols.len()
                ));
                if !symbol_filter_query.is_empty() {
                    shell.push_diag(format!(
                        "[symbol] filter='{}' matches={}",
                        symbol_filter_query,
                        symbol_filtered_indices.len()
                    ));
                }
                if symbol_filter_active {
                    shell.set_symbol_palette(
                        true,
                        build_symbol_palette_lines(
                            &project_symbols,
                            &symbol_filtered_indices,
                            symbol_index,
                            symbol_page,
                            symbol_page_size,
                            &symbol_filter_query,
                        ),
                        symbol_palette_cursor_line(symbol_index, symbol_page, symbol_page_size),
                    );
                }
                shell.set_status("change detected");
                shell.render()?;
            } else {
                println!("\n[ide] change detected: {}", active_path.display());
            }
        }

        if let Some(change_at) = pending_change_at {
            if change_at.elapsed() >= debounce {
                pending_change_at = None;
                if let Some(shell) = shell.as_mut() {
                    shell.push_diag("[ide] applying debounced reload");
                    shell.set_status("reloading");
                    shell.render()?;
                } else {
                    println!("[ide] applying debounced reload");
                }
                match run_vm_file_with_ui_state(
                    &active_path.display().to_string(),
                    args.clone(),
                    pending_runtime_inputs.clone(),
                    Some(ui_state.clone()),
                    show_exec,
                ) {
                    Ok((state, output_lines, trace_events, memory)) => {
                        pending_runtime_inputs.clear();
                        ui_state = state;
                        let selected_key =
                            selected_symbol_key(&project_symbols, &symbol_filtered_indices, symbol_index);
                        project_symbols = build_project_symbol_index(&project_root, &active_path);
                        symbol_filtered_indices =
                            symbol_filter_indices(&project_symbols, &symbol_filter_query);
                        symbol_index = restore_symbol_selection(
                            &project_symbols,
                            &symbol_filtered_indices,
                            selected_key.as_ref(),
                            symbol_index,
                        );
                        symbol_page = symbol_index / symbol_page_size;
                        last_output_lines = output_lines.clone();
                        last_exec_preview_lines = if show_exec {
                            trace_to_preview_exec_lines(&current_source, &trace_events)
                        } else {
                            Vec::new()
                        };
                        inspect_targets = build_inspect_targets(&ui_state, &current_source);
                        if inspect_index >= inspect_targets.len() {
                            inspect_index = 0;
                        }
                        if let Some(shell) = shell.as_mut() {
                            let (theme_name, theme_settings) = extract_theme_from_ui_state(&ui_state);
                            shell.apply_theme(ide_shell::ShellTheme::from_lust(
                                &theme_name,
                                &theme_settings,
                            ));
                            let selected_id =
                                selected_inspect_id(inspect_mode, &inspect_targets, inspect_index);
                            shell.set_preview_lines(build_ui_preview_lines(
                                &last_output_lines,
                                &ui_state,
                                &last_exec_preview_lines,
                                selected_id,
                            ));
                            shell.set_focus_line(selected_inspect_line(
                                inspect_mode,
                                &inspect_targets,
                                inspect_index,
                            ));
                            if show_exec {
                                for line in trace_to_source_hints(&current_source, &trace_events) {
                                    shell.push_diag(format!("[exec] {}", line));
                                }
                            }
                            if symbol_filter_active {
                                shell.set_symbol_palette(
                                    true,
                                    build_symbol_palette_lines(
                                        &project_symbols,
                                        &symbol_filtered_indices,
                                        symbol_index,
                                        symbol_page,
                                        symbol_page_size,
                                        &symbol_filter_query,
                                    ),
                                    symbol_palette_cursor_line(
                                        symbol_index,
                                        symbol_page,
                                        symbol_page_size,
                                    ),
                                );
                            }
                            shell.push_diag(format!("[mem] {}", format_memory_snapshot(&memory)));
                            shell.push_diag("[ide] run ok");
                            if inspect_mode && !inspect_targets.is_empty() {
                                shell.set_status(format!(
                                    "inspect {}/{}",
                                    inspect_index + 1,
                                    inspect_targets.len()
                                ));
                            } else if inspect_mode {
                                shell.set_status("inspect: no targets");
                            } else {
                                shell.set_status("ready");
                            }
                            shell.render()?;
                        } else {
                            println!("[mem] {}", format_memory_snapshot(&memory));
                            println!("[ide] run ok");
                        }
                    }
                    Err(err) => {
                        pending_runtime_inputs.clear();
                        if let Some(shell) = shell.as_mut() {
                            let focus_line = extract_first_line_number(&err);
                            shell.push_diag(err);
                            shell.push_diag("[ide] keeping last good UI/runtime state");
                            shell.set_focus_line(focus_line);
                            shell.set_status("last run failed");
                            shell.render()?;
                        } else {
                            println!("{}", err);
                            println!("[ide] keeping last good UI/runtime state");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn extract_first_line_number(message: &str) -> Option<usize> {
    for line in message.lines() {
        if let Some(pos) = line.find("line ") {
            let suffix = &line[pos + 5..];
            let digits = suffix
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(number) = digits.parse::<usize>() {
                if number > 0 {
                    return Some(number);
                }
            }
        }
    }
    None
}

#[derive(Clone)]
struct AstNodeSpan {
    kind: String,
    label: Option<String>,
    start_line: usize,
    end_line: usize,
}

fn semantic_source_target_from_ast(
    source: &str,
    clicked_line: usize,
) -> (usize, Option<String>, Option<(usize, usize)>) {
    let spans = ast_node_spans(source);
    if spans.is_empty() {
        let (target_line, fn_name) = semantic_source_goto_target(source, clicked_line);
        let fn_range = semantic_function_range(source, target_line);
        return (target_line, fn_name, fn_range);
    }
    let max_line = source.lines().count().max(1);
    let safe_clicked = clicked_line.clamp(1, max_line);
    let mut best: Option<AstNodeSpan> = None;
    for span in spans {
        if safe_clicked < span.start_line || safe_clicked > span.end_line {
            continue;
        }
        match &best {
            None => best = Some(span),
            Some(current) => {
                let span_size = span.end_line.saturating_sub(span.start_line);
                let current_size = current.end_line.saturating_sub(current.start_line);
                if span_size < current_size
                    || (span_size == current_size && span.start_line >= current.start_line)
                {
                    best = Some(span);
                }
            }
        }
    }
    if let Some(span) = best {
        let label = span
            .label
            .clone()
            .unwrap_or_else(|| span.kind.clone());
        return (
            span.start_line,
            Some(label),
            Some((span.start_line, span.end_line)),
        );
    }
    (safe_clicked, None, None)
}

fn ast_node_spans(source: &str) -> Vec<AstNodeSpan> {
    let Ok(decls) = parse_program(source) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    for decl in decls {
        match decl {
            Decl::Fn(name, _, _, _, body) => {
                if body.is_empty() {
                    continue;
                }
                let (start_line, end_line) = stmt_block_line_span(&body);
                if start_line > 0 && end_line >= start_line {
                    spans.push(AstNodeSpan {
                        kind: "fn".to_string(),
                        label: Some(format!("fn {}", name)),
                        start_line,
                        end_line,
                    });
                }
                collect_stmt_spans(&body, &mut spans);
            }
            Decl::Stmt(stmt) => collect_stmt_span(&stmt, &mut spans),
            _ => {}
        }
    }
    spans.sort_by_key(|span| (span.start_line, span.end_line));
    spans
}

fn collect_stmt_spans(stmts: &[Stmt], out: &mut Vec<AstNodeSpan>) {
    for stmt in stmts {
        collect_stmt_span(stmt, out);
    }
}

fn collect_stmt_span(stmt: &Stmt, out: &mut Vec<AstNodeSpan>) {
    let (start_line, end_line) = stmt_line_span(stmt);
    if start_line > 0 && end_line >= start_line {
        out.push(AstNodeSpan {
            kind: stmt_kind_name(stmt).to_string(),
            label: stmt_label(stmt),
            start_line,
            end_line,
        });
    }
    match stmt {
        Stmt::If(_, _, then_body, else_body) => {
            collect_stmt_spans(then_body, out);
            if let Some(else_stmts) = else_body {
                collect_stmt_spans(else_stmts, out);
            }
        }
        Stmt::While(_, _, body) | Stmt::For(_, _, _, _, body) => {
            collect_stmt_spans(body, out);
        }
        Stmt::Match(_, _, cases) => {
            for case in cases {
                collect_stmt_spans(&case.body, out);
            }
        }
        _ => {}
    }
}

fn stmt_kind_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Let(_, _, _, _) | Stmt::LetPattern(_, _, _) => "let",
        Stmt::Assign(_, _, _) => "assign",
        Stmt::Pass(_) => "pass",
        Stmt::Return(_, _) => "return",
        Stmt::Break(_) => "break",
        Stmt::Continue(_) => "continue",
        Stmt::Print(_, _) => "print",
        Stmt::If(_, _, _, _) => "if",
        Stmt::Match(_, _, _) => "match",
        Stmt::While(_, _, _) => "while",
        Stmt::For(_, _, _, _, _) => "for",
        Stmt::ExprStmt(_, _) => "expr",
        Stmt::Spawn(_, _, _) => "spawn",
    }
}

fn stmt_label(stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::Let(_, name, _, _) => Some(format!("let {}", name)),
        Stmt::Assign(_, _, _) => Some("assign".to_string()),
        Stmt::If(_, _, _, _) => Some("if".to_string()),
        Stmt::Match(_, _, _) => Some("match".to_string()),
        Stmt::While(_, _, _) => Some("while".to_string()),
        Stmt::For(_, _, item_name, _, _) => Some(format!("for {}", item_name)),
        Stmt::ExprStmt(_, _) => Some("expr".to_string()),
        Stmt::Spawn(_, name, _) => Some(format!("spawn {}", name)),
        _ => None,
    }
}

fn stmt_block_line_span(stmts: &[Stmt]) -> (usize, usize) {
    let mut start = usize::MAX;
    let mut end = 0usize;
    for stmt in stmts {
        let (s, e) = stmt_line_span(stmt);
        if s < start {
            start = s;
        }
        if e > end {
            end = e;
        }
    }
    if start == usize::MAX {
        (0, 0)
    } else {
        (start, end)
    }
}

fn stmt_line_span(stmt: &Stmt) -> (usize, usize) {
    let line = stmt.line();
    match stmt {
        Stmt::If(_, _, then_body, else_body) => {
            let mut start = line;
            let mut end = line;
            let (then_start, then_end) = stmt_block_line_span(then_body);
            if then_start > 0 {
                start = start.min(then_start);
                end = end.max(then_end);
            }
            if let Some(else_stmts) = else_body {
                let (else_start, else_end) = stmt_block_line_span(else_stmts);
                if else_start > 0 {
                    start = start.min(else_start);
                    end = end.max(else_end);
                }
            }
            (start, end)
        }
        Stmt::Match(_, _, cases) => {
            let mut start = line;
            let mut end = line;
            for case in cases {
                let (case_start, case_end) = stmt_block_line_span(&case.body);
                if case_start > 0 {
                    start = start.min(case_start);
                    end = end.max(case_end);
                }
            }
            (start, end)
        }
        Stmt::While(_, _, body) | Stmt::For(_, _, _, _, body) => {
            let (body_start, body_end) = stmt_block_line_span(body);
            if body_start > 0 {
                (line.min(body_start), line.max(body_end))
            } else {
                (line, line)
            }
        }
        _ => (line, line),
    }
}

fn semantic_source_goto_target(source: &str, clicked_line: usize) -> (usize, Option<String>) {
    let source_lines = source.lines().collect::<Vec<_>>();
    if source_lines.is_empty() {
        return (1, None);
    }
    let safe_clicked = clicked_line.clamp(1, source_lines.len());
    let mut idx = safe_clicked.saturating_sub(1);
    loop {
        let trimmed = source_lines[idx].trim_start();
        if let Some(stripped) = trimmed.strip_prefix("fn ") {
            let name = parse_function_name(stripped);
            return (idx + 1, name);
        }
        if idx == 0 {
            break;
        }
        idx = idx.saturating_sub(1);
    }
    (safe_clicked, None)
}

fn semantic_function_range(source: &str, fn_start_line: usize) -> Option<(usize, usize)> {
    let source_lines = source.lines().collect::<Vec<_>>();
    if source_lines.is_empty() || fn_start_line == 0 || fn_start_line > source_lines.len() {
        return None;
    }
    let start_idx = fn_start_line - 1;
    if !source_lines[start_idx].trim_start().starts_with("fn ") {
        return None;
    }
    let mut depth: isize = 1;
    let mut idx = start_idx + 1;
    while idx < source_lines.len() {
        let trimmed = source_lines[idx].trim_start();
        if starts_block(trimmed) {
            depth += 1;
        }
        if trimmed == "end" {
            depth -= 1;
            if depth == 0 {
                return Some((fn_start_line, idx + 1));
            }
        }
        idx += 1;
    }
    None
}

fn starts_block(trimmed: &str) -> bool {
    trimmed.starts_with("fn ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("match ")
}

fn parse_function_name(after_fn: &str) -> Option<String> {
    let mut name = String::new();
    for ch in after_fn.chars() {
        if ch == '(' || ch.is_whitespace() {
            break;
        }
        name.push(ch);
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn trace_to_source_hints(source: &str, trace_events: &[String]) -> Vec<String> {
    let mut hints = Vec::new();
    let source_lines = source.lines().collect::<Vec<_>>();
    let mut seen = HashSet::new();

    for event in trace_events.iter().rev().take(60).rev() {
        let (builtin, id, _) = parse_trace_event(event);
        if builtin.is_empty() || id.is_empty() {
            hints.push(event.clone());
            continue;
        }

        if let Some((line_number, line_text)) = source_match_for_trace(&source_lines, &builtin, &id) {
            let rendered = format!("line {}: {}", line_number, line_text);
            if seen.insert(rendered.clone()) {
                hints.push(rendered);
            }
        } else {
            hints.push(event.clone());
        }
    }

    hints
}

fn trace_to_preview_exec_lines(source: &str, trace_events: &[String]) -> Vec<String> {
    let source_lines = source.lines().collect::<Vec<_>>();
    let mut rendered = Vec::new();
    for event in trace_events.iter().rev().take(12).rev() {
        let (builtin, id, result) = parse_trace_event(event);
        if builtin.is_empty() {
            continue;
        }
        let base = if id.is_empty() || id == "-" {
            format!("{} => {}", builtin, result)
        } else {
            format!("{}({}) => {}", builtin, id, result)
        };
        if let Some((line_number, _)) = source_match_for_trace(&source_lines, &builtin, &id) {
            rendered.push(format!("{} @ line {}", base, line_number));
        } else {
            rendered.push(base);
        }
    }
    rendered
}

fn parse_trace_event(event: &str) -> (String, String, String) {
    let mut builtin = String::new();
    let mut id = String::new();
    let mut result = String::new();
    for part in event.split_whitespace() {
        if let Some(value) = part.strip_prefix("builtin=") {
            builtin = value.to_string();
            continue;
        }
        if let Some(value) = part.strip_prefix("id=") {
            id = value.to_string();
            continue;
        }
        if let Some(value) = part.strip_prefix("result=") {
            result = value.to_string();
            continue;
        }
    }
    (builtin, id, result)
}

fn source_match_for_trace<'a>(
    source_lines: &[&'a str],
    builtin: &str,
    id: &str,
) -> Option<(usize, &'a str)> {
    let id_literal = format!("\"{}\"", id);
    for (idx, line) in source_lines.iter().enumerate() {
        if line.contains(builtin) && (id == "-" || id.is_empty() || line.contains(&id_literal)) {
            return Some((idx + 1, line.trim()));
        }
    }
    None
}

#[derive(Clone)]
struct InspectTarget {
    id: String,
    source_line: Option<usize>,
}

#[derive(Clone)]
struct SymbolEntry {
    kind: &'static str,
    name: String,
    path: PathBuf,
    line: usize,
}

#[derive(Clone)]
struct SymbolSelectionKey {
    kind: &'static str,
    name: String,
    path: PathBuf,
    line: usize,
}

#[derive(Clone)]
struct SymbolNavPoint {
    path: PathBuf,
    line: usize,
}

impl InspectTarget {
    fn source_label(&self) -> String {
        self.source_line
            .map(|line| format!("line {}", line))
            .unwrap_or_else(|| "no source match".to_string())
    }
}

fn selected_inspect_id<'a>(
    inspect_mode: bool,
    targets: &'a [InspectTarget],
    index: usize,
) -> Option<&'a str> {
    if !inspect_mode {
        return None;
    }
    targets.get(index).map(|target| target.id.as_str())
}

fn selected_inspect_line(
    inspect_mode: bool,
    targets: &[InspectTarget],
    index: usize,
) -> Option<usize> {
    if !inspect_mode {
        return None;
    }
    targets.get(index).and_then(|target| target.source_line)
}

fn symbol_selection_key(symbol: &SymbolEntry) -> SymbolSelectionKey {
    SymbolSelectionKey {
        kind: symbol.kind,
        name: symbol.name.clone(),
        path: symbol.path.clone(),
        line: symbol.line,
    }
}

fn selected_symbol_key(
    symbols: &[SymbolEntry],
    filtered_indices: &[usize],
    selected: usize,
) -> Option<SymbolSelectionKey> {
    filtered_indices
        .get(selected)
        .and_then(|pool_idx| symbols.get(*pool_idx))
        .map(symbol_selection_key)
}

fn restore_symbol_selection(
    symbols: &[SymbolEntry],
    filtered_indices: &[usize],
    previous: Option<&SymbolSelectionKey>,
    fallback: usize,
) -> usize {
    if filtered_indices.is_empty() {
        return 0;
    }
    if let Some(prev) = previous {
        if let Some(idx) = filtered_indices.iter().position(|pool_idx| {
            symbols.get(*pool_idx).is_some_and(|symbol| {
                symbol.kind == prev.kind
                    && symbol.name == prev.name
                    && symbol.path == prev.path
                    && symbol.line == prev.line
            })
        }) {
            return idx;
        }
    }
    fallback.min(filtered_indices.len().saturating_sub(1))
}

fn symbol_nav_point(path: &Path, line: usize) -> SymbolNavPoint {
    SymbolNavPoint {
        path: path.to_path_buf(),
        line: line.max(1),
    }
}

fn push_symbol_nav_history(
    history: &mut Vec<SymbolNavPoint>,
    history_index: &mut usize,
    point: SymbolNavPoint,
) {
    if history
        .get(*history_index)
        .is_some_and(|current| current.path == point.path && current.line == point.line)
    {
        return;
    }
    if *history_index + 1 < history.len() {
        history.truncate(*history_index + 1);
    }
    history.push(point);
    *history_index = history.len().saturating_sub(1);
}

fn infer_builtin_from_state_id(id: &str) -> Option<&'static str> {
    if id.starts_with("field.") {
        return Some("ui_textbox");
    }
    if id.starts_with("toggle.") {
        return Some("ui_toggle");
    }
    if id.starts_with("slider.") {
        return Some("ui_slider");
    }
    if id.starts_with("knob.") {
        return Some("ui_knob");
    }
    if id.starts_with("button.") {
        return Some("ui_button");
    }
    None
}

fn build_project_symbol_index(root: &Path, active_file: &Path) -> Vec<SymbolEntry> {
    let mut files = Vec::new();
    collect_lust_files(root, &mut files);
    if !files.iter().any(|path| path == active_file) {
        files.push(active_file.to_path_buf());
    }

    let mut symbols = Vec::new();
    for file in files {
        let Ok(source) = fs::read_to_string(&file) else {
            continue;
        };
        let Ok(decls) = parse_program(&source) else {
            symbols.extend(collect_symbols_from_text_fallback(&source, &file));
            continue;
        };
        for decl in decls {
            match decl {
                Decl::Fn(name, _, _, _, body) => {
                    let line = body.first().map(Stmt::line).unwrap_or(1);
                    symbols.push(SymbolEntry {
                        kind: "fn",
                        name,
                        path: file.clone(),
                        line,
                    });
                }
                Decl::Type(name, _) => symbols.push(SymbolEntry {
                    kind: "type",
                    name,
                    path: file.clone(),
                    line: 1,
                }),
                Decl::Enum(name, _) => symbols.push(SymbolEntry {
                    kind: "enum",
                    name,
                    path: file.clone(),
                    line: 1,
                }),
                _ => {}
            }
        }
    }

    symbols.sort_by(|a, b| {
        let pa = a.path.display().to_string();
        let pb = b.path.display().to_string();
        pa.cmp(&pb)
            .then(a.line.cmp(&b.line))
            .then(a.kind.cmp(&b.kind))
            .then(a.name.cmp(&b.name))
    });
    symbols.dedup_by(|a, b| {
        a.kind == b.kind && a.name == b.name && a.path == b.path && a.line == b.line
    });
    symbols
}

fn collect_symbols_from_text_fallback(source: &str, path: &Path) -> Vec<SymbolEntry> {
    let mut symbols = Vec::new();
    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_start();
        if line.starts_with("fn ") {
            if let Some(name) = parse_decl_name_from_line(line, "fn") {
                symbols.push(SymbolEntry {
                    kind: "fn",
                    name,
                    path: path.to_path_buf(),
                    line: line_no,
                });
            }
            continue;
        }
        if line.starts_with("type ") {
            if let Some(name) = parse_decl_name_from_line(line, "type") {
                symbols.push(SymbolEntry {
                    kind: "type",
                    name,
                    path: path.to_path_buf(),
                    line: line_no,
                });
            }
            continue;
        }
        if line.starts_with("enum ") {
            if let Some(name) = parse_decl_name_from_line(line, "enum") {
                symbols.push(SymbolEntry {
                    kind: "enum",
                    name,
                    path: path.to_path_buf(),
                    line: line_no,
                });
            }
        }
    }
    symbols
}

fn parse_decl_name_from_line(line: &str, keyword: &str) -> Option<String> {
    let suffix = line.strip_prefix(keyword)?.trim_start();
    let mut chars = suffix.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let mut name = String::new();
    name.push(first);
    for ch in chars {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    Some(name)
}

#[cfg(test)]
mod ide_symbol_tests {
    use super::{
        collect_symbols_from_text_fallback, parse_decl_name_from_line, push_symbol_nav_history,
        symbol_index_from_palette_line, symbol_nav_point, symbol_palette_cursor_line,
    };
    use std::path::Path;

    #[test]
    fn parse_decl_name_extracts_identifier() {
        assert_eq!(
            parse_decl_name_from_line("fn run_demo(x)", "fn"),
            Some("run_demo".to_string())
        );
        assert_eq!(
            parse_decl_name_from_line("type AppState", "type"),
            Some("AppState".to_string())
        );
        assert_eq!(
            parse_decl_name_from_line("enum Mode", "enum"),
            Some("Mode".to_string())
        );
    }

    #[test]
    fn parse_decl_name_rejects_invalid_identifiers() {
        assert_eq!(parse_decl_name_from_line("fn 9bad()", "fn"), None);
        assert_eq!(parse_decl_name_from_line("fn", "fn"), None);
        assert_eq!(parse_decl_name_from_line("let x = 1", "fn"), None);
    }

    #[test]
    fn text_fallback_collects_top_level_declarations() {
        let source = r#"
fn live_ui()
    print("ok")
end

fn broken(
type ThemeConfig
enum UiMode
"#;
        let symbols = collect_symbols_from_text_fallback(source, Path::new("demo.lust"));
        let names = symbols
            .iter()
            .map(|symbol| (symbol.kind, symbol.name.as_str(), symbol.line))
            .collect::<Vec<_>>();

        assert!(names.contains(&("fn", "live_ui", 2)));
        assert!(names.contains(&("fn", "broken", 6)));
        assert!(names.contains(&("type", "ThemeConfig", 7)));
        assert!(names.contains(&("enum", "UiMode", 8)));
    }

    #[test]
    fn symbol_palette_line_mapping_round_trips_within_page() {
        let page = 2usize;
        let page_size = 12usize;
        let selected = 29usize;
        let cursor_line = symbol_palette_cursor_line(selected, page, page_size);
        let mapped = symbol_index_from_palette_line(40, page, page_size, cursor_line);
        assert_eq!(mapped, Some(selected));
    }

    #[test]
    fn symbol_palette_line_mapping_ignores_header_and_footer_space() {
        assert_eq!(symbol_index_from_palette_line(25, 0, 12, 0), None);
        assert_eq!(symbol_index_from_palette_line(25, 0, 12, 2), None);
        assert_eq!(symbol_index_from_palette_line(25, 0, 12, 15), None);
    }

    #[test]
    fn symbol_history_push_truncates_forward_branch() {
        let mut history = vec![
            symbol_nav_point(Path::new("a.lust"), 1),
            symbol_nav_point(Path::new("b.lust"), 4),
            symbol_nav_point(Path::new("c.lust"), 7),
        ];
        let mut index = 1usize;
        push_symbol_nav_history(&mut history, &mut index, symbol_nav_point(Path::new("d.lust"), 2));
        assert_eq!(index, 2);
        assert_eq!(history.len(), 3);
        assert_eq!(history[2].path, Path::new("d.lust"));
        assert_eq!(history[2].line, 2);
    }

    #[test]
    fn symbol_history_push_skips_duplicate_current_point() {
        let mut history = vec![symbol_nav_point(Path::new("main.lust"), 12)];
        let mut index = 0usize;
        push_symbol_nav_history(
            &mut history,
            &mut index,
            symbol_nav_point(Path::new("main.lust"), 12),
        );
        assert_eq!(index, 0);
        assert_eq!(history.len(), 1);
    }
}

fn symbol_filter_indices(symbols: &[SymbolEntry], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return (0..symbols.len()).collect();
    }
    let mut scored = Vec::new();
    for (idx, symbol) in symbols.iter().enumerate() {
        let path = symbol.path.display().to_string();
        let combined = format!("{} {} {}", symbol.kind, symbol.name, path);
        let mut best: Option<i64> = None;
        for (candidate, weight) in [
            (symbol.name.as_str(), 1000i64),
            (symbol.kind, 400i64),
            (path.as_str(), 150i64),
            (combined.as_str(), 50i64),
        ] {
            if let Some(score) = fuzzy_match_score(&needle, &candidate.to_lowercase()) {
                let weighted = score + weight;
                best = Some(best.map_or(weighted, |current| current.max(weighted)));
            }
        }
        if let Some(score) = best {
            scored.push((idx, score));
        }
    }
    scored.sort_by(|(idx_a, score_a), (idx_b, score_b)| {
        score_b.cmp(score_a).then(idx_a.cmp(idx_b))
    });
    scored.into_iter().map(|(idx, _)| idx).collect()
}

fn build_symbol_palette_lines(
    symbols: &[SymbolEntry],
    filtered_indices: &[usize],
    selected: usize,
    page: usize,
    page_size: usize,
    query: &str,
) -> Vec<String> {
    if filtered_indices.is_empty() {
        return vec![format!("  no matches for '{}'", query)];
    }
    let symbol_row_start_line = symbol_palette_symbol_row_start_line();
    let max_page = filtered_indices.len().saturating_sub(1) / page_size.max(1);
    let page_idx = page.min(max_page);
    let start = page_idx * page_size.max(1);
    let end = (start + page_size.max(1)).min(filtered_indices.len());
    let mut lines = Vec::new();
    lines.push(format!(
        "  filter='{}' matches={} page {}/{}",
        query,
        filtered_indices.len(),
        page_idx + 1,
        max_page + 1
    ));
    if let Some(&selected_pool_idx) = filtered_indices.get(selected) {
        if let Some(selected_symbol) = symbols.get(selected_pool_idx) {
            lines.push(format!(
                "  peek: {} {} {}:{}",
                selected_symbol.kind,
                truncate_symbol_name(&selected_symbol.name, 24),
                selected_symbol.path.display(),
                selected_symbol.line
            ));
            lines.push(format!(
                "  code: {}",
                symbol_peek_line(selected_symbol)
                    .unwrap_or_else(|| "<source unavailable>".to_string())
            ));
        }
    }
    while lines.len() < symbol_row_start_line {
        lines.push("".to_string());
    }
    for abs_pos in start..end {
        let idx = filtered_indices[abs_pos];
        let Some(symbol) = symbols.get(idx) else {
            continue;
        };
        let marker = if abs_pos == selected { ">" } else { " " };
        let path = symbol.path.display().to_string();
        lines.push(format!(
            "{} {:>4}. {:<6} {:<24} {}:{}",
            marker,
            abs_pos + 1,
            symbol.kind,
            truncate_symbol_name(&symbol.name, 24),
            path,
            symbol.line
        ));
    }
    lines.push("  keys: up/down move, pgup/pgdn page, enter/click open, esc cancel".to_string());
    lines
}

fn symbol_palette_cursor_line(selected: usize, page: usize, page_size: usize) -> usize {
    let page_size = page_size.max(1);
    let page_start = page * page_size;
    let row_in_page = selected.saturating_sub(page_start);
    symbol_palette_symbol_row_start_line() + row_in_page.min(page_size.saturating_sub(1))
}

fn symbol_index_from_palette_line(
    filtered_len: usize,
    page: usize,
    page_size: usize,
    line_index: usize,
) -> Option<usize> {
    let page_size = page_size.max(1);
    if filtered_len == 0 || line_index == 0 {
        return None;
    }
    let start_line = symbol_palette_symbol_row_start_line();
    if line_index < start_line {
        return None;
    }
    let max_page = filtered_len.saturating_sub(1) / page_size;
    let page_idx = page.min(max_page);
    let page_start = page_idx * page_size;
    let page_end = (page_start + page_size).min(filtered_len);
    let row_in_page = line_index.saturating_sub(start_line);
    let absolute = page_start + row_in_page;
    if absolute < page_end {
        Some(absolute)
    } else {
        None
    }
}

fn symbol_palette_symbol_row_start_line() -> usize {
    3
}

fn symbol_peek_line(symbol: &SymbolEntry) -> Option<String> {
    let source = fs::read_to_string(&symbol.path).ok()?;
    let line = source
        .lines()
        .nth(symbol.line.saturating_sub(1))
        .map(str::trim)?;
    if line.is_empty() {
        return Some("<blank>".to_string());
    }
    Some(truncate_symbol_name(line, 72))
}

fn truncate_symbol_name(name: &str, max: usize) -> String {
    let chars = name.chars().collect::<Vec<_>>();
    if chars.len() <= max {
        return name.to_string();
    }
    chars.into_iter().take(max.saturating_sub(1)).collect::<String>() + "…"
}

fn fuzzy_match_score(needle: &str, hay: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    if hay == needle {
        return Some(20_000);
    }
    if hay.starts_with(needle) {
        return Some(15_000 - needle.len() as i64);
    }
    if let Some(pos) = hay.find(needle) {
        return Some(10_000 - pos as i64);
    }

    let mut score = 0i64;
    let mut hay_idx = 0usize;
    let hay_chars = hay.chars().collect::<Vec<_>>();
    let mut last_match = None::<usize>;
    for ch in needle.chars() {
        let mut found = None;
        while hay_idx < hay_chars.len() {
            if hay_chars[hay_idx] == ch {
                found = Some(hay_idx);
                hay_idx += 1;
                break;
            }
            hay_idx += 1;
        }
        let pos = found?;
        score += 100;
        if let Some(prev) = last_match {
            if pos == prev + 1 {
                score += 30;
            }
        }
        if pos == 0 {
            score += 20;
        }
        last_match = Some(pos);
    }
    Some(score)
}

fn collect_lust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if name == "target" || name == ".git" {
                continue;
            }
            collect_lust_files(&path, out);
            continue;
        }
        if path.extension().and_then(|v| v.to_str()) == Some("lust") {
            out.push(path);
        }
    }
}

fn apply_symbol_navigation(
    symbol: &SymbolEntry,
    active_file: &mut PathBuf,
    current_source: &mut String,
    ui_state: &mut HashMap<String, Value>,
    last_observed_modified: &mut SystemTime,
    shell: &mut ide_shell::IdeShell,
) -> bool {
    let path_label = symbol.path.display().to_string();
    shell.push_diag(format!(
        "[symbol] {} {} @ {}:{}",
        symbol.kind, symbol.name, path_label, symbol.line
    ));
    if symbol.path == *active_file {
        ui_state.insert(
            "editor.host.goto_line".to_string(),
            Value::Number(symbol.line as f64),
        );
        shell.set_focus_line(Some(symbol.line));
        shell.set_path_label(path_label);
        return true;
    }

    let source = fs::read_to_string(&symbol.path).unwrap_or_default();
    *current_source = source.clone();
    shell.set_source(&source);
    shell.set_path_label(path_label);
    shell.set_focus_line(Some(symbol.line));
    *active_file = symbol.path.clone();
    if let Ok(modified) = fs::metadata(active_file).and_then(|meta| meta.modified()) {
        *last_observed_modified = modified;
    }
    ui_state.insert(
        "editor.host.goto_line".to_string(),
        Value::Number(symbol.line as f64),
    );
    true
}

fn apply_symbol_nav_point(
    point: &SymbolNavPoint,
    active_file: &mut PathBuf,
    current_source: &mut String,
    ui_state: &mut HashMap<String, Value>,
    last_observed_modified: &mut SystemTime,
    shell: &mut ide_shell::IdeShell,
) -> bool {
    let path_label = point.path.display().to_string();
    shell.push_diag(format!("[symbol] history @ {}:{}", path_label, point.line));

    if point.path == *active_file {
        ui_state.insert(
            "editor.host.goto_line".to_string(),
            Value::Number(point.line as f64),
        );
        shell.set_focus_line(Some(point.line));
        shell.set_path_label(path_label);
        return true;
    }

    let source = fs::read_to_string(&point.path).unwrap_or_default();
    *current_source = source.clone();
    shell.set_source(&source);
    shell.set_path_label(path_label);
    shell.set_focus_line(Some(point.line));
    *active_file = point.path.clone();
    if let Ok(modified) = fs::metadata(active_file).and_then(|meta| meta.modified()) {
        *last_observed_modified = modified;
    }
    ui_state.insert(
        "editor.host.goto_line".to_string(),
        Value::Number(point.line as f64),
    );
    true
}

fn build_inspect_targets(state: &HashMap<String, Value>, source: &str) -> Vec<InspectTarget> {
    let source_lines = source.lines().collect::<Vec<_>>();
    let mut ids = state
        .keys()
        .filter(|key| infer_builtin_from_state_id(key).is_some())
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();

    let mut targets = Vec::new();
    for id in ids {
        let source_line = infer_builtin_from_state_id(&id)
            .and_then(|builtin| source_match_for_trace(&source_lines, builtin, &id))
            .map(|(line_number, _)| line_number);
        targets.push(InspectTarget { id, source_line });
    }
    targets
}

fn extract_theme_from_ui_state(
    state: &HashMap<String, Value>,
) -> (String, HashMap<String, String>) {
    let theme_name = state
        .get("__theme")
        .map(Value::as_string)
        .unwrap_or_else(|| "default".to_string());

    let mut settings = HashMap::new();
    for (key, value) in state {
        if let Some(suffix) = key.strip_prefix("theme.") {
            settings.insert(suffix.to_string(), value.as_string());
        }
    }
    (theme_name, settings)
}

fn build_ui_preview_lines(
    output_lines: &[String],
    state: &HashMap<String, Value>,
    exec_preview_lines: &[String],
    selected_id: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(title) = state.get("app.title") {
        lines.push(format!("App: {}", title.as_string()));
        lines.push(String::new());
    }

    let mut sections = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("widget.section.")
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    sections.sort_by(|a, b| a.0.cmp(&b.0));

    let mut labels = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("widget.label.")
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    labels.sort_by(|a, b| a.0.cmp(&b.0));

    let mut buttons = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("widget.button.")
                .and_then(|suffix| suffix.strip_suffix(".label"))
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    buttons.sort_by(|a, b| a.0.cmp(&b.0));

    let mut values = state
        .iter()
        .filter_map(|(key, value)| {
            if let Some(id) = key.strip_prefix("field.") {
                return Some((id.to_string(), key.clone(), value.as_string()));
            }
            if let Some(id) = key.strip_prefix("toggle.") {
                return Some((id.to_string(), key.clone(), value.as_string()));
            }
            if let Some(id) = key.strip_prefix("slider.") {
                return Some((id.to_string(), key.clone(), value.as_string()));
            }
            if let Some(id) = key.strip_prefix("knob.") {
                return Some((id.to_string(), key.clone(), value.as_string()));
            }
            if let Some(id) = key.strip_prefix("button.") {
                return Some((id.to_string(), key.clone(), value.as_string()));
            }
            None
        })
        .collect::<Vec<_>>();
    values.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hints = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("widget.hint.")
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    hints.sort_by(|a, b| a.0.cmp(&b.0));

    let mut layout_groups = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("layout.group.")
                .and_then(|suffix| suffix.strip_suffix(".title"))
                .map(|id| (id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    layout_groups.sort_by(|a, b| a.0.cmp(&b.0));

    let mut layout_rows = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("layout.row.").and_then(|suffix| {
                let mut parts = suffix.splitn(2, '.');
                let group = parts.next()?;
                let row = parts.next()?;
                Some((group.to_string(), row.to_string(), value.as_string()))
            })
        })
        .collect::<Vec<_>>();
    layout_rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut layout_columns = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("layout.column.").and_then(|suffix| {
                let mut parts = suffix.splitn(2, '.');
                let group = parts.next()?;
                let column = parts.next()?;
                Some((group.to_string(), column.to_string(), value.as_string()))
            })
        })
        .collect::<Vec<_>>();
    layout_columns.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut layout_binds = state
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("layout.bind.")
                .map(|widget_id| (widget_id.to_string(), value.as_string()))
        })
        .collect::<Vec<_>>();
    layout_binds.sort_by(|a, b| a.0.cmp(&b.0));

    if !sections.is_empty() || !labels.is_empty() || !buttons.is_empty() || !values.is_empty() || !hints.is_empty() {
        lines.push("UI State".to_string());
        for (id, title) in &sections {
            lines.push(format!("  section:{} = {}", id, title));
        }
        for (id, text) in &labels {
            lines.push(format!("  label:{} = {}", id, text));
        }
        for (id, text) in &buttons {
            lines.push(format!("  button:{} = {}", id, text));
        }
        for (id, full_id, value) in &values {
            let marker = if selected_id == Some(full_id.as_str()) {
                ">"
            } else {
                " "
            };
            lines.push(format!(" {} value:{} = {}", marker, id, value));
        }
        for (id, hint) in &hints {
            lines.push(format!("  hint:{} = {}", id, hint));
        }
        lines.push(String::new());
    }

    if !layout_groups.is_empty()
        || !layout_rows.is_empty()
        || !layout_columns.is_empty()
        || !layout_binds.is_empty()
    {
        lines.push("Layout".to_string());
        for (id, title) in &layout_groups {
            lines.push(format!("  group:{} = {}", id, title));
        }
        for (group, row, label) in &layout_rows {
            lines.push(format!("  row:{}/{} = {}", group, row, label));
        }
        for (group, column, label) in &layout_columns {
            lines.push(format!("  column:{}/{} = {}", group, column, label));
        }
        for (widget_id, slot) in &layout_binds {
            lines.push(format!("  bind:{} -> {}", widget_id, slot));
        }
        lines.push(String::new());
    }

    if !exec_preview_lines.is_empty() {
        lines.push("Recent Executed UI Calls".to_string());
        lines.extend(exec_preview_lines.iter().map(|line| format!("  {}", line)));
        lines.push(String::new());
    }

    lines.push("Program Output".to_string());
    lines.extend(output_lines.iter().cloned());
    lines
}

fn run_file_capture(path: &str, args: Vec<String>) -> String {
    run_vm_capture(path, args)
}

fn format_memory_snapshot(memory: &VmMemorySnapshot) -> String {
    format!(
        "stack {}/{} (peak {}) | globals {}/{} | ui {}/{} (peak {}) | list_allocs {} map_allocs {} struct_allocs {} | list_push {} map_set {}",
        memory.stack_len,
        memory.max_stack,
        memory.stack_peak,
        memory.globals_len,
        memory.max_globals,
        memory.ui_state_len,
        memory.max_ui_state_entries,
        memory.ui_state_peak,
        memory.list_allocations,
        memory.map_allocations,
        memory.struct_allocations,
        memory.list_push_ops,
        memory.map_insert_ops
    )
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
        Commands::Ide {
            path,
            args,
            interval_ms,
            debounce_ms,
            show_exec,
        } => {
            if let Err(err) = run_ide_watch(path, args.clone(), *interval_ms, *debounce_ms, *show_exec) {
                println!("{}", err);
            }
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
