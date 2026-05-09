# Arimo Lang — Proje El Teslim Belgesi

## Proje Özeti

Arimo Lang; OS, game engine ve uygulama geliştirmeye uygun, modern, statik tipli,
GC'siz, OOP odaklı bir programlama dili.
Compiler adı: `arc` | Kaynak uzantısı: `.arm` | Compiler dili: Rust

**Temel felsefe:**
- Kullanıcı `&`, `mut`, lifetime yazmaz — compiler arka planda halleder
- Otomatik bellek yönetimi varsayılan (3 katmanlı: BorrowChecker → ARC → GC)
- `@ManualMemory` annotation ile performans kritik kodda tam kontrol
- C FFI opsiyonel — OS için `asm{}` yeterli, userland için extern "C"
- `const` yok — `static readonly` aynı rolü doldurur

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
├── src/
│   ├── main.rs              ✅ Pipeline: parse → type check → borrow check
│   ├── lexer/mod.rs         ✅ Tüm tokenlar, tüm keyword'ler
│   ├── ast/mod.rs           ✅ Tüm node tipleri
│   ├── parser/mod.rs        ✅ Pratt parser — tam v1.4 syntax
│   ├── typechecker/mod.rs   ✅ Tam tip sistemi + annotation enforcement
│   ├── borrow/mod.rs        ✅ Ownership tracking
│   └── codegen/mod.rs       ❌ Sadece stub (pub struct CodeGen;)
└── documents/
    ├── arimo-lang-v1.4-documentation.md  ← güncel dil spesifikasyonu
    ├── arimo-lang-task-list.md            ← bu dosya
    └── arimo-lang-roadmap.md              ← yol haritası
```

### Test Dosyaları

```
src/tests/samples/
├── hello.arm          → temel class/method
├── comprehensive.arm  → v1.3 full coverage
├── phase1a.arm        → fixed-size ints, bitwise, cast, type alias
├── phase1c.arm        → struct, operator overloading, @ForceInline
├── phase1d.arm        → Array, Slice, function pointers
├── phase1ef.arm       → enum with data, match, Result, generic bounds
├── phase2.arm         → volatile, union, extern, asm, noreturn, defer, annotations
├── phase3.arm         → SIMD, @Likely/@Unlikely, default methods, async/await
└── annotations.arm    → @Deprecated, @FunctionalInterface, @Immutable, @Sealed, @Pure...
```

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1 — Lexer
`src/lexer/mod.rs`
- Tüm Arimo token'ları (keyword, literal, operator, delimiter)
- String interpolation `${}` desteği
- hex (`0x`) ve binary (`0b`) literal
- `@` annotation işareti
- Systems: `volatile`, `union`, `extern`, `asm`, `defer`, `noreturn`
- Performance: `async`, `await`, `default`
- Variadics: `...` (Ellipsis)

### ✅ Milestone 2 — Parser & AST
`src/parser/mod.rs` + `src/ast/mod.rs`
- Pratt parser — tam v1.4 syntax
- Class, struct, interface, enum, exception, union, extern block
- Generic bounds: `<T: Interface>`, `<T: A + B>`
- Operator overloading: `operator +`, `operator []` vb.
- Pattern matching: `match expr { Variant(a) => expr, _ => expr }`
- `@ManualMemory`, `@ForceInline`, `@Freestanding`, `@Packed`, `@Align(N)`
- `@Section`, `@CallingConvention("C"/"Windows"/"Interrupt")`
- `@Likely`, `@Unlikely` (if statement)
- `@Deprecated`, `@Experimental`, `@FunctionalInterface`, `@Throws`, `@SuppressWarnings`
- `@Sealed`, `@Pure`, `@Immutable`
- `super(...)` constructor çağrısı
- `async` method modifier, `await` expression
- `defer` statement
- `asm { }` inline assembly block

### ✅ Milestone 3 — TypeChecker
`src/typechecker/mod.rs`
- Sembol tablosu (class, interface, enum, exception, struct, union, extern)
- Tip çıkarımı (infer_expr) — tüm expression'lar
- Null safety — smart cast, nullable tip (`?`)
- Visibility kontrolü (public/private/protected/internal)
- Abstract metod implementasyon kontrolü
- Return path analizi (all_paths_return)
- Enum switch exhaustiveness
- Builtin koleksiyon metodları (List, Map, HashMap, TreeMap, Pair)
- Stdlib: IO, Math, Time, Memory
- `@ManualMemory` class'larda `RawPtr<T>` ve `asm {}` desteği
- `Memory.alloc/free/copy`, `RawPtr.read/write/offset`, `T.sizeOf()`
- `noreturn` dönüş tipi — `all_paths_return` kontrolünden muaf
- SIMD builtin tipler: `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i`
- `@FunctionalInterface`: tam 1 abstract method zorunlu
- `@Immutable`: tüm instance field'lar readonly zorunlu
- `@Sealed`: aynı modülden extend zorunlu
- `@Deprecated` / `@Experimental`: kullanımda uyarı (non-fatal)
- Interface `default` method'lar desteği
- `Expr::Await` — inner type döndürür
- Uyarı sistemi: `tc.warnings: Vec<String>`

### ✅ Milestone 4 — BorrowChecker
`src/borrow/mod.rs`
- UseAfterMove, MoveWhileBorrowed, MutationWhileBorrowed
- Copy tipler: Integer, Float, Boolean, u8..i64, struct, Array, Slice, FnPtr
- Scope bazlı drop schedule (LIFO) — CodeGen için hazır
- `@ManualMemory` class'lar tamamen atlanır

### ✅ Milestone 5-9 — Faza 1 (1a–1f)
- `u8..i64` fixed-size integers, bitwise, cast
- Type alias
- struct + operator overloading + @ForceInline
- Array<T,N> + Slice<T> + FnPtr
- Generic bounds
- Enum with data + match + Result<T,E>

### ✅ Milestone 10 — Faza 2: Systems
- `volatile`, `union`, `extern "C"`, `asm {}`, `defer`, `noreturn`
- `@ManualMemory`, `@Freestanding`, `@Packed`, `@Align(N)`
- `@Section`, `@CallingConvention`

### ✅ Milestone 11 — Faza 3: Performance
- SIMD: `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i`
- `@Likely` / `@Unlikely`
- Interface default methods
- `async` / `await`

### ✅ Milestone 12 — Annotation Sistemi
- `@Deprecated`, `@Experimental`, `@FunctionalInterface`
- `@Throws`, `@SuppressWarnings`
- `@Sealed`, `@Pure`, `@Immutable`

---

## Test Durumu (2026-05-09)

```
hello.arm         → parse OK + type OK + borrow OK
comprehensive.arm → parse OK + type OK + borrow OK
phase1a.arm       → parse OK + type OK + borrow OK
phase1c.arm       → parse OK + type OK + borrow OK
phase1d.arm       → parse OK + type OK + borrow OK
phase1ef.arm      → parse OK + type OK + borrow OK
phase2.arm        → parse OK + type OK + borrow OK
phase3.arm        → parse OK + type OK + borrow OK
annotations.arm   → parse OK + type OK + borrow OK
```

---

## Tasarım Kararları

### Bellek Yönetimi — 3 Katmanlı Hibrit
```
Katman 1 — BorrowChecker Zone: compile-time, runtime overhead sıfır
Katman 2 — ARC: refcount++ / refcount--
Katman 3 — GC: döngüsel referans
```

### struct vs class
```
class              → heap, reference semantics, ARC/GC
struct             → stack, value semantics, copy
@ManualMemory class → heap, manual memory, zero GC overhead
```

### Annotation Sistemi
- PascalCase — Java/Kotlin stilinde
- Açıklayıcı tam isimler: `@ManualMemory`, `@ForceInline`, `@FunctionalInterface`
- Parametreli: `@Align(16)`, `@CallingConvention("C")`, `@Deprecated("msg")`

### const Yok
- `static readonly` aynı rolü doldurur — tekrar ortadan kalkar

### Move vs Borrow
- Method call argümanları → borrow
- Constructor / return → move
- Copy tipler: Integer, Float, Boolean, u8..i64, struct, Array, Slice, FnPtr

---

## Sıradaki Adım — FAZA 4: CodeGen

Detaylar: `arimo-lang-roadmap.md`

### Öneri: Adım Adım Yaklaşım
1. **4.1 + 4.3 + 4.4** — Temel altyapı + fonksiyon + kontrol akışı → ilk binary
2. **4.2 Katman 1** — BorrowChecker drop schedule → free()
3. **4.6** — Systems CodeGen (volatile, extern, asm, noreturn)
4. **4.7** — Performance (SIMD, @ForceInline, @Likely/@Unlikely)
5. **4.8** — String + koleksiyonlar (arc_runtime)
6. **4.2 Katman 2** — ARC
7. **4.9** — Exception handling
8. **4.10** — Native binary output

---

## Bilinen Sınırlar (2026-05-09)

| Sınır | Açıklama |
|---|---|
| Lambda tip çıkarımı | Parametreler `Unknown` — false positive bastırılıyor |
| `arr[i] = val` | TypeChecker pass-through |
| Generic instantiation | Yüzeysel — tam doğrulama yok |
| Interface generic param | `Foo<T>` → T scope'a girilmiyor |
| match exhaustiveness | Sealed/enum tam kapsama kontrolü yok |
| BorrowChecker method args | Borrow (move değil) — false negative olabilir |
| async/await | Parse+AST tamam; CodeGen state machine henüz yok |
| ARC / GC | CodeGen'de henüz implement edilmedi |
