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
- `type NodeId = u32;`
- TypeChecker alias expansion

### 1c. struct + Operator Overloading + @ForceInline ✅
- `struct` — stack-allocated, copy semantics, auto-constructor
- `operator +` / `operator ==` vb. — class ve struct'ta
- `@ForceInline` annotation

### 1d. Array<T,N> + Slice<T> + Function Pointers ✅
- `Array<Float, 16>` — compile-time boyutlu stack array
- `Slice<T>` — non-owning fat pointer
- `(Integer, Integer) -> Boolean` — function pointer type
- Lambda → FnPtr uyumu

### 1e. Generic Bounds ✅
- `<T: Interface>` ve `<T: A + B>` syntax

### 1f. Enum with Data + match + Result<T,E> ✅
- Veri taşıyan enum variant: `Circle(Float)`, `Rectangle(Float, Float)`
- `match expr { Pattern(a, b) => expr, _ => expr }`
- `Result<T, E>` yerleşik: `Ok(T)`, `Err(E)`, `isOk()`, `isErr()`

---

## FAZA 2 — Systems Desteği ✅ TAMAMLANDI

### 2.1 @Packed / @Align ✅
### 2.2 volatile ✅
### 2.3 union ✅
### 2.4 extern "C" + variadic ✅
### 2.5 asm { } inline assembly ✅
### 2.6 @Freestanding + @Section + @CallingConvention ✅
### 2.7 noreturn ✅
### 2.8 defer ✅

---

## FAZA 3 — Performance + Annotation Sistemi ✅ TAMAMLANDI

### 3.1 SIMD Tipleri ✅
- `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i` — TypeChecker'da kayıtlı

### 3.2 @Likely / @Unlikely ✅
- `if @Likely (cond)` — AST'de saklanır, CodeGen'de implement edilecek

### 3.3 Interface default metodlar ✅
- `default methodName() : Type { body }` — override etmek zorunda değil

### 3.4 async / await ✅
- Parse+AST tamam; CodeGen state machine (sonraki adım)

### 3.5 Annotation Sistemi ✅ (PascalCase, Java/Kotlin stilinde)

| Annotation | Seviye | Durum |
|---|---|---|
| `@ManualMemory` | class | ✅ BorrowChecker atlar |
| `@ForceInline` | method | ✅ Kaydedilir |
| `@Freestanding` | module | ✅ -nostdlib flag |
| `@Packed` | struct | ✅ Kaydedilir |
| `@Align(N)` | struct | ✅ Kaydedilir |
| `@Section("...")` | method | ✅ Kaydedilir |
| `@CallingConvention("...")` | method | ✅ Kaydedilir |
| `@Likely` / `@Unlikely` | if | ✅ AST'de BranchHint |
| `@Deprecated("msg")` | class/method | ✅ Kullanımda uyarı |
| `@Experimental` | class/method | ✅ Kullanımda uyarı |
| `@FunctionalInterface` | interface | ✅ 1 abstract metod zorunlu |
| `@Throws(...)` | method | ✅ Kaydedilir |
| `@SuppressWarnings(...)` | class/method | ✅ Kaydedilir |
| `@Sealed` | class/interface | ✅ Aynı modülden extend zorunlu |
| `@Pure` | method | ✅ Kaydedilir |
| `@Immutable` | class | ✅ Tüm field'lar readonly zorunlu |

---

## FAZA 4 — CodeGen (LLVM / inkwell) 🚧 DEVAM EDİYOR

**Kurulum:**
- LLVM 21.1.8 (MSYS2 MinGW): `C:\msys64\mingw64`
- inkwell 0.9.0 — `llvm21-1` feature
- Target: `x86_64-pc-windows-gnu`
- `.cargo/config.toml` → linker + env ayarları

**Derleme:**
```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"
cargo build --target x86_64-pc-windows-gnu
.\target\x86_64-pc-windows-gnu\debug\arc.exe src\tests\samples\hello.arm
```

### 4.1 Temel Altyapı ✅
- CodeGen struct, LLVM context/module/builder
- Type mapping: Integer→i64, Float→f64, Boolean→i1, String→ptr, u/i8-64→i8-i64, Enum→i32, Class→ptr
- Primitif literal kod üretimi (IntLit, FloatLit, BoolLit, StrLit, StrInterp)
- String interpolation: `${x}` → printf format specifier
- `.arm → .o → .exe` pipeline (gcc sadece linker olarak)

### 4.2 Operatörler ✅
- Aritmetik: `+ - * / %` (int + float)
- Bitwise: `& | ^ ~ << >>` (tip uyum düzeltmesi)
- Karşılaştırma: `== != < <= > >=`
- Mantıksal: `&& ||`
- Compound assignment: `+= -= *=`
- Unary: `-x !x ~x ++x x++`
- Cast: `value as Type`

### 4.3 Kontrol Akışı ✅
- `if/else` — her iki dal return yapınca `unreachable` terminatörü
- `while` döngüsü
- Klasik `for` döngüsü
- `switch` — if/else zinciri, tüm case'ler return yapınca `unreachable`

### 4.4 Fonksiyon ve Metod Üretimi ✅
- Static metodlar + parametreler + return değerleri
- `main()` → LLVM `i32 @main()`
- Instance metodlar — first param `this` pointer
- Static call dispatch
- Instance call dispatch (parser `StaticCall` üretse de çözümleniyor)

### 4.5 Class Instances ✅
- Class struct tipi kaydı (LLVM struct type)
- Constructor → `malloc` + field init
- Field okuma: GEP + load
- Field yazma: GEP + store
- `this` pointer parametresi
- `VarSlot.class_name` → method dispatch

### 4.6 Enum CodeGen ✅
- Enum variant'lar → `i32` sabitler (North=0, South=1, ...)
- `Direction.North` → `i32 0`
- Enum metod gövdeleri (this = i32)
- Switch tüm case'ler return → `unreachable`

### 4.7 Inheritance ✅
- Parent struct field'ları child struct'ta önce gelir
- `super()` çağrısı → parent field'ları init eder
- Method lookup: child önce, parent sonra
- `field_arimo_types` → `this.field.method()` dispatch

### 4.8 Static Fields ✅
- `public static MAX : Integer = 50` → LLVM global değişken
- `public static readonly VERSION : String = "..."` → global + string init
- `Config.MAX` → global load
- `Config.count = x` → global store

### 4.9 Stdlib Stubs ✅
- `IO.print()` + string interpolation → printf
- `Math.sqrt/abs/pow/PI/E` → C math library
- `Time.now()` → sabit string stub
- `Time.generateId()` → LLVM inline counter (`arc_generate_id`)
- `Memory.alloc/free` → malloc/free

### 4.10 `comprehensive.arm` → native `.exe` ✅
- Tüm OOP özellikleri (class, enum, interface, inheritance) çalışıyor
- Pipeline: `.arm → LLVM IR → .o → .exe`

---

## FAZA 4 — Kalan CodeGen İşleri ⬜

### 4.11 Collections Runtime
- [ ] `List<T>` → malloc'd dinamik dizi, length + alloc tracked
  - `append()`, `length()`, `isEmpty()`, `get(i)`, `filter(lambda)`, `take(n)`
- [ ] `HashMap<K,V>` → basit lineer arama (ilk impl)
  - `set()`, `get()` nullable, `getOrDefault()`, `containsKey()`, `remove()`
- [ ] `Pair<A,B>` → 2-field struct
  - `getFirst()`, `getSecond()`

### 4.12 Lambda / Function Pointer Çalıştırma
- [ ] Lambda → LLVM function pointer
- [ ] `list.filter(lambda)` → lambda çağrısı
- [ ] `list.sortedBy(comparator)` → qsort veya inline sort

### 4.13 String Metodları
- [ ] `str.length()` → strlen
- [ ] `str.contains(sub)` → strstr
- [ ] `str.toUpper/toLower` → runtime impl
- [ ] `str.split(delim)` → List<String> döndür
- [ ] String `+` veya interpolation runtime concat

### 4.14 Exception Handling
- [ ] `throw` → abort() (basit)
- [ ] `try/catch` → LLVM landingpad (gelişmiş)

### 4.15 ARC Memory Management
- [ ] Scope çıkışında free() (BorrowChecker drop_schedule kullan)
- [ ] refcount++ / refcount-- for shared objects

### 4.16 Systems CodeGen (Faza 2 özelliklerinin LLVM'si)
- [ ] `volatile load/store`
- [ ] `extern "C"` → LLVM declare
- [ ] `asm {}` → LLVM inline asm
- [ ] `noreturn` → LLVM unreachable
- [ ] `@Packed` → LLVM packed struct
- [ ] `@Align(N)` → LLVM alignment

### 4.17 Performance CodeGen (Faza 3 özelliklerinin LLVM'si)
- [ ] SIMD → LLVM vector types (`<4 x float>` vb.)
- [ ] `@Likely/@Unlikely` → LLVM branch_weights
- [ ] `async/await` → state machine transform
- [ ] `@ForceInline` → LLVM `alwaysinline`
- [ ] `@Pure` → LLVM `readnone`

### 4.18 Output İyileştirme
- [ ] `-O2` release modu
- [ ] Debug info (DWARF)
- [ ] Cross-compilation (Linux/macOS)
- [ ] `-emit-llvm` flag (IR dosyası)

---

## FAZA 5 — Stdlib + Tooling ⬜

- [ ] `arimo.io` — dosya sistemi, stdin/stdout
- [ ] `arimo.net` — TCP/UDP, HTTP client
- [ ] `arimo.fs` — path, directory, file
- [ ] `arimo.collections` — gelişmiş koleksiyonlar
- [ ] `arimo.time` — gerçek tarih/saat
- [ ] VSCode extension — syntax highlighting v1.4
- [ ] Language Server Protocol (LSP)
- [ ] Package manager (`arc.toml` manifest)
- [ ] Bootstrapping — arc'ı Arimo ile yeniden yaz

---

## Önemli Notlar

### Commit Kuralı
- Merge commit bırakma — cherry-pick kullan
- Worktree branch'ini master'a cherry-pick ile aktar

### Çalışma Düzeni
```powershell
# Worktree'de değişiklik yap
cd "<worktree-dizini>"
git add ...
git commit -m "..."

# Master'a aktar
cd "C:\Users\Arimo\Desktop\arimo-compiler"
git cherry-pick <commit-hash>
git push origin master
```

### Derleme ve Test
```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"
cd "<worktree-dizini>"
cargo build --target x86_64-pc-windows-gnu
.\target\x86_64-pc-windows-gnu\debug\arc.exe src\tests\samples\comprehensive.arm
.\target\x86_64-pc-windows-gnu\debug\arc.exe src\tests\samples\codegen_class.arm
```

### Beklenen Çıktı
```
arc: compiling  ... linking ... OK
arc: → hello.exe
```
