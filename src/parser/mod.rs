// ─────────────────────────────────────────────────────────────────────────────
// Arimo Lang — Parser (Pratt Parser)
// ─────────────────────────────────────────────────────────────────────────────

use crate::lexer::{Token, SpannedToken};
use crate::ast::*;

// ── Hata tipi ─────────────────────────────────────────────────────────────────
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

// ── Parser ────────────────────────────────────────────────────────────────────
pub struct Parser {
    tokens  : Vec<SpannedToken>,
    pos     : usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0 }
    }

    // ── Token yönetimi ────────────────────────────────────────────────────────

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

    fn check(&self, tok: &Token) -> bool {
        self.current() == tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.current() == tok { self.advance(); true }
        else { false }
    }

    // ── Program girişi ────────────────────────────────────────────────────────

    pub fn parse(&mut self) -> ParseResult<Module> {
        let path    = self.parse_module_decl()?;
        let imports = self.parse_imports()?;
        let items   = self.parse_items()?;
        Ok(Module { path, imports, items })
    }

    fn parse_module_decl(&mut self) -> ParseResult<String> {
        self.expect(&Token::Module)?;
        let path = self.parse_dotted_path()?;
        self.expect(&Token::Semicolon)?;
        Ok(path)
    }

    fn parse_dotted_path(&mut self) -> ParseResult<String> {
        let mut path = self.expect_ident()?;
        while self.eat(&Token::Dot) {
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

    // ── Üst düzey tanımlar ────────────────────────────────────────────────────

    fn parse_item(&mut self) -> ParseResult<Item> {
        let visibility = self.parse_visibility()?;
        let abstract_  = self.eat(&Token::Abstract);

        match self.current().clone() {
            Token::Class     => Ok(Item::Class(self.parse_class(visibility, abstract_)?)),
            Token::Interface => Ok(Item::Interface(self.parse_interface()?)),
            Token::Enum      => Ok(Item::Enum(self.parse_enum(visibility)?)),
            _ => {
                let (line, col) = self.current_span();
                Err(ParseError::new(
                    &format!("Expected class, interface or enum, found {:?}", self.current()),
                    line, col,
                ))
            }
        }
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

    // ── Class parser ──────────────────────────────────────────────────────────

    fn parse_class(&mut self, visibility: Visibility, abstract_: bool) -> ParseResult<ClassDecl> {
        self.expect(&Token::Class)?;
        let name     = self.expect_ident()?;
        let generics = self.parse_generics_decl()?;

        let extends = if self.eat(&Token::Extends) {
            Some(self.expect_ident()?)
        } else { None };

        let implements = if self.eat(&Token::Implements) {
            self.parse_comma_separated_idents()?
        } else { Vec::new() };

        self.expect(&Token::LBrace)?;

        let mut fields      = Vec::new();
        let mut constructor = None;
        let mut methods     = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let vis      = self.parse_visibility()?;
            let static_  = self.eat(&Token::Static);
            let readonly = self.eat(&Token::Readonly);
            let abstract_ = self.eat(&Token::Abstract);
            let override_ = self.eat(&Token::Override);

            match self.current().clone() {
                Token::Constructor => {
                    self.advance();
                    constructor = Some(self.parse_constructor(vis)?);
                }
                _ => {
                    // Önce Ident + LParen kontrolü — dönüş tipi olmayan metod (main gibi)
                    if let Token::Ident(iname) = self.current().clone() {
                        if self.pos + 1 < self.tokens.len() {
                            let next = &self.tokens[self.pos + 1].token;
                            if *next == Token::LParen {
                                self.advance();
                                let method = self.parse_method_body(
                                    vis, static_, abstract_, override_, iname, None
                                )?;
                                methods.push(method);
                                continue;
                            }
                        }
                        // Ident : Type → field (readonly + Ident isim kullanımı)
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

                    // Tip ile başlayan → field veya metod
                    if self.is_type_token() {
                        let ty   = self.parse_type()?;
                        let name = self.expect_ident()?;

                        if self.check(&Token::LParen) {
                            // metod
                            let method = self.parse_method_body(
                                vis, static_, abstract_, override_, name, Some(ty)
                            )?;
                            methods.push(method);
                        } else {
                            // field
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

        Ok(ClassDecl { visibility, abstract_, name, generics, extends, implements, fields, constructor, methods })
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
        visibility : Visibility,
        static_    : bool,
        abstract_  : bool,
        override_  : bool,
        name       : String,
        return_ty  : Option<Type>,
    ) -> ParseResult<Method> {
        let params = self.parse_params()?;

        // : ReturnType — eğer return_ty dışarıdan verilmediyse
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

        Ok(Method { visibility, static_, abstract_, override_, name, params, return_ty, body })
    }

    // ── Interface parser ──────────────────────────────────────────────────────

    fn parse_interface(&mut self) -> ParseResult<InterfaceDecl> {
        self.expect(&Token::Interface)?;
        let name     = self.expect_ident()?;
        let generics = self.parse_generics_decl()?;
        self.expect(&Token::LBrace)?;

        let mut methods = Vec::new();

        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let name      = self.expect_ident()?;
            let params    = self.parse_params()?;
            self.expect(&Token::Colon)?;
            let return_ty = self.parse_type()?;
            self.expect(&Token::Semicolon)?;

            methods.push(Method {
                visibility : Visibility::Public,
                static_    : false,
                abstract_  : true,
                override_  : false,
                name,
                params,
                return_ty  : Some(return_ty),
                body       : None,
            });
        }

        self.expect(&Token::RBrace)?;
        Ok(InterfaceDecl { name, generics, methods })
    }

    // ── Enum parser ───────────────────────────────────────────────────────────

    fn parse_enum(&mut self, visibility: Visibility) -> ParseResult<EnumDecl> {
        self.expect(&Token::Enum)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut variants = Vec::new();
        let mut methods  = Vec::new();

        // Önce variant'ları oku — ; gelene kadar
        while !self.check(&Token::Semicolon) && !self.check(&Token::RBrace) {
            variants.push(self.expect_ident()?);
            if !self.eat(&Token::Comma) { break; }
        }
        self.eat(&Token::Semicolon);

        // Sonra metodları oku
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            let vis      = self.parse_visibility()?;
            let static_  = self.eat(&Token::Static);
            let name     = self.expect_ident()?;
            let method   = self.parse_method_body(vis, static_, false, false, name, None)?;
            methods.push(method);
        }

        self.expect(&Token::RBrace)?;
        Ok(EnumDecl { visibility, name, variants, methods })
    }

    // ── Tip parser ────────────────────────────────────────────────────────────

    fn is_type_token(&self) -> bool {
        matches!(self.current(),
            Token::TypeInteger | Token::TypeFloat | Token::TypeBoolean |
            Token::TypeString  | Token::TypeVoid  | Token::TypeList    |
            Token::TypeMap     | Token::TypeHashMap | Token::TypeTreeMap |
            Token::TypePair    | Token::TypeException | Token::Ident(_)
        )
    }

    fn parse_type(&mut self) -> ParseResult<Type> {
        let ty = match self.current().clone() {
            Token::TypeInteger   => { self.advance(); Type::Integer }
            Token::TypeFloat     => { self.advance(); Type::Float   }
            Token::TypeBoolean   => { self.advance(); Type::Boolean }
            Token::TypeString    => { self.advance(); Type::Str     }
            Token::TypeVoid      => { self.advance(); Type::Void    }
            Token::TypeException => { self.advance(); Type::Named("Exception".to_string()) }

            Token::TypeList => {
                self.advance();
                self.expect(&Token::Lt)?;
                let inner = self.parse_type()?;
                self.expect(&Token::Gt)?;
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
                self.expect(&Token::Gt)?;
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
                self.expect(&Token::Gt)?;
                Type::Pair(Box::new(first), Box::new(second))
            }
            Token::Ident(name) => {
                self.advance();
                let name = name.clone();
                // Generics var mı? — Pair<First, Second>
                if self.check(&Token::Lt) {
                    self.advance();
                    let mut args = Vec::new();
                    args.push(self.parse_type()?);
                    while self.eat(&Token::Comma) {
                        args.push(self.parse_type()?);
                    }
                    self.expect(&Token::Gt)?;
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

        // Nullable? — String?
        if self.eat(&Token::Question) {
            Ok(Type::Nullable(Box::new(ty)))
        } else {
            Ok(ty)
        }
    }

    // ── Parametre listesi ─────────────────────────────────────────────────────

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
        // Arimo param sözdizimi: name: Type
        // Önce isim gel, sonra : , sonra tip
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let ty   = self.parse_type()?;
        Ok(Param { name, ty })
    }

    // ── Generics tanımı ───────────────────────────────────────────────────────

    fn parse_generics_decl(&mut self) -> ParseResult<Vec<String>> {
        if !self.check(&Token::Lt) { return Ok(Vec::new()); }
        self.advance();
        let mut params = vec![self.expect_ident()?];
        while self.eat(&Token::Comma) {
            params.push(self.expect_ident()?);
        }
        self.expect(&Token::Gt)?;
        Ok(params)
    }

    fn parse_comma_separated_idents(&mut self) -> ParseResult<Vec<String>> {
        let mut names = vec![self.expect_ident()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_ident()?);
        }
        Ok(names)
    }

    // ── Statement parser ──────────────────────────────────────────────────────

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

            Token::Break    => { self.advance(); self.expect(&Token::Semicolon)?; Ok(Stmt::Break)    }
            Token::Continue => { self.advance(); self.expect(&Token::Semicolon)?; Ok(Stmt::Continue) }

            Token::LBrace => {
                self.advance();
                let stmts = self.parse_stmts()?;
                self.expect(&Token::RBrace)?;
                Ok(Stmt::Block(stmts))
            }

            // Tip ile başlayan → değişken tanımı (is_var_decl kontrolü)
            _ if self.is_var_decl() => self.parse_var_decl(),

            // Diğer → expression statement
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
        Ok(Stmt::VarDecl { ty, name, value })
    }

    // Ident + Ident ise var decl, değilse expr stmt
    fn is_var_decl(&self) -> bool {
        // Mevcut token bir tip token (built-in veya Ident)
        // Bir sonraki token da Ident (değişken ismi) ise → var decl
        if !self.is_type_token() { return false; }

        // Özel tipler için direkt true
        match self.current() {
            Token::TypeInteger | Token::TypeFloat | Token::TypeBoolean |
            Token::TypeString  | Token::TypeVoid  | Token::TypeList    |
            Token::TypeMap     | Token::TypeHashMap | Token::TypeTreeMap |
            Token::TypePair    | Token::TypeException => return true,
            _ => {}
        }

        // Ident ise: arkasına bak
        // Ident + Ident → var decl (MyClass name = ...)
        // Ident + Dot   → static call, expr stmt
        // Ident + LParen → constructor, expr stmt
        // Ident + Lt    → generic tip (MyClass<T> name = ...)
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

        Ok(Stmt::If { cond, then, else_if, else_ })
    }

    fn parse_for(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::For)?;
        self.expect(&Token::LParen)?;

        // for-each mi klasik for mu?
        // for (Type name : expr) → for-each
        // for (Type name = ...; ...; ...) → klasik
        let ty   = self.parse_type()?;
        let name = self.expect_ident()?;

        if self.eat(&Token::Colon) {
            // for-each
            let iter = self.parse_expr(0)?;
            self.expect(&Token::RParen)?;
            self.expect(&Token::LBrace)?;
            let body = self.parse_stmts()?;
            self.expect(&Token::RBrace)?;
            Ok(Stmt::ForEach { ty, name, iter, body })
        } else {
            // klasik for
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

    // ── Expression parser — Pratt ─────────────────────────────────────────────

    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<Expr> {
        let mut left = self.parse_prefix()?;

        loop {
            // Postfix operatörler — x++ x--
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

            // Alan erişimi ve metod çağrısı — obj.field  obj.method()
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

            // Null-safe erişim/çağrı — obj?.field  obj?.method()
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

            // Ternary — cond ? then : else
            if self.check(&Token::Question) {
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

            // Binary operatör
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
            // Literaller
            Token::Int(n)   => { self.advance(); Ok(Expr::IntLit(n))   }
            Token::Float(f) => { self.advance(); Ok(Expr::FloatLit(f)) }
            Token::Bool(b)  => { self.advance(); Ok(Expr::BoolLit(b))  }
            Token::Null     => { self.advance(); Ok(Expr::NullLit)      }

            // String — interpolation parçalarını birleştir
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
                            // InterpolEnd token'ı yut
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

            // this / super
            Token::This  => { self.advance(); Ok(Expr::This)  }
            Token::Super => { self.advance(); Ok(Expr::Super) }

            // Identifier — değişken, static çağrı, constructor çağrısı
            Token::Ident(name) => {
                self.advance();
                let name = name.clone();

                if self.check(&Token::LParen) {
                    // Constructor çağrısı — Task(...)
                    let args = self.parse_args()?;
                    Ok(Expr::ConstructorCall { class: name, args })
                } else if self.check(&Token::Dot) {
                    // Static çağrı — Task.create(...)
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

            // Standart kütüphane — IO.print(...)  Math.sqrt(...)  Time.now()
            Token::StdIO | Token::StdMath | Token::StdTime => {
                let class = match self.advance().clone() {
                    Token::StdIO   => "IO",
                    Token::StdMath => "Math",
                    Token::StdTime => "Time",
                    _              => unreachable!(),
                }.to_string();
                self.expect(&Token::Dot)?;
                let method = self.expect_ident()?;
                let args   = self.parse_args()?;
                Ok(Expr::StaticCall { class, method, args })
            }

            // Parantez — (expr)
            Token::LParen => {
                self.advance();
                // Lambda mı? — (a, b) -> ...
                if self.is_lambda() {
                    self.parse_lambda()
                } else {
                    let expr = self.parse_expr(0)?;
                    self.expect(&Token::RParen)?;
                    Ok(expr)
                }
            }

            // Prefix unary operatörler
            Token::Minus => {
                self.advance();
                let expr = self.parse_expr(20)?;
                Ok(Expr::UnaryOp { op: UnaryOp::Neg, expr: Box::new(expr) })
            }
            Token::Bang => {
                self.advance();
                let expr = self.parse_expr(20)?;
                Ok(Expr::UnaryOp { op: UnaryOp::Not, expr: Box::new(expr) })
            }
            Token::PlusPlus => {
                self.advance();
                let expr = self.parse_expr(20)?;
                Ok(Expr::UnaryOp { op: UnaryOp::PreInc, expr: Box::new(expr) })
            }
            Token::MinusMinus => {
                self.advance();
                let expr = self.parse_expr(20)?;
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

    // Pratt parser — operatör öncelikleri
    fn infix_binding_power(&self) -> Option<(BinOp, u8, u8)> {
        let op = match self.current() {
            Token::Eq      => return Some((BinOp::Assign,    1,  2)),
            Token::PlusEq  => return Some((BinOp::AddAssign, 1,  2)),
            Token::MinusEq => return Some((BinOp::SubAssign, 1,  2)),
            Token::StarEq  => return Some((BinOp::MulAssign, 1,  2)),
            Token::SlashEq => return Some((BinOp::DivAssign, 1,  2)),
            Token::PipePipe => return Some((BinOp::Or,       3,  4)),
            Token::AndAnd   => return Some((BinOp::And,      5,  6)),
            Token::EqEq    => return Some((BinOp::Eq,        7,  8)),
            Token::BangEq  => return Some((BinOp::Ne,        7,  8)),
            Token::Lt      => return Some((BinOp::Lt,        9, 10)),
            Token::LtEq    => return Some((BinOp::Le,        9, 10)),
            Token::Gt      => return Some((BinOp::Gt,        9, 10)),
            Token::GtEq    => return Some((BinOp::Ge,        9, 10)),
            Token::Plus    => return Some((BinOp::Add,      11, 12)),
            Token::Minus   => return Some((BinOp::Sub,      11, 12)),
            Token::Star    => return Some((BinOp::Mul,      13, 14)),
            Token::Slash   => return Some((BinOp::Div,      13, 14)),
            Token::Percent => return Some((BinOp::Mod,      13, 14)),
            _ => return None,
        };
    }

    // Lambda tespiti — (a, b) -> ... ya da (a) -> ...
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
}