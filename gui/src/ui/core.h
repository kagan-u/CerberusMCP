#pragma once
// Core rendering layer: animation state, theme, fonts and draw-list primitives
// (gradients, soft shadows, radial glows, gradient arcs, spaced text).
#include "imgui.h"
#include "imgui_internal.h"
#include <cmath>

namespace gfx { extern float dpi; }
inline float  S(float v)            { return v * gfx::dpi; }
inline ImVec2 S2(float x, float y)  { return ImVec2(x * gfx::dpi, y * gfx::dpi); }

// ------------------------------------------------------------------ animation
namespace anim {
extern float speed;                 // global multiplier from settings
float   Dt();                       // scaled, clamped frame delta
float   Time();
float&  Ref(ImGuiID id, float initial);
float   To(ImGuiID id, float target, float rate = 12.0f);                 // starts at target
float   ToFrom(ImGuiID id, float target, float rate, float initial);      // starts at initial
inline ImGuiID Key(ImGuiID id, int slot) { return ImHashData(&slot, sizeof(slot), id); }

inline float Clamp01(float t)   { return t < 0.0f ? 0.0f : (t > 1.0f ? 1.0f : t); }
inline float OutCubic(float t)  { t = Clamp01(t); float f = 1.0f - t; return 1.0f - f * f * f; }
inline float OutQuint(float t)  { t = Clamp01(t); float f = 1.0f - t; return 1.0f - f * f * f * f * f; }
inline float InOutCubic(float t){ t = Clamp01(t); return t < 0.5f ? 4.0f * t * t * t : 1.0f - powf(-2.0f * t + 2.0f, 3.0f) * 0.5f; }
inline float OutBack(float t)   { t = Clamp01(t); const float c1 = 1.70158f, c3 = c1 + 1.0f; return 1.0f + c3 * powf(t - 1.0f, 3.0f) + c1 * powf(t - 1.0f, 2.0f); }
}

// ------------------------------------------------------------------ theme
namespace theme {
struct Preset { const char* name; ImVec4 a, b; };
extern const Preset kPresets[];
extern const int    kPresetCount;

extern bool light;                   // white/light theme
extern ImVec4 accent, accent2;      // animated live accent pair
extern ImVec4 bg0, bg1, surface, surface2, border, text, textDim, textMute, success, warning, danger;

// Subtle tint that flips with the theme: lightens a dark surface, darkens a
// light one. White on the dark theme, black on the white theme.
inline ImVec4 Ink(float a = 1.0f) { return light ? ImVec4(0, 0, 0, a) : ImVec4(1, 1, 1, a); }

void   TargetAccent(int index, ImVec4& a, ImVec4& b);
void   Update();
void   ApplyStyle(float dpi);
float  Brightness();        // 0..1, how close the live accent is to white
ImVec4 OnAccent();          // readable text/icon colour on top of an accent fill
}

// ------------------------------------------------------------------ fonts
namespace fonts {
extern ImFont *body, *bold, *small, *smallBold, *title, *hero, *display, *mono;
void Load(float dpi);
}

// ------------------------------------------------------------------ drawing
namespace gfx {
ImU32  Col(const ImVec4& c, float alpha = 1.0f);   // honours style alpha
ImVec4 Mix(const ImVec4& a, const ImVec4& b, float t);
inline ImVec4 White(float a = 1.0f) { return ImVec4(1, 1, 1, a); }
inline ImVec4 Black(float a = 1.0f) { return ImVec4(0, 0, 0, a); }
inline float  Len(ImVec2 v) { return sqrtf(v.x * v.x + v.y * v.y); }

void   Shade(ImDrawList* dl, int vtxStart, ImVec2 p0, ImVec2 p1, ImU32 c0, ImU32 c1);
void   Gradient(ImDrawList* dl, ImVec2 a, ImVec2 b, ImU32 c0, ImU32 c1, float rounding = 0.0f, bool vertical = false, ImDrawFlags flags = 0);
void   GradientDiag(ImDrawList* dl, ImVec2 a, ImVec2 b, ImU32 c0, ImU32 c1, float rounding = 0.0f);
void   Shadow(ImDrawList* dl, ImVec2 a, ImVec2 b, float rounding, float size, ImU32 col, ImVec2 offset = ImVec2(0, 0));
void   Glow(ImDrawList* dl, ImVec2 c, float radius, ImU32 col);
void   Panel(ImDrawList* dl, ImVec2 a, ImVec2 b, float rounding, float strength = 1.0f);
void   GradientBorder(ImDrawList* dl, ImVec2 a, ImVec2 b, float rounding, ImU32 c0, ImU32 c1, float thickness = 1.0f);

ImVec2 Measure(ImFont* f, const char* s, float size = 0.0f, const char* end = nullptr);
void   Text(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 col, const char* s, float size = 0.0f, const char* end = nullptr);
void   TextC(ImDrawList* dl, ImFont* f, ImVec2 center, ImU32 col, const char* s, float size = 0.0f);
void   TextGradient(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 c0, ImU32 c1, const char* s, float size = 0.0f);
float  Spaced(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 col, const char* s, float spacing, float size = 0.0f);
float  SpacedWidth(ImFont* f, const char* s, float spacing, float size = 0.0f);

void   Spinner(ImDrawList* dl, ImVec2 c, float r, float thick, ImU32 col, float speed = 1.0f);
void   Check(ImDrawList* dl, ImVec2 c, float size, ImU32 col, float thick, float progress);
void   Arc(ImDrawList* dl, ImVec2 c, float r, float a0, float a1, float thick, ImU32 c0, ImU32 c1);
void   BrandMark(ImDrawList* dl, ImVec2 c, float r, bool glow);
}

// ------------------------------------------------------------------ background
namespace bg {
enum Style { BG_AUTO, BG_AURORA, BG_WAVES, BG_WARP, BG_GRID, BG_CONSTELLATION, BG_COUNT };
void SetScene(int style);   // style used when the setting is "auto"
void SetBoost(bool on);     // warp speed-up while loading
void Draw(ImDrawList* dl, ImVec2 a, ImVec2 b);
}
