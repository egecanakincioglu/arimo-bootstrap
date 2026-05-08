use std::collections::{HashMap, HashSet};
use crate::ast::*;

// ─────────────────────────────────────────────────────────────────────────────
// Hata yapısı
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum BorrowErrorKind {
    UseAfterMove,
    MoveWhileBorrowed,
    MutationWhileBorrowed,
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
// Drop schedule
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
    decl_order : usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scope
// ─────────────────────────────────────────────────────────────────────────────

struct BorrowScope {
    vars     : HashMap<String, VarState>,
    decl_seq : usize,
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
    iter_borrows  : HashSet<String>,
    struct_names  : HashSet<String>,  // copy type olan struct isimleri
}

impl BorrowChecker {
    pub fn new() -> Self {
        BorrowChecker {
            scopes        : Vec::new(),
            errors        : Vec::new(),
            drops         : Vec::new(),
            current_class : None,
            iter_borrows  : HashSet::new(),
            struct_names  : HashSet::new(),
        }
    }

    pub fn check(&mut self, module: &Module) -> &[BorrowError] {
        // Struct isimlerini topla — bunlar copy type
        for item in &module.items {
            if let Item::Struct(s) = item {
                self.struct_names.insert(s.name.clone());
            }
        }
        for item in &module.items {
            match item {
                Item::Class(c)     => self.check_class(c),
                Item::Struct(s)    => self.check_struct(s),
                Item::Enum(e)      => self.check_enum(e),
                Item::Exception(e) => self.check_exception(e),
                Item::Interface(_) => {}
                Item::TypeAlias(_) => {}
            }
        }
        &self.errors
    }

    // ── Tip yardımcıları ─────────────────────────────────────────────────────

    fn is_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Integer | Type::Float | Type::Boolean |
            Type::U8 | Type::U16 | Type::U32 | Type::U64 |
            Type::I8 | Type::I16 | Type::I32 | Type::I64 => true,
            // Array<T, N> — value type, tümü kopyalanır (borrow checker katmanında)
            Type::Array(_, _) => true,
            // Slice<T> — fat pointer (ptr + len), shallow copy
            Type::Slice(_)    => true,
            // Function pointer — sadece adres, copy
            Type::FnPtr(_, _) => true,
            // Struct tipler copy semantiği
            Type::Named(n) => self.struct_names.contains(n.as_str()),
            _ => false,
        }
    }

    // Mutasyon yapan koleksiyon metodları
    fn is_mutating_method(method: &str) -> bool {
        matches!(method,
            "append" | "set" | "remove" | "clear" | "insert"
            | "push" | "pop" | "addFirst" | "addLast"
        )
    }

    fn borrow_key(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::This) => {
                Some(format!("this.{}", field))
            }
            _ => None,
        }
    }

    // ── Class / Struct / Enum / Exception giriş ──────────────────────────────

    fn check_class(&mut self, c: &ClassDecl) {
        if c.manual { return; }
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

    fn check_struct(&mut self, s: &StructDecl) {
        self.current_class = Some(s.name.clone());
        if let Some(con) = &s.constructor.clone() {
            self.check_constructor_body(&con.params, &con.body);
        }
        for m in &s.methods.clone() {
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
            let copy = self.is_copy(&p.ty);
            self.declare_var(&p.name, p.ty.clone(), copy);
        }
        for s in body { self.check_stmt(s); }
        self.pop_scope_with_drops();
    }

    fn check_method_body(&mut self, params: &[Param], body: &[Stmt]) {
        self.push_scope();
        for p in params {
            let copy = self.is_copy(&p.ty);
            self.declare_var(&p.name, p.ty.clone(), copy);
        }
        for s in body { self.check_stmt(s); }
        self.pop_scope_with_drops();
    }

    // ── Statement kontrolü ───────────────────────────────────────────────────

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl { ty, name, value } => {
                if let Some(val) = value {
                    let rhs_moves = matches!(val, Expr::Ident(_)) && !self.is_copy_expr(val);
                    self.check_expr_operand(val, rhs_moves);
                }
                let copy = self.is_copy(ty);
                self.declare_var(name, ty.clone(), copy);
            }

            Stmt::ExprStmt(e) => {
                self.check_expr_operand(e, false);
            }

            Stmt::Return(Some(e)) => {
                let moves = matches!(e, Expr::Ident(_)) && !self.is_copy_expr(e);
                self.check_expr_operand(e, moves);
            }
            Stmt::Return(None) => {}

            Stmt::Throw(e) => {
                self.check_expr_operand(e, true);
            }

            Stmt::If { cond, then, else_if, else_ } => {
                self.check_expr_operand(cond, false);
                self.push_scope();
                for s in then { self.check_stmt(s); }
                self.pop_scope_with_drops();

                for (ei_cond, ei_body) in else_if {
                    self.check_expr_operand(ei_cond, false);
                    self.push_scope();
                    for s in ei_body { self.check_stmt(s); }
                    self.pop_scope_with_drops();
                }
                if let Some(eb) = else_ {
                    self.push_scope();
                    for s in eb { self.check_stmt(s); }
                    self.pop_scope_with_drops();
                }
            }

            Stmt::While { cond, body } => {
                self.check_expr_operand(cond, false);
                self.push_scope();
                for s in body { self.check_stmt(s); }
                self.pop_scope_with_drops();
            }

            Stmt::ForEach { name, ty, iter, body } => {
                // iter borrow ediliyor — for süresi boyunca taşınamaz
                let borrow_key = match iter {
                    Expr::Ident(n) => Some(n.clone()),
                    Expr::FieldAccess { object, field }
                        if matches!(object.as_ref(), Expr::This) =>
                    {
                        Some(format!("this.{}", field))
                    }
                    _ => None,
                };
                self.check_expr_operand(iter, false);
                if let Some(key) = &borrow_key {
                    self.iter_borrows.insert(key.clone());
                }
                self.push_scope();
                let copy = self.is_copy(ty);
                self.declare_var(name, ty.clone(), copy);
                for s in body { self.check_stmt(s); }
                self.pop_scope_with_drops();
                if let Some(key) = &borrow_key {
                    self.iter_borrows.remove(key);
                }
            }

            Stmt::For { init, cond, step, body } => {
                self.push_scope();
                self.check_stmt(init);
                self.check_expr_operand(cond, false);
                self.check_expr_operand(step, false);
                for s in body { self.check_stmt(s); }
                self.pop_scope_with_drops();
            }

            Stmt::Switch { expr, cases } => {
                self.check_expr_operand(expr, false);
                for case in cases {
                    self.push_scope();
                    for s in &case.body { self.check_stmt(s); }
                    self.pop_scope_with_drops();
                }
            }

            Stmt::TryCatch { try_body, catches, finally_body } => {
                self.push_scope();
                for s in try_body { self.check_stmt(s); }
                self.pop_scope_with_drops();

                for catch in catches {
                    self.push_scope();
                    self.declare_var(&catch.name, catch.exception_type.clone(), false);
                    for s in &catch.body { self.check_stmt(s); }
                    self.pop_scope_with_drops();
                }
                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin { self.check_stmt(s); }
                    self.pop_scope_with_drops();
                }
            }

            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts { self.check_stmt(s); }
                self.pop_scope_with_drops();
            }

            Stmt::Break | Stmt::Continue => {}
        }
    }

    // ── Expression operand kontrolü ──────────────────────────────────────────

    fn check_expr_operand(&mut self, expr: &Expr, do_move: bool) {
        match expr {
            Expr::Ident(name) => {
                if self.is_moved(name) {
                    self.error(BorrowErrorKind::UseAfterMove, format!(
                        "use of moved value '{}' — this variable was already moved", name
                    ));
                    return;
                }
                if do_move {
                    if self.iter_borrows.contains(name.as_str()) {
                        self.error(BorrowErrorKind::MoveWhileBorrowed, format!(
                            "cannot move '{}' while it is being iterated", name
                        ));
                        return;
                    }
                    if !self.is_var_copy(name) {
                        self.mark_moved(name);
                    }
                }
            }

            Expr::ConstructorCall { args, .. } => {
                for arg in args { self.check_expr_operand(arg, true); }
            }

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
                for arg in args { self.check_expr_operand(arg, false); }
            }

            Expr::StaticCall { class, method, args } => {
                if Self::is_mutating_method(method) && self.iter_borrows.contains(class.as_str()) {
                    self.error(BorrowErrorKind::MutationWhileBorrowed, format!(
                        "cannot call .{}() on '{}' while it is being iterated", method, class
                    ));
                }
                if self.is_moved(class) {
                    self.error(BorrowErrorKind::UseAfterMove, format!(
                        "use of moved value '{}' — this variable was already moved", class
                    ));
                }
                for arg in args { self.check_expr_operand(arg, false); }
            }

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

            Expr::Cast { expr, .. } => {
                self.check_expr_operand(expr, do_move);
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

            Expr::Match { expr, arms } => {
                self.check_expr_operand(expr, false);
                for arm in arms {
                    // Pattern binding'ler: copy type olarak ekle (Unknown type)
                    self.push_scope();
                    match &arm.pattern {
                        MatchPattern::Variant { bindings, .. } => {
                            for b in bindings {
                                // Binding tipini bilmiyoruz — Unknown → copy olarak ekle
                                self.declare_var(b, Type::Named("Unknown".to_string()), true);
                            }
                        }
                        MatchPattern::Wildcard => {}
                    }
                    self.check_expr_operand(&arm.body, false);
                    self.pop_scope_with_drops();
                }
            }

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
            Expr::Ident(name) => self.is_var_copy(name),
            Expr::IntLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) => true,
            _ => false,
        }
    }

    // ── Scope / variable yönetimi ─────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(BorrowScope::new());
    }

    fn pop_scope_with_drops(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            let mut entries: Vec<DropEntry> = scope.vars.into_values()
                .filter(|v| !v.is_copy && v.move_state == MoveState::Owned)
                .map(|v| DropEntry { name: String::new(), ty: v.ty })
                .collect();
            entries.sort_by(|a, b| {
                // LIFO: ters sırada drop — decl_order yok ama entries az, sıralama opsiyonel
                let _ = (a, b);
                std::cmp::Ordering::Equal
            });
            self.drops.push(entries);
        }
    }

    fn declare_var(&mut self, name: &str, ty: Type, is_copy: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty, is_copy);
        }
    }

    fn is_moved(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.vars.get(name) {
                return v.move_state == MoveState::Moved;
            }
        }
        false
    }

    fn mark_moved(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.vars.get_mut(name) {
                v.move_state = MoveState::Moved;
                return;
            }
        }
    }

    fn is_var_copy(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.vars.get(name) {
                return v.is_copy;
            }
        }
        false
    }

    fn error(&mut self, kind: BorrowErrorKind, msg: String) {
        self.errors.push(BorrowError::new(kind, msg));
    }
}
