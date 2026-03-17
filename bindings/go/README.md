# Go Bindings

This package exposes an importable Go wrapper around the `probly` C ABI.

Import path:

```go
import "github.com/noahfraiture/probly/bindings/go/probly"
```

Prepare the Rust artifacts from the repository checkout:

```sh
cargo build -p probly-core --release
```

Dynamic linking is the default:

```sh
CGO_LDFLAGS="$PWD/target/release/libprobly_core.dylib -Wl,-rpath,$PWD/target/release" \
  go build ./examples/go/local
```

Static linking for the Rust archive is selected by changing `CGO_LDFLAGS`:

```sh
CGO_LDFLAGS="$PWD/target/release/libprobly_core.a" \
  go build ./examples/go/local
```

This works cleanly from a repository checkout or a local `replace` in another Go module.
It is not a pure-Go package: `cgo`, `cargo`, and the Rust source tree are still required.

What this does not support today:

- `go get github.com/noahfraiture/probly/...` as a zero-setup experience from an arbitrary module

Why not:

- the package links against Rust artifacts under `target/release`
- those artifacts are not built by `go get`
- `cgo` consumers still need a Rust toolchain and native linker setup
