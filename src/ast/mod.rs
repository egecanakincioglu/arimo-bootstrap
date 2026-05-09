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
    NoReturn,

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

    // Fixed-size array — Array<Float, 16>
    // N=0 "unknown size" wildcard (Array.zeroed() gibi literal'lar için)
    Array(Box<Type>, usize),

    // Slice — fat pointer (ptr + len), non-owning view — Slice<u8>
    Slice(Box<Type>),

    // Function pointer — (Integer, String) -> Boolean
    FnPtr(Vec<Type>, Box<Type>),
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

    // await expr — async bekleme
    Await(Box<Expr>),

    // Pattern matching — match expr { Enum.Variant(a, b) => expr, _ => expr }
    Match {
        expr : Box<Expr>,
        arms : Vec<MatchArm>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern : MatchPattern,
    pub body    : Box<Expr>,
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    // Enum.Variant veya Enum.Variant(a, b, ...)
    Variant {
        enum_name : String,
        variant   : String,
        bindings  : Vec<String>,
    },
    // _ wildcard
    Wildcard,
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
        volatile : bool,
        ty       : Type,
        name     : String,
        value    : Option<Expr>,
    },

    ExprStmt(Expr),

    Return(Option<Expr>),

    Throw(Expr),

    If {
        hint     : Option<BranchHint>,
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

    Asm(String),
    Defer(Box<Expr>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum BranchHint {
    Likely,
    Unlikely,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallingConv {
    Cdecl,
    Stdcall,
    Interrupt,
}

// ── Generic parametre — bound opsiyonel ──────────────────────────────────────
// <T>         → GenericParam { name: "T", bounds: [] }
// <T: Drawable> → GenericParam { name: "T", bounds: ["Drawable"] }

#[derive(Debug, Clone)]
pub struct GenericParam {
    pub name   : String,
    pub bounds : Vec<String>,  // interface isimleri
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
    pub visibility   : Visibility,
    pub static_      : bool,
    pub abstract_    : bool,
    pub default_     : bool,
    pub override_    : bool,
    pub inline_      : bool,
    pub async_       : bool,
    pub pure_        : bool,            // @Pure — yan etki yok
    pub deprecated   : Option<String>, // @Deprecated("mesaj")
    pub experimental : bool,           // @Experimental
    pub throws       : Vec<String>,    // @Throws(ExType1, ...)
    pub suppress     : Vec<String>,    // @SuppressWarnings("tip")
    pub calling_conv : Option<CallingConv>,
    pub section      : Option<String>,
    pub name         : String,
    pub params       : Vec<Param>,
    pub return_ty    : Option<Type>,
    pub body         : Option<Vec<Stmt>>,
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
    pub visibility   : Visibility,
    pub abstract_    : bool,
    pub manual       : bool,
    pub sealed       : bool,           // @Sealed — sadece aynı modülden extend
    pub immutable    : bool,           // @Immutable — tüm field'lar readonly zorunlu
    pub deprecated   : Option<String>, // @Deprecated("mesaj")
    pub experimental : bool,           // @Experimental
    pub name         : String,
    pub generics     : Vec<GenericParam>,
    pub extends      : Option<String>,
    pub implements   : Vec<String>,
    pub fields       : Vec<Field>,
    pub constructor  : Option<Constructor>,
    pub methods      : Vec<Method>,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub name         : String,
    pub generics     : Vec<GenericParam>,
    pub functional   : bool,           // @FunctionalInterface — tam 1 abstract method
    pub sealed       : bool,           // @Sealed
    pub deprecated   : Option<String>, // @Deprecated("mesaj")
    pub experimental : bool,           // @Experimental
    pub methods      : Vec<Method>,
}

// Enum variant — veri taşıyabilir veya saf olabilir
// Circle(Float) → EnumVariant { name: "Circle", data: [Float] }
// Low           → EnumVariant { name: "Low",    data: []      }
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name : String,
    pub data : Vec<Type>,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub visibility   : Visibility,
    pub deprecated   : Option<String>,
    pub experimental : bool,
    pub name         : String,
    pub variants     : Vec<EnumVariant>,
    pub methods      : Vec<Method>,
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

#[derive(Debug, Clone)]
pub struct UnionDecl {
    pub visibility : Visibility,
    pub name       : String,
    pub fields     : Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct ExternParam {
    pub name : String,
    pub ty   : Type,
}

#[derive(Debug, Clone)]
pub struct ExternDecl {
    pub name      : String,
    pub params    : Vec<ExternParam>,
    pub return_ty : Option<Type>,
    pub variadic  : bool,
}

#[derive(Debug, Clone)]
pub struct ExternBlock {
    pub abi   : String,
    pub decls : Vec<ExternDecl>,
}

// ── Struct (value type, stack-allocated) ──────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub visibility   : Visibility,
    pub packed       : bool,
    pub align        : Option<usize>,
    pub deprecated   : Option<String>,
    pub experimental : bool,
    pub name         : String,
    pub generics     : Vec<GenericParam>,
    pub implements   : Vec<String>,
    pub fields       : Vec<Field>,
    pub constructor  : Option<Constructor>,
    pub methods      : Vec<Method>,
}

// ── Program (kök node) ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Module {
    pub path    : String,
    pub nostd   : bool,
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
    Union(UnionDecl),
    Extern(ExternBlock),
}
