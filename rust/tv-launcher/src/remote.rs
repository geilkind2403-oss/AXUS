//! The phone as a remote: a small web page on port 8080 (same WLAN). Big
//! arrow pad, OK, back, home, play/pause, volume -- and a text field that
//! types into search fields (YouTube) and password boxes.
//!
//! Pairing: a phone the TV does not know gets only the pairing page. It asks
//! for a code; the TV shows four random digits for a minute; the phone sends
//! them back and receives a long random key (a cookie) that it keeps. Every
//! other request needs that key. Five wrong tries end the code.
use crate::app::Msg;
use crate::platform::input::{InputMsg, Key};
use std::io::Read;
use std::sync::mpsc::Sender;

pub const PORT: u16 = 8080;
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn start(tx: Sender<Msg>, enabled: bool) {
    set_enabled(enabled);
    std::thread::spawn(move || loop {
        if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(250));
            continue;
        }
        let server = match tiny_http::Server::http(("0.0.0.0", PORT)) {
            Ok(s) => s,
            Err(e) => {
                crate::log(&format!("Handy-Fernbedienung: Port {PORT} belegt ({e})"));
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };
        crate::log(&format!(
            "Handy-Fernbedienung: {}",
            if ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
                "aktiv"
            } else {
                "aus"
            }
        ));
        while ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            match server.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Some(req)) => handle(req, &tx),
                Ok(None) => {}
                Err(_) => break,
            }
        }
        drop(server);
        crate::log("Handy-Fernbedienung: aus");
    });
}

/// Keys of paired phones, kept in the data folder (not in the settings
/// file: that one may end up in a bug report).
fn keys_file() -> std::path::PathBuf {
    crate::app::data_dir().join("remote_keys")
}

static KEYS: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// (code, when shown, wrong tries)
static CODE: std::sync::Mutex<Option<(String, std::time::Instant, u8)>> =
    std::sync::Mutex::new(None);

const CODE_SECS: u64 = 60;

fn with_keys<R>(f: impl FnOnce(&mut Vec<String>) -> R) -> R {
    let mut g = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let keys = g.get_or_insert_with(|| {
        std::fs::read_to_string(keys_file())
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| l.len() == 32)
            .collect()
    });
    f(keys)
}

fn save_keys(keys: &[String]) {
    let path = keys_file();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).ok();
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, keys.join("\n")).is_ok() {
        std::fs::rename(tmp, path).ok();
    }
}

/// How many phones are paired.
pub fn paired_count() -> usize {
    with_keys(|k| k.len())
}

/// Forgets every paired phone (Settings).
pub fn forget_all() {
    with_keys(|k| {
        k.clear();
        save_keys(k);
    });
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        // no /dev/urandom (not on Linux): mixed from the clock and addresses
        use std::hash::{BuildHasher, Hasher};
        for (i, x) in b.iter_mut().enumerate() {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_usize(i);
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0),
            );
            *x = h.finish() as u8;
        }
    }
    b
}

/// The key this request carries in its cookie, if it is a paired phone's.
fn authorized(req: &tiny_http::Request) -> bool {
    let Some(key) = req
        .headers()
        .iter()
        .filter(|h| h.field.equiv("Cookie"))
        .flat_map(|h| h.value.as_str().split(';').map(|p| p.trim().to_string()).collect::<Vec<_>>())
        .find_map(|p| p.strip_prefix("axus=").map(|v| v.to_string()))
    else {
        return false;
    };
    if key.len() != 32 {
        return false;
    }
    with_keys(|k| {
        // compared in full every time (no early exit on the first difference)
        k.iter().any(|known| {
            known
                .bytes()
                .zip(key.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        })
    })
}

fn text(s: &str, status: u16) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(s)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap(),
        )
}

/// The pairing steps: /pair/start shows a code on the TV, /pair/<code>
/// trades a correct code for a key.
fn pair(req: tiny_http::Request, url: &str, tx: &Sender<Msg>) {
    if url == "/pair/start" {
        let b = random_bytes(4);
        let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 10_000;
        let code = format!("{n:04}");
        *CODE.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((code.clone(), std::time::Instant::now(), 0));
        let _ = tx.send(Msg::Run(Box::new(move |cx| {
            cx.notice(format!("Handy koppeln – Code: {code}"), CODE_SECS as f32);
        })));
        let _ = req.respond(text("ok", 200));
        return;
    }
    let given = url.trim_start_matches("/pair/").to_string();
    let mut slot = CODE.lock().unwrap_or_else(|e| e.into_inner());
    let ok = match slot.as_mut() {
        Some((code, at, tries)) if at.elapsed().as_secs() < CODE_SECS && *tries < 5 => {
            if *code == given {
                true
            } else {
                *tries += 1;
                false
            }
        }
        _ => {
            let _ = req.respond(text("Der Code ist abgelaufen. Bitte neu anfordern.", 410));
            return;
        }
    };
    if !ok {
        let _ = req.respond(text("Falscher Code", 403));
        return;
    }
    *slot = None;
    drop(slot);
    let key: String = random_bytes(16).iter().map(|b| format!("{b:02x}")).collect();
    with_keys(|k| {
        k.push(key.clone());
        // the oldest phones go when there are many
        if k.len() > 16 {
            k.remove(0);
        }
        save_keys(k);
    });
    let _ = tx.send(Msg::Run(Box::new(|cx| {
        cx.notice("Handy gekoppelt", 3.0);
    })));
    let cookie = format!("axus={key}; Max-Age=315360000; Path=/; SameSite=Strict; HttpOnly");
    let _ = req.respond(
        text("ok", 200).with_header(tiny_http::Header::from_bytes("Set-Cookie", cookie).unwrap()),
    );
}

fn handle(req: tiny_http::Request, tx: &Sender<Msg>) {
    let url = req.url().to_string();
    if url.starts_with("/pair/") {
        return pair(req, &url, tx);
    }
    if !authorized(&req) {
        let resp = if url == "/" || url.starts_with("/?") {
            tiny_http::Response::from_string(PAIR_PAGE).with_header(
                tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
            )
        } else {
            text("Nicht gekoppelt", 401)
        };
        let _ = req.respond(resp);
        return;
    }
    handle_paired(req, tx, url);
}

fn handle_paired(mut req: tiny_http::Request, tx: &Sender<Msg>, url: String) {
    let resp = if url == "/" || url.starts_with("/?") {
        tiny_http::Response::from_string(PAGE).with_header(
            tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
        )
    } else if let Some(name) = url.strip_prefix("/key/") {
        match Key::from_name(name) {
            Some(k) => {
                #[cfg(target_os = "linux")]
                if crate::platform::input::browser_active()
                    && !crate::platform::input::control_open()
                    && k != Key::Home
                    && k != Key::Back
                    && k != Key::LongOk
                    && k != Key::LongBack
                {
                    crate::platform::uinput::press(k);
                    let _ = req.respond(tiny_http::Response::from_string("ok"));
                    return;
                }
                let _ = tx.send(Msg::Input(InputMsg::Key(k)));
                tiny_http::Response::from_string("ok")
            }
            None => tiny_http::Response::from_string("unknown key").with_status_code(400),
        }
    } else if url == "/testlauf" {
        let msg = if crate::selftest::start(tx.clone()) {
            "Testlauf läuft (etwa 2 Minuten, bitte nichts drücken). Danach den Messbericht herunterladen."
        } else {
            "Ein Testlauf läuft schon."
        };
        let _ = req.respond(tiny_http::Response::from_string(msg).with_header(
            tiny_http::Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap(),
        ));
        return;
    } else if url == "/messbericht" {
        let r = tiny_http::Response::from_string(crate::perf::report())
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(
                    "Content-Disposition",
                    "attachment; filename=\"messbericht.txt\"",
                )
                .unwrap(),
            );
        let _ = req.respond(r);
        return;
    } else if url == "/text" {
        let mut body = String::new();
        let _ = req.as_reader().take(2000).read_to_string(&mut body);
        if !body.trim().is_empty() {
            #[cfg(target_os = "linux")]
            if crate::platform::input::browser_active() && !crate::platform::input::control_open() {
                crate::platform::uinput::type_text(body.trim());
                let _ = req.respond(tiny_http::Response::from_string("ok"));
                return;
            }
            let _ = tx.send(Msg::Input(InputMsg::Text(body.trim().to_string())));
        }
        tiny_http::Response::from_string("ok")
    } else {
        tiny_http::Response::from_string("not found").with_status_code(404)
    };
    let _ = req.respond(resp);
}

const PAGE: &str = r##"<!doctype html>
<html lang="de"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<meta name="theme-color" content="#0a0a0c"><title>TV Fernbedienung</title>
<style>
:root{--bg:#0a0a0c;--p:#1c1c20;--t:#f1f1f3;--m:#8b8b94;--a:#3b6dff}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
body{margin:0;background:var(--bg);color:var(--t);font:16px system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;padding:18px 16px 28px;min-height:100vh;user-select:none}
h1{font-size:15px;color:var(--m);font-weight:600;margin:0 0 14px;letter-spacing:.04em}
.pad{position:relative;width:min(78vw,320px);aspect-ratio:1;border-radius:50%;background:var(--p);margin:6px 0 22px}
.pad button{position:absolute;background:none;border:0;color:var(--t);font-size:30px;width:33%;height:33%}
.u{left:33.5%;top:0}.d{left:33.5%;bottom:0}.l{left:0;top:33.5%}.r{right:0;top:33.5%}
.pad .ok{left:28%;top:28%;width:44%;height:44%;border-radius:50%;background:var(--a);font-size:20px;font-weight:700}
.row{display:flex;gap:12px;width:min(90vw,360px);margin-bottom:12px}
.row button{flex:1;height:58px;border-radius:16px;border:0;background:var(--p);color:var(--t);font-size:17px;font-weight:600}
button:active{filter:brightness(1.6)}
form{display:flex;gap:8px;width:min(90vw,360px);margin-top:10px}
input{flex:1;height:52px;border-radius:14px;border:1px solid #333;background:#121215;color:var(--t);padding:0 14px;font-size:17px}
form button{height:52px;border-radius:14px;border:0;background:var(--a);color:#fff;padding:0 18px;font-size:16px;font-weight:700}
small{color:var(--m);margin-top:14px;text-align:center}
</style></head><body>
<h1>TV FERNBEDIENUNG</h1>
<div class="row"><button data-k="home">⌂ Home</button><button data-k="back">↩ Zurück</button><button data-k="menu">☰ Menü</button></div>
<div class="pad"><button class="u" data-k="up">▲</button><button class="l" data-k="left">◀</button><button class="ok" data-k="ok">OK</button><button class="r" data-k="right">▶</button><button class="d" data-k="down">▼</button></div>
<div class="row"><button data-k="rewind">⏪</button><button data-k="playpause">⏯</button><button data-k="forward">⏩</button></div>
<div class="row"><button data-k="voldown">🔉 −</button><button data-k="mute">🔇</button><button data-k="volup">🔊 +</button></div>
<form id="f"><input id="t" placeholder="Text / Suche eingeben" autocomplete="off"><button>Senden</button></form>
<small>Text geht in das Suchfeld bzw. Passwortfeld, das gerade offen ist.</small>
<a href="/testlauf" style="margin-top:22px;color:#e2bb74;font-weight:600;text-decoration:none;border:1px solid #6b5630;border-radius:14px;padding:12px 18px">🧪 Testlauf starten</a>
<a href="/messbericht" download style="margin-top:22px;color:#e2bb74;font-weight:600;text-decoration:none;border:1px solid #6b5630;border-radius:14px;padding:12px 18px">📊 Messbericht herunterladen</a>
<script>
const send=k=>{fetch('/key/'+k,{method:'POST'});navigator.vibrate&&navigator.vibrate(12)};
let rep=null,backTimer=null;
document.querySelectorAll('[data-k]').forEach(b=>{
  const k=b.dataset.k;
  b.addEventListener('pointerdown',e=>{e.preventDefault();b.setPointerCapture&&b.setPointerCapture(e.pointerId);
    if(k==='back'){clearTimeout(backTimer);backTimer=setTimeout(()=>{backTimer=null;send('longback')},2000);return}
    send(k);
    if(['up','down','left','right','volup','voldown'].includes(k)){clearInterval(rep);rep=setTimeout(()=>{rep=setInterval(()=>send(k),120)},450)}});
  ['pointerup','pointerleave','pointercancel'].forEach(ev=>b.addEventListener(ev,()=>{
    clearTimeout(rep);clearInterval(rep);
    if(k==='back'&&backTimer){clearTimeout(backTimer);backTimer=null;send('back')}
  }));
});
document.getElementById('f').addEventListener('submit',e=>{e.preventDefault();const t=document.getElementById('t');if(t.value.trim()){fetch('/text',{method:'POST',body:t.value});t.value='';t.blur()}});
</script></body></html>"##;

const PAIR_PAGE: &str = r##"<!doctype html>
<html lang="de"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<meta name="theme-color" content="#0a0a0c"><title>TV koppeln</title>
<style>
body{margin:0;background:#0a0a0c;color:#f1f1f3;font:16px system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;padding:40px 20px;min-height:100vh;box-sizing:border-box}
h1{font-size:20px;margin:0 0 10px}p{color:#8b8b94;text-align:center;max-width:340px}
button{height:56px;border-radius:16px;border:0;background:#3b6dff;color:#fff;font-size:17px;font-weight:700;padding:0 24px;margin-top:14px}
input{width:200px;height:64px;border-radius:16px;border:1px solid #333;background:#121215;color:#f1f1f3;font-size:34px;text-align:center;letter-spacing:.3em;margin-top:18px}
#msg{color:#e2bb74;min-height:24px;margin-top:14px}
</style></head><body>
<h1>Mit dem Fernseher koppeln</h1>
<p>Damit nur deine Handys den Fernseher steuern können: Tippe auf „Code anzeigen“ und gib die vier Ziffern ein, die der Fernseher zeigt.</p>
<button id="s">Code anzeigen</button>
<input id="c" inputmode="numeric" maxlength="4" autocomplete="one-time-code" placeholder="····">
<button id="g">Koppeln</button>
<div id="msg"></div>
<script>
const m=document.getElementById('msg');
document.getElementById('s').onclick=()=>fetch('/pair/start',{method:'POST'}).then(()=>{m.textContent='Der Code steht jetzt auf dem Fernseher.';document.getElementById('c').focus()});
const go=()=>{const c=document.getElementById('c').value.trim();if(!/^\d{4}$/.test(c)){m.textContent='Bitte vier Ziffern eingeben.';return}
fetch('/pair/'+c,{method:'POST'}).then(r=>r.ok?location.reload():r.text().then(t=>m.textContent=t))};
document.getElementById('g').onclick=go;
document.getElementById('c').addEventListener('input',e=>{if(e.target.value.length===4)go()});
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn http(req: &str) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", PORT)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).ok();
        out
    }

    #[test]
    fn only_paired_phones_control_the_tv() {
        let dir = std::env::temp_dir().join(format!("axus-pair-{}", std::process::id()));
        std::env::set_var("TV_DATA", &dir);
        let (tx, rx) = std::sync::mpsc::channel();
        start(tx, true);
        std::thread::sleep(std::time::Duration::from_millis(400));
        let get = |path: &str, cookie: &str| {
            http(&format!(
                "POST {path} HTTP/1.1\r\nHost: tv\r\nContent-Length: 0\r\n{cookie}Connection: close\r\n\r\n"
            ))
        };
        // a stranger gets the pairing page and nothing else
        assert!(http("GET / HTTP/1.1\r\nHost: tv\r\nConnection: close\r\n\r\n").contains("koppeln"));
        assert!(get("/key/ok", "").contains("401"));
        assert!(get("/messbericht", "").contains("401"));
        assert!(get("/key/ok", "Cookie: axus=00000000000000000000000000000000\r\n").contains("401"));
        // the code goes to the TV
        assert!(get("/pair/start", "").contains("200"));
        let code = CODE.lock().unwrap().as_ref().unwrap().0.clone();
        let wrong = if code == "0000" { "1111" } else { "0000" };
        assert!(get(&format!("/pair/{wrong}"), "").contains("403"));
        let ok = get(&format!("/pair/{code}"), "");
        assert!(ok.contains("200"), "{ok}");
        let key = ok
            .split("axus=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        // the code works once
        assert!(get(&format!("/pair/{code}"), "").contains("410"));
        // the paired phone may press keys
        let cookie = format!("Cookie: other=1; axus={key}\r\n");
        assert!(get("/key/ok", &cookie).contains("200"));
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Input(_))));
        // five wrong tries end a code
        get("/pair/start", "");
        let code = CODE.lock().unwrap().as_ref().unwrap().0.clone();
        for _ in 0..5 {
            get(if code == "0000" { "/pair/1111" } else { "/pair/0000" }, "");
        }
        assert!(get(&format!("/pair/{code}"), "").contains("410"));
        // forgetting locks it out again
        forget_all();
        assert!(get("/key/ok", &cookie).contains("401"));
        std::fs::remove_dir_all(dir).ok();
    }
}
