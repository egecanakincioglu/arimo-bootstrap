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

    NullCoalesce {
        left  : Box<Expr>,
        right : Box<Expr>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern : MatchPattern,
    pub guard   : Option<Box<Expr>>,
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
    Binding(String),
    StrLit(String),
    IntLit(i64),
    Multi(Vec<MatchPattern>),
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
    Neg,
    Not,
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
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
    pub bounds : Vec<String>,
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
    pub name    : String,
    pub ty      : Type,
    pub default : Option<Expr>,
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
    pub pure_        : bool,
    pub deprecated   : Option<String>,
    pub experimental : bool,
    pub throws       : Vec<String>,
    pub suppress     : Vec<String>,
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
    pub sealed       : bool,
    pub immutable    : bool,
    pub deprecated   : Option<String>,
    pub experimental : bool,
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
    pub functional   : bool,
    pub sealed       : bool,
    pub deprecated   : Option<String>,
    pub experimental : bool,
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
    Extension(ExtensionDecl),
}

#[derive(Debug, Clone)]
pub struct ExtensionDecl {
    pub target  : String,
    pub methods : Vec<Method>,
}
