@echo off
powershell.exe -NoProfile -File "%~dp0codex-deck.ps1" %*
exit /b %errorlevel%
