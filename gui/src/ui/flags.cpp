#include "flags.h"
#include "core.h"
#include "../lang/lang.h"
#include "../platform.h"
#include <vector>

namespace flags {

static ImTextureID g_tex[LANG_COUNT];

static ImVec4 RGB(int r, int g, int b) { return ImVec4(r / 255.0f, g / 255.0f, b / 255.0f, 1.0f); }
static float  Dist(float x0, float y0, float x1, float y1) { return sqrtf((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)); }

// Point-in-polygon (even-odd), used for stars.
static bool InPoly(const ImVec2* p, int n, float x, float y)
{
    bool in = false;
    for (int i = 0, j = n - 1; i < n; j = i++)
        if (((p[i].y > y) != (p[j].y > y)) && (x < (p[j].x - p[i].x) * (y - p[i].y) / (p[j].y - p[i].y) + p[i].x))
            in = !in;
    return in;
}

static bool InStar(float x, float y, float cx, float cy, float r, float rot)
{
    ImVec2 pts[10];
    for (int i = 0; i < 10; i++)
    {
        float rr = (i % 2) ? r * 0.382f : r;
        float a = rot + i * IM_PI / 5.0f;
        pts[i] = ImVec2(cx + cosf(a) * rr, cy + sinf(a) * rr);
    }
    return InPoly(pts, 10, x, y);
}

// All flags are sampled in "height units": X in [0, 1.5], Y in [0, 1].
static ImVec4 FlagTR(float X, float Y)
{
    const ImVec4 red = RGB(227, 10, 23), white = RGB(255, 255, 255);
    if (Dist(X, Y, 0.50f, 0.5f) <= 0.25f && Dist(X, Y, 0.5625f, 0.5f) > 0.20f) return white;
    if (InStar(X, Y, 0.866f, 0.5f, 0.125f, IM_PI)) return white;
    return red;
}

static ImVec4 FlagUK(float X, float Y)
{
    const ImVec4 blue = RGB(1, 33, 105), red = RGB(200, 16, 46), white = RGB(255, 255, 255);
    const float W = 1.5f, H = 1.0f, L = sqrtf(W * W + H * H);
    float d1 = fabsf(H * X - W * Y) / L;
    float d2 = fabsf(H * (W - X) - W * Y) / L;
    float dc = ImMin(fabsf(X - W * 0.5f), fabsf(Y - H * 0.5f));
    if (dc < 0.10f) return red;
    if (dc < 0.167f) return white;
    if (d1 < 0.035f || d2 < 0.035f) return red;
    if (d1 < 0.10f || d2 < 0.10f) return white;
    return blue;
}

static ImVec4 FlagDE(float, float Y)
{
    if (Y < 1.0f / 3.0f) return RGB(24, 24, 24);
    if (Y < 2.0f / 3.0f) return RGB(221, 0, 0);
    return RGB(255, 206, 0);
}

static ImVec4 FlagFR(float X, float)
{
    if (X < 0.5f) return RGB(0, 85, 164);
    if (X < 1.0f) return RGB(255, 255, 255);
    return RGB(239, 65, 53);
}

static ImVec4 FlagES(float, float Y)
{
    if (Y < 0.25f || Y >= 0.75f) return RGB(198, 11, 30);
    return RGB(255, 196, 0);
}

static ImVec4 FlagRU(float, float Y)
{
    if (Y < 1.0f / 3.0f) return RGB(255, 255, 255);
    if (Y < 2.0f / 3.0f) return RGB(0, 57, 166);
    return RGB(213, 43, 30);
}

static ImVec4 FlagBR(float X, float Y)
{
    const ImVec4 green = RGB(0, 156, 59), yellow = RGB(255, 223, 0), blue = RGB(0, 39, 118), white = RGB(255, 255, 255);
    if (Dist(X, Y, 0.75f, 0.5f) <= 0.25f)
    {
        float d = Dist(X, Y, 0.62f, 1.28f);
        if (d > 0.80f && d < 0.855f) return white;
        return blue;
    }
    if (fabsf(X - 0.75f) / 0.62f + fabsf(Y - 0.5f) / 0.41f <= 1.0f) return yellow;
    return green;
}

typedef ImVec4 (*FlagFn)(float, float);

void Build()
{
    static const FlagFn fns[LANG_COUNT] = { FlagTR, FlagUK, FlagDE, FlagFR, FlagES, FlagRU, FlagBR };
    // ~2x the on-screen size keeps bilinear minification clean (no mipmaps).
    const int H = (int)(48.0f * gfx::dpi), W = H * 3 / 2, SS = 4;
    const float R = W * 0.1f; // corner radius in texels
    std::vector<unsigned char> px(W * H * 4);

    for (int l = 0; l < LANG_COUNT; l++)
    {
        for (int y = 0; y < H; y++)
            for (int x = 0; x < W; x++)
            {
                ImVec4 acc(0, 0, 0, 0);
                float cover = 0.0f;
                for (int sy = 0; sy < SS; sy++)
                    for (int sx = 0; sx < SS; sx++)
                    {
                        float fx = x + (sx + 0.5f) / SS, fy = y + (sy + 0.5f) / SS;
                        // rounded-rect signed distance
                        float qx = fabsf(fx - W * 0.5f) - (W * 0.5f - R), qy = fabsf(fy - H * 0.5f) - (H * 0.5f - R);
                        float out = sqrtf(ImMax(qx, 0.0f) * ImMax(qx, 0.0f) + ImMax(qy, 0.0f) * ImMax(qy, 0.0f)) + ImMin(ImMax(qx, qy), 0.0f) - R;
                        if (out > 0.0f) continue;
                        ImVec4 c = fns[l](fx / H, fy / H);
                        // soft inner edge for depth
                        if (out > -2.0f) { c.x *= 0.82f; c.y *= 0.82f; c.z *= 0.82f; }
                        acc.x += c.x; acc.y += c.y; acc.z += c.z;
                        cover += 1.0f;
                    }
                unsigned char* p = &px[(y * W + x) * 4];
                if (cover <= 0.0f) { p[0] = p[1] = p[2] = p[3] = 0; continue; }
                float v = (float)y / H;
                float shade = 1.06f - 0.16f * v;          // subtle vertical lighting
                float gloss = v < 0.45f ? 0.06f * (1.0f - v / 0.45f) : 0.0f;
                p[0] = (unsigned char)(ImSaturate(acc.x / cover * shade + gloss) * 255.0f);
                p[1] = (unsigned char)(ImSaturate(acc.y / cover * shade + gloss) * 255.0f);
                p[2] = (unsigned char)(ImSaturate(acc.z / cover * shade + gloss) * 255.0f);
                p[3] = (unsigned char)(cover / (SS * SS) * 255.0f);
            }
        g_tex[l] = platform::CreateTexture(px.data(), W, H);
    }
}

void Draw(ImDrawList* dl, int l, ImVec2 a, ImVec2 b, float alpha)
{
    if (l < 0 || l >= LANG_COUNT || !g_tex[l]) return;
    dl->AddImage(g_tex[l], a, b, ImVec2(0, 0), ImVec2(1, 1), gfx::Col(gfx::White(), alpha));
}

} // namespace flags
