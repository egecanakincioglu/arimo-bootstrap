
mod lexer;
mod ast;
mod parser;
mod typechecker;
mod borrow;
mod codegen;

use std::{env, fs, path::Path, process};
use lexer::Lexer;
use parser::Parser;
use typechecker::TypeChecker;
use borrow::BorrowChecker;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("arc v0.3.0");
        eprintln!("Usage: arc <file.arm> [--emit-ir]");
        process::exit(1);
    }

    let emit_ir    = args.contains(&"--emit-ir".to_string());
    let only_obj   = args.contains(&"-c".to_string());
    let optimize   = args.contains(&"-O2".to_string()) || args.contains(&"-O3".to_string());
    let path = args.iter().find(|a| a.ends_with(".arm")).unwrap_or_else(|| {
        eprintln!("error: no .arm source file provided");
        process::exit(1);
    });

    if !path.ends_with(".arm") {
        eprintln!("error: source file must have .arm extension");
        process::exit(1);
    }

    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: could not read '{}': {}", path, e);
        process::exit(1);
    });

    let mut lexer  = Lexer::new(&source);
    let tokens     = lexer.tokenize();

    println!("arc: parsing '{}'", path);

    let mut parser = Parser::new(tokens);
    let module = match parser.parse() {
        Ok(m) => {
            println!("arc: parse OK");
            println!("");
            println!("Module   : {}", m.path);
            println!("Imports  : {:?}", m.imports);
            println!("Items    : {}", m.items.len());
            println!("");
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
                }
            }
            m
        }
        Err(e) => {
            eprintln!("arc: parse error at {}:{} — {}", e.line, e.col, e.message);
            process::exit(1);
        }
    };

    println!("");
    let mut tc      = TypeChecker::new();
    tc.check(&module);

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

    let mut bc       = BorrowChecker::new();
    let borrow_errors = bc.check(&module);

    if borrow_errors.is_empty() {
        println!("arc: borrow check OK");
        println!("arc: drop schedule — {} scope(s) tracked", bc.drops.len());
    } else {
        for err in borrow_errors {
            eprintln!("arc: {}", err);
        }
        process::exit(1);
    }

    println!("");
    let stem   = Path::new(path).file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let obj_path = format!("{}.o", stem);
    let exe_path = format!("{}.exe", stem);

    if emit_ir {
        let ir = codegen::emit_ir_only(&module, stem);
        match ir {
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
    match codegen::compile_to_object_opts(&module, stem, Path::new(&obj_out), optimize) {
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