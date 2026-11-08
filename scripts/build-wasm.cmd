@echo off
rem Build the browser runtime: the raw module the dukao shell embeds, and the
rem wasm-bindgen package the website playground imports. Run it whenever
rem backend-wasm changes:
rem
rem     scripts\build-wasm.cmd
setlocal
set TARGET=wasm32-unknown-unknown
set ARTIFACT=target\%TARGET%\release\duka_backend_wasm.wasm
set WEB_OUT=website\public\wasm

cargo build --release -p duka-backend-wasm --target %TARGET%
if errorlevel 1 exit /b 1

rem The app shell packages the module itself (dukao\res).
copy /Y %ARTIFACT% dukao\res\duka-backend-wasm.wasm >nul
if errorlevel 1 exit /b 1

rem The site imports `DukaRuntime` from the generated glue, so the artifacts
rem are written under the playground's /wasm path.
if not exist %WEB_OUT% mkdir %WEB_OUT%
wasm-bindgen --target web --out-dir %WEB_OUT% --out-name duka_backend_wasm %ARTIFACT%
if errorlevel 1 exit /b 1

echo built %WEB_OUT%\duka_backend_wasm.js
