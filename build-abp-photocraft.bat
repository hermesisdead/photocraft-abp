@echo off
REM Builds PhotoCraft into a .exe you can run.
REM Put this file in the SAME folder as Cargo.toml, then double-click it.

echo Checking for Rust...
where cargo >nul 2>nul
if errorlevel 1 (
    echo.
    echo Rust is not installed on this computer.
    echo Go to https://rustup.rs , download rustup-init.exe, run it with the
    echo default options, then close this window, reopen a new one, and
    echo double-click this file again.
    echo.
    pause
    exit /b 1
)

echo.
echo Building PhotoCraft. This can take 10-20+ minutes the first time.
echo A lot of text will scroll by - that is normal. Do not close this window.
echo.

cargo build --release -p photocraft

if errorlevel 1 (
    echo.
    echo The build failed. Scroll up in this window to see the actual error
    echo and send that text back for help.
    echo.
    pause
    exit /b 1
)

echo.
echo Build finished successfully.
echo The program is saved at: target\release\photocraft.exe
echo Starting it now...
echo.

start "" "target\release\photocraft.exe"

pause
