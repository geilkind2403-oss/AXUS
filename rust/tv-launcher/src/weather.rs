//! Weather from Open-Meteo (no key needed) for the configured place.
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Hour {
    pub hour: i32,
    pub temp: f32,
    pub code: u32,
    pub rain: u32,
    pub day: bool,
}

#[derive(Clone, Debug)]
pub struct Day {
    pub weekday: usize,
    pub max: f32,
    pub min: f32,
    pub code: u32,
    pub rain: u32,
    /// Minutes after local midnight; None when the service sent nothing.
    pub sunrise: Option<u16>,
    pub sunset: Option<u16>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f32,
    pub feels: f32,
    pub code: u32,
    pub day: bool,
    pub wind: f32,
    pub humidity: f32,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
    pub fetched: std::time::Instant,
}

pub fn fetch(lat: f64, lon: f64) -> Result<Weather, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=8\
&current=temperature_2m,apparent_temperature,weather_code,is_day,wind_speed_10m,relative_humidity_2m\
&hourly=temperature_2m,weather_code,precipitation_probability,is_day\
&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset"
    );
    let j = crate::net::get_json(&url)?;
    let c = &j["current"];
    let f = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
    let now_iso = c["time"].as_str().unwrap_or("").to_string();
    let times: Vec<&str> = j["hourly"]["time"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    let start = times
        .iter()
        .position(|t| *t >= now_iso.as_str())
        .unwrap_or(0)
        .saturating_sub(if now_iso.ends_with(":00") { 0 } else { 1 });
    let h = &j["hourly"];
    let hours = (start..(start + 25).min(times.len()))
        .map(|i| Hour {
            hour: times[i]
                .get(11..13)
                .and_then(|x| x.parse().ok())
                .unwrap_or(0),
            temp: f(&h["temperature_2m"][i]),
            code: h["weather_code"][i].as_u64().unwrap_or(0) as u32,
            rain: h["precipitation_probability"][i].as_u64().unwrap_or(0) as u32,
            day: h["is_day"][i].as_u64().unwrap_or(1) == 1,
        })
        .collect();
    let d = &j["daily"];
    let days = d["time"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, t)| Day {
                    weekday: weekday(t.as_str().unwrap_or("")),
                    max: f(&d["temperature_2m_max"][i]),
                    min: f(&d["temperature_2m_min"][i]),
                    code: d["weather_code"][i].as_u64().unwrap_or(0) as u32,
                    rain: d["precipitation_probability_max"][i].as_u64().unwrap_or(0) as u32,
                    sunrise: clock_minutes(d["sunrise"][i].as_str().unwrap_or("")),
                    sunset: clock_minutes(d["sunset"][i].as_str().unwrap_or("")),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Weather {
        temp: f(&c["temperature_2m"]),
        feels: f(&c["apparent_temperature"]),
        code: c["weather_code"].as_u64().unwrap_or(0) as u32,
        day: c["is_day"].as_u64().unwrap_or(1) == 1,
        wind: f(&c["wind_speed_10m"]),
        humidity: f(&c["relative_humidity_2m"]),
        hours,
        days,
        fetched: std::time::Instant::now(),
    })
}

/// "2026-11-15T07:21" -> 441 (local minutes after midnight).
fn clock_minutes(iso: &str) -> Option<u16> {
    let h: u16 = iso.get(11..13)?.parse().ok()?;
    let m: u16 = iso.get(14..16)?.parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// How strong the sunrise / sunset light is now (0..1): full at the moment
/// itself, gone 75 minutes before or after.
pub fn golden_hour(day: &Day, now_min: i32) -> f32 {
    [day.sunrise, day.sunset]
        .iter()
        .flatten()
        .map(|&m| 1.0 - ((now_min - m as i32).abs() as f32 / 75.0))
        .fold(0.0f32, f32::max)
        .clamp(0.0, 1.0)
}

/// How heavy rain or snow is for a WMO code (0..1), for the animation.
pub fn intensity(code: u32) -> f32 {
    match code {
        51 | 56 | 61 | 71 | 77 | 80 | 85 => 0.35,
        53 | 63 | 73 | 81 => 0.6,
        55 | 57 | 66 | 86 | 95 => 0.8,
        65 | 67 | 75 | 82 | 96 | 99 => 1.0,
        _ => 0.5,
    }
}

/// "2026-11-08" -> 0 = Sunday (Sakamoto).
fn weekday(date: &str) -> usize {
    let p: Vec<i32> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    if p.len() != 3 {
        return 0;
    }
    let (mut y, m, d) = (p[0], p[1], p[2]);
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if m < 3 {
        y -= 1;
    }
    ((y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d) % 7) as usize
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

/// WMO weather code -> (picture, German text).
pub fn describe(code: u32) -> (Sky, &'static str) {
    match code {
        0 => (Sky::Clear, "Klar"),
        1 => (Sky::Clear, "Überwiegend klar"),
        2 => (Sky::PartlyCloudy, "Teilweise bewölkt"),
        3 => (Sky::Cloudy, "Bedeckt"),
        45 | 48 => (Sky::Fog, "Nebel"),
        51 | 53 | 55 | 56 | 57 => (Sky::Drizzle, "Nieselregen"),
        61 | 63 | 66 => (Sky::Rain, "Regen"),
        65 | 67 => (Sky::Rain, "Starker Regen"),
        80 | 81 => (Sky::Rain, "Regenschauer"),
        82 => (Sky::Rain, "Heftige Schauer"),
        71 | 73 | 77 => (Sky::Snow, "Schnee"),
        75 => (Sky::Snow, "Starker Schneefall"),
        85 | 86 => (Sky::Snow, "Schneeschauer"),
        95 => (Sky::Thunder, "Gewitter"),
        96 | 99 => (Sky::Thunder, "Gewitter mit Hagel"),
        _ => (Sky::Cloudy, "–"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn weekdays() {
        assert_eq!(super::weekday("2026-11-08"), 0); // a Sunday
        assert_eq!(super::weekday("2024-02-29"), 4);
    }

    #[test]
    fn sun_times() {
        assert_eq!(super::clock_minutes("2026-11-15T07:21"), Some(441));
        assert_eq!(super::clock_minutes("2026-11-15T16:48"), Some(1008));
        assert_eq!(super::clock_minutes(""), None);
        let day = super::Day {
            weekday: 0,
            max: 0.0,
            min: 0.0,
            code: 0,
            rain: 0,
            sunrise: Some(441),
            sunset: Some(1008),
        };
        assert_eq!(super::golden_hour(&day, 1008), 1.0);
        assert_eq!(super::golden_hour(&day, 720), 0.0);
        assert!((super::golden_hour(&day, 441 - 30) - 0.6).abs() < 1e-6);
    }
}
