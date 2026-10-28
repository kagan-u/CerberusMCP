#pragma once
#include "core.h"
#include "icons.h"

namespace ui {

enum ButtonStyle { Btn_Primary, Btn_Secondary, Btn_Ghost, Btn_Danger };

bool  Button(const char* label, ImVec2 size, ButtonStyle style = Btn_Primary, Icon icon = Ico_None, ImFont* font = nullptr);
bool  IconButton(const char* id, Icon icon, float size, const char* tooltip = nullptr, ImU32 hoverTint = 0);
bool  Checkbox(const char* label, bool* v);
bool  Toggle(const char* label, const char* desc, bool* v, float width);
bool  Slider(const char* label, float* v, float mn, float mx, const char* fmt, float width, float displayMul = 1.0f);
bool  Swatch(const char* id, ImVec4 a, ImVec4 b, bool selected, float size, bool rainbow = false);
bool  FlagButton(const char* id, int lang, ImVec2 size, bool selected);
float Badge(ImDrawList* dl, ImVec2 pos, const char* text, ImVec4 color, bool pulse = true);
float Chip(ImDrawList* dl, ImVec2 pos, const char* text, ImVec4 color, Icon icon = Ico_None);
float FeatureChip(ImDrawList* dl, ImVec2 pos, const char* text, Icon icon, bool measureOnly = false);
// Current flag + name; click opens an animated list of every language. Returns the picked language or -1.
int   LanguagePicker(const char* id, float width, bool openUp);
void  Tooltip(const char* text);
void  Shine(ImDrawList* dl, ImVec2 a, ImVec2 b, float rounding, float alpha, float period = 2.6f);

// Card container: content is laid out inside a group, the glass panel is drawn behind it afterwards.
void  CardBegin(const char* title, Icon icon, ImVec2 pos, float width);
float CardEnd();                 // returns the card bottom (screen y)
float CardInnerWidth();

} // namespace ui

namespace toast {
enum Type { Info, Success, Warning, Error };
void Push(Type type, const char* title, const char* text = nullptr, float duration = 3.6f);
void Render();
}
