// weather_fx -- the living sky of the Weather app. See weather_fx.h.
//
// Budget (Raspberry Pi 3, Cortex-A53, 1080p): the animated band is about
// 1920 x 530 pixels. Per frame and row the work is one row copy from the
// still sky, a few hundred small particles, pre-rendered cloud sprites and
// one blend of the UI layer over its visible spans only. Everything that
// is expensive per pixel (noise, angles, gradients) is done once, when the
// scene is made.
#include "weather_fx.h"

extern "C" void *malloc(size_t);
extern "C" void free(void *);

namespace {

// ------------------------------------------------------------------ math
const float PI = 3.14159265f;
const float TAU = 6.28318531f;

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
inline float fract(float x) { return x - (float)floori(x); }
inline float smooth(float e0, float e1, float x) {
    float t = clamp01((x - e0) / (e1 - e0));
    return t * t * (3.0f - 2.0f * t);
}
// sine to about 1e-3, enough for motion
inline float sinf_(float x) {
    x *= 1.0f / TAU;
    x -= (float)floori(x + 0.5f);
    x *= TAU;
    float y = 1.27323954f * x - 0.405284735f * x * absf(x);
    return 0.225f * (y * absf(y) - y) + y;
}
inline float cosf_(float x) { return sinf_(x + PI * 0.5f); }
float atan2f_(float y, float x) {
    float ax = absf(x), ay = absf(y);
    float mx = maxf(ax, ay), mn = minf(ax, ay);
    if (mx == 0.0f) return 0.0f;
    float a = mn / mx, s = a * a;
    float r = ((-0.0464964749f * s + 0.15931422f) * s - 0.327622764f) * s * a + a;
    if (ay > ax) r = 1.57079637f - r;
    if (x < 0.0f) r = PI - r;
    if (y < 0.0f) r = -r;
    return r;
}
inline float wrap(float x, float span) { return x - (float)floori(x / span) * span; }

inline uint32_t hash32(uint32_t x) {
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x;
}
inline float hashf(uint32_t x) { return (float)(hash32(x) >> 8) * (1.0f / 16777216.0f); }

float vnoise(float x, float y, uint32_t seed) {
    int xi = floori(x), yi = floori(y);
    float fx = x - (float)xi, fy = y - (float)yi;
    float u = fx * fx * (3.0f - 2.0f * fx), v = fy * fy * (3.0f - 2.0f * fy);
    auto h = [seed](int i, int j) {
        return hashf((uint32_t)i * 73856093u ^ (uint32_t)j * 19349663u ^ seed);
    };
    return lerp(lerp(h(xi, yi), h(xi + 1, yi), u), lerp(h(xi, yi + 1), h(xi + 1, yi + 1), u), v);
}
float fbm(float x, float y, uint32_t seed) {
    float s = 0.0f, a = 0.5f;
    for (int i = 0; i < 5; i++) {
        s += a * vnoise(x, y, seed + (uint32_t)i * 1013u);
        x = x * 2.03f + 17.1f;
        y = y * 2.03f + 9.7f;
        a *= 0.5f;
    }
    return s / 0.96875f;
}

// ---------------------------------------------------------------- pixels
inline uint32_t div255(uint32_t x) {
    x += 128;
    return (x + (x >> 8)) >> 8;
}

struct Rgb {
    float r, g, b;
};
inline Rgb rgb(uint32_t hex) {
    return Rgb{(float)((hex >> 16) & 255), (float)((hex >> 8) & 255), (float)(hex & 255)};
}
inline Rgb mix(Rgb a, Rgb b, float t) { return Rgb{lerp(a.r, b.r, t), lerp(a.g, b.g, t), lerp(a.b, b.b, t)}; }

// Source-over with a straight colour and alpha 0..255 (works on any
// premultiplied destination, transparent or not).
inline void over(uint8_t *d, uint32_t r, uint32_t g, uint32_t b, uint32_t a) {
    if (a == 0) return;
    uint32_t inv = 255 - a;
    d[0] = (uint8_t)div255(b * a + d[0] * inv);
    d[1] = (uint8_t)div255(g * a + d[1] * inv);
    d[2] = (uint8_t)div255(r * a + d[2] * inv);
    d[3] = (uint8_t)(a + div255(d[3] * inv));
}
// Light: added, saturating. Only on the opaque sky.
inline void add(uint8_t *d, uint32_t r, uint32_t g, uint32_t b, uint32_t a) {
    if (a == 0) return;
    uint32_t v;
    v = d[0] + div255(b * a);
    d[0] = (uint8_t)(v > 255 ? 255 : v);
    v = d[1] + div255(g * a);
    d[1] = (uint8_t)(v > 255 ? 255 : v);
    v = d[2] + div255(r * a);
    d[2] = (uint8_t)(v > 255 ? 255 : v);
}

// A run of rows somewhere in a buffer: row0 is row y0.
struct Target {
    uint8_t *row0;
    int32_t stride;
    int32_t w;
    int32_t y0, y1;
    const uint8_t *fade; // per-row strength 0..255 (bottom of the band), or null
    uint32_t gain;       // 0..256: the weather fading in when the screen opens
    inline uint8_t *px(int x, int y) const { return row0 + (ptrdiff_t)(y - y0) * stride + x * 4; }
    inline uint32_t strength(int y, uint32_t a) const { return ((fade ? div255(a * fade[y]) : a) * gain) >> 8; }
};

// A clipped box to paint over; returns false when nothing is left.
inline bool clip(const Target &t, float fx0, float fy0, float fx1, float fy1, int &x0, int &y0, int &x1,
                 int &y1) {
    x0 = maxi(0, floori(fx0));
    y0 = maxi(t.y0, floori(fy0));
    x1 = mini(t.w, ceili(fx1));
    y1 = mini(t.y1, ceili(fy1));
    return x0 < x1 && y0 < y1;
}

// Distance from p to the segment a-b and where along it (0..1).
inline float seg_dist(float px, float py, float ax, float ay, float bx, float by, float &along) {
    float vx = bx - ax, vy = by - ay;
    float wx = px - ax, wy = py - ay;
    float l2 = vx * vx + vy * vy;
    float h = l2 > 0.0f ? clamp01((wx * vx + wy * vy) / l2) : 0.0f;
    along = h;
    float dx = wx - vx * h, dy = wy - vy * h;
    return sqrtf_(dx * dx + dy * dy);
}

// Anti-aliased capsule (a rain streak, a ray of the icon's sun). `tail`
// fades the start of the segment: 0 = even, 1 = from nothing.
void capsule(const Target &t, float ax, float ay, float bx, float by, float width, Rgb c, float alpha,
             float tail, bool light) {
    float hw = width * 0.5f;
    int x0, y0, x1, y1;
    if (!clip(t, minf(ax, bx) - hw - 1, minf(ay, by) - hw - 1, maxf(ax, bx) + hw + 1, maxf(ay, by) + hw + 1, x0,
              y0, x1, y1))
        return;
    uint32_t r = (uint32_t)c.r, g = (uint32_t)c.g, b = (uint32_t)c.b;
    for (int y = y0; y < y1; y++) {
        float py = (float)y + 0.5f;
        for (int x = x0; x < x1; x++) {
            float along;
            float d = seg_dist((float)x + 0.5f, py, ax, ay, bx, by, along);
            float cov = clamp01(hw + 0.5f - d);
            if (cov <= 0.0f) continue;
            float k = alpha * cov * (1.0f - tail + tail * along);
            uint32_t a = t.strength(y, (uint32_t)(k * 255.0f + 0.5f));
            if (light)
                add(t.px(x, y), r, g, b, a);
            else
                over(t.px(x, y), r, g, b, a);
        }
    }
}

// A near-vertical streak (rain), top (x0, y0) to bottom (x1, y1): per row a
// short span with horizontal anti-aliasing; fades in from the top.
void streak(const Target &t, float x0, float y0, float x1, float y1, float width, Rgb c, float alpha, float tail) {
    int ya = maxi(t.y0, floori(y0)), yb = mini(t.y1, ceili(y1));
    if (ya >= yb || y1 <= y0) return;
    float slope = (x1 - x0) / (y1 - y0);
    float hw = width * 0.5f * sqrtf_(1.0f + slope * slope);
    uint32_t r = (uint32_t)c.r, g = (uint32_t)c.g, b = (uint32_t)c.b;
    float inv_len = 1.0f / (y1 - y0);
    for (int y = ya; y < yb; y++) {
        float py = (float)y + 0.5f;
        float along = clamp01((py - y0) * inv_len);
        // round-ish ends: the first and last pixel row are partial
        float cap = clamp01(minf(py - y0, y1 - py) + 0.5f);
        float k = alpha * cap * (1.0f - tail + tail * along) * (float)t.strength(y, 255);
        if (k < 1.0f) continue;
        float cx = x0 + (py - y0) * slope;
        int xa = maxi(0, floori(cx - hw - 0.5f)), xb = mini(t.w, ceili(cx + hw + 0.5f));
        uint8_t *d = t.px(0, y);
        for (int x = xa; x < xb; x++) {
            float cov = clamp01(hw + 0.5f - absf((float)x + 0.5f - cx));
            over(d + x * 4, r, g, b, (uint32_t)(cov * k));
        }
    }
}

// Soft round light: strength (1 - d^2/R^2)^2.
void glow(const Target &t, float cx, float cy, float radius, Rgb c, float alpha, bool light) {
    int x0, y0, x1, y1;
    if (alpha <= 0.0f || !clip(t, cx - radius, cy - radius, cx + radius, cy + radius, x0, y0, x1, y1)) return;
    float inv = 1.0f / (radius * radius);
    uint32_t r = (uint32_t)c.r, g = (uint32_t)c.g, b = (uint32_t)c.b;
    float a255 = alpha * 255.0f;
    for (int y = y0; y < y1; y++) {
        float dy = (float)y + 0.5f - cy;
        float dy2 = dy * dy;
        for (int x = x0; x < x1; x++) {
            float dx = (float)x + 0.5f - cx;
            float f = 1.0f - (dx * dx + dy2) * inv;
            if (f <= 0.0f) continue;
            uint32_t a = t.strength(y, (uint32_t)(a255 * f * f + 0.5f));
            if (light)
                add(t.px(x, y), r, g, b, a);
            else
                over(t.px(x, y), r, g, b, a);
        }
    }
}

// Anti-aliased disc with a soft rim (snowflakes, stars).
void dot(const Target &t, float cx, float cy, float radius, Rgb c, float alpha, float soft) {
    float outer = radius + soft;
    int x0, y0, x1, y1;
    if (alpha <= 0.0f || !clip(t, cx - outer - 1, cy - outer - 1, cx + outer + 1, cy + outer + 1, x0, y0, x1, y1))
        return;
    uint32_t r = (uint32_t)c.r, g = (uint32_t)c.g, b = (uint32_t)c.b;
    for (int y = y0; y < y1; y++) {
        float dy = (float)y + 0.5f - cy;
        for (int x = x0; x < x1; x++) {
            float dx = (float)x + 0.5f - cx;
            float d = sqrtf_(dx * dx + dy * dy);
            float cov = clamp01((outer + 0.5f - d) / (soft + 1.0f));
            if (cov <= 0.0f) continue;
            over(t.px(x, y), r, g, b, t.strength(y, (uint32_t)(alpha * cov * 255.0f + 0.5f)));
        }
    }
}

// ---------------------------------------------------------------- sprites
// A premultiplied BGRA picture made once (clouds, fog).
struct Sprite {
    int w, h;
    uint8_t *px;
    int16_t *span; // per row: first and last+1 visible column
};

// Premultiplied src over an opaque dst, n pixels, global alpha ga1 (1..256).
// No branches and 16-bit products: clang turns this into NEON code.
inline void over_run(uint8_t *__restrict d, const uint8_t *__restrict src, int n, uint32_t ga1) {
    for (int i = 0; i < n * 4; i += 4) {
        uint32_t inv = 256 - ((src[i + 3] * ga1) >> 8);
        d[i] = (uint8_t)(((src[i] * ga1) >> 8) + ((d[i] * inv) >> 8));
        d[i + 1] = (uint8_t)(((src[i + 1] * ga1) >> 8) + ((d[i + 1] * inv) >> 8));
        d[i + 2] = (uint8_t)(((src[i + 2] * ga1) >> 8) + ((d[i + 2] * inv) >> 8));
    }
}

// Sprite over the target with a global alpha 0..255; only its visible spans.
void blit(const Target &t, const Sprite &s, int sx, int sy, uint32_t ga) {
    if (!s.px || ga == 0) return;
    int x0 = maxi(0, sx), x1 = mini(t.w, sx + s.w);
    int y0 = maxi(t.y0, sy), y1 = mini(t.y1, sy + s.h);
    for (int y = y0; y < y1; y++) {
        uint32_t a_row = t.strength(y, ga);
        if (a_row == 0) continue;
        int r = y - sy;
        int a0 = maxi(x0, sx + s.span[r * 2]), a1 = mini(x1, sx + s.span[r * 2 + 1]);
        if (a0 >= a1) continue;
        over_run(t.px(a0, y), s.px + ((size_t)r * s.w + (a0 - sx)) * 4, a1 - a0, a_row + 1);
    }
}

// Density sampled on a coarse grid (noise is the costly part), then
// interpolated to every pixel: soft shapes lose nothing, the Pi saves 15/16
// of the noise.
template <class Density, class Shade>
bool make_sprite(Sprite &s, int w, int h, Density density, Shade shade) {
    const int STEP = 4;
    int gw = w / STEP + 2, gh = h / STEP + 2;
    float *grid = (float *)malloc(sizeof(float) * (size_t)gw * gh * 2);
    s.px = (uint8_t *)malloc((size_t)w * h * 4);
    s.span = (int16_t *)malloc(sizeof(int16_t) * 2 * (size_t)h);
    if (!grid || !s.px || !s.span) {
        free(grid);
        free(s.px);
        free(s.span);
        s.px = nullptr;
        s.span = nullptr;
        return false;
    }
    s.w = w;
    s.h = h;
    for (int j = 0; j < gh; j++)
        for (int i = 0; i < gw; i++) {
            float nx = (float)(i * STEP) / (float)w, ny = (float)(j * STEP) / (float)h;
            float n;
            grid[(j * gw + i) * 2] = density(nx, ny, n);
            grid[(j * gw + i) * 2 + 1] = n;
        }
    for (int y = 0; y < h; y++) {
        float gy = (float)y / STEP;
        int j = (int)gy;
        float fy = gy - (float)j;
        for (int x = 0; x < w; x++) {
            float gx = (float)x / STEP;
            int i = (int)gx;
            float fx = gx - (float)i;
            const float *a = grid + (j * gw + i) * 2, *b = a + gw * 2;
            float dens = lerp(lerp(a[0], a[2], fx), lerp(b[0], b[2], fx), fy);
            float n = lerp(lerp(a[1], a[3], fx), lerp(b[1], b[3], fx), fy);
            float al;
            Rgb c = shade((float)x / (float)w, (float)y / (float)h, dens, n, al);
            uint8_t *p = s.px + ((size_t)y * w + x) * 4;
            uint32_t a8 = (uint32_t)(clamp01(al) * 255.0f + 0.5f);
            p[0] = (uint8_t)div255((uint32_t)c.b * a8);
            p[1] = (uint8_t)div255((uint32_t)c.g * a8);
            p[2] = (uint8_t)div255((uint32_t)c.r * a8);
            p[3] = (uint8_t)a8;
        }
        // the visible part of the row (clouds are round: corners are empty)
        int a = 0, b = w;
        const uint8_t *row = s.px + (size_t)y * w * 4;
        while (a < w && row[a * 4 + 3] == 0) a++;
        while (b > a && row[(b - 1) * 4 + 3] == 0) b--;
        s.span[y * 2] = (int16_t)a;
        s.span[y * 2 + 1] = (int16_t)b;
    }
    free(grid);
    return true;
}

// ------------------------------------------------------------------ scene
const int MAX_DROPS = 260;
const int MAX_FLAKES = 150;
const int MAX_STARS = 90;
const int MAX_CLOUDS = 7;
const int CLOUD_SPRITES = 4;
const int BOLT_POINTS = 9;
const uint32_t FROST = 140; // of 256: how much a glass card calms the weather

struct Drop {
    float x0, y0, x1, y1, width, alpha;
};
struct Flake {
    float x, y, r, alpha;
};
struct Star {
    float x, y, r, alpha;
};
struct Cloud {
    int sprite;
    float x0, y, speed, alpha;
    int x; // now
};

struct Look {
    bool sun, moon, stars, rays;
    int clouds;         // how many drift
    float cloud_alpha;  // how dense
    Rgb cloud_light, cloud_dark;
    int drops;          // rain
    int flakes;         // snow
    int fog_bands;
    bool lightning;
};

} // namespace

struct WfxScene {
    WfxParams p;
    Look look;
    Rgb top, bottom, sun_col;
    float sun_x, sun_y;
    uint8_t *fade;          // per band row
    // sun rays: two angular profiles (baked into the still sky)
    uint8_t ray_a[1024], ray_b[1024];
    // calm behind the title: a feathered mask, made once
    uint8_t *scrim;
    int scrim_x0, scrim_y0, scrim_w, scrim_h;
    Sprite cloud[CLOUD_SPRITES];
    Sprite fog;
    // the still sky from the launcher
    const uint8_t *sky;
    int32_t sky_stride;
    // Everything laid over the moving weather -- the calm behind the title,
    // the frosted glass of the cards and the UI itself -- is affine in the
    // weather pixel d: out = base + d * mul / 256. Folded into one layer
    // when the UI arrives: per frame a single multiply-add per byte.
    int32_t *post_row;      // band_h + 1 offsets into post_runs
    int32_t *post_runs;     // x0, x1, offset into base/mul (in pixels)
    uint8_t *post_base;     // BGRA
    uint8_t *post_mul;      // the same factor in all four bytes
    // state at time t
    float t;
    float intro;            // 0..1 over the first seconds
    Cloud clouds[MAX_CLOUDS];
    Drop drops[MAX_DROPS];
    int n_drops;
    Flake flakes[MAX_FLAKES];
    int n_flakes;
    Star stars[MAX_STARS];
    int n_stars;
    float fog_x[3];
    float flash;            // 0..1 sky brightening
    float bolt_alpha;
    float bolt[BOLT_POINTS][2];
    float branch[4][2];
    float meteor[5];        // x, y, dx, dy, alpha
};

namespace {

Look look_for(int sky, bool day, float intensity) {
    Look l{};
    l.cloud_light = day ? rgb(0xe4ecf6) : rgb(0x56627e);
    l.cloud_dark = day ? rgb(0x9aa8bd) : rgb(0x263046);
    l.cloud_alpha = 0.7f;
    float k = 0.35f + 0.65f * clamp01(intensity);
    switch (sky) {
    case WFX_CLEAR:
        l.sun = day;
        l.moon = !day;
        l.stars = !day;
        l.rays = day;
        break;
    case WFX_PARTLY_CLOUDY:
        l.sun = day;
        l.moon = !day;
        l.stars = !day;
        l.rays = day;
        l.clouds = 4;
        l.cloud_alpha = 0.58f;
        break;
    case WFX_CLOUDY:
        l.clouds = 6;
        l.cloud_alpha = 0.62f;
        l.cloud_light = day ? rgb(0xdfe5ee) : rgb(0x4d5871);
        break;
    case WFX_FOG:
        l.clouds = 3;
        l.cloud_alpha = 0.4f;
        l.fog_bands = 2;
        break;
    case WFX_DRIZZLE:
        l.clouds = 5;
        l.cloud_alpha = 0.6f;
        l.drops = (int)(90 * k);
        l.cloud_light = day ? rgb(0xd9e0ea) : rgb(0x4a5570);
        break;
    case WFX_RAIN:
        l.clouds = 5;
        l.cloud_alpha = 0.7f;
        l.drops = (int)(220 * k);
        l.cloud_light = day ? rgb(0xcfd7e3) : rgb(0x46516b);
        l.cloud_dark = day ? rgb(0x8793a6) : rgb(0x232b40);
        break;
    case WFX_SNOW:
        l.clouds = 5;
        l.cloud_alpha = 0.55f;
        l.flakes = (int)(140 * k);
        break;
    case WFX_THUNDER:
    default:
        l.clouds = 6;
        l.cloud_alpha = 0.72f;
        l.drops = (int)(240 * k);
        l.lightning = true;
        l.cloud_light = day ? rgb(0x9a9cb4) : rgb(0x3b3f57);
        l.cloud_dark = day ? rgb(0x4d4f66) : rgb(0x1a1c2a);
        break;
    }
    if (l.drops > MAX_DROPS) l.drops = MAX_DROPS;
    if (l.flakes > MAX_FLAKES) l.flakes = MAX_FLAKES;
    return l;
}

// The still sky at one pixel: the launcher's two-colour gradient, a broad
// light where the sun (or moon) stands, a warm horizon at sunrise/sunset,
// a soft vignette; dithered so 8 bits show no bands.
inline void sky_pixel(const WfxScene *s, int x, int y, uint8_t *d) {
    const WfxParams &p = s->p;
    float fy = (float)y / (float)(p.height - 1);
    float fx = (float)x / (float)(p.width - 1);
    float g = fy * fy * (3.0f - 2.0f * fy) * 0.65f + fy * 0.35f;
    Rgb c = mix(s->top, s->bottom, g);
    // light around the sun / moon
    float dx = ((float)x - s->sun_x) / (float)p.width, dy = ((float)y - s->sun_y) / (float)p.width;
    float f = 1.0f - (dx * dx + dy * dy) * 3.2f;
    if (f > 0.0f) {
        float k = f * f * (s->look.sun ? 0.22f : (s->look.moon ? 0.10f : 0.05f));
        c.r += (s->sun_col.r - c.r) * k;
        c.g += (s->sun_col.g - c.g) * k;
        c.b += (s->sun_col.b - c.b) * k;
    }
    // sunrise / sunset: warm low sky
    if (p.golden > 0.0f) {
        float k = p.golden * smooth(0.25f, 1.0f, fy) * 0.38f;
        Rgb warm = rgb(0xff9a5c);
        c = mix(c, warm, k);
    }
    // sun rays, still: two soft fans of different width
    if (s->look.rays && y < p.band_h + p.band_h / 2) {
        float rx = (float)x - s->sun_x, ry = (float)y - s->sun_y;
        float d = sqrtf_(rx * rx + ry * ry);
        float reach = 1100.0f * p.scale;
        if (d < reach) {
            float a = (atan2f_(ry, rx) + PI) / TAU * 1024.0f;
            int ia = (int)a & 1023;
            float pr = ((float)s->ray_a[ia] + (float)s->ray_b[(ia * 7 / 5 + 300) & 1023]) * (1.0f / 510.0f);
            float fall = clamp01(1.0f - d / reach) * smooth(30.0f * p.scale, 170.0f * p.scale, d);
            float k = pr * fall * fall * (p.sky == WFX_CLEAR ? 34.0f : 20.0f);
            c.r += s->sun_col.r / 255.0f * k;
            c.g += s->sun_col.g / 255.0f * k;
            c.b += s->sun_col.b / 255.0f * k;
        }
    }
    // vignette (separable, cheap)
    float vx = fx * (1.0f - fx) * 4.0f, vy = 0.55f + 0.45f * (1.0f - fy * fy);
    float v = 0.82f + 0.18f * minf(1.0f, vx * vy * 1.15f);
    // ordered dither (4x4 Bayer), +-0.5 of a step
    static const uint8_t bayer[16] = {0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5};
    float dth = ((float)bayer[(y & 3) * 4 + (x & 3)] + 0.5f) / 16.0f;
    auto q = [&](float ch) {
        float vch = ch * v + dth;
        return (uint8_t)(vch < 0.0f ? 0 : (vch > 255.0f ? 255 : (int)vch));
    };
    d[0] = q(c.b);
    d[1] = q(c.g);
    d[2] = q(c.r);
    d[3] = 255;
}

// -------------------------------------------------------- the hero icon
// Signed distances (negative inside) in pixels.
inline float sd_circle(float px, float py, float cx, float cy, float r) {
    float dx = px - cx, dy = py - cy;
    return sqrtf_(dx * dx + dy * dy) - r;
}
inline float sd_rrect(float px, float py, float x0, float y0, float x1, float y1, float r) {
    float cx = (x0 + x1) * 0.5f, cy = (y0 + y1) * 0.5f;
    float hx = (x1 - x0) * 0.5f - r, hy = (y1 - y0) * 0.5f - r;
    float qx = absf(px - cx) - hx, qy = absf(py - cy) - hy;
    float ox = maxf(qx, 0.0f), oy = maxf(qy, 0.0f);
    return sqrtf_(ox * ox + oy * oy) + minf(maxf(qx, qy), 0.0f) - r;
}
// The launcher's cloud (same circles as ui::icon_weather).
inline float sd_cloud(float px, float py, float cx, float cy, float k) {
    float d = sd_circle(px, py, cx - k * 0.22f, cy + k * 0.05f, k * 0.2f);
    d = minf(d, sd_circle(px, py, cx + k * 0.02f, cy - k * 0.08f, k * 0.27f));
    d = minf(d, sd_circle(px, py, cx + k * 0.26f, cy + k * 0.06f, k * 0.18f));
    d = minf(d, sd_rrect(px, py, cx - k * 0.42f, cy + k * 0.04f, cx + k * 0.44f, cy + k * 0.24f, k * 0.1f));
    return d;
}

void icon_cloud(const Target &t, float cx, float cy, float k, bool dark, bool day) {
    Rgb top = dark ? rgb(0xb3bccb) : rgb(0xfbfcff);
    Rgb bot = dark ? rgb(0x7f899b) : rgb(0xc9d1de);
    if (!day && !dark) {
        top = rgb(0xe9edf5);
        bot = rgb(0xaeb7c8);
    }
    int x0, y0, x1, y1;
    if (!clip(t, cx - k * 0.5f, cy - k * 0.4f, cx + k * 0.52f, cy + k * 0.36f, x0, y0, x1, y1)) return;
    float soft = k * 0.07f;
    for (int y = y0; y < y1; y++) {
        float py = (float)y + 0.5f;
        float gy = clamp01((py - (cy - k * 0.36f)) / (k * 0.6f));
        Rgb c = mix(top, bot, gy * gy);
        for (int x = x0; x < x1; x++) {
            float px = (float)x + 0.5f;
            uint8_t *d = t.px(x, y);
            // soft shadow under the cloud: depth
            float db = sd_cloud(px, py, cx, cy, k);
            float cov = clamp01(0.5f - db);
            float sh = 0.0f;
            if (cov < 1.0f) {
                float ds = sd_cloud(px, py - k * 0.045f, cx, cy, k);
                sh = clamp01(0.5f - ds / soft) * 0.22f;
            }
            if (sh > 0.0f && cov < 1.0f) over(d, 20, 30, 50, (uint32_t)(sh * (1.0f - cov) * 255.0f));
            if (cov > 0.0f) {
                // a rim of light along the upper edge
                float rim = clamp01(1.0f + db / (k * 0.06f)) * clamp01(1.0f - gy * 2.0f) * 0.35f;
                over(d, (uint32_t)lerp(c.r, 255.0f, rim), (uint32_t)lerp(c.g, 255.0f, rim),
                     (uint32_t)lerp(c.b, 255.0f, rim), (uint32_t)(cov * 255.0f + 0.5f));
            }
        }
    }
}

void icon_sun(const Target &t, float cx, float cy, float k, float time) {
    float pulse = 0.85f + 0.15f * sinf_(time * 1.3f);
    glow(t, cx, cy, k * 0.72f, rgb(0xffd36e), 0.42f * pulse, false);
    float spin = time * 0.22f;
    for (int i = 0; i < 8; i++) {
        float a = (float)i * PI * 0.25f + spin;
        float ca = cosf_(a), sa = sinf_(a);
        float r1 = k * (0.45f + 0.025f * sinf_(time * 2.1f + (float)i * 1.7f));
        capsule(t, cx + ca * k * 0.33f, cy + sa * k * 0.33f, cx + ca * r1, cy + sa * r1, k * 0.075f, rgb(0xffcc33),
                1.0f, 0.0f, false);
    }
    int x0, y0, x1, y1;
    float r = k * 0.24f;
    if (!clip(t, cx - r - 1, cy - r - 1, cx + r + 1, cy + r + 1, x0, y0, x1, y1)) return;
    Rgb in = rgb(0xffe589), out = rgb(0xffc12b);
    for (int y = y0; y < y1; y++)
        for (int x = x0; x < x1; x++) {
            float dx = (float)x + 0.5f - cx, dy = (float)y + 0.5f - cy;
            float d = sqrtf_(dx * dx + dy * dy);
            float cov = clamp01(r + 0.5f - d);
            if (cov <= 0.0f) continue;
            // lit from the upper left
            float lt = clamp01((d / r) * 0.8f + (dx + dy) / (r * 4.0f));
            Rgb c = mix(in, out, lt);
            over(t.px(x, y), (uint32_t)c.r, (uint32_t)c.g, (uint32_t)c.b, (uint32_t)(cov * 255.0f + 0.5f));
        }
}

void icon_moon(const Target &t, float cx, float cy, float k, float time) {
    float pulse = 0.8f + 0.2f * sinf_(time * 0.7f);
    glow(t, cx, cy, k * 0.62f, rgb(0xc9d6ff), 0.32f * pulse, false);
    dot(t, cx, cy, k * 0.34f - 1.0f, rgb(0xdfe6f5), 0.16f, 1.0f);
    int x0, y0, x1, y1;
    float r = k * 0.25f;
    if (!clip(t, cx - r - 1, cy - r - 1, cx + r + 1, cy + r + 1, x0, y0, x1, y1)) return;
    for (int y = y0; y < y1; y++)
        for (int x = x0; x < x1; x++) {
            float px = (float)x + 0.5f, py = (float)y + 0.5f;
            float d = sd_circle(px, py, cx, cy, r);
            float cov = clamp01(0.5f - d);
            if (cov <= 0.0f) continue;
            float c1 = sd_circle(px, py, cx - k * 0.07f, cy - k * 0.05f, k * 0.05f);
            float c2 = sd_circle(px, py, cx + k * 0.08f, cy + k * 0.07f, k * 0.035f);
            float c3 = sd_circle(px, py, cx + k * 0.02f, cy - k * 0.13f, k * 0.022f);
            float crater = clamp01(0.5f - minf(c1, minf(c2, c3)));
            float lt = clamp01(((px - cx) + (py - cy)) / (r * 3.0f) + 0.5f);
            Rgb c = mix(rgb(0xf1f4fb), rgb(0xcfd7e8), lt);
            c = mix(c, rgb(0xb9c3d6), crater);
            over(t.px(x, y), (uint32_t)c.r, (uint32_t)c.g, (uint32_t)c.b, (uint32_t)(cov * 255.0f + 0.5f));
        }
}

void icon_bolt(const Target &t, float x, float y, float s, float alpha) {
    const float P[6][2] = {{0.0f, 0.12f}, {-0.1f, 0.32f}, {0.0f, 0.30f}, {-0.06f, 0.46f}, {0.1f, 0.24f}, {0.0f, 0.26f}};
    float vx[6], vy[6];
    float bx0 = 1e9f, by0 = 1e9f, bx1 = -1e9f, by1 = -1e9f;
    for (int i = 0; i < 6; i++) {
        vx[i] = x + P[i][0] * s;
        vy[i] = y + P[i][1] * s;
        bx0 = minf(bx0, vx[i]);
        by0 = minf(by0, vy[i]);
        bx1 = maxf(bx1, vx[i]);
        by1 = maxf(by1, vy[i]);
    }
    glow(t, x - s * 0.01f, y + s * 0.3f, s * 0.3f, rgb(0xffe08a), 0.35f * alpha, false);
    int x0, y0, x1, y1;
    if (!clip(t, bx0 - 1, by0 - 1, bx1 + 1, by1 + 1, x0, y0, x1, y1)) return;
    for (int yy = y0; yy < y1; yy++)
        for (int xx = x0; xx < x1; xx++) {
            float px = (float)xx + 0.5f, py = (float)yy + 0.5f;
            bool inside = false;
            float dmin = 1e9f;
            for (int i = 0, j = 5; i < 6; j = i++) {
                if (((vy[i] > py) != (vy[j] > py)) &&
                    (px < (vx[j] - vx[i]) * (py - vy[i]) / (vy[j] - vy[i]) + vx[i]))
                    inside = !inside;
                float al;
                dmin = minf(dmin, seg_dist(px, py, vx[i], vy[i], vx[j], vy[j], al));
            }
            float cov = clamp01(inside ? 0.5f + dmin : 0.5f - dmin);
            if (cov > 0.0f) over(t.px(xx, yy), 255, 204, 51, (uint32_t)(cov * alpha * 255.0f + 0.5f));
        }
}

void draw_icon(const WfxScene *s, const Target &t, float x, float y, float time) {
    float sz = s->p.icon_size;
    bool day = s->p.day != 0;
    float bob = sinf_(time * 0.9f) * sz * 0.012f;
    float drift = sinf_(time * 0.45f) * sz * 0.03f;
    auto sun_or_moon = [&](float cx, float cy, float k) {
        if (day)
            icon_sun(t, cx, cy, k, time);
        else
            icon_moon(t, cx, cy, k, time);
    };
    switch (s->p.sky) {
    case WFX_CLEAR:
        sun_or_moon(x, y, sz);
        break;
    case WFX_PARTLY_CLOUDY:
        sun_or_moon(x - sz * 0.14f, y - sz * 0.14f, sz * 0.8f);
        icon_cloud(t, x + sz * 0.08f + drift, y + sz * 0.08f + bob, sz * 0.8f, false, day);
        break;
    case WFX_CLOUDY:
    case WFX_FOG:
        icon_cloud(t, x - sz * 0.1f - drift * 0.6f, y - sz * 0.06f, sz * 0.7f, true, day);
        icon_cloud(t, x + sz * 0.06f + drift, y + sz * 0.06f + bob, sz * 0.85f, false, day);
        if (s->p.sky == WFX_FOG)
            for (int i = 0; i < 2; i++) {
                float o = sinf_(time * 0.8f + (float)i * 2.0f) * sz * 0.05f;
                float yy = y + sz * 0.34f + (float)i * sz * 0.1f;
                capsule(t, x - sz * 0.35f + o, yy, x + sz * 0.35f + o, yy, sz * 0.05f, rgb(0xdfe3ea), 0.95f, 0.0f,
                        false);
            }
        break;
    default: {
        bool thunder = s->p.sky == WFX_THUNDER;
        // falling first: they come from under the cloud
        for (int i = 0; i < 3; i++) {
            float dx = x - sz * 0.2f + (float)i * sz * 0.2f;
            for (int j = 0; j < 2; j++) {
                float speed = s->p.sky == WFX_DRIZZLE ? 0.8f : 1.25f;
                float ph = fract(time * speed + (float)i * 0.37f + (float)j * 0.5f);
                float a = ph < 0.2f ? ph / 0.2f : (1.0f - ph) / 0.8f;
                if (s->p.sky == WFX_SNOW) {
                    float sway = sinf_(time * 2.0f + (float)(i * 3 + j)) * sz * 0.03f;
                    dot(t, dx + sway, y + sz * (0.2f + ph * 0.28f), sz * 0.045f, rgb(0xf2f5fb), a, 1.2f);
                } else if (!(thunder && i == 1)) {
                    float oy = sz * (0.16f + ph * 0.2f);
                    float ox = -ph * sz * 0.05f;
                    float len = s->p.sky == WFX_DRIZZLE ? 0.09f : 0.14f;
                    capsule(t, dx + sz * 0.03f + ox, y + oy, dx + sz * 0.03f + ox - sz * len * 0.35f, y + oy + sz * len,
                            sz * (s->p.sky == WFX_DRIZZLE ? 0.04f : 0.05f), rgb(0x5ab0ff), a, 0.5f, false);
                }
            }
        }
        if (thunder) icon_bolt(t, x, y + bob * 0.5f, sz, clamp01(0.75f + s->flash * 0.6f));
        icon_cloud(t, x + drift * 0.5f, y - sz * 0.1f + bob, sz * 0.9f, thunder, day);
        break;
    }
    }
}

} // namespace

// ================================================================== C ABI

extern "C" WfxScene *wfx_create(const WfxParams *p) {
    if (!p || p->width <= 0 || p->height <= 0 || p->band_h <= 0 || p->band_h > p->height || p->scale <= 0.0f)
        return nullptr;
    WfxScene *s = (WfxScene *)malloc(sizeof(WfxScene));
    if (!s) return nullptr;
    __builtin_memset(s, 0, sizeof(WfxScene));
    s->p = *p;
    const float sc = p->scale;
    const bool day = p->day != 0;
    s->look = look_for(p->sky, day, p->intensity);
    s->top = rgb(p->sky_top);
    s->bottom = rgb(p->sky_bottom);
    s->sun_x = p->icon_x;
    s->sun_y = p->icon_y;
    s->sun_col = day ? mix(rgb(0xffe2a0), rgb(0xffa060), p->golden) : rgb(0x9fb6ff);
    if (!s->look.sun && !s->look.moon) s->sun_col = mix(s->top, rgb(0xffffff), 0.35f);

    // the effects thin out towards the bottom of the band (no hard edge)
    s->fade = (uint8_t *)malloc((size_t)p->band_h);
    if (!s->fade) {
        wfx_destroy(s);
        return nullptr;
    }
    float feather = 70.0f * sc;
    for (int y = 0; y < p->band_h; y++) {
        float f = clamp01((float)(p->band_h - y) / feather);
        s->fade[y] = (uint8_t)(f * f * (3.0f - 2.0f * f) * 255.0f + 0.5f);
    }

    uint32_t seed = hash32(p->seed + 1);

    // sun rays: two angular profiles of soft spikes
    if (s->look.rays) {
        for (int k = 0; k < 2; k++) {
            uint8_t *prof = k ? s->ray_b : s->ray_a;
            float acc[1024];
            for (int i = 0; i < 1024; i++) acc[i] = 0.0f;
            int rays = k ? 7 : 9;
            for (int r = 0; r < rays; r++) {
                float centre = hashf(seed + 31u * (uint32_t)r + 977u * (uint32_t)k) * 1024.0f;
                float width = 14.0f + hashf(seed + 7u * (uint32_t)r + 5u + (uint32_t)k) * 34.0f;
                float str = 0.5f + 0.5f * hashf(seed + 13u * (uint32_t)r + 3u + (uint32_t)k);
                for (int i = 0; i < 1024; i++) {
                    float d = absf((float)i - centre);
                    if (d > 512.0f) d = 1024.0f - d;
                    float f = clamp01(1.0f - d / width);
                    acc[i] += str * f * f * (3.0f - 2.0f * f);
                }
            }
            for (int i = 0; i < 1024; i++) prof[i] = (uint8_t)(clamp01(acc[i]) * 255.0f);
        }
    }

    // the calm behind the title: k (0..205) per pixel of a feathered box
    if (p->scrim[2] > p->scrim[0] && p->scrim[3] > p->scrim[1]) {
        float feather = 56.0f * sc;
        int x0 = maxi(0, floori(p->scrim[0] - feather)), x1 = mini(p->width, ceili(p->scrim[2] + feather));
        int y0 = maxi(0, floori(p->scrim[1] - feather)), y1 = mini(p->band_h, ceili(p->scrim[3] + feather));
        if (x0 < x1 && y0 < y1) {
            s->scrim = (uint8_t *)malloc((size_t)(x1 - x0) * (y1 - y0));
            if (s->scrim) {
                s->scrim_x0 = x0;
                s->scrim_y0 = y0;
                s->scrim_w = x1 - x0;
                s->scrim_h = y1 - y0;
                for (int y = y0; y < y1; y++) {
                    float py = (float)y + 0.5f;
                    float dy = maxf(0.0f, maxf(p->scrim[1] - py, py - p->scrim[3])) / feather;
                    for (int x = x0; x < x1; x++) {
                        float px = (float)x + 0.5f;
                        float dx = maxf(0.0f, maxf(p->scrim[0] - px, px - p->scrim[2])) / feather;
                        float f = clamp01(1.0f - (dx * dx + dy * dy));
                        s->scrim[(size_t)(y - y0) * s->scrim_w + (x - x0)] = (uint8_t)(f * f * 205.0f);
                    }
                }
            }
        }
    }

    // cloud sprites: fbm noise in a flat-bottomed puff, lit from above
    if (s->look.clouds > 0) {
        const float sizes[CLOUD_SPRITES] = {560.0f, 460.0f, 380.0f, 300.0f};
        for (int i = 0; i < CLOUD_SPRITES; i++) {
            int w = (int)(sizes[i] * sc), h = (int)(sizes[i] * 0.4f * sc);
            uint32_t cs = seed + 101u * (uint32_t)(i + 1);
            Rgb light = s->look.cloud_light, dark = s->look.cloud_dark;
            make_sprite(
                s->cloud[i], w, h,
                [cs](float nx, float ny, float &n) {
                    n = fbm(nx * 3.2f, ny * 1.6f, cs);
                    float dx = (nx - 0.5f) * 2.0f, dy = (ny - 0.62f) / 0.4f;
                    float base = 1.0f - (dx * dx + (dy < 0.0f ? dy * dy * 0.85f : dy * dy * 2.2f));
                    float win = clamp01(minf(minf(nx, 1.0f - nx), minf(ny, 1.0f - ny)) * 7.0f);
                    return (base + (n - 0.5f) * 1.1f) * win;
                },
                [light, dark](float, float ny, float dens, float n, float &al) {
                    al = smooth(0.14f, 0.52f, dens);
                    float lit = clamp01(1.15f - ny * 0.9f + (n - 0.5f) * 0.7f);
                    return mix(dark, light, lit);
                });
        }
    }
    if (s->look.fog_bands > 0) {
        int w = (int)(1000.0f * sc), h = (int)(110.0f * sc);
        uint32_t fs = seed + 4242u;
        make_sprite(
            s->fog, w, h,
            [fs](float nx, float ny, float &n) {
                n = fbm(nx * 7.0f, ny * 1.4f, fs);
                float vy = 1.0f - absf(ny - 0.5f) * 2.0f;
                float vx = clamp01(minf(nx, 1.0f - nx) * 4.0f);
                return vy * vy * vx * (0.45f + 0.55f * n);
            },
            [](float, float, float dens, float, float &al) {
                al = clamp01(dens) * 0.75f;
                return rgb(0xe6ebf2);
            });
    }

    // where every cloud drifts: depth decides size, speed and density
    float drift = 6.0f + p->wind * 0.6f;
    for (int i = 0; i < s->look.clouds && i < MAX_CLOUDS; i++) {
        Cloud &c = s->clouds[i];
        c.sprite = i % CLOUD_SPRITES;
        float depth = 1.0f - (float)c.sprite / (float)(CLOUD_SPRITES - 1); // 1 = near
        c.x0 = hashf(seed + 3u * (uint32_t)i) * (float)p->width;
        c.y = (lerp(-70.0f, (float)p->band_h / sc * 0.55f, hashf(seed + 5u * (uint32_t)i + 1)) +
               (1.0f - depth) * -30.0f) * sc;
        c.speed = (drift * (0.45f + 0.75f * depth) + hashf(seed + 9u * (uint32_t)i) * 3.0f) * sc;
        c.alpha = s->look.cloud_alpha * (0.6f + 0.4f * depth);
    }
    s->n_drops = s->look.drops;
    s->n_flakes = s->look.flakes;
    s->n_stars = s->look.stars ? (s->look.clouds ? 45 : MAX_STARS) : 0;
    for (int i = 0; i < s->n_stars; i++) {
        Star &st = s->stars[i];
        st.x = hashf(seed + 11u * (uint32_t)i) * (float)p->width;
        st.y = hashf(seed + 13u * (uint32_t)i + 7) * (float)p->band_h * 0.85f;
        st.r = (0.5f + hashf(seed + 17u * (uint32_t)i) * 0.9f) * sc;
    }
    wfx_advance(s, 0.0f);
    return s;
}

extern "C" void wfx_destroy(WfxScene *s) {
    if (!s) return;
    free(s->fade);
    free(s->scrim);
    for (int i = 0; i < CLOUD_SPRITES; i++) {
        free(s->cloud[i].px);
        free(s->cloud[i].span);
    }
    free(s->fog.px);
    free(s->fog.span);
    free(s->post_row);
    free(s->post_runs);
    free(s->post_base);
    free(s->post_mul);
    free(s);
}

extern "C" void wfx_render_sky_rows(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t y0, int32_t y1) {
    if (!s || !dst) return;
    y0 = maxi(0, y0);
    y1 = mini(s->p.height, y1);
    for (int y = y0; y < y1; y++) {
        uint8_t *row = dst + (ptrdiff_t)(y - y0) * stride;
        for (int x = 0; x < s->p.width; x++) sky_pixel(s, x, y, row + x * 4);
    }
}

extern "C" int32_t wfx_set_layers(WfxScene *s, const uint8_t *sky, int32_t sky_stride, const uint8_t *ui,
                                  int32_t ui_stride) {
    if (!s || !sky) return 0;
    s->sky = sky;
    s->sky_stride = sky_stride;
    free(s->post_row);
    free(s->post_runs);
    free(s->post_base);
    free(s->post_mul);
    s->post_row = nullptr;
    s->post_runs = nullptr;
    s->post_base = nullptr;
    s->post_mul = nullptr;
    const int h = s->p.band_h, w = s->p.width;
    // a pixel is part of the layer when the UI or the calm touches it
    auto scrim_k = [s](int x, int y) -> uint32_t {
        if (!s->scrim || x < s->scrim_x0 || y < s->scrim_y0 || x >= s->scrim_x0 + s->scrim_w ||
            y >= s->scrim_y0 + s->scrim_h)
            return 0;
        return s->scrim[(size_t)(y - s->scrim_y0) * s->scrim_w + (x - s->scrim_x0)];
    };
    auto touched = [&](int x, int y) {
        return (ui && ui[(ptrdiff_t)y * ui_stride + x * 4 + 3] != 0) || scrim_k(x, y) != 0;
    };
    size_t runs = 0, pixels = 0;
    for (int y = 0; y < h; y++) {
        bool in = false;
        for (int x = 0; x < w; x++) {
            bool t = touched(x, y);
            if (t && !in) runs++;
            pixels += t;
            in = t;
        }
    }
    s->post_row = (int32_t *)malloc(sizeof(int32_t) * (size_t)(h + 1));
    s->post_runs = (int32_t *)malloc(sizeof(int32_t) * 3 * (runs + 1));
    s->post_base = (uint8_t *)malloc(pixels * 4 + 4);
    s->post_mul = (uint8_t *)malloc(pixels * 4 + 4);
    if (!s->post_row || !s->post_runs || !s->post_base || !s->post_mul) {
        s->sky = nullptr;
        return 0;
    }
    size_t k = 0, at = 0;
    for (int y = 0; y < h; y++) {
        s->post_row[y] = (int32_t)k;
        const uint8_t *q = sky + (ptrdiff_t)y * sky_stride;
        int x = 0;
        while (x < w) {
            while (x < w && !touched(x, y)) x++;
            if (x >= w) break;
            int a = x;
            s->post_runs[k * 3] = a;
            s->post_runs[k * 3 + 2] = (int32_t)at;
            for (; x < w && touched(x, y); x++, at++) {
                // calm: d1 = (d (256-k) + q k) / 256 * dk / 256
                float ks = (float)scrim_k(x, y), dk = (256.0f - (float)(((uint32_t)ks * 50) >> 8)) / 256.0f;
                float ms = (256.0f - ks) / 256.0f * dk;
                // glass and UI: d2 = sp + ((d1 (256-F) + q F) / 256) (256-sa) / 256
                const uint8_t *sp = ui ? ui + (ptrdiff_t)y * ui_stride + x * 4 : nullptr;
                float sa = sp ? (float)sp[3] : 0.0f;
                float mu = sp && sa > 0.0f ? (256.0f - (float)FROST) / 256.0f * (256.0f - sa) / 256.0f : 1.0f;
                float m = mu * ms;
                uint32_t m8 = (uint32_t)(m * 256.0f + 0.5f);
                if (m8 > 255) m8 = 255;
                for (int ch = 0; ch < 3; ch++) {
                    float qv = (float)q[x * 4 + ch];
                    float bs = qv * ks / 256.0f * dk;
                    float bu = sp && sa > 0.0f ? (float)sp[ch] + qv * (float)FROST / 256.0f * (256.0f - sa) / 256.0f
                                               : 0.0f;
                    float b = bu + mu * bs;
                    s->post_base[at * 4 + ch] = (uint8_t)(b > 255.0f ? 255 : (int)(b + 0.5f));
                    s->post_mul[at * 4 + ch] = (uint8_t)m8;
                }
                s->post_base[at * 4 + 3] = (uint8_t)(255 - ((255 * m8) >> 8));
                s->post_mul[at * 4 + 3] = (uint8_t)m8;
            }
            s->post_runs[k * 3 + 1] = x;
            k++;
        }
    }
    s->post_row[h] = (int32_t)k;
    return 1;
}

extern "C" void wfx_advance(WfxScene *s, float t) {
    if (!s) return;
    const WfxParams &p = s->p;
    const float sc = p.scale;
    const uint32_t seed = hash32(p.seed + 1);
    s->t = t;
    s->intro = smooth(0.0f, 1.6f, t);
    const float W = (float)p.width, B = (float)p.band_h;

    for (int i = 0; i < s->look.clouds && i < MAX_CLOUDS; i++) {
        Cloud &c = s->clouds[i];
        float sw = (float)s->cloud[c.sprite].w;
        c.x = (int)(wrap(c.x0 + c.speed * t, W + sw) - sw);
    }

    // rain: two depths; far drops are thin, slow and faint
    float slant = minf(p.wind / 70.0f, 0.38f) + 0.06f;
    for (int i = 0; i < s->n_drops; i++) {
        Drop &d = s->drops[i];
        bool near = (hash32(seed + 7u * (uint32_t)i) & 3u) == 0;
        float drizzle = p.sky == WFX_DRIZZLE ? 0.6f : 1.0f;
        float speed = (near ? 1100.0f : 720.0f) * sc * drizzle;
        float len = (near ? 34.0f : 20.0f) * sc * drizzle;
        float span = B + len + 60.0f * sc;
        float y = wrap(hashf(seed + 3u * (uint32_t)i) * span + speed * t, span) - len;
        float x = wrap(hashf(seed + 5u * (uint32_t)i) * (W + 200.0f * sc) + y * slant, W + 200.0f * sc) - 100.0f * sc;
        d.x1 = x;
        d.y1 = y + len;
        d.x0 = x - len * slant;
        d.y0 = y;
        d.width = (near ? 2.2f : 1.4f) * sc * (p.sky == WFX_DRIZZLE ? 0.75f : 1.0f);
        d.alpha = near ? 0.55f : 0.3f;
    }

    // snow: slow, swaying, three sizes
    for (int i = 0; i < s->n_flakes; i++) {
        Flake &f = s->flakes[i];
        float depth = hashf(seed + 19u * (uint32_t)i);
        float speed = (22.0f + 46.0f * depth) * sc;
        float span = B + 20.0f * sc;
        float y = wrap(hashf(seed + 23u * (uint32_t)i) * span + speed * t, span) - 10.0f * sc;
        float sway = sinf_(t * (0.6f + depth) + (float)i * 1.3f) * (10.0f + 22.0f * depth) * sc;
        f.x = wrap(hashf(seed + 29u * (uint32_t)i) * W + sway + p.wind * 0.4f * sc * t, W);
        f.y = y;
        f.r = (0.9f + 1.7f * depth * depth) * sc;
        f.alpha = 0.35f + 0.4f * depth;
    }

    for (int i = 0; i < s->n_stars; i++) {
        Star &st = s->stars[i];
        float tw = sinf_(t * (0.8f + hashf(seed + 37u * (uint32_t)i) * 2.2f) + (float)i * 2.7f);
        st.alpha = (0.35f + 0.45f * hashf(seed + 41u * (uint32_t)i)) * (0.65f + 0.35f * tw);
    }

    for (int i = 0; i < s->look.fog_bands && i < 3; i++) {
        float sw = (float)s->fog.w;
        float dir = (i & 1) ? -1.0f : 1.0f;
        s->fog_x[i] = wrap(hashf(seed + 51u * (uint32_t)i) * (W + sw) + dir * (9.0f + 5.0f * (float)i) * sc * t,
                           W + sw) - sw;
    }

    // lightning: at most one strike every period, a double flicker
    s->flash = 0.0f;
    s->bolt_alpha = 0.0f;
    if (s->look.lightning) {
        const float period = 7.5f;
        int k = floori(t / period);
        for (int kk = k - 1; kk <= k; kk++) {
            if (kk < 0) continue;
            uint32_t ks = seed + 61u * (uint32_t)kk;
            if (hashf(ks) > 0.8f) continue;
            float at = (float)kk * period + 1.5f + hashf(ks + 1) * (period - 2.5f);
            float dt = t - at;
            if (dt < 0.0f || dt > 1.2f) continue;
            float f = dt < 0.07f ? 1.0f : dt < 0.13f ? 0.2f : dt < 0.2f ? 0.8f : (1.0f - (dt - 0.2f)) * 0.5f;
            s->flash = maxf(s->flash, clamp01(f));
            if (dt < 0.32f) {
                s->bolt_alpha = dt < 0.07f || (dt > 0.13f && dt < 0.26f) ? 1.0f : 0.35f;
                float x = (0.12f + 0.76f * hashf(ks + 2)) * W;
                float y = -10.0f * sc;
                float step = B * 0.72f / (float)(BOLT_POINTS - 1);
                for (int i = 0; i < BOLT_POINTS; i++) {
                    s->bolt[i][0] = x;
                    s->bolt[i][1] = y;
                    x += (hashf(ks + 10u + (uint32_t)i) - 0.5f) * 70.0f * sc;
                    y += step * (0.7f + 0.6f * hashf(ks + 30u + (uint32_t)i));
                }
                int from = 3 + (int)(hashf(ks + 50) * 3.0f);
                float bx = s->bolt[from][0], by = s->bolt[from][1];
                float dir = hashf(ks + 51) < 0.5f ? -1.0f : 1.0f;
                for (int i = 0; i < 4; i++) {
                    s->branch[i][0] = bx;
                    s->branch[i][1] = by;
                    bx += dir * (18.0f + 30.0f * hashf(ks + 60u + (uint32_t)i)) * sc;
                    by += step * 0.55f;
                }
            }
        }
    }

    // a shooting star now and then on clear nights
    s->meteor[4] = 0.0f;
    if (s->look.stars && s->look.clouds == 0) {
        const float period = 11.0f;
        int k = floori(t / period);
        uint32_t ks = seed + 71u * (uint32_t)k;
        float at = (float)k * period + 2.0f + hashf(ks) * 6.0f;
        float dt = t - at;
        if (hashf(ks + 1) < 0.65f && dt >= 0.0f && dt < 0.9f) {
            float x = (0.35f + 0.6f * hashf(ks + 2)) * W, y = (0.05f + 0.3f * hashf(ks + 3)) * B;
            float v = 900.0f * sc;
            s->meteor[0] = x - v * dt;
            s->meteor[1] = y + v * dt * 0.38f;
            s->meteor[2] = v * 0.16f;
            s->meteor[3] = -v * 0.16f * 0.38f;
            s->meteor[4] = sinf_(dt / 0.9f * PI);
        }
    }
}

extern "C" void wfx_render_rows(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t y0, int32_t y1) {
    if (!s || !dst || !s->sky) return;
    const WfxParams &p = s->p;
    y0 = maxi(0, y0);
    y1 = mini(p.band_h, y1);
    if (y0 >= y1) return;
    const int W = p.width;
    const float sc = p.scale;
    Target t{dst, stride, W, y0, y1, s->fade, (uint32_t)(s->intro * 256.0f + 0.5f)};

    // 1. the still sky
    for (int y = y0; y < y1; y++) __builtin_memcpy(t.px(0, y), s->sky + (ptrdiff_t)y * s->sky_stride, (size_t)W * 4);

    // 2. stars and a shooting star
    for (int i = 0; i < s->n_stars; i++) {
        const Star &st = s->stars[i];
        float m = st.r + 1.2f * sc + 3.0f;
        if (st.y + m < y0 || st.y - m > y1) continue;
        dot(t, st.x, st.y, st.r, rgb(0xf4f7ff), st.alpha, 1.2f * sc);
    }
    if (s->meteor[4] > 0.0f) {
        const float *m = s->meteor;
        capsule(t, m[0] + m[2], m[1] + m[3], m[0], m[1], 2.0f * sc, rgb(0xf6f8ff), m[4] * 0.9f, 1.0f, true);
    }

    // 3. clouds, far to near, with the fog between them
    for (int i = s->look.clouds - 1; i >= 0; i--) {
        const Cloud &c = s->clouds[i];
        if (c.sprite >= 2) blit(t, s->cloud[c.sprite], c.x, (int)c.y, (uint32_t)(c.alpha * 255.0f));
    }
    for (int i = 0; i < s->look.fog_bands && i < 3; i++) {
        float yy = ((float)p.band_h * (0.28f + 0.34f * (float)i)) + sinf_(s->t * 0.3f + (float)i) * 8.0f * sc;
        blit(t, s->fog, (int)s->fog_x[i], (int)yy - s->fog.h / 2, 170);
    }
    // far rain behind the near clouds
    for (int i = 0; i < s->n_drops; i++) {
        const Drop &d = s->drops[i];
        if (d.alpha > 0.4f || d.y1 + d.width + 2 < y0 || d.y0 - d.width - 2 > y1) continue;
        streak(t, d.x0, d.y0, d.x1, d.y1, d.width, rgb(0xc8dcf4), d.alpha, 0.85f);
    }
    for (int i = s->look.clouds - 1; i >= 0; i--) {
        const Cloud &c = s->clouds[i];
        if (c.sprite < 2) blit(t, s->cloud[c.sprite], c.x, (int)c.y, (uint32_t)(c.alpha * 255.0f));
    }

    // 4. lightning (the flash lights everything: after the UI, step 7)
    if (s->bolt_alpha > 0.0f) {
        for (int pass = 0; pass < 2; pass++) {
            float w = pass ? 2.6f * sc : 14.0f * sc;
            float a = s->bolt_alpha * (pass ? 1.0f : 0.18f);
            Rgb c = pass ? rgb(0xffffff) : rgb(0xc9d4ff);
            for (int i = 0; i + 1 < BOLT_POINTS; i++)
                capsule(t, s->bolt[i][0], s->bolt[i][1], s->bolt[i + 1][0], s->bolt[i + 1][1], w, c, a, 0.0f, true);
            for (int i = 0; i + 1 < 4; i++)
                capsule(t, s->branch[i][0], s->branch[i][1], s->branch[i + 1][0], s->branch[i + 1][1], w * 0.6f, c,
                        a * 0.7f, 0.0f, true);
        }
    }

    // 5. near rain and snow
    for (int i = 0; i < s->n_drops; i++) {
        const Drop &d = s->drops[i];
        if (d.alpha <= 0.4f || d.y1 + d.width + 2 < y0 || d.y0 - d.width - 2 > y1) continue;
        streak(t, d.x0, d.y0, d.x1, d.y1, d.width, rgb(0xdbe8fa), d.alpha, 0.85f);
    }
    for (int i = 0; i < s->n_flakes; i++) {
        const Flake &f = s->flakes[i];
        float m = f.r * 1.8f + 3.0f;
        if (f.y + m < y0 || f.y - m > y1) continue;
        dot(t, f.x, f.y, f.r, rgb(0xffffff), f.alpha, f.r * 0.8f);
    }

    // 6. calm behind the title, frosted glass and the UI: one affine layer
    if (s->post_row) {
        for (int y = y0; y < y1; y++) {
            uint8_t *row = t.px(0, y);
            for (int k = s->post_row[y]; k < s->post_row[y + 1]; k++) {
                const int32_t *r = s->post_runs + k * 3;
                uint8_t *__restrict d = row + r[0] * 4;
                const uint8_t *__restrict b = s->post_base + (size_t)r[2] * 4;
                const uint8_t *__restrict m = s->post_mul + (size_t)r[2] * 4;
                int n = (r[1] - r[0]) * 4;
                for (int i = 0; i < n; i++) d[i] = (uint8_t)(b[i] + ((d[i] * m[i]) >> 8));
            }
        }
    }

    // 7. the lightning flash lights the whole band, glass and text too
    if (s->flash > 0.0f) {
        uint32_t a = (uint32_t)(s->flash * 70.0f);
        for (int y = y0; y < y1; y++) {
            uint32_t ar = t.strength(y, a);
            if (!ar) continue;
            uint8_t *__restrict d = t.px(0, y);
            uint32_t ab = (255 * ar) >> 8, ag = (215 * ar) >> 8, arr = (205 * ar) >> 8;
            for (int i = 0; i < W * 4; i += 4) {
                uint32_t b = d[i] + ab, g = d[i + 1] + ag, r = d[i + 2] + arr;
                d[i] = (uint8_t)(b > 255 ? 255 : b);
                d[i + 1] = (uint8_t)(g > 255 ? 255 : g);
                d[i + 2] = (uint8_t)(r > 255 ? 255 : r);
            }
        }
    }

    // 8. the hero icon on top, no fade (it sits in the card)
    Target ti{dst, stride, W, y0, y1, nullptr, 256};
    draw_icon(s, ti, p.icon_x, p.icon_y, s->t);
}

extern "C" void wfx_draw_icon(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t w, int32_t h, float cx,
                              float cy, float t) {
    if (!s || !dst || w <= 0 || h <= 0) return;
    Target tg{dst, stride, w, 0, h, nullptr, 256};
    draw_icon(s, tg, cx, cy, t);
}
