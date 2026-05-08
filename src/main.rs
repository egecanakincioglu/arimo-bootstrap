mod lexer;
mod ast;
mod parser;
mod typechecker;
mod borrow;
mod codegen;

use std::{env, fs, process};
use lexer::Lexer;
use parser::Parser;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("arc v0.2.0");
        eprintln!("Usage: arc <file.arm>");
        process::exit(1);
    }

    let path = &args[1];

    if !path.ends_with(".arm") {
        eprintln!("error: source file must have .arm extension");
        process::exit(1);
    }

    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: could not read '{}': {}", path, e);
        process::exit(1);
    });

    // Lexer
    let mut lexer  = Lexer::new(&source);
    let tokens     = lexer.tokenize();

    println!("arc: parsing '{}'", path);

    // Parser
    let mut parser = Parser::new(tokens);
    match parser.parse() {
        Ok(module) => {
            println!("arc: parse OK");
            println!("");
            println!("Module   : {}", module.path);
            println!("Imports  : {:?}", module.imports);
            println!("Items    : {}", module.items.len());
            println!("");
            for item in &module.items {
                match item {
                    ast::Item::Class(c) => {
                        println!("  class {} {{", c.name);
                        println!("    fields      : {}", c.fields.len());
                        println!("    constructor : {}", c.constructor.is_some());
                        println!("    methods     : {}", c.methods.len());
                        for m in &c.methods {
                            let ret = match &m.return_ty {
                                Some(t) => format!("{:?}", t),
                                None    => "—".to_string(),
                            };
                            println!("      {} {}() : {}", 
                                if m.static_ { "static" } else { "      " },
                                m.name, ret
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
                        println!("  exception {}", e.name);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("arc: parse error at {}:{} — {}", e.line, e.col, e.message);
            process::exit(1);
        }
    }
}