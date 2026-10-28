#pragma once
#include "imgui.h"
#include <string>

struct Config
{
    int         language      = 0;      // Lang enum, Turkish first
    int         accent        = 0;      // preset index, kPresetCount == custom
    ImVec4      customAccent  = ImVec4(0.30f, 0.55f, 1.00f, 1.00f);
    bool        bgEffects     = true;
    int         bgStyle       = 0;      // bg::Style, 0 = auto (changes per page)
    bool        particles     = true;
    float       glass         = 0.78f;
    float       animSpeed     = 1.0f;
    bool        autoClose     = false;
    bool        streamProof   = false;
    bool        notifications = true;
    bool        alwaysOnTop   = false;
    std::string userName;             // chosen once in the onboarding wizard
    bool        onboarded     = false;// language + name wizard finished
    int         totalLoads    = 0;    // gateway sessions started from the GUI

    // embedded gateway defaults (start options + server dialog prefill)
    std::string gwBind        = "127.0.0.1:8713";
    bool        gwWatch       = true;
    int         gwWatchMs     = 500;
    std::string gwToken;              // optional bearer for the gateway HTTP API
    std::string gwProfile;            // "" = active profile / all servers
    int         timeout       = 15;   // default per-server timeout, seconds

    // Discord Rich Presence
    bool        discordRpc = true;
    std::string discordId;            // application id ("" = disabled)
};

namespace config {
extern Config c;
void Load();
void Save();
void MarkDirty();   // debounced save
void Tick(float dt);
void ResetVisuals();
}
