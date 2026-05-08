# Arimo Lang — Language Specification v1.0

## Module sistemi
```arimo
module arimo.shop.model;       // dosya başı, zorunlu
import arimo.shop.exception;   // bağımlılık
```
- Bir dosya = bir public class
- Dosya adı = class adı (ArimoShop.arm → public class ArimoShop)
- module klasör yapısıyla örtüşür

---

## Tipler
```arimo
Integer   Float   Boolean   String   Void
List<T>   Map<K,V>   Pair<A,B>
```
- Tüm tipler büyük harfle başlar
- Kullanıcı tanımlı tipler de büyük harfle: Point, Money, Order

---

## Tip ayracı — her yerde aynı kural
```arimo
name    : String  = "Arimo";   // değişken
radius  : Float;               // field
area()  : Float { }            // metod dönüş tipi
```
- `:` her yerde tip ayracı
- Tip her zaman sağda

---

## Class
```arimo
public class Circle extends Shape implements Drawable, Movable {

    private readonly id     : String;   // değişmez
    private readonly radius : Float;    // değişmez
    private          color  : String;   // değişebilir

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
- `public` / `private` / `protected` / `internal` — zorunlu
- `readonly` — bir kez atanır, değişmez
- `static` — class seviyesi
- `abstract` — soyut
- `constructor` — açık anahtar kelime
- `new` yok — `Circle.create(...)` veya `Circle(...)`

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
}
```

---

## Exception
```arimo
public class TaskNotFoundException extends Exception {

    private readonly taskId : String;

    public constructor(taskId: String) {
        super("Task not found: " + taskId);
        this.taskId = taskId;
    }

    public getTaskId() : String { return this.taskId; }
}
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

// switch — break yok
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
    IO.print(task.toString());
}

// klasik for
for (Integer i = 0; i < 10; i++) {
    IO.print(i);
}

// try / catch / finally
try {
    Task task = repo.findById(id);
} catch (TaskNotFoundException exception) {
    IO.print(exception.message());
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

## Generics
```arimo
public class Pair<First, Second> {
    private readonly first  : First;
    private readonly second : Second;

    public getFirst()  : First  { return this.first;  }
    public getSecond() : Second { return this.second; }
}

// Kullanım
Pair<String, Integer> pair = Pair.of("score", 100);
List<Task>            list = List.empty();
Map<String, Integer>  map  = Map.empty();
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
- `static main()` olan class entry point
- `arc Application.arm` ile çalıştırılır

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

## Örnek — tam class
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
}
```
