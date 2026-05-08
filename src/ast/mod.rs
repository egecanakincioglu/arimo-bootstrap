// ─────────────────────────────────────────────────────────────────────────────
// Arimo Lang — AST (Abstract Syntax Tree)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Span {
    pub line : usize,
    pub col  : usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
}

// ── Tip sistemi ───────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Type {
    // Primitifler
    Integer,
    Float,
    Boolean,
    Str,
    Void,

    // Fixed-size integer types
    U8, U16, U32, U64,
    I8, I16, I32, I64,

    // Koleksiyonlar
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    HashMap(Box<Type>, Box<Type>),
    TreeMap(Box<Type>, Box<Type>),
    Pair(Box<Type>, Box<Type>),

    // Kullanıcı tanımlı
    Named(String),

    // Nullable — String?
    Nullable(Box<Type>),

    // Generics — Pair<First, Second>
    Generic(String, Vec<Type>),

    // Ham pointer — sadece @manual sınıflarda
    RawPtr(Box<Type>),
}

// ── Expression ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Expr {
    // Literaller
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StrLit(String),
    NullLit,

    // String interpolation
    StrInterp(Vec<StringPart>),

    // Değişken / isim
    Ident(String),

    // this / super
    This,
    Super,

    // Alan erişimi
    FieldAccess {
        object : Box<Expr>,
        field  : String,
    },

    // Null-safe erişim/çağrı
    NullSafeAccess {
        object : Box<Expr>,
        field  : String,
        args   : Option<Vec<Expr>>,
    },

    // Metod çağrısı
    MethodCall {
        object : Box<Expr>,
        method : String,
        args   : Vec<Expr>,
    },

    // Statik metod çağrısı
    StaticCall {
        class  : String,
        method : String,
        args   : Vec<Expr>,
    },

    // Constructor çağrısı
    ConstructorCall {
        class : String,
        args  : Vec<Expr>,
    },

    // Binary operatörler
    BinOp {
        op    : BinOp,
        left  : Box<Expr>,
        right : Box<Expr>,
    },

    // Unary operatörler
    UnaryOp {
        op   : UnaryOp,
        expr : Box<Expr>,
    },

    // Type cast — expr as Type
    Cast {
        expr : Box<Expr>,
        ty   : Type,
    },

    // Ternary
    Ternary {
        cond  : Box<Expr>,
        then  : Box<Expr>,
        else_ : Box<Expr>,
    },

    // Lambda
    Lambda {
        params : Vec<String>,
        body   : Box<Expr>,
    },

    // Array/List index
    Index {
        object : Box<Expr>,
        index  : Box<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum StringPart {
    Text(String),
    Interp(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
    // Bitwise
    BitAnd, BitOr, BitXor, Shl, Shr,
    // Assignments
    Assign,
    AddAssign, SubAssign, MulAssign, DivAssign,
}

#[derive(Debug, Clone)]
pub enum UnaryOp {
    Neg,        // -x
    Not,        // !x
    BitNot,     // ~x
    PreInc,     // ++x
    PreDec,     // --x
    PostInc,    // x++
    PostDec,    // x--
}

// ── Statement ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl {
        ty    : Type,
        name  : String,
        value : Option<Expr>,
    },

    ExprStmt(Expr),

    Return(Option<Expr>),

    Throw(Expr),

    If {
        cond     : Expr,
        then     : Vec<Stmt>,
        else_if  : Vec<(Expr, Vec<Stmt>)>,
        else_    : Option<Vec<Stmt>>,
    },

    While {
        cond : Expr,
        body : Vec<Stmt>,
    },

    ForEach {
        ty   : Type,
        name : String,
        iter : Expr,
        body : Vec<Stmt>,
    },

    For {
        init : Box<Stmt>,
        cond : Expr,
        step : Expr,
        body : Vec<Stmt>,
    },

    Switch {
        expr  : Expr,
        cases : Vec<SwitchCase>,
    },

    TryCatch {
        try_body     : Vec<Stmt>,
        catches      : Vec<CatchClause>,
        finally_body : Option<Vec<Stmt>>,
    },

    Break,
    Continue,

    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub pattern : Expr,
    pub body    : Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct CatchClause {
    pub exception_type : Type,
    pub name           : String,
    pub body           : Vec<Stmt>,
}

// ── Üye tanımları ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Field {
    pub visibility : Visibility,
    pub readonly   : bool,
    pub static_    : bool,
    pub name       : String,
    pub ty         : Type,
    pub value      : Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name : String,
    pub ty   : Type,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub visibility : Visibility,
    pub static_    : bool,
    pub abstract_  : bool,
    pub override_  : bool,
    pub inline_    : bool,   // @inline annotation
    pub name       : String,
    pub params     : Vec<Param>,
    pub return_ty  : Option<Type>,
    pub body       : Option<Vec<Stmt>>,
}

#[derive(Debug, Clone)]
pub struct Constructor {
    pub visibility : Visibility,
    pub params     : Vec<Param>,
    pub body       : Vec<Stmt>,
}

// ── Üst düzey tanımlar ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub visibility  : Visibility,
    pub abstract_   : bool,
    pub manual      : bool,
    pub name        : String,
    pub generics    : Vec<String>,
    pub extends     : Option<String>,
    pub implements  : Vec<String>,
    pub fields      : Vec<Field>,
    pub constructor : Option<Constructor>,
    pub methods     : Vec<Method>,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub name     : String,
    pub generics : Vec<String>,
    pub methods  : Vec<Method>,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub visibility : Visibility,
    pub name       : String,
    pub variants   : Vec<String>,
    pub methods    : Vec<Method>,
}

#[derive(Debug, Clone)]
pub struct ExceptionDecl {
    pub visibility  : Visibility,
    pub manual      : bool,
    pub name        : String,
    pub extends     : String,
    pub fields      : Vec<Field>,
    pub constructor : Option<Constructor>,
    pub methods     : Vec<Method>,
}

#[derive(Debug, Clone)]
pub struct TypeAliasDecl {
    pub name : String,
    pub ty   : Type,
}

// ── Struct (value type, stack-allocated) ──────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub visibility  : Visibility,
    pub name        : String,
    pub generics    : Vec<String>,
    pub implements  : Vec<String>,
    pub fields      : Vec<Field>,
    pub constructor : Option<Constructor>,  // None → auto-generate from fields
    pub methods     : Vec<Method>,
}

// ── Program (kök node) ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Module {
    pub path    : String,
    pub imports : Vec<String>,
    pub items   : Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Class(ClassDecl),
    Struct(StructDecl),
    Interface(InterfaceDecl),
    Enum(EnumDecl),
    Exception(ExceptionDecl),
    TypeAlias(TypeAliasDecl),
}
