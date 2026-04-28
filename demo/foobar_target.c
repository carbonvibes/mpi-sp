/*
 * foobar_target.c — minimal crash target for LibAFL harness validation.
 *
 * Compiled with -fsanitize-coverage=trace-pc-guard and linked directly into
 * the Rust fuzzer binary.  The Rust harness calls fuzz_foobar() in-process
 * with the primary content bytes extracted from each FsDelta.  SanCov
 * instrumentation on this file feeds the edge-coverage map that MapFeedback
 * uses to drive corpus evolution.
 *
 * Crash condition: content starts with the 6-byte string "foobar".
 * This is the canonical LibAFL end-to-end proof-of-life: a cold corpus of
 * semantic deltas (UpdateFile("/input", random bytes)) must evolve to produce
 * "foobar" content and trigger abort() within a bounded iteration budget.
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

void fuzz_foobar(const uint8_t *data, size_t len)
{
    if (len < 1) return;

    /* Gate 1: first byte is 'f' */
    if (data[0] != 'f') return;
    if (len < 2) return;

    /* Gate 2: second byte is 'o' */
    if (data[1] != 'o') return;
    if (len < 3) return;

    /* Gate 3: third byte is 'o' */
    if (data[2] != 'o') return;
    if (len < 4) return;

    /* Gate 4: fourth byte is 'b' */
    if (data[3] != 'b') return;
    if (len < 5) return;

    /* Gate 5: fifth byte is 'a' */
    if (data[4] != 'a') return;
    if (len < 6) return;

    /* Gate 6: sixth byte is 'r' */
    if (data[5] != 'r') return;
    if (len < 7) return;

    /* Gate 7: seventh byte is 'w' */
    if (data[6] != 'w') return;
    if (len < 8) return;

    /* Gate 8: eighth byte is 'o' */
    if (data[7] != 'o') return;
    if (len < 9) return;

    /* Gate 9: ninth byte is 'r' */
    if (data[8] != 'r') return;
    if (len < 10) return;

    /* Gate 10: tenth byte is 'l' */
    if (data[9] != 'l') return;
    if (len < 11) return;

    /* Gate 11: eleventh byte is 'd' */
    if (data[10] != 'd') return;
    if (len < 12) return;

    /* Gate 12: twelfth byte is '!' */
    if (data[11] != '!') return;

    abort();
}

/* Read the file at path and apply fuzz_foobar() to its contents. */
void fuzz_foobar_from_path(const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) return;
    uint8_t buf[256];
    size_t  len = fread(buf, 1, sizeof(buf), f);
    fclose(f);
    fuzz_foobar(buf, len);
}
