@echo off
cargo build --release -p duka-app
if errorlevel 1 exit /b 1
copy /Y target\release\duka-app.exe dukao\res\duka-app.exe
cargo build --release -p duka-backend-wasm --target wasm32-unknown-unknown
if errorlevel 1 exit /b 1
copy /Y target\wasm32-unknown-unknown\release\duka_backend_wasm.wasm dukao\res\duka-backend-wasm.wasm
