#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

void fuzz_foobar(const uint8_t *data, size_t len)
{
    if (len < 1) return;

    /* abort only on "foobar" */
    if (data[0] != 'f') return;
    if (len < 2) return;
    if (data[1] != 'o') return;
    if (len < 3) return;
    if (data[2] != 'o') return;
    if (len < 4) return;
    if (data[3] != 'b') return;
    if (len < 5) return;
    if (data[4] != 'a') return;
    if (len < 6) return;
    if (data[5] != 'r') return;
    
    abort();
}

void fuzz_foobar_from_path(const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) return;
    uint8_t buf[256];
    size_t  len = fread(buf, 1, sizeof(buf), f);
    fclose(f);
    fuzz_foobar(buf, len);
}
