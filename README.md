# Arimo Lang — Dil Spesifikasyonu v1.4

> Statically typed · Compiled · OOP · GC-free ownership  
> Compiler: `arc` | Kaynak uzantısı: `.arm` | Compiler dili: Rust

---

## 1. Module Sistemi

```arimo
module arimo.shop.model;       // dosya başı — zorunlu
import arimo.shop.exception;   // bağımlılık bildirimi
import arimo.io;
```

- Bir dosya = bir `public class` (ya da struct / interface / enum)
- Dosya adı class adıyla eşleşir: `Task.arm` → `public class Task`
- `module` klasör yapısıyla birebir örtüşür
- `arc TaskApplication.arm` ile derlenir

---

## 2. Tipler

### 2.1 Primitifler

```arimo
Integer   Float   Boolean   String   Void   noreturn
```

- `noreturn` — fonksiyon hiç geri dönmez (panic, exit)

### 2.2 Fixed-size Integer'lar

```arimo
u8   u16   u32   u64    // işaretsiz
i8   i16   i32   i64    // işaretli
```

- Implicit widening YOK: `u32 x = some_u64;` → type error
- Integer literal her boyuta atanabilir: `u8 x = 42;`

### 2.3 Koleksiyonlar

```arimo
List<T>
Map<K,V>          // interface
  HashMap<K,V>    // sırasız, hızlı
  TreeMap<K,V>    // key'e göre sıralı
Pair<A,B>         // iki değer, tuple yok
Array<T, N>       // compile-time boyutlu, stack-allocated
Slice<T>          // non-owning fat pointer (ptr + len)
```

### 2.4 SIMD Tipleri

```arimo
Vec4f   Vec8f   // 4/8 × Float32
Vec4i   Vec8i   // 4/8 × Integer
```

- Operator overloading: `+`, `-`, `*`, `/`
- Metodlar: `length()`, `normalize()`, `dot(other)`

### 2.5 Diğer Tipler

```arimo
RawPtr<T>                     // ham pointer — sadece @ManualMemory
(T1, T2) -> R                 // function pointer
Result<T, E>                  // yerleşik generic enum
type NodeId = u32;             // type alias
```

### 2.6 Tip Uyumu Kuralları

```arimo
// Integer literal → Float'a atanabilir
Float pi = 3;        // ✅  (3 → 3.0)
Float f  = 42;       // ✅

// Integer literal → herhangi bir sized integer'a atanabilir
u8  byte  = 255;     // ✅
i32 val   = -1;      // ✅

// Sized integer → Integer (genel tip) atanabilir
Integer n  = some_u32;   // ✅

// Implicit widening YOK — aynı boyut grubu dışında cast zorunlu
u32 word   = some_u64;   // ❌ HATA — as kullan
u32 casted = some_u64 as u32;  // ✅

// HashMap / TreeMap → Map (interface) atanabilir
Map<String, Integer> m = HashMap();  // ✅
```

### 2.7 toString()

Tüm class'lar otomatik olarak `toString() : String` metoduna sahiptir (Object'ten gelir).
Override etmek için class içinde aynı imzalı metodu tanımla:

```arimo
public class Task {
    public toString() : String {
        return "[${this.status}] ${this.title}";
    }
}
```

---

## 3. Tip Bildirimi Kuralları

Arimo'da iki farklı bağlama göre iki sözdizimi kullanılır:

### 3.1 Bildirim bağlamı — `: Type` (kolon, tip sağda)

Field, parametre ve dönüş tipi bildirimlerinde tip her zaman sağda gelir:

```arimo
// Field
private readonly radius : Float;
private          color  : String;

// Parametre
public constructor(id: String, radius: Float) { ... }

// Dönüş tipi
public getRadius() : Float { ... }
public area()      : Float { ... }
```

### 3.2 Local değişken — `Type name` (tip solda)

Method gövdesi içindeki yerel değişkenlerde tip solda gelir:

```arimo
public static main() : Void {
    String  name  = "Arimo";
    Integer count = 0;
    Boolean done  = false;

    List<Task>           tasks  = List();
    Map<String, Integer> scores = HashMap();
}
```

### Özet

| Bağlam | Sözdizimi | Örnek |
|---|---|---|
| Field | `name : Type` | `private id : String;` |
| Parametre | `name: Type` | `(radius: Float)` |
| Dönüş tipi | `() : Type` | `getArea() : Float` |
| Local değişken | `Type name` | `String name = "Arimo";` |

---

## 4. Null Güvenliği

```arimo
String  name = "Arimo";      // null olamaz — garanti
String? name = null;         // nullable — açıkça işaretlenmeli

// Smart cast — if bloğu içinde derleyici null olmadığını bilir
String? title = task.getTitle();
if (title != null) {
    IO.print(title);         // burada String, String? değil
}

// Null-safe erişim — field ve method call
String?  name    = user?.getName();         // method çağrısı
Integer? len     = name?.length();          // zincir
String?  found   = user?.findTask("id");    // argümanlı method çağrısı
String?  address = user?.address;           // field erişimi
```

---

## 5. Literal'lar

```arimo
// Sayılar
Integer x = 42;
Float   f = 3.14;
u32     h = 0xDEADBEEF;   // hex literal
u8      b = 0b10101010;   // binary literal

// String interpolation — + operatörü string birleştirme için değil
IO.print("Merhaba ${name}!");
IO.print("Sonuç: ${a + b} birim");
IO.print("Oran: %${rate}");

// Boolean
Boolean t = true;
Boolean f = false;
```

---

## 6. Erişim Belirteçleri

```arimo
public    // her yerden
private   // sadece bu class
protected // bu class + alt sınıflar
internal  // sadece aynı module
```

- Class içinde her field ve metodda **zorunlu**
- Interface içinde yazılmaz — zaten public

---

## 7. Modifier'lar

```arimo
private readonly id      : String;               // bir kez atanır
public  static   MAX     : Integer = 50;         // class seviyesi
public  static readonly  VERSION : String = "1.4.0";
```

---

## 8. Class

```arimo
public class Circle extends Shape implements Drawable, Movable {

    private readonly id     : String;
    private readonly radius : Float;
    private          color  : String;

    public constructor(id: String, radius: Float, color: String) {
        this.id     = id;
        this.radius = radius;
        this.color  = color;
    }

    public static create(radius: Float, color: String) : Circle {
        return Circle(Time.generateId(), radius, color);
    }

    public getRadius() : Float  { return this.radius; }
    public getColor()  : String { return this.color;  }

    public setColor(color: String) : Void {
        this.color = color;
    }
}
```

- `new` yok — `Circle(...)` veya `Circle.create(...)`
- `override` keyword — opsiyonel, metodu üst sınıf metodunu override ediyor olarak işaretler (kaydedilir, şu an enforce edilmiyor)
- `constructor` açık anahtar kelime
- `super(...)` — üst sınıf constructor'ını çağırır
- Field'lara varsayılan değer verilebilir: `private color : String = "red";`

---

## 9. Abstract Class

```arimo
public abstract class Shape implements Drawable {

    private readonly color : String;

    protected constructor(color: String) {
        this.color = color;
    }

    public getColor() : String { return this.color; }

    public abstract draw() : Void;
    public abstract area() : Float;
}
```

---

## 10. Interface

```arimo
interface Drawable {
    draw() : Void;
    area() : Float;
}

// Default method — implementing class override etmek zorunda değil
interface Updatable {
    update(dt: Float) : Void;

    default updateFixed() : Void {
        Float dt = 0.016;
        IO.print("fixed update");
    }
}
```

- `public` yazılmaz — her method zaten public
- `default` keyword ile gövdeli method tanımlanabilir
- Bir class birden fazla interface implement edebilir

---

## 11. Struct (Value Type)

```arimo
public struct Vec3 {
    x : Float;
    y : Float;
    z : Float;

    @ForceInline
    public operator +(other: Vec3) : Vec3 {
        return Vec3(this.x + other.x, this.y + other.y, this.z + other.z);
    }

    public operator ==(other: Vec3) : Boolean {
        return this.x == other.x && this.y == other.y && this.z == other.z;
    }

    public dot(other: Vec3) : Float {
        return this.x * other.x + this.y * other.y + this.z * other.z;
    }
}

// Kullanım — auto-constructor (field sırası)
Vec3 pos = Vec3(0.0, 1.0, 0.0);
Vec3 vel = Vec3(1.0, 0.0, 0.0);
Vec3 sum = pos + vel;    // operator overloading
```

- Stack-allocated, copy semantics, `extends` yok
- Auto-constructor field sırasından üretilir
- `implements InterfaceName` ile interface implement edebilir
- BorrowChecker: struct copy type, move takibi yok

---

## 12. Enum

```arimo
// Saf variant'lar
public enum Priority {
    Low, Medium, High, Critical;

    public isUrgent() : Boolean {
        return this == Priority.High || this == Priority.Critical;
    }

    public label() : String {
        switch (this) {
            case Priority.Low:      return "Low";
            case Priority.Medium:   return "Medium";
            case Priority.High:     return "High";
            case Priority.Critical: return "Critical";
        }
    }
}

// Veri taşıyan variant'lar
public enum Shape {
    Circle(Float),
    Rectangle(Float, Float),
    Point;
}

// Pattern matching
match shape {
    Shape.Circle(r)       => IO.print("circle r=${r}"),
    Shape.Rectangle(w, h) => IO.print("rect ${w}x${h}"),
    Shape.Point           => IO.print("point"),
    _                     => IO.print("other"),
}
```

---

## 13. Exception

```arimo
public class TaskNotFoundException extends Exception {

    private readonly taskId : String;

    public constructor(taskId: String) {
        super("Task not found: ${taskId}");
        this.taskId = taskId;
    }

    public getTaskId() : String { return this.taskId; }
}
```

---

## 14. Union (Systems)

```arimo
// CPU register overlapping — OS / embedded için
public union Register {
    full  : u32;
    bytes : Array<u8, 4>;
}
```

- Field'lar aynı bellek adresini paylaşır
- `@ManualMemory` context'inde kullanım önerilir

---

## 15. Generics

```arimo
// Parametreli class
public class Pair<First, Second> {
    private readonly first  : First;
    private readonly second : Second;

    public static of(first: First, second: Second) : Pair<First, Second> {
        return Pair(first, second);
    }

    public getFirst()  : First  { return this.first;  }
    public getSecond() : Second { return this.second; }
}

// Bound — T, Drawable interface'ini implement etmeli
public class Renderer<T: Drawable> {
    public render(item: T) : Void {
        item.draw();
    }
}

// Çoklu bound
public class Sorter<T: Comparable + Printable> { ... }
```

---

## 16. Type Alias

```arimo
type NodeId   = u32;
type ByteFlag = u8;
type Callback = (String) -> Void;
```

---

## 17. Result<T, E>

```arimo
// Yerleşik generic enum
Result<String, Integer> r = Result.Ok("success");
Result<String, String>  e = Result.Err("not found");

if (r.isOk()) {
    IO.print("ok");
}
```

---

## 18. Koleksiyonlar

```arimo
// List — üç oluşturma yolu
List<Task>   tasks = List();                        // boş
List<String> names = List.of("Alice", "Bob");       // başlangıç değerleriyle
List<Task>   empty = List.empty();                  // boş (alternatif)

tasks.append(task);
tasks.length();
tasks.isEmpty();
tasks.filter((task)   -> task.isDone());
tasks.sortedBy((a, b) -> a.getTitle().compareTo(b.getTitle()));
tasks.take(5);
tasks.takeLast(5);
tasks.reduce(0, (sum, item) -> sum + item.getCount());

// HashMap — sırasız
Map<String, Integer> scores = HashMap();
Map<String, Integer> preset = HashMap.of("alice", 100, "bob", 90);  // veya
Map<String, Integer> alt    = HashMap.create();   // alternatif factory

scores.set("alice", 100);
Integer? val = scores.get("alice");              // nullable döndürür!
Integer  v   = scores.getOrDefault("bob", 0);   // null-safe
scores.containsKey("alice");
scores.remove("alice");
scores.keys();
scores.values();
scores.entries();
scores.length();

// TreeMap — key'e göre sıralı
Map<String, Integer> sorted = TreeMap();
Map<String, Integer> sortedAlt = TreeMap.create();

// Array — compile-time boyut, stack
Array<Float, 4> vec = Array.zeroed();
vec[0] = 1.0;              // index ile yaz
Float f = vec[0];          // index ile oku
vec.get(0);                // alternatif getter
vec.set(0, 1.0);           // alternatif setter
Integer len  = vec.length();
Slice<Float> view  = vec.asSlice();   // Slice'a dönüştür
Slice<Float> view2 = vec.slice();     // alternatif

// Slice — non-owning view
Slice<Float> s = Slice.of(rawPtr, count);  // pointer'dan oluştur
s.length();
Float elem = s[0];   // index ile oku
s.get(0);            // alternatif
s.set(0, 1.0);       // alternatif setter

// Pair — iki yol
Pair<String, Integer> pair  = Pair.of("score", 100);   // factory
Pair<String, Integer> pair2 = Pair("score", 100);       // direkt constructor
pair.getFirst();
pair.getSecond();
```

---

## 19. Kontrol Akışı

### if / else

```arimo
if (total > 10) {
    IO.print("Large");
} else if (total > 5) {
    IO.print("Medium");
} else {
    IO.print("Small");
}
```

### Ternary

```arimo
String label = isUrgent ? "urgent" : "normal";
```

### switch

```arimo
// break yok — her case direkt döner
switch (priority) {
    case Priority.Low:      return "Low";
    case Priority.High:     return "High";
    case Priority.Critical: return "Critical";
}
```

### match (pattern matching — expression)

`match` bir **expression**'dır, statement değil. Değer döndürür veya statement olarak kullanılır.

```arimo
// Statement olarak
match shape {
    Shape.Circle(r)       => IO.print("r=${r}"),
    Shape.Rectangle(w, h) => IO.print("${w}x${h}"),
    _                     => IO.print("other"),
}

// Expression olarak — atama veya return
String desc = match shape {
    Shape.Circle(r)       => "circle",
    Shape.Rectangle(w, h) => "rect",
    _                     => "other",
};
```

### while

```arimo
while (count > 0) {
    count--;
}
```

### for-each

```arimo
for (Task task : this.tasks) {
    IO.print(task.getTitle());
}
```

### klasik for

```arimo
for (Integer i = 0; i < 10; i++) {
    IO.print("${i}. adım");
}
```

### break / continue

```arimo
for (Integer i = 0; i < 10; i++) {
    if (i == 5) { break; }     // döngüden çık
    if (i == 3) { continue; }  // sonraki iterasyona geç
    IO.print("${i}");
}
```

### try / catch / finally

Birden fazla catch bloğu desteklenir:

```arimo
try {
    Task task = repo.findById(id);
    task.complete();
} catch (TaskNotFoundException ex) {
    IO.print("Not found: ${ex.message()}");
} catch (InvalidTaskException ex) {
    IO.print("Invalid: ${ex.message()}");
} finally {
    IO.print("done.");
}
```

### defer

```arimo
// Scope çıkışında çalışır — exception olsa bile
public static openFile(path: String) : Void {
    File f = File.open(path);
    defer f.close();
    // ...
}
```

---

## 20. Lambda

```arimo
// Tek parametre
this.tasks.filter((task) -> task.isDone());

// İki parametre
this.tasks.sortedBy((a, b) -> a.getTitle().compareTo(b.getTitle()));

// Zincir — LINQ tarzı
this.tasks
    .filter((task)   -> task.isUrgent() && !task.isDone())
    .sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()))
    .take(5);
```

---

## 21. Function Pointer

```arimo
// Tip tanımı: (ParamTipler) -> DönüşTipi
(Integer) -> Boolean isPositive = (x) -> x > 0;
(Integer, Integer) -> Integer add = (a, b) -> a + b;

// Lambda → FnPtr uyumlu
Boolean result = isPositive(42);
Integer sum    = add(1, 2);

// Parametre olarak geçirme
List<Integer> pos = nums.filter(isPositive);
```

---

## 22. Operatörler

```arimo
// Aritmetik
+ - * / %

// Karşılaştırma
== != < <= > >=

// Mantıksal
&& ||

// Bitwise
& | ^ ~ << >>

// Atama
= += -= *= /=

// Artırma / azaltma
++ --

// Diğer
?:   // ternary
?.   // null-safe erişim
as   // explicit tip cast: value as u32
[]   // index: arr[i]

// Operator overloading (user-defined)
public operator +(other: Vec3) : Vec3 { ... }
public operator ==(other: Vec3) : Boolean { ... }
public operator [](index: Integer) : Float { ... }
```

---

## 23. Type Cast

```arimo
u32 word  = 0xDEADBEEF;
u8  low   = word as u8;      // explicit cast
i32 signed = word as i32;

// Implicit widening YOK
// u64 x = some_u32;  → HATA
```

---

## 24. Bitwise İşlemler

```arimo
u32 flags   = 0xFF00FF00;
u8  mask    = 0b10101010;

u32 andOp   = flags & 0xFF;         // AND
u32 orOp    = flags | 0x00FF;       // OR
u32 xorOp   = flags ^ 0xAAAA;       // XOR
u8  notOp   = ~mask;                // NOT (prefix)
u32 shlOp   = flags << 4;           // left shift
u32 shrOp   = flags >> 2;           // right shift
```

---

## 25. volatile

```arimo
// Memory-mapped I/O için
volatile u32 status = hardware_register.read();
```

- `@ManualMemory` class veya `@Freestanding` module'de kullanım önerilir
- CodeGen: LLVM `volatile load/store`

---

## 26. Inline Assembly

```arimo
@ManualMemory
public class Syscall {
    public static exit(code: i32) : noreturn {
        asm {
            mov rax, 60
            syscall
        }
    }
}
```

- Sadece `@ManualMemory` class'larda geçerli
- `asm { }` bloğu içinde ham assembly kodu

---

## 27. C FFI

```arimo
extern "C" {
    printf(fmt: RawPtr<u8>, ...) : i32;
    malloc(size: u64)            : RawPtr<Void>;
    free(ptr: RawPtr<Void>)      : Void;
}
```

- `...` — variadic parametre
- TypeChecker extern bloğu kayıt eder

---

## 28. async / await

```arimo
public class ApiService {

    public async fetchUser(id: String) : String {
        String result = await ApiService.getData(id);
        return result;
    }

    public static async getData(id: String) : String {
        return "data";
    }
}
```

- `async` — method modifier
- `await` — expression prefix, inner type'ı döndürür
- CodeGen: state machine transform (Faza 4'te implement edilecek)

---

## 29. Entry Point

```arimo
public class Application {

    public static readonly NAME    : String = "Arimo";
    public static readonly VERSION : String = "1.4.0";

    public static main() : Void {
        IO.print("${Application.NAME} v${Application.VERSION}");
    }
}
```

- `static main()` olan class entry point
- `arc Application.arm` ile derlenir

---

## 30. String Metodları

```arimo
String s = "Merhaba Dünya";

s.length();                   // → Integer
s.compareTo("other");         // → Integer  (-1 / 0 / 1)
s.contains("Dünya");          // → Boolean
s.startsWith("Mer");          // → Boolean
s.endsWith("ya");             // → Boolean
s.toUpper();                  // → String  ("MERHABA DÜNYA")
s.toLower();                  // → String  ("merhaba dünya")
s.trim();                     // → String  (baştan/sondan boşluk siler)
s.split(" ");                 // → List<String>  (["Merhaba", "Dünya"])
```

---

## 31. Stdlib

```arimo
IO.print("mesaj");
IO.read();

Math.sqrt(16.0);
Math.abs(-5.0);
Math.pow(2.0, 8.0);
Math.PI;
Math.E;

Time.now();
Time.generateId();

Memory.alloc(256 * u8.sizeOf());
Memory.free(ptr);
Memory.copy(dst, src, n);
Memory.set(ptr, 0, n);       // belleği sıfırla

RawPtr<u8> ptr = Memory.alloc(1024);
ptr.read(0);
ptr.write(0, 255 as u8);
ptr.offset(4);

Integer.sizeOf();
Float.sizeOf();
u32.sizeOf();
```

---

## 31. Annotation'lar

### Bellek Yönetimi

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@ManualMemory` | class | GC/ARC kapalı, bellek kullanıcıda |

### Optimizasyon

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@ForceInline` | method | Her zaman inline et |
| `@Pure` | method | Yan etki yok, same input → same output |

### Bellek Layout

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@Packed` | struct | Field'lar arasında padding yok |
| `@Align(N)` | struct | N byte sınırına hizala |

### Bağlama (Linking)

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@Freestanding` | module | Stdlib yok, bare-metal |
| `@Section("isim")` | method | Linker section belirt |
| `@CallingConvention("C")` | method | C calling convention |
| `@CallingConvention("Windows")` | method | Windows stdcall |
| `@CallingConvention("Interrupt")` | method | CPU interrupt handler |

### Branch Tahmini

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@Likely` | if | Bu dal büyük ihtimalle çalışır |
| `@Unlikely` | if | Bu dal nadiren çalışır |

### API Kalitesi

| Annotation | Seviye | Açıklama |
|---|---|---|
| `@Deprecated("mesaj")` | class/struct/enum/method | Kullanıldığında uyarı |
| `@Experimental` | class/struct/enum/method | Kararsız API, değişebilir |
| `@FunctionalInterface` | interface | Tam 1 abstract method zorunlu |
| `@Throws(ExType, ...)` | method | Fırlatılabilecek exception'ları belgele |
| `@SuppressWarnings("tip")` | class/method | Belirli uyarıyı sustur |
| `@Sealed` | class/interface | Sadece aynı modülden extend edilebilir |
| `@Immutable` | class | Tüm instance field'lar `readonly` zorunlu |

### Kullanım Örnekleri

```arimo
@Freestanding
module kernel.boot;

@Packed
public struct PacketHeader {
    magic   : u16;
    version : u8;
    flags   : u8;
}

@Align(16)
public struct SimdVec {
    data : Array<Float, 4>;
}

@ManualMemory
public class Syscall {
    @Section(".init")
    @CallingConvention("Interrupt")
    public static handler() : Void { ... }

    @ForceInline
    public static fast() : Void { ... }

    @CallingConvention("C")
    public static cCompatible() : Void { ... }
}

if @Likely (x > 0) { ... }
if @Unlikely (err != null) { ... }

@Deprecated("NewService kullan")
public class OldService { ... }

@Experimental
public class GpuCompute { ... }

@FunctionalInterface
interface Runnable {
    run() : Void;
}

@Immutable
public class Point {
    private readonly x : Float;
    private readonly y : Float;
    public constructor(x: Float, y: Float) {
        this.x = x;
        this.y = y;
    }
}

@Sealed
public abstract class Shape {
    public abstract area() : Float;
}

public class FileReader {
    @Throws(IOException, NetworkError)
    public static read(path: String) : String { ... }
}

public class LegacyCode {
    @SuppressWarnings("deprecated")
    public static useOld() : Void { ... }
}

public class MathUtils {
    @Pure
    public static square(x: Float) : Float { return x * x; }
}
```

---

## 32. Ownership — Kullanıcı Görmez

```arimo
// Kullanıcı sadece şunu yazar:
Task task = repo.findById(id);
service.process(task);

// Derleyici arka planda halleder:
// — task'ın sahibi kim?
// — process() ödünç mü alıyor?
// — GC yok, bellek derleme zamanında yönetilir
// — & ve mut kullanıcıya gösterilmez
```

### 3 Katmanlı Bellek Yönetimi

```
Katman 1 — BorrowChecker Zone (compile-time):
  Tek sahiplik kanıtlanırsa → scope çıkışında free
  Runtime overhead: sıfır

Katman 2 — ARC (Reference Counting):
  Değer paylaşılıyorsa → refcount++ / refcount--
  Runtime overhead: küçük

Katman 3 — GC:
  Döngüsel referans → GC heap
  Runtime overhead: var ama küçük heap
```

### @ManualMemory

```arimo
@ManualMemory
public class Renderer {
    private vertexBuffer : RawPtr<Float>;

    public constructor() {
        this.vertexBuffer = Memory.alloc(1024 * Float.sizeOf());
    }

    public dispose() : Void {
        Memory.free(this.vertexBuffer);
    }
}
```

---

## 33. Hızlı Karşılaştırma

| Özellik | Arimo | Java | TypeScript |
|---|---|---|---|
| Tip ayracı | `name : String` | `String name` | `name: string` |
| Constructor | `constructor` | sınıf adı | `constructor` |
| Değişmezlik | `readonly` | `final` | `readonly` |
| Null güvenliği | `String?` | — | `string \| null` |
| String interpolation | `${name}` | — | `${name}` |
| Interface erişim | yazılmaz | opsiyonel | yazılmaz |
| new keyword | yok | zorunlu | zorunlu |
| Override belirteci | `override` (opsiyonel) | `@Override` | `override` |
| Entry point | `static main()` | `static void main(String[])` | — |
| GC | yok (3 katman) | var | var |
| Koleksiyon | `List()` `HashMap()` | `new ArrayList<>()` | `[]` `{}` |
| Systems | asm, extern, volatile | — | — |
| SIMD | `Vec4f`, `Vec8f` | — | — |
| Annotation stili | `@Immutable` | `@Immutable` | `@immutable` |

---

## 34. Tam Örnek — Task.arm

```arimo
module arimo.task.model;

import arimo.task.exception;

public class Task {

    private readonly id          : String;
    private readonly title       : String;
    private          description : String;
    private          status      : TaskStatus;
    private          priority    : Priority;
    private readonly createdAt   : String;

    public constructor(
        id          : String,
        title       : String,
        description : String,
        priority    : Priority
    ) {
        this.id          = id;
        this.title       = title;
        this.description = description;
        this.status      = TaskStatus.Todo;
        this.priority    = priority;
        this.createdAt   = Time.now();
    }

    public static create(
        title       : String,
        description : String,
        priority    : Priority
    ) : Task {
        return Task(Time.generateId(), title, description, priority);
    }

    public getId()          : String     { return this.id;          }
    public getTitle()       : String     { return this.title;       }
    public getDescription() : String     { return this.description; }
    public getStatus()      : TaskStatus { return this.status;      }
    public getPriority()    : Priority   { return this.priority;    }
    public getCreatedAt()   : String     { return this.createdAt;   }

    public isDone()   : Boolean { return this.status == TaskStatus.Done; }
    public isUrgent() : Boolean { return this.priority.isUrgent();       }

    public complete() : Void {
        if (this.isDone()) {
            throw InvalidTaskException("Task is already completed.");
        }
        this.status = TaskStatus.Done;
    }

    public toString() : String {
        return "[${this.status}] ${this.title}  (${this.priority.label()})";
    }
}
```

---

## 35. Bilinen Sınırlar (v1.4)

| Sınır | Açıklama |
|---|---|
| Lambda tip çıkarımı | Parametreler `Unknown` tipte — false positive hatalar bastırılıyor |
| `arr[i] = val` | TypeChecker'da assignment hedefi olarak pass-through |
| Generic instantiation | Yüzeysel: `Result<T,E>` çalışıyor ama tam tip doğrulama yok |
| Interface generic param | `interface Foo<T> { m(v: T) }` → TypeChecker T'yi scope'a almıyor |
| match exhaustiveness | Sealed type veya enum için kapsama kontrolü henüz yok |
| BorrowChecker method args | Borrow sayıyor (move değil) — false negative olabilir |
| async/await | Parse edilip AST'de saklanıyor; CodeGen state machine henüz yok |
| @Pure enforcement | Kaydedilir; mutasyon kontrolü CodeGen'de |
| ARC / GC | CodeGen'de henüz implement edilmedi |
