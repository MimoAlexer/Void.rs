# Protocol fuzzing workspace

This independent workspace fuzzes the existing public `void-protocol` APIs; it
does not change the root workspace or product build. No network, accounts, asset
downloads, game server or EULA acceptance is involved.

| Target | Input and checks |
| --- | --- |
| `frame_codec` | Byte 0 selects compression none/0/256; byte 1 selects a 1–256-byte fragment size; the rest is arbitrary wire frames. Decode incrementally, reject errors, and re-frame successful packets for a round-trip assertion. Exercise packet primitives on decoded bytes. |
| `network_nbt` | Arbitrary unnamed-root network NBT, including text extraction. Decoder errors are expected; panics and uncontrolled resource consumption are defects. |

Each harness rejects inputs over **65,536 bytes**. The frame harness processes at
most 4,096 packets per input. Production frame/decompression/NBT bounds still
apply (including the 8 MiB decompressed-packet ceiling). These bounds are not a
proof of safety; sanitizers, resource limits and retained crash artifacts remain
necessary.

## Deterministic seed check

From the repository root, with the normal pinned Rust toolchain:

```text
cargo run --manifest-path fuzz/Cargo.toml --bin seed_check
```

This invokes the exact harness logic for committed small seeds plus one high-bit
mutation per byte. It does not require libFuzzer or nightly. A passing seed check
is **not** a coverage-guided fuzz campaign, security certification or a substitute
for official packet fixtures.

Verified locally on 2026-09-26: **9 seed files and 359 deterministic mutations**
completed successfully on Windows with Rust 1.98.1. The actual libFuzzer targets,
sanitizer instrumentation and nightly coverage-guided execution remain unverified.

The committed `seeds/` examples include normal/fragmented frames, zlib compression,
overlong frame lengths, a network NBT compound, modified-UTF-8 text, excessive
nesting and a negative array length. They are synthetic wire data, not game assets
or recordings of user sessions. Evolving corpora and crash artifacts are ignored
under this workspace; retain valuable minimized regressions intentionally.

## Coverage-guided runs

Use a configured Linux x86-64 development environment with a C++ compiler and
nightly Rust. The [Rust Fuzz setup guide](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html)
documents libFuzzer/sanitizer prerequisites, including separate Windows setup.
The current local Windows task has no `cargo-fuzz` or nightly toolchain installed;
no coverage-guided result is claimed here.

From the repository root:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
mkdir -p fuzz/corpus/frame_codec fuzz/corpus/network_nbt
cp fuzz/seeds/frame_codec/* fuzz/corpus/frame_codec/
cp fuzz/seeds/network_nbt/* fuzz/corpus/network_nbt/
cargo +nightly fuzz run frame_codec --features fuzzing -- -max_len=65536 -timeout=5 -rss_limit_mb=512 -max_total_time=60
cargo +nightly fuzz run network_nbt --features fuzzing -- -max_len=65536 -timeout=5 -rss_limit_mb=512 -max_total_time=60
```

The `fuzzing` feature enables the optional libFuzzer dependency and targets. Keep
the nightly override on these commands rather than changing the product's pinned
toolchain. The initial 60-second limits are smoke campaigns; release qualification
requires sustained runs with recorded toolchain, sanitizer, seed/corpus hashes,
duration, coverage and findings. A time/RSS limit failure needs investigation,
not silent exclusion of the input.

Minimize and reproduce a finding using the
[cargo-fuzz commands](https://github.com/rust-fuzz/cargo-fuzz):

```sh
cargo +nightly fuzz tmin frame_codec fuzz/artifacts/frame_codec/crash-INPUT --features fuzzing
cargo +nightly fuzz run frame_codec fuzz/artifacts/frame_codec/crash-INPUT --features fuzzing
```

Turn every confirmed minimized failure into a normal protocol regression test
before fixing it. Keep the original artifact and record whether the failure came
from malformed-input handling, a round-trip assertion, timeout or sanitizer.
