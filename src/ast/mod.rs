// ─────────────────────────────────────────────────────────────────────────────
// Arimo Lang — AST (Abstract Syntax Tree)
// ─────────────────────────────────────────────────────────────────────────────

// ── Konum bilgisi ─────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct Span {
    pub line : usize,
    pub col  : usize,
}

// ── Erişim belirteçleri ───────────────────────────────────────────────────────
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

    // String interpolation — "Merhaba ${name}!"
    // Vec<StringPart> — her parça ya metin ya expression
    StrInterp(Vec<StringPart>),

    // Değişken / isim
    Ident(String),

    // this / super
    This,
    Super,

    // Alan erişimi — this.name  player.score
    FieldAccess {
        object : Box<Expr>,
        field  : String,
    },

    // Null-safe erişim/çağrı — user?.name  user?.getName()
    NullSafeAccess {
        object : Box<Expr>,
        field  : String,
        args   : Option<Vec<Expr>>,  // None = field, Some = method call
    },

    // Metod çağrısı — task.complete()
    MethodCall {
        object : Box<Expr>,
        method : String,
        args   : Vec<Expr>,
    },

    // Statik metod çağrısı — Task.create(...)
    StaticCall {
        class  : String,
        method : String,
        args   : Vec<Expr>,
    },

    // Constructor çağrısı — Task(...)
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

    // Ternary — isUrgent ? "urgent" : "normal"
    Ternary {
        cond  : Box<Expr>,
        then  : Box<Expr>,
        else_ : Box<Expr>,
    },

    // Lambda — (task) -> task.isDone()
    Lambda {
        params : Vec<String>,
        body   : Box<Expr>,
    },

    // Array/List index — items[0]
    Index {
        object : Box<Expr>,
        index  : Box<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum StringPart {
    Text(String),       // düz metin parçası
    Interp(Box<Expr>),  // ${expression}
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
    Assign,
    AddAssign, SubAssign, MulAssign, DivAssign,
}

#[derive(Debug, Clone)]
pub enum UnaryOp {
    Neg,        // -x
    Not,        // !x
    PreInc,     // ++x
    PreDec,     // --x
    PostInc,    // x++
    PostDec,    // x--
}

// ── Statement ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Stmt {
    // Değişken tanımı — String name = "Arimo";
    VarDecl {
        ty    : Type,
        name  : String,
        value : Option<Expr>,
    },

    // Expression statement — task.complete();
    ExprStmt(Expr),

    // return
    Return(Option<Expr>),

    // throw
    Throw(Expr),

    // if / else if / else
    If {
        cond     : Expr,
        then     : Vec<Stmt>,
        else_if  : Vec<(Expr, Vec<Stmt>)>,
        else_    : Option<Vec<Stmt>>,
    },

    // while
    While {
        cond : Expr,
        body : Vec<Stmt>,
    },

    // for-each — for (Task task : this.tasks)
    ForEach {
        ty   : Type,
        name : String,
        iter : Expr,
        body : Vec<Stmt>,
    },

    // klasik for — for (Integer i = 0; i < 10; i++)
    For {
        init : Box<Stmt>,
        cond : Expr,
        step : Expr,
        body : Vec<Stmt>,
    },

    // switch
    Switch {
        expr  : Expr,
        cases : Vec<SwitchCase>,
    },

    // try / catch / finally
    TryCatch {
        try_body     : Vec<Stmt>,
        catches      : Vec<CatchClause>,
        finally_body : Option<Vec<Stmt>>,
    },

    // break / continue
    Break,
    Continue,

    // Blok — { ... }
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
    pub value      : Option<Expr>,   // static field'lar için başlangıç değeri
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
    pub name       : String,
    pub params     : Vec<Param>,
    pub return_ty  : Option<Type>,   // None = main() gibi dönüş tipi yok
    pub body       : Option<Vec<Stmt>>, // None = abstract metod
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
    pub name        : String,
    pub generics    : Vec<String>,       // class Pair<First, Second>
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
    pub methods  : Vec<Method>,          // sadece imza, body yok
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
    pub name        : String,
    pub extends     : String,            // her zaman Exception'dan extends
    pub fields      : Vec<Field>,
    pub constructor : Option<Constructor>,
    pub methods     : Vec<Method>,
}

// ── Program (kök node) ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Module {
    pub path    : String,          // "arimo.task.model"
    pub imports : Vec<String>,     // ["arimo.io", "arimo.task.exception"]
    pub items   : Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Class(ClassDecl),
    Interface(InterfaceDecl),
    Enum(EnumDecl),
    Exception(ExceptionDecl),
}