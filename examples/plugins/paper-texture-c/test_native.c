// Native preview harness: renders the plug-in's code to a PPM. Not part of the plug-in.
#define NATIVE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "paper_texture.c"
int main(int argc, char **argv) {
    int w = atoi(argv[1]), h = atoi(argv[2]);
    const char *json = argv[3];
    char buf[2048];
    snprintf(buf, sizeof buf, "{%s,\"_image\":{\"x\":0,\"y\":0,\"width\":%d,\"height\":%d,\"canvasWidth\":%d,\"canvasHeight\":%d}}", json, w, h, w, h);
    float *px = malloc(sizeof(float) * w * h * 4);
    for (int i = 0; i < w * h * 4; i++) px[i] = 0.5f;
    int r = pc_filter(px, w * h * 16, w, h, 4, 8 | (1 << 8) | (3 << 16), buf, strlen(buf));
    if (r) { fprintf(stderr, "rc=%d\n", r); return 1; }
    FILE *f = fopen(argv[4], "wb");
    fprintf(f, "P6\n%d %d\n255\n", w, h);
    for (int i = 0; i < w * h; i++) {
        float a = px[i*4+3];
        for (int k = 0; k < 3; k++) {
            float bg = (((i % w) / 16 + (i / w) / 16) & 1) ? 0.35f : 0.55f; // checkerboard for transparency
            float v = px[i*4+k] * a + bg * (1 - a);
            fputc((int)(v * 255 + 0.5f), f);
        }
    }
    fclose(f);
    return 0;
}
