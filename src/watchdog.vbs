' Starts watchdog.ps1 without any console window (window style 0).
' Arguments: <electron.exe> <app folder> <userData folder>
Set args = WScript.Arguments
Set shell = CreateObject("WScript.Shell")
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
cmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File """ & here & "\watchdog.ps1"" -Electron """ & args(0) & """ -App """ & args(1) & """ -UserData """ & args(2) & """"
shell.Run cmd, 0, False
