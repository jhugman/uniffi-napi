# Auto-Refactor: Rust Style Guide + Patterns

## Goal
Adopt the official Rust style guide (rustwiki.org/en/style-guide) and Rust design patterns (rust-unofficial/patterns) across all Rust code in `src/`.

## Classification
Objective — concrete rules, tests are sufficient.

## Verification
- **Fast:** `cargo fmt -- --check && cargo check`
- **Full:** `npm run build && npm run test`

## Baseline
- Commit: `466ef53` (HEAD at start)
- Tests: 44/44 passing

## Candidate List

### Mechanical Formatting (cargo fmt)
- [ ] Run `cargo fmt` to fix line length, brace placement, argument formatting

### Import Ordering
- [ ] `fn_pointer.rs`: Group std imports before external crates
- [ ] `callback.rs`: Consolidate libffi imports
- [ ] All files: Ensure ascii-betical sorting within groups

### Comment Style
- [ ] Ensure comments are complete sentences (capital letter, period)
- [ ] Prefer `//` over `/* */`

### Expression-Oriented Style
- [ ] `register.rs:438-447`: Refactor conditional assignment to expression style

### Anti-patterns
- [ ] `callback.rs:142`: Replace `panic!` with `Result` return

### Unnecessary Clones
- [ ] `fn_pointer.rs:267`: Remove unnecessary clone before Rc::new

### Patterns to Apply
- [ ] Borrowed types for arguments — check for &String, &Vec<T>
- [ ] Contain unsafety — verify unsafe blocks are minimal and well-wrapped

## Iteration Log

(Iterations will be logged here)
