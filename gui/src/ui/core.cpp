#include "core.h"
#include "../config.h"
#include "../platform.h"
#include <unordered_map>
#include <vector>
#include <string>

// ================================================================== anim
namespace anim {

float speed = 1.0f;
static std::unordered_map<ImGuiID, float> g_values;

float Dt()   { return ImMin(ImGui::GetIO().DeltaTime, 0.1f) * speed; }
float Time() { return (float)ImGui::GetTime(); }

float& Ref(ImGuiID id, float initial)
{
    auto it = g_values.find(id);
    if (it == g_values.end()) it = g_values.emplace(id, initial).first;
    return it->second;
}

float ToFrom(ImGuiID id, float target, float rate, float initial)
{
    float& v = Ref(id, initial);
    v += (target - v) * (1.0f - expf(-rate * Dt()));
    if (fabsf(target - v) < 0.0005f) v = target;
    return v;
}

float To(ImGuiID id, float target, float rate) { return ToFrom(id, target, rate, target); }

} // namespace anim

// ================================================================== gfx (basic)
namespace gfx {

float dpi = 1.0f;

ImU32 Col(const ImVec4& c, float alpha)
{
    return ImGui::ColorConvertFloat4ToU32(ImVec4(c.x, c.y, c.z, ImSaturate(c.w * alpha * ImGui::GetStyle().Alpha)));
}

ImVec4 Mix(const ImVec4& a, const ImVec4& b, float t)
{
    return ImVec4(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t, a.w + (b.w - a.w) * t);
}

} // namespace gfx

// ================================================================== theme
namespace theme {

const Preset kPresets[] = {
    { "Mono",    ImVec4(0.80f, 0.80f, 0.83f, 1), ImVec4(1.00f, 1.00f, 1.00f, 1) },
    { "CerberusMCP",  ImVec4(0.25f, 0.47f, 1.00f, 1), ImVec4(0.36f, 0.84f, 1.00f, 1) },
    { "Violet",  ImVec4(0.56f, 0.36f, 1.00f, 1), ImVec4(0.93f, 0.45f, 0.98f, 1) },
    { "Emerald", ImVec4(0.08f, 0.76f, 0.52f, 1), ImVec4(0.52f, 0.96f, 0.80f, 1) },
    { "Crimson", ImVec4(1.00f, 0.26f, 0.40f, 1), ImVec4(1.00f, 0.58f, 0.38f, 1) },
    { "Amber",   ImVec4(1.00f, 0.56f, 0.10f, 1), ImVec4(1.00f, 0.86f, 0.36f, 1) },
};
const int kPresetCount = 6;

bool light = true;   // the app ships a single white theme

// Neutral black / graphite base so every accent (and the default mono theme) sits on true black.
ImVec4 accent   = kPresets[0].a;
ImVec4 accent2  = kPresets[0].b;
ImVec4 bg0      (0.945f, 0.949f, 0.958f, 1.0f);
ImVec4 bg1      (0.978f, 0.981f, 0.988f, 1.0f);
ImVec4 surface  (1.000f, 1.000f, 1.000f, 1.0f);
ImVec4 surface2 (0.962f, 0.965f, 0.973f, 1.0f);
ImVec4 border   (0.0f, 0.0f, 0.0f, 0.085f);
ImVec4 text     (0.086f, 0.094f, 0.110f, 1.0f);
ImVec4 textDim  (0.360f, 0.380f, 0.430f, 1.0f);
ImVec4 textMute (0.560f, 0.580f, 0.630f, 1.0f);
ImVec4 success  (0.055f, 0.640f, 0.380f, 1.0f);
ImVec4 warning  (0.850f, 0.560f, 0.060f, 1.0f);
ImVec4 danger   (0.880f, 0.210f, 0.310f, 1.0f);

void TargetAccent(int index, ImVec4& a, ImVec4& b)
{
    if (index >= 0 && index < kPresetCount) { a = kPresets[index].a; b = kPresets[index].b; return; }
    a = config::c.customAccent;
    a.w = 1.0f;
    float h, s, v;
    ImGui::ColorConvertRGBtoHSV(a.x, a.y, a.z, h, s, v);
    h = fmodf(h + 0.07f, 1.0f);
    s *= 0.70f;
    v = ImMin(1.0f, v * 1.10f + 0.12f);
    ImGui::ColorConvertHSVtoRGB(h, s, v, b.x, b.y, b.z);
    b.w = 1.0f;
}

float Brightness()
{
    float l = 0.2126f * accent.x + 0.7152f * accent.y + 0.0722f * accent.z;
    float s = ImMax(accent.x, ImMax(accent.y, accent.z)) - ImMin(accent.x, ImMin(accent.y, accent.z));
    return ImSaturate((l - 0.55f) / 0.18f) * ImSaturate(1.0f - s * 2.5f);
}

ImVec4 OnAccent()
{
    return gfx::Mix(ImVec4(1, 1, 1, 1), ImVec4(0.05f, 0.05f, 0.06f, 1), Brightness());
}

void Update()
{
    ImVec4 ta, tb;
    TargetAccent(config::c.accent, ta, tb);
    float k = 1.0f - expf(-7.0f * anim::Dt());
    accent = gfx::Mix(accent, ta, k);
    accent2 = gfx::Mix(accent2, tb, k);

    ImVec4* c = ImGui::GetStyle().Colors;
    c[ImGuiCol_SliderGrab]       = accent;
    c[ImGuiCol_SliderGrabActive] = accent2;
    c[ImGuiCol_CheckMark]        = accent2;
    c[ImGuiCol_TextSelectedBg]   = ImVec4(accent.x, accent.y, accent.z, 0.35f);
    c[ImGuiCol_ScrollbarGrabActive] = accent;
    c[ImGuiCol_NavHighlight]     = ImVec4(accent.x, accent.y, accent.z, 0.0f);
}

void ApplyStyle(float scale)
{
    ImGuiStyle& s = ImGui::GetStyle();
    s.WindowPadding     = ImVec2(12, 12);
    s.FramePadding      = ImVec2(10, 7);
    s.ItemSpacing       = ImVec2(10, 10);
    s.ItemInnerSpacing  = ImVec2(8, 6);
    s.WindowRounding    = 12;
    s.ChildRounding     = 12;
    s.FrameRounding     = 8;
    s.PopupRounding     = 12;
    s.GrabRounding      = 8;
    s.ScrollbarRounding = 8;
    s.ScrollbarSize     = 6;
    s.WindowBorderSize  = 0;
    s.PopupBorderSize   = 1;
    s.ChildBorderSize   = 0;
    s.FrameBorderSize   = 0;
    s.ScaleAllSizes(scale);

    ImVec4* c = s.Colors;
    c[ImGuiCol_Text]                 = text;
    c[ImGuiCol_TextDisabled]         = textMute;
    c[ImGuiCol_WindowBg]             = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_ChildBg]              = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_PopupBg]              = light ? ImVec4(1, 1, 1, 0.99f) : ImVec4(0.078f, 0.078f, 0.086f, 0.98f);
    c[ImGuiCol_Border]               = border;
    c[ImGuiCol_BorderShadow]         = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_FrameBg]              = Ink(0.050f);
    c[ImGuiCol_FrameBgHovered]       = Ink(0.080f);
    c[ImGuiCol_FrameBgActive]        = Ink(0.100f);
    c[ImGuiCol_ScrollbarBg]          = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_ScrollbarGrab]        = Ink(0.10f);
    c[ImGuiCol_ScrollbarGrabHovered] = Ink(0.18f);
    c[ImGuiCol_Button]               = Ink(0.050f);
    c[ImGuiCol_ButtonHovered]        = Ink(0.090f);
    c[ImGuiCol_ButtonActive]         = Ink(0.120f);
    c[ImGuiCol_Header]               = Ink(0.050f);
    c[ImGuiCol_HeaderHovered]        = Ink(0.090f);
    c[ImGuiCol_HeaderActive]         = Ink(0.120f);
    c[ImGuiCol_Separator]            = border;
    c[ImGuiCol_ModalWindowDimBg]     = ImVec4(0, 0, 0, light ? 0.42f : 0.5f);
    Update();
}

} // namespace theme

// ================================================================== fonts
namespace fonts {

ImFont *body, *bold, *small, *smallBold, *title, *hero, *display, *mono;

static const ImWchar kRanges[] = {
    0x0020, 0x00FF,   // Basic Latin + Latin-1
    0x0100, 0x017F,   // Latin Extended-A (Türkçe: ğ ş ı İ)
    0x0400, 0x04FF,   // Cyrillic
    0x2010, 0x2027,   // dashes, bullets, ellipsis
    0x2030, 0x203A,
    0x20AC, 0x20AC,
    0x2190, 0x2193,   // arrows
    0,
};

static ImFont* Add(std::initializer_list<std::string> candidates, float size)
{
    ImGuiIO& io = ImGui::GetIO();
    for (const std::string& path : candidates)
    {
        if (path.empty() || !platform::FileExists(path)) continue;
        ImFontConfig cfg;
        cfg.OversampleH = 3;
        cfg.OversampleV = 1;
        cfg.PixelSnapH = false;
        if (ImFont* f = io.Fonts->AddFontFromFileTTF(path.c_str(), size, &cfg, kRanges))
            return f;
    }
    ImFontConfig cfg;
    cfg.SizePixels = size;
    return io.Fonts->AddFontDefault(&cfg);
}

void Load(float d)
{
    using platform::FindAsset;
    using platform::SystemFont;
    // Geist ships with the app (assets/fonts) - full Türkçe coverage (ğ ş ı İ).
    std::string gReg  = FindAsset("assets/fonts/Geist-Regular.ttf");
    std::string gMed  = FindAsset("assets/fonts/Geist-Medium.ttf");
    std::string gSemi = FindAsset("assets/fonts/Geist-SemiBold.ttf");
    std::string gBold = FindAsset("assets/fonts/Geist-Bold.ttf");
    std::string mReg  = FindAsset("assets/fonts/GeistMono-Regular.ttf");
    std::string mMed  = FindAsset("assets/fonts/GeistMono-Medium.ttf");
    // Optional user overrides / system fallbacks.
    std::string uBody = FindAsset("assets/fonts/Body.ttf");
    std::string uBold = FindAsset("assets/fonts/Bold.ttf");
    std::string uDisp = FindAsset("assets/fonts/Display.ttf");

    body      = Add({ gReg, gMed, uBody, SystemFont("segoeui.ttf") }, 15.0f * d);
    bold      = Add({ gSemi, gMed, gBold, uBold, SystemFont("seguisb.ttf"), SystemFont("segoeuib.ttf") }, 15.0f * d);
    small     = Add({ gReg, gMed, uBody, SystemFont("segoeui.ttf") }, 12.5f * d);
    smallBold = Add({ gSemi, gMed, gBold, uBold, SystemFont("seguisb.ttf") }, 12.0f * d);
    title     = Add({ gBold, gSemi, uBold, SystemFont("segoeuib.ttf") }, 19.0f * d);
    hero      = Add({ gBold, gSemi, uDisp, uBold, SystemFont("segoeuib.ttf") }, 30.0f * d);
    display   = Add({ gBold, gSemi, uDisp, SystemFont("bahnschrift.ttf"), SystemFont("segoeuib.ttf") }, 46.0f * d);
    mono      = Add({ mMed, mReg, SystemFont("consolab.ttf"), SystemFont("consola.ttf") }, 14.5f * d);
    ImGui::GetIO().FontDefault = body;
}

} // namespace fonts

// ================================================================== gfx (shapes)
namespace gfx {

void Shade(ImDrawList* dl, int v0, ImVec2 p0, ImVec2 p1, ImU32 c0, ImU32 c1)
{
    ImVec2 d = p1 - p0;
    float l2 = ImMax(ImLengthSqr(d), 1e-6f);
    ImVec4 a = ImGui::ColorConvertU32ToFloat4(c0), b = ImGui::ColorConvertU32ToFloat4(c1);
    for (int i = v0; i < dl->VtxBuffer.Size; i++)
    {
        ImDrawVert& v = dl->VtxBuffer[i];
        float t = ImSaturate(ImDot(v.pos - p0, d) / l2);
        float va = (float)((v.col >> IM_COL32_A_SHIFT) & 0xFF) / 255.0f;
        ImVec4 c = Mix(a, b, t);
        c.w *= va;
        v.col = ImGui::ColorConvertFloat4ToU32(c);
    }
}

void Gradient(ImDrawList* dl, ImVec2 a, ImVec2 b, ImU32 c0, ImU32 c1, float r, bool vertical, ImDrawFlags flags)
{
    int v0 = dl->VtxBuffer.Size;
    dl->AddRectFilled(a, b, IM_COL32_WHITE, r, flags);
    Shade(dl, v0, a, vertical ? ImVec2(a.x, b.y) : ImVec2(b.x, a.y), c0, c1);
}

void GradientDiag(ImDrawList* dl, ImVec2 a, ImVec2 b, ImU32 c0, ImU32 c1, float r)
{
    int v0 = dl->VtxBuffer.Size;
    dl->AddRectFilled(a, b, IM_COL32_WHITE, r);
    Shade(dl, v0, a, b, c0, c1);
}

void GradientBorder(ImDrawList* dl, ImVec2 a, ImVec2 b, float r, ImU32 c0, ImU32 c1, float thickness)
{
    int v0 = dl->VtxBuffer.Size;
    dl->AddRect(a, b, IM_COL32_WHITE, r, 0, thickness);
    Shade(dl, v0, a, b, c0, c1);
}

void Shadow(ImDrawList* dl, ImVec2 a, ImVec2 b, float r, float size, ImU32 col, ImVec2 off)
{
    ImVec4 c = ImGui::ColorConvertU32ToFloat4(col);
    if (c.w <= 0.001f || size <= 0.5f) return;
    const int n = 8;
    for (int i = n; i >= 1; i--)
    {
        float t = (float)i / n;
        float g = size * t;
        ImVec4 cc = c;
        cc.w = c.w * (1.0f - t) * (1.0f - t) * 0.42f;
        dl->AddRectFilled(a - ImVec2(g, g) + off, b + ImVec2(g, g) + off, ImGui::ColorConvertFloat4ToU32(cc), r + g);
    }
}

void Glow(ImDrawList* dl, ImVec2 c, float radius, ImU32 col)
{
    ImVec4 base = ImGui::ColorConvertU32ToFloat4(col);
    if (base.w <= 0.001f || radius <= 0.5f) return;
    const int seg = 48, rings = 4;
    static const float radii[rings + 1]  = { 0.0f, 0.22f, 0.48f, 0.74f, 1.0f };
    static const float alphas[rings + 1] = { 1.0f, 0.80f, 0.42f, 0.13f, 0.0f };

    dl->PrimReserve(seg * 3 + (rings - 1) * seg * 6, 1 + seg * rings);
    ImVec2 uv = dl->_Data->TexUvWhitePixel;
    ImDrawIdx start = (ImDrawIdx)dl->_VtxCurrentIdx;

    auto colAt = [&](int k) { ImVec4 cc = base; cc.w *= alphas[k]; return ImGui::ColorConvertFloat4ToU32(cc); };
    dl->PrimWriteVtx(c, uv, colAt(0));
    for (int k = 1; k <= rings; k++)
    {
        ImU32 ck = colAt(k);
        for (int i = 0; i < seg; i++)
        {
            float a = (float)i / seg * IM_PI * 2.0f;
            dl->PrimWriteVtx(ImVec2(c.x + cosf(a) * radius * radii[k], c.y + sinf(a) * radius * radii[k]), uv, ck);
        }
    }
    for (int i = 0; i < seg; i++)
    {
        dl->PrimWriteIdx(start);
        dl->PrimWriteIdx((ImDrawIdx)(start + 1 + i));
        dl->PrimWriteIdx((ImDrawIdx)(start + 1 + (i + 1) % seg));
    }
    for (int k = 1; k < rings; k++)
    {
        ImDrawIdx b0 = (ImDrawIdx)(start + 1 + (k - 1) * seg), b1 = (ImDrawIdx)(start + 1 + k * seg);
        for (int i = 0; i < seg; i++)
        {
            int i1 = (i + 1) % seg;
            dl->PrimWriteIdx((ImDrawIdx)(b0 + i));  dl->PrimWriteIdx((ImDrawIdx)(b1 + i));  dl->PrimWriteIdx((ImDrawIdx)(b1 + i1));
            dl->PrimWriteIdx((ImDrawIdx)(b0 + i));  dl->PrimWriteIdx((ImDrawIdx)(b1 + i1)); dl->PrimWriteIdx((ImDrawIdx)(b0 + i1));
        }
    }
}

void Panel(ImDrawList* dl, ImVec2 a, ImVec2 b, float r, float strength)
{
    float g = config::c.glass * strength;
    if (theme::light) g = ImClamp(g + 0.22f, 0.0f, 1.0f);
    Shadow(dl, a, b, r, S(22), Col(Black((theme::light ? 0.15f : 0.40f) * strength)), ImVec2(0, S(8)));
    Gradient(dl, a, b, Col(theme::surface2, g), Col(theme::surface, g), r, true);
    GradientBorder(dl, a, b, r, Col(theme::Ink(0.12f * strength)), Col(theme::Ink(0.03f * strength)));
}

ImVec2 Measure(ImFont* f, const char* s, float size, const char* end)
{
    if (size <= 0.0f) size = f->FontSize;
    return f->CalcTextSizeA(size, FLT_MAX, 0.0f, s, end);
}

void Text(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 col, const char* s, float size, const char* end)
{
    if (size <= 0.0f) size = f->FontSize;
    dl->AddText(f, size, ImVec2(floorf(pos.x), floorf(pos.y)), col, s, end);
}

void TextC(ImDrawList* dl, ImFont* f, ImVec2 c, ImU32 col, const char* s, float size)
{
    ImVec2 ts = Measure(f, s, size);
    Text(dl, f, c - ts * 0.5f, col, s, size);
}

void TextGradient(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 c0, ImU32 c1, const char* s, float size)
{
    ImVec2 ts = Measure(f, s, size);
    int v0 = dl->VtxBuffer.Size;
    Text(dl, f, pos, IM_COL32_WHITE, s, size);
    Shade(dl, v0, pos, pos + ImVec2(ts.x, 0), c0, c1);
}

static float SpacedImpl(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 col, const char* s, float spacing, float size)
{
    if (size <= 0.0f) size = f->FontSize;
    float x = pos.x;
    const char* p = s;
    bool any = false;
    while (*p)
    {
        unsigned int ch = 0;
        int n = ImTextCharFromUtf8(&ch, p, nullptr);
        if (n <= 0) break;
        if (dl) dl->AddText(f, size, ImVec2(floorf(x), floorf(pos.y)), col, p, p + n);
        x += f->CalcTextSizeA(size, FLT_MAX, 0.0f, p, p + n).x + spacing;
        p += n;
        any = true;
    }
    return any ? (x - pos.x - spacing) : 0.0f;
}

float Spaced(ImDrawList* dl, ImFont* f, ImVec2 pos, ImU32 col, const char* s, float spacing, float size)
{
    return SpacedImpl(dl, f, pos, col, s, spacing, size);
}

float SpacedWidth(ImFont* f, const char* s, float spacing, float size)
{
    return SpacedImpl(nullptr, f, ImVec2(0, 0), 0, s, spacing, size);
}

void Spinner(ImDrawList* dl, ImVec2 c, float r, float thick, ImU32 col, float speed)
{
    float t = (float)ImGui::GetTime() * speed;
    float a0 = t * 5.5f;
    float len = IM_PI * (0.55f + 0.45f * sinf(t * 2.3f));
    dl->PathArcTo(c, r, a0, a0 + len, 28);
    dl->PathStroke(col, 0, thick);
}

void Check(ImDrawList* dl, ImVec2 c, float size, ImU32 col, float thick, float p)
{
    if (p <= 0.0f) return;
    ImVec2 p0 = c + ImVec2(-0.42f, 0.02f) * size;
    ImVec2 p1 = c + ImVec2(-0.12f, 0.32f) * size;
    ImVec2 p2 = c + ImVec2(0.45f, -0.30f) * size;
    float l1 = Len(p1 - p0), l2 = Len(p2 - p1);
    float d = ImSaturate(p) * (l1 + l2);
    dl->PathLineTo(p0);
    if (d <= l1) dl->PathLineTo(p0 + (p1 - p0) * (d / l1));
    else { dl->PathLineTo(p1); dl->PathLineTo(p1 + (p2 - p1) * ((d - l1) / l2)); }
    dl->PathStroke(col, 0, thick);
}

void Arc(ImDrawList* dl, ImVec2 c, float r, float a0, float a1, float thick, ImU32 c0, ImU32 c1)
{
    if (a1 - a0 < 0.002f) return;
    int seg = ImMax(6, (int)((a1 - a0) / (IM_PI * 2.0f) * 110.0f));
    int v0 = dl->VtxBuffer.Size;
    dl->PathArcTo(c, r, a0, a1, seg);
    dl->PathStroke(IM_COL32_WHITE, 0, thick);
    ImVec4 ca = ImGui::ColorConvertU32ToFloat4(c0), cb = ImGui::ColorConvertU32ToFloat4(c1);
    float span = a1 - a0;
    for (int i = v0; i < dl->VtxBuffer.Size; i++)
    {
        ImDrawVert& v = dl->VtxBuffer[i];
        float ang = atan2f(v.pos.y - c.y, v.pos.x - c.x);
        float rel = fmodf(ang - a0 + 0.15f + IM_PI * 8.0f, IM_PI * 2.0f) - 0.15f;
        float t = ImSaturate(rel / span);
        float va = (float)((v.col >> IM_COL32_A_SHIFT) & 0xFF) / 255.0f;
        ImVec4 cc = Mix(ca, cb, t);
        cc.w *= va;
        v.col = ImGui::ColorConvertFloat4ToU32(cc);
    }
}

void BrandMark(ImDrawList* dl, ImVec2 c, float r, bool glow)
{
    if (glow) Glow(dl, c, r * 3.4f, Col(theme::accent, 0.40f));
    const float open = 0.78f;
    Arc(dl, c, r, open, IM_PI * 2.0f - open, r * 0.38f, Col(theme::accent2), Col(theme::accent));
    dl->AddCircleFilled(c + ImVec2(r * 0.98f, 0), r * 0.21f, Col(theme::accent2), 20);
}

} // namespace gfx

// ================================================================== background
// Five animated scenes (aurora, waves, warp tunnel, perspective grid, constellation).
// The active scene crossfades whenever the page or the setting changes.
namespace bg {

struct Particle { ImVec2 p, v; float r, phase; };
struct Star     { float x, y, z, pz; };

static std::vector<Particle> g_particles;
static std::vector<Star>     g_stars;
static ImVec2 g_area;
static int    g_scene = BG_WAVES, g_cur = -1, g_prev = -1;
static float  g_mix = 1.0f, g_boost = 0.0f;
static bool   g_boostOn = false;

void SetScene(int s) { g_scene = s; }
void SetBoost(bool on) { g_boostOn = on; }

static float Rand01()
{
    static unsigned int s = 0x9E3779B9u;
    s ^= s << 13; s ^= s >> 17; s ^= s << 5;
    return (s & 0xFFFFFF) / (float)0xFFFFFF;
}

static float GlowScale() { return 1.0f - 0.45f * theme::Brightness(); }

static void Aurora(ImDrawList* dl, ImVec2 a, ImVec2 b, float t)
{
    using namespace gfx;
    ImVec2 sz = b - a;
    float gs = GlowScale();
    ImVec2 p1 = a + ImVec2(sz.x * (0.16f + 0.06f * sinf(t * 0.21f)), sz.y * (0.12f + 0.08f * cosf(t * 0.17f)));
    ImVec2 p2 = a + ImVec2(sz.x * (0.86f + 0.05f * cosf(t * 0.15f)), sz.y * (0.84f + 0.07f * sinf(t * 0.19f)));
    ImVec2 p3 = a + ImVec2(sz.x * (0.55f + 0.14f * sinf(t * 0.11f)), sz.y * (0.48f + 0.12f * sinf(t * 0.13f + 1.0f)));
    Glow(dl, p1, S(560), Col(theme::accent, 0.20f * gs));
    Glow(dl, p2, S(600), Col(theme::accent2, 0.13f * gs));
    Glow(dl, p3, S(420), Col(Mix(theme::accent, theme::accent2, 0.5f), 0.07f * gs));

    ImVec2 m = ImGui::GetIO().MousePos;
    bool mv = ImGui::IsMousePosValid(&m);
    float sp = S(26), half = S(0.75f), spot = S(210);
    for (float y = a.y + sp * 0.5f; y < b.y; y += sp)
        for (float x = a.x + sp * 0.5f; x < b.x; x += sp)
        {
            float k = 0.0f;
            if (mv)
            {
                float dx = x - m.x, dy = y - m.y;
                float d = sqrtf(dx * dx + dy * dy);
                if (d < spot) { k = 1.0f - d / spot; k *= k; }
            }
            ImVec4 c = Mix(theme::Ink(0.035f), ImVec4(theme::accent2.x, theme::accent2.y, theme::accent2.z, 0.55f), k);
            float h = half * (1.0f + k * 0.8f);
            dl->AddRectFilled(ImVec2(x - h, y - h), ImVec2(x + h, y + h), Col(c));
        }
}

static void Waves(ImDrawList* dl, ImVec2 a, ImVec2 b, float t)
{
    using namespace gfx;
    ImVec2 sz = b - a;
    Glow(dl, a + ImVec2(sz.x * 0.5f, sz.y * 0.55f), S(620), Col(theme::accent, 0.10f * GlowScale()));
    const int lines = 24;
    float step = S(14), inv = 1.0f / dpi;
    static ImVec2 pts[256];
    for (int i = 0; i < lines; i++)
    {
        float k = (float)i / (lines - 1);
        float base = a.y + sz.y * (0.26f + 0.52f * k);
        float edge = 1.0f - fabsf(k - 0.5f) * 2.0f;
        int n = 0;
        for (float x = a.x - step; x <= b.x + step && n < 256; x += step)
        {
            float lx = (x - a.x) * inv;
            float y = base + sinf(lx * 0.0055f + t * 0.55f + i * 0.30f) * S(34) * (0.5f + 0.5f * edge)
                           + sinf(lx * 0.0125f - t * 0.80f + i * 0.17f) * S(12);
            pts[n++] = ImVec2(x, y);
        }
        ImVec4 c = Mix(theme::Ink(), theme::accent2, 0.5f);
        dl->AddPolyline(pts, n, Col(c, (0.035f + 0.15f * edge * edge) * (theme::light ? 1.7f : 1.0f)), 0, S(1.1f));
    }
}

static void Warp(ImDrawList* dl, ImVec2 a, ImVec2 b, float dt)
{
    using namespace gfx;
    ImVec2 sz = b - a, c = a + sz * 0.5f;
    if (g_stars.empty())
        for (int i = 0; i < 280; i++)
        {
            float z = 0.05f + Rand01() * 0.95f;
            g_stars.push_back({ (Rand01() - 0.5f) * 2.0f, (Rand01() - 0.5f) * 2.0f, z, z });
        }
    Glow(dl, c, S(340) + S(160) * g_boost, Col(theme::accent2, (0.10f + 0.12f * g_boost) * GlowScale()));
    float speed = 0.10f + 1.4f * g_boost;
    float scale = sz.x * 0.32f;
    for (Star& s : g_stars)
    {
        s.pz = s.z;
        s.z -= speed * dt;
        ImVec2 p = c + ImVec2(s.x / s.z, s.y / s.z) * scale;
        if (s.z <= 0.02f || p.x < a.x - 40 || p.x > b.x + 40 || p.y < a.y - 40 || p.y > b.y + 40)
        {
            s.x = (Rand01() - 0.5f) * 2.0f;
            s.y = (Rand01() - 0.5f) * 2.0f;
            s.z = s.pz = 1.0f;
            continue;
        }
        ImVec2 pp = c + ImVec2(s.x / s.pz, s.y / s.pz) * scale;
        float depth = 1.0f - s.z;
        // stretch the trail so streaks stay visible at low speed too
        ImVec2 tail = p + (pp - p) * (2.5f + 3.0f * g_boost);
        dl->AddLine(tail, p, Col(Mix(theme::Ink(), theme::accent2, 0.3f), ImSaturate(depth * 1.1f) * 0.75f), S(0.8f) + depth * S(1.4f));
    }
}

static void Grid(ImDrawList* dl, ImVec2 a, ImVec2 b, float t)
{
    using namespace gfx;
    ImVec2 sz = b - a;
    float cx = a.x + sz.x * 0.5f, hy = a.y + sz.y * 0.58f;
    float gs = GlowScale();
    Glow(dl, ImVec2(cx, hy), S(560), Col(theme::accent2, 0.12f * gs));
    Glow(dl, ImVec2(cx, hy), S(220), Col(theme::accent2, 0.10f * gs));
    Gradient(dl, ImVec2(a.x, hy - S(0.5f)), ImVec2(cx, hy + S(0.5f)), Col(theme::Ink(0.0f)), Col(theme::accent2, 0.45f));
    Gradient(dl, ImVec2(cx, hy - S(0.5f)), ImVec2(b.x, hy + S(0.5f)), Col(theme::accent2, 0.45f), Col(theme::Ink(0.0f)));
    for (int k = -26; k <= 26; k++)
    {
        float f = 1.0f - fabsf((float)k) / 26.0f;
        dl->AddLine(ImVec2(cx + k * S(7), hy), ImVec2(cx + k * S(95), b.y), Col(theme::accent2, 0.13f * f), S(1));
    }
    const int N = 14;
    for (int j = 0; j < N; j++)
    {
        float f = fmodf((j + t * 0.55f) / N, 1.0f);
        float yy = hy + (b.y - hy) * f * f;
        dl->AddLine(ImVec2(a.x, yy), ImVec2(b.x, yy), Col(theme::accent2, 0.02f + 0.16f * f), S(1));
    }
    // twinkling sky
    for (int i = 0; i < 46; i++)
    {
        float x = a.x + fmodf(i * 137.5f * 7.31f, sz.x), y = a.y + fmodf(i * 59.3f * 3.17f, (hy - a.y) * 0.9f);
        float tw = 0.25f + 0.25f * sinf(t * 1.3f + i * 1.7f);
        dl->AddCircleFilled(ImVec2(x, y), S(0.9f), Col(theme::Ink(tw)), 6);
    }
}

static void Particles(ImDrawList* dl, ImVec2 a, ImVec2 b, float t, float dt, float strength)
{
    using namespace gfx;
    ImVec2 sz = b - a;
    if (g_particles.empty() || g_area.x != sz.x || g_area.y != sz.y)
    {
        g_area = sz;
        g_particles.clear();
        for (int i = 0; i < 64; i++)
        {
            float ang = Rand01() * IM_PI * 2.0f, spd = S(6.0f + Rand01() * 14.0f);
            g_particles.push_back({ a + ImVec2(Rand01() * sz.x, Rand01() * sz.y), ImVec2(cosf(ang), sinf(ang)) * spd,
                                    S(0.8f + Rand01() * 1.4f), Rand01() * 6.28f });
        }
    }
    ImVec2 m = ImGui::GetIO().MousePos;
    bool mv = ImGui::IsMousePosValid(&m);
    const float link = S(118), mouseLink = S(160);
    for (Particle& p : g_particles)
    {
        p.p += p.v * dt;
        if (p.p.x < a.x - 10) p.p.x = b.x + 10; else if (p.p.x > b.x + 10) p.p.x = a.x - 10;
        if (p.p.y < a.y - 10) p.p.y = b.y + 10; else if (p.p.y > b.y + 10) p.p.y = a.y - 10;
    }
    for (size_t i = 0; i < g_particles.size(); i++)
    {
        const Particle& pi = g_particles[i];
        for (size_t j = i + 1; j < g_particles.size(); j++)
        {
            ImVec2 d = g_particles[j].p - pi.p;
            if (fabsf(d.x) > link || fabsf(d.y) > link) continue;
            float l = Len(d);
            if (l < link) dl->AddLine(pi.p, g_particles[j].p, Col(theme::accent2, 0.16f * strength * (1.0f - l / link)), S(1.0f));
        }
        if (mv)
        {
            float l = Len(m - pi.p);
            if (l < mouseLink) dl->AddLine(pi.p, m, Col(theme::accent2, 0.22f * strength * (1.0f - l / mouseLink)), S(1.0f));
        }
        float tw = 0.45f + 0.35f * sinf(t * 1.7f + pi.phase);
        dl->AddCircleFilled(pi.p, pi.r, Col(Mix(theme::Ink(), theme::accent2, 0.5f), tw * strength), 8);
    }
}

static void Scene(ImDrawList* dl, int s, ImVec2 a, ImVec2 b, float t, float dt, float alpha)
{
    if (alpha <= 0.01f) return;
    float& sa = ImGui::GetStyle().Alpha;
    float saved = sa;
    sa = alpha;
    switch (s)
    {
    case BG_AURORA: Aurora(dl, a, b, t); break;
    case BG_WAVES:  Waves(dl, a, b, t); break;
    case BG_WARP:   Warp(dl, a, b, dt); break;
    case BG_GRID:   Grid(dl, a, b, t); break;
    case BG_CONSTELLATION:
        gfx::Glow(dl, a + (b - a) * ImVec2(0.25f, 0.3f), S(480), gfx::Col(theme::accent, 0.08f * GlowScale()));
        gfx::Glow(dl, a + (b - a) * ImVec2(0.8f, 0.75f), S(480), gfx::Col(theme::accent2, 0.06f * GlowScale()));
        Particles(dl, a, b, t, dt, 1.6f);
        break;
    }
    sa = saved;
}

void Draw(ImDrawList* dl, ImVec2 a, ImVec2 b)
{
    using namespace gfx;
    float t = anim::Time(), dt = anim::Dt();
    g_boost += ((g_boostOn ? 1.0f : 0.0f) - g_boost) * (1.0f - expf(-2.5f * dt));

    dl->AddRectFilledMultiColor(a, b, Col(theme::bg1), Col(theme::bg1), Col(theme::bg0), Col(theme::bg0));

    int target = config::c.bgStyle == BG_AUTO ? g_scene : config::c.bgStyle;
    if (g_cur < 0) g_cur = target;
    if (target != g_cur) { g_prev = g_cur; g_cur = target; g_mix = 0.0f; }
    g_mix = ImMin(1.0f, g_mix + dt * 1.4f);

    if (config::c.bgEffects)
    {
        if (g_mix < 1.0f && g_prev >= 0) Scene(dl, g_prev, a, b, t, dt, 1.0f - g_mix);
        Scene(dl, g_cur, a, b, t, dt, anim::InOutCubic(g_mix));
    }
    if (config::c.particles && !(config::c.bgEffects && g_cur == BG_CONSTELLATION))
        Particles(dl, a, b, t, dt, 1.0f);

    // soft vignette
    Gradient(dl, a, ImVec2(b.x, a.y + S(120)), Col(Black(theme::light ? 0.16f : 0.35f)), Col(Black(0.0f)), 0.0f, true);
    Gradient(dl, ImVec2(a.x, b.y - S(140)), b, Col(Black(0.0f)), Col(Black(theme::light ? 0.22f : 0.45f)), 0.0f, true);
}

} // namespace bg
