# Arimo Lang — Proje El Teslim Belgesi (Güncel)

## Proje Özeti

Arimo Lang; OS, game engine ve uygulama geliştirmeye uygun, modern, statik tipli, GC'siz, OOP odaklı bir programlama dili.  
Compiler adı: `arc` | Kaynak uzantısı: `.arm` | Compiler dili: Rust

**Temel felsefe:**
- Kullanıcı `&`, `mut`, lifetime yazmaz — compiler arka planda halleder
- Otomatik bellek yönetimi varsayılan (3 katmanlı: BorrowChecker → ARC → GC)
- `@manual` annotation ile performans kritik kodda tam kontrol
- C FFI opsiyonel — OS için `asm{}` yeterli, userland için C kütüphaneleri

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1 — Lexer
`src/lexer/mod.rs`
- Tüm Arimo token'ları
- String interpolation `${}` desteği
- `@` (annotation), `RawPtr`, `Memory` token'ları eklendi

### ✅ Milestone 2 — Parser & AST
`src/parser/mod.rs` + `src/ast/mod.rs`
- Pratt parser — 11/11 test dosyası parse OK
- `super(...)` constructor çağrısı
- `List()` / `HashMap()` / `TreeMap()` v1.3 constructor syntax
- `Math.PI` gibi parantez olmayan stdlib sabit erişimi
- `@manual` annotation parse
- `RawPtr<T>` tip parse
- Exception sınıfları `Item::Exception` olarak ayrıştırılıyor
- `Float.sizeOf()` gibi primitif tip static çağrıları
- `extends Exception` → TypeException token kabul ediliyor

### ✅ Milestone 3 — Type Checker
`src/typechecker/mod.rs`
- Sembol tablosu (class, interface, enum, exception)
- Tip çıkarımı (infer_expr)
- Null safety — smart cast, nullable tip
- Visibility kontrolü (public/private/protected/internal)
- Abstract metod implementasyon kontrolü
- Return path analizi (all_paths_return)
- Enum exhaustiveness (switch)
- Builtin koleksiyon metodları (List, Map, HashMap, TreeMap, Pair)
- IO / Math / Time stdlib
- `@manual` sınıflarda `RawPtr<T>` field'larına izin
- `Memory.alloc/free/copy`, `RawPtr.read/write/offset`
- `T.sizeOf()` her tip için
- `RawPtr<Void>` → `RawPtr<T>` atama (void* benzeri)
- `StaticCall` için değişken/class ayrımı
- Lambda scope (Unknown wildcard)
- `Map ← HashMap/TreeMap` subtype
- Pair tipi method çağrısı

### ✅ Milestone 4 — Borrow Checker
`src/borrow/mod.rs`
- UseAfterMove — taşınmış değişkeni kullanma
- MoveWhileBorrowed — itere edilirken taşıma
- MutationWhileBorrowed — for-each sırasında `.append()` / `.set()` vb.
- Copy tipler: Integer, Float, Boolean (move takibi yok)
- Move tipler: String, user-defined class, koleksiyonlar
- Scope bazlı drop schedule (LIFO sırası) — CodeGen için
- `@manual` sınıflar tamamen atlanıyor

### ✅ @manual Annotation
- Sınıf seviyesinde opt-in manuel bellek yönetimi
- `RawPtr<T>` tip desteği
- `Memory.alloc()` / `Memory.free()` / `Memory.copy()`
- `T.sizeOf()` static metod
- BorrowChecker ve otomatik drop devre dışı
- Normal sınıfta `RawPtr<T>` kullanmak → type error

---

## Mevcut Compiler Pipeline

```
.arm → Lexer → Parser → TypeChecker → BorrowChecker → [CodeGen ← SIRADA]
  ✅      ✅       ✅          ✅              ✅
```

---

## Proje Yapısı

```
arimo-compiler/
├── Cargo.toml
├── Cargo.lock
├── .gitignore
├── src/
│   ├── main.rs              ✅ Tam pipeline: parse → type check → borrow check
│   ├── lexer/mod.rs         ✅ Tamamlandı
│   ├── ast/mod.rs           ✅ Tamamlandı
│   ├── parser/mod.rs        ✅ Tamamlandı
│   ├── typechecker/mod.rs   ✅ Tamamlandı
│   ├── borrow/mod.rs        ✅ Tamamlandı
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
    └── comprehensive.arm
```

---

## Önemli Tasarım Kararları

### Bellek Yönetimi — 3 Katmanlı Hibrit
```
Katman 1 — BorrowChecker Zone (compile-time):
  BorrowChecker tek sahipliği kanıtlayabilirse → scope çıkışında free
  Runtime overhead: sıfır

Katman 2 — ARC (Reference Counting):
  Değer paylaşılıyorsa (koleksiyona giriyorsa) → refcount++ / refcount--
  Runtime overhead: küçük

Katman 3 — GC:
  Döngüsel referans veya çok karmaşık → GC heap
  Runtime overhead: var ama küçük heap
```

Kullanıcı hangi katmanda olduğunu görmez.

### @manual
- Sınıf seviyesinde opt-in
- Otomasyona hiç girmez
- RawPtr<T>, Memory.alloc/free erişimi
- Kullanıcı dispose() çağırmakla sorumlu

### Move vs Borrow Kuralları
- Method call argümanları → borrow (taşımaz)
- Constructor call argümanları → move
- `this.field = x` constructor içi → move
- `return localVar` → move
- Copy tipler: Integer, Float, Boolean

### Parser Mimarisi
- `Ident + Dot + method()` → StaticCall (parser sınırı)
- TypeChecker: class değilse variable method call olarak yönlendiriyor
- Exception tespiti: `extends Exception` veya `ends_with("Exception")`

### C FFI Felsefesi
- C ABI = evrensel arabirim, C kodu değil
- OS kernel için gerekli değil → `asm{}` yeterli
- Userland C kütüphaneleri için (OpenGL, SQLite, zlib)

---

## Master Branch Commit Geçmişi

```
b82f4fb  @manual annotation: manuel bellek yonetimi destegi
a20cb59  BorrowChecker: ownership tracking, use-after-move, mutation-while-borrowed
05b8693  TypeChecker: lambda scope, Unknown type passthrough, Pair method support
d4a3d1a  Fix parser & typechecker, wire pipeline, add .gitignore
40bed40  Borrow source added
3a158b7  Code Gen source added
0a966ea  Type Checker files added
2a513eb  Test ARM files added
6b46638  Parser files added
```

---

## Test Durumu

```
src/tests/samples/hello.arm         → parse OK + type check OK + borrow check OK
src/tests/samples/comprehensive.arm → parse OK + type check OK + borrow check OK

comprehensive.arm kapsamı:
  ✅ Enum + switch exhaustiveness
  ✅ Exception + super(...)
  ✅ Interface + abstract class
  ✅ Inheritance (extends, implements)
  ✅ List() / HashMap() / Pair() constructors
  ✅ Lambda + .filter() zinciri
  ✅ for-each, klasik for, while
  ✅ Ternary, null safety, smart cast
  ✅ try / catch / finally
  ✅ String interpolation
```

---

## Sıradaki Adım — Öncelik Sırası

### Öncelik 1: Dil Spec Genişletmesi (CodeGen'den önce)
1. Fixed-size integer tipler: `u8 u16 u32 u64 i8 i16 i32 i64`
2. Bitwise operatörler: `& | ^ ~ << >>`
3. `struct` keyword (value type, stack-allocated)
4. Operator overloading (`operator +`, `operator ==` vb.)
5. `@inline` annotation
6. `Array<T, N>` fixed-size array

### Öncelik 2: CodeGen — LLVM (inkwell)
Detaylar: arimo-lang-roadmap.md

### Öncelik 3: İleri Özellikler
- C FFI (`extern "C" {}`)
- Inline assembly (`asm {}`)
- `@nostd` annotation
- `async/await`

---

## Dil Spesifikasyonu Özeti (v1.3 + Planlanan)

### Mevcut Tipler
```
Integer   Float   Boolean   String   Void
List<T>   Map<K,V>   HashMap<K,V>   TreeMap<K,V>   Pair<A,B>
RawPtr<T>  (sadece @manual)
```

### Planlanan Tipler
```
u8  u16  u32  u64
i8  i16  i32  i64
Array<T, N>   (fixed-size, stack)
struct        (value type, stack-allocated)
```

### Mevcut Operatörler
```
+ - * / %   (aritmetik)
== != < <= > >=   (karşılaştırma)
&& ||   (mantıksal)
= += -= *= /=   (atama)
++ --   (artırma/azaltma)
?: (ternary)   ?. (null-safe)
```

### Planlanan Operatörler
```
& | ^ ~ << >>   (bitwise)
operator keyword ile overloading
```

### Mevcut Annotations
```
@manual   — manuel bellek yönetimi
```

### Planlanan Annotations
```
@inline   — fonksiyon inlining
@nostd    — stdlib'siz, bare-metal
@packed   — struct memory layout kontrolü
```
