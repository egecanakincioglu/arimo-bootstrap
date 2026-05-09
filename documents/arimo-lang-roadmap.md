# Arimo Lang — Yol Haritası

> Proje detayları: `arimo-lang-task-list.md`

---

## Hedef

Modern bir programlama dili:
- **OS yazılabilir** — bare-metal, inline asm, donanım erişimi
- **Game engine yazılabilir** — sıfır GC overhead, struct, SIMD, operator overloading
- **Uygulama yazılabilir** — otomatik bellek, async/await, yüksek seviye OOP

---

## FAZA 1 — Dil Temeli ✅ TAMAMLANDI

### 1a. Fixed-size Integer Tipler + Bitwise + as Cast ✅
- `u8 u16 u32 u64 i8 i16 i32 i64`
- `& | ^ ~ << >>` — bitwise operatörler, C öncelik sırası
- `0xDEADBEEF` / `0b1010` — hex ve binary literal
- `value as u32` — explicit cast, implicit widening YOK

### 1b. Type Alias ✅
- `type NodeId = u32;` — modül seviyesinde tip takma adı
- TypeChecker alias expansion ile şeffaf çalışır

### 1c. struct + Operator Overloading + @ForceInline ✅
- `struct` — stack-allocated, copy semantics, auto-constructor
- `operator +` / `operator ==` vb. — class ve struct'ta
- `@ForceInline` — method seviyesinde inlining hint
- BorrowChecker: struct tipler copy, move takibi yok

### 1d. Array<T,N> + Slice<T> + Function Pointers ✅
- `Array<Float, 16>` — compile-time boyutlu stack array
- `Slice<T>` — non-owning fat pointer (ptr + len)
- `(Integer, Integer) -> Boolean` — function pointer type
- Lambda → FnPtr uyumu

### 1e. Generic Bounds ✅
- `<T: Interface>` ve `<T: A + B>` syntax
- TypeChecker: bound üzerinde method çözümleme

### 1f. Enum with Data + match + Result<T,E> ✅
- Veri taşıyan enum variant: `Circle(Float)`, `Rectangle(Float, Float)`
- `match expr { Pattern(a, b) => expr, _ => expr }`
- `Result<T, E>` yerleşik: `Ok(T)`, `Err(E)`, `isOk()`, `isErr()`

---

## FAZA 2 — Systems Desteği ✅ TAMAMLANDI

### 2.1 Bellek Layout Kontrol ✅
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

### 2.2 volatile ✅
```arimo
volatile u32 status = hardware_register.read();
```
- CodeGen: LLVM `volatile load/store` (Faza 4)

### 2.3 union ✅
```arimo
public union Register {
    full  : u32;
    bytes : Array<u8, 4>;
}
```

### 2.4 C FFI ✅
```arimo
extern "C" {
    printf(fmt: RawPtr<u8>, ...) : i32;
    malloc(size: u64)            : RawPtr<Void>;
}
```

### 2.5 Inline Assembly ✅
```arimo
@ManualMemory
public class Syscall {
    public static exit(code: i32) : noreturn {
        asm {
            mov rax, 60
            syscall
        }
    }
}
```

### 2.6 @Freestanding + @Section + @CallingConvention ✅
```arimo
@Freestanding
module kernel.boot;

@Section(".boot")
@CallingConvention("C")
public static _start() : Void { ... }

@CallingConvention("Interrupt")
public static onTimer() : Void { ... }
```

### 2.7 noreturn ✅
```arimo
public static panic(msg: String) : noreturn {
    IO.print(msg);
}
```

### 2.8 defer ✅
```arimo
public static readFile(path: String) : Void {
    File f = File.open(path);
    defer f.close();
}
```

---

## FAZA 3 — Performance Desteği ✅ TAMAMLANDI

### 3.1 SIMD Tipleri ✅
```arimo
Vec4f a = Vec4f(1.0, 2.0, 3.0, 4.0);
Vec4f b = Vec4f(5.0, 6.0, 7.0, 8.0);
Vec4f c = a + b;
Float len = a.length();
```
- `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i`
- Operator overloading + `length()`, `normalize()`, `dot()`
- CodeGen: LLVM vector types (Faza 4)

### 3.2 Branch Prediction Hints ✅
```arimo
if @Likely (fast_path) { ... }
if @Unlikely (error_path) { ... }
```
- CodeGen: LLVM `branch_weights` metadata (Faza 4)

### 3.3 Interface Default Methods ✅
```arimo
interface Updatable {
    update(dt: Float) : Void;

    default updateFixed() : Void {
        IO.print("fixed update");
    }
}
```

### 3.4 async / await ✅
```arimo
public async fetchUser(id: String) : String {
    String result = await ApiService.getData(id);
    return result;
}
```
- CodeGen: state machine transform (Faza 4)

### 3.5 Annotation Sistemi ✅

Tüm annotation'lar — PascalCase, Java/Kotlin stilinde:

| Annotation | Seviye | TypeChecker davranışı |
|---|---|---|
| `@ManualMemory` | class | GC kapalı, BorrowChecker atlar |
| `@ForceInline` | method | Kaydedilir, CodeGen'de alwaysinline |
| `@Freestanding` | module | -nostdlib flag |
| `@Packed` | struct | LLVM packed struct (Faza 4) |
| `@Align(N)` | struct | LLVM alignment (Faza 4) |
| `@Section("...")` | method | Linker section (Faza 4) |
| `@CallingConvention("...")` | method | Calling convention (Faza 4) |
| `@Likely` / `@Unlikely` | if | Branch hint (Faza 4) |
| `@Deprecated("msg")` | class/method | Kullanıldığında uyarı |
| `@Experimental` | class/method | Kullanıldığında uyarı |
| `@FunctionalInterface` | interface | 1 abstract method zorunlu |
| `@Throws(ExType, ...)` | method | Kaydedilir |
| `@SuppressWarnings("tip")` | class/method | Kaydedilir |
| `@Sealed` | class/interface | Aynı modülden extend zorunlu |
| `@Pure` | method | Kaydedilir, CodeGen opt. |
| `@Immutable` | class | Tüm field'lar readonly zorunlu |

---

## FAZA 4 — CodeGen (LLVM / inkwell) ⬜ SIRADA

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
  Void      → void     noreturn  → LLVM unreachable
  u8/i8     → i8       u16/i16   → i16
  u32/i32   → i32      u64/i64   → i64
  RawPtr<T> → T*       Array<T,N> → [N x T]
  Slice<T>  → { T*, i64 }
  FnPtr     → function pointer
  struct    → LLVM struct (stack-allocated)
  union     → LLVM union layout (max field size)
  Vec4f     → <4 x float>   Vec8f → <8 x float>
  Vec4i     → <4 x i32>     Vec8i → <8 x i32>
  ```
- [ ] Primitif literal kod üretimi
- [ ] Aritmetik ve bitwise operatörler

### 4.2 Bellek Yönetimi
- [ ] **Katman 1 (BorrowChecker Zone):** scope çıkışında `free()` insert
  - BorrowChecker'ın `drop_schedule` kullan
  - LIFO sırasında LLVM `free` call insert
- [ ] **Katman 2 (ARC):** escape analysis + refcount
  - Her heap objesi için `{ refcount: i64, data: T }` struct
  - Constructor → `alloc + refcount=1`
  - Method arg → `retain(ptr)` + call + `release(ptr)`
- [ ] **Katman 3 (GC):** döngüsel referans tespiti — sonraya bırak

### 4.3 Fonksiyon & Metot Üretimi
- [ ] Static metot → LLVM function
- [ ] Instance metot → LLVM function (ilk param: `this` pointer)
- [ ] Constructor → alloc + field init + return pointer
- [ ] `main()` → LLVM `main` entry point
- [ ] Operator methods → LLVM function call

### 4.4 Kontrol Akışı
- [ ] if/else → LLVM branch
- [ ] while → LLVM loop
- [ ] for-each → iterator pattern
- [ ] klasik for → counter loop
- [ ] switch/match → LLVM switch + pattern destructure
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
- [ ] `union` → LLVM union layout
- [ ] `extern "C"` → LLVM `declare` + C calling convention
- [ ] `asm {}` → LLVM inline asm
- [ ] `@Freestanding` → `-nostdlib` linker flag
- [ ] `@Section` → LLVM section attribute
- [ ] `@CallingConvention` → calling convention attributes
- [ ] `noreturn` → LLVM `unreachable` terminator
- [ ] `@Packed` → LLVM packed struct
- [ ] `@Align(N)` → LLVM alignment attribute

### 4.7 Performance CodeGen
- [ ] SIMD → LLVM vector types + vectorized ops
- [ ] `@Likely`/`@Unlikely` → LLVM branch_weights
- [ ] `async/await` → state machine transform
- [ ] `@ForceInline` → LLVM `alwaysinline`
- [ ] `@Pure` → LLVM `readnone`/`readonly`

### 4.8 String & Koleksiyonlar
- [ ] String literal → global LLVM constant
- [ ] String interpolation → `arc_str_concat`
- [ ] `List<T>` → `arc_list_*`
- [ ] `HashMap<K,V>` → `arc_map_*`

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
- [ ] VSCode extension — syntax highlighting v1.4
- [ ] Language Server Protocol (LSP)
- [ ] Package manager (`arc.toml` manifest)
- [ ] Bootstrapping — arc'ı Arimo ile yeniden yaz

---

## Önemli Notlar

### Commit Kuralı
- Merge commit bırakma — cherry-pick kullan
- Worktree branch'ini master'a cherry-pick ile aktar

### Worktree Çalışma Düzeni
```powershell
# Değişiklik yap (worktree'de)
cd "<worktree-dizini>"
git add ...
git commit -m "..."

# Master'a aktar ve push
cd "C:\Users\Arimo\Desktop\arimo-compiler"
git cherry-pick <commit-hash>
git push origin master
```

### Derleme ve Test
```powershell
cd "<worktree-dizini>"
cargo build
.\target\debug\arc.exe src\tests\samples\comprehensive.arm
.\target\debug\arc.exe src\tests\samples\phase2.arm
.\target\debug\arc.exe src\tests\samples\phase3.arm
.\target\debug\arc.exe src\tests\samples\annotations.arm
```

### Beklenen Çıktı
```
arc: parse OK
arc: type check OK
arc: borrow check OK
arc: drop schedule — N scope(s) tracked
```
