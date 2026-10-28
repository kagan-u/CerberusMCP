#include "rpc.h"
#include "config.h"

#include <cstdio>
#include <cstring>
#include <string>

#if !defined(_WIN32)
#include <cerrno>
#include <cstdlib>
#include <ctime>
#include <fcntl.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <vector>
#endif

namespace rpc {

#if defined(_WIN32)

// Discord IPC is not implemented on Windows yet (the Win32 build stays valid).
void Init() {}
void Tick(float) {}
void SetActivity(const char*) {}
void Clear() {}
void Shutdown() {}

#else

namespace {

enum State { ST_OFF, ST_DISCONNECTED, ST_CONNECTING, ST_HANDSHAKE, ST_READY };

State       g_st = ST_OFF;
int         g_fd = -1;
float       g_retry = 0.0f;
float       g_hsAge = 0.0f;
float       g_pingAge = 0.0f;
std::string g_rx;
std::string g_state;        // current state line ("" = presence without a state)
bool        g_dirty = false;// activity needs to be pushed
uint64_t    g_nonce = 1;
time_t      g_start = 0;
char        g_pendingPath[256] = {};

const char* kDetails = "CerberusMCP GUI";

bool Enabled()
{
    return config::c.discordRpc && !config::c.discordId.empty();
}

std::string JsonEscape(const char* s)
{
    std::string out;
    for (const char* p = s; p && *p; p++)
    {
        unsigned char c = (unsigned char)*p;
        if (c == '"' || c == '\\') { out += '\\'; out += (char)c; }
        else if (c < 0x20) { char b[8]; snprintf(b, sizeof(b), "\\u%04x", c); out += b; }
        else out += (char)c;
    }
    return out;
}

void CloseNow()
{
    if (g_fd >= 0) close(g_fd);
    g_fd = -1;
    g_rx.clear();
    g_pendingPath[0] = 0;
    if (g_st != ST_OFF) { g_st = ST_DISCONNECTED; g_retry = 8.0f; }
}

bool SendFrame(int opcode, const std::string& payload)
{
    if (g_fd < 0) return false;
    unsigned char hdr[8];
    unsigned int op = (unsigned int)opcode, len = (unsigned int)payload.size();
    for (int i = 0; i < 4; i++) hdr[i] = (unsigned char)((op >> (8 * i)) & 0xFF);
    for (int i = 0; i < 4; i++) hdr[4 + i] = (unsigned char)((len >> (8 * i)) & 0xFF);

    std::string msg((const char*)hdr, sizeof(hdr));
    msg += payload;

    size_t sent = 0;
    for (int spin = 0; sent < msg.size() && spin < 40; spin++)
    {
        ssize_t n = send(g_fd, msg.data() + sent, msg.size() - sent, 0);
        if (n > 0) { sent += (size_t)n; continue; }
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK))
        {
            struct pollfd p = { g_fd, POLLOUT, 0 };
            if (poll(&p, 1, 25) > 0) continue;
        }
        return false;
    }
    return sent == msg.size();
}

void CollectCandidates(std::vector<std::string>& out)
{
    const char* envs[] = { getenv("XDG_RUNTIME_DIR"), getenv("TMPDIR"), getenv("RUNTIME_DIR") };
    for (const char* dir : envs)
    {
        if (!dir || !*dir) continue;
        for (int n = 0; n < 10; n++)
        {
            char b[512];
            snprintf(b, sizeof(b), "%s/discord-ipc-%d", dir, n);
            out.push_back(b);
        }
    }
    for (int n = 0; n < 10; n++)
    {
        char b[128];
        snprintf(b, sizeof(b), "/tmp/discord-ipc-%d", n);
        out.push_back(b);
    }
}

bool TryConnect()
{
    std::vector<std::string> paths;
    CollectCandidates(paths);
    for (const std::string& p : paths)
    {
        if (access(p.c_str(), F_OK) != 0) continue;
        int fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (fd < 0) return false;
        int fl = fcntl(fd, F_GETFL, 0);
        fcntl(fd, F_SETFL, fl | O_NONBLOCK);
        struct sockaddr_un addr;
        memset(&addr, 0, sizeof(addr));
        addr.sun_family = AF_UNIX;
        snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", p.c_str());
        int r = connect(fd, (struct sockaddr*)&addr, sizeof(addr));
        if (r == 0)
        {
            g_fd = fd;
            g_st = ST_HANDSHAKE;
            snprintf(g_pendingPath, sizeof(g_pendingPath), "%s", p.c_str());
            SendFrame(0, "{\"v\":1,\"client_id\":\"" + JsonEscape(config::c.discordId.c_str()) + "\"}");
            g_hsAge = 0.0f;
            g_dirty = true;
            return true;
        }
        if (errno == EINPROGRESS)
        {
            g_fd = fd;
            g_st = ST_CONNECTING;
            snprintf(g_pendingPath, sizeof(g_pendingPath), "%s", p.c_str());
            g_hsAge = 0.0f;
            return true;
        }
        close(fd);
    }
    return false;
}

void ParseFrames(std::string& buf)
{
    while (buf.size() >= 8)
    {
        const unsigned char* h = (const unsigned char*)buf.data();
        unsigned int opcode = 0, len = 0;
        for (int i = 0; i < 4; i++) opcode |= (unsigned int)h[i] << (8 * i);
        for (int i = 0; i < 4; i++) len |= (unsigned int)h[4 + i] << (8 * i);
        if (len > (64u << 20)) { CloseNow(); return; }          // garbage frame
        if (buf.size() < 8 + len) return;                       // incomplete
        std::string payload = buf.substr(8, len);
        buf.erase(0, 8 + len);

        if (opcode == 2) { CloseNow(); return; }                 // CLOSE
        if (opcode == 3) { SendFrame(4, "{}"); continue; }       // PING -> PONG
        if (opcode != 1) continue;                               // PONG / other
        if (payload.find("\"READY\"") != std::string::npos || g_st == ST_HANDSHAKE)
        {
            g_st = ST_READY;
            g_dirty = true;
        }
    }
}

void SendActivity(bool clear)
{
    if (g_fd < 0 || g_st != ST_READY) return;
    char pidBuf[32];
    snprintf(pidBuf, sizeof(pidBuf), "%d", (int)getpid());
    char nonce[32];
    snprintf(nonce, sizeof(nonce), "%llu", (unsigned long long)g_nonce++);

    std::string activity = "{\"details\":\"" + std::string(kDetails) + "\"";
    if (!g_state.empty()) activity += ",\"state\":\"" + JsonEscape(g_state.c_str()) + "\"";
    activity += ",\"timestamps\":{\"start\":" + std::to_string((long long)g_start) + "}}";

    std::string args = "{\"pid\":" + std::string(pidBuf) + ",\"activity\":";
    args += clear ? "null" : activity;
    args += "}";

    SendFrame(1, "{\"cmd\":\"SET_ACTIVITY\",\"args\":" + args + ",\"nonce\":\"" + nonce + "\"}");
}

} // namespace

void Init()
{
    g_start = time(nullptr);
    g_st = ST_DISCONNECTED;
    g_retry = 1.5f;
    g_dirty = false;
}

void Tick(float dt)
{
    if (!Enabled())
    {
        if (g_st != ST_OFF) { CloseNow(); g_st = ST_OFF; }
        return;
    }
    if (g_st == ST_OFF) { g_st = ST_DISCONNECTED; g_retry = 1.0f; }

    g_retry -= dt;
    if (g_st == ST_DISCONNECTED)
    {
        if (g_retry <= 0.0f)
        {
            if (!TryConnect()) g_retry = 10.0f;
        }
        return;
    }

    if (g_st == ST_CONNECTING)
    {
        struct pollfd p = { g_fd, POLLOUT, 0 };
        if (poll(&p, 1, 0) <= 0)
        {
            g_hsAge += dt;
            if (g_hsAge > 3.0f) CloseNow();
            return;
        }
        int err = 0;
        socklen_t el = sizeof(err);
        getsockopt(g_fd, SOL_SOCKET, SO_ERROR, &err, &el);
        if (err != 0) { CloseNow(); return; }
        g_st = ST_HANDSHAKE;
        g_hsAge = 0.0f;
        SendFrame(0, "{\"v\":1,\"client_id\":\"" + JsonEscape(config::c.discordId.c_str()) + "\"}");
        g_dirty = true;
        return;
    }

    // ST_HANDSHAKE / ST_READY
    struct pollfd p = { g_fd, POLLIN, 0 };
    int pr = poll(&p, 1, 0);
    if (pr > 0)
    {
        char buf[4096];
        for (;;)
        {
            ssize_t n = recv(g_fd, buf, sizeof(buf), 0);
            if (n > 0) { g_rx.append(buf, (size_t)n); continue; }
            if (n == 0) { CloseNow(); return; }
            if (errno == EAGAIN || errno == EWOULDBLOCK) break;
            CloseNow();
            return;
        }
        ParseFrames(g_rx);
        if (g_fd < 0) return;
    }
    else if (pr < 0) { CloseNow(); return; }

    if (g_st == ST_HANDSHAKE)
    {
        g_hsAge += dt;
        if (g_hsAge > 5.0f) CloseNow();     // Discord rejected / never answered
        return;
    }

    if (g_dirty)
    {
        g_dirty = false;
        SendActivity(false);
    }
    g_pingAge += dt;
    if (g_pingAge > 30.0f) { g_pingAge = 0.0f; SendFrame(3, "{}"); }
}

void SetActivity(const char* state)
{
    std::string s = state ? state : "";
    if (s == g_state && !g_dirty) return;
    g_state = s;
    g_dirty = true;
    if (g_st == ST_READY) SendActivity(false);
}

void Clear()
{
    g_state.clear();
    if (g_st == ST_READY) SendActivity(true);
    g_dirty = true;
}

void Shutdown()
{
    if (g_fd >= 0 && g_st == ST_READY) SendActivity(true);
    CloseNow();
    g_st = ST_OFF;
}

#endif // !_WIN32

} // namespace rpc
