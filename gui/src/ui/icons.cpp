#include "icons.h"
#include "core.h"

void DrawIcon(ImDrawList* dl, Icon icon, ImVec2 c, float size, ImU32 col, float th)
{
    if (icon == Ico_None) return;
    const float h = size * 0.5f;
    if (th <= 0.0f) th = ImMax(S(1.4f), size * 0.085f);
    auto P = [&](float x, float y) { return ImVec2(c.x + x * h, c.y + y * h); };
    auto Line = [&](float x0, float y0, float x1, float y1) { dl->AddLine(P(x0, y0), P(x1, y1), col, th); };
    auto Stroke = [&](bool closed) { dl->PathStroke(col, closed ? ImDrawFlags_Closed : 0, th); };

    switch (icon)
    {
    case Ico_Home:
        dl->PathLineTo(P(-0.85f, -0.05f)); dl->PathLineTo(P(0.0f, -0.82f)); dl->PathLineTo(P(0.85f, -0.05f)); Stroke(false);
        dl->PathLineTo(P(-0.6f, -0.25f)); dl->PathLineTo(P(-0.6f, 0.82f)); dl->PathLineTo(P(0.6f, 0.82f)); dl->PathLineTo(P(0.6f, -0.25f)); Stroke(false);
        dl->PathLineTo(P(-0.18f, 0.82f)); dl->PathLineTo(P(-0.18f, 0.32f)); dl->PathLineTo(P(0.18f, 0.32f)); dl->PathLineTo(P(0.18f, 0.82f)); Stroke(false);
        break;
    case Ico_Grid:
        for (int i = 0; i < 4; i++)
        {
            float x = (i % 2) ? 0.45f : -0.45f, y = (i / 2) ? 0.45f : -0.45f;
            dl->AddRect(P(x - 0.32f, y - 0.32f), P(x + 0.32f, y + 0.32f), col, h * 0.14f, 0, th);
        }
        break;
    case Ico_Gear:
    {
        const int teeth = 8;
        for (int i = 0; i < teeth; i++)
        {
            float a = (float)i / teeth * IM_PI * 2.0f;
            float rin = 0.64f, rout = 0.92f;
            dl->PathLineTo(P(cosf(a - 0.34f) * rin, sinf(a - 0.34f) * rin));
            dl->PathLineTo(P(cosf(a - 0.17f) * rout, sinf(a - 0.17f) * rout));
            dl->PathLineTo(P(cosf(a + 0.17f) * rout, sinf(a + 0.17f) * rout));
            dl->PathLineTo(P(cosf(a + 0.34f) * rin, sinf(a + 0.34f) * rin));
        }
        Stroke(true);
        dl->AddCircle(c, h * 0.28f, col, 20, th);
        break;
    }
    case Ico_User:
        dl->AddCircle(P(0, -0.38f), h * 0.34f, col, 24, th);
        dl->PathArcTo(P(0, 0.92f), h * 0.74f, IM_PI * 1.08f, IM_PI * 1.92f, 20); Stroke(false);
        break;
    case Ico_Key:
        dl->AddCircle(P(-0.45f, 0.0f), h * 0.32f, col, 20, th);
        Line(-0.13f, 0.0f, 0.88f, 0.0f);
        Line(0.55f, 0.0f, 0.55f, 0.32f);
        Line(0.80f, 0.0f, 0.80f, 0.26f);
        break;
    case Ico_Lock:
        dl->AddRect(P(-0.66f, -0.12f), P(0.66f, 0.86f), col, h * 0.18f, 0, th);
        dl->PathLineTo(P(-0.40f, -0.12f)); dl->PathArcTo(P(0, -0.40f), h * 0.40f, IM_PI, IM_PI * 2.0f, 14); dl->PathLineTo(P(0.40f, -0.12f)); Stroke(false);
        dl->AddCircleFilled(P(0, 0.36f), h * 0.11f, col, 10);
        break;
    case Ico_Eye:
    case Ico_EyeOff:
        dl->PathLineTo(P(-0.92f, 0.0f));
        dl->PathBezierCubicCurveTo(P(-0.45f, -0.62f), P(0.45f, -0.62f), P(0.92f, 0.0f));
        dl->PathBezierCubicCurveTo(P(0.45f, 0.62f), P(-0.45f, 0.62f), P(-0.92f, 0.0f));
        Stroke(true);
        dl->AddCircle(c, h * 0.24f, col, 16, th);
        if (icon == Ico_EyeOff) Line(-0.78f, -0.78f, 0.78f, 0.78f);
        break;
    case Ico_Clipboard:
        dl->AddRect(P(-0.62f, -0.62f), P(0.62f, 0.90f), col, h * 0.16f, 0, th);
        dl->AddRectFilled(P(-0.30f, -0.88f), P(0.30f, -0.46f), col, h * 0.10f);
        Line(-0.32f, 0.05f, 0.32f, 0.05f);
        Line(-0.32f, 0.42f, 0.18f, 0.42f);
        break;
    case Ico_Copy:
        dl->AddRect(P(-0.30f, -0.30f), P(0.82f, 0.86f), col, h * 0.14f, 0, th);
        dl->PathLineTo(P(-0.62f, 0.50f)); dl->PathLineTo(P(-0.82f, 0.50f)); dl->PathLineTo(P(-0.82f, -0.86f));
        dl->PathLineTo(P(0.46f, -0.86f)); dl->PathLineTo(P(0.46f, -0.62f)); Stroke(false);
        break;
    case Ico_Check:
        dl->PathLineTo(P(-0.62f, 0.04f)); dl->PathLineTo(P(-0.16f, 0.50f)); dl->PathLineTo(P(0.66f, -0.46f)); Stroke(false);
        break;
    case Ico_Close:
        Line(-0.58f, -0.58f, 0.58f, 0.58f);
        Line(0.58f, -0.58f, -0.58f, 0.58f);
        break;
    case Ico_Minimize:
        Line(-0.62f, 0.0f, 0.62f, 0.0f);
        break;
    case Ico_Play:
        dl->AddTriangleFilled(P(-0.46f, -0.70f), P(0.74f, 0.0f), P(-0.46f, 0.70f), col);
        break;
    case Ico_Shield:
        dl->PathLineTo(P(0.0f, -0.92f));
        dl->PathLineTo(P(0.76f, -0.62f));
        dl->PathLineTo(P(0.76f, 0.02f));
        dl->PathBezierCubicCurveTo(P(0.76f, 0.50f), P(0.38f, 0.76f), P(0.0f, 0.94f));
        dl->PathBezierCubicCurveTo(P(-0.38f, 0.76f), P(-0.76f, 0.50f), P(-0.76f, 0.02f));
        dl->PathLineTo(P(-0.76f, -0.62f));
        Stroke(true);
        dl->PathLineTo(P(-0.30f, 0.02f)); dl->PathLineTo(P(-0.06f, 0.26f)); dl->PathLineTo(P(0.34f, -0.18f)); Stroke(false);
        break;
    case Ico_Clock:
        dl->AddCircle(c, h * 0.88f, col, 28, th);
        Line(0.0f, 0.0f, 0.0f, -0.52f);
        Line(0.0f, 0.0f, 0.36f, 0.22f);
        break;
    case Ico_Download:
        Line(0.0f, -0.88f, 0.0f, 0.34f);
        dl->PathLineTo(P(-0.40f, -0.06f)); dl->PathLineTo(P(0.0f, 0.34f)); dl->PathLineTo(P(0.40f, -0.06f)); Stroke(false);
        dl->PathLineTo(P(-0.82f, 0.44f)); dl->PathLineTo(P(-0.82f, 0.86f)); dl->PathLineTo(P(0.82f, 0.86f)); dl->PathLineTo(P(0.82f, 0.44f)); Stroke(false);
        break;
    case Ico_Logout:
        dl->PathLineTo(P(0.10f, -0.82f)); dl->PathLineTo(P(-0.78f, -0.82f)); dl->PathLineTo(P(-0.78f, 0.82f)); dl->PathLineTo(P(0.10f, 0.82f)); Stroke(false);
        Line(-0.22f, 0.0f, 0.86f, 0.0f);
        dl->PathLineTo(P(0.50f, -0.36f)); dl->PathLineTo(P(0.86f, 0.0f)); dl->PathLineTo(P(0.50f, 0.36f)); Stroke(false);
        break;
    case Ico_Globe:
        dl->AddCircle(c, h * 0.88f, col, 28, th);
        dl->AddEllipse(c, ImVec2(h * 0.38f, h * 0.88f), col, 0.0f, 24, th);
        Line(-0.88f, 0.0f, 0.88f, 0.0f);
        break;
    case Ico_Bolt:
    {
        ImVec2 pts[6] = { P(0.18f, -0.94f), P(-0.58f, 0.12f), P(-0.04f, 0.12f), P(-0.20f, 0.94f), P(0.58f, -0.16f), P(0.04f, -0.16f) };
        dl->AddConcavePolyFilled(pts, 6, col);
        break;
    }
    case Ico_Signal:
        for (int i = 0; i < 4; i++)
        {
            float x = -0.78f + i * 0.46f, top = 0.62f - i * 0.46f;
            dl->AddRectFilled(P(x, top), P(x + 0.28f, 0.84f), col, h * 0.08f);
        }
        break;
    case Ico_Info:
        dl->AddCircle(c, h * 0.88f, col, 28, th);
        dl->AddCircleFilled(P(0.0f, -0.40f), th * 0.75f, col, 10);
        Line(0.0f, -0.08f, 0.0f, 0.48f);
        break;
    case Ico_Sparkle:
    {
        ImVec2 v[4] = { P(0, -0.95f), P(0.24f, 0), P(0, 0.95f), P(-0.24f, 0) };
        ImVec2 hz[4] = { P(-0.95f, 0), P(0, -0.24f), P(0.95f, 0), P(0, 0.24f) };
        dl->AddConvexPolyFilled(v, 4, col);
        dl->AddConvexPolyFilled(hz, 4, col);
        break;
    }
    case Ico_Bell:
        dl->PathLineTo(P(-0.74f, 0.50f));
        dl->PathLineTo(P(-0.56f, 0.28f));
        dl->PathLineTo(P(-0.56f, -0.14f));
        dl->PathBezierCubicCurveTo(P(-0.56f, -0.60f), P(-0.30f, -0.82f), P(0.0f, -0.82f));
        dl->PathBezierCubicCurveTo(P(0.30f, -0.82f), P(0.56f, -0.60f), P(0.56f, -0.14f));
        dl->PathLineTo(P(0.56f, 0.28f));
        dl->PathLineTo(P(0.74f, 0.50f));
        Stroke(true);
        dl->AddCircleFilled(P(0, 0.76f), h * 0.15f, col, 12);
        break;
    case Ico_Chevron:
        dl->PathLineTo(P(-0.24f, -0.56f)); dl->PathLineTo(P(0.30f, 0.0f)); dl->PathLineTo(P(-0.24f, 0.56f)); Stroke(false);
        break;
    case Ico_Arrow:
        Line(-0.70f, 0.0f, 0.70f, 0.0f);
        dl->PathLineTo(P(0.24f, -0.46f)); dl->PathLineTo(P(0.70f, 0.0f)); dl->PathLineTo(P(0.24f, 0.46f)); Stroke(false);
        break;
    case Ico_Cpu:
        dl->AddRect(P(-0.56f, -0.56f), P(0.56f, 0.56f), col, h * 0.14f, 0, th);
        dl->AddRectFilled(P(-0.22f, -0.22f), P(0.22f, 0.22f), col, h * 0.06f);
        for (int i = -1; i <= 1; i++)
        {
            float o = i * 0.30f;
            Line(o, -0.56f, o, -0.86f); Line(o, 0.56f, o, 0.86f);
            Line(-0.56f, o, -0.86f, o); Line(0.56f, o, 0.86f, o);
        }
        break;
    case Ico_Crown:
        dl->PathLineTo(P(-0.82f, 0.62f)); dl->PathLineTo(P(-0.82f, -0.42f)); dl->PathLineTo(P(-0.40f, 0.04f));
        dl->PathLineTo(P(0.0f, -0.64f)); dl->PathLineTo(P(0.40f, 0.04f)); dl->PathLineTo(P(0.82f, -0.42f)); dl->PathLineTo(P(0.82f, 0.62f));
        Stroke(true);
        break;
    case Ico_Refresh:
        dl->PathArcTo(c, h * 0.78f, -IM_PI * 0.35f, IM_PI * 1.45f, 24); Stroke(false);
        {
            ImVec2 tip = P(cosf(-IM_PI * 0.35f) * 0.78f, sinf(-IM_PI * 0.35f) * 0.78f);
            dl->AddTriangleFilled(tip + ImVec2(-h * 0.36f, -h * 0.12f), tip + ImVec2(h * 0.18f, -h * 0.30f), tip + ImVec2(h * 0.10f, h * 0.28f), col);
        }
        break;
    case Ico_Layers:
    {
        ImVec2 top[4] = { P(0, -0.82f), P(0.86f, -0.36f), P(0, 0.10f), P(-0.86f, -0.36f) };
        dl->AddPolyline(top, 4, col, ImDrawFlags_Closed, th);
        dl->PathLineTo(P(-0.86f, 0.06f)); dl->PathLineTo(P(0, 0.52f)); dl->PathLineTo(P(0.86f, 0.06f)); Stroke(false);
        dl->PathLineTo(P(-0.86f, 0.44f)); dl->PathLineTo(P(0, 0.90f)); dl->PathLineTo(P(0.86f, 0.44f)); Stroke(false);
        break;
    }
    case Ico_ChevronLeft:
        dl->PathLineTo(P(0.24f, -0.56f)); dl->PathLineTo(P(-0.30f, 0.0f)); dl->PathLineTo(P(0.24f, 0.56f)); Stroke(false);
        break;
    case Ico_Target:
        dl->AddCircle(c, h * 0.62f, col, 24, th);
        Line(0.0f, -0.95f, 0.0f, -0.38f); Line(0.0f, 0.38f, 0.0f, 0.95f);
        Line(-0.95f, 0.0f, -0.38f, 0.0f); Line(0.38f, 0.0f, 0.95f, 0.0f);
        dl->AddCircleFilled(c, th * 0.9f, col, 10);
        break;
    case Ico_Radar:
        dl->AddCircle(c, h * 0.88f, col, 28, th);
        dl->AddCircle(c, h * 0.45f, col, 20, th);
        Line(0.0f, 0.0f, 0.62f, -0.62f);
        dl->AddCircleFilled(P(0.30f, 0.34f), th * 0.9f, col, 8);
        break;
    case Ico_Cloud:
        dl->PathArcTo(P(-0.42f, 0.18f), h * 0.36f, IM_PI * 0.5f, IM_PI * 1.5f, 10);
        dl->PathArcTo(P(0.0f, -0.12f), h * 0.46f, IM_PI * 1.1f, IM_PI * 1.95f, 12);
        dl->PathArcTo(P(0.46f, 0.20f), h * 0.34f, IM_PI * 1.5f, IM_PI * 2.5f, 10);
        Stroke(true);
        break;
    case Ico_Car:
        dl->PathLineTo(P(-0.88f, 0.40f)); dl->PathLineTo(P(-0.88f, 0.02f)); dl->PathLineTo(P(-0.56f, -0.50f));
        dl->PathLineTo(P(0.50f, -0.50f)); dl->PathLineTo(P(0.88f, 0.02f)); dl->PathLineTo(P(0.88f, 0.40f));
        Stroke(true);
        Line(-0.88f, 0.02f, 0.88f, 0.02f);
        dl->AddCircleFilled(P(-0.50f, 0.56f), h * 0.17f, col, 10);
        dl->AddCircleFilled(P(0.50f, 0.56f), h * 0.17f, col, 10);
        break;
    case Ico_Code:
        dl->PathLineTo(P(-0.40f, -0.55f)); dl->PathLineTo(P(-0.92f, 0.0f)); dl->PathLineTo(P(-0.40f, 0.55f)); Stroke(false);
        dl->PathLineTo(P(0.40f, -0.55f)); dl->PathLineTo(P(0.92f, 0.0f)); dl->PathLineTo(P(0.40f, 0.55f)); Stroke(false);
        Line(0.16f, -0.78f, -0.16f, 0.78f);
        break;
    default: break;
    }
}
