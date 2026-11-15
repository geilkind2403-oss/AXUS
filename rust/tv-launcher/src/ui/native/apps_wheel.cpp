// Small freestanding renderer for the app wheel. Rust owns app launches and
// decoded image assets; this module owns its animation and drawing layout.
using size_t = __SIZE_TYPE__;
using u32 = unsigned int;

struct TextView { const char* data; size_t size; };
struct Input {
    u32 focus;
    u32 count;
    u32 pass;
    float dt;
    float launch;
    float entry;
    u32 detail_color;
    TextView title;
    TextView detail;
};
struct Animation {
    float rotation;
    float target;
    float elapsed;
    u32 last_focus;
    u32 initialized;
    u32 moving;
};
using Emit = void (*)(void*, u32, float, float, float, float, float, float,
                      u32, u32, u32, const char*, size_t);

namespace {
constexpr u32 Circle = 0, Rect = 1, Line = 2, Text = 3, Fit = 4, Image = 5;
constexpr float Pi = 3.14159265358979323846f;
constexpr u32 GlassAsset = 100;
constexpr u32 LogoGlowAsset = 101;
constexpr u32 CenterAsset = 102;

u32 rgba(u32 r, u32 g, u32 b, u32 a) {
    return (r << 24) | (g << 16) | (b << 8) | a;
}
float clamp01(float x) { return x < 0 ? 0 : (x > 1 ? 1 : x); }
float ease(float x) {
    x = clamp01(x);
    return x * x * (3.0f - 2.0f * x);
}
float ease_out(float x) {
    x = clamp01(x);
    const float y = 1.0f - x;
    return 1.0f - y * y * y;
}
float approach(float x, float target, float dt) {
    const float k = dt / (0.085f + dt);
    const float next = x + (target - x) * k;
    return (target - next < 0.001f && target - next > -0.001f) ? target : next;
}

struct Draw {
    void* ctx;
    Emit emit;
    float scale;
};
void command(Draw* d, u32 kind, float x, float y, float w, float h,
             float radius, float stroke, u32 color, u32 arg = 0,
             u32 px = 0, const char* text = nullptr, size_t size = 0) {
    const float cx = 640.0f, cy = 360.0f;
    x = cx + (x - cx) * d->scale;
    y = cy + (y - cy) * d->scale;
    w *= d->scale;
    h *= d->scale;
    radius *= d->scale;
    stroke *= d->scale;
    px = static_cast<u32>(px * d->scale);
    d->emit(d->ctx, kind, x, y, w, h, radius, stroke, color, arg, px, text, size);
}
void label(Draw* d, const char* text, size_t n, float x, float y,
           u32 px, u32 weight, u32 c, u32 align = 0) {
    command(d, Text, x, y, 0, 0, 0, 0, c, (weight << 8) | align, px, text, n);
}
u32 accent(u32 i) {
    switch (i) {
        case 0: return rgba(240, 68, 75, 255);
        case 1: return rgba(84, 191, 255, 255);
        case 2: return rgba(155, 180, 220, 255);
        case 3: return rgba(154, 114, 255, 255);
        default: return rgba(229, 232, 239, 255);
    }
}
} // namespace

extern "C" void tv_apps_wheel_draw(const Input* in, Animation* a,
                                     void* ctx, Emit emit) {
    if (!in || !a || !emit || in->count == 0) return;
    const float dt = in->dt < 0 ? 0 : (in->dt > 0.08f ? 0.08f : in->dt);
    const u32 focus = in->focus % in->count;
    const float step = 2.0f * Pi / static_cast<float>(in->count);
    if (in->pass != 2) {
        if (!a->initialized) {
            a->rotation = focus * step;
            a->target = a->rotation;
            a->last_focus = focus;
            a->initialized = 1;
        } else if (focus != a->last_focus) {
            int delta = static_cast<int>(focus) - static_cast<int>(a->last_focus);
            const int count = static_cast<int>(in->count);
            if (delta > count / 2) delta -= count;
            if (delta < -count / 2) delta += count;
            a->target += static_cast<float>(delta) * step;
            a->last_focus = focus;
        }
        a->rotation = approach(a->rotation, a->target, dt);
        a->moving = (a->rotation != a->target) ? 1u : 0u;
    }
    a->elapsed += in->pass == 2 ? 0.0f : dt;

    const bool entering = in->pass == 0 && in->entry >= 0.0f;
    const float enter = entering ? ease_out(in->entry) : 1.0f;
    const float draw_scale = entering ? 0.91f + 0.09f * enter : 1.0f;
    Draw d{ctx, emit, draw_scale};
    const u32 white = rgba(250, 247, 239, 255);
    const u32 muted = rgba(181, 175, 162, 255);
    const u32 gold = rgba(224, 185, 109, 255);
    const float cx = 640.0f, cy = 370.0f;

    const bool draw_static = in->pass != 2;
    const bool draw_items = in->pass != 1;
    if (draw_static) {
        constexpr char Heading[] = "Apps";
        label(&d, Heading, sizeof(Heading) - 1, 64.0f, 42.0f, 38, 2, white);
        constexpr char Hint[] = "App-Kreis";
        label(&d, Hint, sizeof(Hint) - 1, 64.0f, 92.0f, 14, 0, muted);
        // This glass layer is part of the retained base, not the moving icons.
        command(&d, Image, cx - 239.0f, cy - 239.0f, 478.0f, 478.0f,
                0, 0, 0xffffffffu, GlassAsset);
        command(&d, Image, cx - 112.0f, cy - 112.0f, 224.0f, 224.0f,
                0, 0, 0xffffffffu, CenterAsset);
        if (in->launch < 0.0f) {
            constexpr char Back[] = "← / → AUSWÄHLEN     ·     OK STARTEN     ·     ↑ ZURÜCK";
            label(&d, Back, sizeof(Back) - 1, 640.0f, 674.0f, 14, 2, muted, 1);
        }
    }

    if (draw_items) {
    if (in->launch < 0.0f) {
        label(&d, in->title.data, in->title.size, cx, cy - 25.0f,
              16, 2, white, 1);
        label(&d, in->detail.data, in->detail.size, cx, cy - 2.0f,
              9, 0, in->detail_color, 1);
        constexpr char Open[] = "OK · ÖFFNEN";
        label(&d, Open, sizeof(Open) - 1, cx, cy + 19.0f, 9, 1, gold, 1);
    }
    const float entry_spin = entering ? (1.0f - enter) * (0.42f * 2.0f * Pi) : 0.0f;
    const float rotation = a->rotation + entry_spin;
    for (u32 i = 0; i < in->count; ++i) {
        const float angle = -Pi * 0.5f + i * step - rotation;
        const float x = cx + __builtin_cosf(angle) * 164.0f;
        const float y = cy + __builtin_sinf(angle) * 164.0f;
        if (in->launch >= 0.0f && i == focus) continue;
        const bool selected = i == focus;
        const u32 color = accent(i);
        if (selected) {
            command(&d, Image, x - 56.0f, y - 56.0f, 112.0f, 112.0f,
                    0, 0, 0xffffffffu, LogoGlowAsset);
        }
        command(&d, Image, x - 31.0f, y - 31.0f, 62.0f, 62.0f,
                15.0f, 0, color, i);
    }

    if (in->launch >= 0.0f) {
        const float p = clamp01(in->launch);
        const u32 color = accent(focus);
        if (p < 0.62f) {
            const float t = ease_out(p / 0.62f);
            const float y = 132.0f + (cy - 132.0f) * t;
            const float size = 62.0f + 45.0f * ease(t);
            command(&d, Image, cx - size * 0.95f, y - size * 0.95f,
                    size * 1.9f, size * 1.9f, 0, 0, 0xffffffffu, LogoGlowAsset);
            command(&d, Image, cx - size * 0.5f, y - size * 0.5f,
                    size, size, size * 0.18f, 0, color, focus);
        } else {
            const float t = ease_out((p - 0.62f) / 0.38f);
            const float glow_radius = 92.0f + 680.0f * t;
            command(&d, Image, cx - glow_radius, cy - glow_radius,
                    glow_radius * 2.0f, glow_radius * 2.0f,
                    0, 0, 0xffffffffu, LogoGlowAsset);
            command(&d, Image, cx - (106.0f + 122.0f * t) * 0.5f,
                    cy - (106.0f + 122.0f * t) * 0.5f,
                    106.0f + 122.0f * t, 106.0f + 122.0f * t,
                    22.0f, 0, color, focus);
        }
    }
    }
}
