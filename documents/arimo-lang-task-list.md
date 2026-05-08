# Arimo Lang — Proje El Teslim Belgesi

## Proje Özeti

Arimo Lang; OS, game engine ve uygulama geliştirmeye uygun, modern, statik tipli,
GC'siz, OOP odaklı bir programlama dili.
Compiler adı: `arc` | Kaynak uzantısı: `.arm` | Compiler dili: Rust

**Temel felsefe:**
- Kullanıcı `&`, `mut`, lifetime yazmaz — compiler arka planda halleder
- Otomatik bellek yönetimi varsayılan (3 katmanlı: BorrowChecker → ARC → GC)
- `@manual` annotation ile performans kritik kodda tam kontrol
- C FFI opsiyonel — OS için `asm{}` yeterli, userland için C kütüphaneleri
- `const` yok — `static readonly` aynı rolü doldurur, tekrarlama olmaz

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1 — Lexer (v1.0)
`src/lexer/mod.rs`
- Tüm Arimo token'ları
- String interpolation `${}` desteği
- `@` (annotation), `RawPtr`, `Memory` token'ları

### ✅ Milestone 2 — Parser & AST (v1.0–v1.3)
`src/parser/mod.rs` + `src/ast/mod.rs`
- Pratt parser — tüm v1.3 syntax
- `super(...)` constructor çağrısı
- `List()` / `HashMap()` / `TreeMap()` v1.3 constructor syntax
- `@manual` annotation parse
- `RawPtr<T>` tip parse
- Exception sınıfları `Item::Exception` olarak ayrıştırılıyor
- `Float.sizeOf()` gibi primitif tip static çağrıları

### ✅ Milestone 3 — Type Checker (v1.3)
`src/typechecker/mod.rs`
- Sembol tablosu (class, interface, enum, exception, struct)
- Tip çıkarımı (infer_expr)
- Null safety — smart cast, nullable tip
- Visibility kontrolü (public/private/protected/internal)
- Abstract metod implementasyon kontrolü
- Return path analizi (all_paths_return)
- Enum exhaustiveness (switch)
- Builtin koleksiyon metodları (List, Map, HashMap, TreeMap, Pair)
- IO / Math / Time / Memory stdlib
- `@manual` sınıflarda `RawPtr<T>` field'larına izin
- `Memory.alloc/free/copy`, `RawPtr.read/write/offset`
- `T.sizeOf()` her tip için

### ✅ Milestone 4 — Borrow Checker (v1.3)
`src/borrow/mod.rs`
- UseAfterMove — taşınmış değişkeni kullanma
- MoveWhileBorrowed — itere edilirken taşıma
- MutationWhileBorrowed — for-each sırasında mutasyon
- Copy tipler: Integer, Float, Boolean (move takibi yok)
- Move tipler: String, user-defined class, koleksiyonlar
- Scope bazlı drop schedule (LIFO sırası) — CodeGen için
- `@manual` sınıflar tamamen atlanıyor

### ✅ Milestone 5 — Phase 1a: Integers + Bitwise + Cast
- `u8 u16 u32 u64 i8 i16 i32 i64` — fixed-size integer tipler
- `& | ^ ~ << >>` — bitwise operatörler, doğru öncelik tablosu
- `0xFF00` / `0b1010` — hex ve binary literal
- `value as u32` — explicit type cast
- Implicit widening YOK — güvenli by design
- Integer literal → herhangi bir sized integer atanabilir

### ✅ Milestone 6 — Phase 1b: Type Alias
- `type NodeId = u32;` — modül seviyesinde tip takma adı
- TypeChecker alias expansion ile şeffaf çözümleme

### ✅ Milestone 7 — Phase 1c: struct + Operator Overloading + @inline
- `struct` keyword — stack-allocated, copy semantics
- `Vec3(1.0, 2.0, 3.0)` — auto-constructor (field sırası)
- `operator +`, `operator ==` vb. — class ve struct'ta overloading
- TypeChecker: `a + b` için `operator+` metodu aranır
- `@inline` annotation — method seviyesinde, CodeGen'de `alwaysinline`
- BorrowChecker: struct copy tipi, move takibi yok

### ✅ Milestone 8 — Phase 1d: Array + Slice + FnPtr + Index
- `Array<Float, 16>` — compile-time boyutlu stack array
- `Array.zeroed()`, `arr[i]`, `arr.length()`, `arr.asSlice()`
- `Slice<T>` — non-owning fat pointer (ptr + len)
- `(Integer, Integer) -> Boolean` — function pointer type
- Lambda → FnPtr uyumlu atama
- `arr[i]` index operatörü (Array, Slice, List, Map)
- `is_fn_ptr_type_ahead` — statement'ta `(T)->R name = ...` tespiti

### ✅ Milestone 9 — Phase 1e+1f: Generic Bounds + Enum Data + match + Result
- `<T: Interface>` ve `<T: A + B>` generic bounds syntax
- Bound-aware method resolution on generic params
- Enum variant'lar veri taşıyabilir: `Circle(Float)`, `Rectangle(Float, Float)`
- Pure variant'lar (data yok) eskisi gibi çalışır
- `match expr { Pattern(a, b) => expr, _ => expr }` — pattern matching
- Generic substitution: match arm'larında binding tipleri çıkarılır
- `Result<T, E>` yerleşik generic enum — `Ok(T)`, `Err(E)`, `isOk()`, `isErr()`
- Generic enum instantiation + Unknown wildcard assignability
- `FatArrow` (=>) token eklendi

---

## Mevcut Compiler Pipeline

```
.arm → Lexer → Parser → TypeChecker → BorrowChecker → [CodeGen ← SIRADA]
  ✅      ✅       ✅          ✅              ✅
```

---

## Proje Yapısı (2026-05-09)

```
arimo-compiler/
├── Cargo.toml
├── Cargo.lock
├── .gitignore
├── src/
│   ├── main.rs              ✅ Pipeline: parse → type check → borrow check
│   ├── lexer/mod.rs         ✅ Tüm tokenlar, Phase 1a-1f token'ları
│   ├── ast/mod.rs           ✅ Tüm node tipleri, GenericParam, StructDecl, EnumVariant
│   ├── parser/mod.rs        ✅ Pratt parser, match, struct, operator, fn ptr
│   ├── typechecker/mod.rs   ✅ Tam tip sistemi, generic bounds, Result, match
│   ├── borrow/mod.rs        ✅ Ownership tracking, struct copy, match
│   └── codegen/mod.rs       ❌ Sadece stub (pub struct CodeGen;)
├── documents/
│   ├── arimo-lang-v1-documentation.md
│   ├── arimo-lang-v1.1-documentation.md
│   ├── arimo-lang-v1.2-documentation.md
│   ├── arimo-lang-v1.3-documentation.md
│   ├── arimo-lang-task-list.md        ← bu dosya
│   └── arimo-lang-roadmap.md          ← yol haritası
└── src/tests/samples/
    ├── hello.arm
    ├── comprehensive.arm   ← v1.3 full coverage (7 items)
    ├── phase1a.arm         ← fixed-size ints, bitwise, cast, type alias
    ├── phase1c.arm         ← struct, operator overloading, @inline
    ├── phase1d.arm         ← Array, Slice, function pointers
    └── phase1ef.arm        ← enum with data, match, Result, generic bounds
```

---

## Önemli Tasarım Kararları

### Bellek Yönetimi — 3 Katmanlı Hibrit
```
Katman 1 — BorrowChecker Zone (compile-time):
  BorrowChecker tek sahipliği kanıtlayabilirse → scope çıkışında free
  Runtime overhead: sıfır

Katman 2 — ARC (Reference Counting):
  Değer paylaşılıyorsa → refcount++ / refcount--
  Runtime overhead: küçük

Katman 3 — GC:
  Döngüsel referans → GC heap
  Runtime overhead: var ama küçük heap
```

### @manual Boundary
- Sınıf seviyesinde opt-in — class seviyesinde en temiz sınır
- Field/metod seviyesinde olsaydı drop schedule belirsizleşirdi
- `@manual` class nesnesi normal class'a parametre olabilir (borrow, free etmez)

### struct vs class
```
class  → heap, reference semantics, ARC/GC
struct → stack, value semantics, copy
@manual class → heap, manual memory, zero GC overhead
```

### const yok
- `static readonly` + compile-time constant değeri aynı rolü doldurur
- Tekrar ortadan kalktı, dil tutarlı

### Move vs Borrow Kuralları
- Method call argümanları → borrow (taşımaz)
- Constructor call argümanları → move
- `this.field = x` constructor içi → move
- `return localVar` → move
- Copy tipler: Integer, Float, Boolean, u8..i64, struct, Array, Slice, FnPtr

### Operator Overloading
- `operator+` normal method ismi olarak saklanır — yeni AST node yok
- TypeChecker: `a + b` numeric değilse `operator+` metodu arar
- Sadece binary operatörler (`[]` dahil)

### Generic Bounds
- `<T: Interface>` → `GenericParam { name: "T", bounds: ["Interface"] }`
- TypeChecker: generic param üzerinde method çağrısında bound kontrolü
- Bound tanımlıysa bound'daki interface'in metodlarını çözümle

### Result<T, E>
- Yerleşik generic enum — kullanıcı tanımına gerek yok
- `Result.Ok("x")` → `Generic("Result", [Str, Unknown])`
- Assignment uyumu: Unknown wildcard ile esnek

---

## Test Durumu (2026-05-09)

```
src/tests/samples/hello.arm         → parse OK + type OK + borrow OK
src/tests/samples/comprehensive.arm → parse OK + type OK + borrow OK
src/tests/samples/phase1a.arm       → parse OK + type OK + borrow OK
src/tests/samples/phase1c.arm       → parse OK + type OK + borrow OK
src/tests/samples/phase1d.arm       → parse OK + type OK + borrow OK
src/tests/samples/phase1ef.arm      → parse OK + type OK + borrow OK
```

---

## Sıradaki Adım — FAZA 2: CodeGen

Detaylar: `arimo-lang-roadmap.md`

### Öneri: Adım Adım Yaklaşım
1. **2.1 + 2.3 + 2.4** — Temel altyapı + fonksiyon üretimi + kontrol akışı
   - İlk çalışan `.arm → binary` pipeline
   - `main()` → executable, `IO.print()` çalışıyor
2. **2.2 Katman 1** — BorrowChecker Zone drop schedule → free()
   - Scope çıkışında otomatik bellek serbest bırakma
3. **2.6** — @manual sınıfları (Memory.alloc/free)
4. **2.7 + 2.8** — String ve koleksiyonlar
5. **2.2 Katman 2** — ARC (reference counting)
6. **2.9** — Exception handling
7. **2.10 + 2.11** — Struct/Array/Slice + output

### Sonraki Öncelik
- Faza 3: Systems (volatile, @packed, C FFI, inline asm, @nostd)
- Faza 4: Performance (@likely/@unlikely, defer, async/await)

---

## Dil Spesifikasyonu Özeti (Mevcut — 2026-05-09)

### Tipler
```
// Primitifler
Integer   Float   Boolean   String   Void

// Fixed-size integers
u8  u16  u32  u64   (unsigned)
i8  i16  i32  i64   (signed)

// Koleksiyonlar
List<T>   Map<K,V>   HashMap<K,V>   TreeMap<K,V>   Pair<A,B>
Array<T, N>   (fixed-size, stack)
Slice<T>      (non-owning view)

// Systems
RawPtr<T>     (sadece @manual)

// Generic
Result<T, E>  (yerleşik enum: Ok(T), Err(E))

// Fonksiyon pointer
(T1, T2) -> R

// Kullanıcı tanımlı
class   struct   enum (plain + with data)   interface   exception

// Type alias
type NodeId = u32;
```

### Operatörler
```
+ - * / %       (aritmetik)
== != < <= > >= (karşılaştırma)
&& ||           (mantıksal)
& | ^ ~ << >>   (bitwise)
= += -= *= /=   (atama)
++ --           (artırma/azaltma)
?: (ternary)    ?. (null-safe)   as (cast)
[]  (index)
operator +/-/*/ vb. (user-defined)
```

### Annotations
```
@manual   — sınıf düzeyinde manuel bellek yönetimi
@inline   — metod düzeyinde inlining hint
```

### Dil Yapıları
```
if / else if / else
while
for (each)   for (classic)
switch (enum exhaustiveness)
match (pattern matching, enum data destructure)
try / catch / finally
```
