# Presswerk ABI/FFI Documentation

## Overview

This library follows the **Hyperpolymath RSR Standard** for ABI and FFI design:

- **ABI (Application Binary Interface)** defined in **Idris2** with formal proofs
- **FFI (Foreign Function Interface)** implemented in **Zig** for C compatibility
- **Generated C headers** bridge Idris2 ABI to Zig FFI
- **Any language** can call through standard C ABI

## Architecture

```
┌─────────────────────────────────────────────┐
│  ABI Definitions (Idris2)                   │
│  src/abi/                                   │
│  - Types.idr      (Type definitions)        │
│  - Layout.idr     (Memory layout proofs)    │
│  - Foreign.idr    (FFI declarations)        │
└─────────────────┬───────────────────────────┘
                  │
                  │ generates (at compile time)
                  ▼
┌─────────────────────────────────────────────┐
│  C Headers (auto-generated)                 │
│  generated/abi/presswerk.h                │
└─────────────────┬───────────────────────────┘
                  │
                  │ imported by
                  ▼
┌─────────────────────────────────────────────┐
│  FFI Implementation (Zig)                   │
│  ffi/zig/src/main.zig                       │
│  - Implements C-compatible functions        │
│  - Zero-cost abstractions                   │
│  - Memory-safe by default                   │
└─────────────────┬───────────────────────────┘
                  │
                  │ compiled to libpresswerk.so/.a
                  ▼
┌─────────────────────────────────────────────┐
│  Any Language via C ABI                     │
│  - Rust, ReScript, Julia, Python, etc.     │
└─────────────────────────────────────────────┘
```

## Directory Structure

```
presswerk/
├── src/
│   ├── abi/                    # ABI definitions (Idris2)
│   │   ├── Types.idr           # Core type definitions with proofs
│   │   ├── Layout.idr          # Memory layout verification
│   │   ├── Foreign.idr         # FFI function declarations
│   │   ├── Bridge.idr          # Gutenberg block bridge types
│   │   ├── Protocol.idr        # WordPress REST protocol proofs
│   │   └── Encryption.idr      # Content encryption ABI
│   └── lib/                    # Core library (any language)
│
├── ffi/
│   └── zig/                    # FFI implementation (Zig)
│       ├── build.zig           # Build configuration
│       ├── build.zig.zon       # Dependencies
│       ├── src/
│       │   └── main.zig        # C-compatible FFI implementation
│       ├── test/
│       │   └── integration_test.zig
│       └── include/
│           └── presswerk.h   # C header (optional, can be generated)
│
├── generated/                  # Auto-generated files
│   └── abi/
│       └── presswerk.h       # Generated from Idris2 ABI
│
└── bindings/                   # Language-specific wrappers (optional)
    ├── rust/
    ├── rescript/
    └── julia/
```

## Why Idris2 for ABI?

### 1. **Formal Verification**

Idris2's dependent types allow proving properties about the ABI at compile-time:

```idris
-- Prove struct size is correct
public export
exampleStructSize : HasSize ExampleStruct 16

-- Prove field alignment is correct
public export
fieldAligned : Divides 8 (offsetOf ExampleStruct.field)

-- Prove ABI is platform-compatible
public export
abiCompatible : Compatible (ABI 1) (ABI 2)
```

### 2. **Type Safety**

Encode invariants that C/Zig cannot express:

```idris
-- Non-null pointer guaranteed at type level
data Handle : Type where
  MkHandle : (ptr : Bits64) -> {auto 0 nonNull : So (ptr /= 0)} -> Handle

-- Array with length proof
data Buffer : (n : Nat) -> Type where
  MkBuffer : Vect n Byte -> Buffer n
```

### 3. **Platform Abstraction**

Platform-specific types with compile-time selection:

```idris
CInt : Platform -> Type
CInt Linux = Bits32
CInt Windows = Bits32

CSize : Platform -> Type
CSize Linux = Bits64
CSize Windows = Bits64
```

### 4. **Safe Evolution**

Prove that new ABI versions are backward-compatible:

```idris
-- Compiler enforces compatibility
abiUpgrade : ABI 1 -> ABI 2
abiUpgrade old = MkABI2 {
  -- Must preserve all v1 fields
  v1_compat = old,
  -- Can add new fields
  new_features = defaults
}
```

## Why Zig for FFI?

### 1. **C ABI Compatibility**

Zig exports C-compatible functions naturally:

```zig
export fn presswerk_init() ?*anyopaque {
    return internal_init() catch null;
}

export fn presswerk_free(handle: *anyopaque) void {
    internal_free(handle);
}

export fn presswerk_hash(handle: *anyopaque, data: [*]const u8, len: usize) u64 {
    return internal_hash(handle, data[0..len]);
}

export fn presswerk_validate_transition(handle: *anyopaque, from: [*:0]const u8, to: [*:0]const u8) i32 {
    return if (internal_validate(handle, from, to)) 1 else 0;
}
```

### 2. **Memory Safety**

Compile-time safety without runtime overhead:

```zig
// Null check enforced at compile time
const handle = init() orelse return error.InitFailed;
defer free(handle);
```

### 3. **Cross-Compilation**

Built-in cross-compilation to any platform:

```bash
zig build -Dtarget=x86_64-linux
zig build -Dtarget=aarch64-macos
zig build -Dtarget=x86_64-windows
```

### 4. **Zero Dependencies**

No runtime, no libc required (unless explicitly needed):

```zig
// Minimal binary size
pub const lib = @import("std");
// Only includes what you use
```

## Building

### Build FFI Library

```bash
cd ffi/zig
zig build                         # Build debug
zig build -Doptimize=ReleaseFast  # Build optimized
zig build test                    # Run tests
```

### Generate C Header from Idris2 ABI

```bash
cd src/abi
idris2 --cg c-header Types.idr -o ../../generated/abi/presswerk.h
```

### Cross-Compile

```bash
cd ffi/zig

# Linux x86_64
zig build -Dtarget=x86_64-linux

# macOS ARM64
zig build -Dtarget=aarch64-macos

# Windows x86_64
zig build -Dtarget=x86_64-windows
```

## Usage

### From C

```c
#include "presswerk.h"

int main() {
    void* handle = presswerk_init();
    if (!handle) return 1;

    // Hash content for integrity verification
    const char* content = "Hello, Presswerk!";
    uint64_t hash = presswerk_hash(handle, content, strlen(content));

    // Validate a state transition
    int valid = presswerk_validate_transition(handle, "draft", "published");
    if (!valid) {
        const char* err = presswerk_last_error();
        fprintf(stderr, "Error: %s\n", err);
    }

    presswerk_free(handle);
    return 0;
}
```

Compile with:
```bash
gcc -o example example.c -lpresswerk -L./zig-out/lib
```

### From Idris2

```idris
import Presswerk.ABI.Foreign

main : IO ()
main = do
  Just handle <- presswerk_init
    | Nothing => putStrLn "Failed to initialize Presswerk"

  let hash = presswerk_hash handle "Hello, Presswerk!"
  putStrLn $ "Content hash: " ++ show hash

  Right () <- presswerk_validate_transition handle "draft" "published"
    | Left err => putStrLn $ "Transition error: " ++ errorDescription err

  presswerk_free handle
  putStrLn "Success"
```

### From Rust

```rust
#[link(name = "presswerk")]
extern "C" {
    fn presswerk_init() -> *mut std::ffi::c_void;
    fn presswerk_free(handle: *mut std::ffi::c_void);
    fn presswerk_hash(handle: *mut std::ffi::c_void, data: *const u8, len: usize) -> u64;
    fn presswerk_validate_transition(
        handle: *mut std::ffi::c_void,
        from: *const std::ffi::c_char,
        to: *const std::ffi::c_char,
    ) -> i32;
}

fn main() {
    unsafe {
        let handle = presswerk_init();
        assert!(!handle.is_null());

        let content = b"Hello, Presswerk!";
        let hash = presswerk_hash(handle, content.as_ptr(), content.len());
        println!("Content hash: {hash}");

        let from = c"draft";
        let to = c"published";
        let valid = presswerk_validate_transition(handle, from.as_ptr(), to.as_ptr());
        assert_eq!(valid, 1);

        presswerk_free(handle);
    }
}
```

### From Julia

```julia
const libpresswerk = "libpresswerk"

function presswerk_init()
    handle = ccall((:presswerk_init, libpresswerk), Ptr{Cvoid}, ())
    handle == C_NULL && error("Failed to initialize Presswerk")
    handle
end

function presswerk_hash(handle, data::AbstractString)
    ccall((:presswerk_hash, libpresswerk), UInt64,
          (Ptr{Cvoid}, Ptr{UInt8}, Csize_t), handle, data, sizeof(data))
end

function presswerk_validate_transition(handle, from::AbstractString, to::AbstractString)
    result = ccall((:presswerk_validate_transition, libpresswerk), Cint,
                   (Ptr{Cvoid}, Cstring, Cstring), handle, from, to)
    result != 0
end

function presswerk_free(handle)
    ccall((:presswerk_free, libpresswerk), Cvoid, (Ptr{Cvoid},), handle)
end

# Usage
handle = presswerk_init()
try
    hash = presswerk_hash(handle, "Hello, Presswerk!")
    println("Content hash: $hash")

    valid = presswerk_validate_transition(handle, "draft", "published")
    println("Transition valid: $valid")
finally
    presswerk_free(handle)
end
```

## Testing

### Unit Tests (Zig)

```bash
cd ffi/zig
zig build test
```

### Integration Tests

```bash
cd ffi/zig
zig build test-integration
```

### ABI Verification (Idris2)

```idris
-- Compile-time verification
%runElab verifyABI

-- Runtime checks
main : IO ()
main = do
  verifyLayoutsCorrect
  verifyAlignmentsCorrect
  putStrLn "ABI verification passed"
```

## Contributing

When modifying the ABI/FFI:

1. **Update ABI first** (`src/abi/*.idr`)
   - Modify type definitions
   - Update proofs
   - Ensure backward compatibility

2. **Generate C header**
   ```bash
   idris2 --cg c-header src/abi/Types.idr -o generated/abi/presswerk.h
   ```

3. **Update FFI implementation** (`ffi/zig/src/main.zig`)
   - Implement new functions
   - Match ABI types exactly

4. **Add tests**
   - Unit tests in Zig
   - Integration tests
   - ABI verification tests

5. **Update documentation**
   - Function signatures
   - Usage examples
   - Migration guide (if breaking changes)

## License

MPL-2.0

## See Also

- [Idris2 Documentation](https://idris2.readthedocs.io)
- [Zig Documentation](https://ziglang.org/documentation/master/)
- [Rhodium Standard Repositories](https://github.com/hyperpolymath/rhodium-standard-repositories)
- [FFI Migration Guide](../ffi-migration-guide.md)
- [ABI Migration Guide](../abi-migration-guide.md)
