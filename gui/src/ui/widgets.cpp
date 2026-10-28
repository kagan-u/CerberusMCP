#include "widgets.h"
#include "flags.h"
#include "../config.h"
#include "../lang/lang.h"
#include <string>
#include <vector>
#include <cstdio>

using namespace gfx;

namespace ui {

static ImVec4 WithA(ImVec4 c, float a) { c.w = a; return c; }

void Shine(ImDrawList* dl, ImVec2 a, ImVec2 b, float r, float alpha, float period)
{
    if (alpha <= 0.01f) return;
    float w = b.x - a.x, hgt = b.y - a.y;
    float t = fmodf((float)ImGui::GetTime(), period) / (period * 0.62f);
    if (t > 1.0f) return;
    float x = a.x - hgt - S(60) + t * (w + hgt + S(120));
    dl->PushClipRect(ImVec2(a.x + r * 0.6f, a.y), ImVec2(b.x - r * 0.6f, b.y), true);
    dl->AddQuadFilled(ImVec2(x, b.y), ImVec2(x + hgt * 0.6f, a.y), ImVec2(x + hgt * 0.6f + S(34), a.y), ImVec2(x + S(34), b.y), Col(White(0.16f * alpha)));
    dl->AddQuadFilled(ImVec2(x + S(44), b.y), ImVec2(x + hgt * 0.6f + S(44), a.y), ImVec2(x + hgt * 0.6f + S(52), a.y), ImVec2(x + S(52), b.y), Col(White(0.10f * alpha)));
    dl->PopClipRect();
}

bool Button(const char* label, ImVec2 size, ButtonStyle style, Icon icon, ImFont* font)
{
    ImGuiID id = ImGui::GetID(label);
    ImVec2 p = ImGui::GetCursorScreenPos();
    bool pressed = ImGui::InvisibleButton(label, size);
    bool hov = ImGui::IsItemHovered(), act = ImGui::IsItemActive();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float h  = anim::To(anim::Key(id, 0), hov ? 1.0f : 0.0f, 14.0f);
    float pr = anim::To(anim::Key(id, 1), act ? 1.0f : 0.0f, 24.0f);

    ImDrawList* dl = ImGui::GetWindowDrawList();
    float sc = 1.0f - 0.03f * pr;
    ImVec2 c = p + size * 0.5f, hs = size * 0.5f * sc;
    ImVec2 a = c - hs, b = c + hs;
    float r = ImMin(S(12), size.y * 0.5f);
    ImU32 tc = Col(theme::text);

    switch (style)
    {
    case Btn_Primary:
        Shadow(dl, a, b, r, S(16), Col(theme::accent, 0.22f + 0.30f * h), ImVec2(0, S(5)));
        Gradient(dl, a, b, Col(theme::accent), Col(theme::accent2), r);
        dl->AddRectFilled(a, b, Col(White(0.10f * h)), r);
        GradientBorder(dl, a, b, r, Col(White(0.35f)), Col(White(0.05f)));
        Shine(dl, a, b, r, (0.5f + 0.5f * h) * (1.0f - 0.6f * theme::Brightness()));
        tc = Col(theme::OnAccent());
        break;
    case Btn_Secondary:
        dl->AddRectFilled(a, b, Col(theme::surface2, 0.9f), r);
        dl->AddRectFilled(a, b, Col(theme::Ink(0.05f * h)), r);
        GradientBorder(dl, a, b, r, Col(Mix(theme::Ink(0.10f), theme::accent, h)), Col(Mix(theme::Ink(0.04f), theme::accent2, h * 0.6f)));
        break;
    case Btn_Ghost:
        dl->AddRectFilled(a, b, Col(theme::Ink(0.03f + 0.05f * h)), r);
        dl->AddRect(a, b, Col(theme::Ink(0.07f + 0.05f * h)), r);
        tc = Col(Mix(theme::textDim, theme::text, h));
        break;
    case Btn_Danger:
        dl->AddRectFilled(a, b, Col(theme::danger, 0.10f + 0.10f * h), r);
        dl->AddRect(a, b, Col(theme::danger, 0.30f + 0.35f * h), r);
        tc = Col(Mix(theme::danger, theme::text, 0.25f + 0.35f * h));
        break;
    }

    const char* end = ImGui::FindRenderedTextEnd(label);
    ImFont* f = font ? font : fonts::bold;
    ImVec2 ts = Measure(f, label, 0.0f, end);
    float iconW = icon != Ico_None ? S(16) + (end > label ? S(9) : 0.0f) : 0.0f;
    float x = c.x - (iconW + ts.x) * 0.5f;
    if (icon != Ico_None) DrawIcon(dl, icon, ImVec2(x + S(8), c.y), S(15), tc);
    if (end > label) Text(dl, f, ImVec2(x + iconW, c.y - ts.y * 0.5f), tc, label, 0.0f, end);
    return pressed;
}

bool IconButton(const char* id, Icon icon, float size, const char* tooltip, ImU32 hoverTint)
{
    ImGuiID gid = ImGui::GetID(id);
    ImVec2 p = ImGui::GetCursorScreenPos();
    bool pressed = ImGui::InvisibleButton(id, ImVec2(size, size));
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float h = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 16.0f);
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 c = p + ImVec2(size, size) * 0.5f;
    if (hoverTint)
    {
        ImVec4 tint = ImGui::ColorConvertU32ToFloat4(hoverTint);
        dl->AddRectFilled(p, p + ImVec2(size, size), Col(tint, h), S(9));
        DrawIcon(dl, icon, c, size * 0.42f, Col(Mix(theme::textDim, theme::text, h)));
    }
    else
    {
        dl->AddRectFilled(p, p + ImVec2(size, size), Col(theme::Ink(0.07f * h)), S(9));
        DrawIcon(dl, icon, c, size * 0.46f, Col(Mix(theme::textMute, theme::text, h)));
    }
    if (tooltip) Tooltip(tooltip);
    return pressed;
}

bool Checkbox(const char* label, bool* v)
{
    ImGuiID id = ImGui::GetID(label);
    ImVec2 p = ImGui::GetCursorScreenPos();
    const char* end = ImGui::FindRenderedTextEnd(label);
    ImVec2 ts = Measure(fonts::body, label, 0.0f, end);
    float bs = S(18);
    ImVec2 size(bs + S(10) + ts.x, ImMax(bs, ts.y));
    bool pressed = ImGui::InvisibleButton(label, size);
    if (pressed) *v = !*v;
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float on = anim::To(anim::Key(id, 0), *v ? 1.0f : 0.0f, 14.0f);
    float h = anim::To(anim::Key(id, 1), hov ? 1.0f : 0.0f, 14.0f);

    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 a(p.x, p.y + (size.y - bs) * 0.5f), b = a + ImVec2(bs, bs);
    float r = S(5);
    dl->AddRectFilled(a, b, Col(theme::Ink(0.05f + 0.04f * h)), r);
    dl->AddRect(a, b, Col(Mix(theme::Ink(0.16f), theme::accent2, h * 0.6f)), r, 0, S(1.2f));
    if (on > 0.01f)
    {
        int v0 = dl->VtxBuffer.Size;
        dl->AddRectFilled(a, b, IM_COL32_WHITE, r);
        Shade(dl, v0, a, b, Col(theme::accent, on), Col(theme::accent2, on));
        Glow(dl, (a + b) * 0.5f, bs * 1.4f, Col(theme::accent, 0.25f * on));
        Check(dl, (a + b) * 0.5f, bs * 0.62f, Col(theme::OnAccent()), S(2.0f), anim::OutCubic(on));
    }
    Text(dl, fonts::body, ImVec2(b.x + S(10), p.y + (size.y - ts.y) * 0.5f), Col(Mix(theme::textDim, theme::text, h)), label, 0.0f, end);
    return pressed;
}

bool Toggle(const char* label, const char* desc, bool* v, float width)
{
    ImGuiID id = ImGui::GetID(label);
    ImVec2 p = ImGui::GetCursorScreenPos();
    float hgt = desc ? S(46) : S(32);
    bool pressed = ImGui::InvisibleButton(label, ImVec2(width, hgt));
    if (pressed) *v = !*v;
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float on = anim::To(anim::Key(id, 0), *v ? 1.0f : 0.0f, 13.0f);
    float h = anim::To(anim::Key(id, 1), hov ? 1.0f : 0.0f, 14.0f);

    ImDrawList* dl = ImGui::GetWindowDrawList();
    const char* end = ImGui::FindRenderedTextEnd(label);
    float lh = fonts::body->FontSize;
    float ty = desc ? p.y + S(4) : p.y + (hgt - lh) * 0.5f;
    Text(dl, fonts::body, ImVec2(p.x, ty), Col(theme::light ? theme::text : Mix(theme::text, White(), h * 0.3f)), label, 0.0f, end);
    if (desc) Text(dl, fonts::small, ImVec2(p.x, ty + lh + S(3)), Col(theme::textMute), desc);

    float sw = S(42), sh = S(24);
    ImVec2 sa(p.x + width - sw, p.y + (hgt - sh) * 0.5f), sb = sa + ImVec2(sw, sh);
    dl->AddRectFilled(sa, sb, Col(theme::Ink(0.09f + 0.04f * h)), sh * 0.5f);
    if (on > 0.01f)
    {
        Shadow(dl, sa, sb, sh * 0.5f, S(9), Col(theme::accent, 0.40f * on));
        Gradient(dl, sa, sb, Col(theme::accent, on), Col(theme::accent2, on), sh * 0.5f);
    }
    dl->AddRect(sa, sb, Col(theme::Ink(0.06f + 0.06f * h)), sh * 0.5f);
    float kr = sh * 0.5f - S(3);
    float kx = sa.x + sh * 0.5f + (sw - sh) * anim::OutBack(on);
    ImVec2 kc(kx, sa.y + sh * 0.5f);
    dl->AddCircleFilled(kc + ImVec2(0, S(1)), kr + S(0.5f), Col(Black(0.30f)), 24);
    ImVec4 knobOn = Mix(White(), ImVec4(0.06f, 0.06f, 0.07f, 1.0f), theme::Brightness());
    dl->AddCircleFilled(kc, kr, Col(Mix(ImVec4(0.80f, 0.80f, 0.84f, 1.0f), knobOn, on)), 24);
    return pressed;
}

bool Slider(const char* label, float* v, float mn, float mx, const char* fmt, float width, float mul)
{
    ImGuiID id = ImGui::GetID(label);
    ImVec2 p = ImGui::GetCursorScreenPos();
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const char* end = ImGui::FindRenderedTextEnd(label);
    Text(dl, fonts::body, p, Col(theme::text), label, 0.0f, end);

    char buf[32];
    snprintf(buf, sizeof(buf), fmt, *v * mul);
    ImVec2 vs = Measure(fonts::smallBold, buf);
    ImVec2 pa(p.x + width - vs.x - S(16), p.y - S(1)), pb(p.x + width, p.y + vs.y + S(5));
    dl->AddRectFilled(pa, pb, Col(theme::accent, 0.16f), S(7));
    Text(dl, fonts::smallBold, ImVec2(pa.x + S(8), pa.y + S(3)), Col(theme::accent2), buf);

    float ty = p.y + S(36), th = S(6);
    ImGui::SetCursorScreenPos(ImVec2(p.x, ty - S(11)));
    ImGui::InvisibleButton(label, ImVec2(width, S(22)));
    bool act = ImGui::IsItemActive(), hov = ImGui::IsItemHovered();
    if (hov || act) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    bool changed = false;
    if (act)
    {
        float t = ImSaturate((ImGui::GetIO().MousePos.x - p.x) / width);
        float nv = mn + t * (mx - mn);
        if (nv != *v) { *v = nv; changed = true; }
    }
    float target = ImSaturate((*v - mn) / (mx - mn));
    float t = anim::ToFrom(anim::Key(id, 0), target, 20.0f, target);
    float h = anim::To(anim::Key(id, 1), (hov || act) ? 1.0f : 0.0f, 14.0f);

    ImVec2 ta(p.x, ty - th * 0.5f), tb(p.x + width, ty + th * 0.5f);
    dl->AddRectFilled(ta, tb, Col(theme::Ink(0.08f)), th * 0.5f);
    ImVec2 fe(p.x + ImMax(width * t, th), tb.y);
    Gradient(dl, ta, fe, Col(theme::accent), Col(theme::accent2), th * 0.5f);
    ImVec2 kc(p.x + width * t, ty);
    Glow(dl, kc, S(18) + S(8) * h, Col(theme::accent2, 0.35f + 0.25f * h));
    dl->AddCircleFilled(kc, S(7.5f) + S(1.5f) * h, Col(White()), 24);
    dl->AddCircle(kc, S(7.5f) + S(1.5f) * h, Col(theme::Ink(0.28f)), 24, S(1.2f));
    dl->AddCircleFilled(kc, S(3.0f), Col(theme::bg0), 16);
    return changed;
}

bool Swatch(const char* id, ImVec4 ca, ImVec4 cb, bool selected, float size, bool rainbow)
{
    ImGuiID gid = ImGui::GetID(id);
    ImVec2 p = ImGui::GetCursorScreenPos();
    bool pressed = ImGui::InvisibleButton(id, ImVec2(size, size));
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float s = anim::To(anim::Key(gid, 0), selected ? 1.0f : 0.0f, 12.0f);
    float h = anim::To(anim::Key(gid, 1), hov ? 1.0f : 0.0f, 14.0f);
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 c = p + ImVec2(size, size) * 0.5f;
    float r = size * 0.5f - S(5) + S(1.5f) * h - S(2) * s;

    if (rainbow)
    {
        const int seg = 36;
        for (int i = 0; i < seg; i++)
        {
            float a0 = (float)i / seg * IM_PI * 2.0f, a1 = (float)(i + 1) / seg * IM_PI * 2.0f + 0.02f;
            ImVec4 col(0, 0, 0, 1);
            ImGui::ColorConvertHSVtoRGB((float)i / seg, 0.75f, 1.0f, col.x, col.y, col.z);
            dl->PathArcTo(c, r - S(2), a0, a1, 3);
            dl->PathStroke(Col(col), 0, S(4));
        }
        dl->AddCircleFilled(c, r - S(5), Col(ca), 24);
    }
    else
    {
        Glow(dl, c, r * 2.2f, Col(ca, 0.20f + 0.30f * s));
        int v0 = dl->VtxBuffer.Size;
        dl->AddCircleFilled(c, r, IM_COL32_WHITE, 32);
        Shade(dl, v0, c - ImVec2(r, r), c + ImVec2(r, r), Col(ca), Col(cb));
    }
    if (s > 0.01f)
    {
        float lum = 0.2126f * ca.x + 0.7152f * ca.y + 0.0722f * ca.z;
        dl->AddCircle(c, r + S(4), Col(theme::Ink(0.80f * s)), 32, S(1.6f));
        Check(dl, c, r * 0.9f, Col(lum > 0.6f ? ImVec4(0.05f, 0.05f, 0.06f, s) : White(s)), S(2.0f), s);
    }
    return pressed;
}

bool FlagButton(const char* id, int lang, ImVec2 size, bool selected)
{
    ImGuiID gid = ImGui::GetID(id);
    ImVec2 p = ImGui::GetCursorScreenPos();
    bool pressed = ImGui::InvisibleButton(id, size);
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float s = anim::To(anim::Key(gid, 0), selected ? 1.0f : 0.0f, 12.0f);
    float h = anim::To(anim::Key(gid, 1), hov ? 1.0f : 0.0f, 14.0f);
    ImDrawList* dl = ImGui::GetWindowDrawList();

    float lift = S(3) * ImMax(h, s);
    ImVec2 a = p - ImVec2(0, lift), b = p + size - ImVec2(0, lift);
    float r = size.y * 0.2f;
    Shadow(dl, a, b, r, S(8), Col(Black((theme::light ? 0.16f : 0.35f) + 0.2f * h)), ImVec2(0, S(3)));
    if (s > 0.01f)
    {
        Glow(dl, (a + b) * 0.5f, size.x * 1.1f, Col(theme::accent, 0.30f * s));
        GradientBorder(dl, a - S2(3, 3), b + S2(3, 3), r + S(3), Col(theme::accent, s), Col(theme::accent2, s), S(1.6f));
    }
    flags::Draw(dl, lang, a, b, 0.55f + 0.45f * ImMax(h, s));
    if (hov) Tooltip(lang::NativeName(lang));
    return pressed;
}

float Badge(ImDrawList* dl, ImVec2 p, const char* txt, ImVec4 c, bool pulse)
{
    ImFont* f = fonts::smallBold;
    ImVec2 ts = Measure(f, txt);
    float hgt = S(22), w = ts.x + S(28);
    dl->AddRectFilled(p, p + ImVec2(w, hgt), Col(c, 0.13f), hgt * 0.5f);
    dl->AddRect(p, p + ImVec2(w, hgt), Col(c, 0.28f), hgt * 0.5f);
    ImVec2 dot(p.x + S(11), p.y + hgt * 0.5f);
    if (pulse)
    {
        float k = fmodf((float)ImGui::GetTime(), 1.6f) / 1.6f;
        dl->AddCircleFilled(dot, S(3) + S(5) * k, Col(c, 0.40f * (1.0f - k)), 16);
    }
    dl->AddCircleFilled(dot, S(3), Col(c), 12);
    Text(dl, f, ImVec2(p.x + S(19), p.y + (hgt - ts.y) * 0.5f), Col(c), txt);
    return w;
}

float Chip(ImDrawList* dl, ImVec2 p, const char* txt, ImVec4 c, Icon icon)
{
    ImFont* f = fonts::smallBold;
    ImVec2 ts = Measure(f, txt);
    float hgt = S(28), iw = icon != Ico_None ? S(18) : 0.0f;
    float w = ts.x + S(24) + iw;
    dl->AddRectFilled(p, p + ImVec2(w, hgt), Col(c, 0.10f), S(8));
    dl->AddRect(p, p + ImVec2(w, hgt), Col(c, 0.22f), S(8));
    if (icon != Ico_None) DrawIcon(dl, icon, ImVec2(p.x + S(12) + S(5), p.y + hgt * 0.5f), S(11), Col(c));
    Text(dl, f, ImVec2(p.x + S(12) + iw, p.y + (hgt - ts.y) * 0.5f), Col(Mix(c, theme::text, 0.55f)), txt);
    return w;
}

float FeatureChip(ImDrawList* dl, ImVec2 p, const char* txt, Icon icon, bool measureOnly)
{
    ImFont* f = fonts::smallBold;
    ImVec2 ts = Measure(f, txt);
    float hgt = S(34), ic = S(24);
    float w = S(5) + ic + S(9) + ts.x + S(15);
    if (measureOnly) return w;

    ImVec2 b = p + ImVec2(w, hgt);
    bool hov = ImGui::IsMouseHoveringRect(p, b) && ImGui::IsWindowHovered();
    float hv = anim::To(ImGui::GetID(txt), hov ? 1.0f : 0.0f, 14.0f);
    float r = hgt * 0.5f;

    Shadow(dl, p, b, r, S(10), Col(Black(theme::light ? 0.20f : 0.40f)), ImVec2(0, S(3)));
    Gradient(dl, p, b, Col(theme::Ink(0.09f + 0.05f * hv)), Col(theme::Ink(0.02f)), r, true);
    GradientBorder(dl, p, b, r, Col(Mix(theme::Ink(0.28f + 0.22f * hv), theme::accent2, hv)), Col(theme::Ink(0.05f)), S(1.0f));
    ImVec2 icc(p.x + S(5) + ic * 0.5f, p.y + hgt * 0.5f);
    Glow(dl, icc, ic * 1.5f, Col(theme::accent2, 0.22f + 0.25f * hv));
    int v0 = dl->VtxBuffer.Size;
    dl->AddCircleFilled(icc, ic * 0.5f, IM_COL32_WHITE, 24);
    Shade(dl, v0, icc - ImVec2(0, ic * 0.5f), icc + ImVec2(0, ic * 0.5f), Col(White()), Col(ImVec4(0.60f, 0.60f, 0.64f, 1.0f)));
    DrawIcon(dl, icon, icc, S(12), Col(ImVec4(0.06f, 0.06f, 0.07f, 1.0f)), S(1.6f));
    Text(dl, f, ImVec2(icc.x + ic * 0.5f + S(9), p.y + (hgt - ts.y) * 0.5f), Col(theme::text), txt);
    if (hv > 0.01f) Shine(dl, p, b, r, hv, 1.8f);
    return w;
}

int LanguagePicker(const char* id, float width, bool openUp)
{
    ImGuiID gid = ImGui::GetID(id);
    ImVec2 p = ImGui::GetCursorScreenPos();
    float bh = S(42);
    char popupId[64];
    snprintf(popupId, sizeof(popupId), "%s_list", id);
    bool isOpen = ImGui::IsPopupOpen(popupId);

    // a click on the button while the list is open only closes it (ImGui closes on outside click)
    float& suppress = anim::Ref(anim::Key(gid, 5), 0.0f);
    if (isOpen && ImGui::IsMouseClicked(0) && ImGui::IsMouseHoveringRect(p, p + ImVec2(width, bh))) suppress = 1.0f;

    bool pressed = ImGui::InvisibleButton(id, ImVec2(width, bh));
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    if (pressed)
    {
        if (suppress > 0.5f) suppress = 0.0f;
        else if (!isOpen) ImGui::OpenPopup(popupId);
    }
    float h = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 14.0f);
    float o = anim::To(anim::Key(gid, 1), isOpen ? 1.0f : 0.0f, 14.0f);

    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 b = p + ImVec2(width, bh);
    float r = S(12);
    dl->AddRectFilled(p, b, Col(theme::Ink(0.04f + 0.03f * h + 0.03f * o)), r);
    GradientBorder(dl, p, b, r, Col(Mix(theme::Ink(0.12f), theme::accent2, 0.5f * ImMax(h, o))), Col(theme::Ink(0.04f)), S(1.1f));
    int cur = lang::current;
    ImVec2 fs = S2(30, 20);
    ImVec2 fa(p.x + S(12), p.y + (bh - fs.y) * 0.5f);
    flags::Draw(dl, cur, fa, fa + fs);
    Text(dl, fonts::bold, ImVec2(fa.x + fs.x + S(12), p.y + (bh - fonts::bold->FontSize) * 0.5f), Col(theme::text), lang::NativeName(cur));
    const char* code = lang::Code(cur);
    ImVec2 cs = Measure(fonts::smallBold, code);
    Text(dl, fonts::smallBold, ImVec2(b.x - S(38) - cs.x, p.y + (bh - cs.y) * 0.5f), Col(theme::textMute), code);
    // chevron that flips while opening
    {
        float fl = cosf(IM_PI * o) * (openUp ? -1.0f : 1.0f);
        ImVec2 cc(b.x - S(20), p.y + bh * 0.5f);
        float cw = S(5);
        dl->PathLineTo(cc + ImVec2(-cw, -cw * 0.5f * fl));
        dl->PathLineTo(cc + ImVec2(0, cw * 0.5f * fl));
        dl->PathLineTo(cc + ImVec2(cw, -cw * 0.5f * fl));
        dl->PathStroke(Col(Mix(theme::textDim, theme::text, ImMax(h, o))), 0, S(1.6f));
    }

    int picked = -1;
    float rh = S(38), pad = S(6);
    float listH = LANG_COUNT * rh + pad * 2;
    ImVec2 lp = openUp ? ImVec2(p.x, p.y - listH - S(8)) : ImVec2(p.x, b.y + S(8));
    ImGui::SetNextWindowPos(lp);
    ImGui::SetNextWindowSize(ImVec2(width, listH));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
    ImGui::PushStyleVar(ImGuiStyleVar_PopupBorderSize, 0.0f);
    ImGui::PushStyleColor(ImGuiCol_PopupBg, ImVec4(0, 0, 0, 0));
    if (ImGui::BeginPopup(popupId, ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse))
    {
        float& t = anim::Ref(anim::Key(gid, 3), 0.0f);
        if (ImGui::IsWindowAppearing()) t = 0.0f;
        t = ImMin(1.0f, t + ImMin(ImGui::GetIO().DeltaTime, 0.1f) * 3.5f);
        float e = anim::OutCubic(t);
        ImDrawList* pdl = ImGui::GetWindowDrawList();
        ImVec2 wa = ImGui::GetWindowPos(), wb = wa + ImVec2(width, listH);
        float vis = listH * e;
        ImVec2 ca = openUp ? ImVec2(wa.x - S(20), wb.y - vis) : ImVec2(wa.x - S(20), wa.y - S(20));
        ImVec2 cb = openUp ? ImVec2(wb.x + S(20), wb.y + S(20)) : ImVec2(wb.x + S(20), wa.y + vis);
        pdl->PushClipRect(ca, cb, false);
        Shadow(pdl, wa, wb, S(14), S(20), Col(Black(theme::light ? 0.24f : 0.55f)), ImVec2(0, S(8)));
        pdl->AddRectFilled(wa, wb, Col(theme::light ? ImVec4(0.985f, 0.986f, 0.992f, 0.99f) : ImVec4(0.068f, 0.068f, 0.076f, 0.985f)), S(14));
        GradientBorder(pdl, wa, wb, S(14), Col(theme::Ink(0.16f)), Col(theme::Ink(0.04f)), S(1.0f));

        for (int l = 0; l < LANG_COUNT; l++)
        {
            int idx = openUp ? (LANG_COUNT - 1 - l) : l;  // keep the list order growing away from the button
            int lg = l;
            float rk = anim::OutCubic(t * 2.2f - idx * 0.12f);
            ImVec2 r0(wa.x + pad, wa.y + pad + lg * rh), rs(width - pad * 2, rh);
            ImGui::SetCursorScreenPos(r0);
            char rid[24];
            snprintf(rid, sizeof(rid), "##lrow%d", lg);
            bool rp = ImGui::InvisibleButton(rid, rs);
            bool rhov = ImGui::IsItemHovered();
            if (rhov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
            float hv = anim::To(ImGui::GetItemID(), rhov ? 1.0f : 0.0f, 16.0f);
            bool sel = lg == cur;
            float ox = (1.0f - rk) * S(14);
            if (sel)
            {
                Gradient(pdl, r0, r0 + rs, Col(theme::accent, 0.20f * rk), Col(theme::accent, 0.02f), S(9));
                pdl->AddRectFilled(ImVec2(r0.x, r0.y + S(10)), ImVec2(r0.x + S(3), r0.y + rs.y - S(10)), Col(theme::accent2, rk), S(2));
            }
            pdl->AddRectFilled(r0, r0 + rs, Col(theme::Ink(0.06f * hv)), S(9));
            ImVec2 ff(r0.x + S(12) + ox + S(3) * hv, r0.y + (rh - S(19)) * 0.5f);
            flags::Draw(pdl, lg, ff, ff + S2(28, 19), rk);
            Text(pdl, sel ? fonts::bold : fonts::body, ImVec2(ff.x + S(40), r0.y + (rh - fonts::body->FontSize) * 0.5f),
                 Col(Mix(theme::textDim, theme::text, ImMax(hv, sel ? 1.0f : 0.0f)), rk), lang::NativeName(lg));
            if (sel) DrawIcon(pdl, Ico_Check, ImVec2(r0.x + rs.x - S(18), r0.y + rh * 0.5f), S(13), Col(theme::accent2, rk));
            else
            {
                ImVec2 ccs = Measure(fonts::smallBold, lang::Code(lg));
                Text(pdl, fonts::smallBold, ImVec2(r0.x + rs.x - S(12) - ccs.x, r0.y + (rh - ccs.y) * 0.5f), Col(theme::textMute, rk), lang::Code(lg));
            }
            if (rp)
            {
                picked = lg;
                ImGui::CloseCurrentPopup();
            }
        }
        pdl->PopClipRect();
        ImGui::EndPopup();
    }
    ImGui::PopStyleColor();
    ImGui::PopStyleVar(2);

    // leave the cursor below the button for normal layout
    ImGui::SetCursorScreenPos(ImVec2(p.x, b.y));
    ImGui::Dummy(ImVec2(width, 0.0f));
    return picked;
}

void Tooltip(const char* text)
{
    if (!ImGui::IsItemHovered(ImGuiHoveredFlags_DelayShort | ImGuiHoveredFlags_NoSharedDelay)) return;
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, S2(10, 6));
    ImGui::PushStyleVar(ImGuiStyleVar_PopupRounding, S(8));
    ImGui::BeginTooltip();
    ImGui::PushFont(fonts::small);
    ImGui::TextUnformatted(text);
    ImGui::PopFont();
    ImGui::EndTooltip();
    ImGui::PopStyleVar(2);
}

// ------------------------------------------------------------------ cards
static ImVec2      g_cardPos;
static float       g_cardW;
static ImDrawList* g_cardDl;
static const float kCardPad = 22.0f;

void CardBegin(const char* title, Icon icon, ImVec2 pos, float width)
{
    g_cardDl = ImGui::GetWindowDrawList();
    g_cardDl->ChannelsSplit(2);
    g_cardDl->ChannelsSetCurrent(1);
    g_cardPos = pos;
    g_cardW = width;

    float pad = S(kCardPad), chip = S(32);
    ImVec2 ca = pos + ImVec2(pad, pad);
    Gradient(g_cardDl, ca, ca + ImVec2(chip, chip), Col(theme::accent, 0.28f), Col(theme::accent2, 0.12f), S(9));
    g_cardDl->AddRect(ca, ca + ImVec2(chip, chip), Col(theme::accent2, 0.25f), S(9));
    DrawIcon(g_cardDl, icon, ca + ImVec2(chip, chip) * 0.5f, S(15), Col(theme::accent2));
    Text(g_cardDl, fonts::title, ImVec2(ca.x + chip + S(12), ca.y + (chip - fonts::title->FontSize) * 0.5f), Col(theme::text), title);

    ImGui::SetCursorScreenPos(ImVec2(pos.x + pad, pos.y + pad + chip + S(18)));
    ImGui::BeginGroup();
}

float CardEnd()
{
    ImGui::EndGroup();
    float bottom = ImGui::GetItemRectMax().y + S(kCardPad);
    g_cardDl->ChannelsSetCurrent(0);
    Panel(g_cardDl, g_cardPos, ImVec2(g_cardPos.x + g_cardW, bottom), S(16));
    g_cardDl->ChannelsMerge();
    ImGui::SetCursorScreenPos(ImVec2(g_cardPos.x, bottom));
    ImGui::Dummy(ImVec2(g_cardW, 0.0f));
    return bottom;
}

float CardInnerWidth() { return g_cardW - S(kCardPad) * 2.0f; }

} // namespace ui

// ================================================================== toasts
namespace toast {

struct Item { Type type; std::string title, text; float life, dur, y; bool placed; };
static std::vector<Item> g_items;

void Push(Type type, const char* title, const char* text, float duration)
{
    if (!config::c.notifications && type != Error) return;
    g_items.insert(g_items.begin(), Item{ type, title ? title : "", text ? text : "", 0.0f, duration, 0.0f, false });
    if (g_items.size() > 4) g_items.resize(4);
}

void Render()
{
    if (g_items.empty()) return;
    ImDrawList* dl = ImGui::GetForegroundDrawList();
    ImVec2 ds = ImGui::GetIO().DisplaySize;
    float dt = ImMin(ImGui::GetIO().DeltaTime, 0.1f);
    float w = S(320), hgt = S(64), x1 = ds.x - S(18), y = S(58);

    for (size_t i = 0; i < g_items.size();)
    {
        Item& it = g_items[i];
        it.life += dt;
        float out = it.life > it.dur ? 1.0f - (it.life - it.dur) / 0.35f : 1.0f;
        if (out <= 0.0f) { g_items.erase(g_items.begin() + i); continue; }
        if (!it.placed) { it.y = y; it.placed = true; }
        it.y += (y - it.y) * (1.0f - expf(-14.0f * dt));

        float in = anim::OutBack(ImMin(it.life / 0.45f, 1.0f));
        float alpha = ImSaturate(ImMin(it.life / 0.2f, out));
        float off = (1.0f - in) * (w + S(40)) + (1.0f - out) * S(40);
        ImVec2 a(x1 - w + off, it.y), b(x1 + off, it.y + hgt);

        ImVec4 c = it.type == Success ? theme::success : it.type == Warning ? theme::warning : it.type == Error ? theme::danger : theme::accent2;
        Icon ic = it.type == Success ? Ico_Check : it.type == Warning ? Ico_Info : it.type == Error ? Ico_Close : Ico_Bell;

        Shadow(dl, a, b, S(14), S(18), Col(Black((theme::light ? 0.22f : 0.45f) * alpha)), ImVec2(0, S(6)));
        dl->AddRectFilled(a, b, Col(theme::light ? ImVec4(0.985f, 0.986f, 0.992f, 0.97f) : ImVec4(0.075f, 0.075f, 0.083f, 0.97f), alpha), S(14));
        Gradient(dl, a, ImVec2(a.x + w * 0.55f, b.y), Col(c, 0.12f * alpha), Col(c, 0.0f), S(14));
        GradientBorder(dl, a, b, S(14), Col(c, 0.45f * alpha), Col(theme::Ink(0.06f * alpha)));

        ImVec2 ic0(a.x + S(32), a.y + hgt * 0.5f);
        Glow(dl, ic0, S(30), Col(c, 0.35f * alpha));
        dl->AddCircleFilled(ic0, S(15), Col(c, 0.18f * alpha), 32);
        DrawIcon(dl, ic, ic0, S(13), Col(c, alpha));

        float tx = a.x + S(58);
        if (it.text.empty())
            Text(dl, fonts::bold, ImVec2(tx, a.y + (hgt - fonts::bold->FontSize) * 0.5f), Col(theme::text, alpha), it.title.c_str());
        else
        {
            Text(dl, fonts::bold, ImVec2(tx, a.y + S(13)), Col(theme::text, alpha), it.title.c_str());
            dl->PushClipRect(ImVec2(tx, a.y), ImVec2(b.x - S(12), b.y), true);
            Text(dl, fonts::small, ImVec2(tx, a.y + S(34)), Col(theme::textDim, alpha), it.text.c_str());
            dl->PopClipRect();
        }
        float prog = 1.0f - ImSaturate(it.life / it.dur);
        dl->PushClipRect(a, b, true);
        Gradient(dl, ImVec2(a.x + S(14), b.y - S(3)), ImVec2(a.x + S(14) + (w - S(28)) * prog, b.y - S(1)), Col(c, 0.9f * alpha), Col(c, 0.2f * alpha), S(1));
        dl->PopClipRect();

        y += hgt + S(10);
        i++;
    }
}

} // namespace toast
