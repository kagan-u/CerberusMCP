#pragma once
#include "imgui.h"

// Vector icon set drawn straight into a draw list (crisp at any DPI, no icon font needed).
enum Icon
{
    Ico_None, Ico_Home, Ico_Grid, Ico_Gear, Ico_User, Ico_Key, Ico_Lock, Ico_Eye, Ico_EyeOff,
    Ico_Clipboard, Ico_Copy, Ico_Check, Ico_Close, Ico_Minimize, Ico_Play, Ico_Shield, Ico_Clock,
    Ico_Download, Ico_Logout, Ico_Globe, Ico_Bolt, Ico_Signal, Ico_Info, Ico_Sparkle, Ico_Bell,
    Ico_Chevron, Ico_Cpu, Ico_Crown, Ico_Refresh, Ico_Arrow, Ico_Layers,
    Ico_Target, Ico_Radar, Ico_Cloud, Ico_Car, Ico_Code, Ico_ChevronLeft,
};

void DrawIcon(ImDrawList* dl, Icon icon, ImVec2 center, float size, ImU32 col, float thickness = 0.0f);
