
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

#[derive(Debug, Clone)]
pub enum Type {
    Integer,
    Float,
    Boolean,
    Str,
    Void,
    NoReturn,

    U8, U16, U32, U64,
    I8, I16, I32, I64,

    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    HashMap(Box<Type>, Box<Type>),
    TreeMap(Box<Type>, Box<Type>),
    Pair(Box<Type>, Box<Type>),

    Named(String),

    Nullable(Box<Type>),

    Generic(String, Vec<Type>),

    RawPtr(Box<Type>),

    Array(Box<Type>, usize),

    Slice(Box<Type>),

    FnPtr(Vec<Type>, Box<Type>),
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StrLit(String),
    NullLit,

    StrInterp(Vec<StringPart>),

    Ident(String),

    This,
    Super,

    FieldAccess {
        object : Box<Expr>,
        field  : String,
    },

    NullSafeAccess {
        object : Box<Expr>,
        field  : String,
        args   : Option<Vec<Expr>>,
    },

    MethodCall {
        object : Box<Expr>,
        method : String,
        args   : Vec<Expr>,
    },

    StaticCall {
        class  : String,
        method : String,
        args   : Vec<Expr>,
    },

    ConstructorCall {
        class : String,
        args  : Vec<Expr>,
    },

    BinOp {
        op    : BinOp,
        left  : Box<Expr>,
        right : Box<Expr>,
    },

    UnaryOp {
        op   : UnaryOp,
        expr : Box<Expr>,
    },

    Cast {
        expr : Box<Expr>,
        ty   : Type,
    },

    Ternary {
        cond  : Box<Expr>,
        then  : Box<Expr>,
        else_ : Box<Expr>,
    },

    Lambda {
        params : Vec<String>,
        body   : Box<Expr>,
    },

    Index {
        object : Box<Expr>,
        index  : Box<Expr>,
    },

    Await(Box<Expr>),

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
    Variant {
        enum_name : String,
        variant   : String,
        bindings  : Vec<String>,
    },
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
    BitAnd, BitOr, BitXor, Shl, Shr,
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

#[derive(Debug, Clone)]
pub struct GenericParam {
    pub name   : String,
    pub bounds : Vec<String>,  // interface isimleri
}

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
