# Examples

This directory contains consumer programs, not reusable bindings.

Current approaches:

- `c/` — direct C ABI consumers
- `go/local/` — Go consumer built inside this repository
- `go/importer/` — separate Go module importing the package via `replace`

Build the Rust artifacts first:

```sh
make ffi
```

That release build now emits both:

- `target/release/libprobly_core.dylib` for dynamic linking
- `target/release/libprobly_core.a` for static linking

Build the C example with dynamic linking:

```sh
make example-c-dynamic
./target/examples/ull_c
```

Source: `examples/c/ull.c`

Build the C example with static linking:

```sh
make example-c-static
./target/examples/ull_c_static
```

Build the Go example with dynamic linking:

```sh
make example-go-dynamic
./target/examples/ull_go
```

Source consumer: `examples/go/local/main.go`
Imported package: `github.com/noahfraiture/probly/bindings/go/probly`

Build the Go example with static linking:

```sh
make example-go-static
./target/examples/ull_go_static
```

Source consumer: `examples/go/local/main.go`
Imported package: `github.com/noahfraiture/probly/bindings/go/probly`

Build a separate Go module that imports the package via `replace`:

```sh
make example-go-importer
./target/examples/ull_go_importer
```

Static variant:

```sh
make example-go-importer-static
./target/examples/ull_go_importer_static
```

Importer source: `examples/go/importer/main.go`

Important:

- these Go flows work from a repository checkout
- they do not currently provide a zero-setup `go get` experience from an unrelated project

The public header is [`include/probly.h`](../include/probly.h). The examples link against the
artifacts produced by `cargo build -p probly-core --release`.

Dynamic executables depend on `libprobly_core.dylib` at runtime. Static executables embed the
Rust ABI archive, but they still depend on the platform system libraries used by the final link.
