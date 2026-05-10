# Arimo Lang — Yol Haritası

> Proje detayları: `arimo-lang-task-list.md`

---

## Hedef

Modern bir programlama dili:
- **OS yazılabilir** — bare-metal, inline asm, donanım erişimi
- **Game engine yazılabilir** — sıfır GC overhead, struct, SIMD, operator overloading
- **Uygulama yazılabilir** — otomatik bellek, async/await, yüksek seviye OOP

Nihai hedef: Arimo compiler'ını (arc) Arimo'nun kendisiyle yeniden yazmak.

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
### 3.2 @Likely / @Unlikely ✅
### 3.3 Interface default metodlar ✅
### 3.4 async / await ✅ (parse + AST)
### 3.5 Annotation Sistemi ✅ (16 annotation)

---

## FAZA 4 — CodeGen (LLVM / inkwell) 🚧

**Kurulum:**
- LLVM 21.1.8 (MSYS2 MinGW): `C:\msys64\mingw64`
- inkwell 0.9.0 — `llvm21-1` feature
- Target: `x86_64-pc-windows-gnu`

### 4.1–4.11 ✅ TAMAMLANDI
Temel altyapı, operatörler, kontrol akışı, fonksiyon/metod üretimi, class instances, enum codegen, inheritance, static fields, stdlib stubs, collections runtime (List/HashMap/Pair).

### 4.12–4.18 ✅ TAMAMLANDI (2026-05-10)
Break/Continue, string metodları (14 metod), string concat, IO.println, lambda genel codegen, match expression, asm{} codegen, @Packed struct, LLVM fonksiyon attribute'ları (@ForceInline/@Pure/@Section/@CallingConvention), output flag'leri (-O2/-c).

### 4.19 Kalan CodeGen Eksikleri ⬜

#### 4.19.1 Bellek Yönetimi — ARC
- [ ] Class struct'a `refcount` field'ı ekle (i64)
- [ ] Constructor'da `refcount = 1` init
- [ ] Nesne paylaşılınca `refcount++`
- [ ] `refcount--` + `if refcount == 0 { free() }` — scope çıkışında
- [ ] `@ManualMemory` class'larında otomatik free atla
- [ ] BorrowChecker `drop_schedule`'ı kullanarak scope sırasını belirle
- [ ] Weak referanslar için `weak_count` field'ı (gelecek)

#### 4.19.2 Exception Handling — Gerçek Implementasyon
- [ ] `throw ExceptionType(msg)` → heap'te exception nesnesi oluştur + LLVM `resume`
- [ ] `try/catch` → LLVM `invoke` + `landingpad` + `personality` fonksiyonu
- [ ] Birden fazla `catch` tipi → tip karşılaştırması (RTTI benzeri)
- [ ] `finally` bloğu → her çıkış yolunda (return, throw) çalışması garanti
- [ ] `Exception.message()` → heap'teki mesaj string'ini döndür
- [ ] Exception kalıtımı → `instanceof` kontrolü (parent type catch)

#### 4.19.3 Lambda — Closure Capture
- [ ] Dış scope değişkenlerini tespit et (free variable analysis)
- [ ] Capture list'i heap'te struct olarak paketle
- [ ] Lambda fonksiyonuna closure ptr'ı ekstra parametre olarak geç
- [ ] `[x, y] -> x + y` syntax ile explicit capture (gelecek)

#### 4.19.4 Eksik Expression Codegen
- [ ] `Expr::NullSafeAccess { object, field, args }` → null check + conditional dispatch
- [ ] `Expr::Super` → parent class field/metod erişimi (constructor dışında)
- [ ] `Stmt::Defer` → scope sonunda çalışacak ifade listesi (LIFO sırasıyla)

#### 4.19.5 Systems CodeGen Tamamlama
- [ ] `volatile` load/store → LLVM `volatile` flag
- [ ] `noreturn` fonksiyon → LLVM `noreturn` attribute + `unreachable` terminatör
- [ ] `@Likely/@Unlikely` → LLVM `branch_weights` metadata
- [ ] `@Align(N)` → alloca ve global'lere alignment attribute
- [ ] SIMD tipleri → LLVM `<4 x float>` vb. vector types
  - Vec4f/Vec8f/Vec4i/Vec8i için aritmetik operatörler
  - `length()`, `normalize()`, `dot(other)` metodları

#### 4.19.6 Collections Tamamlama
- [ ] `List.sortedBy(comparator)` → C `qsort` entegrasyonu
- [ ] `List.reduce(init, fn)` → fold işlemi
- [ ] `List.map(fn)` → dönüştürme, yeni liste döndür
- [ ] `List.flatMap(fn)` → nested list düzleştirme
- [ ] `List.any(fn)` / `List.all(fn)` → boolean aggregate
- [ ] `List.distinct()` → tekrar edenleri çıkar
- [ ] `HashMap.entries()` → `List<Pair<K,V>>`
- [ ] `HashMap.remove(key)` → slot temizle
- [ ] `HashMap.containsKey(key)` → gerçek implementasyon (şu an stub)
- [ ] `HashMap.length()` → kayıtlı eleman sayısı

#### 4.19.7 String Tamamlama
- [ ] `str.substring(start, end)` → malloc + memcpy
- [ ] `str.replace(old, new)` → strstr + malloc + concat
- [ ] Enum değerinin string'e dönüşümü: `${enumVal}` → label fonksiyonu

#### 4.19.8 async/await — State Machine
- [ ] Async fonksiyon → LLVM coroutine (`llvm.coro.*` intrinsics)
- [ ] `await expr` → suspend point
- [ ] Basit polling tabanlı executor (Faza 9'a bağımlı)

---

## FAZA 5 — Stdlib ⬜

Stdlib, Arimo kaynak dosyaları (`.arm`) olarak yazılır ve arc ile derlenir.

### 5.1 arimo.io — Giriş/Çıkış
- [ ] `IO.print(msg)` → terminal çıktısı (şu an çalışıyor, stdlib'e taşınacak)
- [ ] `IO.println(msg)` → newline ekleyerek yaz (codegen var, stdlib'e taşınacak)
- [ ] `IO.read()` → stdin'den satır oku
- [ ] `IO.readInt()` → stdin'den Integer oku
- [ ] `IO.readFloat()` → stdin'den Float oku
- [ ] `IO.error(msg)` → stderr'e yaz

### 5.2 arimo.fs — Dosya Sistemi ← **Bootstrap için öncelikli**
- [ ] `File.open(path, mode)` → dosya handle'ı aç
- [ ] `File.read(path)` → tüm içeriği String olarak oku
- [ ] `File.readLines(path)` → `List<String>`
- [ ] `File.write(path, content)` → dosyaya yaz
- [ ] `File.append(path, content)` → dosyaya ekle
- [ ] `File.exists(path)` → Boolean
- [ ] `File.delete(path)` → dosyayı sil
- [ ] `File.size(path)` → Integer (byte)
- [ ] `Directory.list(path)` → `List<String>` dosya/klasör adları
- [ ] `Directory.create(path)` → klasör oluştur
- [ ] `Directory.exists(path)` → Boolean
- [ ] `Path.join(parts...)` → yol birleştir
- [ ] `Path.extension(path)` → uzantı al
- [ ] `Path.basename(path)` → dosya adı
- [ ] `Path.dirname(path)` → klasör adı

### 5.3 arimo.time — Tarih/Saat
- [ ] `Time.now()` → gerçek zaman damgası
- [ ] `Time.nowMillis()` → Unix millisaniye
- [ ] `Time.nowNanos()` → yüksek çözünürlüklü zaman
- [ ] `Time.generateId()` → gerçek UUID (time-based)
- [ ] `Time.format(timestamp, pattern)` → tarih formatlama
- [ ] `Time.parse(str, pattern)` → string → timestamp
- [ ] `Time.sleep(ms)` → bekle

### 5.4 arimo.collections — Gelişmiş Koleksiyonlar
- [ ] `List.sortedBy(fn)` → sıralı liste
- [ ] `List.take(n)` / `List.drop(n)`
- [ ] `List.takeLast(n)` / `List.dropLast(n)`
- [ ] `List.reduce(init, fn)` → fold
- [ ] `List.map(fn)` → dönüştürme
- [ ] `List.flatMap(fn)` → nested düzleştirme
- [ ] `List.any(fn)` / `List.all(fn)`
- [ ] `List.distinct()` → tekrarsız
- [ ] `List.zip(other)` → `List<Pair<A,B>>`
- [ ] `List.chunked(n)` → `List<List<T>>`
- [ ] `List.reversed()` → tersine sıralı
- [ ] `List.joinToString(sep)` → String
- [ ] `HashMap.entries()` → `List<Pair<K,V>>`
- [ ] `HashMap.keys()` → `List<K>`
- [ ] `HashMap.values()` → `List<V>`
- [ ] `HashMap.remove(key)` → sil
- [ ] `HashMap.containsKey(key)` → Boolean
- [ ] `HashMap.forEach((k,v) -> ...)` → iterasyon
- [ ] `TreeMap<K,V>` → sıralı map implementasyonu
- [ ] `Set<T>` → benzersiz eleman koleksiyonu
- [ ] `Queue<T>` → FIFO kuyruk
- [ ] `Stack<T>` → LIFO yığın
- [ ] `LinkedList<T>` → çift bağlı liste

### 5.5 arimo.math — Matematik
- [ ] `Math.min(a, b)` / `Math.max(a, b)`
- [ ] `Math.floor(f)` / `Math.ceil(f)` / `Math.round(f)`
- [ ] `Math.log(x)` / `Math.log2(x)` / `Math.log10(x)`
- [ ] `Math.sin(x)` / `Math.cos(x)` / `Math.tan(x)`
- [ ] `Math.asin(x)` / `Math.acos(x)` / `Math.atan2(y, x)`
- [ ] `Math.random()` → 0.0–1.0 arası Float
- [ ] `Math.randomInt(min, max)` → Integer
- [ ] `Integer.MAX` / `Integer.MIN` sabitleri
- [ ] `Float.INFINITY` / `Float.NAN` sabitleri
- [ ] `Float.isNaN(f)` / `Float.isInfinite(f)`

### 5.6 arimo.string — String Yardımcıları
- [ ] `String.format(template, args...)` → sprintf benzeri
- [ ] `String.parseInt(s)` → Integer veya throws
- [ ] `String.parseFloat(s)` → Float veya throws
- [ ] `Integer.toString(n)` → String
- [ ] `Float.toString(f)` → String
- [ ] `Boolean.toString(b)` → "true" / "false"
- [ ] `String.repeat(n)` → tekrarlı string
- [ ] `String.padStart(len, char)` → sola pad
- [ ] `String.padEnd(len, char)` → sağa pad
- [ ] `String.isEmpty()` → Boolean
- [ ] `String.isBlank()` → boşluk kontrolü
- [ ] `String.chars()` → `List<Char>` (Char tipi sonraki fazda)

### 5.7 arimo.env — Ortam
- [ ] `Env.get(key)` → String? (nullable)
- [ ] `Env.set(key, value)` → Void
- [ ] `Env.args()` → `List<String>` (komut satırı argümanları)
- [ ] `Env.exit(code)` → noreturn
- [ ] `Env.platform()` → "windows" / "linux" / "macos"

### 5.8 arimo.process — Süreç
- [ ] `Process.run(cmd, args)` → çıktı + exit code
- [ ] `Process.spawn(cmd, args)` → arka planda çalıştır
- [ ] `Process.pid()` → mevcut process ID

### 5.9 arimo.net — Ağ *(ileri aşama)*
- [ ] `HttpClient.get(url)` → String response
- [ ] `HttpClient.post(url, body)` → String response
- [ ] `TcpSocket.connect(host, port)` → bağlantı aç
- [ ] `TcpSocket.read()` → String
- [ ] `TcpSocket.write(data)` → Void
- [ ] `TcpSocket.close()` → Void
- [ ] `UdpSocket` → temel UDP desteği

### 5.10 arimo.sync — Eşzamanlılık *(Faza 9'a bağımlı)*
- [ ] `Mutex<T>` → karşılıklı dışlama kilidi
- [ ] `RwLock<T>` → okuma/yazma kilidi
- [ ] `Channel<T>` → mesaj iletimi (mpsc)
- [ ] `Atomic<T>` → atomik okuma/yazma
- [ ] `Barrier` → senkronizasyon noktası

---

## FAZA 6 — Dil Genişletmeleri v2.0 ⬜

Arimo v1.4 üzerine eklenen yeni dil özellikleri.

### 6.1 Null Coalescing `??`
```arimo
String name = user.getName() ?? "Anonim";
Integer port = config.port ?? 8080;
```

### 6.2 `is` Type Check + `as?` Safe Cast
```arimo
if (shape is Circle) { ... }
Circle? c = shape as? Circle;  // null döner, throw değil
```

### 6.3 `?` Error Propagation Operatörü
```arimo
// Result<T,E> döndüren metodlarda ? kullanılabilir
Integer n = parseInt(input)?;  // Err ise dışarı fırlat
```

### 6.4 Default Parametre Değerleri
```arimo
method greet(String name, String prefix = "Merhaba") : String {
    return "${prefix}, ${name}!";
}
greet("Ali");            // "Merhaba, Ali!"
greet("Ali", "Selam");  // "Selam, Ali!"
```

### 6.5 Named Parameters
```arimo
method connect(String host, Integer port = 80, Boolean ssl = false) : Connection { ... }
connect(host: "example.com", ssl: true);  // port varsayılan
```

### 6.6 Destructuring
```arimo
Pair<Integer, String> p = Pair(42, "merhaba");
Integer (n, s) = p;  // n=42, s="merhaba"

// List destructuring
List<Integer> nums = [1, 2, 3, 4, 5];
Integer [first, second, ...rest] = nums;
```

### 6.7 `when` Expression (Gelişmiş Match)
```arimo
String result = when (status) {
    TaskStatus.PENDING  -> "Bekliyor"
    TaskStatus.DONE     -> "Tamamlandı"
    else                -> "Bilinmiyor"
};

// Tip kontrolü ile
when (obj) {
    is Circle c    -> IO.println("Çap: ${c.radius}")
    is Rectangle r -> IO.println("Alan: ${r.w * r.h}")
    else           -> IO.println("Bilinmeyen şekil")
}
```

### 6.8 Match Guard (Koşullu Pattern)
```arimo
match n {
    x if x < 0  => IO.println("Negatif")
    x if x == 0 => IO.println("Sıfır")
    x           => IO.println("Pozitif: ${x}")
}
```

### 6.9 String Pattern Matching
```arimo
match command {
    "quit" | "exit" => Env.exit(0)
    "help"          => printHelp()
    _               => IO.println("Bilinmeyen komut")
}
```

### 6.10 Range Type + Range Patterns
```arimo
Range<Integer> r = 1..=100;
for n in r { IO.println(n); }

match score {
    90..=100 => IO.println("A")
    80..=89  => IO.println("B")
    _        => IO.println("F")
}
```

### 6.11 Extension Methods
```arimo
extend Integer {
    method isEven() : Boolean { return this % 2 == 0; }
    method squared() : Integer { return this * this; }
}

IO.println(42.isEven());   // true
IO.println(5.squared());   // 25
```

### 6.12 Char Tipi
```arimo
Char c = 'A';
Integer code = c.code();     // 65
String s = c.toString();     // "A"
Boolean isDigit = c.isDigit();
Boolean isAlpha = c.isAlpha();
```

### 6.13 Enum Iteration
```arimo
for status in TaskStatus.values() {
    IO.println(status);  // otomatik toString
}
Integer count = TaskStatus.count();  // variant sayısı
```

### 6.14 Object Copy Expression
```arimo
Task updated = original.copy(
    status: TaskStatus.DONE,
    priority: 1
);
```

### 6.15 `@Test` + `@Benchmark` Annotation
```arimo
@Test
public static testAddition() : Void {
    Integer result = add(2, 3);
    assert(result == 5, "2 + 3 should be 5");
}

@Benchmark(iterations: 1000)
public static benchmarkSort() : Void { ... }
```

### 6.16 Const Expressions
```arimo
const Integer MAX_SIZE = 1024;
const Float PI = 3.14159265358979;
const String VERSION = "1.0.0";
// Compile-time sabit — static readonly'den farklı: değer derleme zamanında hesaplanır
```

### 6.17 Multiple Exception Catch
```arimo
try {
    risky();
} catch (IOException | NetworkException e) {
    IO.println(e.message());
} catch (Exception e) {
    IO.println("Genel hata");
}
```

### 6.18 String Template Fonksiyonları
```arimo
// Multiline string
String sql = """
    SELECT *
    FROM users
    WHERE id = ${userId}
""";

// Raw string (escape yok)
String path = r"C:\Users\Arimo\Documents";
```

---

## FAZA 7 — Araçlar ⬜

### 7.1 Gelişmiş Hata Mesajları
- [ ] Her hata için satır + sütun numarası
- [ ] Hata bağlamı (ilgili kod satırı + `^^^` işaretçi)
- [ ] "did you mean?" önerisi (benzer isim bulma)
- [ ] Renklendirmeli terminal çıktısı
- [ ] Hata kodu sistemi (E001, E002, ...)

### 7.2 Uyarı Sistemi
- [ ] Kullanılmayan değişken uyarısı
- [ ] Erişilemeyen kod uyarısı (dead code)
- [ ] Kullanılmayan `import` uyarısı
- [ ] Gölgelenen değişken uyarısı
- [ ] `--no-warnings` flag

### 7.3 Debug Bilgisi (DWARF)
- [ ] `--debug` flag → DWARF debug info üret
- [ ] Satır numarası eşlemesi (source map)
- [ ] Değişken adları binary'de saklansın
- [ ] GDB/LLDB ile debug edilebilsin

### 7.4 Cross-Compilation
- [ ] `--target linux-x64` → `x86_64-unknown-linux-gnu`
- [ ] `--target macos-arm64` → `aarch64-apple-darwin`
- [ ] `--target linux-arm64` → `aarch64-unknown-linux-gnu`
- [ ] `--target wasm32` → WebAssembly (gelecek)

### 7.5 arc.toml — Proje Manifestosu
```toml
[project]
name = "myapp"
version = "1.0.0"
entry = "src/Main.arm"

[dependencies]
arimo-collections = "1.0"

[build]
optimize = true
target = "x86_64-pc-windows-gnu"
```

### 7.6 arc CLI Komutları
- [ ] `arc build` → projeyi derle (arc.toml'a göre)
- [ ] `arc run` → derle + çalıştır
- [ ] `arc test` → @Test metotlarını çalıştır
- [ ] `arc clean` → build çıktılarını sil
- [ ] `arc check` → sadece tip kontrolü (binary üretme)
- [ ] `arc fmt` → kodu formatla
- [ ] `arc init <name>` → yeni proje oluştur

### 7.7 Multi-File Compilation
- [ ] Birden fazla `.arm` dosyasını tek binary'e derle
- [ ] `import` ifadelerini dosya bağımlılığı olarak çözümle
- [ ] Döngüsel import tespiti + hata

### 7.8 İnkremental Derleme
- [ ] Değişen dosyaları tespit et
- [ ] Sadece etkilenen dosyaları yeniden derle
- [ ] `.arc-cache/` klasöründe ara sonuçları sakla

---

## FAZA 8 — Araç Ekosistemi ⬜

### 8.1 VSCode Extension
- [ ] `.arm` dosyaları için syntax highlighting
- [ ] Anahtar kelime, tip, annotation renklendirmesi
- [ ] Temel kod parçacıkları (snippets)
- [ ] Dosya simgesi

### 8.2 Language Server Protocol (LSP)
- [ ] Otomatik tamamlama (completions)
- [ ] Hata ve uyarı gösterimi (diagnostics)
- [ ] Tanıma git (go-to-definition)
- [ ] Referansları bul (find references)
- [ ] Hover bilgisi (tip + dokümantasyon)
- [ ] Yeniden adlandırma (rename symbol)
- [ ] Kod aksiyonları (quick fixes)

### 8.3 arc fmt — Formatter
- [ ] Girinti standardizasyonu (4 boşluk)
- [ ] Operatör etrafında boşluk
- [ ] Blok açma/kapama kuralları
- [ ] Maksimum satır uzunluğu (120 karakter)
- [ ] Import sıralama

### 8.4 arc doc — Dokümantasyon Üreticisi
- [ ] `/** */` yorum bloklarından HTML/Markdown üret
- [ ] Class, metod, field dokümantasyonu
- [ ] `@param`, `@return`, `@throws` tag'leri
- [ ] Arama desteği

### 8.5 arc pkg — Paket Yöneticisi
- [ ] Paket yayınlama / indirme
- [ ] Versiyon çözümleme (semver)
- [ ] `arc.lock` — bağımlılık kilitleme
- [ ] Merkezi paket deposu (arimo-packages)

---

## FAZA 9 — Runtime & Concurrency ⬜

### 9.1 Thread Desteği
- [ ] `Thread.spawn(() -> { ... })` → yeni thread
- [ ] `Thread.join()` → thread tamamlanmasını bekle
- [ ] `Thread.sleep(ms)` → bekle
- [ ] `Thread.current().id()` → thread kimliği
- [ ] Platform: pthreads (Linux/macOS) + Windows Threads

### 9.2 Eşzamanlı Veri Yapıları
- [ ] `Mutex<T>` → `lock()` / `unlock()` / `withLock(() -> ...)`
- [ ] `RwLock<T>` → çoklu okuyucu, tekil yazıcı
- [ ] `Atomic<Integer>` → atomik sayaç
- [ ] `Channel<T>` → bounded/unbounded mesaj kanalı
- [ ] `Barrier(n)` → n thread'in buluşma noktası

### 9.3 Async Runtime
- [ ] Event loop (poll tabanlı)
- [ ] `Task<T>` → async hesaplama birimi
- [ ] `Task.run(asyncFn)` → event loop'a gönder
- [ ] `Task.await(task)` → tamamlanmasını bekle
- [ ] I/O multiplexing (epoll/IOCP)

### 9.4 Signal Handling
- [ ] `Signal.handle(SIGTERM, () -> { ... })` → graceful shutdown
- [ ] `Signal.handle(SIGINT, handler)` → Ctrl+C
- [ ] `Signal.ignore(SIGPIPE)` → broken pipe

### 9.5 Panic Handler + Stack Traces
- [ ] `panic(msg)` → programı sonlandır, mesaj yaz
- [ ] Stack trace yazdırma (debug modda)
- [ ] `@catchPanic` → panic'i yakalama mekanizması
- [ ] Out-of-bounds, null-deref → otomatik panic

### 9.6 Thread-Local Storage
- [ ] `@ThreadLocal` annotation → thread başına ayrı değişken
- [ ] `ThreadLocal<T>` tip

---

## FAZA 10 — Bootstrapping ⬜

Bootstrapping: arc compiler'ını Arimo diliyle yeniden yazmak.

### Ön Koşullar (Faza 10 başlamadan önce tamamlanmalı)

| Bileşen | Neden Gerekli |
|---|---|
| String metodları (4.19.7) | Lexer karakter/string işleme yapar |
| Exception handling (4.19.2) | Parse hataları, tip hataları fırlatılır |
| Lambda closure (4.19.3) | Derleyici içi yüksek seviye işlemler |
| arimo.fs (5.2) | Kaynak `.arm` dosyası okunur |
| arimo.io (5.1) | Hata ve bilgi mesajları |
| arimo.collections (5.4) | Token listesi, AST node'ları, scope tabloları |
| ARC bellek yönetimi (4.19.1) | Derleyici uzun süre çalışır, bellek sızmamalı |

### Stage 0 — Rust Derleyici (arc-rust)
- Mevcut `arc` (Rust ile yazılmış)
- Bu repo: `arimo-compiler` — tarihsel referans olarak saklanır

### Stage 1 — Arimo Derleyici İlk Derleme
```
arc-arimo/
├── src/
│   ├── Lexer.arm
│   ├── Parser.arm
│   ├── TypeChecker.arm
│   ├── BorrowChecker.arm
│   ├── CodeGen.arm     — LLVM IR metin çıktısı (inkwell gerekmez)
│   └── Main.arm
```
- Stage 0 ile derlenir
- LLVM IR metin olarak üretir → `llc` + `gcc` ile binary

### Stage 2 — Kendini Derleyen Compiler
- Stage 1'deki arc-arimo, kendisiyle derlenir
- Aynı binary çıkıyorsa bootstrapping tamamdır
- Artık Rust bağımlılığı yok

---

## Önemli Notlar

### Commit Kuralı
- Co-Authored-By veya benzeri otomatik imza ekleme
- Merge commit bırakma — cherry-pick kullan

### Derleme
```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"
cargo build --target x86_64-pc-windows-gnu
.\target\x86_64-pc-windows-gnu\debug\arc.exe src.arm [-O2] [-c] [--emit-ir]
```
