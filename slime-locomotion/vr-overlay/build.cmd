@echo off
setlocal
cd /d "%~dp0"
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 exit /b 1
if not exist bin mkdir bin
cl /nologo /std:c++17 /EHsc /W4 /O2 /I ..\openvr\headers main.cpp /Fo:bin\main.obj /Fe:bin\vrcraft-vr-overlay.exe /link ..\openvr\lib\win64\openvr_api.lib user32.lib gdi32.lib winhttp.lib
if errorlevel 1 exit /b 1
copy /y ..\openvr\bin\win64\openvr_api.dll bin\openvr_api.dll >nul
exit /b %errorlevel%
