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
- `Vec4f`, `Vec8f`, `Vec4i`, `Vec8i` — TypeChecker'da kayıtlı

### 3.2 @Likely / @Unlikely ✅
- `if @Likely (cond)` — AST'de saklanır, CodeGen'de implement edilecek

### 3.3 Interface default metodlar ✅
- `default methodName() : Type { body }` — override etmek zorunda değil

### 3.4 async / await ✅
- Parse+AST tamam; CodeGen state machine (Faza 4'te)

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
### 4.2 Operatörler ✅
### 4.3 Kontrol Akışı ✅
### 4.4 Fonksiyon ve Metod Üretimi ✅
### 4.5 Class Instances ✅
### 4.6 Enum CodeGen ✅
### 4.7 Inheritance ✅
### 4.8 Static Fields ✅
### 4.9 Stdlib Stubs ✅
### 4.10 comprehensive.arm → native .exe ✅

### 4.11 Collections Runtime ✅ TAMAMLANDI
- `List<T>` → saf LLVM IR, flat-array tasarımı
  - `append()`, `length()`, `isEmpty()`, `get(i)`, `filter(lambda)`, `for-each`
- `HashMap<K,V>` → lineer arama, strcmp ile key karşılaştırma
  - `set()`, `getOrDefault()`, `get()`
- `Pair<A,B>` → 16-byte malloc blok
  - constructor, `getFirst()`, `getSecond()`
- Lambda → LLVM function pointer (`arc_lambda_N`)
- ForEach → `arc_list_length` + `arc_list_get` döngüsü
- StrInterp return bağlamında → `sprintf` + `malloc(1024)`

---

## FAZA 4 — Kalan CodeGen İşleri ⬜

### 4.12 Lambda — Tam Destek
- [ ] Genel lambda → LLVM function pointer (şu an sadece filter'da çalışıyor)
- [ ] Closure capture — dış scope değişkenlerine erişim
- [ ] `list.sortedBy(comparator)` → qsort entegrasyonu
- [ ] `list.reduce(init, fn)` → fold işlemi
- [ ] Lambda'nın değişkene atanması: `(Integer) -> Boolean fn = (x) -> x > 0;`
- [ ] Atanan lambda'nın çağrılması: `Boolean r = fn(42);`

### 4.13 String Metodları
- [ ] `str.length()` → strlen
- [ ] `str.contains(sub)` → strstr != null
- [ ] `str.startsWith(prefix)` → strncmp
- [ ] `str.endsWith(suffix)` → strncmp + offset
- [ ] `str.toUpper()` → toupper loop
- [ ] `str.toLower()` → tolower loop
- [ ] `str.trim()` → baştan ve sondan boşluk sil
- [ ] `str.split(delim)` → `List<String>` döndür
- [ ] `str.compareTo(other)` → strcmp → Integer
- [ ] `str + other` → runtime concat (strcat tabanlı)
- [ ] Enum değerinin string'e otomatik dönüşümü: `${this.status}` → `TaskStatus_label(val)`

### 4.14 Exception Handling
- [ ] `throw ExceptionType(args)` → basit: `abort()` çağrısı
- [ ] `try/catch` → catch bloğunu şimdilik atla, try body'yi çalıştır (mevcut)
- [ ] `try/catch` → LLVM `landingpad` ile gerçek exception (gelişmiş, sonraki adım)
- [ ] `finally` bloğu → her durumda çalışması garantilenmeli
- [ ] `Exception.message()` → mesaj string'i döndür
- [ ] Exception kalıtımı → `instanceof` benzeri kontrol

### 4.15 ARC Memory Management
- [ ] BorrowChecker `drop_schedule`'ı kullanarak scope çıkışında `free()` ekle
- [ ] `@ManualMemory` class'larda otomatik free atla
- [ ] Referans sayımı için refcount field'ı (class struct'a ekle)
- [ ] `refcount++` → nesne paylaşılınca
- [ ] `refcount--` + `free()` → refcount 0'a düşünce

### 4.16 Systems CodeGen
- [ ] `volatile load/store` → LLVM `volatile` attribute
- [ ] `extern "C" { ... }` → LLVM `declare` (şu an TypeChecker'da kayıtlı, CodeGen'e taşınmalı)
- [ ] `asm { ... }` → LLVM inline asm string
- [ ] `noreturn` → LLVM `unreachable` terminatörü
- [ ] `@Packed` struct → LLVM `{ packed }` struct type
- [ ] `@Align(N)` struct → LLVM alignment attribute
- [ ] `@Section("isim")` → LLVM section attribute
- [ ] `@CallingConvention("C")` → LLVM calling convention

### 4.17 Performance CodeGen
- [ ] SIMD tipleri → LLVM vector types (`<4 x float>` vb.)
  - `Vec4f` + `Vec8f` operatörleri → `fadd <4 x float>` vb.
- [ ] `@Likely/@Unlikely` → LLVM `branch_weights` metadata
- [ ] `async/await` → state machine dönüşümü (coroutine tabanlı)
- [ ] `@ForceInline` → LLVM `alwaysinline` attribute
- [ ] `@Pure` → LLVM `readnone` attribute

### 4.18 Output İyileştirme
- [ ] `-O2` / `-O0` flag → `OptimizationLevel::Aggressive` / `None`
- [ ] Debug info (DWARF) → inkwell debug info API
- [ ] Cross-compilation: Linux (`x86_64-unknown-linux-gnu`), macOS (`aarch64-apple-darwin`)
- [ ] `-emit-llvm` flag → ham LLVM IR dosyası (şu an `--emit-ir` var, geliştirilebilir)
- [ ] `-c` flag → sadece `.o` üret, link etme

---

## FAZA 5 — Stdlib ⬜

Stdlib, Arimo kaynak dosyaları (`.arm`) olarak yazılır ve arc ile derlenir.
Her modül ayrı bir `.arm` dosyasıdır.

### 5.1 arimo.io — Giriş/Çıkış
- [ ] `IO.print(msg)` → terminal çıktısı (şu an çalışıyor, stdlib'e taşınacak)
- [ ] `IO.println(msg)` → newline ekleyerek yaz
- [ ] `IO.read()` → stdin'den satır oku
- [ ] `IO.readInt()` → stdin'den Integer oku
- [ ] `IO.error(msg)` → stderr'e yaz

### 5.2 arimo.fs — Dosya Sistemi
- [ ] `File.open(path)` → dosya aç
- [ ] `File.read(path)` → tüm içeriği String olarak oku
- [ ] `File.write(path, content)` → dosyaya yaz
- [ ] `File.append(path, content)` → dosyaya ekle
- [ ] `File.exists(path)` → Boolean
- [ ] `File.delete(path)` → dosyayı sil
- [ ] `Directory.list(path)` → `List<String>` dosya/klasör adları
- [ ] `Directory.create(path)` → klasör oluştur
- [ ] `Path.join(parts...)` → yol birleştir
- [ ] `Path.extension(path)` → uzantı al

### 5.3 arimo.time — Tarih/Saat
- [ ] `Time.now()` → gerçek zaman damgası (şu an stub)
- [ ] `Time.nowMillis()` → Unix millisaniye
- [ ] `Time.generateId()` → gerçek UUID (şu an counter tabanlı)
- [ ] `Time.format(timestamp, pattern)` → tarih formatlama

### 5.4 arimo.collections — Gelişmiş Koleksiyonlar
- [ ] `List.sortedBy()` → qsort entegrasyonu
- [ ] `List.take(n)` / `List.takeLast(n)`
- [ ] `List.reduce(init, fn)`
- [ ] `List.map(fn)` → dönüştürme
- [ ] `List.flatMap(fn)`
- [ ] `List.any(fn)` / `List.all(fn)`
- [ ] `List.distinct()` → tekrar edenleri çıkar
- [ ] `HashMap.entries()` → `List<Pair<K,V>>`
- [ ] `HashMap.keys()` → `List<K>`
- [ ] `HashMap.values()` → `List<V>`
- [ ] `HashMap.remove(key)`
- [ ] `HashMap.containsKey(key)`
- [ ] `TreeMap<K,V>` → sıralı map implementasyonu

### 5.5 arimo.math — Matematik
- [ ] `Math.min(a, b)` / `Math.max(a, b)`
- [ ] `Math.floor(f)` / `Math.ceil(f)` / `Math.round(f)`
- [ ] `Math.log(x)` / `Math.log2(x)` / `Math.log10(x)`
- [ ] `Math.sin(x)` / `Math.cos(x)` / `Math.tan(x)`
- [ ] `Math.random()` → 0.0–1.0 arası Float
- [ ] `Integer.MAX` / `Integer.MIN` sabitleri

### 5.6 arimo.string — String Yardımcıları
- [ ] `String.format(template, args...)` → sprintf benzeri
- [ ] `String.parseInt(s)` → Integer
- [ ] `String.parseFloat(s)` → Float
- [ ] `Integer.toString(n)` → String
- [ ] `Float.toString(f)` → String

### 5.7 arimo.net — Ağ (İleri Aşama)
- [ ] `HttpClient.get(url)` → String response
- [ ] `HttpClient.post(url, body)` → String response
- [ ] `TcpSocket` — bağlantı aç/kapat, oku/yaz

### 5.8 Tooling
- [ ] VSCode extension — syntax highlighting (`.arm` dosyaları için)
- [ ] Language Server Protocol (LSP) — otomatik tamamlama, hata gösterimi
- [ ] `arc.toml` — proje manifest dosyası (bağımlılık yönetimi)
- [ ] `arc build` / `arc run` / `arc test` — CLI komutları

---

## FAZA 6 — Bootstrapping (arc'ı Arimo ile Yeniden Yazma) ⬜

Bootstrapping: bir derleyiciyi kendi derlediği dille yeniden yazmak.
Bu Arimo'nun olgunluk kanıtıdır.

### Ön Koşullar

Bootstrapping başlamadan önce şunlar tamamlanmış olmalıdır:

| Bileşen | Neden Gerekli |
|---|---|
| String metodları (4.13) | Lexer karakter/string işleme yapar |
| Exception handling (4.14) | Parse hataları, tip hataları fırlatılır |
| Lambda tam destek (4.12) | Derleyici içi yüksek seviye işlemler |
| arimo.fs (5.2) | Kaynak `.arm` dosyası okunur |
| arimo.io (5.1) | Hata ve bilgi mesajları |
| arimo.collections (5.4) | Token listesi, AST node'ları, scope tabloları |
| ARC bellek yönetimi (4.15) | Derleyici uzun süre çalışır, bellek sızmamalı |

### Bootstrapping Aşamaları

#### Stage 0 — Rust Derleyici (arc-rust)
- Mevcut `arc` (Rust ile yazılmış)
- Arimo kodunu derleyebilir
- Tarihsel referans olarak saklanır, public kalır
- Bu repo: `arimo-compiler` (mevcut)

#### Stage 1 — Arimo Derleyici İlk Derleme (arc-arimo)
- Yeni repo: `arimo-compiler-self` (veya `arc`)
- Arimo diliyle yazılmış yeni compiler
- Stage 0 (Rust compiler) ile derlenir
- Backend: LLVM IR metin çıktısı üretir (inkwell gerekmez)
  - `arc --emit-ir src.arm > src.ll`
  - `llc src.ll -o src.o`
  - `gcc src.o -o src.exe`
- Rust compiler ile aynı çıktıyı üretmesi doğrulama kriteri

#### Stage 2 — Kendini Derleyen Compiler
- Stage 1'deki arc-arimo, Stage 1'in kendisiyle derlenir
- Eğer aynı binary çıkıyorsa bootstrapping tamamdır
- Artık Rust bağımlılığı yok

### Bootstrapping Stratejisi

Arimo'daki compiler yapısı:

```
arc-arimo/
├── src/
│   ├── Lexer.arm       — token üretimi
│   ├── Parser.arm      — AST üretimi
│   ├── TypeChecker.arm — tip kontrolü
│   ├── BorrowChecker.arm
│   ├── CodeGen.arm     — LLVM IR metin çıktısı
│   └── Main.arm        — pipeline
```

LLVM IR metin üretimi Arimo'dan çok daha basit:
```arimo
// LLVM IR metin olarak üretilir — inkwell binding gerekmez
IO.write("define i32 @main() {\n");
IO.write("  ret i32 0\n");
IO.write("}\n");
```

### Neden Önemli

- Dilin kendisini ifade edebildiğinin kanıtı (Turing completeness değil, pratik yeterlilik)
- Rust bağımlılığı ortadan kalkar
- Arimo ile Arimo geliştirilebilir hale gelir
- Rust derleyici tarihsel belge olarak kalır — dilin sıfırdan nasıl inşa edildiğini gösterir

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
```

### Beklenen Çıktı
```
arc: compiling  ... linking ... OK
arc: → hello.exe
```
