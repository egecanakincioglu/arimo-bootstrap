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

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Literals ─────────────────────────────────────────────────────────
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),        // string parçası — interpolation için bölünür
    Ident(String),

    // ── Module & Import ───────────────────────────────────────────────────
    Module,
    Import,

    // ── Access modifiers ─────────────────────────────────────────────────
    Public,
    Private,
    Protected,
    Internal,

    // ── Class keywords ───────────────────────────────────────────────────
    Class,
    Struct,      // struct — value type, stack-allocated
    Abstract,
    Interface,
    Enum,
    Extends,
    Implements,
    Operator,    // operator — metod seviyesinde operator overloading

    // ── Member modifiers ─────────────────────────────────────────────────
    Static,
    Readonly,
    Override,

    // ── Special members ──────────────────────────────────────────────────
    Constructor,
    Super,
    This,

    // ── Built-in types ───────────────────────────────────────────────────
    TypeInteger,
    TypeFloat,
    TypeBoolean,
    TypeString,
    TypeVoid,
    TypeList,
    TypeMap,
    TypeHashMap,
    TypeTreeMap,
    TypePair,
    TypeException,    // Exception — built-in base exception
    TypeRawPtr,       // RawPtr<T> — @manual sınıflarda ham bellek erişimi
    TypeArray,        // Array<T, N> — fixed-size stack array
    TypeSlice,        // Slice<T> — fat pointer (ptr + len), non-owning view

    // ── Fixed-size integer types ──────────────────────────────────────────
    TypeU8,
    TypeU16,
    TypeU32,
    TypeU64,
    TypeI8,
    TypeI16,
    TypeI32,
    TypeI64,

    // ── Standard library ─────────────────────────────────────────────────
    StdIO,            // IO.print() IO.read()
    StdMath,          // Math.sqrt() Math.PI
    StdTime,          // Time.now() Time.generateId()
    StdMemory,        // Memory.alloc() Memory.free() Memory.copy()

    // ── Control flow ─────────────────────────────────────────────────────
    If,
    Else,
    While,
    For,
    Return,
    Break,
    Continue,
    Switch,
    Case,
    Match,       // match — pattern matching
    Throw,
    Try,
    Catch,
    Finally,

    // ── Null safety ──────────────────────────────────────────────────────
    Null,

    // ── Additional keywords ───────────────────────────────────────────────
    As,        // as  — type cast
    KwType,    // type — type alias declaration

    // ── Systems & Performance keywords ───────────────────────────────────────
    Volatile,       // volatile
    Union,          // union
    Extern,         // extern
    Asm,            // asm
    Defer,          // defer

    // ── Async / Default keywords ─────────────────────────────────────────────
    Async,          // async — asenkron metod modifier
    Await,          // await — asenkron bekleme operatörü
    Default,        // default — interface default method

    // ── Additional type keywords ─────────────────────────────────────────────
    TypeNoReturn,   // noreturn

    // ── Variadic ─────────────────────────────────────────────────────────────
    Ellipsis,       // ...

    // ── Operators ────────────────────────────────────────────────────────
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,          // =
    EqEq,        // ==
    Bang,        // !
    BangEq,      // !=
    Lt,          // <
    LtEq,        // <=
    Gt,          // >
    GtEq,        // >=
    AndAnd,      // &&
    PipePipe,    // ||
    PlusPlus,    // ++
    MinusMinus,  // --
    PlusEq,      // +=
    MinusEq,     // -=
    StarEq,      // *=
    SlashEq,     // /=
    Arrow,       // ->  lambda
    FatArrow,    // =>  match arm
    Question,    // ?   nullable + ternary (parser bağlama göre ayırt eder)
    QuestionDot, // ?.  null-safe erişim

    // ── Bitwise operators ─────────────────────────────────────────────────
    Amp,         // &   bitwise AND
    Pipe,        // |   bitwise OR
    Caret,       // ^   bitwise XOR
    Tilde,       // ~   bitwise NOT (prefix unary)
    LtLt,        // <<  left shift
    GtGt,        // >>  right shift

    // ── Delimiters ───────────────────────────────────────────────────────
    LParen,       // (
    RParen,       // )
    LBrace,       // {
    RBrace,       // }
    LBracket,     // [
    RBracket,     // ]
    Comma,        // ,
    Semicolon,    // ;
    Colon,        // :
    Dot,          // .
    DollarLBrace, // ${  string interpolation başlangıcı
    InterpolEnd,  // }   string interpolation bitişi (string içinde)

    // ── Annotations ──────────────────────────────────────────────────────
    At,             // @  annotation işareti

    // ── Special ──────────────────────────────────────────────────────────
    Eof,
    Unknown(char),
}

#[derive(Debug, Clone)]
pub struct Span {
    pub line : usize,
    pub col  : usize,
}

#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token : Token,
    pub span  : Span,
}

pub struct Lexer<'a> {
    source        : &'a str,
    chars         : std::iter::Peekable<std::str::CharIndices<'a>>,
    line          : usize,
    col           : usize,
    in_interp     : bool,
    interp_depth  : usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source,
            chars        : source.char_indices().peekable(),
            line         : 1,
            col          : 1,
            in_interp    : false,
            interp_depth : 0,
        }
    }

    fn span(&self) -> Span {
        Span { line: self.line, col: self.col }
    }

    fn advance(&mut self) -> Option<(usize, char)> {
        let next = self.chars.next();
        if let Some((_, c)) = next {
            if c == '\n' { self.line += 1; self.col = 1; }
            else         { self.col  += 1; }
        }
        next
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn peek_next(&self) -> Option<char> {
        let mut iter = self.chars.clone();
        iter.next();
        iter.next().map(|(_, c)| c)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => { self.advance(); }
                Some('/') => {
                    if self.peek_next() == Some('/') {
                        while self.peek().map(|c| c != '\n').unwrap_or(false) {
                            self.advance();
                        }
                    } else if self.peek_next() == Some('*') {
                        self.advance(); self.advance();
                        loop {
                            match self.advance() {
                                Some((_, '*')) if self.peek() == Some('/') => {
                                    self.advance();
                                    break;
                                }
                                None => break,
                                _    => {}
                            }
                        }
                    } else { break; }
                }
                _ => break,
            }
        }
    }

    fn read_number(&mut self, first: char) -> Token {
        // Hex literal: 0x... veya 0X...
        if first == '0' {
            match self.peek() {
                Some('x') | Some('X') => {
                    self.advance();
                    let mut hex = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            if c != '_' { hex.push(c); }
                            self.advance();
                        } else { break; }
                    }
                    return Token::Int(i64::from_str_radix(&hex, 16).unwrap_or(0));
                }
                Some('b') | Some('B') => {
                    self.advance();
                    let mut bin = String::new();
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            if c != '_' { bin.push(c); }
                            self.advance();
                        } else { break; }
                    }
                    return Token::Int(i64::from_str_radix(&bin, 2).unwrap_or(0));
                }
                _ => {}
            }
        }

        let mut num      = String::from(first);
        let mut is_float = false;
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_digit() || c == '_' => {
                    if c != '_' { num.push(c); }
                    self.advance();
                }
                Some('.') if !is_float => {
                    if self.peek_next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        is_float = true;
                        num.push('.');
                        self.advance();
                    } else { break; }
                }
                _ => break,
            }
        }
        if is_float { Token::Float(num.parse().unwrap_or(0.0)) }
        else        { Token::Int(num.parse().unwrap_or(0))     }
    }

    fn read_ident(&mut self, first: char) -> Token {
        let mut word = String::from(first);
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { word.push(c); self.advance(); }
            else { break; }
        }
        match word.as_str() {
            "module"      => Token::Module,
            "import"      => Token::Import,
            "public"      => Token::Public,
            "private"     => Token::Private,
            "protected"   => Token::Protected,
            "internal"    => Token::Internal,
            "class"       => Token::Class,
            "struct"      => Token::Struct,
            "abstract"    => Token::Abstract,
            "operator"    => Token::Operator,
            "interface"   => Token::Interface,
            "enum"        => Token::Enum,
            "extends"     => Token::Extends,
            "implements"  => Token::Implements,
            "static"      => Token::Static,
            "readonly"    => Token::Readonly,
            "override"    => Token::Override,
            "constructor" => Token::Constructor,
            "super"       => Token::Super,
            "this"        => Token::This,

            // Built-in types
            "Integer"     => Token::TypeInteger,
            "Float"       => Token::TypeFloat,
            "Boolean"     => Token::TypeBoolean,
            "String"      => Token::TypeString,
            "Void"        => Token::TypeVoid,
            "List"        => Token::TypeList,
            "Map"         => Token::TypeMap,
            "HashMap"     => Token::TypeHashMap,
            "TreeMap"     => Token::TypeTreeMap,
            "Pair"        => Token::TypePair,
            "Exception"   => Token::TypeException,
            "RawPtr"      => Token::TypeRawPtr,
            "Array"       => Token::TypeArray,
            "Slice"       => Token::TypeSlice,

            // Fixed-size integer types
            "u8"          => Token::TypeU8,
            "u16"         => Token::TypeU16,
            "u32"         => Token::TypeU32,
            "u64"         => Token::TypeU64,
            "i8"          => Token::TypeI8,
            "i16"         => Token::TypeI16,
            "i32"         => Token::TypeI32,
            "i64"         => Token::TypeI64,

            // Standard library
            "IO"          => Token::StdIO,
            "Math"        => Token::StdMath,
            "Time"        => Token::StdTime,
            "Memory"      => Token::StdMemory,

            // Control flow
            "if"          => Token::If,
            "else"        => Token::Else,
            "while"       => Token::While,
            "for"         => Token::For,
            "return"      => Token::Return,
            "break"       => Token::Break,
            "continue"    => Token::Continue,
            "switch"      => Token::Switch,
            "case"        => Token::Case,
            "match"       => Token::Match,
            "throw"       => Token::Throw,
            "try"         => Token::Try,
            "catch"       => Token::Catch,
            "finally"     => Token::Finally,

            // Null
            "null"        => Token::Null,

            // Booleans
            "true"        => Token::Bool(true),
            "false"       => Token::Bool(false),

            // Cast & type alias
            "as"          => Token::As,
            "type"        => Token::KwType,

            // Systems & Performance
            "volatile"  => Token::Volatile,
            "union"     => Token::Union,
            "extern"    => Token::Extern,
            "asm"       => Token::Asm,
            "defer"     => Token::Defer,
            "noreturn"  => Token::TypeNoReturn,
            "async"     => Token::Async,
            "await"     => Token::Await,
            "default"   => Token::Default,

            _             => Token::Ident(word),
        }
    }

    fn read_string(&mut self, tokens: &mut Vec<SpannedToken>) {
        let mut buf = String::new();

        loop {
            match self.advance() {
                Some((_, '"')) => {
                    if !buf.is_empty() {
                        let span = self.span();
                        tokens.push(SpannedToken { token: Token::Str(buf.clone()), span });
                        buf.clear();
                    }
                    break;
                }
                Some((_, '$')) if self.peek() == Some('{') => {
                    self.advance();
                    if !buf.is_empty() {
                        let span = self.span();
                        tokens.push(SpannedToken { token: Token::Str(buf.clone()), span });
                        buf.clear();
                    }
                    let span = self.span();
                    tokens.push(SpannedToken { token: Token::DollarLBrace, span });
                    self.tokenize_interpolation(tokens);
                    let span = self.span();
                    tokens.push(SpannedToken { token: Token::InterpolEnd, span });
                }
                Some((_, '\\')) => match self.advance() {
                    Some((_, 'n'))  => buf.push('\n'),
                    Some((_, 't'))  => buf.push('\t'),
                    Some((_, '"'))  => buf.push('"'),
                    Some((_, '\\')) => buf.push('\\'),
                    Some((_, '$'))  => buf.push('$'),
                    _               => {}
                },
                Some((_, c)) => buf.push(c),
                None         => break,
            }
        }
    }

    fn tokenize_interpolation(&mut self, tokens: &mut Vec<SpannedToken>) {
        let mut depth = 1usize;
        loop {
            self.skip_whitespace_and_comments();
            let span  = self.span();
            match self.peek() {
                None => break,
                Some('{') => {
                    depth += 1;
                    self.advance();
                    tokens.push(SpannedToken { token: Token::LBrace, span });
                }
                Some('}') => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        break;
                    } else {
                        self.advance();
                        tokens.push(SpannedToken { token: Token::RBrace, span });
                    }
                }
                _ => {
                    let token = match self.advance() {
                        None            => break,
                        Some((_, c))    => match c {
                            '0'..='9' => self.read_number(c),
                            'a'..='z' | 'A'..='Z' | '_' => self.read_ident(c),
                            '.' => Token::Dot,
                            '(' => Token::LParen,
                            ')' => Token::RParen,
                            '[' => Token::LBracket,
                            ']' => Token::RBracket,
                            ',' => Token::Comma,
                            '+' => Token::Plus,
                            '-' => Token::Minus,
                            '*' => Token::Star,
                            '/' => Token::Slash,
                            '%' => Token::Percent,
                            '&' => if self.peek() == Some('&') { self.advance(); Token::AndAnd } else { Token::Amp },
                            '|' => if self.peek() == Some('|') { self.advance(); Token::PipePipe } else { Token::Pipe },
                            '^' => Token::Caret,
                            '~' => Token::Tilde,
                            other => Token::Unknown(other),
                        }
                    };
                    tokens.push(SpannedToken { token, span });
                }
            }
        }
    }

    pub fn tokenize(&mut self) -> Vec<SpannedToken> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            let span  = self.span();

            match self.peek() {
                None => {
                    tokens.push(SpannedToken { token: Token::Eof, span });
                    break;
                }
                Some('"') => {
                    self.advance();
                    self.read_string(&mut tokens);
                    continue;
                }
                _ => {}
            }

            let token = match self.advance() {
                None            => Token::Eof,
                Some((_, c))    => match c {
                    '0'..='9'                    => self.read_number(c),
                    'a'..='z' | 'A'..='Z' | '_' => self.read_ident(c),

                    '+' => match self.peek() {
                        Some('+') => { self.advance(); Token::PlusPlus  }
                        Some('=') => { self.advance(); Token::PlusEq    }
                        _         => Token::Plus
                    },
                    '-' => match self.peek() {
                        Some('-') => { self.advance(); Token::MinusMinus }
                        Some('>') => { self.advance(); Token::Arrow      }
                        Some('=') => { self.advance(); Token::MinusEq    }
                        _         => Token::Minus
                    },
                    '*' => if self.peek() == Some('=') { self.advance(); Token::StarEq  } else { Token::Star  },
                    '/' => if self.peek() == Some('=') { self.advance(); Token::SlashEq } else { Token::Slash },
                    '%' => Token::Percent,

                    '=' => match self.peek() {
                        Some('=') => { self.advance(); Token::EqEq    }
                        Some('>') => { self.advance(); Token::FatArrow }
                        _         => Token::Eq
                    },
                    '!' => if self.peek() == Some('=') { self.advance(); Token::BangEq } else { Token::Bang },

                    '<' => match self.peek() {
                        Some('<') => { self.advance(); Token::LtLt }
                        Some('=') => { self.advance(); Token::LtEq }
                        _         => Token::Lt
                    },
                    '>' => match self.peek() {
                        Some('>') => { self.advance(); Token::GtGt }
                        Some('=') => { self.advance(); Token::GtEq }
                        _         => Token::Gt
                    },

                    '&' => if self.peek() == Some('&') { self.advance(); Token::AndAnd   } else { Token::Amp      },
                    '|' => if self.peek() == Some('|') { self.advance(); Token::PipePipe } else { Token::Pipe     },
                    '^' => Token::Caret,
                    '~' => Token::Tilde,

                    '?' => if self.peek() == Some('.') { self.advance(); Token::QuestionDot } else { Token::Question },

                    ':' => Token::Colon,
                    '.' => {
                        if self.peek() == Some('.') && self.peek_next() == Some('.') {
                            self.advance();
                            self.advance();
                            Token::Ellipsis
                        } else {
                            Token::Dot
                        }
                    },

                    '(' => Token::LParen,
                    ')' => Token::RParen,
                    '{' => Token::LBrace,
                    '}' => Token::RBrace,
                    '[' => Token::LBracket,
                    ']' => Token::RBracket,
                    ',' => Token::Comma,
                    ';' => Token::Semicolon,
                    '@' => Token::At,

                    other => Token::Unknown(other),
                }
            };

            let is_eof = token == Token::Eof;
            tokens.push(SpannedToken { token, span });
            if is_eof { break; }
        }
        tokens
    }
}
