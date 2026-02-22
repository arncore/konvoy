# Code Style

This document describes the Rust conventions used in Konvoy. These are enforced by our workspace lint configuration and CI.

## Error handling

Never call `.unwrap()`, `.expect()`, or `panic!()`. Every fallible operation must be handled explicitly.

```rust
// Bad — crashes at runtime
let config = Manifest::from_path(&path).unwrap();

// Good — propagate with ?
let config = Manifest::from_path(&path)?;

// Good — handle inline
let config = match Manifest::from_path(&path) {
    Ok(c) => c,
    Err(e) => {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
};
```

Use `let-else` for early returns on failure, rather than nested `match` or `if let`:

```rust
// Bad
let target = match resolve_target(flag) {
    Some(t) => t,
    None => return Err(Error::NoTarget),
};

// Good
let Some(target) = resolve_target(flag) else {
    return Err(Error::NoTarget);
};
```

## Safe access

Never index into slices or vectors directly. Use `.get()` and handle the `None` case.

```rust
// Bad — panics on out-of-bounds
let first = args[0];

// Good
let Some(first) = args.get(0) else {
    return Err(Error::MissingArgument);
};
```

## Immutability by default

Prefer immutable bindings. Only use `mut` when mutation is genuinely needed.

```rust
// Bad — mutable for no reason
let mut name = "konvoy";

// Good
let name = "konvoy";
```

Prefer borrowing (`&T`) over taking ownership (`T`) in function signatures unless ownership is required:

```rust
// Bad — takes ownership unnecessarily
fn validate(name: String) -> bool { ... }

// Good — borrows
fn validate(name: &str) -> bool { ... }
```

## Expression-oriented style

Rust blocks return their last expression. Use this instead of explicit `return` statements:

```rust
// Bad
fn profile_dir(release: bool) -> &'static str {
    if release {
        return "release";
    } else {
        return "debug";
    }
}

// Good
fn profile_dir(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}
```

`match` is an expression. Use it as one:

```rust
// Good
let label = match status {
    Status::Building => "compiling",
    Status::Cached => "fresh",
    Status::Failed => "error",
};
```

When a function returns `()`, add a trailing semicolon to the last statement for clarity:

```rust
// Good — semicolon makes the unit return explicit
fn clean(dir: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
```

## Imports

Never use wildcard imports. Always import specific items:

```rust
// Bad
use std::collections::*;

// Good
use std::collections::HashMap;
```

Group imports in this order, separated by blank lines:

1. Standard library (`std`)
2. External crates
3. Workspace crates (`konvoy_*`)
4. Local modules (`crate::`, `super::`)

## Type conversions and casting

Never use `as` for numeric casts. Use the safe conversion methods:

```rust
// Bad — silently truncates if value > 255
let byte = count as u8;

// Good — returns Err if value doesn't fit
let byte = u8::try_from(count)?;

// Good — for lossless widening conversions
let wide = u64::from(narrow_u32);
```

## String allocation

Be explicit about when you allocate heap strings. Use `.to_owned()` on string slices, not `.to_string()`:

```rust
// Bad — hides the allocation behind a Display trait call
let name = "konvoy".to_string();

// Good — clearly says "I am copying this into an owned String"
let name = "konvoy".to_owned();
```

Reserve `.to_string()` for types that implement `Display` where you want their formatted representation:

```rust
let msg = some_error.to_string();  // fine — Display formatting
let count_str = 42.to_string();    // fine — numeric formatting
```

## Closures and iterators

Use method references when a closure just calls a single method:

```rust
// Bad
let names: Vec<_> = items.iter().map(|item| item.name()).collect();

// Good
let names: Vec<_> = items.iter().map(Item::name).collect();
```

Use `.copied()` instead of `.cloned()` when working with `Copy` types:

```rust
// Bad — implies an expensive clone
let ids: Vec<u32> = id_refs.iter().cloned().collect();

// Good — makes it clear the copy is trivial
let ids: Vec<u32> = id_refs.iter().copied().collect();
```

When collecting a sequence of `Result` values, collect into `Result<Vec<T>, E>` to short-circuit on the first error:

```rust
// Bad — gives you Vec<Result<T, E>>, errors are buried
let results: Vec<Result<_, _>> = paths.iter().map(parse).collect();

// Good — stops at first error
let parsed: Result<Vec<_>, _> = paths.iter().map(parse).collect();
let parsed = parsed?;
```

## Pattern matching

Prefer `match` over chains of `if-else` when there are more than two branches:

```rust
// Bad
if cmd == "build" {
    do_build();
} else if cmd == "run" {
    do_run();
} else if cmd == "clean" {
    do_clean();
} else {
    unknown(cmd);
}

// Good
match cmd {
    "build" => do_build(),
    "run" => do_run(),
    "clean" => do_clean(),
    other => unknown(other),
}
```

When matching a single variant, use `if let` or `let-else` instead of a full `match`:

```rust
// Bad
match config.toolchain {
    Some(tc) => validate(tc),
    None => {}
}

// Good
if let Some(tc) = config.toolchain {
    validate(tc);
}
```

## Shadowing

Avoid shadowing a variable with an unrelated value. Shadowing should only transform the same data:

```rust
// Bad — name reused for completely different data
let input = read_file(path)?;
let input = parse_config(&input)?;  // now it's a Config, not a String

// Good — distinct names for distinct types
let raw = read_file(path)?;
let config = parse_config(&raw)?;

// OK — same data, refined type
let port = std::env::var("PORT")?;
let port: u16 = port.parse()?;  // same concept, narrower type
```

## Error types

Define error types with `thiserror`. Error messages should be lowercase, actionable, and avoid jargon:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("missing required field `package.name` in {path}")]
    MissingName { path: String },
}
```

## Documentation

Public functions that return `Result` must include an `# Errors` section:

```rust
/// Load the project manifest from disk.
///
/// # Errors
/// Returns an error if the file cannot be read or contains invalid TOML.
pub fn load_manifest(path: &Path) -> Result<Manifest, ConfigError> {
    ...
}
```

## Functions

Keep functions short and focused on a single task. If a function needs a comment explaining what a section does, that section is a candidate for extraction:

```rust
// Bad — one long function doing everything
fn build(config: &Config) -> Result<()> {
    // resolve target
    ...
    // compute cache key
    ...
    // invoke compiler
    ...
    // store artifacts
    ...
}

// Good — composed of clear steps
fn build(config: &Config) -> Result<()> {
    let target = resolve_target(config)?;
    let key = compute_cache_key(config, &target)?;
    let output = invoke_compiler(config, &target)?;
    store_artifacts(&output, &key)?;
    Ok(())
}
```
