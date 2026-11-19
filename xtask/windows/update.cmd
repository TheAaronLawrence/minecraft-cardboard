@echo off
rem Runs update.ps1 without changing PowerShell's script policy, and keeps the window open.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0update.ps1"
pause
