## Arimo v1.0.0

First stable release of the Arimo programming language.

### What's included

- `arc.exe` — the Arimo compiler (Stage 1: compiler written in Arimo itself)
- `stdlib/` — standard library (.arm source files)

### Install

Run the installer and follow the wizard. Optionally add `arc` to your system PATH.

### Verify

```
arc --version
```

### Language features

- Statically-typed OOP with generics, pattern matching, lambdas
- Three-layer memory safety: BorrowChecker + ARC + Manual (`@ManualMemory`)
- Own IR pipeline — no LLVM, no GCC required
- Native PE32+ executable output (x86-64)
- Null safety built into the type system (`String?`, `?.`, `??`)
- Inline assembly (`asm {}`), `@Freestanding`, `@CallingConvention`
- Full standard library: `arimo.lang`, `arimo.fs`, `arimo.io`, `arimo.util`, `arimo.env`

### Platform

Windows x64 only in v1.0.0. Linux/macOS planned.

### Changelog

See [documentation/reference/versioning.md](https://github.com/egecanakincioglu/arimo/blob/main/documentation/reference/versioning.md) for full history.
