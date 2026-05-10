# Arimo Standard Library — Tasarım Belgesi

> Java'nın modül sistemi referans alınarak tasarlanmıştır.
> Java ile farklar: ARC bellek yönetimi, GC yok, native derleme, struct value tipler.

---

## 1. Sözdizimi Değişikliği: module → package

### Mevcut Durum
```arimo
module arimo.io;          // dosya başlığı — aslında "paket bildirimi"
import arimo.shop.model;  // bağımlılık
```

### Yeni Sözdizimi
```arimo
package arimo.io;         // dosya pakette nerede?
import arimo.fs.File;     // tek sınıf import
import arimo.util.*;      // paket wildcard import
```

### `module` Anahtar Kelimesi — Yeni Anlam
`module` artık sadece **üst düzey modül gruplandırması** için kullanılır:
```arimo
// arimo-base.mod dosyası (veya module-info.arm)
module arimo.base {
    exports arimo.lang;       // auto-import: her dosyaya otomatik gelir
    exports arimo.io;
    exports arimo.fs;
    exports arimo.util;
    exports arimo.math;
    exports arimo.time;
    exports arimo.text;
    exports arimo.net;
    exports arimo.concurrent;
    exports arimo.security;
    exports arimo.sys;
}
```

### Auto-Import Kuralı
- `arimo.lang` içindeki her şey **otomatik olarak** her `.arm` dosyasına gelir
- Diğer paketler `import` gerektirir
- Kullanıcı `import arimo.lang.*;` yazmak zorunda değil (Java'nın java.lang'i gibi)

---

## 2. Hiyerarşi

```
arimo.base (module — üst düzey gruplandırma)
│
├── arimo.lang          ← AUTO-IMPORT
├── arimo.lang.annotation
│
├── arimo.io
├── arimo.fs
│
├── arimo.util
├── arimo.util.concurrent
│
├── arimo.math
├── arimo.text
├── arimo.time
│
├── arimo.net
├── arimo.net.http
│
├── arimo.security
├── arimo.sys
└── arimo.reflect       ← ileri faz
```

---

## 3. arimo.lang — Temel Sınıflar (AUTO-IMPORT)

> Java'nın java.lang eşdeğeri. Her dosyaya otomatik gelir.
> Java'dan fark: boxed/unboxed ayrımı yok, ARC-based bellek.

### Interface'ler
| İsim | Açıklama | Java Karşılığı |
|---|---|---|
| `Comparable<T>` | Doğal sıralama: `compareTo(T) : Integer` | java.lang.Comparable |
| `Iterable<T>` | for-each döngüsü desteği | java.lang.Iterable |
| `Runnable` | `run() : Void` — parametresiz işlem birimi | java.lang.Runnable |
| `Callable<T>` | `call() : T` — sonuç döndüren işlem birimi | java.util.concurrent.Callable |
| `AutoCloseable` | `close() : Void` — defer/try-with-resources | java.lang.AutoCloseable |
| `CharSequence` | Okunabilir karakter dizisi soyutlaması | java.lang.CharSequence |

### Sınıflar — Temel Tipler
| İsim | Açıklama | Not |
|---|---|---|
| `Object` | Tüm class'ların kökü: `toString()`, `equals()`, `hashCode()` | Implicit extend |
| `String` | Değişmez karakter dizisi — tüm metodlar | Built-in, lib'e taşınır |
| `StringBuilder` | Değişken string builder: `append()`, `insert()`, `delete()`, `toString()` | Yeni |
| `Integer` | Integer wrapper: `parseInt()`, `toString()`, `MAX`, `MIN`, `toBinaryString()` | Genişletilir |
| `Float` | Float wrapper: `parseFloat()`, `toString()`, `INFINITY`, `NaN`, `isNaN()` | Genişletilir |
| `Boolean` | Boolean wrapper: `parseBoolean()`, `toString()` | Genişletilir |
| `Char` | Karakter tipi: `code()`, `isDigit()`, `isAlpha()`, `toUpper()`, `toLower()` | Yeni tip |
| `Long` | u64/i64 wrapper: `parseLong()`, `MAX`, `MIN` | Yeni |
| `Byte` | u8/i8 wrapper | Yeni |
| `Short` | u16/i16 wrapper | Yeni |
| `Number` | Sayısal wrapper'ların soyut tabanı: `toInteger()`, `toFloat()` | Yeni |

### Sınıflar — Sistem
| İsim | Açıklama | Not |
|---|---|---|
| `System` | `exit(code)`, `env(key)`, `args()`, `currentTimeMillis()`, `nanoTime()` | Java.System benzeri |
| `Math` | `sqrt`, `abs`, `pow`, `sin`, `cos`, `log`, `random`, `min`, `max` | Mevcut stub'dan lib'e |
| `Memory` | `alloc(n)`, `free(ptr)`, `copy(dst, src, n)`, `set(ptr, val, n)` | Mevcut + genişletilir |
| `IO` | `print()`, `println()`, `read()`, `readLine()`, `readInt()`, `error()` | Mevcut stub'dan lib'e |

### Sınıflar — İstisna Hiyerarşisi
```
Throwable
├── Exception                        ← kontrol edilebilir
│   ├── RuntimeException             ← kontrol dışı
│   │   ├── NullPointerException
│   │   ├── IllegalArgumentException
│   │   ├── IllegalStateException
│   │   ├── IndexOutOfBoundsException
│   │   │   ├── ArrayIndexOutOfBoundsException
│   │   │   └── StringIndexOutOfBoundsException
│   │   ├── UnsupportedOperationException
│   │   ├── ArithmeticException      ← sıfıra bölme vb.
│   │   ├── NumberFormatException    ← parseInt başarısız
│   │   ├── ClassCastException       ← as? başarısız
│   │   ├── ConcurrentModificationException
│   │   └── NoSuchElementException
│   └── IOException                  ← arimo.io'ya taşınır
└── Error                            ← kurtarılamaz
    ├── OutOfMemoryError
    ├── StackOverflowError
    └── AssertionError
```

### Enum'lar
| İsim | Açıklama |
|---|---|
| `ThreadState` | NEW, RUNNABLE, BLOCKED, WAITING, TERMINATED |

---

## 4. arimo.lang.annotation — Annotation Altyapısı

> Compile-time annotation metadata. TypeChecker'da zaten var, lib'e taşınır.

### Interface'ler / Annotation'lar
| İsim | Açıklama |
|---|---|
| `Annotation` | Tüm annotation'ların kök arayüzü |
| `@Retention` | Annotation'ın ne kadar süre saklanacağı |
| `@Target` | Hangi bağlamlarda kullanılabilir |
| `@Documented` | Public API'nin parçası |
| `@Inherited` | Alt sınıflara miras |
| `@Repeatable` | Aynı annotation birden fazla kullanılabilir |

### Enum'lar
| İsim | Değerler |
|---|---|
| `ElementType` | CLASS, INTERFACE, METHOD, FIELD, PARAMETER, CONSTRUCTOR, LOCAL_VARIABLE, ANNOTATION |
| `RetentionPolicy` | SOURCE, COMPILE, RUNTIME |

---

## 5. arimo.io — Giriş/Çıkış Akışları

> java.io eşdeğeri. ARC-managed stream'ler.

### Interface'ler
| İsim | Açıklama |
|---|---|
| `Closeable` | `close() : Void` — try/defer ile otomatik kapama |
| `Flushable` | `flush() : Void` — buffer'ı boşalt |
| `Readable` | `read(buf: Slice<Char>) : Integer` |
| `Appendable` | `append(c: Char) : Appendable` |

### Sınıflar — Stream Hiyerarşisi
```
Byte Streams:
  InputStream (abstract)
    ├── FileInputStream
    ├── ByteArrayInputStream
    └── BufferedInputStream

  OutputStream (abstract)
    ├── FileOutputStream
    ├── ByteArrayOutputStream
    └── BufferedOutputStream

Character Streams:
  Reader (abstract)
    ├── FileReader
    ├── StringReader
    ├── BufferedReader         ← readLine() : String?
    └── InputStreamReader      ← Byte→Char dönüşümü

  Writer (abstract)
    ├── FileWriter
    ├── StringWriter
    ├── BufferedWriter         ← newLine(), flush()
    └── OutputStreamWriter     ← Char→Byte dönüşümü

Formatted:
  PrintWriter                  ← print()/println()/printf()
```

### İstisnalar
| İsim | Açıklama |
|---|---|
| `IOException` | Genel G/Ç hatası |
| `FileNotFoundException` | Dosya bulunamadı |
| `EOFException` | Beklenmedik stream sonu |
| `UnsupportedEncodingException` | Desteklenmeyen karakter seti |

---

## 6. arimo.fs — Dosya Sistemi

> java.nio.file eşdeğeri (Java'nın java.io.File değil, daha modern NIO.file).

### Interface'ler
| İsim | Açıklama |
|---|---|
| `FileVisitor<T>` | Dizin ağacı traversal callback |

### Sınıflar
| İsim | Açıklama | Metodlar |
|---|---|---|
| `Path` | Dosya sistemi yolu (immutable) | `of()`, `join()`, `parent()`, `filename()`, `extension()`, `toString()` |
| `File` | Dosya işlemleri (static metod koleksiyonu) | `read(path)`, `readLines(path)`, `write(path, content)`, `append(path, content)`, `exists(path)`, `delete(path)`, `size(path)`, `copy(src, dst)`, `move(src, dst)` |
| `Directory` | Klasör işlemleri | `list(path)`, `create(path)`, `createAll(path)`, `delete(path)`, `exists(path)` |
| `FileStream` | Dosya stream'i | `open(path, mode)`, `read(n)`, `write(data)`, `seek(pos)`, `close()` |
| `FileMode` | Açma modu enum | READ, WRITE, APPEND, READ_WRITE |

### İstisnalar
| İsim | Açıklama |
|---|---|
| `FileNotFoundException` | arimo.io'dan miras |
| `AccessDeniedException` | İzin hatası |
| `DirectoryNotEmptyException` | Dolu dizin silme |
| `FileAlreadyExistsException` | Dosya var zaten |

---

## 7. arimo.util — Koleksiyonlar

> java.util eşdeğeri. Arimo'ya özgü ARC-managed.

### Interface Hiyerarşisi
```
Collection<T> (Iterable<T>)
├── List<T>
├── Set<T>
│   └── SortedSet<T> (Comparable<T>)
└── Queue<T>
    └── Deque<T>

Map<K,V>
└── SortedMap<K,V>
    └── NavigableMap<K,V>
```

### Implementasyonlar
| İsim | Açıklama | Java Karşılığı |
|---|---|---|
| `ArrayList<T>` | Dinamik dizi: O(1) get, amortize O(1) append | ArrayList |
| `LinkedList<T>` | Çift bağlı: O(1) baştan/sondan ekleme | LinkedList |
| `HashMap<K,V>` | Hash map: O(1) ortalama | HashMap |
| `LinkedHashMap<K,V>` | Ekleme sırası koruyan HashMap | LinkedHashMap |
| `TreeMap<K,V>` | Kırmızı-siyah ağaç: O(log n) | TreeMap |
| `HashSet<T>` | HashMap tabanlı Set | HashSet |
| `TreeSet<T>` | TreeMap tabanlı SortedSet | TreeSet |
| `ArrayDeque<T>` | Dizi tabanlı Deque + Stack + Queue | ArrayDeque |
| `PriorityQueue<T>` | Min-heap | PriorityQueue |

### Yardımcı Sınıflar
| İsim | Açıklama |
|---|---|
| `Collections` | `sort()`, `reverse()`, `shuffle()`, `min()`, `max()`, `frequency()`, `unmodifiableList()` |
| `Arrays` | `sort()`, `fill()`, `copyOf()`, `asList()`, `binarySearch()` |
| `Objects` | `requireNonNull()`, `equals()`, `hash()`, `toString()`, `isNull()` |
| `Optional<T>` | Null-safe değer: `of()`, `empty()`, `get()`, `orElse()`, `ifPresent()`, `map()`, `filter()` |
| `Scanner` | Metin/stdin okuma: `nextLine()`, `nextInt()`, `nextFloat()`, `hasNext()` |
| `Random` | `nextInt(max)`, `nextFloat()`, `nextBoolean()`, `nextLong()` |
| `UUID` | `random()`, `fromString()`, `toString()` |
| `Base64` | `encode(bytes)`, `decode(str)` |
| `StringJoiner` | `add()`, `toString()` (sınırlayıcı, prefix, suffix) |

### İstisnalar
| İsim | Açıklama |
|---|---|
| `NoSuchElementException` | Boş koleksiyona erişim |
| `ConcurrentModificationException` | Eşzamanlı değişiklik |
| `EmptyStackException` | Boş stack pop |

---

## 8. arimo.util.concurrent — Eşzamanlılık

> java.util.concurrent eşdeğeri. Faza 9'da implement edilecek.

| İsim | Açıklama |
|---|---|
| `Mutex` | Karşılıklı dışlama kilidi: `lock()`, `unlock()`, `withLock(fn)` |
| `RwLock<T>` | Okuma/yazma kilidi: `readLock()`, `writeLock()` |
| `Atomic<T>` | Atomik okuma/yazma: `get()`, `set()`, `compareAndSwap()` |
| `Channel<T>` | Mesaj kanalı (mpsc): `send()`, `receive()`, `tryReceive()` |
| `CountDownLatch` | N thread buluşma noktası |
| `Semaphore` | Erişim sayacı: `acquire()`, `release()` |
| `Future<T>` | Async sonuç: `get()`, `isDone()`, `cancel()` |
| `ThreadPool` | Thread havuzu: `submit(task)`, `shutdown()` |

---

## 9. arimo.math — Matematik

> java.lang.Math genişletmesi + sayısal tipler.

| İsim | Açıklama |
|---|---|
| `Math` | Tüm temel matematiksel operasyonlar (mevcut stub genişletilir) |
| `BigInteger` | Sınırsız hassasiyetli tam sayı (ilerisi) |
| `BigDecimal` | Sınırsız hassasiyetli ondalık (ilerisi) |
| `MathContext` | BigDecimal yuvarlama bağlamı (ilerisi) |
| `Complex` | Karmaşık sayı: `real`, `imag`, `abs()`, `conjugate()` |
| `Matrix<T>` | Matris operasyonları (SIMD ile) |
| `Vector2`, `Vector3`, `Vector4` | Geometri vektörleri |

---

## 10. arimo.text — Metin İşleme

> java.lang.String'in dışına taşan text utilities.

| İsim | Açıklama |
|---|---|
| `Regex` | Desen eşleme: `compile(pattern)`, `match(str)`, `find()`, `replace()` |
| `Formatter` | `printf` tarzı: `format(template, args...)` |
| `StringTokenizer` | Basit string bölme (Scanner'dan hafif) |
| `Charset` | Karakter seti: UTF_8, UTF_16, ASCII, ISO_8859_1 |
| `Encoder` | Charset dönüşümü: `encode(str)`, `decode(bytes)` |

---

## 11. arimo.time — Tarih/Saat

> java.time eşdeğeri (legacy Calendar/Date değil).

| İsim | Açıklama |
|---|---|
| `Instant` | Mutlak zaman noktası (Unix epoch'tan nanosaniye) |
| `Duration` | Zaman aralığı: `ofSeconds()`, `ofMillis()`, `ofNanos()`, `plus()`, `minus()` |
| `LocalDate` | Tarihe göre: `now()`, `of(year, month, day)`, `plusDays()` |
| `LocalTime` | Saate göre: `now()`, `of(hour, min, sec)` |
| `LocalDateTime` | Tarih + saat (timezone yok) |
| `ZonedDateTime` | Timezone ile tarih/saat |
| `ZoneId` | Timezone tanımlayıcısı: `of("Europe/Istanbul")`, `systemDefault()` |
| `DateTimeFormatter` | `format(dt)`, `parse(str)` |
| `Clock` | `systemUTC()`, `systemDefaultZone()`, `tick()` |
| `Time` | Mevcut stub'dan lib'e: `now()`, `nowMillis()`, `sleep()`, `generateId()` |

---

## 12. arimo.net — Ağ

> java.net + java.net.http eşdeğeri.

### arimo.net
| İsim | Açıklama |
|---|---|
| `InetAddress` | IP adresi: `getByName(host)`, `getLocalHost()` |
| `URL` | URL parse/build: `of(str)`, `host()`, `path()`, `query()` |
| `URI` | URI: `of(str)`, `toString()`, `resolve()` |
| `TcpSocket` | TCP bağlantısı: `connect()`, `read()`, `write()`, `close()` |
| `UdpSocket` | UDP datagram: `bind()`, `send()`, `receive()` |
| `ServerSocket` | TCP sunucusu: `bind()`, `accept()` |
| `Proxy` | Proxy yapılandırması |
| `NetworkInterface` | Ağ arayüzü: `getAll()`, `name()`, `addresses()` |

### arimo.net.http
| İsim | Açıklama |
|---|---|
| `HttpClient` | `get(url)`, `post(url, body)`, `put()`, `delete()`, `send(request)` |
| `HttpRequest` | Builder pattern: `newBuilder()`, `uri()`, `header()`, `body()` |
| `HttpResponse<T>` | `statusCode()`, `body()`, `headers()` |
| `HttpHeaders` | `get(name)`, `all()` |
| `WebSocket` | `connect()`, `send()`, `onMessage()`, `close()` |

---

## 13. arimo.security — Güvenlik

| İsim | Açıklama |
|---|---|
| `Hash` | `sha256(data)`, `sha512(data)`, `md5(data)` |
| `Hmac` | `sign(key, data)`, `verify(key, data, sig)` |
| `Random` | Kriptografik güvenli rastgele: `bytes(n)`, `uuid()` |
| `Base64` | `encode()`, `decode()` (arimo.util'den miras?) |
| `Hex` | `encode(bytes)`, `decode(str)` |
| `AES` | Simetrik şifreleme: `encrypt()`, `decrypt()` |

---

## 14. arimo.sys — Sistem Erişimi

> java.lang.foreign + low-level sistem erişimi.

| İsim | Açıklama |
|---|---|
| `Process` | `run(cmd, args)`, `spawn()`, `pid()`, `exitCode()` |
| `Environment` | `get(key)`, `set(key, value)`, `all()` → Map<String,String> |
| `Signal` | `handle(SIGTERM, fn)`, `ignore(SIGPIPE)` |
| `NativeLib` | `load(path)`, `symbol(name)` → fonksiyon pointer |
| `RawMemory` | `read<T>(addr)`, `write<T>(addr, val)`, `map(addr, len)` |
| `Platform` | `os()`, `arch()`, `pageSize()`, `cpuCount()` |

---

## 15. Dosya Yapısı

```
stdlib/
├── module-info.arm          ← arimo.base modül tanımlayıcısı
│
├── arimo/
│   ├── lang/
│   │   ├── Object.arm
│   │   ├── String.arm
│   │   ├── StringBuilder.arm
│   │   ├── Number.arm
│   │   ├── Integer.arm
│   │   ├── Float.arm
│   │   ├── Boolean.arm
│   │   ├── Char.arm
│   │   ├── Long.arm
│   │   ├── Byte.arm
│   │   ├── Short.arm
│   │   ├── Math.arm
│   │   ├── IO.arm
│   │   ├── System.arm
│   │   ├── Memory.arm
│   │   ├── Throwable.arm
│   │   ├── Exception.arm
│   │   ├── RuntimeException.arm
│   │   ├── Error.arm
│   │   ├── NullPointerException.arm
│   │   ├── IllegalArgumentException.arm
│   │   ├── IndexOutOfBoundsException.arm
│   │   ├── ArithmeticException.arm
│   │   ├── NumberFormatException.arm
│   │   ├── Comparable.arm        ← interface
│   │   ├── Iterable.arm          ← interface
│   │   ├── Runnable.arm          ← interface
│   │   ├── Callable.arm          ← interface
│   │   └── AutoCloseable.arm     ← interface
│   │
│   ├── lang/annotation/
│   │   ├── Annotation.arm
│   │   ├── ElementType.arm
│   │   ├── RetentionPolicy.arm
│   │   ├── Retention.arm
│   │   ├── Target.arm
│   │   ├── Documented.arm
│   │   └── Inherited.arm
│   │
│   ├── io/
│   │   ├── InputStream.arm
│   │   ├── OutputStream.arm
│   │   ├── Reader.arm
│   │   ├── Writer.arm
│   │   ├── FileInputStream.arm
│   │   ├── FileOutputStream.arm
│   │   ├── FileReader.arm
│   │   ├── FileWriter.arm
│   │   ├── BufferedReader.arm
│   │   ├── BufferedWriter.arm
│   │   ├── ByteArrayInputStream.arm
│   │   ├── ByteArrayOutputStream.arm
│   │   ├── StringReader.arm
│   │   ├── StringWriter.arm
│   │   ├── PrintWriter.arm
│   │   ├── InputStreamReader.arm
│   │   ├── OutputStreamWriter.arm
│   │   ├── Closeable.arm         ← interface
│   │   ├── Flushable.arm         ← interface
│   │   ├── IOException.arm
│   │   ├── FileNotFoundException.arm
│   │   └── EOFException.arm
│   │
│   ├── fs/
│   │   ├── Path.arm
│   │   ├── File.arm
│   │   ├── Directory.arm
│   │   ├── FileStream.arm
│   │   ├── FileMode.arm          ← enum
│   │   ├── AccessDeniedException.arm
│   │   └── FileAlreadyExistsException.arm
│   │
│   ├── util/
│   │   ├── Collection.arm        ← interface
│   │   ├── List.arm              ← interface
│   │   ├── Set.arm               ← interface
│   │   ├── Map.arm               ← interface
│   │   ├── Queue.arm             ← interface
│   │   ├── Deque.arm             ← interface
│   │   ├── Iterator.arm          ← interface
│   │   ├── Comparator.arm        ← interface
│   │   ├── ArrayList.arm
│   │   ├── LinkedList.arm
│   │   ├── HashMap.arm
│   │   ├── LinkedHashMap.arm
│   │   ├── TreeMap.arm
│   │   ├── HashSet.arm
│   │   ├── TreeSet.arm
│   │   ├── ArrayDeque.arm
│   │   ├── PriorityQueue.arm
│   │   ├── Collections.arm       ← statik yardımcılar
│   │   ├── Arrays.arm
│   │   ├── Objects.arm
│   │   ├── Optional.arm
│   │   ├── Scanner.arm
│   │   ├── Random.arm
│   │   ├── UUID.arm
│   │   ├── Base64.arm
│   │   ├── StringJoiner.arm
│   │   ├── NoSuchElementException.arm
│   │   └── ConcurrentModificationException.arm
│   │
│   ├── util/concurrent/
│   │   ├── Mutex.arm
│   │   ├── RwLock.arm
│   │   ├── Atomic.arm
│   │   ├── Channel.arm
│   │   ├── Future.arm
│   │   ├── CountDownLatch.arm
│   │   ├── Semaphore.arm
│   │   └── ThreadPool.arm
│   │
│   ├── math/
│   │   ├── Math.arm              ← genişletilmiş
│   │   ├── Complex.arm
│   │   └── Matrix.arm
│   │
│   ├── text/
│   │   ├── Regex.arm
│   │   ├── Formatter.arm
│   │   ├── Charset.arm
│   │   └── Encoder.arm
│   │
│   ├── time/
│   │   ├── Instant.arm
│   │   ├── Duration.arm
│   │   ├── LocalDate.arm
│   │   ├── LocalTime.arm
│   │   ├── LocalDateTime.arm
│   │   ├── ZonedDateTime.arm
│   │   ├── ZoneId.arm
│   │   ├── DateTimeFormatter.arm
│   │   ├── Clock.arm
│   │   └── Time.arm              ← mevcut stub'dan
│   │
│   ├── net/
│   │   ├── InetAddress.arm
│   │   ├── URL.arm
│   │   ├── URI.arm
│   │   ├── TcpSocket.arm
│   │   ├── UdpSocket.arm
│   │   └── ServerSocket.arm
│   │
│   ├── net/http/
│   │   ├── HttpClient.arm
│   │   ├── HttpRequest.arm
│   │   ├── HttpResponse.arm
│   │   ├── HttpHeaders.arm
│   │   └── WebSocket.arm
│   │
│   ├── security/
│   │   ├── Hash.arm
│   │   ├── Hmac.arm
│   │   ├── Hex.arm
│   │   └── AES.arm
│   │
│   └── sys/
│       ├── Process.arm
│       ├── Environment.arm
│       ├── Signal.arm
│       ├── Platform.arm
│       └── RawMemory.arm
```

---

## 16. Compiler Değişiklikleri

### Lexer
- `package` keyword ekle (yeni)
- `module` keyword'ü üst düzey modül tanımı için ayır

### Parser
- Dosya başlığı: `module X;` → `package X;` (geriye dönük: module de kabul edilir)
- Yeni: `module X { exports Y; }` sözdizimi
- Wildcard import: `import X.Y.*;`

### TypeChecker
- `arimo.lang.*` → otomatik inject (IO, Math, System, Memory, String vs.)
- Package resolution: `import arimo.fs.File;` → dosya bul + parse
- Wildcard import çözümleme

### Multi-file Compilation
- `arc` komutu birden fazla `.arm` dosyasını birleştirir
- `arc build` arc.toml'a göre projeyi derler
- Import grafiği → topological sort → derleme sırası

---

## 17. Uygulama Önceliği

### Faz 5.0 — Parser + Lexer Değişikliği (hemen)
- `package` keyword
- `module X { }` tanımı
- Wildcard import

### Faz 5.1 — arimo.lang core
Object, String, StringBuilder, Integer, Float, Boolean, Char, Math, IO, System,
Exception hiyerarşisi, Comparable, Iterable, AutoCloseable

### Faz 5.2 — arimo.fs
File, Directory, Path, FileStream, FileMode — bu bootstrap için kritik

### Faz 5.3 — arimo.io
InputStream/OutputStream hiyerarşisi, Reader/Writer, Buffered sınıflar

### Faz 5.4 — arimo.util
Collection hiyerarşisi, ArrayList, HashMap, TreeMap, Optional, Scanner, Random, UUID

### Faz 5.5 — arimo.time
Instant, Duration, LocalDate/Time, ZonedDateTime, DateTimeFormatter

### Faz 5.6 — arimo.math + arimo.text
Math genişleme, Regex, Formatter, Charset

### Faz 5.7 — arimo.net
TcpSocket, HttpClient

### Faz 5.8 — arimo.sys
Process, Environment, Platform, Signal

### Faz 5.9 — arimo.security
Hash, Hmac, Hex, Base64

### Faz 9 — arimo.util.concurrent
Thread, Mutex, Channel, Future, ThreadPool
