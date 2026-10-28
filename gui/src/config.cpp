#include "config.h"
#include "platform.h"
#include <cstdio>
#include <cstdlib>
#include <sstream>

namespace config {

Config c;
static float g_dirtyTimer = -1.0f;

static std::string Path() { return platform::PathJoin(platform::ExeDir(), "cerberusmcp.ini"); }

void Load()
{
    std::istringstream in(platform::ReadFile(Path()));
    std::string line;
    int themeVersion = 1;
    while (std::getline(in, line))
    {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        size_t eq = line.find('=');
        if (eq == std::string::npos) continue;
        std::string k = line.substr(0, eq), v = line.substr(eq + 1);
        float f = (float)atof(v.c_str());
        int   i = atoi(v.c_str());
        if      (k == "language")      c.language = i;
        else if (k == "accent")        c.accent = i;
        else if (k == "custom")        sscanf(v.c_str(), "%f,%f,%f", &c.customAccent.x, &c.customAccent.y, &c.customAccent.z);
        else if (k == "theme_v")       themeVersion = i;
        else if (k == "bg_effects")    c.bgEffects = i != 0;
        else if (k == "bg_style")      c.bgStyle = i;
        else if (k == "particles")     c.particles = i != 0;
        else if (k == "glass")         c.glass = f;
        else if (k == "anim_speed")    c.animSpeed = f;
        else if (k == "auto_close")    c.autoClose = i != 0;
        else if (k == "stream_proof")  c.streamProof = i != 0;
        else if (k == "notifications") c.notifications = i != 0;
        else if (k == "always_on_top") c.alwaysOnTop = i != 0;
        else if (k == "user_name")     c.userName = v;
        else if (k == "onboarded")     c.onboarded = i != 0;
        else if (k == "total_loads")   c.totalLoads = i;
        else if (k == "timeout")       c.timeout = i;
        else if (k == "gw_bind")       c.gwBind = v;
        else if (k == "gw_watch")      c.gwWatch = i != 0;
        else if (k == "gw_watch_ms")   c.gwWatchMs = i;
        else if (k == "gw_token")      c.gwToken = v;
        else if (k == "gw_profile")    c.gwProfile = v;
        else if (k == "discord_rpc")   c.discordRpc = i != 0;
        else if (k == "discord_id")    c.discordId = v;
    }
    if (themeVersion < 3) c.accent = 0;   // v3 switched to the white theme
    if (c.bgStyle < 0 || c.bgStyle > 5) c.bgStyle = 0;
    if (c.glass < 0.3f || c.glass > 1.0f) c.glass = 0.78f;
    if (c.animSpeed < 0.5f || c.animSpeed > 2.0f) c.animSpeed = 1.0f;
    if (c.gwBind.empty()) c.gwBind = "127.0.0.1:8713";
    if (c.gwWatchMs < 100 || c.gwWatchMs > 60000) c.gwWatchMs = 500;
    if (c.timeout < 3 || c.timeout > 300) c.timeout = 15;
    if (c.discordId.empty()) c.discordId = "1564987469475221624";  // shipped default app
}

void Save()
{
    char buf[2048];
    snprintf(buf, sizeof(buf),
             "theme_v=3\nlanguage=%d\naccent=%d\ncustom=%.3f,%.3f,%.3f\nbg_effects=%d\nbg_style=%d\nparticles=%d\nglass=%.3f\nanim_speed=%.3f\n"
             "auto_close=%d\nstream_proof=%d\nnotifications=%d\nalways_on_top=%d\nuser_name=%s\nonboarded=%d\ntotal_loads=%d\n"
             "timeout=%d\ngw_bind=%s\ngw_watch=%d\ngw_watch_ms=%d\ngw_token=%s\ngw_profile=%s\ndiscord_rpc=%d\ndiscord_id=%s\n",
             c.language, c.accent, c.customAccent.x, c.customAccent.y, c.customAccent.z, c.bgEffects, c.bgStyle, c.particles, c.glass,
             c.animSpeed, c.autoClose, c.streamProof, c.notifications, c.alwaysOnTop, c.userName.c_str(), c.onboarded ? 1 : 0, c.totalLoads,
             c.timeout, c.gwBind.c_str(), c.gwWatch ? 1 : 0, c.gwWatchMs, c.gwToken.c_str(), c.gwProfile.c_str(),
             c.discordRpc ? 1 : 0, c.discordId.c_str());
    platform::WriteFile(Path(), buf);
    g_dirtyTimer = -1.0f;
}

void MarkDirty() { g_dirtyTimer = 0.6f; }

void Tick(float dt)
{
    if (g_dirtyTimer < 0.0f) return;
    g_dirtyTimer -= dt;
    if (g_dirtyTimer <= 0.0f) Save();
}

void ResetVisuals()
{
    Config d;
    c.accent = d.accent;
    c.customAccent = d.customAccent;
    c.bgEffects = d.bgEffects;
    c.bgStyle = d.bgStyle;
    c.particles = d.particles;
    c.glass = d.glass;
    c.animSpeed = d.animSpeed;
    c.autoClose = d.autoClose;
    c.streamProof = d.streamProof;
    c.notifications = d.notifications;
    c.alwaysOnTop = d.alwaysOnTop;
    MarkDirty();
}

} // namespace config
