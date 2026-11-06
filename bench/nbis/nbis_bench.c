/* Time the NBIS WSQ codec in memory.
 *
 *   nbis_bench decode <reps> <file.wsq>...
 *   nbis_bench encode <reps> <rate> <file.pgm>...
 *
 * Prints one line per file: name, width, height, best and median time in milliseconds.
 * Build: see Makefile (needs the NBIS 5.0.0 headers and static libraries).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wsq.h>

int debug = 0; /* NBIS expects the application to define it */

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static unsigned char *read_file(const char *path, int *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    *len = (int)ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *d = malloc(*len);
    if (fread(d, 1, *len, f) != (size_t)*len) { fclose(f); free(d); return NULL; }
    fclose(f);
    return d;
}

/* binary PGM: header tokens, then one whitespace byte */
static unsigned char *read_pgm(const char *path, int *w, int *h) {
    int len;
    unsigned char *d = read_file(path, &len);
    if (!d || len < 2 || d[0] != 'P' || d[1] != '5') return NULL;
    int pos = 2, tok[3], nt = 0;
    while (nt < 3) {
        while (pos < len && (d[pos] == ' ' || d[pos] == '\n' || d[pos] == '\r' || d[pos] == '\t')) pos++;
        if (d[pos] == '#') { while (pos < len && d[pos] != '\n') pos++; continue; }
        int v = 0;
        while (pos < len && d[pos] >= '0' && d[pos] <= '9') v = v * 10 + (d[pos++] - '0');
        tok[nt++] = v;
    }
    pos++;
    *w = tok[0]; *h = tok[1];
    unsigned char *p = malloc((size_t)*w * *h);
    memcpy(p, d + pos, (size_t)*w * *h);
    free(d);
    return p;
}

static int cmp(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}

int main(int argc, char **argv) {
    if (argc < 4) { fprintf(stderr, "usage: see source\n"); return 2; }
    int reps = atoi(argv[2]);
    int enc = strcmp(argv[1], "encode") == 0;
    float rate = enc ? (float)atof(argv[3]) : 0;
    double *t = malloc(sizeof(double) * reps);
    for (int i = enc ? 4 : 3; i < argc; i++) {
        int w = 0, h = 0, len = 0;
        unsigned char *in = enc ? read_pgm(argv[i], &w, &h) : read_file(argv[i], &len);
        if (!in) { fprintf(stderr, "cannot read %s\n", argv[i]); return 1; }
        for (int r = -1; r < reps; r++) {
            unsigned char *out; int ow, oh, od, oppi, lossy, olen, ret;
            double t0 = now_ms();
            if (enc) ret = wsq_encode_mem(&out, &olen, rate, in, w, h, 8, 500, NULL);
            else ret = wsq_decode_mem(&out, &ow, &oh, &od, &oppi, &lossy, in, len);
            double dt = now_ms() - t0;
            if (ret) { fprintf(stderr, "%s: error %d\n", argv[i], ret); return 1; }
            if (!enc) { w = ow; h = oh; }
            free(out);
            if (r >= 0) t[r] = dt;
        }
        qsort(t, reps, sizeof(double), cmp);
        const char *name = strrchr(argv[i], '/');
        printf("%s %d %d %.4f %.4f\n", name ? name + 1 : argv[i], w, h, t[0], t[reps / 2]);
        free(in);
    }
    free(t);
    return 0;
}
