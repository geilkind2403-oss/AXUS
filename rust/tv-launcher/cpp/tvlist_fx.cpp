// tvlist_fx -- the Live TV channel list's picture (C ABI, freestanding).
//
// A black-gold wave background in the style of a games-console menu, the
// Settings-like glass rows, the gliding gold focus, and a window cut into
// it where the running channel keeps playing (the video plane below shows
// through). Rust draws the text once per key press into a transparent
// layer; this module lays it on top.
//
// Cost model (Pi 3, 1080p): the wave is computed on a 3x smaller grid with
// its curves evaluated once per column, then scaled up bilinearly -- a few
// integer multiply-adds per pixel. Rows, focus and the window only touch
// their own rectangles. Pixels: B,G,R,A bytes, premultiplied.

typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

extern "C" void *malloc(size_t);
extern "C" void free(void *);

namespace {

const float PI = 3.14159265f;
const float TAU = 6.28318531f;
inline float absf(float x) { return x < 0.0f ? -x : x; }
inline float minf(float a, float b) { return a < b ? a : b; }
inline float maxf(float a, float b) { return a > b ? a : b; }
inline int mini(int a, int b) { return a < b ? a : b; }
inline int maxi(int a, int b) { return a > b ? a : b; }
inline float clamp01(float x) { return x < 0.0f ? 0.0f : (x > 1.0f ? 1.0f : x); }
inline float sqrtf_(float x) { return __builtin_sqrtf(x); }
inline int floori(float x) {
    int i = (int)x;
    return x < (float)i ? i - 1 : i;
}
inline int ceili(float x) {
    int i = (int)x;
    return x > (float)i ? i + 1 : i;
}
inline float sinf_(float x) {
    x *= 1.0f / TAU;
    x -= (float)floori(x + 0.5f);
    x *= TAU;
    float y = 1.27323954f * x - 0.405284735f * x * absf(x);
    return 0.225f * (y * absf(y) - y) + y;
}
inline uint32_t div255(uint32_t x) {
    x += 128;
    return (x + (x >> 8)) >> 8;
}
inline float sd_box(float px, float py, float x, float y, float w, float h, float r) {
    r = minf(r, minf(w, h) * 0.5f);
    float cx = x + w * 0.5f, cy = y + h * 0.5f;
    float qx = absf(px - cx) - (w * 0.5f - r), qy = absf(py - cy) - (h * 0.5f - r);
    float ox = maxf(qx, 0.0f), oy = maxf(qy, 0.0f);
    return sqrtf_(ox * ox + oy * oy) + minf(maxf(qx, qy), 0.0f) - r;
}
// straight colour over a premultiplied pixel, alpha 0..255
inline void over(uint8_t *d, uint32_t r, uint32_t g, uint32_t b, uint32_t a) {
    if (a == 0) return;
    uint32_t inv = 255 - a;
    d[0] = (uint8_t)div255(b * a + d[0] * inv);
    d[1] = (uint8_t)div255(g * a + d[1] * inv);
    d[2] = (uint8_t)div255(r * a + d[2] * inv);
    d[3] = (uint8_t)(a + div255(d[3] * inv));
}

const int D = 3;          // wave grid: every 3rd pixel
const int RIBBONS = 5;
const int MAX_ROWS = 16;

struct Box {
    float x, y, w, h, r;
};

} // namespace

extern "C" {

typedef struct TvlScene {
    int32_t w, h;
    float sc;
    int32_t gw, gh;      // wave grid
    uint8_t *wave;       // gw * gh * 3 (B,G,R)
    float *curve;        // RIBBONS * gw: centre line of each ribbon
    int32_t *xs;         // per frame column: grid column and weight (fixed 8.8)
    Box hole;            // the video window (physical px)
    int has_hole;
    Box rows[MAX_ROWS];
    uint8_t row_flags[MAX_ROWS];
    int n_rows;
    const uint8_t *text; // full-frame transparent layer from Rust
    int32_t text_stride;
    int32_t *text_span;  // per frame row: first, last+1 visible column
    uint8_t *full;       // the wave at full size, w * h * 4, made once
    int32_t offset;      // how far it has drifted (columns)
} TvlScene;
static void build_wave(TvlScene *s);

TvlScene *tvl_create(int32_t w, int32_t h, float scale) {
    if (w <= 0 || h <= 0 || scale <= 0.0f) return nullptr;
    TvlScene *s = (TvlScene *)malloc(sizeof(TvlScene));
    if (!s) return nullptr;
    __builtin_memset(s, 0, sizeof(TvlScene));
    s->w = w;
    s->h = h;
    s->sc = scale;
    s->gw = w / D + 2;
    s->gh = h / D + 2;
    s->wave = (uint8_t *)malloc((size_t)s->gw * s->gh * 3);
    s->curve = (float *)malloc(sizeof(float) * RIBBONS * (size_t)s->gw);
    s->xs = (int32_t *)malloc(sizeof(int32_t) * 2 * (size_t)w);
    s->text_span = (int32_t *)malloc(sizeof(int32_t) * 2 * (size_t)h);
    if (!s->wave || !s->curve || !s->xs || !s->text_span) {
        free(s->wave);
        free(s->curve);
        free(s->xs);
        free(s->text_span);
        free(s);
        return nullptr;
    }
    for (int x = 0; x < w; x++) {
        float gx = ((float)x + 0.5f) / D - 0.5f;
        if (gx < 0.0f) gx = 0.0f;
        int i = (int)gx;
        s->xs[x * 2] = i;
        s->xs[x * 2 + 1] = (int32_t)((gx - (float)i) * 256.0f);
    }
    for (int y = 0; y < h; y++) s->text_span[y * 2] = s->text_span[y * 2 + 1] = 0;
    s->full = (uint8_t *)malloc((size_t)w * h * 4);
    if (s->full) build_wave(s);
    // the grid is only needed to make the wave
    free(s->wave);
    free(s->curve);
    s->wave = nullptr;
    s->curve = nullptr;
    return s;
}

void tvl_destroy(TvlScene *s) {
    if (!s) return;
    free(s->wave);
    free(s->curve);
    free(s->xs);
    free(s->text_span);
    free(s->full);
    free(s);
}

// The window where the video plays (logical); w <= 0: none.
void tvl_set_hole(TvlScene *s, float x, float y, float w, float h, float r) {
    if (!s) return;
    s->has_hole = w > 0.0f && h > 0.0f;
    float k = s->sc;
    s->hole = Box{x * k, y * k, w * k, h * k, r * k};
}

// Glass rows (logical x, y, w, h each); flag 1: the playing channel.
void tvl_set_rows(TvlScene *s, const float *rects, const uint8_t *flags, int32_t n, float radius) {
    if (!s) return;
    s->n_rows = mini(maxi(n, 0), MAX_ROWS);
    float k = s->sc;
    for (int i = 0; i < s->n_rows; i++) {
        s->rows[i] = Box{rects[i * 4] * k, rects[i * 4 + 1] * k, rects[i * 4 + 2] * k, rects[i * 4 + 3] * k,
                         radius * k};
        s->row_flags[i] = flags ? flags[i] : 0;
    }
}

// The text layer (full frame, premultiplied); kept by the caller while used.
void tvl_set_text(TvlScene *s, const uint8_t *px, int32_t stride) {
    if (!s) return;
    s->text = px;
    s->text_stride = stride;
    for (int y = 0; y < s->h; y++) {
        int a = 0, b = s->w;
        if (px) {
            const uint8_t *row = px + (ptrdiff_t)stride * y;
            while (a < b && row[a * 4 + 3] == 0) a++;
            while (b > a && row[(b - 1) * 4 + 3] == 0) b--;
        } else {
            a = b = 0;
        }
        s->text_span[y * 2] = a;
        s->text_span[y * 2 + 1] = b;
    }
}

// The wave, once, at full size: a warm black gradient and soft gold
// ribbons whose curves repeat exactly across the width, so the picture can
// drift sideways forever by copying rows with an offset.
static void build_wave(TvlScene *s) {
    const int gw = s->gw, gh = s->gh, W = s->w;
    const float H = (float)s->h;
    for (int r = 0; r < RIBBONS; r++) {
        float base = H * (0.50f + 0.075f * (float)r);
        float a1 = H * (0.07f + 0.02f * (float)r), a2 = H * 0.035f;
        // whole periods across the width: seamless when it wraps
        float k1 = TAU * (float)(1 + (r & 1)) / (float)W;
        float k2 = TAU * (float)(2 + r) / (float)W;
        float p = (float)r * 1.7f;
        float *c = s->curve + r * gw;
        for (int gx = 0; gx < gw; gx++) {
            float x = (float)(gx * D);
            c[gx] = (base + a1 * sinf_(x * k1 + p) + a2 * sinf_(x * k2 + p * 2.3f)) / D;
        }
    }
    // soft, out-of-focus ribbons: a wide glowing core in a broad haze
    const float line_s = 1.0f / (4.5f * s->sc / D * 1.6f);
    const float band_s = 1.0f / (60.0f * s->sc / D);
    for (int gy = 0; gy < gh; gy++) {
        float fy = (float)gy / (float)(gh - 1);
        float glow = 1.0f - absf(fy - 0.58f) * 2.2f;
        glow = glow > 0.0f ? glow * glow : 0.0f;
        float br = 10.0f + 26.0f * glow, bg = 8.0f + 18.0f * glow, bb = 5.0f + 7.0f * glow;
        uint8_t *o = s->wave + (size_t)gy * gw * 3;
        for (int gx = 0; gx < gw; gx++) {
            float light = 0.0f;
            for (int r = 0; r < RIBBONS; r++) {
                float d = ((float)gy - s->curve[r * gw + gx]);
                float dl = d * line_s, db = d * band_s;
                float band = d < 0.0f ? 0.09f : 0.04f;
                light += 0.26f / (1.0f + dl * dl) + band / (1.0f + db * db);
            }
            float rr = br + 116.0f * light, gg = bg + 89.0f * light, bl = bb + 44.0f * light;
            o[gx * 3] = (uint8_t)(bl > 255.0f ? 255 : (int)bl);
            o[gx * 3 + 1] = (uint8_t)(gg > 255.0f ? 255 : (int)gg);
            o[gx * 3 + 2] = (uint8_t)(rr > 255.0f ? 255 : (int)rr);
        }
    }
    // scale the grid up once (bilinear), wrapping at the right edge
    for (int y = 0; y < s->h; y++) {
        float gyf = ((float)y + 0.5f) / D - 0.5f;
        if (gyf < 0.0f) gyf = 0.0f;
        int gy = (int)gyf;
        uint32_t fy = (uint32_t)((gyf - (float)gy) * 256.0f);
        const uint8_t *ra = s->wave + (size_t)gy * gw * 3, *rb = ra + gw * 3;
        uint8_t *d = s->full + (size_t)y * W * 4;
        for (int x = 0; x < W; x++) {
            int i = s->xs[x * 2] * 3;
            uint32_t fx = (uint32_t)s->xs[x * 2 + 1];
            for (int c = 0; c < 3; c++) {
                uint32_t top = ra[i + c] * (256 - fx) + ra[i + 3 + c] * fx;
                uint32_t bot = rb[i + c] * (256 - fx) + rb[i + 3 + c] * fx;
                d[x * 4 + c] = (uint8_t)((top * (256 - fy) + bot * fy) >> 16);
            }
            d[x * 4 + 3] = 255;
        }
    }
}

// The wave drifts sideways: t seconds -> a column offset.
void tvl_advance(TvlScene *s, float t) {
    if (!s || !s->full) return;
    float px = t * 14.0f * s->sc; // 14 logical px a second: calm
    s->offset = (int32_t)px % s->w;
}

// A rounded box's span on one row (horizontal anti-aliasing at the ends,
// the corners by their circle): x0..x1 and the vertical coverage.
static inline bool box_span(const Box &b, float py, float &x0, float &x1, float &vcov) {
    if (py < b.y - 0.5f || py > b.y + b.h + 0.5f) return false;
    vcov = clamp01(py - b.y + 0.5f) * clamp01(b.y + b.h - py + 0.5f);
    float r = minf(b.r, minf(b.w, b.h) * 0.5f), e = 0.0f;
    float dy = py < b.y + r ? b.y + r - py : (py > b.y + b.h - r ? py - (b.y + b.h - r) : 0.0f);
    if (dy > 0.0f) {
        float q = r * r - dy * dy;
        e = q > 0.0f ? r - sqrtf_(q) : r;
    }
    x0 = b.x + e;
    x1 = b.x + b.w - e;
    return x1 > x0;
}

// A straight colour over a span of the row, ends anti-aliased.
static inline void fill_span(uint8_t *d, int W, float x0, float x1, uint32_t r, uint32_t g, uint32_t b, float a) {
    int xa = maxi(0, floori(x0)), xb = mini(W, ceili(x1));
    uint32_t af = (uint32_t)(a * 255.0f + 0.5f);
    if (af == 0) return;
    uint32_t pr = r * af, pg = g * af, pb = b * af, inv = 255 - af;
    for (int x = xa; x < xb; x++) {
        float cov = clamp01(minf((float)x + 1.0f - x0, x1 - (float)x));
        uint8_t *o = d + x * 4;
        if (cov >= 1.0f) {
            o[0] = (uint8_t)div255(pb + o[0] * inv);
            o[1] = (uint8_t)div255(pg + o[1] * inv);
            o[2] = (uint8_t)div255(pr + o[2] * inv);
            o[3] = (uint8_t)(af + div255(o[3] * inv));
        } else {
            over(o, r, g, b, (uint32_t)(af * cov));
        }
    }
}

// A soft glow ring around a box (only the pixels near it are evaluated).
static void glow_ring(uint8_t *d, int W, const Box &b, float py, float reach, uint32_t r, uint32_t g, uint32_t bl,
                      float strength, float pw) {
    if (py < b.y - reach || py > b.y + b.h + reach) return;
    int xa = maxi(0, floori(b.x - reach)), xb = mini(W, ceili(b.x + b.w + reach));
    bool inside_rows = py > b.y + b.r && py < b.y + b.h - b.r;
    for (int x = xa; x < xb; x++) {
        // inside the straight middle of the box nothing to do: jump over it
        if (inside_rows && x >= ceili(b.x) && x < floori(b.x + b.w)) {
            x = floori(b.x + b.w) - 1;
            continue;
        }
        float sd = sd_box((float)x + 0.5f, py, b.x, b.y, b.w, b.h, b.r);
        if (sd <= 0.0f || sd >= reach) continue;
        float k = 1.0f - sd / reach;
        float a = strength * (pw == 3.0f ? k * k * k : k * k);
        over(d + x * 4, r, g, bl, (uint32_t)(a * 255.0f));
    }
}

// Composes rows [y0, y1) of the whole frame; dst points at row y0.
// focus: x, y, w, h, r (logical) or null.
void tvl_render_rows(const TvlScene *s, uint8_t *dst, int32_t stride, int32_t y0, int32_t y1, const float *focus) {
    if (!s || !dst || !s->full) return;
    const int W = s->w;
    y0 = maxi(0, y0);
    y1 = mini(s->h, y1);
    Box F{};
    bool has_focus = focus != nullptr;
    if (has_focus) F = Box{focus[0] * s->sc, focus[1] * s->sc, focus[2] * s->sc, focus[3] * s->sc, focus[4] * s->sc};
    const int o = s->offset;
    for (int y = y0; y < y1; y++) {
        uint8_t *__restrict d = dst + (ptrdiff_t)stride * (y - y0);
        float py = (float)y + 0.5f;
        // 1. the wave: two copies, the picture moved sideways by `o`
        const uint8_t *src = s->full + (size_t)y * W * 4;
        __builtin_memcpy(d, src + (size_t)o * 4, (size_t)(W - o) * 4);
        __builtin_memcpy(d + (size_t)(W - o) * 4, src, (size_t)o * 4);
        // 2. the glass rows (as in Settings; the playing channel's is warmer)
        for (int n = 0; n < s->n_rows; n++) {
            float x0, x1, v;
            if (!box_span(s->rows[n], py, x0, x1, v)) continue;
            bool live = s->row_flags[n] != 0;
            fill_span(d, W, x0, x1, live ? 46 : 9, live ? 36 : 9, live ? 18 : 8, 0.275f * v);
        }
        // 3. the focus: warm gold glass with a soft glow, no outline
        if (has_focus) {
            glow_ring(d, W, F, py, 12.0f * s->sc, 214, 160, 70, 0.235f, 2.0f);
            float x0, x1, v;
            if (box_span(F, py, x0, x1, v)) {
                float k = clamp01((py - F.y) / F.h);
                fill_span(d, W, x0, x1, (uint32_t)(140.0f - 40.0f * k), (uint32_t)(104.0f - 30.0f * k),
                          (uint32_t)(44.0f - 14.0f * k), 0.59f * v);
            }
        }
        // 4. the video window: transparent, a soft dark rim and gold glow
        if (s->has_hole) {
            const Box &Hb = s->hole;
            glow_ring(d, W, Hb, py, 18.0f * s->sc, 0, 0, 0, 0.47f, 2.0f);
            glow_ring(d, W, Hb, py, 18.0f * s->sc, 222, 172, 84, 0.16f, 3.0f);
            float x0, x1, v;
            if (box_span(Hb, py, x0, x1, v)) {
                int xa = maxi(0, ceili(x0)), xb = mini(W, floori(x1));
                if (v >= 1.0f && xb > xa) __builtin_memset(d + xa * 4, 0, (size_t)(xb - xa) * 4);
                // the anti-aliased ends (and the partial top/bottom row)
                for (int x = maxi(0, floori(x0)); x < mini(W, ceili(x1)); x++) {
                    if (v >= 1.0f && x >= xa && x < xb) continue;
                    float cov = clamp01(minf((float)x + 1.0f - x0, x1 - (float)x)) * v;
                    uint32_t keep = (uint32_t)((1.0f - cov) * 256.0f);
                    uint8_t *p = d + x * 4;
                    for (int c = 0; c < 4; c++) p[c] = (uint8_t)((p[c] * keep) >> 8);
                }
            }
        }
        // 5. the text over everything, only where it has pixels
        if (s->text) {
            int a0 = s->text_span[y * 2], a1 = s->text_span[y * 2 + 1];
            const uint8_t *__restrict t = s->text + (ptrdiff_t)s->text_stride * y;
            for (int i = a0 * 4; i < a1 * 4; i += 4) {
                uint32_t inv = 256 - t[i + 3];
                d[i] = (uint8_t)(t[i] + ((d[i] * inv) >> 8));
                d[i + 1] = (uint8_t)(t[i + 1] + ((d[i + 1] * inv) >> 8));
                d[i + 2] = (uint8_t)(t[i + 2] + ((d[i + 2] * inv) >> 8));
                d[i + 3] = (uint8_t)(t[i + 3] + ((d[i + 3] * inv) >> 8));
            }
        }
    }
    (void)PI;
}

} // extern "C"
