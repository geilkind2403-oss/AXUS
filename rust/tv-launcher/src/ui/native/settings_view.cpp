// No system headers: a Pi cross build (clang++ for armv7 musl) would pick
// up the host's C library headers and fail. The compiler knows the types.
typedef __SIZE_TYPE__ size_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __INT32_TYPE__ int32_t;

// Lightweight C++ presentation layer. Rust retains navigation and all system
// operations; this file owns only the Settings layout and focus animation.
namespace {
enum Command : uint32_t { Rect = 0, Stroke = 1, Line = 2, Text = 3, Icon = 4, Clip = 5, TextFit = 6 };
enum Weight : uint32_t { Regular = 0, Semi = 1, Bold = 2 };
enum Align : uint32_t { Left = 0, Center = 1, Right = 2 };

struct TextView { const char* data; size_t size; };
struct RowView { TextView label; TextView value; uint32_t chevron; };
struct Input {
    uint32_t section;
    uint32_t item;
    uint32_t inside;
    uint32_t row_count;
    float dt;
    TextView title;
    TextView hint;
};
struct Animation {
    float row_y;
    float scroll_y;
    uint32_t initialized;
    uint32_t moving;
};
using Emit = void (*)(void*, uint32_t, float, float, float, float, float, float,
                      uint32_t, uint32_t, uint32_t, uint32_t, uint32_t,
                      const char*, size_t);

constexpr uint32_t rgba(uint32_t r, uint32_t g, uint32_t b, uint32_t a) {
    return (r << 24) | (g << 16) | (b << 8) | a;
}
void emit(Emit fn, void* ctx, uint32_t kind, float x, float y, float w, float h,
          float radius, float stroke, uint32_t color, uint32_t px = 0,
          uint32_t weight = Regular, uint32_t align = Left, uint32_t icon = 0,
          const char* value = nullptr, size_t size = 0) {
    fn(ctx, kind, x, y, w, h, radius, stroke, color, px, weight, align, icon, value, size);
}
void rect(Emit fn, void* ctx, float x, float y, float w, float h, float r, uint32_t c) {
    emit(fn, ctx, Rect, x, y, w, h, r, 0, c);
}
void line(Emit fn, void* ctx, float x1, float y1, float x2, float y2, float width, uint32_t c) {
    emit(fn, ctx, Line, x1, y1, x2, y2, 0, width, c);
}
void text(Emit fn, void* ctx, const char* s, size_t n, float x, float y,
          uint32_t px, uint32_t weight, uint32_t color, uint32_t align = Left) {
    emit(fn, ctx, Text, x, y, 0, 0, 0, 0, color, px, weight, align, 0, s, n);
}
void fit(Emit fn, void* ctx, const char* s, size_t n, float x, float y,
         float width, uint32_t px, uint32_t weight, uint32_t color) {
    emit(fn, ctx, TextFit, x, y, width, 0, 0, 0, color, px, weight, Left, 0, s, n);
}
// Sonic-quick: the focus is most of the way there after two frames and
// lands in about 0.15 s (it was 0.4 s), so a press feels instant and fewer
// frames need drawing.
float approach(float value, float target, float dt) {
    const float k = dt / (0.032f + dt);
    const float next = value + (target - value) * k;
    return (target - next < 0.6f && target - next > -0.6f) ? target : next;
}
void icon(Emit fn, void* ctx, uint32_t which, float x, float y, uint32_t c) {
    emit(fn, ctx, Icon, x, y, 0, 0, 0, 0, c, 0, Regular, Left, which);
}
constexpr float RowX = 650.0f;
constexpr float RowW = 570.0f;
constexpr float RowY = 154.0f;
constexpr float RowH = 43.0f;
constexpr float RowStep = 55.0f;
constexpr uint32_t VisibleRows = 9;
} // namespace

extern "C" void tv_settings_draw_shell(const Input* in, Animation* a, void* ctx, Emit fn) {
    const float dt = in->dt < 0.0f ? 0.0f : (in->dt > 0.08f ? 0.08f : in->dt);
    const uint32_t first = in->item >= VisibleRows ? in->item - (VisibleRows - 1) : 0;
    const float scroll_goal = static_cast<float>(first) * RowStep;
    const float row_goal = RowY + static_cast<float>(in->item) * RowStep - scroll_goal;
    if (!a->initialized) {
        a->row_y = row_goal;
        a->scroll_y = scroll_goal;
        a->initialized = 1;
    } else {
        a->row_y = approach(a->row_y, row_goal, dt);
        a->scroll_y = approach(a->scroll_y, scroll_goal, dt);
    }
    a->moving = (a->row_y != row_goal || a->scroll_y != scroll_goal) ? 1u : 0u;

    // Shared right edge: heading, all rows, and the page hint end at x=1220.
    text(fn, ctx, in->title.data, in->title.size, 1220.0f, 70.0f, 34, Bold,
         rgba(250, 247, 239, 255), Right);
    if (in->hint.size != 0) {
        text(fn, ctx, in->hint.data, in->hint.size, 1220.0f, 112.0f, 15, Regular,
             rgba(180, 166, 136, 255), Right);
    }
    constexpr char Help[] = "↑↓ Navigieren    ·    OK Öffnen    ·    Zurück";
    text(fn, ctx, Help, sizeof(Help) - 1,
         1220.0f, 680.0f, 13, Semi, rgba(169, 151, 111, 235), Right);
}

extern "C" void tv_settings_draw_rows(const Input* in, const Animation* a,
                                        const RowView* rows, void* ctx, Emit fn) {
    const bool root = in->section == 9;
    const float scroll = a->scroll_y;
    emit(fn, ctx, Clip, 145.0f, 654.0f, 0, 0, 0, 0, 0);
    for (uint32_t i = 0; i < in->row_count; ++i) {
        const float y = RowY + static_cast<float>(i) * RowStep - scroll;
        if (y + RowH < 145.0f || y > 654.0f) continue;
        const bool focused = root ? i == in->item : (in->inside && i == in->item);
        const float focus_y = focused ? a->row_y : y;
        const uint32_t body = focused ? rgba(76, 57, 25, 92) : rgba(9, 9, 8, 66);
        if (focused) {
            // speed lines: while the focus flies, fading copies trail behind
            // it (away from where it is going), gone the moment it lands
            const float d = y - focus_y;
            const float ad = d < 0.0f ? -d : d;
            if (ad > 3.0f) {
                for (int k = 2; k >= 1; --k) {
                    const float gy = focus_y - d * 0.22f * static_cast<float>(k);
                    const uint32_t ga = static_cast<uint32_t>(
                        (ad > 55.0f ? 1.0f : ad / 55.0f) * (44.0f - 14.0f * static_cast<float>(k)));
                    rect(fn, ctx, RowX + 6.0f * static_cast<float>(k), gy,
                         RowW - 12.0f * static_cast<float>(k), RowH, 15.0f,
                         rgba(226, 187, 116, ga));
                }
            }
            rect(fn, ctx, RowX, focus_y, RowW, RowH, 15.0f, body);
            // the ring only once it has landed (a rounded stroke is costly
            // to draw, and in flight the trail says enough)
            if (ad <= 3.0f) emit(fn, ctx, Stroke, RowX + 0.5f, focus_y + 0.5f, RowW - 1.0f, RowH - 1.0f,
                 15.0f, 1.5f, rgba(226, 187, 116, 170));
            // a gold bar on the left edge says "you are here"
            rect(fn, ctx, RowX + 7.0f, focus_y + 11.0f, 3.0f, RowH - 22.0f, 1.5f,
                 rgba(255, 218, 137, 235));
        } else {
            rect(fn, ctx, RowX, focus_y, RowW, RowH, 15.0f, body);
        }
        const float label_x = root ? RowX + 62.0f : RowX + 22.0f;
        if (root) {
            icon(fn, ctx, i, RowX + 36.0f, y + RowH * 0.5f,
                 focused ? rgba(255, 218, 137, 255) : rgba(205, 171, 105, 200));
        }
        fit(fn, ctx, rows[i].label.data, rows[i].label.size,
            label_x, y + 11.0f, root ? RowW - 102.0f : 280.0f, 18,
            focused ? Semi : Regular,
            focused ? rgba(255, 250, 236, 255) : rgba(238, 233, 223, 255));
        if (!root && rows[i].value.size != 0) {
            fit(fn, ctx, rows[i].value.data, rows[i].value.size,
                RowX + 310.0f, y + 12.0f, rows[i].chevron ? 205.0f : 235.0f, 15, Regular,
                focused ? rgba(244, 211, 142, 255) : rgba(185, 174, 151, 255));
        }
        // The arrow only marks rows that open something (a list, a keyboard);
        // actions and choices have none.
        if (rows[i].chevron) {
            const float cx = RowX + RowW - 25.0f;
            const float cy = focus_y + RowH * 0.5f;
            const uint32_t chevron = focused ? rgba(255, 218, 137, 255) : rgba(205, 171, 105, 220);
            line(fn, ctx, cx - 3.0f, cy - 6.0f, cx + 3.0f, cy, 2.0f, chevron);
            line(fn, ctx, cx + 3.0f, cy, cx - 3.0f, cy + 6.0f, 2.0f, chevron);
        }
    }
    emit(fn, ctx, Clip, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1);
    if (in->row_count > VisibleRows) {
        rect(fn, ctx, RowX + RowW - 3.0f, 153.0f, 2.0f, 484.0f, 1.0f,
             rgba(192, 157, 91, 35));
        const float thumb = 484.0f * static_cast<float>(VisibleRows) / static_cast<float>(in->row_count);
        const float max_scroll = static_cast<float>(in->row_count - VisibleRows) * RowStep;
        const float y = 153.0f + (max_scroll > 0.0f ? scroll / max_scroll : 0.0f) * (484.0f - thumb);
        rect(fn, ctx, RowX + RowW - 3.0f, y, 2.0f, thumb, 1.0f,
             rgba(226, 187, 116, 170));
    }
}
