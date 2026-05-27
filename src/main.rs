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

// ─── arc.toml config ──────────────────────────────────────────────────────────

#[derive(Default, Debug)]
struct Author {
    name  : String,
    email : String,
}

struct ArcConfig {
    // [project]
    name        : String,
    version     : String,
    description : String,
    entry       : String,
    license     : String,
    readme      : String,
    keywords    : Vec<String>,
    homepage    : String,
    repository  : String,
    authors     : Vec<Author>,
    // [build]
    target      : String,
    // [profiles.release] / [profiles.dev]
    optimize_release : bool,
    optimize_dev     : bool,
    // [stdlib]
    stdlib      : Vec<String>,
    // [dependencies]
    dependencies     : Vec<(String, String)>,
    dev_dependencies : Vec<(String, String)>,
    // [scripts]
    scripts     : Vec<(String, String)>,
}

impl Default for ArcConfig {
    fn default() -> Self {
        ArcConfig {
            name             : "project".to_string(),
            version          : "0.1.0".to_string(),
            description      : String::new(),
            entry            : "Main.arm".to_string(),
            license          : "MIT".to_string(),
            readme           : "README.md".to_string(),
            keywords         : Vec::new(),
            homepage         : String::new(),
            repository       : String::new(),
            authors          : Vec::new(),
            target           : "x86_64-pc-windows-gnu".to_string(),
            optimize_release : true,
            optimize_dev     : false,
            stdlib           : Vec::new(),
            dependencies     : Vec::new(),
            dev_dependencies : Vec::new(),
            scripts          : Vec::new(),
        }
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
    || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len()-1].to_string()
    } else {
        s.to_string()
    }
}

fn parse_toml_array(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner.split(',')
        .map(|item| unquote(item.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Full arc.toml parser.
fn parse_arc_toml(content: &str) -> ArcConfig {
    let mut cfg = ArcConfig::default();
    let mut section = String::new();
    let mut cur_author: Option<Author> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('#') || line.is_empty() { continue; }

        // Array-of-tables header: [[project.authors]]
        if line.starts_with("[[") && line.ends_with("]]") {
            // Flush pending author
            if let Some(a) = cur_author.take() {
                cfg.authors.push(a);
            }
            let inner = line[2..line.len()-2].trim();
            section = inner.to_string();
            if inner == "project.authors" {
                cur_author = Some(Author::default());
            }
            continue;
        }

        // Table header: [section]
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(a) = cur_author.take() {
                cfg.authors.push(a);
            }
            section = line[1..line.len()-1].trim().to_string();
            continue;
        }

        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let val = line[eq+1..].trim();

        // Skip commented-out lines (value starts with #)
        if val.starts_with('#') { continue; }

        match section.as_str() {
            "project" => match key {
                "name"        => cfg.name        = unquote(val),
                "version"     => cfg.version      = unquote(val),
                "description" => cfg.description  = unquote(val),
                "entry"       => cfg.entry        = unquote(val),
                "license"     => cfg.license      = unquote(val),
                "readme"      => cfg.readme       = unquote(val),
                "homepage"    => cfg.homepage     = unquote(val),
                "repository"  => cfg.repository   = unquote(val),
                "keywords"    => cfg.keywords     = parse_toml_array(val),
                _ => {}
            },
            "project.authors" => {
                if let Some(ref mut a) = cur_author {
                    match key {
                        "name"  => a.name  = unquote(val),
                        "email" => a.email = unquote(val),
                        _ => {}
                    }
                }
            },
            "build" => match key {
                "target" => cfg.target = unquote(val),
                _ => {}
            },
            "profiles.release" => match key {
                "optimize" => cfg.optimize_release = val == "true",
                _ => {}
            },
            "profiles.dev" => match key {
                "optimize" => cfg.optimize_dev = val == "true",
                _ => {}
            },
            "stdlib" => match key {
                "include" => cfg.stdlib = parse_toml_array(val),
                _ => {}
            },
            "dependencies" => {
                let pkg = key.to_string();
                let ver = unquote(val);
                if !pkg.is_empty() && !ver.is_empty() {
                    cfg.dependencies.push((pkg, ver));
                }
            },
            "dev-dependencies" => {
                let pkg = key.to_string();
                let ver = unquote(val);
                if !pkg.is_empty() && !ver.is_empty() {
                    cfg.dev_dependencies.push((pkg, ver));
                }
            },
            "scripts" => {
                cfg.scripts.push((key.to_string(), unquote(val)));
            },
            _ => {}
        }
    }

    // Flush last pending author
    if let Some(a) = cur_author {
        cfg.authors.push(a);
    }

    cfg
}

// ─── stdlib discovery ─────────────────────────────────────────────────────────

/// Find the stdlib/ dir adjacent to arc.exe.
fn find_stdlib_dir() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let stdlib = exe_dir.join("stdlib");
    if stdlib.exists() { Some(stdlib) } else { None }
}

/// Collect all .arm files under stdlib/ for the given package (e.g. "arimo.fs").
fn stdlib_files_for(stdlib_dir: &Path, pkg: &str) -> Vec<PathBuf> {
    let rel = pkg.replace('.', "/");
    let pkg_dir = stdlib_dir.join(&rel);
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(&pkg_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("arm") {
                files.push(p);
            }
        }
    }
    files
}

// ─── import resolution ────────────────────────────────────────────────────────

/// Resolve `arimo.fs.File` → path.
/// Search order:
///   1. Relative to the importing file's directory
///   2. Relative to the project root (arc.toml location)
///   3. From the stdlib directory (arc.exe/stdlib/)
fn resolve_import(
    import_str  : &str,
    base_dir    : &Path,
    project_root: Option<&Path>,
    stdlib_dir  : Option<&Path>,
) -> Option<PathBuf> {
    if import_str.ends_with(".*") { return None; }
    let rel = import_str.replace('.', "/") + ".arm";

    // 1. Relative to the importing file's directory
    let local = base_dir.join(&rel);
    if local.exists() { return Some(local); }

    // 2. Relative to project root (enables `import myproject.Lexer` → `myproject/Lexer.arm`)
    if let Some(root) = project_root {
        let from_root = root.join(&rel);
        if from_root.exists() { return Some(from_root); }
    }

    // 3. From stdlib
    if let Some(stdlib) = stdlib_dir {
        let from_stdlib = stdlib.join(&rel);
        if from_stdlib.exists() { return Some(from_stdlib); }
    }

    // 4. Flat project: try just the class filename in project root
    //    e.g. `import myapp.Greeter` → `Greeter.arm` in project root
    if let Some(root) = project_root {
        let class_name = import_str.split('.').last().unwrap_or(import_str);
        let flat = root.join(format!("{}.arm", class_name));
        if flat.exists() { return Some(flat); }
    }

    None
}

// ─── BFS + topo sort ──────────────────────────────────────────────────────────

fn discover_modules(entry_files: &[&str], project_root: Option<&Path>, stdlib_dir: Option<&Path>) -> Vec<String> {
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
            if let Some(dep_path) = resolve_import(imp, base_dir, project_root, stdlib_dir) {
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

fn topological_sort(modules: &[(String, ast::Module)], project_root: Option<&Path>, stdlib_dir: Option<&Path>) -> Vec<usize> {
    let path_to_idx: HashMap<String, usize> = modules.iter()
        .enumerate()
        .map(|(i, (p, _))| {
            let canonical = fs::canonicalize(p).unwrap_or(PathBuf::from(p));
            (canonical.to_string_lossy().to_string(), i)
        })
        .collect();

    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); modules.len()];
    for (i, (path, module)) in modules.iter().enumerate() {
        let base_dir = Path::new(path).parent().unwrap_or(Path::new("."));
        for imp in &module.imports {
            if let Some(dep_path) = resolve_import(imp, base_dir, project_root, stdlib_dir) {
                let canonical = fs::canonicalize(&dep_path).unwrap_or(dep_path.clone());
                let key = canonical.to_string_lossy().to_string();
                if let Some(&j) = path_to_idx.get(&key) {
                    deps[i].push(j);
                }
            }
        }
    }

    let mut visited = vec![false; modules.len()];
    let mut result: Vec<usize> = Vec::new();

    fn dfs(node: usize, deps: &[Vec<usize>], visited: &mut Vec<bool>, result: &mut Vec<usize>) {
        if visited[node] { return; }
        visited[node] = true;
        for &dep in &deps[node] { dfs(dep, deps, visited, result); }
        result.push(node);
    }

    for i in 0..modules.len() { dfs(i, &deps, &mut visited, &mut result); }
    result
}

// ─── compile pipeline ─────────────────────────────────────────────────────────

struct CompileOptions<'a> {
    entry_files  : Vec<String>,
    project_root : Option<&'a Path>,
    stdlib_dir   : Option<&'a Path>,
    emit_ir      : bool,
    only_obj     : bool,
    optimize     : bool,
    out_stem     : Option<String>,
    check_only   : bool,
}

fn run_pipeline(opts: CompileOptions) -> i32 {
    let entry_refs: Vec<&str> = opts.entry_files.iter().map(|s| s.as_str()).collect();
    let all_paths = discover_modules(&entry_refs, opts.project_root, opts.stdlib_dir);

    let mut modules: Vec<(String, ast::Module)> = Vec::new();
    for path in &all_paths {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("arc: error: could not read '{}': {}", path, e);
                return 1;
            }
        };
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
                return 1;
            }
        }
    }

    if modules.len() > 1 {
        println!("\narc: {} modules loaded", modules.len());
    } else if modules.len() == 1 {
        let (_, m) = &modules[0];
        println!("\nModule   : {}", m.path);
        println!("Imports  : {:?}", m.imports);
        println!("Items    : {}", m.items.len());
        println!();
        print_module_summary(m);
    } else {
        eprintln!("arc: error: no modules to compile");
        return 1;
    }

    let sorted_indices = topological_sort(&modules, opts.project_root, opts.stdlib_dir);

    println!();
    let mut tc = TypeChecker::new();
    for &i in &sorted_indices { tc.register(&modules[i].1); }
    for &i in &sorted_indices { tc.check(&modules[i].1); }
    for warn in &tc.warnings { println!("arc: {}", warn); }
    if tc.errors.is_empty() {
        println!("arc: type check OK");
    } else {
        for err in &tc.errors { eprintln!("arc: {}", err); }
        return 1;
    }

    let mut total_drops = 0;
    for &i in &sorted_indices {
        let mut bc = BorrowChecker::new();
        let errors = bc.check(&modules[i].1);
        if !errors.is_empty() {
            for err in errors { eprintln!("arc: {}", err); }
            return 1;
        }
        total_drops += bc.drops.len();
    }
    println!("arc: borrow check OK");
    println!("arc: drop schedule — {} scope(s) tracked", total_drops);

    if opts.check_only { return 0; }

    let stem = opts.out_stem.as_deref().unwrap_or_else(|| {
        entry_refs[0].trim_end_matches(".arm").rsplit(['/', '\\']).next().unwrap_or("output")
    });
    let stem_owned = stem.to_string();

    let sorted_modules: Vec<&ast::Module> = sorted_indices.iter()
        .map(|&i| &modules[i].1)
        .collect();

    if opts.emit_ir {
        match codegen::emit_ir_multi(&sorted_modules, &stem_owned) {
            Ok(text) => { println!("\n; === LLVM IR ===\n{}", text); }
            Err(e)   => { eprintln!("arc: ir error — {}", e); return 1; }
        }
        return 0;
    }

    let obj_out = if opts.only_obj {
        format!("{}.o", stem_owned)
    } else {
        std::env::temp_dir()
            .join(format!("{}.o", stem_owned))
            .to_string_lossy()
            .to_string()
    };

    print!("arc: compiling  ...");
    match codegen::compile_to_object_multi(&sorted_modules, &stem_owned, Path::new(&obj_out), opts.optimize) {
        Ok(()) => {
            if opts.only_obj {
                println!(" OK\narc: → {}", obj_out);
                return 0;
            }
            let exe_ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
            let exe_path = format!("{}{}", stem_owned, exe_ext);
            print!(" linking ...");
            let mut gcc_args: Vec<String> = vec![
                obj_out.clone(), "-o".into(), exe_path.clone(), "-lm".into(),
            ];
            if !cfg!(target_os = "windows") {
                gcc_args.push("-no-pie".into());
            }
            let status = std::process::Command::new("gcc")
                .args(&gcc_args)
                .status();
            let _ = fs::remove_file(&obj_out);
            match status {
                Ok(s) if s.success() => {
                    println!(" OK\narc: → {}", exe_path);
                    0
                }
                Ok(s) => { eprintln!("\narc: linker failed (exit {})", s); 1 }
                Err(e) => { eprintln!("\narc: linker not found — {}", e); 1 }
            }
        }
        Err(e) => { eprintln!("\narc: codegen error — {}", e); 1 }
    }
}

// ─── subcommands ──────────────────────────────────────────────────────────────

fn cmd_init(name: &str) {
    let dir = PathBuf::from(name);
    if dir.exists() {
        eprintln!("arc: '{}' already exists", name);
        process::exit(1);
    }
    fs::create_dir_all(&dir).unwrap_or_else(|e| {
        eprintln!("arc: could not create project: {}", e); process::exit(1);
    });
    fs::create_dir_all(dir.join("tests")).unwrap_or_else(|_| {});

    let toml = format!(
r#"# arc.toml -- Arimo project manifest

[project]
name        = "{name}"
version     = "0.1.0"
description = ""
entry       = "Main.arm"
license     = "MIT"
readme      = "README.md"
keywords    = []
homepage    = ""
repository  = ""

[[project.authors]]
name  = ""
email = ""

# -- Build --------------------------------------------------------------------

[build]
target = "x86_64-pc-windows-gnu"

[profiles.release]
optimize = true

[profiles.dev]
optimize = false

# -- Stdlib -------------------------------------------------------------------

[stdlib]
include = []

# -- Dependencies -------------------------------------------------------------

[dependencies]
# arimo-http = "1.0.0"

[dev-dependencies]
# arimo-test = "0.1.0"

# -- Scripts ------------------------------------------------------------------

[scripts]
# lint = "arc check --strict"
# docs = "arc doc"
"#
    );
    fs::write(dir.join("arc.toml"), toml).unwrap();

    let main_arm = format!(
r#"package {name};

public class Main {{
    public static main() : Void {{
        IO.println("Hello from {name}!");
    }}
}}
"#
    );
    fs::write(dir.join("Main.arm"), main_arm).unwrap();

    println!("arc: created project '{}'", name);
    println!("arc: → {}/arc.toml", name);
    println!("arc: → {}/Main.arm", name);
    println!("arc: → {}/tests/", name);
    println!("\nRun: cd {} && arc build", name);
}

fn cmd_clean() {
    let mut removed = 0usize;
    for entry in fs::read_dir(".").unwrap_or_else(|_| { process::exit(0) }).flatten() {
        let p = entry.path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "exe" | "o") {
            if fs::remove_file(&p).is_ok() { removed += 1; }
        }
    }
    println!("arc: clean — removed {} file(s)", removed);
}

// ─── main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();
    let version = "arc v0.5.0 (Arimo Compiler)";

    if args.len() < 2 {
        println!("{}", version);
        println!("Usage:");
        println!("  arc build            compile project (arc.toml)");
        println!("  arc run              compile + run project");
        println!("  arc check            type-check only");
        println!("  arc clean            remove build artifacts");
        println!("  arc init <name>      create new project");
        println!("  arc <file.arm> ...   compile files directly");
        process::exit(0);
    }

    let stdlib_dir = find_stdlib_dir();
    let stdlib_opt: Option<&Path> = stdlib_dir.as_deref();

    match args[1].as_str() {
        "init" => {
            let name = args.get(2).map(|s| s.as_str()).unwrap_or("myproject");
            cmd_init(name);
        }

        "clean" => {
            cmd_clean();
        }

        "build" | "run" | "check" => {
            let toml_path = PathBuf::from("arc.toml");
            if !toml_path.exists() {
                eprintln!("arc: arc.toml not found — run 'arc init <name>' to create a project");
                eprintln!("     or compile directly: arc <file.arm>");
                process::exit(1);
            }
            let toml_content = fs::read_to_string(&toml_path).unwrap_or_default();
            let cfg = parse_arc_toml(&toml_content);

            // Collect stdlib files
            let mut entry_files: Vec<String> = Vec::new();

            if let Some(ref stdlib) = stdlib_dir {
                for pkg in &cfg.stdlib {
                    for f in stdlib_files_for(stdlib, pkg) {
                        entry_files.push(f.to_string_lossy().to_string());
                    }
                }
            }

            // Add the entry file last
            let entry_path = PathBuf::from(&cfg.entry);
            if !entry_path.exists() {
                eprintln!("arc: entry file '{}' not found", cfg.entry);
                process::exit(1);
            }
            entry_files.push(cfg.entry.clone());

            let check_only = args[1] == "check";
            let emit_ir    = args.contains(&"--emit-ir".to_string());
            let only_obj   = args.contains(&"-c".to_string());
            let is_release = args.contains(&"--release".to_string());
            let optimize   = (if is_release { cfg.optimize_release } else { cfg.optimize_dev })
                             || args.contains(&"-O2".to_string());

            println!("{}", version);
            println!("arc: building '{}'  v{}  ({})", cfg.name, cfg.version, cfg.target);

            let entry_refs: Vec<&str> = entry_files.iter().map(|s| s.as_str()).collect();
            // Use entry .arm stem for output name
            let stem = Path::new(&cfg.entry)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&cfg.name)
                .to_string();
            let out_stem = cfg.name.clone();

            let project_root = fs::canonicalize(".").ok();
            let project_root_ref: Option<&Path> = project_root.as_deref();

            let opts = CompileOptions {
                entry_files: entry_refs.iter().map(|s| s.to_string()).collect(),
                project_root: project_root_ref,
                stdlib_dir: stdlib_opt,
                emit_ir,
                only_obj,
                optimize,
                out_stem: Some(out_stem.clone()),
                check_only,
            };

            let code = run_pipeline(opts);

            if code == 0 && args[1] == "run" && !check_only {
                let run_ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
                let run_path = format!("./{}{}", out_stem, run_ext);
                println!("\narc: running '{}{}'", out_stem, run_ext);
                println!("{}", "─".repeat(40));
                let status = std::process::Command::new(&run_path)
                    .status()
                    .unwrap_or_else(|e| {
                        eprintln!("arc: could not run: {}", e);
                        process::exit(1);
                    });
                process::exit(status.code().unwrap_or(0));
            }

            process::exit(code);
        }

        "version" | "--version" | "-v" => {
            println!("{}", version);
        }

        _ => {
            // Legacy direct-file mode: arc file.arm [file2.arm ...] [flags]
            let emit_ir  = args.contains(&"--emit-ir".to_string());
            let only_obj = args.contains(&"-c".to_string());
            let optimize = args.contains(&"-O2".to_string()) || args.contains(&"-O3".to_string());

            let arm_files: Vec<String> = args.iter()
                .skip(1)
                .filter(|a| a.ends_with(".arm"))
                .map(|s| s.clone())
                .collect();

            if arm_files.is_empty() {
                eprintln!("arc: unknown command '{}' — try 'arc build' or 'arc <file.arm>'", args[1]);
                process::exit(1);
            }

            let direct_root = fs::canonicalize(".").ok();
            let direct_root_ref: Option<&Path> = direct_root.as_deref();
            let opts = CompileOptions {
                entry_files: arm_files,
                project_root: direct_root_ref,
                stdlib_dir: stdlib_opt,
                emit_ir,
                only_obj,
                optimize,
                out_stem: None,
                check_only: false,
            };
            process::exit(run_pipeline(opts));
        }
    }
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
            ast::Item::Interface(i) => println!("  interface {} ({} methods)", i.name, i.methods.len()),
            ast::Item::Enum(e)      => println!("  enum {} {:?}", e.name, e.variants),
            ast::Item::Exception(e) => println!("  exception {} extends {}", e.name, e.extends),
            ast::Item::Struct(s) => {
                println!("  struct {} {{", s.name);
                println!("    fields  : {}", s.fields.len());
                println!("    methods : {}", s.methods.len());
                println!("  }}");
            }
            ast::Item::TypeAlias(a)  => println!("  type {} = {:?}", a.name, a.ty),
            ast::Item::Union(u)      => println!("  union {} ({} fields)", u.name, u.fields.len()),
            ast::Item::Extern(e)     => println!("  extern \"{}\" ({} decls)", e.abi, e.decls.len()),
            ast::Item::Extension(e)  => println!("  extend {} ({} methods)", e.target, e.methods.len()),
        }
    }
}
