#include "registry.h"
#include "cerberus_api.h"

#include <cstdio>

namespace reg {

json                    registry;
std::vector<ServerView> servers;
std::string             registryPath;

namespace {

ImVec4 HueColor(float hue, float s, float v)
{
    float r, g, b;
    ImGui::ColorConvertHSVtoRGB(hue, s, v, r, g, b);
    return ImVec4(r, g, b, 1.0f);
}

void DeriveAccent(ServerView& s)
{
    unsigned int h = 2166136261u;
    for (size_t i = 0; i < s.name.size(); i++) { h ^= (unsigned char)s.name[i]; h *= 16777619u; }
    float hue = (h % 3600) / 3600.0f;
    s.c0 = HueColor(hue, 0.72f, 0.95f);
    s.c1 = HueColor(fmodf(hue + 0.09f, 1.0f), 0.80f, 0.78f);
}

void RebuildViews()
{
    servers.clear();
    if (!registry.is_object()) return;
    const json* jsp = nullptr;
    if (registry.contains("servers") && registry["servers"].is_object()) jsp = &registry["servers"];
    if (!jsp) return;

    for (auto it = jsp->begin(); it != jsp->end(); ++it)
    {
        const json& def = it.value();
        if (!def.is_object()) continue;

        ServerView sv;
        sv.name        = it.key();
        sv.enabled     = def.value("enabled", true);
        sv.description = def.value("description", std::string());
        if (def.contains("tags") && def["tags"].is_array())
            for (const auto& t : def["tags"])
                if (t.is_string()) sv.tags.push_back(t.get<std::string>());

        const json* trp = nullptr;
        if (def.contains("transport") && def["transport"].is_object()) trp = &def["transport"];
        sv.transportName = (trp && trp->contains("type") && (*trp)["type"].is_string())
                               ? (*trp)["type"].get<std::string>()
                               : "stdio";
        if (sv.transportName == "http")
        {
            sv.endpoint = (trp && trp->contains("url") && (*trp)["url"].is_string())
                              ? (*trp)["url"].get<std::string>()
                              : std::string();
        }
        else
        {
            if (trp && trp->contains("command") && (*trp)["command"].is_string())
                sv.endpoint = (*trp)["command"].get<std::string>();
            if (trp && trp->contains("args") && (*trp)["args"].is_array())
                for (const auto& a : (*trp)["args"])
                    if (a.is_string())
                    {
                        std::string arg = a.get<std::string>();
                        if (!arg.empty())
                        {
                            if (!sv.endpoint.empty()) sv.endpoint += " ";
                            sv.endpoint += arg;
                        }
                    }
        }

        sv.def = def;
        DeriveAccent(sv);
        servers.push_back(std::move(sv));
    }
}

} // namespace

std::vector<std::string> SplitTags(const std::string& csv)
{
    std::vector<std::string> out;
    std::string cur;
    for (char ch : csv)
    {
        if (ch == ',' || ch == ';')
        {
            while (!cur.empty() && cur.front() == ' ') cur.erase(cur.begin());
            while (!cur.empty() && cur.back() == ' ') cur.pop_back();
            if (!cur.empty()) out.push_back(cur);
            cur.clear();
        }
        else cur += ch;
    }
    while (!cur.empty() && cur.front() == ' ') cur.erase(cur.begin());
    while (!cur.empty() && cur.back() == ' ') cur.pop_back();
    if (!cur.empty()) out.push_back(cur);
    return out;
}

std::string JoinTags(const std::vector<std::string>& tags)
{
    std::string out;
    for (size_t i = 0; i < tags.size(); i++)
    {
        if (i) out += ", ";
        out += tags[i];
    }
    return out;
}

bool Refresh(std::string* err)
{
    json raw = core::RegistryGet();
    if (!raw.is_object())
    {
        if (err) *err = core::LastError();
        return false;
    }
    registry = std::move(raw);
    RebuildViews();
    return true;
}

bool Save(std::string* err)
{
    if (!core::RegistrySet(registry))
    {
        if (err) *err = core::LastError().empty() ? "registry save failed" : core::LastError();
        std::string ignored;
        Refresh(&ignored); // resync with the untouched on-disk copy
        return false;
    }
    std::string e;
    if (!Refresh(&e))
    {
        if (err) *err = e;
        return false;
    }
    return true;
}

int IndexOf(const std::string& name)
{
    for (size_t i = 0; i < servers.size(); i++)
        if (servers[i].name == name) return (int)i;
    return -1;
}

void Shutdown()
{
    registry = json();
    servers.clear();
    registryPath.clear();
}

} // namespace reg
