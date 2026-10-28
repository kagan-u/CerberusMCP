#pragma once
// Real MCP server registry (schema v2) behind the core FFI.
//
// `reg::registry` is the full JSON document; `reg::servers` is a flat,
// display-ready view built from it. All persistence goes through
// core::RegistrySet (validate + atomic save); nothing is written by hand.
#include "imgui.h"
#include "nlohmann/json.hpp"
#include <string>
#include <vector>

namespace reg {

using json = nlohmann::json;

struct ServerView
{
    std::string              name;
    std::string              description;
    std::string              endpoint;      // stdio: "npx -y pkg arg"; http: url
    std::string              transportName; // "stdio" | "http"
    std::vector<std::string> tags;
    bool                     enabled = true;
    json                     def;           // ServerDef JSON (editor round-trip)
    ImVec4                   c0, c1;        // accent pair derived from the name
};

extern json                    registry;
extern std::vector<ServerView> servers;
extern std::string             registryPath;

bool Refresh(std::string* err = nullptr);  // core::RegistryGet + rebuild views
bool Save(std::string* err = nullptr);     // core::RegistrySet + refresh; reloads on failure
int  IndexOf(const std::string& name);     // -1 when missing
void Shutdown();

std::vector<std::string> SplitTags(const std::string& csv);
std::string              JoinTags(const std::vector<std::string>& tags);

} // namespace reg
