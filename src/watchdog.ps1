# TV Launcher watchdog. Started by the launcher (see appliance-main.js) via
# watchdog.vbs, so no console window ever shows. Starts the launcher and brings
# it back whenever it is gone -- crash, killed, out of memory -- like a TV that
# never drops to a desktop. Choosing "Beenden" in the launcher writes
# quit.flag, which ends the watchdog too.
param(
  [Parameter(Mandatory = $true)][string]$Electron,
  [Parameter(Mandatory = $true)][string]$App,
  [Parameter(Mandatory = $true)][string]$UserData
)

# One watchdog per user session.
$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'Local\TvLauncherWatchdog', [ref]$created)
if (-not $created) { exit 0 }

$quitFlag = Join-Path $UserData 'quit.flag'
$logFile = Join-Path $UserData 'watchdog.log'
Remove-Item $quitFlag -ErrorAction SilentlyContinue

function Log($text) {
  try {
    if ((Test-Path $logFile) -and (Get-Item $logFile).Length -gt 200KB) { Remove-Item $logFile }
    Add-Content -Path $logFile -Value ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $text)
  } catch { }
}

function LauncherRunning {
  $procs = Get-CimInstance Win32_Process -Filter "Name='electron.exe'" -ErrorAction SilentlyContinue
  foreach ($p in $procs) { if ($p.ExecutablePath -eq $Electron -and $p.CommandLine -notmatch '--type=') { return $true } }
  return $false
}

$starts = New-Object System.Collections.Generic.List[datetime]
Log 'Watchdog gestartet'
while ($true) {
  if (Test-Path $quitFlag) { Log 'Beendet ueber das Menue, Watchdog stoppt'; break }
  if (-not (LauncherRunning)) {
    # Crash loop guard: more than 5 starts in 3 minutes -> wait a minute
    # instead of hammering a launcher that dies right away.
    $now = Get-Date
    $starts.RemoveAll({ param($t) ($now - $t).TotalMinutes -gt 3 }) | Out-Null
    if ($starts.Count -ge 5) {
      Log 'Startet immer wieder neu, warte 60 s'
      Start-Sleep -Seconds 60
      $starts.Clear()
      continue
    }
    # Give an in-app relaunch (app.relaunch) a moment to come up on its own.
    Start-Sleep -Seconds 3
    if ((Test-Path $quitFlag) -or (LauncherRunning)) { continue }
    Log 'Launcher laeuft nicht, starte'
    Start-Process -FilePath $Electron -ArgumentList ('"' + $App + '"') -WorkingDirectory $App
    $starts.Add((Get-Date))
    Start-Sleep -Seconds 10
  }
  Start-Sleep -Seconds 4
}
$mutex.ReleaseMutex()
