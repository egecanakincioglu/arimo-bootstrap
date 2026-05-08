# Arimo Lang — Yol Haritası & Yapılacaklar Listesi

> Bu belge bir sonraki Claude Code oturumu için tam bağlam içerir.
> Proje detayları: `arimo-lang-task-list.md`

---

## Hedef

Modern bir programlama dili:
- **OS yazılabilir** — bare-metal, inline asm, donanım erişimi
- **Game engine yazılabilir** — sıfır GC overhead, struct, SIMD hint, operator overloading
- **Uygulama yazılabilir** — otomatik bellek, async/await, yüksek seviye OOP

---

## Mevcut Durum (Tamamlananlar)

- [x] Lexer — tüm tokenlar, string interpolation, `@`, `RawPtr`, `Memory`
- [x] Parser — Pratt parser, tüm v1.3 syntax, `@manual`, `RawPtr<T>`, `super(...)`
- [x] AST — tüm node tipleri, `manual` flag, `RawPtr` tipi
- [x] TypeChecker — tam tip sistemi, null safety, generics, builtin metodlar
- [x] BorrowChecker — use-after-move, mutation-while-borrowed, drop schedule
- [x] `@manual` annotation — RawPtr, Memory.alloc/free, sizeOf
- [x] Pipeline — parse → type check → borrow check
- [x] Test — comprehensive.arm (7 item, tüm v1.3 özellikleri)

---

## FAZA 1 — Dil Genişletmesi (CodeGen'den önce yapılmalı)

Bu özellikler spec'e ve compiler'a eklenmeli. Önce lexer/parser/AST, sonra typechecker.

### 1.1 Fixed-size Integer Tipler
**Öncelik: KRİTİK** — OS ve game engine için bloke edici

**Lexer'a eklenecek tokenlar:**
```
Token::TypeU8   Token::TypeU16  Token::TypeU32  Token::TypeU64
Token::TypeI8   Token::TypeI16  Token::TypeI32  Token::TypeI64
```

**AST'ye eklenecek tipler:**
```rust
Type::U8 | Type::U16 | Type::U32 | Type::U64
Type::I8 | Type::I16 | Type::I32 | Type::I64
```

**Sözdizimi:**
```arimo
u8  u16  u32  u64   // unsigned
i8  i16  i32  i64   // signed

u8  byte  = 255;
u32 flags = 0xDEADBEEF;
i64 count = -1;
```

**TypeChecker kuralları:**
- u8/u16/u32/u64 → copy tipler (Integer gibi)
- i8/i16/i32/i64 → copy tipler
- Integer → i64 eşdeğeri (mevcut Integer kalır, uyumluluk için)
- u8 ← u16 ataması → type error (implicit widening yok)
- Cast syntax: `value as u32`

**Dikkat:** Mevcut `Integer` tipi kalmaya devam eder (geriye dönük uyumluluk).

---

### 1.2 Bitwise Operatörler
**Öncelik: KRİTİK** — OS ve game engine için bloke edici

**Lexer'a eklenecek tokenlar:**
```
Token::Amp        // &   bitwise AND (&&'den farklı)
Token::Pipe       // |   bitwise OR  (||'den farklı)
Token::Caret      // ^   bitwise XOR
Token::Tilde      // ~   bitwise NOT (unary)
Token::LtLt       // <<  left shift
Token::GtGt       // >>  right shift
```

**Dikkat:** `&` zaten `&&`'in parçası, lexer'da dikkatli ayırt edilmeli:
- `&` tek başına → `Token::Amp`
- `&&` → `Token::AndAnd` (mevcut)
- `|` tek başına → `Token::Pipe`
- `||` → `Token::PipePipe` (mevcut)

**Pratt parser öncelikleri (infix_binding_power'a ekle):**
```
Token::Pipe    → BinOp::BitOr,   bp: (3, 4)   // || ile aynı seviye ama ayrı
Token::Caret   → BinOp::BitXor,  bp: (5, 6)
Token::Amp     → BinOp::BitAnd,  bp: (7, 8)
Token::LtLt    → BinOp::Shl,     bp: (11, 12)
Token::GtGt    → BinOp::Shr,     bp: (11, 12)
Token::Tilde   → UnaryOp::BitNot (prefix)
```

**AST BinOp enum'a ekle:**
```rust
BitAnd, BitOr, BitXor, Shl, Shr
```

**AST UnaryOp enum'a ekle:**
```rust
BitNot  // ~x
```

**TypeChecker kuralları:**
- Bitwise ops sadece integer tiplerde geçerli (u8/u16/u32/u64/i8/i16/i32/i64/Integer)
- `Float` üzerinde bitwise → type error
- Sonuç tipi: operandların tipiyle aynı

**Sözdizimi:**
```arimo
u32 flags = Permission.READ | Permission.WRITE;
u32 masked = flags & 0xFF;
u32 toggled = flags ^ 0x01;
u32 shifted = value << 3;
u8  inv = ~byte;
```

---

### 1.3 struct Keyword (Value Type)
**Öncelik: YÜKSEK** — game engine için kritik

**Temel fark: class vs struct**
```
class  → heap'te, referans semantiği, otomatik bellek yönetimi
struct → stack'te, value semantiği, kopyalanır, heap yok
```

**Lexer'a ekle:**
```
Token::Struct
```

**AST'ye ekle:**
```rust
pub struct StructDecl {
    pub visibility : Visibility,
    pub name       : String,
    pub generics   : Vec<String>,
    pub fields     : Vec<StructField>,
    pub methods    : Vec<Method>,
}

pub struct StructField {
    pub name : String,
    pub ty   : Type,
}

// Item enum'a ekle:
Item::Struct(StructDecl)
```

**Parser:** `public struct Name { field: Type; ... }` → StructDecl

**TypeChecker kuralları:**
- struct oluşturma: `Vec3(1.0, 2.0, 3.0)` veya `Vec3 { x: 1.0, y: 2.0, z: 3.0 }`
- struct atama → kopyalanır (clone), move değil
- struct metotları `this` ile alan erişir
- struct miras alamaz (extends yok), interface implemente edebilir

**BorrowChecker:** struct tipler copy semantiği — move takibi yok

**Sözdizimi:**
```arimo
public struct Vec3 {
    x : Float;
    y : Float;
    z : Float;

    public length() : Float {
        return Math.sqrt(this.x * this.x + this.y * this.y + this.z * this.z);
    }
}

Vec3 a = Vec3(1.0, 0.0, 0.0);
Vec3 b = a;   // kopyalanır, a hâlâ geçerli
```

---

### 1.4 Operator Overloading
**Öncelik: YÜKSEK** — game math için kritik

**Lexer'a ekle:**
```
Token::Operator   // "operator" keyword
```

**AST:** Method'a `operator_: Option<String>` ekle (veya ayrı OperatorMethod)

**Parser:** `public operator +(other: Vec3) : Vec3 { ... }` → normal Method, name = "operator+"

**TypeChecker:** BinOp çözümlemede önce builtin kontrol, sonra `operator+` metodu ara

**Desteklenecek operatörler:**
```arimo
operator +   operator -   operator *   operator /   operator %
operator ==  operator !=  operator <   operator <=  operator >  operator >=
operator []  (index erişimi)
```

**Kural:** Sadece `struct` ve `class`'larda, sadece binary/unary operatörler

**Sözdizimi:**
```arimo
public struct Vec3 {
    ...
    public operator +(other: Vec3) : Vec3 {
        return Vec3(this.x + other.x, this.y + other.y, this.z + other.z);
    }
}

Vec3 c = a + b;   // Vec3.operator+(a, b) çağrılır
```

---

### 1.5 @inline Annotation
**Öncelik: ORTA** — game engine optimizasyonu

**Lexer:** `@inline` → `Token::At` + `Ident("inline")` (mevcut @ ile yeterli)

**AST:** Method'a `inline_: bool` ekle

**Parser:** Metot başında `@inline` → method.inline_ = true

**CodeGen:** LLVM `alwaysinline` attribute ekle

**Sözdizimi:**
```arimo
@inline
public dot(other: Vec3) : Float {
    return this.x * other.x + this.y * other.y + this.z * other.z;
}
```

---

### 1.6 Array<T, N> Fixed-size Array
**Öncelik: ORTA** — game engine ve systems için

**Lexer:** `Array` → `Token::TypeArray`

**AST:**
```rust
Type::Array(Box<Type>, usize)   // Array<Float, 16>
```

**Parser:** `Array<Float, 16>` → Type::Array(Float, 16), ikinci parametre literal integer

**TypeChecker:**
- `Array.zeroed()` → sıfırlarla dolu array
- `arr[i]` → index erişimi (Expr::Index)
- `arr.length()` → Integer
- Sınır dışı erişim: compile-time sabit index için kontrol, runtime için @manual'da kullanıcı sorumlu

**Sözdizimi:**
```arimo
Array<Float, 16> matrix = Array.zeroed();
matrix[0] = 1.0;
Integer len = matrix.length();   // 16
```

---

## FAZA 2 — CodeGen (LLVM / inkwell)

**Bağımlılık:** `Cargo.toml`'a inkwell ekle:
```toml
[dependencies]
inkwell = { version = "0.4", features = ["llvm17-0"] }
```

### 2.1 Temel Altyapı
- [ ] `src/codegen/mod.rs` — CodeGen struct, LLVM context/module/builder
- [ ] Type mapping: Arimo tipi → LLVM tipi
  ```
  Integer → i64
  Float   → f64
  Boolean → i1
  String  → { i8*, i64 } (ptr + length)
  Void    → void
  u8/i8   → i8
  u16/i16 → i16
  u32/i32 → i32
  u64/i64 → i64
  RawPtr<T> → T*
  ```
- [ ] Primitif literal kod üretimi
- [ ] Aritmetik operatörler

### 2.2 Bellek Yönetimi (BorrowChecker drop schedule kullan)
- [ ] **Katman 1 (BorrowChecker Zone):** scope çıkışında `free()` insert
  - BorrowChecker'ın `drop_schedule` kullan
  - LIFO sırasında LLVM `free` call insert
- [ ] **Katman 2 (ARC):** escape analysis + refcount
  - Her heap objesi için `{ refcount: i64, data: T }` struct
  - Constructor → `alloc + refcount=1`
  - Method arg → `retain(ptr)` + call + `release(ptr)`
  - Scope çıkış → `release(ptr)`, 0 ise `free(ptr)`
- [ ] **Katman 3 (GC):** döngüsel referans tespiti — sonraya bırak

### 2.3 Fonksiyon & Metot Üretimi
- [ ] Static metot → LLVM function
- [ ] Instance metot → LLVM function (ilk param: `this` pointer)
- [ ] Constructor → alloc + field init + return pointer
- [ ] `main()` → LLVM `main` entry point

### 2.4 Kontrol Akışı
- [ ] if/else → LLVM branch
- [ ] while → LLVM loop
- [ ] for-each → iterator pattern
- [ ] klasik for → counter loop
- [ ] switch → LLVM switch veya chain of branches

### 2.5 Runtime Kütüphane (arc_runtime)
```
arc_alloc(size: u64) → void*      // malloc wrapper
arc_free(ptr: void*)              // free wrapper
arc_retain(ptr: void*)            // refcount++
arc_release(ptr: void*)           // refcount--, free if 0
arc_print(str: char*)             // IO.print impl
arc_panic(msg: char*, line: u32)  // hata ve exit
```

### 2.6 @manual Sınıflar
- [ ] `Memory.alloc(n)` → `malloc(n)` LLVM call
- [ ] `Memory.free(ptr)` → `free(ptr)` LLVM call
- [ ] Drop schedule üretilmez (BorrowChecker zaten atlıyor)

### 2.7 String
- [ ] String literal → global LLVM constant
- [ ] String interpolation → `sprintf` veya arc_concat
- [ ] String metodları → runtime impl

### 2.8 Koleksiyonlar
- [ ] `List<T>` → dynamic array impl (arc_list_*)
- [ ] `HashMap<K,V>` → hash table impl (arc_map_*)
- [ ] Koleksiyon metodları → runtime call

### 2.9 Exception
- [ ] `throw` → `longjmp` veya LLVM `landingpad`
- [ ] `try/catch` → LLVM exception handling
- [ ] `finally` → cleanup block

### 2.10 Output
- [ ] Object file üretimi (`.o`)
- [ ] Native binary (linker çağrısı)
- [ ] `arc file.arm` → `./file` (Linux/macOS/Windows)

---

## FAZA 3 — İleri Dil Özellikleri

### 3.1 C FFI
```arimo
extern "C" {
    printf(fmt: RawPtr<u8>, ...) : i32;
    malloc(size: u64)            : RawPtr<Void>;
}
```
- Lexer: `Token::Extern`
- AST: `ExternBlock { abi: String, decls: Vec<ExternDecl> }`
- Parser: `extern "C" { ... }`
- CodeGen: LLVM `declare` + C calling convention

### 3.2 Inline Assembly
```arimo
asm {
    mov rax, 60
    xor rdi, rdi
    syscall
}
```
- Sadece `@manual` sınıflarda
- Lexer: `Token::Asm`
- AST: `Stmt::Asm(String)` — raw asm string
- CodeGen: LLVM inline asm

### 3.3 @nostd
```arimo
@nostd
module kernel.boot;
```
- Stdlib import yok
- `main()` yerine `_start()` entry point
- Linker: `-nostdlib` flag

### 3.4 async/await
```arimo
public async fetchUser(id: String) : User? {
    Response res = await Http.get("/users/${id}");
    return res.json<User>();
}
```
- Coroutine / state machine transform
- Lexer: `Token::Async`, `Token::Await`
- AST: `async_: bool` on Method, `Expr::Await`
- CodeGen: state machine veya Rust-style poll model

### 3.5 Diğer
- [ ] `as` cast operatörü: `value as u32`
- [ ] Multi-catch: `catch (Exc1 | Exc2 e)`
- [ ] Default metot parametreleri
- [ ] Destructuring: `Pair<String, Integer> (k, v) = pair`

---

## FAZA 4 — Tooling

- [ ] VSCode extension güncelleme (yeni keyword'ler için)
- [ ] Language Server Protocol (LSP) — autocomplete, go-to-def
- [ ] Package manager (`arc.toml` manifest)
- [ ] Standard library (`arimo.io`, `arimo.net`, `arimo.fs`)
- [ ] Bootstrapping — arc'ı Arimo ile yeniden yaz

---

## Önemli Notlar Yeni Oturum İçin

### Commit kuralı
- `Co-Authored-By: Claude` satırı ASLA ekleme
- Merge commit bırakma — cherry-pick kullan
- Worktree branch'ini master'a cherry-pick ile aktar

### Worktree çalışma düzeni
```powershell
# Değişiklik yap (worktree'de)
cd "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\<worktree-adı>"

# Commit (worktree'de)
git add ...
git commit -m "..."

# Master'a aktar
cd "C:\Users\Arimo\Desktop\arimo-compiler"
git cherry-pick <commit-hash>
git push origin master
```

### Derleme ve test
```powershell
cd "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\<worktree-adı>"
cargo build
.\target\debug\arc.exe src\tests\samples\comprehensive.arm
```

### Beklenen çıktı (her test dosyası için)
```
arc: parse OK
arc: type check OK
arc: borrow check OK
```

### Bilinen sınırlar
- Lambda tipi `Named("Lambda")` — tam tip çıkarımı yok
- BorrowChecker method call'ları borrow sayıyor (move değil) — false negative olabilir
- Katman 2 (ARC) ve Katman 3 (GC) henüz CodeGen'de implement edilmedi
- `Expr::Index` AST'de var ama parser oluşturmuyor — `Array<T,N>` ile birlikte eklenecek
