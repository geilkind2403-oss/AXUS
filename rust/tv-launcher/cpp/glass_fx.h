/*
 * glass_fx -- the fast menu (control centre) as liquid glass (C ABI).
 *
 * The menu floats over Home (or over a playing video) like a
 * picture-in-picture panel. Rust draws Home and the menu's text and icons;
 * this module draws the glass: the Home picture behind the panel blurred and
 * bent at the edges like a lens, a light rim, glass tiles, the soft shadow,
 * and the liquid focus highlight that glides between the tiles.
 *
 * Built for instant reaction: everything that does not move is composed once
 * (glass_build). A frame redraws only the rectangle the highlight moved
 * through -- a few tens of thousands of pixels.
 *
 * Pixels: B,G,R,A bytes, premultiplied. Rectangles are in logical pixels of
 * the menu's region (its top left is 0,0); `scale` makes them physical.
 * Freestanding like weather_fx: no C++ runtime, no system headers.
 */
#ifndef TV_GLASS_FX_H
#define TV_GLASS_FX_H

typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

#ifdef __cplusplus
extern "C" {
#endif

enum GlassKind {
    GLASS_TILE = 0,        /* a button */
    GLASS_TILE_ACTIVE = 1, /* a switch that is on */
    GLASS_INSET = 2        /* a status well (clock, network) */
};

typedef struct GlassRect {
    float x, y, w, h, r;
    int32_t kind;
} GlassRect;

typedef struct GlassParams {
    int32_t width;  /* the region, physical px */
    int32_t height;
    float scale;
    int32_t over_video; /* 1: nothing to blur behind; tinted glass */
    GlassRect panel;
} GlassParams;

typedef struct GlassScene GlassScene;

GlassScene *glass_create(const GlassParams *p);
void glass_destroy(GlassScene *s);

/* What is behind the region (copied). NULL: transparent (video plane). */
int32_t glass_set_backdrop(GlassScene *s, const uint8_t *px, int32_t stride);
int32_t glass_set_tiles(GlassScene *s, const GlassRect *tiles, int32_t n);
/* Text and icons (transparent, copied). */
int32_t glass_set_content(GlassScene *s, const uint8_t *px, int32_t stride);
/* Composes the still glass after any setter: begin (the blur), the rows
 * (may run on several threads for disjoint rows), end. glass_build does
 * all three on one thread. */
int32_t glass_build(GlassScene *s);
int32_t glass_build_begin(GlassScene *s);
void glass_build_rows(const GlassScene *s, int32_t y0, int32_t y1);
int32_t glass_build_end(GlassScene *s);

/* Draws the region rectangle [x0,x1) x [y0,y1) (logical) into dst (the
 * region's top left): glass, the highlight `focus` (NULL: none), content.
 * `opacity` < 1 fades the menu over the backdrop (opening). */
void glass_render(const GlassScene *s, uint8_t *dst, int32_t stride, const GlassRect *focus, float opacity,
                  float x0, float y0, float x1, float y1);

#ifdef __cplusplus
}
#endif

#endif
