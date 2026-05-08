# Arimo Lang — Proje El Teslim Belgesi

## Proje Özeti
Arimo Lang, TypeScript, Java ve C#'ın güçlü yanlarını bir araya getiren, GC'siz (ownership tabanlı), statik tipli, OOP odaklı, native binary üreten yeni bir programlama dili. Compiler adı `arc`, kaynak dosya uzantısı `.arm`.

**Hedef:** Java/TypeScript/C# kullanıcılarına daha temiz, daha az boilerplate, ama aynı güçte bir alternatif sunmak. GC yok — Rust'ın ownership prensiplerine dayanıyor ama `&` ve `mut` kullanıcıya gösterilmiyor, derleyici arka planda hallediyor.

---

## Tamamlanan Milestone'lar

### ✅ Milestone 1 — Lexer
`src/lexer/mod.rs` — Tüm Arimo token'larını tanıyan lexer. String interpolation `${}` destekli.

### ✅ Milestone 2 — Parser & AST
`src/parser/mod.rs` + `src/ast/mod.rs` — Pratt parser. 11/11 test dosyası parse OK.

### ⏳ Milestone 3 — Type Checker (SIRADA)
### ⏳ Milestone 4 — Borrow Checker
### ⏳ Milestone 5 — LLVM Kod Üretimi
### ⏳ Milestone 6 — Standart Kütüphane

---

## Proje Yapısı

```
arimo/
├── Cargo.toml
├── src/
│   ├── main.rs              ← CLI: arc <file.arm>
│   ├── lexer/mod.rs         ✅ tamamlandı
│   ├── ast/mod.rs           ✅ tamamlandı
│   ├── parser/mod.rs        ✅ tamamlandı
│   ├── typechecker/mod.rs   ⏳ sadece stub
│   ├── borrow/mod.rs        ⏳ sadece stub
│   └── codegen/mod.rs       ⏳ sadece stub
└── tests/samples/
    ├── hello.arm
    └── ownership.arm
```

**Cargo.toml:**
```toml
[package]
name = "arimo"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "arc"
path = "src/main.rs"
```

---

## Dil Spesifikasyonu v1.3

### 1. Module Sistemi
```arimo
module arimo.shop.model;       // dosya başı, zorunlu
import arimo.shop.exception;   // bağımlılık
```
- Bir dosya = bir public class
- Dosya adı = class adı → Task.arm = public class Task
- module klasör yapısıyla birebir örtüşür

### 2. Tipler
```
Integer  Float  Boolean  String  Void
List<T>
Map<K,V>        // interface
HashMap<K,V>    // hash tabanlı, sırasız
TreeMap<K,V>    // key'e göre sıralı
Pair<First,Second>
```
- Tüm tipler büyük harfle başlar
- Tuple yok — Pair<A,B> kullan
- new keyword yok

### 3. Tip Ayracı — Her Yerde Aynı Kural
```arimo
name    : String  = "Arimo";   // değişken
radius  : Float;               // field
area()  : Float   { }          // metod dönüş tipi
```

### 4. Null Güvenliği
```arimo
String  name = "Arimo";     // null olamaz
String? name = null;        // nullable

// Smart cast
String? title = task.getTitle();
if (title != null) {
    IO.print(title);        // burada String, String? değil
}

// Null-safe erişim
String? name = user?.getName();
```

### 5. String Interpolation
```arimo
IO.print("Merhaba ${name}!");
IO.print("Toplam: ${a + b}");
IO.print("Görev: ${task.getTitle()}");
```
- `+` sadece sayı toplama için
- String birleştirme = `${}`

### 6. Erişim Belirteçleri
```
public      // her yerden
private     // sadece bu class
protected   // bu class + alt sınıflar
internal    // sadece aynı module
readonly    // bir kez atanır, değişmez
static      // class seviyesi
```
- Class içinde her field/metodda zorunlu
- Interface içinde yazılmaz (zaten public)

### 7. Class
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
- `new` yok — Circle(...) veya Circle.create(...)
- `@Override` yok — derleyici anlar
- `constructor` açık anahtar kelime

### 8. Interface
```arimo
interface Drawable {
    draw()  : Void;
    area()  : Float;
}
```
- `public` yazılmaz — hepsi zaten public
- Sadece imza, gövde yok

### 9. Abstract Class
```arimo
public abstract class Shape implements Drawable {
    private readonly color : String;

    protected constructor(color: String) {
        this.color = color;
    }

    public abstract draw() : Void;
    public abstract area() : Float;
}
```

### 10. Enum
```arimo
public enum Priority {
    Low, Medium, High, Critical;

    public isUrgent() : Boolean {
        return this == Priority.High || this == Priority.Critical;
    }
}
```

### 11. Exception
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

### 12. Generics
```arimo
public class Pair<First, Second> {
    private readonly first  : First;
    private readonly second : Second;

    public static of(first: First, second: Second) : Pair<First, Second> {
        return Pair(first, second);
    }
}

// Kullanım
Pair<String, Integer>  pair   = Pair.of("score", 100);
List<Task>             tasks  = List();
Map<String, Integer>   scores = HashMap();
Map<String, Integer>   sorted = TreeMap();
```

### 13. Koleksiyonlar
```arimo
List<Task> tasks = List();
List<String> names = List.of("Alice", "Bob");
tasks.append(task);
tasks.length();
tasks.isEmpty();
tasks.filter((task) -> task.isDone());
tasks.sortedBy((a, b) -> a.getTitle().compareTo(b.getTitle()));
tasks.take(5);
tasks.reduce(Money.zero(), (sum, item) -> sum.add(item.getPrice()));

Map<String, Integer> scores = HashMap();
scores.set("alice", 100);
scores.get("alice");
scores.getOrDefault("bob", 0);
scores.containsKey("alice");
scores.values();
scores.entries();
```

### 14. Kontrol Akışı
```arimo
// if/else
if (total > 10) {
    IO.print("Large");
} else if (total > 5) {
    IO.print("Medium");
} else {
    IO.print("Small");
}

// ternary — sadece tek satır, iç içe yasak
String label = isUrgent ? "urgent" : "normal";

// switch — break yok
switch (priority) {
    case Priority.Low:      return "Low";
    case Priority.High:     return "High";
    case Priority.Critical: return "Critical";
}

// while
while (count > 0) { count--; }

// for-each
for (Task task : this.tasks) {
    IO.print(task.getTitle());
}

// klasik for
for (Integer i = 0; i < 10; i++) {
    IO.print("${i}");
}

// try/catch/finally
try {
    Task task = repo.findById(id);
} catch (TaskNotFoundException exception) {
    IO.print("Caught: ${exception.message()}");
} finally {
    IO.print("done.");
}
```

### 15. Lambda
```arimo
tasks.filter((task) -> task.isDone());
tasks.sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()));
tasks
    .filter((task)   -> task.isUrgent() && !task.isDone())
    .sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()))
    .take(5);
```

### 16. Entry Point
```arimo
public class Application {
    public static main() {   // dönüş tipi yazılmaz — sadece main() istisnası
        // burası başlar
    }
}
```
- `arc Application.arm` ile çalıştırılır
- Diğer tüm metodlarda dönüş tipi zorunlu, Void dahil

### 17. Ownership — Kullanıcı Görmez
- GC yok
- `&` ve `mut` kullanıcıya gösterilmez
- arc arka planda ownership inference yapar
- Bellek derleme zamanında yönetilir

### 18. Standart Kütüphane Token'ları
```arimo
IO.print("...");           // çıktı
IO.read();                 // girdi
Math.sqrt(x);              // matematik
Math.PI;
Time.now();                // zaman
Time.generateId();         // UUID üretimi
```

---

## Lexer Token Listesi

Temel kategoriler:
- **Literals:** Int, Float, Bool, Str, Ident
- **Module:** Module, Import
- **Access:** Public, Private, Protected, Internal
- **Class:** Class, Abstract, Interface, Enum, Extends, Implements
- **Modifiers:** Static, Readonly, Override
- **Special:** Constructor, Super, This
- **Built-in Types:** TypeInteger, TypeFloat, TypeBoolean, TypeString, TypeVoid, TypeList, TypeMap, TypeHashMap, TypeTreeMap, TypePair, TypeException
- **Stdlib:** StdIO, StdMath, StdTime
- **Control:** If, Else, While, For, Return, Break, Continue, Switch, Case, Throw, Try, Catch, Finally
- **Null:** Null
- **Operators:** Plus, Minus, Star, Slash, Percent, Eq, EqEq, Bang, BangEq, Lt, LtEq, Gt, GtEq, AndAnd, PipePipe, PlusPlus, MinusMinus, PlusEq, MinusEq, StarEq, SlashEq, Arrow(->), Question(?), QuestionDot(?.)
- **Delimiters:** LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Semicolon, Colon, Dot, DollarLBrace(${), InterpolEnd

---

## AST Node Yapısı

### Tipler
```
Type::Integer | Float | Boolean | Str | Void
Type::List(Box<Type>)
Type::Map/HashMap/TreeMap(Box<Type>, Box<Type>)
Type::Pair(Box<Type>, Box<Type>)
Type::Named(String)
Type::Nullable(Box<Type>)
Type::Generic(String, Vec<Type>)
```

### Expression'lar
```
Expr::IntLit(i64) | FloatLit(f64) | BoolLit(bool) | StrLit(String) | NullLit
Expr::StrInterp(Vec<StringPart>)   // ${} interpolation
Expr::Ident(String)
Expr::This | Super
Expr::FieldAccess { object, field }
Expr::NullSafeAccess { object, field }
Expr::MethodCall { object, method, args }
Expr::StaticCall { class, method, args }
Expr::ConstructorCall { class, args }
Expr::BinOp { op, left, right }
Expr::UnaryOp { op, expr }
Expr::Ternary { cond, then, else_ }
Expr::Lambda { params, body }
Expr::Index { object, index }
```

### Statement'lar
```
Stmt::VarDecl { ty, name, value }
Stmt::ExprStmt(Expr)
Stmt::Return(Option<Expr>)
Stmt::Throw(Expr)
Stmt::If { cond, then, else_if, else_ }
Stmt::While { cond, body }
Stmt::ForEach { ty, name, iter, body }
Stmt::For { init, cond, step, body }
Stmt::Switch { expr, cases }
Stmt::TryCatch { try_body, catches, finally_body }
Stmt::Break | Continue
Stmt::Block(Vec<Stmt>)
```

### Üst Düzey
```
Module { path, imports, items }
Item::Class(ClassDecl)
Item::Interface(InterfaceDecl)
Item::Enum(EnumDecl)
Item::Exception(ExceptionDecl)

ClassDecl { visibility, abstract_, name, generics, extends, implements, fields, constructor, methods }
InterfaceDecl { name, generics, methods }
EnumDecl { visibility, name, variants, methods }
Field { visibility, readonly, static_, name, ty, value }
Method { visibility, static_, abstract_, override_, name, params, return_ty, body }
Constructor { visibility, params, body }
```

---

## Parser Notları

### Önemli Kararlar
1. **Parametre sözdizimi:** `name: Type` (Java'dan farklı — isim önce, tip sonra)
2. **Field sözdizimi:** `private readonly name : String;` (Ident + Colon + Type)
3. **Var decl tespiti:** `is_var_decl()` — Ident + Ident ise var decl, Ident + Dot ise expr stmt
4. **main() dönüş tipi yok:** Özel durum — sadece `public static main() {}`
5. **super():** `StaticCall { class: "super", method: "__constructor__" }` olarak parse edilir
6. **Built-in tipler static call yapabilir:** `List.empty()`, `HashMap.create()`, `Exception("msg")`

### Pratt Parser Operatör Öncelikleri
```
Assign(=, +=, -=, *=, /=)  : 1-2
Or(||)                      : 3-4
And(&&)                     : 5-6
Eq/Ne(==, !=)               : 7-8
Compare(<, <=, >, >=)       : 9-10
Add/Sub(+, -)               : 11-12
Mul/Div/Mod(*, /, %)        : 13-14
```

---

## Test Sonuçları (11/11 ✓)

```
✓ TaskApplication.arm
✓ InvalidTaskException.arm
✓ TaskNotFoundException.arm
✓ Priority.arm
✓ Project.arm
✓ Tag.arm
✓ Task.arm
✓ ProjectRepository.arm
✓ TaskRepository.arm
✓ TaskService.arm
✓ BankingSystem.arm
```

Test komutu:
```powershell
.\target\debug\arc.exe tests\samples\hello.arm
```

Beklenen çıktı:
```
arc: parsing 'hello.arm'
arc: parse OK
Module   : arimo.hello
Imports  : ["arimo.io"]
Items    : 1
  class HelloWorld {
    fields      : 0
    constructor : false
    methods     : 1
      static main() : —
  }
```

---

## Sıradaki Adım — Milestone 3: Type Checker

### Ne Yapacak
1. **Sembol tablosu** — hangi class, metod, field var
2. **Tip çıkarımı** — her expression'ın tipini belirle
3. **Tip kontrolü** — uyumsuzlukları yakala
4. **Referans kontrolü** — tanımsız değişken/metod yok

### Örnek Hata Mesajları Hedefi
```
arc: type error at Task.arm:42
  cannot assign Integer to String
  expected: String
  found:    Integer

arc: type error at OrderService.arm:18
  method 'findByName' not found on type 'ProductRepository'

arc: type error at Circle.arm:7
  cannot assign null to non-nullable type String
  hint: use String? for nullable types
```

### Mimari Plan
```rust
// src/typechecker/mod.rs

pub struct SymbolTable {
    classes  : HashMap<String, ClassDecl>,
    // ...
}

pub struct TypeChecker {
    symbols  : SymbolTable,
    errors   : Vec<TypeError>,
}

impl TypeChecker {
    pub fn check(&mut self, module: &Module) -> Vec<TypeError>
    fn check_class(&mut self, class: &ClassDecl)
    fn check_method(&mut self, method: &Method)
    fn infer_expr(&mut self, expr: &Expr) -> Type
    fn check_stmt(&mut self, stmt: &Stmt)
}
```

---

## VSCode Extension

`arimo-lang-0.2.0.vsix` — kurulum:
```
Ctrl+Shift+X → ··· → Install from VSIX → dosyayı seç
```
Tema: Ctrl+K Ctrl+T → "Arimo Dark"

---

## Teknoloji Kararları

| Karar | Seçim | Neden |
|---|---|---|
| Compiler dili | Rust | GC yok, ownership, LLVM binding |
| Hedef | Native binary | LLVM via inkwell |
| Tip sistemi | Statik | Derleme zamanı güvenliği |
| Bellek | Ownership (gizli) | GC yok, kullanıcı & görmez |
| Bootstrapping | Hedef | arc'ı Arimo ile yeniden yaz |

---

## Önemli Notlar

- `new` keyword yok — `Task(...)` veya `Task.create(...)`
- `@Override` yok — derleyici anlar
- `void main(String[] args)` yok — sadece `public static main()`
- `let` yok — tip direkt yazılır: `String name = "Arimo";`
- `in` keyword yok — for-each `:` kullanır
- `=>` yok — lambda için sadece `->`
- `::` yok — method reference henüz yok
- Interface'de `public` yazılmaz
- Tuple yok — `Pair<A,B>` kullan
- `break` switch'te yok — her case direkt return
- `main()` dönüş tipi yazılmaz — tek istisna bu
