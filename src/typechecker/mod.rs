use std::collections::HashMap;
use crate::ast::*;

// ─────────────────────────────────────────────────────────────────────────────
// Hata Yapısı
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Span {
    pub line : usize,
    pub col  : usize,
}

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message  : String,
    pub hint     : Option<String>,
    pub location : Option<Span>,
}

impl TypeError {
    fn new(msg: impl Into<String>) -> Self {
        TypeError { message: msg.into(), hint: None, location: None }
    }

    fn with_hint(msg: impl Into<String>, hint: impl Into<String>) -> Self {
        TypeError { message: msg.into(), hint: Some(hint.into()), location: None }
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.location {
            Some(s) => write!(f, "type error at {}:{} — {}", s.line, s.col, self.message)?,
            None    => write!(f, "type error — {}", self.message)?,
        }
        if let Some(h) = &self.hint {
            write!(f, "\n  hint: {}", h)?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sembol Tablosu
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ClassKind {
    Concrete,
    Abstract,
    Interface,
    Enum,
    Exception,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub kind        : ClassKind,
    pub generics    : Vec<String>,
    pub extends     : Option<String>,
    pub implements  : Vec<String>,
    pub fields      : HashMap<String, FieldInfo>,
    pub methods     : HashMap<String, Vec<MethodInfo>>,
    pub constructor : Option<ConstructorInfo>,
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub ty       : Type,
    pub readonly : bool,
    pub static_  : bool,
    pub vis      : Visibility,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub params    : Vec<(String, Type)>,
    pub return_ty : Option<Type>,
    pub static_   : bool,
    pub abstract_ : bool,
    pub vis       : Visibility,
}

#[derive(Debug, Clone)]
pub struct ConstructorInfo {
    pub params : Vec<(String, Type)>,
    pub vis    : Visibility,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scope — local değişkenler
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct VarInfo {
    ty            : Type,
    non_null_cast : bool,
    readonly      : bool,
}

#[derive(Debug, Clone)]
struct Scope {
    vars : HashMap<String, VarInfo>,
}

impl Scope {
    fn new() -> Self {
        Scope { vars: HashMap::new() }
    }

    fn insert(&mut self, name: &str, ty: Type, readonly: bool) {
        self.vars.insert(name.to_string(), VarInfo { ty, non_null_cast: false, readonly });
    }

    fn get(&self, name: &str) -> Option<&VarInfo> {
        self.vars.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut VarInfo> {
        self.vars.get_mut(name)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TypeChecker
// ─────────────────────────────────────────────────────────────────────────────

pub struct TypeChecker {
    classes            : HashMap<String, ClassInfo>,
    pub errors         : Vec<TypeError>,
    current_class      : Option<String>,
    current_return_ty  : Option<Option<Type>>,
    scopes             : Vec<Scope>,
    in_constructor     : bool,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut tc = TypeChecker {
            classes           : HashMap::new(),
            errors            : Vec::new(),
            current_class     : None,
            current_return_ty : None,
            scopes            : Vec::new(),
            in_constructor    : false,
        };
        tc.register_builtins();
        tc
    }

    // ── Giriş noktası ────────────────────────────────────────────────────────

    pub fn check(&mut self, module: &Module) -> &[TypeError] {
        self.collect_symbols(module);
        for item in &module.items {
            match item {
                Item::Class(c)     => self.check_class(c),
                Item::Interface(i) => self.check_interface(i),
                Item::Enum(e)      => self.check_enum(e),
                Item::Exception(e) => self.check_exception(e),
            }
        }
        &self.errors
    }

    // ── Geçiş 1: sembol toplama ───────────────────────────────────────────────

    fn collect_symbols(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Class(c)     => self.register_class(c),
                Item::Interface(i) => self.register_interface(i),
                Item::Enum(e)      => self.register_enum(e),
                Item::Exception(e) => self.register_exception(e),
            }
        }
    }

    fn register_class(&mut self, c: &ClassDecl) {
        let mut info = ClassInfo {
            kind        : if c.abstract_ { ClassKind::Abstract } else { ClassKind::Concrete },
            // @manual sınıflar için özel kind eklenebilir, şimdilik Concrete
            generics    : c.generics.clone(),
            extends     : c.extends.clone(),
            implements  : c.implements.clone(),
            fields      : HashMap::new(),
            methods     : HashMap::new(),
            constructor : None,
        };
        for f in &c.fields {
            info.fields.insert(f.name.clone(), FieldInfo {
                ty       : f.ty.clone(),
                readonly : f.readonly,
                static_  : f.static_,
                vis      : f.visibility.clone(),
            });
        }
        for m in &c.methods {
            let mi = MethodInfo {
                params    : m.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                return_ty : m.return_ty.clone(),
                static_   : m.static_,
                abstract_ : m.abstract_,
                vis       : m.visibility.clone(),
            };
            info.methods.entry(m.name.clone()).or_default().push(mi);
        }
        if let Some(con) = &c.constructor {
            info.constructor = Some(ConstructorInfo {
                params : con.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                vis    : con.visibility.clone(),
            });
        }
        self.classes.insert(c.name.clone(), info);
    }

    fn register_interface(&mut self, i: &InterfaceDecl) {
        let mut info = ClassInfo {
            kind        : ClassKind::Interface,
            generics    : i.generics.clone(),
            extends     : None,
            implements  : Vec::new(),
            fields      : HashMap::new(),
            methods     : HashMap::new(),
            constructor : None,
        };
        for m in &i.methods {
            let mi = MethodInfo {
                params    : m.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                return_ty : m.return_ty.clone(),
                static_   : false,
                abstract_ : true,
                vis       : Visibility::Public,
            };
            info.methods.entry(m.name.clone()).or_default().push(mi);
        }
        self.classes.insert(i.name.clone(), info);
    }

    fn register_enum(&mut self, e: &EnumDecl) {
        let mut info = ClassInfo {
            kind        : ClassKind::Enum,
            generics    : Vec::new(),
            extends     : None,
            implements  : Vec::new(),
            fields      : HashMap::new(),
            methods     : HashMap::new(),
            constructor : None,
        };
        for v in &e.variants {
            info.fields.insert(v.clone(), FieldInfo {
                ty       : Type::Named(e.name.clone()),
                readonly : true,
                static_  : true,
                vis      : Visibility::Public,
            });
        }
        for m in &e.methods {
            let mi = MethodInfo {
                params    : m.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                return_ty : m.return_ty.clone(),
                static_   : m.static_,
                abstract_ : false,
                vis       : m.visibility.clone(),
            };
            info.methods.entry(m.name.clone()).or_default().push(mi);
        }
        self.classes.insert(e.name.clone(), info);
    }

    fn register_exception(&mut self, e: &ExceptionDecl) {
        let mut info = ClassInfo {
            kind        : ClassKind::Exception,
            generics    : Vec::new(),
            extends     : Some(e.extends.clone()),
            implements  : Vec::new(),
            fields      : HashMap::new(),
            methods     : HashMap::new(),
            constructor : None,
        };
        for f in &e.fields {
            info.fields.insert(f.name.clone(), FieldInfo {
                ty       : f.ty.clone(),
                readonly : f.readonly,
                static_  : f.static_,
                vis      : f.visibility.clone(),
            });
        }
        for m in &e.methods {
            let mi = MethodInfo {
                params    : m.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                return_ty : m.return_ty.clone(),
                static_   : m.static_,
                abstract_ : false,
                vis       : m.visibility.clone(),
            };
            info.methods.entry(m.name.clone()).or_default().push(mi);
        }
        if let Some(con) = &e.constructor {
            info.constructor = Some(ConstructorInfo {
                params : con.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                vis    : con.visibility.clone(),
            });
        }
        self.classes.insert(e.name.clone(), info);
    }

    // ── Geçiş 2: detaylı kontrol ──────────────────────────────────────────────

    fn check_class(&mut self, c: &ClassDecl) {
        self.current_class = Some(c.name.clone());

        if let Some(parent) = &c.extends {
            if !self.classes.contains_key(parent.as_str()) {
                self.error(format!(
                    "class '{}' extends unknown type '{}'", c.name, parent
                ));
            } else if let Some(info) = self.classes.get(parent.as_str()) {
                if info.kind == ClassKind::Interface {
                    self.error(format!(
                        "class '{}' cannot extend interface '{}' — use implements",
                        c.name, parent
                    ));
                }
            }
        }

        for iface in &c.implements {
            match self.classes.get(iface.as_str()) {
                None => self.error(format!(
                    "class '{}' implements unknown interface '{}'", c.name, iface
                )),
                Some(info) if info.kind != ClassKind::Interface => self.error(format!(
                    "'{}' is not an interface — class '{}' cannot implement it",
                    iface, c.name
                )),
                _ => {}
            }
        }

        if !c.abstract_ {
            self.check_abstract_methods_implemented(c);
        }

        for f in &c.fields {
            // @manual sınıflarda RawPtr field'larına izin ver
            if !c.manual {
                if matches!(f.ty, Type::RawPtr(_)) {
                    self.error(format!(
                        "field '{}': RawPtr<T> only allowed in @manual classes",
                        f.name
                    ));
                }
            }
            self.check_type_exists(&f.ty, &format!("field '{}'", f.name));
            if let Some(val) = &f.value {
                let val_ty = self.infer_expr(val);
                self.check_assignable(&f.ty, &val_ty, &format!("field '{}' initializer", f.name));
            }
        }

        if let Some(con) = &c.constructor.clone() {
            self.push_scope();
            self.in_constructor = true;
            for p in &con.params {
                self.check_type_exists(&p.ty, &format!("constructor param '{}'", p.name));
                self.define_var(&p.name, p.ty.clone(), false);
            }
            self.current_return_ty = None;
            for stmt in &con.body {
                self.check_stmt(stmt);
            }
            self.in_constructor = false;
            self.pop_scope();
        }

        for m in &c.methods.clone() {
            self.check_method(m, &c.name.clone());
        }

        self.current_class = None;
    }

    fn check_interface(&mut self, i: &InterfaceDecl) {
        for m in &i.methods {
            if m.body.is_some() {
                self.error(format!(
                    "interface method '{}::{}' cannot have a body — interfaces only declare signatures",
                    i.name, m.name
                ));
            }
            if let Some(ret) = &m.return_ty {
                self.check_type_exists(ret, &format!("interface method '{}::{}' return type", i.name, m.name));
            }
            for p in &m.params {
                self.check_type_exists(&p.ty, &format!(
                    "interface method '{}::{}' param '{}'", i.name, m.name, p.name
                ));
            }
        }
    }

    fn check_enum(&mut self, e: &EnumDecl) {
        self.current_class = Some(e.name.clone());
        for m in &e.methods.clone() {
            self.check_method(m, &e.name.clone());
        }
        self.current_class = None;
    }

    fn check_exception(&mut self, e: &ExceptionDecl) {
        if !self.is_exception_subtype(&e.extends) {
            self.error(format!(
                "exception class '{}' must extend 'Exception' or another exception class, found '{}'",
                e.name, e.extends
            ));
        }

        self.current_class = Some(e.name.clone());

        if let Some(con) = &e.constructor.clone() {
            self.push_scope();
            self.in_constructor = true;
            for p in &con.params {
                self.check_type_exists(&p.ty, &format!(
                    "exception '{}' constructor param '{}'", e.name, p.name
                ));
                self.define_var(&p.name, p.ty.clone(), false);
            }
            self.current_return_ty = None;
            for stmt in &con.body {
                self.check_stmt(stmt);
            }
            self.in_constructor = false;
            self.pop_scope();
        }

        for m in &e.methods.clone() {
            self.check_method(m, &e.name.clone());
        }

        self.current_class = None;
    }

    fn check_method(&mut self, m: &Method, class_name: &str) {
        if m.abstract_ && m.body.is_some() {
            self.error(format!(
                "abstract method '{}::{}' cannot have a body",
                class_name, m.name
            ));
            return;
        }

        if !m.abstract_ && m.body.is_none() {
            self.error(format!(
                "method '{}::{}' has no body and is not abstract",
                class_name, m.name
            ));
            return;
        }

        if let Some(ret) = &m.return_ty {
            self.check_type_exists(ret, &format!("method '{}::{}' return type", class_name, m.name));
        }

        for p in &m.params {
            self.check_type_exists(&p.ty, &format!(
                "method '{}::{}' param '{}'", class_name, m.name, p.name
            ));
        }

        let body = match &m.body {
            Some(b) => b.clone(),
            None    => return,
        };

        self.push_scope();

        for p in &m.params {
            self.define_var(&p.name, p.ty.clone(), false);
        }

        self.current_return_ty = Some(m.return_ty.clone());

        for stmt in &body {
            self.check_stmt(stmt);
        }

        match &m.return_ty {
            Some(Type::Void) | None => {}
            Some(_) => {
                if !self.all_paths_return(&body) {
                    self.error(format!(
                        "method '{}::{}' does not return a value on all paths",
                        class_name, m.name
                    ));
                }
            }
        }

        self.pop_scope();
    }

    // ── Statement kontrolü ────────────────────────────────────────────────────

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl { ty, name, value } => {
                self.check_type_exists(ty, &format!("variable '{}'", name));
                if let Some(val) = value {
                    let val_ty = self.infer_expr(val);
                    self.check_assignable(ty, &val_ty, &format!("variable '{}' initializer", name));
                }
                self.define_var(name, ty.clone(), false);
            }

            Stmt::ExprStmt(e) => {
                self.infer_expr(e);
            }

            Stmt::Return(expr) => {
                let expected = self.current_return_ty.clone();
                match (expr, expected) {
                    (None, None) => {}
                    (None, Some(None)) => {}
                    (None, Some(Some(Type::Void))) => {}
                    (None, Some(Some(ret_ty))) => {
                        self.error(format!(
                            "missing return value — expected {:?}", ret_ty
                        ));
                    }
                    (Some(e), Some(Some(Type::Void))) => {
                        self.infer_expr(e);
                        self.error("cannot return a value from a Void method".to_string());
                    }
                    (Some(e), Some(Some(ret_ty))) => {
                        let actual = self.infer_expr(e);
                        self.check_assignable(&ret_ty, &actual, "return statement");
                    }
                    (Some(e), Some(None)) => {
                        self.infer_expr(e);
                        self.error("main() cannot return a value".to_string());
                    }
                    (Some(e), None) => {
                        self.infer_expr(e);
                    }
                }
            }

            Stmt::Throw(e) => {
                let ty = self.infer_expr(e);
                if !self.is_throwable(&ty) {
                    self.error(format!(
                        "can only throw exception types, found {:?}", ty
                    ));
                }
            }

            Stmt::If { cond, then, else_if, else_ } => {
                let cond_ty = self.infer_expr(cond);
                if !self.is_boolean(&cond_ty) {
                    self.error(format!(
                        "if condition must be Boolean, found {:?}", cond_ty
                    ));
                }

                let then_casts = self.extract_null_checks(cond);
                self.push_scope();
                for name in &then_casts {
                    self.apply_smart_cast(name);
                }
                for s in then {
                    self.check_stmt(s);
                }
                self.pop_scope();

                for (elif_cond, elif_body) in else_if {
                    let ec_ty = self.infer_expr(elif_cond);
                    if !self.is_boolean(&ec_ty) {
                        self.error(format!(
                            "else-if condition must be Boolean, found {:?}", ec_ty
                        ));
                    }
                    let elif_casts = self.extract_null_checks(elif_cond);
                    self.push_scope();
                    for name in &elif_casts {
                        self.apply_smart_cast(name);
                    }
                    for s in elif_body {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }

                if let Some(else_body) = else_ {
                    self.push_scope();
                    for s in else_body {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }
            }

            Stmt::While { cond, body } => {
                let cond_ty = self.infer_expr(cond);
                if !self.is_boolean(&cond_ty) {
                    self.error(format!(
                        "while condition must be Boolean, found {:?}", cond_ty
                    ));
                }
                self.push_scope();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }

            Stmt::ForEach { ty, name, iter, body } => {
                self.check_type_exists(ty, &format!("for-each variable '{}'", name));
                let iter_ty = self.infer_expr(iter);
                match &iter_ty {
                    Type::List(elem_ty) => {
                        let elem = *elem_ty.clone();
                        self.check_assignable(ty, &elem, &format!(
                            "for-each: loop variable '{}' has type {:?} but list element is {:?}",
                            name, ty, elem
                        ));
                    }
                    _ => self.error(format!(
                        "for-each requires List<T>, found {:?}", iter_ty
                    )),
                }
                self.push_scope();
                self.define_var(name, ty.clone(), false);
                for s in body {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }

            Stmt::For { init, cond, step, body } => {
                self.push_scope();
                self.check_stmt(init);
                let cond_ty = self.infer_expr(cond);
                if !self.is_boolean(&cond_ty) {
                    self.error(format!(
                        "for condition must be Boolean, found {:?}", cond_ty
                    ));
                }
                self.infer_expr(step);
                for s in body {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }

            Stmt::Switch { expr, cases } => {
                let switch_ty = self.infer_expr(expr);
                for case in cases {
                    let case_ty = self.infer_expr(&case.pattern);
                    if !self.types_compatible(&switch_ty, &case_ty) {
                        self.error(format!(
                            "switch case type {:?} is not compatible with switch expression type {:?}",
                            case_ty, switch_ty
                        ));
                    }
                    self.push_scope();
                    for s in &case.body {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }

                // Enum exhaustiveness: tüm variant'lar kapsanmalı
                if let Type::Named(enum_name) = &switch_ty {
                    let enum_name = enum_name.clone();
                    if let Some(info) = self.classes.get(&enum_name).cloned() {
                        if info.kind == ClassKind::Enum {
                            let variants: Vec<String> = info.fields.keys().cloned().collect();
                            let covered: Vec<String> = cases.iter()
                                .filter_map(|c| match &c.pattern {
                                    Expr::FieldAccess { object, field } => {
                                        if matches!(object.as_ref(), Expr::Ident(n) if n == &enum_name) {
                                            Some(field.clone())
                                        } else { None }
                                    }
                                    _ => None,
                                })
                                .collect();
                            for variant in &variants {
                                if !covered.contains(variant) {
                                    self.error(format!(
                                        "switch on enum '{}' does not cover variant '{}'",
                                        enum_name, variant
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            Stmt::TryCatch { try_body, catches, finally_body } => {
                self.push_scope();
                for s in try_body {
                    self.check_stmt(s);
                }
                self.pop_scope();

                for catch in catches {
                    self.check_type_exists(&catch.exception_type, "catch clause type");
                    match &catch.exception_type {
                        Type::Named(n) => {
                            if !self.is_throwable(&Type::Named(n.clone())) {
                                self.error(format!(
                                    "'{}' is not an exception type and cannot be caught", n
                                ));
                            }
                        }
                        _ => self.error(format!(
                            "catch clause requires a named exception type, found {:?}",
                            catch.exception_type
                        )),
                    }
                    self.push_scope();
                    self.define_var(&catch.name, catch.exception_type.clone(), false);
                    for s in &catch.body {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }

                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin {
                        self.check_stmt(s);
                    }
                    self.pop_scope();
                }
            }

            Stmt::Break | Stmt::Continue => {}

            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }
        }
    }

    // ── Expression tip çıkarımı ───────────────────────────────────────────────

    pub fn infer_expr(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::IntLit(_)   => Type::Integer,
            Expr::FloatLit(_) => Type::Float,
            Expr::BoolLit(_)  => Type::Boolean,
            Expr::StrLit(_)   => Type::Str,
            Expr::NullLit     => Type::Nullable(Box::new(Type::Named("Unknown".to_string()))),

            Expr::StrInterp(parts) => {
                for part in parts {
                    if let StringPart::Interp(e) = part {
                        self.infer_expr(e);
                    }
                }
                Type::Str
            }

            Expr::Ident(name) => {
                if let Some(var) = self.lookup_var(name) {
                    let ty        = var.ty.clone();
                    let non_null  = var.non_null_cast;
                    if non_null {
                        return self.strip_nullable(ty);
                    }
                    return ty;
                }
                if let Some(class_name) = self.current_class.clone() {
                    if let Some(info) = self.classes.get(&class_name) {
                        if let Some(fi) = info.fields.get(name.as_str()) {
                            return fi.ty.clone();
                        }
                    }
                }
                if self.classes.contains_key(name.as_str()) {
                    return Type::Named(name.clone());
                }
                self.error(format!("undefined variable '{}'", name));
                Type::Named("Error".to_string())
            }

            Expr::This => {
                match &self.current_class {
                    Some(name) => Type::Named(name.clone()),
                    None => {
                        self.error("'this' used outside of a class".to_string());
                        Type::Named("Error".to_string())
                    }
                }
            }

            Expr::Super => {
                let parent = self.current_class.as_ref()
                    .and_then(|c| self.classes.get(c.as_str()))
                    .and_then(|info| info.extends.clone());
                match parent {
                    Some(p) => Type::Named(p),
                    None => {
                        self.error("'super' used in a class with no parent".to_string());
                        Type::Named("Error".to_string())
                    }
                }
            }

            Expr::FieldAccess { object, field } => {
                let obj_ty = self.infer_expr(object);
                // Static enforcement: Ident bir class ismi ise (değişken değil) sadece static field'a izin ver
                if let Expr::Ident(ident_name) = object.as_ref() {
                    if self.lookup_var(ident_name).is_none() && self.classes.contains_key(ident_name.as_str()) {
                        if let Some(info) = self.classes.get(ident_name.as_str()) {
                            if let Some(fi) = info.fields.get(field.as_str()) {
                                if !fi.static_ {
                                    self.error(format!(
                                        "field '{}::{}' is not static — use an instance to access it",
                                        ident_name, field
                                    ));
                                }
                            }
                        }
                    }
                }
                self.resolve_field_access(&obj_ty, field, object)
            }

            Expr::NullSafeAccess { object, field, args } => {
                let obj_ty = self.infer_expr(object);
                let base   = self.strip_nullable(obj_ty.clone());
                if !matches!(obj_ty, Type::Nullable(_)) {
                    self.error(format!(
                        "null-safe access '?.' used on non-nullable type {:?} — use '.' instead",
                        obj_ty
                    ));
                }
                let result = match args {
                    None => self.resolve_field_inner(&base, field),
                    Some(args) => {
                        let arg_types: Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                        self.resolve_method_call(&base, field, &arg_types, false)
                    }
                };
                match result {
                    Type::Nullable(_) | Type::Void => result,
                    other => Type::Nullable(Box::new(other)),
                }
            }

            Expr::MethodCall { object, method, args } => {
                let obj_ty    = self.infer_expr(object);
                let arg_types : Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                self.resolve_method_call(&obj_ty, method, &arg_types, false)
            }

            Expr::StaticCall { class, method, args } => {
                let arg_types : Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                if class == "super" {
                    if !self.in_constructor {
                        self.error("super() can only be called inside a constructor".to_string());
                    }
                    return Type::Void;
                }
                // Parser Ident+Dot+method() daima StaticCall üretiyor.
                // Eğer class ismi bilinen bir class değil ama local değişkense
                // instance method call olarak yönlendir.
                if !self.classes.contains_key(class.as_str()) {
                    if let Some(var) = self.lookup_var(class) {
                        let var_ty = var.ty.clone();
                        return self.resolve_method_call(&var_ty, method, &arg_types, false);
                    }
                }
                let class_ty = Type::Named(class.clone());
                self.resolve_method_call(&class_ty, method, &arg_types, true)
            }

            Expr::ConstructorCall { class, args } => {
                // Builtin koleksiyon constructor'ları — List()  HashMap()  TreeMap()
                match class.as_str() {
                    "List" => {
                        for a in args { self.infer_expr(a); }
                        return Type::List(Box::new(Type::Named("Unknown".to_string())));
                    }
                    "HashMap" => {
                        for a in args { self.infer_expr(a); }
                        return Type::HashMap(
                            Box::new(Type::Named("Unknown".to_string())),
                            Box::new(Type::Named("Unknown".to_string())),
                        );
                    }
                    "TreeMap" => {
                        for a in args { self.infer_expr(a); }
                        return Type::TreeMap(
                            Box::new(Type::Named("Unknown".to_string())),
                            Box::new(Type::Named("Unknown".to_string())),
                        );
                    }
                    "Pair" if args.len() == 2 => {
                        let f = self.infer_expr(&args[0]);
                        let s = self.infer_expr(&args[1]);
                        return Type::Pair(Box::new(f), Box::new(s));
                    }
                    _ => {}
                }

                let arg_types : Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                match self.classes.get(class.as_str()).cloned() {
                    None => {
                        self.error(format!("unknown type '{}'", class));
                        Type::Named("Error".to_string())
                    }
                    Some(info) => {
                        if info.kind == ClassKind::Interface {
                            self.error(format!(
                                "cannot instantiate interface '{}'", class
                            ));
                        } else if info.kind == ClassKind::Abstract {
                            self.error(format!(
                                "cannot instantiate abstract class '{}'", class
                            ));
                        }
                        if let Some(con) = info.constructor.clone() {
                            if con.params.len() != arg_types.len() {
                                self.error(format!(
                                    "constructor '{}' expects {} argument(s), got {}",
                                    class, con.params.len(), arg_types.len()
                                ));
                            } else {
                                for (i, ((_, param_ty), arg_ty)) in
                                    con.params.iter().zip(arg_types.iter()).enumerate()
                                {
                                    self.check_assignable(param_ty, arg_ty, &format!(
                                        "constructor '{}' argument {}", class, i + 1
                                    ));
                                }
                            }
                        } else if !arg_types.is_empty() {
                            self.error(format!(
                                "class '{}' has no constructor but was called with {} argument(s)",
                                class, arg_types.len()
                            ));
                        }
                        Type::Named(class.clone())
                    }
                }
            }

            Expr::BinOp { op, left, right } => {
                let lt = self.infer_expr(left);
                let rt = self.infer_expr(right);
                self.infer_binop(op, &lt, &rt, left, right)
            }

            Expr::UnaryOp { op, expr } => {
                let ty = self.infer_expr(expr);
                match op {
                    UnaryOp::Not => {
                        if !self.is_boolean(&ty) {
                            self.error(format!(
                                "'!' requires Boolean, found {:?}", ty
                            ));
                        }
                        Type::Boolean
                    }
                    UnaryOp::Neg => {
                        if !self.is_numeric(&ty) {
                            self.error(format!(
                                "unary '-' requires numeric type, found {:?}", ty
                            ));
                        }
                        ty
                    }
                    UnaryOp::PreInc | UnaryOp::PreDec
                    | UnaryOp::PostInc | UnaryOp::PostDec => {
                        if !self.is_numeric(&ty) {
                            self.error(format!(
                                "'++/--' requires numeric type, found {:?}", ty
                            ));
                        }
                        ty
                    }
                }
            }

            Expr::Ternary { cond, then, else_ } => {
                let cond_ty = self.infer_expr(cond);
                if !self.is_boolean(&cond_ty) {
                    self.error(format!(
                        "ternary condition must be Boolean, found {:?}", cond_ty
                    ));
                }
                let then_ty = self.infer_expr(then);
                let else_ty = self.infer_expr(else_);
                if !self.types_compatible(&then_ty, &else_ty) {
                    self.error(format!(
                        "ternary branches have incompatible types: {:?} vs {:?}",
                        then_ty, else_ty
                    ));
                }
                then_ty
            }

            Expr::Lambda { params, body } => {
                // Lambda parametrelerini Unknown tipte scope'a ekle
                // (tam tip çıkarımı olmadan false-positive hatayı önlemek için)
                self.push_scope();
                for param in params {
                    self.define_var(param, Type::Named("Unknown".to_string()), false);
                }
                self.infer_expr(body);
                self.pop_scope();
                Type::Named("Lambda".to_string())
            }

            Expr::Index { object, index } => {
                let obj_ty = self.infer_expr(object);
                let idx_ty = self.infer_expr(index);
                match &obj_ty {
                    Type::List(elem) => {
                        if !matches!(idx_ty, Type::Integer) {
                            self.error(format!(
                                "List index must be Integer, found {:?}", idx_ty
                            ));
                        }
                        *elem.clone()
                    }
                    Type::HashMap(_, v) | Type::TreeMap(_, v) | Type::Map(_, v) => {
                        *v.clone()
                    }
                    _ => {
                        self.error(format!("type {:?} does not support indexing", obj_ty));
                        Type::Named("Error".to_string())
                    }
                }
            }
        }
    }

    // ── Binary operatör ───────────────────────────────────────────────────────

    fn infer_binop(
        &mut self,
        op    : &BinOp,
        left  : &Type,
        right : &Type,
        left_expr  : &Expr,
        _right_expr : &Expr,
    ) -> Type {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                if !self.is_numeric(left) || !self.is_numeric(right) {
                    self.error(format!(
                        "arithmetic operator requires numeric types, found {:?} and {:?}",
                        left, right
                    ));
                    return Type::Integer;
                }
                if matches!(left, Type::Float) || matches!(right, Type::Float) {
                    Type::Float
                } else {
                    Type::Integer
                }
            }

            BinOp::Eq | BinOp::Ne => {
                Type::Boolean
            }

            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if !self.is_numeric(left) && !matches!(left, Type::Str) {
                    self.error(format!(
                        "comparison requires numeric or String type, found {:?}", left
                    ));
                }
                Type::Boolean
            }

            BinOp::And | BinOp::Or => {
                if !self.is_boolean(left) || !self.is_boolean(right) {
                    self.error(format!(
                        "logical operator requires Boolean, found {:?} and {:?}",
                        left, right
                    ));
                }
                Type::Boolean
            }

            BinOp::Assign => {
                self.check_assignment_target(left_expr);
                self.check_assignable(left, right, "assignment");
                left.clone()
            }

            BinOp::AddAssign | BinOp::SubAssign | BinOp::MulAssign | BinOp::DivAssign => {
                self.check_assignment_target(left_expr);
                if !self.is_numeric(left) {
                    self.error(format!(
                        "compound assignment requires numeric type, found {:?}", left
                    ));
                }
                left.clone()
            }
        }
    }

    fn check_assignment_target(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name) => {
                if let Some(var) = self.lookup_var(name) {
                    if var.readonly {
                        self.error(format!(
                            "cannot assign to '{}' — it is declared readonly", name
                        ));
                    }
                }
            }
            Expr::FieldAccess { object, field } => {
                let obj_ty = self.infer_expr(object);
                let class_name = match &obj_ty {
                    Type::Named(n) => Some(n.clone()),
                    _ => None,
                };
                if let Some(cn) = class_name {
                    if let Some(info) = self.classes.get(&cn) {
                        if let Some(fi) = info.fields.get(field.as_str()) {
                            if fi.readonly {
                                let is_in_constructor = self.in_constructor;
                                let is_own_class = self.current_class.as_deref() == Some(&cn);
                                if !(is_in_constructor && is_own_class) {
                                    self.error(format!(
                                        "cannot assign to readonly field '{}'", field
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ── Field erişim çözümleme ────────────────────────────────────────────────

    fn resolve_field_access(&mut self, ty: &Type, field: &str, _object_expr: &Expr) -> Type {
        if let Type::Nullable(_) = ty {
            self.error(format!(
                "cannot access field '{}' on nullable type {:?} — null-check first or use '?.'",
                field, ty
            ));
            return Type::Named("Error".to_string());
        }
        self.resolve_field_inner(ty, field)
    }

    fn resolve_field_inner(&mut self, ty: &Type, field: &str) -> Type {
        let class_name = match ty {
            Type::Named(n)      => n.clone(),
            Type::List(_)       => "List".to_string(),
            Type::Map(_, _)     => "Map".to_string(),
            Type::HashMap(_, _) => "HashMap".to_string(),
            Type::TreeMap(_, _) => "TreeMap".to_string(),
            Type::Pair(_, _)    => "Pair".to_string(),
            Type::RawPtr(_)     => "RawPtr".to_string(),
            _ => {
                self.error(format!(
                    "cannot access field '{}' on type {:?}", field, ty
                ));
                return Type::Named("Error".to_string());
            }
        };

        // Lambda param veya çıkarılamayan tip — sessizce geç
        if class_name == "Unknown" {
            return Type::Named("Unknown".to_string());
        }

        if let Some(builtin) = self.resolve_builtin_member(&class_name, field, ty) {
            return builtin;
        }

        match self.classes.get(&class_name).cloned() {
            None => {
                self.error(format!("unknown type '{}'", class_name));
                Type::Named("Error".to_string())
            }
            Some(info) => {
                if let Some(fi) = info.fields.get(field) {
                    self.check_member_visibility(&class_name, field, &fi.vis);
                    return fi.ty.clone();
                }
                if let Some(parent) = info.extends.clone() {
                    let parent_ty = Type::Named(parent);
                    return self.resolve_field_inner(&parent_ty, field);
                }
                self.error(format!(
                    "field '{}' not found on type '{}'", field, class_name
                ));
                Type::Named("Error".to_string())
            }
        }
    }

    // ── Metod çözümleme ───────────────────────────────────────────────────────

    fn resolve_method_call(
        &mut self,
        ty        : &Type,
        method    : &str,
        args      : &[Type],
        is_static : bool,
    ) -> Type {
        if let Type::Nullable(_) = ty {
            self.error(format!(
                "cannot call method '{}' on nullable type {:?} — null-check first or use '?.'",
                method, ty
            ));
            return Type::Named("Error".to_string());
        }

        let class_name = match ty {
            Type::Named(n)      => n.clone(),
            Type::List(_)       => "List".to_string(),
            Type::Map(_, _)     => "Map".to_string(),
            Type::HashMap(_, _) => "HashMap".to_string(),
            Type::TreeMap(_, _) => "TreeMap".to_string(),
            Type::Pair(_, _)    => "Pair".to_string(),
            Type::RawPtr(_)     => "RawPtr".to_string(),
            _ => {
                self.error(format!(
                    "cannot call method '{}' on type {:?}", method, ty
                ));
                return Type::Named("Error".to_string());
            }
        };

        // Lambda param veya çıkarılamayan tip — sessizce geç
        if class_name == "Unknown" {
            return Type::Named("Unknown".to_string());
        }

        if let Some(ret) = self.resolve_builtin_method(&class_name, method, ty, args) {
            return ret;
        }

        let class_info = match self.classes.get(&class_name).cloned() {
            Some(info) => info,
            None => {
                self.error(format!("unknown type '{}'", class_name));
                return Type::Named("Error".to_string());
            }
        };

        match class_info.methods.get(method).cloned() {
            None => {
                if let Some(parent) = class_info.extends.clone() {
                    let parent_ty = Type::Named(parent);
                    return self.resolve_method_call(&parent_ty, method, args, is_static);
                }
                for iface in &class_info.implements.clone() {
                    let iface_ty = Type::Named(iface.clone());
                    let result = self.resolve_method_call(&iface_ty, method, args, is_static);
                    if !matches!(result, Type::Named(ref n) if n == "Error") {
                        return result;
                    }
                }
                let kind_str = if is_static { "static method" } else { "method" };
                self.error(format!(
                    "{} '{}' not found on type '{}'", kind_str, method, class_name
                ));
                Type::Named("Error".to_string())
            }
            Some(overloads) => {
                for mi in &overloads {
                    if mi.static_ != is_static {
                        continue;
                    }
                    if mi.params.len() != args.len() {
                        continue;
                    }
                    self.check_member_visibility(&class_name, method, &mi.vis.clone());
                    for (i, ((_, param_ty), arg_ty)) in
                        mi.params.iter().zip(args.iter()).enumerate()
                    {
                        self.check_assignable(param_ty, arg_ty, &format!(
                            "{}::{} argument {}", class_name, method, i + 1
                        ));
                    }
                    return mi.return_ty.clone().unwrap_or(Type::Void);
                }
                let static_mismatch = overloads.iter().any(|mi| {
                    mi.static_ != is_static && mi.params.len() == args.len()
                });
                if static_mismatch {
                    if is_static {
                        self.error(format!(
                            "method '{}::{}' is not static — call it on an instance",
                            class_name, method
                        ));
                    } else {
                        self.error(format!(
                            "method '{}::{}' is static — call it as {}::{}()",
                            class_name, method, class_name, method
                        ));
                    }
                } else {
                    self.error(format!(
                        "method '{}::{}' has no overload matching {} argument(s)",
                        class_name, method, args.len()
                    ));
                }
                Type::Named("Error".to_string())
            }
        }
    }

    // ── Built-in metod/üye çözümleme ─────────────────────────────────────────

    fn resolve_builtin_method(
        &self,
        class  : &str,
        method : &str,
        ty     : &Type,
        _args  : &[Type],
    ) -> Option<Type> {
        match (class, method) {
            ("List", "append")   => Some(Type::Void),
            ("List", "length")   => Some(Type::Integer),
            ("List", "isEmpty")  => Some(Type::Boolean),
            ("List", "take")     => Some(ty.clone()),
            ("List", "takeLast") => Some(ty.clone()),
            ("List", "filter")   => Some(ty.clone()),
            ("List", "sortedBy") => Some(ty.clone()),
            ("List", "reduce")   => Some(Type::Named("Unknown".to_string())),
            ("List", "of")       => Some(ty.clone()),
            ("List", "empty")    => Some(ty.clone()),

            ("Map" | "HashMap" | "TreeMap", "set")  => Some(Type::Void),
            ("Map" | "HashMap" | "TreeMap", "get")  => {
                match ty {
                    Type::HashMap(_, v) | Type::TreeMap(_, v) | Type::Map(_, v) =>
                        Some(Type::Nullable(v.clone())),
                    _ => None,
                }
            }
            ("Map" | "HashMap" | "TreeMap", "getOrDefault") => {
                match ty {
                    Type::HashMap(_, v) | Type::TreeMap(_, v) | Type::Map(_, v) =>
                        Some(*v.clone()),
                    _ => None,
                }
            }
            ("Map" | "HashMap" | "TreeMap", "containsKey") => Some(Type::Boolean),
            ("Map" | "HashMap" | "TreeMap", "remove")      => Some(Type::Void),
            ("Map" | "HashMap" | "TreeMap", "keys")        => {
                match ty {
                    Type::HashMap(k, _) | Type::TreeMap(k, _) | Type::Map(k, _) =>
                        Some(Type::List(k.clone())),
                    _ => Some(Type::List(Box::new(Type::Named("Unknown".to_string())))),
                }
            }
            ("Map" | "HashMap" | "TreeMap", "values") => {
                match ty {
                    Type::HashMap(_, v) | Type::TreeMap(_, v) | Type::Map(_, v) =>
                        Some(Type::List(v.clone())),
                    _ => Some(Type::List(Box::new(Type::Named("Unknown".to_string())))),
                }
            }
            ("Map" | "HashMap" | "TreeMap", "entries") =>
                Some(Type::List(Box::new(Type::Named("Unknown".to_string())))),
            ("Map" | "HashMap" | "TreeMap", "length") => Some(Type::Integer),
            ("Map" | "HashMap" | "TreeMap", "of")     => Some(ty.clone()),
            ("Map" | "HashMap" | "TreeMap", "create") => Some(ty.clone()),

            ("IO", "print") => Some(Type::Void),
            ("IO", "read")  => Some(Type::Str),

            ("Math", "sqrt") => Some(Type::Float),
            ("Math", "abs")  => Some(Type::Float),
            ("Math", "pow")  => Some(Type::Float),
            ("Math", "PI")   => Some(Type::Float),
            ("Math", "E")    => Some(Type::Float),

            ("Time", "now")        => Some(Type::Str),
            ("Time", "generateId") => Some(Type::Str),

            ("String" | "Str", "length")     => Some(Type::Integer),
            ("String" | "Str", "compareTo")  => Some(Type::Integer),
            ("String" | "Str", "contains")   => Some(Type::Boolean),
            ("String" | "Str", "startsWith") => Some(Type::Boolean),
            ("String" | "Str", "endsWith")   => Some(Type::Boolean),
            ("String" | "Str", "toUpper")    => Some(Type::Str),
            ("String" | "Str", "toLower")    => Some(Type::Str),
            ("String" | "Str", "trim")       => Some(Type::Str),
            ("String" | "Str", "split")      => Some(Type::List(Box::new(Type::Str))),

            ("Pair", "getFirst") => {
                match ty {
                    Type::Pair(f, _) => Some(*f.clone()),
                    _ => None,
                }
            }
            ("Pair", "getSecond") => {
                match ty {
                    Type::Pair(_, s) => Some(*s.clone()),
                    _ => None,
                }
            }

            // @manual — Memory stdlib
            ("Memory", "alloc")  => Some(Type::RawPtr(Box::new(Type::Void))),
            ("Memory", "free")   => Some(Type::Void),
            ("Memory", "copy")   => Some(Type::Void),
            ("Memory", "set")    => Some(Type::Void),

            // @manual — RawPtr<T> metodları
            ("RawPtr", "read") => {
                match ty {
                    Type::RawPtr(inner) => Some(*inner.clone()),
                    _ => Some(Type::Named("Unknown".to_string())),
                }
            }
            ("RawPtr", "write")  => Some(Type::Void),
            ("RawPtr", "offset") => Some(ty.clone()),

            // sizeOf — her tip için geçerli static metod
            (_, "sizeOf") => Some(Type::Integer),

            ("Exception", "message") => Some(Type::Str),
            (_, "message") => {
                if self.classes.get(class)
                    .map(|i| i.kind == ClassKind::Exception
                        || i.extends.as_deref() == Some("Exception")
                        || i.extends.as_ref().map(|p| self.is_exception_subtype(p)).unwrap_or(false))
                    .unwrap_or(false)
                {
                    Some(Type::Str)
                } else {
                    None
                }
            }

            _ => None,
        }
    }

    fn resolve_builtin_member(&self, class: &str, member: &str, _ty: &Type) -> Option<Type> {
        match (class, member) {
            ("Math", "PI") => Some(Type::Float),
            ("Math", "E")  => Some(Type::Float),
            _ => None,
        }
    }

    // ── Tip uyumluluk ─────────────────────────────────────────────────────────

    fn check_assignable(&mut self, target: &Type, source: &Type, context: &str) {
        if self.is_assignable(target, source) {
            return;
        }
        if matches!(source, Type::Nullable(_)) && !matches!(target, Type::Nullable(_)) {
            self.push_error(TypeError::with_hint(
                format!(
                    "in {}: cannot assign nullable {:?} to non-nullable {:?}",
                    context, source, target
                ),
                format!("declare the variable as {:?}? to allow null", target),
            ));
            return;
        }
        self.error(format!(
            "in {}: type mismatch — expected {:?}, found {:?}",
            context, target, source
        ));
    }

    fn is_assignable(&self, target: &Type, source: &Type) -> bool {
        if self.types_equal(target, source) {
            return true;
        }

        if let Type::Named(n) = source {
            if n == "Error" {
                return true;
            }
        }

        if matches!(target, Type::Float) && matches!(source, Type::Integer) {
            return true;
        }

        // Boş koleksiyon literal'ı (Unknown wildcard) herhangi bir koleksiyona atanabilir
        // Örn: List<Task> tasks = List()  →  List<Unknown> → List<Task> OK
        if let (Type::List(te), Type::List(ts)) = (target, source) {
            if matches!(ts.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(te, ts);
        }
        if let (Type::HashMap(tk, tv), Type::HashMap(sk, sv)) = (target, source) {
            if matches!(sk.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(tk, sk) && self.is_assignable(tv, sv);
        }
        if let (Type::TreeMap(tk, tv), Type::TreeMap(sk, sv)) = (target, source) {
            if matches!(sk.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(tk, sk) && self.is_assignable(tv, sv);
        }
        if let (Type::Map(tk, tv), Type::Map(sk, sv)) = (target, source) {
            if matches!(sk.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(tk, sk) && self.is_assignable(tv, sv);
        }
        // Map (interface) ← HashMap veya TreeMap (implementasyonlar)
        if let Type::Map(tk, tv) = target {
            let (sk, sv) = match source {
                Type::HashMap(k, v) | Type::TreeMap(k, v) => (k, v),
                _ => return false,
            };
            if matches!(sk.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(tk, sk) && self.is_assignable(tv, sv);
        }
        if let (Type::Pair(ta, tb), Type::Pair(sa, sb)) = (target, source) {
            if matches!(sa.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(ta, sa) && self.is_assignable(tb, sb);
        }

        // RawPtr<Void> herhangi bir RawPtr<T>'ye atanabilir (C'nin void* gibi)
        if let (Type::RawPtr(_), Type::RawPtr(s_inner)) = (target, source) {
            if matches!(s_inner.as_ref(), Type::Void | Type::Named(_)) { return true; }
        }

        if let Type::Nullable(t_inner) = target {
            if let Type::Nullable(s_inner) = source {
                // null literal (Nullable(Unknown)) can be assigned to any nullable
                if let Type::Named(n) = s_inner.as_ref() {
                    if n == "Unknown" {
                        return true;
                    }
                }
                return self.is_assignable(t_inner, s_inner);
            }
            // T → T?: non-nullable value is assignable to its nullable counterpart
            return self.is_assignable(t_inner, source);
        }

        if let (Type::Named(t), Type::Named(s)) = (target, source) {
            if self.is_subtype(s, t) {
                return true;
            }
        }

        if let (Type::Named(t), Type::Named(s)) = (target, source) {
            if t == "Exception" && self.is_exception_subtype(s) {
                return true;
            }
        }

        false
    }

    fn is_subtype(&self, child: &str, parent: &str) -> bool {
        if child == parent {
            return true;
        }
        if let Some(info) = self.classes.get(child) {
            if let Some(p) = &info.extends {
                if self.is_subtype(p, parent) {
                    return true;
                }
            }
            for iface in &info.implements {
                if self.is_subtype(iface, parent) {
                    return true;
                }
            }
        }
        false
    }

    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        match (a, b) {
            (Type::Integer,  Type::Integer)  => true,
            (Type::Float,    Type::Float)    => true,
            (Type::Boolean,  Type::Boolean)  => true,
            (Type::Str,      Type::Str)      => true,
            (Type::Void,     Type::Void)     => true,
            (Type::Named(a), Type::Named(b)) => a == b,
            (Type::List(a),  Type::List(b))  => self.types_equal(a, b),
            (Type::Nullable(a), Type::Nullable(b)) => self.types_equal(a, b),
            (Type::HashMap(k1, v1), Type::HashMap(k2, v2)) =>
                self.types_equal(k1, k2) && self.types_equal(v1, v2),
            (Type::TreeMap(k1, v1), Type::TreeMap(k2, v2)) =>
                self.types_equal(k1, k2) && self.types_equal(v1, v2),
            (Type::Map(k1, v1), Type::Map(k2, v2)) =>
                self.types_equal(k1, k2) && self.types_equal(v1, v2),
            (Type::Pair(a1, b1), Type::Pair(a2, b2)) =>
                self.types_equal(a1, a2) && self.types_equal(b1, b2),
            _ => false,
        }
    }

    fn types_compatible(&self, a: &Type, b: &Type) -> bool {
        self.types_equal(a, b)
            || self.is_assignable(a, b)
            || self.is_assignable(b, a)
    }

    fn is_numeric(&self, ty: &Type) -> bool {
        matches!(ty, Type::Integer | Type::Float)
    }

    fn is_boolean(&self, ty: &Type) -> bool {
        matches!(ty, Type::Boolean)
    }

    fn is_throwable(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(n) => {
                n == "Exception" || self.is_exception_subtype(n)
            }
            _ => false,
        }
    }

    fn is_exception_subtype(&self, name: &str) -> bool {
        if name == "Exception" {
            return true;
        }
        if let Some(info) = self.classes.get(name) {
            if info.kind == ClassKind::Exception {
                return true;
            }
            if let Some(parent) = &info.extends {
                return self.is_exception_subtype(parent);
            }
        }
        false
    }

    // ── Null / smart-cast ─────────────────────────────────────────────────────

    fn extract_null_checks(&self, cond: &Expr) -> Vec<String> {
        let mut names = Vec::new();
        match cond {
            Expr::BinOp { op: BinOp::Ne, left, right } => {
                if matches!(right.as_ref(), Expr::NullLit) {
                    if let Expr::Ident(name) = left.as_ref() {
                        names.push(name.clone());
                    }
                }
                if matches!(left.as_ref(), Expr::NullLit) {
                    if let Expr::Ident(name) = right.as_ref() {
                        names.push(name.clone());
                    }
                }
            }
            _ => {}
        }
        names
    }

    fn apply_smart_cast(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(var) = scope.get_mut(name) {
                var.non_null_cast = true;
                return;
            }
        }
    }

    fn check_member_visibility(&mut self, class_name: &str, member: &str, vis: &Visibility) {
        match vis {
            Visibility::Public | Visibility::Internal => {}
            Visibility::Private => {
                if self.current_class.as_deref() != Some(class_name) {
                    self.error(format!(
                        "'{}::{}' is private — cannot be accessed outside class '{}'",
                        class_name, member, class_name
                    ));
                }
            }
            Visibility::Protected => {
                let accessible = match &self.current_class {
                    None     => false,
                    Some(cc) => cc == class_name || self.is_subtype(cc, class_name),
                };
                if !accessible {
                    self.error(format!(
                        "'{}::{}' is protected — only accessible from '{}' or its subclasses",
                        class_name, member, class_name
                    ));
                }
            }
        }
    }

    fn strip_nullable(&self, ty: Type) -> Type {
        match ty {
            Type::Nullable(inner) => *inner,
            other                 => other,
        }
    }

    // ── Return path analizi ───────────────────────────────────────────────────

    fn all_paths_return(&self, stmts: &[Stmt]) -> bool {
        for stmt in stmts.iter().rev() {
            match stmt {
                Stmt::Return(_) => return true,
                Stmt::Throw(_)  => return true,

                Stmt::If { then, else_if, else_: Some(else_body), .. } => {
                    let then_ok    = self.all_paths_return(then);
                    let else_ifs_ok = else_if.iter().all(|(_, b)| self.all_paths_return(b));
                    let else_ok    = self.all_paths_return(else_body);
                    if then_ok && else_ifs_ok && else_ok {
                        return true;
                    }
                }

                Stmt::Switch { cases, .. } => {
                    if !cases.is_empty() && cases.iter().all(|c| self.all_paths_return(&c.body)) {
                        return true;
                    }
                }

                Stmt::Block(b) => {
                    if self.all_paths_return(b) {
                        return true;
                    }
                }

                Stmt::TryCatch { try_body, catches, finally_body: _ } => {
                    if self.all_paths_return(try_body)
                        && catches.iter().all(|c| self.all_paths_return(&c.body))
                    {
                        return true;
                    }
                }

                _ => {}
            }
        }
        false
    }

    // ── Abstract metod implementasyon kontrolü ────────────────────────────────

    fn check_abstract_methods_implemented(&mut self, c: &ClassDecl) {
        let ifaces: Vec<String> = c.implements.clone();
        for iface_name in ifaces {
            if let Some(iface_info) = self.classes.get(&iface_name).cloned() {
                let method_names: Vec<String> = iface_info.methods.keys().cloned().collect();
                for method_name in method_names {
                    let implemented = c.methods.iter().any(|m| m.name == method_name)
                        || self.method_in_parent(&c.extends, &method_name);
                    if !implemented {
                        self.error(format!(
                            "class '{}' implements '{}' but does not implement method '{}'",
                            c.name, iface_name, method_name
                        ));
                    }
                }
            }
        }

        if let Some(parent_name) = &c.extends.clone() {
            if let Some(parent_info) = self.classes.get(parent_name.as_str()).cloned() {
                if parent_info.kind == ClassKind::Abstract {
                    let abstract_methods: Vec<String> = parent_info.methods
                        .iter()
                        .filter(|(_, overloads)| overloads.iter().any(|m| m.abstract_))
                        .map(|(name, _)| name.clone())
                        .collect();
                    for method_name in abstract_methods {
                        if !c.methods.iter().any(|m| m.name == method_name) {
                            self.error(format!(
                                "class '{}' extends abstract class '{}' but does not implement abstract method '{}'",
                                c.name, parent_name, method_name
                            ));
                        }
                    }
                }
            }
        }
    }

    fn method_in_parent(&self, parent: &Option<String>, method_name: &str) -> bool {
        if let Some(p) = parent {
            if let Some(info) = self.classes.get(p.as_str()) {
                if info.methods.contains_key(method_name) {
                    return true;
                }
                return self.method_in_parent(&info.extends, method_name);
            }
        }
        false
    }

    // ── Tip varlık kontrolü ───────────────────────────────────────────────────

    fn check_type_exists(&mut self, ty: &Type, context: &str) {
        match ty {
            Type::Named(name) => {
                if !self.is_known_type(name) {
                    self.error(format!(
                        "unknown type '{}' in {}", name, context
                    ));
                }
            }
            Type::List(inner)  => self.check_type_exists(inner, context),
            Type::Map(k, v) | Type::HashMap(k, v) | Type::TreeMap(k, v) => {
                self.check_type_exists(k, context);
                self.check_type_exists(v, context);
            }
            Type::Pair(a, b) => {
                self.check_type_exists(a, context);
                self.check_type_exists(b, context);
            }
            Type::Nullable(inner) => self.check_type_exists(inner, context),
            Type::RawPtr(inner)   => self.check_type_exists(inner, context),
            Type::Generic(name, params) => {
                if !self.is_known_type(name) {
                    self.error(format!(
                        "unknown generic type '{}' in {}", name, context
                    ));
                }
                for p in params {
                    self.check_type_exists(p, context);
                }
            }
            _ => {}
        }
    }

    fn is_known_type(&self, name: &str) -> bool {
        matches!(
            name,
            "Integer" | "Float" | "Boolean" | "String" | "Void"
            | "IO" | "Math" | "Time" | "Memory"
            | "Exception" | "Object"
            | "List" | "Map" | "HashMap" | "TreeMap" | "Pair"
            | "RawPtr" | "Void"
            | "Lambda" | "Unknown" | "Error"
        ) || self.classes.contains_key(name)
    }

    // ── Scope yönetimi ────────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, ty: Type, readonly: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty, readonly);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    // ── Hata yardımcıları ─────────────────────────────────────────────────────

    fn error(&mut self, msg: String) {
        self.errors.push(TypeError::new(msg));
    }

    fn push_error(&mut self, err: TypeError) {
        self.errors.push(err);
    }

    // ── Built-in tipler ───────────────────────────────────────────────────────

    fn register_builtins(&mut self) {
        let mut exception_methods = HashMap::new();
        exception_methods.insert("message".to_string(), vec![MethodInfo {
            params    : Vec::new(),
            return_ty : Some(Type::Str),
            static_   : false,
            abstract_ : false,
            vis       : Visibility::Public,
        }]);
        self.classes.insert("Exception".to_string(), ClassInfo {
            kind        : ClassKind::Exception,
            generics    : Vec::new(),
            extends     : None,
            implements  : Vec::new(),
            fields      : HashMap::new(),
            methods     : exception_methods,
            constructor : Some(ConstructorInfo {
                params : vec![("message".to_string(), Type::Str)],
                vis    : Visibility::Public,
            }),
        });

        let mut object_methods = HashMap::new();
        object_methods.insert("toString".to_string(), vec![MethodInfo {
            params    : Vec::new(),
            return_ty : Some(Type::Str),
            static_   : false,
            abstract_ : false,
            vis       : Visibility::Public,
        }]);
        self.classes.insert("Object".to_string(), ClassInfo {
            kind        : ClassKind::Concrete,
            generics    : Vec::new(),
            extends     : None,
            implements  : Vec::new(),
            fields      : HashMap::new(),
            methods     : object_methods,
            constructor : None,
        });
    }
}