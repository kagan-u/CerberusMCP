// CerberusMCP Loader — GLFW + OpenGL 3 host for the ImGui front-end (macOS & Linux).
#include "imgui.h"
#include "imgui_impl_glfw.h"
#include "imgui_impl_opengl3.h"

#define GLFW_INCLUDE_NONE
#include <GLFW/glfw3.h>
#if defined(__APPLE__)
#define GLFW_EXPOSE_NATIVE_COCOA
#include <GLFW/glfw3native.h>
#define GL_SILENCE_DEPRECATION
#include <OpenGL/gl3.h>
#include <mach-o/dyld.h>
#include <objc/objc.h>
#include <objc/message.h>
#else
#include <GL/gl.h>
#endif
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <pwd.h>
#include <unistd.h>
#include <cfloat>
#include <climits>
#include <cerrno>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#define STB_IMAGE_IMPLEMENTATION
#include "stb_image.h"

#include "fontpaths.h"
#include "platform.h"
#include "app.h"

static const int kBaseW = 1060, kBaseH = 660;

static GLFWwindow* g_win;
static float       g_scale = 1.0f;
static bool        g_dragging;
static double      g_dragX0, g_dragY0;
// ---------------------------------------------------------------- path helpers
static std::string DirName(const std::string& p)
{
    size_t i = p.find_last_of('/');
    if (i == std::string::npos) return {};
    return p.substr(0, i);
}

// Keeps ImGui's coordinate space in framebuffer pixels (like the Win32 host) so
// fonts are rasterised at the real device resolution and stay crisp on Retina /
// HiDPI displays. The GLFW backend reports points, so the display size is
// rescaled once per frame here and the mouse position is rescaled at the source
// (OnCursorPos) - scaling io.MousePos inside the frame would be overwritten by
// the queued events in ImGui::NewFrame(), which used to leave the mouse two
// times off and made buttons hard to hit.
static void ApplyDisplayScale()
{
    int w = 0, h = 0, fw = 0, fh = 0;
    glfwGetWindowSize(g_win, &w, &h);
    glfwGetFramebufferSize(g_win, &fw, &fh);
    if (w <= 0 || h <= 0) return;

    ImGuiIO& io = ImGui::GetIO();
    if (fw == w && fh == h) return;

    io.DisplaySize = ImVec2((float)fw, (float)fh);
    io.DisplayFramebufferScale = ImVec2(1.0f, 1.0f);
}

// Mouse position in points -> framebuffer pixels, forwarded to the ImGui backend.
static void OnCursorPos(GLFWwindow* win, double x, double y)
{
    int w = 0, h = 0, fw = 0, fh = 0;
    glfwGetWindowSize(win, &w, &h);
    glfwGetFramebufferSize(win, &fw, &fh);
    float sx = (w > 0) ? (float)fw / (float)w : 1.0f;
    float sy = (h > 0) ? (float)fh / (float)h : 1.0f;
    ImGui_ImplGlfw_CursorPosCallback(win, x * sx, y * sy);
}

// ---------------------------------------------------------------- platform api
namespace platform {

ImTextureID CreateTexture(const unsigned char* rgba, int w, int h)
{
    GLuint tex = 0;
    glGenTextures(1, &tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba);
    glBindTexture(GL_TEXTURE_2D, 0);
    return (ImTextureID)(uintptr_t)tex;
}

// White background -> alpha: a = 1 - min(r,g,b)/255, colour unpremultiplied against white.
static void KeyWhite(unsigned char* px, int w, int h)
{
    for (int i = 0, n = w * h * 4; i < n; i += 4)
    {
        int mn = px[i] < px[i + 1] ? px[i] : px[i + 1];
        if (px[i + 2] < mn) mn = px[i + 2];
        int a = 255 - mn;
        if (a <= 0) { px[i + 3] = 0; continue; }
        int inv = 255 - a;
        for (int k = 0; k < 3; k++)
        {
            int v = px[i + k] - inv;
            px[i + k] = v > 0 ? (unsigned char)v : 0;
        }
        px[i + 3] = (unsigned char)a;
    }
}

ImTextureID LoadTextureWhiteKey(const std::string& utf8Path, int* outW, int* outH)
{
    int w = 0, h = 0, comp = 0;
    unsigned char* px = stbi_load(utf8Path.c_str(), &w, &h, &comp, 4);
    if (!px) return 0;
    KeyWhite(px, w, h);
    ImTextureID tex = CreateTexture(px, w, h);
    stbi_image_free(px);
    if (outW) *outW = w;
    if (outH) *outH = h;
    return tex;
}

std::string ExeDir()
{
    char buf[PATH_MAX] = {};
#if defined(__APPLE__)
    uint32_t size = sizeof(buf);
    if (_NSGetExecutablePath(buf, &size) != 0) return {};
    char resolved[PATH_MAX] = {};
    if (realpath(buf, resolved)) return DirName(resolved);
    return DirName(buf);
#else
    ssize_t n = readlink("/proc/self/exe", buf, sizeof(buf) - 1);
    if (n <= 0) return {};
    buf[n] = 0;
    return DirName(buf);
#endif
}

std::string PathJoin(const std::string& a, const std::string& b)
{
    if (a.empty()) return b;
    if (b.empty()) return a;
    if (a.back() == '/') return a + b;
    return a + "/" + b;
}

bool FileExists(const std::string& utf8Path)
{
    struct stat st;
    return stat(utf8Path.c_str(), &st) == 0 && S_ISREG(st.st_mode);
}

std::string FindAsset(const char* relative)
{
    std::string exe = ExeDir();
    std::vector<std::string> candidates = {
        PathJoin(exe, relative),
        PathJoin(PathJoin(exe, ".."), relative),
        PathJoin(PathJoin(PathJoin(exe, ".."), ".."), relative),
        std::string(relative),
    };
#if defined(__APPLE__)
    candidates.insert(candidates.begin(), PathJoin(PathJoin(PathJoin(exe, ".."), "Resources"), relative));
#endif
    for (const std::string& c : candidates)
        if (!c.empty() && FileExists(c)) return c;
    return {};
}

std::string SystemFont(const char* fileName)
{
    if (const char* g = fontpaths::Bundled(fileName); g && *g)
    {
        const std::string p = FindAsset((std::string("assets/fonts/") + g).c_str());
        if (!p.empty()) return p;
    }
    int n = 0;
    const char* const* list = fontpaths::Candidates(fileName, &n);
    for (int i = 0; i < n; i++)
        if (FileExists(list[i])) return list[i];
    return {};
}

std::string ReadFile(const std::string& utf8Path)
{
    FILE* f = fopen(utf8Path.c_str(), "rb");
    if (!f) return {};
    std::string out;
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), f)) > 0) out.append(buf, n);
    fclose(f);
    return out;
}

bool WriteFile(const std::string& utf8Path, const std::string& data)
{
    FILE* f = fopen(utf8Path.c_str(), "wb");
    if (!f) return false;
    size_t written = data.empty() ? 0 : fwrite(data.data(), 1, data.size(), f);
    fclose(f);
    return written == data.size();
}

std::string UserName()
{
    if (const char* u = getenv("USER")) if (*u) return u;
    if (struct passwd* p = getpwuid(getuid())) if (p->pw_name && *p->pw_name) return p->pw_name;
    return "User";
}

std::string HardwareId()
{
    std::string seed = ReadFile("/etc/machine-id");
    if (seed.empty()) seed = ReadFile("/var/lib/dbus/machine-id");
    while (!seed.empty() && (seed.back() == '\n' || seed.back() == '\r' || seed.back() == ' '))
        seed.pop_back();
    if (seed.empty())
    {
        struct utsname u;
        if (uname(&u) == 0) { seed = u.nodename; seed += u.machine; }
    }
    if (seed.empty()) seed = "cerberusmcp";

    unsigned int h = 2166136261u;
    for (size_t i = 0; i < seed.size(); i++) { h ^= (unsigned char)seed[i]; h *= 16777619u; }
    unsigned int h2 = h * 2654435761u ^ (unsigned int)(seed.size() << 3);
    char out[32];
    snprintf(out, sizeof(out), "%04X-%04X-%04X-%04X", (h >> 16) & 0xFFFF, h & 0xFFFF, (h2 >> 16) & 0xFFFF, h2 & 0xFFFF);
    return out;
}

#if defined(__APPLE__)
// Frameless windows are square; round the content view so the panel doesn't
// look like it was cut out of the desktop. masksToBounds on the content layer
// clips the GL drawable to the rounded shape.
static void RoundWindowCorners(double radius)
{
    if (!g_win) return;
    id win = (id)glfwGetCocoaWindow(g_win);
    if (!win) return;

    // responds to setWantsLayer:? (checked first: the window class varies)
    SEL respSel = sel_registerName("respondsToSelector:");
    SEL wantsSel = sel_registerName("setWantsLayer:");
    BOOL ok = ((BOOL (*)(id, SEL, SEL))objc_msgSend)(win, respSel, wantsSel);
    id target = win;
    if (!ok)
    {
        target = ((id (*)(id, SEL))objc_msgSend)(win, sel_registerName("contentView"));
        if (!target) return;
        ok = ((BOOL (*)(id, SEL, SEL))objc_msgSend)(target, respSel, wantsSel);
        if (!ok) return;
    }
    ((void (*)(id, SEL, BOOL))objc_msgSend)(target, wantsSel, YES);
    id layer = ((id (*)(id, SEL))objc_msgSend)(target, sel_registerName("layer"));
    if (!layer) return;
    ((void (*)(id, SEL, double))objc_msgSend)(layer, sel_registerName("setCornerRadius:"), radius);
    ((void (*)(id, SEL, BOOL))objc_msgSend)(layer, sel_registerName("setMasksToBounds:"), YES);
}
#else
static void RoundWindowCorners(double) {}
#endif

void Minimize() { if (g_win) glfwIconifyWindow(g_win); }
void Close()    { if (g_win) glfwSetWindowShouldClose(g_win, GLFW_TRUE); }

void BeginWindowDrag()
{
    if (!g_win) return;
    glfwGetCursorPos(g_win, &g_dragX0, &g_dragY0);
    g_dragging = true;
}

void SetTopmost(bool on)
{
#if defined(__APPLE__)
    // Frameless window: toggle NSFloatingWindowLevel on the backing NSWindow.
    if (!g_win) return;
    void* ns = glfwGetCocoaWindow(g_win);
    if (!ns) return;
    ((void (*)(id, SEL, long))objc_msgSend)((id)ns, sel_registerName("setLevel:"), on ? 3L : 0L);
#else
    (void)on;   // GLFW has no cross-platform "always on top" (X11/Wayland differ)
#endif
}

void SetStreamProof(bool on) { (void)on; }   // Windows-only capture protection

bool MakeDirs(const std::string& utf8Path)
{
    if (utf8Path.empty()) return false;
    std::string acc;
    size_t i = 0;
    if (utf8Path[0] == '/') { acc = "/"; i = 1; }
    while (i <= utf8Path.size())
    {
        size_t j = utf8Path.find('/', i);
        if (j == std::string::npos) j = utf8Path.size();
        if (j > i)
        {
            acc += utf8Path.substr(i, j - i);
            if (j < utf8Path.size() && mkdir(acc.c_str(), 0755) != 0 && errno != EEXIST) return false;
            acc += "/";
        }
        i = j + 1;
    }
    return true;
}

float DpiScale()
{
    if (!g_win) return g_scale;
    int w = 0, h = 0, fw = 0, fh = 0;
    glfwGetWindowSize(g_win, &w, &h);
    glfwGetFramebufferSize(g_win, &fw, &fh);
    g_scale = (w > 0) ? (float)fw / (float)w : 1.0f;
    if (g_scale < 1.0f) g_scale = 1.0f;
    return g_scale;
}

} // namespace platform

// ---------------------------------------------------------------- window
int main()
{
    if (!glfwInit())
    {
        fprintf(stderr, "CerberusMCP Loader: glfwInit failed.\n");
        return 1;
    }

#if defined(__APPLE__)
    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 2);
    glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);
    glfwWindowHint(GLFW_OPENGL_FORWARD_COMPAT, GL_TRUE);
    const char* glsl = "#version 150";
#else
    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 0);
    const char* glsl = "#version 130";
#endif
    glfwWindowHint(GLFW_DECORATED, GLFW_FALSE);
    glfwWindowHint(GLFW_RESIZABLE, GLFW_FALSE);

    g_win = glfwCreateWindow(kBaseW, kBaseH, "CerberusMCP Loader", nullptr, nullptr);
    if (!g_win)
    {
        fprintf(stderr, "CerberusMCP Loader: window creation failed.\n");
        glfwTerminate();
        return 1;
    }
    glfwMakeContextCurrent(g_win);
    glfwSwapInterval(1);

    platform::RoundWindowCorners(15.0);   // slightly rounder window edges

    if (GLFWmonitor* mon = glfwGetPrimaryMonitor())
    {
        int mx = 0, my = 0, mw = 0, mh = 0;
        glfwGetMonitorWorkarea(mon, &mx, &my, &mw, &mh);
        if (mw > 0 && mh > 0)
            glfwSetWindowPos(g_win, mx + (mw - kBaseW) / 2, my + (mh - kBaseH) / 2);
    }

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();
    io.IniFilename = nullptr;
    io.LogFilename = nullptr;
    io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;

    ImGui_ImplGlfw_InitForOpenGL(g_win, true);
    ImGui_ImplOpenGL3_Init(glsl);
    // Replace the backend's cursor callback so positions arrive already scaled.
    glfwSetCursorPosCallback(g_win, OnCursorPos);

    app::Init();

    while (!glfwWindowShouldClose(g_win))
    {
        glfwPollEvents();

        if (g_dragging)
        {
            double cx = 0, cy = 0;
            int ww = 0, wh = 0;
            glfwGetCursorPos(g_win, &cx, &cy);
            glfwGetWindowSize(g_win, &ww, &wh);
            if (glfwGetMouseButton(g_win, GLFW_MOUSE_BUTTON_LEFT) != GLFW_PRESS ||
                !ImGui::IsMouseDown(ImGuiMouseButton_Left) ||
                cx < 0.0 || cy < 0.0 || cx > ww || cy > wh)
                g_dragging = false;
            else
            {
                int wx = 0, wy = 0;
                glfwGetWindowPos(g_win, &wx, &wy);
                glfwSetWindowPos(g_win, wx + (int)(cx - g_dragX0), wy + (int)(cy - g_dragY0));
            }
        }

        ImGui_ImplOpenGL3_NewFrame();
        ImGui_ImplGlfw_NewFrame();
        ApplyDisplayScale();
        ImGui::NewFrame();
        app::Frame();
        ImGui::Render();

        int fw = 0, fh = 0;
        glfwGetFramebufferSize(g_win, &fw, &fh);
        glViewport(0, 0, fw, fh);
        const float clear[4] = { 0.945f, 0.949f, 0.958f, 1.0f };   // theme::bg0
        glClearColor(clear[0], clear[1], clear[2], clear[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
        glfwSwapBuffers(g_win);
    }

    app::Shutdown();
    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();
    glfwDestroyWindow(g_win);
    glfwTerminate();
    return 0;
}
