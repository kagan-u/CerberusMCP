// CerberusMCP GUI — splash, onboarding, home, MCP servers, logs, settings.
#include "app.h"
#include "ui/core.h"
#include "ui/widgets.h"
#include "ui/flags.h"
#include "lang/lang.h"
#include "cerberus_api.h"
#include "registry.h"
#include "rpc.h"
#include "config.h"
#include "platform.h"

#include "misc/cpp/imgui_stdlib.h"

#include <string>
#include <vector>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cctype>
#include <ctime>

using namespace gfx;
using json = core::json;

namespace app {
namespace {

const float kTitleH = 44.0f;
const float kSideW  = 214.0f;

enum Screen { SCR_SPLASH, SCR_LANG, SCR_NAME, SCR_MAIN };
enum Page   { PG_HOME, PG_MCP, PG_LOGS, PG_SETTINGS, PG_COUNT };

struct EnvRow { std::string key, val; int type; };   // 0 literal, 1 env, 2 secret_ref

struct LogLine { std::string text; float age; };

struct State
{
    Screen     screen = SCR_SPLASH;
    float      screenT = 0.0f;
    float      splashT = 0.0f;

    // onboarding (language, then name) - runs once, config::c.onboarded remembers it
    char       nameBuf[40] = {};

    // server dialog (create + edit)
    bool        dlgOpen = false;
    std::string dlgName, dlgTags, dlgDesc, dlgCmd, dlgArgs, dlgCwd, dlgUrl, dlgToken;
    int         dlgTransport = 0;      // 0 stdio, 1 http
    int         dlgTimeout   = 15;     // seconds
    bool        dlgEnabled   = true;
    bool        dlgShowToken = false;
    std::string dlgEditName;           // "" = creating; else the server being edited
    std::vector<EnvRow> dlgEnv;

    std::string confirmDelete;         // non-empty: delete-confirmation modal open
    bool        confirmWasOpen = false;

    bool       titleDrag = false;      // title-bar drag with a movement threshold
    bool       titleDragOk = false;

    float      carousel = 1.0f;
    float      wheelCd = 0.0f;

    bool       trans = false;
    float      transT = 0.0f;
    ImVec2     transOrigin;
    bool       transSwitched = false;
    Screen     transTarget = SCR_MAIN;

    int        page = PG_HOME;
    float      pageT = 1.0f;
    int        selected = 0;
    float      detailT = 1.0f;

    // core bridge state
    std::string version = "v?";
    json        gwStatus;               // last core::GatewayStatus()
    float       gwTimer = 0.0f;
    uint64_t    gwStartJob = 0, gwStopJob = 0, orphanStopJob = 0;
    uint64_t    doctorJob = 0;
    json        doctorReport;
    float       doctorTimer = 0.0f;

    // log stream
    std::vector<LogLine> logLines;
    uint64_t    logCursor = 0;
    float       logTimer  = 0.0f;
    bool        logsPaused = false;
    bool        logsAuto   = true;
    int         newLines   = 0;         // lines since the last spark bucket
    float       spark[48] = {};
    float       sparkTimer = 0.0f;

    std::string user, hwid;

    ImTextureID logo = 0;
} g;

// The brand logo (assets/logo.png, white background keyed out) drawn centred on c.
void Logo(ImDrawList* dl, ImVec2 c, float size, float alpha = 1.0f)
{
    if (!g.logo) { BrandMark(dl, c, size * 0.45f, true); return; }
    ImVec2 a(c.x - size * 0.5f, c.y - size * 0.5f);
    dl->AddImageRounded(g.logo, a, a + ImVec2(size, size), ImVec2(0, 0), ImVec2(1, 1),
                        Col(White(alpha)), S(6));
}

struct Confetti { ImVec2 p, v; float rot, vr, life, size; ImVec4 c; };

struct Loader
{
    bool  open = false, closing = false, done = false, justOpened = false, spawnConfetti = false;
    bool  failed = false, opDone = false;
    float openT = 0.0f, stageTime = 0.0f, doneT = 0.0f, shown = 0.0f, closeTimer = 0.0f;
    int   stage = 0;
    int   jobKind = 0;                  // 0 idle, 1 doctor, 2 gateway start
    uint64_t job = 0;
    std::string serverName, error;
    json  opInfo;                       // doctor report / gateway status of the last stage
    char  session[16] = {};
    std::vector<Confetti> confetti;
} ld;

const float kStageWeight[6] = { 0.10f, 0.12f, 0.16f, 0.36f, 0.14f, 0.12f };
const float kStageDwell      = 0.45f;   // minimum seconds a stage stays visible
const Str   kStageStr[6]     = { S_STAGE_REG, S_STAGE_VALIDATE, S_STAGE_DOCTOR, S_STAGE_GATEWAY, S_STAGE_STATUS, S_STAGE_READY };

void OpenServerDialog(int editIndex);   // -1 = create
void SaveDialogServer();
bool StylePill(const char* label, bool sel, ImVec2 size);

float Rand01()
{
    static unsigned int s = (unsigned int)time(nullptr) | 1u;
    s ^= s << 13; s ^= s >> 17; s ^= s << 5;
    return (s & 0xFFFFFF) / (float)0xFFFFFF;
}

float RealDt() { return ImMin(ImGui::GetIO().DeltaTime, 0.1f); }

std::string Initial(const std::string& s)
{
    if (s.empty()) return "?";
    unsigned int c = 0;
    int n = ImTextCharFromUtf8(&c, s.c_str(), nullptr);
    std::string r = s.substr(0, n > 0 ? n : 1);
    if (r.size() == 1) r[0] = (char)toupper((unsigned char)r[0]);
    return r;
}

void IconChip(ImDrawList* dl, ImVec2 p, Icon ic, ImVec4 c, float size)
{
    Gradient(dl, p, p + ImVec2(size, size), Col(c, 0.24f), Col(c, 0.08f), S(9));
    dl->AddRect(p, p + ImVec2(size, size), Col(c, 0.25f), S(9));
    DrawIcon(dl, ic, p + ImVec2(size, size) * 0.5f, size * 0.48f, Col(c));
}

void Label(const char* s)
{
    ImVec2 p = ImGui::GetCursorScreenPos();
    Text(ImGui::GetWindowDrawList(), fonts::small, p, Col(theme::textMute), s);
    ImGui::Dummy(ImVec2(Measure(fonts::small, s).x, fonts::small->FontSize));
}

// ------------------------------------------------------------------ state changes
void SetPage(int p)
{
    if (p == g.page) return;
    g.page = p;
    g.pageT = 0.0f;
    if (p == PG_MCP) g.detailT = 0.0f;
}

void SetLanguage(int l)
{
    if (l == lang::current) return;
    lang::current = l;
    config::c.language = l;
    config::MarkDirty();
    toast::Push(toast::Info, T(S_LANG_CHANGED), lang::NativeName(l), 2.4f);
}

void StartTransition(ImVec2 origin, Screen target)
{
    if (g.trans) return;
    g.trans = true;
    g.transT = 0.0f;
    g.transOrigin = origin;
    g.transSwitched = false;
    g.transTarget = target;
}

void SwitchScreen(Screen s)
{
    g.screen = s;
    g.screenT = 0.0f;
    if (s == SCR_MAIN)
    {
        g.page = PG_HOME;
        g.pageT = 0.0f;
    }
    else if (s == SCR_SPLASH)
    {
        g.splashT = 0.0f;
        ld = Loader();
    }
}

void FinishOnboarding()
{
    std::string n = g.nameBuf;
    while (!n.empty() && n.front() == ' ') n.erase(n.begin());
    while (!n.empty() && n.back() == ' ') n.pop_back();
    config::c.userName = n.empty() ? platform::UserName() : n;
    config::c.onboarded = true;
    config::Save();
    g.user = config::c.userName;
    StartTransition(ImVec2(0, 0), SCR_MAIN);
}

void RestartWizard()
{
    config::c.onboarded = false;
    config::Save();
    g.splashT = 0.0f;
    StartTransition(ImVec2(0, 0), SCR_LANG);
    toast::Push(toast::Info, T(S_RESTART_WIZARD));
}

json GatewayOpts()
{
    json opts;
    opts["bind"] = config::c.gwBind.empty() ? "127.0.0.1:8713" : config::c.gwBind;
    if (!config::c.gwToken.empty()) opts["token"] = config::c.gwToken;
    opts["watch"] = config::c.gwWatch;
    opts["watchInterval"] = config::c.gwWatchMs;
    if (!config::c.gwProfile.empty()) opts["profile"] = config::c.gwProfile;
    return opts;
}

void StartLoader(const std::string& name)
{
    if (reg::IndexOf(name) < 0) return;
    ld = Loader();
    ld.open = true;
    ld.justOpened = true;
    ld.serverName = name;
    snprintf(ld.session, sizeof(ld.session), "%04X%04X", (unsigned)(Rand01() * 0xFFFF), (unsigned)(Rand01() * 0xFFFF));
}

void PollLogs(float dt)
{
    for (LogLine& l : g.logLines) l.age += dt;
    g.sparkTimer += dt;
    while (g.sparkTimer > 0.4f)
    {
        g.sparkTimer -= 0.4f;
        memmove(g.spark, g.spark + 1, sizeof(g.spark) - sizeof(float));
        g.spark[47] = (float)g.newLines;
        g.newLines = 0;
    }
    g.logTimer += dt;
    if (g.logTimer < 0.25f || g.logsPaused) return;
    g.logTimer = 0.0f;
    json r = core::LogsTake(g.logCursor);
    if (!r.is_object()) return;
    if (r.contains("lines") && r["lines"].is_array())
    {
        for (const auto& line : r["lines"])
        {
            if (!line.is_string()) continue;
            g.logLines.push_back({ line.get<std::string>(), 0.0f });
            g.newLines++;
        }
        if (g.logLines.size() > 1000)
            g.logLines.erase(g.logLines.begin(), g.logLines.begin() + (long)(g.logLines.size() - 1000));
    }
    if (r.contains("next") && r["next"].is_number_unsigned()) g.logCursor = r["next"].get<uint64_t>();
}

void PollGateway(float dt)
{
    if (g.gwStartJob)
    {
        json p = core::JobPoll(g.gwStartJob);
        std::string st = p.value("state", "pending");
        if (st == "done")
        {
            core::JobFree(g.gwStartJob);
            g.gwStartJob = 0;
            g.gwStatus = core::GatewayStatus();
            toast::Push(toast::Success, T(S_GATEWAY), T(S_RUNNING));
        }
        else if (st == "error")
        {
            std::string e = p.value("error", "gateway start failed");
            core::JobFree(g.gwStartJob);
            g.gwStartJob = 0;
            g.gwStatus = core::GatewayStatus();
            toast::Push(toast::Error, T(S_GATEWAY), e.c_str(), 6.0f);
        }
    }
    if (g.gwStopJob)
    {
        json p = core::JobPoll(g.gwStopJob);
        std::string st = p.value("state", "pending");
        if (st == "done" || st == "error")
        {
            if (st == "error")
                toast::Push(toast::Error, T(S_GATEWAY), p.value("error", "gateway stop failed").c_str(), 6.0f);
            else
                toast::Push(toast::Info, T(S_GATEWAY), T(S_STOPPED));
            core::JobFree(g.gwStopJob);
            g.gwStopJob = 0;
            g.gwStatus = core::GatewayStatus();
        }
    }
    if (g.orphanStopJob)
    {
        json p = core::JobPoll(g.orphanStopJob);
        std::string st = p.value("state", "pending");
        if (st == "done" || st == "error")
        {
            core::JobFree(g.orphanStopJob);
            g.orphanStopJob = 0;
        }
    }
    g.gwTimer += dt;
    if (g.gwTimer >= 0.4f && !g.gwStartJob && !g.gwStopJob)
    {
        g.gwTimer = 0.0f;
        json s = core::GatewayStatus();
        if (s.is_object()) g.gwStatus = std::move(s);
    }
}

void PollDoctor(float dt)
{
    (void)dt;
    if (!g.doctorJob) return;
    json p = core::JobPoll(g.doctorJob);
    std::string st = p.value("state", "pending");
    if (st == "done")
    {
        if (p.contains("value") && p["value"].is_object()) g.doctorReport = p["value"];
        core::JobFree(g.doctorJob);
        g.doctorJob = 0;
    }
    else if (st == "error")
    {
        toast::Push(toast::Error, T(S_DIAGNOSTICS), p.value("error", "doctor failed").c_str(), 6.0f);
        core::JobFree(g.doctorJob);
        g.doctorJob = 0;
    }
}

void StartDoctor()
{
    if (g.doctorJob) return;
    g.doctorJob = core::DoctorStart(false);
    if (!g.doctorJob)
        toast::Push(toast::Error, T(S_DIAGNOSTICS), core::LastError().c_str(), 6.0f);
}

void StartGateway()
{
    if (g.gwStartJob || (g.gwStatus.is_object() && g.gwStatus.value("running", false))) return;
    g.gwStartJob = core::GatewayStart(GatewayOpts());
    if (!g.gwStartJob)
        toast::Push(toast::Error, T(S_GATEWAY), core::LastError().c_str(), 6.0f);
}

void StopGateway()
{
    if (g.gwStopJob) return;
    g.gwStopJob = core::GatewayStop();
    if (!g.gwStopJob)
        toast::Push(toast::Error, T(S_GATEWAY), core::LastError().c_str(), 6.0f);
}

// ================================================================== title bar
void TitleBar(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float th = S(kTitleH);
    if (g.screen == SCR_MAIN)
    {
        dl->AddRectFilled(ImVec2(0, 0), ImVec2(ds.x, th), Col(theme::bg0, 0.55f));
        Gradient(dl, ImVec2(0, th - S(1)), ImVec2(ds.x, th), Col(theme::Ink(0.02f)), Col(theme::Ink(0.08f)));
    }

    Logo(dl, ImVec2(S(26), th * 0.5f), S(17));
    float x = S(44);
    float w = Spaced(dl, fonts::smallBold, ImVec2(x, th * 0.5f - fonts::smallBold->FontSize * 0.5f), Col(theme::text), "CERBERUSMCP", S(3));
    x += w + S(10);
    ImVec2 vs = Measure(fonts::small, g.version.c_str());
    ImVec2 pa(x, th * 0.5f - S(10)), pb(x + vs.x + S(14), th * 0.5f + S(10));
    dl->AddRectFilled(pa, pb, Col(theme::accent, 0.14f), S(10));
    Text(dl, fonts::small, ImVec2(pa.x + S(7), pa.y + (S(20) - vs.y) * 0.5f), Col(theme::accent2), g.version.c_str());

    float bs = S(32), y = (th - bs) * 0.5f;
    ImGui::SetCursorScreenPos(ImVec2(ds.x - S(10) - bs * 2 - S(4), y));
    if (ui::IconButton("##min", Ico_Minimize, bs, T(S_MINIMIZE))) platform::Minimize();
    ImGui::SetCursorScreenPos(ImVec2(ds.x - S(10) - bs, y));
    if (ui::IconButton("##close", Ico_Close, bs, T(S_CLOSE_APP), Col(theme::danger, 0.9f))) platform::Close();

    bool inBar = ImGui::IsWindowHovered() && !ImGui::IsAnyItemHovered() &&
                 ImGui::IsMouseHoveringRect(ImVec2(0, 0), ImVec2(ds.x, th), false);
    if (ImGui::IsMouseClicked(0) && inBar) { g.titleDrag = true; g.titleDragOk = false; }
    if (g.titleDrag)
    {
        if (!ImGui::IsMouseDown(0)) { g.titleDrag = false; g.titleDragOk = false; }
        else if (!g.titleDragOk && ImGui::IsMouseDragging(0, 5.0f))
        {
            g.titleDragOk = true;
            platform::BeginWindowDrag();
        }
    }
}

// ================================================================== splash
// Opening animation: the wordmark slides in from both sides, locks together in
// the middle, holds, then retracts. Runs on every launch.
void Splash(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    g.splashT += RealDt();
    const float tIn = 0.60f, tHold = 1.05f, tOut = 1.55f, tEnd = 1.95f;
    float t = g.splashT;

    float fade = 1.0f - anim::OutCubic((t - (tOut - 0.30f)) / 0.32f);
    fade = ImClamp(fade, 0.0f, 1.0f);
    ImGui::PushStyleVar(ImGuiStyleVar_Alpha, ImGui::GetStyle().Alpha * fade);

    dl->AddRectFilled(ImVec2(0, 0), ds, Col(theme::bg0));
    Glow(dl, ds * 0.5f, S(340), Col(theme::accent, 0.13f));
    Glow(dl, ImVec2(ds.x * 0.5f, ds.y * 0.44f), S(220), Col(theme::accent2, 0.10f));

    // 0 = pulled apart, 1 = joined
    float p;
    if (t < tIn)        p = anim::OutCubic(t / tIn);
    else if (t < tHold) p = 1.0f;
    else if (t < tOut)  p = 1.0f - anim::InOutCubic((t - tHold) / (tOut - tHold));
    else                p = 0.0f;

    const char* wm = "CERBERUSMCP";
    float sp = S(12);
    float ww = SpacedWidth(fonts::display, wm, sp);
    float fs = fonts::display->FontSize;
    ImVec2 c(ds.x * 0.5f, ds.y * 0.5f - fs * 0.25f);
    float travel = ds.x * 0.55f + ww * 0.5f;
    float off = (1.0f - p) * travel;
    ImVec2 base(c.x - ww * 0.5f, c.y - fs * 0.5f);

    dl->PushClipRect(ImVec2(0, c.y - fs), ImVec2(c.x, c.y + fs), true);
    Spaced(dl, fonts::display, ImVec2(base.x - off, base.y), Col(theme::text), wm, sp);
    dl->PopClipRect();
    dl->PushClipRect(ImVec2(c.x, c.y - fs), ImVec2(ds.x, c.y + fs), true);
    Spaced(dl, fonts::display, ImVec2(base.x + off, base.y), Col(theme::text), wm, sp);
    dl->PopClipRect();

    float markP = anim::OutCubic((t - 0.12f) / 0.5f);
    if (markP > 0.01f) Logo(dl, ImVec2(c.x, c.y - fs - S(64)), S(36) * markP, markP);

    float lineW = ww * p;
    if (lineW > S(6))
    {
        float ly = c.y + fs * 0.5f + S(24);
        Gradient(dl, ImVec2(c.x - lineW * 0.5f, ly), ImVec2(c.x + lineW * 0.5f, ly + S(1.5f)),
                 Col(theme::accent, 0.0f), Col(theme::accent2, 0.95f));
        Gradient(dl, ImVec2(c.x - lineW * 0.5f, ly), ImVec2(c.x + lineW * 0.5f, ly + S(1.5f)),
                 Col(theme::accent2, 0.95f), Col(theme::accent, 0.0f));
    }
    float tagP = anim::OutCubic((t - 0.55f) / 0.5f);
    if (tagP > 0.01f)
    {
        ImGui::PushStyleVar(ImGuiStyleVar_Alpha, ImGui::GetStyle().Alpha * tagP);
        TextC(dl, fonts::small, ImVec2(c.x, c.y + fs * 0.5f + S(44)), Col(theme::textDim), T(S_TAGLINE));
        ImGui::PopStyleVar();
    }

    ImGui::PopStyleVar();

    if (t >= tEnd) SwitchScreen(config::c.onboarded ? SCR_MAIN : SCR_LANG);
}

// ================================================================== onboarding (once)
static void WizardRow(ImDrawList* dl, ImVec2 p, float w, float h, int lang, bool selected)
{
    ImGui::SetCursorScreenPos(p);
    bool pressed = ImGui::InvisibleButton("##lang", ImVec2(w, h));
    ImGuiID id = ImGui::GetItemID();
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float hv = anim::To(anim::Key(id, 0), hov ? 1.0f : 0.0f, 14.0f);
    float sk = anim::To(anim::Key(id, 1), selected ? 1.0f : 0.0f, 12.0f);
    ImVec2 b = p + ImVec2(w, h);
    float r = S(12);

    dl->AddRectFilled(p, b, Col(theme::Ink(0.035f + 0.05f * hv)), r);
    if (sk > 0.01f || hv > 0.01f)
        GradientBorder(dl, p, b, r, Col(Mix(theme::Ink(0.10f + 0.10f * hv), theme::accent2, sk * 0.85f)),
                       Col(theme::Ink(0.05f)), S(1.2f));
    else
        dl->AddRect(p, b, Col(theme::border), r);

    ImVec2 fa(p.x + S(16), p.y + (h - S(20)) * 0.5f);
    flags::Draw(dl, lang, fa, fa + ImVec2(S(28), S(20)), 1.0f);
    Text(dl, fonts::bold, ImVec2(p.x + S(56), p.y + (h - fonts::bold->FontSize) * 0.5f),
         Col(Mix(theme::textDim, theme::text, ImMax(hv, sk))), lang::NativeName(lang));
    const char* code = lang::Code(lang);
    ImVec2 cs = Measure(fonts::smallBold, code);
    Text(dl, fonts::smallBold, ImVec2(b.x - S(16) - cs.x, p.y + (h - cs.y) * 0.5f),
         Col(sk > 0.5f ? theme::accent2 : theme::textMute), code);

    if (pressed) StartTransition(p + ImVec2(w, h) * 0.5f, SCR_NAME);
}

void OnboardLang(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float cx = ds.x * 0.5f;
    float y = ds.y * 0.5f - S(214);

    Logo(dl, ImVec2(cx, y), S(40));
    y += S(58);
    TextC(dl, fonts::hero, ImVec2(cx, y), Col(theme::text), T(S_ONBOARD_LANG));
    y += S(42);
    TextC(dl, fonts::body, ImVec2(cx, y), Col(theme::textDim), T(S_ONBOARD_LANG_SUB));
    y += S(40);

    float colW = S(300), gap = S(14), hgt = S(52), rowGap = S(12);
    float x0 = cx - colW - gap * 0.5f;
    for (int l = 0; l < LANG_COUNT; l++)
    {
        int col = l % 2, row = l / 2;
        ImVec2 p(x0 + col * (colW + gap), y + row * (hgt + rowGap));
        WizardRow(dl, p, colW, hgt, l, lang::current == l);
    }
}

void OnboardName(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float cx = ds.x * 0.5f;
    float y = ds.y * 0.5f - S(160);

    Logo(dl, ImVec2(cx, y), S(40));
    y += S(58);
    TextC(dl, fonts::hero, ImVec2(cx, y), Col(theme::text), T(S_ONBOARD_NAME));
    y += S(42);
    TextC(dl, fonts::body, ImVec2(cx, y), Col(theme::textDim), T(S_ONBOARD_NAME_SUB));
    y += S(44);

    float w = S(380);
    ImVec2 ia(cx - w * 0.5f, y);
    ImGui::SetCursorScreenPos(ia);
    ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(14), S(13)));
    ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, S(12));
    ImGui::SetNextItemWidth(w);
    std::string hint = platform::UserName();
    bool enter = ImGui::InputTextWithHint("##wizname", hint.c_str(), g.nameBuf, sizeof(g.nameBuf),
                                          ImGuiInputTextFlags_EnterReturnsTrue);
    ImGui::PopStyleVar(2);
    y += S(52) + S(26);

    ImVec2 bs(S(180), S(48));
    ImGui::SetCursorScreenPos(ImVec2(cx - bs.x - S(7), y));
    char back[64];
    snprintf(back, sizeof(back), "%s##back", T(S_ONBOARD_LANG));
    if (ui::Button(back, bs, ui::Btn_Ghost, Ico_ChevronLeft))
        StartTransition(ImVec2(cx, y), SCR_LANG);
    ImGui::SetCursorScreenPos(ImVec2(cx + S(7), y));
    if (ui::Button(T(S_CONTINUE), bs, ui::Btn_Primary, Ico_Arrow) || enter) FinishOnboarding();

    y += bs.y + S(30);
    char langLine[96];
    snprintf(langLine, sizeof(langLine), "%s  ·  %s", T(S_LANGUAGE), lang::NativeName(lang::current));
    TextC(dl, fonts::small, ImVec2(cx, y), Col(theme::textMute), langLine);
}

// ================================================================== main shell
void Sidebar(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 a(0, S(kTitleH)), b(S(kSideW), ds.y);
    dl->AddRectFilled(a, b, Col(theme::bg0, 0.45f));
    Gradient(dl, ImVec2(b.x - S(1), a.y), b, Col(theme::Ink(0.07f)), Col(theme::Ink(0.01f)), 0.0f, true);
    Spaced(dl, fonts::smallBold, ImVec2(S(28), a.y + S(22)), Col(theme::textMute), T(S_MENU), S(1.6f));

    static const Str  names[PG_COUNT] = { S_NAV_HOME, S_NAV_GAMES, S_NAV_LOGS, S_NAV_SETTINGS };
    static const Icon icons[PG_COUNT] = { Ico_Home, Ico_Grid, Ico_Code, Ico_Gear };
    float y0 = a.y + S(48), ih = S(44), gap = S(6), iw = b.x - S(28);
    float indY = anim::To(ImGui::GetID("##navind"), y0 + g.page * (ih + gap), 15.0f);
    ImVec2 ia(S(14), indY), ib(S(14) + iw, indY + ih);
    Gradient(dl, ia, ib, Col(theme::accent, 0.26f), Col(theme::accent, 0.02f), S(11));
    dl->AddRect(ia, ib, Col(theme::accent2, 0.12f), S(11));
    Glow(dl, ImVec2(ia.x, indY + ih * 0.5f), S(34), Col(theme::accent2, 0.35f));
    dl->AddRectFilled(ImVec2(ia.x, indY + S(12)), ImVec2(ia.x + S(3), ib.y - S(12)), Col(theme::accent2), S(2));

    for (int i = 0; i < PG_COUNT; i++)
    {
        ImVec2 p(S(14), y0 + i * (ih + gap));
        ImGui::SetCursorScreenPos(p);
        char id[16];
        snprintf(id, sizeof(id), "##nav%d", i);
        bool pressed = ImGui::InvisibleButton(id, ImVec2(iw, ih));
        ImGuiID gid = ImGui::GetItemID();
        bool hov = ImGui::IsItemHovered();
        if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
        float h = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 14.0f);
        float act = anim::To(anim::Key(gid, 1), g.page == i ? 1.0f : 0.0f, 12.0f);
        if (g.page != i) dl->AddRectFilled(p, p + ImVec2(iw, ih), Col(theme::Ink(0.035f * h)), S(11));
        ImVec4 col = Mix(Mix(theme::textMute, theme::text, h), theme::text, act);
        DrawIcon(dl, icons[i], ImVec2(p.x + S(26) + S(2) * h, p.y + ih * 0.5f), S(17), Col(Mix(col, theme::accent2, act)));
        Text(dl, act > 0.5f ? fonts::bold : fonts::body, ImVec2(p.x + S(48) + S(2) * h, p.y + (ih - fonts::body->FontSize) * 0.5f), Col(col), T(names[i]));
        if (pressed) SetPage(i);
    }

    // gateway status line
    {
        float sy = ds.y - S(118);
        bool running = g.gwStatus.is_object() && g.gwStatus.value("running", false);
        bool managed = g.gwStatus.is_object() && g.gwStatus.value("managed", false);
        ImVec4 dc = running ? (managed ? theme::success : theme::warning) : theme::textMute;
        ImVec2 dot(S(30), sy + S(8));
        if (running)
        {
            float k = fmodf(anim::Time(), 1.6f) / 1.6f;
            dl->AddCircleFilled(dot, S(3) + S(5) * k, Col(dc, 0.35f * (1.0f - k)), 16);
        }
        dl->AddCircleFilled(dot, S(3.2f), Col(dc), 12);
        std::string text;
        if (running)
        {
            text = T(S_GATEWAY);
            text += " · ";
            std::string bind = g.gwStatus.value("bind", std::string());
            text += bind.empty() ? T(S_RUNNING) : bind;
            if (!managed) { text += " · "; text += T(S_GATEWAY_EXTERNAL); }
        }
        else text = std::string(T(S_GATEWAY)) + " · " + T(S_STOPPED);
        Text(dl, fonts::small, ImVec2(S(42), sy), Col(theme::textDim), text.c_str());
    }

    // user card
    ImVec2 ua(S(14), ds.y - S(88)), ub(b.x - S(14), ds.y - S(16));
    dl->AddRectFilled(ua, ub, Col(theme::Ink(0.04f)), S(14));
    dl->AddRect(ua, ub, Col(theme::Ink(0.07f)), S(14));
    ImVec2 av(ua.x + S(30), (ua.y + ub.y) * 0.5f);
    Glow(dl, av, S(36), Col(theme::accent, 0.35f));
    int v0 = dl->VtxBuffer.Size;
    dl->AddCircleFilled(av, S(17), IM_COL32_WHITE, 32);
    Shade(dl, v0, av - S2(17, 17), av + S2(17, 17), Col(theme::accent), Col(theme::accent2));
    TextC(dl, fonts::bold, av, Col(theme::OnAccent()), Initial(g.user).c_str());
    dl->AddCircleFilled(av + S2(12, 12), S(5.5f), Col(theme::bg0), 12);
    dl->AddCircleFilled(av + S2(12, 12), S(3.5f),
                        Col(g.gwStatus.is_object() && g.gwStatus.value("running", false) ? theme::success : theme::textMute), 12);

    dl->PushClipRect(ua, ImVec2(ub.x - S(44), ub.y), true);
    Text(dl, fonts::bold, ImVec2(av.x + S(28), av.y - S(19)), Col(theme::text), g.user.c_str());
    IconChip(dl, ImVec2(av.x + S(30), av.y + S(1)), Ico_Layers, theme::accent2, S(24));
    Text(dl, fonts::smallBold, ImVec2(av.x + S(62), av.y + S(6)), Col(theme::accent2), "MCP");
    dl->PopClipRect();

    ImGui::SetCursorScreenPos(ImVec2(ub.x - S(40), av.y - S(15)));
    if (ui::IconButton("##wizard", Ico_Refresh, S(30), T(S_RESTART_WIZARD))) RestartWizard();
}

void PageHeader(ImVec2 p, float w, const char* title, const char* sub)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    Text(dl, fonts::hero, p, Col(theme::text), title);
    Text(dl, fonts::body, ImVec2(p.x, p.y + S(42)), Col(theme::textDim), sub);
    (void)w;
}

// ================================================================== home
void StatServers(ImVec2 a, ImVec2 sz)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 b = a + sz;
    Panel(dl, a, b, S(16));
    dl->PushClipRect(a, b, true);
    Glow(dl, ImVec2(b.x - S(40), a.y + S(20)), S(130), Col(theme::accent, 0.12f));
    dl->PopClipRect();
    IconChip(dl, a + S2(18, 18), Ico_Layers, theme::accent2, S(30));
    Text(dl, fonts::body, ImVec2(a.x + S(58), a.y + S(24)), Col(theme::textDim), T(S_NAV_GAMES));

    int total = (int)reg::servers.size(), enabled = 0;
    for (const reg::ServerView& sv : reg::servers) enabled += sv.enabled ? 1 : 0;
    float target = (float)ImMax(total, 1);
    float n = anim::ToFrom(ImGui::GetID("##servers"), target, 3.0f, 0.0f);
    char num[8];
    snprintf(num, sizeof(num), "%d", (int)(n + 0.5f));
    ImVec2 ns = Measure(fonts::display, num, S(36));
    Text(dl, fonts::display, ImVec2(a.x + S(18), a.y + S(58)), Col(theme::text), num, S(36));
    char en[32];
    snprintf(en, sizeof(en), "+%d %s", enabled, T(S_ENABLED));
    Text(dl, fonts::small, ImVec2(a.x + S(24) + ns.x, a.y + S(58) + ns.y - S(20)), Col(theme::textMute), en);

    ImVec2 rc(b.x - S(44), a.y + sz.y * 0.5f + S(10));
    float rr = S(22), frac = total > 0 ? ImSaturate((float)enabled / (float)total) : 0.0f;
    dl->AddCircle(rc, rr, Col(theme::Ink(0.07f)), 48, S(5));
    Arc(dl, rc, rr, -IM_PI * 0.5f, -IM_PI * 0.5f + IM_PI * 2.0f * frac, S(5), Col(theme::accent), Col(theme::accent2));
    char pct[8];
    snprintf(pct, sizeof(pct), "%d%%", (int)(frac * 100.0f + 0.5f));
    TextC(dl, fonts::smallBold, rc, Col(theme::text), pct);
}

void StatGateway(ImVec2 a, ImVec2 sz)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 b = a + sz;
    Panel(dl, a, b, S(16));
    bool running = g.gwStatus.is_object() && g.gwStatus.value("running", false);
    bool managed = g.gwStatus.is_object() && g.gwStatus.value("managed", false);
    ImVec4 c = running ? (managed ? theme::success : theme::warning) : theme::textMute;
    IconChip(dl, a + S2(18, 18), Ico_Signal, c, S(30));
    Text(dl, fonts::body, ImVec2(a.x + S(58), a.y + S(24)), Col(theme::textDim), T(S_GATEWAY));

    std::string label = running ? (managed ? T(S_RUNNING) : T(S_GATEWAY_EXTERNAL)) : T(S_STOPPED);
    ImVec2 mss = Measure(fonts::mono, label.c_str(), S(12));
    Text(dl, fonts::mono, ImVec2(b.x - S(18) - mss.x, a.y + S(26)), Col(c), label.c_str(), S(12));

    // log-activity sparkline
    float x0 = a.x + S(18), x1 = b.x - S(18), top = a.y + S(62), bot = b.y - S(14);
    float step = (x1 - x0) / 46.0f, frac = g.sparkTimer / 0.4f;
    ImVec2 pts[48];
    for (int i = 0; i < 48; i++)
    {
        float v = ImSaturate(g.spark[i] / 4.0f);
        pts[i] = ImVec2(x0 + (i - frac) * step, bot - v * (bot - top));
    }
    dl->PushClipRect(ImVec2(x0, top - S(6)), ImVec2(x1, bot + S(1)), true);
    ImDrawListFlags old = dl->Flags;
    dl->Flags &= ~ImDrawListFlags_AntiAliasedFill;
    int v0 = dl->VtxBuffer.Size;
    for (int i = 0; i < 47; i++)
        dl->AddQuadFilled(pts[i], pts[i + 1], ImVec2(pts[i + 1].x, bot), ImVec2(pts[i].x, bot), IM_COL32_WHITE);
    Shade(dl, v0, ImVec2(0, top), ImVec2(0, bot), Col(c, 0.30f), Col(c, 0.0f));
    dl->Flags = old;
    v0 = dl->VtxBuffer.Size;
    dl->AddPolyline(pts, 48, IM_COL32_WHITE, 0, S(1.8f));
    Shade(dl, v0, ImVec2(x0, 0), ImVec2(x1, 0), Col(c, 0.2f), Col(c));
    dl->PopClipRect();
    ImVec2 last = pts[46] + (pts[47] - pts[46]) * (1.0f - frac);
    last.x = ImMin(last.x, x1);
    Glow(dl, last, S(18), Col(c, 0.7f));
    dl->AddCircleFilled(last, S(3), Col(Mix(c, theme::text, 0.35f)), 12);
}

void StatDiagnostics(ImVec2 a, ImVec2 sz, float grow)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 b = a + sz;
    Panel(dl, a, b, S(16));
    IconChip(dl, a + S2(18, 18), Ico_Shield, theme::success, S(30));
    Text(dl, fonts::body, ImVec2(a.x + S(58), a.y + S(24)), Col(theme::textDim), T(S_DIAGNOSTICS));

    bool have = g.doctorReport.is_object();
    int  okN = 0, warn = 0, crit = 0;
    if (have)
    {
        warn = (int)g.doctorReport.value("warnings", 0);
        crit = (int)g.doctorReport.value("critical", 0);
        size_t diags = g.doctorReport.contains("diagnostics") && g.doctorReport["diagnostics"].is_array()
                           ? g.doctorReport["diagnostics"].size()
                           : 0;
        okN = (int)diags - warn - crit;
        if (okN < 0) okN = 0;
    }
    float target = have ? (float)okN : 0.0f;
    float n = anim::ToFrom(ImGui::GetID("##diag"), target, 3.0f, 0.0f);
    char num[16];
    snprintf(num, sizeof(num), "%s", have ? std::to_string((int)(n + 0.5f)).c_str() : "—");
    ImVec2 ns = Measure(fonts::display, num, S(36));
    Text(dl, fonts::display, ImVec2(a.x + S(18), a.y + S(58)), Col(theme::text), num, S(36));
    char sub[64];
    if (have) snprintf(sub, sizeof(sub), "+%d %s · %d %s", warn, T(S_K_WARN), crit, T(S_K_CRIT));
    else      snprintf(sub, sizeof(sub), "%s…", T(S_DIAGNOSTICS));
    ImVec4 sc = crit > 0 ? theme::danger : warn > 0 ? theme::warning : theme::success;
    Text(dl, fonts::small, ImVec2(a.x + S(24) + ns.x, a.y + S(58) + ns.y - S(20)), Col(sc), sub);

    // severity bars: ok / warnings / critical
    const float vals[3] = { have ? ImSaturate(okN / 8.0f) : 0.15f,
                            have ? ImSaturate(warn / 4.0f) : 0.15f,
                            have ? ImSaturate(crit / 2.0f) : 0.15f };
    const ImVec4 cols[3] = { theme::success, theme::warning, theme::danger };
    float bw = S(7), bg = S(5), maxH = S(44);
    float bx = b.x - S(18) - 3 * bw - 2 * bg, by = b.y - S(18);
    for (int i = 0; i < 3; i++)
    {
        float k = anim::OutCubic(grow * 1.4f - i * 0.06f);
        float hgt = ImMax(maxH * vals[i] * k, S(3));
        ImVec2 p0(bx + i * (bw + bg), by - hgt), p1(p0.x + bw, by);
        Gradient(dl, p0, p1, Col(cols[i], 0.95f), Col(cols[i], 0.30f), S(3), true);
    }

    // refresh
    float bs = S(26);
    ImGui::SetCursorScreenPos(ImVec2(b.x - bs - S(12), a.y + S(12)));
    if (ui::IconButton("##docrefresh", Ico_Refresh, bs, T(S_REFRESH))) StartDoctor();
}

// Accent placeholder with the brand mark (registry servers have no photos).
void ServerArt(ImDrawList* dl, const reg::ServerView& sv, ImVec2 a, ImVec2 b, float r)
{
    Gradient(dl, a, b, Col(sv.c0, 0.92f), Col(sv.c1, 0.92f), r, true);
    dl->PushClipRect(a, b, true);
    Glow(dl, ImVec2((a.x + b.x) * 0.5f, (a.y + b.y) * 0.5f), (b.x - a.x) * 0.8f, Col(White(), 0.22f));
    dl->PopClipRect();
    BrandMark(dl, ImVec2((a.x + b.x) * 0.5f, (a.y + b.y) * 0.5f), (b.x - a.x) * 0.16f, false);
}

// One "+" cell: opens the add-server dialog (empty library gets a big one).
void AddTile(ImVec2 p, ImVec2 sz, bool big)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    char id[16];
    snprintf(id, sizeof(id), "##add%s", big ? "big" : "sml");
    ImGui::SetCursorScreenPos(p);
    bool pressed = ImGui::InvisibleButton(id, sz);
    ImGuiID gid = ImGui::GetItemID();
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float h = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 12.0f);
    ImVec2 a = p - ImVec2(0, S(3) * h), b = a + sz;
    float r = big ? S(24) : S(16);

    dl->AddRectFilled(a, b, Col(theme::Ink(0.030f + 0.045f * h)), r);
    float dash = big ? S(30) : S(17);
    GradientBorder(dl, a, b, r, Col(Mix(theme::Ink(0.14f + 0.12f * h), theme::accent2, 0.55f * h)),
                   Col(theme::Ink(0.07f)), S(1.4f));
    ImVec2 c((a.x + b.x) * 0.5f, (a.y + b.y) * 0.5f - (big ? S(8) : 0.0f));
    float arm = dash * (1.0f + 0.06f * h), th = big ? S(4.0f) : S(2.6f);
    dl->AddLine(c - ImVec2(arm, 0), c + ImVec2(arm, 0), Col(Mix(theme::textDim, theme::accent2, 0.5f * h)), th);
    dl->AddLine(c - ImVec2(0, arm), c + ImVec2(0, arm), Col(Mix(theme::textDim, theme::accent2, 0.5f * h)), th);
    if (big)
        TextC(dl, fonts::smallBold, ImVec2(c.x, c.y + S(34)), Col(theme::textDim), T(S_ADD_SERVER));
    if (pressed) { SetPage(PG_MCP); OpenServerDialog(-1); }
}

void QuickTile(int i, ImVec2 p, ImVec2 sz)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const reg::ServerView& sv = reg::servers[i];
    char id[16];
    snprintf(id, sizeof(id), "##qt%d", i);
    ImGui::SetCursorScreenPos(p);
    bool pressed = ImGui::InvisibleButton(id, sz);
    ImGuiID gid = ImGui::GetItemID();
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float h = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 12.0f);
    ImVec2 a = p - ImVec2(0, S(4) * h), b = a + sz;

    Shadow(dl, a, b, S(16), S(18), Col(sv.c0, 0.28f * h), ImVec2(0, S(8)));
    Panel(dl, a, b, S(16));
    dl->PushClipRect(a, b, true);
    Glow(dl, ImVec2((a.x + b.x) * 0.5f, a.y + S(40)), S(90), Col(sv.c0, 0.16f + 0.16f * h));
    dl->PopClipRect();
    if (h > 0.01f) GradientBorder(dl, a, b, S(16), Col(sv.c0, 0.6f * h), Col(theme::Ink(0.0f)), S(1.2f));

    float es = S(50);
    ImVec2 ea((a.x + b.x) * 0.5f - es * 0.5f, a.y + S(15));
    ServerArt(dl, sv, ea, ea + ImVec2(es, es), S(14));
    if (h > 0.01f)
    {
        ImVec2 pc = ea + ImVec2(es, es) * 0.5f;
        dl->AddRectFilled(ea, ea + ImVec2(es, es), Col(Black(0.45f * h)), S(14));
        dl->AddCircleFilled(pc, S(13) * h, Col(White(0.95f * h)), 24);
        DrawIcon(dl, Ico_Play, pc + ImVec2(S(1), 0), S(10) * h, Col(Black(0.85f * h)));
    }
    TextC(dl, fonts::bold, ImVec2((a.x + b.x) * 0.5f, a.y + S(82)), Col(theme::text), sv.name.c_str());

    ImVec2 dot(b.x - S(14), a.y + S(14));
    dl->AddCircleFilled(dot, S(3.5f), Col(sv.enabled ? theme::success : theme::textMute), 12);

    if (pressed)
    {
        if (!sv.enabled) toast::Push(toast::Warning, T(S_SERVER_DISABLED), sv.name.c_str());
        else             StartLoader(sv.name);
    }
}

void Home(ImVec2 a, ImVec2 b)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float x = a.x + S(30), y = a.y + S(26), w = b.x - a.x - S(60);

    char hello[160];
    snprintf(hello, sizeof(hello), "%s, %s", T(S_HELLO), g.user.c_str());
    PageHeader(ImVec2(x, y), w, hello, T(S_HOME_SUB));

    time_t now = time(nullptr);
    tm lt = *localtime(&now);
    char clk[16], date[16];
    strftime(clk, sizeof(clk), "%H:%M:%S", &lt);
    strftime(date, sizeof(date), "%d.%m.%Y", &lt);
    ImVec2 cs = Measure(fonts::mono, clk, S(22));
    Text(dl, fonts::mono, ImVec2(x + w - cs.x, y + S(4)), Col(theme::text), clk, S(22));
    ImVec2 dsz = Measure(fonts::small, date);
    Text(dl, fonts::small, ImVec2(x + w - dsz.x, y + S(34)), Col(theme::textMute), date);
    y += S(80);

    float gap = S(16), cw = (w - gap * 2) / 3, ch = S(116);
    StatServers(ImVec2(x, y), ImVec2(cw, ch));
    StatGateway(ImVec2(x + cw + gap, y), ImVec2(cw, ch));
    StatDiagnostics(ImVec2(x + (cw + gap) * 2, y), ImVec2(cw, ch), g.pageT);
    y += ch + S(20);

    // gateway control strip
    {
        bool running  = g.gwStatus.is_object() && g.gwStatus.value("running", false);
        bool starting = g.gwStartJob != 0, stopping = g.gwStopJob != 0;
        float sh = S(56);
        ImVec2 na(x, y), nb(x + w, y + sh);
        Panel(dl, na, nb, S(16));
        ImVec4 c = running ? (g.gwStatus.value("managed", false) ? theme::success : theme::warning) : theme::textMute;
        dl->AddCircleFilled(ImVec2(na.x + S(24), na.y + sh * 0.5f), S(4), Col(c), 16);
        Text(dl, fonts::bold, ImVec2(na.x + S(40), na.y + S(11)), Col(theme::text), T(S_GATEWAY));
        std::string sub = running ? g.gwStatus.value("bind", std::string()) : T(S_STOPPED);
        if (running && !g.gwStatus.value("managed", false)) { sub += " · "; sub += T(S_GATEWAY_EXTERNAL); }
        Text(dl, fonts::mono, ImVec2(na.x + S(40), na.y + S(31)), Col(theme::textMute), sub.c_str(), S(12));
        std::string prof = config::c.gwProfile.empty() ? T(S_ALL_SERVERS) : config::c.gwProfile;
        ImVec2 ps = Measure(fonts::small, prof.c_str());
        Text(dl, fonts::small, ImVec2(nb.x - S(110) - ps.x - S(28), na.y + (sh - ps.y) * 0.5f), Col(theme::textDim), prof.c_str());
        float bw = S(110), bh = S(36);
        ImGui::SetCursorScreenPos(ImVec2(nb.x - S(14) - bw, na.y + (sh - bh) * 0.5f));
        if (running)
        {
            char lbl[48];
            snprintf(lbl, sizeof(lbl), "%s##gwstop", stopping ? "…" : T(S_STOP));
            if (ui::Button(lbl, ImVec2(bw, bh), ui::Btn_Secondary, Ico_Close) && !stopping) StopGateway();
        }
        else
        {
            char lbl[48];
            snprintf(lbl, sizeof(lbl), "%s##gwstart", starting ? "…" : T(S_START));
            if (ui::Button(lbl, ImVec2(bw, bh), ui::Btn_Primary, Ico_Play) && !starting) StartGateway();
        }
        y += sh + S(24);
    }

    Text(dl, fonts::title, ImVec2(x, y), Col(theme::text), T(S_QUICK_LAUNCH));
    y += S(34);
    float tw = (w - gap * 4) / 5, th = S(108);
    std::vector<int> quickIdx;
    for (int i = 0; i < (int)reg::servers.size() && (int)quickIdx.size() < 5; i++)
        if (reg::servers[i].enabled) quickIdx.push_back(i);
    for (size_t i = 0; i < quickIdx.size(); i++)
        QuickTile(quickIdx[i], ImVec2(x + (int)i * (tw + gap), y), ImVec2(tw, th));
    if (!quickIdx.empty())
        AddTile(ImVec2(x + (int)quickIdx.size() * (tw + gap), y), ImVec2(tw, th), false);
    else
        AddTile(ImVec2(x, y), ImVec2(w, th), true);
    y += th + S(24);

    Text(dl, fonts::title, ImVec2(x, y), Col(theme::text), T(S_RECENT));
    y += S(34);
    float rh = S(40);
    int rows = ImClamp((int)g.logLines.size(), 0, 3);
    ImVec2 na2(x, y), nb2(x + w, y + rh * (rows > 0 ? rows : 1) + S(10));
    Panel(dl, na2, nb2, S(16));
    if (rows == 0)
    {
        Text(dl, fonts::body, ImVec2(na2.x + S(18), na2.y + S(10)), Col(theme::textMute), T(S_NO_ACTIVITY));
    }
    else
    {
        for (int i = 0; i < rows; i++)
        {
            const LogLine& L = g.logLines[(int)g.logLines.size() - rows + i];
            float ry = na2.y + S(5) + i * rh;
            if (i > 0) dl->AddLine(ImVec2(na2.x + S(16), ry), ImVec2(nb2.x - S(16), ry), Col(theme::Ink(0.05f)));
            ImVec4 ic = theme::accent2;
            if (L.text.find("ERROR") != std::string::npos) ic = theme::danger;
            else if (L.text.find("WARN") != std::string::npos) ic = theme::warning;
            else if (L.text.find("DEBUG") != std::string::npos) ic = theme::textMute;
            IconChip(dl, ImVec2(na2.x + S(14), ry + S(6)), Ico_Info, ic, S(28));
            ImVec2 ta(na2.x + S(54), ry + (rh - fonts::body->FontSize) * 0.5f);
            dl->PushClipRect(ImVec2(ta.x, ry + S(2)), ImVec2(nb2.x - S(90), ry + rh - S(2)), true);
            Text(dl, fonts::body, ta, Col(theme::text), L.text.c_str());
            dl->PopClipRect();
            char ago[24];
            if (L.age < 5.0f) snprintf(ago, sizeof(ago), "%s", T(S_JUST_NOW));
            else              snprintf(ago, sizeof(ago), "%ds", (int)L.age);
            ImVec2 as = Measure(fonts::small, ago);
            Text(dl, fonts::small, ImVec2(nb2.x - S(18) - as.x, ry + (rh - as.y) * 0.5f), Col(theme::textMute), ago);
        }
    }
}

// ================================================================== MCP servers
void OpenServerDialog(int editIndex)
{
    g.dlgName.clear();
    g.dlgTags.clear();
    g.dlgDesc.clear();
    g.dlgCmd = "npx";
    g.dlgArgs.clear();
    g.dlgCwd.clear();
    g.dlgUrl.clear();
    g.dlgToken.clear();
    g.dlgTransport = 0;
    g.dlgTimeout = config::c.timeout;
    g.dlgEnabled = true;
    g.dlgShowToken = false;
    g.dlgEditName.clear();
    g.dlgEnv.clear();

    if (editIndex >= 0 && editIndex < (int)reg::servers.size())
    {
        const reg::ServerView& sv = reg::servers[editIndex];
        const json& def = sv.def;
        g.dlgEditName = sv.name;
        g.dlgName = sv.name;
        g.dlgTags = reg::JoinTags(sv.tags);
        g.dlgDesc = sv.description;
        g.dlgEnabled = sv.enabled;
        if (def.contains("timeout_ms") && def["timeout_ms"].is_number())
            g.dlgTimeout = (int)(def["timeout_ms"].get<uint64_t>() / 1000);

        const json& tr = def.contains("transport") ? def["transport"] : json::object();
        if (sv.transportName == "http")
        {
            g.dlgTransport = 1;
            g.dlgUrl = tr.value("url", std::string());
            if (tr.contains("bearer_token") && tr["bearer_token"].is_object())
            {
                const json& bt = tr["bearer_token"];
                std::string src = bt.value("source", "literal");
                if (src == "literal") g.dlgToken = bt.value("value", std::string());
                else
                {
                    g.dlgToken = src + ":" + bt.value("name", std::string());
                    // non-literal tokens round-trip as env:/secret_ref: references
                }
            }
        }
        else
        {
            g.dlgTransport = 0;
            g.dlgCmd = tr.value("command", std::string());
            if (tr.contains("args") && tr["args"].is_array())
            {
                for (const auto& a : tr["args"])
                    if (a.is_string())
                    {
                        if (!g.dlgArgs.empty()) g.dlgArgs += " ";
                        g.dlgArgs += a.get<std::string>();
                    }
            }
            g.dlgCwd = tr.value("cwd", std::string());
            if (tr.contains("env") && tr["env"].is_object())
            {
                for (auto it = tr["env"].begin(); it != tr["env"].end(); ++it)
                {
                    EnvRow row;
                    row.key = it.key();
                    std::string src = it.value().value("source", "literal");
                    if (src == "literal") { row.type = 0; row.val = it.value().value("value", std::string()); }
                    else                  { row.type = src == "env" ? 1 : 2; row.val = it.value().value("name", std::string()); }
                    g.dlgEnv.push_back(row);
                }
            }
        }
    }
    g.dlgOpen = true;
}

// Left-aligned word-wrapped text (up to maxLines), returns the y after the last line.
float WrapText(ImDrawList* dl, ImFont* f, float size, float x, float y, float width, ImU32 col, const char* text, int maxLines)
{
    const char* s = text;
    const char* end = text + strlen(text);
    float scale = size / f->FontSize;
    int lines = 0;
    for (; s < end && lines < maxLines; lines++)
    {
        const char* e = f->CalcWordWrapPositionA(scale, s, end, width);
        if (e <= s) e = s + 1;
        const char* te = e;
        while (te > s && te[-1] == ' ') te--;
        dl->AddText(f, size, ImVec2(floorf(x), floorf(y + lines * size * 1.35f)), col, s, te);
        s = e;
        while (s < end && (*s == ' ' || *s == '\n')) s++;
    }
    return y + lines * size * 1.35f;
}

static bool DialogField(const char* id, char* buf, size_t n, const char* hint, float w, float h,
                        bool multiline = false, ImGuiInputTextFlags extra = 0)
{
    ImGui::SetNextItemWidth(w);
    ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(12), multiline ? S(9) : S(11)));
    ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, S(10));
    bool enter = false;
    if (multiline)
        ImGui::InputTextMultiline(id, buf, n, ImVec2(w, h), extra);
    else
        enter = ImGui::InputTextWithHint(id, hint, buf, n, extra | ImGuiInputTextFlags_EnterReturnsTrue);
    ImGui::PopStyleVar(2);
    return enter;
}

static bool DialogFieldStr(const char* id, std::string& buf, const char* hint, float w, float h,
                           bool multiline = false, ImGuiInputTextFlags extra = 0)
{
    ImGui::SetNextItemWidth(w);
    ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(12), multiline ? S(9) : S(11)));
    ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, S(10));
    bool enter = false;
    if (multiline)
        ImGui::InputTextMultiline(id, &buf, ImVec2(w, h), extra);
    else
        enter = ImGui::InputTextWithHint(id, hint, &buf, extra | ImGuiInputTextFlags_EnterReturnsTrue);
    ImGui::PopStyleVar(2);
    return enter;
}

void ServerDialog(ImVec2 ds)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();

    // dim + swallow clicks that fall outside the panel
    dl->AddRectFilled(ImVec2(0, 0), ds, Col(Black(0.42f)));
    ImGui::SetCursorScreenPos(ImVec2(0, 0));
    if (ImGui::InvisibleButton("##dlgdim", ds) || ImGui::IsKeyPressed(ImGuiKey_Escape)) g.dlgOpen = false;
    if (!g.dlgOpen) return;

    ImVec2 csz(ImVec2(S(600), S(680))), cc = ds * 0.5f;
    ImVec2 ca = cc - csz * 0.5f, cb = ca + csz;
    Shadow(dl, ca, cb, S(22), S(34), Col(Black(0.5f)), ImVec2(0, S(12)));
    Gradient(dl, ca, cb, Col(ImVec4(0.995f, 0.995f, 1.0f, 0.99f)), Col(ImVec4(0.965f, 0.968f, 0.976f, 0.99f)), S(22), true);
    GradientBorder(dl, ca, cb, S(22), Col(theme::Ink(0.14f)), Col(theme::Ink(0.05f)), S(1.2f));
    Gradient(dl, ImVec2(ca.x + csz.x * 0.28f, ca.y), ImVec2(ca.x + csz.x * 0.72f, ca.y + S(2)),
             Col(theme::accent, 0.0f), Col(theme::accent2, 0.9f));

    float px = ca.x + S(28), w = csz.x - S(56), y = ca.y + S(24);
    bool editing = !g.dlgEditName.empty();
    Text(dl, fonts::title, ImVec2(px, y), Col(theme::text), editing ? T(S_EDIT_SERVER) : T(S_ADD_SERVER_TITLE));
    float bs = S(30);
    ImGui::SetCursorScreenPos(ImVec2(cb.x - S(16) - bs, y - S(2)));
    if (ui::IconButton("##dlgclose", Ico_Close, bs)) g.dlgOpen = false;
    y += S(40);

    // scrollable fields
    float footY = cb.y - S(80);
    ImGui::SetCursorScreenPos(ImVec2(px - S(6), y));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
    ImGui::BeginChild("##dlgfields", ImVec2(w + S(12), footY - y - S(6)),
                      ImGuiChildFlags_None, ImGuiWindowFlags_NoBackground | ImGuiWindowFlags_AlwaysVerticalScrollbar);
    ImGui::PopStyleVar();
    float fw = w - S(10);
    bool enter = false;

    Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_SERVER_NAME));
    ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
    enter = DialogFieldStr("##dlgname", g.dlgName, "claude-bridge", fw, S(38));
    y += S(20) + S(38) + S(12);

    Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_TRANSPORT));
    {
        float pw = S(120);
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        if (StylePill("##trstdio", g.dlgTransport == 0, ImVec2(pw, S(34)))) g.dlgTransport = 0;
        ImGui::SameLine(0, S(8));
        ImGui::SetCursorScreenPos(ImGui::GetCursorScreenPos() + ImVec2(0, 0));
        if (StylePill("##trhttp", g.dlgTransport == 1, ImVec2(pw, S(34)))) g.dlgTransport = 1;
        ImVec2 ts = Measure(fonts::smallBold, "stdio");
        Text(dl, fonts::smallBold, ImVec2(px + (pw - ts.x) * 0.5f, y + S(20) + (S(34) - ts.y) * 0.5f), Col(theme::textDim), "stdio");
        ts = Measure(fonts::smallBold, "http");
        Text(dl, fonts::smallBold, ImVec2(px + pw + S(8) + (pw - ts.x) * 0.5f, y + S(20) + (S(34) - ts.y) * 0.5f), Col(theme::textDim), "http");
    }
    y += S(20) + S(34) + S(12);

    if (g.dlgTransport == 0)
    {
        Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_COMMAND));
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        enter = DialogFieldStr("##dlgcmd", g.dlgCmd, "npx", fw, S(38)) || enter;
        y += S(20) + S(38) + S(12);
        Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_ARGS));
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        enter = DialogFieldStr("##dlgargs", g.dlgArgs, "-y @scope/package", fw, S(38)) || enter;
        y += S(20) + S(38) + S(12);
        Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_CWD));
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        enter = DialogFieldStr("##dlgcwd", g.dlgCwd, "/home/user/project", fw, S(38)) || enter;
        y += S(20) + S(38) + S(12);
    }
    else
    {
        Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_URL));
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        enter = DialogFieldStr("##dlgurl", g.dlgUrl, "https://example.com/mcp", fw, S(38)) || enter;
        y += S(20) + S(38) + S(12);
        Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_BEARER));
        ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
        enter = DialogFieldStr("##dlgtok", g.dlgToken, "", fw - S(46), S(38), false,
                               g.dlgShowToken ? ImGuiInputTextFlags_None : ImGuiInputTextFlags_Password) || enter;
        ImGui::SetCursorScreenPos(ImVec2(px + fw - S(40), y + S(22)));
        if (ui::IconButton("##dlgtokeneye", g.dlgShowToken ? Ico_EyeOff : Ico_Eye, S(34)))
            g.dlgShowToken = !g.dlgShowToken;
        y += S(20) + S(38) + S(12);
    }

    // environment rows
    Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_ENV));
    y += S(22);
    for (size_t r = 0; r < g.dlgEnv.size(); r++)
    {
        EnvRow& row = g.dlgEnv[r];
        float kw = S(140), tw = S(110), xw = S(30);
        ImGui::SetCursorScreenPos(ImVec2(px, y));
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, S(8));
        ImGui::SetNextItemWidth(kw);
        ImGui::InputTextWithHint(("##evk" + std::to_string(r)).c_str(), T(S_ENV_KEY), &row.key);
        ImGui::SameLine(0, S(8));
        ImGui::SetNextItemWidth(fw - kw - tw - xw - S(16));
        ImGui::InputTextWithHint(("##evv" + std::to_string(r)).c_str(), T(S_ENV_VALUE), &row.val);
        ImGui::SameLine(0, S(8));
        ImGui::SetNextItemWidth(tw);
        const char* preview = row.type == 0 ? "literal" : row.type == 1 ? "env" : "secret_ref";
        if (ImGui::BeginCombo(("##evt" + std::to_string(r)).c_str(), preview))
        {
            if (ImGui::Selectable("literal", row.type == 0)) row.type = 0;
            if (ImGui::Selectable("env", row.type == 1)) row.type = 1;
            if (ImGui::Selectable("secret_ref", row.type == 2)) row.type = 2;
            ImGui::EndCombo();
        }
        ImGui::SameLine(0, S(6));
        char xid[16];
        snprintf(xid, sizeof(xid), "##evx%d", (int)r);
        if (ui::IconButton(xid, Ico_Close, S(28))) { g.dlgEnv.erase(g.dlgEnv.begin() + (long)r); r--; }
        ImGui::PopStyleVar(2);
        y += S(36) + S(8);
    }
    {
        char addLbl[48];
        snprintf(addLbl, sizeof(addLbl), "%s##evadd", T(S_ADD_ROW));
        ImGui::SetCursorScreenPos(ImVec2(px, y));
        if (ui::Button(addLbl, ImVec2(S(160), S(34)), ui::Btn_Secondary)) g.dlgEnv.push_back({});
        y += S(34) + S(12);
    }

    // tags + timeout + enabled
    char tagHint[96];
    snprintf(tagHint, sizeof(tagHint), "%s: files, git, browser", T(S_SERVER_TAGS));
    ImGui::SetCursorScreenPos(ImVec2(px, y));
    enter = DialogFieldStr("##dlgtags", g.dlgTags, tagHint, fw, S(38)) || enter;
    y += S(38) + S(12);

    Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_TIMEOUT));
    Text(dl, fonts::smallBold, ImVec2(px + S(200), y), Col(theme::textMute), T(S_ENABLED));
    ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
    {
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, S(8));
        ImGui::SetNextItemWidth(S(160));
        float tv = (float)g.dlgTimeout;
        if (ImGui::InputFloat("##dlgto", &tv, 0, 0, "%.0f")) g.dlgTimeout = (int)ImClamp(tv, 3.0f, 600.0f);
        ImGui::PopStyleVar(2);
        ImGui::SameLine(0, S(40));
        ImGui::SetCursorScreenPos(ImGui::GetCursorScreenPos() + ImVec2(0, S(4)));
        ImGui::Checkbox("##dlgen", &g.dlgEnabled);
    }
    y += S(20) + S(34) + S(12);

    Text(dl, fonts::smallBold, ImVec2(px, y), Col(theme::textMute), T(S_SERVER_NOTE));
    ImGui::SetCursorScreenPos(ImVec2(px, y + S(20)));
    DialogFieldStr("##dlgdesc", g.dlgDesc, "", fw, S(50), true);

    float contentY = ImGui::GetCursorScreenPos().y;
    ImGui::Dummy(ImVec2(1, 1));
    ImGui::EndChild();

    // footer buttons
    ImGui::SetCursorScreenPos(ImVec2(px, footY + S(8)));
    char cancelLbl[48];
    snprintf(cancelLbl, sizeof(cancelLbl), "%s##dlgcancel", T(S_CANCEL));
    if (ui::Button(cancelLbl, ImVec2(S(150), S(44)), ui::Btn_Ghost, Ico_Close)) g.dlgOpen = false;
    ImGui::SetCursorScreenPos(ImVec2(px + w - S(170), footY + S(8)));
    char saveLbl[48];
    snprintf(saveLbl, sizeof(saveLbl), "%s##dlgsave", T(S_SAVE));
    if (ui::Button(saveLbl, ImVec2(S(170), S(44)), ui::Btn_Primary, Ico_Check) || enter) SaveDialogServer();
    (void)contentY;
}

void SaveDialogServer()
{
    std::string name = g.dlgName;
    while (!name.empty() && name.front() == ' ') name.erase(name.begin());
    while (!name.empty() && name.back() == ' ') name.pop_back();
    if (name.empty()) { toast::Push(toast::Error, T(S_NAME_REQUIRED)); return; }

    int existing = reg::IndexOf(name);
    bool editing = !g.dlgEditName.empty();
    if (existing >= 0 && (!editing || name != g.dlgEditName))
    {
        toast::Push(toast::Error, T(S_NAME_EXISTS), name.c_str());
        return;
    }

    json tr;
    if (g.dlgTransport == 0)
    {
        std::string cmd = g.dlgCmd;
        while (!cmd.empty() && cmd.front() == ' ') cmd.erase(cmd.begin());
        while (!cmd.empty() && cmd.back() == ' ') cmd.pop_back();
        if (cmd.empty()) { toast::Push(toast::Error, T(S_COMMAND)); return; }
        tr["type"] = "stdio";
        tr["command"] = cmd;
        std::vector<std::string> args = reg::SplitTags(g.dlgArgs);   // whitespace split
        if (!args.empty()) tr["args"] = args;
        std::string cwd = g.dlgCwd;
        while (!cwd.empty() && cwd.back() == ' ') cwd.pop_back();
        if (!cwd.empty()) tr["cwd"] = cwd;
        json env = json::object();
        for (const EnvRow& row : g.dlgEnv)
        {
            if (row.key.empty()) continue;
            if (row.type == 0)      env[row.key] = json{ { "source", "literal" }, { "value", row.val } };
            else if (row.type == 1) env[row.key] = json{ { "source", "env" }, { "name", row.val } };
            else                    env[row.key] = json{ { "source", "secret_ref" }, { "name", row.val } };
        }
        if (!env.empty()) tr["env"] = env;
    }
    else
    {
        std::string url = g.dlgUrl;
        while (!url.empty() && url.front() == ' ') url.erase(url.begin());
        while (!url.empty() && url.back() == ' ') url.pop_back();
        if (url.empty()) { toast::Push(toast::Error, T(S_URL)); return; }
        tr["type"] = "http";
        tr["url"] = url;
        if (!g.dlgToken.empty())
        {
            std::string tok = g.dlgToken;
            if (tok.rfind("env:", 0) == 0)
                tr["bearer_token"] = json{ { "source", "env" }, { "name", tok.substr(4) } };
            else if (tok.rfind("secret_ref:", 0) == 0)
                tr["bearer_token"] = json{ { "source", "secret_ref" }, { "name", tok.substr(11) } };
            else
                tr["bearer_token"] = json{ { "source", "literal" }, { "value", tok } };
        }
    }

    json def;
    def["enabled"] = g.dlgEnabled;
    def["transport"] = tr;
    std::vector<std::string> tags = reg::SplitTags(g.dlgTags);
    if (!tags.empty()) def["tags"] = tags;
    if (!g.dlgDesc.empty()) def["description"] = g.dlgDesc;
    if (g.dlgTimeout > 0) def["timeout_ms"] = (uint64_t)g.dlgTimeout * 1000ULL;

    if (!reg::registry.contains("servers") || !reg::registry["servers"].is_object())
        reg::registry["servers"] = json::object();
    if (editing && g.dlgEditName != name) reg::registry["servers"].erase(g.dlgEditName);
    reg::registry["servers"][name] = def;

    std::string err;
    if (!reg::Save(&err))
    {
        toast::Push(toast::Error, T(S_SAVE), err.c_str(), 6.0f);
        return;
    }
    g.selected = reg::IndexOf(name);
    g.dlgOpen = false;
    toast::Push(toast::Success, T(S_SERVER_ADDED), name.c_str());
}

void ServerCard(int i, ImVec2 p, float w)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const reg::ServerView& sv = reg::servers[i];
    float h = S(258);
    char id[16];
    snprintf(id, sizeof(id), "##sv%d", i);
    ImGui::SetCursorScreenPos(p);
    bool pressed = ImGui::InvisibleButton(id, ImVec2(w, h));
    ImGuiID gid = ImGui::GetItemID();
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    float hv = anim::To(anim::Key(gid, 0), hov ? 1.0f : 0.0f, 12.0f);
    float lift = S(5) * hv;
    ImVec2 a = p - ImVec2(0, lift), b = a + ImVec2(w, h);

    Shadow(dl, a, b, S(18), S(20) * (0.5f + 0.5f * hv), Col(sv.c0, 0.24f * hv), ImVec2(0, S(8)));
    Panel(dl, a, b, S(18));
    if (hv > 0.01f) GradientBorder(dl, a, b, S(18), Col(sv.c0, 0.55f * hv), Col(theme::Ink(0.04f)), S(1.2f));

    // art + hover overlay (play = connect)
    ImVec2 pa = a + ImVec2(S(10), S(10)), pb = pa + ImVec2(w - S(20), S(112));
    ServerArt(dl, sv, pa, pb, S(12));
    if (hov && sv.enabled)
    {
        dl->PushClipRect(pa, pb, true);
        dl->AddRectFilled(pa, pb, Col(Black(0.34f * hv)), S(12));
        ImVec2 pc = (pa + pb) * 0.5f;
        dl->AddCircleFilled(pc, S(17) * hv, Col(White(0.95f)), 32);
        DrawIcon(dl, Ico_Play, pc + ImVec2(S(1.5f), 0), S(13) * hv, Col(Black(0.85f)));
        dl->PopClipRect();
    }
    ImVec2 dot(pb.x - S(9), pa.y + S(9));
    dl->AddCircleFilled(dot, S(4.5f), Col(White()), 16);
    dl->AddCircleFilled(dot, S(3), Col(sv.enabled ? theme::success : theme::textMute), 16);

    // name
    float y = a.y + S(130);
    float maxW = w - S(32);
    float fsz = S(17);
    ImVec2 ns = Measure(fonts::bold, sv.name.c_str(), fsz);
    if (ns.x > maxW) fsz *= maxW / ns.x;
    Text(dl, fonts::bold, ImVec2(a.x + S(16), y), Col(theme::text), sv.name.c_str(), fsz);
    y += S(24);

    // endpoint (command line or url)
    if (!sv.endpoint.empty())
    {
        std::string ep = sv.endpoint;
        if (ep.size() > 42) ep = ep.substr(0, 41) + "…";
        Text(dl, fonts::mono, ImVec2(a.x + S(16), y), Col(theme::accent2), ep.c_str(), S(11));
        y += S(18);
    }

    if (hov)
    {
        // action row: edit / toggle / delete
        float bw = (w - S(32) - S(12)) / 3, bh = S(30);
        float bx = a.x + S(16), by = a.y + h - S(46);
        ImGui::SetCursorScreenPos(ImVec2(bx, by));
        char e1[48];
        snprintf(e1, sizeof(e1), "%s##ed%d", T(S_EDIT_SERVER), i);
        if (ui::Button(e1, ImVec2(bw, bh), ui::Btn_Secondary)) OpenServerDialog(i);
        ImGui::SetCursorScreenPos(ImVec2(bx + bw + S(6), by));
        char e2[48];
        snprintf(e2, sizeof(e2), "%s##tg%d", sv.enabled ? T(S_DISABLED) : T(S_ENABLED), i);
        if (ui::Button(e2, ImVec2(bw, bh), ui::Btn_Ghost))
        {
            reg::registry["servers"][sv.name]["enabled"] = !sv.enabled;
            std::string err;
            if (!reg::Save(&err)) toast::Push(toast::Error, T(S_SAVE), err.c_str(), 6.0f);
            else                  toast::Push(toast::Info, sv.name.c_str(), sv.enabled ? T(S_DISABLED) : T(S_ENABLED));
        }
        ImGui::SetCursorScreenPos(ImVec2(bx + (bw + S(6)) * 2, by));
        char e3[48];
        snprintf(e3, sizeof(e3), "%s##del%d", T(S_DELETE), i);
        if (ui::Button(e3, ImVec2(bw, bh), ui::Btn_Danger)) g.confirmDelete = sv.name;
        return;   // buttons replaced the note area
    }

    // tags (max two + counter)
    float tx = a.x + S(16);
    int shown = 0;
    for (size_t k = 0; k < sv.tags.size() && shown < 2; k++, shown++)
    {
        std::string t = sv.tags[k];
        if (t.size() > 12) t = t.substr(0, 12);
        float cw = ui::Chip(dl, ImVec2(tx, y), t.c_str(), sv.c0);
        if (tx + cw > a.x + w - S(16)) { tx = a.x + S(16); y += S(24); }
        tx += cw + S(6);
    }
    if ((int)sv.tags.size() > shown)
    {
        char more[16];
        snprintf(more, sizeof(more), "+%d", (int)sv.tags.size() - shown);
        ui::Chip(dl, ImVec2(tx, y), more, theme::textMute);
    }
    y += S(28);

    // description
    if (!sv.description.empty())
        WrapText(dl, fonts::small, fonts::small->FontSize, a.x + S(16), y, w - S(32),
                 Col(theme::textDim), sv.description.c_str(), 2);

    if (pressed)
    {
        if (!sv.enabled) toast::Push(toast::Warning, T(S_SERVER_DISABLED), sv.name.c_str());
        else             StartLoader(sv.name);
    }
}

void Mcps(ImVec2 a, ImVec2 b)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float x = a.x + S(30), y = a.y + S(22), w = b.x - a.x - S(60);
    int n = (int)reg::servers.size();
    PageHeader(ImVec2(x, y), w, T(S_NAV_GAMES), T(S_PRODUCTS_SUB));

    // "+" on the side of the header whenever something is registered
    if (n > 0)
    {
        float bs = S(40);
        ImVec2 pb(x + w - bs, y + S(6));
        ImGui::SetCursorScreenPos(pb);
        bool pressed = ImGui::InvisibleButton("##addhdr", ImVec2(bs, bs));
        ImGuiID id = ImGui::GetItemID();
        bool hov = ImGui::IsItemHovered();
        if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
        float hv = anim::To(anim::Key(id, 0), hov ? 1.0f : 0.0f, 12.0f);
        ImVec2 pb2(pb.x + bs, pb.y + bs);
        ImVec2 pc = pb + ImVec2(bs * 0.5f, bs * 0.5f);
        dl->AddRectFilled(pb, pb2, Col(theme::Ink(0.035f + 0.05f * hv)), S(11));
        GradientBorder(dl, pb, pb2, S(11), Col(Mix(theme::Ink(0.14f + 0.12f * hv), theme::accent2, 0.7f * hv)),
                       Col(theme::Ink(0.06f)), S(1.2f));
        float arm = S(9) * (1.0f + 0.08f * hv);
        dl->AddLine(pc - ImVec2(arm, 0), pc + ImVec2(arm, 0), Col(theme::accent2), S(2.2f));
        dl->AddLine(pc - ImVec2(0, arm), pc + ImVec2(0, arm), Col(theme::accent2), S(2.2f));
        ui::Tooltip(T(S_ADD_SERVER));
        if (pressed) OpenServerDialog(-1);
    }

    float top = y + S(74);
    if (n == 0)
    {
        float cy = ImMax(top, (a.y + b.y) * 0.5f - S(70));
        TextC(dl, fonts::title, ImVec2((a.x + b.x) * 0.5f, cy - S(50)), Col(theme::textDim), T(S_EMPTY_MCP));
        ImVec2 sz(S(124), S(124));
        AddTile(ImVec2((a.x + b.x) * 0.5f - sz.x * 0.5f, cy - S(16)), sz, true);
    }
    else
    {
        float gap = S(18);
        int cols = 3;
        float cw = (w - gap * (cols - 1)) / cols;
        float ch = S(258) + gap;
        int cells = n + 1;                       // the trailing "+" lives in the grid
        int rows = (cells + cols - 1) / cols;

        ImGui::SetCursorScreenPos(ImVec2(x - S(12), top));
        ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
        ImGui::BeginChild("##mcplist", ImVec2(w + S(24), b.y - top - S(6)),
                          ImGuiChildFlags_None, ImGuiWindowFlags_NoBackground | ImGuiWindowFlags_AlwaysVerticalScrollbar);
        ImVec2 o = ImGui::GetCursorScreenPos();
        for (int c = 0; c < cells; c++)
        {
            int col = c % cols, row = c / cols;
            ImVec2 p(o.x + col * (cw + gap), o.y + row * ch);
            if (c < n) ServerCard(c, p, cw);
            else       AddTile(p, ImVec2(cw, S(258)), false);
        }
        ImGui::Dummy(ImVec2(cols * (cw + gap), rows * ch));
        ImGui::EndChild();
        ImGui::PopStyleVar();
    }

    // delete confirmation
    if (!g.confirmDelete.empty())
        ImGui::OpenPopup("##confirmdel");
    bool wasOpen = g.confirmWasOpen;
    bool delOpen = false;
    if (ImGui::BeginPopupModal("##confirmdel", nullptr, ImGuiWindowFlags_AlwaysAutoResize))
    {
        delOpen = true;
        ImGui::Text("%s", T(S_DELETE));
        ImGui::Separator();
        ImGui::TextWrapped("%s", g.confirmDelete.c_str());
        ImGui::Dummy(ImVec2(0, S(6)));
        if (ui::Button(T(S_DELETE), ImVec2(S(140), S(42)), ui::Btn_Danger))
        {
            reg::registry["servers"].erase(g.confirmDelete);
            std::string err;
            if (!reg::Save(&err)) toast::Push(toast::Error, T(S_DELETE), err.c_str(), 6.0f);
            else                  toast::Push(toast::Info, T(S_DELETE), g.confirmDelete.c_str());
            g.confirmDelete.clear();
            ImGui::CloseCurrentPopup();
        }
        ImGui::SameLine(0, S(10));
        if (ui::Button(T(S_CANCEL), ImVec2(S(140), S(42)), ui::Btn_Ghost))
        {
            g.confirmDelete.clear();
            ImGui::CloseCurrentPopup();
        }
        ImGui::EndPopup();
    }
    g.confirmWasOpen = delOpen;
    if (!delOpen && wasOpen) g.confirmDelete.clear();   // dismissed with escape

    if (g.dlgOpen) ServerDialog(ImGui::GetIO().DisplaySize);
}

// ================================================================== settings
void InfoRow(const char* id, const char* label, const char* value, const char* copyValue, float w)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 p = ImGui::GetCursorScreenPos();
    float h = S(34);
    Text(dl, fonts::body, ImVec2(p.x, p.y + (h - fonts::body->FontSize) * 0.5f), Col(theme::textDim), label);
    float right = p.x + w - (copyValue ? S(36) : 0.0f);
    ImVec2 vs = Measure(fonts::mono, value, S(13));
    Text(dl, fonts::mono, ImVec2(right - vs.x, p.y + (h - vs.y) * 0.5f), Col(theme::text), value, S(13));
    if (copyValue)
    {
        ImGui::SetCursorScreenPos(ImVec2(p.x + w - S(28), p.y + (h - S(28)) * 0.5f));
        if (ui::IconButton(id, Ico_Copy, S(28), T(S_COPY)))
        {
            ImGui::SetClipboardText(copyValue);
            toast::Push(toast::Success, T(S_COPIED), nullptr, 2.2f);
        }
    }
    dl->AddLine(ImVec2(p.x, p.y + h + S(4)), ImVec2(p.x + w, p.y + h + S(4)), Col(theme::Ink(0.05f)));
    ImGui::SetCursorScreenPos(p);
    ImGui::Dummy(ImVec2(w, h));
}

bool StylePill(const char* label, bool sel, ImVec2 size)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    ImVec2 p = ImGui::GetCursorScreenPos();
    bool pressed = ImGui::InvisibleButton(label, size);
    bool hov = ImGui::IsItemHovered();
    if (hov) ImGui::SetMouseCursor(ImGuiMouseCursor_Hand);
    ImGuiID id = ImGui::GetItemID();
    float h = anim::To(anim::Key(id, 0), hov ? 1.0f : 0.0f, 14.0f);
    float s = anim::To(anim::Key(id, 1), sel ? 1.0f : 0.0f, 12.0f);
    ImVec2 b = p + size;
    float r = S(10);
    dl->AddRectFilled(p, b, Col(theme::Ink(0.035f + 0.04f * h)), r);
    if (s > 0.01f) Gradient(dl, p, b, Col(theme::accent, 0.30f * s), Col(theme::accent2, 0.10f * s), r);
    GradientBorder(dl, p, b, r, Col(Mix(theme::Ink(0.08f + 0.06f * h), theme::accent2, s * 0.8f)), Col(theme::Ink(0.03f)), S(1.0f));
    const char* end = ImGui::FindRenderedTextEnd(label);
    ImVec2 ts = Measure(fonts::smallBold, label, 0.0f, end);
    Text(dl, fonts::smallBold, p + (size - ts) * 0.5f, Col(Mix(theme::textDim, theme::text, ImMax(h, s))), label, 0.0f, end);
    return pressed;
}

void Settings(ImVec2 a, ImVec2 b)
{
    float x = a.x + S(30), y = a.y + S(26), w = b.x - a.x - S(60);
    PageHeader(ImVec2(x, y), w, T(S_NAV_SETTINGS), T(S_SETTINGS_SUB));
    float top = y + S(80);

    ImGui::SetCursorScreenPos(ImVec2(x - S(12), top));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
    ImGui::BeginChild("##settings", ImVec2(w + S(24), b.y - top), ImGuiChildFlags_None, ImGuiWindowFlags_NoBackground);
    ImVec2 o = ImGui::GetCursorScreenPos();
    float gap = S(18), cw = (w - gap) * 0.5f;
    float lx = o.x + S(12), rx = lx + cw + gap, ty = o.y + S(6);
    char buf[64];

    // ---------------------------------------------------------- appearance
    ui::CardBegin(T(S_APPEARANCE), Ico_Sparkle, ImVec2(lx, ty), cw);
    float iw = ui::CardInnerWidth();
    Label(T(S_ACCENT));
    float sw = S(36);
    for (int k = 0; k <= theme::kPresetCount; k++)
    {
        snprintf(buf, sizeof(buf), "##sw%d", k);
        bool custom = k == theme::kPresetCount;
        ImVec4 ca, cb;
        theme::TargetAccent(k, ca, cb);
        if (ui::Swatch(buf, ca, cb, config::c.accent == k, sw, custom))
        {
            if (custom) ImGui::OpenPopup("##accentpick");
            else { config::c.accent = k; config::MarkDirty(); }
        }
        ui::Tooltip(custom ? T(S_CUSTOM) : theme::kPresets[k].name);
        if (k < theme::kPresetCount) ImGui::SameLine(0, S(8));
    }
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, S2(12, 12));
    if (ImGui::BeginPopup("##accentpick"))
    {
        ImGui::SetNextItemWidth(S(220));
        if (ImGui::ColorPicker3("##pick", &config::c.customAccent.x,
                                ImGuiColorEditFlags_NoSidePreview | ImGuiColorEditFlags_PickerHueWheel | ImGuiColorEditFlags_NoInputs | ImGuiColorEditFlags_NoLabel))
        {
            config::c.accent = theme::kPresetCount;
            config::MarkDirty();
        }
        ImGui::EndPopup();
    }
    ImGui::PopStyleVar();
    ImGui::Dummy(ImVec2(0, S(2)));
    if (ui::Toggle(T(S_BG_EFFECTS), T(S_BG_EFFECTS_D), &config::c.bgEffects, iw)) config::MarkDirty();
    if (ui::Toggle(T(S_PARTICLES), T(S_PARTICLES_D), &config::c.particles, iw)) config::MarkDirty();
    ImGui::Dummy(ImVec2(0, S(2)));
    Label(T(S_BG_STYLE));
    {
        static const Str names[bg::BG_COUNT] = { S_BG_AUTO, S_BG_AURORA, S_BG_WAVES, S_BG_WARP, S_BG_GRID, S_BG_CONST };
        float pg = S(8), pw = (iw - pg * 2) / 3;
        for (int k = 0; k < bg::BG_COUNT; k++)
        {
            snprintf(buf, sizeof(buf), "%s##bgs%d", T(names[k]), k);
            if (StylePill(buf, config::c.bgStyle == k, ImVec2(pw, S(34)))) { config::c.bgStyle = k; config::MarkDirty(); }
            if (k % 3 != 2) ImGui::SameLine(0, pg);
        }
    }
    ImGui::Dummy(ImVec2(0, S(2)));
    if (ui::Slider(T(S_GLASS), &config::c.glass, 0.35f, 1.0f, "%.0f%%", iw, 100.0f)) config::MarkDirty();
    if (ui::Slider(T(S_ANIM_SPEED), &config::c.animSpeed, 0.5f, 2.0f, "%.1fx", iw)) config::MarkDirty();
    float ly = ui::CardEnd() + gap;

    // ---------------------------------------------------------- gateway
    ui::CardBegin(T(S_GATEWAY), Ico_Signal, ImVec2(lx, ly), cw);
    iw = ui::CardInnerWidth();
    {
        ImDrawList* sdl = ImGui::GetWindowDrawList();
        bool running = g.gwStatus.is_object() && g.gwStatus.value("running", false);
        bool managed = g.gwStatus.is_object() && g.gwStatus.value("managed", false);
        ImVec2 p = ImGui::GetCursorScreenPos();
        ImVec4 c = running ? (managed ? theme::success : theme::warning) : theme::textMute;
        sdl->AddCircleFilled(ImVec2(p.x + S(5), p.y + S(9)), S(4), Col(c), 12);
        std::string st = running ? (managed ? T(S_RUNNING) : T(S_GATEWAY_EXTERNAL)) : T(S_STOPPED);
        ImGui::SetCursorScreenPos(ImVec2(p.x + S(18), p.y));
        ImGui::TextUnformatted(st.c_str());
        ImGui::Dummy(ImVec2(0, S(4)));
    }
    Label(T(S_BIND));
    {
        ImGui::SetNextItemWidth(iw);
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        if (ImGui::InputTextWithHint("##gwbind", "127.0.0.1:8713", &config::c.gwBind))
            config::MarkDirty();
        ImGui::PopStyleVar();
    }
    ImGui::Dummy(ImVec2(0, S(8)));
    Label(T(S_BEARER));
    {
        static bool showTok = false;
        ImGui::SetNextItemWidth(iw - S(44));
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        ImGuiInputTextFlags fl = showTok ? ImGuiInputTextFlags_None : ImGuiInputTextFlags_Password;
        if (ImGui::InputTextWithHint("##gwtoken", "—", &config::c.gwToken, fl))
            config::MarkDirty();
        ImGui::PopStyleVar();
        ImGui::SameLine(0, S(8));
        if (ui::IconButton("##gwtokeneye", showTok ? Ico_EyeOff : Ico_Eye, S(34))) showTok = !showTok;
    }
    ImGui::Dummy(ImVec2(0, S(4)));
    if (ui::Toggle(T(S_WATCH), T(S_WATCH_D), &config::c.gwWatch, iw)) config::MarkDirty();
    {
        float ms = (float)config::c.gwWatchMs;
        if (ui::Slider(T(S_REFRESH), &ms, 100.0f, 5000.0f, "%.0f ms", iw))
        {
            config::c.gwWatchMs = (int)ms;
            config::MarkDirty();
        }
    }
    Label(T(S_PROFILE));
    {
        std::vector<std::string> profs;
        if (reg::registry.contains("profiles") && reg::registry["profiles"].is_object())
            for (auto it = reg::registry["profiles"].begin(); it != reg::registry["profiles"].end(); ++it)
                profs.push_back(it.key());
        float pg = S(8), pw = (iw - pg * 4) / 5;
        char lbl0[64];
        snprintf(lbl0, sizeof(lbl0), "%s##profnone", T(S_ALL_SERVERS));
        if (StylePill(lbl0, config::c.gwProfile.empty(), ImVec2(pw, S(32))))
        {
            config::c.gwProfile.clear();
            config::MarkDirty();
        }
        int shown = 1;
        for (size_t i = 0; i < profs.size() && shown < 5; i++, shown++)
        {
            ImGui::SameLine(0, pg);
            char lbl[80];
            snprintf(lbl, sizeof(lbl), "%s##prof%d", profs[i].c_str(), (int)i);
            if (StylePill(lbl, config::c.gwProfile == profs[i], ImVec2(pw, S(32))))
            {
                config::c.gwProfile = profs[i];
                config::MarkDirty();
            }
        }
    }
    ImGui::Dummy(ImVec2(0, S(6)));
    {
        float tf = (float)config::c.timeout;
        if (ui::Slider(T(S_TIMEOUT), &tf, 3.0f, 300.0f, "%.0f s", iw))
        {
            config::c.timeout = (int)(tf + 0.5f);
            config::MarkDirty();
        }
    }
    ImGui::Dummy(ImVec2(0, S(6)));
    {
        bool running = g.gwStatus.is_object() && g.gwStatus.value("running", false);
        float bw2 = (iw - S(10)) * 0.5f;
        char s1[48], s2[48];
        snprintf(s1, sizeof(s1), "%s##setgwstart", T(S_START));
        snprintf(s2, sizeof(s2), "%s##setgwstop", T(S_STOP));
        if (ui::Button(s1, ImVec2(bw2, S(40)), ui::Btn_Primary, Ico_Play) && !running) StartGateway();
        ImGui::SameLine(0, S(10));
        if (ui::Button(s2, ImVec2(bw2, S(40)), ui::Btn_Secondary, Ico_Close) && running) StopGateway();
    }
    ly = ui::CardEnd() + gap;

    // ---------------------------------------------------------- account
    ui::CardBegin(T(S_ACCOUNT), Ico_User, ImVec2(lx, ly), cw);
    iw = ui::CardInnerWidth();
    Label(T(S_YOUR_NAME));
    {
        static char nameBuf[40] = {};
        static bool init = false;
        if (!init) { init = true; snprintf(nameBuf, sizeof(nameBuf), "%s", config::c.userName.c_str()); }
        ImGui::SetNextItemWidth(iw);
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        if (ImGui::InputText("##username", nameBuf, sizeof(nameBuf)))
        {
            config::c.userName = nameBuf;
            g.user = nameBuf;
            config::MarkDirty();
        }
        ImGui::PopStyleVar();
    }
    ImGui::Dummy(ImVec2(0, S(8)));
    InfoRow("##cphw", T(S_HWID), g.hwid.c_str(), g.hwid.c_str(), iw);
    ImGui::Dummy(ImVec2(0, S(4)));
    {
        std::string rp = reg::registryPath;
        std::string shortp = rp;
        if (shortp.size() > 44) shortp = "…" + shortp.substr(shortp.size() - 43);
        InfoRow("##cpreg", T(S_REG_PATH), shortp.c_str(), rp.c_str(), iw);
    }
    ImGui::Dummy(ImVec2(0, S(4)));
    snprintf(buf, sizeof(buf), "%s##wizard2", T(S_RESTART_WIZARD));
    if (ui::Button(buf, ImVec2(iw, S(42)), ui::Btn_Secondary, Ico_Refresh)) RestartWizard();
    float leftBottom = ui::CardEnd();

    // ---------------------------------------------------------- general
    ui::CardBegin(T(S_GENERAL), Ico_Gear, ImVec2(rx, ty), cw);
    iw = ui::CardInnerWidth();
    Label(T(S_LANGUAGE));
    {
        int picked = ui::LanguagePicker("##setlang", iw, false);
        if (picked >= 0) SetLanguage(picked);
    }
    ImGui::Dummy(ImVec2(0, S(4)));
    if (ui::Toggle(T(S_NOTIFICATIONS), T(S_NOTIFICATIONS_D), &config::c.notifications, iw)) config::MarkDirty();
    if (ui::Toggle(T(S_TOPMOST), T(S_TOPMOST_D), &config::c.alwaysOnTop, iw))
    {
        platform::SetTopmost(config::c.alwaysOnTop);
        config::MarkDirty();
    }
    if (ui::Toggle(T(S_STREAMPROOF), T(S_STREAMPROOF_D), &config::c.streamProof, iw))
    {
        platform::SetStreamProof(config::c.streamProof);
        config::MarkDirty();
    }
    if (ui::Toggle(T(S_AUTOCLOSE), T(S_AUTOCLOSE_D), &config::c.autoClose, iw)) config::MarkDirty();
    float ry = ui::CardEnd() + gap;

    // ---------------------------------------------------------- discord
    ui::CardBegin(T(S_DISCORD_RPC), Ico_Bell, ImVec2(rx, ry), cw);
    iw = ui::CardInnerWidth();
    if (ui::Toggle(T(S_DISCORD_RPC), nullptr, &config::c.discordRpc, iw)) config::MarkDirty();
    ImGui::Dummy(ImVec2(0, S(6)));
    Label(T(S_DISCORD_ID));
    {
        static char idBuf[40] = {};
        static bool dinit = false;
        if (!dinit) { dinit = true; snprintf(idBuf, sizeof(idBuf), "%s", config::c.discordId.c_str()); }
        ImGui::SetNextItemWidth(iw);
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(S(10), S(8)));
        if (ImGui::InputTextWithHint("##disid", T(S_DISCORD_ID_D), idBuf, sizeof(idBuf)))
        {
            config::c.discordId = idBuf;
            config::MarkDirty();
        }
        ImGui::PopStyleVar();
    }
    ry = ui::CardEnd() + gap;

    // ---------------------------------------------------------- about
    ui::CardBegin(T(S_ABOUT), Ico_Info, ImVec2(rx, ry), cw);
    iw = ui::CardInnerWidth();
    {
        ImDrawList* dl = ImGui::GetWindowDrawList();
        ImVec2 p = ImGui::GetCursorScreenPos();
        Logo(dl, p + S2(20, 22), S(24));
        Text(dl, fonts::bold, ImVec2(p.x + S(46), p.y + S(4)), Col(theme::text), "CerberusMCP GUI");
        snprintf(buf, sizeof(buf), "%s  ·  x64", g.version.c_str());
        Text(dl, fonts::small, ImVec2(p.x + S(46), p.y + S(25)), Col(theme::textMute), buf);
        ImGui::Dummy(ImVec2(iw, S(46)));
#if defined(_WIN32)
        const char* renderer = "Dear ImGui " IMGUI_VERSION "  ·  DirectX 11";
#else
        const char* renderer = "Dear ImGui " IMGUI_VERSION "  ·  OpenGL 3";
#endif
        Text(dl, fonts::small, ImGui::GetCursorScreenPos(), Col(theme::textMute), renderer);
        ImGui::Dummy(ImVec2(iw, fonts::small->FontSize + S(4)));
    }
    snprintf(buf, sizeof(buf), "%s##reset", T(S_RESET));
    if (ui::Button(buf, ImVec2(iw, S(42)), ui::Btn_Secondary, Ico_Refresh))
    {
        config::ResetVisuals();
        platform::SetTopmost(config::c.alwaysOnTop);
        platform::SetStreamProof(config::c.streamProof);
        toast::Push(toast::Success, T(S_RESET_DONE));
    }
    float rightBottom = ui::CardEnd();

    ImGui::SetCursorScreenPos(ImVec2(o.x, ImMax(leftBottom, rightBottom) + S(18)));
    ImGui::Dummy(ImVec2(1, 1));
    ImGui::EndChild();
    ImGui::PopStyleVar();
}

// ================================================================== logs
void LogsPage(ImVec2 a, ImVec2 b)
{
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float x = a.x + S(30), y = a.y + S(22), w = b.x - a.x - S(60);
    PageHeader(ImVec2(x, y), w, T(S_NAV_LOGS), T(S_LOGS_SUB));

    // toolbar
    {
        float bs = S(34), bx = x + w - bs, by = y + S(8);
        ImGui::SetCursorScreenPos(ImVec2(bx, by));
        if (ui::IconButton("##logclear", Ico_Close, bs, T(S_CLEAR))) g.logLines.clear();
        bx -= bs + S(8);
        ImGui::SetCursorScreenPos(ImVec2(bx, by));
        if (ui::IconButton("##logpause", g.logsPaused ? Ico_Play : Ico_Minimize, bs,
                           g.logsPaused ? T(S_START) : T(S_PAUSE)))
            g.logsPaused = !g.logsPaused;
        bx -= S(160) + S(8);
        ImGui::SetCursorScreenPos(ImVec2(bx, by + S(2)));
        char al[48];
        snprintf(al, sizeof(al), "%s##logauto", T(S_AUTOSCROLL));
        if (ui::Button(al, ImVec2(S(160), S(30)), g.logsAuto ? ui::Btn_Secondary : ui::Btn_Ghost))
            g.logsAuto = !g.logsAuto;
    }

    float top = y + S(72);
    ImGui::SetCursorScreenPos(ImVec2(x - S(12), top));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(S(16), S(10)));
    ImGui::BeginChild("##loglines", ImVec2(w + S(24), b.y - top - S(10)), ImGuiChildFlags_None,
                      ImGuiWindowFlags_AlwaysVerticalScrollbar);
    if (g.logLines.empty())
    {
        Text(ImGui::GetWindowDrawList(), fonts::body, ImGui::GetCursorScreenPos(), Col(theme::textMute), T(S_NO_LOGS));
        ImGui::Dummy(ImVec2(0, fonts::body->FontSize + S(8)));
    }
    else
    {
        for (const LogLine& L : g.logLines)
        {
            ImVec4 c = theme::textDim;
            if (L.text.find("ERROR") != std::string::npos) c = theme::danger;
            else if (L.text.find("WARN") != std::string::npos) c = theme::warning;
            else if (L.text.find("DEBUG") != std::string::npos) c = theme::textMute;
            Text(ImGui::GetWindowDrawList(), fonts::mono, ImGui::GetCursorScreenPos(), Col(c), L.text.c_str(), S(13));
            ImGui::Dummy(ImVec2(0, fonts::mono->FontSize + S(3)));
        }
    }
    if (g.logsAuto && ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - S(24))
        ImGui::SetScrollHereY(1.0f);
    ImGui::EndChild();
    ImGui::PopStyleVar();

    if (g.logsPaused)
    {
        ImVec2 ps = Measure(fonts::smallBold, T(S_PAUSED));
        ImGui::SetCursorScreenPos(ImVec2(x + w - S(160) - ps.x - S(16), y + S(18)));
        ImGui::Dummy(ImVec2(0, 0));
        Text(ImGui::GetWindowDrawList(), fonts::smallBold,
             ImVec2(x + w - S(160) - ps.x - S(16), y + S(18)), Col(theme::warning), T(S_PAUSED));
    }
}

void Main(ImVec2 ds)
{
    Sidebar(ds);
    g.pageT = ImMin(1.0f, g.pageT + anim::Dt() * 2.4f);
    float pt = anim::OutCubic(g.pageT);
    ImVec2 off(0, (1.0f - pt) * S(16));
    ImVec2 a(S(kSideW), S(kTitleH)), b = ds;
    ImGui::PushStyleVar(ImGuiStyleVar_Alpha, ImGui::GetStyle().Alpha * pt);
    switch (g.page)
    {
    case PG_HOME:     Home(a + off, b + off); break;
    case PG_MCP:      Mcps(a + off, b + off); break;
    case PG_LOGS:     LogsPage(a + off, b + off); break;
    case PG_SETTINGS: Settings(a + off, b + off); break;
    }
    ImGui::PopStyleVar();
}

// ================================================================== loader overlay
void OnLoadDone()
{
    ld.spawnConfetti = true;
    config::c.totalLoads++;
    config::MarkDirty();
    toast::Push(toast::Success, T(S_LOAD_DONE), ld.serverName.c_str());
}

void LoaderFail(const char* msg)
{
    ld.failed = true;
    ld.error = msg ? msg : "";
    if (ld.job)
    {
        core::JobFree(ld.job);
        ld.job = 0;
    }
    ld.jobKind = 0;
    toast::Push(toast::Error, T(S_LOAD_FAILED), ld.serverName.c_str(), 6.0f);
}

void LoaderTick(float rdt)
{
    if (ld.done || ld.closing || ld.failed) return;
    ld.stageTime += rdt;

    switch (ld.stage)
    {
    case 0:   // registry
    {
        std::string err;
        if (!reg::Refresh(&err)) { LoaderFail(err.c_str()); return; }
        ld.opDone = true;
        break;
    }
    case 1:   // validate + target present & enabled
    {
        json v = core::RegistryValidate();
        if (!v.is_object()) { LoaderFail(core::LastError().c_str()); return; }
        if (!v.value("ok", false)) { LoaderFail(v.value("error", "invalid registry").c_str()); return; }
        int idx = reg::IndexOf(ld.serverName);
        if (idx < 0) { LoaderFail(T(S_SERVER_MISSING)); return; }
        if (!reg::servers[idx].enabled) { LoaderFail(T(S_SERVER_DISABLED)); return; }
        ld.opDone = true;
        break;
    }
    case 2:   // doctor job
    {
        if (ld.jobKind == 0)
        {
            ld.job = core::DoctorStart(false);
            if (!ld.job) { LoaderFail(core::LastError().c_str()); return; }
            ld.jobKind = 1;
        }
        json p = core::JobPoll(ld.job);
        std::string st = p.value("state", "pending");
        if (st == "done")
        {
            ld.opInfo = p.value("value", json());
            core::JobFree(ld.job);
            ld.job = 0;
            ld.jobKind = 0;
            ld.opDone = true;
        }
        else if (st == "error")
        {
            std::string e = p.value("error", "doctor failed");
            core::JobFree(ld.job);
            ld.job = 0;
            ld.jobKind = 0;
            LoaderFail(e.c_str());
            return;
        }
        break;
    }
    case 3:   // gateway start (or reuse a running one)
    {
        if (!ld.opDone)
        {
            json st = core::GatewayStatus();
            if (st.is_object() && st.value("running", false))
            {
                ld.opInfo = st;
                ld.opDone = true;
                break;
            }
            if (ld.jobKind == 0)
            {
                ld.job = core::GatewayStart(GatewayOpts());
                if (!ld.job) { LoaderFail(core::LastError().c_str()); return; }
                ld.jobKind = 2;
            }
            json p = core::JobPoll(ld.job);
            std::string s = p.value("state", "pending");
            if (s == "done")
            {
                ld.opInfo = p.contains("value") ? p["value"] : core::GatewayStatus();
                core::JobFree(ld.job);
                ld.job = 0;
                ld.jobKind = 0;
                ld.opDone = true;
            }
            else if (s == "error")
            {
                std::string e = p.value("error", "gateway start failed");
                core::JobFree(ld.job);
                ld.job = 0;
                ld.jobKind = 0;
                LoaderFail(e.c_str());
                return;
            }
        }
        break;
    }
    case 4:   // status verify
    {
        if (!ld.opDone)
        {
            json st = core::GatewayStatus();
            if (!st.is_object()) { LoaderFail(core::LastError().c_str()); return; }
            if (!st.value("running", false))
            {
                std::string ev = st.value("last_event", std::string());
                LoaderFail(ev.empty() ? T(S_LOAD_FAILED) : ev.c_str());
                return;
            }
            ld.opInfo = st;
            ld.opDone = true;
        }
        break;
    }
    case 5:
        ld.opDone = true;
        break;
    }

    if (ld.opDone && ld.stageTime >= kStageDwell)
    {
        ld.stageTime = 0.0f;
        ld.opDone = false;
        if (++ld.stage >= 6)
        {
            ld.stage = 6;
            ld.done = true;
            ld.doneT = 0.0f;
            OnLoadDone();
        }
    }
}

const char* StageDetail(char* buf, size_t n)
{
    switch (ld.stage)
    {
    case 0:
    {
        std::string base = reg::registryPath;
        size_t sep = base.find_last_of("/\\");
        if (sep != std::string::npos) base = base.substr(sep + 1);
        snprintf(buf, n, "%s", base.empty() ? "registry.json" : base.c_str());
        break;
    }
    case 1:
        snprintf(buf, n, "%s  ·  schema v%u", ld.serverName.c_str(),
                 reg::registry.value("schema_version", 0u));
        break;
    case 2:
    {
        if (ld.opInfo.is_object())
        {
            size_t dcount = ld.opInfo.contains("diagnostics") && ld.opInfo["diagnostics"].is_array()
                                ? ld.opInfo["diagnostics"].size()
                                : 0;
            int w = (int)ld.opInfo.value("warnings", 0), c = (int)ld.opInfo.value("critical", 0);
            snprintf(buf, n, "%zu %s · %d %s · %d %s", dcount, T(S_DIAGNOSTICS), w, T(S_K_WARN), c, T(S_K_CRIT));
        }
        else snprintf(buf, n, "%s…", T(S_DIAGNOSTICS));
        break;
    }
    case 3:
    {
        std::string bind = ld.opInfo.is_object() ? ld.opInfo.value("bind", std::string()) : std::string();
        if (!bind.empty()) snprintf(buf, n, "%s", bind.c_str());
        else               snprintf(buf, n, "%s  ·  %s", config::c.gwBind.c_str(), T(S_START));
        break;
    }
    case 4:
    {
        std::string bind = ld.opInfo.is_object() ? ld.opInfo.value("bind", std::string()) : std::string();
        if (bind.empty()) bind = config::c.gwBind;
        if (config::c.gwWatch)
            snprintf(buf, n, "%s  ·  watch %dms", bind.c_str(), config::c.gwWatchMs);
        else
            snprintf(buf, n, "%s", bind.c_str());
        break;
    }
    case 5:
        snprintf(buf, n, "session  %s", ld.session);
        break;
    default:
        snprintf(buf, n, "%s", T(S_LOAD_DONE_SUB));
        break;
    }
    return buf;
}

void LoaderOverlay(ImVec2 ds)
{
    if (!ld.open) return;
    float rdt = RealDt();
    ld.openT += ((ld.closing ? 0.0f : 1.0f) - ld.openT) * (1.0f - expf(-11.0f * rdt));
    if (ld.closing && ld.openT < 0.02f) { ld.open = false; return; }

    if (!ld.done && !ld.closing) LoaderTick(rdt);
    if (ld.done) ld.doneT += rdt;

    float st = ld.stage < 6 ? ImSaturate(ld.stageTime / kStageDwell) : 1.0f;
    float prog = 0.0f;
    for (int k = 0; k < ImMin(ld.stage, 6); k++) prog += kStageWeight[k];
    if (ld.stage < 6) prog += kStageWeight[ld.stage] * (ld.failed ? 0.5f : st);
    ld.shown += (prog - ld.shown) * (1.0f - expf(-7.0f * rdt));
    if (ld.done) ld.shown = ImMin(1.0f, ld.shown + rdt * 0.8f);

    ImGui::SetNextWindowPos(ImVec2(0, S(kTitleH)));
    ImGui::SetNextWindowSize(ImVec2(ds.x, ds.y - S(kTitleH)));
    if (ld.justOpened) { ImGui::SetNextWindowFocus(); ld.justOpened = false; }
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
    ImGui::Begin("##loader", nullptr, ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings |
                                      ImGuiWindowFlags_NoBackground | ImGuiWindowFlags_NoScrollWithMouse);
    ImGui::PopStyleVar();
    ImDrawList* dl = ImGui::GetWindowDrawList();
    float o = anim::OutCubic(ld.openT), t = anim::Time();
    ImVec2 wa = ImGui::GetWindowPos(), wb = wa + ImGui::GetWindowSize();
    dl->AddRectFilled(wa, wb, Col(Black(0.48f * o)));

    ImGui::PushStyleVar(ImGuiStyleVar_Alpha, o);
    reg::ServerView sview;
    int sidx = reg::IndexOf(ld.serverName);
    if (sidx >= 0) sview = reg::servers[sidx];
    else
    {
        sview.name = ld.serverName;
        sview.c0 = theme::accent;
        sview.c1 = theme::accent2;
    }
    float dk = ld.done ? anim::OutCubic(ld.doneT * 2.5f) : 0.0f;
    ImVec4 ringA = ld.failed ? theme::danger
                              : Mix(theme::accent, theme::success, dk);
    ImVec4 ringB = ld.failed ? Mix(theme::danger, White(), 0.3f)
                             : Mix(theme::accent2, Mix(theme::success, White(), 0.4f), dk);

    ImVec2 csz = S2(468, 500), cc = (wa + wb) * 0.5f;
    ImVec2 ca = cc - csz * 0.5f + ImVec2(0, (1.0f - o) * S(30)), cb = ca + csz;
    Glow(dl, cc, S(400), Col(ringA, 0.16f));
    Shadow(dl, ca, cb, S(22), S(30), Col(Black(0.5f)), ImVec2(0, S(10)));
    Gradient(dl, ca, cb, Col(ImVec4(0.995f, 0.995f, 1.0f, 0.99f)), Col(ImVec4(0.960f, 0.963f, 0.972f, 0.99f)), S(22), true);
    GradientBorder(dl, ca, cb, S(22), Col(ringA, 0.50f), Col(theme::Ink(0.06f)), S(1.2f));

    // header
    float es = S(42);
    ImVec2 ea(ca.x + S(24), ca.y + S(22));
    ServerArt(dl, sview, ea, ea + ImVec2(es, es), S(12));
    Text(dl, fonts::bold, ImVec2(ea.x + es + S(14), ea.y + S(2)), Col(theme::text), sview.name.c_str());
    char sub[64];
    if (ld.done) snprintf(sub, sizeof(sub), "%s", T(S_LOAD_DONE));
    else snprintf(sub, sizeof(sub), "%s%s", T(S_LOADING), ((int)(t * 2.5f) % 4) == 0 ? "" : ((int)(t * 2.5f) % 4) == 1 ? "." : ((int)(t * 2.5f) % 4) == 2 ? ".." : "...");
    Text(dl, fonts::small, ImVec2(ea.x + es + S(14), ea.y + S(23)), Col(ld.done ? theme::success : theme::textDim), sub);
    char sess[32];
    snprintf(sess, sizeof(sess), "#%s", ld.session);
    ImVec2 ss = Measure(fonts::mono, sess, S(12));
    Text(dl, fonts::mono, ImVec2(cb.x - S(24) - ss.x, ea.y + S(14)), Col(theme::textMute), sess, S(12));
    dl->AddLine(ImVec2(ca.x + S(24), ca.y + S(84)), ImVec2(cb.x - S(24), ca.y + S(84)), Col(theme::Ink(0.07f)));

    // progress ring
    ImVec2 rc(cc.x, ca.y + S(186));
    float R = S(70), th = S(9);
    for (int k = 0; k < 60; k++)
    {
        float ang = k * IM_PI * 2.0f / 60.0f + t * 0.12f;
        bool major = k % 5 == 0;
        float lit = fmodf((float)k / 60.0f - t * 0.22f + 10.0f, 1.0f);
        float al = 0.06f + (lit < 0.18f ? 0.30f * (1.0f - lit / 0.18f) : 0.0f) + (major ? 0.05f : 0.0f);
        ImVec2 d(cosf(ang), sinf(ang));
        dl->AddLine(rc + d * (R + S(15)), rc + d * (R + S(major ? 22.0f : 18.0f)), Col(ringB, al), S(1.2f));
    }
    dl->AddCircle(rc, R, Col(theme::Ink(0.07f)), 96, th);
    float a0 = -IM_PI * 0.5f, a1 = a0 + IM_PI * 2.0f * ld.shown;
    Arc(dl, rc, R, a0, a1, th, Col(ringA), Col(ringB));
    if (ld.shown > 0.01f && !ld.done)
    {
        ImVec2 hp = rc + ImVec2(cosf(a1), sinf(a1)) * R;
        Glow(dl, hp, S(28), Col(ringB, 0.85f));
        dl->AddCircleFilled(hp, S(3.5f), Col(White()), 16);
    }
    Glow(dl, rc, R * 0.95f, Col(ringA, 0.10f + 0.06f * sinf(t * 3.0f)));
    dl->AddCircle(rc, R - S(17), Col(theme::Ink(0.05f)), 64, S(1));

    if (!ld.done && !ld.failed)
    {
        for (int k = 0; k < 3; k++)
        {
            float ang = t * 2.4f + k * 2.094f;
            dl->AddCircleFilled(rc + ImVec2(cosf(ang), sinf(ang)) * (R - S(17)), S(2.2f), Col(ringB, 0.85f), 10);
        }
        char pct[8];
        snprintf(pct, sizeof(pct), "%d", (int)(ld.shown * 100.0f + 0.5f));
        ImVec2 ps = Measure(fonts::display, pct, S(44));
        ImVec2 pp(rc.x - ps.x * 0.5f - S(7), rc.y - ps.y * 0.5f - S(6));
        TextGradient(dl, fonts::display, pp, Col(theme::text), Col(theme::accent2), pct, S(44));
        Text(dl, fonts::bold, ImVec2(pp.x + ps.x + S(3), pp.y + S(8)), Col(theme::textDim), "%");
        char sn[16];
        snprintf(sn, sizeof(sn), "%d / 6", ImMin(ld.stage + 1, 6));
        TextC(dl, fonts::small, ImVec2(rc.x, rc.y + S(26)), Col(theme::textMute), sn);
    }
    else if (ld.failed)
    {
        float k = anim::OutBack(ld.doneT * 2.2f + 0.6f);
        dl->AddCircleFilled(rc, ImMax(0.0f, (R - S(22)) * ImClamp(k, 0.0f, 1.0f)), Col(theme::danger, 0.14f), 64);
        float arm = S(18) * ImClamp(k, 0.0f, 1.0f);
        dl->AddLine(rc - ImVec2(arm, arm), rc + ImVec2(arm, arm), Col(theme::danger), S(6));
        dl->AddLine(rc + ImVec2(arm, -arm), rc - ImVec2(arm, -arm), Col(theme::danger), S(6));
    }
    else
    {
        float k = anim::OutBack(ld.doneT * 2.2f);
        dl->AddCircleFilled(rc, ImMax(0.0f, (R - S(22)) * k), Col(theme::success, 0.16f), 64);
        float rp = fmodf(ld.doneT, 1.6f) / 1.6f;
        dl->AddCircle(rc, R + rp * S(44), Col(theme::success, 0.45f * (1.0f - rp)), 96, S(2));
        Check(dl, rc, S(56) * ImMax(0.0f, k), Col(theme::success), S(6), anim::OutCubic(ld.doneT * 1.8f - 0.15f));
    }

    // status
    float sy = rc.y + R + S(30);
    char det[192];
    if (ld.failed)
    {
        TextC(dl, fonts::title, ImVec2(cc.x, sy), Col(theme::danger), T(S_LOAD_FAILED));
        std::string err = ld.error;
        if (err.size() > 56) err = err.substr(0, 55) + "…";
        TextC(dl, fonts::body, ImVec2(cc.x, sy + S(26)), Col(theme::textDim), err.c_str());
    }
    else
    {
        TextC(dl, fonts::title, ImVec2(cc.x, sy), Col(ld.done ? theme::success : theme::text),
              ld.done ? T(S_LOAD_DONE) : T(kStageStr[ld.stage]));
        StageDetail(det, sizeof(det));
        if (ld.done) TextC(dl, fonts::body, ImVec2(cc.x, sy + S(26)), Col(theme::textDim), det);
        else         TextC(dl, fonts::mono, ImVec2(cc.x, sy + S(26)), Col(theme::textDim), det, S(13));
    }

    // stage checklist (2 x 3)
    float gy = sy + S(58), colW = (csz.x - S(48)) * 0.5f;
    for (int k = 0; k < 6; k++)
    {
        ImVec2 sp(ca.x + S(28) + (k % 2) * colW, gy + (k / 2) * S(26));
        ImVec2 ic(sp.x + S(8), sp.y + S(9));
        bool isDone = k < ld.stage, isActive = k == ld.stage && !ld.done && !ld.failed;
        bool isFail = ld.failed && k == ld.stage;
        if (isDone)
        {
            dl->AddCircleFilled(ic, S(8), Col(theme::success, 0.18f), 20);
            Check(dl, ic, S(9), Col(theme::success), S(1.8f), 1.0f);
        }
        else if (isFail)
        {
            dl->AddCircleFilled(ic, S(8), Col(theme::danger, 0.18f), 20);
            dl->AddLine(ic - ImVec2(S(3.5f), S(3.5f)), ic + ImVec2(S(3.5f), S(3.5f)), Col(theme::danger), S(1.8f));
            dl->AddLine(ic + ImVec2(S(3.5f), -S(3.5f)), ic - ImVec2(S(3.5f), -S(3.5f)), Col(theme::danger), S(1.8f));
        }
        else if (isActive)
        {
            dl->AddCircle(ic, S(7.5f), Col(theme::Ink(0.10f)), 24, S(1.6f));
            Spinner(dl, ic, S(7.5f), S(1.8f), Col(theme::accent2));
        }
        else dl->AddCircle(ic, S(7), Col(theme::Ink(0.13f)), 24, S(1.4f));
        Text(dl, fonts::small, ImVec2(sp.x + S(24), sp.y + S(1)),
             Col(isFail ? theme::danger : isActive ? theme::text : isDone ? theme::textDim : theme::textMute), T(kStageStr[k]));
    }

    // action button
    ImVec2 bp(ca.x + S(24), cb.y - S(24) - S(46)), bsz(csz.x - S(48), S(46));
    ImGui::SetCursorScreenPos(bp);
    char lbl[64];
    if (ld.failed)
    {
        snprintf(lbl, sizeof(lbl), "%s##ldclose", T(S_CLOSE_APP));
        if (ui::Button(lbl, bsz, ui::Btn_Danger, Ico_Close)) ld.closing = true;
    }
    else if (!ld.done)
    {
        snprintf(lbl, sizeof(lbl), "%s##ldcancel", T(S_CANCEL));
        if (ui::Button(lbl, bsz, ui::Btn_Ghost, Ico_Close) && !ld.closing)
        {
            ld.closing = true;
            if (ld.job)
            {
                if (ld.jobKind == 2) g.orphanStopJob = core::GatewayStop();
                core::JobFree(ld.job);
                ld.job = 0;
                ld.jobKind = 0;
            }
            toast::Push(toast::Warning, T(S_LOAD_CANCELLED), sview.name.c_str());
        }
    }
    else
    {
        snprintf(lbl, sizeof(lbl), "%s##ldlaunch", T(S_DONE));
        if (ui::Button(lbl, bsz, ui::Btn_Primary, Ico_Check)) ld.closing = true;
        if (config::c.autoClose && !ld.closing)
        {
            ld.closeTimer += rdt;
            TextC(dl, fonts::small, ImVec2(cc.x, bp.y - S(12)), Col(theme::textMute), T(S_CLOSING_IN));
            if (ld.closeTimer > 2.6f) { ld.closing = true; platform::Close(); }
        }
    }

    // confetti
    if (ld.spawnConfetti)
    {
        ld.spawnConfetti = false;
        const ImVec4 cols[5] = { theme::accent, theme::accent2, theme::success, theme::textDim, theme::warning };
        for (int k = 0; k < 90; k++)
        {
            float ang = -IM_PI * 0.5f + (Rand01() - 0.5f) * IM_PI * 1.7f, spd = S(200.0f + Rand01() * 340.0f);
            ld.confetti.push_back({ rc, ImVec2(cosf(ang), sinf(ang)) * spd, Rand01() * 6.28f, (Rand01() - 0.5f) * 16.0f, 0.0f,
                                    S(3.0f + Rand01() * 3.5f), cols[k % 5] });
        }
    }
    for (size_t k = 0; k < ld.confetti.size();)
    {
        Confetti& c = ld.confetti[k];
        c.life += rdt;
        if (c.life > 2.4f) { ld.confetti.erase(ld.confetti.begin() + k); continue; }
        c.v.y += S(560) * rdt;
        c.v = c.v * (1.0f - 1.3f * rdt);
        c.p += c.v * rdt;
        c.rot += c.vr * rdt;
        float al = 1.0f - ImSaturate((c.life - 1.4f) / 1.0f);
        float cs = cosf(c.rot), sn = sinf(c.rot), hw = c.size, hh = c.size * 0.5f * (0.6f + 0.4f * fabsf(sinf(c.rot * 1.7f)));
        ImVec2 ax(cs * hw, sn * hw), ay(-sn * hh, cs * hh);
        dl->AddQuadFilled(c.p - ax - ay, c.p + ax - ay, c.p + ax + ay, c.p - ax + ay, Col(c.c, al));
        k++;
    }

    ImGui::PopStyleVar();
    ImGui::End();
}

// ================================================================== screen transition
void Transition(ImVec2 ds)
{
    if (!g.trans) return;
    g.transT += ImMin(ImGui::GetIO().DeltaTime, 0.05f) / 1.15f;
    ImDrawList* dl = ImGui::GetForegroundDrawList();
    float maxR = Len(ds) * 1.1f;
    float& alpha = ImGui::GetStyle().Alpha;
    float saved = alpha;
    if (g.transT < 0.5f)
    {
        float k = anim::InOutCubic(g.transT / 0.5f);
        float r = maxR * k;
        dl->AddCircleFilled(g.transOrigin, r, Col(theme::bg0), 128);
        Glow(dl, g.transOrigin, r * 0.7f, Col(theme::accent, 0.22f * k));
        dl->AddCircle(g.transOrigin, r, Col(theme::accent2, 0.9f * (1.0f - k * 0.6f)), 128, S(2.5f));
        alpha = ImSaturate(k * 1.6f - 0.6f);
        Logo(dl, ds * 0.5f, S(50), ImSaturate(k * 1.6f - 0.6f));
    }
    else
    {
        if (!g.transSwitched)
        {
            g.transSwitched = true;
            SwitchScreen(g.transTarget);
        }
        float k = 1.0f - anim::OutCubic((g.transT - 0.5f) / 0.5f);
        dl->AddRectFilled(ImVec2(0, 0), ds, Col(theme::bg0, k));
        alpha = k;
        Logo(dl, ds * 0.5f, S(50) + (1.0f - k) * S(12), k);
    }
    alpha = saved;
    if (g.transT >= 1.0f) g.trans = false;
}

} // namespace

// ================================================================== public
void Init()
{
    config::Load();
    lang::Init();
    lang::current = (config::c.language >= 0 && config::c.language < LANG_COUNT) ? config::c.language : LANG_TR;

    gfx::dpi = platform::DpiScale();
    fonts::Load(gfx::dpi);
    theme::ApplyStyle(gfx::dpi);
    theme::TargetAccent(config::c.accent, theme::accent, theme::accent2);
    flags::Build();
    g.logo = platform::LoadTextureWhiteKey(platform::FindAsset("assets/logo.png"), nullptr, nullptr);
    rpc::Init();

    // core bridge: paths + version, then the registry and first status sweep
    json ci = core::Init();
    if (ci.is_object())
    {
        std::string ver = ci.value("version", std::string("?"));
        g.version = "v" + ver;
        reg::registryPath = ci.value("registryPath", std::string());
    }
    std::string regErr;
    if (!reg::Refresh(&regErr) && !regErr.empty())
        toast::Push(toast::Error, T(S_REG_PATH), regErr.c_str(), 6.0f);
    g.gwStatus = core::GatewayStatus();
    StartDoctor();

    g.user = config::c.userName.empty() ? platform::UserName() : config::c.userName;
    g.screen = config::c.onboarded ? SCR_MAIN : SCR_SPLASH;
    g.splashT = 0.0f;
    g.hwid = platform::HardwareId();

    platform::SetTopmost(config::c.alwaysOnTop);
    platform::SetStreamProof(config::c.streamProof);
}

void Frame()
{
    ImGuiIO& io = ImGui::GetIO();
    anim::speed = config::c.animSpeed;
    theme::Update();
    config::Tick(RealDt());
    float rdt = RealDt();
    PollLogs(rdt);
    PollGateway(rdt);
    PollDoctor(rdt);
    rpc::Tick(rdt);

    ImVec2 ds = io.DisplaySize;
    int scene = bg::BG_WAVES;
    if (g.screen == SCR_MAIN)
        scene = g.page == PG_HOME ? bg::BG_AURORA
                : g.page == PG_MCP  ? bg::BG_GRID
                : g.page == PG_LOGS ? bg::BG_CONSTELLATION
                                    : bg::BG_WAVES;
    if (ld.open && !ld.closing) scene = bg::BG_WARP;
    bg::SetScene(scene);
    bg::SetBoost(ld.open && !ld.done && !ld.closing);
    bg::Draw(ImGui::GetBackgroundDrawList(), ImVec2(0, 0), ds);

    g.screenT = ImMin(1.0f, g.screenT + anim::Dt() * 2.2f);

    ImGui::SetNextWindowPos(ImVec2(0, 0));
    ImGui::SetNextWindowSize(ds);
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0, 0));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.0f);
    ImGui::Begin("##root", nullptr, ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings |
                                    ImGuiWindowFlags_NoBackground | ImGuiWindowFlags_NoBringToFrontOnFocus |
                                    ImGuiWindowFlags_NoScrollWithMouse | ImGuiWindowFlags_NoNavFocus);
    ImGui::PopStyleVar(2);

    if (g.screen != SCR_SPLASH) TitleBar(ds);
    ImGui::PushStyleVar(ImGuiStyleVar_Alpha, anim::OutCubic(g.screenT));
    if (g.screen == SCR_SPLASH) Splash(ds);
    else if (g.screen == SCR_LANG) OnboardLang(ds);
    else if (g.screen == SCR_NAME) OnboardName(ds);
    else Main(ds);
    ImGui::PopStyleVar();
    ImGui::Dummy(ImVec2(0, 0));
    ImGui::End();

    // Discord presence: current page, onboarding step or the server being connected to
    {
        const char* st = "";
        if (ld.open && !ld.closing && !ld.serverName.empty())
            st = ld.serverName.c_str();
        else if (g.screen == SCR_LANG) st = T(S_ONBOARD_LANG);
        else if (g.screen == SCR_NAME) st = T(S_ONBOARD_NAME);
        else if (g.screen == SCR_MAIN)
            st = g.page == PG_HOME ? T(S_NAV_HOME)
                 : g.page == PG_MCP  ? T(S_NAV_GAMES)
                 : g.page == PG_LOGS ? T(S_NAV_LOGS)
                                     : T(S_NAV_SETTINGS);
        rpc::SetActivity(st);
    }

    LoaderOverlay(ds);
    Transition(ds);
    toast::Render();

    ImDrawList* fg = ImGui::GetForegroundDrawList();
    fg->AddRect(ImVec2(0.5f, 0.5f), ds - ImVec2(0.5f, 0.5f), Col(theme::Ink(0.10f)), S(14), 0, 1.0f);
}

void Shutdown()
{
    rpc::Shutdown();
    if (ld.job) core::JobFree(ld.job);
    if (g.gwStartJob) core::JobFree(g.gwStartJob);
    if (g.gwStopJob) core::JobFree(g.gwStopJob);
    if (g.orphanStopJob) core::JobFree(g.orphanStopJob);
    if (g.doctorJob) core::JobFree(g.doctorJob);
    reg::Shutdown();
    config::Save();
}

} // namespace app
