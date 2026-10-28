#include "cerberus_api.h"

#include <cstdlib>
#include <utility>

extern "C" {
// Declarations must match src/ffi.rs exactly.
void cerberus_string_free(char *ptr);
const char *cerberus_last_error(void);
char *cerberus_init(void);
char *cerberus_registry_get(void);
char *cerberus_registry_set(const char *payload);
char *cerberus_registry_validate(void);
uint64_t cerberus_doctor_start(int probe_servers);
uint64_t cerberus_gateway_start(const char *opts_json);
uint64_t cerberus_gateway_stop(void);
char *cerberus_gateway_status(void);
char *cerberus_logs_take(uint64_t since);
char *cerberus_job_poll(uint64_t id);
void cerberus_job_free(uint64_t id);
}

namespace core {

namespace {

// Takes ownership of a core-allocated string: parse, free, return.
json TakeStrict(char *ptr)
{
    if (!ptr) return nullptr;
    std::string text(ptr);
    cerberus_string_free(ptr);
    try {
        return json::parse(text);
    } catch (...) {
        return nullptr;
    }
}

json Take(char *ptr) { return TakeStrict(ptr); }

} // namespace

json Init() { return TakeStrict(cerberus_init()); }
json RegistryGet() { return TakeStrict(cerberus_registry_get()); }

bool RegistrySet(const json &registry)
{
    if (registry.is_null()) return false;
    const std::string text = registry.dump();
    json result = Take(cerberus_registry_set(text.c_str()));
    return result.is_object() && result.value("ok", false);
}

json RegistryValidate() { return TakeStrict(cerberus_registry_validate()); }

uint64_t DoctorStart(bool probeServers)
{
    return cerberus_doctor_start(probeServers ? 1 : 0);
}

uint64_t GatewayStart(const json &opts)
{
    const std::string text = opts.is_object() ? opts.dump() : std::string("{}");
    return cerberus_gateway_start(text.c_str());
}

uint64_t GatewayStop() { return cerberus_gateway_stop(); }
json GatewayStatus() { return TakeStrict(cerberus_gateway_status()); }
json JobPoll(uint64_t id) { return TakeStrict(cerberus_job_poll(id)); }
void JobFree(uint64_t id) { cerberus_job_free(id); }
json LogsTake(uint64_t since) { return TakeStrict(cerberus_logs_take(since)); }

std::string LastError()
{
    const char *msg = cerberus_last_error();
    return msg ? std::string(msg) : std::string();
}

} // namespace core
