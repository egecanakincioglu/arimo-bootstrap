# Arimo Lang — Genel Mimari

## Dil Felsefesi

Arimo tek dilde üç hedefi bir araya getirir:
- **OS seviyesi**: inline asm, volatile, @Freestanding, zero overhead
- **Oyun motoru**: SIMD, struct value types, operator overloading, sıfır GC
- **Uygulama**: ARC bellek, OOP, async/await, yüksek seviye koleksiyonlar

---

## İki Temel Katman

### Katman 1 — Sistem / Performans (Compiler Primitive)

**Değiştirilemez, import edilmez, her zaman orada.**

Compiler'ın Rust kodu içinde tanımlı. LLVM'e direkt map edilir.
Hiçbir zaman .arm dosyası olmaz. Zero-cost abstraction.

```
Tipler:    Integer, Float, Boolean, String, Char, Void, NoReturn
Boyutlu:   u8..u64, i8..i64
Bileşik:   Array<T,N>, Slice<T>, RawPtr<T>, FnPtr(..→T)
Operatör:  +,-,*,/,%, ==,!=,<,>,<=,>=, &&,||,!, &,|,^,~,<<,>>, as
Bellek:    ARC (retain/release), BorrowChecker, @ManualMemory
```

Bu katman C ve Rust ile aynı güçte — kernel yazılabilir.

### Katman 2 — Modern OOP / Uygulama (.arm dosyaları)

**Arimo ile yazılmış, arc ile derlenmiş stdlib.**

```
arimo.lang    → Object, String, Integer, Math, IO, Exception hiyerarşisi (AUTO-IMPORT)
arimo.util    → ArrayList, HashMap, TreeMap, Optional, Scanner, UUID
arimo.fs      → File, Directory, Path, FileStream
arimo.io      → InputStream, OutputStream, Reader, Writer, BufferedReader
arimo.time    → Instant, Duration, LocalDate, ZonedDateTime
arimo.math    → Math (genişletilmiş), Complex, Matrix, Vector
arimo.text    → Regex, Formatter, Charset
arimo.net     → HttpClient, TcpSocket, WebSocket
arimo.security→ Hash, Hmac, AES
arimo.sys     → Syscall, Allocator, Process, Platform (ileri faz)
arimo.util.concurrent → Mutex, Channel, Future, ThreadPool (Faza 9)
```

Bu katman Java ve Kotlin ile aynı rahat — uygulama yazılabilir.

---

## AVM — Arimo Virtual Machine

JVM'e benzer konsept, farklı implementasyon:

```
Java:   bytecode → JVM (C++) → OS
Arimo:  native binary + AVM runtime (.arm) → OS syscalls
```

AVM native çalışır, yorumlayıcı yoktur. JVM'den farkı:
- C++ değil, Arimo ile yazılmış
- Bytecode değil, LLVM native kod
- Her binary'e statik bağlantı (veya dinamik paylaşımlı)
- libc bağımlılığı yok — kendi syscall katmanı

### AVM Bileşenleri
```
1. Bellek Yöneticisi   → mmap/VirtualAlloc tabanlı, kendi heap
2. ARC Runtime         → retain/release/scope cleanup
3. Exception Runtime   → stack unwinding, type matching
4. I/O Runtime         → buffered I/O, async I/O
5. Thread Yöneticisi   → platform thread wrap (Faza 9)
6. Platform Katmanı    → Linux/Windows/macOS/bare-metal syscall
```

---

## Derleme Pipeline

### Mevcut (Faza 4 sonu)
```
.arm → Lexer → Parser → TypeChecker → BorrowChecker → CodeGen → .o → .exe
  ✅      ✅       ✅          ✅              ✅           ✅
```

### Hedef (Faza 7 sonrası)
```
arc.toml (proje)
    ↓
Birden fazla .arm dosyası → bağımlılık grafiği → topological sort
    ↓
Her dosya: Lexer → Parser → TypeChecker → BorrowChecker → CodeGen → .o
    ↓
Tüm .o + stdlib.a + avm.a → linker → .exe
```

### Bootstrapping (Faza 10)
```
Stage 0: arc (Rust)     → mevcut compiler
Stage 1: arc-arimo.arm  → [Stage 0] → arc-arimo.exe  (Arimo ile yazılmış compiler)
Stage 2: arc-arimo.arm  → [Stage 1] → arc-arimo2.exe (aynı binary = self-hosting ✓)
```

---

## Modül Sistemi

### Hiyerarşi
```
Module (üst düzey gruplandırma):  arimo.base
  └── Package (dosya bildirimi):  arimo.io, arimo.util, ...
        └── Class/Interface/Enum: File, ArrayList, Comparable, ...
```

### Sözdizimi
```arimo
package arimo.io;          // dosya bildirimi (module → package değişti)
import arimo.fs.File;      // tekil sınıf import
import arimo.util.*;       // wildcard import

module arimo.base {        // üst düzey modül tanımı
    exports arimo.lang;    // auto-import
    exports arimo.io;
}
```

### Auto-Import Kuralı
`arimo.lang` içindeki her şey tüm dosyalara otomatik gelir.
Kullanıcı `import arimo.lang.String` yazmaz — zaten orada.

---

## Bellek Yönetimi — 3 Katmanlı Hibrit

```
1. BorrowChecker   → compile-time, zero runtime overhead
                     (scope analizi, use-after-free tespiti)

2. ARC             → refcount++/-- runtime (stack ve scope bazlı)
                     retain: değişkene atama (Ident)
                     release: scope çıkışı, return öncesi
                     @ManualMemory ile tamamen atlanır

3. GC              → döngüsel referans temizliği (ilerisi)
```

---

## Compiler Durumu (Tamamlanan)

### Dil Özellikleri
```
✅ Fixed-size integers (u8..u64, i8..i64)
✅ Bitwise operatörler, as cast
✅ Type alias
✅ struct (stack, copy semantics, @Packed)
✅ Operator overloading, @ForceInline
✅ Array<T,N>, Slice<T>, FnPtr
✅ Generic bounds (<T: Interface>)
✅ Enum with data, match expression
✅ Result<T,E>
✅ volatile, union, extern "C" + variadic
✅ asm {} inline assembly
✅ @Freestanding, @Section, @CallingConvention
✅ noreturn, defer
✅ SIMD tipleri (Vec4f, Vec8f, Vec4i, Vec8i)
✅ @Likely/@Unlikely (branch weights)
✅ Interface default methods
✅ async/await (parse + AST; state machine sonraki faz)
✅ 16 annotation (@ManualMemory, @Sealed, @Immutable, @Deprecated...)
```

### CodeGen (LLVM)
```
✅ Tüm operatörler ve kontrol akışı
✅ Fonksiyon/metod üretimi
✅ Class instances, constructor, field erişimi
✅ Inheritance, super(), virtual dispatch
✅ Static fields (global)
✅ Enum codegen + metodlar
✅ Collections runtime (List, HashMap, Pair — pure LLVM IR)
✅ Lambda → fn pointer + closure capture (global)
✅ Match expression (enum pattern)
✅ Break/Continue (loop stack)
✅ String metodları (14 metod)
✅ Defer (LIFO, scope entegrasyonu)
✅ NullSafeAccess (phi node)
✅ Exception (finally guarantee, catch derleme, throw→abort)
✅ ARC tam implementasyon (retain/release/field-store ARC)
✅ Systems: volatile, noreturn, @ForceInline/@Pure, @Section
✅ Output: -O2, -c, --emit-ir
✅ package keyword, wildcard import
```

---

## Bilinen Sınırlar (Sonraki Fazlara Bırakılan)

| Sınır | Açıklama | Faz |
|---|---|---|
| async/await state machine | Şu an sync gibi çalışır | 9 |
| Exception catch propagation | catch body çalışmıyor (setjmp gerekli) | 9 |
| SIMD aritmetik | Tip sistemi var, IR yok | 6 |
| Multi-file compilation | Tek dosya derleniyor | 7 |
| Stdlib | Yok — stub'lar var | 5 |
| AVM | Tasarlandı, yazılmadı | 11 |
| libc bağımsızlığı | Syscall katmanı yazılmadı | 11 |
| Field load retain | `b = a.field` → b dangling olabilir | 6 |
