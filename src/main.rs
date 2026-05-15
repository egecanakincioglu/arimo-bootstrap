/*
Arimo Lang - A modern programming language and compiler
Copyright (C) 2026 Egecan Akıncıoğlu

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

mod lexer;
mod ast;
mod parser;
mod typechecker;
mod borrow;
mod codegen;

use std::{collections::{HashMap, HashSet, VecDeque}, env, fs, path::{Path, PathBuf}, process};
use lexer::Lexer;
use parser::Parser;
use typechecker::TypeChecker;
use borrow::BorrowChecker;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("arc v0.4.0");
        eprintln!("Usage: arc <file.arm> [file2.arm ...] [--emit-ir] [-c] [-O2]");
        process::exit(1);
    }

    let emit_ir  = args.contains(&"--emit-ir".to_string());
    let only_obj = args.contains(&"-c".to_string());
    let optimize = args.contains(&"-O2".to_string()) || args.contains(&"-O3".to_string());

    let arm_files: Vec<&str> = args.iter()
        .skip(1)
        .filter(|a| a.ends_with(".arm"))
        .map(|s| s.as_str())
        .collect();

    if arm_files.is_empty() {
        eprintln!("error: no .arm source file provided");
        process::exit(1);
    }

    // Discover all modules starting from entry files, following imports
    let all_paths = discover_modules(&arm_files);

    // Parse all discovered files
    let mut modules: Vec<(String, ast::Module)> = Vec::new();
    for path in &all_paths {
        let source = fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error: could not read '{}': {}", path, e);
            process::exit(1);
        });
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize();
        println!("arc: parsing '{}'", path);
        let mut parser = Parser::new(tokens);
        match parser.parse() {
            Ok(m) => {
                println!("arc: parse OK — module '{}'", m.path);
                modules.push((path.clone(), m));
            }
            Err(e) => {
                eprintln!("arc: parse error in '{}' at {}:{} — {}", path, e.line, e.col, e.message);
                process::exit(1);
            }
        }
    }

    if modules.len() > 1 {
        println!("");
        println!("arc: {} modules loaded", modules.len());
    } else {
        let (_, m) = &modules[0];
        println!("");
        println!("Module   : {}", m.path);
        println!("Imports  : {:?}", m.imports);
        println!("Items    : {}", m.items.len());
        println!("");
        print_module_summary(m);
    }

    // Topological sort by import dependencies
    let sorted_indices = topological_sort(&modules);

    // Type check all modules with a shared checker (dependencies first)
    println!("");
    let mut tc = TypeChecker::new();
    for &i in &sorted_indices {
        tc.check(&modules[i].1);
    }
    for warn in &tc.warnings {
        println!("arc: {}", warn);
    }
    if tc.errors.is_empty() {
        println!("arc: type check OK");
    } else {
        for err in &tc.errors {
            eprintln!("arc: {}", err);
        }
        process::exit(1);
    }

    // Borrow check each module independently
    let mut total_drops = 0;
    for &i in &sorted_indices {
        let mut bc = BorrowChecker::new();
        let errors = bc.check(&modules[i].1);
        if !errors.is_empty() {
            for err in errors {
                eprintln!("arc: {}", err);
            }
            process::exit(1);
        }
        total_drops += bc.drops.len();
    }
    println!("arc: borrow check OK");
    println!("arc: drop schedule — {} scope(s) tracked", total_drops);

    // Determine output name from entry file
    let entry_path = arm_files[0];
    let stem = Path::new(entry_path).file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let exe_path = format!("{}.exe", stem);

    // Sorted module refs for codegen
    let sorted_modules: Vec<&ast::Module> = sorted_indices.iter()
        .map(|&i| &modules[i].1)
        .collect();

    if emit_ir {
        match codegen::emit_ir_multi(&sorted_modules, stem) {
            Ok(text) => { println!("\n; === LLVM IR ===\n{}", text); }
            Err(e)   => { eprintln!("arc: ir error — {}", e); process::exit(1); }
        }
        return;
    }

    let obj_out = if only_obj {
        format!("{}.o", stem)
    } else {
        std::env::temp_dir()
            .join(format!("{}.o", stem))
            .to_string_lossy()
            .to_string()
    };

    print!("arc: compiling  ...");
    match codegen::compile_to_object_multi(&sorted_modules, stem, Path::new(&obj_out), optimize) {
        Ok(()) => {
            if only_obj {
                println!(" OK");
                println!("arc: → {}", obj_out);
                return;
            }
            print!(" linking ...");
            let linker_status = std::process::Command::new("gcc")
                .args([&obj_out, "-o", &exe_path, "-lm"])
                .status();
            let _ = fs::remove_file(&obj_out);
            match linker_status {
                Ok(s) if s.success() => {
                    println!(" OK");
                    println!("arc: → {}", exe_path);
                }
                Ok(s) => {
                    eprintln!("\narc: linker failed (exit {})", s);
                    process::exit(1);
                }
                Err(e) => {
                    eprintln!("\narc: linker not found — {}", e);
                    process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("\narc: codegen error — {}", e);
            process::exit(1);
        }
    }
}

/// Resolve `arimo.fs.File` → `<base_dir>/arimo/fs/File.arm`
fn resolve_import(import_str: &str, base_dir: &Path) -> Option<PathBuf> {
    if import_str.ends_with(".*") { return None; }
    let rel = import_str.replace('.', "/") + ".arm";
    let full = base_dir.join(&rel);
    if full.exists() { Some(full) } else { None }
}

/// BFS discovery: start from entry files, follow imports, collect unique paths in order.
fn discover_modules(entry_files: &[&str]) -> Vec<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    for f in entry_files {
        let p = PathBuf::from(f);
        let canonical = fs::canonicalize(&p).unwrap_or(p.clone());
        let key = canonical.to_string_lossy().to_string();
        if visited.insert(key) {
            queue.push_back(canonical);
            order.push(p.to_string_lossy().to_string());
        }
    }

    while let Some(path) = queue.pop_front() {
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let module = match parser.parse() {
            Ok(m) => m,
            Err(_) => continue,
        };
        for imp in &module.imports {
            if let Some(dep_path) = resolve_import(imp, base_dir) {
                let canonical = fs::canonicalize(&dep_path).unwrap_or(dep_path.clone());
                let key = canonical.to_string_lossy().to_string();
                if visited.insert(key) {
                    order.push(dep_path.to_string_lossy().to_string());
                    queue.push_back(canonical);
                }
            }
        }
    }

    order
}

/// DFS topological sort. Returns indices into `modules` in dependency-first order.
fn topological_sort(modules: &[(String, ast::Module)]) -> Vec<usize> {
    // Build path → index map
    let path_to_idx: HashMap<String, usize> = modules.iter()
        .enumerate()
        .map(|(i, (p, _))| {
            let canonical = fs::canonicalize(p).unwrap_or(PathBuf::from(p));
            (canonical.to_string_lossy().to_string(), i)
        })
        .collect();

    // Build adjacency: index → dependency indices
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); modules.len()];
    for (i, (path, module)) in modules.iter().enumerate() {
        let base_dir = Path::new(path).parent().unwrap_or(Path::new("."));
        for imp in &module.imports {
            if let Some(dep_path) = resolve_import(imp, base_dir) {
                let canonical = fs::canonicalize(&dep_path).unwrap_or(dep_path.clone());
                let key = canonical.to_string_lossy().to_string();
                if let Some(&j) = path_to_idx.get(&key) {
                    deps[i].push(j);
                }
            }
        }
    }

    // DFS post-order
    let mut visited = vec![false; modules.len()];
    let mut result: Vec<usize> = Vec::new();

    fn dfs(node: usize, deps: &[Vec<usize>], visited: &mut Vec<bool>, result: &mut Vec<usize>) {
        if visited[node] { return; }
        visited[node] = true;
        for &dep in &deps[node] {
            dfs(dep, deps, visited, result);
        }
        result.push(node);
    }

    for i in 0..modules.len() {
        dfs(i, &deps, &mut visited, &mut result);
    }

    result
}

fn print_module_summary(m: &ast::Module) {
    for item in &m.items {
        match item {
            ast::Item::Class(c) => {
                let manual_tag = if c.manual { " [@manual]" } else { "" };
                println!("  class {}{} {{", c.name, manual_tag);
                println!("    fields      : {}", c.fields.len());
                println!("    constructor : {}", c.constructor.is_some());
                println!("    methods     : {}", c.methods.len());
                for method in &c.methods {
                    let ret = match &method.return_ty {
                        Some(t) => format!("{:?}", t),
                        None    => "—".to_string(),
                    };
                    println!("      {} {}() : {}",
                        if method.static_ { "static" } else { "      " },
                        method.name, ret
                    );
                }
                println!("  }}");
            }
            ast::Item::Interface(i) => {
                println!("  interface {} ({} methods)", i.name, i.methods.len());
            }
            ast::Item::Enum(e) => {
                println!("  enum {} {:?}", e.name, e.variants);
            }
            ast::Item::Exception(e) => {
                println!("  exception {} extends {}", e.name, e.extends);
            }
            ast::Item::Struct(s) => {
                println!("  struct {} {{", s.name);
                println!("    fields  : {}", s.fields.len());
                println!("    methods : {}", s.methods.len());
                println!("  }}");
            }
            ast::Item::TypeAlias(a) => {
                println!("  type {} = {:?}", a.name, a.ty);
            }
            ast::Item::Union(u) => {
                println!("  union {} ({} fields)", u.name, u.fields.len());
            }
            ast::Item::Extern(e) => {
                println!("  extern \"{}\" ({} decls)", e.abi, e.decls.len());
            }
            ast::Item::Extension(e) => {
                println!("  extend {} ({} methods)", e.target, e.methods.len());
            }
        }
    }
}
