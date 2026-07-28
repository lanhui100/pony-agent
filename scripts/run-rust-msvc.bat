@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 (
  echo failed to load MSVC environment
  exit /b 1
)
%*
exit /b %ERRORLEVEL%
