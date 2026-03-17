#ifndef PROBLY_H
#define PROBLY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct probly_ull probly_ull_t;

/// Allocates a new UltraLogLog sketch with 2^precision registers.
probly_ull_t *probly_ull_new(uint8_t precision);

/// Adds a raw byte slice to the sketch. Returns false on invalid pointers.
bool probly_ull_add_bytes(probly_ull_t *sketch, const uint8_t *value, size_t len);

/// Merges other into sketch. Returns false on invalid pointers or precision mismatch.
bool probly_ull_merge(probly_ull_t *sketch, const probly_ull_t *other);

/// Returns the approximate distinct count for the sketch.
size_t probly_ull_count(const probly_ull_t *sketch);

/// Frees a sketch returned by probly_ull_new.
void probly_ull_free(probly_ull_t *sketch);

#ifdef __cplusplus
}
#endif

#endif
