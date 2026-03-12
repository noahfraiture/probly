# AGENTS.md

## Project
`probly` — Rust library for probabilistic / approximate analytics primitives.

Goal: provide small, mergeable streaming data structures (HLL, Bloom filter, Top-K, etc.) usable from Rust and via a C ABI.

## Crates
- `probly-core` — core algorithms and data structures
- `probly-ffi` — C ABI wrapper around `probly-core`

## Primitives (v1)
- HyperLogLog — approximate distinct count
- Bloom filter — approximate membership
- Top-K — heavy hitters

## Design principles
- streaming updates (`add`)
- mergeable states
- deterministic hashing
- serialization / deserialization
- safe Rust core, thin FFI layer
- no panics across FFI

## API pattern
Lifecycle for all primitives:

```

new → add → merge → estimate/result → serialize

```

## FFI
C ABI symbols use the `probly_` prefix and opaque pointers.

Example:

```

probly_hll_new()
probly_hll_add()
probly_hll_estimate()
probly_hll_merge()
probly_hll_free()

```
