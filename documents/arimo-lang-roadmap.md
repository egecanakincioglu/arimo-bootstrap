# Arimo Lang — Yol Haritası

> Proje detayları: `arimo-lang-task-list.md`

---

## Hedef

Modern bir programlama dili:
- **OS yazılabilir** — bare-metal, inline asm, donanım erişimi
- **Game engine yazılabilir** — sıfır GC overhead, struct, SIMD hint, operator overloading
- **Uygulama yazılabilir** — otomatik bellek, async/await, yüksek seviye OOP

---

## FAZA 1 — Dil Temeli ✅ TAMAMLANDI

### 1a. Fixed-size Integer Tipler + Bitwise Ops + as Cast ✅
- `u8 u16 u32 u64 i8 i16 i32 i64` — tüm boyutlarda integer
- `& | ^ ~ << >>` — bitwise operatörler, C öncelik sırası korundu
- `0xDEADBEEF` / `0b1010` — hex ve binary literal
- `value as u32` — explicit cast operatörü
- Implicit widening YOK — `u32 x = some_u64;` → type error
- Integer literal → any sized int (atama uyumu)

### 1b. Type Alias ✅
- `type NodeId = u32;` — modül seviyesinde tip takma adı
- TypeChecker alias expansion ile şeffaf çalışır
- **Not:** `const` eklenmedi — `static readonly` zaten o rolü dolduruyor

### 1c. struct (Value Type) + Operator Overloading + @ForceInline ✅
- `struct` keyword: stack-allocated, copy semantics, extends yok
- Auto-constructor: field sırasından otomatik üretilir
- `operator +` / `operator ==` vb. — class ve struct'ta
- `@ForceInline` annotation — metod seviyesinde, CodeGen'de `alwaysinline`
- BorrowChecker: struct tipler copy, move takibi yok

### 1d. Array<T,N> + Slice<T> + Function Pointers ✅
- `Array<Float, 16>` — compile-time boyutlu, stack array
- `Array.zeroed()`, `arr[i]`, `arr.length()`, `arr.asSlice()`
- `Slice<T>` — non-owning fat pointer (ptr + len)
- `(Integer, Integer) -> Boolean` — function pointer type
- Lambda → FnPtr atama uyumu
- `arr[i]` index operatörü (Array, Slice, List, Map)

### 1e. Generic Bounds ✅
- `<T: Interface>` syntax — class, struct, interface tanımlarında
- `<T: A + B>` — birden fazla bound
- TypeChecker: bound tanımlı ise generic param üzerinde metod çağrısı geçerli
- GenericParam { name, bounds } — AST'de

### 1f. Enum with Data + match + Result<T,E> ✅
- Enum variant'lar veri taşıyabilir: `Circle(Float)`, `Rectangle(Float, Float)`
- Saf variant'lar eskisi gibi çalışır: `Low`, `Medium`, `High`
- `match expr { Enum.Variant(a, b) => expr, _ => expr }` — pattern matching
- `Result<T, E>` yerleşik generic enum: `Ok(T)`, `Err(E)`, `isOk()`, `isErr()`
- Generic enum instantiation: `Result.Ok("x")` → `Result<String, Unknown>`
- Enum variant constructor: `Shape.Circle(1.5)` → `Type::Named("Shape")`

---

## FAZA 2 — Systems Desteği ⬜ SIRADA

### 2.1 Bellek Layout Kontrol
```arimo
@Packed
public struct PacketHeader {
    magic   : u16;
    version : u8;
    flags   : u8;
}

@Align(16)
public struct SimdVec {
    data : Array<Float, 4>;
}
```
- Lexer: `@Packed`, `@align(N)` annotation'ları
- AST: StructDecl'da `packed: bool`, `align: Option<usize>`
- CodeGen: LLVM struct packed layout, alignment attribute

### 2.2 volatile Keyword
```arimo
// Memory-mapped I/O için
volatile u32 status = hardware_register.read();
```
- Lexer: `Token::Volatile`
- AST: VarDecl'da `volatile: bool`
- CodeGen: LLVM `volatile load/store`
- Sadece `@ManualMemory` sınıflar ve `@Freestanding` modüllerde kullanım tavsiyesi

### 2.3 union Type
```arimo
public union Register {
    full  : u32;
    bytes : Array<u8, 4>;
}
```
- OS/embedded için register overlapping
- Lexer: `Token::Union`
- AST: `UnionDecl`, `Item::Union`
- TypeChecker: union erişimi `@ManualMemory` gerektiriyor
- CodeGen: LLVM union layout

### 2.4 C FFI
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
- Variadics (`...`) desteği

### 2.5 Inline Assembly
```arimo
@ManualMemory
public class Syscall {
    public static exit(code: i32) : Void {
        asm {
            mov rax, 60
            mov rdi, {code}
            syscall
        }
    }
}
```
- Sadece `@ManualMemory` sınıflarda
- Lexer: `Token::Asm`
- AST: `Stmt::Asm(String)`
- CodeGen: LLVM inline asm

### 2.6 @Freestanding + @Section + Calling Conventions
```arimo
@Freestanding
module kernel.boot;

@Section(".boot")
@CallingConvention("C")
public static _start() : Void { ... }
```
- `@Freestanding` → stdlib import yok, `_start()` entry point, `-nostdlib` linker
- `@Section(".text.init")` → linker section
- `@CallingConvention("C")`, `@CallingConvention("Windows")`, `@CallingConvention("Interrupt")` → calling convention

### 2.7 noreturn
```arimo
public static panic(msg: String) : noreturn {
    IO.print(msg);
    // LLVM unreachable — optimizer bilir
}
```
- Lexer/AST: `Type::NoReturn` veya method attribute
- CodeGen: LLVM `unreachable` terminator

---

## FAZA 3 — Performance Desteği ⬜

### 3.1 SIMD
```arimo
Vec4f a = Vec4f(1.0, 2.0, 3.0, 4.0);
Vec4f b = Vec4f(5.0, 6.0, 7.0, 8.0);
Vec4f c = a + b;   // LLVM SIMD instruction
```
- Yerleşik SIMD tipler: `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i`
- Operator overloading ile LLVM vectorized operations
- CodeGen: LLVM vector types

### 3.2 Branch Prediction Hints
```arimo
if @likely (fast_path) { ... }
if @unlikely (error_path) { ... }
```
- Lexer: `@likely`, `@unlikely` expression annotation
- CodeGen: LLVM `branch_weights` metadata

### 3.3 Interface Default Methods
```arimo
interface Updatable {
    update(dt: Float) : Void;

    default updateBatch(items: List<Updatable>, dt: Float) : Void {
        for (Updatable item : items) {
            item.update(dt);
        }
    }
}
```
- AST: Method'da `default_: bool` (body olan interface metodu)
- TypeChecker: default metotlar override edilmeyebilir

### 3.4 defer Statement
```arimo
public static openFile(path: String) : Void {
    File f = File.open(path);
    defer f.close();   // scope çıkışında çalışır, exception olsa bile
    // ...
}
```
- Lexer: `Token::Defer`
- AST: `Stmt::Defer(Expr)`
- CodeGen: scope çıkışında, LIFO sırasında çalıştır

### 3.5 async/await
```arimo
public async fetchUser(id: String) : User? {
    Response res = await Http.get("/users/${id}");
    return res.json<User>();
}
```
- Coroutine / state machine transform
- Lexer: `Token::Async`, `Token::Await`
- AST: `async_: bool` on Method, `Expr::Await`
- CodeGen: state machine veya poll model

---

## FAZA 4 — CodeGen (LLVM / inkwell) ⬜

**Bağımlılık:**
```toml
[dependencies]
inkwell = { version = "0.4", features = ["llvm17-0"] }
```

### 4.1 Temel Altyapı
- [ ] `src/codegen/mod.rs` — CodeGen struct, LLVM context/module/builder
- [ ] Type mapping: Arimo tipi → LLVM tipi
  ```
  Integer   → i64      Float     → f64
  Boolean   → i1       String    → { i8*, i64 }
  Void      → void
  u8/i8     → i8       u16/i16   → i16
  u32/i32   → i32      u64/i64   → i64
  RawPtr<T> → T*       Array<T,N> → [N x T]
  Slice<T>  → { T*, i64 }
  FnPtr     → function pointer
  struct    → LLVM struct (stack-allocated)
  union     → LLVM union layout
  ```
- [ ] Primitif literal kod üretimi
- [ ] Aritmetik ve bitwise operatörler

### 4.2 Bellek Yönetimi
- [ ] **Katman 1 (BorrowChecker Zone):** scope çıkışında `free()` insert
- [ ] **Katman 2 (ARC):** escape analysis + refcount
- [ ] **Katman 3 (GC):** döngüsel referans tespiti — sonraya bırak

### 4.3 Fonksiyon & Metot Üretimi
- [ ] Static metot → LLVM function
- [ ] Instance metot → LLVM function (ilk param: `this` pointer)
- [ ] Constructor → alloc + field init + return pointer
- [ ] `main()` → LLVM `main` entry point
- [ ] Operator methods → LLVM function

### 4.4 Kontrol Akışı
- [ ] if/else → LLVM branch
- [ ] while → LLVM loop
- [ ] for-each → iterator pattern
- [ ] klasik for → counter loop
- [ ] switch/match → LLVM switch veya chain of branches
- [ ] defer → scope çıkışında LIFO

### 4.5 Runtime Kütüphane (arc_runtime)
```
arc_alloc(size: u64) → void*
arc_free(ptr: void*)
arc_retain(ptr: void*)
arc_release(ptr: void*)
arc_print(str: char*, len: i64)
arc_panic(msg: char*, line: u32)
arc_str_concat(...)
```

### 4.6 Systems CodeGen
- [ ] `volatile load/store` — LLVM volatile flag
- [ ] `union` → LLVM union layout (max field size)
- [ ] `extern "C"` → LLVM `declare` + C calling convention
- [ ] `asm {}` → LLVM inline asm
- [ ] `@Freestanding` → `-nostdlib` linker flag
- [ ] `@Section` → LLVM section attribute
- [ ] `@CallingConvention("C")` / `@CallingConvention("Windows")` / `@CallingConvention("Interrupt")` → calling convention
- [ ] `noreturn` → LLVM `unreachable` terminator
- [ ] `@Packed` → LLVM packed struct
- [ ] `@align(N)` → LLVM alignment attribute

### 4.7 Performance CodeGen
- [ ] SIMD types → LLVM vector types
- [ ] `@likely` / `@unlikely` → LLVM branch_weights metadata
- [ ] `async/await` → state machine transform

### 4.8 String & Koleksiyonlar
- [ ] String literal → global LLVM constant
- [ ] String interpolation → arc_str_concat
- [ ] `List<T>` → arc_list_*
- [ ] `HashMap<K,V>` → arc_map_*

### 4.9 Exception
- [ ] `throw` → LLVM `landingpad`
- [ ] `try/catch` → LLVM exception handling

### 4.10 Output
- [ ] Object file üretimi (`.o`)
- [ ] Native binary (linker çağrısı)
- [ ] `arc file.arm` → `./file`

---

## FAZA 5 — Stdlib + Tooling ⬜

- [ ] `arimo.io` — dosya sistemi, stdin/stdout
- [ ] `arimo.net` — TCP/UDP, HTTP client
- [ ] `arimo.fs` — path, directory, file
- [ ] `arimo.collections` — gelişmiş koleksiyonlar
- [ ] `arimo.time` — tarih/saat
- [ ] VSCode extension güncelleme (yeni keyword'ler)
- [ ] Language Server Protocol (LSP) — autocomplete, go-to-def
- [ ] Package manager (`arc.toml` manifest)
- [ ] Bootstrapping — arc'ı Arimo ile yeniden yaz

---

## Önemli Notlar Yeni Oturum İçin

### Commit kuralı
- Merge commit bırakma — cherry-pick kullan
- Worktree branch'ini master'a cherry-pick ile aktar

### Worktree çalışma düzeni
```powershell
# Değişiklik yap (worktree'de)
cd "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\<worktree-adı>"
git add ...
git commit -m "..."

# Master'a aktar ve push
cd "C:\Users\Arimo\Desktop\arimo-compiler"
git cherry-pick <commit-hash>
git push origin master
```

### Derleme ve test
```powershell
cd "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\<worktree-adı>"
cargo build
.\target\debug\arc.exe src\tests\samples\comprehensive.arm
.\target\debug\arc.exe src\tests\samples\phase1ef.arm
```

### Beklenen çıktı
```
arc: parse OK
arc: type check OK
arc: borrow check OK
```

### Bilinen Sınırlar (2026-05-09 itibariyle)
- Lambda tip çıkarımı yok — parametreler `Unknown` tipte, false positive hatalar bastırılıyor
- `Expr::Index` atama hedefi (`arr[i] = val`) TypeChecker'da pass-through
- Generic instantiation yüzeysel: `Result<T,E>` çalışıyor ama tam tip doğrulama yok
- BorrowChecker method call argümanlarını borrow sayıyor (move değil) — false negative olabilir
- Katman 2 (ARC) ve Katman 3 (GC) CodeGen'de henüz implement edilmedi
- `@ForceInline` AST'de saklanıyor ama CodeGen olmadığı için çalışmıyor
- `match` exhaustiveness kontrolü henüz yok (tüm variant'ların kapsanması)
