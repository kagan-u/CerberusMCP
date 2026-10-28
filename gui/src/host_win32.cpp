// CerberusMCP Loader — Win32 + DirectX 11 host for the ImGui front-end.
#include "imgui.h"
#include "imgui_impl_win32.h"
#include "imgui_impl_dx11.h"

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d11.h>
#include <dwmapi.h>
#include <wincodec.h>
#include <shlwapi.h>
#include <vector>

#include "platform.h"
#include "fontpaths.h"
#include "app.h"

#pragma comment(lib, "d3d11.lib")
#pragma comment(lib, "dxgi.lib")
#pragma comment(lib, "dwmapi.lib")
#pragma comment(lib, "windowscodecs.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shlwapi.lib")

extern IMGUI_IMPL_API LRESULT ImGui_ImplWin32_WndProcHandler(HWND, UINT, WPARAM, LPARAM);

static const int kBaseW = 1060, kBaseH = 660;

static HWND                    g_hwnd;
static ID3D11Device*           g_device;
static ID3D11DeviceContext*    g_context;
static IDXGISwapChain*         g_swapChain;
static ID3D11RenderTargetView* g_rtv;
static float                   g_dpi = 1.0f;
static bool                    g_dragging;
static POINT                   g_dragCursor;
static RECT                    g_dragRect;
static bool                    g_quit;

// ---------------------------------------------------------------- utf8 helpers
static std::wstring Widen(const std::string& s)
{
    if (s.empty()) return {};
    int n = MultiByteToWideChar(CP_UTF8, 0, s.c_str(), (int)s.size(), nullptr, 0);
    std::wstring w(n, 0);
    MultiByteToWideChar(CP_UTF8, 0, s.c_str(), (int)s.size(), &w[0], n);
    return w;
}

static std::string Narrow(const std::wstring& w)
{
    if (w.empty()) return {};
    int n = WideCharToMultiByte(CP_UTF8, 0, w.c_str(), (int)w.size(), nullptr, 0, nullptr, nullptr);
    std::string s(n, 0);
    WideCharToMultiByte(CP_UTF8, 0, w.c_str(), (int)w.size(), &s[0], n, nullptr, nullptr);
    return s;
}

// ---------------------------------------------------------------- d3d
static void CreateRenderTarget()
{
    ID3D11Texture2D* back = nullptr;
    g_swapChain->GetBuffer(0, IID_PPV_ARGS(&back));
    if (back) { g_device->CreateRenderTargetView(back, nullptr, &g_rtv); back->Release(); }
}

static void CleanupRenderTarget()
{
    if (g_rtv) { g_rtv->Release(); g_rtv = nullptr; }
}

static bool CreateDeviceD3D(HWND hWnd)
{
    DXGI_SWAP_CHAIN_DESC sd = {};
    sd.BufferCount = 2;
    sd.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    sd.BufferDesc.RefreshRate.Numerator = 60;
    sd.BufferDesc.RefreshRate.Denominator = 1;
    sd.Flags = DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH;
    sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    sd.OutputWindow = hWnd;
    sd.SampleDesc.Count = 1;
    sd.Windowed = TRUE;
    sd.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

    const D3D_FEATURE_LEVEL levels[] = { D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0 };
    D3D_FEATURE_LEVEL got;
    HRESULT hr = D3D11CreateDeviceAndSwapChain(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, 0, levels, 2,
                                               D3D11_SDK_VERSION, &sd, &g_swapChain, &g_device, &got, &g_context);
    if (hr == DXGI_ERROR_UNSUPPORTED)
        hr = D3D11CreateDeviceAndSwapChain(nullptr, D3D_DRIVER_TYPE_WARP, nullptr, 0, levels, 2,
                                           D3D11_SDK_VERSION, &sd, &g_swapChain, &g_device, &got, &g_context);
    if (FAILED(hr)) return false;
    CreateRenderTarget();
    return true;
}

static void CleanupDeviceD3D()
{
    CleanupRenderTarget();
    if (g_swapChain) { g_swapChain->Release(); g_swapChain = nullptr; }
    if (g_context)   { g_context->Release();   g_context = nullptr; }
    if (g_device)    { g_device->Release();    g_device = nullptr; }
}

// ---------------------------------------------------------------- platform api
namespace platform {

ImTextureID CreateTexture(const unsigned char* rgba, int w, int h)
{
    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = w;
    desc.Height = h;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;

    D3D11_SUBRESOURCE_DATA sub = {};
    sub.pSysMem = rgba;
    sub.SysMemPitch = w * 4;

    ID3D11Texture2D* tex = nullptr;
    if (FAILED(g_device->CreateTexture2D(&desc, &sub, &tex))) return 0;

    D3D11_SHADER_RESOURCE_VIEW_DESC sv = {};
    sv.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    sv.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
    sv.Texture2D.MipLevels = 1;
    ID3D11ShaderResourceView* srv = nullptr;
    g_device->CreateShaderResourceView(tex, &sv, &srv);
    tex->Release();
    return (ImTextureID)(uintptr_t)srv;
}

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
    IWICImagingFactory* factory = nullptr;
    if (FAILED(CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&factory))))
        return 0;

    ImTextureID result = 0;
    IWICBitmapDecoder* decoder = nullptr;
    IWICBitmapFrameDecode* frame = nullptr;
    IWICFormatConverter* conv = nullptr;
    std::wstring wpath = Widen(utf8Path);

    if (SUCCEEDED(factory->CreateDecoderFromFilename(wpath.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnDemand, &decoder)) &&
        SUCCEEDED(decoder->GetFrame(0, &frame)) &&
        SUCCEEDED(factory->CreateFormatConverter(&conv)) &&
        SUCCEEDED(conv->Initialize(frame, GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, nullptr, 0.0, WICBitmapPaletteTypeCustom)))
    {
        UINT w = 0, h = 0;
        conv->GetSize(&w, &h);
        std::vector<unsigned char> px((size_t)w * h * 4);
        if (SUCCEEDED(conv->CopyPixels(nullptr, w * 4, (UINT)px.size(), px.data())))
        {
            KeyWhite(px.data(), (int)w, (int)h);
            result = CreateTexture(px.data(), (int)w, (int)h);
            if (outW) *outW = (int)w;
            if (outH) *outH = (int)h;
        }
    }
    if (conv) conv->Release();
    if (frame) frame->Release();
    if (decoder) decoder->Release();
    factory->Release();
    return result;
}

std::string ExeDir()
{
    wchar_t buf[MAX_PATH];
    GetModuleFileNameW(nullptr, buf, MAX_PATH);
    PathRemoveFileSpecW(buf);
    return Narrow(buf);
}

std::string PathJoin(const std::string& a, const std::string& b)
{
    if (a.empty()) return b;
    if (b.empty()) return a;
    if (a.back() == '\\' || a.back() == '/') return a + b;
    return a + "\\" + b;
}

bool FileExists(const std::string& utf8Path)
{
    DWORD a = GetFileAttributesW(Widen(utf8Path).c_str());
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

std::string FindAsset(const char* relative)
{
    std::string exe = ExeDir();
    const std::string candidates[] = {
        PathJoin(exe, relative),
        PathJoin(PathJoin(exe, ".."), relative),
        PathJoin(PathJoin(PathJoin(exe, ".."), ".."), relative),
        std::string(relative),
    };
    for (const std::string& c : candidates)
        if (FileExists(c)) return c;
    return {};
}

std::string SystemFont(const char* fileName)
{
    if (const char* g = fontpaths::Bundled(fileName); g && *g)
    {
        const std::string p = FindAsset((std::string("assets/fonts/") + g).c_str());
        if (!p.empty()) return p;
    }
    wchar_t win[MAX_PATH];
    GetWindowsDirectoryW(win, MAX_PATH);
    std::string p = Narrow(win) + "\\Fonts\\" + fileName;
    return FileExists(p) ? p : std::string();
}

std::string ReadFile(const std::string& utf8Path)
{
    HANDLE f = CreateFileW(Widen(utf8Path).c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
    if (f == INVALID_HANDLE_VALUE) return {};
    std::string out;
    DWORD size = GetFileSize(f, nullptr), read = 0;
    if (size != INVALID_FILE_SIZE && size > 0)
    {
        out.resize(size);
        ::ReadFile(f, &out[0], size, &read, nullptr);
        out.resize(read);
    }
    CloseHandle(f);
    return out;
}

bool WriteFile(const std::string& utf8Path, const std::string& data)
{
    HANDLE f = CreateFileW(Widen(utf8Path).c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, 0, nullptr);
    if (f == INVALID_HANDLE_VALUE) return false;
    DWORD written = 0;
    BOOL ok = ::WriteFile(f, data.data(), (DWORD)data.size(), &written, nullptr);
    CloseHandle(f);
    return ok && written == data.size();
}

std::string UserName()
{
    wchar_t buf[256];
    DWORD n = 256;
    if (GetUserNameW(buf, &n)) return Narrow(buf);
    return "User";
}

std::string HardwareId()
{
    DWORD serial = 0;
    GetVolumeInformationW(L"C:\\", nullptr, 0, &serial, nullptr, nullptr, nullptr, 0);
    wchar_t pc[MAX_COMPUTERNAME_LENGTH + 1];
    DWORD n = MAX_COMPUTERNAME_LENGTH + 1;
    GetComputerNameW(pc, &n);
    unsigned int h = 2166136261u ^ serial;
    for (DWORD i = 0; i < n; i++) { h ^= pc[i]; h *= 16777619u; }
    unsigned int h2 = h * 2654435761u ^ (serial >> 3);
    char out[32];
    snprintf(out, sizeof(out), "%04X-%04X-%04X-%04X", (h >> 16) & 0xFFFF, h & 0xFFFF, (h2 >> 16) & 0xFFFF, h2 & 0xFFFF);
    return out;
}

void Minimize() { ShowWindow(g_hwnd, SW_MINIMIZE); }
void Close()    { PostMessageW(g_hwnd, WM_CLOSE, 0, 0); }

void BeginWindowDrag()
{
    g_dragging = true;
    GetCursorPos(&g_dragCursor);
    GetWindowRect(g_hwnd, &g_dragRect);
}

void SetTopmost(bool on)
{
    SetWindowPos(g_hwnd, on ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}

void SetStreamProof(bool on)
{
    // WDA_EXCLUDEFROMCAPTURE (Windows 10 2004+). Falls back to WDA_MONITOR on older builds.
    if (!SetWindowDisplayAffinity(g_hwnd, on ? 0x00000011 : WDA_NONE) && on)
        SetWindowDisplayAffinity(g_hwnd, WDA_MONITOR);
}

float DpiScale() { return g_dpi; }

bool MakeDirs(const std::string& utf8Path)
{
    if (utf8Path.empty()) return false;
    std::string acc;
    size_t i = 0;
    if (utf8Path.size() > 1 && utf8Path[1] == ':') { acc = utf8Path.substr(0, 2); i = 2; if (utf8Path.size() > 2 && utf8Path[2] == '/') { acc += '/'; i = 3; } }
    while (i <= utf8Path.size())
    {
        size_t j = utf8Path.find_first_of("/\\", i);
        if (j == std::string::npos) j = utf8Path.size();
        if (j > i)
        {
            acc += utf8Path.substr(i, j - i);
            if (j < utf8Path.size() && !CreateDirectoryW(Widen(acc).c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS)
                return false;
            acc += "\\";
        }
        i = j + 1;
    }
    return true;
}

} // namespace platform

// ---------------------------------------------------------------- window
static LRESULT WINAPI WndProc(HWND hWnd, UINT msg, WPARAM wParam, LPARAM lParam)
{
    if (ImGui_ImplWin32_WndProcHandler(hWnd, msg, wParam, lParam))
        return true;

    switch (msg)
    {
    case WM_SIZE:
        if (g_device && wParam != SIZE_MINIMIZED && LOWORD(lParam) > 0)
        {
            CleanupRenderTarget();
            g_swapChain->ResizeBuffers(0, LOWORD(lParam), HIWORD(lParam), DXGI_FORMAT_UNKNOWN, 0);
            CreateRenderTarget();
        }
        return 0;
    case WM_SYSCOMMAND:
        if ((wParam & 0xfff0) == SC_KEYMENU) return 0;
        break;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hWnd, msg, wParam, lParam);
}

static void ApplyRoundedCorners(HWND hwnd, int w, int h)
{
    // Windows 11: native DWM rounding. Windows 10: fall back to a window region.
    const DWORD DWMWA_WINDOW_CORNER_PREFERENCE_ = 33;
    int pref = 2; // DWMWCP_ROUND
    if (FAILED(DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE_, &pref, sizeof(pref))))
    {
        int d = (int)(24 * g_dpi);
        SetWindowRgn(hwnd, CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d), TRUE);
    }
}

int WINAPI wWinMain(HINSTANCE hInst, HINSTANCE, PWSTR, int)
{
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    ImGui_ImplWin32_EnableDpiAwareness();
    g_dpi = ImGui_ImplWin32_GetDpiScaleForMonitor(MonitorFromPoint(POINT{ 0, 0 }, MONITOR_DEFAULTTOPRIMARY));
    if (g_dpi < 1.0f) g_dpi = 1.0f;

    WNDCLASSEXW wc = { sizeof(wc), CS_CLASSDC | CS_DROPSHADOW, WndProc, 0, 0, hInst, nullptr, LoadCursor(nullptr, IDC_ARROW), nullptr, nullptr, L"CerberusMCP", nullptr };
    wc.hIcon = LoadIcon(nullptr, IDI_APPLICATION);
    RegisterClassExW(&wc);

    int w = (int)(kBaseW * g_dpi), h = (int)(kBaseH * g_dpi);
    RECT work;
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
    int x = work.left + ((work.right - work.left) - w) / 2;
    int y = work.top + ((work.bottom - work.top) - h) / 2;

    g_hwnd = CreateWindowExW(WS_EX_APPWINDOW, wc.lpszClassName, L"CerberusMCP Loader",
                             WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU, x, y, w, h, nullptr, nullptr, hInst, nullptr);
    if (!CreateDeviceD3D(g_hwnd))
    {
        CleanupDeviceD3D();
        UnregisterClassW(wc.lpszClassName, hInst);
        MessageBoxW(nullptr, L"DirectX 11 could not be initialised.", L"CerberusMCP Loader", MB_ICONERROR);
        return 1;
    }
    ApplyRoundedCorners(g_hwnd, w, h);

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();
    io.IniFilename = nullptr;
    io.LogFilename = nullptr;
    io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;

    ImGui_ImplWin32_Init(g_hwnd);
    ImGui_ImplDX11_Init(g_device, g_context);

    app::Init();

    ShowWindow(g_hwnd, SW_SHOWDEFAULT);
    UpdateWindow(g_hwnd);

    while (!g_quit)
    {
        MSG msg;
        while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE))
        {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if (msg.message == WM_QUIT) g_quit = true;
        }
        if (g_quit) break;

        if (IsIconic(g_hwnd)) { Sleep(16); continue; }

        if (g_dragging)
        {
            if (!(GetAsyncKeyState(VK_LBUTTON) & 0x8000))
                g_dragging = false;
            else
            {
                POINT p;
                GetCursorPos(&p);
                SetWindowPos(g_hwnd, nullptr, g_dragRect.left + (p.x - g_dragCursor.x), g_dragRect.top + (p.y - g_dragCursor.y),
                             0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }

        ImGui_ImplDX11_NewFrame();
        ImGui_ImplWin32_NewFrame();
        ImGui::NewFrame();
        app::Frame();
        ImGui::Render();

        const float clear[4] = { 0.945f, 0.949f, 0.958f, 1.0f };   // theme::bg0
        g_context->OMSetRenderTargets(1, &g_rtv, nullptr);
        g_context->ClearRenderTargetView(g_rtv, clear);
        ImGui_ImplDX11_RenderDrawData(ImGui::GetDrawData());
        g_swapChain->Present(1, 0);
    }

    app::Shutdown();
    ImGui_ImplDX11_Shutdown();
    ImGui_ImplWin32_Shutdown();
    ImGui::DestroyContext();
    CleanupDeviceD3D();
    DestroyWindow(g_hwnd);
    UnregisterClassW(wc.lpszClassName, hInst);
    CoUninitialize();
    return 0;
}
