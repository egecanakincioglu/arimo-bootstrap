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

use crate::lexer::{Token, SpannedToken};
use crate::ast::*;

#[derive(Debug)]
pub struct ParseError {
    pub message : String,
    pub line    : usize,
    pub col     : usize,
}

impl ParseError {
    fn new(msg: &str, line: usize, col: usize) -> Self {
        ParseError { message: msg.to_string(), line, col }
    }
}

type ParseResult<T> = Result<T, ParseError>;

pub struct Parser {
    tokens           : Vec<SpannedToken>,
    pos              : usize,
    pending_close_gt : bool,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0, pending_close_gt: false }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn current_span(&self) -> (usize, usize) {
        let s = &self.tokens[self.pos].span;
        (s.line, s.col)
    }

    fn peek(&self) -> &Token {
        if self.pos + 1 < self.tokens.len() {
            &self.tokens[self.pos + 1].token
        } else {
            &Token::Eof
        }
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos].token;
        if self.pos + 1 < self.tokens.len() { self.pos += 1; }
        tok
    }

    fn expect(&mut self, expected: &Token) -> ParseResult<()> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            let (line, col) = self.current_span();
            Err(ParseError::new(
                &format!("Expected {:?}, found {:?}", expected, self.current()),
                line, col,
            ))
        }
    }

    fn eat_close_gt(&mut self) -> bool {
        if self.pending_close_gt {
            self.pending_close_gt = false;
            return true;
        }
        if self.current() == &Token::Gt {
            self.advance();
            return true;
        }
        if self.current() == &Token::GtGt {
            self.advance();
            self.pending_close_gt = true;
            return true;
        }
        false
    }

    fn expect_close_gt(&mut self) -> ParseResult<()> {
        if self.eat_close_gt() {
            Ok(())
        } else {
            let (line, col) = self.current_span();
            Err(ParseError::new(
                &format!("Expected '>', found {:?}", self.current()),
                line, col,
            ))
        }
    }

    fn expect_ident(&mut self) -> ParseResult<String> {
        match self.current().clone() {
            Token::Ident(name) => { self.advance(); Ok(name) }
            _ => {
                let (line, col) = self.current_span();
                Err(ParseError::new(
                    &format!("Expected identifier, found {:?}", self.current()),
                    line, col,
                ))
            }
        }
    }

    fn expect_class_name(&mut self) -> ParseResult<String> {
        match self.current().clone() {
            Token::Ident(name)   => { self.advance(); Ok(name) }
            Token::TypeException => { self.advance(); Ok("Exception".to_string()) }
            _ => {
                let (line, col) = self.current_span();
                Err(ParseError::new(
                    &format!("Expected class name, found {:?}", self.current()),
                    line, col,
                ))
            }
        }
    }

    fn check(&self, tok: &Token) -> bool {
        self.current() == tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.current() == tok { self.advance(); true }
        else { false }
    }

    pub fn parse(&mut self) -> ParseResult<Module> {
        let mut nostd = false;
        while self.check(&Token::At) {
            self.advance();
            let ann = self.expect_ident()?;
            match ann.as_str() {
                "Freestanding" => { nostd = true; }
                other => {
                    let (line, col) = self.current_span();
                    return Err(ParseError::new(
                        &format!("unknown module annotation '@{}' — use @Freestanding", other),
                        line, col,
                    ));
                }
            }
        }
        let path    = self.parse_module_decl()?;
        let imports = self.parse_imports()?;
        let items   = self.parse_items()?;
        Ok(Module { path, nostd, imports, items })
    }

    fn parse_module_decl(&mut self) -> ParseResult<String> {
        self.expect(&Token::Package)?;
        let path = self.parse_dotted_path()?;
        self.expect(&Token::Semicolon)?;
        Ok(path)
    }

    fn parse_dotted_path(&mut self) -> ParseResult<String> {
        let mut path = self.expect_ident()?;
        while self.eat(&Token::Dot) {
            if self.check(&Token::Star) {
                self.advance();
                path.push_str(".*");
                break;
            }
            let part = self.expect_ident()?;
            path.push('.');
            path.push_str(&part);
        }
        Ok(path)
    }

    fn parse_imports(&mut self) -> ParseResult<Vec<String>> {
        let mut imports = Vec::new();
        while self.check(&Token::Import) {
            self.advance();
            let path = self.parse_dotted_path()?;
            self.expect(&Token::Semicolon)?;
            imports.push(path);
        }
        Ok(imports)
    }

    fn parse_items(&mut self) -> ParseResult<Vec<Item>> {
        let mut items = Vec::new();
        while !self.check(&Token::Eof) {
            let item = self.parse_item()?;
            items.push(item);
        }
        Ok(items)
    }

    fn parse_item(&mut self) -> ParseResult<Item> {
        let mut manual       = false;
        let mut packed       = false;
        let mut align: Option<usize> = None;
        let mut sealed       = false;
        let mut immutable    = false;
        let mut functional   = false;
        let mut deprecated   : Option<String> = None;
        let mut experimental = false;

        while self.check(&Token::At) {
            self.advance();
            let annotation = self.expect_ident()?;
            match annotation.as_str() {
                "ManualMemory" => { manual = true; }
                "Packed"       => { packed = true; }
                "Sealed"       => { sealed = true; }
                "Immutable"    => { immutable = true; }
                "FunctionalInterface" => { functional = true; }
                "Experimental" => { experimental = true; }
                "Align" => {
                    self.expect(&Token::LParen)?;
                    match self.current().clone() {
                        Token::Int(n) if n > 0 => { align = Some(n as usize); self.advance(); }
                        _ => {
                            let (line, col) = self.current_span();
                            return Err(ParseError::new("@Align expects a positive integer", line, col));
                        }
                    }
                    self.expect(&Token::RParen)?;
                }
                "Deprecated" => {
                    self.expect(&Token::LParen)?;
                    match self.current().clone() {
                        Token::Str(s) => { deprecated = Some(s); self.advance(); }
                        _ => {
                            let (line, col) = self.current_span();
                            return Err(ParseError::new("@Deprecated expects a string message", line, col));
                        }
                    }
                    self.expect(&Token::RParen)?;
                }
                other => {
                    let (line, col) = self.current_span();
                    return Err(ParseError::new(
                        &format!("unknown item annotation '@{}' — available: @ManualMemory, @Packed, @Align(N), @Sealed, @Immutable, @FunctionalInterface, @Deprecated(\"...\"), @Experimental", other),
                        line, col,
                    ));
                }
            }
        }

        if self.check(&Token::Extern) {
            return self.parse_extern_block();
        }

        if self.check(&Token::KwType) {
            return self.parse_type_alias_item();
        }

        let visibility = self.parse_visibility()?;
        let abstract_  = self.eat(&Token::Abstract);

        match self.current().clone() {
            Token::Class => {
                let class = self.parse_class(visibility, abstract_, manual, sealed, immutable, deprecated.clone(), experimental)?;
                if let Some(ref parent) = class.extends.clone() {
                    if parent == "Exception" || parent.ends_with("Exception") {
                        return Ok(Item::Exception(ExceptionDecl {
                            visibility  : class.visibility,
                            manual      : class.manual,
                            name        : class.name,
                            extends     : parent.clone(),
                            fields      : class.fields,
                            constructor : class.constructor,
                            methods     : class.methods,
                        }));
                    }
                }
                Ok(Item::Class(class))
            }
            Token::Struct    => Ok(Item::Struct(self.parse_struct(visibility, packed, align, deprecated.clone(), experimental)?)),
            Token::Interface => Ok(Item::Interface(self.parse_interface(functional, sealed, deprecated.clone(), experimental)?)),
            Token::Enum      => Ok(Item::Enum(self.parse_enum(visibility, deprecated.clone(), experimental)?)),
            Token::Union     => Ok(Item::Union(self.parse_union(visibility)?)),
            _ => {
                let (line, col) = self.current_span();
                Err(ParseError::new(
                    &format!("Expected class, struct, interface, enum or union, found {:?}", self.current()),
                    line, col,
                ))
            }
        }
    }

    fn parse_type_alias_item(&mut self) -> ParseResult<Item> {
        self.expect(&Token::KwType)?;
        let name = self.expect_ident()?;
        self.expect(&Token::Eq)?;
        let ty = self.parse_type()?;
        self.expect(&Token::Semicolon)?;
        Ok(Item::TypeAlias(TypeAliasDecl { name, ty }))
    }

    fn parse_visibility(&mut self) -> ParseResult<Visibility> {
        match self.current() {
            Token::Public    => { self.advance(); Ok(Visibility::Public)    }
            Token::Private   => { self.advance(); Ok(Visibility::Private)   }
            Token::Protected => { self.advance(); Ok(Visibility::Protected) }
            Token::Internal  => { self.advance(); Ok(Visibility::Internal)  }
            _ => Ok(Visibility::Public),
        }
    }

    fn parse_class(&mut self, visibility: Visibility, abstract_: bool, manual: bool, sealed: bool, immutable: bool, deprecated: Option<String>, experimental: bool) -> ParseResult<ClassDecl> {
        self.expect(&Token::Class)?;
        let name     = self.expect_ident()?;
        let generics = self.parse_generics_decl()?;

        let extends = if self.eat(&Token::Extends) {
            Some(self.expect_class_name()?)
        } else { None };

        let implements = if self.eat(&Token::Implements) {
            self.parse_comma_separated_class_names()?
        } else { Vec::new() };

        self.expect(&Token::LBrace)?;

        let mut fields      = Vec::new();
        let mut constructor = None;
        let mut methods     = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let mut inline_      = false;
            let mut async_       = false;
            let mut pure_        = false;
            let mut meth_deprecated   : Option<String> = None;
            let mut meth_experimental = false;
            let mut throws       : Vec<String> = Vec::new();
            let mut suppress     : Vec<String> = Vec::new();
            let mut calling_conv : Option<CallingConv> = None;
            let mut section      : Option<String> = None;
            while self.check(&Token::At) {
                self.advance();
                let ann = self.expect_ident()?;
                match ann.as_str() {
                    "ForceInline"  => { inline_ = true; }
                    "async"        => { async_  = true; }
                    "Pure"         => { pure_   = true; }
                    "Experimental" => { meth_experimental = true; }
                    "Deprecated"   => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { meth_deprecated = Some(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@Deprecated expects a string message", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "Throws" => {
                        self.expect(&Token::LParen)?;
                        while !self.check(&Token::RParen) && !self.check(&Token::Eof) {
                            throws.push(self.expect_ident()?);
                            if !self.check(&Token::RParen) { self.eat(&Token::Comma); }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "SuppressWarnings" => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { suppress.push(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@SuppressWarnings expects a string", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "CallingConvention" => {
                        self.expect(&Token::LParen)?;
                        let conv = match self.current().clone() {
                            Token::Str(s) => { self.advance(); s }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new(
                                    "@CallingConvention expects a string: \"C\", \"Windows\" or \"Interrupt\"",
                                    line, col,
                                ));
                            }
                        };
                        self.expect(&Token::RParen)?;
                        calling_conv = Some(match conv.as_str() {
                            "C"         => CallingConv::Cdecl,
                            "Windows"   => CallingConv::Stdcall,
                            "Interrupt" => CallingConv::Interrupt,
                            other => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new(
                                    &format!("unknown calling convention '{}' — use \"C\", \"Windows\" or \"Interrupt\"", other),
                                    line, col,
                                ));
                            }
                        });
                    }
                    "Section" => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { section = Some(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@Section expects a string literal", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    other => {
                        let (line, col) = self.current_span();
                        return Err(ParseError::new(
                            &format!("unknown method annotation '@{}' — available: @ForceInline, @Pure, @Deprecated(\"...\"), @Experimental, @Throws(...), @SuppressWarnings(\"...\"), @CallingConvention(\"...\"), @Section(\"...\")", other),
                            line, col,
                        ));
                    }
                }
            }

            let vis       = self.parse_visibility()?;
            let static_   = self.eat(&Token::Static);
            let async_    = async_ || self.eat(&Token::Async);
            let readonly  = self.eat(&Token::Readonly);
            let abstract_ = self.eat(&Token::Abstract);
            let override_ = self.eat(&Token::Override);

            if self.check(&Token::Operator) {
                self.advance();
                let op_sym = self.parse_operator_symbol()?;
                let method_name = format!("operator{}", op_sym);
                let method = self.parse_method_body(vis, static_, abstract_, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, method_name, None)?;
                methods.push(method);
                continue;
            }

            match self.current().clone() {
                Token::Constructor => {
                    self.advance();
                    constructor = Some(self.parse_constructor(vis)?);
                }
                _ => {
                    if let Token::Ident(iname) = self.current().clone() {
                        if self.pos + 1 < self.tokens.len() {
                            let next = &self.tokens[self.pos + 1].token;
                            if *next == Token::LParen {
                                self.advance();
                                let method = self.parse_method_body(
                                    vis, static_, abstract_, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, iname, None
                                )?;
                                methods.push(method);
                                continue;
                            }
                        }
                        if self.pos + 1 < self.tokens.len() {
                            let next = &self.tokens[self.pos + 1].token;
                            if *next == Token::Colon {
                                let name = self.expect_ident()?;
                                self.expect(&Token::Colon)?;
                                let ty = self.parse_type()?;
                                let value = if self.eat(&Token::Eq) {
                                    Some(self.parse_expr(0)?)
                                } else { None };
                                self.expect(&Token::Semicolon)?;
                                fields.push(Field { visibility: vis, readonly, static_, name, ty, value });
                                continue;
                            }
                        }
                    }

                    if self.is_type_token() {
                        let ty   = self.parse_type()?;
                        let name = self.expect_ident()?;

                        if self.check(&Token::LParen) {
                            let method = self.parse_method_body(
                                vis, static_, abstract_, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, name, Some(ty)
                            )?;
                            methods.push(method);
                        } else {
                            let value = if self.eat(&Token::Eq) {
                                Some(self.parse_expr(0)?)
                            } else { None };
                            self.expect(&Token::Semicolon)?;
                            fields.push(Field { visibility: vis, readonly, static_, name, ty, value });
                        }
                    } else {
                        let (line, col) = self.current_span();
                        return Err(ParseError::new(
                            &format!("Unexpected token in class body: {:?}", self.current()),
                            line, col,
                        ));
                    }
                }
            }
        }

        self.expect(&Token::RBrace)?;
        Ok(ClassDecl { visibility, abstract_, manual, sealed, immutable, deprecated, experimental, name, generics, extends, implements, fields, constructor, methods })
    }

    fn parse_constructor(&mut self, visibility: Visibility) -> ParseResult<Constructor> {
        let params = self.parse_params()?;
        self.expect(&Token::LBrace)?;
        let body = self.parse_stmts()?;
        self.expect(&Token::RBrace)?;
        Ok(Constructor { visibility, params, body })
    }

    fn parse_method_body(
        &mut self,
        visibility   : Visibility,
        static_      : bool,
        abstract_    : bool,
        default_     : bool,
        override_    : bool,
        inline_      : bool,
        async_       : bool,
        pure_        : bool,
        deprecated   : Option<String>,
        experimental : bool,
        throws       : Vec<String>,
        suppress     : Vec<String>,
        calling_conv : Option<CallingConv>,
        section      : Option<String>,
        name         : String,
        return_ty    : Option<Type>,
    ) -> ParseResult<Method> {
        let params = self.parse_params()?;

        let return_ty = if self.eat(&Token::Colon) {
            Some(self.parse_type()?)
        } else {
            return_ty
        };

        let body = if abstract_ || self.check(&Token::Semicolon) {
            self.eat(&Token::Semicolon);
            None
        } else {
            self.expect(&Token::LBrace)?;
            let stmts = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;
            Some(stmts)
        };

        Ok(Method { visibility, static_, abstract_, default_, override_, inline_, async_, pure_, deprecated, experimental, throws, suppress, calling_conv, section, name, params, return_ty, body })
    }

    fn parse_operator_symbol(&mut self) -> ParseResult<String> {
        let sym = match self.current() {
            Token::Plus    => "+",
            Token::Minus   => "-",
            Token::Star    => "*",
            Token::Slash   => "/",
            Token::Percent => "%",
            Token::EqEq    => "==",
            Token::BangEq  => "!=",
            Token::Lt      => "<",
            Token::LtEq    => "<=",
            Token::Gt      => ">",
            Token::GtEq    => ">=",
            Token::LBracket => {
                self.advance();
                self.expect(&Token::RBracket)?;
                return Ok("[]".to_string());
            }
            _ => {
                let (line, col) = self.current_span();
                return Err(ParseError::new(
                    &format!("Expected operator symbol (+, -, *, /, ==, !=, <, >, etc.), found {:?}", self.current()),
                    line, col,
                ));
            }
        };
        self.advance();
        Ok(sym.to_string())
    }

    fn parse_struct(&mut self, visibility: Visibility, packed: bool, align: Option<usize>, deprecated: Option<String>, experimental: bool) -> ParseResult<StructDecl> {
        self.expect(&Token::Struct)?;
        let name     = self.expect_ident()?;
        let generics = self.parse_generics_decl()?;

        let implements = if self.eat(&Token::Implements) {
            self.parse_comma_separated_class_names()?
        } else { Vec::new() };

        self.expect(&Token::LBrace)?;

        let mut fields      = Vec::new();
        let mut constructor = None;
        let mut methods     = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let mut inline_      = false;
            let mut async_       = false;
            let mut pure_        = false;
            let mut meth_deprecated   : Option<String> = None;
            let mut meth_experimental = false;
            let mut throws       : Vec<String> = Vec::new();
            let mut suppress     : Vec<String> = Vec::new();
            let mut calling_conv : Option<CallingConv> = None;
            let mut section      : Option<String> = None;
            while self.check(&Token::At) {
                self.advance();
                let ann = self.expect_ident()?;
                match ann.as_str() {
                    "ForceInline"  => { inline_ = true; }
                    "async"        => { async_  = true; }
                    "Pure"         => { pure_   = true; }
                    "Experimental" => { meth_experimental = true; }
                    "Deprecated"   => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { meth_deprecated = Some(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@Deprecated expects a string message", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "Throws" => {
                        self.expect(&Token::LParen)?;
                        while !self.check(&Token::RParen) && !self.check(&Token::Eof) {
                            throws.push(self.expect_ident()?);
                            if !self.check(&Token::RParen) { self.eat(&Token::Comma); }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "SuppressWarnings" => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { suppress.push(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@SuppressWarnings expects a string", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    "CallingConvention" => {
                        self.expect(&Token::LParen)?;
                        let conv = match self.current().clone() {
                            Token::Str(s) => { self.advance(); s }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new(
                                    "@CallingConvention expects a string: \"C\", \"Windows\" or \"Interrupt\"",
                                    line, col,
                                ));
                            }
                        };
                        self.expect(&Token::RParen)?;
                        calling_conv = Some(match conv.as_str() {
                            "C"         => CallingConv::Cdecl,
                            "Windows"   => CallingConv::Stdcall,
                            "Interrupt" => CallingConv::Interrupt,
                            other => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new(
                                    &format!("unknown calling convention '{}' — use \"C\", \"Windows\" or \"Interrupt\"", other),
                                    line, col,
                                ));
                            }
                        });
                    }
                    "Section" => {
                        self.expect(&Token::LParen)?;
                        match self.current().clone() {
                            Token::Str(s) => { section = Some(s); self.advance(); }
                            _ => {
                                let (line, col) = self.current_span();
                                return Err(ParseError::new("@Section expects a string literal", line, col));
                            }
                        }
                        self.expect(&Token::RParen)?;
                    }
                    other => {
                        let (line, col) = self.current_span();
                        return Err(ParseError::new(
                            &format!("unknown method annotation '@{}' — available: @ForceInline, @Pure, @Deprecated(\"...\"), @Experimental, @Throws(...), @SuppressWarnings(\"...\"), @CallingConvention(\"...\"), @Section(\"...\")", other),
                            line, col,
                        ));
                    }
                }
            }

            let vis       = self.parse_visibility()?;
            let static_   = self.eat(&Token::Static);
            let async_    = async_ || self.eat(&Token::Async);
            let _readonly = self.eat(&Token::Readonly);
            let override_ = self.eat(&Token::Override);

            if self.check(&Token::Operator) {
                self.advance();
                let op_sym = self.parse_operator_symbol()?;
                let method_name = format!("operator{}", op_sym);
                let method = self.parse_method_body(vis, static_, false, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, method_name, None)?;
                methods.push(method);
                continue;
            }

            match self.current().clone() {
                Token::Constructor => {
                    self.advance();
                    constructor = Some(self.parse_constructor(vis)?);
                }
                _ => {
                    if let Token::Ident(iname) = self.current().clone() {
                        if self.pos + 1 < self.tokens.len() {
                            let next = &self.tokens[self.pos + 1].token;
                            if *next == Token::LParen {
                                self.advance();
                                let method = self.parse_method_body(
                                    vis, static_, false, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, iname, None
                                )?;
                                methods.push(method);
                                continue;
                            }
                        }
                        if self.pos + 1 < self.tokens.len() {
                            let next = &self.tokens[self.pos + 1].token;
                            if *next == Token::Colon {
                                let fname = self.expect_ident()?;
                                self.expect(&Token::Colon)?;
                                let ty = self.parse_type()?;
                                let value = if self.eat(&Token::Eq) {
                                    Some(self.parse_expr(0)?)
                                } else { None };
                                self.expect(&Token::Semicolon)?;
                                fields.push(Field { visibility: vis, readonly: false, static_: static_, name: fname, ty, value });
                                continue;
                            }
                        }
                    }

                    if self.is_type_token() {
                        let ty   = self.parse_type()?;
                        let name = self.expect_ident()?;
                        if self.check(&Token::LParen) {
                            let method = self.parse_method_body(
                                vis, static_, false, false, override_, inline_, async_, pure_, meth_deprecated.clone(), meth_experimental, throws.clone(), suppress.clone(), calling_conv, section, name, Some(ty)
                            )?;
                            methods.push(method);
                        } else {
                            let value = if self.eat(&Token::Eq) {
                                Some(self.parse_expr(0)?)
                            } else { None };
                            self.expect(&Token::Semicolon)?;
                            fields.push(Field { visibility: vis, readonly: false, static_: static_, name, ty, value });
                        }
                    } else {
                        let (line, col) = self.current_span();
                        return Err(ParseError::new(
                            &format!("Unexpected token in struct body: {:?}", self.current()),
                            line, col,
                        ));
                    }
                }
            }
        }

        self.expect(&Token::RBrace)?;
        Ok(StructDecl { visibility, packed, align, deprecated, experimental, name, generics, implements, fields, constructor, methods })
    }

    fn parse_union(&mut self, visibility: Visibility) -> ParseResult<UnionDecl> {
        self.expect(&Token::Union)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;
        let mut fields = Vec::new();
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let vis   = self.parse_visibility()?;
            let fname = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;
            self.expect(&Token::Semicolon)?;
            fields.push(Field { visibility: vis, readonly: false, static_: false, name: fname, ty, value: None });
        }
        self.expect(&Token::RBrace)?;
        Ok(UnionDecl { visibility, name, fields })
    }

    fn parse_extern_block(&mut self) -> ParseResult<Item> {
        self.expect(&Token::Extern)?;
        let abi = match self.current().clone() {
            Token::Str(s) => { self.advance(); s }
            _ => {
                let (line, col) = self.current_span();
                return Err(ParseError::new("extern expects ABI string, e.g. \"C\"", line, col));
            }
        };
        self.expect(&Token::LBrace)?;
        let mut decls = Vec::new();
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            decls.push(self.parse_extern_decl()?);
        }
        self.expect(&Token::RBrace)?;
        Ok(Item::Extern(ExternBlock { abi, decls }))
    }

    fn parse_extern_decl(&mut self) -> ParseResult<ExternDecl> {
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let mut params   = Vec::new();
        let mut variadic = false;
        while !self.check(&Token::RParen) && !self.check(&Token::Eof) {
            if self.check(&Token::Ellipsis) {
                self.advance();
                variadic = true;
                break;
            }
            let pname = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let pty = self.parse_type()?;
            params.push(ExternParam { name: pname, ty: pty });
            if !self.check(&Token::RParen) {
                self.eat(&Token::Comma);
            }
        }
        self.expect(&Token::RParen)?;
        let return_ty = if self.eat(&Token::Colon) { Some(self.parse_type()?) } else { None };
        self.expect(&Token::Semicolon)?;
        Ok(ExternDecl { name, params, return_ty, variadic })
    }

    fn parse_asm_block(&mut self) -> ParseResult<String> {
        self.expect(&Token::LBrace)?;
        let mut content = String::new();
        let mut depth   = 1usize;
        while depth > 0 && !self.check(&Token::Eof) {
            match self.current().clone() {
                Token::LBrace  => { depth += 1; content.push_str("{ "); self.advance(); }
                Token::RBrace  => {
                    depth -= 1;
                    if depth > 0 { content.push_str("} "); }
                    self.advance();
                }
                Token::Ident(s) => { content.push_str(&s); content.push(' '); self.advance(); }
                Token::Int(n)   => { content.push_str(&n.to_string()); content.push(' '); self.advance(); }
                Token::Comma    => { content.push_str(", "); self.advance(); }
                Token::Semicolon => { content.push('\n'); self.advance(); }
                Token::Colon    => { content.push_str(": "); self.advance(); }
                _ => { self.advance(); }
            }
        }
        Ok(content.trim().to_string())
    }

    fn parse_interface(&mut self, functional: bool, sealed: bool, deprecated: Option<String>, experimental: bool) -> ParseResult<InterfaceDecl> {
        self.expect(&Token::Interface)?;
        let name     = self.expect_ident()?;
        let generics = self.parse_generics_decl()?;
        self.expect(&Token::LBrace)?;

        let mut methods = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let default_ = self.eat(&Token::Default);
            let name     = self.expect_ident()?;
            let params   = self.parse_params()?;
            self.expect(&Token::Colon)?;
            let return_ty = self.parse_type()?;
            let body = if default_ {
                self.expect(&Token::LBrace)?;
                let stmts = self.parse_stmts()?;
                self.expect(&Token::RBrace)?;
                Some(stmts)
            } else {
                self.expect(&Token::Semicolon)?;
                None
            };
            methods.push(Method {
                visibility   : Visibility::Public,
                static_      : false,
                abstract_    : !default_,
                default_     : default_,
                override_    : false,
                inline_      : false,
                async_       : false,
                pure_        : false,
                deprecated   : None,
                experimental : false,
                throws       : Vec::new(),
                suppress     : Vec::new(),
                calling_conv : None,
                section      : None,
                name,
                params,
                return_ty    : Some(return_ty),
                body,
            });
        }

        self.expect(&Token::RBrace)?;
        Ok(InterfaceDecl { name, generics, functional, sealed, deprecated, experimental, methods })
    }

    fn parse_enum(&mut self, visibility: Visibility, deprecated: Option<String>, experimental: bool) -> ParseResult<EnumDecl> {
        self.expect(&Token::Enum)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut variants = Vec::new();
        let mut methods  = Vec::new();

        while !self.check(&Token::Semicolon) && !self.check(&Token::RBrace) {
            let vname = self.expect_ident()?;
            let data = if self.check(&Token::LParen) {
                self.advance();
                let mut types = Vec::new();
                if !self.check(&Token::RParen) {
                    types.push(self.parse_type()?);
                    while self.eat(&Token::Comma) {
                        types.push(self.parse_type()?);
                    }
                }
                self.expect(&Token::RParen)?;
                types
            } else {
                Vec::new()
            };
            variants.push(EnumVariant { name: vname, data });
            if !self.eat(&Token::Comma) { break; }
        }
        self.eat(&Token::Semicolon);

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let vis     = self.parse_visibility()?;
            let static_ = self.eat(&Token::Static);
            let name    = self.expect_ident()?;
            let method  = self.parse_method_body(vis, static_, false, false, false, false, false, false, None, false, Vec::new(), Vec::new(), None, None, name, None)?;
            methods.push(method);
        }

        self.expect(&Token::RBrace)?;
        Ok(EnumDecl { visibility, deprecated, experimental, name, variants, methods })
    }

    fn is_type_token(&self) -> bool {
        matches!(self.current(),
            Token::TypeInteger | Token::TypeFloat | Token::TypeBoolean |
            Token::TypeString  | Token::TypeVoid  | Token::TypeNoReturn | Token::TypeList |
            Token::TypeMap     | Token::TypeHashMap | Token::TypeTreeMap |
            Token::TypePair    | Token::TypeException | Token::TypeRawPtr |
            Token::TypeU8  | Token::TypeU16 | Token::TypeU32 | Token::TypeU64 |
            Token::TypeI8  | Token::TypeI16 | Token::TypeI32 | Token::TypeI64 |
            Token::TypeArray | Token::TypeSlice |
            Token::Ident(_)
        )
    }

    fn is_fn_ptr_type_ahead(&self) -> bool {
        let mut i = self.pos;
        if !matches!(self.tokens.get(i).map(|t| &t.token), Some(&Token::LParen)) {
            return false;
        }
        i += 1;
        let mut depth = 1usize;
        while i < self.tokens.len() {
            match &self.tokens[i].token {
                Token::LParen  => { depth += 1; i += 1; }
                Token::RParen  => {
                    depth -= 1; i += 1;
                    if depth == 0 { break; }
                }
                Token::Eof => return false,
                _ => { i += 1; }
            }
        }
        matches!(self.tokens.get(i).map(|t| &t.token), Some(&Token::Arrow))
    }

    fn parse_type(&mut self) -> ParseResult<Type> {
        let ty = match self.current().clone() {
            Token::TypeInteger   => { self.advance(); Type::Integer }
            Token::TypeFloat     => { self.advance(); Type::Float   }
            Token::TypeBoolean   => { self.advance(); Type::Boolean }
            Token::TypeString    => { self.advance(); Type::Str     }
            Token::TypeVoid      => { self.advance(); Type::Void    }
            Token::TypeNoReturn  => { self.advance(); Type::NoReturn }
            Token::TypeException => { self.advance(); Type::Named("Exception".to_string()) }

            Token::TypeU8  => { self.advance(); Type::U8  }
            Token::TypeU16 => { self.advance(); Type::U16 }
            Token::TypeU32 => { self.advance(); Type::U32 }
            Token::TypeU64 => { self.advance(); Type::U64 }
            Token::TypeI8  => { self.advance(); Type::I8  }
            Token::TypeI16 => { self.advance(); Type::I16 }
            Token::TypeI32 => { self.advance(); Type::I32 }
            Token::TypeI64 => { self.advance(); Type::I64 }

            Token::TypeRawPtr => {
                self.advance();
                self.expect(&Token::Lt)?;
                let inner = self.parse_type()?;
                self.expect_close_gt()?;
                Type::RawPtr(Box::new(inner))
            }

            Token::TypeArray => {
                self.advance();
                self.expect(&Token::Lt)?;
                let elem = self.parse_type()?;
                self.expect(&Token::Comma)?;
                let size = match self.current().clone() {
                    Token::Int(n) if n >= 0 => { self.advance(); n as usize }
                    _ => {
                        let (line, col) = self.current_span();
                        return Err(ParseError::new(
                            &format!("Array size must be a non-negative integer literal, found {:?}", self.current()),
                            line, col,
                        ));
                    }
                };
                self.expect_close_gt()?;
                Type::Array(Box::new(elem), size)
            }

            Token::TypeSlice => {
                self.advance();
                self.expect(&Token::Lt)?;
                let elem = self.parse_type()?;
                self.expect_close_gt()?;
                Type::Slice(Box::new(elem))
            }

            Token::LParen => {
                self.advance();
                let mut param_tys = Vec::new();
                if !self.check(&Token::RParen) {
                    param_tys.push(self.parse_type()?);
                    while self.eat(&Token::Comma) {
                        param_tys.push(self.parse_type()?);
                    }
                }
                self.expect(&Token::RParen)?;
                self.expect(&Token::Arrow)?;
                let ret_ty = self.parse_type()?;
                Type::FnPtr(param_tys, Box::new(ret_ty))
            }

            Token::TypeList => {
                self.advance();
                self.expect(&Token::Lt)?;
                let inner = self.parse_type()?;
                self.expect_close_gt()?;
                Type::List(Box::new(inner))
            }
            Token::TypeMap | Token::TypeHashMap | Token::TypeTreeMap => {
                let is_hash = self.current() == &Token::TypeHashMap;
                let is_tree = self.current() == &Token::TypeTreeMap;
                self.advance();
                self.expect(&Token::Lt)?;
                let key = self.parse_type()?;
                self.expect(&Token::Comma)?;
                let val = self.parse_type()?;
                self.expect_close_gt()?;
                if is_hash      { Type::HashMap(Box::new(key), Box::new(val)) }
                else if is_tree { Type::TreeMap(Box::new(key), Box::new(val)) }
                else            { Type::Map(Box::new(key), Box::new(val))     }
            }
            Token::TypePair => {
                self.advance();
                self.expect(&Token::Lt)?;
                let first  = self.parse_type()?;
                self.expect(&Token::Comma)?;
                let second = self.parse_type()?;
                self.expect_close_gt()?;
                Type::Pair(Box::new(first), Box::new(second))
            }
            Token::Ident(name) => {
                self.advance();
                let name = name.clone();
                if self.check(&Token::Lt) {
                    self.advance();
                    let mut args = Vec::new();
                    args.push(self.parse_type()?);
                    while self.eat(&Token::Comma) {
                        args.push(self.parse_type()?);
                    }
                    self.expect_close_gt()?;
                    Type::Generic(name, args)
                } else {
                    Type::Named(name)
                }
            }
            _ => {
                let (line, col) = self.current_span();
                return Err(ParseError::new(
                    &format!("Expected type, found {:?}", self.current()),
                    line, col,
                ));
            }
        };

        if self.eat(&Token::Question) {
            Ok(Type::Nullable(Box::new(ty)))
        } else {
            Ok(ty)
        }
    }

    fn parse_params(&mut self) -> ParseResult<Vec<Param>> {
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();

        if !self.check(&Token::RParen) {
            params.push(self.parse_param()?);
            while self.eat(&Token::Comma) {
                params.push(self.parse_param()?);
            }
        }

        self.expect(&Token::RParen)?;
        Ok(params)
    }

    fn parse_param(&mut self) -> ParseResult<Param> {
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let ty   = self.parse_type()?;
        Ok(Param { name, ty })
    }

    fn parse_generics_decl(&mut self) -> ParseResult<Vec<GenericParam>> {
        if !self.check(&Token::Lt) { return Ok(Vec::new()); }
        self.advance();
        let mut params = vec![self.parse_generic_param()?];
        while self.eat(&Token::Comma) {
            params.push(self.parse_generic_param()?);
        }
        self.expect_close_gt()?;
        Ok(params)
    }

    fn parse_generic_param(&mut self) -> ParseResult<GenericParam> {
        let name = self.expect_ident()?;
        let bounds = if self.eat(&Token::Colon) {
            let mut b = vec![self.expect_ident()?];
            while self.eat(&Token::Plus) {
                b.push(self.expect_ident()?);
            }
            b
        } else {
            Vec::new()
        };
        Ok(GenericParam { name, bounds })
    }

    fn parse_comma_separated_class_names(&mut self) -> ParseResult<Vec<String>> {
        let mut names = vec![self.expect_class_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_class_name()?);
        }
        Ok(names)
    }

    fn parse_stmts(&mut self) -> ParseResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match self.current().clone() {
            Token::Return => {
                self.advance();
                if self.eat(&Token::Semicolon) {
                    Ok(Stmt::Return(None))
                } else {
                    let expr = self.parse_expr(0)?;
                    self.expect(&Token::Semicolon)?;
                    Ok(Stmt::Return(Some(expr)))
                }
            }

            Token::Throw => {
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::Throw(expr))
            }

            Token::If => self.parse_if(),

            Token::While => {
                self.advance();
                self.expect(&Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                self.expect(&Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(&Token::RBrace)?;
                Ok(Stmt::While { cond, body })
            }

            Token::For => self.parse_for(),

            Token::Switch => self.parse_switch(),

            Token::Try => self.parse_try_catch(),

            Token::Match => {
                let expr = self.parse_expr(0)?;
                Ok(Stmt::ExprStmt(expr))
            }

            Token::Asm => {
                self.advance();
                let content = self.parse_asm_block()?;
                Ok(Stmt::Asm(content))
            }
            Token::Defer => {
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::Defer(Box::new(expr)))
            }
            Token::Volatile => {
                self.advance();
                let ty   = self.parse_type()?;
                let name = self.expect_ident()?;
                let value = if self.eat(&Token::Eq) { Some(self.parse_expr(0)?) } else { None };
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::VarDecl { volatile: true, ty, name, value })
            }
            Token::Break    => { self.advance(); self.expect(&Token::Semicolon)?; Ok(Stmt::Break)    }
            Token::Continue => { self.advance(); self.expect(&Token::Semicolon)?; Ok(Stmt::Continue) }

            Token::LBrace => {
                self.advance();
                let stmts = self.parse_stmts()?;
                self.expect(&Token::RBrace)?;
                Ok(Stmt::Block(stmts))
            }

            _ if self.is_var_decl() => self.parse_var_decl(),

            _ => {
                let expr = self.parse_expr(0)?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::ExprStmt(expr))
            }
        }
    }

    fn parse_var_decl(&mut self) -> ParseResult<Stmt> {
        let ty   = self.parse_type()?;
        let name = self.expect_ident()?;
        let value = if self.eat(&Token::Eq) {
            Some(self.parse_expr(0)?)
        } else { None };
        self.expect(&Token::Semicolon)?;
        Ok(Stmt::VarDecl { volatile: false, ty, name, value })
    }

    fn is_var_decl(&self) -> bool {
        if self.check(&Token::Volatile) { return true; }
        if self.check(&Token::LParen) {
            return self.is_fn_ptr_type_ahead();
        }

        if !self.is_type_token() { return false; }

        match self.current() {
            Token::TypeInteger | Token::TypeFloat | Token::TypeBoolean |
            Token::TypeString  | Token::TypeVoid  | Token::TypeList    |
            Token::TypeMap     | Token::TypeHashMap | Token::TypeTreeMap |
            Token::TypePair    | Token::TypeException | Token::TypeRawPtr |
            Token::TypeU8  | Token::TypeU16 | Token::TypeU32 | Token::TypeU64 |
            Token::TypeI8  | Token::TypeI16 | Token::TypeI32 | Token::TypeI64 |
            Token::TypeArray | Token::TypeSlice => return true,
            _ => {}
        }

        if let Token::Ident(_) = self.current() {
            let next = if self.pos + 1 < self.tokens.len() {
                &self.tokens[self.pos + 1].token
            } else {
                &Token::Eof
            };
            matches!(next, Token::Ident(_) | Token::Lt | Token::Question)
        } else {
            false
        }
    }

    fn parse_if(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::If)?;
        let hint = if self.check(&Token::At) {
            self.advance();
            let ann = self.expect_ident()?;
            match ann.as_str() {
                "Likely"   => Some(BranchHint::Likely),
                "Unlikely" => Some(BranchHint::Unlikely),
                other => {
                    let (line, col) = self.current_span();
                    return Err(ParseError::new(
                        &format!("unknown if annotation '@{}' â€” only @likely and @unlikely are supported", other),
                        line, col,
                    ));
                }
            }
        } else {
            None
        };
        self.expect(&Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(&Token::RParen)?;
        self.expect(&Token::LBrace)?;
        let then = self.parse_stmts()?;
        self.expect(&Token::RBrace)?;

        let mut else_if = Vec::new();
        let mut else_   = None;

        while self.check(&Token::Else) {
            self.advance();
            if self.check(&Token::If) {
                self.advance();
                self.expect(&Token::LParen)?;
                let ei_cond = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                self.expect(&Token::LBrace)?;
                let ei_body = self.parse_stmts()?;
                self.expect(&Token::RBrace)?;
                else_if.push((ei_cond, ei_body));
            } else {
                self.expect(&Token::LBrace)?;
                else_ = Some(self.parse_stmts()?);
                self.expect(&Token::RBrace)?;
                break;
            }
        }

        Ok(Stmt::If { hint, cond, then, else_if, else_ })
    }

    fn parse_for(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::For)?;
        self.expect(&Token::LParen)?;

        let ty   = self.parse_type()?;
        let name = self.expect_ident()?;

        if self.eat(&Token::Colon) {
            let iter = self.parse_expr(0)?;
            self.expect(&Token::RParen)?;
            self.expect(&Token::LBrace)?;
            let body = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;
            Ok(Stmt::ForEach { ty, name, iter, body })
        } else {
            self.expect(&Token::Eq)?;
            let init_val = self.parse_expr(0)?;
            self.expect(&Token::Semicolon)?;
            let cond = self.parse_expr(0)?;
            self.expect(&Token::Semicolon)?;
            let step = self.parse_expr(0)?;
            self.expect(&Token::RParen)?;
            self.expect(&Token::LBrace)?;
            let body = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;

            let init = Box::new(Stmt::VarDecl {
                volatile: false,
                ty,
                name,
                value: Some(init_val),
            });

            Ok(Stmt::For { init, cond, step, body })
        }
    }

    fn parse_switch(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::Switch)?;
        self.expect(&Token::LParen)?;
        let expr = self.parse_expr(0)?;
        self.expect(&Token::RParen)?;
        self.expect(&Token::LBrace)?;

        let mut cases = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            self.expect(&Token::Case)?;
            let pattern = self.parse_expr(0)?;
            self.expect(&Token::Colon)?;
            let mut body = Vec::new();
            while !self.check(&Token::Case) && !self.check(&Token::RBrace) {
                body.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase { pattern, body });
        }

        self.expect(&Token::RBrace)?;
        Ok(Stmt::Switch { expr, cases })
    }

    fn parse_try_catch(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::Try)?;
        self.expect(&Token::LBrace)?;
        let try_body = self.parse_stmts()?;
        self.expect(&Token::RBrace)?;

        let mut catches = Vec::new();

        while self.check(&Token::Catch) {
            self.advance();
            self.expect(&Token::LParen)?;
            let exception_type = self.parse_type()?;
            let name           = self.expect_ident()?;
            self.expect(&Token::RParen)?;
            self.expect(&Token::LBrace)?;
            let body = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;
            catches.push(CatchClause { exception_type, name, body });
        }

        let finally_body = if self.check(&Token::Finally) {
            self.advance();
            self.expect(&Token::LBrace)?;
            let body = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;
            Some(body)
        } else { None };

        Ok(Stmt::TryCatch { try_body, catches, finally_body })
    }

    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<Expr> {
        let mut left = self.parse_prefix()?;

        loop {
            match self.current() {
                Token::PlusPlus => {
                    self.advance();
                    left = Expr::UnaryOp { op: UnaryOp::PostInc, expr: Box::new(left) };
                    continue;
                }
                Token::MinusMinus => {
                    self.advance();
                    left = Expr::UnaryOp { op: UnaryOp::PostDec, expr: Box::new(left) };
                    continue;
                }
                _ => {}
            }

            if self.check(&Token::LBracket) {
                self.advance();
                let idx = self.parse_expr(0)?;
                self.expect(&Token::RBracket)?;
                left = Expr::Index { object: Box::new(left), index: Box::new(idx) };
                continue;
            }

            if self.check(&Token::As) {
                if 23 < min_bp { break; }
                self.advance();
                let cast_ty = self.parse_type()?;
                left = Expr::Cast { expr: Box::new(left), ty: cast_ty };
                continue;
            }

            if self.check(&Token::Dot) {
                self.advance();
                let field = self.expect_ident()?;
                if self.check(&Token::LParen) {
                    let args = self.parse_args()?;
                    left = Expr::MethodCall {
                        object : Box::new(left),
                        method : field,
                        args,
                    };
                } else {
                    left = Expr::FieldAccess {
                        object : Box::new(left),
                        field,
                    };
                }
                continue;
            }

            if self.check(&Token::QuestionDot) {
                self.advance();
                let field = self.expect_ident()?;
                let args = if self.check(&Token::LParen) {
                    Some(self.parse_args()?)
                } else {
                    None
                };
                left = Expr::NullSafeAccess {
                    object : Box::new(left),
                    field,
                    args,
                };
                continue;
            }

            if self.check(&Token::Question) {
                if 2 < min_bp { break; }
                self.advance();
                let then  = self.parse_expr(0)?;
                self.expect(&Token::Colon)?;
                let else_ = self.parse_expr(0)?;
                left = Expr::Ternary {
                    cond  : Box::new(left),
                    then  : Box::new(then),
                    else_ : Box::new(else_),
                };
                continue;
            }

            if self.check(&Token::QuestionQuestion) {
                if 5 < min_bp { break; }
                self.advance();
                let right = self.parse_expr(6)?;
                left = Expr::NullCoalesce { left: Box::new(left), right: Box::new(right) };
                continue;
            }

            let (op, left_bp, right_bp) = match self.infix_binding_power() {
                Some(x) => x,
                None    => break,
            };

            if left_bp < min_bp { break; }

            self.advance();
            let right = self.parse_expr(right_bp)?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }

        Ok(left)
    }

    fn parse_prefix(&mut self) -> ParseResult<Expr> {
        match self.current().clone() {
            Token::Int(n)   => { self.advance(); Ok(Expr::IntLit(n))   }
            Token::Float(f) => { self.advance(); Ok(Expr::FloatLit(f)) }
            Token::Bool(b)  => { self.advance(); Ok(Expr::BoolLit(b))  }
            Token::Null     => { self.advance(); Ok(Expr::NullLit)      }

            Token::Str(_) | Token::DollarLBrace => {
                let mut parts: Vec<StringPart> = Vec::new();

                loop {
                    match self.current().clone() {
                        Token::Str(s) => {
                            self.advance();
                            parts.push(StringPart::Text(s));
                        }
                        Token::DollarLBrace => {
                            self.advance();
                            let expr = self.parse_expr(0)?;
                            self.eat(&Token::InterpolEnd);
                            parts.push(StringPart::Interp(Box::new(expr)));
                        }
                        _ => break,
                    }
                }

                if parts.len() == 1 {
                    if let StringPart::Text(s) = &parts[0] {
                        return Ok(Expr::StrLit(s.clone()));
                    }
                }
                Ok(Expr::StrInterp(parts))
            }

            Token::This  => { self.advance(); Ok(Expr::This) }
            Token::Super => {
                self.advance();
                if self.check(&Token::LParen) {
                    let args = self.parse_args()?;
                    Ok(Expr::StaticCall {
                        class  : "super".to_string(),
                        method : "__constructor__".to_string(),
                        args,
                    })
                } else {
                    Ok(Expr::Super)
                }
            }

            Token::Ident(name) => {
                self.advance();
                let name = name.clone();

                if self.check(&Token::LParen) {
                    let args = self.parse_args()?;
                    Ok(Expr::ConstructorCall { class: name, args })
                } else if self.check(&Token::Dot) {
                    self.advance();
                    let method = self.expect_ident()?;
                    if self.check(&Token::LParen) {
                        let args = self.parse_args()?;
                        Ok(Expr::StaticCall { class: name, method, args })
                    } else {
                        Ok(Expr::FieldAccess {
                            object : Box::new(Expr::Ident(name)),
                            field  : method,
                        })
                    }
                } else {
                    Ok(Expr::Ident(name))
                }
            }

            Token::StdIO | Token::StdMath | Token::StdTime | Token::StdMemory => {
                let class = match self.advance().clone() {
                    Token::StdIO     => "IO",
                    Token::StdMath   => "Math",
                    Token::StdTime   => "Time",
                    Token::StdMemory => "Memory",
                    _                => unreachable!(),
                }.to_string();
                self.expect(&Token::Dot)?;
                let member = self.expect_ident()?;
                if self.check(&Token::LParen) {
                    let args = self.parse_args()?;
                    Ok(Expr::StaticCall { class, method: member, args })
                } else {
                    Ok(Expr::FieldAccess {
                        object : Box::new(Expr::Ident(class)),
                        field  : member,
                    })
                }
            }

            Token::TypeInteger | Token::TypeFloat | Token::TypeBoolean
            | Token::TypeString | Token::TypeVoid => {
                let class = match self.current() {
                    Token::TypeInteger => "Integer",
                    Token::TypeFloat   => "Float",
                    Token::TypeBoolean => "Boolean",
                    Token::TypeString  => "String",
                    Token::TypeVoid    => "Void",
                    _                  => unreachable!(),
                }.to_string();
                self.advance();
                if self.check(&Token::Dot) {
                    self.advance();
                    let method = self.expect_ident()?;
                    if self.check(&Token::LParen) {
                        let args = self.parse_args()?;
                        Ok(Expr::StaticCall { class, method, args })
                    } else {
                        Ok(Expr::FieldAccess {
                            object : Box::new(Expr::Ident(class)),
                            field  : method,
                        })
                    }
                } else {
                    let (line, col) = self.current_span();
                    Err(ParseError::new(
                        &format!("unexpected type '{}' in expression — did you mean {}.sizeOf()?", class, class),
                        line, col,
                    ))
                }
            }

            Token::TypeU8  | Token::TypeU16 | Token::TypeU32 | Token::TypeU64 |
            Token::TypeI8  | Token::TypeI16 | Token::TypeI32 | Token::TypeI64 => {
                let class = match self.current() {
                    Token::TypeU8  => "u8",
                    Token::TypeU16 => "u16",
                    Token::TypeU32 => "u32",
                    Token::TypeU64 => "u64",
                    Token::TypeI8  => "i8",
                    Token::TypeI16 => "i16",
                    Token::TypeI32 => "i32",
                    Token::TypeI64 => "i64",
                    _              => unreachable!(),
                }.to_string();
                self.advance();
                if self.check(&Token::Dot) {
                    self.advance();
                    let method = self.expect_ident()?;
                    if self.check(&Token::LParen) {
                        let args = self.parse_args()?;
                        Ok(Expr::StaticCall { class, method, args })
                    } else {
                        Ok(Expr::FieldAccess {
                            object : Box::new(Expr::Ident(class)),
                            field  : method,
                        })
                    }
                } else {
                    let (line, col) = self.current_span();
                    Err(ParseError::new(
                        &format!("unexpected type '{}' in expression — did you mean {}.sizeOf()?", class, class),
                        line, col,
                    ))
                }
            }

            Token::TypeArray | Token::TypeSlice => {
                let class = match self.current() {
                    Token::TypeArray => "Array",
                    Token::TypeSlice => "Slice",
                    _                => unreachable!(),
                }.to_string();
                self.advance();
                if self.check(&Token::Dot) {
                    self.advance();
                    let method = self.expect_ident()?;
                    if self.check(&Token::LParen) {
                        let args = self.parse_args()?;
                        Ok(Expr::StaticCall { class, method, args })
                    } else {
                        Ok(Expr::FieldAccess {
                            object : Box::new(Expr::Ident(class)),
                            field  : method,
                        })
                    }
                } else {
                    Ok(Expr::Ident(class))
                }
            }

            Token::TypeList | Token::TypeMap | Token::TypeHashMap | Token::TypeTreeMap | Token::TypePair => {
                let class = match self.current() {
                    Token::TypeList    => "List",
                    Token::TypeMap     => "Map",
                    Token::TypeHashMap => "HashMap",
                    Token::TypeTreeMap => "TreeMap",
                    Token::TypePair    => "Pair",
                    _                  => unreachable!(),
                }.to_string();
                self.advance();
                if self.check(&Token::LParen) {
                    let args = self.parse_args()?;
                    Ok(Expr::ConstructorCall { class, args })
                } else if self.check(&Token::Dot) {
                    self.advance();
                    let method = self.expect_ident()?;
                    if self.check(&Token::LParen) {
                        let args = self.parse_args()?;
                        Ok(Expr::StaticCall { class, method, args })
                    } else {
                        Ok(Expr::FieldAccess {
                            object : Box::new(Expr::Ident(class)),
                            field  : method,
                        })
                    }
                } else {
                    Ok(Expr::Ident(class))
                }
            }

            Token::LParen => {
                self.advance();
                if self.is_lambda() {
                    self.parse_lambda()
                } else {
                    let expr = self.parse_expr(0)?;
                    self.expect(&Token::RParen)?;
                    Ok(expr)
                }
            }

            Token::Minus => {
                self.advance();
                let expr = self.parse_expr(25)?;
                Ok(Expr::UnaryOp { op: UnaryOp::Neg, expr: Box::new(expr) })
            }
            Token::Bang => {
                self.advance();
                let expr = self.parse_expr(25)?;
                Ok(Expr::UnaryOp { op: UnaryOp::Not, expr: Box::new(expr) })
            }
            Token::Tilde => {
                self.advance();
                let expr = self.parse_expr(25)?;
                Ok(Expr::UnaryOp { op: UnaryOp::BitNot, expr: Box::new(expr) })
            }

            Token::Await => {
                self.advance();
                let inner = self.parse_expr(24)?;
                Ok(Expr::Await(Box::new(inner)))
            }

            Token::Match => {
                self.advance();
                self.parse_match_expr()
            }
            Token::PlusPlus => {
                self.advance();
                let expr = self.parse_expr(25)?;
                Ok(Expr::UnaryOp { op: UnaryOp::PreInc, expr: Box::new(expr) })
            }
            Token::MinusMinus => {
                self.advance();
                let expr = self.parse_expr(25)?;
                Ok(Expr::UnaryOp { op: UnaryOp::PreDec, expr: Box::new(expr) })
            }

            _ => {
                let (line, col) = self.current_span();
                Err(ParseError::new(
                    &format!("Unexpected token in expression: {:?}", self.current()),
                    line, col,
                ))
            }
        }
    }

    fn infix_binding_power(&self) -> Option<(BinOp, u8, u8)> {
        match self.current() {
            Token::Eq       => Some((BinOp::Assign,    1,  2)),
            Token::PlusEq   => Some((BinOp::AddAssign, 1,  2)),
            Token::MinusEq  => Some((BinOp::SubAssign, 1,  2)),
            Token::StarEq   => Some((BinOp::MulAssign, 1,  2)),
            Token::SlashEq  => Some((BinOp::DivAssign, 1,  2)),
            Token::PipePipe => Some((BinOp::Or,        3,  4)),
            Token::AndAnd   => Some((BinOp::And,       5,  6)),
            Token::Pipe     => Some((BinOp::BitOr,     7,  8)),
            Token::Caret    => Some((BinOp::BitXor,    9, 10)),
            Token::Amp      => Some((BinOp::BitAnd,   11, 12)),
            Token::EqEq     => Some((BinOp::Eq,       13, 14)),
            Token::BangEq   => Some((BinOp::Ne,       13, 14)),
            Token::Lt       => Some((BinOp::Lt,       15, 16)),
            Token::LtEq     => Some((BinOp::Le,       15, 16)),
            Token::Gt       => Some((BinOp::Gt,       15, 16)),
            Token::GtEq     => Some((BinOp::Ge,       15, 16)),
            Token::LtLt     => Some((BinOp::Shl,      17, 18)),
            Token::GtGt     => Some((BinOp::Shr,      17, 18)),
            Token::Plus     => Some((BinOp::Add,      19, 20)),
            Token::Minus    => Some((BinOp::Sub,      19, 20)),
            Token::Star     => Some((BinOp::Mul,      21, 22)),
            Token::Slash    => Some((BinOp::Div,      21, 22)),
            Token::Percent  => Some((BinOp::Mod,      21, 22)),
            _               => None,
        }
    }

    fn is_lambda(&self) -> bool {
        let mut i = self.pos;
        loop {
            match &self.tokens[i].token {
                Token::RParen => {
                    return i + 1 < self.tokens.len()
                        && self.tokens[i + 1].token == Token::Arrow;
                }
                Token::Ident(_) | Token::Comma => { i += 1; }
                _ => return false,
            }
        }
    }

    fn parse_lambda(&mut self) -> ParseResult<Expr> {
        let mut params = Vec::new();

        if !self.check(&Token::RParen) {
            params.push(self.expect_ident()?);
            while self.eat(&Token::Comma) {
                params.push(self.expect_ident()?);
            }
        }

        self.expect(&Token::RParen)?;
        self.expect(&Token::Arrow)?;
        let body = self.parse_expr(0)?;

        Ok(Expr::Lambda { params, body: Box::new(body) })
    }

    fn parse_args(&mut self) -> ParseResult<Vec<Expr>> {
        self.expect(&Token::LParen)?;
        let mut args = Vec::new();

        if !self.check(&Token::RParen) {
            args.push(self.parse_expr(0)?);
            while self.eat(&Token::Comma) {
                args.push(self.parse_expr(0)?);
            }
        }

        self.expect(&Token::RParen)?;
        Ok(args)
    }

    fn parse_match_expr(&mut self) -> ParseResult<Expr> {
        let expr = self.parse_expr(0)?;
        self.expect(&Token::LBrace)?;

        let mut arms = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let pattern = self.parse_match_pattern()?;
            let guard = if self.eat(&Token::If) {
                Some(Box::new(self.parse_expr(0)?))
            } else {
                None
            };
            self.expect(&Token::FatArrow)?;
            let body = self.parse_expr(0)?;
            arms.push(MatchArm { pattern, guard, body: Box::new(body) });
            self.eat(&Token::Comma);
        }

        self.expect(&Token::RBrace)?;
        Ok(Expr::Match { expr: Box::new(expr), arms })
    }

    fn parse_match_pattern(&mut self) -> ParseResult<MatchPattern> {
        let pat = self.parse_single_match_pattern()?;
        if self.check(&Token::Pipe) {
            let mut alts = vec![pat];
            while self.eat(&Token::Pipe) {
                alts.push(self.parse_single_match_pattern()?);
            }
            return Ok(MatchPattern::Multi(alts));
        }
        Ok(pat)
    }

    fn parse_single_match_pattern(&mut self) -> ParseResult<MatchPattern> {
        // String literal pattern
        if let Token::Str(_) | Token::DollarLBrace = self.current().clone() {
            if let Token::Str(s) = self.current().clone() {
                self.advance();
                return Ok(MatchPattern::StrLit(s));
            }
        }
        if let Token::Str(s) = self.current().clone() {
            self.advance();
            return Ok(MatchPattern::StrLit(s));
        }

        // Negative integer literal
        if self.check(&Token::Minus) {
            self.advance();
            if let Token::Int(n) = self.current().clone() {
                self.advance();
                return Ok(MatchPattern::IntLit(-n));
            }
        }

        // Integer literal
        if let Token::Int(n) = self.current().clone() {
            self.advance();
            return Ok(MatchPattern::IntLit(n));
        }

        // Null literal
        if self.check(&Token::Null) {
            self.advance();
            return Ok(MatchPattern::StrLit("null".to_string()));
        }

        // Wildcard or binding
        if let Token::Ident(name) = self.current().clone() {
            // Check for enum variant: Name.Variant
            let next_is_dot = matches!(self.tokens.get(self.pos + 1), Some(st) if matches!(st.token, Token::Dot));
            if name == "_" || !next_is_dot {
                self.advance();
                return if name == "_" {
                    Ok(MatchPattern::Wildcard)
                } else {
                    Ok(MatchPattern::Binding(name))
                };
            }
        }

        // Enum variant: EnumName.Variant(bindings)
        let enum_name = self.expect_ident()?;
        self.expect(&Token::Dot)?;
        let variant = self.expect_ident()?;

        let bindings = if self.check(&Token::LParen) {
            self.advance();
            let mut names = Vec::new();
            if !self.check(&Token::RParen) {
                names.push(self.expect_ident()?);
                while self.eat(&Token::Comma) {
                    names.push(self.expect_ident()?);
                }
            }
            self.expect(&Token::RParen)?;
            names
        } else {
            Vec::new()
        };

        Ok(MatchPattern::Variant { enum_name, variant, bindings })
    }
}
