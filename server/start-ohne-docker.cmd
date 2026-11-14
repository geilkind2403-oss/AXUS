@echo off
rem Startet den Media-Server ohne Docker (braucht Node.js: https://nodejs.org).
rem Filmordner hier anpassen:
set MEDIA_DIR=D:\Filme
set DATA_DIR=%~dp0data
set PORT=8420
node "%~dp0mediaserver.js"
pause
