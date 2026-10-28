#pragma once
#include "imgui.h"

// Country flags are rasterised at start-up (4x4 supersampled, rounded corners baked in)
// into small GPU textures, so they stay perfectly smooth without shipping image files.
namespace flags {
void Build();
void Draw(ImDrawList* dl, int lang, ImVec2 a, ImVec2 b, float alpha = 1.0f);
}
