const http = require('http');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const fetch = require('cross-fetch');

const CLIENT_ID = '52a0f9d9281243c793e2b36b85e7ef5a';
const REDIRECT_URI = 'http://127.0.0.1:43287/callback';
const SCOPES = 'user-read-playback-state user-modify-playback-state user-read-currently-playing';

function base64url(buffer) {
  return buffer.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

class SpotifyAuth {
  constructor(userDataPath) {
    this.tokenFile = path.join(userDataPath, 'spotify-tokens.json');
    this.tokens = this._load();
  }

  _load() {
    try {
      return JSON.parse(fs.readFileSync(this.tokenFile, 'utf-8'));
    } catch {
      return null;
    }
  }

  _save(tokens) {
    this.tokens = tokens;
    fs.writeFileSync(this.tokenFile, JSON.stringify(tokens, null, 2));
  }

  isLoggedIn() {
    return !!this.tokens?.refresh_token;
  }

  // Opens the Spotify authorize page (via the caller-supplied navigate function,
  // e.g. loading it into the app's content view) and spins up a one-shot local
  // server to catch the redirect -- this is the PKCE flow, no client secret needed.
  login(navigateFn) {
    return new Promise((resolve, reject) => {
      const verifier = base64url(crypto.randomBytes(32));
      const challenge = base64url(crypto.createHash('sha256').update(verifier).digest());
      const state = base64url(crypto.randomBytes(16));

      const server = http.createServer(async (req, res) => {
        const url = new URL(req.url, REDIRECT_URI);
        if (url.pathname !== '/callback') {
          res.writeHead(404);
          res.end();
          return;
        }
        const code = url.searchParams.get('code');
        const returnedState = url.searchParams.get('state');
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end('<html><body style="background:#111;color:#fff;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;"><h2>Spotify verbunden — Fenster kann geschlossen werden.</h2></body></html>');
        server.close();

        if (!code || returnedState !== state) {
          reject(new Error('Spotify-Login abgebrochen oder ungültig.'));
          return;
        }

        try {
          const tokenRes = await fetch('https://accounts.spotify.com/api/token', {
            method: 'POST',
            headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
            body: new URLSearchParams({
              grant_type: 'authorization_code',
              code,
              redirect_uri: REDIRECT_URI,
              client_id: CLIENT_ID,
              code_verifier: verifier
            })
          });
          const data = await tokenRes.json();
          if (data.error) throw new Error(data.error_description || data.error);
          data.obtained_at = Date.now();
          this._save(data);
          resolve();
        } catch (err) {
          reject(err);
        }
      });

      server.listen(43287, '127.0.0.1', () => {
        const authUrl = new URL('https://accounts.spotify.com/authorize');
        authUrl.searchParams.set('client_id', CLIENT_ID);
        authUrl.searchParams.set('response_type', 'code');
        authUrl.searchParams.set('redirect_uri', REDIRECT_URI);
        authUrl.searchParams.set('code_challenge_method', 'S256');
        authUrl.searchParams.set('code_challenge', challenge);
        authUrl.searchParams.set('scope', SCOPES);
        authUrl.searchParams.set('state', state);
        navigateFn(authUrl.toString());
      });
    });
  }

  async _refreshIfNeeded() {
    if (!this.tokens) throw new Error('Nicht eingeloggt.');
    const expiresAt = this.tokens.obtained_at + this.tokens.expires_in * 1000;
    if (Date.now() < expiresAt - 30000) return; // still valid

    const res = await fetch('https://accounts.spotify.com/api/token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'refresh_token',
        refresh_token: this.tokens.refresh_token,
        client_id: CLIENT_ID
      })
    });
    const data = await res.json();
    if (data.error) throw new Error(data.error_description || data.error);
    data.obtained_at = Date.now();
    // Spotify doesn't always return a new refresh_token -- keep the old one if so.
    if (!data.refresh_token) data.refresh_token = this.tokens.refresh_token;
    this._save(data);
  }

  async apiRequest(pathAndQuery, options = {}) {
    await this._refreshIfNeeded();
    return fetch(`https://api.spotify.com/v1${pathAndQuery}`, {
      ...options,
      headers: {
        ...options.headers,
        Authorization: `Bearer ${this.tokens.access_token}`
      }
    });
  }

  async nowPlaying() {
    if (!this.isLoggedIn()) return null;
    const res = await this.apiRequest('/me/player/currently-playing');
    if (res.status === 204 || res.status === 404) return { playing: false };
    if (!res.ok) return { playing: false, error: res.status };
    const data = await res.json();
    if (!data || !data.item) return { playing: false };
    return {
      playing: data.is_playing,
      title: data.item.name,
      artist: data.item.artists.map((a) => a.name).join(', '),
      cover: data.item.album.images?.[0]?.url || null
    };
  }

  async control(action) {
    if (!this.isLoggedIn()) return;
    const map = {
      play: { method: 'PUT', path: '/me/player/play' },
      pause: { method: 'PUT', path: '/me/player/pause' },
      next: { method: 'POST', path: '/me/player/next' },
      previous: { method: 'POST', path: '/me/player/previous' }
    };
    const req = map[action];
    if (!req) return;
    await this.apiRequest(req.path, { method: req.method });
  }
}

module.exports = { SpotifyAuth };
