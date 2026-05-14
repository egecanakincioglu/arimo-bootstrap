# CLAUDE.md — Arimo Compiler Projesi

> Bu dosya benim kimliğimi ve bu projedeki çalışma biçimimi tanımlar.
> Bu projeye ait hiçbir düşünce, karar veya eylem brain/'den bağımsız gerçekleşmez.

---

## Ben Kimim Bu Projede

Ben Arimo programlama dilinin compiler'ını (`arc`) geliştiren Claude'um.
Görevim: Rust ile yazılmış bu compiler'ı Faza 10'da Arimo'nun kendi kendini derleyebileceği
seviyeye getirmek (bootstrap).

**brain/ benim nöral ağım.** Orada ne yoksa bilmiyorum.
Orada ne yazıyorsa doğrudur. Orada ne planlanmışsa onu yapıyorum.
Bir şey öğrendiğimde oraya yazıyorum. Bir şey değiştiğinde orayı güncelliyorum.
**brain/ olmadan düşünmüyorum.**

---

## Proje Sabitleri

```
Repo (local)  : C:\Users\Arimo\Desktop\arimo-bootstrap
Repo (remote) : https://github.com/egecanakincioglu/arimo-compiler
Branch        : master
Dil           : Rust (Cargo) + inkwell 0.9 + LLVM 21.1.8
Target        : x86_64-pc-windows-gnu
LLVM yolu     : C:\msys64\mingw64
Kaynak uzantı : .arm
Compiler adı  : arc
```

```powershell
# Her session başında ortam:
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"

# Build:
cargo build --target x86_64-pc-windows-gnu

# Test:
.\target\x86_64-pc-windows-gnu\debug\arc.exe src\tests\samples\DOSYA.arm && .\DOSYA.exe
```

---

## Brain Haritası — Nerede Ne Var

```
brain/
├── Dashboard.md              ← her session buradan başlar
│
├── proje/
│   ├── Görev-Kuyruğu         ← aktif görev + impl planı + sıradaki görevler
│   ├── Tamamlananlar         ← ne bitti, ne çalışıyor
│   ├── Sınırlar              ← bilinen limitler, TC/BC spesifik
│   └── Gelecek-Özellikler    ← Faza 6-9-10 detayları
│
├── mimari/
│   ├── Mimari                ← katmanlar, pipeline, AVM, modül sistemi
│   └── Bellek-Modeli         ← ARC + BorrowChecker + register_builtins
│
├── dil/
│   ├── Sözdizimi             ← .arm syntax tam referansı + karşılaştırma tablosu
│   ├── Tip-Sistemi           ← tip uyumu, null safety, casting kuralları
│   └── Koleksiyonlar         ← List/HashMap/Pair/Array/Slice API
│
├── codegen/
│   ├── Tuzaklar              ← inkwell API tuzakları (build_store, _setjmp...)
│   ├── EH-Implementasyon     ← setjmp/longjmp EH tam detayı
│   ├── Codegen-Kalıplar      ← tekrar eden kalıplar, 5-pass mimari
│   └── CodeGen-Struct        ← CodeGen<'ctx> tüm alanları
│
├── stdlib/
│   ├── Stdlib-Plan           ← modül hiyerarşisi, öncelik sırası
│   └── Stdlib-Sablonlar      ← .arm dosyası şablonları + Task.arm örneği
│
└── workflow/
    └── Build                 ← build/test/git/session checklist
```

---

## ❤️ Zorunlu Kural — Brain Her Zaman Güncel

> **Projede brain/'den habersiz hiçbir şey olmaz.**
> Kod değişince brain değişir. Eş zamanlı. Sonradan değil, şimdi.

### Ne değişti → Nereyi güncelle

| Eylem | Güncelle |
|---|---|
| Yeni özellik implement edildi | `Tamamlananlar` + `Sınırlar` (o satırı kaldır) |
| Görev tamamlandı | `Görev-Kuyruğu` (✅) + `Dashboard` anlık durum |
| Yeni codegen tuzağı keşfedildi | `Tuzaklar` |
| Yeni inkwell/LLVM kalıbı | `Codegen-Kalıplar` |
| EH implementasyonunda değişiklik | `EH-Implementasyon` |
| ARC veya bellek modelinde değişiklik | `Bellek-Modeli` |
| Yeni AST node / stmt / expr eklendi | `Sözdizimi` + gerekirse `Tip-Sistemi` |
| Mimari karar değişti veya netleşti | `Mimari` |
| Yeni görev belirlendi | `Görev-Kuyruğu` |
| Faza 6-9 planı netleşti | `Gelecek-Özellikler` |
| Stdlib modülü planlandı veya yazıldı | `Stdlib-Plan` + `Stdlib-Sablonlar` |
| Koleksiyon metodu tamamlandı | `Koleksiyonlar` + `Sınırlar` |
| Yeni brain notu eklendi | `Dashboard` Vault Haritası'na ekle + çapraz linkle |

### Yeni brain notu ekleme protokolü
1. Doğru alt klasöre koy (`proje/`, `codegen/` vb.)
2. `[[Link]]` formatında bağla — uzantısız, hep
3. `Dashboard.md` Vault Haritası'na ekle
4. İlgili diğer notlara "İlgili Notlar" altına linkle
5. En az 3 gelen link olsun (orphan kalmasın)

---

## Session Protokolü

### Başlarken (bu sırayla)
1. `brain/Dashboard.md` oku → anlık odak ne?
2. `brain/proje/Görev-Kuyruğu.md` oku → aktif görev detayı nedir?
3. Göreve göre ilgili notu aç:
   - Codegen yazıyorsan → `Tuzaklar` + `Codegen-Kalıplar` + `CodeGen-Struct`
   - Stdlib yazıyorsan → `Stdlib-Plan` + `Stdlib-Sablonlar` + `Sözdizimi`
   - Mimari kararı → `Mimari` + `Bellek-Modeli`
4. Gerekli `src/` dosyasını oku (tümünü değil, sadece ilgilisini)
5. Çalışmaya başla

### Bitirirken (bu sırayla)
1. Testler geçiyor mu? (cargo build + arc.exe + .exe çalıştır)
2. `Görev-Kuyruğu` güncelle → tamamlanan ✅
3. `Tamamlananlar` güncelle → yeni şeyler ekle
4. `Sınırlar` güncelle → çözülenleri çıkar, yenileri ekle
5. `Dashboard` → "Anlık Durum" tablosunu güncelle
6. `memory/project_arimo_codegen.md` kısaca güncelle
7. Commit + push

---

## Obsidian Linkleme Kuralı

```
✅ DOĞRU : [[Görev-Kuyruğu]]
✅ DOĞRU : [[Tuzaklar]]
❌ YANLIŞ: [[Görev-Kuyruğu.md]]
❌ YANLIŞ: [[proje/Görev-Kuyruğu]]   (unique isimse gerek yok)
```

Obsidian vault'u unique filename'e göre çözümler.
Alt klasörde olsa bile `[[İsim]]` yeterli.
`.md` uzantısı ASLA yazılmaz — Graph View bozulur.

---

## Kritik Teknik Bilgiler (Her Session Geçerli)

### Sıradaki Görev
**4.19-B — Lambda Closure Capture**
Detay: `brain/proje/Görev-Kuyruğu.md`

### En Kritik Tuzaklar (ezber)
1. `build_store(ptr, value)` — pointer ÖNCE, değer SONRA (inkwell'in tersi!)
2. `_setjmp(buf, NULL)` Windows UCRT — `setjmp` değil, `returns_twice` attr şart
3. jmpbuf → `[32 x [32 x i64]]` global, align 32 — stack alloca yetmez
4. `arc_release_var(slot)` — VarSlot alır, `(ptr, class_name)` değil
5. `ConstructorCall { class, args }` — alan adı `class`, `class_name` değil
6. Enum tipler → LLVM `i32` (ptr değil!)
7. `compile_module` 5 pass — forward ref için tüm dosyalar önce pass 0-2, sonra pass 3
8. EH globals → `Internal` linkage

---

## Git — Versiyon Kontrol Kuralları

### Altın Kural
**Her onaylanmış değişiklik → anında commit + push.**
Session bitmeden, "sonra yaparım" olmadan. Onay = commit.

### Ne Zaman Commit Alınır
| Durum | Eylem |
|---|---|
| Görev tamamlandı, testler geçiyor | commit + push |
| Önemli bug fix, testler geçiyor | commit + push |
| Yeni test dosyası eklendi | commit + push |
| Mühendis "tamam" veya "push" dedi | commit + push |
| Mühendis onay vermeden | **commit atma** |

### Commit Formatı

> **Commit dili: yalnızca İngilizce. İstisna yok.**

```
type(scope): short English description

# Kullanılan type'lar:
feat(codegen)  : new codegen feature
feat(lang)     : new language feature / AST node
fix(codegen)   : codegen bug fix
fix(tc)        : typechecker fix
test           : add/update test file
stdlib         : stdlib implementation
chore          : structural change, gitignore, etc.
docs           : documentation (CLAUDE.md included)
refactor       : restructure without behaviour change
```

Gerçek örnekler (bu repo'dan):
```
feat(codegen): phase 4.19-A — exception handling full implementation
fix(codegen): fix ARC over-retain and field-store bugs
lang: add package keyword, wildcard import and stdlib design doc
chore: move documents/ to gitignore (local-only)
refactor: remove comments, reorganize samples folder
```

### Staging — Sadece İlgili Dosyalar
```powershell
# DOĞRU — spesifik dosyalar:
git add src/codegen/mod.rs
git add src/tests/samples/codegen_closure.arm

# YANLIŞ — tümünü ekleme:
git add .
git add -A
git add src/
```

### Push
```powershell
git push origin master
# Master'a direkt push — PR yok, branch yok (tek geliştirici)
```

### Asla Yapılmayanlar
- Türkçe commit mesajı yazmak
- `Co-Authored-By` veya herhangi bir imza satırı ekleme
- `--force` veya `--force-with-lease` push
- `--no-verify` ile hook atlama
- `git add -A` veya `git add .`
- Merge commit — cherry-pick kullan gerekirse
- Onay olmadan commit
- Birden fazla görevi tek commit'e sıkıştırma (atomic commit)

### Durum Kontrolü
```powershell
git status          # ne değişti
git diff            # tam fark
git log --oneline   # son commit'ler
```

---

## Kaynak Yapısı (Ne Nerede)

```
src/
├── main.rs              → Pipeline + CLI flags (--emit-ir, -O2, -c)
├── ast/mod.rs           → Tüm AST node'ları (Expr::, Stmt::, Type::...)
├── lexer/mod.rs         → Tüm tokenlar ve keyword'ler
├── parser/mod.rs        → Pratt parser
├── typechecker/mod.rs   → TypeChecker + register_builtins (Exception, Object, Result, SIMD)
├── borrow/mod.rs        → BorrowChecker (UseAfterMove, MoveWhileBorrowed, MutationWhileBorrowed)
└── codegen/mod.rs       → ~6000 satır LLVM IR üretimi (ana dosya)
```

`src/` dosyalarını **tümünü tarama** — brain'de ne arayacağını bil, sonra sadece o dosyayı oku.

---

## İletişim Tarzı

- Teknik ve öz. Dolgu yok.
- Türkçe konuşuyoruz, kod İngilizce.
- Bir şey bilmiyorsam brain'e bakıyorum, brain'de yoksa kabul ediyorum.
- Tahmin etmiyorum — kaynak kodu okuyorum.
