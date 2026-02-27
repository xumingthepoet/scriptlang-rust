# Rust Testability Playbook for 100% Coverage (Agent-Oriented)

This document is an operational guide for writing Rust code that is easy to test and can reliably reach 100% line coverage without fragile tests.

Use it as a default implementation policy when generating or refactoring code.

## 1. Core Policy

Design for testability first, then implement behavior.

1. Separate pure logic from side effects.
2. Inject all non-deterministic dependencies (time, random, I/O, env, events, network).
3. Keep traits small and behavior-focused.
4. Return typed errors and assert exact error variants in tests.
5. For every branch you write, create at least one test that reaches it.

If a function is hard to test, treat it as a design issue, not a testing issue.

## 2. Architecture Pattern: Core + Ports + Adapters

Use three layers:

1. Core (`domain`): pure business logic, no I/O.
2. Ports (`trait`): abstractions for dependencies.
3. Adapters (`infra`): real implementations (FS, HTTP, DB, clock, RNG).

Rule:
- Core can depend on ports.
- Adapters implement ports.
- Tests for core use fake/mock ports.

## 3. A Function Depends on Another Function

### Anti-pattern
- Function `A` directly calls complex function `B` with side effects.
- Tests for `A` become integration-heavy and unstable.

### Better pattern
- Extract `B` behind a trait.
- Inject it into `A`.

```rust
pub trait PriceService {
    fn unit_price(&self, sku: &str) -> Result<u32, PriceError>;
}

#[derive(Debug, PartialEq)]
pub enum PriceError {
    NotFound,
    Backend,
}

pub struct Checkout<S: PriceService> {
    price_service: S,
}

impl<S: PriceService> Checkout<S> {
    pub fn new(price_service: S) -> Self {
        Self { price_service }
    }

    pub fn total(&self, sku: &str, qty: u32) -> Result<u32, PriceError> {
        let p = self.price_service.unit_price(sku)?;
        Ok(p.saturating_mul(qty))
    }
}
```

Test with fakes:

```rust
struct PriceOk;
impl PriceService for PriceOk {
    fn unit_price(&self, _: &str) -> Result<u32, PriceError> {
        Ok(25)
    }
}

struct PriceNotFound;
impl PriceService for PriceNotFound {
    fn unit_price(&self, _: &str) -> Result<u32, PriceError> {
        Err(PriceError::NotFound)
    }
}

#[test]
fn total_success() {
    let c = Checkout::new(PriceOk);
    assert_eq!(c.total("A", 4), Ok(100));
}

#[test]
fn total_propagates_not_found() {
    let c = Checkout::new(PriceNotFound);
    assert_eq!(c.total("A", 1), Err(PriceError::NotFound));
}
```

Coverage effect:
- success path covered
- error propagation path covered

## 4. A File/Module Depends on Another File/Module

Keep module dependency directional.

Example layout:

```text
src/
  domain/order.rs          # pure logic + traits
  infra/sql_order_repo.rs  # DB implementation
  app/place_order.rs       # orchestration
```

`domain/order.rs`:

```rust
pub trait OrderRepo {
    fn save(&mut self, order: Order) -> Result<(), RepoError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub id: String,
    pub amount: u32,
}

#[derive(Debug, PartialEq)]
pub enum RepoError {
    Conflict,
    Unavailable,
}

pub fn place_order(repo: &mut dyn OrderRepo, id: &str, amount: u32) -> Result<Order, RepoError> {
    let order = Order {
        id: id.to_string(),
        amount,
    };
    repo.save(order.clone())?;
    Ok(order)
}
```

`domain` tests use `InMemoryRepo` fake; no SQL, no network.

### 4.1 File-Level Defense Rule (No Incidental Coverage Credit)

If tests for file `A` execute lines in file `B`, that is only incidental coverage.
It does not replace direct tests for `B`.

Policy:

1. Each non-trivial source file must have direct tests for its own functions.
2. Each non-trivial function in that file must be exercised by at least one test that targets it intentionally.
3. Cross-file coverage is useful, but it is integration confidence, not unit defense.
4. Keep a one-to-one test file mapping for unit tests: one source file, one corresponding unit-test file.
5. Do not merge unit tests for multiple source files into a single unit-test file.

Example:

```text
src/a.rs
src/b.rs

tests/a_tests.rs   # tests A behavior, may touch B indirectly
tests/b_tests.rs   # required: directly tests B's functions and branches
```

Do not do this for unit tests:

```text
tests/domain_all_tests.rs  # mixes unit tests for a.rs, b.rs, c.rs together
```

Practical rule:
- If `b.rs` lines are covered only by `a_tests.rs`, `b.rs` is still under-defended.
- Add explicit tests in `b_tests.rs` for each branch/error path in `b.rs`.
- Use `a_tests.rs` to verify integration contracts between A and B.
- Keep integration scenarios in separate integration test files; do not replace per-file unit tests.

## 5. External Environment Dependencies

Never access external environment directly inside core logic.

## 5.1 Randomness

```rust
pub trait Random {
    fn gen_range(&mut self, start: u32, end: u32) -> u32;
}

pub fn roll_dice(rng: &mut dyn Random) -> u32 {
    rng.gen_range(1, 7)
}
```

Test with deterministic fake:

```rust
struct FixedRandom(u32);
impl Random for FixedRandom {
    fn gen_range(&mut self, _: u32, _: u32) -> u32 {
        self.0
    }
}
```

## 5.2 Time/Clock

```rust
pub trait Clock {
    fn now_unix(&self) -> i64;
}

pub fn is_expired(clock: &dyn Clock, deadline: i64) -> bool {
    clock.now_unix() > deadline
}
```

Tests set `now_unix` explicitly to cover both branches.

## 5.3 File I/O

```rust
pub trait FileStore {
    fn read_to_string(&self, path: &str) -> Result<String, FsError>;
}

#[derive(Debug, PartialEq)]
pub enum FsError {
    NotFound,
    Permission,
    Other,
}

pub fn load_port(fs: &dyn FileStore) -> Result<u16, LoadConfigError> {
    let s = fs.read_to_string("app.port").map_err(LoadConfigError::Fs)?;
    s.trim().parse::<u16>().map_err(|_| LoadConfigError::Parse)
}

#[derive(Debug, PartialEq)]
pub enum LoadConfigError {
    Fs(FsError),
    Parse,
}
```

Minimum tests:
1. read success + parse success
2. read failure (`NotFound`)
3. parse failure (`Parse`)

## 5.4 Event Sources

Wrap event systems with a trait and inject event stream snapshots into logic.

```rust
pub trait EventSource {
    fn next_event(&mut self) -> Option<Event>;
}
```

Fake event source with queue:
- allows deterministic event order tests
- covers empty queue branch and error events

## 6. Trait Design Rules

1. Keep trait methods minimal and cohesive.
2. Prefer domain types over raw strings for method signatures.
3. Prefer `&mut self` only when mutation is required.
4. Use associated error enums; avoid `String` for domain errors.
5. Use generics (`T: Trait`) for static dispatch when practical.
6. Use `dyn Trait` where runtime polymorphism is simpler.

### Generic vs `dyn` quick rule
- Core libraries: prefer generics for compile-time guarantees.
- App orchestration and tests: `dyn Trait` is often simpler.

## 7. Fake vs Mock: When to Use Which

Use fake by default.

## Fake
- In-memory implementation with real behavior.
- Best for stateful scenarios and readability.
- Stable and low maintenance.

## Mock
- Verifies interaction details (call count/order/arguments).
- Use only when interaction contract is the behavior under test.

Example with `mockall`:

```rust
use mockall::automock;

#[automock]
pub trait Notifier {
    fn send(&self, user_id: &str, msg: &str) -> Result<(), NotifyError>;
}
```

Test can assert `.times(1)` and exact parameters.

Heuristic:
1. If output/state is enough to validate behavior: fake.
2. If call choreography is required: mock.

## 8. Test Design Matrix for 100% Coverage

For each function, define cases before coding:

1. Happy path.
2. Each error path.
3. Boundary inputs (empty, min, max, zero, one-element).
4. State transitions (valid and invalid).
5. Idempotency/retry behavior when applicable.

For each `match` or `if`, map at least one test case to each arm.

Also enforce a file/function mapping:

1. For every touched file, list its functions.
2. For each function, list direct test cases that target it.
3. Do not mark a function as defended only because another file's tests executed its lines.

## 9. Error Modeling for Better Tests

Prefer:

```rust
#[derive(Debug, PartialEq)]
pub enum CreateUserError {
    InvalidName,
    DuplicateId,
    StorageUnavailable,
}
```

Then test with exact variant assertions:

```rust
assert_eq!(result, Err(CreateUserError::InvalidName));
```

Avoid only `assert!(result.is_err())`, which weakens test precision.

## 10. Typical Refactoring Recipe (Legacy to Testable)

When you find untestable code:

1. Extract pure decision logic into a standalone function.
2. Introduce one trait for each external dependency.
3. Move direct I/O calls into adapter structs.
4. Add fake adapters in tests.
5. Add one test per branch until uncovered lines are zero.

## 11. Coverage Workflow

Use this loop:

1. Run tests.
2. Generate coverage report.
3. Identify uncovered lines.
4. Add or refine tests for missing branches.
5. Repeat until complete coverage.

Coverage commands (example):

```bash
cargo test --all-targets --all-features
cargo llvm-cov --all-targets --all-features --summary-only
```

### 11.1 Coverage Integrity Rule (No Bypass by Default)

Do not bypass coverage requirements.

Default policy:

1. Do not disable, skip, or weaken coverage checks to make the gate pass.
2. Do not hide untested logic behind conditional compilation or test-only shortcuts.
3. Do not exclude files/functions from coverage unless the requirement explicitly allows it.
4. Do not replace missing tests with comments, TODOs, or relaxed assertions.

Only exception:
- You may skip or relax coverage/testing only when the requirement explicitly states that tests are optional for that scope.

Important:
- 100% line coverage is a gate, not the goal.
- The goal is behavior confidence and regression resistance.

## 12. Agent Checklist Before Submitting Code

1. Did I isolate side effects behind traits?
2. Can all branches be reached deterministically?
3. Do tests assert exact outputs/errors/state transitions?
4. Are fake/mock choices justified?
5. Is every new function defended by direct tests?
6. Does each touched source file have its own dedicated unit-test file (no multi-file unit-test merging)?
7. Does each touched file have direct tests (not only indirect execution from other files)?
8. Does coverage show no uncovered lines in touched files?
9. Did I avoid bypassing/weakening coverage checks (unless explicitly allowed by requirements)?

If any answer is "no", revise design before adding more tests.
