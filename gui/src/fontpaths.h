#pragma once
// Maps a Windows font file name (as used by the UI code) to candidate paths on
// macOS and Linux. Shared by the loader and the game menus; header-only so both
// link targets can use it without an extra translation unit.
#include <cstring>
#include <string>

namespace fontpaths {

// Bundled Geist equivalent for a Windows font file name ("" when none).
// The Geist family ships in assets/fonts and covers Türkçe (ğ ş ı İ) fully.
inline const char* Bundled(const char* windowsName)
{
    if (!windowsName || !*windowsName) return "";
    if (!strcmp(windowsName, "segoeui.ttf"))     return "Geist-Regular.ttf";
    if (!strcmp(windowsName, "seguisb.ttf"))     return "Geist-SemiBold.ttf";
    if (!strcmp(windowsName, "segoeuib.ttf"))    return "Geist-Bold.ttf";
    if (!strcmp(windowsName, "bahnschrift.ttf")) return "Geist-Bold.ttf";
    if (!strcmp(windowsName, "consola.ttf"))     return "GeistMono-Regular.ttf";
    if (!strcmp(windowsName, "consolab.ttf"))    return "GeistMono-Medium.ttf";
    return "";
}

// Resolves the bundled Geist file for `windowsName` through a caller-supplied
// existence check, trying every plausible assets/ location relative to the
// process working directory (loader: ./assets, menus: ../../assets).
template <typename Exists>
inline std::string BundledPath(const char* windowsName, Exists exists)
{
    const char* file = Bundled(windowsName);
    if (!*file) return {};
    static const char* const kDirs[] = {
        "assets/fonts/", "../assets/fonts/", "../../assets/fonts/", "../../../assets/fonts/",
    };
    for (const char* d : kDirs)
    {
        std::string p = std::string(d) + file;
        if (exists(p)) return p;
    }
    return {};
}

inline const char* const* Candidates(const char* windowsName, int* count)
{
    static const char* const kSans[] = {
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
    };
    static const char* const kBold[] = {
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/Library/Fonts/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
    };
    static const char* const kMono[] = {
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/SFNSMono.ttf",
        "/System/Library/Fonts/Supplemental/Courier New.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationMono-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
        "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
    };
    static const char* const kMonoBold[] = {
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationMono-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/ubuntu/UbuntuMono-B.ttf",
        "/usr/share/fonts/truetype/freefont/FreeMonoBold.ttf",
    };

    const char* const* list = kSans;
    int n = (int)(sizeof(kSans) / sizeof(*kSans));

    if (!windowsName || !*windowsName) { *count = 0; return nullptr; }

    if (!strcmp(windowsName, "consola.ttf"))      { list = kMono;      n = (int)(sizeof(kMono) / sizeof(*kMono)); }
    else if (!strcmp(windowsName, "consolab.ttf")){ list = kMonoBold;  n = (int)(sizeof(kMonoBold) / sizeof(*kMonoBold)); }
    else if (!strcmp(windowsName, "segoeuib.ttf") ||
             !strcmp(windowsName, "seguisb.ttf")  ||
             !strcmp(windowsName, "bahnschrift.ttf"))
                                                  { list = kBold;      n = (int)(sizeof(kBold) / sizeof(*kBold)); }

    *count = n;
    return list;
}

} // namespace fontpaths
