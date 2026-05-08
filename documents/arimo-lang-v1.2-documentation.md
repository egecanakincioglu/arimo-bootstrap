# Arimo Lang — Language Specification v1.1

---

## Module sistemi
```arimo
module arimo.shop.model;       // dosya başı, zorunlu
import arimo.shop.exception;   // bağımlılık
```
- Bir dosya = bir public class
- Dosya adı = class adı  →  `Task.arm` = `public class Task`
- module klasör yapısıyla birebir örtüşür

---

## Tipler
```arimo
Integer   Float   Boolean   String   Void
List<T>   Map<K,V>   HashMap<K,V>   TreeMap<K,V>   Pair<A,B>
```
- Tüm tipler büyük harfle başlar
- Kullanıcı tanımlı tipler de büyük harfle: `Point`, `Money`, `Task`
- Tuple yok — `Pair<A,B>` kullan

---

## Null güvenliği
```arimo
String  name = "Arimo";      // null olamaz — garanti
String? name = null;         // nullable — açıkça işaretlenmeli

// Smart cast — if bloğu içinde derleyici null olmadığını bilir
String? title = task.getTitle();
if (title != null) {
    IO.print(title);         // burada String, String? değil
}

// Null-safe erişim
String? name = user?.getName();   // user null ise name de null
Integer len  = name?.length();    // zincir
```

---

## Tip ayracı — her yerde aynı kural
```arimo
name    : String  = "Arimo";   // değişken
radius  : Float;               // field
area()  : Float { }            // metod dönüş tipi
```

---

## String interpolation
```arimo
IO.print("Merhaba ${name}!");
IO.print("Görev: ${task.getTitle()}  Öncelik: ${task.getPriority().label()}");
IO.print("Tamamlanma: %${project.getCompletionRate()}");
```
- `+` operatörü sadece sayı toplama için
- String birleştirme = `${}` içinde her expression çalışır

---

## Erişim belirteçleri
```arimo
public    // her yerden erişilebilir
private   // sadece bu class
protected // bu class + alt sınıflar
internal  // sadece aynı module (arimo.shop.model içindeki internal → sadece arimo.shop.model görür)
```
- Class içinde her field ve metodda zorunlu
- Interface içinde yazılmaz — zaten public

---

## readonly / static
```arimo
private readonly id    : String;    // bir kez atanır, değişmez
public  static   MAX   : Integer = 50;
public  static readonly VERSION : String = "1.0.0";
```

---

## Class
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

---

## Interface
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

---

## Abstract class
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

## Enum
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

## Exception
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

## Generics
```arimo
public class Pair<First, Second> {
    private readonly first  : First;
    private readonly second : Second;

    public static of(first: First, second: Second) : Pair<First, Second> {
        return Pair(first, second);
    }

    public getFirst()  : First  { return this.first;  }
    public getSecond() : Second { return this.second; }
}

// Kullanım
Pair<String, Integer>  pair = Pair.of("score", 100);
List<Task>             list = List.empty();
Map<String, Integer>   map  = Map.empty();
```

---

## Kontrol akışı
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

// switch — break yok, her case direkt return
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
    IO.print("${i}");
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

## Lambda
```arimo
// tek parametre
this.tasks.filter((task) -> task.isDone());

// iki parametre
this.tasks.sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()));

// zincir
this.tasks
    .filter((task)   -> task.isUrgent() && !task.isDone())
    .sortedBy((a, b) -> a.getDueDate().compareTo(b.getDueDate()))
    .take(5);
```

---

## Entry point
```arimo
public class Application {
    public static main() : Void {
        // burası başlar
    }
}
```
- `arc Application.arm` ile çalıştırılır
- `static main()` olan class entry point

---

## Ownership — kullanıcı görmez
```arimo
// Kullanıcı sadece bunu yazar
Task task = repo.findById(id);
service.process(task);

// arc arka planda halleder:
// — task'ın sahibi kim?
// — process() ödünç mü alıyor, tüketiyor mu?
// — GC yok, bellek derleme zamanında yönetilir
```

---

## Tam örnek
```arimo
module arimo.task.model;

import arimo.task.exception;

public class Task {

    private readonly id       : String;
    private readonly title    : String;
    private          status   : TaskStatus;
    private          priority : Priority;

    public constructor(id: String, title: String, priority: Priority) {
        this.id       = id;
        this.title    = title;
        this.status   = TaskStatus.Todo;
        this.priority = priority;
    }

    public static create(title: String, priority: Priority) : Task {
        return Task(Time.generateId(), title, priority);
    }

    public getId()       : String     { return this.id;       }
    public getTitle()    : String     { return this.title;    }
    public getStatus()   : TaskStatus { return this.status;   }
    public getPriority() : Priority   { return this.priority; }

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

## Koleksiyonlar

```arimo
// List
List<Task>  tasks = List.empty();
tasks.append(task);
tasks.length();
tasks.isEmpty();
tasks.filter((task) -> task.isDone());
tasks.sortedBy((a, b) -> a.getTitle().compareTo(b.getTitle()));
tasks.take(5);
tasks.takeLast(5);
tasks.reduce(Money.zero(), (total, item) -> total.add(item.getPrice()));

// Map — interface, somut implementasyon seç
Map<String, Integer>  scores = HashMap.create();   // hash tabanlı, sırasız
Map<String, Integer>  sorted = TreeMap.create();   // sıralı, key'e göre

scores.set("alice", 100);
scores.get("alice");
scores.getOrDefault("bob", 0);
scores.containsKey("alice");
scores.remove("alice");
scores.keys();
scores.values();
scores.entries();
scores.length();

// Pair — iki değer birlikte
Pair<String, Integer> pair = Pair.of("score", 100);
pair.getFirst();
pair.getSecond();
```
