# Bindings

This directory contains reusable language bindings built on top of the public C ABI.

Current layout:

- `go/` — importable Go package wrapping the C ABI

Conventions:

- The stable native boundary lives in [`include/probly.h`](../include/probly.h).
- Reusable language bindings live under `bindings/<language>/`.
- Consumer programs and experiments live under `examples/`.

Current status:

- Go bindings are usable from a repository checkout or from another local module via `replace`.
- The Go bindings are not yet a zero-setup `go get` package because they still require Rust build
  artifacts and `cgo`.
