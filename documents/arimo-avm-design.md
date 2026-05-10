# Arimo Virtual Machine (AVM) — Tasarım Belgesi

## Genel Vizyon

Arimo, native kod üretir — JVM gibi bir bytecode yorumlayıcısı değildir.
Ama JVM'den alınan ilham şudur: **tam bağımsızlık**.

AVM = Arimo'nun kendi runtime ortamı.
- libc bağımlılığı yok
- C/C++ bağımlılığı yok
- Arimo'nun kendisiyle yazılmış
- Her platformda kendi syscall katmanı üzerinden çalışır

---

## JVM ile Karşılaştırma

```
Java:   kaynak.java → javac → bytecode.class → JVM (C++) → OS
Arimo:  kaynak.arm  → arc   → native binary + AVM runtime → OS
```

Fark: Arimo native derleme yapar — yorumlayıcı yok, JIT yok.
AVM bir "sanal makine" değil, Arimo'nun **runtime kütüphanesi**.
JVM C++ ile yazılmıştır. AVM Arimo ile yazılacaktır.

---

## Mimari Katmanlar

```
┌─────────────────────────────────┐
│     Kullanıcı Kodu (.arm)       │  ← package myapp;
├─────────────────────────────────┤
│     Katman 2 — Stdlib (.arm)    │  ← arimo.util, arimo.fs, arimo.io...
├─────────────────────────────────┤
│     AVM Runtime (.arm)          │  ← bellek, thread, I/O, exception
├─────────────────────────────────┤
│     Katman 1 — Primitifler      │  ← Integer, Float, operatörler (compiler)
├─────────────────────────────────┤
│     Platform Syscall Katmanı    │  ← Linux/Windows/macOS/bare-metal
└─────────────────────────────────┘
```

---

## Katman 1 — Compiler Primitive'leri (değiştirilemez)

Bunlar compiler'ın içinde tanımlı. LLVM'e direkt map edilir.
Hiçbir zaman .arm dosyası olmaz.

### Temel Tipler
```
Integer   → LLVM i64
Float     → LLVM f64
Boolean   → LLVM i1
String    → LLVM ptr (i8*)
Char      → LLVM i8
Void      → LLVM void
NoReturn  → LLVM noreturn attribute
u8..u64   → LLVM iN (unsigned)
i8..i64   → LLVM iN (signed)
```

### Bileşik Tipler
```
Array<T, N>  → LLVM [N x T]      (compile-time boyutlu, stack)
Slice<T>     → LLVM {ptr, i64}   (fat pointer: veri + uzunluk)
RawPtr<T>    → LLVM ptr          (ham pointer, @ManualMemory ile)
FnPtr(..→T)  → LLVM function ptr
```

### Operatörler
```
+  -  *  /  %            → aritmetik
== != < > <= >=          → karşılaştırma
&& || !                  → mantıksal
&  |  ^  ~  << >>        → bitwise
as                       → cast
=  +=  -=  *=  /=        → atama
```

### Bellek Modeli (compiler-generated)
```
ARC:    refcount++  →  arc_retain_ptr()
        refcount--  →  arc_release_var()
        scope exit  →  arc_release_scope()
        return      →  arc_release_all_scopes_except()
@ManualMemory → ARC tamamen atlanır
```

---

## Katman 2 — Stdlib (.arm dosyaları)

Arimo ile yazılmış, arc ile derlenmiş. AVM üstünde çalışır.

### arimo.lang (AUTO-IMPORT — her dosyaya otomatik gelir)
```
Object            → toString(), equals(), hashCode()
StringBuilder     → append(), insert(), delete(), toString()
Integer           → parseInt(), toString(), MAX, MIN, toBinaryString()
Float             → parseFloat(), toString(), isNaN(), INFINITY
Boolean           → parseBoolean(), toString()
Char              → code(), isDigit(), isAlpha(), toUpper(), toLower()
Long / Byte / Short → sayısal yardımcılar
Math              → sqrt, sin, cos, log, random, min, max, floor, ceil
IO                → print(), println(), read(), readLine(), error()
System            → exit(), env(), args(), currentTimeMillis()
Throwable / Exception / RuntimeException / Error
NullPointerException, IllegalArgumentException, IndexOutOfBoundsException
ArithmeticException, NumberFormatException, ClassCastException
Comparable<T>, Iterable<T>, Runnable, Callable<T>, AutoCloseable
```

### arimo.util
```
Collection<T>, List<T>, Set<T>, Map<K,V>, Queue<T>, Deque<T>
ArrayList<T>      → dinamik dizi
LinkedList<T>     → çift bağlı
HashMap<K,V>      → hash tabanlı
LinkedHashMap<K,V>→ sıra koruyan
TreeMap<K,V>      → kırmızı-siyah ağaç
HashSet<T>        → tekrarsız
TreeSet<T>        → sıralı tekrarsız
ArrayDeque<T>     → iki uçlu kuyruk
PriorityQueue<T>  → öncelikli kuyruk
Optional<T>       → null-safe sarmalayıcı
Collections       → sort, reverse, min, max, unmodifiable...
Arrays            → sort, fill, copyOf, binarySearch
Objects           → requireNonNull, equals, hash, toString
Scanner           → metin/stdin okuma
Random            → nextInt, nextFloat, nextBoolean
UUID              → random(), fromString()
Base64            → encode(), decode()
StringJoiner      → sınırlayıcılı birleştirme
```

### arimo.util.concurrent (Faza 9)
```
Mutex, RwLock, Atomic<T>, Channel<T>
Future<T>, CountDownLatch, Semaphore, ThreadPool
```

### arimo.fs
```
Path              → of(), join(), parent(), filename(), extension()
File              → read(), readLines(), write(), append(), exists(), delete(), size()
Directory         → list(), create(), createAll(), delete(), exists()
FileStream        → open(), read(), write(), seek(), close()
FileMode (enum)   → READ, WRITE, APPEND, READ_WRITE
```

### arimo.io
```
InputStream / OutputStream (abstract)
FileInputStream / FileOutputStream
BufferedInputStream / BufferedOutputStream
Reader / Writer (abstract)
FileReader / FileWriter
BufferedReader     → readLine()
BufferedWriter     → newLine(), flush()
StringReader / StringWriter
PrintWriter        → print(), println(), printf()
InputStreamReader  → byte→char
OutputStreamWriter → char→byte
```

### arimo.time
```
Instant           → now(), ofEpochMilli(), toEpochMilli()
Duration          → ofSeconds(), ofMillis(), plus(), minus(), between()
LocalDate         → now(), of(y,m,d), plusDays(), minusDays()
LocalTime         → now(), of(h,m,s)
LocalDateTime     → now(), of(date, time)
ZonedDateTime     → now(zone), withZoneSameInstant()
ZoneId            → of("Europe/Istanbul"), systemDefault()
DateTimeFormatter → format(), parse()
```

### arimo.math
```
Math (genişletilmiş) → BigInteger, BigDecimal ilerisi
Complex           → real, imag, abs(), conjugate(), multiply()
Matrix<T>         → multiply(), transpose(), determinant()
Vector2/3/4       → geometri vektörleri (SIMD ile)
```

### arimo.text
```
Regex             → compile(), match(), find(), replace(), split()
Formatter         → format(template, args...) — printf tarzı
Charset           → UTF_8, UTF_16, ASCII, ISO_8859_1
Encoder           → encode(str)→bytes, decode(bytes)→str
```

### arimo.net
```
InetAddress       → getByName(), getLocalHost()
URL / URI         → parse, build, resolve
TcpSocket         → connect(), read(), write(), close()
UdpSocket         → bind(), send(), receive()
ServerSocket      → bind(), accept()
HttpClient        → get(), post(), put(), delete()
HttpRequest       → builder pattern
HttpResponse<T>   → statusCode(), body(), headers()
WebSocket         → connect(), send(), onMessage()
```

### arimo.security
```
Hash              → sha256(), sha512(), md5()
Hmac              → sign(), verify()
Hex               → encode(), decode()
AES               → encrypt(), decrypt()
```

### arimo.sys (Sistem Bağımsızlığı — Sonraki Faz)
```
Syscall           → write(), read(), mmap(), munmap(), exit()
Allocator         → alloc(), free(), realloc()
Environment       → get(), set(), all()
Process           → run(), spawn(), pid()
Platform          → os(), arch(), pageSize(), cpuCount()
Signal            → handle(), ignore()
```

---

## AVM Runtime Bileşenleri

AVM'in kendisi Arimo ile yazılacak (libc bağımlılığı yok):

### 1. Bellek Yöneticisi
```
- OS'tan büyük bloklar al (mmap / VirtualAlloc)
- Kendi heap implementasyonu (slab allocator veya buddy allocator)
- ARC refcount yönetimi
- GC (döngüsel referanslar için)
```

### 2. Thread Yöneticisi (Faza 9)
```
- Platform thread'lerini wrap et (pthread / Windows Thread)
- Thread pool
- Green thread / coroutine ilerisi
```

### 3. I/O Runtime
```
- Dosya descriptor yönetimi
- Buffered I/O
- Async I/O (epoll / IOCP)
```

### 4. Exception Runtime
```
- Stack unwinding
- Exception type matching
- Finally guarantee (defer ile şimdilik)
```

### 5. Platform Katmanı
```
Linux:    syscall direkt (write=1, read=0, mmap=9, exit=60)
Windows:  kernel32.dll (WriteConsoleA, VirtualAlloc, ExitProcess)
macOS:    BSD syscall (write=4, read=3, mmap=197)
Bare-metal: sadece asm{} — OS yok
```

---

## AVM'nin Kendisini Derlemesi

```
Faza 5:  stdlib.arm → [Rust arc]   → stdlib.o + avm.o → arimo-runtime.a
Faza 10: arc-arimo  → [Rust arc]   → arc-arimo.exe     (Stage 1)
         arc-arimo  → [arc-arimo]  → arc-arimo2.exe    (Stage 2: self-hosting)
         avm.arm    → [arc-arimo]  → avm.o             (AVM Arimo ile derleniyor)
```

---

## Sözdizimi Değişiklikleri (Tamamlanan)

```arimo
// Eski (hâlâ çalışır — geriye dönük uyumlu)
module arimo.io;

// Yeni (tercih edilen)
package arimo.io;

// Wildcard import
import arimo.util.*;

// Üst düzey modül tanımı
module arimo.base {
    exports arimo.lang;   // auto-import
    exports arimo.io;
    exports arimo.fs;
    exports arimo.util;
}
```

---

## Uygulama Sırası

```
Şu an:
  ✅ Faza 1-4.19: Compiler tam (ARC, exception, closure, SIMD tipler, tüm codegen)
  ✅ Sözdizimi: package, wildcard import, exports
  ✅ Tasarım: stdlib + AVM tasarım belgeleri

Sıradaki:
  [ ] Multi-file compilation (stdlib import için şart)
  [ ] Faza 5.1: arimo.lang core
  [ ] Faza 5.2: arimo.fs
  [ ] Faza 5.3: arimo.io
  [ ] Faza 5.4: arimo.util
  [ ] Faza 5.5: arimo.time
  [ ] Faza 5.6: arimo.math + arimo.text
  [ ] Faza 5.7: arimo.net
  [ ] Faza 5.8: arimo.security
  [ ] Faza 6:   Dil v2.0 (??, is/as?, ?, default param, when, extension...)
  [ ] Faza 7:   Araçlar (hata mesajları, DWARF, arc.toml, CLI, multi-file)
  [ ] Faza 8:   Ekosistem (VSCode, LSP, fmt, doc, pkg)
  [ ] Faza 9:   Concurrency (Thread, Mutex, Channel, async runtime)
  [ ] Faza 10:  Bootstrapping (arc-arimo: compiler Arimo ile yazılıyor)
  [ ] Faza 11:  AVM (runtime Arimo ile yazılıyor, libc bağımlılığı sıfıra iniyor)
```
