// --- Wetter: eigenes Vollbild-Menü (Tile / top-bar pill -> openAction
// 'weather'). Everything is keyless public data:
//   - Open-Meteo forecast: current, 16 days hourly/daily, 15-min nowcast
//   - Open-Meteo air quality: European AQI, particulates, pollen
//   - Bright Sky: the official DWD weather warnings for the location
//   - DWD GeoServer WMS: German radar composite (RV product: analysis +
//     2 h nowcast, 1 km, 5 min) and Bundesland borders / warned districts
//   - CARTO dark basemap tiles underneath
// Loaded after renderer.js and uses its globals (MURR_LAT/LON,
// animatedOpen/Close, resetIdleTimer, weatherCodeToCondition).
(() => {
  const LAT = MURR_LAT;
  const LON = MURR_LON;
  const $ = (id) => document.getElementById(id);
  const pad2 = (n) => String(n).padStart(2, '0');
  const round = Math.round;
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const WD_SHORT = ['So', 'Mo', 'Di', 'Mi', 'Do', 'Fr', 'Sa'];
  const WD_LONG = ['Sonntag', 'Montag', 'Dienstag', 'Mittwoch', 'Donnerstag', 'Freitag', 'Samstag'];
  const MONTHS = ['Januar', 'Februar', 'März', 'April', 'Mai', 'Juni', 'Juli', 'August', 'September', 'Oktober', 'November', 'Dezember'];
  const fmtTime = hhmm; // renderer.js
  const fmtNum = (x, digits = 1) => Number(x).toFixed(digits).replace('.', ',');
  // Warning texts come from the network and go into innerHTML.
  const esc = escapeHtml; // renderer.js

  // ---------------------------------------------------------------- icons
  const CLOUD_PATH = 'M18 50H46A10 10 0 0 0 46 30A14 14 0 0 0 19.5 26A12 12 0 0 0 18 50Z';

  function sunSvg(cx, cy, r) {
    let rays = '';
    for (let a = 0; a < 360; a += 45) {
      const rad = a * Math.PI / 180;
      rays += `<line x1="${(cx + Math.cos(rad) * (r + 4)).toFixed(1)}" y1="${(cy + Math.sin(rad) * (r + 4)).toFixed(1)}" x2="${(cx + Math.cos(rad) * (r + 8.5)).toFixed(1)}" y2="${(cy + Math.sin(rad) * (r + 8.5)).toFixed(1)}"/>`;
    }
    return `<circle cx="${cx}" cy="${cy}" r="${r}" fill="#FFCB45"/><g stroke="#FFCB45" stroke-width="3.2" stroke-linecap="round">${rays}</g>`;
  }
  function moonSvg(cx, cy, r) {
    // Crescent: long way round the outer circle, back along a smaller arc.
    const p1 = [cx + 0.174 * r, cy - 0.985 * r];
    const p2 = [cx + 0.985 * r, cy + 0.174 * r];
    const f = (p) => `${p[0].toFixed(1)} ${p[1].toFixed(1)}`;
    return `<path d="M${f(p1)} A${r} ${r} 0 1 0 ${f(p2)} A${(0.78 * r).toFixed(1)} ${(0.78 * r).toFixed(1)} 0 0 1 ${f(p1)} Z" fill="#E7E5FF"/>`;
  }
  const cloud = (fill, transform = '') => `<path d="${CLOUD_PATH}" fill="${fill}"${transform ? ` transform="${transform}"` : ''}/>`;

  function iconKind(code) {
    if (code === 0) return 'clear';
    if (code === 1 || code === 2) return 'partly';
    if (code === 3) return 'cloudy';
    if (code === 45 || code === 48) return 'fog';
    if (code >= 51 && code <= 57) return 'drizzle';
    if (code === 65 || code === 67 || code === 82) return 'heavy';
    if ((code >= 61 && code <= 67) || (code >= 80 && code <= 82)) return 'rain';
    if ((code >= 71 && code <= 77) || code === 85 || code === 86) return 'snow';
    if (code >= 95) return 'thunder';
    return 'cloudy';
  }

  function wxIcon(code, isDay = true) {
    const kind = iconKind(code);
    const celestial = (cx, cy, r) => (isDay ? sunSvg(cx, cy, r) : moonSvg(cx, cy, r));
    let body;
    switch (kind) {
      case 'clear': body = celestial(32, 32, 13); break;
      case 'partly': body = celestial(23, 22, 10) + cloud('#E4E8EF', 'translate(9 7) scale(0.84)'); break;
      case 'cloudy': body = cloud('#8D96A7', 'translate(12 -4) scale(0.78)') + cloud('#DDE2EA', 'translate(-2 4)'); break;
      case 'fog':
        body = cloud('#C9CFD9', 'translate(0 -8)') +
          '<g stroke="#AEB6C4" stroke-width="3.5" stroke-linecap="round"><line x1="12" y1="49" x2="52" y2="49"/><line x1="18" y1="57" x2="46" y2="57"/></g>';
        break;
      case 'drizzle':
        body = cloud('#C3CAD6', 'translate(0 -7)') +
          '<g stroke="#6DB4FF" stroke-width="3" stroke-linecap="round"><line x1="23" y1="49" x2="22" y2="53"/><line x1="33" y1="52" x2="32" y2="56"/><line x1="43" y1="49" x2="42" y2="53"/></g>';
        break;
      case 'rain':
        body = cloud('#B5BDCA', 'translate(0 -7)') +
          '<g stroke="#5AA9FF" stroke-width="3.2" stroke-linecap="round"><line x1="24" y1="48" x2="21" y2="57"/><line x1="34" y1="48" x2="31" y2="57"/><line x1="44" y1="48" x2="41" y2="57"/></g>';
        break;
      case 'heavy':
        body = cloud('#98A1B1', 'translate(0 -8)') +
          '<g stroke="#4A9BFF" stroke-width="3.2" stroke-linecap="round"><line x1="20" y1="47" x2="16" y2="60"/><line x1="29" y1="47" x2="25" y2="60"/><line x1="38" y1="47" x2="34" y2="60"/><line x1="47" y1="47" x2="43" y2="60"/></g>';
        break;
      case 'snow':
        body = cloud('#C9CFD9', 'translate(0 -7)') +
          '<g fill="#FFFFFF"><circle cx="22" cy="51" r="2.8"/><circle cx="32" cy="57" r="2.8"/><circle cx="42" cy="51" r="2.8"/></g>';
        break;
      case 'thunder':
        body = cloud('#8D96A7', 'translate(0 -8)') +
          '<path d="M35 38 L26 52 H33 L29 62 L42 46 H35 L39 38 Z" fill="#FFCB45"/>';
        break;
    }
    return `<svg viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg">${body}</svg>`;
  }
  window.wxIcon = wxIcon;

  const WMO_TEXT = {
    0: 'Klar', 1: 'Überwiegend klar', 2: 'Teils bewölkt', 3: 'Bedeckt',
    45: 'Nebel', 48: 'Reifnebel',
    51: 'Leichter Nieselregen', 53: 'Nieselregen', 55: 'Starker Nieselregen', 56: 'Gefrierender Niesel', 57: 'Gefrierender Niesel',
    61: 'Leichter Regen', 63: 'Regen', 65: 'Starker Regen', 66: 'Gefrierender Regen', 67: 'Gefrierender Regen',
    71: 'Leichter Schneefall', 73: 'Schneefall', 75: 'Starker Schneefall', 77: 'Schneegriesel',
    80: 'Leichte Regenschauer', 81: 'Regenschauer', 82: 'Heftige Regenschauer', 85: 'Schneeschauer', 86: 'Starke Schneeschauer',
    95: 'Gewitter', 96: 'Gewitter mit Hagel', 99: 'Schweres Gewitter'
  };
  function wmoText(code, isDay = true) {
    if (code === 0) return isDay ? 'Sonnig' : 'Klar';
    if (code === 1) return isDay ? 'Überwiegend sonnig' : 'Überwiegend klar';
    return WMO_TEXT[code] || '–';
  }

  // Colour for a temperature, for the 14-day range bars.
  const TEMP_STOPS = [[-10, [120, 160, 255]], [0, [127, 178, 255]], [10, [110, 220, 205]], [18, [170, 226, 100]], [24, [255, 210, 90]], [30, [255, 138, 76]], [36, [255, 80, 80]]];
  function tempColor(t) {
    for (let i = 1; i < TEMP_STOPS.length; i++) {
      const [t1, c1] = TEMP_STOPS[i];
      const [t0, c0] = TEMP_STOPS[i - 1];
      if (t <= t1) {
        const k = clamp((t - t0) / (t1 - t0), 0, 1);
        return `rgb(${c0.map((v, j) => round(v + (c1[j] - v) * k)).join(',')})`;
      }
    }
    return `rgb(${TEMP_STOPS[TEMP_STOPS.length - 1][1].join(',')})`;
  }

  const compass = (deg) => ['N', 'NO', 'O', 'SO', 'S', 'SW', 'W', 'NW'][round(deg / 45) % 8];
  function uvColor(uv) {
    if (uv < 3) return '#5ADB6B';
    if (uv < 6) return '#F4E04D';
    if (uv < 8) return '#FF9F3A';
    if (uv < 11) return '#FF4D4D';
    return '#B45CFF';
  }
  function uvLabel(uv) {
    if (uv < 3) return 'Niedrig';
    if (uv < 6) return 'Mäßig';
    if (uv < 8) return 'Hoch';
    if (uv < 11) return 'Sehr hoch';
    return 'Extrem';
  }

  // ---------------------------------------------------------------- data
  const FORECAST_URL = `https://api.open-meteo.com/v1/forecast?latitude=${LAT}&longitude=${LON}&timezone=Europe%2FBerlin&forecast_days=16` +
    '&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,wind_gusts_10m,wind_direction_10m,pressure_msl,cloud_cover,precipitation,is_day,uv_index,visibility,dew_point_2m' +
    '&hourly=temperature_2m,precipitation_probability,precipitation,weather_code,wind_speed_10m,wind_gusts_10m,is_day,pressure_msl' +
    '&daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,precipitation_sum,precipitation_probability_max,precipitation_hours,wind_speed_10m_max,wind_gusts_10m_max,wind_direction_10m_dominant,uv_index_max,sunshine_duration,daylight_duration' +
    '&minutely_15=precipitation&forecast_minutely_15=16';
  const AIR_URL = `https://air-quality-api.open-meteo.com/v1/air-quality?latitude=${LAT}&longitude=${LON}&timezone=Europe%2FBerlin&forecast_days=2` +
    '&current=european_aqi,pm10,pm2_5,ozone,nitrogen_dioxide,uv_index' +
    '&hourly=alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,ragweed_pollen';
  const ALERTS_URL = `https://api.brightsky.dev/alerts?lat=${LAT}&lon=${LON}`;

  let data = null;
  let fetchedAt = 0;
  let loading = null;

  const getJson = (url) => fetch(url, { cache: 'no-store' }).then((r) => {
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    return r.json();
  });

  function loadData(force = false) {
    if (!force && data && Date.now() - fetchedAt < 10 * 60 * 1000) return Promise.resolve(data);
    if (loading) return loading;
    loading = Promise.allSettled([getJson(FORECAST_URL), getJson(AIR_URL), getJson(ALERTS_URL)]).then(([f, a, w]) => {
      loading = null;
      if (f.status !== 'fulfilled') throw f.reason;
      data = {
        f: f.value,
        air: a.status === 'fulfilled' ? a.value : null,
        alerts: w.status === 'fulfilled' ? (w.value.alerts || []) : null
      };
      fetchedAt = Date.now();
      return data;
    }, (err) => { loading = null; throw err; });
    return loading;
  }

  function hourIndexNow(f) {
    const key = f.current.time.slice(0, 13) + ':00';
    const i = f.hourly.time.indexOf(key);
    return i >= 0 ? i : 0;
  }
  function dayStartIndex(f, di) {
    return f.hourly.time.indexOf(f.daily.time[di] + 'T00:00');
  }
  function dominantCode(codes) {
    const wet = codes.filter((c) => c >= 51);
    if (wet.length >= 2) return Math.max(...wet);
    const counts = {};
    codes.forEach((c) => { counts[c] = (counts[c] || 0) + 1; });
    return Number(Object.keys(counts).sort((a, b) => counts[b] - counts[a] || b - a)[0]);
  }
  // Open-Meteo's daily weather_code is the day's *worst* hour -- one cloudy
  // hour at 3 am makes a sunny day "Bedeckt". What matters for the day's
  // icon/text is what it's like while people are awake, so it's derived
  // from the hourly codes 07-19 Uhr instead.
  function dayCode(f, di) {
    const base = dayStartIndex(f, di);
    if (base < 0) return f.daily.weather_code[di];
    const codes = [];
    for (let hh = 7; hh <= 19; hh++) if (base + hh < f.hourly.time.length) codes.push(f.hourly.weather_code[base + hh]);
    return codes.length ? dominantCode(codes) : f.daily.weather_code[di];
  }
  function dayLabel(f, di) {
    const d = new Date(f.daily.time[di] + 'T12:00');
    if (di === 0) return 'Heute';
    if (di === 1) return 'Morgen';
    return WD_SHORT[d.getDay()];
  }
  const dayDate = (f, di) => new Date(f.daily.time[di] + 'T12:00');

  // ---------------------------------------------------------------- texts
  function nowSummary(f) {
    const m = f.minutely_15;
    const now = Date.now();
    const slots = m.time.map((t, i) => ({ t: new Date(t), p: m.precipitation[i] || 0 }))
      .filter((s) => s.t.getTime() + 15 * 60000 > now);
    const wet = (s) => s.p >= 0.05;
    if (slots.length && wet(slots[0])) {
      const stop = slots.find((s) => !wet(s));
      return stop ? `Es regnet gerade – hört voraussichtlich gegen ${fmtTime(stop.t)} auf.` : 'Es regnet gerade – und das bleibt die nächsten Stunden so.';
    }
    const start = slots.find(wet);
    if (start) {
      const mins = Math.max(5, round((start.t - now) / 60000 / 5) * 5);
      return `Regen in etwa ${mins} Minuten (gegen ${fmtTime(start.t)}).`;
    }
    const i0 = hourIndexNow(f);
    for (let i = i0 + 1; i < Math.min(i0 + 13, f.hourly.time.length); i++) {
      if (f.hourly.precipitation_probability[i] >= 50 && f.hourly.precipitation[i] >= 0.1) {
        return `Vorerst trocken – Regen ab etwa ${new Date(f.hourly.time[i]).getHours()} Uhr (${f.hourly.precipitation_probability[i]} %).`;
      }
    }
    return 'Trocken in den nächsten 12 Stunden.';
  }
  // Tile-sized sibling of nowSummary: one short line for the home tile's glass
  // bar (~30 characters at most, the bar is only ~200 px wide). It answers the
  // one question people have when they glance at a weather tile: will I get wet?
  function homeLine(f) {
    const m = f.minutely_15;
    const now = Date.now();
    // Hourly codes tell snow from rain from thunder, the 15-min nowcast does not.
    const noun = (code) => ((code >= 71 && code <= 77) || code === 85 || code === 86 ? 'Schnee' : code >= 95 ? 'Gewitter' : 'Regen');
    const i0 = hourIndexNow(f);
    const slots = m.time.map((t, i) => ({ t: new Date(t), key: `${t.slice(0, 13)}:00`, p: m.precipitation[i] || 0 }))
      .filter((s) => s.t.getTime() + 15 * 60000 > now);
    const wet = (s) => s.p >= 0.05;
    if (slots.length && wet(slots[0])) {
      const stop = slots.find((s) => !wet(s));
      const n = noun(f.current.weather_code);
      return stop ? `${n} hört gegen ${fmtTime(stop.t)} auf` : `${n} hält noch länger an`;
    }
    const start = slots.find(wet);
    if (start) {
      const mins = Math.max(5, round((start.t - now) / 60000 / 5) * 5);
      const n = noun(f.hourly.weather_code[f.hourly.time.indexOf(start.key)]);
      return mins < 60 ? `${n} in etwa ${mins} Min.` : `${n} ab etwa ${start.t.getHours()} Uhr`;
    }
    // Beyond the nowcast: first clearly wet hour within the next 24 h. Dates are
    // compared as Berlin strings (the API's timezone), not via the machine's zone.
    const today = f.current.time.slice(0, 10);
    for (let i = i0 + 1; i < Math.min(i0 + 25, f.hourly.time.length); i++) {
      if (f.hourly.precipitation_probability[i] >= 50 && f.hourly.precipitation[i] >= 0.1) {
        const h = Number(f.hourly.time[i].slice(11, 13));
        const sameDay = f.hourly.time[i].slice(0, 10) === today;
        const n = noun(f.hourly.weather_code[i]);
        if (sameDay && h < 22) return `${n} ab etwa ${h} Uhr`;
        if (sameDay || h < 6) return 'Trocken bis heute Nacht';
        return h < 12 ? 'Trocken bis morgen früh' : 'Trocken bis morgen';
      }
    }
    return 'Die nächsten 24 Std. trocken';
  }
  // Everything the home tile and the top-bar pill show, from the same cached
  // forecast the weather menu uses (one request per 10 min for all of them).
  async function homeSummary(force = false) {
    const { f } = await loadData(force);
    let line = '';
    try { line = homeLine(f); } catch { /* a missing nowcast must not take the temperature down with it */ }
    return {
      temp: round(f.current.temperature_2m),
      feels: round(f.current.apparent_temperature),
      code: f.current.weather_code,
      isDay: f.current.is_day === 1,
      hi: round(f.daily.temperature_2m_max[0]),
      lo: round(f.daily.temperature_2m_min[0]),
      line
    };
  }
  function nowSubline(f) {
    const i0 = hourIndexNow(f);
    const tomorrow8 = dayStartIndex(f, 1) + 8;
    const nightTemps = f.hourly.temperature_2m.slice(i0, Math.max(i0 + 1, tomorrow8));
    const hourNow = new Date().getHours();
    const max = round(f.daily.temperature_2m_max[0]);
    const low = round(Math.min(...nightTemps));
    return hourNow < 15
      ? `Heute bis ${max}°, in der Nacht ${low}°.`
      : `Heute Nacht bis ${low}° – morgen ${round(f.daily.temperature_2m_min[1])}° bis ${round(f.daily.temperature_2m_max[1])}°.`;
  }
  function daySummary(f, di) {
    const d = f.daily;
    let s = `${wmoText(dayCode(f, di))}, ${round(d.temperature_2m_min[di])}° bis ${round(d.temperature_2m_max[di])}°.`;
    const prob = d.precipitation_probability_max[di] || 0;
    const sum = d.precipitation_sum[di] || 0;
    if (sum >= 0.2 && prob >= 30) {
      s += ` Regen ${prob >= 70 ? 'sehr wahrscheinlich' : 'möglich'} (${prob} %), insgesamt etwa ${fmtNum(sum, sum < 10 ? 1 : 0)} mm.`;
    } else {
      s += ' Kaum Regen zu erwarten.';
    }
    if (d.wind_gusts_10m_max[di] >= 50) s += ` Stürmische Böen bis ${round(d.wind_gusts_10m_max[di])} km/h.`;
    const sunH = (d.sunshine_duration[di] || 0) / 3600;
    if (sunH >= 7) s += ` Viel Sonne (${round(sunH)} Std.).`;
    return s;
  }

  // ---------------------------------------------------------------- UI bits
  let chartSeq = 0;
  function hourlyChart(f, indices, { labelNow = false } = {}) {
    const h = f.hourly;
    indices = indices.filter((i) => i >= 0 && i < h.time.length);
    const n = indices.length;
    const temps = indices.map((i) => h.temperature_2m[i]);
    const minT = Math.min(...temps);
    const maxT = Math.max(...temps);
    const span = Math.max(1, maxT - minT);
    const CH = 72;
    const TOP = 24;
    const BOTTOM = 10;
    const ys = temps.map((t) => TOP + (maxT - t) / span * (CH - TOP - BOTTOM));
    const pts = indices.map((_, k) => [(k + 0.5) / n * 1000, ys[k]]);
    let d = `M${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
    for (let k = 0; k < n - 1; k++) {
      const p0 = pts[k - 1] || pts[k];
      const p1 = pts[k];
      const p2 = pts[k + 1];
      const p3 = pts[k + 2] || p2;
      const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
      const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
      d += ` C${c1[0].toFixed(1)},${c1[1].toFixed(1)} ${c2[0].toFixed(1)},${c2[1].toFixed(1)} ${p2[0].toFixed(1)},${p2[1].toFixed(1)}`;
    }
    const gid = `wxg${++chartSeq}`;
    const area = `${d} L${pts[n - 1][0].toFixed(1)},${CH} L${pts[0][0].toFixed(1)},${CH} Z`;
    const nights = indices.map((i) => h.is_day[i] === 0);

    const cols = indices.map((i, k) => {
      const t = new Date(h.time[i]);
      const label = labelNow && k === 0 ? 'Jetzt' : pad2(t.getHours());
      const prob = h.precipitation_probability[i] || 0;
      const night = nights[k];
      const cls = ['wx-hr-col', night ? 'night' : '', night && !nights[k - 1] ? 'first' : '', night && !nights[k + 1] ? 'last' : ''].join(' ');
      return `<div class="${cls}" style="--k:${k}">
        <div class="t${labelNow && k === 0 ? ' now' : ''}">${label}</div>
        <div class="i">${wxIcon(h.weather_code[i], h.is_day[i] === 1)}</div>
        <div class="gap"></div>
        <div class="p">${prob >= 10 ? `<div class="bar" style="height:${round(prob / 100 * 24)}px"></div>${prob}%` : '<span class="none">–</span>'}</div>
      </div>`;
    }).join('');
    const labels = indices.map((i, k) => `<span style="--k:${k};left:${((k + 0.5) / n * 100).toFixed(3)}%;top:${(ys[k] - 20).toFixed(1)}px">${round(temps[k])}°</span>`).join('');

    return `<div class="wx-card wx-hourly"><div class="wx-hr-wrap">
      <div class="wx-hr-cols" style="--n:${n}">${cols}</div>
      <svg class="wx-hr-curve" viewBox="0 0 1000 ${CH}" preserveAspectRatio="none">
        <defs><linearGradient id="${gid}" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#FFB547" stop-opacity="0.35"/><stop offset="1" stop-color="#FFB547" stop-opacity="0"/>
        </linearGradient></defs>
        <path d="${area}" fill="url(#${gid})"/>
        <path d="${d}" fill="none" stroke="#FFC45C" stroke-width="2.5" vector-effect="non-scaling-stroke" stroke-linecap="round"/>
      </svg>
      <div class="wx-hr-labels">${labels}</div>
    </div></div>`;
  }

  function tile(label, value, sub, viz = '', cls = '') {
    return `<div class="wx-card wx-tile ${cls}"><div class="lbl">${label}</div><div class="val">${value}</div>${sub ? `<div class="sub">${sub}</div>` : ''}${viz ? `<div class="viz">${viz}</div>` : ''}</div>`;
  }
  function compassSvg(dirFrom) {
    // Arrow points where the wind blows *to* (from + 180).
    return `<svg class="wx-compass" viewBox="0 0 60 60">
      <circle cx="30" cy="30" r="26" fill="none" stroke="rgba(255,255,255,0.14)" stroke-width="2"/>
      <text x="30" y="11" text-anchor="middle" font-size="8" font-weight="800" fill="#8b8b94">N</text>
      <g transform="rotate(${(dirFrom + 180) % 360} 30 30)">
        <path d="M30 13 L37 32 L30 28 L23 32 Z" fill="#fff"/><line x1="30" y1="28" x2="30" y2="46" stroke="#fff" stroke-width="2.5" stroke-linecap="round"/>
      </g></svg>`;
  }
  function sunArc(f, di) {
    const rise = new Date(f.daily.sunrise[di]);
    const set = new Date(f.daily.sunset[di]);
    const now = Date.now();
    const frac = (now - rise) / (set - rise);
    const up = di === 0 && frac >= 0 && frac <= 1;
    const k = clamp(frac, 0, 1);
    const x = 100 - 84 * Math.cos(Math.PI * k);
    const y = 60 - 48 * Math.sin(Math.PI * k);
    return `<svg class="wx-sunarc" viewBox="0 0 200 66" preserveAspectRatio="xMidYMid meet">
      <line x1="6" y1="60" x2="194" y2="60" stroke="rgba(255,255,255,0.15)" stroke-width="1.5"/>
      <path d="M16 60 A84 48 0 0 1 184 60" fill="none" stroke="rgba(255,203,69,0.35)" stroke-width="2" stroke-dasharray="4 5"/>
      ${up ? `<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="7" fill="#FFCB45"/><circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="12" fill="#FFCB45" opacity="0.2"/>` : ''}
    </svg>`;
  }

  // ---------------------------------------------------------------- panels
  function renderNow(f) {
    const c = f.current;
    const i0 = hourIndexNow(f);
    const m = f.minutely_15;
    const maxP = Math.max(0.6, ...m.precipitation.map((p) => p || 0));
    const bars = m.precipitation.map((p) => {
      p = p || 0;
      return p >= 0.02 ? `<i style="height:${clamp(p / maxP * 100, 8, 100)}%"></i>` : '<i class="dry"></i>';
    }).join('');
    const pNow = f.hourly.pressure_msl[i0];
    const pBefore = f.hourly.pressure_msl[Math.max(0, i0 - 3)];
    const trend = pNow - pBefore > 1 ? 'steigend ↗' : pNow - pBefore < -1 ? 'fallend ↘' : 'gleichbleibend →';
    const vis = c.visibility || 0;
    const visTxt = vis >= 20000 ? 'Sehr gute Fernsicht' : vis >= 10000 ? 'Gute Sicht' : vis >= 4000 ? 'Mäßige Sicht' : 'Schlechte Sicht';
    const uv = c.uv_index || 0;
    const rise = new Date(f.daily.sunrise[0]);
    const set = new Date(f.daily.sunset[0]);
    const afterSunset = Date.now() > set.getTime();
    const dayLen = f.daily.daylight_duration[0] / 3600;

    return `
      <div class="wx-card wx-summary">
        <div class="wx-summary-text">${nowSummary(f)}<small>${nowSubline(f)}</small></div>
        <div class="wx-nowcast">
          <div class="wx-h" style="margin-bottom:8px">Niederschlag · nächste 4 Std.</div>
          <div class="wx-nowcast-bars">${bars}</div>
          <div class="wx-nowcast-lbl"><span>Jetzt</span><span>+1 h</span><span>+2 h</span><span>+3 h</span><span>+4 h</span></div>
        </div>
      </div>
      ${hourlyChart(f, Array.from({ length: 24 }, (_, k) => i0 + k), { labelNow: true })}
      <div class="wx-tiles">
        <div class="wx-card wx-tile row"><div>
          <div class="lbl">Wind</div><div class="val">${round(c.wind_speed_10m)}<small>km/h</small></div>
          <div class="sub">Böen ${round(c.wind_gusts_10m)} · aus ${compass(c.wind_direction_10m)}</div>
        </div>${compassSvg(c.wind_direction_10m)}</div>
        ${tile('Luftfeuchte', `${round(c.relative_humidity_2m)}<small>%</small>`, `Taupunkt ${round(c.dew_point_2m)}°`,
          `<div class="wx-meter"><i style="width:${round(c.relative_humidity_2m)}%;background:linear-gradient(90deg,#3B7BFF,#8CC6FF)"></i></div>`)}
        ${tile('UV-Index', `${round(uv)}<small>${uvLabel(uv)}</small>`, `Heute max. ${round(f.daily.uv_index_max[0])}`,
          `<div class="wx-meter uv"><b style="left:${clamp(uv / 11 * 100, 2, 98)}%"></b></div>`)}
        ${tile('Luftdruck', `${round(c.pressure_msl)}<small>hPa</small>`, trend)}
        ${tile('Sichtweite', `${fmtNum(vis / 1000, vis < 10000 ? 1 : 0)}<small>km</small>`, visTxt)}
        ${tile('Bewölkung', `${round(c.cloud_cover)}<small>%</small>`, c.cloud_cover >= 85 ? 'Geschlossene Wolkendecke' : c.cloud_cover >= 50 ? 'Überwiegend bewölkt' : c.cloud_cover >= 15 ? 'Aufgelockert' : 'Kaum Wolken')}
        <div class="wx-card wx-tile span2">
          <div class="lbl">Sonne · ${fmtNum(dayLen, 1)} Std. Tageslicht</div>
          ${sunArc(f, 0)}
          <div class="wx-sunrow"><span>Aufgang <b>${fmtTime(rise)}</b></span><span>${afterSunset ? `Morgen <b>${fmtTime(new Date(f.daily.sunrise[1]))}</b>` : `Untergang <b>${fmtTime(set)}</b>`}</span></div>
        </div>
      </div>`;
  }

  const PARTS = [['Früh', 6, 10], ['Mittag', 11, 16], ['Abend', 17, 21], ['Nacht', 22, 29]];
  function renderDay(f, di, { withBack = false } = {}) {
    const d = f.daily;
    const base = dayStartIndex(f, di);
    const date = dayDate(f, di);
    const title = di === 0 ? 'Heute' : di === 1 ? 'Morgen' : WD_LONG[date.getDay()];
    const parts = PARTS.map(([name, a, b]) => {
      const idx = [];
      for (let hh = a; hh <= b; hh++) if (base + hh < f.hourly.time.length) idx.push(base + hh);
      if (!idx.length) return '';
      const temps = idx.map((i) => f.hourly.temperature_2m[i]);
      const probs = idx.map((i) => f.hourly.precipitation_probability[i] || 0);
      const code = dominantCode(idx.map((i) => f.hourly.weather_code[i]));
      const t = name === 'Nacht' ? Math.min(...temps) : name === 'Mittag' ? Math.max(...temps) : temps.reduce((s, v) => s + v, 0) / temps.length;
      const p = Math.max(...probs);
      return `<div class="wx-card wx-part">
        <div class="n">${name}<small>${pad2(a)}–${pad2((b + 1) % 24)} Uhr</small></div>
        <div class="ic">${wxIcon(code, name !== 'Nacht')}</div>
        <div class="tt">${round(t)}°</div>
        <div class="pp${p < 10 ? ' dry' : ''}">${p < 10 ? 'trocken' : `${p} % Regen`}</div>
      </div>`;
    }).join('');
    const sunH = (d.sunshine_duration[di] || 0) / 3600;
    const hours = Array.from({ length: 24 }, (_, k) => base + k);

    return `
      ${withBack ? '<button class="wx-back wx-nav" data-act="back">‹ 14 Tage</button>' : ''}
      <div class="wx-card wx-dayhead">
        <div class="ic">${wxIcon(dayCode(f, di))}</div>
        <div class="temps">${round(d.temperature_2m_max[di])}°<span class="lo">${round(d.temperature_2m_min[di])}°</span></div>
        <div class="txt"><h2>${title}, ${date.getDate()}. ${MONTHS[date.getMonth()]}</h2><p>${daySummary(f, di)}</p></div>
      </div>
      <div class="wx-parts">${parts}</div>
      <div class="wx-tiles five">
        ${tile('Regen', `${fmtNum(d.precipitation_sum[di] || 0, 1)}<small>mm</small>`, `${d.precipitation_probability_max[di] || 0} % · ${round(d.precipitation_hours[di] || 0)} Std.`)}
        ${tile('Sonne', `${fmtNum(sunH, 1)}<small>Std.</small>`, `von ${fmtNum(d.daylight_duration[di] / 3600, 1)} Std. Tag`)}
        ${tile('Wind', `${round(d.wind_speed_10m_max[di])}<small>km/h</small>`, `Böen ${round(d.wind_gusts_10m_max[di])} · aus ${compass(d.wind_direction_10m_dominant[di])}`)}
        ${tile('UV max.', `${round(d.uv_index_max[di] || 0)}<small>${uvLabel(d.uv_index_max[di] || 0)}</small>`, (d.uv_index_max[di] || 0) >= 3 ? 'Sonnenschutz empfohlen' : 'Kein Schutz nötig')}
        ${tile('Sonnenzeiten', `${fmtTime(new Date(d.sunrise[di]))}`, `bis ${fmtTime(new Date(d.sunset[di]))} Uhr`)}
      </div>
      ${hourlyChart(f, hours)}`;
  }

  function renderDays(f) {
    const d = f.daily;
    const N = Math.min(14, d.time.length);
    const gMin = Math.min(...d.temperature_2m_min.slice(0, N));
    const gMax = Math.max(...d.temperature_2m_max.slice(0, N));
    const span = Math.max(1, gMax - gMin);
    const rows = [];
    for (let di = 0; di < N; di++) {
      const lo = d.temperature_2m_min[di];
      const hi = d.temperature_2m_max[di];
      const date = dayDate(f, di);
      const p = d.precipitation_probability_max[di] || 0;
      const mm = d.precipitation_sum[di] || 0;
      const left = (lo - gMin) / span * 100;
      const width = Math.max(4, (hi - lo) / span * 100);
      const nowDot = di === 0 ? `<b style="left:${clamp((f.current.temperature_2m - gMin) / span * 100, 0, 100)}%"></b>` : '';
      rows.push(`<div class="wx-day wx-nav" data-day="${di}" style="--k:${di}">
        <div class="dn">${dayLabel(f, di)}<small>${date.getDate()}.${date.getMonth() + 1}.</small></div>
        <div class="ic">${wxIcon(dayCode(f, di))}</div>
        <div class="pp">${p >= 20 ? `${p} %` : ''}</div>
        <div class="lo">${round(lo)}°</div>
        <div class="wx-range"><i style="left:${left.toFixed(1)}%;width:${width.toFixed(1)}%;background:linear-gradient(90deg,${tempColor(lo)},${tempColor(hi)})"></i>${nowDot}</div>
        <div class="hi">${round(hi)}°</div>
        <div class="mm">${mm >= 0.1 ? `${fmtNum(mm, 1)} mm` : ''}</div>
        <div class="wd">${round(d.wind_gusts_10m_max[di])} km/h</div>
      </div>`);
    }
    return `<div class="wx-card wx-days">${rows.join('')}</div>`;
  }

  const SEVERITY = {
    minor: { label: 'Wetterwarnung', color: '#FFD84A', lvl: 1 },
    moderate: { label: 'Markantes Wetter', color: '#FF9A2E', lvl: 2 },
    severe: { label: 'Unwetterwarnung', color: '#FF3B3B', lvl: 3 },
    extreme: { label: 'Extremes Unwetter', color: '#C23BD4', lvl: 4 }
  };
  const sev = (a) => SEVERITY[a.severity] || SEVERITY.minor;
  function fmtRange(a, b) {
    const f = (d) => `${WD_SHORT[d.getDay()]} ${fmtTime(d)}`;
    const s = new Date(a);
    if (!b) return `ab ${f(s)} Uhr`;
    return `${f(s)} – ${f(new Date(b))} Uhr`;
  }

  function renderAlerts(alerts) {
    let list;
    if (alerts === null) {
      list = '<div class="wx-card wx-error">Warnungen konnten gerade nicht geladen werden.</div>';
    } else if (!alerts.length) {
      list = `<div class="wx-card wx-empty">
        <div class="big"><svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg></div>
        <h3>Keine Warnungen für Murr</h3>
        <p>Der Deutsche Wetterdienst hat aktuell keine Wetterwarnung für die Region ausgegeben. Die Karte zeigt, ob anderswo in Baden-Württemberg gewarnt wird.</p>
      </div>`;
    } else {
      list = alerts.slice().sort((a, b) => sev(b).lvl - sev(a).lvl || new Date(a.onset) - new Date(b.onset)).map((a) => {
        const s = sev(a);
        return `<div class="wx-card wx-alert wx-nav" style="--c:${s.color}">
          <div class="lv">${s.label}</div>
          <h3>${esc(a.headline_de || a.event_de)}</h3>
          <div class="when">${esc(fmtRange(a.onset || a.effective, a.expires))}</div>
          ${a.description_de ? `<p>${esc(a.description_de)}</p>` : ''}
          ${a.instruction_de ? `<p class="instr">${esc(a.instruction_de)}</p>` : ''}
        </div>`;
      }).join('');
    }
    return `<div class="wx-alerts">
      <div class="wx-map" id="wx-warn-map"></div>
      <div class="wx-alerts-list">
        ${list}
        <div class="wx-card">
          <div class="wx-h">Warnstufen des DWD auf der Karte</div>
          <div class="wx-warn-legend">
            <span><i style="background:#FFD84A"></i>Wetter</span><span><i style="background:#FF9A2E"></i>Markant</span>
            <span><i style="background:#FF3B3B"></i>Unwetter</span><span><i style="background:#C23BD4"></i>Extrem</span>
          </div>
        </div>
      </div>
    </div>`;
  }

  const POLLEN = [
    ['alder_pollen', 'Erle', [1, 10, 100]],
    ['birch_pollen', 'Birke', [1, 10, 100]],
    ['grass_pollen', 'Gräser', [1, 10, 50]],
    ['mugwort_pollen', 'Beifuß', [1, 6, 20]],
    ['ragweed_pollen', 'Ambrosia', [1, 6, 20]]
  ];
  const POLLEN_LVL = [['Keine', 'rgba(255,255,255,0.35)'], ['Gering', '#7EE081'], ['Mäßig', '#FFC845'], ['Hoch', '#FF5A5A']];
  const EAQI = [[20, 'Sehr gut', '#50F0E6'], [40, 'Gut', '#50CCAA'], [60, 'Mäßig', '#F0E641'], [80, 'Schlecht', '#FF5050'], [100, 'Sehr schlecht', '#C0305A'], [Infinity, 'Extrem schlecht', '#9D3DC0']];
  const eaqiCat = (v) => EAQI.find(([max]) => v <= max);
  // EAQI band limits per pollutant (µg/m³), same six categories.
  const POLLUTANT_BANDS = { pm2_5: [10, 20, 25, 50, 75], pm10: [20, 40, 50, 100, 150], ozone: [50, 100, 130, 240, 380], nitrogen_dioxide: [40, 90, 120, 230, 340] };
  function bandColor(key, v) {
    const i = POLLUTANT_BANDS[key].findIndex((lim) => v <= lim);
    return EAQI[i === -1 ? 5 : i][2];
  }

  function renderAir(air, f) {
    if (!air) return '<div class="wx-card wx-error">Pollen- und Luftdaten konnten gerade nicht geladen werden.</div>';
    const today = f.daily.time[0];
    const tomorrow = f.daily.time[1];
    const dayMax = (key, date) => {
      let max = 0;
      air.hourly.time.forEach((t, i) => { if (t.startsWith(date)) max = Math.max(max, air.hourly[key][i] || 0); });
      return max;
    };
    const level = (v, th) => (v >= th[2] ? 3 : v >= th[1] ? 2 : v >= th[0] ? 1 : 0);
    let anyPollen = false;
    const rows = POLLEN.map(([key, name, th]) => {
      const lt = level(dayMax(key, today), th);
      const lm = level(dayMax(key, tomorrow), th);
      if (lt || lm) anyPollen = true;
      const [lblT, colT] = POLLEN_LVL[lt];
      const segs = [1, 2, 3].map((k) => `<i style="${k <= lt ? `background:${colT}` : ''}"></i>`).join('');
      return `<div class="wx-pollen-row">
        <div class="nm">${name}</div><div class="wx-lvl">${segs}</div>
        <div class="t" style="color:${colT}">${lblT}</div><div class="tm">morgen: ${POLLEN_LVL[lm][0].toLowerCase()}</div>
      </div>`;
    }).join('');

    const c = air.current;
    const aqi = round(c.european_aqi || 0);
    const [, catName, catColor] = eaqiCat(aqi);
    const aqiPos = clamp(aqi / 120 * 100, 1, 99);
    const pollutant = (key, label) => {
      const v = c[key];
      if (v == null) return '';
      return `<div class="wx-pollutant"><i style="background:${bandColor(key, v)}"></i>${label}<span>${round(v)} µg/m³</span></div>`;
    };
    const uvNow = f.current.uv_index || 0;
    const uvMax = f.daily.uv_index_max[0] || 0;

    return `<div class="wx-air">
      <div class="wx-card">
        <div class="wx-h">Pollenflug heute</div>
        ${rows}
        <div class="wx-note">${anyPollen ? 'Tageshöchstwerte aus dem Copernicus-Pollenmodell (CAMS).' : 'Aktuell kaum Pollen in der Luft – gute Zeit für Allergiker.'}</div>
      </div>
      <div class="wx-air-col">
        <div class="wx-card">
          <div class="wx-h">Luftqualität · Europäischer Index</div>
          <div class="wx-aqi"><div class="num">${aqi}</div><div><span class="cat" style="background:${catColor}">${catName}</span><div class="sub">0–20 sehr gut · über 100 extrem schlecht</div></div></div>
          <div class="wx-aqi-bar">${EAQI.map(([, , col]) => `<i style="background:${col}"></i>`).join('')}<b style="left:${aqiPos}%"></b></div>
          <div class="wx-pollutants">
            ${pollutant('pm2_5', 'Feinstaub PM2,5')}${pollutant('pm10', 'Feinstaub PM10')}
            ${pollutant('ozone', 'Ozon')}${pollutant('nitrogen_dioxide', 'Stickstoffdioxid')}
          </div>
        </div>
        <div class="wx-card">
          <div class="wx-h">UV-Strahlung</div>
          <div class="wx-aqi"><div class="num">${round(uvNow)}</div><div><span class="cat" style="background:${uvColor(uvNow)}">${uvLabel(uvNow)}</span><div class="sub">Heute max. ${round(uvMax)} · ${uvMax >= 3 ? 'mittags Sonnenschutz empfohlen' : 'kein Sonnenschutz nötig'}</div></div></div>
          <div class="wx-meter uv" style="margin-top:14px"><b style="left:${clamp(uvNow / 11 * 100, 2, 98)}%"></b></div>
        </div>
      </div>
    </div>`;
  }

  // ---------------------------------------------------------------- maps
  // Fixed Web-Mercator view of Baden-Württemberg at zoom 8. Basemap tiles
  // and DWD WMS images are both requested for exactly this box, so they
  // line up pixel-perfectly without a map library -- positions are all in
  // percent of the box, so the map scales to whatever height it gets.
  const Z = 8;
  const TILE = 256;
  const WORLD = TILE * 2 ** Z;
  const M = 40075016.68557849;
  const worldX = (lon) => (lon + 180) / 360 * WORLD;
  const worldY = (lat) => { const r = lat * Math.PI / 180; return (1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2 * WORLD; };
  const VIEW = { x0: worldX(7.45), x1: worldX(10.55), y0: worldY(49.85), y1: worldY(47.45) };
  VIEW.w = VIEW.x1 - VIEW.x0;
  VIEW.h = VIEW.y1 - VIEW.y0;
  const mercX = (px) => px / WORLD * M - M / 2;
  const mercY = (py) => M / 2 - py / WORLD * M;
  const VIEW_BBOX = [mercX(VIEW.x0), mercY(VIEW.y1), mercX(VIEW.x1), mercY(VIEW.y0)].map((v) => v.toFixed(1)).join(',');
  const pctX = (px) => `${((px - VIEW.x0) / VIEW.w * 100).toFixed(4)}%`;
  const pctY = (py) => `${((py - VIEW.y0) / VIEW.h * 100).toFixed(4)}%`;

  function wmsUrl(layer, width, height, time) {
    return 'https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&crs=EPSG:3857' +
      `&bbox=${VIEW_BBOX}&width=${width}&height=${height}&format=image/png&transparent=true&styles=&layers=${layer}` +
      (time ? `&time=${time}` : '');
  }

  // Esri's dark "Canvas" basemap: keyless (CARTO's dark tiles now demand an
  // API key and serve a watermark otherwise). 'Base' is land/water/borders,
  // 'Reference' the place names, drawn above the radar so they stay legible.
  function tiles(layer) {
    let html = '';
    const tx0 = Math.floor(VIEW.x0 / TILE);
    const tx1 = Math.floor((VIEW.x1 - 1) / TILE);
    const ty0 = Math.floor(VIEW.y0 / TILE);
    const ty1 = Math.floor((VIEW.y1 - 1) / TILE);
    const w = `calc(${(TILE / VIEW.w * 100).toFixed(4)}% + 0.6px)`;
    const h = `calc(${(TILE / VIEW.h * 100).toFixed(4)}% + 0.6px)`;
    for (let tx = tx0; tx <= tx1; tx++) {
      for (let ty = ty0; ty <= ty1; ty++) {
        html += `<img class="wx-maptile" alt="" src="https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_${layer}/MapServer/tile/${Z}/${ty}/${tx}" style="left:${pctX(tx * TILE)};top:${pctY(ty * TILE)};width:${w};height:${h}">`;
      }
    }
    return html;
  }

  function buildMap(el, overlayHtml = '') {
    el.innerHTML = `<div class="wx-map-inner" style="aspect-ratio:${VIEW.w.toFixed(2)} / ${VIEW.h.toFixed(2)}">
      <div class="wx-map-layer wx-map-base">${tiles('Base')}</div>
      ${overlayHtml}
      <img class="wx-map-borders" alt="" src="${wmsUrl('dwd:Warngebiete_Bundeslaender', round(VIEW.w * 2), round(VIEW.h * 2))}">
      <div class="wx-map-layer wx-map-labels">${tiles('Reference')}</div>
      <div class="wx-murr" style="left:${pctX(worldX(LON))};top:${pctY(worldY(LAT))}"><span class="dot"></span><span class="lbl">Murr</span></div>
      <div class="wx-map-credit">© DWD · © Esri, HERE, Garmin, OpenStreetMap</div>
    </div>`;
    return el.querySelector('.wx-map-inner');
  }

  // ---------------------------------------------------------------- radar
  const RADAR_LAYER = 'dwd:Radar_rv_product_1x1km_ger';
  const RADAR_CAPS = 'https://maps.dwd.de/geoserver/dwd/Radar_rv_product_1x1km_ger/ows?service=WMS&version=1.3.0&request=GetCapabilities';
  // Requested smaller than displayed on purpose: the browser's bilinear
  // upscale turns the blocky 1 km cells into soft, app-like rain fields.
  const FRAME_W = 420;
  const FRAME_H = round(FRAME_W * VIEW.h / VIEW.w);
  const MURR_FX = round((worldX(LON) - VIEW.x0) / VIEW.w * FRAME_W);
  const MURR_FY = round((worldY(LAT) - VIEW.y0) / VIEW.h * FRAME_H);
  // DWD's exact legend colours (0.1 mm/h ... >= 150 mm/h). Anything else in
  // the image -- the grey "no data" veil outside radar range and its
  // anti-aliased magenta outline -- is dropped.
  const DWD_CLASSES = [[51, 255, 255], [26, 204, 154], [1, 153, 52], [77, 179, 27], [153, 204, 1], [204, 230, 1], [255, 255, 1], [255, 196, 1], [255, 137, 1], [255, 69, 1], [254, 0, 0], [229, 0, 76], [204, 0, 152], [102, 0, 203], [0, 0, 254]];
  const PALETTE = [
    [70, 150, 255, 120], [70, 165, 255, 160], [50, 190, 255, 190], [40, 215, 205, 210], [70, 222, 120, 220],
    [160, 228, 60, 228], [245, 226, 45, 236], [255, 192, 35, 242], [255, 142, 35, 248], [255, 84, 45, 252],
    [240, 34, 64, 255], [222, 24, 136, 255], [190, 44, 224, 255], [150, 96, 255, 255], [255, 255, 255, 255]
  ];
  const CLASS_BY_RGB = new Map(DWD_CLASSES.map((c, i) => [(c[0] << 16) | (c[1] << 8) | c[2], i]));
  const intensityWord = (cls) => (cls <= 2 ? 'leichter Regen' : cls <= 5 ? 'mäßiger Regen' : cls <= 9 ? 'starker Regen' : 'Starkregen');

  const radar = { frames: [], nowIdx: 0, idx: 0, playing: false, timer: null, loadedAt: 0, loadingPromise: null, error: null, rainShare: 0 };

  function recolor(img) {
    const d = img.data;
    const w = img.width;
    let murr = -1;
    let rain = 0;
    for (let p = 0, px = 0; p < d.length; p += 4, px++) {
      if (d[p + 3] !== 255) { d[p + 3] = 0; continue; }
      const cls = CLASS_BY_RGB.get((d[p] << 16) | (d[p + 1] << 8) | d[p + 2]);
      if (cls === undefined) { d[p + 3] = 0; continue; }
      rain++;
      const x = px % w;
      const y = (px / w) | 0;
      if (cls > murr && Math.abs(x - MURR_FX) <= 2 && Math.abs(y - MURR_FY) <= 2) murr = cls;
      const c = PALETTE[cls];
      d[p] = c[0]; d[p + 1] = c[1]; d[p + 2] = c[2]; d[p + 3] = c[3];
    }
    return { murr, share: rain / (d.length / 4) };
  }

  function loadFrame(time) {
    return new Promise((resolve) => {
      const img = new Image();
      img.crossOrigin = 'anonymous';
      img.onload = () => {
        const canvas = document.createElement('canvas');
        canvas.width = FRAME_W;
        canvas.height = FRAME_H;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, FRAME_W, FRAME_H);
        const px = ctx.getImageData(0, 0, FRAME_W, FRAME_H);
        const { murr, share } = recolor(px);
        ctx.putImageData(px, 0, 0);
        resolve({ time, canvas, murr, share });
      };
      img.onerror = () => resolve({ time, canvas: null, murr: -1, share: 0 });
      img.src = wmsUrl(RADAR_LAYER, FRAME_W, FRAME_H, time.toISOString());
    });
  }

  async function radarTimes() {
    const txt = await fetch(RADAR_CAPS, { cache: 'no-store' }).then((r) => r.text());
    const m = txt.match(/<Dimension[^>]*name="time"[^>]*>([^<]+)<\/Dimension>/);
    if (!m) throw new Error('no time dimension');
    const v = m[1].trim();
    const endStr = v.includes('/') ? v.split('/')[1] : v.split(',').pop();
    const end = new Date(endStr.trim());
    // The RV product is analysis + 2 h nowcast: its last step is "now + 2 h".
    const now = new Date(end.getTime() - 120 * 60000);
    const times = [];
    for (let t = now.getTime() - 90 * 60000; t <= end.getTime(); t += 10 * 60000) times.push(new Date(t));
    return { times, nowIdx: 9 };
  }

  function loadRadar(onProgress) {
    if (radar.loadingPromise) return radar.loadingPromise;
    radar.loadingPromise = (async () => {
      const { times, nowIdx } = await radarTimes();
      const frames = new Array(times.length);
      let done = 0;
      // The "now" frame first, so the map shows something immediately.
      const order = [nowIdx, ...times.map((_, i) => i).filter((i) => i !== nowIdx)];
      let next = 0;
      const worker = async () => {
        while (next < order.length) {
          const i = order[next++];
          frames[i] = await loadFrame(times[i]);
          done++;
          onProgress(done, times.length, i, frames);
        }
      };
      await Promise.all(Array.from({ length: 6 }, worker));
      radar.frames = frames;
      radar.nowIdx = nowIdx;
      radar.loadedAt = Date.now();
      radar.error = null;
    })().catch((err) => { radar.error = err; }).finally(() => { radar.loadingPromise = null; });
    return radar.loadingPromise;
  }

  let radarCanvasFlip = 0;
  function drawFrame(frame) {
    const a = $('wx-radar-a');
    const b = $('wx-radar-b');
    if (!a || !b) return;
    // Crossfade: draw into the hidden canvas, then swap which one shows.
    const [show, hide] = radarCanvasFlip ? [a, b] : [b, a];
    radarCanvasFlip ^= 1;
    const ctx = show.getContext('2d');
    ctx.clearRect(0, 0, FRAME_W, FRAME_H);
    if (frame && frame.canvas) ctx.drawImage(frame.canvas, 0, 0);
    show.classList.add('on');
    hide.classList.remove('on');
  }

  function showFrame(i) {
    const frames = radar.frames;
    if (!frames.length) return;
    radar.idx = (i + frames.length) % frames.length;
    const fr = frames[radar.idx];
    drawFrame(fr);
    const diff = round((fr.time - frames[radar.nowIdx].time) / 60000);
    $('wx-rtime').textContent = fmtTime(fr.time);
    const rel = $('wx-rrel');
    rel.className = 'wx-rrel' + (diff > 0 ? ' fc' : diff === 0 ? ' now' : '');
    rel.textContent = diff === 0 ? 'Jetzt' : diff < 0 ? `vor ${-diff} Min.` : `in ${diff} Min. · Vorhersage`;
    document.querySelectorAll('#wx-tl-segs .seg').forEach((s, k) => s.classList.toggle('cur', k === radar.idx));
  }

  function scheduleNext() {
    clearTimeout(radar.timer);
    if (!radar.playing) return;
    const last = radar.idx === radar.frames.length - 1;
    const delay = last ? 1700 : radar.idx === radar.nowIdx ? 1000 : 420;
    radar.timer = setTimeout(() => { showFrame(radar.idx + 1); scheduleNext(); }, delay);
  }
  function radarPlay() {
    if (!radar.frames.length) return;
    radar.playing = true;
    updatePlayLabel();
    scheduleNext();
  }
  function radarPause() {
    radar.playing = false;
    clearTimeout(radar.timer);
    updatePlayLabel();
  }
  function radarToggle() { if (radar.playing) radarPause(); else radarPlay(); }
  function radarStep(dir) { radarPause(); showFrame(radar.idx + dir); }
  function updatePlayLabel() {
    const el = $('wx-play');
    if (el) el.textContent = radar.playing ? '❚❚ Läuft' : '▶ Pausiert';
  }

  function murrRadarText() {
    const f = radar.frames;
    const ni = radar.nowIdx;
    const cur = f[ni].murr;
    const after = f.slice(ni + 1);
    if (cur >= 0) {
      const stop = after.find((fr) => fr.murr < 0);
      return stop
        ? `Über Murr fällt gerade ${intensityWord(cur)} – laut Radar-Vorhersage vorbei gegen ${fmtTime(stop.time)}.`
        : `Über Murr fällt gerade ${intensityWord(cur)} – und das hält laut Vorhersage mindestens 2 Stunden an.`;
    }
    const next = after.find((fr) => fr.murr >= 0);
    if (next) return `Regen erreicht Murr laut Radar gegen ${fmtTime(next.time)} (${intensityWord(Math.max(...after.map((fr) => fr.murr)))}).`;
    if (f.slice(0, ni).some((fr) => fr.murr >= 0)) return 'Der Regen über Murr ist durchgezogen – die nächsten 2 Stunden bleibt es trocken.';
    return 'Kein Regen über Murr – laut Radar auch nicht in den nächsten 2 Stunden.';
  }

  function renderRadarShell() {
    return `<div class="wx-radar">
      <div class="wx-map" id="wx-radar-map"></div>
      <div class="wx-radar-side">
        <div class="wx-card">
          <div class="wx-h">Regenradar Baden-Württemberg</div>
          <div class="wx-rtime" id="wx-rtime">--:--</div>
          <div class="wx-rrel" id="wx-rrel">Lädt…</div>
        </div>
        <div class="wx-card wx-tl wx-nav" id="wx-tl" data-scrub="1">
          <div class="wx-tl-head"><div class="wx-h">Zeitleiste</div><div class="wx-play" id="wx-play"></div></div>
          <div class="wx-tl-segs" id="wx-tl-segs"></div>
          <div class="wx-tl-murr" id="wx-tl-murr"></div>
          <div class="wx-tl-lbl"><span id="wx-tl-start">–90 Min.</span><span class="mid">Jetzt</span><span id="wx-tl-end">+2 Std.</span></div>
          <div class="wx-tl-hint">← → spulen · Enter abspielen/pausieren</div>
        </div>
        <div class="wx-card"><div class="wx-murr-text" id="wx-murr-text">Radar wird geladen…</div></div>
        <div class="wx-card">
          <div class="wx-h">Niederschlag</div>
          <div class="wx-legend-bar" style="background:linear-gradient(90deg,${PALETTE.slice(0, 14).map((c) => `rgba(${c[0]},${c[1]},${c[2]},${(c[3] / 255).toFixed(2)})`).join(',')})"></div>
          <div class="wx-legend-lbl"><span>leicht</span><span>mäßig</span><span>stark</span><span>Unwetter</span></div>
        </div>
      </div>
    </div>`;
  }

  function mountRadar() {
    const inner = buildMap($('wx-radar-map'),
      `<canvas class="wx-radar-canvas" id="wx-radar-a" width="${FRAME_W}" height="${FRAME_H}"></canvas>` +
      `<canvas class="wx-radar-canvas" id="wx-radar-b" width="${FRAME_W}" height="${FRAME_H}"></canvas>`);
    const msg = document.createElement('div');
    msg.className = 'wx-map-msg';
    msg.id = 'wx-radar-msg';
    msg.textContent = 'Radar wird geladen…';
    inner.appendChild(msg);

    $('wx-tl').addEventListener('click', (e) => {
      const seg = e.target.closest('.seg');
      if (seg) { radarPause(); showFrame(Number(seg.dataset.i)); } else radarToggle();
    });
  }

  function paintTimeline() {
    const f = radar.frames;
    const segs = $('wx-tl-segs');
    const strip = $('wx-tl-murr');
    if (!segs) return;
    segs.style.setProperty('--n', f.length);
    strip.style.setProperty('--n', f.length);
    segs.innerHTML = f.map((fr, k) => `<span class="seg${k > radar.nowIdx ? ' fc' : ''}${k === radar.nowIdx ? ' now' : ''}${fr.canvas ? '' : ' missing'}" data-i="${k}"></span>`).join('');
    strip.innerHTML = f.map((fr) => {
      if (fr.murr < 0) return '<i></i>';
      const c = PALETTE[fr.murr];
      return `<i style="background:rgb(${c[0]},${c[1]},${c[2]})"></i>`;
    }).join('');
    $('wx-tl-start').textContent = fmtTime(f[0].time);
    $('wx-tl-end').textContent = fmtTime(f[f.length - 1].time);
  }

  async function ensureRadar() {
    if (!$('wx-radar-map').firstChild) mountRadar();
    const fresh = radar.frames.length && Date.now() - radar.loadedAt < 5 * 60 * 1000;
    if (fresh) {
      $('wx-radar-msg').classList.add('gone');
      paintTimeline();
      $('wx-murr-text').innerHTML = murrRadarText() + `<small>Aktuell regnet es auf ${round(radar.frames[radar.nowIdx].share * 100)} % der Kartenfläche.</small>`;
      showFrame(radar.idx);
      if (activePanel === 'radar') radarPlay();
      return;
    }
    const msg = $('wx-radar-msg');
    msg.classList.remove('gone');
    msg.textContent = 'Radar wird geladen…';
    let shownNow = false;
    await loadRadar((done, total, i, frames) => {
      if (msg) msg.textContent = `Radar wird geladen… ${done}/${total}`;
      if (!shownNow && i === 9 && frames[9]) {
        // Show the current picture right away while the loop loads.
        shownNow = true;
        drawFrame(frames[9]);
        msg.classList.add('gone');
      }
    });
    if (radar.error || !radar.frames.length) {
      msg.classList.remove('gone');
      msg.textContent = 'Radar gerade nicht erreichbar';
      $('wx-rrel').textContent = 'Keine Verbindung zum DWD';
      $('wx-murr-text').textContent = 'Das Radarbild konnte nicht geladen werden.';
      return;
    }
    msg.classList.add('gone');
    paintTimeline();
    $('wx-murr-text').innerHTML = murrRadarText() + `<small>Aktuell regnet es auf ${round(radar.frames[radar.nowIdx].share * 100)} % der Kartenfläche.</small>`;
    radar.idx = 0;
    showFrame(0);
    if (open && activePanel === 'radar') radarPlay(); else updatePlayLabel();
  }

  function ensureWarnMap() {
    const el = $('wx-warn-map');
    if (!el || el.firstChild) return;
    buildMap(el, `<img class="wx-map-overlay wx-warn-overlay" alt="" src="${wmsUrl('dwd:Warnungen_Landkreise', round(VIEW.w * 1.5), round(VIEW.h * 1.5))}&_=${Date.now()}">`);
  }

  // ---------------------------------------------------------------- shell
  const CATS = ['now', 'tomorrow', 'days', 'radar', 'alerts', 'air'];
  let open = false;
  let zone = 'side';
  let sideIdx = 0;
  let contentIdx = 0;
  let activePanel = 'now';
  let dayDetail = null;
  let refreshTimer = null;

  const panelEl = (cat) => document.querySelector(`#weather-overlay .wx-panel[data-panel="${cat}"]`);
  const sideItems = () => [...document.querySelectorAll('#wx-cats .wx-cat'), $('wx-close')];
  const contentItems = () => {
    const p = panelEl(activePanel);
    return p ? [...p.querySelectorAll('.wx-nav')] : [];
  };
  function clearFocus() {
    document.querySelectorAll('#weather-overlay .focused').forEach((el) => el.classList.remove('focused'));
  }

  // The big temperature counts up to its value when the menu opens -- a
  // small "live instrument" moment. Only once per opening, not on refreshes.
  let heroCounted = false;
  let countRaf = 0;
  function countUp(el, to) {
    cancelAnimationFrame(countRaf);
    const from = to - 12;
    const t0 = performance.now();
    const tick = (now) => {
      // rAF's frame timestamp can be slightly *earlier* than t0 (it's the
      // frame's start time) -- unclamped, the exponential ease then explodes.
      const k = clamp((now - t0) / 1100, 0, 1);
      const eased = 1 - Math.pow(2, -10 * k);
      el.textContent = `${round(from + (to - from) * eased)}°`;
      countRaf = k < 1 ? requestAnimationFrame(tick) : 0;
    };
    countRaf = requestAnimationFrame(tick);
  }

  function renderHero(d) {
    const c = d.f.current;
    const day = d.f.daily;
    $('wx-hero-icon').innerHTML = wxIcon(c.weather_code, c.is_day === 1);
    if (!heroCounted) { heroCounted = true; countUp($('wx-hero-temp'), round(c.temperature_2m)); }
    else if (!countRaf) $('wx-hero-temp').textContent = `${round(c.temperature_2m)}°`; // don't cut into a running count-up
    $('wx-hero-cond').textContent = wmoText(c.weather_code, c.is_day === 1);
    $('wx-hero-sub').textContent = `H ${round(day.temperature_2m_max[0])}°  ·  T ${round(day.temperature_2m_min[0])}°  ·  Gefühlt ${round(c.apparent_temperature)}°`;
    $('wx-updated').textContent = `Aktualisiert ${fmtTime(new Date(fetchedAt))}`;

    const alerts = d.alerts || [];
    const chip = $('wx-hero-alert');
    const badge = $('wx-alert-badge');
    if (alerts.length) {
      const top = alerts.reduce((a, b) => (sev(b).lvl > sev(a).lvl ? b : a));
      chip.hidden = false;
      chip.style.setProperty('--c', sev(top).color);
      chip.textContent = `⚠ ${top.event_de || sev(top).label}`;
      badge.hidden = false;
      badge.style.setProperty('--c', sev(top).color);
      badge.textContent = alerts.length;
    } else {
      chip.hidden = true;
      badge.hidden = true;
    }

    // Tint the backdrop to match the sky.
    const kind = iconKind(c.weather_code);
    const tint = c.is_day !== 1 ? 'rgba(80,90,200,0.18)'
      : ['rain', 'heavy', 'drizzle', 'thunder'].includes(kind) ? 'rgba(70,130,255,0.18)'
      : kind === 'clear' || kind === 'partly' ? 'rgba(255,170,60,0.16)' : 'rgba(160,170,190,0.12)';
    $('weather-overlay').style.setProperty('--wx-tint', tint);
  }

  function renderPanels(d) {
    panelEl('now').innerHTML = renderNow(d.f);
    panelEl('tomorrow').innerHTML = renderDay(d.f, 1);
    renderDaysPanel();
    panelEl('alerts').innerHTML = renderAlerts(d.alerts);
    panelEl('air').innerHTML = renderAir(d.air, d.f);
    if (!panelEl('radar').firstChild) panelEl('radar').innerHTML = renderRadarShell();
    if (activePanel === 'alerts') ensureWarnMap();
  }

  function renderDaysPanel() {
    const p = panelEl('days');
    p.innerHTML = dayDetail === null ? renderDays(data.f) : renderDay(data.f, dayDetail, { withBack: true });
  }

  function selectPanel(cat) {
    if (cat === activePanel && !(cat === 'days' && dayDetail !== null)) return;
    if (activePanel === 'radar' && cat !== 'radar') radarPause();
    if (cat === 'days' && dayDetail !== null) { dayDetail = null; if (data) renderDaysPanel(); }
    activePanel = cat;
    document.querySelectorAll('#wx-cats .wx-cat').forEach((b) => b.classList.toggle('active', b.dataset.cat === cat));
    document.querySelectorAll('#weather-overlay .wx-panel').forEach((p) => p.classList.toggle('active', p.dataset.panel === cat));
    $('wx-content').scrollTop = 0;
    if (!data) return;
    if (cat === 'radar') ensureRadar();
    if (cat === 'alerts') ensureWarnMap();
  }

  function focusSide(i, { select = true } = {}) {
    const items = sideItems();
    sideIdx = (i + items.length) % items.length;
    zone = 'side';
    clearFocus();
    const el = items[sideIdx];
    el.classList.add('focused');
    if (select && el.dataset.cat) selectPanel(el.dataset.cat);
  }
  function focusContent(i) {
    const items = contentItems();
    if (!items.length) return false;
    contentIdx = clamp(i, 0, items.length - 1);
    zone = 'content';
    clearFocus();
    const el = items[contentIdx];
    el.classList.add('focused');
    el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    return true;
  }
  function backToSide() { focusSide(sideIdx, { select: false }); }

  function openDayDetail(di) {
    dayDetail = di;
    renderDaysPanel();
    $('wx-content').scrollTop = 0;
    focusContent(0);
  }
  function closeDayDetail() {
    const di = dayDetail;
    dayDetail = null;
    renderDaysPanel();
    focusContent(di);
  }

  function handleKey(e) {
    const k = e.key;
    if (zone === 'side') {
      if (k === 'ArrowDown') focusSide(sideIdx + 1);
      else if (k === 'ArrowUp') focusSide(sideIdx - 1);
      else if (k === 'ArrowRight' || k === 'Enter') {
        const el = sideItems()[sideIdx];
        if (el.id === 'wx-close') { if (k === 'Enter') close(); } else focusContent(0);
      } else if (k === 'Escape' || k === 'Backspace') close();
      return;
    }
    const items = contentItems();
    const cur = items[contentIdx];
    if (cur && cur.dataset.scrub) {
      if (k === 'ArrowLeft') { radarStep(-1); return; }
      if (k === 'ArrowRight') { radarStep(1); return; }
      if (k === 'Enter') { radarToggle(); return; }
    }
    if (k === 'ArrowDown') focusContent(contentIdx + 1);
    else if (k === 'ArrowUp') { if (contentIdx === 0) backToSide(); else focusContent(contentIdx - 1); }
    else if (k === 'ArrowLeft') { if (activePanel === 'days' && dayDetail !== null) closeDayDetail(); else backToSide(); }
    else if (k === 'Enter') { if (cur) activateContent(cur); }
    else if (k === 'Escape' || k === 'Backspace') { if (activePanel === 'days' && dayDetail !== null) closeDayDetail(); else backToSide(); }
  }

  function activateContent(el) {
    if (el.dataset.day) openDayDetail(Number(el.dataset.day));
    else if (el.dataset.act === 'back') closeDayDetail();
  }

  // Remote pointer hover: highlight what's under it without switching
  // category (sweeping across the sidebar shouldn't flip panels around).
  function pointerHover(el) {
    const side = el.closest('#weather-overlay .wx-cat');
    if (side) {
      const i = sideItems().indexOf(side);
      if (i >= 0 && !(zone === 'side' && i === sideIdx)) { sideIdx = i; zone = 'side'; clearFocus(); side.classList.add('focused'); }
      return;
    }
    const nav = el.closest('#weather-overlay .wx-nav');
    if (nav) {
      const i = contentItems().indexOf(nav);
      if (i >= 0 && !(zone === 'content' && i === contentIdx)) { contentIdx = i; zone = 'content'; clearFocus(); nav.classList.add('focused'); }
    }
  }

  async function refresh(force) {
    try {
      const d = await loadData(force);
      if (!open) return;
      renderHero(d);
      renderPanels(d);
      if (activePanel === 'radar') ensureRadar();
      loadBackdropVideo(d.f.current.weather_code);
    } catch {
      if (!open) return;
      if (!data) {
        document.querySelectorAll('#weather-overlay .wx-panel').forEach((p) => {
          if (p.dataset.panel !== 'radar') p.innerHTML = '<div class="wx-card wx-error">Wetterdaten gerade nicht erreichbar – bitte später nochmal öffnen.</div>';
        });
      }
    }
  }

  async function loadBackdropVideo(code) {
    const video = $('wx-bg-video');
    const src = await window.launcher.getWeatherVideo(weatherCodeToCondition(code));
    if (!open || !src || video.dataset.src === src) return;
    video.dataset.src = src;
    video.classList.remove('loaded');
    video.src = src;
    video.addEventListener('loadeddata', () => video.classList.add('loaded'), { once: true });
    video.play().catch(() => {});
  }

  function openWeather(startCat = 'now') {
    if (open) return;
    open = true;
    heroCounted = false;
    animatedOpen($('weather-overlay'));
    dayDetail = null;
    activePanel = null;
    if (!data) {
      document.querySelectorAll('#weather-overlay .wx-panel').forEach((p) => {
        if (p.dataset.panel !== 'radar') p.innerHTML = '<div class="wx-loading">Wetterdaten werden geladen…</div>';
      });
    } else {
      renderHero(data);
      renderPanels(data);
    }
    focusSide(CATS.indexOf(startCat));
    refresh(false);
    clearInterval(refreshTimer);
    refreshTimer = setInterval(() => refresh(true), 10 * 60 * 1000);
  }

  function close() {
    if (!open) return;
    open = false;
    radarPause();
    clearInterval(refreshTimer);
    const video = $('wx-bg-video');
    animatedClose($('weather-overlay'), {
      duration: 200,
      onDone: () => {
        video.pause();
        video.removeAttribute('src');
        delete video.dataset.src;
        video.classList.remove('loaded');
        video.load();
      }
    });
  }

  // Wiring
  document.querySelectorAll('#wx-cats .wx-cat').forEach((btn, i) => {
    btn.addEventListener('click', () => focusSide(i));
  });
  $('wx-close').addEventListener('click', close);
  $('wx-content').addEventListener('click', (e) => {
    const nav = e.target.closest('.wx-nav');
    if (!nav || nav.id === 'wx-tl') return;
    const i = contentItems().indexOf(nav);
    if (i >= 0) { contentIdx = i; zone = 'content'; clearFocus(); nav.classList.add('focused'); }
    activateContent(nav);
  });

  window.wx = {
    open: openWeather,
    close,
    isOpen: () => open,
    handleKey,
    pointerHover,
    // Read-only summary for the home tile / top-bar pill (see homeSummary).
    homeSummary,
    // Boot preloading (boot.js): the radar loop is ready before anyone opens it.
    preloadRadar: () => loadRadar(() => {})
  };
})();
