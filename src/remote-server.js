// Gyro-Fernbedienung: ein winziges HTTP(S)+WebSocket-Server-Modul, das das
// Handy (remote.html) mit dem Launcher verbindet. Kein "ws"-Paket und kein
// openssl -- beides ist auf dem Zielrechner (kein VS/.NET-SDK, kein npm im
// PATH garantiert) nicht verlÃ¤sslich vorhanden, genau wie beim
// bt-helper-Ansatz. Stattdessen:
//
//   - Zertifikate: PowerShells eingebaute New-SelfSignedCertificate /
//     Export-PfxCertificate (Teil von Windows selbst). Node nimmt die PFX
//     direkt (https.createServer({ pfx, passphrase })), ohne Konvertierung.
//   - WebSocket: von Hand implementiert (RFC 6455 Handshake + Frames).
//
// Warum eine eigene Mini-CA statt eines einzelnen selbstsignierten
// Zertifikats: iOS-Safari lÃ¤sst sich bei einem unbekannten Zertifikat zwar
// (manchmal) per "Trotzdem besuchen" auf die Seite, verweigert danach aber
// die wss://-Verbindung -- und ohne https gibt iOS die Bewegungssensoren
// gar nicht erst frei. Der einzige verlÃ¤ssliche Weg ist ein Zertifikat,
// dem das iPhone wirklich vertraut. Also:
//
//   - Root-CA ("TV Launcher Fernbedienung"): einmal erzeugt, dauerhaft
//     behalten. Das Handy installiert sie genau einmal (Einrichtungsseite).
//   - Server-Zertifikat: von dieser CA signiert, mit der aktuellen LAN-IP.
//     Wird bei IP-Wechsel/Ablauf still neu ausgestellt -- das Handy merkt
//     davon nichts, weil es der CA vertraut, nicht dem Einzelzertifikat.
//
// HTTP und HTTPS teilen sich einen Port (erstes Byte 0x16 = TLS-Handshake),
// damit die einmal angelegte Firewall-Regel fÃ¼r beides reicht: die
// Einrichtungsseite muss per http:// erreichbar sein, weil https:// ja
// genau das ist, was vor der Einrichtung noch nicht funktioniert.
const http = require('http');
const https = require('https');
const net = require('net');
const crypto = require('crypto');
const os = require('os');
const fs = require('fs');
const path = require('path');
const { EventEmitter } = require('events');
const { execFile } = require('child_process');

const PORT = 8443;
const PFX_PASSPHRASE = 'tv-launcher-remote';

// Bump whenever the certificate recipe below changes, so certificates
// cached by an older version get regenerated instead of reused.
const CERT_VERSION = 3;

function getLanIp() {
  const nets = os.networkInterfaces();
  for (const name of Object.keys(nets)) {
    for (const net of nets[name]) {
      // 169.254.x.x is link-local (Wi-Fi Direct / Bluetooth PAN adapters
      // that exist but aren't connected) -- never reachable from the phone.
      if (net.family === 'IPv4' && !net.internal && !net.address.startsWith('169.254.')) return net.address;
    }
  }
  return null;
}

function certPaths(userDataDir) {
  return {
    rootPfx: path.join(userDataDir, 'remote-ca.pfx'),
    rootCer: path.join(userDataDir, 'remote-ca.cer'),
    leafPfx: path.join(userDataDir, 'remote-cert.pfx'),
    meta: path.join(userDataDir, 'remote-cert-meta.json')
  };
}

function ensureCertificates(userDataDir, ip) {
  return new Promise((resolve, reject) => {
    const p = certPaths(userDataDir);
    try {
      if (fs.existsSync(p.leafPfx) && fs.existsSync(p.rootCer) && fs.existsSync(p.meta)) {
        const meta = JSON.parse(fs.readFileSync(p.meta, 'utf8'));
        const renewAfter = (meta.expires || 0) - 30 * 24 * 3600 * 1000;
        if (meta.ip === ip && meta.v === CERT_VERSION && Date.now() < renewAfter) { resolve(p); return; }
      }
    } catch { /* fall through to regenerate */ }

    // iOS rejects a TLS *server* certificate outright if it is valid for
    // more than 825 days, lacks the serverAuth EKU, or doesn't list the
    // host in its SAN as the right type (an IP must be an IPAddress entry).
    // Hence 800 days + IPAddress SAN on the leaf. The root is exempt from
    // the 825-day rule, so it lives 10 years -- installed on the phone once.
    const script = `
      $ErrorActionPreference = 'Stop'
      $pw = ConvertTo-SecureString -String '${PFX_PASSPHRASE}' -Force -AsPlainText
      if (Test-Path '${p.rootPfx}') {
        $root = Import-PfxCertificate -FilePath '${p.rootPfx}' -CertStoreLocation 'Cert:\\CurrentUser\\My' -Password $pw -Exportable
      } else {
        $root = New-SelfSignedCertificate -Type Custom -Subject 'CN=TV Launcher Fernbedienung' -KeyUsage CertSign,CRLSign,DigitalSignature -TextExtension @('2.5.29.19={critical}{text}ca=1&pathlength=0') -KeyExportPolicy Exportable -CertStoreLocation 'Cert:\\CurrentUser\\My' -NotAfter (Get-Date).AddYears(10) -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256
        Export-PfxCertificate -Cert $root -FilePath '${p.rootPfx}' -Password $pw | Out-Null
      }
      try {
        Export-Certificate -Cert $root -FilePath '${p.rootCer}' -Type CERT | Out-Null
        $leaf = New-SelfSignedCertificate -Type Custom -Subject 'CN=TV Launcher' -Signer $root -KeyUsage DigitalSignature,KeyEncipherment -TextExtension @('2.5.29.17={text}IPAddress=${ip}','2.5.29.37={text}1.3.6.1.5.5.7.3.1') -KeyExportPolicy Exportable -CertStoreLocation 'Cert:\\CurrentUser\\My' -NotAfter (Get-Date).AddDays(800) -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256
        Export-PfxCertificate -Cert $leaf -FilePath '${p.leafPfx}' -Password $pw -ChainOption EndEntityCertOnly | Out-Null
        Remove-Item "Cert:\\CurrentUser\\My\\$($leaf.Thumbprint)" -DeleteKey
      } finally {
        Remove-Item "Cert:\\CurrentUser\\My\\$($root.Thumbprint)" -DeleteKey
      }
    `;

    execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true }, (err, stdout, stderr) => {
      if (err) { reject(new Error(stderr || err.message)); return; }
      const expires = Date.now() + 800 * 24 * 3600 * 1000;
      try { fs.writeFileSync(p.meta, JSON.stringify({ ip, v: CERT_VERSION, expires })); } catch { /* regenerates next start, harmless */ }
      resolve(p);
    });
  });
}

// --- Minimal RFC 6455 WebSocket server, just enough for JSON text-frame
// messages from the phone, a few small ones back to it (haptic tick, what
// music is playing) and close/ping housekeeping. ---
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

// Every connected phone, for messages from the TV (emitter.send).
const openSockets = new Set();

function wsAcceptKey(key) {
  return crypto.createHash('sha1').update(key + WS_GUID).digest('base64');
}

function wsSendFrame(socket, opcode, payload) {
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.from([0x80 | opcode, len]);
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  try { socket.write(Buffer.concat([header, payload])); } catch { /* socket already gone */ }
}

// onLifecycle('open' | 'close') lets the caller count connected phones, so the
// TV can say "Handy-Fernbedienung verbunden" like a real smart TV does.
function attachWebSocket(server, onMessage, onLifecycle) {
  server.on('upgrade', (req, socket, head) => {
    if (req.url !== '/ws' || (req.headers['upgrade'] || '').toLowerCase() !== 'websocket') {
      socket.destroy();
      return;
    }
    const key = req.headers['sec-websocket-key'];
    if (!key) { socket.destroy(); return; }

    socket.write(
      'HTTP/1.1 101 Switching Protocols\r\n' +
      'Upgrade: websocket\r\n' +
      'Connection: Upgrade\r\n' +
      'Sec-WebSocket-Accept: ' + wsAcceptKey(key) + '\r\n' +
      '\r\n'
    );
    socket.setNoDelay(true); // Nagle would batch-delay exactly the small frequent packets we care about most
    // A phone that locks its screen just vanishes without a close frame;
    // keep-alive probes let the socket die (and the count drop) in ~half a minute.
    socket.setKeepAlive(true, 10000);
    openSockets.add(socket);
    socket.on('close', () => openSockets.delete(socket));

    if (onLifecycle) {
      let counted = true;
      onLifecycle('open', req.socket.remoteAddress);
      socket.on('close', () => { if (counted) { counted = false; onLifecycle('close'); } });
    }

    let buffer = head && head.length ? head : Buffer.alloc(0);

    function processBuffer() {
      for (;;) {
        if (buffer.length < 2) return;
        const b0 = buffer[0], b1 = buffer[1];
        const opcode = b0 & 0x0f;
        const masked = (b1 & 0x80) !== 0;
        let len = b1 & 0x7f;
        let offset = 2;
        if (len === 126) {
          if (buffer.length < 4) return;
          len = buffer.readUInt16BE(2);
          offset = 4;
        } else if (len === 127) {
          if (buffer.length < 10) return;
          len = Number(buffer.readBigUInt64BE(2));
          offset = 10;
        }
        let maskKey = null;
        if (masked) {
          if (buffer.length < offset + 4) return;
          maskKey = buffer.slice(offset, offset + 4);
          offset += 4;
        }
        if (buffer.length < offset + len) return;
        let payload = buffer.slice(offset, offset + len);
        if (masked) {
          const unmasked = Buffer.alloc(len);
          for (let i = 0; i < len; i++) unmasked[i] = payload[i] ^ maskKey[i % 4];
          payload = unmasked;
        }

        if (opcode === 0x8) { // close
          try { socket.end(); } catch { /* ignore */ }
        } else if (opcode === 0x9) { // ping -> pong
          wsSendFrame(socket, 0xA, payload);
        } else if (opcode === 0x1) { // text
          try { onMessage(JSON.parse(payload.toString('utf8'))); } catch { /* malformed, ignore */ }
        }

        buffer = buffer.slice(offset + len);
      }
    }

    socket.on('data', (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      processBuffer();
    });
    socket.on('error', () => {});
  });
}

// Everything the phone sends is untrusted input from the network: only
// known message types get through, numbers are clamped, and key names are
// whitelisted (they end up as synthetic keydown/input events).
const REMOTE_KEYS = new Set(['Escape', 'Enter', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home',
  'MediaPlayPause', 'MediaTrackNext', 'MediaTrackPrevious']);

function sanitizeMessage(msg) {
  if (!msg || typeof msg.t !== 'string') return null;
  const num = (v) => {
    const n = Number(v);
    return Number.isFinite(n) ? Math.max(-2000, Math.min(2000, n)) : 0;
  };
  switch (msg.t) {
    case 'd': // gyro delta, degrees
    case 'm': // touchpad move, phone px (already accelerated)
      return { t: msg.t, x: num(msg.x), y: num(msg.y) };
    case 's': // two-finger scroll, phone px
      return { t: 's', y: num(msg.y) };
    case 'c': // click / OK
    case 'r': // recenter
    case 'cc': // Home held 1.5 s -> Control Center
      return { t: msg.t };
    case 'k':
      return REMOTE_KEYS.has(msg.k) ? { t: 'k', k: msg.k } : null;
    default:
      return null;
  }
}

function serveFile(res, filePath, contentType) {
  fs.readFile(filePath, (err, data) => {
    if (err) { res.writeHead(500); res.end(); return; }
    res.writeHead(200, { 'Content-Type': contentType, 'Cache-Control': 'no-store' });
    res.end(data);
  });
}

const REMOTE_HTML_PATH = path.join(__dirname, 'remote.html');
const SETUP_HTML_PATH = path.join(__dirname, 'remote-setup.html');

// --- Phone keep-alive: an idle iPhone drops off the LAN (its WLAN dozes and
// the laptop's ARP entry goes stale), after which the remote page can't
// reach the TV until something talks to the phone first. So every phone
// that ever connected gets one tiny ping per minute. Remembered across
// restarts in remote-phones.json.
const PHONE_PING_MS = 60 * 1000;

function startPhoneKeepAlive(userDataDir) {
  const file = path.join(userDataDir, 'remote-phones.json');
  let phones = [];
  try { phones = JSON.parse(fs.readFileSync(file, 'utf8')).filter((a) => typeof a === 'string'); } catch { /* none yet */ }

  const pingAll = () => phones.forEach((ip) => {
    // -l 1: one byte payload; -w 1000: give up after a second.
    execFile('ping', ['-n', '1', '-l', '1', '-w', '1000', ip], { windowsHide: true }, () => {});
  });
  pingAll();
  setInterval(pingAll, PHONE_PING_MS).unref();

  return (addr) => {
    const ip = String(addr || '').replace(/^::ffff:/, '');
    if (!/^\d+\.\d+\.\d+\.\d+$/.test(ip) || ip.startsWith('127.') || ip === getLanIp() || phones.includes(ip)) return;
    phones = [ip, ...phones].slice(0, 5);
    try { fs.writeFileSync(file, JSON.stringify(phones)); } catch { /* retried on next connect */ }
  };
}

function startRemoteServer(userDataDir) {
  const emitter = new EventEmitter();
  // TV -> every phone: { t: 'h' } haptic tick, { t: 'np', ... } now playing.
  emitter.send = (obj) => {
    const payload = Buffer.from(JSON.stringify(obj));
    openSockets.forEach((s) => wsSendFrame(s, 0x1, payload));
  };
  const ip = getLanIp();
  emitter.url = ip ? `http://${ip}:${PORT}` : null;
  if (!ip) return emitter;
  const rememberPhone = startPhoneKeepAlive(userDataDir);

  ensureCertificates(userDataDir, ip).then((p) => {
    // /ca.cer is served on both sides: over http it's how a new phone gets
    // it in the first place; over https it's harmless.
    const serveCa = (res) => {
      fs.readFile(p.rootCer, (err, data) => {
        if (err) { res.writeHead(500); res.end(); return; }
        res.writeHead(200, {
          // This exact type is what makes iOS Safari offer to install it
          // as a profile instead of just displaying/downloading bytes.
          'Content-Type': 'application/x-x509-ca-cert',
          'Content-Disposition': 'attachment; filename="TV-Launcher.cer"',
          'Cache-Control': 'no-store'
        });
        res.end(data);
      });
    };

    const httpsServer = https.createServer({ pfx: fs.readFileSync(p.leafPfx), passphrase: PFX_PASSPHRASE }, (req, res) => {
      if (req.url === '/' || req.url === '/index.html' || req.url === '/remote') serveFile(res, REMOTE_HTML_PATH, 'text/html; charset=utf-8');
      else if (req.url === '/ca.cer') serveCa(res);
      else if (req.url.startsWith('/ping')) {
        // Fetched cross-origin from the http:// setup page: it only
        // succeeds once the phone trusts the CA, which is exactly the
        // signal the setup page waits for to jump over automatically.
        res.writeHead(204, { 'Access-Control-Allow-Origin': '*', 'Cache-Control': 'no-store' });
        res.end();
      } else { res.writeHead(404); res.end(); }
    });

    const httpServer = http.createServer((req, res) => {
      if (req.url === '/' || req.url === '/index.html') serveFile(res, SETUP_HTML_PATH, 'text/html; charset=utf-8');
      else if (req.url === '/ca.cer') serveCa(res);
      // Touchpad-only mode without any setup: touch events need no secure
      // context (only the motion sensors do), so the remote page works over
      // plain http too -- it just hides the gyro mode there.
      else if (req.url === '/remote') serveFile(res, REMOTE_HTML_PATH, 'text/html; charset=utf-8');
      else { res.writeHead(404); res.end(); }
    });

    const onMessage = (raw) => {
      const msg = sanitizeMessage(raw);
      if (msg) emitter.emit('message', msg);
    };
    let clients = 0;
    const onLifecycle = (event, addr) => {
      if (event === 'open') {
        rememberPhone(addr);
        clients += 1;
        emitter.emit('client-connected', clients);
      } else {
        clients = Math.max(0, clients - 1);
        emitter.emit('client-disconnected', clients);
      }
    };
    attachWebSocket(httpsServer, onMessage, onLifecycle);
    attachWebSocket(httpServer, onMessage, onLifecycle);

    // One port, two protocols: a TLS connection always opens with a
    // handshake record (first byte 0x16); plain HTTP opens with an ASCII
    // method name. Peek, put the byte back, hand the socket over.
    const mux = net.createServer({ pauseOnConnect: true }, (socket) => {
      socket.on('error', () => {}); // a reset before handoff must not become an uncaught exception
      socket.setNoDelay(true);
      socket.once('data', (chunk) => {
        socket.pause();
        socket.unshift(chunk);
        (chunk[0] === 0x16 ? httpsServer : httpServer).emit('connection', socket);
        process.nextTick(() => socket.resume());
      });
      socket.resume();
    });

    mux.listen(PORT, '0.0.0.0', () => emitter.emit('ready'));
    mux.on('error', (err) => console.error('Remote server error:', err.message));
    emitter._server = mux;
  }).catch((err) => {
    // Deliberately not emitter.emit('error') -- an 'error' event with no
    // listener throws, and main.js's uncaughtException handler would turn
    // that into a relaunch loop. The remote is optional; the TV stays usable.
    console.error('Remote-Zertifikat konnte nicht erstellt werden:', err.message);
    emitter.url = null;
    emitter.emit('failed', err);
  });

  return emitter;
}

module.exports = { startRemoteServer };
