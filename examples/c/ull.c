#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "probly.h"

static int add_string(probly_ull_t *sketch, const char *value) {
    return probly_ull_add_bytes(
        sketch,
        (const uint8_t *)value,
        strlen(value)
    );
}

int main(void) {
    const char *left_values[] = {"alice", "bob", "carol", "alice"};
    const char *right_values[] = {"dave", "erin", "carol", "frank"};
    const size_t left_len = sizeof(left_values) / sizeof(left_values[0]);
    const size_t right_len = sizeof(right_values) / sizeof(right_values[0]);

    probly_ull_t *left = probly_ull_new(12);
    probly_ull_t *right = probly_ull_new(12);

    if (left == NULL || right == NULL) {
        fprintf(stderr, "failed to allocate sketches\n");
        probly_ull_free(left);
        probly_ull_free(right);
        return 1;
    }

    for (size_t i = 0; i < left_len; i++) {
        if (!add_string(left, left_values[i])) {
            fprintf(stderr, "failed to add %s to left sketch\n", left_values[i]);
            probly_ull_free(left);
            probly_ull_free(right);
            return 1;
        }
    }

    for (size_t i = 0; i < right_len; i++) {
        if (!add_string(right, right_values[i])) {
            fprintf(stderr, "failed to add %s to right sketch\n", right_values[i]);
            probly_ull_free(left);
            probly_ull_free(right);
            return 1;
        }
    }

    printf("left estimate:  %zu\n", probly_ull_count(left));
    printf("right estimate: %zu\n", probly_ull_count(right));

    if (!probly_ull_merge(left, right)) {
        fprintf(stderr, "merge failed\n");
        probly_ull_free(left);
        probly_ull_free(right);
        return 1;
    }

    printf("union estimate: %zu\n", probly_ull_count(left));

    probly_ull_free(left);
    probly_ull_free(right);
    return 0;
}
