use std::collections::{HashMap, HashSet};
use crate::ast::*;

// ─────────────────────────────────────────────────────────────────────────────
// Hata yapısı
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum BorrowErrorKind {
    UseAfterMove,          // Taşınmış değişkeni kullanma
    MoveWhileBorrowed,     // İtere edilirken değişkeni taşıma
    MutationWhileBorrowed, // İtere edilirken koleksiyonu değiştirme
}

#[derive(Debug, Clone)]
pub struct BorrowError {
    pub message : String,
    pub kind    : BorrowErrorKind,
}

impl BorrowError {
    fn new(kind: BorrowErrorKind, msg: impl Into<String>) -> Self {
        BorrowError { message: msg.into(), kind }
    }
}

impl std::fmt::Display for BorrowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self.kind {
            BorrowErrorKind::UseAfterMove          => "use-after-move",
            BorrowErrorKind::MoveWhileBorrowed     => "move-while-borrowed",
            BorrowErrorKind::MutationWhileBorrowed => "mutation-while-borrowed",
        };
        write!(f, "borrow error [{}] — {}", prefix, self.message)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Drop schedule — CodeGen için her scope çıkışında hangi değişkenler drop edilir
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DropEntry {
    pub name : String,
    pub ty   : Type,
}

// ─────────────────────────────────────────────────────────────────────────────
// Değişken sahiplik durumu
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum MoveState {
    Owned,
    Moved,
}

#[derive(Debug, Clone)]
struct VarState {
    ty         : Type,
    move_state : MoveState,
    is_copy    : bool,
    decl_order : usize, // drop sırası için (LIFO)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scope
// ─────────────────────────────────────────────────────────────────────────────

struct BorrowScope {
    vars      : HashMap<String, VarState>,
    decl_seq  : usize,
}

impl BorrowScope {
    fn new() -> Self {
        BorrowScope { vars: HashMap::new(), decl_seq: 0 }
    }

    fn insert(&mut self, name: &str, ty: Type, is_copy: bool) {
        let order = self.decl_seq;
        self.decl_seq += 1;
        self.vars.insert(name.to_string(), VarState {
            ty,
            move_state : MoveState::Owned,
            is_copy,
            decl_order : order,
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BorrowChecker
// ─────────────────────────────────────────────────────────────────────────────

pub struct BorrowChecker {
    scopes        : Vec<BorrowScope>,
    pub errors    : Vec<BorrowError>,
    pub drops     : Vec<Vec<DropEntry>>,  // scope başına drop listesi (CodeGen için)
    current_class : Option<String>,
    iter_borrows  : HashSet<String>, // aktif iterasyon borrow'ları (key = var adı veya "this.field")
}

impl BorrowChecker {
    pub fn new() -> Self {
        BorrowChecker {
            scopes        : Vec::new(),
            errors        : Vec::new(),
            drops         : Vec::new(),
            current_class : None,
            iter_borrows  : HashSet::new(),
        }
    }

    pub fn check(&mut self, module: &Module) -> &[BorrowError] {
        for item in &module.items {
            match item {
                Item::Class(c)     => self.check_class(c),
                Item::Enum(e)      => self.check_enum(e),
                Item::Exception(e) => self.check_exception(e),
                Item::Interface(_) => {}
            }
        }
        &self.errors
    }

    // ── Tip yardımcıları ─────────────────────────────────────────────────────

    fn is_copy(ty: &Type) -> bool {
        matches!(ty, Type::Integer | Type::Float | Type::Boolean)
    }

    // Mutasyon yapan koleksiyon metodları
    fn is_mutating_method(method: &str) -> bool {
        matches!(method,
            "append" | "set" | "remove" | "clear" | "insert"
            | "push" | "pop" | "addFirst" | "addLast"
        )
    }

    // Bir expression'ın borrow anahtarını çıkar:
    // Ident("x")             → Some("x")
    // FieldAccess(This, "f") → Some("this.f")
    // Diğer                  → None
    fn borrow_key(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::This) => {
                Some(format!("this.{}", field))
            }
            _ => None,
        }
    }

    // ── Class / Enum / Exception giriş ──────────────────────────────────────

    fn check_class(&mut self, c: &ClassDecl) {
        self.current_class = Some(c.name.clone());

        if let Some(con) = &c.constructor.clone() {
            self.check_constructor_body(&con.params, &con.body);
        }
        for m in &c.methods.clone() {
            if let Some(body) = &m.body {
                self.check_method_body(&m.params, body);
            }
        }

        self.current_class = None;
    }

    fn check_enum(&mut self, e: &EnumDecl) {
        self.current_class = Some(e.name.clone());
        for m in &e.methods.clone() {
            if let Some(body) = &m.body {
                self.check_method_body(&m.params, body);
            }
        }
        self.current_class = None;
    }

    fn check_exception(&mut self, e: &ExceptionDecl) {
        self.current_class = Some(e.name.clone());
        if let Some(con) = &e.constructor.clone() {
            self.check_constructor_body(&con.params, &con.body);
        }
        for m in &e.methods.clone() {
            if let Some(body) = &m.body {
                self.check_method_body(&m.params, body);
            }
        }
        self.current_class = None;
    }

    fn check_constructor_body(&mut self, params: &[Param], body: &[Stmt]) {
        self.push_scope();
        for p in params {
            self.define_var(&p.name, p.ty.clone());
        }
        for s in body {
            self.check_stmt(s);
        }
        let d = self.pop_scope();
        self.drops.push(d);
    }

    fn check_method_body(&mut self, params: &[Param], body: &[Stmt]) {
        self.push_scope();
        for p in params {
            self.define_var(&p.name, p.ty.clone());
        }
        for s in body {
            self.check_stmt(s);
        }
        let d = self.pop_scope();
        self.drops.push(d);
    }

    // ── Statement kontrolü ───────────────────────────────────────────────────

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl { ty, name, value } => {
                if let Some(expr) = value {
                    // Non-copy tipler Ident'ten geliyorsa move edilir
                    self.check_expr_operand(expr, !Self::is_copy(ty));
                }
                self.define_var(name, ty.clone());
            }

            Stmt::ExprStmt(e) => {
                self.check_expr_operand(e, false);
            }

            Stmt::Return(Some(e)) => {
                // Return değeri move edilir (caller'a geçer)
                self.check_expr_operand(e, true);
            }
            Stmt::Return(None) => {}

            Stmt::Throw(e) => {
                self.check_expr_operand(e, true);
            }

            Stmt::If { cond, then, else_if, else_ } => {
                self.check_expr_operand(cond, false);

                self.push_scope();
                for s in then { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);

                for (c, body) in else_if {
                    self.check_expr_operand(c, false);
                    self.push_scope();
                    for s in body { self.check_stmt(s); }
                    let d = self.pop_scope(); self.drops.push(d);
                }

                if let Some(body) = else_ {
                    self.push_scope();
                    for s in body { self.check_stmt(s); }
                    let d = self.pop_scope(); self.drops.push(d);
                }
            }

            Stmt::While { cond, body } => {
                self.check_expr_operand(cond, false);
                self.push_scope();
                for s in body { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);
            }

            Stmt::ForEach { ty, name, iter, body } => {
                // Koleksiyonu borrow et — iterasyon süresi boyunca korunmalı
                let key = Self::borrow_key(iter);
                if let Some(ref k) = key {
                    self.iter_borrows.insert(k.clone());
                }
                self.check_expr_operand(iter, false);

                self.push_scope();
                self.define_var(name, ty.clone());
                for s in body { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);

                // Borrow'u serbest bırak
                if let Some(ref k) = key {
                    self.iter_borrows.remove(k);
                }
            }

            Stmt::For { init, cond, step, body } => {
                self.push_scope();
                self.check_stmt(init);
                self.check_expr_operand(cond, false);
                self.check_expr_operand(step, false);
                for s in body { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);
            }

            Stmt::Switch { expr, cases } => {
                self.check_expr_operand(expr, false);
                for case in cases {
                    self.push_scope();
                    for s in &case.body { self.check_stmt(s); }
                    let d = self.pop_scope(); self.drops.push(d);
                }
            }

            Stmt::TryCatch { try_body, catches, finally_body } => {
                self.push_scope();
                for s in try_body { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);

                for catch in catches {
                    self.push_scope();
                    self.define_var(&catch.name, catch.exception_type.clone());
                    for s in &catch.body { self.check_stmt(s); }
                    let d = self.pop_scope(); self.drops.push(d);
                }

                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin { self.check_stmt(s); }
                    let d = self.pop_scope(); self.drops.push(d);
                }
            }

            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts { self.check_stmt(s); }
                let d = self.pop_scope(); self.drops.push(d);
            }

            Stmt::Break | Stmt::Continue => {}
        }
    }

    // ── Expression operand kontrolü ──────────────────────────────────────────
    // do_move: true → değişken taşınıyor; false → sadece borrow (okuma)

    fn check_expr_operand(&mut self, expr: &Expr, do_move: bool) {
        match expr {
            // Local değişken
            Expr::Ident(name) => {
                if self.is_moved(name) {
                    self.error(BorrowErrorKind::UseAfterMove, format!(
                        "use of moved value '{}' — this variable was already moved",
                        name
                    ));
                    return;
                }
                if do_move {
                    if self.iter_borrows.contains(name.as_str()) {
                        self.error(BorrowErrorKind::MoveWhileBorrowed, format!(
                            "cannot move '{}' while it is being iterated",
                            name
                        ));
                        return;
                    }
                    if !self.is_var_copy(name) {
                        self.mark_moved(name);
                    }
                }
            }

            // Constructor çağrısı — argümanlar move edilir
            Expr::ConstructorCall { args, .. } => {
                for arg in args {
                    self.check_expr_operand(arg, true);
                }
            }

            // Method çağrısı — object + args borrow; mutasyon kontrolü
            Expr::MethodCall { object, method, args } => {
                if let Some(key) = Self::borrow_key(object) {
                    if self.iter_borrows.contains(&key) && Self::is_mutating_method(method) {
                        self.error(BorrowErrorKind::MutationWhileBorrowed, format!(
                            "cannot call .{}() on '{}' while it is being iterated — move the mutation after the loop",
                            method, key
                        ));
                    }
                }
                self.check_expr_operand(object, false);
                for arg in args {
                    self.check_expr_operand(arg, false);
                }
            }

            // Static çağrı — class bir değişken adı olabilir (parser sınırı)
            Expr::StaticCall { class, method, args } => {
                // class iter-borrowed bir koleksiyon ise mutasyon kontrolü
                if Self::is_mutating_method(method) && self.iter_borrows.contains(class.as_str()) {
                    self.error(BorrowErrorKind::MutationWhileBorrowed, format!(
                        "cannot call .{}() on '{}' while it is being iterated",
                        method, class
                    ));
                }
                // Use-after-move kontrolü: class bir lokal değişken ise
                if self.is_moved(class) {
                    self.error(BorrowErrorKind::UseAfterMove, format!(
                        "use of moved value '{}' — this variable was already moved",
                        class
                    ));
                }
                for arg in args {
                    self.check_expr_operand(arg, false);
                }
            }

            // Atama: sağ taraf non-copy Ident ise move edilir
            Expr::BinOp { op: BinOp::Assign, left, right } => {
                self.check_assign_target(left);
                let rhs_moves = matches!(right.as_ref(), Expr::Ident(_))
                    && !self.is_copy_expr(right);
                self.check_expr_operand(right, rhs_moves);
            }

            Expr::BinOp { left, right, .. } => {
                self.check_expr_operand(left, false);
                self.check_expr_operand(right, false);
            }

            Expr::UnaryOp { expr, .. } => {
                self.check_expr_operand(expr, false);
            }

            Expr::FieldAccess { object, .. } => {
                self.check_expr_operand(object, false);
            }

            Expr::NullSafeAccess { object, args, .. } => {
                self.check_expr_operand(object, false);
                if let Some(call_args) = args {
                    for a in call_args { self.check_expr_operand(a, false); }
                }
            }

            Expr::Ternary { cond, then, else_ } => {
                self.check_expr_operand(cond, false);
                self.check_expr_operand(then, do_move);
                self.check_expr_operand(else_, do_move);
            }

            // Lambda: body'yi borrow olarak kontrol et (capture by borrow)
            Expr::Lambda { body, .. } => {
                self.check_expr_operand(body, false);
            }

            Expr::StrInterp(parts) => {
                for part in parts {
                    if let StringPart::Interp(e) = part {
                        self.check_expr_operand(e, false);
                    }
                }
            }

            Expr::Index { object, index } => {
                self.check_expr_operand(object, false);
                self.check_expr_operand(index, false);
            }

            // Literaller ve this/super — kontrol gerekmez
            Expr::IntLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_)
            | Expr::StrLit(_) | Expr::NullLit | Expr::This | Expr::Super => {}
        }
    }

    fn check_assign_target(&mut self, expr: &Expr) {
        if let Expr::Ident(name) = expr {
            if self.is_moved(name) {
                self.error(BorrowErrorKind::UseAfterMove, format!(
                    "cannot assign to '{}' — it was already moved", name
                ));
            }
        }
    }

    fn is_copy_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::IntLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) => true,
            Expr::Ident(name) => self.is_var_copy(name),
            _ => false,
        }
    }

    // ── Scope yönetimi ───────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(BorrowScope::new());
    }

    fn pop_scope(&mut self) -> Vec<DropEntry> {
        let scope = match self.scopes.pop() {
            Some(s) => s,
            None    => return Vec::new(),
        };
        // Owned (taşınmamış) ve non-copy değişkenler drop edilir — LIFO sırası
        let mut entries: Vec<(usize, DropEntry)> = scope.vars.into_iter()
            .filter(|(_, v)| v.move_state == MoveState::Owned && !v.is_copy)
            .map(|(name, v)| (v.decl_order, DropEntry { name, ty: v.ty }))
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0)); // LIFO
        entries.into_iter().map(|(_, e)| e).collect()
    }

    fn define_var(&mut self, name: &str, ty: Type) {
        let is_copy = Self::is_copy(&ty);
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty, is_copy);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarState> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.vars.get(name) { return Some(v); }
        }
        None
    }

    fn lookup_var_mut(&mut self, name: &str) -> Option<&mut VarState> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.vars.get_mut(name) { return Some(v); }
        }
        None
    }

    fn mark_moved(&mut self, name: &str) {
        if let Some(v) = self.lookup_var_mut(name) {
            v.move_state = MoveState::Moved;
        }
    }

    fn is_moved(&self, name: &str) -> bool {
        self.lookup_var(name).map(|v| v.move_state == MoveState::Moved).unwrap_or(false)
    }

    fn is_var_copy(&self, name: &str) -> bool {
        self.lookup_var(name).map(|v| v.is_copy).unwrap_or(false)
    }

    // ── Hata yardımcısı ──────────────────────────────────────────────────────

    fn error(&mut self, kind: BorrowErrorKind, msg: String) {
        self.errors.push(BorrowError::new(kind, msg));
    }
}
