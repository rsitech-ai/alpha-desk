# Stage 0 Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a reproducible, security-checked Rust/Swift monorepo with exact domain primitives, versioned contracts, golden fixtures, local dependencies, telemetry, CI, and deployment scaffolding.

**Architecture:** Establish dependency-inverted domain crates before any venue adapter or database implementation. Tooling, schemas, fixtures, health contracts, and build provenance are treated as production interfaces so every later stage can be replayed, audited, and released from the same workspace.

**Tech Stack:** Rust 1.97.1 edition 2024, Cargo, Tokio 1.52.x, Serde, Thiserror, Proptest, Prost/Tonic 0.14.x, `tracing`, OpenTelemetry, Docker Compose/Podman, NATS 2.14.x, ClickHouse 26.3 LTS, PostgreSQL 18.4, MinIO, Swift 6.3 package skeleton, GitHub Actions-compatible CI, Ansible and systemd.

## Global Constraints

- The approved source of truth is `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` at tag `design-approved-v1.0.0`; `spec-v1.0.0` preserves the reviewed design content before approval metadata was recorded.
- Rust production code uses Rust 1.97.1, edition 2024, with committed `Cargo.lock` and no unreviewed `unsafe` blocks.
- Swift code uses Swift 6.3 language mode with strict concurrency and treats controllable concurrency warnings as errors.
- Canonical accounting uses checked fixed-point values. `f64` is forbidden in balances, positions, fees, funding, margin, event identity, and reconciliation.
- Canonical reducers are synchronous and deterministic; asynchronous I/O belongs outside the reducer boundary.
- Every state-affecting message is idempotent by stable `EventId`; transport is at least once and effects are exactly once.
- Live and historical replay use the same parser, reducer, feature, and signal code paths.
- Point-in-time and bitemporal correctness are mandatory. Historical outputs may use only information known at the evaluated timestamp.
- Raw source evidence and canonical events are archived before analytical compaction. ClickHouse is rebuildable and never the only copy.
- V1 is read-only. `hl-exec`, trading credentials, signing keys, order placement, and automatic copy trading are excluded from all V1 artifacts and deployments.
- Canonical signal direction is never produced by a language model. Production learned models must be approved, signed, local, schema-matched artifacts.
- A red data-health dependency suppresses the affected feature or signal. The system fails closed for alpha.
- Every task follows test-driven development, includes exact verification commands, and ends in a focused commit.

---

### Task 1: Bootstrap the monorepo and pinned toolchains

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.cargo/config.toml`
- Create: `justfile`
- Modify: `.gitignore`
- Modify: `README.md`
- Create: `LICENSE`
- Create: `crates/domain-types/Cargo.toml`
- Create: `crates/hl-protocol/Cargo.toml`
- Create: `crates/canonical-events/Cargo.toml`
- Create: `crates/canonical-ledger/Cargo.toml`
- Create: `crates/orderbook/Cargo.toml`
- Create: `crates/margin-models/Cargo.toml`
- Create: `crates/entity-graph/Cargo.toml`
- Create: `crates/feature-core/Cargo.toml`
- Create: `crates/wallet-intelligence/Cargo.toml`
- Create: `crates/market-intelligence/Cargo.toml`
- Create: `crates/signal-core/Cargo.toml`
- Create: `crates/execution-sim/Cargo.toml`
- Create: `crates/replay-engine/Cargo.toml`
- Create: `crates/model-runtime/Cargo.toml`
- Create: `crates/storage-ports/Cargo.toml`
- Create: `crates/api-contracts/Cargo.toml`
- Create: `crates/telemetry/Cargo.toml`
- Create: `crates/test-fixtures/Cargo.toml`
- Create: `crates/domain-types/src/lib.rs`
- Create: `crates/hl-protocol/src/lib.rs`
- Create: `crates/canonical-events/src/lib.rs`
- Create: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/orderbook/src/lib.rs`
- Create: `crates/margin-models/src/lib.rs`
- Create: `crates/entity-graph/src/lib.rs`
- Create: `crates/feature-core/src/lib.rs`
- Create: `crates/wallet-intelligence/src/lib.rs`
- Create: `crates/market-intelligence/src/lib.rs`
- Create: `crates/signal-core/src/lib.rs`
- Create: `crates/execution-sim/src/lib.rs`
- Create: `crates/replay-engine/src/lib.rs`
- Create: `crates/model-runtime/src/lib.rs`
- Create: `crates/storage-ports/src/lib.rs`
- Create: `crates/api-contracts/src/lib.rs`
- Create: `crates/telemetry/src/lib.rs`
- Create: `crates/test-fixtures/src/lib.rs`
- Create: `services/hl-capture/Cargo.toml`
- Create: `services/hl-core/Cargo.toml`
- Create: `services/hl-analytics/Cargo.toml`
- Create: `services/hl-research/Cargo.toml`
- Create: `services/hl-api/Cargo.toml`
- Create: `services/hl-capture/src/main.rs`
- Create: `services/hl-core/src/main.rs`
- Create: `services/hl-analytics/src/main.rs`
- Create: `services/hl-research/src/main.rs`
- Create: `services/hl-api/src/main.rs`
- Create: `apps/AlphaDesk/Package.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Bootstrap.swift`
- Create: `apps/AlphaDesk/Sources/DeskNetworking/Bootstrap.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Bootstrap.swift`
- Create: `apps/AlphaDesk/Tests/DeskDomainTests/BootstrapTests.swift`
- Create: `tools/ci/check-workspace.sh`

**Interfaces:**
- Consumes: approved monorepo layout from design section 9.1.
- Produces: a Cargo workspace, Swift package graph, common `just` commands, and stable crate/service names used by all subsequent tasks.

- [ ] **Step 1: Write the workspace layout check before creating the workspace**

Create `tools/ci/check-workspace.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

required=(
  Cargo.toml rust-toolchain.toml justfile
  crates/domain-types/Cargo.toml
  crates/canonical-events/Cargo.toml
  crates/telemetry/Cargo.toml
  services/hl-capture/Cargo.toml
  services/hl-core/Cargo.toml
  services/hl-analytics/Cargo.toml
  services/hl-research/Cargo.toml
  services/hl-api/Cargo.toml
  apps/AlphaDesk/Package.swift
)

for path in "${required[@]}"; do
  [[ -f "$path" ]] || { echo "missing:$path" >&2; exit 1; }
done

cargo metadata --format-version 1 --no-deps >/dev/null
swift package --package-path apps/AlphaDesk describe >/dev/null
printf 'workspace-layout:ok\n'
```

Run:

```bash
chmod +x tools/ci/check-workspace.sh
./tools/ci/check-workspace.sh
```

Expected: FAIL with the first missing workspace file.

- [ ] **Step 2: Create the pinned Rust workspace root**

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.97.1"
components = ["clippy", "rustfmt", "rust-src"]
profile = "minimal"
```

Create `.cargo/config.toml`:

```toml
[build]
rustflags = ["-Dwarnings"]

[term]
color = "always"

[env]
RUST_BACKTRACE = "1"
```

Create the root `Cargo.toml` with every approved crate and service as a workspace member:

```toml
[workspace]
resolver = "3"
members = [
  "crates/domain-types",
  "crates/hl-protocol",
  "crates/canonical-events",
  "crates/canonical-ledger",
  "crates/orderbook",
  "crates/margin-models",
  "crates/entity-graph",
  "crates/feature-core",
  "crates/wallet-intelligence",
  "crates/market-intelligence",
  "crates/signal-core",
  "crates/execution-sim",
  "crates/replay-engine",
  "crates/model-runtime",
  "crates/storage-ports",
  "crates/api-contracts",
  "crates/telemetry",
  "crates/test-fixtures",
  "services/hl-capture",
  "services/hl-core",
  "services/hl-analytics",
  "services/hl-research",
  "services/hl-api",
]

[workspace.package]
edition = "2024"
rust-version = "1.97.1"
license = "Apache-2.0"

[workspace.dependencies]
anyhow = "1.0"
async-trait = "0.1"
blake3 = "1.8"
bytes = "1.10"
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
hex = "0.4"
proptest = "1.7"
prost = "0.14"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
tokio = { version = "1.52", features = ["macros", "rt-multi-thread", "signal", "time", "sync"] }
tonic = { version = "0.14", features = ["transport"] }
tracing = "0.1"
uuid = { version = "1.18", features = ["serde", "v7"] }
```

Do not add a `repository` package field until the public canonical URL exists; the production-hardening plan adds it in the same commit that creates the public release metadata.

- [ ] **Step 3: Create focused crate and service manifests**

For every library crate, create a `Cargo.toml` with only dependencies required by that crate. The minimum `crates/domain-types/Cargo.toml` is:

```toml
[package]
name = "domain-types"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
hex.workspace = true
serde.workspace = true
thiserror.workspace = true
```

The minimum service manifest pattern is:

```toml
[package]
name = "hl-capture"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
anyhow.workspace = true
tokio.workspace = true
tracing.workspace = true
```

Create each listed `src/lib.rs` with crate-level `#![forbid(unsafe_code)]` and a public `pub const CRATE_BOOTSTRAPPED: bool = true;`. Create each listed service `src/main.rs` with `#![forbid(unsafe_code)]`, `#[tokio::main]`, and `Ok::<(), anyhow::Error>(())`. Each package must compile without reaching the network. Do not add `services/hl-exec` to the V1 workspace.

Create `LICENSE` with the unmodified Apache License, Version 2.0 canonical text and add `Apache-2.0` SPDX metadata to package manifests. Update the existing root README to state that the current V1 is read-only and contains no execution or signing capability.

- [ ] **Step 4: Create the Swift package skeleton**

Create `apps/AlphaDesk/Package.swift`:

```swift
// swift-tools-version: 6.3
import PackageDescription

let package = Package(
    name: "AlphaDesk",
    platforms: [.macOS(.v15), .iOS(.v18)],
    products: [
        .library(name: "DeskDomain", targets: ["DeskDomain"]),
        .library(name: "DeskNetworking", targets: ["DeskNetworking"]),
        .library(name: "DeskStorage", targets: ["DeskStorage"]),
    ],
    targets: [
        .target(name: "DeskDomain"),
        .target(name: "DeskNetworking", dependencies: ["DeskDomain"]),
        .target(name: "DeskStorage", dependencies: ["DeskDomain"]),
        .testTarget(name: "DeskDomainTests", dependencies: ["DeskDomain"]),
    ],
    swiftLanguageModes: [.v6]
)
```

Create the listed bootstrap files. Each source file contains one public namespaced bootstrap enum with no cases; `BootstrapTests.swift` imports `DeskDomain` and asserts the module is loadable. These bootstrap files remain focused and may be deleted only in the task that replaces their last responsibility.

- [ ] **Step 5: Add common developer commands and verify**

Create `justfile`:

```make
set shell := ["bash", "-euo", "pipefail", "-c"]

fmt:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features
    swift test --package-path apps/AlphaDesk

check-workspace:
    ./tools/ci/check-workspace.sh

verify: check-workspace fmt clippy test
```

Run:

```bash
cargo generate-lockfile
just check-workspace
cargo check --workspace --all-targets
swift test --package-path apps/AlphaDesk
```

Expected: all commands exit 0 and print `workspace-layout:ok`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .cargo justfile .gitignore README.md LICENSE crates services apps tools/ci/check-workspace.sh
git commit -m "chore(workspace): bootstrap pinned Rust and Swift monorepo"
```

---

### Task 2: Implement strongly typed identifiers and checked fixed-point primitives

**Files:**
- Modify: `crates/domain-types/src/lib.rs`
- Create: `crates/domain-types/src/decimal.rs`
- Create: `crates/domain-types/src/ids.rs`
- Create: `crates/domain-types/src/shared.rs`
- Create: `crates/domain-types/src/time.rs`
- Create: `crates/domain-types/tests/decimal.rs`
- Create: `crates/domain-types/tests/ids.rs`
- Create: `crates/domain-types/tests/shared.rs`

**Interfaces:**
- Consumes: no venue, storage, wall-clock, or floating-point types.
- Produces: distinct monetary/rate newtypes over exact scaled `i128`, `ProbabilityPpm`, all core identifiers from design section 11.1 plus `RegimeId`, `CohortId`, `FeeScheduleId`, `SourceId`, `EvidenceId`, `ScenarioId`, `ManifestId`, and `LabelDefinitionId`, `ProtocolTime`, `KnownTime`, `ClosedInterval`, `Horizon`, `LatencyDistribution`, `Direction`, `CalibrationStatus`, `BlockRange`, and analytical conversion carrying source-scale metadata.

- [ ] **Step 1: Write exact-decimal, probability, ID, and shared-value tests**

Create `crates/domain-types/tests/decimal.rs`:

```rust
use domain_types::{AnalyticFloat, Price, ProbabilityPpm, Quantity, RoundingMode, ValueError};
use std::str::FromStr;

#[test]
fn price_parses_at_metadata_scale_and_formats_without_float() {
    let price = Price::parse_at_scale("12345.6789", 6).unwrap();
    assert_eq!(price.raw(), 12_345_678_900);
    assert_eq!(price.scale(), 6);
    assert_eq!(price.to_string(), "12345.678900");
}

#[test]
fn parsing_rejects_precision_beyond_metadata_scale() {
    let error = Quantity::parse_at_scale("1.0000001", 6).unwrap_err();
    assert_eq!(error, ValueError::ExcessPrecision { allowed: 6 });
}

#[test]
fn downscaling_requires_an_explicit_rounding_mode() {
    let price = Price::from_str("1.005").unwrap();
    assert_eq!(price.rescale(2, RoundingMode::NearestTiesToEven).unwrap().to_string(), "1.00");
    assert_eq!(price.rescale(2, RoundingMode::Ceiling).unwrap().to_string(), "1.01");
}

#[test]
fn analytical_conversion_retains_source_scale() {
    let converted: AnalyticFloat = Price::parse_at_scale("12.34", 4).unwrap().to_analytic_float();
    assert_eq!(converted.source_scale, 4);
    assert!((converted.value - 12.34).abs() < f64::EPSILON);
}

#[test]
fn probability_is_bounded_and_scales_integers_exactly() {
    assert!(ProbabilityPpm::from_ppm(1_000_001).is_err());
    let half = ProbabilityPpm::from_ppm(500_000).unwrap();
    assert_eq!(half.checked_scale_i128_toward_zero(101).unwrap(), 50);
}
```

Create `crates/domain-types/tests/shared.rs` with monotonic latency-distribution, non-empty block-range, and ordered closed-interval tests. Create `crates/domain-types/tests/ids.rs` with empty-ID rejection, `BlockHeight::new(42).get() == 42`, 20-byte address round trip, and lowercase API formatting tests.

Run:

```bash
cargo test -p domain-types
```

Expected: FAIL because the module and types do not exist.

- [ ] **Step 2: Implement exact scaled decimal arithmetic with explicit rounding**

Create `crates/domain-types/src/decimal.rs`:

```rust
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};
use thiserror::Error;

pub const MAX_DECIMAL_SCALE: u8 = 38;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyticFloat {
    pub value: f64,
    pub source_scale: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    TowardZero,
    Floor,
    Ceiling,
    NearestTiesToEven,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValueError {
    #[error("empty decimal")]
    Empty,
    #[error("invalid decimal")]
    Invalid,
    #[error("decimal has more than {allowed} fractional digits")]
    ExcessPrecision { allowed: u8 },
    #[error("scale {scale} exceeds maximum {maximum}")]
    ScaleOutOfRange { scale: u8, maximum: u8 },
    #[error("scales differ: left={left}, right={right}")]
    ScaleMismatch { left: u8, right: u8 },
    #[error("value is outside the permitted range")]
    OutOfRange,
    #[error("division by zero")]
    DivisionByZero,
    #[error("fixed-point arithmetic overflow")]
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal {
    raw: i128,
    scale: u8,
}

impl Decimal {
    pub fn from_raw(raw: i128, scale: u8) -> Result<Self, ValueError> {
        if scale > MAX_DECIMAL_SCALE {
            return Err(ValueError::ScaleOutOfRange { scale, maximum: MAX_DECIMAL_SCALE });
        }
        Ok(Self { raw, scale })
    }

    pub const fn raw(self) -> i128 { self.raw }
    pub const fn scale(self) -> u8 { self.scale }

    pub fn parse_at_scale(input: &str, scale: u8) -> Result<Self, ValueError> {
        let parsed = Self::from_str(input)?;
        if parsed.scale > scale {
            return Err(ValueError::ExcessPrecision { allowed: scale });
        }
        parsed.rescale(scale, RoundingMode::TowardZero)
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, ValueError> {
        self.require_same_scale(rhs)?;
        Self::from_raw(self.raw.checked_add(rhs.raw).ok_or(ValueError::Overflow)?, self.scale)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, ValueError> {
        self.require_same_scale(rhs)?;
        Self::from_raw(self.raw.checked_sub(rhs.raw).ok_or(ValueError::Overflow)?, self.scale)
    }

    pub fn checked_mul(
        self,
        rhs: Self,
        output_scale: u8,
        rounding: RoundingMode,
    ) -> Result<Self, ValueError> {
        let product = self.raw.checked_mul(rhs.raw).ok_or(ValueError::Overflow)?;
        let product_scale = self.scale.checked_add(rhs.scale).ok_or(ValueError::Overflow)?;
        Self::from_raw(product, product_scale)?.rescale(output_scale, rounding)
    }

    pub fn checked_div(
        self,
        rhs: Self,
        output_scale: u8,
        rounding: RoundingMode,
    ) -> Result<Self, ValueError> {
        if rhs.raw == 0 { return Err(ValueError::DivisionByZero); }
        let shift = i32::from(output_scale) + i32::from(rhs.scale) - i32::from(self.scale);
        let (numerator, denominator) = if shift >= 0 {
            let factor = pow10(u32::try_from(shift).map_err(|_| ValueError::Overflow)?)?;
            (self.raw.checked_mul(factor).ok_or(ValueError::Overflow)?, rhs.raw)
        } else {
            let factor = pow10(shift.unsigned_abs())?;
            (self.raw, rhs.raw.checked_mul(factor).ok_or(ValueError::Overflow)?)
        };
        Self::from_raw(div_round(numerator, denominator, rounding)?, output_scale)
    }

    pub fn rescale(self, target_scale: u8, rounding: RoundingMode) -> Result<Self, ValueError> {
        if target_scale > MAX_DECIMAL_SCALE {
            return Err(ValueError::ScaleOutOfRange { scale: target_scale, maximum: MAX_DECIMAL_SCALE });
        }
        match target_scale.cmp(&self.scale) {
            std::cmp::Ordering::Equal => Ok(self),
            std::cmp::Ordering::Greater => {
                let factor = pow10(u32::from(target_scale - self.scale))?;
                Self::from_raw(self.raw.checked_mul(factor).ok_or(ValueError::Overflow)?, target_scale)
            }
            std::cmp::Ordering::Less => {
                let factor = pow10(u32::from(self.scale - target_scale))?;
                Self::from_raw(div_round(self.raw, factor, rounding)?, target_scale)
            }
        }
    }

    pub fn to_analytic_float(self) -> AnalyticFloat {
        AnalyticFloat {
            value: self.raw as f64 / 10_f64.powi(i32::from(self.scale)),
            source_scale: self.scale,
        }
    }

    fn require_same_scale(self, rhs: Self) -> Result<(), ValueError> {
        if self.scale == rhs.scale { Ok(()) } else {
            Err(ValueError::ScaleMismatch { left: self.scale, right: rhs.scale })
        }
    }
}

fn pow10(exponent: u32) -> Result<i128, ValueError> {
    10_i128.checked_pow(exponent).ok_or(ValueError::Overflow)
}

fn div_round(numerator: i128, denominator: i128, mode: RoundingMode) -> Result<i128, ValueError> {
    if denominator == 0 { return Err(ValueError::DivisionByZero); }
    let quotient = numerator.checked_div(denominator).ok_or(ValueError::Overflow)?;
    let remainder = numerator.checked_rem(denominator).ok_or(ValueError::Overflow)?;
    if remainder == 0 || mode == RoundingMode::TowardZero { return Ok(quotient); }
    let same_sign = (numerator < 0) == (denominator < 0);
    let step = if same_sign { 1_i128 } else { -1_i128 };
    match mode {
        RoundingMode::TowardZero => Ok(quotient),
        RoundingMode::Floor => if same_sign { Ok(quotient) } else { quotient.checked_sub(1).ok_or(ValueError::Overflow) },
        RoundingMode::Ceiling => if same_sign { quotient.checked_add(1).ok_or(ValueError::Overflow) } else { Ok(quotient) },
        RoundingMode::NearestTiesToEven => {
            let twice_remainder = remainder.unsigned_abs().checked_mul(2).ok_or(ValueError::Overflow)?;
            let divisor = denominator.unsigned_abs();
            if twice_remainder < divisor || (twice_remainder == divisor && quotient % 2 == 0) {
                Ok(quotient)
            } else {
                quotient.checked_add(step).ok_or(ValueError::Overflow)
            }
        }
    }
}

impl FromStr for Decimal {
    type Err = ValueError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() { return Err(ValueError::Empty); }
        let (negative, unsigned) = if let Some(rest) = input.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = input.strip_prefix('+') {
            (false, rest)
        } else {
            (false, input)
        };
        if unsigned.is_empty() { return Err(ValueError::Invalid); }
        let mut parts = unsigned.split('.');
        let whole = parts.next().ok_or(ValueError::Invalid)?;
        let fraction = parts.next().unwrap_or("");
        if parts.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ValueError::Invalid);
        }
        let scale = u8::try_from(fraction.len()).map_err(|_| ValueError::ScaleOutOfRange {
            scale: u8::MAX,
            maximum: MAX_DECIMAL_SCALE,
        })?;
        if scale > MAX_DECIMAL_SCALE {
            return Err(ValueError::ScaleOutOfRange { scale, maximum: MAX_DECIMAL_SCALE });
        }
        let factor = pow10(u32::from(scale))?;
        let whole = whole.parse::<i128>().map_err(|_| ValueError::Overflow)?;
        let fractional = if fraction.is_empty() { 0 } else {
            fraction.parse::<i128>().map_err(|_| ValueError::Overflow)?
        };
        let raw = whole.checked_mul(factor)
            .and_then(|value| value.checked_add(fractional))
            .ok_or(ValueError::Overflow)?;
        Self::from_raw(if negative { raw.checked_neg().ok_or(ValueError::Overflow)? } else { raw }, scale)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let factor = 10_u128.pow(u32::from(self.scale));
        let absolute = self.raw.unsigned_abs();
        let whole = absolute / factor;
        let fraction = absolute % factor;
        if self.raw < 0 { write!(formatter, "-")?; }
        if self.scale == 0 {
            write!(formatter, "{whole}")
        } else {
            write!(formatter, "{whole}.{fraction:0width$}", width = usize::from(self.scale))
        }
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}
```

The only floating-point operation in this file is `to_analytic_float`; add a Clippy allow on that method rather than the crate. Canonical code never consumes `AnalyticFloat`.

- [ ] **Step 3: Generate distinct decimal newtypes and a bounded probability type**

Append to `decimal.rs`:

```rust
macro_rules! decimal_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Decimal);

        impl $name {
            pub fn from_raw(raw: i128, scale: u8) -> Result<Self, ValueError> {
                Decimal::from_raw(raw, scale).map(Self)
            }
            pub fn parse_at_scale(input: &str, scale: u8) -> Result<Self, ValueError> {
                Decimal::parse_at_scale(input, scale).map(Self)
            }
            pub const fn raw(self) -> i128 { self.0.raw() }
            pub const fn scale(self) -> u8 { self.0.scale() }
            pub fn checked_add(self, rhs: Self) -> Result<Self, ValueError> { self.0.checked_add(rhs.0).map(Self) }
            pub fn checked_sub(self, rhs: Self) -> Result<Self, ValueError> { self.0.checked_sub(rhs.0).map(Self) }
            pub fn rescale(self, scale: u8, rounding: RoundingMode) -> Result<Self, ValueError> {
                self.0.rescale(scale, rounding).map(Self)
            }
            pub fn to_analytic_float(self) -> AnalyticFloat { self.0.to_analytic_float() }
        }

        impl FromStr for $name {
            type Err = ValueError;
            fn from_str(input: &str) -> Result<Self, Self::Err> { input.parse::<Decimal>().map(Self) }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
        }
    };
}

decimal_newtype!(Price);
decimal_newtype!(Quantity);
decimal_newtype!(QuoteAmount);
decimal_newtype!(BaseAmount);
decimal_newtype!(UsdAmount);
decimal_newtype!(FundingRate);
decimal_newtype!(FeeRate);
decimal_newtype!(Leverage);
decimal_newtype!(MarginRatio);
decimal_newtype!(BasisPoints);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProbabilityPpm(u32);

impl ProbabilityPpm {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1_000_000);

    pub fn from_ppm(value: u32) -> Result<Self, ValueError> {
        if value <= 1_000_000 { Ok(Self(value)) } else { Err(ValueError::OutOfRange) }
    }

    pub const fn ppm(self) -> u32 { self.0 }

    pub fn checked_scale_i128_toward_zero(self, value: i128) -> Result<i128, ValueError> {
        value.checked_mul(i128::from(self.0))
            .and_then(|product| product.checked_div(1_000_000))
            .ok_or(ValueError::Overflow)
    }
}
```

`Price`, `Quantity`, `QuoteAmount`, and `BaseAmount` are parsed at the scale supplied by the effective market/asset metadata. Adding unlike monetary types is impossible at compile time. Multiplication and division occur through explicit domain functions that unwrap the two values, select an output scale, and name the rounding rule required by the protocol boundary.

- [ ] **Step 4: Implement all identifiers, timestamps, and shared types centrally**

Create `ids.rs`:

```rust
use crate::ValueError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
                let value = value.into();
                if value.is_empty() || value.trim() != value {
                    return Err(ValueError::Invalid);
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(ChainId);
string_id!(TransactionId);
string_id!(EventId);
string_id!(AccountId);
string_id!(MasterAccountId);
string_id!(VaultId);
string_id!(EntityId);
string_id!(ClusterVersionId);
string_id!(DexId);
string_id!(MarketId);
string_id!(AssetId);
string_id!(OrderId);
string_id!(ClientOrderId);
string_id!(TradeId);
string_id!(PositionEpisodeId);
string_id!(FeatureSetVersion);
string_id!(ModelVersion);
string_id!(SignalId);
string_id!(ExperimentId);
string_id!(RegimeId);
string_id!(CohortId);
string_id!(FeeScheduleId);
string_id!(SourceId);
string_id!(EvidenceId);
string_id!(ScenarioId);
string_id!(ManifestId);
string_id!(LabelDefinitionId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockHeight(u64);

impl BlockHeight {
    pub const fn new(value: u64) -> Self { Self(value) }
    pub const fn get(self) -> u64 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address([u8; 20]);

impl Address {
    pub const fn from_bytes(bytes: [u8; 20]) -> Self { Self(bytes) }
    pub const fn as_bytes(&self) -> &[u8; 20] { &self.0 }
    pub fn parse_api(input: &str) -> Result<Self, ValueError> {
        let hex_value = input.strip_prefix("0x").ok_or(ValueError::Invalid)?;
        if hex_value.len() != 40 || hex_value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(ValueError::Invalid);
        }
        let mut bytes = [0_u8; 20];
        hex::decode_to_slice(hex_value, &mut bytes).map_err(|_| ValueError::Invalid)?;
        Ok(Self(bytes))
    }
    pub fn to_api_string(self) -> String { format!("0x{}", hex::encode(self.0)) }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        serializer.serialize_str(&self.to_api_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        let value = String::deserialize(deserializer)?;
        Self::parse_api(&value).map_err(serde::de::Error::custom)
    }
}
```

Create `time.rs`:

```rust
use crate::ValueError;
use serde::{Deserialize, Serialize};

macro_rules! protocol_time {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub fn from_unix_micros(value: i64) -> Result<Self, ValueError> {
                if value < 0 { Err(ValueError::OutOfRange) } else { Ok(Self(value)) }
            }
            pub const fn unix_micros(self) -> i64 { self.0 }
        }
    };
}

protocol_time!(ProtocolTime);
protocol_time!(KnownTime);
```

Neither timestamp type provides a `now()` method; orchestration code must inject observed wall-clock values explicitly.

Create `shared.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedInterval<T> {
    pub lower: T,
    pub upper: T,
}

impl<T: PartialOrd> ClosedInterval<T> {
    pub fn new(lower: T, upper: T) -> Result<Self, ValueError> {
        if lower <= upper { Ok(Self { lower, upper }) } else { Err(ValueError::OutOfRange) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Horizon(u64);

impl Horizon {
    pub const MS_250: Self = Self(250_000);
    pub const SECOND_1: Self = Self(1_000_000);
    pub const SECONDS_5: Self = Self(5_000_000);
    pub const SECONDS_30: Self = Self(30_000_000);
    pub const MINUTES_2: Self = Self(120_000_000);
    pub const MINUTES_5: Self = Self(300_000_000);
    pub const MINUTES_30: Self = Self(1_800_000_000);
    pub const HOURS_4: Self = Self(14_400_000_000);
    pub const DAY_1: Self = Self(86_400_000_000);
    pub const fn from_micros(value: u64) -> Self { Self(value) }
    pub const fn as_micros(self) -> u64 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction { Long, Short, Flat }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalibrationStatus { Calibrated, UnderReview, InsufficientEvidence, Failed }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyDistribution {
    pub p10_micros: u64,
    pub p50_micros: u64,
    pub p90_micros: u64,
    pub p99_micros: u64,
}

impl LatencyDistribution {
    pub fn new(p10_micros: u64, p50_micros: u64, p90_micros: u64, p99_micros: u64) -> Result<Self, ValueError> {
        if p10_micros <= p50_micros && p50_micros <= p90_micros && p90_micros <= p99_micros {
            Ok(Self { p10_micros, p50_micros, p90_micros, p99_micros })
        } else {
            Err(ValueError::OutOfRange)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRange {
    pub start_inclusive: BlockHeight,
    pub end_inclusive: BlockHeight,
}

impl BlockRange {
    pub fn new(start_inclusive: BlockHeight, end_inclusive: BlockHeight) -> Result<Self, ValueError> {
        if start_inclusive <= end_inclusive {
            Ok(Self { start_inclusive, end_inclusive })
        } else {
            Err(ValueError::OutOfRange)
        }
    }
}
```

Export the modules from `lib.rs`. This task owns these shared names; later plans import them from `domain-types` instead of redefining them.

- [ ] **Step 5: Add property tests and run the crate suite**

Add Proptest cases for decimal parse/display/Serde round trips, rescale laws, checked overflow, tie-to-even behavior across signs, probability bounds, timestamp round trips, ordered intervals, and monotonic latency percentiles. Run:

```bash
cargo test -p domain-types --all-features
cargo clippy -p domain-types --all-targets -- -D warnings
rg 'f32|f64' crates/domain-types/src --glob '*.rs'
```

Expected: tests and Clippy PASS. The search returns only `AnalyticFloat.value`, `to_analytic_float`, and the conversion expression inside that explicitly named method.

- [ ] **Step 6: Commit**

```bash
git add crates/domain-types Cargo.toml Cargo.lock
git commit -m "feat(domain): add typed identifiers and exact decimal values"
```

---

### Task 3: Define versioned canonical and API contract schemas

**Files:**
- Create: `schemas/proto/canonical/v1/events.proto`
- Create: `schemas/proto/common/v1/types.proto`
- Create: `schemas/proto/stream/v1/envelope.proto`
- Create: `crates/api-contracts/build.rs`
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Create: `crates/canonical-events/tests/envelope.rs`
- Create: `tools/schema-check/src/main.rs`

**Interfaces:**
- Consumes: `domain-types` identifiers and exact decimal strings.
- Produces: semantic-major-v1 Protobuf contracts, Rust domain envelope types, generated wire types, and a compatibility checker used by every service and client.

- [ ] **Step 1: Write a round-trip test for the canonical envelope**

Create `crates/canonical-events/tests/envelope.rs`:

```rust
use canonical_events::{CanonicalEventEnvelope, ConfirmationClass, EventKind};
use domain_types::{BlockHeight, ChainId, EventId, ProtocolTime, TransactionId};

#[test]
fn envelope_round_trip_preserves_identity_and_order() {
    let envelope = CanonicalEventEnvelope::fixture();
    let bytes = envelope.encode_to_vec().unwrap();
    let decoded = CanonicalEventEnvelope::decode(&bytes).unwrap();
    assert_eq!(decoded.event_id(), envelope.event_id());
    assert_eq!(decoded.ordering_key(), envelope.ordering_key());
    assert_eq!(decoded.confirmation_class(), ConfirmationClass::CommittedPrimary);
    assert_eq!(decoded.event_kind(), EventKind::TradeMatched);
}
```

Run:

```bash
cargo test -p canonical-events --test envelope
```

Expected: FAIL because the contract is absent.

- [ ] **Step 2: Create the common and canonical Protobuf schemas**

The envelope in `schemas/proto/canonical/v1/events.proto` must contain these exact fields and stable field numbers:

```proto
syntax = "proto3";
package hl.canonical.v1;

import "common/v1/types.proto";

message CanonicalEventEnvelope {
  string schema_version = 1;
  string chain_id = 2;
  uint64 block_height = 3;
  int64 block_time_micros = 4;
  string transaction_id = 5;
  uint32 transaction_index = 6;
  uint32 event_index = 7;
  string event_id = 8;
  string event_kind = 9;
  repeated string market_ids = 10;
  repeated string account_ids = 11;
  repeated SourceEvidence source_evidence = 12;
  ConfirmationClass confirmation_class = 13;
  int64 observed_at_micros = 14;
  int64 ingested_at_micros = 15;
  int64 canonicalized_at_micros = 16;
  bytes payload_hash = 17;
  string parser_version = 18;
  bytes payload = 19;
}

message SourceEvidence {
  string source_id = 1;
  string source_version = 2;
  string source_offset = 3;
  bytes content_hash = 4;
}

enum ConfirmationClass {
  CONFIRMATION_CLASS_UNSPECIFIED = 0;
  PROVISIONAL_SOURCE = 1;
  COMMITTED_PRIMARY = 2;
  COMMITTED_INDEPENDENT = 3;
  RECONCILED_SNAPSHOT = 4;
  CORRECTED = 5;
  EXPIRED = 6;
}
```

Create separate payload messages for every V1 event family listed in design section 11.2. Reserve removed field numbers rather than reusing them.

- [ ] **Step 3: Generate wire types and map them to domain types**

Use `tonic-build`/`prost-build` in `crates/api-contracts/build.rs`. Keep generated types inside `api-contracts`; `canonical-events` exposes domain enums and explicit `TryFrom` conversions so vendor or generated types never leak into reducers.

Implement errors:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(String),
    #[error("missing required field {0}")]
    Missing(&'static str),
    #[error("invalid field {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("wire decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
}
```

- [ ] **Step 4: Implement schema compatibility enforcement**

`tools/schema-check` compares the current descriptor set to `schemas/proto/baseline/v1.pb` and fails on removed fields, field-number reuse, incompatible type changes, or semantic-major changes without a new package path.

Run:

```bash
cargo run -p schema-check -- check schemas/proto/baseline/v1.pb target/schema/current.pb
cargo test -p canonical-events -p api-contracts
```

Expected: PASS and output `schema-compatibility:compatible`.

- [ ] **Step 5: Commit**

```bash
git add schemas/proto crates/api-contracts crates/canonical-events tools/schema-check Cargo.toml Cargo.lock
git commit -m "feat(contracts): add canonical v1 protobuf schemas"
```

---

### Task 4: Build the deterministic fixture and golden-test harness

**Files:**
- Modify: `crates/test-fixtures/src/lib.rs`
- Create: `crates/test-fixtures/src/builders.rs`
- Create: `crates/test-fixtures/src/manifest.rs`
- Create: `crates/test-fixtures/tests/manifest.rs`
- Create: `fixtures/golden/blocks/minimal-trade.json`
- Create: `fixtures/golden/expected/minimal-trade.canonical.json`
- Generate: `fixtures/golden/manifest.toml`
- Create: `tools/fixture-inspect/src/main.rs`

**Interfaces:**
- Consumes: canonical V1 wire/domain contracts and exact domain values.
- Produces: immutable, hash-verified fixture bundles; a deterministic manifest generator/verifier; and seeded builders for unit, property, replay, differential, and Swift contract tests.

- [ ] **Step 1: Write failing fixture-manifest verification tests**

Create `crates/test-fixtures/tests/manifest.rs`:

```rust
use std::path::Path;
use test_fixtures::FixtureManifest;

#[test]
fn every_declared_fixture_and_expected_output_matches_its_hash() {
    let root = Path::new("../../fixtures/golden");
    let manifest = FixtureManifest::load(root.join("manifest.toml")).unwrap();
    manifest.verify(root).unwrap();
}

#[test]
fn undeclared_files_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    std::fs::create_dir_all(temporary.path().join("expected")).unwrap();
    std::fs::write(temporary.path().join("blocks/orphan.json"), b"{}\n").unwrap();
    let error = FixtureManifest::empty().verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("undeclared fixture file"));
}
```

Add `tempfile = "3.20"` as a workspace development dependency for fixture tests. Run `cargo test -p test-fixtures --test manifest`; expect FAIL because the manifest implementation is absent.

- [ ] **Step 2: Implement a canonical manifest and deterministic generator**

Define these exact manifest records in `manifest.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FixtureManifest {
    pub version: u32,
    pub fixture: Vec<FixtureEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FixtureEntry {
    pub id: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_schema: String,
    pub expected_path: String,
    pub expected_sha256: String,
    pub expected_schema: String,
}
```

`FixtureManifest::verify` rejects missing files, duplicate IDs or paths, uppercase/non-64-character digests, digest mismatches, path traversal, symlinks, and undeclared regular files below `blocks/` or `expected/`. `fixture-inspect generate-manifest --root fixtures/golden` scans in bytewise path order, writes lowercase SHA-256 values to a temporary file, fsyncs it, atomically renames it to `manifest.toml`, and prints the resulting manifest SHA-256.

Create these exact UTF-8 fixture files with a final newline:

`fixtures/golden/blocks/minimal-trade.json`

```json
{"account_ids":["0x1111111111111111111111111111111111111111","0x2222222222222222222222222222222222222222"],"block_height":42,"block_time_micros":1721779200000042,"event_index":0,"event_kind":"trade_matched","market_id":"perp:BTC","price":"65000.000000","quantity":"0.01000000","schema":"hl.source.fixture.v1","transaction_id":"fixture-tx-42-0","transaction_index":0}
```

`fixtures/golden/expected/minimal-trade.canonical.json`

```json
{"account_ids":["0x1111111111111111111111111111111111111111","0x2222222222222222222222222222222222222222"],"block_height":42,"block_time_micros":1721779200000042,"confirmation_class":"committed_primary","event_id":"fixture-mainnet-42-0-0","event_index":0,"event_kind":"trade_matched","market_ids":["perp:BTC"],"parser_version":"fixture-parser-v1","payload":{"price":"65000.000000","quantity":"0.01000000"},"schema_version":"1.0.0","transaction_id":"fixture-tx-42-0","transaction_index":0}
```

Generate the manifest only after both files are committed in the working tree:

```bash
cargo run -p fixture-inspect -- generate-manifest --root fixtures/golden
cargo run -p fixture-inspect -- verify fixtures/golden/manifest.toml
```

The first command writes concrete digests; no hand-edited hash token is permitted.

- [ ] **Step 3: Add deterministic scenario builders with explicit defaults**

Create `builders.rs`:

```rust
use canonical_events::{CanonicalEventEnvelope, ConfirmationClass, EventPayload, TradeMatched};
use domain_types::{Address, BlockHeight, EventId, MarketId, Price, ProtocolTime, Quantity, TransactionId};

pub struct TradeScenarioBuilder {
    block_height: BlockHeight,
    transaction_index: u32,
    event_index: u32,
    seed: u64,
}

impl TradeScenarioBuilder {
    pub fn at_block(block_height: u64) -> Self {
        Self {
            block_height: BlockHeight::new(block_height),
            transaction_index: 0,
            event_index: 0,
            seed: 0,
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn matched_trade(self, buyer: Address, seller: Address) -> CanonicalEventEnvelope {
        let height = self.block_height.get();
        CanonicalEventEnvelope::try_new(
            "1.0.0",
            "mainnet",
            self.block_height,
            ProtocolTime::from_unix_micros(1_721_779_200_000_000_i64 + i64::try_from(height).unwrap()).unwrap(),
            TransactionId::new(format!("fixture-tx-{height}-{}", self.transaction_index)).unwrap(),
            self.transaction_index,
            self.event_index,
            EventId::new(format!("fixture-mainnet-{height}-{}-{}", self.transaction_index, self.event_index)).unwrap(),
            vec![MarketId::new("perp:BTC").unwrap()],
            vec![buyer, seller],
            ConfirmationClass::CommittedPrimary,
            EventPayload::TradeMatched(TradeMatched {
                price: Price::parse_at_scale("65000", 6).unwrap(),
                quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
                deterministic_seed: self.seed,
            }),
            "fixture-parser-v1",
        ).unwrap()
    }
}
```

Task 3 must expose the shown `CanonicalEventEnvelope::try_new` constructor and payload types. Builders never read wall time and never use unseeded randomness.

- [ ] **Step 4: Verify hash stability and generated fixtures**

Run:

```bash
cargo run -p fixture-inspect -- generate-manifest --root fixtures/golden
cp fixtures/golden/manifest.toml target/manifest.first.toml
cargo run -p fixture-inspect -- generate-manifest --root fixtures/golden
cmp target/manifest.first.toml fixtures/golden/manifest.toml
cargo run -p fixture-inspect -- verify fixtures/golden/manifest.toml
cargo test -p test-fixtures
```

Expected: every command PASS; manifest generation is byte-identical on the second run; fixture summary is sorted by fixture ID.

- [ ] **Step 5: Commit**

```bash
git add crates/test-fixtures fixtures/golden tools/fixture-inspect Cargo.toml Cargo.lock
git commit -m "test(fixtures): add hash-verified deterministic golden harness"
```

---

### Task 5: Establish telemetry, build provenance, and data-health contracts

**Files:**
- Modify: `crates/telemetry/src/lib.rs`
- Create: `crates/telemetry/src/health.rs`
- Create: `crates/telemetry/src/metrics.rs`
- Create: `crates/telemetry/src/provenance.rs`
- Create: `crates/telemetry/tests/health.rs`
- Create: `schemas/proto/health/v1/health.proto`
- Create: `tools/build-info/build.rs`

**Interfaces:**
- Consumes: typed IDs, protocol/known time, and service name/version.
- Produces: `HealthState`, scoped health assessments, OpenTelemetry initialization, Prometheus metrics registration, and immutable build provenance attached to archives, models, APIs, and logs.

- [ ] **Step 1: Write health aggregation tests**

```rust
#[test]
fn aggregate_uses_most_severe_required_dependency() {
    let health = HealthAssessment::aggregate([
        HealthAssessment::green("primary"),
        HealthAssessment::amber("secondary", "temporarily unavailable"),
        HealthAssessment::red("book:BTC", "sequence gap"),
    ]);
    assert_eq!(health.state, HealthState::Red);
    assert!(health.suppresses("market:BTC:capacity"));
}
```

Run `cargo test -p telemetry --test health`; expect FAIL.

- [ ] **Step 2: Implement health types and suppression dependencies**

Use exhaustive types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthState { Green, Amber, Red }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthAssessment {
    pub scope: String,
    pub state: HealthState,
    pub reason_code: String,
    pub observed_at_micros: i64,
    pub suppresses: Vec<String>,
}
```

Aggregation must be deterministic and stable-sort reasons by scope and reason code.

- [ ] **Step 3: Add structured tracing and metric initialization**

Expose one entry point:

```rust
pub fn init_telemetry(config: &TelemetryConfig, build: &BuildProvenance) -> Result<TelemetryGuard, TelemetryError>;
```

It installs JSON logs, trace IDs, service/build fields, OTLP export when configured, and a Prometheus registry. Repeated initialization returns an explicit `AlreadyInitialized` error.

- [ ] **Step 4: Generate immutable build provenance**

`BuildProvenance` contains Git SHA, dirty flag, Rust version, target triple, build timestamp from `SOURCE_DATE_EPOCH`, schema fingerprint, and Cargo lock hash. Builds without `SOURCE_DATE_EPOCH` are allowed in development but marked non-reproducible; release builds fail.

Run:

```bash
SOURCE_DATE_EPOCH=1784894400 cargo test -p telemetry
SOURCE_DATE_EPOCH=1784894400 cargo run -p build-info -- print
```

Expected: PASS and stable JSON for repeated builds from the same checkout.

- [ ] **Step 5: Commit**

```bash
git add crates/telemetry schemas/proto/health tools/build-info Cargo.toml Cargo.lock
git commit -m "feat(telemetry): add health contracts and build provenance"
```

---

### Task 6: Enforce dependency direction and supply-chain policy

**Files:**
- Create: `deny.toml`
- Create: `tools/architecture-check/src/main.rs`
- Create: `tools/architecture-check/tests/fixtures/invalid-cycle.json`
- Create: `tools/ci/check-unsafe.sh`
- Modify: `justfile`
- Create: `docs/adr/0001-dependency-direction.md`
- Create: `docs/engineering/dependency-policy.md`

**Interfaces:**
- Consumes: Cargo metadata and approved dependency direction.
- Produces: automated failure on forbidden crate edges, cyclic dependencies, unreviewed unsafe code, vulnerable/advisory dependencies, and disallowed licenses.

- [ ] **Step 1: Write architecture-check tests for forbidden edges**

The checker must reject `domain-types -> storage-ports`, `feature-core -> model-runtime`, any domain crate depending on a service, and any dependency on a crate named `hl-exec` in V1.

Run:

```bash
cargo test -p architecture-check
```

Expected: FAIL until the checker is implemented.

- [ ] **Step 2: Implement the dependency policy graph**

Represent allowed layers in code, parse `cargo metadata`, and report the shortest forbidden path. The output format is:

```text
forbidden-dependency: feature-core -> model-runtime
rule: feature definitions must not depend on an inference runtime
```

- [ ] **Step 3: Configure dependency and license checks**

`deny.toml` must deny known vulnerabilities, duplicate major versions unless explicitly documented, unknown registries, git dependencies except reviewed allowlisted commits, and licenses outside Apache-2.0/MIT/BSD/ISC/Unicode/CDLA-Permissive families.

- [ ] **Step 4: Detect unreviewed unsafe blocks**

`tools/ci/check-unsafe.sh` fails on `unsafe` outside an allowlist file containing path, line hash, reviewer, rationale, and expiry. An empty allowlist is the Stage 0 expected state.

- [ ] **Step 5: Add and run the quality target**

Add to `justfile`:

```make
architecture:
    cargo run -p architecture-check -- check
    ./tools/ci/check-unsafe.sh

deny:
    cargo deny check

quality: fmt clippy architecture deny
```

Run `just quality`; expect PASS.

- [ ] **Step 6: Commit**

```bash
git add deny.toml tools/architecture-check tools/ci/check-unsafe.sh justfile docs/adr/0001-dependency-direction.md docs/engineering/dependency-policy.md Cargo.toml Cargo.lock
git commit -m "chore(quality): enforce architecture and dependency policy"
```

---

### Task 7: Create the isolated local development stack

**Files:**
- Create: `infra/docker-compose/compose.yaml`
- Create: `infra/docker-compose/nats/nats.conf`
- Create: `infra/docker-compose/clickhouse/config.xml`
- Create: `infra/docker-compose/postgres/init.sql`
- Create: `infra/docker-compose/minio/create-buckets.sh`
- Create: `infra/docker-compose/otel/collector.yaml`
- Create: `infra/docker-compose/victoriametrics/prometheus.yml`
- Create: `tools/ci/wait-for-dev-stack.sh`
- Modify: `justfile`

**Interfaces:**
- Consumes: no production credentials; all ports bind to loopback.
- Produces: deterministic local NATS, ClickHouse, PostgreSQL, MinIO, OpenTelemetry Collector, and VictoriaMetrics services with health checks and named volumes.

- [ ] **Step 1: Write a failing dependency smoke test**

`tools/ci/wait-for-dev-stack.sh` must check:

```bash
curl --fail --silent http://127.0.0.1:8222/healthz
curl --fail --silent http://127.0.0.1:8123/ping
pg_isready -h 127.0.0.1 -p 5432 -U alpha
curl --fail --silent http://127.0.0.1:9000/minio/health/live
```

Run it before Compose; expect FAIL.

- [ ] **Step 2: Define pinned containers and loopback-only networking**

The Compose file uses exact image digests recorded in `infra/docker-compose/images.lock`; ports bind as `127.0.0.1:PORT:PORT`; containers run with read-only root filesystems where supported, dropped capabilities, health checks, and explicit memory limits.

Create NATS streams during initialization for `HL_CANONICAL`, `HL_STATE`, `HL_FEATURE`, `HL_SIGNAL`, `HL_HEALTH`, and `HL_DEADLETTER` with six-hour development retention.

- [ ] **Step 3: Add local lifecycle commands**

```make
dev-up:
    docker compose -f infra/docker-compose/compose.yaml up -d
    ./tools/ci/wait-for-dev-stack.sh

dev-down:
    docker compose -f infra/docker-compose/compose.yaml down

dev-reset:
    docker compose -f infra/docker-compose/compose.yaml down -v
```

- [ ] **Step 4: Start and verify the stack**

Run:

```bash
just dev-up
./tools/ci/wait-for-dev-stack.sh
just dev-down
```

Expected: health script prints one `ok` line per dependency and exits 0.

- [ ] **Step 5: Commit**

```bash
git add infra/docker-compose tools/ci/wait-for-dev-stack.sh justfile
git commit -m "chore(dev): add pinned local data infrastructure"
```

---

### Task 8: Add CI, reproducible builds, and cross-platform verification

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/schema.yml`
- Create: `.github/workflows/security.yml`
- Create: `tools/ci/verify-reproducible-build.sh`
- Create: `tools/ci/check-generated.sh`
- Modify: `justfile`

**Interfaces:**
- Consumes: all Stage 0 commands.
- Produces: required checks for formatting, linting, tests, schemas, supply chain, generated-code cleanliness, Swift strict-concurrency builds, and reproducibility.

- [ ] **Step 1: Make generated-code drift detectable locally**

`tools/ci/check-generated.sh` runs schema generation, fixture generation, and build-info generation, then fails if `git diff --exit-code` is non-zero.

Run before wiring generation; expect FAIL until all generators are deterministic.

- [ ] **Step 2: Create the CI matrix**

`ci.yml` contains:

- Linux Rust workspace format, clippy, nextest, doctests, and architecture checks.
- macOS Swift package build/test in Swift 6 mode.
- Linux integration job with the local Compose stack.
- Dependency cache keyed by `Cargo.lock`, `Package.resolved`, toolchain, and target.
- Minimal token permissions and no privileged pull-request execution.

- [ ] **Step 3: Create schema and security workflows**

`schema.yml` verifies Protobuf compatibility and generated artifacts. `security.yml` runs `cargo deny`, `cargo audit`, SBOM generation, secret scanning, and image scan against pinned dependency containers.

- [ ] **Step 4: Add reproducibility verification**

`verify-reproducible-build.sh` builds release binaries twice in clean target directories with the same `SOURCE_DATE_EPOCH`, strips paths through remap flags, and compares SHA-256 hashes. The script prints differing sections and fails if hashes differ.

- [ ] **Step 5: Run all CI commands locally**

```bash
just quality
just test
./tools/ci/check-generated.sh
SOURCE_DATE_EPOCH=1784894400 ./tools/ci/verify-reproducible-build.sh
```

Expected: all commands pass.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows tools/ci justfile
git commit -m "ci: add reproducible cross-platform quality gates"
```

---

### Task 9: Create production deployment scaffolding without enabling services

**Files:**
- Create: `infra/ansible/ansible.cfg`
- Create: `infra/ansible/inventory/example/hosts.yml`
- Create: `infra/ansible/group_vars/all.yml`
- Create: `infra/ansible/roles/common/tasks/main.yml`
- Create: `infra/ansible/roles/alpha_service/tasks/main.yml`
- Create: `infra/systemd/hl-service@.service`
- Create: `infra/systemd/hl-service.env.example`
- Create: `infra/podman/quadlet/nats.container`
- Create: `infra/podman/quadlet/postgresql.container`
- Create: `infra/podman/quadlet/clickhouse.container`
- Create: `infra/podman/quadlet/minio.container`
- Create: `infra/monitoring/alerts/foundations.yml`
- Create: `infra/backup/README.md`
- Create: `docs/runbooks/service-lifecycle.md`

**Interfaces:**
- Consumes: five V1 service names and production security zones.
- Produces: idempotent host preparation, hardened service template, container definitions for stateful dependencies, and documented service lifecycle; it deploys no production workload in Stage 0.

- [ ] **Step 1: Write an Ansible syntax/idempotence test harness**

Create `infra/ansible/tests/check.sh` that runs `ansible-playbook --syntax-check` and two Molecule/converge passes against an Ubuntu 24.04 container, failing if the second pass reports changes.

- [ ] **Step 2: Implement the hardened systemd unit template**

The template must include:

```ini
[Service]
User=hl-%i
Group=hl-%i
ExecStart=/opt/hyperliquid-alpha-desk/bin/%i --config /etc/hyperliquid-alpha-desk/%i.toml
Restart=on-failure
RestartSec=2s
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LockPersonality=true
MemoryDenyWriteExecute=true
CapabilityBoundingSet=
AmbientCapabilities=
ReadWritePaths=/var/lib/hyperliquid-alpha-desk/%i /var/log/hyperliquid-alpha-desk/%i
LimitNOFILE=1048576
```

Add service-specific overrides only where justified by measured requirements.

- [ ] **Step 3: Implement common host hardening and directory ownership**

The Ansible role creates dedicated service users, UTC/chrony configuration, secure SSH policy, firewall defaults, data directories, log rotation, kernel limits, and package pinning. It must not store secrets in Git.

- [ ] **Step 4: Verify syntax and idempotence**

```bash
./infra/ansible/tests/check.sh
systemd-analyze verify infra/systemd/hl-service@.service
```

Expected: PASS with zero idempotence changes.

- [ ] **Step 5: Commit**

```bash
git add infra/ansible infra/systemd infra/podman infra/monitoring/alerts/foundations.yml infra/backup/README.md docs/runbooks/service-lifecycle.md
git commit -m "chore(infra): add hardened deployment scaffolding"
```

---

### Task 10: Implement the Stage 0 gate and record reproducible evidence

**Files:**
- Create: `tools/stage-gate/src/main.rs`
- Create: `tools/stage-gate/tests/config.rs`
- Create: `config/stage-gates/stage-0.toml`
- Generate after verification: `docs/stage-gates/stage-0-foundations.evidence.json`
- Create after approval: `docs/stage-gates/stage-0-foundations.md`
- Modify: `justfile`
- Modify: `README.md`

**Interfaces:**
- Consumes: workspace, quality, schema, fixture, local dependency, CI-equivalent, and infrastructure checks.
- Produces: a clean-commit machine-verifiable Stage 0 report, signed human approval record, and signed tag required by the Truth Layer plan.

- [ ] **Step 1: Write gate-runner tests before implementation**

Tests must prove that duplicate check IDs, missing commands, missing artifacts, malformed expected hashes, non-zero subprocess status, a dirty worktree, a mismatched design SHA, or a missing reviewer role prevent `PASS`. A successful fixture uses a temporary Git repository with one clean commit and asserts canonical JSON field ordering.

Run:

```bash
cargo test -p stage-gate --test config
```

Expected: FAIL because the gate runner does not exist.

- [ ] **Step 2: Implement the gate runner and exact Stage 0 configuration**

`stage-gate run config/stage-gates/stage-0.toml --output target/stage-gates/stage-0.json` must:

1. Require an empty `git status --porcelain` result before executing checks.
2. Verify `design-approved-v1.0.0` resolves to the approved design commit recorded in configuration.
3. Record the current clean implementation commit before any command runs.
4. Run `just verify`, `just quality`, schema checks, fixture checks, Compose smoke test, Ansible verification, and reproducibility checks in the configured order.
5. Hash `Cargo.lock`, the schema descriptor set, fixture manifest, toolchain files, and produced binaries with SHA-256.
6. Emit canonical JSON to ignored `target/stage-gates/stage-0.json`; never modify tracked files during verification.
7. Exit non-zero unless every required check and artifact succeeds.

Add:

```make
stage-0-gate:
    cargo run -p stage-gate -- run config/stage-gates/stage-0.toml --output target/stage-gates/stage-0.json
```

- [ ] **Step 3: Commit gate tooling and freeze the implementation commit**

```bash
git add tools/stage-gate config/stage-gates/stage-0.toml justfile README.md Cargo.toml Cargo.lock
git commit -m "chore(gate): add Stage 0 verification tooling"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

Record the printed SHA as the Stage 0 implementation commit. Do not amend this commit after verification begins.

- [ ] **Step 4: Run verification from the clean commit on two builders**

```bash
just stage-0-gate
sha256sum target/stage-gates/stage-0.json > target/stage-gates/stage-0.builder-a.sha256
```

Repeat from a fresh clone on the second supported builder and compare canonical report hashes. Expected: PASS and identical hashes after excluding the explicitly documented builder-identity field from the canonical comparison view.

- [ ] **Step 5: Commit evidence, collect approvals, and create the signed stage tag**

Copy the canonical report without modification and write the approval record using the concrete values defined in the program roadmap:

```bash
cp target/stage-gates/stage-0.json docs/stage-gates/stage-0-foundations.evidence.json
cargo run -p stage-gate -- render-record \
  --evidence docs/stage-gates/stage-0-foundations.evidence.json \
  --output docs/stage-gates/stage-0-foundations.md
```

Platform/data and independent reviewers sign detached approval files referenced by the record. Then run:

```bash
git add docs/stage-gates/stage-0-foundations.evidence.json docs/stage-gates/stage-0-foundations.md
git commit -m "docs(gate): record Stage 0 foundations evidence"
git tag -s stage-0-foundations -m "Stage 0 foundations gate passed"
git verify-tag stage-0-foundations
```

Do not create the tag if either required approval, the second-builder comparison, or any check is absent.

## Stage 0 Exit Criteria

- Clean checkout builds reproducibly with the pinned Rust and Swift toolchains.
- All crate/service names and dependency boundaries match the approved architecture.
- Exact fixed-point and typed identity contracts are tested and versioned.
- Canonical Protobuf compatibility checks and golden fixture hashes are operational.
- Health, telemetry, and provenance contracts exist before data ingestion.
- Local dependencies start on loopback only and pass health checks.
- CI-equivalent, supply-chain, architecture, and infrastructure checks pass.
- `docs/stage-gates/stage-0-foundations.md` is approved and tag `stage-0-foundations` exists.
