@echo off
cargo build --release -p duka-app
if errorlevel 1 exit /b 1
copy /Y target\release\duka-app.exe dukao\res\duka-app.exe || exit /b 1
call "%~dp0build-wasm.cmd"
if errorlevel 1 exit /b 1

powershell -NoProfile -ExecutionPolicy Bypass -Command ^
    "$f = 'dukao\res\README.md';" ^
    "$t = '**[' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss') + '] built**';" ^
    "$l = @();" ^
    "if (Test-Path $f) {" ^
    "   $a = @(Get-Content $f);" ^
    "   if ($a.Count -gt 1) {" ^
    "       $l = $a[0..($a.Count - 2)];" ^
    "   }" ^
    "};" ^
    "Set-Content -Path $f -Value ($l + $t) -Encoding utf8"