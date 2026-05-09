# Arimo Lang — Proje El Teslim Belgesi

## Proje Özeti

Arimo Lang; OS, game engine ve uygulama geliştirmeye uygun, modern, statik tipli,
GC'siz, OOP odaklı bir programlama dili.
Compiler adı: `arc` | Kaynak uzantısı: `.arm` | Compiler dili: Rust

**Temel felsefe:**
- Kullanıcı `&`, `mut`, lifetime yazmaz — compiler arka planda halleder
- Otomatik bellek yönetimi varsayılan (3 katmanlı: BorrowChecker → ARC → GC)
- `@ManualMemory` annotation ile tam kontrol
- C FFI opsiyonel — OS için `asm{}` yeterli
- `const` yok — `static readonly` aynı rolü doldurur

---

## Mevcut Compiler Pipeline

```
.arm → Lexer → Parser → TypeChecker → BorrowChecker → CodeGen → .o → .exe
  ✅      ✅       ✅          ✅              ✅           🚧
```

**Hedef:** `.arm → arc → .exe` (kullanıcı sadece bunu görür)

---

## Proje Yapısı (2026-05-09)

```
arimo-compiler/
├── Cargo.toml          — inkwell 0.9 (llvm21-1 feature)
├── Cargo.lock
├── .cargo/config.toml  — LLVM prefix, linker, FFI workaround
├── src/
│   ├── main.rs              ✅ Pipeline: parse → typecheck → borrow → codegen → link
│   ├── lexer/mod.rs         ✅ Tüm tokenlar, tüm keyword'ler
│   ├── ast/mod.rs           ✅ Tüm node tipleri
│   ├── parser/mod.rs        ✅ Pratt parser — tam v1.4 syntax
│   ├── typechecker/mod.rs   ✅ Tam tip sistemi + annotation enforcement
│   ├── borrow/mod.rs        ✅ Ownership tracking
│   └── codegen/mod.rs       🚧 LLVM IR üretimi — temel OOP çalışıyor
└── documents/
    ├── arimo-lang-v1.4-documentation.md  ← güncel dil spesifikasyonu
    ├── arimo-lang-task-list.md            ← bu dosya
    └── arimo-lang-roadmap.md              ← yol haritası
```

### Test Dosyaları

```
src/tests/samples/
├── hello.arm            → temel class/method
├── comprehensive.arm    → v1.3 full OOP coverage → ✅ native binary üretiyor
├── phase1a.arm          → fixed-size ints, bitwise, cast, type alias
├── phase1c.arm          → struct, operator overloading, @ForceInline
├── phase1d.arm          → Array, Slice, function pointers
├── phase1ef.arm         → enum with data, match, Result, generic bounds
├── phase2.arm           → volatile, union, extern, asm, noreturn, defer
├── phase3.arm           → SIMD, @Likely/@Unlikely, default methods, async/await
├── annotations.arm      → @Deprecated, @FunctionalInterface, @Immutable, @Sealed...
├── codegen_hello.arm    → Hello from Arimo! → native binary ✅
├── codegen_basic.arm    → değişkenler, aritmetik, if/else, while, for → ✅
├── codegen_methods.arm  → static metodlar, parametreler, return → ✅
├── codegen_test2.arm    → float, bool, bitwise, string interpolation → ✅
├── codegen_class.arm    → class instances, constructor, this.field → ✅
├── codegen_enum.arm     → enum variants, switch, enum metodlar → ✅
├── codegen_inherit.arm  → inheritance (extends) → ✅
├── codegen_super.arm    → super() constructor, parent field access → ✅
└── codegen_static.arm   → static fields (global), mutable/readonly → ✅
```

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1 — Lexer
- Tüm Arimo token'ları
- String interpolation, hex/binary literal
- Systems: volatile, union, extern, asm, defer, noreturn
- Performance: async, await, default
- Variadic: `...` (Ellipsis)

### ✅ Milestone 2 — Parser & AST
- Pratt parser — tam v1.4 syntax
- Class, struct, interface, enum, exception, union, extern block
- Generic bounds, operator overloading, pattern matching
- Tüm annotation'lar (PascalCase, parametreli)
- async/await, defer, asm {}

### ✅ Milestone 3 — TypeChecker
- Tam tip sistemi, null safety, generic bounds
- Visibility, abstract/override, return path analizi
- SIMD tipler, stdlib (IO/Math/Time/Memory), Result<T,E>
- `@FunctionalInterface` → 1 abstract metod enforce
- `@Immutable` → readonly field enforce
- `@Sealed` → aynı modülden extend enforce
- `@Deprecated`/`@Experimental` → kullanımda uyarı
- Interface default metodlar, async/await

### ✅ Milestone 4 — BorrowChecker
- UseAfterMove, MoveWhileBorrowed, MutationWhileBorrowed
- Drop schedule (LIFO) — CodeGen için hazır
- @ManualMemory tamamen atlanır

### ✅ Milestone 5-9 — Faza 1 (1a–1f)
Dil temeli — tüm özellikleri tamamlandı.

### ✅ Milestone 10 — Faza 2: Systems
volatile, union, extern "C", asm{}, defer, noreturn, tüm systems annotation'lar.

### ✅ Milestone 11 — Faza 3: Performance
SIMD, @Likely/@Unlikely, interface default metodlar, async/await.

### ✅ Milestone 12 — Annotation Sistemi
16 annotation, PascalCase, TypeChecker enforcement.

### ✅ Milestone 13 — CodeGen Altyapısı (Faza 4.1–4.9)
- **LLVM 21.1.8** + **inkwell 0.9.0** entegrasyonu
- `x86_64-pc-windows-gnu` target
- Type mapping (Integer→i64, Float→f64, Bool→i1, String→ptr, Enum→i32, Class→ptr)
- Tüm operatörler (aritmetik, bitwise, karşılaştırma, compound, unary, cast)
- Kontrol akışı (if/else, while, for, switch) — unreachable terminatör düzeltmesi
- Static metodlar + parametreler + return değerleri
- `.arm → .o (LLVM) → .exe (gcc linker)` pipeline
- `.o` geçici dizinde, kullanıcı sadece `.exe` görür
- `--emit-ir` flag: LLVM IR çıktısı

### ✅ Milestone 14 — CodeGen OOP (Faza 4.5–4.9)
- **Class instances**: malloc constructor, GEP field erişimi, this pointer
- **VarSlot.class_name**: method dispatch için class tipi takibi
- **StaticCall → instance dispatch**: `c.method()` doğru çözümleniyor
- **Enum**: i32 sabitler, enum metodları, switch tam coverage
- **Inheritance**: parent fields struct'ta önce, super() çalışıyor
- **field_arimo_types**: `this.priority.isUrgent()` gibi zincirli dispatch
- **Static fields**: LLVM global değişken, initializer, okuma/yazma
- **Stdlib stubs**: Time.generateId() (LLVM inline counter), Math, Memory
- **`comprehensive.arm` → native binary**: 10/10 codegen testi geçiyor

---

## Test Durumu (2026-05-09)

### Frontend (Lexer → TypeChecker → BorrowChecker)
```
hello.arm         → parse OK + type OK + borrow OK
comprehensive.arm → parse OK + type OK + borrow OK
phase1a-1f.arm    → parse OK + type OK + borrow OK (hepsi)
phase2.arm        → parse OK + type OK + borrow OK
phase3.arm        → parse OK + type OK + borrow OK
annotations.arm   → parse OK + type OK + borrow OK
```

### CodeGen (→ native .exe)
```
codegen_hello.arm    → Hello from Arimo! ✅
codegen_basic.arm    → if/else/while/for/aritmetik ✅
codegen_methods.arm  → static metodlar, return ✅
codegen_test2.arm    → float, bool, bitwise, interpolation ✅
codegen_class.arm    → class instances, fields, metodlar ✅
codegen_enum.arm     → enum variants, switch, metodlar ✅
codegen_inherit.arm  → inheritance ✅
codegen_super.arm    → super(), parent field access ✅
codegen_static.arm   → static fields ✅
comprehensive.arm    → native binary üretiyor ✅ (collections stub)
```

---

## Tasarım Kararları

### Bellek Yönetimi — 3 Katmanlı Hibrit
```
Katman 1 — BorrowChecker Zone: compile-time, runtime overhead sıfır
Katman 2 — ARC: refcount++ / refcount--
Katman 3 — GC: döngüsel referans
```
> Şu an: `malloc` ile basit alloc. ARC CodeGen'de implement edilecek.

### struct vs class
```
class              → heap, reference semantics (malloc)
struct             → stack, value semantics, copy
@ManualMemory class → heap, manual memory, zero GC overhead
```

### Annotation Sistemi
- PascalCase — Java/Kotlin stilinde
- Açıklayıcı tam isimler: `@ManualMemory`, `@ForceInline`, `@FunctionalInterface`
- Parametreli: `@Align(16)`, `@CallingConvention("C")`, `@Deprecated("msg")`

### CodeGen Mimarisi
- **LLVM 21 / inkwell 0.9** — doğrudan IR üretimi, C intermediary yok
- **gcc** sadece linker olarak kullanılır (`.o → .exe`)
- `.o` dosyası `%TEMP%` klasöründe, kullanıcıya görünmez
- `arc hello.arm` → `hello.exe` (tek komut)

---

## Sıradaki Adım — Collections Runtime (Faza 4.11)

### Öneri: Adım Adım Yaklaşım
1. **List<T>** — `arc_list_*` runtime fonksiyonları (malloc, append, get, length)
2. **HashMap<K,V>** — basit lineer arama impl (arc_map_*)
3. **Pair<A,B>** — 2-field LLVM struct
4. **Lambda** — LLVM function pointer + closure
5. **String metodları** (length, contains, split)
6. **try/catch** — abort() basit impl
7. **ARC** — BorrowChecker drop_schedule kullanarak free() insert

### Sonra
- Systems CodeGen (volatile, asm, noreturn, @Packed, @Align)
- Performance CodeGen (SIMD, @Likely, async state machine)
- Release mode (-O2)
- Faza 5: Stdlib + Tooling

---

## Bilinen Sınırlar (2026-05-09)

| Sınır | Açıklama |
|---|---|
| Lambda tip çıkarımı | Parametreler `Unknown` tipte |
| Generic instantiation | Yüzeysel — tam doğrulama yok |
| Interface generic param | `Foo<T>` → T scope'a girilmiyor |
| match exhaustiveness | Sealed/enum tam kapsama kontrolü yok |
| Collections | CodeGen'de stub — gerçek veri yapıları yok |
| Lambda CodeGen | Parse+AST tamam; LLVM function pointer yok |
| async/await | Parse+AST tamam; state machine yok |
| ARC / GC | CodeGen'de henüz implement edilmedi |
| String metodları | TypeChecker'da var; CodeGen'de yok |
| try/catch | TypeChecker'da var; CodeGen'de yok |
