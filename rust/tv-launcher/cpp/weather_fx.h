/*
 * weather_fx -- the living sky of the Weather app (C ABI).
 *
 * The Rust launcher owns the screen, text, data and input. This module
 * renders the parts that are pure pixel work: the sky backdrop and the
 * animated weather (clouds, rain, snow, fog, stars, lightning, sun rays and
 * the animated hero icon).
 *
 * Pixels are the launcher's scanout format: B,G,R,A bytes, premultiplied.
 * All coordinates here are physical pixels.
 *
 * Threading: wfx_render_rows / wfx_render_sky_rows only read the scene and
 * may run on several threads at once for disjoint row ranges.
 * wfx_create, wfx_set_layers, wfx_advance and wfx_destroy must not overlap
 * with any other call on the same scene.
 *
 * Built freestanding: no C++ runtime, no exceptions, no libm, no system
 * headers; only malloc/free/memcpy from the C library the launcher links.
 */
#ifndef TV_WEATHER_FX_H
#define TV_WEATHER_FX_H

/* No system headers: the module is built with -nostdinc so a cross build
 * can never pick up the host's C library. The compiler knows the types. */
typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __INT16_TYPE__ int16_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

#ifdef __cplusplus
extern "C" {
#endif

enum WfxSky {
    WFX_CLEAR = 0,
    WFX_PARTLY_CLOUDY = 1,
    WFX_CLOUDY = 2,
    WFX_FOG = 3,
    WFX_DRIZZLE = 4,
    WFX_RAIN = 5,
    WFX_SNOW = 6,
    WFX_THUNDER = 7
};

typedef struct WfxParams {
    int32_t width;      /* full frame, physical px */
    int32_t height;
    int32_t band_h;     /* animated rows, from the top */
    float scale;        /* physical px per logical px */
    int32_t sky;        /* enum WfxSky */
    int32_t day;        /* 1 day, 0 night */
    float intensity;    /* 0..1 strength of rain / snow */
    float wind;         /* km/h: drift and slant */
    float golden;       /* 0..1 sunrise / sunset warmth */
    uint32_t sky_top;   /* 0xRRGGBB, the launcher's palette */
    uint32_t sky_bottom;
    uint32_t seed;
    float icon_x;       /* hero icon centre and size, physical px */
    float icon_y;
    float icon_size;
    float scrim[4];     /* x0, y0, x1, y1: the weather is calmed behind
                           free-floating text there (all 0: none) */
} WfxParams;

typedef struct WfxScene WfxScene;

/* NULL when out of memory or the parameters make no sense. */
WfxScene *wfx_create(const WfxParams *p);
void wfx_destroy(WfxScene *s);

/* The still sky (gradient, glow, vignette, dithering) for rows [y0, y1).
 * `dst` points at row y0. */
void wfx_render_sky_rows(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t y0, int32_t y1);

/* The finished sky (whole frame; must stay alive while the scene renders)
 * and the UI layer (transparent, band_h rows, drawn over the weather; only
 * read during this call: it is folded together with the frosted glass and
 * the calm behind the title into one precomputed layer). */
int32_t wfx_set_layers(WfxScene *s, const uint8_t *sky, int32_t sky_stride, const uint8_t *ui,
                       int32_t ui_stride);

/* Moves every animated element to time t (seconds since the weather
 * started moving; it fades in over the first 1.6 s). */
void wfx_advance(WfxScene *s, float t);

/* Composes rows [y0, y1) of the animated band: sky, weather, UI, icon.
 * `dst` points at row y0. */
void wfx_render_rows(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t y0, int32_t y1);

/* The hero icon alone at time t into any premultiplied buffer (used while
 * the screen slides in, before the band is animated). */
void wfx_draw_icon(const WfxScene *s, uint8_t *dst, int32_t stride, int32_t w, int32_t h, float cx,
                   float cy, float t);

#ifdef __cplusplus
}
#endif

#endif
