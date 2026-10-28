#pragma once
#include "imgui.h"
#include <string>

// Thin OS layer. Windows implementation lives in host_win32.cpp (Win32 + D3D11),
// macOS/Linux implementation in host_glfw.cpp (GLFW + OpenGL 3).
namespace platform {
ImTextureID CreateTexture(const unsigned char* rgba, int w, int h);
ImTextureID LoadTextureWhiteKey(const std::string& utf8Path, int* w, int* h);  // white bg -> alpha (logo)

std::string ExeDir();
std::string PathJoin(const std::string& a, const std::string& b);
std::string FindAsset(const char* relative);     // "" if not found
std::string SystemFont(const char* fileName);    // Windows font file name -> path ("" if not found)
bool        FileExists(const std::string& utf8Path);
std::string ReadFile(const std::string& utf8Path);
bool        WriteFile(const std::string& utf8Path, const std::string& data);

std::string UserName();
std::string HardwareId();

bool        MakeDirs(const std::string& utf8Path);   // mkdir -p (UTF-8)

void  Minimize();
void  Close();
void  BeginWindowDrag();
void  SetTopmost(bool on);
void  SetStreamProof(bool on);                   // Windows only, no-op elsewhere
float DpiScale();
}
