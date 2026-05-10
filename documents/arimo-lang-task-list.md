# Arimo Lang — Proje El Teslim Belgesi

## Proje Özeti

Arimo Lang; OS, game engine ve uygulama geliştirmeye uygun, modern, statik tipli,
GC-free, OOP odaklı bir programlama dili.
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
  ✅      ✅       ✅          ✅              ✅           ✅
```

**Hedef:** `.arm → arc → .exe` (kullanıcı sadece bunu görür)

---

## Proje Yapısı (2026-05-10)

```
arimo-compiler/
├── Cargo.toml          — inkwell 0.9 (llvm21-1 feature)
├── Cargo.lock
├── .cargo/config.toml  — LLVM prefix, linker, FFI workaround
├── src/
│   ├── main.rs              ✅ Pipeline + flag'ler (--emit-ir, -O2, -c)
│   ├── lexer/mod.rs         ✅ Tüm tokenlar, tüm keyword'ler
│   ├── ast/mod.rs           ✅ Tüm node tipleri
│   ├── parser/mod.rs        ✅ Pratt parser — tam v1.4 syntax
│   ├── typechecker/mod.rs   ✅ Tam tip sistemi + annotation enforcement
│   ├── borrow/mod.rs        ✅ Ownership tracking
│   └── codegen/mod.rs       ✅ LLVM IR — Faza 4.1–4.18 tamamlandı
└── documents/
    ├── arimo-lang-v1.4-documentation.md  ← güncel dil spesifikasyonu
    ├── arimo-lang-task-list.md            ← bu dosya
    └── arimo-lang-roadmap.md              ← yol haritası
```

### Test Dosyaları

```
src/tests/samples/
├── codegen_hello.arm      ✅ Hello from Arimo!
├── codegen_basic.arm      ✅ if/else/while/for/aritmetik
├── codegen_methods.arm    ✅ static metodlar, return
├── codegen_test2.arm      ✅ float, bool, bitwise, string interpolation
├── codegen_class.arm      ✅ class instances, fields, metodlar
├── codegen_enum.arm       ✅ enum variants, switch, metodlar
├── codegen_inherit.arm    ✅ inheritance
├── codegen_super.arm      ✅ super() constructor, parent field access
├── codegen_static.arm     ✅ static fields
├── codegen_phase4.arm     ✅ string metodları, break/continue, match
├── comprehensive.arm      ✅ native binary üretiyor
├── hello.arm              ✅ temel class/method
├── phase1a–1f.arm         ✅ Faza 1 özellikleri
├── phase2.arm             ✅ Systems özellikleri
├── phase3.arm             ✅ Performance özellikleri
└── annotations.arm        ✅ Annotation sistemi
```

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1–4 — Frontend
- Lexer: tüm Arimo token'ları, string interpolation, hex/binary literal
- Parser: Pratt parser, tam v1.4 syntax, tüm dil yapıları
- TypeChecker: tam tip sistemi, null safety, generic bounds, annotation enforcement
- BorrowChecker: UseAfterMove, MoveWhileBorrowed, drop schedule

### ✅ Milestone 5–12 — Faza 1–3 (Dil Özellikleri)
Tüm dil özellikleri parse + tip kontrolü seviyesinde tamamlandı.

### ✅ Milestone 13 — CodeGen Altyapısı (Faza 4.1–4.9)
- LLVM 21.1.8 + inkwell 0.9.0 entegrasyonu
- Tüm operatörler, kontrol akışı, fonksiyonlar, OOP
- `.arm → .o → .exe` pipeline

### ✅ Milestone 14 — CodeGen OOP (Faza 4.5–4.9)
- Class instances, enum, inheritance, static fields, stdlib stubs

### ✅ Milestone 15 — Collections Runtime (Faza 4.11)
- List, HashMap, Pair — saf LLVM IR implementasyonu
- Lambda filter, ForEach

### ✅ Milestone 16 — Faza 4.12–4.18 (2026-05-10)
- Break/Continue — loop stack
- String metodları — 14 metod (length, contains, startsWith, endsWith, compareTo, toUpper, toLower, split, indexOf, parseInt, parseFloat, trim, + concat)
- IO.println, IO.error
- Lambda genel codegen — fn pointer + ConstructorCall çağrısı
- Match expression — enum pattern matching
- Asm{} — LLVM create_inline_asm
- StructDecl @Packed codegen
- LLVM attribute'ları — @ForceInline, @Pure, @Section, @CallingConvention
- Output flag'leri — -O2, -c

---

## Sıradaki Görevler

### ⬜ Faza 4.19 — Kalan CodeGen Eksikleri

#### 4.19.1 ARC Bellek Yönetimi ← KRİTİK
- [ ] `refcount : i64` field'ını her class struct'a ekle
- [ ] Constructor'da `refcount = 1` başlat
- [ ] Atamada `refcount++` üret
- [ ] BorrowChecker `drop_schedule`'ını CodeGen'de oku
- [ ] Scope çıkışında: `refcount--` + `if == 0 { free() }`
- [ ] `@ManualMemory` class'larını atla

#### 4.19.2 Exception Handling — Gerçek Implementasyon ← KRİTİK
- [ ] `throw` → heap exception nesnesi + LLVM `invoke`/`resume`
- [ ] `try/catch` → LLVM `landingpad` + personality fonksiyonu
- [ ] Catch tip filtresi → RTTI tabanlı tip karşılaştırması
- [ ] `finally` → tüm çıkış yollarına emit et
- [ ] `Exception.message()` string döndürsün
- [ ] Exception kalıtımı → parent tip catch desteklensin

#### 4.19.3 Lambda Closure Capture ← KRİTİK
- [ ] Free variable analizi (hangi dış değişkenler kullanılıyor)
- [ ] Capture struct'ı oluştur (heap'te)
- [ ] Lambda fonksiyonuna closure ptr'ı parametre olarak geç
- [ ] Closure içindeki değişkene erişim → struct field'ından yükle

#### 4.19.4 Eksik Expression Codegen
- [ ] `Expr::NullSafeAccess` → null check + conditional dispatch
- [ ] `Expr::Super` → parent field/metod erişimi (constructor dışında)
- [ ] `Stmt::Defer` → scope sonunda LIFO sırasıyla çalıştır

#### 4.19.5 Systems Codegen Tamamlama
- [ ] `volatile` load/store → LLVM volatile flag
- [ ] `noreturn` → `noreturn` LLVM attribute
- [ ] `@Likely/@Unlikely` → `branch_weights` metadata
- [ ] `@Align(N)` → alloca + global alignment attribute
- [ ] SIMD operatörler → `<N x float>` IR + aritmetik

#### 4.19.6 Collections Eksikleri
- [ ] `List.sortedBy(fn)` → qsort + comparator
- [ ] `List.reduce(init, fn)` → fold
- [ ] `List.map(fn)` → yeni liste
- [ ] `List.flatMap(fn)` → düzleştir
- [ ] `List.any(fn)` / `List.all(fn)`
- [ ] `List.distinct()` — benzersiz liste
- [ ] `HashMap.entries()`, `keys()`, `values()`
- [ ] `HashMap.remove(key)` — slot temizle
- [ ] `HashMap.containsKey(key)` — gerçek impl

#### 4.19.7 String Tamamlama
- [ ] `str.substring(start, end)` → malloc + memcpy
- [ ] `str.replace(old, new)` → strstr + malloc + concat
- [ ] Enum → String dönüşümü interpolasyonda

#### 4.19.8 async/await State Machine
- [ ] LLVM coroutine intrinsics (`llvm.coro.*`)
- [ ] Suspend/resume noktaları
- [ ] Basit polling executor

---

### ⬜ Faza 5 — Stdlib

Sırasıyla işlenecek (her biri `.arm` dosyası olarak):

1. **arimo.fs** ← Bootstrap için öncelikli
2. **arimo.io** (tamamlama)
3. **arimo.env**
4. **arimo.time** (gerçek implementasyon)
5. **arimo.math** (eksikler)
6. **arimo.string** (tamamlama)
7. **arimo.collections** (gelişmiş)
8. **arimo.process**
9. **arimo.net** *(ileri aşama)*
10. **arimo.sync** *(Faza 9'a bağımlı)*

---

### ⬜ Faza 6 — Dil Genişletmeleri v2.0

Sırasıyla:

1. `??` null coalescing
2. `is` / `as?` type operators
3. `?` error propagation
4. Default + named parameters
5. Destructuring
6. `when` expression
7. Match guard clauses
8. String patterns in match
9. Range type + range patterns
10. Extension methods
11. `Char` tipi
12. Enum iteration (`.values()`)
13. Object copy (`.copy(field:)`)
14. Multiline + raw string literals
15. `@Test` / `@Benchmark` annotations
16. `const` expressions
17. Multiple exception catch

---

### ⬜ Faza 7 — Araçlar

1. Gelişmiş hata mesajları (satır/sütun + context + renk)
2. Uyarı sistemi (unused, dead code, shadow)
3. Debug bilgisi (DWARF)
4. Cross-compilation (Linux, macOS)
5. `arc.toml` manifest
6. `arc build/run/test/clean/check/init` CLI
7. Multi-file compilation
8. İnkremental derleme

---

### ⬜ Faza 8 — Araç Ekosistemi

1. VSCode extension (syntax highlighting)
2. LSP server
3. `arc fmt` formatter
4. `arc doc` documentation generator
5. `arc pkg` package manager

---

### ⬜ Faza 9 — Runtime & Concurrency

1. Thread spawn/join
2. Mutex + RwLock + Atomic
3. Channel (mpsc)
4. Async runtime (event loop)
5. Signal handling
6. Panic handler + stack traces
7. Thread-local storage

---

### ⬜ Faza 10 — Bootstrapping

Tüm önceki fazlar tamamlandıktan sonra.

---

## Bilinen Sınırlar (2026-05-10)

| Sınır | Açıklama |
|---|---|
| ARC | malloc var, refcount/free yok |
| Lambda closure | Parametre bağlama var, dış değişken yok |
| Exception | Try body çalışıyor, catch propagation yok |
| async/await | Parse+AST tamam, state machine yok |
| SIMD | Tip sistemi var, IR yok |
| Multi-file | Tek dosya derleniyor |
| Stdlib | Yalnızca stub implementasyonlar |
| Generic instantiation | Yüzeysel doğrulama |

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
class              → heap, reference semantics (malloc)
struct             → stack, value semantics, copy
@ManualMemory class → heap, manual memory, zero GC overhead
```

### CodeGen Mimarisi
- **LLVM 21 / inkwell 0.9** — doğrudan IR üretimi
- **gcc** sadece linker (`.o → .exe`)
- `.o` dosyası `%TEMP%` klasöründe, kullanıcıya görünmez
- `arc hello.arm` → `hello.exe` (tek komut)
