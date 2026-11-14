@echo off
rem Startet den Musik-Server ohne Docker (braucht Node.js: https://nodejs.org).
rem Musikordner hier anpassen (darin: Interpret\Interpret - Album\01 - Interpret - Titel.mp3):
set MUSIC_DIR=D:\Musik
set DATA_DIR=%~dp0data
set PORT=8421
node "%~dp0musicserver.js"
pause
