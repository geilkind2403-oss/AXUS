// Home's liquid gold focus frame in one pass: wash, ring, outer glow and the
// light on its top edge, blended straight into the BGRA premultiplied frame.
// Rows are independent, so Rust shares them between the Pi's four cores.
//
// No system headers (the Pi cross build would pick up the host's libc).
typedef __UINT8_TYPE__ uint8_t;
typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;

struct FocusParams {
    float scale;              // physical px per logical px
    float x, y, w, h, r;      // the frame (logical)
    float reach;              // how far the glow reaches outside (logical)
    float wash, ring, glow;   // gold alphas 0..1
    float top;                // light on the top edge 0..1
    float ring_w;             // ring width (logical)
};

namespace {
inline float clampf(float v, float a, float b) { return v < a ? a : (v > b ? b : v); }
inline float absf(float v) { return v < 0.0f ? -v : v; }
inline float maxf(float a, float b) { return a > b ? a : b; }
inline int32_t floori(float v) { int32_t i = (int32_t)v; return (float)i > v ? i - 1 : i; }
inline int32_t ceili(float v) { int32_t i = (int32_t)v; return (float)i < v ? i + 1 : i; }

// gold and the warm light, BGRA order
constexpr float GB = 116.0f, GG = 187.0f, GR = 226.0f;
constexpr float LB = 205.0f, LG = 240.0f, LR = 255.0f;

inline void blend(uint8_t* o, float ag, float al) {
    float k = 1.0f - ag;
    float b = GB * ag + o[0] * k;
    float g = GG * ag + o[1] * k;
    float r = GR * ag + o[2] * k;
    float a = 255.0f * ag + o[3] * k;
    if (al > 0.0f) {
        const float k2 = 1.0f - al;
        b = LB * al + b * k2;
        g = LG * al + g * k2;
        r = LR * al + r * k2;
        a = 255.0f * al + a * k2;
    }
    o[0] = (uint8_t)(b + 0.5f);
    o[1] = (uint8_t)(g + 0.5f);
    o[2] = (uint8_t)(r + 0.5f);
    o[3] = (uint8_t)(a + 0.5f);
}
}  // namespace

extern "C" void ffx_draw_rows(uint8_t* px, int32_t stride, int32_t width, int32_t height,
                              int32_t row0, int32_t row1, const FocusParams* p) {
    const float s = p->scale;
    if (s <= 0.0f || p->w <= 0.0f || p->h <= 0.0f) return;
    const float reach = maxf(p->reach, 0.5f);
    int32_t x0 = floori((p->x - reach) * s), x1 = ceili((p->x + p->w + reach) * s);
    int32_t y0 = floori((p->y - reach) * s), y1 = ceili((p->y + p->h + reach) * s);
    if (x0 < 0) x0 = 0;
    if (x1 > width) x1 = width;
    if (y0 < row0) y0 = row0;
    if (y1 > row1) y1 = row1;
    if (y1 > height) y1 = height;
    const float cx = p->x + p->w * 0.5f, cy = p->y + p->h * 0.5f;
    const float r = clampf(p->r, 0.0f, (p->w < p->h ? p->w : p->h) * 0.5f);
    const float hx = p->w * 0.5f - r, hy = p->h * 0.5f - r;
    const float half = p->ring_w * 0.5f;
    // deeper inside than this only the wash shows
    const float margin = p->ring_w + 2.0f / s + 1.0f;

    // gold (wash, ring, glow) at distance d from the edge, and the light on
    // the top edge for a row at dy below the frame's top
    auto gold = [&](float d) {
        float ag = p->wash * clampf(0.5f - d * s, 0.0f, 1.0f);
        const float rd = absf(d + half) - half;
        const float ring = p->ring * clampf(0.5f - rd * s, 0.0f, 1.0f);
        ag = ag + ring - ag * ring;
        if (d > 0.0f) {
            const float t = 1.0f - d / reach;
            if (t > 0.0f) {
                const float g = p->glow * t * t;
                ag = ag + g - ag * g;
            }
        }
        return clampf(ag, 0.0f, 1.0f);
    };
    auto light = [&](float dy) {
        if (p->top <= 0.0f || dy <= 0.5f || dy >= 3.0f) return 0.0f;
        return clampf(p->top * (1.0f - absf(dy - 1.75f) / 1.25f), 0.0f, 1.0f);
    };

    for (int32_t py = y0; py < y1; ++py) {
        const float ly = (py + 0.5f) / s;
        const float qy = absf(ly - cy) - hy;
        uint8_t* row = px + (long)py * stride;
        // One stretch of this row has the same colour all along: between the
        // corners of the top and bottom bands, or the inside of the middle
        // rows. It is blended as one constant (src + dst * k, no division).
        float span_ag, span_al = 0.0f, inner;
        if (qy > 0.0f) {
            inner = hx;
            span_ag = gold(qy - r);
            span_al = light(ly - p->y);
        } else {
            inner = hx + r - margin;
            span_ag = p->wash;
        }
        int32_t fa = x1, fb = x1;
        if (inner > 0.0f) {
            fa = ceili((cx - inner) * s - 0.5f);
            fb = floori((cx + inner) * s - 0.5f) + 1;
            if (fa < x0) fa = x0;
            if (fb > x1) fb = x1;
            if (fa >= fb) fa = fb = x1;
        }
        if (fa < fb && (span_ag > 0.002f || span_al > 0.002f)) {
            const float k = (1.0f - span_ag) * (1.0f - span_al);
            const uint32_t k8 = (uint32_t)(k * 256.0f + 0.5f);
            const uint32_t sb = (uint32_t)(LB * span_al + GB * span_ag * (1.0f - span_al) + 0.5f);
            const uint32_t sg = (uint32_t)(LG * span_al + GG * span_ag * (1.0f - span_al) + 0.5f);
            const uint32_t sr = (uint32_t)(LR * span_al + GR * span_ag * (1.0f - span_al) + 0.5f);
            const uint32_t sa = (uint32_t)(255.0f * (1.0f - k) + 0.5f);
            uint8_t* o = row + fa * 4;
            for (int32_t i = fa; i < fb; ++i, o += 4) {
                uint32_t b = sb + ((o[0] * k8) >> 8);
                uint32_t g = sg + ((o[1] * k8) >> 8);
                uint32_t rr = sr + ((o[2] * k8) >> 8);
                uint32_t a = sa + ((o[3] * k8) >> 8);
                o[0] = (uint8_t)(b > 255 ? 255 : b);
                o[1] = (uint8_t)(g > 255 ? 255 : g);
                o[2] = (uint8_t)(rr > 255 ? 255 : rr);
                o[3] = (uint8_t)(a > 255 ? 255 : a);
            }
        }
        // the rest of the row: corners and the frame's sides, per pixel
        for (int32_t pxi = x0; pxi < x1; ++pxi) {
            if (pxi == fa) {
                pxi = fb - 1;
                continue;
            }
            const float lx = (pxi + 0.5f) / s;
            const float qx = absf(lx - cx) - hx;
            const float d = (qx > 0.0f && qy > 0.0f)
                                ? __builtin_sqrtf(qx * qx + qy * qy) - r
                                : maxf(qx, qy) - r;
            const float ag = gold(d);
            const float al = (lx > p->x + r && lx < p->x + p->w - r) ? light(ly - p->y) : 0.0f;
            if (ag <= 0.002f && al <= 0.002f) continue;
            blend(row + pxi * 4, ag, al);
        }
    }
}
