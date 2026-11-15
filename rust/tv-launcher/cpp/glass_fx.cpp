// glass_fx -- the fast menu as liquid glass. See glass_fx.h.
//
// Cost model (Raspberry Pi 3, 1080p: the region is 585 x 705 pixels):
//  * glass_build, when the menu opens or Home behind it changes (once a
//    minute for the clock): a blur on a 4x smaller copy, one pass over the
//    region for refraction, rim, shadow and tiles.
//  * a frame: only the rectangle the highlight moved through -- a row copy
//    of the still glass, the highlight's rounded box, the text over it.
#include "glass_fx.h"

extern "C" void *malloc(size_t);
extern "C" void free(void *);

namespace {

inline float absf(float x) { return x < 0.0f ? -x : x; }
inline float minf(float a, float b) { return a < b ? a : b; }
inline float maxf(float a, float b) { return a > b ? a : b; }
inline int mini(int a, int b) { return a < b ? a : b; }
inline int maxi(int a, int b) { return a > b ? a : b; }
inline float clamp01(float x) { return x < 0.0f ? 0.0f : (x > 1.0f ? 1.0f : x); }
inline float lerp(float a, float b, float t) { return a + (b - a) * t; }
inline float sqrtf_(float x) { return __builtin_sqrtf(x); }
inline int floori(float x) {
    int i = (int)x;
    return x < (float)i ? i - 1 : i;
}
inline int ceili(float x) {
    int i = (int)x;
    return x > (float)i ? i + 1 : i;
}
inline uint32_t div255(uint32_t x) {
    x += 128;
    return (x + (x >> 8)) >> 8;
}
inline uint8_t u8(float v) { return (uint8_t)(v <= 0.0f ? 0 : (v >= 255.0f ? 255 : (int)(v + 0.5f))); }

// Rounded box, negative inside (physical px).
inline float sd_box(float px, float py, float x, float y, float w, float h, float r) {
    r = minf(r, minf(w, h) * 0.5f);
    float cx = x + w * 0.5f, cy = y + h * 0.5f;
    float qx = absf(px - cx) - (w * 0.5f - r), qy = absf(py - cy) - (h * 0.5f - r);
    float ox = maxf(qx, 0.0f), oy = maxf(qy, 0.0f);
    return sqrtf_(ox * ox + oy * oy) + minf(maxf(qx, qy), 0.0f) - r;
}

// A premultiplied colour accumulated in floats (build time only).
struct Px {
    float b, g, r, a;
};
inline void over(Px &d, float r, float g, float b, float a) {
    float k = 1.0f - a;
    d.b = b * a + d.b * k;
    d.g = g * a + d.g * k;
    d.r = r * a + d.r * k;
    d.a = a * 255.0f + d.a * k;
}

const int MAX_TILES = 16;

} // namespace

struct GlassScene {
    GlassParams p;
    GlassRect panel; // physical
    GlassRect tiles[MAX_TILES];
    int n_tiles;
    uint8_t *backdrop; // w*h*4, or null (video)
    uint8_t *content;  // w*h*4
    int32_t *content_span; // per row: first, last+1 visible column
    uint8_t *glass;    // the still glass, w*h*4
    float *small;      // the blurred backdrop, 4x smaller
    int bw, bh;
    bool built;
};

namespace {

GlassRect phys(const GlassRect &r, float sc) { return GlassRect{r.x * sc, r.y * sc, r.w * sc, r.h * sc, r.r * sc, r.kind}; }

// Box blur of a w x h x 4 float image, radius r, along x then y.
void box_blur(float *img, float *tmp, int w, int h, int r) {
    float inv = 1.0f / (float)(2 * r + 1);
    for (int y = 0; y < h; y++) {
        float *row = img + (size_t)y * w * 4, *out = tmp + (size_t)y * w * 4;
        for (int c = 0; c < 4; c++) {
            float acc = 0.0f;
            for (int i = -r; i <= r; i++) acc += row[maxi(0, mini(w - 1, i)) * 4 + c];
            for (int x = 0; x < w; x++) {
                out[x * 4 + c] = acc * inv;
                acc += row[mini(w - 1, x + r + 1) * 4 + c] - row[maxi(0, x - r) * 4 + c];
            }
        }
    }
    for (int x = 0; x < w; x++)
        for (int c = 0; c < 4; c++) {
            float acc = 0.0f;
            for (int i = -r; i <= r; i++) acc += tmp[((size_t)maxi(0, mini(h - 1, i)) * w + x) * 4 + c];
            for (int y = 0; y < h; y++) {
                img[((size_t)y * w + x) * 4 + c] = acc * inv;
                acc += tmp[((size_t)mini(h - 1, y + r + 1) * w + x) * 4 + c] -
                       tmp[((size_t)maxi(0, y - r) * w + x) * 4 + c];
            }
        }
}

inline Px sample(const float *img, int w, int h, float x, float y) {
    x = maxf(0.0f, minf((float)w - 1.001f, x));
    y = maxf(0.0f, minf((float)h - 1.001f, y));
    int ix = (int)x, iy = (int)y;
    float fx = x - (float)ix, fy = y - (float)iy;
    const float *a = img + ((size_t)iy * w + ix) * 4, *b = a + (size_t)w * 4;
    Px o;
    float *op = &o.b;
    for (int c = 0; c < 4; c++) op[c] = lerp(lerp(a[c], a[c + 4], fx), lerp(b[c], b[c + 4], fx), fy);
    return o;
}

} // namespace

extern "C" GlassScene *glass_create(const GlassParams *p) {
    if (!p || p->width <= 0 || p->height <= 0 || p->scale <= 0.0f) return nullptr;
    GlassScene *s = (GlassScene *)malloc(sizeof(GlassScene));
    if (!s) return nullptr;
    __builtin_memset(s, 0, sizeof(GlassScene));
    s->p = *p;
    s->panel = phys(p->panel, p->scale);
    size_t n = (size_t)p->width * p->height * 4;
    s->content = (uint8_t *)malloc(n);
    s->glass = (uint8_t *)malloc(n);
    s->content_span = (int32_t *)malloc(sizeof(int32_t) * 2 * (size_t)p->height);
    if (!s->content || !s->glass || !s->content_span) {
        glass_destroy(s);
        return nullptr;
    }
    __builtin_memset(s->content, 0, n);
    for (int y = 0; y < p->height; y++) {
        s->content_span[y * 2] = 0;
        s->content_span[y * 2 + 1] = 0;
    }
    return s;
}

extern "C" void glass_destroy(GlassScene *s) {
    if (!s) return;
    free(s->backdrop);
    free(s->content);
    free(s->content_span);
    free(s->glass);
    free(s->small);
    free(s);
}

extern "C" int32_t glass_set_backdrop(GlassScene *s, const uint8_t *px, int32_t stride) {
    if (!s) return 0;
    s->built = false;
    if (!px || s->p.over_video) {
        free(s->backdrop);
        s->backdrop = nullptr;
        return 1;
    }
    size_t row = (size_t)s->p.width * 4;
    if (!s->backdrop) s->backdrop = (uint8_t *)malloc(row * s->p.height);
    if (!s->backdrop) return 0;
    for (int y = 0; y < s->p.height; y++) __builtin_memcpy(s->backdrop + row * y, px + (ptrdiff_t)stride * y, row);
    return 1;
}

extern "C" int32_t glass_set_tiles(GlassScene *s, const GlassRect *tiles, int32_t n) {
    if (!s || n < 0) return 0;
    s->built = false;
    s->n_tiles = mini(n, MAX_TILES);
    for (int i = 0; i < s->n_tiles; i++) s->tiles[i] = phys(tiles[i], s->p.scale);
    return 1;
}

extern "C" int32_t glass_set_content(GlassScene *s, const uint8_t *px, int32_t stride) {
    if (!s || !px) return 0;
    size_t row = (size_t)s->p.width * 4;
    for (int y = 0; y < s->p.height; y++) {
        const uint8_t *src = px + (ptrdiff_t)stride * y;
        __builtin_memcpy(s->content + row * y, src, row);
        int a = 0, b = s->p.width;
        while (a < b && src[a * 4 + 3] == 0) a++;
        while (b > a && src[(b - 1) * 4 + 3] == 0) b--;
        s->content_span[y * 2] = a;
        s->content_span[y * 2 + 1] = b;
    }
    return 1;
}

extern "C" int32_t glass_build_begin(GlassScene *s) {
    if (!s) return 0;
    s->built = false;
    const int W = s->p.width, H = s->p.height;
    const bool video = s->backdrop == nullptr;
    free(s->small);
    s->small = nullptr;
    // Home behind the panel, 4x smaller and blurred (three box passes:
    // nearly Gaussian); the rows sample it back with refraction.
    const int D = 4;
    int bw = (W + D - 1) / D, bh = (H + D - 1) / D;
    s->bw = bw;
    s->bh = bh;
    if (!video) {
        float *small = (float *)malloc(sizeof(float) * 4 * (size_t)bw * bh);
        float *tmp = (float *)malloc(sizeof(float) * 4 * (size_t)bw * bh);
        if (!small || !tmp) {
            free(small);
            free(tmp);
            return 0;
        }
        for (int by = 0; by < bh; by++)
            for (int bx = 0; bx < bw; bx++) {
                float acc[4] = {0, 0, 0, 0};
                int n = 0;
                for (int y = by * D; y < mini(H, by * D + D); y++)
                    for (int x = bx * D; x < mini(W, bx * D + D); x++, n++)
                        for (int c = 0; c < 4; c++) acc[c] += s->backdrop[((size_t)y * W + x) * 4 + c];
                for (int c = 0; c < 4; c++) small[((size_t)by * bw + bx) * 4 + c] = acc[c] / (float)n;
            }
        for (int i = 0; i < 3; i++) box_blur(small, tmp, bw, bh, 2);
        free(tmp);
        s->small = small;
    }
    return 1;
}

extern "C" void glass_build_rows(const GlassScene *s, int32_t y0, int32_t y1) {
    if (!s) return;
    const int W = s->p.width, H = s->p.height;
    const float sc = s->p.scale;
    const bool video = s->backdrop == nullptr;
    if (!video && !s->small) return;
    const int D = 4;
    const int bw = s->bw, bh = s->bh;
    const float *small = s->small;
    y0 = maxi(0, y0);
    y1 = mini(H, y1);
    const GlassRect &P = s->panel;
    const float edge = 22.0f * sc;   // the lens zone along the rim
    const float bend = 16.0f * sc;   // how far the picture bends there
    const float shadow = 26.0f * sc; // soft shadow reach
    const float lx = -0.5f, ly = -0.866f; // light from the upper left

    const float reach = shadow + 10.0f * sc;
    for (int y = y0; y < y1; y++) {
        float py = (float)y + 0.5f;
        // the tiles on this row
        int row_tiles[MAX_TILES], nt = 0;
        for (int i = 0; i < s->n_tiles; i++)
            if (py >= s->tiles[i].y - 2 && py <= s->tiles[i].y + s->tiles[i].h + 2) row_tiles[nt++] = i;
        // far from the panel the row is Home itself (one copy)
        int xa = maxi(0, floori(P.x - reach)), xb = mini(W, ceili(P.x + P.w + reach));
        uint8_t *orow = s->glass + (size_t)y * W * 4;
        if (py < P.y - reach || py > P.y + P.h + reach) xa = xb = 0;
        if (video) {
            __builtin_memset(orow, 0, (size_t)xa * 4);
            __builtin_memset(orow + xb * 4, 0, (size_t)(W - xb) * 4);
            if (xa == xb) __builtin_memset(orow, 0, (size_t)W * 4);
        } else {
            const uint8_t *brow = s->backdrop + (size_t)y * W * 4;
            __builtin_memcpy(orow, brow, (size_t)xa * 4);
            __builtin_memcpy(orow + xb * 4, brow + xb * 4, (size_t)(W - xb) * 4);
            if (xa == xb) __builtin_memcpy(orow, brow, (size_t)W * 4);
        }
        for (int x = xa; x < xb; x++) {
            float px = (float)x + 0.5f;
            uint8_t *o = s->glass + ((size_t)y * W + x) * 4;
            Px d;
            if (video) {
                d = Px{0, 0, 0, 0};
            } else {
                const uint8_t *b = s->backdrop + ((size_t)y * W + x) * 4;
                d = Px{(float)b[0], (float)b[1], (float)b[2], (float)b[3]};
            }
            float sd = sd_box(px, py, P.x, P.y, P.w, P.h, P.r);
            // shadow below the floating panel
            if (sd > -1.0f) {
                float ss = sd_box(px, py - 9.0f * sc, P.x, P.y, P.w, P.h, P.r);
                float k = clamp01(1.0f - ss / shadow);
                float a = k * k * (video ? 0.5f : 0.34f) * clamp01(sd + 0.5f);
                over(d, 0, 2, 8, a);
            }
            float cov = clamp01(0.5f - sd);
            if (cov <= 0.0f) {
                o[0] = u8(d.b), o[1] = u8(d.g), o[2] = u8(d.r), o[3] = u8(d.a);
                continue;
            }
            float depth = -sd; // px inside the rim
            // outward normal of the rim (only the lens zone needs it)
            float nx = 0.0f, ny = 0.0f;
            const bool near = depth < edge;
            if (near) {
                float e = 0.75f;
                nx = sd_box(px + e, py, P.x, P.y, P.w, P.h, P.r) - sd_box(px - e, py, P.x, P.y, P.w, P.h, P.r);
                ny = sd_box(px, py + e, P.x, P.y, P.w, P.h, P.r) - sd_box(px, py - e, P.x, P.y, P.w, P.h, P.r);
                float nl = sqrtf_(nx * nx + ny * ny);
                if (nl > 0.0f) nx /= nl, ny /= nl;
            }
            Px g;
            if (video) {
                g = Px{34.0f * 0.84f, 24.0f * 0.84f, 18.0f * 0.84f, 0.84f * 255.0f};
            } else {
                // the lens: near the rim the picture is pulled in from the
                // middle (magnified, bent); red and blue part a little
                float t = clamp01(1.0f - depth / edge);
                float disp = t * t * bend;
                float sx = px / D - 0.5f, sy = py / D - 0.5f, dd = disp / D;
                Px m = sample(small, bw, bh, sx - nx * dd, sy - ny * dd);
                if (near) {
                    Px rr = sample(small, bw, bh, sx - nx * dd * 1.25f, sy - ny * dd * 1.25f);
                    Px bb = sample(small, bw, bh, sx - nx * dd * 0.78f, sy - ny * dd * 0.78f);
                    g = Px{bb.b, m.g, rr.r, 255.0f};
                } else {
                    g = Px{m.b, m.g, m.r, 255.0f};
                }
                // livelier colour, then a dark tint for legible text
                float lum = g.r * 0.299f + g.g * 0.587f + g.b * 0.114f;
                g.r = lum + (g.r - lum) * 1.35f;
                g.g = lum + (g.g - lum) * 1.35f;
                g.b = lum + (g.b - lum) * 1.35f;
                over(g, 18, 24, 38, 0.52f);
            }
            // a soft inner light along the whole edge, a bright rim where the
            // light falls on it and a faint one opposite
            float glowk = clamp01(1.0f - depth / (18.0f * sc));
            over(g, 255, 255, 255, glowk * glowk * 0.07f);
            float rim = clamp01(1.0f - absf(depth - 1.1f * sc) / (1.3f * sc));
            float lit = nx * lx + ny * ly;
            float ra = rim * (0.16f + 0.62f * maxf(0.0f, lit) * maxf(0.0f, lit) + 0.22f * maxf(0.0f, -lit));
            over(g, 255, 255, 255, ra);
            // a sheen over the upper part, like light on a curved surface
            float top = clamp01(1.0f - (py - P.y) / (P.h * 0.42f));
            over(g, 255, 255, 255, top * top * 0.045f);

            // glass tiles inside
            for (int ti = 0; ti < nt; ti++) {
                const GlassRect &T = s->tiles[row_tiles[ti]];
                if (px < T.x - 2 || px > T.x + T.w + 2 || py < T.y - 2 || py > T.y + T.h + 2) continue;
                float ts = sd_box(px, py, T.x, T.y, T.w, T.h, T.r);
                float tc = clamp01(0.5f - ts);
                if (tc <= 0.0f) continue;
                if (T.kind == GLASS_INSET)
                    over(g, 0, 4, 14, 0.26f * tc);
                else if (T.kind == GLASS_TILE_ACTIVE)
                    over(g, 74, 136, 255, 0.30f * tc);
                else
                    over(g, 255, 255, 255, 0.075f * tc);
                // the tile's own rim: lit on top, a shade below
                float tr = clamp01(1.0f - absf(-ts - 0.8f * sc) / (1.0f * sc)) * tc;
                float v = clamp01((py - T.y) / T.h);
                if (T.kind == GLASS_INSET)
                    over(g, 0, 0, 0, tr * 0.18f);
                else
                    over(g, 255, 255, 255, tr * lerp(0.42f, 0.08f, v));
            }

            // anti-aliased edge of the panel over what is behind it
            Px out;
            out.b = g.b * cov + d.b * (1.0f - cov * g.a / 255.0f);
            out.g = g.g * cov + d.g * (1.0f - cov * g.a / 255.0f);
            out.r = g.r * cov + d.r * (1.0f - cov * g.a / 255.0f);
            out.a = g.a * cov + d.a * (1.0f - cov * g.a / 255.0f);
            o[0] = u8(out.b), o[1] = u8(out.g), o[2] = u8(out.r), o[3] = u8(out.a);
        }
    }
}

extern "C" int32_t glass_build_end(GlassScene *s) {
    if (!s || (!s->backdrop && !s->p.over_video) || (s->backdrop && !s->small)) return 0;
    s->built = true;
    return 1;
}

extern "C" int32_t glass_build(GlassScene *s) {
    if (!glass_build_begin(s)) return 0;
    glass_build_rows(s, 0, s->p.height);
    return glass_build_end(s);
}

extern "C" void glass_render(const GlassScene *s, uint8_t *dst, int32_t stride, const GlassRect *focus, float opacity,
                             float fx0, float fy0, float fx1, float fy1) {
    if (!s || !dst || !s->built) return;
    const int W = s->p.width, H = s->p.height;
    const float sc = s->p.scale;
    int x0 = maxi(0, floori(fx0 * sc)), y0 = maxi(0, floori(fy0 * sc));
    int x1 = mini(W, ceili(fx1 * sc)), y1 = mini(H, ceili(fy1 * sc));
    if (x0 >= x1 || y0 >= y1) return;
    GlassRect F{};
    bool has_focus = focus != nullptr;
    if (has_focus) F = phys(*focus, sc);
    const float glow = 12.0f * sc;
    int fx0i = 0, fy0i = 0, fx1i = 0, fy1i = 0;
    if (has_focus) {
        fx0i = maxi(x0, floori(F.x - glow));
        fy0i = maxi(y0, floori(F.y - glow));
        fx1i = mini(x1, ceili(F.x + F.w + glow));
        fy1i = mini(y1, ceili(F.y + F.h + glow));
    }
    const uint32_t k = (uint32_t)(clamp01(opacity) * 256.0f + 0.5f);
    for (int y = y0; y < y1; y++) {
        uint8_t *__restrict d = dst + (ptrdiff_t)stride * y;
        const uint8_t *__restrict g = s->glass + ((size_t)y * W) * 4;
        // 1. the still glass
        __builtin_memcpy(d + x0 * 4, g + x0 * 4, (size_t)(x1 - x0) * 4);
        // 2. the liquid highlight: a lit glass drop over the focused tile
        if (has_focus && y >= fy0i && y < fy1i) {
            float py = (float)y + 0.5f;
            float v = clamp01((py - F.y) / F.h);
            // colour of the drop, top to bottom (straight RGB, alpha)
            float cr = lerp(140.0f, 74.0f, v), cg = lerp(186.0f, 124.0f, v), cb = 255.0f;
            float fa = lerp(0.50f, 0.40f, v);
            float sheen = clamp01(1.0f - v * 2.2f) * 0.22f;
            float rim_a = lerp(0.95f, 0.5f, v);
            for (int x = fx0i; x < fx1i; x++) {
                float px = (float)x + 0.5f;
                float sd = sd_box(px, py, F.x, F.y, F.w, F.h, F.r);
                uint8_t *o = d + x * 4;
                float a, r, gg, b;
                if (sd > 0.0f) {
                    // the glow around the drop
                    float t = clamp01(1.0f - sd / glow);
                    a = t * t * 0.34f * clamp01(sd + 0.5f);
                    r = 96, gg = 150, b = 255;
                    if (a > 0.0f) {
                        uint32_t ai = (uint32_t)(a * 255.0f);
                        uint32_t inv = 255 - ai;
                        o[0] = (uint8_t)div255((uint32_t)b * ai + o[0] * inv);
                        o[1] = (uint8_t)div255((uint32_t)gg * ai + o[1] * inv);
                        o[2] = (uint8_t)div255((uint32_t)r * ai + o[2] * inv);
                        o[3] = (uint8_t)(ai + div255(o[3] * inv));
                    }
                }
                float cov = clamp01(0.5f - sd);
                if (cov <= 0.0f) continue;
                float depth = -sd;
                float rim = clamp01(1.0f - absf(depth - 1.0f * sc) / (1.3f * sc));
                // fill, then sheen and rim in white (combined, one blend)
                a = fa;
                r = cr, gg = cg, b = cb;
                float wa = maxf(sheen, rim * rim_a);
                r = lerp(r, 255.0f, wa), gg = lerp(gg, 255.0f, wa), b = lerp(b, 255.0f, wa);
                a = a + (1.0f - a) * wa;
                uint32_t ai = (uint32_t)(a * cov * 255.0f + 0.5f);
                uint32_t inv = 255 - ai;
                o[0] = (uint8_t)div255((uint32_t)b * ai + o[0] * inv);
                o[1] = (uint8_t)div255((uint32_t)gg * ai + o[1] * inv);
                o[2] = (uint8_t)div255((uint32_t)r * ai + o[2] * inv);
                o[3] = (uint8_t)(ai + div255(o[3] * inv));
            }
        }
        // 3. text and icons over it, only where they have pixels
        int a0 = maxi(x0, s->content_span[y * 2]), a1 = mini(x1, s->content_span[y * 2 + 1]);
        const uint8_t *__restrict c = s->content + ((size_t)y * W) * 4;
        for (int i = a0 * 4; i < a1 * 4; i += 4) {
            uint32_t inv = 256 - c[i + 3];
            d[i] = (uint8_t)(c[i] + ((d[i] * inv) >> 8));
            d[i + 1] = (uint8_t)(c[i + 1] + ((d[i + 1] * inv) >> 8));
            d[i + 2] = (uint8_t)(c[i + 2] + ((d[i + 2] * inv) >> 8));
            d[i + 3] = (uint8_t)(c[i + 3] + ((d[i + 3] * inv) >> 8));
        }
        // 4. opening: the menu fades in over what is behind it
        if (k < 256) {
            const uint8_t *__restrict b = s->backdrop ? s->backdrop + ((size_t)y * W) * 4 : nullptr;
            for (int i = x0 * 4; i < x1 * 4; i++) {
                uint32_t under = b ? b[i] : 0;
                d[i] = (uint8_t)((d[i] * k + under * (256 - k)) >> 8);
            }
        }
    }
}
