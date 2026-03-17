RELEASE_DIR := target/release
EXAMPLE_DIR := target/examples
RPATH := $(abspath $(RELEASE_DIR))
STATIC_LIB := $(RELEASE_DIR)/libprobly_core.a
GO_DYNAMIC_LDFLAGS := $(abspath $(RELEASE_DIR))/libprobly_core.dylib -Wl,-rpath,$(abspath $(RELEASE_DIR))
GO_STATIC_LDFLAGS := $(abspath $(STATIC_LIB))
UNAME_S := $(shell uname -s)

.PHONY: ffi ffi-shared ffi-static example-c example-c-dynamic example-c-static example-go example-go-dynamic example-go-static example-go-importer example-go-importer-static examples

ffi:
	cargo build -p probly-core --release

ffi-shared: ffi
ifeq ($(UNAME_S),Darwin)
	install_name_tool -id @rpath/libprobly_core.dylib $(RELEASE_DIR)/libprobly_core.dylib
endif

ffi-static: ffi

example-c: example-c-dynamic

example-c-dynamic: ffi-shared
	mkdir -p $(EXAMPLE_DIR)
	cc examples/c/ull.c -Iinclude -L$(RELEASE_DIR) -lprobly_core -Wl,-rpath,$(RPATH) -o $(EXAMPLE_DIR)/ull_c

example-c-static: ffi-static
	mkdir -p $(EXAMPLE_DIR)
	cc examples/c/ull.c -Iinclude $(STATIC_LIB) -o $(EXAMPLE_DIR)/ull_c_static

example-go: example-go-dynamic

example-go-dynamic: ffi-shared
	mkdir -p $(EXAMPLE_DIR)
	CGO_LDFLAGS='$(GO_DYNAMIC_LDFLAGS)' go build -o $(EXAMPLE_DIR)/ull_go ./examples/go/local

example-go-static: ffi-static
	mkdir -p $(EXAMPLE_DIR)
	CGO_LDFLAGS='$(GO_STATIC_LDFLAGS)' go build -o $(EXAMPLE_DIR)/ull_go_static ./examples/go/local

example-go-importer: ffi-shared
	mkdir -p $(EXAMPLE_DIR)
	CGO_LDFLAGS='$(GO_DYNAMIC_LDFLAGS)' go -C examples/go/importer build -o ../../../$(EXAMPLE_DIR)/ull_go_importer .

example-go-importer-static: ffi-static
	mkdir -p $(EXAMPLE_DIR)
	CGO_LDFLAGS='$(GO_STATIC_LDFLAGS)' go -C examples/go/importer build -o ../../../$(EXAMPLE_DIR)/ull_go_importer_static .

examples: example-c-dynamic example-c-static example-go-dynamic example-go-static example-go-importer example-go-importer-static
