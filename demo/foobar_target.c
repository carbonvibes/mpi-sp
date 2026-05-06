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
