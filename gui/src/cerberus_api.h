#pragma once
// C++ wrapper over the cerberus-mcp static core (src/ffi.rs).
//
// Every JSON-returning call yields `null` on failure; read core::LastError()
// for the message (also logged to the GUI log stream). Returned nlohmann
// values own their data — the raw C string is freed immediately.

#include <cstdint>
#include <string>

#include "nlohmann/json.hpp"

namespace core {

using json = nlohmann::json;

json Init();                                  // paths + version; also starts log capture
json RegistryGet();                           // full schema-v2 registry (or default)
bool RegistrySet(const json &registry);       // validate + atomic save
json RegistryValidate();                      // {ok, error?}
uint64_t DoctorStart(bool probeServers);      // job id (0 = failed to start)
uint64_t GatewayStart(const json &opts);      // job id; opts {bind, token, profile, watch, watchInterval}
uint64_t GatewayStop();                       // job id
json GatewayStatus();                         // {running, managed, pid, bind, watch, ...}
json JobPoll(uint64_t id);                    // {state: pending|done|error, value?, error?}
void JobFree(uint64_t id);
json LogsTake(uint64_t since);                // {lines, next, earliest, dropped}
std::string LastError();

} // namespace core
