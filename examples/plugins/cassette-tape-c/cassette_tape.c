/* Cassette Tape: a procedural compact-cassette generator for PhotoCraft (plug-in ABI v1, see
 * docs/plugins.md). Freestanding C compiled to WebAssembly: no imports, no allocator, no state.
 *
 * The cassette is drawn from above, centred on the canvas, in millimetres (a real cassette is
 * about 100 x 64 mm), so every detail keeps its proportions at any resolution. Everything is a pure
 * function of (seed, document position, canvas size): the same settings always give the same
 * image and bands stay seamless.
 *
 * Layout: backdrop and shadow, shell (finish, rim, edge wear), recessed label (paper, ruled lines,
 * stripes, handwriting, wear, stains, rips), raised window frame with hubs, tape packs and window,
 * the lower trapezoid with its holes, five screws, then scratches, scuffs, grime and dust.
 *
 * Cost note: the sandbox charges instruction budget even for skipped code in the same block, so
 * every optional effect is its own noinline function, called only when it is enabled.
 */
#include <stdint.h>

typedef float f32;
typedef uint32_t u32;
typedef int32_t i32;

static const char MANIFEST[] =
    "{\"id\":\"org.photocraft.cassette-tape\",\"name\":\"Cassette Tape\\u2026\",\"version\":\"1.0.0\",\"kind\":\"fi"
    "lter\",\"author\":\"PhotoCraft contributors\",\"description\":\"Draws a compact cassette from above with selec"
    "table shell, label and tape, plus scratch, rip, scuff, dust and grime generators.\",\"area\":\"canvas\",\"over"
    "lap\":0,\"params\":{\"seed\":{\"type\":\"int\",\"min\":0,\"max\":999999,\"default\":1},\"size\":{\"type\":\"nu"
    "mber\",\"min\":30,\"max\":95,\"default\":62},\"angle\":{\"type\":\"number\",\"min\":-45,\"max\":45,\"default\""
    ":0},\"backdrop\":{\"type\":\"choice\",\"options\":[\"studio grey\",\"white\",\"black\",\"charcoal\",\"kraft\","
    "\"blue\",\"wood\",\"transparent\"],\"default\":\"studio grey\"},\"shell_color\":{\"type\":\"choice\",\"options"
    "\":[\"white\",\"black\",\"grey\",\"red\",\"blue\",\"green\",\"yellow\",\"orange\",\"smoke\"],\"default\":\"whi"
    "te\"},\"shell_finish\":{\"type\":\"choice\",\"options\":[\"smooth\",\"matte grain\",\"speckled\",\"brushed\","
    "\"glossy\"],\"default\":\"matte grain\"},\"shell_texture\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default"
    "\":40},\"edge_wear\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"label_paper\":{\"type\":\"cho"
    "ice\",\"options\":[\"grey\",\"white\",\"cream\",\"kraft\",\"newsprint\",\"yellow\"],\"default\":\"grey\"},\"la"
    "bel_style\":{\"type\":\"choice\",\"options\":[\"blank\",\"ruled\",\"stripe\",\"stripe and ruled\",\"double str"
    "ipe\"],\"default\":\"blank\"},\"stripe_color\":{\"type\":\"choice\",\"options\":[\"red\",\"blue\",\"green\",\""
    "orange\",\"black\",\"yellow\"],\"default\":\"red\"},\"label_texture\":{\"type\":\"number\",\"min\":0,\"max\":1"
    "00,\"default\":40},\"handwriting\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"label_wear\":{"
    "\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"label_stains\":{\"type\":\"number\",\"min\":0,\"max"
    "\":100,\"default\":0},\"label_rips\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"rip_side\":{\""
    "type\":\"choice\",\"options\":[\"all\",\"top\",\"bottom\",\"left\",\"right\",\"corner\"],\"default\":\"all\"},"
    "\"tape_type\":{\"type\":\"choice\",\"options\":[\"ferric brown\",\"chrome\",\"metal\",\"gold\"],\"default\":\""
    "ferric brown\"},\"tape_amount\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":35},\"tape_texture\":{"
    "\"type\":\"number\",\"min\":0,\"max\":100,\"default\":40},\"window_glare\":{\"type\":\"number\",\"min\":0,\"max"
    "\":100,\"default\":40},\"scratches\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"scratch_count"
    "\":{\"type\":\"int\",\"min\":1,\"max\":48,\"default\":18},\"scratch_style\":{\"type\":\"choice\",\"options\":["
    "\"fine hairlines\",\"deep gouges\",\"mixed\"],\"default\":\"mixed\"},\"scuffs\":{\"type\":\"number\",\"min\":0"
    ",\"max\":100,\"default\":0},\"dust\":{\"type\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"grime\":{\"ty"
    "pe\":\"number\",\"min\":0,\"max\":100,\"default\":0},\"shadow\":{\"type\":\"number\",\"min\":0,\"max\":100,\"d"
    "efault\":60}}}";


/* ------------------------------------------------------------------ small maths */

#define PI 3.14159265f

static inline f32 fl(f32 x) { return __builtin_floorf(x); }
static inline f32 ab(f32 x) { return __builtin_fabsf(x); }
static inline f32 sq(f32 x) { return __builtin_sqrtf(x); }
static inline f32 cl(f32 x, f32 a, f32 b) { return x < a ? a : (x > b ? b : x); }
static inline f32 mx(f32 a, f32 b, f32 t) { return a + (b - a) * t; }
static inline f32 mn(f32 a, f32 b) { return a < b ? a : b; }
static inline f32 mxf(f32 a, f32 b) { return a > b ? a : b; }
static inline f32 sst(f32 e0, f32 e1, f32 x) {
    f32 t = cl((x - e0) / (e1 - e0), 0.0f, 1.0f);
    return t * t * (3.0f - 2.0f * t);
}

static f32 sn(f32 a) { /* set-up only */
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

static f32 vn(f32 x, f32 y, u32 s) { /* smooth value noise, 0..1 */
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

typedef struct { const char *name; f32 *dst; } NumKey;
typedef struct { const char *name; f32 *dst; const char *const *opts; int n; } ChoiceKey;

typedef struct {
    f32 seed, size, angle, shell_texture, label_texture, handwriting, label_wear, label_stains, label_rips, tape_amount, tape_texture, window_glare, scratches, scratch_count, scuffs, dust, grime, edge_wear, shadow;
    f32 backdrop, shell_color, shell_finish, label_paper, label_style, stripe_color, rip_side, tape_type, scratch_style;
    f32 ix, iy, iw, ih, cw, chh;
} Params;
static Params P;
static const char *const OPT_backdrop[] = {"studio grey", "white", "black", "charcoal", "kraft", "blue", "wood", "transparent"};
static const char *const OPT_shell_color[] = {"white", "black", "grey", "red", "blue", "green", "yellow", "orange", "smoke"};
static const char *const OPT_shell_finish[] = {"smooth", "matte grain", "speckled", "brushed", "glossy"};
static const char *const OPT_label_paper[] = {"grey", "white", "cream", "kraft", "newsprint", "yellow"};
static const char *const OPT_label_style[] = {"blank", "ruled", "stripe", "stripe and ruled", "double stripe"};
static const char *const OPT_stripe_color[] = {"red", "blue", "green", "orange", "black", "yellow"};
static const char *const OPT_rip_side[] = {"all", "top", "bottom", "left", "right", "corner"};
static const char *const OPT_tape_type[] = {"ferric brown", "chrome", "metal", "gold"};
static const char *const OPT_scratch_style[] = {"fine hairlines", "deep gouges", "mixed"};
static const NumKey NUMS[] = {
    {"seed", &P.seed},
    {"size", &P.size},
    {"angle", &P.angle},
    {"shell_texture", &P.shell_texture},
    {"label_texture", &P.label_texture},
    {"handwriting", &P.handwriting},
    {"label_wear", &P.label_wear},
    {"label_stains", &P.label_stains},
    {"label_rips", &P.label_rips},
    {"tape_amount", &P.tape_amount},
    {"tape_texture", &P.tape_texture},
    {"window_glare", &P.window_glare},
    {"scratches", &P.scratches},
    {"scratch_count", &P.scratch_count},
    {"scuffs", &P.scuffs},
    {"dust", &P.dust},
    {"grime", &P.grime},
    {"edge_wear", &P.edge_wear},
    {"shadow", &P.shadow},
};
#define NUM_COUNT 19
static const ChoiceKey CHOICES[] = {
    {"backdrop", &P.backdrop, OPT_backdrop, 8},
    {"shell_color", &P.shell_color, OPT_shell_color, 9},
    {"shell_finish", &P.shell_finish, OPT_shell_finish, 5},
    {"label_paper", &P.label_paper, OPT_label_paper, 6},
    {"label_style", &P.label_style, OPT_label_style, 5},
    {"stripe_color", &P.stripe_color, OPT_stripe_color, 6},
    {"rip_side", &P.rip_side, OPT_rip_side, 6},
    {"tape_type", &P.tape_type, OPT_tape_type, 4},
    {"scratch_style", &P.scratch_style, OPT_scratch_style, 3},
};
#define CHOICE_COUNT 9
#define DEF_backdrop 0
#define DEF_shell_color 0
#define DEF_shell_finish 1
#define DEF_label_paper 0
#define DEF_label_style 0
#define DEF_stripe_color 0
#define DEF_rip_side 0
#define DEF_tape_type 0
#define DEF_scratch_style 2
static void set_defaults(void) {
    P.seed = 1;
    P.size = 62;
    P.angle = 0;
    P.shell_texture = 40;
    P.label_texture = 40;
    P.handwriting = 0;
    P.label_wear = 0;
    P.label_stains = 0;
    P.label_rips = 0;
    P.tape_amount = 35;
    P.tape_texture = 40;
    P.window_glare = 40;
    P.scratches = 0;
    P.scratch_count = 18;
    P.scuffs = 0;
    P.dust = 0;
    P.grime = 0;
    P.edge_wear = 0;
    P.shadow = 60;
    P.backdrop = DEF_backdrop;
    P.shell_color = DEF_shell_color;
    P.shell_finish = DEF_shell_finish;
    P.label_paper = DEF_label_paper;
    P.label_style = DEF_label_style;
    P.stripe_color = DEF_stripe_color;
    P.rip_side = DEF_rip_side;
    P.tape_type = DEF_tape_type;
    P.scratch_style = DEF_scratch_style;
}


static const NumKey IMG[] = {
    {"x", &P.ix}, {"y", &P.iy}, {"width", &P.iw}, {"height", &P.ih},
    {"canvasWidth", &P.cw}, {"canvasHeight", &P.chh},
};

static int streq(const char *a, int al, const char *b) {
    int i = 0;
    for (; i < al; i++) if (b[i] == 0 || a[i] != b[i]) return 0;
    return b[i] == 0;
}
static void set_num(const NumKey *t, int n, const char *k, int kl, f32 v) {
    for (int i = 0; i < n; i++) if (streq(k, kl, t[i].name)) { *t[i].dst = v; return; }
}
static void set_choice(const char *k, int kl, const char *v, int vl) {
    for (int i = 0; i < CHOICE_COUNT; i++) {
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
        if (b[i] == 't' || b[i] == 'f') { while (i < n && b[i] >= 'a' && b[i] <= 'z') i++; continue; }
        f32 v = read_num(b, n, &i);
        if (img) set_num(IMG, 6, b + ks, kl, v); else set_num(NUMS, NUM_COUNT, b + ks, kl, v);
    }
}

/* ------------------------------------------------------------------ palettes */

static const f32 SHELL[9][3] = {
    {0.860f, 0.860f, 0.850f}, {0.075f, 0.075f, 0.080f}, {0.550f, 0.555f, 0.560f},
    {0.700f, 0.100f, 0.090f}, {0.120f, 0.280f, 0.620f}, {0.140f, 0.450f, 0.250f},
    {0.880f, 0.740f, 0.120f}, {0.880f, 0.420f, 0.100f}, {0.170f, 0.180f, 0.200f},
};
static const f32 PAPER[6][3] = {
    {0.780f, 0.780f, 0.770f}, {0.930f, 0.930f, 0.910f}, {0.910f, 0.870f, 0.760f},
    {0.700f, 0.550f, 0.380f}, {0.800f, 0.780f, 0.720f}, {0.930f, 0.840f, 0.450f},
};
static const f32 STRIPE[6][3] = {
    {0.750f, 0.120f, 0.100f}, {0.120f, 0.270f, 0.650f}, {0.120f, 0.500f, 0.280f},
    {0.900f, 0.450f, 0.100f}, {0.080f, 0.080f, 0.090f}, {0.930f, 0.780f, 0.150f},
};
static const f32 TAPE[4][3] = {
    {0.200f, 0.120f, 0.070f}, {0.340f, 0.340f, 0.360f}, {0.070f, 0.070f, 0.080f}, {0.420f, 0.300f, 0.120f},
};
static const f32 BACK[7][3] = {
    {0.820f, 0.820f, 0.820f}, {0.950f, 0.950f, 0.950f}, {0.060f, 0.060f, 0.065f}, {0.200f, 0.205f, 0.215f},
    {0.660f, 0.520f, 0.360f}, {0.250f, 0.400f, 0.620f}, {0.450f, 0.300f, 0.180f},
};

/* ------------------------------------------------------------------ scene set-up */

#define MAX_SCRATCH 48
#define MAX_STAIN 4

typedef struct { f32 cx, cy, dx, dy, half, str, deep; } Scratch;

static struct {
    f32 W, H, k, ca, sa; /* canvas, pixels per mm, rotation */
    u32 seed;
    f32 shell[3], paper[3], stripe[3], tape[3], back[3];
    f32 shell_lum, finish, style, ripside, scrstyle;
    int transparent, backdrop, wood;
    f32 sh_tex, lab_tex, hand, wear, stainI, rip, tape_amt, tape_tex, glare;
    f32 scr, scuff, dust, grime, edge, shadow;
    f32 rL, rR; /* tape pack radii (mm) */
    f32 scuff_dx, scuff_dy;
    int nscr, nstain;
    Scratch scratch[MAX_SCRATCH];
    f32 stain[MAX_STAIN][3];
    f32 screw_a[5];
} G;

static int ci(f32 v, int lo, int hi) {
    int n = (int)(v + 0.5f);
    return n < lo ? lo : (n > hi ? hi : n);
}

#define CASS_W 100.4f
#define CASS_H 63.8f

static void setup(f32 cw, f32 ch) {
    G.W = cw < 1.0f ? 1.0f : cw;
    G.H = ch < 1.0f ? 1.0f : ch;
    f32 span = mn(G.W, G.H * (CASS_W / CASS_H)) * cl(P.size, 30.0f, 95.0f) * 0.01f;
    G.k = span / CASS_W;
    f32 ang = cl(P.angle, -45.0f, 45.0f) * (PI / 180.0f);
    G.ca = cs(ang);
    G.sa = sn(ang);
    G.seed = (u32)ci(P.seed, 0, 999999) * 2654435761u + 777u;
    int sc = ci(P.shell_color, 0, 8);
    for (int i = 0; i < 3; i++) {
        G.shell[i] = SHELL[sc][i];
        G.paper[i] = PAPER[ci(P.label_paper, 0, 5)][i];
        G.stripe[i] = STRIPE[ci(P.stripe_color, 0, 5)][i];
        G.tape[i] = TAPE[ci(P.tape_type, 0, 3)][i];
    }
    G.shell_lum = 0.299f * G.shell[0] + 0.587f * G.shell[1] + 0.114f * G.shell[2];
    G.backdrop = ci(P.backdrop, 0, 7);
    G.transparent = G.backdrop == 7;
    G.wood = G.backdrop == 6;
    for (int i = 0; i < 3; i++) G.back[i] = BACK[G.backdrop == 7 ? 0 : G.backdrop][i];
    G.finish = (f32)ci(P.shell_finish, 0, 4);
    G.style = (f32)ci(P.label_style, 0, 4);
    G.ripside = (f32)ci(P.rip_side, 0, 5);
    G.scrstyle = (f32)ci(P.scratch_style, 0, 2);
    G.sh_tex = cl(P.shell_texture, 0, 100) * 0.01f;
    G.lab_tex = cl(P.label_texture, 0, 100) * 0.01f;
    G.hand = cl(P.handwriting, 0, 100) * 0.01f;
    G.wear = cl(P.label_wear, 0, 100) * 0.01f;
    G.stainI = cl(P.label_stains, 0, 100) * 0.01f;
    G.rip = cl(P.label_rips, 0, 100) * 0.01f;
    G.tape_amt = cl(P.tape_amount, 0, 100) * 0.01f;
    G.tape_tex = cl(P.tape_texture, 0, 100) * 0.01f;
    G.glare = cl(P.window_glare, 0, 100) * 0.01f;
    G.scr = cl(P.scratches, 0, 100) * 0.01f;
    G.scuff = cl(P.scuffs, 0, 100) * 0.01f;
    G.dust = cl(P.dust, 0, 100) * 0.01f;
    G.grime = cl(P.grime, 0, 100) * 0.01f;
    G.edge = cl(P.edge_wear, 0, 100) * 0.01f;
    G.shadow = cl(P.shadow, 0, 100) * 0.01f;

    G.rL = 6.2f + G.tape_amt * 19.0f;
    G.rR = sq(mxf(40.0f, 26.0f * 26.0f + 6.2f * 6.2f - G.rL * G.rL));

    G.nscr = G.scr > 0.0f ? ci(P.scratch_count, 1, MAX_SCRATCH) : 0;
    for (int i = 0; i < G.nscr; i++) {
        Scratch *s = &G.scratch[i];
        u32 sd = G.seed + (u32)i * 17u;
        s->cx = (r01(hs(i, 1, G.seed)) - 0.5f) * 96.0f;
        s->cy = (r01(hs(i, 2, G.seed)) - 0.5f) * 60.0f;
        /* most scratches run in a similar direction, like a sleeve rubbing the shell */
        f32 base = r01(hs(7, 7, G.seed)) * PI;
        f32 a = base + (r01(hs(i, 3, G.seed)) - 0.5f) * (r01(hs(i, 9, G.seed)) < 0.3f ? 3.0f : 0.5f);
        s->dx = cs(a);
        s->dy = sn(a);
        s->half = 2.0f + 22.0f * r01(hs(i, 4, sd)) * r01(hs(i, 5, sd));
        s->str = 0.4f + 0.6f * r01(hs(i, 6, sd));
        f32 deep = G.scrstyle == 1.0f ? 1.0f : (G.scrstyle == 2.0f ? (r01(hs(i, 8, sd)) < 0.35f ? 1.0f : 0.0f) : 0.0f);
        s->deep = deep;
    }
    G.nstain = G.stainI > 0.0f ? MAX_STAIN : 0;
    for (int i = 0; i < G.nstain; i++) {
        G.stain[i][0] = (r01(hs(i, 31, G.seed)) - 0.5f) * 70.0f;
        G.stain[i][1] = -5.9f + (r01(hs(i, 32, G.seed)) - 0.5f) * 36.0f;
        G.stain[i][2] = 3.0f + 5.5f * r01(hs(i, 33, G.seed));
    }
    f32 sa = r01(hs(5, 5, G.seed)) * PI;
    G.scuff_dx = cs(sa);
    G.scuff_dy = sn(sa);
    for (int i = 0; i < 5; i++) G.screw_a[i] = r01(hs(i, 41, G.seed)) * PI;
}

/* ------------------------------------------------------------------ geometry (millimetres) */

typedef struct { f32 r, g, b, a; } Px;

static f32 sd_box(f32 x, f32 y, f32 hx, f32 hy, f32 r) {
    f32 qx = ab(x) - hx + r, qy = ab(y) - hy + r;
    f32 ax = qx > 0.0f ? qx : 0.0f, ay = qy > 0.0f ? qy : 0.0f;
    f32 m = qx > qy ? qx : qy;
    if (m > 0.0f) m = 0.0f;
    return sq(ax * ax + ay * ay) + m - r;
}
static f32 sd_shell(f32 x, f32 y) { return sd_box(x, y, CASS_W * 0.5f, CASS_H * 0.5f, 3.4f); }
static f32 sd_label(f32 x, f32 y) { /* recessed label with chamfered top corners */
    f32 yy = y + 5.9f;
    f32 d = sd_box(x, yy, 44.8f, 20.6f, 1.2f);
    f32 cut = ((ab(x) - 44.8f) + (-yy - 20.6f) + 3.4f) * 0.70711f;
    return d > cut ? d : cut;
}
static f32 sd_pill(f32 x, f32 y) { return sd_box(x, y + 3.0f, 29.0f, 8.3f, 8.3f); }
static f32 sd_trap(f32 x, f32 y) { /* the raised lower part: top edge plus two slanted sides */
    f32 t = 16.3f - y;
    f32 l = -0.979f * (x + 31.5f) - 0.205f * (y - 16.3f);
    f32 r = 0.979f * (x - 31.5f) - 0.205f * (y - 16.3f);
    return mxf(t, mxf(l, r));
}
static f32 sd_win(f32 x, f32 y) { return sd_box(x + 0.2f, y + 4.9f, 10.6f, 3.4f, 0.5f); }

static void mulc(Px *p, f32 k) { p->r *= k; p->g *= k; p->b *= k; }
static void addc(Px *p, f32 k) { p->r += k; p->g += k; p->b += k; }
static void mixc(Px *p, f32 r, f32 g, f32 b, f32 t) { p->r = mx(p->r, r, t); p->g = mx(p->g, g, t); p->b = mx(p->b, b, t); }

/* Light comes from the top left. Brightness change for a surface edge facing (nx, ny). */
static f32 lit(f32 nx, f32 ny) { return -(nx * 0.55f + ny * 0.83f); }

/* ------------------------------------------------------------------ shell */

__attribute__((noinline)) static void fx_shell_finish(Px *p, f32 lx, f32 ly, u32 sd) {
    f32 t = G.sh_tex;
    f32 d = 0.0f;
    int f = (int)G.finish;
    if (f == 0) { /* smooth: a whisper of grain */
        d = (vn(lx * 7.0f, ly * 7.0f, sd + 1u) - 0.5f) * 0.03f;
    } else if (f == 1) { /* matte grain */
        d = (vn(lx * 8.0f, ly * 8.0f, sd + 1u) - 0.5f) * 0.07f + (vn(lx * 2.2f, ly * 2.2f, sd + 2u) - 0.5f) * 0.03f;
    } else if (f == 2) { /* speckled flecks */
        d = (vn(lx * 8.0f, ly * 8.0f, sd + 1u) - 0.5f) * 0.05f;
        f32 cx = fl(lx * 5.0f), cy = fl(ly * 5.0f);
        u32 h = hs((i32)cx, (i32)cy, sd + 3u);
        if ((h & 15u) == 0u) {
            f32 fx = lx * 5.0f - cx - r01(h >> 4) * 0.6f - 0.2f, fy = ly * 5.0f - cy - r01(h >> 12) * 0.6f - 0.2f;
            f32 e = 1.0f - sst(0.0f, 0.22f, sq(fx * fx + fy * fy));
            d += e * ((h & 16u) ? 0.30f : -0.22f);
        }
    } else if (f == 3) { /* brushed: horizontal streaks */
        d = (vn(lx * 0.6f, ly * 38.0f, sd + 1u) - 0.5f) * 0.10f + (vn(lx * 0.25f, ly * 11.0f, sd + 2u) - 0.5f) * 0.05f;
    } else { /* glossy: smooth with broad reflections */
        d = (vn(lx * 7.0f, ly * 7.0f, sd + 1u) - 0.5f) * 0.015f;
        f32 band = (lx * 0.55f + ly * 0.83f) * 0.07f;
        f32 s = 0.5f + 0.5f * (vn(band * 3.0f + 2.0f, 4.0f, sd + 5u) - 0.5f) * 2.0f;
        d += 0.18f * sst(0.62f, 0.9f, s) - 0.05f * sst(0.35f, 0.1f, s);
    }
    mulc(p, 1.0f + d * (0.3f + 1.4f * t));
}

__attribute__((noinline)) static void fx_edge_wear(Px *p, f32 ds, f32 lx, f32 ly, u32 sd) {
    /* chipped, scuffed rim: paint worn back to lighter (or paler, on dark shells) plastic */
    f32 n = fbm(lx * 0.9f, ly * 0.9f, sd + 61u, 3);
    f32 n2 = vn(lx * 6.0f, ly * 6.0f, sd + 62u);
    f32 reach = 0.4f + 1.3f * G.edge * n;
    f32 w = (1.0f - sst(0.0f, reach, -ds)) * sst(0.3f, 0.6f, n * 0.6f + n2 * 0.5f + G.edge * 0.45f - 0.2f);
    if (G.shell_lum < 0.4f) mixc(p, 0.50f, 0.50f, 0.50f, w * 0.7f);
    else mixc(p, 0.97f, 0.97f, 0.96f, w * 0.65f);
}

static void rim_shade(Px *p, f32 ds, f32 lx, f32 ly) {
    /* bevel at the shell rim */
    f32 nx = 0.0f, ny = 0.0f;
    if (ab(lx) * (CASS_H * 0.5f) > ab(ly) * (CASS_W * 0.5f)) nx = lx > 0.0f ? 1.0f : -1.0f; else ny = ly > 0.0f ? 1.0f : -1.0f;
    f32 band = 1.0f - sst(0.0f, 1.3f, -ds);
    mulc(p, 1.0f + band * 0.26f * lit(nx, ny));
    f32 inner = (1.0f - sst(0.0f, 0.35f, -ds));
    mulc(p, 1.0f - inner * 0.10f);
}

/* ------------------------------------------------------------------ label */

__attribute__((noinline)) static void fx_label_paper(Px *p, f32 lx, f32 ly, u32 sd) {
    f32 t = G.lab_tex;
    f32 grain = (vn(lx * 7.5f, ly * 7.5f, sd + 71u) - 0.5f) * 2.0f;
    f32 fib1 = vn(lx * 3.0f, ly * 0.45f, sd + 72u), fib2 = vn(lx * 0.45f, ly * 3.0f, sd + 73u);
    f32 blot = vn(lx * 0.11f, ly * 0.11f, sd + 74u) - 0.5f;
    mulc(p, 1.0f + t * (0.050f * grain + 0.060f * (fib1 + fib2 - 1.0f) + 0.10f * blot));
}

/* Ruled lines, stripes: returns nothing, edits the colour. ly is label-relative top-down. */
__attribute__((noinline)) static void fx_label_print(Px *p, f32 lx, f32 ly) {
    int style = (int)G.style;
    f32 aa = 0.7f / G.k;
    if (style == 1 || style == 3) { /* ruled lines in pale blue */
        f32 y = (ly + 22.5f) / 4.3f;
        f32 fr = ab(y - fl(y + 0.5f));
        f32 line = 1.0f - sst(0.0f, mxf(0.09f, aa) / 4.3f, fr);
        if (ly > -22.0f && ly < 14.0f && ab(lx) < 41.0f) mixc(p, 0.38f, 0.50f, 0.72f, line * 0.65f);
    }
    if (style >= 2) { /* colour stripes */
        f32 s = 1.0f - sst(-0.4f * 0 + 0.0f, aa, ly - (-21.2f) - 0.0f); /* top band: ly < -21.2 */
        f32 top = cl(0.5f - (ly + 21.2f) / aa, 0.0f, 1.0f);
        f32 bot = style == 4 ? cl(0.5f + (ly - 9.6f) / aa, 0.0f, 1.0f) : 0.0f;
        (void)s;
        f32 band = top > bot ? top : bot;
        mixc(p, G.stripe[0], G.stripe[1], G.stripe[2], band * 0.95f);
        /* a thin pale line under the top band */
        f32 hair = 1.0f - sst(0.0f, aa, ab(ly + 20.2f) - 0.12f);
        if (style != 4) mixc(p, 0.95f, 0.95f, 0.93f, hair * 0.35f);
    }
}

__attribute__((noinline)) static void fx_label_hand(Px *p, f32 lx, f32 ly, u32 sd) {
    /* two lines of cursive-like scribble above the window: a wavering pen line broken into words */
    for (int i = 0; i < 2; i++) {
        f32 yc = i == 0 ? -17.8f : -13.8f;
        f32 dy0 = ly - yc;
        if (ab(dy0) > 2.6f) continue;
        f32 x0 = -36.0f, x1 = i == 0 ? 31.0f : 6.0f + 24.0f * r01(hs(9, 9, sd));
        if (lx < x0 || lx > x1) continue;
        f32 word = vn(lx * 0.30f, (f32)i * 13.0f, sd + 82u);
        if (word < 0.33f) continue; /* gaps between words */
        f32 amp = 0.30f + 1.35f * vn(lx * 0.9f, (f32)i * 3.0f, sd + 84u);
        f32 ph = lx * 6.3f + 3.0f * vn(lx * 0.5f, 5.0f, sd + 83u);
        f32 wave = sn(ph) * amp * 0.55f + sn(ph * 2.1f + 1.0f) * amp * 0.25f;
        f32 drift = (vn(lx * 0.12f, (f32)i * 7.0f, sd + 85u) - 0.5f) * 1.2f;
        f32 dy = dy0 - drift - wave;
        f32 th = 0.17f + 0.06f * vn(lx * 2.0f, 1.0f, sd + 86u);
        f32 ink = 1.0f - sst(th, th + 0.55f / G.k, ab(dy));
        mixc(p, 0.07f, 0.09f, 0.22f, G.hand * ink * 0.88f);
    }
}

__attribute__((noinline)) static void fx_label_age(Px *p, f32 dl, f32 lx, f32 ly, u32 sd) {
    f32 w = G.wear;
    f32 bl = fbm(lx * 0.12f, ly * 0.12f, sd + 91u, 3);
    f32 edge = 1.0f - sst(0.0f, 5.0f, -dl);
    f32 y = w * (0.25f + 0.55f * bl + 0.7f * edge);
    mulc(p, 1.0f - y * 0.10f);
    p->b *= 1.0f - y * 0.22f; p->g *= 1.0f - y * 0.07f;
    f32 spot = sst(0.80f, 0.86f, vn(lx * 3.0f, ly * 3.0f, sd + 92u)) * sst(0.4f, 0.65f, vn(lx * 0.25f, ly * 0.25f, sd + 93u));
    mixc(p, 0.45f, 0.30f, 0.15f, w * 0.35f * spot);
    f32 scuff = sst(0.62f, 0.85f, vn(lx * 0.7f, ly * 0.7f, sd + 94u)) * sst(0.0f, 4.0f, 4.0f + dl * 0.0f) * (0.5f + 0.5f * edge);
    mulc(p, 1.0f + w * 0.10f * scuff);
}

__attribute__((noinline)) static void stain_one(Px *p, int i, f32 lx, f32 ly, u32 sd) {
    f32 dx = lx - G.stain[i][0], dy = ly - G.stain[i][1], r = G.stain[i][2];
    f32 dist = sq(dx * dx + dy * dy) + (vn(lx * 0.35f, ly * 0.35f, sd + 101u + (u32)i) - 0.5f) * r * 0.5f;
    f32 e = (dist - r) / (r * 0.07f);
    f32 ring = 1.0f / (1.0f + e * e);
    f32 fill = 0.25f * (1.0f - sst(0.0f, r, dist));
    f32 k = G.stainI * (0.85f * ring + fill);
    p->r *= 1.0f - k * 0.30f; p->g *= 1.0f - k * 0.50f; p->b *= 1.0f - k * 0.80f;
}

__attribute__((noinline)) static void fx_label_stains(Px *p, f32 lx, f32 ly, u32 sd) {
    for (int i = 0; i < G.nstain; i++) {
        f32 dx = lx - G.stain[i][0], dy = ly - G.stain[i][1], r = G.stain[i][2];
        if (dx * dx + dy * dy < 2.6f * r * r) stain_one(p, i, lx, ly, sd);
    }
}

/* Side weight for where label rips can happen. */
static f32 rip_weight(f32 lx, f32 ly) {
    int s = (int)G.ripside;
    f32 top = 1.0f - sst(-26.5f, -14.0f, ly);
    f32 bottom = sst(2.0f, 14.7f, ly);
    f32 left = 1.0f - sst(-44.8f, -26.0f, lx);
    f32 right = sst(26.0f, 44.8f, lx);
    if (s == 0) return mxf(mxf(top, bottom), mxf(left, right));
    if (s == 1) return top;
    if (s == 2) return bottom;
    if (s == 3) return left;
    if (s == 4) return right;
    f32 dx = lx + 44.8f, dy = ly + 26.5f; /* torn top-left corner */
    return 1.0f - sst(4.0f, 26.0f, sq(dx * dx + dy * dy));
}

/* The label is torn where the jagged depth reaches in from its edge. Returns the paper's own
 * coverage 0..1 and tints the paper rim; the caller shows the shell where coverage is 0. */
__attribute__((noinline)) static f32 fx_label_rip(Px *p, f32 dl, f32 lx, f32 ly, u32 sd) {
    f32 wgt = rip_weight(lx, ly);
    f32 maxd = 1.5f + 13.0f * G.rip;
    if (wgt <= 0.0f || -dl > maxd * 1.3f + 1.0f) return 1.0f;
    f32 n = fbm(lx * 0.28f, ly * 0.28f, sd + 111u, 3);
    f32 j = vn(lx * 2.4f, ly * 2.4f, sd + 112u);
    f32 depth = maxd * wgt * (0.10f + 1.5f * n * n) + 0.5f * j * wgt * (0.3f + G.rip);
    f32 m = -dl - depth; /* positive on the paper side of the tear */
    f32 cov = sst(-0.5f / G.k, 0.5f / G.k, m);
    if (m >= 0.0f) {
        f32 rim = (1.0f - sst(0.0f, 0.45f + 0.5f * G.rip, m)) * (0.55f + 0.45f * vn(lx * 9.0f, ly * 9.0f, sd + 113u));
        mixc(p, 0.97f, 0.96f, 0.93f, 0.8f * rim);
    }
    return cov;
}

/* ------------------------------------------------------------------ screws */

__attribute__((noinline)) static void fx_screw(Px *p, f32 dx, f32 dy, int i) {
    f32 r = sq(dx * dx + dy * dy);
    f32 cov = cl(0.5f - (r - 1.65f) * G.k, 0.0f, 1.0f);
    if (cov <= 0.0f) return;
    f32 head = 0.10f + 0.07f * (1.0f - sst(0.0f, 1.65f, r)) + 0.06f * (dx * -0.5f + dy * -0.7f) / 1.65f;
    f32 ca = cs(G.screw_a[i]), sa = sn(G.screw_a[i]);
    f32 u = dx * ca + dy * sa, v = -dx * sa + dy * ca;
    f32 cross = mxf(1.0f - sst(0.22f, 0.22f + 0.4f / G.k, ab(v)), 1.0f - sst(0.22f, 0.22f + 0.4f / G.k, ab(u))) * (1.0f - sst(0.9f, 1.15f, r));
    f32 c = head * (1.0f - 0.65f * cross) + 0.25f * (1.0f - sst(1.2f, 1.65f, r)) * 0.0f;
    f32 ring = sst(1.25f, 1.55f, r) * (1.0f - sst(1.55f, 1.7f, r));
    c += ring * 0.10f;
    mixc(p, c, c, c * 1.05f, cov);
    /* soft shadow around the screw on the shell */
    (void)0;
}

/* ------------------------------------------------------------------ window frame, hubs, tape */

__attribute__((noinline)) static void fx_hub(Px *p, f32 dx, f32 dy) {
    f32 r = sq(dx * dx + dy * dy);
    if (r > 5.6f) return;
    /* collar, hole and six teeth */
    f32 ring_edge = sst(5.0f, 5.6f, r);
    mulc(p, 1.0f + 0.05f * (dx * -0.55f + dy * -0.83f) / 5.6f - 0.06f * ring_edge);
    if (r < 3.7f) {
        f32 hole = 1.0f - sst(3.5f, 3.7f, r);
        f32 shade = 0.70f + 0.10f * (dx * 0.4f + dy * 0.8f) / 3.7f;
        f32 tooth = 0.0f;
        for (int a = 0; a < 3; a++) {
            f32 ca = a == 0 ? 1.0f : (a == 1 ? 0.5f : -0.5f), sa = a == 0 ? 0.0f : (a == 1 ? 0.86603f : 0.86603f);
            f32 along = dx * ca + dy * sa, perp = -dx * sa + dy * ca;
            if (ab(perp) < 0.55f && ab(along) > 2.5f && ab(along) < 3.8f) tooth = 1.0f;
        }
        f32 centre = 1.0f - sst(2.2f, 2.5f, r);
        f32 k = tooth > 0.0f ? 0.97f : shade;
        mulc(p, mx(1.0f, k, hole));
        mulc(p, 1.0f - 0.35f * centre * hole);
    }
}

/* Tape pack (concentric layers) seen through the window; returns coverage. */
static f32 tape_pack(Px *t, f32 dx, f32 dy, f32 R, u32 sd) {
    f32 r = sq(dx * dx + dy * dy);
    f32 wob = G.tape_tex * 0.5f * (vn(dx * 0.5f, dy * 0.5f, sd + 121u) - 0.5f);
    f32 cov = cl(0.5f - (r - R - wob) * G.k, 0.0f, 1.0f);
    if (cov <= 0.0f) return 0.0f;
    f32 ringv = 0.5f + 0.5f * (1.0f - 2.0f * ab((r * 3.2f) - fl(r * 3.2f) - 0.5f));
    f32 sheen = sst(0.35f, 1.0f, (dx * -0.55f + dy * -0.83f) / (r + 0.001f));
    f32 k = 1.0f + G.tape_tex * (0.30f * (ringv - 0.5f) + 0.20f * (vn(dx * 3.0f, dy * 3.0f, sd + 122u) - 0.5f)) + 0.45f * sheen * (0.4f + G.tape_tex);
    t->r = G.tape[0] * k; t->g = G.tape[1] * k; t->b = G.tape[2] * k;
    return cov;
}

__attribute__((noinline)) static void fx_window(Px *p, f32 lx, f32 ly, f32 dw, u32 sd) {
    Px c = {0.025f, 0.025f, 0.03f, 1.0f};
    /* tape runs along the bottom of the window */
    f32 strand = cl(0.5f - (ab(ly + 2.1f) - 0.55f) * G.k, 0.0f, 1.0f);
    if (strand > 0.0f) {
        f32 g = 1.0f + 0.5f * G.tape_tex * (vn(lx * 0.8f, 9.0f, sd + 123u) - 0.3f);
        mixc(&c, G.tape[0] * g * 1.3f, G.tape[1] * g * 1.3f, G.tape[2] * g * 1.3f, strand);
    }
    Px t = {0, 0, 0, 1};
    f32 a = tape_pack(&t, lx + 20.5f, ly + 3.2f, G.rL, sd);
    if (a > 0.0f) mixc(&c, t.r, t.g, t.b, a);
    a = tape_pack(&t, lx - 20.5f, ly + 3.2f, G.rR, sd + 5u);
    if (a > 0.0f) mixc(&c, t.r, t.g, t.b, a);
    /* the window sits a little below the frame: shade its top and left walls */
    f32 wall = (1.0f - sst(0.0f, 0.9f, -dw));
    mulc(&c, 1.0f - 0.45f * wall * (ly < -4.9f ? 1.0f : 0.4f));
    /* clear plastic: diagonal glare plus a soft overall sheen */
    if (G.glare > 0.0f) {
        f32 s = (lx * 0.9f + ly * 0.5f) * 0.18f;
        f32 g = sst(0.0f, 0.5f, 0.5f - ab(s - fl(s + 0.5f) + 0.15f) * 2.2f) * 0.5f;
        g += 0.10f * (1.0f - sst(-8.3f, -1.5f, ly));
        addc(&c, G.glare * g * 0.55f);
    }
    mixc(p, c.r, c.g, c.b, cl(0.5f - dw * G.k, 0.0f, 1.0f) * 1.0f);
}

__attribute__((noinline)) static void fx_pill(Px *p, f32 dp, f32 lx, f32 ly, u32 sd) {
    /* the raised frame round the hubs: lit top-left rim, tick marks under the window */
    f32 cxp = lx < -20.7f ? -20.7f : (lx > 20.7f ? 20.7f : lx);
    f32 nx = lx - cxp, ny = ly + 3.0f;
    f32 nl = sq(nx * nx + ny * ny) + 0.0001f;
    f32 band = 1.0f - sst(0.0f, 0.9f, -dp);
    mulc(p, 1.0f + 0.30f * band * lit(nx / nl, ny / nl) + 0.025f);
    fx_hub(p, lx + 20.5f, ly + 3.2f);
    fx_hub(p, lx - 20.5f, ly + 3.2f);
    f32 dw = sd_win(lx, ly);
    if (dw < 0.9f) fx_window(p, lx, ly, dw, sd);
    /* scale ticks below the window */
    if (ly > -1.1f && ly < 0.2f && ab(lx + 0.2f) < 10.6f) {
        f32 u = (lx + 10.8f) / 2.1f;
        f32 tick = 1.0f - sst(0.0f, mxf(0.07f, 0.8f / G.k) / 2.1f, ab(u - fl(u + 0.5f)));
        mulc(p, 1.0f - 0.18f * tick);
    }
}

/* ------------------------------------------------------------------ lower part */

__attribute__((noinline)) static void fx_hole(Px *p, f32 dx, f32 dy, f32 r, int square) {
    f32 d = square ? sd_box(dx, dy, r, r, 0.35f) : sq(dx * dx + dy * dy) - r;
    f32 cov = cl(0.5f - d * G.k, 0.0f, 1.0f);
    if (cov <= 0.0f) return;
    f32 depth = -d;
    f32 c = 0.11f + 0.20f * (1.0f - sst(0.0f, 1.3f, depth)) * (dy > 0.0f ? 1.0f : 0.0f);
    if (!square) c += 0.18f * (dy > 0.7f ? sst(r - 0.5f, r, depth + 0.0f) * 0.0f : 0.0f);
    mixc(p, c, c, c * 1.04f, cov);
}

__attribute__((noinline)) static void fx_trap(Px *p, f32 dt, f32 lx, f32 ly) {
    f32 band = 1.0f - sst(0.0f, 0.9f, -dt);
    f32 tt = 16.3f - ly;
    f32 ll = -0.979f * (lx + 31.5f) - 0.205f * (ly - 16.3f);
    f32 rr = 0.979f * (lx - 31.5f) - 0.205f * (ly - 16.3f);
    f32 nx = 0.0f, ny = -1.0f;
    if (ll > tt && ll > rr) { nx = -0.979f; ny = -0.205f; }
    else if (rr > tt) { nx = 0.979f; ny = -0.205f; }
    mulc(p, 1.0f + 0.28f * band * lit(nx, ny) + 0.02f);
    f32 hx = 23.65f;
    for (int i = 0; i < 2; i++) {
        f32 sgn = i ? 1.0f : -1.0f;
        f32 dx = lx - sgn * hx, dy = ly - 27.8f;
        if (dx * dx + dy * dy < 7.0f) fx_hole(p, dx, dy, 2.3f, 0);
        f32 sx = lx - sgn * 14.15f, sy = ly - 26.8f;
        if (ab(sx) < 2.6f && ab(sy) < 2.6f) fx_hole(p, sx, sy, 1.8f, 1);
    }
}

/* ------------------------------------------------------------------ wear on top of everything */

__attribute__((noinline)) static f32 scratch_hit(const Scratch *s, f32 rx, f32 ry, f32 across, f32 lum) {
    f32 along = rx * s->dx + ry * s->dy;
    f32 ends = 1.0f - sst(s->half * 0.55f, s->half, ab(along));
    if (ends <= 0.0f) return 0.0f;
    f32 w = mxf(0.05f + 0.04f * s->deep, 0.8f / G.k);
    f32 wob = (vn(along * 0.9f, 3.0f, G.seed + 131u) - 0.5f) * 0.25f;
    f32 q = (across + wob) / w;
    f32 core = 1.0f / (1.0f + q * q);
    f32 d = 0.0f;
    f32 broken = 0.4f + 0.9f * vn(along * 2.6f, s->cx, G.seed + 132u);
    if (s->deep > 0.5f) { /* gouge: a dark groove with a bright lip */
        d = -0.40f * core + 0.45f * (q / (1.0f + q * q * 0.6f)) * 0.5f;
        d += lum < 0.4f ? 0.20f * core : 0.0f;
    } else { /* hairline: pale on dark plastic, dark on pale plastic */
        d = lum < 0.45f ? 0.45f * core : -0.16f * core;
    }
    return d * s->str * ends * broken;
}

__attribute__((noinline)) static void fx_scratches(Px *p, f32 lx, f32 ly, f32 lum) {
    f32 d = 0.0f;
    for (int i = 0; i < G.nscr; i++) {
        const Scratch *s = &G.scratch[i];
        f32 rx = lx - s->cx, ry = ly - s->cy;
        f32 across = -rx * s->dy + ry * s->dx;
        if (ab(across) < 0.32f) d += scratch_hit(s, rx, ry, across, lum);
    }
    if (d != 0.0f) {
        d *= G.scr;
        if (d > 0.0f) mixc(p, 0.92f, 0.92f, 0.92f, cl(d, 0.0f, 0.9f)); else mulc(p, 1.0f + d);
    }
}

__attribute__((noinline)) static void fx_scuffs(Px *p, f32 lx, f32 ly, f32 lum) {
    f32 u = lx * G.scuff_dx + ly * G.scuff_dy, v = -lx * G.scuff_dy + ly * G.scuff_dx;
    f32 n = fbm(u * 0.10f, v * 0.9f, G.seed + 141u, 3);
    f32 m = sst(0.52f, 0.78f, n) * (0.5f + 0.5f * vn(lx * 4.0f, ly * 4.0f, G.seed + 142u));
    f32 a = G.scuff * m;
    if (lum < 0.45f) mixc(p, 0.45f, 0.45f, 0.46f, a * 0.55f); else mulc(p, 1.0f - a * 0.22f);
}

__attribute__((noinline)) static void fx_grime(Px *p, f32 ds, f32 dl, f32 dp, f32 lx, f32 ly) {
    /* dirt collects in the creases around the label, frame and rim */
    f32 n = 0.35f + 0.65f * fbm(lx * 0.45f, ly * 0.45f, G.seed + 151u, 2);
    f32 crease = mxf(1.0f - sst(0.0f, 1.1f, ab(dl)), mxf(1.0f - sst(0.0f, 1.1f, ab(dp)), 1.0f - sst(0.0f, 2.0f, -ds)));
    f32 broad = 0.35f * (1.0f - sst(0.0f, 14.0f, -ds));
    f32 g = G.grime * (crease * 0.55f + broad) * n;
    mixc(p, p->r * 0.55f, p->g * 0.5f, p->b * 0.42f, cl(g, 0.0f, 0.8f));
}

__attribute__((noinline)) static void fx_dust(Px *p, f32 lx, f32 ly, u32 sd) {
    f32 cx = fl(lx * 2.4f), cy = fl(ly * 2.4f);
    u32 h = hs((i32)cx, (i32)cy, sd + 161u);
    if (r01(h) > G.dust * 0.06f) return;
    f32 fx = lx * 2.4f - cx - r01(h >> 3), fy = ly * 2.4f - cy - r01(h >> 11);
    f32 sz = 0.05f + 0.16f * r01(h >> 5);
    f32 e = 1.0f - sst(0.0f, sz * 2.4f, sq(fx * fx + fy * fy) / 2.4f * 2.4f * 0.5f);
    if (h & 0x100u) mixc(p, 0.95f, 0.95f, 0.93f, e * 0.8f); else mulc(p, 1.0f - e * 0.6f);
}

/* ------------------------------------------------------------------ the picture */

static void backdrop(Px *p, f32 px, f32 py) {
    f32 u = (px - G.W * 0.5f) / G.W, v = (py - G.H * 0.5f) / G.H;
    f32 vig = 1.0f - 0.20f * (u * u * 1.3f + v * v * 1.3f);
    f32 g = (vn(px * 0.9f, py * 0.9f, G.seed + 171u) - 0.5f) * 0.035f;
    f32 k = vig + g;
    if (G.wood) k += (vn(px * 0.02f, py * 0.9f, G.seed + 172u) - 0.5f) * 0.30f * (0.4f + 0.6f * vn(px * 0.004f, py * 0.05f, G.seed + 173u));
    p->r = G.back[0] * k; p->g = G.back[1] * k; p->b = G.back[2] * k;
}

static Px shade(f32 px, f32 py) {
    f32 cx = px - G.W * 0.5f, cy = py - G.H * 0.5f;
    f32 lx = (G.ca * cx + G.sa * cy) / G.k, ly = (-G.sa * cx + G.ca * cy) / G.k;
    u32 sd = G.seed;
    Px o = {0.0f, 0.0f, 0.0f, 1.0f};
    backdrop(&o, px, py);
    f32 ds = sd_shell(lx, ly);
    f32 shadow_a = 0.0f;
    if (ds < 9.0f && G.shadow > 0.0f) {
        f32 d2 = sd_shell(lx - 1.6f, ly - 2.4f);
        f32 s = (1.0f - sst(-3.0f, 6.5f, d2)) * 0.42f + (1.0f - sst(0.0f, 1.4f, ds)) * 0.28f;
        shadow_a = s * G.shadow;
        mulc(&o, 1.0f - shadow_a);
    }
    f32 cov = cl(0.5f - ds * G.k, 0.0f, 1.0f);
    if (cov > 0.0f) {
        Px c = {G.shell[0], G.shell[1], G.shell[2], 1.0f};
        fx_shell_finish(&c, lx, ly, sd);
        f32 dl = sd_label(lx, ly), dp = sd_pill(lx, ly);
        f32 dt = sd_trap(lx, ly);
        f32 lum = G.shell_lum;
        if (dp < 0.0f) {
            fx_pill(&c, dp, lx, ly, sd);
        } else if (dl < 0.0f) {
            Px lab = {G.paper[0], G.paper[1], G.paper[2], 1.0f};
            if (G.lab_tex > 0.0f) fx_label_paper(&lab, lx, ly, sd);
            if (G.style > 0.0f) fx_label_print(&lab, lx, ly);
            if (G.hand > 0.0f) fx_label_hand(&lab, lx, ly, sd);
            if (G.wear > 0.0f) fx_label_age(&lab, dl, lx, ly, sd);
            if (G.nstain > 0) fx_label_stains(&lab, lx, ly, sd);
            f32 paper_cov = 1.0f;
            if (G.rip > 0.0f) paper_cov = fx_label_rip(&lab, dl, lx, ly, sd);
            /* the label sits in a shallow recess: shadow on its top and left edges */
            f32 nxl = lx < -44.0f + 0.0f ? 1.0f : 0.0f; (void)nxl;
            f32 edge = 1.0f - sst(0.0f, 0.8f, -dl);
            f32 top = 1.0f - sst(-26.5f, -24.8f, ly), left = 1.0f - sst(-44.8f, -43.2f, lx);
            mulc(&lab, 1.0f - 0.16f * edge * mxf(top, left) + 0.05f * edge * (1.0f - mxf(top, left)));
            /* soft shadow of the window frame on the label */
            mulc(&lab, 1.0f - 0.30f * (1.0f - sst(0.0f, 1.8f, dp)));
            lum = 0.6f;
            if (paper_cov < 1.0f) {
                /* torn away: the shell recess shows, with a thin shadow under the torn edge */
                mulc(&c, 0.80f);
                mixc(&c, lab.r, lab.g, lab.b, paper_cov);
                if (paper_cov < 0.99f) mulc(&c, 1.0f - 0.0f);
            } else {
                c = lab;
            }
        } else if (dt < 0.0f) {
            fx_trap(&c, dt, lx, ly);
        } else {
            /* plain shell: a faint step where it meets the label recess */
            mulc(&c, 1.0f - 0.10f * (1.0f - sst(0.0f, 1.0f, dl)));
        }
        rim_shade(&c, ds, lx, ly);
        if (G.edge > 0.0f) fx_edge_wear(&c, ds, lx, ly, sd);
        /* screws */
        static const f32 SX[5] = {-46.6f, 46.6f, -46.6f, 46.6f, 0.0f};
        static const f32 SY[5] = {-28.5f, -28.5f, 28.0f, 28.0f, 20.8f};
        for (int i = 0; i < 5; i++) {
            f32 dx = lx - SX[i], dy = ly - SY[i];
            if (dx * dx + dy * dy < 7.5f) {
                f32 sh = 1.0f - 0.18f * (1.0f - sst(1.5f, 2.8f, sq(dx * dx + dy * dy)));
                mulc(&c, sh);
                fx_screw(&c, dx, dy, i);
            }
        }
        if (G.nscr > 0) fx_scratches(&c, lx, ly, lum);
        if (G.scuff > 0.0f) fx_scuffs(&c, lx, ly, lum);
        if (G.grime > 0.0f) fx_grime(&c, ds, dl, dp, lx, ly);
        o.r = mx(o.r, c.r, cov); o.g = mx(o.g, c.g, cov); o.b = mx(o.b, c.b, cov);
    }
    if (G.dust > 0.0f) fx_dust(&o, lx, ly, sd);
    o.r = cl(o.r, 0.0f, 1.0f); o.g = cl(o.g, 0.0f, 1.0f); o.b = cl(o.b, 0.0f, 1.0f);
    if (G.transparent) o.a = cl(cov + shadow_a * (1.0f - cov), 0.0f, 1.0f);
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

static void put(f32 *q, const Px *c, f32 lum, int colour, int alpha, u32 channels) {
    if (colour == 3) {
        f32 bg = alpha ? 0.0f : 0.5f;
        f32 t = alpha ? 1.0f : c->a;
        q[0] = mx(bg, c->r, t);
        q[1] = mx(bg, c->g, t);
        q[2] = mx(bg, c->b, t);
    } else {
        q[0] = alpha ? lum : mx(0.5f, lum, c->a);
    }
    if (alpha) q[channels - 1] = c->a;
}

EXPORT("pc_filter")
i32 pc_filter(f32 *buf, u32 len, u32 width, u32 height, u32 channels, u32 format, const char *params, u32 params_len) {
    if (channels == 0 || channels > 8) return 1;
    for (f32 *p = (f32 *)&P; p < (f32 *)(&P + 1); p++) *p = 0.0f;
    set_defaults();
    P.cw = (f32)width; P.chh = (f32)height; P.iw = (f32)width; P.ih = (f32)height;
    parse(params, (int)params_len);

    int alpha = (format & HAS_ALPHA) != 0;
    int colour = (int)channels - alpha;
    if (colour != 1 && colour != 3) return 2; /* RGB or grayscale documents only */
    if ((u32)len / 4u < width * height * channels) return 3;

    setup(P.cw, P.chh);

    /* Fixed instruction budget per sample: a grayscale layer without alpha (one sample per pixel) gets a
     * value per 2 x 2 block, aligned to the document so bands stay seamless. */
    i32 step = (colour == 1 && !alpha) ? 2 : 1;
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
                    put(q, &c, lum, colour, alpha, channels);
                }
            }
        }
    }
    return 0;
}
