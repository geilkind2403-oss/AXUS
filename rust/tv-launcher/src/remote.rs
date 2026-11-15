//! The phone as a remote: a small web page on port 8080 (same WLAN). Big
//! arrow pad, OK, back, home, play/pause, volume -- and a text field that
//! types into search fields (YouTube) and password boxes.
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

fn handle(mut req: tiny_http::Request, tx: &Sender<Msg>) {
    let url = req.url().to_string();
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
