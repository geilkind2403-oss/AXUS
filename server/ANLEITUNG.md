# Media-Server auf dem PC

Ein kleiner eigener Server (`mediaserver.js`, eine Datei, keine Pakete), der deine Filme
im Launcher-Schema ins WLAN streamt:

```
Filme/
  Crossroads/
    Crossroads.mp4
    Name/Name.txt   Info/Info.txt   FSK/FSK.txt
    Personas/Personas.txt   Madeby/Madeby.txt
    Icon/Icon.jpg   Banner/Banner.png
```

Unterordner sind erlaubt (z. B. `Filme/Action/Crossroads/...`) – jeder Ordner mit einer
Videodatei (mp4, mkv, webm, mov, m4v, avi, ts, m2ts, wmv, mpg) ist ein Film.

## Variante A: mit Docker

1. **Docker Desktop** installieren (https://www.docker.com/products/docker-desktop/), PC neu starten.
2. Ordner `server` auf den PC kopieren, z. B. nach `C:\tv-mediaserver`.
3. `.env.example` kopieren → `.env`, darin `FILME_ORDNER` eintragen (mit `/`, z. B. `D:/Filme`).
4. Im Ordner eine Eingabeaufforderung öffnen:
   ```
   docker compose up -d --build
   ```
5. Windows-Firewall fragt nach → **"Private Netzwerke" erlauben**.

## Variante B: ohne Docker

1. Node.js installieren (https://nodejs.org, LTS).
2. In `start-ohne-docker.cmd` den Filmordner eintragen, Doppelklick – fertig.

## Test

Am PC im Browser: `http://localhost:8420/api/ping` → zeigt Name und Anzahl Filme.
Vom Laptop aus: `http://<PC-IP>:8420/api/library` (PC-IP: am PC `ipconfig` → IPv4-Adresse).

## Im Launcher

Die Adresse `http://<PC-IP>:8420` im Launcher eintragen (siehe unten, Datei
`data/mediaserver.json` auf dem Laptop). Die Filme erscheinen dann in der Mediathek neben
den lokalen, gestreamt vom PC.

## MKV-Filme (und AVI, TS, M2TS, WMV …)

Laufen auch – der Server packt sie beim Abspielen live in MP4 um. Das Bild wird dabei nur
kopiert (keine Qualitätsverluste, kaum Rechenlast), nur der Ton wird bei Bedarf in AAC
umgewandelt (AC3, DTS, TrueHD …). Spulen geht wie gewohnt.

- **Tonspur:** Deutsch wird automatisch genommen, wenn es eine gibt.
- **Untertitel:** Text-Untertitel (SRT/ASS) aus der MKV werden angezeigt – bei deutschem Ton
  nur „erzwungene“ deutsche Untertitel, bei fremdsprachigem Ton die deutschen.
  (Bild-Untertitel wie PGS von Blu-rays gehen nicht.)
- **HEVC/H.265:** wird direkt abgespielt, wenn die Grafik im Laptop das kann; sonst wandelt der
  Server das Bild automatisch in H.264 um (braucht dann mehr Leistung am PC).

Dafür braucht der Server **ffmpeg**:
- **Docker:** ist schon drin, nichts zu tun.
- **Ohne Docker (Windows):** einmal in der Eingabeaufforderung `winget install Gyan.FFmpeg`,
  danach das Fenster vom Server neu öffnen. Alternativ `ffmpeg.exe` und `ffprobe.exe` einfach
  neben `mediaserver.js` legen.

Beim Start sagt der Server im Fenster Bescheid, wenn ffmpeg fehlt. MKV-Filme direkt auf dem
Laptop (`data/media`) brauchen ffmpeg auf dem Laptop – genauso installieren.

## Gut zu wissen

- Deine Filme werden nur gelesen, nie verändert.
- Spulen funktioniert sofort: es wird immer nur das gerade gebrauchte Stück übertragen.
- Weiterschauen-Stände liegen auf dem Server (`data/progress.json`) – gelten für jedes Gerät.
- Neue Filme erscheinen nach spätestens 30 Sekunden, kein Neustart nötig.
- MP4 (H.264/AAC) wird unverändert gestreamt, MKV & Co. live umgepackt (siehe oben).
- LAN-Kabel am PC = stabileres Streaming als WLAN.

# Musik-Server auf dem PC

Genauso gebaut wie der Media-Server (`musicserver.js`, eine Datei, keine Pakete), nur für
Musik und auf **Port 8421**. Der Musikplayer im Launcher („Music“ oben in der Leiste)
spielt alles von hier.

```
Musik/
  Madison Beer/
    artist.jpg            (oder artist.png – Bild vom Interpreten)
    Madison Beer - Life Support/
      Folder.jpg          (Cover)
      AlbumArtSmall.jpg   (kleines Cover, optional)
      01 - Madison Beer - The Beginning.mp3
      02 - Madison Beer - Good In Goodbye.mp3
      01 - Madison Beer - The Beginning.lrc   (optional: Songtext, läuft mit)
```

- Titel, Interpret und Nummer kommen aus dem Dateinamen; ID3-Tags gewinnen, wenn sie mehr
  wissen (z. B. „Channel Surfing / The End“, das kein Windows-Dateiname sein darf).
- Kein Folder.jpg? Dann wird das in der MP3 eingebettete Cover genommen.
- Auch möglich: `CD1`/`CD2`-Unterordner, flac, m4a, ogg, opus, wav.
- Songtexte: `.lrc` (mit Zeitstempeln, scrollt mit) oder `.txt` mit gleichem Namen wie der Song,
  oder im MP3 eingebettet.

**Mit Docker:** in `.env` zusätzlich `MUSIK_ORDNER` eintragen (z. B. `D:/Musik`), dann wie oben
`docker compose up -d --build` – startet beide Server.

**Ohne Docker:** in `start-musik-ohne-docker.cmd` den Musikordner eintragen, Doppelklick.

**Test:** am PC `http://localhost:8421/api/ping` → Anzahl Interpreten, Alben, Titel.

**Im Launcher:** nichts zu tun, wenn der Media-Server schon eingetragen ist – der Launcher
sucht den Musik-Server automatisch auf demselben PC (Port 8421). Sonst unter
Einstellungen → Mediathek → Musik-Server die PC-Adresse eintragen.
Die Windows-Firewall am PC fragt beim ersten Start nach → „Private Netzwerke“ erlauben.
