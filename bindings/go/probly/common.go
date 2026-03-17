//go:build cgo

package probly

/*
#cgo CFLAGS: -I../../../include
#include "probly.h"
#include <stdlib.h>
*/
import "C"

import (
	"errors"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

var (
	errClosed           = errors.New("probly: sketch is closed")
	errAllocationFailed = errors.New("probly: failed to allocate sketch")
	errMergeFailed      = errors.New("probly: merge failed")
	errAddBytesFailed   = errors.New("probly: add bytes failed")
)

// UltraLogLog is a Go wrapper around the probly C ABI.
// Call Close when finished to release the underlying native allocation.
type UltraLogLog struct {
	mu     sync.Mutex
	sketch *C.probly_ull_t
}

// NewUltraLogLog allocates a new sketch with 2^precision registers.
func NewUltraLogLog(precision uint8) (*UltraLogLog, error) {
	sketch := C.probly_ull_new(C.uint8_t(precision))
	if sketch == nil {
		return nil, errAllocationFailed
	}

	ull := &UltraLogLog{sketch: sketch}
	runtime.SetFinalizer(ull, func(ull *UltraLogLog) {
		_ = ull.Close()
	})
	return ull, nil
}

// AddBytes hashes a byte slice into the sketch.
func (u *UltraLogLog) AddBytes(value []byte) error {
	u.mu.Lock()
	defer u.mu.Unlock()

	if u.sketch == nil {
		return errClosed
	}

	if len(value) == 0 {
		if !bool(C.probly_ull_add_bytes(u.sketch, nil, 0)) {
			return errAddBytesFailed
		}
		return nil
	}

	raw := C.CBytes(value)
	defer C.free(raw)

	if !bool(C.probly_ull_add_bytes(u.sketch, (*C.uint8_t)(raw), C.size_t(len(value)))) {
		return errAddBytesFailed
	}
	return nil
}

// AddString is a convenience wrapper around AddBytes.
func (u *UltraLogLog) AddString(value string) error {
	return u.AddBytes([]byte(value))
}

// Merge folds another sketch with matching precision into this sketch.
func (u *UltraLogLog) Merge(other *UltraLogLog) error {
	if other == nil {
		return fmt.Errorf("%w: other sketch is nil", errMergeFailed)
	}

	if u == other {
		return nil
	}

	first, second := orderForLock(u, other)
	first.mu.Lock()
	second.mu.Lock()
	defer second.mu.Unlock()
	defer first.mu.Unlock()

	if u.sketch == nil || other.sketch == nil {
		return errClosed
	}

	if !bool(C.probly_ull_merge(u.sketch, other.sketch)) {
		return errMergeFailed
	}
	return nil
}

// Count returns the current approximate distinct count.
func (u *UltraLogLog) Count() (uint64, error) {
	u.mu.Lock()
	defer u.mu.Unlock()

	if u.sketch == nil {
		return 0, errClosed
	}

	return uint64(C.probly_ull_count(u.sketch)), nil
}

// Close releases the native sketch allocation.
func (u *UltraLogLog) Close() error {
	u.mu.Lock()
	defer u.mu.Unlock()

	if u.sketch == nil {
		return nil
	}

	C.probly_ull_free(u.sketch)
	u.sketch = nil
	runtime.SetFinalizer(u, nil)
	return nil
}

func orderForLock(left, right *UltraLogLog) (*UltraLogLog, *UltraLogLog) {
	if uintptr(unsafe.Pointer(left)) < uintptr(unsafe.Pointer(right)) {
		return left, right
	}
	return right, left
}
