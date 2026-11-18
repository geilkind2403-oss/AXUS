//! The test run ("Testlauf"): presses the remote's keys by itself through
//! every screen -- Home and its tabs, Live TV, Films, Weather, News,
//! Settings, the quick menu -- so the Messbericht holds every animation,
//! measured the same way as when a person uses the TV. Started from the
//! phone page (/testlauf) or with TV_SELFTEST=1. Takes about two minutes.
use crate::app::Msg;
use crate::platform::input::{InputMsg, Key};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::time::Duration;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Starts the run (once at a time). Returns false when one already runs.
pub fn start(tx: Sender<Msg>) -> bool {
    if RUNNING.swap(true, Relaxed) {
        return false;
    }
    let _ = std::thread::Builder::new().name("testlauf".into()).spawn(move || {
        run(&tx);
        crate::perf::note("TEST ende");
        crate::perf::flush();
        RUNNING.store(false, Relaxed);
    });
    true
}

use Key::*;

fn run(tx: &Sender<Msg>) {
    let press = |k: Key, wait: u64| {
        let _ = tx.send(Msg::Input(InputMsg::Key(k)));
        std::thread::sleep(Duration::from_millis(wait));
    };
    let step = |name: &str| crate::perf::note(&format!("TEST {name}"));
    let keys = |ks: &[Key], wait: u64| {
        for k in ks {
            press(*k, wait);
        }
    };
    // to Home, focus on the first tab
    let home = || {
        press(Home, 1200);
        press(Back, 300);
        keys(&[Left; 7], 120);
        std::thread::sleep(Duration::from_millis(600));
    };
    // open the app at this place of the Apps grid (5 columns)
    let open = |name: &str, index: usize| {
        home();
        keys(&[Right; 3], 250); // Apps tab
        press(Down, 400);
        for _ in 0..index / 5 {
            press(Down, 250);
        }
        for _ in 0..index % 5 {
            press(Right, 250);
        }
        step(&format!("{name} öffnen"));
        press(Ok, 2500);
    };

    step("start");
    home();
    step("Home Reiter durchgehen");
    keys(&[Right, Right, Right, Right, Left, Left, Left, Left], 500);
    step("Home Für dich Fokus");
    press(Down, 400);
    keys(&[Left, Right, Right, Left, Down, Right, Right, Right, Left, Left, Left, Up, Up], 400);
    step("Home Schnell-Tasten");
    keys(&[Down, Right, Right, Right, Right, Left, Left, Left, Left], 90);

    open("Live TV", 4);
    step("Live TV Liste");
    keys(&[Down, Down, Down, Down, Up, Up], 500);
    step("Live TV Liste schnell");
    keys(&[Down; 8], 90);
    keys(&[Up; 8], 90);
    step("Live TV Gruppen");
    keys(&[Left, Down, Down, Up, Right], 500);
    step("Live TV Sender wechseln");
    press(Ok, 3000);
    press(Back, 1500);
    press(Back, 1200);

    open("Filme", 5);
    step("Filme blättern");
    keys(&[Down, Down, Right, Right, Left, Up, Up], 500);
    press(Back, 1200);

    open("Wetter", 6);
    step("Wetter Animation");
    std::thread::sleep(Duration::from_secs(5));
    press(Back, 1200);

    open("Nachrichten", 7);
    step("Nachrichten Liste");
    keys(&[Down, Down, Down, Up, Down, Down], 500);
    step("Nachrichten Artikel");
    press(Right, 1500);
    keys(&[Down, Down, Up], 500);
    press(Left, 800);
    press(Back, 1200);

    open("Einstellungen", 8);
    step("Einstellungen Übersicht");
    keys(&[Down, Down, Down, Down, Down, Up, Up, Up, Up, Up], 350);
    step("Einstellungen Bereich");
    press(Ok, 800);
    keys(&[Down, Down, Down, Up, Up, Up], 350);
    press(Back, 800);
    step("Einstellungen schnell");
    keys(&[Down; 8], 90);
    keys(&[Up; 8], 90);
    press(Back, 1200);

    home();
    step("Schnellmenü");
    press(LongOk, 1500);
    keys(&[Right, Right, Left, Left], 400);
    press(Back, 1200);
    home();
}
