// --- Bild: TV-style picture settings (Bildmodus, Helligkeit, Kontrast,
// Farbsättigung, Farbtemperatur, Schärfe, Augenschonend) for everything on
// screen -- the launcher itself and whatever the content view shows (films,
// Live TV, Netflix, YouTube, the browser). The rows live in the "Bild"
// category of Settings (markup: index.html, wiring: settings.js); this file is
// the model and the one place that turns the settings into a filter.
//
// How it is applied: one CSS `filter` on the root element,
//   brightness() contrast() saturate() [url(#__tv_picture__)]
// The url() part only exists when it is needed: a tiny inline SVG filter with
// a feColorMatrix for the white point (Farbtemperatur, Augenschonend) and a
// 3x3 feConvolveMatrix for Schärfe. All of it runs in the compositor on the
// GPU -- no per-frame JavaScript. Standard with Augenschonend off is no filter
// at all. (The SVG must be in the page itself: a data: URL filter would be
// blocked by the Content-Security-Policy of many sites.)
//
// The launcher applies it to its own <html> here. The same payload goes to the
// main process (window.launcher.setPicture), which hands it to every page the
// content view loads; content-preload.js applies it there the same way.
//
// Persisted in prefs.picture ({ mode, brightness, contrast, saturation,
// temperature, sharpness, eyeCare }) and prefs.pictureCustom (the values of
// "Benutzerdefiniert", kept while a preset is selected, so stepping back to it
// restores them). Every savePrefs() re-applies the filter if it changed
// (wrapper below, same pattern as tvos.js), so code elsewhere only has to edit
// prefs.picture and save.
(() => {
  'use strict';

  const FILTER_ID = '__tv_picture__';

  const MODES = [
    { id: 'standard', label: 'Standard' },
    { id: 'vivid', label: 'Lebendig' },
    { id: 'cinema', label: 'Kino' },
    { id: 'sport', label: 'Sport' },
    { id: 'natural', label: 'Natürlich' }
  ];
  const CUSTOM = { id: 'custom', label: 'Benutzerdefiniert' };

  // brightness / contrast / saturation: 0..100 in steps of 5, 50 = unchanged.
  // temperature: an id from TEMPERATURES. sharpness: index into SHARPNESS.
  // Schärfe is the only part with a real cost (a 3x3 convolution over the
  // whole screen: next to nothing on a GPU, about a quarter of the frame rate
  // at 1080p without one), so no preset uses it -- it is a manual choice. The
  // presets only need colour passes, which cost nothing measurable.
  const PRESETS = {
    standard: { brightness: 50, contrast: 50, saturation: 50, temperature: 'neutral', sharpness: 0 },
    vivid: { brightness: 55, contrast: 65, saturation: 75, temperature: 'cool', sharpness: 0 },
    cinema: { brightness: 45, contrast: 55, saturation: 50, temperature: 'warm2', sharpness: 0 },
    sport: { brightness: 60, contrast: 60, saturation: 60, temperature: 'cool', sharpness: 0 },
    natural: { brightness: 50, contrast: 45, saturation: 40, temperature: 'warm1', sharpness: 0 }
  };

  // R/G/B gains of the white point, roughly 8000 K / 6500 K / 5500 K / 4700 K.
  // Gains never go above 1: like on a TV, "kalt" takes red away rather than
  // pushing blue past white.
  const TEMPERATURES = [
    { id: 'cool', label: 'Kalt', gains: [0.89, 0.93, 1] },
    { id: 'neutral', label: 'Neutral', gains: [1, 1, 1] },
    { id: 'warm1', label: 'Warm 1', gains: [1, 0.93, 0.85] },
    { id: 'warm2', label: 'Warm 2', gains: [1, 0.87, 0.73] }
  ];
  // Augenschonend (less blue light), multiplied on top of the temperature.
  const EYE_CARE_GAINS = [1, 0.88, 0.68];
  // Centre-weighted 3x3 sharpen kernel strength per step.
  const SHARPNESS = [
    { label: 'Aus', k: 0 },
    { label: 'Niedrig', k: 0.12 },
    { label: 'Mittel', k: 0.25 },
    { label: 'Hoch', k: 0.4 }
  ];
  const SCALE_KEYS = ['brightness', 'contrast', 'saturation'];

  // ------------------------------------------------------------ model
  const scaleValue = (v, fallback) => {
    const n = Math.round(Number(v) / 5) * 5;
    return Number.isFinite(n) ? Math.max(0, Math.min(100, n)) : fallback;
  };

  function normalizeValues(v, base) {
    const src = v && typeof v === 'object' ? v : {};
    const out = {};
    SCALE_KEYS.forEach((k) => { out[k] = scaleValue(src[k], base[k]); });
    out.temperature = TEMPERATURES.some((t) => t.id === src.temperature) ? src.temperature : base.temperature;
    const s = Number(src.sharpness);
    out.sharpness = Number.isInteger(s) && s >= 0 && s < SHARPNESS.length ? s : base.sharpness;
    return out;
  }

  function normalize(p) {
    const src = p && typeof p === 'object' ? p : {};
    const mode = src.mode === CUSTOM.id || PRESETS[src.mode] ? src.mode : 'standard';
    // A preset always means exactly its values; only "custom" keeps its own.
    const values = mode === CUSTOM.id ? normalizeValues(src, PRESETS.standard) : { ...PRESETS[mode] };
    return { mode, ...values, eyeCare: src.eyeCare === true };
  }

  // prefs (renderer.js) may come from an older version without these keys.
  prefs.picture = normalize(prefs.picture);
  prefs.pictureCustom = prefs.pictureCustom ? normalizeValues(prefs.pictureCustom, PRESETS.standard) : null;
  if (prefs.picture.mode === CUSTOM.id) prefs.pictureCustom = pickValues(prefs.picture);

  function pickValues(p) {
    return { brightness: p.brightness, contrast: p.contrast, saturation: p.saturation, temperature: p.temperature, sharpness: p.sharpness };
  }

  const current = () => prefs.picture;
  const hasCustom = () => !!prefs.pictureCustom;

  function modeLabel(id) {
    if (id === CUSTOM.id) return CUSTOM.label;
    const m = MODES.find((x) => x.id === id);
    return m ? m.label : '';
  }

  // Selecting a mode: a preset loads its values; "custom" brings back the
  // values the user made (if there are none yet, it is not offered).
  function setMode(id) {
    const p = current();
    if (id === CUSTOM.id) {
      if (!prefs.pictureCustom) return;
      prefs.picture = { ...p, mode: CUSTOM.id, ...prefs.pictureCustom };
    } else if (PRESETS[id]) {
      prefs.picture = { ...p, mode: id, ...PRESETS[id] };
    } else {
      return;
    }
    savePrefs(prefs);
  }

  // Changing a single value turns the current mode into "Benutzerdefiniert".
  function setValue(key, value) {
    const p = current();
    const next = normalizeValues({ ...pickValues(p), [key]: value }, p);
    if (next[key] === p[key] && p.mode === CUSTOM.id) return;
    prefs.picture = { ...p, mode: CUSTOM.id, ...next };
    prefs.pictureCustom = pickValues(prefs.picture);
    savePrefs(prefs);
  }

  function setEyeCare(on) {
    prefs.picture = { ...current(), eyeCare: !!on };
    savePrefs(prefs);
  }

  // Zurücksetzen: back to Standard, Augenschonend off, custom values dropped.
  function reset() {
    prefs.picture = normalize(null);
    prefs.pictureCustom = null;
    savePrefs(prefs);
  }

  function summary() {
    const p = current();
    return [modeLabel(p.mode), p.eyeCare ? 'Augenschonend' : ''].filter(Boolean).join(' · ');
  }

  // ------------------------------------------------------------ filter
  const fmt = (n) => String(Math.round(n * 1000) / 1000);

  // { filter, matrix, kernel } -- or null for "no filter at all".
  //   filter: the CSS filter value for the root element
  //   matrix: feColorMatrix values (white point), '' when neutral
  //   kernel: feConvolveMatrix kernel (Schärfe), '' when off
  function build(p) {
    const b = 1 + ((p.brightness - 50) / 50) * 0.4; // 0.6 .. 1.4
    const c = 1 + ((p.contrast - 50) / 50) * 0.5; // 0.5 .. 1.5
    const s = p.saturation / 50; // 0 (grey) .. 2
    const temp = (TEMPERATURES.find((t) => t.id === p.temperature) || TEMPERATURES[1]).gains;
    const eye = p.eyeCare ? EYE_CARE_GAINS : [1, 1, 1];
    const gains = temp.map((g, i) => g * eye[i]);
    const k = (SHARPNESS[p.sharpness] || SHARPNESS[0]).k;

    const parts = [];
    if (b !== 1) parts.push(`brightness(${fmt(b)})`);
    if (c !== 1) parts.push(`contrast(${fmt(c)})`);
    if (s !== 1) parts.push(`saturate(${fmt(s)})`);
    const matrix = gains.some((g) => g !== 1)
      ? `${fmt(gains[0])} 0 0 0 0 0 ${fmt(gains[1])} 0 0 0 0 0 ${fmt(gains[2])} 0 0 0 0 0 1 0`
      : '';
    const kernel = k ? `0 ${fmt(-k)} 0 ${fmt(-k)} ${fmt(1 + 4 * k)} ${fmt(-k)} 0 ${fmt(-k)} 0` : '';
    if (matrix || kernel) parts.push(`url(#${FILTER_ID})`);
    return parts.length ? { filter: parts.join(' '), matrix, kernel } : null;
  }

  // The SVG half of the filter, in this document (content-preload.js has the
  // same for web pages). Removed again when nothing needs it.
  const SVG_NS = 'http://www.w3.org/2000/svg';
  function stampSvg(payload) {
    let svg = document.getElementById(`${FILTER_ID}svg`);
    if (!payload || (!payload.matrix && !payload.kernel)) {
      if (svg) svg.remove();
      return;
    }
    if (!svg) {
      svg = document.createElementNS(SVG_NS, 'svg');
      svg.id = `${FILTER_ID}svg`;
      svg.setAttribute('width', '0');
      svg.setAttribute('height', '0');
      svg.setAttribute('aria-hidden', 'true');
      svg.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden;pointer-events:none;';
      document.body.appendChild(svg);
    }
    const filter = document.createElementNS(SVG_NS, 'filter');
    filter.id = FILTER_ID;
    filter.setAttribute('color-interpolation-filters', 'sRGB');
    if (payload.matrix) {
      const m = document.createElementNS(SVG_NS, 'feColorMatrix');
      m.setAttribute('type', 'matrix');
      m.setAttribute('values', payload.matrix);
      filter.appendChild(m);
    }
    if (payload.kernel) {
      const cm = document.createElementNS(SVG_NS, 'feConvolveMatrix');
      cm.setAttribute('order', '3');
      cm.setAttribute('kernelMatrix', payload.kernel);
      cm.setAttribute('divisor', '1');
      cm.setAttribute('edgeMode', 'duplicate');
      cm.setAttribute('preserveAlpha', 'true');
      filter.appendChild(cm);
    }
    svg.replaceChildren(filter);
  }

  let appliedKey = null;
  function apply() {
    const payload = build(current());
    const key = JSON.stringify(payload);
    if (key === appliedKey) return; // other prefs were saved: nothing to do
    appliedKey = key;
    stampSvg(payload);
    document.documentElement.style.filter = payload ? payload.filter : '';
    try {
      Promise.resolve(window.launcher.setPicture(payload)).catch(() => {});
    } catch { /* no bridge (plain browser): the launcher itself still changes */ }
  }

  const baseSavePrefs = window.savePrefs;
  if (typeof baseSavePrefs === 'function') {
    window.savePrefs = function savePrefs() {
      const result = baseSavePrefs.apply(this, arguments);
      apply();
      return result;
    };
  }

  // At load, i.e. long before the boot splash lifts; the main process gets it
  // before any web app can open.
  apply();

  window.picture = {
    MODES,
    CUSTOM,
    TEMPERATURES,
    SHARPNESS,
    current,
    hasCustom,
    modeLabel,
    setMode,
    setValue,
    setEyeCare,
    reset,
    summary,
    build
  };
})();
