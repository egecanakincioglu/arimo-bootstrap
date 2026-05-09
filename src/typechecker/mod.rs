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
    Struct,     // value type — copy semantics, stack-allocated
    Union,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub kind         : ClassKind,
    pub generics     : Vec<String>,           // parametre isimleri: ["T", "E"]
    pub generic_bounds: HashMap<String, Vec<String>>,  // "T" → ["Drawable"]
    pub extends      : Option<String>,
    pub implements   : Vec<String>,
    pub fields       : HashMap<String, FieldInfo>,
    pub methods      : HashMap<String, Vec<MethodInfo>>,
    pub constructor  : Option<ConstructorInfo>,
    pub variant_data : HashMap<String, Vec<Type>>,  // enum varyant → veri tipleri
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
// Scope
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
    type_aliases       : HashMap<String, Type>,
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
            type_aliases      : HashMap::new(),
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
                Item::Struct(s)    => self.check_struct(s),
                Item::Interface(i) => self.check_interface(i),
                Item::Enum(e)      => self.check_enum(e),
                Item::Exception(e) => self.check_exception(e),
                Item::TypeAlias(_) => {}
                Item::Union(_)     => {}
                Item::Extern(_)    => {}
            }
        }
        &self.errors
    }

    // ── Geçiş 1: sembol toplama ───────────────────────────────────────────────

    fn collect_symbols(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Class(c)     => self.register_class(c),
                Item::Struct(s)    => self.register_struct(s),
                Item::Interface(i) => self.register_interface(i),
                Item::Enum(e)      => self.register_enum(e),
                Item::Exception(e) => self.register_exception(e),
                Item::TypeAlias(a) => {
                    self.type_aliases.insert(a.name.clone(), a.ty.clone());
                }
                Item::Union(u)  => self.register_union(u),
                Item::Extern(e) => self.register_extern(e),
            }
        }
    }

    fn register_struct(&mut self, s: &StructDecl) {
        let mut info = ClassInfo {
            kind          : ClassKind::Struct,
            generics      : s.generics.iter().map(|gp| gp.name.clone()).collect(),
            generic_bounds: s.generics.iter().filter(|gp| !gp.bounds.is_empty())
                             .map(|gp| (gp.name.clone(), gp.bounds.clone())).collect(),
            extends       : None,
            implements    : s.implements.clone(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
        };
        for f in &s.fields {
            info.fields.insert(f.name.clone(), FieldInfo {
                ty       : f.ty.clone(),
                readonly : f.readonly,
                static_  : f.static_,
                vis      : f.visibility.clone(),
            });
        }
        for m in &s.methods {
            let mi = MethodInfo {
                params    : m.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                return_ty : m.return_ty.clone(),
                static_   : m.static_,
                abstract_ : m.abstract_,
                vis       : m.visibility.clone(),
            };
            info.methods.entry(m.name.clone()).or_default().push(mi);
        }
        // Auto-constructor from fields if no explicit constructor
        if let Some(con) = &s.constructor {
            info.constructor = Some(ConstructorInfo {
                params : con.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect(),
                vis    : con.visibility.clone(),
            });
        } else if !s.fields.is_empty() {
            // Generate positional constructor: StructName(field1, field2, ...)
            let params: Vec<(String, Type)> = s.fields.iter()
                .filter(|f| !f.static_)
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect();
            if !params.is_empty() {
                info.constructor = Some(ConstructorInfo {
                    params,
                    vis : Visibility::Public,
                });
            }
        }
        self.classes.insert(s.name.clone(), info);
    }

    fn register_class(&mut self, c: &ClassDecl) {
        let mut bounds_map = HashMap::new();
        for gp in &c.generics {
            if !gp.bounds.is_empty() {
                bounds_map.insert(gp.name.clone(), gp.bounds.clone());
            }
        }
        let mut info = ClassInfo {
            kind          : if c.abstract_ { ClassKind::Abstract } else { ClassKind::Concrete },
            generics      : c.generics.iter().map(|gp| gp.name.clone()).collect(),
            generic_bounds: bounds_map,
            extends       : c.extends.clone(),
            implements    : c.implements.clone(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
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
            kind          : ClassKind::Interface,
            generics      : i.generics.iter().map(|gp| gp.name.clone()).collect(),
            generic_bounds: i.generics.iter().filter(|gp| !gp.bounds.is_empty())
                             .map(|gp| (gp.name.clone(), gp.bounds.clone())).collect(),
            extends       : None,
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
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
            kind          : ClassKind::Enum,
            generics      : Vec::new(),
            generic_bounds: HashMap::new(),
            extends       : None,
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
        };
        for v in &e.variants {
            // Her variant hem fields'ta (Priority.High erişimi için) hem variant_data'da
            info.fields.insert(v.name.clone(), FieldInfo {
                ty       : Type::Named(e.name.clone()),
                readonly : true,
                static_  : true,
                vis      : Visibility::Public,
            });
            info.variant_data.insert(v.name.clone(), v.data.clone());
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
            kind          : ClassKind::Exception,
            generics      : Vec::new(),
            generic_bounds: HashMap::new(),
            extends       : Some(e.extends.clone()),
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
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

    fn register_union(&mut self, u: &UnionDecl) {
        let mut info = ClassInfo {
            kind          : ClassKind::Union,
            generics      : Vec::new(),
            generic_bounds: HashMap::new(),
            extends       : None,
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : HashMap::new(),
            constructor   : None,
            variant_data  : HashMap::new(),
        };
        for f in &u.fields {
            info.fields.insert(f.name.clone(), FieldInfo {
                ty       : f.ty.clone(),
                readonly : false,
                static_  : false,
                vis      : f.visibility.clone(),
            });
        }
        self.classes.insert(u.name.clone(), info);
    }

    fn register_extern(&mut self, block: &ExternBlock) {
        for decl in &block.decls {
            let params: Vec<(String, Type)> = decl.params.iter()
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect();
            let mi = MethodInfo {
                params,
                return_ty : decl.return_ty.clone(),
                static_   : true,
                abstract_ : false,
                vis       : Visibility::Public,
            };
            let class_entry = self.classes.entry(format!("__extern_{}", block.abi))
                .or_insert_with(|| ClassInfo {
                    kind          : ClassKind::Concrete,
                    generics      : Vec::new(),
                    generic_bounds: HashMap::new(),
                    extends       : None,
                    implements    : Vec::new(),
                    fields        : HashMap::new(),
                    methods       : HashMap::new(),
                    constructor   : None,
                    variant_data  : HashMap::new(),
                });
            class_entry.methods.entry(decl.name.clone()).or_default().push(mi);
        }
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

    fn check_struct(&mut self, s: &StructDecl) {
        self.current_class = Some(s.name.clone());

        for iface in &s.implements {
            match self.classes.get(iface.as_str()) {
                None => self.error(format!(
                    "struct '{}' implements unknown interface '{}'", s.name, iface
                )),
                Some(info) if info.kind != ClassKind::Interface => self.error(format!(
                    "'{}' is not an interface — struct '{}' cannot implement it",
                    iface, s.name
                )),
                _ => {}
            }
        }

        for f in &s.fields {
            self.check_type_exists(&f.ty, &format!("struct field '{}'", f.name));
            if let Some(val) = &f.value {
                let val_ty = self.infer_expr(val);
                self.check_assignable(&f.ty, &val_ty, &format!("struct field '{}' initializer", f.name));
            }
        }

        if let Some(con) = &s.constructor.clone() {
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

        for m in &s.methods.clone() {
            // operator methods are validated the same as regular methods
            self.check_method(m, &s.name.clone());
        }

        self.current_class = None;
    }

    fn check_interface(&mut self, i: &InterfaceDecl) {
        for m in &i.methods {
            if m.body.is_some() {
                self.error(format!(
                    "interface method '{}::{}' cannot have a body",
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
            Some(Type::Void) | None | Some(Type::NoReturn) => {}
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
            Stmt::VarDecl { ty, name, value, .. } => {
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

            Stmt::Asm(_) => {}

            Stmt::Defer(expr) => {
                self.infer_expr(expr);
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
                    let ty       = var.ty.clone();
                    let non_null = var.non_null_cast;
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
                // Stdlib isimleri expression context'te Ident olarak gelebilir (Math.PI gibi)
                if matches!(name.as_str(), "IO" | "Math" | "Time" | "Memory") {
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
                // Enum variant constructor: Shape.Circle(1.5) veya Result.Ok("hello")
                if let Some(result_ty) = self.try_enum_variant_constructor(class, method, &arg_types) {
                    return result_ty;
                }
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
                // Function pointer çağrısı: pred(42) → class = "pred", local var of FnPtr type
                if let Some(var) = self.lookup_var(class) {
                    let var_ty = var.ty.clone();
                    if let Type::FnPtr(param_tys, ret_ty) = var_ty {
                        let arg_types: Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                        if arg_types.len() != param_tys.len() {
                            self.error(format!(
                                "function call '{}' expects {} argument(s), got {}",
                                class, param_tys.len(), arg_types.len()
                            ));
                        } else {
                            for (i, (pt, at)) in param_tys.iter().zip(arg_types.iter()).enumerate() {
                                self.check_assignable(pt, at, &format!("fn call '{}' arg {}", class, i + 1));
                            }
                        }
                        return *ret_ty;
                    }
                }

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
                            self.error(format!("cannot instantiate interface '{}'", class));
                        } else if info.kind == ClassKind::Abstract {
                            self.error(format!("cannot instantiate abstract class '{}'", class));
                        }
                        // Struct: copy type — oluşturma normaldir
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
                            self.error(format!("'!' requires Boolean, found {:?}", ty));
                        }
                        Type::Boolean
                    }
                    UnaryOp::Neg => {
                        if !self.is_numeric(&ty) {
                            self.error(format!("unary '-' requires numeric type, found {:?}", ty));
                        }
                        ty
                    }
                    UnaryOp::BitNot => {
                        if !self.is_integer_type(&ty) {
                            self.error(format!("'~' requires integer type, found {:?}", ty));
                        }
                        ty
                    }
                    UnaryOp::PreInc | UnaryOp::PreDec
                    | UnaryOp::PostInc | UnaryOp::PostDec => {
                        if !self.is_numeric(&ty) {
                            self.error(format!("'++/--' requires numeric type, found {:?}", ty));
                        }
                        ty
                    }
                }
            }

            Expr::Cast { expr, ty } => {
                let source = self.infer_expr(expr);
                if !self.is_valid_cast(&source, ty) {
                    self.error(format!(
                        "invalid cast from {:?} to {:?} — only numeric casts and pointer casts are valid",
                        source, ty
                    ));
                }
                ty.clone()
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
                self.push_scope();
                for param in params {
                    self.define_var(param, Type::Named("Unknown".to_string()), false);
                }
                self.infer_expr(body);
                self.pop_scope();
                Type::Named("Lambda".to_string())
            }

            Expr::Match { expr, arms } => {
                let scrutinee_ty = self.infer_expr(expr);
                let mut result_ty: Option<Type> = None;

                for arm in arms {
                    match &arm.pattern {
                        MatchPattern::Wildcard => {
                            let arm_ty = self.infer_expr(&arm.body);
                            if result_ty.is_none() { result_ty = Some(arm_ty); }
                        }
                        MatchPattern::Variant { enum_name, variant, bindings } => {
                            self.push_scope();
                            let binding_tys = self.resolve_variant_binding_types(
                                enum_name, variant, &scrutinee_ty
                            );
                            for (i, b) in bindings.iter().enumerate() {
                                let ty = binding_tys.get(i).cloned()
                                    .unwrap_or_else(|| Type::Named("Unknown".to_string()));
                                self.define_var(b, ty, false);
                            }
                            let arm_ty = self.infer_expr(&arm.body);
                            self.pop_scope();
                            if result_ty.is_none() { result_ty = Some(arm_ty); }
                        }
                    }
                }

                result_ty.unwrap_or(Type::Void)
            }

            Expr::Index { object, index } => {
                let obj_ty = self.infer_expr(object);
                let idx_ty = self.infer_expr(index);
                match &obj_ty {
                    Type::List(elem) => {
                        if !self.is_integer_type(&idx_ty) {
                            self.error(format!("List index must be integer type, found {:?}", idx_ty));
                        }
                        *elem.clone()
                    }
                    Type::Array(elem, _) => {
                        if !self.is_integer_type(&idx_ty) {
                            self.error(format!("Array index must be integer type, found {:?}", idx_ty));
                        }
                        *elem.clone()
                    }
                    Type::Slice(elem) => {
                        if !self.is_integer_type(&idx_ty) {
                            self.error(format!("Slice index must be integer type, found {:?}", idx_ty));
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

    fn is_unknown(ty: &Type) -> bool {
        matches!(ty, Type::Named(n) if n == "Unknown")
    }

    fn infer_binop(
        &mut self,
        op         : &BinOp,
        left       : &Type,
        right      : &Type,
        left_expr  : &Expr,
        _right_expr: &Expr,
    ) -> Type {
        // Lambda parametresi (Unknown) içeren ifadeler — sessizce geç
        if Self::is_unknown(left) || Self::is_unknown(right) {
            return match op {
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le |
                BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or => Type::Boolean,
                BinOp::Assign | BinOp::AddAssign | BinOp::SubAssign |
                BinOp::MulAssign | BinOp::DivAssign => left.clone(),
                _ => Type::Named("Unknown".to_string()),
            };
        }

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                if !self.is_numeric(left) || !self.is_numeric(right) {
                    // operator overload denemesi
                    let op_sym = match op {
                        BinOp::Add => "+", BinOp::Sub => "-",
                        BinOp::Mul => "*", BinOp::Div => "/", BinOp::Mod => "%",
                        _ => unreachable!(),
                    };
                    if let Some(ret) = self.try_operator_overload(left, op_sym, right) {
                        return ret;
                    }
                    self.error(format!(
                        "arithmetic operator requires numeric types, found {:?} and {:?}",
                        left, right
                    ));
                    return Type::Integer;
                }
                // Float beats everything
                if matches!(left, Type::Float) || matches!(right, Type::Float) {
                    return Type::Float;
                }
                // Same type → same type
                if self.types_equal(left, right) {
                    return left.clone();
                }
                // Integer literal (untyped) + sized type → sized type
                if matches!(left, Type::Integer) { return right.clone(); }
                if matches!(right, Type::Integer) { return left.clone(); }
                // Mixed sized types → error, require explicit cast
                self.error(format!(
                    "arithmetic on mixed integer types {:?} and {:?} — use 'as' to cast",
                    left, right
                ));
                left.clone()
            }

            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                if !self.is_integer_type(left) || !self.is_integer_type(right) {
                    self.error(format!(
                        "bitwise operator requires integer types, found {:?} and {:?}",
                        left, right
                    ));
                    return Type::Integer;
                }
                if self.types_equal(left, right) { return left.clone(); }
                if matches!(left, Type::Integer)  { return right.clone(); }
                if matches!(right, Type::Integer) { return left.clone(); }
                self.error(format!(
                    "bitwise op on mixed integer types {:?} and {:?} — use 'as' to cast",
                    left, right
                ));
                left.clone()
            }

            BinOp::Shl | BinOp::Shr => {
                if !self.is_integer_type(left) {
                    self.error(format!(
                        "shift operator requires integer type on left, found {:?}", left
                    ));
                }
                if !self.is_integer_type(right) {
                    self.error(format!(
                        "shift amount must be integer type, found {:?}", right
                    ));
                }
                left.clone()
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

    // ── Generic substitution & variant binding ────────────────────────────────

    fn substitute_generics(&self, ty: &Type, params: &[String], args: &[Type]) -> Type {
        match ty {
            Type::Named(n) => {
                if let Some(idx) = params.iter().position(|p| p == n) {
                    args.get(idx).cloned().unwrap_or_else(|| Type::Named("Unknown".to_string()))
                } else {
                    ty.clone()
                }
            }
            Type::Nullable(inner) =>
                Type::Nullable(Box::new(self.substitute_generics(inner, params, args))),
            Type::List(inner) =>
                Type::List(Box::new(self.substitute_generics(inner, params, args))),
            Type::Generic(n, a) =>
                Type::Generic(n.clone(), a.iter().map(|t| self.substitute_generics(t, params, args)).collect()),
            _ => ty.clone(),
        }
    }

    fn resolve_variant_binding_types(
        &self,
        enum_name    : &str,
        variant      : &str,
        scrutinee_ty : &Type,
    ) -> Vec<Type> {
        let info = match self.classes.get(enum_name) {
            Some(i) => i.clone(),
            None    => return Vec::new(),
        };
        let data_types = match info.variant_data.get(variant) {
            Some(d) => d.clone(),
            None    => return Vec::new(),
        };
        // Generic parametre substitution
        let type_args: Vec<Type> = match scrutinee_ty {
            Type::Generic(_, args) => args.clone(),
            _ => Vec::new(),
        };
        if type_args.is_empty() || info.generics.is_empty() {
            data_types
        } else {
            data_types.iter()
                .map(|ty| self.substitute_generics(ty, &info.generics, &type_args))
                .collect()
        }
    }

    // Enum variant constructor çağrısı: Shape.Circle(1.5) veya Result.Ok("hello")
    fn try_enum_variant_constructor(
        &mut self,
        class    : &str,
        variant  : &str,
        arg_types: &[Type],
    ) -> Option<Type> {
        let info = self.classes.get(class)?.clone();
        if info.kind != ClassKind::Enum { return None; }
        let data_types = info.variant_data.get(variant)?.clone();

        if arg_types.len() != data_types.len() {
            self.error(format!(
                "enum variant '{}::{}' expects {} argument(s), got {}",
                class, variant, data_types.len(), arg_types.len()
            ));
            return Some(Type::Named(class.to_string()));
        }

        // Generic enum: Result.Ok("hello") → Result<String, Unknown>
        if !info.generics.is_empty() {
            let mut type_args: Vec<Type> = info.generics.iter()
                .map(|_| Type::Named("Unknown".to_string()))
                .collect();
            for (i, (data_ty, arg_ty)) in data_types.iter().zip(arg_types.iter()).enumerate() {
                if let Type::Named(gp_name) = data_ty {
                    if let Some(idx) = info.generics.iter().position(|g| g == gp_name) {
                        type_args[idx] = arg_ty.clone();
                    } else {
                        self.check_assignable(data_ty, arg_ty, &format!(
                            "{}::{} arg {}", class, variant, i + 1
                        ));
                    }
                }
            }
            return Some(Type::Generic(class.to_string(), type_args));
        }

        // Non-generic enum: validate arg types
        for (i, (data_ty, arg_ty)) in data_types.iter().zip(arg_types.iter()).enumerate() {
            self.check_assignable(data_ty, arg_ty, &format!(
                "{}::{} arg {}", class, variant, i + 1
            ));
        }
        Some(Type::Named(class.to_string()))
    }

    // ── Operator overload resolution ──────────────────────────────────────────

    fn try_operator_overload(&mut self, left: &Type, op: &str, _right: &Type) -> Option<Type> {
        let class_name = match left {
            Type::Named(n) => n.clone(),
            _ => return None,
        };
        let method_name = format!("operator{}", op);
        let info = self.classes.get(&class_name)?.clone();
        let overloads = info.methods.get(&method_name)?.clone();
        for mi in &overloads {
            if !mi.static_ && mi.params.len() == 1 {
                return Some(mi.return_ty.clone().unwrap_or(Type::Void));
            }
        }
        None
    }

    // ── Cast validation ───────────────────────────────────────────────────────

    fn is_valid_cast(&self, from: &Type, to: &Type) -> bool {
        // integer → integer (including Integer ↔ sized types)
        if self.is_integer_type(from) && self.is_integer_type(to) { return true; }
        // integer → float
        if self.is_integer_type(from) && matches!(to, Type::Float) { return true; }
        // float → integer
        if matches!(from, Type::Float) && self.is_integer_type(to) { return true; }
        // float → float (no-op)
        if matches!(from, Type::Float) && matches!(to, Type::Float) { return true; }
        // RawPtr → RawPtr (pointer cast)
        if matches!(from, Type::RawPtr(_)) && matches!(to, Type::RawPtr(_)) { return true; }
        // integer → RawPtr (address-as-pointer in @manual code)
        if self.is_integer_type(from) && matches!(to, Type::RawPtr(_)) { return true; }
        // same type (no-op)
        if self.types_equal(from, to) { return true; }
        false
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
            Expr::Index { .. } => {} // Array/Slice index assignment — her zaman izinli
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
            Type::Named(n)       => n.clone(),
            Type::Generic(n, _)  => n.clone(),  // Result<T,E> → "Result"
            Type::List(_)        => "List".to_string(),
            Type::Map(_, _)      => "Map".to_string(),
            Type::HashMap(_, _)  => "HashMap".to_string(),
            Type::TreeMap(_, _)  => "TreeMap".to_string(),
            Type::Pair(_, _)     => "Pair".to_string(),
            Type::RawPtr(_)      => "RawPtr".to_string(),
            Type::Array(_, _)    => "Array".to_string(),
            Type::Slice(_)       => "Slice".to_string(),
            Type::U8  => "u8".to_string(),
            Type::U16 => "u16".to_string(),
            Type::U32 => "u32".to_string(),
            Type::U64 => "u64".to_string(),
            Type::I8  => "i8".to_string(),
            Type::I16 => "i16".to_string(),
            Type::I32 => "i32".to_string(),
            Type::I64 => "i64".to_string(),
            _ => {
                self.error(format!(
                    "cannot call method '{}' on type {:?}", method, ty
                ));
                return Type::Named("Error".to_string());
            }
        };

        if class_name == "Unknown" {
            return Type::Named("Unknown".to_string());
        }

        // Generic bound kontrolü: eğer class_name bir generic parametre ise
        // (örn: T, E) ve mevcut sınıfın generic_bounds'unda kayıtlı ise,
        // method'un o bound'ların birinde tanımlı olup olmadığını kontrol et
        if let Some(current_class) = self.current_class.clone() {
            if let Some(current_info) = self.classes.get(&current_class).cloned() {
                if current_info.generics.contains(&class_name) {
                    // T bir generic param — bound'lardan çözümle
                    if let Some(bounds) = current_info.generic_bounds.get(&class_name).cloned() {
                        for bound in &bounds {
                            let bound_ty = Type::Named(bound.clone());
                            let ret = self.resolve_method_call(&bound_ty, method, args, is_static);
                            if !matches!(ret, Type::Named(ref n) if n == "Error") {
                                return ret;
                            }
                        }
                    }
                    // Bound yok ya da çözümlenmedi — Unknown döndür
                    return Type::Named("Unknown".to_string());
                }
            }
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
                    if mi.static_ != is_static { continue; }
                    if mi.params.len() != args.len() { continue; }
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
            ("Map" | "HashMap" | "TreeMap", "keys") => {
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

            // Array<T, N> metodları
            ("Array", "length")  => Some(Type::Integer),
            ("Array", "zeroed")  => Some(Type::Array(Box::new(Type::Named("Unknown".to_string())), 0)),
            ("Array", "get")     => {
                match ty {
                    Type::Array(elem, _) => Some(*elem.clone()),
                    _ => Some(Type::Named("Unknown".to_string())),
                }
            }
            ("Array", "set")     => Some(Type::Void),
            ("Array", "asSlice") => {
                match ty {
                    Type::Array(elem, _) => Some(Type::Slice(elem.clone())),
                    _ => Some(Type::Slice(Box::new(Type::Named("Unknown".to_string())))),
                }
            }
            ("Array", "slice")   => {
                match ty {
                    Type::Array(elem, _) => Some(Type::Slice(elem.clone())),
                    _ => Some(Type::Slice(Box::new(Type::Named("Unknown".to_string())))),
                }
            }

            // Slice<T> metodları
            ("Slice", "length")  => Some(Type::Integer),
            ("Slice", "get")     => {
                match ty {
                    Type::Slice(elem) => Some(*elem.clone()),
                    _ => Some(Type::Named("Unknown".to_string())),
                }
            }
            ("Slice", "set")     => Some(Type::Void),
            ("Slice", "of")      => Some(Type::Slice(Box::new(Type::Named("Unknown".to_string())))),

            ("Memory", "alloc")  => Some(Type::RawPtr(Box::new(Type::Void))),
            ("Memory", "free")   => Some(Type::Void),
            ("Memory", "copy")   => Some(Type::Void),
            ("Memory", "set")    => Some(Type::Void),

            ("RawPtr", "read") => {
                match ty {
                    Type::RawPtr(inner) => Some(*inner.clone()),
                    _ => Some(Type::Named("Unknown".to_string())),
                }
            }
            ("RawPtr", "write")  => Some(Type::Void),
            ("RawPtr", "offset") => Some(ty.clone()),

            // sizeOf — tüm tipler için
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

    fn expand_alias(&self, ty: &Type) -> Type {
        if let Type::Named(n) = ty {
            if let Some(aliased) = self.type_aliases.get(n.as_str()) {
                return self.expand_alias(aliased);
            }
        }
        ty.clone()
    }

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
        // Expand type aliases before comparison
        let target_exp = self.expand_alias(target);
        let source_exp = self.expand_alias(source);
        let target = &target_exp;
        let source = &source_exp;

        if self.types_equal(target, source) {
            return true;
        }

        if let Type::Named(n) = source {
            if n == "Error" { return true; }
        }

        // Float accepts Integer (untyped literal)
        if matches!(target, Type::Float) && matches!(source, Type::Integer) {
            return true;
        }

        // Integer literal → any sized integer type
        if self.is_sized_integer(target) && matches!(source, Type::Integer) {
            return true;
        }

        // Sized integer → Integer (untyped bigint)
        if matches!(target, Type::Integer) && self.is_sized_integer(source) {
            return true;
        }

        // No implicit widening between sized types (u8 ← u16 is error)

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

        if let (Type::RawPtr(_), Type::RawPtr(s_inner)) = (target, source) {
            if matches!(s_inner.as_ref(), Type::Void | Type::Named(_)) { return true; }
        }

        // Array: Unknown wildcard (Array.zeroed() sonucu) herhangi bir Array'e atanabilir
        if let (Type::Array(te, _tn), Type::Array(se, sn)) = (target, source) {
            if *sn == 0 || matches!(se.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(te, se);
        }

        // Slice: Unknown wildcard herhangi bir Slice'a atanabilir
        if let (Type::Slice(te), Type::Slice(se)) = (target, source) {
            if matches!(se.as_ref(), Type::Named(n) if n == "Unknown") { return true; }
            return self.is_assignable(te, se);
        }

        // FnPtr: Lambda herhangi bir function pointer'a atanabilir (tip uyumu ileriki aşamada)
        if matches!(target, Type::FnPtr(_, _)) {
            if matches!(source, Type::Named(n) if n == "Lambda") { return true; }
        }

        // FnPtr ↔ FnPtr: tam eşleşme gerekir
        if let (Type::FnPtr(tp, tr), Type::FnPtr(sp, sr)) = (target, source) {
            if tp.len() != sp.len() { return false; }
            return self.is_assignable(tr, sr)
                && tp.iter().zip(sp.iter()).all(|(t, s)| self.is_assignable(t, s));
        }

        // Generic<T, E> ← Generic<T, Unknown> — Unknown wildcard
        if let (Type::Generic(tn, ta), Type::Generic(sn, sa)) = (target, source) {
            if tn != sn || ta.len() != sa.len() { return false; }
            return ta.iter().zip(sa.iter()).all(|(t, s)| {
                matches!(s, Type::Named(n) if n == "Unknown") || self.is_assignable(t, s)
            });
        }

        if let Type::Nullable(t_inner) = target {
            if let Type::Nullable(s_inner) = source {
                if let Type::Named(n) = s_inner.as_ref() {
                    if n == "Unknown" { return true; }
                }
                return self.is_assignable(t_inner, s_inner);
            }
            return self.is_assignable(t_inner, source);
        }

        if let (Type::Named(t), Type::Named(s)) = (target, source) {
            if self.is_subtype(s, t) { return true; }
        }

        if let (Type::Named(t), Type::Named(s)) = (target, source) {
            if t == "Exception" && self.is_exception_subtype(s) { return true; }
        }

        false
    }

    fn is_subtype(&self, child: &str, parent: &str) -> bool {
        if child == parent { return true; }
        if let Some(info) = self.classes.get(child) {
            if let Some(p) = &info.extends {
                if self.is_subtype(p, parent) { return true; }
            }
            for iface in &info.implements {
                if self.is_subtype(iface, parent) { return true; }
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
            (Type::U8,  Type::U8)  => true,
            (Type::U16, Type::U16) => true,
            (Type::U32, Type::U32) => true,
            (Type::U64, Type::U64) => true,
            (Type::I8,  Type::I8)  => true,
            (Type::I16, Type::I16) => true,
            (Type::I32, Type::I32) => true,
            (Type::I64, Type::I64) => true,
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
            (Type::Array(e1, n1), Type::Array(e2, n2)) =>
                n1 == n2 && self.types_equal(e1, e2),
            (Type::Slice(e1), Type::Slice(e2)) => self.types_equal(e1, e2),
            (Type::FnPtr(p1, r1), Type::FnPtr(p2, r2)) =>
                p1.len() == p2.len()
                && self.types_equal(r1, r2)
                && p1.iter().zip(p2.iter()).all(|(a, b)| self.types_equal(a, b)),
            (Type::Generic(n1, a1), Type::Generic(n2, a2)) =>
                n1 == n2
                && a1.len() == a2.len()
                && a1.iter().zip(a2.iter()).all(|(x, y)| self.types_equal(x, y)),
            _ => false,
        }
    }

    fn types_compatible(&self, a: &Type, b: &Type) -> bool {
        self.types_equal(a, b)
            || self.is_assignable(a, b)
            || self.is_assignable(b, a)
    }

    fn is_numeric(&self, ty: &Type) -> bool {
        matches!(ty,
            Type::Integer | Type::Float |
            Type::U8 | Type::U16 | Type::U32 | Type::U64 |
            Type::I8 | Type::I16 | Type::I32 | Type::I64
        )
    }

    fn is_integer_type(&self, ty: &Type) -> bool {
        matches!(ty,
            Type::Integer |
            Type::U8 | Type::U16 | Type::U32 | Type::U64 |
            Type::I8 | Type::I16 | Type::I32 | Type::I64
        )
    }

    // Sized integers only (not the generic Integer type)
    fn is_sized_integer(&self, ty: &Type) -> bool {
        matches!(ty,
            Type::U8 | Type::U16 | Type::U32 | Type::U64 |
            Type::I8 | Type::I16 | Type::I32 | Type::I64
        )
    }

    fn is_boolean(&self, ty: &Type) -> bool {
        matches!(ty, Type::Boolean)
    }

    fn is_throwable(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(n) => n == "Exception" || self.is_exception_subtype(n),
            _ => false,
        }
    }

    fn is_exception_subtype(&self, name: &str) -> bool {
        if name == "Exception" { return true; }
        if let Some(info) = self.classes.get(name) {
            if info.kind == ClassKind::Exception { return true; }
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
                    let then_ok     = self.all_paths_return(then);
                    let else_ifs_ok = else_if.iter().all(|(_, b)| self.all_paths_return(b));
                    let else_ok     = self.all_paths_return(else_body);
                    if then_ok && else_ifs_ok && else_ok { return true; }
                }

                Stmt::Switch { cases, .. } => {
                    if !cases.is_empty() && cases.iter().all(|c| self.all_paths_return(&c.body)) {
                        return true;
                    }
                }

                Stmt::Block(b) => {
                    if self.all_paths_return(b) { return true; }
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
                if info.methods.contains_key(method_name) { return true; }
                return self.method_in_parent(&info.extends, method_name);
            }
        }
        false
    }

    // ── Tip varlık kontrolü ───────────────────────────────────────────────────

    fn check_type_exists(&mut self, ty: &Type, context: &str) {
        match ty {
            Type::Named(name) => {
                // Generic parametreler kendi class içinde geçerli tip
                if let Some(cc) = &self.current_class.clone() {
                    if let Some(info) = self.classes.get(cc.as_str()) {
                        if info.generics.contains(name) { return; }
                    }
                }
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
            Type::Array(inner, _) => self.check_type_exists(inner, context),
            Type::Slice(inner)    => self.check_type_exists(inner, context),
            Type::FnPtr(params, ret) => {
                for p in params { self.check_type_exists(p, context); }
                self.check_type_exists(ret, context);
            }
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
            | "Exception" | "Object" | "Result"
            | "List" | "Map" | "HashMap" | "TreeMap" | "Pair"
            | "RawPtr" | "Void"
            | "Lambda" | "Unknown" | "Error"
        ) || self.classes.contains_key(name)
          || self.type_aliases.contains_key(name)
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
            kind          : ClassKind::Exception,
            generics      : Vec::new(),
            generic_bounds: HashMap::new(),
            extends       : None,
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : exception_methods,
            constructor   : Some(ConstructorInfo {
                params : vec![("message".to_string(), Type::Str)],
                vis    : Visibility::Public,
            }),
            variant_data  : HashMap::new(),
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
            kind          : ClassKind::Concrete,
            generics      : Vec::new(),
            generic_bounds: HashMap::new(),
            extends       : None,
            implements    : Vec::new(),
            fields        : HashMap::new(),
            methods       : object_methods,
            constructor   : None,
            variant_data  : HashMap::new(),
        });

        // ── Built-in Result<T, E> ─────────────────────────────────────────────
        // Result.Ok(value)  → Result<T, Unknown>
        // Result.Err(error) → Result<Unknown, E>
        let mut result_variant_data = HashMap::new();
        result_variant_data.insert("Ok".to_string(),  vec![Type::Named("T".to_string())]);
        result_variant_data.insert("Err".to_string(), vec![Type::Named("E".to_string())]);
        let mut result_fields = HashMap::new();
        result_fields.insert("Ok".to_string(),  FieldInfo { ty: Type::Named("Result".to_string()), readonly: true, static_: true, vis: Visibility::Public });
        result_fields.insert("Err".to_string(), FieldInfo { ty: Type::Named("Result".to_string()), readonly: true, static_: true, vis: Visibility::Public });
        let mut result_methods = HashMap::new();
        result_methods.insert("isOk".to_string(),  vec![MethodInfo { params: Vec::new(), return_ty: Some(Type::Boolean), static_: false, abstract_: false, vis: Visibility::Public }]);
        result_methods.insert("isErr".to_string(), vec![MethodInfo { params: Vec::new(), return_ty: Some(Type::Boolean), static_: false, abstract_: false, vis: Visibility::Public }]);
        self.classes.insert("Result".to_string(), ClassInfo {
            kind          : ClassKind::Enum,
            generics      : vec!["T".to_string(), "E".to_string()],
            generic_bounds: HashMap::new(),
            extends       : None,
            implements    : Vec::new(),
            fields        : result_fields,
            methods       : result_methods,
            constructor   : None,
            variant_data  : result_variant_data,
        });
    }
}
