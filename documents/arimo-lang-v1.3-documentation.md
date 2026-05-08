# Arimo Lang — Language Specification v1.3

> Statically typed, compiled, OOP language.  
> GC yok — ownership arka planda.  
> Compiler: `arc`  |  Source extension: `.arm`

---

## 1. Module sistemi

```arimo
module arimo.shop.model;       // dosya başı, zorunlu
import arimo.shop.exception;   // bağımlılık
import arimo.io;
```

- Bir dosya = bir `public class`
- Dosya adı = class adı → `Task.arm` = `public class Task`
- `module` klasör yapısıyla birebir örtüşür
- `arc TaskApplication.arm` ile derlenir

---

## 2. Tipler

```arimo
// Primitifler
Integer   Float   Boolean   String   Void

// Koleksiyonlar
List<T>
Map<K,V>              // interface
  HashMap<K,V>        // hash tabanlı, sırasız
  TreeMap<K,V>        // key'e göre sıralı

Pair<First, Second>   // iki değer, tuple yok
```

- Tüm tipler büyük harfle başlar
- Kullanıcı tanımlı tipler de büyük harfle: `Point`, `Money`, `Task`

---

## 3. Tip ayracı — her yerde aynı kural

```arimo
name      : String  = "Arimo";   // değişken
radius    : Float;               // field
area()    : Float   { }          // metod dönüş tipi
```

- `:` her yerde tip ayracı — değişkende, field'da, metodda hep aynı
- Tip her zaman sağda

---

## 4. Null güvenliği

```arimo
String  name = "Arimo";     // null olamaz — garanti
String? name = null;        // nullable — açıkça işaretlenmeli

// Smart cast — if bloğu içinde derleyici null olmadığını bilir
String? title = task.getTitle();
if (title != null) {
    IO.print(title);        // burada String, String? değil
}

// Null-safe erişim
String?  name = user?.getName();    // user null ise name de null
Integer? len  = name?.length();     // zincir
```

---

## 5. String interpolation

```arimo
IO.print("Merhaba ${name}!");
IO.print("Görev: ${task.getTitle()}  Öncelik: ${task.getPriority().label()}");
IO.print("Tamamlanma: %${project.getCompletionRate()}");
IO.print("Toplam: ${a + b} birim");
```

- `+` operatörü sadece sayı toplama
- String birleştirme = `${}` — içinde her expression çalışır

---

## 6. Erişim belirteçleri

```arimo
public      // her yerden erişilebilir
private     // sadece bu class
protected   // bu class + alt sınıflar
internal    // sadece aynı module
```

- Class içinde her field ve metodda **zorunlu**
- Interface içinde yazılmaz — zaten public
- `readonly` — bir kez atanır, değişmez
- `static`   — class seviyesi, nesne oluşturmadan erişilir

```arimo
private readonly id      : String;
public  static   MAX     : Integer = 50;
public  static readonly  VERSION : String = "1.0.0";
```

---

## 7. Class

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
- `@Override` yok — derleyici anlar
- `constructor` açık anahtar kelime
- `super(...)` üst sınıf constructor'ı çağırır

---

## 8. Interface

```arimo
interface Drawable {
    draw()  : Void;
    area()  : Float;
}

interface Movable {
    move(dx: Integer, dy: Integer) : Void;
    position() : Point;
}
```

- `public` yazılmaz — interface içi her zaman public
- Sadece imza, gövde yok
- Bir class birden fazla interface implement edebilir

---

## 9. Abstract class

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

## 10. Enum

```arimo
public enum Priority {
    Low,
    Medium,
    High,
    Critical;

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
```

---

## 11. Exception

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

## 12. Generics

```arimo
public class Pair<First, Second> {

    private readonly first  : First;
    private readonly second : Second;

    public constructor(first: First, second: Second) {
        this.first  = first;
        this.second = second;
    }

    public static of(first: First, second: Second) : Pair<First, Second> {
        return Pair(first, second);
    }

    public getFirst()  : First  { return this.first;  }
    public getSecond() : Second { return this.second; }
}

// Kullanım
Pair<String, Integer>  pair   = Pair.of("score", 100);
List<Task>             tasks  = List();
Map<String, Integer>   scores = HashMap();
Map<String, Integer>   sorted = TreeMap();
```

---

## 13. Koleksiyonlar

```arimo
// List
List<Task> tasks = List();
List<String> names = List.of("Alice", "Bob", "Carol");

tasks.append(task);
tasks.length();
tasks.isEmpty();
tasks.filter((task)   -> task.isDone());
tasks.sortedBy((a, b) -> a.getTitle().compareTo(b.getTitle()));
tasks.take(5);
tasks.takeLast(5);
tasks.reduce(Money.zero(), (sum, item) -> sum.add(item.getPrice()));

// HashMap — sırasız, hızlı
Map<String, Integer> scores = HashMap();
Map<String, Integer> scores = HashMap.of("alice", 100, "bob", 90);

scores.set("alice", 100);
scores.get("alice");
scores.getOrDefault("bob", 0);
scores.containsKey("alice");
scores.remove("alice");
scores.keys();
scores.values();
scores.entries();
scores.length();

// TreeMap — key'e göre sıralı
Map<String, Integer> sorted = TreeMap();
```

---

## 14. Kontrol akışı

```arimo
// if / else if / else
if (total > 10) {
    IO.print("Large");
} else if (total > 5) {
    IO.print("Medium");
} else {
    IO.print("Small");
}

// ternary — sadece tek satır, iç içe yasak
String label = isUrgent ? "urgent" : "normal";

// switch — break yok, her case direkt döner
switch (priority) {
    case Priority.Low:      return "Low";
    case Priority.High:     return "High";
    case Priority.Critical: return "Critical";
}

// while
while (count > 0) {
    count--;
}

// for-each
for (Task task : this.tasks) {
    IO.print(task.getTitle());
}

// klasik for
for (Integer i = 0; i < 10; i++) {
    IO.print("${i}. adım");
}

// try / catch / finally
try {
    Task task = repo.findById(id);
} catch (TaskNotFoundException exception) {
    IO.print("Caught: ${exception.message()}");
} finally {
    IO.print("done.");
}
```

---

## 15. Lambda

```arimo
// tek parametre
this.tasks.filter((task) -> task.isDone());

// iki parametre
this.tasks.sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()));

// zincir — LINQ tarzı
this.tasks
    .filter((task)   -> task.isUrgent() && !task.isDone())
    .sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()))
    .take(5);
```

---

## 16. Entry point

```arimo
public class TaskApplication {

    public static readonly NAME    : String = "Arimo Tasks";
    public static readonly VERSION : String = "1.0.0";

    public static main() : Void {
        IO.print("${TaskApplication.NAME}  v${TaskApplication.VERSION}");
        // burası başlar
    }
}
```

- `static main()` olan class entry point
- `arc TaskApplication.arm` ile çalıştırılır

---

## 17. Ownership — kullanıcı görmez

```arimo
// Kullanıcı sadece bunu yazar
Task task = repo.findById(id);
service.process(task);

// arc arka planda halleder:
// — task'ın sahibi kim?
// — process() ödünç mü alıyor, tüketiyor mu?
// — GC yok, bellek derleme zamanında yönetilir
// — & ve mut kullanıcıya gösterilmez
```

---

## 18. Tam örnek — Task.arm

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

## Hızlı referans

| Özellik | Arimo | Java | TypeScript |
|---|---|---|---|
| Tip ayracı | `name : String` | `String name` | `name: string` |
| Constructor | `constructor` | sınıf adı | `constructor` |
| Değişmezlik | `readonly` | `final` | `readonly` |
| Null güvenliği | `String?` | — | `string \| null` |
| String interpolation | `${name}` | — | `${name}` |
| Interface erişim | yazılmaz | opsiyonel | yazılmaz |
| new keyword | yok | zorunlu | zorunlu |
| Override belirteci | yok | `@Override` | `override` |
| Entry point | `static main()` | `static void main(String[])` | — |
| GC | yok | var | var |
| Koleksiyon | `List()` `HashMap()` | `new ArrayList<>()` | `[]` `{}` |
