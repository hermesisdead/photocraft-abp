/* Paper Texture: a seeded, procedural paper generator for PhotoCraft (plug-in ABI v1, see
 * docs/plugins.md). Freestanding C compiled to WebAssembly: no imports, no allocator, no state.
 *
 * Every effect is a pure function of (seed, document position, canvas size), so the result is
 * identical for a given seed and seamless across the host's bands. Features scale with the canvas
 * (they are measured in units of its longer side), so a texture looks the same at any resolution.
 *
 * Build: see build.sh.  Native test harness: compile with -DNATIVE (see test_native.c).
 */
#include <stdint.h>

typedef float f32;
typedef uint32_t u32;
typedef int32_t i32;

/* ------------------------------------------------------------------ manifest */

#define MANIFEST_TEXT \
"{" \
"\"id\":\"org.photocraft.paper-texture\"," \
"\"name\":\"Paper Texture\\u2026\"," \
"\"version\":\"1.0.0\"," \
"\"kind\":\"filter\"," \
"\"author\":\"PhotoCraft contributors\"," \
"\"description\":\"Generates a seeded paper texture: grain, wrinkles, creases, folds, stains, aging, burns and rips, each with its own intensity.\"," \
"\"area\":\"canvas\"," \
"\"overlap\":0," \
"\"params\":{" \
"\"seed\":{\"type\":\"int\",\"min\":0,\"max\":999999,\"default\":1}," \
"\"paper\":{\"type\":\"choice\",\"options\":[\"white\",\"cream\",\"aged\",\"parchment\",\"newsprint\",\"kraft\"],\"default\":\"cream\"}," \
"\"apply\":{\"type\":\"choice\",\"options\":[\"replace layer\",\"multiply over layer\"],\"default\":\"replace layer\"}," \
"\"texture\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":50}," \
"\"texture_size\":{\"type\":\"number\",\"min\":10,\"max\":100,\"default\":50}," \
"\"wrinkles\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":40}," \
"\"wrinkle_size\":{\"type\":\"number\",\"min\":10,\"max\":100,\"default\":50}," \
"\"creases\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":30}," \
"\"crease_count\":{\"type\":\"int\",\"min\":1,\"max\":24,\"default\":6}," \
"\"folds\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":30}," \
"\"fold_count\":{\"type\":\"int\",\"min\":1,\"max\":6,\"default\":2}," \
"\"fold_layout\":{\"type\":\"choice\",\"options\":[\"horizontal\",\"vertical\",\"both\",\"diagonal\"],\"default\":\"horizontal\"}," \
"\"stains\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":20}," \
"\"stain_count\":{\"type\":\"int\",\"min\":1,\"max\":8,\"default\":3}," \
"\"aging\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":30}," \
"\"burns\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0}," \
"\"burn_spots\":{\"type\":\"int\",\"min\":0,\"max\":8,\"default\":2}," \
"\"burn_through\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":30}," \
"\"rips\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0}," \
"\"rip_edges\":{\"type\":\"choice\",\"options\":[\"all edges\",\"top\",\"bottom\",\"left\",\"right\",\"corner\"],\"default\":\"all edges\"}" \
"}}"

static const char MANIFEST[] = MANIFEST_TEXT;

/* ------------------------------------------------------------------ small maths */

#define PI 3.14159265f

static inline f32 fl(f32 x) { return __builtin_floorf(x); }
static inline f32 ab(f32 x) { return __builtin_fabsf(x); }
static inline f32 sq(f32 x) { return __builtin_sqrtf(x); }
static inline f32 cl(f32 x, f32 a, f32 b) { return x < a ? a : (x > b ? b : x); }
static inline f32 mx(f32 a, f32 b, f32 t) { return a + (b - a) * t; }
static inline f32 sst(f32 e0, f32 e1, f32 x) {
    f32 t = cl((x - e0) / (e1 - e0), 0.0f, 1.0f);
    return t * t * (3.0f - 2.0f * t);
}

/* Taylor sine for set-up work only (never per pixel). */
static f32 sn(f32 a) {
    a = a - 2.0f * PI * fl((a + PI) / (2.0f * PI));
    f32 a2 = a * a;
    return a * (1.0f - a2 / 6.0f * (1.0f - a2 / 20.0f * (1.0f - a2 / 42.0f * (1.0f - a2 / 72.0f * (1.0f - a2 / 110.0f)))));
}
static f32 cs(f32 a) { return sn(a + PI * 0.5f); }

static u32 hs(i32 x, i32 y, u32 s) {
    u32 h = (u32)x * 0x27d4eb2du ^ (u32)y * 0x165667b1u ^ s * 0x9e3779b9u;
    h ^= h >> 15; h *= 0x85ebca6bu; h ^= h >> 13; h *= 0xc2b2ae35u; h ^= h >> 16;
    return h;
}
static f32 r01(u32 h) { return (f32)(h >> 8) * (1.0f / 16777216.0f); }

/* Smooth value noise in 0..1. */
static f32 vn(f32 x, f32 y, u32 s) {
    f32 fx = fl(x), fy = fl(y);
    i32 ix = (i32)fx, iy = (i32)fy;
    f32 tx = x - fx, ty = y - fy;
    tx = tx * tx * (3.0f - 2.0f * tx);
    ty = ty * ty * (3.0f - 2.0f * ty);
    f32 a = r01(hs(ix, iy, s)), b = r01(hs(ix + 1, iy, s));
    f32 c = r01(hs(ix, iy + 1, s)), d = r01(hs(ix + 1, iy + 1, s));
    return mx(mx(a, b, tx), mx(c, d, tx), ty);
}
static f32 fbm(f32 x, f32 y, u32 s, int o) {
    f32 a = 0.5f, t = 0.0f, n = 0.0f;
    for (int i = 0; i < o; i++) {
        t += a * vn(x, y, s + (u32)i * 101u);
        n += a;
        x = x * 2.03f + 17.1f;
        y = y * 2.03f + 9.7f;
        a *= 0.5f;
    }
    return t / n;
}

/* ------------------------------------------------------------------ parameters */

typedef struct {
    f32 seed, paper, apply, texture, texture_size, wrinkles, wrinkle_size, creases, crease_count;
    f32 folds, fold_count, fold_layout, stains, stain_count, aging, burns, burn_spots, burn_through;
    f32 rips, rip_edges;
    f32 ix, iy, iw, ih, cw, chh; /* _image geometry */
} Params;

static Params P;

static const char *const PAPERS[] = {"white", "cream", "aged", "parchment", "newsprint", "kraft"};
static const char *const APPLY[] = {"replace layer", "multiply over layer"};
static const char *const LAYOUTS[] = {"horizontal", "vertical", "both", "diagonal"};
static const char *const EDGES[] = {"all edges", "top", "bottom", "left", "right", "corner"};

typedef struct { const char *name; f32 *dst; } NumKey;
typedef struct { const char *name; f32 *dst; const char *const *opts; int n; } ChoiceKey;

static int streq(const char *a, int al, const char *b) {
    int i = 0;
    for (; i < al; i++) if (b[i] == 0 || a[i] != b[i]) return 0;
    return b[i] == 0;
}

static const NumKey NUMS[] = {
    {"seed", &P.seed}, {"texture", &P.texture}, {"texture_size", &P.texture_size},
    {"wrinkles", &P.wrinkles}, {"wrinkle_size", &P.wrinkle_size}, {"creases", &P.creases},
    {"crease_count", &P.crease_count}, {"folds", &P.folds}, {"fold_count", &P.fold_count},
    {"stains", &P.stains}, {"stain_count", &P.stain_count}, {"aging", &P.aging},
    {"burns", &P.burns}, {"burn_spots", &P.burn_spots}, {"burn_through", &P.burn_through},
    {"rips", &P.rips},
};
static const ChoiceKey CHOICES[] = {
    {"paper", &P.paper, PAPERS, 6}, {"apply", &P.apply, APPLY, 2},
    {"fold_layout", &P.fold_layout, LAYOUTS, 4}, {"rip_edges", &P.rip_edges, EDGES, 6},
};
static const NumKey IMG[] = {
    {"x", &P.ix}, {"y", &P.iy}, {"width", &P.iw}, {"height", &P.ih},
    {"canvasWidth", &P.cw}, {"canvasHeight", &P.chh},
};

static void set_num(const NumKey *t, int n, const char *k, int kl, f32 v) {
    for (int i = 0; i < n; i++) if (streq(k, kl, t[i].name)) { *t[i].dst = v; return; }
}
static void set_choice(const char *k, int kl, const char *v, int vl) {
    for (int i = 0; i < 4; i++) {
        if (!streq(k, kl, CHOICES[i].name)) continue;
        for (int j = 0; j < CHOICES[i].n; j++) if (streq(v, vl, CHOICES[i].opts[j])) { *CHOICES[i].dst = (f32)j; return; }
    }
}

static f32 read_num(const char *b, int n, int *pi) {
    int i = *pi;
    f32 sign = 1.0f, v = 0.0f;
    if (i < n && b[i] == '-') { sign = -1.0f; i++; }
    while (i < n && b[i] >= '0' && b[i] <= '9') { v = v * 10.0f + (f32)(b[i] - '0'); i++; }
    if (i < n && b[i] == '.') {
        i++;
        f32 s = 0.1f;
        while (i < n && b[i] >= '0' && b[i] <= '9') { v += s * (f32)(b[i] - '0'); s *= 0.1f; i++; }
    }
    if (i < n && (b[i] == 'e' || b[i] == 'E')) {
        i++;
        int es = 1, e = 0;
        if (i < n && (b[i] == '-' || b[i] == '+')) { es = b[i] == '-' ? -1 : 1; i++; }
        while (i < n && b[i] >= '0' && b[i] <= '9') { e = e * 10 + (b[i] - '0'); if (e > 60) e = 60; i++; }
        for (int k = 0; k < e; k++) v = es > 0 ? v * 10.0f : v * 0.1f;
    }
    *pi = i;
    return sign * v;
}

static void parse(const char *b, int n) {
    int i = 0, depth = 0, img = 0;
    while (i < n) {
        char c = b[i];
        if (c == '{') { depth++; i++; continue; }
        if (c == '}') { if (depth == 2) img = 0; depth--; i++; continue; }
        if (c != '"') { i++; continue; }
        int ks = ++i;
        while (i < n && b[i] != '"') i++;
        int kl = i - ks;
        i++;
        while (i < n && (b[i] == ' ' || b[i] == ':' || b[i] == '\n' || b[i] == '\t' || b[i] == '\r')) i++;
        if (i >= n) break;
        if (b[i] == '{') { if (depth == 1 && streq(b + ks, kl, "_image")) img = 1; continue; }
        if (b[i] == '"') {
            int vs = ++i;
            while (i < n && b[i] != '"') i++;
            set_choice(b + ks, kl, b + vs, i - vs);
            i++;
            continue;
        }
        if (b[i] == 't' || b[i] == 'f') {
            f32 v = b[i] == 't' ? 1.0f : 0.0f;
            while (i < n && b[i] >= 'a' && b[i] <= 'z') i++;
            (void)v;
            continue;
        }
        f32 v = read_num(b, n, &i);
        if (img) set_num(IMG, 6, b + ks, kl, v); else set_num(NUMS, 16, b + ks, kl, v);
    }
}

/* ------------------------------------------------------------------ scene set-up */

#define MAX_CREASES 24
#define MAX_STAINS 8
#define MAX_BURNS 8

typedef struct { f32 cx, cy, dx, dy, half, str; } Line;
typedef struct { f32 cx, cy, r, str; } Blob;

static struct {
    f32 W, H, S, Wu, Hu; /* canvas size in pixels, longer side, size in S units */
    u32 seed;
    f32 base[3];
    f32 T, Wr, C, F, St, A, B, R, bt;
    int ncrease, nfold, nstain, nburn;
    Line crease[MAX_CREASES];
    f32 fold_pos[16];
    f32 fold_axis[16]; /* 0 horizontal, 1 vertical, 2 diagonal */
    Blob stain[MAX_STAINS], burn[MAX_BURNS];
    f32 gf, wf;
    int rip_mask; /* bit0 top, 1 bottom, 2 left, 3 right, 4 corner */
} G;

static const f32 TONES[6][3] = {
    {0.965f, 0.962f, 0.950f}, /* white */
    {0.940f, 0.910f, 0.830f}, /* cream */
    {0.850f, 0.770f, 0.600f}, /* aged */
    {0.900f, 0.810f, 0.640f}, /* parchment */
    {0.800f, 0.790f, 0.735f}, /* newsprint */
    {0.720f, 0.550f, 0.360f}, /* kraft */
};

static f32 jag_calc(f32 t, f32 depth, u32 s) {
    return depth * (0.25f + 1.5f * fbm(t * 5.0f, (f32)(s & 255), s, 4) - 0.35f) + depth * 0.10f * vn(t * 110.0f, 3.0f, s + 7u);
}

/* The jagged profile of each torn side, sampled once per call over t in 0..2 (S units). */
#define LUT_N 1025
static f32 JAG[5][LUT_N];
static f32 jag(int side, f32 t) {
    f32 x = cl(t, 0.0f, 1.999f) * 512.0f;
    int i = (int)x;
    f32 f = x - (f32)i;
    return JAG[side][i] + (JAG[side][i + 1] - JAG[side][i]) * f;
}

static int ci(f32 v, int lo, int hi) {
    int n = (int)(v + 0.5f);
    return n < lo ? lo : (n > hi ? hi : n);
}

static void setup(f32 cw, f32 ch) {
    G.W = cw < 1.0f ? 1.0f : cw;
    G.H = ch < 1.0f ? 1.0f : ch;
    G.S = G.W > G.H ? G.W : G.H;
    G.Wu = G.W / G.S;
    G.Hu = G.H / G.S;
    G.seed = (u32)ci(P.seed, 0, 999999) * 2654435761u + 12345u;
    int paper = ci(P.paper, 0, 5);
    for (int k = 0; k < 3; k++) G.base[k] = TONES[paper][k];
    G.T = cl(P.texture, 0, 100) * 0.01f;
    G.Wr = cl(P.wrinkles, 0, 100) * 0.01f;
    G.C = cl(P.creases, 0, 100) * 0.01f;
    G.F = cl(P.folds, 0, 100) * 0.01f;
    G.St = cl(P.stains, 0, 100) * 0.01f;
    G.A = cl(P.aging, 0, 100) * 0.01f;
    G.B = cl(P.burns, 0, 100) * 0.01f;
    G.R = cl(P.rips, 0, 100) * 0.01f;
    G.bt = cl(P.burn_through, 0, 100) * 0.01f;

    f32 ts = cl(P.texture_size, 10, 100);
    G.gf = 700.0f - ts * 5.5f;
    if (G.gf > G.S / 1.5f) G.gf = G.S / 1.5f;
    f32 ws = cl(P.wrinkle_size, 10, 100);
    G.wf = 2.0f + (100.0f - ws) * 0.12f;

    G.ncrease = G.C > 0.0f ? ci(P.crease_count, 1, MAX_CREASES) : 0;
    for (int i = 0; i < G.ncrease; i++) {
        Line *l = &G.crease[i];
        l->cx = r01(hs(i, 1, G.seed)) * G.Wu;
        l->cy = r01(hs(i, 2, G.seed)) * G.Hu;
        f32 a = r01(hs(i, 3, G.seed)) * PI;
        l->dx = cs(a);
        l->dy = sn(a);
        l->half = 0.18f + 0.55f * r01(hs(i, 4, G.seed));
        l->str = 0.45f + 0.55f * r01(hs(i, 5, G.seed));
    }

    int n = ci(P.fold_count, 1, 6);
    int layout = ci(P.fold_layout, 0, 3);
    G.nfold = 0;
    if (G.F > 0.0f) {
        for (int pass = 0; pass < 2; pass++) {
            f32 axis = layout == 3 ? 2.0f : (layout == 2 ? (f32)pass : (f32)layout);
            if (layout != 2 && pass == 1) break;
            for (int i = 0; i < n && G.nfold < 16; i++) {
                f32 f = (f32)(i + 1) / (f32)(n + 1);
                f32 len = axis == 0.0f ? G.Hu : (axis == 1.0f ? G.Wu : G.Wu + G.Hu);
                G.fold_pos[G.nfold] = len * f;
                G.fold_axis[G.nfold] = axis;
                G.nfold++;
            }
        }
    }

    G.nstain = G.St > 0.0f ? ci(P.stain_count, 1, MAX_STAINS) : 0;
    for (int i = 0; i < G.nstain; i++) {
        Blob *b = &G.stain[i];
        b->cx = (0.1f + 0.8f * r01(hs(i, 11, G.seed))) * G.Wu;
        b->cy = (0.1f + 0.8f * r01(hs(i, 12, G.seed))) * G.Hu;
        b->r = 0.03f + 0.09f * r01(hs(i, 13, G.seed));
        b->str = 0.5f + 0.5f * r01(hs(i, 14, G.seed));
    }

    G.nburn = G.B > 0.0f ? ci(P.burn_spots, 0, MAX_BURNS) : 0;
    for (int i = 0; i < G.nburn; i++) {
        Blob *b = &G.burn[i];
        b->cx = (0.1f + 0.8f * r01(hs(i, 21, G.seed))) * G.Wu;
        b->cy = (0.1f + 0.8f * r01(hs(i, 22, G.seed))) * G.Hu;
        b->r = (0.015f + 0.05f * r01(hs(i, 23, G.seed))) * (0.4f + 1.4f * G.B);
        b->str = 1.0f;
    }

    int e = ci(P.rip_edges, 0, 5);
    G.rip_mask = G.R > 0.0f ? (e == 0 ? 15 : (e == 5 ? 16 : 1 << (e - 1))) : 0;
    for (int side = 0; side < 5; side++) {
        if (!(G.rip_mask & (1 << side))) continue;
        for (int i = 0; i < LUT_N; i++) JAG[side][i] = jag_calc((f32)i / 512.0f, G.R * 0.14f, G.seed + 31u + (u32)side);
    }
}

/* ------------------------------------------------------------------ the paper */

typedef struct { f32 r, g, b, a; } Px;

/* Scorch colours for a burn coverage 0..1 (1 = fully charred). */
static void burn_into(Px *p, f32 cov) {
    if (cov <= 0.0f) return;
    f32 tan = sst(0.0f, 0.55f, cov);
    p->r *= mx(1.0f, 0.80f, tan); p->g *= mx(1.0f, 0.52f, tan); p->b *= mx(1.0f, 0.26f, tan);
    f32 chr = sst(0.45f, 0.80f, cov);
    p->r = mx(p->r, 0.060f, chr); p->g = mx(p->g, 0.035f, chr); p->b = mx(p->b, 0.020f, chr);
}

__attribute__((noinline)) static void fx_tex(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    Px o = *op; f32 d = *dp; f32 S = G.S;
    (void)px; (void)py; (void)u; (void)v; (void)sd; (void)S;
    if (G.T > 0.0f) { /* grain, fibres and tooth */
        f32 g = (vn(u * G.gf, v * G.gf, sd + 1u) - 0.5f) * 2.0f;
        f32 f1 = vn(u * G.gf * 0.40f, v * G.gf * 0.10f + u * 9.0f, sd + 2u);
        f32 f2 = vn(u * G.gf * 0.10f + v * 9.0f, v * G.gf * 0.40f, sd + 3u);
        f32 m = (vn(u * 24.0f, v * 24.0f, sd + 4u) - 0.5f) * 2.0f;
        d += G.T * (0.055f * g + 0.050f * (f1 + f2 - 1.0f) + 0.035f * m);
    }

    *op = o; *dp = d;
}

__attribute__((noinline)) static void fx_wrinkle(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    Px o = *op; f32 d = *dp; f32 S = G.S;
    (void)px; (void)py; (void)u; (void)v; (void)sd; (void)S;
    if (G.Wr > 0.0f) { /* wrinkles: lit ridged noise */
        f32 f = G.wf;
        f32 wx = vn(u * f * 0.6f, v * f * 0.6f, sd + 5u) - 0.5f;
        f32 wy = vn(u * f * 0.6f + 31.0f, v * f * 0.6f + 17.0f, sd + 6u) - 0.5f;
        f32 uu = (u + wx * 0.14f) * f, vv = (v + wy * 0.14f) * f;
        f32 h0 = 1.0f - ab(2.0f * fbm(uu, vv, sd + 7u, 2) - 1.0f);
        f32 h1 = 1.0f - ab(2.0f * fbm(uu + 0.12f, vv + 0.12f, sd + 7u, 2) - 1.0f);
        f32 lit = (h1 - h0) * 4.0f;
        d += G.Wr * (0.22f * cl(lit, -1.0f, 1.0f) - 0.07f * (1.0f - h0) * (1.0f - h0));
    }

    *op = o; *dp = d;
}

/* One crease line's contribution; only called for pixels within its strip. */
__attribute__((noinline)) static f32 crease_hit(const Line *l, int i, f32 rx, f32 ry, f32 across, u32 sd) {
    f32 along = rx * l->dx + ry * l->dy;
    f32 ends = 1.0f - sst(l->half * 0.6f, l->half, ab(along));
    if (ends <= 0.0f) return 0.0f;
    across += (vn(along * 9.0f, (f32)i * 7.1f, sd + 9u) - 0.5f) * 0.006f;
    f32 w = 0.0010f > 0.9f / G.S ? 0.0010f : 0.9f / G.S;
    f32 dark = w * w / (across * across + w * w);
    f32 q = across / (3.5f * w);
    f32 side = q / (1.0f + q * q);
    f32 wear = 0.45f + 0.9f * vn(along * 28.0f, (f32)i * 3.3f, sd + 10u);
    return G.C * l->str * ends * (-0.50f * dark * wear + 0.30f * side);
}

__attribute__((noinline)) static void fx_crease(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    (void)op; (void)px; (void)py;
    f32 d = *dp;
    for (int i = 0; i < G.ncrease; i++) { /* creases: thin dark line with a lit side */
        const Line *l = &G.crease[i];
        f32 rx = u - l->cx, ry = v - l->cy;
        f32 across = -rx * l->dy + ry * l->dx;
        if (ab(across) < 0.03f) d += crease_hit(l, i, rx, ry, across, sd);
    }
    *dp = d;
}

/* One fold's contribution; only called for pixels within its band. */
__attribute__((noinline)) static f32 fold_hit(int i, f32 s, f32 t, u32 sd) {
    f32 a = ab(s);
    f32 w = 0.0016f > 1.2f / G.S ? 0.0016f : 1.2f / G.S;
    f32 line = w * w / (s * s + w * w);
    f32 wear = 0.55f + 0.8f * vn(t * 14.0f, (f32)i, sd + 12u);
    f32 dir = (i & 1) ? -1.0f : 1.0f;
    f32 slope = (s / (a + 0.05f)) * (1.0f - sst(0.03f, 0.14f, a));
    return G.F * (-0.55f * line * wear + 0.11f * dir * slope - 0.05f * (1.0f - sst(0.0f, 0.14f, a)));
}

__attribute__((noinline)) static void fx_fold(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    (void)op; (void)px; (void)py;
    f32 d = *dp;
    for (int i = 0; i < G.nfold; i++) { /* folds: a worn line plus a lit slope each side */
        f32 axis = G.fold_axis[i], s, t;
        if (axis == 0.0f) { s = v - G.fold_pos[i]; t = u; }
        else if (axis == 1.0f) { s = u - G.fold_pos[i]; t = v; }
        else { s = (u + v - G.fold_pos[i]) * 0.70711f; t = (u - v) * 0.70711f; }
        if (ab(s) < 0.14f) d += fold_hit(i, s, t, sd);
    }
    *dp = d;
}

/* One stain's contribution (multiplies the colour); only called for pixels near it. */
__attribute__((noinline)) static void stain_hit(Px *o, const Blob *b, int i, f32 dx, f32 dy, f32 u, f32 v, u32 sd) {
    f32 r = b->r;
    f32 dist = sq(dx * dx + dy * dy) + (vn(u * 26.0f, v * 26.0f, sd + 13u + (u32)i) - 0.5f) * r * 0.5f;
    f32 e = (dist - r) / (r * 0.07f);
    f32 ring = 1.0f / (1.0f + e * e);
    f32 fill = 0.28f * (1.0f - sst(0.0f, r, dist));
    f32 k = G.St * b->str * (0.85f * ring + fill);
    o->r *= 1.0f - k * 0.30f; o->g *= 1.0f - k * 0.50f; o->b *= 1.0f - k * 0.80f;
}

__attribute__((noinline)) static void fx_stain(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    (void)dp; (void)px; (void)py;
    for (int i = 0; i < G.nstain; i++) { /* coffee rings */
        const Blob *b = &G.stain[i];
        f32 dx = u - b->cx, dy = v - b->cy;
        if (dx * dx + dy * dy < 2.6f * b->r * b->r) stain_hit(op, b, i, dx, dy, u, v, sd);
    }
}

__attribute__((noinline)) static void fx_aging(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    Px o = *op; f32 d = *dp; f32 S = G.S;
    (void)px; (void)py; (void)u; (void)v; (void)sd; (void)S;
    if (G.A > 0.0f) { /* yellowing, blotches, edge wear, foxing */
        f32 bl = fbm(u * 3.0f, v * 3.0f, sd + 15u, 2);
        f32 edge = (px < py ? px : py);
        f32 e2 = G.W - px < G.H - py ? G.W - px : G.H - py;
        if (e2 < edge) edge = e2;
        f32 ed = edge / S + (vn(u * 9.0f, v * 9.0f, sd + 16u) - 0.5f) * 0.06f;
        f32 wear = 1.0f - sst(0.0f, 0.14f, ed);
        f32 y = G.A * (0.30f + 0.5f * bl + 0.9f * wear);
        o.r *= 1.0f - y * 0.04f; o.g *= 1.0f - y * 0.14f; o.b *= 1.0f - y * 0.34f;
        f32 dk = 1.0f - G.A * (0.10f * (1.0f - bl) + 0.28f * wear);
        o.r *= dk; o.g *= dk; o.b *= dk;
        f32 ff = 150.0f < S / 9.0f ? 150.0f : S / 9.0f;
        f32 spot = sst(0.80f, 0.86f, vn(u * ff, v * ff, sd + 17u)) * sst(0.45f, 0.65f, vn(u * 5.0f, v * 5.0f, sd + 18u));
        o.r *= 1.0f - G.A * 0.14f * spot; o.g *= 1.0f - G.A * 0.20f * spot; o.b *= 1.0f - G.A * 0.30f * spot;
    }

    *op = o; *dp = d;
}

__attribute__((noinline)) static void burn_edge(Px *o, f32 edge_u, f32 w, f32 wob, f32 u, f32 v, u32 sd) {
    f32 dd = edge_u + (fbm(u * 7.0f, v * 7.0f, sd + 21u, 3) - 0.5f) * wob;
    f32 cov = cl(1.0f - dd / w, 0.0f, 1.0f);
    burn_into(o, cov);
    if (G.bt > 0.0f) o->a *= 1.0f - sst(1.0f - G.bt * 0.55f - 0.06f, 1.0f - G.bt * 0.55f, cov);
}

__attribute__((noinline)) static void burn_hit(Px *o, const Blob *b, int i, f32 dx, f32 dy, f32 u, f32 v, u32 sd) {
    f32 r = b->r;
    f32 dist = sq(dx * dx + dy * dy) + (vn(u * 30.0f, v * 30.0f, sd + 23u + (u32)i) - 0.5f) * r * 0.7f;
    burn_into(o, cl(1.0f - dist / (2.2f * r), 0.0f, 1.0f));
    if (G.bt > 0.0f) {
        f32 hole = r * 0.95f * G.bt;
        o->a *= sst(hole, hole + 1.0f / G.S + r * 0.05f, dist);
    }
}

__attribute__((noinline)) static void fx_burn(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    (void)dp;
    f32 edge = px < py ? px : py;
    f32 e2 = G.W - px < G.H - py ? G.W - px : G.H - py;
    if (e2 < edge) edge = e2;
    f32 w = 0.02f + G.B * 0.20f;
    f32 wob = 0.04f + 0.14f * G.B;
    if (edge / G.S < w + 0.5f * wob) burn_edge(op, edge / G.S, w, wob, u, v, sd);
    for (int i = 0; i < G.nburn; i++) { /* burnt-through spots */
        const Blob *b = &G.burn[i];
        f32 dx = u - b->cx, dy = v - b->cy;
        if (dx * dx + dy * dy < 5.0f * b->r * b->r) burn_hit(op, b, i, dx, dy, u, v, sd);
    }
}

__attribute__((noinline)) static void fx_rip(Px *op, f32 *dp, f32 px, f32 py, f32 u, f32 v, u32 sd) {
    Px o = *op; f32 d = *dp; f32 S = G.S;
    (void)px; (void)py; (void)u; (void)v; (void)sd; (void)S;
    if (G.rip_mask) { /* torn edges with a pale fibrous rim */
        f32 depth = G.R * 0.14f;
        f32 m = 1e9f;
        if (G.rip_mask & 1) { f32 q = v - jag(0, u); if (q < m) m = q; }
        if (G.rip_mask & 2) { f32 q = (G.Hu - v) - jag(1, u); if (q < m) m = q; }
        if (G.rip_mask & 4) { f32 q = u - jag(2, v); if (q < m) m = q; }
        if (G.rip_mask & 8) { f32 q = (G.Wu - u) - jag(3, v); if (q < m) m = q; }
        if (G.rip_mask & 16) {
            f32 t = (u - v) * 0.70711f;
            f32 q = (u + v) * 0.70711f - (depth * 3.0f + jag(4, t + 1.0f));
            if (q < m) m = q;
        }
        if (m < 2.0f * depth + 0.03f) o.a *= sst(-0.5f / S, 0.5f / S, m);
        if (m >= 0.0f && m < depth * 0.04f + 0.004f) {
            f32 rim = (1.0f - sst(0.0f, 0.004f + depth * 0.04f, m)) * (0.55f + 0.45f * vn(u * 400.0f, v * 400.0f, sd + 36u));
            f32 k = 0.85f * rim;
            o.r = mx(o.r, 0.975f, k); o.g = mx(o.g, 0.965f, k); o.b = mx(o.b, 0.94f, k);
        }
    }

    *op = o; *dp = d;
}

static Px shade(f32 px, f32 py) {
    f32 S = G.S;
    f32 u = px / S, v = py / S;
    u32 sd = G.seed;
    Px o = {G.base[0], G.base[1], G.base[2], 1.0f};
    f32 d = 0.0f; /* brightness delta */

    if (G.T > 0.0f) fx_tex(&o, &d, px, py, u, v, sd);
    if (G.Wr > 0.0f) fx_wrinkle(&o, &d, px, py, u, v, sd);
    if (G.ncrease > 0) fx_crease(&o, &d, px, py, u, v, sd);
    if (G.nfold > 0) fx_fold(&o, &d, px, py, u, v, sd);
    f32 sh = 1.0f + d;
    o.r *= sh; o.g *= sh; o.b *= sh;

    if (G.nstain > 0) fx_stain(&o, &d, px, py, u, v, sd);
    if (G.A > 0.0f) fx_aging(&o, &d, px, py, u, v, sd);
    if (G.B > 0.0f) fx_burn(&o, &d, px, py, u, v, sd);
    if (G.rip_mask) fx_rip(&o, &d, px, py, u, v, sd);
    o.r = cl(o.r, 0.0f, 1.0f); o.g = cl(o.g, 0.0f, 1.0f); o.b = cl(o.b, 0.0f, 1.0f);
    o.a = cl(o.a, 0.0f, 1.0f);
    return o;
}

/* ------------------------------------------------------------------ ABI */

#define HAS_ALPHA (1u << 8)

#ifdef NATIVE
#define EXPORT(n)
#else
#define EXPORT(n) __attribute__((export_name(n)))
#endif

EXPORT("pc_abi_version") u32 pc_abi_version(void) { return 1; }

EXPORT("pc_manifest") uint64_t pc_manifest(void) {
    return ((uint64_t)(sizeof(MANIFEST) - 1) << 32) | (uint64_t)(uintptr_t)MANIFEST;
}

#ifndef NATIVE
EXPORT("pc_alloc") u32 pc_alloc(u32 size) {
    u32 pages = (size + 65535u) / 65536u;
    u32 old = (u32)__builtin_wasm_memory_grow(0, pages);
    return old == 0xffffffffu ? 0 : old * 65536u;
}
#endif

static void put(f32 *q, const Px *c, f32 lum, int colour, int alpha, u32 channels, int multiply) {
    if (multiply) {
        for (int k = 0; k < colour; k++) {
            f32 paper = colour == 3 ? (k == 0 ? c->r : (k == 1 ? c->g : c->b)) : lum;
            q[k] *= 1.0f - c->a + c->a * paper;
        }
        return;
    }
    if (colour == 3) {
        f32 bg = alpha ? 0.0f : 0.07f, t = alpha ? 1.0f : c->a;
        q[0] = mx(bg, c->r, t);
        q[1] = mx(bg, c->g, t);
        q[2] = mx(bg, c->b, t);
    } else {
        q[0] = alpha ? lum : mx(0.07f, lum, c->a);
    }
    if (alpha) q[channels - 1] = c->a;
}

EXPORT("pc_filter")
i32 pc_filter(f32 *buf, u32 len, u32 width, u32 height, u32 channels, u32 format, const char *params, u32 params_len) {
    if (channels == 0 || channels > 8) return 1;
    for (f32 *p = (f32 *)&P; p < (f32 *)(&P + 1); p++) *p = 0.0f;
    P.seed = 1; P.paper = 1; P.texture = 50; P.texture_size = 50; P.wrinkle_size = 50;
    P.crease_count = 6; P.fold_count = 2; P.stain_count = 3; P.burn_spots = 2; P.burn_through = 30;
    P.cw = (f32)width; P.chh = (f32)height; P.iw = (f32)width; P.ih = (f32)height;
    parse(params, (int)params_len);

    int alpha = (format & HAS_ALPHA) != 0;
    int colour = (int)channels - alpha;
    if (colour != 1 && colour != 3) return 2; /* RGB or grayscale documents only */
    if ((u32)len / 4u < width * height * channels) return 3;
    int multiply = ci(P.apply, 0, 1) == 1;

    setup(P.cw, P.chh);

    /* The sandbox gives every sample a fixed instruction budget, so documents with fewer channels
     * (grayscale) get less per pixel. There, one value is computed per step x step block of the
     * document (blocks aligned to the document, so bands stay seamless). RGB gets full detail. */
    i32 step = channels <= 2 ? (colour == 1 && !alpha ? 3 : 2) : 1;
    i32 gx = (i32)P.ix, gy = (i32)P.iy;
    if (gx < 0) gx = 0;
    if (gy < 0) gy = 0;
    i32 w = (i32)width, h = (i32)height;
    for (i32 y0 = -(gy % step); y0 < h; y0 += step) {
        for (i32 x0 = -(gx % step); x0 < w; x0 += step) {
            Px c = shade(P.ix + (f32)x0 + (f32)step * 0.5f, P.iy + (f32)y0 + (f32)step * 0.5f);
            f32 lum = 0.299f * c.r + 0.587f * c.g + 0.114f * c.b;
            for (i32 y = (y0 < 0 ? 0 : y0); y < y0 + step && y < h; y++) {
                for (i32 x = (x0 < 0 ? 0 : x0); x < x0 + step && x < w; x++) {
                    f32 *q = buf + ((u32)y * width + (u32)x) * channels;
                    put(q, &c, lum, colour, alpha, channels, multiply);
                }
            }
        }
    }
    return 0;
}
